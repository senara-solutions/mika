#!/bin/bash
# Fixtures de la porte de grooming, partagées entre familles (mika#3178, vol 2).
#
# POURQUOI CE FICHIER EXISTE, et c'est l'agrégat qui le dit. Ses lignes 1426-1428
# annoncent la structure que la scission rencontre :
#
#     # The five cases below are DEFINED here next to the gate's code-shape
#     # assertions, but INVOKED near the end of this file — they need
#     # `_fixture_setup`, which is defined further down.
#
# Les douze cas `test_groom_gate_*` sont donc définis dans la famille 2
# (l. 1429-1827), `_fixture_setup` dans la famille 4 (l. 2826), et la boucle qui
# les appelle dans la famille 5 (l. 4950-4988). Trois familles, une seule chaîne
# définition → fixture → appel : AUCUN découpage en plages contiguës ne la garde
# entière. Mesuré au vol 2 — la famille 5 extraite seule rendait 12 `rc=127`,
# quand l'agrégat en rend 0.
#
# C'est le cas que le ticket tranche d'avance : « un helper utilisé par plusieurs
# familles va dans cette bibliothèque ; un helper propre à une famille reste avec
# elle. » Les deux plages extraites ici ne contiennent AUCUNE assertion et aucun
# `echo` de section — seulement des définitions de fonctions et les constantes
# qu'elles lisent. Sourcer ce fichier ne compte donc rien : les compteurs d'une
# suite restent ceux de sa propre plage. Vérifié ligne à ligne avant extraction.
#
# Plages d'origine, à la base 890f3f5f, extraites par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) — mot pour mot par construction :
#   - l. 2824-2855 : `_fixture_setup`, `_fixture_cleanup`,
#     `_assert_fixture_is_local` (famille 4). D'abord, car les douze cas les
#     appellent.
#   - l. 1426-1827 : les douze cas `test_groom_gate_*`,
#     `_fixture_foreign_plan_on_main`, et les quatre constantes de corps et de
#     chemin qu'ils lisent (famille 2).
#
# Les deux plages restent AUSSI dans le corps des suites 2 et 4, qui copient leur
# famille mot pour mot : une redéfinition de fonction est sans effet, et c'est ce
# qui garde le `diff` d'AC3 de ces deux suites exact. L'agrégat n'est pas touché à
# ce vol ; au vol 7, qui le supprime, ce fichier devient l'unique définition.
#
# Comme ses voisins `lib-mika-bin.sh` et `lib-dispatch-lib-suite.sh`, son nom ne
# porte aucun des cinq préfixes que `scripts/check-suite-wiring.sh` reconnaît :
# il n'est pas une suite, n'a rien à déclarer dans `scripts/suite-wiring.tsv`, et
# rien ne le lance directement.
# Helper: create a throwaway bare repo + working clone, wired with file:// remotes.
# Sets FIXTURE_BARE, FIXTURE_CLONE. Caller must clean up via _fixture_cleanup.
_fixture_setup() {
    FIXTURE_BARE=$(mktemp -d)
    FIXTURE_CLONE=$(mktemp -d)

    git -C "$FIXTURE_BARE" init --bare -q
    git clone -q "file://$FIXTURE_BARE" "$FIXTURE_CLONE"
    git -C "$FIXTURE_CLONE" config user.email "test@mika.local"
    git -C "$FIXTURE_CLONE" config user.name "mika test"

    # Initial commit on main so we have a base.
    echo "base" > "$FIXTURE_CLONE/file.txt"
    git -C "$FIXTURE_CLONE" add file.txt
    git -C "$FIXTURE_CLONE" commit -q -m "initial commit"
    git -C "$FIXTURE_CLONE" push -q origin main
}

_fixture_cleanup() {
    rm -rf "$FIXTURE_BARE" "$FIXTURE_CLONE" 2>/dev/null
}

# Safety assertion: verify fixture remotes are local file:// paths, never the real origin.
_assert_fixture_is_local() {
    local remote_url
    remote_url=$(git -C "$FIXTURE_CLONE" remote get-url origin 2>/dev/null)
    if ! grep -q -- "^file://\|^/" <<<"$remote_url"; then
        echo "SAFETY ABORT: fixture remote is not local: $remote_url" >&2
        _fixture_cleanup
        return 1
    fi
}
# The five cases below are DEFINED here next to the gate's code-shape
# assertions, but INVOKED near the end of this file — they need
# `_fixture_setup`, which is defined further down.
CALLOUT_PRESENT_BODY='## Symptom
Some text.

> - **Branch:** `fix/2012/plan-gate`
> - **Plan:** `docs/plans/2026-08-27-001-plan.md` (committed on branch @ `deadbeef`)
> - **Grooming history:** first-pass (READY, single-pass GROOMED) — no second pass required — session-id: t1
'

# --- Case 1 (the discriminating one): callout present, file ABSENT on branch.
# The gate must NOT fire — this ticket genuinely needs grooming.
test_groom_gate_callout_present_file_absent() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # Branch exists on the remote but carries NO plan file.
    git -C "$FIXTURE_CLONE" checkout -q -b fix/2012/plan-gate
    echo "code" > "$FIXTURE_CLONE/src.txt"
    git -C "$FIXTURE_CLONE" add src.txt
    git -C "$FIXTURE_CLONE" commit -q -m "work, no plan"
    git -C "$FIXTURE_CLONE" push -q origin fix/2012/plan-gate

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/plan-gate" "$CALLOUT_PRESENT_BODY" "mika" >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then echo "PASS"; else echo "FAIL: gate fired on a callout whose plan file is absent — would strand the ticket"; fi
}

# --- Case 2: callout present AND file committed on branch → gate fires.
test_groom_gate_plan_committed() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b fix/2012/plan-gate
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    echo "# plan" > "$FIXTURE_CLONE/docs/plans/2026-08-27-001-plan.md"
    git -C "$FIXTURE_CLONE" add docs/plans/2026-08-27-001-plan.md
    git -C "$FIXTURE_CLONE" commit -q -m "commit plan"
    git -C "$FIXTURE_CLONE" push -q origin fix/2012/plan-gate

    local out rc=0
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/plan-gate" "$CALLOUT_PRESENT_BODY" "mika" 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "docs/plans/2026-08-27-001-plan.md" ]; then
        echo "PASS"
    else
        echo "FAIL: expected rc=0 and the plan path, got rc=$rc out='$out'"
    fi
}

# --- Case 3: repo-prefixed callout form (`mika/docs/plans/...`) still resolves.
# Tickets groomed before U3's path normalization carry this shape.
test_groom_gate_repo_prefixed_path() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b fix/2012/prefixed
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    echo "# plan" > "$FIXTURE_CLONE/docs/plans/legacy-plan.md"
    git -C "$FIXTURE_CLONE" add docs/plans/legacy-plan.md
    git -C "$FIXTURE_CLONE" commit -q -m "commit plan"
    git -C "$FIXTURE_CLONE" push -q origin fix/2012/prefixed

    local prefixed_body out rc=0
    prefixed_body='> - **Plan:** `mika/docs/plans/legacy-plan.md` (committed on branch @ `abc1234`)'
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/prefixed" "$prefixed_body" "mika" 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "docs/plans/legacy-plan.md" ]; then
        echo "PASS"
    else
        echo "FAIL: repo-prefixed callout should resolve to the relative path, got rc=$rc out='$out'"
    fi
}

# --- Case 4: no Plan callout at all → gate never fires (ungroomed ticket).
test_groom_gate_no_callout() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "main" "## Symptom
Plain ungroomed body, no callout." "mika" >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then echo "PASS"; else echo "FAIL: gate fired on a body with no Plan callout"; fi
}

# --- Case 5: branch does not exist on the remote → gate must not fire.
test_groom_gate_branch_absent() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/never-pushed" "$CALLOUT_PRESENT_BODY" "mika" >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then echo "PASS"; else echo "FAIL: gate fired although the dispatch branch does not exist on the remote"; fi
}

# ----------------------------------------------------------------------------
# Issue binding in the dispatch gate (mika#2034)
# ----------------------------------------------------------------------------
# Cases 1–5 above all run against a fixture whose `main` carries no plan files.
# That is precisely why they were green while the defect was live: the fixture
# could not express the production shape, where every dispatch branch descends
# from a `main` carrying 769 plans and therefore resolves ANY valid plan path.
# The gate read the path out of the ticket's own callout and asked only whether
# it resolved — an attestation produced by the claim it was meant to check.
#
# Measured 2026-08-30, both stranded by that gate:
#   mika#1887 → a plan whose header reads `issue: senara-solutions/mika#1933`
#   mika#2026 → a plan whose header reads `**Issue:** #539`
#
# Case 6 is the one that would have caught them.

# Writes a plan file on `main` whose header claims $2, and pushes. The branch in
# $3 is then created on top and adds NOTHING — so the plan resolves on it purely
# by inheritance, exactly as in production.
_fixture_foreign_plan_on_main() {
    local plan_rel="$1" claims_issue="$2" branch="$3"
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    {
        printf -- '---\n'
        printf 'issue: senara-solutions/mika#%s\n' "$claims_issue"
        printf 'type: fix\n'
        printf -- '---\n\n'
        printf '# Plan — a plan belonging to another ticket\n\n'
        printf 'Padding so the file clears any size filter. %s\n' \
            "$(head -c 600 /dev/zero | tr '\0' 'x')"
    } > "$FIXTURE_CLONE/$plan_rel"
    git -C "$FIXTURE_CLONE" add "$plan_rel"
    git -C "$FIXTURE_CLONE" commit -q -m "plan for another ticket, on main"
    git -C "$FIXTURE_CLONE" push -q origin main

    git -C "$FIXTURE_CLONE" checkout -q -b "$branch"
    echo "code" > "$FIXTURE_CLONE/src.txt"
    git -C "$FIXTURE_CLONE" add src.txt
    git -C "$FIXTURE_CLONE" commit -q -m "work, no plan of its own"
    git -C "$FIXTURE_CLONE" push -q origin "$branch"
}

FOREIGN_PLAN_REL='docs/plans/2026-08-21-002-fix-1933-reader-completed-section-plan.md'
FOREIGN_PLAN_BODY='## Symptom
Some text.

> - **Branch:** `feat/1887/research-peer-b`
> - **Plan:** `docs/plans/2026-08-21-002-fix-1933-reader-completed-section-plan.md` (committed on branch @ `deadbeef`)
'

# --- Case 6 (the discriminating one): the plan the body names belongs to
# ANOTHER ticket and the branch merely inherited it from main. The gate must NOT
# fire — this ticket has never been groomed.
test_groom_gate_foreign_plan_inherited_from_main() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    local rc=0 err
    err=$(_committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 1887 2>&1 >/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ]; then
        echo "FAIL: gate fired on a plan whose header claims issue 1933 while grooming 1887 — this is the mika#1887 / mika#2026 strand"
    elif ! grep -q -- 'dispatch_gate_groom_plan_refuted' <<<"$err"; then
        echo "FAIL: gate declined but emitted no greppable refusal diagnostic; got: $err"
    elif ! grep -q -- '1933' <<<"$err"; then
        echo "FAIL: refusal diagnostic does not name the issue the plan claims; got: $err"
    else
        echo "PASS"
    fi
}

# --- Case 7: same file, but its header claims the TARGET issue → gate fires.
# Without this, case 6 could be satisfied by refusing everything.
test_groom_gate_plan_header_claims_target_issue() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1887 "feat/1887/research-peer-b"

    local out rc=0
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 1887 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "$FOREIGN_PLAN_REL" ]; then
        echo "PASS"
    else
        echo "FAIL: a plan whose header claims the target issue must still fire the gate, got rc=$rc out='$out'"
    fi
}

# --- Case 8: the plan carries NO issue marker at all → gate still fires.
# Refutation, not confirmation: 95 of the 745 plans in docs/plans/ name no
# issue, and demanding a positive match would strand every one of them.
test_groom_gate_plan_without_issue_marker_still_fires() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    mkdir -p "$FIXTURE_CLONE/docs/plans"
    printf '# Plan — no issue marker anywhere in this header\n\nBody.\n' \
        > "$FIXTURE_CLONE/$FOREIGN_PLAN_REL"
    git -C "$FIXTURE_CLONE" add "$FOREIGN_PLAN_REL"
    git -C "$FIXTURE_CLONE" commit -q -m "markerless plan on main"
    git -C "$FIXTURE_CLONE" push -q origin main
    git -C "$FIXTURE_CLONE" checkout -q -b feat/1887/research-peer-b
    git -C "$FIXTURE_CLONE" push -q origin feat/1887/research-peer-b

    local out rc=0
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 1887 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "$FOREIGN_PLAN_REL" ]; then
        echo "PASS"
    else
        echo "FAIL: silence is not refutation — a markerless plan must still fire the gate, got rc=$rc out='$out'"
    fi
}

# --- Case 9: the four existing call shapes keep working with no 5th argument.
# KTD2 — the parameter is optional; a required one would break all five cases
# above and both production call sites.
test_groom_gate_issue_arg_is_optional() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    # No 5th argument and no ambient ISSUE_NUM: the binding check is skipped
    # rather than guessed, so the gate behaves exactly as it did before mika#2034.
    local out rc=0
    out=$(ISSUE_NUM='' _committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "$FOREIGN_PLAN_REL" ]; then
        echo "PASS"
    else
        echo "FAIL: the 5th argument must be optional, got rc=$rc out='$out'"
    fi
}

# --- Case 10: provenance is measured, not asserted. A blob identical to main's
# is reported as inherited; one the branch actually added is reported as
# committed. This is the sentence the operator reads.
test_plan_provenance_distinguishes_inherited_from_committed() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    # Prime the gate ref the same way _committed_plan_on_branch does.
    git -C "$FIXTURE_CLONE" fetch --quiet --force origin \
        "refs/heads/feat/1887/research-peer-b:refs/dispatch-gate/feat/1887/research-peer-b" 2>/dev/null

    local inherited committed own_plan='docs/plans/2026-08-30-001-fix-2034-own-plan.md'
    inherited=$(_plan_provenance "$FIXTURE_CLONE" "feat/1887/research-peer-b" "$FOREIGN_PLAN_REL")

    printf -- '---\nissue: senara-solutions/mika#1887\n---\n\nOwn plan.\n' \
        > "$FIXTURE_CLONE/$own_plan"
    git -C "$FIXTURE_CLONE" add "$own_plan"
    git -C "$FIXTURE_CLONE" commit -q -m "this branch's own plan"
    git -C "$FIXTURE_CLONE" push -q origin feat/1887/research-peer-b
    git -C "$FIXTURE_CLONE" fetch --quiet --force origin \
        "refs/heads/feat/1887/research-peer-b:refs/dispatch-gate/feat/1887/research-peer-b" 2>/dev/null
    committed=$(_plan_provenance "$FIXTURE_CLONE" "feat/1887/research-peer-b" "$own_plan")

    _fixture_cleanup
    if ! grep -q -- 'inherited unchanged from main' <<<"$inherited"; then
        echo "FAIL: a blob identical to main's must be reported as inherited, got '$inherited'"
    elif ! grep -q -- 'committed on the dispatch branch' <<<"$committed"; then
        echo "FAIL: a blob the branch added must be reported as committed, got '$committed'"
    else
        echo "PASS"
    fi
}

# --- Case 11: a callout naming a DIRECTORY must not fire the gate.
# Written to pin the unbindable-candidate path and it found a real one instead:
# `cat-file -e` is satisfied by a tree, and `git show` on a tree prints a
# listing — non-empty, carrying no issue header, therefore "not refuted". A
# ticket whose callout said `docs/plans` was refused grooming on the strength of
# a directory. The gate now demands a blob.
test_groom_gate_declines_on_directory_candidate() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "> - **Plan:** \`docs/plans\` (committed on branch @ \`deadbeef\`)" \
        "mika" 1887 >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then
        echo "PASS"
    else
        echo "FAIL: a directory is not a plan — the gate must not fire on one"
    fi
}

# ----------------------------------------------------------------------------
# mika#2158 — le croisement contractuel Rust ↔ Bash (AC7)
# ----------------------------------------------------------------------------
#
# Trois composants répondent à « ce ticket est-il groomé ? » : `is_groomed`
# (promotion, Rust), `check_grooming_markers` (routage du dispatch, Rust) et
# cette garde (refus, Bash). Les deux premiers sont croisés entre eux dans
# `grooming_marker::tests::ac7_both_rust_predicates_agree_on_the_frozen_bodies`.
# Le troisième ne peut pas l'être au même endroit : il mesure un artefact git
# que le Rust ne peut pas évaluer sans I/O.
#
# Le croisement testable est donc unidirectionnel, et c'est celui qui compte :
#
#     si la garde refuse pour `already_groomed`, alors le Rust doit dire groomé.
#
# C'est exactement l'implication qui était fausse avant mika#2158, et c'est ce
# qui faisait le livelock : la garde refusait le grooming de #2108/#2127 parce
# que leur plan était sur la branche, pendant que le Rust les déclarait non
# groomés et les redemandait en grooming toutes les dix minutes. Douze refus en
# trois heures et demie, sans qu'aucun état ne change entre deux tours.
#
# L'implication INVERSE n'est pas vraie et ne doit pas être testée : un ticket
# peut être groomé et son plan absent de la branche (jamais poussé), et c'est le
# cas que la branche `elif` du site d'appel traite déjà.
#
# La moitié Rust est vérifiée ici structurellement, pas paraphrasée : le test
# échoue si la fixture n'est plus dans la table Rust. Retirer `2108.md` de
# `grooming_marker.rs` casse ce test — ce qui est le point.
GROOMING_FIXTURE_DIR="$SCRIPT_DIR/../../../crates/mika-agent/tests/fixtures/grooming_bodies"
GROOMING_MARKER_RS="$SCRIPT_DIR/../../../crates/mika-agent/src/grooming_marker.rs"

test_groom_gate_refusal_implies_rust_says_groomed() {
    local ticket fixture body plan_path branch rc out problems=""

    if [ ! -d "$GROOMING_FIXTURE_DIR" ]; then
        echo "FAIL: fixture dir absent: $GROOMING_FIXTURE_DIR"
        return 0
    fi
    if [ ! -f "$GROOMING_MARKER_RS" ]; then
        echo "FAIL: grooming_marker.rs absent: $GROOMING_MARKER_RS"
        return 0
    fi

    # Les deux corps que la garde refusait pendant que le Rust les redemandait :
    # #2108 (première passe READY, AC1) et #2127 (GROOMED après arbitrage, AC3).
    for ticket in 2108 2127; do
        fixture="$GROOMING_FIXTURE_DIR/$ticket.md"
        if [ ! -f "$fixture" ]; then
            problems="$problems; fixture $ticket.md absente"
            continue
        fi
        # Moitié Rust : la fixture doit être dans la table du module.
        if ! grep -qF "grooming_bodies/$ticket.md" "$GROOMING_MARKER_RS"; then
            problems="$problems; $ticket.md n'est plus dans la table Rust de grooming_marker.rs"
            continue
        fi

        body=$(cat "$fixture")
        plan_path=$(printf '%s\n' "$body" \
            | sed -n 's/^> - \*\*Plan:\*\* *`\([^`]*\)`.*/\1/p' | head -1)
        branch=$(printf '%s\n' "$body" \
            | sed -n 's/^> - \*\*Branch:\*\* *`\([^`]*\)`.*/\1/p' | head -1)
        if [ -z "$plan_path" ] || [ -z "$branch" ]; then
            problems="$problems; $ticket.md ne porte pas les callouts Branch/Plan"
            continue
        fi

        _fixture_setup
        _assert_fixture_is_local || return 1

        git -C "$FIXTURE_CLONE" checkout -q -b "$branch"
        mkdir -p "$FIXTURE_CLONE/$(dirname "$plan_path")"
        printf '# Plan — mika#%s\n' "$ticket" > "$FIXTURE_CLONE/$plan_path"
        git -C "$FIXTURE_CLONE" add "$plan_path"
        git -C "$FIXTURE_CLONE" commit -q -m "commit plan for $ticket"
        git -C "$FIXTURE_CLONE" push -q origin "$branch"

        rc=0
        out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "$branch" "$body" "mika" "$ticket" 2>/dev/null) || rc=$?
        _fixture_cleanup

        if [ "$rc" -ne 0 ] || [ "$out" != "$plan_path" ]; then
            problems="$problems; $ticket: la garde n'a pas refusé (rc=$rc out='$out', attendu '$plan_path')"
        fi
    done

    if [ -z "$problems" ]; then
        echo "PASS"
    else
        echo "FAIL${problems}"
    fi
}
