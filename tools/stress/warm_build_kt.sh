#!/bin/sh
# Starts a release build of the copied source in the background, so the compile cache is warm before a stress run.
# Run on the Mac. Logs to $HOME/lr-stress-out/build.log and writes "BUILD_DONE status" when it finishes.
set -u
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
mkdir -p "$HOME/lr-stress-out"
LOG="$HOME/lr-stress-out/build.log"
cd "$HOME/lr-stress" || { echo "no $HOME/lr-stress" > "$LOG"; exit 1; }
nohup sh -c "cargo build --release --bin last_route >> '$LOG' 2>&1; echo BUILD_DONE \$? >> '$LOG'" >/dev/null 2>&1 </dev/null &
echo "started"
