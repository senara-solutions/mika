---
title: KG subject extractor returns 0 entities due to LLM prose around JSON
date: 2026-05-01
category: runtime-errors
module: kg/subject_extractor
problem_type: runtime_error
component: tooling
symptoms:
  - "extraction_parse_failed_retry events in server.log (6600+ cumulative)"
  - "extraction_semantic_exhausted log-and-skip after retry"
  - "subject_extraction_complete shows entities=0, relationships=0 for whole batches"
  - "failed=0 in batch summaries masks actual parse failures (C2.3 log-and-skip)"
root_cause: logic_error
resolution_type: code_fix
severity: high
tags:
  - kg
  - subject-extractor
  - json-parsing
  - haiku
  - llm-output
  - brace-matching
  - prompt-reinforcement
---

# KG subject extractor returns 0 entities due to LLM prose around JSON

## Problem

KG subject-extraction batches frequently returned 0 entities and 0 relationships across all agents (mika-arch, mika-dev, mika-qa, mika). The `failed=0` in batch summaries masked the problem because the C2.3 log-and-skip path doesn't count toward `docs_failed`. Both `claude-haiku-4-5-20251001` and `openai/gpt-5-nano` were affected.

## Symptoms

- `extraction_parse_failed_retry` events: 6600+ cumulative across server.log
- `extraction_semantic_exhausted` events: 5764 (final fail after reinforcement retry)
- Error message: `"failed to parse extraction JSON: "` — empty body after the colon, suggesting the parser couldn't even start to lex
- mika-arch secondary corpora showed 33–66% pending docs that never extracted
- The same docs re-failed on every restart (non-transient parse failure)

## What Didn't Work

- **Prompt-only approach was insufficient.** The existing system prompt already said `"Return ONLY the JSON object, no markdown fencing, no explanation"` but haiku-class models emitted reasoning prose around the JSON regardless.
- **Markdown fence stripping (existing)** handled `` ```json ... ``` `` but NOT prose before or after the JSON object (e.g., `"Here is the extraction:\n\n{...}\n\nThe entities cover..."`).
- **Truncation was ruled out** as a root cause. Worst-case doc (~5,500 tokens) uses only 3.3% of haiku's 200K context window. The empty error body confirms the parser failed to lex prose, not that the LLM returned empty bytes.

## Solution

Three-layer defense-in-depth:

### 1. Brace-matching JSON extractor (`extract_first_json_object`)

A new module-level function that locates the first balanced `{…}` substring in text with string-literal/escape-aware depth tracking:

```rust
fn extract_first_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut start = None;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, &b) in bytes.iter().enumerate() {
        if escape_next { escape_next = false; continue; }
        if in_string {
            if b == b'\\' { escape_next = true; }
            else if b == b'"' { in_string = false; }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 { start = Some(i); }
                depth += 1;
            }
            b'}' => {
                if depth == 0 { continue; } // stray closing brace
                depth -= 1;
                if depth == 0 {
                    return start.map(|s| &text[s..=i]);
                }
            }
            _ => {}
        }
    }
    None
}
```

Key design decisions:
- **Brace-matching, not regex.** Handles nested objects/arrays correctly; regex on JSON is structurally fragile.
- **String-literal awareness.** Tracks `in_string` and `escape_next` states so braces inside JSON string values don't affect depth.
- **Stray `}` guard.** Leading `}` in prose before the JSON is ignored (depth never goes negative).
- **Zero allocation.** Returns `Option<&str>` borrowed from input.

### 2. Three-layer parse pipeline in `parse_extraction_json`

```rust
fn parse_extraction_json(&self, text: &str) -> Result<ExtractionOutput> {
    // Layer 1: Strip markdown code fences (existing behavior)
    let cleaned = /* strip ```json ... ``` */;

    // Layer 2: Direct serde parse (fast path — common case)
    if let Ok(output) = serde_json::from_str(cleaned) {
        return Ok(output);
    }

    // Layer 3: Brace-matching fallback (prose-tolerant, schema-strict)
    if let Some(json_substr) = extract_first_json_object(cleaned)
        && let Ok(output) = serde_json::from_str(json_substr)
    {
        warn!(event = "extraction_parse_slow_path", ...);
        return Ok(output);
    }

    // All failed → return original error for C2.2 retry
    Err(...)
}
```

Schema validation stays strict — only surrounding-prose tolerance is added (R4 from plan).

### 3. Prompt reinforcement

Added a `CRITICAL:` instruction at the top of the extraction prompt:

```
CRITICAL: Respond with a single JSON object only. Do NOT include any
explanatory prose, markdown headers, code fences, reasoning, or summary
text before or after the JSON. Your entire response must be parseable as
JSON from the first byte to the last byte.
```

Defense-in-depth: reduces prose-around-JSON emissions at the source while the parser tolerates them.

## Why This Works

The root cause was a mismatch between what the LLM produces and what the parser accepts. Haiku-class models (and gpt-5-nano) frequently emit reasoning prose around their JSON output — a known behavior class documented in sibling issue #768 (permission-policy). The existing parser required the response to be valid JSON from the first byte, so any leading prose caused immediate parse failure.

The brace-matching extractor recovers the JSON object from within arbitrary surrounding text. Since `serde_json::from_str` on the extracted substring still validates the full JSON schema, no data quality is sacrificed — the fix is purely about locating the JSON, not about accepting malformed JSON.

## Prevention

- **New `extraction_parse_slow_path` log event.** When the brace-matching fallback succeeds, operators can monitor LLM prompt-compliance regression without waiting for extraction quality to degrade.
- **10 unit tests** covering: prose prefix/suffix/both-sides, string literals containing braces, escaped quotes, leading stray `}`, no-JSON-present, unbalanced braces, and schema-strict validation rejection.
- **Escalation path documented.** If 24h post-deploy parse-failure rate stays > 5%, escalate to provider-native structured outputs (Option 2 from the original ticket — deferred per `engine-guards-vs-prompt-rules` precedent).

## Related Issues

- [mika#876](https://github.com/senara-solutions/mika/issues/876) — This fix
- [mika#768](https://github.com/senara-solutions/mika/issues/768) — Sibling failure class in permission-policy (haiku reasoning prose around JSON)
- [mika#766](https://github.com/senara-solutions/mika/issues/766) — Prompt truncation (ruled out as root cause here)
- [docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md](../architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md) — Informed Option 1 choice + Option 2 deferral
- Milestone [#19](https://github.com/senara-solutions/mika/milestone/19) — KG quality improvements
