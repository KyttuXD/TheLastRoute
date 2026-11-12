#!/bin/bash
# CI-only experiment: never edits the checkout's manifests or release lockfile.
set -euo pipefail
ROOT=$(git rev-parse --show-toplevel)
TOOLCHAIN=${1:?usage: build-experiment.sh BOOTSTRAP_DIRECTORY [10.6|10.5]}
# The system to build for: 10.6 (Snow Leopard, the default) or 10.5 (Leopard; needs bootstrap-leopard.sh run first).
# The 10.5 build uses the same compiler program with the standard library MacPorts built for Darwin 9 and a
# compatibility library built for 10.5; the compiler itself still runs with the Darwin 10 library.
OSVER=${2:-10.6}
case "$OSVER" in
    10.6) COMPILER="$TOOLCHAIN/compiler"; SUPPORT="$TOOLCHAIN/support/opt/local/lib" ;;
    10.5) COMPILER="$TOOLCHAIN/compiler-leopard"; SUPPORT="$TOOLCHAIN/support-leopard/opt/local/lib" ;;
    *) echo "usage: build-experiment.sh BOOTSTRAP_DIRECTORY [10.6|10.5]" >&2; exit 2 ;;
esac
test -x "$COMPILER/bin/rustc"
test -f "$SUPPORT/libMacportsLegacySystem.B.dylib"
export DYLD_LIBRARY_PATH="$TOOLCHAIN/support/opt/local/lib:$COMPILER/lib:/usr/lib"
export RUSTC="$COMPILER/bin/rustc"
export LR_MACOS_LEGACY=1
export CARGO_BUILD_JOBS=4
export CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback
export MACOSX_DEPLOYMENT_TARGET=$OSVER
# Keep experiment objects separate from the modern-compiler pass artifacts.
export CARGO_TARGET_DIR="$ROOT/target/pass/legacy-$OSVER"
export RUSTFLAGS="-L native=$SUPPORT -C link-arg=-lMacportsLegacySystem.B"
STAGE=$(mktemp -d "${RUNNER_TEMP:-/tmp}/lr-snow-source.XXXXXX")
git -C "$ROOT" archive HEAD | tar -xf - -C "$STAGE"
echo "Legacy build source copy: $STAGE"
# Modern Cargo resolves a dependency graph the legacy compiler can build (stage-dependencies.sh); the matching old
# Cargo then builds it, because it does not pass newer rustc command-line switches.
bash "$ROOT/packaging/macos/legacy/stage-dependencies.sh" "$STAGE"
mkdir -p "$ROOT/target/test-reports"
cp "$STAGE/Cargo.lock" "$ROOT/target/test-reports/legacy-$OSVER-dependencies.lock"
cd "$STAGE"
# Cargo itself only runs on the modern build host. RUSTC points at the patched
# compiler above, so its bundled upstream standard library is never selected.
rustup run 1.77.0 cargo build --locked --profile ci --target x86_64-apple-darwin --bin last_route
# An unsigned .app for a real Snow Leopard Mac to try. Built, never run: CI has no 10.6 machine.
BIN="$CARGO_TARGET_DIR/x86_64-apple-darwin/ci/last_route"
otool -L "$BIN" | tee "$ROOT/target/test-reports/legacy-$OSVER-libraries.txt"
otool -l "$BIN" | grep -A4 -E 'LC_VERSION_MIN_MACOSX|LC_BUILD_VERSION' | tee "$ROOT/target/test-reports/legacy-$OSVER-min-os.txt"
# every symbol the program asks the system for, for checking against what that system has (docs/MACOS_SNOW_LEOPARD.md)
nm -u "$BIN" | sort > "$ROOT/target/test-reports/legacy-$OSVER-imports.txt"
APP="$ROOT/dist/legacy-$OSVER/KYTTU The Last Route ($OSVER test).app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/last_route"
# the compatibility library the binary links (MacPorts legacy-support), beside the executable
cp "$SUPPORT/libMacportsLegacySystem.B.dylib" "$APP/Contents/MacOS/"
install_name_tool -change /opt/local/lib/libMacportsLegacySystem.B.dylib \
    @executable_path/libMacportsLegacySystem.B.dylib "$APP/Contents/MacOS/last_route"
sed '/<key>LSMinimumSystemVersion<\/key>/{n;s#<string>[^<]*</string>#<string>'"$OSVER"'</string>#;}' \
    "$ROOT/packaging/macos/Info.plist" > "$APP/Contents/Info.plist"
(cd "$ROOT/dist/legacy-$OSVER" && zip -qry "../last_route-macos-$OSVER-UNTESTED.zip" "$(basename "$APP")")
echo "Built dist/last_route-macos-$OSVER-UNTESTED.zip. It has never run on OS X $OSVER: graphics, audio and TLS are unverified."
