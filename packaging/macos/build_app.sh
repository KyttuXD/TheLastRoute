#!/bin/sh
# Builds "KYTTU The Last Route.app" for macOS as one universal binary (Apple Silicon + Intel).
# Run on a Mac from the app/ folder:  sh packaging/macos/build_app.sh
# Needs Xcode command line tools (lipo, iconutil, sips) and rustup.
set -e
cd "$(dirname "$0")/../.."
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

APP="dist/KYTTU The Last Route.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create \
    target/aarch64-apple-darwin/release/last_route \
    target/x86_64-apple-darwin/release/last_route \
    -output "$APP/Contents/MacOS/last_route"
cp packaging/macos/Info.plist "$APP/Contents/Info.plist"

# the original's logo as the app icon
ICONSET="$(mktemp -d)/last_route.iconset"
mkdir -p "$ICONSET"
for s in 16 32 64 128 256; do
    cp "assets/branding/logo_$s.png" "$ICONSET/icon_${s}x${s}.png"
done
cp assets/branding/logo_32.png "$ICONSET/icon_16x16@2x.png"
cp assets/branding/logo_64.png "$ICONSET/icon_32x32@2x.png"
cp assets/branding/logo_256.png "$ICONSET/icon_128x128@2x.png"
sips -z 512 512 assets/branding/logo_256.png --out "$ICONSET/icon_256x256@2x.png" >/dev/null
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/last_route.icns"

# unsigned local build: clear the quarantine flag so it opens on this Mac
xattr -cr "$APP" 2>/dev/null || true
echo "built $APP"
