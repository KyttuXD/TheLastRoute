# Deploying the web build to vps2

This is the procedure for publishing the Rust web game (`lastrouterust.games.kyttuslab.org`) to vps2. It is written so another agent (for example Cursor) can follow it end to end.

## What lives where

| Thing | Location |
|---|---|
| Live site (static files) | vps2 `/srv/lr-rust-web` |
| Public URL | https://lastrouterust.games.kyttuslab.org |
| Caddy routing | vps2 `/etc/caddy/Caddyfile`, block `lastrouterust.games.kyttuslab.org`. `/api/*` goes to `127.0.0.1:8205`; everything else is static with `Cache-Control: no-cache`. |
| Deploy script | `packaging/web/deploy-vps2.sh` (in this repo) |
| Files uploaded | `index.html update.js keys.js hw.js audio.js mq_js_bundle.js last_route.wasm`, then `version.json` last |

Do **not** touch the Godot site (`lastroutegodot.games.kyttuslab.org`, `/srv/lr-godot-web`), its backend on port 8205, the Caddyfile (this deploy never edits it), or the `lastroute-*` services.

## Prerequisites

- The repo checked out at `main` (the `git` remote is `https://github.com/KyttuXD/TheLastRoute`, private).
- Rust with the wasm target: `rustup target add wasm32-unknown-unknown`.
- `ssh vps2` works without a password prompt (a key in `~/.ssh/config` under the alias `vps2`).
- `sha256sum` or `shasum`, `tar`, `curl`.

## Step 1: build the wasm

```sh
cargo build --release --target wasm32-unknown-unknown --bin last_route --target-dir target/web
```

On a busy machine set `CARGO_BUILD_JOBS=4`. The build takes 3 to 8 minutes. The result is `target/web/wasm32-unknown-unknown/release/last_route.wasm`.

## Step 2: assemble the folder `dist/web`

```sh
mkdir -p dist/web
cp target/web/wasm32-unknown-unknown/release/last_route.wasm dist/web/
for f in index.html update.js keys.js hw.js; do cp packaging/web/$f dist/web/$f; done
```

`audio.js` and `mq_js_bundle.js` must also be in `dist/web`. `audio.js` is kept in the repo at `src/bin/last_route/audio/quad-snd/js/audio.js`; copy it from there if `dist/web` lacks it. `mq_js_bundle.js` is **not** in git: copy it from a previous `dist/web` (or the macroquad bundle). Check both exist before continuing; the script refuses to run without all seven files.

## Step 3: dry run (changes nothing)

```sh
DRY_RUN=1 sh packaging/web/deploy-vps2.sh dist/web
```

It prints the build id and the plan and exits. Check the printed `version` line looks right.

## Step 4: deploy

```sh
sh packaging/web/deploy-vps2.sh dist/web
```

The script, in order:
1. Writes `dist/web/version.json` as `{"build":"<sha256 of last_route.wasm>","built":"<UTC time>"}`.
2. Uploads the six game files by tar over ssh into a staging folder on the server, then moves each into `/srv/lr-rust-web`.
3. Checks the server's `last_route.wasm` sha256 equals the build. If not, it stops **before** uploading `version.json`, so no player is told about a broken build.
4. Uploads `version.json` last. Open game tabs poll this file and show the UPDATING screen only once every file is in place.
5. Checks the live URL serves the same `version.json` and wasm.

Success ends with `deployed: build <sha> at <time>`. Any `fail` message means the live site was **not** switched; read the message, fix, and re-run.

## Step 5: verify by hand

- Open https://lastrouterust.games.kyttuslab.org in Firefox (hard reload).
- The title screen loads, a solo run starts, and the console (F12) shows no errors on load.
- `curl -s https://lastrouterust.games.kyttuslab.org/version.json` shows the new build id.

## Rolling back

The previous wasm is gone once overwritten. To roll back, redeploy the previous build's `dist/web` folder (keep a copy of each `dist/web` before deploying). There is no automatic rollback on vps2.

## Things that must not happen

- Do not run any command that restarts, stops or edits `lastroute-godot-test`, `lastroute-region-agent`, `lastroute-room@*`, or Caddy. A web deploy only copies files.
- Do not commit `dist/`, `.wasm` files, secrets or `.env` files to git. `dist/` is ignored.
- Do not deploy from a dirty build of unmerged branches to the live site without the owner's approval.
- Do not run `deploy-vps2.sh` with `HOST` pointing at another server.

## Known limits

- The Node backend on vps2 (port 8205) is separate. A web deploy does not change room or account behaviour; the Rust room host and the BUILD MISMATCH fix need separate steps (see `docs/SERVERS.md`).
- `mq_js_bundle.js` is only updated by hand; if the macroquad version changes, copy the new bundle too.
