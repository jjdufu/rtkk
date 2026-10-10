use crate::connector::ConnectorError;
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CertPolicy {
    Always,
    IfNotPresent,
    None,
}
impl<'de> serde::Deserialize<'de> for CertPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "always" => Ok(CertPolicy::Always),
            "ifnotpresent" | "if_not_present" => Ok(CertPolicy::IfNotPresent),
            "none" => Ok(CertPolicy::None),
            _ => Err(serde::de::Error::custom(format!(
                "unknown cert policy: {s}"
            ))),
        }
    }
}
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct GenCertArgs {
    pub cn: String,
    pub out_key: PathBuf,
    pub out_cert: PathBuf,
    pub root_key: Option<PathBuf>,
    pub root_cert: Option<PathBuf>,
    #[serde(deserialize_with = "deserialize_sans")]
    pub sans: Vec<String>,
    pub is_ca: bool,
    #[serde(
        alias = "date",
        deserialize_with = "deserialize_duration_hours",
        default = "default_date_hours"
    )]
    pub date_hours: u64,
    pub policy: CertPolicy,
}
fn default_date_hours() -> u64 {
    867_240
}
fn deserialize_duration_hours<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct DurationVisitor;
    impl serde::de::Visitor<'_> for DurationVisitor {
        type Value = u64;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a string representing hours (e.g., '867240h') or an integer")
        }
        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v)
        }
        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            let val: u64 = v
                .try_into()
                .map_err(|e| E::custom(format!("invalid integer: {e}")))?;
            Ok(val)
        }
        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            let trimmed = v.trim();
            let s = if trimmed.ends_with('h') {
                trimmed.get(..trimmed.len().saturating_sub(1)).unwrap_or("")
            } else {
                trimmed
            };
            s.parse::<u64>()
                .map_err(|e| E::custom(format!("invalid hours: {e}")))
        }
    }
    deserializer.deserialize_any(DurationVisitor)
}
fn deserialize_sans<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct SansVisitor;
    impl<'de> serde::de::Visitor<'de> for SansVisitor {
        type Value = Vec<String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(
                "a sequence of strings or a JSON string representing a sequence of strings",
            )
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
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                serde_json::from_str::<Vec<String>>(trimmed)
                    .map_err(|e| E::custom(format!("failed to parse SANs JSON string: {e}")))
            } else {
                Ok(vec![v.to_string()])
            }
        }
    }
    deserializer.deserialize_any(SansVisitor)
}
#[must_use]
pub(crate) fn is_cert_valid(pem_str: &str) -> bool {
    if let Ok((_, pem)) = x509_parser::pem::parse_x509_pem(pem_str.as_bytes()) {
        if let Ok(cert) = pem.parse_x509() {
            return cert.validity().is_valid();
        }
    }
    false
}
pub(crate) fn check_cert_files(key_path: &Path, cert_path: &Path) -> Result<bool, String> {
    if !key_path.exists() || !cert_path.exists() {
        return Ok(false);
    }
    let cert_content = std::fs::read_to_string(cert_path)
        .map_err(|e| format!("Failed to read certificate: {e}"))?;
    if is_cert_valid(&cert_content) {
        Ok(true)
    } else {
        Ok(false)
    }
}
fn sign_or_self_sign_cert(
    params: rcgen::CertificateParams,
    leaf_key: &rcgen::KeyPair,
    root_key_path: Option<&std::path::PathBuf>,
    root_cert_path: Option<&std::path::PathBuf>,
) -> Result<(String, String), ConnectorError> {
    if let (Some(rkey_path), Some(rcert_path)) = (root_key_path, root_cert_path) {
        let ca_key_pem = std::fs::read_to_string(rkey_path)
            .map_err(|e| ConnectorError::IOError(format!("Failed to read CA key: {e}")))?;
        let ca_cert_pem = std::fs::read_to_string(rcert_path)
            .map_err(|e| ConnectorError::IOError(format!("Failed to read CA cert: {e}")))?;
        let ca_key = rcgen::KeyPair::from_pem(&ca_key_pem)
            .map_err(|e| ConnectorError::IOError(format!("Failed to parse CA key: {e}")))?;
        let issuer = rcgen::Issuer::from_ca_cert_pem(&ca_cert_pem, ca_key).map_err(|e| {
            ConnectorError::IOError(format!("Failed to parse CA cert for issuer: {e}"))
        })?;
        let cert = rcgen::CertifiedIssuer::signed_by(params, leaf_key, &issuer)
            .map_err(|e| ConnectorError::IOError(format!("Failed to sign certificate: {e}")))?;
        Ok((cert.pem(), leaf_key.serialize_pem()))
    } else {
        let cert = rcgen::CertifiedIssuer::self_signed(params, leaf_key).map_err(|e| {
            ConnectorError::IOError(format!("Failed to self-sign certificate: {e}"))
        })?;
        Ok((cert.pem(), leaf_key.serialize_pem()))
    }
}
fn apply_policy(args: &GenCertArgs) -> Result<bool, ConnectorError> {
    let key_path = &args.out_key;
    let cert_path = &args.out_cert;
    match args.policy {
        CertPolicy::Always => Ok(false),
        CertPolicy::IfNotPresent => {
            let present = check_cert_files(key_path, cert_path).map_err(ConnectorError::IOError)?;
            Ok(present)
        }
        CertPolicy::None => {
            if !key_path.exists() || !cert_path.exists() {
                let kp = key_path.display();
                let cp = cert_path.display();
                return Err(ConnectorError::IOError(format!(
                    "Certificate or key file not found under policy None: {kp} or {cp}"
                )));
            }
            let cert_content = std::fs::read_to_string(cert_path).map_err(|e| {
                ConnectorError::IOError(format!(
                    "Failed to read certificate under policy None: {e}"
                ))
            })?;
            if !is_cert_valid(&cert_content) {
                let cp = cert_path.display();
                return Err(ConnectorError::AuthenticationFailed(format!(
                    "Certificate is expired or not yet valid: {cp}"
                )));
            }
            Ok(true)
        }
    }
}
fn build_cert_params(
    args: &GenCertArgs,
    duration_hours: u64,
) -> Result<rcgen::CertificateParams, ConnectorError> {
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
            .key_usages
            .push(rcgen::KeyUsagePurpose::KeyEncipherment);
        params
            .extended_key_usages
            .push(rcgen::ExtendedKeyUsagePurpose::ServerAuth);
        params
            .extended_key_usages
            .push(rcgen::ExtendedKeyUsagePurpose::ClientAuth);
    }
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::hours(1);
    let duration_i64: i64 = duration_hours
        .try_into()
        .map_err(|e| ConnectorError::IOError(format!("Invalid duration value: {e}")))?;
    params.not_after = now + time::Duration::hours(duration_i64);
    Ok(params)
}
fn write_key_and_cert(
    key_path: &Path,
    cert_path: &Path,
    key_pem: &str,
    cert_pem: &str,
) -> Result<(), ConnectorError> {
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ConnectorError::IOError(format!("Failed to create key parent dir: {e}"))
        })?;
    }
    std::fs::write(key_path, key_pem)
        .map_err(|e| ConnectorError::IOError(format!("Failed to write private key: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600)).map_err(
            |e| {
                let kp = key_path.display();
                ConnectorError::IOError(format!(
                    "Security error: Failed to limit private key permissions to 0600 on {kp}: {e}"
                ))
            },
        )?;
    }
    if let Some(parent) = cert_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ConnectorError::IOError(format!("Failed to create cert parent dir: {e}"))
        })?;
    }
    std::fs::write(cert_path, cert_pem)
        .map_err(|e| ConnectorError::IOError(format!("Failed to write certificate: {e}")))?;
    Ok(())
}
pub(crate) fn generate_certificate(args: &GenCertArgs) -> Result<(), ConnectorError> {
    if apply_policy(args)? {
        return Ok(());
    }
    let duration_hours = if args.is_ca || args.root_key.is_some() {
        867_240
    } else {
        args.date_hours
    };
    let leaf_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| ConnectorError::IOError(format!("Failed to generate KeyPair: {e}")))?;
    let params = build_cert_params(args, duration_hours)?;
    let (cert_pem, key_pem) = sign_or_self_sign_cert(
        params,
        &leaf_key,
        args.root_key.as_ref(),
        args.root_cert.as_ref(),
    )?;
    write_key_and_cert(&args.out_key, &args.out_cert, &key_pem, &cert_pem)?;
    Ok(())
}
