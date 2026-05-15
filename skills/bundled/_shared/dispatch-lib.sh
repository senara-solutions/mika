#!/bin/bash
# Shared dispatch library for claude-pilot skills (dev-pilot, dev-groom, etc.)
#
# Single entrypoint: dispatch_claude_pilot (no args — entry command derived from $SKILL)
# Reads JSON from process-inherited stdin (fd 0).
# Sets up worktree, scrubs env, installs EXIT trap, runs claude-pilot,
# and delivers the result via mika callback.
#
# Callers MUST NOT set their own EXIT trap (it would be overwritten silently —
# bash trap is process-scoped, not function-scoped).
#
# One host skill (dev-pilot) owns the `run_claude_pilot` tool with a union-enum
# `skill` parameter. Sibling skills in the self-dev family extend the enum and
# add a case in this lib's skill→entry mapping. Sibling skills do not register
# their own `run_claude_pilot`. See mika#932 for context.

# --- Internal helpers (underscore-prefixed, not part of the API contract) ---

_dispatch_lib_exit_trap() {
    _EXIT_CODE=$?
    # Guard: skip if already delivered or no task ID
    [ "$CALLBACK_SENT" -eq 1 ] && { [ -n "$STDOUT_FILE" ] && rm -f "$STDOUT_FILE"; [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"; rm -f "$TRACE_FILE"; return; }
    [ -z "$TASK_ID" ] && { [ -n "$STDOUT_FILE" ] && rm -f "$STDOUT_FILE"; [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"; rm -f "$TRACE_FILE"; return; }
    # Try to recover result from stdout file if RESULT was never populated.
    if [ -z "$RESULT" ] && [ -n "$STDOUT_FILE" ] && [ -f "$STDOUT_FILE" ]; then
        _RECOVERED_RAW=$(cat "$STDOUT_FILE" 2>/dev/null)
        # Issue #135: extract first JSON line from possible preamble (dotenvx banner)
        _RECOVERED=$(printf '%s\n' "$_RECOVERED_RAW" | grep -m1 '^{' || true)
        : "${_RECOVERED:=$_RECOVERED_RAW}"
        _STATUS=$(printf '%s\n' "$_RECOVERED" | jq -r '.status // empty' 2>/dev/null)
        if [ -n "$_STATUS" ]; then
            RESULT="claude-pilot completed (status: ${_STATUS}, recovered from crash).
Exit code: ${_EXIT_CODE}
Stdout recovered from file."
        fi
    fi
    # Capture stderr tail on crash path BEFORE deleting the file (#104)
    # Scrub secrets from stderr to prevent PAT leakage in callback delivery (mika#903).
    if [ -z "$RESULT" ] && [ -n "$STDERR_FILE" ] && [ -f "$STDERR_FILE" ]; then
        _STDERR_TAIL=$(tail -c 10000 "$STDERR_FILE" 2>/dev/null | _scrub_secrets_from_output)
        if [ -n "$_STDERR_TAIL" ]; then
            RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result.

Stderr (last 10KB):
${_STDERR_TAIL}"
        fi
    fi
    # Clean up temp files AFTER capture
    [ -n "$STDOUT_FILE" ] && rm -f "$STDOUT_FILE"
    [ -n "$STDERR_FILE" ] && rm -f "$STDERR_FILE"
    if [ -z "$RESULT" ]; then
        RESULT="HANDLER CRASH (exit code ${_EXIT_CODE}). Script failed before building result."
    fi
    # --- Diagnostic trace tail (mika#887) ---
    # Scrub secrets from trace tail to prevent PAT leakage in callback delivery (mika#903).
    if [ -f "$TRACE_FILE" ]; then
        case "$RESULT" in
            "HANDLER CRASH"*)
                # Crash path: append trace tail, preserve file for forensics
                _TRACE_TAIL=$(tail -50 "$TRACE_FILE" 2>/dev/null \
                    | _scrub_secrets_from_output \
                    | sed 's/^/    /')
                if [ -n "$_TRACE_TAIL" ]; then
                    RESULT="${RESULT}

Trace tail (last 50 lines):
${_TRACE_TAIL}"
                fi
                ;;
            *)
                # Success/recovery path: clean up trace file
                rm -f "$TRACE_FILE"
                ;;
        esac
    fi
    # Issue #138: best-effort PR URL discovery on crash recovery path.
    if [ -n "$REPO" ] && [ -n "$BRANCH" ]; then
        _PR_URL=$(gh pr list --repo "senara-solutions/$REPO" --head "$BRANCH" --json url --jq '.[0].url' 2>/dev/null || true)
        if [ -n "$_PR_URL" ]; then
            RESULT="${RESULT}
PR: ${_PR_URL}"
        fi
    fi
    RESULT=$(printf '%s' "$RESULT" | head -c 92000)
    set +e
    if [ -n "$AGENT" ]; then
        mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
    else
        mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
    fi
    CALLBACK_SENT=1
    set -e
}

_parse_input_json() {
    # Read input JSON from stdin
    INPUT=$(cat)

    # Parse callback fields injected by the long-running executor
    TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.__mika_task_id // empty')
    AGENT=$(printf '%s\n' "$INPUT" | jq -r '.__mika_agent // empty')

    if [ -z "$TASK_ID" ]; then
        echo "Error: no __mika_task_id in input (not running as long-running handler?)" >&2
        exit 1
    fi

    # Parse user-provided fields
    SKILL=$(printf '%s\n' "$INPUT" | jq -r '.skill // empty')
    PROMPT=$(printf '%s\n' "$INPUT" | jq -r '.prompt // empty')
    USER_TASK_ID=$(printf '%s\n' "$INPUT" | jq -r '.task_id // empty')
    DRY_RUN=$(printf '%s\n' "$INPUT" | jq -r '.dry_run // empty')
    ITERATION_CTX=$(printf '%s\n' "$INPUT" | jq -r '.iteration_context // empty')
}

_validate_inputs() {
    # Structured validation errors (#955): emit parseable JSON to stderr so that
    # the exit trap delivers an actionable error (not a generic crash string).
    # Downstream consumers (mika-dev's callback turn) can `jq` the result to
    # distinguish "LLM forgot a required field" (retry-safe) from "handler bug" (escalate).
    if [ -z "$SKILL" ]; then
        printf 'DISPATCH_VALIDATION_ERROR: {"error":"missing_required_field","field":"skill","valid_values":["dev-pilot","dev-groom"],"reason":"The skill field is required but was not provided in the tool call."}\n' >&2
        exit 1
    fi

    # Skill validation is handled by the case switch in dispatch_claude_pilot
    # which derives ENTRY_COMMAND from SKILL. Unknown skills exit 1 there.

    if [ -z "$PROMPT" ]; then
        printf 'DISPATCH_VALIDATION_ERROR: {"error":"missing_required_field","field":"prompt","reason":"The prompt field is required but was not provided in the tool call."}\n' >&2
        exit 1
    fi

    if [ -z "$USER_TASK_ID" ]; then
        printf 'DISPATCH_VALIDATION_ERROR: {"error":"missing_required_field","field":"task_id","reason":"The task_id field is required but was not provided in the tool call."}\n' >&2
        exit 1
    fi

    # Reject non-UUID task_id at the handler boundary (#958)
    if ! printf '%s' "$USER_TASK_ID" | grep -qiE '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'; then
        # Sanitize value for JSON safety: escape backslashes and double-quotes.
        _sanitized_tid=$(printf '%s' "$USER_TASK_ID" | sed 's/\\/\\\\/g; s/"/\\"/g' | head -c 200)
        printf 'DISPATCH_VALIDATION_ERROR: {"error":"invalid_uuid","field":"task_id","value":"%s","reason":"task_id must be a valid UUID (36-char format like 15383984-a3e7-41bf-ac6f-630ba9a89d63). Got a non-UUID string — this is likely an unsubstituted template placeholder."}\n' "$_sanitized_tid" >&2
        exit 1
    fi
}

_scrub_secrets_from_output() {
    # Redact known secret patterns from diagnostic output before callback delivery (mika#903).
    # Covers: env var assignments (GH_APP_TOKEN=..., MIKA_*=..., GH_TOKEN=...),
    #         fine-grained PATs (github_pat_*), classic PATs (ghp_*),
    #         GitHub App installation tokens (ghs_*), and user-to-server OAuth tokens (ghu_*).
    sed -E 's/(GH_APP_TOKEN|GH_TOKEN|MIKA_[A-Z_]*TOKEN|MIKA_[A-Z_]*API_KEY|MIKA_[A-Z_]*PRIVATE_KEY)=[^ ]*/\1=<REDACTED>/g' \
        | sed -E 's/github_pat_[A-Za-z0-9_]+/<REDACTED_PAT>/g' \
        | sed -E 's/gh[spu]_[A-Za-z0-9_]+/<REDACTED_TOKEN>/g'
}

_verify_and_write_body_callout() {
    # Post-flight body-callout recovery (mika#1123): detect dev-groom drift where
    # the plan is committed and pushed but the issue body never received the
    # canonical callout block. If missing, write a recovery callout that surfaces
    # the drift without fabricating an architect verdict.
    #
    # Dual-write documentation: body callouts can be written by two paths:
    #   (1) the LLM in dev-groom step 18 (organic, passes dispatch gate)
    #   (2) this structural recovery (partial, does NOT pass dispatch gate)
    local repo="$1" issue_num="$2" worktree_dir="$3" branch="$4"

    # 1. Fetch current issue body
    local current_body
    current_body=$(gh issue view "$issue_num" --repo "senara-solutions/$repo" \
        --json body -q '.body' 2>/dev/null) || return 0

    # 2. Check if all three callout signals are already present
    #    (mirrors check_grooming_markers() in executor.rs — Pin B)
    local has_branch has_plan has_verdict
    has_branch=$(printf '%s' "$current_body" | grep -cF '> - **Branch:**' || true)
    has_plan=$(printf '%s' "$current_body" | grep -cF 'docs/plans/' || true)
    has_verdict=$(printf '%s' "$current_body" | grep -cE 'second-pass \(GROOMED\)|second-pass \(READY, paraphrased GROOMED' || true)

    if [ "$has_branch" -gt 0 ] && [ "$has_plan" -gt 0 ] && [ "$has_verdict" -gt 0 ]; then
        return 0  # All present, nothing to do
    fi

    # 3. Find the plan file on the branch — scoped to issue number first
    local plan_file
    plan_file=$(find "$worktree_dir/docs/plans" -name "*-${issue_num}-*-plan.md" -size +500c \
        2>/dev/null | sort -r | head -1)

    # Fall back to any plan file if issue-scoped search finds nothing
    if [ -z "$plan_file" ]; then
        plan_file=$(find "$worktree_dir/docs/plans" -name "*-plan.md" -size +500c \
            2>/dev/null | sort -r | head -1)
    fi

    [ -n "$plan_file" ] || return 0  # No valid plan file — can't write callout

    local plan_relpath="${plan_file#"$worktree_dir/"}"
    local head_sha
    head_sha=$(git -C "$worktree_dir" rev-parse --short HEAD 2>/dev/null)

    # 4. Construct recovery callout.
    #    IMPORTANT: The grooming-history line does NOT contain "second-pass (GROOMED)"
    #    because we cannot verify the architect actually issued that verdict from
    #    branch state alone. This callout surfaces the drift for operator visibility
    #    but does NOT pass the dispatch gate — the operator must verify and dispatch
    #    manually (or re-run dev-groom which will write the organic callout).
    local callout_block
    callout_block=$(cat <<CALLOUT_EOF
> - **Branch:** \`${branch}\`
> - **Plan:** \`${plan_relpath}\` (committed on branch @ \`${head_sha}\`)
> - **Grooming history:** body callout recovered by post-flight (mika#1123) — architect verdict not verified, operator dispatch required
CALLOUT_EOF
    )

    # 5. Prepend callout to existing body and write
    local new_body
    new_body=$(printf '%s\n\n%s' "$callout_block" "$current_body")
    local tmpfile
    tmpfile=$(mktemp /tmp/body-callout-recover-XXXXXX.md)
    printf '%s' "$new_body" > "$tmpfile"

    if gh issue edit "$issue_num" --repo "senara-solutions/$repo" \
        --body-file "$tmpfile" 2>/dev/null; then
        echo "body_callout_drift_recovered: wrote missing callout to $repo#$issue_num (verdict NOT fabricated — operator dispatch required)" >&2
    else
        echo "WARN: body_callout_drift_recovery_failed for $repo#$issue_num" >&2
    fi
    rm -f "$tmpfile"
}

_setup_gh_auth() {
    # Suppress xtrace to prevent PAT from appearing in trace logs (mika#903).
    { set +x; } 2>/dev/null
    # GitHub App installation token for gh CLI.
    # See mika#520 for context on why we check GH_TOKEN before calling gh auth login.
    if [ -z "${GH_TOKEN:-}" ]; then
        GH_APP_TOKEN=$(mika ${AGENT:+--agent "$AGENT"} token github 2>/dev/null)
        if [ -n "$GH_APP_TOKEN" ]; then
            echo "$GH_APP_TOKEN" | gh auth login --with-token 2>/dev/null
            unset GH_APP_TOKEN
            gh auth switch --user "mika-platform-bot[bot]" 2>/dev/null || true
        else
            echo "WARNING: mika token github failed — gh CLI will fall back to host credentials" >&2
        fi
    fi
    # Re-enable xtrace (was set by dispatch_claude_pilot before calling us).
    # GH_APP_TOKEN is already unset above, so set -x won't leak it.
    set -x
}

_scrub_env() {
    unset MIKA_ANTHROPIC_API_KEY MIKA_INTERNAL_TOKEN MIKA_OPENAI_API_KEY MIKA_BRAVE_API_KEY
}

_set_up_worktree() {
    # --- Parse repo#number format ---
    # Matches: mika#214, mika-skills#8, mika-cloud#50
    REPO=""
    ISSUE_NUM=""
    if printf '%s' "$PROMPT" | grep -qE '^[a-zA-Z0-9_-]+#[0-9]+$'; then
        REPO=$(printf '%s' "$PROMPT" | sed 's/#.*//')
        ISSUE_NUM=$(printf '%s' "$PROMPT" | sed 's/.*#//')
    fi

    if [ -n "$REPO" ] && [ -n "$ISSUE_NUM" ]; then
        # --- repo#number mode: derive everything from the issue ---
        LOG_ID="$TASK_ID"

        # Validate repo directory exists (mika-platform itself IS PLATFORM_DIR)
        if [ "$REPO" = "$PLATFORM_REPO_NAME" ]; then
            SUB_REPO_DIR="$PLATFORM_DIR"
        else
            SUB_REPO_DIR="$PLATFORM_DIR/$REPO"
        fi
        if [ ! -d "$SUB_REPO_DIR/.git" ] && ! [ -f "$SUB_REPO_DIR/.git" ]; then
            echo "Error: $SUB_REPO_DIR is not a git repository" >&2
            exit 1
        fi

        # Fetch issue — validates it exists and is open, gets labels + title + body
        ISSUE_JSON=$(gh issue view "$ISSUE_NUM" --repo "senara-solutions/$REPO" --json state,title,labels,body 2>/dev/null) || {
            echo "Error: Issue #${ISSUE_NUM} not found in senara-solutions/${REPO}. Aborting." >&2
            exit 1
        }

        ISSUE_STATE=$(printf '%s' "$ISSUE_JSON" | jq -r '.state')
        if [ "$ISSUE_STATE" = "CLOSED" ]; then
            # Auto-skip: PR merge (or any other close) raced ahead of the webhook-triggered
            # dispatch enqueue. This is an expected race, not a handler bug — deliver a
            # structured skip result via the canonical _deliver_callback() helper so
            # mika-dev's callback turn can recognise it as a no-op and the audit dashboard
            # can filter on status: "auto_skipped". See mika#988 for the failure mode.
            # Position on human-closes vs PR-closes: treated identically — see plan §Scope.
            #
            # Auto-skip rationale (mika#988):
            # On 2026-05-06 the autonomous loop stalled ~7h because this branch previously
            # did `exit 1`, causing the EXIT trap to wrap the error as HANDLER CRASH.
            # mika-dev read the crash envelope, posted a confirmation question, and idled.
            # The correct exit semantics for foreseeable races: exit 0 + structured JSON
            # delivered via _deliver_callback(). Reserve exit 1 for actual handler bugs.
            # Symptom sessions: callback-476caa1d-ef6d-4bac-a60c-a3c78f9a342d (failure),
            # 40a52d43-f186-4175-9c86-b998aafcf4bb (drift).
            RESULT=$(printf '{"status":"auto_skipped","reason":"issue_closed","issue":"senara-solutions/%s#%s","note":"Issue was already closed before dispatch fired. Presumed handled."}' "$REPO" "$ISSUE_NUM")
            _deliver_callback
            exit 0
        fi

        # Branch-name derivation is centralized in mika-platform/scripts/derive-branch-name.
        # See senara-solutions/mika-platform#58 for context on the drift class this eliminates.
        ISSUE_BODY=$(printf '%s' "$ISSUE_JSON" | jq -r '.body // empty')
        ISSUE_TITLE=$(printf '%s' "$ISSUE_JSON" | jq -r '.title')
        LABELS=$(printf '%s' "$ISSUE_JSON" | jq -r '[.labels[].name] | join(",")' 2>/dev/null)

        BRANCH=$("$PLATFORM_DIR/scripts/derive-branch-name" \
            --title "$ISSUE_TITLE" \
            --issue "$ISSUE_NUM" \
            --labels "$LABELS" \
            --body-callout "$ISSUE_BODY")

        # Sync main before branching to avoid stale worktrees.
        git -C "$SUB_REPO_DIR" fetch origin main 2>/dev/null || true

        # Worktree path is centralized in mika-platform/scripts/derive-worktree-path
        WORKTREE_DIR=$("$PLATFORM_DIR/scripts/derive-worktree-path" --branch "$BRANCH" --repo "$REPO")

        # Reuse existing worktree if valid
        if [ -d "$WORKTREE_DIR" ] && git -C "$WORKTREE_DIR" rev-parse --git-dir >/dev/null 2>&1; then
            git -C "$WORKTREE_DIR" checkout "$BRANCH" 2>/dev/null || true
        else
            git -C "$SUB_REPO_DIR" worktree remove --force "$WORKTREE_DIR" 2>/dev/null || true
            if ! git -C "$SUB_REPO_DIR" worktree add -b "$BRANCH" "$WORKTREE_DIR" origin/main 2>/dev/null; then
                git -C "$SUB_REPO_DIR" worktree add "$WORKTREE_DIR" "$BRANCH"
            fi
        fi

        # Rebase-or-abort guard
        BEHIND=$(git -C "$WORKTREE_DIR" rev-list --count HEAD..origin/main 2>/dev/null || echo 0)
        if [ "$BEHIND" -gt 0 ]; then
            if git -C "$WORKTREE_DIR" rebase origin/main 2>/dev/null; then
                echo "Rebased ${BRANCH} onto origin/main (${BEHIND} commits caught up)." >&2
            else
                CONFLICTS=$(git -C "$WORKTREE_DIR" diff --name-only --diff-filter=U 2>/dev/null | tr '\n' ' ')
                git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
                RESULT="STATUS=REBASE_CONFLICT
Branch ${BRANCH} is ${BEHIND} commits behind origin/main.
Conflicted files: ${CONFLICTS:-<unable to capture>}
Resolve manually before re-dispatching ${REPO}#${ISSUE_NUM}."
                exit 1
            fi
        fi

        # Copy gitignored .claude/ config into worktree (relay + permissions only)
        mkdir -p "$WORKTREE_DIR/.claude"
        cp "$PLATFORM_DIR/.claude/claude-pilot.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
        cp "$PLATFORM_DIR/.claude/settings.local.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true

        CWD_ARGS="--cwd $WORKTREE_DIR"
        if [ -f "$WORKTREE_DIR/.claude/claude-pilot.json" ]; then
            CWD_ARGS="$CWD_ARGS --relay-config $WORKTREE_DIR/.claude/claude-pilot.json"
        elif [ -f "$PLATFORM_DIR/.claude/claude-pilot.json" ]; then
            CWD_ARGS="$CWD_ARGS --relay-config $PLATFORM_DIR/.claude/claude-pilot.json"
        fi

        # The prompt becomes the QUALIFIED issue reference.
        # Must use `${REPO}#${ISSUE_NUM}` (not bare `#${ISSUE_NUM}`) — see mika#138.
        PROMPT="${REPO}#${ISSUE_NUM}"

        # Append iteration context if provided
        if [ -n "$ITERATION_CTX" ]; then
            ITERATION_CTX=$(printf '%s' "$ITERATION_CTX" | head -c 4096)
            PROMPT=$(printf '%s#%s\n\nITERATION CONTEXT:\n%s' "$REPO" "$ISSUE_NUM" "$ITERATION_CTX")
        fi

        # Save pre-run HEAD SHA for post-flight diff check
        PRE_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
    else
        # --- Free-text mode: pass prompt as-is, no worktree ---
        PRE_RUN_HEAD=""
        LOG_ID="$TASK_ID"
        CWD_ARGS="--cwd $PLATFORM_DIR"

        if [ -n "$ITERATION_CTX" ]; then
            echo "Warning: iteration_context provided but prompt is not in repo#number format — ignoring" >&2
        fi
    fi
}

_handle_dry_run() {
    if [ "$DRY_RUN" = "true" ] || [ "$DRY_RUN" = "1" ]; then
        if [ -n "$REPO" ] && [ -n "$ISSUE_NUM" ]; then
            jq -n --arg repo "$REPO" --argjson issue "$ISSUE_NUM" --arg branch "$BRANCH" \
                --arg worktree "$WORKTREE_DIR" --arg prompt "$PROMPT" \
                --arg entry_command "$ENTRY_COMMAND" \
                '{dry_run:true, repo:$repo, issue:$issue, branch:$branch, worktree_dir:$worktree, prompt:$prompt, entry_command:$entry_command}'
            git -C "$SUB_REPO_DIR" worktree remove --force "$WORKTREE_DIR" 2>/dev/null || true
            PARENT_DIR=$("$PLATFORM_DIR/scripts/derive-worktree-path" --branch "$BRANCH" --no-repo)
            rmdir "$PARENT_DIR" 2>/dev/null || true
        else
            jq -n --arg prompt "$PROMPT" \
                '{dry_run:true, repo:null, issue:null, branch:null, worktree_dir:null, prompt:$prompt}'
        fi
        exit 0
    fi
}

_run_claude_pilot() {
    local ENTRY_COMMAND="$1"

    STDERR_FILE=$(mktemp)
    STDOUT_FILE=$(mktemp)
    # Persistent stderr copy for post-mortem forensics (mika#1097 Step 0-A).
    # The mktemp file above is deleted after callback delivery; this copy persists
    # alongside the claude-pilot log file so operators can inspect it independently.
    PERSISTENT_STDERR="/var/log/claude-pilot/${LOG_ID}.stderr"
    # --trace flag for full event-stream capture (mika#1097 Step 0-B).
    # Enabled via CLAUDE_PILOT_TRACE env var (set per-skill in the case switch below).
    local TRACE_FLAG=""
    if [ "${CLAUDE_PILOT_TRACE:-}" = "1" ] || [ "${CLAUDE_PILOT_TRACE:-}" = "true" ]; then
        TRACE_FLAG="--trace"
    fi
    set +e
    # CWD_ARGS is intentionally word-split (multiple flags)
    # shellcheck disable=SC2086
    claude-pilot --verbose --log-dir --task-id "$LOG_ID" --command "$ENTRY_COMMAND" $TRACE_FLAG $CWD_ARGS -- "$PROMPT" >"$STDOUT_FILE" 2>"$STDERR_FILE"
    PILOT_EXIT=$?
    # Persist stderr to durable file before any processing (mika#1097).
    # Scrub secrets from the persistent copy to prevent durable secret retention (mika#903).
    if [ -s "$STDERR_FILE" ]; then
        mkdir -p "$(dirname "$PERSISTENT_STDERR")" 2>/dev/null || true
        _scrub_secrets_from_output < "$STDERR_FILE" > "$PERSISTENT_STDERR" 2>/dev/null || echo "Warning: failed to persist stderr to $PERSISTENT_STDERR" >&2
    fi
    # Issue #135: extract first JSON-object line from stdout
    PILOT_OUTPUT_RAW=$(cat "$STDOUT_FILE" 2>/dev/null)
    PILOT_OUTPUT=$(printf '%s\n' "$PILOT_OUTPUT_RAW" | grep -m1 '^{' || true)
    : "${PILOT_OUTPUT:=$PILOT_OUTPUT_RAW}"
    rm -f "$STDOUT_FILE"

    # Build result message from structured stdout
    STATUS=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.status // empty' 2>/dev/null)
    SESSION_ID=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.session_id // empty' 2>/dev/null)
    TURNS=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.turns // empty' 2>/dev/null)
    COST=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.cost_usd // empty' 2>/dev/null)
    DURATION=$(printf '%s\n' "$PILOT_OUTPUT" | jq -r '.duration_ms // empty' 2>/dev/null)

    if [ -n "$STATUS" ]; then
        RESULT="claude-pilot completed (status: ${STATUS}).
Session: ${SESSION_ID:-unknown}
Turns: ${TURNS:-unknown}
Cost: \$${COST:-unknown}
Duration: ${DURATION:-unknown}ms"

        if [ "$PILOT_EXIT" -ne 0 ]; then
            RESULT="${RESULT}
Note: process exited with code ${PILOT_EXIT} after session completed — result is valid."
        fi

        # Post-flight diff check: detect zero-commit "success" in repo#number mode.
        if [ -n "$PRE_RUN_HEAD" ] && [ -n "$REPO" ]; then
            POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
            if [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ]; then
                RESULT="PIPELINE FAILURE: claude-pilot exited 0 but HEAD unchanged (pre: ${PRE_RUN_HEAD}, post: ${POST_RUN_HEAD}). Zero new commits produced.

${RESULT}"
            fi
        fi

        # Post-flight plan validation (mika#1033, mika#1032): detect dev-groom
        # drift where the session exits "success" but produced no valid plan file
        # (or only a stub/empty one) and/or never invoked /ce:plan. The plan-file-
        # size check is the primary structural defense; the log-grep for /ce:plan
        # adds root-cause diagnostic specificity. Both checks are merged into a
        # single block and fire before callback delivery.
        if [ "$SKILL" = "dev-groom" ] && [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ]; then
            TODAY_PREFIX=$(date +%Y-%m-%d)
            VALID_PLAN=$(find "$WORKTREE_DIR/docs/plans" -name "${TODAY_PREFIX}-*-plan.md" -size +500c 2>/dev/null | head -1)

            # Check session log for /ce:plan invocation (mika#1032).
            # Broad pattern covers Skill tool call JSON, command strings, etc.
            # Fail-open: if log is unavailable, skip the check with a warning.
            SESSION_LOG="/var/log/claude-pilot/${LOG_ID}.log"
            CE_PLAN_INVOKED=""
            if [ -f "$SESSION_LOG" ] && [ -r "$SESSION_LOG" ]; then
                if grep -qiE 'ce[.:\-_]plan' "$SESSION_LOG" 2>/dev/null; then
                    CE_PLAN_INVOKED="1"
                fi
            else
                echo "Warning: session log not available at $SESSION_LOG — skipping /ce:plan invocation check" >&2
                # Treat as unknown — don't fail on missing log
                CE_PLAN_INVOKED="unknown"
            fi

            if [ -z "$VALID_PLAN" ] && [ "$CE_PLAN_INVOKED" != "1" ]; then
                # Both checks failed: no plan file AND /ce:plan never called
                RESULT="PIPELINE FAILURE: dev-groom produced no valid plan file (no docs/plans/${TODAY_PREFIX}-*-plan.md >500 bytes found) and no /ce:plan invocation detected in session log. Session drifted into executor mode.

${RESULT}"
            elif [ -z "$VALID_PLAN" ]; then
                # Plan file missing but /ce:plan was called (or log unavailable)
                RESULT="PIPELINE FAILURE: dev-groom produced no valid plan file (no docs/plans/${TODAY_PREFIX}-*-plan.md >500 bytes found). Session likely drifted into executor mode.

${RESULT}"
            elif [ "$CE_PLAN_INVOKED" != "1" ] && [ "$CE_PLAN_INVOKED" != "unknown" ]; then
                # Valid plan file exists but /ce:plan was never invoked — LLM
                # wrote a plan-shaped file without running the planning skill
                RESULT="PIPELINE FAILURE: dev-groom produced a plan file but no /ce:plan invocation detected in session log. Plan may not have been generated through the planning skill.

${RESULT}"
            fi
        fi

        # Post-flight body-callout verification (mika#1123): detect dev-groom drift
        # where the plan is committed and pushed but the issue body never received the
        # canonical callout block. If missing, write a recovery callout that surfaces
        # the drift without fabricating an architect verdict.
        if [ "$SKILL" = "dev-groom" ] && [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] && [ -n "$REPO" ] && [ -n "$ISSUE_NUM" ]; then
            _verify_and_write_body_callout "$REPO" "$ISSUE_NUM" "$WORKTREE_DIR" "$BRANCH"
        fi

        # Issue #138: Discover actual PR URL from the branch
        if [ -n "$REPO" ] && [ -n "$BRANCH" ]; then
            PR_URL=$(gh pr list --repo "senara-solutions/$REPO" --head "$BRANCH" --json url --jq '.[0].url' 2>/dev/null || true)
            if [ -n "$PR_URL" ]; then
                RESULT="${RESULT}
PR: ${PR_URL}"
            fi
        fi
    elif [ "$PILOT_EXIT" -eq 0 ]; then
        RESULT="claude-pilot completed (exit 0) but output was not structured JSON.

Stdout:
${PILOT_OUTPUT_RAW}"
    else
        RESULT="Log path: /var/log/claude-pilot/${LOG_ID}.log

claude-pilot FAILED (exit code ${PILOT_EXIT}).

Stdout:
${PILOT_OUTPUT_RAW}"
    fi

    # Append stderr tail for debugging context (last 10KB)
    if [ -s "$STDERR_FILE" ]; then
        STDERR_TAIL=$(tail -c 10000 "$STDERR_FILE" | _scrub_secrets_from_output)
        RESULT="${RESULT}

Logs (last 10KB):
${STDERR_TAIL}"
    fi
    rm -f "$STDERR_FILE"

    # Truncate to ~90KB to stay within the 100KB callback limit
    RESULT=$(printf '%s' "$RESULT" | head -c 92000)
}

_deliver_callback() {
    set +e
    if [ -n "$AGENT" ]; then
        mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
    else
        mika ask --task-id "$TASK_ID" --task-complete -- "$RESULT"
    fi
    CALLBACK_EXIT=$?
    CALLBACK_SENT=1
    # Success path: clean up trace file (mika#887)
    rm -f "$TRACE_FILE"
    set -e

    if [ "$CALLBACK_EXIT" -ne 0 ]; then
        echo "ERROR: callback delivery failed (exit $CALLBACK_EXIT) for task $TASK_ID" >&2
    fi
}

_detect_plan_on_branch() {
    # Plan-on-branch detection (mika#1074): When the issue body contains a groomed
    # plan callout, override ENTRY_COMMAND from "/mika" to "/ce:work <path>".
    # This eliminates the narrate-then-exit failure class — the model no longer
    # needs to "decide" to invoke /ce:work because the entry command does it directly.
    #
    # Only applies to dev-pilot skill. dev-groom has its own entry command.
    # Falls back silently (no-op) when any precondition fails.

    # Guard: only override for dev-pilot
    [ "$SKILL" = "dev-pilot" ] || return 0

    # Guard: need an issue body to parse
    [ -n "$ISSUE_BODY" ] || return 0

    # Guard: need a worktree directory for file validation
    [ -n "$WORKTREE_DIR" ] || return 0

    # Extract plan path from the callout pattern:
    #   > - **Plan:** `docs/plans/<filename>.md` (committed on branch @ <sha>)
    # The pattern requires `docs/plans/` prefix to avoid false positives on
    # prose containing "Plan:" (consistent with self-dev bypass predicate).
    local PLAN_PATH
    PLAN_PATH=$(printf '%s\n' "$ISSUE_BODY" | grep -oP '> - \*\*Plan:\*\* `\Kdocs/plans/[^`]+' | head -1)

    [ -n "$PLAN_PATH" ] || return 0

    # Validate the plan file exists in the worktree
    if [ -f "$WORKTREE_DIR/$PLAN_PATH" ]; then
        ENTRY_COMMAND="/ce:work $PLAN_PATH"
        echo "Plan-on-branch detected: overriding entry command to '/ce:work $PLAN_PATH'" >&2
    else
        echo "Plan-on-branch callout found but file not in worktree: $WORKTREE_DIR/$PLAN_PATH — falling back to /mika" >&2
    fi
}

# --- Public API ---

# Single entrypoint. No args — entry command is derived from the $SKILL field
# in the input JSON via the case switch below.
# Reads JSON from process stdin (fd 0) — inherited from the calling handler script.
# Sets up worktree, scrubs env, invokes relay, installs EXIT trap, runs claude-pilot.
# Delivers result via callback when complete.
dispatch_claude_pilot() {
    # --- Diagnostic trace (mika#887) ---
    TRACE_FILE="/tmp/dev-pilot-trace-$$.log"
    # Restrict trace file to owner-only (0600) to prevent local users from reading
    # secrets that may appear in the trace before _setup_gh_auth's set+x guard (mika#903).
    _umask_prev=$(umask)
    umask 077
    exec 9>>"$TRACE_FILE" 2>/dev/null || exec 9>/dev/null
    umask "$_umask_prev"
    BASH_XTRACEFD=9
    set -x

    # Ensure ~/.local/bin is in PATH (mika CLI needed for callback delivery)
    export PATH="$HOME/.local/bin:$PATH"

    # Dependency checks
    command -v jq >/dev/null 2>&1 || { echo "Error: jq is required but not installed" >&2; exit 1; }
    command -v mika >/dev/null 2>&1 || { echo "Error: mika CLI is required but not in PATH" >&2; exit 1; }
    command -v claude-pilot >/dev/null 2>&1 || { echo "Error: claude-pilot CLI is required but not in PATH" >&2; exit 1; }

    # mika-platform root — base for sub-repo resolution
    PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
    PLATFORM_DIR=$(cd "$PLATFORM_DIR" 2>/dev/null && pwd -P) || PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
    PLATFORM_REPO_NAME=$(basename "$PLATFORM_DIR")

    # Initialize callback guard
    CALLBACK_SENT=0

    _parse_input_json

    # Install EXIT trap for crash-recovery callback delivery
    trap '_dispatch_lib_exit_trap' EXIT

    _validate_inputs

    # SIBLING SKILL DISPATCH MAPPING (mika#932)
    # Each arm maps a sibling skill to its slash-command entry point.
    # Adding a sibling requires:
    #   1. Add a new arm here.
    #   2. Widen dev-pilot/tools.json `skill.enum` to include the new value.
    #   3. Update self-dev/system_prompt.md to teach mika-dev when to dispatch.
    # Threshold for refactor: if N>5 siblings, escalate to skill-scoped tool
    # registries (Option C from mika#932). Until then, this case switch is
    # the contract.
    local ENTRY_COMMAND
    case "$SKILL" in
      dev-pilot)  ENTRY_COMMAND="/mika" ;;
      dev-groom)
        ENTRY_COMMAND="/mika-groom-ticket"
        # Early-exit guard (mika#1097 Layer B): dev-groom sessions MUST produce
        # tool calls (at minimum: gh issue view, git worktree, /ce:plan, mika ask).
        # If the session exits "success" with fewer than this threshold, claude-pilot
        # re-prompts once; a second early-exit emits early_exit_zero_action.
        # Calibrated from the 2026-05-13 incident: failures had 0 tool calls;
        # successful grooming sessions have 15-40+.
        export CLAUDE_PILOT_MIN_TOOL_CALLS="${CLAUDE_PILOT_MIN_TOOL_CALLS:-3}"
        ;;
      *) echo "Unknown skill: $SKILL" >&2; exit 1 ;;
    esac
    _setup_gh_auth
    _scrub_env
    _set_up_worktree
    _detect_plan_on_branch
    _handle_dry_run
    _run_claude_pilot "$ENTRY_COMMAND"
    _deliver_callback
}
