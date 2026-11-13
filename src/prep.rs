//! Preparation utilities for synchronizing `/etc/hosts` mappings across inventory nodes.

use crate::connector::{TargetConfig, create_connector, unique_tmp_id};
use crate::inventory::FlatInventory;
use crate::utils::{sed_replacement_escape, shell_single_quote};
use serde_json::Value;
use std::collections::HashMap;
use std::hash::BuildHasher;

fn validate_hosts_field(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{label} cannot be empty"));
    }
    if value.contains(['\n', '\r', '\0']) || value.chars().any(char::is_whitespace) {
        return Err(format!(
            "invalid {label} '{value}': must not contain newlines, control characters, or whitespace"
        ));
    }
    Ok(())
}

fn escape_unescaped_slash(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut trailing_backslashes: usize = 0;
    for ch in input.chars() {
        if ch == '/' && trailing_backslashes.is_multiple_of(2) {
            out.push('\\');
        }
        out.push(ch);
        if ch == '\\' {
            trailing_backslashes = trailing_backslashes.saturating_add(1);
        } else {
            trailing_backslashes = 0;
        }
    }
    out
}

fn sorted_remote_hosts(flat_inventory: &FlatInventory) -> Vec<&String> {
    let mut hosts: Vec<&String> = flat_inventory
        .host_vars
        .keys()
        .filter(|h| h.as_str() != "localhost")
        .collect();
    hosts.sort();
    hosts
}

async fn resolve_single_os_hostname(
    host: String,
    tc: TargetConfig,
) -> Result<(String, String), String> {
    crate::logging::start_task(&host, "Resolve OS hostname");
    let conn = match create_connector(&tc).await {
        Ok(c) => c,
        Err(e) => {
            let msg = e.to_string();
            crate::logging::finish_task_failed(&host, "Resolve OS hostname", &msg);
            return Err(msg);
        }
    };
    let step_res = async {
        let res = conn
            .execute_shell("hostname")
            .await
            .map_err(|e| e.to_string())?;
        if res.exit_code != 0 {
            let stderr_str = String::from_utf8_lossy(&res.stderr);
            return Err(format!("failed to get hostname on {host}: {stderr_str}"));
        }
        let os_hostname = String::from_utf8_lossy(&res.stdout).trim().to_string();
        if !os_hostname.is_empty() {
            validate_hosts_field("OS hostname", &os_hostname)?;
        }
        Ok(os_hostname)
    }
    .await;
    let _ = conn.close().await;
    match &step_res {
        Ok(_) => crate::logging::finish_task_success(&host, "Resolve OS hostname"),
        Err(e) => crate::logging::finish_task_failed(&host, "Resolve OS hostname", e),
    }
    step_res.map(|os_hostname| (host, os_hostname))
}

async fn get_hosts_os_hostnames<S: BuildHasher + Sync>(
    flat_inventory: &FlatInventory,
    target_configs: &HashMap<String, TargetConfig, S>,
    best_effort: bool,
) -> Result<HashMap<String, String>, String> {
    let mut hostname_handles = Vec::new();
    for host in sorted_remote_hosts(flat_inventory) {
        validate_hosts_field("inventory hostname", host)?;
        let host_clone = host.clone();
        let tc = target_configs
            .get(host)
            .cloned()
            .unwrap_or_else(|| TargetConfig {
                host: host_clone.clone(),
                ..TargetConfig::default()
            });
        hostname_handles.push(tokio::spawn(resolve_single_os_hostname(host_clone, tc)));
    }
    let mut host_to_os_hostname = HashMap::new();
    let mut failures = Vec::new();
    for h in hostname_handles {
        match h.await {
            Ok(Ok((host, os_hostname))) => {
                host_to_os_hostname.insert(host, os_hostname);
            }
            Ok(Err(e)) => {
                tracing::warn!("Failed to fetch hostname: {e}");
                failures.push(e);
            }
            Err(e) => {
                let msg = format!("Task join error: {e}");
                tracing::warn!("{msg}");
                failures.push(msg);
            }
        }
    }
    if !failures.is_empty() && !best_effort {
        return Err(format!(
            "hostname resolution failed on {} host(s): {}",
            failures.len(),
            failures.join("; ")
        ));
    }
    if !failures.is_empty() {
        crate::log_println!(
            "Warning: hostname resolution failed on {} host(s) (best-effort mode)",
            failures.len()
        );
    }
    Ok(host_to_os_hostname)
}

fn map_name_to_loopback_cmd(name: &str) -> String {
    let (pid, rand_id) = unique_tmp_id();
    let tmp_path = format!("/tmp/.rtkk_loopback.{pid}.{rand_id:016x}");
    let tmp_q = shell_single_quote(&tmp_path);
    let name_q = shell_single_quote(name);
    let escaped = escape_unescaped_slash(&sed_replacement_escape(name));
    let sed_expr = shell_single_quote(&format!(
        "s/^([[:space:]]*127\\.0\\.0\\.1([[:space:]]+[^#]*[^#[:space:]])?)([[:space:]]*#.*)?$/\\1 {escaped}\\3/"
    ));
    let fallback_line = shell_single_quote(&format!("127.0.0.1 localhost {name}"));
    format!(
        "sed 's/#.*//' /etc/hosts | grep -E '^[[:space:]]*127\\.0\\.0\\.1([[:space:]]|$)' | tr '[:space:]' '\\n' | grep -qxF -- {name_q} || \
         (if grep -qE '^[[:space:]]*127\\.0\\.0\\.1([[:space:]]|#|$)' /etc/hosts; then \
            (sed -E {sed_expr} /etc/hosts > {tmp_q} && cat {tmp_q} > /etc/hosts && rm -f {tmp_q} || (rm -f {tmp_q}; exit 1)); \
          else printf '%s\\n' {fallback_line} >> /etc/hosts; fi)"
    )
}

fn build_rewrite_hosts_block_cmd(host_block: &str) -> String {
    let (pid, rand_id) = unique_tmp_id();
    let tmp_path = format!("/tmp/.rtkk_hosts.{pid}.{rand_id:016x}");
    let tmp_q = shell_single_quote(&tmp_path);
    let block_q = shell_single_quote(host_block);
    let awk_script = shell_single_quote(
        "/^# RTKK HOSTS START$/ { skip=1; next } \
         /^# RTKK HOSTS END$/ { skip=0; next } \
         !skip { lines[++n] = $0 } \
         END { \
           while (n > 0 && lines[n] ~ /^[[:space:]]*$/) n--; \
           for (i = 1; i <= n; i++) print lines[i]; \
           if (n > 0) print \"\" \
         }",
    );
    format!(
        "[ -f /etc/hosts ] || touch /etc/hosts; \
         (awk {awk_script} /etc/hosts && printf '%s' {block_q}) > {tmp_q} && \
         cat {tmp_q} > /etc/hosts && rm -f {tmp_q} || (rm -f {tmp_q}; exit 1)"
    )
}

async fn update_single_node_hosts(
    host: String,
    tc: TargetConfig,
    os_name: String,
    host_block: String,
) -> Result<String, String> {
    crate::logging::start_task(&host, "Update /etc/hosts");
    let conn = match create_connector(&tc).await {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("failed to connect to {host}: {e}");
            crate::logging::finish_task_failed(&host, "Update /etc/hosts", &msg);
            return Err(msg);
        }
    };
    let step_res = async {
        let rewrite_cmd = build_rewrite_hosts_block_cmd(&host_block);
        let res = conn
            .execute_shell(&rewrite_cmd)
            .await
            .map_err(|e| e.to_string())?;
        if res.exit_code != 0 {
            let stderr_str = String::from_utf8_lossy(&res.stderr);
            return Err(format!(
                "prep failed on {host} with exit code {}: {stderr_str}",
                res.exit_code
            ));
        }
        let local_inv_cmd = map_name_to_loopback_cmd(&host);
        let inv_res = conn
            .execute_shell(&local_inv_cmd)
            .await
            .map_err(|e| e.to_string())?;
        if inv_res.exit_code != 0 {
            return Err(format!(
                "failed to map inventory hostname locally on {host}"
            ));
        }
        if !os_name.is_empty() && os_name != host {
            let local_os_cmd = map_name_to_loopback_cmd(&os_name);
            let os_res = conn
                .execute_shell(&local_os_cmd)
                .await
                .map_err(|e| e.to_string())?;
            if os_res.exit_code != 0 {
                return Err(format!("failed to map OS hostname locally on {host}"));
            }
        }
        Ok(host.clone())
    }
    .await;
    let _ = conn.close().await;
    match &step_res {
        Ok(_) => crate::logging::finish_task_success(&host, "Update /etc/hosts"),
        Err(e) => crate::logging::finish_task_failed(&host, "Update /etc/hosts", e),
    }
    step_res
}

async fn update_hosts_on_nodes<S: BuildHasher + Sync>(
    flat_inventory: &FlatInventory,
    target_configs: &HashMap<String, TargetConfig, S>,
    host_to_os_hostname: &HashMap<String, String>,
    host_block: &str,
    best_effort: bool,
) -> Result<Vec<String>, String> {
    let mut handles = Vec::new();
    for host in sorted_remote_hosts(flat_inventory) {
        let host_clone = host.clone();
        let host_block_clone = host_block.to_string();
        let tc = target_configs
            .get(host)
            .cloned()
            .unwrap_or_else(|| TargetConfig {
                host: host_clone.clone(),
                ..TargetConfig::default()
            });
        let os_name = host_to_os_hostname.get(host).cloned().unwrap_or_default();
        handles.push(tokio::spawn(update_single_node_hosts(
            host_clone,
            tc,
            os_name,
            host_block_clone,
        )));
    }
    let mut succeeded = Vec::new();
    let mut failures = Vec::new();
    for h in handles {
        match h.await {
            Ok(Ok(host_ok)) => succeeded.push(host_ok),
            Ok(Err(e)) => {
                tracing::warn!("Failed to update /etc/hosts: {e}");
                failures.push(e);
            }
            Err(e) => {
                let msg = format!("Task join error: {e}");
                tracing::warn!("{msg}");
                failures.push(msg);
            }
        }
    }
    if !failures.is_empty() && !best_effort {
        return Err(format!(
            "/etc/hosts update failed on {} host(s): {}",
            failures.len(),
            failures.join("; ")
        ));
    }
    if !failures.is_empty() {
        crate::log_println!(
            "Warning: /etc/hosts update failed on {} host(s) (best-effort mode)",
            failures.len()
        );
    }
    succeeded.sort();
    Ok(succeeded)
}

/// Resolves `OS` hostnames across inventory nodes and updates `/etc/hosts` idempotently.
///
/// # Errors
///
/// Returns an error string if any hostname or `IP` contains invalid characters, or if node
/// inspection or `/etc/hosts` updates fail when `best_effort` is `false`.
pub async fn execute_prep_hosts<S: BuildHasher + Sync>(
    inventory_path: &str,
    flat_inventory: &FlatInventory,
    target_configs: &HashMap<String, TargetConfig, S>,
    best_effort: bool,
) -> Result<(), String> {
    crate::log_println!("Reading inventory from: {inventory_path}");
    crate::log_println!("Resolving actual OS hostnames on inventory nodes...");
    let host_to_os_hostname =
        get_hosts_os_hostnames(flat_inventory, target_configs, best_effort).await?;
    let mut host_block = String::from("# RTKK HOSTS START\n");
    let mut resolved_details = Vec::new();
    for hname in sorted_remote_hosts(flat_inventory) {
        validate_hosts_field("inventory hostname", hname)?;
        if let Some(vars) = flat_inventory.host_vars.get(hname)
            && let Some(ip) = vars.get("ip").and_then(Value::as_str)
        {
            validate_hosts_field("host IP", ip)?;
            let os_name = host_to_os_hostname.get(hname).map_or("", String::as_str);
            if !os_name.is_empty() {
                validate_hosts_field("OS hostname", os_name)?;
            }
            resolved_details.push(format!("  - {hname} ({ip}) -> OS hostname: {os_name}"));
            host_block.push_str(ip);
            host_block.push(' ');
            host_block.push_str(hname);
            if !os_name.is_empty() && os_name != hname {
                host_block.push(' ');
                host_block.push_str(os_name);
            }
            host_block.push('\n');
        }
    }
    host_block.push_str("# RTKK HOSTS END\n");
    for detail in resolved_details {
        crate::log_println!("{detail}");
    }
    crate::log_println!("\nGenerated /etc/hosts mapping block:\n{host_block}");
    crate::log_println!("Updating /etc/hosts on all nodes...");
    let updated_hosts = update_hosts_on_nodes(
        flat_inventory,
        target_configs,
        &host_to_os_hostname,
        &host_block,
        best_effort,
    )
    .await?;
    crate::log_println!("Successfully updated /etc/hosts on nodes: {updated_hosts:?}");
    crate::log_println!("Prep hosts completed successfully.");
    crate::logging::shutdown_logging();
    Ok(())
}
