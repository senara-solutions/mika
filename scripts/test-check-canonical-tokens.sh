#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-canonical-tokens.sh and
# scripts/canonical-tokens-survey.sh (mika#2201).
#
# House constraint, written into ci.yml since mika#2103: *a guard nobody has
# watched go red is a decoration*. The founding incident there is exactly a lint
# that could not fail on the defect it existed to catch, and stayed green
# through 26 production panics.
#
# This suite proves the lint bites on each of its five rules, that the survey
# compares its list in both directions, that an exception suppresses and that a
# STALE one fails — and, just as load-bearing, that NONE of it bites on the
# shapes the readers deliberately accept. That second half is not symmetry for
# its own sake: mika#2201 § R1 is the finding that the naive rule accuses
# `seconde passe`, which `grooming_marker.rs` reads by written decision. A lint
# red at birth gets disarmed, and then it protects nothing.
#
# "Delete the thing the test protects; confirm the test goes red."

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LINT="$REPO_ROOT/scripts/check-canonical-tokens.sh"
SURVEY="$REPO_ROOT/scripts/canonical-tokens-survey.sh"
FIXTURES="$REPO_ROOT/scripts/fixtures/canonical-tokens"

PASS=0
FAIL=0

# Run the lint on a body file; assert its exit equals $2. $3 = case name.
assert_body_exit() {
    local file="$1" want="$2" name="$3"
    local got=0
    bash "$LINT" --body "$file" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

# Assert the lint's output on body $1 mentions $2. $3 = case name.
#
# Output is captured and matched with `case`, not piped into `grep -q`: under
# `pipefail` a failing producer poisons the pipeline's status even when grep
# matches, and `echo | grep -q` is itself refused by the sigpipe lint
# (mika#2055).
assert_body_says() {
    local file="$1" want="$2" name="$3"
    local out
    out="$(bash "$LINT" --body "$file" 2>&1 || true)"
    if [[ "$out" == *"$want"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not mention '$want')"
        FAIL=$((FAIL + 1))
    fi
}

# A throwaway body file whose content is $1.
make_body() {
    local f
    f="$(mktemp --suffix=.md)"
    printf '%s\n' "$1" > "$f"
    echo "$f"
}

echo "═══ The shipped fixtures (V1–V4) ═══"

# ── V1. The mika#2188 residue: the positional doctrine covers a resolved
#    escalation only when a LATER pass follows. With nothing after, the prose
#    says resolved and the machine says escalated.
assert_body_exit "$FIXTURES/escalate-divergence.md" 1 "V1: ESCALATE-divergence in a callout is refused (AC3)"
assert_body_says "$FIXTURES/escalate-divergence.md" "L1:" "V1: the refusal names rule L1"

# ── V2. THE SIGN OF THIS ONE IS THE FINDING (§ R1). mika#2201's AC3 asked for
#    `seconde passe` as a red fixture; `grooming_marker.rs` reads that form by
#    written decision since mika#2158, so a red here would be a REGRESSION, not
#    a guard. It stays as a NON-REGRESSION fixture and must remain green.
assert_body_exit "$FIXTURES/seconde-passe.md" 0 "V2: 'seconde passe' + 'première passe' PASS (R1 non-regression)"

# ── V3. The French typographic space — the most probable variant in a
#    French-writing repository, and the one neither cited bite has produced yet.
assert_body_exit "$FIXTURES/plan-espace-fr.md" 1 "V3: '> - **Plan :**' is refused"
assert_body_says "$FIXTURES/plan-espace-fr.md" "L3:" "V3: the refusal names rule L3"

# ── V4. Case. And on a line whose `second-pass` is ALSO lower-case and must NOT
#    be accused — two tokens, two tolerances, one line.
assert_body_exit "$FIXTURES/verdict-minuscule.md" 1 "V4: lower-case 'groomed' in a callout is refused"
assert_body_says "$FIXTURES/verdict-minuscule.md" "L2:" "V4: the refusal names rule L2"

echo ""
echo "═══ The compliant shapes must NOT fire (the R4 half) ═══"

# The canonical callout, in all three shapes `_write_canonical_callout` emits.
d="$(make_body '> - **Branch:** `feat/1/x`
> - **Plan:** `docs/plans/2026-09-20-001-feat-1-x-plan.md` (committed on branch @ `abc1234`)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: aaaa')"
assert_body_exit "$d" 0 "the canonical callout passes"
rm -f "$d"

d="$(make_body '> - **Grooming history:** first-pass (ITERATE) → revised → second-pass (GROOMED) — session-id: b')"
assert_body_exit "$d" 0 "the ITERATE→GROOMED shape passes"
rm -f "$d"

d="$(make_body '> - **Grooming history:** first-pass (READY, single-pass GROOMED) — no second pass required — session-id: c')"
assert_body_exit "$d" 0 "the single-pass shape passes"
rm -f "$d"

# `ITERATE` carries no class B row — `VERDICT_TOKEN_RE` does not read it and
# `grooming_marker.rs` says so — so L1 must not accuse a compound built on it.
d="$(make_body '> - **Grooming history:** first-pass (ITERATE-after-review) → second-pass (GROOMED)')"
assert_body_exit "$d" 0 "L1 does not accuse a compound on a token no strict reader reads"
rm -f "$d"

# The milestone flow writes callout keys of its own. A rule reading "any key
# outside the canonical three" would be red at birth on all three of these.
d="$(make_body '> - **Sub-issues:** #1, #2
> - **Sequencing record:** `docs/plans/seq.md`
> - **Coordination branch:** `coord/14`')"
assert_body_exit "$d" 0 "L3 admits the milestone callout keys (AC: no false positive)"
rm -f "$d"

# THE CENTRAL NEGATIVE CONTROL. The same compound, in PROSE rather than in a
# callout line, is out of scope exactly as `CALLOUT_LINE_RE` is out of scope —
# and mika#2201's own ticket body carries this sentence.
d="$(make_body 'Deux morsures cette semaine : `ESCALATE` matché en sous-chaîne de
`ESCALATE-divergence` (#2188). Le ticket a été GROOMED hier, et groomed est un
mot de prose.')"
assert_body_exit "$d" 0 "L1/L2 are bounded to the callout line, never to prose"
rm -f "$d"

# A tolerance carrying `ci` disarms L2 for that token. `DEPTH_RE` is `(?mi)`, so
# a lower-case `depth:` IS read and must not be accused.
d="$(make_body '> - **Grooming history:** first-pass (READY) → second-pass (GROOMED)
depth: shallow')"
assert_body_exit "$d" 0 "a ci-tolerant token is not accused of case"
rm -f "$d"

echo ""
echo "═══ Rule L4 — the French space on a protocol label ═══"

d="$(make_body 'Disposition : READY')"
assert_body_exit "$d" 1 "L4: 'Disposition :' is refused"
rm -f "$d"

d="$(make_body 'Verdict : GROOMED')"
assert_body_exit "$d" 1 "L4: 'Verdict :' is refused"
rm -f "$d"

d="$(make_body 'Disposition: READY
Verdict: GROOMED
VERDICT: pass')"
assert_body_exit "$d" 0 "L4 admits the canonical labels"
rm -f "$d"

echo ""
echo "═══ A token QUOTED as code is a mention, never an instruction ═══"

# MEASURED on this ticket's own plan, whose § M4 describes rule L4 by citing the
# faulty forms. The lint accused both — and so would the PR body shipping it,
# and every future ticket discussing the rule. That is R4 at its purest: a
# document explaining the lint makes the lint red, and the lint gets disarmed.
# Exact precedent one file away: mika#2050's Signal S false positive.
d="$(make_body '- **L4 — a translated class B token.** `Verdict :`, `Disposition :`,
  `Résultat:` instead of `Outcome:` — same reason as L3.')"
assert_body_exit "$d" 0 "L4 does not accuse a form quoted in an inline code span"
rm -f "$d"

d="$(make_body 'Example of what NOT to write:

```
> - **Plan :** `docs/plans/x.md`
> - **Grooming history:** second-pass (groomed)
```

The forms above are refused.')"
assert_body_exit "$d" 0 "L1–L4 do not accuse a fenced example block"
rm -f "$d"

# ...and the stripping must not blind the rules: the SAME forms outside a code
# span are still refused. Without this pair, "it does not fire" would be
# indistinguishable from "it fires on nothing".
d="$(make_body '> - **Plan :** `docs/plans/x.md`')"
assert_body_exit "$d" 1 "the same form OUTSIDE a code span is still refused"
rm -f "$d"

# The canonical callout survives the stripping: its KEY is outside the
# backticks, only the path is inside. A stripper that ate the key would make L3
# structurally unable to fire on the callout it exists to protect.
d="$(make_body '> - **Plan :** `docs/plans/x.md` (committed on branch @ `abc`)')"
assert_body_exit "$d" 1 "a callout with a backticked path still exposes its key to L3"
rm -f "$d"

echo ""
echo "═══ Rule L5 — an undeclared label written as an instruction ═══"

# A fixture tree carrying one prescriber and a two-label vocabulary.
make_tree() {
    local dir
    dir="$(mktemp -d)"
    mkdir -p "$dir/scripts" "$dir/.github" "$dir/skills/bundled/_shared"
    cp "$REPO_ROOT/scripts/canonical-tokens.tsv" "$dir/scripts/"
    printf '%s\n' '- name: ready' '  color: "0e8a16"' '- name: blocked' '  color: "b60205"' \
        > "$dir/.github/labels.yml"
    printf '%s\n' "$1" > "$dir/skills/bundled/_shared/dispatch-lib.sh"
    echo "$dir"
}

assert_tree_exit() {
    local root="$1" want="$2" name="$3"
    local got=0
    bash "$LINT" "$root" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

t="$(make_tree 'gh issue edit "$n" --add-label zorglub-undeclared || true')"
assert_tree_exit "$t" 1 "L5: an undeclared --add-label is refused"
rm -rf "$t"

t="$(make_tree 'gh issue edit "$n" --add-label ready
gh issue edit "$n" --remove-label blocked')"
assert_tree_exit "$t" 0 "L5: declared labels pass"
rm -rf "$t"

# The three placeholder shapes measured in the tree. Accusing any of them would
# make the rule red at birth on `.claude/commands/`.
t="$(make_tree 'gh issue edit "$n" --add-label "$label"
gh issue edit "$n" --add-label "phase:$phase"
gh issue create --label "type,priority"')"
assert_tree_exit "$t" 0 "L5: shell variables and template placeholders are not labels"
rm -rf "$t"

# `--label` on an issue CREATION is deliberately out of scope: an unknown label
# fails that call loudly, where every `--add-label` in dispatch-lib is followed
# by `|| true` and fails in silence.
t="$(make_tree 'gh issue create --repo x/y --label zorglub-undeclared')"
assert_tree_exit "$t" 0 "L5: --label on issue creation is out of scope (fails loudly)"
rm -rf "$t"

echo ""
echo "═══ Exceptions: suppression, and the self-cleaning assertion (V7b) ═══"

t="$(make_tree 'gh issue edit "$n" --add-label zorglub-undeclared || true')"
printf '%s\n' 'skills/bundled/_shared/dispatch-lib.sh	zorglub-undeclared	mika#9999	2026-09-20' \
    > "$t/scripts/canonical-tokens-exceptions.tsv"
assert_tree_exit "$t" 0 "an exception with its four fields suppresses the accusation"
rm -rf "$t"

t="$(make_tree 'gh issue edit "$n" --add-label ready')"
printf '%s\n' 'skills/bundled/_shared/dispatch-lib.sh	zorglub-undeclared	mika#9999	2026-09-20' \
    > "$t/scripts/canonical-tokens-exceptions.tsv"
assert_tree_exit "$t" 1 "V7b: an exception whose file no longer carries the token FAILS"
rm -rf "$t"

t="$(make_tree 'gh issue edit "$n" --add-label zorglub-undeclared || true')"
printf '%s\n' 'skills/bundled/_shared/dispatch-lib.sh	zorglub-undeclared' \
    > "$t/scripts/canonical-tokens-exceptions.tsv"
assert_tree_exit "$t" 1 "an exception missing its ticket and date is refused"
rm -rf "$t"

echo ""
echo "═══ A scan with nothing to scan is a vacuous pass (mika#2103) ═══"

d="$(mktemp -d)"
mkdir -p "$d/scripts"
cp "$REPO_ROOT/scripts/canonical-tokens.tsv" "$d/scripts/"
assert_tree_exit "$d" 1 "an empty prescriber perimeter is refused, not silently passed"
rm -rf "$d"

echo ""
echo "═══ The survey, and what it does and does not compare (V6b, D5/D6) ═══"

assert_survey_exit() {
    local root="$1" want="$2" name="$3"
    local got=0
    bash "$SURVEY" --check "$root" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

# The real tree agrees with the shipped list.
assert_survey_exit "$REPO_ROOT" 0 "V6b: the survey agrees with canonical-tokens.tsv on the real tree"

# V7 — a NEW class B site that nobody declared fails the build. This is AC4's
# whole contract, and the shell half of it.
d="$(mktemp -d)"
mkdir -p "$d/scripts" "$d/crates/probe/src"
cp "$REPO_ROOT/scripts/canonical-tokens.tsv" "$d/scripts/"
printf '%s\n' 'static PROBE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bGROOMED\b").unwrap());' \
    > "$d/crates/probe/src/probe.rs"
assert_survey_exit "$d" 1 "V7: an undeclared class B match site fails the build (AC4)"
rm -rf "$d"

# The survey does NOT check the stale direction, and that is a decision rather
# than an omission — see the comment above its comparison. The list legitimately
# carries rows it structurally cannot produce: a `(?i)` alternation, a fuzzy
# tier, a `contains` reader. Forcing a two-way comparison was measured at
# fourteen false rows against two real ones. The stale direction is held, and
# held better, by `mika2201_every_declared_symbol_still_exists`, which asks
# "does this symbol still exist?" directly.
d="$(mktemp -d)"
mkdir -p "$d/scripts" "$d/crates/probe/src"
printf '%s\n' '# jeton	classe	site de match	tolérance' \
    'second-pass	A	crates/probe/src/invisible.rs::LATER_PASS_RE	ci+fr:seconde passe' \
    'GROOMED	B	crates/probe/src/loose.rs::reads_by_contains	exact:literal' \
    > "$d/scripts/canonical-tokens.tsv"
printf '%s\n' 'fn noop() {}' > "$d/crates/probe/src/probe.rs"
assert_survey_exit "$d" 0 "rows the survey cannot see are admitted, in either class"
rm -rf "$d"

# A survey whose perimeter does not exist is a vacuous pass.
d="$(mktemp -d)"
assert_survey_exit "$d" 1 "an empty survey perimeter is refused"
rm -rf "$d"

echo ""
echo "check-canonical-tokens anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
