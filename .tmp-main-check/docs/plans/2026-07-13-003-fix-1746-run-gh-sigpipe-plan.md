---
type: fix
issue: 1746
title: Fix run_gh SIGPIPE truncation on large diffs — drain child stdout past cap in spawn_and_collect
status: draft
---

# Plan — mika#1746 run_gh SIGPIPE truncation on large diffs

## Ticket

mika#1746 — `run_gh pr diff` truncates via SIGPIPE when the child's stdout exceeds the reader's byte cap. The current `read_with_counter` wraps the child's stdout in `.take(max_bytes)` and drops the handle once the cap is hit. The kernel then closes the pipe; the child receives SIGPIPE on its next `write(2)` and aborts. `gh` exits non-zero, its buffered stdout is discarded, and mika-qa loses the tail of the diff on PRs with ≥10 files / ≥600 additions.

The visible symptom for mika-qa is the **worktree fallback**: she cannot re-issue `run_gh pr diff` because a second call reproduces the truncation, so she falls back to `run_shell` on individual files. Each file read burns one step against the per-turn 14-step budget; complex PRs (mika-cloud#144, #146) exhaust the budget on diff reconstruction alone. From the outside this looks like she "gave up" or "used a simpler approach" — from inside, she resumed exactly where she left off, but with the diff truncated at the pipe.

This ticket ships **Option A** from mika#1746 (small/local mechanism fix at the read layer). Option B (engine-inject the full diff into her context without a tool call) is deliberately out of scope — larger architectural change, tracked separately if needed.

## Problem

`read_with_counter` in `crates/mika-agent/src/skills/builtin_handlers.rs`:

```rust
async fn read_with_counter<R: AsyncRead + Unpin>(
    reader: R,
    max_bytes: usize,
    counter: Arc<AtomicUsize>,
) -> Vec<u8> {
    let mut take = reader.take(max_bytes as u64);
    // ... reads up to max_bytes into buf, then returns.
    // `take` (and the underlying reader) drops here.
}
```

Once the cap is hit the wrapper drops, the underlying `ChildStdout` half of the pipe closes, and the child sees `EPIPE` / SIGPIPE on its next write. `gh` treats SIGPIPE as a fatal signal, exits with a non-zero status, and its already-buffered stdout is thrown away. Downstream: `spawn_and_collect` returns a truncated diff without any marker telling the caller the tail was lost, and callers (mika-qa) silently review a partial diff or fall back to per-file reads.

## Scope

**In scope (v1 ships):**

1. **Drain past the cap.** Change `read_with_counter` to keep reading from the child's stdout after the buffer reaches `max_bytes`, discarding the excess bytes but counting them. Draining lets the child exit cleanly (status 0, no SIGPIPE) and its stdout arrives complete up to the cap.
2. **Return a discard counter.** Return `ReaderResult { buf, discarded_bytes }` instead of `Vec<u8>` so the caller can distinguish "output fit under the cap" from "output was drained past the cap."
3. **Surface truncation to the caller.** When `discarded_bytes > 0`, append a human-readable truncation marker to the returned tool output:
   ```
   [... run_gh output truncated at <cap> bytes; <N> more bytes discarded to prevent SIGPIPE on the child (mika#1746) ...]
   ```
   This lets mika-qa see, in-band, that the diff is a bounded prefix — she can decide whether the visible portion is enough to review or whether to fall back to the worktree explicitly.
4. **Log the truncation event.** Emit the existing `spawn_and_collect complete` info-log with an additional `stdout_discarded_bytes` field and a distinct message (`... (stdout truncated at cap; mika#1746)`) when discard is non-zero. Keeps `grep stdout_discarded_bytes` searchable for the same class in the future.
5. **Widen the chunk size to 8192 bytes.** The old 256-byte chunk multiplies the number of `read()` syscalls on large diffs; 8 KiB matches the `Vec::with_capacity` hint already in place. Reduces user-CPU cost of the drain phase.

**Out of scope:**

- Raising the `MAX_OUTPUT_LEN` cap itself. That's a separate policy call (mika#1746 Option B territory).
- Engine-injecting the full diff into mika-qa's context without a tool call (Option B).
- Changing mika-qa's per-turn step budget or her worktree-fallback ordering.
- Any behavior change on stderr's truncation marker — stderr shares the drain fix but does not surface a marker (stderr is warning noise, not authoritative content).

## Committed positions

1. **Drain, don't buffer unbounded.** The alternative is to grow `buf` past `max_bytes` (raise or remove the cap). That defeats the memory-guard purpose of the cap and doesn't fix the class — a big enough diff still SIGPIPEs. Draining fixes the class regardless of cap value.
2. **In-band marker over silent truncation.** mika-qa is the primary caller. She reviews text, not log JSON. A visible marker lets her make an informed choice; a silent truncation forces her to correlate with the log or discover the loss empirically.
3. **`stdout` marker only, not `stderr`.** Stderr is used for warning noise and gh's own progress; users don't review it as authoritative content. Draining stderr is still correct (same SIGPIPE class), but a stderr marker adds noise for no signal.
4. **8 KiB chunk over 256 B.** Matches the `Vec::with_capacity` hint. On a 5 MiB `gh pr diff` the syscall count drops ~32×; the drain phase is short enough to matter.
5. **Additive log field, not a new event.** Keeping `spawn_and_collect complete` as the event name preserves the existing #900 telemetry contract; `stdout_discarded_bytes` is additive.

## Acceptance criteria

- [ ] **AC1 — Drain-past-cap behavior.** After the fix, feeding a 10-MiB stdout stream through `read_with_counter` with `max_bytes = 128 KiB` returns `buf.len() == 128 KiB` and `discarded_bytes ≈ 10 MiB − 128 KiB`, and the child process exits with status 0 (no SIGPIPE). Concrete assertion in a new unit test in `builtin_handlers.rs`'s test module.
- [ ] **AC2 — Truncation marker present on discard.** When `spawn_and_collect` returns a success `ToolOutput` and `stdout_discarded > 0`, the output string ends with a line matching `[... run_gh output truncated at <N> bytes; <M> more bytes discarded to prevent SIGPIPE on the child (mika#1746) ...]`. When `stdout_discarded == 0`, no marker is appended. Unit test covers both branches.
- [ ] **AC3 — Log field emitted on discard.** When `stdout_discarded > 0`, the `spawn_and_collect complete` info-log includes `stdout_discarded_bytes = <M>` and `stdout_cap_bytes = MAX_OUTPUT_LEN`. When `stdout_discarded == 0`, the field is absent (no zero-noise). Verified by tracing-test capture in the unit test.
- [ ] **AC4 — Existing behavior preserved.** All existing tests for `spawn_and_collect` (success path, failure path, stderr capture, elapsed-ms field) remain green. `cargo test -p mika-agent` clean.
- [ ] **AC5 — No regression on small outputs.** For outputs strictly under the cap, `discarded_bytes == 0`, the returned buffer matches the child's full stdout byte-for-byte, and no marker is appended. Explicit test case with a 1 KiB stdout.
- [ ] **AC6 — Chunk-size change verified.** The drain-loop uses an 8192-byte chunk (not 256). Grep assertion in code review; not a test-case (behavior-preserving).
- [ ] **AC7 — Clippy + fmt clean.** `cargo clippy -p mika-agent --all-targets` and `cargo fmt --check` pass on the changed file.

## Definition of Done

- All acceptance criteria above pass.
- PR opened against `main` with `Closes mika#1746` in the body.
- CI green: `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`, and the meta-repo `verify-pipeline.sh` Pipeline Artifacts check (per-ticket plan doc present with `## Acceptance criteria` section — this document satisfies that check).
- No changes to `MAX_OUTPUT_LEN` or to mika-qa's skill prompt in this PR.

## References

- **Issue:** senara-solutions/mika#1746 — original ticket with mika-qa's self-account and the Option A / Option B split.
- **Relocated from:** senara-solutions/mika-platform#186 (2026-07-08) — the mp zero-open-issues invariant moved this ticket to mika because the code surface is here.
- **Code:** `crates/mika-agent/src/skills/builtin_handlers.rs::read_with_counter`, `spawn_and_collect`.
- **Parallel:** mika-platform#184 (ask-channel substrate) and mika-platform#185 (mika-dev gate-change) — orthogonal loop fixes, not blocked-by/blocking this ticket.
- **Doctrine:** mika#189 (per-ticket plan doc with `## Acceptance criteria` — enforced by `verify-pipeline.sh` in the Pipeline Artifacts CI job).
- **Prior art:** mika#900 (`spawn_and_collect` progress telemetry — the info-log this plan extends is the #900 log).
