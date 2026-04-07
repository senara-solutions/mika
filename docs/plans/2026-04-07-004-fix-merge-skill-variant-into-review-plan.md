---
title: "fix: Merge write_skill_variant into review_skill (single atomic tool)"
type: fix
status: active
date: 2026-04-07
issue: senara-solutions/mika#477
---

# fix: Merge `write_skill_variant` into `review_skill`

## Context

The skill-review workflow is broken end-to-end and the failure mode has been observed in production. Two turn audits on 2026-04-07 (`mika-qa` session `68ce6546-5b06-4fa9-9e55-8a335606f179` and `mika-dev` session `93bb81be-...`) show the same pattern:

1. `review_skill` succeeds and returns `next_action: "Call write_skill_variant with skill_name and content. Do not pass a path …"` (`crates/mika-agent/src/skills/builtin_handlers.rs:1080`).
2. The agent tries to comply, but `write_skill_variant` is **not declared** in the skill-review skill's `tools.json` and is not part of `default_tools()` for non-orchestrator agents in any meaningful way that the runtime registry surfaces to the LLM. Both agents reported `"write_skill_variant … is a built-in function that I do not have direct access to"`.
3. They fall back to `write_agent_file`, which is sandbox-rejected (`"Path resolves outside the base directory"` and `"Only orchestrator agents can access other agents"`).
4. Net result: no variant is ever written. `~/.mika/agents/mika-dev/skills/self-dev/` has no `generated/` directory; `audit_events` has zero recent skill-write entries; `tool_calls` has zero `write_skill_variant` invocations in April.

`write_skill_variant` is only ever called as the terminal step of `review_skill`. The Explore agent confirmed there are **no independent callers** in the codebase or tests outside that response string. Keeping it as a separate tool is a YAGNI violation and leaks an implementation detail (two-step persist) into the LLM's tool surface — exactly what's failing.

**Outcome:** `review_skill` becomes a single atomic tool that analyses *and* writes the variant. `write_skill_variant` is deleted entirely. The skill-review system prompt no longer instructs a second tool call. Any agent that can call `review_skill` automatically gets persistence — no extra registration, no separate permission gate, no surface area for the LLM to misuse.

## Acceptance Criteria

- [x] `review_skill` accepts an optional `content: String` (≤200 KB). When `Some`, it writes the variant in the same call.
- [x] When `content = None`, `review_skill` behaves exactly as today (analysis-only) — no behaviour change for existing callers.
- [x] When `content = Some(...)` and `dry_run = true`, the response reports the resolved path and bytes that *would* be written but does not touch disk.
- [x] When `content = Some(...)` and `dry_run = false`, the variant is written under `skills/<name>/generated/<provider>/<sanitized_model>/system_prompt.md`, the skills registry dirty flag is set, and the response includes `written_path`, `content_bytes`, `source_bytes`, `linked`, and any `warning`.
- [x] All existing safety guards from `write_skill_variant` are preserved: skill_name traversal rejection, 200 KB content cap, source-size truncation guard (≥50% of source), overwrite guard (requires `force = true`), linked-skill warning, no path input.
- [x] `write_skill_variant` is deleted: removed from `KNOWN_BUILTINS`, `execute()` dispatch, every test, and every reference in skill prompts and templates.
- [x] The `next_action` field is **removed** from `review_skill`'s response (no leaky instruction string).
- [x] `templates/skills/skill-review/system_prompt.md` is rewritten to describe the one-call workflow and to never mention `write_skill_variant` or `write_agent_file` for variant writes.
- [x] `cargo build`, `cargo test -p mika-agent skills::builtin_handlers`, and `cargo clippy --all-targets -- -D warnings` all pass.
- [x] After `make deploy`, asking `mika-dev` (DeepSeek v3.2) to "review skill build-mika and write the variant" results in a single `review_skill` tool call (visible in `tool_calls`) and the file `~/.mika/agents/mika-dev/skills/build-mika/generated/deepseek/deepseek-v3-2/system_prompt.md` exists on disk.
- [x] Same end-to-end check works for `mika-qa` and for the linked `self-dev` skill (write lands at the source path via symlink).

## Critical Files

| File | Change |
|---|---|
| `crates/mika-agent/src/skills/builtin_handlers.rs` (lines 907–1250) | Extend `review_skill` schema with `content`; extract write logic into a private `persist_variant` helper; delete `write_skill_variant` (lines 1108–1250); remove from `KNOWN_BUILTINS` (line ~44) and from the `execute()` dispatch (line ~77). |
| `crates/mika-agent/src/skills/builtin_handlers.rs` (test module, lines ~2403–2596) | Migrate every `test_write_skill_variant_*` to drive `review_skill` with a `content` argument. Keep all `test_review_skill_*` cases. Add a regression test that analysis-only response contains no `next_action` field. Add a round-trip test (inspect → persist). |
| `crates/mika-agent/templates/skills/skill-review/system_prompt.md` | Rewrite to describe a single-call workflow. Delete every reference to `write_skill_variant` and to `write_agent_file` for variant persistence. Keep the `dry_run` note for previewing the path. |
| `crates/mika-agent/templates/skills/skill-review/tools.json` | Verify it lists only `review_skill` (already true per Explore findings — no edit expected, but confirm). |

## Implementation

### 1. Extend `review_skill` input schema

`builtin_handlers.rs:917-944` already validates `skill_name`, `dry_run`, and `force`. Add:

```rust
let content = match input.get("content").and_then(|v| v.as_str()) {
    Some(s) if !s.is_empty() => Some(s),
    Some(_) => return ToolOutput::error("'content' must be non-empty when provided."),
    None => None,
};
if let Some(c) = content {
    if c.len() > crate::tools::MAX_PAYLOAD_BYTES {
        return ToolOutput::error(format!(
            "'content' exceeds maximum payload size of {} bytes.",
            crate::tools::MAX_PAYLOAD_BYTES
        ));
    }
}
```

Thread `content` through to `review_skill_single` (new parameter). Batch mode (`skill_name == "*"`) ignores `content` — return a clear error if both are provided, since batch + write makes no sense ("Pass `skill_name` for a specific skill when supplying `content`.").

### 2. Refactor existing-variant short-circuit

The current early return at `builtin_handlers.rs:1036-1046` returns "skipped" when an existing variant exists and `!force`. With the merged tool, the semantics become:

- `content = None`, variant exists, `!force` → keep current behaviour: return analysis fields including `existing_variant` so the agent can read it before drafting. (Today this returns the "skipped" stub; **change** it to return the full analysis with `existing_variant: Some(...)` so a single inspect call gives the agent everything it needs to decide whether to overwrite. The current "skipped" stub is a UX wart that disappears with the merge.)
- `content = Some(...)`, variant exists, `!force` → error: `"Variant already exists at '<path>'. Re-call with force=true to overwrite."` (matches current `write_skill_variant` overwrite guard at line 1207).
- `content = Some(...)`, `force = true` (or no existing variant) → write.

### 3. Inline the write path as `persist_variant`

Move `builtin_handlers.rs:1147-1248` (everything after the input validation in `write_skill_variant`) into a private helper:

```rust
async fn persist_variant(
    skill_dir: &Path,
    skill_name: &str,
    content: &str,
    canonical_provider: &str,
    sanitized_model: &str,
    linked: bool,
    force: bool,
    dry_run: bool,
    skills_dirty: &AtomicBool,
) -> Result<serde_json::Value, ToolOutput> { ... }
```

Reuse verbatim:
- Source-size lookup and 50% truncation guard (lines 1170–1193).
- Path computation: `skill_dir.join("generated").join(provider).join(model).join("system_prompt.md")` (lines 1200–1204).
- Overwrite guard (lines 1207–1212).
- `create_dir_all` + `fs::write` (lines 1215–1226).
- `skills_dirty.store(true, Release)` (lines 1231–1232).
- Result struct shape (`written_path`, `provider`, `model`, `content_bytes`, `source_bytes`, `linked`, `warning`).

For `dry_run = true` with `content = Some(...)`: skip the `create_dir_all`/`write`/`skills_dirty` calls but compute the full result and add `"dry_run": true, "would_write": true`.

`review_skill_single` calls `persist_variant` only when `content.is_some()`. The provider/model and linked-skill detection (already computed at lines 947–948 and 994–1004) are passed in to avoid double-resolution.

### 4. Merge the response

Drop `next_action` (line 1080) entirely. The merged response shape:

```jsonc
{
  // analysis fields (always present for non-batch single-skill mode)
  "skill_name": "...",
  "root_prompt": "...",                  // truncated to MAX_PROMPT_IN_RESPONSE
  "tools_json": "...",
  "runtime_provider": "...",
  "runtime_model": "...",
  "existing_variant": "..." | null,
  "linked": true,
  "dry_run": false,
  "warning": null,                       // or linked-skill warning text

  // write fields (only present when content was supplied)
  "written": true,                       // false when dry_run=true
  "written_path": "/.../generated/.../system_prompt.md",
  "content_bytes": 1234,
  "source_bytes": 2345
}
```

### 5. Delete `write_skill_variant` entirely

- Remove the handler at `builtin_handlers.rs:1108-1250`.
- Remove the entry from `KNOWN_BUILTINS` (line ~44).
- Remove the dispatch arm in `execute()` (line ~77).
- Remove `MIN_VARIANT_RATIO` if it has no other callers (move it as a private const inside `persist_variant` if `review_skill` is the only consumer).
- Compiler will surface every other reference; delete or redirect each one.

### 6. Update the skill-review system prompt

Rewrite `crates/mika-agent/templates/skills/skill-review/system_prompt.md` to teach the one-call workflow:

> **Reviewing a skill prompt for your runtime model**
>
> 1. Call `review_skill { skill_name: "<name>" }` with no `content` to inspect the current `system_prompt.md`, declared tools, runtime provider/model, and any existing variant.
> 2. Draft a variant of the prompt tuned for the runtime model the response reports.
> 3. Call `review_skill { skill_name: "<name>", content: "<your full draft>" }` to persist it. The destination path is computed from the runtime provider/model — do not pass a path. If a variant already exists, re-call with `force: true` to overwrite. Use `dry_run: true` first if you want to preview the destination path.
>
> Do not call `write_skill_variant` (it no longer exists). Do not call `write_agent_file` to persist a variant — `review_skill` is the only correct tool.

Delete every other mention of `write_skill_variant` or of `write_agent_file` as a variant-write target in this file.

### 7. Tests

In `builtin_handlers.rs` test module:

- **Keep:** `test_review_skill_*` cases (existing analysis paths), with one update — the existing-variant case must now return analysis fields with `existing_variant: Some(...)` instead of the legacy `"skipped": true` stub.
- **Migrate:** every `test_write_skill_variant_*` (lines ~2403–2596) to drive `review_skill` with `content`. Cover: runtime model derivation, OpenRouter canonicalisation, path traversal rejection, linked skill warning, no-overwrite guard without `force`, dirty flag marking, truncation rejection (<50% of source), 200 KB cap, `dry_run = true` does not touch disk.
- **Add:** regression — `review_skill` with `content = None` returns a JSON object that does **not** contain the key `next_action`.
- **Add:** round-trip — call `review_skill` with `content = None`, capture `runtime_model`, then call again with `content = "<expanded prompt>"`, assert `written_path` ends with the expected `generated/<provider>/<sanitized_model>/system_prompt.md`.

### 8. Reuse, do not duplicate

- `resolve_canonical_provider_model` (`builtin_handlers.rs:947`)
- `sanitize_model_dir_name` (called at line 949 / 1198)
- `warn_linked_skill_write` (`builtin_handlers.rs:1258`)
- `crate::tools::MAX_PAYLOAD_BYTES` (already imported)
- `ctx.skills_dirty` atomic on `ToolContext`

Do not introduce new helpers beyond `persist_variant`.

## System-Wide Impact

- **Interaction graph:** `review_skill` is invoked by skill-review skill (template) and by agents directly. After the merge, the only change for callers is the optional `content` parameter — analysis-only callers see no behaviour change except the loss of the `next_action` string. Skills marketplace (`mika-skills/skill-review/`) ships its own copy of `system_prompt.md` and `tools.json` downstream of the core template; it picks up the new prompt on the next `make deploy` cycle.
- **Error propagation:** All current error paths in `write_skill_variant` are preserved by `persist_variant`. The new "content provided in batch mode" error is the only addition.
- **State lifecycle risks:** None new. `persist_variant` performs the same `create_dir_all` + `write` + `skills_dirty.store(true)` sequence atomically per call. No partial state worse than today.
- **API surface parity:** This is a breaking change to the built-in tool surface (one fewer tool, one new optional parameter). Per the repo's pre-1.0 versioning convention, this ships as a minor/patch release with the migration steps documented in the PR body. No external API consumers — all callers are in-tree.
- **Integration test scenarios:** (1) inspect-only call, (2) inspect → persist round-trip, (3) persist with existing variant + `force=false` → error, (4) persist with `force=true` → overwrite, (5) persist on linked skill → warning + write to source dir.

## Verification

1. **Build & lint:**
   ```bash
   cd mika
   cargo build -p mika-agent
   cargo test -p mika-agent skills::builtin_handlers
   cargo clippy -p mika-agent --all-targets -- -D warnings
   cargo fmt --check
   ```

2. **Symbol absence:**
   ```bash
   ! grep -RIn '\bwrite_skill_variant\b' crates/ templates/ docs/
   ```
   Should produce no matches outside the migration note in the PR body.

3. **End-to-end (live agent):**
   ```bash
   make deploy
   mika ask --agent mika-dev "use skill-review to review skill build-mika and write the variant"
   ls ~/.mika/agents/mika-dev/skills/build-mika/generated/deepseek/deepseek-v3-2/system_prompt.md
   ```
   Expect: a single `review_skill` tool call (verify with `sqlite3 ~/.mika/data/mika.db "SELECT tool_name, success FROM tool_calls ORDER BY created_at DESC LIMIT 5"`), success response with `written_path`, file present on disk.

4. **Linked skill regression:**
   ```bash
   mika ask --agent mika-dev "use skill-review to review skill self-dev and write the variant"
   ```
   Expect: the response includes a `linked: true` warning and the file is written through the symlink to `mika-skills/self-dev/generated/.../system_prompt.md`.

5. **mika-qa parity:**
   ```bash
   mika ask --agent mika-qa "use skill-review to review skill build-mika and write the variant"
   ```
   Expect: the previous `"write_skill_variant … I do not have direct access to"` failure is gone; a single successful `review_skill` call writes the variant.

6. **Audit trail:**
   ```bash
   sqlite3 ~/.mika/data/mika.db \
     "SELECT tool_name, success, substr(input,1,80) FROM tool_calls WHERE tool_name='review_skill' ORDER BY created_at DESC LIMIT 5"
   ```
   Each persist call should be `success=1` with `content` visible in the input preview.

## Out of scope / follow-ups

- The mika-skills marketplace copy of `skill-review/system_prompt.md` (if it diverges from the core template) needs the same prompt rewrite. Track as a separate issue if they have already drifted.
- Audit other built-ins (`update_core_memory`, `write_agent_file`) for the same two-step persist anti-pattern. Not in this PR.

## Sources

- Origin issue: senara-solutions/mika#477
- Turn audit: mika-qa session `68ce6546-5b06-4fa9-9e55-8a335606f179` (2026-04-07 ~15:09 UTC)
- Turn audit: mika-dev session `93bb81be…` trace `9aafe780d3ca4fd4923bfaa928a2dbde` (2026-04-07 ~13:14 UTC)
- Current `review_skill` handler: `crates/mika-agent/src/skills/builtin_handlers.rs:917-1084`
- Current `write_skill_variant` handler: `crates/mika-agent/src/skills/builtin_handlers.rs:1108-1250`
- skill-review template: `crates/mika-agent/templates/skills/skill-review/system_prompt.md`
- Related prior plan: `docs/plans/2026-04-07-002-fix-harden-write-skill-variant-plan.md` (hardening that this PR supersedes by deletion)
