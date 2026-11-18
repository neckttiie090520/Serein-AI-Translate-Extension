#!/usr/bin/env bash
# Serein Translate setup wizard (Linux/macOS).
# Checks the toolchain, builds the LTEngine translation server (applying the
# translate-context patch), downloads a gemma3 GGUF sized to your GPU and
# writes the config consumed by scripts/start-translate.sh.
set -euo pipefail

CHECK_ONLY=0 SKIP_BUILD=0 START_ENGINE=0 AUTOSTART=0 NO_AUTOSTART=0
MODEL=auto MODEL_FILE="" ENGINE_DIR="" PORT=5050
usage() { grep "^#" "$0" | sed "s/^# \{0,1\}//"; exit 0; }
while [ $# -gt 0 ]; do
  case "$1" in
    --check-only) CHECK_ONLY=1 ;;
    --skip-build) SKIP_BUILD=1 ;;
    --start-engine) START_ENGINE=1 ;;
    --autostart) AUTOSTART=1 ;;
    --no-autostart) NO_AUTOSTART=1 ;;
    --model) MODEL="$2"; shift ;;
    --model-file) MODEL_FILE="$2"; shift ;;
    --engine-dir) ENGINE_DIR="$2"; shift ;;
    --port) PORT="$2"; shift ;;
    -h|--help) usage ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/serein-translate"
MODELS_DIR="$DATA_DIR/models"
CONFIG_PATH="$DATA_DIR/config.json"
PATCH_PATH="$SCRIPT_DIR/ltengine-translate-context.patch"
ENGINE_URL="https://github.com/LibreTranslate/LTEngine"
BIND_HOST="127.0.0.1"

case "$(uname -s)" in
  Darwin) FEATURE=metal ;;
  *)      FEATURE=vulkan ;;
esac

have() { command -v "$1" >/dev/null 2>&1; }
step() { printf "\n==> %s\n" "$1"; }
tip() { printf "    %s\n" "$1"; }

VRAM="$(nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits 2>/dev/null | head -n1 | tr -d ' ')" || VRAM=-1
case "$VRAM" in ''|*[!0-9]*) VRAM=-1 ;; esac
if [ "$MODEL" = auto ]; then
  if   [ "$VRAM" -ge 17000 ]; then MODEL=gemma3-27b
  elif [ "$VRAM" -ge 9500 ];  then MODEL=gemma3-12b
  elif [ "$VRAM" -ge 3500 ];  then MODEL=gemma3-4b
  elif [ "$VRAM" -ge 0 ];     then MODEL=gemma3-1b
  else MODEL=gemma3-4b; fi
fi
case "$MODEL" in
  gemma3-1b)  REPO=unsloth/gemma-3-1b-it-GGUF;  FILE=gemma-3-1b-it-Q4_K_M.gguf;  DISK="~0.8 GB";  NOTE="smoke test only" ;;
  gemma3-4b)  REPO=unsloth/gemma-3-4b-it-GGUF;  FILE=gemma-3-4b-it-Q4_K_M.gguf;  DISK="~2.5 GB";  NOTE="recommended default" ;;
  gemma3-12b) REPO=unsloth/gemma-3-12b-it-GGUF; FILE=gemma-3-12b-it-Q4_K_M.gguf; DISK="~7.5 GB";  NOTE="better nuance" ;;
  gemma3-27b) REPO=unsloth/gemma-3-27b-it-GGUF; FILE=gemma-3-27b-it-Q4_K_M.gguf; DISK="~16.5 GB"; NOTE="best quality, slowest" ;;
  *) echo "unknown model: $MODEL" >&2; exit 2 ;;
esac

echo "Serein Translate setup"
printf '  Model: %s (%s)\n' "$MODEL" "$NOTE"

MISSING=""
for t in git cargo cmake clang; do have "$t" || MISSING="$MISSING $t"; done
if [ "$CHECK_ONLY" = 1 ]; then
  step "Environment report"
  for t in git cargo cmake clang; do if have "$t"; then tip "$t: ok"; else tip "$t: not found"; fi; done
  tip "model would be: $MODEL -> $MODELS_DIR/$FILE"
  tip "engine dir would be: ${ENGINE_DIR:-<sibling LTEngine if present, else $DATA_DIR/LTEngine>}"
  if [ -n "$MISSING" ]; then
    printf 'Install hints: %s\n' "$MISSING"
    tip "Debian/Ubuntu: sudo apt install build-essential cmake clang git curl libvulkan-dev libshaderc-dev"
    tip "Fedora: sudo dnf install gcc gcc-c++ cmake clang git curl vulkan-headers shaderc"
    tip "macOS: xcode-select --install && brew install cmake llvm"	
  fi
  exit 0
fi
if [ -n "$MISSING" ]; then echo "Missing tools:$MISSING" >&2; exit 1; fi

if [ -z "$ENGINE_DIR" ]; then
  SIBLING="$(dirname "$REPO_ROOT")/LTEngine"
  if [ -f "$SIBLING/Cargo.toml" ]; then ENGINE_DIR="$SIBLING"; else ENGINE_DIR="$DATA_DIR/LTEngine"; fi
fi
if [ ! -f "$ENGINE_DIR/Cargo.toml" ]; then
  step "Cloning LTEngine into $ENGINE_DIR"
  mkdir -p "$(dirname "$ENGINE_DIR")"
  git clone --depth 1 "$ENGINE_URL" "$ENGINE_DIR"
fi

MAIN_SRC="$ENGINE_DIR/ltengine/src/main.rs"
if [ -f "$PATCH_PATH" ] && [ -f "$MAIN_SRC" ] && ! grep -q "translate-context" "$MAIN_SRC"; then
  step "Applying translate-context patch (context-aware mode endpoint)"
  if git -C "$ENGINE_DIR" apply --check "$PATCH_PATH" 2>/dev/null; then
    git -C "$ENGINE_DIR" apply "$PATCH_PATH"
    tip "patched /translate-context into the engine"
  else
    tip "patch did not apply; continuing without context-aware mode"
  fi
fi

ENGINE_EXE="$ENGINE_DIR/target/release/ltengine"
if [ "$SKIP_BUILD" = 1 ]; then
  [ -x "$ENGINE_EXE" ] || { echo "No existing engine build found; rerun without --skip-build" >&2; exit 1; }
  step "Reusing existing engine build"
else
  step "Building LTEngine (cargo build --release --features $FEATURE)"
  tip "first build compiles llama.cpp and can take 10-30 minutes"
  (cd "$ENGINE_DIR" && cargo build --release --features "$FEATURE")
fi

[ -n "$MODEL_FILE" ] || MODEL_FILE="$MODELS_DIR/$FILE"
if [ ! -s "$MODEL_FILE" ] || [ "$(wc -c < "$MODEL_FILE")" -lt 100000000 ]; then
  step "Downloading $MODEL GGUF ($DISK) - resumable, no Hugging Face account needed"
  mkdir -p "$MODELS_DIR"
  curl -L -C - --retry 3 --retry-delay 2 -o "$MODEL_FILE" "https://huggingface.co/$REPO/resolve/main/$FILE"
fi

mkdir -p "$DATA_DIR"
cat > "$CONFIG_PATH" <<EOF
{
  "engineExe": "$ENGINE_EXE",
  "modelFile": "$MODEL_FILE",
  "host": "$BIND_HOST",
  "port": $PORT,
  "model": "$MODEL"
}
EOF

step "Background autostart (recommended)"
if [ "$NO_AUTOSTART" = 1 ]; then
  tip "skipped (--no-autostart)"
elif [ "$AUTOSTART" = 1 ]; then
  "$SCRIPT_DIR/start-translate.sh" --install
else
  printf "    Install autostart at login? The engine then runs in the background [Y/n] "
  read -r answer
  case "$answer" in ""|[yY]*) "$SCRIPT_DIR/start-translate.sh" --install ;;
  *) tip "skipped; add it any time:  scripts/start-translate.sh --install" ;;
  esac
fi

step "Done"
tip "config written to $CONFIG_PATH"
tip "manage:  scripts/start-translate.sh --status / --stop / --uninstall"
tip "engine log: $DATA_DIR/engine.log"
tip "then run the client from the repo root:  cargo run --release"
if [ "$START_ENGINE" = 1 ]; then "$SCRIPT_DIR/start-translate.sh"; fi

