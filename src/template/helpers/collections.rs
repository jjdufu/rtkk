//! Collection, comparison, and emptiness template helper functions.

fn is_empty_val(val: &gtmpl::Value) -> bool {
    match val {
        gtmpl::Value::Nil | gtmpl::Value::NoValue => true,
        gtmpl::Value::Bool(b) => !*b,
        gtmpl::Value::String(s) => s.trim().is_empty(),
        gtmpl::Value::Array(a) => a.is_empty(),
        gtmpl::Value::Map(o) | gtmpl::Value::Object(o) => o.is_empty(),
        gtmpl::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i == 0
            } else if let Some(u) = n.as_u64() {
                u == 0
            } else if let Some(f) = n.as_f64() {
                f.abs() < f64::EPSILON
            } else {
                false
            }
        }
        gtmpl::Value::Function(_) => false,
    }
}

fn numbers_equal(na: &gtmpl_value::Number, nb: &gtmpl_value::Number) -> bool {
    if let (Some(ia), Some(ib)) = (na.as_i64(), nb.as_i64()) {
        ia == ib
    } else if let (Some(ua), Some(ub)) = (na.as_u64(), nb.as_u64()) {
        ua == ub
    } else if let (Some(ia), Some(ub)) = (na.as_i64(), nb.as_u64()) {
        u64::try_from(ia).is_ok_and(|ua| ua == ub)
    } else if let (Some(ua), Some(ib)) = (na.as_u64(), nb.as_i64()) {
        u64::try_from(ib).is_ok_and(|ub| ua == ub)
    } else if let (Some(fa), Some(fb)) = (na.as_f64(), nb.as_f64()) {
        (fa - fb).abs() < f64::EPSILON
    } else {
        false
    }
}

fn values_equal(a: &gtmpl::Value, b: &gtmpl::Value) -> bool {
    match (a, b) {
        (gtmpl::Value::Nil | gtmpl::Value::NoValue, gtmpl::Value::Nil | gtmpl::Value::NoValue) => {
            true
        }
        (gtmpl::Value::Nil | gtmpl::Value::NoValue, _)
        | (_, gtmpl::Value::Nil | gtmpl::Value::NoValue) => false,
        (gtmpl::Value::Number(na), gtmpl::Value::Number(nb)) => numbers_equal(na, nb),
        (gtmpl::Value::Array(aa), gtmpl::Value::Array(ab)) => {
            aa.len() == ab.len() && aa.iter().zip(ab.iter()).all(|(x, y)| values_equal(x, y))
        }
        (
            gtmpl::Value::Map(ma) | gtmpl::Value::Object(ma),
            gtmpl::Value::Map(mb) | gtmpl::Value::Object(mb),
        ) => {
            ma.len() == mb.len()
                && ma
                    .iter()
                    .all(|(k, va)| mb.get(k).is_some_and(|vb| values_equal(va, vb)))
        }
        (_, _) => a == b,
    }
}

pub(super) fn has_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(elem) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("has".to_string(), 2));
    };
    let Some(second) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("has".to_string(), 2));
    };
    let gtmpl::Value::Array(arr) = second else {
        return Ok(gtmpl::Value::Bool(false));
    };
    Ok(gtmpl::Value::Bool(
        arr.iter().any(|item| values_equal(item, elem)),
    ))
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
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("slice".to_string(), 1));
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
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("first".to_string(), 1));
    };
    match first {
        gtmpl::Value::Array(a) => Ok(a.first().cloned().unwrap_or(gtmpl::Value::Nil)),
        _ => Ok(first.clone()),
    }
}

fn parse_array_index(key: &gtmpl::Value) -> Result<usize, gtmpl::FuncError> {
    let gtmpl::Value::Number(n) = key else {
        return Err(gtmpl::FuncError::Generic(
            "index on array requires an integer index".to_string(),
        ));
    };
    if let Some(i) = n.as_i64() {
        if i < 0 {
            return Err(gtmpl::FuncError::Generic(format!(
                "index: negative array index {i}"
            )));
        }
        usize::try_from(i)
            .map_err(|_| gtmpl::FuncError::Generic(format!("index: invalid array index {i}")))
    } else if let Some(u) = n.as_u64() {
        usize::try_from(u)
            .map_err(|_| gtmpl::FuncError::Generic(format!("index: array index overflow {u}")))
    } else {
        Err(gtmpl::FuncError::Generic(
            "index on array requires an integer index".to_string(),
        ))
    }
}

pub(super) fn index_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.len() < 2 {
        return Err(gtmpl::FuncError::AtLeastXArgs("index".to_string(), 2));
    }
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("index".to_string(), 2));
    };
    let Some(keys) = args.get(1..) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("index".to_string(), 2));
    };
    let mut target = first.clone();
    for key in keys {
        let next = match &target {
            gtmpl::Value::Map(m) | gtmpl::Value::Object(m) => {
                let gtmpl::Value::String(k) = key else {
                    return Err(gtmpl::FuncError::Generic(
                        "index on map requires a string key".to_string(),
                    ));
                };
                match m.get(k) {
                    Some(v) => v.clone(),
                    None => return Ok(gtmpl::Value::Nil),
                }
            }
            gtmpl::Value::Array(a) => {
                let idx = parse_array_index(key)?;
                let Some(v) = a.get(idx) else {
                    return Err(gtmpl::FuncError::Generic(format!(
                        "index: array index {idx} out of bounds (len {})",
                        a.len()
                    )));
                };
                v.clone()
            }
            gtmpl::Value::Nil | gtmpl::Value::NoValue => return Ok(gtmpl::Value::Nil),
            _ => {
                return Err(gtmpl::FuncError::Generic(
                    "index target is neither map nor array".to_string(),
                ));
            }
        };
        target = next;
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
        if !list_b.iter().any(|item_b| values_equal(item_a, item_b)) {
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
    let Some(val) = map.get(key) else {
        return Ok(gtmpl::Value::Array(Vec::new()));
    };
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
            if trimmed.starts_with('[')
                && trimmed.ends_with(']')
                && let Ok(arr) = serde_json::from_str::<Vec<String>>(trimmed)
            {
                let result = arr.into_iter().map(gtmpl::Value::String).collect();
                return Ok(gtmpl::Value::Array(result));
            }
            Ok(gtmpl::Value::Array(vec![gtmpl::Value::String(s.clone())]))
        }
        _ => Ok(gtmpl::Value::Array(Vec::new())),
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
                } else if let Some(u) = n.as_u64() {
                    u.to_string()
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
    if is_empty_val(cond_val) {
        Ok(val_f.clone())
    } else {
        Ok(val_t.clone())
    }
}

pub(super) fn default_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(default_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("default".to_string(), 2));
    };
    let Some(input_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("default".to_string(), 2));
    };
    if is_empty_val(input_val) {
        Ok(default_val.clone())
    } else {
        Ok(input_val.clone())
    }
}

pub(super) fn eq_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.len() < 2 {
        return Err(gtmpl::FuncError::AtLeastXArgs("eq".to_string(), 2));
    }
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("eq".to_string(), 2));
    };
    let Some(others) = args.get(1..) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("eq".to_string(), 2));
    };
    Ok(gtmpl::Value::Bool(
        others.iter().any(|other| values_equal(first, other)),
    ))
}

pub(super) fn ne_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.len() < 2 {
        return Err(gtmpl::FuncError::AtLeastXArgs("ne".to_string(), 2));
    }
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("ne".to_string(), 2));
    };
    let Some(second) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("ne".to_string(), 2));
    };
    Ok(gtmpl::Value::Bool(!values_equal(first, second)))
}

pub(super) fn empty_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("empty".to_string(), 1));
    };
    Ok(gtmpl::Value::Bool(is_empty_val(first)))
}

pub(super) fn not_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("not".to_string(), 1));
    };
    Ok(gtmpl::Value::Bool(is_empty_val(first)))
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
