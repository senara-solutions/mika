//! Integration tests for the operator-shell MCP config surface (mika#1737).
//!
//! Covers:
//! - AC1: `commands/mcp.rs` writes to the operator-shell path — indirectly
//!   via `McpConfig::save_operator_shell` + roundtrip.
//! - AC3/AC4: spirit reads from the same operator-shell path (path
//!   detectable without CLI coordination — env-driven resolution chain).
//! - AC5: one-shot migration from `{agent_home}/mcp.json` on first
//!   invocation; idempotent thereafter.

use std::collections::HashMap;

use mika_agent::mcp::config::{McpConfig, McpServerConfig, McpTransport};
use mika_common::mcp_config_path::MCP_CONFIG_ENV;
use serial_test::serial;

fn scratch_config_and_env(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    let path = tmp.path().join("mcp-servers.json");
    // SAFETY: single-threaded via serial_test.
    unsafe {
        std::env::set_var(MCP_CONFIG_ENV, &path);
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HOME");
    }
    path
}

fn cleanup_env() {
    unsafe {
        std::env::remove_var(MCP_CONFIG_ENV);
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("HOME");
    }
}

fn stdio_server(command: &str) -> McpServerConfig {
    McpServerConfig {
        transport: McpTransport::Stdio,
        command: Some(command.to_string()),
        args: None,
        env: None,
        url: None,
        headers: None,
        enabled: true,
    }
}

/// AC1/AC3/AC4: save via CLI-side path (`save_operator_shell`) and
/// load from server-side path (`load_operator_shell`) roundtrips a
/// non-trivial config.
#[test]
#[serial]
fn save_operator_shell_and_load_operator_shell_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let _path = scratch_config_and_env(&tmp);

    let mut config = McpConfig::default();
    config
        .mcp_servers
        .insert("filesystem".to_string(), stdio_server("npx"));
    config.mcp_servers.insert("disabled".to_string(), {
        let mut c = stdio_server("echo");
        c.enabled = false;
        c
    });
    config.save_operator_shell().unwrap();

    let loaded = McpConfig::load_operator_shell().unwrap();
    assert_eq!(loaded.mcp_servers.len(), 2);
    assert!(loaded.mcp_servers.contains_key("filesystem"));
    assert!(loaded.mcp_servers.contains_key("disabled"));
    assert!(loaded.mcp_servers["filesystem"].enabled);
    assert!(!loaded.mcp_servers["disabled"].enabled);

    cleanup_env();
}

/// AC3/AC4: `load_operator_shell` on a missing file returns an empty
/// config (no error), so a fresh install boots cleanly with zero MCP
/// servers.
#[test]
#[serial]
fn load_operator_shell_missing_file_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let _path = scratch_config_and_env(&tmp);
    let loaded = McpConfig::load_operator_shell().unwrap();
    assert!(loaded.mcp_servers.is_empty());
    cleanup_env();
}

/// AC5: one-shot migration copies `{agent_home}/mcp.json` to the
/// operator-shell path when the operator-shell path does not exist.
#[test]
#[serial]
fn migrate_from_agent_home_populates_operator_shell() {
    let tmp = tempfile::tempdir().unwrap();
    let operator_path = scratch_config_and_env(&tmp);

    let agent_home = tmp.path().join("agent");
    std::fs::create_dir_all(&agent_home).unwrap();
    let mut per_agent = McpConfig::default();
    per_agent
        .mcp_servers
        .insert("legacy".to_string(), stdio_server("legacy-cmd"));
    per_agent.save(&agent_home).unwrap();

    assert!(!operator_path.exists());
    let migrated = McpConfig::migrate_from_agent_home_if_needed(&agent_home).unwrap();
    assert!(migrated);
    assert!(operator_path.exists());

    let loaded = McpConfig::load_operator_shell().unwrap();
    assert_eq!(loaded.mcp_servers.len(), 1);
    assert_eq!(
        loaded.mcp_servers["legacy"].command.as_deref(),
        Some("legacy-cmd")
    );

    cleanup_env();
}

/// AC5: migration is a no-op when the operator-shell path already
/// exists (even if the per-agent legacy path also exists).
#[test]
#[serial]
fn migrate_is_idempotent_when_operator_shell_populated() {
    let tmp = tempfile::tempdir().unwrap();
    let operator_path = scratch_config_and_env(&tmp);

    // Pre-populate operator-shell path with a distinct config.
    let mut existing = McpConfig::default();
    existing
        .mcp_servers
        .insert("kept".to_string(), stdio_server("kept-cmd"));
    existing.save_operator_shell().unwrap();
    assert!(operator_path.exists());

    // Legacy per-agent file has a DIFFERENT server.
    let agent_home = tmp.path().join("agent");
    std::fs::create_dir_all(&agent_home).unwrap();
    let mut per_agent = McpConfig::default();
    per_agent
        .mcp_servers
        .insert("dropped".to_string(), stdio_server("dropped-cmd"));
    per_agent.save(&agent_home).unwrap();

    // Migration must NOT overwrite the operator-shell content.
    let migrated = McpConfig::migrate_from_agent_home_if_needed(&agent_home).unwrap();
    assert!(!migrated);

    let loaded = McpConfig::load_operator_shell().unwrap();
    assert_eq!(loaded.mcp_servers.len(), 1);
    assert!(loaded.mcp_servers.contains_key("kept"));
    assert!(!loaded.mcp_servers.contains_key("dropped"));

    cleanup_env();
}

/// AC5: migration is a no-op when neither the operator-shell path nor
/// the per-agent path exists (fresh install).
#[test]
#[serial]
fn migrate_is_noop_on_fresh_install() {
    let tmp = tempfile::tempdir().unwrap();
    let _operator_path = scratch_config_and_env(&tmp);
    let agent_home = tmp.path().join("agent");
    std::fs::create_dir_all(&agent_home).unwrap();

    let migrated = McpConfig::migrate_from_agent_home_if_needed(&agent_home).unwrap();
    assert!(!migrated);

    cleanup_env();
}

/// AC1: headers with secret values roundtrip through save/load on the
/// operator-shell path (regression cover — the same pattern the
/// per-agent path already tests).
#[test]
#[serial]
fn headers_roundtrip_through_operator_shell() {
    let tmp = tempfile::tempdir().unwrap();
    let _path = scratch_config_and_env(&tmp);

    let mut config = McpConfig::default();
    let mut headers = HashMap::new();
    headers.insert("Authorization".to_string(), "Bearer secret-tok".to_string());
    let mut server = McpServerConfig {
        transport: McpTransport::Http,
        command: None,
        args: None,
        env: None,
        url: Some("https://api.example.com/mcp".to_string()),
        headers: Some(headers),
        enabled: true,
    };
    server.headers = server.headers.take();
    config.mcp_servers.insert("api".to_string(), server);
    config.save_operator_shell().unwrap();

    let loaded = McpConfig::load_operator_shell().unwrap();
    let got = loaded.mcp_servers["api"].headers.as_ref().unwrap();
    assert_eq!(got["Authorization"], "Bearer secret-tok");

    cleanup_env();
}
