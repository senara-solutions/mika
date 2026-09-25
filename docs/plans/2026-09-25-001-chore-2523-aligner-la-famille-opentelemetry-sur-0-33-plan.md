# mika#2523 — Les trois erreurs sont un désalignement de version, pas trois ruptures d'API

> **Ticket** : senara-solutions/mika#2523 — « Migrer opentelemetry 0.31 → 0.33 dans mika-common »
> **Bloque** : #2301 (bump Dependabot, CI rouge)
> **Réveille** : mika#2139 (dormeur, `docs/dormeurs.md`) — dont la condition de réveil est **incomplète**, et c'est ce qui a produit #2301
> **Voisins** : #2008 (première occurrence de la classe, `opentelemetry_sdk` 0.31→0.32 rouge en CI), mika#1729 (config Dependabot), mika#1913 (Langfuse v4, dormeur, échéance 2026-11-16)

---

## 0. Ce que la lecture du code déplace dans le ticket

Le ticket a bien fait le travail d'observation : il a relevé les trois erreurs, leur
code, leur job. Mais il en tire un diagnostic — *« c'est une **vraie rupture d'API**,
pas un bump trivial »*, *« les 3 ruptures »*, *« résolues par migration d'API »* — que
la lecture des sources **réfute**. Les cinq rectifications ci-dessous sont le premier
livrable de ce plan, parce que chacune change le remède.

### R1 — Il n'y a pas trois ruptures. Il y a une ligne manquante.

`tracing-opentelemetry 0.32.1` épingle `opentelemetry = "^0.31.0"`
(`~/.cargo/registry/src/…/tracing-opentelemetry-0.32.1/Cargo.toml:137-140`). En semver
pre-1.0, `^0.31` vaut `>=0.31.0, <0.32.0` : **0.33 n'est pas couvert**. Le bump #2301 a
monté `opentelemetry`, `opentelemetry_sdk` et `opentelemetry-otlp` vers 0.33 en laissant
`tracing-opentelemetry` à 0.32 — et cargo, qui **résout sans compiler**, a produit un
graphe légal portant **deux copies** d'`opentelemetry` : la 0.31 pour
`tracing-opentelemetry`, la 0.33 pour notre code et pour le sdk.

Les trois erreurs sont les trois signatures canoniques de ce diamant, à la ligne près :

| # | erreur | site | ce qui se passe réellement |
|---|---|---|---|
| 1 | E0271 `Item == KeyValue` | `telemetry.rs:76-80` | `Resource` vient d'`opentelemetry_sdk` 0.33 et attend `opentelemetry_v0.33::KeyValue` ; le `KeyValue` importé résout vers l'autre copie |
| 2 | E0599 `tracer` absent | `telemetry.rs:83` | `use opentelemetry::trace::TracerProvider` importe le trait d'**une** copie ; `SdkTracerProvider` implémente celui de **l'autre** |
| 3 | E0599 `span` absent | `trace.rs:15` | `OpenTelemetrySpanExt::context()` (tracing-opentelemetry 0.32) rend un `opentelemetry_v0.31::Context` ; le `TraceContextExt` importé est celui de la 0.33 |

**La forme des erreurs est elle-même la preuve.** Une rupture d'API réelle rend
« argument de type X attendu, Y trouvé », « champ inconnu », « ce constructeur prend 2
arguments ». « Trait non implémenté sur ce type », « méthode absente », « `Item` ne
correspond pas » sont les trois formes que produit un type homonyme de deux versions.

**Le remède est une quatrième ligne**, mesurée sur l'index cargo local
(`~/.cargo/registry/index/…/.cache/tr/ac/tracing-opentelemetry`, index daté du
2026-09-24) :

| `tracing-opentelemetry` | exige `opentelemetry` |
|---|---|
| 0.32.1 (en vigueur) | `^0.31.0` |
| 0.33.0 | `^0.32.0` |
| **0.34.0** | **`^0.33.0`** ✅ |

`tracing-opentelemetry = "0.34"` referme les trois erreurs d'un seul trait. **Aucun des
cinq fichiers ne demande de migration d'API pour ces trois erreurs** — ce qu'AC1 nomme
« migration d'API » est un bump de dépendance que Dependabot a fait aux trois quarts.

### R2 — Sur les surfaces que le code touche, l'API n'a pas bougé de 0.31 à 0.32

Établi par `diff` des sources en cache (0.31.0 et 0.32.x sont tous deux présents
localement) :

| surface | 0.31 → 0.32 |
|---|---|
| `opentelemetry::trace::context` (`TraceContextExt`, `SpanRef::span_context`) | **fichier byte-identique** |
| `opentelemetry::common` (`KeyValue`, `Key`, `Value`) | deux lignes de doc, `to_string`→`into_string` sur un interne privé |
| `opentelemetry::trace::span_context` | **additif** (`TraceStateIter`) |
| `opentelemetry::trace::tracer_provider` (`TracerProvider::tracer`) | une URL dans un doc-comment |
| `opentelemetry_sdk::trace::provider` (`SdkTracerProvider`) | seul `with_sampler` a changé — **non utilisé** |
| `opentelemetry_sdk::resource` (`Resource`) | **additif** (`get_ref`) |

Ce n'est pas la preuve que 0.32 → 0.33 est aussi calme — cette moitié est **inconnue**
et §5 en fait le point de vérification. Mais ça borne la classe de risque : sur deux
crans consécutifs, les six surfaces que mika-common consomme n'ont pris aucune rupture.

### R3 — Ce ticket réveille un dormeur dont la condition de réveil MENT

`docs/dormeurs.md` porte, ligne 45 :

> | #2139 | migration de la famille `opentelemetry` d'un seul bloc (0.31 → 0.32) | quand `opentelemetry`, `opentelemetry_sdk` et `opentelemetry-otlp` ont une version **mutuellement alignée** publiée sur crates.io |

La condition nomme **trois** crates et **omet `tracing-opentelemetry`** — exactement la
quatrième, celle dont le désalignement produit les trois erreurs. Les trois crates
nommées *sont* alignées à 0.33 aujourd'hui : la condition se lit **remplie**, et le bump
qu'elle autorise ne compile pas. C'est la classe que le contrat du registre existe pour
refuser (*« un lecteur peut dire, sans contexte, si elle est remplie »*) : une condition
satisfaite par un travail impossible n'est pas une condition, c'est un feu vert.

**Conséquence de conduite :** ce plan ne se contente pas de retirer la ligne (ce que le
contrat du registre exige au réveil, motif mika#2420/U6). Il doit livrer la moitié
structurelle qui empêche la quatrième crate d'être oubliée une troisième fois — voir
§4. La classe est mesurée **n = 3** : #2008 (2026-08, `opentelemetry_sdk` 0.31→0.32
rouge en CI, cité par `docs/solutions/workflow-issues/enabling-an-automated-pr-author-without-a-gate-path-2026-08-27.md:52`), #2139/#2301, et ce ticket.

### R4 — « non-substrat, ne casse pas la boucle » est vrai aujourd'hui et faux au merge

Le ticket classe le périmètre *« non-substrat (ne casse pas la boucle ; bloque un bump
dependabot) »*. C'est exact **tant que #2301 n'est pas mergé**. Mais
`Makefile:9-10` :

```make
build: ## Build release binaries with telemetry
	cargo build --release --features telemetry
```

`make deploy` passe par `make build`. Donc un bump de cette famille qui ne compile pas
sous `--features telemetry` **casse le déploiement**, pas seulement le job CI
`cargo test --workspace --features telemetry` (`.github/workflows/ci.yml:113-114`). Ce
qui protège la boucle aujourd'hui est que le bump est rouge et donc non mergé — le gate,
pas l'innocuité. La priorité de file reste celle que le ticket donne (après les p1) ;
ce qui change est qu'on ne merge pas ceci sans que `make build --features telemetry`
soit vert localement.

### R5 — AC2 est satisfaite à vide par la suite actuelle

`cargo test -p mika-common --features telemetry` compte **trois** tests de télémétrie
(`telemetry.rs:145-192`) : deux sur `normalize_auth_header` (une fonction pure, sans
rapport avec l'API otel) et `test_build_otel_layer_disabled`, qui assert `is_none()` sur
le **chemin désactivé** — celui qui rend `None` avant d'avoir touché une seule API
d'opentelemetry. Un `grep` sur `build_otel_layer|try_init_otel|TelemetryGuard` hors du
module rend trois sites de production (`mika-cli/src/main.rs`,
`mika-agent/src/bin/mika-spirit.rs`) et **zéro test**.

Donc : **aucun test n'exerce le chemin armé**, qui est précisément celui que le bump
casse. AC2 (« tests verts ») est aujourd'hui une assertion sur la compilation déguisée
en assertion sur le comportement, et AC3 (« aucune régression fonctionnelle de
l'export ») n'a **aucun instrument**. §3 livre le test manquant.

---

## 1. Requirements

| # | requirement | AC |
|---|---|---|
| **U1** | Les **quatre** crates de la famille otel montent ensemble : `opentelemetry` 0.33, `opentelemetry_sdk` 0.33, `opentelemetry-otlp` 0.33, `tracing-opentelemetry` **0.34**. `Cargo.lock` régénéré et committé. | AC1, AC4 |
| **U2** | Tout résidu d'API réel entre 0.32 et 0.33 est migré sur les six surfaces inventoriées en §2, aux cinq fichiers concernés. Conditionnel : **si** la compilation en trouve. | AC1 |
| **U3** | Le chemin **armé** de `build_otel_layer` est couvert par un test, sans réseau ni runtime. Les clés `gen_ai.*` émises sont épinglées. | AC2, AC3 |
| **U4** | La quatrième crate ne peut plus être oubliée : un scan hors compilation refuse un `Cargo.lock` où `mika-common` et `tracing-opentelemetry` ne partagent pas la même copie d'`opentelemetry`. | — (moitié structurelle de R3) |
| **U5** | La famille est groupée dans `dependabot.yml` pour que le bump soit **atomique** à la source. | — (moitié structurelle de R3) |
| **U6** | Le dormeur #2139 est retiré de `docs/dormeurs.md` ; `CLAUDE.md:23` nomme les versions en vigueur. | — (contrat du registre) |
| **U7** | #2301 est disposé — geste **opérateur**, nommé et non exécuté par le pilote. | AC4 |

**Invariant transverse :** aucune clé `gen_ai.*` ne change de nom, de type ni de site.
L'export Langfuse est un format de fil lu par un backend externe ; un renommage
opportuniste pendant un bump serait une régression invisible en CI.

---

## 2. Conception — U1/U2 : l'alignement, et l'inventaire de ce qui pourrait résister

### U1 — La quatrième ligne

`Cargo.toml:54-57`, un seul bloc :

```toml
# OpenTelemetry (optional per-crate via "telemetry" feature)
# Les QUATRE montent ensemble : tracing-opentelemetry <N> exige opentelemetry ^<N-1>
# (0.34 → ^0.33). Un bump partiel produit deux copies dans le graphe, que cargo
# résout sans erreur et que seul `--features telemetry` refuse — mika#2523.
opentelemetry = "0.33"
opentelemetry_sdk = { version = "0.33", features = ["rt-tokio"] }
opentelemetry-otlp = { version = "0.33", features = ["http-proto"] }
tracing-opentelemetry = "0.34"
```

`crates/mika-common/Cargo.toml` n'est **pas** touché : ses quatre déclarations sont
`workspace = true`.

Deux features à re-vérifier plutôt qu'à supposer, dans cet ordre :

- **`http-proto`** sur `opentelemetry-otlp` — `opentelemetry-otlp 0.33.0` existe bien
  dans l'index local ; le nom de la feature n'a pas été lu en 0.33. Si elle a disparu,
  l'échec est un `unknown feature`, immédiat et sans ambiguïté.
- **`rt-tokio`** sur `opentelemetry_sdk` — **probablement déjà morte chez nous.**
  `with_batch_exporter` en 0.32.1 ne prend **aucun** paramètre de runtime
  (`opentelemetry_sdk-0.32.1/src/trace/provider.rs:334`), il construit
  `BatchSpanProcessor::builder(exporter).build()`, et ce processeur tourne sur un
  **thread OS dédié** (`span_processor.rs:365`, `thread::Builder::new().spawn`), pas sur
  tokio. `telemetry.rs` est le seul consommateur du sdk et n'utilise rien d'autre.
  Si la feature survit en 0.33, la laisser (retirer une feature morte est un
  changement orthogonal, hors de ce ticket) ; si elle a disparu, la retirer est la
  seule action et elle ne coûte rien.

### U2 — Les six surfaces, et les cinq fichiers

Inventaire exhaustif (`grep -rln opentelemetry crates/ --include="*.rs"`), tous les
sites sous `#[cfg(feature = "telemetry")]` :

| fichier | surfaces consommées |
|---|---|
| `crates/mika-common/src/telemetry.rs` | `SdkTracerProvider::{builder, with_batch_exporter, with_resource, build, shutdown}` · `opentelemetry::trace::TracerProvider::tracer` · `KeyValue::new` · `Resource::builder().with_attributes().build()` · `opentelemetry_otlp::{SpanExporter, WithExportConfig, WithHttpConfig}` + `builder().with_http().with_endpoint().with_headers().build()` · `tracing_opentelemetry::layer().with_tracer()` |
| `crates/mika-common/src/trace.rs` | `OpenTelemetrySpanExt::context` · `TraceContextExt::span` · `SpanRef::span_context` · `SpanContext::trace_id` · `TraceId::INVALID` |
| `crates/mika-common/src/llm/openai.rs` | `OpenTelemetrySpanExt::set_attribute` × 2 blocs |
| `crates/mika-common/src/llm/ollama.rs` | idem × 2 blocs |
| `crates/mika-common/src/claude.rs` | idem × 2 blocs |

`set_attribute` a pour signature `(impl Into<Key>, impl Into<Value>)`
(`tracing-opentelemetry-0.32.1/src/span_ext.rs:152`), et les six blocs ne lui passent
que des `&'static str`, `String` et `i64` — les trois conversions les plus stables de
l'API. **Aucune migration n'est attendue ici** ; la ligne est dans l'inventaire parce
que c'est la surface la plus dupliquée (6 blocs, 3 rails) et que si elle bougeait, le
coût serait mécanique et réparti.

**U2 est conditionnel par construction et c'est assumé.** Le bac à sable de grooming n'a
pas le réseau (`curl` refusé par la permission-policy) et les crates 0.33 ne sont pas en
cache : le delta 0.32 → 0.33 **n'a pas pu être lu**. Le plan ne prétend donc pas qu'il
est vide — il pose le remède principal comme établi (R1, mesuré sur l'index), borne le
risque résiduel (R2, mesuré sur les sources), et fait de la compilation le juge. La
conduite si un résidu apparaît est en §5, halte H2.

---

## 3. Conception — U3 : le test du chemin armé

Un nouveau test dans `crates/mika-common/src/telemetry.rs`, module `tests` existant :

```rust
/// mika#2523 : le chemin ARMÉ de `build_otel_layer` n'était couvert par aucun test —
/// seul le chemin désactivé l'était, et il rend `None` avant de toucher une API otel.
/// C'est donc exactement le chemin que le bump 0.31→0.33 casse, et celui qu'AC2/AC3
/// demandent de garder. Sans réseau : la construction de l'exporter HTTP ne se
/// connecte pas, et le BatchSpanProcessor tourne sur un thread OS dédié (pas tokio),
/// donc un `#[test]` sync suffit.
#[cfg(feature = "telemetry")]
#[test]
#[serial]
fn mika2523_the_armed_path_builds_a_layer_and_a_guard() { … }
```

Forme, et chaque contrainte a sa raison :

- `#[serial]`, comme son voisin `test_build_otel_layer_disabled` : il pose
  `MIKA_TELEMETRY_ENABLED` / `MIKA_OTLP_ENDPOINT` dans l'environnement du process, et
  `#[serial]` ne séquence que ses propres porteurs (leçon mika#2073).
- Endpoint `http://127.0.0.1:1/v1/traces` — le port 1 garantit qu'aucun octet ne partira
  même si le thread de batch s'éveille. Le chemin `/v1/traces` complet est requis :
  `with_endpoint` n'ajoute aucun segment (`docs/solutions/integration-issues/otlp-endpoint-path-requirement.md`).
- Aucun span n'est émis, donc la file du batch est vide et le `shutdown()` du `Drop`
  n'attend rien.
- Assertion : `build_otel_layer(&settings).is_some()`. Le guard est droppé en fin de
  test, ce qui exerce aussi `TelemetryGuard::drop`.
- **Contrôle négatif déjà présent** : `test_build_otel_layer_disabled` reste inchangé et
  atteste que le prédicat de `settings.telemetry_enabled` décide encore. Sans lui, un
  `build_otel_layer` devenu inconditionnel passerait le test ci-dessus.

Un second test, pur et sans environnement, épingle l'invariant transverse d'AC3 — la
**liste** des clés `gen_ai.*` émises par les trois rails :

```rust
/// mika#2523 / AC3 : les clés `gen_ai.*` sont un format de fil lu par Langfuse.
/// Un renommage opportuniste pendant un bump de dépendance serait une régression
/// qu'aucun test de compilation ne voit. Scan de source sur les trois rails.
#[test]
fn mika2523_the_gen_ai_attribute_keys_are_a_wire_format() { … }
```

Prédicat : les huit clés (`gen_ai.operation.name`, `gen_ai.provider.name`,
`gen_ai.request.model`, `gen_ai.request.max_tokens`, `gen_ai.usage.input_tokens`,
`gen_ai.usage.output_tokens`, `gen_ai.response.finish_reasons`, `gen_ai.prompt`,
`gen_ai.completion` — neuf) sont présentes dans `llm/openai.rs`, `llm/ollama.rs` et
`claude.rs`, aux cardinalités mesurées. **Anti-vacuité obligatoire** : le scan échoue
si un fichier n'est pas lisible ou ne porte aucune clé — un scan pointé sur un chemin
mort se lit exactement comme un arbre propre (classe mika#2205).

**Ce que U3 n'achète pas, écrit plutôt que découvert.** Il atteste que le layer se
**construit** et que les clés sont **écrites**. Il n'atteste pas qu'un span arrive dans
Langfuse — ça demande un backend joignable, donc un geste opérateur (§6). AC3 est donc
livrée en deux moitiés : structurelle (testée) et fonctionnelle (sonde).

---

## 4. Conception — U4/U5 : la quatrième crate ne peut plus être oubliée

### U4 — Le scan, et pourquoi il doit vivre HORS de la compilation

Le job `cargo test --workspace --features telemetry` détecte déjà le symptôme. Ce que
le scan ajoute est décisif et mesuré : **le diagnostic**. Les trois E0271/E0599 ne
nomment pas leur cause — la preuve étant que le ticket, écrit à partir de ces trois
erreurs, a conclu « vraie rupture d'API » et cadré un travail de migration là où il y
avait une ligne de dépendance. Le scan remplace trois erreurs de type par une phrase
actionnable.

**Le détecteur ne peut pas être un test Rust.** Sur un désalignement, `mika-common` ne
compile pas : un test du crate ne tourne **pas du tout**, donc il serait inerte
exactement quand on a besoin de lui. D'où un script + job CI, motif établi du dépôt
(11 `scripts/check-*.sh`, autant de jobs `*-lint`).

**Le prédicat n'a PAS de table de compatibilité**, et c'est ce qui l'empêche de pourrir.
Il lit la forme de `Cargo.lock` : quand une seule copie d'un paquet existe, les
références s'écrivent sans version (`"opentelemetry"`) ; quand il y en a plusieurs, elles
portent la version (`"thiserror 2.0.20"` — la preuve vivante de la convention est
**déjà dans ce fichier**, ligne 2655). Donc :

> **Les blocs `[[package]] name = "mika-common"` et `[[package]] name = "tracing-opentelemetry"` doivent désigner la même entrée `opentelemetry`.**

Quatre propriétés, chacune contre un mode d'échec nommé :

1. **Résiste au faux positif tiers.** Si demain une dépendance tierce épingle otel 0.32
   et ajoute une troisième copie, `mika-common` et `tracing-opentelemetry` peuvent
   toujours partager la leur : le scan reste **vert** sur un état sain. Un prédicat
   « une seule copie dans le graphe » rougirait, on lui ajouterait une allowlist, et il
   se viderait de sens.
2. **Pas de version écrite nulle part.** Aucune table `0.34 → ^0.33` à maintenir, donc
   rien à mettre à jour au cran suivant.
3. **Anti-vacuité.** Le scan échoue si l'un des deux blocs est introuvable, ou si l'un
   ne porte aucune référence `opentelemetry`. Sans ça, un renommage de crate ou un
   `Cargo.lock` déplacé rendrait un scan silencieusement inerte, indistinguable d'un
   arbre propre.
4. **Contrôle négatif vu rouge.** `scripts/test-check-otel-version-alignment.sh`, sur
   une fixture `Cargo.lock` désalignée (`"opentelemetry 0.31.0"` d'un côté,
   `"opentelemetry 0.33.0"` de l'autre) — discipline mika#2103, appliquée par
   `dispatch-seats-lint` et `landing-tokens-lint` : *une garde que personne n'a vue
   rougir est une décoration*.

Livrables : `scripts/check-otel-version-alignment.sh`,
`scripts/test-check-otel-version-alignment.sh`, cible `make test-otel-alignment`, job CI
`otel-alignment-lint`. Le message de refus **nomme le remède** : « monte
`tracing-opentelemetry` au cran qui exige cette version d'`opentelemetry` ; les quatre
crates de la famille bougent ensemble (mika#2523) ».

### U5 — Le groupe Dependabot

`.github/dependabot.yml` ne porte qu'un groupe `cargo-minor-patch` par `update-types`.
Pour cargo, un bump `0.31 → 0.33` est un `semver-minor` du point de vue de Dependabot et
tombe donc dans ce groupe — mais Dependabot **met à jour chaque dépendance directe
indépendamment** et ne modélise pas la contrainte qu'une de nos deps directes impose sur
une autre. Cargo réussit la résolution (deux copies, c'est légal), Dependabot ne compile
pas : **le désalignement est silencieux jusqu'au CI.**

Un groupe nommé, déclaré **avant** `cargo-minor-patch` (l'ordre des clés YAML décide de
l'assignation), sur des **patterns de noms** et non de versions :

```yaml
      # mika#2523 : les quatre crates de la famille otel ne se bumpent pas
      # séparément. `tracing-opentelemetry <N>` exige `opentelemetry ^<N-1>` ; un bump
      # partiel produit deux copies dans le graphe, que cargo résout sans erreur et que
      # seul `--features telemetry` refuse. Mesuré trois fois : #2008, #2139, #2301.
      opentelemetry-family:
        patterns:
          - "opentelemetry*"
          - "tracing-opentelemetry"
```

**Ce que U5 n'achète pas :** il rend le bump **atomique**, pas **compilable**. Si
`tracing-opentelemetry` n'a pas encore de cran compatible au moment du bump, le PR
groupé sera rouge — mais rouge pour la bonne raison, et lisible : les quatre lignes
seront dans le diff. C'est U4 qui donne le message.

### U6 — Le dormeur et la doc

- `docs/dormeurs.md` : retirer la ligne #2139. Le contrat du registre dit *« rouvrir le
  ticket GitHub cité et retirer la ligne d'ici »* ; ici le travail est **absorbé par
  #2523**, donc la ligne part avec cette raison et #2139 n'est pas rouvert. Motif
  identique à mika#2420/U6 pour #1694.
- `CLAUDE.md:23` : `opentelemetry 0.33 + tracing-opentelemetry 0.34`.
- `docs/dormeurs.md` n'est **pas** dans la liste `DOCS` de `scripts/sync-agent-docs.sh`,
  et aucun fichier qui l'est ne mentionne de version otel : **le job `docs-sync` n'a rien
  à faire ici.**
- `docs/solutions/integration-issues/otlp-endpoint-path-requirement.md` nomme « 0.31 »
  dans son titre et son corps. **Laissé intact** : c'est un document daté qui rapporte ce
  qui a été mesuré sur 0.31, et le réécrire au présent ferait passer une observation pour
  une vérité intemporelle. Son invariant (`with_endpoint` n'ajoute aucun segment) est
  re-vérifié par le test U3, qui échouerait s'il avait cessé d'être vrai.

### U7 — #2301, geste opérateur

Le pilote d'implémentation **n'a pas de `gh` authentifié** (mesuré : `gh issue view`
refuse dans ce bac à sable), donc ce geste est nommé et non exécuté. L'ordre est
contraint :

1. Merger la PR de migration (elle porte les quatre lignes + `Cargo.lock`).
2. **Ensuite** disposer de #2301. Dependabot ferme de lui-même un PR dont la dépendance
   a atteint la version cible sur `main` ; un commentaire nommant la PR de migration
   rend la fermeture lisible plutôt qu'automatique et muette.

**Ne pas fermer #2301 avant le merge** : Dependabot le recréerait au cycle suivant, le
bump n'étant pas encore appliqué.

---

## 5. Contrat de vérification

Dans cet ordre, chaque étape étant la précondition de la suivante.

| # | vérification | commande | attendu |
|---|---|---|---|
| V1 | Le graphe ne porte qu'une copie par paquet otel | `cargo tree -p mika-common --features telemetry -i opentelemetry` | **une seule** version listée |
| V2 | La compilation du chemin armé | `cargo build -p mika-common --features telemetry` | vert, **zéro** E0271/E0599 |
| V3 | Le scan U4 voit et accepte | `bash scripts/check-otel-version-alignment.sh` | exit 0, en annonçant les deux blocs lus |
| V4 | Le scan U4 rougit sur la fixture désalignée | `bash scripts/test-check-otel-version-alignment.sh` | vert (le contrôle négatif passe) |
| V5 | Les tests du crate | `cargo test -p mika-common --features telemetry` | vert, **U3 inclus** |
| V6 | La suite du CI | `cargo test --workspace --features telemetry` | vert |
| V7 | Le déploiement (R4) | `cargo build --release --features telemetry` | vert |
| V8 | Sans la feature, rien n'a bougé | `cargo test -p mika-common` | vert |

**H1 — V2 rouge sur E0271/E0599 après l'alignement.** Le diagnostic R1 serait faux.
**Ne pas commencer à réécrire les sites** : relancer V1 d'abord. Deux copies encore
présentes signifient qu'une **cinquième** crate du graphe épingle une otel ancienne et
que la contrainte vient d'ailleurs — c'est alors cette crate qu'il faut nommer, pas nos
sites qu'il faut migrer.

**H2 — V2 rouge sur une erreur d'une AUTRE forme** (« argument de type X attendu »,
« champ inconnu », « ce constructeur prend N arguments »). C'est **le résidu réel
0.32 → 0.33**, celui que ce plan n'a pas pu lire. Conduite : migrer surface par surface
dans l'ordre de l'inventaire §2, en documentant chaque changement d'API dans le corps de
la PR. Ne pas contourner par un pin, une exclusion Dependabot ou un
`#[allow]` — AC1 l'interdit explicitement, et la conduite est de migrer ou de s'arrêter.

**H3 — aucune version de `tracing-opentelemetry` n'exige otel 0.33.** L'index local dit
que 0.34.0 le fait ; si `cargo update` le réfute (index rafraîchi, version yankée), alors
AC1 est **inatteignable sans contournement** et la conduite est de **s'arrêter** : ne pas
forker, ne pas pinner, ne pas dupliquer d'API. Remettre #2139 en dormeur avec sa condition
**corrigée à quatre crates** (c'est le défaut R3, et le corriger vaut plus que le bump),
et laisser #2301 fermé jusqu'au cran suivant.

**H4 — V5 rouge sur le test armé, V2 vert.** La compilation passe, la construction du
layer échoue : c'est une régression **fonctionnelle** de 0.33, pas un désalignement. Lire
le `warn!` de `build_otel_layer` (il dégrade gracieusement et journalise la cause : build
d'exporter, endpoint, headers) **avant** de toucher au test — le test mesure, il ne
décide pas.

**H5 — V3 vert sans avoir rien lu.** Le scan doit annoncer les deux blocs qu'il a
trouvés. Un exit 0 muet est indistinguable d'un scan pointé sur un chemin mort (classe
mika#2205), et c'est ce que l'anti-vacuité de §4 refuse — si elle se déclenche, réparer
le chemin, **jamais baisser le prédicat**.

---

## 6. Sondes post-déploiement, et leurs haltes

Aucun compteur, aucun événement de journal n'est livré : le défaut est une compilation
rouge, et une compilation rouge ne s'émet pas. Les instruments sont ceux qui existent.

**S1 — l'export vit encore (AC3, moitié fonctionnelle).** Geste **opérateur**, avec un
Langfuse joignable. Après `make deploy`, poser `MIKA_TELEMETRY_ENABLED=1` +
`MIKA_OTLP_ENDPOINT` + `MIKA_OTLP_AUTH_HEADER`, faire un tour LLM, et vérifier qu'une
Generation apparaît avec ses `gen_ai.request.model`, `gen_ai.usage.*` et
`gen_ai.response.finish_reasons`.
*Halte* — aucune trace n'arrive : lire d'abord le `warn!` de `build_otel_layer` dans
`$MIKA_SPIRIT_LOG_FILE`, et **établir le déploiement** (le binaire servi porte-t-il le
bump ? classe mika#2340) **avant** toute conclusion sur l'API. Une absence de span ne
prouve rien tant qu'on n'a pas établi que le binaire qui tourne sait en émettre.

**S2 — le `trace_id` reste corrélé.** `generate_trace_id()` (`trace.rs`) extrait le
trace-id du span courant quand la télémétrie est active, et retombe sur un UUID v4 sinon.
Les deux rendent 32 caractères hex, donc **les deux tests existants passent dans les deux
régimes** et ne peuvent pas distinguer une extraction cassée d'un repli. Avec la
télémétrie armée, un `trace_id` de log doit se retrouver dans la trace Langfuse
correspondante.
*Halte* — s'ils ne correspondent plus, le repli est silencieux : c'est le site 3 de R1
(`TraceContextExt::span`) qui compile mais ne trouve plus de contexte. Ne pas conclure de
la longueur du champ.

**S3 — contrôle positif du prochain bump (U5, à la première occasion réelle).** Le
prochain PR Dependabot de la famille otel doit porter les **quatre** lignes dans un seul
diff.
*Halte* — il n'en porte que trois : le groupe ne prend pas (ordre des clés, pattern
inopérant). C'est **U4** qui rattrape alors, et sa ligne de refus est la preuve que la
chaîne tient à deux étages plutôt qu'à un.

---

## 7. Fire-Disposition

Ce plan livre **un détecteur** : le scan U4
(`scripts/check-otel-version-alignment.sh`), dont le chemin de succès est « aucun
désalignement trouvé ».

*(Les deux tests de U3 n'en sont pas : `mika2523_the_armed_path_builds_a_layer_and_a_guard`
atteste un comportement positif — la fonction rend `Some` — et non l'absence d'une
population. `mika2523_the_gen_ai_attribute_keys_are_a_wire_format` est en revanche un scan
de source : il est donc couvert par la disposition ci-dessous, au même titre.)*

**Disposition retenue : (a) exception nommée en allowlist — livrée VIDE.**

- **Population actuelle : zéro.** Mesuré sur le `Cargo.lock` de `292cce60` : les six
  paquets de la famille (`opentelemetry`, `opentelemetry-http`, `opentelemetry-otlp`,
  `opentelemetry-proto`, `opentelemetry_sdk`, `tracing-opentelemetry`) apparaissent
  **exactement une fois chacun**, et les deux blocs contrôlés désignent la même entrée
  `"opentelemetry"`, sans version. Il n'y a donc **rien à exempter** — le scan atterrit
  vert sur l'arbre d'aujourd'hui, et vert aussi sur l'arbre d'après le bump.
- **Support de l'allowlist :** `scripts/otel-alignment-allowlist.txt`, **créé vide**
  (en-tête de commentaire seul), une entrée par ligne au format
  `<paquet>  # <raison>  # <ticket de suivi>`.
- **Assertion auto-nettoyante :** `scripts/test-check-otel-version-alignment.sh`
  compare l'allowlist **dans les deux sens** — motif `dispatch-seats-lint` (mika#2092) et
  `check-pilot-push-sites.sh` (mika#2520). Une entrée qui ne correspond plus à aucun
  désalignement réel **fait rougir le build**, ce qui empêche une exemption de survivre à
  sa cause.
- **Résolution quand le scan tire : on aligne le site, on n'allowliste pas** (doctrine
  mika#2201). Le scan porte sur une famille de quatre crates dont l'alignement **est** la
  correction ; une entrée d'allowlist y serait un désalignement ratifié, c'est-à-dire la
  panne que le scan existe pour refuser. L'allowlist n'existe que pour le cas non mesuré
  d'une contrainte tierce irréductible, et son emploi exige un ticket de suivi nommé dans
  l'entrée.
- **Pour le scan de clés `gen_ai.*` (U3) :** aucune allowlist, et il n'y a pas de
  constante pour en créer une — **halte-et-remontée**. Une clé de ce format qui
  disparaît est une régression de l'export vers Langfuse, jamais une exception à ratifier :
  quand il tire, on restaure la clé.

---

## 8. Hors périmètre, délibérément

- **La migration de `rt-tokio` hors de notre déclaration** si la feature survit en 0.33.
  Établi ci-dessus qu'elle est probablement morte pour nous (`with_batch_exporter` ne
  prend aucun runtime), mais la retirer est un changement orthogonal, sans rapport avec
  les trois erreurs, et qui mélangerait deux diffs. **Suivi**, précondition : que V2 soit
  vert *avec* la feature, ce qui prouve qu'elle n'est pas requise.
- **mika#1913 — compatibilité Langfuse v4** (dormeur, échéance 2026-11-16). Adjacent par
  le sujet (endpoint OTLP + format d'auth, donc `normalize_auth_header` et
  `with_endpoint`), disjoint par la cause : v4 est une rupture **du backend**, pas de la
  crate. Ce bump ne l'anticipe pas et ne doit pas l'absorber.
- **Les autres dépôts du workspace.** `mika-cloud` et `mika-platform` peuvent porter la
  même famille de deps ; le worktree de dispatch ne matérialise que le sous-dépôt `mika/`,
  donc c'est structurellement invérifiable ici. Un ticket par dépôt, avec ce plan comme
  référence.
- **La correction de la condition de réveil de #2139 comme travail en soi.** La ligne est
  **retirée** (U6), donc la condition disparaît avec elle. Elle ne serait à réécrire que
  sur la halte H3, et c'est H3 qui le prescrit.
- **Un scan qui vérifierait une table de compatibilité `tracing-opentelemetry` ↔
  `opentelemetry`.** Refusé avec sa raison : la table serait à maintenir à chaque cran,
  donc soit périmée soit allowlistée, et U4 obtient le même refus **sans** table en lisant
  la forme de `Cargo.lock`.
- **Le passage de `SimpleSpanProcessor` ou tout réglage du batch** (taille de file, délai,
  timeout d'export). Aucune mesure ne le demande.

---

## 9. Definition of Done

- [ ] Les **quatre** crates otel alignées dans `Cargo.toml` (0.33 / 0.33 / 0.33 / **0.34**), avec le commentaire qui nomme la contrainte ; `Cargo.lock` régénéré et committé.
- [ ] V1 rend une seule version d'`opentelemetry` dans le graphe de `mika-common --features telemetry`.
- [ ] V2, V5, V6, V7, V8 verts. Tout résidu d'API réel 0.32→0.33 migré et **documenté dans le corps de la PR**, surface par surface.
- [ ] `mika2523_the_armed_path_builds_a_layer_and_a_guard` livré et vert ; `test_build_otel_layer_disabled` **inchangé** et toujours vert (son rôle de contrôle négatif).
- [ ] `mika2523_the_gen_ai_attribute_keys_are_a_wire_format` livré, avec son anti-vacuité.
- [ ] `scripts/check-otel-version-alignment.sh` + `scripts/test-check-otel-version-alignment.sh` + `scripts/otel-alignment-allowlist.txt` (**vide**) + cible `make test-otel-alignment` + job CI `otel-alignment-lint`. Le contrôle négatif **vu rouge** avant d'être rendu vert.
- [ ] Groupe `opentelemetry-family` dans `.github/dependabot.yml`, déclaré **avant** `cargo-minor-patch`, avec son commentaire de raison.
- [ ] Ligne #2139 retirée de `docs/dormeurs.md` ; `CLAUDE.md:23` à jour.
- [ ] Aucune clé `gen_ai.*` renommée, retypée ou déplacée ; aucun `#[allow]`, aucun pin, aucune exclusion Dependabot ajoutés.
- [ ] Le corps de la PR porte la rectification R1 (les trois erreurs sont un désalignement) et nomme #2301 + #2139.

---

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2523, avec la note de ce qui les satisfait :

- **AC1** — `mika-common` compile sur opentelemetry 0.33 (les 3 erreurs ci-dessus
  résolues par migration d'API, pas par contournement).
  *Satisfaite par U1 + U2, vérifiée par V1/V2. **Rectification à porter** : les trois
  erreurs se résolvent par l'alignement de la quatrième crate (R1) ; « migration d'API »
  ne s'applique qu'au résidu 0.32→0.33 s'il existe (halte H2). Aucun contournement n'est
  livré : H3 prescrit l'arrêt plutôt qu'un pin.*
- **AC2** — `cargo test -p mika-common` (et la feature `telemetry`) verts sur 0.33.
  *Satisfaite par V5 + V8. **Renforcée** : la suite actuelle n'exerçait pas le chemin armé
  (R5) ; U3 ajoute la couverture qui rend cet AC non vide.*
- **AC3** — Aucune régression fonctionnelle de l'export OTLP/Langfuse (les spans
  `gen_ai.*`, `turn_usage`, etc. continuent d'être émis).
  *Livrée en deux moitiés, et la frontière est dite : **structurelle** (U3 — le layer se
  construit, les neuf clés `gen_ai.*` sont écrites aux trois rails) et **fonctionnelle**
  (sonde S1, geste opérateur avec un Langfuse joignable ; S2 pour la corrélation du
  `trace_id`). Note que `turn_usage` est un événement `tracing` sur la cible
  `mika::otel`, indépendant de l'API otel : il continue d'être émis même télémétrie
  désactivée.*
- **AC4** — Le bump est porté dans #2301 (ou une PR de migration qui la remplace) ;
  #2301 ne merge que CI verte.
  *Satisfaite par la voie « PR de migration », que l'AC autorise explicitement : la
  branche `feat/2523/…` porte les quatre lignes. U7 nomme la disposition de #2301 comme
  geste **opérateur** post-merge — le pilote n'a pas de `gh` authentifié, et #2301 ne doit
  pas être fermé avant le merge sous peine d'être recréé.*
