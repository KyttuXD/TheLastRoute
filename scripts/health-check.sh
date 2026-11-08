#!/usr/bin/env bash
# Health check for lr_server: does the HTTP API answer on 8105 and is the ENet relay up on 1105?
#
#   scripts/health-check.sh [host] [http_port] [relay_port]
#   HEALTH_HOST=vps3.example HEALTH_HTTP_PORT=8105 HEALTH_RELAY_PORT=1105 scripts/health-check.sh
#
# HTTP : GET http://<host>:<http_port>/api/health must answer 200 with "status":"ok".
# Relay: the same answer lists the relay ("relay":{"listening":true,...}); the UDP port is then probed with nc when
#        nc is installed. UDP has no handshake, so the probe is only a hint: the health answer decides.
# Exit : 0 = HTTP ok and relay reported on; 1 = HTTP did not answer ok; 2 = HTTP ok but the relay is off or missing.
# It prints no tokens, passwords or keys and sends none. Run it from a machine that can reach the server; it is not run
# by the build or the tests.

set -u

host="${1:-${HEALTH_HOST:-127.0.0.1}}"
http_port="${2:-${HEALTH_HTTP_PORT:-8105}}"
relay_port="${3:-${HEALTH_RELAY_PORT:-1105}}"
timeout="${HEALTH_TIMEOUT_SECONDS:-5}"

if ! command -v curl >/dev/null 2>&1; then
  echo "HTTP  : UNKNOWN (curl is not installed)"
  exit 1
fi

body="$(curl -sS --max-time "$timeout" -w '\n%{http_code}' "http://${host}:${http_port}/api/health" 2>/dev/null)"
code="${body##*$'\n'}"
json="${body%$'\n'*}"

if [ "$code" != "200" ]; then
  echo "HTTP  : DOWN (http://${host}:${http_port}/api/health answered '${code:-no answer}')"
  exit 1
fi
if ! printf '%s' "$json" | grep -Eq '"status" *: *"ok"'; then
  echo "HTTP  : BAD ANSWER (200, but no \"status\":\"ok\")"
  exit 1
fi
revision="$(printf '%s' "$json" | grep -Eo '"revision" *: *"[^"]*"' | head -n1 | sed -E 's/.*: *"([^"]*)"/\1/')"
uptime="$(printf '%s' "$json" | grep -Eo '"uptimeSeconds" *: *[0-9]+' | head -n1 | grep -Eo '[0-9]+$')"
echo "HTTP  : OK (port ${http_port}, revision ${revision:-unknown}, up ${uptime:-?} s)"

# only the relay's own status carries "listening" (the websocket relay entry never does)
if ! printf '%s' "$json" | grep -Eq '"listening" *: *true'; then
  echo "RELAY : OFF (the server reports no ENet relay; expected UDP ${relay_port})"
  exit 2
fi
if command -v nc >/dev/null 2>&1; then
  if nc -u -z -w 2 "$host" "$relay_port" >/dev/null 2>&1; then
    echo "RELAY : OK (reported on by the server, UDP ${relay_port} not refused)"
  else
    echo "RELAY : REPORTED ON, UDP ${relay_port} PROBE FAILED (a firewall may drop UDP; check the port is open)"
    exit 2
  fi
else
  echo "RELAY : REPORTED ON (UDP ${relay_port} not probed: nc is not installed)"
fi
exit 0
