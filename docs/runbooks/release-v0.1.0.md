# v0.1.0 Release Runbook (RC -> GA)

## Purpose

Standardize how `v0.1.0-rc.1` and `v0.1.0` are cut, validated, published, and rolled back for Aice.

## Scope and support contract

- Distribution channel: GitHub Releases only.
- Official binary support for `v0.1.0`: macOS arm64.
- Release assets: `pod-voice`, `desktop-runner`, `pod-gateway` tarball and SHA256 checksum.
- Firmware (`pod-firmware`) remains experimental and source-driven for `v0.1.0` (no firmware release artifact).
- Backward compatibility target: preserve current runtime and config defaults from `config.example.json` for the `0.1.x` line unless explicitly documented in release notes.

## Branch and tag policy

1. Cut stabilization branch from `main`:
   - `git checkout -b release/v0.1.0`
2. Scope freeze after branch cut:
   - only bug fixes, reliability fixes, release blocker fixes, and release docs updates.
3. Tags:
   - RC: `v0.1.0-rc.1`
   - GA: `v0.1.0`
4. All post-cut commits on `release/v0.1.0` require release-manager approval.
5. CI must be green on the exact commit that will be tagged.

## Mandatory quality gates

Run from repo root on the release branch and on the exact tag commit:

```bash
cargo aice-fmt
cargo aice-clippy
cargo aice-audit
cargo aice-test
```

Equivalent raw commands:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo audit
cargo test --all
```

## RC cut procedure (`v0.1.0-rc.1`)

1. Ensure branch is up to date and all quality gates pass.
2. Run macOS arm64 smoke checks:

```bash
./scripts/release/smoke-macos-arm64.sh
```

3. Tag and push:

```bash
git tag v0.1.0-rc.1
git push origin release/v0.1.0
git push origin v0.1.0-rc.1
```

4. Run GitHub Actions `Release` workflow manually with:
   - `release_tag=v0.1.0-rc.1`
5. Verify release assets:
   - `aice-v0.1.0-rc.1-macos-arm64.tar.gz`
   - `aice-v0.1.0-rc.1-macos-arm64.tar.gz.sha256`

## Soak window (RC validation)

Soak period: March 24, 2026 through March 30, 2026.

Required checks:

1. Startup preflight with local dependencies (`ollama`, `whisper-cli`, `piper`) present.
2. `pod-voice` end-to-end voice turn (`speech -> STT -> LLM -> TTS`).
3. `pod-gateway` health and pod ingress/egress validation.
4. At least one real skill path (weather or media) and one policy-denied path.
5. Metrics/log review confirms expected success and error emissions.

## GA promotion criteria (`v0.1.0`)

All criteria must be true:

1. Zero open release-blocker defects at end of soak.
2. `cargo audit` reports no unresolved advisories.
3. Full quality gates pass on the exact GA tag commit.
4. RC smoke tests are re-run and pass on GA candidate commit.

## GA cut procedure (`v0.1.0`)

1. Confirm promotion criteria are met.
2. Tag and push:

```bash
git tag v0.1.0
git push origin v0.1.0
```

3. Run GitHub Actions `Release` workflow manually with:
   - `release_tag=v0.1.0`
4. Verify release assets:
   - `aice-v0.1.0-macos-arm64.tar.gz`
   - `aice-v0.1.0-macos-arm64.tar.gz.sha256`

## Rollback and hotfix policy

If GA criteria fail, do not publish `v0.1.0`. Continue on `release/v0.1.0` until blocker resolution.

If RC or GA artifact is bad:

1. Stop promotion.
2. Revert or fix on `release/v0.1.0`.
3. Re-run quality gates and smoke checks.
4. Cut a new RC tag (`v0.1.0-rc.2`, `v0.1.0-rc.3`, ...).
5. Use [Pod Gateway Disaster Recovery](./pod-gateway-disaster-recovery.md) for runtime incident handling.
