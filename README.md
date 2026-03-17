# Aice: Local-First Streaming Jarvis

Aice is a private, local voice runtime for home automation and media control.

Core flow is built for low latency:
`speech -> STT -> model -> TTS` with streaming where possible.

No cloud lock-in is required for the runtime loop. You run it on your own PC or mini computer.

## Product direction

- Primary runtime: `pod-voice` (single process, voice loop + pod transport).
- `pod-gateway` remains available as an advanced/internal transport service.
- Hardware direction: Signal Pod is the target device.
- M5Stack ATOM Echo is supported as an experimental test device.
- Skills are pluggable by design (`core-skills`), including Apple Music on Windows and smart-home integrations.

## Quick start

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

## Skills

Aice is designed for plug-and-play skills.

Current notable integrations:

- Apple Music control on Windows (MusicKit bridge)
- Smart-home lighting via Hue
- Weather, time, distance, assistant, memory, and computer control skill interfaces

## Apple Music (Windows)

Apple Music control uses MusicKit bridge playback and developer-token-based catalog operations.
The old Rust OAuth helper binaries were removed; authentication is handled in the MusicKit runtime path.

**Status:** `incomplete` (high setup complexity).  
Apple Music integration currently requires Apple developer setup (`team_id`, `key_id`, `.p8` private key).  
This requirement applies only to the Apple Music skill; the core local Jarvis runtime works without it.

## Repository layout

- `apps/desktop-runner`: local voice runtime binaries (`desktop-runner`, `pod-voice`)
- `apps/pod-gateway`: standalone WebSocket ingest/egress transport
- `crates/core-*`: core pipeline and platform modules
- `crates/core-skills`: pluggable skills
- `pod-firmware`: embedded firmware for experimental M5Stack testing

## Ops and hardware docs

- Local setup: [docs/setup/local-dev.md](docs/setup/local-dev.md)
- Architecture: [docs/architecture/README.md](docs/architecture/README.md)
- Experimental M5Stack deployment: [docs/deployment/m5stack-pod.md](docs/deployment/m5stack-pod.md)
- Pod networking: [docs/network/wifi-configuration.md](docs/network/wifi-configuration.md)
