use super::manifest::ManifestEntry;
use super::media::{MediaMeta, MEDIA_META_NAME};
use super::pull::PulledImage;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
pub(super) fn write_legacy_tar(pulled: &PulledImage, output_tar: &Path) -> Result<(), String> {
    if let Some(parent) = output_tar.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create output dir {}: {e}", parent.display()))?;
        }
    }
    let pid = std::process::id();
    let tmp_path = output_tar.with_extension(format!("tar.tmp.{pid}"));
    let _ = std::fs::remove_file(&tmp_path);
    write_legacy_tar_to_path(pulled, &tmp_path).inspect_err(|_e| {
        let _ = std::fs::remove_file(&tmp_path);
    })?;
    {
        let f =
            File::open(&tmp_path).map_err(|e| format!("failed to open tmp tar for fsync: {e}"))?;
        f.sync_all()
            .map_err(|e| format!("failed to fsync tmp tar: {e}"))?;
    }
    std::fs::rename(&tmp_path, output_tar).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        format!(
            "failed to rename {} -> {}: {e}",
            tmp_path.display(),
            output_tar.display()
        )
    })?;
    Ok(())
}
fn write_legacy_tar_to_path(pulled: &PulledImage, path: &Path) -> Result<(), String> {
    let config_hex = strip_sha256_prefix(&pulled.config_digest)?;
    let config_name = format!("{config_hex}.json");
    let mut layer_names = Vec::with_capacity(pulled.layers.len());
    let mut media = MediaMeta {
        config: Some(pulled.config_media_type.clone()),
        layers: HashMap::new(),
    };
    for layer in &pulled.layers {
        let hex = strip_sha256_prefix(&layer.digest)?;
        let name = format!("layers/{hex}/layer.tar");
        media.layers.insert(name.clone(), layer.media_type.clone());
        layer_names.push(name);
    }
    let entry = ManifestEntry {
        config: config_name.clone(),
        repo_tags: Some(vec![pulled.target_ref.clone()]),
        layers: layer_names.clone(),
    };
    let manifest_json = serde_json::to_vec(&vec![entry])
        .map_err(|e| format!("failed to serialize manifest.json: {e}"))?;
    let media_json =
        serde_json::to_vec(&media).map_err(|e| format!("failed to serialize rtkk-media.json: {e}"))?;
    let file =
        File::create(path).map_err(|e| format!("failed to create tar {}: {e}", path.display()))?;
    let mut builder = tar::Builder::new(BufWriter::new(file));
    append_bytes(&mut builder, "manifest.json", &manifest_json)?;
    append_bytes(&mut builder, MEDIA_META_NAME, &media_json)?;
    append_file_stream(&mut builder, &config_name, &pulled.config_path)?;
    for (layer, name) in pulled.layers.iter().zip(layer_names.iter()) {
        append_file_stream(&mut builder, name, &layer.path)?;
    }
    builder
        .finish()
        .map_err(|e| format!("failed to finish tar archive: {e}"))?;
    Ok(())
}
fn append_bytes<W: Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    data: &[u8],
) -> Result<(), String> {
    let mut header = tar::Header::new_gnu();
    header
        .set_path(name)
        .map_err(|e| format!("tar set_path {name}: {e}"))?;
    let size = u64::try_from(data.len()).map_err(|_| format!("blob too large for tar: {name}"))?;
    header.set_size(size);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append(&header, data)
        .map_err(|e| format!("tar append {name}: {e}"))?;
    Ok(())
}
fn append_file_stream<W: Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    path: &Path,
) -> Result<(), String> {
    let file = File::open(path)
        .map_err(|e| format!("failed to open blob {} for tar: {e}", path.display()))?;
    let meta = file
        .metadata()
        .map_err(|e| format!("failed to stat blob {}: {e}", path.display()))?;
    let mut header = tar::Header::new_gnu();
    header
        .set_path(name)
        .map_err(|e| format!("tar set_path {name}: {e}"))?;
    header.set_size(meta.len());
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append(&header, file)
        .map_err(|e| format!("tar append stream {name}: {e}"))?;
    Ok(())
}
fn strip_sha256_prefix(digest: &str) -> Result<String, String> {
    let hex = digest
        .strip_prefix("sha256:")
        .unwrap_or(digest)
        .to_ascii_lowercase();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("invalid sha256 digest: {digest}"));
    }
    Ok(hex)
}
pub(super) fn existing_tar_matches(tar_path: &Path, target_ref: &str) -> Result<bool, String> {
    if !tar_path.is_file() {
        return Ok(false);
    }
    let file = File::open(tar_path)
        .map_err(|e| format!("failed to open existing tar {}: {e}", tar_path.display()))?;
    let mut archive = tar::Archive::new(file);
    let mut manifest_bytes = None;
    let mut member_paths = std::collections::HashSet::new();
    for entry_res in archive
        .entries()
        .map_err(|e| format!("failed to read tar entries: {e}"))?
    {
        let mut entry = entry_res.map_err(|e| format!("failed to read tar entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("failed to get entry path: {e}"))?
            .to_string_lossy()
            .into_owned();
        if path == "manifest.json" {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf)
                .map_err(|e| format!("failed to read manifest.json: {e}"))?;
            manifest_bytes = Some(buf);
        } else {
            member_paths.insert(path);
        }
    }
    let Some(manifest_bytes) = manifest_bytes else {
        return Ok(false);
    };
    let entries: Vec<ManifestEntry> = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("failed to parse existing manifest.json: {e}"))?;
    for entry in &entries {
        let tags = entry
            .repo_tags
            .as_ref()
            .is_some_and(|t| t.iter().any(|tag| tag == target_ref));
        if !tags {
            continue;
        }
        if !member_paths.contains(&entry.config) {
            return Ok(false);
        }
        for layer in &entry.layers {
            if !member_paths.contains(layer) {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    Ok(false)
}
pub(super) fn image_tar_path(output_dir: &str, name: &str) -> PathBuf {
    Path::new(output_dir).join(format!("{name}.tar"))
}
