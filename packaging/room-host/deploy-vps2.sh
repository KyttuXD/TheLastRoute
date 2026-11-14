#!/bin/bash
# Installs the Linux build of last_route (the Rust room host) on VPS 2: sh packaging/room-host/deploy-vps2.sh BINARY
#   BINARY  last_route built for linux x86_64 (the CI tarball last-route-linux-x86_64.tar.gz, extracted). It MUST come from
#           the same commit as the web build that is served (the protocol fingerprint depends on it): docs/DEPLOY_ROOM_HOST.md.
#
# Default is a DRY RUN: it prints the plan and every remote command and changes nothing, on this PC or on the server
# (it does not even contact the server). Set DRY_RUN=0 to do it. NOTHING here is approved to run: KT approves the exact steps.
#
# What DRY_RUN=0 does, in order (running it twice with the same binary changes nothing the second time):
#   1. uploads BINARY to /opt/lastroute-rust/releases/<sha256>/last_route through a staging file, and checks the SHA-256 on the
#      server. A release folder that already holds a file with that SHA-256 is left alone (no upload).
#   2. records the release `current` points at now as `previous` (unless it is already the new one), then flips `current`
#      atomically (ln -sfn to current.new, then mv -T). Open rooms keep running the old file: nothing is restarted.
#   3. prints the rollback line.
# It restarts NOTHING unless RESTART=1. Then it restarts lastroute-region-agent (so the next spawn is sure to use the new file)
# and prints the warning first: every room hosted by the agent at that moment is dropped.
# The libraries (libasound.so.2, glibc 2.35 or newer) are only CHECKED (ldd on the server); nothing is installed.
# Environment: HOST (ssh alias, default vps2), DRY_RUN (default 1), RESTART (default 0), AGENT_UNIT (default lastroute-region-agent).
set -euo pipefail

BIN="${1:-}"
HOST="${HOST:-vps2}"
DRY_RUN="${DRY_RUN:-1}"
RESTART="${RESTART:-0}"
AGENT_UNIT="${AGENT_UNIT:-lastroute-region-agent}"
ROOT=/opt/lastroute-rust

fail() {
    echo "deploy-room-host: $*" >&2
    exit 1
}

[ -n "$BIN" ] || fail "usage: deploy-vps2.sh BINARY   (DRY_RUN=0 to do it, RESTART=1 to restart the agent)"
[ -f "$BIN" ] || fail "$BIN is not a file"
if command -v sha256sum >/dev/null 2>&1; then SHA="$(sha256sum "$BIN" | cut -d' ' -f1)"; else SHA="$(shasum -a 256 "$BIN" | cut -d' ' -f1)"; fi
case "$SHA" in *[!0-9a-f]* | "") fail "could not read the SHA-256 of $BIN" ;; esac
REL="$ROOT/releases/$SHA"

echo "binary:   $BIN ($SHA)"
echo "server:   $HOST:$ROOT"
echo "release:  $REL/last_route"
echo "restart:  $([ "$RESTART" = "1" ] && echo "YES, $AGENT_UNIT" || echo no)"

if [ "$DRY_RUN" != "0" ]; then
    echo
    echo "DRY RUN (default): nothing uploaded, nothing changed, the server was not contacted. It would run on $HOST:"
    echo "  ldd $ROOT/current/last_route or the uploaded file: must show no 'not found' (libasound.so.2, glibc >= 2.35)"
    echo "  mkdir -p $ROOT/releases; upload to $ROOT/releases/.stage.<random> and check sha256sum = $SHA"
    echo "  mv it to $REL/last_route (chmod 755, owner root) unless that file already has this SHA-256"
    echo "  previous <- the release current points at; current <- $REL (ln -sfn current.new; mv -T)"
    [ "$RESTART" = "1" ] && echo "  systemctl restart $AGENT_UNIT   (open hosted rooms are dropped)"
    echo "Run again with DRY_RUN=0 to do it."
    exit 0
fi

command -v ssh >/dev/null 2>&1 || fail "ssh is not installed"

echo "checking the server..."
ssh "$HOST" "set -e
mkdir -p $ROOT/releases
if [ -f $REL/last_route ] && [ \"\$(sha256sum $REL/last_route | cut -d' ' -f1)\" = $SHA ]; then echo have; else echo missing; fi" >/tmp/.lr-room-host-have.$$ || fail "cannot reach $HOST"
HAVE="$(cat /tmp/.lr-room-host-have.$$)"
rm -f /tmp/.lr-room-host-have.$$

if [ "$HAVE" = "have" ]; then
    echo "release $SHA is already installed and verified: not uploading again"
else
    echo "uploading..."
    ssh "$HOST" "set -e
stage=\$(mktemp $ROOT/releases/.stage.XXXXXX)
trap 'rm -f \"\$stage\"' EXIT
cat > \"\$stage\"
[ \"\$(sha256sum \"\$stage\" | cut -d' ' -f1)\" = $SHA ] || { echo 'SHA-256 mismatch after upload' >&2; exit 1; }
mkdir -p $REL
chmod 755 \"\$stage\"
mv -f \"\$stage\" $REL/last_route
trap - EXIT
chown -R root:root $REL" <"$BIN" || fail "the upload or the SHA-256 check failed; nothing was switched"
fi

echo "checking the libraries on the server..."
MISSING="$(ssh "$HOST" "ldd $REL/last_route 2>&1 | grep -i 'not found' || true")"
[ -z "$MISSING" ] || fail "missing on the server (install is a change KT must approve; current was NOT switched): $MISSING"

echo "switching current..."
OLD="$(ssh "$HOST" "set -e
old=\$(readlink $ROOT/current 2>/dev/null || true)
if [ \"\$old\" = $REL ]; then echo same; else
  if [ -n \"\$old\" ]; then ln -sfn \"\$old\" $ROOT/previous.new && mv -T $ROOT/previous.new $ROOT/previous; fi
  ln -sfn $REL $ROOT/current.new && mv -T $ROOT/current.new $ROOT/current
  echo \"\$old\"
fi")"

REMOTE_SHA="$(ssh "$HOST" "sha256sum $ROOT/current/last_route" | cut -d' ' -f1)"
[ "$REMOTE_SHA" = "$SHA" ] || fail "current runs $REMOTE_SHA, expected $SHA"

if [ "$OLD" = "same" ]; then
    echo "current already pointed at $SHA: nothing changed"
else
    echo "current now: $REL"
    if [ -n "$OLD" ]; then
        echo "rollback:    ssh $HOST 'ln -sfn $OLD $ROOT/current.new && mv -T $ROOT/current.new $ROOT/current'   (add: systemctl restart $AGENT_UNIT if you restarted it)"
    else
        echo "rollback:    there was no earlier release; ssh $HOST 'rm -f $ROOT/current'"
    fi
fi

if [ "$RESTART" = "1" ]; then
    echo "WARNING: restarting $AGENT_UNIT now. Every hosted room the agent runs at this moment is dropped."
    ssh "$HOST" "systemctl restart $AGENT_UNIT && sleep 3 && systemctl is-active --quiet $AGENT_UNIT" || fail "$AGENT_UNIT did not come back; see journalctl -u $AGENT_UNIT"
    echo "$AGENT_UNIT is active"
else
    echo "nothing was restarted. The agent starts the new file for rooms spawned after this; RESTART=1 forces it."
fi
echo "deployed: $SHA"
