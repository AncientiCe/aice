#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "smoke-macos-arm64.sh must run on macOS."
  exit 1
fi

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "Expected arm64 host; found $(uname -m)."
  exit 1
fi

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing required command: $cmd"
    exit 1
  fi
}

echo "==> Checking required tooling"
require_cmd cargo
require_cmd ollama
if ! command -v whisper-cli >/dev/null 2>&1 && [[ -z "${WHISPER_CLI_BIN:-}" ]]; then
  echo "Missing whisper-cli and WHISPER_CLI_BIN is not set."
  exit 1
fi

echo "==> Verifying required config and models"
[[ -f config.json ]] || { echo "Missing config.json"; exit 1; }
[[ -f models/whisper/ggml-tiny.en.bin ]] || { echo "Missing models/whisper/ggml-tiny.en.bin"; exit 1; }

echo "==> Running local quality gates"
cargo aice-fmt
cargo aice-clippy
cargo aice-audit
cargo aice-test

echo "==> Building release binaries in scope"
cargo build --release -p aice-backend --bin aice-backend

echo "==> Verifying aice-backend binary starts"
./target/release/aice-backend --help >/dev/null

cat <<'EOF'
==> Scripted checks passed.

Manual RC validation still required:
1. Start `cargo aice-backend` with a paired frontend (e.g. aice-macos) and complete one real voice turn (speech -> STT -> LLM -> TTS).
2. Execute at least one skill intent (weather or media) and confirm spoken response.
3. Trigger one policy denial (emergency stop or budget exhausted) and confirm chat fallback.
4. Validate metrics/log emission for both success and failure paths.
EOF
