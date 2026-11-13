//! Comprehensive integration tests for `X.509` certificate generation, 99-year defaults,
//! configurable multi-year durations, `CA` ceiling clamping, `RFC` 5480 `ECDSA` `KeyUsage`,
//! atomic `0600` private key permissions, and `IfNotPresent`/`None` policies.

use rtkk::cert::{
    CertPolicy, DEFAULT_CERT_VALIDITY_HOURS, GenCertArgs, MAX_CERT_VALIDITY_HOURS,
    check_cert_files, generate_certificate, is_cert_valid, parse_duration_hours,
    parse_duration_value, verify_certificate_files,
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("rtkk-test-{prefix}-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        Ok(Self { path })
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn ensure(cond: bool, msg: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.to_string()) }
}

#[test]
fn duration_parsing_supports_99y_default_and_1y_or_multiyear() -> Result<(), String> {
    ensure(
        DEFAULT_CERT_VALIDITY_HOURS == 867_240,
        "default must remain 99y (867240h)",
    )?;
    ensure(
        parse_duration_hours("99y")? == 867_240,
        "99y must equal 867240h",
    )?;
    ensure(parse_duration_hours("1y")? == 8_760, "1y must equal 8760h")?;
    ensure(
        parse_duration_hours("10y")? == 87_600,
        "10y must equal 87600h",
    )?;
    ensure(
        parse_duration_hours("365d")? == 8_760,
        "365d must equal 8760h",
    )?;
    ensure(
        parse_duration_hours("8760h")? == 8_760,
        "8760h must equal 8760",
    )?;
    ensure(
        parse_duration_hours("24")? == 24,
        "bare integer hours must parse",
    )?;
    ensure(
        parse_duration_value(&serde_json::json!(8760))? == 8_760,
        "JSON numeric hours must parse",
    )?;
    ensure(
        parse_duration_value(&serde_json::json!("3y"))? == 26_280,
        "JSON string 3y must parse",
    )?;
    ensure(
        parse_duration_hours("0h").is_err(),
        "0h duration must be rejected",
    )?;
    ensure(
        parse_duration_hours(&format!("{}h", MAX_CERT_VALIDITY_HOURS.saturating_add(1))).is_err(),
        "duration exceeding 250y Go time.Duration safety cap must be rejected",
    )?;
    Ok(())
}

#[test]
fn ca_and_leaf_99_year_generalized_time_and_permissions() -> Result<(), String> {
    let dir = TestDir::new("cert-99y")?;
    let ca_key = dir.path.join("ca.key");
    let ca_cert = dir.path.join("ca.crt");
    let ca_args = GenCertArgs {
        cn: "rtkk-root-ca".to_string(),
        out_key: ca_key.clone(),
        out_cert: ca_cert.clone(),
        root_key: None,
        root_cert: None,
        sans: Vec::new(),
        is_ca: true,
        date_hours: DEFAULT_CERT_VALIDITY_HOURS,
        policy: CertPolicy::Always,
    };
    let ca_created = generate_certificate(&ca_args).map_err(|e| e.to_string())?;
    ensure(ca_created, "CA certificate should report changed=true")?;

    #[cfg(unix)]
    {
        let key_mode = std::fs::metadata(&ca_key)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o777;
        ensure(
            key_mode == 0o600,
            &format!("expected private key mode 0600, got {key_mode:04o}"),
        )?;
    }

    let leaf_key = dir.path.join("apiserver.key");
    let leaf_cert = dir.path.join("apiserver.crt");
    let leaf_args = GenCertArgs {
        cn: "kube-apiserver".to_string(),
        out_key: leaf_key.clone(),
        out_cert: leaf_cert.clone(),
        root_key: Some(ca_key.clone()),
        root_cert: Some(ca_cert.clone()),
        sans: vec![
            "kubernetes.default.svc".to_string(),
            "10.0.0.10".to_string(),
            "::1".to_string(),
        ],
        is_ca: false,
        date_hours: DEFAULT_CERT_VALIDITY_HOURS,
        policy: CertPolicy::IfNotPresent,
    };
    let leaf_created = generate_certificate(&leaf_args).map_err(|e| e.to_string())?;
    ensure(leaf_created, "initial leaf generation must return true")?;

    let leaf_pem = std::fs::read_to_string(&leaf_cert).map_err(|e| e.to_string())?;
    ensure(is_cert_valid(&leaf_pem), "99y leaf cert must be valid")?;
    ensure(
        check_cert_files(&leaf_key, &leaf_cert)?,
        "check_cert_files must confirm keypair match",
    )?;

    let (_, pem_obj) =
        x509_parser::pem::parse_x509_pem(leaf_pem.as_bytes()).map_err(|e| e.to_string())?;
    let parsed = pem_obj.parse_x509().map_err(|e| e.to_string())?;
    if let Ok(Some(ku_ext)) = parsed.key_usage() {
        let ku = ku_ext.value;
        ensure(
            ku.digital_signature(),
            "ECDSA leaf cert must set DigitalSignature",
        )?;
        ensure(
            !ku.key_encipherment(),
            "ECDSA P-256 leaf cert must NOT set KeyEncipherment per RFC 5480",
        )?;
    } else {
        return Err("missing KeyUsage extension on leaf cert".to_string());
    }

    let report = verify_certificate_files(&leaf_cert, Some(&leaf_key), Some(&ca_cert))
        .map_err(|e| e.to_string())?;
    ensure(!report.is_ca, "leaf must not be marked CA")?;
    ensure(
        report.key_matched == Some(true),
        "verify report must confirm key match",
    )?;
    ensure(
        report.ca_verified == Some(true),
        "verify report must confirm CA signature",
    )?;
    ensure(
        report.sans.contains(&"10.0.0.10".to_string()),
        "SANs must contain 10.0.0.10",
    )?;

    // Re-running with IfNotPresent and identical SANs must skip regeneration.
    let second_run = generate_certificate(&leaf_args).map_err(|e| e.to_string())?;
    ensure(
        !second_run,
        "IfNotPresent must skip when cert, key, CA, and SANs match",
    )?;

    // Adding a new SAN under IfNotPresent must regenerate!
    let mut expanded_args = leaf_args;
    expanded_args.sans.push("10.0.0.99".to_string());
    let third_run = generate_certificate(&expanded_args).map_err(|e| e.to_string())?;
    ensure(
        third_run,
        "IfNotPresent must regenerate when requested SANs expand",
    )?;
    Ok(())
}

#[test]
fn leaf_validity_honors_1y_and_clamps_to_shorter_ca_ceiling() -> Result<(), String> {
    let dir = TestDir::new("cert-clamp")?;
    let ca_key = dir.path.join("short-ca.key");
    let ca_cert = dir.path.join("short-ca.crt");

    let ca_args = GenCertArgs {
        cn: "one-year-ca".to_string(),
        out_key: ca_key.clone(),
        out_cert: ca_cert.clone(),
        root_key: None,
        root_cert: None,
        sans: Vec::new(),
        is_ca: true,
        date_hours: 8_760,
        policy: CertPolicy::Always,
    };
    generate_certificate(&ca_args).map_err(|e| e.to_string())?;

    let leaf_key = dir.path.join("clamped-leaf.key");
    let leaf_cert = dir.path.join("clamped-leaf.crt");
    let leaf_args = GenCertArgs {
        cn: "clamped-leaf".to_string(),
        out_key: leaf_key.clone(),
        out_cert: leaf_cert.clone(),
        root_key: Some(ca_key),
        root_cert: Some(ca_cert.clone()),
        sans: vec!["localhost".to_string()],
        is_ca: false,
        date_hours: DEFAULT_CERT_VALIDITY_HOURS,
        policy: CertPolicy::Always,
    };
    generate_certificate(&leaf_args).map_err(|e| e.to_string())?;

    let ca_pem = std::fs::read_to_string(&ca_cert).map_err(|e| e.to_string())?;
    let leaf_pem = std::fs::read_to_string(&leaf_cert).map_err(|e| e.to_string())?;
    let (_, ca_p) =
        x509_parser::pem::parse_x509_pem(ca_pem.as_bytes()).map_err(|e| e.to_string())?;
    let (_, leaf_p) =
        x509_parser::pem::parse_x509_pem(leaf_pem.as_bytes()).map_err(|e| e.to_string())?;
    let ca_x509 = ca_p.parse_x509().map_err(|e| e.to_string())?;
    let leaf_x509 = leaf_p.parse_x509().map_err(|e| e.to_string())?;

    let ca_end = ca_x509.validity().not_after.timestamp();
    let leaf_end = leaf_x509.validity().not_after.timestamp();
    ensure(
        leaf_end <= ca_end,
        &format!("leaf not_after ({leaf_end}) exceeded CA not_after ({ca_end})"),
    )?;

    let bad_args = GenCertArgs {
        cn: "bad".to_string(),
        out_key: dir.path.join("bad.key"),
        out_cert: dir.path.join("bad.crt"),
        root_key: Some(leaf_key),
        root_cert: None,
        sans: Vec::new(),
        is_ca: false,
        date_hours: 8_760,
        policy: CertPolicy::Always,
    };
    ensure(
        generate_certificate(&bad_args).is_err(),
        "asymmetric root_key without root_cert must fail",
    )?;
    Ok(())
}
