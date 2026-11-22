//! Integration tests for Step 01: concurrent [`EventSink`] & [`TemplateRegistry`] isolation
//! and idempotent [`Workspace`] default `configs/` extraction.

use rtkk::connector::{ConnectorType, TargetConfig};
use rtkk::inventory::FlatInventory;
use rtkk::logging::{ExecutionEvent, MemoryCaptureSink};
use rtkk::playbook::{ExecutionEngine, Play, Playbook};
use rtkk::tasks::Task;
use rtkk::template::TemplateRegistry;
use rtkk::workspace::Workspace;
use serde_json::{Value, json};
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct TempSandbox {
    path: PathBuf,
}

impl TempSandbox {
    fn new(prefix: &str) -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("rtkk-s01-{prefix}-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        Ok(Self { path })
    }
}

impl Drop for TempSandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn ensure(cond: bool, msg: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.to_string()) }
}

fn single_local_engine(
    host_alias: &str,
    marker: &str,
    sink: Arc<MemoryCaptureSink>,
    tpl_registry: TemplateRegistry,
) -> Arc<ExecutionEngine> {
    let mut groups = HashMap::new();
    groups.insert("target_group".to_string(), vec![host_alias.to_string()]);
    let mut host_vars = HashMap::new();
    let mut hv = HashMap::new();
    hv.insert("marker".to_string(), Value::String(marker.to_string()));
    host_vars.insert(host_alias.to_string(), hv);
    let flat = FlatInventory { groups, host_vars };

    let mut target_configs = HashMap::new();
    target_configs.insert(
        host_alias.to_string(),
        TargetConfig {
            host: "127.0.0.1".to_string(),
            connector_type: Some(ConnectorType::Local),
            ssh: None,
            privilege: None,
        },
    );

    let mut engine = ExecutionEngine::new(flat, target_configs, 4, None);
    engine.event_sink = Some(sink);
    engine.template_registry = tpl_registry;
    Arc::new(engine)
}

fn single_template_playbook(play_name: &str, task_name: &str, src: &Path, dest: &Path) -> Playbook {
    Playbook(vec![Play {
        name: Some(play_name.to_string()),
        hosts: "target_group".to_string(),
        serial: None,
        vars: None,
        roles: None,
        tasks: Some(vec![Task {
            name: Some(task_name.to_string()),
            template: Some(json!({
                "src": src.to_string_lossy(),
                "dest": dest.to_string_lossy(),
                "mode": "0600"
            })),
            ..Task::default()
        }]),
        r#become: None,
        become_method: None,
        become_user: None,
        ignore_errors: None,
    }])
}

fn verify_isolated_events(
    events: &[ExecutionEvent],
    expected_play: &str,
    expected_host: &str,
    forbidden_play: &str,
    forbidden_host: &str,
) -> Result<(), String> {
    ensure(!events.is_empty(), "event sink must record events")?;
    for ev in events {
        match ev {
            ExecutionEvent::PlayStarted { play_name, hosts } => {
                ensure(play_name == expected_play, "play_name cross-talk detected")?;
                ensure(
                    hosts == &vec![expected_host.to_string()],
                    "play hosts cross-talk detected",
                )?;
            }
            ExecutionEvent::TaskStarted { host, .. }
            | ExecutionEvent::TaskFinished { host, .. } => {
                ensure(host == expected_host, "task host cross-talk detected")?;
            }
            ExecutionEvent::LogLine { message, .. } => {
                ensure(
                    !message.contains(forbidden_play) && !message.contains(forbidden_host),
                    &format!("foreign log leaked into sink: {message}"),
                )?;
            }
            ExecutionEvent::TaskProgress { .. } | ExecutionEvent::ArtifactProgress { .. } => {}
        }
    }
    Ok(())
}

#[tokio::test]
async fn concurrent_sink_isolated() -> Result<(), String> {
    let sb = TempSandbox::new("concurrent")?;
    let tmpl_file = sb.path.join("entry.gtmpl");
    std::fs::write(&tmpl_file, r#"{{ include "shared-snippet.gtmpl" . }}"#)
        .map_err(|e| e.to_string())?;

    let sink_alpha = Arc::new(MemoryCaptureSink::new());
    let sink_beta = Arc::new(MemoryCaptureSink::new());

    let reg_alpha = TemplateRegistry::new();
    reg_alpha.register("shared-snippet.gtmpl", "ALPHA:{{ .marker }}")?;
    let reg_beta = TemplateRegistry::new();
    reg_beta.register("shared-snippet.gtmpl", "BETA:{{ .marker }}")?;

    let engine_alpha = single_local_engine(
        "node-alpha",
        "cluster-a",
        Arc::clone(&sink_alpha),
        reg_alpha,
    );
    let engine_beta =
        single_local_engine("node-beta", "cluster-b", Arc::clone(&sink_beta), reg_beta);

    let out_alpha = sb.path.join("alpha.out");
    let out_beta = sb.path.join("beta.out");
    let pb_alpha = single_template_playbook("Play Alpha", "Render Alpha", &tmpl_file, &out_alpha);
    let pb_beta = single_template_playbook("Play Beta", "Render Beta", &tmpl_file, &out_beta);

    let (res_a, res_b) = tokio::join!(
        engine_alpha.run_playbook(&pb_alpha),
        engine_beta.run_playbook(&pb_beta)
    );
    res_a?;
    res_b?;

    ensure(
        std::fs::read_to_string(&out_alpha).map_err(|e| e.to_string())? == "ALPHA:cluster-a",
        "unexpected alpha output",
    )?;
    ensure(
        std::fs::read_to_string(&out_beta).map_err(|e| e.to_string())? == "BETA:cluster-b",
        "unexpected beta output",
    )?;

    verify_isolated_events(
        &sink_alpha.snapshot(),
        "Play Alpha",
        "node-alpha",
        "Play Beta",
        "node-beta",
    )?;
    verify_isolated_events(
        &sink_beta.snapshot(),
        "Play Beta",
        "node-beta",
        "Play Alpha",
        "node-alpha",
    )
}

#[test]
fn workspace_bootstrap_idempotent() -> Result<(), String> {
    let sb = TempSandbox::new("workspace")?;
    let ws = Workspace::new(sb.path.join("work"));

    let created_first = ws.ensure_initialized().map_err(|e| e.to_string())?;
    ensure(
        created_first,
        "first ensure_initialized on empty dir must return true",
    )?;
    ensure(
        ws.inventory_path().is_file(),
        "configs/inventory.yaml must exist",
    )?;
    ensure(ws.images_path().is_file(), "configs/images.yaml must exist")?;
    ensure(
        ws.configs_dir().join("deploy.yaml").is_file(),
        "configs/deploy.yaml must exist",
    )?;
    ensure(
        ws.configs_dir()
            .join("templates/containerd-config.toml.gtmpl")
            .is_file(),
        "embedded templates must be extracted",
    )?;
    ensure(
        !ws.configs_dir().join("inventory.yaml.full").exists(),
        "local test inventory.yaml.full must never be embedded",
    )?;

    #[cfg(unix)]
    for dir in [ws.pki_dir(), ws.backups_dir(), ws.daemon_state_dir()] {
        let mode = std::fs::metadata(&dir)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o777;
        ensure(
            mode == 0o700,
            &format!("expected 0700 on {}, got {mode:04o}", dir.display()),
        )?;
    }

    let custom_marker = "# user-customized-inventory-marker\n";
    let mut inv_text = std::fs::read_to_string(ws.inventory_path()).map_err(|e| e.to_string())?;
    inv_text.push_str(custom_marker);
    std::fs::write(ws.inventory_path(), &inv_text).map_err(|e| e.to_string())?;

    let created_second = ws.ensure_initialized().map_err(|e| e.to_string())?;
    ensure(
        !created_second,
        "second ensure_initialized must return false",
    )?;
    let after_text = std::fs::read_to_string(ws.inventory_path()).map_err(|e| e.to_string())?;
    ensure(
        after_text.ends_with(custom_marker),
        "ensure_initialized must preserve existing user modifications",
    )?;

    Ok(())
}
