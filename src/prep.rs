use crate::connector::{TargetConfig, create_connector};
use crate::inventory::FlatInventory;
use crate::utils::shell_single_quote;
use serde_json::Value;
use std::collections::HashMap;
async fn get_hosts_os_hostnames(
    flat_inventory: &FlatInventory,
    target_configs: &HashMap<String, TargetConfig>,
    best_effort: bool,
) -> Result<HashMap<String, String>, String> {
    let mut hostname_handles = Vec::new();
    for host in flat_inventory.host_vars.keys() {
        if host == "localhost" {
            continue;
        }
        let host_clone = host.clone();
        let tc = target_configs
            .get(host)
            .cloned()
            .unwrap_or_else(|| TargetConfig {
                host: host_clone.clone(),
                connector_type: None,
                ssh: None,
            });
        crate::logging::start_task(&host_clone, "Resolve OS hostname");
        hostname_handles.push(tokio::spawn(async move {
            let res = async {
                let conn = create_connector(&tc).await.map_err(|e| e.to_string())?;
                let res = conn
                    .execute_command("hostname")
                    .await
                    .map_err(|e| e.to_string())?;
                if res.exit_code != 0 {
                    let stderr_str = String::from_utf8_lossy(&res.stderr);
                    return Err(format!(
                        "failed to get hostname on {host_clone}: {stderr_str}"
                    ));
                }
                let os_hostname = String::from_utf8_lossy(&res.stdout).trim().to_string();
                Ok(os_hostname)
            }
            .await;
            match &res {
                Ok(_) => crate::logging::finish_task_success(&host_clone, "Resolve OS hostname"),
                Err(e) => crate::logging::finish_task_failed(&host_clone, "Resolve OS hostname", e),
            }
            res.map(|os_hostname| (host_clone, os_hostname))
        }));
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
    let name_q = shell_single_quote(name);
    let sed_expr = shell_single_quote(&format!("s/^[[:space:]]*127.0.0.1.*/& {name}/"));
    format!(
        "grep -E '^[[:space:]]*127.0.0.1[[:space:]]' /etc/hosts | grep -qF {name_q} || sed -i.bak {sed_expr} /etc/hosts"
    )
}
async fn update_hosts_on_nodes(
    flat_inventory: &FlatInventory,
    target_configs: &HashMap<String, TargetConfig>,
    host_to_os_hostname: &HashMap<String, String>,
    host_block: &str,
) -> Result<(), String> {
    let mut handles = Vec::new();
    for host in flat_inventory.host_vars.keys() {
        if host == "localhost" {
            continue;
        }
        let host_clone = host.clone();
        let host_block_clone = host_block.to_string();
        let tc = target_configs
            .get(host)
            .cloned()
            .unwrap_or_else(|| TargetConfig {
                host: host_clone.clone(),
                connector_type: None,
                ssh: None,
            });
        let os_name = host_to_os_hostname.get(host).cloned().unwrap_or_default();
        crate::logging::start_task(&host_clone, "Update /etc/hosts");
        handles.push(tokio::spawn(async move {
            let res = async {
                let conn = create_connector(&tc).await.map_err(|e| e.to_string())?;
                let check_cmd = "grep -q '# RTKK HOSTS START' /etc/hosts";
                let check_res = conn.execute_command(check_cmd).await;
                let host_block_q = shell_single_quote(&host_block_clone);
                let append_cmd = if check_res.as_ref().is_ok_and(|r| r.exit_code == 0) {
                    format!(
                        "sed -i '/# RTKK HOSTS START/,/# RTKK HOSTS END/d' /etc/hosts && echo {host_block_q} >> /etc/hosts"
                    )
                } else {
                    format!("echo {host_block_q} >> /etc/hosts")
                };
                let res = conn
                    .execute_command(&append_cmd)
                    .await
                    .map_err(|e| e.to_string())?;
                if res.exit_code != 0 {
                    let exit_code = res.exit_code;
                    let stderr_str = String::from_utf8_lossy(&res.stderr);
                    return Err(format!(
                        "prep failed on {host_clone} with exit code {exit_code}: {stderr_str}"
                    ));
                }
                let local_inv_cmd = map_name_to_loopback_cmd(&host_clone);
                let res = conn
                    .execute_command(&local_inv_cmd)
                    .await
                    .map_err(|e| e.to_string())?;
                if res.exit_code != 0 {
                    return Err(format!(
                        "failed to map inventory hostname locally on {host_clone}"
                    ));
                }
                if !os_name.is_empty() && os_name != host_clone {
                    let local_os_cmd = map_name_to_loopback_cmd(&os_name);
                    let res = conn
                        .execute_command(&local_os_cmd)
                        .await
                        .map_err(|e| e.to_string())?;
                    if res.exit_code != 0 {
                        return Err(format!(
                            "failed to map OS hostname locally on {host_clone}"
                        ));
                    }
                }
                Ok(())
            }
            .await;
            match &res {
                Ok(()) => crate::logging::finish_task_success(&host_clone, "Update /etc/hosts"),
                Err(e) => crate::logging::finish_task_failed(&host_clone, "Update /etc/hosts", e),
            }
            res
        }));
    }
    for h in handles {
        h.await.map_err(|e| e.to_string())??;
    }
    Ok(())
}
pub(crate) async fn execute_prep_hosts(
    inventory_path: &str,
    flat_inventory: &FlatInventory,
    target_configs: &HashMap<String, TargetConfig>,
    best_effort: bool,
) -> Result<(), String> {
    crate::log_println!("Reading inventory from: {inventory_path}");
    crate::log_println!("Resolving actual OS hostnames on inventory nodes...");
    let host_to_os_hostname =
        get_hosts_os_hostnames(flat_inventory, target_configs, best_effort).await?;
    let mut host_block = String::new();
    host_block.push_str("\n# RTKK HOSTS START\n");
    let mut resolved_details = Vec::new();
    let mut updated_hosts = Vec::new();
    for (hname, vars) in &flat_inventory.host_vars {
        if hname != "localhost" {
            if let Some(ip) = vars.get("ip").and_then(Value::as_str) {
                let os_name = host_to_os_hostname.get(hname).map_or("", String::as_str);
                resolved_details.push(format!("  - {hname} ({ip}) -> OS hostname: {os_name}"));
                updated_hosts.push(hname.clone());
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
    }
    host_block.push_str("# RTKK HOSTS END\n");
    for detail in resolved_details {
        crate::log_println!("{detail}");
    }
    crate::log_println!("\nGenerated /etc/hosts mapping block:{host_block}");
    crate::log_println!("Updating /etc/hosts on all nodes...");
    update_hosts_on_nodes(
        flat_inventory,
        target_configs,
        &host_to_os_hostname,
        &host_block,
    )
    .await?;
    crate::log_println!("Successfully updated /etc/hosts on nodes: {updated_hosts:?}");
    crate::log_println!("Prep hosts completed successfully.");
    crate::logging::shutdown_logging();
    Ok(())
}
