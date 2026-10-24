# v0.4.0 Release Runbook

## Purpose

Cut and publish `v0.4.0` from a green `main` commit. This release makes aice deployable in hotels, care homes and wards: staff logins, device tokens, TLS, per-stay memory, staff alerts, the room bridge and pod firmware.

## Scope

- Distribution: GitHub Releases.
- Official binaries: macOS arm64, Linux x86_64, Linux aarch64.
- Archive members: `aice-backend`, `aice-hotels`, `aice-care`, `aice-ward`, `pod-gateway`, `room-loadtest`.
- Asset names: `aice-v0.4.0-<platform>.tar.gz` and `aice-v0.4.0-<platform>.tar.gz.sha256`.
- `pod-firmware` is built in CI but not in the archive.

## Gates on the exact commit

CI and Release Build Check must be green on the commit that will be tagged. Do not push the tag before they are. Locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used
cargo audit --deny warnings
cargo test --workspace
```

On a Mac arm64 host, also run `./scripts/release/smoke-macos-arm64.sh`.

## Cut

```bash
git tag v0.4.0
git push origin v0.4.0
```

Then run the GitHub Actions **Release** workflow manually with `release_tag=v0.4.0`.

The release notes are the tag's `## [v0.4.0]` section of `CHANGELOG.md`, copied verbatim. The workflow fails before building if the tagged `CHANGELOG.md` has no such section, so move the Unreleased entries under the version heading in the commit you tag.

Confirm the release page has all six assets and that each tarball contains the six binaries.

## Breaking changes to call out

- A property backend requires device tokens on `/turns/stream`; `aice-macos` cannot connect to one until it sends them. Home installs are unchanged.
- `pod-gateway`'s `run_gateway` is replaced by `spawn_bridge`; firmware no longer has `DEVICE_ROOM`.
- `PropertyClient::new` takes a service token.
- Configs that omit `wake_word.cooldown_secs` get 8 s instead of 2 s.

## Rollback

Do not move an existing tag. Fix on `main`, let CI go green, and cut the next patch tag.
