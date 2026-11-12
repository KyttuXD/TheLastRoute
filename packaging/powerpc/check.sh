#!/bin/sh
# Type-checks the game for big-endian PowerPC Linux (docs/POWERPC.md). It does not link or run anything.
#   sh packaging/powerpc/check.sh            64-bit (G5, POWER): passes
#   sh packaging/powerpc/check.sh 32         32-bit (G3/G4): passes
# ring's build script wants a C compiler for the target. A check never links its objects, so the host gcc stands in
# when no powerpc gcc is installed. A real build needs gcc-powerpc64-linux-gnu (or gcc-powerpc-linux-gnu).
set -e
cd "$(dirname "$0")/../.."
case "${1:-64}" in
    64) T=powerpc64-unknown-linux-gnu; V=powerpc64_unknown_linux_gnu; GCC=powerpc64-linux-gnu-gcc ;;
    32) T=powerpc-unknown-linux-gnu; V=powerpc_unknown_linux_gnu; GCC=powerpc-linux-gnu-gcc ;;
    *) echo "usage: check.sh [64|32]" >&2; exit 2 ;;
esac
rustup target add "$T"
if ! command -v "$GCC" >/dev/null 2>&1; then
    export "CC_$V=gcc"
    # the host gcc has no 32-bit libraries; the objects are never linked, so build them as 64-bit
    [ "${1:-64}" = 32 ] && export "CFLAGS_$V=-m64"
fi
cargo check --target "$T" --bin last_route
