//! Worktree reaper for PR-closed webhook turns (mika#1694).
//!
//! Substrate hygiene (AC3). When a `[GitHub] PR closed:` webhook arrives, the
//! branch's dispatch worktree under `.claude/worktrees/` is terminal — no
//! further pilot work will target it, whether the PR merged or was closed
//! unmerged. This handler reaps that worktree deterministically, in-process,
//! host-side, the moment the PR closes. It closes the debt-accumulation gap
//! Vincent surfaced in mika#1694 (14 stranded worktrees, "is the platform
//! going to be stable one day").
//!
//! ## Why a structural handler and not a bundled skill
//!
//! The mika#1694 plan's first shape (Option b) was a `self-dev-webhook-pr-closed`
//! bundled skill that would reap from inside the LLM turn. The plan flagged an
//! F1 pre-implementation blocker: verify a webhook-triggered turn can reach the
//! host worktree filesystem. It cannot reliably — `self-dev-webhook-ci`'s Rule 5
//! documents that `run_shell`/`write_agent_file` on a webhook turn are sandboxed
//! to the agent home and do not reach worktree paths. LLM-driven reaping would
//! also be non-deterministic — the opposite of the "flawless, no shortcuts"
//! reliability the ticket asks for.
//!
//! `[GitHub] PR closed:` events are already intercepted in-process by
//! [`super::milestone_context_handler`], which runs in the mika-spirit process
//! (host-side in dev mode) with the same filesystem reach dispatch-lib uses for
//! `git worktree remove`. A structural handler here is deterministic, unit-
//! testable, and carries no cross-repo ordering dependency on the companion
//! mika-platform `worktrees-clean` script (AC2).
//!
//! ## Safety contract
//!
//! - **Scoped removal.** Only reaps worktrees whose path is under
//!   `.claude/worktrees/` — never the primary checkout or a human's manual
//!   worktree.
//! - **Dirty refusal.** Refuses to remove a dirty worktree; flags it via a
//!   `worktree.reap_skipped_dirty` audit event so residual pilot work is never
//!   silently discarded (the silent-rot class in mika#1694). After mika#1282
//!   post-flight recovery, dirty content is normally committed into a `wip()`
//!   draft PR before close, so this is the safety net, not the common path.
//! - **Graceful no-op.** When the configured repo dir is absent or is not a git
//!   repository, the handler is a silent no-op — the correct behavior in
//!   containerized production, where dispatch worktrees do not live on the
//!   agent's filesystem. It never fails the turn.
//! - **Side-effect only.** Always returns `Passthrough { enrichment: None }`;
//!   the existing milestone-advance / acknowledgement flow is unchanged.

use super::verdict_handler::VerdictAction;
use crate::async_db::AsyncDatabase;
use tokio::process::Command;
use tracing::{debug, info, warn};

/// Prefix on PR-closed webhook messages from the gateway.
const PR_CLOSED_PREFIX: &str = "[GitHub] PR closed:";

/// Path segment every managed dispatch worktree contains. The reaper refuses to
/// touch any worktree whose path does not include this segment.
const MANAGED_WORKTREE_SEGMENT: &str = "/.claude/worktrees/";

/// Default mika checkout used to enumerate worktrees, when
/// `MIKA_WORKTREE_REAP_REPO_DIR` is unset. Matches the canonical dev-mode path
/// documented in the workspace `CLAUDE.md`. When this path does not exist (e.g.
/// containerized production), the handler no-ops.
const DEFAULT_REPO_DIR: &str = "/data/workspace/mika-platform/mika";

/// Inspect a PR-closed webhook message and reap the branch's dispatch worktree.
///
/// Runs before the LLM turn as a pure side effect. Always returns
/// `Passthrough { enrichment: None }`.
pub(crate) async fn try_reap_closed_pr_worktree(
    text: &str,
    db: &AsyncDatabase,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    // Step 1: Match prefix. Reap on both merge and close-without-merge — both
    // are terminal for the branch's worktree (mika#1694 Layer C.1).
    if !text.starts_with(PR_CLOSED_PREFIX) {
        return passthrough();
    }

    // Step 2: Resolve the repo dir. No-op when absent / not a git repo.
    let repo_dir = resolve_repo_dir();
    if repo_dir.is_empty() {
        debug!("worktree_reaper: reaping disabled (empty repo dir)");
        return passthrough();
    }
    if !std::path::Path::new(&repo_dir).join(".git").exists() {
        debug!(
            repo_dir = %repo_dir,
            "worktree_reaper: repo dir is not a git checkout, skipping (expected in containerized prod)"
        );
        return passthrough();
    }

    // Step 3: Extract the branch from the webhook message.
    let branch = match extract_branch(text) {
        Some(b) => b,
        None => {
            debug!("worktree_reaper: no branch found in PR-closed message");
            return passthrough();
        }
    };

    // Step 4: Enumerate the git worktree registry (ground truth) and match the
    // branch. Reading the registry sidesteps any path-derivation drift — we reap
    // the worktree git itself has checked out for this branch.
    let porcelain = match run_git(&repo_dir, &["worktree", "list", "--porcelain"]).await {
        Some(out) => out,
        None => {
            warn!(repo_dir = %repo_dir, "worktree_reaper: `git worktree list` failed");
            return passthrough();
        }
    };
    let worktree_path = match parse_worktree_for_branch(&porcelain, &branch) {
        Some(p) => p,
        None => {
            debug!(branch = %branch, "worktree_reaper: no worktree for branch (already reaped or never created)");
            return passthrough();
        }
    };

    // Step 5: Safety guard — only touch managed dispatch worktrees.
    if !worktree_path.contains(MANAGED_WORKTREE_SEGMENT) {
        warn!(
            branch = %branch,
            worktree_path = %worktree_path,
            "worktree_reaper: matched worktree is outside .claude/worktrees/, refusing to reap"
        );
        return passthrough();
    }

    // Step 6: Dirty refusal — never discard uncommitted pilot work silently.
    match run_git(&worktree_path, &["status", "--porcelain"]).await {
        Some(status) if !status.trim().is_empty() => {
            warn!(
                branch = %branch,
                worktree_path = %worktree_path,
                "worktree_reaper: DIRTY worktree — refusing to remove, run `make worktrees-clean` or salvage (see docs/operator/worktree-hygiene.md)"
            );
            log_audit(
                db,
                session_id,
                "worktree.reap_skipped_dirty",
                &worktree_path,
                &format!("branch={branch} dirty=true"),
                trace_id,
            )
            .await;
            return passthrough();
        }
        Some(_) => {} // clean — proceed
        None => {
            warn!(worktree_path = %worktree_path, "worktree_reaper: `git status` failed, skipping reap");
            return passthrough();
        }
    }

    // Step 7: Remove the worktree, then delete the local branch. `worktree
    // remove` from the primary checkout unregisters the linked worktree and
    // deletes its directory.
    if run_git(&repo_dir, &["worktree", "remove", &worktree_path])
        .await
        .is_none()
    {
        warn!(
            branch = %branch,
            worktree_path = %worktree_path,
            "worktree_reaper: `git worktree remove` failed"
        );
        log_audit(
            db,
            session_id,
            "worktree.reap_failed",
            &worktree_path,
            &format!("branch={branch} stage=remove"),
            trace_id,
        )
        .await;
        return passthrough();
    }

    // Branch delete is best-effort: a merged branch deletes cleanly with -d, but
    // a closed-unmerged branch needs -D. The worktree is already gone regardless.
    let branch_deleted = run_git(&repo_dir, &["branch", "-D", &branch])
        .await
        .is_some();
    if !branch_deleted {
        debug!(branch = %branch, "worktree_reaper: local branch delete failed (may not exist)");
    }

    info!(
        branch = %branch,
        worktree_path = %worktree_path,
        branch_deleted = branch_deleted,
        "worktree_reaper: reaped worktree on PR close"
    );
    log_audit(
        db,
        session_id,
        "worktree.reaped",
        &worktree_path,
        &format!("branch={branch} branch_deleted={branch_deleted}"),
        trace_id,
    )
    .await;

    passthrough()
}

/// This handler never consumes the event.
fn passthrough() -> VerdictAction {
    VerdictAction::Passthrough { enrichment: None }
}

/// Resolve the mika checkout used to enumerate worktrees.
/// `MIKA_WORKTREE_REAP_REPO_DIR` overrides the dev-mode default. An explicit
/// empty value disables the reaper.
fn resolve_repo_dir() -> String {
    match std::env::var("MIKA_WORKTREE_REAP_REPO_DIR") {
        Ok(v) => v.trim().to_string(),
        Err(_) => DEFAULT_REPO_DIR.to_string(),
    }
}

/// Run a git subcommand in `cwd`. Returns stdout on exit 0, `None` otherwise.
async fn run_git(cwd: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Write an audit event, warning on DB failure (never fatal to the turn).
async fn log_audit(
    db: &AsyncDatabase,
    session_id: &str,
    tool_name: &str,
    worktree_path: &str,
    reasoning: &str,
    trace_id: &str,
) {
    if let Err(e) = db
        .log_audit_event(
            session_id,
            tool_name,
            &format!("worktree:{worktree_path}"),
            None,
            None,
            Some(reasoning),
            Some(trace_id),
        )
        .await
    {
        warn!(error = %e, tool_name = %tool_name, "worktree_reaper: failed to write audit event");
    }
}

/// Extract the branch name from a PR-closed webhook first line.
/// Format: `[GitHub] PR closed: <repo>#<n> — <title> (branch: <branch>)`.
/// Parses the last `(branch: …)` group so a title containing the literal
/// `(branch: …)` cannot shadow the real trailer.
fn extract_branch(text: &str) -> Option<String> {
    let first_line = text.lines().next()?;
    let marker = "(branch: ";
    let start = first_line.rfind(marker)? + marker.len();
    let rest = &first_line[start..];
    let end = rest.find(')')?;
    let branch = rest[..end].trim();
    if branch.is_empty() {
        return None;
    }
    Some(branch.to_string())
}

/// Parse `git worktree list --porcelain` output and return the worktree path
/// whose checked-out branch is `refs/heads/<branch>`.
fn parse_worktree_for_branch(porcelain: &str, branch: &str) -> Option<String> {
    let target_ref = format!("refs/heads/{branch}");
    let mut current_path: Option<&str> = None;
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current_path = Some(path.trim());
        } else if let Some(branch_ref) = line.strip_prefix("branch ") {
            if branch_ref.trim() == target_ref {
                return current_path.map(|p| p.to_string());
            }
        } else if line.trim().is_empty() {
            current_path = None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_branch_valid() {
        let text = "[GitHub] PR closed: senara-solutions/mika#1694 — fix substrate (branch: fix/1694/substrate-hygiene)\nhttps://github.com/senara-solutions/mika/pull/1700\nMerged: true";
        assert_eq!(
            extract_branch(text),
            Some("fix/1694/substrate-hygiene".to_string())
        );
    }

    #[test]
    fn test_extract_branch_title_contains_branch_literal() {
        // A title mentioning "(branch: foo)" must not shadow the real trailer.
        let text = "[GitHub] PR closed: senara-solutions/mika#1 — rename (branch: old) to (branch: new) (branch: feat/real)";
        assert_eq!(extract_branch(text), Some("feat/real".to_string()));
    }

    #[test]
    fn test_extract_branch_missing() {
        let text = "[GitHub] PR closed: senara-solutions/mika#1 — no branch trailer";
        assert_eq!(extract_branch(text), None);
    }

    #[test]
    fn test_extract_branch_empty() {
        let text = "[GitHub] PR closed: senara-solutions/mika#1 — x (branch: )";
        assert_eq!(extract_branch(text), None);
    }

    #[test]
    fn test_parse_worktree_for_branch_found() {
        let porcelain = "worktree /data/workspace/mika-platform/mika\nHEAD abc123\nbranch refs/heads/main\n\nworktree /data/workspace/mika-platform/.claude/worktrees/fix-1694-x/mika\nHEAD def456\nbranch refs/heads/fix/1694/substrate-hygiene\n";
        assert_eq!(
            parse_worktree_for_branch(porcelain, "fix/1694/substrate-hygiene"),
            Some("/data/workspace/mika-platform/.claude/worktrees/fix-1694-x/mika".to_string())
        );
    }

    #[test]
    fn test_parse_worktree_for_branch_returns_main_only_when_matched() {
        let porcelain =
            "worktree /data/workspace/mika-platform/mika\nHEAD abc123\nbranch refs/heads/main\n";
        assert_eq!(
            parse_worktree_for_branch(porcelain, "main"),
            Some("/data/workspace/mika-platform/mika".to_string())
        );
        assert_eq!(parse_worktree_for_branch(porcelain, "feat/nope"), None);
    }

    #[test]
    fn test_parse_worktree_for_branch_not_found() {
        let porcelain = "worktree /data/workspace/mika-platform/mika\nHEAD abc123\nbranch refs/heads/main\n\nworktree /tmp/wt/other\nHEAD def456\nbranch refs/heads/feat/other\n";
        assert_eq!(parse_worktree_for_branch(porcelain, "fix/absent"), None);
    }

    #[test]
    fn test_parse_worktree_detached_head_skipped() {
        // A detached-HEAD worktree has no `branch` line; must not false-match.
        let porcelain = "worktree /data/workspace/mika-platform/mika\nHEAD abc123\nbranch refs/heads/main\n\nworktree /tmp/wt/detached\nHEAD def456\ndetached\n";
        assert_eq!(
            parse_worktree_for_branch(porcelain, "main"),
            Some("/data/workspace/mika-platform/mika".to_string())
        );
    }

    #[test]
    fn test_managed_worktree_segment_guard() {
        // The primary checkout path does not contain the managed segment.
        assert!(!"/data/workspace/mika-platform/mika".contains(MANAGED_WORKTREE_SEGMENT));
        // A dispatch worktree does.
        assert!(
            "/data/workspace/mika-platform/.claude/worktrees/fix-1694-x/mika"
                .contains(MANAGED_WORKTREE_SEGMENT)
        );
    }
}
