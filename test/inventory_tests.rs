//! Integration tests for hierarchical and flat `YAML` inventory resolution,
//! deterministic host/group ordering, `localhost` exclusion from `all`, and `null` children.

use rtkk::inventory::{Inventory, resolve_inventory};
use rtkk::utils::{
    base64_encode, clean_ansi_escapes, is_safe_module_name, parse_file_mode, wrap_condition_string,
};

fn ensure(cond: bool, msg: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.to_string()) }
}

#[test]
fn flat_inventory_excludes_localhost_from_all_and_sorts_deterministically() -> Result<(), String> {
    let yaml = r#"
vars:
  cluster_domain: "cluster.local"
hosts:
  node-c: 10.0.0.63
  node-a: 10.0.0.61
  node-b:
    ip: 10.0.0.62
    custom_label: "ssd"
roles:
  kube_control_plane:
    - node-c
    - node-a
  kube_node:
    - node-b
  k8s_cluster:
    - kube_control_plane
    - kube_node
"#;
    let inv: Inventory = serde_yml::from_str(yaml).map_err(|e| e.to_string())?;
    let flat = resolve_inventory(&inv)?;

    let all_hosts = flat
        .groups
        .get("all")
        .ok_or_else(|| "missing 'all' group".to_string())?;
    ensure(
        !all_hosts.contains(&"localhost".to_string()),
        "localhost must NOT be injected into groups['all']",
    )?;
    ensure(
        all_hosts
            == &vec![
                "node-a".to_string(),
                "node-b".to_string(),
                "node-c".to_string(),
            ],
        &format!("unexpected groups['all'] ordering: {all_hosts:?}"),
    )?;
    ensure(
        flat.host_vars.contains_key("localhost"),
        "localhost must still be present in host_vars for hosts: localhost / delegate_to: localhost",
    )?;

    for _ in 0..10 {
        let again = resolve_inventory(&inv)?;
        ensure(
            again.groups.get("k8s_cluster") == flat.groups.get("k8s_cluster"),
            "group resolution must be 100% deterministic across runs",
        )?;
    }
    Ok(())
}

#[test]
fn hierarchical_inventory_handles_null_children_and_top_level_groups() -> Result<(), String> {
    let yaml = r"
all:
  vars:
    env: prod
    shared: from_all
  children:
    kube_control_plane:
      hosts:
        cp1:
          ip: 10.0.0.11
      vars:
        shared: from_cp
    kube_node:
extra_top_group:
  hosts:
    worker9:
      ip: 10.0.0.99
";
    let inv: Inventory = serde_yml::from_str(yaml).map_err(|e| e.to_string())?;
    let flat = resolve_inventory(&inv)?;

    let empty_nodes = flat
        .groups
        .get("kube_node")
        .ok_or_else(|| "null child group kube_node must exist".to_string())?;
    ensure(empty_nodes.is_empty(), "kube_node must be empty")?;

    let all_hosts = flat
        .groups
        .get("all")
        .ok_or_else(|| "missing all group".to_string())?;
    ensure(
        all_hosts.contains(&"worker9".to_string()),
        "top-level group hosts must be merged into groups['all']",
    )?;

    let cp1_vars = flat
        .host_vars
        .get("cp1")
        .ok_or_else(|| "missing cp1 vars".to_string())?;
    ensure(
        cp1_vars.get("shared") == Some(&serde_json::json!("from_cp")),
        "child group vars must override all.vars",
    )?;
    let w9_vars = flat
        .host_vars
        .get("worker9")
        .ok_or_else(|| "missing worker9 vars".to_string())?;
    ensure(
        w9_vars.get("env") == Some(&serde_json::json!("prod")),
        "top-level group host must inherit all.vars",
    )?;
    Ok(())
}

#[test]
fn cycle_detection_rejects_circular_group_hierarchy() -> Result<(), String> {
    let yaml = r"
group_a:
  groups: [group_b]
group_b:
  groups: [group_a]
";
    let inv: Inventory = serde_yml::from_str(yaml).map_err(|e| e.to_string())?;
    ensure(
        resolve_inventory(&inv).is_err(),
        "circular group reference must return Err",
    )?;
    Ok(())
}

#[test]
fn utils_helpers_handle_modes_base64_and_module_names() -> Result<(), String> {
    ensure(
        base64_encode(b"admin:change-me") == "YWRtaW46Y2hhbmdlLW1l",
        "RFC 4648 base64 encoding mismatch",
    )?;
    ensure(
        parse_file_mode(Some(&serde_json::json!("0755")), 0o644)? == 0o755,
        "string 0755 must parse to 0o755",
    )?;
    ensure(
        parse_file_mode(Some(&serde_json::json!("0o600")), 0o644)? == 0o600,
        "string 0o600 must parse to 0o600",
    )?;
    ensure(
        parse_file_mode(Some(&serde_json::json!(755)), 0o644)? == 0o755,
        "YAML 1.2 integer 755 must parse to 0o755",
    )?;
    ensure(
        parse_file_mode(Some(&serde_json::json!("0999")), 0o644).is_err(),
        "invalid octal digits must fail",
    )?;
    ensure(is_safe_module_name("br_netfilter"), "br_netfilter is valid")?;
    ensure(!is_safe_module_name("-rf"), "leading dash must be rejected")?;
    ensure(!is_safe_module_name(".."), "dot-dot must be rejected")?;
    ensure(
        clean_ansi_escapes("\x1b[32mok\x1b[0m[RTKK_SUDO_PROMPT]") == "ok",
        "ANSI and sudo prompt marker must be stripped",
    )?;
    ensure(
        wrap_condition_string("eq .a .b") == "{{ eq .a .b }}",
        "bare condition must wrap in {{ }}",
    )?;
    ensure(
        wrap_condition_string("{{ .a }} == {{ .b }}") == "{{ .a }} == {{ .b }}",
        "multi-block template string must not be double-wrapped",
    )?;
    Ok(())
}
