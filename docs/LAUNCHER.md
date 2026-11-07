# Launcher: native builds delivered from the website

The website is the update. A tiny native program, the **launcher**, asks `https://lastrouterust.games.kyttuslab.org` for the newest
signed *native* build of this computer's platform, installs it into a per-user folder, and starts it. The game that runs is the
same Rust binary KT's machines run today (Metal or OpenGL, many threads, no 32-bit wasm memory limit, no WebGL limits). The web
version is only the fallback. Deploying to vps2 therefore reaches every desktop install on its next start, with no installer and
no GitHub.

Status (2026-11-06): built and tested on Windows against a fake website on 127.0.0.1. **Nothing is on vps2 yet**, no signing key
exists yet, and nothing was run on a Mac or Linux machine. [What is verified and what is not](#what-is-verified-and-what-is-not)
lists it exactly.

```
                         start
                           |
                 state.json: current build?
                  /                          \
            installed                      nothing installed
               |                                 |
   start it NOW, and in the background      ask the site (4 s), download,
   look for a newer build for next start    verify, install  (console progress)
               |                                 |
   crash in the first 30 s twice?        site unreachable? -> message, exit 2
     -> mark bad, go back to previous    no native build / OS too old / refused /
     -> none left: open the web version    download failed   -> open the web version
```

## What is where

| Part | Where |
|---|---|
| Launcher program | `crates/lr_launcher` (bin `lr_launcher`; `src/bin/lr_stub_game.rs` is a fake game for the tests) |
| Shared logic: manifest, signature, https, install, state | `crates/lr_update` (also used by the game's own updater) |
| Public keys the programs trust | `crates/lr_update/keys/trusted.txt` |
| Build the launcher | `packaging/native/build-launcher.sh` |
| Make and check a signed build tree | `packaging/native/manifest.mjs` |
| Publish a build to vps2 | `packaging/publish-native-vps2.sh` |
| In-game updater (website first, vps3 as fallback) | `src/bin/last_route/app/updater.rs`, `updater_site.rs` |

The crates do not depend on macroquad, so `cargo build -p lr_launcher` never compiles the game. The root `Cargo.toml` is now a
workspace root (`members = [".", "crates/*"]`); `cargo test` and `cargo build` at the root still build only the game package.

## The website layout

Static files under the vps2 web root (`/srv/lr-rust-web`). `deploy-vps2.sh` (the web build) never deletes other folders, so
`native/` survives web deploys. Caddy needs no change (it already serves static files and byte ranges; checked read-only).

```
native/<platform>/latest.json                         small pointer, written LAST
native/<platform>/builds/<N>/manifest.json            schema lr.native/1
native/<platform>/builds/<N>/manifest.json.sig        ed25519 over the exact manifest bytes, 128 hex digits
native/<platform>/builds/<N>/f0.z, f1.z, ...          one zlib-compressed file each
```

Platforms: `windows-x86_64`, `macos-universal` (one universal binary for Intel and Apple Silicon), `linux-x86_64`
(`linux-aarch64` is accepted but nothing builds it). A channel other than the default publishes to `native-<channel>/` instead
of `native/` and installs into its own `native-<channel>` user folder.

`latest.json` is `{"build":47,"manifest":"builds/47/manifest.json","sha256":"<of manifest.json>"}`. Everything it names is already
in place when it is written, so a client never sees a half-published build, and it can only name `builds/<its own build>/`.

`manifest.json`:

```json
{"schema":"lr.native/1","platform":"macos-universal","build":47,"version":"0.1.0","revision":"<git sha>","built":"2026-11-07T10:00:00Z",
 "min_os":"11.0","launcher_min":1,"web_build":"<sha256 of the web wasm it was made with>","notes":"short text",
 "run":"KYTTU The Last Route.app/Contents/MacOS/last_route",
 "files":[{"path":"KYTTU The Last Route.app/Contents/MacOS/last_route","size":57760672,"sha256":"...",
           "z":"f0.z","z_size":14000000,"z_sha256":"...","mode":"755"}]}
```

Everything in it is treated as hostile until checked (`crates/lr_update/src/manifest.rs` is the reference, `manifest.mjs` mirrors
it): the signature must verify first; file names must be plain relative paths (no `..`, no absolute path, no drive letter or `:`,
no backslash, no hidden or Windows-reserved names, no file inside a file, no two names that differ only by case); at most 2000
files, 512 MB per file, 1 GB in all; hashes must be 64 hex digits; `mode` is `755` or `644`; the platform must be the one asked for;
`run` must be one of the files; the compressed copy is a plain file name in the build's folder. Every download is cut off at its
listed size, unpacked with a hard output limit (a zip bomb stops at the listed size), and its sha256 is compared before it is
written. Rules the launcher applies on top: a build lower than any build it ever installed is refused (a replayed old
`latest.json` cannot downgrade it, not even with `--force`); a build marked bad is skipped until a higher one appears; `min_os`
and `launcher_min` are checked before downloading.

## Signing

Without a signature, whoever controls vps2 controls what runs on every player's computer. With it they would also need KT's key.
The launcher and the game verify `manifest.json.sig` against the public keys compiled in from `crates/lr_update/keys/trusted.txt`.
**The file has no key yet, on purpose:** with no key, nothing can be installed and the launcher falls back to the web version
(the game keeps using its old updater). First-time setup, once:

```sh
node packaging/native/manifest.mjs keygen "C:/Users/Kyttu/lr-native-signing.pem"
```

This refuses a path inside the repository and refuses to overwrite a key. It prints the public key (64 hex digits): add it as a line
to `trusted.txt`, rebuild the launcher and the game, and **back the private key up** (losing it means a new launcher with a new
public key on every machine). For a rotation, add the new public key as a second line, ship, then publish with the new private key.
Publishing needs `SIGN_KEY=<path to the pem>`.

## Install location and state

| System | Folder |
|---|---|
| Windows | `%LOCALAPPDATA%\KYTTU\TheLastRoute\native` (not Roaming: a 60 MB program must not roam) |
| macOS | `~/Library/Application Support/KYTTU/TheLastRoute/native` |
| Linux | `$XDG_DATA_HOME` or `~/.local/share`, then `KYTTU/TheLastRoute/native` |

`LR_NATIVE_DIR` or `--dir` overrides it. Inside: `builds/<N>/` (the files exactly as the manifest lists them, plus `.manifest.json` and
`.manifest.json.sig`, verified again at every start), `tmp/` (downloads in progress, removed at every start), `state.json`,
`launcher.log` (plain text, rotated at 256 KB), `install.lock`.

A download goes to `tmp/<N>.partial`, is verified file by file, and is renamed to `builds/<N>` only when whole; `state.json` is
replaced by writing a temp file and renaming it. **A running program is never replaced:** versions sit side by side, so there are no
Windows file locks. Garbage collection keeps `current` and `previous` and ignores errors (it tries again at the next start).
A damaged `state.json` is rebuilt from the `builds/` folder.

`state.json` holds `current`, `previous` (the rollback target), `bad` (builds that crashed at start), `floor` (the highest build ever
installed), `failures` (failed starts of `current` in a row) and the last launch record.

## The launcher flow

1. Read `state.json`; check the current build is whole (its saved manifest still verifies against the trusted keys, all files exist
   with the listed size; `--verify` also re-hashes everything).
2. **A build is installed:** start it at once, passing the original arguments and setting `LR_LAUNCHER=1` and `LR_BUILD=<n>`. While it
   runs, a background thread asks the site (4 s timeout) and, if a newer verified build exists, installs it for the next start. The
   launcher waits for that to finish after the game ends (up to 15 minutes). `--update-first` does the check and install before
   starting instead; the first start after a publish is then already the new build.
3. **Nothing installed:** ask the site and install in the foreground with a console progress line ("Downloading build 47: 42%").
4. **No native build can run** (the site has no `latest.json` for this platform, `min_os` is higher than this system, the
   manifest fails a check or signature, the download fails, or the signing key is missing): the default browser opens the web
   version, with one line saying why. A browser is used and not an embedded web view: it is about 30 lines and no dependencies, and
   it handles pointer lock and the gamepad (WebKit's pointer lock in `WKWebView` is an open bug, so a web view would break mouse-look
   on macOS). **Exception:** if the site cannot be reached at all and nothing is installed, the launcher prints a message and exits
   with code 2 and does not open a browser (it would not get through either).
5. **Offline with a build installed:** the build starts as usual; the failed check is only logged.
6. **Crash-loop detection.** A start counts as failed if the game exits non-zero, is killed, or cannot be started within the first
   30 seconds. A game still running after 30 seconds, or one that exits with 0, counts as a good start and resets the count. The
   second failure in a row marks the build bad, goes back to `previous` in the same run and starts that. With no earlier build the
   web version opens. A bad build is not downloaded again; the next higher build is. (Release builds use `panic = "abort"`, so a
   panic at start is a non-zero exit.) A "ready file" written by the game after N drawn frames would be a better signal; it is deferred.

On Windows the launcher is a console program so first-run progress can print. If it was started by a double click (it is alone in its
console window), the window is closed as soon as the game is running; started from a terminal, the terminal is left alone.
`LR_LAUNCHER_KEEP_CONSOLE=1` keeps the window.

### Command line

Anything the launcher does not know goes to the game unchanged (`--headless`, `--test-suite`, ... reach the game); `--` ends the launcher's flags.

| Flag | Meaning |
|---|---|
| `--check` | Report what is installed and what the site offers; change nothing. Exit 0 up to date, **10 update available**, 2 offline, 3 no native build, 4 refused. |
| `--dry-run` | Like a normal start, but only say what would be downloaded and started. Changes nothing, starts nothing. |
| `--force` | Install the site's build even if it is the current one or was marked bad (never a lower build than ever installed). Implies `--update-first`. |
| `--update-first` | Check and install before starting. |
| `--no-launch` | Install, then stop. |
| `--verify` | Re-hash every installed file before starting. |
| `--rollback` | Mark the current build bad and make `previous` current. Exit 1 if there is nothing to go back to. |
| `--channel NAME` | Use `native-NAME` on the site and in the user folder. |
| `--dir PATH`, `--base-url URL`, `--web-url URL`, `--platform ID`, `--os-version V` | Overrides for tests. `--base-url` must be https, or http on localhost. |
| `--test-mode` | Quiet; never opens a browser; prints one `LAUNCHER_RESULT {json}` line. |
| `--help`, `--version` | |

Exit code: the game's own exit code when it ran; 2 offline with nothing installed; 64 bad arguments.
Environment: `LR_NATIVE_DIR`; `LR_NATIVE_PUBKEY` (64 hex digits) adds a trusted key **only when `--base-url` is on localhost** (tests);
the real site never accepts it.

## The in-game updater

`app/updater.rs` now looks at the website first (`updater_site.rs`, the same checks as the launcher: pointer, manifest hash,
signature, every file's sha256, `min_os`) and uses the old vps3 update API (with the `download-key` file) only when the website
cannot be used: no build for this platform, unreachable, or a failed check. No download key is needed for the website path. A newer
build must have a higher number and another commit than the running one. Windows and Linux download just the program file and
replace the running executable as before; macOS downloads the whole bundle to a hidden folder beside the running one and swaps it.
A game started by the launcher (`LR_LAUNCHER=1`) does no update check at all: the launcher owns updates, so they cannot disagree.
`LR_UPDATE_SITE_URL` may point the game at a local test site; any non-localhost value is ignored.

Target state (not done): the game keeps no network update code and reads `state.json` plus a progress file the launcher writes, which
retires `updater.rs`, the `download-key` and the vps3 file route. Kept for now so nothing breaks while the launcher is proven.

## Install the launcher

Build it (the signing key must already be in `trusted.txt`, see above):

```sh
sh packaging/native/build-launcher.sh        # -> dist/launcher/
```

* **Windows** (KT's PC): copy `dist/launcher/lr_launcher.exe` anywhere, e.g. `%LOCALAPPDATA%\KYTTU\lr_launcher.exe`, and make a
  shortcut to it. The first start downloads about 7 MB (a console shows progress). Files the launcher writes carry no
  Mark-of-the-Web, so SmartScreen does not apply to the downloaded game; the launcher exe itself only gets the warning if it
  comes through a browser download. Defender sometimes distrusts unsigned downloaders: test on the PC first.
* **macOS** (KT-MBP, the 2015 Airs, KT-NEO): `build-launcher.sh` on a Mac makes `KYTTU The Last Route.app` (universal, ad-hoc
  signed with `codesign --sign -`, no Dock icon of its own). Copy it with `scp -r` or `rsync` to the target Mac (**not** through a
  browser: a browser download is quarantined and needs right-click Open or `xattr -dr com.apple.quarantine`). Files a plain process
  writes get no quarantine attribute, so the downloaded game does not trigger Gatekeeper. The 2015 Airs run macOS 12.7.6; the build's
  `min_os` (11.0 by default for macOS) is checked against `sw_vers`, and a Mac below it gets the web version.
* **Linux:** copy `dist/launcher/lr_launcher` to `~/.local/bin/`. It needs no libraries of its own (rustls, no OpenSSL);
  the game needs the packages in `packaging/linux/build.sh`.

Remove it again by deleting the launcher and the install folder above. Nothing else is touched: no registry entry, no service,
no persistence.

## Publish a native build

1. Build the platform artifacts as before: `sh packaging/local/publish-local.sh build [windows|linux|macos|all]` (files appear in
   `dist-local/<N>/`). Do **not** run its `publish` step for the website; it publishes to vps3.
2. Publish each platform to vps2:

   ```sh
   SIGN_KEY="C:/Users/Kyttu/lr-native-signing.pem" \
   sh packaging/publish-native-vps2.sh windows-x86_64 dist-local/48/last_route-windows-x86_64.exe 48 <git sha>
   ```

   Try it first with `DRY_RUN=1` (no key needed): it unpacks, compresses and builds the manifest in a temporary folder, prints the
   file list and every server step, and uploads nothing. The real run checks the tree locally, refuses a build number that is not
   higher than the live one (`FORCE=1` overrides), uploads `builds/<N>` into a staging folder and moves it into place, fetches every
   file back through the live URL and compares sha256, writes `latest.json` last, and prunes to the newest 3 builds.
3. Check from any machine: `lr_launcher --check --base-url https://lastrouterust.games.kyttuslab.org` (exit 10 means a newer build
   than that machine has) and, with the public key, `node packaging/native/manifest.mjs verify` on a copy of the tree.

The web build and the native build are published separately (`deploy-vps2.sh` and this script). Publish both for a release; the
manifest records which web build it was made with (`web_build`, informational).

## Roll back

* **One machine:** `lr_launcher --rollback` marks the current build bad and makes the previous one current. The bad build waits for a
  higher one. (Crashing twice at start does this by itself.)
* **Everyone:** publish the old code again under a **higher** build number; clients never go down. The previous `builds/<N>` folders
  on the server (newest 3) can be used to check what was live.
* **Stop native updates temporarily:** remove `native/<platform>/latest.json` on vps2. Installed copies keep running; new installs
  get the web version.

## Tests

```sh
cargo test -p lr_update --offline                 # manifest, signature, http, install, state: 38 tests
cargo test -p lr_launcher --offline               # args; flow (30 tests); the real binary (6); the Node publisher contract (2)
cargo test --no-run --lib --bin last_route        # then run the newest test exe directly
```

The launcher tests run the real flow against an in-process website on 127.0.0.1 with a stub game: first install, up to date,
background install and next-start switch, `--update-first`, a flipped byte, a cut-off download, a wrong key, offline with and
without an installed build, no native build, OS too old, launcher too old, crash loop and automatic rollback, crash loop with
nothing to go back to, a game that survives the startup window, `--rollback`, downgrade refused (also with `--force`), hostile
manifests signed with the right key (path traversal, absolute and drive paths, huge size, wrong platform), no signing key,
damaged `state.json`, a changed installed file, `--check`, `--dry-run`, `--no-launch`, a held install lock, channels, exit codes and
argument pass-through. `node_contract.rs` builds a tree with the real Node publisher (key generation, build, signature, verify) and
installs it with the launcher, so the two languages cannot drift apart.

## What is verified and what is not

**Verified by tests on Windows:** everything in the list above, the publisher script's `DRY_RUN` for the exe, tar.gz and folder
layouts, the Node and Rust sides agreeing on manifest, zlib and ed25519, and that the game and the wasm build still compile.

**Needs a Mac:**
* The game started by `exec` from inside the downloaded `.app` folder: does it get the right Dock icon and focus? If not, switch to
  `open -n -a` plus the ready-file signal (deferred).
* `build-launcher.sh` on macOS (lipo, `codesign --sign -`, the `.app` layout) and ad-hoc signing on Apple Silicon.
* `sw_vers` parsing for `min_os`, the `unzip` step of `publish-native-vps2.sh` on a real `universal.sh` zip, the in-game updater's
  bundle swap, and whether Metal on the 2015 Airs behaves (unmeasured; the Airs hold 60 fps with OpenGL).
* Gatekeeper: confirm a launcher copied with `scp` and a game written by it never show a prompt.

**Needs the live site or vps2:** the actual upload, Caddy serving `native/...` with the right types and ranges (`curl -I` it after
the first publish; Caddy sends ETags and 206 for ranges; there is no `precompressed` option and the launcher sends
`Accept-Encoding: identity`), real TLS from an old Mac (rustls with the bundled Mozilla roots needs no system store), disk space
(172 GB free on vps2; a build is about 7 MB compressed on Windows, 14 MB macOS, 31 MB Linux), and a first real signing key.

**Needs Linux:** nothing was built or run on Linux; `cargo check --target x86_64-unknown-linux-gnu` was not run (the target is not installed on the PC).

## Honest limits and what is deferred

* **No numbers for the web gap.** The repo has no web frame-rate measurement; "native is faster than the browser build on a weak Air"
  is an inference from structure (the web build is single-threaded 32-bit wasm over WebGL and starts at a lower render scale), not a
  measurement. The 2015 Air holds 60 fps native with OpenGL; the 100+ target is not met; Metal on Intel is unmeasured.
* **A new build lags one start** unless `--update-first` is used (a shortcut can pass it). A native build also lags the web deploy
  until it is published; players may run an older native build while the web is newer.
* **No window UI.** First run is a console line; a splash window is deferred. The console closes itself on a double click (Windows).
* **No launcher self-update.** If a manifest's `launcher_min` is higher than the launcher's level, the launcher logs it and keeps
  the installed build; replacing the launcher is by hand (scp). `launcher_min` is 1 and the level is 1.
* **No resumable downloads.** A cut-off download starts over (the server supports ranges; deferred). Memory use is the largest
  compressed file plus its unpacked size (about 75 MB for the macOS binary).
* **Public files.** vps2 serves `native/` to anyone, unlike the keyed vps3 downloads. The wasm is already public, so the exposure is
  about the same. Gating them needs a Caddy change.
* **The ready file** (the game writes it after 120 drawn frames) and a better "did it start" signal: deferred. Until then a start
  that dies after 30 s is not counted as a failed start.
* **Embedded web view:** not built (about 100 extra crates, broken pointer lock in WebKit, unverified gamepad). The browser
  fallback is the supported route.
* **The game does not yet read the launcher's state**, and `updater.rs`, the `download-key` and the vps3 update route stay until the
  launcher is proven on every machine.
* **Trust, stated honestly.** A downloaded native binary is a code update, the same trust the vps3 update path already has. It is
  not game-code scripting, but it should be recorded as an explicit exception to any "no remote code" rule (the other repo's
  AGENTS.md forbids executing remote JS/YAML from game code; this repo has no AGENTS.md). The signature makes vps2 alone
  insufficient to push code; the key on KT's PC is now the thing to protect.

## Questions for KT

1. Are public native files acceptable, or should vps2 gate them?
2. Which macOS versions do the Airs run? Do any run below 11.0? (`min_os` decides who gets the web version.)
3. When a native build lags the web deploy, may players run the older native build, or should they get the web version?
4. Is a console window on first run acceptable for v1?
5. Where should the signing key live, and is a backup needed?
6. Retire the vps3 update API after the transition?
