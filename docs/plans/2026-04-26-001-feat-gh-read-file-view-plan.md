---
title: "feat(gh_read): add file_view op — read repo working-tree files by path"
type: feat
status: groomed
date: 2026-04-26
origin: senara-solutions/mika#817
depth: medium
architect_session: 83519e10-7970-4b4a-b0fc-272762b15e26
architect_model: anthropic/claude-opus-4-7
architect_verdict: GROOMED (post spec correction; see Architect grooming closure below)
---

## Architect grooming closure (2026-04-26)

Two-pass review on session `83519e10-7970-4b4a-b0fc-272762b15e26` (mika-arch on Opus 4.7 via `skill_overrides`).

**First-pass: `Disposition: ITERATE`** — 7 findings:
1. Path charset enforcement (load-bearing — URL-decoding attack surface)
2. Pin `--method GET` explicitly in argv
3. Push `--repo` flag into per-op arms of `build_gh_read_command`
4. Reframe repo allowlist deferral as "uniform across all five ops if introduced"
5. Add `blob_sha` to audit log (cost-free)
6. Document GitHub's >100 MiB → 403 → AuthFailed boundary
7. Audit `resource` shape: `<ref>:<path>` not `<repo>:<ref>:<path>`

All 7 applied (this commit + finding 1 in Unit 2; finding 2 in Unit 3 argv; finding 3 in Unit 3 refactor; finding 4 in Out-of-scope; finding 5 in Unit 5 audit; finding 6 in D4; finding 7 in Unit 5 resource).

**Second-pass: `Verdict: ESCALATE`** — six of seven first-pass findings cleanly RESOLVED. Seventh (repo allowlist) flagged as plan-vs-issue-spec divergence: the issue body had specified an allowlist that the plan deferred. Architect refused to ratify unilateral spec divergence per the issue-as-contract discipline.

**Resolution: issue body corrected to match plan reasoning.** The architect's substantive review of plan content is unchanged; the six-of-seven RESOLVED verdict applies to the corrected spec. **No third architect pass** — R11 holds, plan content unchanged, spec changed to match reviewed plan. Audit trail: this annotation, the issue body edit-notice comment on mika#817, and the architect session record at `83519e10-…`. Effective verdict on corrected spec: GROOMED.

---

# feat(gh_read): add file_view op

## Overview

Extend the `gh_read` builtin handler with a fifth read-only op `file_view` that fetches a single file's content from a GitHub repo by `(repo, ref, path)`. The op uses `gh api /repos/{owner}/{repo}/contents/{path}?ref={ref}` under the hood, base64-decodes the response, and returns content + resolved commit sha + size. Maintains the existing op-allowlist, structured-error, audit-log, and auth contract patterns established in PR #813. ~80 lines added inside `crates/mika-agent/src/skills/builtin_handlers.rs` (lines 977-1275 today). 3 new test scenarios on top of the 10 already covering `gh_read`.

## Problem Frame

`gh_read` today supports four ops — `issue_view`, `pr_view`, `pr_diff`, `issue_list` — covering GitHub PR/issue state but not arbitrary working-tree files. The `mika-arch` architect agent's review-guide §6 demands citation-grounded review ("cite or shut up"); without file-by-path read, citations decay into "trust the operator's paste of what the source says."

Empirical evidence:
- **2026-04-26 mika-arch session `cf3c33a9-…`** asked to review the new slash-command spec files at `mika-platform/.claude/commands/`. mika-arch tried `read_agent_file` (rejected — wrong tool surface), then `gh_read` against the wrong repo, then resorted to passive paste-comprehension when the operator inlined the source. Audit shows zero source-read attempts on her second turn even with source in her context window — once she has no active-read tool, even when source is present, she doesn't probe it.
- **2026-04-25 mika#814 dogfood:** mika-arch (on Kimi K2.5) GROOMED a plan that specified `with_list_parse_key + try_parsing(true)`. claude-pilot rejected that mid-implementation because `try_parsing(true)` would auto-parse all `MIKA_*` env vars globally as bool/i64/f64 (real risk). PR #816 shipped a custom serde Visitor instead. Architect missed a real architectural concern that source-read would have surfaced.

## Requirements Trace

- **R1.** Read-only op: `file_view(repo, ref, path)` returns `{content, ref, path, size_bytes}`. No write capability.
- **R2.** ref defaults to `main` if unspecified. Explicit branch/tag/sha supported.
- **R3.** Error variants extended: add `FileTooLarge` to the existing five (`NotFound`, `AuthFailed`, `RateLimited`, `NetworkError`, `MalformedRequest`). Fail-loud not silent-truncate — reading 80% of a file and citing line 1340 from the truncated 20% is worse than no read.
- **R4.** Audit on success only — consistent with existing four ops. No carve-out for the new op.
- **R5.** Auth via `ToolContext.github_token` reused unchanged. No new credential surface.
- **R6.** No new repo allowlist — match the existing four ops' behavior (any repo string accepted, validation only against shell-injection / flag-smuggling). If repo restriction is desired, that's a separate decision applied uniformly to all five ops.

## Scope Boundaries

**In scope:**
- New op `file_view` in `gh_read`'s allowlist + dispatch.
- New error variant `FileTooLarge` with size info.
- `gh api` subprocess invocation + base64 decode.
- 3 new test scenarios (happy path, FileTooLarge, malformed path).
- CLAUDE.md update reflecting five ops.

**NOT in scope:**
- Writing files. Read-only invariant preserved.
- Listing directory contents (would be a separate op like `dir_view` if needed).
- Repo allowlist (architect first-pass finding 4: this is a separate decision applied **uniformly to all five `gh_read` ops**, not file_view-specific. file_view does have higher sensitivity than PR/issue metadata — raw bytes including any accidentally-committed credentials vs curated JSON — but that argues for *uniform* allowlist on all five ops, not a per-op exception. Mika-cloud source is as sensitive as a file inside it. Defer entirely.)
- Configurable size threshold (const is fine; YAGNI).
- Caching (one read per call; if the same file is read twice in a session, that's two API calls; defer optimization until empirical pressure).

### Deferred to separate tasks
- Repo allowlist for all `gh_read` ops if desired (out of scope per R6).
- Larger-file-via-Git-Trees-API support if 1 MiB cap becomes restrictive.

### Non-goals
- Replicating `git show <ref>:<path>` semantics fully (we use GitHub's `contents` API, which is HTTP not git protocol; that's fine).
- Allowing arbitrary `gh api` paths (this op is *one specific endpoint with structured input/output*, not a `gh api` passthrough).

## Context & Research

### Relevant code

- **`crates/mika-agent/src/skills/builtin_handlers.rs:977-1275`** — the entire `gh_read` implementation: `GH_READ_ALLOWED_OPS` (line 980), `GhReadArgs` (982-988), `GhReadError` (991-1035), `classify_gh_error` (1037-1069), `validate_gh_read_input` (1074-1168), `build_gh_read_command` (1171-1218), `gh_read` async fn (1227-1275). All extension points live in this single file.
- **`crates/mika-agent/CLAUDE.md` § GitHub Read-Only Handler** — describes the contract; needs to be updated to list five ops and the `FileTooLarge` variant.

### Patterns this plan follows

- **Op allowlist enforcement at validation time** — `GH_READ_ALLOWED_OPS` is a `&[&str]`; add `"file_view"` and the existing rejection logic (lines 1087-1096) handles unknown-op rejection automatically.
- **Anti-flag-smuggling on string inputs** — `repo.starts_with('-')` and `target.starts_with('-')` checks (1117, 1147) reject argv injection. Apply the same pattern to `path` and `ref`.
- **Structured error variants with `to_json()`** — extend the `GhReadError` enum with one new variant + matching `to_json()` arm. Same `{"error": "<snake_case>", "message": "..."}` envelope.
- **`build_gh_read_command` returns argv `Vec<String>`** — file_view's command shape is `["api", "/repos/{owner}/{repo}/contents/{path}", "-q", ".", "--jq", ".content,.sha,.size,.encoding", "-X", "GET"]` or similar. Per-op argv is built in this helper.
- **`gh_read_invocation` audit log** — extend `resource` field to encode `<repo>:<ref>:<path>` for `file_view` (existing ops use `target` which is just the issue/PR number).
- **`ToolContext.github_token` injected as `GH_TOKEN` env var** — existing `gh_read` does this at line 1242-1244; no change needed.

### Institutional learnings

- `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — captures the architect's tool-surface limits; this plan addresses one of them.
- `mika/docs/solutions/workflow-issues/comment-event-fires-autonomous-dispatch-2026-04-25.md` — the mika-dev autodispatch path will fire on this issue's comment-with-plan, so the plan-on-branch must be the actual contract (not just the comment).
- `mika/CLAUDE.md` § GitHub Read-Only Handler — the existing four-op contract is the pattern to extend, not refactor.

## Key Technical Decisions

### D1. `gh api` subcommand vs. cloning the repo

The `gh` CLI exposes the GitHub REST API via `gh api <path>`. Equivalent for `repos/{owner}/{repo}/contents/{path}` returns:

```json
{
  "name": "...", "path": "...", "sha": "<file-blob-sha>",
  "size": <bytes>, "url": "...", "html_url": "...",
  "git_url": "...", "download_url": "...",
  "type": "file", "content": "<base64-encoded>",
  "encoding": "base64"
}
```

This is the simplest path: same `gh` CLI auth, same subprocess pattern, same error classification. Alternative (cloning the repo) would require disk space, network bandwidth, and clone-staleness handling — deployment coupling we explicitly rejected (per the peer analysis that motivated the ticket).

**Decision: use `gh api`.**

### D2. Default ref = `main`, not `HEAD`

`HEAD` would imply "the ref the operator has checked out locally," but `gh_read` doesn't have a local checkout — it queries GitHub. The natural default is the repo's default branch, which is `main` for all four corpus repos.

GitHub's `contents` endpoint accepts `?ref=<branch|tag|sha>`; if omitted, GitHub defaults to the repo's default branch. We could rely on that, but explicit `?ref=main` is clearer in audit logs and is what the architect would think of when asking "what's currently in main."

**Decision: default to `"main"` in argv when caller omits ref. Argv-explicit, not GitHub-implicit.**

### D3. Resolved sha returned alongside content

GitHub's API returns the file blob's sha (`response.sha`), which identifies the *file content version* but not the commit it lives in. For audit and "what version did the architect read" queries, we want the *commit* sha at the resolved ref.

Two paths:
1. Single API call to `contents/{path}?ref={ref}` returns the *blob* sha (file-content sha). Cheap.
2. Two API calls: first to `contents/{path}?ref={ref}` for content, then to `repos/{owner}/{repo}/branches/{ref}` (or `commits/{ref}`) for the commit sha. Expensive.

For audit, the *blob* sha is sufficient — it identifies the exact file content read, which is what "what version" means at file granularity. If we need commit-level provenance, that's a future extension.

**Decision: return the blob sha. Field is `ref` in the response (named to match input), value is `<blob-sha>`. Document this in CLAUDE.md.**

### D4. FileTooLarge — what threshold, what behavior

GitHub's `contents` API caps response files at 1 MiB. Files > 1 MiB return a 200 response with an empty `content` field and a non-zero `size` field. Programmatic detection: `size > 1024*1024 && content.is_empty()`.

Two thresholds we could enforce:
- **Soft (rely on GitHub's 1 MiB cap):** check `size > 1_000_000 && content.is_empty()` in our handler, classify as `FileTooLarge { size_bytes }`. No client-side cap.
- **Hard (our own cap, smaller):** reject before invoking gh if we somehow knew the size; we don't, so this isn't really an option.

Mika could also classify `size > 1 MiB AND content present` (impossible per GitHub's behavior, but defensive).

**Decision: implement soft threshold. Detect GitHub's empty-content + non-zero-size signal and classify as `FileTooLarge { size_bytes: <gh's reported size>, max_bytes: 1_048_576 }`. The const `FILE_VIEW_MAX_BYTES: u64 = 1_048_576` documents the cap. Don't make it configurable.**

**Boundary clarification (architect first-pass finding 6):** GitHub's `contents` endpoint behavior changes at scale:
- Files ≤ 1 MiB: 200 response with non-empty `content` (decoded by us).
- Files 1–100 MiB: 200 response with empty `content` + non-zero `size` → classified as `FileTooLarge` per the soft threshold above.
- Files > 100 MiB: 403 response (GitHub's limit on the `contents` endpoint) → falls through to existing `classify_gh_error()` and is returned as `AuthFailed` per the existing 403 handling at lines 1052-1063.

The 100 MiB cliff is a pre-existing GitHub quirk; we don't add a fourth-tier handling for it. Document this in `crates/mika-agent/CLAUDE.md` so operators understand "FileTooLarge disappeared at very-large scale" isn't a regression — it's the 403→AuthFailed path firing.

### D5. Path validation

`path` is a repo-root-relative string. Validation:
- **Must not be empty.**
- **Must not start with `-`** (anti-flag-smuggling, mirrors existing `repo`/`target` checks).
- **Must not start with `/`** — repo-root-relative, not absolute.
- **Must not contain `..`** — defense against weird API behavior, even though GitHub's API would return 404 anyway.
- **Must match charset `[A-Za-z0-9._/\-]`** — explicit charset enforcement (architect first-pass finding 1).
- **Length cap** — match the existing 10K input control limit per `crates/mika-agent/src/tools/mod.rs` `MAX_INPUT_LEN`.

**Why charset enforcement is load-bearing (architect first-pass finding 1):** GitHub's API URL-decodes path segments server-side. A path like `foo%2F..%2Fbaz` passes the literal-`..` substring check (the encoded form is `%2E%2E`) but decodes to `foo/../baz` server-side after our check. The existing four ops never accept operator-controlled URL path components — they construct `/repos/{owner}/{repo}/issues/{number}` server-side from `gh issue view`-style argv. file_view is the first op with operator-controlled URL path, so the URL-shape construction defense alone is insufficient. The charset whitelist `[A-Za-z0-9._/\-]` rejects `%` (no URL-encoded payloads in path), all whitespace, all control characters, all shell metacharacters. Reject characters outside this set with a `MalformedRequest` carrying the offending path verbatim (so operator can see what was rejected and why).

These checks live in `validate_gh_read_input`, gated on `op == "file_view"`. They reuse the existing `MalformedRequest` error variant, no new variant for path-shape errors.

### D6. ref validation

Branch/tag/sha names. Validation:
- **Must not start with `-`** (anti-flag-smuggling).
- **Length cap** — refs are typically <100 chars; 256 is generous.
- **No further character class enforcement** — Git allows a wide character set in refs and we don't want to be the gatekeeper for what Git accepts.

If ref is invalid, GitHub returns 404; we classify as `NotFound`. No new error variant.

### D7. No new tool name; same `gh_read` builtin

The new op is dispatched within `gh_read`'s existing function, not a new top-level tool. Skill `tools.json` files that already declare `gh_read` don't need updating — `mika-arch-groom-ticket` and `mika-arch-second-review` get the new op for free. The `mika-arch` agent's `[tools]` config also doesn't change.

## Open Questions

### Resolved during planning

- gh subcommand to use → D1 (`gh api`)
- Default ref → D2 (`"main"`, argv-explicit)
- Sha granularity in response → D3 (blob sha; document in CLAUDE.md)
- FileTooLarge threshold → D4 (1 MiB matching GitHub's cap)
- Path validation surface → D5 (empty/flag-smuggling/absolute/traversal/length)
- ref validation surface → D6 (flag-smuggling/length only)
- New top-level tool vs new op → D7 (new op, no top-level)

### Deferred to implementation

- Exact JSON shape of the response — should we return raw GitHub fields or normalize to `{content, ref, path, size_bytes}`? Lean toward normalize.
- Whether to base64-decode in our code or pass through the encoded string — lean toward decode-in-code (simpler downstream; UTF-8 only).
- Whether non-UTF-8 file content should hit `FileTooLarge` or a new `BinaryFile` variant — lean toward graceful: detect non-UTF-8 in decode step and return `MalformedRequest { reason: "file content is not valid UTF-8" }` for now. Binary file support is a future extension.

## Output Structure

### New / modified files

```
mika/
└── crates/mika-agent/
    ├── src/skills/builtin_handlers.rs                # MODIFY — add file_view op
    └── ../CLAUDE.md                                   # MODIFY — update ops list to 5
```

No new source files. No new abstraction. Single-file extension at the existing site.

### Skill prompt updates

None. mika-arch's `tools.json` declares `gh_read` as a builtin; the new op is exposed transparently. The skill prompts can mention `file_view` in their next round of prompt-tightening (separate concern; not blocked by this PR).

## High-Level Technical Design

```
LLM tool call:
  gh_read({op: "file_view", repo: "senara-solutions/mika", ref: "main", path: "crates/mika-agent/src/skills/builtin_handlers.rs"})
       │
       ▼
validate_gh_read_input
       │ ─── op in allowlist (5 entries now) ✓
       │ ─── repo non-empty + no flag-smuggle ✓
       │ ─── (op == file_view) path non-empty + no flag-smuggle + no leading-/ + no .. + len ≤ 10K ✓
       │ ─── (op == file_view) ref no flag-smuggle + len ≤ 256 ✓
       ▼
build_gh_read_command
       │ ─── for file_view: ["api", "/repos/senara-solutions/mika/contents/crates/.../builtin_handlers.rs?ref=main", ...]
       ▼
spawn_and_collect (gh subprocess, with GH_TOKEN injected)
       │
       ▼
Parse JSON response
       │ ─── if size > 1 MiB && content empty → FileTooLarge
       │ ─── if content invalid UTF-8 → MalformedRequest
       │ ─── else → base64 decode, return {content, ref: <blob-sha>, path, size_bytes}
       ▼
tracing::info!(event="gh_read_invocation", op="file_view", resource="<repo>:<ref>:<path>", ...)
       │
       ▼
ToolOutput::success(<json>)
```

The flow mirrors the existing `gh_read` exactly; only the input validation and command-building branches diverge per-op.

## Implementation Units

- [ ] **Unit 1: Extend allowlist + types**

**Goal:** Add `"file_view"` to `GH_READ_ALLOWED_OPS`, `FileTooLarge { size_bytes, max_bytes }` to `GhReadError`, with matching `to_json()` arm. Add the `FILE_VIEW_MAX_BYTES` const.

**Files:**
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs:980` (add `"file_view"` to const)
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs:991-1034` (add variant + arm)
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs:977` (add `FILE_VIEW_MAX_BYTES`)

**Verification:** `cargo check -p mika-agent` passes. No test changes needed yet.

- [ ] **Unit 2: Validate file_view inputs**

**Goal:** Extend `validate_gh_read_input` to handle `op == "file_view"` cases — `path` and `ref` parameters with their validation rules per D5/D6.

**Files:**
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs:1074-1168` (add file_view branch)

**Approach:** Add fields `path: Option<String>` and `r#ref: Option<String>` to `GhReadArgs`. Extend the validation function with branch on `op == "file_view"`:
- `path` is required, non-empty, no flag-smuggle, no leading `/`, no `..`, len ≤ 10K.
- `ref` is optional; if present, no flag-smuggle, len ≤ 256.
- For `file_view`, `target` is NOT required (the existing branch at line 1133 only requires it for issue_view/pr_view/pr_diff — already correct).

**Tests:** Inline `#[cfg(test)] mod tests` near line 3953:
- `test_validate_gh_read_input_valid_file_view` (happy path)
- `test_validate_gh_read_input_file_view_missing_path`
- `test_validate_gh_read_input_file_view_path_starts_with_dash`
- `test_validate_gh_read_input_file_view_path_starts_with_slash`
- `test_validate_gh_read_input_file_view_path_with_traversal`
- **`test_validate_gh_read_input_file_view_path_charset_rejection`** (architect first-pass finding 1) — paths containing `%`, whitespace, control chars, or shell metacharacters → `MalformedRequest`. Test cases include URL-encoded traversal (`foo%2F..%2Fbaz`), spaces, semicolons, and a representative non-ASCII char.
- `test_validate_gh_read_input_file_view_ref_starts_with_dash`
- `test_validate_gh_read_input_file_view_ref_too_long`

- [ ] **Unit 3: Build the gh argv for file_view**

**Goal:** Extend `build_gh_read_command` with a `"file_view"` arm that produces the right `gh api` argv.

**Files:**
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs:1171-1218`

**Approach:** Argv shape:
```
["api", "/repos/<repo>/contents/<urlencoded-path>?ref=<ref>", "--method", "GET", "-H", "Accept: application/vnd.github+json"]
```

**Pin `--method GET` explicitly (architect first-pass finding 2):** `gh api` defaults to GET but defaults can change. Explicit `--method GET` matches the structured-construction discipline of the existing four ops (each uses `gh issue view`/`gh pr view` etc. — verb-explicit subcommands, never relying on defaults). Bonus: the no-write invariant becomes a static argv assertion (`argv.contains("--method GET")`) instead of a behavioral test, which is cheaper and more reliable.

**Refactor `--repo` flag handling (architect first-pass finding 3):** Today, line 1238 appends `--repo` unconditionally after `build_gh_read_command` returns. The naive "skip for file_view" approach pushes a per-op exception into the dispatch layer at line 1227. Cleaner shape: **push `--repo` into the per-op arms of `build_gh_read_command`**. Each existing arm (issue_view, pr_view, pr_diff, issue_list) appends `["--repo", repo]` at the end of its argv vec; the file_view arm does not. Adds four lines of repetition; removes the per-op exception from the dispatch layer. The dispatch site at line 1238 then drops the `--repo` append entirely (it's now per-op responsibility). Cite: Orthogonality (review-guide.md) — minimal propagation of per-op exceptions out of the structured builder.

**Tests:** Inline:
- `test_build_gh_read_command_file_view` (verify argv shape, encoded path, ref query param, `--method GET` present, no `--repo` flag).
- `test_build_gh_read_command_existing_ops_have_repo` (regression — confirm the four existing ops still emit `--repo <repo>` after the refactor moves the append into their arms).
- Verify file_view argv does NOT contain `--repo`, `--method PUT`, `--method POST`, or `--method DELETE`.

- [ ] **Unit 4: Parse response + handle file-size + base64 decode**

**Goal:** After the subprocess returns, parse the JSON response, detect FileTooLarge (size > cap && content empty), base64-decode, return normalized response.

**Files:**
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs:1227-1275` (`gh_read` async fn)

**Approach:** Add a post-processing branch on `op == "file_view"` after the existing classify_gh_error path:
1. If `is_err`, classify as before — file_view's errors go through the same classification path.
2. If success, parse the output JSON. Extract `size`, `content`, `sha`, `encoding`.
3. If `size > FILE_VIEW_MAX_BYTES && content.is_empty()`, return `FileTooLarge { size_bytes: size, max_bytes: FILE_VIEW_MAX_BYTES }`.
4. Else if `encoding != "base64"`, return `MalformedRequest { reason: "unexpected encoding: <encoding>" }`.
5. Else base64-decode. If decode fails or result is non-UTF-8, return `MalformedRequest { reason: "file content is not valid UTF-8" }`.
6. Build normalized response: `{"content": "<decoded>", "ref": "<blob-sha>", "path": "<echoed-path>", "size_bytes": <size>}`.

**Tests:** Use mocked subprocess output (mirroring existing test patterns):
- `test_gh_read_file_view_happy_path` — content + sha + size correctly parsed and returned.
- `test_gh_read_file_view_file_too_large` — synthetic response with empty content + size > 1 MiB → `FileTooLarge`.
- `test_gh_read_file_view_non_utf8_content` — synthetic response with non-UTF-8 base64 → `MalformedRequest`.

- [ ] **Unit 5: Audit log + CLAUDE.md update**

**Goal:** Extend the audit log at lines 1259-1267 for file_view. Existing ops emit `op`, `resource`, `repo`, `latency_ms`, `status`. For file_view the `resource` field should be `<ref>:<path>` and a new `blob_sha` field carries the resolved file-content sha (cost-free — already in the response body).

**Files:**
- Modify: `crates/mika-agent/src/skills/builtin_handlers.rs:1259-1267`
- Modify: `crates/mika-agent/CLAUDE.md` § GitHub Read-Only Handler

**Approach:**
- Extract a helper `audit_resource(args: &GhReadArgs) -> String` that returns `target` for issue/pr ops and `format!("{}:{}", ref, path)` for file_view. **Don't compose `<repo>:<ref>:<path>` (architect first-pass finding 7)** — the existing audit log already has a separate `repo` field, so composing repo into resource creates redundancy. Minimal divergence from the existing audit shape: `repo` stays in its own field; `resource` carries only the operation-specific identifier.
- Add a new `blob_sha` field to the audit log line for file_view only (None / omitted for the four existing ops). Source: parsed from the `gh api` response body during the file_view post-processing in Unit 4. **Cost-free addition (architect first-pass finding 5)** — the sha is already parsed and returned to the caller; logging it has no extra API cost. Gives audit visibility for "what version did the architect read" without the second commit-sha API call from D3.

**CLAUDE.md updates:**
- "Four allowed ops" → "Five allowed ops"
- Add `file_view` row with the response shape and FileTooLarge behavior.
- Note `FILE_VIEW_MAX_BYTES` const and the > 100 MiB → 403 → AuthFailed boundary clarification from D4.
- Audit log section: document the `blob_sha` field (file_view only) and the `<ref>:<path>` resource shape.

**Tests:** No new tests — audit-log emission is verified by the existing test pattern.

## Acceptance criteria

- [ ] All 17 `gh_read` test scenarios pass (`cargo test -p mika-agent gh_read`) — 10 existing + 3 new content-handling for file_view (happy/too-large/non-utf8) + 4 new validation for file_view (charset rejection, ref-starts-with-dash, ref-too-long, build-command argv assertions).
- [ ] `cargo clippy --all-targets` clean.
- [ ] `cargo fmt` clean.
- [ ] `gh_read` registered in builtin handlers exposes 5 ops. `validate_gh_read_input` rejects non-allowlisted ops with `MalformedRequest`.
- [ ] `mika-arch` can call `file_view(repo="senara-solutions/mika-platform", path=".claude/commands/mika-ask-arch.md")` from a `mika ask --agent mika-arch` test invocation post-deploy and get the file content.
- [ ] Audit log line for a successful `file_view` includes `op="file_view"`, `resource="<repo>:<ref>:<path>"`, and the existing latency/status/repo fields.
- [ ] `crates/mika-agent/CLAUDE.md` § GitHub Read-Only Handler reflects 5 ops + the file-size cap.

## Validation post-deploy

After PR merges + `make deploy`:
1. `mika ask --agent mika-arch "<brief asking her to fetch and quote a specific file via gh_read.file_view>"`
2. Verify the response actually quotes file content (not paraphrase).
3. `sqlite3 ~/.mika/data/mika.db "SELECT input, output FROM tool_calls WHERE tool_name='gh_read' AND input LIKE '%file_view%' ORDER BY created_at DESC LIMIT 3"` — confirm the new op fires.
4. Re-run the architect review on `mika-platform/.claude/commands/mika-ask-arch.md` (the same source files mika-arch couldn't read on session `cf3c33a9-…`). Compare findings to the paste-comprehension review; expected: more concrete, with line-level citations.

## Related

- senara-solutions/mika#817 — this ticket.
- senara-solutions/mika#811 / PR #813 — introduced gh_read with the existing four ops.
- `mika/docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — flagged the tool-surface limit.
- `mika/docs/solutions/workflow-issues/comment-event-fires-autonomous-dispatch-2026-04-25.md` — discusses the autonomous dispatch path that fires from this issue's GROOMED-plan comment.
- `mika/docs/architecture/review-guide.md` §6 — the citation-or-silence operating discipline this op restores.
