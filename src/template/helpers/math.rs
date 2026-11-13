//! Numeric and arithmetic template helper functions.

use rand::Rng as _;

fn f64_to_i64_checked(f: f64) -> Option<i64> {
    if !f.is_finite() {
        return None;
    }
    let truncated = f.trunc();
    format!("{truncated:.0}").parse::<i64>().ok()
}

fn number_to_i64(n: &gtmpl_value::Number, fn_name: &str) -> Result<i64, gtmpl::FuncError> {
    if let Some(i) = n.as_i64() {
        Ok(i)
    } else if let Some(u) = n.as_u64() {
        i64::try_from(u)
            .map_err(|_| gtmpl::FuncError::Generic(format!("{fn_name}: integer {u} overflows i64")))
    } else if let Some(f) = n.as_f64() {
        f64_to_i64_checked(f).ok_or_else(|| {
            gtmpl::FuncError::Generic(format!("{fn_name}: float {f} out of i64 range"))
        })
    } else {
        Err(gtmpl::FuncError::Generic(format!(
            "{fn_name}: unsupported numeric value"
        )))
    }
}

fn arg_to_i64(val: &gtmpl::Value, fn_name: &str) -> Result<i64, gtmpl::FuncError> {
    match val {
        gtmpl::Value::Number(n) => number_to_i64(n, fn_name),
        gtmpl::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(0)
            } else if let Ok(i) = trimmed.parse::<i64>() {
                Ok(i)
            } else if let Ok(f) = trimmed.parse::<f64>() {
                f64_to_i64_checked(f).ok_or_else(|| {
                    gtmpl::FuncError::Generic(format!("{fn_name}: float '{s}' out of i64 range"))
                })
            } else {
                Err(gtmpl::FuncError::Generic(format!(
                    "{fn_name}: invalid integer '{s}'"
                )))
            }
        }
        gtmpl::Value::Bool(b) => Ok(i64::from(*b)),
        gtmpl::Value::Nil | gtmpl::Value::NoValue => Ok(0),
        _ => Err(gtmpl::FuncError::Generic(format!(
            "{fn_name}: expected numeric argument"
        ))),
    }
}

pub(super) fn rand_int_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(min_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("randInt".to_string(), 2));
    };
    let Some(max_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("randInt".to_string(), 2));
    };
    let min = arg_to_i64(min_val, "randInt")?;
    let max = arg_to_i64(max_val, "randInt")?;
    if min >= max {
        return Ok(gtmpl::Value::Number(gtmpl_value::Number::from(min)));
    }
    let val: i64 = rand::thread_rng().gen_range(min..max);
    Ok(gtmpl::Value::Number(gtmpl_value::Number::from(val)))
}

pub(super) fn add_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(a_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("add".to_string(), 2));
    };
    let Some(b_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("add".to_string(), 2));
    };
    let a = arg_to_i64(a_val, "add")?;
    let b = arg_to_i64(b_val, "add")?;
    let sum = a
        .checked_add(b)
        .ok_or_else(|| gtmpl::FuncError::Generic(format!("integer overflow in add({a}, {b})")))?;
    Ok(gtmpl::Value::Number(gtmpl_value::Number::from(sum)))
}

pub(super) fn sub_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(a_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("sub".to_string(), 2));
    };
    let Some(b_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("sub".to_string(), 2));
    };
    let a = arg_to_i64(a_val, "sub")?;
    let b = arg_to_i64(b_val, "sub")?;
    let diff = a
        .checked_sub(b)
        .ok_or_else(|| gtmpl::FuncError::Generic(format!("integer overflow in sub({a}, {b})")))?;
    Ok(gtmpl::Value::Number(gtmpl_value::Number::from(diff)))
}

pub(super) fn int_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("int".to_string(), 1));
    };
    let val = match first {
        gtmpl::Value::Number(n) => number_to_i64(n, "int")?,
        gtmpl::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                0
            } else if let Ok(i) = trimmed.parse::<i64>() {
                i
            } else if let Ok(f) = trimmed.parse::<f64>() {
                f64_to_i64_checked(f).ok_or_else(|| {
                    gtmpl::FuncError::Generic(format!("int: float '{s}' out of i64 range"))
                })?
            } else {
                0
            }
        }
        gtmpl::Value::Bool(b) => i64::from(*b),
        _ => 0,
    };
    Ok(gtmpl::Value::Number(gtmpl_value::Number::from(val)))
}

pub(super) fn len_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("len".to_string(), 1));
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
