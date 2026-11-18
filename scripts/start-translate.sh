#!/usr/bin/env bash
# Start, stop, monitor or autostart the local LTEngine translation server
# (config written by scripts/setup-translate.sh). The engine runs headless;
# --install adds a systemd user service (Linux) or LaunchAgent (macOS) so it
# comes back after a reboot without any terminal.
set -euo pipefail

STOP=0 STATUS=0 INSTALL=0 UNINSTALL=0 PORT=0
usage() { grep "^#" "$0" | sed "s/^# \{0,1\}//"; exit 0; }
while [ $# -gt 0 ]; do
  case "$1" in
    --stop) STOP=1 ;;
    --status) STATUS=1 ;;
    --install) INSTALL=1 ;;
    --uninstall) UNINSTALL=1 ;;
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
[ "$PORT" -gt 0 ] 2>/dev/null || PORT="$(sed -n 's/.*"port": \([0-9]*\),/\1/p' "$CONFIG")"

SYSTEMD_UNIT="$HOME/.config/systemd/user/serein-translate.service"
LAUNCHD_PLIST="$HOME/Library/LaunchAgents/com.serein.translate.plist"

port_open() { (exec 3<>"/dev/tcp/$1/$2") 2>/dev/null || return 1; }
engine_running() { pgrep -f "ltengine --host" >/dev/null 2>&1; }

if [ "$UNINSTALL" = 1 ]; then
  if [ "$(uname)" = "Darwin" ]; then
    launchctl unload -w "$LAUNCHD_PLIST" 2>/dev/null || true
    rm -f "$LAUNCHD_PLIST"
  else
    systemctl --user disable --now serein-translate.service 2>/dev/null || true
    rm -f "$SYSTEMD_UNIT"
    systemctl --user daemon-reload 2>/dev/null || true
  fi
  echo "Autostart removed. A running engine is left alone; stop it with --stop."
  exit 0
fi

if [ "$INSTALL" = 1 ]; then
  if [ "$(uname)" = "Darwin" ]; then
    mkdir -p "$(dirname "$LAUNCHD_PLIST")"
    cat > "$LAUNCHD_PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.serein.translate</string>
  <key>ProgramArguments</key><array>
    <string>$ENGINE_EXE</string><string>--host</string><string>$HOST</string>
    <string>--port</string><string>$PORT</string><string>--model-file</string><string>$MODEL_FILE</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$DATA_DIR/engine.log</string>
  <key>StandardErrorPath</key><string>$DATA_DIR/engine.err</string>
</dict></plist>
EOF
    launchctl load -w "$LAUNCHD_PLIST"
    echo "Installed LaunchAgent (starts at login, kept alive). Logs: $DATA_DIR"
  else
    mkdir -p "$(dirname "$SYSTEMD_UNIT")"
    cat > "$SYSTEMD_UNIT" <<EOF
[Unit]
Description=Serein Translate engine (LTEngine)

[Service]
ExecStart=$ENGINE_EXE --host $HOST --port $PORT --model-file $MODEL_FILE
Restart=on-failure
StandardOutput=append:$DATA_DIR/engine.log
StandardError=append:$DATA_DIR/engine.err

[Install]
WantedBy=default.target
EOF
    systemctl --user daemon-reload
    systemctl --user enable --now serein-translate.service
    echo "Installed systemd user service (starts at login, restarted on failure). Logs: $DATA_DIR"
  fi
  exit 0
fi

if [ "$STATUS" = 1 ]; then
  if port_open "$HOST" "$PORT"; then STATE="listening on $HOST:$PORT"; else STATE="down"; fi
  echo "Engine   : $STATE"
  echo "Process  : $(engine_running && pgrep -af 'ltengine --host' || echo 'not running')"
  if [ "$(uname)" = "Darwin" ]; then
    [ -f "$LAUNCHD_PLIST" ] && echo "Autostart: installed (LaunchAgent)" || echo "Autostart: not installed (--install to add)"
  else
    systemctl --user is-enabled serein-translate.service >/dev/null 2>&1 \
      && echo "Autostart: installed (systemd user service)" \
      || echo "Autostart: not installed (--install to add)"
  fi
  echo "Config   : $CONFIG"
  [ -f "$DATA_DIR/engine.err" ] && { echo "engine.err (last lines):"; tail -n 5 "$DATA_DIR/engine.err" | sed 's/^/  /'; }
  exit 0
fi

if [ "$STOP" = 1 ]; then
  pkill -f "ltengine --host" 2>/dev/null || true
  echo "Engine stopped."
  exit 0
fi

if port_open "$HOST" "$PORT"; then echo "Engine already listening on $HOST:$PORT"; exit 0; fi

echo "Starting LTEngine headless (model load can take up to a minute)..."
("$ENGINE_EXE" --host "$HOST" --port "$PORT" --model-file "$MODEL_FILE" \
  >>"$DATA_DIR/engine.log" 2>>"$DATA_DIR/engine.err" &)

for _ in $(seq 1 90); do
  sleep 2
  if port_open "$HOST" "$PORT"; then
    echo "Translation engine ready on $HOST:$PORT"
    exit 0
  fi
done
echo "Timed out waiting for the engine port. Last engine.err lines:" >&2
tail -n 5 "$DATA_DIR/engine.err" >&2 2>/dev/null || true
exit 1
