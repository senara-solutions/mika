#!/bin/bash
# Nettoyage de reliques pré-vol et diagnostic double-échec (mika#1472) — extrait
# de test-dispatch-lib.sh par mika#3178, vol 2.
# Cette famille porte le `trap "rm -rf …" EXIT` de premier niveau (l. 4238 de
# l'agrégat) : c'est pour lui que l'ossature de mika#3178 ne porte AUCUN trap de
# résumé, et que chaque suite garde sa queue en clair — mesuré des deux côtés.
#
# Plage d'origine : lignes 3922–4989 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 890f3f5f. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ============================================================================
# Pre-flight stale-relic cleanup + dual-failure diagnostic (mika#1472)
# ============================================================================

echo ""
echo "Test: Pre-flight stale-relic cleanup (mika#1472 U1)"
echo "---------------------------------------------------"

# Extract the _set_up_worktree function body for structural assertions
SET_UP_WT_FUNC=$(sed -n '/^_set_up_worktree()/,/^}/p' "$DISPATCH_LIB")

# mika#2976 a EXTRAIT le geste « stash puis retire » en `_stash_and_remove_worktree`
# (deux appelants : ce site-ci, et `_reset_spent_branch`). Les six assertions U1
# ci-dessous se répartissent donc en deux moitiés, et aucune n'est perdue : la
# DÉTECTION du relic reste dans `_set_up_worktree`, le GESTE est dans le helper,
# et la DÉLÉGATION entre les deux est elle-même assertionnée. Un helper juste
# qu'aucun site n'appelle ne nettoierait rien.
STASH_REMOVE_FUNC=$(sed -n '/^_stash_and_remove_worktree()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2976 bonne foi: le corps de _stash_and_remove_worktree est non vide" "yes" \
    "$([ -n "$STASH_REMOVE_FUNC" ] && echo yes || echo non)"

# U1: Pre-flight block contains worktree list --porcelain detection
assert_contains "Pre-flight uses worktree list --porcelain" \
    'worktree list --porcelain' "$SET_UP_WT_FUNC"

# U1: Pre-flight block contains the branch-comparison awk script
assert_contains "Pre-flight awk matches refs/heads/\$BRANCH" \
    'refs/heads/$BRANCH' "$SET_UP_WT_FUNC"

# U1: Pre-flight delegates the relic's stash-then-remove to the single-site helper,
# passing the relic as the removal target and the CANONICAL worktree as the
# recovery target (mika#2449, now an argument rather than a global read).
assert_contains "Pre-flight délègue le geste au helper, relic retiré / canonique nommé" \
    '_stash_and_remove_worktree "$SUB_REPO_DIR" "$existing_wt" "$WORKTREE_DIR" "$BRANCH" set_up_worktree_relic' \
    "$SET_UP_WT_FUNC"

# U1: the helper carries the directory-exists guard
assert_contains "Helper has directory-exists guard for the removal target" \
    '[ -d "$remove_dir" ]' "$STASH_REMOVE_FUNC"

# U1: the helper checks dirty state via status --porcelain
assert_contains "Helper checks dirty state via status --porcelain" \
    'status --porcelain' "$STASH_REMOVE_FUNC"

# U1: the helper stashes with a descriptive name
assert_contains "Helper stashes with descriptive name (stash push --include-untracked -m)" \
    'stash push --include-untracked -m "$stash_name"' "$STASH_REMOVE_FUNC"

# U1: the helper keeps the stale-worktree-cleanup naming convention
assert_contains "Helper stash name uses dispatch-lib-stale-worktree-cleanup prefix" \
    'dispatch-lib-stale-worktree-cleanup' "$STASH_REMOVE_FUNC"

# U1: the helper removes the worktree with --force
assert_contains "Helper removes the worktree with --force" \
    'worktree remove --force "$remove_dir"' "$STASH_REMOVE_FUNC"

# U1: Call ordering — pre-flight cleanup runs BEFORE the existing dashed-path
# collision check (the "Reuse existing worktree if valid" block).
PREFLIGHT_LINE=$(printf '%s\n' "$SET_UP_WT_FUNC" | grep -n 'worktree list --porcelain' | head -1 | cut -d: -f1)
COLLISION_LINE=$(printf '%s\n' "$SET_UP_WT_FUNC" | grep -n 'Reuse existing worktree if valid' | head -1 | cut -d: -f1)
if [ -n "$PREFLIGHT_LINE" ] && [ -n "$COLLISION_LINE" ] && [ "$PREFLIGHT_LINE" -lt "$COLLISION_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Pre-flight cleanup (line $PREFLIGHT_LINE) runs before dashed-path collision check (line $COLLISION_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Pre-flight cleanup must run before dashed-path collision check"
    echo "    preflight_line=$PREFLIGHT_LINE collision_line=$COLLISION_LINE"
fi

echo ""
echo "Test: Dual-failure diagnostic (mika#1472 U2)"
echo "---------------------------------------------"

# U2: Wrapped block contains stderr-capture redirects
assert_contains "Worktree add captures stderr to wt-add-1-err temp file" \
    'wt_err_1' "$SET_UP_WT_FUNC"
assert_contains "Worktree add captures stderr to wt-add-2-err temp file" \
    'wt_err_2' "$SET_UP_WT_FUNC"

# U2: Wrapped block contains the worktree_setup_failed: diagnostic prefix
assert_contains "Dual-failure emits worktree_setup_failed: structured diagnostic" \
    'worktree_setup_failed:' "$SET_UP_WT_FUNC"

# U2: Diagnostic includes both attempts' stderr content
assert_contains "Diagnostic includes attempt 1 stderr (cat wt_err_1)" \
    'cat "$wt_err_1"' "$SET_UP_WT_FUNC"
assert_contains "Diagnostic includes attempt 2 stderr (cat wt_err_2)" \
    'cat "$wt_err_2"' "$SET_UP_WT_FUNC"

# U2: Temp files cleaned up in both success and dual-failure paths
CLEANUP_COUNT=$(printf '%s\n' "$SET_UP_WT_FUNC" | grep -c 'rm -f "$wt_err_1" "$wt_err_2"')
if [ "$CLEANUP_COUNT" -ge 2 ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Temp files cleaned up in both success and dual-failure paths ($CLEANUP_COUNT occurrences)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Temp file cleanup must appear in both success and failure paths"
    echo "    found $CLEANUP_COUNT occurrences, expected >= 2"
fi

# U2: Call ordering — worktree_setup_failed: appears AFTER the pre-flight cleanup
DIAGNOSTIC_LINE=$(printf '%s\n' "$SET_UP_WT_FUNC" | grep -n 'worktree_setup_failed:' | head -1 | cut -d: -f1)
if [ -n "$PREFLIGHT_LINE" ] && [ -n "$DIAGNOSTIC_LINE" ] && [ "$PREFLIGHT_LINE" -lt "$DIAGNOSTIC_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Pre-flight (line $PREFLIGHT_LINE) runs before dual-failure diagnostic (line $DIAGNOSTIC_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Pre-flight must run before dual-failure diagnostic"
    echo "    preflight_line=$PREFLIGHT_LINE diagnostic_line=$DIAGNOSTIC_LINE"
fi

echo ""
echo "Test: Doc comment on _set_up_worktree (mika#1472 U3)"
echo "----------------------------------------------------"

# U3: Doc comment exists above _set_up_worktree citing mika#1472
DOC_COMMENT=$(sed -n '/^# Pre-flight cleanup (mika#1472)/,/^_set_up_worktree()/p' "$DISPATCH_LIB")
assert_contains "Doc comment cites mika#1472" 'mika#1472' "$DOC_COMMENT"
assert_contains "Doc comment cites sibling mika#1414" 'mika#1414' "$DOC_COMMENT"
assert_contains "Doc comment cites sibling mika#1364" 'mika#1364' "$DOC_COMMENT"
assert_contains "Doc comment mentions worktree_setup_failed:" 'worktree_setup_failed:' "$DOC_COMMENT"

# --- Test: _label_to_type mapping (mika#1515) ---

echo ""
echo "Test: _label_to_type mapping (mika#1515)"
echo "------------------------------------------"

assert_eq "_label_to_type enhancement" "feat" "$(_label_to_type "enhancement")"
assert_eq "_label_to_type feature" "feat" "$(_label_to_type "feature")"
assert_eq "_label_to_type bug" "fix" "$(_label_to_type "bug")"
assert_eq "_label_to_type infrastructure" "chore" "$(_label_to_type "infrastructure")"
assert_eq "_label_to_type documentation" "docs" "$(_label_to_type "documentation")"
assert_eq "_label_to_type refactor" "refactor" "$(_label_to_type "refactor")"
assert_eq "_label_to_type test" "test" "$(_label_to_type "test")"
assert_eq "_label_to_type unknown label defaults to chore" "chore" "$(_label_to_type "priority:high")"
assert_eq "_label_to_type empty string defaults to chore" "chore" "$(_label_to_type "")"
assert_eq "_label_to_type comma-separated with enhancement" "feat" "$(_label_to_type "priority:high,enhancement")"
assert_eq "_label_to_type comma-separated with bug" "fix" "$(_label_to_type "component:agent,bug,ready")"

# --- Test: _derive_recovery_pr_title (mika#1515) ---

echo ""
echo "Test: _derive_recovery_pr_title (mika#1515)"
echo "----------------------------------------------"

# Set up a temporary git repo for commit-pushed-no-pr tests
_TEST_REPO_DIR=$(mktemp -d)
git -C "$_TEST_REPO_DIR" init -q 2>/dev/null
git -C "$_TEST_REPO_DIR" config user.email "test@test.com"
git -C "$_TEST_REPO_DIR" config user.name "Test"
echo "content" > "$_TEST_REPO_DIR/file.txt"
git -C "$_TEST_REPO_DIR" add file.txt
git -C "$_TEST_REPO_DIR" commit -q -m "feat(agent): add dispatch recovery (mika#1515)" 2>/dev/null

# commit-pushed-no-pr: should return the commit subject
assert_eq "commit-pushed-no-pr returns commit subject" \
    "feat(agent): add dispatch recovery (mika#1515)" \
    "$(_derive_recovery_pr_title "commit-pushed-no-pr" "$_TEST_REPO_DIR" "mika" "1515" "enhancement" "Some issue title")"

# dirty-worktree with plan file that has H1
mkdir -p "$_TEST_REPO_DIR/docs/plans"
echo '# PR titles on recovery should carry the conventional-commit subject' > "$_TEST_REPO_DIR/docs/plans/2026-06-14-005-1515-dispatch-lib-pr-titles-on-recovery-plan.md"

assert_eq "dirty-worktree with plan H1 constructs conventional title" \
    "feat: PR titles on recovery should carry the conventional-commit subject (mika#1515)" \
    "$(_derive_recovery_pr_title "dirty-worktree" "$_TEST_REPO_DIR" "mika" "1515" "enhancement" "Some issue title")"

# dirty-worktree with plan H1 that already has conventional-commit format
echo '# feat(dispatch-lib): PR titles on recovery' > "$_TEST_REPO_DIR/docs/plans/2026-06-14-005-1515-dispatch-lib-pr-titles-on-recovery-plan.md"

assert_eq "dirty-worktree with conventional-commit H1 passes through" \
    "feat(dispatch-lib): PR titles on recovery" \
    "$(_derive_recovery_pr_title "dirty-worktree" "$_TEST_REPO_DIR" "mika" "1515" "enhancement" "Some issue title")"

# dirty-worktree with no plan file → fallback to issue title
rm -rf "$_TEST_REPO_DIR/docs/plans"

assert_eq "dirty-worktree without plan falls back to issue title" \
    "feat: Some issue title (mika#1515)" \
    "$(_derive_recovery_pr_title "dirty-worktree" "$_TEST_REPO_DIR" "mika" "1515" "enhancement" "Some issue title")"

# dirty-worktree with bug label → fix prefix
assert_eq "dirty-worktree with bug label uses fix prefix" \
    "fix: Fix broken dispatch (mika#42)" \
    "$(_derive_recovery_pr_title "dirty-worktree" "$_TEST_REPO_DIR" "mika" "42" "bug" "Fix broken dispatch")"

# issue title already has conventional-commit format → pass through
assert_eq "issue title with conventional-commit passes through" \
    "fix(cli): handle edge case" \
    "$(_derive_recovery_pr_title "dirty-worktree" "$_TEST_REPO_DIR" "mika" "99" "bug" "fix(cli): handle edge case")"

# Cleanup
rm -rf "$_TEST_REPO_DIR"

# --- Test: Recovery block uses _derive_recovery_pr_title (mika#1515, structural) ---

echo ""
echo "Test: Recovery block uses _derive_recovery_pr_title (mika#1515, structural)"
echo "-----------------------------------------------------------------------------"

RECOVERY_BLOCK=$(sed -n '/Recovery classes:/,/RESCUED_PR_URL/p' "$DISPATCH_LIB")

assert_contains "dirty-worktree uses _derive_recovery_pr_title" \
    '_derive_recovery_pr_title "dirty-worktree"' "$RECOVERY_BLOCK"
assert_contains "commit-pushed-no-pr uses _derive_recovery_pr_title" \
    '_derive_recovery_pr_title "commit-pushed-no-pr"' "$RECOVERY_BLOCK"
assert_not_contains "No hardcoded wip() rescue title" \
    'rescued impl (dispatch-lib recovery)' "$RECOVERY_BLOCK"
assert_not_contains "No hardcoded pilot impl rescue title" \
    'pilot impl (dispatch-lib PR-create recovery' "$RECOVERY_BLOCK"

# --- Test 13: policy-deny disambiguation (drift-misdiagnosis fix) ---
# Mirrors investigation in docs/solutions/workflow-issues/
# 2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md.
# Verifies the new POLICY_DENY pre-check exists, precedes the existing
# drift-detection chain, reads from the persistent stderr path, and emits
# a distinct message that does NOT conflate the failure with LLM drift.

echo ""
echo "Test 13: Policy-deny disambiguation precedes drift detection (structural)"
echo "-------------------------------------------------------------------------"

DRIFT_BLOCK=$(sed -n '/Post-flight plan validation/,/Issue #138: Discover/p' "$DISPATCH_LIB")
# mika#3040 (G2): la lecture du refus a quitté les deux sites pour une fonction
# unique. Les trois claims qui portaient sur le CORPS de la lecture
# (initialisation, strip ANSI, marqueur cherché) se mesurent donc là où la
# lecture vit désormais ; les sites, eux, gardent leur claim propre — qu'ils
# passent bien par cet appel-là. Les assertions n'ont pas été supprimées : une
# assertion qui reste verte parce que son bloc contient encore le mot `sed` par
# ailleurs ne mesure plus rien.
POLICY_DENY_READ_BLOCK=$(sed -n '/^_policy_deny_read() {/,/^}/p' "$DISPATCH_LIB")

assert_contains "POLICY_DENY variable initialized (dans le lecteur unique, mika#3040)" \
    'POLICY_DENY=""' "$POLICY_DENY_READ_BLOCK"
assert_contains "le site dev-groom passe par le lecteur unique (mika#3040 AC3)" \
    '_policy_deny_read "$PERSISTENT_STDERR_PATH"' "$DRIFT_BLOCK"
# mika#2165 moved the directory behind the single resolver `_pilot_log_dir`;
# the LOG_ID convention this assertion exists for is unchanged, only its
# spelling is. The default literal now lives at exactly one site, pinned below.
# The resolver is called ON THE SAME LINE as the read — an assigning accessor
# (not a printing one, see the mika#2039 collision pinned below) can go stale,
# and co-location is what stops it.
assert_contains "PERSISTENT_STDERR_PATH uses LOG_ID convention" \
    '_pilot_log_dir; PERSISTENT_STDERR_PATH="$_PILOT_LOG_DIR/${LOG_ID}.stderr"' "$DRIFT_BLOCK"
assert_contains "Reads from persistent stderr (mika#1097 channel)" \
    '"$PERSISTENT_STDERR_PATH"' "$DRIFT_BLOCK"
assert_contains "Strips ANSI before grep (UI ANSI shouldn't break match)" \
    "sed 's/\\x1b\\[[0-9;]*[mK]//g'" "$POLICY_DENY_READ_BLOCK"
assert_contains "Searches for [policy:deny] marker" \
    '_policy_deny_excerpt' "$POLICY_DENY_READ_BLOCK"
assert_contains "Class C message identifies policy halt explicitly" \
    'halted by claude-pilot policy deny' "$DRIFT_BLOCK"
assert_contains "Class C message explicitly NOT drift" \
    'not LLM drift' "$DRIFT_BLOCK"
assert_contains "Class C message links to investigation doc" \
    'drift-misdiagnosis-policy-deny-halt' "$DRIFT_BLOCK"
# mika#2312: the remedies paragraph used to send the reader straight to
# "widen the policy" / "rewrite the dispatch context" without ever saying the
# deny names a refused CALL and a RULE. That reading is what produced mika#2312's
# inference that the CONTENT of a system_prompt.md was protected.
assert_contains "Class C message teaches rule-id reading first (dev-groom site)" \
    "Read the halt event's bracketed [rule-id] FIRST" "$DRIFT_BLOCK"
# The absent-rule-id semantics are the half mika#2312's first attempt got
# BACKWARDS: rule_id=None is claude-pilot's policy DEFAULT deny (policy.py
# returns rule_id=None when no rule matched; ui.py renders no tag for it), and
# permissions.yaml's default reason prescribes widening. Pin the corrected
# claim, not merely the sentence's presence — a message that is present and
# false is what this ticket exists to repair.
assert_contains "Class C message says an absent rule-id is the policy DEFAULT (dev-groom site)" \
    'NO [rule-id] means the policy DEFAULT fired' "$DRIFT_BLOCK"
assert_not_contains "Class C message does not blame the judgment stage (dev-groom site)" \
    'canUseTool judgment stage' "$DRIFT_BLOCK"

# Branch ordering: the POLICY_DENY branch must be the FIRST elif/if, so it wins
# over the drift messages when both conditions could fire.
POLICY_DENY_LINE=$(echo "$DRIFT_BLOCK" | grep -n 'if \[ -n "\$POLICY_DENY" \]' | head -1 | cut -d: -f1)
DRIFT_MSG_LINE=$(echo "$DRIFT_BLOCK" | grep -n 'pilot drifted into executor mode' | head -1 | cut -d: -f1)
if [ -n "$POLICY_DENY_LINE" ] && [ -n "$DRIFT_MSG_LINE" ] && [ "$POLICY_DENY_LINE" -lt "$DRIFT_MSG_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ POLICY_DENY branch precedes drift message in source"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ POLICY_DENY branch must precede drift message (POLICY_DENY_LINE=$POLICY_DENY_LINE, DRIFT_MSG_LINE=$DRIFT_MSG_LINE)"
fi

# Behavioral test — extract the policy-deny check logic into a function and
# exercise it with synthetic stderr fixtures.
echo ""
echo "Test 13b: Policy-deny check (behavioral, synthetic stderr fixtures)"
echo "-------------------------------------------------------------------"

# Minimal reproduction of the policy-deny check from dispatch-lib.sh.
# Kept in sync with the version in dispatch-lib.sh via the structural
# assertions above (mirrors the mika#1364 self-test approach).
_test_policy_deny_check() {
    local stderr_path="$1"
    local policy_deny=""
    if [ -f "$stderr_path" ] && [ -r "$stderr_path" ]; then
        policy_deny=$(sed 's/\x1b\[[0-9;]*[mK]//g' "$stderr_path" 2>/dev/null \
            | grep -m1 '\[policy:deny\]' || true)
    fi
    printf '%s' "$policy_deny"
}

POLICY_DENY_FIXTURE_DIR=$(mktemp -d)
trap "rm -rf '$POLICY_DENY_FIXTURE_DIR'" EXIT

# Fixture 1: stderr with a real ANSI-coded policy-deny line (today's mika#624 shape).
printf '\x1b[31m[policy:deny]\x1b[0m \x1b[1mBash\x1b[0m: gh auth status 2>&1 | head -10\n' \
    > "$POLICY_DENY_FIXTURE_DIR/with_deny.stderr"
RESULT=$(_test_policy_deny_check "$POLICY_DENY_FIXTURE_DIR/with_deny.stderr")
assert_contains "ANSI-stripped deny line extracted" \
    '[policy:deny] Bash: gh auth status' "$RESULT"

# Fixture 2: stderr without any policy-deny lines (healthy session).
printf '[init] Session abc123\n[done] Success | 5 turns | $0.20\n' \
    > "$POLICY_DENY_FIXTURE_DIR/clean.stderr"
RESULT=$(_test_policy_deny_check "$POLICY_DENY_FIXTURE_DIR/clean.stderr")
assert_eq "Clean stderr yields empty POLICY_DENY" "" "$RESULT"

# Fixture 3: stderr missing entirely (fail-open, fall through to drift checks).
RESULT=$(_test_policy_deny_check "$POLICY_DENY_FIXTURE_DIR/does_not_exist.stderr")
assert_eq "Missing stderr yields empty POLICY_DENY (fail-open)" "" "$RESULT"

# Fixture 4: deny line with rule-id suffix (today's mika#96 shape).
printf '\x1b[31m[policy:deny]\x1b[0m \x1b[1mBash\x1b[0m: grep -r "x" /tmp/ [bash-grep]\n' \
    > "$POLICY_DENY_FIXTURE_DIR/deny_with_rule.stderr"
RESULT=$(_test_policy_deny_check "$POLICY_DENY_FIXTURE_DIR/deny_with_rule.stderr")
assert_contains "Deny line with rule-id suffix extracted" \
    '[bash-grep]' "$RESULT"

# Fixture 5: multiple deny lines — only the first should be extracted (head -1
# behavior). Operators get the actionable signal without spam.
printf '[policy:deny] Bash: cmd1\n[policy:deny] Bash: cmd2\n[policy:deny] Bash: cmd3\n' \
    > "$POLICY_DENY_FIXTURE_DIR/multi_deny.stderr"
RESULT=$(_test_policy_deny_check "$POLICY_DENY_FIXTURE_DIR/multi_deny.stderr")
assert_contains "First deny extracted" "cmd1" "$RESULT"
assert_not_contains "Subsequent denies not included" "cmd2" "$RESULT"

# --- Test 14: policy-deny disambiguation extended to dev-pilot ---
# Companion to mika#1534 (dev-groom disambiguation). The Class C policy-deny
# check now also fires on dev-pilot post-flight before the generic "Zero new
# commits" / "HEAD unchanged" message. Validates the structural placement.

echo ""
echo "Test 14: Policy-deny disambiguation extended to dev-pilot (structural)"
echo "----------------------------------------------------------------------"

POSTFLIGHT_BLOCK=$(sed -n '/Post-flight diff check: detect zero-commit/,/Unit 1 (mika#1282)/p' "$DISPATCH_LIB")

assert_contains "Class C check fires on HEAD-unchanged for ALL skills (not just dev-groom)" \
    'Policy-deny pre-check (Class C disambiguation, extended to dev-pilot' "$POSTFLIGHT_BLOCK"
# mika#3040 (G2) : même déplacement qu'au Test 13 ci-dessus, et c'est le fond
# du geste — ce site et celui de dev-groom étaient des copies quasi-littérales,
# et c'est cette duplication qui a laissé l'extrait et la létalité répondre sur
# deux objets différents. Ce que ce site doit encore prouver, c'est qu'il passe
# par le MÊME appel.
assert_contains "POLICY_DENY set via the single reader in HEAD-unchanged path (mika#3040)" \
    '_policy_deny_read "$PERSISTENT_STDERR_PATH"' "$POSTFLIGHT_BLOCK"
assert_contains "Reads persistent stderr at LOG_ID path" \
    '_pilot_log_dir; PERSISTENT_STDERR_PATH="$_PILOT_LOG_DIR/${LOG_ID}.stderr"' "$POSTFLIGHT_BLOCK"
assert_contains "Strips ANSI before grep" \
    "sed 's/\\x1b\\[[0-9;]*[mK]//g'" "$POLICY_DENY_READ_BLOCK"
assert_contains "Searches for [policy:deny] marker" \
    '_policy_deny_excerpt' "$POLICY_DENY_READ_BLOCK"
assert_contains "Class C message — halted by policy deny, NOT generic exit" \
    'halted by policy deny — not generic exit' "$POSTFLIGHT_BLOCK"
assert_contains "Links to investigation doc" \
    'drift-misdiagnosis-policy-deny-halt' "$POSTFLIGHT_BLOCK"
# mika#2312 — companion of the dev-groom assertions in Test 13. Same phrases,
# second site: the two class-C blocks are NOT copies of one another (their
# headers and remedies paragraphs legitimately differ), so a single assertion
# would only bite on one of them.
assert_contains "Class C message teaches rule-id reading first (dev-pilot site)" \
    "Read the halt event's bracketed [rule-id] FIRST" "$POSTFLIGHT_BLOCK"
assert_contains "Class C message says an absent rule-id is the policy DEFAULT (dev-pilot site)" \
    'NO [rule-id] means the policy DEFAULT fired' "$POSTFLIGHT_BLOCK"
assert_not_contains "Class C message does not blame the judgment stage (dev-pilot site)" \
    'canUseTool judgment stage' "$POSTFLIGHT_BLOCK"

# Reading ORDER, not just presence (mika#2312): the rule-id instruction must
# come BEFORE the allow-list-gap paragraph — a reader who reaches "widen the
# policy" first goes hunting for a gap without knowing which rule fired, which
# is the sequence that produced this ticket. assert_contains cannot see order,
# so compare source line numbers the way the POLICY_DENY branch-ordering check
# below already does.
RULEID_LINE=$(echo "$POSTFLIGHT_BLOCK" | grep -n "bracketed \[rule-id\] FIRST" | head -1 | cut -d: -f1)
ALLOWLIST_LINE=$(echo "$POSTFLIGHT_BLOCK" | grep -n 'Likely a tier1 or tier2 allow-list gap' | head -1 | cut -d: -f1)
if [ -n "$RULEID_LINE" ] && [ -n "$ALLOWLIST_LINE" ] && [ "$RULEID_LINE" -lt "$ALLOWLIST_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ rule-id instruction precedes the allow-list-gap paragraph (dev-pilot site)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ rule-id instruction must precede the allow-list-gap paragraph (dev-pilot site)"
fi

# Branch ordering: POLICY_DENY must precede BOTH the dev-groom-re-dispatch
# Note AND the generic "Zero new commits" message in source order.
POLICY_LINE=$(echo "$POSTFLIGHT_BLOCK" | grep -n 'if \[ -n "\$POLICY_DENY" \]' | head -1 | cut -d: -f1)
GROOM_LINE=$(echo "$POSTFLIGHT_BLOCK" | grep -n 'HEAD unchanged on dev-groom re-dispatch' | head -1 | cut -d: -f1)
ZERO_COMMITS_LINE=$(echo "$POSTFLIGHT_BLOCK" | grep -n 'Zero new commits produced' | head -1 | cut -d: -f1)
if [ -n "$POLICY_LINE" ] && [ -n "$GROOM_LINE" ] && [ -n "$ZERO_COMMITS_LINE" ] \
    && [ "$POLICY_LINE" -lt "$GROOM_LINE" ] && [ "$POLICY_LINE" -lt "$ZERO_COMMITS_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ POLICY_DENY branch precedes both dev-groom-note and zero-commits messages"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ POLICY_DENY branch must precede both messages in HEAD-unchanged block"
    echo "    (POLICY_LINE=$POLICY_LINE, GROOM_LINE=$GROOM_LINE, ZERO_COMMITS_LINE=$ZERO_COMMITS_LINE)"
fi

# --- Test 15: mika#1383 structural completion gate — Phase A only, no PR (mika#1679) ---
#
# mika#1383 originally auto-created a PR when the pilot committed but didn't
# reach `gh pr create`. mika#1679 OVERTURNED that: the gate opened a NON-draft
# PR and set the global PR_URL, which SHADOWED the mika#1396 commit-pushed-no-pr
# rescue (Path B) — letting a non-draft PR bypass the mika#1613 recovery guards
# (evidence mika#PR1678/#PR1683). Under R2 the gate keeps only Phase A (trailing
# dirty rescue) and defers ALL PR creation to Path B (single source of truth).
# These tests assert the gate no longer creates a PR and the deferral is wired.

echo ""
echo "Test 15: mika#1383 gate — Phase A retained, PR creation deferred to Path B (mika#1679)"
echo "------------------------------------------------------------------------------------"

# Block extraction: from the gate's marker comment to the next major section.
GATE_BLOCK=$(sed -n '/mika#1383: structural completion gate/,/Post-flight plan validation/p' "$DISPATCH_LIB")

assert_contains "Gate references mika#1271 (content/workflow split)" \
    'mika#1271' "$GATE_BLOCK"
assert_contains "Gate references mika#1282 (companion handler for HEAD-unchanged + dirty)" \
    'mika#1282' "$GATE_BLOCK"
assert_contains "Gate scoped to dev-pilot only (groom intentionally has no PR)" \
    'SKILL" = "dev-pilot"' "$GATE_BLOCK"
assert_contains "Gate fires only when HEAD has advanced (PRE != POST)" \
    'PRE_RUN_HEAD" != "$POST_RUN_HEAD"' "$GATE_BLOCK"
assert_contains "Phase A: trailing dirty rescue with wip() prefix" \
    'wip(${REPO}#${ISSUE_NUM}): trailing content after pilot end_turn (mika#1383)' "$GATE_BLOCK"
# mika#2348 D2: the exclusion is the shared RESCUE_EXCLUDE_PATHSPEC array, not
# a literal on the add line — assert the gate expands it, and that the array
# still carries the two scaffold exclusions (same list as mika#1282, by
# construction rather than by copy).
GATE_EXCLUDE_DEF=$(grep -E '^RESCUE_EXCLUDE_PATHSPEC=\(' "$DISPATCH_LIB" | head -1)
assert_contains "Phase A: same scaffold-path exclusion as mika#1282 (git add expands RESCUE_EXCLUDE_PATHSPEC)" \
    'add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}"' "$GATE_BLOCK"
assert_contains "Phase A: RESCUE_EXCLUDE_PATHSPEC excludes :!.claude/commands/" \
    ":!.claude/commands/" "$GATE_EXCLUDE_DEF"
assert_contains "Phase A: RESCUE_EXCLUDE_PATHSPEC excludes :!.claude/claude-pilot.json" \
    ":!.claude/claude-pilot.json" "$GATE_EXCLUDE_DEF"

# mika#1679 (AC1): the gate must NOT create a PR. Path A creating a PR is what
# set PR_URL and shadowed Path B. The negative checks run against the CODE only
# (comment lines stripped) — the comments legitimately document the removed
# behavior and the shadow history, so they must not trip the regression net.
GATE_CODE=$(printf '%s\n' "$GATE_BLOCK" | grep -v '^[[:space:]]*#')
assert_not_contains "AC1: gate code no longer invokes gh pr create (deferred to Path B)" \
    'gh pr create' "$GATE_CODE"
assert_not_contains "AC1: gate code no longer lists PRs for an existence check" \
    'gh pr list --repo "senara-solutions/$REPO" --head "$BRANCH"' "$GATE_CODE"
assert_not_contains "AC1: gate code no longer emits the auto-created-PR result line" \
    'auto-created PR' "$GATE_CODE"
assert_not_contains "AC1: gate code no longer surfaces the manual-recovery PIPELINE FAILURE" \
    'PIPELINE FAILURE: pilot produced commits on' "$GATE_CODE"

# mika#1679: deferral to Path B (mika#1396 commit-pushed-no-pr) is documented in
# the gate so the shadow cannot be silently reintroduced.
assert_contains "Gate documents deferral to the mika#1396 commit-pushed-no-pr rescue" \
    'mika#1396' "$GATE_BLOCK"
assert_contains "Gate names mika#1679 as the reason PR creation moved out" \
    'mika#1679' "$GATE_BLOCK"

# Structural placement: the gate must fire AFTER the mika#1282 dirty-rescue
# (which only handles HEAD-unchanged) and BEFORE the dev-groom-specific
# post-flight plan validation. Both bounds verified by source order.
GATE_LINE=$(grep -n 'mika#1383: structural completion gate' "$DISPATCH_LIB" | head -1 | cut -d: -f1)
M1282_LINE=$(grep -n 'Unit 1 (mika#1282): detect dirty worktree' "$DISPATCH_LIB" | head -1 | cut -d: -f1)
GROOM_PLAN_LINE=$(grep -n 'Post-flight plan validation (mika#1033' "$DISPATCH_LIB" | head -1 | cut -d: -f1)
if [ -n "$GATE_LINE" ] && [ -n "$M1282_LINE" ] && [ -n "$GROOM_PLAN_LINE" ] \
    && [ "$M1282_LINE" -lt "$GATE_LINE" ] && [ "$GATE_LINE" -lt "$GROOM_PLAN_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Gate placement: after mika#1282 dirty-rescue, before dev-groom plan validation"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Gate placement violated source-order invariant"
    echo "    (GATE_LINE=$GATE_LINE, M1282_LINE=$M1282_LINE, GROOM_PLAN_LINE=$GROOM_PLAN_LINE)"
fi

# --- Test 15b: Path B (mika#1396) owns the commit-pushed-no-pr rescue PR (mika#1679) ---
#
# After mika#1679, the mika#1383 trigger flows to Path B's commit-pushed-no-pr
# branch. Path B is the single source of truth for the rescue-PR shape: --draft,
# rescue header, RECOVERY_PENDING marker, wip-rescue label, canonical PR: line,
# plus (mika#1679 Edit 2 / AC6) a wip(mika#1383) marker commit so Guard 2's
# `isDraft AND ^wip\(` conjunction fires.

echo ""
echo "Test 15b: Path B owns commit-pushed-no-pr rescue + Guard 2 marker commit (mika#1679)"
echo "-----------------------------------------------------------------------------------"

# Extract Path B (the mika#1282 + mika#1396 draft-PR rescue) up to its callback.
PATHB_BLOCK=$(sed -n '/Unit 2 (mika#1282 + mika#1396): open a draft PR/,/^    _deliver_callback/p' "$DISPATCH_LIB")

# Needle avoids a leading '--' so grep doesn't parse it as a flag; 'draft \'
# uniquely matches the `--draft \` line of the gh pr create invocation.
assert_contains "AC3: Path B opens the rescue PR as draft" \
    'draft \' "$PATHB_BLOCK"
# mika#2157 extracted the body out of the inline heredoc into
# `_compose_rescue_pr_body` so the closing-reference decision could be tested
# against real git repositories instead of a stubbed `gh` argv. The property
# these two assertions pin is unchanged — Path B's PR body carries the rescue
# header and the verification marker — so they now follow it across the
# extraction: Path B must route through the composer, and the composer must
# emit both. Splitting the assertion is what keeps it measuring the shipped
# body rather than a heredoc that no longer exists.
assert_contains "AC3: Path B composes its body through _compose_rescue_pr_body (mika#2157)" \
    '_compose_rescue_pr_body "$WORKTREE_DIR"' "$PATHB_BLOCK"
RESCUE_BODY_FN=$(awk '/^_compose_rescue_pr_body\(\) \{/,/^\}/' "$DISPATCH_LIB")
assert_contains "AC3: Path B writes the Auto-rescued PR rescue header (qa-review Step 1.5)" \
    '## Auto-rescued PR (dispatch-lib recovery, class: ${recovery_class})' "$RESCUE_BODY_FN"
# mika#2354: the marker is no longer the hard-coded literal `no` — it is the
# measured verdict, interpolated. Asserting `rescue-pipeline-verified: no` here
# would pin the very defect that ticket exists to remove, so the assertion
# follows the property (a marker IS emitted, and it carries the measured value)
# rather than the literal. The two values it can take are covered end-to-end in
# `tests/test_rescue_pipeline_verified.sh` against real repositories.
assert_contains "AC3: Path B emits the rescue-pipeline-verified marker (mika#2354: measured, not literal)" \
    'rescue-pipeline-verified: ${verified}' "$RESCUE_BODY_FN"
assert_not_contains "AC3/mika#2354: no hard-coded 'no' literal survives in the composer" \
    'rescue-pipeline-verified: no -->' "$RESCUE_BODY_FN"
assert_contains "AC3/mika#2354: Path B measures the pipeline before opening the PR" \
    '_measure_pipeline_verified "$WORKTREE_DIR"' "$PATHB_BLOCK"
assert_contains "AC3/mika#2354: the measurement is behind the kill-switch" \
    'if _rescue_verify_enabled; then' "$PATHB_BLOCK"
assert_contains "AC3/mika#2354: the measured verdict is passed to the composer" \
    '"$_rescue_verified" "$_rescue_verify_term" "$_rescue_verify_excerpt"' "$PATHB_BLOCK"

# mika#2354 AC10: term 5 is invoked with `origin/main`, never the script's own
# `main` default. A dispatch worktree's local `main` can be days stale, so the
# docs/source bucket split would be computed on a diff that is not the one the
# PR publishes. Structural assertion — the wrong base ref produces a plausible
# verdict, not an error, so no behavioural test can see it.
MEASURE_FN=$(awk '/^_measure_pipeline_verified\(\) \{/,/^\}/' "$DISPATCH_LIB")
assert_contains "AC10: verify-pipeline.sh is invoked against origin/main" \
    './scripts/verify-pipeline.sh origin/main' "$MEASURE_FN"
assert_not_contains "AC10: never against the worktree's local main" \
    'verify-pipeline.sh main' "$MEASURE_FN"
assert_contains "AC3: Path B emits RECOVERY_PENDING: true (Guard 1)" \
    'RECOVERY_PENDING: true' "$PATHB_BLOCK"
assert_contains "AC3: Path B tags the rescued PR with the wip-rescue label" \
    'add-label "wip-rescue"' "$PATHB_BLOCK"
assert_contains "AC3: Path B emits the canonical PR: line (mika#1352)" \
    'PR: ${PR_URL}' "$PATHB_BLOCK"

# AC6 (Edit 2): the wip(mika#1383) marker commit makes the head-commit headline
# match Guard 2's ^wip\( regex — and must be guarded to commit-pushed-no-pr ONLY
# (the dirty-worktree class is already wip()-prefixed; a second empty commit there
# would be wrong).
assert_contains "AC6: Path B adds an empty wip(mika#1383) marker commit for Guard 2" \
    'commit --allow-empty --no-verify -m "wip(mika#1383): auto-PR-create rescue' "$PATHB_BLOCK"

# Structural: the marker commit must sit INSIDE a commit-pushed-no-pr class guard.
# Assert the guard opens before the marker commit and that the dirty-worktree
# arm does not contain the marker.
MARKER_GUARD_BLOCK=$(printf '%s\n' "$PATHB_BLOCK" | sed -n '/RECOVERY_CLASS" = "commit-pushed-no-pr"/,/RESCUED_PR_URL=\$(gh pr create/p')
assert_contains "AC6: marker commit is scoped under the commit-pushed-no-pr guard" \
    'wip(mika#1383): auto-PR-create rescue' "$MARKER_GUARD_BLOCK"
assert_contains "AC6: the marker-commit guard tests RECOVERY_CLASS = commit-pushed-no-pr" \
    'RECOVERY_CLASS" = "commit-pushed-no-pr"' "$MARKER_GUARD_BLOCK"

# AC5 regression: the dirty-worktree class still flows through Path B unchanged
# (its title/fact branch and the shared draft body remain).
assert_contains "AC5: dirty-worktree class still handled by Path B" \
    'RECOVERY_CLASS="dirty-worktree"' "$PATHB_BLOCK"

# AC6 structural: the marker commit appears EXACTLY ONCE in Path B (only in the
# commit-pushed-no-pr arm; the dirty-worktree class is already wip()-prefixed and
# must not get a second empty commit). A count > 1 means it leaked into both arms.
MARKER_COUNT=$(printf '%s\n' "$PATHB_BLOCK" | grep -c 'commit --allow-empty --no-verify -m "wip(mika#1383): auto-PR-create rescue')
if [ "$MARKER_COUNT" -eq 1 ]; then
    PASS=$((PASS + 1))
    echo "  ✓ AC6: wip(mika#1383) marker commit appears exactly once (commit-pushed-no-pr arm only)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ AC6: expected exactly 1 wip(mika#1383) marker commit in Path B, found $MARKER_COUNT"
fi

# mika#1679 hardening (review follow-up): the marker commit is idempotent on
# re-dispatch (skip when HEAD is already a wip(mika#1383) marker) and the push
# failure is surfaced as an observable signal rather than silently swallowed
# (so an unpushed marker = unarmed Guard 2 is visible to operator/telemetry).
# Le needle porte le `--` et la lecture est en here-string depuis mika#3130 : la
# forme `printf … | grep -qF` d'avant pouvait lire FAUX sur un sujet PRÉSENT.
assert_contains "Hardening: marker commit is idempotent (skip when HEAD already a wip(mika#1383) marker)" \
    "grep -qF -- 'wip(mika#1383): auto-PR-create rescue' <<<" "$MARKER_GUARD_BLOCK"
assert_contains "Hardening: marker push failure is surfaced (rescue_marker_push.failed), not silenced" \
    'rescue_marker_push.failed' "$PATHB_BLOCK"
assert_not_contains "Hardening: marker push no longer uses a bare '|| true' silent swallow" \
    'push origin "$BRANCH" 2>&9 || true' "$MARKER_GUARD_BLOCK"

# --- Test: repo#number parse normalizes an optional owner/ prefix (mika#1593) ---
echo ""
echo "Test: _set_up_worktree prompt parse — owner-prefix normalization (mika#1593)"

# (a) Static: the live parser carries the broadened regex + owner-strip, so the
#     behavioral replica below cannot silently drift from the real code.
PARSE_REGION=$(sed -n '/--- Parse repo#number format ---/,/^    if \[ -n "\$REPO" \]/p' "$DISPATCH_LIB")
assert_contains "Parser regex accepts an optional owner/ segment" \
    '^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$' "$PARSE_REGION"
assert_contains "Parser strips the owner/ prefix to the bare basename" \
    "sed 's#.*/##'" "$PARSE_REGION"

# (b) Behavioral: replicate the exact two-line parse and assert normalization.
#     Mirrors the harness convention of testing extracted logic in isolation
#     (the full dispatch_claude_pilot needs git/gh/claude-pilot).
_parse_prompt() {
    local PROMPT="$1" REPO="" ISSUE_NUM=""
    if grep -qE -- '^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$' <<<"$PROMPT"; then
        REPO=$(printf '%s' "$PROMPT" | sed 's/#.*//' | sed 's#.*/##')
        ISSUE_NUM=$(printf '%s' "$PROMPT" | sed 's/.*#//')
    fi
    printf '%s|%s' "$REPO" "$ISSUE_NUM"
}
assert_eq "Bare repo#number parses unchanged" "mika|214" "$(_parse_prompt 'mika#214')"
assert_eq "Owner-qualified ref normalizes to bare basename" \
    "mika|1576" "$(_parse_prompt 'senara-solutions/mika#1576')"
assert_eq "Hyphenated repo basename is preserved" \
    "mika-cloud|50" "$(_parse_prompt 'senara-solutions/mika-cloud#50')"
assert_eq "Bare hyphenated repo parses unchanged" \
    "mika-skills|8" "$(_parse_prompt 'mika-skills#8')"
assert_eq "Free-text prompt with embedded # stays free-text (empty REPO)" \
    "|" "$(_parse_prompt 'fix the foo#bar thing and more')"

# --- Test 16: _find_issue_plan header-shape discovery (mika#1602, n=3; tier 3 added in mika#1617) ---
#
# Behavioral test: source dispatch-lib.sh (verified side-effect-free — function
# definitions only, no top-level execution) and call the real _find_issue_plan
# against temp `docs/plans/` fixtures. Proves AC1–AC4 for tier-2 shapes:
#   AC1 — `**Issue:** mika#N` (and `issue: mika#N`) headers are discoverable.
#   AC2 — the legacy `**Ticket:** mika#N` and `ticket: mika#N` shapes still match.
#   AC3 — the primary filename pass (`*-N-*-plan.md`) still matches.
#   AC4 — the `**Issue:**` case FAILS on the pre-fix regex and PASSES after.
# Tier-3 broad content scan tests live in tests/test_find_issue_plan.sh (mika#1617).
#
# Each fixture is padded > 500 bytes to satisfy the mika#1033 size filter.

echo ""
echo "Test 16: _find_issue_plan header-shape discovery (mika#1602)"
echo "------------------------------------------------------------"

# _fip_probe <issue_num> <header_line> <filename_slug> [header_offset_lines]
# Builds a one-off plan fixture and returns _find_issue_plan's verdict as
# "FOUND <basename>" (exit 0) or "NOTFOUND" (exit non-zero). Sourcing happens
# in this same subshell so $WORKTREE_DIR / $ISSUE_NUM scope to the call only.
_fip_probe() {
    local issue_num="$1" header_line="$2" slug="$3" offset="${4:-2}"
    local tmp result rc i
    tmp=$(mktemp -d)
    mkdir -p "$tmp/docs/plans"
    local plan="$tmp/docs/plans/$slug"
    {
        echo "# Plan: synthetic fixture"
        # Pad with $offset blank/filler lines BEFORE the header so callers can
        # push the header above or below the 20-line header-zone boundary.
        for ((i = 1; i < offset; i++)); do echo ""; done
        echo "$header_line"
        echo ""
        # >500 bytes of body so the size filter does not reject the fixture.
        for i in $(seq 1 12); do
            echo "Body line $i — padding padding padding padding padding padding."
        done
    } > "$plan"
    result=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        WORKTREE_DIR="$tmp" ISSUE_NUM="$issue_num" _find_issue_plan
    ) && rc=0 || rc=$?
    rm -rf "$tmp"
    if [ "$rc" -eq 0 ] && [ -n "$result" ]; then
        printf 'FOUND %s' "$(basename "$result")"
    else
        printf 'NOTFOUND'
    fi
}

# AC1 — the n=3 case: **Issue:** header, filename has NO -1602- token.
assert_eq "AC1: **Issue:** mika#1602 header found despite unrelated filename" \
    "FOUND 2026-06-27-006-fix-unrelated-slug-plan.md" \
    "$(_fip_probe 1602 '**Issue:** mika#1602' '2026-06-27-006-fix-unrelated-slug-plan.md')"

# AC1 variant — `issue:` YAML frontmatter shape.
assert_eq "AC1: issue: mika#1602 YAML header found" \
    "FOUND 2026-06-27-007-yaml-issue-shape-plan.md" \
    "$(_fip_probe 1602 'issue: mika#1602' '2026-06-27-007-yaml-issue-shape-plan.md')"

# AC2 regression — legacy **Ticket:** shape still matches.
assert_eq "AC2: **Ticket:** mika#771 header still found (no regression)" \
    "FOUND 2026-06-06-003-feat-some-other-slug-plan.md" \
    "$(_fip_probe 771 '**Ticket:** mika#771' '2026-06-06-003-feat-some-other-slug-plan.md')"

# AC2 regression — legacy ticket: YAML shape still matches.
assert_eq "AC2: ticket: mika#771 YAML header still found (no regression)" \
    "FOUND 2026-06-06-004-feat-yaml-ticket-plan.md" \
    "$(_fip_probe 771 'ticket: mika#771' '2026-06-06-004-feat-yaml-ticket-plan.md')"

# AC3 regression — primary filename pass (issue number in filename, no content header).
assert_eq "AC3: filename-embedded issue number found via primary pass" \
    "FOUND 2026-06-06-003-fix-1407-pilot-push-plan.md" \
    "$(_fip_probe 1407 '## No matching header here' '2026-06-06-003-fix-1407-pilot-push-plan.md')"

# Negative — wrong issue number must NOT match (guards against over-broad union).
assert_eq "Negative: **Issue:** mika#9999 not matched for ISSUE_NUM=1602" \
    "NOTFOUND" \
    "$(_fip_probe 1602 '**Issue:** mika#9999' '2026-06-27-008-wrong-number-plan.md')"

# mika#2038 — tier 1 refutes a candidate whose header names another issue.
# The founding incident, verbatim: the glob `*-2026-*-plan.md` matches the
# RustSec advisory id in this filename, and the old tier 1 returned it first,
# so a pilot dispatched for mika#2026 ran /ce-work on an April `rand` bump.
# The header says #539, so the candidate is refuted and no tier picks it up.
assert_eq "mika#2038: rustsec-2026-0097 filename not returned for ISSUE_NUM=2026" \
    "NOTFOUND" \
    "$(_fip_probe 2026 '**Issue:** #539' '2026-04-11-003-chore-deps-bump-rand-clear-rustsec-2026-0097-plan.md')"

# mika#2038 — a header-less, off-slot filename must STILL resolve at tier 1.
# 95 of 745 real plans carry no issue marker and 490 do not honour the
# `<date>-<NNN>-<type>-<issue>-` slot; refutation must not turn one false
# positive into that many false negatives (mika#1421 / #1602 / #1617 lineage).
assert_eq "mika#2038: header-less 'mika-' prefixed filename still found" \
    "FOUND 2026-06-10-001-fix-mika-1475-deploy-info-off-main-abort-plan.md" \
    "$(_fip_probe 1475 '## No matching header here' '2026-06-10-001-fix-mika-1475-deploy-info-off-main-abort-plan.md')"

# mika#2038 — a plan whose slug cites another ticket is refuted by its header.
assert_eq "mika#2038: plan for #1679 not returned for the #1383 it merely cites" \
    "NOTFOUND" \
    "$(_fip_probe 1383 'issue: 1679' '2026-06-30-011-fix-1679-dispatch-lib-mika-1383-recovery-guards-plan.md')"

# Negative — header below the 50-line zone must NOT match (tier-3 zone boundary).
# Pre-mika#1617 this was offset 30 (tier-2's 20-line zone); now tier 3 extends
# the scan to 50 lines, so the offset must be past 50 to remain a negative case.
assert_eq "Negative: **Issue:** mika#1602 on line 55 not matched (past tier-3 zone)" \
    "NOTFOUND" \
    "$(_fip_probe 1602 '**Issue:** mika#1602' '2026-06-27-009-deep-header-plan.md' 55)"

# --- Test 17: Post-flight recovery fires on all exit paths (mika#1615) ---

echo ""
echo "Test 17: Post-flight recovery fires on all exit paths (mika#1615)"
echo "-----------------------------------------------------------------"

# Structural: POST_RUN_HEAD computed unconditionally (before the three-branch if/elif/else)
# The computation must appear BEFORE the `if [ -n "$STATUS" ]` line that opens Branch A.
POST_RUN_HEAD_LINE=$(grep -n 'POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD' "$DISPATCH_LIB" \
    | grep -v '#' | grep -v 'rescue' | grep -v 'trailing' | head -1 | cut -d: -f1)
BRANCH_A_LINE=$(grep -n 'if \[ -n "$STATUS" \]; then' "$DISPATCH_LIB" | head -1 | cut -d: -f1)

if [ -n "$POST_RUN_HEAD_LINE" ] && [ -n "$BRANCH_A_LINE" ] && [ "$POST_RUN_HEAD_LINE" -lt "$BRANCH_A_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ POST_RUN_HEAD computed before Branch A (line $POST_RUN_HEAD_LINE < $BRANCH_A_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ POST_RUN_HEAD must be computed before Branch A (POST_RUN_HEAD=$POST_RUN_HEAD_LINE, Branch A=$BRANCH_A_LINE)"
fi

# Structural: _post_flight_recovery function exists
assert_contains "_post_flight_recovery function defined" \
    "_post_flight_recovery()" \
    "$(grep '_post_flight_recovery()' "$DISPATCH_LIB")"

# Structural: _post_flight_recovery called from _run_claude_pilot after three-branch fi
RUN_CLAUDE_PILOT_BODY=$(sed -n '/_run_claude_pilot()/,/^}/p' "$DISPATCH_LIB")
assert_contains "_post_flight_recovery called in _run_claude_pilot" \
    "_post_flight_recovery" \
    "$RUN_CLAUDE_PILOT_BODY"

# Structural: dirty-worktree rescue is inside _post_flight_recovery, NOT inside Branch A
BRANCH_A_BLOCK=$(sed -n '/if \[ -n "\$STATUS" \]; then/,/elif \[ "\$PILOT_EXIT" -eq 0 \]/p' "$DISPATCH_LIB")
assert_not_contains "Dirty-worktree rescue NOT inside Branch A" \
    "Unit 1 (mika#1282): detect dirty worktree" \
    "$BRANCH_A_BLOCK"

RECOVERY_FUNC=$(sed -n '/_post_flight_recovery()/,/^}/p' "$DISPATCH_LIB")
assert_contains "Dirty-worktree rescue inside _post_flight_recovery" \
    "Unit 1 (mika#1282): detect dirty worktree" \
    "$RECOVERY_FUNC"

# Structural: dev-groom plan validation is inside _post_flight_recovery
assert_contains "Dev-groom plan validation inside _post_flight_recovery" \
    "Post-flight plan validation" \
    "$RECOVERY_FUNC"
assert_not_contains "Dev-groom plan validation NOT inside Branch A" \
    "Post-flight plan validation" \
    "$BRANCH_A_BLOCK"

# Structural: PR-existence check is inside _post_flight_recovery
assert_contains "PR-existence check inside _post_flight_recovery" \
    "Discover actual PR URL" \
    "$RECOVERY_FUNC"

# Structural: outcome classification is inside _post_flight_recovery
assert_contains "Outcome classification inside _post_flight_recovery" \
    "outcome classification line" \
    "$RECOVERY_FUNC"

# Structural: POST_RUN_HEAD initialization (empty string) alongside RESCUED_DIRTY_WORKTREE
assert_contains "POST_RUN_HEAD initialized to empty" \
    'POST_RUN_HEAD=""' \
    "$RUN_CLAUDE_PILOT_BODY"

# Structural: the unconditional POST_RUN_HEAD computation is guarded by PRE_RUN_HEAD and WORKTREE_DIR
UNCONDITIONAL_HEAD_BLOCK=$(sed -n '/Compute POST_RUN_HEAD unconditionally/,/fi/p' "$DISPATCH_LIB" | head -10)
assert_contains "POST_RUN_HEAD guard checks PRE_RUN_HEAD" \
    'PRE_RUN_HEAD' \
    "$UNCONDITIONAL_HEAD_BLOCK"
assert_contains "POST_RUN_HEAD guard checks WORKTREE_DIR" \
    'WORKTREE_DIR' \
    "$UNCONDITIONAL_HEAD_BLOCK"

# Behavioral: verify recovery fires when STATUS is empty (Branch B scenario)
# Simulates a dirty-worktree rescue path with no structured JSON output.
test_recovery_fires_on_branch_b() {
    local base_dir wt_dir pre_head
    base_dir=$(mktemp -d)
    wt_dir="$base_dir/worktree"

    # Set up a git repo to simulate dirty worktree
    git init -q "$wt_dir" 2>/dev/null
    git -C "$wt_dir" config user.email "test@test.com"
    git -C "$wt_dir" config user.name "Test"
    echo "initial" > "$wt_dir/file.txt"
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "initial" 2>/dev/null
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)

    # Create dirty file (pilot-authored content)
    echo "pilot work" > "$wt_dir/new_feature.rs"

    # Source dispatch-lib and set up variables as _run_claude_pilot would
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB"

        # Simulate Branch B state: STATUS is empty, exit 0, dirty worktree
        PRE_RUN_HEAD="$pre_head"
        POST_RUN_HEAD="$pre_head"  # Same as PRE — no commits
        WORKTREE_DIR="$wt_dir"
        SKILL="dev-pilot"
        REPO="mika"
        BRANCH="test-branch"
        ISSUE_NUM="9999"
        SESSION_ID="test-session"
        LOG_ID="test-log"
        STATUS=""  # Branch B — no structured JSON output
        RESULT="claude-pilot completed (exit 0) but output was not structured JSON."
        RESCUED_DIRTY_WORKTREE=0

        # Redirect fd 9 to stderr for the rescue block
        exec 9>&2

        # Run the recovery function
        _post_flight_recovery 2>/dev/null

        # Check results
        post_head=$(git -C "$wt_dir" rev-parse HEAD)
        if [ "$pre_head" != "$post_head" ] && [ "$RESCUED_DIRTY_WORKTREE" -eq 1 ]; then
            echo "OK"
        else
            echo "FAIL: pre=$pre_head post=$post_head rescued=$RESCUED_DIRTY_WORKTREE"
        fi
    )

    rm -rf "$base_dir"
}

RESULT_17A=$(test_recovery_fires_on_branch_b 2>/dev/null)
assert_eq "Branch B (non-JSON exit 0): dirty-worktree rescue fires and commits" "OK" "$RESULT_17A"

# Behavioral: verify recovery fires when exit code is non-zero (Branch C scenario)
test_recovery_fires_on_branch_c() {
    local base_dir wt_dir pre_head
    base_dir=$(mktemp -d)
    wt_dir="$base_dir/worktree"

    git init -q "$wt_dir" 2>/dev/null
    git -C "$wt_dir" config user.email "test@test.com"
    git -C "$wt_dir" config user.name "Test"
    echo "initial" > "$wt_dir/file.txt"
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "initial" 2>/dev/null
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)

    # Create dirty file
    echo "pilot crash content" > "$wt_dir/partial_impl.rs"

    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB"

        PRE_RUN_HEAD="$pre_head"
        POST_RUN_HEAD="$pre_head"
        WORKTREE_DIR="$wt_dir"
        SKILL="dev-pilot"
        REPO="mika"
        BRANCH="test-branch"
        ISSUE_NUM="9999"
        SESSION_ID="test-session"
        LOG_ID="test-log"
        STATUS=""  # Branch C — non-zero exit, no structured output
        RESULT="claude-pilot FAILED (exit code 1)."
        RESCUED_DIRTY_WORKTREE=0

        exec 9>&2

        _post_flight_recovery 2>/dev/null

        post_head=$(git -C "$wt_dir" rev-parse HEAD)
        if [ "$pre_head" != "$post_head" ] && [ "$RESCUED_DIRTY_WORKTREE" -eq 1 ]; then
            echo "OK"
        else
            echo "FAIL: pre=$pre_head post=$post_head rescued=$RESCUED_DIRTY_WORKTREE"
        fi
    )

    rm -rf "$base_dir"
}

RESULT_17B=$(test_recovery_fires_on_branch_c 2>/dev/null)
assert_eq "Branch C (non-zero exit): dirty-worktree rescue fires and commits" "OK" "$RESULT_17B"

# Behavioral: verify POST_RUN_HEAD is available for outcome classification
# when STATUS is empty (previously Outcome was never emitted for Branch B/C)
test_outcome_emitted_on_branch_b() {
    local base_dir wt_dir pre_head
    base_dir=$(mktemp -d)
    wt_dir="$base_dir/worktree"

    git init -q "$wt_dir" 2>/dev/null
    git -C "$wt_dir" config user.email "test@test.com"
    git -C "$wt_dir" config user.name "Test"
    echo "initial" > "$wt_dir/file.txt"
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "initial" 2>/dev/null
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)

    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB"

        PRE_RUN_HEAD="$pre_head"
        POST_RUN_HEAD="$pre_head"
        WORKTREE_DIR="$wt_dir"
        SKILL="dev-pilot"
        REPO="mika"
        BRANCH="test-branch"
        ISSUE_NUM="9999"
        SESSION_ID="test-session"
        LOG_ID="test-log"
        STATUS=""
        RESULT="claude-pilot completed (exit 0) but output was not structured JSON."
        RESCUED_DIRTY_WORKTREE=0

        exec 9>&2

        _post_flight_recovery 2>/dev/null

        # Outcome classification should have run (PIPELINE_INCOMPLETE for zero-commit)
        if grep -qF -- "Outcome:" <<<"$RESULT"; then
            echo "OK"
        else
            echo "FAIL: no Outcome line in RESULT"
        fi
    )

    rm -rf "$base_dir"
}

RESULT_17C=$(test_outcome_emitted_on_branch_b 2>/dev/null)
assert_eq "Branch B: outcome classification fires (Outcome: line present)" "OK" "$RESULT_17C"

# --- Test: wip-rescue label application (mika#1631) ---

echo ""
echo "Test: wip-rescue label applied to rescued PRs (mika#1631)"
echo "----------------------------------------------------------"

# Verify the rescue flow applies the wip-rescue label after PR creation.
# The `gh pr edit ... --add-label "wip-rescue"` call is inside the deeply-nested
# `if [ -n "$RESCUED_PR_URL" ]` block of _post_flight_recovery. We grep the
# file directly rather than extracting by sed range (nested braces defeat the
# simple /pattern/,/^}/p extraction; full-file cat hits shell string limits).

if grep -qF -- '--add-label "wip-rescue"' "$DISPATCH_LIB"; then
    PASS=$((PASS + 1)); echo "  ✓ Rescue flow applies wip-rescue label"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Rescue flow applies wip-rescue label"
fi

# Verify the label application targets RESCUED_PR_URL
if grep -qF 'gh pr edit "$RESCUED_PR_URL" --add-label "wip-rescue"' "$DISPATCH_LIB"; then
    PASS=$((PASS + 1)); echo "  ✓ Label applied to RESCUED_PR_URL"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Label applied to RESCUED_PR_URL"
fi

# Verify it's fault-tolerant (|| true) — label failure must not break the rescue
if grep -qF 'wip-rescue" 2>&9 || true' "$DISPATCH_LIB"; then
    PASS=$((PASS + 1)); echo "  ✓ Label application is fault-tolerant (|| true)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Label application is fault-tolerant (|| true)"
fi

# --- Regression: no bare $REPO in gh --repo arguments (mika#1643) ---

echo ""
echo "=== Regression: no bare \$REPO in gh --repo (mika#1643) ==="

# All gh --repo call sites must use senara-solutions/$REPO, never bare $REPO.
BARE_REPO_HITS=$(grep -n 'gh.*--repo[[:space:]]*"\$REPO"' "$DISPATCH_LIB" | grep -v 'senara-solutions/' || true)
BARE_REPO_HITS_UNQUOTED=$(grep -n 'gh.*--repo[[:space:]]*\${REPO}' "$DISPATCH_LIB" | grep -v 'senara-solutions/' || true)

if [ -z "$BARE_REPO_HITS" ] && [ -z "$BARE_REPO_HITS_UNQUOTED" ]; then
    PASS=$((PASS + 1)); echo "  ✓ No bare \$REPO in gh --repo arguments"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Found bare \$REPO in gh --repo arguments:"
    [ -n "$BARE_REPO_HITS" ] && echo "$BARE_REPO_HITS"
    [ -n "$BARE_REPO_HITS_UNQUOTED" ] && echo "$BARE_REPO_HITS_UNQUOTED"
fi

# --- Redundant-groom refusal gate: functional cases (mika#2012) ---
# Defined above next to the gate's code-shape assertions; invoked here because
# they depend on _fixture_setup.

echo ""
echo "Test: redundant-groom refusal gate (mika#2012)"
echo "-----------------------------------------------"

# mika#2608 : ces douze cas appellent `_committed_plan_on_branch`, qui délègue
# désormais à `mika plan-callout`. Sans le binaire ils ne mesurent rien, et le
# dire est le contrat de la troisième colonne (mika#2149) — un cas de porte qui
# n'a pas pu s'armer ne doit pas se lire comme un vert.
if ! mika_bin_or_skip "porte de grooming mika#2012 (12 cas)"; then
    : # le SKIP est déjà compté et dit par le helper
else
for gate_case in \
    "test_groom_gate_callout_present_file_absent|callout present but plan file ABSENT → gate does NOT fire" \
    "test_groom_gate_plan_committed|plan committed on branch → gate fires with the path" \
    "test_groom_gate_repo_prefixed_path|repo-prefixed callout resolves to relative path" \
    "test_groom_gate_no_callout|no Plan callout → gate does NOT fire" \
    "test_groom_gate_branch_absent|branch absent from remote → gate does NOT fire" \
    "test_groom_gate_foreign_plan_inherited_from_main|mika#2034: plan claims another issue, inherited from main → gate does NOT fire" \
    "test_groom_gate_plan_header_claims_target_issue|mika#2034: plan header claims the target issue → gate fires" \
    "test_groom_gate_plan_without_issue_marker_still_fires|mika#2034: plan with no issue marker → gate still fires" \
    "test_groom_gate_issue_arg_is_optional|mika#2034: the 5th argument is optional" \
    "test_plan_provenance_distinguishes_inherited_from_committed|mika#2034: provenance separates inherited from committed" \
    "test_groom_gate_declines_on_directory_candidate|mika#2034: callout naming a directory → gate does NOT fire" \
    "test_groom_gate_refusal_implies_rust_says_groomed|mika#2158 AC7: la garde refuse ⇒ le Rust dit groomé (six corps figés)"
do
    gate_fn="${gate_case%%|*}"
    gate_label="${gate_case#*|}"
    gate_result=$("$gate_fn" 2>/dev/null) || gate_result="ABORTED (rc=$?)"
    if [ "$gate_result" = "PASS" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $gate_label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $gate_label: $gate_result"
    fi
done
fi

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
