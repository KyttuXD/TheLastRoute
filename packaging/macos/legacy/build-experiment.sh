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
# Modern Cargo understands newer lockfiles/manifests and can select dependency
# versions compatible with the active legacy rustc. Build with matching old
# Cargo afterward, which does not inject newer rustc command-line switches.
cargo +1.99.1 update --manifest-path "$STAGE/Cargo.toml"
python3 - "$STAGE/Cargo.lock" <<'PY'
import pathlib
import sys
lock = pathlib.Path(sys.argv[1])
text = lock.read_text()
assert 'version = 4\n' in text or 'version = 3\n' in text
lock.write_text(text.replace('version = 4\n', 'version = 3\n', 1))
PY
mkdir -p "$ROOT/target/test-reports"
cp "$STAGE/Cargo.lock" "$ROOT/target/test-reports/snow-leopard-dependencies.lock"
cd "$STAGE"
# Cargo itself only runs on the modern build host. RUSTC points at the patched
# compiler above, so its bundled upstream standard library is never selected.
rustup run 1.77.0 cargo build --locked --profile ci --target x86_64-apple-darwin --bin last_route
echo "Experimental game compiled. Not packaged: OS/runtime/graphics compatibility still requires validation."
