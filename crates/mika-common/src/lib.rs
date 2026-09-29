pub mod agent;
pub mod auth_boundary;
pub mod build_info;
pub mod claude;
pub mod config;
/// Point de terminaison de boucle locale **réservé** et refusant les connexions
/// (mika#2569). Test-only par construction : il n'entre dans aucun binaire de
/// production, comme [`source_guard`].
#[cfg(any(test, feature = "test-utils"))]
pub mod dead_endpoint;
pub mod dotenv;
pub mod embedding;
pub mod forge_identity;
pub mod github_app;
pub mod github_event_format;
pub mod home;
pub mod label_write;
pub mod llm;
pub mod logging;
pub mod mcp_config_path;
pub mod oauth;
pub mod permission_authority;
/// Single reader of the production/test boundary for this repo's structural
/// guards (mika#2398). Test-only by construction: it enters no production
/// binary, which is how R7 is guaranteed rather than asserted.
#[cfg(any(test, feature = "test-utils"))]
pub mod source_guard;
pub mod team;
pub mod telegram;
pub mod telemetry;
pub mod text;
pub mod trace;
pub mod validation;
