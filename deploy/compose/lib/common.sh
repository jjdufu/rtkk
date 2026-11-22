#!/usr/bin/env bash
# Self-contained Docker Compose lifecycle helper library for rtkk (:19912, uid=10001).
set -euo pipefail

COMPOSE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly COMPOSE_ROOT
readonly ENV_FILE="${COMPOSE_ROOT}/config/.env"
readonly COMPOSE_FILE="${COMPOSE_ROOT}/config/compose.yml"
readonly DATA_DIR="${COMPOSE_ROOT}/data"
readonly LOGS_DIR="${COMPOSE_ROOT}/logs"
readonly BACKUPS_DIR="${COMPOSE_ROOT}/backups"
readonly CONTAINER_NAME="rtkk-daemon"

log_info() {
  printf '[rtkk-compose] %s\n' "$*"
}

log_err() {
  printf '[rtkk-compose] ERROR: %s\n' "$*" >&2
}

assert_safe_runtime_dirs() {
  if [[ -z "${COMPOSE_ROOT:-}" || "$COMPOSE_ROOT" == "/" ]]; then
    log_err "unsafe COMPOSE_ROOT='${COMPOSE_ROOT:-}'"
    exit 1
  fi

  if [[ -n "${HOME:-}" ]]; then
    local home_real=""
    home_real="$(cd "$HOME" 2>/dev/null && pwd -P || printf '%s' "$HOME")"
    if [[ "$COMPOSE_ROOT" == "$HOME" || "$COMPOSE_ROOT" == "$home_real" ]]; then
      log_err "COMPOSE_ROOT must not equal HOME ('${COMPOSE_ROOT}')"
      exit 1
    fi
  fi

  if [[ ! -f "${COMPOSE_ROOT}/config/compose.yml" || -L "${COMPOSE_ROOT}/config/compose.yml" ]]; then
    log_err "missing or symlinked compose definition at ${COMPOSE_ROOT}/config/compose.yml"
    exit 1
  fi

  local dir base parent_dir parent_real
  for dir in "$DATA_DIR" "$LOGS_DIR" "$BACKUPS_DIR"; do
    if [[ -L "$dir" ]]; then
      log_err "refusing symlink runtime directory: ${dir}"
      exit 1
    fi
    base="$(basename "$dir")"
    case "$base" in
      data|logs|backups) ;;
      *)
        log_err "unexpected runtime directory basename '${base}' (${dir})"
        exit 1
        ;;
    esac
    parent_dir="$(dirname "$dir")"
    if [[ -d "$parent_dir" ]]; then
      parent_real="$(cd "$parent_dir" && pwd -P)"
      if [[ "$parent_real" != "$COMPOSE_ROOT" ]]; then
        log_err "runtime directory parent '${parent_real}' does not match COMPOSE_ROOT '${COMPOSE_ROOT}'"
        exit 1
      fi
    fi
  done

  if [[ -L "${DATA_DIR}/.rtkk-daemon" || -L "${DATA_DIR}/pki" ]]; then
    log_err "refusing symlink inside sensitive runtime directory under ${DATA_DIR}"
    exit 1
  fi
}

load_compose_env() {
  if [[ ! -f "$ENV_FILE" || -L "$ENV_FILE" ]]; then
    log_err "missing or symlinked config/.env at ${ENV_FILE}"
    exit 1
  fi

  local parsed_version=""
  local parsed_port=""
  local parsed_tz="Asia/Shanghai"
  local raw_line line key val

  while IFS= read -r raw_line || [[ -n "$raw_line" ]]; do
    raw_line="${raw_line%$'\r'}"
    line="${raw_line#"${raw_line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    if [[ -z "$line" || "$line" == \#* ]]; then
      continue
    fi
    if [[ "$line" != *=* ]]; then
      log_err "malformed KEY=VALUE entry in ${ENV_FILE}: '${line}'"
      exit 1
    fi

    key="${line%%=*}"
    val="${line#*=}"
    key="${key#"${key%%[![:space:]]*}"}"
    key="${key%"${key##*[![:space:]]}"}"
    val="${val#"${val%%[![:space:]]*}"}"
    val="${val%"${val##*[![:space:]]}"}"

    if [[ "${#val}" -ge 2 ]]; then
      if [[ "${val:0:1}" == '"' && "${val: -1}" == '"' ]]; then
        val="${val:1:${#val}-2}"
      elif [[ "${val:0:1}" == "'" && "${val: -1}" == "'" ]]; then
        val="${val:1:${#val}-2}"
      fi
    fi

    case "$key" in
      RTKK_VERSION)
        parsed_version="$val"
        ;;
      RTKK_PORT)
        parsed_port="$val"
        ;;
      TZ)
        parsed_tz="$val"
        ;;
      *)
        log_err "unsupported variable '${key}' in ${ENV_FILE} (allowed: RTKK_VERSION, RTKK_PORT, TZ)"
        exit 1
        ;;
    esac
  done < "$ENV_FILE"

  if [[ -z "$parsed_version" ]] || ! [[ "$parsed_version" =~ ^[A-Za-z0-9._-]+$ ]]; then
    log_err "invalid RTKK_VERSION='${parsed_version}' in ${ENV_FILE} (expected ^[A-Za-z0-9._-]+$)"
    exit 1
  fi

  if [[ -z "$parsed_port" ]] || ! [[ "$parsed_port" =~ ^[0-9]+$ ]] || (( 10#$parsed_port < 1 || 10#$parsed_port > 65535 )); then
    log_err "invalid RTKK_PORT='${parsed_port}' in ${ENV_FILE} (expected integer 1..65535)"
    exit 1
  fi

  if [[ -z "$parsed_tz" ]] || ! [[ "$parsed_tz" =~ ^[A-Za-z0-9/_+-]+$ ]]; then
    log_err "invalid TZ='${parsed_tz}' in ${ENV_FILE}"
    exit 1
  fi

  RTKK_VERSION="$parsed_version"
  RTKK_PORT="$parsed_port"
  TZ="$parsed_tz"
  export RTKK_VERSION RTKK_PORT TZ
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
  assert_safe_runtime_dirs
  load_compose_env
  mkdir -p -- "$DATA_DIR" "$LOGS_DIR" "$BACKUPS_DIR"
  assert_safe_runtime_dirs

  local image_ref="dex2all/rtkk:${RTKK_VERSION}"
  if [[ "$(id -u)" -eq 0 ]]; then
    mkdir -p -- "${DATA_DIR}/.rtkk-daemon" "${DATA_DIR}/pki"
    chmod 0700 "${DATA_DIR}/.rtkk-daemon" "${DATA_DIR}/pki"
    chown -RP 10001:10001 "$DATA_DIR" "$LOGS_DIR" "$BACKUPS_DIR"
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
        -c 'test ! -L /mnt/data/.rtkk-daemon && test ! -L /mnt/data/pki && mkdir -p /mnt/data/.rtkk-daemon /mnt/data/pki && chmod 0700 /mnt/data/.rtkk-daemon /mnt/data/pki && chown -RP 10001:10001 /mnt/data /mnt/logs /mnt/backups'
    else
      mkdir -p -m 0700 -- "${DATA_DIR}/.rtkk-daemon" "${DATA_DIR}/pki"
    fi
  else
    mkdir -p -m 0700 -- "${DATA_DIR}/.rtkk-daemon" "${DATA_DIR}/pki"
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
  assert_safe_runtime_dirs
  load_compose_env
  require_docker_compose
  compose_cmd down -v --remove-orphans

  local image_ref="dex2all/rtkk:${RTKK_VERSION}"
  if [[ "$(id -u)" -ne 0 ]] && ([[ -d "$DATA_DIR" ]] || [[ -d "$LOGS_DIR" ]] || [[ -d "$BACKUPS_DIR" ]]); then
    mkdir -p -- "$DATA_DIR" "$LOGS_DIR" "$BACKUPS_DIR"
    assert_safe_runtime_dirs
    if docker image inspect "$image_ref" >/dev/null 2>&1; then
      docker run --rm --user 0:0 --entrypoint /bin/sh \
        -v "${DATA_DIR}:/mnt/data" \
        -v "${LOGS_DIR}:/mnt/logs" \
        -v "${BACKUPS_DIR}:/mnt/backups" \
        "$image_ref" \
        -c 'find -P /mnt/data /mnt/logs /mnt/backups -mindepth 1 -delete'
    fi
  fi

  rm -rf -- "$DATA_DIR" "$LOGS_DIR" "$BACKUPS_DIR"
  log_info "removed compose stack and runtime directories (data, logs, backups)"
}
