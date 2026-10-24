# Changelog

All notable changes to this project are documented in this file.

The format is based on Keep a Changelog and this project follows Semantic Versioning.

## [Unreleased]

### Changed

- `aice-skills` and `palace-rs` are fetched over HTTPS (`https://github.com/AncientiCe/...`) instead of SSH, so the workspace builds without a GitHub SSH key. CI no longer needs the `AICE_SKILLS_READ_TOKEN` secret.

### Fixed

- Release `.sha256` files named the archive as `dist/aice-...tar.gz`, so `shasum -a 256 -c` failed next to the downloaded archive. They now name the archive alone. (The `v0.4.0` checksum files were replaced on the release page.)

## [v0.4.0] - 2026-10-24

### Added

- Guest journey timing and slow-turn tracing: `pod_bridge_backend_wait_seconds`, `pod_bridge_synthesis_duration_seconds`, `voice_turn_time_to_first_audio_seconds`, `voice_tts_first_audio_latency_seconds`, `property_ticket_resolve_duration_seconds{pack,tool}`; a turn whose wait after speech reaches `service.slow_turn_threshold_ms` (default 3000) is logged once as `slow_turn` with its full stage timeline and counted in `voice_slow_turns_total{component,slowest_stage}`, with `voice_turn_unaccounted_duration_seconds{component}` for time no stage covers. New "Aice Guest Journey" dashboard; Prometheus also scrapes `pod-gateway` and `aice-hotels`.
- Pod firmware for the Waveshare ESP32-S3-AUDIO-Board (ES8311 speaker, ES7210 mics with a playback echo reference, 7-LED ring, three buttons); the ATOM Echo target is removed. Docs moved to `docs/deployment/pod.md`.
- The pod sets its clock over NTP before any TLS connection and resolves `.local` host names over mDNS.
- Talking over an answer on a full-duplex pod: the pod streams its mic and its playback loopback during answers; the bridge cancels the echo (delay search + NLMS) and stops the answer on speech that clearly beats the predicted echo (`pod_gateway.barge_in_start_level`, `barge_in_start_frames`, `barge_in_echo_ratio`). `pod_bridge_barge_in_total{result}`, `pod_bridge_barge_in_latency_seconds`.
- Short-term conversation context: the backend keeps the last `memory.max_recent_turns` exchanges per session in memory for chat replies, cleared on a new stay or after 10 idle minutes; `playback_interrupted {spoken_ratio}` cuts an interrupted answer to what was heard.

### Fixed

- Duration metrics were exported as summaries, so every dashboard panel built on `histogram_quantile` over `_bucket` series was empty; every `*_seconds` metric is now exported as a histogram with fixed buckets (5 ms to 4 h).
- A failed skill (a provider timed out, a place was not found) was silent on the pod; it now says it could not get the information. This now covers every skill (most returned their error past the apology).
- A pause after "Computer, ..." split one request into fragments ("of Tahini"); speech that goes on before any answer is now sent again together with the first part as one request.
- The bridge ignores answers for turns it has replaced, so a cancelled turn's empty answer no longer swallows the real one.
- The horoscope classifier slot is limited to the 12 signs (it took "today" as the sign).
- A full-duplex pod's last buffered words after an answer were heard as a new request; the bridge waits until the pod's speaker is quiet.
- Room pods could never make hotel requests: the classifier only offered them to clients that declared `skill_hotel`. A backend with a property facilitator now always offers them.
- The classifier's answer was capped at 24 tokens, so hotel requests with details were cut off, failed to parse, and fell back to chat. The cap is 64 (decoding still stops when the JSON is complete).
- Chat replies could promise actions that never happened ("I'll get you an extra pillow right away"). Chat is now told it cannot act and must say the request could not be sent.
- Ollama reloaded the model on every call (about 3 s each) because classifier and memory calls asked for a different context size than the warm-up and answers. Calls to a shared host now use one size, so the model loaded at startup stays loaded.
- Whisper encoded a full 30 s window for every clip; it now encodes only the audio plus a margin, so a short request transcribes about four times faster on the CPU.

- Staff desk login with `staff` and `supervisor` roles, CSRF protection, lockout after five failed passwords, and an append-only audit log; `user add|remove|list` on every pack.
- A service token between backend and facilitator (`service.token`, `property.service_token_file`).
- Room pod provisioning: pods enrol with a nonce, a supervisor assigns the room, the pod receives a device token; `device list|assign|revoke`. The backend verifies the token on every connection and turn and takes the room from it.
- TLS from a property-local CA (`crates/core-tls`): facilitator `tls.mode: auto|files|off`, backend `service.tls`, bridge `pod_gateway.tls`; `tls fingerprint|issue`.
- Stays with private, stay-scoped memory; per-stay memory consent; `memory_retention` `keep` (default for hotels and care), `archive`, or `wipe_on_close` (default for wards); a backend retention job; `stay open|close|list`.
- Staff alert tiers, webhook and property-MCP channels, and per-tool SLA re-escalation that survives restarts.
- Room bridge (`pod-gateway`): speech detection, turns on the backend with the pod's token, Piper speech back to the pod paced for its playback queue.
- Device heartbeats and `device_offline` tickets; a help button (long press) that raises a ticket without speech; a privacy mute (double tap).
- Signed pod firmware updates with staged rollout (`firmware keygen|publish|rollout|pod-header`) and lost-token recovery.
- Admission control (`service.max_concurrent_turns`, `service.turn_queue_max_wait_ms`), a Whisper worker pool (`stt.workers`), and LLM host failover (`ollama_urls`).
- `room-loadtest` and a CI capacity gate; CI builds the pod firmware; Linux x86_64 and aarch64 release archives.
- Pilot runbook, DPIA template, and clinical safety starter.

### Changed

- Memory Palace moves from `mempalace-rust` `v0.1.0` to `palace-rs` `v0.18.0` (renamed repository `AncientiCe/palace-rs`, library `palace`), which drops `serde_yaml` and its unmaintained `unsafe-libyaml` (RUSTSEC-2024-0449). Existing palaces are marked with their embedding model on first open; no vectors are rewritten. The default data directory is `~/.palace`; installs that only have `~/.mempalace` keep using it. Stay purges also clear the new wing registry, usage events, mined-file records, and feedback for the stay.
- The facilitator refuses plain HTTP off loopback, and a property backend or bridge refuses plain HTTP/WebSocket on a network bind unless `service.allow_plaintext_lan` is set.
- In a property deployment a turn without an open, consented stay remembers nothing, and the shared journal skill is hidden from rooms.
- `pod-gateway` is no longer a logging stub; its old `run_gateway` library API is replaced by `spawn_bridge`.
- Pods take their room from the desk, not from firmware (`DEVICE_ROOM` is gone).

### Security

- Previously the desk, ticket API and `/mcp` were reachable without credentials, and a room's turn stream accepted any client-supplied room.

### Release assets

- `aice-v0.4.0-macos-arm64.tar.gz` and `.sha256`
- `aice-v0.4.0-linux-x86_64.tar.gz` and `.sha256`
- `aice-v0.4.0-linux-aarch64.tar.gz` and `.sha256`

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
