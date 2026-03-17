# Local voice AI status

Aice is focused on one clear outcome: a private, local, streaming Jarvis runtime for home use.

## Current direction

- Primary runtime: `pod-voice`
- Transport component: `pod-gateway` (advanced/internal)
- Skills-first architecture for easy plug-in integrations
- Signal Pod target hardware, M5Stack experimental

## Command interface

Canonical workflow is cargo aliases:

- `cargo aice-fmt`
- `cargo aice-clippy`
- `cargo aice-audit`
- `cargo aice-test`
- `cargo aice-pod-voice`

## Core docs

- [README.md](../README.md)
- [Setup](setup/local-dev.md)
- [Architecture](architecture/README.md)
- [Experimental M5Stack deployment](deployment/m5stack-pod.md)
- [Pod networking](network/wifi-configuration.md)
