#!/bin/sh
# Builds the browser build of KYTTU - LRS (lr_story) into dist/story-web: sh packaging/story-web/build.sh
#
# What it does, in order:
#   1. cargo build --release --target wasm32-unknown-unknown --bin lr_story (CARGO_BUILD_JOBS defaults to 4)
#   2. node packaging/story-web/assemble.mjs: copies the wasm to dist/story-web/lr_story.wasm, writes manifest.json (the
#      file list from the pack folders and lr_story's assets), packs those files into dist/story-web/vfs.bin (and its
#      gzip copy vfs.bin.gz), and copies
#      the page files (index.html, mq_js_bundle.js, audio.js, hw.js, keys.js) from this folder
#   3. writes dist/story-web/version.json: {"build":"<sha256 of lr_story.wasm>","built":"<iso time>"}
#
# The page files are copied from packaging/story-web and the packaging/web copies of mq_js_bundle.js, keys.js, audio.js
# and hw.js. This script never touches a server; deploy-vps2.sh does that, and only when run by hand.
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OUT="$ROOT/dist/story-web"
WASM="$ROOT/target/wasm32-unknown-unknown/release/lr_story.wasm"

fail() {
    echo "build-story-web: $*" >&2
    exit 1
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

command -v node >/dev/null 2>&1 || fail "node is needed (assemble.mjs)"

cd "$ROOT"
echo "building lr_story for wasm32 (release)..."
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" cargo build --release --target wasm32-unknown-unknown --bin lr_story

[ -f "$WASM" ] || fail "$WASM was not built"

echo "assembling $OUT ..."
node "$HERE/assemble.mjs" "$WASM" "$OUT"

[ -f "$OUT/lr_story.wasm" ] || fail "$OUT/lr_story.wasm is missing after assembly"
BUILD="$(sha256_of "$OUT/lr_story.wasm")"
BUILT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
printf '{"build":"%s","built":"%s"}\n' "$BUILD" "$BUILT" > "$OUT/version.json"

echo "built: $OUT"
echo "wasm sha256: $BUILD"
