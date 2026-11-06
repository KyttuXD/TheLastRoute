#!/bin/sh
# Deploys the browser build to VPS 2 (static files over Caddy, /srv/lr-rust-web).
# Usage: sh packaging/web/deploy-vps2.sh [FOLDER]        FOLDER defaults to dist/web
#
# FOLDER holds the built game: index.html, update.js, keys.js, audio.js, mq_js_bundle.js and last_route.wasm.
#
# What it does, in order:
#   1. writes FOLDER/version.json from the sha256 of last_route.wasm: {"build":"<sha256>","built":"<iso time>"}
#   2. uploads the six game files by tar over ssh (extracted with --no-same-owner), into a staging folder on the
#      server and then moved into place one by one, so a browser never reads a half-written file
#   3. uploads version.json LAST: open game tabs poll it (update.js), so they only see a new version once every file
#      they will reload is already in place
#   4. verifies the wasm sha256 on the server and through the live URL, and that the live version.json names it
#
# It never touches the Caddyfile or anything else on the server.
# Environment: HOST (ssh alias, default vps2), REMOTE_DIR (default /srv/lr-rust-web),
#   LIVE_URL (default https://lastrouterust.games.kyttuslab.org), DRY_RUN=1 (print the plan, write and upload nothing).
set -eu

DIR="${1:-dist/web}"
HOST="${HOST:-vps2}"
REMOTE_DIR="${REMOTE_DIR:-/srv/lr-rust-web}"
LIVE_URL="${LIVE_URL:-https://lastrouterust.games.kyttuslab.org}"
LIVE_URL="${LIVE_URL%/}"
FILES="index.html update.js keys.js audio.js mq_js_bundle.js last_route.wasm"

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

[ -d "$DIR" ] || fail "$DIR is not a folder"
for f in $FILES; do
    [ -f "$DIR/$f" ] || fail "$DIR/$f is missing (build the web game first)"
done

BUILD="$(sha256_of "$DIR/last_route.wasm")"
BUILT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
VERSION_JSON="{\"build\":\"$BUILD\",\"built\":\"$BUILT\"}"

echo "folder:  $DIR"
echo "server:  $HOST:$REMOTE_DIR"
echo "live:    $LIVE_URL"
echo "version: $VERSION_JSON"

if [ "${DRY_RUN:-}" = "1" ]; then
    echo "DRY_RUN=1: nothing written, nothing uploaded. It would upload: $FILES, then version.json"
    exit 0
fi

printf '%s\n' "$VERSION_JSON" > "$DIR/version.json"

# the game files: extracted into a staging folder on the server, then moved into place (same disk, so each move is atomic)
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

# the files are in place: check the wasm on the server before anyone is told about it
REMOTE_SHA="$(ssh "$HOST" "sha256sum '$REMOTE_DIR/last_route.wasm'" | cut -d' ' -f1)"
[ "$REMOTE_SHA" = "$BUILD" ] || fail "the server's last_route.wasm is $REMOTE_SHA, expected $BUILD; version.json was NOT uploaded"

# version.json last: from here open tabs see the new build and offer the update
echo "uploading version.json..."
tar -C "$DIR" -cf - version.json | ssh "$HOST" "set -e
mkdir -p '$REMOTE_DIR'
stage=\$(mktemp -d '$REMOTE_DIR/.stage.XXXXXX')
trap 'rm -rf \"\$stage\"' EXIT
tar -C \"\$stage\" --no-same-owner -xf -
chmod 644 \"\$stage/version.json\"
mv -f \"\$stage/version.json\" '$REMOTE_DIR/version.json'"

# the live URL serves what was uploaded
echo "verifying $LIVE_URL ..."
LIVE_JSON="$(curl -fsS "$LIVE_URL/version.json?t=$(date +%s)")" || fail "$LIVE_URL/version.json did not answer"
case "$LIVE_JSON" in
    *"$BUILD"*) ;;
    *) fail "the live version.json does not name build $BUILD: $LIVE_JSON" ;;
esac
curl -fsS -o /dev/null "$LIVE_URL/" || fail "$LIVE_URL/ did not answer"
LIVE_SHA="$(curl -fsS "$LIVE_URL/last_route.wasm?t=$(date +%s)" | { if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi; } | cut -d' ' -f1)"
[ "$LIVE_SHA" = "$BUILD" ] || fail "the live last_route.wasm is $LIVE_SHA, expected $BUILD"

echo "deployed: build $BUILD at $BUILT"
