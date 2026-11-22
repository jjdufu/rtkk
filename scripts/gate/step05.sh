#!/usr/bin/env bash
# Stage B · Step 05 Gate: Offline artifact inventory alignment & mock OCI/Harbor sync via Daemon API (C24, C25).
set -euo pipefail

# shellcheck source=scripts/gate/lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

gate_init "step05"
ensure_rtkk_bin

python3 "${REPO_ROOT}/scripts/check_routes.py"

TOKEN_FILE="${GATE_WORK_DIR}/.rtkk-daemon/admin.token"
"$RTKK_BIN" -l "${GATE_TMP_DIR}/daemon.log" serve \
  --listen "$GATE_BIND_ADDR" \
  --work-dir "$GATE_WORK_DIR" \
  --token-file "$TOKEN_FILE" &
DAEMON_PID=$!
gate_register_pid "$DAEMON_PID"
wait_daemon_ready "${GATE_BASE_URL}/api/v1/health" 20

ADMIN_TOKEN="$(tr -d '\r\n' < "$TOKEN_FILE")"
AUTH_HDR="Authorization: Bearer ${ADMIN_TOKEN}"

test_artifacts_status_inventory_aligned() {
  local code
  code="$(http_status GET "${GATE_BASE_URL}/api/v1/artifacts/status" -H "$AUTH_HDR")"
  assert_eq "$code" "200" "GET /api/v1/artifacts/status must return 200"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.summary.binaries_ready')" "0" "fresh workspace must report 0 ready binaries"

  local bin_total img_total
  bin_total="$(json_field "$GATE_HTTP_BODY" '.summary.binaries_total')"
  img_total="$(json_field "$GATE_HTTP_BODY" '.summary.images_total')"
  if (( bin_total < 5 || img_total < 6 )); then
    echo "Unexpected artifact checklist totals: binaries=${bin_total}, images=${img_total}" >&2
    return 1
  fi

  # Seed one binary artifact and confirm status reflects ready=1 with exact byte size
  mkdir -p "${GATE_WORK_DIR}/downloads/bin"
  printf 'rtkk-dummy-kubectl-binary' > "${GATE_WORK_DIR}/downloads/bin/kubectl"
  chmod 0755 "${GATE_WORK_DIR}/downloads/bin/kubectl"

  code="$(http_status GET "${GATE_BASE_URL}/api/v1/artifacts/status" -H "$AUTH_HDR")"
  assert_eq "$code" "200" "second GET /api/v1/artifacts/status must return 200"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.summary.binaries_ready')" "1" "seeded kubectl must increment binaries_ready to 1"
}

test_daemon_registry_ping_and_sync() {
  local mock_port_file="${GATE_TMP_DIR}/mock-reg.port"
  local mock_script="${GATE_TMP_DIR}/mock_oci_harbor.py"

  cat > "$mock_script" <<'PYEOF'
import hashlib
import io
import json
import sys
import tarfile
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT_FILE = sys.argv[1]

# Minimal valid uncompressed layer tar + OCI config + Docker v2 manifest
layer_buf = io.BytesIO()
with tarfile.open(fileobj=layer_buf, mode="w") as tf:
    data = b"hello-from-rtkk-step05\n"
    info = tarfile.TarInfo(name="hello.txt")
    info.size = len(data)
    tf.addfile(info, io.BytesIO(data))
LAYER_BYTES = layer_buf.getvalue()
LAYER_DIGEST = "sha256:" + hashlib.sha256(LAYER_BYTES).hexdigest()

CONFIG_OBJ = {
    "architecture": "amd64",
    "os": "linux",
    "rootfs": {"type": "layers", "diff_ids": [LAYER_DIGEST]},
    "config": {},
}
CONFIG_BYTES = json.dumps(CONFIG_OBJ).encode("utf-8")
CONFIG_DIGEST = "sha256:" + hashlib.sha256(CONFIG_BYTES).hexdigest()

MANIFEST_OBJ = {
    "schemaVersion": 2,
    "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
    "config": {
        "mediaType": "application/vnd.docker.container.image.v1+json",
        "size": len(CONFIG_BYTES),
        "digest": CONFIG_DIGEST,
    },
    "layers": [
        {
            "mediaType": "application/vnd.docker.image.layer.diff.tar",
            "size": len(LAYER_BYTES),
            "digest": LAYER_DIGEST,
        }
    ],
}
MANIFEST_BYTES = json.dumps(MANIFEST_OBJ).encode("utf-8")
MANIFEST_DIGEST = "sha256:" + hashlib.sha256(MANIFEST_BYTES).hexdigest()

BLOBS = {CONFIG_DIGEST: CONFIG_BYTES, LAYER_DIGEST: LAYER_BYTES}
MANIFESTS = {("library/pause", "3.10.2"): MANIFEST_BYTES, ("library/pause", MANIFEST_DIGEST): MANIFEST_BYTES}
PROJECTS = set()
UPLOADS = {}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        return

    def do_GET(self):
        if self.path == "/v2/":
            self.send_response(200)
            self.send_header("Docker-Distribution-Api-Version", "registry/2.0")
            self.end_headers()
            self.wfile.write(b"{}")
            return
        if self.path.startswith("/api/v2.0/projects"):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            items = [{"name": p} for p in sorted(PROJECTS)]
            self.wfile.write(json.dumps(items).encode("utf-8"))
            return
        if "/manifests/" in self.path and self.path.startswith("/v2/"):
            rest = self.path[len("/v2/"):]
            repo, ref = rest.split("/manifests/", 1)
            raw = MANIFESTS.get((repo, ref))
            if raw is None:
                self.send_response(404)
                self.end_headers()
                return
            self.send_response(200)
            self.send_header("Content-Type", "application/vnd.docker.distribution.manifest.v2+json")
            self.send_header("Docker-Content-Digest", "sha256:" + hashlib.sha256(raw).hexdigest())
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)
            return
        if "/blobs/" in self.path and self.path.startswith("/v2/"):
            digest = self.path.rsplit("/blobs/", 1)[1]
            raw = BLOBS.get(digest)
            if raw is None:
                self.send_response(404)
                self.end_headers()
                return
            self.send_response(200)
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)
            return
        self.send_response(404)
        self.end_headers()

    def do_HEAD(self):
        if "/blobs/" in self.path:
            digest = self.path.rsplit("/blobs/", 1)[1]
            if digest in BLOBS:
                self.send_response(200)
                self.send_header("Content-Length", str(len(BLOBS[digest])))
                self.end_headers()
                return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length > 0 else b""
        if self.path == "/api/v2.0/projects":
            obj = json.loads(body.decode("utf-8") or "{}")
            name = obj.get("project_name", "rtkk")
            if name in PROJECTS:
                self.send_response(409)
            else:
                PROJECTS.add(name)
                self.send_response(201)
            self.end_headers()
            return
        if self.path.endswith("/blobs/uploads/"):
            uid = str(len(UPLOADS) + 1)
            UPLOADS[uid] = bytearray()
            repo = self.path[len("/v2/") : -len("/blobs/uploads/")]
            self.send_response(202)
            self.send_header("Location", f"/v2/{repo}/blobs/uploads/{uid}")
            self.end_headers()
            return
        self.send_response(404)
        self.end_headers()

    def do_PATCH(self):
        length = int(self.headers.get("Content-Length", "0"))
        chunk = self.rfile.read(length) if length > 0 else b""
        uid = self.path.rsplit("/", 1)[1].split("?", 1)[0]
        UPLOADS.setdefault(uid, bytearray()).extend(chunk)
        self.send_response(202)
        self.send_header("Location", self.path)
        self.end_headers()

    def do_PUT(self):
        length = int(self.headers.get("Content-Length", "0"))
        chunk = self.rfile.read(length) if length > 0 else b""
        if "/blobs/uploads/" in self.path:
            path_part, _, query = self.path.partition("?")
            uid = path_part.rsplit("/", 1)[1]
            buf = UPLOADS.pop(uid, bytearray())
            buf.extend(chunk)
            digest = ""
            for kv in query.split("&"):
                if kv.startswith("digest="):
                    digest = kv.split("=", 1)[1]
            BLOBS[digest] = bytes(buf)
            self.send_response(201)
            self.send_header("Docker-Content-Digest", digest)
            self.end_headers()
            return
        if "/manifests/" in self.path:
            rest = self.path[len("/v2/"):]
            repo, ref = rest.split("/manifests/", 1)
            MANIFESTS[(repo, ref)] = chunk
            self.send_response(201)
            self.send_header("Docker-Content-Digest", "sha256:" + hashlib.sha256(chunk).hexdigest())
            self.end_headers()
            return
        self.send_response(404)
        self.end_headers()


srv = HTTPServer(("127.0.0.1", 0), Handler)
with open(PORT_FILE, "w", encoding="utf-8") as fh:
    fh.write(str(srv.server_port))
srv.serve_forever()
PYEOF

  python3 "$mock_script" "$mock_port_file" &
  local mock_pid=$!
  gate_register_pid "$mock_pid"

  local waited=0
  while [[ ! -s "$mock_port_file" ]] && (( waited < 50 )); do
    sleep 0.1
    waited=$((waited + 1))
  done
  local mock_port
  mock_port="$(cat "$mock_port_file")"

  # Configure workspace inventory & single-item images.yaml pointing at local mock OCI/Harbor server
  sed -i.orig \
    -e "s|url: \"10.0.0.10/rtkk\"|url: \"127.0.0.1:${mock_port}/rtkk\"|" \
    -e "s|host: \"10.0.0.10\"|host: \"127.0.0.1:${mock_port}\"|" \
    -e "s|scheme: \"auto\"|scheme: \"http\"|" \
    "${GATE_WORK_DIR}/configs/inventory.yaml"
  rm -f "${GATE_WORK_DIR}/configs/inventory.yaml.orig"

  cat > "${GATE_WORK_DIR}/configs/images.yaml" <<EOF
images:
  - name: pause
    source: "127.0.0.1:${mock_port}/library/pause:3.10.2"
    target: "{{ .registry.url }}/pause:3.10.2"
EOF

  local ping_code
  ping_code="$(http_status POST "${GATE_BASE_URL}/api/v1/registry/ping" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{}')"
  assert_eq "$ping_code" "200" "POST /api/v1/registry/ping must return 200"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.reachable')" "true" "mock registry must report reachable=true"

  local pull_code pull_job pull_state
  pull_code="$(http_status POST "${GATE_BASE_URL}/api/v1/registry/pull" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"image_names":["pause"],"force":true,"insecure":true}')"
  assert_eq "$pull_code" "200" "POST /api/v1/registry/pull must return 200"
  pull_job="$(json_field "$GATE_HTTP_BODY" '.job_id')"
  pull_state="$(wait_job_terminal "$pull_job" "$ADMIN_TOKEN" 20)"
  assert_eq "$pull_state" "succeeded" "registry pull job must succeed"
  assert_file_exists "${GATE_WORK_DIR}/downloads/images/pause.tar"

  local push_code push_job push_state
  push_code="$(http_status POST "${GATE_BASE_URL}/api/v1/registry/push" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"image_names":["pause"]}')"
  assert_eq "$push_code" "200" "POST /api/v1/registry/push must return 200"
  push_job="$(json_field "$GATE_HTTP_BODY" '.job_id')"
  push_state="$(wait_job_terminal "$push_job" "$ADMIN_TOKEN" 20)"
  assert_eq "$push_state" "succeeded" "registry push job must succeed"
}

run_assertion "artifacts-status-inventory-aligned" test_artifacts_status_inventory_aligned
run_assertion "daemon-registry-ping-and-sync" test_daemon_registry_ping_and_sync

gate_pass "step05"
