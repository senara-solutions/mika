#!/bin/bash
# La suppression de worktree et sa preuve — extrait de test-dispatch-lib.sh par mika#3178, vol 4.
# Un chemin qu'on ne peut pas prouver worktree n'est pas supprimé (mika#1943,
# dont l'incident fondateur est une suppression automatisée) ; une
# `WORKTREE_DIR` vide propagée en silence, nommée plutôt que subie ; les deux
# appels directs qu'avait `_set_up_worktree`, dont l'un est parti en
# `_stash_and_remove_worktree` (mika#2976).
#
# Plage d'origine : lignes 9154–9402 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 3849d757. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ===========================================================================
# mika#1943 — un chemin qu'on ne peut pas prouver worktree n'est pas supprimé
# ===========================================================================
#
# L'incident fondateur (28/07) est une suppression automatisée qui a emporté
# `/data/workspace/bbytaa`, un répertoire qui n'était protégé par aucune liste —
# il était protégé par btrbk. La garde posée ici est une **allowlist positive**
# alignée terme pour terme sur `worktree_reaper::is_managed_worktree_path`
# (mika#2420) : un chemin doit *prouver* qu'il est un worktree géré, au lieu
# d'être absent d'une denylist. Une denylist est fausse le jour où un répertoire
# précieux n'y figure pas — c'est-à-dire le jour où elle servirait.
#
# L'asymétrie qui décide de tout : un faux négatif laisse un worktree résiduel
# sur le disque (le reaper mika#2420 le ramasse au tick suivant, coût borné) ;
# un faux positif supprime un répertoire qui n'est pas un worktree, et c'est
# irréversible. Donc tout terme illisible conserve.

echo ""
echo "Test: garde de suppression — allowlist worktree (mika#1943)"
echo "------------------------------------------------------------"

_MIKA1943_MANAGED_ROOT='/data/workspace/mika-platform/.claude/worktrees'

# Rend REFUSED / ACCEPTED, et sur un refus le jeton `term=` émis sur stderr.
_mika1943_probe() {
    local lib="$1" path="$2" err rc
    err=$(
        # shellcheck disable=SC1090
        source "$lib" 2>/dev/null || true
        _assert_removable_worktree_path "$path" probe 2>&1 >/dev/null
        printf '\nRC=%s' "$?"
    )
    rc=${err##*RC=}
    if [ "$rc" = "0" ]; then
        printf 'ACCEPTED'
        return 0
    fi
    # Extraction du jeton `term=` par expansion bash, sans pipe : un
    # `sed … | head -1` sous `set -o pipefail` (en tête de ce fichier) rend 141
    # dès que `head` ferme le tuyau avant la fin de `sed`. Ça ne se produit pas
    # sur une ligne courte, donc ça passerait — et rougirait le jour où le
    # message du refus s'allonge, pour une raison sans rapport avec ce qu'il
    # teste.
    local term=""
    case "$err" in *term=*) term=${err#*term=}; term=${term%% *} ;; esac
    printf 'REFUSED:%s' "$term"
}

# --- Contrôles positifs : chaque chemin est refusé, et par le bon terme ------

assert_eq "mika#1943: chaîne vide refusée (la racine du défaut)" \
    "REFUSED:empty" "$(_mika1943_probe "$DISPATCH_LIB" "")"
assert_eq "mika#1943: /data/workspace/foo refusé (fixture de l'AC2)" \
    "REFUSED:outside_managed_root" "$(_mika1943_probe "$DISPATCH_LIB" "/data/workspace/foo")"
assert_eq "mika#1943: /data/workspace/bbytaa refusé (le répertoire de l'incident)" \
    "REFUSED:outside_managed_root" "$(_mika1943_probe "$DISPATCH_LIB" "/data/workspace/bbytaa")"
assert_eq "mika#1943: chemin relatif refusé" \
    "REFUSED:not_absolute" "$(_mika1943_probe "$DISPATCH_LIB" "relatif/x")"
assert_eq "mika#1943: traversée par .. refusée malgré le segment gardé" \
    "REFUSED:parent_dir_component" \
    "$(_mika1943_probe "$DISPATCH_LIB" "$_MIKA1943_MANAGED_ROOT/../../../etc")"
assert_eq "mika#1943: / refusé" \
    "REFUSED:outside_managed_root" "$(_mika1943_probe "$DISPATCH_LIB" "/")"

# --- Contrôle négatif : c'est lui qui donne un sens aux positifs -------------
#
# Doctrine mika#2420 : sans ce contrôle, une fonction qui refuse *tout* passerait
# la suite en vert tout en cassant chaque dispatch.
assert_eq "mika#1943: un vrai worktree de dispatch est accepté (contrôle négatif)" \
    "ACCEPTED" \
    "$(_mika1943_probe "$DISPATCH_LIB" "$_MIKA1943_MANAGED_ROOT/chore-1943-platform-hygiene-re-file-p0-28-07/mika")"

# --- Le refus est DIT ------------------------------------------------------
#
# Sans cette ligne, un refus se lirait exactement comme une absence de travail
# (classe mika#2205) : le dispatch continuerait, le worktree résiduel resterait,
# et rien dans les journaux ne nommerait la cause.
_MIKA1943_REFUSAL_STDERR=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB" 2>/dev/null || true
    _assert_removable_worktree_path "/data/workspace/bbytaa" site_probe 2>&1 >/dev/null || true
)
assert_contains "mika#1943: le refus émet l'événement nommé" \
    "dispatch_lib_unsafe_removal_refused" "$_MIKA1943_REFUSAL_STDERR"
assert_contains "mika#1943: le refus nomme le chemin" \
    "/data/workspace/bbytaa" "$_MIKA1943_REFUSAL_STDERR"
assert_contains "mika#1943: le refus nomme le terme qui a échoué" \
    "term=outside_managed_root" "$_MIKA1943_REFUSAL_STDERR"
assert_contains "mika#1943: le refus nomme le site appelant" \
    "site=site_probe" "$_MIKA1943_REFUSAL_STDERR"

# --- Vérification par INJECTION, exigée par l'AC2 ---------------------------
#
# Neutraliser les quatre termes d'un coup ne prouverait rien : une conjonction ne
# se teste pas en désarmant tous ses termes ensemble (leçon mika#2277). Chaque
# terme porte donc un marqueur de fin de ligne `# mika1943:T<n>`, et l'injection
# remplace **une** ligne à la fois par un no-op.
#
# `MUTATION_ABSENTE` est le garde-fou du garde-fou : si quelqu'un renomme un
# marqueur ou fusionne deux termes, le `sed` ne mord plus et le test passerait au
# vert en n'ayant rien vérifié.
_mika1943_inject() {
    local term="$1" path="$2" tmp
    tmp=$(mktemp)
    sed "s|^.*# mika1943:${term}\$|    :|" "$DISPATCH_LIB" >"$tmp"
    if cmp -s "$DISPATCH_LIB" "$tmp"; then
        rm -f "$tmp"
        printf 'MUTATION_ABSENTE'
        return 0
    fi
    _mika1943_probe "$tmp" "$path"
    rm -f "$tmp"
}

# T4 — LE terme de l'AC2. Le tuer laisse passer le répertoire de l'incident.
assert_eq "mika#1943 injection T4: sans l'allowlist, /data/workspace/bbytaa passe" \
    "ACCEPTED" "$(_mika1943_inject T4 "/data/workspace/bbytaa")"

# T3 — sans lui, le segment gardé peut être traversé jusqu'à n'importe quoi.
assert_eq "mika#1943 injection T3: sans le refus de .., la traversée passe" \
    "ACCEPTED" "$(_mika1943_inject T3 "$_MIKA1943_MANAGED_ROOT/../../../etc")"

# T2 — sans lui, un chemin relatif portant le segment passe. `git -C` et `rm -rf`
# le résoudraient depuis le cwd du moment, qui n'est pas une garantie.
assert_eq "mika#1943 injection T2: sans l'exigence d'absolu, le relatif passe" \
    "ACCEPTED" "$(_mika1943_inject T2 "mika-platform/.claude/worktrees/foo/mika")"

# T1 — cas particulier, et il faut dire pourquoi il ne rend pas ACCEPTED.
# La chaîne vide est aussi non-absolue : T2 la rattrape. Ce que T1 porte n'est
# donc pas la sûreté mais le **diagnostic** — et c'est exactement la racine de
# mika#1943, une `WORKTREE_DIR` vide propagée en silence. La nommer
# `not_absolute` enverrait l'opérateur chercher une faute de frappe de chemin
# au lieu d'une dérivation qui a échoué.
assert_eq "mika#1943 injection T1: sans lui, le vide est mal nommé (T2 le rattrape)" \
    "REFUSED:not_absolute" "$(_mika1943_inject T1 "")"

# --- Lecteur unique --------------------------------------------------------
#
# Cinq sites destructifs, donc cinq occasions de diverger. C'est la leçon que
# `grooming_marker` (mika#2158) a dû engraver une fois dans ce dépôt : deux
# prédicats répondant différemment à « ce chemin est-il supprimable ».
_MIKA1943_GUARD_BODY=$(awk '/^_assert_removable_worktree_path\(\) \{/,/^\}/' "$DISPATCH_LIB")
_MIKA1943_MARKERS_TOTAL=$(grep -c '# mika1943:T[0-9]$' "$DISPATCH_LIB" || true)
_MIKA1943_MARKERS_IN_GUARD=$(printf '%s\n' "$_MIKA1943_GUARD_BODY" | grep -c '# mika1943:T[0-9]$' || true)
assert_eq "mika#1943: quatre termes, et pas un de plus" "4" "$_MIKA1943_MARKERS_TOTAL"
assert_eq "mika#1943: les quatre termes vivent dans la seule fonction qui décide" \
    "4" "$_MIKA1943_MARKERS_IN_GUARD"

# --- U2: la racine est fermée ----------------------------------------------
#
# `derive-worktree-path` vit dans mika-platform et le fichier n'a ni `set -e` ni
# `set -u`. Sans vérification, un script absent ou en échec rend une chaîne vide
# qui se propage en silence jusqu'aux sites de suppression — et jusqu'à la
# comparaison d'égalité qui ÉLIT une cible.
_MIKA1943_SETUP_BODY=$(awk '/^_set_up_worktree\(\) \{/,/^\}/' "$DISPATCH_LIB")
assert_contains "mika#1943 U2: l'échec de dérivation est nommé et abandonne le dispatch" \
    "worktree_path_derivation_failed" "$_MIKA1943_SETUP_BODY"
assert_contains "mika#1943 U2: la comparaison d'élection exige une cible non vide" \
    '[ -n "$WORKTREE_DIR" ] && [ -n "$existing_wt" ]' "$_MIKA1943_SETUP_BODY"

# --- U2: les cinq sites destructifs appellent la garde ----------------------
#
# Le plan en nomme quatre plus la racine ; `_handle_dry_run` est le cinquième,
# trouvé à la lecture. L'invariant du Product Contract porte sur *tout* site de
# suppression, pas sur la liste énumérée.
_mika1943_guard_calls_in() {
    awk -v fn="^$1\\\\(\\\\) \\\\{" '$0 ~ fn, /^\}/' "$DISPATCH_LIB" \
        | grep -c '_assert_removable_worktree_path' || true
}
# mika#2976: `_set_up_worktree` avait DEUX appels directs. L'un des deux — le
# relic non canonique — vit maintenant dans `_stash_and_remove_worktree`, qui
# est le second appelant du helper avec `_reset_spent_branch`. Le total de sites
# de suppression gardés est inchangé ; c'est leur répartition qui a bougé, et le
# recensement suit, parce que l'invariant porte sur *tout* site de suppression
# et non sur une liste figée.
assert_eq "mika#1943 U2: _set_up_worktree garde sa suppression directe (registre périmé)" \
    "1" "$(_mika1943_guard_calls_in _set_up_worktree)"
assert_eq "mika#1943 U2 (mika#2976): _stash_and_remove_worktree garde la sienne" \
    "1" "$(_mika1943_guard_calls_in _stash_and_remove_worktree)"
assert_eq "mika#1943 U2: _handle_dry_run garde la sienne" \
    "1" "$(_mika1943_guard_calls_in _handle_dry_run)"
assert_eq "mika#1943 U2: _clean_worktree_for_rebase garde son rm -rf" \
    "1" "$(_mika1943_guard_calls_in _clean_worktree_for_rebase)"
assert_eq "mika#1943 U2: _cleanup_iterate_findings garde son rm -rf" \
    "1" "$(_mika1943_guard_calls_in _cleanup_iterate_findings)"
assert_eq "mika#1943 U2: _seed_pilot_scratch_dir garde son rm -rf (mika#2548)" \
    "1" "$(_mika1943_guard_calls_in _seed_pilot_scratch_dir)"

# --- U2: la garde est CÂBLÉE au site, et elle n'y casse rien ----------------
#
# Les assertions structurelles ci-dessus prouvent qu'un appel existe, jamais
# qu'il décide. Et sur ce site précis le reset chirurgical est plus porteur
# qu'il n'y paraît : `.iterate/` est gitignored (`.gitignore:53`), donc le
# tier 3 ne le voit pas (`status --porcelain` ignore les fichiers ignorés) et
# `clean -fd` ne le supprime pas (pas de `-x`). Ce `rm -rf` est la SEULE chose
# qui le retire. Une garde trop stricte y ferait s'accumuler les findings d'une
# passe de grooming à l'autre, en silence.
#
# D'où les deux moitiés : un chemin de la forme réelle doit être balayé, un
# chemin non géré doit être épargné ET dit.
_mika1943_clean_probe() {
    local wt="$1" out
    mkdir -p "$wt"
    git -C "$wt" init -q
    printf '.iterate/\n' >"$wt/.gitignore"
    git -C "$wt" add .gitignore
    git -C "$wt" commit -q -m "base"
    mkdir -p "$wt/.iterate"
    printf 'findings\n' >"$wt/.iterate/findings-1.md"
    out=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        LOG_ID="test-1943" _clean_worktree_for_rebase "$wt" 2>&1 >/dev/null
    )
    if [ -d "$wt/.iterate" ]; then printf 'KEPT|%s' "$out"; else printf 'SWEPT|%s' "$out"; fi
}

_MIKA1943_CLEAN_ROOT=$(mktemp -d)
_MIKA1943_CLEAN_MANAGED=$(_mika1943_clean_probe \
    "$_MIKA1943_CLEAN_ROOT/.claude/worktrees/chore-1943-fixture/mika")
_MIKA1943_CLEAN_FOREIGN=$(_mika1943_clean_probe "$_MIKA1943_CLEAN_ROOT/pas-un-worktree")
rm -rf "$_MIKA1943_CLEAN_ROOT"

assert_contains "mika#1943 U2: sur un worktree géré, .iterate est bien balayé (contrôle négatif)" \
    "SWEPT|" "$_MIKA1943_CLEAN_MANAGED"
assert_not_contains "mika#1943 U2: et le balayage nominal n'émet aucun refus" \
    "dispatch_lib_unsafe_removal_refused" "$_MIKA1943_CLEAN_MANAGED"
assert_contains "mika#1943 U2: sur un chemin non géré, .iterate est épargné" \
    "KEPT|" "$_MIKA1943_CLEAN_FOREIGN"
assert_contains "mika#1943 U2: et l'épargne est DITE, pas silencieuse" \
    "term=outside_managed_root" "$_MIKA1943_CLEAN_FOREIGN"

# Et aucun site de suppression n'y échappe : tout `rm -rf` portant une variable
# de worktree, et tout `worktree remove`, sont précédés de la garde. Un test
# comportemental ne peut pas voir cette classe — un sixième site ajouté demain
# ne rendrait aucune décision fausse, il rouvrirait le trou en silence.
#
# Le recensement porte sur les lignes de CODE : compter aussi les commentaires
# ferait passer le test au vert pour la mauvaise raison — le premier jet le
# faisait, et il comptait les commentaires que ce même correctif venait
# d'ajouter.
_MIKA1943_CODE_LINES=$(grep -v '^[[:space:]]*#' "$DISPATCH_LIB" || true)
_MIKA1943_WT_REMOVES=$(printf '%s\n' "$_MIKA1943_CODE_LINES" | grep -c 'worktree remove --force' || true)
assert_eq "mika#1943 U2: exactement trois 'worktree remove --force' recensés" \
    "3" "$_MIKA1943_WT_REMOVES"
_MIKA1943_RMRF_WT=$(printf '%s\n' "$_MIKA1943_CODE_LINES" | grep -cE 'rm -rf "\$(wt|WORKTREE_DIR|findings_dir|scratch)' || true)
assert_eq "mika#1943 U2: exactement trois 'rm -rf' sur un chemin de worktree" \
    "3" "$_MIKA1943_RMRF_WT"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
