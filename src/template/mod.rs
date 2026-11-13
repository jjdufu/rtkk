//! `Go`-template rendering engine, value conversions, and global template registry.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

mod helpers;

pub use helpers::register_functions;

/// Returns a reference to the global named-template registry used by `include`.
#[must_use]
pub fn get_templates_registry() -> &'static RwLock<HashMap<String, String>> {
    static REGISTRY: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Registers a named template in the global registry so it can be rendered via `include`.
///
/// # Errors
///
/// Returns an error if the global template registry write lock is poisoned.
pub fn register_template(name: &str, content: &str) -> Result<(), String> {
    let mut guard = get_templates_registry()
        .write()
        .map_err(|e| format!("template registry write lock poisoned: {e}"))?;
    guard.insert(name.to_string(), content.to_string());
    Ok(())
}

/// Converts a `JSON` [`serde_json::Value`] into a [`gtmpl::Value`] context value.
#[must_use]
pub fn json_to_gtmpl(val: &serde_json::Value) -> gtmpl::Value {
    match val {
        serde_json::Value::Null => gtmpl::Value::Nil,
        serde_json::Value::Bool(b) => gtmpl::Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                gtmpl::Value::Number(gtmpl_value::Number::from(i))
            } else if let Some(u) = n.as_u64() {
                if let Ok(i) = i64::try_from(u) {
                    gtmpl::Value::Number(gtmpl_value::Number::from(i))
                } else {
                    let f = crate::utils::u64_to_f64(u);
                    gtmpl::Value::Number(gtmpl_value::Number::from(f))
                }
            } else if let Some(f) = n.as_f64() {
                gtmpl::Value::Number(gtmpl_value::Number::from(f))
            } else {
                gtmpl::Value::Nil
            }
        }
        serde_json::Value::String(s) => gtmpl::Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            let vec: Vec<gtmpl::Value> = arr.iter().map(json_to_gtmpl).collect();
            gtmpl::Value::Array(vec)
        }
        serde_json::Value::Object(obj) => {
            let mut map = HashMap::with_capacity(obj.len());
            for (k, v) in obj {
                map.insert(k.clone(), json_to_gtmpl(v));
            }
            gtmpl::Value::Map(map)
        }
    }
}

/// Converts a [`gtmpl::Value`] back into a `JSON` [`serde_json::Value`].
#[must_use]
pub fn gtmpl_to_json(val: &gtmpl::Value) -> serde_json::Value {
    match val {
        gtmpl::Value::Bool(b) => serde_json::Value::Bool(*b),
        gtmpl::Value::String(s) => serde_json::Value::String(s.clone()),
        gtmpl::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(serde_json::Number::from(i))
            } else if let Some(u) = n.as_u64() {
                serde_json::Value::Number(serde_json::Number::from(u))
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map_or(serde_json::Value::Null, serde_json::Value::Number)
            } else {
                serde_json::Value::Null
            }
        }
        gtmpl::Value::Array(arr) => {
            let vec: Vec<serde_json::Value> = arr.iter().map(gtmpl_to_json).collect();
            serde_json::Value::Array(vec)
        }
        gtmpl::Value::Map(obj) | gtmpl::Value::Object(obj) => {
            let mut map = serde_json::Map::new();
            for (k, v) in obj {
                map.insert(k.clone(), gtmpl_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        gtmpl::Value::Nil | gtmpl::Value::NoValue | gtmpl::Value::Function(_) => {
            serde_json::Value::Null
        }
    }
}

/// Renders a `Go` template string against the provided [`gtmpl::Value`] context.
///
/// # Errors
///
/// Returns an error if parsing the template syntax fails or executing any template
/// helper function fails.
pub fn render_template_string(
    template_content: &str,
    context: &gtmpl::Value,
) -> Result<String, String> {
    let mut t = gtmpl::Template::default();
    register_functions(&mut t);
    t.parse(template_content).map_err(|e| e.to_string())?;
    let gtmpl_ctx = gtmpl::Context::from(context.clone());
    let output = t.render(&gtmpl_ctx).map_err(|e| e.to_string())?;
    Ok(output.replace("<no value>", ""))
}

#[cfg(test)]
mod tests {
    use super::{json_to_gtmpl, register_template, render_template_string};

    #[test]
    fn missing_keys_work_in_default_empty_not_ternary_eq_ne() -> Result<(), String> {
        let ctx = json_to_gtmpl(&serde_json::json!({ "present": "ok" }));
        let d = render_template_string(r#"{{ default "false" .force_reinstall }}"#, &ctx)?;
        if d != "false" {
            return Err(format!("expected 'false', got '{d}'"));
        }
        let emp = render_template_string("{{ empty .force_reinstall }}", &ctx)?;
        if emp != "true" {
            return Err(format!("expected 'true', got '{emp}'"));
        }
        let neg = render_template_string("{{ not .force_reinstall }}", &ctx)?;
        if neg != "true" {
            return Err(format!("expected 'true', got '{neg}'"));
        }
        let tern = render_template_string(r#"{{ ternary "yes" "no" .force_reinstall }}"#, &ctx)?;
        if tern != "no" {
            return Err(format!("expected 'no', got '{tern}'"));
        }
        let eq_nil = render_template_string("{{ eq .missing_a .missing_b }}", &ctx)?;
        if eq_nil != "true" {
            return Err(format!("expected 'true', got '{eq_nil}'"));
        }
        let ne_present = render_template_string("{{ ne .present .missing_a }}", &ctx)?;
        if ne_present != "true" {
            return Err(format!("expected 'true', got '{ne_present}'"));
        }
        Ok(())
    }

    #[test]
    fn helpers_index_math_semver_b64enc_and_include() -> Result<(), String> {
        let ctx = json_to_gtmpl(&serde_json::json!({
            "items": ["alpha", "beta"],
            "map": {"k": "v"},
            "ver": "V1.29.2",
        }));
        if render_template_string("{{ index .items -1 }}", &ctx).is_ok() {
            return Err("expected negative index to fail".to_string());
        }
        if render_template_string("{{ index .items 5 }}", &ctx).is_ok() {
            return Err("expected out-of-bounds index to fail".to_string());
        }
        let missing_map = render_template_string(r#"{{ default "none" (index .map "x") }}"#, &ctx)?;
        if missing_map != "none" {
            return Err(format!("expected 'none', got '{missing_map}'"));
        }
        let parsed_int = render_template_string(r#"{{ add (int " 41.9 ") 1 }}"#, &ctx)?;
        if parsed_int != "42" {
            return Err(format!("expected '42', got '{parsed_int}'"));
        }
        let sem =
            render_template_string(r#"{{ semverCompare ">= v1.28.0, < v2.0.0" .ver }}"#, &ctx)?;
        if sem != "true" {
            return Err(format!("expected 'true', got '{sem}'"));
        }
        let b64 = render_template_string(r#"{{ b64enc "admin:secret" }}"#, &ctx)?;
        if b64 != "YWRtaW46c2VjcmV0" {
            return Err(format!("unexpected base64: '{b64}'"));
        }
        register_template("unit_test_ snippet.gtmpl", "hello {{ .ver }}")?;
        let inc = render_template_string(r#"{{ include "unit_test_ snippet.gtmpl" . }}"#, &ctx)?;
        if inc != "hello V1.29.2" {
            return Err(format!("unexpected include output: '{inc}'"));
        }
        Ok(())
    }
}
