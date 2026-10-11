#!/usr/bin/env bash
#
# Build and run ai-todo locally, then open it in the browser.
# Linux/macOS counterpart of run-local.ps1.
#
#   ./run-local.sh                  persistent data in data/ai-todo.db
#   ./run-local.sh --fresh          throwaway in-memory data (clean demo)
#   ./run-local.sh --port 9000 --log-capture
#
# Flags can be joined (--port=9000) or given short (-p 9000, -f, -l).
set -euo pipefail

port=8080
fresh=0
log_capture=0

usage() {
    sed -n '3,10p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        -f|--fresh)     fresh=1 ;;
        -l|--log-capture) log_capture=1 ;;
        -p|--port)      port="${2:?--port needs a number}"; shift ;;
        -p=*|--port=*)  port="${1#*=}" ;;
        -h|--help)      usage; exit 0 ;;
        *) printf 'run-local.sh: unknown option %s\n\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

case "$port" in
    ''|*[!0-9]*) printf 'run-local.sh: --port must be a number, got %s\n' "$port" >&2; exit 2 ;;
esac

# Run from wherever the script lives, so data/ and static/ resolve.
cd "$(dirname "${BASH_SOURCE[0]}")"

# Jev key: .env (TYPESAFE_API_KEY=...) is loaded by the server itself; fall
# back to jev-api-key.env. The file is either "TYPESAFE_API_KEY=..." or just
# the bare key, so handle both. The value is exported, never printed.
if [ -z "${TYPESAFE_API_KEY:-}" ] && [ ! -f .env ] && [ -f jev-api-key.env ]; then
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
if [ -z "${TYPESAFE_API_KEY:-}" ] && [ ! -f .env ]; then
    printf 'warning: no Jev key found (.env or jev-api-key.env).\n' >&2
    printf '         Capture falls back to one task per line.\n' >&2
fi

PORT=$port
export PORT
[ "$fresh" -eq 1 ]     && DATA_PATH=':memory:' && export DATA_PATH
[ "$log_capture" -eq 1 ] && CAPTURE_DEBUG=1 && export CAPTURE_DEBUG

cargo build --release -p server

url="http://localhost:$port"
printf 'ai-todo on %s  (Ctrl+C to stop)\n' "$url"

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

./target/release/server
