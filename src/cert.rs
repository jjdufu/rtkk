//! `X.509` certificate generation, policy enforcement, and verification utilities.

use crate::connector::ConnectorError;
use std::fmt;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

/// Default certificate validity duration in hours (99 years: `99 * 8760`).
pub const DEFAULT_CERT_VALIDITY_HOURS: u64 = 867_240;

/// Maximum allowed certificate validity duration in hours (250 years: `250 * 8760`).
pub const MAX_CERT_VALIDITY_HOURS: u64 = 2_190_000;

/// Parses a human-readable duration string (such as `"867240h"`, `"365d"`, `"99y"`, or `"8760"`)
/// into hours within `1..=MAX_CERT_VALIDITY_HOURS`.
///
/// # Errors
///
/// Returns an error if `raw` is empty, cannot be parsed, overflows `u64`, is `0`,
/// or exceeds [`MAX_CERT_VALIDITY_HOURS`].
pub fn parse_duration_hours(raw: &str) -> Result<u64, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("certificate duration cannot be empty".to_string());
    }
    let (num_str, multiplier) = if let Some(s) = trimmed
        .strip_suffix('h')
        .or_else(|| trimmed.strip_suffix('H'))
    {
        (s.trim(), 1_u64)
    } else if let Some(s) = trimmed
        .strip_suffix('d')
        .or_else(|| trimmed.strip_suffix('D'))
    {
        (s.trim(), 24_u64)
    } else if let Some(s) = trimmed
        .strip_suffix('y')
        .or_else(|| trimmed.strip_suffix('Y'))
    {
        (s.trim(), 8_760_u64)
    } else {
        (trimmed, 1_u64)
    };
    let base = num_str
        .parse::<u64>()
        .map_err(|e| format!("invalid certificate duration '{raw}': {e}"))?;
    let hours = base
        .checked_mul(multiplier)
        .ok_or_else(|| format!("certificate duration '{raw}' overflows u64 hours"))?;
    validate_duration_range(hours)
}

/// Parses a `JSON`/`YAML` duration value (integer hours or duration string) into hours.
///
/// # Errors
///
/// Returns an error if `val` is neither a string nor a non-negative integer, or falls
/// outside `1..=MAX_CERT_VALIDITY_HOURS`.
pub fn parse_duration_value(val: &serde_json::Value) -> Result<u64, String> {
    match val {
        serde_json::Value::String(s) => parse_duration_hours(s),
        serde_json::Value::Number(n) => {
            let Some(hours) = n.as_u64() else {
                return Err(format!("invalid numeric certificate duration: {n}"));
            };
            validate_duration_range(hours)
        }
        _ => Err("certificate duration must be a string or positive integer".to_string()),
    }
}

fn validate_duration_range(hours: u64) -> Result<u64, String> {
    if hours == 0 {
        return Err("certificate duration must be at least 1 hour".to_string());
    }
    if hours > MAX_CERT_VALIDITY_HOURS {
        return Err(format!(
            "certificate duration {hours}h exceeds maximum {MAX_CERT_VALIDITY_HOURS}h"
        ));
    }
    Ok(hours)
}

/// Policy controlling how existing certificate and private key files are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CertPolicy {
    /// Always generate and overwrite the target private key and certificate.
    Always,
    /// Reuse existing files if present, valid, matching, and compatible with requested parameters.
    IfNotPresent,
    /// Never generate; verify that valid matching certificate and key files already exist.
    None,
}

impl CertPolicy {
    /// Parses a certificate policy name case-insensitively (`"always"`, `"ifnotpresent"`,
    /// `"if_not_present"`, or `"none"`).
    ///
    /// # Errors
    ///
    /// Returns an error if `s` does not match a known policy variant.
    pub fn from_str_policy(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "always" => Ok(Self::Always),
            "ifnotpresent" | "if_not_present" => Ok(Self::IfNotPresent),
            "none" => Ok(Self::None),
            _ => Err(format!("unknown cert policy: {s}")),
        }
    }
}

impl<'de> serde::Deserialize<'de> for CertPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str_policy(&s).map_err(serde::de::Error::custom)
    }
}

/// Parameters for generating or validating an `X.509` certificate and `ECDSA` `P-256` key pair.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct GenCertArgs {
    /// Subject Common Name (`CN`) for the certificate.
    pub cn: String,
    /// Destination path for the `PEM`-encoded private key.
    pub out_key: PathBuf,
    /// Destination path for the `PEM`-encoded certificate.
    pub out_cert: PathBuf,
    /// Optional `CA` private key path used to sign the certificate.
    pub root_key: Option<PathBuf>,
    /// Optional `CA` certificate path used to sign the certificate.
    pub root_cert: Option<PathBuf>,
    /// Subject Alternative Names (`DNS` names and `IPv4`/`IPv6` addresses).
    #[serde(default, deserialize_with = "deserialize_sans")]
    pub sans: Vec<String>,
    /// Whether to issue a Certificate Authority (`CA`) certificate.
    #[serde(default)]
    pub is_ca: bool,
    /// Requested validity duration in hours (defaults to [`DEFAULT_CERT_VALIDITY_HOURS`]).
    #[serde(
        alias = "date",
        deserialize_with = "deserialize_duration_hours",
        default = "default_date_hours"
    )]
    pub date_hours: u64,
    /// Certificate generation policy.
    pub policy: CertPolicy,
}

/// Verification report describing an inspected `X.509` certificate and optional key/`CA` chain.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CertVerifyReport {
    /// Subject Distinguished Name string.
    pub subject: String,
    /// Issuer Distinguished Name string.
    pub issuer: String,
    /// Formatted `notBefore` validity timestamp.
    pub not_before: String,
    /// Formatted `notAfter` validity timestamp.
    pub not_after: String,
    /// Whether `BasicConstraints` marks this certificate as a `CA`.
    pub is_ca: bool,
    /// Extracted Subject Alternative Names (`DNS` names and `IPv4`/`IPv6` addresses).
    pub sans: Vec<String>,
    /// Whether the supplied private key matched the certificate public key.
    pub key_matched: Option<bool>,
    /// Whether the certificate was verified against the supplied `CA` certificate.
    pub ca_verified: Option<bool>,
}

impl fmt::Display for CertVerifyReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Subject:    {}", self.subject)?;
        writeln!(f, "Issuer:     {}", self.issuer)?;
        writeln!(f, "Not Before: {}", self.not_before)?;
        writeln!(f, "Not After:  {}", self.not_after)?;
        writeln!(f, "Is CA:      {}", self.is_ca)?;
        if !self.sans.is_empty() {
            writeln!(f, "SANs:       {}", self.sans.join(", "))?;
        }
        if let Some(km) = self.key_matched {
            writeln!(f, "Key Match:  {km}")?;
        }
        if let Some(cv) = self.ca_verified {
            writeln!(f, "CA Valid:   {cv}")?;
        }
        Ok(())
    }
}

const fn default_date_hours() -> u64 {
    DEFAULT_CERT_VALIDITY_HOURS
}

struct DurationVisitor;

impl serde::de::Visitor<'_> for DurationVisitor {
    type Value = u64;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a duration string (e.g., '867240h', '365d', '99y') or positive hours")
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        validate_duration_range(v).map_err(E::custom)
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let val = u64::try_from(v).map_err(|e| E::custom(format!("invalid integer: {e}")))?;
        validate_duration_range(val).map_err(E::custom)
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        parse_duration_hours(v).map_err(E::custom)
    }
}

fn deserialize_duration_hours<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_any(DurationVisitor)
}

struct SansVisitor;

impl<'de> serde::de::Visitor<'de> for SansVisitor {
    type Value = Vec<String>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a sequence of strings or a JSON-encoded list of strings")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Vec::new())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Vec::new())
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut vec = Vec::new();
        while let Some(elem) = seq.next_element()? {
            vec.push(elem);
        }
        Ok(vec)
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            serde_json::from_str::<Vec<String>>(trimmed)
                .map_err(|e| E::custom(format!("failed to parse SANs JSON string: {e}")))
        } else {
            Ok(vec![v.to_string()])
        }
    }
}

fn deserialize_sans<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_any(SansVisitor)
}

/// Checks whether `pem_str` contains a syntactically valid `X.509` certificate that is
/// currently within its `notBefore`..=`notAfter` validity window.
#[must_use]
pub fn is_cert_valid(pem_str: &str) -> bool {
    let Ok((_, pem)) = x509_parser::pem::parse_x509_pem(pem_str.as_bytes()) else {
        return false;
    };
    let Ok(cert) = pem.parse_x509() else {
        return false;
    };
    cert.validity().is_valid()
}

/// Checks whether both `key_path` and `cert_path` exist, are currently valid, and form a
/// matching key pair.
///
/// # Errors
///
/// Returns an error if reading an existing certificate or key file fails due to filesystem permissions
/// or `I/O` errors.
pub fn check_cert_files(key_path: &Path, cert_path: &Path) -> Result<bool, String> {
    if !key_path.exists() || !cert_path.exists() {
        return Ok(false);
    }
    let cert_content = std::fs::read_to_string(cert_path)
        .map_err(|e| format!("Failed to read certificate: {e}"))?;
    let key_content = std::fs::read_to_string(key_path)
        .map_err(|e| format!("Failed to read private key: {e}"))?;
    let Ok((_, pem)) = x509_parser::pem::parse_x509_pem(cert_content.as_bytes()) else {
        return Ok(false);
    };
    let Ok(cert) = pem.parse_x509() else {
        return Ok(false);
    };
    if !cert.validity().is_valid() {
        return Ok(false);
    }
    let Ok(key_pair) = rcgen::KeyPair::from_pem(&key_content) else {
        return Ok(false);
    };
    Ok(key_pair.public_key_raw() == cert.public_key().subject_public_key.data.as_ref())
}

/// Verifies an `X.509` certificate file along with an optional private key and optional `CA`
/// certificate, returning a structured [`CertVerifyReport`].
///
/// # Errors
///
/// Returns [`ConnectorError`] if any specified file cannot be read or parsed, if the certificate
/// or `CA` is expired/not-yet-valid, if the private key does not match the certificate's public
/// key, or if the certificate's issuer/signature does not match the `CA`.
pub fn verify_certificate_files(
    cert_path: &Path,
    key_path: Option<&Path>,
    ca_cert_path: Option<&Path>,
) -> Result<CertVerifyReport, ConnectorError> {
    let cert_pem = std::fs::read_to_string(cert_path).map_err(|e| {
        ConnectorError::IOError(format!(
            "Failed to read certificate {}: {e}",
            cert_path.display()
        ))
    })?;
    let (_, pem_obj) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()).map_err(|e| {
        ConnectorError::IOError(format!(
            "Failed to parse certificate PEM {}: {e}",
            cert_path.display()
        ))
    })?;
    let cert = pem_obj.parse_x509().map_err(|e| {
        ConnectorError::IOError(format!(
            "Failed to parse X.509 certificate {}: {e}",
            cert_path.display()
        ))
    })?;
    if !cert.validity().is_valid() {
        return Err(ConnectorError::AuthenticationFailed(format!(
            "Certificate is expired or not yet valid: {}",
            cert_path.display()
        )));
    }

    let key_matched = if let Some(kpath) = key_path {
        verify_key_matches_cert(kpath, &cert)?;
        Some(true)
    } else {
        None
    };

    let ca_verified = if let Some(cpath) = ca_cert_path {
        verify_cert_against_ca(&cert, cpath)?;
        Some(true)
    } else {
        None
    };

    Ok(CertVerifyReport {
        subject: cert.subject().to_string(),
        issuer: cert.issuer().to_string(),
        not_before: cert.validity().not_before.to_string(),
        not_after: cert.validity().not_after.to_string(),
        is_ca: cert.is_ca(),
        sans: extract_cert_sans(&cert),
        key_matched,
        ca_verified,
    })
}

fn verify_key_matches_cert(
    key_path: &Path,
    cert: &x509_parser::certificate::X509Certificate<'_>,
) -> Result<(), ConnectorError> {
    let key_pem = std::fs::read_to_string(key_path).map_err(|e| {
        ConnectorError::IOError(format!(
            "Failed to read private key {}: {e}",
            key_path.display()
        ))
    })?;
    let key_pair = rcgen::KeyPair::from_pem(&key_pem).map_err(|e| {
        ConnectorError::IOError(format!(
            "Failed to parse private key {}: {e}",
            key_path.display()
        ))
    })?;
    if key_pair.public_key_raw() != cert.public_key().subject_public_key.data.as_ref() {
        return Err(ConnectorError::AuthenticationFailed(format!(
            "Private key {} does not match certificate public key",
            key_path.display()
        )));
    }
    Ok(())
}

fn verify_cert_against_ca(
    cert: &x509_parser::certificate::X509Certificate<'_>,
    ca_path: &Path,
) -> Result<(), ConnectorError> {
    let ca_pem = std::fs::read_to_string(ca_path).map_err(|e| {
        ConnectorError::IOError(format!(
            "Failed to read CA certificate {}: {e}",
            ca_path.display()
        ))
    })?;
    let (_, ca_pem_obj) = x509_parser::pem::parse_x509_pem(ca_pem.as_bytes()).map_err(|e| {
        ConnectorError::IOError(format!(
            "Failed to parse CA certificate PEM {}: {e}",
            ca_path.display()
        ))
    })?;
    let ca_cert = ca_pem_obj.parse_x509().map_err(|e| {
        ConnectorError::IOError(format!(
            "Failed to parse CA X.509 certificate {}: {e}",
            ca_path.display()
        ))
    })?;
    if !ca_cert.validity().is_valid() {
        return Err(ConnectorError::AuthenticationFailed(format!(
            "CA certificate is expired or not yet valid: {}",
            ca_path.display()
        )));
    }
    if !ca_cert.is_ca() {
        return Err(ConnectorError::AuthenticationFailed(format!(
            "Root certificate is not marked as a CA: {}",
            ca_path.display()
        )));
    }
    if cert.issuer() != ca_cert.subject() {
        return Err(ConnectorError::AuthenticationFailed(
            "Certificate issuer does not match CA subject".to_string(),
        ));
    }
    cert.verify_signature(Some(ca_cert.public_key()))
        .map_err(|e| {
            ConnectorError::AuthenticationFailed(format!(
                "Certificate signature verification against CA failed: {e}"
            ))
        })?;
    Ok(())
}

fn extract_cert_sans(cert: &x509_parser::certificate::X509Certificate<'_>) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(Some(san_ext)) = cert.subject_alternative_name() else {
        return out;
    };
    for gn in &san_ext.value.general_names {
        match gn {
            x509_parser::extensions::GeneralName::DNSName(dns) => {
                out.push((*dns).to_string());
            }
            x509_parser::extensions::GeneralName::IPAddress(bytes) => {
                if let Ok(v4) = <[u8; 4]>::try_from(*bytes) {
                    out.push(std::net::IpAddr::from(v4).to_string());
                } else if let Ok(v6) = <[u8; 16]>::try_from(*bytes) {
                    out.push(std::net::IpAddr::from(v6).to_string());
                }
            }
            _ => {}
        }
    }
    out
}

fn cert_has_all_sans(
    cert: &x509_parser::certificate::X509Certificate<'_>,
    requested_sans: &[String],
) -> bool {
    if requested_sans.is_empty() {
        return true;
    }
    let Ok(Some(san_ext)) = cert.subject_alternative_name() else {
        return false;
    };
    let mut cert_dns = Vec::new();
    let mut cert_ips = Vec::new();
    for gn in &san_ext.value.general_names {
        match gn {
            x509_parser::extensions::GeneralName::DNSName(dns) => cert_dns.push(*dns),
            x509_parser::extensions::GeneralName::IPAddress(bytes) => {
                if let Ok(v4) = <[u8; 4]>::try_from(*bytes) {
                    cert_ips.push(std::net::IpAddr::from(v4));
                } else if let Ok(v6) = <[u8; 16]>::try_from(*bytes) {
                    cert_ips.push(std::net::IpAddr::from(v6));
                }
            }
            _ => {}
        }
    }
    for req in requested_sans {
        if let Ok(req_ip) = req.parse::<std::net::IpAddr>() {
            if !cert_ips.contains(&req_ip) {
                return false;
            }
        } else if !cert_dns.iter().any(|d| d.eq_ignore_ascii_case(req)) {
            return false;
        }
    }
    true
}

fn check_existing_for_if_not_present(args: &GenCertArgs) -> Result<bool, ConnectorError> {
    if !check_cert_files(&args.out_key, &args.out_cert).map_err(ConnectorError::IOError)? {
        return Ok(false);
    }
    let cert_pem = std::fs::read_to_string(&args.out_cert)
        .map_err(|e| ConnectorError::IOError(format!("Failed to read certificate: {e}")))?;
    let Ok((_, pem_obj)) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes()) else {
        return Ok(false);
    };
    let Ok(cert) = pem_obj.parse_x509() else {
        return Ok(false);
    };
    if cert.is_ca() != args.is_ca {
        return Ok(false);
    }
    if !cert_has_all_sans(&cert, &args.sans) {
        return Ok(false);
    }
    if let Some(ref ca_path) = args.root_cert {
        return Ok(verify_cert_against_ca(&cert, ca_path).is_ok());
    }
    Ok(true)
}

fn apply_policy(args: &GenCertArgs) -> Result<bool, ConnectorError> {
    match args.policy {
        CertPolicy::Always => Ok(false),
        CertPolicy::IfNotPresent => check_existing_for_if_not_present(args),
        CertPolicy::None => {
            if !args.out_key.exists() || !args.out_cert.exists() {
                return Err(ConnectorError::IOError(format!(
                    "Certificate or key file not found under policy None: {} or {}",
                    args.out_key.display(),
                    args.out_cert.display()
                )));
            }
            verify_certificate_files(
                &args.out_cert,
                Some(&args.out_key),
                args.root_cert.as_deref(),
            )?;
            Ok(true)
        }
    }
}

fn clamp_not_after_to_ca(
    not_after: time::OffsetDateTime,
    now: time::OffsetDateTime,
    ca_cert_path: &Path,
) -> Result<time::OffsetDateTime, ConnectorError> {
    let ca_pem = std::fs::read_to_string(ca_cert_path)
        .map_err(|e| ConnectorError::IOError(format!("Failed to read CA cert: {e}")))?;
    let (_, pem_obj) = x509_parser::pem::parse_x509_pem(ca_pem.as_bytes())
        .map_err(|e| ConnectorError::IOError(format!("Failed to parse CA cert PEM: {e}")))?;
    let ca_cert = pem_obj
        .parse_x509()
        .map_err(|e| ConnectorError::IOError(format!("Failed to parse CA X.509 cert: {e}")))?;
    if !ca_cert.validity().is_valid() {
        return Err(ConnectorError::AuthenticationFailed(format!(
            "CA certificate is expired or not yet valid: {}",
            ca_cert_path.display()
        )));
    }
    if !ca_cert.is_ca() {
        return Err(ConnectorError::IOError(format!(
            "Provided root_cert is not a CA certificate: {}",
            ca_cert_path.display()
        )));
    }
    let ca_not_after =
        time::OffsetDateTime::from_unix_timestamp(ca_cert.validity().not_after.timestamp())
            .map_err(|e| ConnectorError::IOError(format!("Invalid CA not_after timestamp: {e}")))?;
    let clamped = if not_after > ca_not_after {
        ca_not_after
    } else {
        not_after
    };
    if clamped <= now {
        return Err(ConnectorError::IOError(
            "Certificate not_after timestamp must be in the future".to_string(),
        ));
    }
    Ok(clamped)
}

fn build_cert_params(
    args: &GenCertArgs,
    duration_hours: u64,
) -> Result<rcgen::CertificateParams, ConnectorError> {
    validate_duration_range(duration_hours).map_err(ConnectorError::IOError)?;
    let mut params = rcgen::CertificateParams::default();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, &args.cn);
    for san in &args.sans {
        if let Ok(ip) = san.parse::<std::net::IpAddr>() {
            params.subject_alt_names.push(rcgen::SanType::IpAddress(ip));
        } else {
            let ia5 = rcgen::string::Ia5String::try_from(san.clone())
                .map_err(|e| ConnectorError::IOError(format!("Invalid DNS name '{san}': {e}")))?;
            params.subject_alt_names.push(rcgen::SanType::DnsName(ia5));
        }
    }
    if args.is_ca {
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages.push(rcgen::KeyUsagePurpose::KeyCertSign);
        params.key_usages.push(rcgen::KeyUsagePurpose::CrlSign);
    } else {
        params
            .key_usages
            .push(rcgen::KeyUsagePurpose::DigitalSignature);
        params
            .extended_key_usages
            .push(rcgen::ExtendedKeyUsagePurpose::ServerAuth);
        params
            .extended_key_usages
            .push(rcgen::ExtendedKeyUsagePurpose::ClientAuth);
    }
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now.checked_sub(time::Duration::hours(1)).ok_or_else(|| {
        ConnectorError::IOError("Timestamp underflow computing not_before".to_string())
    })?;
    let duration_i64 = i64::try_from(duration_hours)
        .map_err(|e| ConnectorError::IOError(format!("Invalid duration value: {e}")))?;
    let requested_not_after = now
        .checked_add(time::Duration::hours(duration_i64))
        .ok_or_else(|| {
            ConnectorError::IOError("Timestamp overflow computing not_after".to_string())
        })?;
    params.not_after = if let Some(ref ca_path) = args.root_cert {
        clamp_not_after_to_ca(requested_not_after, now, ca_path)?
    } else {
        if requested_not_after <= now {
            return Err(ConnectorError::IOError(
                "Certificate not_after timestamp must be in the future".to_string(),
            ));
        }
        requested_not_after
    };
    Ok(params)
}

fn sign_or_self_sign_cert(
    params: rcgen::CertificateParams,
    leaf_key: &rcgen::KeyPair,
    root_key_path: Option<&Path>,
    root_cert_path: Option<&Path>,
) -> Result<(String, String), ConnectorError> {
    match (root_key_path, root_cert_path) {
        (Some(rkey_path), Some(rcert_path)) => {
            let ca_key_pem = std::fs::read_to_string(rkey_path)
                .map_err(|e| ConnectorError::IOError(format!("Failed to read CA key: {e}")))?;
            let ca_cert_pem = std::fs::read_to_string(rcert_path)
                .map_err(|e| ConnectorError::IOError(format!("Failed to read CA cert: {e}")))?;
            let (_, ca_pem_obj) = x509_parser::pem::parse_x509_pem(ca_cert_pem.as_bytes())
                .map_err(|e| {
                    ConnectorError::IOError(format!("Failed to parse CA cert PEM: {e}"))
                })?;
            let ca_x509 = ca_pem_obj
                .parse_x509()
                .map_err(|e| ConnectorError::IOError(format!("Failed to parse CA X.509: {e}")))?;
            let ca_key = rcgen::KeyPair::from_pem(&ca_key_pem)
                .map_err(|e| ConnectorError::IOError(format!("Failed to parse CA key: {e}")))?;
            if ca_key.public_key_raw() != ca_x509.public_key().subject_public_key.data.as_ref() {
                return Err(ConnectorError::AuthenticationFailed(
                    "Root CA private key does not match root CA certificate".to_string(),
                ));
            }
            let issuer = rcgen::Issuer::from_ca_cert_pem(&ca_cert_pem, ca_key).map_err(|e| {
                ConnectorError::IOError(format!("Failed to parse CA cert for issuer: {e}"))
            })?;
            let cert = rcgen::CertifiedIssuer::signed_by(params, leaf_key, &issuer)
                .map_err(|e| ConnectorError::IOError(format!("Failed to sign certificate: {e}")))?;
            Ok((cert.pem(), leaf_key.serialize_pem()))
        }
        (None, None) => {
            let cert = rcgen::CertifiedIssuer::self_signed(params, leaf_key).map_err(|e| {
                ConnectorError::IOError(format!("Failed to self-sign certificate: {e}"))
            })?;
            Ok((cert.pem(), leaf_key.serialize_pem()))
        }
        _ => Err(ConnectorError::IOError(
            "Both root_key and root_cert must be provided together, or neither".to_string(),
        )),
    }
}

fn write_file_atomic(path: &Path, content: &[u8], mode: u32) -> Result<(), ConnectorError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| {
            ConnectorError::IOError(format!(
                "Failed to create parent directory {}: {e}",
                parent.display()
            ))
        })?;
    }
    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| {
            ConnectorError::IOError(format!("Invalid target file path: {}", path.display()))
        })?;
    let pid = std::process::id();
    let tmp_path = path.with_file_name(format!("{file_name}.tmp.{pid}"));

    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        opts.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;

    let mut file = opts.open(&tmp_path).map_err(|e| {
        ConnectorError::IOError(format!(
            "Failed to open temporary file {}: {e}",
            tmp_path.display()
        ))
    })?;
    #[cfg(unix)]
    {
        file.set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp_path);
                ConnectorError::IOError(format!(
                    "Security error: Failed to set permissions {mode:04o} on {}: {e}",
                    tmp_path.display()
                ))
            })?;
    }
    std::io::Write::write_all(&mut file, content).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        ConnectorError::IOError(format!(
            "Failed to write temporary file {}: {e}",
            tmp_path.display()
        ))
    })?;
    file.sync_all().map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        ConnectorError::IOError(format!(
            "Failed to sync temporary file {}: {e}",
            tmp_path.display()
        ))
    })?;
    drop(file);
    std::fs::rename(&tmp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        ConnectorError::IOError(format!(
            "Failed to rename {} to {}: {e}",
            tmp_path.display(),
            path.display()
        ))
    })
}

fn write_key_and_cert(
    key_path: &Path,
    cert_path: &Path,
    key_pem: &str,
    cert_pem: &str,
) -> Result<(), ConnectorError> {
    write_file_atomic(key_path, key_pem.as_bytes(), 0o600)?;
    write_file_atomic(cert_path, cert_pem.as_bytes(), 0o644)
}

/// Generates (or validates/reuses according to [`GenCertArgs::policy`]) an `ECDSA` `P-256`
/// key pair and `X.509` certificate.
///
/// Returns `Ok(true)` when a new certificate/key pair was generated and written to disk,
/// or `Ok(false)` when generation was skipped due to policy (`IfNotPresent` or `None`).
///
/// # Errors
///
/// Returns [`ConnectorError`] if only one of `root_key`/`root_cert` is provided, if duration
/// parameters are out of range, if `CA` files are invalid or mismatched, if policy `None`
/// validation fails, or if atomic file creation fails.
pub fn generate_certificate(args: &GenCertArgs) -> Result<bool, ConnectorError> {
    if args.root_key.is_some() != args.root_cert.is_some() {
        return Err(ConnectorError::IOError(
            "Both root_key and root_cert must be provided together, or neither".to_string(),
        ));
    }
    validate_duration_range(args.date_hours).map_err(ConnectorError::IOError)?;
    if apply_policy(args)? {
        return Ok(false);
    }
    let leaf_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| ConnectorError::IOError(format!("Failed to generate KeyPair: {e}")))?;
    let params = build_cert_params(args, args.date_hours)?;
    let (cert_pem, key_pem) = sign_or_self_sign_cert(
        params,
        &leaf_key,
        args.root_key.as_deref(),
        args.root_cert.as_deref(),
    )?;
    write_key_and_cert(&args.out_key, &args.out_cert, &key_pem, &cert_pem)?;
    Ok(true)
}
