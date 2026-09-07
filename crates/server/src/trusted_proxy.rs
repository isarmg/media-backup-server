use std::{net::IpAddr, str::FromStr};

use axum::http::HeaderMap;

use crate::error::AppError;

const MAX_FORWARDED_HOPS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrustedNetwork {
    address: IpAddr,
    prefix: u8,
}

impl TrustedNetwork {
    pub(crate) fn contains(&self, candidate: IpAddr) -> bool {
        match (canonical_ip(self.address), canonical_ip(candidate)) {
            (IpAddr::V4(network), IpAddr::V4(candidate)) => {
                let prefix = u32::from(self.prefix);
                let mask = if prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - prefix)
                };
                u32::from(network) & mask == u32::from(candidate) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(candidate)) => {
                let prefix = u32::from(self.prefix);
                let mask = if prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - prefix)
                };
                u128::from(network) & mask == u128::from(candidate) & mask
            }
            _ => false,
        }
    }
}

impl FromStr for TrustedNetwork {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err("trusted proxy CIDR cannot be empty".to_owned());
        }
        let (address, prefix) = match value.split_once('/') {
            Some((address, prefix)) => {
                let address: IpAddr = address
                    .parse()
                    .map_err(|_| format!("invalid trusted proxy address: {address}"))?;
                let prefix: u8 = prefix
                    .parse()
                    .map_err(|_| format!("invalid trusted proxy prefix: {prefix}"))?;
                (canonical_ip(address), prefix)
            }
            None => {
                let address: IpAddr = value
                    .parse()
                    .map_err(|_| format!("invalid trusted proxy address: {value}"))?;
                let address = canonical_ip(address);
                let prefix = if address.is_ipv4() { 32 } else { 128 };
                (address, prefix)
            }
        };
        let maximum = if address.is_ipv4() { 32 } else { 128 };
        if prefix > maximum {
            return Err(format!("trusted proxy prefix exceeds {maximum}"));
        }
        Ok(Self { address, prefix })
    }
}

pub(crate) fn resolve_client_ip(
    peer: IpAddr,
    headers: &HeaderMap,
    trusted: &[TrustedNetwork],
) -> Result<IpAddr, AppError> {
    let peer = canonical_ip(peer);
    if !is_trusted(peer, trusted) {
        return Ok(peer);
    }
    let mut hops = Vec::new();
    for value in headers.get_all("x-forwarded-for") {
        let value = value
            .to_str()
            .map_err(|_| AppError::bad_request("invalid X-Forwarded-For header"))?;
        for hop in value.split(',') {
            if hops.len() >= MAX_FORWARDED_HOPS {
                return Err(AppError::bad_request("too many forwarded hops"));
            }
            let address = hop
                .trim()
                .parse::<IpAddr>()
                .map(canonical_ip)
                .map_err(|_| AppError::bad_request("invalid X-Forwarded-For header"))?;
            hops.push(address);
        }
    }
    let mut client = peer;
    for hop in hops.into_iter().rev() {
        if !is_trusted(client, trusted) {
            break;
        }
        client = hop;
    }
    Ok(client)
}

pub(crate) fn forwarded_as_https(
    peer: IpAddr,
    headers: &HeaderMap,
    trusted: &[TrustedNetwork],
) -> bool {
    if !is_trusted(canonical_ip(peer), trusted) {
        return false;
    }
    headers
        .get_all("x-forwarded-proto")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .next_back()
        .is_some_and(|value| value.eq_ignore_ascii_case("https"))
}

fn is_trusted(address: IpAddr, trusted: &[TrustedNetwork]) -> bool {
    trusted.iter().any(|network| network.contains(address))
}

fn canonical_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(address)),
        IpAddr::V4(_) => address,
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::*;

    fn networks(values: &[&str]) -> Vec<TrustedNetwork> {
        values.iter().map(|value| value.parse().unwrap()).collect()
    }

    #[test]
    fn untrusted_transport_peer_cannot_spoof_forwarded_identity_or_https() {
        let trusted = networks(&["10.0.0.0/8"]);
        let peer = "198.51.100.7".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.9"));
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert_eq!(resolve_client_ip(peer, &headers, &trusted).unwrap(), peer);
        assert!(!forwarded_as_https(peer, &headers, &trusted));
    }

    #[test]
    fn trusted_proxy_chain_is_walked_from_the_real_peer_right_to_left() {
        let trusted = networks(&["10.0.0.0/8", "192.0.2.10/32"]);
        let peer = "10.0.0.3".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.5, 203.0.113.8, 192.0.2.10"),
        );
        headers.insert("x-forwarded-proto", HeaderValue::from_static("http, https"));
        assert_eq!(
            resolve_client_ip(peer, &headers, &trusted).unwrap(),
            "203.0.113.8".parse::<IpAddr>().unwrap()
        );
        assert!(forwarded_as_https(peer, &headers, &trusted));
    }

    #[test]
    fn cidr_parsing_handles_exact_ipv4_ipv6_and_mapped_addresses() {
        assert!("127.0.0.1"
            .parse::<TrustedNetwork>()
            .unwrap()
            .contains("127.0.0.1".parse().unwrap()));
        assert!("::1/128"
            .parse::<TrustedNetwork>()
            .unwrap()
            .contains("::1".parse().unwrap()));
        assert!("10.0.0.0/8"
            .parse::<TrustedNetwork>()
            .unwrap()
            .contains("10.4.5.6".parse().unwrap()));
        assert!("10.0.0.0/33".parse::<TrustedNetwork>().is_err());
    }
}
