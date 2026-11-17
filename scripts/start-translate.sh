#!/usr/bin/env bash
# Start or stop the local LTEngine translation server (config written by setup-translate.sh).
set -euo pipefail

STOP=0 PORT=0
usage() { grep "^#" "$0" | sed "s/^# \{0,1\}//"; exit 0; }
while [ $# -gt 0 ]; do
  case "$1" in
    --stop) STOP=1 ;;
    --port) PORT="$2"; shift ;;
    -h|--help) usage ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/serein-translate"
CONFIG="$DATA_DIR/config.json"
[ -f "$CONFIG" ] || { echo "No config found. Run scripts/setup-translate.sh first." >&2; exit 1; }
ENGINE_EXE="$(sed -n 's/.*"engineExe": "\(.*\)",/\1/p' "$CONFIG")"
HOST="$(sed -n 's/.*"host": "\(.*\)",/\1/p' "$CONFIG")"
MODEL_FILE="$(sed -n 's/.*"modelFile": "\(.*\)",/\1/p' "$CONFIG")"
[ "$PORT" -gt 0 ] || PORT="$(sed -n 's/.*"port": \([0-9]*\),/\1/p' "$CONFIG")"

port_open() { (exec 3<>"/dev/tcp/$1/$2") 2>/dev/null || return 1; }

if [ "$STOP" = 1 ]; then
  pkill -f "ltengine --host" 2>/dev/null || true
  echo "Engine stopped."
  exit 0
fi

if port_open "$HOST" "$PORT"; then echo "Engine already listening on $HOST:$PORT"; exit 0; fi

echo "Starting LTEngine and loading the model (this can take up to a minute)..."
("$ENGINE_EXE" --host "$HOST" --port "$PORT" --model-file "$MODEL_FILE" >/dev/null 2>&1 &)

for _ in $(seq 1 90); do
  sleep 2
  if port_open "$HOST" "$PORT"; then
    echo "Translation engine ready on $HOST:$PORT"
    echo "Now run the Serein client from the repo root: cargo run --release"
    exit 0
  fi
done
echo "Timed out waiting for the engine port." >&2
exit 1

