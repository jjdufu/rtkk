//! End-to-end validation and rendering tests for repository `configs/*.yaml` playbooks,
//! `configs/inventory.yaml`, `configs/images.yaml`, and all six `configs/templates/*.gtmpl`
//! manifests across 3-master `HA`, single-node control-plane, and dedicated external `etcd` topologies.

use rtkk::inventory::{FlatInventory, Inventory, resolve_inventory};
use rtkk::playbook::{
    compile_playbook, eval_condition, inject_dynamic_inventory_vars, inject_runtime_vars,
};
use rtkk::template::{json_to_gtmpl, render_template_string};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const TEMPLATE_FILES: [&str; 6] = [
    "kubeadm-config.yaml.gtmpl",
    "containerd-config.toml.gtmpl",
    "etcd.service.gtmpl",
    "cilium-values.yaml.gtmpl",
    "local-volume.yaml.gtmpl",
    "nodelocaldns.yaml.gtmpl",
];

const YAML_MANIFEST_TEMPLATES: [&str; 4] = [
    "kubeadm-config.yaml.gtmpl",
    "cilium-values.yaml.gtmpl",
    "local-volume.yaml.gtmpl",
    "nodelocaldns.yaml.gtmpl",
];

fn ensure(cond: bool, msg: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.to_string()) }
}

fn configs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("configs")
}

fn read_config_file(rel: &str) -> Result<String, String> {
    let p = configs_dir().join(rel);
    std::fs::read_to_string(&p).map_err(|e| format!("failed to read {}: {e}", p.display()))
}

fn load_base_inventory() -> Result<Inventory, String> {
    let raw = read_config_file("inventory.yaml")?;
    serde_yml::from_str(&raw).map_err(|e| format!("failed to parse configs/inventory.yaml: {e}"))
}

fn finalize_flat_inventory(inv: &Inventory) -> Result<FlatInventory, String> {
    let mut flat = resolve_inventory(inv)?;
    inject_dynamic_inventory_vars(&flat.groups, &mut flat.host_vars);
    inject_runtime_vars(&mut flat.host_vars, "configs/inventory.yaml");
    Ok(flat)
}

fn host_context_json(flat: &FlatInventory, host: &str) -> Result<Value, String> {
    let vars = flat
        .host_vars
        .get(host)
        .ok_or_else(|| format!("missing host '{host}' in resolved inventory"))?;
    serde_json::to_value(vars).map_err(|e| e.to_string())
}

fn render_config_template(rel_path: &str, host_vars: &Value) -> Result<String, String> {
    let src = read_config_file(rel_path)?;
    let ctx = json_to_gtmpl(host_vars);
    let rendered = render_template_string(&src, &ctx)
        .map_err(|e| format!("failed to render {rel_path}: {e}"))?;
    ensure(
        !rendered.contains("<no value>"),
        &format!("{rel_path} rendered '<no value>' placeholder"),
    )?;
    ensure(
        !rendered.contains("{{"),
        &format!("{rel_path} left unrendered '{{{{' delimiter"),
    )?;
    Ok(rendered)
}

fn parse_multi_doc_yaml(label: &str, rendered: &str) -> Result<Vec<Value>, String> {
    let mut docs = Vec::new();
    let mut current = String::new();
    for line in rendered.lines() {
        if line.trim_end() == "---" {
            if !current.trim().is_empty() {
                let val: Value = serde_yml::from_str(&current)
                    .map_err(|e| format!("{label}: multi-document YAML syntax error: {e}"))?;
                if !val.is_null() {
                    docs.push(val);
                }
            }
            current.clear();
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    if !current.trim().is_empty() {
        let val: Value = serde_yml::from_str(&current)
            .map_err(|e| format!("{label}: multi-document YAML syntax error: {e}"))?;
        if !val.is_null() {
            docs.push(val);
        }
    }
    ensure(
        !docs.is_empty(),
        &format!("{label}: expected at least one non-null YAML document"),
    )?;
    Ok(docs)
}

fn render_and_validate_all_templates(
    flat: &FlatInventory,
    cp_host: &str,
    etcd_host: &str,
) -> Result<HashMap<String, String>, String> {
    let cp_vars = host_context_json(flat, cp_host)?;
    let etcd_vars = host_context_json(flat, etcd_host)?;

    let images_rendered = render_config_template("images.yaml", &cp_vars)?;
    let images_doc: Value = serde_yml::from_str(&images_rendered)
        .map_err(|e| format!("rendered images.yaml invalid YAML: {e}"))?;
    let items = images_doc
        .get("images")
        .and_then(Value::as_array)
        .ok_or("images.yaml must contain images sequence")?;
    ensure(
        items.len() >= 10,
        "images.yaml should define all core cluster images",
    )?;

    let mut rendered_map = HashMap::new();
    for tmpl in TEMPLATE_FILES {
        let vars = if tmpl == "etcd.service.gtmpl" {
            &etcd_vars
        } else {
            &cp_vars
        };
        let out = render_config_template(&format!("templates/{tmpl}"), vars)?;
        rendered_map.insert(tmpl.to_string(), out);
    }

    for yaml_tmpl in YAML_MANIFEST_TEMPLATES {
        let body = rendered_map
            .get(yaml_tmpl)
            .ok_or_else(|| format!("missing rendered {yaml_tmpl}"))?;
        parse_multi_doc_yaml(yaml_tmpl, body)?;
    }
    Ok(rendered_map)
}

fn find_cilium_operator_replicas(cilium_docs: &[Value]) -> Result<u64, String> {
    let doc = cilium_docs
        .first()
        .ok_or("cilium-values.yaml must contain a root YAML mapping")?;
    doc.get("operator")
        .and_then(|op| op.get("replicas"))
        .and_then(Value::as_u64)
        .ok_or_else(|| "cilium-values.yaml missing integer operator.replicas".into())
}

#[test]
fn all_real_playbooks_compile_cleanly() -> Result<(), String> {
    let base = configs_dir();
    for name in ["download.yaml", "cleanup.yaml", "deploy.yaml"] {
        let pb = compile_playbook(&base.join(name))
            .map_err(|e| format!("compile_playbook failed for configs/{name}: {e}"))?;
        ensure(
            !pb.0.is_empty(),
            &format!("configs/{name} compiled to zero plays"),
        )?;
    }
    Ok(())
}

#[test]
fn render_ha_3master_3worker_topology_and_verify_defaults() -> Result<(), String> {
    let inv = load_base_inventory()?;
    let flat = finalize_flat_inventory(&inv)?;
    let rendered = render_and_validate_all_templates(&flat, "c23-p61", "c23-p61")?;

    let containerd_toml = rendered
        .get("containerd-config.toml.gtmpl")
        .ok_or("missing containerd render")?;
    ensure(
        containerd_toml.contains("version = 3")
            && containerd_toml.contains("sandbox = \"10.0.0.10/rtkk/pause:3.10.2\"")
            && containerd_toml.contains("SystemdCgroup = true"),
        "containerd-config.toml must render v3 private pause image and SystemdCgroup = true",
    )?;

    let cilium_yaml = rendered
        .get("cilium-values.yaml.gtmpl")
        .ok_or("missing cilium-values render")?;
    let cilium_docs = parse_multi_doc_yaml("cilium-ha", cilium_yaml)?;
    let replicas = find_cilium_operator_replicas(&cilium_docs)?;
    ensure(
        replicas == 2,
        &format!("HA 6-node cluster should run 2 cilium-operator replicas, got {replicas}"),
    )?;
    ensure(
        cilium_yaml.contains("override: \"10.0.0.10/rtkk/cilium:v1.20.5\"")
            && cilium_yaml.contains("override: \"10.0.0.10/rtkk/operator-generic:v1.20.5\""),
        "cilium-values.yaml must point cilium and operator-generic overrides to private registry",
    )?;

    let kubeadm_yaml = rendered
        .get("kubeadm-config.yaml.gtmpl")
        .ok_or("missing kubeadm render")?;
    verify_kubeadm_validity_and_openebs_analytics(
        kubeadm_yaml,
        rendered
            .get("local-volume.yaml.gtmpl")
            .ok_or("missing local-volume render")?,
        &flat,
    )
}

fn verify_kubeadm_validity_and_openebs_analytics(
    default_kubeadm_yaml: &str,
    local_vol_yaml: &str,
    flat: &FlatInventory,
) -> Result<(), String> {
    let docs_default = parse_multi_doc_yaml("kubeadm-default", default_kubeadm_yaml)?;
    check_cluster_config_cert_validity(&docs_default, "867240h")?;
    let dns_tag = docs_default
        .iter()
        .find(|d| d.get("kind").and_then(Value::as_str) == Some("ClusterConfiguration"))
        .and_then(|d| d.get("dns"))
        .and_then(|dns| dns.get("imageTag"))
        .and_then(Value::as_str);
    ensure(
        dns_tag == Some("v1.14.6"),
        &format!("expected ClusterConfiguration.dns.imageTag == \"v1.14.6\", got {dns_tag:?}"),
    )?;

    let mut vars_unset = host_context_json(flat, "c23-p61")?;
    if let Some(obj) = vars_unset.as_object_mut() {
        obj.remove("cert_validity");
    }
    let rendered_fallback =
        render_config_template("templates/kubeadm-config.yaml.gtmpl", &vars_unset)?;
    let docs_fallback = parse_multi_doc_yaml("kubeadm-fallback", &rendered_fallback)?;
    check_cluster_config_cert_validity(&docs_fallback, "867240h")?;

    let mut vars_1y = host_context_json(flat, "c23-p61")?;
    if let Some(obj) = vars_1y.as_object_mut() {
        obj.insert("cert_validity".to_string(), json!("8760h"));
    }
    let rendered_1y = render_config_template("templates/kubeadm-config.yaml.gtmpl", &vars_1y)?;
    let docs_1y = parse_multi_doc_yaml("kubeadm-1y", &rendered_1y)?;
    check_cluster_config_cert_validity(&docs_1y, "8760h")?;

    let lv_docs = parse_multi_doc_yaml("local-volume", local_vol_yaml)?;
    verify_openebs_analytics_disabled(&lv_docs)
}

fn check_cluster_config_cert_validity(docs: &[Value], expected: &str) -> Result<(), String> {
    let cluster_cfg = docs
        .iter()
        .find(|d| d.get("kind").and_then(Value::as_str) == Some("ClusterConfiguration"))
        .ok_or("ClusterConfiguration missing in kubeadm-config.yaml")?;
    let cert_period = cluster_cfg
        .get("certificateValidityPeriod")
        .and_then(Value::as_str);
    let ca_period = cluster_cfg
        .get("caCertificateValidityPeriod")
        .and_then(Value::as_str);
    ensure(
        cert_period == Some(expected) && ca_period == Some(expected),
        &format!(
            "expected certificateValidityPeriod & caCertificateValidityPeriod == {expected}, got {cert_period:?} / {ca_period:?}"
        ),
    )
}

fn verify_openebs_analytics_disabled(lv_docs: &[Value]) -> Result<(), String> {
    let deploy = lv_docs
        .iter()
        .find(|d| {
            d.get("kind").and_then(Value::as_str) == Some("Deployment")
                && d.get("metadata")
                    .and_then(|m| m.get("name"))
                    .and_then(Value::as_str)
                    == Some("openebs-localpv-provisioner")
        })
        .ok_or("openebs-localpv-provisioner Deployment missing")?;
    let envs = deploy
        .get("spec")
        .and_then(|s| s.get("template"))
        .and_then(|t| t.get("spec"))
        .and_then(|s| s.get("containers"))
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .and_then(|c| c.get("env"))
        .and_then(Value::as_array)
        .ok_or("openebs container env list missing")?;
    let analytics = envs
        .iter()
        .find(|e| e.get("name").and_then(Value::as_str) == Some("OPENEBS_IO_ENABLE_ANALYTICS"))
        .and_then(|e| e.get("value"))
        .and_then(Value::as_str);
    ensure(
        analytics == Some("false"),
        &format!("OPENEBS_IO_ENABLE_ANALYTICS must be \"false\", got {analytics:?}"),
    )
}

#[test]
fn render_single_node_control_plane_only_topology_sets_cilium_operator_replicas_1()
-> Result<(), String> {
    let mut inv = load_base_inventory()?;
    let mut single_hosts = HashMap::new();
    single_hosts.insert("c23-p61".to_string(), json!("10.0.0.61"));
    inv.hosts = Some(single_hosts);

    let mut roles = HashMap::new();
    roles.insert(
        "kube_control_plane".to_string(),
        vec!["c23-p61".to_string()],
    );
    roles.insert("kube_node".to_string(), Vec::new());
    roles.insert("etcd".to_string(), vec!["c23-p61".to_string()]);
    roles.insert(
        "k8s_cluster".to_string(),
        vec!["kube_control_plane".to_string(), "kube_node".to_string()],
    );
    inv.roles = Some(roles);

    let flat = finalize_flat_inventory(&inv)?;
    let rendered = render_and_validate_all_templates(&flat, "c23-p61", "c23-p61")?;
    let cilium_yaml = rendered
        .get("cilium-values.yaml.gtmpl")
        .ok_or("missing single-node cilium-values render")?;
    let cilium_docs = parse_multi_doc_yaml("cilium-single-node", cilium_yaml)?;
    let replicas = find_cilium_operator_replicas(&cilium_docs)?;
    ensure(
        replicas == 1,
        &format!(
            "single-node control-plane-only topology (kube_node: []) must set cilium-operator replicas: 1, got {replicas}"
        ),
    )?;
    Ok(())
}

#[test]
fn render_cilium_with_dynamic_pki_registers_secrets_without_static_keys() -> Result<(), String> {
    let inv = load_base_inventory()?;
    let flat = finalize_flat_inventory(&inv)?;
    let base_vars = host_context_json(&flat, "c23-p61")?;

    let fallback_yaml = render_config_template("templates/cilium-values.yaml.gtmpl", &base_vars)?;
    let fallback_docs = parse_multi_doc_yaml("cilium-auto-tls", &fallback_yaml)?;
    let auto_enabled = fallback_docs
        .first()
        .and_then(|d| d.get("hubble"))
        .and_then(|h| h.get("tls"))
        .and_then(|t| t.get("auto"))
        .and_then(|a| a.get("enabled"))
        .and_then(Value::as_bool);
    ensure(
        auto_enabled == Some(true),
        "cilium-values.yaml without pre-generated PKI should enable hubble.tls.auto",
    )?;

    let mut vars = base_vars;
    if let Some(obj) = vars.as_object_mut() {
        obj.insert("cilium_ca_crt".to_string(), json!({ "stdout": "Y2EtY3J0" }));
        obj.insert("cilium_ca_key".to_string(), json!({ "stdout": "Y2Eta2V5" }));
        obj.insert(
            "hubble_tls_crt".to_string(),
            json!({ "stdout": "dGxzLWNydA==" }),
        );
        obj.insert(
            "hubble_tls_key".to_string(),
            json!({ "stdout": "dGxzLWtleQ==" }),
        );
    }
    let cilium_yaml = render_config_template("templates/cilium-values.yaml.gtmpl", &vars)?;
    let docs = parse_multi_doc_yaml("cilium-dynamic-pki", &cilium_yaml)?;
    let root = docs.first().ok_or("missing cilium-values root doc")?;
    let auto_disabled = root
        .get("hubble")
        .and_then(|h| h.get("tls"))
        .and_then(|t| t.get("auto"))
        .and_then(|a| a.get("enabled"))
        .and_then(Value::as_bool);
    let ca_crt = root
        .get("tls")
        .and_then(|t| t.get("ca"))
        .and_then(|ca| ca.get("cert"))
        .and_then(Value::as_str);
    let srv_crt = root
        .get("hubble")
        .and_then(|h| h.get("tls"))
        .and_then(|t| t.get("server"))
        .and_then(|s| s.get("cert"))
        .and_then(Value::as_str);
    ensure(
        auto_disabled == Some(false)
            && ca_crt == Some("Y2EtY3J0")
            && srv_crt == Some("dGxzLWNydA=="),
        "dynamic PKI variables must populate tls.ca and hubble.tls.server with auto.enabled=false",
    )
}

#[test]
fn render_external_dedicated_etcd_topology_points_endpoints_to_dedicated_etcd_ips()
-> Result<(), String> {
    let mut inv = load_base_inventory()?;
    let hosts_map = inv.hosts.get_or_insert_with(HashMap::new);
    hosts_map.insert("etcd-1".to_string(), json!("10.0.0.81"));
    hosts_map.insert("etcd-2".to_string(), json!("10.0.0.82"));
    hosts_map.insert("etcd-3".to_string(), json!("10.0.0.83"));
    let roles_map = inv.roles.get_or_insert_with(HashMap::new);
    roles_map.insert(
        "etcd".to_string(),
        vec![
            "etcd-1".to_string(),
            "etcd-2".to_string(),
            "etcd-3".to_string(),
        ],
    );

    let flat = finalize_flat_inventory(&inv)?;
    let rendered = render_and_validate_all_templates(&flat, "c23-p61", "etcd-1")?;

    let kubeadm_yaml = rendered
        .get("kubeadm-config.yaml.gtmpl")
        .ok_or("missing kubeadm render")?;
    let docs = parse_multi_doc_yaml("kubeadm-ext-etcd", kubeadm_yaml)?;
    let cluster_cfg = docs
        .iter()
        .find(|d| d.get("kind").and_then(Value::as_str) == Some("ClusterConfiguration"))
        .ok_or("ClusterConfiguration missing")?;
    let endpoints: Vec<&str> = cluster_cfg
        .get("etcd")
        .and_then(|e| e.get("external"))
        .and_then(|ex| ex.get("endpoints"))
        .and_then(Value::as_array)
        .ok_or("etcd.external.endpoints missing")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    ensure(
        endpoints
            == vec![
                "https://10.0.0.81:2379",
                "https://10.0.0.82:2379",
                "https://10.0.0.83:2379",
            ],
        &format!("expected dedicated etcd IPs in kubeadm-config, got {endpoints:?}"),
    )?;

    let etcd_unit = rendered
        .get("etcd.service.gtmpl")
        .ok_or("missing etcd.service render")?;
    ensure(
        etcd_unit.contains(
            "--initial-cluster=etcd-1=https://10.0.0.81:2380,etcd-2=https://10.0.0.82:2380,etcd-3=https://10.0.0.83:2380"
        ),
        "etcd.service must target dedicated etcd peer endpoints",
    )?;
    Ok(())
}

fn render_containerd_for_version(
    base_vars: &Value,
    version: Option<&str>,
) -> Result<String, String> {
    let mut vars = base_vars.clone();
    if let Some(obj) = vars.as_object_mut() {
        if let Some(v) = version {
            obj.insert("containerd_version".to_string(), json!(v));
        } else {
            obj.remove("containerd_version");
        }
    }
    render_config_template("templates/containerd-config.toml.gtmpl", &vars)
}

const EXPECTED_CONTAINERD_V2: &str = "\
version = 2
[plugins]
  [plugins.\"io.containerd.grpc.v1.cri\"]
    sandbox_image = \"10.0.0.10/rtkk/pause:3.10.2\"
    [plugins.\"io.containerd.grpc.v1.cri\".cni]
      bin_dir = \"/opt/cni/bin\"
      conf_dir = \"/etc/cni/net.d\"
    [plugins.\"io.containerd.grpc.v1.cri\".containerd]
      snapshotter = \"overlayfs\"
      default_runtime_name = \"runc\"
      [plugins.\"io.containerd.grpc.v1.cri\".containerd.runtimes]
        [plugins.\"io.containerd.grpc.v1.cri\".containerd.runtimes.runc]
          runtime_type = \"io.containerd.runc.v2\"
          [plugins.\"io.containerd.grpc.v1.cri\".containerd.runtimes.runc.options]
            SystemdCgroup = true
    [plugins.\"io.containerd.grpc.v1.cri\".registry]
      config_path = \"/etc/containerd/certs.d\"
";

const EXPECTED_CONTAINERD_V3_2_0: &str = "\
version = 3
[plugins]
  [plugins.\"io.containerd.cri.v1.images\"]
    snapshotter = \"overlayfs\"
    [plugins.\"io.containerd.cri.v1.images\".pinned_images]
      sandbox = \"10.0.0.10/rtkk/pause:3.10.2\"
    [plugins.\"io.containerd.cri.v1.images\".registry]
      config_path = \"/etc/containerd/certs.d\"
  [plugins.\"io.containerd.cri.v1.runtime\"]
    [plugins.\"io.containerd.cri.v1.runtime\".cni]
      bin_dir = \"/opt/cni/bin\"
      conf_dir = \"/etc/cni/net.d\"
    [plugins.\"io.containerd.cri.v1.runtime\".containerd]
      default_runtime_name = \"runc\"
      [plugins.\"io.containerd.cri.v1.runtime\".containerd.runtimes]
        [plugins.\"io.containerd.cri.v1.runtime\".containerd.runtimes.runc]
          runtime_type = \"io.containerd.runc.v2\"
          [plugins.\"io.containerd.cri.v1.runtime\".containerd.runtimes.runc.options]
            SystemdCgroup = true
  [plugins.\"io.containerd.transfer.v1.local\"]
    config_path = \"/etc/containerd/certs.d\"
";

const EXPECTED_CONTAINERD_V3_2_1_PLUS: &str = "\
version = 3
[plugins]
  [plugins.\"io.containerd.cri.v1.images\"]
    snapshotter = \"overlayfs\"
    [plugins.\"io.containerd.cri.v1.images\".pinned_images]
      sandbox = \"10.0.0.10/rtkk/pause:3.10.2\"
    [plugins.\"io.containerd.cri.v1.images\".registry]
      config_path = \"/etc/containerd/certs.d\"
  [plugins.\"io.containerd.cri.v1.runtime\"]
    [plugins.\"io.containerd.cri.v1.runtime\".cni]
      bin_dirs = [\"/opt/cni/bin\"]
      conf_dir = \"/etc/cni/net.d\"
    [plugins.\"io.containerd.cri.v1.runtime\".containerd]
      default_runtime_name = \"runc\"
      [plugins.\"io.containerd.cri.v1.runtime\".containerd.runtimes]
        [plugins.\"io.containerd.cri.v1.runtime\".containerd.runtimes.runc]
          runtime_type = \"io.containerd.runc.v2\"
          [plugins.\"io.containerd.cri.v1.runtime\".containerd.runtimes.runc.options]
            SystemdCgroup = true
  [plugins.\"io.containerd.transfer.v1.local\"]
    config_path = \"/etc/containerd/certs.d\"
";

#[test]
fn render_containerd_config_auto_switches_between_v2_and_v3_across_versions() -> Result<(), String>
{
    let inv = load_base_inventory()?;
    let flat = finalize_flat_inventory(&inv)?;
    let base_vars = host_context_json(&flat, "c23-p61")?;

    for v1 in ["1.7.32", "v1.7.32", "1.7.32-rc.1"] {
        let out = render_containerd_for_version(&base_vars, Some(v1))?;
        ensure(
            out == EXPECTED_CONTAINERD_V2,
            &format!("unexpected Containerd 1.x TOML for {v1}:\n{out}"),
        )?;
    }

    for v2_0 in ["2.0.0", "2.0.5", "v2.0.5", "2.0.0-rc.3"] {
        let out = render_containerd_for_version(&base_vars, Some(v2_0))?;
        ensure(
            out == EXPECTED_CONTAINERD_V3_2_0,
            &format!("unexpected Containerd 2.0.x TOML for {v2_0}:\n{out}"),
        )?;
    }

    for v2_1_plus in [
        None,
        Some("2.1.0"),
        Some("2.2.0"),
        Some("v2.2.0"),
        Some("2.4.2"),
        Some("v2.4.2-rc.1"),
    ] {
        let out = render_containerd_for_version(&base_vars, v2_1_plus)?;
        ensure(
            out == EXPECTED_CONTAINERD_V3_2_1_PLUS,
            &format!("unexpected Containerd 2.1+ TOML for {v2_1_plus:?}:\n{out}"),
        )?;
    }
    Ok(())
}

#[test]
fn download_playbook_normalizes_containerd_version_with_or_without_v_prefix() -> Result<(), String>
{
    let inv = load_base_inventory()?;
    let flat = finalize_flat_inventory(&inv)?;
    let base_vars = host_context_json(&flat, "c23-p61")?;

    for (raw_ver, clean_ver) in [
        ("1.7.32", "1.7.32"),
        ("v1.7.32", "1.7.32"),
        ("2.4.2", "2.4.2"),
        ("v2.4.2", "2.4.2"),
        ("V2.4.2", "2.4.2"),
    ] {
        let mut vars = base_vars.clone();
        if let Some(obj) = vars.as_object_mut() {
            obj.insert("containerd_version".to_string(), json!(raw_ver));
        }
        let rendered = render_config_template("download.yaml", &vars)?;
        let expected_url = format!(
            "https://github.com/containerd/containerd/releases/download/v{clean_ver}/containerd-{clean_ver}-linux-amd64.tar.gz"
        );
        ensure(
            rendered.contains(&expected_url),
            &format!("download.yaml for '{raw_ver}' did not contain '{expected_url}':\n{rendered}"),
        )?;
        ensure(
            !rendered.contains("/vv"),
            &format!("download.yaml for '{raw_ver}' rendered duplicate 'vv' tag prefix"),
        )?;
    }
    Ok(())
}

#[test]
fn images_yaml_supports_optional_cni_addons_and_extra_images() -> Result<(), String> {
    let inv = load_base_inventory()?;
    let flat = finalize_flat_inventory(&inv)?;
    let base_vars = host_context_json(&flat, "c23-p61")?;

    let mut minimal_vars = base_vars.clone();
    if let Some(obj) = minimal_vars.as_object_mut() {
        obj.insert("cni_plugin".to_string(), json!("none"));
        obj.insert(
            "addons".to_string(),
            json!({ "nodelocaldns": false, "openebs_localpv": false }),
        );
    }
    let minimal_rendered = render_config_template("images.yaml", &minimal_vars)?;
    let minimal_doc: Value = serde_yml::from_str(&minimal_rendered).map_err(|e| e.to_string())?;
    let minimal_items = minimal_doc
        .get("images")
        .and_then(Value::as_array)
        .ok_or("missing images array")?;
    ensure(
        minimal_items.len() == 7,
        &format!(
            "expected 7 core images when CNI & addons disabled, got {}",
            minimal_items.len()
        ),
    )?;
    ensure(
        !minimal_rendered.contains("cilium")
            && !minimal_rendered.contains("provisioner-localpv")
            && !minimal_rendered.contains("k8s-dns-node-cache"),
        "disabled CNI and addons must be omitted from rendered images.yaml",
    )?;

    let mut custom_vars = base_vars;
    if let Some(obj) = custom_vars.as_object_mut() {
        obj.insert("cni_plugin".to_string(), json!("custom"));
        obj.insert(
            "extra_images".to_string(),
            json!([{
                "name": "calico-node",
                "source": "quay.io/calico/node:v3.30.0",
                "target": "10.0.0.10/rtkk/calico-node:v3.30.0"
            }]),
        );
    }
    let custom_rendered = render_config_template("images.yaml", &custom_vars)?;
    ensure(
        !custom_rendered.contains("quay.io/cilium/cilium")
            && custom_rendered.contains("quay.io/calico/node:v3.30.0"),
        "custom CNI mode with extra_images must replace Cilium with custom image entries",
    )
}

#[test]
fn deploy_playbook_compiles_pluggable_addon_roles_and_evaluates_conditions() -> Result<(), String> {
    let pb = compile_playbook(&configs_dir().join("deploy.yaml"))?;
    let addons_play =
        pb.0.iter()
            .find(|p| {
                p.name
                    .as_deref()
                    .is_some_and(|n| n.contains("Pluggable CNI"))
            })
            .ok_or("missing Pluggable CNI play in deploy.yaml")?;
    let role_blocks = addons_play
        .tasks
        .as_ref()
        .ok_or("Pluggable CNI play missing compiled role tasks")?;
    ensure(
        role_blocks.len() == 4,
        &format!(
            "expected 4 compiled addon role blocks, got {}",
            role_blocks.len()
        ),
    )?;

    let inv = load_base_inventory()?;
    let flat = finalize_flat_inventory(&inv)?;
    let cp1 = host_context_json(&flat, "c23-p61")?;
    let cp2 = host_context_json(&flat, "c23-p62")?;

    let cilium_when = role_blocks
        .first()
        .and_then(|t| t.when.as_ref())
        .ok_or("missing cni-cilium role condition")?;
    let openebs_when = role_blocks
        .get(1)
        .and_then(|t| t.when.as_ref())
        .ok_or("missing addon-openebs role condition")?;
    let custom_when = role_blocks
        .get(3)
        .and_then(|t| t.when.as_ref())
        .ok_or("missing custom-manifests role condition")?;

    ensure(
        eval_condition(cilium_when, &cp1)? && !eval_condition(cilium_when, &cp2)?,
        "cni-cilium must run on first_control_plane only by default",
    )?;
    ensure(
        eval_condition(openebs_when, &cp1)? && !eval_condition(custom_when, &cp1)?,
        "default inventory must enable openebs and skip empty custom_manifests",
    )?;

    let mut bare_cp1 = cp1;
    if let Some(obj) = bare_cp1.as_object_mut() {
        obj.insert("cni_plugin".to_string(), json!("none"));
        obj.insert("addons".to_string(), json!({ "openebs_localpv": false }));
        obj.insert(
            "custom_manifests".to_string(),
            json!(["templates/local-volume.yaml.gtmpl"]),
        );
    }
    ensure(
        !eval_condition(cilium_when, &bare_cp1)?
            && !eval_condition(openebs_when, &bare_cp1)?
            && eval_condition(custom_when, &bare_cp1)?,
        "toggling cni_plugin=none, openebs_localpv=false, and custom_manifests must flip role conditions",
    )
}

#[test]
fn deploy_hosts_sync_task_preserves_existing_prep_hosts_block() -> Result<(), String> {
    let pb = compile_playbook(&configs_dir().join("deploy.yaml"))?;
    let play1 =
        pb.0.iter()
            .find(|p| p.name.as_deref().is_some_and(|n| n.contains("Play 1")))
            .ok_or("missing Play 1 in deploy.yaml")?;
    let hosts_task = play1
        .tasks
        .as_ref()
        .and_then(|tasks| {
            tasks
                .iter()
                .find(|t| t.name.as_deref().is_some_and(|n| n.contains("/etc/hosts")))
        })
        .ok_or("missing /etc/hosts task in Play 1")?;
    let raw_script = hosts_task
        .shell
        .as_ref()
        .and_then(Value::as_str)
        .ok_or("expected shell string on /etc/hosts task")?;

    let inv = load_base_inventory()?;
    let flat = finalize_flat_inventory(&inv)?;
    let mut cp1 = host_context_json(&flat, "c23-p61")?;
    let mut hostvars_map = serde_json::Map::new();
    for (h, hv) in &flat.host_vars {
        let val = serde_json::to_value(hv).map_err(|e| e.to_string())?;
        hostvars_map.insert(h.clone(), val);
    }
    if let Some(obj) = cp1.as_object_mut() {
        obj.insert("hostvars".to_string(), Value::Object(hostvars_map));
    }

    let rendered = render_template_string(raw_script, &json_to_gtmpl(&cp1))?;
    let tmp_hosts = std::env::temp_dir().join(format!("rtkk_hosts_guard_{}", std::process::id()));
    let sandbox_script = rendered
        .replace("/etc/hosts", &tmp_hosts.to_string_lossy())
        .replace(
            "/tmp/.rtkk_deploy_hosts",
            &format!("{}.tmp", tmp_hosts.display()),
        );

    let prepared = "127.0.0.1 localhost\n# RTKK HOSTS START\n10.0.0.61 c23-p61 os-host-p61\n# RTKK HOSTS END\n";
    std::fs::write(&tmp_hosts, prepared).map_err(|e| e.to_string())?;
    let st1 = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&sandbox_script)
        .status()
        .map_err(|e| e.to_string())?;
    let after_prepared = std::fs::read_to_string(&tmp_hosts).map_err(|e| e.to_string())?;
    ensure(
        st1.success() && after_prepared.contains("os-host-p61"),
        "deploy.yaml must preserve OS hostname entries written by `rtkk prep hosts`",
    )?;

    std::fs::write(&tmp_hosts, "127.0.0.1 localhost\n").map_err(|e| e.to_string())?;
    let st2 = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(&sandbox_script)
        .status()
        .map_err(|e| e.to_string())?;
    let after_bare = std::fs::read_to_string(&tmp_hosts).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp_hosts);
    ensure(
        st2.success() && after_bare.contains("10.0.0.61 c23-p61"),
        "deploy.yaml must populate fallback /etc/hosts block when prep hosts has not run",
    )
}
