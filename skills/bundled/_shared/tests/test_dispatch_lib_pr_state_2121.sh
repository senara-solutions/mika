#!/bin/bash
# Le callback nomme toujours son état de PR (mika#2121 U1) — extrait de test-dispatch-lib.sh par mika#3178, vol 3.
# Avec le contrat de mika#138 et la lecture ancrée de `_set_up_worktree` ; le
# motif parti de `_detect_plan_on_branch` (mika#2120) et la délégation de
# `_extract_plan_path` (mika#2194) ; la règle de confinement du corps de PR
# (mika#2211) ; le here-string obligatoire (mika#2055/#3130) ; le scratch
# désigné du pilote (mika#2548).
#
# Plage d'origine : lignes 7322–8004 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 8de06d66. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ============================================================================
# mika#2121 (U1): the callback always names its PR state
# ============================================================================

echo ""
echo "Test: NO_PR total-output contract (mika#2121 U1)"
echo "------------------------------------------------"

# The lib was sourced at the iterate-loop section above, so the helpers are
# callable here. _classify_no_pr_reason is pure; _set_pr_status_line mutates the
# global RESULT, so every call below runs inside a subshell that owns its own
# RESULT — the mutation never escapes the assertion.

# --- _classify_no_pr_reason: one token per state, unset diagnosed first ---
assert_eq "reason: repo unset (before exit code)" "repo_unset" \
    "$(_classify_no_pr_reason '' 'some-branch' 1)"
assert_eq "reason: branch unset (before exit code)" "branch_unset" \
    "$(_classify_no_pr_reason 'mika' '' 1)"
assert_eq "reason: gh query failed" "gh_query_failed" \
    "$(_classify_no_pr_reason 'mika' 'some-branch' 1)"
assert_eq "reason: no pr on branch (clean query, empty result)" "no_pr_on_branch" \
    "$(_classify_no_pr_reason 'mika' 'some-branch' 0)"

# --- _set_pr_status_line: exactly one PR-status line, never two ---
# The site-2-then-site-3 sequence: main path finds no PR (writes NO_PR), the
# rescue then opens one (writes PR). The delivered callback must carry ONLY the
# later, truer line.
assert_eq "totality: NO_PR then PR keeps only PR" "PR: https://github.com/x/pull/1" \
    "$(RESULT='some prefix line'; _set_pr_status_line 'NO_PR: no_pr_on_branch'; _set_pr_status_line 'PR: https://github.com/x/pull/1'; printf '%s' "$RESULT" | grep -E '^(PR|NO_PR): ' || true)"
assert_eq "totality: exactly one PR-status line after two emissions" "1" \
    "$(RESULT='some prefix line'; _set_pr_status_line 'NO_PR: no_pr_on_branch'; _set_pr_status_line 'PR: https://github.com/x/pull/1'; printf '%s' "$RESULT" | grep -cE '^(PR|NO_PR): ' || true)"
# The reverse (rescue-fail supersedes site-2 no_pr_on_branch with the specific
# rescue_pr_create_failed) must also leave exactly one line.
assert_eq "totality: NO_PR then more-specific NO_PR keeps only the latter" "NO_PR: rescue_pr_create_failed" \
    "$(RESULT='x'; _set_pr_status_line 'NO_PR: no_pr_on_branch'; _set_pr_status_line 'NO_PR: rescue_pr_create_failed'; printf '%s' "$RESULT" | grep -E '^(PR|NO_PR): ' || true)"
# A non-status prefix line survives untouched (the strip is anchored, not greedy).
assert_contains "totality: strip does not eat non-status lines" "Outcome: PR_OPENED" \
    "$(RESULT='Outcome: PR_OPENED'; _set_pr_status_line 'PR: https://github.com/x/pull/1'; printf '%s' "$RESULT")"

# --- Structural guard: no emission site can go silent (jamais zéro) ---
# Each of the three sites must be able to emit a NO_PR line. If a future edit
# drops an `else`, the corresponding assert turns red instead of the site
# silently reverting to the pre-mika#2121 absence-means-no-PR behaviour that
# produced 306 mute failures.
TRAP_SRC=$(sed -n '/^_dispatch_lib_exit_trap() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "site 1 (crash trap) can emit NO_PR" 'NO_PR:' "$TRAP_SRC"

PFR_SRC=$(sed -n '/^_post_flight_recovery() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "site 2 (main path) can emit NO_PR" 'NO_PR:' "$PFR_SRC"

DCP_SRC=$(sed -n '/^dispatch_claude_pilot() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "site 3 (rescue) can emit NO_PR: rescue_pr_create_failed" 'rescue_pr_create_failed' "$DCP_SRC"

# === mika#2178 — the ticket context reaches the pilot (T1–T9) ===
#
# What these nine detectors measure: before mika#2178, `PROMPT` was exactly
# `<repo>#<num>`, and claude-pilot builds its opening prompt by plain
# concatenation, `f"{ns.command} {opening}"` (claude_pilot/cli.py:290
# interactive, :251 headless). On the plan-callout path the input became
# `/ce-work docs/plans/x.md mika#N`: neither the ticket body nor its comments
# reached the implementer. The seven readers of `ISSUE_BODY` are all internal to
# dispatch-lib and none of them writes into the prompt.
#
# Hermeticity (T9): no fixture calls `gh`. Everything is literal JSON handed to
# the helper on stdin — the suite runs on a runner with no network and no token.

echo ""
echo "Test: mika#2178 — ticket context in the pilot's opening prompt (T1–T9)"
echo "-----------------------------------------------------------------------"

# --- Section-local tooling ---------------------------------------------------

# Reproduces claude-pilot's opening-prompt assembly
# (`opening_prompt = f"{ns.command} {opening}"`). T1/T2 assert on THIS STRING —
# never on ISSUE_JSON nor on any dispatch-lib internal variable. That is the
# definition of AC3 (anti-vacuity).
_t2178_pilot_input() {
    printf '%s %s' "$1" "$2"
}

# Replays dispatch-lib's injection site. T3 pins the literal source line this
# mirror reproduces, so the mirror cannot drift silently.
_t2178_inject() {
    local repo="$1" issue_num="$2" issue_json="$3"
    local prompt ticket_context
    prompt="${repo}#${issue_num}"
    ticket_context=$(printf '%s' "$issue_json" | _render_ticket_context "$repo" "$issue_num" 2>/dev/null || true)
    if [ -n "$ticket_context" ]; then
        prompt=$(printf '%s\n\n%s' "$prompt" "$ticket_context")
    fi
    printf '%s' "$prompt"
}

_t2178_render() {
    printf '%s' "$1" | _render_ticket_context "${2:-mika}" "${3:-2178}" 2>/dev/null || true
}

_t2178_line_of() {
    grep -nF -- "$1" "$DISPATCH_LIB" 2>/dev/null | head -1 | cut -d: -f1
}

# "yes" when $1 appears further down the source than $2, "no" otherwise —
# including when either is absent: a position invariant over a site that does
# not exist is not satisfied.
_t2178_after() {
    local a b
    a=$(_t2178_line_of "$1")
    b=$(_t2178_line_of "$2")
    if [ -n "$a" ] && [ -n "$b" ] && [ "$a" -gt "$b" ]; then printf 'yes'; else printf 'no'; fi
}

_t2178_lineno_in() {
    printf '%s\n' "$2" | grep -nF -- "$1" | head -1 | cut -d: -f1
}

_t2178_assert_le() {
    local label="$1" limit="$2" actual="$3"
    if [ -n "$actual" ] && [ "$actual" -le "$limit" ] 2>/dev/null; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected: <= $limit"
        echo "    actual:   $actual"
    fi
}

T2178_SUW_SRC=$(sed -n '/^_set_up_worktree() {/,/^}/p' "$DISPATCH_LIB")
T2178_HELPER_SRC=$(sed -n '/^_render_ticket_context() {/,/^}/p' "$DISPATCH_LIB")

# --- T1: anti-vacuity, against the pilot's real input (AC3) ------------------

T2178_BODY_PHRASE="CONSIGNE-CORPS-2178 le plan se commite depuis un fichier deja present"
T2178_COMMENT_PHRASE="CONSIGNE-COMMENTAIRE-2178 aucune chaine cat redirigee vers un fichier temporaire"

# Excerpt from the mika#2158 grooming-enrichment comment of 2026-09-04T09:43:28Z.
# RECONSTRUCTED, not literal: the session that wrote these tests had no `gh`
# access (no token, no `gh auth`), and plan §1.3 is the only available source on
# that comment's content. The measured-vs-reconstructed distinction is the one
# crates/mika-agent/tests/fixtures/grooming_bodies/README.md already carries.
# What is load-bearing for AC3 is that one instruction lives in a comment and
# another in the body — not the literalness of the excerpt.
T2178_C2158_EXCERPT="Enrichissement de grooming (reconstruit) : la forme refusee par le
classifieur de permissions est la chaine \`cat > /tmp/plan.md <<EOF\`. Ecris le
plan avec l'outil d'ecriture de fichier, puis commite-le."

T2178_FIXTURE=$(jq -n \
    --arg body "Contexte du ticket.

${T2178_BODY_PHRASE}

> - **Plan:** \`docs/plans/x.md\` (committed on branch @ deadbee)" \
    --arg c1 "Premier commentaire, sans consigne particuliere." \
    --arg c2 "${T2178_COMMENT_PHRASE}

${T2178_C2158_EXCERPT}" \
    '{state:"OPEN", title:"le contexte du ticket atteint le pilote",
      labels:[{name:"ready"}], body:$body,
      comments:[
        {author:{login:"mika-platform-dev"}, createdAt:"2026-09-04T08:12:00Z", body:$c1},
        {author:{login:"samidarko"},         createdAt:"2026-09-04T09:43:28Z", body:$c2}
      ]}')

T2178_PROMPT=$(_t2178_inject "mika" "2178" "$T2178_FIXTURE")
T2178_INPUT_A=$(_t2178_pilot_input "/ce-work docs/plans/x.md" "$T2178_PROMPT")

assert_contains "T1: the BODY instruction reaches the pilot's real input (/ce-work)" \
    "$T2178_BODY_PHRASE" "$T2178_INPUT_A"
assert_contains "T1: the COMMENT instruction reaches the pilot's real input (/ce-work)" \
    "$T2178_COMMENT_PHRASE" "$T2178_INPUT_A"

# Red-before, term by term: neutralising ONE term does not pin the others, so
# three separate negative controls.

# (a) `--json` without `comments` — the comment phrase disappears, the body
#     phrase stays. This is what separates "we widened the fetch" from "we
#     widened it AND forwarded it".
T2178_FIXTURE_NO_COMMENTS=$(printf '%s' "$T2178_FIXTURE" | jq 'del(.comments)')
T2178_INPUT_NC=$(_t2178_pilot_input "/ce-work docs/plans/x.md" \
    "$(_t2178_inject "mika" "2178" "$T2178_FIXTURE_NO_COMMENTS")")
assert_not_contains "T1 negative control (a): without the comments field, the comment instruction is gone" \
    "$T2178_COMMENT_PHRASE" "$T2178_INPUT_NC"
assert_contains "T1 negative control (a): without the comments field, the body instruction remains" \
    "$T2178_BODY_PHRASE" "$T2178_INPUT_NC"

# (b) helper neutralised (renders the empty string) — both phrases disappear.
#     The neutralisation is real: the helper is shadowed by a subshell-local
#     redefinition, not simulated with a hand-written string. That proves the
#     append is fed by the helper and by nothing else.
T2178_INPUT_NOHELPER=$(
    _render_ticket_context() { :; }
    _t2178_pilot_input "/ce-work docs/plans/x.md" \
        "$(_t2178_inject "mika" "2178" "$T2178_FIXTURE")"
)
assert_not_contains "T1 negative control (b): helper neutralised, the body instruction is gone" \
    "$T2178_BODY_PHRASE" "$T2178_INPUT_NOHELPER"
assert_not_contains "T1 negative control (b): helper neutralised, the comment instruction is gone" \
    "$T2178_COMMENT_PHRASE" "$T2178_INPUT_NOHELPER"

# (c) helper intact but the append removed — the helper does render text, yet
#     the pilot's input stays `/ce-work docs/plans/x.md mika#2178`.
T2178_RENDER_ONLY=$(_t2178_render "$T2178_FIXTURE")
T2178_INPUT_NOAPPEND=$(_t2178_pilot_input "/ce-work docs/plans/x.md" "mika#2178")
assert_eq "T1 negative control (c): without the append, the pilot input is exactly the old one" \
    "/ce-work docs/plans/x.md mika#2178" "$T2178_INPUT_NOAPPEND"
assert_not_contains "T1 negative control (c): without the append, the body instruction never reaches the pilot" \
    "$T2178_BODY_PHRASE" "$T2178_INPUT_NOAPPEND"
assert_not_contains "T1 negative control (c): without the append, the comment instruction never reaches the pilot" \
    "$T2178_COMMENT_PHRASE" "$T2178_INPUT_NOAPPEND"
assert_contains "T1 negative control (c): the helper itself does render the comment instruction" \
    "$T2178_COMMENT_PHRASE" "$T2178_RENDER_ONLY"

# --- T2: symmetry of the two paths (AC1, AC2) --------------------------------

T2178_INPUT_B=$(_t2178_pilot_input "/mika" "$T2178_PROMPT")
assert_contains "T2: the BODY instruction also reaches the no-callout path (/mika)" \
    "$T2178_BODY_PHRASE" "$T2178_INPUT_B"
assert_contains "T2: the COMMENT instruction also reaches the no-callout path (/mika)" \
    "$T2178_COMMENT_PHRASE" "$T2178_INPUT_B"

# The injection is unconditional: at the point where it happens the entry
# command is not arbitrated yet (_detect_plan_on_branch runs after). A single
# occurrence IN CODE would reintroduce the asymmetry this ticket closes.
#
# The count excludes comment lines, like this suite's pre-existing structural
# guards (NON_COMMENT_EXIT1, NON_COMMENT_RETURNS): the comment explaining WHY
# the site cannot branch on the entry command is precisely what stops a future
# editor from branching there. An assertion demanding its silence would remove
# the reason to keep the rule.
assert_eq "T2: unconditional injection — no code-level branch on ENTRY_COMMAND in _set_up_worktree" \
    "0" "$(printf '%s\n' "$T2178_SUW_SRC" | grep -v '^[[:space:]]*#' | grep -c 'ENTRY_COMMAND' || true)"

# --- T3: position invariants (plan §3.3) -------------------------------------

T2178_APPEND_EXPR='PROMPT=$(printf '"'"'%s\n\n%s'"'"' "$PROMPT" "$TICKET_CONTEXT")'

# Invariant 1 — after the ITERATION_CTX branch, which REASSIGNS PROMPT from
# scratch instead of appending: injecting before it would overwrite the context
# silently whenever an iteration is in flight.
assert_eq "T3 invariant 1: the injection sits AFTER the ITERATION CONTEXT reassignment" "yes" \
    "$(_t2178_after 'PROMPT" "$TICKET_CONTEXT")' 'ITERATION CONTEXT:')"

# Invariant 2 — after the anchored repo#N parse
# (`^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$`): injecting before it makes the
# regex miss, dispatch falls into free-text mode and NO worktree is created.
assert_eq "T3 invariant 2: the injection sits AFTER the anchored repo#N parse" "yes" \
    "$(_t2178_after 'PROMPT" "$TICKET_CONTEXT")' '[a-zA-Z0-9_-]+#[0-9]+')"

# Invariant 3 — the injection lives INSIDE _set_up_worktree, hence before
# _detect_plan_on_branch and _handle_dry_run. The call order itself
# (_set_up_worktree → _detect_plan_on_branch → _handle_dry_run) is already
# asserted earlier in this suite ("Call ordering: ..."); T3 references that
# rather than duplicating it, and adds only the missing half here: the injection
# site is indeed inside the first of the three.
assert_contains "T3 invariant 3: the injection site lives inside _set_up_worktree" \
    '_render_ticket_context' "$T2178_SUW_SRC"

# T1's mirror is only legitimate if the append expression it replays is
# literally the one in the source.
assert_contains "T3: the source append expression is the one T1 replays" \
    "$T2178_APPEND_EXPR" "$(cat "$DISPATCH_LIB")"

# mika#138 contract + the anchored parse in _set_up_worktree: the FIRST LINE of
# PROMPT stays exactly `<repo>#<num>`. Strict equality, not assert_contains.
assert_eq "T3: the first line of PROMPT stays exactly mika#2178" "mika#2178" \
    "$(printf '%s' "$T2178_PROMPT" | head -1)"

# --- T4: readable rendering (AC1) --------------------------------------------

T2178_FX_ROLES=$(jq -n '{state:"OPEN", title:"t", labels:[], body:"corps du ticket de reference",
  comments:[
    {author:{login:"mika-platform-dev"}, createdAt:"2026-09-04T08:00:00Z",
     body:"Verdict QA: block[pipeline] — la CI est rouge sur le job clippy."},
    {author:{login:"samidarko"}, createdAt:"2026-09-04T09:43:28Z",
     body:"Consigne operateur: refais le plan avant de reimplementer."}
  ]}')
T2178_RENDER_ROLES=$(_t2178_render "$T2178_FX_ROLES")

assert_contains "T4: bot comment header — login, bot role, ISO timestamp" \
    "--- commentaire 1/2 · mika-platform-dev (bot) · 2026-09-04T08:00:00Z ---" "$T2178_RENDER_ROLES"
assert_contains "T4: human comment header — login, human role, ISO timestamp" \
    "--- commentaire 2/2 · samidarko (humain) · 2026-09-04T09:43:28Z ---" "$T2178_RENDER_ROLES"
assert_contains "T4: the ticket body sits under its own separator" \
    "--- corps du ticket ---" "$T2178_RENDER_ROLES"
assert_contains "T4: the header says what to do with the block, not just that it exists" \
    "Lis les deux avant d'agir." "$T2178_RENDER_ROLES"
assert_contains "T4: the bot's QA verdict is forwarded verbatim" \
    "block[pipeline]" "$T2178_RENDER_ROLES"

# --- T5: bounds --------------------------------------------------------------

# (a) 15 SHORT comments (≤ 200 B each) so that only the COUNT cap bites, never
#     the block cap. Without that size constraint on the fixture, a correct
#     implementation could render fewer than 10 (block-cap eviction) and the
#     test would go red for the wrong reason.
T2178_FX_15=$(jq -n '{state:"OPEN", title:"t", labels:[], body:"corps court",
  comments:[range(1;16) | {author:{login:"samidarko"},
                           createdAt:"2026-09-04T00:00:00Z",
                           body:"COMMENTAIRE-NUMERO-\(.)"}]}')
T2178_RENDER_15=$(_t2178_render "$T2178_FX_15")

assert_eq "T5(a): exactly 10 comments rendered out of 15" "10" \
    "$(printf '%s\n' "$T2178_RENDER_15" | grep -c '^--- commentaire ' || true)"
assert_contains "T5(a): the omission line carries the count 5" \
    "[5 commentaire(s) plus ancien(s) omis]" "$T2178_RENDER_15"
assert_contains "T5(a): the oldest rendered is 6/15" \
    "--- commentaire 6/15 · " "$T2178_RENDER_15"
assert_contains "T5(a): the most recent rendered is 15/15" \
    "--- commentaire 15/15 · " "$T2178_RENDER_15"
assert_not_contains "T5(a): 5/15 is indeed evicted" \
    "--- commentaire 5/15 · " "$T2178_RENDER_15"
assert_eq "T5(a): ascending chronological order (6/15 before 15/15)" "yes" \
    "$(a=$(_t2178_lineno_in '--- commentaire 6/15 · ' "$T2178_RENDER_15"); \
       b=$(_t2178_lineno_in '--- commentaire 15/15 · ' "$T2178_RENDER_15"); \
       if [ -n "$a" ] && [ -n "$b" ] && [ "$a" -lt "$b" ]; then printf 'yes'; else printf 'no'; fi)"

# (b) an 8 KiB comment → truncated to 4096 B with a marker.
T2178_FX_8K=$(jq -n '{state:"OPEN", title:"t", labels:[], body:"corps",
  comments:[{author:{login:"samidarko"}, createdAt:"2026-09-04T09:00:00Z", body:("Y" * 8192)}]}')
T2178_RENDER_8K=$(_t2178_render "$T2178_FX_8K")
assert_contains "T5(b): comment truncation marker present" \
    "[… tronqué à 4096 o]" "$T2178_RENDER_8K"
# Uppercase 'Y' appears nowhere else in the rendering (header, markers,
# timestamps), so the 'Y' count IS the length of the retained body.
assert_eq "T5(b): the comment body is cut at 4096 B" "4096" \
    "$(printf '%s' "$T2178_RENDER_8K" | tr -cd 'Y' | wc -c)"

# (c) 10 comments of 4 KiB → block ≤ 16384 B AND an omission line present.
T2178_FX_10x4K=$(jq -n '{state:"OPEN", title:"t", labels:[], body:"corps",
  comments:[range(1;11) | {author:{login:"samidarko"},
                           createdAt:"2026-09-04T00:00:00Z",
                           body:("W" * 4096)}]}')
T2178_RENDER_10x4K=$(_t2178_render "$T2178_FX_10x4K")
T2178_BLOCK_10x4K=$(printf '%s\n' "$T2178_RENDER_10x4K" | sed -n '/^--- commentaire /,$p')
_t2178_assert_le "T5(c): the comment block stays under 16384 B" "16384" \
    "$(printf '%s' "$T2178_BLOCK_10x4K" | wc -c)"
assert_eq "T5(c): omission line present when the block cap bites" "1" \
    "$(printf '%s\n' "$T2178_RENDER_10x4K" | grep -cE '^\[[0-9]+ commentaire\(s\) plus ancien\(s\) omis\]$' || true)"

# (d) a 32 KiB body → truncated to 16384 B with a marker.
T2178_FX_32K=$(jq -n '{state:"OPEN", title:"t", labels:[], body:("Q" * 32768), comments:[]}')
T2178_RENDER_32K=$(_t2178_render "$T2178_FX_32K")
assert_contains "T5(d): body truncation marker present" \
    "[… corps tronqué à 16384 o]" "$T2178_RENDER_32K"
assert_eq "T5(d): the body is cut at 16384 B" "16384" \
    "$(printf '%s' "$T2178_RENDER_32K" | tr -cd 'Q' | wc -c)"

# (e) payload beyond the pipe buffer (64 KiB on Linux). This is not a variant of
#     (b)/(d): it is the only case that separates a truncation written as
#     `printf … | head -c N` from one written without a pipe. With the pipe,
#     head exits after N bytes while printf is still writing, printf dies of
#     SIGPIPE, and under dispatch-lib's `set -euo pipefail` the substitution
#     fails and the whole dispatch aborts — the helper would render empty.
#     A GitHub issue body caps at 65 536 CHARACTERS, hence well past 64 KiB in
#     UTF-8: reachable on a real maximal ticket.
T2178_FX_HUGE=$(jq -n '{state:"OPEN", title:"t", labels:[], body:("Q" * 204800),
  comments:[{author:{login:"samidarko"}, createdAt:"2026-09-04T09:00:00Z", body:("Y" * 204800)}]}')
T2178_RENDER_HUGE=$(_t2178_render "$T2178_FX_HUGE")
assert_contains "T5(e): a payload > pipe buffer does not abort the helper (body)" \
    "[… corps tronqué à 16384 o]" "$T2178_RENDER_HUGE"
assert_contains "T5(e): a payload > pipe buffer does not abort the helper (comment)" \
    "[… tronqué à 4096 o]" "$T2178_RENDER_HUGE"
assert_eq "T5(e): the 200 KiB body is cut at 16384 B" "16384" \
    "$(printf '%s' "$T2178_RENDER_HUGE" | tr -cd 'Q' | wc -c)"
assert_eq "T5(e): the 200 KiB comment is cut at 4096 B" "4096" \
    "$(printf '%s' "$T2178_RENDER_HUGE" | tr -cd 'Y' | wc -c)"

# --- T6: degenerate inputs ---------------------------------------------------

T2178_FX_NO_COMMENTS=$(jq -n '{state:"OPEN", title:"t", labels:[],
  body:"CORPS-SEUL-DISTINCTIF-2178", comments:[]}')
T2178_RENDER_NO_COMMENTS=$(_t2178_render "$T2178_FX_NO_COMMENTS")
assert_not_contains "T6(a): no comment separator when comments is empty" \
    "--- commentaire " "$T2178_RENDER_NO_COMMENTS"
assert_not_contains "T6(a): no omission line when comments is empty" \
    "plus ancien(s) omis" "$T2178_RENDER_NO_COMMENTS"
assert_contains "T6(a): the body alone is present" \
    "CORPS-SEUL-DISTINCTIF-2178" "$T2178_RENDER_NO_COMMENTS"

T2178_FX_EMPTY=$(jq -n '{state:"OPEN", title:"t", labels:[], body:"", comments:[]}')
assert_eq "T6(b): with no body and no comment, the helper renders the empty string" "" \
    "$(_t2178_render "$T2178_FX_EMPTY")"
assert_eq "T6(b): with no body and no comment, PROMPT is STRICTLY equal to mika#2178" "mika#2178" \
    "$(_t2178_inject "mika" "2178" "$T2178_FX_EMPTY")"

# --- T7: non-regression of the seven ISSUE_BODY readers (AC4) ----------------
#
# INVARIANCE detector: it must be GREEN on unmodified main. If it goes red
# before any fix, the reading of the code the mika#2178 plan rests on is wrong —
# halt-and-surface, no allowlist (Fire-Disposition, option (c)).

assert_eq "T7: exactly 7 ISSUE_BODY readers (AC4 freezes them)" "7" \
    "$(grep -c 'ISSUE_BODY' "$DISPATCH_LIB" || true)"

T2178_ISSUE_BODY_LINES=$(grep -h 'ISSUE_BODY' "$DISPATCH_LIB" | sed 's/^[[:space:]]*//')
assert_contains "T7: reader 1 — ISSUE_BODY derived from ISSUE_JSON" \
    "ISSUE_BODY=\$(printf '%s' \"\$ISSUE_JSON\" | jq -r '.body // empty')" "$T2178_ISSUE_BODY_LINES"
assert_contains "T7: reader 2 — derive-branch-name --body-callout" \
    '--body-callout "$ISSUE_BODY")' "$T2178_ISSUE_BODY_LINES"
assert_contains "T7: reader 3 — anti-re-groom gate _committed_plan_on_branch (mika#2012)" \
    'if existing_plan=$(_committed_plan_on_branch "$SUB_REPO_DIR" "$BRANCH" "$ISSUE_BODY" "$REPO" "$ISSUE_NUM"); then' \
    "$T2178_ISSUE_BODY_LINES"
# Reader 4 a changé de forme avec mika#2608, et le lecteur est resté unique.
#
# Il portait `grep -qE -- '^> - \*\*Plan:\*\*'`, c'est-à-dire le second des deux
# lecteurs bash que la phase 1 de mika#2194 avait DÉLIBÉRÉMENT laissés en place.
# La phase 2 le fait déléguer : la distinction 0/1 de `mika plan-callout` EST la
# réponse booléenne que ce site cherchait, donc aucune sous-commande ni drapeau
# n'a été ajouté. Le site ne décide toujours rien — il journalise
# `dispatch_gate_groom_allowed_stale_callout` — et sa place dans la chaîne est
# inchangée (mika#2012 U4).
#
# Ce détecteur est un détecteur d'INVARIANCE : il doit rougir quand la forme d'un
# lecteur bouge, et c'est exactement ce qu'il a fait ici. L'attendu est donc mis à
# jour avec sa raison, jamais relâché en sous-chaîne.
assert_contains "T7: reader 4 — stale callout, délégué depuis mika#2608" \
    'elif _extract_plan_path "$ISSUE_BODY" >/dev/null 2>&1; then' \
    "$T2178_ISSUE_BODY_LINES"
assert_contains "T7: reader 5 — _detect_plan_on_branch guard" \
    '[ -n "$ISSUE_BODY" ] || return 0' "$T2178_ISSUE_BODY_LINES"
# Reader 6 a changé de forme deux fois, et le lecteur est resté unique.
#
# mika#2120 : le motif a quitté `_detect_plan_on_branch` pour
# `_extract_plan_path`, qui accepte les deux écritures du callout et normalise.
#
# mika#2194 : `_extract_plan_path` ne porte plus de motif du tout — elle délègue
# au lecteur Rust — et son code de retour est désormais CAPTURÉ au lieu d'être
# écrasé par `|| return 0`, parce qu'il porte trois valeurs et non deux
# (0 lu · 1 aucun callout · ≥2 illisible). C'est le livrable d'AC4 : confondre
# les deux dernières fabriquerait un silence que la migration vient de créer.
#
# Ce que T7 fige reste vrai et inchangé : il y a UN lecteur, et il lit
# `$ISSUE_BODY`.
assert_contains "T7: reader 6 — plan-path extraction" \
    'PLAN_PATH=$(_extract_plan_path "$ISSUE_BODY") || _plan_rc=$?' \
    "$T2178_ISSUE_BODY_LINES"
assert_contains "T7: reader 7 — plan-line rescue in _iterate_groom_loop" \
    'elif _groom_plan_path=$(_committed_plan_on_branch "$SUB_REPO_DIR" "$BRANCH" "$ISSUE_BODY" "$REPO" "$ISSUE_NUM" 2>/dev/null); then' \
    "$T2178_ISSUE_BODY_LINES"

# The helper reads ISSUE_JSON, never ISSUE_BODY — a shared access would couple
# the two surfaces and turn any change to the rendering into a change to the
# seven readers.
assert_not_contains "T7: _render_ticket_context never reads ISSUE_BODY" \
    "ISSUE_BODY" "$T2178_HELPER_SRC"

# --- T8: structure -----------------------------------------------------------

assert_eq "T8: --json state,title,labels,body,comments present at exactly ONE site" "1" \
    "$(grep -cF -- '--json state,title,labels,body,comments' "$DISPATCH_LIB" || true)"
assert_eq "T8: _render_ticket_context() defined exactly once" "1" \
    "$(grep -c '^_render_ticket_context() {' "$DISPATCH_LIB" || true)"
assert_contains "T8: the injection reads ISSUE_JSON (no second network round trip)" \
    'ISSUE_JSON" | _render_ticket_context' "$T2178_SUW_SRC"
# A second `gh issue view` would open a TOCTOU window between the state read by
# the issue-close gate and the comments — the same one the /mika-groom-ticket
# spec closed at its step 5a.
assert_eq "T8: exactly one gh issue view call in _set_up_worktree" "1" \
    "$(printf '%s\n' "$T2178_SUW_SRC" | grep -c 'gh issue view' || true)"

# --- T9: hermeticity ---------------------------------------------------------
#
# INVARIANCE detector: green on unmodified main. No test in this section calls
# `gh` — the fixtures are literal JSON handed to the helper on stdin, so the
# suite runs on a runner with no network and no token.

T2178_SECTION=$(sed -n '/^# === mika#2178 — the ticket context reaches the pilot/,$p' "${BASH_SOURCE[0]}")
# Guard the guard: if the section marker above ever drifts from the real header,
# the sed returns nothing and the two greps below pass on an empty haystack — a
# silent false green on the very check that certifies hermeticity.
assert_eq "T9: the section extraction actually found the section (guards the guard)" "yes" \
    "$(if [ -n "$T2178_SECTION" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "T9: no gh invocation at command head in the mika#2178 section" "0" \
    "$(printf '%s\n' "$T2178_SECTION" | grep -cE '^[[:space:]]*gh[[:space:]]' || true)"
assert_eq "T9: no gh invocation in a substitution in the mika#2178 section" "0" \
    "$(printf '%s\n' "$T2178_SECTION" | grep -cE '\$\(gh[[:space:]]' || true)"
assert_not_contains "T9: _render_ticket_context does not call gh (it reads stdin)" \
    "gh issue" "$T2178_HELPER_SRC"

# =============================================================================
# mika#2211 — the PR-body containment rule reaches the pilot
# =============================================================================
#
# Why a structural test and not a behavioural one: the defect is a pilot writing
# `/tmp/pr-body-<N>.md` on its own initiative, which no test in this repo can
# reproduce without launching a model. What CAN be pinned is the thing that was
# missing — that the rule is present, and present on the ONE channel the pilot
# actually reads. A test asserting only that some file somewhere contains the
# sentence would have stayed green through the exact failure this closes:
# mika-dev's own system prompt never reaches the pilot process.

T2211_SUW_SRC=$(sed -n '/^_set_up_worktree() {/,/^}/p' "$DISPATCH_LIB")

# --- The rule exists and is non-empty ---------------------------------------
assert_eq "mika#2211: _PR_BODY_CONTAINMENT_RULE is defined exactly once" "1" \
    "$(grep -c '^_PR_BODY_CONTAINMENT_RULE=' "$DISPATCH_LIB" || true)"

# --- It is injected into the prompt the pilot receives ----------------------
assert_contains "mika#2211: the rule is appended to PROMPT inside _set_up_worktree" \
    '_PR_BODY_CONTAINMENT_RULE' "$T2211_SUW_SRC"

# The same position invariant mika#2178 documents: the ITERATION_CTX branch
# REASSIGNS PROMPT from scratch, so an injection above it is dropped silently on
# every iteration — the failure would then reappear on exactly the reruns that
# follow a first attempt.
assert_eq "mika#2211: the injection sits AFTER the ITERATION CONTEXT reassignment" "yes" \
    "$(_t2178_after 'PROMPT" "$_PR_BODY_CONTAINMENT_RULE")' 'ITERATION CONTEXT:')"

# The mika#138 contract: the first line of PROMPT stays `<repo>#<num>`. The
# anchored parse in _set_up_worktree depends on it, so appending must never
# prepend.
# Here-string, et jamais `printf … | grep -q` (mika#2055, mika#3130) : `grep -q`
# sort au premier match et ferme le tube, `printf` prend EPIPE et sort 141, et
# sous le `pipefail` de l'en-tête ce 141 est promu au statut du pipeline — le
# `if` lirait alors FAUX sur une chaîne PRÉSENTE. Mesuré : rouge à la tentative 1
# du run 43079844489 (#3075), vert à la tentative 2 sur le même diff.
# Le needle sort en variable : il porte des quotes imbriquées, et le lire deux
# fois serait une occasion de le désaligner.
T2211_APPEND_NEEDLE='PROMPT=$(printf '"'"'%s\n\n%s'"'"' "$PROMPT" "$_PR_BODY_CONTAINMENT_RULE")'
assert_eq "mika#2211: the rule is APPENDED (repo#N stays the first line)" "yes" \
    "$(if grep -qF -- "$T2211_APPEND_NEEDLE" <<<"$T2211_SUW_SRC"; \
       then printf 'yes'; else printf 'no'; fi)"

# --- The rule says the three things that make it actionable -----------------
# Stated positively first: a pilot told only "not /tmp" still has to invent a
# replacement, and the one it reaches for next (a heredoc) breaks on a body
# containing its own delimiter.
T2211_RULE=$(sed -n '/^_PR_BODY_CONTAINMENT_RULE=/,/^$/p' "$DISPATCH_LIB")

assert_contains "mika#2211: the rule names the working form (--body-file under the worktree)" \
    '--body-file pr-body.md' "$T2211_RULE"
assert_contains "mika#2211: the rule forbids the measured failure path (/tmp)" \
    '/tmp' "$T2211_RULE"
assert_contains "mika#2211: the rule forbids the heredoc the pilot would reach for next" \
    "heredoc" "$T2211_RULE"
assert_contains "mika#2211: the rule forbids ending the session on a question" \
    "session morte" "$T2211_RULE"

# --- The target repo's own command file carries the canonical form ----------
# AC1. Checked here because this suite already resolves REPO_ROOT and runs on
# every PR; a prompt-only fix with no gate is how the pre-fix inline instruction
# survived unexamined.
T2211_MIKA_CMD="$REPO_ROOT/.claude/commands/mika.md"
if [ -f "$T2211_MIKA_CMD" ]; then
    T2211_CMD_SRC=$(cat "$T2211_MIKA_CMD")
    assert_contains "mika#2211 AC1: mika.md PR step uses --body-file under the worktree" \
        '--body-file pr-body.md' "$T2211_CMD_SRC"
    assert_contains "mika#2211 AC1: mika.md PR step forbids writing the body outside the worktree" \
        'Never write the PR body outside the worktree' "$T2211_CMD_SRC"
    # The residue guard: the ignore is what keeps a failed `gh pr create` from
    # leaving a file that `git status --porcelain` reports, which is precisely
    # the probe that triggers the mika#1282 wip-rescue draft.
    assert_contains "mika#2211: pr-body.md is gitignored (a residue must not read as dirty)" \
        'pr-body.md' "$(cat "$REPO_ROOT/.gitignore" 2>/dev/null || true)"
fi
# =============================================================================
# mika#2548 — le scratch du pilote : désigné, vide, invisible à git
# =============================================================================
#
# Fonctionnel, pas seulement structurel : la propriété qui compte est ce que
# `git status` rend dans un worktree LIÉ, et elle n'est lisible qu'en exécutant
# git. Le worktree est créé sous un chemin qui porte `/.claude/worktrees/`, la
# forme réelle d'un worktree de dispatch — sans quoi la garde mika#1943
# refuserait la remise à zéro et le test mesurerait autre chose que la prod.

_t2548_run() {
    local root base wt out=""
    root=$(mktemp -d)
    base="$root/repo"
    wt="$root/.claude/worktrees/fix-2548-probe/mika"
    mkdir -p "$base" "$(dirname "$wt")"
    git -C "$base" init -q
    git -C "$base" commit --allow-empty -q -m initial
    git -C "$base" worktree add -q -b t2548 "$wt" 2>/dev/null

    # S1 — le répertoire existe après le seed.
    _seed_pilot_scratch_dir "$wt" 2>/dev/null
    [ -d "$wt/.pilot-scratch" ] && out="${out}exists;"

    # S2 — un fixture écrit dedans (non vide, en sous-répertoire) est invisible.
    mkdir -p "$wt/.pilot-scratch/sub" && printf 'x\n' > "$wt/.pilot-scratch/sub/f"
    [ -z "$(git -C "$wt" status --porcelain)" ] && out="${out}scratch_invisible;"

    # S3 — contrôle positif du prédicat : un fichier HORS scratch reste visible,
    # sinon S2 prouverait seulement que `git status` ne rend jamais rien.
    printf 'y\n' > "$wt/visible.txt"
    [ -n "$(git -C "$wt" status --porcelain)" ] && out="${out}outside_visible;"
    rm -f "$wt/visible.txt"

    # S4 — idempotence : un second seed laisse UNE seule ligne d'exclusion.
    _seed_pilot_scratch_dir "$wt" 2>/dev/null
    out="${out}lines=$(grep -cxF '.pilot-scratch/' "$base/.git/info/exclude");"

    # S5 — remise à zéro : le résidu d'une session précédente a disparu, le
    # répertoire est là et vide (réutilisation de worktree, KTD6).
    printf 'stale\n' > "$wt/.pilot-scratch/stale"
    _seed_pilot_scratch_dir "$wt" 2>/dev/null
    if [ -d "$wt/.pilot-scratch" ] && [ -z "$(ls -A "$wt/.pilot-scratch")" ]; then
        out="${out}reset;"
    fi

    # S6 — un exclude préexistant sans saut de ligne final n'est pas concaténé.
    printf 'foo' > "$base/.git/info/exclude"
    _seed_pilot_scratch_dir "$wt" 2>/dev/null
    grep -qxF 'foo' "$base/.git/info/exclude" && grep -qxF '.pilot-scratch/' "$base/.git/info/exclude" \
        && out="${out}newline_guard;"

    # S7 — non-régression mika#1415 après la factorisation : un fichier semé par
    # _seed_worktree_slash_commands reste exclu.
    local plat="$root/platform"
    mkdir -p "$plat/.claude/commands"
    printf 'meta\n' > "$plat/.claude/commands/mika-groom-ticket.md"
    _seed_worktree_slash_commands "$plat" "$wt"
    [ -f "$wt/.claude/commands/mika-groom-ticket.md" ] && [ -z "$(git -C "$wt" status --porcelain)" ] \
        && out="${out}cmds_seed_ok;"

    git -C "$base" worktree remove --force "$wt" 2>/dev/null
    rm -rf "$root"
    printf '%s' "$out"
}

# Les jetons sont choisis pour qu'aucun ne soit sous-chaîne d'un autre :
# assert_contains est un test de sous-chaîne, et un premier jet (`clean;` contre
# `commands_still_clean;`) passait vert sans l'exclusion — vu au contrôle négatif.
T2548_OUT=$(_t2548_run)
assert_contains "mika#2548: .pilot-scratch existe après le seed" "exists;" "$T2548_OUT"
assert_contains "mika#2548: un fixture sous .pilot-scratch/ est invisible à git status" "scratch_invisible;" "$T2548_OUT"
assert_contains "mika#2548: contrôle positif — un fichier hors scratch reste visible" "outside_visible;" "$T2548_OUT"
assert_contains "mika#2548: deux seeds laissent une seule ligne d'exclusion" "lines=1;" "$T2548_OUT"
assert_contains "mika#2548: un résidu est vidé à la préparation suivante" "reset;" "$T2548_OUT"
assert_contains "mika#2548: un exclude sans saut de ligne final n'est pas concaténé" "newline_guard;" "$T2548_OUT"
assert_contains "mika#2548: le seeding des commandes (mika#1415) reste propre" "cmds_seed_ok;" "$T2548_OUT"

# --- Structure : le seed est câblé dans _set_up_worktree, et sa suppression
# passe par la garde mika#1943 (un rm -rf nu sur un chemin non prouvé est la
# classe que cette garde existe pour fermer).
T2548_SUW_SRC=$(sed -n '/^_set_up_worktree() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "mika#2548: _set_up_worktree appelle _seed_pilot_scratch_dir" \
    '_seed_pilot_scratch_dir "$WORKTREE_DIR"' "$T2548_SUW_SRC"
T2548_SEED_SRC=$(sed -n '/^_seed_pilot_scratch_dir() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "mika#2548: la remise à zéro passe par _assert_removable_worktree_path" \
    '_assert_removable_worktree_path "$scratch" seed_pilot_scratch_dir' "$T2548_SEED_SRC"

# --- U2 : la règle atteint le pilote, au bon endroit ------------------------
assert_eq "mika#2548: _PILOT_SCRATCH_RULE est définie exactement une fois" "1" \
    "$(grep -c '^_PILOT_SCRATCH_RULE=' "$DISPATCH_LIB" || true)"
assert_contains "mika#2548: la règle est APPENDUE au PROMPT (repo#N reste la 1re ligne)" \
    'PROMPT=$(printf '"'"'%s\n\n%s'"'"' "$PROMPT" "$_PILOT_SCRATCH_RULE")' "$T2548_SUW_SRC"
assert_eq "mika#2548: l'injection suit la réaffectation ITERATION CONTEXT" "yes" \
    "$(_t2178_after 'PROMPT" "$_PILOT_SCRATCH_RULE")' 'ITERATION CONTEXT:')"
assert_eq "mika#2548: l'injection suit la règle mika#2211" "yes" \
    "$(_t2178_after 'PROMPT" "$_PILOT_SCRATCH_RULE")' 'PROMPT" "$_PR_BODY_CONTAINMENT_RULE")')"
# La règle Fire-Disposition doit rester la plus récente pour le groomeur.
assert_eq "mika#2548: la règle Fire-Disposition (mika#2306) reste APRÈS la règle scratch" "yes" \
    "$(_t2178_after 'PROMPT" "$_FIRE_DISPOSITION_RULE")' 'PROMPT" "$_PILOT_SCRATCH_RULE")')"
# Inconditionnelle : l'injection n'est pas dans le bloc `if [ "$SKILL" = "dev-groom" ]`.
T2548_GROOM_BLOCK=$(printf '%s\n' "$T2548_SUW_SRC" | sed -n '/if \[ "\$SKILL" = "dev-groom" \]; then/,/^        fi/p')
assert_not_contains "mika#2548: l'injection n'est pas conditionnée au skill" \
    '_PILOT_SCRATCH_RULE' "$T2548_GROOM_BLOCK"

T2548_RULE=$(bash -c 'source "$1" 2>/dev/null; printf "%s" "$_PILOT_SCRATCH_RULE"' _ "$DISPATCH_LIB")
assert_contains "mika#2548: la règle porte son étiquette de ticket" "mika#2548" "$T2548_RULE"
assert_contains "mika#2548: la règle nomme le lieu du scratch" ".pilot-scratch/" "$T2548_RULE"
assert_contains "mika#2548: la règle interdit rm nu" 'pas de `rm`' "$T2548_RULE"
assert_contains "mika#2548: la règle interdit rmdir" '`rmdir`' "$T2548_RULE"
assert_contains "mika#2548: la règle interdit rm -rf" '`rm -rf`' "$T2548_RULE"
assert_contains "mika#2548: la règle dit que le refus est terminal" "TERMINAL" "$T2548_RULE"
assert_contains "mika#2548: la règle écarte /tmp comme lieu de fixture" "/tmp" "$T2548_RULE"
# La commande qui a réellement échoué dans l'incident fondateur (mika#2054) était un
# `git show … -- > … 2>/dev/null; wc -l`, refusé pour sa forme et non pour sa cible :
# la règle nomme la seule forme que la politique autorise (cpp#35, bash-git-show-redirect).
assert_contains "mika#2548: la règle nomme la forme git show autorisée" \
    'git show <ref>:<chemin> > .pilot-scratch/<chemin>' "$T2548_RULE"
# L'interdit porte sur le scratch, pas sur tout fichier : sinon il contredit la
# règle mika#2211 (« puis supprime-le ») et un pr-body.md laissé à la racine est
# commité par le rescue dans les dépôts qui ne l'ignorent pas.
assert_contains "mika#2548: l'interdit de suppression est borné au scratch" \
    'Ne supprime JAMAIS un brouillon de `.pilot-scratch/`' "$T2548_RULE"
assert_contains "mika#2548: pr-body.md (mika#2211) est l'exception nommée" 'pr-body.md' "$T2548_RULE"
assert_not_contains "mika#2548: plus d'interdit général sur tout brouillon" \
    'Ne supprime JAMAIS un brouillon,' "$T2548_RULE"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
