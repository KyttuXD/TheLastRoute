#!/bin/bash
# CI-only experiment: never edits the checkout's manifests or release lockfile.
set -euo pipefail
ROOT=$(git rev-parse --show-toplevel)
TOOLCHAIN=${1:?usage: build-experiment.sh BOOTSTRAP_DIRECTORY [10.6|10.5]}
# The system to build for: 10.5 (Leopard, the default; needs bootstrap-leopard.sh run first) or 10.6.
# The 10.5 build is THE legacy app: one program for OS X 10.5 up to 10.11. It uses the same compiler program with the
# standard library MacPorts built for Darwin 9 and a compatibility library built for 10.5; the compiler itself still
# runs with the Darwin 10 library. The 10.6 target is kept for comparison only and does not run on 10.6: the current
# linker writes it in the LC_MAIN program format, which arrived in 10.8 (seen on a Mac Pro 3,1: it dies in dyld).
OSVER=${2:-10.5}
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
# Flat namespace: the linker on a current Mac finds clock_gettime, getentropy and the like in its own libSystem and
# would tie each one to libSystem by name. The old system's libSystem does not have them, and the program would die at
# the first call (the 10.6 test app did, on a Mac Pro 3,1). With a flat namespace the old system looks in every loaded
# library and finds them in libMacportsLegacySystem.
export RUSTFLAGS="-L native=$SUPPORT -C link-arg=-lMacportsLegacySystem.B -C link-arg=-Wl,-flat_namespace"
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
# LR_LEGACY_PROFILE: release (the default: fat LTO, the fastest program, what is shipped) or ci (quicker to build,
# keeps line numbers in a backtrace: use it to chase a crash).
PROFILE=${LR_LEGACY_PROFILE:-release}
rustup run 1.77.0 cargo build --locked --profile "$PROFILE" --target x86_64-apple-darwin --bin last_route
# An unsigned .app for a real Snow Leopard Mac to try. Built, never run: CI has no 10.6 machine.
BIN="$CARGO_TARGET_DIR/x86_64-apple-darwin/$PROFILE/last_route"
otool -L "$BIN" | tee "$ROOT/target/test-reports/legacy-$OSVER-libraries.txt"
otool -l "$BIN" | grep -A4 -E 'LC_VERSION_MIN_MACOSX|LC_BUILD_VERSION' | tee "$ROOT/target/test-reports/legacy-$OSVER-min-os.txt"
# every symbol the program asks the system for, for checking against what that system has (docs/MACOS_SNOW_LEOPARD.md)
python3 "$ROOT/packaging/macos/legacy/machobind.py" "$BIN" > "$ROOT/target/test-reports/legacy-$OSVER-imports.txt"
head -n 1 "$ROOT/target/test-reports/legacy-$OSVER-imports.txt"
if ! head -n 1 "$ROOT/target/test-reports/legacy-$OSVER-imports.txt" | grep -q FLAT; then
    echo "the program is not linked with a flat namespace: it would not find the compatibility library's functions" >&2
    exit 1
fi
# the program format must be the old one (LC_UNIXTHREAD, load command 5): OS X before 10.8 cannot start LC_MAIN
python3 -c 'import struct,sys;d=open(sys.argv[1],"rb").read();n=struct.unpack_from("<I",d,16)[0];o=32;c=[]
for _ in range(n):
    k,s=struct.unpack_from("<II",d,o);c.append(k);o+=s
print("program format:", "LC_UNIXTHREAD (runs on 10.5+)" if 5 in c else "LC_MAIN (needs 10.8+)")' "$BIN" | tee "$ROOT/target/test-reports/legacy-$OSVER-format.txt"
if [ "$OSVER" = 10.5 ] && ! grep -q LC_UNIXTHREAD "$ROOT/target/test-reports/legacy-$OSVER-format.txt"; then
    echo "the legacy program is in a format OS X 10.5 to 10.7 cannot start" >&2
    exit 1
fi
if [ "$OSVER" = 10.5 ]; then NAME=legacy; else NAME=$OSVER; fi
APP="$ROOT/dist/legacy-$OSVER/KYTTU The Last Route ($NAME).app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/last_route"
# the compatibility library the binary links (MacPorts legacy-support), beside the executable
cp "$SUPPORT/libMacportsLegacySystem.B.dylib" "$APP/Contents/MacOS/"
install_name_tool -change /opt/local/lib/libMacportsLegacySystem.B.dylib \
    @executable_path/libMacportsLegacySystem.B.dylib "$APP/Contents/MacOS/last_route"
sed '/<key>LSMinimumSystemVersion<\/key>/{n;s#<string>[^<]*</string>#<string>'"$OSVER"'</string>#;}' \
    "$ROOT/packaging/macos/Info.plist" > "$APP/Contents/Info.plist"
(cd "$ROOT/dist/legacy-$OSVER" && zip -qry "../last_route-macos-$NAME.zip" "$(basename "$APP")")
echo "Built dist/last_route-macos-$NAME.zip for OS X $OSVER and later."
