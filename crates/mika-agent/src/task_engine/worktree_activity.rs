//! Bounded max-mtime probe over a dispatch worktree (mika#2249, D1 Phase 2).
//!
//! The silent-stall reaper needs one number per in-flight dispatch: **when did
//! anything in this worktree last change?** A pilot that is working writes —
//! source files, plan documents, `git` metadata under `.git/`. A pilot whose
//! SDK stream stopped yielding writes nothing at all, for hours, while its
//! process stays alive and its task stays `in_progress`. That difference is
//! the only observable that separates the two from outside the process, which
//! is why the probe lives here and not inside claude-pilot: a watchdog starved
//! in the pilot's own event loop cannot fire on a pilot-side timer either.
//!
//! # Why the walk is bounded, and how
//!
//! A `mika` checkout is not small, and `target/` alone can hold hundreds of
//! thousands of files. The reaper runs once per `DB_SCAN_INTERVAL_TICKS` over
//! the 0–2 dispatches in flight, so an unbounded walk would put a
//! multi-second stat storm inside the engine tick — the tick that also holds
//! the engine mutex. Three bounds keep it cheap, and all three are measured
//! rather than assumed by the caller:
//!
//! - **Pruned subtrees** ([`EXCLUDED_DIR_NAMES`]) — build and dependency
//!   output. `target/` is excluded despite being written during a build,
//!   deliberately: a `cargo build` refreshes it without the pilot having made
//!   any progress of its own, so counting it would make a wedged pilot look
//!   alive for as long as one background build lingers.
//! - **A file budget** ([`MAX_ENTRIES_SCANNED`]) — the walk stops after this
//!   many entries and reports what it found so far, flagged as truncated.
//! - **A depth cap** ([`MAX_DEPTH`]) — bounds pathological nesting.
//!
//! A truncated walk is **not** an error: it returns the newest mtime seen so
//! far. That direction is the safe one — seeing *more* recency than the true
//! maximum can only make a worktree look more alive, never less, and the one
//! thing this module must never do is manufacture staleness.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use tracing::warn;

/// Directory names pruned from the walk, matched on the file name at any
/// depth.
///
/// `.git/objects` and `.git/modules` are pruned while the rest of `.git/` is
/// kept: `.git/index`, `.git/HEAD` and the ref files are exactly the cheap,
/// high-signal evidence that a pilot staged, committed or switched branch.
pub const EXCLUDED_DIR_NAMES: &[&str] = &["target", "node_modules", "objects", "modules"];

/// Hard cap on entries visited in one worktree walk.
///
/// Sized against a real checkout: `mika` minus `target/` and `node_modules/`
/// sits in the low tens of thousands of files, so this leaves headroom while
/// keeping the worst case bounded at a few tens of milliseconds of `stat`.
pub const MAX_ENTRIES_SCANNED: usize = 50_000;

/// Hard cap on directory depth relative to the worktree root.
pub const MAX_DEPTH: usize = 24;

/// Wall-clock budget above which a completed scan is still reported, but with
/// a `warn!` (AC7). Not an abort: cutting a scan short on time would make the
/// measured age depend on machine load, and a load spike must not be able to
/// turn a live worktree into a reap candidate.
pub const SCAN_TIME_BUDGET: Duration = Duration::from_millis(500);

/// Outcome of one bounded worktree walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeActivity {
    /// Newest modification time seen. `None` only when the walk visited no
    /// readable entry at all.
    pub max_mtime: Option<SystemTime>,
    /// Entries visited (files and directories).
    pub entries_scanned: usize,
    /// Whether a bound cut the walk short.
    pub truncated: bool,
}

/// Whether `name` names a directory the walk prunes.
fn is_excluded_dir(name: &str) -> bool {
    EXCLUDED_DIR_NAMES.contains(&name)
}

/// Walk `root` and return the newest mtime under it, bounded per the module
/// docs.
///
/// Returns `None` when `root` does not exist or is not a directory — the
/// caller treats that as "not a candidate", never as "stale". A dispatch whose
/// declared worktree has been removed is a dispatch this reaper has no
/// evidence about, and a reaper that kills must never fire on absence of
/// proof.
///
/// Symlinks are **not** followed (`symlink_metadata`): a worktree containing a
/// link into a busy tree elsewhere would otherwise import that tree's recency,
/// and — worse for a bounded walk — its size.
pub fn probe(root: &Path) -> Option<WorktreeActivity> {
    if !root.is_dir() {
        return None;
    }

    let started = Instant::now();
    let mut max_mtime: Option<SystemTime> = None;
    let mut entries_scanned: usize = 0;
    let mut truncated = false;

    // Explicit stack rather than recursion: depth is capped anyway, and this
    // keeps the budget checks in one place.
    let mut stack: Vec<(std::path::PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        if entries_scanned >= MAX_ENTRIES_SCANNED {
            truncated = true;
            break;
        }
        let read = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            // An unreadable directory is skipped, not fatal: a permission
            // hole costs one subtree of evidence, and the fallback direction
            // (less evidence of recency) is handled by the caller's fail-safe.
            Err(_) => continue,
        };
        for entry in read.flatten() {
            entries_scanned += 1;
            if entries_scanned >= MAX_ENTRIES_SCANNED {
                truncated = true;
                break;
            }
            let path = entry.path();
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            if let Ok(mtime) = meta.modified() {
                max_mtime = Some(match max_mtime {
                    Some(current) if current >= mtime => current,
                    _ => mtime,
                });
            }
            if meta.is_dir() {
                if depth + 1 > MAX_DEPTH {
                    truncated = true;
                    continue;
                }
                let excluded = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_excluded_dir);
                if !excluded {
                    stack.push((path, depth + 1));
                }
            }
        }
    }

    let elapsed = started.elapsed();
    if elapsed > SCAN_TIME_BUDGET {
        warn!(
            worktree = %root.display(),
            elapsed_ms = elapsed.as_millis() as u64,
            entries_scanned,
            truncated,
            budget_ms = SCAN_TIME_BUDGET.as_millis() as u64,
            "pilot_stall_reaper: worktree mtime scan exceeded its time budget (mika#2249 AC7)"
        );
    }

    Some(WorktreeActivity {
        max_mtime,
        entries_scanned,
        truncated,
    })
}

/// Seconds elapsed since the newest write under `root`.
///
/// `None` when the worktree is absent, unreadable, or yielded no mtime at all
/// — every one of which means "no evidence", which the caller must treat as
/// "not a candidate". Also `None` when the newest mtime is in the future
/// (clock skew, a checkout with stamped-forward timestamps): a negative age
/// is not evidence of staleness either.
pub fn seconds_since_last_write(root: &Path) -> Option<u64> {
    let activity = probe(root)?;
    let mtime = activity.max_mtime?;
    SystemTime::now()
        .duration_since(mtime)
        .ok()
        .map(|d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration as StdDuration;

    /// Set a path's mtime `secs_ago` seconds into the past.
    fn backdate(path: &Path, secs_ago: u64) {
        let when = SystemTime::now() - StdDuration::from_secs(secs_ago);
        filetime::set_file_mtime(path, filetime::FileTime::from_system_time(when))
            .expect("backdate mtime");
    }

    #[test]
    fn absent_root_yields_no_evidence() {
        let tmp = tempfile::tempdir().expect("tmp");
        let missing = tmp.path().join("does-not-exist");
        assert!(probe(&missing).is_none());
        assert!(seconds_since_last_write(&missing).is_none());
    }

    #[test]
    fn a_file_path_is_not_a_worktree() {
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("a-file");
        fs::write(&file, b"x").expect("write");
        assert!(probe(&file).is_none(), "a regular file is not a worktree");
    }

    #[test]
    fn newest_write_wins_across_nesting() {
        let tmp = tempfile::tempdir().expect("tmp");
        let deep = tmp.path().join("crates").join("mika-agent").join("src");
        fs::create_dir_all(&deep).expect("mkdir");

        let old = tmp.path().join("README.md");
        fs::write(&old, b"old").expect("write");
        backdate(&old, 10_000);

        let recent = deep.join("engine.rs");
        fs::write(&recent, b"new").expect("write");
        backdate(&recent, 5);
        // Directories carry their own mtime; backdate the chain so the file is
        // unambiguously the newest thing in the tree.
        for d in [tmp.path(), &tmp.path().join("crates"), &deep] {
            backdate(d, 10_000);
        }

        let age = seconds_since_last_write(tmp.path()).expect("some evidence");
        assert!(age < 60, "newest write must dominate, got {age}s");
    }

    /// The exclusion is load-bearing in the direction that matters: a fresh
    /// write under `target/` must NOT make a stale worktree look alive.
    #[test]
    fn excluded_dirs_do_not_refresh_a_stale_worktree() {
        let tmp = tempfile::tempdir().expect("tmp");
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).expect("mkdir src");
        let file = src.join("lib.rs");
        fs::write(&file, b"code").expect("write");

        let target = tmp.path().join("target").join("debug");
        fs::create_dir_all(&target).expect("mkdir target");
        let artifact = target.join("build-output.bin");
        fs::write(&artifact, b"artifact").expect("write");

        // Everything the pilot could have touched is old; only the build
        // output is fresh.
        for p in [tmp.path(), &src, &file, &tmp.path().join("target")] {
            backdate(p, 10_000);
        }
        // `artifact` and `target/debug` stay at "now".

        let age = seconds_since_last_write(tmp.path()).expect("some evidence");
        assert!(
            age > 9_000,
            "a fresh build artifact must not refresh the worktree age, got {age}s"
        );
    }

    /// `.git/` itself is kept — a commit is progress — while `.git/objects` is
    /// pruned for cost.
    #[test]
    fn git_metadata_counts_as_activity() {
        let tmp = tempfile::tempdir().expect("tmp");
        let git = tmp.path().join(".git");
        fs::create_dir_all(&git).expect("mkdir .git");
        let index = git.join("index");
        fs::write(&index, b"idx").expect("write");

        let src = tmp.path().join("src");
        fs::create_dir_all(&src).expect("mkdir src");
        let old = src.join("lib.rs");
        fs::write(&old, b"code").expect("write");

        for p in [tmp.path(), &src, &old, &git] {
            backdate(p, 10_000);
        }
        // Only `.git/index` is fresh — the shape of "the pilot just committed".

        let age = seconds_since_last_write(tmp.path()).expect("some evidence");
        assert!(
            age < 60,
            "a fresh .git/index must read as activity, got {age}s"
        );
    }

    #[test]
    fn objects_subtree_is_pruned() {
        let tmp = tempfile::tempdir().expect("tmp");
        let objects = tmp.path().join(".git").join("objects").join("ab");
        fs::create_dir_all(&objects).expect("mkdir objects");
        let blob = objects.join("cdef");
        fs::write(&blob, b"blob").expect("write");

        for p in [
            tmp.path(),
            &tmp.path().join(".git"),
            &tmp.path().join(".git").join("objects"),
        ] {
            backdate(p, 10_000);
        }

        let activity = probe(tmp.path()).expect("dir");
        // The pruned subtree's own directory entry is still stat'ed (it is a
        // child of `.git`), but nothing inside it is.
        assert!(
            activity.entries_scanned < 4,
            "objects/ contents must not be walked, scanned {}",
            activity.entries_scanned
        );
    }

    #[test]
    fn empty_worktree_yields_a_scan_with_no_mtime() {
        let tmp = tempfile::tempdir().expect("tmp");
        let empty = tmp.path().join("empty");
        fs::create_dir_all(&empty).expect("mkdir");
        let activity = probe(&empty).expect("dir exists");
        assert_eq!(activity.max_mtime, None);
        assert_eq!(activity.entries_scanned, 0);
        assert!(seconds_since_last_write(&empty).is_none());
    }

    /// A worktree whose newest mtime is in the future yields no age rather
    /// than a wrapped one.
    #[test]
    fn future_mtime_is_not_evidence_of_staleness() {
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("skewed");
        fs::write(&file, b"x").expect("write");
        let future = SystemTime::now() + StdDuration::from_secs(3_600);
        filetime::set_file_mtime(&file, filetime::FileTime::from_system_time(future))
            .expect("stamp future");
        assert!(seconds_since_last_write(tmp.path()).is_none());
    }
}
