#!/bin/bash
# Shared dispatch library for claude-pilot skills (dev-pilot, dev-groom, etc.)
#
# Single entrypoint: dispatch_claude_pilot (no args — entry command derived from $SKILL)
# Reads JSON from process-inherited stdin (fd 0).
# Sets up worktree, scrubs env, installs EXIT trap, runs claude-pilot,
# and delivers the result via mika callback.
#
# Callers MUST NOT set their own EXIT or TERM trap (they would be overwritten
# silently — bash trap is process-scoped, not function-scoped).
#
# Cancel discriminator protocol (mika#749):
#   Reason file: /tmp/mika-cancel-reason-{PID}
#   Writer (cancel-time): cancel_task_and_kill pre-writes STATUS=CANCELLED_BY_OPERATOR
#   Writer (signal-time): TERM trap self-writes STATUS=CANCELLED_BY_SIGNAL (if absent)
#   Reader (exit-time): EXIT trap reads the file and prefixes the callback envelope
#   Consumer: self-dev-callback recognizes STATUS=CANCELLED_BY_* and skips retry
#
# Per-skill tool ownership (mika#1173 restored after the prompt-only design
# regressed 5 times since #934): each dispatch skill registers its OWN tool —
# dev-pilot owns `run_claude_pilot` (skill enum: ["dev-pilot"], entry /mika),
# dev-groom owns `run_claude_pilot_groom` (skill enum: ["dev-groom"], entry
# /mika-groom-ticket). Both handlers source this lib and call
# dispatch_claude_pilot; the case switch on $SKILL routes to the right entry
# command. The skill field stays required on both tools for engine
# dispatch-class derivation (executor.rs derive_dispatch_class).

# --- Internal helpers (underscore-prefixed, not part of the API contract) ---

# mika#749: TERM trap writes cancel discriminator before exit.
# Convention: reason file at /tmp/mika-cancel-reason-$$ (PID-based).
# cancel_task pre-writes CANCELLED_BY_OPERATOR before SIGTERM; this trap
# writes CANCELLED_BY_SIGNAL only if no reason file exists yet (the "if
# not exists" check ensures operator pre-write wins the race).
_dispatch_lib_term_trap() {
    if [ ! -e "/tmp/mika-cancel-reason-$$" ]; then
        echo "STATUS=CANCELLED_BY_SIGNAL" > "/tmp/mika-cancel-reason-$$" 2>/dev/null || true
    fi
    exit 143
}

_dispatch_lib_exit_trap() {
    _EXIT_CODE=$?
    # Cleanup fuzzy-match side-channel tmpfile (mika#1272)
    rm -f "${_DISPOSITION_FUZZY_FILE:-}" 2>/dev/null
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
    # --- Cancel discriminator envelope prefix (mika#749) ---
    # Read the reason file written by cancel_task (CANCELLED_BY_OPERATOR) or
    # the TERM trap (CANCELLED_BY_SIGNAL). Prefix the RESULT so the consumer
    # (mika-dev callback parser) sees the STATUS= line first.
    _CANCEL_REASON=""
    if [ -f "/tmp/mika-cancel-reason-$$" ]; then
        _CANCEL_REASON=$(cat "/tmp/mika-cancel-reason-$$" 2>/dev/null || true)
        rm -f "/tmp/mika-cancel-reason-$$" 2>/dev/null || true
    fi
    if [ -n "$_CANCEL_REASON" ]; then
        RESULT="${_CANCEL_REASON}

Original exit code: ${_EXIT_CODE}
${RESULT}"
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

# mika#1414: Pre-rebase worktree cleanup for the resume path.
#
# On a resume dispatch _set_up_worktree() reuses an existing worktree, then
# rebases it onto origin/main. `git rebase` refuses to run on a dirty tree
# (`error: cannot rebase: You have unstaged changes` → STATUS=REBASE_CONFLICT
# with `Rebase failure mode: other`, no real conflict), re-blocking the task
# with no recovery path (confirmed n=2 on 2026-06-05: mika#1255, mika#1381).
# This helper guarantees a clean tree before the rebase, in three tiers:
#
#   1. Abort any half-finished rebase left by a killed prior dispatch — a
#      rebase-in-progress state would make the stash below fail and re-trigger
#      the exact crash this fixes.
#   2. Surgically reset dispatch-lib-owned scaffold/ephemeral paths to HEAD.
#      These are re-copied / re-derived post-rebase anyway, so resetting them
#      costs nothing and keeps them out of the operator-recovery stash. The
#      `.claude/commands/` reset covers the dominant case: `make deploy` writing
#      a stale mika.md into worktree working trees (modified TRACKED file).
#      Subsumes the mika#1301 (.iterate/, groom-verdict-trail.log) and mika#1311
#      (docs/plans/) surgical resets that previously lived inline.
#   3. Blanket fallback: if any residue survives the surgical resets it is
#      genuinely unexpected (crash leftovers, new untracked files deploy added).
#      Stash it as a safety net — capturing the IMMUTABLE stash commit SHA and
#      logging a self-contained recovery command — then hard-reset + clean so
#      the rebase precondition holds. The stash is operator-recoverable via
#      `git -C <worktree> stash list` (its message embeds the task id +
#      timestamp) — this is the durable recovery path; the stderr echo is a
#      convenience and does NOT land in /var/log/claude-pilot/<id>.stderr (that
#      file captures only the later claude-pilot subprocess stderr). `clean -fd`
#      omits -x so it never deletes gitignored worktree state (.claude/*.local.json,
#      .claude/worktrees, scheduled_tasks.lock); the .claude config files are
#      re-copied from $PLATFORM_DIR post-rebase regardless.
#
# Args: $1 = worktree dir (defaults to $WORKTREE_DIR). Reads $LOG_ID for the
# stash label. Sets RESUME_CLEANUP_STASH to the stash SHA when one is created
# (empty otherwise). Returns 1 without touching anything if $wt is not a worktree.
_clean_worktree_for_rebase() {
    local wt="${1:-$WORKTREE_DIR}"
    RESUME_CLEANUP_STASH=""

    # Guard: refuse destructive cleanup on an invalid target. `git -C ""` silently
    # operates on the dispatch process CWD (a live checkout), so an empty/unset $wt
    # would point `reset --hard` / `clean -fd` at the wrong tree. Not reachable on
    # the live path (WORKTREE_DIR is always derived first) but the helper is a
    # sourceable, destructive primitive — fail closed.
    if [ -z "$wt" ] || ! git -C "$wt" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        echo "dispatch-lib: _clean_worktree_for_rebase got an invalid worktree ('${wt}'); refusing destructive cleanup" >&2
        return 1
    fi

    # Tier 1: abort any half-finished rebase (hardening). stdout is suppressed
    # along with stderr throughout — dispatch-lib reserves stdout for the RESULT
    # payload, and `reset --hard` / `clean -fd` below print to stdout.
    git -C "$wt" rebase --abort >/dev/null 2>&1 || true

    # Tier 2: surgical resets of dispatch-lib-owned scaffold/ephemeral paths.
    git -C "$wt" checkout -- .claude/groom-verdict-trail.log 2>/dev/null || true
    rm -rf "$wt/.iterate" 2>/dev/null || true
    git -C "$wt" checkout HEAD -- docs/plans/ 2>/dev/null || true
    git -C "$wt" checkout HEAD -- .claude/commands/ 2>/dev/null || true

    # Tier 3: blanket fallback for genuinely-unexpected residue.
    if [ -n "$(git -C "$wt" status --porcelain 2>/dev/null)" ]; then
        local stash_msg
        stash_msg="dispatch-lib-resume-cleanup-${LOG_ID:-unknown}-$(date -u +%Y%m%dT%H%M%SZ)"
        if git -C "$wt" stash push --include-untracked -m "$stash_msg" >/dev/null 2>&1; then
            # Capture the IMMUTABLE stash commit SHA — stash@{0} shifts as other
            # worktrees push/pop on the shared stash stack. Use `--verify --quiet`:
            # plain `rev-parse 'stash@{0}'` on a missing ref exits non-zero but
            # echoes the literal string "stash@{0}" to stdout, which `|| true`
            # would capture as a bogus handle when `stash push` reported success
            # yet created no entry (e.g. nothing actually stashable). --verify
            # --quiet prints nothing and exits non-zero in that case → empty.
            RESUME_CLEANUP_STASH=$(git -C "$wt" rev-parse --verify --quiet 'stash@{0}' 2>/dev/null || true)
            echo "dispatch-lib: resume-cleanup stashed dirty worktree before rebase → stash ${RESUME_CLEANUP_STASH:-<unknown>} (msg: ${stash_msg}); recover with: git -C ${wt} stash apply ${RESUME_CLEANUP_STASH:-<sha>}" >&2
        else
            echo "dispatch-lib: resume-cleanup found nothing to stash or stash errored; proceeding with hard reset" >&2
        fi
        # Belt-and-suspenders: ensure a clean tree even if the stash captured
        # nothing (e.g. unmerged paths). No -x, so gitignored config survives.
        git -C "$wt" reset --hard HEAD >/dev/null 2>&1 || true
        git -C "$wt" clean -fd >/dev/null 2>&1 || true
    fi
}

# Seed the worktree's .claude/commands/ with the meta-repo orchestration slash
# commands the inner Claude Code session may invoke (/mika-groom-ticket,
# /mika-revise-plan, etc.). The pilot runs with `--cwd "$WORKTREE_DIR"`, and
# Claude Code discovers project commands from <cwd>/.claude/commands/, so those
# commands must physically exist there or they arrive as raw text and the LLM
# improvises (mika#1173).
#
# The naive `cp -r "$PLATFORM_DIR/.claude/commands" "$WORKTREE_DIR/.claude/"`
# this replaces caused two regressions (mika#1415), both enforced against here:
#
#   1. NEVER overwrite a command the worktree's branch already tracks. The mika
#      sub-repo ships its OWN polymorphic /mika (mika#1255) and sub-repo-scoped
#      /mika-issue; the blanket copy clobbered them back to the 260-line
#      meta-repo dispatcher — re-creating the exact pre-#1255 recursion bug on
#      every dispatch. The worktree's tracked version always wins.
#
#   2. Copied meta-only commands MUST NOT dirty `git status`. They are ephemeral
#      dispatch scaffold (dispatch-lib already excludes .claude/commands/ from
#      its rescue `git add` — mika#1288); left visible they appear as ~18
#      untracked files that break the resume rebase ("cannot rebase: You have
#      unstaged changes") — the dirty-worktree class mika#1414 defends against.
#      We shield them via the worktree's shared info/exclude so the tree stays
#      clean at the source. (Verified: the per-worktree $GIT_DIR/info/exclude is
#      NOT honored for status; the common-dir info/exclude is.)
#
# Boundary (mika#1414 coordination): this helper owns ONLY the post-rebase
# command-seed. The pre-rebase dirty-state cleanup + rebase guard (the mika#1301
# block inside _set_up_worktree) is mika#1414's surface; the two do not overlap.
_seed_worktree_slash_commands() {
    local platform_dir=$1 worktree_dir=$2
    [ -d "$platform_dir/.claude/commands" ] || return 0
    mkdir -p "$worktree_dir/.claude/commands"

    # Shared exclude lives in the common git dir (a linked worktree's own
    # $GIT_DIR/info/exclude is not consulted for status). --path-format=absolute
    # needs git >= 2.31; fall back to the bare form otherwise.
    local common_dir exclude_file=""
    common_dir=$(git -C "$worktree_dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) \
        || common_dir=$(git -C "$worktree_dir" rev-parse --git-common-dir 2>/dev/null)
    if [ -n "$common_dir" ]; then
        exclude_file="$common_dir/info/exclude"
        mkdir -p "$(dirname "$exclude_file")"
    fi

    local src base
    for src in "$platform_dir/.claude/commands"/*.md; do
        [ -e "$src" ] || continue
        base=$(basename "$src")
        # Invariant 1: the worktree's own tracked command wins — skip the copy.
        # Name-based: today only /mika, /mika-issue, /mika-issues collide, and
        # the sub-repo version is correct for all three. If a sub-repo ever
        # tracks a meta-ONLY orchestration command name (e.g. mika-groom-ticket.md)
        # this would silently shadow it (a mika#1173 risk) — revisit with an
        # explicit must-seed set if that case ever arises.
        if git -C "$worktree_dir" ls-files --error-unmatch ".claude/commands/$base" >/dev/null 2>&1; then
            continue
        fi
        cp "$src" "$worktree_dir/.claude/commands/$base" 2>/dev/null || true
        # Invariant 2: shield the scaffold copy from git status (idempotent).
        # Concurrent dispatches off the same sub-repo share this exclude file;
        # the grep/append is non-atomic, so an overlap may append a duplicate
        # (inert — git collapses repeated patterns) but never corrupts shielding.
        # flock was judged not worth the complexity (P3).
        if [ -n "$exclude_file" ] && ! grep -qxF ".claude/commands/$base" "$exclude_file" 2>/dev/null; then
            # Guard a pre-existing exclude file with no trailing newline, which
            # would otherwise concatenate our entry onto its last line.
            if [ -s "$exclude_file" ] && [ -n "$(tail -c1 "$exclude_file" 2>/dev/null)" ]; then
                printf '\n' >> "$exclude_file"
            fi
            printf '%s\n' ".claude/commands/$base" >> "$exclude_file"
        fi
    done
}

# Set up a git worktree for the target issue's branch. Parses the repo#number
# prompt format, derives branch name and canonical worktree path, creates or
# reuses the worktree, rebases onto origin/main, and seeds slash commands.
#
# Pre-flight cleanup (mika#1472): before creating a new worktree, detects if the
# target branch is already checked out at a non-canonical path (e.g. a slashed-path
# relic from before the derive-worktree-path invariant). Stashes any dirty state
# with a descriptive name (mirroring _clean_worktree_for_rebase's discipline from
# mika#1414) and removes the relic so the worktree add can succeed on the canonical
# dashed-slug path.
#
# Dual-failure diagnostic (mika#1472): when both worktree add attempts fail, emits
# a structured worktree_setup_failed: line to stderr with per-attempt error text,
# replacing the previous silent exit-128 trap. This is the fifth dispatch-lib
# silent-failure defense — siblings: mika#1364 (force-with-lease gap), #1407
# (stale-main mis-diagnosis), #1414 (dirty-worktree on resume), #1415 (worktree-
# setup clobbers .claude/commands).
_set_up_worktree() {
    # --- Parse repo#number format ---
    # Matches: mika#214, mika-skills#8, mika-cloud#50, and an optional owner/
    # prefix (senara-solutions/mika#214). The owner prefix is stripped so REPO
    # is always the bare basename — dispatch-lib hardcodes the senara-solutions
    # owner for the gh call below. Normalizing here means an owner-qualified ref
    # is routed into worktree mode instead of silently falling through to
    # free-text mode (mika#1593). The match stays fully anchored, so genuine
    # free-text prompts with an embedded '#' still fall through as before.
    REPO=""
    ISSUE_NUM=""
    if printf '%s' "$PROMPT" | grep -qE '^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$'; then
        REPO=$(printf '%s' "$PROMPT" | sed 's/#.*//' | sed 's#.*/##')
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

        # --- Pre-flight: detect and clean up non-canonical worktree paths (mika#1472) ---
        # Before the canonical dashed-path collision check below, detect if the target
        # branch is already checked out at a DIFFERENT (non-canonical) worktree path —
        # e.g. a slashed-path relic from before the derive-worktree-path invariant
        # (worktree_path_slug == sanitize(branch_ref)). If found, stash any dirty state
        # (mirroring _clean_worktree_for_rebase's discipline from mika#1414) and remove
        # the relic so the subsequent worktree add can proceed on the canonical path.
        local existing_wt
        existing_wt=$(git -C "$SUB_REPO_DIR" worktree list --porcelain 2>/dev/null \
            | awk -v b="refs/heads/$BRANCH" '/^worktree / {wt = substr($0, 10)} $0 == "branch " b {print wt; exit}')
        if [ -n "$existing_wt" ] && [ "$existing_wt" != "$WORKTREE_DIR" ]; then
            echo "[dispatch-lib] pre-flight: branch $BRANCH is checked out at non-canonical path $existing_wt (canonical: $WORKTREE_DIR); cleaning up relic" >&2
            if [ -d "$existing_wt" ]; then
                local dirty_state
                dirty_state=$(git -C "$existing_wt" status --porcelain 2>/dev/null || true)
                if [ -n "$dirty_state" ]; then
                    local stash_name
                    stash_name="dispatch-lib-stale-worktree-cleanup-$(printf '%s' "$BRANCH" | tr / -)-$(date -u +%Y%m%dT%H%M%SZ)"
                    if git -C "$existing_wt" stash push --include-untracked -m "$stash_name" >/dev/null 2>&1; then
                        local stash_sha
                        stash_sha=$(git -C "$existing_wt" rev-parse --verify --quiet 'stash@{0}' 2>/dev/null || true)
                        echo "[dispatch-lib] stashed dirty state from $existing_wt as: $stash_name (sha: ${stash_sha:-<unknown>}; recover with: git -C $SUB_REPO_DIR stash apply ${stash_sha:-<sha>})" >&2
                    else
                        echo "[dispatch-lib] stash push failed or nothing to stash in $existing_wt; proceeding with remove" >&2
                    fi
                fi
            fi
            git -C "$SUB_REPO_DIR" worktree remove --force "$existing_wt" 2>/dev/null || true
        fi

        # Reuse existing worktree if valid
        if [ -d "$WORKTREE_DIR" ] && git -C "$WORKTREE_DIR" rev-parse --git-dir >/dev/null 2>&1; then
            git -C "$WORKTREE_DIR" checkout "$BRANCH" 2>/dev/null || true
        else
            git -C "$SUB_REPO_DIR" worktree remove --force "$WORKTREE_DIR" 2>/dev/null || true
            # mika#1311: when origin/$BRANCH already exists from a prior
            # successful dispatch, base the worktree on it (preserves prior
            # history) rather than creating a fresh local branch from
            # origin/main. Without this, re-dispatches after origin/main
            # advances past the prior groom diverge silently — the new
            # local branch has one commit on new main, the remote has the
            # prior groom on older main, and the post-flight push fails
            # with `branch is behind its remote counterpart`. The LLM
            # then correctly escalates to operator (status=blocked) but
            # the queue stays wedged. The downstream BEHIND-block rebases
            # onto origin/main and mika#784's _check_duplicate_commits
            # handles cherry-mark dedup of any merged-but-still-present
            # commits — so basing on origin/$BRANCH first and rebasing
            # afterward composes cleanly with the existing flow.
            # Stderr capture for dual-failure diagnostic (mika#1472 U2).
            # Each worktree add attempt captures stderr to a temp file; on
            # dual-failure, both are emitted as a structured worktree_setup_failed:
            # diagnostic so the operator sees WHY instead of a silent exit-128 trap.
            local wt_err_1="/tmp/wt-add-1-err.$$" wt_err_2="/tmp/wt-add-2-err.$$"
            local wt_add_ok=0
            if git -C "$SUB_REPO_DIR" ls-remote --exit-code origin "refs/heads/$BRANCH" >/dev/null 2>&1; then
                git -C "$SUB_REPO_DIR" fetch origin "$BRANCH" 2>/dev/null || true
                if git -C "$SUB_REPO_DIR" worktree add -b "$BRANCH" "$WORKTREE_DIR" "origin/$BRANCH" 2>"$wt_err_1"; then
                    wt_add_ok=1
                elif git -C "$SUB_REPO_DIR" worktree add "$WORKTREE_DIR" "$BRANCH" 2>"$wt_err_2"; then
                    wt_add_ok=1
                fi
            else
                if git -C "$SUB_REPO_DIR" worktree add -b "$BRANCH" "$WORKTREE_DIR" origin/main 2>"$wt_err_1"; then
                    wt_add_ok=1
                elif git -C "$SUB_REPO_DIR" worktree add "$WORKTREE_DIR" "$BRANCH" 2>"$wt_err_2"; then
                    wt_add_ok=1
                fi
            fi
            if [ "$wt_add_ok" -eq 0 ]; then
                echo "[dispatch-lib] worktree_setup_failed: branch=$BRANCH path=$WORKTREE_DIR" >&2
                echo "  attempt 1 (with -b): $(cat "$wt_err_1" 2>/dev/null)" >&2
                echo "  attempt 2 (without -b): $(cat "$wt_err_2" 2>/dev/null)" >&2
                rm -f "$wt_err_1" "$wt_err_2"
                return 1
            fi
            rm -f "$wt_err_1" "$wt_err_2"
        fi

        # Rebase-or-abort guard
        BEHIND=$(git -C "$WORKTREE_DIR" rev-list --count HEAD..origin/main 2>/dev/null || echo 0)
        if [ "$BEHIND" -gt 0 ]; then
            # mika#1414: guarantee a clean tree before rebase on the resume path —
            # a reused worktree can carry dirty state (dominant case: a stale
            # .claude/commands/mika.md from `make deploy`) that would otherwise
            # abort the rebase with a misleading REBASE_CONFLICT and re-block the
            # task. Rationale + tier design live in _clean_worktree_for_rebase.
            _clean_worktree_for_rebase "$WORKTREE_DIR"

            # Capture rebase stderr instead of discarding to /dev/null (mika#1364 AC#4).
            local rebase_err
            rebase_err=$(mktemp "${TMPDIR:-/tmp}/dispatch-lib-rebase-err.XXXXXX")
            if git -C "$WORKTREE_DIR" rebase origin/main 2>"$rebase_err"; then
                echo "Rebased ${BRANCH} onto origin/main (${BEHIND} commits caught up)." >&2
                rm -f "$rebase_err"
            else
                # Capture conflict list and rebase reason BEFORE --abort resets the index.
                CONFLICTS=$(git -C "$WORKTREE_DIR" diff --name-only --diff-filter=U 2>/dev/null | tr '\n' ' ')
                local rebase_reason rebase_mode
                rebase_reason=$(cat "$rebase_err" 2>/dev/null | head -20)
                if [ -n "$CONFLICTS" ]; then
                    rebase_mode="conflict"
                else
                    rebase_mode="other"
                fi
                git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
                rm -f "$rebase_err"
                RESULT="STATUS=REBASE_CONFLICT
Branch ${BRANCH} is ${BEHIND} commits behind origin/main.
Rebase failure mode: ${rebase_mode}
Conflicted files: ${CONFLICTS:-<none>}
Rebase stderr: ${rebase_reason:-<empty>}
Resolve manually before re-dispatching ${REPO}#${ISSUE_NUM}."
                exit 1
            fi
        fi

        # Copy gitignored .claude/ config into worktree (relay + permissions only)
        mkdir -p "$WORKTREE_DIR/.claude"
        cp "$PLATFORM_DIR/.claude/claude-pilot.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
        cp "$PLATFORM_DIR/.claude/settings.local.json" "$WORKTREE_DIR/.claude/" 2>/dev/null || true
        # Seed meta-repo orchestration slash commands for the inner session.
        # Invariants (no-clobber of the worktree's tracked commands; clean git
        # status) live in _seed_worktree_slash_commands (mika#1173, #1255, #1415).
        #
        # Snapshot semantics: the copy is taken at worktree-creation time; a
        # platform-root command edited mid-session is not picked up. Acceptable
        # because worktrees are short-lived and mid-session command edits
        # violate slug-immutability (mika#844).
        _seed_worktree_slash_commands "$PLATFORM_DIR" "$WORKTREE_DIR"

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
        # Save pre-run remote HEAD for pilot push guard (mika#1318).
        # Empty if branch doesn't exist on remote yet.
        PRE_RUN_REMOTE_HEAD=$(git -C "$WORKTREE_DIR" ls-remote origin "refs/heads/$BRANCH" 2>/dev/null | cut -f1 || true)
    else
        # --- Free-text mode: pass prompt as-is, no worktree ---
        PRE_RUN_HEAD=""
        PRE_RUN_REMOTE_HEAD=""
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

    # Unit 3 (mika#1282): flag for dirty-worktree rescue, checked by Unit 2.
    RESCUED_DIRTY_WORKTREE=0
    POST_RUN_HEAD=""

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

    # Compute POST_RUN_HEAD unconditionally (mika#1615): needed by recovery
    # blocks and downstream Unit 2 draft-PR creation regardless of whether
    # claude-pilot produced structured JSON output. Previously computed inside
    # Branch A only — Branch B (exit 0, non-JSON) and Branch C (non-zero exit)
    # silently skipped recovery because POST_RUN_HEAD was never set.
    if [ -n "$PRE_RUN_HEAD" ] && [ -n "$WORKTREE_DIR" ]; then
        POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
    fi

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

    # Post-flight recovery (mika#1615): runs unconditionally after exit
    # classification. Previously this logic lived inside the if [ -n "$STATUS" ]
    # branch only — Branch B (exit 0, non-JSON) and Branch C (non-zero exit)
    # silently skipped all recovery, losing uncommitted work.
    _post_flight_recovery

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

_post_flight_recovery() {
    # Post-flight recovery (mika#1615): extracted from the if [ -n "$STATUS" ]
    # branch so recovery fires on ALL exit paths — structured JSON output,
    # non-structured exit 0, and non-zero exit. Guards within each block use
    # PRE_RUN_HEAD, POST_RUN_HEAD, SKILL, REPO, BRANCH, WORKTREE_DIR — none
    # depend on STATUS. The mika#940 check explicitly checks STATUS=success
    # and naturally short-circuits when STATUS is empty.
    #
    # Variables read/written: PRE_RUN_HEAD, POST_RUN_HEAD, WORKTREE_DIR, SKILL,
    # REPO, BRANCH, ISSUE_NUM, SESSION_ID, LOG_ID, RESULT, STATUS,
    # RESCUED_DIRTY_WORKTREE, PR_URL, VALID_PLAN (all global/caller-scoped).

    # Post-flight diff check: detect zero-commit "success" in repo#number mode.
    if [ -n "$PRE_RUN_HEAD" ] && [ -n "$REPO" ]; then
        if [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ]; then
            # Policy-deny pre-check (Class C disambiguation, extended to dev-pilot
            # from dev-groom — companion to mika#1534). If the pilot halted on a
            # tier1/policy deny mid-flight, "Zero new commits" is the SYMPTOM, not
            # the cause. Read persistent stderr for [policy:deny] before declaring
            # the generic HEAD-unchanged failure. Fail-open: missing stderr → empty
            # POLICY_DENY → fall through to existing messages.
            #
            # See: docs/solutions/workflow-issues/
            #      2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md
            POLICY_DENY=""
            PERSISTENT_STDERR_PATH="/var/log/claude-pilot/${LOG_ID}.stderr"
            if [ -f "$PERSISTENT_STDERR_PATH" ] && [ -r "$PERSISTENT_STDERR_PATH" ]; then
                POLICY_DENY=$(sed 's/\x1b\[[0-9;]*[mK]//g' "$PERSISTENT_STDERR_PATH" 2>/dev/null \
                    | grep -m1 '\[policy:deny\]' || true)
            fi

            # mika#1333 Unit 2: For dev-groom re-dispatch, HEAD-unchanged is
            # expected when the plan was already committed in a prior run.
            # The architect pass (_iterate_groom_loop) is what matters — don't
            # poison RESULT with PIPELINE FAILURE for the expected re-dispatch state.
            if [ -n "$POLICY_DENY" ]; then
                # Class C — policy-deny halt. The pilot tried to do legitimate
                # work and was prevented by a tier1/policy allow-list gap. NOT
                # to be confused with LLM drift or genuine dirty-worktree-rescue.
                RESULT="PIPELINE FAILURE: claude-pilot session halted by policy deny — not generic exit.

Halt event: ${POLICY_DENY}

Likely a tier1 or tier2 allow-list gap in claude-pilot-py. Investigate the deny rule and either (a) widen the policy to include the legitimate command shape, or (b) rewrite the dispatch context so the pilot avoids the denied command. The pilot was prevented from completing its work — re-dispatching without addressing the substrate gap will hit the same wall.

See: docs/solutions/workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md

${RESULT}"
            elif [ "$SKILL" = "dev-groom" ] && [ -n "$WORKTREE_DIR" ] && \
               find "$WORKTREE_DIR/docs/plans" -name "*-plan.md" -size +500c 2>/dev/null | grep -q .; then
                RESULT="Note: HEAD unchanged on dev-groom re-dispatch — plan already committed from prior run. Architect pass will determine outcome.

${RESULT}"
            else
                RESULT="PIPELINE FAILURE: claude-pilot exited 0 but HEAD unchanged (pre: ${PRE_RUN_HEAD}, post: ${POST_RUN_HEAD}). Zero new commits produced.

${RESULT}"
            fi
        fi

        # Unit 1 (mika#1282): detect dirty worktree on zero-commit dev-pilot.
        # If the pilot wrote files but never committed, auto-rescue the content
        # so it isn't lost with the worktree. This is dispatch-lib exercising its
        # structural git-workflow ownership per the content/workflow split
        # (mika#1271 architect verdict; pilot-vs-substrate-contract-split-2026-05-25.md).
        if [ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ] && [ "$SKILL" = "dev-pilot" ] && [ -n "$WORKTREE_DIR" ]; then
            DIRTY_FILES=$(git -C "$WORKTREE_DIR" status --porcelain 2>/dev/null | head -20)
            if [ -n "$DIRTY_FILES" ]; then
                # Stage all dirty files EXCEPT worktree-scaffold paths copied by
                # _set_up_worktree (mika#1288, mika#1419, mika#1552):
                #   - .claude/commands/         slash-command snapshots from mika-platform
                #   - .claude/claude-pilot.json relay config cp'd from $PLATFORM_DIR at :489
                #   - .claude/settings.local.json  permission allowlist cp'd at :490 (mika#1552)
                #   - .claude/*.local.*          general guard for any future Claude-local
                #                                files (.env-class — operator-machine-specific)
                # None is pilot-authored content. Without the second exclusion, the rescue
                # commit re-introduces .claude/claude-pilot.json whose intentional deletion
                # shipped in PR #1348 (mika#1193 Phase C) — the founding incident for
                # mika#1419. The third + fourth catch the .claude/settings.local.json class
                # — cm#5 dispatch (2026-06-16) produced PR #16 whose only "rescued" content
                # was a 143-line operator allowlist leak (mika#1552 founding incident).
                git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' ':!.claude/claude-pilot.json' ':!.claude/settings.local.json' ':!.claude/*.local.*' 2>&9

                # Guard: if pathspec exclusion left nothing staged, skip the rescue
                # commit. Handles the edge case where the pilot wrote ONLY to scaffold
                # paths (mika#1288, mika#1419).
                if git -C "$WORKTREE_DIR" diff --cached --quiet 2>&9; then
                    echo "NOTE: dirty worktree contained only scaffold paths (.claude/commands/, .claude/claude-pilot.json) — no pilot content to rescue" >&2
                    RESCUED_DIRTY_WORKTREE=0
                else
                    # Compute accurate rescued-files list for the PIPELINE FAILURE
                    # message. DIRTY_FILES (from git status --porcelain) includes
                    # excluded scaffold paths; RESCUED_FILES reflects what was actually
                    # staged and will be committed.
                    RESCUED_FILES=$(git -C "$WORKTREE_DIR" diff --cached --name-only 2>&9)

                    # Proactive formatting (mika#1336): the dominant rescue-failure class is
                    # pilot-authored Rust that was never `cargo fmt`-ed, so the first commit
                    # trips the lefthook rust-fmt gate. Formatting up front makes the first
                    # commit succeed, halves wall-clock (one clippy compile, not two), and
                    # removes reliance on parsing lefthook stdout to detect a fmt rejection.
                    # The reactive rust-fmt retry below remains as belt-and-suspenders.
                    # Gated on staged *.rs so docs-only / non-Rust pilots don't pay cargo startup.
                    if git -C "$WORKTREE_DIR" diff --cached --name-only 2>&9 | grep -q '\.rs$'; then
                        PROACTIVE_FMT_ERR=$( (cd "$WORKTREE_DIR" && cargo fmt --all) 2>&1 ) || true
                        [ -n "$PROACTIVE_FMT_ERR" ] && echo "NOTE: proactive cargo fmt: ${PROACTIVE_FMT_ERR}" >&2
                        # Same exclusion pathspec as the initial `git add -A` above
                        # (mika#1288, mika#1419) — keeps scaffold paths out of the
                        # post-fmt re-add.
                        git -C "$WORKTREE_DIR" add -u -- ':!.claude/commands/' ':!.claude/claude-pilot.json' ':!.claude/settings.local.json' ':!.claude/*.local.*' 2>&9
                    fi

                    # Attempt rescue commit — capture stderr for hook-failure diagnosis (mika#1296).
                    # mika#1341: scratch file MUST live outside the worktree tree, NOT under
                    # "$WORKTREE_DIR/.git/". In a linked worktree (every autonomous dev-pilot run)
                    # ".git" is a FILE (a `gitdir:` pointer), not a directory — so a redirect into
                    # "$WORKTREE_DIR/.git/<name>" fails to OPEN (ENOTDIR). A failed output redirect
                    # means `git commit` never runs and exits non-zero with no captured output,
                    # producing the "non-rustfmt empty-capture" PIPELINE FAILURE with HEAD unchanged.
                    # `mktemp` keeps the original intent (off the working tree, away from .iterate/)
                    # while guaranteeing a real, writable path in both linked and non-linked checkouts.
                    # Named template preserves the descriptive "mika-rescue-commit-err" scratch name.
                    # NOTE: the literal token "mika-rescue-commit-err" is also a sed anchor in
                    # test-dispatch-lib.sh (rescue-block extraction); renaming it breaks those tests.
                    RESCUE_COMMIT_ERR="$(mktemp "${TMPDIR:-/tmp}/mika-rescue-commit-err.XXXXXX")"

                    # mika#1310: capture BOTH stdout and stderr. Lefthook
                    # pre-commit hooks print their summary + failure marks
                    # to stdout (not stderr); a `2>` redirect alone captured
                    # an empty file and the operator saw "Hook output:"
                    # blank on every false-positive rejection. Combined
                    # `>file 2>&1` captures the full lefthook decoration
                    # block including ⛔ failure lines.
                    if git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): impl staged by post-flight recovery (mika#1282)

Content written by pilot session ${SESSION_ID:-unknown} but git commit was never invoked.
Auto-rescued by dispatch-lib dirty-worktree detection.
Scaffold paths excluded (mika#1288, mika#1419)." > "$RESCUE_COMMIT_ERR" 2>&1; then
                        # Commit succeeded on first try — proceed normally
                        rm -f "$RESCUE_COMMIT_ERR"

                        # Update POST_RUN_HEAD so _push_branch sees new commits
                        POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)

                        # Amend the PIPELINE FAILURE message (already set above) with rescue note
                        RESULT="PIPELINE FAILURE: claude-pilot exited 0 but HEAD unchanged — dirty worktree detected and auto-committed (mika#1282 recovery).
Files rescued:
${RESCUED_FILES}

${RESULT}"

                        # Mark for draft PR creation in Unit 2
                        RESCUED_DIRTY_WORKTREE=1
                    elif grep -q "rust-fmt\|cargo fmt\|rustfmt" "$RESCUE_COMMIT_ERR" 2>/dev/null; then
                        # Pre-commit rust-fmt hook rejected — auto-fix and retry (mika#1296).
                        # Capture cargo fmt stderr so it surfaces in the PIPELINE FAILURE message
                        # if the retry also fails (review-guide.md § Single Responsibility — failure
                        # paths must surface all available diagnostic information).
                        CARGO_FMT_ERR=""
                        echo "NOTE: rescue commit rejected by rust-fmt hook — running cargo fmt and retrying" >&2
                        CARGO_FMT_ERR=$( (cd "$WORKTREE_DIR" && cargo fmt --all) 2>&1 ) || true
                        # Same exclusion pathspec as the initial `git add -A` above
                        # (mika#1288, mika#1419) — scaffold paths stay excluded on the
                        # post-fmt retry path too.
                        git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' ':!.claude/claude-pilot.json' ':!.claude/settings.local.json' ':!.claude/*.local.*' 2>&9

                        # mika#1310: capture both stdout+stderr (see above).
                        if git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): impl staged by post-flight recovery (mika#1282)

Content written by pilot session ${SESSION_ID:-unknown} but git commit was never invoked.
Auto-rescued by dispatch-lib dirty-worktree detection (cargo fmt applied).
Scaffold paths excluded (mika#1288, mika#1419)." > "$RESCUE_COMMIT_ERR" 2>&1; then
                            # Retry succeeded after cargo fmt
                            rm -f "$RESCUE_COMMIT_ERR"

                            POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)

                            RESULT="PIPELINE FAILURE: claude-pilot exited 0 but HEAD unchanged — dirty worktree detected and auto-committed after cargo fmt (mika#1282 + mika#1296 recovery).
Files rescued:
${RESCUED_FILES}

${RESULT}"

                            RESCUED_DIRTY_WORKTREE=1
                        else
                            # Retry also failed — abort rescue, leave dirty.
                            # Surface the full diagnostic chain: cargo fmt output + retry commit
                            # hook output, so the operator can diagnose from the message alone
                            # (mika#1296 acceptance criteria).
                            RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -50)
                        # mika#1310: if captured output is empty, dump git
                        # diagnostic state as fallback so PIPELINE FAILURE
                        # carries SOMETHING the operator can act on.
                        if [ -z "$(printf '%s' "$RESCUE_ERR_CONTENT" | tr -d '[:space:]')" ]; then
                            RESCUE_ERR_CONTENT="<rescue capture was empty — likely no hook output, falling back to git diagnostic>
git status:
$(git -C "$WORKTREE_DIR" status --short 2>&1 | head -10)
git diff --cached --name-only:
$(git -C "$WORKTREE_DIR" diff --cached --name-only 2>&1 | head -10)"
                        fi
                            RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook after cargo-fmt retry.
cargo fmt stderr: ${CARGO_FMT_ERR:-<empty>}
Hook output: ${RESCUE_ERR_CONTENT}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}

${RESULT}"
                            # Do NOT set RESCUED_DIRTY_WORKTREE — prevents empty draft PR
                            rm -f "$RESCUE_COMMIT_ERR"
                        fi
                    else
                        # Unknown hook failure — abort rescue, leave dirty
                        RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -50)
                        # mika#1310: if captured output is empty, dump git
                        # diagnostic state as fallback so PIPELINE FAILURE
                        # carries SOMETHING the operator can act on.
                        if [ -z "$(printf '%s' "$RESCUE_ERR_CONTENT" | tr -d '[:space:]')" ]; then
                            RESCUE_ERR_CONTENT="<rescue capture was empty — likely no hook output, falling back to git diagnostic>
git status:
$(git -C "$WORKTREE_DIR" status --short 2>&1 | head -10)
git diff --cached --name-only:
$(git -C "$WORKTREE_DIR" diff --cached --name-only 2>&1 | head -10)"
                        fi
                        RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook (non-rustfmt).
Hook output: ${RESCUE_ERR_CONTENT}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}

${RESULT}"
                        # Do NOT set RESCUED_DIRTY_WORKTREE — prevents empty draft PR
                        rm -f "$RESCUE_COMMIT_ERR"
                    fi
                fi
            fi
        fi

        # mika#1383: structural completion gate for HEAD-advanced-no-PR.
        # The pilot session ran content and committed, but ended its turn
        # before invoking `gh pr create` (Mode 1 = bare `/ce-work` launch
        # never had commit→PR in scope; Mode 2 = full `/mika` launch hit
        # prompt-enforcement fragility on the tail). dispatch-lib owns the
        # git/PR tail per mika#1271 (content/workflow split). Honors
        # Vincent's pre-reboot framing: "gate the loop until the tail's
        # fixed". Companion to mika#1282 (handles HEAD-unchanged + dirty);
        # this block handles HEAD-changed + missing PR.
        #
        # Decision matrix when this block fires (HEAD changed):
        #   dirty worktree           → Phase A: rescue dirty into wip() commit
        #   PR exists for branch     → Phase B no-op (existing path is success)
        #   no PR exists for branch  → Phase B: gh pr create from existing commits
        #
        # Scoped to dev-pilot only — dev-groom produces plan-only commits
        # and intentionally has no PR (plan goes on the branch, not in a PR).
        if [ "$SKILL" = "dev-pilot" ] && \
           [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" != "$POST_RUN_HEAD" ] && \
           [ -n "$WORKTREE_DIR" ] && [ -n "$BRANCH" ]; then

            # Phase A: rescue any trailing dirty content (pilot committed but
            # left additional uncommitted changes). Same exclusion pattern as
            # mika#1282 (scaffold paths must not be re-committed).
            DIRTY_AFTER_COMMITS=$(git -C "$WORKTREE_DIR" status --porcelain 2>/dev/null | head -5)
            if [ -n "$DIRTY_AFTER_COMMITS" ]; then
                git -C "$WORKTREE_DIR" add -A -- \
                    ':!.claude/commands/' ':!.claude/claude-pilot.json' ':!.claude/settings.local.json' ':!.claude/*.local.*' 2>&9 || true
                if ! git -C "$WORKTREE_DIR" diff --cached --quiet 2>&9; then
                    if git -C "$WORKTREE_DIR" commit -m "wip(${REPO}#${ISSUE_NUM}): trailing content after pilot end_turn (mika#1383)" 2>&9; then
                        git -C "$WORKTREE_DIR" push origin "$BRANCH" 2>&9 || true
                        POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD 2>/dev/null || true)
                        RESULT="${RESULT}

dispatch-lib (mika#1383): rescued trailing dirty content into wip() commit before PR check."
                    fi
                fi
            fi

            # Phase B: PR existence check. Use `gh pr list` filtered by
            # head branch — `gh pr view <branch>` requires the PR exist;
            # listing is the discoverable read.
            EXISTING_PR=""
            if command -v gh &>/dev/null; then
                EXISTING_PR=$(gh pr list --repo "$REPO" --head "$BRANCH" --state open --json url --jq '.[0].url // ""' 2>/dev/null || true)
            fi

            if [ -z "$EXISTING_PR" ]; then
                # No PR exists for this branch. Auto-create from existing commits.
                # Title: derive from latest commit subject (preserves pilot intent).
                # Body: link to dispatch-lib's structural completion gate doc.
                PR_TITLE=$(git -C "$WORKTREE_DIR" log -1 --format='%s' 2>/dev/null || echo "Auto-PR for ${REPO}#${ISSUE_NUM}")
                PR_BODY="$(cat <<PR_BODY_EOF
Auto-created by dispatch-lib (mika#1383 structural completion gate).

The pilot session completed work and committed but did not reach \`gh pr create\` before its turn ended. dispatch-lib takes ownership of the commit→PR tail per mika#1271 (content/workflow split). The pilot owned content; dispatch-lib owns git/PR.

Pilot session: \`${LOG_ID}\`
Branch: \`${BRANCH}\`

Closes #${ISSUE_NUM}

🤖 Generated with [Claude Code](https://claude.com/claude-code)
PR_BODY_EOF
)"

                if PR_CREATE_OUT=$(gh pr create \
                        --repo "$REPO" \
                        --base main \
                        --head "$BRANCH" \
                        --title "$PR_TITLE" \
                        --body "$PR_BODY" 2>&1); then
                    # Re-query the PR URL (gh pr create prints it but parsing is fragile).
                    EXISTING_PR=$(gh pr list --repo "$REPO" --head "$BRANCH" --state open --json url --jq '.[0].url // ""' 2>/dev/null || true)
                    RESULT="${RESULT}

dispatch-lib (mika#1383): auto-created PR ${EXISTING_PR} from pilot's commits — pilot reached end_turn without invoking gh pr create."
                else
                    # PR creation failed — surface manual recovery.
                    # Common causes: gh auth scope, branch already has closed PR, base branch mismatch.
                    RESULT="PIPELINE FAILURE: pilot produced commits on ${BRANCH} but no PR exists, and dispatch-lib's auto-create attempt failed.
gh pr create error: $(printf '%s' "$PR_CREATE_OUT" | head -5)

Manual recovery:
  gh pr create --repo ${REPO} --base main --head ${BRANCH} --title \"<title>\" --body \"<body>\"

${RESULT}"
                fi
            fi
        fi
    fi

    # Post-flight plan validation (mika#1033, mika#1032, mika#1394): detect
    # dev-groom drift where the session exits "success" but produced no valid
    # plan file (or only a stub/empty one) and/or never invoked /ce:plan.
    #
    # mika#1394: replaced date-specific `${TODAY_PREFIX}-*-plan.md` with
    # `_find_issue_plan` (issue-number match + content fallback). The old
    # date-prefix pattern false-negatived on re-dispatch when the plan was
    # committed on a prior day, poisoning RESULT with PIPELINE_INCOMPLETE
    # and preventing the GROOMED outcome from reaching mika-dev.
    if [ "$SKILL" = "dev-groom" ] && [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ]; then
        VALID_PLAN=$(_find_issue_plan 2>/dev/null) || VALID_PLAN=""

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

        # Policy-deny pre-check (drift-misdiagnosis fix, docs/solutions/
        # workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md).
        # If the pilot was halted by claude-pilot's tier1/policy classifier
        # on a research bash command, it is NOT LLM drift — the pilot
        # tried to do its work and was prevented. Disambiguate by reading
        # the persistent stderr for [policy:deny] before declaring drift.
        # Fail-open: if stderr is unavailable, fall through to the
        # existing drift messages.
        POLICY_DENY=""
        PERSISTENT_STDERR_PATH="/var/log/claude-pilot/${LOG_ID}.stderr"
        if [ -f "$PERSISTENT_STDERR_PATH" ] && [ -r "$PERSISTENT_STDERR_PATH" ]; then
            # Strip ANSI color codes, then extract the first [policy:deny] line.
            # The line shape is `[policy:deny] <Tool>: <command>[ \[rule-id\]]`.
            POLICY_DENY=$(sed 's/\x1b\[[0-9;]*[mK]//g' "$PERSISTENT_STDERR_PATH" 2>/dev/null \
                | grep -m1 '\[policy:deny\]' || true)
        fi

        if [ -n "$POLICY_DENY" ]; then
            # Class C — policy-deny-induced early halt. The pilot made a
            # legitimate research request that hit a tier1/policy allow-list
            # gap. This is NOT LLM drift; the operator should investigate
            # the deny rule, not the pilot's reasoning.
            RESULT="PIPELINE FAILURE: dev-groom session halted by claude-pilot policy deny — not LLM drift.

Halt event: ${POLICY_DENY}

Likely a tier1 or tier2 allow-list gap in claude-pilot-py. Investigate the deny rule and either (a) widen the policy to include the legitimate research command shape, or (b) rewrite the dispatch context so the pilot avoids the denied command. The pilot was prevented from completing its work — re-grooming this ticket without addressing the substrate gap will hit the same wall.

See: docs/solutions/workflow-issues/2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md

${RESULT}"
        elif [ -z "$VALID_PLAN" ] && [ "$CE_PLAN_INVOKED" != "1" ]; then
            # Both checks failed: no plan file AND /ce:plan never called
            RESULT="PIPELINE FAILURE: dev-groom: _find_issue_plan returned empty for $REPO#$ISSUE_NUM (no filename match *-${ISSUE_NUM}-*-plan.md and no header-line match in first 20 lines for known prefixes) and no /ce:plan invocation detected in session log. Likely causes: (a) pilot drifted into executor mode without writing a plan, (b) plan was written but _find_issue_plan's regex didn't match the header shape — check \${WORKTREE_DIR}/docs/plans/*-plan.md >500 bytes to distinguish (see mika#1602 class).

${RESULT}"
        elif [ -z "$VALID_PLAN" ]; then
            # Plan file missing but /ce:plan was called (or log unavailable)
            RESULT="PIPELINE FAILURE: dev-groom: _find_issue_plan returned empty for $REPO#$ISSUE_NUM (no filename match *-${ISSUE_NUM}-*-plan.md and no header-line match in first 20 lines for known prefixes). Inspect \${WORKTREE_DIR}/docs/plans/*-plan.md >500 bytes directly — if a plan exists, this is a _find_issue_plan discovery bug (see mika#1602 class); if no plan exists, the pilot drifted into executor mode.

${RESULT}"
        elif [ "$CE_PLAN_INVOKED" != "1" ] && [ "$CE_PLAN_INVOKED" != "unknown" ]; then
            # Valid plan file exists but /ce:plan was never invoked.
            # Demoted from PIPELINE FAILURE to advisory note (mika#1303):
            # pilot Write-tool plan creation is a valid path. The plan
            # file's existence + size threshold + downstream architect
            # verdict are the structural contract — the slash-command
            # invocation is one of multiple valid paths to producing a
            # plan, not the gate itself.
            echo "Note: dev-groom produced a plan file ($VALID_PLAN) without explicit /ce:plan invocation. Plan-file existence is the operative gate." >&2
        fi
    fi

    # Issue #138: Discover actual PR URL from the branch
    PR_URL=""
    if [ -n "$REPO" ] && [ -n "$BRANCH" ]; then
        PR_URL=$(gh pr list --repo "senara-solutions/$REPO" --head "$BRANCH" --json url --jq '.[0].url' 2>/dev/null || true)
        if [ -n "$PR_URL" ]; then
            RESULT="${RESULT}
PR: ${PR_URL}"
        fi
    fi

    # mika#940 Unit 1: post-flight PR-existence check.
    # Detect dev-pilot success-with-commits-but-no-PR — the premature-EndTurn
    # family where the model emits `[done] Success` after Edit/Compound
    # phases but before reaching git push + gh pr create. Classify as
    # PIPELINE FAILURE so mika-dev surfaces the gap instead of marking the
    # parent task `completed` on a stranded worktree.
    #
    # Guards:
    #   - $STATUS = success: don't double-classify already-failed sessions
    #     (per architect-validated plan; QA-review-#1140 finding 1).
    #   - $SKILL = dev-pilot: dev-groom commits a plan but no PR; the
    #     existing plan-validation check (mika#1134) covers that path.
    #   - $PR_URL empty: PR-discovery above found nothing.
    #   - $PRE_RUN_HEAD != $POST_RUN_HEAD: commits exist. If HEAD unchanged,
    #     the zero-commit check earlier in this block already fires.
    if [ "$STATUS" = "success" ] && [ "$SKILL" = "dev-pilot" ] && [ -z "$PR_URL" ] && [ -n "$PRE_RUN_HEAD" ] && [ -n "$POST_RUN_HEAD" ] && [ "$PRE_RUN_HEAD" != "$POST_RUN_HEAD" ]; then
        RESULT="PIPELINE FAILURE: claude-pilot produced commits (${PRE_RUN_HEAD}..${POST_RUN_HEAD}) but no PR was opened on branch '${BRANCH}'. Pipeline truncated before git push + gh pr create.

${RESULT}"
    fi

    # mika#940 Unit 3: outcome classification line for operator/mika-dev
    # consumption. Replaces heuristic log inspection with a single
    # structured marker. Order matters: pipeline failure wins over any
    # success-shape outcome.
    if echo "$RESULT" | grep -qF "PIPELINE FAILURE:"; then
        RESULT="${RESULT}

Outcome: PIPELINE_INCOMPLETE — manual recovery needed."
    elif [ -n "$PR_URL" ]; then
        RESULT="${RESULT}

Outcome: PR_OPENED — ${PR_URL}"
    elif [ "$SKILL" = "dev-groom" ] && [ -n "${VALID_PLAN:-}" ]; then
        # $VALID_PLAN is set by the dev-groom plan-validation block earlier
        # when a docs/plans/*-plan.md file >500 bytes is found.
        # mika#1333: emit PLAN_COMMITTED (not PLAN_GROOMED) at this stage.
        # The architect pass hasn't run yet — PLAN_GROOMED is only emitted
        # after _iterate_groom_loop succeeds (see dispatch_claude_pilot).
        RESULT="${RESULT}

Outcome: PLAN_COMMITTED — ${VALID_PLAN}"
    else
        RESULT="${RESULT}

Outcome: UNKNOWN — inspect worktree manually."
    fi
}

_check_pilot_force_push() {
    # Post-flight pilot push guard (mika#1318). Detects whether the pilot
    # pushed to the remote during its session — a scope-of-authority violation
    # for dev-groom (content-only; push is dispatch-lib's job). Returns 0 if
    # no violation, 1 if violation detected. Called unconditionally from
    # dispatch_claude_pilot(); skill-scoping is internal (early-return for
    # non-dev-groom skills).

    # Skill scope: dev-groom only (R5). Dev-pilot's push is legitimate.
    [ "$SKILL" = "dev-groom" ] || return 0

    # Guard: repo#number mode only (worktree must exist).
    [ -n "$WORKTREE_DIR" ] && [ -n "$BRANCH" ] || return 0

    # Query current remote HEAD. Fail-open on network error — a network
    # failure shouldn't block a legitimate dispatch; _push_branch will fail
    # independently if the remote is truly unreachable.
    local ls_remote_out post_remote_head
    if ! ls_remote_out=$(git -C "$WORKTREE_DIR" ls-remote origin "refs/heads/$BRANCH" 2>/dev/null); then
        echo "pilot_push_guard.clean: ls-remote failed (network?) — fail-open (branch=$BRANCH)" >&2
        return 0
    fi
    post_remote_head=$(printf '%s' "$ls_remote_out" | cut -f1)

    # Compare: if remote state changed between pre-run and post-run, the
    # pilot pushed (any push, not just force-push, is a violation for dev-groom).
    if [ "${PRE_RUN_REMOTE_HEAD:-}" = "${post_remote_head:-}" ]; then
        echo "pilot_push_guard.clean: no remote-ref change during pilot session (branch=$BRANCH)" >&2
        return 0
    fi

    # Violation detected.
    PUSH_VIOLATION_DETECTED=1
    PUSH_VIOLATION_EVIDENCE="pre_remote=${PRE_RUN_REMOTE_HEAD:-<none>} post_remote=${post_remote_head:-<none>}"
    echo "pilot_push_guard.violation: pilot pushed to remote during session (branch=$BRANCH, $PUSH_VIOLATION_EVIDENCE)" >&2
    return 1
}

_push_branch() {
    # Canonical push step in dispatch-lib's git workflow (mika#1271 contract
    # refactor; introduced as _post_flight_push in mika#1268). After
    # _run_claude_pilot completes, push any local-ahead commits to origin
    # regardless of pilot exit code. Handles both first-push (no origin/$BRANCH)
    # and existing-remote cases.

    # Guard: repo#number mode only — free-text mode has no branch to push.
    [ -n "$REPO" ] && [ -n "$WORKTREE_DIR" ] && [ -n "$BRANCH" ] || return 0

    # Pre-push duplicate-commit guard (mika#784)
    if ! _check_duplicate_commits; then
        echo "WARN: push_branch skipped — duplicate-commit guard failed for $BRANCH" >&2
        RESULT="${RESULT}
Push: SKIPPED — duplicate-commit guard detected patch-equivalent commits on branch that could not be auto-rebased. Manual resolution required."
        return 1
    fi

    # Fetch fresh remote state. No-ops if origin/$BRANCH doesn't exist (first-push).
    git -C "$WORKTREE_DIR" fetch origin "$BRANCH" 2>/dev/null || true

    # Three git states, distinguished against the REMOTE-TRACKING branch
    # (origin/$BRANCH) — never against local `main` (mika#1407):
    #   (a) HEAD == origin/$BRANCH         → nothing to push. NO-OP, NOT a
    #                                        divergence (early `return 0` below).
    #   (b) HEAD ahead of origin/$BRANCH   → push (fast-forward, or
    #                                        --force-with-lease when ancestry
    #                                        proves the branch was rebased).
    #   (c) branch base behind origin/main → a REBASE concern owned by
    #                                        _set_up_worktree; ORTHOGONAL to the
    #                                        push decision and not consulted here.
    # mika#1407: the dev-groom pilot used to make this call in prose and
    # conflated (c) — a stale local `main` ref — with (b), emitting a spurious
    # "remote divergence detected; abort" on a branch that had nothing to push.
    # The push decision lives here in code, keyed solely on origin/$BRANCH..HEAD,
    # so the stale-main symptom can never drive it.
    #
    # Branch on remote-ref existence (F1 fix from architect review on mika#1268):
    # Determine push mode: first-push, fast-forward, or diverged (mika#1364).
    local push_mode="first-push"
    if git -C "$WORKTREE_DIR" rev-parse --verify "origin/$BRANCH" >/dev/null 2>&1; then
        # Existing-remote case — state (a)/(b). Push only if HEAD is ahead of
        # the remote-tracking branch; ahead==0 is state (a), a clean no-op.
        local ahead
        ahead=$(git -C "$WORKTREE_DIR" rev-list "origin/$BRANCH..HEAD" --count 2>/dev/null || echo 0)
        [ "${ahead:-0}" -eq 0 ] && return 0

        # Ancestry check (mika#1364 KTD-1): determine if origin/$BRANCH is an
        # ancestor of HEAD (fast-forward) or not (diverged — rebase rewrote
        # history). Only the diverged case needs --force-with-lease.
        # Exit codes: 0 = is ancestor, 1 = not ancestor, 128+ = error.
        local ancestry_rc=0
        git -C "$WORKTREE_DIR" merge-base --is-ancestor "origin/$BRANCH" HEAD 2>/dev/null || ancestry_rc=$?
        if [ "$ancestry_rc" -eq 0 ]; then
            push_mode="fast-forward"
        elif [ "$ancestry_rc" -eq 1 ]; then
            push_mode="diverged"
        else
            # Ancestry probe itself errored (shallow clone, missing objects).
            # Fall back to plain push — this is current behavior, not a
            # regression. If the remote diverged, the push will reject as
            # non-fast-forward and land in the FAILED arm below. We do NOT
            # silently force on uncertain state (mika#1364 F2).
            push_mode="fast-forward"
            echo "WARN: push_branch ancestry probe failed (rc=$ancestry_rc) — falling back to plain push" >&2
        fi
    fi
    # First-push case (no origin/$BRANCH ref) — always push.
    # (Sub-PR 7b retired the Class D recovery shim's first-push path;
    # this helper is now the sole git-push site for dev-groom dispatches.)

    # Push with upstream tracking (-u sets upstream on first push).
    # Diverged branches use --force-with-lease to land rebased history
    # without clobbering concurrent remote advances (mika#1364 KTD-1).
    local push_err push_cmd
    push_err=$(mktemp /tmp/push-branch-err-XXXXXX)
    if [ "$push_mode" = "diverged" ]; then
        push_cmd=(git -C "$WORKTREE_DIR" push --force-with-lease="$BRANCH:origin/$BRANCH" -u origin "$BRANCH")
    else
        push_cmd=(git -C "$WORKTREE_DIR" push -u origin "$BRANCH")
    fi
    if "${push_cmd[@]}" >/dev/null 2>"$push_err"; then
        echo "push_branch: pushed $BRANCH to origin (mode=$push_mode)" >&2
        RESULT="${RESULT}
Push: pushed to origin/$BRANCH (mode=$push_mode)"
    else
        local push_err_content
        push_err_content=$(cat "$push_err" 2>/dev/null)
        echo "WARN: push_branch_failed for $BRANCH — commits remain local-only" >&2
        cat "$push_err" >&2
        # Distinguish lease-stale abort from other push failures (mika#1364).
        if printf '%s' "$push_err_content" | grep -q "stale info\|expected old/new\|failed to push"; then
            RESULT="${RESULT}
Push: FAILED — remote advanced since fetch (lease aborted); commits remain local-only on $BRANCH"
        else
            RESULT="${RESULT}
Push: FAILED — commits remain local-only on $BRANCH"
        fi
    fi
    rm -f "$push_err"
}

_check_duplicate_commits() {
    # Pre-push guard: detect commits on the branch that are patch-equivalent
    # to commits already on origin/main. These duplicates cause
    # mergeable=CONFLICTING on GitHub even though content is identical.
    # See mika#784 for the observed failure mode.
    #
    # Uses git log --cherry-mark --right-only which shows commits on HEAD
    # that do NOT have a patch-equivalent on origin/main. By inverting
    # (--left-right --cherry-mark), we can detect commits marked as '='
    # (equivalent on both sides).

    [ -n "$WORKTREE_DIR" ] || return 0

    # Fetch fresh main to compare against.
    # Failure-open: if fetch fails (network, auth), skip the guard but warn.
    # Rationale: don't block push on connectivity; but surface the degraded state
    # so dispatch logs show the guard was skipped. (review-guide.md § Single Responsibility)
    if ! git -C "$WORKTREE_DIR" fetch origin main 2>/dev/null; then
        echo "WARN: duplicate-commit guard skipped — could not fetch origin/main" >&2
        return 0
    fi

    # Find commits on HEAD that are patch-equivalent to commits on origin/main.
    # --cherry-mark marks equivalent commits with '=' prefix.
    # --right-only shows only commits on the right side (HEAD).
    # Equivalent commits on HEAD = duplicates that will conflict.
    local duplicates
    duplicates=$(git -C "$WORKTREE_DIR" log --cherry-mark --right-only \
        --format="%m %H %s" origin/main...HEAD 2>/dev/null \
        | grep "^=" || true)

    [ -z "$duplicates" ] && return 0

    # Duplicates found — attempt automatic rebase to clean them up
    echo "WARN: duplicate-commit guard found patch-equivalent commits on branch:" >&2
    echo "$duplicates" >&2
    echo "Attempting rebase onto origin/main to deduplicate..." >&2

    # Capture rebase stderr instead of discarding to /dev/null (mika#1364 AC#4).
    local dedup_rebase_err
    dedup_rebase_err=$(mktemp "${TMPDIR:-/tmp}/dispatch-lib-dedup-rebase-err.XXXXXX")
    if git -C "$WORKTREE_DIR" rebase origin/main 2>"$dedup_rebase_err"; then
        echo "Rebase succeeded — duplicate commits resolved." >&2
        rm -f "$dedup_rebase_err"
        return 0
    fi

    # Rebase failed — capture reason BEFORE --abort resets the index.
    local dedup_conflicts dedup_reason
    dedup_conflicts=$(git -C "$WORKTREE_DIR" diff --name-only --diff-filter=U 2>/dev/null | tr '\n' ' ')
    dedup_reason=$(cat "$dedup_rebase_err" 2>/dev/null | head -20)
    git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
    rm -f "$dedup_rebase_err"
    echo "ERROR: duplicate-commit rebase failed. Branch has commits equivalent to main:" >&2
    echo "$duplicates" >&2
    echo "Rebase stderr: ${dedup_reason:-<empty>}" >&2
    RESULT="${RESULT}
Dedup-rebase failed (${dedup_conflicts:+conflict: $dedup_conflicts}${dedup_conflicts:-other}): ${dedup_reason:-<no stderr>}"
    return 1
}

# ============================================================================
# Iterate-loop primitives (mika#1271 contract refactor — Phase A/B/C).
#
# These helpers will be wired into a state machine (`_iterate_groom_loop`) in a
# follow-up PR. v1 ships the primitives only — defined and unit-testable, no
# call sites in the live dispatch path. See
# docs/plans/2026-05-25-003-feat-1271-iterate-loop-state-machine-plan.md.
# ============================================================================

_find_issue_plan() {
    # Locate the plan file for $REPO#$ISSUE_NUM in $WORKTREE_DIR/docs/plans.
    #
    # Primary: filename embeds the issue number — `*-${ISSUE_NUM}-*-plan.md`
    #          (the convention most existing plans follow:
    #          e.g. `2026-06-05-001-fix-1407-pilot-push-diagnosis-plan.md`).
    #
    # Fallback: scan recently-written plan files for an explicit ticket
    #          reference in their content. The pilot is instructed to set
    #          `**Ticket:** mika issue#N` in the plan header but may also
    #          name the plan with a date-prefix slug-tail that does NOT
    #          embed the issue number (e.g. mika#771 wrote
    #          `2026-06-06-003-feat-post-condition-guard-send-message-plan.md`).
    #          Without this fallback, `_iterate_groom_loop` returns 1, the
    #          architect is never called, and the ticket lands in a half-state
    #          (plan committed, verdict missing). This is the founding
    #          incident for mika#1421 — bound at n=2 on 2026-06-06 across
    #          mika#1381 (n=1, 11:37Z) and mika#771 (n=2, 17:29Z).
    #
    # Both passes apply the >500-byte filter (mika#1033) and return the
    # most-recent match. Prints the absolute plan path on success; returns
    # non-zero with no stdout on failure. Callers must check `[ -n ... ]`
    # AND `[ -r ... ]` exactly as before.
    [ -n "$WORKTREE_DIR" ] && [ -n "$ISSUE_NUM" ] || return 1

    # Primary: filename-embedded issue number
    local plan_path
    plan_path=$(find "$WORKTREE_DIR/docs/plans" \
        -name "*-${ISSUE_NUM}-*-plan.md" -size +500c 2>/dev/null \
        | sort -r | head -1)
    if [ -n "$plan_path" ] && [ -r "$plan_path" ]; then
        printf '%s' "$plan_path"
        return 0
    fi

    # Fallback: content references the issue. Pattern handles four
    # header shapes the pilot has been observed to produce in plan headers:
    #   **Ticket:** mika issue#N    (current `/mika-groom-plan-only` shape)
    #   **Ticket:** mika#N          (older convention)
    #   **Issue:** mika#N           ("Issue" synonym, matches GitHub's UI; mika#1602)
    #   ticket: mika#N              (YAML frontmatter)
    #   issue: mika#N               (YAML frontmatter, "Issue" synonym; mika#1602)
    #
    # mika#1602 (n=3) widened the union to add the `**Issue:**` / `issue:`
    # branches after mika#1600's dev-groom dispatch wrote `**Issue:** mika#1600`
    # and BOTH passes missed (filename had no `-1600-` token AND the header was
    # not `**Ticket:**`). Founding cases for the content-fallback itself were
    # mika#1421 (n=2: mika#1381 + mika#771, both filename-shape gaps).
    #
    # Header-zone scope: the grep is restricted to the first 20 lines of
    # each plan file. The canonical ticket reference always sits in YAML
    # frontmatter or the markdown header above the Problem section.
    # Without this scope, a plan that QUOTES another ticket's `**Ticket:**`
    # line in body prose (e.g. to illustrate a founding incident) would
    # false-positive — observed during the mika#1421 v1 self-test where
    # the #1421 plan quoted mika#771's header on line 49 and matched
    # `ISSUE_NUM=771`. Headers stay in the first 20 lines; bodies don't.
    while IFS= read -r candidate; do
        [ -r "$candidate" ] || continue
        if head -n 20 "$candidate" 2>/dev/null \
            | grep -qE "^(\*\*Ticket:\*\*|\*\*Issue:\*\*|ticket:|issue:)\s+mika[[:space:]]?(issue)?#${ISSUE_NUM}\b"; then
            printf '%s' "$candidate"
            return 0
        fi
    done < <(find "$WORKTREE_DIR/docs/plans" -name "*-plan.md" -size +500c 2>/dev/null | sort -r)

    return 1
}

_arch_ask() {
    # Phase A — architect-call helper. Invokes mika-arch via the CLI with the
    # given skill, delivering plan content via stdin. Returns the full JSON
    # envelope on stdout for the caller to parse `.content` and
    # `.metadata.session_id`.
    #
    # mika#1283: previously passed "@${plan_path}" as the message argument,
    # expecting `mika ask` to expand it to file content. `mika ask` does NOT
    # support `@<path>` expansion (verified 2026-05-25 via direct probe — the
    # literal path string was sent, and mika-arch's `read_agent_file` is
    # scoped to /home/samidarko/.mika/agents/mika-arch/ so worktree paths
    # like /data/workspace/.../docs/plans/...md are unreadable). The
    # architect was reviewing whatever issue-body context was already in
    # session memory, not the plan content. Fix: pipe content via stdin
    # (mika ask "-" reads the message from stdin per `mika ask --help`).
    #
    # Args:
    #   $1: skill name (mika-arch-groom-ticket | mika-arch-second-review)
    #   $2: absolute path to plan file (content piped via stdin)
    #   $3: optional session_id to continue an existing architect session
    local skill="$1" plan_path="$2" session_id="${3:-}"

    [ -n "$skill" ] && [ -n "$plan_path" ] || { echo "_arch_ask: missing skill or plan_path" >&2; return 2; }
    [ -r "$plan_path" ] || { echo "_arch_ask: plan_path not readable: $plan_path" >&2; return 2; }

    local args=( ask --agent mika-arch --format json --verbose --enable-skill "$skill" )
    [ -n "$session_id" ] && args+=( --session-id "$session_id" )
    args+=( - )

    mika "${args[@]}" < "$plan_path"
}

# Module-global flag: set to 1 when tier-2 fuzzy matching fires, 0 otherwise.
# Read by _iterate_groom_loop to annotate trail entries with "(fuzzy)".
# Side-channel design per mika#1272 rev 2 — parser stdout stays clean.
#
# Implementation note: bash subshells ($(...)) cannot set parent variables, so
# we use a tmpfile to communicate the flag across the subshell boundary. The
# tmpfile path is set once at module load and cleaned up by callers' EXIT traps
# (dispatch-lib already installs one). Functions write "1" or "0" to the file;
# callers read it after the $(...) returns.
_DISPOSITION_FUZZY=0
_DISPOSITION_FUZZY_FILE="${TMPDIR:-/tmp}/.dispatch-lib-fuzzy-$$"

_disposition_was_fuzzy() {
    # Returns 0 (true) if the last _parse_disposition/_parse_verdict call used
    # tier-2 fuzzy matching; 1 (false) otherwise. Reads the tmpfile side-channel.
    [ -f "$_DISPOSITION_FUZZY_FILE" ] && [ "$(cat "$_DISPOSITION_FUZZY_FILE" 2>/dev/null)" = "1" ]
}

_parse_disposition_fuzzy() {
    # Tier 2 — fuzzy disposition parser (mika#1272). Reads architect response
    # text from stdin, applies case-insensitive pattern matching against known
    # paraphrase indicators, and emits the canonical disposition on stdout.
    #
    # Priority: ESCALATE > ITERATE > READY (most conservative wins).
    # Emits nothing if no pattern matches. Logs matched snippet to stderr.
    local text
    text=$(cat)

    local matched_escalate="" matched_iterate="" matched_ready=""
    local snippet=""

    # ESCALATE patterns
    for pat in "escalate" "human review" "cannot proceed" "fundamental" "out of scope for"; do
        if snippet=$(printf '%s' "$text" | grep -oi "$pat" | head -1) && [ -n "$snippet" ]; then
            matched_escalate="$snippet"
            break
        fi
    done

    # ITERATE patterns
    for pat in "needs revision" "another pass" "revise" "address the following" "concerns that require"; do
        if snippet=$(printf '%s' "$text" | grep -oi "$pat" | head -1) && [ -n "$snippet" ]; then
            matched_iterate="$snippet"
            break
        fi
    done

    # READY patterns (affirmative forward-motion signals only — no negated-absence)
    for pat in "proceed" "ship it" "dispatch" "good to go" "plan is clean"; do
        if snippet=$(printf '%s' "$text" | grep -oi "$pat" | head -1) && [ -n "$snippet" ]; then
            matched_ready="$snippet"
            break
        fi
    done

    # Disambiguation: ESCALATE > ITERATE > READY
    if [ -n "$matched_escalate" ]; then
        echo "_parse_disposition_fuzzy: mapped paraphrased disposition → ESCALATE (matched: '$matched_escalate')" >&2
        echo "ESCALATE"
    elif [ -n "$matched_iterate" ]; then
        echo "_parse_disposition_fuzzy: mapped paraphrased disposition → ITERATE (matched: '$matched_iterate')" >&2
        echo "ITERATE"
    elif [ -n "$matched_ready" ]; then
        echo "_parse_disposition_fuzzy: mapped paraphrased disposition → READY (matched: '$matched_ready')" >&2
        echo "READY"
    fi
    # No match → emit nothing (caller's * case fires)
}

_parse_verdict_fuzzy() {
    # Tier 2 — fuzzy verdict parser (mika#1272). Same pattern as
    # _parse_disposition_fuzzy but for second-pass verdicts (GROOMED vs ESCALATE).
    #
    # Priority: ESCALATE > GROOMED (conservative).
    local text
    text=$(cat)

    local matched_escalate="" matched_groomed=""
    local snippet=""

    # ESCALATE patterns
    for pat in "escalate" "cannot approve" "human review needed" "fundamental issues remain"; do
        if snippet=$(printf '%s' "$text" | grep -oi "$pat" | head -1) && [ -n "$snippet" ]; then
            matched_escalate="$snippet"
            break
        fi
    done

    # GROOMED patterns (use "ship it" not bare "ship" — avoids substring
    # false positives in "relationship", "ownership", "leadership", etc.)
    for pat in "groomed" "approved" "plan is ready" "ship it" "no remaining concerns"; do
        if snippet=$(printf '%s' "$text" | grep -oi "$pat" | head -1) && [ -n "$snippet" ]; then
            matched_groomed="$snippet"
            break
        fi
    done

    # Disambiguation: ESCALATE > GROOMED
    if [ -n "$matched_escalate" ]; then
        echo "_parse_verdict_fuzzy: mapped paraphrased verdict → ESCALATE (matched: '$matched_escalate')" >&2
        echo "ESCALATE"
    elif [ -n "$matched_groomed" ]; then
        echo "_parse_verdict_fuzzy: mapped paraphrased verdict → GROOMED (matched: '$matched_groomed')" >&2
        echo "GROOMED"
    fi
}

_parse_disposition() {
    # Phase B — first-pass verdict parser. Reads architect response text from
    # stdin, emits READY|ITERATE|ESCALATE on stdout (or nothing on no match).
    #
    # Tiered matching:
    #   Tier 1a: strict literal `Disposition: <X>` (zero-cost fast path).
    #   Tier 1b: literal `Verdict: GROOMED`/`Verdict: ESCALATE` (mika#1421 v3).
    #            When mika-arch's session memory has prior ITERATE findings on
    #            the same plan, a first-pass invocation can return second-pass
    #            keyword shapes — the architect has effectively "carried over"
    #            into a second-review stance. Without this tolerance, the
    #            iterate-loop logs UNPARSED, _iterate_groom_loop returns 1, and
    #            the groom lands in the half-state #1421 v1+v2 closed for a
    #            different sub-class. Mapping: GROOMED → READY (loop runs a
    #            confirmatory second-pass), ESCALATE → ESCALATE.
    #   Tier 2:  fuzzy paraphrase matching (conservative, ESCALATE wins ties).
    # Writes "1" to $_DISPOSITION_FUZZY_FILE when tier 2 fires, "0" when tier
    # 1a/1b fires. Callers read _disposition_was_fuzzy() after the $(...)
    # returns.
    printf '0' > "$_DISPOSITION_FUZZY_FILE"
    local text
    text=$(cat)
    local result
    # Tier 1a — canonical first-pass shape
    result=$(printf '%s' "$text" | grep -oE 'Disposition:[[:space:]]*(READY|ITERATE|ESCALATE)' \
        | grep -oE '(READY|ITERATE|ESCALATE)' \
        | head -1)
    if [ -n "$result" ]; then
        echo "$result"
        return
    fi
    # Tier 1b — Verdict-shape carry-over from architect session memory
    local verdict_keyword
    verdict_keyword=$(printf '%s' "$text" | grep -oE 'Verdict:[[:space:]]*(GROOMED|ESCALATE)' \
        | grep -oE '(GROOMED|ESCALATE)' \
        | head -1)
    case "$verdict_keyword" in
        GROOMED)
            echo "_parse_disposition: tier 1b accepted Verdict: GROOMED → READY (mika#1421 v3 session-carry-over tolerance)" >&2
            echo "READY"
            return
            ;;
        ESCALATE)
            echo "_parse_disposition: tier 1b accepted Verdict: ESCALATE → ESCALATE (mika#1421 v3 session-carry-over tolerance)" >&2
            echo "ESCALATE"
            return
            ;;
    esac
    # Tier 2 fallback
    result=$(printf '%s' "$text" | _parse_disposition_fuzzy)
    if [ -n "$result" ]; then
        printf '1' > "$_DISPOSITION_FUZZY_FILE"
        echo "$result"
    fi
}

_parse_verdict() {
    # Phase B — second-pass verdict parser. Reads architect response text from
    # stdin, emits GROOMED|ESCALATE on stdout (or nothing on no match).
    #
    # Two-tier matching (mika#1272):
    #   Tier 1: strict literal `Verdict: <X>` (zero-cost fast path)
    #   Tier 2: fuzzy paraphrase matching (conservative, ESCALATE wins ties)
    # Writes "1" to $_DISPOSITION_FUZZY_FILE when tier 2 fires, "0" when tier 1
    # fires. Callers read _disposition_was_fuzzy() after the $(...) returns.
    printf '0' > "$_DISPOSITION_FUZZY_FILE"
    local text
    text=$(cat)
    local result
    result=$(printf '%s' "$text" | grep -oE 'Verdict:[[:space:]]*(GROOMED|ESCALATE)' \
        | grep -oE '(GROOMED|ESCALATE)' \
        | head -1)
    if [ -n "$result" ]; then
        echo "$result"
        return
    fi
    # Tier 2 fallback
    result=$(printf '%s' "$text" | _parse_verdict_fuzzy)
    if [ -n "$result" ]; then
        printf '1' > "$_DISPOSITION_FUZZY_FILE"
        echo "$result"
    fi
}

_trail_append() {
    # Phase C — verdict-trail capture. Appends a single line to
    # $WORKTREE_DIR/.claude/groom-verdict-trail.log capturing one architect
    # call's metadata. Used by the eventual canonical-callout writer to render
    # the Grooming history field.
    #
    # Args: $1 = skill (groom-ticket | second-review), $2 = session_id,
    #       $3 = disposition (READY|ITERATE|ESCALATE) or verdict (GROOMED|ESCALATE)
    local skill="$1" session_id="$2" outcome="$3"
    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || return 0
    local trail_dir="$WORKTREE_DIR/.claude"
    mkdir -p "$trail_dir" 2>/dev/null
    local trail_file="$trail_dir/groom-verdict-trail.log"
    printf '%s\t%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$skill" "$session_id" "$outcome" >> "$trail_file"
}

_trail_read() {
    # Phase C — verdict-trail reader. Emits the trail entries as TSV on stdout.
    # Caller composes the Grooming history line from these entries.
    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || return 0
    local trail_file="$WORKTREE_DIR/.claude/groom-verdict-trail.log"
    [ -r "$trail_file" ] && cat "$trail_file"
}

_launch_revise_pilot() {
    # Phase D companion (mika#1271) — launch claude-pilot for content-only plan
    # revision against architect findings. Entry command: /mika-revise-plan
    # (slash command at mika-platform/.claude/commands/mika-revise-plan.md,
    # copied into the worktree by _set_up_worktree at task start).
    #
    # The pilot reads findings, revises the plan on disk in-place, exits. We
    # detect revision via sha256 of the plan file before-and-after. Identical
    # content = "no revision happened" = caller falls through.
    #
    # Args: $1 = absolute path to findings file
    # Returns: 0 if plan content changed, 1 otherwise (missing args, no plan
    #          found, pilot failed to revise).

    local findings_file="$1"
    [ -r "$findings_file" ] || {
        echo "WARN: _launch_revise_pilot: findings file not readable: $findings_file" >&2
        return 1
    }
    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || {
        echo "WARN: _launch_revise_pilot: WORKTREE_DIR missing" >&2; return 1; }
    [ -n "$ISSUE_NUM" ] || {
        echo "WARN: _launch_revise_pilot: ISSUE_NUM unset" >&2; return 1; }

    # Locate the plan file via _find_issue_plan (mika#1421 — filename pattern
    # with content-fallback for date-prefix slug-tail filenames).
    local plan_path
    plan_path=$(_find_issue_plan) || {
        echo "WARN: _launch_revise_pilot: no plan file to revise" >&2; return 1; }

    # sha256 before revise (detection mechanism; mtime is too coarse).
    local pre_hash; pre_hash=$(sha256sum "$plan_path" | cut -d' ' -f1)

    # Distinct sub-session log id for the revise pilot.
    local revise_log_id="${LOG_ID}-revise-$(date +%s)"
    local revise_stdout; revise_stdout=$(mktemp /tmp/revise-stdout-XXXXXX)
    local revise_stderr; revise_stderr=$(mktemp /tmp/revise-stderr-XXXXXX)

    echo "_launch_revise_pilot: launching (log $revise_log_id) for $REPO#$ISSUE_NUM with $(basename "$findings_file")" >&2
    set +e
    # CWD_ARGS is intentionally word-split (multiple flags)
    # shellcheck disable=SC2086
    claude-pilot --verbose --log-dir --task-id "$revise_log_id" \
        --command "/mika-revise-plan" $CWD_ARGS \
        -- "@${findings_file}" \
        >"$revise_stdout" 2>"$revise_stderr"
    local revise_exit=$?
    set -e

    # sha256 after revise
    local post_hash; post_hash=$(sha256sum "$plan_path" | cut -d' ' -f1)
    rm -f "$revise_stdout" "$revise_stderr"

    if [ "$pre_hash" != "$post_hash" ]; then
        echo "_launch_revise_pilot: plan revised (sha changed from ${pre_hash:0:12} to ${post_hash:0:12})" >&2
        return 0
    else
        echo "WARN: _launch_revise_pilot: plan unchanged after revise pilot (exit=$revise_exit)" >&2
        return 1
    fi
}

_cleanup_iterate_findings() {
    # Sweep $WORKTREE_DIR/.iterate/ on GROOMED success. PRESERVE on ESCALATE
    # for forensic access — the worktree TTL handles eventual cleanup, and the
    # findings file is the operator's primary forensic artifact when deciding
    # whether to retry, refactor, or kill the plan. Sweeping it on ESCALATE
    # deletes the evidence at exactly the moment it's most useful.
    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || return 0
    local findings_dir="$WORKTREE_DIR/.iterate"
    [ -d "$findings_dir" ] || return 0
    rm -rf "$findings_dir" 2>/dev/null || true
    echo "_cleanup_iterate_findings: swept $findings_dir on GROOMED" >&2
}

_escalate_groom() {
    # Phase D escalation helper (mika#1271) — fail loudly per mika#1033 precedent
    # when the architect returns ESCALATE (first-pass or second-pass). Writes the
    # architect's escalation rationale to $WORKTREE_DIR/.iterate/escalate-<stage>.md
    # for operator forensic access, and appends a structured PIPELINE FAILURE
    # marker to RESULT so the callback delivers an actionable error rather than
    # a generic "no PR" message.
    #
    # Findings are PRESERVED on ESCALATE — never swept. Worktree TTL handles
    # eventual cleanup. The findings file is the operator's primary forensic
    # artifact when deciding whether to retry, refactor, or kill the plan.
    #
    # Args:
    #   $1: stage label — "first-pass" | "second-pass-after-ready" | "second-pass-after-iterate"
    #   $2: architect content (the escalation rationale text)
    #   $3: architect session_id (for callback observability + log correlation)
    local stage="$1" content="$2" session_id="$3"

    local findings_dir="$WORKTREE_DIR/.iterate"
    mkdir -p "$findings_dir" 2>/dev/null || true
    local findings_file="$findings_dir/escalate-${stage}.md"
    printf '%s\n' "$content" > "$findings_file" 2>/dev/null || true

    echo "iterate_groom_loop: ESCALATE at ${stage} — failing loudly per mika#1033 (findings at ${findings_file})" >&2

    RESULT="${RESULT}
PIPELINE FAILURE: groom escalated by mika-arch ${stage}.
Verdict: ESCALATE — human review required.
Session: ${session_id}
Architect findings preserved at: ${findings_file}"
}

_write_canonical_callout() {
    # Phase D canonical body-callout writer (mika#1271). Called from
    # _iterate_groom_loop on GROOMED success. Prepends the canonical 3-line
    # callout block to the issue body so downstream dispatch gates (Pin B /
    # check_grooming_markers in executor.rs) pass with a verified architect
    # verdict.
    #
    # Sole structural writer as of sub-PR 7b: the Class D recovery shim
    # (_verify_and_write_body_callout, mika#1123) and its post-flight call site
    # in _run_claude_pilot were retired now that the iterate loop's architect
    # convergence provides the verified verdict directly.
    #
    # Idempotent: if all three dispatch-gate signals are already in the body
    # (branch line + plan path + second-pass GROOMED marker), skip writing.
    # The organic LLM writer in the dev-groom skill prompt may still emit a
    # callout until the dev-groom-prompt-update follow-up ships; the
    # idempotency check absorbs that overlap cleanly.
    #
    # Args:
    #   $1: stage label — "ready-to-groomed" | "iterate-to-groomed"
    #   $2: architect session_id (for forensic correlation in the body)
    local stage="$1" session_id="$2"

    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || {
        echo "WARN: write_canonical_callout: WORKTREE_DIR unset or missing" >&2; return 1; }
    [ -n "$REPO" ] && [ -n "$ISSUE_NUM" ] && [ -n "$BRANCH" ] || {
        echo "WARN: write_canonical_callout: REPO/ISSUE_NUM/BRANCH unset" >&2; return 1; }

    # Compose the Grooming history line per stage. Both forms include
    # "second-pass (GROOMED)" to satisfy the dispatch-gate has_verdict regex.
    local history_line
    case "$stage" in
        ready-to-groomed)
            history_line="> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: ${session_id}"
            ;;
        iterate-to-groomed)
            history_line="> - **Grooming history:** first-pass (ITERATE) → revised → second-pass (GROOMED) — session-id: ${session_id}"
            ;;
        *)
            echo "WARN: write_canonical_callout: unknown stage \"$stage\"" >&2
            return 1
            ;;
    esac

    # Locate the plan file via _find_issue_plan (mika#1421 — filename pattern
    # with content-fallback for date-prefix slug-tail filenames).
    local plan_path
    plan_path=$(_find_issue_plan) || {
        echo "WARN: write_canonical_callout: no issue-scoped plan file for $REPO#$ISSUE_NUM" >&2
        return 1
    }
    local plan_relpath="${plan_path#"$WORKTREE_DIR/"}"

    # Fetch current body for idempotency check.
    local current_body
    current_body=$(gh issue view "$ISSUE_NUM" --repo "senara-solutions/$REPO" \
        --json body -q '.body' 2>/dev/null) || {
        echo "WARN: write_canonical_callout: gh issue view failed for $REPO#$ISSUE_NUM" >&2
        return 1
    }

    # Same three-signal check the dispatch gate uses in
    # executor.rs::check_grooming_markers (Pin B).
    local has_branch has_plan has_verdict
    has_branch=$(printf '%s' "$current_body" | grep -cF '> - **Branch:**' || true)
    has_plan=$(printf '%s' "$current_body" | grep -cF 'docs/plans/' || true)
    has_verdict=$(printf '%s' "$current_body" | grep -cE 'second-pass \(GROOMED\)|second-pass \(READY, paraphrased GROOMED' || true)

    if [ "$has_branch" -gt 0 ] && [ "$has_plan" -gt 0 ] && [ "$has_verdict" -gt 0 ]; then
        echo "write_canonical_callout: dispatch-gate signals already present in $REPO#$ISSUE_NUM body — skipping (idempotent)" >&2
        return 0
    fi

    local head_sha
    head_sha=$(git -C "$WORKTREE_DIR" rev-parse --short HEAD 2>/dev/null)

    local callout_block
    callout_block=$(cat <<CALLOUT_EOF
> - **Branch:** \`${BRANCH}\`
> - **Plan:** \`${plan_relpath}\` (committed on branch @ \`${head_sha}\`)
${history_line}
CALLOUT_EOF
    )

    local new_body
    new_body=$(printf '%s\n\n%s' "$callout_block" "$current_body")
    local tmpfile
    tmpfile=$(mktemp /tmp/canonical-callout-XXXXXX.md)
    printf '%s' "$new_body" > "$tmpfile"

    # mika#1309: capture stderr from gh issue edit so failure cause is visible.
    # The previous `2>/dev/null` silently dropped permission/rate-limit/network
    # errors, leaving the dispatch-gate signals missing without operator-visible
    # cause. We also redirect stdout to /dev/null (gh prints URL on success)
    # and route stderr to a captured variable so it can be surfaced in the WARN
    # line on failure.
    local gh_stderr gh_exit
    gh_stderr=$(gh issue edit "$ISSUE_NUM" --repo "senara-solutions/$REPO" \
        --body-file "$tmpfile" 2>&1 >/dev/null)
    gh_exit=$?
    if [ "$gh_exit" -eq 0 ]; then
        echo "write_canonical_callout: wrote canonical callout to $REPO#$ISSUE_NUM (stage=$stage, session=$session_id)" >&2
        rm -f "$tmpfile"
        return 0
    else
        echo "WARN: write_canonical_callout: gh issue edit failed for $REPO#$ISSUE_NUM (exit=$gh_exit): ${gh_stderr:-<empty>}" >&2
        rm -f "$tmpfile"
        return 1
    fi
}

_iterate_groom_loop() {
    # Phase D — the iterate-loop state machine (mika#1271).
    #
    # Architect-driven groom convergence with five terminal states:
    #   READY    → second-pass GROOMED → _write_canonical_callout "ready-to-groomed"
    #   READY    → second-pass *      → _escalate_groom "second-pass-after-ready"
    #   ITERATE  → revise → second-pass GROOMED → _write_canonical_callout "iterate-to-groomed"
    #   ITERATE  → revise → second-pass *      → _escalate_groom "second-pass-after-iterate"
    #   ESCALATE (first-pass)                   → _escalate_groom "first-pass"
    #
    # GROOMED paths preserve findings in $WORKTREE_DIR/.iterate/ until cleanup
    # at the end of the success branch. ESCALATE paths PRESERVE findings for
    # operator forensic access (worktree TTL handles eventual sweep).
    #
    # Always-on for the dev-groom skill. As of sub-PR 7b the Class D recovery
    # shim is retired; non-zero return from this loop means the dispatch gate
    # may not be satisfied by a canonical writer block on this run, but the
    # pilot's organic write in the dev-groom skill prompt remains a fallback
    # until the dev-groom-prompt-update follow-up ships.
    #
    # Guards: requires WORKTREE_DIR, ISSUE_NUM, REPO; finds the plan file via
    # `_find_issue_plan` (issue-scoped filename pattern first, content-grep
    # fallback for date-prefix slug-tail filenames per mika#1421).
    # Returns 1 if any guard fails.

    [ -n "$WORKTREE_DIR" ] && [ -d "$WORKTREE_DIR" ] || {
        echo "WARN: iterate_groom_loop: WORKTREE_DIR unset or missing" >&2; return 1; }
    [ -n "$ISSUE_NUM" ] && [ -n "$REPO" ] || {
        echo "WARN: iterate_groom_loop: ISSUE_NUM or REPO unset" >&2; return 1; }

    # Locate the plan file via _find_issue_plan (mika#1421 — filename pattern
    # with content-fallback for date-prefix slug-tail filenames).
    local plan_path
    plan_path=$(_find_issue_plan) || {
        echo "WARN: iterate_groom_loop: no issue-scoped plan file for $REPO#$ISSUE_NUM" >&2
        return 1
    }

    echo "iterate_groom_loop: invoking mika-arch first-pass on $(basename "$plan_path")" >&2

    # Phase 1 — first-pass
    local resp1; resp1=$(_arch_ask "mika-arch-groom-ticket" "$plan_path" 2>/dev/null) || {
        echo "WARN: iterate_groom_loop: first-pass _arch_ask failed" >&2; return 1; }
    local content1; content1=$(printf '%s' "$resp1" | jq -r '.content // empty' 2>/dev/null)
    local session_id; session_id=$(printf '%s' "$resp1" | jq -r '.metadata.session_id // empty' 2>/dev/null)
    [ -n "$content1" ] && [ -n "$session_id" ] || {
        echo "WARN: iterate_groom_loop: first-pass response missing .content or .metadata.session_id" >&2
        return 1
    }
    local disposition; disposition=$(printf '%s' "$content1" | _parse_disposition)
    local _trail_suffix=""
    _disposition_was_fuzzy && _trail_suffix=" (fuzzy)"
    _trail_append "groom-ticket" "$session_id" "${disposition:-UNPARSED}${_trail_suffix}"

    case "$disposition" in
        READY)
            echo "iterate_groom_loop: first-pass READY; invoking mika-arch second-pass" >&2
            # Phase 2 — second-pass, continuing the architect session
            local resp2; resp2=$(_arch_ask "mika-arch-second-review" "$plan_path" "$session_id" 2>/dev/null) || {
                echo "WARN: iterate_groom_loop: second-pass _arch_ask failed" >&2; return 1; }
            local content2; content2=$(printf '%s' "$resp2" | jq -r '.content // empty' 2>/dev/null)
            [ -n "$content2" ] || {
                echo "WARN: iterate_groom_loop: second-pass response missing .content" >&2; return 1; }
            local verdict; verdict=$(printf '%s' "$content2" | _parse_verdict)
            local _trail_suffix_v=""
            _disposition_was_fuzzy && _trail_suffix_v=" (fuzzy)"
            _trail_append "second-review" "$session_id" "${verdict:-UNPARSED}${_trail_suffix_v}"

            case "$verdict" in
                GROOMED)
                    echo "iterate_groom_loop: converged on GROOMED for $REPO#$ISSUE_NUM (session $session_id)" >&2
                    _write_canonical_callout "ready-to-groomed" "$session_id" || \
                        echo "WARN: canonical_callout_failed — dispatch gate will reject next ready unless pilot organic write or operator-direct rescue fills the body callout" >&2
                    _cleanup_iterate_findings
                    return 0
                    ;;
                *)
                    _escalate_groom "second-pass-after-ready" "$content2" "$session_id"
                    return 1
                    ;;
            esac
            ;;
        ITERATE)
            echo "iterate_groom_loop: first-pass ITERATE — launching revise pilot with findings" >&2
            # Write architect findings to a tempfile in $WORKTREE_DIR/.iterate/
            # (out-of-namespace from .claude/ to avoid collision with the
            # slash-command snapshot that _set_up_worktree copies in).
            local findings_dir="$WORKTREE_DIR/.iterate"
            mkdir -p "$findings_dir" 2>/dev/null || {
                echo "WARN: iterate_groom_loop: cannot create $findings_dir" >&2; return 1; }
            local findings_file="$findings_dir/findings-1.md"
            printf '%s\n' "$content1" > "$findings_file" || {
                echo "WARN: iterate_groom_loop: cannot write $findings_file" >&2; return 1; }

            # Launch revise pilot with the findings as @-file payload. Pilot
            # revises plan on disk; we detect via sha256.
            _launch_revise_pilot "$findings_file" || {
                echo "WARN: iterate_groom_loop: revise pilot did not converge — preserving $findings_file for forensics" >&2
                return 1
            }

            # Plan revised. Invoke mika-arch second-pass on the revised plan,
            # continuing the architect session so findings stay in conversation
            # memory (per mika-arch-second-review session-continuity contract).
            echo "iterate_groom_loop: invoking mika-arch second-pass on revised plan" >&2
            local resp2_iter; resp2_iter=$(_arch_ask "mika-arch-second-review" "$plan_path" "$session_id" 2>/dev/null) || {
                echo "WARN: iterate_groom_loop: second-pass _arch_ask failed (after revise)" >&2
                return 1
            }
            local content2_iter; content2_iter=$(printf '%s' "$resp2_iter" | jq -r '.content // empty' 2>/dev/null)
            [ -n "$content2_iter" ] || {
                echo "WARN: iterate_groom_loop: second-pass response missing .content (after revise)" >&2
                return 1
            }
            local verdict_iter; verdict_iter=$(printf '%s' "$content2_iter" | _parse_verdict)
            local _trail_suffix_vi=""
            _disposition_was_fuzzy && _trail_suffix_vi=" (fuzzy)"
            _trail_append "second-review" "$session_id" "${verdict_iter:-UNPARSED}${_trail_suffix_vi}"

            case "$verdict_iter" in
                GROOMED)
                    echo "iterate_groom_loop: revised plan converged on GROOMED for $REPO#$ISSUE_NUM (session $session_id)" >&2
                    _write_canonical_callout "iterate-to-groomed" "$session_id" || \
                        echo "WARN: canonical_callout_failed — dispatch gate will reject next ready unless pilot organic write or operator-direct rescue fills the body callout" >&2
                    _cleanup_iterate_findings
                    return 0
                    ;;
                *)
                    _escalate_groom "second-pass-after-iterate" "$content2_iter" "$session_id"
                    return 1
                    ;;
            esac
            ;;
        ESCALATE)
            _escalate_groom "first-pass" "$content1" "$session_id"
            return 1
            ;;
        *)
            echo "WARN: iterate_groom_loop: first-pass disposition unparsed (literal Disposition: line absent or paraphrased — see mika#1272)" >&2
            return 1
            ;;
    esac
}

# _label_to_type — Map GitHub issue label to conventional-commit type prefix.
# Uses the first matching label from a comma-separated list.
_label_to_type() {
    case "$1" in
        *enhancement*|*feature*) echo "feat" ;;
        *bug*)                   echo "fix" ;;
        *infrastructure*)        echo "chore" ;;
        *documentation*)         echo "docs" ;;
        *refactor*)              echo "refactor" ;;
        *test*)                  echo "test" ;;
        *)                       echo "chore" ;;
    esac
}

# _derive_recovery_pr_title — Compute a conventional-commit PR title for
# recovery-class PRs. Called by the recovery block (mika#1282 + mika#1396).
#
# For commit-pushed-no-pr: reads the impl commit subject from branch tip.
# For dirty-worktree: reads the plan file H1 or falls back to issue title.
#
# Args:
#   $1 — recovery class ("dirty-worktree" or "commit-pushed-no-pr")
#   $2 — worktree dir
#   $3 — repo name
#   $4 — issue number
#   $5 — labels (comma-separated)
#   $6 — issue title
#
# Outputs: PR title string to stdout
_derive_recovery_pr_title() {
    local recovery_class="$1"
    local wt_dir="$2"
    local repo="$3"
    local issue_num="$4"
    local labels="$5"
    local issue_title="$6"

    if [ "$recovery_class" = "commit-pushed-no-pr" ]; then
        local impl_subject
        impl_subject=$(git -C "$wt_dir" log -1 --format='%s' HEAD 2>/dev/null)
        if [ -n "$impl_subject" ]; then
            echo "$impl_subject"
            return
        fi
    fi

    # dirty-worktree or fallback: derive from plan H1 + labels
    local type_prefix
    type_prefix=$(_label_to_type "$labels")

    # Look for plan file
    local plan_file
    plan_file=$(find "$wt_dir/docs/plans" -name "*-${issue_num}-*-plan.md" 2>/dev/null | sort -r | head -1)

    if [ -n "$plan_file" ]; then
        local plan_h1
        plan_h1=$(head -5 "$plan_file" | grep -m1 '^# ' | sed 's/^# //')
        if [ -n "$plan_h1" ]; then
            # Check if H1 already has conventional-commit format
            if echo "$plan_h1" | grep -qE '^(feat|fix|chore|docs|refactor|test|perf|ci)[:(]'; then
                echo "$plan_h1"
                return
            fi
            echo "${type_prefix}: ${plan_h1} (${repo}#${issue_num})"
            return
        fi
    fi

    # Final fallback: issue title
    if echo "$issue_title" | grep -qE '^(feat|fix|chore|docs|refactor|test|perf|ci)[:(]'; then
        echo "$issue_title"
        return
    fi
    echo "${type_prefix}: ${issue_title} (${repo}#${issue_num})"
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
        # compound-engineering 3.x renamed `name: ce:work` → `name: ce-work`
        # (CHANGELOG #503). The plugin's `/ce:work` slash command was removed; the
        # canonical invocation is now `/ce-work`. Dispatch-lib must use the new
        # form or claude-pilot exits 7ms with `[error] pipeline_incomplete:` (no
        # API call). See mika#1345.
        ENTRY_COMMAND="/ce-work $PLAN_PATH"
        echo "Plan-on-branch detected: overriding entry command to '/ce-work $PLAN_PATH'" >&2
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

    # claude-pilot venv smoke test (mika#1200): force the import chain that imports
    # yaml (and all other dependencies) to actually execute. Relies on cli.py keeping
    # its imports at module top level — if cli.py is ever refactored to lazy-import
    # .agent / .permissions inside main(), THIS smoke test silently stops detecting
    # the failure class. See
    # mika/docs/plans/2026-05-18-001-bug-dev-groom-pilot-empty-handed-plan.md
    # § Phase 0 Pin / cli.py invariant.
    if ! timeout 15 claude-pilot --help >/dev/null 2>&9; then
        cat >&2 <<'EOF'
Error: claude-pilot venv is broken — `claude-pilot --help` exited non-zero.
Most likely cause: pyproject.toml changed in claude-pilot-py without an
accompanying `uv tool install` to re-sync dependencies. Editable installs pick
up new source automatically but do NOT auto-install new declared dependencies.

To restore the loop:
    cd <mika-platform-root> && uv tool install --force --editable ./claude-pilot-py

Reference: mika#1200 +
mika/docs/plans/2026-05-18-001-bug-dev-groom-pilot-empty-handed-plan.md
EOF
        exit 1
    fi

    # mika-platform root — base for sub-repo resolution
    PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
    PLATFORM_DIR=$(cd "$PLATFORM_DIR" 2>/dev/null && pwd -P) || PLATFORM_DIR="${MIKA_PLATFORM_DIR:-$HOME/workspace/mika-platform}"
    PLATFORM_REPO_NAME=$(basename "$PLATFORM_DIR")

    # Initialize callback guard
    CALLBACK_SENT=0

    _parse_input_json

    # Install EXIT trap for crash-recovery callback delivery
    trap '_dispatch_lib_exit_trap' EXIT
    # Install TERM trap for cancel discriminator (mika#749)
    trap '_dispatch_lib_term_trap' TERM

    _validate_inputs

    # PER-SKILL DISPATCH MAPPING (mika#932 origin, mika#1173 per-tool revert)
    # Each arm maps a SKILL value to its slash-command entry point. After the
    # mika#1173 revert, each dispatch skill owns its own tool (dev-pilot →
    # run_claude_pilot, dev-groom → run_claude_pilot_groom), so a given arm
    # fires only when the matching tool's handler sources this lib.
    # Adding a new dispatch sibling requires:
    #   1. Create the skill's tools.json registering its own tool name.
    #   2. Create the skill's handlers/run.sh that sources this lib and calls
    #      dispatch_claude_pilot.
    #   3. Add a new arm below mapping its SKILL value → ENTRY_COMMAND.
    #   4. Add the skill to the relevant well-known agent allowlist
    #      (well_known_agents.rs MIKA_*_IDENTITY).
    #   5. Update self-dev/system_prompt.md to teach mika-dev when to dispatch.
    # Threshold for refactor: if N>5 dispatch skills, consider engine-side
    # routing helpers. Until then, the case switch is the contract.
    local ENTRY_COMMAND
    case "$SKILL" in
      dev-pilot)
        ENTRY_COMMAND="/mika"
        # mika#940: signal claude-pilot to fail the session if `gh pr create`
        # is never invoked. Caught by the source-level pipeline_incomplete
        # detection in claude-pilot-py (Unit 2). Defense-in-depth against the
        # premature-EndTurn family observed on 2026-05-02 (mika#931, #938,
        # #939) — the model emits `[done] Success` after Edit-heavy phases
        # before reaching git push + gh pr create.
        export CLAUDE_PILOT_REQUIRE_PR=1
        ;;
      dev-groom)
        # As of mika#1271 sub-PR 8: autonomous-loop pilot uses /mika-groom-plan-only
        # (content-only — generate plan, commit, push, exit). Architect convergence
        # + canonical body-callout write are owned by dispatch-lib's _iterate_groom_loop
        # below. /mika-groom-ticket remains the operator-facing full pipeline
        # (Phase 1-6 + architect + body callout + comment) and is unchanged.
        ENTRY_COMMAND="/mika-groom-plan-only"
        # Early-exit guard (mika#1097 Layer B): dev-groom sessions MUST produce
        # tool calls (at minimum: gh issue view, git worktree, /ce:plan, git commit/push).
        # If the session exits "success" with fewer than this threshold, claude-pilot
        # re-prompts once; a second early-exit emits early_exit_zero_action.
        # Threshold unchanged from /mika-groom-ticket — /mika-groom-plan-only still
        # produces 5+ tool calls (issue view, /ce:plan, file edits, git add/commit/push).
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

    # mika#1318 — pilot push guard (defense-in-depth). Called unconditionally;
    # skill-scoping is internal (early-return for non-dev-groom). If violation
    # detected, poison RESULT and skip iterate loop + push — deliver callback
    # immediately so mika-dev receives the violation.
    if ! _check_pilot_force_push; then
        RESULT="STRUCTURAL VIOLATION: pilot push detected (mika#1318). The dev-groom pilot pushed to the remote during its session — this is a scope-of-authority violation. Push is dispatch-lib's responsibility, not the pilot's.

Evidence: ${PUSH_VIOLATION_EVIDENCE}

Outcome: PIPELINE_INCOMPLETE — push violation

${RESULT}"
        _deliver_callback
        return
    fi

    # mika#1271 — iterate-loop state machine (always-on for dev-groom).
    # Invokes mika-arch first-pass on the plan-on-branch, then second-pass on READY
    # or ITERATE-then-revise; on GROOMED writes the canonical body callout via
    # _write_canonical_callout (idempotent vs. the pilot's organic write); on ESCALATE
    # appends a structured PIPELINE FAILURE marker to RESULT.
    #
    # As of sub-PR 7b the Class D recovery shim is retired — dispatch-lib's
    # iterate loop + canonical writer is the sole structural authority for the
    # body callout. The pilot's organic write in the dev-groom skill prompt
    # remains as a fallback until the dev-groom-prompt-update follow-up
    # ships. See docs/plans/2026-05-25-009-feat-1271-class-d-shim-retire-plan.md.
    if [ "$SKILL" = "dev-groom" ]; then
        if _iterate_groom_loop; then
            # mika#1394: Architect converged on GROOMED — unconditionally override
            # the outcome to PLAN_GROOMED. The previous sed only matched
            # "Outcome: PLAN_COMMITTED"; on re-dispatch the plan validation block
            # may have already set PIPELINE_INCOMPLETE (e.g., plan created on a
            # prior day), making the old sed a no-op. The canonical callout was
            # written and the grooming is complete — strip any stale PIPELINE
            # FAILURE markers and set the authoritative outcome.
            RESULT=$(printf '%s' "$RESULT" | sed '/^PIPELINE FAILURE:/d')
            RESULT=$(printf '%s' "$RESULT" | sed 's/Outcome: .*/Outcome: PLAN_GROOMED/')
            # Safety net: if no Outcome: line existed (edge case), append one.
            if ! printf '%s' "$RESULT" | grep -qF 'Outcome: PLAN_GROOMED'; then
                RESULT="${RESULT}

Outcome: PLAN_GROOMED"
            fi
        else
            # mika#1333: propagate architect-convergence failure into RESULT.
            # Replaces the silent-tolerance pattern that caused mid-flow
            # short-circuit (plan committed but architect never ran/failed).
            # mika#1394: match any Outcome: line (not just PLAN_COMMITTED) to
            # handle re-dispatch where PIPELINE_INCOMPLETE was already set.
            RESULT=$(printf '%s' "$RESULT" | sed 's/Outcome: .*/Outcome: PIPELINE_INCOMPLETE — architect convergence did not complete./')
            # If no Outcome: line existed, append one.
            if ! printf '%s' "$RESULT" | grep -qF 'Outcome: PIPELINE_INCOMPLETE'; then
                RESULT="${RESULT}

Outcome: PIPELINE_INCOMPLETE — architect convergence did not complete."
            fi
            RESULT="PIPELINE FAILURE: architect convergence did not complete (_iterate_groom_loop returned non-zero). Plan exists on branch but architect verdict is missing.

${RESULT}"
        fi
    fi

    _push_branch

    # Unit 2 (mika#1282 + mika#1396): open a draft PR when content was rescued
    # by dispatch-lib's git-workflow ownership.
    #
    # Recovery classes:
    # - "dirty-worktree" (mika#1282 original): pilot wrote files but never committed.
    #   dispatch-lib staged + committed with wip() + pushed; this opens the PR.
    # - "commit-pushed-no-pr" (mika#1396): pilot committed AND pushed but
    #   gh pr create failed (e.g., AxiosError 5000ms timeout). Branch has the
    #   commit on origin; PR was never opened. dispatch-lib opens it.
    #
    # Runs after _push_branch (lines 558-564) and before _deliver_callback.
    local RECOVERY_CLASS=""
    if [ "${RESCUED_DIRTY_WORKTREE:-}" = "1" ]; then
        RECOVERY_CLASS="dirty-worktree"
    elif [ -z "$PR_URL" ] && [ -n "$PRE_RUN_HEAD" ] && [ -n "$POST_RUN_HEAD" ] \
         && [ "$PRE_RUN_HEAD" != "$POST_RUN_HEAD" ] && [ "$SKILL" = "dev-pilot" ]; then
        RECOVERY_CLASS="commit-pushed-no-pr"
    fi

    if [ -n "$RECOVERY_CLASS" ] && [ -n "$REPO" ] && [ -n "$BRANCH" ] && [ -z "$PR_URL" ]; then
        # Recovery-class-specific PR body
        local _rescue_title
        local _rescue_body_note
        if [ "$RECOVERY_CLASS" = "dirty-worktree" ]; then
            _rescue_title=$(_derive_recovery_pr_title "dirty-worktree" "$WORKTREE_DIR" "$REPO" "$ISSUE_NUM" "$LABELS" "$ISSUE_TITLE")
            _rescue_body_note="The dev-pilot session wrote file changes but never completed the git workflow (no \`git commit\` or \`gh pr create\`). Per the mika#1271 content/workflow split contract, dispatch-lib took ownership of the git layer: staged, committed with \`wip()\` prefix, pushed, and opened this draft PR to preserve the content.

**This is a draft PR requiring human review.** The content has NOT passed \`/ce:review\` and may contain partially-coherent multi-file changes."
        else
            _rescue_title=$(_derive_recovery_pr_title "commit-pushed-no-pr" "$WORKTREE_DIR" "$REPO" "$ISSUE_NUM" "$LABELS" "$ISSUE_TITLE")
            _rescue_body_note="The dev-pilot session committed and pushed the implementation but \`gh pr create\` failed (typically a transient AxiosError 5000ms timeout from claude-cli's internal HTTP relay). Branch is on origin with the impl commit; only the final PR-creation step needed recovery.

**This is a draft PR — operator should verify pilot's pipeline (/ce:work, /ce:review, /ce:compound) completed before marking ready.** The recovery path is uniform with mika#1282's draft-PR pattern for audit consistency."
        fi

        RESCUED_PR_URL=$(gh pr create \
            --repo "senara-solutions/$REPO" \
            --head "$BRANCH" \
            --base main \
            --draft \
            --title "$_rescue_title" \
            --body "$(cat <<RESCUEBODY
## Auto-rescued PR (dispatch-lib recovery, class: ${RECOVERY_CLASS})

This PR was created by dispatch-lib's git-workflow recovery.

${_rescue_body_note}

### Recovery metadata
- Recovery class: \`${RECOVERY_CLASS}\`
- Pilot session: \`${SESSION_ID:-unknown}\`
- Turns: ${TURNS:-unknown}
- Cost: \$${COST:-unknown}

Closes #${ISSUE_NUM}
RESCUEBODY
)" 2>&9 || true)

        if [ -n "$RESCUED_PR_URL" ]; then
            PR_URL="$RESCUED_PR_URL"
            # mika#1631: tag rescued PRs for staleness-probe targeting
            gh pr edit "$RESCUED_PR_URL" --add-label "wip-rescue" 2>&9 || true
            # mika#1352: emit canonical `PR:` line alongside the descriptive
            # `Draft PR (dispatch-lib recovery):` line. mika-dev's callback
            # parser (dispatcher.rs:1780) matches line-anchored `^PR: ` —
            # without this, claude_pilot.pr_url is never written and the
            # parent task false-fails as `callback_delivered_without_pr_url`
            # despite the rescued PR being open and reviewable. See mika#871
            # R4 for the canonical contract.
            RESULT="${RESULT}
Draft PR (dispatch-lib recovery): ${PR_URL}
PR: ${PR_URL}"
        fi
    fi

    _deliver_callback
}
