#[must_use]
pub(crate) fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}
#[must_use]
pub(crate) fn sed_replacement_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' | '|' | '&' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}
pub(crate) fn is_safe_module_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}
#[must_use]
pub(crate) fn env_flag_enabled(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            t == "1" || t == "true" || t == "yes" || t == "on"
        }
        Err(_) => false,
    }
}
pub(crate) fn deep_merge(base: &mut serde_json::Value, incoming: &serde_json::Value) {
    match (base, incoming) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(inc_map)) => {
            for (k, v) in inc_map {
                if let Some(base_val) = base_map.get_mut(k) {
                    deep_merge(base_val, v);
                } else {
                    base_map.insert(k.clone(), v.clone());
                }
            }
        }
        (serde_json::Value::Array(base_arr), serde_json::Value::Array(inc_arr)) => {
            for item in inc_arr {
                if !base_arr.contains(item) {
                    base_arr.push(item.clone());
                }
            }
        }
        (base_val, inc_val) => {
            *base_val = inc_val.clone();
        }
    }
}
#[must_use]
pub(crate) fn wrap_condition_string(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
        trimmed.to_string()
    } else {
        format!("{{{{ {trimmed} }}}}")
    }
}
#[must_use]
pub(crate) fn u64_to_f64(u: u64) -> f64 {
    let high = u32::try_from(u >> 32).unwrap_or(0);
    let low = u32::try_from(u & 0xFFFF_FFFF).unwrap_or(0);
    f64::from(high) * 4_294_967_296.0 + f64::from(low)
}
#[must_use]
pub(crate) fn usize_to_f64(u: usize) -> f64 {
    let u64_val = u64::try_from(u).unwrap_or(0);
    u64_to_f64(u64_val)
}
