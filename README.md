# Aice: Local-First Streaming Jarvis

Aice is a private, local voice runtime for home automation and media control.

Core flow is built for low latency:
`speech -> STT -> model -> TTS` with streaming where possible.

No cloud lock-in is required for the runtime loop. You run it on your own machine.

## Product direction

- Primary runtime: `pod-voice` (single process, voice loop + pod transport).
- `pod-gateway` remains available as an advanced/internal transport service.
- Hardware direction: Signal Pod is the target device.
- M5Stack ATOM Echo is supported as an experimental test device.
- Skills are pluggable by design (`core-skills`), including macOS Music.app control and smart-home integrations.
- Recommended host for home deployments: Mac mini on macOS.

## Quick start

First-time setup on macOS (install + verification, step by step):
[docs/setup/local-dev.md#0-first-time-macos-setup-checklist](docs/setup/local-dev.md#0-first-time-macos-setup-checklist)

Download all required models in one command:

```bash
./scripts/download-required-models.sh
```

Override model selections (example):

```bash
OLLAMA_MODEL=llama3.2 \
WHISPER_URL=https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin \
PIPER_ONNX_URL=https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/alba/medium/en_GB-alba-medium.onnx \
PIPER_JSON_URL=https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/alba/medium/en_GB-alba-medium.onnx.json \
./scripts/download-required-models.sh
```

1. Create config:

```bash
cp config.example.json config.json
```

2. Run quality gates:

```bash
cargo aice-fmt
cargo aice-clippy
cargo aice-audit
cargo aice-test
```

3. Start the primary runtime:

```bash
cargo aice-pod-voice
```

## Canonical cargo commands

Defined in `.cargo/config.toml`:

- `cargo aice-pod-voice` -> run local Jarvis runtime (primary path)
- `cargo aice-desktop` -> desktop-only runtime
- `cargo aice-gateway` -> standalone pod transport service
- `cargo aice-fmt` -> `cargo fmt --all -- --check`
- `cargo aice-clippy` -> `cargo clippy --all-targets -- -D warnings`
- `cargo aice-audit` -> dependency audit
- `cargo aice-test` -> full test suite

## Releases (v0.1.0)

- Distribution channel for `v0.1.0`: GitHub Releases (no crates.io publishing for this release).
- Official binary support matrix: macOS arm64.
- Runtime compatibility target for `0.1.x`: preserve current `config.example.json` defaults unless a change is explicitly called out in release notes.
- Firmware status: `pod-firmware` remains experimental in `v0.1.0` and is shipped as source + docs only (no firmware artifact).

Install from release assets:

1. Download `aice-v0.1.0-macos-arm64.tar.gz` and `aice-v0.1.0-macos-arm64.tar.gz.sha256` from the GitHub release page.
2. Verify checksum:

```bash
shasum -a 256 -c aice-v0.1.0-macos-arm64.tar.gz.sha256
```

3. Extract and run binaries:

```bash
tar -xzf aice-v0.1.0-macos-arm64.tar.gz
./pod-voice
```

## Skills

Aice is designed for plug-and-play skills.

Current notable integrations:

- Music.app control on macOS (AppleScript bridge)
- Smart-home lighting via Hue
- Weather, time, distance, assistant, memory, and computer control skill interfaces

## macOS Music.app

The media skill uses native Music.app control via AppleScript on macOS.
Authentication and Apple developer token setup are not required.

**Host recommendation:** Run this project on a home Mac mini.

## Repository layout

- `apps/desktop-runner`: local voice runtime binaries (`desktop-runner`, `pod-voice`)
- `apps/pod-gateway`: standalone WebSocket ingest/egress transport
- `crates/core-*`: core pipeline and platform modules
- `crates/core-skills`: pluggable skills
- `pod-firmware`: embedded firmware for experimental M5Stack testing

## Ops and hardware docs

- Local setup: [docs/setup/local-dev.md](docs/setup/local-dev.md)
- Architecture: [docs/architecture/README.md](docs/architecture/README.md)
- Release runbook: [docs/runbooks/release-v0.1.0.md](docs/runbooks/release-v0.1.0.md)
- Experimental M5Stack deployment: [docs/deployment/m5stack-pod.md](docs/deployment/m5stack-pod.md)
- Pod networking: [docs/network/wifi-configuration.md](docs/network/wifi-configuration.md)
- Changelog: [CHANGELOG.md](CHANGELOG.md)
