# v0.3.0 Release Runbook

## Purpose

Cut and publish `v0.3.0` from a green `main` commit. This release adds the on-prem property facilitator beside `aice-backend`.

## Scope

- Distribution: GitHub Releases.
- Official binaries: macOS arm64.
- Archive members: `aice-backend`, `aice-hotels`, `aice-care`, `aice-ward`.
- Asset names: `aice-v0.3.0-macos-arm64.tar.gz` and `aice-v0.3.0-macos-arm64.tar.gz.sha256`.
- `pod-firmware` stays experimental and is not in the archive.

## Gates on the exact commit

CI and Release Build Check must be green on the commit that will be tagged. Locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used
cargo audit --deny warnings
cargo test --workspace
```

On a Mac arm64 host, also run `./scripts/release/smoke-macos-arm64.sh`.

## Cut

The Release workflow accepts `vMAJOR.MINOR.PATCH` and `vMAJOR.MINOR.PATCH-rc.N`. It runs from `main` and checks out the tag you name.

```bash
git tag v0.3.0
git push origin v0.3.0
```

Then run the GitHub Actions **Release** workflow manually with `release_tag=v0.3.0`.

Confirm the release page has both assets and that the tarball contains the four binaries.

## Rollback

Do not move the `v0.3.0` tag. Fix on `main`, let CI go green, and cut `v0.3.1`.
