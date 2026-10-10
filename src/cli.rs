use crate::connector::{ConnectorType, SshConfig, TargetConfig};
use crate::error::RtkkError;
use crate::inventory::{FlatInventory, Inventory};
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[derive(Parser, Debug)]
#[command(
    name = "rtkk",
    version = env!("CARGO_PKG_VERSION"),
    about = "rtkk CLI tool",
    override_usage = "rtkk [OPTIONS] <COMMAND>"
)]
struct Cli {
    #[arg(
        long,
        short = 'l',
        global = true,
        help = "Write all execution outputs and logs to the specified file directly."
    )]
    log_file: Option<String>,
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand, Debug)]
enum Commands {
    #[command(about = "Run Playbook")]
    Run {
        playbook_path: String,
        #[arg(
            long,
            short = 'i',
            required = true,
            help = "Path to the inventory file"
        )]
        inventory: String,
        #[arg(long, short = 's', help = "Set extra variable (key=val)", value_parser = parse_key_value)]
        set: Vec<(String, String)>,
        #[arg(long, short = 'f', help = "Concurrency limit (forks)")]
        forks: Option<usize>,
        #[arg(
            long,
            help = "Only run tasks with these tags (comma-separated). Empty = all tasks."
        )]
        tags: Option<String>,
        #[arg(
            long,
            help = "Global per-task timeout in seconds (0 or omitted = unlimited)"
        )]
        task_timeout: Option<u64>,
    },
    #[command(about = "Prepare target nodes")]
    Prep {
        #[command(subcommand)]
        sub: PrepSubcommands,
    },
    #[command(
        about = "Push OCI images to registry (single tar XOR list batch)",
        after_help = "Two mutually exclusive modes:\n  \
            Single tar:  rtkk push-images <TAR> -i <inventory>\n  \
            List batch:  rtkk push-images -i <inventory> --images-file <images.yaml> [--output-dir DIR]\n  \
            Do not pass both a tar path and --images-file."
    )]
    PushImages {
        #[arg(help = "Single docker-legacy tar path (omit when using --images-file)")]
        tar_path: Option<String>,
        #[arg(
            long,
            short = 'i',
            required = true,
            help = "Path to the inventory file"
        )]
        inventory: String,
        #[arg(
            long,
            help = "Images list YAML for batch push (mutually exclusive with tar path)"
        )]
        images_file: Option<String>,
        #[arg(
            long,
            help = "Tar directory for batch mode (default: vars.image_sync.output_dir)"
        )]
        output_dir: Option<String>,
    },
    #[command(about = "Pull OCI images into local docker-legacy tarballs (pure Rust)")]
    PullImages {
        #[arg(
            long,
            short = 'i',
            required = true,
            help = "Path to the inventory file"
        )]
        inventory: String,
        #[arg(
            long,
            required = true,
            help = "Path to images list YAML (templates rendered with inventory vars)"
        )]
        images_file: String,
        #[arg(
            long,
            help = "Output directory for *.tar (default: vars.image_sync.output_dir)"
        )]
        output_dir: Option<String>,
        #[arg(
            long,
            help = "Platform os/arch (default: vars.image_sync.platform or linux/amd64)"
        )]
        platform: Option<String>,
        #[arg(long, help = "Overwrite existing tar files")]
        force: bool,
        #[arg(long, help = "Accept invalid TLS certificates for pull")]
        insecure: bool,
    },
    #[command(about = "Pull images to local tar then push to private registry")]
    SyncImages {
        #[arg(
            long,
            short = 'i',
            required = true,
            help = "Path to the inventory file"
        )]
        inventory: String,
        #[arg(
            long,
            required = true,
            help = "Path to images list YAML (templates rendered with inventory vars)"
        )]
        images_file: String,
        #[arg(
            long,
            help = "Output directory for *.tar (default: vars.image_sync.output_dir)"
        )]
        output_dir: Option<String>,
        #[arg(
            long,
            help = "Platform os/arch (default: vars.image_sync.platform or linux/amd64)"
        )]
        platform: Option<String>,
        #[arg(long, help = "Overwrite existing tar files")]
        force: bool,
        #[arg(long, help = "Accept invalid TLS certificates for pull")]
        insecure: bool,
    },
}
#[derive(Subcommand, Debug)]
enum PrepSubcommands {
    #[command(about = "Update hosts mapping on all target nodes")]
    Hosts {
        #[arg(
            long,
            short = 'i',
            required = true,
            help = "Path to the inventory file"
        )]
        inventory: String,
        #[arg(
            long,
            help = "Continue when some hosts fail hostname resolution (default: fail-fast)"
        )]
        best_effort: bool,
    },
}
fn parse_key_value(s: &str) -> Result<(String, String), String> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("invalid key=value: '{s}'"))
}
pub(crate) fn clean_ansi_escapes(s: &str) -> String {
    static RE: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])").ok()
    });
    let cleaned = if let Some(re) = re.as_ref() {
        re.replace_all(s, "").into_owned()
    } else {
        s.to_string()
    };
    cleaned.replace("[RTKK_SUDO_PROMPT]", "")
}
pub(crate) struct LoadedInventory {
    pub inventory: Inventory,
    pub flat: FlatInventory,
    pub target_configs: HashMap<String, TargetConfig>,
    pub inventory_dir: Option<PathBuf>,
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
        .map(String::from);
    let port = vars
        .get("ansible_port")
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok());
    let private_key_fallback_password = vars
        .get("ansible_ssh_private_key_fallback_password")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(SshConfig {
        username: user.to_string(),
        port,
        private_key_content: None,
        private_key,
        password,
        private_key_fallback_password,
    })
}
fn connector_type_from_vars(vars: &HashMap<String, Value>) -> Option<ConnectorType> {
    let raw = vars
        .get("ansible_connection")
        .and_then(Value::as_str)
        .or_else(|| vars.get("connector_type").and_then(Value::as_str))?;
    match raw.to_ascii_lowercase().as_str() {
        "local" => Some(ConnectorType::Local),
        "ssh" => Some(ConnectorType::Ssh),
        _ => None,
    }
}
fn build_target_configs(flat: &FlatInventory) -> HashMap<String, TargetConfig> {
    let mut target_configs = HashMap::new();
    for (host, vars) in &flat.host_vars {
        if host == "localhost" {
            target_configs.insert(
                host.clone(),
                TargetConfig {
                    host: "127.0.0.1".to_string(),
                    connector_type: Some(ConnectorType::Local),
                    ssh: None,
                },
            );
            continue;
        }
        let mut host_ip = host.clone();
        if let Some(ip) = vars.get("ip").and_then(Value::as_str) {
            host_ip = ip.to_string();
        }
        target_configs.insert(
            host.clone(),
            TargetConfig {
                host: host_ip,
                connector_type: connector_type_from_vars(vars),
                ssh: ssh_config_from_vars(vars),
            },
        );
    }
    target_configs
}
pub(crate) async fn load_inventory(inventory_path: &str) -> Result<LoadedInventory, RtkkError> {
    let inventory_content = tokio::fs::read_to_string(inventory_path)
        .await
        .map_err(|e| {
            RtkkError::Inventory(format!("failed to read inventory {inventory_path}: {e}"))
        })?;
    let inventory: Inventory = serde_yml::from_str(&inventory_content)
        .map_err(|e| RtkkError::Inventory(format!("failed to parse inventory: {e}")))?;
    let mut flat = crate::inventory::resolve_inventory(&inventory).map_err(RtkkError::Inventory)?;
    crate::playbook::inject_dynamic_inventory_vars(&flat.groups, &mut flat.host_vars);
    let target_configs = build_target_configs(&flat);
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
fn forks_from_inventory(inventory: &Inventory) -> Option<usize> {
    if let Some(ref all) = inventory.all {
        if let Some(ref vars) = all.vars {
            if let Some(f_val) = vars.get("forks") {
                if let Some(f_str) = f_val.as_str() {
                    return f_str.parse::<usize>().ok();
                } else if let Some(f_u64) = f_val.as_u64() {
                    return usize::try_from(f_u64).ok();
                }
            }
        }
    }
    if let Some(ref vars) = inventory.vars {
        if let Some(f_val) = vars.get("forks") {
            if let Some(f_str) = f_val.as_str() {
                return f_str.parse::<usize>().ok();
            } else if let Some(f_u64) = f_val.as_u64() {
                return usize::try_from(f_u64).ok();
            }
        }
    }
    None
}
async fn execute_run(
    playbook_path: &str,
    inventory_path: &str,
    set_vars: &[(String, String)],
    forks: Option<usize>,
    tags: Option<&str>,
    task_timeout: Option<u64>,
) -> Result<(), RtkkError> {
    let mut extra_vars = HashMap::new();
    for (k, v) in set_vars {
        extra_vars.insert(k.clone(), Value::String(v.clone()));
    }
    let loaded = load_inventory(inventory_path).await?;
    let forks_count = forks
        .or_else(|| forks_from_inventory(&loaded.inventory))
        .unwrap_or(50);
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
    if let Some(t) = task_timeout {
        if t > 0 {
            engine.task_timeout_secs = Some(t);
        }
    }
    let engine = Arc::new(engine);
    if !extra_vars.is_empty() {
        let mut bus = engine.host_vars_bus.lock().await;
        for h_val in bus.values_mut() {
            if let Some(obj) = h_val.as_object_mut() {
                for (k, v) in &extra_vars {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
    }
    engine
        .run_playbook(&playbook)
        .await
        .map_err(RtkkError::Task)?;
    crate::log_println!("Playbook executed successfully.");
    Ok(())
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
            crate::registry::execute_push_images(&tar, &inventory)
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
            };
            crate::registry::execute_push_images_batch(&opts)
                .await
                .map_err(RtkkError::Registry)
        }
    }
}
async fn dispatch_command(command: Commands) -> Result<(), RtkkError> {
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
        } => match load_inventory(&inventory).await {
            Ok(loaded) => crate::prep::execute_prep_hosts(
                &inventory,
                &loaded.flat,
                &loaded.target_configs,
                best_effort,
            )
            .await
            .map_err(RtkkError::Prep),
            Err(e) => Err(e),
        },
        Commands::PushImages {
            tar_path,
            inventory,
            images_file,
            output_dir,
        } => dispatch_push_images(tar_path, inventory, images_file, output_dir).await,
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
    }
}
pub async fn run_cli() -> Result<(), String> {
    let mut env_args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if env_args.len() > 2 {
        if let Some(last) = env_args.last_mut() {
            if last
                .to_str()
                .is_some_and(|s| s.eq_ignore_ascii_case("help"))
            {
                *last = std::ffi::OsString::from("--help");
            }
        }
    }
    let args = Cli::parse_from(env_args);
    crate::logging::Logger::init(args.log_file.as_deref())
        .map_err(RtkkError::Logging)
        .map_err(|e| e.to_string())?;
    dispatch_command(args.command)
        .await
        .map_err(|e| e.to_string())
}
