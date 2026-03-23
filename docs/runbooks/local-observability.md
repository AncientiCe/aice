# Local observability runbook

This runbook covers the local Prometheus + Grafana stack for `desktop-runner`, `aice-backend`, and `aice-macos`.

## Purpose

- scrape local runtime metrics from `desktop-runner`, `aice-backend`, and `aice-macos`
- provide prebuilt Grafana dashboards for runtime, timings, and skill outcomes
- keep data on disk locally for before/after tuning checks

## Prerequisites

- Docker with `docker compose`
- target runtime(s) running with metrics enabled
- `config.json` has:
  - `service.metrics_enabled: true`
  - `service.metrics_bind: "127.0.0.1:9000"` for `desktop-runner` (or matching scrape target)
  - For split services, use distinct binds to avoid collisions:
    - `AICE_BACKEND_METRICS_BIND=127.0.0.1:9001`
    - `AICE_MACOS_METRICS_BIND=127.0.0.1:9002`

## Start and stop

Start:

```bash
./scripts/observability.sh up
```

Validate compose:

```bash
./scripts/observability.sh config
```

Status:

```bash
./scripts/observability.sh status
```

Logs:

```bash
./scripts/observability.sh logs
```

Stop:

```bash
./scripts/observability.sh down
```

## Access

- Grafana: `http://127.0.0.1:3000`
- Prometheus: `http://127.0.0.1:9090`

Both are loopback-only by default in `ops/observability/docker-compose.yml`.

## Split-service startup example

```bash
AICE_BACKEND_METRICS_BIND=127.0.0.1:9001 cargo aice-backend
```

```bash
AICE_MACOS_METRICS_BIND=127.0.0.1:9002 cargo aice-macos
```

Prometheus is preconfigured to scrape:

- `host.docker.internal:9000` (`desktop-runner`)
- `host.docker.internal:9001` (`aice-backend`)
- `host.docker.internal:9002` (`aice-macos`)

## Dashboard pack

Provisioned from `ops/observability/grafana/provisioning/dashboards/json/`:

- `runtime-overview.json`
- `timing-deep-dive.json`
- `skills-and-outcomes.json`
- `backend-timings.json`
- `frontend-timings.json`

New latency-attribution metric used by backend dashboard:
- `backend_turn_stage_duration_seconds{stage}` with stages such as `classify_intent`, `skill_execute`, `answer_compose`, `chat_generate`, `sse_write`

Suggested latency gates for optimization passes:
- `voice_turn_time_to_first_audio_seconds` p95 <= 2.0s
- end-to-end `voice_stage_duration_seconds{stage="orchestrator"}` p95 <= 4.5s

## Data persistence

Local persistent storage:

- `.local/observability/prometheus`
- `.local/observability/grafana`

## Troubleshooting

- `up{job="aice-backend"} == 0`:
  - confirm `aice-backend` is running
  - confirm `AICE_BACKEND_METRICS_BIND` or `service.metrics_bind` is reachable from Docker target `host.docker.internal:9001`
- `up{job="aice-macos"} == 0`:
  - confirm `aice-macos` is running
  - confirm `AICE_MACOS_METRICS_BIND` or `service.metrics_bind` is reachable from Docker target `host.docker.internal:9002`
- `up{job="desktop-runner"} == 0`:
  - confirm `desktop-runner` is running
  - confirm `service.metrics_bind` is reachable from Docker target `host.docker.internal:9000`
- Grafana has no dashboards:
  - check `grafana` logs for provisioning errors
  - verify JSON in `ops/observability/grafana/provisioning/dashboards/json/` is valid
- Port conflicts on `3000` or `9090`:
  - stop conflicting local services or adjust compose port mappings
