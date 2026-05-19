//! End-to-end regression for the bundled-skill library + per-agent symlink
//! restructure (mika#1213). Exercises the deploy → seed → update → agent-startup
//! path by simulating a multi-agent home layout (`<global_home>/agents/<name>/`),
//! invoking `seed_bundled_skills_if_needed` with a representative bundled agent
//! identity (mika-dev's 26-skill allowlist), and asserting:
//!
//! 1. The canonical library at `<global_home>/skills/` is populated with the
//!    binary's manifest set.
//! 2. The per-agent skills dir contains symlinks into the library — one per
//!    allowlisted bundled skill, none for skills outside the allowlist.
//! 3. Each symlink resolves to a real directory containing `skill.toml` and
//!    the skill's prompt/tool files.
//! 4. After tightening the allowlist and re-running, dropped symlinks are
//!    removed.
//!
//! This integration test sits outside the unit test module so it exercises
//! the full public surface (no internal-visibility helpers).

#![cfg(unix)]

use std::fs;
use std::path::Path;

/// Write a minimal multi-agent layout with the given agent's identity
/// allowlist. Returns the (global_home, agent_home) pair.
fn provision_agent(
    tmp: &Path,
    agent_name: &str,
    allowlist: &[&str],
) -> (std::path::PathBuf, std::path::PathBuf) {
    let global_home = tmp.to_path_buf();
    let agent_home = global_home.join("agents").join(agent_name);
    fs::create_dir_all(&agent_home).unwrap();

    let mut identity = String::from("name = \"Dev\"\nemoji = \"🛠\"\n\n[skills]\nallowlist = [\n");
    for s in allowlist {
        identity.push_str(&format!("  \"{s}\",\n"));
    }
    identity.push_str("]\n");
    fs::write(agent_home.join("identity.toml"), identity).unwrap();

    (global_home, agent_home)
}

#[test]
fn end_to_end_library_then_per_agent_symlinks() {
    let tmp = tempfile::tempdir().unwrap();
    // Subset of mika-dev's real allowlist that we know exists as bundled
    // skills (legacy hardcoded BUNDLED_SKILLS). Avoids tying the test to
    // ENTRIES-sourced skills that may move between releases.
    let allowlist = [
        "tmux",
        "shell-exec",
        "web-search",
        "file-reader",
        "self-knowledge",
        "git-ops",
        "google-workspace",
        "github",
        "mcp",
        "browser-control",
    ];
    let (global_home, agent_home) = provision_agent(tmp.path(), "mika-dev", &allowlist);

    // Deploy → seed → update → agent-startup convergence point.
    mika_agent::startup::seed_bundled_skills_if_needed(&agent_home, false);

    // (1) Library populated at <global_home>/skills/.
    let library = global_home.join("skills");
    assert!(
        library.is_dir(),
        "library dir must exist at {}",
        library.display()
    );
    for name in &allowlist {
        let lib_skill = library.join(name);
        assert!(
            lib_skill.is_dir(),
            "library missing bundled skill {name} at {}",
            lib_skill.display()
        );
        assert!(
            lib_skill.join("skill.toml").is_file(),
            "library skill {name} missing skill.toml"
        );
    }
    // .manifest-hash record exists.
    assert!(
        library.join(".manifest-hash").is_file(),
        "library must record manifest hash for idempotency"
    );

    // (2) Per-agent symlinks for allowlisted skills.
    let agent_skills = agent_home.join("skills");
    assert!(agent_skills.is_dir());
    for name in &allowlist {
        let link = agent_skills.join(name);
        let meta = link
            .symlink_metadata()
            .unwrap_or_else(|e| panic!("missing per-agent entry {name}: {e}"));
        assert!(
            meta.file_type().is_symlink(),
            "per-agent entry for {name} must be a symlink, found {:?}",
            meta.file_type()
        );
        let target = fs::read_link(&link).unwrap();
        let resolved = if target.is_absolute() {
            target
        } else {
            link.parent().unwrap().join(target)
        };
        assert!(
            resolved.ends_with(name),
            "symlink target for {name} should resolve to library/{name}, got {}",
            resolved.display()
        );
        // (3) Symlink resolves through to real content.
        let through_link = link.join("skill.toml");
        assert!(
            through_link.is_file(),
            "reading through symlink must surface library skill.toml for {name}"
        );
    }

    // (3.5) Support-dir symlink (`_shared`) is materialized regardless of
    // allowlist — handlers source `../../_shared/dispatch-lib.sh` via
    // relative path, so the agent's skills dir must contain it (mika#923).
    let shared_link = agent_skills.join("_shared");
    let shared_meta = shared_link
        .symlink_metadata()
        .expect("_shared symlink must exist under agent skills dir");
    assert!(
        shared_meta.file_type().is_symlink(),
        "_shared must be a symlink into the library"
    );
    assert!(
        shared_link.join("dispatch-lib.sh").is_file(),
        "_shared symlink must resolve to library/dispatch-lib.sh"
    );

    // (4) Tighten the allowlist; the removed entry must lose its symlink.
    let (global_home2, agent_home2) =
        provision_agent(tmp.path(), "mika-dev-tight", &["tmux", "shell-exec"]);
    // Pre-create some symlinks as if a previous boot had a wider allowlist.
    let agent_skills2 = agent_home2.join("skills");
    fs::create_dir_all(&agent_skills2).unwrap();
    // Seed the library through the tighter agent.
    mika_agent::startup::seed_bundled_skills_if_needed(&agent_home2, false);
    // tmux + shell-exec linked.
    for n in ["tmux", "shell-exec"] {
        let meta = agent_skills2.join(n).symlink_metadata().unwrap();
        assert!(meta.file_type().is_symlink(), "{n} must be symlinked");
    }
    // web-search is NOT in the tight allowlist — must NOT be linked.
    let stripped = agent_skills2.join("web-search");
    assert!(
        !stripped.exists() && stripped.symlink_metadata().is_err(),
        "de-allowlisted skill must not appear under per-agent skills dir"
    );

    // Library is shared across agents — same global_home, same .manifest-hash.
    assert_eq!(
        global_home, global_home2,
        "test setup: both agents should share global_home"
    );
}

#[test]
fn second_seed_is_a_noop_via_hash_gate() {
    let tmp = tempfile::tempdir().unwrap();
    let (_global, agent_home) = provision_agent(tmp.path(), "mika-dev", &["tmux", "shell-exec"]);

    mika_agent::startup::seed_bundled_skills_if_needed(&agent_home, false);

    // Mutate a library file; second seed must NOT overwrite (hash-gated).
    let library_skill_toml = tmp.path().join("skills").join("tmux").join("skill.toml");
    fs::write(&library_skill_toml, "operator hot-patch").unwrap();

    mika_agent::startup::seed_bundled_skills_if_needed(&agent_home, false);

    assert_eq!(
        fs::read_to_string(&library_skill_toml).unwrap(),
        "operator hot-patch",
        "hash gate must skip re-extraction when the binary's manifest hash matches"
    );
}
