#!/bin/sh
# Builds the Android APK (docs/ANDROID.md).
# Usage: sh packaging/android/build-apk.sh [--release]       The APK lands in dist/android/.
#
# cargo quad-apk edits the main.rs it builds (it injects the `quad_main` entry point), so the build runs in a copy of the
# tree under target/android-apk-src, never in the working tree. The copy keeps its own target folder between runs.
#
# Needs: `cargo install --git https://github.com/not-fl3/cargo-quad-apk --locked`, `rustup target add aarch64-linux-android`,
# the Android SDK (platform 36, build-tools), NDK r27c, and a full JDK (javac, jarsigner). Environment (defaults below):
#   ANDROID_HOME  the SDK (~/Android/Sdk)        NDK_HOME  the NDK (\$ANDROID_HOME/ndk/27.2.12479018)      JAVA_HOME  a JDK
# Debug APKs are signed with ~/.android/debug.keystore (made on first use); a release build needs a real keystore.
set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORK="$ROOT/target/android-apk-src"
MODE=debug
[ "${1:-}" = "--release" ] && MODE=release

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export NDK_HOME="${NDK_HOME:-$ANDROID_HOME/ndk/27.2.12479018}"
export ANDROID_NDK_HOME="$NDK_HOME"
[ -n "${JAVA_HOME:-}" ] && export PATH="$JAVA_HOME/bin:$PATH"

cargo quad-apk --help >/dev/null 2>&1 || { echo "build-apk: cargo-quad-apk is not installed (see the header of this script)" >&2; exit 1; }
command -v javac >/dev/null 2>&1 || { echo "build-apk: no javac on the path: set JAVA_HOME to a full JDK" >&2; exit 1; }
[ -d "$NDK_HOME" ] || { echo "build-apk: the NDK is not at $NDK_HOME" >&2; exit 1; }

mkdir -p "$WORK"
rsync -a --delete --exclude /target --exclude /dist --exclude /.git --exclude /archive "$ROOT"/ "$WORK"/
cd "$WORK"
if [ "$MODE" = release ]; then
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" cargo quad-apk build --release --bin last_route
else
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" cargo quad-apk build --bin last_route
fi
mkdir -p "$ROOT/dist/android"
cp "$WORK/target/android-artifacts/$MODE/apk/last_route.apk" "$ROOT/dist/android/last_route-$MODE.apk"
echo "build-apk: $ROOT/dist/android/last_route-$MODE.apk"
