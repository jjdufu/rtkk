//! Integration tests for `Go`-template helper functions, `NoValue` handling,
//! `b64enc`, `semverCompare`, `index` negative bounds, `int`/`sub`, `CIDR` helpers, and `include`.

use rtkk::template::{json_to_gtmpl, register_template, render_template_string};

fn ensure(cond: bool, msg: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.to_string()) }
}

fn render(tmpl: &str, json_ctx: &serde_json::Value) -> Result<String, String> {
    let ctx = json_to_gtmpl(json_ctx);
    render_template_string(tmpl, &ctx)
}

#[test]
fn novalue_handling_in_default_empty_not_ternary_and_eq() -> Result<(), String> {
    let ctx = serde_json::json!({
        "present_str": "hello",
        "empty_str": "",
        "flag_true": true
    });

    ensure(
        render(r#"{{ default "fallback" .missing_key }}"#, &ctx)? == "fallback",
        "default must return fallback when key evaluates to NoValue",
    )?;
    ensure(
        render(r#"{{ default "fallback" .empty_str }}"#, &ctx)? == "fallback",
        "default must return fallback for empty string",
    )?;
    ensure(
        render(r#"{{ default "fallback" .present_str }}"#, &ctx)? == "hello",
        "default must return present value",
    )?;
    ensure(
        render("{{ empty .missing_key }}", &ctx)? == "true",
        "empty must be true on NoValue",
    )?;
    ensure(
        render("{{ not .missing_key }}", &ctx)? == "true",
        "not must be true on NoValue",
    )?;
    ensure(
        render(r#"{{ ternary "yes" "no" .missing_key }}"#, &ctx)? == "no",
        "ternary must select false branch on NoValue",
    )?;
    ensure(
        render(r#"{{ ternary "yes" "no" .flag_true }}"#, &ctx)? == "yes",
        "ternary must select true branch on true",
    )?;
    ensure(
        render("{{ eq .missing_a .missing_b }}", &ctx)? == "true",
        "two NoValue fields must compare equal",
    )?;
    Ok(())
}

#[test]
fn b64enc_semver_math_index_and_include_helpers() -> Result<(), String> {
    let ctx = serde_json::json!({
        "user": "admin",
        "items": ["alpha", "beta", "gamma"],
        "ver": "v1.35.6",
        "float_num": 42.9,
        "subnet": "10.233.64.0/18",
        "dns_ip": "10.233.64.10"
    });

    ensure(
        render(r#"{{ b64enc "admin:change-me" }}"#, &ctx)? == "YWRtaW46Y2hhbmdlLW1l",
        "b64enc output mismatch",
    )?;
    ensure(
        render(r#"{{ semverCompare ">= v1.30.0, < v2.0.0" .ver }}"#, &ctx)? == "true",
        "semverCompare must strip 'v' prefix in both constraint and version",
    )?;
    ensure(
        render("{{ int .float_num }}", &ctx)? == "42",
        "int helper must truncate floating-point numbers cleanly",
    )?;
    ensure(
        render("{{ sub 5 12 }}", &ctx)? == "-7",
        "sub helper must support signed negative results without u64 underflow",
    )?;
    ensure(
        render("{{ index .items 1 }}", &ctx)? == "beta",
        "index helper must read array element 1",
    )?;
    ensure(
        render("{{ index .items -1 }}", &ctx).is_err(),
        "negative array index must return Err instead of silently returning element 0",
    )?;
    ensure(
        render("{{ ipInCIDR .dns_ip .subnet }}", &ctx)? == "true",
        "ipInCIDR must detect 10.233.64.10 inside 10.233.64.0/18",
    )?;
    ensure(
        render("{{ ipFamily .dns_ip }}", &ctx)? == "IPv4",
        "ipFamily must return IPv4",
    )?;

    register_template("unit_snippet.gtmpl", "hello-{{ .user }}")?;
    ensure(
        render(r#"{{ include "unit_snippet.gtmpl" . }}"#, &ctx)? == "hello-admin",
        "include helper must render registered named template",
    )?;

    // Broken template syntax must fail loudly.
    ensure(
        render("{{ .unclosed", &ctx).is_err(),
        "unclosed template expression must return Err",
    )?;
    Ok(())
}
