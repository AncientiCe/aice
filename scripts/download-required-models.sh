#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WHISPER_DIR="$ROOT_DIR/models/whisper"
PIPER_DIR="$ROOT_DIR/models/piper"

OLLAMA_MODEL="${OLLAMA_MODEL:-llama3.2}"
WHISPER_URL="${WHISPER_URL:-https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin}"
PIPER_ONNX_URL="${PIPER_ONNX_URL:-https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx}"
PIPER_JSON_URL="${PIPER_JSON_URL:-https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json}"

WHISPER_OUT="$WHISPER_DIR/ggml-tiny.en.bin"
PIPER_ONNX_OUT="$PIPER_DIR/model.onnx"
PIPER_JSON_OUT="$PIPER_DIR/model.onnx.json"

echo "==> Creating model directories"
mkdir -p "$WHISPER_DIR" "$PIPER_DIR"

download_if_missing() {
  local url="$1"
  local out="$2"
  if [[ -f "$out" ]]; then
    echo "==> Exists, skipping: $out"
    return
  fi
  echo "==> Downloading: $out"
  curl -fL "$url" -o "$out"
}

download_if_missing "$WHISPER_URL" "$WHISPER_OUT"
download_if_missing "$PIPER_ONNX_URL" "$PIPER_ONNX_OUT"
download_if_missing "$PIPER_JSON_URL" "$PIPER_JSON_OUT"

echo "==> Pulling Ollama model: $OLLAMA_MODEL"
if ! command -v ollama >/dev/null 2>&1; then
  echo "ERROR: ollama not found in PATH"
  exit 1
fi

if ! ollama list >/dev/null 2>&1; then
  cat <<'EOF'
ERROR: ollama server is not reachable.
Start it first, then re-run this script:
  brew services start ollama
or:
  ollama serve
EOF
  exit 1
fi

ollama pull "$OLLAMA_MODEL"

cat <<EOF
==> Done
Whisper model: $WHISPER_OUT
Piper model:   $PIPER_ONNX_OUT
Piper config:  $PIPER_JSON_OUT
Ollama model:  $OLLAMA_MODEL
EOF
