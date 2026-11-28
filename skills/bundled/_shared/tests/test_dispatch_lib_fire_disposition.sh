#!/bin/bash
# La section `## Fire-Disposition` et son site de production — extrait de test-dispatch-lib.sh par mika#3178, vol 4.
# Les onze détecteurs du site de production de la section (mika#2306, T1–T11) ;
# les lecteurs qui tolèrent la numérotation des options (mika#2544, T12) ; le
# site extrait en `_stash_and_remove_worktree` (mika#2976) ; `PLAN_GROOMED`,
# seule des trois valeurs à n'être pas passée par la porte (mika#2981) ; le refus
# de halte qui nomme son motif dans la ligne d'outcome elle-même (mika#3142).
#
# Plage d'origine : lignes 9403–10311 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 3849d757. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# `_t2178_after` (agrégat l. 7426, famille 9) : lue par le T4 de mika#2306
# ci-dessous. Sans elle, la suite tourne jusqu'au bout et rend un FAIL de plus
# que l'agrégat, sans rien dire — mesuré au groom du vol 4.
# shellcheck source=lib-dispatch-lib-shared-probes-fixtures.sh
source "$_SUITE_DIR/lib-dispatch-lib-shared-probes-fixtures.sh"
# ============================================================================
# mika#2306 — la section `## Fire-Disposition` a un site de production (T1–T11)
# ============================================================================
#
# Ce que ces onze détecteurs mesurent. Avant mika#2306, aucun canal que CE dépôt
# contrôle ne prescrivait `## Fire-Disposition` : `/ce:plan` est un plugin tiers
# qui ignore mika#1574, les trois commandes de groom vivent dans mika-platform,
# et `grep -n "Fire-Disposition" .claude/commands/*.md` rendait zéro. Un plan
# neuf livrant un détecteur arrivait donc devant mika-arch sans la section, et
# l'unique itération de la boucle était dépensée sur un motif formel — puis le
# gate de seconde passe, sans recours, rendait ESCALATE.
#
# Les deux moitiés sont livrées ensemble à dessein : la prescription (T1–T4)
# ne tiendrait pas seule, et le rattrapage (T5–T11) ne tiendrait pas seul non
# plus. Les quatre contrôles négatifs (T3, T6, T7, T11) sont porteurs, pas
# décoratifs : sans eux, une garde qui relance TOUJOURS passerait T5 en vert
# tout en doublant le coût de chaque grooming du dépôt.

echo ""
echo 'Test: mika#2306 — site de production de `## Fire-Disposition` (T1–T11)'
echo "-----------------------------------------------------------------------"

# --- Outillage local de section ---------------------------------------------

# Réplique le site d'injection de `_set_up_worktree` : contexte de ticket, puis
# règle de corps de PR (inconditionnelle), puis règle Fire-Disposition
# (conditionnée au skill). T2/T3 assertent sur CETTE chaîne ; T4 asserte que
# l'expression source est littéralement celle répliquée ici, donc le miroir ne
# peut pas dériver en silence (patron mika#2178 T3).
_t2306_inject() {
    local skill="$1" repo="${2:-mika}" issue_num="${3:-2306}"
    local prompt="${repo}#${issue_num}"
    prompt=$(printf '%s\n\n%s' "$prompt" "$_PR_BODY_CONTAINMENT_RULE")
    if [ "$skill" = "dev-groom" ]; then
        prompt=$(printf '%s\n\n%s' "$prompt" "$_FIRE_DISPOSITION_RULE")
    fi
    printf '%s' "$prompt"
}

# Sonde comportementale de `_launch_revise_pilot`.
#
# Elle exerce la VRAIE fonction : seul `_run_pilot_sandboxed` est neutralisé
# (par redéfinition locale au sous-shell, jamais par une chaîne écrite à la
# main), ce qui est la seule façon de prouver que le compte de relances vient de
# la garde et de rien d'autre. Chaque invocation du pilote simulé incrémente un
# compteur sur disque et applique la mutation demandée au plan — modifier le
# plan est indispensable, sans quoi `pre_hash == post_hash` et le flot tombe
# dans la branche d'échec avant d'atteindre la garde.
#
# $1 = contenu de .iterate/findings-1.md
# $2 = le plan initial porte-t-il déjà la section ? (yes|no), ou sous quelle
#      forme de titre (mika#2544) : numbered (`## 3.`), upper (`## 3.` en
#      capitales), ish (`## Fire-Disposition-ish`), notes (`## 11. Notes on …`),
#      subsub (`## 1.1 …`)
# $3 = comportement du pilote simulé :
#      never  — ne l'ajoute jamais
#      second — l'ajoute à la deuxième invocation
#      second-numbered — l'ajoute NUMÉROTÉE à la deuxième invocation (mika#2544)
#      wipe   — supprime le findings-file de première passe (fail-safe en vol)
# $4 = (optionnel) "residual-fd" dépose un findings-1-fd.md résiduel
# Rend : "<rc>|<nombre d'invocations du pilote>|<stderr>"
_t2306_revise_probe() {
    local t2306_findings="$1" t2306_has_section="$2" t2306_behaviour="$3" t2306_extra="${4:-}"
    local t2306_root t2306_wt t2306_plan t2306_counter t2306_err t2306_rc
    t2306_root=$(mktemp -d)
    t2306_wt="$t2306_root/.claude/worktrees/fix-2306-probe/mika"
    mkdir -p "$t2306_wt/docs/plans" "$t2306_wt/.iterate"
    t2306_plan="$t2306_wt/docs/plans/2026-09-21-001-fix-2306-sonde-plan.md"

    # > 500 octets : `_find_issue_plan` filtre les plans plus courts (mika#1033).
    # Le remplissage est volontairement large — un plan qui tombe SOUS le seuil
    # fait rendre « no plan file to revise », c'est-à-dire un vert qui n'a rien
    # exercé de la garde. C'est le premier défaut que cette sonde a eu.
    {
        printf '# mika#2306 — plan de sonde\n\n**Ticket :** mika issue#2306\n\n## Problème\n\n'
        printf 'Corps de remplissage pour franchir le filtre des 500 octets applique par\n'
        printf '_find_issue_plan a tous ses tiers de decouverte. Ce texte ne porte aucune\n'
        printf 'signification pour la sonde : seules comptent sa longueur, et la presence\n'
        printf "ou l'absence de la section testee.\n\n"
        printf 'Le seuil existe pour ecarter les fichiers-fantomes et les ebauches vides ;\n'
        printf "il s'applique identiquement aux trois tiers de decouverte, donc un plan\n"
        printf 'trop court est invisible quelle que soit la façon dont il est nomme. Une\n'
        printf 'sonde qui passe sous ce seuil ne mesure rien et le dit mal : elle rend\n'
        printf 'exactement le meme « plan introuvable » que la garde de fonction.\n\n'
        printf '## Implementation Units\n\nU1 — un livrable detecteur quelconque.\n\n'
        # mika#2544 — cinq formes de titre en plus des deux d'origine. La ligne
        # `## Acceptance criteria` qui suit reste HORS du `case` : l'emporter en
        # réécrivant ce bloc casserait tout futur gate AC sur ce fixture.
        case "$t2306_has_section" in
            yes)      printf '## Fire-Disposition\n\nOption (a) — exception nommee, table vide.\n\n' ;;
            numbered) printf '## 3. Fire-Disposition\n\nOption (a) — exception nommee, table vide.\n\n' ;;
            upper)    printf '## 3. FIRE-DISPOSITION\n\nOption (a) — exception nommee, table vide.\n\n' ;;
            ish)      printf '## Fire-Disposition-ish\n\nOption (a) — exception nommee, table vide.\n\n' ;;
            notes)    printf '## 11. Notes on Fire-Disposition\n\nRemarques, pas la section.\n\n' ;;
            subsub)   printf '## 1.1 Fire-Disposition\n\nOption (a) — exception nommee, table vide.\n\n' ;;
        esac
        printf '## Acceptance criteria\n\nAC1 — la sonde tourne.\n'
    } > "$t2306_plan"

    printf '%s\n' "$t2306_findings" > "$t2306_wt/.iterate/findings-1.md"
    if [ "$t2306_extra" = "residual-fd" ]; then
        printf 'Fire-Disposition — residu laisse par un dispatch anterieur.\n' \
            > "$t2306_wt/.iterate/findings-1-fd.md"
    fi

    t2306_counter="$t2306_root/pilot-invocations"
    printf '0\n' > "$t2306_counter"

    # UNE seule exécution : le rc transite par un fichier plutôt que par une
    # seconde passe. Rejouer la fonction avec un pilote inerte donnerait
    # `pre_hash == post_hash`, donc un rc de 1 sans rapport avec ce qui est
    # mesuré — un faux vert sur la moitié fail-safe.
    t2306_err=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        # Neutralisation RÉELLE : seul le lancement du pilote est shadowé, tout
        # le reste de `_launch_revise_pilot` s'exécute tel quel. C'est la seule
        # façon de prouver que le compte de relances vient de la garde.
        _run_pilot_sandboxed() {
            local n; n=$(( $(cat "$t2306_counter") + 1 ))
            printf '%s\n' "$n" > "$t2306_counter"
            case "$t2306_behaviour" in
                wipe) rm -f "$t2306_wt/.iterate/findings-1.md" ;;
                second) [ "$n" -ge 2 ] && printf '\n## Fire-Disposition\n\nOption (a).\n' >> "$t2306_plan" ;;
                second-numbered) [ "$n" -ge 2 ] && printf '\n## 3. Fire-Disposition\n\nOption (a).\n' >> "$t2306_plan" ;;
            esac
            # Mutation inconditionnelle : sans elle le sha ne bouge pas, et la
            # branche de succès — donc la garde — n'est jamais atteinte.
            printf '\n<!-- revise %s -->\n' "$n" >> "$t2306_plan"
            return 0
        }
        set +e
        WORKTREE_DIR="$t2306_wt" ISSUE_NUM="2306" REPO="mika" LOG_ID="t2306" \
            CWD_ARGS="--cwd $t2306_wt" \
            _launch_revise_pilot "$t2306_wt/.iterate/findings-1.md" 2>&1 >/dev/null
        printf '%s\n' "$?" > "$t2306_root/rc"
    )
    t2306_rc=$(cat "$t2306_root/rc")
    printf '%s|%s|%s' "$t2306_rc" "$(cat "$t2306_counter")" "$t2306_err"
    rm -rf "$t2306_root"
}

# Le champ 3 est du stderr libre — multiligne, et pouvant contenir des `|`. Un
# `cut` nu le traiterait ligne par ligne et rendrait, pour les champs 1 et 2,
# les lignes de stderr qui ne portent aucun délimiteur. Deuxième défaut que
# cette sonde a eu, et il produisait des `expected: 1 / actual: 1` illisibles.
_t2306_field() { printf '%s' "$2" | head -1 | cut -d'|' -f"$1"; }
_t2306_err()   { printf '%s' "$1" | sed '1s/^[0-9]*|[0-9]*|//'; }

T2306_FINDINGS_WITH_FD="F1 [BLOQUANT] — le plan livre des tests et ne porte pas de section
\`## Fire-Disposition\`. Ajoute-la en nommant l'une des trois options de mika#1574.

Disposition: ITERATE"
T2306_FINDINGS_WITHOUT_FD="F1 [BLOQUANT] — l'unité U2 laisse le choix de la structure de
données à l'implémenteur. Tranche-le dans le plan.

Disposition: ITERATE"

T2306_LIB_SRC=$(cat "$DISPATCH_LIB")
T2306_SUW_SRC=$(sed -n '/^_set_up_worktree() {/,/^}/p' "$DISPATCH_LIB")
T2306_REVISE_SRC=$(declare -f _launch_revise_pilot)
T2306_GUARD_SRC=$(declare -f _fd_retry_if_section_still_missing)

# --- T1 : la règle existe et dit quoi écrire ---------------------------------
#
# Une règle tronquée qui prescrit la section sans nommer ses options ferait
# inventer une quatrième disposition au groomeur — pire que le silence, parce
# qu'elle passerait le `grep` du gate et échouerait sur son contenu.

assert_eq "T1: _FIRE_DISPOSITION_RULE est définie et non vide" "non-vide" \
    "$([ -n "${_FIRE_DISPOSITION_RULE:-}" ] && echo non-vide || echo vide)"
assert_contains "T1: la règle nomme la section exacte" \
    '## Fire-Disposition' "${_FIRE_DISPOSITION_RULE:-}"
assert_contains "T1: la règle nomme l'option (a)" "(a)" "${_FIRE_DISPOSITION_RULE:-}"
assert_contains "T1: la règle nomme l'option (b)" "(b)" "${_FIRE_DISPOSITION_RULE:-}"
assert_contains "T1: la règle nomme l'option (c)" "(c)" "${_FIRE_DISPOSITION_RULE:-}"
assert_contains "T1: la règle cite la doctrine par référence plutôt que de la reformuler" \
    "mika#1574" "${_FIRE_DISPOSITION_RULE:-}"
# La conditionnalité est la moitié que l'option (a) du gate rend explicite :
# sans elle, un groomeur ajouterait la section à tout plan, ce qui contredit
# « Plan has no detector-class deliverables ⇒ gate is N/A ».
assert_contains "T1: la règle dit que la section n'est PAS requise sans détecteur" \
    "N/A" "${_FIRE_DISPOSITION_RULE:-}"

# --- T2 : la prescription atteint le groomeur --------------------------------

T2306_PROMPT_GROOM=$(_t2306_inject "dev-groom")
assert_contains "T2: la règle est injectée dans le PROMPT pour SKILL=dev-groom" \
    '## Fire-Disposition' "$T2306_PROMPT_GROOM"
assert_contains "T2: et le site d'injection existe bien dans _set_up_worktree" \
    '_FIRE_DISPOSITION_RULE' "$T2306_SUW_SRC"

# --- T3 : contrôle négatif — dev-pilot ne la reçoit pas ----------------------
#
# Porteur, pas décoratif : le site voisin (`_PR_BODY_CONTAINMENT_RULE`) est
# inconditionnel dans cette branche, donc la condition se PERD en copiant le
# voisin. C'est exactement l'écart que ce test attrape — et il ne rendrait
# aucune décision fausse, il mettrait juste du bruit dans le prompt de tout
# pilote d'implémentation.

T2306_PROMPT_PILOT=$(_t2306_inject "dev-pilot")
assert_not_contains "T3: la règle n'est PAS injectée pour SKILL=dev-pilot" \
    '## Fire-Disposition' "$T2306_PROMPT_PILOT"
assert_contains "T3: mais dev-pilot reçoit bien la règle inconditionnelle voisine" \
    "RÈGLE DE DISPATCH (mika#2211)" "$T2306_PROMPT_PILOT"
# La garde est lue par POSITION dans les lignes de code, pas par un `grep -B`
# à fenêtre fixe : une ligne de commentaire ajoutée demain entre les deux
# déplacerait la fenêtre et rendrait le test vert pour la mauvaise raison.
T2306_SUW_CODE=$(printf '%s\n' "$T2306_SUW_SRC" | grep -v '^[[:space:]]*#')
T2306_FD_INJ_LN=$(printf '%s\n' "$T2306_SUW_CODE" | grep -n '_FIRE_DISPOSITION_RULE' | head -1 | cut -d: -f1)
T2306_FD_PREV_LN=""
if [ -n "$T2306_FD_INJ_LN" ] && [ "$T2306_FD_INJ_LN" -gt 1 ]; then
    T2306_FD_PREV_LN=$(printf '%s\n' "$T2306_SUW_CODE" | sed -n "$((T2306_FD_INJ_LN - 1))p")
fi
assert_contains "T3: la ligne de code qui précède l'injection est la garde sur le skill" \
    'if [ "$SKILL" = "dev-groom" ]; then' "$T2306_FD_PREV_LN"

# --- T4 : contrat mika#138 et fidélité du miroir -----------------------------
#
# Invariant de position 2 : si la première ligne de PROMPT cesse d'être
# `<repo>#<num>`, la regex ancrée de `_set_up_worktree` manque, le dispatch
# tombe en mode free-text et AUCUN worktree n'est créé. Régression de premier
# ordre, et invisible à toute assertion sur le contenu de la règle.

assert_eq "T4: la première ligne du PROMPT reste exactement mika#2306" "mika#2306" \
    "$(printf '%s' "$T2306_PROMPT_GROOM" | head -1)"
assert_eq "T4: idem sur le chemin dev-pilot" "mika#2306" \
    "$(printf '%s' "$T2306_PROMPT_PILOT" | head -1)"
assert_contains "T4: l'expression source est littéralement celle que le miroir T2/T3 réplique" \
    'PROMPT=$(printf '"'"'%s\n\n%s'"'"' "$PROMPT" "$_FIRE_DISPOSITION_RULE")' "$T2306_LIB_SRC"
# La règle est appendue APRÈS celle de mika#2211 : l'inverse laisserait la règle
# de corps de PR en dernier sur le chemin dev-groom, or la recency est le seul
# levier qu'a une règle en fin d'un prompt de 16 KiB.
assert_eq "T4: l'injection FD suit celle de mika#2211" "yes" \
    "$(_t2178_after '"$PROMPT" "$_FIRE_DISPOSITION_RULE")' '"$PROMPT" "$_PR_BODY_CONTAINMENT_RULE")')"

# --- T5 : le cœur du correctif ----------------------------------------------
#
# Findings réclamant la section + plan révisé qui ne la porte toujours pas
# ⇒ exactement UNE relance (deux invocations du pilote : la nominale et elle).

T2306_T5=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "no" "never")
assert_eq "T5: findings réclamant FD + section absente ⇒ exactement une relance" \
    "2" "$(_t2306_field 2 "$T2306_T5")"
assert_contains "T5: et la relance est DITE, pas silencieuse" \
    "fire_disposition_revise_retried" "$T2306_T5"
assert_eq "T5: la valeur de retour reste 0 — la garde ajoute une tentative, pas un mode d'échec" \
    "0" "$(_t2306_field 1 "$T2306_T5")"

# Et quand la seconde tentative réussit, elle se tait : l'événement d'échec est
# réservé à l'échec, sinon il ne mesure plus rien.
T2306_T5B=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "no" "second")
assert_eq "T5: seconde tentative réussie ⇒ toujours une seule relance" \
    "2" "$(_t2306_field 2 "$T2306_T5B")"
assert_not_contains "T5: et aucun événement d'échec n'est émis quand la section arrive" \
    "fire_disposition_still_missing_after_retry" "$T2306_T5B"

# --- T6 : contrôle négatif — l'architecte n'a rien demandé -------------------

T2306_T6=$(_t2306_revise_probe "$T2306_FINDINGS_WITHOUT_FD" "no" "never")
assert_eq "T6: findings sans mention de FD ⇒ zéro relance" \
    "1" "$(_t2306_field 2 "$T2306_T6")"
assert_not_contains "T6: et rien n'est journalisé" \
    "fire_disposition_revise_retried" "$T2306_T6"

# --- T7 : contrôle négatif — le revise a fait son travail -------------------

T2306_T7=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "yes" "never")
assert_eq "T7: section déjà présente ⇒ zéro relance (le chemin nominal ne paie rien)" \
    "1" "$(_t2306_field 2 "$T2306_T7")"
assert_not_contains "T7: et rien n'est journalisé" \
    "fire_disposition_revise_retried" "$T2306_T7"

# --- T8 : fail-safe — une information illisible sort de la population --------
#
# Deux moitiés. (a) À l'entrée : la garde préexistante rend 1, et la garde FD
# n'y change rien. (b) EN VOL : le pilote de revise supprime le findings-file
# entre-temps. C'est la seule forme que l'illisibilité peut réellement prendre
# ici — l'entrée est déjà gardée — et elle doit sortir le dispatch de la
# population, jamais l'y faire entrer.

assert_eq "T8 (a): findings illisible à l'entrée ⇒ retour 1, inchangé" "1" \
    "$(WORKTREE_DIR="/tmp" ISSUE_NUM="2306" _launch_revise_pilot "/nonexistent/findings-2306.md" 2>/dev/null; echo $?)"

T2306_T8=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "no" "wipe")
assert_eq "T8 (b): findings disparu en vol ⇒ zéro relance" \
    "1" "$(_t2306_field 2 "$T2306_T8")"
assert_eq "T8 (b): et la valeur de retour reste celle d'avant le correctif" \
    "0" "$(_t2306_field 1 "$T2306_T8")"

# --- T9 : le budget architecte est intact ------------------------------------
#
# Structurel, et il l'est pour la raison qui rend ces scans nécessaires ailleurs
# dans ce dépôt : la régression qu'il attrape ne rendrait AUCUNE décision
# fausse. Un futur éditeur qui « améliorerait » la garde en redemandant l'avis
# de l'architecte doublerait le budget LLM par grooming sans qu'un seul test de
# comportement ne rougisse.

assert_eq "T9: zéro appel _arch_ask dans _launch_revise_pilot" "0" \
    "$(printf '%s\n' "$T2306_REVISE_SRC" | grep -c '_arch_ask' || true)"
assert_eq "T9: zéro appel _arch_ask dans la garde de rattrapage" "0" \
    "$(printf '%s\n' "$T2306_GUARD_SRC" | grep -c '_arch_ask' || true)"

# Fire-Disposition, option (a) — la table d'exceptions de CE scan est vide, et
# la vacuité est assertée À L'EXÉCUTION (modèle scripts/test-guard-shared-checkout.sh).
# Le test rougit donc LE JOUR où une exception est ajoutée, pas seulement quand
# elle devient stale. Une table vide assertée vide est aussi ce qui distingue
# « aucune violation » de « le scan ne regarde rien ».
#
# Toute entrée future doit porter LES TROIS propriétés que l'option (a) exige —
# (1) nommer la donnée précise, (2) référencer un ticket de suivi, (3) porter une
# assertion auto-nettoyante — et non la seule troisième.
T2306_ARCH_ASK_ALLOWLIST=()
assert_eq "T9: table d'exceptions du scan _arch_ask — zero entries" "0" \
    "${#T2306_ARCH_ASK_ALLOWLIST[@]}"
# Contre-vacuité : le scan doit voir quelque chose. `_iterate_groom_loop`, la
# fonction voisine, en contient bien — sinon un `grep -c` qui rend 0 partout
# prouverait seulement que la source n'est pas lue.
_T2306_ITERATE_SRC=$(declare -f _iterate_groom_loop)
if [ "$(printf '%s\n' "$_T2306_ITERATE_SRC" | grep -c '_arch_ask' || true)" -gt 0 ]; then
    assert_eq "T9 contre-vacuité: le scan voit bien les _arch_ask de _iterate_groom_loop" "ok" "ok"
else
    assert_eq "T9 contre-vacuité: le scan voit bien les _arch_ask de _iterate_groom_loop" "ok" \
        "le scan ne trouve aucun _arch_ask nulle part — il ne regarde rien"
fi

# --- T10 : terminaison, et l'échec de second tour est DIT --------------------
#
# Sans ce test, une garde qui rendrait la main en silence sur ce chemin
# passerait T5 en vert : l'échec de second tour deviendrait indistinguable d'un
# succès — précisément l'angle mort que ce correctif reproche au critère sha256.

assert_contains "T10: seconde tentative échouée ⇒ fire_disposition_still_missing_after_retry" \
    "fire_disposition_still_missing_after_retry" "$T2306_T5"
assert_eq "T10: émis exactement une fois" "1" \
    "$(_t2306_err "$T2306_T5" | grep -c 'fire_disposition_still_missing_after_retry' || true)"
assert_eq "T10: et AUCUNE troisième relance" "2" "$(_t2306_field 2 "$T2306_T5")"
# La terminaison est lisible sans dérouler le flot : un compteur explicite, armé
# AVANT toute action, donc un échec en aval ne peut pas rouvrir le budget.
assert_contains "T10: le compteur de garde existe et est armé avant l'action" \
    "_FD_REVISE_RETRIED=1" "$T2306_GUARD_SRC"
assert_contains "T10: et il est remis à zéro à chaque entrée de _launch_revise_pilot" \
    "_FD_REVISE_RETRIED=0" "$T2306_REVISE_SRC"

# --- T11 : contrôle de source — le prédicat auto-entretenu ------------------
#
# La seule défaillance de cette famille qu'aucun autre test ne voit. Un
# implémenteur qui lit le findings CIBLÉ (qui contient nécessairement la chaîne,
# c'est son objet) rend le premier terme vrai par construction : la garde
# relance même quand l'architecte n'a rien demandé, le compteur borne la boucle,
# et T5, T6, T8 restent TOUS verts. T6 exerce un findings de première passe
# propre, pas la lecture du mauvais fichier.

# (a) forme de code : aucune ligne qui manipule le findings ciblé ne l'interroge,
#     et la relance n'est pas une récursion.
T2306_FD_VAR_LINES=$(printf '%s\n' "$T2306_GUARD_SRC" | grep 'fd_findings_file' || true)
assert_eq "T11 (a): aucune ligne touchant findings-1-fd.md ne l'interroge par grep" "0" \
    "$(printf '%s\n' "$T2306_FD_VAR_LINES" | grep -c 'grep' || true)"
assert_eq "T11 (a): et la garde ne se relance pas par récursion sur _launch_revise_pilot" "0" \
    "$(printf '%s\n' "$T2306_GUARD_SRC" | grep -c '_launch_revise_pilot' || true)"
# Contre-vacuité de la moitié (a) : la garde manipule réellement ce fichier.
assert_eq "T11 (a) contre-vacuité: la garde écrit bien un findings ciblé" "yes" \
    "$([ -n "$T2306_FD_VAR_LINES" ] && echo yes || echo non-trouvé)"
# Et le terme 1 lit bien le paramètre reçu, pas un chemin reconstruit.
assert_contains "T11 (a): le terme 1 interroge le findings de première passe reçu en argument" \
    'first_pass_findings' "$(printf '%s\n' "$T2306_GUARD_SRC" | grep 'Fire-Disposition' | grep 'grep -qF' || true)"

# (b) comportemental : un findings-1-fd.md résiduel laissé par un dispatch
#     antérieur ne rend aucun terme vrai.
T2306_T11=$(_t2306_revise_probe "$T2306_FINDINGS_WITHOUT_FD" "no" "never" "residual-fd")
assert_eq "T11 (b): un findings-1-fd.md résiduel ne déclenche aucune relance" \
    "1" "$(_t2306_field 2 "$T2306_T11")"
assert_not_contains "T11 (b): et rien n'est journalisé" \
    "fire_disposition_revise_retried" "$T2306_T11"

# ============================================================================
# mika#2544 — les lecteurs `Fire-Disposition` tolèrent la numérotation (T12)
# ============================================================================
#
# `/ce:plan` numérote ses titres de section (`## 3. Fire-Disposition`). Le gate
# `Acceptance criteria` le tolère depuis mika#2516 ; les deux lecteurs FD de
# `_fd_retry_if_section_still_missing` ne le toléraient pas. Sur un plan
# CONFORME à titre numéroté, le rattrapage relançait un pilote de revise pour
# rien, avec un finding synthétique faux, et `fire_disposition_still_missing_after_retry`
# (régime attendu : zéro) était émis à tort — classe mika#2205, un instrument
# qui affirme un état qui n'a pas eu lieu.
#
# T12a est la fixture porteuse. T12e est obligatoire : avec le seul terme 2
# élargi, T12a passe au vert et le re-test de journalisation émet toujours
# l'événement faux. T12c/T12d sont les contrôles négatifs de la tolérance, et
# T7 (plus haut) reste celui de la forme non numérotée.

echo ""
echo 'Test: mika#2544 — lecteurs `Fire-Disposition` tolérant la numérotation (T12)'
echo "-----------------------------------------------------------------------------"

# --- T12a–T12g : comportement de la vraie fonction ---------------------------

T2544_T12A=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "numbered" "never")
assert_eq "T12a: findings réclamant FD + section NUMÉROTÉE ⇒ zéro relance" \
    "1" "$(_t2306_field 2 "$T2544_T12A")"
assert_not_contains "T12a: et rien n'est journalisé" \
    "fire_disposition_revise_retried" "$T2544_T12A"

T2544_T12B=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "upper" "never")
assert_eq "T12b: section numérotée en capitales ⇒ zéro relance (casse tolérée, D2)" \
    "1" "$(_t2306_field 2 "$T2544_T12B")"

# Contrôles négatifs : la tolérance n'a pas avalé les refus. Vus verts AVANT le
# correctif aussi — c'est ce qui prouve un élargissement et non un désarmement.
T2544_T12C=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "notes" "never")
assert_eq "T12c: seul '## 11. Notes on Fire-Disposition' ⇒ une relance (refus tenu)" \
    "2" "$(_t2306_field 2 "$T2544_T12C")"
T2544_T12D=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "subsub" "never")
assert_eq "T12d: '## 1.1 Fire-Disposition' ⇒ une relance (borne du préfixe assumée, D2)" \
    "2" "$(_t2306_field 2 "$T2544_T12D")"

# La moitié oubliée : le revise AJOUTE la section, numérotée. La relance est
# légitime (le plan initial ne la porte pas) ; l'événement d'échec, non.
T2544_T12E=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "no" "second-numbered")
assert_eq "T12e: section ajoutée numérotée à la relance ⇒ toujours une seule relance" \
    "2" "$(_t2306_field 2 "$T2544_T12E")"
assert_not_contains "T12e: et AUCUN fire_disposition_still_missing_after_retry (re-test de journalisation)" \
    "fire_disposition_still_missing_after_retry" "$T2544_T12E"

# Chaîne C2 : le plan porte déjà la section numérotée et l'architecte en
# critique le CONTENU — le terme 1 est vrai sans ITERATE préalable sur la
# section elle-même.
T2544_FINDINGS_FD_CONTENT="F1 [BLOQUANT] — la section Fire-Disposition nomme l'option (a) sans
assertion auto-nettoyante. Précise-la.

Disposition: ITERATE"
T2544_T12F=$(_t2306_revise_probe "$T2544_FINDINGS_FD_CONTENT" "numbered" "never")
assert_eq "T12f: findings critiquant le contenu + section numérotée ⇒ zéro relance" \
    "1" "$(_t2306_field 2 "$T2544_T12F")"

# Le coût de D3, rendu lisible : pas d'ancre de fin (alignée sur AC_HEADING_RE),
# donc un titre `## Fire-Disposition-ish` est lu comme la section.
T2544_T12G=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "ish" "never")
assert_eq "T12g: '## Fire-Disposition-ish' en tête de ligne ⇒ lu présent, zéro relance (D3)" \
    "1" "$(_t2306_field 2 "$T2544_T12G")"

# --- T12h : la matrice du motif, ligne par ligne ------------------------------
#
# Les cas négatifs du ticket (`###`, prose, backticks) et les bornes de D2, lus
# par la constante elle-même avec le drapeau des deux sites. Un motif vide
# matcherait tout : la non-vacuité est assertée d'abord.

assert_eq "T12h: _FD_HEADING_RE est définie et non vide" "non-vide" \
    "$([ -n "${_FD_HEADING_RE:-}" ] && echo non-vide || echo vide)"
_t2544_reads() {
    if [ -n "${_FD_HEADING_RE:-}" ] && grep -qiE -- "$_FD_HEADING_RE" <<<"$1"; then
        echo present
    else
        echo absent
    fi
}
while IFS='|' read -r t2544_want t2544_line; do
    assert_eq "T12h: « $t2544_line » ⇒ $t2544_want" "$t2544_want" "$(_t2544_reads "$t2544_line")"
done <<'T2544_MATRIX'
present|## Fire-Disposition
present|## 3. Fire-Disposition
present|## 3 Fire-Disposition
present|## 11. Fire-Disposition
present|## fire-disposition
present|##   Fire-Disposition
present|## Fire-Disposition (option a)
present|## Fire-Disposition-ish
absent|### Fire-Disposition
absent|# Fire-Disposition
absent|##Fire-Disposition
absent|la section `## Fire-Disposition` est requise
absent|une section Fire-Disposition-ish dans une ligne de prose
absent|## Notes on Fire-Disposition
absent|## 1.1 Fire-Disposition
T2544_MATRIX

# --- T12i : la clause de vocabulaire des deux gates architecte ---------------
#
# Assertion de PRÉSENCE, pas de comportement : le verdict est rendu par un LLM,
# que ce harnais ne peut pas exercer. Le contrat côté mika est « la clause est
# dans le prompt servi ».

for t2544_prompt in mika-arch-groom-ticket mika-arch-second-review; do
    t2544_pf="$SCRIPT_DIR/../$t2544_prompt/system_prompt.md"
    if [ -r "$t2544_pf" ]; then
        t2544_gate=$(sed -n '/^### Fire-Disposition Gate/,/^### /p' "$t2544_pf")
        assert_contains "T12i: $t2544_prompt — le gate FD dit qu'un titre numéroté est la section" \
            '`## 3. Fire-Disposition`' "$t2544_gate"
    else
        assert_eq "T12i: $t2544_prompt/system_prompt.md lisible" "lisible" "fichier introuvable: $t2544_pf"
    fi
done

# --- T12j : lecteur unique par section, et co-mutation FD ⇔ AC ---------------
#
# Le commentaire de `AC_HEADING_RE` promettait une garde de lecteur unique dans
# scripts/verify-pipeline-test.sh ; elle n'y était pas, et ce harnais-là n'est
# lancé par aucun job CI. Elle vit ici, dans le seul harnais shell câblé en CI,
# et couvre les deux sections.
#
# Prédicat POSITIONNEL, jamais lexical : une ligne de code (hors commentaire)
# qui invoque `grep`/`sed` ET porte, entre quotes, un littéral de titre
# (`#…<section>`). Les `echo "FAIL: … '## Acceptance criteria' …"` de
# verify-pipeline.sh et la prose de `_FIRE_DISPOSITION_RULE` citent le littéral
# sans être des lecteurs : ils sont hors population par construction. Le terme 1
# de la garde FD (`grep -qF -- 'Fire-Disposition'` sur les FINDINGS, sans `#`)
# l'est aussi — autre population, délibérément grossière.
#
# Borne nommée, pas découverte : la population est celle des lignes `grep`/`sed`.
# Un lecteur `awk`, `[[ =~ ]]`, `case`, un motif en variable ou sur une ligne de
# continuation échappe au scan. Aucun n'existe dans les deux fichiers ; le jour
# où l'un est écrit, on élargit l'alternance de commandes ici, dans le même
# commit.
#
# Contre-vacuité en deux moitiés : (i) au moins un lecteur via la constante par
# section ; (ii) un fichier illisible ROUGIT avec « fichier introuvable », au
# lieu de sauter comme le `if [ -f ]` de T2211 — un scan qui ne regarde rien se
# lit exactement comme un arbre propre (mika#2205).
#
# Rend : « introuvable », ou « <littéraux en position de motif>|<lecteurs via la constante> ».
_t2544_heading_scan() {
    local file="$1" section="$2" const="$3" code violations readers
    [ -r "$file" ] || { printf 'introuvable'; return 0; }
    code=$(grep -v '^[[:space:]]*#' "$file" \
        | grep -E '(^|[^[:alnum:]_-])(grep|sed)([[:space:]]|$)' || true)
    violations=$(printf '%s\n' "$code" \
        | grep -iE "['\"][^'\"]*#[^'\"]*${section}" | grep -c . || true)
    readers=$(printf '%s\n' "$code" | grep -F "\$${const}" | grep -c . || true)
    printf '%s|%s' "$violations" "$readers"
}

T2544_FD_SCAN=$(_t2544_heading_scan "$DISPATCH_LIB" "Fire-Disposition" "_FD_HEADING_RE")
T2544_AC_SCAN=$(_t2544_heading_scan "$REPO_ROOT/scripts/verify-pipeline.sh" "Acceptance criteria" "AC_HEADING_RE")

assert_eq "T12j (FD): dispatch-lib.sh lisible par le scan" "lu" \
    "$([ "$T2544_FD_SCAN" != introuvable ] && echo lu || echo "fichier introuvable: $DISPATCH_LIB")"
assert_eq "T12j (FD): aucun littéral de titre Fire-Disposition en position de motif" "0" \
    "${T2544_FD_SCAN%%|*}"
assert_eq "T12j (FD): exactement deux lecteurs, tous deux via \$_FD_HEADING_RE" "2" \
    "${T2544_FD_SCAN##*|}"
# Le drapeau combiné, pas `-E` seul : écrire `-qi` en remplaçant `-qE` perd
# l'ERE en silence, le motif ne matche plus rien et le rattrapage tire sur tout.
assert_eq "T12j (FD): les deux lecteurs portent -qiE" "2" \
    "$(grep -v '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF 'grep -qiE "$_FD_HEADING_RE"' || true)"

assert_eq "T12j (AC): verify-pipeline.sh lisible par le scan" "lu" \
    "$([ "$T2544_AC_SCAN" != introuvable ] && echo lu || echo "fichier introuvable: $REPO_ROOT/scripts/verify-pipeline.sh")"
assert_eq "T12j (AC): aucun littéral de titre Acceptance criteria en position de motif" "0" \
    "${T2544_AC_SCAN%%|*}"
assert_eq "T12j (AC): exactement deux lecteurs, tous deux via \$AC_HEADING_RE" "2" \
    "${T2544_AC_SCAN##*|}"

# Contre-vacuité (ii), exercée à chaque exécution plutôt qu'une fois à la main.
assert_eq "T12j contre-vacuité: un fichier absent rend « introuvable », pas zéro violation" \
    "introuvable" "$(_t2544_heading_scan "/nonexistent/mika-2544.sh" "Fire-Disposition" "_FD_HEADING_RE")"
# Et le prédicat positionnel mord : un troisième lecteur littéral est vu.
assert_eq "T12j contre-vacuité: un grep littéral '^## Fire-Disposition' est compté" "1|0" \
    "$(_t2544_heading_scan <(printf '%s\n' "    ! grep -qE '^## Fire-Disposition' \"\$p\" || return 0") \
        "Fire-Disposition" "_FD_HEADING_RE")"

# Co-mutation : aucun fichier n'est lisible par les deux gates au même instant
# (verify-pipeline.sh est lu depuis le dépôt, dispatch-lib.sh est une projection
# du binaire — mika#2340), donc le préfixe est doublé et tenu ici. Le jour où
# l'un des deux évolue, ce test rougit au lieu de laisser les gates rediverger.
T2544_AC_RE=$(sed -nE "s/^[[:space:]]*AC_HEADING_RE='(.*)'[[:space:]]*\$/\\1/p" \
    "$REPO_ROOT/scripts/verify-pipeline.sh" 2>/dev/null | head -1)
assert_eq "T12j (co-mutation): AC_HEADING_RE extraite de verify-pipeline.sh" "non-vide" \
    "$([ -n "$T2544_AC_RE" ] && echo non-vide || echo vide)"
T2544_FD_RE="${_FD_HEADING_RE:-}"
assert_eq "T12j (co-mutation): préfixe de _FD_HEADING_RE == préfixe de AC_HEADING_RE" \
    "${T2544_AC_RE%Acceptance criteria}" "${T2544_FD_RE%Fire-Disposition}"

# Fire-Disposition, option (a) — table d'exceptions du scan vide, vacuité
# assertée à l'exécution (modèle T2306_ARCH_ASK_ALLOWLIST). Quand le scan tire,
# on route le site vers la constante ; on n'ajoute pas de ligne ici (mika#2201).
T2544_HEADING_READER_ALLOWLIST=()
assert_eq "T12j: table d'exceptions du scan de lecteurs — zero entries" "0" \
    "${#T2544_HEADING_READER_ALLOWLIST[@]}"

# --- mika#2449: aucun message de récupération de stash ne nomme le checkout principal ---
#
# Deux sites de dispatch-lib stashent un worktree sale et impriment une
# consigne « recover with: git -C <dir> stash apply <sha> ». Le site A
# (_clean_worktree_for_rebase) nomme le worktree ; le site B (_set_up_worktree,
# relic non canonique) nommait `$SUB_REPO_DIR` — le checkout PRINCIPAL. Exécutée,
# cette consigne dépose le contenu non committé d'un worktree dans le checkout de
# déploiement, exactement la signature du sinistre du 2026-09-21 (staged +
# modified sur main, rebuild bloqué). Le lecteur de ce message est un acteur non
# contenu (opérateur, mika-dev lisant un dispatch en échec), donc une
# prescription écrite est un producteur latent.
#
# Scan de source à ALLOWLIST VIDE : toute ligne portant à la fois `stash apply`
# et `recover with` est un site ; aucun ne peut nommer SUB_REPO_DIR. Quand il
# tire, la résolution est de corriger le message, jamais d'ajouter une entrée.
# Contrôle de bonne foi : ≥ 2 sites trouvés, sinon le scan est vert parce qu'il
# ne regarde rien (classe mika#2205).
T2449_RECOVER_SITES=$(grep -n 'stash apply' "$DISPATCH_LIB" | grep 'recover with' || true)
T2449_SITE_COUNT=$(printf '%s\n' "$T2449_RECOVER_SITES" | grep -c 'recover with' || true)
assert_eq "mika#2449 bonne foi: le scan trouve ≥ 2 sites « recover with … stash apply »" "yes" \
    "$([ "$T2449_SITE_COUNT" -ge 2 ] && echo yes || echo "non ($T2449_SITE_COUNT)")"
assert_eq "mika#2449: aucun site « recover with » ne nomme SUB_REPO_DIR (allowlist vide)" "0" \
    "$(printf '%s\n' "$T2449_RECOVER_SITES" | grep -c 'SUB_REPO_DIR' || true)"
# Le site B doit nommer le worktree CANONIQUE, pas celui qu'il retire : ce
# dernier est supprimé quelques lignes plus bas (`worktree remove --force`), donc
# une consigne qui le nommerait échouerait au moment où l'opérateur la lit.
#
# mika#2976 a EXTRAIT ce site en `_stash_and_remove_worktree`, donc il ne nomme
# plus `$existing_wt` ni `$WORKTREE_DIR` : il porte deux paramètres, le worktree
# retiré (`$remove_dir`) et celui que l'indice nomme (`$recover_in`). L'invariant
# est inchangé — c'est son ancrage textuel qui a bougé, et les trois assertions
# d'appel ci-dessous le referment par l'autre bout : chaque appelant doit passer
# le worktree CANONIQUE en troisième argument.
T2449_SITE_B=$(printf '%s\n' "$T2449_RECOVER_SITES" | grep 'remove_dir' || true)
assert_contains "mika#2449: le site B (helper extrait) prescrit le worktree de récupération \$recover_in" \
    'git -C $recover_in stash apply' "$T2449_SITE_B"
assert_not_contains "mika#2449: le site B ne prescrit pas le worktree retiré \$remove_dir (supprimé plus bas)" \
    'git -C $remove_dir stash apply' "$T2449_SITE_B"
assert_contains "mika#2449: le site B dit explicitement de ne pas appliquer dans le checkout principal" \
    'NOT in the primary checkout' "$T2449_SITE_B"

# --- mika#2492: la PR nominale de la boucle n'est pas une épave ---
#
# Tout ticket groomé est dispatché sous `/ce-work <plan>` (override mika#1074),
# dont le périmètre est « implementation and local verification only, without
# the shipping tail ». Ce pilote n'ouvre donc JAMAIS de PR — c'est son périmètre,
# pas une troncature — et dispatch-lib classait pourtant sa PR en épave
# (`commit-pushed-no-pr`), ce qui armait trois gardes contre la PR même que la
# boucle existe pour produire.
#
# Le croisement a DEUX axes et une seule case change :
#   A périmètre (present/absent) × B session (conclue/tronquée)
# Les deux états de B existent dans la population mesurée du 2026-09-22 :
# #2425 (`[done] Success | 142 turns`) contre #2484 (`[guardrail]
# error_max_turns`), tous deux sous `/ce-work`.

echo ""
echo "Test mika#2492: classe no-shipping-tail — le chemin nominal cesse d'être une épave"
echo "---------------------------------------------------------------------------------"

# --- T1/T2/T3/T4 : le prédicat, sur ses quatre croisements (comportemental) ---
#
# Sonde pure : source la lib, pose les deux axes, interroge le prédicat.
# $1 = PILOT_SHIPPING_TAIL, $2 = STATUS, $3 = SKILL
_t2492_predicate() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        PILOT_SHIPPING_TAIL="$1"
        STATUS="$2"
        SKILL="$3"
        if _pilot_had_no_shipping_tail; then echo "yes"; else echo "no"; fi
    )
}

assert_eq "T1: périmètre absent + session conclue ⇒ classe nouvelle" \
    "yes" "$(_t2492_predicate absent success dev-pilot)"
assert_eq "T2 (contrôle négatif, axe B): périmètre absent + session tronquée ⇒ épave" \
    "no" "$(_t2492_predicate absent terminated dev-pilot)"
assert_eq "T3 (non-régression, axe A): périmètre présent + session conclue ⇒ inchangé" \
    "no" "$(_t2492_predicate present success dev-pilot)"
assert_eq "T3 (non-régression): périmètre présent + session tronquée ⇒ inchangé" \
    "no" "$(_t2492_predicate present terminated dev-pilot)"
assert_eq "T4 (fail-safe R4): estampille vide ⇒ comportement d'avant le ticket" \
    "no" "$(_t2492_predicate '' success dev-pilot)"
assert_eq "T4 (fail-safe R4): statut vide ⇒ comportement d'avant le ticket" \
    "no" "$(_t2492_predicate absent '' dev-pilot)"
assert_eq "T4 (fail-safe R4): dev-groom n'entre jamais dans la classe" \
    "no" "$(_t2492_predicate absent success dev-groom)"

# --- T7b/T8 : la garde a PRIS, et la ligne Outcome de la fenêtre (comportemental) ---
#
# Un scan de présence ne distingue pas « la garde est écrite » de « la garde est
# écrite dans le bon sens » : retirer le `!` de la conjonction du bloc mika#940
# Unit 1 est une régression d'UN caractère qui laisse T7a vert. Cette sonde
# exécute `_post_flight_recovery` sur un vrai dépôt et lit le RESULT produit.
#
# $1 = PILOT_SHIPPING_TAIL, $2 = STATUS. Rend le RESULT complet.
_t2492_recovery_probe() {
    local base_dir wt_dir pre_head
    base_dir=$(mktemp -d)
    wt_dir="$base_dir/worktree"

    git init -q "$wt_dir" 2>/dev/null
    echo "initial" > "$wt_dir/file.txt"
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "initial" 2>/dev/null
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)
    # Le pilote a commité : HEAD avance, worktree propre.
    echo "impl" > "$wt_dir/feature.rs"
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "fix(2492): the pilot's own implementation commit" 2>/dev/null

    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        # Stub : aucune sortie réseau depuis le harness. `gh` n'est jamais
        # invoqué via `command`, donc la fonction l'emporte.
        gh() { return 1; }

        PRE_RUN_HEAD="$pre_head"
        POST_RUN_HEAD=$(git -C "$wt_dir" rev-parse HEAD)
        WORKTREE_DIR="$wt_dir"
        SKILL="dev-pilot"
        REPO="mika"
        BRANCH="fix/2492/probe"
        ISSUE_NUM="2492"
        SESSION_ID="t2492"
        LOG_ID="t2492"
        RESULT="claude-pilot completed (status: ${2})."
        RESCUED_DIRTY_WORKTREE=0
        PILOT_SHIPPING_TAIL="$1"
        STATUS="$2"

        exec 9>&2
        _post_flight_recovery 2>/dev/null
        printf '%s' "$RESULT"
    )

    rm -rf "$base_dir"
}

T2492_NOMINAL=$(_t2492_recovery_probe absent success 2>/dev/null)
assert_not_contains "T7b: périmètre absent + conclue ⇒ AUCUN PIPELINE FAILURE (la garde a pris)" \
    "PIPELINE FAILURE:" "$T2492_NOMINAL"
assert_contains "T8: la fenêtre avant Path B porte un motif nommé, jamais UNKNOWN" \
    "Outcome: PIPELINE_INCOMPLETE — no_shipping_tail: dispatch-lib did not reach PR creation." \
    "$T2492_NOMINAL"
assert_not_contains "T8: et jamais le UNKNOWN générique (moins actionnable qu'aujourd'hui)" \
    "Outcome: UNKNOWN" "$T2492_NOMINAL"
assert_eq "T8: exactement une ligne Outcome dans la fenêtre" \
    "1" "$(printf '%s\n' "$T2492_NOMINAL" | grep -c '^Outcome: ' || true)"

# Contrôle négatif de la même sonde : le périmètre présent garde son
# PIPELINE FAILURE, mot pour mot. Sans lui, T7b ne distingue pas « la garde
# discrimine » de « le bloc mika#940 ne fire plus du tout ».
T2492_PRESENT=$(_t2492_recovery_probe present success 2>/dev/null)
assert_contains "T7b (contrôle négatif): périmètre présent ⇒ PIPELINE FAILURE inchangé" \
    "PIPELINE FAILURE: claude-pilot produced commits" "$T2492_PRESENT"
assert_contains "T3: et sa ligne Outcome reste PIPELINE_INCOMPLETE — manual recovery needed" \
    "Outcome: PIPELINE_INCOMPLETE — manual recovery needed." "$T2492_PRESENT"
assert_eq "T3: une seule ligne Outcome sur ce croisement aussi" \
    "1" "$(printf '%s\n' "$T2492_PRESENT" | grep -c '^Outcome: ' || true)"

# Le fail-safe, vu de bout en bout : estampille vide ⇒ chemin d'avant le ticket.
T2492_UNSTAMPED=$(_t2492_recovery_probe '' success 2>/dev/null)
assert_contains "T4 (bout en bout): estampille vide ⇒ PIPELINE FAILURE, comme avant" \
    "PIPELINE FAILURE: claude-pilot produced commits" "$T2492_UNSTAMPED"

# --- T8 (suite) : _set_outcome_line rend le contrat vrai par construction ---
_t2492_outcome_line_probe() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        RESULT="body line

Outcome: PIPELINE_INCOMPLETE — no_shipping_tail: dispatch-lib did not reach PR creation."
        _set_outcome_line "Outcome: PR_OPENED — https://example/pr/1"
        printf '%s' "$RESULT"
    )
}
T2492_OUTCOME=$(_t2492_outcome_line_probe 2>/dev/null)
assert_eq "T8: après réécriture, exactement une ligne Outcome" \
    "1" "$(printf '%s\n' "$T2492_OUTCOME" | grep -c '^Outcome: ' || true)"
assert_contains "T8: et c'est la plus vraie des deux" \
    "Outcome: PR_OPENED — https://example/pr/1" "$T2492_OUTCOME"
assert_not_contains "T8: l'ancienne valeur de la fenêtre a disparu" \
    "no_shipping_tail" "$T2492_OUTCOME"
assert_contains "T8: le corps du RESULT est préservé" \
    "body line" "$T2492_OUTCOME"

# --- T5 : l'estampille est écrite à UN site par valeur (scan, allowlist vide) ---
#
# « On déclare, on n'allowliste pas » (mika#2201) : quand ce scan tire, la
# résolution est de RETIRER le second site, jamais d'y ajouter une entrée. La
# lecture couvre l'autre moitié — qu'aucun chemin d'entrée n'atteigne la
# sélection de classe sans estamper — et aucun scan ne peut l'atteindre :
# `SKILL` a un site d'écriture unique, le `case` de `dispatch_claude_pilot` est
# exhaustif et fatal (`*) exit 1`), la suite est linéaire, et la fonction n'a
# que deux appelants (dev-pilot/handlers/run.sh, dev-groom/handlers/run.sh).
SHIPPING_TAIL_WRITERS_ALLOWED=""   # mika#2492: livrée VIDE, et un test le tient
assert_eq "T5: allowlist des écrivains de l'estampille — zero entries" \
    "" "$SHIPPING_TAIL_WRITERS_ALLOWED"
assert_eq "T5: PILOT_SHIPPING_TAIL=\"absent\" n'est écrit qu'à un seul site" \
    "1" "$(grep -c 'PILOT_SHIPPING_TAIL="absent"' "$DISPATCH_LIB" || true)"
assert_eq "T5: PILOT_SHIPPING_TAIL=\"present\" n'est écrit qu'à un seul site" \
    "1" "$(grep -c 'PILOT_SHIPPING_TAIL="present"' "$DISPATCH_LIB" || true)"
# Le site `absent` doit être celui de l'override — pas un site voisin qui
# devinerait le périmètre après coup.
T2492_DETECT_FN=$(sed -n '/^_detect_plan_on_branch() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "T5: le site « absent » est la ligne même qui pose l'override /ce-work" \
    'PILOT_SHIPPING_TAIL="absent"' "$T2492_DETECT_FN"
assert_contains "T5: et il est adjacent à l'override qu'il décrit" \
    'ENTRY_COMMAND="/ce-work $PLAN_PATH"' "$T2492_DETECT_FN"
# Un seul lecteur décisionnel de l'estampille : le prédicat. Tout autre lecteur
# ferait diverger deux jugements d'un même fait.
assert_eq "T5: l'estampille n'a qu'un lecteur décisionnel (le prédicat)" \
    "1" "$(grep -c 'PILOT_SHIPPING_TAIL:-' "$DISPATCH_LIB" || true)"

# --- T7a : la conjonction du bloc mika#940 Unit 1 porte la NÉGATION ---
#
# Cherchée sur la LIGNE de la conjonction, pas dans le fichier : à l'échelle du
# fichier, l'appel légitime — et sans `!` — de la sélection de classe satisfait
# un scan de présence naïf.
UNIT1_GUARD_EXEMPT=""   # mika#2492: livrée VIDE
assert_eq "T7a: allowlist d'exemption du guard Unit 1 — zero entries" \
    "" "$UNIT1_GUARD_EXEMPT"
T2492_UNIT1_LINE=$(grep -n 'STATUS" = "success" \] && \[ "\$SKILL" = "dev-pilot"' "$DISPATCH_LIB" | head -1)
assert_contains "T7a: la conjonction Unit 1 porte « ! _pilot_had_no_shipping_tail » (négation incluse)" \
    '! _pilot_had_no_shipping_tail' "$T2492_UNIT1_LINE"

# --- T6 : le bras de la classe nouvelle n'émet JAMAIS RECOVERY_PENDING ---
NO_TAIL_RECOVERY_PENDING_ALLOWED=""   # mika#2492: livrée VIDE
assert_eq "T6: allowlist RECOVERY_PENDING de la classe nouvelle — zero entries" \
    "" "$NO_TAIL_RECOVERY_PENDING_ALLOWED"
PATHB_2492=$(sed -n '/Unit 2 (mika#1282 + mika#1396): open a draft PR/,/^    _deliver_callback/p' "$DISPATCH_LIB")
# Ancré sur le `if` à douze espaces — l'`elif` du bras titre/fact porte la même
# comparaison et ouvrirait la plage trop tôt. Les commentaires sont retirés : le
# scan porte sur le CODE, et le commentaire de ce bras nomme légitimement le
# marqueur qu'il n'émet pas (même geste que GATE_CODE au test 15).
T2492_NOTAIL_ARM=$(printf '%s\n' "$PATHB_2492" \
    | sed -n '/^            if \[ "\$RECOVERY_CLASS" = "no-shipping-tail"/,/^            else$/p' \
    | grep -v '^[[:space:]]*#')
assert_contains "T6 (bonne foi): le bras de la classe nouvelle existe dans Path B" \
    'PR: ${PR_URL}' "$T2492_NOTAIL_ARM"
assert_not_contains "T6: et il n'émet jamais RECOVERY_PENDING: true (Guard 1 désarmée)" \
    "RECOVERY_PENDING: true" "$T2492_NOTAIL_ARM"
assert_contains "T6: il réécrit la ligne Outcome en PR_OPENED" \
    '_set_outcome_line "Outcome: PR_OPENED — ${PR_URL}"' "$T2492_NOTAIL_ARM"
# R3 : l'autre bras garde son marqueur, mot pour mot.
assert_contains "T3 (R3): le bras des épaves émet toujours RECOVERY_PENDING: true" \
    "RECOVERY_PENDING: true" "$PATHB_2492"

# --- Sélection de classe : le bras nouveau est AVANT l'épave, et gardé ---
T2492_SELECT=$(printf '%s\n' "$PATHB_2492" | sed -n '/local RECOVERY_CLASS=""/,/^    fi$/p')
assert_contains "U3b: le bras no-shipping-tail est gardé par le prédicat" \
    '_pilot_had_no_shipping_tail; then' "$T2492_SELECT"
assert_contains "U3b: la garde RESCUED_DIRTY_WORKTREE reste première" \
    'RESCUED_DIRTY_WORKTREE:-}" = "1" ]; then' "$T2492_SELECT"
T2492_NOTAIL_POS=$(printf '%s\n' "$T2492_SELECT" | grep -n 'RECOVERY_CLASS="no-shipping-tail"' | head -1 | cut -d: -f1)
T2492_WRECK_POS=$(printf '%s\n' "$T2492_SELECT" | grep -n 'RECOVERY_CLASS="commit-pushed-no-pr"' | head -1 | cut -d: -f1)
assert_eq "U3b: le bras no-shipping-tail précède le bras commit-pushed-no-pr" "yes" \
    "$([ -n "$T2492_NOTAIL_POS" ] && [ -n "$T2492_WRECK_POS" ] && [ "$T2492_NOTAIL_POS" -lt "$T2492_WRECK_POS" ] && echo yes || echo "non ($T2492_NOTAIL_POS vs $T2492_WRECK_POS)")"

# --- Le commit marqueur reste réservé aux épaves (égalité stricte) ---
# La borne de fin est `RESCUED_PR_URL=` seul : écrire le motif complet de
# l'invocation ferait de cette ligne une fausse positive du détecteur
# d'herméticité mika#2178 T9, qui balaie tout ce qui suit sa propre section.
T2492_MARKER_GUARD=$(printf '%s\n' "$PATHB_2492" | sed -n '/RECOVERY_CLASS" = "commit-pushed-no-pr"/,/^        RESCUED_PR_URL=/p')
assert_not_contains "U3c: le commit marqueur wip(mika#1383) n'atteint pas la classe nouvelle" \
    "no-shipping-tail" "$T2492_MARKER_GUARD"
# L'intention de cette assertion est que la réécriture en PR_OPENED n'ait qu'un
# site — c'est la valeur, non l'helper, qui doit être unique. Le prédicat comptait
# sur le fichier entier parce qu'il n'existait alors qu'un seul appel ; mika#2545
# en a ajouté un second, pour `Outcome: ESCALATE`, mika#2590 un troisième pour
# `Outcome: PLAN_GROOMED`, et mika#3142 un quatrième pour
# `Outcome: PIPELINE_INCOMPLETE — pilot_halt_declared`. Chaque valeur est donc
# nommée et comptée à un site, ce qui est plus fort que l'ancien compte global :
# une cinquième valeur écrite par un helper partagé fait rougir la cardinalité
# ci-dessous au lieu de passer inaperçue. **La cardinalité a mordu à l'arrivée de
# la troisième, puis de la quatrième** — c'est ce qui la rend crédible, et le
# geste juste est de nommer la valeur neuve, jamais de relâcher le compte.
assert_eq "U3c: la réécriture Outcome: PR_OPENED n'est appelée qu'au seul bras de la classe nouvelle" \
    "1" "$(grep -c '_set_outcome_line "Outcome: PR_OPENED' "$DISPATCH_LIB" || true)"
assert_eq "U3c (mika#2545): Outcome: ESCALATE n'est estampillé qu'à un site" \
    "1" "$(grep -c '_set_outcome_line "Outcome: ESCALATE' "$DISPATCH_LIB" || true)"
# mika#2981 — PLAN_GROOMED est la seule des trois valeurs qui ne soit PAS passée
# littéralement à `_set_outcome_line` : sa ligne porte depuis ce ticket un
# suffixe ` — <plan>` nommant le plan que le groom a validé, composé dans
# `_groomed_outcome` parce qu'il est conditionnel (repli nu sur le bras
# `canonical_callout_failed`). La cardinalité de la VALEUR est donc comptée à son
# site de composition, et elle reste de 1 : le geste juste est de suivre la
# valeur là où elle est écrite, jamais de relâcher le compte.
assert_eq "U3c (mika#2590): Outcome: PLAN_GROOMED n'est estampillé qu'à un site" \
    "1" "$(grep -c '_groomed_outcome="Outcome: PLAN_GROOMED' "$DISPATCH_LIB" || true)"
# mika#3142 — le refus de halte nomme son motif dans la ligne d'outcome elle-même
# (`PIPELINE_INCOMPLETE — pilot_halt_declared`), posée au seul site du troisième
# refus structurel du bloc Unit 2. Comptée comme les trois autres : sur la valeur.
assert_eq "U3c (mika#3142): Outcome: PIPELINE_INCOMPLETE n'est estampillé qu'à un site" \
    "1" "$(grep -c '_set_outcome_line "Outcome: PIPELINE_INCOMPLETE' "$DISPATCH_LIB" || true)"
assert_eq "U3c: et _set_outcome_line n'a que ces quatre valeurs déclarées" \
    "4" "$(grep -cE '_set_outcome_line "Outcome:|_groomed_outcome="Outcome:' "$DISPATCH_LIB" || true)"

# --- D5 : la classe nouvelle passe par le même producteur de marqueur ---
assert_contains "D5: _measure_pipeline_verified couvre toutes les classes de Path B" \
    '_measure_pipeline_verified "$WORKTREE_DIR"' "$PATHB_2492"
# D3 : le label est CONSERVÉ — le retirer sortirait la PR de wip_rescue.rs,
# seul mécanisme qui la rebase, la passe à clippy et la sort du draft, sans que
# qa_review_reconcile (qui exige isDraft == false) la rattrape.
assert_contains "D3: le label wip-rescue reste appliqué (sinon la PR sort de tous les filets)" \
    'add-label "wip-rescue"' "$PATHB_2492"

# --- Le titre de la classe nouvelle vient du vrai commit d'implémentation ---
_t2492_title_probe() {
    local base_dir wt_dir
    base_dir=$(mktemp -d); wt_dir="$base_dir/wt"
    git init -q "$wt_dir" 2>/dev/null
    echo x > "$wt_dir/a"; git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "fix(2492): le vrai sujet du commit du pilote" 2>/dev/null
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        _derive_recovery_pr_title "no-shipping-tail" "$wt_dir" "mika" "2492" "" "titre d'issue"
    )
    rm -rf "$base_dir"
}
assert_eq "U3c: le titre de la classe nouvelle est le sujet du commit d'implémentation" \
    "fix(2492): le vrai sujet du commit du pilote" "$(_t2492_title_probe 2>/dev/null)"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
