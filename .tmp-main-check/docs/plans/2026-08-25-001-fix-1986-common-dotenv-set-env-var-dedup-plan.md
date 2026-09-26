# Plan — fix(common,dotenv): set_env_var must dedup existing duplicate lines

**Status:** DRAFT
**Date:** 2026-08-25
**Ticket:** mika#1986
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** Substrate reliability — silent config-drift → hours of Prime downtime

## Why

Founding incident 2026-08-24: Prime KO ~4h. `~/.mika/.env` silently contained **two** `MIKA_ANTHROPIC_API_KEY` lines — a metered `sk-ant-api*` on line 2 and a subscription `sk-ant-oat*` on line 3. `dotenvy` last-wins, so the OAuth token won; `is_oauth_token()` routed auth through `OAuthTokenManager`; Anthropic revoked the subscription grant that morning; refresh failed; Prime KO. Two-hour duo diag with sami-Claude to reach root cause. Fix path once surfaced: `sed -i '3d' ~/.mika/.env` + restart spirit.

The doublon appearance was silent: no WARN at write time, no WARN at load time. The write helper (`mika-common::dotenv::set_env_var`) does NOT guarantee at-most-one entry per key when the input file already has duplicates — see "Codebase reality" below.

## Codebase reality (verified, not inferred)

`crates/mika-common/src/dotenv.rs:190-226` (`set_env_var`) today:

```rust
if let Ok(content) = std::fs::read_to_string(&env_path) {
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && let Some((k, _)) = trimmed.split_once('=')
            && k.trim() == key
        {
            let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
            lines.push(format!("{key}=\"{escaped}\""));
            found = true;
            continue;
        }
        lines.push(line.to_string());
    }
}
```

Each matching line pushes a replacement. Given input `key=A\nkey=B\n` + `set_env_var(key, C)`, the output is `key=C\nkey=C\n` — **two** lines, both new value. The function does NOT dedup. Existing test `test_set_env_var_updates_existing` (dotenv.rs:414-426) exercises the single-match case (which happens to work) via `assert_eq!(content.matches("FOO=").count(), 1)`; the multi-match case has no test coverage today.

`crates/mika-common/src/dotenv.rs:26-74` (`load_dotenv`) emits the three-state observability triad (`dotenv_loaded` / `dotenv_absent` / `dotenv_load_error`) via both `eprintln!` (pre-init durable) and `info!/error!` (post-init structured). No detection of same-key duplicates — the operator has no greppable signal that config drift exists.

Related but out-of-scope: setup.rs:275-297 filters existing `MIKA_ANTHROPIC_API_KEY` on `is_oauth_token()`; when the filter returns `None` it prompts + calls `set_env_var()`. That code is untouched — the fix is defense-in-depth at the write layer and observability at the read layer.

## What

Three surgical edits to `crates/mika-common/src/dotenv.rs`, plus unit tests.

### AC1 — Dedup pass in `set_env_var`

Rewrite the loop so the output guarantees at most ONE line per key, regardless of input state:

- First match → push replacement, mark `written`.
- Subsequent matches (same key) → drop silently (dedup).
- Non-matching lines (comments, blanks, other keys) → preserved verbatim in order.
- No match found → append at end (unchanged).

The escape/format is identical to today (`value.replace('\\', "\\\\").replace('"', "\\\"")`, `format!("{key}=\"{escaped}\"")`). Only the loop shape changes.

**Write-semantics invariant for the single-key case (mandatory):** for inputs where the key appears zero or one times, the output MUST be byte-identical to today's implementation. This is verified by keeping the existing tests (`test_set_env_var_creates_file`, `test_set_env_var_updates_existing`, `test_set_env_var_roundtrip_*`, `test_set_env_var_permissions`) unchanged and green — they lock the single-line case shape.

### AC2 — Load-time doublon warning in `load_dotenv`

Add a helper `count_duplicate_keys(env_path: &Path) -> HashMap<String, usize>` that parses the file as text (mirrors `env_file_contains_key`'s parser: strip whitespace, skip comments, split on first `=`, tolerate optional `export ` prefix) and returns per-key **count** for keys appearing more than once.

In `load_dotenv`'s `Ok(())` arm, after emitting `dotenv_loaded`, iterate the map and emit one WARN per duplicate key:

```rust
warn!(
    target: "mika::env",
    event = "dotenv_duplicate_key",
    key = %key,
    count = count,
    path = %env_path.display(),
    "duplicate .env key — dotenvy last-wins may select unintended value"
);
```

Also emit an `eprintln!` per duplicate on the pre-init durable channel (same rationale as the load/absent/error triad — `logging::init()` runs after `load_dotenv()` in mika-spirit).

Non-fatal. `dotenvy`'s last-wins semantics are preserved. The whole point is to make silent config drift greppable in server logs.

**Value NOT emitted.** The doublon in the founding incident carries the OAuth token verbatim — logging the value would leak the secret. Emit key + count only.

### AC3 — Unit tests (fixture-based)

Add three inline tests to `crates/mika-common/src/dotenv.rs::tests`:

1. `test_set_env_var_dedups_existing_duplicates` — write fixture `.env` with two lines for the same key with **different values** (matching the founding incident's shape), call `set_env_var(key, new_value)`, assert exactly ONE line for the key with the new value.
2. `test_set_env_var_dedups_three_duplicates_preserves_others` — three duplicate lines + one unrelated key; asserts dedup + unrelated preserved + comments preserved.
3. `test_count_duplicate_keys_detects_multi_line_same_key` — write fixture `.env` with two-line doublon + one unique key, assert the helper returns `{doubled_key: 2}` and nothing for the unique key.

The write-semantics-invariant assertion from AC1 is covered by keeping the existing `test_set_env_var_updates_existing` (which asserts `content.matches("FOO=").count() == 1` on a single-match input) unchanged and green.

### AC4 — Manual verification

Post-merge, on a synthetic `.env` copy in a tempdir shaped as:

```
KEY_X=old-a
KEY_X=old-b
```

Run a code path that calls `set_env_var(home, "KEY_X", "new")` (a `cargo test` on the new test suffices, or a one-shot binary). Verify the resulting `.env` has exactly `KEY_X="new"` (one line). Not automated in CI beyond AC3 tests; documented here for operator record.

## Acceptance Criteria

- [ ] AC1: `set_env_var` produces at most one line per key on write (dedup on write), including when input already has ≥2 lines matching the key.
- [ ] AC2: `load_dotenv` emits a WARN `dotenv_duplicate_key` structured event (and pre-init `eprintln!`) per-key when the file contains multiple non-comment lines with the same key. Key + count + path fields; value NEVER logged.
- [ ] AC3: unit tests cover (a) two-line duplicate dedup on write, (b) three-line duplicate + unrelated key preservation, (c) `count_duplicate_keys` helper detects the doublon shape.
- [ ] AC4: manual verify on synthetic `.env` doublon fixture (satisfied by AC3 test execution).
- [ ] Write-semantics invariant: single-key case output byte-identical to prior impl — locked by existing `test_set_env_var_updates_existing` remaining unchanged and green.
- [ ] Secret hygiene: duplicate-key WARN emits key + count only, never the value.

## Definition of Done

- `cargo fmt` clean.
- `cargo clippy --all-targets --all-features -- -D warnings` clean for `mika-common`.
- `cargo test -p mika-common` green (existing + new tests).
- PR opened with `Closes #1986` and HEAD sha in body.
- Sister ticket mika#1985 code untouched.

## Non-goals

- Not changing dotenvy's own load-time last-wins behavior (upstream decision, correct at that layer).
- Not touching secret content, secret rotation, or OAuth-vs-API-key routing (that's Rail A/B separation, orthogonal — see `project_anthropic_rail_split_api_vs_oauth_2026-08-24`).
- Not rewriting `.env` in-place at load time. Write is `set_env_var`'s job; load is diagnostic.
- Not filing follow-ups for the setup.rs `is_oauth_token()` filter behavior — separate concern, not a bug.

## Risk

- **Low.** Single-file change scoped to `crates/mika-common/src/dotenv.rs`. No API signature change. No caller updates required. Existing tests remain in place and green as the write-semantics-invariant proof.
- Failure mode if regression: existing `test_set_env_var_updates_existing` (single-line dedup assertion) would break — caught in CI.

## Related

- Sister ticket: mika#1985 — CLI error-chain fix for the same founding incident (separate PR).
- Founding incident: 2026-08-24 Prime OAuth invalidation → 4h Prime downtime, duo diag.
- Doctrine memory: `project_anthropic_rail_split_api_vs_oauth_2026-08-24.md` — Rail A (`sk-ant-api*` metered) vs Rail B (`sk-ant-oat*` subscription); a doublon that mixes the two silently promotes Rail B and takes the whole spirit down when the OAuth grant is revoked.
