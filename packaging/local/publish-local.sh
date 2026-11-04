#!/bin/sh
# Builds the committed state (git archive HEAD, never the working tree) on this Windows PC, on a Linux
# ThinkPad and on a Mac, and publishes the results to the update server the same way CI does.
#   sh packaging/local/publish-local.sh build   [windows|linux|macos|all]
#   sh packaging/local/publish-local.sh publish [all|windows|linux|macos]
#   sh packaging/local/publish-local.sh status
# Runs from Git Bash. See packaging/local/README.md. Never prints keys; uses the ssh aliases of ~/.ssh/config.
set -eu

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
DIST="$ROOT/dist-local"

VPS="${VPS_HOST:-vps3}"
UPDATES="/var/lib/lastroute-rust/updates"
MANIFEST="/var/lib/lastroute-rust/update-manifest.json"
LOCK="$UPDATES/.manifest.lock"
CHANNEL="debug"
MAC_HOST="${MAC_HOST:-KT-NEO}"
LINUX_HOSTS="${LINUX_HOSTS:-KT-T480 AK-T480}"
WAIT_MIN="${BUILD_TIMEOUT_MIN:-90}"

WORK="${LR_LOCAL_WORK:-C:/lr-local}"
TARGET_DIR="${LR_LOCAL_TARGET:-C:/lr-local-target}"
if command -v cygpath >/dev/null 2>&1; then
    WORK_U="$(cygpath -u "$WORK")"
    TARGET_U="$(cygpath -u "$TARGET_DIR")"
    TARGET_M="$(cygpath -m "$TARGET_DIR")"
else
    WORK_U="$WORK"; TARGET_U="$TARGET_DIR"; TARGET_M="$TARGET_DIR"
fi

SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=30 -o ServerAliveCountMax=6"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
tmo() { _t="$1"; shift; timeout "$_t" "$@"; }

# the file each platform publishes (the names CI uploads)
file_of() {
    case "$1" in
        windows) echo "last_route-windows-x86_64.exe" ;;
        linux) echo "last-route-linux-x86_64.tar.gz" ;;
        macos) echo "last_route-macos-universal.zip" ;;
        *) echo "unknown-platform" ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
size_of() { wc -c < "$1" | tr -d ' '; }
env_get() { ( . "$1" && eval "printf '%s' \"\${$2:-}\"" ); }

# ---- the update manifest (read-only here; publish writes it with packaging/update-manifest.mjs) ----
MJSON=""
load_manifest() {
    MJSON="$(tmo 60 ssh $SSH_OPTS "$VPS" "cat $MANIFEST")" || { say "cannot read the manifest on $VPS" >&2; return 1; }
}
# mf_get PLATFORM KEY: a field of the platform entry (empty when absent)
mf_get() {
    printf '%s\n' "$MJSON" | awk -v p="\"$1\":" -v k="\"$2\":" '
        $1 == p { f = 1; next }
        f && $1 == k { v = $2; gsub(/[",]/, "", v); print v; exit }
        f && $1 ~ /^\}/ { exit }'
}
ci_latest() {
    _n="$(gh run list --workflow build.yml --limit 20 --json number --jq 'map(.number) | max // 0' 2>/dev/null || true)"
    case "$_n" in
        ''|*[!0-9]*) say "warning: could not read the latest CI run number (gh); using the manifest only" >&2; _n=0 ;;
    esac
    echo "$_n"
}

HEAD_SHA="$(git rev-parse HEAD)"
VERSION="$(git show HEAD:Cargo.toml | sed -n 's/^version = "\(.*\)"/\1/p' | head -n 1)"
[ -n "$VERSION" ] || die "no version in Cargo.toml at HEAD"

# resolve_n: sets N. BUILD from the environment wins; else max(manifest builds, latest CI run) + 1, but a local
# build dir for the same commit that is still ahead of both is reused, so build/publish calls can be split up.
resolve_n() {
    if [ -n "${BUILD:-}" ]; then N="$BUILD"; return 0; fi
    load_manifest || die "cannot work out the build number without the manifest (or give BUILD=n)"
    _mx=0
    for _p in windows linux macos; do
        _b="$(mf_get "$_p" build)"
        case "$_b" in ''|*[!0-9]*) _b=0 ;; esac
        if [ "$_b" -gt "$_mx" ]; then _mx="$_b"; fi
    done
    _ci="$(ci_latest)"
    if [ "$_ci" -gt "$_mx" ]; then _mx="$_ci"; fi
    N=$((_mx + 1))
    for _d in "$DIST"/*/; do
        [ -f "${_d}BUILD.env" ] || continue
        _dn="$(env_get "${_d}BUILD.env" N)"; _ds="$(env_get "${_d}BUILD.env" SHA)"
        case "$_dn" in ''|*[!0-9]*) continue ;; esac
        if [ "$_ds" = "$HEAD_SHA" ] && [ "$_dn" -ge "$N" ]; then N="$_dn"; fi
    done
}

latest_local_n() {
    _best=""
    for _d in "$DIST"/*/; do
        [ -f "${_d}BUILD.env" ] || continue
        _dn="$(env_get "${_d}BUILD.env" N)"
        case "$_dn" in ''|*[!0-9]*) continue ;; esac
        if [ -z "$_best" ] || [ "$_dn" -gt "$_best" ]; then _best="$_dn"; fi
    done
    echo "$_best"
}

# ---- build ----
prepare_dir() {
    OUT="$DIST/$N"
    mkdir -p "$OUT"
    if [ -f "$OUT/BUILD.env" ]; then
        if [ "$(env_get "$OUT/BUILD.env" SHA)" != "$HEAD_SHA" ] && [ "${FORCE:-0}" != 1 ]; then
            die "dist-local/$N was built from another commit; use a new BUILD number or FORCE=1"
        fi
    fi
    printf 'N=%s\nSHA=%s\nVERSION=%s\n' "$N" "$HEAD_SHA" "$VERSION" > "$OUT/BUILD.env"
    if [ ! -f "$OUT/src.tar.gz" ] || [ "${FORCE:-0}" = 1 ]; then
        git archive --format=tar.gz -o "$OUT/src.tar.gz" HEAD
    fi
    say "build $N of $HEAD_SHA (version $VERSION), sources dist-local/$N/src.tar.gz"
}

have_artifact() {
    if [ -f "$OUT/$(file_of "$1")" ] && [ "${FORCE:-0}" != 1 ]; then return 0; fi
    return 1
}

build_windows() {
    if have_artifact windows; then say "windows: already built (FORCE=1 rebuilds)"; return 0; fi
    src="$WORK_U/src-windows"
    rm -rf "$src"; mkdir -p "$src" "$TARGET_U"
    # -m: files get the extraction time, so cargo's mtime check sees them as changed against a cached target dir
    tar -xzmf "$OUT/src.tar.gz" -C "$src"
    say "windows: cargo build (log: dist-local/$N/windows.log)"
    if ! ( cd "$src" && GITHUB_RUN_NUMBER="$N" GITHUB_SHA="$HEAD_SHA" CARGO_TARGET_DIR="$TARGET_M" \
            cargo build --release --locked --bins --features server ) > "$OUT/windows.log" 2>&1; then
        tail -n 30 "$OUT/windows.log" >&2
        say "windows: build FAILED" >&2
        return 1
    fi
    cp "$TARGET_U/release/last_route.exe" "$OUT/last_route-windows-x86_64.exe"
    cp "$TARGET_U/release/night_watch.exe" "$OUT/night_watch-windows-x86_64.exe"
    echo "$N" > "$src/.lr-local-n"
    say "windows: built $(file_of windows) ($(size_of "$OUT/$(file_of windows)") bytes)"
}

# ---- remote builders (linux, macos) ----
write_runner() { # PLATFORM
    case "$1" in
    linux)
        cat > "$OUT/run-linux.sh" <<'EOF'
#!/bin/sh
# Runs on the Linux builder: sh run-linux.sh N SHA. Writes ~/lr-local/done-linux-N (the exit code) when finished.
N="$1"; SHA="$2"
export PATH="$HOME/.cargo/bin:$PATH"
B="$HOME/lr-local"
cd "$B" || exit 1
rm -f "done-linux-$N" "out-linux-$N.tar.gz"
(
    set -e
    mkdir -p "$B/target-linux"
    rm -rf "$B/src-linux"; mkdir -p "$B/src-linux"
    tar -xzmf "$B/src-$N.tar.gz" -C "$B/src-linux"
    cd "$B/src-linux"
    ln -s "$B/target-linux" target
    export GITHUB_RUN_NUMBER="$N" GITHUB_SHA="$SHA"
    command -v cargo
    sh packaging/linux/build.sh
    cp dist/last-route-linux-x86_64.tar.gz "$B/out-linux-$N.tar.gz"
)
rc=$?
echo "$rc" > "$B/done-linux-$N"
EOF
        ;;
    macos)
        cat > "$OUT/run-macos.sh" <<'EOF'
#!/bin/sh
# Runs on the Mac builder: sh run-macos.sh N SHA. Writes ~/lr-local/done-macos-N (the exit code) when finished.
if [ -z "${LR_CAFFEINATED:-}" ] && command -v caffeinate >/dev/null 2>&1; then
    LR_CAFFEINATED=1 exec caffeinate -i sh "$0" "$@"
fi
N="$1"; SHA="$2"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
B="$HOME/lr-local"
cd "$B" || exit 1
rm -f "done-macos-$N" "out-macos-$N.zip"
(
    set -e
    mkdir -p "$B/target-macos"
    rm -rf "$B/src-macos"; mkdir -p "$B/src-macos"
    tar -xzmf "$B/src-$N.tar.gz" -C "$B/src-macos"
    cd "$B/src-macos"
    ln -s "$B/target-macos" target
    export GITHUB_RUN_NUMBER="$N" GITHUB_SHA="$SHA"
    command -v cargo
    rustup target add x86_64-apple-darwin aarch64-apple-darwin
    rm -rf dist
    ARCHS=x86_64-apple-darwin sh packaging/macos/build_app.sh
    ARCHS=aarch64-apple-darwin sh packaging/macos/build_app.sh
    sh packaging/macos/universal.sh
    cp dist/last_route-macos-universal.zip "$B/out-macos-$N.zip"
)
rc=$?
echo "$rc" > "$B/done-macos-$N"
EOF
        ;;
    esac
}

# remote_state PLATFORM HOST: prints DONE <rc>, RUN <last log line>, DEAD, NONE or UNREACHABLE
remote_state() {
    _p="$1"; _h="$2"
    tmo 60 ssh $SSH_OPTS "$_h" "cd lr-local 2>/dev/null || { echo NONE; exit 0; }
d=\$(cat done-$_p-$N 2>/dev/null)
if [ -n \"\$d\" ]; then echo \"DONE \$d\"
elif [ -f pid-$_p-$N ]; then
  if kill -0 \$(cat pid-$_p-$N) 2>/dev/null; then echo \"RUN \$(tail -n 1 build-$_p-$N.log 2>/dev/null | tr '\\r' ' ' | cut -c1-110)\"; else echo DEAD; fi
else echo NONE; fi" 2>/dev/null || echo UNREACHABLE
}

pick_linux_host() {
    if [ -f "$OUT/linux.host" ]; then LINUX_HOST="$(cat "$OUT/linux.host")"; return 0; fi
    for _h in $LINUX_HOSTS; do
        if tmo 30 ssh $SSH_OPTS -o ConnectTimeout=10 "$_h" true >/dev/null 2>&1; then
            LINUX_HOST="$_h"; echo "$_h" > "$OUT/linux.host"; return 0
        fi
        say "linux: $_h unreachable" >&2
    done
    return 1
}

# remote_start PLATFORM HOST: uploads the sources and the runner and starts the build detached, or attaches to a
# build of this N that is already running or finished.
remote_start() {
    _p="$1"; _h="$2"
    if have_artifact "$_p"; then say "$_p: already built (FORCE=1 rebuilds)"; return 0; fi
    _st="$(remote_state "$_p" "$_h")"
    case "$_st" in
        RUN*) say "$_p: build $N already running on $_h"; return 0 ;;
        "DONE 0") say "$_p: build $N already finished on $_h"; return 0 ;;
        UNREACHABLE) say "$_p: $_h is unreachable" >&2; return 1 ;;
    esac
    write_runner "$_p"
    say "$_p: uploading sources to $_h"
    ( cd "$OUT" && tmo 900 ssh $SSH_OPTS "$_h" "mkdir -p lr-local" \
        && tmo 900 scp $SSH_OPTS src.tar.gz "$_h:lr-local/src-$N.tar.gz" \
        && tmo 120 scp $SSH_OPTS "run-$_p.sh" "$_h:lr-local/run-$_p.sh" ) || { say "$_p: upload to $_h failed" >&2; return 1; }
    tmo 60 ssh $SSH_OPTS "$_h" "cd lr-local && rm -f done-$_p-$N pid-$_p-$N && (nohup sh run-$_p.sh $N $HEAD_SHA > build-$_p-$N.log 2>&1 < /dev/null & echo \$! > pid-$_p-$N)" \
        || { say "$_p: could not start the build on $_h" >&2; return 1; }
    say "$_p: build started on $_h (log: ~/lr-local/build-$_p-$N.log)"
}

# remote_collect PLATFORM HOST REMOTE_FILE: waits for the build, then fetches the file and checks its checksum
remote_collect() {
    _p="$1"; _h="$2"; _remote="$3"; _local="$(file_of "$_p")"
    if have_artifact "$_p"; then return 0; fi
    _deadline=$(( $(date +%s) + WAIT_MIN * 60 )); _lost=0; _last=""
    while :; do
        _st="$(remote_state "$_p" "$_h")"
        case "$_st" in
            "DONE 0") break ;;
            DONE*) say "$_p: build FAILED on $_h (exit ${_st#DONE }); log tail:" >&2
                   tmo 60 ssh $SSH_OPTS "$_h" "tail -n 30 lr-local/build-$_p-$N.log" >&2 || true
                   return 1 ;;
            DEAD) say "$_p: the build process on $_h died without a result; log tail:" >&2
                  tmo 60 ssh $SSH_OPTS "$_h" "tail -n 30 lr-local/build-$_p-$N.log" >&2 || true
                  return 1 ;;
            NONE) say "$_p: no build of $N found on $_h" >&2; return 1 ;;
            UNREACHABLE)
                _lost=$((_lost + 1))
                if [ "$_lost" -gt 15 ]; then say "$_p: $_h unreachable for too long" >&2; return 1; fi
                say "$_p: $_h unreachable, retrying" ;;
            RUN*)
                _lost=0
                if [ "$_st" != "$_last" ]; then say "$_p: $_st"; _last="$_st"; fi ;;
        esac
        if [ "$(date +%s)" -ge "$_deadline" ]; then
            say "$_p: timed out after $WAIT_MIN minutes (the build continues on $_h; run 'build $_p' again to collect it)" >&2
            return 1
        fi
        sleep 60
    done
    say "$_p: build finished on $_h, fetching"
    ( cd "$OUT" && tmo 900 scp $SSH_OPTS "$_h:lr-local/$_remote" "$_local.part" ) || { say "$_p: fetch failed" >&2; return 1; }
    _rsha="$(tmo 120 ssh $SSH_OPTS "$_h" "cd lr-local && (sha256sum '$_remote' 2>/dev/null || shasum -a 256 '$_remote') | cut -d' ' -f1")" || _rsha=""
    _lsha="$(sha256_of "$OUT/$_local.part")"
    if [ -n "$_rsha" ] && [ "$_rsha" != "$_lsha" ]; then
        rm -f "$OUT/$_local.part"; say "$_p: checksum mismatch after the copy" >&2; return 1
    fi
    mv "$OUT/$_local.part" "$OUT/$_local"
    say "$_p: fetched $_local ($(size_of "$OUT/$_local") bytes, sha256 $_lsha)"
}

do_build() {
    _what="${1:-all}"
    case "$_what" in
        all) _list="windows linux macos" ;;
        windows|linux|macos) _list="$_what" ;;
        *) die "build: windows, linux, macos or all" ;;
    esac
    resolve_n
    prepare_dir
    _fail=""
    LHOST=""
    # start the remote builds first, build Windows here meanwhile, then collect the remote results
    case " $_list " in *" linux "*)
        if pick_linux_host; then
            LHOST="$LINUX_HOST"; remote_start linux "$LHOST" || _fail="$_fail linux"
        else
            say "linux: no builder reachable ($LINUX_HOSTS)" >&2; _fail="$_fail linux"
        fi ;;
    esac
    case " $_list " in *" macos "*) remote_start macos "$MAC_HOST" || _fail="$_fail macos" ;; esac
    case " $_list " in *" windows "*) build_windows || _fail="$_fail windows" ;; esac
    case " $_list " in *" linux "*)
        case "$_fail" in *linux*) ;; *) remote_collect linux "$LHOST" "out-linux-$N.tar.gz" || _fail="$_fail linux" ;; esac ;;
    esac
    case " $_list " in *" macos "*)
        case "$_fail" in *macos*) ;; *) remote_collect macos "$MAC_HOST" "out-macos-$N.zip" || _fail="$_fail macos" ;; esac ;;
    esac
    if [ -n "$_fail" ]; then die "build $N failed for:$_fail"; fi
    say "build $N done: dist-local/$N"
}

# ---- publish ----
smoke_windows() {
    if [ "${SKIP_SMOKE:-0}" = 1 ]; then say "windows: smoke test skipped (SKIP_SMOKE=1)"; return 0; fi
    _src="$WORK_U/src-windows"
    if [ ! -f "$_src/.lr-local-n" ] || [ "$(cat "$_src/.lr-local-n")" != "$N" ]; then
        say "windows: no matching source dir for the smoke test, skipped"; return 0
    fi
    _suite="$(ls -S "$_src"/tests/*.json 2>/dev/null | tail -n 1)"
    if [ -z "$_suite" ]; then say "windows: no suite file, smoke test skipped"; return 0; fi
    say "windows: smoke test ($(basename "$_suite"))"
    _rc=0
    ( cd "$_src" && tmo 120 "$DIST/$N/$(file_of windows)" --headless --test-suite "tests/$(basename "$_suite")" ) >/dev/null 2>&1 || _rc=$?
    case "$_rc" in
        0) say "windows: smoke test passed" ;;
        124) say "windows: smoke test took over 120 s, skipped" ;;
        *) die "windows: smoke test failed (exit $_rc); fix it or set SKIP_SMOKE=1" ;;
    esac
}

# publish_one PLATFORM: returns 2 when the platform has no built file
publish_one() {
    _p="$1"; _name="$(file_of "$_p")"; _f="$DIST/$N/$_name"
    if [ ! -f "$_f" ]; then say "$_p: $_name not built in dist-local/$N" >&2; return 2; fi
    _size="$(size_of "$_f")"
    if [ "$_size" -le 5000000 ]; then die "$_p: $_name is only $_size bytes"; fi
    _sha="$(sha256_of "$_f")"
    if [ "$_p" = windows ]; then smoke_windows; fi
    load_manifest || die "cannot read the manifest"
    _old="$(mf_get "$_p" build)"
    case "$_old" in ''|*[!0-9]*) _old=0 ;; esac
    if [ "$_old" -gt "$N" ] && [ "${FORCE:-0}" != 1 ]; then
        say "$_p: manifest has build $_old, higher than $N: not published"; return 0
    fi
    case "$_p" in windows) _mode=755 ;; *) _mode=644 ;; esac
    say "$_p: uploading $_name ($_size bytes, sha256 $_sha)"
    ( cd "$DIST/$N" && tmo 1800 scp $SSH_OPTS "$_name" "$VPS:$UPDATES/.$_name.part" ) || die "$_p: upload failed"
    tmo 120 ssh $SSH_OPTS "$VPS" "set -e
f='$UPDATES/.$_name.part'
[ \"\$(sha256sum \"\$f\" | cut -d' ' -f1)\" = '$_sha' ] || { rm -f \"\$f\"; echo 'sha256 mismatch after upload' >&2; exit 1; }
chown nightly-upload:lastroute-rust \"\$f\"
chmod $_mode \"\$f\"
mv \"\$f\" '$UPDATES/$_name'" || die "$_p: could not put the file in place on $VPS"
    tmo 120 ssh $SSH_OPTS "$VPS" "runuser -u nightly-upload -- flock '$LOCK' node - $CHANNEL $_p '$_name' $VERSION $N $BUILD_SHA $_sha" < "$ROOT/packaging/update-manifest.mjs" \
        || die "$_p: manifest update failed"
}

do_publish() {
    _what="${1:-all}"
    case "$_what" in
        all) _list="windows linux macos" ;;
        windows|linux|macos) _list="$_what" ;;
        *) die "publish: all, windows, linux or macos" ;;
    esac
    if [ -n "${BUILD:-}" ]; then N="$BUILD"; else N="$(latest_local_n)"; fi
    [ -n "$N" ] || die "no local build: run 'build' first"
    [ -f "$DIST/$N/BUILD.env" ] || die "dist-local/$N has no BUILD.env"
    BUILD_SHA="$(env_get "$DIST/$N/BUILD.env" SHA)"
    VERSION="$(env_get "$DIST/$N/BUILD.env" VERSION)"
    say "publishing build $N ($BUILD_SHA, version $VERSION) to $VPS"
    _done=0
    for _p in $_list; do
        _rc=0; publish_one "$_p" || _rc=$?
        if [ "$_rc" = 2 ]; then
            if [ "$_what" != all ]; then die "$_p is not built"; fi
        elif [ "$_rc" != 0 ]; then
            exit "$_rc"
        else
            _done=$((_done + 1))
        fi
    done
    if [ "$_done" -eq 0 ]; then say "nothing was published"; fi
    do_status
}

# ---- status ----
do_status() {
    say "== update manifest on $VPS =="
    if load_manifest; then
        for _p in windows linux macos; do
            _b="$(mf_get "$_p" build)"
            if [ -z "$_b" ]; then say "$_p: no entry"; continue; fi
            say "$_p: build $_b, revision $(mf_get "$_p" revision | cut -c1-9), $(mf_get "$_p" file), sha256 $(mf_get "$_p" sha256 | cut -c1-12)"
        done
    else
        say "(manifest unreadable)"
    fi
    say "latest build-workflow run: $(ci_latest)"
    say "== local builds in dist-local =="
    _any=0
    for _d in "$DIST"/*/; do
        [ -f "${_d}BUILD.env" ] || continue
        _any=1
        N="$(env_get "${_d}BUILD.env" N)"; OUT="${_d%/}"
        say "build $N: revision $(env_get "${_d}BUILD.env" SHA | cut -c1-9), version $(env_get "${_d}BUILD.env" VERSION)"
        for _p in windows linux macos; do
            _f="$OUT/$(file_of "$_p")"
            if [ -f "$_f" ]; then
                say "  $_p: $(file_of "$_p") $(size_of "$_f") bytes, sha256 $(sha256_of "$_f" | cut -c1-12)"
                continue
            fi
            _h=""
            case "$_p" in
                linux) if [ -f "$OUT/linux.host" ]; then _h="$(cat "$OUT/linux.host")"; fi ;;
                macos) if [ -f "$OUT/run-macos.sh" ]; then _h="$MAC_HOST"; fi ;;
            esac
            if [ -n "$_h" ]; then say "  $_p: not fetched yet; $_h reports: $(remote_state "$_p" "$_h")"; else say "  $_p: not built"; fi
        done
    done
    if [ "$_any" = 0 ]; then say "(none)"; fi
}

case "${1:-}" in
    build) do_build "${2:-all}" ;;
    publish) do_publish "${2:-all}" ;;
    status) do_status ;;
    *) say "usage: sh packaging/local/publish-local.sh build [windows|linux|macos|all] | publish [all|windows|linux|macos] | status"; exit 2 ;;
esac
