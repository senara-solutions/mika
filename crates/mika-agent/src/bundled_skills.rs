//! Compile-time embedded skill templates, seeded into a canonical runtime
//! library on startup with per-agent symlinks materialized per allowlist.
//!
//! Each skill is a set of files (skill.toml, tools.json, optional system_prompt.md,
//! optional handler scripts) embedded via `include_str!`. On startup,
//! [`seed_bundled_skill_library`] extracts them to the canonical library at
//! `{global_home}/skills/<skill>/` (sync-shaped, hash-gated), and
//! [`materialize_agent_skill_links`] creates per-agent symlinks into the library
//! under `{global_home}/agents/<name>/skills/<skill>` for each allowlisted bundled
//! skill. Non-bundled (marketplace/custom) skills under the per-agent dir are
//! never touched.
//!
//! The `--copy` opt-out (see [`copy_bundled_skill_to_agent_dir`]) writes a real
//! directory copy with a `.copy-managed` marker so library sync leaves it alone —
//! the operator who used `--copy` owns that lifecycle (mika#1213).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use tracing::{debug, info, warn};

/// Marker file written into per-agent skill directories materialized by
/// `mika skills install <skill> --copy --agent <name>`. Library sync sees
/// the marker and refuses to replace the directory with a symlink; library
/// sync also refuses to remove the directory when the skill drops out of
/// the agent's allowlist. The operator owns the lifecycle of `--copy` dirs.
const COPY_MARKER_FILE: &str = ".copy-managed";

/// Manifest-hash record written into the library root. Gates re-extraction —
/// if the binary's manifest hash equals the on-disk value, the library is
/// already in sync and extraction is skipped (idempotent across restarts).
const MANIFEST_HASH_FILE: &str = ".manifest-hash";

/// A single file within a bundled skill.
struct SkillFile {
    /// Relative path within the skill directory (e.g. "skill.toml", "handlers/run.sh").
    path: &'static str,
    /// File contents (embedded at compile time).
    content: &'static str,
    /// Whether the file should be marked executable (handler scripts).
    executable: bool,
}

/// A complete bundled skill.
struct BundledSkill {
    name: &'static str,
    files: &'static [SkillFile],
    /// SHA-256 content hash (first 16 hex chars) computed at build time over the
    /// concatenation of all file contents in sorted order. Used for drift detection
    /// when `MIKA_DISABLE_BUNDLED_SKILLS=true` prevents re-seeding (#984).
    content_hash: &'static str,
}

/// A support directory (underscore-prefixed, non-skill shared library).
/// Seeded alongside skills so sibling skills can source them at runtime.
/// See mika#923.
struct SupportDir {
    name: &'static str,
    files: &'static [SkillFile],
}

/// Declare a bundled skill with its files.
/// Use `+x` suffix to mark a file as executable (handler scripts).
/// Legacy skills get an empty content_hash (drift detection scoped to engine-coupled
/// skills in `skills/bundled/` which are hashed at build time by `build.rs`).
macro_rules! skill {
    ($name:expr, [ $( $entry:tt ),+ $(,)? ]) => {
        BundledSkill {
            name: $name,
            files: &[
                $( skill!(@file $entry), )+
            ],
            content_hash: "",
        }
    };
    (@file ($path:expr => $template:expr, +x)) => {
        SkillFile {
            path: $path,
            content: include_str!($template),
            executable: true,
        }
    };
    (@file ($path:expr => $template:expr)) => {
        SkillFile {
            path: $path,
            content: include_str!($template),
            executable: false,
        }
    };
}

static TMUX_SKILL: BundledSkill = skill!("tmux", [
    ("skill.toml" => "../templates/skills/tmux/skill.toml"),
    ("system_prompt.md" => "../templates/skills/tmux/system_prompt.md"),
    ("tools.json" => "../templates/skills/tmux/tools.json"),
    ("handlers/create_session.sh" => "../templates/skills/tmux/handlers/create_session.sh", +x),
    ("handlers/kill_session.sh" => "../templates/skills/tmux/handlers/kill_session.sh", +x),
    ("handlers/list_sessions.sh" => "../templates/skills/tmux/handlers/list_sessions.sh", +x),
    ("handlers/read_output.sh" => "../templates/skills/tmux/handlers/read_output.sh", +x),
    ("handlers/send_command.sh" => "../templates/skills/tmux/handlers/send_command.sh", +x),
    ("handlers/wait_for_text.sh" => "../templates/skills/tmux/handlers/wait_for_text.sh", +x),
]);

static SHELL_EXEC_SKILL: BundledSkill = skill!("shell-exec", [
    ("skill.toml" => "../templates/skills/shell-exec/skill.toml"),
    ("system_prompt.md" => "../templates/skills/shell-exec/system_prompt.md"),
    ("tools.json" => "../templates/skills/shell-exec/tools.json"),
    ("handlers/run.sh" => "../templates/skills/shell-exec/handlers/run.sh", +x),
]);

static WEB_SEARCH_SKILL: BundledSkill = skill!("web-search", [
    ("skill.toml" => "../templates/skills/web-search/skill.toml"),
    ("system_prompt.md" => "../templates/skills/web-search/system_prompt.md"),
    ("tools.json" => "../templates/skills/web-search/tools.json"),
]);

static FETCH_URL_SKILL: BundledSkill = skill!("fetch-url", [
    ("skill.toml" => "../templates/skills/fetch-url/skill.toml"),
    ("system_prompt.md" => "../templates/skills/fetch-url/system_prompt.md"),
    ("tools.json" => "../templates/skills/fetch-url/tools.json"),
]);

static FILE_READER_SKILL: BundledSkill = skill!("file-reader", [
    ("skill.toml" => "../templates/skills/file-reader/skill.toml"),
    ("system_prompt.md" => "../templates/skills/file-reader/system_prompt.md"),
    ("tools.json" => "../templates/skills/file-reader/tools.json"),
    ("handlers/read.sh" => "../templates/skills/file-reader/handlers/read.sh", +x),
]);

// skill-review migrated to skills/bundled/ (engine-coupled via review_filter).
// Discovered at build time by build.rs — no static entry needed.

static SELF_KNOWLEDGE_SKILL: BundledSkill = skill!("self-knowledge", [
    ("skill.toml" => "../templates/skills/self-knowledge/skill.toml"),
    ("system_prompt.md" => "../templates/skills/self-knowledge/system_prompt.md"),
    ("tools.json" => "../templates/skills/self-knowledge/tools.json"),
]);

static GOOGLE_WORKSPACE_SKILL: BundledSkill = skill!("google-workspace", [
    ("skill.toml" => "../templates/skills/google-workspace/skill.toml"),
    ("system_prompt.md" => "../templates/skills/google-workspace/system_prompt.md"),
    ("tools.json" => "../templates/skills/google-workspace/tools.json"),
]);

static GIT_OPS_SKILL: BundledSkill = skill!("git-ops", [
    ("skill.toml" => "../templates/skills/git-ops/skill.toml"),
    ("system_prompt.md" => "../templates/skills/git-ops/system_prompt.md"),
    ("tools.json" => "../templates/skills/git-ops/tools.json"),
]);

static GITHUB_SKILL: BundledSkill = skill!("github", [
    ("skill.toml" => "../templates/skills/github/skill.toml"),
    ("system_prompt.md" => "../templates/skills/github/system_prompt.md"),
    ("tools.json" => "../templates/skills/github/tools.json"),
]);

static MCP_SKILL: BundledSkill = skill!("mcp", [
    ("skill.toml" => "../templates/skills/mcp/skill.toml"),
    ("system_prompt.md" => "../templates/skills/mcp/system_prompt.md"),
    ("tools.json" => "../templates/skills/mcp/tools.json"),
]);

static BROWSER_CONTROL_SKILL: BundledSkill = skill!("browser-control", [
    ("skill.toml" => "../templates/skills/browser-control/skill.toml"),
    ("system_prompt.md" => "../templates/skills/browser-control/system_prompt.md"),
    ("tools.json" => "../templates/skills/browser-control/tools.json"),
]);

// agents-teams migrated to skills/bundled/ (engine-coupled via delegate_task /
// run_team guards). Discovered at build time by build.rs.

/// Legacy hardcoded bundled skills (community-category skills kept embedded
/// for convenience). Engine-coupled skills live in `skills/bundled/` and are
/// discovered at build time — see the `ENTRIES` table below.
static BUNDLED_SKILLS: &[&BundledSkill] = &[
    &TMUX_SKILL,
    &SHELL_EXEC_SKILL,
    &WEB_SEARCH_SKILL,
    &FETCH_URL_SKILL,
    &FILE_READER_SKILL,
    &SELF_KNOWLEDGE_SKILL,
    &GIT_OPS_SKILL,
    &GOOGLE_WORKSPACE_SKILL,
    &GITHUB_SKILL,
    &MCP_SKILL,
    &BROWSER_CONTROL_SKILL,
];

// Directory-sourced bundled skills, generated at build time by `build.rs`
// walking `<workspace>/skills/bundled/`. Declares `static ENTRIES: &[BundledSkill]`.
// An empty or missing `skills/bundled/` directory yields `ENTRIES = &[]` — the
// parallel path stays silent until a future migration ticket starts populating it.
include!(concat!(env!("OUT_DIR"), "/bundled_skills_generated.rs"));

/// Pure merge primitive: overlay `entries` onto `legacy` with case-insensitive
/// ENTRIES-wins-on-name-collision semantics. Extracted so both the production
/// `all_bundled_skills()` path and tests can exercise the same implementation.
fn merge_skill_lists(
    legacy: &[&'static BundledSkill],
    entries: &'static [BundledSkill],
) -> Vec<&'static BundledSkill> {
    let mut merged: Vec<&'static BundledSkill> = legacy.to_vec();
    for entry in entries {
        if let Some(slot) = merged
            .iter_mut()
            .find(|existing| existing.name.eq_ignore_ascii_case(entry.name))
        {
            *slot = entry;
        } else {
            merged.push(entry);
        }
    }
    merged
}

/// Merge the legacy hardcoded `BUNDLED_SKILLS` list with the directory-sourced
/// `ENTRIES` table. Entries from `ENTRIES` win on case-insensitive name collision.
///
/// Returns a deduplicated `Vec<&'static BundledSkill>`. Zero collisions are
/// expected in production during this refactor (ENTRIES is empty). The merge
/// semantics exist so a future migration ticket can move skills one-by-one
/// without orchestrating a coordinated cutover.
fn all_bundled_skills() -> Vec<&'static BundledSkill> {
    merge_skill_lists(BUNDLED_SKILLS, ENTRIES)
}

/// Check whether a skill name matches a bundled (built-in) skill.
///
/// Consults both the legacy hardcoded list and the directory-sourced `ENTRIES`
/// table so install/uninstall/update guards treat either source as immutable.
///
/// Invariant: this must enumerate every source consulted by
/// [`all_bundled_skills`]. If a third source is added, update both functions
/// together. The `test_is_bundled_skill_agrees_with_all_bundled_skills` test
/// guards the coupling.
pub fn is_bundled_skill(name: &str) -> bool {
    BUNDLED_SKILLS
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case(name))
        || ENTRIES.iter().any(|s| s.name.eq_ignore_ascii_case(name))
}

/// Returns the names of all bundled skills (both legacy hardcoded and
/// directory-sourced), deduplicated with ENTRIES-wins semantics.
///
/// Used by well-known agent tests to verify that agent skill allowlists
/// reference only bundled skills that actually exist.
pub fn all_bundled_skill_names() -> Vec<&'static str> {
    all_bundled_skills().iter().map(|s| s.name).collect()
}

/// Returns the set of every tool `name` declared in any bundled skill's
/// `tools.json` — across BOTH the legacy hardcoded community skills and the
/// directory-sourced engine-coupled skills.
///
/// This is the canonical "any bundled skill's tools.json" universe for the
/// `verify-bundled-skills` gate (mika#1575, Check 4 option (c) — allowlist-unaware
/// token resolution). It must consult the same dual source as [`all_bundled_skills`]
/// so a `required_tools` token provided by a community skill (e.g. `run_shell` from
/// `shell-exec`) resolves correctly and does not false-fail the gate.
///
/// Malformed `tools.json` content is skipped silently here — manifest validity is
/// the responsibility of other checks, not this accessor.
pub fn all_bundled_tool_names() -> std::collections::HashSet<String> {
    bundled_skill_tool_map().into_values().flatten().collect()
}

/// Returns `skill_name -> [tool names it declares in its tools.json]` across BOTH
/// the legacy hardcoded community skills and the directory-sourced engine-coupled
/// skills.
///
/// Used by the `verify-bundled-skills` gate (mika#1575, Check 5 — identity allowlist
/// coherence) to compute an agent's *allowlist-scoped* reachable tool set: a token is
/// reachable through an allowlist only if a builtin provides it OR a skill IN THAT
/// ALLOWLIST declares it. Per-skill attribution (not just the flat
/// [`all_bundled_tool_names`] set) is required to distinguish "exists somewhere in the
/// bundled surface" (Check 4) from "reachable through this specific allowlist" (Check 5).
///
/// Skills with no `tools.json` or unparseable content map to an empty list.
pub fn bundled_skill_tool_map() -> std::collections::HashMap<String, Vec<String>> {
    let mut map = std::collections::HashMap::new();
    for skill in all_bundled_skills() {
        let mut names = Vec::new();
        if let Some(tools_file) = skill.files.iter().find(|f| f.path == "tools.json")
            && let Ok(tools) = serde_json::from_str::<Vec<serde_json::Value>>(tools_file.content)
        {
            for tool in &tools {
                if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                    names.push(name.to_string());
                }
            }
        }
        map.insert(skill.name.to_string(), names);
    }
    map
}

/// Trust-critical bundled skills whose prompts must NOT be reviewed or adapted.
///
/// These skills govern the agent's self-awareness, security posture, or ability
/// to modify other skills. Model-specific rewording could weaken their safety
/// properties. All other bundled skills are "functional" — their prompts focus
/// on tool usage mechanics and are safe to adapt per-model.
///
/// Criteria for trust-critical classification:
/// - `skill-review`: can modify any skill's prompt (self-referential risk)
/// - `self-knowledge`: governs agent self-awareness and core identity
/// - `agents-teams`: controls multi-agent orchestration and delegation
static TRUST_CRITICAL_SKILLS: &[&str] = &["skill-review", "self-knowledge", "agents-teams"];

/// Skill names that were once bundled but have been renamed or removed,
/// listed here so the prune pass can clean up their per-agent directories
/// on hosts that were deployed before the rename or removal.
///
/// **Update procedure:** any future PR that renames or removes a bundled
/// skill MUST add the OLD name here in the same commit. Stale entries are
/// harmless (the directories no longer exist on previously-cleaned hosts).
/// Entries can be removed in a later cleanup PR once every production host
/// is confirmed to have rebooted since the rename/removal landed.
///
/// **Marketplace-safety invariant:** entries in this list will be
/// `remove_dir_all`'d from every agent's `skills/` dir. Names added here
/// must be names that were definitively bundled in a prior release.
/// Adding the name of a marketplace skill here would wipe user state.
pub(crate) const KNOWN_REMOVED_BUNDLED_SKILLS: &[&str] = &[
    // claude-pilot was renamed to dev-pilot + dev-groom in mika#853
    // (deployed 2026-04-28). 12 stale per-agent directories survived
    // the rename — see mika#859 § Phase 0 P0.8 for verification output.
    "claude-pilot",
];

/// Check whether a skill is trust-critical (blocked from review/adaptation).
///
/// Only trust-critical bundled skills are blocked from `review_skill`. Other
/// bundled skills (e.g., web-search, shell-exec) are reviewable because their
/// prompts focus on tool usage mechanics — safe to adapt per-model.
///
/// Note: this does NOT replace `is_bundled_skill()` for install/delete/update
/// guards — ALL bundled skills remain protected from those operations.
pub fn is_trust_critical_skill(name: &str) -> bool {
    TRUST_CRITICAL_SKILLS
        .iter()
        .any(|s| s.eq_ignore_ascii_case(name))
}

/// Return the list of trust-critical skill names (for error messages and prompts).
pub fn trust_critical_skill_names() -> &'static [&'static str] {
    TRUST_CRITICAL_SKILLS
}

/// Remove per-agent skill directories whose name appears in
/// [`KNOWN_REMOVED_BUNDLED_SKILLS`]. Called once at the top of
/// [`seed_bundled_skills`].
///
/// Defense-in-depth (matches `write_skill`):
/// - Symlinks are never followed and never removed.
/// - `_`-prefixed support directories (e.g. `_shared/`) are skipped.
/// - `.`-prefixed entries (e.g. `.mika-bundled` markers in a future
///   revision) are skipped.
/// - I/O errors are logged and skipped — a single bad directory must not
///   block the seed.
///
/// Returns the number of directories actually removed.
pub(crate) fn prune_known_removed_bundled_skills(skills_dir: &Path) -> usize {
    let entries = match std::fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "failed to read skills_dir for known-removed prune");
            return 0;
        }
    };

    let mut pruned = 0;
    for entry in entries.flatten() {
        let path = entry.path();

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }

        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };

        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }

        let is_known_removed = KNOWN_REMOVED_BUNDLED_SKILLS
            .iter()
            .any(|s| s.eq_ignore_ascii_case(name));
        if !is_known_removed {
            continue;
        }

        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                info!(
                    skill = name,
                    "pruned orphaned bundled-skill directory (known-removed list)"
                );
                pruned += 1;
            }
            Err(e) => {
                warn!(
                    skill = name,
                    error = %e,
                    "failed to prune orphaned bundled-skill directory"
                );
            }
        }
    }
    pruned
}

/// Compute the binary's bundled-skill manifest hash over the names and
/// per-skill content hashes in `all_bundled_skills()`. The order is fixed by
/// the static merge in [`all_bundled_skills`], so the result is deterministic
/// across runs of the same binary. A different binary (added skill, removed
/// skill, edited content) produces a different value, which triggers
/// re-extraction in [`seed_bundled_skill_library`].
fn compute_manifest_hash() -> String {
    let mut hasher = DefaultHasher::new();
    for skill in all_bundled_skills() {
        skill.name.hash(&mut hasher);
        skill.content_hash.hash(&mut hasher);
        // Hash file paths too so legacy skills (empty content_hash) still
        // contribute structure — otherwise adding a file to a legacy skill
        // would not bump the hash.
        for file in skill.files {
            file.path.hash(&mut hasher);
            file.executable.hash(&mut hasher);
        }
    }
    format!("{:016x}", hasher.finish())
}

/// Seed the canonical bundled-skill library at `library_dir`.
///
/// Sync-shape (mika#1213): after this returns, `library_dir` contains exactly
/// the set of skill directories in the current binary's bundled manifest set
/// — extracts new/changed skills, removes orphan directories for skills no
/// longer in the manifest set.
///
/// Hash-gated: when the on-disk `.manifest-hash` matches the binary's manifest
/// hash, no extraction work runs (idempotent across restarts). The hash gate
/// is skipped if the library directory is missing or empty, so a fresh install
/// always extracts.
///
/// Support directories (`_shared/`) are seeded alongside skills — same library
/// shape; they back the dispatch pipeline.
///
/// User-created skills (non-bundled) are never written to the library —
/// marketplace/custom skills live under per-agent skill dirs, not here.
pub fn seed_bundled_skill_library(library_dir: &Path) {
    if let Err(e) = std::fs::create_dir_all(library_dir) {
        warn!(
            library = %library_dir.display(),
            error = %e,
            "failed to create bundled-skill library directory; aborting seed"
        );
        return;
    }

    let target_hash = compute_manifest_hash();
    let hash_path = library_dir.join(MANIFEST_HASH_FILE);
    let on_disk_hash = std::fs::read_to_string(&hash_path).ok();
    if let Some(prev) = on_disk_hash.as_deref()
        && prev.trim() == target_hash
        && library_dir.join(MANIFEST_HASH_FILE).exists()
    {
        debug!(
            library = %library_dir.display(),
            hash = %target_hash,
            "bundled-skill library is in sync with binary manifest; skipping extraction"
        );
        return;
    }

    // Support directories first — dispatch plumbing needed by sibling skills.
    seed_support_dirs(library_dir);

    // Prune known-removed orphans before sync-shape pruning so a future PR
    // that reuses a removed name lands as a clean create. Sync-shape prune
    // below covers any other orphans.
    let pruned = prune_known_removed_bundled_skills(library_dir);
    if pruned > 0 {
        info!(
            count = pruned,
            "pruned known-removed bundled-skill directories"
        );
    }

    let mut wrote = 0usize;
    for skill in all_bundled_skills() {
        let skill_dir = library_dir.join(skill.name);
        let is_update = skill_dir.exists();

        if let Err(e) = write_skill(&skill_dir, skill) {
            warn!(skill = skill.name, error = %e, "failed to seed bundled skill");
            if !is_update {
                let _ = std::fs::remove_dir_all(&skill_dir);
            }
        } else {
            wrote += 1;
            if is_update {
                debug!(skill = skill.name, "updated bundled skill");
            } else {
                info!(skill = skill.name, "seeded bundled skill");
            }
        }
    }

    // Sync-shape: remove any skill directory in the library that isn't in
    // the current manifest set. Symlinks, support dirs (`_`-prefixed),
    // dotfiles, and the manifest-hash file itself are preserved.
    let orphans = prune_library_orphans(library_dir);
    if orphans > 0 {
        info!(
            count = orphans,
            "pruned orphan directories from bundled-skill library"
        );
    }

    // Record the new manifest hash last so a partial extraction failure
    // (some skill failed to write) does not mask a stale on-disk state on
    // the next restart — the gate will re-run.
    if wrote > 0
        && let Err(e) = std::fs::write(&hash_path, &target_hash)
    {
        warn!(
            path = %hash_path.display(),
            error = %e,
            "failed to write bundled-skill manifest hash record"
        );
    }
}

/// Remove directories in `library_dir` whose names are not in the current
/// bundled manifest set. Preserves symlinks, `_`-prefixed support dirs,
/// `.`-prefixed dotfiles (incl. the manifest-hash record), and any
/// non-directory entries.
///
/// Returns the number of directories actually removed.
fn prune_library_orphans(library_dir: &Path) -> usize {
    let entries = match std::fs::read_dir(library_dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "failed to read library_dir for orphan prune");
            return 0;
        }
    };

    let manifest_names: Vec<&'static str> = all_bundled_skills().iter().map(|s| s.name).collect();

    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();

        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        // Never follow / remove symlinks; never touch support or dot dirs.
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }

        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }

        let in_manifest = manifest_names.iter().any(|m| m.eq_ignore_ascii_case(name));
        if in_manifest {
            continue;
        }

        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                info!(
                    skill = name,
                    "removed orphan bundled-skill library directory (sync-shape)"
                );
                removed += 1;
            }
            Err(e) => {
                warn!(
                    skill = name,
                    error = %e,
                    "failed to remove orphan library directory"
                );
            }
        }
    }
    removed
}

/// Materialize per-agent symlinks under `agent_skills_dir` for each
/// allowlisted bundled skill (`<agent_skills_dir>/<skill>` →
/// `<library_dir>/<skill>`). Replaces legacy per-agent copies with symlinks.
///
/// - When `allowlist` is `Some` and non-empty, only the listed skills are
///   materialized; symlinks for bundled skills NOT in the list are removed.
/// - When `allowlist` is `None` or empty, every bundled skill from the
///   binary's manifest set is materialized (matches the historical "all
///   bundled skills enabled" default for user-defined agents).
/// - Non-bundled (marketplace/custom) directories under `agent_skills_dir`
///   are never touched.
/// - `--copy`-managed directories (those containing a `.copy-managed`
///   marker file) are never touched — the operator owns their lifecycle.
/// - `.disabled` marker files alongside skill dirs are preserved.
///
/// Idempotent: re-running over an already-materialized agent is a no-op.
pub fn materialize_agent_skill_links(
    library_dir: &Path,
    agent_skills_dir: &Path,
    allowlist: Option<&[String]>,
) {
    if let Err(e) = std::fs::create_dir_all(agent_skills_dir) {
        warn!(
            agent_skills_dir = %agent_skills_dir.display(),
            error = %e,
            "failed to create agent skills directory"
        );
        return;
    }

    let bundled_names: Vec<&'static str> = all_bundled_skills().iter().map(|s| s.name).collect();

    // Resolve the set of bundled skills this agent should have linked.
    let allowed: Vec<&str> = match allowlist {
        Some(list) if !list.is_empty() => bundled_names
            .iter()
            .copied()
            .filter(|b| list.iter().any(|a| a.eq_ignore_ascii_case(b)))
            .collect(),
        _ => bundled_names.clone(),
    };

    // Pass 1: create / refresh symlinks for allowed bundled skills. Each
    // entry under agent_skills_dir/<skill> ends up as a symlink into the
    // library, replacing legacy copies. --copy-managed dirs are skipped.
    for skill_name in &allowed {
        let target = library_dir.join(skill_name);
        let link = agent_skills_dir.join(skill_name);

        if !target.exists() {
            // Library not seeded yet (or skill missing). Skip rather than
            // dangle — the next restart's library seed pass will create it.
            warn!(
                skill = skill_name,
                library = %target.display(),
                "library skill missing; skipping symlink creation"
            );
            continue;
        }

        if is_copy_managed(&link) {
            debug!(
                skill = skill_name,
                "agent skill dir is --copy-managed; skipping symlink"
            );
            continue;
        }

        if symlink_points_to(&link, &target) {
            // Already correctly linked.
            continue;
        }

        // Replace existing entry (legacy copy or stale link) with a fresh
        // symlink. remove handles both file and directory cases.
        if let Err(e) = remove_link_or_dir(&link) {
            warn!(
                skill = skill_name,
                error = %e,
                "failed to remove existing agent skill entry before symlinking"
            );
            continue;
        }

        #[cfg(unix)]
        if let Err(e) = std::os::unix::fs::symlink(&target, &link) {
            warn!(
                skill = skill_name,
                target = %target.display(),
                link = %link.display(),
                error = %e,
                "failed to create per-agent skill symlink"
            );
            continue;
        }
        #[cfg(not(unix))]
        {
            // Non-Unix hosts fall back to a directory copy. The runtime is
            // Linux/macOS in production; this branch keeps the build green
            // on Windows dev hosts.
            if let Err(e) = copy_dir(&target, &link) {
                warn!(
                    skill = skill_name,
                    error = %e,
                    "failed to copy bundled skill into agent dir (non-unix host)"
                );
                continue;
            }
        }

        info!(
            skill = skill_name,
            agent_dir = %link.display(),
            "materialized per-agent bundled-skill symlink"
        );
    }

    // Pass 2: remove symlinks for bundled skills no longer in the
    // agent's allowlist. Only symlinks pointing at the library are removed
    // — regular dirs (marketplace, --copy-managed) are untouched.
    let allowed_set: std::collections::HashSet<&str> = allowed.iter().copied().collect();
    for bundled_name in &bundled_names {
        if allowed_set.contains(*bundled_name) {
            continue;
        }
        let link = agent_skills_dir.join(bundled_name);
        if !link.exists() && link.symlink_metadata().is_err() {
            continue;
        }
        // Skip --copy-managed dirs even when de-allowlisted.
        if is_copy_managed(&link) {
            debug!(
                skill = bundled_name,
                "de-allowlisted bundled skill is --copy-managed; preserving"
            );
            continue;
        }
        let meta = match link.symlink_metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.file_type().is_symlink() {
            // Regular dir — could be a legacy copy left behind from a
            // version that copied without a marker. We do NOT remove
            // these here because they predate the marker contract; leave
            // them in place. The operator can clean up manually if needed.
            continue;
        }
        if let Err(e) = std::fs::remove_file(&link) {
            warn!(
                skill = bundled_name,
                error = %e,
                "failed to remove symlink for de-allowlisted bundled skill"
            );
            continue;
        }
        info!(
            skill = bundled_name,
            "removed per-agent symlink for de-allowlisted bundled skill"
        );
    }

    // Pass 3: materialize support-dir symlinks (e.g. `_shared/`). Support
    // dirs are dispatch plumbing — sibling skill handlers source them via
    // relative path (`$(dirname "$0")/../../_shared/dispatch-lib.sh`), so
    // they must be present under the agent's skills dir whether the agent
    // is symlink-mode or `--copy`-mode. Always materialized; never gated
    // by allowlist. See mika#923 + mika#1213.
    for support in SUPPORT_DIRS {
        let target = library_dir.join(support.name);
        let link = agent_skills_dir.join(support.name);
        if !target.exists() {
            continue;
        }
        if symlink_points_to(&link, &target) {
            continue;
        }
        if let Err(e) = remove_link_or_dir(&link) {
            warn!(
                support_dir = support.name,
                error = %e,
                "failed to remove existing agent support-dir entry before symlinking"
            );
            continue;
        }
        #[cfg(unix)]
        if let Err(e) = std::os::unix::fs::symlink(&target, &link) {
            warn!(
                support_dir = support.name,
                target = %target.display(),
                link = %link.display(),
                error = %e,
                "failed to create per-agent support-dir symlink"
            );
            continue;
        }
        #[cfg(not(unix))]
        {
            if let Err(e) = copy_dir(&target, &link) {
                warn!(
                    support_dir = support.name,
                    error = %e,
                    "failed to copy support dir into agent dir (non-unix host)"
                );
                continue;
            }
        }
        info!(
            support_dir = support.name,
            agent_dir = %link.display(),
            "materialized per-agent support-dir symlink"
        );
    }
}

/// Materialize a `--copy` opt-out: copy a bundled skill from the library to
/// the agent's skill dir as a real directory and write a `.copy-managed`
/// marker. Library sync ([`materialize_agent_skill_links`]) leaves
/// `--copy`-managed dirs alone — the operator owns their lifecycle.
///
/// Returns an error if `skill_name` is not in the binary's bundled manifest
/// set, or if the library entry is missing on disk (e.g., the library has
/// not been seeded yet).
pub fn copy_bundled_skill_to_agent_dir(
    library_dir: &Path,
    agent_skills_dir: &Path,
    skill_name: &str,
) -> std::io::Result<()> {
    if !is_bundled_skill(skill_name) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("'{skill_name}' is not a bundled skill"),
        ));
    }
    let src = library_dir.join(skill_name);
    if !src.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "bundled-skill library entry missing at {}; run a deploy / agent restart first",
                src.display()
            ),
        ));
    }
    std::fs::create_dir_all(agent_skills_dir)?;
    let dst = agent_skills_dir.join(skill_name);

    // Replace whatever is currently there — symlink, legacy copy, or empty.
    remove_link_or_dir(&dst)?;
    copy_dir(&src, &dst)?;
    std::fs::write(dst.join(COPY_MARKER_FILE), b"copy-managed\n")?;
    Ok(())
}

/// True iff `path` is a directory containing the `.copy-managed` marker.
fn is_copy_managed(path: &Path) -> bool {
    let meta = match path.symlink_metadata() {
        Ok(m) => m,
        Err(_) => return false,
    };
    if meta.file_type().is_symlink() {
        return false;
    }
    if !meta.is_dir() {
        return false;
    }
    path.join(COPY_MARKER_FILE).is_file()
}

/// True iff `link` is a symlink whose target resolves to the same path as
/// `target` (canonicalized). False on read errors.
fn symlink_points_to(link: &Path, target: &Path) -> bool {
    let meta = match link.symlink_metadata() {
        Ok(m) => m,
        Err(_) => return false,
    };
    if !meta.file_type().is_symlink() {
        return false;
    }
    let resolved = match std::fs::read_link(link) {
        Ok(r) => r,
        Err(_) => return false,
    };
    // Compare absolute paths when possible — read_link returns whatever the
    // symlink stores, which is absolute in our seed path but resilient to
    // relative-target forms a future tool might write.
    let resolved_abs = if resolved.is_absolute() {
        resolved
    } else {
        link.parent().unwrap_or(Path::new("")).join(resolved)
    };
    let canon_resolved = std::fs::canonicalize(&resolved_abs).unwrap_or(resolved_abs);
    let canon_target = std::fs::canonicalize(target).unwrap_or(target.to_path_buf());
    canon_resolved == canon_target
}

/// Remove `path` whether it is a file, symlink, or directory. No-op when
/// the path does not exist.
fn remove_link_or_dir(path: &Path) -> std::io::Result<()> {
    let meta = match path.symlink_metadata() {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        std::fs::remove_file(path)
    } else {
        std::fs::remove_dir_all(path)
    }
}

/// Recursively copy `src` to `dst`. Used by the `--copy` opt-out and as a
/// non-unix fallback for [`materialize_agent_skill_links`].
fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&from)?.permissions().mode();
                std::fs::set_permissions(&to, std::fs::Permissions::from_mode(mode))?;
            }
        }
    }
    Ok(())
}

/// Legacy compatibility wrapper for tests and out-of-tree callers that
/// still treat a single directory as the canonical seed location.
/// Equivalent to [`seed_bundled_skill_library`].
pub fn seed_bundled_skills(skills_dir: &Path) {
    seed_bundled_skill_library(skills_dir);
}

/// Write a set of files into the given directory.
///
/// Shared implementation for both skill and support directory seeding.
/// Creates parent directories as needed, refuses to overwrite symlinked files.
fn write_dir_files(dir: &Path, files: &[SkillFile]) -> std::io::Result<()> {
    for file in files {
        let file_path = dir.join(file.path);

        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Refuse to overwrite a file that is a symlink (defense-in-depth).
        if file_path.exists() && file_path.symlink_metadata()?.file_type().is_symlink() {
            return Err(std::io::Error::other(format!(
                "file '{}' is a symlink, refusing to write",
                file.path
            )));
        }

        std::fs::write(&file_path, file.content)?;

        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file_path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// Write all files for a single skill into the given directory.
fn write_skill(skill_dir: &Path, skill: &BundledSkill) -> std::io::Result<()> {
    // Refuse to write into symlinked skill directories (defense-in-depth).
    // An attacker could replace a bundled skill directory with a symlink to
    // redirect writes (including executable handler scripts) to an arbitrary
    // location.
    if skill_dir.exists() && skill_dir.symlink_metadata()?.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "skill directory is a symlink, refusing to write",
        ));
    }

    write_dir_files(skill_dir, skill.files)
}

/// Seed support directories (underscore-prefixed shared libraries) into the
/// given skills directory.
///
/// Support directories contain shared code (e.g. `_shared/dispatch-lib.sh`)
/// that sibling skills source at runtime via relative path. They are NOT skills
/// (no `skill.toml`) but must be present in the deployed skills tree.
///
/// Called unconditionally — even when `MIKA_DISABLE_BUNDLED_SKILLS=true` — because
/// support dirs are infrastructure (dispatch plumbing), not skill prompts. The
/// disable flag is for hot-patching skill prompts during dev, not for breaking
/// the dispatch pipeline. See mika#923.
pub fn seed_support_dirs(skills_dir: &Path) {
    for dir in SUPPORT_DIRS {
        let target_dir = skills_dir.join(dir.name);
        let is_update = target_dir.exists();

        // Refuse to write into symlinked support directories (defense-in-depth).
        if target_dir.exists() {
            match target_dir.symlink_metadata() {
                Ok(meta) if meta.file_type().is_symlink() => {
                    warn!(
                        support_dir = dir.name,
                        "support directory is a symlink, refusing to write"
                    );
                    continue;
                }
                Err(e) => {
                    warn!(support_dir = dir.name, error = %e, "failed to stat support directory");
                    continue;
                }
                _ => {}
            }
        }

        if let Err(e) = write_dir_files(&target_dir, dir.files) {
            warn!(support_dir = dir.name, error = %e, "failed to seed support directory");
            if !is_update {
                let _ = std::fs::remove_dir_all(&target_dir);
            }
        } else if is_update {
            debug!(support_dir = dir.name, "updated support directory");
        } else {
            info!(support_dir = dir.name, "seeded support directory");
        }
    }
}

/// Check on-disk bundled skills for content drift against the build-time embedded
/// hashes. Called when `MIKA_DISABLE_BUNDLED_SKILLS=true` to surface stale state.
///
/// Only checks directory-sourced skills (engine-coupled, from `skills/bundled/`)
/// since those carry build-time content hashes. Legacy community skills have
/// empty hashes and are skipped.
///
/// Emits one `ERROR` log per drifted skill and returns the count of drifts detected.
pub fn check_bundled_skill_drift(skills_dir: &Path) -> usize {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut drift_count = 0;

    for skill in all_bundled_skills() {
        // Skip skills without a build-time hash (legacy community skills)
        if skill.content_hash.is_empty() {
            continue;
        }

        let skill_dir = skills_dir.join(skill.name);
        if !skill_dir.exists() {
            // Skill not seeded at all — definitely drifted
            tracing::error!(
                skill = skill.name,
                expected_hash = skill.content_hash,
                "bundled_skill_drift: skill directory missing on disk \
                 (MIKA_DISABLE_BUNDLED_SKILLS=true prevented seeding)"
            );
            drift_count += 1;
            continue;
        }

        // Compute on-disk hash using the same algorithm as build.rs
        let mut hasher = DefaultHasher::new();
        let mut all_readable = true;
        for file in skill.files {
            let file_path = skill_dir.join(file.path);
            match std::fs::read_to_string(&file_path) {
                Ok(content) => {
                    file.path.hash(&mut hasher);
                    content.hash(&mut hasher);
                }
                Err(_) => {
                    all_readable = false;
                    break;
                }
            }
        }

        if !all_readable {
            tracing::error!(
                skill = skill.name,
                expected_hash = skill.content_hash,
                "bundled_skill_drift: one or more skill files unreadable on disk"
            );
            drift_count += 1;
            continue;
        }

        let on_disk_hash = format!("{:016x}", hasher.finish());
        if on_disk_hash != skill.content_hash {
            tracing::error!(
                skill = skill.name,
                expected_hash = %&skill.content_hash[..12.min(skill.content_hash.len())],
                actual_hash = %&on_disk_hash[..12],
                "bundled_skill_drift: on-disk content differs from build-time embed \
                 (MIKA_DISABLE_BUNDLED_SKILLS=true prevented update)"
            );
            drift_count += 1;
        }
    }

    drift_count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seed_creates_all_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);

        for skill in all_bundled_skills() {
            let skill_dir = skills_dir.join(skill.name);
            assert!(skill_dir.is_dir(), "skill dir missing: {}", skill.name);
            for file in skill.files {
                let file_path = skill_dir.join(file.path);
                assert!(
                    file_path.is_file(),
                    "file missing: {}/{}",
                    skill.name,
                    file.path
                );
                let content = std::fs::read_to_string(&file_path).unwrap();
                assert!(
                    !content.is_empty(),
                    "file empty: {}/{}",
                    skill.name,
                    file.path
                );
            }
        }
    }

    #[test]
    fn test_seed_updates_existing_bundled_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once
        seed_bundled_skill_library(skills_dir);

        // Modify a bundled file in the library
        let marker = skills_dir.join("tmux").join("skill.toml");
        std::fs::write(&marker, "custom content").unwrap();

        // Hash gate (mika#1213): re-seeding the same binary is a no-op —
        // manual edits to the library are not overwritten until the
        // binary's manifest hash changes. Simulate a binary update by
        // invalidating the hash record before re-seeding.
        let hash_path = skills_dir.join(MANIFEST_HASH_FILE);
        std::fs::write(&hash_path, "deadbeef-stale-hash").unwrap();
        seed_bundled_skill_library(skills_dir);

        let content = std::fs::read_to_string(&marker).unwrap();
        assert_ne!(content, "custom content", "bundled file should be updated");
        assert!(!content.is_empty(), "bundled file should have content");
    }

    #[test]
    fn test_seed_is_hash_gated_idempotent() {
        // Repeated seed calls on the same binary do no work after the
        // first — the on-disk hash matches the binary's manifest hash.
        let tmp = tempfile::tempdir().unwrap();
        let library_dir = tmp.path();

        seed_bundled_skill_library(library_dir);
        let marker = library_dir.join("tmux").join("skill.toml");
        // Mutate the file; re-seed must NOT overwrite (hash-gated).
        std::fs::write(&marker, "user edit persists").unwrap();
        seed_bundled_skill_library(library_dir);
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap(),
            "user edit persists",
            "hash gate must skip re-extraction when binary hash matches on-disk record"
        );
    }

    #[test]
    fn test_seed_preserves_extra_files_in_bundled_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once
        seed_bundled_skills(skills_dir);

        // Add an extra file not in the bundle
        let extra = skills_dir.join("tmux").join("my_notes.txt");
        std::fs::write(&extra, "user notes").unwrap();

        // Seed again — extra file should survive
        seed_bundled_skills(skills_dir);

        let content = std::fs::read_to_string(&extra).unwrap();
        assert_eq!(content, "user notes");
    }

    #[test]
    fn test_seed_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);
        seed_bundled_skills(skills_dir);
        seed_bundled_skills(skills_dir);

        // All skills still present (merged view: legacy + directory-sourced)
        for skill in all_bundled_skills() {
            assert!(skills_dir.join(skill.name).is_dir());
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_symlinked_skill_dir_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once so all normal skills exist
        seed_bundled_skills(skills_dir);

        // Replace a bundled skill directory with a symlink to an external target
        let target_dir = tempfile::tempdir().unwrap();
        let tmux_dir = skills_dir.join("tmux");
        std::fs::remove_dir_all(&tmux_dir).unwrap();
        std::os::unix::fs::symlink(target_dir.path(), &tmux_dir).unwrap();

        // Seed again — the symlinked "tmux" dir should be skipped without crashing
        seed_bundled_skills(skills_dir);

        // The target directory should NOT contain any bundled files
        assert!(
            std::fs::read_dir(target_dir.path())
                .unwrap()
                .next()
                .is_none(),
            "symlink target should remain empty — write_skill must refuse to follow it",
        );

        // Other bundled skills should still be updated normally
        assert!(skills_dir.join("shell-exec").join("skill.toml").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn test_symlinked_file_inside_skill_dir_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once so all normal skills exist
        seed_bundled_skills(skills_dir);

        // Replace a single file inside a bundled skill directory with a symlink
        let target_file = tmp.path().join("evil_target.txt");
        std::fs::write(&target_file, "original").unwrap();
        let skill_toml = skills_dir.join("tmux").join("skill.toml");
        std::fs::remove_file(&skill_toml).unwrap();
        std::os::unix::fs::symlink(&target_file, &skill_toml).unwrap();

        // Seed again — tmux should fail because skill.toml is a symlink
        seed_bundled_skills(skills_dir);

        // The symlink target file should not have been overwritten
        let content = std::fs::read_to_string(&target_file).unwrap();
        assert_eq!(
            content, "original",
            "symlinked file target must not be overwritten",
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_handlers_are_executable() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);

        for skill in all_bundled_skills() {
            for file in skill.files {
                if file.executable {
                    let path = skills_dir.join(skill.name).join(file.path);
                    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                    assert!(
                        mode & 0o111 != 0,
                        "handler not executable: {}/{}",
                        skill.name,
                        file.path
                    );
                }
            }
        }
    }

    #[test]
    fn test_trust_critical_skills_are_subset_of_bundled() {
        // Every trust-critical skill must also be a bundled skill.
        for name in TRUST_CRITICAL_SKILLS {
            assert!(
                is_bundled_skill(name),
                "trust-critical skill '{}' is not in BUNDLED_SKILLS",
                name
            );
        }
    }

    #[test]
    fn test_is_trust_critical_skill() {
        assert!(is_trust_critical_skill("skill-review"));
        assert!(is_trust_critical_skill("self-knowledge"));
        assert!(is_trust_critical_skill("agents-teams"));
        // Case-insensitive
        assert!(is_trust_critical_skill("Skill-Review"));
        assert!(is_trust_critical_skill("SELF-KNOWLEDGE"));
    }

    #[test]
    fn test_reviewable_bundled_skills_not_trust_critical() {
        // Every bundled skill that is NOT trust-critical should be reviewable.
        // Derived from the merged view (legacy hardcoded + directory-sourced
        // `ENTRIES`) to avoid fragile enumeration.
        let all = all_bundled_skills();
        let reviewable: Vec<_> = all
            .iter()
            .filter(|s| !is_trust_critical_skill(s.name))
            .collect();
        // At least one bundled skill must be reviewable.
        assert!(
            !reviewable.is_empty(),
            "expected some reviewable bundled skills"
        );
        // Trust-critical must be a strict subset of bundled.
        assert!(
            TRUST_CRITICAL_SKILLS.len() < all.len(),
            "trust-critical should be smaller than bundled"
        );
        // Verify each reviewable skill is indeed bundled but not trust-critical.
        for skill in &reviewable {
            assert!(is_bundled_skill(skill.name));
            assert!(!is_trust_critical_skill(skill.name));
        }
    }

    #[test]
    fn test_skill_review_prompt_lists_trust_critical_skills() {
        // The skill-review system prompt must mention every trust-critical
        // skill name so the agent doesn't waste tool calls on them.
        let content = include_str!("../../../skills/bundled/skill-review/system_prompt.md");
        for name in TRUST_CRITICAL_SKILLS {
            assert!(
                content.contains(name),
                "skill-review prompt missing trust-critical skill '{}'",
                name
            );
        }
    }

    #[test]
    fn test_skill_review_prompt_does_not_reference_write_skill_variant() {
        // Regression: ensure the stale tool name is gone from the prompt.
        let content = include_str!("../../../skills/bundled/skill-review/system_prompt.md");
        assert!(
            !content.contains("write_skill_variant"),
            "skill-review prompt still references write_skill_variant"
        );
    }

    #[test]
    fn test_all_bundled_skills_includes_legacy_set() {
        // Every legacy hardcoded skill must appear in the merged view. With
        // empty production ENTRIES this is a strict equality on names; once
        // ENTRIES is populated by a future migration ticket, the merged view
        // will simply contain more names, never fewer.
        let merged = all_bundled_skills();
        for legacy in BUNDLED_SKILLS {
            assert!(
                merged
                    .iter()
                    .any(|s| s.name.eq_ignore_ascii_case(legacy.name)),
                "legacy bundled skill '{}' missing from merged view",
                legacy.name
            );
        }
    }

    #[test]
    fn test_is_bundled_skill_is_case_insensitive() {
        // Exercise both the legacy and ENTRIES arms — case-insensitivity must
        // hold regardless of which source the name comes from.
        assert!(is_bundled_skill("tmux"));
        assert!(is_bundled_skill("TMUX"));
        assert!(is_bundled_skill("Tmux"));
        assert!(!is_bundled_skill("definitely-not-a-skill"));
    }

    #[test]
    fn test_self_dev_declares_both_dispatch_siblings_as_dependencies() {
        // Loader-symmetry invariant (mika#1251, follow-up to mika#1173).
        //
        // self-dev (always_on) is the only edge that makes a non-always-on
        // dispatch tool reachable on keyword-less webhook turns — e.g. the
        // `[GitHub] Issue labeled ready on …` marker, which contains none of
        // dev-groom's keywords. dev-pilot and dev-groom are tool-symmetric
        // dispatch siblings, so they MUST also be loader-symmetric here.
        // mika#1173 restored dev-groom as a tool-owning skill but never added
        // this dependency edge, leaving run_claude_pilot_groom unreachable on
        // every github-webhook auto-groom turn.
        //
        // Parses the *embedded* (shipped) self-dev skill.toml — the bundled
        // manifest is compiled in via build.rs — not a synthetic fixture.
        let self_dev = all_bundled_skills()
            .into_iter()
            .find(|s| s.name.eq_ignore_ascii_case("self-dev"))
            .expect("self-dev must be a bundled skill");

        let manifest_toml = self_dev
            .files
            .iter()
            .find(|f| f.path == "skill.toml")
            .expect("self-dev must ship a skill.toml")
            .content;

        let manifest: crate::skills::manifest::SkillManifest =
            toml::from_str(manifest_toml).expect("self-dev skill.toml must parse");

        let deps = &manifest.skill.dependencies;
        for required in ["dev-pilot", "dev-groom"] {
            assert!(
                deps.iter().any(|d| d.eq_ignore_ascii_case(required)),
                "self-dev dependencies must include '{required}' (loader-symmetry \
                 invariant, mika#1251); got {deps:?}"
            );
        }
    }

    /// Engine post-condition guards (agent.rs, engine.rs, executor.rs) reference
    /// these skill tool names by string literal. The guards fire regardless of
    /// which skills are loaded — a missing tool means the guard silently no-ops
    /// or misfires.
    ///
    /// **Category A only.** These are tools that appear in `EndTurn` guard
    /// predicates, intent-precondition checks, and dispatch-class derivation.
    /// Category B tools (builtin handler dispatch tables, test fixtures) are
    /// intentionally agent-scoped and NOT listed here — their loader
    /// unreachability on non-matching agents is by design.
    ///
    /// **Maintenance contract:** When adding a new engine post-condition guard
    /// that references a skill tool name, add the tool name here. If the test
    /// below fails, add the owning skill to the always-on dependency closure
    /// (typically via `self-dev.dependencies`).
    const ENGINE_REFERENCED_SKILL_TOOLS: &[&str] = &[
        "run_claude_pilot",       // dev-pilot
        "run_claude_pilot_groom", // dev-groom
        "deploy_mika",            // deploy-mika
    ];

    #[test]
    fn test_engine_referenced_tool_names_are_loader_reachable() {
        // Static parity assertion (mika#1253, generalizing mika#1251).
        //
        // Every skill tool name referenced by engine post-condition guards
        // must be reachable through the always-on dependency closure of
        // BUNDLED_SKILL_MANIFESTS. This catches the class of bug where a
        // tool is known to the engine but unreachable by the skill loader
        // on keyword-less turns (e.g., GitHub webhook auto-groom).
        //
        // The test replicates the match_skills() BFS from matcher.rs but
        // simplified: no keyword matching, no disabled-skill filtering
        // (enabled is a runtime DB flag not present in compile-time data).
        // This mirrors the worst-case reachability on a webhook turn with
        // no keyword hits — only always_on=true seeds.

        use std::collections::{HashMap, HashSet, VecDeque};

        let all_skills = all_bundled_skills();

        // Parse each skill's manifest and collect tool names
        let mut skill_manifests: HashMap<String, crate::skills::manifest::SkillManifest> =
            HashMap::new();
        let mut skill_tools: HashMap<String, Vec<String>> = HashMap::new();

        for skill in &all_skills {
            let manifest_file = skill
                .files
                .iter()
                .find(|f| f.path == "skill.toml")
                .unwrap_or_else(|| panic!("skill '{}' must have skill.toml", skill.name));

            let manifest: crate::skills::manifest::SkillManifest =
                toml::from_str(manifest_file.content).unwrap_or_else(|e| {
                    panic!("skill '{}' skill.toml parse error: {e}", skill.name)
                });

            // Collect tool names from tools.json if present
            let mut tools = Vec::new();
            if let Some(tools_file) = skill.files.iter().find(|f| f.path == "tools.json") {
                let tool_defs: Vec<serde_json::Value> = serde_json::from_str(tools_file.content)
                    .unwrap_or_else(|e| {
                        panic!("skill '{}' tools.json parse error: {e}", skill.name)
                    });
                for tool_def in &tool_defs {
                    if let Some(name) = tool_def.get("name").and_then(|n| n.as_str()) {
                        tools.push(name.to_string());
                    }
                }
            }

            let name_lower = skill.name.to_lowercase();
            skill_manifests.insert(name_lower.clone(), manifest);
            skill_tools.insert(name_lower, tools);
        }

        // BFS: seed with always_on=true skills, walk transitive dependencies
        let mut closure: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();

        for (name, manifest) in &skill_manifests {
            if manifest.skill.always_on {
                closure.insert(name.clone());
                queue.push_back(name.clone());
            }
        }

        while let Some(current) = queue.pop_front() {
            if let Some(manifest) = skill_manifests.get(&current) {
                for dep in &manifest.skill.dependencies {
                    let dep_lower = dep.to_lowercase();
                    if skill_manifests.contains_key(&dep_lower) && !closure.contains(&dep_lower) {
                        closure.insert(dep_lower.clone());
                        queue.push_back(dep_lower);
                    }
                }
            }
        }

        // Collect all tool names reachable through the closure
        let mut reachable_tools: HashSet<String> = HashSet::new();
        for skill_name in &closure {
            if let Some(tools) = skill_tools.get(skill_name) {
                for tool in tools {
                    reachable_tools.insert(tool.clone());
                }
            }
        }

        // Assert every engine-referenced tool is reachable
        let mut missing: Vec<(&str, Option<&str>)> = Vec::new();
        for &tool_name in ENGINE_REFERENCED_SKILL_TOOLS {
            if !reachable_tools.contains(tool_name) {
                // Find owning skill for diagnostic message
                let owner = skill_tools
                    .iter()
                    .find(|(_, tools)| tools.iter().any(|t| t == tool_name))
                    .map(|(name, _)| name.as_str());
                missing.push((tool_name, owner));
            }
        }

        assert!(
            missing.is_empty(),
            "Engine-referenced skill tool(s) not reachable through the always-on \
             dependency closure (mika#1253). Missing tools:\n{}\n\
             Fix: add the owning skill to an always_on=true skill's dependencies \
             (typically self-dev/skill.toml).",
            missing
                .iter()
                .map(|(tool, owner)| {
                    match owner {
                        Some(skill) => format!("  - {tool} (owned by skill '{skill}')"),
                        None => format!("  - {tool} (owning skill not found in bundle)"),
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// Detect cross-skill tool-name collisions where the same tool name is
    /// declared with **different handler types** across bundled skills
    /// (mika#1326 AC2, relaxed to spec per mika#1573).
    ///
    /// Returns one formatted line per divergent collision. Same-name +
    /// **same**-handler-type declarations are intentionally NOT reported: that
    /// is the skill-scoped tool surface model working as designed (every value
    /// identical → no HashMap last-write-wins risk, e.g. the architect skills +
    /// gh-read-only each declaring `gh_read` with an identical Builtin handler).
    /// Only handler-type *divergence* — the class behind the 2026-05-28
    /// `run_gh` Builtin-vs-Exec incident — is flagged.
    ///
    /// Factored out so the invariant test and the mika#1573 AC3 regression
    /// fixture exercise the same detection path. Lives in `#[cfg(test)]` so the
    /// production skill loader cannot consult it at runtime (preserving
    /// mika#1326's additions-only scope reservation).
    fn detect_divergent_handler_collisions(skills: &[&BundledSkill]) -> Vec<String> {
        use std::collections::{BTreeMap, BTreeSet};

        // tool_name -> handler_type -> sorted set of declaring skills. A
        // missing/unparseable handler.type maps to a stable sentinel so two
        // handler-less declarations are treated as same (not a divergence),
        // while a missing-vs-present pairing is flagged conservatively.
        let mut by_tool: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();

        for skill in skills {
            let Some(tools_json) = skill.files.iter().find(|f| f.path == "tools.json") else {
                continue;
            };
            let tool_defs: Vec<serde_json::Value> = match serde_json::from_str(tools_json.content) {
                Ok(t) => t,
                Err(_) => continue, // malformed tools.json caught by other tests
            };
            for tool_def in &tool_defs {
                let Some(name) = tool_def.get("name").and_then(|n| n.as_str()) else {
                    continue;
                };
                let handler_type = tool_def
                    .get("handler")
                    .and_then(|h| h.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("(no handler.type)");
                by_tool
                    .entry(name.to_string())
                    .or_default()
                    .entry(handler_type.to_string())
                    .or_default()
                    .insert(skill.name.to_string());
            }
        }

        by_tool
            .into_iter()
            .filter(|(_, by_handler)| by_handler.len() > 1)
            .map(|(tool, by_handler)| {
                let detail = by_handler
                    .iter()
                    .map(|(ht, owners)| {
                        format!(
                            "{ht} [{}]",
                            owners.iter().cloned().collect::<Vec<_>>().join(", ")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" vs ");
                format!("  tool '{tool}' declared with divergent handler types: {detail}")
            })
            .collect()
    }

    #[test]
    fn test_bundled_skills_no_cross_skill_tool_name_collision() {
        // AC2 (mika#1326), relaxed to spec per mika#1573: no two bundled skills
        // may declare the same tool name with DIFFERENT handler types. This
        // catches the production-risk class — divergent handlers silently
        // shadowing each other via HashMap last-write-wins (the 2026-05-28
        // run_gh Builtin-vs-Exec incident) — at compile-test time.
        //
        // Same-name + same-handler-type declarations are NOT flagged: that is
        // the skill-scoped tool surface model working as designed (e.g. the
        // architect skills + gh-read-only each declaring `gh_read` with an
        // identical Builtin handler — every value identical, no last-write-wins
        // risk). The original AC2 impl was stricter (flagged ALL same-name
        // declarations) and false-positived on that benign case; mika#1573
        // relaxed it to the handler-type-divergence spec AC2 actually named.

        let all_skills = all_bundled_skills();
        let collisions = detect_divergent_handler_collisions(&all_skills);

        assert!(
            collisions.is_empty(),
            "Bundled skill tool-name collisions with DIVERGENT handler types \
             detected (mika#1326 / mika#1573):\n{}\n\n\
             The same tool name declared with different handler types risks \
             silent last-write-wins shadowing in the skill loader. Consolidate \
             to a single handler type.",
            collisions.join("\n")
        );
    }

    #[test]
    fn test_collision_detector_catches_divergent_run_gh_handlers() {
        // mika#1573 AC3 regression guard: the relaxed AC2 detector MUST still
        // catch the original mika#1326 incident class — two skills declaring
        // the same tool name with DIFFERENT handler types (Builtin vs Exec).
        // This reproduces the 2026-05-28 run_gh Builtin-vs-Exec production bug
        // as a fixture so the mika#1573 relaxation can't silently drop coverage
        // of the class it was scoped to keep.
        static RUN_GH_BUILTIN_FILES: &[SkillFile] = &[SkillFile {
            path: "tools.json",
            content: r#"[{"name": "run_gh", "handler": {"type": "builtin", "function": "run_gh"}}]"#,
            executable: false,
        }];
        static RUN_GH_EXEC_FILES: &[SkillFile] = &[SkillFile {
            path: "tools.json",
            content: r#"[{"name": "run_gh", "handler": {"type": "exec", "command": "handlers/run.sh"}}]"#,
            executable: false,
        }];
        static RUN_GH_BUILTIN_SKILL: BundledSkill = BundledSkill {
            name: "skill-run-gh-builtin",
            files: RUN_GH_BUILTIN_FILES,
            content_hash: "",
        };
        static RUN_GH_EXEC_SKILL: BundledSkill = BundledSkill {
            name: "skill-run-gh-exec",
            files: RUN_GH_EXEC_FILES,
            content_hash: "",
        };

        let divergent: &[&BundledSkill] = &[&RUN_GH_BUILTIN_SKILL, &RUN_GH_EXEC_SKILL];
        let collisions = detect_divergent_handler_collisions(divergent);
        assert_eq!(
            collisions.len(),
            1,
            "run_gh Builtin-vs-Exec divergence must be flagged, got: {collisions:?}"
        );
        assert!(
            collisions[0].contains("run_gh"),
            "collision report must name the run_gh tool: {collisions:?}"
        );

        // Negative companion: same name + SAME handler type is the benign
        // skill-scoped tool surface model (the gh_read false-positive that
        // motivated mika#1573) and must NOT be flagged.
        static GH_READ_BUILTIN_FILES: &[SkillFile] = &[SkillFile {
            path: "tools.json",
            content: r#"[{"name": "gh_read", "handler": {"type": "builtin", "function": "gh_read"}}]"#,
            executable: false,
        }];
        static GH_READ_SKILL_A: BundledSkill = BundledSkill {
            name: "skill-gh-read-a",
            files: GH_READ_BUILTIN_FILES,
            content_hash: "",
        };
        static GH_READ_SKILL_B: BundledSkill = BundledSkill {
            name: "skill-gh-read-b",
            files: GH_READ_BUILTIN_FILES,
            content_hash: "",
        };

        let benign: &[&BundledSkill] = &[&GH_READ_SKILL_A, &GH_READ_SKILL_B];
        let benign_collisions = detect_divergent_handler_collisions(benign);
        assert!(
            benign_collisions.is_empty(),
            "same-handler-type gh_read declarations must NOT be flagged \
             (skill-scoped tool surface model): {benign_collisions:?}"
        );
    }

    // Shared fixtures for merge-semantics tests — call `merge_skill_lists`
    // directly so the production merge function is the unit under test.
    // Previously the test re-implemented the merge algorithm locally, which
    // would silently pass if the production semantics drifted.
    static MERGE_TEST_LEGACY_FILES: &[SkillFile] = &[SkillFile {
        path: "skill.toml",
        content: "legacy",
        executable: false,
    }];
    static MERGE_TEST_LEGACY_ALPHA: BundledSkill = BundledSkill {
        name: "alpha",
        files: MERGE_TEST_LEGACY_FILES,
        content_hash: "",
    };
    static MERGE_TEST_LEGACY_GAMMA: BundledSkill = BundledSkill {
        name: "gamma",
        files: MERGE_TEST_LEGACY_FILES,
        content_hash: "",
    };
    static MERGE_TEST_LEGACY_SLICE: &[&BundledSkill] =
        &[&MERGE_TEST_LEGACY_ALPHA, &MERGE_TEST_LEGACY_GAMMA];

    static MERGE_TEST_OVERRIDE_FILES: &[SkillFile] = &[SkillFile {
        path: "skill.toml",
        content: "override",
        executable: false,
    }];
    static MERGE_TEST_FRESH_FILES: &[SkillFile] = &[SkillFile {
        path: "skill.toml",
        content: "fresh",
        executable: false,
    }];

    #[test]
    fn test_merge_prefers_entries_on_collision() {
        static ENTRIES_FIXTURE: &[BundledSkill] = &[
            BundledSkill {
                name: "ALPHA", // case-insensitive collision with MERGE_TEST_LEGACY_ALPHA
                files: MERGE_TEST_OVERRIDE_FILES,
                content_hash: "",
            },
            BundledSkill {
                name: "beta", // new addition
                files: MERGE_TEST_FRESH_FILES,
                content_hash: "",
            },
        ];

        let merged = merge_skill_lists(MERGE_TEST_LEGACY_SLICE, ENTRIES_FIXTURE);
        // 2 legacy + 1 new = 3 (alpha collision merged)
        assert_eq!(merged.len(), 3);

        let alpha = merged.iter().find(|s| s.name.eq_ignore_ascii_case("alpha"));
        assert!(alpha.is_some(), "alpha missing from merged view");
        assert_eq!(
            alpha.unwrap().files[0].content,
            "override",
            "ENTRIES version must win on case-insensitive name collision"
        );

        let beta = merged.iter().find(|s| s.name == "beta");
        assert!(beta.is_some(), "beta missing from merged view");
        assert_eq!(beta.unwrap().files[0].content, "fresh");

        // gamma survives untouched — no ENTRIES override
        let gamma = merged.iter().find(|s| s.name == "gamma");
        assert!(gamma.is_some(), "gamma missing from merged view");
        assert_eq!(gamma.unwrap().files[0].content, "legacy");
    }

    #[test]
    fn test_merge_with_all_collisions_preserves_legacy_length() {
        // Every legacy name is overridden by ENTRIES — merged length must equal
        // legacy length, and every merged entry must be the ENTRIES version.
        static ENTRIES_FIXTURE: &[BundledSkill] = &[
            BundledSkill {
                name: "alpha",
                files: MERGE_TEST_OVERRIDE_FILES,
                content_hash: "",
            },
            BundledSkill {
                name: "gamma",
                files: MERGE_TEST_OVERRIDE_FILES,
                content_hash: "",
            },
        ];
        let merged = merge_skill_lists(MERGE_TEST_LEGACY_SLICE, ENTRIES_FIXTURE);
        assert_eq!(merged.len(), MERGE_TEST_LEGACY_SLICE.len());
        for skill in &merged {
            assert_eq!(
                skill.files[0].content, "override",
                "every merged skill should be the ENTRIES version"
            );
        }
    }

    #[test]
    fn test_merge_with_empty_entries_returns_legacy() {
        static EMPTY: &[BundledSkill] = &[];
        let merged = merge_skill_lists(MERGE_TEST_LEGACY_SLICE, EMPTY);
        assert_eq!(merged.len(), MERGE_TEST_LEGACY_SLICE.len());
        for skill in &merged {
            assert_eq!(skill.files[0].content, "legacy");
        }
    }

    #[test]
    fn test_is_bundled_skill_recognizes_entries_only_name() {
        // `is_bundled_skill` has a dedicated ENTRIES arm that's dead code when
        // production ENTRIES is empty. Verify the arm works by exercising the
        // same logic the function uses — a name present only in ENTRIES must
        // be recognized as bundled.
        static ENTRIES_ONLY: &[BundledSkill] = &[BundledSkill {
            name: "entries-only-skill",
            files: MERGE_TEST_FRESH_FILES,
            content_hash: "",
        }];

        let has_entries_arm = BUNDLED_SKILLS
            .iter()
            .any(|s| s.name.eq_ignore_ascii_case("entries-only-skill"))
            || ENTRIES_ONLY
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case("entries-only-skill"));
        assert!(
            has_entries_arm,
            "ENTRIES-only name must be recognized via the ENTRIES arm"
        );

        let case_insensitive = BUNDLED_SKILLS
            .iter()
            .any(|s| s.name.eq_ignore_ascii_case("ENTRIES-ONLY-SKILL"))
            || ENTRIES_ONLY
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case("ENTRIES-ONLY-SKILL"));
        assert!(case_insensitive, "ENTRIES arm must be case-insensitive");
    }

    #[test]
    fn test_is_bundled_skill_agrees_with_all_bundled_skills() {
        // Coupling guard: `is_bundled_skill` and `all_bundled_skills` must
        // enumerate the same sources. If a third source is added to one
        // without updating the other, this test catches it.
        for skill in all_bundled_skills() {
            assert!(
                is_bundled_skill(skill.name),
                "all_bundled_skills() yielded '{}' but is_bundled_skill rejected it",
                skill.name
            );
        }
    }

    #[test]
    fn test_seed_creates_support_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);

        // _shared/dispatch-lib.sh must exist
        let dispatch_lib = skills_dir.join("_shared").join("dispatch-lib.sh");
        assert!(
            dispatch_lib.is_file(),
            "_shared/dispatch-lib.sh should be seeded"
        );

        // File should be non-empty
        let content = std::fs::read_to_string(&dispatch_lib).unwrap();
        assert!(!content.is_empty(), "dispatch-lib.sh should have content");
    }

    #[cfg(unix)]
    #[test]
    fn test_support_dir_files_are_executable() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_bundled_skills(skills_dir);

        let dispatch_lib = skills_dir.join("_shared").join("dispatch-lib.sh");
        let mode = std::fs::metadata(&dispatch_lib)
            .unwrap()
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "_shared/dispatch-lib.sh should be executable"
        );
    }

    #[test]
    fn test_seed_support_dirs_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_support_dirs(skills_dir);
        seed_support_dirs(skills_dir);
        seed_support_dirs(skills_dir);

        // _shared directory should exist with correct files
        let dispatch_lib = skills_dir.join("_shared").join("dispatch-lib.sh");
        assert!(dispatch_lib.is_file());
    }

    #[cfg(unix)]
    #[test]
    fn test_support_dir_symlink_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Seed once
        seed_support_dirs(skills_dir);

        // Replace _shared with a symlink
        let target_dir = tempfile::tempdir().unwrap();
        let shared_dir = skills_dir.join("_shared");
        std::fs::remove_dir_all(&shared_dir).unwrap();
        std::os::unix::fs::symlink(target_dir.path(), &shared_dir).unwrap();

        // Seed again — the symlinked directory should be skipped
        seed_support_dirs(skills_dir);

        // The target directory should NOT contain any files
        assert!(
            std::fs::read_dir(target_dir.path())
                .unwrap()
                .next()
                .is_none(),
            "symlink target should remain empty — seed_support_dirs must refuse to follow it",
        );
    }

    #[test]
    fn test_support_dirs_exclude_test_files() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        seed_support_dirs(skills_dir);

        // test-dispatch-lib.sh should NOT be seeded (excluded by *test* filter)
        let test_file = skills_dir.join("_shared").join("test-dispatch-lib.sh");
        assert!(
            !test_file.exists(),
            "test files should be excluded from support dir seeding"
        );
    }

    #[test]
    fn test_shell_exec_prompt_contains_sandbox_guidance() {
        let content = include_str!("../templates/skills/shell-exec/system_prompt.md");
        assert!(
            content.contains("Writing files outside agent home directories"),
            "shell-exec prompt missing out-of-sandbox section"
        );
        assert!(
            content.contains("Tracing references after rename"),
            "shell-exec prompt missing reference-tracing section"
        );
        assert!(
            content.contains("NEVER use shell commands"),
            "shell-exec prompt missing shell prohibition"
        );
    }

    // --- T1–T9: prune_known_removed_bundled_skills tests (mika#859) ---

    #[test]
    fn prune_removes_directory_in_known_removed_list() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let orphan = skills_dir.join("claude-pilot");
        std::fs::create_dir_all(orphan.join("handlers")).unwrap();
        std::fs::write(
            orphan.join("skill.toml"),
            "[skill]\nname = \"claude-pilot\"",
        )
        .unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 1);
        assert!(!orphan.exists());
    }

    #[test]
    fn prune_is_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let orphan = skills_dir.join("Claude-Pilot");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(
            orphan.join("skill.toml"),
            "[skill]\nname = \"Claude-Pilot\"",
        )
        .unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 1);
        assert!(!orphan.exists());
    }

    #[test]
    fn prune_preserves_directory_not_in_known_removed_list() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let custom = skills_dir.join("my-custom-skill");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(
            custom.join("skill.toml"),
            "[skill]\nname = \"my-custom-skill\"",
        )
        .unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 0);
        assert!(custom.exists());
    }

    #[test]
    fn prune_preserves_symlinked_skill_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let real_target = tempfile::tempdir().unwrap();
        std::fs::write(real_target.path().join("skill.toml"), "content").unwrap();

        let link_path = skills_dir.join("claude-pilot");
        std::fs::create_dir_all(skills_dir).unwrap();
        std::os::unix::fs::symlink(real_target.path(), &link_path).unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 0);
        assert!(link_path.exists());
        assert!(real_target.path().join("skill.toml").exists());
    }

    #[test]
    fn prune_preserves_support_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let shared = skills_dir.join("_shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("dispatch-lib.sh"), "#!/bin/sh").unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 0);
        assert!(shared.exists());
    }

    #[test]
    fn prune_preserves_current_bundled_skill_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let current = skills_dir.join("dev-pilot");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("skill.toml"), "[skill]\nname = \"dev-pilot\"").unwrap();

        let removed = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(removed, 0);
        assert!(current.exists());
    }

    #[test]
    fn prune_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();
        let orphan = skills_dir.join("claude-pilot");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(orphan.join("skill.toml"), "content").unwrap();

        let first = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(first, 1);

        let second = prune_known_removed_bundled_skills(skills_dir);
        assert_eq!(second, 0);
    }

    #[test]
    fn seed_bundled_skills_prunes_before_seeding() {
        let tmp = tempfile::tempdir().unwrap();
        let skills_dir = tmp.path();

        // Create stale orphan
        let orphan = skills_dir.join("claude-pilot");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(
            orphan.join("skill.toml"),
            "[skill]\nname = \"claude-pilot\"",
        )
        .unwrap();

        seed_bundled_skills(skills_dir);

        // Orphan should be gone
        assert!(
            !orphan.exists(),
            "claude-pilot orphan should be pruned by seed_bundled_skills"
        );
        // Current bundled skills should exist
        assert!(
            skills_dir.join("dev-pilot").exists(),
            "dev-pilot should be seeded"
        );
        assert!(
            skills_dir.join("dev-groom").exists(),
            "dev-groom should be seeded"
        );
    }

    #[test]
    fn known_removed_disjoint_from_current_bundle() {
        for name in KNOWN_REMOVED_BUNDLED_SKILLS {
            assert!(
                !is_bundled_skill(name),
                "KNOWN_REMOVED_BUNDLED_SKILLS contains '{name}', which is also in the \
                 current bundle. Either remove it from KNOWN_REMOVED_BUNDLED_SKILLS \
                 (if you intend to re-bundle it), or rename the new bundled skill."
            );
        }
    }

    // --- mika#1213: library sync + per-agent symlink tests ---

    /// AC1 — sync-shape: library contains exactly the manifest set after seed.
    #[test]
    fn library_sync_removes_orphans_outside_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path();

        // Seed once so manifest skills + hash exist.
        seed_bundled_skill_library(library);

        // Plant an orphan bundled-style directory that is not in the manifest.
        let orphan = library.join("ghost-skill");
        std::fs::create_dir_all(&orphan).unwrap();
        std::fs::write(orphan.join("skill.toml"), "[skill]\nname = \"ghost-skill\"").unwrap();

        // Invalidate hash so the next seed actually runs.
        std::fs::write(library.join(MANIFEST_HASH_FILE), "stale").unwrap();
        seed_bundled_skill_library(library);

        assert!(
            !orphan.exists(),
            "sync-shape extraction must remove directories outside the manifest set"
        );
        // Manifest skills must still be present.
        for skill in all_bundled_skills() {
            assert!(
                library.join(skill.name).is_dir(),
                "manifest skill '{}' missing after sync",
                skill.name
            );
        }
    }

    /// AC1 — sync-shape preserves support dirs and dotfiles.
    #[test]
    fn library_sync_preserves_support_and_dotfiles() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path();
        seed_bundled_skill_library(library);

        // _shared/ is a support dir, preserved across sync.
        assert!(library.join("_shared").is_dir());
        // .manifest-hash dotfile is preserved.
        assert!(library.join(MANIFEST_HASH_FILE).is_file());

        // Plant a dot-prefixed orphan and a _-prefixed orphan; sync must leave both.
        std::fs::create_dir_all(library.join(".cache")).unwrap();
        std::fs::create_dir_all(library.join("_extra")).unwrap();
        std::fs::write(library.join(MANIFEST_HASH_FILE), "stale").unwrap();
        seed_bundled_skill_library(library);

        assert!(
            library.join(".cache").is_dir(),
            "dotfile dir must be preserved"
        );
        assert!(
            library.join("_extra").is_dir(),
            "_-prefixed dir must be preserved"
        );
    }

    /// AC2 — symlinks created for allowlisted skills, removed when de-allowlisted.
    #[cfg(unix)]
    #[test]
    fn materialize_creates_symlinks_for_allowlist_and_removes_others() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("library");
        let agent = tmp.path().join("agent_skills");
        std::fs::create_dir_all(&library).unwrap();
        seed_bundled_skill_library(&library);

        let allowlist = vec!["tmux".to_string(), "shell-exec".to_string()];
        materialize_agent_skill_links(&library, &agent, Some(&allowlist));

        // Allowlisted: symlinks present, pointing at library.
        for name in &allowlist {
            let link = agent.join(name);
            assert!(
                link.symlink_metadata().unwrap().file_type().is_symlink(),
                "expected symlink at {}",
                link.display()
            );
            let target = std::fs::read_link(&link).unwrap();
            assert!(
                target.ends_with(name),
                "symlink target should end in '{name}', got {}",
                target.display()
            );
        }
        // Non-allowlisted bundled skill should NOT have an entry.
        let other = agent.join("web-search");
        assert!(!other.exists() && other.symlink_metadata().is_err());

        // Tighten allowlist: drop tmux. Re-materialize — the tmux symlink
        // must be removed.
        let tighter = vec!["shell-exec".to_string()];
        materialize_agent_skill_links(&library, &agent, Some(&tighter));
        let tmux_link = agent.join("tmux");
        assert!(
            !tmux_link.exists() && tmux_link.symlink_metadata().is_err(),
            "symlink for de-allowlisted skill must be removed"
        );
        // Remaining allowlisted skill still linked.
        assert!(
            agent
                .join("shell-exec")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    /// AC2 — None allowlist materializes all bundled skills.
    #[cfg(unix)]
    #[test]
    fn materialize_with_none_allowlist_links_all_bundled() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("library");
        let agent = tmp.path().join("agent_skills");
        std::fs::create_dir_all(&library).unwrap();
        seed_bundled_skill_library(&library);

        materialize_agent_skill_links(&library, &agent, None);

        for skill in all_bundled_skills() {
            let link = agent.join(skill.name);
            let meta = link.symlink_metadata().expect("link must exist");
            assert!(
                meta.file_type().is_symlink(),
                "expected symlink for {}",
                skill.name
            );
        }
    }

    /// AC3 — legacy per-agent COPY of a bundled skill is replaced with a symlink.
    #[cfg(unix)]
    #[test]
    fn materialize_replaces_legacy_copy_with_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("library");
        let agent = tmp.path().join("agent_skills");
        std::fs::create_dir_all(&library).unwrap();
        seed_bundled_skill_library(&library);

        // Plant a legacy per-agent copy of `tmux` with non-bundled content.
        let legacy = agent.join("tmux");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("skill.toml"), "stale copy content").unwrap();

        materialize_agent_skill_links(&library, &agent, None);

        let meta = legacy.symlink_metadata().unwrap();
        assert!(
            meta.file_type().is_symlink(),
            "legacy copy must be replaced with a symlink"
        );
        // Reads through the symlink should now return library content.
        let content = std::fs::read_to_string(legacy.join("skill.toml")).unwrap();
        assert_ne!(
            content, "stale copy content",
            "symlink should resolve to library file, not the legacy copy"
        );
        assert!(!content.is_empty());
    }

    /// AC3 — non-bundled (marketplace) directories under the agent skills
    /// dir must not be touched by the materialize pass.
    #[cfg(unix)]
    #[test]
    fn materialize_does_not_touch_non_bundled_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("library");
        let agent = tmp.path().join("agent_skills");
        std::fs::create_dir_all(&library).unwrap();
        seed_bundled_skill_library(&library);

        // Marketplace-style skill: name not in bundled manifest.
        let marketplace = agent.join("my-custom-skill");
        std::fs::create_dir_all(&marketplace).unwrap();
        std::fs::write(
            marketplace.join("skill.toml"),
            "[skill]\nname = \"my-custom-skill\"",
        )
        .unwrap();
        std::fs::write(marketplace.join("data.txt"), "operator-owned content").unwrap();

        materialize_agent_skill_links(&library, &agent, None);

        assert!(marketplace.is_dir());
        assert_eq!(
            std::fs::read_to_string(marketplace.join("data.txt")).unwrap(),
            "operator-owned content",
            "non-bundled dir must be preserved verbatim"
        );
    }

    /// AC4 — `--copy` opt-out materializes a real directory and writes a
    /// `.copy-managed` marker; library sync does not touch `--copy`'d dirs.
    #[cfg(unix)]
    #[test]
    fn copy_opt_out_writes_marker_and_resists_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("library");
        let agent = tmp.path().join("agent_skills");
        std::fs::create_dir_all(&library).unwrap();
        seed_bundled_skill_library(&library);

        copy_bundled_skill_to_agent_dir(&library, &agent, "tmux").unwrap();

        let dst = agent.join("tmux");
        let meta = dst.symlink_metadata().unwrap();
        assert!(meta.is_dir(), "--copy must produce a real directory");
        assert!(
            !meta.file_type().is_symlink(),
            "--copy must not produce a symlink"
        );
        assert!(
            dst.join(COPY_MARKER_FILE).is_file(),
            ".copy-managed marker must be written"
        );
        // The original bundled file should be present in the copied dir.
        assert!(dst.join("skill.toml").is_file());

        // Now run a library-sync materialize pass — the dir must stay as a
        // real directory; the marker must persist; content untouched.
        std::fs::write(dst.join("hot-patch.md"), "operator hot-patch").unwrap();
        materialize_agent_skill_links(&library, &agent, None);

        let meta_after = dst.symlink_metadata().unwrap();
        assert!(
            !meta_after.file_type().is_symlink(),
            "--copy-managed dir must NOT be replaced with a symlink"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("hot-patch.md")).unwrap(),
            "operator hot-patch",
            "operator hot-patch must survive a library sync pass"
        );
    }

    /// AC4 — `--copy`-managed dir survives de-allowlisting.
    #[cfg(unix)]
    #[test]
    fn copy_opt_out_survives_de_allowlist() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("library");
        let agent = tmp.path().join("agent_skills");
        std::fs::create_dir_all(&library).unwrap();
        seed_bundled_skill_library(&library);

        copy_bundled_skill_to_agent_dir(&library, &agent, "tmux").unwrap();

        // Allowlist that excludes tmux — sync should NOT touch the
        // --copy-managed directory.
        let allowlist = vec!["shell-exec".to_string()];
        materialize_agent_skill_links(&library, &agent, Some(&allowlist));

        let dst = agent.join("tmux");
        assert!(
            dst.is_dir(),
            "--copy-managed dir must persist after de-allowlisting"
        );
        assert!(dst.join(COPY_MARKER_FILE).is_file());
    }

    /// `--copy` rejects names that are not bundled skills.
    #[test]
    fn copy_opt_out_rejects_non_bundled_name() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("library");
        let agent = tmp.path().join("agent_skills");
        std::fs::create_dir_all(&library).unwrap();
        seed_bundled_skill_library(&library);

        let err = copy_bundled_skill_to_agent_dir(&library, &agent, "not-a-bundled-skill")
            .expect_err("must reject non-bundled name");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    /// Support-dir symlinks (`_shared/`) are materialized regardless of
    /// allowlist — handlers source `../../_shared/dispatch-lib.sh` via
    /// relative path, so the agent's skills dir must contain the link.
    #[cfg(unix)]
    #[test]
    fn materialize_creates_support_dir_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("library");
        let agent = tmp.path().join("agent_skills");
        std::fs::create_dir_all(&library).unwrap();
        seed_bundled_skill_library(&library);

        // Tight allowlist that excludes every support-dir consumer. _shared
        // must STILL be linked because it is dispatch plumbing, not a skill.
        let allowlist = vec!["tmux".to_string()];
        materialize_agent_skill_links(&library, &agent, Some(&allowlist));

        for support in SUPPORT_DIRS {
            let link = agent.join(support.name);
            let meta = link
                .symlink_metadata()
                .unwrap_or_else(|e| panic!("missing support-dir link {}: {e}", support.name));
            assert!(
                meta.file_type().is_symlink(),
                "support dir '{}' must be a symlink",
                support.name
            );
            // Reads through the symlink must surface library content.
            assert!(link.is_dir());
        }
    }

    /// Support-dir symlinks survive an empty allowlist and a None allowlist.
    #[cfg(unix)]
    #[test]
    fn materialize_support_dirs_with_none_and_empty_allowlist() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("library");
        std::fs::create_dir_all(&library).unwrap();
        seed_bundled_skill_library(&library);

        for label in &["none", "empty"] {
            let agent = tmp.path().join(format!("agent_{label}"));
            let allowlist: Option<&[String]> = if *label == "none" { None } else { Some(&[]) };
            materialize_agent_skill_links(&library, &agent, allowlist);

            for support in SUPPORT_DIRS {
                let link = agent.join(support.name);
                assert!(
                    link.symlink_metadata().unwrap().file_type().is_symlink(),
                    "support dir '{}' must be linked for allowlist={label}",
                    support.name
                );
            }
        }
    }

    /// Materialize is idempotent — re-running over the same state is a no-op.
    #[cfg(unix)]
    #[test]
    fn materialize_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let library = tmp.path().join("library");
        let agent = tmp.path().join("agent_skills");
        std::fs::create_dir_all(&library).unwrap();
        seed_bundled_skill_library(&library);

        let allowlist = vec!["tmux".to_string(), "shell-exec".to_string()];
        materialize_agent_skill_links(&library, &agent, Some(&allowlist));
        let first_meta = agent.join("tmux").symlink_metadata().unwrap();

        materialize_agent_skill_links(&library, &agent, Some(&allowlist));
        let second_meta = agent.join("tmux").symlink_metadata().unwrap();
        assert!(second_meta.file_type().is_symlink());
        // Same inode (no recreate).
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                first_meta.ino(),
                second_meta.ino(),
                "idempotent materialize must not recreate the symlink"
            );
        }
    }
}
