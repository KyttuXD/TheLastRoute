#!/bin/sh
# sh tools/perf/mem.sh SECONDS target/debug/last_route --offline [--stress 40 --stress-scenario mixed]
# Runs the game, prints its resident memory (MB) each second and the peak, then stops it. Linux only (docs/RAM_PASS.md).
secs=$1; shift
"$@" >/dev/null 2>&1 &
pid=$!
peak=0; i=0
while [ $i -lt $secs ] && kill -0 $pid 2>/dev/null; do
    sleep 1; i=$((i+1))
    rss=$(awk '/VmRSS/{print int($2/1024)}' /proc/$pid/status 2>/dev/null)
    [ -n "$rss" ] || break
    [ "$rss" -gt "$peak" ] && peak=$rss
    printf '%s ' "$rss"
done
echo; echo "peak ${peak} MB after ${i}s"
kill $pid 2>/dev/null; wait $pid 2>/dev/null
