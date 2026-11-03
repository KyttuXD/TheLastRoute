#!/bin/sh
# Keeps "KYTTU The Last Route.app" in /Applications on this Mac up to date with the nightly macOS Intel build.
# Uses curl and the GitHub API, so the GitHub CLI is not needed. Run by launchd (com.kyttu.lastroute.nightly.plist).
# One-time setup: create a fine-grained GitHub token with read-only "Contents" access to KyttuXD/TheLastRoute only,
# and save it to the file TOKEN_FILE below (chmod 600). The token is never printed or logged.
REPO="KyttuXD/TheLastRoute"
ASSET="last_route-macos-x86_64.zip"
APP="/Applications/KYTTU The Last Route.app"
STATE_DIR="$HOME/Library/Application Support/LastRoutePlaytest"
TOKEN_FILE="$STATE_DIR/github-token"
LOG="$STATE_DIR/nightly.log"
mkdir -p "$STATE_DIR"

log() { echo "$(date '+%Y-%m-%dT%H:%M:%S') $*" >> "$LOG"; }

if [ ! -s "$TOKEN_FILE" ]; then
    log "no token: save a read-only GitHub token to $TOKEN_FILE"
    exit 1
fi
TOKEN="$(cat "$TOKEN_FILE")"
API="https://api.github.com/repos/$REPO/releases/tags/nightly"
AUTH="Authorization: Bearer $TOKEN"

# the release JSON as one line; the asset block for our file holds its id and its updated_at time
RELEASE="$(curl -fsSL -H "$AUTH" -H "Accept: application/vnd.github+json" "$API" 2>/dev/null | tr -d '\n')"
if [ -z "$RELEASE" ]; then
    log "could not read the nightly release (offline, or the token is wrong)"
    exit 1
fi
ASSET_ID="$(printf '%s' "$RELEASE" | grep -o "\"id\": *[0-9]*[^{}]*\"name\": *\"$ASSET\"" | grep -o '[0-9]*' | head -n 1)"
STAMP="$(printf '%s' "$RELEASE" | grep -o "\"updated_at\": *\"[^\"]*\"[^{}]*\"name\": *\"$ASSET\"" | sed -E 's/.*"updated_at": *"([^"]*)".*/\1/' | head -n 1)"
if [ -z "$ASSET_ID" ] || [ -z "$STAMP" ]; then
    log "the nightly release has no $ASSET yet"
    exit 1
fi

CURRENT="$(cat "$STATE_DIR/nightly.stamp" 2>/dev/null)"
if [ "$STAMP" = "$CURRENT" ] && [ -d "$APP" ]; then
    exit 0
fi

WORK="$(mktemp -d)"
if ! curl -fsSL -H "$AUTH" -H "Accept: application/octet-stream" \
        "https://api.github.com/repos/$REPO/releases/assets/$ASSET_ID" -o "$WORK/$ASSET"; then
    log "download of $ASSET failed"
    rm -rf "$WORK"
    exit 1
fi
if ! unzip -q "$WORK/$ASSET" -d "$WORK/unpacked"; then
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

# keep the previous app as a backup, then move the new one into place
rm -rf "$APP.previous"
[ -d "$APP" ] && mv "$APP" "$APP.previous"
if mv "$NEW" "$APP"; then
    echo "$STAMP" > "$STATE_DIR/nightly.stamp"
    log "updated the app to nightly $STAMP"
else
    log "could not move the new app into /Applications; restoring the previous one"
    [ -d "$APP.previous" ] && mv "$APP.previous" "$APP"
fi
rm -rf "$WORK"
