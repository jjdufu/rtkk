//! Command-line interface parsing, inventory loading, and subcommand dispatch.

use crate::cert::{
    CertPolicy, GenCertArgs, generate_certificate, parse_duration_hours, verify_certificate_files,
};
use crate::connector::{
    BecomeMethod, ConnectorType, HostKeyPolicy, PrivilegeConfig, SshConfig, TargetConfig,
};
use crate::error::RtkkError;
use crate::inventory::{FlatInventory, Inventory};
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Top-level `rtkk` command-line parser.
#[derive(Parser, Debug)]
#[command(
    name = "rtkk",
    version = env!("CARGO_PKG_VERSION"),
    about = "Pure-Rust Kubernetes & Etcd deployment and OCI image synchronization tool",
    override_usage = "rtkk [OPTIONS] <COMMAND>"
)]
pub struct Cli {
    /// Optional log file path (defaults to `deploy.log`).
    #[arg(
        long,
        short = 'l',
        global = true,
        help = "Write all execution outputs and logs to the specified file directly."
    )]
    pub log_file: Option<String>,
    /// Selected CLI subcommand.
    #[command(subcommand)]
    pub command: Commands,
}

/// Supported top-level subcommands for `rtkk`.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Execute a YAML playbook against an inventory.
    #[command(about = "Run Playbook")]
    Run {
        /// Path to the playbook YAML file.
        playbook_path: String,
        /// Path to the inventory YAML file.
        #[arg(
            long,
            short = 'i',
            required = true,
            help = "Path to the inventory file"
        )]
        inventory: String,
        /// Extra `key=value` variable overrides.
        #[arg(long, short = 's', help = "Set extra variable (key=val)", value_parser = parse_key_value)]
        set: Vec<(String, String)>,
        /// Maximum parallel host concurrency.
        #[arg(long, short = 'f', help = "Concurrency limit (forks, must be >= 1)")]
        forks: Option<usize>,
        /// Comma-separated tag filter.
        #[arg(
            long,
            help = "Only run tasks with these tags (comma-separated). Empty = all tasks."
        )]
        tags: Option<String>,
        /// Per-task execution timeout in seconds (`0` or omitted = unlimited).
        #[arg(
            long,
            help = "Global per-task timeout in seconds (0 or omitted = unlimited)"
        )]
        task_timeout: Option<u64>,
    },
    /// Prepare target cluster nodes (e.g. `/etc/hosts` synchronization).
    #[command(about = "Prepare target nodes")]
    Prep {
        /// Node preparation subcommand.
        #[command(subcommand)]
        sub: PrepSubcommands,
    },
    /// Generate or verify `X.509` certificates (defaults to 99-year validity, configurable to 1y/multi-year).
    #[command(about = "Generate or verify X.509 certificates")]
    Cert {
        /// Certificate management subcommand.
        #[command(subcommand)]
        sub: CertSubcommands,
    },
    /// Push `Docker`-legacy `OCI` image tarballs to a target registry.
    #[command(
        about = "Push OCI images to registry (single tar XOR list batch)",
        after_help = "Two mutually exclusive modes:\n  \
            Single tar:  rtkk push-images <TAR> -i <inventory> [--retag]\n  \
            List batch:  rtkk push-images -i <inventory> --images-file <images.yaml> [--output-dir DIR] [--retag]\n  \
            Do not pass both a tar path and --images-file."
    )]
    PushImages {
        /// Single `Docker`-legacy tar path (mutually exclusive with `--images-file`).
        #[arg(help = "Single docker-legacy tar path (omit when using --images-file)")]
        tar_path: Option<String>,
        /// Path to the inventory YAML file.
        #[arg(
            long,
            short = 'i',
            required = true,
            help = "Path to the inventory file"
        )]
        inventory: String,
        /// Images list YAML for batch push.
        #[arg(
            long,
            help = "Images list YAML for batch push (mutually exclusive with tar path)"
        )]
        images_file: Option<String>,
        /// Tarball directory override for batch mode.
        #[arg(
            long,
            help = "Tar directory for batch mode (default: vars.image_sync.output_dir)"
        )]
        output_dir: Option<String>,
        /// Rewrite or remap offline archive tags to match the configured target registry.
        #[arg(
            long,
            help = "Retag offline archive RepoTags to target registry prefix when pushing across environments"
        )]
        retag: bool,
    },
    /// Pull `OCI` images into local `Docker`-legacy tarballs.
    #[command(about = "Pull OCI images into local docker-legacy tarballs (pure Rust)")]
    PullImages {
        /// Path to the inventory YAML file.
        #[arg(
            long,
            short = 'i',
            required = true,
            help = "Path to the inventory file"
        )]
        inventory: String,
        /// Path to the images list YAML file.
        #[arg(
            long,
            required = true,
            help = "Path to images list YAML (templates rendered with inventory vars)"
        )]
        images_file: String,
        /// Output directory for downloaded `*.tar` archives.
        #[arg(
            long,
            help = "Output directory for *.tar (default: vars.image_sync.output_dir)"
        )]
        output_dir: Option<String>,
        /// Target platform in `os/arch` format.
        #[arg(
            long,
            help = "Platform os/arch (default: vars.image_sync.platform or linux/amd64)"
        )]
        platform: Option<String>,
        /// Overwrite existing tarballs even if matching.
        #[arg(long, help = "Overwrite existing tar files")]
        force: bool,
        /// Accept self-signed or invalid `TLS` certificates when pulling.
        #[arg(long, help = "Accept invalid TLS certificates for pull")]
        insecure: bool,
    },
    /// Pull `OCI` images to local tarballs and push them to the configured private registry.
    #[command(about = "Pull images to local tar then push to private registry")]
    SyncImages {
        /// Path to the inventory YAML file.
        #[arg(
            long,
            short = 'i',
            required = true,
            help = "Path to the inventory file"
        )]
        inventory: String,
        /// Path to the images list YAML file.
        #[arg(
            long,
            required = true,
            help = "Path to images list YAML (templates rendered with inventory vars)"
        )]
        images_file: String,
        /// Output directory for intermediate `*.tar` archives.
        #[arg(
            long,
            help = "Output directory for *.tar (default: vars.image_sync.output_dir)"
        )]
        output_dir: Option<String>,
        /// Target platform in `os/arch` format.
        #[arg(
            long,
            help = "Platform os/arch (default: vars.image_sync.platform or linux/amd64)"
        )]
        platform: Option<String>,
        /// Overwrite existing tarballs before pushing.
        #[arg(long, help = "Overwrite existing tar files")]
        force: bool,
        /// Accept self-signed or invalid `TLS` certificates when pulling.
        #[arg(long, help = "Accept invalid TLS certificates for pull")]
        insecure: bool,
    },
    /// Start the long-running HTTP/SSE Daemon and embedded Park UI console (`127.0.0.1:19912`).
    #[command(about = "Start rtkk HTTP/SSE daemon and embedded WebUI console")]
    Serve {
        /// Listen socket address (defaults to `RTKK_BIND_ADDR` or `127.0.0.1:19912`).
        #[arg(
            long,
            help = "Bind address (default: RTKK_BIND_ADDR or 127.0.0.1:19912)"
        )]
        listen: Option<String>,
        /// Workspace root directory (defaults to `RTKK_WORK_DIR` or `.`).
        #[arg(long, help = "Workspace directory (default: RTKK_WORK_DIR or .)")]
        work_dir: Option<PathBuf>,
        /// Admin token file path (defaults to `<work-dir>/.rtkk-daemon/admin.token`).
        #[arg(long, help = "Admin token file path (`0600`)")]
        token_file: Option<PathBuf>,
    },
    /// Probe a running `rtkk serve` daemon health endpoint.
    #[command(about = "Probe daemon /api/v1/health endpoint")]
    Healthcheck {
        /// Full health endpoint URL.
        #[arg(
            long,
            default_value = "http://127.0.0.1:19912/api/v1/health",
            help = "Daemon health check URL"
        )]
        url: String,
        /// Request timeout in seconds.
        #[arg(long, default_value_t = 3, help = "HTTP probe timeout in seconds")]
        timeout_secs: u64,
    },
}

/// Subcommands under `rtkk prep`.
#[derive(Subcommand, Debug)]
pub enum PrepSubcommands {
    /// Synchronize `/etc/hosts` mappings across all target nodes.
    #[command(about = "Update hosts mapping on all target nodes")]
    Hosts {
        /// Path to the inventory YAML file.
        #[arg(
            long,
            short = 'i',
            required = true,
            help = "Path to the inventory file"
        )]
        inventory: String,
        /// Continue updating reachable nodes even if some hosts fail resolution.
        #[arg(
            long,
            help = "Continue when some hosts fail hostname resolution (default: fail-fast)"
        )]
        best_effort: bool,
    },
}

/// Subcommands under `rtkk cert`.
#[derive(Subcommand, Debug)]
pub enum CertSubcommands {
    /// Generate a self-signed CA or CA-signed `ECDSA` P-256 certificate.
    #[command(about = "Generate a CA or leaf X.509 certificate (default validity: 99y / 867240h)")]
    Gen {
        /// Certificate Subject Common Name (`CN`).
        #[arg(long, help = "Subject Common Name (CN)")]
        cn: String,
        /// Output path for the generated `PEM` private key (`0600`).
        #[arg(long, help = "Output private key PEM path")]
        out_key: PathBuf,
        /// Output path for the generated `PEM` certificate (`0644`).
        #[arg(long, help = "Output certificate PEM path")]
        out_cert: PathBuf,
        /// Signing CA private key `PEM` path (requires `--root-cert` / `--ca-cert`).
        #[arg(
            long,
            visible_alias = "ca-key",
            help = "Issuer CA private key PEM path (for leaf certificates)"
        )]
        root_key: Option<PathBuf>,
        /// Signing CA certificate `PEM` path (requires `--root-key` / `--ca-key`).
        #[arg(
            long,
            visible_alias = "ca-cert",
            help = "Issuer CA certificate PEM path (for leaf certificates)"
        )]
        root_cert: Option<PathBuf>,
        /// Subject Alternative Names (`DNS` names or `IPv4`/`IPv6` addresses).
        #[arg(
            long = "san",
            help = "Subject Alternative Name (repeatable: DNS or IP)"
        )]
        sans: Vec<String>,
        /// Issue a Certificate Authority (`CA:TRUE`) certificate.
        #[arg(
            long = "ca",
            visible_alias = "is-ca",
            help = "Generate a CA certificate"
        )]
        is_ca: bool,
        /// Certificate validity duration (e.g. `867240h`, `99y`, `8760h`, `1y`, `365d`).
        #[arg(
            long,
            default_value = "867240h",
            help = "Certificate validity (default 867240h = 99y; supports e.g. 1y, 3y, 10y, 365d, 8760h)"
        )]
        validity: String,
        /// Generation policy (`always`, `ifnotpresent`, or `none`).
        #[arg(
            long,
            default_value = "always",
            help = "Generation policy: always | ifnotpresent | none"
        )]
        policy: String,
    },
    /// Inspect and verify an `X.509` certificate, optional private key match, and optional CA chain.
    #[command(about = "Verify X.509 certificate validity, keypair match, and CA signature")]
    Verify {
        /// Path to the certificate `PEM` file.
        cert_path: PathBuf,
        /// Optional path to private key `PEM` file to verify public-key match.
        #[arg(long, help = "Private key PEM path to verify keypair match")]
        key: Option<PathBuf>,
        /// Optional path to issuer CA certificate `PEM` file to verify signature chain.
        #[arg(
            long,
            visible_alias = "ca-cert",
            help = "CA certificate PEM path to verify issuer signature"
        )]
        ca: Option<PathBuf>,
    },
}

/// Loaded and resolved inventory state ready for execution.
#[derive(Debug)]
pub struct LoadedInventory {
    /// Raw parsed YAML inventory model.
    pub inventory: Inventory,
    /// Flattened groups and merged per-host variables.
    pub flat: FlatInventory,
    /// Per-host transport and privilege escalation configurations.
    pub target_configs: HashMap<String, TargetConfig>,
    /// Parent directory of the loaded inventory file.
    pub inventory_dir: Option<PathBuf>,
}

fn parse_key_value(s: &str) -> Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("invalid key=value: '{s}'"))?;
    if k.trim().is_empty() {
        return Err(format!("invalid key=value (empty key): '{s}'"));
    }
    Ok((k.trim().to_string(), v.to_string()))
}

fn parse_bool_val(val: &Value) -> Option<bool> {
    if let Some(b) = val.as_bool() {
        return Some(b);
    }
    let s = val.as_str()?.trim().to_ascii_lowercase();
    match s.as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_u16_val(val: &Value) -> Option<u16> {
    if let Some(n) = val.as_u64() {
        return u16::try_from(n).ok();
    }
    val.as_str()?.trim().parse::<u16>().ok()
}

fn host_key_policy_from_vars(vars: &HashMap<String, Value>) -> HostKeyPolicy {
    let raw = vars
        .get("ansible_ssh_host_key_checking")
        .or_else(|| vars.get("host_key_checking"));
    let Some(val) = raw else {
        return HostKeyPolicy::Off;
    };
    if let Some(b) = val.as_bool() {
        return if b {
            HostKeyPolicy::Strict
        } else {
            HostKeyPolicy::Off
        };
    }
    if let Some(s) = val.as_str() {
        return match s.trim().to_ascii_lowercase().as_str() {
            "accept-new" | "accept_new" => HostKeyPolicy::AcceptNew,
            "true" | "1" | "yes" | "strict" => HostKeyPolicy::Strict,
            _ => HostKeyPolicy::Off,
        };
    }
    HostKeyPolicy::Off
}

fn ssh_config_from_vars(vars: &HashMap<String, Value>) -> Option<SshConfig> {
    let user = vars.get("ansible_user").and_then(Value::as_str)?;
    let password = vars
        .get("ansible_ssh_pass")
        .and_then(Value::as_str)
        .map(String::from);
    let private_key = vars
        .get("ansible_ssh_private_key_file")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(crate::utils::expand_tilde_str);
    let port = vars.get("ansible_port").and_then(parse_u16_val);
    let private_key_fallback_password = vars
        .get("ansible_ssh_private_key_fallback_password")
        .and_then(parse_bool_val)
        .unwrap_or(false);
    let known_hosts_file = vars
        .get("ansible_ssh_known_hosts_file")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(crate::utils::expand_tilde_str);
    Some(SshConfig {
        username: user.to_string(),
        port,
        private_key_content: None,
        private_key,
        password,
        private_key_fallback_password,
        host_key_policy: host_key_policy_from_vars(vars),
        known_hosts_file,
    })
}

fn privilege_from_vars(
    vars: &HashMap<String, Value>,
) -> Result<Option<PrivilegeConfig>, RtkkError> {
    let become_enabled = vars
        .get("ansible_become")
        .or_else(|| vars.get("become"))
        .and_then(parse_bool_val)
        .unwrap_or(false);
    let become_method_opt = match vars
        .get("ansible_become_method")
        .or_else(|| vars.get("become_method"))
        .and_then(Value::as_str)
    {
        Some(raw) => Some(BecomeMethod::parse_str(raw).map_err(RtkkError::Inventory)?),
        None => None,
    };
    let become_user = vars
        .get("ansible_become_user")
        .or_else(|| vars.get("become_user"))
        .and_then(Value::as_str)
        .map(String::from);
    let become_password = vars
        .get("ansible_become_pass")
        .or_else(|| vars.get("ansible_become_password"))
        .or_else(|| vars.get("become_pass"))
        .and_then(Value::as_str)
        .map(String::from);
    if !become_enabled
        && become_method_opt.is_none()
        && become_user.is_none()
        && become_password.is_none()
    {
        return Ok(None);
    }
    Ok(Some(PrivilegeConfig {
        become_enabled,
        become_method: become_method_opt.unwrap_or_default(),
        become_user,
        become_password,
    }))
}

fn connector_type_from_vars(vars: &HashMap<String, Value>) -> Option<ConnectorType> {
    let raw = vars
        .get("ansible_connection")
        .and_then(Value::as_str)
        .or_else(|| vars.get("connector_type").and_then(Value::as_str))?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "local" => Some(ConnectorType::Local),
        "ssh" => Some(ConnectorType::Ssh),
        _ => None,
    }
}

/// Constructs per-host [`TargetConfig`] entries from a resolved [`FlatInventory`].
///
/// # Errors
///
/// Returns [`RtkkError::Inventory`] if any host specifies an unrecognized privilege escalation method.
pub fn build_target_configs_for_flat(
    flat: &FlatInventory,
) -> Result<HashMap<String, TargetConfig>, RtkkError> {
    let mut target_configs = HashMap::new();
    for (host, vars) in &flat.host_vars {
        let privilege = privilege_from_vars(vars)?;
        if host == "localhost" {
            target_configs.insert(
                host.clone(),
                TargetConfig {
                    host: "127.0.0.1".to_string(),
                    connector_type: Some(ConnectorType::Local),
                    ssh: None,
                    privilege,
                },
            );
            continue;
        }
        let host_ip = vars
            .get("ip")
            .and_then(Value::as_str)
            .map_or_else(|| host.clone(), String::from);
        target_configs.insert(
            host.clone(),
            TargetConfig {
                host: host_ip,
                connector_type: connector_type_from_vars(vars),
                ssh: ssh_config_from_vars(vars),
                privilege,
            },
        );
    }
    Ok(target_configs)
}

/// Load and resolve an inventory file without extra CLI `-s` overrides.
///
/// # Errors
///
/// Returns [`RtkkError::Inventory`] if the file cannot be read, parsed, or resolved.
pub async fn load_inventory(inventory_path: &str) -> Result<LoadedInventory, RtkkError> {
    load_inventory_with_overrides(inventory_path, &[]).await
}

/// Load and resolve an inventory file, applying `-s key=value` overrides before building connectors.
///
/// # Errors
///
/// Returns [`RtkkError::Inventory`] if reading, parsing, or resolving the inventory fails.
pub async fn load_inventory_with_overrides(
    inventory_path: &str,
    set_vars: &[(String, String)],
) -> Result<LoadedInventory, RtkkError> {
    let inventory_content = tokio::fs::read_to_string(inventory_path)
        .await
        .map_err(|e| {
            RtkkError::Inventory(format!("failed to read inventory {inventory_path}: {e}"))
        })?;
    let inventory: Inventory = serde_yml::from_str(&inventory_content)
        .map_err(|e| RtkkError::Inventory(format!("failed to parse inventory: {e}")))?;
    let mut flat = crate::inventory::resolve_inventory(&inventory).map_err(RtkkError::Inventory)?;
    if !set_vars.is_empty() {
        for vars in flat.host_vars.values_mut() {
            for (k, v) in set_vars {
                vars.insert(k.clone(), Value::String(v.clone()));
            }
        }
    }
    for (hname, hvars) in &flat.host_vars {
        if let Some(cv) = hvars.get("cert_validity")
            && !cv.is_null()
            && cv.as_str().is_none_or(|s| !s.trim().is_empty())
        {
            crate::cert::parse_duration_value(cv).map_err(|e| {
                RtkkError::Inventory(format!("invalid cert_validity on host '{hname}': {e}"))
            })?;
        }
    }
    crate::playbook::inject_dynamic_inventory_vars(&flat.groups, &mut flat.host_vars);
    crate::playbook::inject_runtime_vars(&mut flat.host_vars, inventory_path);
    let target_configs = build_target_configs_for_flat(&flat)?;
    let inventory_dir = Path::new(inventory_path).parent().map(|p| {
        if p.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            p.to_path_buf()
        }
    });
    Ok(LoadedInventory {
        inventory,
        flat,
        target_configs,
        inventory_dir,
    })
}

fn parse_usize_val(f_val: &Value) -> Option<usize> {
    if let Some(f_str) = f_val.as_str() {
        return f_str.trim().parse::<usize>().ok();
    }
    f_val.as_u64().and_then(|u| usize::try_from(u).ok())
}

fn forks_from_inventory(inventory: &Inventory) -> Option<usize> {
    if let Some(f_val) = inventory
        .all
        .as_ref()
        .and_then(|a| a.vars.as_ref())
        .and_then(|v| v.get("forks"))
    {
        return parse_usize_val(f_val);
    }
    inventory
        .vars
        .as_ref()
        .and_then(|v| v.get("forks"))
        .and_then(parse_usize_val)
}

async fn execute_run(
    playbook_path: &str,
    inventory_path: &str,
    set_vars: &[(String, String)],
    forks: Option<usize>,
    tags: Option<&str>,
    task_timeout: Option<u64>,
) -> Result<(), RtkkError> {
    let loaded = load_inventory_with_overrides(inventory_path, set_vars).await?;
    let forks_count = forks
        .or_else(|| forks_from_inventory(&loaded.inventory))
        .unwrap_or(50);
    if forks_count == 0 {
        return Err(RtkkError::Playbook(
            "forks concurrency limit must be at least 1".to_string(),
        ));
    }
    let playbook_path_buf = PathBuf::from(playbook_path);
    let playbook =
        tokio::task::spawn_blocking(move || crate::playbook::compile_playbook(&playbook_path_buf))
            .await
            .map_err(|e| RtkkError::Playbook(format!("spawn_blocking join error: {e}")))?
            .map_err(RtkkError::Playbook)?;
    let mut engine = crate::playbook::ExecutionEngine::new(
        loaded.flat,
        loaded.target_configs,
        forks_count,
        loaded.inventory_dir,
    );
    if let Some(tag_str) = tags {
        engine.tag_filter = tag_str
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    if let Some(t) = task_timeout.filter(|t| *t > 0) {
        engine.task_timeout_secs = Some(t);
    }
    let engine = Arc::new(engine);
    engine
        .run_playbook(&playbook)
        .await
        .map_err(RtkkError::Task)?;
    crate::log_println!("Playbook executed successfully.");
    Ok(())
}

async fn dispatch_cert(sub: CertSubcommands) -> Result<(), RtkkError> {
    match sub {
        CertSubcommands::Gen {
            cn,
            out_key,
            out_cert,
            root_key,
            root_cert,
            sans,
            is_ca,
            validity,
            policy,
        } => {
            let date_hours = parse_duration_hours(&validity).map_err(RtkkError::Cert)?;
            let cert_policy = CertPolicy::from_str_policy(&policy).map_err(RtkkError::Cert)?;
            let args = GenCertArgs {
                cn,
                out_key: out_key.clone(),
                out_cert: out_cert.clone(),
                root_key,
                root_cert,
                sans,
                is_ca,
                date_hours,
                policy: cert_policy,
            };
            let changed = tokio::task::spawn_blocking(move || generate_certificate(&args))
                .await
                .map_err(|e| RtkkError::Cert(format!("cert task join error: {e}")))?
                .map_err(|e| RtkkError::Cert(e.to_string()))?;
            if changed {
                crate::log_println!(
                    "Generated certificate {} and key {} (validity={}h).",
                    out_cert.display(),
                    out_key.display(),
                    date_hours
                );
            } else {
                crate::log_println!(
                    "Certificate {} already valid under requested policy; skipped.",
                    out_cert.display()
                );
            }
            Ok(())
        }
        CertSubcommands::Verify { cert_path, key, ca } => {
            let report = tokio::task::spawn_blocking(move || {
                verify_certificate_files(&cert_path, key.as_deref(), ca.as_deref())
            })
            .await
            .map_err(|e| RtkkError::Cert(format!("cert verify join error: {e}")))?
            .map_err(|e| RtkkError::Cert(e.to_string()))?;
            crate::log_print!("{report}");
            Ok(())
        }
    }
}

fn pull_images_options(
    inventory: String,
    images_file: String,
    output_dir: Option<String>,
    platform: Option<String>,
    force: bool,
    insecure: bool,
) -> crate::registry::PullImagesOptions {
    crate::registry::PullImagesOptions {
        inventory_path: inventory,
        images_file,
        output_dir,
        platform,
        force,
        insecure: if insecure { Some(true) } else { None },
    }
}

async fn dispatch_push_images(
    tar_path: Option<String>,
    inventory: String,
    images_file: Option<String>,
    output_dir: Option<String>,
    retag: bool,
) -> Result<(), RtkkError> {
    let has_tar = tar_path.as_ref().is_some_and(|s| !s.trim().is_empty());
    let has_list = images_file.as_ref().is_some_and(|s| !s.trim().is_empty());
    match (has_tar, has_list) {
        (true, true) => Err(RtkkError::Registry(
            "push-images: pass either a tar path or --images-file, not both".to_string(),
        )),
        (false, false) => Err(RtkkError::Registry(
            "push-images: provide a tar path (single mode) or --images-file (batch mode)"
                .to_string(),
        )),
        (true, false) => {
            let Some(tar) = tar_path else {
                return Err(RtkkError::Registry(
                    "push-images: internal error: missing tar path".to_string(),
                ));
            };
            if output_dir.is_some() {
                return Err(RtkkError::Registry(
                    "push-images: --output-dir is only valid with --images-file (batch mode)"
                        .to_string(),
                ));
            }
            crate::registry::execute_push_images_with_retag(&tar, &inventory, retag)
                .await
                .map_err(RtkkError::Registry)
        }
        (false, true) => {
            let Some(list) = images_file else {
                return Err(RtkkError::Registry(
                    "push-images: internal error: missing images-file".to_string(),
                ));
            };
            let opts = crate::registry::PushImagesBatchOptions {
                inventory_path: inventory,
                images_file: list,
                output_dir,
                retag,
            };
            crate::registry::execute_push_images_batch(&opts)
                .await
                .map_err(RtkkError::Registry)
        }
    }
}

async fn dispatch_serve(
    listen: Option<String>,
    work_dir: Option<PathBuf>,
    token_file: Option<PathBuf>,
) -> Result<(), RtkkError> {
    let bind = listen
        .or_else(|| {
            std::env::var("RTKK_BIND_ADDR")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "127.0.0.1:19912".to_string());
    let wd = work_dir.unwrap_or_else(|| {
        crate::workspace::Workspace::from_env_or_current()
            .root()
            .to_path_buf()
    });
    crate::daemon::run_daemon(crate::daemon::ServeOptions {
        listen: bind,
        work_dir: wd,
        token_file,
    })
    .await
}

/// Execute a parsed [`Commands`] variant.
///
/// # Errors
///
/// Returns [`RtkkError`] if inventory loading, playbook execution, certificate generation,
/// node preparation, or registry synchronization fails.
pub async fn dispatch_command(command: Commands) -> Result<(), RtkkError> {
    match command {
        Commands::Run {
            playbook_path,
            inventory,
            set,
            forks,
            tags,
            task_timeout,
        } => {
            execute_run(
                &playbook_path,
                &inventory,
                &set,
                forks,
                tags.as_deref(),
                task_timeout,
            )
            .await
        }
        Commands::Prep {
            sub:
                PrepSubcommands::Hosts {
                    inventory,
                    best_effort,
                },
        } => {
            let loaded = load_inventory(&inventory).await?;
            crate::prep::execute_prep_hosts(
                &inventory,
                &loaded.flat,
                &loaded.target_configs,
                best_effort,
            )
            .await
            .map_err(RtkkError::Prep)
        }
        Commands::Cert { sub } => dispatch_cert(sub).await,
        Commands::PushImages {
            tar_path,
            inventory,
            images_file,
            output_dir,
            retag,
        } => dispatch_push_images(tar_path, inventory, images_file, output_dir, retag).await,
        Commands::PullImages {
            inventory,
            images_file,
            output_dir,
            platform,
            force,
            insecure,
        } => {
            let opts = pull_images_options(
                inventory,
                images_file,
                output_dir,
                platform,
                force,
                insecure,
            );
            crate::registry::execute_pull_images(&opts)
                .await
                .map_err(RtkkError::Registry)
        }
        Commands::SyncImages {
            inventory,
            images_file,
            output_dir,
            platform,
            force,
            insecure,
        } => {
            let opts = pull_images_options(
                inventory,
                images_file,
                output_dir,
                platform,
                force,
                insecure,
            );
            crate::registry::execute_sync_images(&opts)
                .await
                .map_err(RtkkError::Registry)
        }
        Commands::Serve {
            listen,
            work_dir,
            token_file,
        } => dispatch_serve(listen, work_dir, token_file).await,
        Commands::Healthcheck { url, timeout_secs } => {
            crate::daemon::execute_healthcheck(&url, timeout_secs).await
        }
    }
}

/// Parse CLI arguments from an explicit iterator and run the command.
///
/// # Errors
///
/// Returns a formatted error string if argument parsing, logger initialization,
/// or command execution fails.
pub async fn run_cli_from<I, T>(itr: I) -> Result<(), String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Cli::try_parse_from(itr).map_err(|e| e.to_string())?;
    crate::logging::Logger::init(args.log_file.as_deref())
        .map_err(RtkkError::Logging)
        .map_err(|e| e.to_string())?;
    dispatch_command(args.command)
        .await
        .map_err(|e| e.to_string())
}

/// Parse process command-line arguments (`std::env::args_os()`) and execute `rtkk`.
///
/// # Errors
///
/// Returns an error string if logger initialization or command execution fails.
pub async fn run_cli() -> Result<(), String> {
    let args = Cli::parse();
    crate::logging::Logger::init(args.log_file.as_deref())
        .map_err(RtkkError::Logging)
        .map_err(|e| e.to_string())?;
    dispatch_command(args.command)
        .await
        .map_err(|e| e.to_string())
}
