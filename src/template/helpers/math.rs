use rand::Rng as _;
pub(super) fn rand_int_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(min_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("randInt".to_string(), 2));
    };
    let Some(max_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("randInt".to_string(), 2));
    };
    let min = match min_val {
        gtmpl::Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    };
    let max = match max_val {
        gtmpl::Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    };
    if min >= max {
        return Ok(gtmpl::Value::Number(gtmpl_value::Number::from(min)));
    }
    let val: i64 = rand::thread_rng().gen_range(min..max);
    Ok(gtmpl::Value::Number(gtmpl_value::Number::from(val)))
}
pub(super) fn sub_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(a_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("sub".to_string(), 2));
    };
    let Some(b_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("sub".to_string(), 2));
    };
    let a = match a_val {
        gtmpl::Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    };
    let b = match b_val {
        gtmpl::Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    };
    Ok(gtmpl::Value::Number(gtmpl_value::Number::from(a - b)))
}
pub(super) fn int_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.is_empty() {
        return Err(gtmpl::FuncError::AtLeastXArgs("int".to_string(), 1));
    }
    let Some(first) = args.first() else {
        return Ok(gtmpl::Value::Number(gtmpl_value::Number::from(0_i64)));
    };
    let val = match first {
        gtmpl::Value::Number(n) => n.as_i64().unwrap_or(0),
        gtmpl::Value::String(s) => s.parse::<i64>().unwrap_or(0),
        gtmpl::Value::Bool(b) => i64::from(*b),
        _ => 0,
    };
    Ok(gtmpl::Value::Number(gtmpl_value::Number::from(val)))
}
pub(super) fn len_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.is_empty() {
        return Err(gtmpl::FuncError::AtLeastXArgs("len".to_string(), 1));
    }
    let Some(first) = args.first() else {
        return Ok(gtmpl::Value::Number(gtmpl_value::Number::from(0_i64)));
    };
    let length = match first {
        gtmpl::Value::Array(a) => a.len(),
        gtmpl::Value::Map(o) | gtmpl::Value::Object(o) => o.len(),
        gtmpl::Value::String(s) => s.len(),
        _ => 0,
    };
    let len_i64: i64 = length.try_into().unwrap_or(0);
    Ok(gtmpl::Value::Number(gtmpl_value::Number::from(len_i64)))
}
pub(super) fn pow_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(base_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("pow".to_string(), 2));
    };
    let Some(exp_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("pow".to_string(), 2));
    };
    let base = match base_val {
        gtmpl::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    };
    let exponent = match exp_val {
        gtmpl::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    };
    let res = base.powf(exponent);
    Ok(gtmpl::Value::Number(gtmpl_value::Number::from(res)))
}
