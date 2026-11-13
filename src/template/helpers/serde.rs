//! `JSON` and `YAML` serialization and deserialization template helper functions.

use crate::template::{gtmpl_to_json, json_to_gtmpl};

pub(super) fn to_json_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("toJson".to_string(), 1));
    };
    let json = gtmpl_to_json(first);
    Ok(gtmpl::Value::String(json.to_string()))
}

pub(super) fn to_yaml_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("toYaml".to_string(), 1));
    };
    let json = gtmpl_to_json(first);
    let yaml_str = serde_yml::to_string(&json)
        .map_err(|e| gtmpl::FuncError::Generic(format!("toYaml failed: {e}")))?;
    let yaml_trimmed = yaml_str.strip_prefix("---\n").unwrap_or(&yaml_str);
    Ok(gtmpl::Value::String(yaml_trimmed.to_string()))
}

pub(super) fn from_yaml_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("fromYaml".to_string(), 1));
    };
    let gtmpl::Value::String(s) = first else {
        return Err(gtmpl::FuncError::Generic(
            "fromYaml expects a string".to_string(),
        ));
    };
    let json: serde_json::Value = serde_yml::from_str(s)
        .map_err(|e| gtmpl::FuncError::Generic(format!("fromYaml failed: {e}")))?;
    Ok(json_to_gtmpl(&json))
}
