---
title: A flag's semantics must not depend on another flag's value
date: 2026-04-27
category: best-practices
module: mika-cli
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - "Adding a CLI flag whose effect varies across encoding/format flags"
  - "A flag's PR description carves out a specific format and ships only that subset"
  - "Reviewers see 'scoped to text mode only' (or similar) and accept it as scope discipline"
related_components:
  - development_workflow
tags:
  - cli-design
  - orthogonality
  - verbose
  - format
  - envelope
---

# A flag's semantics must not depend on another flag's value

## Context

mika#824 (merged 2026-04-26) added `mika ask --verbose` to emit a `session_id` trailer for cross-command integration (`/mika-groom-ticket` and friends). The PR carved scope tightly: *"Text mode only — JSON mode unchanged."* That sentence read like discipline at review time. It was actually an orthogonality violation written into the contract.

The next day (2026-04-27), the JSON path of `/mika-ask-arch` had to fall back to querying `~/.mika/data/mika.db` directly via `sqlite3` because `mika ask --verbose --format json` returned no metadata at all:

```
$ mika ask --verbose --format json ping
{"role":"assistant","content":"Here."}
```

A slash command was reaching across into another component's storage to recover information the CLI should have returned. mika-platform#54 was opened to fix this and had to drop `--format json` from the slash command, switching to text-mode trailer parsing — because JSON mode had no metadata channel at all.

The structural problem isn't "JSON mode is missing a feature." The problem is that **`--verbose`'s semantics depended on `--format`'s value**. A user reading `mika ask --help` saw two independent flags; in practice they were entangled.

## Guidance

When adding a flag (`A`) that interacts with an existing flag (`B`), assume from the outset that **`A`'s semantics must hold under every value of `B`**. The two flags should be conceptually orthogonal axes that compose, not nested branches.

Concretely for output-shaping flags like `--verbose`:

- **`--verbose` controls *whether* metadata is emitted.**
- **`--format` controls *how* metadata is encoded.**
- Each format must carry the metadata in some shape. "Skip this format" is not an option without a strong reason that survives scrutiny.

For `mika ask`:

```rust
// Before (mika#824) — JSON arm has no `if verbose` clause at all
OutputFormat::Json => {
    let response = AskJsonResponse {
        role: "assistant",
        content: output.text,
        task_id: task_id.map(|s| s.to_string()),
        pending_tasks: pending_callbacks,
    };
    println!("{}", serde_json::to_string(&response)?);
}

// After (mika#829) — same flag, same semantics, different encoding
OutputFormat::Json => {
    let metadata = if verbose {
        Some(MetadataEnvelope { session_id: Some(session_id.clone()) })
    } else {
        None
    };
    let response = AskJsonResponse {
        role: "assistant",
        content: output.text,
        task_id: task_id.map(|s| s.to_string()),
        pending_tasks: pending_callbacks,
        metadata,
    };
    println!("{}", serde_json::to_string(&response)?);
}
```

`#[serde(skip_serializing_if = "Option::is_none")]` on the `metadata` field keeps the no-`--verbose` output byte-identical to today — backward compatibility for existing JSON consumers comes for free with the right serde annotation.

## Why This Matters

A flag whose semantics depend on another flag's value is a **type-system violation expressed in CLI grammar**. The flags are documented as independent inputs but the runtime treats them as a tuple. That gap manifests in three concrete failure modes:

1. **Documentation lies by omission.** `--verbose --help` describes "emit metadata trailer." It does not describe "emit nothing under `--format json`." Even when CLAUDE.md disclosed the gap, the disclosure was a footnote, not a contract.
2. **Downstream consumers grow workarounds.** mika-platform's `/mika-ask-arch` had to query SQLite directly to recover the session_id. That's a slash command crossing component boundaries to do work the CLI's contract should have done.
3. **The fix gets deferred indefinitely.** mika#824's PR-body intention was "scope discipline; JSON mode in a follow-up." No follow-up was filed. The follow-up only materialized when an end-to-end use surfaced the contract violation by failing.

The general lesson: **reviewers should challenge "scoped to format X only" carve-outs in flag PRs.** Either there's a real reason the flag is meaningless under format Y (rare), or the carve-out is technical debt being accepted as scope discipline (common). Distinguish them at review time, not at consumer-bite time.

## When to Apply

- Reviewing or designing a CLI flag whose behavior involves output formatting, encoding, or response shape.
- A PR description includes phrasing like "text mode only", "scoped to format X", "JSON mode unchanged" — *specifically when the flag's name does not imply a format-specific meaning*.
- An existing flag's `--help` text describes a format-independent effect, but the implementation contains `match format { ... }` arms where one arm omits the effect.
- Designing a metadata/observability flag (`--verbose`, `--debug`, `--explain`, `--trace`) that must carry information across multiple output paths.

## Examples

### Anti-pattern: text-mode-only `--verbose` (mika#824)

```rust
// `verbose` only checked inside one format arm
OutputFormat::Text => {
    println!("{text}");
    if verbose { println!("session_id: {session_id}"); }
}
OutputFormat::Json => {
    // No verbose handling — flag is silently no-op here
    println!("{}", serde_json::to_string(&response)?);
}
```

Behavior:
```
$ mika ask --verbose --format json ping
{"role":"assistant","content":"Here."}            # ← --verbose silently dropped
```

### Pattern: orthogonal `--verbose` (mika#829)

```rust
let metadata = if verbose {
    Some(MetadataEnvelope { session_id: Some(session_id.clone()) })
} else {
    None
};
// Both arms read `verbose` consistently — encoding differs, semantics don't
```

Behavior:
```
$ mika ask --verbose ping
Still here.

session_id: 7d2794f6-...

$ mika ask --verbose --format json ping
{"role":"assistant","content":"Still here.","metadata":{"session_id":"7d2794f6-..."}}

$ mika ask --format json ping
{"role":"assistant","content":"Present."}        # ← unchanged byte-for-byte from pre-#829
```

### Per-field gating (extending the envelope)

The `metadata` envelope is structured to support **per-field gating**, not blanket-gated by `--verbose`. Today `session_id` is `--verbose`-gated, but future fields may ship unconditional:

```rust
struct MetadataEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,    // --verbose-gated today
    // Future:
    // #[serde(skip_serializing_if = "Option::is_none")]
    // trace_id: Option<String>,    // could be unconditional for ops
}
```

The envelope is omitted only when *all* its fields are absent (per `Option::is_none` on the parent `metadata: Option<MetadataEnvelope>`). Consumers should treat individual keys as optional and not assume `metadata`'s presence implies `--verbose` was passed.

This shape avoids a future semantics revisit: when `trace_id` lands as unconditional, it goes into the envelope without changing the structural contract. The framing is *"`metadata` is metadata that may be gated per-field"*, not *"`metadata` is gated by `--verbose`."*

## Related

- mika#824 — original `--verbose` PR (text-mode only)
- mika#829 — orthogonality fix (this incident)
- mika-platform#54 — the slash-command consumer that grew a sqlite fallback because of mika#824's gap; will consume `metadata.session_id` in JSON mode after this PR deploys
- `docs/architecture/review-guide.md` § Orthogonality — the principle this guidance derives from
- `docs/solutions/best-practices/structural-check-replaces-human-discipline-2026-04-27.md` — sibling lesson from the same session: when human discipline keeps failing for a class of bug, a structural check replaces it. Same shape applied at review-time discipline: when "scope discipline" repeatedly produces orthogonality violations, the answer is to build orthogonality into the review checklist, not to harden discipline further.
