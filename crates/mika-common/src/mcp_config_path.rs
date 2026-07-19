//! Operator-shell scoped MCP config path resolver (mika#1737 sub-G, AC3+AC4).
//!
//! Ratified architectural disposition: MCP config is CLI/operator-side
//! (edit-time), not per-agent or spirit-runtime. This module owns the
//! single source of truth for the config file location so both the CLI
//! (`mika mcp add/remove/list`) and mika-spirit (session start MCP init)
//! read/write the same path without inter-process coordination.
//!
//! ## Resolution chain (first hit wins)
//!
//! 1. `MIKA_MCP_CONFIG` env var — absolute path override. Load-bearing
//!    for tests and non-XDG hosts.
//! 2. `$XDG_CONFIG_HOME/mika/mcp-servers.json` — XDG-compliant path.
//! 3. `$HOME/.config/mika/mcp-servers.json` — POSIX fallback.
//! 4. `./mcp-servers.json` — CWD fallback (last resort; the caller
//!    should treat this as a warning-worthy configuration state).
//!
//! Format: JSON, matching the pre-existing `mcp.json` schema used by
//! per-agent config (`crates/mika-agent/src/mcp/config.rs`). Keeping the
//! format identical minimizes migration risk and lets an operator hand-
//! copy the file across the boundary if the automatic migration hasn't
//! run yet.

use std::path::PathBuf;

/// Environment variable that overrides the resolved path. Absolute path
/// expected; a relative value is treated as an error at caller layer.
pub const MCP_CONFIG_ENV: &str = "MIKA_MCP_CONFIG";

/// Filename under the config directory. Kept as `.json` to preserve the
/// current serialization format (mika#1737 ships JSON, not TOML — the
/// ratified disposition permits either but JSON minimizes migration risk).
pub const MCP_CONFIG_FILENAME: &str = "mcp-servers.json";

/// Subdirectory under `$XDG_CONFIG_HOME` / `$HOME/.config` for mika
/// operator-shell config files.
pub const MIKA_CONFIG_SUBDIR: &str = "mika";

/// Source tag identifying which resolution tier produced the returned
/// path. Useful for diagnostic logging and for callers that want to WARN
/// when the CWD fallback fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpConfigPathSource {
    /// `MIKA_MCP_CONFIG` env override.
    EnvOverride,
    /// `$XDG_CONFIG_HOME/mika/`.
    XdgConfigHome,
    /// `$HOME/.config/mika/`.
    HomeDotConfig,
    /// Current working directory fallback. Callers SHOULD log a warning
    /// when they see this — it usually indicates neither XDG nor HOME is
    /// set (containerized deployment without a user profile).
    CwdFallback,
}

/// Resolve the operator-shell MCP config path with source-tier reporting.
/// Never fails; the fallback tier (`CwdFallback`) always returns a usable
/// path even in degenerate environments. Callers may WARN on
/// `CwdFallback`.
pub fn resolve_operator_mcp_config_path() -> (PathBuf, McpConfigPathSource) {
    if let Ok(raw) = std::env::var(MCP_CONFIG_ENV)
        && !raw.trim().is_empty()
    {
        return (PathBuf::from(raw), McpConfigPathSource::EnvOverride);
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.trim().is_empty()
    {
        return (
            PathBuf::from(xdg)
                .join(MIKA_CONFIG_SUBDIR)
                .join(MCP_CONFIG_FILENAME),
            McpConfigPathSource::XdgConfigHome,
        );
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        return (
            PathBuf::from(home)
                .join(".config")
                .join(MIKA_CONFIG_SUBDIR)
                .join(MCP_CONFIG_FILENAME),
            McpConfigPathSource::HomeDotConfig,
        );
    }
    (
        PathBuf::from(MCP_CONFIG_FILENAME),
        McpConfigPathSource::CwdFallback,
    )
}

/// Legacy per-agent MCP config path — `{agent_home}/mcp.json`. Kept as a
/// helper so the AC5 one-shot migration has one source of truth for
/// where to copy from.
pub fn legacy_per_agent_mcp_path(agent_home: &std::path::Path) -> PathBuf {
    agent_home.join("mcp.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn clear_env() {
        // SAFETY: single-threaded test bodies via serial_test.
        unsafe {
            std::env::remove_var(MCP_CONFIG_ENV);
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("HOME");
        }
    }

    #[test]
    #[serial]
    fn env_override_wins() {
        clear_env();
        unsafe {
            std::env::set_var(MCP_CONFIG_ENV, "/custom/mcp.json");
            std::env::set_var("XDG_CONFIG_HOME", "/xdg");
            std::env::set_var("HOME", "/home/u");
        }
        let (path, source) = resolve_operator_mcp_config_path();
        assert_eq!(path, PathBuf::from("/custom/mcp.json"));
        assert_eq!(source, McpConfigPathSource::EnvOverride);
        clear_env();
    }

    #[test]
    #[serial]
    fn xdg_config_home_second_priority() {
        clear_env();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", "/xdg");
            std::env::set_var("HOME", "/home/u");
        }
        let (path, source) = resolve_operator_mcp_config_path();
        assert_eq!(path, PathBuf::from("/xdg/mika/mcp-servers.json"));
        assert_eq!(source, McpConfigPathSource::XdgConfigHome);
        clear_env();
    }

    #[test]
    #[serial]
    fn home_dot_config_third_priority() {
        clear_env();
        unsafe { std::env::set_var("HOME", "/home/u") };
        let (path, source) = resolve_operator_mcp_config_path();
        assert_eq!(path, PathBuf::from("/home/u/.config/mika/mcp-servers.json"));
        assert_eq!(source, McpConfigPathSource::HomeDotConfig);
        clear_env();
    }

    #[test]
    #[serial]
    fn cwd_fallback_when_no_env() {
        clear_env();
        let (path, source) = resolve_operator_mcp_config_path();
        assert_eq!(path, PathBuf::from("mcp-servers.json"));
        assert_eq!(source, McpConfigPathSource::CwdFallback);
    }

    #[test]
    #[serial]
    fn empty_env_treated_as_unset() {
        clear_env();
        unsafe {
            std::env::set_var(MCP_CONFIG_ENV, "");
            std::env::set_var("XDG_CONFIG_HOME", "  ");
            std::env::set_var("HOME", "/home/u");
        }
        let (path, source) = resolve_operator_mcp_config_path();
        assert_eq!(path, PathBuf::from("/home/u/.config/mika/mcp-servers.json"));
        assert_eq!(source, McpConfigPathSource::HomeDotConfig);
        clear_env();
    }

    #[test]
    fn legacy_path_is_per_agent_mcp_json() {
        let agent_home = std::path::Path::new("/root/agents/mika");
        let path = legacy_per_agent_mcp_path(agent_home);
        assert_eq!(path, PathBuf::from("/root/agents/mika/mcp.json"));
    }
}
