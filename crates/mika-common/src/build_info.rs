//! Build-time provenance stamp (mika#2066).
//!
//! Every mika binary must be able to state the commit it was built from without
//! a configured server or database — a deploy is then verified by interrogating
//! the binary (`--version`), not only by reasoning about provenance.
//!
//! [`GIT_HASH`] is injected by this crate's `build.rs` at compile time. Because
//! `option_env!` resolves in the crate whose build script set the variable, the
//! capture lives HERE, once, and `mika`, `mika-gateway`, and `mika-spirit` all
//! read it through this module (AC3).

/// Short git commit the binary was built from, or `"unknown"` when built outside
/// a git checkout (Docker layer, source tarball). Never empty.
pub const GIT_HASH: &str = match option_env!("GIT_HASH") {
    Some(h) => h,
    None => "unknown",
};

/// Workspace semantic version (`[workspace.package] version` in `Cargo.toml`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `"0.12.2 (968dbe94)"` — the semantic version plus the commit stamp. The
/// canonical value behind every binary's `--version`.
pub fn version_string() -> String {
    format!("{VERSION} ({GIT_HASH})")
}

/// [`version_string`] as a `&'static str`, computed once. clap's `version`
/// builder takes `&'static str`, not `String`, so `mika-cli` wires its
/// `--version` through this.
pub fn version_static() -> &'static str {
    use std::sync::OnceLock;
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(version_string).as_str()
}

/// If the process was invoked with `--version` or `-V`, print
/// `"{bin_name} {version} ({hash})"` to stdout and exit 0.
///
/// Call this as the FIRST statement of `main`, before any config, database, or
/// env initialization, so an unconfigured binary can still state its provenance
/// (mika#2066 AC2). Binaries that parse args with clap get the same output from
/// clap's own `--version` handling and do not need this helper.
pub fn print_version_if_requested(bin_name: &str) {
    if std::env::args()
        .skip(1)
        .any(|a| a == "--version" || a == "-V")
    {
        println!("{bin_name} {}", version_string());
        std::process::exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_hash_is_never_empty() {
        // Either the real short commit (in a git checkout) or the explicit
        // "unknown" fallback — but never empty. A binary that cannot state a
        // stamp is exactly the failure mika#2066 removes.
        assert!(!GIT_HASH.is_empty());
    }

    #[test]
    fn version_string_pairs_semver_with_stamp() {
        let v = version_string();
        assert!(v.starts_with(VERSION));
        assert!(v.contains('('));
        assert!(v.contains(GIT_HASH));
    }
}
