# Changelog

All notable changes to this project are documented in this file.

The format is based on Keep a Changelog and this project follows Semantic Versioning.

## [Unreleased]

## [v0.3.2] - 2026-09-22

### Fixed

- macOS publish no longer restores a Cargo `target` cache. That cache kept an `ort-sys` linker path for `libclang_rt.osx` from a different Xcode, so `aice-backend` failed to link. `v0.3.1` was tagged and the publish job failed before any assets were uploaded.

### Release assets

- `aice-v0.3.2-macos-arm64.tar.gz`
- `aice-v0.3.2-macos-arm64.tar.gz.sha256`

## [v0.3.1] - 2026-09-22

### Fixed

- macOS release build of `whisper-rs`. The whisper library target was compiled without `-std=gnu++17`, so current Apple Clang rejected `[[noreturn]]` and `constexpr`. `CMAKE_CXX_STANDARD=17` is now set for that build. `v0.3.0` was tagged but the macOS publish job failed before any assets were uploaded.

`v0.3.1` has no release assets. The publish job failed before upload.

## [v0.3.0] - 2026-09-22

Changes after `v0.2.0` (`fc3e032`).

### Added

- Hotel concierge skill. The classifier accepts `skill_hotel` with a closed list of in-room intent kinds and slots (`5c5273d`).
- Memory Palace `v0.1.0` on voice turns (`8402943`). Every non-empty turn can add wake-up context, semantic recall, and knowledge-graph facts to the spoken answer, then ingest the outcome. Journal entries can be mirrored into the palace. New config: `palace_recall_results`, `palace_recall_min_similarity`, `palace_recall_max_chars`, `palace_journal_mirror_enabled`, `palace_kg_enabled`.
- On-prem property facilitator (`988df75`): `aice-hotels`, `aice-care`, and `aice-ward`. A request becomes a staff-desk ticket. Named tools delegate once to a property MCP and stay escalated if that MCP is down. Care distress and falls always escalate. Ward tools outside the non-clinical list are denied and escalated. A phone extension maps to a room.
- Process smoke tests that start each pack binary and check the desk. CI builds the three packs on Linux, macOS, and Windows. The macOS release archive contains `aice-backend`, `aice-hotels`, `aice-care`, and `aice-ward`.
- Release workflow accepts `vMAJOR.MINOR.PATCH` and `vMAJOR.MINOR.PATCH-rc.N`, including `v0.3.0`.

### Changed

- With `property.facilitator_url` set, `skill_hotel` calls the facilitator. The classifier `hik` enum is that server's `tools/list`. Without the URL, the intent still goes to the connected frontend.
- The backend depends on `mempalace` tag `v0.1.0` instead of a raw git revision.

### Fixed

- Hotel dispatch test no longer uses `expect()` (`03c680e`).
- PCM frame decoding uses `as_chunks` so workspace Clippy passes on Rust 1.98 (`c547280`).

`v0.3.0` has no release assets. The publish job failed before upload.

## [v0.2.0] - 2026-04-19

Published as tag `v0.2.0` (`fc3e032`) without a changelog entry. That tree already contained `LICENSE`, `SECURITY.md`, `CONTRIBUTING.md`, and `CODE_OF_CONDUCT.md`, plus the public README guidance that had been left under Unreleased.

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
