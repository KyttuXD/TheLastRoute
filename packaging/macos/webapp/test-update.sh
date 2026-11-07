#!/bin/sh
# Exercises the app's update, offline and rollback logic on a Mac with no GUI interaction. It serves fake "websites" from
# 127.0.0.1 with python3's http.server (made from the web files bundled in the app, so nothing needs the internet) and runs the
# app hidden (--self-test) against each, with a throw-away data folder and log. The real data folder and log are not touched.
#   sh test-update.sh [APP]        APP defaults to ./Last Route Web.app next to this script's parent (~/LastRouteApp)
# Scenarios (each prints the log lines that decide it, then PASS or FAIL):
#   offline    the site is unreachable: the bundled build runs, the update check says offline
#   update     the site has a newer build: it is downloaded, verified, switched to, and the running page reloads itself into it
#   pages      same wasm, one page file changed: a new folder with that file, build unchanged
#   mismatch   version.json names a build the wasm does not match: nothing is installed
#   rollback   the new build's wasm is garbage: two failed starts, then the previous build runs, and the bad build is not fetched again
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="${1:-$HERE/../Last Route Web.app}"
[ -d "$APP" ] || APP="$HOME/LastRouteApp/Last Route Web.app"
BIN="$APP/Contents/MacOS/LastRoute"
WORK="${WORK:-$HOME/LastRouteApp/test-work}"
PORT="${PORT:-8765}"
SRC="$APP/Contents/Resources/web"
FAILS=0
rm -rf "$WORK"
mkdir -p "$WORK"

SERVER_PID=""
stop_server() {
    if [ -n "$SERVER_PID" ]; then
        kill "$SERVER_PID" 2>/dev/null
        wait "$SERVER_PID" 2>/dev/null
        SERVER_PID=""
    fi
}
trap stop_server EXIT

# make_site NAME MODE: a site folder from the bundled files. MODE: same | pages | newwasm | mismatch | garbage
make_site() {
    dir="$WORK/site-$1"
    mkdir -p "$dir"
    cp "$SRC"/* "$dir/"
    case "$2" in
        same) ;;
        pages) printf '\n// changed by test-update.sh\n' >> "$dir/update.js" ;;
        newwasm | mismatch | garbage)
            python3 -I - "$SRC/last_route.wasm" "$dir/last_route.wasm" "$2" > "$dir/.sha" <<'PY'
import hashlib, sys
src, dst, mode = sys.argv[1:4]
data = open(src, "rb").read()
if mode == "garbage":
    out = b"\0asm\x01\0\0\0" + b"\xff" * 200000
else:
    name = b"lrtest"
    section = bytes([len(name)]) + name + b"x" * 16
    out = data + bytes([0, len(section)]) + section  # a valid extra custom section: same game, different hash
open(dst, "wb").write(out)
print(hashlib.sha256(out).hexdigest())
PY
            sha="$(cat "$dir/.sha")"
            [ "$2" = mismatch ] && sha="$(echo "$sha" | tr '0-9a-f' 'a-f0-9')"
            printf '{"build":"%s","built":"2026-11-07T09:00:00Z"}' "$sha" > "$dir/version.json"
            printf '\n// new build\n' >> "$dir/update.js"
            ;;
    esac
    echo "$dir"
}

serve() {
    stop_server
    python3 -I -m http.server "$PORT" --bind 127.0.0.1 --directory "$1" >"$WORK/server.log" 2>&1 &
    SERVER_PID=$!
    sleep 1
}

# run_case NAME SITE_URL HOLD EXTRA_ARGS...: runs the hidden self-test, leaves $WORK/NAME.log and $WORK/NAME-data
run_case() {
    name="$1"; url="$2"; hold="$3"; shift 3
    rm -rf "$WORK/$name-data" "$WORK/$name.log"
    "$BIN" --self-test --self-test-hold "$hold" --site-url "$url" --data-dir "$WORK/$name-data" --log "$WORK/$name.log" \
        --first-check-delay 1 "$@" >"$WORK/$name.out" 2>&1
    echo "exit code $?" >>"$WORK/$name.out"
}

# run_again NAME SITE_URL HOLD EXTRA_ARGS...: starts the app again on the data folder of run_case NAME (log: NAME-again.log)
run_again() {
    name="$1"; url="$2"; hold="$3"; shift 3
    rm -f "$WORK/$name-again.log"
    "$BIN" --self-test --self-test-hold "$hold" --site-url "$url" --data-dir "$WORK/$name-data" --log "$WORK/$name-again.log" \
        --first-check-delay 1 "$@" >"$WORK/$name-again.out" 2>&1
}

expect() {
    # expect NAME PATTERN DESCRIPTION   (NAME is a log file name without .log)
    if grep -q -e "$2" "$WORK/$1.log"; then
        echo "  ok    $3"
    else
        echo "  FAIL  $3   (missing: $2)"
        FAILS=$((FAILS + 1))
    fi
}
forbid() {
    if grep -q -e "$2" "$WORK/$1.log"; then
        echo "  FAIL  $3   (found: $2)"
        FAILS=$((FAILS + 1))
    else
        echo "  ok    $3"
    fi
}
show() {
    grep -e 'update:' -e 'store:' -e 'boot:' -e 'scheme:' -e 'navigation:' -e 'selftest: PASS' -e 'selftest: FAIL' -e 'verify:' "$WORK/$1.log" | sed 's/^[0-9-]* //' | cut -c1-200
}

echo "== offline: unreachable site"
run_case offline "http://127.0.0.1:1" 0
show offline
expect offline 'update: offline' "the update check reports offline"
expect offline 'boot: READY' "the bundled build still starts"
expect offline 'selftest: PASS' "the self-test passes"

echo "== update: a newer build on the site"
SITE="$(make_site update newwasm)"
NEWBUILD="$(sed -n 's/.*"build":"\([0-9a-f]*\)".*/\1/p' "$SITE/version.json" | cut -c1-12)"
serve "$SITE"
run_case update "http://127.0.0.1:$PORT" 150
show update
expect update 'update: installed' "the new build is downloaded and installed"
expect update "now current $NEWBUILD" "the pointer moves to the new build"
expect update "version.json is now served from $NEWBUILD" "the running page sees the new version.json"
expect update 'the page is loading again by itself' "the page reloads itself (update.js and the update screen)"
expect update "boot: READY .*build folder $NEWBUILD" "the page starts on the new build"
echo "  data folder: $(ls "$WORK/update-data/web" | tr '\n' ' ')"

echo "== pages: only a page file changed"
SITE="$(make_site pages pages)"
serve "$SITE"
run_case pages "http://127.0.0.1:$PORT" 0
show pages
expect pages 'page files changed: update.js' "the changed page file is installed"
expect pages 'selftest: PASS' "the self-test passes"

echo "== mismatch: the wasm does not match version.json"
SITE="$(make_site mismatch mismatch)"
serve "$SITE"
run_case mismatch "http://127.0.0.1:$PORT" 0
show mismatch
expect mismatch 'does not match version.json' "the download is refused"
forbid mismatch 'update: installed' "nothing is installed"

echo "== rollback: the new build's wasm is garbage"
SITE="$(make_site rollback garbage)"
BADBUILD="$(sed -n 's/.*"build":"\([0-9a-f]*\)".*/\1/p' "$SITE/version.json" | cut -c1-12)"
serve "$SITE"
run_case rollback "http://127.0.0.1:$PORT" 150 --ready-timeout 12 --check-interval 15
show rollback
expect rollback 'update: installed' "the bad build is installed (its sha256 is right)"
expect rollback 'ROLLED BACK from' "two failed starts roll back"
expect rollback 'has now failed 2 time' "it took two failures"
expect rollback "boot: READY" "the page runs again after the rollback"
echo "  data folder: $(ls "$WORK/rollback-data/web" | tr '\n' ' ') (bad build $BADBUILD)"

echo "== rollback, next launch: the bad build is remembered"
run_again rollback "http://127.0.0.1:$PORT" 0 --ready-timeout 12
show rollback-again
expect rollback-again "update: skipped" "the known-bad build is not fetched again"
expect rollback-again "boot: READY" "the previous build starts"
forbid rollback-again "boot: FAILED" "no failed start this time"

stop_server
echo
if [ "$FAILS" -eq 0 ]; then echo "ALL SCENARIOS PASSED"; else echo "$FAILS CHECK(S) FAILED"; fi
echo "logs: $WORK/*.log"
[ "$FAILS" -eq 0 ]
