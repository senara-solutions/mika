### Ready-Label Dispatch (MANDATORY — do not skip, do not defer)

When the message starts with `[GitHub] Issue labeled ready on <repo>#<n>`, the operator has set the `ready` label on the ticket — the canonical positive-consent signal for autonomous dispatch.

> **The engine enforces this sequence via the `webhook_ready_label_dispatch` intent-precondition guard (mika#846, #907, #1089, #1173).** The guard requires a `run_claude_pilot` attempt (dispatch via dev-pilot for implementation) OR a `run_claude_pilot_groom` attempt (auto-groom via dev-groom). Ending the turn without calling one of these will cause the engine to reject your `EndTurn` once and re-prompt you. The steps below are a structural contract, not advisory prose.

**Atomic handler (label removal first, then grooming check, then dispatch — per mika#841, #907):**

1. **First**, call `run_gh("issue edit <n> --remove-label ready")` with `repo: "<repo>"` to remove the consent signal. Label-removal-first lets the operator re-add the label to retry if subsequent steps fail.

   **On `run_gh` failure (non-zero exit):** Do NOT call `create_task` or `run_claude_pilot`. Send the operator a `send_message` with the gh stderr and stop the turn — the label is still present, and they can fix permissions and re-add to retry.

2. **Second**, call `run_gh` with args `issue view <n> --json title,body --repo <repo>` to fetch the issue title and body — required input for the grooming check and `create_task`.

3. **Third (GROOMING PRE-FLIGHT — mika#907, mika#996, mika#919)**, scan the fetched issue body for the grooming marker. The bypass predicate is `Plan: docs/plans/` — the substring must include the canonical plan-doc path prefix `docs/plans/` to avoid false positives on the word "Plan:" appearing in prose elsewhere in the issue body. (Engine-level coupled guard: `crates/mika-agent/src/skills/executor.rs::validate_dispatch_readiness`.)

   **If the marker IS found:** Proceed to Step 4 (dispatch via `dev-pilot`).

   **If the marker is NOT found in the issue body (auto-groom path — mika#996):** The ticket is ungroomed. Auto-groom via `dev-groom` skill before dispatching.

   a. Call `create_task` with `reference_url: "https://github.com/<repo>/issues/<n>?phase=groom"`, `label: "groom <repo>#<n>"`, `description: <issue body>`, `source: "self_dev"`. Capture the returned `task_id` as `groom_task_id`. The `?phase=groom` discriminator distinguishes the grooming task from the eventual dispatch task (which uses the canonical URL without the suffix).

   b. **IMMEDIATELY** call `run_claude_pilot_groom` with:
      ```json
      {"skill": "dev-groom", "prompt": "<repo>#<n>", "task_id": "<groom_task_id>"}
      ```
      (mika#1173 — grooming has its own tool; `skill: "dev-groom"` is required by the schema for engine dispatch-class derivation.)

   c. Stop the turn. The grooming task runs in the background; its callback re-enters this session's task loop with the grooming result. **Do not call `send_message` to notify the operator** — auto-grooming is the new default behavior, not an exception.

> **PROHIBITION (mika#1089):** In Steps 1-3, do NOT call `check_task`. The engine enforces per-class dispatch slot availability via `run_claude_pilot`'s deferred-status return path; pre-flight slot-checks are not in this handler's contract. Calling `check_task` with stale task IDs produces false negatives that short-circuit dispatch.

   **On the dev-groom callback (received as a regular post-callback turn):**

   d. Parse the callback result text for the verdict line. The dev-groom skill emits `Verdict: GROOMED` or `Verdict: ESCALATE` as its final line (enforced by the engine's required-suffix-line guard).

   e. **If `Verdict: GROOMED` — re-entry:** Re-enter the Ready-Label Dispatch atomic handler at its top (Step 1 of this section). The handler runs through Steps 1-3 again; the issue body now contains `Plan: docs/plans/` because dev-groom edited it via Phase 5 step 18 of its prompt. The handler advances naturally past the grooming branch and into Step 4 (create_task + run_claude_pilot for `dev-pilot`). The dispatch task uses the canonical `reference_url` (no `?phase=groom` suffix). **Do NOT re-implement create_task + run_claude_pilot inline** — the re-entry mechanism keeps dispatch logic in one place.

   f. **If `Verdict: ESCALATE`:** dev-groom surfaces an architect ESCALATION. Treat as a blocking event: `send_message` to operator with the ESCALATE reason from the callback, mark the groom task `blocked` if applicable, stop the turn. Do NOT auto-dispatch.

   g. **If callback indicates failure (HANDLER CRASH, timeout, etc.) — terminal-semantics rule:**
      - **Retry policy:** retry once, **reusing the same `groom_task_id`** (do NOT call `create_task` again). The retry is `run_claude_pilot_groom({"skill": "dev-groom", "prompt": "<repo>#<n>", "task_id": "<existing groom_task_id>"})`.
      - **Second-crash terminal:** on the second consecutive HANDLER CRASH for the same `groom_task_id`, treat as ESCALATE. `send_message` to operator with both failure reasons concatenated; stop the turn. Do NOT retry a third time.
      - **Tracking:** the failure-count is tracked in the `groom_task_id` task's metadata (`metadata.groom_crash_count`, incremented by the callback handler on each HANDLER CRASH). The check `groom_crash_count >= 2` triggers the terminal path.

4. **Fourth**, call `create_task` with `reference_url: "https://github.com/<repo>/issues/<n>"`, `label: <issue title>`, `description: <issue body>`, and `source: "self_dev"`. `create_task` is idempotent on `reference_url`, so a duplicate webhook reuses an existing `task_id`. Capture the returned `task_id` (UUID).

5. **IMMEDIATELY after Step 4, call `run_claude_pilot`.** No other tool calls permitted between Step 4 and this call. Do not read files, analyze code, plan, summarize, or list "next steps." Call `run_claude_pilot` NOW.

   ```json
   {"skill": "dev-pilot", "prompt": "<repo>#<n>", "task_id": "<UUID from Step 4>"}
   ```

   If `run_claude_pilot` returns a terminal error (`global_dispatch_active`, `task_not_dispatchable`, `dispatch_blocked_by`, `dispatch_limit_exceeded`), do NOT retry. Send the operator a `send_message` naming the rejection cause and stop — the engine guard accepts the attempt as satisfying the dispatch contract.

**GATE: If Step 1 succeeded but you have NOT called `run_claude_pilot` in this turn, call Steps 2–5 immediately — do not end the turn.**

**Other label-add events** (`bug`, `enhancement`, `p1-important`, etc.) — any `[GitHub] Issue labeled <name> on ...` where `<name>` is NOT `ready` — match the Webhook Fallthrough scope rule below: acknowledge, do NOT dispatch.

