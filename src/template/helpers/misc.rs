use crate::template::{get_templates_registry, render_template_string};
pub(super) fn semver_compare_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(constraint_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "semverCompare".to_string(),
            2,
        ));
    };
    let Some(ver_val) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "semverCompare".to_string(),
            2,
        ));
    };
    let gtmpl::Value::String(constraint_str) = constraint_val else {
        return Err(gtmpl::FuncError::Generic(
            "semverCompare expects a string constraint".to_string(),
        ));
    };
    let gtmpl::Value::String(ver_str) = ver_val else {
        return Err(gtmpl::FuncError::Generic(
            "semverCompare expects a string version".to_string(),
        ));
    };
    let clean_ver = ver_str.trim().strip_prefix('v').unwrap_or(ver_str.trim());
    let parsed_version = semver::Version::parse(clean_ver).map_err(|e| {
        gtmpl::FuncError::Generic(format!("invalid semver version '{ver_str}': {e}"))
    })?;
    let clean_constraint = constraint_str
        .trim()
        .strip_prefix('v')
        .unwrap_or(constraint_str.trim());
    let constraint_with_op = if clean_constraint.starts_with(|c: char| c.is_ascii_digit()) {
        format!("={clean_constraint}")
    } else {
        clean_constraint.to_string()
    };
    let requirement = semver::VersionReq::parse(&constraint_with_op).map_err(|e| {
        gtmpl::FuncError::Generic(format!("invalid semver constraint '{constraint_str}': {e}"))
    })?;
    let matches = requirement.matches(&parsed_version);
    Ok(gtmpl::Value::Bool(matches))
}
pub(super) fn file_exists_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("fileExists".to_string(), 1));
    };
    let gtmpl::Value::String(path_str) = first else {
        return Err(gtmpl::FuncError::Generic(
            "fileExists expects a string path".to_string(),
        ));
    };
    let path = std::path::Path::new(path_str);
    Ok(gtmpl::Value::Bool(path.exists() && path.is_file()))
}
pub(super) fn tpl_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(tpl_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("tpl".to_string(), 1));
    };
    let gtmpl::Value::String(tpl_str) = tpl_val else {
        return Err(gtmpl::FuncError::Generic(
            "tpl expects a string template".to_string(),
        ));
    };
    let ctx = args.get(1).unwrap_or(&gtmpl::Value::Nil);
    let res = render_template_string(tpl_str, ctx)
        .map_err(|e| gtmpl::FuncError::Generic(format!("tpl render failed: {e}")))?;
    Ok(gtmpl::Value::String(res))
}
pub(super) fn include_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(tpl_name_val) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("include".to_string(), 2));
    };
    let gtmpl::Value::String(tpl_name) = tpl_name_val else {
        return Err(gtmpl::FuncError::Generic(
            "include expects a string template name".to_string(),
        ));
    };
    let Some(ctx) = args.get(1) else {
        return Err(gtmpl::FuncError::AtLeastXArgs("include".to_string(), 2));
    };
    let tpl_content = get_templates_registry()
        .read()
        .ok()
        .and_then(|reg| reg.get(tpl_name).cloned());
    if let Some(content) = tpl_content {
        let res = render_template_string(&content, ctx)
            .map_err(|e| gtmpl::FuncError::Generic(format!("include '{tpl_name}' failed: {e}")))?;
        Ok(gtmpl::Value::String(res))
    } else {
        Err(gtmpl::FuncError::Generic(format!(
            "template '{tpl_name}' not found in registry"
        )))
    }
}
