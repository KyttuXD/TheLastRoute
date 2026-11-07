#!/bin/sh
# Makes the universal (Apple Silicon + Intel) zips that the in-game updater and the nightly release use, from the
# per-architecture apps build_app.sh writes (dist/intel/ and dist/arm64/). Run from the repo root after build_app.sh:
# Needs lipo, zip.
set -e
cd "$(dirname "$0")/../../dist"
rm -rf universal
mkdir -p universal

# universal_zip APP_NAME BINARY ZIP: one universal .app, the arm64 bundle with both slices in its binary.
universal_zip() {
    rm -rf "universal/$1"
    cp -R "arm64/$1" "universal/$1"
    lipo -create "intel/$1/Contents/MacOS/$2" "arm64/$1/Contents/MacOS/$2" \
        -output "universal/$1/Contents/MacOS/$2"
    (cd universal && zip -qry "../$3" "$1")
    echo "built dist/$3"
}

universal_zip "KYTTU The Last Route.app" last_route last_route-macos-universal.zip
