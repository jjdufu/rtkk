//! Recursive credential redaction (`********`) and unmodified secret preservation on write-back.

use crate::utils::base64_encode;
use serde_json::{Map, Value};

/// Placeholder string substituted for all redacted passwords, tokens, and inline keys.
pub const SECRET_MASK: &str = "********";

fn is_direct_secret_key(key: &str) -> bool {
    matches!(
        key,
        "ansible_ssh_pass"
            | "ansible_become_pass"
            | "ansible_become_password"
            | "become_pass"
            | "private_key_content"
    )
}

fn is_contextual_secret_key(
    parent_key: Option<&str>,
    key: &str,
    in_source_auth_entry: bool,
) -> bool {
    if is_direct_secret_key(key) {
        return true;
    }
    if parent_key == Some("registry") && matches!(key, "password" | "auth_basic") {
        return true;
    }
    if in_source_auth_entry && key == "password" {
        return true;
    }
    false
}

fn mask_recursive(val: &Value, parent_key: Option<&str>, in_source_auths_map: bool) -> Value {
    match val {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, v) in map {
                let is_secret = is_contextual_secret_key(parent_key, k, in_source_auths_map);
                if is_secret {
                    match v {
                        Value::String(s) if !s.is_empty() => {
                            out.insert(k.clone(), Value::String(SECRET_MASK.to_string()));
                        }
                        _ => {
                            out.insert(k.clone(), v.clone());
                        }
                    }
                } else {
                    let child_in_source_auths = parent_key == Some("source_auths");
                    out.insert(
                        k.clone(),
                        mask_recursive(v, Some(k.as_str()), child_in_source_auths),
                    );
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|item| mask_recursive(item, parent_key, false))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Returns a deep copy of `doc` with all SSH, privilege-escalation, and registry secrets masked as `"********"`.
#[must_use]
pub fn mask_inventory_value(doc: &Value) -> Value {
    mask_recursive(doc, None, false)
}

fn restore_recursive(candidate: &mut Value, original: Option<&Value>) {
    match candidate {
        Value::Object(cand_map) => {
            let orig_map = original.and_then(Value::as_object);
            for (k, cand_val) in cand_map.iter_mut() {
                let orig_child = orig_map.and_then(|m| m.get(k));
                if cand_val.as_str() == Some(SECRET_MASK) {
                    if let Some(orig_v) = orig_child {
                        *cand_val = orig_v.clone();
                    }
                } else {
                    restore_recursive(cand_val, orig_child);
                }
            }
        }
        Value::Array(cand_arr) => {
            let orig_arr = original.and_then(Value::as_array);
            for (idx, item) in cand_arr.iter_mut().enumerate() {
                let orig_item = orig_arr.and_then(|a| a.get(idx));
                restore_recursive(item, orig_item);
            }
        }
        _ => {}
    }
}

fn extract_registry_obj_mut(doc: &mut Value) -> Option<&mut Map<String, Value>> {
    let in_top_vars = doc
        .get("vars")
        .and_then(|v| v.get("registry"))
        .is_some_and(Value::is_object);
    if in_top_vars {
        return doc
            .get_mut("vars")
            .and_then(|v| v.get_mut("registry"))
            .and_then(Value::as_object_mut);
    }
    doc.get_mut("all")
        .and_then(|a| a.get_mut("vars"))
        .and_then(|v| v.get_mut("registry"))
        .and_then(Value::as_object_mut)
}

fn extract_registry_obj(doc: &Value) -> Option<&Map<String, Value>> {
    doc.get("vars")
        .and_then(|v| v.get("registry"))
        .and_then(Value::as_object)
        .or_else(|| {
            doc.get("all")
                .and_then(|a| a.get("vars"))
                .and_then(|v| v.get("registry"))
                .and_then(Value::as_object)
        })
}

/// Restores unmodified `"********"` secret fields in `candidate` from `disk_original`, and
/// recomputes `registry.auth_basic` when `registry.username` or `registry.password` changed.
pub fn restore_masked_secrets(candidate: &mut Value, disk_original: &Value) {
    let old_reg_user = extract_registry_obj(disk_original)
        .and_then(|r| r.get("username"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let old_reg_pass = extract_registry_obj(disk_original)
        .and_then(|r| r.get("password"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let old_auth_basic = extract_registry_obj(disk_original)
        .and_then(|r| r.get("auth_basic"))
        .and_then(Value::as_str)
        .map(str::to_string);

    restore_recursive(candidate, Some(disk_original));

    if let Some(reg_mut) = extract_registry_obj_mut(candidate) {
        let new_user = reg_mut
            .get("username")
            .and_then(Value::as_str)
            .map(str::to_string);
        let new_pass = reg_mut
            .get("password")
            .and_then(Value::as_str)
            .map(str::to_string);
        let cur_auth = reg_mut
            .get("auth_basic")
            .and_then(Value::as_str)
            .map(str::to_string);

        let creds_changed = new_user != old_reg_user || new_pass != old_reg_pass;
        let auth_unmodified_or_empty = cur_auth.as_deref().is_none_or(str::is_empty)
            || cur_auth == old_auth_basic
            || cur_auth.as_deref() == Some(SECRET_MASK);

        if let (Some(u), Some(p)) = (new_user, new_pass)
            && !u.is_empty()
            && !p.is_empty()
            && p != SECRET_MASK
            && (creds_changed || auth_unmodified_or_empty)
        {
            // Preserve custom read-only robot auth_basic when credentials did not change
            let old_expected = match (&old_reg_user, &old_reg_pass) {
                (Some(ou), Some(op)) => Some(base64_encode(format!("{ou}:{op}").as_bytes())),
                _ => None,
            };
            if creds_changed
                || cur_auth.as_deref().is_none_or(str::is_empty)
                || cur_auth == old_expected
            {
                let derived = base64_encode(format!("{u}:{p}").as_bytes());
                reg_mut.insert("auth_basic".to_string(), Value::String(derived));
            }
        }
    }
}

/// Returns `true` if any string value in `val` still equals `"********"`.
#[must_use]
pub fn contains_mask_placeholder(val: &Value) -> bool {
    match val {
        Value::String(s) => s == SECRET_MASK,
        Value::Array(arr) => arr.iter().any(contains_mask_placeholder),
        Value::Object(map) => map.values().any(contains_mask_placeholder),
        _ => false,
    }
}
