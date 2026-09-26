---
module: server
date: 2026-05-27
problem_type: security_issue
component: authentication
severity: high
symptoms:
  - "ANSI-C quoting $'\\x60id\\x60' bypasses both pre-classifier layers (Rust + Python)"
  - "Neither contains_unquoted_metacharacter() scanner detects the $' two-byte sequence"
  - "Bash expands \\xNN escapes inside $'...' at execution time, enabling command substitution via backtick injection"
root_cause: missing_validation
resolution_type: code_fix
tags:
  - security
  - permission-pre-classifier
  - ansi-c-quoting
  - shell-expansion
  - metacharacter-scanner
  - cross-repo
  - defense-in-depth
related_components:
  - tooling
---

# ANSI-C Quoting `$'\xNN'` Bypasses Both Pre-Classifier Layers

## Problem

Bash supports ANSI-C quoting via the `$'...'` syntax. Inside the quoted region, escape sequences like `\xNN` (hex), `\nNN` (octal), and named escapes (`\n`, `\t`, etc.) are expanded at execution time. Neither the Rust pre-classifier (`permission_pre_classifier.rs`) nor the Python tier1 scanner (`tier1.py`) detected the `$'` two-byte sequence that opens this quoting mode.

## Symptoms

The command `mika ask --agent mika-arch $'\x60id\x60'` contained no literal backtick (0x60) and no `$(` sequence — only `$`, `'`, `\`, `x`, `6`, `0`, etc. Both scanners walked past it without flagging. At bash execution time, `\x60` expands to backtick, enabling command substitution.

Surfaced by `/ce:review` adversarial reviewer (P1, confidence 0.88) during the mika#938 fix PR.

## What Didn't Work

The previous blanket Branch 5 scanner (pre-mika#938) also missed this pattern. The mika#938 quote-aware refactor preserved the gap because the focus was on `$(`, backtick, `>(`, and `<(` — the four patterns that cause direct shell expansion.

## Solution

Add a `$'` two-byte check to `contains_unquoted_metacharacter()` in both scanners, placed between the existing `$(` check and the `>(` check so the `$<x>` patterns cluster contiguously.

**Rust** (`crates/mika-agent/src/server/permission_pre_classifier.rs`):

```rust
// Check for `$'` (ANSI-C quoting — escapes like \xNN expand at execution time)
if bytes[i] == b'$' && i + 1 < len && bytes[i + 1] == b'\'' {
    return true;
}
```

**Python** (`claude-pilot-py/src/claude_pilot/tier1.py`):

```python
# $' (ANSI-C quoting — escapes like \xNN expand at execution time)
if ch == "$" and i + 1 < n and command[i + 1] == "'":
    return True
```

The check fires only when `$` is followed by `'` in **unquoted context**. Inside `"..."` or `'...'`, the `$` never reaches the unquoted branch — the quote-aware carve-out from mika#938 is preserved.

## Why This Works

The `$'` two-byte sequence is the sole entry point for bash ANSI-C quoting. POSIX disallows whitespace between `$` and `'`, so the check is exact. Plain `$` followed by anything else (`$HOME`, `${VAR}`, `$1`, `$_`) is NOT rejected — only the specific `$'` opening sequence triggers.

The existing quote-state FSM handles the carve-out correctly:
- Inside `"..."`: the `$` is consumed in the `Some(q)` arm and never reaches the `None` (unquoted) arm where the check lives
- Inside `'...'`: similarly consumed inside the quoted state
- After a closing quote: `$'` appears in unquoted context and is correctly detected

## Prevention

> **Note (2026-06-03):** the original cross-language framing here described a Rust+Python coupling that no longer exists. The Rust side was retired in mika#1193 (see Update below). The bullets are rewritten as within-`claude-pilot-py` invariants — the only side left.

- **Within-cpp metacharacter sentinel**: `tier1.py`'s `contains_unquoted_metacharacter()` scanner and `policy.py`'s compound-chain allow guard (`_bash_allow_is_chain_safe`, mika cpp#25) share one metacharacter set (`$(`, backtick, `$'`, `>(`, `<(`). The two consumers must agree on what they guard. When adding a new metacharacter pattern, update both files in the same change and add a test on each side.
- **Test parity within cpp**: `tier1.py`'s test suite and `policy.py`'s test suite mirror each other for the metacharacter scanner cases. New bypasses get tests at both layers, locking the coupling.
- **Defense-in-depth ordering**: `contains_unquoted_metacharacter()` runs before `split_compound_command()` in the cpp policy stack. Downstream parsers that don't understand `$'...'` never see it — the scanner rejects the command first.

## References

- mika#944 — this issue
- mika#938 — quote-aware scanner (original design)
- mika#942 — `>(` / `<(` process substitution extension
- mika#946 / claude-pilot-py#16 — Python scanner port
- senara-solutions/mika#1320 — Rust PR
- senara-solutions/claude-pilot-py#17 — Python companion PR

## Update (2026-06-03): Rust pre-classifier retired — `claude-pilot-py/tier1.py` is now the sole owner

The Rust `permission_pre_classifier.rs` and its `contains_unquoted_metacharacter()` scanner described above **no longer exist** — they were deleted in **mika#1193** (2026-05-30) when mika-relay was retired. Bash-command permission classification now lives **exclusively in `claude-pilot-py`** (`src/claude_pilot/tier1.py` allow-list + `src/claude_pilot/policy.py` compound-chain guard + `src/claude_pilot/policies/permissions.yaml` rules). The original "cross-language sentinel" framing in the Prevention section above has been rewritten as a within-cpp invariant (tier1 ↔ policy coupling), which is the only coupling that still exists.

Anyone auditing or extending command-string permission safety should look there, not here. Two known follow-ups tracked on that repo:

- **claude-pilot-py#25** (shipped) — hardened the policy `allow` path against compound-chained tails (allow-list over segments) and heredoc desyncs. Durable writeup: `claude-pilot-py/docs/solutions/security-issues/command-string-policy-allow-rules-are-compound-unsafe.md`.
- **claude-pilot-py#27** (open) — `tier1.py`'s first-word allow-list trusts `awk`/`sed` without guarding their code-exec sub-features (`awk 'BEGIN{system(...)}'`, GNU `sed` `e`-flag auto-approve). The TIER3 denylist is incomplete by nature and is not the safety boundary (the allow-list is).
