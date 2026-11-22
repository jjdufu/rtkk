//! Miscellaneous template helpers (`SemVer` comparison, filesystem checks, and template inclusion).

use crate::template::{lookup_named_template, render_template_string};

fn strip_v_prefix(s: &str) -> &str {
    s.strip_prefix('v')
        .or_else(|| s.strip_prefix('V'))
        .unwrap_or(s)
}

fn normalize_semver_clause(clause: &str) -> String {
    const OPS: &[&str] = &[">=", "<=", "!=", ">", "<", "=", "~", "^"];
    let trimmed = clause.trim();
    for op in OPS {
        if let Some(rest) = trimmed.strip_prefix(op) {
            let ver = strip_v_prefix(rest.trim());
            return format!("{op}{ver}");
        }
    }
    let ver = strip_v_prefix(trimmed);
    if ver.starts_with(|c: char| c.is_ascii_digit())
        && !ver.contains('*')
        && !ver.contains('x')
        && !ver.contains('X')
    {
        format!("={ver}")
    } else {
        ver.to_string()
    }
}

fn normalize_semver_constraint(raw: &str) -> String {
    let parts: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(normalize_semver_clause)
        .collect();
    parts.join(", ")
}

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
    let clean_ver = strip_v_prefix(ver_str.trim());
    let parsed_version = semver::Version::parse(clean_ver).map_err(|e| {
        gtmpl::FuncError::Generic(format!("invalid semver version '{ver_str}': {e}"))
    })?;
    let normalized_constraint = normalize_semver_constraint(constraint_str);
    let requirement = semver::VersionReq::parse(&normalized_constraint).map_err(|e| {
        gtmpl::FuncError::Generic(format!("invalid semver constraint '{constraint_str}': {e}"))
    })?;
    Ok(gtmpl::Value::Bool(requirement.matches(&parsed_version)))
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
    let tpl_content = lookup_named_template(tpl_name).map_err(gtmpl::FuncError::Generic)?;
    let Some(content) = tpl_content else {
        return Err(gtmpl::FuncError::Generic(format!(
            "template '{tpl_name}' not found in registry"
        )));
    };
    let res = render_template_string(&content, ctx)
        .map_err(|e| gtmpl::FuncError::Generic(format!("include '{tpl_name}' failed: {e}")))?;
    Ok(gtmpl::Value::String(res))
}

pub(super) fn duration_hours_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "durationHours".to_string(),
            1,
        ));
    };
    let hours = match first {
        gtmpl::Value::String(s) => {
            if s.trim().is_empty() {
                crate::cert::DEFAULT_CERT_VALIDITY_HOURS
            } else {
                crate::cert::parse_duration_hours(s).map_err(gtmpl::FuncError::Generic)?
            }
        }
        gtmpl::Value::Number(n) => {
            let raw = n.as_u64().ok_or_else(|| {
                gtmpl::FuncError::Generic("durationHours expects a positive integer".to_string())
            })?;
            crate::cert::parse_duration_hours(&raw.to_string())
                .map_err(gtmpl::FuncError::Generic)?
        }
        gtmpl::Value::Nil | gtmpl::Value::NoValue => crate::cert::DEFAULT_CERT_VALIDITY_HOURS,
        _ => {
            return Err(gtmpl::FuncError::Generic(
                "durationHours expects a duration string or integer hours".to_string(),
            ));
        }
    };
    Ok(gtmpl::Value::String(format!("{hours}h")))
}
