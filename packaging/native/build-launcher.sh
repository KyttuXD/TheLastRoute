#!/bin/sh
# Builds the launcher (crates/lr_launcher) in release mode and lays it out for copying to a machine.
#   sh packaging/native/build-launcher.sh          -> dist/launcher/
#
# Windows / Linux: dist/launcher/lr_launcher(.exe). Copy it anywhere (a Start menu shortcut, ~/.local/bin).
# macOS: dist/launcher/KYTTU The Last Route.app (Intel + Apple Silicon in one binary, ad-hoc signed, no Dock icon of its own).
#        NOT TESTED ON A MAC YET (written from the design; see docs/LAUNCHER.md, "Needs a Mac").
#
# The launcher is a small program (1.7 MB on Windows) that almost never changes: the game itself is installed and updated from the
# website. A launcher built before a signing key was added to crates/lr_update/keys/trusted.txt cannot install anything:
# add the key first, then build.
set -eu
cd "$(dirname "$0")/../.."
OUT=dist/launcher
TARGET="${CARGO_TARGET_DIR:-target}"
rm -rf "$OUT"
mkdir -p "$OUT"

case "$(uname -s)" in
    Darwin)
        rustup target add x86_64-apple-darwin aarch64-apple-darwin
        cargo build --release -p lr_launcher --bin lr_launcher --target x86_64-apple-darwin
        cargo build --release -p lr_launcher --bin lr_launcher --target aarch64-apple-darwin
        APP="$OUT/KYTTU The Last Route.app"
        mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
        lipo -create "$TARGET/x86_64-apple-darwin/release/lr_launcher" "$TARGET/aarch64-apple-darwin/release/lr_launcher" \
            -output "$APP/Contents/MacOS/lr_launcher"
        cp packaging/native/macos-launcher/Info.plist "$APP/Contents/Info.plist"
        # the game's icon, if the game app was built before (packaging/macos/build_app.sh puts it in dist/)
        for icns in packaging/macos/icons/*.icns dist/*/KYTTU\ The\ Last\ Route.app/Contents/Resources/*.icns; do
            [ -f "$icns" ] && cp "$icns" "$APP/Contents/Resources/last_route.icns" && break
        done
        # Apple Silicon refuses unsigned code; an ad-hoc signature (no developer account) is enough and harmless on Intel.
        codesign --force --deep --sign - "$APP"
        echo "built $APP (copy it with scp -r or rsync, not through a browser: a browser download is quarantined)"
        ;;
    *)
        cargo build --release -p lr_launcher --bin lr_launcher
        EXE=lr_launcher
        [ -f "$TARGET/release/lr_launcher.exe" ] && EXE=lr_launcher.exe
        cp "$TARGET/release/$EXE" "$OUT/$EXE"
        echo "built $OUT/$EXE"
        ;;
esac
