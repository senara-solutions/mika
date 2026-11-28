#!/bin/bash
# Les passes architecte et leur wrapper de relance — extrait de test-dispatch-lib.sh par mika#3178, vol 4.
# `_arch_ask` déclare la passe qu'il exécute (mika#2363) ; le contrat de session
# des deux passes, épinglé (mika#2305) ; le wrapper `_arch_ask_with_retry`
# interposé entre la boucle et `_arch_ask`, et le brief tué par un restart qui
# est ré-émis (mika#2278), avec son contrôle négatif ; le compte d'appels passé
# de 4 à 6 par les deux relances de seconde passe (mika#2641) ; les deux moitiés
# du code de sortie retryable qui sont le même nombre (mika#2522).
#
# Plage d'origine : lignes 8773–9153 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 3849d757. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ============================================================================
# mika#2363 — `_arch_ask` déclare la passe qu'il exécute
# ============================================================================
#
# mika-arch porte trois skills `always_on` (39 798 o de prompt) et un tour n'en
# exécute qu'une : ~23,5 Ko de chaque prompt système architecte décrivaient deux
# tâches que le tour ne fait pas. `--only-skill` est le canal — strictement
# soustractif — qui le dit au serveur.
#
# Test comportemental : on éclipse `mika` par une fonction shell qui rend son
# argv, puis on appelle le vrai `_arch_ask`. Une assertion sur le texte du
# fichier prouverait que la ligne existe ; celle-ci prouve ce que la commande
# *émet*.

echo ""
echo "Test: _arch_ask — déclaration de passe (mika#2363)"
echo "---------------------------------------------------"

# _arch_ask_argv <skill> [session_id] → l'argv que `_arch_ask` remet à `mika`.
_arch_ask_argv() {
    local skill="$1" session_id="${2:-}"
    local tmp plan
    tmp=$(mktemp -d)
    plan="$tmp/plan.md"
    echo "# Plan: fixture mika#2363" > "$plan"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        # Éclipse le binaire : une fonction shell l'emporte sur $PATH.
        mika() { printf '%s\n' "$*"; }
        _arch_ask "$skill" "$plan" "$session_id"
    )
    rm -rf "$tmp"
}

# Idem, mais à travers le wrapper de retry (mika#2278). Sert à prouver que le
# wrapper transmet ses trois arguments verbatim : les gardes mika#2305 portent
# désormais sur des appels au wrapper, et sans cette assertion elles
# épingleraient une session qui pourrait ne jamais atteindre `mika ask`.
_arch_ask_argv_via_wrapper() {
    local skill="$1" session_id="${2:-}"
    local tmp plan
    tmp=$(mktemp -d)
    plan="$tmp/plan.md"
    echo "# Plan: fixture mika#2278" > "$plan"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        mika() { printf '%s\n' "$*"; }
        _arch_ask_with_retry "$skill" "$plan" "$session_id"
    )
    rm -rf "$tmp"
}

# Les trois passes, une par une. Le test échouerait aussi bien sur une skill
# oubliée que sur un `--only-skill` retiré.
for _MIKA2363_SKILL in mika-arch-groom-ticket mika-arch-second-review mika-arch-groom-milestone; do
    _MIKA2363_ARGV=$(_arch_ask_argv "$_MIKA2363_SKILL")
    assert_contains "mika#2363: _arch_ask déclare $_MIKA2363_SKILL" \
        "--only-skill $_MIKA2363_SKILL" "$_MIKA2363_ARGV"

    # `--enable-skill` et `--only-skill` sont exclusifs côté clap : les garder
    # tous les deux ferait mourir chaque appel architecte au parsing d'arguments.
    assert_not_contains "mika#2363: $_MIKA2363_SKILL — plus de --enable-skill (exclusif de --only-skill)" \
        "--enable-skill" "$_MIKA2363_ARGV"

    # La passe demandée est la seule nommée. Le vocabulaire des trois skills
    # vit dans MIKA_ARCH_SKILL_ALLOWLIST (well_known_agents.rs) ; une seconde
    # copie en shell dériverait en silence.
    for _MIKA2363_SISTER in mika-arch-groom-ticket mika-arch-second-review mika-arch-groom-milestone; do
        [ "$_MIKA2363_SISTER" = "$_MIKA2363_SKILL" ] && continue
        assert_not_contains "mika#2363: $_MIKA2363_SKILL n'énumère pas $_MIKA2363_SISTER" \
            "$_MIKA2363_SISTER" "$_MIKA2363_ARGV"
    done
done

# La continuation de session reste orthogonale à la déclaration de passe : la
# seconde passe relit la première dans la même session architecte.
_MIKA2363_ARGV_SESSION=$(_arch_ask_argv "mika-arch-second-review" "sess-2363")
assert_contains "mika#2363: --session-id survit à la déclaration de passe" \
    "--session-id sess-2363" "$_MIKA2363_ARGV_SESSION"
assert_contains "mika#2363: --only-skill survit à --session-id" \
    "--only-skill mika-arch-second-review" "$_MIKA2363_ARGV_SESSION"

# =============================================================================
# mika#2305 — le contrat de session des passes architecte, épinglé
# =============================================================================
#
# Le ticket soupçonnait une FUITE : « le contexte de mika-arch semble porté d'une
# passe à l'autre SANS --session-id explicite ». La lecture du code dit l'inverse
# sur cet axe — le portage intra-invocation est explicite, délibéré et commenté,
# et c'est même le contrat que la « Piste » du ticket appelle de ses vœux. La
# fuite réelle vivait dans l'assembleur de prompt (HistoryScope::Agent, refermée
# par mika#2295 + mika#2330), pas ici.
#
# D'où ces assertions : elles n'ajoutent aucun comportement, elles empêchent
# qu'une prochaine lecture « corrige » le portage en croyant fermer #2305. Le
# retry UNPARSED en particulier ne re-demande pas la revue — il demande à
# l'architecte de COMPLÉTER sa propre réponse en y ajoutant la ligne
# `Disposition:` manquante. Sans la session, la demande serait inintelligible :
# le portage y est la condition de correction du mécanisme, pas sa contamination.

# --- Moitié 1 : _arch_ask ne pose --session-id que si $3 est non vide ---

_MIKA2305_ARGV_FRESH=$(_arch_ask_argv "mika-arch-groom-ticket")
assert_not_contains "mika#2305: session neuve par défaut (pas de \$3 → pas de --session-id)" \
    "--session-id" "$_MIKA2305_ARGV_FRESH"

_MIKA2305_ARGV_CONT=$(_arch_ask_argv "mika-arch-groom-ticket" "sess-2305")
assert_contains "mika#2305: continuation sur demande explicite (\$3 → --session-id)" \
    "--session-id sess-2305" "$_MIKA2305_ARGV_CONT"

# --- Moitié 2 : les trois usages de _iterate_groom_loop ---
#
# `_iterate_groom_loop` ne s'exécute pas en isolation (il veut gh, un plan, un
# ticket), donc le contrat est lu sur la source. Le prédicat porte sur la
# PRÉSENCE d'un troisième argument à l'appel, ce qui est exactement ce que
# `_arch_ask` teste à la ligne 4688.

_MIKA2305_BODY=$(awk '/^_iterate_groom_loop\(\) \{/,/^\}/' "$DISPATCH_LIB")
assert_contains "mika#2305: le corps de _iterate_groom_loop est lisible" \
    "_arch_ask" "$_MIKA2305_BODY"

# mika#2278 a interposé `_arch_ask_with_retry` entre la boucle et `_arch_ask` :
# le prédicat porte donc sur `_arch_ask*` et non plus sur `_arch_ask` seul. Le
# contrat épinglé, lui, est inchangé — c'est le TROISIÈME argument qui compte,
# pas le nom de la fonction. Le wrapper transmet `$3` verbatim (couvert par le
# test de sa propre section), donc porter la session à son niveau la porte
# jusqu'à `mika ask`.
_MIKA2305_ANY_CALL='_arch_ask\(_with_retry\)\? "'

# 1ʳᵉ passe : session neuve. C'est le SEUL appel architecte de la boucle qui ne
# nomme pas "$session_id" — d'où un prédicat sur l'absence plutôt que sur la
# forme exacte de la fin de ligne.
_MIKA2305_CALLS=$(printf '%s\n' "$_MIKA2305_BODY" | grep "$_MIKA2305_ANY_CALL" || true)
_MIKA2305_FRESH_CALLS=$(printf '%s\n' "$_MIKA2305_CALLS" | grep -v '"\$session_id"' || true)
assert_eq "mika#2305: un seul appel architecte sans session portée" \
    "1" "$(printf '%s\n' "$_MIKA2305_FRESH_CALLS" | grep -c "$_MIKA2305_ANY_CALL" || true)"
assert_contains "mika#2305: et c'est la 1ʳᵉ passe groom-ticket sur le plan" \
    '_arch_ask_with_retry "mika-arch-groom-ticket" "$plan_path"' "$_MIKA2305_FRESH_CALLS"

# Retry UNPARSED : session portée, pour que l'architecte voie sa propre réponse
# et puisse la compléter.
_MIKA2305_RETRY=$(printf '%s\n' "$_MIKA2305_BODY" \
    | grep -c '_arch_ask_with_retry "mika-arch-groom-ticket" "\$retry_prompt" "\$session_id"' || true)
assert_eq "mika#2305: le retry UNPARSED continue la session (D6)" \
    "1" "$_MIKA2305_RETRY"

# 2ᵉ passe : session portée, sur les deux branches (après READY et après ITERATE).
# Le contrat de continuité de session est déclaré par mika-arch-second-review.
_MIKA2305_SECOND=$(printf '%s\n' "$_MIKA2305_BODY" \
    | grep -c '_arch_ask_with_retry "mika-arch-second-review" "\$plan_path" "\$session_id"' || true)
assert_eq "mika#2305: les deux branches de 2ᵉ passe continuent la session" \
    "2" "$_MIKA2305_SECOND"

# Retry UNPARSED de SECONDE passe (mika#2641) : session portée, sur les deux
# branches. LA DÉCISION, prise ici parce que le bloc ci-dessous l'exige — « un
# cinquième appel est un halt-and-surface ; il porte ou ne porte pas la session,
# et c'est une décision ».
#
# Elle porte la session, et pour la raison EXACTE déjà écrite pour le retry de
# première passe : la relance ne re-demande pas la revue, elle demande à
# l'architecte de COMPLÉTER sa propre réponse en y ajoutant la ligne `Verdict:`
# manquante. Sans la session la demande serait inintelligible — le portage y est
# la condition de correction du mécanisme, pas sa contamination. C'est aussi ce
# que le contrat de continuité de `mika-arch-second-review` exige.
#
# Le prédicat ne nomme pas le fichier temporaire ($retry2 / $retry2i) mais la
# POSITION du troisième argument : ce qui est épinglé est le portage, pas le nom
# de la variable locale.
_MIKA2641_SECOND_RETRY=$(printf '%s\n' "$_MIKA2305_BODY" \
    | grep -cE '_arch_ask_with_retry "mika-arch-second-review" "\$retry2i?" "\$session_id"' || true)
assert_eq "mika#2641: les deux relances de 2ᵉ passe continuent la session (D6)" \
    "2" "$_MIKA2641_SECOND_RETRY"

# Aucun appel architecte n'échappe à l'inventaire ci-dessus : 1 + 1 + 2 + 2 = 6.
# Un septième appel est un halt-and-surface — il porte ou ne porte pas la
# session, et c'est une décision, pas une ligne à ajouter au compte.
_MIKA2305_TOTAL=$(printf '%s\n' "$_MIKA2305_BODY" | grep -c "$_MIKA2305_ANY_CALL" || true)
assert_eq "mika#2305: inventaire clos des appels _arch_ask (1 neuf + 5 continués)" \
    "6" "$_MIKA2305_TOTAL"

# mika#2278 : la boucle passe TOUJOURS par le wrapper. Un appel direct à
# `_arch_ask` y serait un site sans retry et sans capture stderr — exactement
# les deux défauts composés que ce ticket ferme, réintroduits sur un seul site
# pendant que les trois autres restent corrects.
_MIKA2305_BARE=$(printf '%s\n' "$_MIKA2305_BODY" | grep -c '_arch_ask "' || true)
assert_eq "mika#2278: aucun appel direct à _arch_ask ne subsiste dans la boucle" \
    "0" "$_MIKA2305_BARE"

# Et le wrapper transmet le troisième argument verbatim : sans ça, les
# assertions ci-dessus épingleraient une session qui n'atteint pas `mika ask`.
_MIKA2278_WRAPPER_FRESH=$(_arch_ask_argv_via_wrapper "mika-arch-groom-ticket")
assert_not_contains "mika#2278: le wrapper préserve la session neuve" \
    "--session-id" "$_MIKA2278_WRAPPER_FRESH"
_MIKA2278_WRAPPER_CONT=$(_arch_ask_argv_via_wrapper "mika-arch-second-review" "sess-2278")
assert_contains "mika#2278: le wrapper transmet la session continuée" \
    "--session-id sess-2278" "$_MIKA2278_WRAPPER_CONT"

# ============================================================================
# mika#2278 — un brief architecte tué par un restart est ré-émis
# ============================================================================
#
# Mesuré le 2026-09-10 sur le groom de #2276 : un restart de `mika-spirit` a tué
# la passe architecte en vol, et rien ne l'a reprise. `mika ask` sort désormais
# en 75 (EX_TEMPFAIL) sur un échec de classe transport, et `_arch_ask_with_retry`
# réessaie une fois — et UNE SEULE FOIS, et SEULEMENT sur 75.
#
# Test comportemental : on éclipse `mika` par une fonction shell qui compte ses
# invocations et rend une séquence de codes imposée, puis on appelle le vrai
# `_arch_ask_with_retry`. Le compte d'invocations est ce qui porte la preuve :
# une assertion sur le seul code de sortie final passerait aussi bien sur un
# wrapper qui ne réessaie jamais.

echo ""
echo "Test: _arch_ask_with_retry — retry borné sur 75 (mika#2278)"
echo "------------------------------------------------------------"

# _mika2278_probe <codes-séparés-par-virgule> <MIKA_ARCH_ASK_RETRY>
# → "status=<code final> calls=<nombre d'invocations de mika>"
_mika2278_probe() {
    local codes="$1" retry_flag="$2"
    local tmp plan calls
    tmp=$(mktemp -d)
    plan="$tmp/plan.md"
    echo "# Plan: fixture mika#2278" > "$plan"
    calls="$tmp/calls"
    printf '0' > "$calls"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        # Le délai réel est de 30 s ; la sonde n'en mesure pas la valeur, elle
        # mesure le nombre d'appels. 1 s garde la suite rapide sans changer ce
        # qui est asserté.
        export MIKA_ARCH_ASK_RETRY_DELAY_SECS=1
        export MIKA_ARCH_ASK_RETRY="$retry_flag"
        # Éclipse le binaire : une fonction shell l'emporte sur $PATH.
        mika() {
            local n code
            n=$(cat "$calls")
            n=$((n + 1))
            printf '%s' "$n" > "$calls"
            code=$(printf '%s' "$codes" | cut -d, -f"$n")
            [ -n "$code" ] || code=0
            if [ "$code" -eq 0 ]; then
                printf '{"content":"ok","metadata":{"session_id":"s-2278"}}'
            else
                echo "mika ask to http://127.0.0.1:8080/a2a/mika-arch failed: unreachable" >&2
            fi
            return "$code"
        }
        local out status
        if out=$(_arch_ask_with_retry "mika-arch-groom-ticket" "$plan" 2>/dev/null); then
            status=0
        else
            status=$?
        fi
        printf 'status=%s calls=%s out=%s' "$status" "$(cat "$calls")" "$out"
    )
    rm -rf "$tmp"
}

# Cas positif : un 75 puis un 0 → deux invocations, succès final, stdout intact.
_MIKA2278_RETRIED=$(_mika2278_probe "75,0" "1")
assert_contains "mika#2278: un 75 est réessayé exactement une fois" \
    "calls=2" "$_MIKA2278_RETRIED"
assert_contains "mika#2278: et le retry aboutit (statut final 0)" \
    "status=0" "$_MIKA2278_RETRIED"
assert_contains "mika#2278: le stdout du retry remonte intact à l'appelant" \
    'out={"content":"ok"' "$_MIKA2278_RETRIED"

# Contrôle négatif — c'est celui qui porte la preuve. Sans lui, un wrapper qui
# réessaierait sur n'importe quel code non nul passerait le cas positif.
_MIKA2278_CONTRACT=$(_mika2278_probe "1,0" "1")
assert_contains "mika#2278: un échec de contrat (1) n'est JAMAIS réessayé" \
    "calls=1" "$_MIKA2278_CONTRACT"
assert_contains "mika#2278: et son code de sortie est propagé tel quel" \
    "status=1" "$_MIKA2278_CONTRACT"

# Fail-safe D4 : un code qu'on ne sait pas lire est traité comme définitif.
# L'inverse ferait d'un futur mode d'échec imprévu une boucle de réessais.
_MIKA2278_UNKNOWN=$(_mika2278_probe "2,0" "1")
assert_contains "mika#2278: un code inconnu (2) n'est pas retryable (D4)" \
    "calls=1" "$_MIKA2278_UNKNOWN"

# Contrôle négatif de désarmement (R7) : la variable doit mordre.
_MIKA2278_DISARMED=$(_mika2278_probe "75,0" "0")
assert_contains "mika#2278: MIKA_ARCH_ASK_RETRY=0 désarme le retry" \
    "calls=1" "$_MIKA2278_DISARMED"
assert_contains "mika#2278: et le 75 est alors propagé à l'appelant" \
    "status=75" "$_MIKA2278_DISARMED"

# Budget de UN : deux 75 d'affilée ne produisent pas un troisième appel.
_MIKA2278_EXHAUSTED=$(_mika2278_probe "75,75,0" "1")
assert_contains "mika#2278: le budget est de un retry, pas d'une boucle" \
    "calls=2" "$_MIKA2278_EXHAUSTED"
assert_contains "mika#2278: budget épuisé → le 75 est propagé" \
    "status=75" "$_MIKA2278_EXHAUSTED"

# Lecteurs d'environnement, trois paliers (R4).
_MIKA2278_DELAY_DEFAULT=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB"; unset MIKA_ARCH_ASK_RETRY_DELAY_SECS; _arch_ask_retry_delay_secs
)
assert_eq "mika#2278: délai par défaut = 30 s" "30" "$_MIKA2278_DELAY_DEFAULT"
_MIKA2278_DELAY_SET=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB"; MIKA_ARCH_ASK_RETRY_DELAY_SECS=45 _arch_ask_retry_delay_secs
)
assert_eq "mika#2278: délai réglé honoré" "45" "$_MIKA2278_DELAY_SET"
# `0` ne désarme PAS — c'est le rôle du kill-switch ; lu autrement, une coquille
# sur le délai restaurerait en silence le défaut que ce ticket ferme.
_MIKA2278_DELAY_ZERO=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB"; MIKA_ARCH_ASK_RETRY_DELAY_SECS=0 _arch_ask_retry_delay_secs 2>/dev/null
)
assert_eq "mika#2278: délai 0 retombe sur le défaut (ne désarme pas)" \
    "30" "$_MIKA2278_DELAY_ZERO"
_MIKA2278_DELAY_ABSURD=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB"; MIKA_ARCH_ASK_RETRY_DELAY_SECS=99999 _arch_ask_retry_delay_secs 2>/dev/null
)
assert_eq "mika#2278: délai absurde borné par le haut" "30" "$_MIKA2278_DELAY_ABSURD"

# Garde structurelle : le retour d'un `2>/dev/null` sur un appel architecte.
#
# Un test comportemental ne peut pas voir cette classe. La régression ne rendrait
# aucune décision fausse — le retry continuerait de fonctionner, toutes les
# assertions ci-dessus resteraient vertes — elle rendrait le diagnostic aveugle :
# l'opérateur relirait `first-pass _arch_ask failed` sans jamais apprendre
# pourquoi. Même motif que mika#2131.
_MIKA2278_LOOP_BODY=$(awk '/^_iterate_groom_loop\(\) \{/,/^\}/' "$DISPATCH_LIB")
_MIKA2278_SWALLOWED=$(printf '%s\n' "$_MIKA2278_LOOP_BODY" \
    | grep -c '_arch_ask[a-z_]* .*2>/dev/null' || true)
assert_eq "mika#2278: aucun appel architecte ne jette plus stderr (D7)" \
    "0" "$_MIKA2278_SWALLOWED"

# Et les SIX sites font remonter ce message dans leur WARN d'échec (R5).
#
# mika#2641 — le compte passe de 4 à 6 : les deux relances de seconde passe sont
# deux appels architecte de plus, donc deux sites d'échec de plus, et l'invariant
# R5 s'applique à eux à l'identique. Un WARN de relance qui ne porterait pas le
# message du CLI laisserait l'opérateur relire « second-pass retry _arch_ask
# failed » sans jamais apprendre pourquoi — exactement la cécité que ce test
# existe pour refuser.
_MIKA2278_SUFFIXED=$(printf '%s\n' "$_MIKA2278_LOOP_BODY" \
    | grep -c '_groom_warn ".*_arch_ask failed.*_arch_ask_error_suffix' || true)
assert_eq "mika#2278: les six WARN d'échec portent le message du CLI (R5)" \
    "6" "$_MIKA2278_SUFFIXED"

# ===========================================================================
# mika#2522 — les deux moitiés du code de sortie retryable sont le même nombre
# ===========================================================================
#
# Le retry de mika#2278 ne rejoue que sur `75`, et mika#2522 fait arriver un `75`
# là où arrivait un `1` : la classe `transport` attestée par le serveur devient
# `FailureClass::Transport` côté CLI, donc `EXIT_TRANSPORT_FAILURE`. Ce ticket ne
# touche PAS `dispatch-lib.sh` — tout ce que le shell doit garantir, c'est que la
# constante qu'il lit est bien celle que le CLI écrit.
#
# Le côté Rust épingle déjà le littéral (`assert_eq!(EXIT_TRANSPORT_FAILURE, 75)`
# dans `remote_ask.rs`). Aucun des deux tests ne voit l'autre : deux moitiés
# épinglées séparément sur deux littéraux peuvent diverger en silence, et le seul
# symptôme serait un retry qui cesse de s'armer — c'est-à-dire exactement le
# défaut que mika#2522 ferme, revenu par la porte de derrière. Cette assertion est
# la jointure.
_MIKA2522_REMOTE_ASK_RS="$SCRIPT_DIR/../../../crates/mika-cli/src/remote_ask.rs"
_MIKA2522_CLI_EXIT=$(grep -oE 'pub const EXIT_TRANSPORT_FAILURE: i32 = [0-9]+' \
    "$_MIKA2522_REMOTE_ASK_RS" 2>/dev/null | grep -oE '[0-9]+$' || true)
# Anti-vacuité : source déplacée, constante renommée, `grep` qui ne rend rien —
# chacun donnerait une chaîne vide, et comparer deux vides passerait. Un prédicat
# qui ne lit rien se lit exactement comme un prédicat sain (classe mika#2205).
assert_eq "mika#2522: la constante du CLI a bien été extraite de sa source" \
    "extracted" "$([ -n "$_MIKA2522_CLI_EXIT" ] && echo extracted || echo empty)"
_MIKA2522_SHELL_EXIT=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB"; printf '%s' "$_ARCH_ASK_RETRYABLE_EXIT"
)
assert_eq "mika#2522: _ARCH_ASK_RETRYABLE_EXIT == EXIT_TRANSPORT_FAILURE du CLI" \
    "$_MIKA2522_CLI_EXIT" "$_MIKA2522_SHELL_EXIT"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
