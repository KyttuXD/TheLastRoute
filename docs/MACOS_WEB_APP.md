# Last Route for macOS: the web app shell

A small native Mac app that shows the game's **web build** (the wasm) in a `WKWebView`, and keeps those web files up to date by
itself. It is for KT's own Macs (Intel today, Apple Silicon compiles too). It is not the native game and not the launcher: see
[how it differs from the launcher](#how-it-differs-from-the-launcher).

Status (2026-11-06): built and tested on KT-MBP (MacBook Pro, Intel, macOS 12.7.6) and KT-MBA (MacBook Air 2015, Intel, macOS 12.7.6)
over ssh with no GUI interaction. [What was verified and what was not](#what-is-verified-and-what-is-not) is listed exactly.

## What it does

```
  KYTTU The Last Route.app                         ~/Library/Application Support/LastRoute/
  ├─ Contents/MacOS/LastRoute  (Objective-C)   ├─ state.json           the pointer: current, previous, bad builds, failures
  └─ Contents/Resources/                       └─ web/<id>/            index.html update.js keys.js hw.js audio.js
       ├─ web/   the bundled build (fallback)                          mq_js_bundle.js last_route.wasm version.json meta.json
       ├─ shell.js    injected into the page   ~/Library/Logs/LastRoute/app.log
       └─ selftest.js for --self-test
```

* **The page** is served to the web view by a custom URL scheme, `lrapp://game/`, straight from the current `web/<id>/` folder
  (right MIME types, `application/wasm` for the wasm, byte ranges). No local HTTP server, no `file://` limits.
  `localStorage` and IndexedDB live under that origin and persist across launches (checked).
* **The server** is still the website. `shell.js` (injected at document start) sets `window.LR_API_BASE` to the site, which is the
  hook `index.html` already has: every `/api` call and the room WebSocket go to `https://lastrouterust.games.kyttuslab.org` directly
  (its `/api` answers any origin and accepts any WebSocket origin; checked with curl). `lrapp://game/api/*` is also forwarded to the
  site by the app (method, headers and body kept) for anything that asks the page's own origin.
* **Updates.** At start plus 5 s, then every 10 minutes, and from the menu (Last Route > Check for Updates...), the app reads
  `<site>/version.json` with no cache. A different `build` (the wasm sha256) makes it download `index.html update.js keys.js hw.js audio.js
  mq_js_bundle.js last_route.wasm` into a stage folder, check that the wasm's sha256 equals `version.json`'s build and starts with the wasm
  magic, re-read `version.json` (a deploy during the download is caught), and move the stage into `web/<id>/`. `<id>` is 12 hex of the wasm hash
  + 8 hex of the other files' hash, so the same wasm with a changed page file is a new copy too: when the build is unchanged the app asks for each
  small page file with its etag and installs a new copy only if one differs. The list of files is the default list, or `files` in
  `version.json` if a deploy ever adds it. The current pointer then moves; the old current becomes `previous`; everything older is deleted.
  Offline or a failed download changes nothing. The first run copies the bundled build in; an app update that carries a newer bundled build
  (`built` time) than the installed one adopts it.
* **How the page and the app cooperate (no double update).** The app serves `version.json` from the *current* pointer and every other file from
  the folder pinned when the page loaded `index.html`. So when the app installs a new build, the running page's own `update.js` sees a different
  build within 30-40 s, the game shows its UPDATING screen, counts 10 s down and reloads itself, and the reload pins to the new folder. The app
  never reloads the page by itself for an update (View > Reload Game does it on demand). The pointer is not moved while a page is still loading,
  so a page never gets a mix of two builds.
* **Rollback.** A start counts as failed when the page does not report ready within 90 s, throws a wasm error before that (the console
  or an exception: `CompileError`, `RuntimeError`, `LinkError`, `unreachable`), fails to load, the web content process dies, or the app dies while the
  page was starting. The first failure of a build retries once; the second in a row marks the build bad, moves the pointer back to `previous`, then
  to the bundled build, and starts the page again. A bad build is never downloaded again (the update check says `skipped`). With nothing left to roll
  back to the app offers: Open in Browser, Try Again, Reset to Bundled Build, Quit. "Ready" is `shell.js`: the miniquad globals `wasm_exports` / `wasm_memory` exist and 1.5 s have passed without a fatal error (a timer, not animation frames, so a covered or minimised window is not a failed start). If a new miniquad bundle renamed both globals the app would never see ready: keep them, or change `shell.js`.
* **Window and system.** One resizable window (size remembered), native full screen (`Ctrl+Cmd+F`; the game's own `canvas.requestFullscreen()` is turned into it,
  because WKWebView has no element fullscreen on older macOS), Cmd+Q / Cmd+R / Cmd+H / Cmd+M work even when the game takes the key,
  Edit menu (Cut/Copy/Paste/Select All), View > Reload Game, Open Game Website in Browser, Show Log in Finder, About with the web build id, Check for Updates,
  Reset to Bundled Build. Audio autoplay is allowed. The file chooser (`<input type=file>`, the SKIN tab) is a native open panel (files or a folder).
  `https` links and `window.open` go to the default browser (http, mailto, file and app-scheme links are dropped). Downloads (`<a download>`, macOS 11.3+) save to Downloads and reveal the file. JS
  alert/confirm are native. The right-click menu of the web view is removed.
* **Clipboard.** `lr_clip` in `index.html` works unchanged (the custom origin counts as a secure context, so `navigator.clipboard` exists), and the Edit menu
  gives WKWebView its paste/copy commands. If WKWebView ever reports paste as unavailable, Cmd+V reads the clipboard in the shell and hands the text to the page as a
  synthetic `paste` event (`window.__lrPaste`; tested).

## Pointer lock

`WKWebView` has no public API for the Pointer Lock API. Without an answer from its UI delegate WebKit refuses every `requestPointerLock()`
(`pointerlockerror`, confirmed). WebKit asks that delegate through two methods of its private WKUIDelegate interface,
`_webViewDidRequestPointerLock:completionHandler:` and `_webViewDidLosePointerLock:`; the app implements them (the first answers yes) and lock
works: on both Macs a real mouse press gave `pointerLockElement === canvas`, and a mouse-moved event with delta (17, -5) sent to the window arrived in the page
as `movementX/Y = 17,-5`. These are private selectors found by asking WebKit which ones it looks up (`respondsToSelector:`), not public API. They are not
called outside the lock path, an unknown selector is harmless, and a future macOS that renames them would only turn the lock back into
a refusal (the game keeps asking every 0.3 s and the player can use the website in a browser: View > Open Game Website in Browser). Tested on macOS 12.7.6 only.

## Build and install

From this repo, in any shell that has the ssh alias (Windows Git Bash too):

```sh
sh packaging/macos/webapp/remote-build.sh KT-MBP        # copies the sources, builds on the Mac, leaves ~/LastRouteApp/"KYTTU The Last Route.app"
```

Or on the Mac itself, from a checkout: `sh packaging/macos/webapp/build-app.sh [OUT_DIR]` (default `packaging/macos/webapp/dist`).
Needs the Command Line Tools only (clang, lipo, iconutil, sips, codesign, curl); no Xcode project, no Rust, no Python. It

1. fetches the web files from the website (`SITE_URL`, default the live site) into `web-bundle/` (gitignored) and checks the wasm sha256 against `version.json`
   (`BUNDLE_SRC=dist/web` uses a local web build instead; the wasm is never committed),
2. compiles `LastRoute.m` for `x86_64` (macOS 10.13+) and `arm64` (11.0+) and joins them with `lipo` (`ARCHS=x86_64` for Intel only),
3. assembles `KYTTU The Last Route.app` (Info.plist from `Info.plist.in`, icon from the game logo PNGs, `shell.js`, `selftest.js`, the web files),
4. signs it ad hoc (`codesign -s -`, no developer account).

To install: `cp -R ~/LastRouteApp/"KYTTU The Last Route.app" /Applications/` (or `~/Applications`), then `open -a "KYTTU The Last Route"`. An app copied by
scp or tar has no quarantine flag and just opens; one downloaded in a browser needs `xattr -dr com.apple.quarantine "KYTTU The Last Route.app"` once, because an
ad-hoc signature is not a notarized one. Updating the *app* (new code) means building and copying it again; updating the *game* never needs that.

Measured: the build takes about 8 s on KT-MBP (universal, web files already fetched about 2 s more); the app is 31 MB (30.5 MB of it the web files); each
installed copy in Application Support is 31 MB (two are kept at most: current and previous).

## Trust and limits

* The scheme handler serves only single plain files of the current build folder: the name must be letters, digits, `.`, `-`, `_` (no `/`, no leading dot, no escapes), so
  `../`, `%2f`, `%5c`, `%00` and subfolders get 400/404 (the self-test probes this). `/api/*` is forwarded only to the configured site host, never to a host the page names.
* A new build is taken only when its wasm sha256 equals `version.json`'s `build`. The page files (`index.html`, the `.js` files) are not hashed by `version.json`, so
  they are trusted as far as the site's HTTPS is. A compromised site could ship a malicious page; there is no signature (that is what the launcher's signed manifests are for).
* No downgrade: a build different from the installed one is only taken when the site's `built` time is later than the installed build's (`deploy-vps2.sh` writes the deploy time,
  so deploying an older commit still counts as newer). The log says `skipped`. If the deploy machine's clock runs behind, updates are skipped until it catches up; View >
  Reset to Bundled Build does not help with that, rebuilding the app does. A hostile site could still replay a *newer-looking* version.json: out of scope.
* Page console lines in the log are cut to 400 characters and anything that looks like a credential (`token=...`, `password: ...`, long random strings) is replaced by `[redacted]`.
* A cancelled load (Reload pressed while loading) is not counted as a failed start.

## Flags and files

`LastRoute --help` lists them. `--site-url URL` (or `LASTROUTE_SITE_URL`) the website; `--data-dir DIR`; `--log FILE`; `--no-update`;
`--check-interval S`; `--ready-timeout S`; `--max-failures N`; `--self-test` (hidden window, probes the page, writes the log, exit code 0 on pass);
`--self-test-visible` (the same with a normal focused window); `--self-test-hold S` (keep running after the probes up to S seconds, ending early when the page reloads itself).
The log is `~/Library/Logs/LastRoute/app.log` (rotated at 2 MB); it holds every boot, update, rollback and page error line.

Tests that need no GUI, run on a Mac: `sh packaging/macos/webapp/test-update.sh [APP]` serves fake sites from 127.0.0.1 with python3's `http.server`
(made from the web files in the app, so no internet is needed, and the real data folder and log are not touched) and runs the app against them:

| Scenario | Checks |
|---|---|
| offline | unreachable site: update check says `offline`, the bundled build starts |
| update | newer build: downloaded, verified, pointer moved, the running page's update.js notices, the page reloads itself into it |
| pages | same wasm, changed `update.js`: a new folder with the new file |
| mismatch | version.json names a hash the wasm does not have: refused, nothing installed |
| older | a different build with an older `built` time than the installed one: skipped, nothing installed (no downgrade) |
| rollback | garbage wasm with a correct hash: installed, two failed starts, rolled back to the previous build, the next launch does not fetch it again |

## What is verified and what is not

Verified from the log (`--self-test`, `open`, and `test-update.sh`), on KT-MBP and KT-MBA (all passes unless noted):

* builds as a universal binary (x86_64 + arm64) with the Command Line Tools only; ad-hoc signature verifies; starts via `open` and directly;
* the bundled build installs on first run; the wasm sha256 is checked at every launch (110 ms on KT-MBP, 240-360 ms on KT-MBA);
* the page starts: wasm instantiated (23 exports, 116 MB memory), WebGL 1 context from the game, 60 frames/s hidden on KT-MBP, 37 on KT-MBA; ready in 3 s (KT-MBP) / 6-8 s (KT-MBA);
* `version.json`, the wasm (`application/wasm`), `localStorage` (value kept across two launches) and IndexedDB work under `lrapp://game`;
* the game's `/api` from the page: direct (`LR_API_BASE`) GET and synchronous XHR both 200 from the live site, the same call through `lrapp://game/api/` 200, and a `wss://.../api/ws` WebSocket opens and closes cleanly;
* AudioContext starts running without a click; `navigator.clipboard`, Gamepad API and `isSecureContext` are present;
* pointer lock granted and `movementX/Y` delivered (section above); synthetic paste event delivered; the web view reports paste/copy/cut/select-all as available;
* the update, offline, pages-only, mismatch and rollback scenarios above, plus a real check against the live site that answered `uptodate`.

Not verified (needs KT to click through; the code is there):

* what the window looks like, the Dock icon, the menu bar contents, Cmd+Q / Cmd+R / full screen toggling by key, window resize by hand, remembered window frame;
* real Cmd+V / Cmd+C into the game's text fields (the page-side path and the shell fallback were tested without touching the real clipboard);
* the native file chooser (SKIN tab), downloads, external links, JS dialogs;
* a game controller, sound actually audible, long play sessions and memory (the wasm memory is 116 MB at start);
* the arm64 slice (compiled and linked, never run), and macOS 10.13-11 (the deployment target is set; nothing older than 12.7.6 was available);
* first launch of a copy that was downloaded in a browser (Gatekeeper with an ad-hoc signature);
* the pointer-lock behaviour on any macOS other than 12.7.6.

## How it differs from the launcher

The `feat-launcher` branch (`docs/LAUNCHER.md` there) is a different approach and they are not meant to be merged: the launcher downloads and starts the
**native** Rust game (signed manifests, per-platform builds, Metal/OpenGL, many threads, no 32-bit wasm memory limit) and needs a signing key and a publish
step on the website. This app runs the **web build** inside the system web view, so it has the web build's limits (WebGL 1, one thread, 4 GB wasm32) but needs
nothing on the server beyond what the site already serves (`version.json` and the seven files), no signature (it trusts the website's HTTPS and checks
the wasm hash against `version.json`), and nothing built per release: a normal `deploy-vps2.sh` is the update. Either can fall back on the other.
