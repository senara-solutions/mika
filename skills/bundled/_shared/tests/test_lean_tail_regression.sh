#!/bin/bash
# Suite d'attestation du détecteur de régression d'invocation (mika#2729).
#
# POURQUOI. mika#2694 a retiré `ce-simplify-code` et la revue par sous-agents de
# la fin de pipeline des pilotes. Son essai borné (#2728,
# docs/solutions/dev-loop/un-essai-dabsence-exige-un-controle-positif-sur-son-instrument.md)
# a mesuré que le crochet tient : 0 invocation sur 45 transcripts post-consigne.
# Ce que l'essai ne fournit pas, c'est un moyen de s'en apercevoir le jour où ça
# repart. `_detect_lean_tail_regression` est cet organe ; cette suite est la
# moitié qui l'atteste, et c'est elle qui tourne en CI (la garde, elle, a besoin
# d'un transcript de vol qu'aucun runner n'a).
#
# PROVENANCE DE LA LIGNE PORTEUSE — c'est une transcription attestée, pas une
# forme inventée. La signature a été établie par `grep -oh` AVANT tout comptage,
# au § III de #2728, sur trois témoins pré-consigne réels, le 2026-10-15 :
#
#   grep -oh '"name": "Skill", "input": {"skill": "compound-engineering:[a-z-]*"' <témoin>
#   # → "name": "Skill", "input": {"skill": "compound-engineering:ce-simplify-code"
#
#   | témoin                                 | ticket     | ce-simplify-code | ce-code-review |
#   |----------------------------------------|------------|------------------|----------------|
#   | 500a76a5-… (201 tours, error_max_turns)| mika#2683  | 1                | 0              |
#   | 59ffa68e-… (201 tours, error_max_turns)| mika#2654  | 1                | 0              |
#   | db73ec9f-… (201 tours, error_max_turns)| mika#2693  | 1                | 1              |
#
# Les fixtures ci-dessous reproduisent cette forme. Elles sont écrites dans un
# `mktemp -d` et JAMAIS commitées : un transcript réel pèse ~1,6 Mo et porte le
# prompt intégral d'un vol.
#
# CE QUE CE CONTRÔLE POSITIF NE PROUVE PAS, et il faut l'écrire plutôt que le
# laisser deviner : une fixture synthétique démontre que l'instrument trouve la
# forme qu'on lui a DÉCRITE, pas que cette forme est celle que `claude-pilot`
# ÉMET. Cette seconde propriété a été établie UNE FOIS, par mesure, sur les trois
# témoins ci-dessus. La tolérance d'espacement couvre la seule dérive de
# sérialiseur mesurée ; une dérive d'une autre nature (clé renommée, bloc
# `tool_use` restructuré) rendrait le détecteur muet, et seule une nouvelle
# mesure sur transcripts vivants le découvrirait. Une sonde contre les
# transcripts de l'hôte a été envisagée et ÉCARTÉE : sur un hôte sain le compte
# est 0, donc elle ne peut pas servir de contrôle positif.
#
# TROIS MOITIÉS, et la troisième est celle qu'on saute :
#   P1-P4   le détecteur TROUVE quand l'occurrence existe (critère 3 du ticket)
#   N1-N6   chacun des faux résultats mesurés est refusé
#   X1-X3   chaque moitié de l'instrument est montrée PORTEUSE par mutation
#   S       le sink et RESULT sont épinglés ; la signature a un seul site
#
# X1-X3 ne sont pas de la décoration. Sans elles, « la suite passe » et « la
# suite ne regarde rien » sont indistinguables (classe mika#2205) — dans la suite
# même qui existe pour refuser cette confusion.
#
# Elle `source` la VRAIE dispatch-lib et appelle la VRAIE fonction. Aucun réseau,
# aucun cargo, aucun claude-pilot.
#
# Run: bash skills/bundled/_shared/tests/test_lean_tail_regression.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

PASS=0
FAIL=0

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

assert_contains() {
    local label="$1" needle="$2" hay="$3"
    if grep -qF -- "$needle" <<<"$hay"; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    missing: '$needle'"
        echo "    in:      '$(printf '%s' "$hay" | head -c 400)'"
    fi
}

assert_not_contains() {
    local label="$1" needle="$2" hay="$3"
    if grep -qF -- "$needle" <<<"$hay"; then
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    unexpectedly present: '$needle'"
        echo "    in:      '$(printf '%s' "$hay" | head -c 400)'"
    else
        PASS=$((PASS + 1)); echo "  ✓ $label"
    fi
}

FIXTURES="$(mktemp -d "${TMPDIR:-/tmp}/mika-2729-fx.XXXXXX")"
trap 'rm -rf "$FIXTURES"' EXIT

# Le texte que `RESULT` porte DÉJÀ quand la garde est appelée : en production
# elle tourne après le bloc de classification, donc sur un RESULT composé. Une
# fixture partant de RESULT="" ne verrait pas qu'un paragraphe doit s'AJOUTER.
RESULT_SEED="Log path: /var/log/claude-pilot/log-2729.log

claude-pilot completed (status: success)."

# Un transcript d'une ligne, nommé. $1 nom  $2… lignes.
make_transcript() {
    local name="$1"; shift
    local path="$FIXTURES/$name.jsonl"
    printf '%s\n' "$@" > "$path"
    printf '%s' "$path"
}

# Lance la VRAIE fonction sur un transcript et publie ce qu'elle a produit.
# Appel DIRECT, jamais en sous-shell : `RESULT` est une sortie de la fonction, et
# une substitution de commande la rendrait invisible.
#
# $1 valeur d'ANTHROPIC_LOG_FILE (peut être "" ou un chemin inexistant)
LT_SINK=""
LT_RESULT=""
run_detector() {
    RESULT="$RESULT_SEED"
    PERSISTENT_STDERR="$FIXTURES/persistent.txt"
    : > "$PERSISTENT_STDERR"
    ANTHROPIC_LOG_FILE="$1"
    _detect_lean_tail_regression 2>/dev/null
    LT_SINK="$(cat "$PERSISTENT_STDERR")"
    LT_RESULT="$RESULT"
}

# ----------------------------------------------------------------------------
# Les fixtures. Chaque ligne est une ligne JSONL de transcript ; la forme de la
# clé `response_body` et l'espacement du bloc `tool_use` sont ceux des témoins.
# ----------------------------------------------------------------------------
SIG_SIMPLIFY='{"schema_version": "v1", "timestamp": "2026-10-13T11:02:44Z", "response_body": {"content": [{"type": "tool_use", "name": "Skill", "input": {"skill": "compound-engineering:ce-simplify-code", "args": ""}}]}}'
SIG_REVIEW='{"schema_version": "v1", "timestamp": "2026-10-13T11:41:08Z", "response_body": {"content": [{"type": "tool_use", "name": "Skill", "input": {"skill": "compound-engineering:ce-code-review", "args": ""}}]}}'
SIG_BOTH='{"schema_version": "v1", "response_body": {"content": [{"type": "tool_use", "name": "Skill", "input": {"skill": "compound-engineering:ce-simplify-code"}}, {"type": "tool_use", "name": "Skill", "input": {"skill": "compound-engineering:ce-code-review"}}]}}'
SIG_TWICE='{"schema_version": "v1", "response_body": {"content": [{"type": "tool_use", "name": "Skill", "input": {"skill": "compound-engineering:ce-simplify-code"}}, {"type": "tool_use", "name": "Skill", "input": {"skill": "compound-engineering:ce-simplify-code"}}]}}'
# La forme COMPACTE : ce que `jq -c` et `json.dumps(separators=(",", ":"))`
# produisent tous deux, et la forme sur laquelle le grooming de mika#2694 a
# obtenu 0 fichier sur 44.
SIG_COMPACT='{"schema_version":"v1","response_body":{"content":[{"type":"tool_use","name":"Skill","input":{"skill":"compound-engineering:ce-simplify-code"}}]}}'
# De la PROSE qui nomme le skill : la forme du corps de mika#2694, du document
# de #2728, de CLAUDE.md et du plan de ce ticket.
PROSE='{"schema_version": "v1", "response_body": {"content": [{"type": "text", "text": "mika#2694 a retiré compound-engineering:ce-simplify-code et compound-engineering:ce-code-review de la fin de pipeline des pilotes ; ce ticket livre le détecteur de leur retour."}]}}'
NO_SIG='{"schema_version": "v1", "response_body": {"content": [{"type": "tool_use", "name": "Bash", "input": {"command": "cargo test --workspace"}}]}}'
# Un appel `Skill` pour un AUTRE skill, puis la mention en prose 300+ caractères
# plus loin : la fixture qui rouvrirait N1 si l'ancrage était relâché en fenêtre.
PAD='Le vol a commité, poussé, vérifié le pipeline et ouvert sa PR dans le budget de tours, conformément à claude-pilot#289 ; le corps de PR a été écrit sous le worktree puis supprimé, et la branche porte le plan groomé. Rien de la fin de pipeline allégée par mika#2694 n a été réinvoqué pendant ce vol, et la PR le dit.'
SKILL_OTHER_THEN_PROSE="{\"schema_version\": \"v1\", \"response_body\": {\"content\": [{\"type\": \"tool_use\", \"name\": \"Skill\", \"input\": {\"skill\": \"compound-engineering:ce-commit-push-pr\"}}, {\"type\": \"text\", \"text\": \"${PAD} compound-engineering:ce-simplify-code\"}]}}"

# ============================================================================
# P1-P4 : LE CONTRÔLE POSITIF — critère 3 du ticket
#
# « Un détecteur d'absence qui ne démontre pas qu'il sait trouver l'occurrence
# quand elle existe ne mesure pas le comportement, il mesure sa propre
# signature. »
# ============================================================================
echo ""
echo "P1-P4: le détecteur TROUVE l'occurrence quand elle existe"
echo "---------------------------------------------------------"

run_detector "$(make_transcript p1 "$NO_SIG" "$SIG_SIMPLIFY" "$NO_SIG")"
assert_contains "P1: ce-simplify-code dans l'espacement du témoin → régression" \
    "lean_tail_regression" "$LT_SINK"
assert_contains "P1: le compte de ce-simplify-code est 1" "simplify=1" "$LT_SINK"
assert_contains "P1: et celui de ce-code-review est 0" "code_review=0" "$LT_SINK"

run_detector "$(make_transcript p2 "$SIG_REVIEW")"
assert_contains "P2: ce-code-review seul → régression" "lean_tail_regression" "$LT_SINK"
assert_contains "P2: le compte de ce-code-review est 1" "code_review=1" "$LT_SINK"
assert_contains "P2: ce-simplify-code reste à 0" "simplify=0" "$LT_SINK"

P3_PATH="$(make_transcript p3 "$SIG_BOTH")"
assert_eq "P3: la fixture tient sur UNE SEULE ligne (c'est tout le propos)" "1" \
    "$(wc -l < "$P3_PATH" | tr -d '[:space:]')"
run_detector "$P3_PATH"
assert_contains "P3: les deux skills sur une ligne sont tous deux comptés" \
    "simplify=1 code_review=1" "$LT_SINK"

run_detector "$(make_transcript p4 "$SIG_TWICE")"
assert_contains "P4: deux occurrences sur une ligne comptent 2, pas 1" \
    "simplify=2" "$LT_SINK"

# ============================================================================
# N1-N6 : chaque contrôle négatif est l'un des faux résultats MESURÉS
# ============================================================================
echo ""
echo "N1-N6: chaque faux résultat mesuré est refusé"
echo "---------------------------------------------"

# N1 — la propriété 1 du ticket. Un détecteur sur la sous-chaîne nue crie sur
# tout document qui PARLE de ce ticket. Mesuré : la session d'observation de
# #2728 était dans la population observée.
run_detector "$(make_transcript n1 "$PROSE")"
assert_contains "N1: une MENTION en prose ne déclenche rien (ancrage)" \
    "lean_tail_clean" "$LT_SINK"
assert_not_contains "N1: et surtout pas la régression" "lean_tail_regression" "$LT_SINK"

# N2 — la propriété 2 du ticket. La forme sans espaces a rendu 0 fichier sur 44
# au grooming de mika#2694 : un faux négatif complet qui se lisait exactement
# comme le résultat espéré.
run_detector "$(make_transcript n2 "$SIG_COMPACT")"
assert_contains "N2: la forme COMPACTE est reconnue (le faux négatif 0/44)" \
    "lean_tail_regression" "$LT_SINK"
assert_contains "N2: et comptée" "simplify=1" "$LT_SINK"

# N3 — le nominal.
run_detector "$(make_transcript n3 "$NO_SIG" "$NO_SIG")"
assert_contains "N3: un transcript sans occurrence est propre" "lean_tail_clean" "$LT_SINK"
assert_contains "N3: avec les deux comptes à zéro, explicitement" \
    "simplify=0 code_review=0" "$LT_SINK"

# N4/N5 — mika#2331 : un `null` n'est jamais un `0`. Sans ces deux cas,
# « l'instrument veille et ne trouve rien » et « l'instrument ne regarde rien »
# seraient indistinguables (classe mika#2205).
run_detector ""
assert_contains "N4: ANTHROPIC_LOG_FILE vide → non mesuré" "lean_tail_unmeasured" "$LT_SINK"
assert_not_contains "N4: 'absent' ne se lit PAS comme 'propre'" "lean_tail_clean" "$LT_SINK"

run_detector "$FIXTURES/ce-chemin-nexiste-pas.jsonl"
assert_contains "N5: un chemin inexistant → non mesuré" "lean_tail_unmeasured" "$LT_SINK"
assert_not_contains "N5: 'absent' ne se lit PAS comme 'propre' non plus" \
    "lean_tail_clean" "$LT_SINK"

# N6 — l'ancrage n'est pas une fenêtre. Un `"name": "Skill"` légitime et la
# mention 300+ caractères plus loin ne doivent pas s'apparier entre eux, sans
# quoi N1 se rouvrirait par la porte de derrière.
run_detector "$(make_transcript n6 "$SKILL_OTHER_THEN_PROSE")"
assert_contains "N6: un autre skill + la mention 300 car. plus loin → propre" \
    "lean_tail_clean" "$LT_SINK"
assert_not_contains "N6: l'ancrage exige l'ADJACENCE, pas la cohabitation" \
    "lean_tail_regression" "$LT_SINK"

# ============================================================================
# S : le sink et RESULT, épinglés
#
# Le sink n'est pas négociable : la garde tourne APRÈS le lancement, donc hors de
# la redirection 2>"$STDERR_FILE", et l'exécuteur ne lit le stderr propre de
# dispatch-lib que sur !status.success(). Un dispatch implement RÉUSSIT.
# ============================================================================
echo ""
echo "S: le sink durable et le canal d'alerte"
echo "---------------------------------------"

run_detector "$(make_transcript s1 "$SIG_BOTH")"
assert_contains "S1: la régression atteint le stderr DURABLE" \
    "lean_tail_regression" "$LT_SINK"
assert_contains "S1: et elle atteint RESULT, donc le callback" \
    "LEAN TAIL REGRESSION (mika#2729)" "$LT_RESULT"
assert_contains "S1: RESULT porte les deux comptes" "ce-simplify-code x1" "$LT_RESULT"
assert_contains "S1: le texte préexistant de RESULT est intact" \
    "claude-pilot completed (status: success)." "$LT_RESULT"

run_detector "$(make_transcript s2 "$NO_SIG")"
assert_contains "S2: 'propre' atteint le stderr durable" "lean_tail_clean" "$LT_SINK"
assert_eq "S2: et n'entre PAS dans RESULT (une ligne nominale par dispatch est du bruit)" \
    "$RESULT_SEED" "$LT_RESULT"

run_detector ""
assert_contains "S3: 'non mesuré' atteint le stderr durable" "lean_tail_unmeasured" "$LT_SINK"
assert_eq "S3: et n'entre PAS dans RESULT" "$RESULT_SEED" "$LT_RESULT"

# Statique : un seul site déclare la signature, et le site d'appel est AVANT la
# queue de logs, donc avant le `head -c 92000` qui coupe la fin de RESULT.
echo ""
echo "S4-S6 (statique): un seul site de vérité, un seul site d'appel, au bon rang"
echo "--------------------------------------------------------------------------"
assert_eq "S4: exactement UN site déclare _LEAN_TAIL_SIG" "1" \
    "$(grep -c '^_LEAN_TAIL_SIG=' "$DISPATCH_LIB")"
assert_eq "S5: exactement UN site appelle le détecteur" "1" \
    "$(grep -c '^    _detect_lean_tail_regression$' "$DISPATCH_LIB")"
_LT_CALL_LINE="$(grep -n '^    _detect_lean_tail_regression$' "$DISPATCH_LIB" | cut -d: -f1)"
_LT_LOGS_LINE="$(grep -n '^Logs (last 10KB):$' "$DISPATCH_LIB" | cut -d: -f1)"
assert_eq "S6: l'appel précède la queue de logs, donc survit à head -c 92000" "1" \
    "$( [ -n "$_LT_CALL_LINE" ] && [ -n "$_LT_LOGS_LINE" ] && [ "$_LT_CALL_LINE" -lt "$_LT_LOGS_LINE" ] && echo 1 || echo 0 )"

# ============================================================================
# X1-X3 : LES PROBES DE MUTATION — l'instrument montré porteur
#
# Armement et lecture sont DÉLIBÉRÉMENT deux fonctions, et ce découpage est toute
# la raison pour laquelle ces probes veulent dire quelque chose.
#
# `arm_mutant` tourne dans le shell courant pour que son verdict atteigne les
# compteurs et l'écran de l'opérateur. Une fonction unique rendant le token
# devrait être appelée en substitution de commande, ce qui mettrait son `✗` et
# son `FAIL=$((FAIL+1))` dans un sous-shell : l'assertion anti-vacuité serait
# capturée dans la variable de l'appelant, invisible et sans effet, et un `sed`
# qui n'apparie rien laisserait le comportement sain en place tout en faisant
# rapporter à la probe une différence qu'elle n'a jamais causée. C'est la forme
# mika#2205 survenant à l'intérieur de la suite écrite pour la refuser.
# ============================================================================
echo ""
echo "X1-X3: chaque moitié de l'instrument est montrée porteuse"
echo "---------------------------------------------------------"

MUTANT_LIB=""
arm_mutant() {
    local label="$1" sed_expr="$2"
    MUTANT_LIB="$FIXTURES/mutant-$(printf '%s' "$label" | tr -c 'A-Za-z0-9' '_').sh"
    sed "$sed_expr" "$DISPATCH_LIB" > "$MUTANT_LIB"
    if cmp -s "$MUTANT_LIB" "$DISPATCH_LIB"; then
        FAIL=$((FAIL + 1))
        echo "  ✗ $label: la mutation n'a rien apparié — cette probe ne prouverait rien"
        MUTANT_LIB=""
    else
        PASS=$((PASS + 1)); echo "  ✓ $label: mutation appliquée (probe armée)"
    fi
}

# Rejoue un transcript contre le mutant armé et rend la ligne qu'il a émise. Sans
# mutant armé, rend une sentinelle qui ne satisfait aucune assertion ci-dessous,
# donc une probe désarmée rougit deux fois plutôt que de passer en silence.
mutant_line() {
    local transcript="$1"
    [ -n "$MUTANT_LIB" ] || { printf 'PROBE-DESARMEE-LA-MUTATION-NA-PAS-PRIS'; return 0; }
    (
        # shellcheck source=/dev/null
        source "$MUTANT_LIB" >/dev/null 2>&1
        RESULT="$RESULT_SEED"
        PERSISTENT_STDERR="$FIXTURES/mutant-persistent.txt"
        : > "$PERSISTENT_STDERR"
        ANTHROPIC_LOG_FILE="$transcript"
        _detect_lean_tail_regression 2>/dev/null
        cat "$PERSISTENT_STDERR"
    )
}

X_PROSE="$(make_transcript x-prose "$PROSE")"
X_COMPACT="$(make_transcript x-compact "$SIG_COMPACT")"
X_SPACED="$(make_transcript x-spaced "$SIG_SIMPLIFY")"

# X1 — le motif réduit à la sous-chaîne nue ⇒ N1 doit devenir ROUGE : la prose
# est alors comptée comme une invocation. C'est l'assertion auto-nettoyante de la
# Fire-Disposition : le jour où le motif se désancre — élargi, ou « réparé » par
# relâchement — N1 rougit, et X1 atteste que c'est bien N1 qui porte cette
# garantie.
#
# LA MUTATION PORTE SUR `_lean_tail_count`, PAS SUR `_LEAN_TAIL_SIG`, et ce
# détail a été mesuré pendant la construction de cette probe : l'ancrage est
# composé de DEUX morceaux, le préfixe des trois clés ET le guillemet fermant que
# `_lean_tail_count` ajoute derrière le nom du skill. Réduire le seul préfixe
# laissait encore le `"` final refuser la prose, et la probe passait pour la
# mauvaise raison. Muter le site de composition désancre l'instrument ENTIER —
# c'est-à-dire le `grep -c 'ce-simplify-code'` naïf que la propriété 1 du ticket
# interdit.
arm_mutant "X1" 's|"${_LEAN_TAIL_SIG}$1\\""|"$1"|'
X1="$(mutant_line "$X_PROSE")"
assert_contains "X1: désancré, le détecteur crie sur la prose — N1 est donc porteur" \
    "lean_tail_regression" "$X1"
# Et le même désancrage rouvre N6, qui est N1 par la porte de derrière.
X1_N6="$(mutant_line "$(make_transcript x-n6 "$SKILL_OTHER_THEN_PROSE")")"
assert_contains "X1: il rouvre aussi N6 (la cohabitation sans adjacence)" \
    "lean_tail_regression" "$X1_N6"

# X2 — l'espacement épinglé sur la forme exacte du témoin ⇒ N2 doit devenir
# ROUGE : la forme compacte redevient invisible, et c'est le piège 5 de #2728
# reconstitué à l'identique.
arm_mutant "X2" "s|^_LEAN_TAIL_SIG=.*|_LEAN_TAIL_SIG='\"name\": \"Skill\", \"input\": [{]\"skill\": \"compound-engineering:'|"
X2_COMPACT="$(mutant_line "$X_COMPACT")"
assert_contains "X2: espacement rigide → la forme compacte redevient invisible (N2 porteur)" \
    "lean_tail_clean" "$X2_COMPACT"
assert_not_contains "X2: le faux négatif 0/44 est exactement ce qui revient" \
    "lean_tail_regression" "$X2_COMPACT"
# Anti-vacuité de X2 : le mutant doit rester capable de trouver la forme ESPACÉE,
# sans quoi X2 ne prouverait que « le motif a été cassé », pas « la tolérance
# d'espacement est la moitié porteuse ».
X2_SPACED="$(mutant_line "$X_SPACED")"
assert_contains "X2: le mutant trouve toujours la forme espacée (la mutation est ciblée)" \
    "lean_tail_regression" "$X2_SPACED"

# X3 — le token d'absence remplacé par celui du nominal ⇒ N4 doit devenir ROUGE :
# l'absence se remet à se lire comme un zéro (mika#2331).
arm_mutant "X3" "s|lean_tail_unmeasured|lean_tail_clean|"
X3="$(mutant_line "")"
assert_not_contains "X3: sans son propre token, l'absence disparaît — N4 est porteur" \
    "lean_tail_unmeasured" "$X3"
assert_contains "X3: et se remet à se lire comme un zéro mesuré" "lean_tail_clean" "$X3"

# ============================================================================
# Summary
# ============================================================================
echo ""
echo "===================================================================="
echo "Passed: $PASS   Failed: $FAIL"
echo "===================================================================="
[ "$FAIL" -eq 0 ] || exit 1
