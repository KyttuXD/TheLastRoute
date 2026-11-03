#!/bin/sh
# Stress runs on KT-MBA (Intel MacBook Air, macOS 12.7.6).
# Builds the copy in $HOME/lr-stress and runs every stress scenario in both vsync modes,
# 60 seconds each, one run per combination.
# SCENARIOS (default "fly npcs hunters interact mixed") and VSYNC (default "off on") limit the list,
# e.g. SCENARIOS="fly mixed" VSYNC=off sh tools/stress/run_kt_mba.sh
# See docs/STRESS_TESTS.md for how the copy gets there.
set -e

PROJECT="$HOME/lr-stress"
OUT_DIR="$HOME/lr-stress-out"
SECONDS_PER_RUN=60
SCENARIOS="${SCENARIOS:-fly npcs hunters interact mixed}"
VSYNC="${VSYNC:-off on}"

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

if command -v caffeinate >/dev/null 2>&1; then
  RUNNER="caffeinate -d"
else
  RUNNER=""
fi

for SCENARIO in $SCENARIOS; do
  for MODE in $VSYNC; do
    NAME="run-$SCENARIO-vsync$MODE-$STAMP"
    CSV="$OUT_DIR/$NAME.csv"
    RUN_OUT="$OUT_DIR/$NAME.out"

    echo "running $SCENARIO, vsync $MODE, $SECONDS_PER_RUN seconds"
    $RUNNER ./target/release/last_route --stress "$SECONDS_PER_RUN" --stress-log "$CSV" \
      --stress-scenario "$SCENARIO" --stress-vsync "$MODE" > "$RUN_OUT" 2>&1 || {
      echo "error: stress run $NAME failed, see $RUN_OUT" >&2
      exit 1
    }

    if [ ! -s "$CSV" ]; then
      echo "error: no CSV written at $CSV, see $RUN_OUT" >&2
      exit 1
    fi

    echo "CSV: $CSV"
    echo "summary: $(tail -n 1 "$RUN_OUT")"
  done
done
