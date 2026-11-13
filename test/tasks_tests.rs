//! Sandboxed local integration tests for `rtkk` task modules: `copy`, `template`,
//! `fetch`, `gen_cert`, `http_get_file`, `command`, and `shell`.

use rtkk::cert::verify_certificate_files;
use rtkk::connector::{Connector, ConnectorType, LocalConnector, TargetConfig};
use rtkk::inventory::FlatInventory;
use rtkk::playbook::{ExecutionEngine, Play, Playbook};
use rtkk::tasks::{Task, execute_task_module};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("rtkk-tasks-test-{prefix}-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        Ok(Self { path })
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn ensure(cond: bool, msg: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.to_string()) }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(unix)]
fn check_unix_mode(path: &Path, expected_octal: u32) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let actual = meta.permissions().mode() & 0o7777;
    ensure(
        actual == expected_octal,
        &format!(
            "expected mode {expected_octal:o} on {}, got {actual:o}",
            path.display()
        ),
    )
}

fn cert_validity_span_hours(pem_path: &Path) -> Result<i64, String> {
    let raw = std::fs::read(pem_path).map_err(|e| e.to_string())?;
    let (_, pem_obj) = x509_parser::pem::parse_x509_pem(&raw)
        .map_err(|e| format!("failed to parse PEM {}: {e}", pem_path.display()))?;
    let cert = pem_obj
        .parse_x509()
        .map_err(|e| format!("failed to parse X.509 {}: {e}", pem_path.display()))?;
    let span_secs = cert.validity().not_after.timestamp() - cert.validity().not_before.timestamp();
    Ok(span_secs / 3_600)
}

#[tokio::test]
async fn copy_task_supports_content_and_streaming_src_with_mode_idempotency() -> Result<(), String>
{
    let dir = TestDir::new("copy")?;
    let conn = Connector::Local(LocalConnector::default());
    let mut vars = json!({ "svc": "etcd" });

    let content_dest = dir.path.join("conf").join("service.env");
    let mut content_task = Task {
        copy: Some(json!({
            "content": "NAME={{ .svc }}\n",
            "dest": content_dest.to_string_lossy(),
            "mode": "0600"
        })),
        ..Task::default()
    };
    let r1 = execute_task_module(&content_task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(r1.changed, "initial copy(content) must report changed=true")?;
    #[cfg(unix)]
    check_unix_mode(&content_dest, 0o600)?;

    let r2 = execute_task_module(&content_task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        !r2.changed,
        "identical copy(content) must report changed=false",
    )?;

    content_task.copy = Some(json!({
        "content": "NAME={{ .svc }}\n",
        "dest": content_dest.to_string_lossy(),
        "mode": "0644"
    }));
    let r3 = execute_task_module(&content_task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        r3.changed,
        "mode change 0600 -> 0644 must report changed=true",
    )?;
    #[cfg(unix)]
    check_unix_mode(&content_dest, 0o644)?;

    verify_streaming_src_copy(&dir.path, &conn, &mut vars).await
}

async fn verify_streaming_src_copy(
    root: &Path,
    conn: &Connector,
    vars: &mut Value,
) -> Result<(), String> {
    let src_file = root.join("large_payload.bin");
    let payload: Vec<u8> = (0..100_000_u32)
        .map(|i| u8::try_from(i % 251).unwrap_or(0))
        .collect();
    std::fs::write(&src_file, &payload).map_err(|e| e.to_string())?;

    let dst_file = root.join("copied_payload.bin");
    let mut src_task = Task {
        copy: Some(json!({
            "src": src_file.to_string_lossy(),
            "dest": dst_file.to_string_lossy(),
            "mode": "0600"
        })),
        ..Task::default()
    };
    let c1 = execute_task_module(&src_task, conn, vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(c1.changed, "initial copy(src) must report changed=true")?;
    let c2 = execute_task_module(&src_task, conn, vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(!c2.changed, "identical copy(src) must report changed=false")?;

    src_task.copy = Some(json!({
        "src": src_file.to_string_lossy(),
        "dest": dst_file.to_string_lossy(),
        "mode": "0644"
    }));
    let c3 = execute_task_module(&src_task, conn, vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(c3.changed, "copy(src) mode change must report changed=true")?;
    #[cfg(unix)]
    check_unix_mode(&dst_file, 0o644)?;
    let copied = std::fs::read(&dst_file).map_err(|e| e.to_string())?;
    ensure(
        copied == payload,
        "streamed copy payload must match byte-for-byte",
    )?;
    Ok(())
}

#[tokio::test]
async fn template_task_renders_idempotently_and_rejects_broken_syntax() -> Result<(), String> {
    let dir = TestDir::new("template")?;
    let conn = Connector::Local(LocalConnector::default());
    let mut vars = json!({ "port": 6443, "host": "10.0.0.61" });

    let good_tmpl = dir.path.join("endpoint.gtmpl");
    std::fs::write(&good_tmpl, "endpoint={{ .host }}:{{ .port }}\n").map_err(|e| e.to_string())?;
    let dest = dir.path.join("out").join("endpoint.conf");

    let mut task = Task {
        template: Some(json!({
            "src": good_tmpl.to_string_lossy(),
            "dest": dest.to_string_lossy(),
            "mode": "0600"
        })),
        ..Task::default()
    };
    let t1 = execute_task_module(&task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(t1.changed, "first template render must report changed=true")?;
    let rendered = std::fs::read_to_string(&dest).map_err(|e| e.to_string())?;
    ensure(
        rendered == "endpoint=10.0.0.61:6443\n",
        &format!("unexpected rendered template content: {rendered}"),
    )?;

    let t2 = execute_task_module(&task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        !t2.changed,
        "second identical template render must report changed=false",
    )?;

    task.template = Some(json!({
        "src": good_tmpl.to_string_lossy(),
        "dest": dest.to_string_lossy(),
        "mode": "0644"
    }));
    let t3 = execute_task_module(&task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        t3.changed,
        "template mode update 0600 -> 0644 must report changed=true",
    )?;

    let bad_tmpl = dir.path.join("broken.gtmpl");
    std::fs::write(&bad_tmpl, "broken={{ .bad").map_err(|e| e.to_string())?;
    let bad_task = Task {
        template: Some(json!({
            "src": bad_tmpl.to_string_lossy(),
            "dest": dir.path.join("never.conf").to_string_lossy()
        })),
        ..Task::default()
    };
    ensure(
        execute_task_module(&bad_task, &conn, &mut vars)
            .await
            .is_err(),
        "broken {{ .bad template syntax must return Err",
    )?;
    Ok(())
}

#[tokio::test]
async fn fetch_task_is_idempotent_when_destination_matches() -> Result<(), String> {
    let dir = TestDir::new("fetch")?;
    let conn = Connector::Local(LocalConnector::default());
    let mut vars = json!({});

    let remote_src = dir.path.join("remote").join("admin.conf");
    std::fs::create_dir_all(dir.path.join("remote")).map_err(|e| e.to_string())?;
    std::fs::write(&remote_src, b"kubeconfig-bytes-v1").map_err(|e| e.to_string())?;
    let local_dest = dir.path.join("fetched").join("admin.conf");

    let task = Task {
        fetch: Some(json!({
            "src": remote_src.to_string_lossy(),
            "dest": local_dest.to_string_lossy()
        })),
        ..Task::default()
    };
    let f1 = execute_task_module(&task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(f1.changed, "initial fetch must report changed=true")?;
    let got = std::fs::read(&local_dest).map_err(|e| e.to_string())?;
    ensure(
        got == b"kubeconfig-bytes-v1",
        "fetched file bytes must match source",
    )?;

    let f2 = execute_task_module(&task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        !f2.changed,
        "second fetch with identical destination bytes must report changed=false",
    )?;
    Ok(())
}

#[tokio::test]
async fn gen_cert_task_supports_99y_default_1y_override_and_ifnotpresent() -> Result<(), String> {
    let dir = TestDir::new("gencert")?;
    let conn = Connector::Local(LocalConnector::default());
    let pki_dir = dir.path.join("pki");
    let remote_ssl = dir.path.join("etc_ssl");
    let mut vars = json!({
        "inventory_hostname": "cp-1",
        "pki_dir": pki_dir.to_string_lossy(),
    });

    let ca_key_remote = remote_ssl.join("ca.key");
    let ca_cert_remote = remote_ssl.join("ca.crt");
    let ca_task = Task {
        gen_cert: Some(json!({
            "cn": "etcd-ca",
            "is_ca": true,
            "out_key": ca_key_remote.to_string_lossy(),
            "out_cert": ca_cert_remote.to_string_lossy(),
            "policy": "ifnotpresent"
        })),
        ..Task::default()
    };
    let c1 = execute_task_module(&ca_task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(c1.changed, "first CA gen_cert must report changed=true")?;
    let c2 = execute_task_module(&ca_task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        !c2.changed,
        "second CA gen_cert with IfNotPresent must report changed=false",
    )?;
    let ca_hours = cert_validity_span_hours(&ca_cert_remote)?;
    ensure(
        (867_240..=867_242).contains(&ca_hours),
        &format!("default CA validity must be ~867240h (99y), got {ca_hours}h"),
    )?;

    verify_leaf_cert_1y_idempotency(&conn, &mut vars, &remote_ssl, &ca_cert_remote).await
}

async fn verify_leaf_cert_1y_idempotency(
    conn: &Connector,
    vars: &mut Value,
    remote_ssl: &Path,
    ca_cert_remote: &Path,
) -> Result<(), String> {
    let leaf_key = remote_ssl.join("apiserver.key");
    let leaf_cert = remote_ssl.join("apiserver.crt");
    let leaf_task = Task {
        gen_cert: Some(json!({
            "cn": "kube-apiserver",
            "is_ca": false,
            "root_key": "ca.key",
            "root_cert": "ca.crt",
            "sans": ["127.0.0.1", "kubernetes.default.svc"],
            "date": "1y",
            "out_key": leaf_key.to_string_lossy(),
            "out_cert": leaf_cert.to_string_lossy(),
            "policy": "ifnotpresent"
        })),
        ..Task::default()
    };
    let l1 = execute_task_module(&leaf_task, conn, vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(l1.changed, "first leaf gen_cert must report changed=true")?;
    let l2 = execute_task_module(&leaf_task, conn, vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        !l2.changed,
        "second leaf gen_cert with IfNotPresent must report changed=false",
    )?;
    let leaf_hours = cert_validity_span_hours(&leaf_cert)?;
    ensure(
        (8_760..=8_762).contains(&leaf_hours),
        &format!("1y leaf validity must be ~8760h, got {leaf_hours}h"),
    )?;
    verify_certificate_files(&leaf_cert, Some(&leaf_key), Some(ca_cert_remote))
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn spawn_http_file_server(payload: Arc<Vec<u8>>) -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let body = Arc::clone(&payload);
            tokio::spawn(async move {
                let mut req_buf = vec![0u8; 2_048];
                let Ok(n) = stream.read(&mut req_buf).await else {
                    return;
                };
                let req_text = req_buf
                    .get(..n)
                    .map_or_else(String::new, |s| String::from_utf8_lossy(s).into_owned());
                let first_line = req_text.lines().next().unwrap_or("");
                if first_line.contains("/fail500/") {
                    let msg = b"upstream failure";
                    let hdr = format!(
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        msg.len()
                    );
                    let _ = stream.write_all(hdr.as_bytes()).await;
                    let _ = stream.write_all(msg).await;
                } else {
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(hdr.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                }
                let _ = stream.shutdown().await;
            });
        }
    });
    Ok(port)
}

#[tokio::test]
async fn http_get_file_verifies_sha256_preserves_tar_gz_and_cleans_up_on_500() -> Result<(), String>
{
    let dir = TestDir::new("http")?;
    let conn = Connector::Local(LocalConnector::default());
    let mut vars = json!({ "inventory_hostname": "localhost" });

    let payload = Arc::new(b"tar-gz-archive-stream-bytes-v1".to_vec());
    let digest = sha256_hex(&payload);
    let port = spawn_http_file_server(Arc::clone(&payload)).await?;

    let dl_dir = dir.path.join("downloads");
    let tar_gz_dest = dl_dir.join("cni-plugins-linux-amd64-v1.6.0.tar.gz");
    let ok_task = Task {
        name: Some("Download CNI archive".to_string()),
        http_get_file: Some(json!({
            "url": format!("http://127.0.0.1:{port}/ok/cni-plugins-linux-amd64-v1.6.0.tar.gz"),
            "dest": tar_gz_dest.to_string_lossy(),
            "sha256": digest,
            "mode": "0644"
        })),
        ..Task::default()
    };
    let d1 = execute_task_module(&ok_task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(d1.changed, "initial http_get_file must report changed=true")?;
    let d2 = execute_task_module(&ok_task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        !d2.changed,
        "second http_get_file with valid sha256 must report changed=false",
    )?;

    let entries: Result<Vec<String>, String> = std::fs::read_dir(&dl_dir)
        .map_err(|e| e.to_string())?
        .map(|res| {
            res.map(|e| e.file_name().to_string_lossy().into_owned())
                .map_err(|e| e.to_string())
        })
        .collect();
    ensure(
        entries? == ["cni-plugins-linux-amd64-v1.6.0.tar.gz"],
        "double extension .tar.gz must be preserved with zero leftover .part files",
    )?;

    verify_http_500_cleanup(&dir.path, port, &conn, &mut vars).await
}

async fn verify_http_500_cleanup(
    root: &Path,
    port: u16,
    conn: &Connector,
    vars: &mut Value,
) -> Result<(), String> {
    let err_dir = root.join("err_downloads");
    std::fs::create_dir_all(&err_dir).map_err(|e| e.to_string())?;
    let broken_dest = err_dir.join("broken-release.tar.gz");
    let fail_task = Task {
        name: Some("Download 500 error".to_string()),
        http_get_file: Some(json!({
            "url": format!("http://127.0.0.1:{port}/fail500/broken-release.tar.gz"),
            "dest": broken_dest.to_string_lossy(),
        })),
        ..Task::default()
    };
    ensure(
        execute_task_module(&fail_task, conn, vars).await.is_err(),
        "HTTP 500 response must return Err",
    )?;
    let remaining: Vec<_> = std::fs::read_dir(&err_dir)
        .map_err(|e| e.to_string())?
        .collect();
    ensure(
        remaining.is_empty(),
        "HTTP 500 failure must clean up temporary .rtkk.part files and leave err_dir empty",
    )?;
    Ok(())
}

#[tokio::test]
async fn command_vs_shell_semantics_and_changed_when_failed_when() -> Result<(), String> {
    let conn = Connector::Local(LocalConnector::default());
    let mut vars = json!({});

    let cmd_task = Task {
        command: Some(json!("echo alpha | tr a-z A-Z")),
        changed_when: Some(json!(false)),
        ..Task::default()
    };
    let cmd_res = execute_task_module(&cmd_task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        cmd_res.stdout.trim() == "alpha | tr a-z A-Z",
        &format!(
            "command must execute argv directly without shell pipe: {}",
            cmd_res.stdout
        ),
    )?;
    ensure(
        !cmd_res.changed,
        "changed_when: false must override changed to false",
    )?;

    let shell_task = Task {
        shell: Some(json!("printf 'alpha' | tr 'a-z' 'A-Z'")),
        changed_when: Some(json!("{{ eq .stdout \"ALPHA\" }}")),
        ..Task::default()
    };
    let sh_res = execute_task_module(&shell_task, &conn, &mut vars)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        sh_res.stdout == "ALPHA" && sh_res.changed,
        "shell must run pipeline and evaluate changed_when against stdout",
    )?;

    verify_failed_when_in_engine().await
}

async fn verify_failed_when_in_engine() -> Result<(), String> {
    let mut host_vars = HashMap::new();
    host_vars.insert("localhost".to_string(), HashMap::new());
    let flat = FlatInventory {
        groups: HashMap::new(),
        host_vars,
    };
    let mut target_configs = HashMap::new();
    target_configs.insert(
        "localhost".to_string(),
        TargetConfig {
            host: "127.0.0.1".to_string(),
            connector_type: Some(ConnectorType::Local),
            ssh: None,
            privilege: None,
        },
    );
    let engine = Arc::new(ExecutionEngine::new(flat, target_configs, 1, None));

    let tolerated_nonzero = Playbook(vec![Play {
        name: Some("Tolerate exit 2 via failed_when".to_string()),
        hosts: "localhost".to_string(),
        tasks: Some(vec![Task {
            shell: Some(json!("printf 'benign'; exit 2")),
            failed_when: Some(json!("{{ eq .rc 99 }}")),
            ..Task::default()
        }]),
        ..Play::default()
    }]);
    engine.run_playbook(&tolerated_nonzero).await?;

    let forced_fail = Playbook(vec![Play {
        name: Some("Fail exit 0 via failed_when".to_string()),
        hosts: "localhost".to_string(),
        tasks: Some(vec![Task {
            shell: Some(json!("printf 'FATAL_ERROR'")),
            failed_when: Some(json!("{{ eq .stdout \"FATAL_ERROR\" }}")),
            ..Task::default()
        }]),
        ..Play::default()
    }]);
    ensure(
        engine.run_playbook(&forced_fail).await.is_err(),
        "failed_when matching stdout must fail play even when exit code is 0",
    )?;
    Ok(())
}
