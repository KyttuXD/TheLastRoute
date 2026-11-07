#!/bin/sh
# Builds "KYTTU The Last Route.app" on a Mac with the Command Line Tools (clang, lipo, iconutil, sips, codesign, curl): no Xcode project.
#   sh build-app.sh [OUT_DIR]          OUT_DIR defaults to ./dist next to this script
# The web files bundled into the app are fetched from the website at build time (never committed):
#   SITE_URL=https://...    the site to fetch from (default the live game site)
#   BUNDLE_SRC=FOLDER       use the web files in FOLDER instead (a local dist/web build with version.json); no network needed
#   ARCHS="x86_64 arm64"    architectures of the executable (default both: a universal app; Intel only: ARCHS=x86_64)
#   VERSION=1.0.0           CFBundleVersion (default: APP_VERSION in LastRoute.m)
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-$HERE/dist}"
SITE_URL="${SITE_URL:-https://lastrouterust.games.kyttuslab.org}"
SITE_URL="${SITE_URL%/}"
BUNDLE_SRC="${BUNDLE_SRC:-}"
ARCHS="${ARCHS:-x86_64 arm64}"
VERSION="${VERSION:-$(sed -n 's/^#define APP_VERSION @"\(.*\)"$/\1/p' "$HERE/LastRoute.m")}"
APP="$OUT/KYTTU The Last Route.app"
WEB="$HERE/web-bundle"
FILES="index.html update.js keys.js hw.js audio.js mq_js_bundle.js last_route.wasm"

fail() {
    echo "build-app: $*" >&2
    exit 1
}
sha256_of() {
    shasum -a 256 "$1" | cut -d' ' -f1
}
build_of() {
    sed -n 's/.*"build" *: *"\([0-9a-f]*\)".*/\1/p' "$1" | head -n 1
}

for tool in clang lipo codesign iconutil sips shasum xcrun; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool not found (install the Command Line Tools: xcode-select --install)"
done
SDK="$(xcrun --show-sdk-path)"
START="$(date +%s)"

# ---- 1. the web files ----
rm -rf "$WEB"
mkdir -p "$WEB"
if [ -n "$BUNDLE_SRC" ]; then
    echo "web files: copying from $BUNDLE_SRC"
    for f in $FILES version.json; do
        [ -f "$BUNDLE_SRC/$f" ] || fail "$BUNDLE_SRC/$f is missing"
        cp "$BUNDLE_SRC/$f" "$WEB/$f"
    done
else
    echo "web files: fetching from $SITE_URL"
    curl -fsS --retry 3 -o "$WEB/version.json" "$SITE_URL/version.json?t=$$" || fail "cannot fetch $SITE_URL/version.json"
    for f in $FILES; do
        curl -fsS --retry 3 -o "$WEB/$f" "$SITE_URL/$f?t=$$" || fail "cannot fetch $SITE_URL/$f"
    done
    AGAIN="$(mktemp)"
    curl -fsS --retry 3 -o "$AGAIN" "$SITE_URL/version.json?t=$$b" || fail "cannot fetch version.json again"
    cmp -s "$AGAIN" "$WEB/version.json" || fail "the site was deployed while fetching; run again"
    rm -f "$AGAIN"
fi
BUILD="$(build_of "$WEB/version.json")"
[ -n "$BUILD" ] || fail "version.json has no build"
[ "$(sha256_of "$WEB/last_route.wasm")" = "$BUILD" ] || fail "last_route.wasm does not match version.json ($BUILD)"
echo "web files: build $BUILD, $(du -sh "$WEB" | cut -f1)"

# ---- 2. the executable ----
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
SLICES=""
for arch in $ARCHS; do
    case "$arch" in
        x86_64) MIN=10.13 ;;
        arm64) MIN=11.0 ;;
        *) fail "unknown architecture $arch" ;;
    esac
    echo "compiling $arch (macOS $MIN and later)"
    clang -fobjc-arc -O2 -Wall -Wextra -Wno-unused-parameter -Wno-unguarded-availability-new \
        -arch "$arch" -mmacosx-version-min="$MIN" -isysroot "$SDK" \
        -framework Cocoa -framework WebKit \
        -o "$TMP/LastRoute-$arch" "$HERE/LastRoute.m"
    SLICES="$SLICES $TMP/LastRoute-$arch"
done

# ---- 3. the bundle ----
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/web"
# shellcheck disable=SC2086
lipo -create $SLICES -output "$APP/Contents/MacOS/LastRoute"
sed "s/@VERSION@/$VERSION/g" "$HERE/Info.plist.in" > "$APP/Contents/Info.plist"
cp "$HERE/shell.js" "$HERE/selftest.js" "$APP/Contents/Resources/"
cp "$WEB"/* "$APP/Contents/Resources/web/"

ICONS="$HERE/icon"
[ -f "$ICONS/logo_256.png" ] || ICONS="$HERE/../../../src/bin/last_route/assets/branding"
if [ -f "$ICONS/logo_256.png" ]; then
    ICONSET="$TMP/app.iconset"
    mkdir -p "$ICONSET"
    for s in 16 32 128 256; do
        cp "$ICONS/logo_$s.png" "$ICONSET/icon_${s}x${s}.png"
    done
    cp "$ICONS/logo_32.png" "$ICONSET/icon_16x16@2x.png"
    cp "$ICONS/logo_64.png" "$ICONSET/icon_32x32@2x.png"
    cp "$ICONS/logo_256.png" "$ICONSET/icon_128x128@2x.png"
    sips -z 512 512 "$ICONS/logo_256.png" --out "$ICONSET/icon_256x256@2x.png" >/dev/null
    iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/LastRoute.icns"
else
    echo "build-app: no logo PNGs found; the app gets the default icon" >&2
fi

# ---- 4. ad-hoc signature (no developer account: the app is for this Mac and its siblings) ----
codesign --force --deep -s - "$APP"
codesign --verify --deep --strict "$APP"

echo
echo "built:   $APP"
echo "archs:   $(lipo -archs "$APP/Contents/MacOS/LastRoute")"
echo "size:    $(du -sh "$APP" | cut -f1)"
echo "version: $VERSION, bundled web build $(echo "$BUILD" | cut -c1-12)"
echo "time:    $(($(date +%s) - START)) s"
echo "run:     open \"$APP\"        self-test: \"$APP/Contents/MacOS/LastRoute\" --self-test"
