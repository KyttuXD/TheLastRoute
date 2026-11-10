#!/bin/bash
# Deploys the KYTTU - LRS (lr_story) room server to VPS 2: sh packaging/story-server/deploy-vps2.sh BINARY [MODS_DIR]
#   BINARY    lr_story_server built for linux x86_64 (release, --features server); see docs/LR_STORY.md
#   MODS_DIR  the story packs the server reads (default: mods/ in the repo). Every pack folder is copied.
#
# What it does, in order:
#   1. uploads BINARY and MODS_DIR by tar over ssh into a staging folder, then moves them into /opt/lr-story-server
#      (releases/<sha256 of the binary>, and mods/): a half-written file is never run
#   2. makes the system user lr-story and the data folder once; writes /etc/lr-story-server/lr-story-server.env from the
#      example only when it does not exist yet (the real file is never overwritten)
#   3. points /opt/lr-story-server/current at the new release, installs the systemd unit, restarts the service
#   4. opens UDP 1307 in ufw (the relay). The HTTP port 8307 is not opened: Caddy reaches it on loopback.
#   5. checks the binary's sha256 on the server and GET http://127.0.0.1:8307/api/health through the server
# The Caddyfile is NOT touched here: its /api/* block is added by hand (docs/LR_STORY.md, "Story server").
# Environment: HOST (ssh alias, default vps2), DRY_RUN=1 (print the plan only).
set -euo pipefail

BIN="${1:-}"
MODS="${2:-$(cd "$(dirname "$0")/../.." && pwd)/mods}"
HOST="${HOST:-vps2}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT=/opt/lr-story-server
ETC=/etc/lr-story-server

fail() {
    echo "deploy-story-server: $*" >&2
    exit 1
}

[ -n "$BIN" ] || fail "usage: deploy-vps2.sh BINARY [MODS_DIR]"
[ -f "$BIN" ] || fail "$BIN is not a file"
[ -d "$MODS" ] || fail "$MODS is not a folder"
if command -v sha256sum >/dev/null 2>&1; then SHA="$(sha256sum "$BIN" | cut -d' ' -f1)"; else SHA="$(shasum -a 256 "$BIN" | cut -d' ' -f1)"; fi
echo "binary:  $BIN ($SHA)"
echo "packs:   $MODS"
echo "server:  $HOST:$ROOT"

if [ "${DRY_RUN:-}" = "1" ]; then
    echo "DRY_RUN=1: nothing uploaded or changed. It would install releases/$SHA, mods/, the unit, and restart lr-story-server."
    exit 0
fi

echo "uploading..."
{ tar -C "$(dirname "$BIN")" -cf - "$(basename "$BIN")"; } | ssh "$HOST" "set -e
mkdir -p $ROOT/releases/$SHA
tar -C $ROOT/releases/$SHA --no-same-owner -xf -
chmod 755 $ROOT/releases/$SHA/$(basename "$BIN")
[ "$(basename "$BIN")" = lr_story_server ] || mv -f $ROOT/releases/$SHA/$(basename "$BIN") $ROOT/releases/$SHA/lr_story_server"
tar -C "$MODS" -cf - . | ssh "$HOST" "set -e
stage=\$(mktemp -d $ROOT/.mods.XXXXXX)
tar -C \"\$stage\" --no-same-owner -xf -
rm -rf $ROOT/mods.old
[ -d $ROOT/mods ] && mv $ROOT/mods $ROOT/mods.old || true
mv \"\$stage\" $ROOT/mods
rm -rf $ROOT/mods.old"

echo "installing..."
ssh "$HOST" "set -e
id lr-story >/dev/null 2>&1 || useradd --system --home-dir /var/lib/lr-story-server --shell /usr/sbin/nologin lr-story
mkdir -p /var/lib/lr-story-server $ETC
chown lr-story:lr-story /var/lib/lr-story-server
chmod 750 /var/lib/lr-story-server
ln -sfn $ROOT/releases/$SHA $ROOT/current.new && mv -T $ROOT/current.new $ROOT/current
chown -R root:root $ROOT/releases/$SHA $ROOT/mods
"
scp -q "$HERE/lr-story-server.service" "$HOST:/etc/systemd/system/lr-story-server.service"
scp -q "$HERE/lr-story-server.env.example" "$HOST:/tmp/lr-story-server.env.example"
ssh "$HOST" "set -e
[ -f $ETC/lr-story-server.env ] || { install -m 0640 -o root -g lr-story /tmp/lr-story-server.env.example $ETC/lr-story-server.env; }
rm -f /tmp/lr-story-server.env.example
ufw allow 1307/udp comment 'lr-story relay' >/dev/null
systemctl daemon-reload
systemctl enable lr-story-server >/dev/null 2>&1 || true
systemctl restart lr-story-server
sleep 2
systemctl is-active --quiet lr-story-server || { journalctl -u lr-story-server -n 30 --no-pager >&2; exit 1; }
"

REMOTE_SHA="$(ssh "$HOST" "sha256sum $ROOT/current/lr_story_server" | cut -d' ' -f1)"
[ "$REMOTE_SHA" = "$SHA" ] || fail "the server runs $REMOTE_SHA, expected $SHA"
HEALTH="$(ssh "$HOST" "curl -fsS -m 5 http://127.0.0.1:8307/api/health")" || fail "the health check failed"
echo "health: $HEALTH"
echo "deployed: $SHA"
