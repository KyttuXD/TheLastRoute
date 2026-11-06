# HTTPS in the game's HTTP client

`src/bin/last_route/net/http.rs` is the one HTTP client of the game. Until 2026-11-04 it refused `https://`, so
FROM URL and the skins host (`https://skins.kyttuslab.org`) could never work. It now speaks TLS.

## What was chosen, and why

| Choice | Reason |
|---|---|
| `rustls` 0.23, features `ring`, `std`, `tls12` (default features off) | Pure-Rust TLS, no system OpenSSL on any platform, TLS 1.3 and 1.2 only. |
| crypto provider `ring` (not `aws-lc-rs`) | `ring` builds with the normal toolchain on macOS arm64 and x86_64, Windows (MSVC and GNU) and Linux. `aws-lc-rs` needs cmake and, on Windows, NASM or a prebuilt step, which would break the local and CI packaging machines (`packaging/`). |
| roots from `webpki-roots` (the Mozilla list compiled into the binary) | No dependency on the OS certificate store, so a download behaves the same on every platform and a machine with a broken store still works. The list is refreshed by raising the crate version. |
| our own HTTP/1.1 code on top of `rustls::StreamOwned` (no `ureq`, no `reqwest`) | The client already had everything the game needs (timeouts, chunked bodies, size caps). `ureq` would add its own agent, a second redirect and header policy and more crates; the thin wrapper adds exactly two crates the game did not have (`rustls`, `webpki-roots`) plus what they pull in (`ring`, `rustls-webpki`, `rustls-pki-types`, `subtle`, `zeroize`, `untrusted`, `once_cell`). |

Server name checking is always on and there is no switch to turn certificate verification off. The tests trust a throwaway
self-signed certificate (`net/testdata/test_cert.der`, valid for 127.0.0.1 and localhost only, with its key
`test_key.der`; test material, never used outside `cfg(test)`) by building a second client config, not by weakening the real one.

### Build-time and size note

Measured on macOS arm64 (aarch64-apple-darwin), `cargo build --release --bin last_route`, rustc 1.99.1:

- Added to a clean dev build of the game: about 25 s of compile time for `ring`, `rustls`, `rustls-webpki` and `webpki-roots`
  (the C and assembly of `ring` is built by the `cc` crate: Xcode command line tools on macOS, MSVC `cl` on Windows, `cc` on Linux).
- Release binary size: not measured on this machine (release builds were not allowed in the run that wrote this). Expect
  about 1 to 1.5 MB more after stripping (`ring` about 0.5 MB, `rustls` and `rustls-webpki` about 0.5 MB, the Mozilla root list
  about 0.2 MB). Measure with `cargo build --release --bin last_route` before and after and put the numbers here.

`lr_server` links none of it (the server never calls out over TLS; `LASTROUTE_ACCOUNT_UPSTREAM` is plain http behind Caddy).

## Rules every request follows

- One request per connection, `Connection: close`, one deadline for DNS, connect, handshake, redirects and body. The system
  resolver has no timeout of its own, so the lookup runs on a helper thread and the request waits for it only until the deadline.
- `https://` and `http://` only. Any other scheme in a redirect is refused.
- A response body is never read past `Request::limit` (the announced `Content-Length` is judged before reading; chunked and
  close-delimited bodies are cut off when they pass the cap).
- `Request::content_types`: a 2xx answer with a `Content-Type` outside the list is refused before its body is read.
  An answer without the header passes (the caller validates the bytes).
- Redirects are **off** by default. A request opts in with `.redirects(n)` (never more than 3, `MAX_REDIRECTS`). Each hop:
  1. the new address is resolved against the old one (absolute, `//host/x`, `/x`, relative);
  2. the request's `hop_guard` must accept it (the skin code passes the skin URL rules, so a redirect cannot leave the allowed hosts);
  3. the scheme must be `https`. Plain `http` is accepted only for a loopback host and only when the previous hop was also plain
     http (tests); an https hop can never go down to http;
  4. a non-public address is refused when the request is `public_only` (see below);
  5. the headers are cut down to `Accept` and `User-Agent` as soon as scheme, host or port change, so a session token, `Authorization`
     or `Cookie` is never sent to another site; a 307/308 with a body to another site is refused; 301/302/303 become a GET;
  6. the redirect's own body is never read.
- `Request::public_only` (downloads from an address a player typed or a server told us): after the name is resolved, loopback,
  private (10/8, 172.16/12, 192.168/16), link-local, carrier-grade NAT, multicast, reserved and documentation addresses, and IPv6
  unique-local and link-local ones are refused, as is any IPv6 address that carries a non-public IPv4 address (v4-mapped
  `::ffff:a.b.c.d`, v4-compatible `::a.b.c.d`, NAT64 `64:ff9b::/96`, 6to4 `2002::/16`, Teredo; the NAT64 local-use prefix is refused
  outright), on every hop, and the socket connects to the address that was checked (no second
  lookup), so a public name that points at the LAN or at this machine cannot be used to reach it.
- Backend calls (accounts, rooms, saves, the update manifest) keep their old behaviour: plain http to the configured origin, no
  redirects, no address filtering (a LAN or loopback server is legitimate).

## Where it is used

- `shared/skins.rs` `fetch_https` (FROM URL) and `download` (the skins host).
- Everything else still uses plain `Request::send()`.

An internet check (`cargo test --bin last_route live_ -- --ignored`) fetches from `skins.kyttuslab.org` and other hosts to prove the built-in roots reach the real hosts.
