#!/bin/sh
# Builds the Linux version and a folder you can unpack anywhere, with a desktop entry and icon.
# Run on Linux from the app/ folder:  sh packaging/linux/build.sh
# Needs (Debian/Ubuntu names): build-essential libasound2-dev (X11 and OpenGL are loaded when the game runs)
# Fonts: the original uses Courier New Bold and Impact; install ttf-mscorefonts-installer for them,
# otherwise Liberation Mono/Sans Bold are used.
set -e
cd "$(dirname "$0")/../.."
cargo build --release --bin last_route
cargo build --release --bin lr_server --features server

OUT=dist/last-route-linux
rm -rf "$OUT"
mkdir -p "$OUT/share/applications" "$OUT/share/icons/hicolor/256x256/apps" "$OUT/bin"
cp target/release/last_route "$OUT/bin/last_route"
cp packaging/linux/last-route.desktop "$OUT/share/applications/"
cp src/bin/last_route/assets/branding/logo_256.png "$OUT/share/icons/hicolor/256x256/apps/last-route.png"
tar -C dist -czf dist/last-route-linux-x86_64.tar.gz last-route-linux
echo "built $OUT and dist/last-route-linux-x86_64.tar.gz"
