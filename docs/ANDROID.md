# Android

What it takes to ship The Last Route as an Android app, what is already done, and what is still open. Checked on
2026-11-13 on a Linux machine with no phone or emulator, so everything under "Verified" is a build or a test, never a run
on a device. iOS is on hold (it needs full Xcode, which only runs on a Mac).

## Verified

| What | How |
|---|---|
| The game crate compiles for `aarch64-linux-android` | `cargo check --target aarch64-linux-android --bin last_route` with NDK r27c (27.2.12479018), clang API 24, Rust 1.99.1. No warnings from our code; the vendored `macroquad` and `miniquad` give three. |
| The renderer fits GLES | The shaders are all `#version 100` (GLSL ES 1.00), the same ones the browser build uses. `miniquad` has a GLES 2 and 3 backend for Android. |
| Assets ship inside the binary | `src/bin/last_route/assets` (about 39 MB) is embedded with `include_bytes!`, like the wasm build. The APK needs no assets folder. Fonts fall back to embedded ones when the system has none. |
| Audio has an Android path | `quad-snd` has an OpenSLES backend. |
| The production directory is HTTPS | `PRIMARY_BASE_URL` is `https://lastrouterust.games.kyttuslab.org`; the gameplay relay is UDP. No cleartext-traffic exception is needed. The dev-only test instance is plain HTTP and stays a debug-build feature. |
| Touch controls exist | `src/bin/last_route/app/touch.rs`: D-pad with double-tap sprint, drag to look, the action buttons, the hotbar arrows, menus owning touches, 16 tests. Turned on by itself on Android and iOS, and by `--touch` on a desktop (the left mouse button stands in for a finger). |

## Build requirements

Install once on the build machine:

- Android SDK with platform 36 and build-tools (present at `~/Android/Sdk` here).
- Android NDK r27c: `sdkmanager --install "ndk;27.2.12479018"`.
- The Rust target: `rustup target add aarch64-linux-android`. Add `armv7-linux-androideabi` for 32-bit phones and `x86_64-linux-android` for an emulator.
- For `cargo check` or `cargo build`, point cargo at the NDK's clang:

```bash
BIN=~/Android/Sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64/bin
CC_aarch64_linux_android=$BIN/aarch64-linux-android24-clang \
AR_aarch64_linux_android=$BIN/llvm-ar \
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$BIN/aarch64-linux-android24-clang \
cargo check --target aarch64-linux-android --bin last_route --target-dir target/android
```

The Java side (`MainActivity`, `QuadNative`) is in `vendor/miniquad/java`. An APK needs the game built as a shared
library that exports `quad_main`, wrapped by that activity. miniquad's own answer is `cargo quad-apk` (a fork of
`cargo-apk`; its README recommends the `notfl3/cargo-apk` Docker image) and says no code changes are needed. **Not tried
here yet.** A spike with that tool is the first thing to do for the APK.

## Open work

Needed before a phone build is playable or installable:

1. **The packaging spike.** Build an APK with `cargo quad-apk` (Docker or installed), install it with `adb`, see whether it
   starts. Adds a `[package.metadata.android]` table to `Cargo.toml` (package name, label, version code, the `INTERNET`
   permission, landscape, fullscreen); that is a `Cargo.toml` change, so it needs KT's go-ahead.
2. **Saves and settings do not persist.** `shared/paths.rs` `root_user_dir` reads `XDG_DATA_HOME` or `HOME`, and neither exists
   on Android, so `user_dir()` is `None`. It needs an Android branch returning the app's private files folder
   (`/data/data/<package>/files`, or `Context.getFilesDir()` through JNI).
3. **No soft keyboard.** `miniquad::window::show_keyboard` exists, but no text field in the game calls it. Account names and
   passwords, room codes and chat cannot be typed on a phone. This is also why the CHAT button is left out of the touch controls.
4. **The in-game updater must be off.** The native updater downloads desktop builds. It needs a `cfg` gate for Android.
5. **Phone budgets.** Godot has a phone profile and a low-memory mode (`game/autoload/platform_profile.gd`): smaller lamp
   pool, nearer camera far plane, smaller streaming budget, pixel effect off, render scale, no parked storeys on low RAM. The
   Rust game has its own quality manager (`app/quality.rs`) and the performance goals in `AGENTS.md` cover laptops and
   desktops, not phones. Phone targets need real numbers from a device before any setting is chosen.
6. **Screen details.** Safe-area insets for notches (the touch layout has none), the back button (miniquad's key mapping is not
   checked), and saving when the app goes to the background.
7. **Menus on touch.** Taps work through macroquad's simulated mouse. Dragging, scrolling and anything that needs hover are
   not checked.
8. **The BAG touch button.** It presses the `inventory` binding; the inventory screen is not ported (only its rules are), so
   nothing answers it.

Release, later:

- A debug keystore is enough for sideloading. A release build needs a keystore kept out of the repo, and Google Play wants
  an AAB; `cargo quad-apk` makes APKs only.
- CI: GitHub's Ubuntu runners ship an Android SDK and NDK (check the NDK version against r27c), so an `android` job next to
  the existing native ones is possible once the spike works.
- A test device. Without a phone, an x86_64 emulator needs KVM and a system image (`sdkmanager`).

## Touch controls

Port of Godot `game/hud/touch_controls.gd`. Everything feeds the game's ordinary actions, so gameplay, networking and the tests
see what a keyboard would make.

- D-pad (bottom-left, three by three cells): arrows at full strength, diagonals by sliding or two fingers, a double tap held on
  an arrow sprints that way.
- Look: a drag on any free part of the screen; a drag across the short side turns 180 degrees.
- Buttons: USE, JUMP, DUCK, LIGHT, TOOL (bottom-right); `<` and `>` hotbar arrows beside the hotbar; PAUSE, MAP, BAG, QUICK along the top.
- A menu, the update screen or the report owns every touch. With the map open only MAP and PAUSE stay.
- Not ported: the floating stick, look smoothing and acceleration settings, notch insets, CHAT (see above).
- With touch on, mouse buttons are not bindings: every finger raises a simulated left click, which would otherwise fire the
  held tool.
