#!/bin/bash
# La position du marqueur et la parité des deux lecteurs — extrait de test-dispatch-lib.sh par mika#3178, vol 4.
# Le producteur qui n'écrit plus le marqueur hors position, et la note d'un refus
# qui ne le cite plus (mika#2590) ; UN corpus, DEUX lecteurs, UNE parité
# (mika#2194 R4) et le scan anti-copie qui refuse qu'un lecteur se recopie
# (mika#2194 R5).
#
# Plage d'origine : lignes 11205–11402 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 3849d757. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ===========================================================================
# mika#2590 — le producteur n'écrit plus le marqueur hors position, et la note
# d'un refus ne le cite plus.
# ===========================================================================
#
# Le lecteur Rust passe à une lecture ANCRÉE sur la ligne. Appliqué seul, cet
# ancrage INTRODUIT un faux négatif (KTD3) : le producteur écrivait
# `sed 's/Outcome: .*/Outcome: PLAN_GROOMED/'`, **non ancré**, donc sur un RESULT
# portant `Outcome: ` en milieu de ligne il posait le marqueur hors position — et
# le filet qui suivait (`grep -qF`, non ancré lui aussi) le voyait et n'ajoutait
# pas la ligne canonique. Un groom RÉELLEMENT convergé aurait alors été refusé.
# Les deux moitiés voyagent dans le même binaire (`skills/bundled/` est une
# projection du binaire, mika#2340), donc elles ne peuvent pas être servies
# séparément — c'est ce qui rend KTD3 sûr.

# --- U8f(a), contrôle négatif PORTEUR : la forme d'avant produit le piège ----
#
# Reproduction explicite du couple sed+grep retiré par U5a, sur le RESULT piège.
# Sans cette moitié, « le bras écrit une ligne ancrée » serait indistinguable de
# « le RESULT d'entrée n'a jamais porté de piège ».
T2590_TRAP_IN="claude-pilot completed (status: success). Outcome: PLAN_COMMITTED — plan posé.

Session: sess-2590"
T2590_OLD_FORM=$(printf '%s' "$T2590_TRAP_IN" | sed 's/Outcome: .*/Outcome: PLAN_GROOMED/')
assert_eq "mika#2590 (contrôle négatif): la forme non ancrée ne produit AUCUNE ligne Outcome ancrée" \
    "0" "$(printf '%s\n' "$T2590_OLD_FORM" | grep -c '^Outcome: PLAN_GROOMED' || true)"
assert_contains "mika#2590 (contrôle négatif): elle pose bien le marqueur, mais en milieu de ligne" \
    "success). Outcome: PLAN_GROOMED" "$T2590_OLD_FORM"

# --- U8f(b), comportemental : _set_outcome_line rend la ligne ancrée ---------
_t2590_groomed_outcome_probe() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        RESULT="claude-pilot completed (status: success). Outcome: PLAN_COMMITTED — plan posé.

Session: sess-2590"
        _set_outcome_line "Outcome: PLAN_GROOMED"
        printf '%s' "$RESULT"
    )
}
T2590_NEW_FORM=$(_t2590_groomed_outcome_probe 2>/dev/null)
assert_eq "mika#2590: _set_outcome_line rend EXACTEMENT une ligne Outcome ancrée" \
    "1" "$(printf '%s\n' "$T2590_NEW_FORM" | grep -c '^Outcome: PLAN_GROOMED' || true)"
assert_contains "mika#2590: et le corps du RESULT est préservé" \
    "Session: sess-2590" "$T2590_NEW_FORM"

# --- U8f(c), scan de source : le bras GROOMED passe par _set_outcome_line ----
#
# C'est cette assertion qui rougit avant U5a. Aucun test comportemental ne peut
# atteindre le bras lui-même : il vit dans `dispatch_claude_pilot`, inatteignable
# hors d'un dispatch complet (worktree, pilote, architecte).
T2590_GROOMED_ARM=$(sed -n '/mika#1394: Architect converged on GROOMED/,/^        elif grep -qE/p' "$DISPATCH_LIB")
assert_eq "mika#2590 (bonne foi): le bras GROOMED a été extrait" \
    "yes" "$(if [ -n "$T2590_GROOMED_ARM" ]; then printf 'yes'; else printf 'no'; fi)"
assert_contains "mika#2590 (U5a): le bras GROOMED pose sa ligne par _set_outcome_line" \
    '_set_outcome_line "$_groomed_outcome"' "$T2590_GROOMED_ARM"
assert_not_contains "mika#2590 (U5a): et le sed NON ANCRÉ a disparu (c'est lui, le faux négatif KTD3)" \
    's/Outcome: .*/Outcome: PLAN_GROOMED/' "$T2590_GROOMED_ARM"
assert_contains "mika#2590 (U5a): le strip des PIPELINE FAILURE périmés est CONSERVÉ" \
    "sed '/^PIPELINE FAILURE:/d'" "$T2590_GROOMED_ARM"

# --- mika#2981, Fire-Disposition : l'assertion auto-nettoyante de la grâce -----
#
# Le livrable de mika#2981 est une porte, et son chemin de succès est « aucune
# violation trouvée ». La grâce D3 — une preuve dont la ligne ancrée ne porte
# AUCUN suffixe de plan reste acceptée — a trois producteurs légitimes et doit se
# vider d'elle-même (rétention de 30 jours de `prune_completed_tasks`). Le seul
# chemin par lequel elle deviendrait la voie NOMINALE au lieu de se vider est un
# retour du producteur à une frappe nue. Ces deux assertions rougissent ce
# jour-là, et elles portent sur le FICHIER entier, pas sur le bras extrait : une
# frappe nue ramenée ailleurs produirait la même population.
#
# Le compteur qui la fait expirer est `groom_proof_unbound_to_plan` dans
# `audit_events` (écrivain unique, `executor.rs`) : c'est la sonde d'AC4, et à 0
# le ticket de suivi retire la grâce et ces deux assertions avec elle.
assert_not_contains "mika#2981 (Fire-Disposition): aucune frappe NUE de PLAN_GROOMED ne subsiste" \
    '_set_outcome_line "Outcome: PLAN_GROOMED"' "$(cat "$DISPATCH_LIB")"
assert_eq "mika#2981 (Fire-Disposition): la frappe SUFFIXÉE est présente une fois" \
    "1" "$(grep -cF '_groomed_outcome="${_groomed_outcome} — ${GROOM_CALLOUT_PLAN}"' "$DISPATCH_LIB" || true)"

# --- U7b (R8/D7) : aucune construction de RESULT auto_skipped ne cite le jeton -
#
# C'est la moitié STRUCTURELLE de R8. La moitié intention (la note reformulée)
# ne tient pas seule au substrat de la boucle
# (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`) : un
# futur éditeur recopie la phrase depuis le commentaire voisin et le défaut
# revient. Le lecteur corrigé le RATTRAPERAIT (l'enveloppe JSON est refusée par
# R2), et c'est exactement pourquoi cette garde vaut le coup — elle empêche la
# régression de se cacher derrière une défense en profondeur.
T2590_AUTOSKIP_LINES=$(grep -n '"status":"auto_skipped"' "$DISPATCH_LIB" || true)
T2590_AUTOSKIP_COUNT=$(printf '%s' "$T2590_AUTOSKIP_LINES" | grep -c . || true)
assert_eq "mika#2590 anti-vacuité: des RESULT auto_skipped existent bien dans dispatch-lib.sh" \
    "yes" "$([ "${T2590_AUTOSKIP_COUNT:-0}" -ge 1 ] && echo yes || echo "non ($T2590_AUTOSKIP_COUNT)")"

T2590_AUTOSKIP_OFFENDERS=$(printf '%s\n' "$T2590_AUTOSKIP_LINES" | grep -c 'Outcome: PLAN_GROOMED' || true)
assert_eq "mika#2590 (U7b): aucune ligne construisant un RESULT auto_skipped ne cite le marqueur" \
    "0" "${T2590_AUTOSKIP_OFFENDERS:-0}"

# Contrôle négatif de l'assertion elle-même : sur une fixture portant le
# littéral, le prédicat DOIT accuser. Sans lui, « l'assertion tient » et
# « l'assertion ne regarde rien » rendent les mêmes octets (classe mika#2205).
T2590_FIXTURE='RESULT=$(printf '"'"'{"status":"auto_skipped","reason":"x","note":"… carrying Outcome: PLAN_GROOMED exists …"}'"'"')'
assert_eq "mika#2590 (contrôle négatif U7b): le prédicat accuse une fixture qui cite le jeton" \
    "1" "$(printf '%s\n' "$T2590_FIXTURE" | grep '"status":"auto_skipped"' | grep -c 'Outcome: PLAN_GROOMED' || true)"

# ============================================================================
# mika#2194 — UN corpus, DEUX lecteurs, UNE parité (R4) + le scan anti-copie (R5)
# ============================================================================
#
# Le lecteur BASH du corpus doré. Son jumeau est
# `crates/mika-agent/tests/plan_callout_parity.rs`, et les deux lisent le MÊME
# fichier d'attendus. Avant mika#2194 chacun des deux lecteurs du callout avait
# un corpus soigné et **aucune entrée n'était commune** : personne n'avait
# jamais exécuté les deux sur la même entrée.
#
# Cette passe-ci est la NON-RÉGRESSION, pas la preuve. La preuve de parité
# (« mêmes entrées → mêmes sorties que le bash ») se mesure contre le bash
# d'AVANT la bascule, et elle a été faite une fois, à l'implémentation : après
# R3 le bash appelle le Rust, donc un lecteur comparé à lui-même est toujours
# d'accord avec lui-même. D'où la colonne `phase` du TSV, et d'où le fait que
# ce bloc n'exerce que les lignes `both`.

echo ""
echo "Test: mika#2194 — parité du callout Plan sur le corpus commun"
echo "-------------------------------------------------------------"

M2194_CORPUS="$SCRIPT_DIR/../../../crates/mika-agent/tests/fixtures/plan_callout_bodies"
M2194_TSV="$M2194_CORPUS/expectations.tsv"

# ── Anti-vacuité, en trois termes. C'est le mode de panne que R4 nomme : une
#    parité verte sur zéro cas. Le motif dominant du harnais est
#    `source "$DISPATCH_LIB" 2>/dev/null || true`, donc un dispatch-lib qui
#    cesse de sourcer laisserait les assertions vertes sur des fonctions
#    inexistantes — éliminatoire pour un test de parité.
assert_eq "mika#2194 (anti-vacuité 1/3): le corpus d'attendus existe" "yes" \
    "$(if [ -f "$M2194_TSV" ]; then printf 'yes'; else printf 'no'; fi)"

M2194_DECLARED=0
if [ -f "$M2194_TSV" ]; then
    M2194_DECLARED=$(grep -cv '^[[:space:]]*\(#\|$\)' "$M2194_TSV" || true)
fi
assert_eq "mika#2194 (anti-vacuité 2/3): le corpus n'est pas vide" "yes" \
    "$(if [ "$M2194_DECLARED" -gt 0 ]; then printf 'yes'; else printf 'no'; fi)"

assert_eq "mika#2194 (anti-vacuité 3/3): _extract_plan_path est définie après le sourcing" "yes" \
    "$(if declare -f _extract_plan_path >/dev/null; then printf 'yes'; else printf 'no'; fi)"

# ── Le binaire est résolu EN TÊTE DE FICHIER depuis mika#2608 (R4) : les douze
#    cas fonctionnels de la porte de grooming en dépendent aussi et tournent
#    bien avant ce bloc, donc une résolution locale ici les laissait sans.
if [ -z "$MIKA_BIN" ] || [ "$M2194_DECLARED" -eq 0 ]; then
    mika_bin_or_skip "mika#2194/#2608 parité du callout" || true
else
    M2194_CASES=0
    M2194_BAD=0
    while IFS=$'\t' read -r m_file m_rc m_raw m_norm m_parity m_phase; do
        case "$m_file" in ''|'#'*) continue ;; esac
        [ -n "${m_rc:-}" ] || continue
        # Seules les lignes `both` sont exerçables après la bascule. Une ligne
        # `pre-switch` décrit le bash d'AVANT, qui n'existe plus ici.
        [ "$m_phase" = "both" ] || continue

        m_body_file="$M2194_CORPUS/$m_file"
        if [ ! -f "$m_body_file" ]; then
            echo "    ✗ $m_file — corps introuvable"
            M2194_BAD=$((M2194_BAD + 1)); continue
        fi
        m_body=$(cat "$m_body_file")
        M2194_CASES=$((M2194_CASES + 1))

        m_got_rc=0
        m_got=$(_extract_plan_path "$m_body") || m_got_rc=$?

        if [ "$m_rc" = "0" ]; then
            # Le bash veut le chemin NORMALISÉ : sa question est quel fichier
            # ouvrir, et il le résout contre "$WORKTREE_DIR".
            if [ "$m_got_rc" != "0" ] || [ "$m_got" != "$m_norm" ]; then
                echo "    ✗ $m_file — attendu rc=0 '$m_norm', lu rc=$m_got_rc '$m_got'"
                M2194_BAD=$((M2194_BAD + 1))
            fi
        else
            if [ "$m_got_rc" = "0" ] || [ -n "$m_got" ]; then
                echo "    ✗ $m_file — attendu un refus, lu rc=$m_got_rc '$m_got'"
                M2194_BAD=$((M2194_BAD + 1))
            fi
        fi
    done < "$M2194_TSV"

    assert_eq "mika#2194: parité bash↔Rust sur le corpus commun (aucun écart)" \
        "0" "$M2194_BAD"
    # Le compte de cas EST une assertion, pas un affichage : c'est le seul
    # moyen qu'a ce lecteur de dire qu'il a regardé quelque chose.
    assert_eq "mika#2194: le lecteur bash nomme son compte de cas (non nul)" "yes" \
        "$(if [ "$M2194_CASES" -gt 0 ]; then printf 'yes'; else printf 'no'; fi)"
    echo "    → mika#2194 parité bash : $M2194_CASES cas exercés (phase=both)"
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
