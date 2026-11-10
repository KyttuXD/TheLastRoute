#!/bin/sh
# Deploys the KYTTU - LRS (lr_story) browser build to VPS 2 (static files over Caddy, /srv/lr-story-web).
# Usage: sh packaging/story-web/deploy-vps2.sh [FOLDER]        FOLDER defaults to dist/story-web
#
# FOLDER holds the built story (build it with packaging/story-web/build.sh): index.html, keys.js, audio.js, hw.js,
# mq_js_bundle.js, lr_story.wasm, manifest.json, vfs.bin and vfs.bin.gz (the game files, one bundle), version.json.
#
# What it does, in order:
#   1. writes FOLDER/version.json from the sha256 of lr_story.wasm: {"build":"<sha256>","built":"<iso time>"}
#   2. uploads the eight files by tar over ssh (extracted with --no-same-owner) into a fresh staging folder on the server,
#      then moved into place one by one, so a browser never reads a half-written file
#   3. uploads version.json LAST: open tabs see a new build only once every file of it is in place
#   4. verifies on the server the sha256 of lr_story.wasm and vfs.bin, then through the live URL: version.json names the
#      build, and the live wasm and bundle match
# It never touches the Caddyfile or anything else on the server.
# Environment: HOST (ssh alias, default vps2), REMOTE_DIR (default /srv/lr-story-web),
#   LIVE_URL (default https://lastroutestory.games.kyttuslab.org), DRY_RUN=1 (print the plan, write and upload nothing).
set -eu

DIR="${1:-dist/story-web}"
HOST="${HOST:-vps2}"
REMOTE_DIR="${REMOTE_DIR:-/srv/lr-story-web}"
LIVE_URL="${LIVE_URL:-https://lastroutestory.games.kyttuslab.org}"
LIVE_URL="${LIVE_URL%/}"
FILES="index.html keys.js audio.js hw.js mq_js_bundle.js lr_story.wasm manifest.json vfs.bin vfs.bin.gz"

fail() {
    echo "deploy-vps2: $*" >&2
    exit 1
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

sha256_stdin() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | cut -d' ' -f1
    else
        shasum -a 256 | cut -d' ' -f1
    fi
}

[ -d "$DIR" ] || fail "$DIR is not a folder"
for f in $FILES; do
    [ -f "$DIR/$f" ] || fail "$DIR/$f is missing (build the web game first)"
done

BUILD="$(sha256_of "$DIR/lr_story.wasm")"
BUNDLE_SHA="$(sha256_of "$DIR/vfs.bin")"
BUILT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
VERSION_JSON="{\"build\":\"$BUILD\",\"built\":\"$BUILT\"}"

echo "folder:  $DIR"
echo "server:  $HOST:$REMOTE_DIR"
echo "live:    $LIVE_URL"
echo "version: $VERSION_JSON"
echo "bundle:  vfs.bin $BUNDLE_SHA"
echo "uploads: $FILES, then version.json"

if [ "${DRY_RUN:-}" = "1" ]; then
    echo "DRY_RUN=1: nothing written, nothing uploaded. It would upload: $FILES, then version.json"
    exit 0
fi

printf '%s\n' "$VERSION_JSON" > "$DIR/version.json"

# the game files: extracted into a fresh staging folder on the server, then moved into place (same disk, so each move is atomic)
echo "uploading the game files..."
tar -C "$DIR" -cf - $FILES | ssh "$HOST" "set -e
mkdir -p '$REMOTE_DIR'
stage=\$(mktemp -d '$REMOTE_DIR/.stage.XXXXXX')
trap 'rm -rf \"\$stage\"' EXIT
tar -C \"\$stage\" --no-same-owner -xf -
for f in $FILES; do
    chmod 644 \"\$stage/\$f\"
    mv -f \"\$stage/\$f\" '$REMOTE_DIR/'\"\$f\"
done"

# the files are in place: check the wasm and the bundle on the server before anyone is told about it
REMOTE_SHA="$(ssh "$HOST" "sha256sum '$REMOTE_DIR/lr_story.wasm'" | cut -d' ' -f1)"
[ "$REMOTE_SHA" = "$BUILD" ] || fail "the server's lr_story.wasm is $REMOTE_SHA, expected $BUILD; version.json was NOT uploaded"
REMOTE_BUNDLE="$(ssh "$HOST" "sha256sum '$REMOTE_DIR/vfs.bin'" | cut -d' ' -f1)"
[ "$REMOTE_BUNDLE" = "$BUNDLE_SHA" ] || fail "the server's vfs.bin is $REMOTE_BUNDLE, expected $BUNDLE_SHA; version.json was NOT uploaded"

# version.json last: from here open tabs see the new build and offer the update
echo "uploading version.json..."
tar -C "$DIR" -cf - version.json | ssh "$HOST" "set -e
mkdir -p '$REMOTE_DIR'
stage=\$(mktemp -d '$REMOTE_DIR/.stage.XXXXXX')
trap 'rm -rf \"\$stage\"' EXIT
tar -C \"\$stage\" --no-same-owner -xf -
chmod 644 \"\$stage/version.json\"
mv -f \"\$stage/version.json\" '$REMOTE_DIR/version.json'"

# the live URL serves what was uploaded. The bundle is asked for without Accept-Encoding, so it is compared as the
# browser's decoded bytes (the server sends vfs.bin.gz only to a client that accepts gzip).
echo "verifying $LIVE_URL ..."
LIVE_JSON="$(curl -fsS "$LIVE_URL/version.json?t=$(date +%s)")" || fail "$LIVE_URL/version.json did not answer"
case "$LIVE_JSON" in
    *"$BUILD"*) ;;
    *) fail "the live version.json does not name build $BUILD: $LIVE_JSON" ;;
esac
curl -fsS -o /dev/null "$LIVE_URL/" || fail "$LIVE_URL/ did not answer"
LIVE_SHA="$(curl -fsS "$LIVE_URL/lr_story.wasm?t=$(date +%s)" | sha256_stdin)"
[ "$LIVE_SHA" = "$BUILD" ] || fail "the live lr_story.wasm is $LIVE_SHA, expected $BUILD"
LIVE_BUNDLE="$(curl -fsS "$LIVE_URL/vfs.bin?t=$(date +%s)" | sha256_stdin)"
[ "$LIVE_BUNDLE" = "$BUNDLE_SHA" ] || fail "the live vfs.bin is $LIVE_BUNDLE, expected $BUNDLE_SHA"

echo "deployed: build $BUILD at $BUILT"
