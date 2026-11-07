#!/bin/sh
# Builds the app on a Mac over ssh, from any shell that has the ssh alias (Windows Git Bash included).
#   sh packaging/macos/webapp/remote-build.sh [HOST]          HOST defaults to KT-MBP
# Copies this folder (and the logo PNGs for the icon) to ~/LastRouteApp/src on the Mac, runs build-app.sh there and leaves
# ~/LastRouteApp/KYTTU The Last Route.app. Nothing else on the Mac is touched. FETCH=1 also copies the app back to ./dist.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
HOST="${1:-KT-MBP}"
REMOTE="${REMOTE:-LastRouteApp}"
BRANDING="$HERE/../../../src/bin/last_route/assets/branding"

ssh "$HOST" "mkdir -p \"\$HOME/$REMOTE/src/icon\""
tar -C "$HERE" --exclude=dist --exclude=web-bundle --exclude=icon -cf - . | ssh "$HOST" "tar -xf - -C \"\$HOME/$REMOTE/src\""
tar -C "$BRANDING" -cf - logo_16.png logo_32.png logo_64.png logo_128.png logo_256.png | ssh "$HOST" "tar -xf - -C \"\$HOME/$REMOTE/src/icon\""
ssh "$HOST" "cd \"\$HOME/$REMOTE/src\" && sh build-app.sh \"\$HOME/$REMOTE\""
if [ "${FETCH:-0}" = 1 ]; then
    mkdir -p "$HERE/dist"
    ssh "$HOST" "cd \"\$HOME/$REMOTE\" && tar -cf - \"KYTTU The Last Route.app\"" | tar -C "$HERE/dist" -xf -
    echo "copied to $HERE/dist/KYTTU The Last Route.app"
fi
