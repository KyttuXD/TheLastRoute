# Android

What it takes to ship The Last Route as an Android app, what is already done, and what is still open. Checked on
2026-11-13 on a Linux machine and on a Samsung SM-S721U1 (Android 17, arm64, GLES 3.2) over USB. iOS is on hold (it needs
full Xcode, which only runs on a Mac).

## Verified

| What | How |
|---|---|
| The game crate compiles for `aarch64-linux-android` | `cargo check --target aarch64-linux-android --bin last_route` with NDK r27c (27.2.12479018), clang API 24, Rust 1.99.1. No warnings from our code; the vendored `macroquad` and `miniquad` give three. |
| The renderer fits GLES | The shaders are all `#version 100` (GLSL ES 1.00), the same ones the browser build uses. `miniquad` has a GLES 2 and 3 backend for Android. |
| Assets ship inside the binary | `src/bin/last_route/assets` (about 39 MB) is embedded with `include_bytes!`, like the wasm build. The APK needs no assets folder. Fonts fall back to embedded ones when the system has none. |
| Audio has an Android path | `quad-snd` has an OpenSLES backend. |
| The production directory is HTTPS | `PRIMARY_BASE_URL` is `https://lastrouterust.games.kyttuslab.org`; the gameplay relay is UDP. No cleartext-traffic exception is needed. The dev-only test instance is plain HTTP and stays a debug-build feature. |
| An APK builds, installs and runs | `sh packaging/android/build-apk.sh` makes `dist/android/last_route-debug.apk` (25 MB packed, a 107 MB debug library). On the phone it starts, shows the title menu in landscape, takes taps in the menus, starts an offline solo run at **60 FPS**, and the D-pad walks the runner. See "Build" below. |
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

## Build

```bash
cargo install --git https://github.com/not-fl3/cargo-quad-apk --locked     # once
JAVA_HOME=/path/to/a/full/jdk sh packaging/android/build-apk.sh          # add --release for a release build
adb install -r dist/android/last_route-debug.apk
adb shell am start -n org.kyttuslab.last_route/org.kyttuslab.last_route.MainActivity
```

The script builds in a copy of the tree (`target/android-apk-src`), because `cargo quad-apk` writes into what it builds. The
Android settings are the `[package.metadata.android]` table in `Cargo.toml`. Things the spike found, all handled:

- **The package name must end in `last_route`.** The tool names the Java library after the last part of `package_name`, so
  `org.kyttuslab.lastroute` fails at start with `library "liblastroute.so" not found`.
- **`android:exported = "true"`** on the activity (set in `activity_attributes`): target API 31 and up refuses to install an
  activity with an intent filter without it.
- **`atexit` is missing.** The tool links with a bare `ld`, and NDK r23 and newer no longer export `atexit` from the system
  libc (clang's startup objects define it), so the library failed to load with `cannot locate symbol "atexit"`.
  `app/android_shim.rs` (Android only) defines it through bionic's `__cxa_atexit`.
- **The tool needs NDK r23 or newer** (r21e fails to find libunwind), and a full JDK: a JRE has no `javac`.
- The phone must be unlocked to see the app: a locked screen stops the activity at once, and `adb` shows a black picture.

## Open work

Needed before a phone build is playable or installable:

1. **Release build and signing.** Only the debug APK is proven. A release APK (fat LTO) is untested, and a real keystore is needed.
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
