//! Structural test: enforces the LECTURE SEULE invariant of the milestone_manager module.
//!
//! **Rationale:** Prompt-level "no dispatch" rules are fragile per
//! `feedback_prompt_enforcement_fragile`. A compile-time-adjacent test that greps the
//! module tree for forbidden write-authority tokens is a structural gate that catches
//! regressions before merge.
//!
//! The invariant this test protects:
//! - No `run_claude_pilot` invocation site (dispatch to dev-pilot).
//! - No `pr_merge_with_gate` invocation site (PR merge).
//! - No `gh api` invocation with any mutating HTTP method (`PATCH`/`POST`/`DELETE`/`PUT`).
//! - No `gh issue edit` (label mutations).
//! - No `gh pr merge` (PR merge).
//! - No `gh pr comment` / `gh issue comment` (write to GitHub).
//!
//! If a legitimate use of any of these ever appears (e.g., Phase 2 promotion after the
//! 3 portes are cleared), the fix is to update this test AND the module docstring
//! atomically — never to silently loosen the test.

use std::fs;
use std::path::Path;

/// Forbidden token patterns — grepped against every `.rs` file under the module.
const FORBIDDEN_TOKENS: &[&str] = &[
    "run_claude_pilot",
    "pr_merge_with_gate",
    "gh pr merge",
    "gh issue edit",
    "gh pr comment",
    "gh issue comment",
    "gh issue close",
    "gh issue reopen",
    "gh pr close",
    "gh pr reopen",
    "gh pr ready",
    "gh label create",
    "gh api -X PATCH",
    "gh api -X POST",
    "gh api -X DELETE",
    "gh api -X PUT",
    "\"PATCH\"",
    "\"POST\"",
    "\"DELETE\"",
    "\"PUT\"",
];

/// Files exempt from the grep — this test file itself contains the tokens as literals.
const EXEMPT_FILES: &[&str] = &["no_dispatch_test.rs"];

#[test]
fn no_dispatch_scaffolding_in_milestone_manager() {
    // Resolve the module directory across the two CWD contexts this test runs
    // under: (a) `cargo test -p mika-agent` from the workspace root — CWD is
    // typically the workspace or crate root; (b) IDE test runners that set CWD
    // to the crate directory. `CARGO_MANIFEST_DIR` is stable across both.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let module_dir = Path::new(manifest_dir)
        .join("src")
        .join("milestone_manager");
    assert!(
        module_dir.exists(),
        "expected module dir at {} (CARGO_MANIFEST_DIR={manifest_dir})",
        module_dir.display()
    );

    let mut violations: Vec<(String, String)> = Vec::new();
    walk_and_scan(&module_dir, &mut violations);

    if !violations.is_empty() {
        let msg = violations
            .iter()
            .map(|(f, t)| format!("  {f} contains forbidden token `{t}`"))
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "LECTURE SEULE invariant violated — milestone_manager must not contain \
             dispatch/write-authority tokens:\n{msg}\n\n\
             If this is intentional (Phase 2 promotion), update no_dispatch_test.rs \
             AND the module docstring atomically."
        );
    }
}

fn walk_and_scan(dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_and_scan(&path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if EXEMPT_FILES.contains(&file_name) {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        // Strip line comments (both `//` and `///`) so doc comments that
        // *describe* the forbidden tokens don't trip the guard. The guard's
        // job is to catch executable code, not prose.
        let code_only = strip_line_comments(&content);
        for token in FORBIDDEN_TOKENS {
            if code_only.contains(token) {
                out.push((path.display().to_string(), (*token).to_string()));
            }
        }
    }
}

/// Strip Rust line comments (everything after `//` on each line). Leaves
/// string literals containing `//` unchanged is out of scope — we only
/// need to filter the common case of documentation prose mentioning
/// forbidden tokens.
fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            // Naive: find the first `//` not inside a string literal.
            // Sufficient for our module's actual code shapes.
            let mut in_string = false;
            let mut chars = line.char_indices().peekable();
            while let Some((i, c)) = chars.next() {
                if c == '"' && !line[..i].ends_with('\\') {
                    in_string = !in_string;
                }
                if !in_string
                    && c == '/'
                    && let Some(&(_, '/')) = chars.peek()
                {
                    return line[..i].to_string();
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
