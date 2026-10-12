#!/usr/bin/env bash
#
# Build and run ai-todo locally, then open it in the browser.
# Linux/macOS counterpart of run-local.ps1.
#
#   ./run-local.sh                  persistent data in data/rambl.db
#   ./run-local.sh --fresh          throwaway in-memory data (clean demo)
#   ./run-local.sh --port 9000 --log-capture
#   ./run-local.sh --debug          fast dev build instead of --release
#
# Flags can be joined (--port=9000) or given short (-p 9000, -f, -l, -d).
set -euo pipefail

port=8080
fresh=0
log_capture=0
profile=--release

usage() {
    sed -n '3,11p' "$0" | sed 's/^# \{0,1\}//'
}

die() {
    printf 'run-local.sh: %s\n' "$1" >&2
    exit "${2:-1}"
}

while [ $# -gt 0 ]; do
    case "$1" in
        -f|--fresh)       fresh=1 ;;
        -l|--log-capture) log_capture=1 ;;
        -d|--debug)       profile="" ;;
        -p|--port)        port="${2:?--port needs a number}"; shift ;;
        -p=*|--port=*)    port="${1#*=}" ;;
        -h|--help)        usage; exit 0 ;;
        *) printf 'run-local.sh: unknown option %s\n\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

case "$port" in
    ''|*[!0-9]*) die "--port must be a number, got '$port'" 2 ;;
esac

# Run from wherever the script lives, so data/ and rambl.env resolve.
cd "$(dirname "${BASH_SOURCE[0]}")"

# A stale server from an earlier run is the usual reason the next one dies
# with "AddrInUse". Catch it here, where the message can be useful.
if command -v ss >/dev/null 2>&1; then
    if ss -ltnH 2>/dev/null | awk '{print $4}' | grep -q ":$port\$"; then
        die "something is already listening on port $port.
Either stop it (ss -ltnp | grep :$port shows what), or use another port:
    ./run-local.sh --port 9000" 3
    fi
elif command -v lsof >/dev/null 2>&1; then
    if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
        die "something is already listening on port $port.
Either stop it (lsof -nP -iTCP:$port -sTCP:LISTEN), or use another port:
    ./run-local.sh --port 9000" 3
    fi
fi

# Jev key: rambl.env (TYPESAFE_API_KEY=...) is loaded by the server itself; fall
# back to jev-api-key.env. The file is either "TYPESAFE_API_KEY=..." or just
# the bare key, so handle both. The value is exported, never printed.
if [ -z "${TYPESAFE_API_KEY:-}" ] && [ ! -f rambl.env ] && [ -f jev-api-key.env ]; then
    value=$(sed -n 's/^[[:space:]]*TYPESAFE_API_KEY[[:space:]]*=[[:space:]]*//p' jev-api-key.env | head -n 1)
    if [ -z "$value" ]; then
        # Not a "NAME=value" file: treat the first non-blank, non-comment line
        # as the key itself.
        value=$(grep -v '^[[:space:]]*\(#\|$\)' jev-api-key.env | head -n 1 || true)
    fi
    value="${value#"${value%%[![:space:]]*}"}"   # trim leading whitespace
    value="${value%"${value##*[![:space:]]}"}"   # trim trailing whitespace
    value="${value#\"}"; value="${value%\"}"     # strip surrounding quotes
    value="${value#\'}"; value="${value%\'}"
    if [ -n "$value" ]; then
        TYPESAFE_API_KEY=$value
        export TYPESAFE_API_KEY
    fi
fi
if [ -z "${TYPESAFE_API_KEY:-}" ] && [ ! -f rambl.env ]; then
    printf 'warning: no Jev key found (rambl.env or jev-api-key.env).\n' >&2
    printf '         Capture falls back to one task per line.\n' >&2
fi

PORT=$port
export PORT
RAMBL_DEFAULT_TZ="${RAMBL_DEFAULT_TZ:-America/New_York}"
export RAMBL_DEFAULT_TZ
[ "$fresh" -eq 1 ]       && DATA_PATH=':memory:' && export DATA_PATH
[ "$log_capture" -eq 1 ] && CAPTURE_DEBUG=1 && export CAPTURE_DEBUG

# --release is what run-local.ps1 uses and what a demo should run, but it
# costs ~100s from cold. --debug rebuilds in a few seconds.
if [ -n "$profile" ]; then
    cargo build --release -p rambl-server
    server=./target/release/rambl-server
else
    cargo build -p rambl-server
    server=./target/debug/rambl-server
fi

url="http://localhost:$port"
printf 'ai-todo on %s  (%s build, Ctrl+C to stop)\n' "$url" \
    "$([ -n "$profile" ] && echo release || echo debug)"

# Open the browser shortly after the server is up. Purely a convenience: if
# there is no opener, or it fails, the server is still fine.
(
    sleep 2
    if command -v xdg-open >/dev/null 2>&1; then
        xdg-open "$url" >/dev/null 2>&1 || true
    elif command -v open >/dev/null 2>&1; then
        open "$url" >/dev/null 2>&1 || true
    fi
) &

exec "$server"
