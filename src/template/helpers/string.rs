use crate::template::gtmpl_to_json;
pub(super) fn trim_prefix_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(prefix_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("trimPrefix".to_string(), 2));
    };
    let Some(s_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("trimPrefix".to_string(), 2));
    };
    let prefix = match prefix_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let s = match s_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    Ok(gtmpl::Value::String(
        s.strip_prefix(prefix).unwrap_or(s).to_string(),
    ))
}
pub(super) fn trim_suffix_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(suffix_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("trimSuffix".to_string(), 2));
    };
    let Some(s_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("trimSuffix".to_string(), 2));
    };
    let suffix = match suffix_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let s = match s_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    Ok(gtmpl::Value::String(
        s.strip_suffix(suffix).unwrap_or(s).to_string(),
    ))
}
pub(super) fn join_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(sep_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("join".to_string(), 2));
    };
    let Some(arr_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("join".to_string(), 2));
    };
    let sep = match sep_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let gtmpl::Value::Array(arr) = arr_val else {
        return Err(gtmpl::FuncError::Generic("join expects array".to_string()));
    };
    let mut parts = Vec::new();
    for v in arr {
        match v {
            gtmpl::Value::String(s) => parts.push(s.clone()),
            _ => parts.push(gtmpl_to_json(v).to_string()),
        }
    }
    Ok(gtmpl::Value::String(parts.join(sep)))
}
pub(super) fn cat_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.is_empty() {
        return Err(gtmpl::FuncError::AtLeastXArgs("cat".to_string(), 1));
    }
    let mut parts = Vec::new();
    for v in args {
        match v {
            gtmpl::Value::String(s) => parts.push(s.clone()),
            _ => parts.push(gtmpl_to_json(v).to_string()),
        }
    }
    Ok(gtmpl::Value::String(parts.join(" ")))
}
pub(super) fn split_list_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(sep_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("splitList".to_string(), 2));
    };
    let Some(s_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("splitList".to_string(), 2));
    };
    let sep = match sep_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let s = match s_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let list = s
        .split(sep)
        .map(|part| gtmpl::Value::String(part.to_string()))
        .collect();
    Ok(gtmpl::Value::Array(list))
}
pub(super) fn indent_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(spaces_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("indent".to_string(), 2));
    };
    let Some(s_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("indent".to_string(), 2));
    };
    let spaces_count = match spaces_val {
        gtmpl::Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    };
    let s = match s_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let pad = " ".repeat(spaces_count.try_into().unwrap_or(0));
    let mut lines = Vec::new();
    for line in s.lines() {
        if line.is_empty() {
            lines.push(String::new());
        } else {
            lines.push(format!("{pad}{line}"));
        }
    }
    Ok(gtmpl::Value::String(lines.join("\n")))
}
pub(super) fn unquote_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("unquote".to_string(), 1));
    };
    let gtmpl::Value::String(s) = first else {
        return Err(gtmpl::FuncError::Generic(
            "unquote expects a string".to_string(),
        ));
    };
    let mut trimmed = s.trim();
    if ((trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
        && trimmed.len() >= 2
    {
        trimmed = trimmed
            .get(1..trimmed.len().saturating_sub(1))
            .unwrap_or("");
    }
    Ok(gtmpl::Value::String(trimmed.to_string()))
}
pub(super) fn regex_match_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(pat_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("regexMatch".to_string(), 2));
    };
    let Some(s_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("regexMatch".to_string(), 2));
    };
    let pattern = match pat_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let s = match s_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let re = regex::Regex::new(pattern)
        .map_err(|e| gtmpl::FuncError::Generic(format!("invalid regex: {e}")))?;
    Ok(gtmpl::Value::Bool(re.is_match(s)))
}
pub(super) fn regex_replace_all_fn(
    args: &[gtmpl::Value],
) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(pat_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "regexReplaceAll".to_string(),
            3,
        ));
    };
    let Some(s_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "regexReplaceAll".to_string(),
            3,
        ));
    };
    let Some(repl_val) = args.get(2) else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "regexReplaceAll".to_string(),
            3,
        ));
    };
    let pattern = match pat_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let s = match s_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let repl = match repl_val {
        gtmpl::Value::String(s) => s.as_str(),
        _ => "",
    };
    let re = regex::Regex::new(pattern)
        .map_err(|e| gtmpl::FuncError::Generic(format!("invalid regex: {e}")))?;
    let result = re.replace_all(s, repl).to_string();
    Ok(gtmpl::Value::String(result))
}
