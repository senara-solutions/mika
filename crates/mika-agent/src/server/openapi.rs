//! OpenAPI spec generation for the mika-server (agent) HTTP API.

use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

use super::embedded_dashboard;
use super::handlers;
use super::types;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Mika Agent API (mika-server)",
        description = "Per-customer agent container HTTP API. Receives messages from the gateway, runs the agent loop, and delivers responses asynchronously.",
        version = "0.1.0"
    ),
    paths(
        handlers::handle_health,
        handlers::handle_message,
        handlers::handle_task_complete,
        embedded_dashboard::handle_enable,
        embedded_dashboard::handle_disable,
        embedded_dashboard::handle_status,
    ),
    components(schemas(
        types::MessageRequest,
        types::AcceptedResponse,
        types::HealthResponse,
        types::TaskCompleteRequest,
        types::TaskCompleteResponse,
    )),
    modifiers(&SecurityAddon),
)]
pub struct AgentApiDoc;

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
                            "Shared secret between gateway and agent container (MIKA_INTERNAL_TOKEN)",
                        ))
                        .build(),
                ),
            );
        }
    }
}

/// Serialize the agent OpenAPI spec to YAML.
pub fn agent_openapi_yaml() -> String {
    let doc = AgentApiDoc::openapi();
    doc.to_yaml().expect("OpenAPI YAML serialization failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_openapi_yaml_contains_endpoints() {
        let yaml = agent_openapi_yaml();
        assert!(yaml.contains("/health"), "missing /health endpoint");
        assert!(yaml.contains("/message"), "missing /message endpoint");
        assert!(
            yaml.contains("/api/v1/dashboard/enable"),
            "missing /api/v1/dashboard/enable endpoint"
        );
        assert!(
            yaml.contains("/api/v1/dashboard/disable"),
            "missing /api/v1/dashboard/disable endpoint"
        );
        assert!(
            yaml.contains("/api/v1/dashboard/status"),
            "missing /api/v1/dashboard/status endpoint"
        );
        assert!(
            yaml.contains("/tasks/{id}/complete"),
            "missing /tasks/{{id}}/complete endpoint"
        );
    }

    #[test]
    fn test_agent_openapi_yaml_is_valid() {
        let yaml = agent_openapi_yaml();
        assert!(yaml.contains("openapi:"));
        assert!(yaml.contains("Mika Agent API"));
    }

    #[test]
    fn test_committed_spec_is_current() {
        let generated = agent_openapi_yaml();
        let committed = include_str!("../../docs/openapi/mika-server.yaml");
        assert_eq!(
            generated, committed,
            "Agent OpenAPI spec is out of date. Run ./scripts/generate-openapi.sh to update."
        );
    }

    /// Write the agent OpenAPI YAML to docs/openapi/mika-server.yaml.
    /// Run via: cargo test -p mika-agent server::openapi::tests::write_agent_openapi_yaml -- --ignored
    #[test]
    #[ignore]
    fn write_agent_openapi_yaml() {
        let yaml = agent_openapi_yaml();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/openapi/mika-server.yaml");
        std::fs::write(&path, &yaml).expect("failed to write mika-server.yaml");
        println!("Wrote {}", path.display());
    }
}
