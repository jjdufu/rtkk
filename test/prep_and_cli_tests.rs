//! Integration tests for `rtkk` `CLI` argument parsing (`clap`), `X.509` `cert gen`/`verify`
//! subcommand dispatch, `-s msg=help` preservation, `--forks 0` deadlock prevention,
//! pre-connector `-s` `SSH`/privilege overrides, `push-images` `XOR` validation, and `prep` guards.

use clap::Parser as _;
use rtkk::cli::{
    CertSubcommands, Cli, Commands, PrepSubcommands, dispatch_command,
    load_inventory_with_overrides,
};
use rtkk::connector::{HostKeyPolicy, TargetConfig};
use rtkk::error::RtkkError;
use rtkk::inventory::FlatInventory;
use rtkk::prep::execute_prep_hosts;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
        let path = std::env::temp_dir().join(format!("rtkk-cli-test-{prefix}-{pid}-{nanos}"));
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

#[test]
fn cli_try_parse_from_covers_all_subcommands_and_flags() -> Result<(), String> {
    let run_cli = Cli::try_parse_from([
        "rtkk",
        "-l",
        "custom.log",
        "run",
        "configs/deploy.yaml",
        "-i",
        "configs/inventory.yaml",
        "-s",
        "k=v",
        "-f",
        "12",
        "--tags",
        "etcd,k8s",
        "--task-timeout",
        "90",
    ])
    .map_err(|e| e.to_string())?;
    ensure(
        run_cli.log_file.as_deref() == Some("custom.log"),
        "global -l log_file must parse",
    )?;
    ensure(
        matches!(
            run_cli.command,
            Commands::Run {
                forks: Some(12),
                task_timeout: Some(90),
                ..
            }
        ),
        "run subcommand flags must parse",
    )?;

    let prep_cli = Cli::try_parse_from([
        "rtkk",
        "prep",
        "hosts",
        "-i",
        "configs/inventory.yaml",
        "--best-effort",
    ])
    .map_err(|e| e.to_string())?;
    ensure(
        matches!(
            prep_cli.command,
            Commands::Prep {
                sub: PrepSubcommands::Hosts {
                    best_effort: true,
                    ..
                }
            }
        ),
        "prep hosts --best-effort must parse",
    )?;

    verify_registry_and_cert_subcommand_parsing()
}

fn verify_registry_and_cert_subcommand_parsing() -> Result<(), String> {
    let pull_cli = Cli::try_parse_from([
        "rtkk",
        "pull-images",
        "-i",
        "inv.yaml",
        "--images-file",
        "imgs.yaml",
        "--output-dir",
        "/tmp/out",
        "--platform",
        "linux/arm64",
        "--force",
        "--insecure",
    ])
    .map_err(|e| e.to_string())?;
    ensure(
        matches!(
            pull_cli.command,
            Commands::PullImages {
                force: true,
                insecure: true,
                ..
            }
        ),
        "pull-images flags must parse",
    )?;

    let sync_cli = Cli::try_parse_from([
        "rtkk",
        "sync-images",
        "-i",
        "inv.yaml",
        "--images-file",
        "imgs.yaml",
        "--force",
    ])
    .map_err(|e| e.to_string())?;
    ensure(
        matches!(sync_cli.command, Commands::SyncImages { force: true, .. }),
        "sync-images flags must parse",
    )?;

    let cert_cli = Cli::try_parse_from([
        "rtkk",
        "cert",
        "gen",
        "--cn",
        "kube-ca",
        "--out-key",
        "ca.key",
        "--out-cert",
        "ca.crt",
        "--ca",
    ])
    .map_err(|e| e.to_string())?;
    let Commands::Cert {
        sub:
            CertSubcommands::Gen {
                validity,
                policy,
                is_ca,
                ..
            },
    } = cert_cli.command
    else {
        return Err("expected Cert Gen command".to_string());
    };
    ensure(
        is_ca && validity == "867240h" && policy == "always",
        "cert gen default validity must be 867240h and policy must be always",
    )?;
    Ok(())
}

#[tokio::test]
async fn cert_gen_default_99y_and_1y_plus_cert_verify_end_to_end() -> Result<(), String> {
    let dir = TestDir::new("cert-e2e")?;
    let ca_key = dir.path.join("ca.key");
    let ca_crt = dir.path.join("ca.crt");
    let leaf_key = dir.path.join("apiserver.key");
    let leaf_crt = dir.path.join("apiserver.crt");

    let gen_ca = Cli::try_parse_from([
        "rtkk",
        "cert",
        "gen",
        "--cn",
        "cluster-root-ca",
        "--out-key",
        &ca_key.to_string_lossy(),
        "--out-cert",
        &ca_crt.to_string_lossy(),
        "--ca",
    ])
    .map_err(|e| e.to_string())?;
    dispatch_command(gen_ca.command)
        .await
        .map_err(|e| e.to_string())?;
    let ca_span = cert_validity_span_hours(&ca_crt)?;
    ensure(
        (867_240..=867_242).contains(&ca_span),
        &format!("default cert gen CA validity must be ~867240h, got {ca_span}h"),
    )?;

    let gen_leaf = Cli::try_parse_from([
        "rtkk",
        "cert",
        "gen",
        "--cn",
        "kube-apiserver",
        "--out-key",
        &leaf_key.to_string_lossy(),
        "--out-cert",
        &leaf_crt.to_string_lossy(),
        "--root-key",
        &ca_key.to_string_lossy(),
        "--root-cert",
        &ca_crt.to_string_lossy(),
        "--san",
        "127.0.0.1",
        "--san",
        "kubernetes.default.svc",
        "--validity",
        "1y",
    ])
    .map_err(|e| e.to_string())?;
    dispatch_command(gen_leaf.command)
        .await
        .map_err(|e| e.to_string())?;
    let leaf_span = cert_validity_span_hours(&leaf_crt)?;
    ensure(
        (8_760..=8_762).contains(&leaf_span),
        &format!("--validity 1y leaf span must be ~8760h, got {leaf_span}h"),
    )?;

    verify_dispatched_certs(&leaf_crt, &leaf_key, &ca_key, &ca_crt).await
}

async fn verify_dispatched_certs(
    leaf_crt: &Path,
    leaf_key: &Path,
    ca_key: &Path,
    ca_crt: &Path,
) -> Result<(), String> {
    let verify_ok = Cli::try_parse_from([
        "rtkk",
        "cert",
        "verify",
        &leaf_crt.to_string_lossy(),
        "--key",
        &leaf_key.to_string_lossy(),
        "--ca",
        &ca_crt.to_string_lossy(),
    ])
    .map_err(|e| e.to_string())?;
    dispatch_command(verify_ok.command)
        .await
        .map_err(|e| e.to_string())?;

    let verify_wrong_key = Cli::try_parse_from([
        "rtkk",
        "cert",
        "verify",
        &leaf_crt.to_string_lossy(),
        "--key",
        &ca_key.to_string_lossy(),
        "--ca",
        &ca_crt.to_string_lossy(),
    ])
    .map_err(|e| e.to_string())?;
    ensure(
        dispatch_command(verify_wrong_key.command).await.is_err(),
        "cert verify with mismatched private key must fail",
    )?;
    Ok(())
}

#[tokio::test]
async fn run_set_msg_help_is_not_corrupted_into_help_flag() -> Result<(), String> {
    let dir = TestDir::new("msg-help")?;
    let inv_path = dir.path.join("inv.yaml");
    let pb_path = dir.path.join("pb.yaml");
    let out_file = dir.path.join("msg.txt");

    std::fs::write(&inv_path, "hosts:\n  localhost: 127.0.0.1\n").map_err(|e| e.to_string())?;
    std::fs::write(
        &pb_path,
        format!(
            "- name: Write msg\n  hosts: localhost\n  tasks:\n    - copy:\n        content: \"{{{{ .msg }}}}\"\n        dest: \"{}\"\n",
            out_file.display()
        ),
    )
    .map_err(|e| e.to_string())?;

    let parsed = Cli::try_parse_from([
        "rtkk",
        "run",
        &pb_path.to_string_lossy(),
        "-i",
        &inv_path.to_string_lossy(),
        "-s",
        "msg=help",
    ])
    .map_err(|e| format!("`-s msg=help` was erroneously intercepted by clap: {e}"))?;

    let Commands::Run { ref set, .. } = parsed.command else {
        return Err("expected Commands::Run".to_string());
    };
    ensure(
        set == &vec![("msg".to_string(), "help".to_string())],
        &format!("expected set=[(\"msg\", \"help\")], got {set:?}"),
    )?;

    dispatch_command(parsed.command)
        .await
        .map_err(|e| e.to_string())?;
    let written = std::fs::read_to_string(&out_file).map_err(|e| e.to_string())?;
    ensure(
        written == "help",
        &format!("expected rendered file to equal 'help', got '{written}'"),
    )?;
    Ok(())
}

#[tokio::test]
async fn forks_zero_returns_clean_error_without_deadlocking() -> Result<(), String> {
    let dir = TestDir::new("forks-zero")?;
    let inv_path = dir.path.join("inv.yaml");
    let pb_path = dir.path.join("pb.yaml");
    std::fs::write(&inv_path, "hosts:\n  localhost: 127.0.0.1\n").map_err(|e| e.to_string())?;
    std::fs::write(
        &pb_path,
        "- name: Noop\n  hosts: localhost\n  tasks:\n    - command: echo hi\n",
    )
    .map_err(|e| e.to_string())?;

    let cli_zero = Cli::try_parse_from([
        "rtkk",
        "run",
        &pb_path.to_string_lossy(),
        "-i",
        &inv_path.to_string_lossy(),
        "--forks",
        "0",
    ])
    .map_err(|e| e.to_string())?;

    let outcome = tokio::time::timeout(Duration::from_secs(5), dispatch_command(cli_zero.command))
        .await
        .map_err(|_| "--forks 0 timed out / deadlocked".to_string())?;
    let Err(RtkkError::Playbook(msg)) = outcome else {
        return Err(format!(
            "expected RtkkError::Playbook on --forks 0, got {outcome:?}"
        ));
    };
    ensure(
        msg.contains("at least 1"),
        &format!("unexpected --forks 0 error message: {msg}"),
    )?;

    let inv_zero = dir.path.join("inv_forks0.yaml");
    std::fs::write(
        &inv_zero,
        "vars:\n  forks: 0\nhosts:\n  localhost: 127.0.0.1\n",
    )
    .map_err(|e| e.to_string())?;
    let cli_inv_zero = Cli::try_parse_from([
        "rtkk",
        "run",
        &pb_path.to_string_lossy(),
        "-i",
        &inv_zero.to_string_lossy(),
    ])
    .map_err(|e| e.to_string())?;
    ensure(
        dispatch_command(cli_inv_zero.command).await.is_err(),
        "inventory vars.forks=0 must also return Err cleanly",
    )?;
    Ok(())
}

#[tokio::test]
async fn set_overrides_apply_before_target_config_construction() -> Result<(), String> {
    let dir = TestDir::new("set-overrides")?;
    let inv_path = dir.path.join("inventory.yaml");
    std::fs::write(
        &inv_path,
        r#"vars:
  ansible_user: "root"
  ansible_port: 22
  ansible_become: false
hosts:
  worker-01: 10.0.0.71
roles:
  kube_node: [worker-01]
"#,
    )
    .map_err(|e| e.to_string())?;

    let overrides = vec![
        ("ansible_user".to_string(), "deploy".to_string()),
        ("ansible_port".to_string(), "2222".to_string()),
        ("ansible_become".to_string(), "true".to_string()),
        ("ansible_become_method".to_string(), "su".to_string()),
        ("ansible_become_user".to_string(), "secadmin".to_string()),
        ("ansible_become_pass".to_string(), "root-secret".to_string()),
        (
            "ansible_ssh_host_key_checking".to_string(),
            "accept-new".to_string(),
        ),
    ];
    let loaded = load_inventory_with_overrides(&inv_path.to_string_lossy(), &overrides)
        .await
        .map_err(|e| e.to_string())?;

    let tc = loaded
        .target_configs
        .get("worker-01")
        .ok_or("missing TargetConfig for worker-01")?;
    let ssh = tc.ssh.as_ref().ok_or("missing SshConfig for worker-01")?;
    ensure(
        ssh.username == "deploy",
        &format!("expected ssh.username='deploy', got '{}'", ssh.username),
    )?;
    ensure(
        ssh.port == Some(2222),
        &format!("expected ssh.port=Some(2222), got {:?}", ssh.port),
    )?;
    ensure(
        ssh.host_key_policy == HostKeyPolicy::AcceptNew,
        "expected HostKeyPolicy::AcceptNew from -s override",
    )?;

    let priv_cfg = tc
        .privilege
        .as_ref()
        .ok_or("expected PrivilegeConfig from -s ansible_become=true")?;
    ensure(
        priv_cfg.become_enabled
            && priv_cfg.become_method == rtkk::connector::BecomeMethod::Su
            && priv_cfg.become_user.as_deref() == Some("secadmin")
            && priv_cfg.become_password.as_deref() == Some("root-secret"),
        "expected become_enabled=true, become_method=Su, become_user=secadmin, become_password=root-secret",
    )?;

    let bad_overrides = vec![("ansible_become_method".to_string(), "doas".to_string())];
    ensure(
        load_inventory_with_overrides(&inv_path.to_string_lossy(), &bad_overrides)
            .await
            .is_err(),
        "unsupported ansible_become_method must be rejected during inventory loading",
    )?;
    Ok(())
}

#[tokio::test]
async fn push_images_xor_validation_and_prep_hosts_input_guards() -> Result<(), String> {
    let both = Commands::PushImages {
        tar_path: Some("a.tar".to_string()),
        inventory: "inv.yaml".to_string(),
        images_file: Some("images.yaml".to_string()),
        output_dir: None,
    };
    let Err(RtkkError::Registry(err_both)) = dispatch_command(both).await else {
        return Err("push-images with both tar and --images-file must fail".to_string());
    };
    ensure(
        err_both.contains("not both"),
        &format!("unexpected both-mode error: {err_both}"),
    )?;

    let neither = Commands::PushImages {
        tar_path: None,
        inventory: "inv.yaml".to_string(),
        images_file: None,
        output_dir: None,
    };
    let Err(RtkkError::Registry(err_neither)) = dispatch_command(neither).await else {
        return Err("push-images with neither tar nor --images-file must fail".to_string());
    };
    ensure(
        err_neither.contains("provide a tar path"),
        &format!("unexpected neither-mode error: {err_neither}"),
    )?;

    let out_with_single = Commands::PushImages {
        tar_path: Some("a.tar".to_string()),
        inventory: "inv.yaml".to_string(),
        images_file: None,
        output_dir: Some("/tmp/out".to_string()),
    };
    let Err(RtkkError::Registry(err_out)) = dispatch_command(out_with_single).await else {
        return Err("push-images --output-dir with single tar must fail".to_string());
    };
    ensure(
        err_out.contains("--output-dir is only valid with --images-file"),
        &format!("unexpected single-tar --output-dir error: {err_out}"),
    )?;

    let mut bad_host_vars = HashMap::new();
    let mut vars = HashMap::new();
    vars.insert("ip".to_string(), json!("10.0.0.1"));
    bad_host_vars.insert("bad host\nname".to_string(), vars);
    let bad_flat = FlatInventory {
        groups: HashMap::new(),
        host_vars: bad_host_vars,
    };
    ensure(
        execute_prep_hosts(
            "inv.yaml",
            &bad_flat,
            &HashMap::<String, TargetConfig>::new(),
            false,
        )
        .await
        .is_err(),
        "execute_prep_hosts must reject hostnames containing whitespace/newlines",
    )?;
    Ok(())
}
