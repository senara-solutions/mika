# Plan: Tracing-Aware Panic Hook for mika-server

**Ticket:** mika issue#765
**Type:** feat
**Date:** 2026-05-28

## Problem

Background tokio tasks whose `JoinHandle` is dropped can panic silently. tokio's default panic handler writes to stderr as raw text, bypassing the structured JSON tracing layer. This means panics in spawned tasks never reach the observability stack (Langfuse, server log file, `jq`-based querying). The KG resolver UTF-8 panic incident cost ~1.5 hours of debugging because the panic was only visible in raw stderr lines mixed into the JSON log file.

Per-site `handle.await` + `JoinError::is_panic()` patterns (already in place for extraction/resolution spawns in `server/mod.rs`) cover specific spawn sites. This ticket is the defense-in-depth layer for any future spawn site that forgets to await its handle.

## Approach

Install a `std::panic::set_hook` in the mika-server binary that emits a `tracing::error!` event with structured fields before chaining to the previous hook. The hook must be installed before any tokio work is spawned.

### Key Design Decisions

1. **Chain, don't replace.** Use `std::panic::take_hook()` to capture the previous hook and call it after logging. This preserves default backtrace output (`RUST_BACKTRACE`) and test framework hooks.

2. **Install location: `main()` in `mika-server.rs`, after tracing init but before `run_server()`.** The `#[tokio::main]` macro creates the runtime before entering `main()`, but no work is spawned until `run_server()`. Tracing must be initialized first so the `tracing::error!` call inside the hook actually reaches the subscriber.

3. **Scoped to mika-server only.** The CLI (`mika`) and gateway (`mika-gateway`) don't have the same long-running background-task problem. If they need it later, the helper can be extracted to `mika-common`.

4. **Helper function in a new module.** Create `crates/mika-agent/src/panic_hook.rs` with an `install_tracing_panic_hook()` function. This keeps the binary entry point clean and makes the hook testable.

## Implementation Steps

### Step 1: Create the panic hook module

**File:** `crates/mika-agent/src/panic_hook.rs`

Create a new module with a single public function:

```rust
/// Install a tracing-aware panic hook that emits structured log events
/// before chaining to the previous (default) hook.
///
/// Must be called after tracing is initialized and before any
/// tokio tasks are spawned.
pub fn install_tracing_panic_hook() {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());

        let payload = info.payload();
        let msg = payload.downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string payload>".to_string());

        tracing::error!(
            target: "panic",
            location = %location,
            message = %msg,
            thread = %std::thread::current().name().unwrap_or("<unnamed>"),
            event = "process_panic",
            "Rust panic caught by tracing hook"
        );

        // Chain to previous hook — preserves backtrace formatting,
        // test panic reporting, and any other registered handlers.
        prev_hook(info);
    }));
}
```

**Key details:**
- `target: "panic"` gives a filterable tracing target
- `event = "process_panic"` matches the ticket spec for `jq` querying
- Payload extraction mirrors the existing pattern in `server/mod.rs:1050-1056`
- Thread name included for identifying which tokio worker thread panicked

### Step 2: Register the module and wire into mika-server

**File:** `crates/mika-agent/src/lib.rs`
- Add `pub mod panic_hook;`

**File:** `crates/mika-agent/src/bin/mika-server.rs`
- Add call to `mika_agent::panic_hook::install_tracing_panic_hook()` after the `_log_guard` initialization (line 30) and before `run_server()` (line 32).

```rust
// Install tracing-aware panic hook (defense-in-depth for spawned tasks
// that panic without their JoinHandle being awaited — see mika#765)
mika_agent::panic_hook::install_tracing_panic_hook();

mika_agent::server::run_server(&settings).await
```

### Step 3: Add unit tests

**File:** `crates/mika-agent/src/panic_hook.rs` (inline `#[cfg(test)] mod tests`)

Four tests covering the acceptance criteria:

1. **`test_hook_chains_to_previous`** — Install a custom hook that sets an `AtomicBool`, then install the tracing hook on top. Trigger a panic in `std::panic::catch_unwind`. Assert the `AtomicBool` was set (chain works).

2. **`test_str_payload_captured`** — Use a `tracing_subscriber::fmt::Layer` with an in-memory writer. Install tracing hook. Panic with `&str` payload inside `catch_unwind`. Assert the captured output contains `process_panic` and the panic message.

3. **`test_string_payload_captured`** — Same as above but panic with `String` payload (`panic!(String::from("owned message"))`).

4. **`test_non_string_payload`** — Panic with a non-string payload (`std::panic::panic_any(42i32)`). Assert the output contains `<non-string payload>`.

**Testing approach for tracing capture:**
- Use `tracing_subscriber::fmt::Layer` with `fmt::TestWriter` or a `Vec<u8>` buffer behind `Arc<Mutex<>>` as the writer.
- Set up a scoped subscriber with `tracing::subscriber::with_default()` so tests don't interfere with each other.
- The panic hook calls `tracing::error!()` which dispatches to the current subscriber — the scoped subscriber captures it.

**Note on `catch_unwind`:** Since the hook chains to the default hook (which prints to stderr), tests should use `catch_unwind` to prevent the test process from actually aborting. The chained hook writes to stderr, which is expected and doesn't affect test pass/fail.

### Step 4: Integration-level verification

**File:** `crates/mika-agent/src/panic_hook.rs` (additional test)

5. **`test_spawned_task_panic_reaches_tracing`** — A `#[tokio::test]` that:
   - Sets up a tracing subscriber with an in-memory capture layer
   - Installs the panic hook
   - Spawns a `tokio::spawn` task that panics (handle dropped)
   - Waits briefly (`tokio::time::sleep(100ms)`) for the panic to fire
   - Asserts the captured tracing output contains `process_panic`

This directly validates the motivating use case: a spawned task with a dropped handle still produces a structured log event.

6. **`test_cargo_test_panics_still_work`** — Not a new test file but a verification step: run the existing test suite (`cargo test -p mika-agent`) and confirm all tests pass, especially any tests that deliberately trigger panics (e.g., grounding regression fixtures). The hook chains to the previous hook, so test framework behavior should be preserved.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/panic_hook.rs` | **New** — panic hook module with `install_tracing_panic_hook()` + tests |
| `crates/mika-agent/src/lib.rs` | Add `pub mod panic_hook;` |
| `crates/mika-agent/src/bin/mika-server.rs` | Add hook installation call after tracing init |

## Out of Scope

- Installing the hook in `mika-cli` or `mika-gateway` (per ticket)
- Crash handling beyond logging (alerts, core dumps, restart)
- Replacing per-spawn-site `handle.await` patterns
- Extracting to `mika-common` (can be done later if other binaries need it)

## Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| Hook installed before tracing subscriber → `tracing::error!` is a no-op | Installation order in `main()` is after `logging::init()`, which sets the global subscriber |
| Hook closure captures `prev_hook` by move, creating lifetime issues | `Box::new(move \|info\| ...)` is the standard pattern; `prev_hook` is `Box<dyn Fn>` which is `'static` |
| Tests interfere with each other via global panic hook state | Use `catch_unwind` for isolation; the tracing capture uses scoped subscribers |
| Thread name unavailable in unnamed tokio worker threads | Fallback to `"<unnamed>"` — tokio names its worker threads `tokio-runtime-worker` by default, so this is informational |

## Acceptance Criteria Mapping

| AC | Covered by |
|----|-----------|
| Panic hook installed before tokio runtime starts | Step 2 — installed after tracing init, before `run_server()` |
| Hook chains to previous hook | Step 1 design + Step 3 test 1 |
| Test: spawned task panic produces structured event | Step 4 test 5 |
| Test: `&str` payload captured | Step 3 test 2 |
| Test: `String` payload captured | Step 3 test 3 |
| Test: `cargo test` panic output still works | Step 4 test 6 (verification) |
