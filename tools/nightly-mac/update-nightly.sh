#!/bin/sh
# Keeps "KYTTU The Last Route.app" in /Applications up to date with the newest macOS build published to the VPS.
# Reads the public manifest, downloads the file with the download key, checks its SHA-256, then installs it.
# Run by launchd (com.kyttu.lastroute.nightly.plist). Needs only curl, unzip and shasum (all present on macOS).
# One-time setup: save the download key to $KEY_FILE (chmod 600). It is never printed or logged.
BASE="http://lastroute.kyttuslab.org:8105/api/updates"
CHANNEL="debug"
PLATFORM="macos"
APP="/Applications/KYTTU The Last Route.app"
STATE_DIR="$HOME/Library/Application Support/LastRoutePlaytest"
KEY_FILE="$STATE_DIR/download-key"
LOG="$STATE_DIR/nightly.log"
mkdir -p "$STATE_DIR"

log() { echo "$(date '+%Y-%m-%dT%H:%M:%S') $*" >> "$LOG"; }

if [ ! -s "$KEY_FILE" ]; then
    log "no download key: save it to $KEY_FILE"
    exit 1
fi
KEY="$(cat "$KEY_FILE")"

MANIFEST="$(curl -fsS -m 30 "$BASE/$CHANNEL/$PLATFORM" 2>/dev/null | tr -d '\n')"
if [ -z "$MANIFEST" ]; then
    log "could not read the update manifest (offline, or the server is down)"
    exit 1
fi
FILE="$(printf '%s' "$MANIFEST" | sed -n 's/.*"file":"\([^"]*\)".*/\1/p')"
SHA="$(printf '%s' "$MANIFEST" | sed -n 's/.*"sha256":"\([0-9a-f]*\)".*/\1/p')"
BUILD="$(printf '%s' "$MANIFEST" | sed -n 's/.*"build":\([0-9]*\).*/\1/p')"
if [ -z "$FILE" ] || [ -z "$SHA" ] || [ -z "$BUILD" ]; then
    log "the manifest has no $PLATFORM build yet"
    exit 1
fi

if [ "$BUILD" = "$(cat "$STATE_DIR/nightly.build" 2>/dev/null)" ] && [ -d "$APP" ]; then
    exit 0
fi

WORK="$(mktemp -d)"
if ! curl -fsSL -m 900 -H "X-Download-Key: $KEY" "$BASE/files/$FILE" -o "$WORK/$FILE"; then
    log "download of $FILE failed (check the download key)"
    rm -rf "$WORK"
    exit 1
fi
GOT="$(shasum -a 256 "$WORK/$FILE" | cut -d' ' -f1)"
if [ "$GOT" != "$SHA" ]; then
    log "checksum mismatch for $FILE: not installed"
    rm -rf "$WORK"
    exit 1
fi
if ! unzip -q "$WORK/$FILE" -d "$WORK/unpacked"; then
    log "the download did not unpack"
    rm -rf "$WORK"
    exit 1
fi
NEW="$WORK/unpacked/KYTTU The Last Route.app"
if [ ! -x "$NEW/Contents/MacOS/last_route" ]; then
    log "the unpacked app has no game binary"
    rm -rf "$WORK"
    exit 1
fi
xattr -cr "$NEW" 2>/dev/null

# move the old app aside, put the new one in its place, then delete the old one (no backup is kept)
rm -rf "$APP.previous"
[ -d "$APP" ] && mv "$APP" "$APP.previous"
if mv "$NEW" "$APP"; then
    rm -rf "$APP.previous"
    echo "$BUILD" > "$STATE_DIR/nightly.build"
    log "updated the app to build $BUILD ($FILE)"
else
    log "could not move the new app into /Applications; restoring the previous one"
    [ -d "$APP.previous" ] && mv "$APP.previous" "$APP"
fi
rm -rf "$WORK"
