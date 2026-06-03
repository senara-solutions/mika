---
title: "feat: one-shot OllamaProvider payload dump for MikaModel OOD diagnostics"
type: feat
status: active
date: 2026-06-03
origin: senara-solutions/mika#1387
---

# feat: one-shot OllamaProvider payload dump for MikaModel OOD diagnostics

## Overview

Add an env-gated, one-shot debug affordance to `OllamaProvider::send_once` that writes the exact JSON body sent to Ollama's `/api/chat` to a configured file path. Lets the operator (or follow-up diagnostic work) capture mika-server's actual runtime payload — system prompt + messages + tools + options — to diagnose the out-of-distribution behavior observed post-mika#1379 with `MIKA_LLM_PROVIDER=mikamodel mika ask "hello"`.

## Problem Frame

After mika#1379 shipped (PR #1380, commit `368d6b0`), local smoke testing surfaced that the MikaModel provider returns unrelated multi-section workplace-assistant boilerplate ("## Completed Tasks", "## Pending", etc.) rather than responding to the user input. Five direct curl probes against the same Ollama backend with various system-prompt shapes (training-shape, none, math-tutor role, explicit "use status report format" instruction, multi-section markdown) all returned sane conversational output. **The upstream model is healthy in isolation; something specific in mika-server's runtime `/api/chat` payload triggers the OOD behavior.**

Three plausible triggers, in descending likelihood:

1. **The `tools` array.** mika-server passes the full ~82-tool catalog on every request. The largest single structural delta vs the probes (which had no `tools` field set). Most plausible distribution-shift culprit.
2. **The full multi-section system prompt.** 14 `## Section` headers (Current Time, Communication Channel, Core Memory, Instructions, Tool Usage, Pending Commitments, Silent Mode, Today's Conversations, Recent Audit Events, File Tools, Task Health, Trigger, etc.), each with detailed rules and XML-tagged content.
3. **Conversation history.** If the agent home dir has prior turns, mika-server appends them; a prior assistant message containing similar structural content could elicit continuation.

Without the actual payload, choosing a fix is guessing — a compact-prompt-builder branch for `ProviderKind::MikaModel` ships fast but is wrong if the trigger is the tools array, and vice versa. The smallest possible affordance to break the guessing cycle is a one-shot dump.

## Requirements Trace

- **R1.** New env var `MIKA_OLLAMA_DUMP_PAYLOAD` accepting a filesystem path. When unset, no dump fires (zero overhead).
- **R2.** When set, the NEXT `/api/chat` request through `OllamaProvider` serializes its JSON body to that path. Subsequent requests in the same process do NOT overwrite (one-shot semantics, enforced via process-level `AtomicBool`).
- **R3.** Hard byte cap on the dumped file (default 256 KiB). When truncated, the file ends with a literal `\n<!-- TRUNCATED at N bytes; total payload was M bytes -->\n` marker so grep / `cat` users notice.
- **R4.** Single `tracing::warn!` log line on successful dump, including the path, bytes written, and truncation status. Failures (e.g. permission denied) emit `tracing::error!` and do NOT flip the one-shot flag — operator can retry after fixing the path.
- **R5.** Scope: `OllamaProvider` only. Routes for both `ProviderKind::Ollama` and `ProviderKind::MikaModel` so the affordance helps both transports.
- **R6.** Zero behavioral effect on the actual HTTP request — dump is observation-only, fires after JSON serialization but before the body is moved into `reqwest::RequestBuilder::json()`. Even on dump failure, the request proceeds normally.
- **R7.** Unit test: with `MIKA_OLLAMA_DUMP_PAYLOAD` set to a `tempfile` path, calling `try_dump_payload(body_json)` writes the expected JSON to the path and flips the one-shot flag.
- **R8.** Unit test: second call with the same env var is a no-op (one-shot enforced).
- **R9.** Unit test: when the body exceeds the byte cap, the file ends with the truncation marker.

## Proposed Solution

Add a small helper in `crates/mika-common/src/llm/ollama.rs`:

```rust
use std::sync::atomic::{AtomicBool, Ordering};

const PAYLOAD_DUMP_CAP_BYTES: usize = 256 * 1024;
static PAYLOAD_DUMP_FIRED: AtomicBool = AtomicBool::new(false);

fn try_dump_payload(body_json: &str) {
    let Ok(path) = std::env::var("MIKA_OLLAMA_DUMP_PAYLOAD") else { return };
    if path.is_empty() { return; }

    // One-shot — flip atomically; if already fired, do nothing.
    if PAYLOAD_DUMP_FIRED.swap(true, Ordering::Relaxed) { return; }

    let total_len = body_json.len();
    let (slice, truncated) = if total_len > PAYLOAD_DUMP_CAP_BYTES {
        (&body_json[..PAYLOAD_DUMP_CAP_BYTES], true)
    } else {
        (body_json, false)
    };

    let mut content = String::with_capacity(slice.len() + 128);
    content.push_str(slice);
    if truncated {
        content.push_str(&format!(
            "\n<!-- TRUNCATED at {PAYLOAD_DUMP_CAP_BYTES} bytes; total payload was {total_len} bytes -->\n"
        ));
    }

    match std::fs::write(&path, &content) {
        Ok(()) => warn!(
            path = %path,
            bytes_written = content.len(),
            truncated,
            "Ollama payload dumped (one-shot)"
        ),
        Err(e) => {
            // Reset the flag so the operator can retry after fixing the path.
            PAYLOAD_DUMP_FIRED.store(false, Ordering::Relaxed);
            error!(path = %path, error = %e, "Ollama payload dump failed");
        }
    }
}
```

Wire it in `send_once` right after the existing dev-mode body logging block (line 348-353), before the `reqwest::Client::post()` call. JSON serialization is already done for the debug log — reuse `body_json` if the log block fires; otherwise serialize once for the dump.

Refactor the small dev-log block so JSON serialization happens once whether or not either gate is enabled:

```rust
let body_json = match serde_json::to_string(request) {
    Ok(s) => s,
    Err(e) => {
        warn!(error = %e, "ollama: failed to serialize request for debug log");
        // Fall through — the .json() call below will surface the real error.
        String::new()
    }
};
if !body_json.is_empty() {
    if tracing::enabled!(target: "mika::llm_debug", tracing::Level::DEBUG) {
        debug!(target: "mika::llm_debug", body = %body_json, provider = "ollama", "llm request body");
    }
    try_dump_payload(&body_json);
}
```

## Key Technical Decisions

### Why a process-level `AtomicBool` (not env-var unset, not per-instance flag)

- **Env-var unset.** Would require `unsafe { std::env::remove_var(...) }` (Rust 2024 edition makes env-modification unsafe). We don't want to introduce unsafe blocks for what is fundamentally a debug affordance.
- **Per-instance flag** (e.g. `AtomicBool` on the `OllamaProvider` struct). Multiple providers can coexist in the same process (mika-agent + cli + gateway in dev workflows); a per-instance flag would let the dump fire once per instance, giving N dumps for N instances. Process-level matches the "one-shot for the operator's diagnostic capture" intent.
- **Static `AtomicBool` at module scope.** Simplest, lock-free, process-scoped. Resets only on process restart, which matches "one-shot per debug session."

### Why a byte cap (not unlimited, not chunked)

- The payload may include a full 82-tool catalog plus a large conversation history. Unlimited writes can produce multi-megabyte files that hang text editors and complicate `grep` / `jq` triage.
- 256 KiB is a comfortable budget: a 14-section system prompt + 82-tool catalog typically lands well below 100 KiB; 256 KiB allows 2-3× headroom for conversation history.
- Chunked / multi-file output: rejected as scope creep. If a real payload exceeds the cap, the truncation marker tells the operator and a future ticket can extend.

### Why fail-fast (do NOT flip flag on dump error)

- A permission-denied or no-such-directory error on first call should be operator-fixable without restarting mika-agent. Flipping the flag would lock the operator out until restart. The error log surfaces the failure; retry on next request after fixing the path.

### Why not generic per-provider dump

- `OpenAiCompatibleProvider` and `AnthropicProvider` would each need their own implementation since the request shape differs. The only known active diagnostic need is MikaModel via OllamaProvider. Premature generalization is YAGNI. If a second provider gains a similar need, lift the helper into `mika-common::llm` then.

## Scope Boundaries

### In scope

- `try_dump_payload` helper + the `send_once` wiring.
- Unit tests for the three R7/R8/R9 cases.
- Plan doc (this file) + solution doc.

### Out of scope

- **The actual OOD bug fix.** This ticket ships the diagnostic; the fix lands in a follow-up once the dumped payload identifies the trigger.
- **Generic LLM-payload-dump affordance for other providers.** Defer until needed.
- **Production telemetry.** This is a local debug flag, not metrics/logs. Cache scrubbing, secret redaction, etc. are deferred — `OllamaProvider` is typically authless so the auth header issue is mostly moot, but a follow-up could harden if needed.
- **Configurable byte cap.** Hard-coded 256 KiB until evidence demands flexibility.
- **Tools array filtering.** A separate follow-up ticket if the dump reveals tools as the trigger.

### Deferred to separate tickets

- Compact-prompt-builder branch for `ProviderKind::MikaModel` (one of the candidate patches — gated on dump evidence).
- Upstream-model retrain on a distribution-matched prompt shape (the other candidate patch — gated on dump evidence + cost authorization).

## Verification

### Build verification

1. `cargo check --workspace` clean.
2. `cargo clippy -p mika-common --no-deps -- -D warnings` clean.
3. `cargo test -p mika-common --lib` — 363 passed (361 prior + 2 new MikaModel tests from #1379) + 3 new dump tests = 366 expected.

### Behavioral verification (post-merge, local)

1. `MIKA_OLLAMA_DUMP_PAYLOAD=/tmp/mika-payload.json MIKA_LLM_PROVIDER=mikamodel mika ask "hello"` writes `/tmp/mika-payload.json` containing the exact serialized JSON sent to Ollama.
2. The file is valid JSON (assuming no truncation) — `jq . /tmp/mika-payload.json` succeeds.
3. Re-running `mika ask "..."` in the same process does NOT overwrite — second invocation produces no log line, file mtime unchanged.
4. Setting the env var to an unwritable path produces a `tracing::error!` line and leaves the request semantics unchanged.
5. After process restart, the one-shot flag resets — first request dumps again.

## Risks

- **Low.** Affordance is observation-only, fires after JSON serialization, has no effect on the actual HTTP request. Worst case (dump failure): an extra `tracing::error!` log line, request proceeds normally.
- **Secret leak risk:** OllamaProvider runs typically authless (local Ollama doesn't need auth); the dumped JSON wouldn't contain the bearer token. If a future MikaModel hosted-endpoint deployment uses auth, the dump file would be operator-readable; current scope keeps this acceptable as a local-only diagnostic affordance.

## Out of scope reminder

This plan ships **the diagnostic dump**. It does NOT:

- Fix the MikaModel OOD bug observed post-#1379.
- Modify behavior of any existing provider or request path.
- Add production telemetry or persistent logging.
- Generalize beyond `OllamaProvider`.
