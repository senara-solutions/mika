# mika#2408 — Un sink sortant neuf ne peut plus apparaître sans être déclaré

> Ticket : senara-solutions/mika#2408 — *Egress gate: fail CI when a new outbound
> sink appears without a data-flow declaration*
> Labels : `enhancement`, `agent-core`, `infrastructure`, `dispatch:loop`
> **decision-core** — relecture @samidarko + sign-off Vincent avant dispatch.

---

## Ce que la lecture du code déplace dans le ticket

C'est le premier livrable du grooming : quatre mesures faites sur l'arbre
(HEAD `13518699`) corrigent la piste du ticket, et chacune change le mécanisme.

### R1 — `orchestrator_inbox.rs:524` n'est PAS un sink de production

Le ticket le cite comme preuve vivante d'un sink non couvert. Mesure :

```
crates/mika-gateway/src/orchestrator_inbox.rs:495:#[cfg(test)]
crates/mika-gateway/src/orchestrator_inbox.rs:524:        let http_client = reqwest::Client::new();
```

La ligne 524 est **après** le `#[cfg(test)]` de la ligne 495 : c'est du code de
test. La thèse du ticket (« la couverture est par-host, pas par-sink ») reste
entièrement vraie — `telegram.rs` et la famille LLM la démontrent — mais cette
pièce-là ne la démontre pas. Elle démontre autre chose, et de plus utile : **un
détecteur qui grep sans découper le code de test rapporte des sinks qui
n'existent pas.** C'est la contrainte d'AC4 (« vert sur main = 0 faux positif »)
rencontrée avant d'avoir écrit une ligne.

### R2 — le prédicat « construction de client » seul rend `telegram.rs` FANTÔME

C'est la mesure qui décide de la forme du manifeste. Le ticket propose un champ
unique `call_site`, vérifié contre un inventaire de constructions de client.
Appliqué au sink le plus évident du ticket :

```
crates/mika-gateway/src/telegram.rs:1741:  let client = TelegramClient::new(Client::new(), …)   # cfg(test) mod @1308
crates/mika-gateway/src/telegram.rs:2256:  CustomerTelegramClient::new(Client::new(), …)        # idem
```

**`telegram.rs` ne construit aucun client HTTP en production.** Son client vient
de `crates/mika-gateway/src/main.rs:108`, un `reqwest::Client::builder()` partagé
injecté dans Telegram, GitHub-gateway et orchestrator-inbox. Une entrée
`call_site = "crates/mika-gateway/src/telegram.rs"` vérifiée contre l'inventaire
des constructeurs serait donc rejetée comme **déclaration fantôme** — AC1b
tirerait sur la déclaration la plus juste du manifeste.

Conséquence : le manifeste a besoin de **deux** champs de localisation, pas d'un.
`client_site` (où le client naît — le champ tenu en lockstep mécanique) et
`call_site` (où la requête part — le champ que l'humain lit). La relation est
N:M : un client partagé alimente trois destinations, et c'est en soi une
information.

### R3 — un prédicat « call-site de requête » est irrécupérable en grep

Trois formulations mesurées, aucune utilisable seule :

| prédicat | population `crates/*/src/` | défaut |
|---|---|---|
| receveur `client`/`http` + verbe, même ligne | **6 fichiers** | rate `github_graphql.rs`, `claude.rs`, `openai.rs`, `oauth.rs` — le chaînage multi-ligne |
| verbe HTTP en tête de ligne | **72 fichiers** | majorité de `.get(` sur `HashMap` (`kg/query.rs`, `skills/matcher.rs`, `well_known_agents.rs`) |
| construction de client (`Client::new()` / `::builder()` / `ClientBuilder::new`) | **34 fichiers** | ne dit pas *où* ça va, et rate un sink dont le client est injecté (R2) |

Le troisième est le seul complet sur sa classe et à bruit ~nul. Il est donc le
prédicat **porteur**, et `call_site` reste documentaire. La moitié « vers où ? »
est fermée par un second prédicat, sur les destinations (R4).

### R4 — le manifeste doit être la source unique des hosts, sinon AC1 a un trou

Un prédicat sur les seuls `client_site` ferme « un module egress **neuf**
apparaît » et laisse ouvert « un host **neuf** apparaît dans un module déjà
déclaré » — ajouter `client.post("https://evil.example/")` dans
`github_graphql.rs` ne créerait aucun fichier neuf.

La fermeture propre est un second prédicat sur les **littéraux de host**, dont le
manifeste est l'allowlist. Il est aussi ce qui ferme AC5 : `PATTERNS` de
`verify-egress-uniqueness.sh` se dérive alors du manifeste au lieu de vivre à
côté. Le bruit est mesuré et bornable :

```
304  "https://github.com        ← identifiants de ressource (reference_url, pr_url), pas des sinks
 59  "https://example.com        ← fixtures
 56  "http://localhost
 22  "http://127.0.0.1
 18  "https://api.github.com     ← sink réel
 …   ~70 hosts distincts au total
```

Le back-fill consiste à classer ces ~70 hosts en trois seaux : **déclarés**
(sinks), **non-sinks** (loopback, `example.com`, `*.invalid`, `*.test`,
`*.svc.cluster.local`), et **identifiants de ressource** (`github.com` —
déclaré de toute façon, puisqu'un `git push` l'atteint).

---

## Objectif

Rendre **structurellement impossible** l'ajout d'un chemin réseau sortant sans
qu'une ligne humainement relisible dise *quelle donnée part, vers où, et si elle
est journalisée* — et rendre impossible qu'une telle ligne soit écrite sans que
le sink existe vraiment.

Le mécanisme est un **lockstep bidirectionnel manifeste↔code**, à quatre
directions, décrites ci-dessous. Ce n'est pas un champ libre : chaque moitié de
la déclaration est confrontée mécaniquement à l'arbre.

---

## Mécanisme

### M1 — Le manifeste : `docs/egress/egress-manifest.toml`

Une entrée par sink. Le schéma étend celui du ticket avec les trois champs que
R2 et R4 rendent nécessaires (`client_site`, `class`, `destination_source`).

```toml
# docs/egress/egress-manifest.toml
#
# Chaque [[sink]] déclare un chemin réseau sortant. Le manifeste est tenu en
# LOCKSTEP avec le code par scripts/verify-egress-manifest.sh : il ne peut ni
# omettre un sink réel, ni nommer un sink qui n'existe pas.
#
# `docs/egress/` est sous CODEOWNERS @samidarko : aucune entrée ne merge sans
# revue humaine.

schema_version = 1

[[sink]]
id                 = "brave-search"
destination        = "<le host API Brave — cf. ALLOWED en egress_search/>"
destination_source = "literal"          # literal | config | skill-declared
class              = "external"         # external | internal
data               = "texte de la requête de recherche de l'utilisateur"
logged             = false              # corps / query journalisés ?
client_site        = "crates/mika-gateway/src/egress_search/mod.rs"
call_site          = "crates/mika-gateway/src/egress_search/"
confined           = true               # alimente PATTERNS de verify-egress-uniqueness.sh (AC5)
owner              = "@samidarko"
notes              = "Substrat E1 (mika#1807). STRIP-TOTAL, no-log tenu par verify-egress-no-log.sh."

[[sink]]
id                 = "telegram-bot-api"
destination        = "api.telegram.org"
destination_source = "literal"
class              = "external"
data               = "texte des messages sortants, chat_id, fichiers entrants (getFile)"
logged             = false
client_site        = "crates/mika-gateway/src/main.rs"   # client partagé — voir R2
call_site          = "crates/mika-gateway/src/telegram.rs"
confined           = false
owner              = "@samidarko"
```

**Les trois champs ajoutés au schéma du ticket, et pourquoi.**

- **`client_site`** — la moitié *vérifiable* de la localisation. Sans elle,
  `telegram.rs` est une déclaration fantôme (R2). `call_site` reste le champ que
  le reviewer lit ; il est documentaire, jamais vérifié mécaniquement, et le
  plan le dit plutôt que de le laisser croire.
- **`class`** — `internal` (loopback, ClusterIP, démon local) vs `external`.
  AC4 autorise « exclus **ou** déclarés » pour les appels internes ; ce plan
  **déclare**, parce qu'exclure par heuristique rouvre exactement le trou du
  ticket : un sink interne qui devient externe ne changerait aucune ligne. Ici,
  il change `class = "internal"` en `"external"` — un diff d'un mot, sous
  CODEOWNERS. **Coût nommé** : ajouter un module CLI qui parle au démon local
  coûte désormais une revue @samidarko.
- **`destination_source`** — `literal` (le host est en dur), `config` (une
  `base_url` repointable : les 13 providers LLM, `ollama`, les endpoints OTLP),
  `skill-declared` (`skills/executor.rs::execute_http`, dont l'URL vient du
  manifeste d'une skill et n'est **pas** connaissable statiquement). Ce champ
  n'est pas une commodité de lint : c'est l'information de sécurité la plus
  dense du fichier. *« api.openai.com » ne décrit le sink que tant que
  `MIKA_OPENAI_BASE_URL` n'est pas posé*, et un manifeste qui tairait ça
  affirmerait une destination que l'exécution peut démentir.

### M2 — Le lint : `scripts/verify-egress-manifest.sh`

Bash pour l'orchestration, `python3 -B` pour le parsing TOML (`tomllib`, stdlib
≥ 3.11 ; `python3 -B` est déjà employé par deux jobs CI). Parser le TOML à la
main en bash est précisément la fragilité que `check-pilot-turn-ceiling-labels.sh`
a dû border par un exit 3 « forme non auditable » ; on ne la réintroduit pas.

Le script prend un **argument de chemin optionnel** (racine à scanner), sur le
modèle de `verify-egress-no-log.sh` — c'est ce qui rend le test négatif possible
sans toucher l'arbre vivant.

**Quatre directions, chacune avec son message d'erreur propre.**

| # | direction | prédicat | ferme |
|---|---|---|---|
| **D1** | code → manifeste | tout fichier construisant un client HTTP en **production** est couvert par ≥1 `client_site` | AC1 — module egress neuf |
| **D2** | code → manifeste | tout littéral de host **externe** en production est couvert par ≥1 `destination`, ou appartient à `NON_SINK_HOSTS` | AC1 — host neuf dans un module existant |
| **D3** | manifeste → code | tout `client_site` couvre ≥1 fichier de l'inventaire D1 | AC1b — déclaration fantôme |
| **D4** | manifeste → code | toute `destination` dont `destination_source = "literal"` apparaît ≥1 fois comme littéral | AC1b — host fantôme |

D4 ne s'applique **pas** aux `config` / `skill-declared` : leur destination n'est
par construction dans aucun littéral, et exiger le contraire ferait rougir le
lint sur les déclarations les plus honnêtes du fichier.

**Découpe production/test — la décision qui tient tout, et elle NE réutilise pas
le parseur voisin.** Le réflexe est d'importer `production_lines()` de
`verify-egress-no-log.sh` : c'est le meilleur parseur `#[cfg(test)]` du dépôt.
**Deux mesures le refusent.**

*(i) Ce parseur refuse deux formes présentes dans la population visée* —
fail-closed exit 3 sur un `#[cfg(test)]` qu'il ne modélise pas :

```
crates/mika-common/src/llm/ollama.rs:30:#[cfg(test)]
crates/mika-common/src/llm/ollama.rs:31:pub(crate) fn reset_payload_dump_flag() {   ← bare fn, non modélisé
crates/mika-gateway/src/routes.rs:2301:#[cfg(test)]
crates/mika-gateway/src/routes.rs:2302:pub(crate) const OUTBOUND_MESSAGE_METADATA_FIELDS …  ← bare const
```

*(ii) L'extraire vers un fichier partagé FAIT ROUGIR son propre test.*
`test-verify-egress-no-log.sh` (V13) cherche la définition **dans ce fichier-là**,
par ancrage de début de ligne :

```bash
pl_calls=$(sed -n '/^production_lines() {/,/^}/p' "$LINT" | grep -c 'sanitize(\$0)')
assert_true "V13 each parser calls sanitize() in its own body" …
```

Une extraction rend ce `sed` vide, `pl_calls=0`, assertion rouge. Et la
dupliquer est très exactement la divergence que le commentaire de
`AWK_SANITIZE_FN` interdit en toutes lettres — avec cette aggravation que V13
est scopé à **un** fichier, donc une copie dans un autre script **ne le ferait
pas rougir** : la divergence serait silencieuse, ce qui est le pire des trois cas.

**Donc : le nouveau lint porte son propre parseur, indépendant, et il a le droit
d'être plus grossier.** Ce n'est pas un pis-aller, c'est la conséquence de son
unité d'analyse : il répond *« ce **fichier** a-t-il un sink en production ? »*,
pas *« quelles **lignes** sont production ? »*. Sa règle de doute est donc
inverse de celle du voisin : **tout doute conclut « production », donc
« déclare »**. Une erreur de découpe ne peut produire qu'une **déclaration de
plus** — jamais un silence. C'est ce qui autorise le parseur simple :

- le premier `#[cfg(test)]` **suivi d'un `mod X {`** (bloc, pas `;`) coupe le
  fichier ; tout ce qui précède est production ;
- toute autre forme de `#[cfg(test)]` — `fn`, `const`, `impl` — **ne coupe pas**
  et le fichier est traité en entier comme production.

Vérifié sur les deux cas mesurés : `orchestrator_inbox.rs:495-496` porte bien
`#[cfg(test)]` + `mod tests {` et sort de l'inventaire D1 (R1 confirmé, ce n'est
pas un sink de production) ; `ollama.rs` et `routes.rs` y restent en entier et
paient une ligne de déclaration.

**Bénéfice qui décide** : le nouveau lint ne partage **aucune ligne** avec la
garde STRIP-TOTAL. Il ne peut donc pas la casser, et `verify-egress-no-log.sh`
comme son test restent intouchés — condition inscrite en DoD.

**Ce que le lint ne scanne pas** (AC4) : `crates/*/tests/`, `*/examples/`,
`*/benches/`, `target/`, et toute ligne de commentaire (`^\s*//`). Mesure de
l'effet : le filtre commentaire seul ne retire que 3 des 304 `github.com` —
l'exclusion qui porte est celle de `tests/` et de `#[cfg(test)]`.

**`NON_SINK_HOSTS`** — allowlist par **classe**, jamais par host individuel :
`localhost`, `127.0.0.0/8`, `0.0.0.0`, `::1`, `*.svc.cluster.local`,
`example.com` / `example.org` / `example.net` (RFC 2606), `*.invalid`, `*.test`,
`*.local`. Chaque classe porte sa raison en commentaire. Un host qui n'entre
dans aucune classe et dans aucune `destination` **doit être déclaré** — c'est le
contrat, pas une friction à contourner.

### M3 — Le test négatif : `scripts/test-verify-egress-manifest.sh`

Modèle exact de `test-verify-egress-no-log.sh` : synthétiser des fixtures à
partir de l'arbre vivant (jamais depuis l'historique git — leçon mika#2039 :
une anti-vacuité qui lit l'état cassé dans l'historique s'inverse le jour où la
branche merge), muter en un seul point, lancer le lint contre la copie.

Cas minimaux, un par direction plus les contrôles :

| cas | mutation | attendu |
|---|---|---|
| N0 | arbre réel, manifeste réel | exit **0** — le contrôle positif |
| N1 | fichier neuf avec `reqwest::Client::new()`, aucune entrée | exit 1, message « undeclared client site » |
| N2 | littéral `"https://evil.example.org/api"` dans un fichier **déjà déclaré** | exit 1, message « undeclared destination » |
| N3 | entrée `[[sink]]` avec `client_site` pointant un chemin sans constructeur | exit 1, message « phantom client_site » |
| N4 | entrée `literal` dont la `destination` n'apparaît nulle part | exit 1, message « phantom destination » |
| N5 | la même mutation que N1, **en `#[cfg(test)]`** | exit **0** — contrôle négatif d'AC4 |
| N6 | la même mutation que N1, sous `crates/x/tests/` | exit **0** — contrôle négatif d'AC4 |
| N7 | manifeste au TOML invalide | exit ≠ 0, message nommant la ligne — jamais un vert |
| N8 | manifeste vide (zéro `[[sink]]`) | exit ≠ 0 — anti-vacuité : un lint qui compare à une liste vide et se tait est un lint inerte |
| N9 | `destination_source = "config"` sans littéral correspondant | exit **0** — D4 ne s'y applique pas |
| N10 | scan de source : le nouveau lint ne source ni ne copie `production_lines()` / `sanitize()` de `verify-egress-no-log.sh` | exit 0, allowlist vide |

**N0, N5, N6 et N9 sont les cas porteurs**, pas de la décoration : sans eux,
« le lint détecte » est indiscernable de « le lint rougit sur tout ».
**N8 est l'anti-vacuité** : elle interdit qu'un futur refactor rende le lint
silencieusement inerte, ce qui se lit exactement comme un arbre propre
(classe mika#2205). **N10 tient la frontière avec la garde STRIP-TOTAL** : la
séparation des parseurs est une décision, pas une intention, et V13 ne peut pas
la voir depuis son côté (M2 (ii)).

Chaque cas doit être **vu rouge avant d'être vu vert** pendant l'implémentation ;
un cas qui n'a jamais échoué n'atteste rien.

### M4 — AC5 : une seule liste de hosts

`verify-egress-uniqueness.sh` garde son rôle (confinement : *ce host n'apparaît
que dans son substrat*) et cesse de porter sa propre liste. Ses `PATTERNS` sont
dérivés du manifeste : les `destination` des entrées portant `confined = true`.
Un sink peut confiner **plusieurs** hosts (le substrat `egress_fetch` en compte
quatre pour une entrée), d'où un champ `confined_hosts = [...]` optionnel qui,
s'il est présent, remplace `destination` dans la dérivation.

`AUTHORIZED_PATHS` reste sa propriété : c'est une liste de **chemins autorisés à
mentionner**, une question que le manifeste ne pose pas.

### M4b — Le manifeste entre en conflit structurel avec ce lint, et l'ordre est contraint

Mesure faite pendant le grooming, sur ce plan lui-même :

```
$ bash scripts/verify-egress-uniqueness.sh
ERROR (egress-uniqueness): search-upstream identifier '<host Brave>' at docs/plans/…-2408-…-plan.md
… 5 violations
```

Ce lint grep **tout `docs/`** pour ses 5 patterns. Le manifeste, par
construction, doit nommer ces hosts : **son premier commit fait rougir
`egress-uniqueness-lint`.** Ce n'est pas un effet de bord du plan, c'est une
propriété du livrable.

Conséquence, à exécuter dans l'étape 1 et pas plus tard :

```diff
     "docs/solutions/best-practices/mirror-substrate-module-for-new-egress-class-2026-08-23.md"
+    # Le manifeste d'egress (mika#2408) nomme chaque destination par
+    # définition — c'est son objet. Il ne réalise aucun appel : la
+    # déclaration est le contraire d'un chemin de reachability.
+    "docs/egress/"
     "scripts/verify-egress-uniqueness.sh"
```

**Et la dérivation d'AC5 doit venir APRÈS.** Sinon `verify-egress-uniqueness.sh`
lit ses patterns depuis un manifeste encore incomplet, et une garde de
confinement qui perd ses patterns se lit exactement comme une garde qui passe.
Ordre : (i) `docs/egress/` dans `AUTHORIZED_PATHS` → (ii) back-fill vert →
(iii) dérivation des `PATTERNS`.

**Ce plan lui-même n'épelle aucun des 5 hosts confinés** — il les désigne. Les
plans #1807 et #1969 ont pris l'autre voie (une entrée `AUTHORIZED_PATHS` par
plan) ; la désignation est préférée ici parce qu'un plan de grooming n'a aucun
besoin d'épeler un host que son substrat possède, et qu'une entrée d'allowlist
par document est du bruit permanent sur une garde de sûreté.

**Repli nommé** : si la dérivation s'avère plus coûteuse que prévu (parsing
TOML depuis un script qui n'en fait pas aujourd'hui), AC5 autorise
explicitement un **ticket de convergence lié** — dans ce cas, poser le ticket,
l'écrire dans l'en-tête de `verify-egress-uniqueness.sh`, et ne pas laisser la
duplication muette. L'étape (i) reste obligatoire dans tous les cas.

### M5 — AC6 : CODEOWNERS

```diff
 /docs/gate/                                         @samidarko
+# Surface de flux sortant (mika#2408) — un nouveau chemin réseau ne merge pas
+# sans revue humaine. Même classe que la forge-gate perimeter ci-dessus.
+/docs/egress/                                       @samidarko
```

### M6 — Câblage CI + Makefile

```yaml
  egress-manifest-lint:
    name: Egress Manifest Lint
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1  # v6
      - name: Enforce manifest↔code lockstep on outbound sinks (mika#2408)
        run: bash scripts/verify-egress-manifest.sh
      - name: Pin the guard's negative behaviour (AC1, AC1b)
        run: bash scripts/test-verify-egress-manifest.sh
```

Placé après `egress-no-log-lint`, même forme que ses trois frères (dont deux
portent déjà leur test négatif en second step). Cible `make verify-egress-manifest`
sur le modèle de `verify-egress-no-log`, et ajout au `.PHONY`.

---

## Découpage de l'implémentation

Six étapes, dans cet ordre — l'ordre est un livrable (M4b). Chacune est
vérifiable seule.

0. **`docs/egress/` dans `AUTHORIZED_PATHS`** de `verify-egress-uniqueness.sh`
   (M4b). Sans ça, le premier commit du manifeste fait rougir un lint en service.
1. **Inventaire + schéma.** Écrire `docs/egress/README.md` (le schéma, le
   contrat des quatre directions, la procédure « j'ajoute un sink ») et
   `egress-manifest.toml` avec les 24+ entrées du back-fill. Produire au passage
   la classification des ~70 hosts distincts en trois seaux.
2. **Le lint, D1 + D3 seulement** (constructions de client). Vert sur main.
3. **Le lint, D2 + D4** (destinations). C'est l'étape qui découvre le bruit
   résiduel ; elle peut demander d'affiner `NON_SINK_HOSTS`. Vert sur main.
4. **Le test négatif** (N0–N9), chaque cas vu rouge puis vert.
5. **CI + Makefile + CODEOWNERS + AC5.**

**Back-fill attendu (AC2) — l'inventaire mesuré, 24 fichiers côté production :**

| famille | `client_site` | `class` | `destination_source` |
|---|---|---|---|
| Brave search | `mika-gateway/src/egress_search/mod.rs` | external | literal |
| gouv.fr fetch | `mika-gateway/src/egress_fetch/mod.rs` | external | literal |
| Telegram | `mika-gateway/src/main.rs` (partagé) | external | literal |
| GitHub REST/GraphQL (agent) | `mika-agent/src/github_graphql.rs` | external | literal |
| GitHub REST (gateway) | `mika-gateway/src/github.rs` | external | literal |
| GitHub App tokens | `mika-common/src/github_app.rs` | external | literal |
| Anthropic messages | `mika-common/src/claude.rs` | external | literal |
| Anthropic OAuth | `mika-common/src/oauth.rs` | external | literal |
| LLM OpenAI-compat (13 providers) | `mika-common/src/llm/openai.rs` | external | **config** |
| Ollama / MikaModel | `mika-common/src/llm/ollama.rs` | internal | **config** |
| Embeddings OpenAI | `mika-common/src/embedding.rs` | external | literal |
| Model listing | `mika-common/src/llm/models.rs` | external | config |
| Handler HTTP de skill | `mika-agent/src/skills/executor.rs` | **external** | **skill-declared** |
| A2A (agent↔spirit) | `mika-a2a/src/client.rs` | internal | config |
| … (agent/CLI/gateway internes) | 10 fichiers restants | internal | config |

Les trois lignes en gras sont celles qui portent une information qu'aucun lint
existant ne donne aujourd'hui, et le manifeste existe d'abord pour elles.

---

## Fire-Disposition

Ce plan livre un détecteur (`verify-egress-manifest.sh`, dont le chemin de
succès est « aucune violation trouvée »). Option retenue, parmi les trois de
mika#1574 :

### (a) — exception nommée en allowlist, **livrée VIDE et pinnée vide**

Doctrine mika#2201 : *« on déclare, on n'allowliste pas. »* Ici elle s'applique à
la lettre, parce que la résolution d'une violation **est** l'écriture d'une
ligne de manifeste — il n'existe aucun sink qu'on ne puisse pas déclarer, y
compris celui dont la destination est arbitraire (`execute_http` se déclare
`destination_source = "skill-declared"`).

Concrètement :

- `scripts/egress-manifest-exceptions.tsv`, **livré vide**, quatre colonnes
  obligatoires (`chemin`, `direction`, `ticket de suivi`, `raison`).
- Une **assertion auto-nettoyante** dans `test-verify-egress-manifest.sh` :
  chaque entrée doit encore correspondre à une violation réelle — une exception
  devenue stale fait rougir le build **le jour de la réparation**, pas des mois
  après. Même mécanisme que `scripts/canonical-tokens-exceptions.tsv`.
- Une assertion qui **pin le fichier vide** : quand elle tire, on lit la ligne
  qu'on vient d'ajouter et on se demande pourquoi ce sink ne peut pas se
  déclarer. Modèle : `canonical_tokens` et `check-pilot-push-sites.sh`.

**Ne pas confondre avec `NON_SINK_HOSTS`** (M2), qui n'est pas une allowlist
d'exception : c'est une liste de **classes de hosts non routables ou réservées
aux fixtures** (RFC 2606, loopback, `*.svc.cluster.local`). Elle est structurelle,
justifiée par classe, et elle ne référence aucun ticket de suivi parce qu'elle ne
décrit aucune violation. Les deux fichiers restent séparés pour que le
`wc -l` du second reste lisible.

**Condition de sortie de l'étape 3** : les quatre directions vertes sur `main`
avec un fichier d'exceptions à zéro ligne. Si l'implémentation n'y parvient pas,
elle **ne remplit pas l'allowlist pour faire passer le build** : elle bascule en
option (c) — halte-et-remontée — et nomme précisément le sink qui résiste. Un
détecteur livré avec cinq exceptions le jour de sa naissance est un détecteur
dont personne ne lira jamais la sixième.

---

## Definition of Done

- `docs/egress/egress-manifest.toml` existe, porte les 24+ sinks de production,
  et `docs/egress/README.md` documente le schéma et la procédure d'ajout.
- `scripts/verify-egress-manifest.sh` implémente D1–D4, prend un argument de
  chemin optionnel, et sort 0 sur `main`.
- `scripts/test-verify-egress-manifest.sh` implémente N0–N9, chaque cas ayant
  été vu rouge avant d'être vu vert, et sort 0.
- Le job CI `Egress Manifest Lint` et la cible `make verify-egress-manifest`
  existent et sont verts.
- `.github/CODEOWNERS` gate `/docs/egress/` sous `@samidarko`.
- `verify-egress-uniqueness.sh` porte `docs/egress/` dans `AUTHORIZED_PATHS`
  (obligatoire, M4b) et **le job `Egress Uniqueness Lint` est vert**.
- `verify-egress-uniqueness.sh` ne porte plus de liste de hosts propre (ses
  `PATTERNS` viennent du manifeste), **ou** un ticket de convergence est ouvert
  et cité dans son en-tête.
- `scripts/egress-manifest-exceptions.tsv` est présent et **vide**, avec ses
  deux assertions (auto-nettoyante + pin-vide).
- `verify-egress-no-log.sh` et `test-verify-egress-no-log.sh` sont **inchangés,
  au byte près** — le nouveau lint porte son propre parseur (M2 (ii) : une
  extraction fait rougir V13, une duplication crée une divergence que V13 ne
  peut pas voir). `git diff --stat` sur ces deux fichiers doit être vide.
- `verify-egress-request-shape.sh` est inchangé.

## Acceptance criteria

Transcrites du corps de mika#2408.

- [ ] **AC1 — sink non déclaré → CI échoue.** Un PR test ajoutant un appel
  client-HTTP vers un nouveau host sans entrée manifeste fait échouer le job.
  Pinné par un test de comportement négatif (comme `test-verify-egress-no-log.sh`).
- [ ] **AC1b — déclaration fantôme → CI échoue.** Un PR test ajoutant une entrée
  `[[sink]]` dont le `call_site` n'a AUCUN sink réel fait AUSSI échouer le job.
  Manifeste et code tenus en lockstep par le check, dans les deux sens ; les deux
  directions pinnées par test.
- [ ] **AC2 — back-fill des sinks existants** : Brave (`egress_search/`),
  gouv.fr (`egress_fetch/`), Telegram (`telegram.rs:371`), LLM
  (`mika-common/src/llm/openai.rs`, `ollama.rs`), GitHub (`github_graphql.rs`),
  OAuth (`oauth.rs`), `orchestrator_inbox.rs`. CI VERTE sur main après back-fill.
- [ ] **AC3 — déclaration humainement relisible** : TOML/YAML, une entrée/sink
  `{data, destination, logged, call_site, owner}` ; un reviewer voit dans un diff
  quelle donnée neuve va où, sans lire le Rust.
- [ ] **AC4 — lint scopé, bas bruit** : code de test, hosts mock
  (localhost/127.0.0.1/example.com/fixtures), appels internes loopback
  exclus/déclarés ; vert sur main = 0 faux positif.
- [ ] **AC5 — pas de 2e liste de hosts** : pas de liste concurrente de
  `verify-egress-uniqueness.sh` (lire le manifeste comme source unique, ou ticket
  de convergence lié).
- [ ] **AC6 — CODEOWNERS** : `docs/egress/` ajouté à `.github/CODEOWNERS` sous
  `@samidarko` (un nouveau flux ne merge pas sans revue humaine ; miroir de la
  perimeter forge-gate).

**Deux notes d'exécution sur les AC, écrites plutôt que découvertes.**

*Sur AC1b* : le ticket écrit « une entrée dont le `call_site` n'a aucun sink
réel ». R2 montre que ce prédicat, appliqué à `call_site`, refuserait la
déclaration correcte de Telegram. Le plan livre la propriété demandée — une
déclaration fantôme échoue — en la portant sur `client_site`, le champ
vérifiable, `call_site` restant documentaire. C'est un déplacement du champ,
pas une réduction de l'AC ; il est signalé ici pour que la revue le ratifie
plutôt que de le découvrir en diff.

*Sur AC2* : `orchestrator_inbox.rs` est cité dans la liste de back-fill. Mesure
R1 : son unique construction de client est en `#[cfg(test)]`. Il **entre quand
même** au manifeste, via son `client_site` réel (`main.rs`, client partagé) et
son `call_site` propre — donc l'AC est satisfaite, mais pas par le chemin que
sa formulation suggère.

---

## Sondes post-déploiement, et leurs haltes

Ce lint n'émet ni compteur, ni événement de journal : **son signal est son
propre rouge**, et son silence ne prouve rien tant qu'on n'a pas établi qu'il
regarde quelque chose.

**S1 — contrôle positif, obligatoire avant toute lecture du silence.**

```bash
bash scripts/verify-egress-manifest.sh --report
```
doit annoncer le nombre de fichiers inventoriés et d'entrées confrontées
(attendu : ~24 / ~24). *Halte 1* — un inventaire à **zéro** avec exit 0 : le
lint ne regarde plus rien (répertoire renommé, extension changée) et se lit
exactement comme un arbre propre (classe mika#2205). Le lint doit **refuser**
ce cas de lui-même (cas N8) ; si le refus n'arrive pas, c'est le refus qu'il
faut croire manquant, pas l'arbre qu'il faut croire sain.

**S2 — le rejeu du défaut fondateur (première PR ajoutant un sink).** Une PR qui
ajoute un appel vers un host neuf doit rougir. *Halte 2* — elle passe : lire
**laquelle** des quatre directions aurait dû tirer. Si c'est D2, la cause est
probablement dans `NON_SINK_HOSTS` (une classe trop large) ; si c'est D1, dans
la découpe production/test. Les deux remèdes diffèrent — **ne pas élargir le
prédicat par réflexe**.

**S3 — contrôle négatif de bruit (30 jours).** Aucune PR sans sink neuf ne doit
rougir. *Halte 3* — un faux positif sur une PR saine coûte une PR entière :
désarmer d'abord (retirer le step CI), diagnostiquer ensuite. Un faux positif
est un arbitrage de prédicat, pas un seuil à régler.

**S4 — l'allowlist reste vide.** `wc -l scripts/egress-manifest-exceptions.tsv`
doit rendre 0. *Halte 4* — une première entrée apparaît : c'est un sink que le
mécanisme ne sait pas déclarer, et le mécanisme est ce qu'il faut relire.

---

## Ce que ce travail n'achète PAS

- **Il ne dit pas ce qui part à l'exécution.** `destination_source = "config"`
  couvre 13 providers LLM repointables par variable d'environnement : le
  manifeste déclare la destination **du dépôt**, jamais celle du pod. Ce que le
  champ achète est que l'écart soit *nommé* au lieu d'être tu — c'est
  exactement le rapport que `llm_budget_resolved` entretient avec le
  `config.toml` (mika#2293).
- **Il ne vérifie pas le champ `logged`.** Ce champ est une **assertion
  humaine** relue sous CODEOWNERS ; sa vérification mécanique existe déjà pour
  un seul sink (`verify-egress-no-log.sh`, substrat Brave) et l'étendre aux 24
  autres est un travail d'un autre ordre. Le plan ne prétend pas le contraire —
  et un `logged = false` faux est le mode de panne le plus coûteux du fichier.
  **Ticket de suivi**, précondition : le back-fill vert.
- **Il sur-déclare, et c'est le prix choisi.** Le parseur `#[cfg(test)]` du
  nouveau lint est volontairement grossier et conclut « production » au moindre
  doute (M2), donc quelques fichiers dont le seul client vit en test devront se
  déclarer. Une ligne de manifeste de trop est visible et corrigible ; un sink
  omis est silencieux. L'asymétrie est le contraire de celle du parseur voisin,
  et c'est réfléchi : là-bas un faux « production » ferait rougir une garde
  STRIP-TOTAL sur du code de test ; ici il demande une déclaration.
- **Il ne couvre pas les egress non-HTTP.** `sqlx` vers Postgres, les sockets
  bruts, un `Command::new("curl")` — hors périmètre. Le dernier est déjà fermé
  ailleurs (containment shell-exec, mika#1991).
- **Il ne couvre pas `dashboard/` ni `packages/ui/`** (TypeScript) : l'inventaire
  est un prédicat Rust. Un `fetch()` côté navigateur est une population distincte
  avec un modèle de menace distinct. **Ticket de suivi**, précondition : une
  mesure montrant qu'un `fetch` du dashboard atteint un host tiers.
- **Il ne rattrape pas mika#1807 / mika#1808.** Le manifeste naît avec le
  back-fill ; rien ici ne rétro-date une déclaration pour un sink ajouté en
  août. La sonde est la **prochaine** PR qui ajoute un chemin sortant.

---

## Hors périmètre, délibérément

- **L'item bundlé « logs ingress/LB »** du ticket (NLB AWS L4, ConfigMap
  ingress-nginx, chemin GKE). Le ticket le pose lui-même *sans priorité*, et ses
  deux GAPS vivants sont **hors dépôt** : un plan de ce dépôt ne peut ni les
  mesurer ni les fermer. À traiter comme un item d'infrastructure opérateur,
  dans son propre ticket.
- **Étendre le parseur `#[cfg(test)]` de `verify-egress-no-log.sh`** — refusé
  avec sa raison en M2 : blast radius sur une garde STRIP-TOTAL en service pour
  un besoin qui n'est pas le sien. Si une mesure future montre que le mode
  best-effort produit trop de fichiers « tout-production », c'est **ce
  ticket-là** qui s'ouvre, avec le compte.
- **Vérifier mécaniquement `data`** (quelle donnée part). Un champ de prose que
  seul un humain peut juger ; le rendre vérifiable demanderait une analyse de
  flux que rien dans ce dépôt ne porte.
- **Un manifeste d'egress côté `mika-cloud`** — autre dépôt, absent de ce
  workspace.
