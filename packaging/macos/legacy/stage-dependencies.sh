#!/bin/bash
# stage-dependencies.sh STAGE: makes a source copy's dependency graph buildable with Rust 1.77.
# STAGE is a disposable copy of the repo (never the checkout: this edits Cargo.toml and Cargo.lock there).
# The caller sets RUSTC to the legacy compiler and CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback, so the
# modern Cargo used here picks dependency versions that compiler can build.
set -euo pipefail
STAGE=${1:?usage: stage-dependencies.sh STAGE_DIRECTORY}
test -f "$STAGE/Cargo.toml"
if git -C "$STAGE" rev-parse --show-toplevel >/dev/null 2>&1 && [ "$(git -C "$STAGE" rev-parse --show-toplevel)" = "$(cd "$STAGE" && pwd -P)" ]; then
    echo "Refusing to stage inside a git checkout: $STAGE" >&2
    exit 1
fi
MANIFEST="$STAGE/Cargo.toml"
# winresource (Windows icon step) pulls edition-2024 toml crates, which Cargo 1.77 cannot read even when unused.
sed -i.bak '/^winresource = /d' "$MANIFEST" && rm "$MANIFEST.bak"
# The repo vendors macroquad 0.4.16 (vendor/macroquad), which needs Rust 1.82. Drop that patch line here; 0.4.14
# from crates.io is staged below instead, with the same atlas fix.
sed -i.bak '/^macroquad = { path = "vendor\/macroquad" }$/d' "$MANIFEST" && rm "$MANIFEST.bak"
cargo +1.99.1 update --manifest-path "$MANIFEST"
# fontdue declares no rust-version; 0.9.4+ uses cast_signed (1.87).
cargo +1.99.1 update --manifest-path "$MANIFEST" -p fontdue --precise 0.9.3
# macroquad: 0.4.14 is the oldest release with the API the game uses; newer ones need const float math (1.82).
# Its only problem with 1.77 is `size_of` without an import (in the prelude since 1.80), so stage a copy with the
# import added to each file that uses it.
cargo +1.99.1 update --manifest-path "$MANIFEST" -p macroquad --precise 0.4.14
SRC=$(cargo +1.99.1 metadata --manifest-path "$MANIFEST" --format-version 1 | python3 -c '
import json, os, sys
for p in json.load(sys.stdin)["packages"]:
    if p["name"] == "macroquad":
        print(os.path.dirname(p["manifest_path"]))')
test -d "$SRC"
mkdir -p "$STAGE/vendor"
cp -R "$SRC" "$STAGE/vendor/macroquad-legacy"
rm -f "$STAGE/vendor/macroquad-legacy/Cargo.toml.orig" "$STAGE/vendor/macroquad-legacy/.cargo-checksum.json"
grep -rlE '(^|[^:_a-zA-Z])size_of(_val)?::<|[^:_a-zA-Z]size_of(_val)?\(' "$STAGE/vendor/macroquad-legacy/src" | while read -r file; do
    # a file that already imports it (any `use ... size_of`) is left alone
    grep -qE '^\s*use .*size_of|^\s*(mem::)?\{?.*size_of.*\},?$' "$file" && continue
    printf '\n#[allow(unused_imports)]\nuse core::mem::size_of;\n' >> "$file"
done
# The glyph atlas row-wrap fix of vendor/macroquad/LAST_ROUTE_PATCHES.md (the line is the same in 0.4.14).
ATLAS="$STAGE/vendor/macroquad-legacy/src/text/atlas.rs"
grep -q 'let x = if self.cursor_x + (width as u16) < self.image.width {' "$ATLAS"
sed -i.bak 's/let x = if self.cursor_x + (width as u16) < self.image.width {/let x = if self.cursor_x + Self::GAP + (width as u16) <= self.image.width {/' "$ATLAS" && rm "$ATLAS.bak"
# Color::from_rgba (and from_hex, which calls it) divides floats in a const fn (allowed since 1.82).
# The game never calls either in a const.
sed -i.bak -e 's/pub const fn from_rgba(/pub fn from_rgba(/' -e 's/pub const fn from_hex(/pub fn from_hex(/' "$STAGE/vendor/macroquad-legacy/src/color.rs" && rm "$STAGE/vendor/macroquad-legacy/src/color.rs.bak"
# macroquad 0.4.14 pins miniquad =0.4.8; the game needs the repo's patched miniquad (vendor/miniquad, 0.4.11).
MINIQUAD=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$STAGE/vendor/miniquad/Cargo.toml" | head -n 1)
sed -i.bak "/^\[dependencies.miniquad\]$/,/^$/s/^version = .*/version = \"=$MINIQUAD\"/" "$STAGE/vendor/macroquad-legacy/Cargo.toml" && rm "$STAGE/vendor/macroquad-legacy/Cargo.toml.bak"
printf 'macroquad = { path = "vendor/macroquad-legacy" }\n' >> "$MANIFEST"
tail -n 8 "$MANIFEST" | grep -q '^\[patch.crates-io\]' || { echo "[patch.crates-io] must be the last section of Cargo.toml" >&2; exit 1; }
cargo +1.99.1 update --manifest-path "$MANIFEST" -p macroquad
# Cargo 1.77 reads lockfile version 3.
sed -i.bak 's/^version = 4$/version = 3/' "$STAGE/Cargo.lock" && rm "$STAGE/Cargo.lock.bak"
