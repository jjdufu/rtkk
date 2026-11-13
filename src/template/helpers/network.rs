//! `IP` and `CIDR` network template helper functions.

pub(super) fn generate_ips_from_cidr_fn(
    args: &[gtmpl::Value],
) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs(
            "generateIPsFromCIDR".to_string(),
            1,
        ));
    };
    let gtmpl::Value::String(cidr_str) = first else {
        return Err(gtmpl::FuncError::Generic(
            "generateIPsFromCIDR expects a string".to_string(),
        ));
    };
    let mut ips = Vec::new();
    for part in cidr_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let net: ipnet::IpNet = part
            .parse()
            .map_err(|e| gtmpl::FuncError::Generic(format!("invalid CIDR '{part}': {e}")))?;
        match net {
            ipnet::IpNet::V4(net4) => {
                if net4.prefix_len() < 24 {
                    let pref = net4.prefix_len();
                    return Err(gtmpl::FuncError::Generic(format!(
                        "IPv4 CIDR prefix /{pref} of '{part}' is too small (subnet too large). Generation is restricted to /24 or larger."
                    )));
                }
                for ip in net4.hosts() {
                    ips.push(gtmpl::Value::String(ip.to_string()));
                }
            }
            ipnet::IpNet::V6(net6) => {
                if net6.prefix_len() < 120 {
                    let pref = net6.prefix_len();
                    return Err(gtmpl::FuncError::Generic(format!(
                        "IPv6 CIDR prefix /{pref} of '{part}' is too small (subnet too large). Generation is restricted to /120 or larger."
                    )));
                }
                for ip in net6.hosts() {
                    ips.push(gtmpl::Value::String(ip.to_string()));
                }
            }
        }
    }
    Ok(gtmpl::Value::Array(ips))
}

pub(super) fn ip_in_cidr_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    if args.len() < 2 {
        return Err(gtmpl::FuncError::AtLeastXArgs("ipInCIDR".to_string(), 2));
    }
    let Some(gtmpl::Value::String(ip_str)) = args.first() else {
        return Err(gtmpl::FuncError::Generic(
            "ipInCIDR expects IP address as first argument".to_string(),
        ));
    };
    let Some(gtmpl::Value::String(cidr_str)) = args.get(1) else {
        return Err(gtmpl::FuncError::Generic(
            "ipInCIDR expects CIDR block as second argument".to_string(),
        ));
    };
    let ip: std::net::IpAddr = ip_str
        .trim()
        .parse()
        .map_err(|e| gtmpl::FuncError::Generic(format!("invalid IP address '{ip_str}': {e}")))?;
    let net: ipnet::IpNet = cidr_str
        .trim()
        .parse()
        .map_err(|e| gtmpl::FuncError::Generic(format!("invalid CIDR block '{cidr_str}': {e}")))?;
    Ok(gtmpl::Value::Bool(net.contains(&ip)))
}

pub(super) fn ip_family_fn(args: &[gtmpl::Value]) -> Result<gtmpl::Value, gtmpl::FuncError> {
    let Some(first) = args.first() else {
        return Err(gtmpl::FuncError::AtLeastXArgs("ipFamily".to_string(), 1));
    };
    let gtmpl::Value::String(s) = first else {
        return Err(gtmpl::FuncError::Generic(
            "ipFamily expects a string".to_string(),
        ));
    };
    let s = s.trim();
    if let Ok(ip) = s.parse::<std::net::IpAddr>() {
        match ip {
            std::net::IpAddr::V4(_) => Ok(gtmpl::Value::String("IPv4".to_string())),
            std::net::IpAddr::V6(_) => Ok(gtmpl::Value::String("IPv6".to_string())),
        }
    } else if let Ok(net) = s.parse::<ipnet::IpNet>() {
        match net {
            ipnet::IpNet::V4(_) => Ok(gtmpl::Value::String("IPv4".to_string())),
            ipnet::IpNet::V6(_) => Ok(gtmpl::Value::String("IPv6".to_string())),
        }
    } else {
        Err(gtmpl::FuncError::Generic(format!(
            "invalid IP or CIDR: {s}"
        )))
    }
}
