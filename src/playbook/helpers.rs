use crate::tasks::{Task, render_value};
use crate::utils::env_flag_enabled;
use serde_json::Value;
#[must_use]
pub(crate) fn resolve_play_hosts(
    hosts_pattern: &str,
    flat_inv: &crate::inventory::FlatInventory,
) -> Vec<String> {
    let trimmed = hosts_pattern.trim();
    if let Some(start_idx) = trimmed.find('[') {
        if let Some(end_idx) = trimmed.find(']') {
            if start_idx < end_idx {
                let group_name = trimmed.get(..start_idx).unwrap_or("").trim();
                let index_str = trimmed.get(start_idx + 1..end_idx).unwrap_or("").trim();
                if let Some(hosts) = flat_inv.groups.get(group_name) {
                    if let Ok(idx) = index_str.parse::<usize>() {
                        if let Some(h) = hosts.get(idx) {
                            return vec![h.clone()];
                        }
                    }
                }
                return Vec::new();
            }
        }
    }
    if let Some((group_name, "random")) = trimmed.split_once('|') {
        let group_name = group_name.trim();
        if let Some(hosts) = flat_inv.groups.get(group_name) {
            if !hosts.is_empty() {
                use rand::seq::SliceRandom as _;
                if let Some(h) = hosts.choose(&mut rand::thread_rng()) {
                    return vec![h.clone()];
                }
            }
        }
        return Vec::new();
    }
    if let Some(hosts) = flat_inv.groups.get(trimmed) {
        hosts.clone()
    } else if flat_inv.host_vars.contains_key(trimmed) {
        vec![trimmed.to_string()]
    } else {
        Vec::new()
    }
}
#[must_use]
pub(crate) fn partition_hosts(hosts: &[String], serial_val: Option<&Value>) -> Vec<Vec<String>> {
    let n = hosts.len();
    if n == 0 {
        return Vec::new();
    }
    let sizes = if let Some(val) = serial_val {
        let mut raw_sizes = Vec::new();
        match val {
            Value::Number(num) => {
                if let Some(i) = num.as_u64() {
                    raw_sizes.push(std::cmp::max(1, usize::try_from(i).unwrap_or(0)));
                }
            }
            Value::String(s) => {
                if s.ends_with('%') {
                    if let Ok(pct) = s.trim_end_matches('%').parse::<usize>() {
                        let sz = (n * pct).div_ceil(100);
                        raw_sizes.push(std::cmp::max(1, sz));
                    }
                } else if let Ok(i) = s.parse::<usize>() {
                    raw_sizes.push(std::cmp::max(1, i));
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    if let Some(i) = item.as_u64() {
                        raw_sizes.push(std::cmp::max(1, usize::try_from(i).unwrap_or(0)));
                    } else if let Some(s) = item.as_str() {
                        if s.ends_with('%') {
                            if let Ok(pct) = s.trim_end_matches('%').parse::<usize>() {
                                let sz = (n * pct).div_ceil(100);
                                raw_sizes.push(std::cmp::max(1, sz));
                            }
                        } else if let Ok(i) = s.parse::<usize>() {
                            raw_sizes.push(std::cmp::max(1, i));
                        }
                    }
                }
            }
            _ => {}
        }
        if raw_sizes.is_empty() {
            vec![n]
        } else {
            raw_sizes
        }
    } else {
        vec![n]
    };
    let mut batches = Vec::new();
    let mut cursor = 0;
    let mut size_idx = 0;
    while cursor < n {
        let sz = if size_idx < sizes.len() {
            sizes.get(size_idx).copied().unwrap_or(n)
        } else {
            *sizes.last().unwrap_or(&n)
        };
        let batch_end = std::cmp::min(n, cursor + sz);
        let segment = hosts.get(cursor..batch_end).unwrap_or(&[]).to_vec();
        batches.push(segment);
        cursor = batch_end;
        size_idx += 1;
    }
    batches
}
#[must_use]
pub(crate) fn task_matches_tags(task: &Task, filter: &[String]) -> bool {
    if filter.is_empty() {
        return true;
    }
    let Some(ref tags) = task.tags else {
        return false;
    };
    tags.iter().any(|t| filter.iter().any(|f| f == t))
}
#[must_use]
pub(crate) fn eval_condition(cond_val: &Value, host_vars: &Value) -> bool {
    eval_condition_detailed(cond_val, host_vars).unwrap_or_default()
}
pub(crate) fn eval_condition_detailed(cond_val: &Value, host_vars: &Value) -> Result<bool, String> {
    match cond_val {
        Value::Null => Ok(true),
        Value::String(s) => {
            if s.trim().is_empty() {
                return Ok(true);
            }
            let wrapped = crate::utils::wrap_condition_string(s);
            match crate::template::render_template_string(
                &wrapped,
                &crate::template::json_to_gtmpl(host_vars),
            ) {
                Ok(rendered) => {
                    let r = rendered.trim().to_lowercase();
                    Ok(r == "true" || r == "1" || r == "yes" || r == "on")
                }
                Err(e) => {
                    tracing::warn!("condition render failed: {e}");
                    if env_flag_enabled("RTKK_STRICT_WHEN") {
                        Err(format!("condition render failed: {e}"))
                    } else {
                        Ok(false)
                    }
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                if !eval_condition_detailed(item, host_vars)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Value::Bool(b) => Ok(*b),
        _ => Ok(false),
    }
}
pub(crate) fn get_loop_items(task: &Task, host_vars: &Value) -> Result<Option<Vec<Value>>, String> {
    if let Some(ref items_expr) = task.r#loop {
        resolve_loop_items_expr(items_expr, host_vars)
    } else if let Some(ref items_expr) = task.with_items {
        resolve_loop_items_expr(items_expr, host_vars)
    } else {
        Ok(None)
    }
}

/// Resolve `loop` / `with_items` to a list of values.
/// - YAML arrays: each element is template-rendered.
/// - Pure host-var reference strings such as `"{{ .public_registries }}"`:
///   use the inventory array as-is (gtmpl string render cannot preserve arrays).
fn resolve_loop_items_expr(
    items_expr: &Value,
    host_vars: &Value,
) -> Result<Option<Vec<Value>>, String> {
    if let Value::String(s) = items_expr {
        if let Some(arr) = host_var_array_from_template_ref(s, host_vars) {
            return Ok(Some(arr));
        }
    }
    let rendered = render_value(items_expr, &crate::template::json_to_gtmpl(host_vars))?;
    if let Value::Array(arr) = rendered {
        Ok(Some(arr))
    } else {
        Ok(Some(vec![rendered]))
    }
}

/// Match `{{ .name }}` / `{{.name}}` and return `host_vars.name` when it is an array.
fn host_var_array_from_template_ref(expr: &str, host_vars: &Value) -> Option<Vec<Value>> {
    let trimmed = expr.trim();
    let inner = trimmed
        .strip_prefix("{{")?
        .strip_suffix("}}")?
        .trim();
    let key = inner.strip_prefix('.')?.trim();
    if key.is_empty() || key.contains('.') || key.contains(' ') {
        return None;
    }
    let Value::Array(arr) = host_vars.get(key)? else {
        return None;
    };
    Some(arr.clone())
}
fn rand_factor_permille() -> u64 {
    use rand::Rng as _;
    rand::thread_rng().gen_range(0..1000)
}
pub(crate) fn calculate_backoff(
    attempt: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
) -> tokio::time::Duration {
    let exponent = attempt.min(10);
    let exponential_delay = base_delay_ms.saturating_mul(2u64.pow(exponent));
    let final_delay = exponential_delay.min(max_delay_ms);
    let jitter = final_delay
        .saturating_mul(3)
        .saturating_mul(rand_factor_permille())
        / 10000;
    tokio::time::Duration::from_millis(final_delay.saturating_add(jitter))
}
