//! Runtime config validator for the testimony lane (mika#1796) — defense in depth.
//!
//! The type system already prevents mis-wiring a cloud provider into the
//! testimony lane at compile time (see [`crate::voice::room::VoiceRoom`]).
//! This validator addresses a *different* failure mode: an operator hand-edits
//! a config file to point a testimony-lane provider at a cloud URL, thinking
//! "it's just a URL, the provider is local". At startup, this check rejects
//! anything that isn't loopback (`127.0.0.1` / `::1`) or an RFC1918 LAN
//! address — and fails closed (gateway refuses to start, not warn+continue).
//!
//! The scope here is narrow: URL/endpoint literal validation, no DNS
//! resolution. DNS-time non-transit enforcement is the runtime-egress
//! companion ticket's job (see mika#1796 § Out of scope; the companion is
//! `voice(p2.5-runtime): nftables egress deny for testimony ports`).
//!
//! # Companion runtime gate
//!
//! At the network layer, an nftables/iptables deny rule on the gateway host
//! MUST also block testimony ports from reaching WAN. That is intentionally
//! carved out of this file (it is a deployment concern, not a build-time
//! invariant). This validator is one belt; the nftables rule is the
//! suspenders.

use core::net::IpAddr;

use thiserror::Error;

/// Errors produced by [`VoiceConfig::validate`].
#[derive(Debug, Error)]
pub enum VoiceConfigError {
    /// A testimony-lane endpoint parsed to an IP outside loopback / LAN.
    #[error(
        "testimony endpoint '{endpoint}' resolves to non-local IP {ip} — \
         non-transit invariant requires loopback (127.0.0.1 / ::1) or RFC1918 LAN"
    )]
    TestimonyEndpointNotLocal { endpoint: String, ip: IpAddr },

    /// A testimony-lane endpoint carried a hostname instead of an IP literal.
    /// DNS resolution is out of scope for this validator — hostnames MUST be
    /// resolved to IP literals in config, or the operator must use the
    /// network-layer companion (nftables rule) to enforce non-transit.
    #[error(
        "testimony endpoint '{endpoint}' is a hostname; testimony configs \
         must use an IP literal (loopback or RFC1918) so the non-transit \
         invariant is checkable without DNS at startup"
    )]
    TestimonyEndpointIsHostname { endpoint: String },

    /// The endpoint string could not be parsed as `host:port` or a URL host.
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
    /// `<ip>:<port>` string (IP literal, not hostname). The validator
    /// rejects anything outside loopback or RFC1918 LAN.
    pub testimony_endpoints: Vec<String>,
}

impl VoiceConfig {
    /// Reject any testimony endpoint that is not loopback or RFC1918 LAN.
    /// Fails closed on first offender — the gateway is expected to `?`-bubble
    /// the error at startup rather than log-and-continue.
    pub fn validate(&self) -> Result<(), VoiceConfigError> {
        for endpoint in &self.testimony_endpoints {
            let ip = parse_endpoint_ip(endpoint)?;
            if !is_loopback_or_rfc1918(ip) {
                return Err(VoiceConfigError::TestimonyEndpointNotLocal {
                    endpoint: endpoint.clone(),
                    ip,
                });
            }
        }
        Ok(())
    }
}

/// Parse an `<ip>:<port>` endpoint string into just the [`IpAddr`].
///
/// Accepts:
/// - `127.0.0.1:8080`
/// - `[::1]:8080`
/// - `10.0.0.5:9000`
///
/// Rejects (returns `TestimonyEndpointIsHostname` or `TestimonyEndpointUnparseable`):
/// - `whisper.example.com:8080` (hostname — DNS-time check is out of scope)
/// - `not-an-endpoint`
fn parse_endpoint_ip(endpoint: &str) -> Result<IpAddr, VoiceConfigError> {
    // Try IPv6-with-brackets first: `[::1]:8080`.
    if let Some(after_bracket) = endpoint.strip_prefix('[') {
        if let Some(close) = after_bracket.find(']') {
            let host = &after_bracket[..close];
            return host.parse::<IpAddr>().map_err(|e| {
                VoiceConfigError::TestimonyEndpointUnparseable {
                    endpoint: endpoint.to_string(),
                    reason: format!("bracketed IPv6 parse failed: {e}"),
                }
            });
        }
        return Err(VoiceConfigError::TestimonyEndpointUnparseable {
            endpoint: endpoint.to_string(),
            reason: "opening '[' without closing ']'".to_string(),
        });
    }

    // IPv4-with-port form: split on the LAST ':' so we don't confuse an
    // unbracketed IPv6 (which would fail the IP parse below anyway).
    let host = match endpoint.rsplit_once(':') {
        Some((h, _port)) => h,
        None => endpoint,
    };

    // Parse as IP. If parse fails, treat non-empty-non-IP as a hostname
    // (the operator MUST use an IP literal for testimony configs).
    match host.parse::<IpAddr>() {
        Ok(ip) => Ok(ip),
        Err(_) => {
            if host.is_empty() {
                Err(VoiceConfigError::TestimonyEndpointUnparseable {
                    endpoint: endpoint.to_string(),
                    reason: "empty host".to_string(),
                })
            } else {
                Err(VoiceConfigError::TestimonyEndpointIsHostname {
                    endpoint: endpoint.to_string(),
                })
            }
        }
    }
}

/// `true` if the address is loopback (`127.0.0.0/8`, `::1`) or an RFC1918
/// LAN address (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`).
fn is_loopback_or_rfc1918(ip: IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => v4.is_private(),
        // For IPv6, only loopback (::1) is treated as local by this
        // validator. RFC4193 unique-local (`fc00::/7`) is intentionally NOT
        // whitelisted here — the testimony lane's early deployments target
        // IPv4 loopback/RFC1918; a v6 ULA expansion should land explicitly.
        IpAddr::V6(_) => false,
    }
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
    fn cloud_ipv4_is_rejected() {
        // 8.8.8.8 stands in for any WAN address — the invariant says NO.
        let err = cfg(&["8.8.8.8:443"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointNotLocal { .. }
        ));
    }

    #[test]
    fn hostname_is_rejected() {
        // Hostnames could resolve to anything at runtime; the validator
        // requires an IP literal so the non-transit check is decidable at
        // startup without DNS.
        let err = cfg(&["whisper.example.com:8080"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointIsHostname { .. }
        ));
    }

    #[test]
    fn unparseable_endpoint_is_rejected() {
        let err = cfg(&["[oops"]).validate().unwrap_err();
        assert!(matches!(
            err,
            VoiceConfigError::TestimonyEndpointUnparseable { .. }
        ));
    }

    #[test]
    fn first_offender_stops_validation() {
        // Fail-closed on the first bad endpoint — the operator gets an
        // actionable error rather than an aggregated list they have to
        // walk through.
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
