//! Which addresses an engine may connect to (`docs/architecture.md` §5).
//!
//! The desktop sends wherever its reader asks. The bridge sends for every
//! account, and only to public addresses ([`TargetPolicy::PublicOnly`]).
//! It refuses a private network, carrier-grade NAT, the benchmarking range
//! (where fake-IP proxies answer), IPv6 unique-local, its own host
//! (loopback, where services trust whoever calls), the link-local range
//! (the cloud's metadata address, which hands out the machine's
//! credentials), and unspecified, multicast, broadcast and reserved
//! addresses. An administrator allows a range back with `--allow-target`.
//! [`TargetPolicy::Networks`] still reaches public and private addresses
//! and refuses only that host, the link and the reserved addresses; the
//! bridge does not select it. An IPv4 address inside IPv6 (mapped,
//! translated, NAT64) is judged as itself.
//!
//! The check is on the resolved address at connect time, so a name that
//! resolves to an inner address — or starts to (DNS rebinding) — is refused
//! as well.

use ipnet::IpNet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TargetPolicy {
    /// Any address: the desktop.
    #[default]
    Anywhere,
    /// Public and private networks, not the bridge's own host or link, and
    /// these ranges besides. The bridge does not select this.
    Networks { allowed: Vec<IpNet> },
    /// Public addresses, and these ranges besides: the bridge.
    PublicOnly { allowed: Vec<IpNet> },
}

impl TargetPolicy {
    pub fn permits(&self, address: IpAddr) -> bool {
        match self {
            Self::Anywhere => true,
            Self::Networks { allowed } => allowed.iter().any(|net| net.contains(&address)) || !is_host_or_link(address),
            Self::PublicOnly { allowed } => allowed.iter().any(|net| net.contains(&address)) || is_public(address),
        }
    }
}

/// The IPv4 address an IPv6 one carries — mapped (`::ffff:a.b.c.d`),
/// translated (`::ffff:0:a.b.c.d`) or NAT64 (`64:ff9b::a.b.c.d`) — which it
/// is judged as.
fn inner_v4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = v6.to_ipv4_mapped() {
        return Some(v4);
    }
    match v6.segments() {
        [0, 0, 0, 0, 0xffff, 0, high, low] | [0x64, 0xff9b, 0, 0, 0, 0, high, low] => {
            Some(Ipv4Addr::from((u32::from(high) << 16) | u32::from(low)))
        }
        _ => None,
    }
}

/// The bridge's own host and its link, and addresses no request is for.
fn is_host_or_link(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_host_or_link_v4(v4),
        IpAddr::V6(v6) => match inner_v4(v6) {
            Some(v4) => is_host_or_link_v4(v4),
            None => {
                let first = v6.segments()[0];
                v6.is_unspecified()
                    || v6.is_loopback()
                    || v6.is_multicast()
                    // fe80::/10 link local
                    || (first & 0xffc0) == 0xfe80
            }
        },
    }
}

fn is_host_or_link_v4(v4: Ipv4Addr) -> bool {
    let a = v4.octets()[0];
    v4.is_unspecified()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_broadcast()
        // 0.0.0.0/8 "this network", 240.0.0.0/4 reserved
        || a == 0
        || a >= 240
}

fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    !(v4.is_unspecified()
        || v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_broadcast()
        || v4.is_documentation()
        // 0.0.0.0/8, 100.64.0.0/10 (shared / CGNAT), 192.0.0.0/24, 198.18.0.0/15, 240.0.0.0/4
        || a == 0
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && v4.octets()[2] == 0)
        || (a == 198 && (b == 18 || b == 19))
        || a >= 240)
}

fn is_public_v6(v6: Ipv6Addr) -> bool {
    if let Some(v4) = inner_v4(v6) {
        return is_public_v4(v4);
    }
    let first = v6.segments()[0];
    !(v6.is_unspecified()
        || v6.is_loopback()
        || v6.is_multicast()
        // fc00::/7 unique local, fe80::/10 link local, the rest of 64:ff9b::/96
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first == 0x64 && v6.segments()[1] == 0xff9b)
        // 2001:db8::/32 documentation
        || (first == 0x2001 && v6.segments()[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("an address")
    }

    #[test]
    fn the_bridge_refuses_inner_addresses() {
        let policy = TargetPolicy::PublicOnly { allowed: vec![] };
        for inner in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "198.18.4.114",
            "::ffff:0:c612:472",
        ] {
            assert!(!policy.permits(ip(inner)), "{inner} must be refused");
        }
        for public in ["93.184.216.34", "1.1.1.1", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
            assert!(policy.permits(ip(public)), "{public} must be allowed");
        }
    }

    /// Public and private networks, but never the bridge's own host or
    /// link — wherever an address hides it. The bridge does not select this.
    #[test]
    fn the_bridge_reaches_networks_but_not_its_host_or_link() {
        let policy = TargetPolicy::Networks { allowed: vec![] };
        for network in [
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "198.18.4.114",
            "fd00::1",
            "::ffff:10.1.2.3",
            "93.184.216.34",
            "2606:4700:4700::1111",
        ] {
            assert!(policy.permits(ip(network)), "{network} must be reached");
        }
        for own in [
            "127.0.0.1",
            "127.9.9.9",
            "169.254.169.254",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:0:a9fe:a9fe",
            "64:ff9b::a9fe:a9fe",
        ] {
            assert!(!policy.permits(ip(own)), "{own} must be refused");
        }
        let with_loopback = TargetPolicy::Networks {
            allowed: vec!["127.0.0.1/32".parse().expect("a range")],
        };
        assert!(with_loopback.permits(ip("127.0.0.1")), "allowed back");
        assert!(!with_loopback.permits(ip("127.0.0.2")));
    }

    #[test]
    fn allowed_ranges_come_back() {
        let policy = TargetPolicy::PublicOnly {
            allowed: vec!["10.0.0.0/8".parse().expect("a range")],
        };
        assert!(policy.permits(ip("10.9.9.9")));
        assert!(!policy.permits(ip("192.168.0.1")));
        assert!(TargetPolicy::Anywhere.permits(ip("127.0.0.1")));
    }
}
