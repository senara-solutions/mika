# mika#1833 — Le thrash du resolver KG : un interblocage à budget nul, pas un graphe de domaine vide

> Ticket : `senara-solutions/mika#1833`
> Branche : `feat/1833/kg-resolver-thrash-agents-burn-26-48s`
> Classe : substrat de boucle (le défaut fondateur a affamé le dev loop — 22 tickets bloqués `READY`-ungroomed, 0 dispatch)

---

## 1. Rectification du diagnostic — premier livrable

Le ticket pose une hypothèse de cause racine et quatre suivis. **La lecture du
code déplace la cause racine et répond à deux des quatre suivis sans écrire une
ligne de Rust.** Ce déplacement est le premier livrable : sans lui, le correctif
viserait un mécanisme qui n'est pas en cause.

### 1.1 Ce que le ticket affirme

> Le Stage-1 exact-match échoue sur les 50 candidats chaque tick → hypothèse :
> **domain graph (`kg_entities`) vide ou incomplet**. […] `domain_builder.rs`
> échoue peut-être silencieusement au boot.

### 1.2 Ce que le symptôme dit réellement

Symptôme mesuré (2026-07-26, chaque agent, chaque tick) :

```
pending: 1288-1493  resolved_in_tick: 0  duration_ms: 26000-48000
aborted_budget: true  llm_calls: 0
```

Le couple **`aborted_budget: true` avec `llm_calls: 0`** est décisif. Il n'a
qu'une cause possible dans le code :

`crates/mika-agent/src/kg/entity_resolver.rs:400`
```rust
let llm_call_allowed = stats.llm_calls < budget;
```

`aborted_budget` n'est posé que lorsque `resolve_single_entity` rend
`SkippedBudget`, ce qui exige `llm_call_allowed == false`. Avec `llm_calls == 0`,
cela impose `0 < budget` faux, donc **`budget == 0`**.

La re-mesure opérateur du 2026-09-30 le confirme sans ambiguïté et **au
présent** : `resolution_pending_start` porte `budget: 0`. Deux mois après
l'incident, `MIKA_KG_BATCH_BUDGET=0` est **toujours en vigueur sur l'hôte**.

### 1.3 La chaîne, maillon par maillon — et aucun maillon n'est fautif isolément

| # | site | ce qui se passe |
|---|---|---|
| 1 | configuration opérateur | `MIKA_KG_BATCH_BUDGET=0`, posé en croyant désactiver la phase (`.env.example:57` : *« `0` disables the phase entirely »*) |
| 2 | `entity_resolver.rs:678` | `self.llm` vaut `Some` (un modèle de résolution EST configuré) — donc `SkippedNoLlm`, **qui écrirait une ligne de log et draînerait**, n'est jamais atteint |
| 3 | `entity_resolver.rs:653` | Stage-1 trouve un exact match, mais le seuil est `confidence > 0.9` **strict** — à la valeur modale `0.9` la condition est **fausse** et l'entité escalade en Stage-2 |
| 4 | `entity_resolver.rs:683` | `llm_call_allowed = (0 < 0)` ⇒ faux ⇒ `SkippedBudget` |
| 5 | `entity_resolver.rs:415-430` | `SkippedBudget` ⇒ `continue` — **`apply_result` n'est jamais appelé, donc aucune ligne `kg_resolutions_log` n'est écrite** |
| 6 | `entity_resolver.rs:1178` | la sélection est `ORDER BY e.id ASC LIMIT <limit>` — **les mêmes lignes de tête sont re-sélectionnées au tick suivant** |
| 7 | → retour en 3 | indéfiniment, toutes les 30 min, par agent |

C'est un **interblocage déterministe**, et **le graphe de domaine n'a pas besoin
d'être vide pour le produire**. Le maillon 3 est celui que l'intuition rate : le
prompt d'extraction (`subject_extractor.rs:1160-1211`) ne donne **aucune
consigne** sur la valeur de confiance à émettre — seulement le schéma
`<0.0-1.0>` et « Only extract entities and relationships you are confident
about ». `0.9` est la valeur qu'un LLM émet pour une extraction confiante (les
fixtures du module l'attestent : 0.8, 0.85, 0.9). **Un exact match parfait à la
valeur modale escalade donc en Stage-2**, et sous budget nul il est jeté.

`resolved_in_tick: 0` s'explique intégralement par là. L'hypothèse « graphe de
domaine vide » est **non nécessaire** — et elle n'est pas vérifiable depuis le
bac à sable de dispatch (`~/.mika/` n'y monte que `data` et `state` ; aucun
`agents/`, aucun `.env`, aucune base lisible — classe mika#2165). R6 ci-dessous
la rend mesurable une fois pour toutes plutôt que de la trancher à l'aveugle.

### 1.4 Le coût — un second défaut, indépendant du premier

`duration_ms: 26000-48000` n'est **pas** le travail de résolution : sous budget
nul il n'y a aucun appel LLM, et au plus ~100 entités sont sélectionnées
(`effective_budget = 50`, limite `max(50 × 2, 50)`). Le coût vient des requêtes
de **détection du pending**, qui portent toutes la même sous-requête corrélée :

```sql
SELECT cs.extraction_trace_id
  FROM kg_chunk_subjects cs
 WHERE cs.subject_entity_id = e.id
 ORDER BY cs.created_at DESC LIMIT 1
```

L'index disponible est `idx_kg_cs_entity ON kg_chunk_subjects(docs_root_hash,
subject_entity_id)` (`db/migrations.rs:938`). **Sa colonne de tête,
`docs_root_hash`, n'apparaît pas dans le `WHERE` de la sous-requête** : SQLite ne
peut donc pas l'utiliser en *seek*, et retombe sur un parcours de
`kg_chunk_subjects` **par ligne candidate**. Avec 1300-1500 entités sujettes et
plusieurs milliers de lignes de provenance, cela fait des millions de visites de
ligne — trois fois par tick pour un agent mono-corpus (`count_pending`,
`count_pending_for_corpus`, `get_pending_entities_for_corpus`), **et six fois
plus pour mika-arch**, qui déclare six corpora.

Ce second défaut **survit au correctif du premier** : dès la ré-activation du KG
avec un budget non nul, les ticks de 26-48 s reviennent. Il doit donc être dans
le périmètre, sans quoi la ré-activation est impossible.

### 1.5 L'audit de dérive — répondu par lecture, et la réponse inverse la question

Le ticket demande : *« mika-dev/mika-qa devraient avoir `[kg].enabled=false` per
topology #800, mais avaient `true`. Quand/comment leur identity a été
overwritten ? »*

**Rien ne l'a écrasée. Elle n'a jamais été écrite.**

- Le code **déclare** bien `[kg] enabled = false` pour mika-dev
  (`well_known_agents.rs:130`), mika-qa (`:314`) et mika-test (`:1497`), et
  `enabled = true` pour mika-arch (`:480`).
- Mais `CODE_OWNED_IDENTITY_SECTIONS` (`well_known_agents.rs:569-584`) **ne
  contient pas `kg`**, et son doc-comment le dit en toutes lettres : *« Sections
  NOT listed here are preserved verbatim from the on-disk file (operator-owned:
  `name`, `emoji`, `[reflection]`, `[kg]`) »*.
- Et `write_default_if_missing` ne réécrit jamais un `identity.toml` existant.

Donc la déclaration du code est **inerte pour tout agent déjà provisionné**.
mika-dev et mika-qa ont été provisionnés avant que #800 ne fasse de
`enabled = false` la valeur déclarée, et **aucun chemin n'a rétro-appliqué la
décision depuis**. C'est très exactement la classe que le dépôt a déjà dû nommer
pour mika#2327 → mika#2330 (`[context.history]` rendu code-owned, mergé, et
**inerte en production** pour la même raison), une section plus loin.

Corollaire utile : **le palliatif du 2026-07-26 survit aux redémarrages
précisément parce que `[kg]` est operator-owned.** C'est une bonne nouvelle pour
la sûreté et une mauvaise pour la visibilité — il ne se lèvera jamais tout seul.

### 1.6 Pourquoi `mika` tique encore, et les autres non

`DEFAULT_IDENTITY` (`crates/mika-common/src/home.rs:677-711`) porte son bloc
`[kg]` **en commentaire**, donc `enabled` prend son défaut `true`. L'agent `mika`
n'a jamais reçu le palliatif ou l'a perdu à un re-provisionnement ; les quatre
autres portent le `enabled = false` écrit à la main. La mesure du 2026-09-30 est
donc cohérente : un seul agent tique, et il tique **sous budget nul** — avec
`pending: 0` aujourd'hui, mais l'interblocage est armé et n'attend qu'un corpus
qui grossit.

### 1.7 Ce qui masque le défaut : **deux** palliatifs, dont un seul est documenté

Le ticket n'en nomme qu'un (`[kg] enabled = false` sur cinq identités). Le
second — `MIKA_KG_BATCH_BUDGET=0`, fleet-wide — n'est mentionné nulle part et
c'est **lui qui rend l'interblocage possible**. Un opérateur qui lèverait le
premier sans lever le second rejouerait l'incident du 2026-07-26 à l'identique.
Cette phrase est le cœur de ce plan.

---

## 2. Exigences

### R1 — Rectifier le diagnostic, par écrit

La §1 ci-dessus est portée dans le corps de PR et dans
`crates/mika-agent/CLAUDE.md`. Le ticket sera clos en citant le déplacement, pas
en confirmant son hypothèse.

**Aucune ligne de code.**

### R2 — `budget == 0` désactive réellement la phase, et le dit

`MIKA_KG_BATCH_BUDGET=0` devient ce que la documentation prétend déjà, plutôt que
de voir la documentation corrigée pour décrire l'interblocage.

- `SubjectEntityResolver::resolve_pending(0)` et
  `resolve_doc_entities(_, _, 0)` rendent `ResolutionStats::default()`
  **immédiatement**, avant `get_pending_entities` et avant `count_pending`.
- `resolver_tick::tick_extraction` et `tick_resolution` court-circuitent
  **avant** leurs requêtes de comptage (`count_pending_docs`,
  `count_pending`) — c'est là que vit le coût de la §1.4.
- Le tick continue d'émettre `kg_resolver_tick.complete`, avec un champ
  `skipped_reason: "zero_budget"` et `pending_before: null`. **`null`, jamais
  `0`** (motif mika#2331) : le comptage n'a pas eu lieu, il n'a pas rendu zéro.
  Conserver la ligne préserve le **contrôle positif** — sans elle, « le tick
  tourne et ne fait rien » et « le tick ne tourne pas » rendraient des octets
  identiques (classe mika#2205).

**Ce que ça retire, nommé.** Le comportement « les exact matches Stage-1 passent
gratuitement même sous budget nul », ajouté délibérément par la revue de #757
(finding P1). Deux mesures le justifient : *(a)* ce comportement n'est **déjà
pas** délivré pour la population de confiance ≤ 0.9, qui est la population
modale — `resolved_in_tick: 0` en est la mesure directe ; *(b)* l'intention
« exact-match seulement, pas de LLM » a **déjà son levier propre et
terminant** : ne pas configurer de modèle de résolution. `self.llm == None` ⇒
les exact matches à confiance > 0.9 résolvent, tout le reste draine en
`skipped_no_llm`, et la file se vide. Deux orthographes pour une intention, dont
une seule termine : R2 en retire une.

**Direction du fail-safe.** Un opérateur qui avait posé `0` pour obtenir le mode
exact-match-seul perd ce comportement. Le changement est donc **dit** (R3 nomme
le bon levier) plutôt que silencieux.

### R3 — La documentation devient vraie

Trois sites portent aujourd'hui des affirmations contradictoires entre elles :

| site | état actuel |
|---|---|
| `.env.example:57` | *« `0` disables the phase entirely »* — faux pour la résolution avant R2, **vrai après** |
| `crates/mika-agent/CLAUDE.md:2813` | *« even `budget=0` lets exact matches resolve »* — décrit l'interblocage comme une fonctionnalité |
| `CLAUDE.md` racine, entrée `MIKA_KG_BATCH_BUDGET` | *« `0` disables the phase entirely »* |

Après R2, les deux premiers énoncés cessent de se contredire. La ligne de
`crates/mika-agent/CLAUDE.md` est remplacée par l'énoncé vrai **plus le coût
nommé** et **plus le renvoi au bon levier** pour le mode exact-match-seul.

### R4 — L'index qui sert la détection du pending

Migration **v53 → v54**, additive, `CREATE INDEX IF NOT EXISTS`, aucune
reconstruction de table :

```sql
CREATE INDEX IF NOT EXISTS idx_kg_cs_entity_recent
    ON kg_chunk_subjects(subject_entity_id, created_at DESC);
```

Sert exactement la sous-requête corrélée de la §1.4, sur ses deux colonnes et
dans son ordre. **Coût nommé** : un index de plus à maintenir à l'insertion dans
`kg_chunk_subjects` — donc une amplification d'écriture sur le chemin
d'extraction, borné par le budget d'extraction.

L'index existant `idx_kg_cs_entity(docs_root_hash, subject_entity_id)` **n'est
pas retiré** : il sert les requêtes scopées par corpus, qui sont une autre
population. Retirer un index pour en ajouter un autre serait un arbitrage que
rien ici ne mesure.

### R5 — Interrupteur d'arrêt à chaud pour le tick KG

Le suivi 3 du ticket demande `MIKA_KG_RESOLVER_DISABLED=1`. **La variable
d'environnement est refusée**, et la raison est déjà écrite dans le dépôt
(`crates/mika-agent/src/auto_pull_stop.rs`, doc de module, mika#2329) :
`load_dotenv` est appelé **une fois** au démarrage, et l'environnement d'un
process Linux vivant n'est pas mutable de l'extérieur — donc éditer `~/.mika/.env`
ne change rien à ce que `std::env::var` renverra, **même relu à chaque tick**. La
maison fige délibérément ses `MIKA_*` ; en rendre une relue à chaud créerait une
exception invisible.

Le mécanisme existant est livré **paramétré par nom de scan** et refuse de
s'étendre sans besoin mesuré. **Un scan qui a brûlé 26-48 s par tick sur cinq
agents et affamé le dev loop est ce besoin**, et il satisfait le critère de
mika#2420 (*une décision distincte mérite un fichier distinct*) : arrêter
l'ingestion KG n'est ni arrêter le feeder de dispatch, ni arrêter le faucheur de
worktrees.

- `pub const KG_TICK_SCAN: &str = "kg-tick";` dans `auto_pull_stop.rs`, à côté de
  ses deux frères, avec son doc-comment portant la raison ci-dessus.
- Ajouté à la liste de littéraux de
  `mika2329_le_chemin_du_fichier_sentinelle_a_un_seul_lecteur` — **obligatoire** :
  un scan né hors de la garde est un scan sur lequel la leçon `grooming_marker`
  se rejoue.
- Lu **en tête de `tick_body`**, avant toute requête et avant la résolution du
  moindre modèle. Geste opérateur : `touch ~/.mika/state/kg-tick-stop` /
  `rm` — effectif au tick suivant (≤ 30 min).
- Exige de passer `global_home` à `spawn_resolver_tick_task`. Un paramètre, pas
  une `Option` : un home absent serait un retour silencieux au comportement
  d'aujourd'hui.
- **Aucune row n'est touchée** : le tick continue de tourner, c'est son corps qui
  rend la main. La réversibilité est l'absence de machinerie.

### R6 — Le recensement du graphe de domaine, au boot

`domain_rebuild_complete` (`server/mod.rs:1132`) gagne `entities_total` et
`per_type` (une carte JSON `type → compte`). Le suivi 1 du ticket — *« vérifier
`kg_entities` row count post-boot »* — devient un `grep` au lieu d'une session
SQL, et **l'hypothèse de la §1.3 cesse d'être une hypothèse**.

Émis sur **toutes** les branches, y compris saine (doctrine mika#2293), donc son
**absence** est elle-même de l'information : un journal qui tourne sans elle dit
que le binaire servi est antérieur au correctif (classe mika#2340).

Plus un `domain_graph_empty` (WARN) quand `entities_total == 0` après un rebuild
**réussi** — l'état exact que le ticket a supposé. **Régime attendu : zéro
ligne.** Sans cet événement, l'état que tout ce ticket a suspecté resterait
inférable et jamais observé.

### R7 — L'audit de dérive et le geste de ré-activation, documentés et non exécutés

- `crates/mika-agent/CLAUDE.md` porte la §1.5 : `[kg]` est operator-owned **par
  décision**, c'est pourquoi la déclaration du code n'a jamais atteint mika-dev
  et mika-qa, et c'est aussi pourquoi le palliatif survit aux redémarrages.
- `CLAUDE.md` racine porte le geste de ré-activation, **conditionné aux sondes**
  de la §6.
- **Aucune ligne de code.** Ajouter `kg.enabled` à
  `CODE_OWNED_IDENTITY_SECTIONS` écraserait le palliatif au prochain démarrage et
  **ré-activerait mika-arch** (le code y déclare `true`) : c'est un arbitrage de
  sûreté, pas un effet de bord — voir §7.

**Le périmètre réel de la ré-activation est plus étroit que le ticket ne le
suggère.** Par topologie #800, mika-dev et mika-qa **doivent rester désactivés**,
et le code les déclare déjà ainsi. Le seul agent où le palliatif contredit la
topologie déclarée est **mika-arch** (code : `enabled = true`). `mika` est déjà
actif. `mikamodel-probe` n'est pas un agent bien connu du code courant
(`WELL_KNOWN_AGENTS` en compte quatre : mika-dev, mika-test, mika-qa,
mika-arch) — c'est un agent client dont l'identité est entièrement à
l'opérateur.

---

## 3. Décisions, et les alternatives refusées

### D1 — Le correctif porte sur le budget, pas sur le seuil de confiance

L'autre remède concevable pour le maillon 3 est d'accepter un exact match Stage-1
**à n'importe quelle confiance** quand le Stage-2 est indisponible. Il est
**refusé ici**, et la raison n'est pas la difficulté :

- C'est un changement de **sémantique de résolution** — écrire `matched_exact` à
  confiance 0.85 là où le LLM aurait pu dire `no_match`. Ça crée des arêtes de
  résolution que personne n'a validées.
- Il ne ferme **pas** l'interblocage à lui seul : d'autres entités de la tête de
  file n'ont aucun exact match du tout et resteraient `SkippedBudget`.
- L'interblocage est **spécifique à `budget == 0`**. Avec un budget non nul, les
  N premières entités atteignent le Stage-2, sont journalisées, et la tête de
  file avance. R2 suffit donc, et c'est le plus petit correctif qui ferme la
  classe entière.

Le seuil `> 0.9` reste néanmoins un gaspillage réel — il envoie au LLM des exact
matches parfaits, soit ~1300 appels sur un premier drain. **Suivi nommé**, dont
la précondition est la distribution des confiances que le frère de R6 peut
produire.

### D2 — Ne pas faire tourner la sélection

Changer `ORDER BY e.id ASC` pour que la tête ne soit pas la même à chaque tick
ferait **progresser** le thrash au lieu de l'arrêter : rien n'étant écrit, la
file ne draînerait toujours pas, la futilité serait simplement répartie. **Refusé
— ça masque l'interblocage.**

### D3 — Ne pas écrire de ligne `kg_resolutions_log` pour `SkippedBudget`

Un outcome `deferred_budget` viderait la file — en **marquant résolu ce qui ne
l'a pas été**. C'est le défaut « la colonne devient pleine et reste muette » que
mika#2133 a dû nommer, transposé. **Refusé.**

### D4 — Ne pas réordonner le tick

Déplacer la phase de résolution avant l'extraction, ou l'inverse, ne touche
aucun maillon de la chaîne. Hors sujet.

### D5 — Le nouvel index s'ajoute, l'ancien reste

Voir R4. Aucune mesure ne dit que `idx_kg_cs_entity` est mort ; le retirer serait
un arbitrage non mesuré dans un ticket qui en mesure deux autres.

---

## 4. Surfaces opérateur

### Journal (`$MIKA_SPIRIT_LOG_FILE`)

```bash
# 1. Quel budget cet agent fait-il TOURNER, et la phase est-elle armée ?
grep kg_budget_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{agent_id, budget, budget_source, resolution_armed, extraction_armed, resolution_model_configured}'

# 2. Le graphe de domaine, recensé au boot
grep domain_rebuild_complete "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{entities_total, per_type, added, updated, removed}'

# 3. CONTRÔLE NÉGATIF — le graphe est-il vide ? (régime attendu : VIDE)
grep domain_graph_empty "$MIKA_SPIRIT_LOG_FILE"

# 4. CONTRÔLE POSITIF — le tick tourne-t-il seulement ?
grep kg_resolver_tick.complete "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{agent_id, skipped_reason, pending_before, pending_after, duration_ms, aborted_budget}'

# 5. La sentinelle est-elle armée ?
grep -E 'kg_tick_stop_(armed|lifted)' "$MIKA_SPIRIT_LOG_FILE"
```

### Base

```sql
-- Les transitions de la sentinelle, datées
SELECT created_at, after_value FROM audit_events
 WHERE tool_name = 'kg_tick_stop' ORDER BY created_at DESC;
```

### Table de lecture

| surface | niveau | régime attendu | lecture |
|---|---|---|---|
| `kg_budget_resolved` | INFO | **1 par agent par démarrage** | `budget: 0` ⇒ la phase est désarmée, par configuration. `budget_source: "config"` ⇒ quelqu'un l'a posé ; `"default"` ⇒ c'est le défaut 500 |
| `domain_rebuild_complete` avec `entities_total` | INFO | non nul | le recensement que le suivi 1 réclamait ; son **absence** = binaire antérieur au correctif (classe mika#2340) |
| `domain_graph_empty` | WARN | **vide** | l'état que le ticket a supposé, enfin observable |
| `kg_resolver_tick.complete` avec `skipped_reason: "zero_budget"` | INFO | non vide **si** budget nul | le contrôle positif : le tick tourne et se sait désarmé |
| `kg_resolver_tick.complete` avec `duration_ms > 20000` | INFO | **vide** après R4 | le symptôme du coût, s'il revient |
| `kg_tick_stop_armed` | INFO | une ligne par tick court-circuité pendant un STOP | pour un interrupteur, la vivacité **est** l'information (mika#2329) |

**`pending_before: null` n'est pas `pending_before: 0`.** Sous budget nul le
comptage n'a pas lieu ; l'annoncer à zéro serait un mensonge lisible
(mika#2331).

---

## 5. Contrat de vérification

### Tests comportementaux

| # | test | ce qu'il atteste | ce qu'il vaut sans son contrôle |
|---|---|---|---|
| V1 | `mika1833_budget_zero_resolution_is_a_declared_noop` | `resolve_pending(0)` rend `default()` sans avoir émis la moindre requête de comptage | rien — voir V2 |
| V2 | **contrôle négatif** `mika1833_a_nonzero_budget_still_resolves` | avec budget 5 et un exact match à confiance 0.95, l'entité résout `matched_exact` | c'est lui qui distingue « R2 décide » de « R2 casse la résolution » |
| V3 | `mika1833_the_founding_livelock_no_longer_recurs` | rejeu du défaut : budget 0, LLM configuré, 3 sujets dont un exact match à **confiance 0.9** ⇒ **zéro** entité est laissée dans un état où le tick suivant la re-sélectionnerait pour rien | le test qui échouerait avant le correctif |
| V4 | `mika1833_zero_budget_tick_still_emits_its_completion_line` | le tick désarmé émet `kg_resolver_tick.complete` avec `skipped_reason` et `pending_before: null` | sans lui, R2 rendrait le tick muet et le contrôle positif serait perdu |
| V5 | `mika1833_the_stop_sentinel_short_circuits_the_tick` | sentinelle posée ⇒ `tick_body` rend la main sans requête ; retirée ⇒ le tick reprend | — |
| V6 | `mika1833_index_serves_the_pending_subquery` | `EXPLAIN QUERY PLAN` de la sous-requête corrélée nomme `idx_kg_cs_entity_recent` en `SEARCH`, pas en `SCAN` | le seul test capable de prouver R4 ; une assertion sur l'existence de l'index ne prouve pas qu'il est **utilisé** |
| V7 | `mika1833_migration_v53_to_v54_is_idempotent` | la migration rejouée deux fois n'échoue pas et ne duplique rien | — |
| V8 | `mika1833_domain_census_counts_what_the_rebuild_wrote` | `entities_total` et `per_type` coïncident avec un `SELECT COUNT(*) GROUP BY type` après rebuild | — |
| V9 | **contrôle négatif** `mika1833_a_populated_graph_does_not_emit_the_empty_warning` | `domain_graph_empty` ne tire pas sur un graphe peuplé | sans lui, « la garde détecte le vide » est indistinguable de « la garde crie toujours » |

**Chaque terme est à voir rouge par mutation, un à la fois.** Une conjonction de
termes fail-safe ne se prouve pas en les neutralisant tous ensemble — leçon
mika#2277, payée deux fois dans ce dépôt.

### Gardes structurelles (scans de source, allowlists livrées **vides**)

| # | garde | la classe qu'aucun test comportemental ne peut voir |
|---|---|---|
| U1 | `mika1833_the_zero_budget_predicate_has_a_single_reader` | un second site testant `budget == 0` pourrait diverger du premier en silence — classe `grooming_marker` (mika#2158), déjà payée deux fois dans ce crate |
| U2 | `mika2329_…_a_un_seul_lecteur` **étendue** à `KG_TICK_SCAN` | un scan né hors de la garde naît sans protection |
| U3 | `mika1833_the_census_event_names_have_a_single_writer` | SOLE WRITER de `domain_graph_empty` et `kg_budget_resolved` : c'est ce qui rend les comptes de la §4 exacts plutôt que discutables entre deux sites |

Chaque scan porte son **assertion d'anti-vacuité** (il rougit si le nom qu'il
cherche n'est écrit nulle part) et son **contrôle de bonne foi** (il rougit sur
un second site injecté) — *un scan visant un nom mort se lit exactement comme un
arbre propre* (classe mika#2103 / mika#2205).

---

## 6. Sondes post-déploiement, et leurs six haltes

> **Préalable, non négociable.** Ces mesures décrivent le **binaire servi**. Après
> `make deploy`, vérifier que le `mika-spirit` qui tourne porte le correctif —
> la ligne `kg_budget_resolved` doit exister — **avant toute conclusion** (classe
> mika#2340). Et rien de ceci n'est exécutable depuis le bac à sable de
> dispatch : `~/.mika/` n'y monte ni `agents/`, ni `.env`, ni la base. **Ce sont
> des gestes d'opérateur sur l'hôte.**

**S0 — établir la configuration AVANT de conclure quoi que ce soit.**
Lire `kg_budget_resolved`. Un `budget: 0` avec `budget_source: "config"` confirme
la §1.2 et **la cause est la configuration**, pas le graphe de domaine.
*Halte 1 —* si `budget` vaut 500 en production, la §1.2 est fausse pour l'hôte
courant et **tout le diagnostic est à refaire** : lire alors `domain_graph_empty`
(sonde S2) avant de toucher au correctif. Ne pas déployer R2 en croyant fermer un
défaut qui n'est pas celui-là.

**S1 — le coût disparaît (48 h).** Aucun `kg_resolver_tick.complete` avec
`duration_ms > 20000`. Sous budget nul le tick doit rendre la main en
millisecondes ; avec un budget non nul, R4 doit ramener la détection du pending
sous la seconde.
*Halte 2 —* la durée reste haute alors que R4 est déployé : lancer `EXPLAIN QUERY
PLAN` sur la sous-requête et lire si `idx_kg_cs_entity_recent` est retenu.
**Ne pas augmenter le budget par réflexe** — ça déplace le coût vers les appels
LLM sans toucher les requêtes.

**S2 — l'hypothèse du ticket est tranchée (premier démarrage).**
`domain_rebuild_complete` porte `entities_total` et `per_type`. Deux issues, et
**les deux sont des résultats** : un compte non nul **réfute** l'hypothèse « graphe
vide » et clôt le suivi 1 ; un compte nul la **confirme** et ouvre un ticket sur
`domain_builder` — avec une mesure plutôt qu'une intuition.
*Halte 3 —* `domain_graph_empty` non vide : c'est un défaut **amont** de tout ce
plan. Ne pas ré-activer le KG sur mika-arch avant de l'avoir fermé.

**S3 — la ré-activation de mika-arch, et elle est séquencée.**
**Uniquement après S0, S1 et S2 vertes.** Poser `MIKA_KG_BATCH_BUDGET=500` (ou
retirer la variable), retirer `enabled = false` de
`~/.mika/agents/mika-arch/identity.toml`, redémarrer. Attendu sur 24 h :
`pending_after` décroît d'un tick au suivant, `aborted_budget` reste `false` ou
devient `true` **avec** un `llm_calls` non nul, et `duration_ms` reste sous la
seconde hors appels LLM.
*Halte 4 —* `resolved_in_tick: 0` réapparaît avec `llm_calls: 0` : **le défaut
fondateur est revenu**. Reposer le palliatif — `touch
~/.mika/state/kg-tick-stop`, effectif en ≤ 30 min, sans redémarrage, ce qui est
très exactement ce que R5 achète — **puis** diagnostiquer.
*Halte 5 —* le coût LLM explose sur le premier drain (~1300 appels) : ce n'est
pas une panne, c'est le seuil `> 0.9` de la §1.3 / D1. Le mesurer et ouvrir le
suivi ; ne pas remettre le budget à zéro, ce qui rouvrirait l'interblocage.

**S4 — contrôle négatif de bruit (7 jours).** Aucun `domain_graph_empty`, aucun
`kg_tick_stop_armed` hors STOP posé.
*Halte 6 —* `kg_tick_stop_armed` apparaît sans fichier posé : le lecteur regarde
ailleurs que là où l'opérateur écrit. Comparer le chemin rapporté avec le chemin
réel et vérifier `MIKA_HOME` **avant** de toucher au prédicat — le fail-open de
`Path::exists` ne peut produire que l'inverse (un STOP non vu), donc une ligne de
trop est une erreur de **chemin**, jamais de prédicat.

**Halte transverse — les sondes muettes.** Zéro ligne de tick **et** zéro ligne
de recensement ne prouve rien : il faut qu'un démarrage ait eu lieu depuis le
déploiement et qu'un agent porte le KG actif. *Une garde que personne n'a exercée
se lit exactement comme une garde qui marche* (mika#2205).

---

## 7. Fire-Disposition

Ce plan livre des détecteurs : les trois scans de source U1–U3 (dont le chemin de
succès est « aucune violation trouvée ») et le WARN runtime `domain_graph_empty`.
La disposition retenue est **(a) exception nommée en allowlist — mais avec les
allowlists livrées VIDES**, ce qui est la forme la plus stricte de (a).

### Disposition par détecteur

| détecteur | violations existantes | disposition |
|---|---|---|
| **U1** — lecteur unique du prédicat `budget == 0` | **zéro** : le prédicat n'existe pas encore, il naît avec ce plan | allowlist **livrée vide et épinglée vide** par un test frère. Quand U1 tire, **on retire le second lecteur ; on n'ajoute pas de ligne** (doctrine mika#2201) |
| **U2** — lecteur unique du chemin de sentinelle (étendue) | **zéro** : la garde existante est verte et `KG_TICK_SCAN` naît dedans, pas à côté | aucune allowlist n'est introduite ; la garde existante gagne un littéral |
| **U3** — SOLE WRITER de `domain_graph_empty` et `kg_budget_resolved` | **zéro** : les deux noms naissent avec ce plan | allowlist **livrée vide et épinglée vide**. Résolution quand elle tire : retirer le second écrivain |
| **`domain_graph_empty`** (WARN runtime) | **inconnu, et c'est le sujet** | **Livré armé.** Ce n'est pas une garde de merge : elle ne bloque aucune CI et ne refuse aucune opération. Son régime attendu est zéro, et si elle tire au premier démarrage c'est le **résultat** que la sonde S2 existe pour obtenir — pas une panne du détecteur |

### Pourquoi aucune allowlist n'est peuplée

Aucun des trois scans ne peut avoir de violation préexistante : ils portent tous
sur des noms et des prédicats **que ce plan crée**. Une allowlist née peuplée
serait une allowlist née fausse ; une allowlist née vide mais *existante* serait
un tiroir où déposer la prochaine infraction (mika#2323). Chacune est donc livrée
vide **et** un test frère refuse qu'elle cesse de l'être.

### Pourquoi aucun détecteur n'est livré désarmé

L'option (b) — `#[ignore]` plus un suivi — est refusée sur le précédent
mika#2272 : mika#2249 avait livré un détecteur derrière une condition d'armement
qui s'est révélée **insatisfiable plutôt qu'insatisfaite**, parce que la
population qu'il comptait était vide par construction. Ici les trois scans ont
une population non vide **par construction** (le code qu'ils gardent est écrit
dans la même PR), donc leur vert au merge est une mesure et non une absence de
mesure. Les assertions d'anti-vacuité de la §5 sont ce qui le garantit.

### Pourquoi pas (c), halte-et-remontée

Aucune décision de cadrage ne manque : le périmètre des trois scans est décidé
par ce plan, et les arbitrages réellement ouverts (le seuil de confiance,
`CODE_OWNED_IDENTITY_SECTIONS`, la ré-activation) sont **hors périmètre et
nommés** en §9, avec leurs préconditions.

---

## 8. Definition of Done

- [ ] `resolve_pending(0)` et `resolve_doc_entities(_, _, 0)` court-circuitent
      avant toute requête, et le tick court-circuite ses deux phases avant leurs
      requêtes de comptage.
- [ ] `kg_resolver_tick.complete` reste émis sous budget nul, avec
      `skipped_reason: "zero_budget"` et `pending_before: null`.
- [ ] `kg_budget_resolved` est émis une fois par agent au démarrage, sur toutes
      les branches, avec la provenance du budget.
- [ ] Migration v53 → v54 : `idx_kg_cs_entity_recent`, additive, idempotente.
- [ ] `KG_TICK_SCAN` déclaré, lu en tête de `tick_body`, couvert par la garde de
      lecteur unique, avec son événement de transition et sa ligne d'audit.
- [ ] `domain_rebuild_complete` porte `entities_total` et `per_type` ;
      `domain_graph_empty` existe et son SOLE WRITER est épinglé.
- [ ] V1–V9 verts, chacun **vu rouge par mutation** de son propre terme.
- [ ] U1–U3 verts, allowlists vides, anti-vacuité et contrôle de bonne foi en
      place.
- [ ] `.env.example`, `crates/mika-agent/CLAUDE.md` et le `CLAUDE.md` racine
      portent l'énoncé vrai, le coût nommé et le renvoi au bon levier pour le
      mode exact-match-seul.
- [ ] `crates/mika-agent/CLAUDE.md` porte la réponse de la §1.5 (l'identité n'a
      jamais été écrasée, elle n'a jamais été écrite) et le `CLAUDE.md` racine le
      geste de ré-activation avec ses préconditions.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` verts.
- [ ] Le corps de PR porte la rectification de la §1 et **n'affirme aucune sonde
      non exécutée** — les six haltes de la §6 sont des gestes d'opérateur sur
      l'hôte, pas des résultats de cette PR.

---

## 9. Acceptance criteria

1. **Le thrash fondateur ne peut plus se produire.** Sous `MIKA_KG_BATCH_BUDGET=0`
   avec un modèle de résolution configuré, aucun tick ne re-sélectionne
   indéfiniment la même tête de file : la phase est un no-op déclaré, et V3
   l'atteste sur la forme mesurée (exact match à confiance 0.9).
2. **`MIKA_KG_BATCH_BUDGET=0` fait ce que la documentation dit.** Les trois sites
   documentaires cessent de se contredire, et l'énoncé qui subsiste est vrai
   après R2. Le levier du mode exact-match-seul est nommé au même endroit.
3. **Le coût de détection du pending est ramené à un ordre de grandeur
   utilisable.** V6 atteste par `EXPLAIN QUERY PLAN` que la sous-requête corrélée
   est servie par un index en `SEARCH` et non en `SCAN`.
4. **Le tick KG est arrêtable à chaud, sans redémarrage et sans édition
   d'identité.** `touch ~/.mika/state/kg-tick-stop` court-circuite le tick au
   suivant ; `rm` le reprend. Aucune row n'est touchée dans un sens ni dans
   l'autre.
5. **L'hypothèse « graphe de domaine vide » est tranchable par un `grep`.**
   `domain_rebuild_complete` porte le recensement, et son absence signifie
   « binaire antérieur au correctif », jamais « graphe sain ».
6. **L'audit de dérive est répondu et la réponse est non récurrente.** La
   documentation nomme que `[kg]` est operator-owned par décision, que c'est
   pourquoi la déclaration du code n'a jamais atteint les agents déjà
   provisionnés, et que le palliatif survit aux redémarrages pour la même raison.
7. **La ré-activation est décrite, séquencée et non exécutée.** Le geste et ses
   préconditions sont écrits ; aucune ligne de cette PR ne ré-active le KG sur un
   agent, parce que tant que les sondes ne sont pas vertes, le palliatif est ce
   qui protège la flotte.
8. **Aucune régression du chemin nominal.** Avec un budget non nul, la résolution
   se comporte exactement comme avant : V2 est le contrôle négatif qui distingue
   « R2 décide » de « R2 casse la résolution ».

---

## 10. Ce que ce travail n'achète PAS

- **Il ne résout aucune entité de plus par appel LLM.** Le seuil `> 0.9` continue
  d'envoyer au Stage-2 des exact matches parfaits ; c'est un gaspillage réel,
  mesuré, et **hors périmètre** (D1).
- **Il ne rattrape pas l'incident du 2026-07-26.** Rien ne rétro-écrit une ligne
  décrivant un drain qui n'a pas eu lieu. La sonde est la **prochaine** occurrence.
- **Il ne ré-active le KG nulle part.** C'est un geste d'opérateur, gardé derrière
  S0-S2.
- **Il ne rend pas le champ surveillé, seulement lisible.** Les seuls instruments
  neufs sont les greps et la requête de la §4, et **leur silence ne prouve rien
  tant que personne ne les exécute** — sur un tick de 30 minutes, l'absence de
  ligne peut simplement vouloir dire qu'aucun agent n'a le KG actif.
- **Il ne borne pas le coût LLM du premier drain.** À la ré-activation, mika-arch
  présentera un backlog dont le drain coûtera ce qu'il coûte, borné par
  `MIKA_KG_BATCH_BUDGET` par tick. C'est la halte 5.

---

## 11. Hors périmètre, délibérément

| sujet | pourquoi dehors | précondition pour l'ouvrir |
|---|---|---|
| Seuil `> 0.9` → `>= 0.9`, ou acceptation d'un exact match à basse confiance sous Stage-2 indisponible | changement de **sémantique de résolution** : il crée des arêtes que personne n'a validées, et il ne ferme pas l'interblocage à lui seul (D1) | la distribution des confiances d'extraction, qu'un frère de R6 peut produire |
| `LOWER(entity_key) = LOWER(?)` dans `try_exact_match` — un parcours complet de `kg_entities` par entité, l'index UNIQUE sur `entity_key` ne pouvant servir un prédicat `LOWER()` | réel, mais borné par la taille de `kg_entities` (des centaines de lignes) ; le remède est une colonne générée ou une normalisation à l'écriture, soit un changement de schéma sur la table la plus centrale du KG | une mesure montrant que ce parcours pèse dans `duration_ms` une fois R4 déployé |
| Ajouter `kg.enabled` à `CODE_OWNED_IDENTITY_SECTIONS` | écraserait le palliatif au prochain démarrage et **ré-activerait mika-arch** sans que personne ne l'ait demandé : un arbitrage de sûreté déguisé en cohérence | les sondes S0-S3 vertes après une ré-activation manuelle de mika-arch |
| Ré-activer le KG sur mika-dev / mika-qa | **contraire à la topologie #800**, que le code déclare déjà (`enabled = false` pour les deux). Le ticket demande « la ré-activation des agents coupés » ; le périmètre réel est mika-arch seul (§1.6, §R7) | une décision produit revenant sur #800 |
| La cause d'un éventuel `domain_rebuild` en échec | ce plan le rend **observable** (R6) ; le réparer suppose d'abord de l'observer | `domain_graph_empty` non vide, ou `entities_total == 0` |
| Interrupteur à chaud pour `wip_rescue` et `qa_review_reconcile` | même défaut boot-time, mais leur arrêt ne détruit rien et ne brûle rien : livrer des gestes que personne n'a demandés reste du YAGNI (raison déjà écrite dans `auto_pull_stop.rs`) | une mesure montrant un coût ou un dommage pendant un incident |
| Le coût d'extraction (`count_pending_docs` par corpus et par tick) | R4 sert la sous-requête de la **résolution** ; l'extraction a ses propres requêtes et sa propre population | une mesure de `kg_extraction_tick.complete` montrant une durée comparable |
