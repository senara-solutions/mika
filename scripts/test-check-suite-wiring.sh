#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-suite-wiring.sh (mika#2761).
#
# "Delete the thing the test protects; confirm the test goes red."
#
# A guard nobody has watched go red is a decoration — mika#2103 is the incident
# where a lint stayed green through 26 production panics because it knew one
# spelling of the defect. This harness exists because the guard under test is,
# by construction, the one whose silence is hardest to notice: a lint about
# unrun suites that itself went vacuous would report "0 violations" on a tree
# where nothing is wired at all, and that output is indistinguishable from a
# clean repository. So every direction is seen RED on a fixture tree and GREEN
# once the fixture is repaired, and the two ways a set comparison passes for the
# wrong reason — an empty population, an empty notion of "reached" — exit 3.
#
# The guard takes one positional argument: the root to read. Every case below
# builds a miniature repository under a throwaway root (scripts/,
# skills/bundled/_shared/, .github/workflows/, Makefile, a registry) and runs
# the real guard against it. Nothing is stubbed and no function is sourced.
#
# Two properties are pinned here that are easy to mistake for bugs:
#
#   - Reachability through another SUITE's shell body is deliberately NOT
#     proven (§5). Interpreting shell errs in both directions, and the
#     dangerous direction is the false green. Under-approximating "reached"
#     yields only false reds — noisy, never silent.
#   - A mention inside a COMMENT reaches nothing (§4, §6). ci.yml carries the
#     prose "make the lint above a measurement"; a guard that counted it would
#     declare `make the` a wired target.
#
# The battery carries accented fixtures on purpose (§12): this repository writes
# its plans, tickets and reasons in French, so an ASCII-only fixture set tests a
# population that does not exist here. Both the accented path and the accented
# reason must survive intact, and the failure message is asserted to reproduce
# the path verbatim — an error that garbles the name is an error nobody can act
# on.
#
# Run: bash scripts/test-check-suite-wiring.sh   (or: make check-suite-wiring)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-suite-wiring.sh"

PASS=0
FAIL=0

# Run the guard against the tree at $1; assert its exit equals $2. $3 = name.
assert_exit() {
    local root="$1" want="$2" name="$3"
    local got=0
    bash "$GUARD" "$root" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

# Assert the guard's combined output for the tree at $1 contains the literal $2.
assert_out() {
    local root="$1" needle="$2" name="$3"
    local out
    out="$(bash "$GUARD" "$root" 2>&1 || true)"
    if [[ "$out" == *"$needle"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not contain: $needle)"
        FAIL=$((FAIL + 1))
    fi
}

# One root for the whole suite, removed by a trap however the suite exits.
#
# Registering each fixture dir in an array instead does NOT work here: new_tree
# is always called in a command substitution, so its body runs in a subshell and
# any array it appends to is discarded on return. (Same reasoning as
# scripts/test-check-dispatch-seats-declared.sh, and worth repeating rather than
# re-deriving.)
FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

# ── Fixture builders. new_tree echoes a root; everything else takes that root
#    as $1 and writes into it, so it can run in the caller's shell.

# A workflow carrying the step lines in $@ (already indented by the caller).
set_workflow() {
    local root="$1" name="$2"
    shift 2
    {
        printf 'name: fixture\non: [pull_request]\njobs:\n  j:\n    runs-on: ubuntu-22.04\n    steps:\n'
        local l
        for l in "$@"; do printf '%s\n' "$l"; done
    } > "$root/.github/workflows/$name"
}

# A Makefile. A line starting with `>` becomes a recipe line, i.e. is written
# with the leading TAB make requires — spelling the tab out in the fixture
# literal is how a harness silently stops testing recipes.
set_makefile() {
    local root="$1"
    shift
    : > "$root/Makefile"
    local l
    for l in "$@"; do
        if [[ "${l:0:1}" == ">" ]]; then
            printf '\t%s\n' "${l:1}" >> "$root/Makefile"
        else
            printf '%s\n' "$l" >> "$root/Makefile"
        fi
    done
}

# A registry declaring population floor $2 and the raw TSV lines in $@.
set_registry() {
    local root="$1" floor="$2"
    shift 2
    {
        printf '# fixture registry (mika#2761)\n'
        printf '# min-population: %s\n' "$floor"
        local l
        for l in "$@"; do printf '%s\n' "$l"; done
    } > "$root/scripts/suite-wiring.tsv"
}

# The same, with no `# min-population:` line at all.
set_registry_nofloor() {
    local root="$1"
    shift
    {
        printf '# fixture registry with no declared floor (mika#2761)\n'
        local l
        for l in "$@"; do printf '%s\n' "$l"; done
    } > "$root/scripts/suite-wiring.tsv"
}

# One well-formed registry row.
row() { printf '%s\t%s\t%s\t%s' "$1" "$2" "$3" "$4"; }

# A suite file at repo-relative path $2, carrying the extra body lines in $@.
put_suite() {
    local root="$1" rel="$2"
    shift 2
    mkdir -p "$root/$(dirname "$rel")"
    {
        printf '#!/usr/bin/env bash\n# fixture suite: %s\n' "$rel"
        local l
        for l in "$@"; do printf '%s\n' "$l"; done
    } > "$root/$rel"
}

# A tree with the three structural prerequisites present and nothing wired:
# one suite on disk, a workflow that names nothing, an empty registry.
new_tree() {
    local dir
    dir="$(mktemp -d -p "$FIXTURE_ROOT")"
    mkdir -p "$dir/scripts" "$dir/skills/bundled/_shared" "$dir/.github/workflows"
    set_makefile "$dir" '.PHONY: all' 'all:' '>@true'
    set_registry "$dir" 1
    set_workflow "$dir" ci.yml '      - name: nothing to do here' '        run: echo bonjour'
    echo "$dir"
}

# ─────────────────────────────────────────────────────────────────────────────
# 1. POSITIVE CONTROL — this repository's own tree agrees.
#
# If this goes red, either a suite landed unwired and undeclared (the mika#2761
# defect, caught) or the registry drifted. It is also the assertion that keeps
# the guard honest about ITSELF: both scripts of this pair are in the suite
# population, so removing the ci.yml job makes this case fail with a [D1] line
# naming them.
assert_exit "$REPO_ROOT" 0 "the repository's real tree: every suite reached or declared"

# ─────────────────────────────────────────────────────────────────────────────
# 2. THE LINT IS ITS OWN FIRST SUBJECT (mika#2761 AC5).
#
# Stated separately from §1 so the failure says "the job was removed" rather
# than making a reader derive it from a D1 line about two paths.
for self in scripts/check-suite-wiring.sh scripts/test-check-suite-wiring.sh; do
    if grep -qrF -- "$self" "$REPO_ROOT/.github/workflows/"; then
        echo "PASS: $self is named by a workflow (the lint wires itself)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $self is in the tree but no workflow names it — the lint is its own counter-example"
        FAIL=$((FAIL + 1))
    fi
done

# ─────────────────────────────────────────────────────────────────────────────
# 3. D1 — THE FOUNDING DEFECT, in the shape it shipped three times: a suite on
#    disk, no workflow reaching it, no declaration. If this ever goes green the
#    class is unprotected regardless of everything below.
T="$(new_tree)"
put_suite "$T" scripts/test-orpheline.sh
assert_exit "$T" 1 "a suite on disk that nothing reaches and nothing declares: rejected"
assert_out "$T" "ERROR [D1] scripts/test-orpheline.sh" \
    "the failure names the path and its direction, not just a count"
assert_out "$T" "WIRE IT" \
    "the report offers wiring first — the normal answer, not the exception"

# ...and the three ways out are each green.
T1="$(new_tree)"
put_suite "$T1" scripts/test-orpheline.sh
set_registry "$T1" 1 "$(row scripts/test-orpheline.sh unwired 'Runs offline, nothing runs it.' mika#2761)"
assert_exit "$T1" 0 "DECLARE IT: a well-formed registry entry clears the violation"

T2="$(new_tree)"
put_suite "$T2" scripts/test-orpheline.sh
set_workflow "$T2" ci.yml '      - name: run it' '        run: bash scripts/test-orpheline.sh'
assert_exit "$T2" 0 "WIRE IT (H1): a workflow naming the path clears the violation"

T3="$(new_tree)"
put_suite "$T3" scripts/test-gardee.sh
set_workflow "$T3" ci.yml '      - name: run it' '        run: bash scripts/test-gardee.sh'
assert_exit "$T3" 0 "DELETE IT: a tree with no orphan is green (the guard is satisfiable)"

# ─────────────────────────────────────────────────────────────────────────────
# 4. H1 PRECISION. Three ways the hop could be wrong, each of which has a real
#    counterpart in this repository's workflows.
#
# (a) A path inside a `run: |` block is a real invocation — the shape most
#     ci.yml steps take.
T="$(new_tree)"
put_suite "$T" scripts/test-bloc.sh
set_workflow "$T" ci.yml '      - name: several commands' '        run: |' \
    '          echo avant' '          bash scripts/test-bloc.sh' '          echo après'
assert_exit "$T" 0 "a path inside a \`run: |\` block counts as reached"

# (b) A path in a COMMENT reaches nothing. ci.yml:256 carries the prose "make
#     the lint above a measurement"; a guard that read comments would be
#     counting documentation as coverage.
T="$(new_tree)"
put_suite "$T" scripts/test-commentee.sh
set_workflow "$T" ci.yml \
    '      # TODO: one day run bash scripts/test-commentee.sh here (mika#2761)' \
    '      - name: nothing' '        run: echo rien'
assert_exit "$T" 1 "a path mentioned only in a workflow comment is NOT reached"
assert_out "$T" "ERROR [D1] scripts/test-commentee.sh" \
    "the comment-only mention is reported, not silently credited"

# (c) A neighbour whose name CONTAINS the suite's path must not satisfy it. A
#     `.bak` left by an editor is the realistic way this happens, and a
#     substring match would credit the real file for its own stale copy.
T="$(new_tree)"
put_suite "$T" scripts/test-voisine.sh
set_workflow "$T" ci.yml '      - name: runs the stale copy' \
    '        run: bash scripts/test-voisine.sh.bak'
assert_exit "$T" 1 "\`test-voisine.sh.bak\` in the workflow does not reach \`test-voisine.sh\`"

# ...while the converse must not over-fire: two real suites whose names are
# prefixes of one another are both reached when both are named.
T="$(new_tree)"
put_suite "$T" scripts/check-x.sh
put_suite "$T" scripts/test-check-x.sh
set_workflow "$T" ci.yml '      - name: both' '        run: |' \
    '          bash scripts/check-x.sh' '          bash scripts/test-check-x.sh'
assert_exit "$T" 0 "\`check-x.sh\` and \`test-check-x.sh\` are told apart, both reached"

# ─────────────────────────────────────────────────────────────────────────────
# 5. THE UNDER-APPROXIMATION OF "REACHED" IS DELIBERATE.
#
# A suite invoked from another suite's shell BODY is not proven reached. The
# real instance: skills/bundled/_shared/_shared/test-dispatch-lib.sh invokes
# scripts/check-pilot-turn-ceiling-labels.sh. Following that would mean
# interpreting shell — conditionals, variables, `$@` — which errs in both
# directions, and the direction that costs is the false green. So this case is
# a false RED by design: noisy, never silent. Pinned so nobody "fixes" it into
# the silence mika#2761 exists to remove.
T="$(new_tree)"
put_suite "$T" scripts/test-appelee.sh
put_suite "$T" scripts/check-appelante.sh 'bash scripts/test-appelee.sh'
set_workflow "$T" ci.yml '      - name: run the caller' \
    '        run: bash scripts/check-appelante.sh'
assert_exit "$T" 1 "reachability through another suite's shell body is NOT proven (by design)"
assert_out "$T" "ERROR [D1] scripts/test-appelee.sh" \
    "the unproven case is a false red, which the registry can answer"

# ─────────────────────────────────────────────────────────────────────────────
# 6. H2 — via a `make <target>` a workflow names. This hop is what makes the
#    guard usable at all: 44 suites in this repository are reached only through
#    a make target.
#
# (a) The direct form.
T="$(new_tree)"
put_suite "$T" scripts/test-par-make.sh
set_makefile "$T" '.PHONY: check-tout' 'check-tout:' '>@bash scripts/test-par-make.sh'
set_workflow "$T" ci.yml '      - name: via make' '        run: make check-tout'
assert_exit "$T" 0 "H2: a workflow naming \`make check-tout\` reaches that recipe's suite"

# (b) The same Makefile, with the workflow no longer naming the target. This is
#     the negative control for H2: without it, a guard that credited every
#     recipe in the Makefile would pass §(a) for the wrong reason — and `make
#     test` is precisely a target no workflow calls.
T="$(new_tree)"
put_suite "$T" scripts/test-par-make.sh
set_makefile "$T" '.PHONY: check-tout' 'check-tout:' '>@bash scripts/test-par-make.sh'
set_workflow "$T" ci.yml '      - name: something else' '        run: echo rien'
assert_exit "$T" 1 "H2 negative control: a recipe no workflow invokes reaches nothing"
assert_out "$T" "ERROR [D1] scripts/test-par-make.sh" \
    "the unreached make target is reported against the suite it was meant to run"

# (c) PREREQUISITE TRANSITIVITY. `deploy: … build …` and `check-ngrok:
#     check-webhook-chain` are the real instances; a guard that read only the
#     named target's own recipe would miss every suite behind a prerequisite.
T="$(new_tree)"
put_suite "$T" scripts/test-prerequis.sh
set_makefile "$T" '.PHONY: haut bas' 'haut: bas' '>@echo haut' 'bas:' '>@bash scripts/test-prerequis.sh'
set_workflow "$T" ci.yml '      - name: top target only' '        run: make haut'
assert_exit "$T" 0 "H2: a suite behind a prerequisite of the named target is reached"

# (d) RECURSIVE MAKE. No `$(MAKE)` exists in this repository's Makefile today,
#     so this is the case that would rot unnoticed — which is the only reason to
#     pin it now rather than when the first one lands.
T="$(new_tree)"
put_suite "$T" scripts/test-recursif.sh
set_makefile "$T" '.PHONY: parent enfant' 'parent:' '>@$(MAKE) enfant' 'enfant:' '>@bash scripts/test-recursif.sh'
set_workflow "$T" ci.yml '      - name: recursive' '        run: make parent'
assert_exit "$T" 0 "H2: \`\$(MAKE) enfant\` inside a reached recipe reaches the child's suite"

# (e) A make target named only in a workflow comment reaches nothing either.
T="$(new_tree)"
put_suite "$T" scripts/test-make-commentee.sh
set_makefile "$T" '.PHONY: check-tout' 'check-tout:' '>@bash scripts/test-make-commentee.sh'
set_workflow "$T" ci.yml '      # we should run make check-tout here' \
    '      - name: nothing' '        run: echo rien'
assert_exit "$T" 1 "a \`make <target>\` named only in a comment reaches nothing"

# (f) A `#`-stripped line inside a recipe must not end the recipe. GNU make does
#     not treat a blank line as a recipe terminator, and comment stripping turns
#     a commented recipe line into one — so a guard that stopped there would
#     lose every suite named after a comment in the same recipe.
T="$(new_tree)"
put_suite "$T" scripts/test-apres-commentaire.sh
set_makefile "$T" '.PHONY: check-tout' 'check-tout:' '>@echo avant' \
    '># a comment in the middle of the recipe' '>@bash scripts/test-apres-commentaire.sh'
set_workflow "$T" ci.yml '      - name: via make' '        run: make check-tout'
assert_exit "$T" 0 "a commented line inside a recipe does not truncate it"

# ─────────────────────────────────────────────────────────────────────────────
# 7. D2 — THE ENTRY THAT OUTLIVED ITS REASON. This is what closes the mika#2309
#    allowlist trap: the gesture that makes an exception pointless (wiring the
#    suite) is the gesture that makes it FAIL. An allowlist nobody is forced to
#    prune is an allowlist that fills up.
T="$(new_tree)"
put_suite "$T" scripts/test-devenue-cablee.sh
set_workflow "$T" ci.yml '      - name: now wired' \
    '        run: bash scripts/test-devenue-cablee.sh'
set_registry "$T" 1 "$(row scripts/test-devenue-cablee.sh unwired 'Nothing runs it.' mika#2761)"
assert_exit "$T" 1 "a declared suite that CI now reaches: rejected (the entry is stale)"
assert_out "$T" "ERROR [D2] scripts/test-devenue-cablee.sh" \
    "the stale entry is named, with \"delete the entry\" as the fix"

set_registry "$T" 1
assert_exit "$T" 0 "removing the stale entry clears it — the registry self-cleans"

# ─────────────────────────────────────────────────────────────────────────────
# 8. D3 — DECLARED, BUT NOT A SUITE. Two distinct causes, two distinct messages,
#    because the fix differs: a deleted file means prune the entry, a file
#    outside the convention means prune OR rename.
T="$(new_tree)"
put_suite "$T" scripts/test-bien-la.sh
set_workflow "$T" ci.yml '      - name: wired' '        run: bash scripts/test-bien-la.sh'
set_registry "$T" 1 "$(row scripts/test-disparue.sh unwired 'Deleted last month.' mika#2761)"
assert_exit "$T" 1 "a declared path that does not exist: rejected"
assert_out "$T" "does not exist — delete the entry" \
    "the vanished file gets the \"delete the entry\" message"

set_registry "$T" 1 "$(row scripts/deploy-helper.sh unwired 'Not a suite at all.' mika#2761)"
put_suite "$T" scripts/deploy-helper.sh
assert_exit "$T" 1 "a declared file that carries no suite prefix: rejected"
assert_out "$T" "is not in the suite population" \
    "the out-of-convention file gets the prefix explanation, not \"does not exist\""

set_registry "$T" 1
assert_exit "$T" 0 "pruning both entries clears D3"

# ─────────────────────────────────────────────────────────────────────────────
# 9. D4 — THE DECLARATION'S FORM. The machine cannot ask whether a script needs
#    the outside world (mika#1801), so it checks the form instead: a mode in the
#    closed vocabulary, a non-empty reason, a ticket. An entry with no reason is
#    exactly the allowlist mika#2309 condemns — it says "skip me" and nothing
#    else — so emptiness is a violation, not a default.
base_tree_with_orphan() {
    local d
    d="$(new_tree)"
    put_suite "$d" scripts/test-a-declarer.sh
    echo "$d"
}

T="$(new_tree)"
put_suite "$T" scripts/test-a-declarer.sh
set_registry "$T" 1 "$(row scripts/test-a-declarer.sh unwired 'Runs offline.' mika#2761)"
assert_exit "$T" 0 "the well-formed entry is accepted (the form is satisfiable)"

set_registry "$T" 1 "$(printf '%s\t%s\t%s\t%s\t%s' scripts/test-a-declarer.sh unwired r mika#2761 surplus)"
assert_exit "$T" 1 "a fifth column is rejected"
assert_out "$T" "more than four tab-separated columns" "the column count is named"

set_registry "$T" 1 "scripts/test-a-declarer.sh"
assert_exit "$T" 1 "a path with no mode is rejected"

set_registry "$T" 1 "$(row scripts/test-a-declarer.sh local-only 'Runs offline.' mika#2761)"
assert_exit "$T" 1 "a mode outside the closed vocabulary is rejected"
assert_out "$T" "is outside the vocabulary" "the rejected mode is named with the vocabulary"

set_registry "$T" 1 "$(row scripts/test-a-declarer.sh unwired '' mika#2761)"
assert_exit "$T" 1 "an empty reason is rejected (the mika#2309 allowlist shape)"
assert_out "$T" "mika#2309" "the empty-reason failure cites why emptiness is the defect"

set_registry "$T" 1 "$(row scripts/test-a-declarer.sh unwired 'Runs offline.' 'see the other ticket')"
assert_exit "$T" 1 "a ticket column citing no \`mika#<n>\` is rejected"

set_registry "$T" 1 \
    "$(row scripts/test-a-declarer.sh unwired 'Runs offline.' mika#2761)" \
    "$(row scripts/test-a-declarer.sh operator 'No, it needs a token.' mika#2761)"
assert_exit "$T" 1 "the same path declared twice is rejected"
assert_out "$T" "more than once" "the duplicate path is named"

# `operator` is the other half of the vocabulary and must be accepted.
set_registry "$T" 1 "$(row scripts/test-a-declarer.sh operator 'Needs a cross-repo token.' mika#1997)"
assert_exit "$T" 0 "\`operator\` is accepted as readily as \`unwired\`"

# ─────────────────────────────────────────────────────────────────────────────
# 10. THE POSITIVE POPULATION CONTROL (mika#2205; layer 2 of the mika#1801
#     taxonomy, "empty population"). A set comparison over an empty D agrees
#     with everything. Without this, a renamed root or a changed prefix
#     convention would turn this guard into the decoration it was written
#     against — and its "0 violations" would read exactly like a clean tree.
T="$(new_tree)"
put_suite "$T" scripts/test-une.sh
put_suite "$T" scripts/test-deux.sh
set_workflow "$T" ci.yml '      - name: both' '        run: |' \
    '          bash scripts/test-une.sh' '          bash scripts/test-deux.sh'
set_registry "$T" 2
assert_exit "$T" 0 "a population exactly at its declared floor is green"

set_registry "$T" 5
assert_exit "$T" 3 "a population under its declared floor exits 3, never 0"
assert_out "$T" "suite population collapsed: found 2, declared floor is 5" \
    "the collapse message gives both numbers"

set_registry_nofloor "$T"
assert_exit "$T" 3 "a registry declaring no floor exits 3 — the control cannot be omitted"
assert_out "$T" "declares no \`# min-population: <n>\` floor" "the missing floor is named"

# ─────────────────────────────────────────────────────────────────────────────
# 11. STRUCTURAL PREREQUISITES. Each of the three inputs missing must exit 2,
#     and an empty workflow directory must exit 3 — a guard with no notion of
#     "reached" to compare against must say so rather than agree.
T="$(new_tree)"
put_suite "$T" scripts/test-une.sh
rm "$T/scripts/suite-wiring.tsv"
assert_exit "$T" 2 "no registry: exit 2, not a silent pass"

T="$(new_tree)"
put_suite "$T" scripts/test-une.sh
rm "$T/Makefile"
assert_exit "$T" 2 "no Makefile: exit 2 (H2 would be silently empty)"

T="$(new_tree)"
put_suite "$T" scripts/test-une.sh
rm -r "$T/.github/workflows"
assert_exit "$T" 2 "no workflow directory: exit 2"

T="$(new_tree)"
put_suite "$T" scripts/test-une.sh
rm "$T/.github/workflows/ci.yml"
assert_exit "$T" 3 "a workflow directory with no workflow in it: exit 3"
assert_out "$T" "no workflow file found" "the empty workflow directory is named"

# A `.yaml` workflow counts as much as a `.yml` one — the glob must not be the
# place the guard quietly stops looking.
T="$(new_tree)"
put_suite "$T" scripts/test-yaml.sh
rm "$T/.github/workflows/ci.yml"
set_workflow "$T" autre.yaml '      - name: runs it' '        run: bash scripts/test-yaml.sh'
assert_exit "$T" 0 "a .yaml workflow is read, not only .yml"

# ─────────────────────────────────────────────────────────────────────────────
# 12. ACCENTED FIXTURES — our actual population. The suite path, the reason and
#     the containing directory all carry accents.
T="$(new_tree)"
put_suite "$T" 'scripts/test-périmètre-dégradé.sh'
assert_exit "$T" 1 "an accented suite path is enumerated, not skipped"
assert_out "$T" "ERROR [D1] scripts/test-périmètre-dégradé.sh" \
    "the failure message reproduces the accented path intact"

set_registry "$T" 1 "$(row 'scripts/test-périmètre-dégradé.sh' unwired \
    "Tourne hors réseau ; rien ne l'exécute. Déclaré ici en attendant son câblage." mika#2761)"
assert_exit "$T" 0 "an accented path and an accented reason are accepted"

set_registry "$T" 1 "$(row 'scripts/test-périmètre-dégradé.sh' unwired '' mika#2761)"
assert_exit "$T" 1 "the empty-reason rule still bites on an accented entry"

# The guard is also run from a root whose own path carries accents and a space —
# the shape a dispatch worktree takes.
ACCENTED_ROOT="$FIXTURE_ROOT/dépôt de l'opérateur"
mkdir -p "$ACCENTED_ROOT/scripts" "$ACCENTED_ROOT/skills/bundled/_shared" \
    "$ACCENTED_ROOT/.github/workflows"
set_makefile "$ACCENTED_ROOT" '.PHONY: all' 'all:' '>@true'
set_registry "$ACCENTED_ROOT" 1
set_workflow "$ACCENTED_ROOT" ci.yml '      - name: nothing' '        run: echo rien'
put_suite "$ACCENTED_ROOT" scripts/test-dans-un-chemin-accentue.sh
assert_exit "$ACCENTED_ROOT" 1 "a root path with accents and a space is read correctly"
assert_out "$ACCENTED_ROOT" "ERROR [D1] scripts/test-dans-un-chemin-accentue.sh" \
    "paths are reported relative to the root, with the root's accents stripped off"

# ─────────────────────────────────────────────────────────────────────────────
# 13. THE REPORT IS ALWAYS PRINTED, including on a green run. A lint whose only
#     green output is silence does not let you see that it stopped looking —
#     the counts are the instrument (mika#2205).
T="$(new_tree)"
put_suite "$T" scripts/test-une.sh
set_workflow "$T" ci.yml '      - name: wired' '        run: bash scripts/test-une.sh'
assert_out "$T" "suite wiring: |D|=1 (floor 1)  |C|=1  |R|=0  unwired=0" \
    "a green run still prints |D|, |C|, |R| and the unwired count"

# ...and the unwired count is the backlog's size, which is the number the
# Fire-Disposition is accountable to.
T="$(new_tree)"
put_suite "$T" scripts/test-une.sh
put_suite "$T" scripts/test-deux.sh
set_registry "$T" 1 \
    "$(row scripts/test-une.sh unwired 'Runs offline.' mika#2761)" \
    "$(row scripts/test-deux.sh operator 'Needs a token.' mika#2761)"
assert_out "$T" "unwired=1" "the unwired count counts \`unwired\` only, not \`operator\`"

echo ""
echo "check-suite-wiring anti-vacuity: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
