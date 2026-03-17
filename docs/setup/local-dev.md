# Local development setup

Run Aice as a private local Jarvis on your own machine.

Primary path is `pod-voice`, which runs the voice runtime and pod transport in one process.

## Prerequisites

- Rust stable (`rustup`)
- Ollama running locally (or reachable on your LAN)
- `whisper-cli` + model file
- `piper` + voice model file
- `config.json` in the repo root (copy from `config.example.json`)

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
- `media.apple_music.*` (developer-token fields only)

Apple Music note:
- Status is currently `incomplete` due to high setup complexity.
- Apple Music requires Apple developer credentials/config.
- This does not block the main local runtime; Apple Music is optional.

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

## 5. Hardware notes

- Signal Pod is the intended product target.
- M5Stack ATOM Echo support is experimental and useful for transport/firmware testing.

## Troubleshooting

- Missing STT/TTS output: verify `whisper-cli`, `piper`, and model paths.
- Wake word not triggering: verify `wake_word.enabled` and `wake_word.phrases`.
- Pod cannot connect: verify `pod_bind`, firewall, and same-LAN host IP.
- LLM issues: verify Ollama process and model availability.
