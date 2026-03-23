# Local observability runbook

This runbook covers the local Prometheus + Grafana stack for `aice-backend` only.

## Purpose

- scrape local runtime metrics from `aice-backend`
- provide prebuilt Grafana dashboards for backend service health, latency, skills, and dependency calls
- keep data on disk locally for before/after tuning checks

## Prerequisites

- Docker with `docker compose`
- `aice-backend` running with metrics enabled
- `config.json` has:
  - `service.metrics_enabled: true`
  - backend metrics bind configured to match scrape target, typically:
    - `AICE_BACKEND_METRICS_BIND=127.0.0.1:9001`

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

## Backend startup example

```bash
AICE_BACKEND_METRICS_BIND=127.0.0.1:9001 cargo aice-backend
```

Prometheus is preconfigured to scrape:

- `host.docker.internal:9001` (`aice-backend`)

## Dashboard pack

Provisioned from `ops/observability/grafana/provisioning/dashboards/json/`:

- `runtime-overview.json` (`Aice Backend Service Overview`)
- `backend-timings.json` (`Aice Backend Latency`)
- `skills-and-outcomes.json` (`Aice Backend Skills`)
- `timing-deep-dive.json` (`Aice Backend Dependency Latency`)

Primary backend metrics covered by these dashboards:

- HTTP: `backend_http_requests_total`, `backend_http_request_duration_seconds`
- Turn flow: `backend_turn_total`, `backend_turn_duration_seconds`, `backend_turn_stage_duration_seconds`
- Skills: `backend_skill_execute_total`, `backend_skill_execute_duration_seconds`
- External dependencies: `backend_dependency_requests_total`, `backend_dependency_request_duration_seconds`
- mDNS startup: `backend_mdns_advertisement_total`, `backend_mdns_advertisement_duration_seconds`

## Data persistence

Local persistent storage:

- `.local/observability/prometheus`
- `.local/observability/grafana`

## Troubleshooting

- `up{job="aice-backend"} == 0`:
  - confirm `aice-backend` is running
  - confirm `AICE_BACKEND_METRICS_BIND` or `service.metrics_bind` is reachable from Docker target `host.docker.internal:9001`
- Grafana has no dashboards:
  - check `grafana` logs for provisioning errors
  - verify JSON in `ops/observability/grafana/provisioning/dashboards/json/` is valid
- Port conflicts on `3000` or `9090`:
  - stop conflicting local services or adjust compose port mappings
