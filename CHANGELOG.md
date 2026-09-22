# Changelog

All notable changes to this project are documented in this file.

The format is based on Keep a Changelog and this project follows Semantic Versioning.

## [Unreleased]

## [v0.3.0] - 2026-09-22

### Added

- On-prem property facilitator for hotels, care homes, and non-clinical ward logistics (`aice-hotels`, `aice-care`, `aice-ward`). Each spoken request becomes a staff-desk ticket. Named tools can be delegated once to a property MCP. Distress and falls in care always escalate. Ward tools outside the non-clinical list are denied and escalated.
- Phone extension to room mapping, and process smoke tests that start each pack binary and check the desk.
- Release archive now includes `aice-backend`, `aice-hotels`, `aice-care`, and `aice-ward`.
- Hotel concierge skill contract (`skill_hotel`) and Memory Palace v0.1.0 on voice turns.
- Public-repo baseline documents: `LICENSE` (Apache-2.0), `SECURITY.md`, `CONTRIBUTING.md`, and `CODE_OF_CONDUCT.md`.

### Changed

- When `property.facilitator_url` is set, `skill_hotel` calls the facilitator MCP. The classifier `hik` list is that server's `tools/list`.
- PCM frame decoding uses `as_chunks` so workspace Clippy passes on Rust 1.98.
- README public-consumer guidance: stability/support matrix, experimental scope boundaries, and repository safety rules for local state and credentials.
- Publication checklist hardening: explicit guidance for secret handling before public visibility changes.

### Release assets

- `aice-v0.3.0-macos-arm64.tar.gz`
- `aice-v0.3.0-macos-arm64.tar.gz.sha256`

## [v0.1.0-rc.1] - 2026-03-17

### Added

- GitHub release workflow for manual tag-driven macOS arm64 binary publishing.
- Release runbook with RC->GA cut steps, soak criteria, promotion rules, and rollback policy.
- macOS arm64 smoke-check script covering tool/model preflight, quality gates, and gateway health probe.
- README release section documenting official v0.1.0 distribution and support contract.

### Release assets

- `aice-v0.1.0-rc.1-macos-arm64.tar.gz`
- `aice-v0.1.0-rc.1-macos-arm64.tar.gz.sha256`

## [v0.1.0] - 2026-03-17

### Planned promotion criteria

- No open release-blocker defects after RC soak.
- `cargo audit` clean on the exact GA tag commit.
- `cargo fmt`, `cargo clippy`, and `cargo test` all pass on the exact GA tag commit.
- RC smoke checks re-run and passing on GA candidate commit.

### Release assets

- `aice-v0.1.0-macos-arm64.tar.gz`
- `aice-v0.1.0-macos-arm64.tar.gz.sha256`
