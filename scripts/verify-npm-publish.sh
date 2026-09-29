#!/usr/bin/env bash
#
# mika#2566 — vérification post-publish d'un paquet npm : bornée, testable,
# auto-diagnostique.
#
#   usage: verify-npm-publish.sh <package> <version>
#
# Générique à dessein : aucun nom de paquet n'est écrit dans ce fichier. Le
# workflow appelant passe le sien.
#
# POURQUOI CE SCRIPT EXISTE, ET PAS UNE BOUCLE DANS UN `run:` DE YAML.
#
# La boucle 5×15 s qu'il remplace ÉTAIT le correctif de mika#1917 (`d32a0fd7`,
# « publish-ui verify step retry+backoff for CDN propagation lag »). Elle a été
# posée sur une borne supposée — « npmjs CDN can take up to ~60s » — jamais
# mesurée, et elle a tenu jusqu'au bump suivant : run 36410711981 du 2026-09-28,
# faux rouge sur `main` pendant que `@samidarko/ui@0.4.0` était bel et bien
# publié. mika#2566 est la DEUXIÈME occurrence de la même classe, et le remède
# que le ticket proposait — rallonger la fenêtre — est celui qui a déjà échoué.
#
# Ce qui change n'est donc pas le nombre, c'est que la logique soit exerçable :
# `scripts/test-verify-npm-publish.sh` la fait tourner en moins d'une seconde,
# sans réseau, et le job CI `publish-verify-lint` l'arme. Le correctif suivant
# sera vérifié avant de partir, pas au bump d'après.
#
# DEUX HYPOTHÈSES DE CAUSE, ET ON N'EN CHOISIT AUCUNE.
#
#   H1  propagation registre / CDN — le packument mis à jour met plus de temps
#       que la fenêtre à être servi. Remède : la fenêtre.
#   H2  cache npm LOCAL du runner — l'étape « Check if version changed » appelle
#       `npm view <pkg> version` AVANT la publication, ce qui met en cache le
#       packument d'avant ; la vérification relit le même document et npm sert
#       une entrée encore fraîche au lieu de revalider. Remède : la revalidation.
#
# Laquelle produit le délai mesuré n'est pas établissable hors d'un run réel. On
# ferme donc les deux — fenêtre de 345 s ET `--prefer-online` — et on instrumente
# (bloc de diagnostic ci-dessous) pour que la PROCHAINE occurrence tranche. Sans
# quoi, si H2 est vraie, une fenêtre longue la couvre par accident et le
# correctif se relit comme une victoire de l'allongement.
#
# Ce que ce script n'achète pas : il ne fait pas publier ni propager plus vite,
# et il ne garantit pas que 345 s suffiront. Il garantit que le jour où ça ne
# suffira pas, on saura pourquoi au lieu de reposer un nombre.

set -euo pipefail

# La géométrie livrée. `345 s` de couverture = 15+30+60+60+60+60+60, sommeils
# ENTRE essais uniquement — l'ancienne boucle dormait aussi après le dernier
# essai, donc ses « 75 s » annoncés n'étaient que 60 s de couverture réelle.
DEFAULT_MAX_ATTEMPTS=8
DEFAULT_SLEEP_BASE_SECS=15
DEFAULT_SLEEP_CAP_SECS=60

REGISTRY_BASE_URL="https://registry.npmjs.org"

usage() {
    printf 'usage: %s <package> <version>\n' "$(basename "$0")" >&2
    printf '  ex.: %s @scope/pkg 1.2.3\n' "$(basename "$0")" >&2
}

warn() { printf '::warning::%s\n' "$*"; }

# ---------------------------------------------------------------------------
# Résolution de la géométrie (D6).
#
# Ces trois clés ne sont PAS des réglages d'opérateur — rien ne les pose sur un
# runner GitHub. Elles existent pour que le harnais rende le test déterministe
# et instantané au lieu de dormir six minutes.
#
#   absent / vide        → défaut, en silence
#   `0`                  → selon le mode : honoré, ou refusé avec avertissement
#   négatif / illisible  → défaut + avertissement nommant la valeur ENTRE
#                          GUILLEMETS (un espace parasite doit se voir)
#
# `MAX_ATTEMPTS=0` est refusé parce qu'il désarmerait la vérification, et un
# désarmement par coquille sur une garde est très exactement la panne silencieuse
# que ce ticket ferme. Les deux durées, elles, honorent `0` : le harnais en a
# besoin.
# ---------------------------------------------------------------------------
RESOLVED=0
resolve_int() {
    local name="$1" raw="$2" default="$3" zero_mode="$4"
    RESOLVED="$default"

    if [ -z "$raw" ]; then
        return 0
    fi

    # Tout ce qui n'est pas une suite de chiffres — signe négatif compris, le
    # `-` étant un non-chiffre — est illisible. La borne de longueur écarte
    # aussi les valeurs absurdes avant toute arithmétique.
    case "$raw" in
        *[!0-9]*)
            warn "$name: valeur illisible \"$raw\" — défaut $default appliqué."
            return 0
            ;;
    esac
    if [ "${#raw}" -gt 6 ]; then
        warn "$name: valeur hors domaine \"$raw\" — défaut $default appliqué."
        return 0
    fi

    # `10#` force la base 10 : sans lui, `08` serait une erreur arithmétique et
    # `010` vaudrait 8.
    local n=$((10#$raw))
    if [ "$n" -eq 0 ] && [ "$zero_mode" = refused ]; then
        warn "$name: \"$raw\" désarmerait la vérification — défaut $default appliqué."
        return 0
    fi

    RESOLVED="$n"
}

# Durée du sommeil qui SUIT l'essai $1 (1-based). Doublement depuis la base,
# plafonné.
sleep_after_attempt() {
    local n="$1" s="$SLEEP_BASE" i=1
    while [ "$i" -lt "$n" ]; do
        if [ "$s" -ge "$SLEEP_CAP" ]; then
            break
        fi
        s=$((s * 2))
        i=$((i + 1))
    done
    if [ "$s" -gt "$SLEEP_CAP" ]; then
        s="$SLEEP_CAP"
    fi
    printf '%s' "$s"
}

# Couverture totale = somme des sommeils, donc l'instant du DERNIER essai.
total_window() {
    local sum=0 n=1
    while [ "$n" -lt "$MAX_ATTEMPTS" ]; do
        sum=$((sum + $(sleep_after_attempt "$n")))
        n=$((n + 1))
    done
    printf '%s' "$sum"
}

# ---------------------------------------------------------------------------
# Bloc de diagnostic (D4) — branche d'échec SEULEMENT, donc coût nul en régime
# nominal. Il existe pour séparer H1 de H2 à la prochaine occurrence, ce que ni
# mika#1917 ni le présent ticket ne pouvaient faire.
# ---------------------------------------------------------------------------
emit_diagnostic() {
    local pkg="$1" version="$2" last_err="$3" workdir="$4"

    printf '\n--- diagnostic post-échec (mika#2566) ---\n'

    # [1] La cause que `2>/dev/null || echo ""` effaçait. Un E404 (la version
    # n'est pas encore là — attendu pendant la propagation), un ENETWORK (le
    # runner n'a pas de réseau) et un E401 (jeton invalide) rendaient des bytes
    # strictement identiques : la chaîne vide. Le message d'échec disait « not
    # visible on the default registry » dans les trois cas, et il n'était vrai
    # que dans le premier.
    printf '\n[1] stderr de la dernière tentative :\n'
    if [ -s "$last_err" ]; then
        sed 's/^/    /' "$last_err"
    else
        printf "    (vide — npm n'a rien écrit sur stderr)\n"
    fi

    # [2] Un dernier appel client, frais. `--json` pour voir la structure que
    # npm rend — y compris son objet d'erreur — et non la seule chaîne de
    # version. Cette forme est aussi ce qui distingue l'appel de diagnostic des
    # appels de boucle dans l'argv, ce dont le harnais a besoin.
    printf '\n[2] npm view --prefer-online final :\n'
    local npm_sees="non"
    if npm view "$pkg@$version" version --prefer-online --json \
        >"$workdir/final.out" 2>"$workdir/final.err"; then
        :
    fi
    if [ -s "$workdir/final.out" ]; then
        sed 's/^/    /' "$workdir/final.out"
    fi
    if [ -s "$workdir/final.err" ]; then
        sed 's/^/    /' "$workdir/final.err"
    fi
    # Égalité STRICTE sur la valeur rendue, jamais une recherche de sous-chaîne :
    # sous `--json`, npm écrit son objet d'erreur sur stdout EN Y CITANT LE SPEC
    # (« 404 '<pkg>@<version>' is not in this registry »). Un `grep` y trouverait
    # la version et répondrait « npm la voit » sur l'échec même qu'on diagnostique
    # — ce qui inverserait la table de lecture ci-dessous, donc enverrait
    # l'opérateur vers le mauvais remède.
    local final_line=""
    if [ -s "$workdir/final.out" ]; then
        read -r final_line <"$workdir/final.out" || true
    fi
    if [ "${final_line//\"/}" = "$version" ]; then
        npm_sees="oui"
    fi
    printf '    → le client npm voit la version : %s\n' "$npm_sees"

    # [3] Le registre, sans passer par le client npm ni par son cache. Le scope
    # est encodé (`/` → `%2F`) parce que le packument d'un paquet scopé vit à
    # `/@scope%2Fname`.
    local url_pkg="${pkg//\//%2F}"
    printf '\n[3] registre HTTP direct (%s/%s) :\n' "$REGISTRY_BASE_URL" "$url_pkg"
    local registry_state="injoignable"
    if curl -fsS --max-time 20 "$REGISTRY_BASE_URL/$url_pkg" \
        >"$workdir/packument" 2>"$workdir/curl.err"; then
        local parsed=""
        if parsed="$(node -e 'const fs=require("fs");let doc;try{doc=JSON.parse(fs.readFileSync(0,"utf8"));}catch(e){console.log("illisible");process.exit(0);}const v=(doc&&doc.versions)?doc.versions:{};console.log(Object.prototype.hasOwnProperty.call(v,process.argv[1])?"presente":"absente");' "$version" <"$workdir/packument" 2>"$workdir/node.err")"; then
            registry_state="$parsed"
        else
            registry_state="non parsé (node indisponible ou en erreur)"
            if [ -s "$workdir/node.err" ]; then
                sed 's/^/    /' "$workdir/node.err"
            fi
        fi
    else
        if [ -s "$workdir/curl.err" ]; then
            sed 's/^/    /' "$workdir/curl.err"
        fi
    fi
    printf '    → la version est %s dans le packument servi\n' "$registry_state"

    # La table de lecture, imprimée ici pour que celui qui lit le log rouge
    # n'ait pas à retrouver le plan. La halte est écrite en premier parce que le
    # geste par réflexe — rallonger — est faux dans le cas H2.
    cat <<'TABLE'

    Table de lecture (mika#2566) — LIRE AVANT DE TOUCHER À LA FENÊTRE :

      registre HTTP direct | npm view | lecture                        | remède
      ---------------------+----------+--------------------------------+-------------------------------
      presente             | non      | H2 — client / cache npm        | côté client, JAMAIS la fenêtre
      absente              | non      | H1 — propagation réelle        | la fenêtre, AVEC cette mesure
      injoignable          | —        | ni H1 ni H2 : pas de réseau    | la boucle a tourné pour rien

      Si c'est H1, c'est la TROISIÈME occurrence de la classe : ouvrir un ticket
      de suivi avec cette mesure, jamais rallonger au jugé.
TABLE
}

# ---------------------------------------------------------------------------

main() {
    if [ "$#" -ne 2 ]; then
        usage
        exit 2
    fi
    local pkg="$1" version="$2"
    if [ -z "$pkg" ] || [ -z "$version" ]; then
        usage
        exit 2
    fi

    resolve_int PUBLISH_VERIFY_MAX_ATTEMPTS "${PUBLISH_VERIFY_MAX_ATTEMPTS:-}" \
        "$DEFAULT_MAX_ATTEMPTS" refused
    MAX_ATTEMPTS="$RESOLVED"
    resolve_int PUBLISH_VERIFY_SLEEP_BASE_SECS "${PUBLISH_VERIFY_SLEEP_BASE_SECS:-}" \
        "$DEFAULT_SLEEP_BASE_SECS" honoured
    SLEEP_BASE="$RESOLVED"
    resolve_int PUBLISH_VERIFY_SLEEP_CAP_SECS "${PUBLISH_VERIFY_SLEEP_CAP_SECS:-}" \
        "$DEFAULT_SLEEP_CAP_SECS" honoured
    SLEEP_CAP="$RESOLVED"

    # D7 — la géométrie résolue est dite à chaque exécution. Doctrine mika#2293 :
    # un réglage qu'on ne peut pas observer n'est pas un réglage, c'est un espoir.
    # Cette ligne est aussi le contrôle positif de la sonde S1 : son absence dans
    # le log d'un run dit que le workflow servi n'appelle pas ce script, jamais
    # que tout va bien.
    printf 'verify geometry: attempts=%s base=%ss cap=%ss window=%ss prefer-online=yes\n' \
        "$MAX_ATTEMPTS" "$SLEEP_BASE" "$SLEEP_CAP" "$(total_window)"

    local workdir
    workdir="$(mktemp -d)"
    # shellcheck disable=SC2064
    trap "rm -rf '$workdir'" EXIT

    local out="$workdir/out" err="$workdir/err"
    local attempt=1 published="" naptime=""

    while [ "$attempt" -le "$MAX_ATTEMPTS" ]; do
        : >"$out"
        : >"$err"
        # `--prefer-online` (D3) ferme H2 à sa cause : il force la revalidation
        # du packument que l'étape pré-publication vient de mettre en cache.
        # Appliqué à la vérification SEULE — l'autre appel tourne avant la
        # publication, son résultat est frais par construction.
        if npm view "$pkg@$version" version --prefer-online >"$out" 2>"$err"; then
            :
        fi
        published=""
        if [ -s "$out" ]; then
            read -r published <"$out" || true
        fi
        if [ "$published" = "$version" ]; then
            printf 'Verified %s@%s is live on the default registry (attempt %s/%s).\n' \
                "$pkg" "$published" "$attempt" "$MAX_ATTEMPTS"
            exit 0
        fi

        if [ "$attempt" -lt "$MAX_ATTEMPTS" ]; then
            naptime="$(sleep_after_attempt "$attempt")"
            printf 'Attempt %s/%s: %s@%s not yet visible. Sleeping %ss...\n' \
                "$attempt" "$MAX_ATTEMPTS" "$pkg" "$version" "$naptime"
            sleep "$naptime"
        else
            printf 'Attempt %s/%s: %s@%s not yet visible. Window exhausted.\n' \
                "$attempt" "$MAX_ATTEMPTS" "$pkg" "$version"
        fi
        attempt=$((attempt + 1))
    done

    # D5 — une erreur non-E404 n'a PAS raccourci la boucle. Sortir tôt sur une
    # erreur réseau transformerait un incident transitoire en faux rouge d'une
    # autre espèce : le défaut de ce ticket, sous un autre nom. La boucle va au
    # bout ; c'est le message final qui nomme la cause.
    emit_diagnostic "$pkg" "$version" "$err" "$workdir"

    local cause=""
    cause="$(grep -m1 -v '^[[:space:]]*$' "$err" 2>/dev/null || true)"
    cause="${cause//$'\r'/}"
    cause="${cause:0:200}"
    if [ -z "$cause" ]; then
        cause="npm n'a écrit aucune cause sur stderr (sortie vide)"
    fi

    printf '::error::Post-publish verification failed after %s attempts over %ss: %s@%s not visible. Dernière cause npm: %s\n' \
        "$MAX_ATTEMPTS" "$(total_window)" "$pkg" "$version" "$cause"
    exit 1
}

main "$@"
