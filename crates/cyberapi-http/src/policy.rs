//! Which addresses an engine may connect to (`docs/architecture.md` §5).
//!
//! The desktop sends wherever its reader asks. The bridge sends for every
//! account from inside its own network, so by default it refuses the
//! addresses that would turn it into a way in: loopback, private and
//! link-local ranges (the cloud metadata address among them), carrier-grade
//! NAT, unspecified, multicast and broadcast, IPv6 unique-local, and IPv4
//! inside IPv6. An administrator can allow ranges back.
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
    /// Public addresses, and these ranges besides: the bridge.
    PublicOnly { allowed: Vec<IpNet> },
}

impl TargetPolicy {
    pub fn permits(&self, address: IpAddr) -> bool {
        match self {
            Self::Anywhere => true,
            Self::PublicOnly { allowed } => allowed.iter().any(|net| net.contains(&address)) || is_public(address),
        }
    }
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
    // An IPv4 address wrapped in IPv6 is judged as itself.
    if let Some(v4) = v6.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let first = v6.segments()[0];
    !(v6.is_unspecified()
        || v6.is_loopback()
        || v6.is_multicast()
        // fc00::/7 unique local, fe80::/10 link local, 64:ff9b::/96 NAT64
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
        ] {
            assert!(!policy.permits(ip(inner)), "{inner} must be refused");
        }
        for public in ["93.184.216.34", "1.1.1.1", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
            assert!(policy.permits(ip(public)), "{public} must be allowed");
        }
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
