#!/bin/sh
#   dist/intel/   x86_64 (Intel Macs)
#   dist/arm64/   aarch64 (Apple Silicon)
# Run on a Mac from the repo root:  sh packaging/macos/build_app.sh
# Choose the architectures with ARCHS (default: both). Intel Macs build only their own:
#   ARCHS=x86_64-apple-darwin sh packaging/macos/build_app.sh
# Needs Xcode command line tools (iconutil, sips, zip) and rustup.
set -e
cd "$(dirname "$0")/../.."
ARCHS="${ARCHS:-x86_64-apple-darwin aarch64-apple-darwin}"
for t in $ARCHS; do
    case "$t" in
        aarch64-apple-darwin|x86_64-apple-darwin) ;;
        *) echo "unknown ARCHS entry: $t" >&2; exit 1 ;;
    esac
done
rustup target add $ARCHS
for t in $ARCHS; do
    cargo build --release --target "$t" --bin last_route
done

BRANDING=src/bin/last_route/assets/branding

# make_icns OUT.icns [SOURCE_256.png]: an icon set. Without a source, the Last Route logo (its own PNG sizes).
make_icns() {
    ICONSET="$(mktemp -d)/app.iconset"
    mkdir -p "$ICONSET"
    if [ -n "$2" ]; then
        for s in 16 32 64 128 256 512; do
            sips -z $s $s "$2" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
        done
        cp "$ICONSET/icon_32x32.png" "$ICONSET/icon_16x16@2x.png"
        cp "$ICONSET/icon_64x64.png" "$ICONSET/icon_32x32@2x.png"
        cp "$ICONSET/icon_256x256.png" "$ICONSET/icon_128x128@2x.png"
        cp "$ICONSET/icon_512x512.png" "$ICONSET/icon_256x256@2x.png"
        rm -f "$ICONSET/icon_64x64.png"
    else
        for s in 16 32 64 128 256; do
            cp "$BRANDING/logo_$s.png" "$ICONSET/icon_${s}x${s}.png"
        done
        cp "$BRANDING/logo_32.png" "$ICONSET/icon_16x16@2x.png"
        cp "$BRANDING/logo_64.png" "$ICONSET/icon_32x32@2x.png"
        cp "$BRANDING/logo_256.png" "$ICONSET/icon_128x128@2x.png"
        sips -z 512 512 "$BRANDING/logo_256.png" --out "$ICONSET/icon_256x256@2x.png" >/dev/null
    fi
    iconutil -c icns "$ICONSET" -o "$1"
}

# make_bundle TRIPLE SLUG APP_NAME BINARY PLIST ICON_NAME [ICON_SOURCE_256.png]: one .app for one architecture.
make_bundle() {
    APP="dist/$2/$3"
    rm -rf "$APP"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
    cp "target/$1/release/$4" "$APP/Contents/MacOS/$4"
    cp "$5" "$APP/Contents/Info.plist"
    make_icns "$APP/Contents/Resources/$6.icns" "$7"
    # unsigned local build: clear the quarantine flag so it opens on this Mac
    xattr -cr "$APP" 2>/dev/null || true
    echo "built $APP"
}

for t in $ARCHS; do
    case "$t" in
        x86_64-apple-darwin) SLUG=intel; ARCH=x86_64 ;;
        aarch64-apple-darwin) SLUG=arm64; ARCH=arm64 ;;
    esac
    make_bundle "$t" "$SLUG" "KYTTU The Last Route.app" last_route packaging/macos/Info.plist last_route
done
