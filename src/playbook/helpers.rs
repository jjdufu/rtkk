//! Playbook host resolution, batch partitioning, tag filtering, condition evaluation, and loop register helpers.

use crate::tasks::{Task, TaskResult, render_value};
use rand::Rng as _;
use rand::seq::SliceRandom as _;
use serde_json::Value;

fn resolve_single_host_pattern(
    pattern: &str,
    flat_inv: &crate::inventory::FlatInventory,
) -> Result<Vec<String>, String> {
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return Err("empty target host pattern".to_string());
    }
    if let Some(start_idx) = trimmed.find('[')
        && let Some(end_idx) = trimmed.find(']')
        && start_idx < end_idx
    {
        let group_name = trimmed.get(..start_idx).unwrap_or("").trim();
        let index_str = trimmed.get(start_idx + 1..end_idx).unwrap_or("").trim();
        let idx = index_str
            .parse::<usize>()
            .map_err(|_| format!("invalid host group index in pattern '{trimmed}'"))?;
        if let Some(hosts) = flat_inv.groups.get(group_name) {
            return Ok(hosts.get(idx).cloned().into_iter().collect());
        }
        return Err(format!("no hosts matched unknown target '{trimmed}'"));
    }
    if let Some((group_name, "random")) = trimmed.split_once('|') {
        let group_name = group_name.trim();
        if let Some(hosts) = flat_inv.groups.get(group_name) {
            return Ok(hosts
                .choose(&mut rand::thread_rng())
                .cloned()
                .into_iter()
                .collect());
        }
        return Err(format!("no hosts matched unknown target '{trimmed}'"));
    }
    if trimmed == "localhost" {
        return Ok(vec!["localhost".to_string()]);
    }
    if let Some(hosts) = flat_inv.groups.get(trimmed) {
        return Ok(hosts.clone());
    }
    if trimmed == "all" {
        let mut all_hosts: Vec<String> = flat_inv.host_vars.keys().cloned().collect();
        all_hosts.sort();
        return Ok(all_hosts);
    }
    if flat_inv.host_vars.contains_key(trimmed) {
        return Ok(vec![trimmed.to_string()]);
    }
    Err(format!("no hosts matched unknown target '{trimmed}'"))
}

pub(crate) fn resolve_play_hosts(
    hosts_pattern: &str,
    flat_inv: &crate::inventory::FlatInventory,
) -> Result<Vec<String>, String> {
    let raw = hosts_pattern.trim();
    if raw.is_empty() {
        return Err("play hosts target cannot be empty".to_string());
    }
    let mut out = Vec::new();
    for segment in raw.split(',') {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        let resolved = resolve_single_host_pattern(seg, flat_inv)?;
        for h in resolved {
            if !out.iter().any(|existing| existing == &h) {
                out.push(h);
            }
        }
    }
    Ok(out)
}

fn parse_serial_entry(val: &Value, total_hosts: usize) -> Result<usize, String> {
    match val {
        Value::Number(num) => {
            if let Some(u) = num.as_u64() {
                if u == 0 {
                    return Err("play serial batch size must be greater than 0".to_string());
                }
                usize::try_from(u).map_err(|_| format!("serial batch size overflow: {u}"))
            } else {
                Err(format!(
                    "play serial batch size must be a positive integer, got {num}"
                ))
            }
        }
        Value::String(s) => {
            let trimmed = s.trim();
            if let Some(pct_str) = trimmed.strip_suffix('%') {
                let pct = pct_str
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| format!("invalid serial percentage '{s}'"))?;
                if pct == 0 {
                    return Err("play serial percentage must be greater than 0%".to_string());
                }
                Ok(std::cmp::max(
                    1,
                    total_hosts.saturating_mul(pct).div_ceil(100),
                ))
            } else {
                let sz = trimmed
                    .parse::<usize>()
                    .map_err(|_| format!("invalid serial batch size '{s}'"))?;
                if sz == 0 {
                    return Err("play serial batch size must be greater than 0".to_string());
                }
                Ok(sz)
            }
        }
        other => Err(format!("unsupported serial specification: {other}")),
    }
}

pub(crate) fn partition_hosts(
    hosts: &[String],
    serial_val: Option<&Value>,
) -> Result<Vec<Vec<String>>, String> {
    let n = hosts.len();
    let sizes = match serial_val {
        None | Some(Value::Null) => vec![n.max(1)],
        Some(Value::Array(arr)) => {
            if arr.is_empty() {
                return Err("play serial array cannot be empty".to_string());
            }
            let mut parsed = Vec::with_capacity(arr.len());
            for item in arr {
                parsed.push(parse_serial_entry(item, n)?);
            }
            parsed
        }
        Some(single) => vec![parse_serial_entry(single, n)?],
    };
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut batches = Vec::new();
    let mut cursor = 0;
    let mut size_idx = 0;
    while cursor < n {
        let sz = if size_idx < sizes.len() {
            sizes.get(size_idx).copied().unwrap_or(n)
        } else {
            *sizes.last().unwrap_or(&n)
        };
        let batch_end = std::cmp::min(n, cursor.saturating_add(sz));
        let segment = hosts.get(cursor..batch_end).unwrap_or(&[]).to_vec();
        batches.push(segment);
        cursor = batch_end;
        size_idx += 1;
    }
    Ok(batches)
}

#[must_use]
pub(crate) fn task_matches_tags_direct(task: &Task, filter: &[String]) -> bool {
    if filter.is_empty() {
        return true;
    }
    let Some(ref tags) = task.tags else {
        return false;
    };
    tags.iter().any(|t| filter.iter().any(|f| f == t))
}

#[must_use]
pub(crate) fn task_matches_tags(task: &Task, filter: &[String]) -> bool {
    if task_matches_tags_direct(task, filter) {
        return true;
    }
    let in_block = task
        .block
        .as_ref()
        .is_some_and(|b| b.iter().any(|child| task_matches_tags(child, filter)));
    let in_rescue = task
        .rescue
        .as_ref()
        .is_some_and(|r| r.iter().any(|child| task_matches_tags(child, filter)));
    let in_always = task
        .always
        .as_ref()
        .is_some_and(|a| a.iter().any(|child| task_matches_tags(child, filter)));
    in_block || in_rescue || in_always
}

/// Evaluates a task condition (`when`, `failed_when`, or `changed_when`) against `host_vars`.
///
/// # Errors
///
/// Returns an error if template parsing or rendering fails, or if the rendered string is not a
/// recognized boolean value (`"true"`, `"false"`, `"1"`, `"0"`, `"yes"`, `"no"`, `"on"`, `"off"`, or `""`).
pub fn eval_condition(cond_val: &Value, host_vars: &Value) -> Result<bool, String> {
    match cond_val {
        Value::Null => Ok(true),
        Value::Bool(b) => Ok(*b),
        Value::Number(n) => {
            if n.as_i64() == Some(0) || n.as_u64() == Some(0) {
                Ok(false)
            } else if n.as_i64() == Some(1) || n.as_u64() == Some(1) {
                Ok(true)
            } else {
                Err(format!("condition evaluated to non-boolean number '{n}'"))
            }
        }
        Value::Array(arr) => {
            for item in arr {
                if !eval_condition(item, host_vars)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Value::String(s) => {
            let raw_trimmed = s.trim();
            if !raw_trimmed.contains("{{") {
                match raw_trimmed.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" | "on" => return Ok(true),
                    "false" | "0" | "no" | "off" | "" => return Ok(false),
                    _ => {}
                }
            }
            let wrapped = crate::utils::wrap_condition_string(s);
            let rendered = crate::template::render_template_string(
                &wrapped,
                &crate::template::json_to_gtmpl(host_vars),
            )
            .map_err(|e| format!("condition render failed for '{s}': {e}"))?;
            let trimmed = rendered.trim();
            match trimmed.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Ok(true),
                "false" | "0" | "no" | "off" | "" | "<no value>" | "nil" | "null" => Ok(false),
                _ => Err(format!(
                    "condition '{s}' evaluated to non-boolean value '{trimmed}'"
                )),
            }
        }
        Value::Object(_) => Err("condition cannot be a JSON/YAML object".to_string()),
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

fn resolve_loop_items_expr(
    items_expr: &Value,
    host_vars: &Value,
) -> Result<Option<Vec<Value>>, String> {
    if let Value::String(s) = items_expr
        && let Some(arr) = host_var_array_from_template_ref(s, host_vars)
    {
        return Ok(Some(arr));
    }
    let rendered = render_value(items_expr, &crate::template::json_to_gtmpl(host_vars))?;
    if let Value::Array(arr) = rendered {
        Ok(Some(arr))
    } else {
        Ok(Some(vec![rendered]))
    }
}

fn host_var_array_from_template_ref(expr: &str, host_vars: &Value) -> Option<Vec<Value>> {
    let trimmed = expr.trim();
    let inner = trimmed.strip_prefix("{{")?.strip_suffix("}}")?.trim();
    let key = inner.strip_prefix('.')?.trim();
    if key.is_empty() || key.contains('.') || key.contains(' ') {
        return None;
    }
    let Value::Array(arr) = host_vars.get(key)? else {
        return None;
    };
    Some(arr.clone())
}

#[derive(Debug, Clone)]
pub(crate) struct LoopItemOutcome {
    pub item: Value,
    pub result: TaskResult,
    pub skipped: bool,
}

#[must_use]
pub(crate) fn parse_register_stdout(stdout: &str, register_type: Option<&str>) -> Value {
    let clean = stdout.trim_end_matches('\n');
    match register_type {
        Some("json") => {
            serde_json::from_str(stdout).unwrap_or_else(|_| Value::String(clean.to_string()))
        }
        Some("yaml") => {
            serde_yml::from_str(stdout).unwrap_or_else(|_| Value::String(clean.to_string()))
        }
        _ => Value::String(clean.to_string()),
    }
}

#[must_use]
pub(crate) fn build_single_register_value(res: &TaskResult, register_type: Option<&str>) -> Value {
    let parsed_stdout = parse_register_stdout(&res.stdout, register_type);
    serde_json::json!({
        "stdout": parsed_stdout,
        "stderr": res.stderr,
        "rc": res.exit_code,
        "changed": res.changed,
        "failed": res.failed,
        "error": if res.failed { Value::String(res.stderr.clone()) } else { Value::Null }
    })
}

#[must_use]
pub(crate) fn build_loop_register_value(
    outcomes: &[LoopItemOutcome],
    register_type: Option<&str>,
) -> Value {
    let mut results_arr = Vec::with_capacity(outcomes.len());
    let mut last_stdout = Value::String(String::new());
    let mut last_stderr = String::new();
    let mut last_rc: i32 = 0;
    let mut any_changed = false;
    let mut any_failed = false;
    for outcome in outcomes {
        let parsed_stdout = parse_register_stdout(&outcome.result.stdout, register_type);
        if !outcome.skipped {
            last_stdout = parsed_stdout.clone();
            last_stderr.clone_from(&outcome.result.stderr);
            last_rc = outcome.result.exit_code;
        }
        if outcome.result.changed {
            any_changed = true;
        }
        if outcome.result.failed {
            any_failed = true;
        }
        results_arr.push(serde_json::json!({
            "item": outcome.item,
            "skipped": outcome.skipped,
            "stdout": parsed_stdout,
            "stderr": outcome.result.stderr,
            "rc": outcome.result.exit_code,
            "changed": outcome.result.changed,
            "failed": outcome.result.failed,
            "error": if outcome.result.failed {
                Value::String(outcome.result.stderr.clone())
            } else {
                Value::Null
            }
        }));
    }
    serde_json::json!({
        "results": results_arr,
        "stdout": last_stdout,
        "stderr": last_stderr,
        "rc": last_rc,
        "changed": any_changed,
        "failed": any_failed,
        "error": if any_failed { Value::String(last_stderr) } else { Value::Null }
    })
}

fn rand_factor_permille() -> u64 {
    rand::thread_rng().gen_range(0..1000)
}

#[must_use]
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
        / 10_000;
    tokio::time::Duration::from_millis(final_delay.saturating_add(jitter))
}

#[cfg(test)]
mod tests {
    use super::{LoopItemOutcome, build_loop_register_value, eval_condition, partition_hosts};
    use crate::tasks::TaskResult;
    use serde_json::json;

    #[test]
    fn eval_condition_strict_errors_and_booleans() -> Result<(), String> {
        let vars = json!({ "flag": true, "answer": "yes", "disabled": "no" });
        if !eval_condition(&json!("{{ .flag }}"), &vars)? {
            return Err("expected .flag to be true".to_string());
        }
        if !eval_condition(&json!("{{ .answer }}"), &vars)? {
            return Err("expected 'yes' to be true".to_string());
        }
        if eval_condition(&json!("{{ .disabled }}"), &vars)? {
            return Err("expected 'no' to be false".to_string());
        }
        if eval_condition(&json!("{{ .missing }}"), &vars)? {
            return Err("expected missing var to be false".to_string());
        }
        if eval_condition(&json!("true == false"), &vars).is_ok() {
            return Err("expected unparsed operator expression to fail".to_string());
        }
        if eval_condition(&json!("{{ unknown_fn .flag }}"), &vars).is_ok() {
            return Err("expected broken template function to fail".to_string());
        }
        Ok(())
    }

    #[test]
    fn serial_zero_fails_and_loop_register_preserves_results() -> Result<(), String> {
        let hosts = vec!["h1".to_string(), "h2".to_string()];
        if partition_hosts(&hosts, Some(&json!(0))).is_ok() {
            return Err("expected serial: 0 to fail".to_string());
        }
        let outcomes = vec![
            LoopItemOutcome {
                item: json!("first"),
                result: TaskResult {
                    stdout: "out1\n".to_string(),
                    stderr: String::new(),
                    exit_code: 0,
                    changed: true,
                    failed: false,
                },
                skipped: false,
            },
            LoopItemOutcome {
                item: json!("second"),
                result: TaskResult {
                    stdout: "out2\n".to_string(),
                    stderr: String::new(),
                    exit_code: 0,
                    changed: false,
                    failed: false,
                },
                skipped: false,
            },
        ];
        let reg = build_loop_register_value(&outcomes, None);
        if reg.get("stdout").and_then(serde_json::Value::as_str) != Some("out2") {
            return Err(format!("expected last stdout 'out2', got {reg}"));
        }
        if reg.get("changed").and_then(serde_json::Value::as_bool) != Some(true) {
            return Err(format!("expected any_changed=true, got {reg}"));
        }
        let len = reg
            .get("results")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        if len != 2 {
            return Err(format!("expected 2 loop results, got {len}"));
        }
        Ok(())
    }
}
