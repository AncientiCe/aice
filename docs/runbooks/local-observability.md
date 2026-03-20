# Local observability runbook

This runbook covers the local Prometheus + Grafana stack for `desktop-runner`.

## Purpose

- scrape local runtime metrics from `desktop-runner`
- provide prebuilt Grafana dashboards for runtime, timings, and skill outcomes
- keep data on disk locally for before/after tuning checks

## Prerequisites

- Docker with `docker compose`
- `desktop-runner` running with metrics enabled
- `config.json` has:
  - `service.metrics_enabled: true`
  - `service.metrics_bind: "127.0.0.1:9000"` (or matching bind used by Prometheus target)

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

## Dashboard pack

Provisioned from `ops/observability/grafana/provisioning/dashboards/json/`:

- `runtime-overview.json`
- `timing-deep-dive.json`
- `skills-and-outcomes.json`

## Data persistence

Local persistent storage:

- `.local/observability/prometheus`
- `.local/observability/grafana`

## Troubleshooting

- `up{job="desktop-runner"} == 0`:
  - confirm `desktop-runner` is running
  - confirm `service.metrics_bind` is reachable from Docker target `host.docker.internal:9000`
- Grafana has no dashboards:
  - check `grafana` logs for provisioning errors
  - verify JSON in `ops/observability/grafana/provisioning/dashboards/json/` is valid
- Port conflicts on `3000` or `9090`:
  - stop conflicting local services or adjust compose port mappings
