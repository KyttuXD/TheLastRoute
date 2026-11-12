#!/bin/sh
# Run on the target Intel Mac with a MacPorts-patched Rust compiler.
# No Cargo dependency resolution and no changes to the normal game build.
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
RUSTC=${LEGACY_RUSTC:-/opt/local/bin/rustc}
if [ "$(uname -s)" != Darwin ]; then
    echo "Run this probe on the Intel macOS target, not Linux." >&2
    exit 1
fi
if [ ! -x "$RUSTC" ]; then
    echo "Missing MacPorts Rust: $RUSTC (override with LEGACY_RUSTC)." >&2
    exit 1
fi
sw_vers
"$RUSTC" -Vv
OUT=$(mktemp -d "${TMPDIR:-/tmp}/lr-runtime-probe.XXXXXX")
trap 'rm -f "$OUT/probe"; rmdir "$OUT"' EXIT
MACOSX_DEPLOYMENT_TARGET=10.6 "$RUSTC" --edition=2021 --target x86_64-apple-darwin \
    "$HERE/probe.rs" -o "$OUT/probe"
file "$OUT/probe"
otool -L "$OUT/probe"
"$OUT/probe"
