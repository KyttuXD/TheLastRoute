#!/bin/sh
# Installs the nightly updater on this Mac and starts it every two hours (and at login).
# Run from this folder:  sh install-nightly.sh
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
SUPPORT="$HOME/Library/Application Support/LastRoutePlaytest"
AGENTS="$HOME/Library/LaunchAgents"
mkdir -p "$SUPPORT" "$AGENTS"
cp "$DIR/update-nightly.sh" "$SUPPORT/update-nightly.sh"
sed "s#__HOME__#$HOME#g" "$DIR/com.kyttu.lastroute.nightly.plist" > "$AGENTS/com.kyttu.lastroute.nightly.plist"
launchctl unload "$AGENTS/com.kyttu.lastroute.nightly.plist" 2>/dev/null || true
launchctl load "$AGENTS/com.kyttu.lastroute.nightly.plist"
echo "Nightly updater installed. Log: $SUPPORT/nightly.log"
