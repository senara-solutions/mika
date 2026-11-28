#!/bin/bash
# Les notes de halte et leurs jetons de cause — extrait de test-dispatch-lib.sh par mika#3178, vol 4.
# Un deny non-terminal survécu est une NOTE, jamais un « halted » (mika#2493) ;
# le jeton de cause qui DÉRIVE de `_halt_family` au lieu de la redéclarer
# (mika#2539) ; le quatrième classificateur `_classify_revise_halt` et les deux
# sessions distinctes qu'ils classent (mika#2077) ; l'ESCALATE de groom terminal
# estampillé par son producteur (mika#2545) ; le Signal S de mika#2050 ; le site
# unique de déclaration du chemin de staging du jeton (mika#2578 D2).
#
# Plage d'origine : lignes 10312–11204 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 3849d757. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# `_gate_probe` (agrégat l. 6858, famille 8) : lue par les cinq sondes mika#2545
# ci-dessous. Sans elle, la suite meurt en `command not found` sans imprimer son
# bloc `Results:` — mesuré au groom du vol 4.
# shellcheck source=lib-dispatch-lib-shared-probes-fixtures.sh
source "$_SUITE_DIR/lib-dispatch-lib-shared-probes-fixtures.sh"
# =============================================================================
# mika#2493 — un deny non-terminal survécu est une NOTE, jamais un « halted »
# =============================================================================
#
# Ce que ces détecteurs mesurent. Le champ `result` préfixait « PIPELINE
# FAILURE: … halted by policy deny » alors que le refus n'avait pas arrêté la
# session et qu'elle avait livré. Le libellé a fait conclure « échec » à tort
# deux fois — opérateur ET orchestrateur — dans l'incident de la nuit du
# 2026-09-22. Preuves re-mesurées (M0) : sessions `98b60020` (2 refus) et
# `a0886164` (5 refus), **zéro terminal**, toutes deux `status: success`.
#
# Trois unités, trois populations de test :
#   U1 — la garde `[ -z "$VALID_PLAN" ]` (T8, T9, T15, T16)
#   U2 — le prédicat de létalité + la note annexée (T1–T7, T10, T11)
#   U3 — la dé-troncature de l'événement rapporté (T13, T14)
#
# Herméticité : aucune fixture ne touche le réseau ni la forge. Les deux sondes
# bout-en-bout redéfinissent `_pr_list_url` dans leur sous-shell.

echo ""
echo "Test mika#2493 : le libellé suit la létalité du deny"
echo "----------------------------------------------------"

T2493_FIXTURE_DIR=$(mktemp -d)

# --- T1–T7 : le prédicat de létalité ----------------------------------------
#
# D2 : le prédicat porte sur le FICHIER, jamais sur la ligne capturée. Trois
# raisons mesurées — le marqueur sort de la première ligne dans 23 % des cas
# (M2, dont la preuve `98b60020`) ; une session porte plusieurs refus ; et un
# refus terminal termine sa session (M4), donc « au moins un terminal » et « le
# dernier est terminal » coïncident, la première formulation seule survivant à
# la troncature.

_t2493_lethality() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        _policy_deny_lethality "$1"
    )
}

# T1 — refus non-terminal mono-ligne (forme la plus fréquente : 1093 des 1185
# marqueurs mesurés depuis cpp#151).
printf '\x1b[31m[policy:deny]\x1b[0m Bash: env | head -3 [bash-env] (non-terminal)\n' \
    > "$T2493_FIXTURE_DIR/nonterm.stderr"
assert_eq "T1: refus non-terminal mono-ligne ⇒ non-terminal" \
    "non-terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/nonterm.stderr")"

# T2 — refus terminal. La population que le verbe « halted » doit garder.
printf '[policy:deny] Bash: rm -rf /etc [bash-destructive] (terminal)\n' \
    > "$T2493_FIXTURE_DIR/term.stderr"
assert_eq "T2: refus terminal ⇒ terminal" \
    "terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/term.stderr")"

# T3 — forme antérieure à cpp#151 (2026-09-04) : aucun marqueur nulle part.
# D3 : rien n'est affirmé. Replier sur `non-terminal` poserait l'affirmation
# fausse dans l'autre sens ; replier sur `terminal` reconduirait le défaut.
printf '[policy:deny] Bash: gh auth status 2>&1 | head -10\n' \
    > "$T2493_FIXTURE_DIR/undeclared.stderr"
assert_eq "T3: refus sans marqueur (pré-cpp#151) ⇒ undeclared" \
    "undeclared" "$(_t2493_lethality "$T2493_FIXTURE_DIR/undeclared.stderr")"

# T4 — LE test qui sépare « lit le fichier » de « lit la ligne ». C'est la forme
# exacte de la preuve `98b60020` du ticket : `<detail>` multi-ligne, marqueur en
# 3ᵉ ligne. Toute implémentation qui lirait la ligne capturée par un
# `grep -m1` rougit ici — et serait donc fausse sur une des deux preuves.
printf '%s\n' \
    '[2026-09-22T17:35:39.081Z] [policy:deny] Bash: cd /data/workspace/mika-platform/.claude/worktrees/test-2471/mika \' \
    '  && git log --oneline -5 \' \
    '  && echo done [bash-cd] (non-terminal)' \
    > "$T2493_FIXTURE_DIR/multiline.stderr"
assert_eq "T4: refus multi-ligne, marqueur en 3ᵉ ligne (forme 98b60020) ⇒ non-terminal" \
    "non-terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/multiline.stderr")"

# T5 — plusieurs refus : la question est « un terminal existe-t-il ? », pas
# « le premier l'était-il ? ». `a0886164` en porte cinq.
printf '%s\n' \
    '[policy:deny] Bash: a [r1] (non-terminal)' \
    '[policy:deny] Bash: b [r2] (non-terminal)' \
    '[policy:deny] Bash: c [r3] (non-terminal)' \
    '[policy:deny] Bash: d [r4] (non-terminal)' \
    '[policy:deny] Bash: e [r5] (terminal)' \
    > "$T2493_FIXTURE_DIR/four_then_term.stderr"
assert_eq "T5: quatre non-terminaux suivis d'un terminal ⇒ terminal" \
    "terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/four_then_term.stderr")"

# T6 — CONTRÔLE NÉGATIF. `(non-terminal)` ne contient pas la sous-chaîne
# `(terminal)` : la parenthèse ouvrante qu'elle exige est occupée par le `-`.
# Inverser cette discrimination reclasserait d'un coup les 1093 refus
# non-terminaux mesurés — c'est l'erreur que ce test existe pour attraper, et
# elle ne se voit sur aucune des fixtures positives ci-dessus.
printf '%s\n' \
    '[policy:deny] Bash: x [r1] (non-terminal)' \
    '[policy:deny] Bash: y [r2] (non-terminal)' \
    > "$T2493_FIXTURE_DIR/only_nonterm.stderr"
assert_eq "T6 (contrôle négatif): un fichier de non-terminaux ne rend JAMAIS terminal" \
    "non-terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/only_nonterm.stderr")"
# Et la vérification directe de la propriété de sous-chaîne dont tout dépend.
assert_eq "T6: la sous-chaîne (terminal) n'est pas contenue dans (non-terminal)" \
    "1" "$(printf '%s\n' 'x (non-terminal)' 'y (terminal)' | grep -c -F '(terminal)' || true)"

# T7 — fail-open préservé, dans les deux formes d'indisponibilité.
assert_eq "T7: fichier absent ⇒ undeclared (fail-open)" \
    "undeclared" "$(_t2493_lethality "$T2493_FIXTURE_DIR/does_not_exist.stderr")"
printf '[policy:deny] Bash: z [r] (terminal)\n' > "$T2493_FIXTURE_DIR/unreadable.stderr"
chmod 000 "$T2493_FIXTURE_DIR/unreadable.stderr" 2>/dev/null || true
if [ -r "$T2493_FIXTURE_DIR/unreadable.stderr" ]; then
    # root ignore les bits de permission : la sonde ne peut pas s'armer, et le
    # dire est ce qui empêche de lire un vert à vide (mika#2149).
    SKIPPED=$((SKIPPED + 1))
    echo "  SKIP T7: fichier illisible — le processus lit malgré chmod 000 (root ?)"
else
    assert_eq "T7: fichier illisible ⇒ undeclared (fail-open)" \
        "undeclared" "$(_t2493_lethality "$T2493_FIXTURE_DIR/unreadable.stderr")"
fi
chmod 644 "$T2493_FIXTURE_DIR/unreadable.stderr" 2>/dev/null || true

# --- T13–T14 : l'événement rapporté est dé-tronqué et borné (U3) -------------
#
# U3 est la condition de VÉRIFIABILITÉ de U2 : sans lui, le message affirme
# « non-terminal » en joignant une preuve où le marqueur n'apparaît pas (23 %
# des cas) — un troisième « croire sur parole » dans un ticket dont le sujet est
# un libellé qu'on a cru sur parole.

_t2493_excerpt() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        sed 's/\x1b\[[0-9;]*[mK]//g' "$1" | _policy_deny_excerpt
    )
}

T2493_EXCERPT_MULTILINE=$(_t2493_excerpt "$T2493_FIXTURE_DIR/multiline.stderr")
assert_contains "T13: l'extrait porte le [rule-id] (perdu par grep -m1)" \
    "[bash-cd]" "$T2493_EXCERPT_MULTILINE"
assert_contains "T13: l'extrait porte le marqueur de létalité" \
    "(non-terminal)" "$T2493_EXCERPT_MULTILINE"
assert_contains "T13: et il porte toujours la première ligne du refus" \
    "[policy:deny] Bash: cd /data/workspace" "$T2493_EXCERPT_MULTILINE"
# Contrôle de bonne foi : ce que l'ancien prédicat rendait sur cette même
# fixture. Si ce test rougit, c'est que la fixture ne reproduit plus la forme
# de M2 et que T13 ne mesure plus rien.
assert_not_contains "T13 (bonne foi): grep -m1 sur la même fixture PERD le rule-id" \
    "[bash-cd]" "$(sed 's/\x1b\[[0-9;]*[mK]//g' "$T2493_FIXTURE_DIR/multiline.stderr" | grep -m1 '\[policy:deny\]' || true)"

# T14 — bornes de D6 : un refus mono-ligne sans marqueur suivi de bruit de
# journal s'arrête à la première ligne ouvrant un autre événement.
printf '%s\n' \
    '[policy:deny] Bash: gh auth status' \
    '[debug] tool_use id=abc' \
    '[debug] tool_result bytes=1200' \
    '[done] Success | 5 turns' \
    > "$T2493_FIXTURE_DIR/deny_then_noise.stderr"
T2493_EXCERPT_NOISE=$(_t2493_excerpt "$T2493_FIXTURE_DIR/deny_then_noise.stderr")
assert_contains "T14: l'extrait porte le refus" \
    "[policy:deny] Bash: gh auth status" "$T2493_EXCERPT_NOISE"
assert_not_contains "T14: et n'emporte pas le bruit [debug] qui suit" \
    "[debug]" "$T2493_EXCERPT_NOISE"
assert_not_contains "T14: ni la ligne de fin de session" \
    "[done]" "$T2493_EXCERPT_NOISE"
# Plafond : sans marqueur ET sans autre événement, la capture reste bornée.
{
    printf '[policy:deny] Bash: long-command \\\n'
    for _i in $(seq 1 40); do printf '  arg%s \\\n' "$_i"; done
} > "$T2493_FIXTURE_DIR/deny_unbounded.stderr"
assert_eq "T14: sans marqueur ni autre événement, la capture est plafonnée" \
    "12" "$(_t2493_excerpt "$T2493_FIXTURE_DIR/deny_unbounded.stderr" | grep -c . || true)"

# --- mika#3040 : l'extrait cite l'événement qui a TUÉ la session -------------
#
# Le site annonce « The deny's lethality marker says (terminal): it ended the
# session », puis joint le PREMIER refus du flux. Les deux lectures portent sur
# des objets différents — la létalité sur le fichier entier, l'extrait sur le
# premier événement — et elles divergent dès qu'une session porte plusieurs
# refus. Rejeu sur les 3162 `.stderr` du 2026-05-15 au 2026-11-03 : 118 sessions
# portent un événement de refus terminal, et 98 d'entre elles citaient un
# événement NON terminal sous le mot « Halt ».
#
# Le défaut vivait exactement dans la case que le produit cartésien des fixtures
# ci-dessus laisse vide : T5 porte cinq refus dont un terminal, mais n'interroge
# que la létalité ; T13 et T14 interrogent l'extrait, mais sur des refus
# uniques. La paire « plusieurs refus ET extrait » est ci-dessous.

# La forme exacte du cas d'origine, groom `e71ada95` (mika#3030) : un refus
# `cause=default-deny` non terminal à 19:53:52Z, du bruit de journal, puis le
# refus tier3 `[bash-grep]` terminal à 19:59:41Z qui a réellement fini la
# session. Aucune ligne de continuation n'ouvre par `[` : la borne D6 s'applique
# aux continuations comme au reste, et une fixture qui l'ignorerait mesurerait
# une forme que le corpus ne porte pas.
printf '%s\n' \
    '[2026-11-03T19:53:52.000Z] [policy:deny] Bash: git -C /data/workspace/mika-platform/claude-pilot rev-parse --git-dir \' \
    '  cause=default-deny (non-terminal)' \
    '[debug] tool_use id=abc' \
    '[2026-11-03T19:59:41.000Z] [policy:deny] Bash: grep -c "bash -c" ~/.mika/data/pilot-transcripts/*.jsonl \' \
    '  | sort -rn \' \
    '  cause=tier3 [bash-grep] (terminal)' \
    '[done] Halted | 37 turns' \
    > "$T2493_FIXTURE_DIR/two_denies.stderr"
# Bonne foi AVANT les assertions : la fixture reproduit bien la forme mesurée.
# Si sa létalité n'était pas `terminal`, le site n'annoncerait pas « it ended
# the session » et tout ce qui suit ne mesurerait plus rien.
assert_eq "AC1 (bonne foi): la fixture à deux refus a bien une létalité terminal" \
    "terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/two_denies.stderr")"
T3040_EXCERPT_TWO=$(_t2493_excerpt "$T2493_FIXTURE_DIR/two_denies.stderr")
assert_contains "AC1: l'extrait cite le [rule-id] du refus TERMINAL" \
    "[bash-grep]" "$T3040_EXCERPT_TWO"
assert_contains "AC1: et le marqueur cité concorde avec la létalité annoncée" \
    "(terminal)" "$T3040_EXCERPT_TWO"
assert_not_contains "AC1: le premier refus, non terminal, n'est PAS celui qu'on cite" \
    "rev-parse --git-dir" "$T3040_EXCERPT_TWO"
assert_not_contains "AC1: et son marqueur non terminal n'apparaît pas" \
    "(non-terminal)" "$T3040_EXCERPT_TWO"
assert_not_contains "AC1: les bornes d'événement tiennent entre deux refus ([debug])" \
    "[debug]" "$T3040_EXCERPT_TWO"
assert_not_contains "AC1: ni la ligne de fin de session ([done])" \
    "[done]" "$T3040_EXCERPT_TWO"
# L'ÉVÉNEMENT TERMINAL EN ENTIER, à l'octet. Assertion d'égalité et non de
# contenance, pour deux raisons : les continuations du terminal doivent être
# emportées (sans quoi le [rule-id] et le marqueur se reperdent, le défaut même
# de mika#2493), et elles doivent rester indentées de 4 espaces — D5 :
# l'indentation neutralise les jetons de reclassification ancrés en début de
# ligne. Les continuations de la fixture portent déjà 2 espaces, d'où 6 ici.
#
# `assert_contains` ne peut pas tenir cette moitié : son `grep -qF` traite une
# aiguille multi-ligne comme PLUSIEURS motifs, et la ligne vide d'une aiguille
# qui ouvre par un saut de ligne matche n'importe quoi — l'assertion passerait
# au vert sur l'extrait d'aujourd'hui (mesuré).
T3040_EXPECTED_TWO=$(printf '%s\n' \
    '[2026-11-03T19:59:41.000Z] [policy:deny] Bash: grep -c "bash -c" ~/.mika/data/pilot-transcripts/*.jsonl \' \
    '      | sort -rn \' \
    '      cause=tier3 [bash-grep] (terminal)')
assert_eq "AC1: l'extrait EST l'événement terminal entier, continuations indentées" \
    "$T3040_EXPECTED_TWO" "$T3040_EXCERPT_TWO"

# AC2, premier contrôle négatif — un refus unique NON terminal : l'extrait est
# inchangé, parce que le candidat de repli a servi (aucun événement du flux ne
# porte `(terminal)`). La valeur attendue est lue sur la FIXTURE et non réécrite
# ici : sur un refus mono-ligne l'événement EST sa première ligne, donc
# `grep -m1` en est la référence — propriété que T13 montre fausse dès que le
# refus est multi-ligne.
assert_eq "AC2: un refus unique non terminal ⇒ extrait inchangé (repli)" \
    "$(sed 's/\x1b\[[0-9;]*[mK]//g' "$T2493_FIXTURE_DIR/nonterm.stderr" | grep -m1 '\[policy:deny\]' || true)" \
    "$(_t2493_excerpt "$T2493_FIXTURE_DIR/nonterm.stderr")"

# AC2, second contrôle négatif — un refus unique TERMINAL : extrait inchangé.
assert_eq "AC2: un refus unique terminal ⇒ extrait inchangé" \
    "$(grep -m1 '\[policy:deny\]' "$T2493_FIXTURE_DIR/term.stderr" || true)" \
    "$(_t2493_excerpt "$T2493_FIXTURE_DIR/term.stderr")"

# Et la même chose sur une forme MULTI-LIGNE. Sans cette fixture, le candidat
# terminal n'est jamais exercé sur un événement à continuations : une
# implémentation qui ne retiendrait que la première ligne du terminal — celle
# qui perd le [rule-id] et le marqueur, le défaut de mika#2493 — passerait.
printf '%s\n' \
    '[policy:deny] Bash: grep -rn "token" /etc/shadow /etc/sudoers \' \
    '  --include="*" \' \
    '  cause=tier3 [bash-sensitive-read] (terminal)' \
    > "$T2493_FIXTURE_DIR/term_multiline.stderr"
T3040_EXCERPT_TERM_ML=$(_t2493_excerpt "$T2493_FIXTURE_DIR/term_multiline.stderr")
T3040_EXPECTED_TERM_ML=$(printf '%s\n' \
    '[policy:deny] Bash: grep -rn "token" /etc/shadow /etc/sudoers \' \
    '      --include="*" \' \
    '      cause=tier3 [bash-sensitive-read] (terminal)')
assert_eq "AC2: terminal multi-ligne — l'événement entier, première ligne à fleur" \
    "$T3040_EXPECTED_TERM_ML" "$T3040_EXCERPT_TERM_ML"

# --- T8 : garde de préemption, formulée sur la POPULATION des sites ----------
#
# Formulé sur la population — toute branche qui déclare un halt sur un deny —
# et non sur deux lignes nommées, pour qu'un troisième site futur soit vu.
# M5 est la mesure qui requalifie U1 : les trois autres branches de la chaîne
# du site dev-groom portaient DÉJÀ `[ -z "$VALID_PLAN" ]`. La branche de refus
# était la seule à ne pas la porter — et c'est elle qui est en tête. U1 rend à
# cette branche la condition que ses sœurs ont ; il n'en invente pas une.
#
# « On déclare, on n'allowliste pas » (mika#2201, mika#2323) : quand ce scan
# tire, la résolution est d'ajouter la garde au site fautif, JAMAIS d'ajouter
# une entrée ici.
POLICY_DENY_UNGUARDED_ALLOWED=""   # mika#2493 : LIVRÉE VIDE, et un test le tient
assert_eq "T8: allowlist des sites de halt sans garde — zero entries" \
    "" "$POLICY_DENY_UNGUARDED_ALLOWED"

# Le scan associe chaque libellé de halt à la condition de la branche qui
# l'englobe, et vérifie que cette condition porte les DEUX gardes. La portée de
# `cond` est bornée à 15 lignes pour qu'un libellé lointain n'hérite pas de la
# condition d'un autre bloc.
T2493_HALT_SCAN=$(awk '
    /^[[:space:]]*(el)?if .*\$POLICY_DENY/ { cond = $0; age = 0; next }
    cond != "" { age++; if (age > 15) cond = "" }
    /RESULT="PIPELINE FAILURE:.*halted by.*policy deny/ {
        seen++
        if (cond !~ /-z "\$VALID_PLAN"/) bad_plan++
        if (cond !~ /POLICY_DENY_LETHALITY" = "terminal"/) bad_leth++
    }
    END { printf "%d:%d:%d", seen + 0, bad_plan + 0, bad_leth + 0 }
' "$DISPATCH_LIB")
# Bonne foi d'abord : un scan qui ne trouve aucun site passerait vert à vide.
assert_eq "T8 (bonne foi): le scan voit les deux sites de halt existants" \
    "yes" "$([ "${T2493_HALT_SCAN%%:*}" -ge 2 ] && echo yes || echo "non ($T2493_HALT_SCAN)")"
assert_eq "T8: tout site de halt sur deny porte la garde [ -z \"\$VALID_PLAN\" ]" \
    "0" "$(printf '%s' "$T2493_HALT_SCAN" | cut -d: -f2)"
assert_eq "T8: tout site de halt sur deny est réservé à la létalité terminal" \
    "0" "$(printf '%s' "$T2493_HALT_SCAN" | cut -d: -f3)"

# L'ORDRE DES CONJOINTS est porteur (D1) : les deux assertions d'ordre
# existantes (Test 13, Test 14) cherchent la sous-chaîne littérale
# `if [ -n "$POLICY_DENY" ]`. Mettre la garde en tête les ferait rougir pour un
# résultat identique — et donnerait l'apparence d'un fix qui corrige ses tests.
assert_eq "D1: les deux sites gardent POLICY_DENY en conjoint de tête" \
    "2" "$(grep -c 'if \[ -n "\$POLICY_DENY" \] && \[ -z "\$VALID_PLAN" \]' "$DISPATCH_LIB" || true)"

# --- mika#3040 AC3 : un seul site construit le couple (extrait, létalité) ----
#
# Le défaut de mika#3040 n'était dans aucune des deux fonctions — chacune
# répondait juste à sa propre question. Il était dans le fait que DEUX sites
# posaient les deux questions séparément, en copies quasi-littérales l'un de
# l'autre : c'est cette duplication qui était la condition matérielle de la
# divergence. `_policy_deny_read` la supprime ; ce scan refuse qu'elle revienne.
#
# Même patron que T8 ci-dessus : une allowlist déclarée vide et tenue vide par
# une assertion, un scan structurel, et un contrôle de bonne foi. « On déclare,
# on n'allowliste pas » (mika#2201, mika#2323) : quand ce scan tire, la
# résolution est de router le nouveau site par `_policy_deny_read`, JAMAIS
# d'ajouter une entrée ici. Une exception ne peut d'ailleurs pas devenir stale,
# parce qu'elle ne peut pas exister : toute entrée rougit l'assertion suivante
# au premier run.
POLICY_DENY_CONSTRUCTION_SITES_ALLOWED=""   # mika#3040 : LIVRÉE VIDE, et un test le tient
assert_eq "AC3: allowlist des sites de construction — zero entries" \
    "" "$POLICY_DENY_CONSTRUCTION_SITES_ALLOWED"

# Les commentaires et les trois lignes de DÉFINITION sont exclus : on compte les
# APPELS. Les trois premières mesures doivent toutes tomber dans
# `_policy_deny_read`, la quatrième est ce qui empêche le scan de mesurer le
# vide — un fichier où plus aucun site ne lirait le refus rendrait `0:0:0` et
# passerait vert à vide, le mode de panne que mika#2578 et mika#2752 ont payé.
T3040_READ_SCAN=$(awk '
    /^[[:space:]]*#/ { next }
    index($0, "_policy_deny_excerpt() {") > 0 { next }
    index($0, "_policy_deny_lethality() {") > 0 { next }
    index($0, "_policy_deny_read() {") > 0 { next }
    index($0, "POLICY_DENY=$(") > 0 { ctor++ }
    index($0, "_policy_deny_excerpt") > 0 { exc++ }
    index($0, "_policy_deny_lethality") > 0 { leth++ }
    index($0, "_policy_deny_read") > 0 { rd++ }
    END { printf "%d:%d:%d:%d", ctor + 0, exc + 0, leth + 0, rd + 0 }
' "$DISPATCH_LIB")
# Bonne foi d'abord, comme T8 : le scan voit bien les deux sites d'appel.
assert_eq "AC3 (bonne foi): le scan voit au moins deux appels de _policy_deny_read" \
    "yes" "$([ "${T3040_READ_SCAN##*:}" -ge 2 ] && echo yes || echo "non ($T3040_READ_SCAN)")"
assert_eq "AC3: une seule ligne construit POLICY_DENY par substitution" \
    "1" "$(printf '%s' "$T3040_READ_SCAN" | cut -d: -f1)"
assert_eq "AC3: _policy_deny_excerpt n'est appelé qu'une fois (jamais sans la létalité)" \
    "1" "$(printf '%s' "$T3040_READ_SCAN" | cut -d: -f2)"
assert_eq "AC3: _policy_deny_lethality n'est appelé qu'une fois (jamais dissocié)" \
    "1" "$(printf '%s' "$T3040_READ_SCAN" | cut -d: -f3)"

# --- T9, T10, T11, T15, T16 : comportement bout-en-bout ---------------------
#
# Les deux sens du DoD, sur la forme exacte des deux preuves : dev-groom,
# HEAD inchangé, un refus dans le stderr — et `VALID_PLAN` peuplé ou vide.

_t2493_e2e() {
    # $1 = chemin d'une fixture stderr ; $2 = "plan" pour peupler VALID_PLAN.
    local base_dir wt_dir log_dir pre_head
    base_dir=$(mktemp -d); wt_dir="$base_dir/wt"; log_dir="$base_dir/logs"
    mkdir -p "$log_dir" "$wt_dir/docs/plans"
    git init -q "$wt_dir" 2>/dev/null
    printf 'seed\n' > "$wt_dir/seed.txt"
    if [ "${2:-}" = "plan" ]; then
        # >500 octets (seuil mika#1033) + en-tête nommant l'issue (tier 2).
        {
            printf '# Plan mika issue#9493\n\n**Ticket:** mika issue#9493\n\n'
            for _i in $(seq 1 40); do printf 'Ligne de corps du plan, pour franchir le seuil de 500 octets.\n'; done
        } > "$wt_dir/docs/plans/2026-09-23-001-fix-9493-sonde-plan.md"
    fi
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "seed" 2>/dev/null
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)
    cp "$1" "$log_dir/t2493probe.stderr"

    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB"

        # Herméticité : aucune requête forge. Le détecteur mika#2178 T9 balaie
        # tout ce qui suit sa section — ces redéfinitions sont ce qui permet à
        # la sonde de tourner sans réseau ni jeton.
        _pr_list_url() { _LAST_PR_QUERY_RC=0; printf ''; }
        _stamp_pr_origin() { return 0; }

        PILOT_LOG_DIR="$log_dir"
        PRE_RUN_HEAD="$pre_head"
        POST_RUN_HEAD="$pre_head"     # HEAD inchangé — la forme des deux preuves
        WORKTREE_DIR="$wt_dir"
        SKILL="dev-groom"
        REPO="mika"
        BRANCH="fix/9493/sonde"
        ISSUE_NUM="9493"
        SESSION_ID="sonde-2493"
        LOG_ID="t2493probe"
        STATUS="success"
        PILOT_EXIT=0
        RESULT="claude-pilot session completed."
        RESCUED_DIRTY_WORKTREE=0

        exec 9>&2
        _post_flight_recovery 2>/dev/null

        printf '%s' "$RESULT"
    )

    rm -rf "$base_dir"
}

# T9 — DoD point 1. La population fondatrice : dev-groom qui a livré son plan,
# refus non-terminal survécu. Ni « PIPELINE FAILURE » ni « halted ».
T2493_E2E_SURVIVED=$(_t2493_e2e "$T2493_FIXTURE_DIR/nonterm.stderr" plan 2>/dev/null)
assert_not_contains "T9 (DoD-1): plan livré + deny non-terminal ⇒ pas de PIPELINE FAILURE" \
    "PIPELINE FAILURE:" "$T2493_E2E_SURVIVED"
assert_not_contains "T9 (DoD-1): plan livré + deny non-terminal ⇒ pas de « halted »" \
    "halted" "$T2493_E2E_SURVIVED"
# AC4 — le refus est REQUALIFIÉ, jamais supprimé : il reste visible en annexe.
assert_contains "AC4: le refus non-terminal reste visible dans le result (note)" \
    "the session continued past it" "$T2493_E2E_SURVIVED"
assert_contains "AC4: et la note joint la preuve du refus" \
    "Observed deny:" "$T2493_E2E_SURVIVED"
assert_contains "AC4: la note nomme la létalité lue" \
    "Lethality marker: (non-terminal)" "$T2493_E2E_SURVIVED"
# Idempotence : les deux sites peuvent tirer sur un même dispatch dev-groom.
assert_eq "La note n'est annexée qu'une fois, même quand les deux sites tirent" \
    "1" "$(grep -c 'the session continued past it' <<<"$T2493_E2E_SURVIVED" || true)"

# T15 — LE TEST DE L'EFFET, et non de la formulation (M6). Le faux
# `PIPELINE FAILURE:` ne se contentait pas de mal nommer : il CAPTURAIT le
# classificateur de la ligne `Outcome:`. T9 assertte une ABSENCE de sous-chaîne,
# qu'une reformulation malheureuse pourrait satisfaire sans rien réparer ; T15
# assertte la PRÉSENCE du bon classement, qu'on ne peut pas obtenir par accident.
assert_contains "T15 (AC6): la session est classée PLAN_COMMITTED" \
    "Outcome: PLAN_COMMITTED" "$T2493_E2E_SURVIVED"
assert_not_contains "T15 (AC6): et non PIPELINE_INCOMPLETE" \
    "Outcome: PIPELINE_INCOMPLETE" "$T2493_E2E_SURVIVED"

# T16 — CONTRÔLE NÉGATIF de T15. Même fixture, `VALID_PLAN` vide : une session
# qui n'a réellement rien produit continue d'être classée PIPELINE_INCOMPLETE.
# C'est ce qui distingue « le correctif répare le faux positif » de « le
# correctif a désarmé le classificateur ».
T2493_E2E_NOPLAN=$(_t2493_e2e "$T2493_FIXTURE_DIR/nonterm.stderr" 2>/dev/null)
assert_contains "T16 (contrôle négatif): sans plan, la session reste PIPELINE_INCOMPLETE" \
    "Outcome: PIPELINE_INCOMPLETE" "$T2493_E2E_NOPLAN"
assert_contains "T16: et le diagnostic de la branche qui s'applique garde la parole" \
    "_find_issue_plan returned empty" "$T2493_E2E_NOPLAN"
# D4 : le refus est annexé, il n'ÉVINCE pas le vrai diagnostic. Remplacer
# « pas de plan trouvé, causes probables (a) dérive (b) bug de découverte » par
# « halted by policy deny » déplacerait le mensonge du ticket au lieu de le
# fermer — on substituerait au vrai diagnostic un faux, sous couvert de précision.
assert_contains "D4: sans plan non plus, le deny non-terminal n'est qu'une annexe" \
    "the session continued past it" "$T2493_E2E_NOPLAN"
assert_not_contains "D4: et le verbe « halted » n'apparaît pas pour autant" \
    "halted" "$T2493_E2E_NOPLAN"

# T10 — DoD point 2, le SENS INVERSE. Un refus terminal produit toujours le
# libellé, augmenté de la mention de sa létalité. Le verbe reste disponible
# pour les vrais halts : c'est la moitié qu'un correctif trop large casserait.
T2493_E2E_TERMINAL=$(_t2493_e2e "$T2493_FIXTURE_DIR/term.stderr" 2>/dev/null)
assert_contains "T10 (DoD-2): deny terminal ⇒ le result porte « halted »" \
    "halted" "$T2493_E2E_TERMINAL"
assert_contains "T10 (DoD-2): et la mention (terminal)" \
    "(terminal)" "$T2493_E2E_TERMINAL"
assert_contains "T10: le libellé historique du site dev-groom est conservé (REQ7)" \
    "halted by claude-pilot policy deny" "$T2493_E2E_TERMINAL"
assert_contains "T10: et la session est bien classée en échec" \
    "Outcome: PIPELINE_INCOMPLETE" "$T2493_E2E_TERMINAL"
# AC3 : les deux sens sont deux tests distincts, et T5/T6 tiennent la frontière.
assert_not_contains "T10: un halt terminal n'annexe pas en plus la note de survie" \
    "the session continued past it" "$T2493_E2E_TERMINAL"

# T11 — D5, contrainte DURE sur la note, assertée sur le TEXTE PRODUIT et non
# sur la constante. `dispatch-lib.sh` grep le contenu de `RESULT` pour cinq
# jetons ; une note d'information qui en introduirait un reclasserait la session
# en échec — exactement le défaut réparé, reconstruit par le correctif.
T2493_NOTE_ONLY=$(
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        RESULT=""
        _annex_policy_deny_note "non-terminal" "[policy:deny] Bash: x [r] (non-terminal)"
        printf '%s' "$RESULT"
    )
)
assert_contains "T11 (bonne foi): la sonde produit bien la note" \
    "Observed deny:" "$T2493_NOTE_ONLY"
for _tok in "PIPELINE FAILURE:" "STRUCTURAL VIOLATION:" "HANDLER CRASH"; do
    assert_not_contains "T11 (D5): la note ne porte pas le jeton « $_tok »" \
        "$_tok" "$T2493_NOTE_ONLY"
done
assert_eq "T11 (D5): la note n'ouvre aucune ligne par STATUS=CANCELLED" \
    "0" "$(grep -c '^STATUS=CANCELLED' <<<"$T2493_NOTE_ONLY" || true)"
assert_eq "T11 (D5): la note n'ouvre aucune ligne par Outcome: PIPELINE_INCOMPLETE" \
    "0" "$(grep -c '^Outcome: PIPELINE_INCOMPLETE' <<<"$T2493_NOTE_ONLY" || true)"
# Le même prédicat que le lecteur de reclassement applique, appliqué à la note.
assert_eq "T11 (D5): le prédicat de reclassement de dispatch-lib ne mord pas sur la note" \
    "0" "$(grep -cE '(PIPELINE FAILURE:|STRUCTURAL VIOLATION:|HANDLER CRASH|^STATUS=CANCELLED|^Outcome: PIPELINE_INCOMPLETE)' <<<"$T2493_NOTE_ONLY" || true)"

# T3 bout-en-bout — REQ4/AC5 : une létalité non déclarée n'affirme rien, et le
# message dit POURQUOI, en nommant le build du pilote plutôt que la lecture.
T2493_E2E_UNDECLARED=$(_t2493_e2e "$T2493_FIXTURE_DIR/undeclared.stderr" plan 2>/dev/null)
assert_contains "AC5: létalité non déclarée ⇒ ni terminal ni non-terminal affirmé" \
    "Lethality marker: undeclared" "$T2493_E2E_UNDECLARED"
assert_contains "AC5: et le message impute l'absence au build du pilote (cpp#151)" \
    "cpp#151" "$T2493_E2E_UNDECLARED"
assert_not_contains "AC5: un undeclared ne produit pas « halted »" \
    "halted" "$T2493_E2E_UNDECLARED"

# --- Herméticité de cette section (patron mika#2178 T9) ---------------------
T2493_SECTION=$(sed -n '/^# mika#2493 — un deny non-terminal survécu est une NOTE/,$p' "${BASH_SOURCE[0]}")
assert_eq "Herméticité: l'extraction de la section mika#2493 a trouvé la section" \
    "yes" "$(if [ -n "$T2493_SECTION" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "Herméticité: aucune invocation de forge en tête de commande" \
    "0" "$(printf '%s\n' "$T2493_SECTION" | grep -cE '^[[:space:]]*gh[[:space:]]' || true)"
assert_eq "Herméticité: aucune invocation de forge en substitution" \
    "0" "$(printf '%s\n' "$T2493_SECTION" | grep -cE '\$\(gh[[:space:]]' || true)"

rm -rf "$T2493_FIXTURE_DIR"

# ============================================================================
# mika#2539 — le jeton de cause DÉRIVE de _halt_family, il ne la redéclare pas
#
# Ces scans sont structurels et non comportementaux, et c'est une nécessité :
# une seconde table de classification ne rendrait AUCUNE décision fausse le jour
# où on l'écrit. Elle rendrait `_rescue_cause_token` juste, et la laisserait
# dériver silencieusement ensuite — un sous-type ajouté en amont ferait rougir le
# drift guard T6 et passerait sans un mot dans le message de sauvetage. C'est la
# panne que mika#2158 a mesurée sur `auto_pull.rs` : une regex commentée
# « Mirrors GROOMED_VERDICT_RE » qui n'a suivi aucun des deux élargissements
# suivants, et promotion et routage ont répondu différemment à la même question
# pendant des mois. Aucune suite comportementale ne voit cette classe.
#
# T6 n'est ni touché ni dupliqué : il couvre déjà le jeton par construction,
# puisque le jeton EST la sortie de `_halt_family`.
# ============================================================================
echo ""
echo "mika#2539: le jeton de cause du sauvetage dérive de la table de halte"
echo "---------------------------------------------------------------------"

T2539_TOKEN_FN=$(sed -n '/^_rescue_cause_token() {$/,/^}$/p' "$DISPATCH_LIB")
T2539_RESOLVER_FN=$(sed -n '/^_resolve_halt_subtype() {$/,/^}$/p' "$DISPATCH_LIB")

# S3 (anti-vacuité) — AVANT tout le reste. Un scan qui regarde une fonction
# disparue ou vide se lit exactement comme un arbre propre (classe mika#2205),
# et les scans S1/S2 ci-dessous seraient vacuement verts sur une chaîne vide.
assert_eq "S3 (anti-vacuité): _rescue_cause_token existe et son corps est non vide" "yes" \
    "$(if [ "$(printf '%s\n' "$T2539_TOKEN_FN" | grep -c .)" -gt 3 ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "S3 (anti-vacuité): _resolve_halt_subtype existe et son corps est non vide" "yes" \
    "$(if [ "$(printf '%s\n' "$T2539_RESOLVER_FN" | grep -c .)" -gt 3 ]; then printf 'yes'; else printf 'no'; fi)"

# S1 — la délégation, mesurée. Sans cet appel, le jeton porte une table à soi.
assert_contains "S1: _rescue_cause_token consulte _halt_family" \
    "_halt_family" "$T2539_TOKEN_FN"
assert_contains "S1: et il en tire la famille (premier champ de la ligne)" \
    '%%|*' "$T2539_TOKEN_FN"

# S2 — aucun nom de sous-type amont dans le corps. C'est R-3 rendu exécutable :
# la seule forme que prendrait une seconde table.
#
# La population des sous-types interdits est DÉRIVÉE de `_halt_family` elle-même,
# jamais recopiée ici. Une liste en dur aurait exactement le défaut qu'elle
# prétend interdire : elle cesserait de couvrir le prochain sous-type ajouté à la
# table, donc le scan rétrécirait en silence pendant que l'arbre reste vert.
T2539_SUBTYPES=$(sed -n '/^_halt_family() {$/,/^}$/p' "$DISPATCH_LIB" \
    | grep -oE '^        [a-z][a-z0-9_]*\)' | tr -d ' )' || true)
# Anti-vacuité de la dérivation : la table en compte douze aujourd'hui. Si
# l'extraction en rend une poignée, l'ancre `case` ne matche plus et S2 ne
# regarde plus rien.
assert_eq "S2 (anti-vacuité): la population de sous-types est dérivée de la table" "yes" \
    "$(if [ "$(printf '%s\n' "$T2539_SUBTYPES" | grep -c .)" -ge 12 ]; then printf 'yes'; else printf 'no'; fi)"

# Une entrée = un site qui énumère un sous-type de halte hors de _halt_family,
# au format `<sous-type>|<raison + ticket de suivi>`. LIVRÉE VIDE : les deux
# fonctions scannées sont créées par mika#2539, donc aucune violation
# préexistante ne peut exister.
#
# QUAND CE SCAN TIRE, LA RÉSOLUTION EST DE ROUTER LE SITE VERS _halt_family —
# JAMAIS d'ajouter une ligne ici (doctrine mika#2201). Un site qu'on ne veut pas
# router est un site à supprimer.
RESCUE_CAUSE_SUBTYPE_ALLOWED=()

T2539_LEAKED=""
for _t2539_st in $T2539_SUBTYPES; do
    grep -qF -- "$_t2539_st" <<<"$T2539_TOKEN_FN" || continue
    _t2539_excused=no
    for _t2539_entry in ${RESCUE_CAUSE_SUBTYPE_ALLOWED+"${RESCUE_CAUSE_SUBTYPE_ALLOWED[@]}"}; do
        [ "${_t2539_entry%%|*}" = "$_t2539_st" ] && _t2539_excused=yes && break
    done
    [ "$_t2539_excused" = "yes" ] || T2539_LEAKED="${T2539_LEAKED}${_t2539_st} "
done
assert_eq "S2: aucun sous-type amont énuméré dans _rescue_cause_token" "" \
    "$(printf '%s' "$T2539_LEAKED" | sed 's/ *$//')"

# Le double sens — l'assertion auto-nettoyante. Une entrée qui ne matche plus
# rien fait rougir le build le jour de la réparation, pas des mois après.
T2539_STALE=""
for _t2539_entry in ${RESCUE_CAUSE_SUBTYPE_ALLOWED+"${RESCUE_CAUSE_SUBTYPE_ALLOWED[@]}"}; do
    _t2539_st="${_t2539_entry%%|*}"
    grep -qF -- "$_t2539_st" <<<"$T2539_TOKEN_FN" \
        || T2539_STALE="${T2539_STALE}${_t2539_st} "
done
assert_eq "S2 (double sens): aucune entrée d'allowlist périmée" "" \
    "$(printf '%s' "$T2539_STALE" | sed 's/ *$//')"

# S4 — un seul résolveur. Un troisième site qui résoudrait le sous-type à la
# main rouvrirait la divergence que `_resolve_halt_subtype` existe pour fermer :
# la bannière du callback dirait `Halt class: session_silent` pendant que le
# commit dirait `rescue no_halt_signal`, pour une même session, l'un des deux
# gravé dans l'historique git pour toujours.
#
# Ce que ce scan compte est le nombre d'APPELANTS DU RÉSOLVEUR, pas un plafond de
# consommateurs : un site de plus qui l'appelle est la conformité, un site de
# plus qui lit `$SUBTYPE` à la main serait la panne. mika#2029 en a ajouté un
# troisième — la branche `_terminated_empty` de `_run_claude_pilot`, qui compose
# le motif `NO_PR: pilot_terminated_<sous-type>` — et il l'appelle AVANT la
# substitution `RESULT=$(_classify_terminated_session)` : résolu ici, le mémo est
# hérité par le sous-shell, donc le motif et la bannière décrivent la même
# résolution par construction. La contrepartie `$SUBTYPE` de cette branche est
# scannée par V3 dans le bloc mika#2029.
#
# mika#2077 en a ajouté un QUATRIÈME — `_classify_revise_halt`, qui classe la
# halte de la session de REVISE. Il est l'exacte conformité que ce scan décrit :
# il APPELLE le résolveur au lieu de relire `$SUBTYPE` à la main. Sa particularité
# est inverse de celle de mika#2029 : il est appelé en SUBSTITUTION DE COMMANDE
# et commence par `unset` le mémo, précisément pour ne PAS hériter de la
# résolution du pilote principal — ici, une session, une réponse signifie deux
# sessions, deux réponses. V5/V6 du bloc mika#2077 mesurent les deux sens.
#
# Le compte est fait hors définition et hors commentaire — seuls les appels.
T2539_RESOLVER_CALLS=$(grep -n '_resolve_halt_subtype' "$DISPATCH_LIB" \
    | grep -v '_resolve_halt_subtype() {' \
    | grep -vE ':[[:space:]]*#' | grep -c . || true)
assert_eq "S4: _resolve_halt_subtype a exactement quatre appelants de production" "4" \
    "$T2539_RESOLVER_CALLS"
assert_contains "S4: le premier est la bannière du callback" \
    '_resolve_halt_subtype' "$(sed -n '/^_classify_terminated_session() {$/,/^}$/p' "$DISPATCH_LIB")"
assert_contains "S4: le deuxième est le compositeur du sujet de sauvetage" \
    '_resolve_halt_subtype' "$T2539_TOKEN_FN"
assert_contains "S4: le troisième est la branche _terminated_empty (mika#2029)" \
    '_resolve_halt_subtype' \
    "$(sed -n '/^    if \[ "$_terminated_empty" = "1" \]; then$/,/^    fi$/p' "$DISPATCH_LIB" \
        | grep -vE '^[[:space:]]*#')"
assert_contains "S4: le quatrième est le classeur de la halte de revise (mika#2077)" \
    '_resolve_halt_subtype' \
    "$(sed -n '/^_classify_revise_halt() {$/,/^}$/p' "$DISPATCH_LIB")"

# Le mémo est réinitialisé au site qui lit le sous-type d'une NOUVELLE session,
# et pas chez l'un des deux appelants : « une session, une réponse » est ainsi
# une propriété de la construction et non de la mémoire du prochain éditeur.
#
# mika#2077 — ils sont DEUX depuis que deux sessions distinctes sont classées
# dans le même processus. Le compte reste exact et chaque site reste adossé à la
# lecture d'un sous-type : `:5503` au `SUBTYPE=` du pilote principal, celui de
# `_classify_revise_halt` au `SUBTYPE` local de la session de revise. Un
# troisième site serait de nouveau une réinitialisation sans session neuve à
# lire — c'est-à-dire la perte de la propriété, pas son renforcement.
assert_eq "S4: le mémo est réinitialisé exactement deux fois, aux deux sites de SUBTYPE" "2" \
    "$(grep -c '^    unset _HALT_SUBTYPE_RESOLVED _HALT_GUARDRAIL_LINE$' "$DISPATCH_LIB")"
assert_contains "S4: la seconde réinitialisation est celle de la session de revise" \
    'unset _HALT_SUBTYPE_RESOLVED _HALT_GUARDRAIL_LINE' \
    "$(sed -n '/^_classify_revise_halt() {$/,/^}$/p' "$DISPATCH_LIB")"

# S5 — dispatch-lib parse toujours.
T2539_RC=0
bash -n "$DISPATCH_LIB" 2>/dev/null || T2539_RC=$?
assert_eq "S5: dispatch-lib.sh passe bash -n" "0" "$T2539_RC"

# --- Herméticité de cette section (patron mika#2178 T9 / mika#2493) ---------
T2539_SECTION=$(sed -n '/^# mika#2539 — le jeton de cause DÉRIVE de _halt_family/,$p' "${BASH_SOURCE[0]}")
assert_eq "Herméticité: l'extraction de la section mika#2539 a trouvé la section" \
    "yes" "$(if [ -n "$T2539_SECTION" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "Herméticité: aucune invocation de forge en tête de commande" \
    "0" "$(printf '%s\n' "$T2539_SECTION" | grep -cE '^[[:space:]]*gh[[:space:]]' || true)"
assert_eq "Herméticité: aucune invocation de forge en substitution" \
    "0" "$(printf '%s\n' "$T2539_SECTION" | grep -cE '\$\(gh[[:space:]]' || true)"

# ============================================================================
# mika#2545 — un ESCALATE de groom est TERMINAL, et son producteur l'estampille
# ============================================================================
#
# Le défaut mesuré : `_escalate_groom` écrivait `PIPELINE FAILURE:`, marqueur
# qui fait entrer le tour dans la population RETRYABLE de `self-dev-callback`.
# Un verdict de halte — terminal par contrat de `/mika-groom-ticket` — était
# donc rejoué : huit dispatches sur mika#2542 le 2026-09-26, chacun ESCALATE
# sur la même cause.
#
# Le producteur pose désormais son propre fait : `Outcome: ESCALATE — <stage>`,
# écrit par `_set_outcome_line` (mika#2492) donc unique par construction, et le
# marqueur retryable disparaît. Les assertions ci-dessous couvrent les trois
# stages, la non-régression de la population retryable LÉGITIME, et la
# non-régression du gate mika#1996 — dont le terme de reclassement est la part
# fragile de ce correctif : `PIPELINE FAILURE:` était aussi ce qui sortait le
# cycle de `empty_completion`.

echo ""
echo "Test: un ESCALATE de groom est terminal (mika#2545)"
echo "-------------------------------------------------------------------"

# Sonde comportementale : `_escalate_groom` est appelable hors contexte — elle
# ne lit que WORKTREE_DIR, RESULT et ses trois arguments.
_t2545_escalate_probe() {
    local stage="$1" content="$2" result_in="${3:-}"
    local base wt
    base=$(mktemp -d "${TMPDIR:-/tmp}/mika-2545-test.XXXXXX")
    wt="$base/wt"; mkdir -p "$wt"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="$wt"
        RESULT="$result_in"
        _escalate_groom "$stage" "$content" "sess-2545" 2>/dev/null
        printf '%s' "$RESULT"
    )
    rm -rf "$base"
}

T2545_BASE_RESULT="claude-pilot completed (status: success).
Session: sess-2545
Turns: 12"

# --- Les trois stages : le fait est posé, le marqueur retryable a disparu ----
for _t2545_stage in first-pass second-pass-after-ready second-pass-after-iterate; do
    T2545_OUT=$(_t2545_escalate_probe "$_t2545_stage" "F1: le plan ne cite rien." "$T2545_BASE_RESULT")
    assert_eq "AC3 ($_t2545_stage): exactement une ligne Outcome: ESCALATE" \
        "1" "$(printf '%s\n' "$T2545_OUT" | grep -c "^Outcome: ESCALATE" || true)"
    assert_eq "AC3 ($_t2545_stage): exactement une ligne Outcome, toutes valeurs confondues" \
        "1" "$(printf '%s\n' "$T2545_OUT" | grep -c '^Outcome: ' || true)"
    assert_not_contains "AC3 ($_t2545_stage): plus aucun marqueur de la population retryable" \
        "PIPELINE FAILURE:" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): la ligne Outcome nomme le stage" \
        "Outcome: ESCALATE — $_t2545_stage" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): le corps nomme sa classe terminale" \
        "GROOM ESCALATED (terminal):" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): la session reste lisible" \
        "Session: sess-2545" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): le verdict reste lisible" \
        "Verdict: ESCALATE" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): le chemin des findings est nommé" \
        "escalate-${_t2545_stage}.md" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): le corps du RESULT antérieur est préservé" \
        "Turns: 12" "$T2545_OUT"
done

# Une ligne `Outcome:` déjà posée par la validation de plan est REMPLACÉE, pas
# doublée — c'est la propriété que `_set_outcome_line` garantit par construction
# et la seule raison de ne pas concaténer à la main ici.
T2545_OVERWRITE=$(_t2545_escalate_probe first-pass "F1: rien." "$T2545_BASE_RESULT

Outcome: PIPELINE_INCOMPLETE — no_shipping_tail: dispatch-lib did not reach PR creation.")
assert_eq "AC3: une ligne Outcome préexistante est remplacée, jamais doublée" \
    "1" "$(printf '%s\n' "$T2545_OVERWRITE" | grep -c '^Outcome: ' || true)"
assert_contains "AC3: et c'est l'ESCALATE qui reste" \
    "Outcome: ESCALATE — first-pass" "$T2545_OVERWRITE"

# La cause moteur de mika#2338 voyage toujours : ce ticket change la CLASSE du
# marqueur, pas la substance transmise.
T2545_ENGINE=$(_t2545_escalate_probe first-pass \
    "F1: (BLOCKING) [mika-engine] review-anchor: attestation withheld after the corrective re-prompt." \
    "$T2545_BASE_RESULT")
assert_contains "mika#2338 (non-régression): la cause moteur voyage toujours dans RESULT" \
    "Engine reason:" "$T2545_ENGINE"

# --- Cardinalité : un seul site d'écriture du fait --------------------------
#
# Sans cette assertion, un prédicat devenu trop étroit passerait en ne regardant
# rien (classe mika#2205). Elle est aussi ce qui refuse un second écrivain : le
# fait doit être posé par son producteur, et par lui seul.
T2545_OUTCOME_WRITERS_ALLOWED=""   # mika#2545: livrée VIDE, et l'assertion suivante le tient
assert_eq "allowlist des écrivains de Outcome: ESCALATE — zero entries" \
    "" "$T2545_OUTCOME_WRITERS_ALLOWED"
assert_eq "exactement un site d'écriture de Outcome: ESCALATE dans dispatch-lib" \
    "1" "$(grep -c '_set_outcome_line "Outcome: ESCALATE' "$DISPATCH_LIB" || true)"
# Les commentaires sont retirés avant le scan : la prose de cette fonction nomme
# légitimement le marqueur qu'elle n'écrit plus — c'est le faux positif que
# mika#2050 a mesuré sur le Signal S, et le même geste que GATE_CODE au test 15.
T2545_ESCALATE_FN=$(sed -n '/^_escalate_groom() {$/,/^}$/p' "$DISPATCH_LIB" | grep -v '^[[:space:]]*#')
assert_contains "et ce site est _escalate_groom elle-même" \
    '_set_outcome_line "Outcome: ESCALATE' "$T2545_ESCALATE_FN"
assert_not_contains "_escalate_groom n'écrit plus le marqueur de la population retryable" \
    "PIPELINE FAILURE:" "$T2545_ESCALATE_FN"
assert_contains "bonne foi: l'extraction du corps de _escalate_groom a bien trouvé son code" \
    "GROOM ESCALATED (terminal):" "$T2545_ESCALATE_FN"

# --- Le gate mika#1996 ne reclasse PAS un ESCALATE en cycle vide ------------
#
# La part fragile du correctif. `PIPELINE FAILURE:` sortait le cycle de
# `empty_completion` ; le retirer exposerait un ESCALATE à un faux rouge sur une
# sortie délibérée. Deux termes le couvrent et il faut les DEUX — le second cas
# ci-dessous (compte d'appels d'outils NON MESURÉ) est celui que P4 ne peut pas
# rattraper, puisque sa conjonction exige `CYCLE_TOOL_CALLS >= 1`.
T2545_ESCALATE_RESULT="claude-pilot completed (status: success).
GROOM ESCALATED (terminal): mika-arch escalated at first-pass.
Verdict: ESCALATE — human review required.

Outcome: ESCALATE — first-pass"

T2545_GATE_UNMEASURED=$(_gate_probe result no none "" none "$T2545_ESCALATE_RESULT")
assert_not_contains "AC7: un ESCALATE dont le compte d'outils n'est pas mesuré n'est pas reclassé vide" \
    "PIPELINE FAILURE: empty_completion" "$T2545_GATE_UNMEASURED"
assert_contains "AC7: et il garde sa propre disposition" \
    "Outcome: ESCALATE — first-pass" "$T2545_GATE_UNMEASURED"

T2545_GATE_ZERO=$(_gate_probe result no none "" 0 "$T2545_ESCALATE_RESULT")
assert_not_contains "AC7: un ESCALATE mesuré à zéro appel d'outil n'est pas reclassé vide non plus" \
    "PIPELINE FAILURE: empty_completion" "$T2545_GATE_ZERO"

# Contrôle POSITIF du gate : P4 continue de reconnaître l'ESCALATE comme une
# disposition motivée quand le compte est mesuré. Sans ce contrôle, « le terme
# ajouté mord » ne se distingue pas de « P4 ne voit plus rien ».
T2545_GATE_VERDICT=$(_gate_probe verdict no none "" 3 "$T2545_ESCALATE_RESULT")
assert_eq "AC7 (contrôle positif): un ESCALATE avec des appels d'outils est 'produced' par P4" \
    "produced" "$T2545_GATE_VERDICT"

# Contrôle NÉGATIF du gate : la population `empty_completion` existe toujours.
T2545_GATE_STILL_EMPTY=$(_gate_probe result no none "" 0 "claude-pilot completed (status: success).")
assert_contains "AC7 (contrôle négatif): un cycle réellement vide est toujours banni" \
    "PIPELINE FAILURE: empty_completion" "$T2545_GATE_STILL_EMPTY"

# --- Non-régression : la population retryable LÉGITIME est intacte ----------
T2545_REAL_FAILURE="PIPELINE FAILURE: the claude-pilot session was terminated before it produced any work.

Outcome: PIPELINE_INCOMPLETE — pilot session terminated by claude-pilot before producing work."
T2545_GATE_REAL=$(_gate_probe result no none "" 0 "$T2545_REAL_FAILURE")
assert_contains "AC6: un PIPELINE FAILURE authentique garde son marqueur" \
    "PIPELINE FAILURE: the claude-pilot session was terminated" "$T2545_GATE_REAL"
assert_contains "AC6: et son Outcome: PIPELINE_INCOMPLETE" \
    "Outcome: PIPELINE_INCOMPLETE — pilot session terminated" "$T2545_GATE_REAL"
assert_eq "AC6: sans bannière empilée" \
    "1" "$(printf '%s\n' "$T2545_GATE_REAL" | grep -c 'PIPELINE FAILURE:' || true)"

# --- La branche `else` de dispatch_claude_pilot PRÉSERVE un ESCALATE --------
#
# Son texte — « grooming did not converge » — reste juste pour ses dix-sept
# autres sorties ; il est FAUX pour un ESCALATE, qui a convergé, sur un verdict
# de halte. Scan de source : ce bras n'est pas atteignable hors d'un dispatch
# complet (worktree, pilote, architecte).
T2545_CONVERGENCE_BLOCK=$(sed -n '/^        if _iterate_groom_loop; then$/,/^    fi$/p' "$DISPATCH_LIB")
assert_eq "AC3 (bonne foi): le bloc de convergence de dispatch_claude_pilot a été extrait" \
    "yes" "$(if [ -n "$T2545_CONVERGENCE_BLOCK" ]; then printf 'yes'; else printf 'no'; fi)"
assert_contains "AC3: un bras consulte la ligne Outcome: ESCALATE avant toute réécriture" \
    "elif grep -qE '^Outcome: ESCALATE'" "$T2545_CONVERGENCE_BLOCK"
assert_contains "AC3: et l'écriture PIPELINE FAILURE reste présente pour les autres sorties" \
    "PIPELINE FAILURE: grooming did not converge" "$T2545_CONVERGENCE_BLOCK"
# L'ordre est porteur : le bras ESCALATE doit précéder celui qui réécrit
# `Outcome: .*` au sed, sinon il n'est jamais atteint.
T2545_ESC_POS=$(printf '%s\n' "$T2545_CONVERGENCE_BLOCK" | grep -n "elif grep -qE '\^Outcome: ESCALATE'" | head -1 | cut -d: -f1)
T2545_SED_POS=$(printf '%s\n' "$T2545_CONVERGENCE_BLOCK" | grep -n 's/Outcome: .\*/Outcome: PIPELINE_INCOMPLETE' | head -1 | cut -d: -f1)
assert_eq "AC3: le bras ESCALATE précède la réécriture sed de la ligne Outcome" "yes" \
    "$([ -n "$T2545_ESC_POS" ] && [ -n "$T2545_SED_POS" ] && [ "$T2545_ESC_POS" -lt "$T2545_SED_POS" ] && echo yes || echo "non ($T2545_ESC_POS vs $T2545_SED_POS)")"

# Le chemin GROOMED est inchangé à l'octet près — c'est la moitié qui garantit
# que ce ticket n'a pas déplacé le succès en même temps que l'échec.
T2545_GROOMED_ARM=$(sed -n '/mika#1394: Architect converged on GROOMED/,/^        elif grep -qE/p' "$DISPATCH_LIB")
assert_contains "AC6: le chemin GROOMED écrit toujours Outcome: PLAN_GROOMED" \
    "Outcome: PLAN_GROOMED" "$T2545_GROOMED_ARM"
assert_contains "AC6: et strippe toujours les marqueurs PIPELINE FAILURE périmés" \
    "sed '/^PIPELINE FAILURE:/d'" "$T2545_GROOMED_ARM"

# ===========================================================================
# mika#2578 (D2) — le chemin de staging du jeton a UN site de déclaration,
# conditionnel.
# ===========================================================================
# Aucun test comportemental ne peut voir cette classe : une régression vers
# l'affectation inconditionnelle (`_PILOT_GH_TOKEN_FILE="$HOME/…"`) laisse
# TOUTES les assertions de staging vertes — le défaut est que la pose d'un
# harness redevient silencieusement inerte au `source`, donc que le canari se
# remet à écraser la credential GitHub de l'hôte sans que rien ne rougisse.

# Anti-vacuité d'abord : un scan dont le chemin pourrit se lit exactement comme
# un arbre propre (classe mika#2205).
T2578_BYTES=$(wc -c < "$DISPATCH_LIB" 2>/dev/null || echo 0)
assert_eq "mika#2578 anti-vacuité: dispatch-lib.sh est bien le vrai fichier" "yes" \
    "$([ "${T2578_BYTES:-0}" -gt 100000 ] && echo yes || echo "non ($T2578_BYTES octets)")"

# Un seul passage de filtrage : dispatch-lib.sh pèse ~537 Ko, et le compte comme
# la ligne se lisent du même résultat.
T2578_DECL_LINES=$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -E '_PILOT_GH_TOKEN_FILE:?=' || true)
T2578_DECL_COUNT=$(printf '%s' "$T2578_DECL_LINES" | grep -c . || true)
assert_eq "mika#2578: exactement un site de déclaration de _PILOT_GH_TOKEN_FILE" \
    "1" "${T2578_DECL_COUNT:-0}"

T2578_DECL=$(printf '%s\n' "$T2578_DECL_LINES" | head -1)
assert_contains "mika#2578: ce site est la forme conditionnelle (une valeur de harness survit au source)" \
    ': "${_PILOT_GH_TOKEN_FILE:=' "$T2578_DECL"
assert_contains "mika#2578: et son défaut est INCHANGÉ (un dispatch réel stache toujours sous \$HOME)" \
    '$HOME/.mika/pilot-gh-token' "$T2578_DECL"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
