---
title: "bash set -x leaks secrets in trace files and callback payloads"
date: 2026-04-30
category: security-issues
module: dev-pilot
problem_type: security_issue
component: skills/bundled/_shared/dispatch-lib.sh
severity: high
tags:
  - bash
  - set-x
  - xtrace
  - bash_xtracefd
  - secret-leakage
  - github-pat
  - exit-trap
  - callback-delivery
  - regression
applies_when:
  - Adding a `set -x` / xtrace block to a shell script for diagnostic instrumentation
  - The script references secret-bearing env vars (`GH_TOKEN`, `GH_APP_TOKEN`, `MIKA_*_API_KEY`, `MIKA_INTERNAL_TOKEN`, etc.) anywhere after `set -x`
  - The script's EXIT trap captures trace tail and forwards it to a downstream consumer (callback delivery, log shipper, error reporter)
related_issues:
  - 887
  - 893
  - 903
last_updated: 2026-04-30
---

# bash `set -x` leaks secrets in trace files and callback payloads

## Context

Diagnostic shell instrumentation via `set -x` (BASH_XTRACEFD or stderr redirect) prints every command before execution **with its expanded arguments**. If any traced command touches a secret — including idiomatic `echo "$TOKEN" | gh auth login --with-token` — the secret value lands in plaintext in the trace destination. When the same trace is also piped to a downstream consumer (an EXIT-trap callback delivery, an error reporter, a log shipper), the leakage propagates to durable storage that may be world-readable, dashboard-visible, or backup-replicated.

mika#887 added a `BASH_XTRACEFD` trace to the dev-pilot dispatch handler for diagnosing silent-exit-0 crashes. The trace recipe didn't include redaction. mika#893's refactor migrated the trace into `_shared/dispatch-lib.sh`, broadening the blast radius across all dispatch-style handlers. The leak surfaced empirically on 2026-04-30 during R5 symmetry-test development: `/tmp/dev-pilot-trace-<pid>.log` contained literal `+ GH_APP_TOKEN=github_pat_<full token>` followed by `+ echo github_pat_<full token>`. The same trace tail was captured by the EXIT trap and delivered into mika.db's `tasks.result` and `messages.content` via `mika ask --task-complete`. World-readable `/tmp` mode + durable DB rows + dashboard visibility = three independent leakage paths from one untouched recipe.

## Guidance

When adding `set -x` (or any equivalent xtrace) to a shell script, treat secret-handling sections as untraced regions and the trace destination as untrusted output:

1. **Wrap every secret-handling command with `set +x` / `set -x`.** This is the simplest, most surgical defense and the one to reach for first. Group the wrapped block tightly — one `set +x` before the first secret expansion, one `set -x` immediately after the last.

2. **Restrict trace file permissions at creation time.** If the script writes a trace via `exec 9>>"$TRACE_FILE"`, set `umask 077` *before* the redirect (or `chmod 0600` immediately after) so the trace is owner-only readable. World-readable `/tmp` mode is the default and is the wrong default for files that may contain secrets even after redaction (timing oracles, partial leaks).

3. **Scrub the trace tail before forwarding it to any downstream consumer.** EXIT-trap-to-callback delivery is the most common case — the trap captures the last N lines of trace and pipes them into `mika ask --task-complete` (or equivalent). Add a regex sieve before the forward:

   ```sh
   _scrub_secrets() {
     sed -E '
       s/(github_pat_[A-Za-z0-9_]+)/<REDACTED-PAT>/g
       s/(MIKA_[A-Z_]*(TOKEN|KEY|SECRET))=([^[:space:]]+)/\1=<REDACTED>/g
       s/(GH_APP_TOKEN|GH_TOKEN)=([^[:space:]]+)/\1=<REDACTED>/g
     '
   }
   _TRACE_TAIL=$(tail -50 "$TRACE_FILE" | _scrub_secrets)
   ```

   Defense-in-depth: even if step 1 misses a path, the scrubber catches known token shapes before the trace leaves the host process.

4. **Audit existing storage when a leak is discovered.** Trace files in `/tmp` are ephemeral but durable storage isn't. Run a post-incident query against:

   ```sql
   SELECT id, created_at FROM tasks WHERE result LIKE '%github_pat_%' OR result LIKE '%MIKA_%TOKEN=%';
   SELECT id, session_id, created_at FROM messages WHERE content LIKE '%github_pat_%';
   ```

   Decide between (a) retain for forensics with restricted access, (b) scrub via UPDATE, (c) rotate the leaked credentials. Default to (c) when the leak window is unclear or the credential is hard to revoke.

   **Audit outcome (2026-04-30, mika#903):** the audit was actually run. The BASH_XTRACEFD trap-callback path produced **0 hits** in `tasks.result`, `messages.content`, or `tool_calls.output`/`.input` for `github_pat_`/`ghp_`/`+ GH_*_TOKEN=` patterns — the speculated trap-forward-to-durable-storage path did not materialize. The broadened audit (motivated by the rotation decision) surfaced a separate older leak class: `tool_calls.id='461c76a1-9a7e-47c5-94a7-d99ad4ab7624'` from 2026-04-13 contained a real PAT recorded by `read_agent_file` reading `mika-qa`'s `.env`. Different tool surface, different durability path, different fix shape. Tracked in mika#908; the row was scrubbed and the credential rotated 2026-04-30.

   **SQL gotcha for these queries:** SQLite `LIKE` treats `_` as a single-char wildcard, so `LIKE '%github_pat_%'` also matches `githubXpatY` shapes (e.g., text with `kg_pa` somewhere nearby). Use `LIKE '%github\_pat\_%' ESCAPE '\'` or prefer literal `instr()` matching: `instr(content, 'github_pat_') > 0`. The unescaped form produced false positives during the mika#903 audit before being corrected.

5. **Restructure secret expansion to avoid traced argv when possible.** Instead of `echo "$TOKEN" | gh auth login --with-token`, write `printf '%s' "$TOKEN" > "$TMPFILE"` (with `chmod 0600` on the tempfile) then `gh auth login --with-token < "$TMPFILE"`. The traced commands become `printf '%s' <REDACTED>` + `gh auth login --with-token < /tmp/foo` — neither expands the secret in the visible argv. Note: even this leaks via `set -x` argument expansion of `"$TOKEN"`; combine with `set +x` for the few seconds it matters.

## Why This Matters

The instrumentation purpose (mika#887: silent-exit-0 diagnosis) is genuinely valuable — without it, today's R5 symmetry test development couldn't have surfaced the handler crash root cause. Removing `set -x` would forfeit that value. The right move is preserving the trace **and** the secret-handling discipline simultaneously. mika#903 documents the regression and the multi-surface fix; this entry generalizes the lesson so the next handler instrumented with xtrace doesn't repeat the regression.

The contrast with the rest of mika's codebase is instructive: `crates/mika-agent/CLAUDE.md` § Secrets describes a thorough discipline — `SecretString`, redacted Debug, scrubbed env vars in exec children — but that discipline lives in the Rust agent and never reached the bash handler. **A diagnostic addition in shell silently bypassed the Rust-side credential discipline.** The class of mistake is not bash-specific; it's "instrumentation introduced after the security review."

## When to Apply

Whenever any of the following:

- Adding `set -x` or `BASH_XTRACEFD`-style xtrace to a script that touches secrets (now or in the future — secret-handling can be added by a later refactor)
- Capturing `tail` of a trace file in any error-reporting path (EXIT trap, `trap ERR`, `set -e` cleanup, etc.)
- Writing trace/log/diagnostic output to `/tmp` or any other shared filesystem location
- Reviewing a PR that adds shell instrumentation — check both that secret-handling is wrapped AND that error-reporting paths sanitize before forwarding

The check is structural: grep the script for `set -x` and for known secret variable names, and verify there is at minimum a `set +x` block bracketing every secret reference. If the script has an EXIT trap that captures trace state, verify the trap sanitizes before forwarding.

## Examples

### Bug shape — what the leak looks like

```
$ tail /tmp/dev-pilot-trace-2998.log
+ _setup_gh_auth
++ mika --agent mika-dev token github
+ GH_APP_TOKEN=github_pat_11CBQ5YXY0V8S9CUXPmdLE_<...rest of PAT in plaintext...>
+ echo github_pat_11CBQ5YXY0V8S9CUXPmdLE_<...rest of PAT in plaintext...>
+ gh auth login --with-token
+ gh auth switch --user 'mika-platform-bot[bot]'
```

The same lines (with the literal PAT) end up in:
- `tasks.result` column when the EXIT trap fires on a crash and pipes the trace tail through `mika ask --task-complete`
- `messages.content` column for the mika-dev session that received the callback delivery
- The dashboard view of that session (anyone with dashboard auth sees the PAT)
- Any backup of mika.db taken between the leak and the credential rotation

### Fix shape — `set +x` wrap (smallest defense)

```sh
# Before:
_setup_gh_auth() {
    GH_APP_TOKEN=$(mika --agent mika-dev token github)
    if [ -n "$GH_APP_TOKEN" ]; then
        echo "$GH_APP_TOKEN" | gh auth login --with-token
        gh auth switch --user 'mika-platform-bot[bot]'
    fi
}

# After:
_setup_gh_auth() {
    set +x  # SECURITY: suppress xtrace for secret expansion (mika#903)
    GH_APP_TOKEN=$(mika --agent mika-dev token github)
    if [ -n "$GH_APP_TOKEN" ]; then
        echo "$GH_APP_TOKEN" | gh auth login --with-token
        gh auth switch --user 'mika-platform-bot[bot]'
    fi
    set -x  # restore xtrace
}
```

### Fix shape — trap-side scrubbing (defense-in-depth)

```sh
_scrub_secrets() {
    sed -E '
      s/github_pat_[A-Za-z0-9_]+/<REDACTED-PAT>/g
      s/(MIKA_[A-Z_]*(TOKEN|KEY|SECRET))=[^[:space:]]+/\1=<REDACTED>/g
      s/(GH_APP_TOKEN|GH_TOKEN)=[^[:space:]]+/\1=<REDACTED>/g
    '
}

trap '
  _EXIT_CODE=$?
  if [ -z "$RESULT" ] && [ -f "$TRACE_FILE" ]; then
      _TRACE_TAIL=$(tail -50 "$TRACE_FILE" | _scrub_secrets)
      RESULT="HANDLER CRASH (exit $_EXIT_CODE).\n\nTrace tail:\n$_TRACE_TAIL"
  fi
  printf "%s" "$RESULT"
' EXIT
```

### Fix shape — file mode at creation time

```sh
# Before: trace file is mode 0644 (world-readable)
exec 9>>/tmp/dev-pilot-trace-$$.log
BASH_XTRACEFD=9
set -x

# After: trace file is mode 0600 (owner-only)
umask 077
exec 9>>/tmp/dev-pilot-trace-$$.log
chmod 0600 /tmp/dev-pilot-trace-$$.log 2>/dev/null  # belt-and-suspenders
BASH_XTRACEFD=9
set -x
```

## Related

- mika#887 — original BASH_XTRACEFD trace injection (trace recipe). The recipe itself is correct for the silent-exit-0 diagnostic value; the regression is everything-around-the-recipe (no `set +x` wrap, no file mode restriction, no trap-side scrubbing).
- mika#893 — refactor that migrated the trace into `_shared/dispatch-lib.sh`. Same code, broader blast radius — both dev-pilot and dev-groom now share the leak surface.
- mika#903 — security ticket for the dev-pilot-specific regression. Filed 2026-04-30; tracks the corrective fix.
- `mika/CLAUDE.md` § Secrets — the Rust-side credential discipline this regression silently bypassed. Cross-language credential discipline doesn't propagate automatically.
- `mika/docs/solutions/dev-loop/dev-pilot-handler-silent-exit-0-pattern-2026-04-29.md` — the diagnostic compound doc that introduced the trace recipe. Cross-reference: that recipe should now embed the `set +x` wrap as part of the canonical pattern.
