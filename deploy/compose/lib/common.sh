#!/usr/bin/env bash
# Self-contained Docker Compose lifecycle helper library for rtkk (:19912, uid=10001).
set -euo pipefail

COMPOSE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${COMPOSE_ROOT}/config/.env"
COMPOSE_FILE="${COMPOSE_ROOT}/config/compose.yml"
DATA_DIR="${COMPOSE_ROOT}/data"
LOGS_DIR="${COMPOSE_ROOT}/logs"
BACKUPS_DIR="${COMPOSE_ROOT}/backups"
CONTAINER_NAME="rtkk-daemon"

log_info() {
  printf '[rtkk-compose] %s\n' "$*"
}

log_err() {
  printf '[rtkk-compose] ERROR: %s\n' "$*" >&2
}

load_compose_env() {
  if [[ ! -f "$ENV_FILE" ]]; then
    log_err "missing config/.env at ${ENV_FILE}"
    return 1
  fi
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
  : "${RTKK_VERSION:?RTKK_VERSION must be set in config/.env}"
  : "${RTKK_PORT:?RTKK_PORT must be set in config/.env}"
  export TZ="${TZ:-Asia/Shanghai}"
}

require_docker_compose() {
  if ! command -v docker >/dev/null 2>&1; then
    log_err "docker CLI not found in PATH"
    return 1
  fi
  if ! docker compose version >/dev/null 2>&1; then
    log_err "docker compose plugin not available"
    return 1
  fi
}

compose_cmd() {
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" "$@"
}

svc_prepare_env() {
  load_compose_env
  mkdir -p "$DATA_DIR" "$LOGS_DIR" "$BACKUPS_DIR"

  local image_ref="dex2all/rtkk:${RTKK_VERSION}"
  if [[ "$(id -u)" -eq 0 ]]; then
    mkdir -p "${DATA_DIR}/.rtkk-daemon" "${DATA_DIR}/pki"
    chmod 0700 "${DATA_DIR}/.rtkk-daemon" "${DATA_DIR}/pki"
    chown -R 10001:10001 "$DATA_DIR" "$LOGS_DIR" "$BACKUPS_DIR"
  elif command -v docker >/dev/null 2>&1; then
    if ! docker image inspect "$image_ref" >/dev/null 2>&1; then
      log_info "pulling ${image_ref} before initializing bind-mount ownership"
      docker pull "$image_ref" >/dev/null 2>&1 || true
    fi
    if docker image inspect "$image_ref" >/dev/null 2>&1; then
      docker run --rm --user 0:0 --entrypoint /bin/sh \
        -v "${DATA_DIR}:/mnt/data" \
        -v "${LOGS_DIR}:/mnt/logs" \
        -v "${BACKUPS_DIR}:/mnt/backups" \
        "$image_ref" \
        -c 'mkdir -p /mnt/data/.rtkk-daemon /mnt/data/pki && chmod 0700 /mnt/data/.rtkk-daemon /mnt/data/pki && chown -R 10001:10001 /mnt/data /mnt/logs /mnt/backups'
    else
      mkdir -p -m 0700 "${DATA_DIR}/.rtkk-daemon" "${DATA_DIR}/pki"
    fi
  else
    mkdir -p -m 0700 "${DATA_DIR}/.rtkk-daemon" "${DATA_DIR}/pki"
  fi
}

svc_wait_healthy() {
  local timeout_secs="${1:-60}"
  local elapsed=0
  local state=""

  while (( elapsed < timeout_secs )); do
    state="$(docker inspect --format '{{.State.Status}}' "$CONTAINER_NAME" 2>/dev/null || true)"
    if [[ "$state" == "exited" || "$state" == "dead" ]]; then
      log_err "container ${CONTAINER_NAME} entered terminal state '${state}'"
      compose_cmd logs --tail=80 >&2 || true
      return 1
    fi

    if [[ "$state" == "running" ]] && docker exec "$CONTAINER_NAME" /usr/local/bin/rtkk healthcheck --url http://127.0.0.1:19912/api/v1/health >/dev/null 2>&1; then
      log_info "${CONTAINER_NAME} healthy on 127.0.0.1:${RTKK_PORT}"
      return 0
    fi

    sleep 1
    elapsed=$((elapsed + 1))
  done

  log_err "timed out after ${timeout_secs}s waiting for ${CONTAINER_NAME} healthcheck"
  compose_cmd logs --tail=80 >&2 || true
  return 1
}

svc_start() {
  require_docker_compose
  bash "${COMPOSE_ROOT}/scripts/env.sh"
  compose_cmd up -d
  svc_wait_healthy 60
}

svc_stop() {
  load_compose_env
  require_docker_compose
  compose_cmd stop
}

svc_down() {
  load_compose_env
  require_docker_compose
  compose_cmd down
}

svc_status() {
  load_compose_env
  require_docker_compose
  compose_cmd ps
  printf 'Endpoint: http://127.0.0.1:%s/api/v1/health (host port %s -> container 19912)\n' "$RTKK_PORT" "$RTKK_PORT"
}

svc_remove() {
  load_compose_env
  require_docker_compose
  compose_cmd down -v --remove-orphans

  local image_ref="dex2all/rtkk:${RTKK_VERSION}"
  if [[ "$(id -u)" -ne 0 ]] && ([[ -d "$DATA_DIR" ]] || [[ -d "$LOGS_DIR" ]] || [[ -d "$BACKUPS_DIR" ]]); then
    mkdir -p "$DATA_DIR" "$LOGS_DIR" "$BACKUPS_DIR"
    if docker image inspect "$image_ref" >/dev/null 2>&1; then
      docker run --rm --user 0:0 --entrypoint /bin/sh \
        -v "${DATA_DIR}:/mnt/data" \
        -v "${LOGS_DIR}:/mnt/logs" \
        -v "${BACKUPS_DIR}:/mnt/backups" \
        "$image_ref" \
        -c 'find /mnt/data /mnt/logs /mnt/backups -mindepth 1 -delete'
    fi
  fi

  rm -rf "$DATA_DIR" "$LOGS_DIR" "$BACKUPS_DIR"
  log_info "removed compose stack and runtime directories (data, logs, backups)"
}
