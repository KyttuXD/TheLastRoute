#!/bin/sh
# Stress run on KT-MBA (Intel MacBook Air, macOS 12.7.6).
# Builds the copy in $HOME/lr-stress and runs the 60 second stress mode.
# See docs/STRESS_TESTS.md for how the copy gets there.
set -e

PROJECT="$HOME/lr-stress"
OUT_DIR="$HOME/lr-stress-out"

if [ ! -f "$PROJECT/Cargo.toml" ]; then
  echo "error: $PROJECT/Cargo.toml is missing. Copy the project to $PROJECT first (docs/STRESS_TESTS.md)." >&2
  exit 1
fi

if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi

mkdir -p "$OUT_DIR"
cd "$PROJECT"

echo "building last_route (release), log: $OUT_DIR/build.log"
cargo build --release --bin last_route > "$OUT_DIR/build.log" 2>&1 || {
  echo "error: build failed, last lines of $OUT_DIR/build.log:" >&2
  tail -n 40 "$OUT_DIR/build.log" >&2
  exit 1
}

STAMP=$(date +%Y%m%d-%H%M%S)
CSV="$OUT_DIR/run-$STAMP.csv"
RUN_OUT="$OUT_DIR/run-$STAMP.out"

if command -v caffeinate >/dev/null 2>&1; then
  RUNNER="caffeinate -d"
else
  RUNNER=""
fi

echo "running stress mode for 60 seconds"
$RUNNER ./target/release/last_route --stress 60 --stress-log "$CSV" > "$RUN_OUT" 2>&1 || {
  echo "error: stress run failed, see $RUN_OUT" >&2
  exit 1
}

if [ ! -s "$CSV" ]; then
  echo "error: no CSV written at $CSV, see $RUN_OUT" >&2
  exit 1
fi

echo "CSV: $CSV"
echo "stdout: $RUN_OUT"
echo "summary: $(tail -n 1 "$RUN_OUT")"
