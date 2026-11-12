#!/bin/sh
# Builds the game for big-endian PowerPC Linux and, with `test`, runs its tests in the qemu user emulator
# (docs/POWERPC.md).
#   sh packaging/powerpc/build.sh [64|32]          builds target/<triple>/ci/last_route
#   sh packaging/powerpc/build.sh [64|32] suites   also runs the headless suites under qemu (about 4 minutes)
#   sh packaging/powerpc/build.sh [64|32] test     the unit tests under qemu too (about 35 minutes; one test that starts node fails there)
# Needs (Ubuntu names): gcc-powerpc64-linux-gnu or gcc-powerpc-linux-gnu, and qemu-user for `test`.
#   LR_PPC_SYSROOT   the folder holding the target's lib/ (default /usr/<gnu triple>), handed to qemu as its root
# The game links one library besides libc: ALSA. Ubuntu ships no big-endian PowerPC libasound, so the link is made
# against a stand-in with the same name and the 19 functions the game calls (each returns an error). The machine's own
# libasound.so.2 is used when the game runs there; under qemu the stand-in is loaded and the game runs without sound.
set -e
cd "$(dirname "$0")/../.."
case "${1:-64}" in
    64) T=powerpc64-unknown-linux-gnu; G=powerpc64-linux-gnu; Q=qemu-ppc64 ;;
    32) T=powerpc-unknown-linux-gnu; G=powerpc-linux-gnu; Q=qemu-ppc ;;
    *) echo "usage: build.sh [64|32] [suites|test]" >&2; exit 2 ;;
esac
GCC=$G-gcc
command -v "$GCC" >/dev/null 2>&1 || GCC=$(ls "$(dirname "$(command -v "$G-ld")")"/"$G"-gcc-[0-9]* 2>/dev/null | head -1)
[ -n "$GCC" ] || { echo "no $G-gcc found" >&2; exit 1; }
SYSROOT=${LR_PPC_SYSROOT:-/usr/$G}
V=$(echo "$T" | tr - _)
U=$(echo "$V" | tr a-z A-Z)
OUT=${CARGO_TARGET_DIR:-target}/$T
rustup target add "$T"

# the ALSA stand-in
STUB=$OUT/alsa-stub
mkdir -p "$STUB"
{
    for f in snd_pcm_hw_params snd_pcm_hw_params_any snd_pcm_hw_params_free snd_pcm_hw_params_malloc \
        snd_pcm_hw_params_set_access snd_pcm_hw_params_set_buffer_size snd_pcm_hw_params_set_channels \
        snd_pcm_hw_params_set_format snd_pcm_hw_params_set_rate_near snd_pcm_open snd_pcm_prepare snd_pcm_recover \
        snd_pcm_sw_params snd_pcm_sw_params_current snd_pcm_sw_params_free snd_pcm_sw_params_malloc \
        snd_pcm_sw_params_set_start_threshold snd_pcm_wait snd_pcm_writei; do
        echo "int $f(void) { return -1; }"
    done
} > "$STUB/asound.c"
"$GCC" -shared -fPIC -Wl,-soname,libasound.so.2 -o "$STUB/libasound.so.2" "$STUB/asound.c"
ln -sf libasound.so.2 "$STUB/libasound.so"

export "CC_$V=$GCC"
export "CARGO_TARGET_${U}_LINKER=$GCC"
export "CARGO_TARGET_${U}_RUSTFLAGS=-L $PWD/$STUB"
cargo build --profile ci --target "$T" --bin last_route
echo "built $OUT/ci/last_route"
[ "$2" = suites ] || [ "$2" = test ] || exit 0

# qemu finds the target's loader and libc under -L, and the ALSA stand-in through the library path
RUN="$Q -L $SYSROOT -E LD_LIBRARY_PATH=$PWD/$STUB:$SYSROOT/lib"
if [ "$2" = test ]; then
    cargo test --profile ci --target "$T" --no-run --lib --bin last_route
    TESTS=$(ls -t "$OUT"/ci/deps/last_route-* | grep -v '\.d$' | head -1)
    $RUN "$TESTS" --test-threads=4
fi
$RUN "$OUT/ci/last_route" --headless --test-suite "tests/*.json"
