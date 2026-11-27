#!/bin/bash
# Porte de dispatch du groom (mika#2811) — extrait de test-dispatch-lib.sh par mika#3178, vol 2.
# Le cas où le refus de la porte de dispatch EST la raison du groom, et l'absence
# des freins de fabrication de mika#1319 : un jeu d'assertions négatives qui
# tiennent qu'aucun aiguillon de fabrication n'est revenu dans dispatch-lib.sh.
#
# Plage d'origine : lignes 1931–2815 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 890f3f5f. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ============================================================================
# mika#2811 — quand le refus de la porte de dispatch EST la raison du groom
# ============================================================================
#
# § 1.1 du ticket : deux portes jugeaient la même question — « ce ticket est-il
# groomé ? » — sur deux sources différentes. La porte de dispatch lit la BASE
# (preuve `Outcome: PLAN_GROOMED`, mika#1620/#2287) ; l'auto-skip assertionné
# ci-dessus lit le CORPS (les trois callouts). Quand elles divergent, aucune des
# deux ne fait le travail : cinq tâches `already_groomed` de deux minutes sur
# #1381 et #1959 le 2026-10-19, jusqu'à épuiser le budget de re-drive de
# mika#2020. Le correctif de mika#2484 avait déplacé la boucle sans la fermer.
#
# Le moteur transmet désormais la provenance du refus (`ready_label_handler`,
# champ moteur-interne lu ici), et ce dispatch-là seulement désarme l'auto-skip.
# La provenance est TRANSMISE, jamais recalculée (motif mika#2026) : le shell ne
# doit pas devenir un troisième lecteur de l'état de grooming (mika#2158).
#
# LE CROISEMENT CONTRACTUEL DE LA LIGNE 1708 RESTE VRAI, et n'a pas à être
# assoupli : « si la garde refuse pour `already_groomed`, alors le Rust doit
# dire groomé. » Le désarmement ne rend la garde permissive dans AUCUN des cas
# qu'elle refusait pour eux-mêmes ; il lui retire seulement ceux où le moteur a
# déjà constaté l'absence de preuve — et dans ceux-là le Rust ne dit justement
# PAS groomé (c'est `MarkersWithoutProof` ou `ProofUnreadable` qui a produit la
# cause). L'implication tient donc avec un antécédent strictement plus petit.
# C'est écrit ici, et assertionné plus bas par la position du RESULT dans la
# chaîne, pour que le prochain lecteur n'aille pas « corriger » le test de la
# ligne 1726.

# --- (a) le lecteur d'entrée lit le champ PAR LA CONSTANTE -------------------
#
# `|| true` : sous `set -e`, `declare -f` sur une fonction renommée tue la suite
# avant que l'anti-vacuité puisse dire quoi (même raison qu'à la ligne 1821).
PARSE_SRC_2811=$(declare -f _parse_input_json || true)
assert_contains "mika#2811 (anti-vacuité): le lecteur d'entrée existe toujours" \
    '_parse_input_json (' "$PARSE_SRC_2811"
assert_contains "mika#2811: la cause est lue par la constante, via --arg" \
    '--arg f "$GROOM_FORCE_CAUSE_FIELD"' "$PARSE_SRC_2811"
assert_not_contains "mika#2811: le nom du champ n'est pas retapé dans le lecteur" \
    '__internal_groom_cause' "$PARSE_SRC_2811"

# Le site de définition est UNIQUE et en colonne 0 — la forme exacte que le scan
# de parité Rust lit (`canonical_tokens::tests::mika2811_le_nom_du_champ_de_cause_est_synchronise_shell_a_rust`).
# COMPTÉ, pas seulement cherché : une seconde déclaration ailleurs laisserait le
# scan vert tout en donnant deux vérités au même nom.
assert_eq "mika#2811: le nom du champ a exactement un site de définition, en colonne 0" \
    "1" "$(grep -c '^GROOM_FORCE_CAUSE_FIELD="__internal_groom_cause"$' "$DISPATCH_LIB" || true)"
assert_eq "mika#2811: et le littéral n'apparaît nulle part ailleurs dans la lib" \
    "1" "$(grep -oF -- '__internal_groom_cause' "$DISPATCH_LIB" | wc -l | tr -d '[:space:]')"

# --- (b) comportemental, les deux sens : la lecture du champ -----------------
_t2811_cause_probe() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        _parse_input_json <<<"$1"
        printf '%s' "${GROOM_FORCE_CAUSE:-}"
    )
}
assert_eq "mika#2811: une cause posée par le moteur arrive dans GROOM_FORCE_CAUSE" \
    "markers_without_proof" \
    "$(_t2811_cause_probe '{"__mika_task_id":"t-2811","skill":"dev-groom","prompt":"mika#2811","__internal_groom_cause":"markers_without_proof"}' 2>/dev/null)"
assert_eq "mika#2811: la panne de base arrive sous son propre nom" \
    "proof_unreadable" \
    "$(_t2811_cause_probe '{"__mika_task_id":"t-2811","skill":"dev-groom","prompt":"mika#2811","__internal_groom_cause":"proof_unreadable"}' 2>/dev/null)"
# Contrôle négatif : sans la clé, la variable est VIDE — pas « absente lue comme
# une cause ». C'est ce vide qui laisse la porte de mika#2012 armée sur tout
# dispatch que le moteur n'a pas routé ici faute de preuve (flux opérateur
# mika#1173 compris).
assert_eq "mika#2811 (contrôle négatif): sans la clé, aucune cause n'est inventée" \
    "" \
    "$(_t2811_cause_probe '{"__mika_task_id":"t-2811","skill":"dev-groom","prompt":"mika#2811"}' 2>/dev/null)"

# --- (c) la chaîne du gate : ordre, noms, et la note qui ne cite rien --------
#
# Aucun test comportemental ne peut atteindre la chaîne depuis `_set_up_worktree`
# lui-même : elle vit après `gh issue view` et avant `worktree add`, donc hors
# d'un dispatch complet. La région est extraite du fichier réel (même idiome que
# mika#2590 U8f(c), vers la ligne 10474), et l'anti-vacuité passe D'ABORD — un
# scan qui ne regarde rien rend les mêmes octets qu'un arbre propre (mika#2205).
T2811_GATE_CHAIN=$(sed -n '/^        if \[ "\$SKILL" = "dev-groom" \] && \[ -n "\$GROOM_FORCE_CAUSE" \]/,/^        fi$/p' "$DISPATCH_LIB")
assert_eq "mika#2811 (bonne foi): la chaîne du gate a été extraite" \
    "yes" "$(if [ -n "$T2811_GATE_CHAIN" ]; then printf 'yes'; else printf 'no'; fi)"

T2811_SHORTCIRCUIT_LN=$(printf '%s\n' "$T2811_GATE_CHAIN" | grep -n -F -- '-n "$GROOM_FORCE_CAUSE"' | head -1 | cut -d: -f1)
T2811_PLANGATE_LN=$(printf '%s\n' "$T2811_GATE_CHAIN" | grep -n -F -- '_committed_plan_on_branch "$SUB_REPO_DIR"' | head -1 | cut -d: -f1)
T2811_AUTOSKIP_LN=$(printf '%s\n' "$T2811_GATE_CHAIN" | grep -n -F -- '"reason":"already_groomed"' | head -1 | cut -d: -f1)
T2811_ELIF_LN=$(printf '%s\n' "$T2811_GATE_CHAIN" | grep -n -F -- 'elif [ "$SKILL" = "dev-groom" ]; then' | head -1 | cut -d: -f1)
# « Ce groom a-t-il été forcé ? » est à coût nul ; mesurer le plan coûte du git.
assert_eq "mika#2811: le court-circuit de cause est AVANT la mesure git du plan" \
    "yes" "$([ "${T2811_SHORTCIRCUIT_LN:-0}" -gt 0 ] && [ "${T2811_PLANGATE_LN:-0}" -gt 0 ] && [ "$T2811_SHORTCIRCUIT_LN" -lt "$T2811_PLANGATE_LN" ] && printf 'yes' || printf 'non (cause=%s plan=%s)' "${T2811_SHORTCIRCUIT_LN:-?}" "${T2811_PLANGATE_LN:-?}")"
# L'implication de la ligne 1708, dite structurellement : le RESULT
# `already_groomed` ne vit QUE dans le bras atteint sans cause.
assert_eq "mika#2811: le RESULT already_groomed vit APRÈS le elif, donc hors du bras forcé" \
    "yes" "$([ "${T2811_ELIF_LN:-0}" -gt 0 ] && [ "${T2811_AUTOSKIP_LN:-0}" -gt 0 ] && [ "$T2811_ELIF_LN" -lt "$T2811_AUTOSKIP_LN" ] && printf 'yes' || printf 'non (elif=%s skip=%s)' "${T2811_ELIF_LN:-?}" "${T2811_AUTOSKIP_LN:-?}")"

# Le bras de forçage, isolé, pour juger la ligne qu'il émet et elle seule — le
# commentaire voisin NOMME les deux autres signaux (à dessein), donc une
# assertion sur la région entière serait vraie pour la mauvaise raison.
T2811_FORCED_ECHO=$(printf '%s\n' "$T2811_GATE_CHAIN" | sed -n '1,/^        elif \[ "\$SKILL" = "dev-groom" \]; then$/p' | grep -F 'echo "dispatch_gate_' || true)
assert_eq "mika#2811 (bonne foi): le bras forcé émet exactement une ligne de signal" \
    "1" "$(printf '%s\n' "$T2811_FORCED_ECHO" | grep -c . || true)"
# Un TROISIÈME nom : chaque population a le sien, et réutiliser un nom la rend
# insoustractible (mika#2012 U4, qui a créé le second pour cette raison).
assert_contains "mika#2811: le désarmement a son propre nom de signal" \
    'dispatch_gate_groom_forced_no_proof:' "$T2811_FORCED_ECHO"
assert_not_contains "mika#2811: ce n'est pas le signal de refus réutilisé" \
    'dispatch_gate_groom_refused' "$T2811_FORCED_ECHO"
assert_not_contains "mika#2811: ni celui du callout périmé" \
    'dispatch_gate_groom_allowed_stale_callout' "$T2811_FORCED_ECHO"
# `cause=` pour que le grep sépare la population nominale
# (`markers_without_proof`) de la panne de base (`proof_unreadable`, régime
# attendu VIDE) — deux populations sous un nom ne sont pas soustractibles
# (mika#2205).
assert_contains "mika#2811: le signal porte la cause, pas seulement le fait" \
    'cause=${GROOM_FORCE_CAUSE}' "$T2811_FORCED_ECHO"
# mika#2590 U5b : la porte prouvait le grooming par une sous-chaîne, donc le
# texte qui disait « ceci n'est pas une preuve » EST devenu la preuve
# (2026-09-29, offset 651). Le lecteur Rust est ancré depuis ; ceci est la
# défense en profondeur, reportée sur le texte neuf.
assert_not_contains "mika#2811 (mika#2590 U5b): la note de forçage ne cite pas le marqueur" \
    'Outcome: PLAN_GROOMED' "$T2811_FORCED_ECHO"
# Contrôle négatif de l'assertion elle-même : sur une fixture qui cite le jeton,
# le prédicat DOIT accuser.
T2811_MARKER_FIXTURE='            echo "dispatch_gate_groom_forced_no_proof: … no Outcome: PLAN_GROOMED exists in the database …" >&2'
assert_eq "mika#2811 (contrôle négatif): le prédicat accuse une note qui cite le jeton" \
    "1" "$(printf '%s\n' "$T2811_MARKER_FIXTURE" | grep -c -F 'Outcome: PLAN_GROOMED' || true)"

# --- (d) comportemental, les deux sens : la disposition de la chaîne ---------
#
# La chaîne extraite est EXÉCUTÉE, pas paraphrasée : elle est enveloppée dans une
# fonction et sourcée, les quatre appels qu'elle fait sont bouchonnés, et
# `_deliver_callback` rend le RESULT au lieu de l'envoyer. Un futur éditeur du
# gate change donc ce que ce test exécute, et non seulement ce qu'il lit.
_t2811_gate_probe() {
    local wrapper
    wrapper=$(mktemp)
    {
        printf '_t2811_gate_arm() {\n'
        printf '%s\n' "$T2811_GATE_CHAIN"
        printf '}\n'
    } > "$wrapper"
    (
        _committed_plan_on_branch() { printf '%s\n' "docs/plans/2026-08-14-001-ancien-plan.md"; }
        _plan_provenance() { printf 'committed on the branch'; }
        _extract_plan_path() { printf 'docs/plans/2026-08-14-001-ancien-plan.md'; }
        _deliver_callback() { printf '%s' "$RESULT"; }
        SKILL="dev-groom"
        REPO="mika"
        ISSUE_NUM="2811"
        BRANCH="fix/2811/probe"
        SUB_REPO_DIR="/nonexistent"
        ISSUE_BODY='> - **Plan:** `docs/plans/2026-08-14-001-ancien-plan.md`'
        RESULT=""
        GROOM_FORCE_CAUSE="$1"
        # shellcheck disable=SC1090
        source "$wrapper"
        _t2811_gate_arm
    )
    rm -f "$wrapper"
}
# Sens 1 — cause posée : la chaîne ne produit AUCUN `already_groomed`. C'est la
# boucle du ticket, fermée : le dispatch continue vers une passe réelle qui
# frappera la preuve, au lieu de rendre un skip de deux minutes.
T2811_FORCED_OUT=$(_t2811_gate_probe "markers_without_proof" 2>/dev/null)
T2811_FORCED_ERR=$(_t2811_gate_probe "markers_without_proof" 2>&1 >/dev/null)
assert_eq "mika#2811 (comportemental): cause posée ⇒ aucun RESULT already_groomed" \
    "0" "$(printf '%s\n' "$T2811_FORCED_OUT" | grep -c -F 'already_groomed' || true)"
assert_contains "mika#2811 (comportemental): cause posée ⇒ le désarmement est DIT, sur stderr" \
    'dispatch_gate_groom_forced_no_proof: repo=mika issue=2811' "$T2811_FORCED_ERR"
assert_contains "mika#2811 (comportemental): et la ligne nomme la cause en vigueur" \
    'cause=markers_without_proof' "$T2811_FORCED_ERR"
# Sens 2 — cause absente : la porte de mika#2012 est INCHANGÉE. Si elle ne
# refusait plus ici, le désarmement serait devenu une désactivation.
T2811_PLAIN_OUT=$(_t2811_gate_probe "" 2>/dev/null)
assert_eq "mika#2811 (contrôle négatif): cause absente ⇒ la porte de mika#2012 refuse comme avant" \
    "auto_skipped|already_groomed" \
    "$(printf '%s' "$T2811_PLAIN_OUT" | jq -r '[.status, .reason] | join("|")' 2>/dev/null || printf 'illisible')"
assert_eq "mika#2811 (contrôle négatif): et le RESULT nomme toujours le plan mesuré" \
    "docs/plans/2026-08-14-001-ancien-plan.md" \
    "$(printf '%s' "$T2811_PLAIN_OUT" | jq -r '.plan' 2>/dev/null || printf 'illisible')"
assert_contains "mika#2811 (contrôle négatif): et le geste de remède est intact" \
    'remove the plan from the branch AND the grooming callouts from the issue body' \
    "$T2811_PLAIN_OUT"

# --- (e) le texte du remède : l'ORDRE des deux routes est le correctif -------
#
# Ce bras ne se lit plus que sans provenance de refus — donc sur un groom
# demandé par un tour LLM (mika#1173). Pour tout ce qui passe par la boucle, le
# retrait à la main du plan et des callouts est devenu du travail opérateur pour
# rien : y repasser suffit. Le geste reste nommé, en SECOND, parce qu'il marche
# toujours et qu'il est la seule issue hors boucle. C'est pourquoi l'assertion
# porte sur l'ordre et pas seulement sur la présence.
T2811_NOTE=$(printf '%s' "$T2811_PLAIN_OUT" | jq -r '.note' 2>/dev/null || printf 'illisible')
assert_contains "mika#2811: la note nomme le retour par la boucle" \
    'Cheapest route back: hand the ticket to the loop' "$T2811_NOTE"
assert_contains "mika#2811: et dit que la passe forcée frappe la preuve, plan en place" \
    'mints the proof with the plan left exactly where it is' "$T2811_NOTE"
T2811_NOTE_HEAD="${T2811_NOTE%%remove the plan from the branch AND*}"
assert_eq "mika#2811: la route de boucle est nommée AVANT le geste à la main" \
    "1" "$(printf '%s' "$T2811_NOTE_HEAD" | grep -c -F 'Cheapest route back' || true)"

# --- Test: Auto-rescue scaffold exclusion (mika#1288) ---

echo ""
echo "Test: Auto-rescue excludes scaffold files (mika#1288)"
echo "------------------------------------------------------"

# Test A: Mixed content — scaffold excluded, pilot content staged
test_auto_rescue_excludes_scaffold_files() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    # Setup: create a git repo simulating a dirty worktree
    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # 1. Tracked file with a pilot modification
    echo "original" > "$test_dir/tracked.rs"
    git -C "$test_dir" add tracked.rs
    git -C "$test_dir" commit -m "add tracked" -q
    echo "modified by pilot" > "$test_dir/tracked.rs"

    # 2. Scaffold file (untracked) — must NOT be staged
    mkdir -p "$test_dir/.claude/commands"
    echo "# scaffold" > "$test_dir/.claude/commands/mika-groom-ticket.md"

    # 3. Pilot-authored new file (untracked) — must be staged
    mkdir -p "$test_dir/src"
    echo "fn main() {}" > "$test_dir/src/new_feature.rs"

    # Exercise: run the pathspec-filtered git add
    git -C "$test_dir" add -A -- ':!.claude/commands/' 2>/dev/null

    # Assert: check what was staged
    local staged
    staged=$(git -C "$test_dir" diff --cached --name-only)

    # Scaffold file must NOT appear in staged files
    if grep -q -- '.claude/commands/' <<<"$staged"; then
        echo "FAIL: scaffold file was staged"
        return 1
    fi

    # Pilot-authored new file must be staged
    if ! grep -q -- 'src/new_feature.rs' <<<"$staged"; then
        echo "FAIL: pilot-authored new file was not staged"
        return 1
    fi

    # Modified tracked file must be staged
    if ! grep -q -- 'tracked.rs' <<<"$staged"; then
        echo "FAIL: modified tracked file was not staged"
        return 1
    fi

    echo "PASS"
}

RESULT_A=$(test_auto_rescue_excludes_scaffold_files 2>/dev/null)
if [ "$RESULT_A" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Mixed content: scaffold excluded, pilot content staged"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Mixed content: $RESULT_A"
fi

# Test B: Scaffold-only worktree — empty index guard
test_auto_rescue_empty_index_guard() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    # Setup: repo where the ONLY dirty content is scaffold files
    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    mkdir -p "$test_dir/.claude/commands"
    echo "# scaffold only" > "$test_dir/.claude/commands/mika-groom-ticket.md"
    echo "# another scaffold" > "$test_dir/.claude/commands/mika.md"

    # Exercise: pathspec-filtered git add
    git -C "$test_dir" add -A -- ':!.claude/commands/' 2>/dev/null

    # Assert: index should be empty (diff --cached --quiet exits 0)
    if ! git -C "$test_dir" diff --cached --quiet 2>/dev/null; then
        echo "FAIL: index is not empty despite only scaffold files being dirty"
        return 1
    fi

    echo "PASS"
}

# Test C (mika#1419): claude-pilot.json scaffold exclusion — relay-config
# file is cp'd from $PLATFORM_DIR by _set_up_worktree at line 489 and is NOT
# pilot-authored content. The rescue commit must exclude it via the same
# pathspec mechanism mika#1288 uses for .claude/commands/. Without this
# exclusion, PR #1348's intentional deletion (mika#1193 Phase C) is silently
# re-introduced on every WIP-rescue flow — the ping-pong on the file's git
# log is the founding incident for mika#1419.
test_auto_rescue_excludes_claude_pilot_json() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # 1. claude-pilot.json scaffold file (untracked, cp'd from meta-repo) —
    #    must NOT be staged by the rescue commit.
    mkdir -p "$test_dir/.claude"
    echo '{"command":"mika","args":["--agent","mika-relay","ask"]}' > "$test_dir/.claude/claude-pilot.json"

    # 2. .claude/commands/ scaffold file — must NOT be staged (mika#1288).
    mkdir -p "$test_dir/.claude/commands"
    echo "# scaffold" > "$test_dir/.claude/commands/mika-groom-ticket.md"

    # 3. Pilot-authored new file — must be staged.
    mkdir -p "$test_dir/src"
    echo "fn main() {}" > "$test_dir/src/new_feature.rs"

    # Exercise: the mika#1419 pathspec exclusion.
    git -C "$test_dir" add -A -- ':!.claude/commands/' ':!.claude/claude-pilot.json' 2>/dev/null

    local staged
    staged=$(git -C "$test_dir" diff --cached --name-only)

    # claude-pilot.json must NOT appear in staged files (mika#1419 regression).
    if grep -q -- '.claude/claude-pilot.json' <<<"$staged"; then
        echo "FAIL: .claude/claude-pilot.json was staged (mika#1419 ping-pong reintroduced)"
        return 1
    fi

    # .claude/commands/ scaffold must NOT appear (mika#1288 regression).
    if grep -q -- '.claude/commands/' <<<"$staged"; then
        echo "FAIL: .claude/commands/ scaffold was staged (mika#1288 regression)"
        return 1
    fi

    # Pilot-authored file MUST be staged.
    if ! grep -q -- 'src/new_feature.rs' <<<"$staged"; then
        echo "FAIL: pilot-authored file was not staged"
        return 1
    fi

    echo "PASS"
}

RESULT_C=$(test_auto_rescue_excludes_claude_pilot_json 2>/dev/null)
if [ "$RESULT_C" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ claude-pilot.json scaffold excluded (mika#1419 ping-pong regression)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ claude-pilot.json scaffold exclusion: $RESULT_C"
fi

# Test D (mika#1419): scaffold-only worktree empty-index guard includes
# claude-pilot.json. Verifies the empty-index path correctly skips the rescue
# commit when both scaffold types are the only dirty content.
test_auto_rescue_empty_index_guard_with_claude_pilot_json() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # Only scaffold dirt: both commands/ AND claude-pilot.json — no pilot content.
    mkdir -p "$test_dir/.claude/commands"
    echo "# scaffold" > "$test_dir/.claude/commands/mika.md"
    echo '{"command":"mika","args":["--agent","mika-relay","ask"]}' > "$test_dir/.claude/claude-pilot.json"

    git -C "$test_dir" add -A -- ':!.claude/commands/' ':!.claude/claude-pilot.json' 2>/dev/null

    if ! git -C "$test_dir" diff --cached --quiet 2>/dev/null; then
        echo "FAIL: index is not empty despite only scaffold-class files being dirty"
        return 1
    fi

    echo "PASS"
}

RESULT_D=$(test_auto_rescue_empty_index_guard_with_claude_pilot_json 2>/dev/null)
if [ "$RESULT_D" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Empty-index guard covers both scaffold classes (mika#1288 + mika#1419)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Empty-index guard with both scaffolds: $RESULT_D"
fi

# Test E (mika#1552): settings.local.json + .claude/*.local.* exclusion —
# permission allowlist file written by claude-pilot (or worktree setup) MUST NOT
# ship as pilot-authored content. cm#5 dispatch (2026-06-16) produced PR #16
# whose ONLY changed file was a 143-line .claude/settings.local.json leak,
# zero adapter implementation — the founding incident.
test_auto_rescue_excludes_settings_local_json() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # 1. settings.local.json — must NOT be staged (mika#1552 explicit case).
    mkdir -p "$test_dir/.claude"
    echo '{"permissions": ["Bash(ls)"]}' > "$test_dir/.claude/settings.local.json"

    # 2. hooks.local.json — must NOT be staged (mika#1552 wildcard case via .local.*).
    echo '{"PreToolUse": []}' > "$test_dir/.claude/hooks.local.json"

    # 3. Pilot-authored new file — must be staged.
    mkdir -p "$test_dir/src"
    echo "fn main() {}" > "$test_dir/src/new_feature.rs"

    # Exercise: the mika#1552 extended pathspec exclusion.
    git -C "$test_dir" add -A -- \
        ':!.claude/commands/' \
        ':!.claude/claude-pilot.json' \
        ':!.claude/settings.local.json' \
        ':!.claude/*.local.*' 2>/dev/null

    local staged
    staged=$(git -C "$test_dir" diff --cached --name-only)

    # settings.local.json must NOT appear (mika#1552 explicit exclusion).
    if grep -q -- '.claude/settings.local.json' <<<"$staged"; then
        echo "FAIL: .claude/settings.local.json was staged (mika#1552 leak)"
        return 1
    fi

    # hooks.local.json must NOT appear (mika#1552 wildcard exclusion).
    if grep -q -- '.claude/hooks.local.json' <<<"$staged"; then
        echo "FAIL: .claude/hooks.local.json was staged (mika#1552 wildcard gap)"
        return 1
    fi

    # Pilot-authored file MUST be staged.
    if ! grep -q -- 'src/new_feature.rs' <<<"$staged"; then
        echo "FAIL: pilot-authored file was not staged"
        return 1
    fi

    echo "PASS"
}

RESULT_E=$(test_auto_rescue_excludes_settings_local_json 2>/dev/null)
if [ "$RESULT_E" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ settings.local.json + *.local.* excluded (mika#1552 founding case)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ settings.local.json exclusion: $RESULT_E"
fi

RESULT_B=$(test_auto_rescue_empty_index_guard 2>/dev/null)
if [ "$RESULT_B" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Scaffold-only worktree: empty index correctly detected"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Scaffold-only worktree: $RESULT_B"
fi

# Test F (mika#2760): les brouillons de corps de ticket à la racine du worktree.
#
# Deux fichiers sont entrés sur `main` par ces sites de récupération même, et
# leurs messages de commit le disent : `b99c6219` (`wip(mika#2317)`) a commité
# `followup-issue-body.md`, `50e98793` (`wip(mika#2309)`) a commité
# `followup-issue.md`. Un corps de ticket écrit pour `gh issue create
# --body-file` n'est pas du contenu de pilote : personne ne le lit, et chaque
# worktree l'embarque depuis main une fois qu'il y est.
#
# CE CAS EXERCE LE TABLEAU RÉEL, jamais un littéral recopié. Les cas A, C et E
# ci-dessus codent leur pathspec en dur : ils testent leur propre copie et
# resteraient verts si `dispatch-lib.sh` perdait l'entrée. Ici la définition est
# chargée depuis le fichier — une ligne unique, auto-contenue, extraite par
# ancrage en colonne zéro (le même ancrage que `RESCUE_EXCLUDE_DEF` plus bas).
# Le fichier entier n'est pas sourcé : cela exécuterait son setup top-level.
_eval_real_rescue_pathspec() {
    local def
    def=$(grep -E '^RESCUE_EXCLUDE_PATHSPEC=\(' "$DISPATCH_LIB" | head -1)
    if [ -z "$def" ]; then
        echo "FAIL: no \`RESCUE_EXCLUDE_PATHSPEC=(\` line found in dispatch-lib.sh"
        return 1
    fi
    eval "$def"
}

# Pose le dépôt fixture partagé par F et G : trois brouillons à la racine, et
# trois chemins qui sont du contenu de pilote et doivent rester indexés.
_mk_pilot_draft_fixture() {
    local dir="$1"
    git -C "$dir" init -q
    git -C "$dir" commit --allow-empty -m "initial" -q

    # Les trois formes de brouillon — ne doivent PAS être indexées.
    echo "## Constat" > "$dir/followup-issue.md"
    echo "## Constat" > "$dir/followup-issue-body.md"
    echo "# brief seconde passe" > "$dir/groom-brief-mika-999-pass2.md"

    # Du contenu de pilote — doit être indexé.
    mkdir -p "$dir/src" "$dir/docs/plans" "$dir/sub"
    echo "fn main() {}" > "$dir/src/new_feature.rs"
    echo "# plan" > "$dir/docs/plans/2026-10-18-001-x-plan.md"
    # L'exclusion est ancrée à la racine (d'où `:(exclude,glob)` et non `:!`) :
    # un homonyme sous un répertoire est du contenu, et le reste.
    echo "# nested" > "$dir/sub/followup-issue.md"
}

test_auto_rescue_excludes_pilot_draft_bodies() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    _mk_pilot_draft_fixture "$test_dir"
    _eval_real_rescue_pathspec || return 1

    git -C "$test_dir" add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}" 2>/dev/null

    local staged
    staged=$(git -C "$test_dir" diff --cached --name-only)

    local draft
    for draft in followup-issue.md followup-issue-body.md groom-brief-mika-999-pass2.md; do
        if grep -qxF -- "$draft" <<<"$staged"; then
            echo "FAIL: $draft was staged (mika#2760)"
            return 1
        fi
    done

    local kept
    for kept in src/new_feature.rs docs/plans/2026-10-18-001-x-plan.md sub/followup-issue.md; do
        if ! grep -qxF -- "$kept" <<<"$staged"; then
            echo "FAIL: pilot content $kept was not staged"
            return 1
        fi
    done

    echo "PASS"
}

# `|| true` : sans lui, `set -e` tue la suite sur l'assignation dès que la
# fonction rend non-zéro, et la branche ✗ — avec son diagnostic — n'est jamais
# atteinte. La branche FAIL compte, et la fin de suite sort 1.
RESULT_F=$(test_auto_rescue_excludes_pilot_draft_bodies 2>/dev/null) || true
if [ "$RESULT_F" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Root draft bodies excluded, nested namesake kept (mika#2760)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Root draft body exclusion: $RESULT_F"
fi

# Test G (mika#2760): le contrôle négatif, qui est ce qui donne sa valeur à F.
# Le même fixture, re-stagé avec le tableau PRIVÉ des deux nouvelles entrées :
# les trois brouillons doivent alors être indexés. Sans ce cas, F passerait
# aussi si l'exclusion venait d'un `.gitignore`, d'un `core.excludesFile` de la
# machine, ou de rien du tout — et la suite dirait vert sur un tableau vide.
test_auto_rescue_draft_exclusion_is_not_accidental() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    _mk_pilot_draft_fixture "$test_dir"
    _eval_real_rescue_pathspec || return 1

    local filtered=() entry
    for entry in "${RESCUE_EXCLUDE_PATHSPEC[@]}"; do
        case "$entry" in
            *followup-issue*|*groom-brief-*) continue ;;
        esac
        filtered+=("$entry")
    done

    # Le filtre doit avoir retiré exactement les deux entrées de mika#2760 :
    # sinon ce cas passerait sur un tableau qui ne les a jamais portées.
    local removed=$(( ${#RESCUE_EXCLUDE_PATHSPEC[@]} - ${#filtered[@]} ))
    if [ "$removed" -ne 2 ]; then
        echo "FAIL: expected 2 draft-body entries in RESCUE_EXCLUDE_PATHSPEC, removed $removed"
        return 1
    fi

    git -C "$test_dir" add -A -- "${filtered[@]}" 2>/dev/null

    local staged
    staged=$(git -C "$test_dir" diff --cached --name-only)

    local draft
    for draft in followup-issue.md followup-issue-body.md groom-brief-mika-999-pass2.md; do
        if ! grep -qxF -- "$draft" <<<"$staged"; then
            echo "FAIL: $draft was NOT staged without its exclusion — F passes for another reason"
            return 1
        fi
    done

    echo "PASS"
}

RESULT_G=$(test_auto_rescue_draft_exclusion_is_not_accidental 2>/dev/null) || true
if [ "$RESULT_G" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Without the two entries the drafts ARE staged (mika#2760 negative control)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Draft exclusion negative control: $RESULT_G"
fi

# Test C: Structural — dispatch-lib uses pathspec exclusion (not bare git add -A)
# mika#2031 moved the rescue body out of _post_flight_recovery into
# _rescue_dirty_worktree(), so the extraction anchors on the function rather than
# on the inline `Unit 1` comment. Same assertions, same block — new address.
RESCUE_BLOCK=$(sed -n '/^_rescue_dirty_worktree() {/,/^}$/p' "$DISPATCH_LIB")
# mika#2348 D2 hoisted the literal exclusions into the shared
# RESCUE_EXCLUDE_PATHSPEC array (one list, every add site). The intent is
# unchanged — scaffold paths never land in a rescue commit — so the assertion
# follows the array: the add line must expand it, and the definition must still
# carry the scaffold exclusions.
RESCUE_EXCLUDE_DEF=$(grep -E '^RESCUE_EXCLUDE_PATHSPEC=\(' "$DISPATCH_LIB" | head -1)
assert_contains "Rescue git add expands RESCUE_EXCLUDE_PATHSPEC (mika#2348 D2)" \
    'add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}"' "$RESCUE_BLOCK"
assert_contains "RESCUE_EXCLUDE_PATHSPEC excludes :!.claude/commands/" \
    ":!.claude/commands/" "$RESCUE_EXCLUDE_DEF"
assert_contains "RESCUE_EXCLUDE_PATHSPEC excludes :!.claude/claude-pilot.json" \
    ":!.claude/claude-pilot.json" "$RESCUE_EXCLUDE_DEF"
# mika#2760 : la FORME, que les cas F/G seuls ne distinguent pas. `:!motif*.md`
# et `:(exclude,glob)motif*.md` excluent tous deux le fichier racine ; seul
# `sub/followup-issue.md` les sépare, parce que sans la magie `glob` git applique
# `fnmatch` sans `FNM_PATHNAME` et le `*` traverse les `/`. C'est ce que F
# mesure, et ce que ces deux lignes rendent lisible dans un nom de test.
assert_contains "RESCUE_EXCLUDE_PATHSPEC excludes :(exclude,glob)followup-issue*.md (mika#2760)" \
    ":(exclude,glob)followup-issue*.md" "$RESCUE_EXCLUDE_DEF"
assert_contains "RESCUE_EXCLUDE_PATHSPEC excludes :(exclude,glob)groom-brief-*.md (mika#2760)" \
    ":(exclude,glob)groom-brief-*.md" "$RESCUE_EXCLUDE_DEF"
assert_contains "Rescue has empty-index guard (diff --cached --quiet)" "diff --cached --quiet" "$RESCUE_BLOCK"
assert_contains "Rescue uses RESCUED_FILES for message" 'RESCUED_FILES' "$RESCUE_BLOCK"

# --- Test: Auto-rescue hook failure handling (mika#1296) ---

echo ""
echo "Test: Auto-rescue checks commit exit code (mika#1296)"
echo "------------------------------------------------------"

# Test A — structural assertion (code-shape): RESCUED_DIRTY_WORKTREE=1 appears
# only inside the commit-success path, not as a fallthrough after the if/elif/else.
# In the rescue block (between "mika-rescue-commit-err" and the closing fi chain),
# every RESCUED_DIRTY_WORKTREE=1 must be preceded by an "rm -f" (cleanup on success)
# within the same branch — not as a standalone unconditional line.
RESCUE_BLOCK_1296=$(sed -n '/mika-rescue-commit-err/,/^[[:space:]]*fi$/p' "$DISPATCH_LIB")

# Count RESCUED_DIRTY_WORKTREE=1 in non-comment lines of the rescue block
RESCUED_SET_COUNT=$(printf '%s\n' "$RESCUE_BLOCK_1296" | grep -v '^\s*#' | grep -c 'RESCUED_DIRTY_WORKTREE=1' || true)
# There should be exactly 2: one for first-try success, one for retry success
assert_eq "RESCUED_DIRTY_WORKTREE=1 appears exactly twice in rescue block (success paths only)" "2" "$RESCUED_SET_COUNT"

# The else branch (unknown hook failure) must NOT set RESCUED_DIRTY_WORKTREE=1
# Verify by checking that no RESCUED_DIRTY_WORKTREE=1 appears after "non-rustfmt" marker
AFTER_NON_RUSTFMT=$(printf '%s\n' "$RESCUE_BLOCK_1296" | sed -n '/non-rustfmt/,/fi/p')
NON_RUSTFMT_RESCUED=$(printf '%s\n' "$AFTER_NON_RUSTFMT" | grep -v '^\s*#' | grep -c 'RESCUED_DIRTY_WORKTREE=1' || true)
assert_eq "Unknown hook failure branch does NOT set RESCUED_DIRTY_WORKTREE" "0" "$NON_RUSTFMT_RESCUED"

# Test B — live invariant (git-repo exercise): pre-commit hook rejects commit,
# verify RESCUED_DIRTY_WORKTREE is NOT set and RESULT contains PIPELINE FAILURE.
test_rescue_hook_failure_invariant() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    # Setup: create a git repo with a pre-commit hook that rejects with "rust-fmt"
    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # Create a pre-commit hook that always fails with rust-fmt in stderr
    mkdir -p "$test_dir/.git/hooks"
    cat > "$test_dir/.git/hooks/pre-commit" << 'HOOK'
#!/bin/bash
echo "error: rust-fmt check failed" >&2
exit 1
HOOK
    chmod +x "$test_dir/.git/hooks/pre-commit"

    # Create a dirty tracked file
    echo "fn main() {}" > "$test_dir/main.rs"
    git -C "$test_dir" add main.rs
    git -C "$test_dir" -c core.hooksPath="$test_dir/.git/hooks" commit --no-verify -m "add file" -q
    echo "fn main() { println!(\"dirty\"); }" > "$test_dir/main.rs"
    git -C "$test_dir" add main.rs

    # Create a stub cargo fmt on PATH that succeeds (simulates formatting)
    local stub_bin
    stub_bin=$(mktemp -d)
    cat > "$stub_bin/cargo" << 'STUB'
#!/bin/bash
# Stub cargo that does nothing for "fmt" subcommand
exit 0
STUB
    chmod +x "$stub_bin/cargo"

    # Exercise: simulate the rescue commit logic
    local WORKTREE_DIR="$test_dir"
    local REPO="mika" ISSUE_NUM="1296" SESSION_ID="test-session"
    local RESCUED_DIRTY_WORKTREE=0
    local RESULT=""
    # mika#1341: mirror the production scratch path (mktemp), not the old "$WORKTREE_DIR/.git/"
    # location. This harness uses a non-linked `git init` repo (where .git is a directory), so
    # either path would open here — but the mirror should track production shape for fidelity.
    local RESCUE_COMMIT_ERR="$(mktemp)"
    local CARGO_FMT_ERR="" RESCUE_ERR_CONTENT=""

    # First attempt — will fail due to pre-commit hook
    if git -C "$WORKTREE_DIR" commit -m "wip(test): rescue" 2>"$RESCUE_COMMIT_ERR"; then
        rm -f "$RESCUE_COMMIT_ERR"
        RESCUED_DIRTY_WORKTREE=1
    elif grep -q "rust-fmt\|cargo fmt\|rustfmt" "$RESCUE_COMMIT_ERR" 2>/dev/null; then
        CARGO_FMT_ERR=""
        CARGO_FMT_ERR=$( (cd "$WORKTREE_DIR" && PATH="$stub_bin:$PATH" cargo fmt --all) 2>&1 ) || true
        git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' 2>/dev/null

        # Retry — will also fail (hook still rejects)
        if git -C "$WORKTREE_DIR" commit -m "wip(test): rescue retry" 2>"$RESCUE_COMMIT_ERR"; then
            rm -f "$RESCUE_COMMIT_ERR"
            RESCUED_DIRTY_WORKTREE=1
        else
            RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -20)
            RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook after cargo-fmt retry.
cargo fmt stderr: ${CARGO_FMT_ERR:-<empty>}
Hook output: ${RESCUE_ERR_CONTENT}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}

${RESULT}"
            rm -f "$RESCUE_COMMIT_ERR"
        fi
    else
        RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -20)
        RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook (non-rustfmt).
Hook output: ${RESCUE_ERR_CONTENT}

${RESULT}"
        rm -f "$RESCUE_COMMIT_ERR"
    fi

    # Assertions
    local failures=""
    if [ "$RESCUED_DIRTY_WORKTREE" != "0" ]; then
        failures="${failures}RESCUED_DIRTY_WORKTREE should be 0 but is $RESCUED_DIRTY_WORKTREE; "
    fi
    if ! grep -qF -- "PIPELINE FAILURE" <<<"$RESULT"; then
        failures="${failures}RESULT missing PIPELINE FAILURE; "
    fi
    if ! grep -qF -- "cargo fmt stderr:" <<<"$RESULT"; then
        failures="${failures}RESULT missing cargo fmt diagnostic; "
    fi
    if [ -f "$RESCUE_COMMIT_ERR" ]; then
        failures="${failures}scratch file not cleaned up; "
    fi

    rm -rf "$stub_bin"
    if [ -z "$failures" ]; then
        echo "PASS"
    else
        echo "FAIL: $failures"
    fi
}

RESULT_HOOK=$(test_rescue_hook_failure_invariant 2>/dev/null)
if [ "$RESULT_HOOK" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Hook failure: RESCUED_DIRTY_WORKTREE=0, PIPELINE FAILURE in RESULT, diagnostics present, scratch cleaned"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Hook failure invariant: $RESULT_HOOK"
fi

# Test C — structural assertion (mika#1341): the rescue scratch path MUST NOT live
# under "$WORKTREE_DIR/.git/". In a linked worktree (every autonomous dev-pilot run)
# ".git" is a FILE, so a redirect into "$WORKTREE_DIR/.git/<name>" fails (ENOTDIR),
# the rescue `git commit` never runs, and the loop wedges. The active assignment must
# use mktemp (off the working tree, valid in linked + non-linked checkouts).
# This assertion FAILS against the pre-mika#1341 ".git/"-scratch code.
RESCUE_ASSIGN=$(grep 'RESCUE_COMMIT_ERR=' "$DISPATCH_LIB" | grep -v '^\s*#')
ACTIVE_ASSIGN=$(printf '%s\n' "$RESCUE_ASSIGN" | grep 'mktemp' || true)
GIT_FILE_ASSIGN=$(printf '%s\n' "$RESCUE_ASSIGN" | grep 'WORKTREE_DIR/.git/mika-rescue-commit-err' || true)
if [ -n "$ACTIVE_ASSIGN" ] && [ -z "$GIT_FILE_ASSIGN" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Rescue scratch path uses mktemp, not \$WORKTREE_DIR/.git/ (mika#1341)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Rescue scratch path must use mktemp and must NOT use \$WORKTREE_DIR/.git/ (mika#1341)"
    echo "    active(mktemp): ${ACTIVE_ASSIGN:-<none>}"
    echo "    git-file path:  ${GIT_FILE_ASSIGN:-<none>}"
fi

# Verify no .iterate/rescue-commit-err reference exists (regression guard)
ITERATE_SCRATCH=$(grep -c '.iterate/rescue-commit-err' "$DISPATCH_LIB" || true)
assert_eq "No .iterate/rescue-commit-err reference in dispatch-lib.sh" "0" "$ITERATE_SCRATCH"

# Test D — live invariant (mika#1341): the rescue commit must succeed inside a real
# LINKED worktree. Reproduces the root cause as an executable artifact: in a linked
# worktree ".git" is a file, a redirect into "<wt>/.git/<name>" fails (ENOTDIR), but
# a commit capturing into an mktemp path lands and advances HEAD.
#
# This is a standalone root-cause PROOF — it reimplements the rescue commit with its own
# mktemp scratch and does NOT source dispatch-lib.sh, so it passes regardless of the
# production code. The source-coupled regression guard is Test C above (which asserts the
# production RESCUE_COMMIT_ERR assignment uses mktemp, not "$WORKTREE_DIR/.git/"). Do not
# delete Test C assuming Test D covers the regression — Test D would still pass on buggy code.
test_rescue_linked_worktree_invariant() {
    local base_dir wt_dir
    base_dir=$(mktemp -d)
    wt_dir="${base_dir}-wt"
    trap "git -C '$base_dir' worktree remove --force '$wt_dir' 2>/dev/null; rm -rf '$base_dir' '$wt_dir'" RETURN

    # Base repo with an initial commit + an identity so commits work in CI.
    git -C "$base_dir" init -q
    git -C "$base_dir" config user.email "test@mika.local"
    git -C "$base_dir" config user.name "mika test"
    git -C "$base_dir" commit --allow-empty -m "initial" -q

    # Create a LINKED worktree on a new branch.
    git -C "$base_dir" worktree add -q -b rescue-test "$wt_dir" 2>/dev/null

    local failures=""

    # (1) In a linked worktree, .git is a FILE, not a directory.
    if [ ! -f "$wt_dir/.git" ]; then
        failures="${failures}linked worktree .git is not a file; "
    fi

    # (2) The OLD path construction fails to open (documents the ENOTDIR root cause).
    if ( echo probe > "$wt_dir/.git/mika-rescue-commit-err" ) 2>/dev/null; then
        failures="${failures}redirect into <wt>/.git/<name> unexpectedly succeeded; "
    fi

    # (3) The FIXED approach: capture into an mktemp path; the rescue commit lands.
    local pre_head post_head scratch
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)
    echo "pilot wrote this but never committed" > "$wt_dir/impl.txt"
    git -C "$wt_dir" add -A -- ':!.claude/commands/' 2>/dev/null
    scratch="$(mktemp)"
    if git -C "$wt_dir" commit -m "wip(mika#1341): rescue in linked worktree" > "$scratch" 2>&1; then
        post_head=$(git -C "$wt_dir" rev-parse HEAD)
        [ "$pre_head" != "$post_head" ] || failures="${failures}HEAD did not advance after rescue commit; "
        git -C "$wt_dir" diff --quiet HEAD -- impl.txt || failures="${failures}impl.txt not committed; "
    else
        failures="${failures}rescue commit failed with mktemp scratch path; "
    fi
    rm -f "$scratch"
    [ ! -f "$scratch" ] || failures="${failures}scratch not cleaned; "

    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_LWT=$(test_rescue_linked_worktree_invariant 2>/dev/null)
if [ "$RESULT_LWT" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Linked-worktree rescue: .git is a file, mktemp scratch lands the commit (mika#1341)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Linked-worktree rescue invariant: $RESULT_LWT"
fi

# --- Test 11: Idempotency-bypass-architect fabrication brake retired (mika#1327) ---

echo ""
echo "Test 11: Idempotency-bypass-architect fabrication brake retired (mika#1327)"
echo "---------------------------------------------------------------------------"

# The mika#1322 brake (fabrication-string grep on session log) was retired in
# mika#1327. Per Vincent's ticket comment IC_kwDORWsgGM8AAAABEDBqdw, the brake
# was a duplicate fabrication-detection mechanism alongside state-grounded
# checks (HEAD-unchanged at line 451, plan-missing at line 621, iterate-loop
# ESCALATE inside _escalate_groom) and post-cpp#20 had become dead code.
#
# These assertions are regression guards against re-introducing the brake.

DISPATCH_LIB_CONTENT=$(cat "$DISPATCH_LIB")

assert_not_contains "Brake comment header (mika#1319 + idempotency-bypass-architect) is absent" 'mika#1319.*idempotency-bypass-architect' "$DISPATCH_LIB_CONTENT"
assert_not_contains "Brake comment header (idempotency-bypass-architect fabrication in dev-groom) is absent" 'idempotency-bypass-architect fabrication in dev-groom' "$DISPATCH_LIB_CONTENT"
assert_not_contains "Fabrication needle (Architect convergence pending via dispatch-lib iterate loop) is absent" 'Architect convergence pending via dispatch-lib iterate loop' "$DISPATCH_LIB_CONTENT"
assert_not_contains "FABRICATION_NEEDLE variable assignment is absent" 'FABRICATION_NEEDLE=' "$DISPATCH_LIB_CONTENT"
assert_not_contains "PIPELINE FAILURE marker with idempotency-bypass-architect sub-type is absent" 'PIPELINE FAILURE:.*idempotency-bypass-architect' "$DISPATCH_LIB_CONTENT"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
