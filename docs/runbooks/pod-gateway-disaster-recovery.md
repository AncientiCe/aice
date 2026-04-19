# Pod Gateway Disaster Recovery

## Incident classes

- Process crash loop
- Pod disconnections/reconnect storms
- Bad deploy/config regression
- Port bind failures

## Immediate response

1. Check health endpoint:
   - `curl -fsS http://127.0.0.1:8780/healthz`
2. Restart runtime process:
   - restart `cargo aice-gateway` (and `cargo aice-backend` if the paired backend was affected)

## Rollback strategy

1. Revert to last known-good commit.
2. Run quality gates:
   - `cargo aice-fmt`
   - `cargo aice-clippy`
   - `cargo aice-audit`
   - `cargo aice-test`
3. Restart runtime.
4. Validate pod connectivity and ingest.

## Config recovery

1. Restore `config.json` from backup.
2. Confirm:
   - `pod_bind`
   - `service.health_bind`
   - `service.restart_backoff_secs`
3. Restart runtime.

## Data-plane recovery checks

- Pod sends `hello`/`identify` successfully.
- `ping` receives `pong`.
- Audio ingest events appear in logs.
- Gateway can egress `led`/`audio` frames to identified pods.

## Post-incident actions

- Capture failing payload sample/log excerpt.
- Add or extend integration tests in `apps/pod-gateway/tests/ingest_integration.rs`.
- Update runbook with newly discovered failure modes.
