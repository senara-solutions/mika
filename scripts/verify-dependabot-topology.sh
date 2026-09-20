#!/usr/bin/env bash
# verify-dependabot-topology.sh — établir, par lecture, si les trois dépôts
# autonomes portent un `.github/dependabot.yml`, et ce que chacun déclare.
#
# TICKET : mika#1997, qui ferme R1 / AC1 de mika#1729.
#
# LA PROPRIÉTÉ, pas une liste d'états.
#   Un dépôt sans `.github/dependabot.yml` est indistinguable, de l'extérieur,
#   d'un dépôt qui en porte un : aucune PR Dependabot ne s'ouvre, et une absence
#   de PR se lit exactement comme « il n'y avait rien à bumper cette semaine ».
#   C'est très précisément le défaut mesuré de mika#1997 : mika-qa avait relevé
#   `AC1 ❌ — dependabot.yml only on mika, missing from mika-cloud +
#   mika-platform`, la PR#1995 a été mergée avec le finding ouvert, et rien dans
#   la machine ne pouvait le redire. La seule façon d'apprendre la différence est
#   d'aller lire les trois dépôts et de rapporter ce qu'on y trouve.
#
# CE QU'IL NE FAIT PAS, et c'est délibéré.
#   Il ne juge pas la chaîne de revue : il n'ouvre rien, n'écrit rien, ne sonde
#   aucun upstream et ne dit rien de mika-qa. Il établit la **précondition** dont
#   l'absence était le défaut. « La revue autonome se déclenche bien sur les
#   trois » s'observe sur une vraie PR Dependabot (mika#1997, phase 3), pas ici.
#   Il ne juge pas non plus l'étalement des jours : il les met côte à côte, pour
#   que la collision se voie, et laisse le verdict à qui lit.
#
# CE N'EST PAS UN PARSEUR YAML, et il ne prétend pas l'être. Il extrait à plat
#   les trois clés que mika#1997 a besoin de voir (`package-ecosystem`, `day`,
#   `open-pull-requests-limit`). Un fichier valide écrit sous une forme YAML
#   inhabituelle (flow mapping, ancres) sera sous-rapporté plutôt que mal
#   rapporté : la sortie dira « aucun écosystème lu », qui est vrai de ce que le
#   script a su lire, et jamais « aucun écosystème déclaré ».
#
# USAGE
#   scripts/verify-dependabot-topology.sh                  # les trois dépôts
#   scripts/verify-dependabot-topology.sh owner/repo ...   # une liste explicite
#
# CODES DE SORTIE — trois verdicts, et le troisième n'est pas un succès.
#   0  les dépôts interrogés portent tous un fichier lisible qui déclare au
#      moins un écosystème.
#   1  il en manque au moins un — fait ÉTABLI : le dépôt est lisible et le
#      fichier n'y est pas, ou il y est et ne déclare rien (donc est inerte).
#   2  RIEN N'A PU ÊTRE ÉTABLI sur au moins un dépôt, et aucune absence n'a été
#      établie par ailleurs : `gh` manquant, non authentifié, jeton sans accès,
#      réseau, réponse illisible. Une vérification qui n'a pas pu s'authentifier
#      n'a rien vérifié, et le dire est la seule sortie honnête : la replier sur
#      0 ferait passer un dépôt nu pour un dépôt couvert, et sur 1 enverrait un
#      opérateur écrire un fichier peut-être déjà là.
#
#   L'ABSENCE ÉTABLIE PRIME SUR L'INDÉTERMINATION, et c'est un choix. Si un
#   dépôt est établi nu et un autre illisible, la sortie est 1 : « la couverture
#   n'est pas complète » est déjà vrai, quoi que dise le troisième. L'inverse
#   (rendre 2) ferait disparaître un fait derrière une incertitude.
#
# LE PIÈGE DU 404, fermé ici plutôt que découvert plus tard.
#   GitHub répond 404 — et non 403 — pour un dépôt privé auquel le jeton n'a pas
#   accès. Lu naïvement, un jeton mal portée ferait dire « le fichier manque »
#   d'un fichier peut-être présent, c'est-à-dire produirait la fausse mesure que
#   ce script existe pour empêcher. Donc sur 404 du fichier, et seulement là, on
#   fait un second appel : si le dépôt lui-même n'est pas lisible, le verdict est
#   INDÉTERMINÉ, pas ABSENT.

set -uo pipefail

readonly EXIT_ALL_PRESENT=0
readonly EXIT_MISSING=1
readonly EXIT_INDETERMINATE=2

# Les trois dépôts déjà autonomes (mika#1729 R1). Même population que
# `DISPATCHABLE_REPOS` (crates/mika-agent/src/webhook_dispatch.rs:102) moins
# mika-skills, qui n'est pas dans le périmètre de mika#1729.
readonly DEFAULT_REPOS=(
    "senara-solutions/mika"
    "senara-solutions/mika-cloud"
    "senara-solutions/mika-platform"
)

readonly DEPENDABOT_PATH=".github/dependabot.yml"

# Largeur du bloc de détail : 2 d'indentation + la colonne %-34s + 3. Les
# messages de `gh` sont multi-lignes (« gh auth login » en fait deux), et sans
# ré-indentation leur seconde ligne repart en colonne zéro, où elle se lit
# comme une sortie du vérificateur plutôt que comme une citation de `gh`.
DETAIL_INDENT="$(printf '%39s' '')"
readonly DETAIL_INDENT

usage() {
    cat >&2 <<'USAGE'
usage: verify-dependabot-topology.sh [owner/repo ...]

  Sans argument, interroge les trois dépôts autonomes de mika#1729 :
  senara-solutions/{mika,mika-cloud,mika-platform}.

exit 0 = tous les dépôts interrogés portent un dependabot.yml qui déclare
         au moins un écosystème
exit 1 = il en manque au moins un (absence ÉTABLIE, ou fichier inerte)
exit 2 = rien n'a pu être établi sur au moins un dépôt et aucune absence
         n'a été établie par ailleurs — ce n'est pas un succès
USAGE
}

if [[ ${1:-} == "-h" || ${1:-} == "--help" ]]; then
    usage
    exit "$EXIT_INDETERMINATE"
fi

if [[ $# -gt 0 ]]; then
    repos=("$@")
else
    repos=("${DEFAULT_REPOS[@]}")
fi

if ! command -v gh >/dev/null 2>&1; then
    echo "verify-dependabot-topology: \`gh\` introuvable sur le PATH." >&2
    echo "  Rien n'a été vérifié." >&2
    exit "$EXIT_INDETERMINATE"
fi

# Extraction à plat d'une clé scalaire YAML, sur toutes ses occurrences.
# Tolère le tiret de liste, les guillemets simples ou doubles, et l'indentation.
# Rend une ligne par valeur, dans l'ordre du fichier.
extract_key() {
    local key="$1"
    sed -n \
        -e "s/^[[:space:]]*-\{0,1\}[[:space:]]*${key}:[[:space:]]*\"\([^\"]*\)\".*/\1/p" \
        -e "s/^[[:space:]]*-\{0,1\}[[:space:]]*${key}:[[:space:]]*'\([^']*\)'.*/\1/p" \
        -e "s/^[[:space:]]*-\{0,1\}[[:space:]]*${key}:[[:space:]]*\([^\"'[:space:]#][^[:space:]#]*\).*/\1/p"
}

# Rend les valeurs distinctes, dans l'ordre de première apparition, sur une
# ligne séparée par des virgules. `sort -u` perdrait l'ordre du fichier, qui est
# l'ordre dans lequel un lecteur humain le relira.
join_unique() {
    awk 'NF && !seen[$0]++ { out = out (out ? ", " : "") $0 } END { print out }'
}

# Verdicts par dépôt, alignés sur `repos`. L'un de :
#   present | missing | inert | indeterminate
verdicts=()
# Jours lus par dépôt, pour l'observation d'étalement en fin de rapport.
days_seen=()

probe_repo() {
    local repo="$1"
    local body err rc detail

    err="$(mktemp)"
    # `Accept: application/vnd.github.raw` rend le fichier tel quel : pas de
    # base64 à décoder, donc pas de dépendance à `base64 -d` dont les drapeaux
    # divergent entre GNU et BSD.
    body="$(gh api "repos/${repo}/contents/${DEPENDABOT_PATH}" \
        -H "Accept: application/vnd.github.raw" 2>"$err")"
    rc=$?
    detail="$(tr -d '\r' <"$err" | head -3)"
    rm -f "$err"

    if [[ $rc -ne 0 ]]; then
        if [[ "$detail" == *"(HTTP 404)"* ]]; then
            # Voir « LE PIÈGE DU 404 » en tête : un 404 ne vaut absence que si
            # le dépôt lui-même est lisible par ce jeton.
            if gh api "repos/${repo}" --jq '.full_name' >/dev/null 2>&1; then
                verdicts+=("missing")
                days_seen+=("")
                printf '  %-34s ABSENT — le dépôt est lisible, %s n'\''y est pas.\n' \
                    "$repo" "$DEPENDABOT_PATH"
                return
            fi
            verdicts+=("indeterminate")
            days_seen+=("")
            printf '  %-34s INDÉTERMINÉ — 404 sur le fichier ET sur le dépôt.\n' "$repo"
            printf '  %-34s   GitHub répond 404 pour un dépôt privé hors de portée du jeton ;\n' ""
            printf '  %-34s   ce 404 ne dit donc rien du fichier. Vérifier la portée du jeton.\n' ""
            return
        fi
        verdicts+=("indeterminate")
        days_seen+=("")
        printf '  %-34s INDÉTERMINÉ — la lecture a échoué.\n' "$repo"
        [[ -n "$detail" ]] && printf '%s\n' "$detail" | sed "s/^/${DETAIL_INDENT}/"
        return
    fi

    local ecosystems days limits
    ecosystems="$(printf '%s\n' "$body" | extract_key "package-ecosystem" | join_unique)"
    days="$(printf '%s\n' "$body" | extract_key "day" | join_unique)"
    limits="$(printf '%s\n' "$body" | extract_key "open-pull-requests-limit" | join_unique)"

    if [[ -z "$ecosystems" ]]; then
        # Présent mais muet : Dependabot n'ouvrira jamais de PR, donc la
        # couverture a l'air acquise et ne l'est pas — la forme de panne que
        # mika#1997 existe pour fermer. Compté avec les manquants, nommé à part
        # parce que le remède diffère (éditer, pas créer).
        verdicts+=("inert")
        days_seen+=("")
        printf '  %-34s INERTE — fichier présent, aucun écosystème lu.\n' "$repo"
        return
    fi

    verdicts+=("present")
    days_seen+=("$days")
    printf '  %-34s PRÉSENT\n' "$repo"
    printf '  %-34s   écosystèmes : %s\n' "" "$ecosystems"
    printf '  %-34s   jours       : %s\n' "" "${days:-<non lu>}"
    printf '  %-34s   limite PR   : %s\n' "" "${limits:-<non lue>}"
}

echo "verify-dependabot-topology — mika#1997 (ferme R1 / AC1 de mika#1729)"
echo "lecture de ${DEPENDABOT_PATH} sur ${#repos[@]} dépôt(s) :"
echo
for repo in "${repos[@]}"; do
    probe_repo "$repo"
done
echo

# Observation d'étalement — NON GATANTE, et c'est dit.
#
# mika#1997 décision D étale les jours parce que la capacité de mika-qa est
# mesurée (~6 revues/heure : enveloppe 600 s, exécution sérialisée par
# `agent_lock`, cf. CLAUDE.md § mika#2347) et que trois dépôts sur le même jour
# donnent un pire cas de ≤ 25 PR le même matin. Le script met les jours côte à
# côte pour que la collision se voie ; il ne la transforme pas en échec, parce
# qu'un étalement est un arbitrage de volume et non une précondition.
declare -A day_owners=()
for i in "${!repos[@]}"; do
    [[ "${verdicts[$i]}" == "present" ]] || continue
    IFS=',' read -r -a parts <<<"${days_seen[$i]}"
    for part in "${parts[@]}"; do
        day="${part// /}"
        [[ -n "$day" ]] || continue
        day_owners["$day"]="${day_owners["$day"]:+${day_owners["$day"]}, }${repos[$i]}"
    done
done
collisions=0
for day in "${!day_owners[@]}"; do
    if [[ "${day_owners[$day]}" == *", "* ]]; then
        [[ $collisions -eq 0 ]] && echo "observation (non gatante) — jours partagés :"
        printf '  %-12s %s\n' "$day" "${day_owners[$day]}"
        collisions=1
    fi
done
[[ $collisions -eq 1 ]] && {
    echo "  Plusieurs dépôts planifient le même jour. Pire cas cumulé sur ce matin-là ;"
    echo "  voir docs/solutions/cross-repo-patterns/dependabot-topologie-trois-depots-2026-09-20.md"
    echo "  (décision D) pour l'arithmétique. Ce n'est pas un échec de vérification."
    echo
}

missing_count=0
indeterminate_count=0
for verdict in "${verdicts[@]}"; do
    case "$verdict" in
        missing | inert) missing_count=$((missing_count + 1)) ;;
        indeterminate) indeterminate_count=$((indeterminate_count + 1)) ;;
    esac
done

if [[ $missing_count -gt 0 ]]; then
    echo "VERDICT : il en manque ${missing_count} — R1 / AC1 de mika#1729 n'est pas fermé."
    if [[ $indeterminate_count -gt 0 ]]; then
        echo "  (${indeterminate_count} dépôt(s) par ailleurs indéterminé(s) ; l'absence établie suffit"
        echo "   à conclure que la couverture est partielle.)"
    fi
    echo "  Remède : poser ${DEPENDABOT_PATH} dans le dépôt concerné, par une PR de ce dépôt."
    echo "  Contenu et écosystèmes : docs/solutions/cross-repo-patterns/dependabot-topologie-trois-depots-2026-09-20.md"
    exit "$EXIT_MISSING"
fi

if [[ $indeterminate_count -gt 0 ]]; then
    echo "VERDICT : rien n'a pu être établi sur ${indeterminate_count} dépôt(s)."
    echo "  Ce n'est NI un succès NI une absence : la question reste ouverte."
    echo "  Vérifier que \`gh\` est authentifié et que le jeton porte sur ces dépôts."
    exit "$EXIT_INDETERMINATE"
fi

echo "VERDICT : les ${#repos[@]} dépôt(s) portent un ${DEPENDABOT_PATH} actif."
echo "  C'est la précondition de mika#1729, pas la preuve que la revue se déclenche :"
echo "  celle-là s'observe sur une vraie PR Dependabot (mika#1997, phase 3)."
exit "$EXIT_ALL_PRESENT"
