---
module: crates/mika-common/src/llm/ollama.rs
tags: [llm, ollama, debug-affordance, one-shot, observability]
problem_type: diagnostic-instrumentation
category: architecture-patterns
---

# One-shot OllamaProvider payload dump for OOD diagnostics

## Problem

After mika#1379 shipped the MikaModel provider, smoke testing surfaced that the upstream model returns content unrelated to the user input — multi-section workplace-assistant boilerplate ("## Completed Tasks", "## Pending", "## Blockers") instead of responding to "hello" or "what is 2+2". Five direct `curl` probes against the same Ollama backend with various system-prompt shapes (training-shape, none, math-tutor role, "use status report format" instruction, multi-section markdown) all returned sane conversational output. **The model is healthy in isolation; something specific in mika-server's runtime `/api/chat` payload triggers it.**

Without the actual payload, diagnosing the trigger means guessing among three candidates: the full multi-section system prompt, the ~82-tool catalog, or appended conversation history. Patching without evidence risks shipping the wrong fix.

## Solution

Env-gated, one-shot debug affordance in `OllamaProvider::send_once` that writes the next request's serialized JSON body to a configured path. Process-scoped one-shot flag (lock-free `AtomicBool`) ensures one dump per debug session. Hard byte cap (256 KiB) keeps the file grep-able.

```rust
// MIKA_OLLAMA_DUMP_PAYLOAD=/tmp/payload.json mika ask "hello"
// → /tmp/payload.json contains the exact JSON sent to /api/chat
// → tracing::warn!() confirms the dump fired
// → next mika ask in same process: no-op (one-shot)
```

### Wire shape

In `send_once`, after auth headers are built and before the `reqwest::post()` call:

```rust
let dump_enabled = std::env::var_os("MIKA_OLLAMA_DUMP_PAYLOAD").is_some_and(|v| !v.is_empty());
let debug_log_enabled = tracing::enabled!(target: "mika::llm_debug", tracing::Level::DEBUG);
if dump_enabled || debug_log_enabled {
    match serde_json::to_string(request) {
        Ok(body_json) => {
            if debug_log_enabled { /* existing debug log */ }
            if dump_enabled { try_dump_payload(&body_json); }
        }
        Err(e) => warn!(error = %e, "ollama: failed to serialize request for debug log/dump"),
    }
}
```

Both gates share the single JSON serialization. When both gates are off, the cost is one `env_var_os` lookup + one `tracing::enabled!` check — both ~zero-cost when not enabled.

### Truncation

When the serialized body exceeds 256 KiB, the dump contains the first 256 KiB verbatim plus a marker line:

```
<!-- TRUNCATED at 262144 bytes; total payload was <N> bytes -->
```

Lets the operator see exactly how much was lost and decide whether to raise the cap.

## Key technical decisions

### Why a process-level `AtomicBool` (not env unset, not per-instance flag)

- **Env-unset.** Would need `unsafe { std::env::remove_var }` (Rust 2024 edition makes env-mutation unsafe). We don't introduce unsafe blocks for a debug affordance.
- **Per-instance flag.** Multiple providers can coexist in the same process; a per-instance flag fires N times for N instances. Process-level matches operator intent ("one dump per debug session").
- **Static `AtomicBool`.** Simplest, lock-free, resets only on process restart. Test-only helper `reset_payload_dump_flag()` (gated behind `#[cfg(test)]`) lets each serial test start clean.

### Why fail-fast (do NOT flip the flag on dump error)

A permission-denied or no-such-directory error on first call should be operator-fixable without restarting mika-agent. Flipping the flag would lock the operator out until restart. On error, the flag is reset; next request retries the dump.

### Why a byte cap (not unlimited, not chunked)

mika-server's payload likely combines a 14-section system prompt (~50 KiB) + an ~82-tool catalog (~80 KiB) + variable conversation history. 256 KiB is 2-3× headroom for the static parts. Unlimited writes can produce multi-megabyte files that hang text editors. Chunked / multi-file output rejected as scope creep — the truncation marker signals the operator if more is needed.

### Why scoped to `OllamaProvider` (not generic)

`OpenAiCompatibleProvider` and `AnthropicProvider` would each need their own implementation since the request shape differs. The known active diagnostic need is MikaModel via OllamaProvider. Premature generalization is YAGNI. If a second provider gains a similar need, lift the helper into `mika-common::llm`.

## Verification

- 5 new unit tests (R7 basic dump, R8 one-shot semantics, R9 truncation marker, env-unset noop, env-empty noop) — all `#[serial]`-gated to handle the process-wide flag without flakiness.
- `cargo test -p mika-common --lib` — 368 passed (363 prior + 5 new).
- `cargo clippy -p mika-common --no-deps --tests -- -D warnings` clean.

## Out of scope

- The actual OOD bug fix (separate follow-up gated on captured payload evidence).
- Generic per-provider dump affordance.
- Production telemetry / secret redaction / cache scrubbing.
- Configurable byte cap.

## Related

- mika#1387 — tracking issue
- mika#1379 / PR #1380 — parent provider integration whose smoke surfaced the OOD bug
- `docs/plans/1387-ollama-payload-dump-one-shot.md` — implementation plan
