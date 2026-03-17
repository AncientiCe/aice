# Pod Gateway Operations Runbook

## Purpose

Operate `pod-gateway` as a long-running transport service with health checks and recovery steps.

## Start modes

- Foreground gateway only:
  - `cargo aice-gateway`
- Primary full runtime (recommended):
  - `cargo aice-pod-voice`
- Split runtime (advanced):
  - terminal 1: `cargo aice-gateway`
  - terminal 2: `cargo aice-desktop`

## Quality and safety checks

Run before release:

- `cargo aice-fmt`
- `cargo aice-clippy`
- `cargo aice-audit`
- `cargo aice-test`

## Health checks

- Health endpoint bind is configured in `config.json` at `service.health_bind` (default `127.0.0.1:8780`).
- Probe:

```bash
curl -fsS http://127.0.0.1:8780/healthz
```

Expected body: `ok`.

## Functional verification

1. Start runtime.
2. Connect a pod and send `identify` or `hello`.
3. Confirm ingest events include `device_id`.
4. Confirm pod receives `hello_ack`, `pong`, and `led` messages.

## Common failures

- Port already in use: change `pod_bind` or `service.health_bind` in `config.json`.
- Pod connects but no ingest: verify pod sends JSON text frames and audio payload size is under limits.
- Invalid messages: gateway returns `error` frames (`invalid_message`, `binary_not_supported`, `payload_too_large`).
