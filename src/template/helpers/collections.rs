pub(super) fn has_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.len() < 2 {
        return Err(gtmpl::FuncError::AtLeastXArgs("has".to_string(), 2));
    }
    let Some(elem) = args.first() else {
        return Ok(gtmpl::Value::Bool(false));
    };
    let Some(second) = args.get(1) else {
        return Ok(gtmpl::Value::Bool(false));
    };
    let gtmpl::Value::Array(arr) = second else {
        return Ok(gtmpl::Value::Bool(false));
    };
    Ok(gtmpl::Value::Bool(arr.contains(elem)))
}
pub(super) fn append_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(arr_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("append".to_string(), 2));
    };
    let Some(item) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("append".to_string(), 2));
    };
    let gtmpl::Value::Array(arr) = arr_val else {
        return Err(gtmpl::FuncError::Generic(
            "append expects array".to_string(),
        ));
    };
    let mut new_arr = arr.clone();
    new_arr.push(item.clone());
    Ok(gtmpl::Value::Array(new_arr))
}
pub(super) fn slice_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.is_empty() {
        return Err(gtmpl::FuncError::AtLeastXArgs("slice".to_string(), 1));
    }
    let Some(first) = args.first() else {
        return Ok(gtmpl::Value::Array(Vec::new()));
    };
    let gtmpl::Value::Array(arr) = first else {
        return Ok(first.clone());
    };
    let start = args
        .get(1)
        .and_then(|v| match v {
            gtmpl::Value::Number(n) => n.as_i64(),
            _ => None,
        })
        .unwrap_or(0);
    let arr_len: i64 = arr.len().try_into().unwrap_or(0);
    let end = args
        .get(2)
        .and_then(|v| match v {
            gtmpl::Value::Number(n) => n.as_i64(),
            _ => None,
        })
        .unwrap_or(arr_len);
    let start_idx = std::cmp::max(0, start);
    let end_idx = std::cmp::min(arr_len, end);
    if start_idx >= end_idx {
        return Ok(gtmpl::Value::Array(Vec::new()));
    }
    let start_usize: usize = start_idx.try_into().unwrap_or(0);
    let end_usize: usize = end_idx.try_into().unwrap_or(0);
    let slice = arr.get(start_usize..end_usize).unwrap_or(&[]).to_vec();
    Ok(gtmpl::Value::Array(slice))
}
pub(super) fn first_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.is_empty() {
        return Err(gtmpl::FuncError::AtLeastXArgs("first".to_string(), 1));
    }
    let Some(first) = args.first() else {
        return Ok(gtmpl::Value::Nil);
    };
    match first {
        gtmpl::Value::Array(a) => Ok(a.first().cloned().unwrap_or(gtmpl::Value::Nil)),
        _ => Ok(first.clone()),
    }
}
pub(super) fn index_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("index".to_string(), 2));
    };
    let mut target = first.clone();
    let Some(keys) = args.get(1..) else {
        return Ok(target);
    };
    for key in keys {
        match target {
            gtmpl::Value::Map(m) | gtmpl::Value::Object(m) => {
                let gtmpl::Value::String(k) = key else {
                    return Ok(gtmpl::Value::Nil);
                };
                if let Some(v) = m.get(k) {
                    target = v.clone();
                } else {
                    return Ok(gtmpl::Value::Nil);
                }
            }
            gtmpl::Value::Array(a) => {
                let gtmpl::Value::Number(n) = key else {
                    return Ok(gtmpl::Value::Nil);
                };
                let Some(i) = n.as_i64() else {
                    return Ok(gtmpl::Value::Nil);
                };
                let idx: usize = i.try_into().unwrap_or(0);
                if let Some(v) = a.get(idx) {
                    target = v.clone();
                } else {
                    return Ok(gtmpl::Value::Nil);
                }
            }
            _ => return Ok(gtmpl::Value::Nil),
        }
    }
    Ok(target)
}
pub(super) fn subtract_list_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first_arg) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "subtractList".to_string(),
            2,
        ));
    };
    let Some(second_arg) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "subtractList".to_string(),
            2,
        ));
    };
    let gtmpl::Value::Array(list_a) = first_arg else {
        return Err(gtmpl::FuncError::Generic(
            "subtractList expects array first argument".to_string(),
        ));
    };
    let gtmpl::Value::Array(list_b) = second_arg else {
        return Err(gtmpl::FuncError::Generic(
            "subtractList expects array second argument".to_string(),
        ));
    };
    let mut result = Vec::new();
    for item_a in list_a {
        if !list_b.contains(item_a) {
            result.push(item_a.clone());
        }
    }
    Ok(gtmpl::Value::Array(result))
}
pub(super) fn get_string_slice_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(map_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "getStringSlice".to_string(),
            2,
        ));
    };
    let Some(key_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "getStringSlice".to_string(),
            2,
        ));
    };
    let (gtmpl::Value::Map(map) | gtmpl::Value::Object(map)) = map_val else {
        return Err(gtmpl::FuncError::Generic(
            "getStringSlice expects a map first argument".to_string(),
        ));
    };
    let gtmpl::Value::String(key) = key_val else {
        return Err(gtmpl::FuncError::Generic(
            "getStringSlice expects a string key second argument".to_string(),
        ));
    };
    if let Some(val) = map.get(key) {
        match val {
            gtmpl::Value::Array(arr) => {
                let mut result = Vec::new();
                for v in arr {
                    if let gtmpl::Value::String(s) = v {
                        result.push(gtmpl::Value::String(s.clone()));
                    }
                }
                Ok(gtmpl::Value::Array(result))
            }
            gtmpl::Value::String(s) => {
                let trimmed = s.trim();
                if trimmed.starts_with('[') && trimmed.ends_with(']') {
                    if let Ok(arr) = serde_json::from_str::<Vec<String>>(trimmed) {
                        let result = arr.into_iter().map(gtmpl::Value::String).collect();
                        return Ok(gtmpl::Value::Array(result));
                    }
                }
                Ok(gtmpl::Value::Array(vec![gtmpl::Value::String(s.clone())]))
            }
            _ => Ok(gtmpl::Value::Array(Vec::new())),
        }
    } else {
        Ok(gtmpl::Value::Array(Vec::new()))
    }
}
pub(super) fn map_to_named_string_args_fn(
    args: &[gtmpl::Value],
) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "mapToNamedStringArgs".to_string(),
            1,
        ));
    };
    let (gtmpl::Value::Map(map) | gtmpl::Value::Object(map)) = first else {
        return Err(gtmpl::FuncError::Generic(
            "mapToNamedStringArgs expects a map".to_string(),
        ));
    };
    let mut result = Vec::new();
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for k in keys {
        let v = map.get(k).unwrap_or(&gtmpl::Value::Nil);
        let val_str = match v {
            gtmpl::Value::String(s) => s.clone(),
            gtmpl::Value::Bool(b) => b.to_string(),
            gtmpl::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    i.to_string()
                } else if let Some(f) = n.as_f64() {
                    f.to_string()
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        };
        let mut item = std::collections::HashMap::new();
        item.insert("name".to_string(), gtmpl::Value::String(k.clone()));
        item.insert("value".to_string(), gtmpl::Value::String(val_str));
        result.push(gtmpl::Value::Map(item));
    }
    Ok(gtmpl::Value::Array(result))
}
pub(super) fn ternary_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(val_t) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("ternary".to_string(), 3));
    };
    let Some(val_f) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("ternary".to_string(), 3));
    };
    let Some(cond_val) = args.get(2) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("ternary".to_string(), 3));
    };
    let cond = match cond_val {
        gtmpl::Value::Bool(b) => *b,
        gtmpl::Value::Nil => false,
        _ => true,
    };
    if cond {
        Ok(val_t.clone())
    } else {
        Ok(val_f.clone())
    }
}
pub(super) fn default_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(default_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("default".to_string(), 2));
    };
    let Some(input_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("default".to_string(), 2));
    };
    match input_val {
        gtmpl::Value::Nil => Ok(default_val.clone()),
        gtmpl::Value::String(s) if s.trim().is_empty() => Ok(default_val.clone()),
        gtmpl::Value::Array(a) if a.is_empty() => Ok(default_val.clone()),
        gtmpl::Value::Map(o) | gtmpl::Value::Object(o) if o.is_empty() => Ok(default_val.clone()),
        gtmpl::Value::Bool(b) if !*b => Ok(default_val.clone()),
        _ => Ok(input_val.clone()),
    }
}
pub(super) fn eq_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.len() < 2 {
        return Err(gtmpl::FuncError::AtLeastXArgs("eq".to_string(), 2));
    }
    let Some(first) = args.first() else {
        return Ok(gtmpl::Value::Bool(false));
    };
    let Some(second) = args.get(1) else {
        return Ok(gtmpl::Value::Bool(false));
    };
    Ok(gtmpl::Value::Bool(first == second))
}
pub(super) fn ne_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.len() < 2 {
        return Err(gtmpl::FuncError::AtLeastXArgs("ne".to_string(), 2));
    }
    let Some(first) = args.first() else {
        return Ok(gtmpl::Value::Bool(true));
    };
    let Some(second) = args.get(1) else {
        return Ok(gtmpl::Value::Bool(true));
    };
    Ok(gtmpl::Value::Bool(first != second))
}
pub(super) fn empty_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.is_empty() {
        return Err(gtmpl::FuncError::AtLeastXArgs("empty".to_string(), 1));
    }
    let Some(first) = args.first() else {
        return Ok(gtmpl::Value::Bool(true));
    };
    let is_empty = match first {
        gtmpl::Value::Nil => true,
        gtmpl::Value::Bool(b) => !*b,
        gtmpl::Value::String(s) => s.trim().is_empty(),
        gtmpl::Value::Array(a) => a.is_empty(),
        gtmpl::Value::Map(o) | gtmpl::Value::Object(o) => o.is_empty(),
        gtmpl::Value::Number(n) => n.as_i64().unwrap_or(0) == 0,
        _ => false,
    };
    Ok(gtmpl::Value::Bool(is_empty))
}
pub(super) fn not_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.is_empty() {
        return Err(gtmpl::FuncError::AtLeastXArgs("not".to_string(), 1));
    }
    let Some(first) = args.first() else {
        return Ok(gtmpl::Value::Bool(true));
    };
    let cond = match first {
        gtmpl::Value::Bool(b) => *b,
        gtmpl::Value::Nil => false,
        _ => true,
    };
    Ok(gtmpl::Value::Bool(!cond))
}
pub(super) fn to_lower_byte_unit_fn(
    args: &[gtmpl::Value],
) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "toLowerByteUnit".to_string(),
            1,
        ));
    };
    let gtmpl::Value::String(s) = first else {
        return Err(gtmpl::FuncError::Generic(
            "toLowerByteUnit expects a string".to_string(),
        ));
    };
    let s = s.trim();
    if s.ends_with('i') && s.len() > 2 {
        let sub_len = s.len().saturating_sub(2);
        let prefix = s.get(..sub_len).unwrap_or("");
        let unit = s.get(sub_len..s.len().saturating_sub(1)).unwrap_or("");
        if prefix.chars().all(|c| c.is_ascii_digit()) && "KMGTPE".contains(unit) {
            return Ok(gtmpl::Value::String(format!(
                "{prefix}{unit_lower}",
                unit_lower = unit.to_lowercase()
            )));
        }
    }
    Ok(gtmpl::Value::String(s.to_string()))
}
