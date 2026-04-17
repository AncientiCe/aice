# Local development setup

Run Aice as a private local Jarvis on your own machine.

Primary path is `pod-voice`, which runs the voice runtime and pod transport in one process.

## 0. First-time macOS setup checklist

Use this section when setting up on a MacBook/Mac mini for the first time.
Each step has:
- a **check** command
- an **install/fix** command if missing

### 0.1 Homebrew

Check:

```bash
brew --version
```

If missing, install Homebrew:

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

### 0.2 Xcode command line tools

Check:

```bash
xcode-select -p
```

If missing:

```bash
xcode-select --install
```

### 0.3 Rust + cargo

Check:

```bash
rustc --version
cargo --version
```

If missing:

```bash
curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"
```

### 0.4 cargo-audit (needed by `cargo aice-audit`)

Check:

```bash
cargo audit --version
```

If missing:

```bash
cargo install cargo-audit
```

### 0.5 Ollama

Check:

```bash
ollama --version
```

Install:

```bash
brew install ollama
brew services start ollama
```

Verify it responds:

```bash
curl -fsS http://127.0.0.1:11434/api/tags
```

Pull the default model used by this repo (`qwen2.5:7b` in `config.example.json`):

```bash
ollama pull qwen2.5:7b
ollama list | grep -E '^qwen2.5:7b'
```

### 0.6 Whisper CLI (`whisper-cli`)

Check:

```bash
whisper-cli --help >/dev/null && echo "whisper-cli found"
```

Install:

```bash
brew install whisper-cpp
```

If `whisper-cli` is not in PATH after install, export it explicitly:

```bash
export WHISPER_CLI_BIN="$(brew --prefix whisper-cpp)/bin/whisper-cli"
```

### 0.7 Piper CLI (`piper`)

Check:

```bash
piper --help >/dev/null && echo "piper found"
```

Homebrew may not provide a `piper` formula in all environments.
If missing, install Piper manually from release binaries:

```bash
mkdir -p "$HOME/tools/piper"
cd "$HOME/tools/piper"

# Apple Silicon:
curl -fL -o piper_macos_aarch64.tar.gz \
  https://github.com/rhasspy/piper/releases/latest/download/piper_macos_aarch64.tar.gz

# Intel (x86_64):
# curl -fL -o piper_macos_x64.tar.gz \
#   https://github.com/rhasspy/piper/releases/latest/download/piper_macos_x64.tar.gz

tar -xzf piper_macos_aarch64.tar.gz
PIPER_PATH="$(find "$HOME/tools/piper" -type f -name piper | head -n1)"
chmod +x "$PIPER_PATH"
echo "piper binary: $PIPER_PATH"
export PIPER_BIN="$PIPER_PATH"
```

Alternative (recommended when binary deps fail): Python venv install

```bash
python3 -m venv "$HOME/.venvs/piper"
"$HOME/.venvs/piper/bin/pip" install --upgrade pip
"$HOME/.venvs/piper/bin/pip" install piper-tts pathvalidate
export PIPER_BIN="$HOME/.venvs/piper/bin/piper"
```

If you see `Library not loaded: @rpath/libespeak-ng.1.dylib`, install runtime libs:

```bash
brew install espeak-ng
```

### 0.8 Download STT and TTS models

Create model directories:

```bash
mkdir -p models/whisper models/piper
```

Download Whisper model expected by default:

```bash
curl -fL \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin \
  -o models/whisper/ggml-base.en.bin
```

Download Piper voice model expected by default:

```bash
curl -fL \
  https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx \
  -o models/piper/model.onnx
curl -fL \
  https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json \
  -o models/piper/model.onnx.json
```

Verify model files:

```bash
ls -lh models/whisper/ggml-base.en.bin
ls -lh models/piper/model.onnx
ls -lh models/piper/model.onnx.json
```

If your files are in different paths, update `config.json`:
- `stt.whisper_model_path`
- `tts.piper_model_path`

#### Which models to pull (defaults and options)

- Ollama default:
  - `qwen2.5:7b`
  - pull: `ollama pull qwen2.5:7b`
- Whisper default:
  - file: `models/whisper/ggml-base.en.bin`
  - URL: `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin`
- Piper default:
  - files: `models/piper/model.onnx` and `models/piper/model.onnx.json`
  - voice source: `en_US-lessac-medium`

Optional bigger/smaller Whisper models (save under `models/whisper/` and point `stt.whisper_model_path`):

```bash
# tiny.en (faster, less accurate)
curl -fL https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin \
  -o models/whisper/ggml-tiny.en.bin

# small.en (better accuracy, slower)
curl -fL https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin \
  -o models/whisper/ggml-small.en.bin
```

Optional Piper voice replacement (save as same filenames so no config change needed):

```bash
curl -fL \
  https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/alba/medium/en_GB-alba-medium.onnx \
  -o models/piper/model.onnx
curl -fL \
  https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/alba/medium/en_GB-alba-medium.onnx.json \
  -o models/piper/model.onnx.json
```

### 0.9 Clone + config

```bash
cd /path/to/aice
cp config.example.json config.json
```

### 0.10 Final preflight check (one-by-one)

```bash
set -e

ok() { printf "OK   %s\n" "$1"; }
bad() { printf "FAIL %s\n" "$1"; }

if command -v rustc >/dev/null; then ok "rustc"; else bad "rustc"; fi
if command -v cargo >/dev/null; then ok "cargo"; else bad "cargo"; fi
if command -v ollama >/dev/null; then ok "ollama"; else bad "ollama"; fi
if command -v whisper-cli >/dev/null || [ -n "${WHISPER_CLI_BIN:-}" ]; then ok "whisper-cli"; else bad "whisper-cli (set WHISPER_CLI_BIN)"; fi
if command -v piper >/dev/null || [ -n "${PIPER_BIN:-}" ]; then ok "piper"; else bad "piper (set PIPER_BIN)"; fi

if [ -f config.json ]; then ok "config.json"; else bad "config.json"; fi
if [ -f models/whisper/ggml-base.en.bin ]; then ok "whisper model"; else bad "models/whisper/ggml-base.en.bin"; fi
if [ -f models/piper/model.onnx ]; then ok "piper model .onnx"; else bad "models/piper/model.onnx"; fi
if [ -f models/piper/model.onnx.json ]; then ok "piper model .onnx.json"; else bad "models/piper/model.onnx.json"; fi

if ollama list 2>/dev/null | grep -q '^qwen2.5:7b'; then ok "ollama model qwen2.5:7b"; else bad "ollama pull qwen2.5:7b"; fi
```

### 0.11 Project gates and run

```bash
cargo aice-fmt
cargo aice-clippy
cargo aice-audit
cargo aice-test
cargo aice-pod-voice
```

## 0.12 One-command model download

If you already installed runtimes (`ollama`, `whisper-cli`, `piper`) and only need models:

```bash
./scripts/download-required-models.sh
```

This downloads to the default paths expected by config:
- `models/whisper/ggml-base.en.bin`
- `models/piper/model.onnx`
- `models/piper/model.onnx.json`
- pulls Ollama model `qwen2.5:7b`

You can override what gets pulled:

```bash
OLLAMA_MODEL=qwen2.5:7b \
WHISPER_URL=https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin \
PIPER_ONNX_URL=https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/alba/medium/en_GB-alba-medium.onnx \
PIPER_JSON_URL=https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/alba/medium/en_GB-alba-medium.onnx.json \
./scripts/download-required-models.sh
```

If you override Whisper URL, also update `stt.whisper_model_path` in `config.json` to the downloaded filename.

## 0.13 Piper troubleshooting

If runtime says:
- `failed to start piper: No such file or directory`

Run with explicit path:

```bash
PIPER_BIN="$HOME/.venvs/piper/bin/piper" cargo run -p desktop-runner
```

Then persist:

```bash
echo 'export PIPER_BIN="$HOME/.venvs/piper/bin/piper"' >> "$HOME/.zshrc"
source "$HOME/.zshrc"
```

If runtime says:
- `ModuleNotFoundError: No module named 'pathvalidate'`

Install dependency into the same venv:

```bash
"$HOME/.venvs/piper/bin/pip" install pathvalidate
```

## Prerequisites

- Rust stable (`rustup`)
- Ollama running locally (or reachable on your LAN)
- `whisper-cli` + model file
- `piper` + voice model file
- `config.json` in the repo root (copy from `config.example.json`)
- macOS host for media control (Mac mini recommended for home setup)

## 1. Build

```bash
cd /path/to/aice
cargo build --workspace
```

## 2. Configure

```bash
cp config.example.json config.json
```

Important fields:

- `ollama_url`, `model`
- `stt.whisper_model_path`
- `tts.piper_model_path`
- `pod_bind`
- `wake_word.*`
- `media.macos_music.enabled`

Music.app note:
- Media control targets Music.app on macOS.
- Apple developer credentials are not required.
- This does not block the main local runtime; media skill remains optional.

## 3. Canonical commands

Use cargo aliases from `.cargo/config.toml`:

```bash
cargo aice-fmt
cargo aice-clippy
cargo aice-audit
cargo aice-test
cargo aice-pod-voice
```

## 4. Runtime options

Primary runtime:

```bash
cargo aice-pod-voice
```

Advanced split runtime:

```bash
cargo aice-gateway
cargo aice-desktop
```

## 5. Local observability dashboard (Prometheus + Grafana)

This repo includes an on-demand local observability stack for `aice-backend`.

Start stack:

```bash
./scripts/observability.sh up
```

Validate resolved compose config:

```bash
./scripts/observability.sh config
```

Check status / logs:

```bash
./scripts/observability.sh status
./scripts/observability.sh logs
```

Stop stack:

```bash
./scripts/observability.sh down
```

Open UIs:

- Grafana: `http://127.0.0.1:3000` (local-only, anonymous dev access)
- Prometheus: `http://127.0.0.1:9090`

Backend metrics endpoint settings are in `config.json` under `service`:

- `metrics_enabled` (default `true`)
- `metrics_bind` (set `AICE_BACKEND_METRICS_BIND=127.0.0.1:9001` to match default observability scrape config)

Persisted local data paths:

- `.local/observability/prometheus`
- `.local/observability/grafana`

## 6. Hardware notes

- Signal Pod is the intended product target.
- M5Stack ATOM Echo support is experimental and useful for transport/firmware testing.
- For home deployments, Mac mini is the recommended host computer.

## Troubleshooting

- Missing STT/TTS output: verify `whisper-cli`, `piper`, and model paths.
- Wake word not triggering: verify `wake_word.enabled` and `wake_word.phrases`.
- Pod cannot connect: verify `pod_bind`, firewall, and same-LAN host IP.
- LLM issues: verify Ollama process and model availability.
