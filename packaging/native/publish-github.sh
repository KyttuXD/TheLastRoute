#!/bin/sh
# Publishes one platform's signed native build to the GitHub release "nightly", where the launcher and the in-game
# updater find it (docs/LAUNCHER.md, "GitHub nightly as the source"). CI runs it on every push to main.
# Usage:  sh packaging/native/publish-github.sh PLATFORM ARTIFACT BUILD REVISION
#   PLATFORM   windows-x86_64 | macos-universal | linux-x86_64 | linux-aarch64
#   ARTIFACT   last_route-windows-x86_64.exe, last-route-linux-x86_64.tar.gz, last_route-macos-universal.zip,
#              or a folder holding the files as they should land (same kinds as packaging/publish-native-vps2.sh)
#   BUILD      the build number (a whole number; CI uses the run number)
#   REVISION   the git commit the build was made from
#
# What it does, in order:
#   1. unpacks the artifact and makes the tree with packaging/native/manifest.mjs (every file zlib-compressed,
#      manifest.json, manifest.json.sig signed with SIGN_KEY, latest.json), then checks it with manifest.mjs verify
#   2. renames every file to its flat release-asset name: ROOT-PLATFORM-<path with / as ->, for example
#      native-linux-x86_64-builds-157-f0.z and native-linux-x86_64-latest.json (ROOT is native, or native-CHANNEL)
#   3. uploads the build files first and latest.json LAST with gh release upload --clobber, so a client only sees the
#      build once every file of it is there
#   4. deletes the assets of older builds of that platform: keeps the newest KEEP (2) and never the one latest.json names
#
# Environment:
#   SIGN_KEY   the ed25519 private key (PEM) file. Required unless DRY_RUN=1. CI writes it from the secret
#              LR_NATIVE_SIGNING_KEY to a temporary file.
#   DRY_RUN=1  build the tree in a temporary folder, print the flat file list and every gh command, upload and delete
#              nothing; needs no key and no gh (the manifest is not signed).
#   TAG (nightly), REPO (GITHUB_REPOSITORY, else KyttuXD/TheLastRoute), CHANNEL (empty = "native"), KEEP (2),
#   VERSION (default: Cargo.toml), MIN_OS (default: 11.0 for macOS), RUN (default per platform), LAUNCHER_MIN (1),
#   WEB_BUILD, NOTES, FORCE=1 (publish a number not higher than the newest on the release).
# macOS (macos-universal) is not built by CI yet; the script supports it for a manual run on a Mac.
set -eu

PLATFORM="${1:-}"
ARTIFACT="${2:-}"
BUILD="${3:-}"
REVISION="${4:-}"
TAG="${TAG:-nightly}"
REPO="${REPO:-${GITHUB_REPOSITORY:-KyttuXD/TheLastRoute}}"
CHANNEL="${CHANNEL:-}"
KEEP="${KEEP:-2}"
LAUNCHER_MIN="${LAUNCHER_MIN:-1}"
NOTES="${NOTES:-}"
WEB_BUILD="${WEB_BUILD:-}"
DRY_RUN="${DRY_RUN:-}"
HERE="$(cd "$(dirname "$0")" && pwd)"
MANIFEST_MJS="$HERE/manifest.mjs"
ROOT_NAME="native"
[ -z "$CHANNEL" ] || ROOT_NAME="native-$CHANNEL"

fail() {
    echo "publish-github: $*" >&2
    exit 1
}

usage() {
    sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

[ -n "$PLATFORM" ] && [ -n "$ARTIFACT" ] && [ -n "$BUILD" ] && [ -n "$REVISION" ] || usage
case "$PLATFORM" in
    windows-x86_64|macos-universal|linux-x86_64|linux-aarch64) ;;
    *) fail "unknown platform $PLATFORM" ;;
esac
case "$BUILD" in ''|*[!0-9]*|0) fail "BUILD must be a whole number of at least 1" ;; esac
case "$CHANNEL" in *[!a-z0-9-]*|-*) fail "CHANNEL may hold lower case letters, digits and dashes only" ;; esac
case "$KEEP" in ''|*[!0-9]*|0) fail "KEEP must be a whole number of at least 1" ;; esac
case "$TAG" in ''|*[!A-Za-z0-9._-]*) fail "TAG may hold letters, digits, dot, dash and underscore only" ;; esac
[ -e "$ARTIFACT" ] || fail "$ARTIFACT does not exist"
command -v node >/dev/null 2>&1 || fail "node is needed (packaging/native/manifest.mjs)"
if [ "$DRY_RUN" != "1" ]; then
    [ -n "${SIGN_KEY:-}" ] || fail "SIGN_KEY (the private signing key file) is not set"
    [ -f "$SIGN_KEY" ] || fail "SIGN_KEY $SIGN_KEY is not a file"
    command -v gh >/dev/null 2>&1 || fail "gh is needed to upload"
fi

case "$PLATFORM" in
    windows-x86_64) DEFAULT_RUN="last_route.exe"; DEFAULT_MIN_OS="" ;;
    macos-universal) DEFAULT_RUN="KYTTU The Last Route.app/Contents/MacOS/last_route"; DEFAULT_MIN_OS="11.0" ;;
    *) DEFAULT_RUN="last-route-linux/bin/last_route"; DEFAULT_MIN_OS="" ;;
esac
RUN="${RUN:-$DEFAULT_RUN}"
MIN_OS="${MIN_OS-$DEFAULT_MIN_OS}"
if [ -z "${VERSION:-}" ]; then
    VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$HERE/../../Cargo.toml" | head -n 1)"
fi
[ -n "$VERSION" ] || fail "no VERSION and none found in Cargo.toml"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
if command -v cygpath >/dev/null 2>&1; then WORK_N="$(cygpath -m "$WORK")"; else WORK_N="$WORK"; fi
mkdir -p "$WORK/src" "$WORK/flat"

echo "platform: $PLATFORM    build: $BUILD    revision: $REVISION    version: $VERSION"
echo "release:  $REPO  tag $TAG    root: $ROOT_NAME"

# ---- unpack (same artifact kinds as publish-native-vps2.sh) ----
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

# ---- 1. the tree ----
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
node "$MANIFEST_MJS" latest --platform-dir "$WORK_N/tree/$PLATFORM" --build "$BUILD" >/dev/null
if [ -n "${SIGN_KEY:-}" ]; then
    node "$MANIFEST_MJS" verify --platform-dir "$WORK_N/tree/$PLATFORM" --key "$SIGN_KEY" --platform "$PLATFORM"
fi

# ---- 2. flat names ----
PREFIX="$ROOT_NAME-$PLATFORM-"
flat_name() { printf '%s%s' "$PREFIX" "$(printf '%s' "$1" | tr '/' '-')"; }
BUILD_FILES=""
for f in "$OUT"/*; do
    [ -f "$f" ] || continue
    name="$(flat_name "builds/$BUILD/$(basename "$f")")"
    cp "$f" "$WORK/flat/$name"
    BUILD_FILES="$BUILD_FILES $name"
done
LATEST_FLAT="$(flat_name latest.json)"
cp "$STAGE/latest.json" "$WORK/flat/$LATEST_FLAT"
[ -f "$WORK/flat/$(flat_name "builds/$BUILD/manifest.json")" ] || fail "the tree has no manifest.json"

echo "flat files (uploaded in this order, latest last):"
for name in $BUILD_FILES "$LATEST_FLAT"; do
    echo "  $(wc -c < "$WORK/flat/$name" | tr -d ' ') bytes  $name"
done

# ---- build number check and the list of existing assets ----
# Asset names of this platform: PREFIX builds-N-...; the number is what follows "builds-".
build_numbers() {
    sed -n "s/^$PREFIX""builds-\([0-9][0-9]*\)-.*/\1/p" | sort -un
}
if [ "$DRY_RUN" = "1" ]; then
    echo "DRY_RUN=1: nothing is uploaded or deleted. A real run would:"
    echo "  gh release view $TAG --repo $REPO --json assets   (refuse a BUILD not higher than the newest listed, unless FORCE=1)"
    for name in $BUILD_FILES; do
        echo "  gh release upload $TAG $name --repo $REPO --clobber"
    done
    echo "  gh release upload $TAG $LATEST_FLAT --repo $REPO --clobber"
    echo "  then, for every build number of $PLATFORM that is neither one of the newest $KEEP nor $BUILD:"
    echo "  gh release delete-asset $TAG ${PREFIX}builds-<N>-<file> --repo $REPO --yes"
    exit 0
fi

ASSETS="$(gh release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name')" || fail "could not list the assets of the $TAG release (try again; FORCE does not skip this)"
NEWEST="$(printf '%s\n' "$ASSETS" | build_numbers | tail -n 1)"
if [ -n "$NEWEST" ]; then
    echo "newest build on the release: $NEWEST"
    if [ "$BUILD" -lt "$NEWEST" ] && [ "${FORCE:-}" != "1" ]; then
        fail "BUILD $BUILD is lower than $NEWEST on the release (clients never go down). Use a higher number, or FORCE=1. The same number is allowed so a failed run can be re-run."
    fi
else
    echo "no build of $PLATFORM on the release yet"
fi

# ---- 3. upload: build files first, latest.json last ----
cd "$WORK/flat"
for name in $BUILD_FILES; do
    echo "uploading $name"
    gh release upload "$TAG" "$name" --repo "$REPO" --clobber
done
echo "uploading $LATEST_FLAT (last)"
gh release upload "$TAG" "$LATEST_FLAT" --repo "$REPO" --clobber

# ---- 4. prune: keep the newest KEEP builds and the one latest.json names ----
ASSETS="$(gh release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name')"
KEEP_NUMS="$(printf '%s\n' "$ASSETS" | build_numbers | tail -n "$KEEP")
$BUILD"
printf '%s\n' "$ASSETS" | grep "^$PREFIX""builds-[0-9]" | while read -r asset; do
    n="$(printf '%s\n' "$asset" | build_numbers)"
    if printf '%s\n' "$KEEP_NUMS" | grep -qx "$n"; then continue; fi
    echo "removing old asset $asset"
    gh release delete-asset "$TAG" "$asset" --repo "$REPO" --yes
done

echo "published: $PLATFORM build $BUILD ($REVISION) to $REPO release $TAG"
