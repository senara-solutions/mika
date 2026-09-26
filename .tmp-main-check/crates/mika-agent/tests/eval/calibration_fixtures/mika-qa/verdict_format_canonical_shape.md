## PR: fix(verdict): parser tolerates non-canonical mika-qa verdict shapes (mika#1828)

**Repository:** senara-solutions/mika
**Branch:** `fix/1828/verdict-parser-drops-mika-qa-reviews`

### Context

A markdown-heavy PR that provides multiple **highly emphasized** sections and headers. The
review body will be heavy in `**bold**` and `__underscored__` markdown — the model has to
resist the temptation to wrap its OWN `VERDICT:` line in similar emphasis just because the
surrounding prose is emphasized.

## Executive Summary

- **Goal.** Widen `parse_verdict` to tolerate the observed non-canonical mika-qa shapes.
- **Ship path.** DECISION-CORE, Vincent hand-merge.
- **Priority.** _p1-important_ — loop substrate.

## Diff

```rust
// crates/mika-agent/src/server/verdict.rs

static VERDICT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?mi)^\s*[*_]*\s*VERDICT:\s*(.+)$").expect("verdict regex")
});

fn strip_md_emphasis(value: &str) -> &str {
    value.trim_matches(|c: char| c == '*' || c == '_')
}
```

New tests added covering `**VERDICT: pass**`, `**Verdict: REQUEST CHANGES**`, aliases
`REQUEST_CHANGES` / `CHANGES_REQUESTED` / `APPROVE` — all mapping to canonical `Verdict`
enum variants.

## Acceptance Criteria

- **AC1.** `parse_verdict("**VERDICT: pass**")` returns `Verdict::Pass`.
- **AC2.** `parse_verdict("VERDICT: REQUEST CHANGES")` returns `Verdict::Block("ac")`.
- **AC3.** `parse_verdict("VERDICT: frobnicate")` still returns `Verdict::Missing`.
- **AC4.** New calibration scenario `verdict_format_canonical_shape` — asserts the model
  itself emits canonical shape (no markdown wrapper, no alias tokens).
- **AC5.** Unit-tests cover markdown-bold + alias + regression + edge cases.

## Verification

- `cargo test -p mika-agent --lib server::verdict` — all pass.
- `cargo clippy` clean, `cargo fmt` clean.

Please review and emit your VERDICT.
