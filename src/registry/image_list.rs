use super::RegistryConfig;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ImageSyncConfig {
    #[serde(default = "default_output_dir")]
    pub output_dir: String,
    #[serde(default = "default_platform")]
    pub platform: String,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default = "default_retries")]
    pub retries: u32,
    #[serde(default = "default_layer_concurrency")]
    pub layer_concurrency: usize,
    #[serde(default = "default_http_timeout_secs")]
    pub http_timeout_secs: u64,
    #[serde(default)]
    pub source_auths: HashMap<String, SourceAuth>,
}
impl Default for ImageSyncConfig {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            platform: default_platform(),
            insecure: false,
            retries: default_retries(),
            layer_concurrency: default_layer_concurrency(),
            http_timeout_secs: default_http_timeout_secs(),
            source_auths: HashMap::new(),
        }
    }
}
fn default_output_dir() -> String {
    "./downloads/images".to_string()
}
fn default_platform() -> String {
    "linux/amd64".to_string()
}
fn default_retries() -> u32 {
    3
}
fn default_layer_concurrency() -> usize {
    4
}
fn default_http_timeout_secs() -> u64 {
    600
}
#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct SourceAuth {
    pub username: Option<String>,
    pub password: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ImageSpec {
    pub name: String,
    pub source: String,
    pub target: String,
}
#[derive(Debug, Deserialize)]
struct ImagesFile {
    images: Vec<ImageSpecRaw>,
}
#[derive(Debug, Deserialize)]
struct ImageSpecRaw {
    name: String,
    source: String,
    target: String,
}
pub(crate) fn extract_vars_and_sync(doc: &Value) -> Result<(Value, ImageSyncConfig), String> {
    let vars = doc
        .get("vars")
        .or_else(|| doc.get("all").and_then(|all| all.get("vars")))
        .cloned()
        .ok_or_else(|| "missing 'vars' or 'all.vars' in inventory".to_string())?;
    let sync = vars
        .get("image_sync")
        .map(|v| {
            serde_json::from_value::<ImageSyncConfig>(v.clone())
                .map_err(|e| format!("failed to parse vars.image_sync: {e}"))
        })
        .transpose()?
        .unwrap_or_default();
    Ok((vars, sync))
}
pub(crate) fn registry_from_doc(doc: &Value) -> Result<RegistryConfig, String> {
    let registry_val = doc
        .get("vars")
        .and_then(|vars| vars.get("registry"))
        .or_else(|| {
            doc.get("all")
                .and_then(|all| all.get("vars"))
                .and_then(|vars| vars.get("registry"))
        })
        .ok_or_else(|| {
            "missing 'vars.registry' or 'all.vars.registry' section in inventory file".to_string()
        })?
        .clone();
    serde_json::from_value(registry_val)
        .map_err(|e| format!("failed to parse registry config: {e}"))
}
pub(crate) fn load_image_specs(images_file: &str, vars: &Value) -> Result<Vec<ImageSpec>, String> {
    let path = Path::new(images_file);
    if !path.is_file() {
        return Err(format!("images file not found: {images_file}"));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read images file {images_file}: {e}"))?;
    let ctx = crate::template::json_to_gtmpl(vars);
    let rendered = crate::template::render_template_string(&content, &ctx)
        .map_err(|e| format!("failed to render images file templates: {e}"))?;
    let file: ImagesFile = serde_yml::from_str(&rendered)
        .map_err(|e| format!("failed to parse images YAML after render: {e}"))?;
    if file.images.is_empty() {
        return Err("image list is empty; refusing to succeed with no work".to_string());
    }
    let mut out = Vec::with_capacity(file.images.len());
    for raw in file.images {
        let name = raw.name.trim().to_string();
        let source = raw.source.trim().to_string();
        let target = raw.target.trim().to_string();
        if name.is_empty() || source.is_empty() || target.is_empty() {
            return Err(format!(
                "invalid image entry: name/source/target must be non-empty (name={name:?})"
            ));
        }
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(format!("unsafe image name for tar path: {name}"));
        }
        out.push(ImageSpec {
            name,
            source,
            target,
        });
    }
    Ok(out)
}
pub(crate) fn parse_platform(platform: &str) -> Result<(String, String, Option<String>), String> {
    let parts: Vec<&str> = platform.split('/').collect();
    match parts.as_slice() {
        [os, arch] if !os.is_empty() && !arch.is_empty() => {
            Ok(((*os).to_string(), (*arch).to_string(), None))
        }
        [os, arch, variant] if !os.is_empty() && !arch.is_empty() && !variant.is_empty() => Ok((
            (*os).to_string(),
            (*arch).to_string(),
            Some((*variant).to_string()),
        )),
        _ => Err(format!(
            "invalid platform '{platform}', expected os/arch or os/arch/variant"
        )),
    }
}
