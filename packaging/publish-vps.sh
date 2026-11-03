#!/bin/sh
# Publishes one nightly build to the owner's VPS, where the game's updater downloads it.
# Usage: sh packaging/publish-vps.sh PLATFORM FILE
#   PLATFORM: linux, windows or macos. Needs NIGHTLY_SSH_KEY and NIGHTLY_KNOWN_HOSTS (repo secrets), and
#   GITHUB_SHA and GITHUB_RUN_NUMBER (set by GitHub Actions). Only the upload user can write; nothing else is opened.
set -e
PLATFORM="$1"
FILE="$2"
HOST="194.156.90.219"
UPLOAD="nightly-upload@$HOST"
DIR="/var/lib/lastroute-rust/updates"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
printf '%s\n' "$NIGHTLY_SSH_KEY" > "$WORK/key"
printf '%s\n' "$NIGHTLY_KNOWN_HOSTS" > "$WORK/known_hosts"
chmod 600 "$WORK/key"
SSH_OPTS="-i $WORK/key -o UserKnownHostsFile=$WORK/known_hosts -o StrictHostKeyChecking=yes -o BatchMode=yes"

NAME="$(basename "$FILE")"
if command -v sha256sum >/dev/null 2>&1; then
    SHA="$(sha256sum "$FILE" | cut -d' ' -f1)"
else
    SHA="$(shasum -a 256 "$FILE" | cut -d' ' -f1)"
fi
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
BUILD="$GITHUB_RUN_NUMBER"
REVISION="$GITHUB_SHA"

# upload to a temporary name, then move it into place so a download never sees a partial file
scp $SSH_OPTS "$FILE" "$UPLOAD:$DIR/.$NAME.part"
ssh $SSH_OPTS "$UPLOAD" "mv '$DIR/.$NAME.part' '$DIR/$NAME'"
ssh $SSH_OPTS "$UPLOAD" "flock '$DIR/.manifest.lock' node - debug $PLATFORM '$NAME' $VERSION $BUILD $REVISION $SHA" < packaging/update-manifest.mjs
echo "published $NAME to the VPS ($PLATFORM, build $BUILD, sha256 $SHA)"
