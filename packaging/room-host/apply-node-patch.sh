#!/bin/bash
# Prepares docs/patches/node-hosted-build.patch for the Node backend on VPS 2, on a COPY: bash packaging/room-host/apply-node-patch.sh [BACKEND_DIR] [PATCH]
#   BACKEND_DIR  the live backend folder (default /opt/lastroute-godot/current/backend). Run this ON the server, from anywhere.
#   PATCH        the patch file (default docs/patches/node-hosted-build.patch next to this repo, or ./node-hosted-build.patch)
#
# It never changes BACKEND_DIR and never restarts anything. Steps:
#   1. cp -a BACKEND_DIR BACKEND_DIR.new
#   2. patch -p1 --dry-run on the copy (NOT git apply: the first hunk of region_agent.mjs needs fuzz 1, git apply refuses it)
#   3. patch -p1 on the copy; any .rej or .orig file left means failure
#   4. node --test region_agent.test.mjs regions.test.mjs on the copy, when node exists
#   5. writes BACKEND_DIR.swap.sh (the swap and the restart) but does NOT run it. KT runs it in a quiet window:
#      the restart drops every open hosted room of BOTH sites (the Godot and the Rust site share this backend).
# Default is a DRY RUN (DRY_RUN=1): it only runs `patch --dry-run` on a temporary copy and prints the plan. DRY_RUN=0 makes BACKEND_DIR.new.
set -euo pipefail

BACKEND="${1:-/opt/lastroute-godot/current/backend}"
HERE="$(cd "$(dirname "$0")" && pwd)"
PATCH="${2:-}"
if [ -z "$PATCH" ]; then
    if [ -f "$HERE/../../docs/patches/node-hosted-build.patch" ]; then PATCH="$HERE/../../docs/patches/node-hosted-build.patch"; else PATCH="./node-hosted-build.patch"; fi
fi
DRY_RUN="${DRY_RUN:-1}"
NEW="$BACKEND.new"
SWAP="$BACKEND.swap.sh"
UNITS="${UNITS:-lastroute-godot-test lastroute-region-agent}"

fail() {
    echo "apply-node-patch: $*" >&2
    exit 1
}

[ -d "$BACKEND" ] || fail "$BACKEND is not a folder"
[ -f "$PATCH" ] || fail "$PATCH is not a file"
command -v patch >/dev/null 2>&1 || fail "patch is not installed"
PATCH="$(cd "$(dirname "$PATCH")" && pwd)/$(basename "$PATCH")"

if [ "$DRY_RUN" != "0" ]; then
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    cp -a "$BACKEND" "$TMP/backend"
    # The patch paths start with one folder level (-p1): run it in a folder that holds backend/.
    (cd "$TMP" && patch -p1 --dry-run <"$PATCH") || fail "the dry run failed: a hunk does not apply to $BACKEND (it may have drifted). Nothing was changed."
    echo "DRY RUN OK: every hunk applies to a temporary copy of $BACKEND. Nothing was changed."
    echo "DRY_RUN=0 would: cp -a $BACKEND $NEW, patch there, run the Node tests there, write $SWAP (not run)."
    exit 0
fi

[ ! -e "$NEW" ] || fail "$NEW exists already: remove it (rm -rf $NEW) or look at it first"
OLD="$BACKEND.old"
STAMP="$(date +%F)"
BAK="$BACKEND.bak-$STAMP"
[ ! -e "$OLD" ] || fail "$OLD exists (a previous swap): look at it, then move or remove it by hand. The swap script would nest folders and the rollback would restore the wrong tree."
[ ! -e "$BAK" ] || fail "$BAK exists (a swap today): move or remove it by hand first"
PARENT="$(dirname "$BACKEND")"
echo "copying $BACKEND to $NEW ..."
cp -a "$BACKEND" "$NEW"
# patch -p1 wants backend/<file>: use a throwaway folder whose backend entry is the copy.
WORK="$(mktemp -d "$PARENT/.patchwork.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
ln -s "$NEW" "$WORK/backend"
echo "dry run on the copy ..."
(cd "$WORK" && patch -p1 --dry-run <"$PATCH") || { rm -rf "$NEW"; fail "the dry run failed: nothing was changed (the copy was removed)"; }
echo "patching the copy ..."
(cd "$WORK" && patch -p1 <"$PATCH") || { rm -rf "$NEW"; fail "patch failed on the copy (removed). The live backend is untouched."; }
if find "$NEW" -name '*.rej' -o -name '*.orig' | grep -q .; then
    fail "patch left .rej or .orig files in $NEW: look at them, do not swap"
fi

if command -v node >/dev/null 2>&1; then
    echo "running the Node tests on the copy ..."
    (cd "$NEW" && node --test region_agent.test.mjs regions.test.mjs) || fail "the tests failed on $NEW: do not swap"
else
    echo "node is not on the PATH here: the tests were NOT run. Run them before swapping:"
    echo "  cd $NEW && /opt/node/bin/node --test region_agent.test.mjs regions.test.mjs   (adjust the node path)"
fi

cat >"$SWAP" <<EOS
#!/bin/sh
# Written by apply-node-patch.sh. NOT run by it. Swaps the patched backend in and restarts the units.
# The restart drops every hosted room on BOTH sites (Godot and Rust share this backend) and may bump reloadId.
set -e
[ -d "$NEW" ] || { echo "$NEW is missing" >&2; exit 1; }
[ ! -e "$OLD" ] || { echo "$OLD exists already: refusing to nest folders" >&2; exit 1; }
[ ! -e "$BAK" ] || { echo "$BAK exists already: refusing to nest folders" >&2; exit 1; }
cp -a "$BACKEND" "$BAK"
mv "$BACKEND" "$OLD"
mv "$NEW" "$BACKEND"
systemctl restart $UNITS
sleep 3
systemctl is-active $UNITS
curl -fsS http://127.0.0.1:8205/api/health
echo
echo "rollback: mv $BACKEND $NEW && mv $OLD $BACKEND && systemctl restart $UNITS"
EOS
chmod 700 "$SWAP"
echo "patched copy: $NEW"
echo "swap script written, NOT run: $SWAP"
echo "rollback after a swap: mv $BACKEND $NEW && mv $OLD $BACKEND && systemctl restart $UNITS"
