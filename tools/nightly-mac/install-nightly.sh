#!/bin/sh
# Installs the update-on-start launcher on this Mac: "LastRoute Playtest.app" on the Desktop.
# Opening it checks the VPS for a newer build, then starts the game. Nothing runs on a timer.
# Run from this folder:  sh install-nightly.sh
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
sh "$DIR/make-launcher.sh"
