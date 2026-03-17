# Changelog

All notable changes to this project are documented in this file.

The format is based on Keep a Changelog and this project follows Semantic Versioning.

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
