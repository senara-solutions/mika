//! Runtime config validator for the testimony lane (mika#1796) — defense in depth.
//!
//! The type system already prevents mis-wiring a cloud provider into the
//! testimony lane at compile time (see [`crate::voice::room::VoiceRoom`]).
//! This validator addresses a *different* failure mode: an operator hand-edits
//! a config file to point a testimony-lane provider at a cloud URL, thinking
//! "it's just a URL, the provider is local". At startup, this check rejects
//! anything that isn't loopback (`127.0.0.1` / `::1`), RFC1918 LAN, or IPv6
//! ULA (`fc00::/7`) — and fails closed (gateway refuses to start, not
//! warn+continue).
//!
//! The scope here is narrow: `<ip>:<port>` literal validation via
//! [`SocketAddr::from_str`]. No DNS resolution — hostnames are rejected
//! outright because a resolved-at-startup address does not guarantee the
//! runtime call's target. DNS-time / packet-time non-transit enforcement is
//! the runtime-egress companion ticket's job (mika#1961 — nftables egress
//! deny for testimony ports).
//!
//! # Companion runtime gate
//!
//! At the network layer, an nftables/iptables deny rule on the gateway host
//! MUST also block testimony ports from reaching WAN. That is intentionally
//! carved out of this file (it is a deployment concern, not a build-time
//! invariant). This validator is one belt; the nftables rule is the
//! suspenders.

use core::net::{IpAddr, Ipv6Addr, SocketAddr};

use thiserror::Error;

/// Errors produced by [`VoiceConfig::validate`].
#[derive(Debug, Error)]
pub enum VoiceConfigError {
    /// A testimony-lane endpoint parsed to an IP outside loopback, RFC1918
    /// LAN, or IPv6 ULA.
    #[error(
        "testimony endpoint '{endpoint}' resolves to non-local IP {ip} — \
         non-transit invariant requires loopback (127.0.0.1 / ::1), RFC1918 \
         LAN, or IPv6 ULA (fc00::/7)"
    )]
    TestimonyEndpointNotLocal { endpoint: String, ip: IpAddr },

    /// A testimony-lane endpoint carried a hostname instead of an IP literal.
    /// DNS resolution is out of scope for this validator — hostnames MUST be
    /// resolved to IP literals in config, or the operator must use the
    /// network-layer companion (nftables rule) to enforce non-transit.
    #[error(
        "testimony endpoint '{endpoint}' is a hostname; testimony configs \
         must use an IP literal (loopback, RFC1918, or IPv6 ULA) so the \
         non-transit invariant is checkable without DNS at startup"
    )]
    TestimonyEndpointIsHostname { endpoint: String },

    /// The endpoint string could not be parsed as `<ip>:<port>` — bad port,
    /// missing port, malformed brackets, etc.
    #[error("testimony endpoint '{endpoint}' is not parseable: {reason}")]
    TestimonyEndpointUnparseable { endpoint: String, reason: String },
}

/// Voice configuration surface. Currently minimal — the fields will grow as
/// mika#1786 / mika#1792 land LiveKit Cloud + self-hosted room configs.
///
/// The presently-shipping property is the testimony-lane endpoint list;
/// wiring the *actual* room orchestration through this struct is deferred to
/// the config-schema tickets referenced above. This ticket contributes the
/// **validator surface** those tickets will bind their config to.
#[derive(Debug, Clone, Default)]
pub struct VoiceConfig {
    /// STT and TTS endpoints for the testimony lane. Each entry is an
    /// `<ip>:<port>` string. Rejected: hostnames, ports outside `1..=65535`,
    /// malformed IP literals, IPs outside loopback / RFC1918 LAN / IPv6 ULA.
    pub testimony_endpoints: Vec<String>,
}

impl VoiceConfig {
    /// Reject any testimony endpoint that fails the local-address check.
    /// Fails closed on first offender — the gateway is expected to `?`-bubble
    /// the error at startup rather than log-and-continue.
    pub fn validate(&self) -> Result<(), VoiceConfigError> {
        for endpoint in &self.testimony_endpoints {
            let ip = parse_endpoint(endpoint)?;
            if !is_local_address(ip) {
                return Err(VoiceConfigError::TestimonyEndpointNotLocal {
                    endpoint: endpoint.clone(),
                    ip,
                });
            }
        }
        Ok(())
    }
}

/// Parse an `<ip>:<port>` string using [`SocketAddr::from_str`]. This
/// enforces port validity, bracket correctness on IPv6, and rejects garbage
/// like `127.0.0.1:8080/path` up-front (a `SocketAddr` cannot carry a path).
///
/// On parse failure, distinguish "hostname" (non-empty, non-IP host) from
/// "unparseable" (missing colon, empty string, empty port, out-of-range port)
/// so the operator gets an actionable error.
fn parse_endpoint(endpoint: &str) -> Result<IpAddr, VoiceConfigError> {
    // Fast path: `SocketAddr` accepts the well-formed shapes exactly.
    if let Ok(sock) = endpoint.parse::<SocketAddr>() {
        return Ok(unmap_v4_in_v6(sock.ip()));
    }

    // Slow path: figure out why it failed to give the operator a useful
    // error. Split on the LAST ':' so unbracketed IPv6 (which is legal only
    // via SocketAddr's bracketed form) is treated as unparseable, not as a
    // hostname.
    let (host, port_str) = match endpoint.rsplit_once(':') {
        Some(pair) => pair,
        None => {
            return Err(VoiceConfigError::TestimonyEndpointUnparseable {
                endpoint: endpoint.to_string(),
                reason: "missing ':port' suffix".to_string(),
            });
        }
    };

    if host.is_empty() {
        return Err(VoiceConfigError::TestimonyEndpointUnparseable {
            endpoint: endpoint.to_string(),
            reason: "empty host".to_string(),
        });
    }
    if port_str.is_empty() {
        return Err(VoiceConfigError::TestimonyEndpointUnparseable {
            endpoint: endpoint.to_string(),
            reason: "empty port".to_string(),
        });
    }
    if port_str.parse::<u16>().is_err() {
        return Err(VoiceConfigError::TestimonyEndpointUnparseable {
            endpoint: endpoint.to_string(),
            reason: format!("port '{port_str}' is not a valid u16"),
        });
    }

    // Strip IPv6 brackets if present (unbracketed IPv6 with port is
    // ambiguous — SocketAddr rejects it, so did the fast path).
    let host_clean = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);

    // If it parses as an IP, the port must have been the reason SocketAddr
    // failed — but we already checked the port above. Reaching here with a
    // valid IP means we've got a shape SocketAddr rejected for structural
    // reasons (unbracketed IPv6-with-port). Report as unparseable.
    if host_clean.parse::<IpAddr>().is_ok() {
        return Err(VoiceConfigError::TestimonyEndpointUnparseable {
            endpoint: endpoint.to_string(),
            reason: "IPv6 addresses must be bracketed as [ipv6]:port".to_string(),
        });
    }

    // Non-empty, non-IP host with a valid port → hostname.
    Err(VoiceConfigError::TestimonyEndpointIsHostname {
        endpoint: endpoint.to_string(),
    })
}

/// If `ip` is an IPv4-mapped IPv6 address (`::ffff:0:0/96`), unmap to
/// [`IpAddr::V4`]. Otherwise, return unchanged.
///
/// This matters because `[::ffff:8.8.8.8]:443` would otherwise pass the
/// `is_local_address` check on the V6 arm (which permits nothing except
/// loopback + ULA) and hit the V4 path only if we unmap. Unmapping first
/// makes the WAN address land in the V4 check, where `is_private()` correctly
/// rejects it.
fn unmap_v4_in_v6(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    }
}

/// `true` if the address is loopback (`127.0.0.0/8`, `::1`), an RFC1918
/// LAN address (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), or an
/// IPv6 ULA (`fc00::/7` — RFC 4193).
fn is_local_address(ip: IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(v6) => is_ipv6_ula(v6),
    }
}

/// `true` if the address is an IPv6 Unique Local Address (`fc00::/7`),
/// per RFC 4193. Matches the top 7 bits: `1111 110x`.
fn is_ipv6_ula(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xfe00) == 0xfc00
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(endpoints: &[&str]) -> VoiceConfig {
        VoiceConfig {
            testimony_endpoints: endpoints.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn empty_config_is_valid() {
        assert!(VoiceConfig::default().validate().is_ok());
    }

    #[test]
    fn loopback_ipv4_is_accepted() {
        assert!(cfg(&["127.0.0.1:8080"]).validate().is_ok());
        assert!(cfg(&["127.0.0.5:1"]).validate().is_ok());
    }

    #[test]
    fn loopback_ipv6_is_accepted() {
        assert!(cfg(&["[::1]:8080"]).validate().is_ok());
    }

    #[test]
    fn rfc1918_lan_is_accepted() {
        assert!(cfg(&["10.0.0.5:9000"]).validate().is_ok());
        assert!(cfg(&["172.16.5.5:9000"]).validate().is_ok());
        assert!(cfg(&["192.168.1.100:9000"]).validate().is_ok());
    }

    #[test]
    fn ipv6_ula_is_accepted() {
        // fc00::/7 covers fc00::/8 and fd00::/8.
        assert!(cfg(&["[fd00::1]:8080"]).validate().is_ok());
        assert!(cfg(&["[fc00::abcd]:9000"]).validate().is_ok());
    }

    #[test]
    fn cloud_ipv4_is_rejected() {
        let err = cfg(&["8.8.8.8:443"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointNotLocal { .. }
        ));
    }

    #[test]
    fn ipv4_mapped_ipv6_cloud_is_rejected() {
        // `[::ffff:8.8.8.8]:443` — must unmap and reject on the V4 path.
        let err = cfg(&["[::ffff:8.8.8.8]:443"]).validate().unwrap_err();
        assert!(
            matches!(err, VoiceConfigError::TestimonyEndpointNotLocal { .. }),
            "expected TestimonyEndpointNotLocal, got {err:?}"
        );
    }

    #[test]
    fn ipv4_mapped_ipv6_loopback_is_accepted() {
        // `[::ffff:127.0.0.1]:8080` — should unmap and pass.
        assert!(cfg(&["[::ffff:127.0.0.1]:8080"]).validate().is_ok());
    }

    #[test]
    fn unspecified_ipv4_is_rejected() {
        // 0.0.0.0 is not loopback and not RFC1918 — reject explicitly to
        // avoid ambiguity about "any interface". Operators must name a
        // concrete local IP.
        let err = cfg(&["0.0.0.0:8080"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointNotLocal { .. }
        ));
    }

    #[test]
    fn link_local_ipv6_is_rejected() {
        // fe80::/10 is not ULA and not loopback — reject.
        let err = cfg(&["[fe80::1]:8080"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointNotLocal { .. }
        ));
    }

    #[test]
    fn hostname_is_rejected() {
        let err = cfg(&["whisper.example.com:8080"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointIsHostname { .. }
        ));
    }

    #[test]
    fn empty_endpoint_is_rejected() {
        let err = cfg(&[""]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointUnparseable { .. }
        ));
    }

    #[test]
    fn missing_port_is_rejected() {
        let err = cfg(&["127.0.0.1"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointUnparseable { .. }
        ));
    }

    #[test]
    fn empty_port_is_rejected() {
        let err = cfg(&["127.0.0.1:"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointUnparseable { .. }
        ));
    }

    #[test]
    fn out_of_range_port_is_rejected() {
        let err = cfg(&["127.0.0.1:99999"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointUnparseable { .. }
        ));
    }

    #[test]
    fn endpoint_with_path_is_rejected() {
        // SocketAddr can't parse this; the slow path treats "8080/path" as
        // an invalid port.
        let err = cfg(&["127.0.0.1:8080/path"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointUnparseable { .. }
        ));
    }

    #[test]
    fn unbracketed_ipv6_is_rejected() {
        // `::1:8080` is ambiguous — SocketAddr rejects it and the slow
        // path reports "IPv6 must be bracketed".
        let err = cfg(&["::1:8080"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointUnparseable { .. }
        ));
    }

    #[test]
    fn first_offender_stops_validation() {
        // Fail-closed on the first bad endpoint — the operator gets an
        // actionable error rather than an aggregated list to walk through.
        let cfg = cfg(&["127.0.0.1:8080", "8.8.8.8:443", "192.168.1.1:9000"]);
        let err = cfg.validate().unwrap_err();
        match err {
            VoiceConfigError::TestimonyEndpointNotLocal { endpoint, .. } => {
                assert_eq!(endpoint, "8.8.8.8:443");
            }
            other => panic!("expected TestimonyEndpointNotLocal, got {other:?}"),
        }
    }
}
