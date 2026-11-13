//! Integration tests for `YAML` playbook compilation, `gtmpl` condition evaluation,
//! role variable precedence, loop registration, `block`/`rescue`/`always` recovery,
//! `serial` batching with `run_once`, group validation, and dynamic variable injection.

use rtkk::connector::{ConnectorType, TargetConfig};
use rtkk::inventory::FlatInventory;
use rtkk::playbook::{
    ExecutionEngine, Play, Playbook, compile_playbook, eval_condition,
    inject_dynamic_inventory_vars, inject_runtime_vars,
};
use rtkk::template::render_template_string;
use rtkk::utils::base64_encode;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
        let path = std::env::temp_dir().join(format!("rtkk-pb-test-{prefix}-{pid}-{nanos}"));
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

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, content).map_err(|e| e.to_string())
}

fn local_target_config() -> TargetConfig {
    TargetConfig {
        host: "127.0.0.1".to_string(),
        connector_type: Some(ConnectorType::Local),
        ssh: None,
        privilege: None,
    }
}

fn build_local_engine(
    hosts: &[&str],
    groups: HashMap<String, Vec<String>>,
    extra_host_vars: &HashMap<String, HashMap<String, Value>>,
    inventory_dir: &Path,
) -> Arc<ExecutionEngine> {
    let mut host_vars: HashMap<String, HashMap<String, Value>> = HashMap::new();
    let mut target_configs: HashMap<String, TargetConfig> = HashMap::new();
    for &h in hosts {
        let mut vars = extra_host_vars.get(h).cloned().unwrap_or_default();
        vars.entry("inventory_hostname".to_string())
            .or_insert_with(|| Value::String(h.to_string()));
        vars.entry("ip".to_string())
            .or_insert_with(|| Value::String("127.0.0.1".to_string()));
        host_vars.insert(h.to_string(), vars);
        target_configs.insert(h.to_string(), local_target_config());
    }
    let flat = FlatInventory { groups, host_vars };
    Arc::new(ExecutionEngine::new(
        flat,
        target_configs,
        4,
        Some(inventory_dir.to_path_buf()),
    ))
}

#[test]
fn eval_condition_handles_eq_ne_bool_and_rejects_broken_syntax() -> Result<(), String> {
    let vars = json!({
        "env": "prod",
        "count": 3,
        "enabled": true,
        "disabled": false,
    });
    ensure(
        eval_condition(&json!(true), &vars)?,
        "Bool(true) must evaluate to true",
    )?;
    ensure(
        !eval_condition(&json!(false), &vars)?,
        "Bool(false) must evaluate to false",
    )?;
    ensure(
        eval_condition(&json!("{{ eq .env \"prod\" }}"), &vars)?,
        "eq matching string must evaluate to true",
    )?;
    ensure(
        !eval_condition(&json!("{{ eq .env \"staging\" }}"), &vars)?,
        "eq non-matching string must evaluate to false",
    )?;
    ensure(
        eval_condition(&json!("{{ ne .count 0 }}"), &vars)?,
        "ne non-zero count must evaluate to true",
    )?;
    ensure(
        eval_condition(&json!(["{{ eq .env \"prod\" }}", true, "yes"]), &vars)?,
        "array of truthy conditions must evaluate to true",
    )?;
    ensure(
        !eval_condition(&json!(["{{ eq .env \"prod\" }}", "{{ .disabled }}"]), &vars)?,
        "array containing a false condition must short-circuit to false",
    )?;
    ensure(
        eval_condition(&json!("{{ if .env"), &vars).is_err(),
        "unclosed template expression must return Err, not Ok(false)",
    )?;
    ensure(
        eval_condition(&json!("{{ nonexistent_fn .env }}"), &vars).is_err(),
        "unknown template function must return Err, not Ok(false)",
    )?;
    ensure(
        eval_condition(&json!("not_a_bool_literal"), &vars).is_err(),
        "non-boolean rendered output must return Err, not Ok(false)",
    )?;
    ensure(
        eval_condition(&json!(123), &vars).is_err(),
        "numeric condition type must return Err",
    )?;
    Ok(())
}

fn write_role_fixture(root: &Path, trace_file: &Path, out_file: &Path) -> Result<PathBuf, String> {
    let role_dir = root.join("roles").join("demo_role");
    write_file(
        &role_dir.join("defaults").join("main.yaml"),
        "only_default: from_default\ninv_wins: from_default\nparam_wins: from_default\n",
    )?;
    write_file(
        &role_dir.join("templates").join("role_sample.gtmpl"),
        "rendered={{ .param_wins }}",
    )?;
    let trace = trace_file.display();
    let out = out_file.display();
    let role_tasks = format!(
        r#"- name: Role direct task
  shell: printf 'role_direct\n' >> '{trace}'
- name: Role nested block verifying precedence
  block:
    - name: Capture precedence inside nested block
      copy:
        content: "{{{{ .only_default }}}}|{{{{ .inv_wins }}}}|{{{{ .param_wins }}}}"
        dest: "{out}"
    - name: Trace nested block order
      shell: printf 'role_nested_block\n' >> '{trace}'
"#
    );
    write_file(&role_dir.join("tasks").join("main.yaml"), &role_tasks)?;

    let pb_path = root.join("role_order.yaml");
    let playbook_yaml = format!(
        r"- name: Verify role order and precedence
  hosts: localhost
  vars:
    param_wins: from_play_vars
  roles:
    - role: demo_role
      param_wins: from_role_params
  tasks:
    - name: Play task after role
      shell: printf 'play_task\n' >> '{trace}'
"
    );
    write_file(&pb_path, &playbook_yaml)?;
    Ok(pb_path)
}

#[tokio::test]
async fn compile_and_run_roles_order_and_nested_block_precedence() -> Result<(), String> {
    let dir = TestDir::new("roles")?;
    let trace_file = dir.path.join("trace.log");
    let out_file = dir.path.join("precedence.txt");
    let pb_path = write_role_fixture(&dir.path, &trace_file, &out_file)?;

    let playbook = compile_playbook(&pb_path)?;
    let tmpl_ctx = rtkk::template::json_to_gtmpl(&json!({ "param_wins": "ok_tmpl" }));
    let tmpl_out = render_template_string(r#"{{ include "role_sample.gtmpl" . }}"#, &tmpl_ctx)?;
    ensure(
        tmpl_out == "rendered=ok_tmpl",
        "compile_playbook must register role templates",
    )?;

    let mut host_map = HashMap::new();
    let mut lh_vars = HashMap::new();
    lh_vars.insert("inv_wins".to_string(), json!("from_inventory"));
    host_map.insert("localhost".to_string(), lh_vars);
    let engine = build_local_engine(&["localhost"], HashMap::new(), &host_map, &dir.path);
    engine.run_playbook(&playbook).await?;

    let trace = std::fs::read_to_string(&trace_file).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = trace.lines().collect();
    ensure(
        lines == ["role_direct", "role_nested_block", "play_task"],
        &format!("expected role tasks before play tasks, got {lines:?}"),
    )?;
    let precedence = std::fs::read_to_string(&out_file).map_err(|e| e.to_string())?;
    ensure(
        precedence == "from_default|from_inventory|from_role_params",
        &format!("unexpected variable precedence inside nested role block: {precedence}"),
    )?;
    Ok(())
}

#[tokio::test]
async fn loop_and_with_items_filter_when_cleanup_item_and_register() -> Result<(), String> {
    let dir = TestDir::new("loops")?;
    let pb_path = dir.path.join("loops.yaml");
    write_file(
        &pb_path,
        r#"- name: Loop semantics
  hosts: localhost
  tasks:
    - name: Loop with per-item when
      command: echo "val-{{ .item }}"
      loop:
        - alpha
        - skip_me
        - beta
      when: '{{ ne .item "skip_me" }}'
      register: loop_reg
    - name: Legacy with_items loop
      command: echo "wi-{{ .item }}"
      with_items:
        - first
        - second
      when: '{{ eq .item "second" }}'
      register: wi_reg
    - name: Check item variable is cleaned up
      command: echo "post={{ default "CLEAN" .item }}"
      register: post_check
"#,
    )?;
    let playbook = compile_playbook(&pb_path)?;
    let engine = build_local_engine(&["localhost"], HashMap::new(), &HashMap::new(), &dir.path);
    engine.run_playbook(&playbook).await?;

    let bus = engine.host_vars_bus.lock().await;
    let lh = bus.get("localhost").ok_or("missing localhost vars")?;
    ensure(
        lh.get("item").is_none(),
        ".item must be removed from host_vars after loop completes",
    )?;
    let post_stdout = lh
        .get("post_check")
        .and_then(|v| v.get("stdout"))
        .and_then(Value::as_str);
    ensure(
        post_stdout == Some("post=CLEAN"),
        "post-loop task must not observe stale .item",
    )?;
    verify_loop_register(lh)?;
    Ok(())
}

fn verify_loop_register(lh: &Value) -> Result<(), String> {
    let loop_reg = lh.get("loop_reg").ok_or("missing loop_reg")?;
    ensure(
        loop_reg.get("changed").and_then(Value::as_bool) == Some(true),
        "loop_reg.changed must be true when non-skipped items changed",
    )?;
    ensure(
        loop_reg.get("failed").and_then(Value::as_bool) == Some(false),
        "loop_reg.failed must be false",
    )?;
    let results = loop_reg
        .get("results")
        .and_then(Value::as_array)
        .ok_or("loop_reg.results must be an array")?;
    ensure(results.len() == 3, "loop_reg.results must contain 3 items")?;
    ensure(
        results
            .first()
            .and_then(|r| r.get("stdout"))
            .and_then(Value::as_str)
            == Some("val-alpha"),
        "first loop item stdout must be val-alpha",
    )?;
    ensure(
        results
            .get(1)
            .and_then(|r| r.get("skipped"))
            .and_then(Value::as_bool)
            == Some(true),
        "second loop item must be marked skipped=true",
    )?;
    ensure(
        results
            .get(2)
            .and_then(|r| r.get("stdout"))
            .and_then(Value::as_str)
            == Some("val-beta"),
        "third loop item stdout must be val-beta",
    )?;
    let wi_results = lh
        .get("wi_reg")
        .and_then(|r| r.get("results"))
        .and_then(Value::as_array)
        .ok_or("wi_reg.results must be an array")?;
    ensure(
        wi_results.len() == 2,
        "with_items results must contain 2 items",
    )?;
    Ok(())
}

#[tokio::test]
async fn block_rescue_always_and_ignore_errors_recovery() -> Result<(), String> {
    let dir = TestDir::new("block-rescue")?;
    let trace_path = dir.path.join("recovery.log");
    let trace = trace_path.display();
    let pb_path = dir.path.join("recovery.yaml");
    write_file(
        &pb_path,
        &format!(
            r"- name: Block rescue always verification
  hosts: localhost
  tasks:
    - name: Failing block with ignore_errors step first
      block:
        - name: Ignored failure inside block
          command: 'false'
          ignore_errors: true
        - name: Runs after ignored failure
          shell: printf 'after_ignore\n' >> '{trace}'
        - name: Real failure triggering rescue
          command: 'false'
        - name: Must not run after failure
          shell: printf 'unreachable_step\n' >> '{trace}'
      rescue:
        - name: Recovery task
          shell: printf 'in_rescue\n' >> '{trace}'
      always:
        - name: Cleanup after rescue
          shell: printf 'always_1\n' >> '{trace}'
    - name: Succeeding block skips rescue but runs always
      block:
        - name: Healthy block step
          shell: printf 'healthy_block\n' >> '{trace}'
      rescue:
        - name: Should not run
          shell: printf 'unexpected_rescue\n' >> '{trace}'
      always:
        - name: Cleanup after healthy block
          shell: printf 'always_2\n' >> '{trace}'
"
        ),
    )?;

    let playbook = compile_playbook(&pb_path)?;
    let engine = build_local_engine(&["localhost"], HashMap::new(), &HashMap::new(), &dir.path);
    engine.run_playbook(&playbook).await?;

    let content = std::fs::read_to_string(&trace_path).map_err(|e| e.to_string())?;
    let events: Vec<&str> = content.lines().collect();
    ensure(
        events
            == vec![
                "after_ignore",
                "in_rescue",
                "always_1",
                "healthy_block",
                "always_2",
            ],
        &format!("unexpected block/rescue/always trace: {events:?}"),
    )?;
    Ok(())
}

#[tokio::test]
async fn serial_batching_executes_run_once_only_once_across_entire_play() -> Result<(), String> {
    let dir = TestDir::new("serial-once")?;
    let once_log = dir.path.join("once.log");
    let pb_path = dir.path.join("serial.yaml");
    write_file(
        &pb_path,
        &format!(
            r#"- name: Rolling serial batches with run_once
  hosts: workers
  serial: 2
  tasks:
    - name: Generate cluster token once across all batches
      shell: printf 'once\n' >> '{}' && printf 'tok-99'
      run_once: true
      register: init_tok
    - name: Write token marker per host
      copy:
        content: "{{{{ .inventory_hostname }}}}={{{{ .global.init_tok.stdout }}}}"
        dest: "{}/{{{{ .inventory_hostname }}}}.marker"
"#,
            once_log.display(),
            dir.path.display()
        ),
    )?;

    let hosts = ["w1", "w2", "w3", "w4"];
    let mut groups = HashMap::new();
    groups.insert(
        "workers".to_string(),
        hosts.iter().map(|s| (*s).to_string()).collect(),
    );
    let playbook = compile_playbook(&pb_path)?;
    let engine = build_local_engine(&hosts, groups, &HashMap::new(), &dir.path);
    engine.run_playbook(&playbook).await?;

    let once_text = std::fs::read_to_string(&once_log).map_err(|e| e.to_string())?;
    let count = once_text.lines().count();
    ensure(
        count == 1,
        &format!("run_once task must execute exactly 1 time across 2 serial batches, got {count}"),
    )?;
    for h in hosts {
        let marker = std::fs::read_to_string(dir.path.join(format!("{h}.marker")))
            .map_err(|e| e.to_string())?;
        ensure(
            marker == format!("{h}=tok-99"),
            &format!("host {h} did not receive global run_once result: {marker}"),
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn empty_known_group_skips_cleanly_while_unknown_group_errors() -> Result<(), String> {
    let dir = TestDir::new("groups")?;
    let mut groups = HashMap::new();
    groups.insert("empty_group".to_string(), Vec::new());
    groups.insert("active_group".to_string(), vec!["n1".to_string()]);
    let engine = build_local_engine(&["n1"], groups, &HashMap::new(), &dir.path);

    let empty_play = Playbook(vec![Play {
        name: Some("Skip empty group".to_string()),
        hosts: "empty_group".to_string(),
        ..Play::default()
    }]);
    engine.run_playbook(&empty_play).await?;

    let typo_play = Playbook(vec![Play {
        name: Some("Typo group".to_string()),
        hosts: "nonexistent_typo".to_string(),
        ..Play::default()
    }]);
    let err_res = engine.run_playbook(&typo_play).await;
    let Err(err_msg) = err_res else {
        return Err("unknown group pattern must return Err".to_string());
    };
    ensure(
        err_msg.contains("nonexistent_typo"),
        &format!("error should mention unknown group name, got: {err_msg}"),
    )?;
    Ok(())
}

#[test]
fn dynamic_and_runtime_var_injection_is_deterministic_and_derives_auth_basic() -> Result<(), String>
{
    let mut groups = HashMap::new();
    groups.insert(
        "kube_control_plane".to_string(),
        vec!["cp-2".to_string(), "cp-1".to_string()],
    );
    groups.insert(
        "etcd".to_string(),
        vec!["etcd-b".to_string(), "etcd-a".to_string()],
    );
    let mut host_vars: HashMap<String, HashMap<String, Value>> = HashMap::new();
    for (name, ip) in [
        ("cp-1", "10.0.0.11"),
        ("cp-2", "10.0.0.12"),
        ("etcd-a", "10.0.0.21"),
        ("etcd-b", "10.0.0.22"),
    ] {
        let mut hv = HashMap::new();
        hv.insert("ip".to_string(), json!(ip));
        hv.insert(
            "registry".to_string(),
            json!({
                "url": "10.0.0.10/rtkk",
                "username": "robot",
                "password": " rotated-secret ",
                "auth_basic": "YWRtaW46Y2hhbmdlLW1l"
            }),
        );
        host_vars.insert(name.to_string(), hv);
    }

    let mut copy_vars = host_vars.clone();
    inject_dynamic_inventory_vars(&groups, &mut host_vars);
    inject_runtime_vars(&mut host_vars, "configs/inventory.yaml");
    inject_dynamic_inventory_vars(&groups, &mut copy_vars);
    inject_runtime_vars(&mut copy_vars, "configs/inventory.yaml");
    ensure(
        host_vars == copy_vars,
        "dynamic and runtime var injection must be deterministic",
    )?;

    let cp1 = host_vars.get("cp-1").ok_or("missing cp-1")?;
    ensure(
        cp1.get("first_control_plane").and_then(Value::as_str) == Some("cp-2"),
        "first_control_plane must preserve group declaration order",
    )?;
    ensure(
        cp1.get("etcd_initial_cluster").and_then(Value::as_str)
            == Some("etcd-a=https://10.0.0.21:2380,etcd-b=https://10.0.0.22:2380"),
        "etcd_initial_cluster must sort etcd members deterministically",
    )?;
    let expected_b64 = base64_encode(b"robot: rotated-secret ");
    let actual_b64 = cp1
        .get("registry")
        .and_then(|r| r.get("auth_basic"))
        .and_then(Value::as_str);
    ensure(
        actual_b64 == Some(expected_b64.as_str()),
        "placeholder auth_basic must be replaced with base64(username:password)",
    )?;
    Ok(())
}
