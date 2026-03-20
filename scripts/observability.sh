#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="${ROOT_DIR}/ops/observability/docker-compose.yml"

cmd="${1:-}"
if [[ -z "${cmd}" ]]; then
  echo "usage: scripts/observability.sh {up|down|restart|status|logs|config}" >&2
  exit 1
fi

case "${cmd}" in
  up)
    docker compose -f "${COMPOSE_FILE}" up -d
    ;;
  down)
    docker compose -f "${COMPOSE_FILE}" down
    ;;
  restart)
    docker compose -f "${COMPOSE_FILE}" down
    docker compose -f "${COMPOSE_FILE}" up -d
    ;;
  status)
    docker compose -f "${COMPOSE_FILE}" ps
    ;;
  logs)
    docker compose -f "${COMPOSE_FILE}" logs -f --tail=200
    ;;
  config)
    docker compose -f "${COMPOSE_FILE}" config
    ;;
  *)
    echo "unknown command: ${cmd}" >&2
    exit 1
    ;;
esac
