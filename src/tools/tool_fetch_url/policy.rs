use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use reqwest::Url;

/// Applies the fetch tool's scheme, credential, and local-network target policy.
///
/// This deliberately does not alter the shared model client: the model API may be
/// hosted on localhost, while a model-requested fetch must not target local or
/// private network addresses.
pub(crate) fn is_allowed_network_target(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|host| !is_local_hostname(host) && !is_private_ip(host))
}

fn is_local_hostname(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host == "metadata.google.internal"
}

fn is_private_ip(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.parse::<IpAddr>().is_ok_and(is_private_address)
}

fn is_private_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_private_ipv4(address),
        IpAddr::V6(address) => is_private_ipv6(address),
    }
}

fn is_private_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, _, _] = address.octets();
    first == 0
        || first == 10
        || first == 127
        || (first == 100 && (64..=127).contains(&second))
        || (first == 169 && second == 254)
        || (first == 172 && (16..=31).contains(&second))
        || (first == 192 && second == 0)
        || (first == 192 && second == 168)
        || (first == 198 && (18..=19).contains(&second))
        || (224..=255).contains(&first)
}

fn is_private_ipv6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    let first = segments[0];
    (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || first == 0
        || (first & 0xff00) == 0xff00
        || is_ipv4_mapped_private(&segments)
}

fn is_ipv4_mapped_private(segments: &[u16; 8]) -> bool {
    if segments[..5] != [0, 0, 0, 0, 0] || segments[5] != 0xffff {
        return false;
    }
    let address = Ipv4Addr::new(
        (segments[6] >> 8) as u8,
        segments[6] as u8,
        (segments[7] >> 8) as u8,
        segments[7] as u8,
    );
    is_private_ipv4(address)
}
