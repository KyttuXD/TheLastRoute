#!/bin/sh
# Makes "LastRoute Playtest.app" on the Desktop. Opening it checks the VPS for a newer build first, then starts
# the game. There is no timer: the check happens only when you start the game.
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
SUPPORT="$HOME/Library/Application Support/LastRoutePlaytest"
mkdir -p "$SUPPORT"
cp "$DIR/update-nightly.sh" "$SUPPORT/update-nightly.sh"
SCRIPT="$(mktemp)"
cat > "$SCRIPT" <<APPLESCRIPT
do shell script "sh \"$SUPPORT/update-nightly.sh\" >/dev/null 2>&1 || true"
tell application "KYTTU The Last Route" to activate
APPLESCRIPT
osacompile -o "$HOME/Desktop/LastRoute Playtest.app" "$SCRIPT"
rm -f "$SCRIPT"
launchctl unload "$HOME/Library/LaunchAgents/com.kyttu.lastroute.nightly.plist" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/com.kyttu.lastroute.nightly.plist"
echo "Launcher on the Desktop: LastRoute Playtest.app (checks for an update on start)"
