# Developer setup

Build Aice from source and run the property stack on one machine: the staff desk, the voice backend and the room bridge. This works on Linux and Windows. For a real installation, use the [property pilot runbook](../runbooks/property-pilot.md).

## 1. Prerequisites

| Tool | Why | Notes |
|------|-----|-------|
| Rust stable (`rustup`) | Builds the workspace | `cargo install cargo-audit` for `cargo aice-audit` |
| Git with HTTPS access to GitHub | `aice-skills` and `palace-rs` are public git dependencies | No SSH key needed |
| CMake and a C/C++ compiler | `whisper-rs` builds whisper.cpp | Visual Studio Build Tools on Windows |
| [Ollama](https://ollama.com) | LLM for classification and answers | Default model `qwen2.5:7b` |
| `whisper-cli` + a Whisper model | Speech to text | Set `WHISPER_CLI_BIN` if it is not on `PATH` |
| `piper` + a voice model | The pod's voice (used by the room bridge) | Set `PIPER_BIN` if it is not on `PATH` |
| Python + PlatformIO (`pip install platformio`) | Only to build pod firmware | `python -m platformio` works without `pio` on `PATH` |

Download the Whisper (`ggml-base.en.bin`) and Piper (`en_US-lessac-medium`) models into `models/`, and pull the Ollama model:

```bash
./scripts/download-required-models.sh
```

The script is POSIX shell. On Windows, run it from Git Bash or WSL, or download the URLs it lists by hand. Override `WHISPER_URL`, `PIPER_ONNX_URL` or `OLLAMA_MODEL` to pick other models, and update `stt.whisper_model_path` or `tts.piper_model_path` to match.

## 2. Build and run the quality gates

```bash
cargo build --workspace
cargo aice-fmt
cargo aice-clippy
cargo aice-audit
cargo aice-test
```

The aliases live in `.cargo/config.toml`. A task is done only when all four pass ([AGENTS.md](../../AGENTS.md)).

## 3. Run the staff desk

Keep runtime state in `.local/` (git-ignored). The pack writes `service.token`, `property.sqlite` and, with TLS on, `tls/` beside its `property.json`.

```bash
mkdir -p .local/desk
cat > .local/desk/property.json <<'EOF'
{
  "pack": "hotels",
  "bind": "127.0.0.1:8791",
  "tls": { "mode": "off" },
  "metrics_bind": "127.0.0.1:9791",
  "database_path": "property.sqlite",
  "service_token_file": "service.token",
  "memory_retention": { "mode": "keep" }
}
EOF
cargo run -p aice-hotels -- .local/desk/property.json user add dev supervisor
cargo run -p aice-hotels -- .local/desk/property.json stay open 101 --consent
cargo run -p aice-hotels -- .local/desk/property.json
```

Open `http://127.0.0.1:8791/desk` and log in. `tls.mode: off` is allowed only on loopback. Any network bind uses TLS ([architecture 24](../architecture/README.md#24-property-security-desk-login-service-token-tls)). Use `aice-care` or `aice-ward` with `"pack": "care"` or `"ward"` to try the other packs.

## 4. Run the voice backend

```bash
cp config.example.json config.json
```

Point `config.json` at the desk:

```json
"property": {
  "facilitator_url": "http://127.0.0.1:8791/mcp",
  "service_token_file": ".local/desk/service.token"
}
```

Then start it on loopback. A property backend refuses plain HTTP on a network bind:

```bash
AICE_BACKEND_BIND=127.0.0.1:8781 cargo aice-backend
```

With a facilitator configured, the backend accepts only desk-issued device tokens on `/turns/stream`.

## 5. Run the room bridge

```bash
cargo aice-gateway
```

The bridge reads the same `config.json`: `pod_bind`, `pod_gateway.backend_url` (default `ws://127.0.0.1:8781/turns/stream`) and `tts.piper_model_path`. With a desk configured, it refuses plain WebSocket on a network bind. Set `"pod_bind": "127.0.0.1:8765"` for local work. Real pods connect over WSS only. Issue the bridge a certificate (`aice-hotels <property.json> tls issue …`), set `pod_gateway.tls`, bind to the network, and follow the [room pod guide](../deployment/pod.md) to build `property_trust.h`, flash and enrol a pod.

Without a pod, drive the whole stack with simulated rooms:

```bash
cargo aice-loadtest --bridge ws://127.0.0.1:8765/ --facilitator http://127.0.0.1:8791 \
  --db .local/desk/property.sqlite --rooms 2 --turns 1 --pcm utterance.raw
```

`--pcm` is a recorded request as raw PCM16, 16 kHz, mono. Revoke the `loadtest-NNNN` pods afterwards with `device revoke`.

## 6. Observe it

```bash
./scripts/observability.sh up
```

This starts Grafana on `http://127.0.0.1:3000` and Prometheus on `http://127.0.0.1:9090`, scraping the backend (`:9001`), the bridge (`:9766`) and the desk (`:9791`). Open the **Aice Guest Journey** dashboard. Slow turns are logged as `slow_turn` lines with their full timeline ([runbook](../runbooks/local-observability.md)). Stop the stack with `./scripts/observability.sh down`.

## Troubleshooting

- **No transcript or no voice:** check `whisper-cli`, `piper` and the model paths (`WHISPER_CLI_BIN`, `PIPER_BIN`, `stt.whisper_model_path`, `tts.piper_model_path`).
- **Slow first answer:** keep the model loaded with `model_keep_alive` (it needs a unit, for example `"30m"` or `"-1m"`), and check `slow_turn` lines for the slowest stage.
- **"refusing plain HTTP" / "refusing plain WebSocket for pods":** with a desk configured, the backend needs `service.tls` and the bridge needs `pod_gateway.tls` on a network bind. Bind to `127.0.0.1`, or set `service.allow_plaintext_lan` on a lab network only.
- **Room requests answered as chat:** check that `property.facilitator_url` is set and the desk is up. Room requests are offered to the classifier only when a desk is configured.
- **Follow-ups need the wake word again:** raise `wake_word.cooldown_secs` (default 8, [architecture 31](../architecture/README.md#31-wake-word-conversation-window)).
- **Pod cannot connect:** use host names, not IP addresses, in certificates and `GATEWAY_HOST`, and check that the pod can reach NTP ([room pod guide](../deployment/pod.md)).
