//! Shared string escaping, `Base64` encoding, file mode parsing, and `JSON` merge utilities.

use std::sync::OnceLock;

/// Standard `RFC` 4648 `Base64` alphabet.
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Compiled regular expression for stripping `ANSI` terminal escape sequences.
static ANSI_REGEX: OnceLock<Option<regex::Regex>> = OnceLock::new();

/// Wraps `s` in POSIX shell single quotes, escaping embedded single quotes safely.
#[must_use]
pub fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(2));
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

/// Escapes `\`, `|`, `&`, and newlines in `s` for safe use as a `sed` replacement string
/// delimited by `|`.
#[must_use]
pub fn sed_replacement_escape(s: &str) -> String {
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

/// Escapes regular expression metacharacters (`.+*?^$()[]{}|\`) in `s` with a leading `\`.
#[must_use]
pub fn regex_escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(
            ch,
            '.' | '+' | '*' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Validates that `name` is a safe kernel or playbook module identifier.
///
/// Returns `true` only when `name` is non-empty, does not start with `'-'` or `'.'`,
/// does not contain `".."`, and consists exclusively of `ASCII` alphanumeric characters,
/// `'_'`, `'-'`, or `'.'`.
#[must_use]
pub fn is_safe_module_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

/// Strips `ANSI` escape sequences and `[RTKK_SUDO_PROMPT]` markers from `s`.
#[must_use]
pub fn clean_ansi_escapes(s: &str) -> String {
    let re_opt = ANSI_REGEX.get_or_init(|| {
        regex::Regex::new(
            r"\x1b\[[0-9:;<=>?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\^_]",
        )
        .ok()
    });
    let without_ansi = if let Some(re) = re_opt {
        re.replace_all(s, "")
    } else {
        std::borrow::Cow::Borrowed(s)
    };
    without_ansi.replace("[RTKK_SUDO_PROMPT]", "")
}

/// Encodes `input` using standard `RFC` 4648 `Base64` with `'='` padding.
#[must_use]
pub fn base64_encode(input: &[u8]) -> String {
    if input.is_empty() {
        return String::new();
    }
    let cap = (input.len().saturating_add(2) / 3).saturating_mul(4);
    let mut out = Vec::with_capacity(cap);
    for chunk in input.chunks(3) {
        let Some(&b0) = chunk.first() else {
            continue;
        };
        let b1 = chunk.get(1).copied();
        let b2 = chunk.get(2).copied();

        let i0 = usize::from(b0 >> 2);
        let i1 = usize::from(((b0 & 0x03) << 4) | (b1.unwrap_or(0) >> 4));
        out.push(BASE64_ALPHABET.get(i0).copied().unwrap_or(b'='));
        out.push(BASE64_ALPHABET.get(i1).copied().unwrap_or(b'='));

        let c2 = if let Some(v1) = b1 {
            let i2 = usize::from(((v1 & 0x0F) << 2) | (b2.unwrap_or(0) >> 6));
            BASE64_ALPHABET.get(i2).copied().unwrap_or(b'=')
        } else {
            b'='
        };
        out.push(c2);

        let c3 = if let Some(v2) = b2 {
            let i3 = usize::from(v2 & 0x3F);
            BASE64_ALPHABET.get(i3).copied().unwrap_or(b'=')
        } else {
            b'='
        };
        out.push(c3);
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Parses an optional `JSON`/`YAML` file permission value into a Unix mode bitmask.
///
/// - `None` or `Some(Value::Null)` returns `default_mode`.
/// - `Some(Value::String)` trims whitespace, strips an optional `"0o"`/`"0O"` prefix,
///   and parses base-8 octal digits up to `0o7777`.
/// - `Some(Value::Number)` returns values `<= 0o777` directly; for values `> 0o777` whose
///   decimal representation contains only octal digits `'0'..='7'` and parses in base 8 to
///   `<= 0o7777` (such as unquoted `YAML` `1.2` `600`, `644`, `755`, or `1777`), returns the
///   parsed octal value; otherwise returns numeric values `<= 0o7777`.
///
/// # Errors
///
/// Returns an error if the value is not a string or non-negative integer, is empty,
/// contains invalid octal digits, or exceeds `0o7777`.
pub fn parse_file_mode(val: Option<&serde_json::Value>, default_mode: u32) -> Result<u32, String> {
    match val {
        None | Some(serde_json::Value::Null) => Ok(default_mode),
        Some(serde_json::Value::String(s)) => parse_file_mode_str(s),
        Some(serde_json::Value::Number(n)) => parse_file_mode_num(n),
        Some(_) => Err("file mode must be an octal string or non-negative integer".to_string()),
    }
}

fn parse_file_mode_str(s: &str) -> Result<u32, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("file mode string cannot be empty".to_string());
    }
    let digits = trimmed
        .strip_prefix("0o")
        .or_else(|| trimmed.strip_prefix("0O"))
        .unwrap_or(trimmed);
    if digits.is_empty() || !digits.chars().all(|c| ('0'..='7').contains(&c)) {
        return Err(format!("invalid octal file mode string: '{s}'"));
    }
    let mode = u32::from_str_radix(digits, 8)
        .map_err(|e| format!("failed to parse octal file mode '{s}': {e}"))?;
    if mode > 0o7777 {
        return Err(format!("file mode '{s}' exceeds maximum 0o7777"));
    }
    Ok(mode)
}

fn parse_file_mode_num(n: &serde_json::Number) -> Result<u32, String> {
    let Some(raw_u64) = n.as_u64() else {
        return Err(format!("invalid numeric file mode: {n}"));
    };
    let m = u32::try_from(raw_u64).map_err(|_| format!("file mode out of range: {raw_u64}"))?;
    if m <= 0o777 {
        return Ok(m);
    }
    let dec = m.to_string();
    if dec.chars().all(|c| ('0'..='7').contains(&c)) {
        let octal = u32::from_str_radix(&dec, 8).unwrap_or(u32::MAX);
        if octal <= 0o7777 {
            return Ok(octal);
        }
    }
    if m <= 0o7777 {
        Ok(m)
    } else {
        Err(format!("file mode {m} exceeds maximum 0o7777"))
    }
}

/// Recursively merges `incoming` into `base`, combining nested objects and deduplicating arrays.
pub fn deep_merge(base: &mut serde_json::Value, incoming: &serde_json::Value) {
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
            base_val.clone_from(inc_val);
        }
    }
}

/// Wraps a `when` condition expression in `{{ ... }}` delimiters unless empty or already templated.
#[must_use]
pub fn wrap_condition_string(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains("{{") && trimmed.contains("}}") {
        trimmed.to_string()
    } else {
        format!("{{{{ {trimmed} }}}}")
    }
}

/// Converts a `u64` integer into an `f64` value without `as` casts.
#[must_use]
pub fn u64_to_f64(u: u64) -> f64 {
    let high = u32::try_from(u >> 32).unwrap_or(0);
    let low = u32::try_from(u & 0xFFFF_FFFF).unwrap_or(0);
    f64::from(high) * 4_294_967_296.0 + f64::from(low)
}

/// Converts a `usize` integer into an `f64` value without `as` casts.
#[must_use]
pub fn usize_to_f64(u: usize) -> f64 {
    let u64_val = u64::try_from(u).unwrap_or(0);
    u64_to_f64(u64_val)
}

/// Expands a leading `"~"` or `"~/..."` prefix against `$HOME` (falling back to `"/root"` when unset).
///
/// Strings that are neither `"~"` nor prefixed with `"~/"` are returned trimmed without modification.
#[must_use]
pub fn expand_tilde_str(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return std::env::var("HOME")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "/root".to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "/root".to_string());
        return std::path::Path::new(&home)
            .join(rest)
            .to_string_lossy()
            .into_owned();
    }
    trimmed.to_string()
}

/// Expands a leading `"~"` or `"~/..."` prefix into a [`std::path::PathBuf`].
#[must_use]
pub fn expand_tilde_path(raw: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(expand_tilde_str(raw))
}
