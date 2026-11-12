#!/bin/bash
# CI-only experiment: never edits the checkout's manifests or release lockfile.
set -euo pipefail
ROOT=$(git rev-parse --show-toplevel)
TOOLCHAIN=${1:?usage: build-experiment.sh BOOTSTRAP_DIRECTORY}
test -x "$TOOLCHAIN/compiler/bin/rustc"
export DYLD_LIBRARY_PATH="$TOOLCHAIN/support/opt/local/lib:$TOOLCHAIN/compiler/lib:/usr/lib"
export RUSTC="$TOOLCHAIN/compiler/bin/rustc"
export LR_MACOS_LEGACY=1
export CARGO_BUILD_JOBS=4
export CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback
export MACOSX_DEPLOYMENT_TARGET=10.6
# Keep experiment objects separate from the modern-compiler pass artifacts.
export CARGO_TARGET_DIR="$ROOT/target/pass/snow-leopard"
export RUSTFLAGS="-L native=$TOOLCHAIN/support/opt/local/lib -C link-arg=-lMacportsLegacySystem.B"
STAGE=$(mktemp -d "${RUNNER_TEMP:-/tmp}/lr-snow-source.XXXXXX")
git -C "$ROOT" archive HEAD | tar -xf - -C "$STAGE"
echo "Legacy build source copy: $STAGE"
# Modern Cargo resolves a dependency graph the legacy compiler can build (stage-dependencies.sh); the matching old
# Cargo then builds it, because it does not pass newer rustc command-line switches.
bash "$ROOT/packaging/macos/legacy/stage-dependencies.sh" "$STAGE"
mkdir -p "$ROOT/target/test-reports"
cp "$STAGE/Cargo.lock" "$ROOT/target/test-reports/snow-leopard-dependencies.lock"
cd "$STAGE"
# Cargo itself only runs on the modern build host. RUSTC points at the patched
# compiler above, so its bundled upstream standard library is never selected.
rustup run 1.77.0 cargo build --locked --profile ci --target x86_64-apple-darwin --bin last_route
# An unsigned .app for a real Snow Leopard Mac to try. Built, never run: CI has no 10.6 machine.
BIN="$CARGO_TARGET_DIR/x86_64-apple-darwin/ci/last_route"
otool -L "$BIN" | tee "$ROOT/target/test-reports/snow-leopard-libraries.txt"
otool -l "$BIN" | grep -A4 -E 'LC_VERSION_MIN_MACOSX|LC_BUILD_VERSION' | tee "$ROOT/target/test-reports/snow-leopard-min-os.txt"
APP="$ROOT/dist/snow-leopard/KYTTU The Last Route (10.6 test).app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/last_route"
# the compatibility library the binary links (MacPorts legacy-support), beside the executable
cp "$TOOLCHAIN/support/opt/local/lib/libMacportsLegacySystem.B.dylib" "$APP/Contents/MacOS/"
install_name_tool -change /opt/local/lib/libMacportsLegacySystem.B.dylib \
    @executable_path/libMacportsLegacySystem.B.dylib "$APP/Contents/MacOS/last_route"
sed '/<key>LSMinimumSystemVersion<\/key>/{n;s#<string>[^<]*</string>#<string>10.6</string>#;}' \
    "$ROOT/packaging/macos/Info.plist" > "$APP/Contents/Info.plist"
(cd "$ROOT/dist/snow-leopard" && zip -qry ../last_route-macos-10.6-UNTESTED.zip "$(basename "$APP")")
echo "Built dist/last_route-macos-10.6-UNTESTED.zip. It has never run on 10.6.8: graphics, audio and TLS are unverified."
