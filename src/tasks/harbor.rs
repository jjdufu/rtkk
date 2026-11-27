//! Built-in playbook task module `ensure_harbor_projects` for idempotent `Harbor` mirror project setup.

use super::{Task, TaskModuleError, TaskResult, render_value};
use crate::registry::{
    DEFAULT_FALLBACK_MIRROR_PROJECT, RegistryConfig, ensure_harbor_mirror_projects,
};
use serde_json::Value;

fn parse_strict_flag(val: Option<&Value>) -> bool {
    match val {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn collect_projects_from_mirrors(mirrors_val: &Value) -> Result<Vec<String>, TaskModuleError> {
    let Some(arr) = mirrors_val.as_array() else {
        return Err(TaskModuleError::Message(
            "ensure_harbor_projects: mirrors source must be a YAML/JSON array".to_string(),
        ));
    };
    let mut projects = Vec::with_capacity(arr.len());
    for entry in arr {
        if let Some(proj) = entry.get("project").and_then(Value::as_str) {
            projects.push(proj.to_string());
        } else if let Some(proj) = entry.as_str() {
            projects.push(proj.to_string());
        } else {
            return Err(TaskModuleError::Message(
                "ensure_harbor_projects: each mirror entry must be an object with 'project' or a project name string".to_string(),
            ));
        }
    }
    Ok(projects)
}

fn resolve_default_fallback_project(rendered: &Value, host_vars: &Value) -> Option<String> {
    if let Some(explicit) = rendered.get("default_project") {
        return explicit
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
    }
    let var_name = rendered
        .get("default_project_var")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("public_registry_default_mirror_project");
    match host_vars.get(var_name) {
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None | Some(Value::Null) => Some(DEFAULT_FALLBACK_MIRROR_PROJECT.to_string()),
        _ => None,
    }
}

pub(crate) async fn execute_ensure_harbor_projects(
    task: &Task,
    host_vars: &Value,
    context: &gtmpl::Value,
) -> Result<Option<TaskResult>, TaskModuleError> {
    let Some(ref raw_cfg) = task.ensure_harbor_projects else {
        return Ok(None);
    };
    let rendered = render_value(raw_cfg, context)?;
    let reg_var_name = rendered
        .get("registry_var")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("registry");
    let reg_val = rendered
        .get("registry")
        .filter(|v| v.is_object())
        .or_else(|| host_vars.get(reg_var_name))
        .ok_or_else(|| {
            TaskModuleError::Message(format!(
                "ensure_harbor_projects: missing registry object variable '{reg_var_name}'"
            ))
        })?;
    let registry: RegistryConfig = serde_json::from_value(reg_val.clone()).map_err(|e| {
        TaskModuleError::Message(format!(
            "ensure_harbor_projects: invalid registry configuration in '{reg_var_name}': {e}"
        ))
    })?;

    let mut projects = if let Some(explicit) = rendered.get("projects") {
        collect_projects_from_mirrors(explicit)?
    } else {
        let mirrors_var_name = rendered
            .get("mirrors_var")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("public_registry_mirrors");
        match host_vars.get(mirrors_var_name) {
            Some(val) if !val.is_null() => collect_projects_from_mirrors(val)?,
            _ => Vec::new(),
        }
    };

    if rendered.get("projects").is_none()
        && let Some(default_proj) = resolve_default_fallback_project(&rendered, host_vars)
    {
        projects.push(default_proj);
    }

    let strict = parse_strict_flag(rendered.get("strict"));
    let outcome = ensure_harbor_mirror_projects(&registry, &projects, strict)
        .await
        .map_err(TaskModuleError::Message)?;

    let summary = format!(
        "Harbor mirror projects ensured at {} (created={}, existing={}, warnings={})",
        outcome.base_url,
        outcome.created.len(),
        outcome.existing.len(),
        outcome.warnings.len()
    );
    Ok(Some(TaskResult {
        stdout: summary,
        stderr: outcome.warnings.join("\n"),
        exit_code: 0,
        changed: !outcome.created.is_empty(),
        failed: false,
    }))
}
