use std::sync::{OnceLock, RwLock};
mod helpers;
pub(crate) use helpers::register_functions;
#[must_use]
pub(crate) fn get_templates_registry() -> &'static RwLock<std::collections::HashMap<String, String>>
{
    static REGISTRY: OnceLock<RwLock<std::collections::HashMap<String, String>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(std::collections::HashMap::new()))
}
#[must_use]
pub(crate) fn json_to_gtmpl(val: &serde_json::Value) -> gtmpl::Value {
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
            let mut map = std::collections::HashMap::new();
            for (k, v) in obj {
                map.insert(k.clone(), json_to_gtmpl(v));
            }
            gtmpl::Value::Map(map)
        }
    }
}
#[must_use]
pub(crate) fn gtmpl_to_json(val: &gtmpl::Value) -> serde_json::Value {
    match val {
        gtmpl::Value::Bool(b) => serde_json::Value::Bool(*b),
        gtmpl::Value::String(s) => serde_json::Value::String(s.clone()),
        gtmpl::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(serde_json::Number::from(i))
            } else if let Some(f) = n.as_f64() {
                if let Some(num) = serde_json::Number::from_f64(f) {
                    serde_json::Value::Number(num)
                } else {
                    serde_json::Value::Null
                }
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
pub(crate) fn render_template_string(
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
