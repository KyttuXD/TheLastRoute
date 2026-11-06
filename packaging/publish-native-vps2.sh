#!/bin/sh
# Publishes one platform's native build to the website (VPS 2), where the launcher (docs/LAUNCHER.md) finds it.
# Usage:  sh packaging/publish-native-vps2.sh PLATFORM ARTIFACT BUILD REVISION
#   PLATFORM   windows-x86_64 | macos-universal | linux-x86_64 | linux-aarch64
#   ARTIFACT   what `sh packaging/local/publish-local.sh build` made: last_route-windows-x86_64.exe,
#              last-route-linux-x86_64.tar.gz, last_route-macos-universal.zip, or a folder holding the files as they should land
#   BUILD      the build number (a whole number; it must be higher than the one live)
#   REVISION   the git commit the build was made from
#
# What it does, in order:
#   1. unpacks the artifact into a work folder and makes OUT/builds/BUILD with packaging/native/manifest.mjs:
#      every file zlib-compressed, manifest.json (schema lr.native/1), manifest.json.sig (ed25519, signed with SIGN_KEY)
#   2. checks that tree the way the launcher will (manifest.mjs verify), then uploads builds/BUILD by tar over ssh into a
#      staging folder on the server and moves it into place in one step (the same pattern as deploy-vps2.sh)
#   3. fetches every file back through the live URL and compares the sha256
#   4. writes latest.json LAST: the launcher only sees the build once every file of it is in place
#   5. removes all but the newest KEEP builds of that platform (never the one latest.json names)
# It never touches the Caddyfile or anything outside REMOTE_DIR/native*/PLATFORM/.
#
# Environment:
#   SIGN_KEY   the ed25519 private key (PEM) made by `node packaging/native/manifest.mjs keygen KEYFILE`, kept OUTSIDE the repo.
#              Required unless DRY_RUN=1.
#   DRY_RUN=1  unpack, compress and build the manifest in a temporary folder, print the plan and the server commands,
#              upload nothing, change nothing (the manifest is not signed without a key).
#   HOST (vps2), REMOTE_DIR (/srv/lr-rust-web), LIVE_URL (https://lastrouterust.games.kyttuslab.org),
#   CHANNEL (empty = the default channel "native"; a name publishes to native-NAME), KEEP (3),
#   VERSION (default: Cargo.toml), MIN_OS (default: 11.0 for macOS, none otherwise), RUN (default per platform),
#   LAUNCHER_MIN (1), WEB_BUILD (default: the live version.json build), NOTES, FORCE=1 (replace an existing BUILD, or publish
#   a number that is not higher than the live one; clients still refuse a lower number than they have installed).
set -eu

PLATFORM="${1:-}"
ARTIFACT="${2:-}"
BUILD="${3:-}"
REVISION="${4:-}"
HOST="${HOST:-vps2}"
REMOTE_DIR="${REMOTE_DIR:-/srv/lr-rust-web}"
LIVE_URL="${LIVE_URL:-https://lastrouterust.games.kyttuslab.org}"
LIVE_URL="${LIVE_URL%/}"
CHANNEL="${CHANNEL:-}"
KEEP="${KEEP:-3}"
LAUNCHER_MIN="${LAUNCHER_MIN:-1}"
NOTES="${NOTES:-}"
HERE="$(cd "$(dirname "$0")" && pwd)"
MANIFEST_MJS="$HERE/native/manifest.mjs"
ROOT_NAME="native"
[ -z "$CHANNEL" ] || ROOT_NAME="native-$CHANNEL"

fail() {
    echo "publish-native-vps2: $*" >&2
    exit 1
}

usage() {
    sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# ---- arguments ----
[ -n "$PLATFORM" ] && [ -n "$ARTIFACT" ] && [ -n "$BUILD" ] && [ -n "$REVISION" ] || usage
case "$PLATFORM" in
    windows-x86_64|macos-universal|linux-x86_64|linux-aarch64) ;;
    *) fail "unknown platform $PLATFORM" ;;
esac
case "$BUILD" in ''|*[!0-9]*|0) fail "BUILD must be a whole number of at least 1" ;; esac
case "$CHANNEL" in *[!a-z0-9-]*|-*) fail "CHANNEL may hold lower case letters, digits and dashes only" ;; esac
case "$KEEP" in ''|*[!0-9]*|0) fail "KEEP must be a whole number of at least 1" ;; esac
[ -e "$ARTIFACT" ] || fail "$ARTIFACT does not exist"
command -v node >/dev/null 2>&1 || fail "node is needed (packaging/native/manifest.mjs)"
DRY_RUN="${DRY_RUN:-}"
if [ "$DRY_RUN" != "1" ]; then
    [ -n "${SIGN_KEY:-}" ] || fail "SIGN_KEY (the private signing key file) is not set"
    [ -f "$SIGN_KEY" ] || fail "SIGN_KEY $SIGN_KEY is not a file"
fi

# ---- defaults per platform ----
case "$PLATFORM" in
    windows-x86_64) DEFAULT_RUN="last_route.exe"; DEFAULT_MIN_OS="" ;;
    macos-universal) DEFAULT_RUN="KYTTU The Last Route.app/Contents/MacOS/last_route"; DEFAULT_MIN_OS="11.0" ;;
    *) DEFAULT_RUN="last-route-linux/bin/last_route"; DEFAULT_MIN_OS="" ;;
esac
RUN="${RUN:-$DEFAULT_RUN}"
MIN_OS="${MIN_OS-$DEFAULT_MIN_OS}"
if [ -z "${VERSION:-}" ]; then
    VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$HERE/../Cargo.toml" | head -n 1)"
fi
[ -n "$VERSION" ] || fail "no VERSION and none found in Cargo.toml"

# ---- work folder ----
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
if command -v cygpath >/dev/null 2>&1; then WORK_N="$(cygpath -m "$WORK")"; else WORK_N="$WORK"; fi
mkdir -p "$WORK/src"

echo "platform: $PLATFORM    build: $BUILD    revision: $REVISION    version: $VERSION"
echo "artifact: $ARTIFACT"
echo "server:   $HOST:$REMOTE_DIR/$ROOT_NAME/$PLATFORM"
echo "live:     $LIVE_URL/$ROOT_NAME/$PLATFORM/latest.json"

if [ -d "$ARTIFACT" ]; then
    cp -R "$ARTIFACT"/. "$WORK/src/"
else
    case "$ARTIFACT" in
        *.exe) cp "$ARTIFACT" "$WORK/src/last_route.exe" ;;
        *.tar.gz|*.tgz) tar -xzf "$ARTIFACT" -C "$WORK/src" ;;
        *.zip)
            if command -v unzip >/dev/null 2>&1; then unzip -q "$ARTIFACT" -d "$WORK/src"
            elif command -v bsdtar >/dev/null 2>&1; then bsdtar -xf "$ARTIFACT" -C "$WORK/src"
            else fail "need unzip or bsdtar to open $ARTIFACT"; fi ;;
        *) fail "do not know how to unpack $ARTIFACT (.exe, .tar.gz, .zip or a folder)" ;;
    esac
fi
[ -f "$WORK/src/$RUN" ] || fail "the artifact has no $RUN (set RUN=... if the program has another path)"

if [ -z "${WEB_BUILD+x}" ] && [ "$DRY_RUN" != "1" ]; then
    WEB_BUILD="$(curl -fsS --max-time 8 "$LIVE_URL/version.json?t=$(date +%s)" 2>/dev/null | sed -n 's/.*"build" *: *"\([0-9a-f]*\)".*/\1/p' | head -n 1)" || WEB_BUILD=""
fi
WEB_BUILD="${WEB_BUILD:-}"

STAGE="$WORK/tree/$PLATFORM"
OUT="$STAGE/builds/$BUILD"
set -- build --platform "$PLATFORM" --build "$BUILD" --revision "$REVISION" --version "$VERSION" \
    --src "$WORK_N/src" --run "$RUN" --out "$WORK_N/tree/$PLATFORM/builds/$BUILD" --launcher-min "$LAUNCHER_MIN"
[ -z "$MIN_OS" ] || set -- "$@" --min-os "$MIN_OS"
[ -z "$WEB_BUILD" ] || set -- "$@" --web-build "$WEB_BUILD"
[ -z "$NOTES" ] || set -- "$@" --notes "$NOTES"
if [ "$DRY_RUN" = "1" ] && [ -z "${SIGN_KEY:-}" ]; then
    set -- "$@" --no-sign
else
    set -- "$@" --key "$SIGN_KEY"
fi
node "$MANIFEST_MJS" "$@"

echo "files:"
node -e '
const m = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
for (const f of m.files) console.log(`  ${f.mode} ${String(f.size).padStart(10)} -> ${String(f.z_size).padStart(10)}  ${f.path}`);
console.log(`  run: ${m.run}   min_os: ${m.min_os ?? "none"}   launcher_min: ${m.launcher_min}   web_build: ${m.web_build ?? "none"}`);
' "$OUT/manifest.json"

REMOTE_PLATFORM="$REMOTE_DIR/$ROOT_NAME/$PLATFORM"
if [ "$DRY_RUN" = "1" ]; then
    echo "DRY_RUN=1: nothing was uploaded or changed on the server. A real run would:"
    echo "  1. check the tree locally (manifest.mjs verify)"
    echo "  2. read $LIVE_URL/$ROOT_NAME/$PLATFORM/latest.json and refuse a BUILD that is not higher (unless FORCE=1)"
    echo "  3. upload builds/$BUILD by tar over ssh into $HOST:$REMOTE_PLATFORM/.stage.XXXXXX and move it to $REMOTE_PLATFORM/builds/$BUILD"
    echo "  4. fetch manifest.json, manifest.json.sig and every .z from $LIVE_URL and compare their sha256"
    echo "  5. write $REMOTE_PLATFORM/latest.json last, and check it through the live URL"
    echo "  6. keep the newest $KEEP builds under $REMOTE_PLATFORM/builds, remove older numbered folders"
    exit 0
fi

# ---- 1. check the tree like the launcher does ----
node "$MANIFEST_MJS" latest --platform-dir "$WORK_N/tree/$PLATFORM" --build "$BUILD" >/dev/null
node "$MANIFEST_MJS" verify --platform-dir "$WORK_N/tree/$PLATFORM" --key "$SIGN_KEY" --platform "$PLATFORM"

# ---- 2. is BUILD higher than what is live? ----
LIVE_LATEST="$(curl -fsS --max-time 10 "$LIVE_URL/$ROOT_NAME/$PLATFORM/latest.json?t=$(date +%s)" 2>/dev/null || true)"
LIVE_BUILD="$(printf '%s' "$LIVE_LATEST" | sed -n 's/.*"build" *: *\([0-9][0-9]*\).*/\1/p' | head -n 1)"
if [ -n "$LIVE_BUILD" ]; then
    echo "live build: $LIVE_BUILD"
    if [ "$BUILD" -le "$LIVE_BUILD" ] && [ "${FORCE:-}" != "1" ]; then
        fail "BUILD $BUILD is not higher than the live build $LIVE_BUILD (clients never go down). Use a higher number, or FORCE=1 to replace this number."
    fi
else
    echo "live build: none yet for $PLATFORM"
fi

# ---- 3. upload builds/BUILD, moved into place in one step ----
echo "uploading build $BUILD..."
tar -C "$STAGE/builds" -cf - "$BUILD" | ssh "$HOST" "set -e
mkdir -p '$REMOTE_PLATFORM/builds'
stage=\$(mktemp -d '$REMOTE_PLATFORM/.stage.XXXXXX')
trap 'rm -rf \"\$stage\"' EXIT
tar -C \"\$stage\" --no-same-owner -xf -
chmod -R u=rwX,go=rX \"\$stage/$BUILD\"
if [ -e '$REMOTE_PLATFORM/builds/$BUILD' ]; then
    [ '${FORCE:-}' = 1 ] || { echo 'build $BUILD already exists on the server (FORCE=1 replaces it)' >&2; exit 3; }
    mv '$REMOTE_PLATFORM/builds/$BUILD' \"\$stage/replaced\"
fi
mv \"\$stage/$BUILD\" '$REMOTE_PLATFORM/builds/$BUILD'"

# ---- 4. every file, through the live URL ----
echo "verifying $LIVE_URL ..."
BASE="$LIVE_URL/$ROOT_NAME/$PLATFORM/builds/$BUILD"
for f in manifest.json manifest.json.sig; do
    LIVE_SHA="$(curl -fsS --max-time 60 "$BASE/$f?t=$(date +%s)" | { if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi; } | cut -d' ' -f1)"
    [ "$LIVE_SHA" = "$(sha256_of "$OUT/$f")" ] || fail "the live $f differs from the local one; latest.json was NOT written"
done
for z in "$OUT"/*.z; do
    name="$(basename "$z")"
    LIVE_SHA="$(curl -fsS --max-time 600 "$BASE/$name?t=$(date +%s)" | { if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi; } | cut -d' ' -f1)"
    [ "$LIVE_SHA" = "$(sha256_of "$z")" ] || fail "the live $name differs from the local one; latest.json was NOT written"
done
echo "all files verified through the live URL"

# ---- 5. latest.json last ----
echo "writing latest.json..."
tar -C "$WORK/tree/$PLATFORM" -cf - latest.json | ssh "$HOST" "set -e
stage=\$(mktemp -d '$REMOTE_PLATFORM/.stage.XXXXXX')
trap 'rm -rf \"\$stage\"' EXIT
tar -C \"\$stage\" --no-same-owner -xf -
chmod 644 \"\$stage/latest.json\"
mv -f \"\$stage/latest.json\" '$REMOTE_PLATFORM/latest.json'"
LIVE_LATEST="$(curl -fsS --max-time 10 "$LIVE_URL/$ROOT_NAME/$PLATFORM/latest.json?t=$(date +%s)")" || fail "the live latest.json did not answer"
[ "$LIVE_LATEST" = "$(cat "$STAGE/latest.json" | tr -d '\r\n')" ] || fail "the live latest.json is not the one just written: $LIVE_LATEST"

# ---- 6. prune old builds (numbered folders only; never the build latest.json names) ----
ssh "$HOST" "set -e
cd '$REMOTE_PLATFORM/builds'
ls -1 | grep -E '^[0-9]+\$' | sort -n | head -n -$KEEP | while read -r old; do
    [ \"\$old\" = '$BUILD' ] || rm -rf -- \"\$old\"
done"

echo "published: $PLATFORM build $BUILD ($REVISION) at $LIVE_URL/$ROOT_NAME/$PLATFORM/"
