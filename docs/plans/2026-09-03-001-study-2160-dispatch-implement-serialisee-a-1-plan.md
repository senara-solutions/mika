# Plan : rendre le plafond de concurrence de la classe `implement` choisissable, et chiffrer ce qui le borne (mika#2160)

**Ticket :** mika issue#2160 — `study(dispatch): la classe `implement` est sérialisée à 1 — quatre heures de pilote gèlent toute la boucle, et aucun réglage n'existe`
**Labels :** `enhancement`, `p1-important`, `operator-gated`
**Type :** study (étude à livrable code — le code ouvre une porte, il ne la franchit pas)
**Palier de priorité :** Tier 2 — *ralentit la boucle*. Rien n'est cassé : la boucle produit. Elle produit deux à trois tickets là où dix sont visés.
**Gate :** OPERATOR-GATED. Pas de label `ready` sans le GO explicite de Vincent.

---

## Problème

Une seule implémentation peut être en vol à la fois pour toute la plateforme. Mesuré le 2026-09-03 : le pilote de mika#2151 démarre à 19:30 et tourne encore à 23:30 ; les promotions `ready` de mika#2140 (22:20) et mika#2151 (22:51) sont refusées avec `global_dispatch_active`. Une nuit de neuf heures rend deux à trois tickets. Aucun réglage n'existe : `crates/mika-common/src/config.rs` ne contient pas une occurrence de `concurren` (vérifié — `grep -ic` rend `0`).

Ce plan **n'enlève pas** la sérialisation. Il fait trois choses : il nomme ce qui la tient, il chiffre ce qui la borne, et il rend le plafond choisissable en le laissant à **1** par défaut.

---

## Ce que le code dit — et où le ticket est incomplet

Le corps du ticket décrit **deux** mécanismes. Le code en porte **trois**, et le troisième est celui qui décide réellement si N>1 est exploitable.

### 1. Le bail — `dispatch_slot_leases`

`crates/mika-agent/src/db.rs:8019` (`try_acquire_dispatch_slot`). Un `INSERT … ON CONFLICT … WHERE expired OR same-holder` dans une transaction IMMEDIATE, sur une table dont la **clé primaire est `(agent_id, dispatch_class)`**. TTL par défaut 120 s (`db.rs:179`), surchargeable par `MIKA_DISPATCH_SLOT_LEASE_TTL_SECS` (`db.rs:184-190`).

Le ticket a raison de dire que régler le TTL ne débloque rien. Il faut ajouter ceci, qui n'est pas dans le corps : **cette clé primaire est elle-même un plafond dur à 1.** Une classe ne peut pas détenir deux baux vivants, quel que soit le TTL. Un plafond paramétrable qui ne toucherait que le prédicat de la §2 laisserait le bail sérialiser à 1 — le réglage existerait et n'aurait aucun effet observable.

### 2. Le verrou effectif — la ligne de rappel enfant

`crates/mika-agent/src/db.rs:7883` (`has_active_callback_tasks_excluding`), appelé depuis `crates/mika-agent/src/skills/executor.rs:1388`. Un `SELECT … LIMIT 1` : *existe-t-il une ligne de rappel `pending`/`in_progress`, même classe, autre parent, hors enveloppes `:deferred`*. C'est un prédicat **booléen d'existence** — la forme même d'un plafond à 1. Le message de refus (`executor.rs:1422-1436`) dit la règle en toutes lettres.

### 3. Le verrou de tour d'agent — `agent_lock` *(non nommé par le ticket)*

`crates/mika-agent/src/server/mod.rs:508` construit un `Arc<tokio::sync::Mutex<()>>` par agent. Le rappel `canUseTool` du pilote passe par le relais `.claude/claude-pilot.json` = `{"command":"mika","args":["--agent","mika-dev","ask"]}` ; `mika ask` frappe `/a2a/{agent}` (`crates/mika-cli/src/commands/ask.rs:332`) ; et `crates/mika-agent/src/server/a2a.rs:226` et `:360` prennent ce mutex en **`try_lock_owned()`** — pas d'attente, pas de file : sur collision, la réponse est `JsonRpcError(INTERNAL_ERROR, "Agent is busy")`, immédiatement.

C'est la contrainte qui gouverne AC1. Le chemin `/message` a reçu une file bornée (mika#1870) ; le chemin A2A, non. Deux pilotes concurrents dont les escalades de permission se recouvrent verraient l'un des deux se faire refuser sa demande de permission — pas différer, refuser. Le `CLAUDE.md` § Cross-Repo Development le disait en prose (*« tool permissions and build callbacks assume a single active session »*) ; `a2a.rs:226` est l'endroit où c'est vrai dans le code.

---

## Ce que le plan livre

### Phase 1 — Inventaire des ressources partagées (AC1)

Livrable : `docs/solutions/cross-repo-patterns/pilot-concurrency-shared-resources-2026-09-03.md`. Pour chacune des sept ressources nommées par AC1, un verdict **parmi trois** — *partageable en l'état* / *partageable après changement nommé* / *bloquante* — avec la citation `fichier:ligne` qui le justifie. Un verdict sans citation n'est pas un verdict.

L'inventaire de départ, établi par la lecture faite pendant le grooming (à confirmer et compléter par l'implémenteur, pas à recopier) :

| ressource | ancrage | verdict de départ |
|---|---|---|
| socket d'egress `/tmp/mika-pilot-egress.sock` | `dispatch-lib.sh:169`, proxy hôte unique, HTTP CONNECT multi-connexions | **partageable en l'état** — à confirmer par une mesure à deux clients simultanés |
| port `8891` du bac à sable | `dispatch-lib.sh:170`, mais `--unshare-net` en `:855` | **partageable en l'état** — chaque bac à sable a son propre namespace réseau ; le port n'est pas partagé, contrairement à ce que le corps du ticket suppose |
| répertoire de secrets | `dispatch-lib.sh:210` `_PILOT_SECRET_DIR_SANDBOX`, monté par `--ro-bind-data` depuis un fd (`:993`) | **partageable en l'état** — par-processus, jamais un chemin hôte commun |
| `~/.mika/data/pilot-transcripts/` | `dispatch-lib.sh:818`, `:1054`, `:1137`, `:2155` — un `.jsonl` par `task-id` | **partageable en l'état** — nommage par tâche, pas de fichier commun |
| `pilot-helper.log` / `pilot-egress-proxy.log` | `dispatch-lib.sh:189`, `/var/log/mika/` | **partageable après changement nommé** — deux écrivains entrelacés restent lisibles par un humain mais cassent tout comptage par session ; nommer le changement (préfixe `task-id` par ligne, ou fichier par dispatch) |
| helper mitmdump `:8892` | `dispatch-lib.sh:178`, `:306` — daemon hôte long-vivant, partagé, déjà multi-clients | **partageable en l'état** — mais le fichier de jeton `_PILOT_GH_TOKEN_FILE` (`:186`) est réécrit **avant chaque spawn** (`:846`) : deux spawns rapprochés se marchent dessus. **Changement nommé requis.** |
| rappel `canUseTool` → mika-dev | `a2a.rs:226`/`:360` `try_lock_owned()` → `"Agent is busy"` ; relais `.claude/claude-pilot.json` timeout 120 000 ms | **bloquante en l'état** — c'est *la* trouvaille de l'inventaire |

Le document doit aussi dire ce que devient une escalade refusée côté pilote : `claude-pilot` reçoit `"Agent is busy"` — le comportement observé (retry ? échec de l'outil ? abandon de session ?) se mesure, il ne se devine pas. La remédiation de cette ligne bloquante est fichée en **mika#2163** (file bornée sur le chemin A2A, l'équivalent de mika#1870 pour `/message`) ; l'inventaire la référence, il ne la réimplémente pas.

### Phase 2 — Borne matérielle chiffrée (AC2)

Livrable : une section chiffrée dans le même document, **à partir de mesures prises sur gentux**, jamais d'estimations.

Mesures déjà relevées pendant le grooming, à reprendre et à étendre :

```
/home  466 G  —  216 G libres        (les worktrees vivent ici)
RAM    61 G total, 43 G disponibles
CPU    16 cœurs
54 worktrees présents ; plus gros target/ survivant : 15 G
target/ du checkout principal : 15 G
```

À compléter par l'implémenteur : mémoire résidente crête d'un `cargo build` sur ce dépôt (`/usr/bin/time -v`), et la borne que la conjonction (disque libre ÷ taille d'un `target/` en vol) × (RAM ÷ crête de build) impose sur N **sur cette machine**. La référence mika#2105 (21–35 G par `target/` de spawn) est le point de départ, pas la conclusion : mes 15 G mesurés ce soir montrent que le chiffre bouge, donc la mesure se refait.

Une borne qui sort à N=2 est un résultat. Une borne qui sort à N=1 est un résultat aussi, et il clôt le ticket par « non, et voici pourquoi » — l'issue le dit explicitement.

### Phase 3 — Rendre le plafond paramétrable (AC3)

**Le défaut reste 1.** Deux endroits bougent ensemble, ou le réglage est décoratif (voir § « Ce que le code dit », points 1 et 2).

**3a — Le réglage.** Une fonction de lecture à la **forme à trois paliers exacte** du module, celle de `parse_max_behind` (`crates/mika-agent/src/auto_pull.rs:214-231`), `parse_max_redrives` (`:137-155`) et `parse_stuck_ready_threshold` (`:112-131`) :

```rust
const MAX_CONCURRENT_IMPLEMENT_DEFAULT: i64 = 1;
const MAX_CONCURRENT_IMPLEMENT_ENV: &str = "MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT";

/// absent/vide → défaut ; illisible/négatif → défaut avec WARN ; `0` → plafond désactivé.
fn parse_max_concurrent_implement(raw: Option<&str>) -> i64 { … }
fn max_concurrent_implement() -> i64 { … }
```

Deux points de forme, non négociables parce que c'est la forme du module :
- la **fonction pure** (`parse_*`) est séparée de la lecture d'environnement, pour être testable sans muter l'environnement du processus (motif posé en mika#1824 step 1) ;
- la sentinelle `0` signifie **désactivé** (plafond levé), pas « zéro dispatch ». C'est la convention déjà portée par `MIKA_AUTO_PULL_MAX_BEHIND` et `MAX_REDRIVES_ENV` — s'en écarter ici créerait deux grammaires de sentinelle dans le même dépôt.

Emplacement : à côté de la garde qu'il paramètre, dans `crates/mika-agent/src/skills/executor.rs` près de `derive_dispatch_class` (`:897`) — pas dans `auto_pull.rs`, qui ne consomme pas ce plafond, ni dans `config.rs`, dont ce module n'utilise aucune entrée.

**3b — Le prédicat d'existence devient un comptage.** `has_active_callback_tasks_excluding` (`db.rs:7883`) rend aujourd'hui `Option<BlockingDispatch>` sur un `LIMIT 1`. Ajouter un **compagnon** — ne pas muter la signature existante, elle a des appelants de test en `executor.rs:5080/5147/5164/5182` et un jumeau async en `async_db.rs:1220` :

```rust
/// Nombre de rappels actifs de cette classe, hors enveloppes `:deferred`,
/// hors le parent exclu. Même clause WHERE que has_active_callback_tasks_excluding.
pub fn count_active_callback_tasks_excluding(&self, …) -> Result<i64>
```

La garde en `executor.rs:1388` compare ce compte à `max_concurrent_implement()` et ne refuse qu'à `count >= N`. Le `BlockingDispatch` cité dans le refus reste celui du `LIMIT 1` — le refus doit nommer **un** détenteur, pas tous.

**3c — Le bail cesse d'être un plafond à 1.** `dispatch_slot_leases` a `PRIMARY KEY (agent_id, dispatch_class)`. Pour N>1 il faut une migration ajoutant un `slot_index INTEGER NOT NULL DEFAULT 0` à la clé primaire, et `try_acquire_dispatch_slot` (`db.rs:8019`) qui tente les index `0..N-1` jusqu'au premier succès. À N=1, exactement une place, `slot_index = 0` : le comportement d'aujourd'hui, bit pour bit.

**Décision à prendre par l'implémenteur, et à écrire dans le plan avant de coder :** 3c est une migration de schéma. Si la mesure de la Phase 2 conclut à N=1 comme borne matérielle, 3c devient du code mort ajouté au chemin critique de l'arbitrage. L'implémenteur **ordonne donc Phase 2 avant Phase 3c** et écrit le verdict dans le ticket ; il ne livre 3c que si la Phase 2 laisse N>1 sur la table. 3a et 3b se livrent dans tous les cas — ils sont le squelette du réglage et n'ont pas de coût de schéma.

**Ce qui ne bouge pas :** `DISPATCH_CLASSES` (`engine.rs`, épinglé sur `derive_dispatch_class` par un test de forme) et la classe `groom`, hors périmètre.

### Phase 4 — Test à N=2 (AC4)

Un test dans `executor.rs` (voisin des tests `:5139` et suivants) qui, avec `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT=2` :

1. deux dispatchs `implement` concurrents obtiennent **chacun leur bail** — donc l'assertion porte sur `SlotClaim::Acquired` **deux fois**, avec deux `holder_task_id` distincts ; un test qui n'assertait que le passage de la garde §2 laisserait passer la régression exacte que le point 3c prévient ;
2. un troisième est **refusé**, avec `error == "global_dispatch_active"`.

Le réglage se passe par la fonction pure `parse_max_concurrent_implement(Some("2"))` partout où c'est possible, pour ne pas dépendre de `std::env::set_var` — les tests du module tournent en parallèle et une mutation d'environnement y est une course.

### Phase 5 — Non-régression à N=1 (AC5)

À défaut (variable absente), reproduire la scène mesurée cette nuit : premier dispatch acquiert, second refusé avec `global_dispatch_active`, **et son rappel différé enregistré** — l'assertion doit couvrir `deferred_dispatch_registered == true` dans le JSON de refus (`executor.rs:1438`), pas seulement le code d'erreur. Un test qui vérifie le refus sans vérifier le différé laisserait passer une régression qui casse la reprise automatique (mika#1011).

### Phase 6 — La décision sur N revient à l'opérateur (AC6)

Un commentaire sur mika#2160 qui écrit : la borne matérielle chiffrée, le verdict de l'inventaire ressource par ressource, et le N que le code rend **atteignable** — suivi de la phrase que ce ticket porte depuis son ouverture : **le code ne choisit pas N ; il rend N choisissable.** Aucun changement de valeur par défaut, aucune variable d'environnement posée dans un service ou un `.env` par cette PR.

---

## Fire-Disposition

Trois livrables de ce plan sont de classe détecteur : le test à N=2 (Phase 4), le test de
non-régression à N=1 (Phase 5), et la garde de configuration à trois paliers (Phase 3a), dont le
palier « valeur illisible » est une détection au sens strict — il observe une entrée invalide et
émet un WARN.

**Disposition retenue : (c) halte-et-remontée à l'opérateur.** Pas (a) — il n'y a pas d'exception
nommée à porter dans une liste blanche, ces détecteurs n'ont pas de population de faux positifs
connue à amnistier. Pas (b) — livrer les tests `#[ignore]` reviendrait à livrer un réglage de
concurrence sans filet, ce qui est exactement ce que KTD1 et le risque « un réglage qui ment »
cherchent à empêcher.

Ce que « tire » veut dire pour chacun, et ce qui arrive alors :

| détecteur | ce qui le fait tirer | disposition |
|---|---|---|
| test N=2 (AC4) | un des deux baux n'est pas `Acquired`, ou le troisième dispatch n'est pas refusé | **échec CI, halte.** Le réglage est décoratif — c'est la régression que KTD1 nomme. Rien ne se livre, l'opérateur tranche. |
| test N=1 (AC5) | le second dispatch n'est plus refusé `global_dispatch_active`, ou son rappel différé n'est plus enregistré | **échec CI, halte.** C'est une régression du défaut, donc du comportement de production actuel. Priorité sur tout le reste du ticket. |
| garde de config (Phase 3a) | `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT` illisible ou négatif | **WARN + repli sur le défaut 1, sans halte.** Le repli est le comportement voulu (c'est le palier 2 de la forme à trois paliers) ; un réglage mal tapé ne doit pas arrêter la boucle. C'est le seul des trois qui ne halte pas, et c'est délibéré. |

Aucun de ces détecteurs n'a de sortie « ignorer et continuer ». Une CI rouge sur AC4 ou AC5 remonte
à l'opérateur avec la mesure, jamais avec un contournement.

---

## Décisions clés

**KTD1 — Le plafond bouge à deux endroits ou nulle part.** Le prédicat d'existence (§2) et la clé primaire du bail (§1) sont deux plafonds à 1 indépendants. Écarté : ne toucher que le prédicat, parce que le bail sérialiserait quand même et le réglage serait un mensonge lisible dans `--help`.

**KTD2 — Le défaut reste 1, et la PR ne pose la variable nulle part.** Le ticket est `operator-gated` ; livrer un défaut à 2 franchirait la porte que ce ticket se contente d'ouvrir.

**KTD3 — La sentinelle est `0` = désactivé, pas une valeur spéciale inventée.** Alignement sur `MIKA_AUTO_PULL_MAX_BEHIND` et `MAX_REDRIVES_ENV`. Écarté : `-1`, ou une chaîne `"off"` — deux grammaires de sentinelle dans un même dépôt sont une dette de lecture.

**KTD4 — Phase 2 avant Phase 3c.** La migration de schéma ne se paie que si la mesure laisse N>1 possible. Écarté : livrer 3c d'abord « pour être prêt » — c'est ajouter une jointure de clé primaire au chemin d'arbitrage sur la foi d'une hypothèse non mesurée.

**KTD5 — `count_*` est un compagnon, pas une mutation de signature.** `has_active_callback_tasks_excluding` a six appelants dont un jumeau async ; changer sa forme ferait de ce ticket un refactor.

**KTD6 — Le rappel `canUseTool` est classé bloquant, et le ticket de remédiation est fiché : mika#2163.** Une classification « bloquante » sans plan de remédiation est une dette documentée et non résolue ; le ticket existe donc avant que ce plan sorte du grooming, avec sa preuve (`a2a.rs:226`/`:360` en `try_lock_owned()`) et une mesure directe — la première passe architecte de ce grooming s'est fait refuser **cinq fois de suite** avec `"Agent is busy"` avant de passer à la sixième tentative, le 2026-09-03 entre 23:43 et 23:46 CEST. mika#2163 est un prérequis d'**exploitation** de N>1, pas un prérequis de **livraison** : ce plan livre son réglage à défaut 1 sans lui. Écarté : l'inclure au périmètre — c'est un changement du chemin de permission, la surface de sûreté délibérée, et il ne se glisse pas dans une étude de concurrence.

---

## Risques

- **Un réglage qui ment.** Si 3a+3b partent sans 3c, `MIKA_DISPATCH_MAX_CONCURRENT_IMPLEMENT=2` sera accepté, journalisé, et sans effet. Le test AC4 tel que spécifié en Phase 4 (assertion sur **deux baux acquis**) est précisément la garde qui l'empêche.
- **N=2 exploitable côté verrou, bloqué côté permission.** Le code peut rendre N=2 atteignable pendant que `a2a.rs:226` refuse la seconde escalade. C'est pourquoi AC6 exige que la décision d'exploitation soit écrite et laissée à l'opérateur : lever le plafond sans KTD6 échange une sérialisation lisible contre des refus de permission illisibles.
- **Deux causes voisines, non traitées ici.** mika#2156 (balayage phantom qui déclare morte une session de 4 h) rend ce plafond illisible ; mika#2158 (livelock des prédicats de grooming) tient la file vide. Corriger la concurrence sans #2158 augmente le débit d'une file qui n'a rien à donner. Les deux sont hors périmètre et le restent — mais le commentaire de la Phase 6 doit les nommer, pour que l'opérateur décide de N en sachant que le gain dépend d'elles.

---

## Acceptance criteria — rattachement

| AC | où c'est livré |
|---|---|
| AC1 — inventaire des sept ressources, trois verdicts possibles | Phase 1 → `docs/solutions/cross-repo-patterns/pilot-concurrency-shared-resources-2026-09-03.md` |
| AC2 — borne matérielle chiffrée sur mesures réelles | Phase 2, même document |
| AC3 — plafond paramétrable, forme à trois paliers, défaut 1 | Phase 3a (réglage) + 3b (comptage) + 3c (bail), KTD1/KTD2/KTD3/KTD4 |
| AC4 — test à N=2 : deux baux, troisième refusé | Phase 4 |
| AC5 — non-régression à N=1 : refus `global_dispatch_active` + différé enregistré | Phase 5 |
| AC6 — décision sur N écrite et laissée à l'opérateur | Phase 6 |

## Hors périmètre — confirmé

La classe `groom` (déjà parallèle). Le balayage phantom (mika#2156). Le livelock de grooming (mika#2158). La file bornée du chemin A2A — **fichée en mika#2163**, référencée par KTD6, non traitée ici.
