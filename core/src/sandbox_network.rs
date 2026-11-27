// ── Network security for the S-LEG sandbox ────────────────────────────────
//
// Extracted from sandbox.rs — domain whitelisting, private IP detection,
// DNS rebinding defense, and auth header resolution from environment.

use std::collections::HashMap;
use std::net::IpAddr;

/// Maximum fetch response body size (10 MB).
pub(crate) const MAX_FETCH_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// Maximum file size for host_read_file (10 MB).
pub(crate) const MAX_READ_FILE_BYTES: usize = 10 * 1024 * 1024;

/// Check if a URL's domain is in the allowed list.
///
/// Security layers:
/// 1. Only http/https schemes allowed (blocks file://, gopher://, etc.)
/// 2. Uses proper URL parsing to prevent substring bypasses
/// 3. Private/reserved IPs blocked even with wildcard `*`
/// 4. Uses `std::net::IpAddr` for unified IPv4/IPv6 handling
///    (prevents IPv6-mapped IPv4 bypass like `::ffff:169.254.169.254`)
pub(crate) fn is_domain_allowed(url_str: &str, allowed_domains: &[String]) -> bool {
    let Ok(parsed) = url::Url::parse(url_str) else {
        return false;
    };

    // Only allow http and https schemes
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }

    let Some(host) = parsed.host_str() else {
        return false;
    };

    // Block private/reserved IPs regardless of whitelist
    if is_private_ip(host) {
        // Exception: explicitly whitelisted IPs (e.g. 127.0.0.1 for server_monitor)
        if !allowed_domains.iter().any(|d| d == host) {
            return false;
        }
    }

    if allowed_domains.iter().any(|d| d == "*") {
        return true;
    }
    allowed_domains
        .iter()
        .any(|d| host == d.as_str() || host.ends_with(&format!(".{d}")))
}

/// Check if a host string is a private/reserved IP address.
///
/// Non-IP hostnames (domain names) return false — they are checked
/// separately by the domain whitelist.
pub(crate) fn is_private_ip(host: &str) -> bool {
    // Strip brackets for IPv6 URLs like [::1]
    let clean = host.trim_start_matches('[').trim_end_matches(']');
    // Explicitly block "localhost" — it resolves to 127.0.0.1 but
    // doesn't parse as an IP, so it would bypass IP-based checks.
    if clean.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match clean.parse::<IpAddr>() {
        Ok(ip) => is_private_ip_addr(ip),
        Err(_) => false, // Not an IP (it's a domain name)
    }
}

/// Check if an IP address is private/reserved.
///
/// Used both for URL-level checks (`is_private_ip`) and for DNS rebinding
/// protection (verifying resolved addresses before connecting).
///
/// Handles:
/// - Standard IPv4 private ranges (10/8, 172.16/12, 192.168/16)
/// - Loopback (127/8, ::1)
/// - Link-local / cloud metadata (169.254/16)
/// - IPv6-mapped IPv4 (::ffff:x.x.x.x)
/// - IPv6 unique local (fc00::/7) and link-local (fe80::/10)
pub(crate) fn is_private_ip_addr(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            matches!(
                (octets[0], octets[1]),
                (0, _)       // 0.0.0.0/8
                | (10, _)    // 10.0.0.0/8
                | (127, _)   // 127.0.0.0/8
                | (169, 254) // 169.254.0.0/16 (link-local, cloud metadata)
                | (192, 168) // 192.168.0.0/16
            ) || (octets[0] == 172 && (16..=31).contains(&octets[1]))
              || (octets[0] == 100 && (64..=127).contains(&octets[1])) // 100.64.0.0/10 (CGN/cloud internal)
              || (octets[0] >= 224 && octets[0] <= 239)  // 224.0.0.0/4 (multicast)
              || octets[0] == 255                         // 255.255.255.255 (broadcast)
              || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)  // 192.0.0.0/24 (IETF protocol)
              || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)  // 192.0.2.0/24 (TEST-NET-1)
              || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100) // 198.51.100.0/24 (TEST-NET-2)
              || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113) // 203.0.113.0/24 (TEST-NET-3)
        }
        IpAddr::V6(v6) => {
            // Check for IPv6-mapped IPv4 (::ffff:x.x.x.x)
            if let Some(mapped_v4) = v6.to_ipv4_mapped() {
                return is_private_ip_addr(IpAddr::V4(mapped_v4));
            }
            // IPv6 loopback
            if v6.is_loopback() {
                return true;
            }
            // IPv6 unique local (fc00::/7), link-local (fe80::/10), multicast (ff00::/8)
            let segments = v6.segments();
            let first = segments[0];
            (first & 0xfe00) == 0xfc00  // fc00::/7
                || (first & 0xffc0) == 0xfe80 // fe80::/10
                || (first & 0xff00) == 0xff00 // ff00::/8 (IPv6 multicast)
                || v6.is_unspecified() // :: (unspecified)
        }
    }
}

/// Build auth headers map from environment variables.
/// Checks: GITHUB_TOKEN -> api.github.com, GOOGLE_CALENDAR_TOKEN -> www.googleapis.com
pub fn auth_headers_from_env() -> HashMap<String, String> {
    let mut headers = HashMap::new();

    if let Ok(token) = std::env::var("GITHUB_TOKEN")
        && !token.is_empty()
    {
        headers.insert("api.github.com".to_string(), format!("Bearer {token}"));
    }

    if let Ok(token) = std::env::var("GOOGLE_CALENDAR_TOKEN")
        && !token.is_empty()
    {
        headers.insert("www.googleapis.com".to_string(), format!("Bearer {token}"));
    }

    // Aegis Executor (read-only diagnostics): the token never enters the Wasm
    // guest — the host injects it for fetches to the executor's host:port only
    // (keyed with the port so other loopback services never receive it).
    if let Ok(token) = std::env::var("AEGIS_EXECUTOR_TOKEN")
        && !token.trim().is_empty()
    {
        let executor_url = std::env::var("AEGIS_EXECUTOR_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:9876".to_string());
        if let Some(host_port) = executor_host_port(&executor_url) {
            headers.insert(host_port, format!("Bearer {}", token.trim()));
        }
    }

    headers
}

/// `host:port` of a URL, with the scheme default when no port is written.
pub fn executor_host_port(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let port = parsed
        .port()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    Some(format!("{host}:{port}"))
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Domain whitelist: substring bypass prevention ──

    #[test]
    fn domain_whitelist_rejects_substring_bypass() {
        // Proper URL parsing must prevent these old substring-match bypasses
        let allowed = vec!["api.github.com".to_string()];
        assert!(
            !is_domain_allowed("https://evil.com/api.github.com", &allowed),
            "path containing domain must not bypass"
        );
        assert!(
            !is_domain_allowed("https://api.github.com.evil.com/steal", &allowed),
            "attacker-owned superdomain must not bypass"
        );
        assert!(
            is_domain_allowed("https://api.github.com/v1/repos", &allowed),
            "exact domain must be allowed"
        );
        assert!(
            is_domain_allowed("https://sub.api.github.com/v1", &allowed),
            "subdomain must be allowed"
        );
        assert!(
            !is_domain_allowed("not-a-url", &allowed),
            "invalid URL must be rejected"
        );
    }

    #[test]
    fn domain_whitelist_wildcard_allows_public() {
        let allowed = vec!["*".to_string()];
        assert!(is_domain_allowed("https://anything.com/path", &allowed));
        // Wildcard must NOT allow private IPs (SSRF prevention)
        assert!(
            !is_domain_allowed("https://169.254.169.254/latest/meta-data/", &allowed),
            "cloud metadata endpoint must be blocked even with wildcard"
        );
        assert!(
            !is_domain_allowed("https://10.0.0.1/internal", &allowed),
            "RFC1918 10.x must be blocked even with wildcard"
        );
        assert!(
            !is_domain_allowed("https://192.168.1.1/admin", &allowed),
            "RFC1918 192.168.x must be blocked even with wildcard"
        );
        assert!(
            !is_domain_allowed("https://172.16.0.1/internal", &allowed),
            "RFC1918 172.16.x must be blocked even with wildcard"
        );
    }

    #[test]
    fn private_ip_explicitly_whitelisted_is_allowed() {
        // server_monitor explicitly lists 127.0.0.1
        let allowed = vec!["127.0.0.1".to_string()];
        assert!(
            is_domain_allowed("http://127.0.0.1:3000/system_metrics", &allowed),
            "127.0.0.1 must be allowed when explicitly whitelisted"
        );
        // But other private IPs must not be allowed by this whitelist
        assert!(
            !is_domain_allowed("http://10.0.0.1/internal", &allowed),
            "10.x must not match 127.0.0.1 whitelist"
        );
    }

    #[test]
    fn private_ip_detection() {
        // IPv4
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("10.0.0.1"));
        assert!(is_private_ip("10.255.255.255"));
        assert!(is_private_ip("172.16.0.1"));
        assert!(is_private_ip("172.31.255.255"));
        assert!(!is_private_ip("172.15.0.1"));
        assert!(!is_private_ip("172.32.0.1"));
        assert!(is_private_ip("192.168.0.1"));
        assert!(is_private_ip("169.254.169.254"));
        assert!(is_private_ip("0.0.0.0"));
        assert!(!is_private_ip("8.8.8.8"));
        assert!(!is_private_ip("1.1.1.1"));
        // IPv6
        assert!(is_private_ip("::1"));
        assert!(is_private_ip("[::1]"));
        // IPv6-mapped IPv4 (SSRF bypass vector)
        assert!(
            is_private_ip("::ffff:169.254.169.254"),
            "IPv6-mapped cloud metadata must be blocked"
        );
        assert!(
            is_private_ip("::ffff:10.0.0.1"),
            "IPv6-mapped RFC1918 must be blocked"
        );
        assert!(
            is_private_ip("::ffff:127.0.0.1"),
            "IPv6-mapped loopback must be blocked"
        );
        assert!(
            !is_private_ip("::ffff:8.8.8.8"),
            "IPv6-mapped public IP must be allowed"
        );
        // IPv6 private ranges
        assert!(is_private_ip("fc00::1"), "unique local fc00::/7");
        assert!(is_private_ip("fd12:3456::1"), "unique local fd::/8");
        assert!(is_private_ip("fe80::1"), "link-local fe80::/10");
        assert!(
            !is_private_ip("2001:db8::1"),
            "documentation prefix is not private"
        );
        // CGN / Carrier-Grade NAT (100.64.0.0/10) — cloud internal services
        assert!(
            is_private_ip("100.64.0.1"),
            "CGN range start must be blocked"
        );
        assert!(
            is_private_ip("100.100.100.100"),
            "CGN mid-range must be blocked"
        );
        assert!(
            is_private_ip("100.127.255.255"),
            "CGN range end must be blocked"
        );
        assert!(
            !is_private_ip("100.63.255.255"),
            "Below CGN range must be allowed"
        );
        assert!(
            !is_private_ip("100.128.0.0"),
            "Above CGN range must be allowed"
        );
        // IPv6-mapped CGN
        assert!(
            is_private_ip("::ffff:100.100.100.100"),
            "IPv6-mapped CGN must be blocked"
        );
        // Domain names are not IPs
        assert!(!is_private_ip("example.com"));
        assert!(
            is_private_ip("localhost"),
            "localhost must be treated as private"
        );
    }

    #[test]
    fn scheme_restriction() {
        let allowed = vec!["*".to_string()];
        assert!(
            !is_domain_allowed("file:///etc/passwd", &allowed),
            "file:// must be blocked"
        );
        assert!(
            !is_domain_allowed("gopher://internal:25/", &allowed),
            "gopher:// must be blocked"
        );
        assert!(
            !is_domain_allowed("ftp://example.com/file", &allowed),
            "ftp:// must be blocked"
        );
        assert!(is_domain_allowed("https://example.com/path", &allowed));
        assert!(is_domain_allowed("http://example.com/path", &allowed));
    }

    // ── is_private_ip_addr security tests (DNS rebinding defense) ────────

    #[test]
    fn is_private_ip_addr_blocks_cloud_metadata() {
        use std::net::{IpAddr, Ipv4Addr};
        // 169.254.169.254 is the cloud metadata endpoint (AWS/GCP/Azure)
        assert!(
            is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))),
            "cloud metadata IP must be blocked"
        );
        // Entire link-local range
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            169, 254, 0, 1
        ))));
    }

    #[test]
    fn is_private_ip_addr_blocks_all_private_ranges() {
        use std::net::{IpAddr, Ipv4Addr};
        // 10.0.0.0/8
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            10, 255, 255, 255
        ))));
        // 172.16.0.0/12
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            172, 31, 255, 255
        ))));
        assert!(!is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            172, 32, 0, 1
        ))));
        // 192.168.0.0/16
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            192, 168, 0, 1
        ))));
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            192, 168, 255, 255
        ))));
        // 127.0.0.0/8 (loopback)
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            127, 255, 255, 255
        ))));
        // 0.0.0.0/8
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
    }

    #[test]
    fn is_private_ip_addr_allows_public_ips() {
        use std::net::{IpAddr, Ipv4Addr};
        assert!(!is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            104, 16, 0, 1
        ))));
        // 203.0.113.0/24 (TEST-NET-3) is now correctly blocked
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            203, 0, 113, 1
        ))));
        // But a real public IP nearby is fine
        assert!(!is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            203, 0, 114, 1
        ))));
    }

    #[test]
    fn is_private_ip_addr_blocks_ipv6_private() {
        use std::net::{IpAddr, Ipv6Addr};
        // IPv6 loopback (::1)
        assert!(is_private_ip_addr(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        // IPv6 unique local (fc00::/7)
        assert!(is_private_ip_addr(IpAddr::V6(Ipv6Addr::new(
            0xfc00, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(is_private_ip_addr(IpAddr::V6(Ipv6Addr::new(
            0xfd00, 0, 0, 0, 0, 0, 0, 1
        ))));
        // IPv6 link-local (fe80::/10)
        assert!(is_private_ip_addr(IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn is_private_ip_addr_blocks_ipv6_mapped_private_ipv4() {
        use std::net::{IpAddr, Ipv6Addr};
        // ::ffff:169.254.169.254 (IPv6-mapped cloud metadata)
        let mapped_metadata = Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xa9fe, 0xa9fe);
        assert!(
            is_private_ip_addr(IpAddr::V6(mapped_metadata)),
            "IPv6-mapped 169.254.169.254 must be blocked (DNS rebinding via IPv6)"
        );
        // ::ffff:10.0.0.1
        let mapped_private = Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x0a00, 0x0001);
        assert!(
            is_private_ip_addr(IpAddr::V6(mapped_private)),
            "IPv6-mapped 10.0.0.1 must be blocked"
        );
        // ::ffff:127.0.0.1
        let mapped_loopback = Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x7f00, 0x0001);
        assert!(
            is_private_ip_addr(IpAddr::V6(mapped_loopback)),
            "IPv6-mapped 127.0.0.1 must be blocked"
        );
    }

    #[test]
    fn is_private_ip_addr_allows_public_ipv6() {
        use std::net::{IpAddr, Ipv6Addr};
        // Google DNS IPv6
        let google_dns = Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888);
        assert!(!is_private_ip_addr(IpAddr::V6(google_dns)));
    }

    #[test]
    fn is_private_ip_addr_blocks_multicast_and_broadcast() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        // IPv4 multicast 224.0.0.0/4
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1))));
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            239, 255, 255, 255
        ))));
        // IPv4 broadcast
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            255, 255, 255, 255
        ))));
        // TEST-NET-1 (192.0.2.0/24)
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))));
        // TEST-NET-2 (198.51.100.0/24)
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            198, 51, 100, 1
        ))));
        // IPv6 multicast (ff00::/8)
        assert!(is_private_ip_addr(IpAddr::V6(Ipv6Addr::new(
            0xff02, 0, 0, 0, 0, 0, 0, 1
        ))));
        // IPv6 unspecified (::)
        assert!(is_private_ip_addr(IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
    }

    // ── Auth header env tests ──

    #[test]
    #[allow(unsafe_code)]
    fn auth_headers_from_env_picks_up_github_token() {
        unsafe { std::env::set_var("GITHUB_TOKEN", "ghp_testtoken") };
        let headers = auth_headers_from_env();
        assert_eq!(
            headers.get("api.github.com").map(|s| s.as_str()),
            Some("Bearer ghp_testtoken"),
        );
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
    }

    #[test]
    #[allow(unsafe_code)]
    fn auth_headers_from_env_picks_up_google_token() {
        unsafe { std::env::set_var("GOOGLE_CALENDAR_TOKEN", "ya29.test") };
        let headers = auth_headers_from_env();
        assert_eq!(
            headers.get("www.googleapis.com").map(|s| s.as_str()),
            Some("Bearer ya29.test"),
        );
        unsafe { std::env::remove_var("GOOGLE_CALENDAR_TOKEN") };
    }

    #[test]
    #[allow(unsafe_code)]
    fn auth_headers_from_env_skips_empty_tokens() {
        unsafe { std::env::set_var("GITHUB_TOKEN", "") };
        let headers = auth_headers_from_env();
        assert!(
            !headers.contains_key("api.github.com"),
            "empty token must not produce an auth header"
        );
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
    }

    // ── Property-based tests (proptest) ─────────────────────────────────

    mod proptest_ssrf {
        use super::*;
        use proptest::prelude::*;
        use std::net::{IpAddr, Ipv4Addr};

        proptest! {
            /// is_private_ip never panics on arbitrary host strings
            #[test]
            fn private_ip_never_panics(host in "\\PC{0,200}") {
                let _ = is_private_ip(&host);
            }

            /// All RFC1918 10.x.y.z addresses are blocked
            #[test]
            fn all_rfc1918_10_blocked(
                a in 0u8..=255u8,
                b in 0u8..=255u8,
                c in 0u8..=255u8
            ) {
                let ip = IpAddr::V4(Ipv4Addr::new(10, a, b, c));
                prop_assert!(is_private_ip_addr(ip), "10.{a}.{b}.{c} must be private");
            }

            /// All 172.16-31.x.y addresses are blocked
            #[test]
            fn all_rfc1918_172_blocked(
                second in 16u8..=31u8,
                c in 0u8..=255u8,
                d in 0u8..=255u8
            ) {
                let ip = IpAddr::V4(Ipv4Addr::new(172, second, c, d));
                prop_assert!(is_private_ip_addr(ip), "172.{second}.{c}.{d} must be private");
            }

            /// All 192.168.x.y addresses are blocked
            #[test]
            fn all_rfc1918_192_168_blocked(c in 0u8..=255u8, d in 0u8..=255u8) {
                let ip = IpAddr::V4(Ipv4Addr::new(192, 168, c, d));
                prop_assert!(is_private_ip_addr(ip), "192.168.{c}.{d} must be private");
            }

            /// All 127.x.y.z loopback addresses are blocked
            #[test]
            fn all_loopback_blocked(a in 0u8..=255u8, b in 0u8..=255u8, c in 0u8..=255u8) {
                let ip = IpAddr::V4(Ipv4Addr::new(127, a, b, c));
                prop_assert!(is_private_ip_addr(ip), "127.{a}.{b}.{c} must be private");
            }

            /// All CGN 100.64-127.x.y addresses are blocked
            #[test]
            fn all_cgn_blocked(second in 64u8..=127u8, c in 0u8..=255u8, d in 0u8..=255u8) {
                let ip = IpAddr::V4(Ipv4Addr::new(100, second, c, d));
                prop_assert!(is_private_ip_addr(ip), "100.{second}.{c}.{d} must be private (CGN)");
            }

            /// All 169.254.x.y link-local addresses are blocked
            #[test]
            fn all_link_local_blocked(c in 0u8..=255u8, d in 0u8..=255u8) {
                let ip = IpAddr::V4(Ipv4Addr::new(169, 254, c, d));
                prop_assert!(is_private_ip_addr(ip), "169.254.{c}.{d} must be private");
            }

            /// Public IPs in safe ranges are allowed
            #[test]
            fn public_ips_allowed(
                first in 1u8..=9u8,  // 1.x-9.x are public
                b in 0u8..=255u8,
                c in 0u8..=255u8,
                d in 1u8..=254u8     // avoid .0 and .255
            ) {
                let ip = IpAddr::V4(Ipv4Addr::new(first, b, c, d));
                prop_assert!(!is_private_ip_addr(ip), "{first}.{b}.{c}.{d} should be public");
            }

            /// IPv6-mapped IPv4 private addresses are also blocked
            #[test]
            fn ipv6_mapped_private_blocked(
                a in 0u8..=255u8,
                b in 0u8..=255u8,
                c in 0u8..=255u8
            ) {
                let v4 = Ipv4Addr::new(10, a, b, c);
                let v6 = v4.to_ipv6_mapped();
                let ip = IpAddr::V6(v6);
                prop_assert!(is_private_ip_addr(ip), "::ffff:10.{a}.{b}.{c} must be private");
            }

            /// All IETF protocol assignment IPs (192.0.0.0/24) are blocked
            #[test]
            fn all_ietf_protocol_blocked(d in 0u8..=255u8) {
                let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 0, d));
                prop_assert!(is_private_ip_addr(ip), "192.0.0.{d} must be private (IETF protocol)");
            }

            /// All TEST-NET-1 IPs (192.0.2.0/24) are blocked
            #[test]
            fn all_test_net_1_blocked(d in 0u8..=255u8) {
                let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, d));
                prop_assert!(is_private_ip_addr(ip), "192.0.2.{d} must be private (TEST-NET-1)");
            }

            /// All TEST-NET-2 IPs (198.51.100.0/24) are blocked
            #[test]
            fn all_test_net_2_blocked(d in 0u8..=255u8) {
                let ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, d));
                prop_assert!(is_private_ip_addr(ip), "198.51.100.{d} must be private (TEST-NET-2)");
            }

            /// All TEST-NET-3 IPs (203.0.113.0/24) are blocked
            #[test]
            fn all_test_net_3_blocked(d in 0u8..=255u8) {
                let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, d));
                prop_assert!(is_private_ip_addr(ip), "203.0.113.{d} must be private (TEST-NET-3)");
            }

            /// All multicast IPs (224.0.0.0/4) are blocked
            #[test]
            fn all_multicast_blocked(first in 224u8..=239u8, b in 0u8..=255u8, c in 0u8..=255u8, d in 0u8..=255u8) {
                let ip = IpAddr::V4(Ipv4Addr::new(first, b, c, d));
                prop_assert!(is_private_ip_addr(ip), "{first}.{b}.{c}.{d} must be private (multicast)");
            }

            /// Broadcast (255.x.y.z) is blocked
            #[test]
            fn all_broadcast_blocked(b in 0u8..=255u8, c in 0u8..=255u8, d in 0u8..=255u8) {
                let ip = IpAddr::V4(Ipv4Addr::new(255, b, c, d));
                prop_assert!(is_private_ip_addr(ip), "255.{b}.{c}.{d} must be private (broadcast)");
            }

            /// is_domain_allowed rejects non-http(s) schemes
            #[test]
            fn rejects_non_http_schemes(
                scheme in "(ftp|gopher|file|data|javascript|ssh)",
                domain in "[a-z]{3,10}\\.[a-z]{2,4}"
            ) {
                let url = format!("{scheme}://{domain}/path");
                let domains = vec!["*".to_string()];
                prop_assert!(
                    !is_domain_allowed(&url, &domains),
                    "{scheme}:// should be rejected even with wildcard"
                );
            }

            /// Wildcard allows any public domain but blocks private IPs
            #[test]
            fn wildcard_allows_public_blocks_private(
                a in 0u8..=255u8,
                b in 0u8..=255u8,
                c in 0u8..=255u8
            ) {
                let url = format!("http://10.{a}.{b}.{c}/");
                let domains = vec!["*".to_string()];
                prop_assert!(
                    !is_domain_allowed(&url, &domains),
                    "Wildcard must still block private IP 10.{a}.{b}.{c}"
                );
            }

            /// is_domain_allowed never panics on arbitrary URL strings
            #[test]
            fn domain_allowed_never_panics(url in "\\PC{0,300}") {
                let domains = vec!["example.com".to_string(), "*".to_string()];
                let _ = is_domain_allowed(&url, &domains);
            }
        }
    }
}
