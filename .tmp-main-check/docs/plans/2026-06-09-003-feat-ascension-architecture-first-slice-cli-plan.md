---
title: "feat: Mika ascension architecture — first slice (CLI dual-mode R1)"
status: active
created: 2026-06-09
type: feat
origin: docs/brainstorms/2026-06-09-mika-ascension-architecture-requirements.md
---

# feat: Mika ascension architecture — first slice (CLI dual-mode R1)

## Summary

Ship **R1 only** from the ascension architecture brainstorm: `mika` CLI gains a remote mode that proxies conversation to a cloud-hosted Mika agent via the existing gateway A2A endpoint. Local mode (in-process agent loop) remains the default.

This is the minimum viable daily-use unblock: Vincent can talk to cloud Mika Prime from his desktop CLI without bringing up the full bundle-transfer pipeline.

## Problem frame

The brainstorm (see origin: `docs/brainstorms/2026-06-09-mika-ascension-architecture-requirements.md`) scopes ten requirements (R1–R10) covering CLI dual-mode, signed-bundle export/import, granular slice selection, identity, transfer endpoint, chain-forward-compat shape, and Phase-2 migration targets. Trying to ship all ten at once is months of work and produces a coordinated change that's hard to validate piecemeal.

The brainstorm specifically calls out R1 (CLI dual-mode) + R5 (gateway transfer endpoint) as candidates for a first slice. Scope analysis at plan time reveals R5 has a **dependency inversion on R2** (signed-bundle export/import producer): a transfer endpoint with no clients that can produce bundles is a surface with no callers. This plan therefore narrows the first slice to **R1 only**. R5 will ship in slice 2 alongside R2 (bundle producer) so the endpoint and its callers land coherently.

## Scope

**In scope (this plan, this PR):**

- `mika ask` and the chat TUI entry gain a remote mode triggered by `--remote <url>` flag or `MIKA_REMOTE_AGENT_URL` env. Flag wins over env; absence of both means local mode (current behavior, default).
- Remote mode dispatches to a cloud Mika agent via `mika-a2a`'s `A2aClient::send_message`. Auth via existing internal-token bearer header (`MIKA_INTERNAL_TOKEN`), matching the gateway's existing internal-token contract.
- Task response from the remote agent is rendered to the CLI: text parts printed to stdout, non-text parts surfaced as `[part-type]` placeholder for now.
- Error handling: connection failure, auth failure, and JSON-RPC error responses each produce a clear single-line CLI error; no panic, no stack trace.

### Deferred to Follow-Up Work

> **Note (2026-06-15 — mika#1538 canvass reconciliation):** The R-number assignments below were drifted from the brainstorm canonical R1–R10 (the brainstorm at `docs/brainstorms/2026-06-09-mika-ascension-architecture-requirements.md` § Requirements is origin-of-truth). The drift invented R-numbers for items not in the brainstorm's R-list — the brainstorm has no Halo2/ZK requirement and no Gaia-framing R (Gaia frame appears as K6, a key decision, not as an R requirement). The section below is reconciled to canonical assignments. The original drifted assignments are preserved in the PR #1468 git history.

Brainstorm canonical R-list (for downstream-plan citations):

- **R1.** CLI dual-mode connection — shipped (this slice).
- **R2.** Signed-bundle export/import — slice 2 (mika#1538).
- **R3.** Granular slice selection — slice 2.
- **R4.** Family-bootstrap as degenerate case — **deferred to slice 3** ("family identity" slice, per mika#1538 canvass 2026-06-15) alongside R7. R4-without-R7 ships a read-only clone rather than a family member; the two form a coherent identity slice.
- **R5.** Gateway endpoint for bundle transfer — slice 2. Re-scoped here from this slice; producing the endpoint without R2 callers would be a surface with no clients (the brainstorm-stated R1+R5 first-slice split assumed R5 could ship independently, which doesn't hold).
- **R6.** Chain-forward-compat shape — slice 2 (ships with R2; manifest format is the bundle).
- **R7.** Per-instance key model — **deferred to slice 3** ("family identity" slice) alongside R4.
- **R8.** Manifest legibility for humans and agents — slice 2 (ships with R2; same reason as R6).
- **R9.** CLI granular operations (`mika bundle list/verify`, `mika agent slices`) — slice 2.
- **R10.** Security baseline (TLS, signature verification, secret wrapping) — slice 2 (ships with R2; security is not optional in slice 2).

Slice 2 signing-key locus (per mika#1538 canvass, ratified 2026-06-15): operator-bundle-shipped, encrypted with operator's existing GitHub App key (`Settings.github_app_private_key`). This is a slice-2-pattern, not load-bearing-forever — slice 3 (R7) introduces per-instance keys and narrows the operator-custody burden.

Phase 2+ items (post-mission, not numbered as R-requirements in the brainstorm): real chain-resolution semantics; continuous bidirectional sync (Approach C); peer-to-peer family Mikas without gateway-as-intermediary; cross-customer knowledge commons; auto-conflict resolution; encrypted-at-rest SQLite; soul-version history. See brainstorm § Scope Boundaries.

### Outside this product's identity

Per origin scope boundaries: streaming chunked transfer, BitTorrent/IPFS transports, cross-tenant federation, agent-level GraphQL APIs over cloud state are out of the product's identity entirely.

## Requirements

Carries forward from origin doc:

- **R1.** CLI MUST support both local mode (in-process agent loop, current default) and remote mode (proxied to gateway). Selection: `--remote <url>` flag OR `MIKA_REMOTE_AGENT_URL` env. Flag wins. Local default preserved.
- **AE1 (carried).** `MIKA_REMOTE_AGENT_URL=https://gw.example.com/a2a/cust-123/mika-prime mika ask "what's on for today"` SHALL dispatch the question to the cloud Mika Prime agent and print its reply.
- **AE2 (carried).** `mika ask "ping"` (no remote flag, no env) SHALL run the local agent loop unchanged.
- **Auth (R1 sub-decision).** Remote mode bearer = `MIKA_INTERNAL_TOKEN` env (existing gateway internal-token contract). Per-operator credentials are a Phase-2 target, not this slice.

## Key Technical Decisions

### KTD1. R1-only scope; R5 re-deferred to slice 2

The brainstorm framed R1+R5 as a candidate first slice. Implementation-time analysis showed R5 (gateway transfer endpoint) requires R2 (bundle producer) to be a useful surface — without R2, the endpoint has no clients that can produce a bundle to send through it.

R1 alone satisfies the brainstorm's stated daily-use unblock criterion ("Vincent can talk to cloud Mika Prime from his CLI"). Re-deferring R5 to slice 2 lets R2+R5 ship as a coherent transfer pipeline.

### KTD2. Selection mechanism: flag-overrides-env, both supported

`--remote <url>` flag wins over `MIKA_REMOTE_AGENT_URL` env. Either alone activates remote mode. Neither means local mode (default).

Rationale: env is the persistent setting Vincent will use day-to-day (point his shell at cloud Prime); flag is the one-shot override for testing against a different endpoint or returning to local for debugging.

### KTD3. Reuse `mika-a2a::A2aClient` — don't introduce a new transport

The A2A client (`crates/mika-a2a/src/client.rs`) already implements JSON-RPC over HTTP with bearer auth, matching the gateway's A2A proxy endpoint contract (`crates/mika-gateway/src/a2a_routes.rs`). Reuse it directly — building a parallel transport would duplicate the JSON-RPC machinery and risk drift.

### KTD4. Render Task result as text-parts-to-stdout; non-text parts placeholder

Initial cut renders Task message parts: text → stdout, file/image/data parts → `[file: <name>]` / `[image]` / `[data: <type>]` placeholder strings. Full multi-modal rendering in the TUI is a future enhancement, not a R1 requirement.

### KTD5. No streaming in this slice

`A2aClient::send_message` is the synchronous request/response path. Streaming (`message/stream` via SSE) is supported by the A2A client but adds CLI rendering complexity (chunk-by-chunk stdout flush, terminal cursor management). Defer to a later slice when the value (long-running cloud-side reasoning displayed incrementally) is worth the rendering work.

## High-Level Technical Design

```
mika ask "<prompt>"
        │
        ▼
┌────────────────────────────────┐
│ Mode selection                 │
│ (--remote flag OR              │
│  MIKA_REMOTE_AGENT_URL env)    │
└──────────┬───────────┬─────────┘
           │ remote    │ local (default)
           │           │
           │           ▼
           │   ┌──────────────────┐
           │   │ run_agent()      │  (unchanged)
           │   │ in-process loop  │
           │   └──────────────────┘
           ▼
┌────────────────────────────────┐
│ A2aClient::new(url, token)     │
│ send_message(                  │
│   MessageSendParams {          │
│     message: { role: user,     │
│                parts: [text] } │
│   })                           │
└──────────┬─────────────────────┘
           ▼
┌────────────────────────────────┐
│ Gateway /a2a/{cust}/{agent}    │
│ proxies to agent pod           │
└──────────┬─────────────────────┘
           ▼
┌────────────────────────────────┐
│ Task response                  │
│ render parts → stdout          │
└────────────────────────────────┘
```

Directional only — not implementation specification. Field names and exact struct shapes resolve at execution time against the live A2A types.

## Implementation Units

### U1. CLI surface — `--remote` flag + env var wiring

**Goal:** Add the user-facing surface that selects local vs remote mode for `mika ask`.

**Requirements:** R1 (selection mechanism), AE1 (env path), AE2 (local default unchanged).

**Dependencies:** none.

**Files:**
- `crates/mika-cli/src/commands/ask.rs` — add `remote: Option<String>` field to the clap subcommand args struct; resolve effective mode (flag → env → local) before agent dispatch.
- `crates/mika-cli/src/main.rs` (only if the ask subcommand's clap derive lives here rather than in `ask.rs`) — wire the new arg through.
- `crates/mika-cli/tests/` (new test file or extend existing) — mode-selection unit tests.

**Approach:**
- Add `--remote <URL>` to the `ask` subcommand's args via clap's `#[arg(long, env = "MIKA_REMOTE_AGENT_URL")]` — clap handles flag-overrides-env natively.
- Resolve effective mode: if `args.remote.is_some()` → remote mode with that URL; else → local mode.
- Validate URL parses as `http://` or `https://` before agent dispatch; on parse failure print a clear error and exit non-zero.

**Test scenarios:**
- Covers AE1. `MIKA_REMOTE_AGENT_URL=https://x.test mika ask "q"` → mode resolution returns Remote("https://x.test"). (No network call in this unit test — the test asserts the resolution branch, not the network behavior.)
- Covers AE2. `mika ask "q"` with no flag and no env → mode resolution returns Local.
- `--remote https://override.test` with `MIKA_REMOTE_AGENT_URL=https://env.test` → flag wins; resolution returns Remote("https://override.test").
- `--remote not-a-url` → exits non-zero with a single-line URL-parse error; does not panic.

**Verification:** `cargo test -p mika-cli` passes the new mode-selection tests. `cargo run --bin mika -- ask --help` shows the new `--remote <URL>` option with the env-var hint clap auto-emits.

### U2. Remote-mode dispatch via `A2aClient`

**Goal:** When mode resolves to Remote, dispatch the user's prompt via A2aClient and render the Task response to stdout.

**Requirements:** R1 (remote-mode behavior), AE1 (round-trip works).

**Dependencies:** U1.

**Files:**
- `crates/mika-cli/src/commands/ask.rs` — branch on the resolved mode; remote branch constructs `A2aClient`, builds `MessageSendParams`, calls `send_message`, renders Task.
- `crates/mika-cli/Cargo.toml` — add `mika-a2a` as a workspace dep if not already present.
- `crates/mika-cli/src/render.rs` (new, or inline in `ask.rs` if small) — Task → stdout rendering helper (text parts to stdout, non-text parts as placeholder strings).

**Approach:**
- Construct `A2aClient::new(remote_url, auth_token)` where `auth_token = std::env::var("MIKA_INTERNAL_TOKEN").ok()`.
- Build `MessageSendParams` with the user's prompt as a single text part, role = `user`, fresh message_id.
- Call `client.send_message(params).await`. Match on `Result<Task, A2aError>`:
  - Ok(task) → walk `task.status.message.parts` (or the equivalent latest-message path on the Task type — exact accessor resolved at execution time against `mika-a2a/src/types.rs`); print text parts; emit placeholder for file/image/data parts.
  - Err(A2aError::Http(e)) → print `connection error: <e>` to stderr, exit non-zero.
  - Err(A2aError::InvalidJsonRpc(msg)) → print `remote error: <msg>` to stderr, exit non-zero.
  - Other A2aError variants → match exhaustively, single-line stderr message per variant.

**Patterns to follow:**
- The eval harness pattern in `crates/mika-agent/tests/eval/` for integration tests against the agent loop — adapt the harness shape for testing the remote path with a mock A2A server (e.g., `wiremock` or an in-process `axum` test server).
- The existing `crates/mika-a2a/CLAUDE.md` for A2A protocol semantics and the `MessageSendParams`/`Task` type shapes.

**Test scenarios:**
- Covers AE1. Mock A2A server accepts `message/send`, returns a Task whose latest message has a single text part with content "hello world"; CLI invocation with `--remote <mock-url>` prints `hello world` and exits 0.
- Mock server returns a JSON-RPC error (code -32600, message "Invalid Request"); CLI prints `remote error: JSON-RPC error -32600: Invalid Request` to stderr and exits non-zero.
- Mock server returns 401 Unauthorized; CLI prints `connection error: <http detail>` to stderr and exits non-zero.
- Mock server responds with a Task containing a file part (e.g., `{kind: "file", name: "foo.txt"}`); CLI renders `[file: foo.txt]` placeholder, exits 0.
- `MIKA_INTERNAL_TOKEN` unset: A2aClient is constructed with `None` auth; the Authorization header is omitted. If the server requires it, the 401 path above fires.

**Verification:** `cargo test -p mika-cli` passes new integration tests. Manual smoke: start a local `mika-spirit` instance, point `--remote http://localhost:8080/a2a/<cust>/<agent>` at it with `MIKA_INTERNAL_TOKEN` set, observe round-trip reply printed to stdout.

### U3. Documentation + help-text polish

**Goal:** Document the new mode surface so Vincent's daily-use path is discoverable.

**Requirements:** carry the R1 surface forward to docs so subsequent slices have an anchor.

**Dependencies:** U1, U2.

**Files:**
- `docs/configuration.md` (or equivalent — exact path resolved at execution time) — add a section on `MIKA_REMOTE_AGENT_URL` under env vars.
- `crates/mika-cli/CLAUDE.md` — add a short note that `mika ask` has dual-mode behavior with selection rules.
- `docs/runtime-structure.md` — note that remote mode bypasses local `~/.mika/data/mika.db` writes (state lives on the cloud agent).

**Approach:**
- Single-paragraph addition per file. No new top-level sections unless the existing doc structure warrants one. Keep it factual: variable, behavior, default.

**Test expectation: none — documentation-only unit. The doc audit step (`/compound-engineering:resolve_todo_parallel` followed by `/ce:compound`) catches missing or stale references.**

**Verification:** doc-sync script (if present) passes; references resolve.

## Risks & Dependencies

- **A2A client API drift.** The plan assumes `A2aClient::send_message` and `MessageSendParams` shape match what `crates/mika-a2a/src/client.rs` ships today. If the client API is in flux for another ticket, this slice's tests will catch the mismatch at `cargo build` time.
- **Gateway internal-token contract.** The plan relies on the gateway accepting `MIKA_INTERNAL_TOKEN` as a bearer header on `/a2a/{customer_id}/{agent_name}`. This is the existing internal-token mechanism per origin doc R1; no new auth surface is introduced.
- **No production routing yet.** This slice ships the *client*. Pointing it at a live cloud-hosted Mika Prime requires the cloud deployment to be reachable on the network and configured with a customer_id Vincent can address. That is operational work on `mika-cloud`, not in this slice.

## Open Questions (deferred to execution)

- Exact accessor path for the Task's most-recent-message parts in `mika-a2a`'s `Task` type — resolved at execution time by reading `crates/mika-a2a/src/types.rs`. The plan does not pre-specify the field name.
- Whether to add `MIKA_REMOTE_AGENT_URL` to `mika-common::Settings` so it gets first-class config-rs handling. Default position: just use `std::env::var` directly in the CLI command — Settings is for engine-side config, CLI flag/env wiring is fine as a direct env read. Revisit if it grows.

## Sources & Research

- Origin: `docs/brainstorms/2026-06-09-mika-ascension-architecture-requirements.md` (R1, R5; KTD1 R5 re-deferral analysis based on R5's dependency on R2).
- Existing primitive: `crates/mika-a2a/src/client.rs` (`A2aClient::send_message`, `MessageSendParams`).
- Existing gateway endpoint: `crates/mika-gateway/src/a2a_routes.rs` (`/a2a/{customer_id}/{agent_name}` proxy).
- CLI entry: `crates/mika-cli/src/commands/ask.rs:45` (`pub async fn run`).
- Reference doc: `crates/mika-a2a/CLAUDE.md` (A2A protocol semantics).
