//! Workspace directory layout, permission hardening, and idempotent embedded `configs/` bootstrap.

use crate::error::RtkkError;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

const EMBEDDED_CONFIG_FILES: &[(&str, &str)] = &[
    (
        "configs/cleanup.yaml",
        include_str!("../configs/cleanup.yaml"),
    ),
    (
        "configs/deploy.yaml",
        include_str!("../configs/deploy.yaml"),
    ),
    (
        "configs/download.yaml",
        include_str!("../configs/download.yaml"),
    ),
    (
        "configs/images.yaml",
        include_str!("../configs/images.yaml"),
    ),
    (
        "configs/inventory.yaml",
        include_str!("../configs/inventory.yaml"),
    ),
    (
        "configs/roles/addon-nodelocaldns/defaults/main.yaml",
        include_str!("../configs/roles/addon-nodelocaldns/defaults/main.yaml"),
    ),
    (
        "configs/roles/addon-nodelocaldns/tasks/main.yaml",
        include_str!("../configs/roles/addon-nodelocaldns/tasks/main.yaml"),
    ),
    (
        "configs/roles/addon-openebs/defaults/main.yaml",
        include_str!("../configs/roles/addon-openebs/defaults/main.yaml"),
    ),
    (
        "configs/roles/addon-openebs/tasks/main.yaml",
        include_str!("../configs/roles/addon-openebs/tasks/main.yaml"),
    ),
    (
        "configs/roles/cni-cilium/defaults/main.yaml",
        include_str!("../configs/roles/cni-cilium/defaults/main.yaml"),
    ),
    (
        "configs/roles/cni-cilium/tasks/main.yaml",
        include_str!("../configs/roles/cni-cilium/tasks/main.yaml"),
    ),
    (
        "configs/roles/custom-manifests/defaults/main.yaml",
        include_str!("../configs/roles/custom-manifests/defaults/main.yaml"),
    ),
    (
        "configs/roles/custom-manifests/tasks/main.yaml",
        include_str!("../configs/roles/custom-manifests/tasks/main.yaml"),
    ),
    (
        "configs/roles/debian13-preflight/defaults/main.yaml",
        include_str!("../configs/roles/debian13-preflight/defaults/main.yaml"),
    ),
    (
        "configs/roles/debian13-preflight/tasks/main.yaml",
        include_str!("../configs/roles/debian13-preflight/tasks/main.yaml"),
    ),
    (
        "configs/templates/cilium-values.yaml.gtmpl",
        include_str!("../configs/templates/cilium-values.yaml.gtmpl"),
    ),
    (
        "configs/templates/containerd-config.toml.gtmpl",
        include_str!("../configs/templates/containerd-config.toml.gtmpl"),
    ),
    (
        "configs/templates/etcd.service.gtmpl",
        include_str!("../configs/templates/etcd.service.gtmpl"),
    ),
    (
        "configs/templates/kubeadm-config.yaml.gtmpl",
        include_str!("../configs/templates/kubeadm-config.yaml.gtmpl"),
    ),
    (
        "configs/templates/local-volume.yaml.gtmpl",
        include_str!("../configs/templates/local-volume.yaml.gtmpl"),
    ),
    (
        "configs/templates/nodelocaldns.yaml.gtmpl",
        include_str!("../configs/templates/nodelocaldns.yaml.gtmpl"),
    ),
];

/// Canonical directory model for a `rtkk` CLI or Daemon working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Creates a [`Workspace`] rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolves a [`Workspace`] from `RTKK_WORK_DIR` if set and non-empty, otherwise `.`.
    #[must_use]
    pub fn from_env_or_current() -> Self {
        match std::env::var("RTKK_WORK_DIR") {
            Ok(val) if !val.trim().is_empty() => Self::new(PathBuf::from(val.trim())),
            _ => Self::new(PathBuf::from(".")),
        }
    }

    /// Returns the workspace root directory path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns `<work_dir>/configs`.
    #[must_use]
    pub fn configs_dir(&self) -> PathBuf {
        self.root.join("configs")
    }

    /// Returns `<work_dir>/configs/inventory.yaml`.
    #[must_use]
    pub fn inventory_path(&self) -> PathBuf {
        self.configs_dir().join("inventory.yaml")
    }

    /// Returns `<work_dir>/configs/images.yaml`.
    #[must_use]
    pub fn images_path(&self) -> PathBuf {
        self.configs_dir().join("images.yaml")
    }

    /// Returns `<work_dir>/downloads`.
    #[must_use]
    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }

    /// Returns `<work_dir>/downloads/bin`.
    #[must_use]
    pub fn downloads_bin_dir(&self) -> PathBuf {
        self.downloads_dir().join("bin")
    }

    /// Returns `<work_dir>/downloads/tar`.
    #[must_use]
    pub fn downloads_tar_dir(&self) -> PathBuf {
        self.downloads_dir().join("tar")
    }

    /// Returns `<work_dir>/downloads/images`.
    #[must_use]
    pub fn downloads_images_dir(&self) -> PathBuf {
        self.downloads_dir().join("images")
    }

    /// Returns `<work_dir>/pki` (`0700`).
    #[must_use]
    pub fn pki_dir(&self) -> PathBuf {
        self.root.join("pki")
    }

    /// Returns `<work_dir>/logs`.
    #[must_use]
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Returns `<work_dir>/logs/jobs`.
    #[must_use]
    pub fn logs_jobs_dir(&self) -> PathBuf {
        self.logs_dir().join("jobs")
    }

    /// Returns `<work_dir>/rtkk_bak` (`0700`).
    #[must_use]
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("rtkk_bak")
    }

    /// Returns `<work_dir>/.rtkk-daemon` (`0700`).
    #[must_use]
    pub fn daemon_state_dir(&self) -> PathBuf {
        self.root.join(".rtkk-daemon")
    }

    /// Returns `<work_dir>/.rtkk-daemon/admin.token` (`0600`).
    #[must_use]
    pub fn default_token_file(&self) -> PathBuf {
        self.daemon_state_dir().join("admin.token")
    }

    /// Ensures all standard subdirectories exist with hardened permissions and extracts
    /// missing default `configs/` templates without overwriting any existing user files.
    ///
    /// Returns `Ok(true)` if `configs/inventory.yaml` was newly created during this call,
    /// or `Ok(false)` if it was already present on disk.
    ///
    /// # Errors
    ///
    /// Returns [`RtkkError::Workspace`] if directory creation, permission enforcement, or
    /// atomic template file extraction fails.
    pub fn ensure_initialized(&self) -> Result<bool, RtkkError> {
        let public_dirs = [
            self.root.clone(),
            self.configs_dir(),
            self.downloads_bin_dir(),
            self.downloads_tar_dir(),
            self.downloads_images_dir(),
            self.logs_jobs_dir(),
        ];
        for dir in &public_dirs {
            std::fs::create_dir_all(dir).map_err(|e| {
                RtkkError::Workspace(format!("failed to create directory {}: {e}", dir.display()))
            })?;
        }

        let restricted_dirs = [self.pki_dir(), self.backups_dir(), self.daemon_state_dir()];
        for dir in &restricted_dirs {
            std::fs::create_dir_all(dir).map_err(|e| {
                RtkkError::Workspace(format!(
                    "failed to create restricted directory {}: {e}",
                    dir.display()
                ))
            })?;
            set_dir_mode_0700(dir)?;
        }

        let was_missing_inventory = !self.inventory_path().exists();
        for (rel_path, content) in EMBEDDED_CONFIG_FILES {
            let dest = self.root.join(rel_path);
            if dest.exists() {
                continue;
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    RtkkError::Workspace(format!(
                        "failed to create template parent directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }
            atomic_write_if_absent(&dest, content.as_bytes())?;
        }

        Ok(was_missing_inventory)
    }
}

#[cfg(unix)]
fn set_dir_mode_0700(path: &Path) -> Result<(), RtkkError> {
    let perms = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(path, perms).map_err(|e| {
        RtkkError::Workspace(format!(
            "failed to set 0700 mode on {}: {e}",
            path.display()
        ))
    })
}

#[cfg(not(unix))]
fn set_dir_mode_0700(_path: &Path) -> Result<(), RtkkError> {
    Ok(())
}

fn atomic_write_if_absent(dest: &Path, bytes: &[u8]) -> Result<(), RtkkError> {
    if dest.exists() {
        return Ok(());
    }
    let pid = std::process::id();
    let rand_suffix: u64 = rand::random();
    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("rtkk-config");
    let tmp_path = dest.with_file_name(format!(".{file_name}.tmp.{pid}.{rand_suffix:016x}"));
    std::fs::write(&tmp_path, bytes).map_err(|e| {
        RtkkError::Workspace(format!(
            "failed to write temporary file {}: {e}",
            tmp_path.display()
        ))
    })?;
    if dest.exists() {
        let _ = std::fs::remove_file(&tmp_path);
        return Ok(());
    }
    std::fs::rename(&tmp_path, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        RtkkError::Workspace(format!(
            "failed to rename {} to {}: {e}",
            tmp_path.display(),
            dest.display()
        ))
    })
}
