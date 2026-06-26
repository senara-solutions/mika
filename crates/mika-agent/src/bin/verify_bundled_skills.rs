//! `verify-bundled-skills` — structural-invariants gate for the engine-coupled
//! bundled skills under `skills/bundled/` (mika#1575).
//!
//! This is the **pre-merge structural counterpart to mika#1326 AC2**. AC2
//! (`test_bundled_skills_no_cross_skill_tool_name_collision`) catches cross-skill
//! tool-name *collisions* at build-test time. This gate catches *incomplete
//! skill-adds* — the gap between "skill bundle files exist" and "skill bundle works
//! in production" — at pre-merge time, so the operator's review of a mika#1282
//! dirty-worktree-rescued draft PR is reduced from "verify structure" to "verify
//! content".
//!
//! Five checks (see `docs/architecture/bundled-skill-verification.md`):
//!   1. Bundle completeness — `skill.toml` + `system_prompt.md` present.
//!   2. Manifest parses + minimum fields.
//!   3. Tool handler resolution (builtin function known; exec command file exists + executable).
//!   4. `required_tools` token consistency (allowlist-unaware — token exists in the bundled surface).
//!   5. Identity allowlist coherence (allowlist names exist; tokens reachable through the allowlist).
//!
//! Fire-disposition (mika#1575 F2): all five checks ship GREEN against the current
//! source tree; `KNOWN_EXCEPTIONS` ships EMPTY. The binary exits non-zero on any
//! unallowlisted failure or any stale exception; CI blocks merge on non-zero exit.
//!
//! Check-1 predicate per mika-arch third-pass ratification (session d8b4c839,
//! Decision A): `skill.toml` + `system_prompt.md` presence only — the
//! `required_tools ⇒ tools.json` coupling was dropped as unsound (a `required_tools`
//! token may resolve via a builtin or another skill, so its presence cannot proxy
//! "this skill ships its own tools.json"). Token validity is owned by Check 4.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mika_agent::bundled_skills::{
    all_bundled_tool_names, bundled_skill_tool_map, is_bundled_skill,
};
use mika_agent::skills::builtin_handlers::KNOWN_BUILTINS;
use mika_agent::tools::BUILTIN_TOOL_NAMES;
use mika_agent::well_known_agents::well_known_skill_allowlists;

// Reuse the canonical on-disk skill walker (shared with build.rs + integration tests).
#[path = "../../build_support/bundled_skills_discover.rs"]
mod discover;

/// A known, deliberately-allowed pre-existing check failure.
///
/// Mirrors `KNOWN_PRE_EXISTING_COLLISIONS` in `bundled_skills.rs` (mika#1326 AC2):
/// each entry names the check, the skill, a substring the failure reason must
/// contain, and the follow-up ticket that will resolve it. **Self-cleaning**: the
/// binary (and a test) fail when an entry no longer matches a real failure, forcing
/// removal of stale exceptions.
///
/// Per mika#1575 F2 this ships **EMPTY**. A non-empty entry MUST be enumerated in
/// the PR description with its resolution ticket.
struct KnownException {
    check: u8,
    skill: &'static str,
    reason_substr: &'static str,
    #[allow(dead_code)]
    ticket: &'static str,
}

const KNOWN_EXCEPTIONS: &[KnownException] = &[];

/// A single structural-invariant violation.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    check: u8,
    skill: String,
    reason: String,
}

impl Failure {
    fn new(check: u8, skill: impl Into<String>, reason: impl Into<String>) -> Self {
        Failure {
            check,
            skill: skill.into(),
            reason: reason.into(),
        }
    }
}

/// Parsed `skill.toml` fields the checks care about.
struct Manifest {
    name: Option<String>,
    version: Option<String>,
    always_on: bool,
    /// Whether `[triggers].keywords` is *declared* (as an array). Per the issue's
    /// Check 2 ("contains [triggers] keywords") this is field-presence, NOT
    /// non-emptiness: `skill-review` deliberately declares `keywords = []` (it is a
    /// programmatically-invoked handler skill, inert-by-keyword on purpose, #265).
    keywords_present: bool,
    required_tools: Vec<String>,
}

/// One tool declaration from a skill's `tools.json`.
struct ToolDecl {
    name: Option<String>,
    handler_type: Option<String>,
    function: Option<String>,
    command: Option<String>,
}

/// A parsed view of one engine-coupled bundled skill (`skills/bundled/<name>/`).
struct SkillView {
    name: String,
    /// Absolute skill directory — used by Check 3 to stat exec handler files.
    dir: PathBuf,
    /// Relative paths of files present in the skill directory.
    rel_files: HashSet<String>,
    /// Parsed `skill.toml`, or the parse error message.
    manifest: Result<Manifest, String>,
    /// Parsed `tools.json` (empty when absent), or the parse error message.
    tools: Result<Vec<ToolDecl>, String>,
}

fn parse_manifest(content: &str) -> Result<Manifest, String> {
    let value: toml::Value = toml::from_str(content).map_err(|e| e.to_string())?;
    let str_at = |a: &str, b: &str| -> Option<String> {
        value
            .get(a)
            .and_then(|t| t.get(b))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let arr_at = |a: &str, b: &str| -> Vec<String> {
        value
            .get(a)
            .and_then(|t| t.get(b))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(Manifest {
        name: str_at("skill", "name"),
        version: str_at("skill", "version"),
        always_on: value
            .get("skill")
            .and_then(|s| s.get("always_on"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        keywords_present: value
            .get("triggers")
            .and_then(|t| t.get("keywords"))
            .and_then(|v| v.as_array())
            .is_some(),
        required_tools: arr_at("constraints", "required_tools"),
    })
}

fn parse_tools(content: &str) -> Result<Vec<ToolDecl>, String> {
    let arr: Vec<serde_json::Value> = serde_json::from_str(content).map_err(|e| e.to_string())?;
    Ok(arr
        .iter()
        .map(|t| ToolDecl {
            name: t.get("name").and_then(|v| v.as_str()).map(str::to_string),
            handler_type: t
                .get("handler")
                .and_then(|h| h.get("type"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            function: t
                .get("handler")
                .and_then(|h| h.get("function"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            command: t
                .get("handler")
                .and_then(|h| h.get("command"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
        .collect())
}

/// Build `SkillView`s from the on-disk `skills/bundled/` tree via the shared walker.
fn load_skill_views(base: &Path) -> Vec<SkillView> {
    discover::discover_bundled_skills(base)
        .into_iter()
        .map(|d| {
            let rel_files: HashSet<String> = d.files.iter().map(|f| f.rel_path.clone()).collect();
            let read = |rel: &str| -> Option<String> {
                d.files
                    .iter()
                    .find(|f| f.rel_path == rel)
                    .and_then(|f| std::fs::read_to_string(&f.abs_path).ok())
            };
            let manifest = match read("skill.toml") {
                Some(c) => parse_manifest(&c),
                None => Err("skill.toml not present".to_string()),
            };
            let tools = match read("tools.json") {
                Some(c) => parse_tools(&c),
                None => Ok(Vec::new()),
            };
            SkillView {
                name: d.name,
                dir: d.abs_dir,
                rel_files,
                manifest,
                tools,
            }
        })
        .collect()
}

// ---- Checks -------------------------------------------------------------------

/// Check 1 — bundle completeness. `skill.toml` + `system_prompt.md` always required.
/// (Per mika-arch session d8b4c839 Decision A, the `required_tools ⇒ tools.json`
/// coupling is intentionally NOT enforced here — Check 4 owns token validity.)
fn check1_completeness(skills: &[SkillView]) -> Vec<Failure> {
    let mut out = Vec::new();
    for s in skills {
        if !s.rel_files.contains("skill.toml") {
            out.push(Failure::new(1, &s.name, "missing required file skill.toml"));
        }
        if !s.rel_files.contains("system_prompt.md") {
            out.push(Failure::new(
                1,
                &s.name,
                "missing required file system_prompt.md",
            ));
        }
    }
    out
}

/// Check 1 (orphan-directory arm) — flag skill-shaped directories under
/// `skills/bundled/` that lack `skill.toml` entirely.
///
/// The shared walker (`discover_bundled_skills`) only returns a directory once it
/// already contains a real `skill.toml`, so a forgotten manifest is invisible to
/// [`check1_completeness`] (its `SkillView` is never built). That is exactly the
/// incomplete-skill-add class this gate exists to catch (a `skills/bundled/<name>/`
/// with `system_prompt.md` / `tools.json` but no manifest), so it is checked here
/// against the raw directory listing.
fn check1_missing_manifest(base: &Path) -> Vec<Failure> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(base) else {
        return out; // missing base dir is surfaced elsewhere; nothing to flag here
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() || !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // Excludes `_shared`, dotfiles — same predicate the walker uses.
        if !discover::is_bundled_skill_dir(&name) {
            continue;
        }
        // `symlink_metadata` so a symlinked `skill.toml` doesn't count (mirrors the walker).
        let toml = entry.path().join("skill.toml");
        let is_real = std::fs::symlink_metadata(&toml)
            .map(|m| m.file_type().is_file())
            .unwrap_or(false);
        if !is_real {
            out.push(Failure::new(1, name, "missing required file skill.toml"));
        }
    }
    out
}

/// Check 2 — manifest parses + minimum fields.
fn check2_manifest(skills: &[SkillView]) -> Vec<Failure> {
    let mut out = Vec::new();
    for s in skills {
        let m = match &s.manifest {
            Ok(m) => m,
            Err(e) => {
                out.push(Failure::new(
                    2,
                    &s.name,
                    format!("skill.toml does not parse: {e}"),
                ));
                continue;
            }
        };
        if m.name.as_deref().unwrap_or("").is_empty() {
            out.push(Failure::new(2, &s.name, "skill.toml missing [skill].name"));
        }
        if m.version.as_deref().unwrap_or("").is_empty() {
            out.push(Failure::new(
                2,
                &s.name,
                "skill.toml missing [skill].version",
            ));
        }
        if !m.always_on && !m.keywords_present {
            out.push(Failure::new(
                2,
                &s.name,
                "skill.toml missing [triggers].keywords field (required unless always_on = true)",
            ));
        }
    }
    out
}

/// Check 3 — tool handler resolution.
/// `builtin` → `handler.function` must be a known builtin *handler* function.
/// `exec` → `handler.command` file must exist at the skill dir and be executable.
fn check3_handler_resolution(skills: &[SkillView], known_builtins: &HashSet<&str>) -> Vec<Failure> {
    let mut out = Vec::new();
    for s in skills {
        let tools = match &s.tools {
            Ok(t) => t,
            Err(e) => {
                out.push(Failure::new(
                    3,
                    &s.name,
                    format!("tools.json does not parse: {e}"),
                ));
                continue;
            }
        };
        for tool in tools {
            let tool_name = tool.name.as_deref().unwrap_or("<unnamed>");
            match tool.handler_type.as_deref() {
                Some("builtin") => match &tool.function {
                    Some(f) if known_builtins.contains(f.as_str()) => {}
                    Some(f) => out.push(Failure::new(
                        3,
                        &s.name,
                        format!("tool '{tool_name}' builtin handler function '{f}' is not a registered builtin"),
                    )),
                    None => out.push(Failure::new(
                        3,
                        &s.name,
                        format!("tool '{tool_name}' has builtin handler with no 'function'"),
                    )),
                },
                Some("exec") => match &tool.command {
                    Some(cmd) => {
                        let path = s.dir.join(cmd);
                        match std::fs::metadata(&path) {
                            Ok(meta) if meta.is_file() => {
                                if meta.permissions().mode() & 0o111 == 0 {
                                    out.push(Failure::new(
                                        3,
                                        &s.name,
                                        format!("tool '{tool_name}' exec command '{cmd}' is not executable"),
                                    ));
                                }
                            }
                            _ => out.push(Failure::new(
                                3,
                                &s.name,
                                format!("tool '{tool_name}' exec command file '{cmd}' does not exist"),
                            )),
                        }
                    }
                    None => out.push(Failure::new(
                        3,
                        &s.name,
                        format!("tool '{tool_name}' has exec handler with no 'command'"),
                    )),
                },
                // `http` is a deliberately out-of-scope handler type (runtime-resolved).
                Some("http") => {}
                // A missing or unrecognized handler.type is an incomplete declaration
                // the runtime would skip — flag it pre-merge rather than passing silently.
                Some(other) => out.push(Failure::new(
                    3,
                    &s.name,
                    format!("tool '{tool_name}' has unrecognized handler.type '{other}'"),
                )),
                None => out.push(Failure::new(
                    3,
                    &s.name,
                    format!("tool '{tool_name}' has no handler.type"),
                )),
            }
        }
    }
    out
}

/// Check 4 — `required_tools` token consistency (allowlist-unaware, per mika#1575 F1).
/// Each token must exist somewhere in the bundled surface: a builtin tool name OR a
/// tool declared in any bundled skill's `tools.json`.
fn check4_token_consistency(skills: &[SkillView], known_tools: &HashSet<String>) -> Vec<Failure> {
    let mut out = Vec::new();
    for s in skills {
        let Ok(m) = &s.manifest else { continue };
        for token in &m.required_tools {
            if !known_tools.contains(token) {
                out.push(Failure::new(
                    4,
                    &s.name,
                    format!(
                        "required_tools token '{token}' resolves to no builtin and no bundled skill's tools.json"
                    ),
                ));
            }
        }
    }
    out
}

/// Check 5 — identity allowlist coherence (allowlist-scoped, per mika#1575 F1).
/// For each well-known agent allowlist: every allowlisted name exists as a bundled
/// skill, and every `required_tools` token of allowlisted *engine-coupled* skills is
/// reachable through the allowlist (a builtin, or a tool declared by a skill IN that
/// allowlist).
fn check5_identity_coherence(
    skills: &[SkillView],
    allowlists: &[(&'static str, Vec<String>)],
    builtin_tools: &HashSet<String>,
    skill_tool_map: &HashMap<String, Vec<String>>,
) -> Vec<Failure> {
    let mut out = Vec::new();
    let view_by_name: HashMap<&str, &SkillView> =
        skills.iter().map(|s| (s.name.as_str(), s)).collect();

    for (agent, allowlist) in allowlists {
        // (a) every allowlisted name must be a bundled skill.
        for name in allowlist {
            if !is_bundled_skill(name) {
                out.push(Failure::new(
                    5,
                    *agent,
                    format!("allowlist references '{name}' which is not a bundled skill"),
                ));
            }
        }

        // Reachable tool set = builtins ∪ tools declared by skills IN this allowlist.
        let mut reachable: HashSet<String> = builtin_tools.clone();
        for name in allowlist {
            if let Some(tools) = skill_tool_map.get(name) {
                reachable.extend(tools.iter().cloned());
            }
        }

        // (b) every required_tools token of an allowlisted engine-coupled skill must
        //     be reachable through this allowlist.
        for name in allowlist {
            let Some(view) = view_by_name.get(name.as_str()) else {
                continue; // community skill (not in skills/bundled/) — token-bearing engine skills only
            };
            let Ok(m) = &view.manifest else { continue };
            for token in &m.required_tools {
                if !reachable.contains(token) {
                    out.push(Failure::new(
                        5,
                        *agent,
                        format!(
                            "skill '{name}' requires tool '{token}', not reachable through {agent}'s allowlist"
                        ),
                    ));
                }
            }
        }
    }
    out
}

// ---- Aggregation + entrypoint -------------------------------------------------

fn run_all_checks(base: &Path) -> Vec<Failure> {
    let skills = load_skill_views(base);
    let known_builtins: HashSet<&str> = KNOWN_BUILTINS.iter().copied().collect();
    let mut known_tools: HashSet<String> =
        BUILTIN_TOOL_NAMES.iter().map(|s| s.to_string()).collect();
    known_tools.extend(all_bundled_tool_names());
    let skill_tool_map = bundled_skill_tool_map();
    let allowlists = well_known_skill_allowlists();

    let mut failures = Vec::new();
    failures.extend(check1_completeness(&skills));
    failures.extend(check1_missing_manifest(base));
    failures.extend(check2_manifest(&skills));
    failures.extend(check3_handler_resolution(&skills, &known_builtins));
    failures.extend(check4_token_consistency(&skills, &known_tools));
    failures.extend(check5_identity_coherence(
        &skills,
        &allowlists,
        &known_tools_builtins_only(),
        &skill_tool_map,
    ));
    failures
}

/// The builtin-only tool set used for Check 5 reachability (allowlist-scoped):
/// builtins are reachable by every agent regardless of allowlist; skill-provided
/// tools are added per-allowlist inside the check.
fn known_tools_builtins_only() -> HashSet<String> {
    BUILTIN_TOOL_NAMES.iter().map(|s| s.to_string()).collect()
}

/// Does a failure match a known exception entry?
fn matches_exception(f: &Failure, e: &KnownException) -> bool {
    e.check == f.check && e.skill == f.skill && f.reason.contains(e.reason_substr)
}

fn main() -> ExitCode {
    // skills/bundled/ lives at <repo-root>/skills/bundled. CARGO_MANIFEST_DIR is
    // <repo-root>/crates/mika-agent at compile time — robust regardless of runtime CWD.
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../skills/bundled")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills/bundled"));

    println!("verify-bundled-skills: scanning {}", base.display());

    let failures = run_all_checks(&base);

    // Partition failures into allowlisted vs reportable.
    let mut reportable: Vec<&Failure> = Vec::new();
    for f in &failures {
        if !KNOWN_EXCEPTIONS.iter().any(|e| matches_exception(f, e)) {
            reportable.push(f);
        }
    }

    // Self-cleaning: any KNOWN_EXCEPTIONS entry that matches no real failure is stale.
    let stale: Vec<&KnownException> = KNOWN_EXCEPTIONS
        .iter()
        .filter(|e| !failures.iter().any(|f| matches_exception(f, e)))
        .collect();

    for f in &reportable {
        println!("CHECK {} FAIL: {} — {}", f.check, f.skill, f.reason);
    }
    for e in &stale {
        println!(
            "STALE EXCEPTION: check {} / skill '{}' / ticket {} no longer matches any failure — remove it",
            e.check, e.skill, e.ticket
        );
    }

    if reportable.is_empty() && stale.is_empty() {
        println!(
            "OK — all 5 structural checks pass for skills/bundled/ ({} reportable failures).",
            reportable.len()
        );
        ExitCode::SUCCESS
    } else {
        println!(
            "\nFAILED — {} reportable failure(s), {} stale exception(s).",
            reportable.len(),
            stale.len()
        );
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(
        name: Option<&str>,
        version: Option<&str>,
        always_on: bool,
        keywords: &[&str],
        required_tools: &[&str],
    ) -> Manifest {
        // In tests, a non-empty keyword slice models a declared `[triggers].keywords`;
        // an empty slice models an ABSENT field. The present-but-empty case
        // (`keywords = []`, e.g. skill-review) is exercised by the real-tree test.
        Manifest {
            name: name.map(str::to_string),
            version: version.map(str::to_string),
            always_on,
            keywords_present: !keywords.is_empty(),
            required_tools: required_tools.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn view(
        name: &str,
        rel_files: &[&str],
        manifest: Result<Manifest, String>,
        tools: Result<Vec<ToolDecl>, String>,
    ) -> SkillView {
        SkillView {
            name: name.to_string(),
            dir: PathBuf::from("/nonexistent"),
            rel_files: rel_files.iter().map(|s| s.to_string()).collect(),
            manifest,
            tools,
        }
    }

    // ---- Check 1 ----
    #[test]
    fn check1_pass_prompt_only_skill() {
        let s = view(
            "prompt-only",
            &["skill.toml", "system_prompt.md"],
            Ok(manifest(
                Some("prompt-only"),
                Some("0.1.0"),
                false,
                &["kw"],
                &[],
            )),
            Ok(vec![]),
        );
        assert!(check1_completeness(&[s]).is_empty());
    }

    #[test]
    fn check1_pass_required_tools_without_tools_json() {
        // OD-1 ratified shape: required_tools + no tools.json is VALID at Check 1.
        let s = view(
            "self-dev-like",
            &["skill.toml", "system_prompt.md"],
            Ok(manifest(
                Some("self-dev-like"),
                Some("0.1.0"),
                false,
                &["kw"],
                &["run_claude_pilot"],
            )),
            Ok(vec![]),
        );
        assert!(check1_completeness(&[s]).is_empty());
    }

    #[test]
    fn check1_fail_missing_system_prompt() {
        let s = view(
            "broken",
            &["skill.toml"],
            Ok(manifest(Some("broken"), Some("0.1.0"), false, &["kw"], &[])),
            Ok(vec![]),
        );
        let f = check1_completeness(&[s]);
        assert_eq!(f.len(), 1);
        assert!(f[0].reason.contains("system_prompt.md"));
    }

    #[test]
    fn check1_fail_missing_skill_toml() {
        let s = view(
            "broken",
            &["system_prompt.md"],
            Err("skill.toml not present".to_string()),
            Ok(vec![]),
        );
        let f = check1_completeness(&[s]);
        assert!(f.iter().any(|x| x.reason.contains("skill.toml")));
    }

    // ---- Check 2 ----
    #[test]
    fn check2_pass_keyworded_and_always_on() {
        let kw = view(
            "kw",
            &["skill.toml", "system_prompt.md"],
            Ok(manifest(Some("kw"), Some("0.1.0"), false, &["x"], &[])),
            Ok(vec![]),
        );
        let ao = view(
            "ao",
            &["skill.toml", "system_prompt.md"],
            Ok(manifest(Some("ao"), Some("0.1.0"), true, &[], &[])),
            Ok(vec![]),
        );
        assert!(check2_manifest(&[kw, ao]).is_empty());
    }

    #[test]
    fn check2_fail_unparseable_missing_fields_and_keywords() {
        let bad_parse = view("bad", &["skill.toml"], Err("boom".to_string()), Ok(vec![]));
        let no_name = view(
            "noname",
            &["skill.toml"],
            Ok(manifest(None, Some("0.1.0"), false, &["x"], &[])),
            Ok(vec![]),
        );
        let no_ver = view(
            "nover",
            &["skill.toml"],
            Ok(manifest(Some("nover"), None, false, &["x"], &[])),
            Ok(vec![]),
        );
        let no_kw = view(
            "nokw",
            &["skill.toml"],
            Ok(manifest(Some("nokw"), Some("0.1.0"), false, &[], &[])),
            Ok(vec![]),
        );
        let f = check2_manifest(&[bad_parse, no_name, no_ver, no_kw]);
        assert!(
            f.iter()
                .any(|x| x.skill == "bad" && x.reason.contains("does not parse"))
        );
        assert!(
            f.iter()
                .any(|x| x.skill == "noname" && x.reason.contains("name"))
        );
        assert!(
            f.iter()
                .any(|x| x.skill == "nover" && x.reason.contains("version"))
        );
        assert!(
            f.iter()
                .any(|x| x.skill == "nokw" && x.reason.contains("keywords"))
        );
    }

    // ---- Check 3 ----
    fn known_builtins_fixture() -> HashSet<&'static str> {
        ["gh_read", "run_gh"].into_iter().collect()
    }

    #[test]
    fn check3_pass_builtin_and_fail_unknown_builtin() {
        let good = view(
            "good",
            &["skill.toml", "tools.json"],
            Ok(manifest(Some("good"), Some("0.1.0"), false, &["x"], &[])),
            Ok(vec![ToolDecl {
                name: Some("gh_read".into()),
                handler_type: Some("builtin".into()),
                function: Some("gh_read".into()),
                command: None,
            }]),
        );
        assert!(check3_handler_resolution(&[good], &known_builtins_fixture()).is_empty());

        let bad = view(
            "bad",
            &["skill.toml", "tools.json"],
            Ok(manifest(Some("bad"), Some("0.1.0"), false, &["x"], &[])),
            Ok(vec![ToolDecl {
                name: Some("mystery".into()),
                handler_type: Some("builtin".into()),
                function: Some("nonexistent_fn".into()),
                command: None,
            }]),
        );
        let f = check3_handler_resolution(&[bad], &known_builtins_fixture());
        assert_eq!(f.len(), 1);
        assert!(f[0].reason.contains("nonexistent_fn"));
    }

    #[test]
    fn check3_exec_missing_and_nonexecutable() {
        let dir = tempfile::tempdir().unwrap();
        // Missing command file.
        let missing = SkillView {
            name: "missing".into(),
            dir: dir.path().to_path_buf(),
            rel_files: ["skill.toml", "tools.json"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            manifest: Ok(manifest(Some("missing"), Some("0.1.0"), false, &["x"], &[])),
            tools: Ok(vec![ToolDecl {
                name: Some("t".into()),
                handler_type: Some("exec".into()),
                function: None,
                command: Some("handlers/run.sh".into()),
            }]),
        };
        let f = check3_handler_resolution(&[missing], &known_builtins_fixture());
        assert!(f.iter().any(|x| x.reason.contains("does not exist")));

        // Present but non-executable.
        let handlers = dir.path().join("handlers");
        std::fs::create_dir_all(&handlers).unwrap();
        let script = handlers.join("run.sh");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();
        let nonexec = SkillView {
            name: "nonexec".into(),
            dir: dir.path().to_path_buf(),
            rel_files: ["skill.toml", "tools.json"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            manifest: Ok(manifest(Some("nonexec"), Some("0.1.0"), false, &["x"], &[])),
            tools: Ok(vec![ToolDecl {
                name: Some("t".into()),
                handler_type: Some("exec".into()),
                function: None,
                command: Some("handlers/run.sh".into()),
            }]),
        };
        let f = check3_handler_resolution(&[nonexec], &known_builtins_fixture());
        assert!(f.iter().any(|x| x.reason.contains("not executable")));

        // Present and executable → pass.
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let exec = SkillView {
            name: "exec".into(),
            dir: dir.path().to_path_buf(),
            rel_files: ["skill.toml", "tools.json"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            manifest: Ok(manifest(Some("exec"), Some("0.1.0"), false, &["x"], &[])),
            tools: Ok(vec![ToolDecl {
                name: Some("t".into()),
                handler_type: Some("exec".into()),
                function: None,
                command: Some("handlers/run.sh".into()),
            }]),
        };
        assert!(check3_handler_resolution(&[exec], &known_builtins_fixture()).is_empty());
    }

    // ---- Check 4 ----
    #[test]
    fn check4_pass_builtin_own_and_crossskill_fail_nonexistent() {
        let mut known: HashSet<String> =
            ["write_agent_file"].iter().map(|s| s.to_string()).collect();
        known.insert("run_claude_pilot".into()); // cross-skill (option c)
        known.insert("qa_pr_view".into()); // own (option b)

        let ok = view(
            "ok",
            &["skill.toml"],
            Ok(manifest(
                Some("ok"),
                Some("0.1.0"),
                false,
                &["x"],
                &["write_agent_file", "run_claude_pilot", "qa_pr_view"],
            )),
            Ok(vec![]),
        );
        assert!(check4_token_consistency(&[ok], &known).is_empty());

        let bad = view(
            "bad",
            &["skill.toml"],
            Ok(manifest(
                Some("bad"),
                Some("0.1.0"),
                false,
                &["x"],
                &["nonexistent_tool"],
            )),
            Ok(vec![]),
        );
        let f = check4_token_consistency(&[bad], &known);
        assert_eq!(f.len(), 1);
        assert!(f[0].reason.contains("nonexistent_tool"));
    }

    // ---- Check 5 ----
    #[test]
    fn check5_pass_and_fail_cases() {
        let builtins: HashSet<String> = ["run_gh"].iter().map(|s| s.to_string()).collect();
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        map.insert("provider".into(), vec!["the_tool".into()]);
        map.insert("consumer".into(), vec![]);

        // consumer requires the_tool, provided by provider. Both names must be
        // is_bundled_skill — so for this synthetic test we cannot rely on is_bundled_skill.
        // Instead validate the reachability arm with names that won't trip arm (a):
        // we only assert arm (b) here by using the real allowlist test below for arm (a).
        let consumer = view(
            "consumer",
            &["skill.toml"],
            Ok(manifest(
                Some("consumer"),
                Some("0.1.0"),
                false,
                &["x"],
                &["the_tool"],
            )),
            Ok(vec![]),
        );
        // Allowlist includes provider → the_tool reachable.
        let reachable_ok = check5_identity_coherence(
            &[consumer],
            &[("agentX", vec!["consumer".into(), "provider".into()])],
            &builtins,
            &map,
        );
        // arm (b) must NOT fire for the_tool (reachable via provider in the allowlist).
        assert!(
            !reachable_ok
                .iter()
                .any(|f| f.reason.contains("not reachable")),
            "the_tool should be reachable via provider in the allowlist: {reachable_ok:?}"
        );
        // arm (a) MUST fire: "consumer"/"provider" are not real bundled skills.
        assert!(
            reachable_ok
                .iter()
                .any(|f| f.reason.contains("not a bundled skill")),
            "arm (a) should flag non-bundled allowlist names: {reachable_ok:?}"
        );

        // Remove provider from allowlist → the_tool not reachable.
        let consumer2 = view(
            "consumer",
            &["skill.toml"],
            Ok(manifest(
                Some("consumer"),
                Some("0.1.0"),
                false,
                &["x"],
                &["the_tool"],
            )),
            Ok(vec![]),
        );
        let unreachable = check5_identity_coherence(
            &[consumer2],
            &[("agentY", vec!["consumer".into()])],
            &builtins,
            &map,
        );
        assert!(
            unreachable
                .iter()
                .any(|f| f.reason.contains("not reachable"))
        );
    }

    // ---- Check 3 unknown/missing handler.type ----
    #[test]
    fn check3_unknown_and_missing_handler_type() {
        let unknown = view(
            "unknown",
            &["skill.toml", "tools.json"],
            Ok(manifest(Some("unknown"), Some("0.1.0"), false, &["x"], &[])),
            Ok(vec![ToolDecl {
                name: Some("t".into()),
                handler_type: Some("mystery".into()),
                function: None,
                command: None,
            }]),
        );
        let f = check3_handler_resolution(&[unknown], &known_builtins_fixture());
        assert!(
            f.iter()
                .any(|x| x.reason.contains("unrecognized handler.type"))
        );

        let missing = view(
            "missing-type",
            &["skill.toml", "tools.json"],
            Ok(manifest(
                Some("missing-type"),
                Some("0.1.0"),
                false,
                &["x"],
                &[],
            )),
            Ok(vec![ToolDecl {
                name: Some("t".into()),
                handler_type: None,
                function: None,
                command: None,
            }]),
        );
        let f = check3_handler_resolution(&[missing], &known_builtins_fixture());
        assert!(f.iter().any(|x| x.reason.contains("no handler.type")));

        // `http` is deliberately out of scope — must NOT fail.
        let http = view(
            "http",
            &["skill.toml", "tools.json"],
            Ok(manifest(Some("http"), Some("0.1.0"), false, &["x"], &[])),
            Ok(vec![ToolDecl {
                name: Some("t".into()),
                handler_type: Some("http".into()),
                function: None,
                command: None,
            }]),
        );
        assert!(check3_handler_resolution(&[http], &known_builtins_fixture()).is_empty());
    }

    // ---- Check 1 orphan-directory (missing skill.toml) ----
    #[test]
    fn check1_missing_manifest_flags_dir_without_skill_toml() {
        let base = tempfile::tempdir().unwrap();
        // A skill-shaped dir with system_prompt.md but NO skill.toml (incomplete add).
        let incomplete = base.path().join("incomplete-skill");
        std::fs::create_dir_all(&incomplete).unwrap();
        std::fs::write(incomplete.join("system_prompt.md"), "prompt").unwrap();
        // A complete dir with skill.toml → must not be flagged.
        let complete = base.path().join("complete-skill");
        std::fs::create_dir_all(&complete).unwrap();
        std::fs::write(complete.join("skill.toml"), "[skill]\nname=\"x\"\n").unwrap();
        // A support dir (`_shared`) → must be skipped.
        let shared = base.path().join("_shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("lib.sh"), "#!/bin/sh\n").unwrap();

        let f = check1_missing_manifest(base.path());
        assert_eq!(
            f.len(),
            1,
            "exactly the incomplete dir should be flagged: {f:?}"
        );
        assert_eq!(f[0].skill, "incomplete-skill");
        assert!(f[0].reason.contains("skill.toml"));
    }

    // ---- KNOWN_EXCEPTIONS hygiene + full-tree green ----
    #[test]
    fn known_exceptions_ship_empty() {
        // mika#1575 F2: KNOWN_EXCEPTIONS ships empty. If you add an entry, the PR
        // description MUST enumerate it with its resolution ticket.
        assert!(
            KNOWN_EXCEPTIONS.is_empty(),
            "KNOWN_EXCEPTIONS must ship empty (mika#1575 F2)"
        );
    }

    #[test]
    fn real_bundled_tree_passes_green() {
        // mika#1575 F2 + mika-arch #1576 F3 (first-deploy fire-disposition): the
        // detector MUST pass green against the current source tree on first run.
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills/bundled");
        let failures = run_all_checks(&base);
        let reportable: Vec<&Failure> = failures
            .iter()
            .filter(|f| !KNOWN_EXCEPTIONS.iter().any(|e| matches_exception(f, e)))
            .collect();
        assert!(
            reportable.is_empty(),
            "skills/bundled/ has structural failures (mika#1575 must ship green):\n{}",
            reportable
                .iter()
                .map(|f| format!("  CHECK {} {} — {}", f.check, f.skill, f.reason))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
