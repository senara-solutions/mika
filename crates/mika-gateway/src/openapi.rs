//! OpenAPI spec generation for the mika-gateway HTTP API.
//!
//! The structs and functions here are used by the `generate-openapi.sh` script
//! (via `#[ignore]` tests) to produce `docs/openapi/gateway.yaml`.

use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

use crate::routes;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Mika Gateway API",
        description = "Shared routing layer for Telegram/WhatsApp webhooks. Receives inbound messages, routes to per-customer agent containers, and relays outbound responses.",
        version = "0.1.0"
    ),
    paths(
        routes::handle_webhook,
        routes::handle_send,
        routes::handle_readiness,
        routes::handle_liveness,
        routes::handle_version,
        // github::handle_github_webhook is excluded from OpenAPI because it
        // consumes raw Bytes for HMAC validation (not JSON-schema-describable).
    ),
    components(schemas(
        routes::SendPayload,
        routes::VersionInfo,
    )),
    modifiers(&SecurityAddon),
)]
pub struct GatewayApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("token")
                        .description(Some(
                            "Shared secret between gateway and agent containers (MIKA_INTERNAL_TOKEN)",
                        ))
                        .build(),
                ),
            );
        }
    }
}

/// Serialize the gateway OpenAPI spec to YAML.
pub fn gateway_openapi_yaml() -> String {
    let doc = GatewayApiDoc::openapi();
    doc.to_yaml().expect("OpenAPI YAML serialization failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gateway_openapi_yaml_contains_endpoints() {
        let yaml = gateway_openapi_yaml();
        assert!(
            yaml.contains("/webhook/telegram"),
            "missing /webhook/telegram endpoint"
        );
        assert!(yaml.contains("/send"), "missing /send endpoint");
        assert!(yaml.contains("/readyz"), "missing /readyz endpoint");
        assert!(yaml.contains("/livez"), "missing /livez endpoint");
    }

    #[test]
    fn test_gateway_openapi_yaml_is_valid() {
        let yaml = gateway_openapi_yaml();
        assert!(yaml.contains("openapi:"));
        assert!(yaml.contains("Mika Gateway API"));
    }

    #[test]
    fn test_committed_spec_is_current() {
        let generated = gateway_openapi_yaml();
        let committed = include_str!("../../../docs/openapi/gateway.yaml");
        assert_eq!(
            generated, committed,
            "Gateway OpenAPI spec is out of date. Run ./scripts/generate-openapi.sh to update."
        );
    }

    /// Write the gateway OpenAPI YAML to docs/openapi/gateway.yaml.
    /// Run via: cargo test -p mika-gateway openapi::tests::write_gateway_openapi_yaml -- --ignored
    #[test]
    #[ignore]
    fn write_gateway_openapi_yaml() {
        let yaml = gateway_openapi_yaml();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/openapi/gateway.yaml");
        std::fs::write(&path, &yaml).expect("failed to write gateway.yaml");
        println!("Wrote {}", path.display());
    }
}
