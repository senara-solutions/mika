# mika#1990 — la freshness de mika-manager bat au rythme du poll, pas de la delivery

> **Ticket :** `senara-solutions/mika#1990` — *obs(manager): emit cm heartbeat per POLL tick,
> not only per delivery — fixes false-RED between 6h beats*
> **Jalon :** #35 — Surface produit — agent et interfaces
> **Type :** observabilité (émetteur neuf, aucun réglage existant déplacé)

---

## Le défaut, tel que le ticket le pose

`mika-manager` poll toutes les 5 min (`MIKA_MANAGER_POLL_INTERVAL_SECS`, défaut `300`) et ne
POSTe **rien** tant que le cycle ne délivre pas. La delivery est hybride — `state_changed ||
heartbeat_fired`, le second sur un plancher de **6 h**. Entre deux battements légitimes, le
registre cm ne reçoit aucun signe de vie, la freshness de l'entité passe RED, et le
nudge-scanner criera au loup dès son premier scan alors que la cadence tourne parfaitement.

Le ticket demande : un POST léger **à chaque tick réussi**, sur
`/api/v1/agents/mika-manager/heartbeat`, avec `reason "poll:<n>"`, la delivery gardant son POST
à elle avec `reason "delivery:<severity>"`.

---

## Ce que la lecture du code déplace dans le ticket — **premier livrable**

Quatre rectifications, chacune changeant un bout du remède.

### R1 — « la delivery garde son POST propre reason » ne peut pas signifier « un champ de plus sur `DeliveryBody` »

Le POST de delivery existe (`HttpReportDeliverer::deliver`, `cadence.rs:304`) et son corps,
`DeliveryBody { milestone_ref, severity, report_markdown, assessment, generated_at, cycle_kind }`,
**ne porte aucun champ `reason`** et est un **format de fil que cm consomme**. Ajouter un champ
là serait une rupture gratuite d'un canal qui fonctionne, pour un besoin — la *freshness* — qui
ne se lit pas dans un rapport de 30 Ko posté quatre fois par jour.

**Lecture retenue :** le heartbeat est un **canal distinct** — endpoint distinct, corps minimal,
un POST par tick — et le `reason` dit *pourquoi ce battement a eu lieu*. C'est ce qui rend AC3
lisible littéralement : « un cycle simulé émet N ticks poll + 1 delivery » = N+1 battements, dont
exactement un porte `delivery:<severity>`.

### R2 — l'endpoint vit dans `control-monitor`, **hors de ce workspace**

Ce dépôt (`mika-platform/mika`) ne contient ni `control-monitor` ni aucune route
`/api/v1/agents/<id>/heartbeat`. Aucune ligne écrite ici ne peut faire exister cet endpoint ni
prouver qu'il répond. Le livrable est donc **l'émetteur seul**, et son URL est **déclarée, jamais
dérivée** :

- pas de composition depuis `delivery_url` (exemple posé dans `.env.example` :
  `https://cm.example.com/api/v1/messages/dispatch` — aucun rapport de forme avec
  `/api/v1/agents/<id>/heartbeat`) ;
- pas de composition depuis `health_url` (`…/api/v1/agents/mika-dev/health`), dont l'entité
  n'est pas la nôtre et dont la sémantique est inverse (on *lit* la santé de l'exécuteur, ici on
  *écrit* la nôtre).

Doctrine maison appliquée, pas inventée : mika#2249 (*« the worktree path is declared, never
derived »*) et mika#2368 (*« la cible PR est dite, jamais dérivée »*).

### R3 — AC4 « log discret » demande, sur ce serveur, un mécanisme **sans aucune trace collectée**

Le filtre de log de ce serveur n'admet **qu'une seule cible en DEBUG**, `mika::llm_debug` — le
`CLAUDE.md` racine le mesure : sur 200 Mo de `server.log`, le `debug!` de
`stuck_ready_reconcile_skipped` apparaissait **0** fois pendant qu'un `info!` du **même module**
y apparaissait 184 fois. Un `debug!` ici serait donc **structurellement invisible**, ce que
mika#2131 a dû fermer une fois déjà.

**« Discret » est donc réinterprété, et la réinterprétation est le livrable :** silencieux par
tick **nominal**, bruyant **sur transition** (l'endpoint tombe / se rétablit), et **une ligne au
démarrage** qui dit si le canal est armé. Détail en **D7**.

### R4 — collision de vocabulaire : `MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS` existe déjà et désigne **autre chose**

Cette variable désigne le **plancher de delivery 6 h**, pas ce battement-ci. Nommer la nouvelle
`MIKA_MANAGER_HEARTBEAT_URL` poserait côte à côte, dans le même `EnvironmentFile` :

```
MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS=21600
MIKA_MANAGER_HEARTBEAT_URL=https://cm.example.com/api/v1/agents/mika-manager/heartbeat
```

dont tout opérateur conclura « le heartbeat vers cette URL bat toutes les 6 h » — **très
exactement la croyance fausse que ce ticket existe pour tuer**. Le nom retenu est
`MIKA_MANAGER_LIVENESS_URL` : il sépare deux concepts que le vocabulaire confond, et la confusion
de ce vocabulaire *est* le défaut. Le ticket ne nomme aucune variable, donc rien n'est
contredit ; ce qui est nommé — l'endpoint `/heartbeat` — reste intact et est cité dans la doc.

---

## Décisions

### D1 — un POST par tick, le `reason` porté par ce POST

Un seul battement par tick. `reason = "poll:<n>"` quand le cycle n'a rien délivré,
`reason = "delivery:<severity>"` quand il a délivré. **Jamais deux POST sur le même tick** : la
freshness est un instant, pas un compte, et deux battements simultanés ne disent rien de plus
qu'un seul tout en dédoublant la population que cm voit.

### D2 — sur le bras `Ok` du cycle, **et seulement là**

« À CHAQUE poll tick **réussi** » (le ticket, en majuscules dans le corps). Un cycle qui échoue —
typiquement `gh` en 401 — est un manager **cassé** ; y poster « je suis vivant » serait le
mensonge exact que la freshness ne doit pas raconter. La panne de cette moitié a déjà son canal :
`manager_cycle_error` (WARN, `auth_class`) et l'alarme `manager_auth_persistent_failure` de
mika#2013. La freshness dit « la cadence tourne **et lit GitHub** » ; c'est plus fort que « le
process existe », et c'est le signal utile.

### D3 — `<n>` compte les **itérations de boucle**, pas les battements émis

Le compteur est incrémenté en tête d'itération, avant le cycle. Conséquence voulue : une suite
`poll:5` → `poll:9` dit **« quatre cycles ont échoué »**, information qu'un compteur de battements
(`poll:5` → `poll:6`) effacerait. Le compteur repart à `1` au démarrage du process, donc `poll:1`
est le marqueur d'un redémarrage — utile, pas un défaut, et **écrit dans la doc** pour qu'un
lecteur ne le prenne pas pour un bug.

### D4 — l'URL est déclarée ; absente ⇒ **rien n'est posté et rien n'échoue**

Trois paliers, la forme maison de `read_string_env` : absente ou vide ⇒ canal désarmé, zéro POST,
zéro erreur, zéro ligne par tick (mais **une** ligne au démarrage, cf. D7) ; posée ⇒ canal armé.
C'est aussi le **rollback** : retirer la variable désarme sans redéploiement.

### D5 — le nom : `MIKA_MANAGER_LIVENESS_URL`

Voir **R4**. La doc et le doc-comment de la constante nomment la collision explicitement, avec sa
raison, plutôt que de laisser un futur lecteur la redécouvrir.

### D6 — l'auth réutilise `delivery_token`, mais le battement **n'alimente pas** le ledger auth-boundary

Même destinataire (cm), même credential : le bearer est `cfg.delivery_token`, comme la delivery.

**Refus raisonné, et c'est la proposition qu'un relecteur fera :** ne pas verser les échecs du
battement dans `AuthBoundaryLedger` sous `manager_to_delivery`. Cette population compte les échecs
de **livraison de rapport** — de l'ordre de 4 tentatives/jour. Y verser **288 tentatives/jour**
changerait complètement ce que la requête opérateur
`SELECT target_key, after_value, count(*) … WHERE tool_name = 'auth_boundary'` mesure, et
noierait le signal que mika#1949 existe pour lever. L'information de classe (401/403 vs réseau)
n'est pas perdue pour autant : elle est portée par le WARN de transition (**D7**).

### D7 — le régime de journal : silencieux au tick, bruyant à la transition, une ligne à la configuration

| surface | niveau | cadence | régime attendu |
|---|---|---|---|
| `manager_delivery_resolved` gagne `liveness_url_set` | INFO | 1 / démarrage | **toujours présente** |
| succès d'un battement | *(aucune ligne)* | — | — |
| `manager_liveness_failed` | WARN | **1 / transition** sain→cassé | **vide** |
| `manager_liveness_recovered` | INFO | 1 / transition cassé→sain | vide |

La détection de transition est un état **en mémoire sur l'émetteur**, perdu au redémarrage à
dessein : un process neuf re-photographie ce qu'il trouve (motif `auto_pull_stop`, mika#2329).
Le WARN porte `consecutive_failures`, `reason`, et une classe (`credential_refused` /
`unreachable` / `other`) dérivée du **statut**, jamais d'une sous-chaîne du message (règle
mika#2179).

**Pourquoi la ligne de configuration est le cœur d'AC4 et pas un supplément :** sans elle,
« l'endpoint n'est pas configuré » et « l'émetteur n'est pas déployé » rendent des octets
identiques — classe mika#2205, *une garde qu'on n'a pas déployée se lit exactement comme une
flotte saine*. `manager_delivery_resolved` est déjà **l'événement de configuration de ce module**
(mika#2267 C3), émis une fois par démarrage, et son absence est déjà documentée comme un signal
de déploiement (classe mika#2340) : y ajouter un booléen est le geste le plus court et le plus
juste. On n'ajoute **pas** un second événement de configuration.

### D8 — `reason` est un **format de fil**

Les valeurs atterrissent dans le registre cm et un opérateur en fera des `GROUP BY`. Deux
orthographes d'un même motif couperaient une population en deux sans le dire. D'où : deux
constantes, **un seul site de composition** (`liveness_reason(tick, delivered, severity)`),
`severity` rendue par le même `snake_case` que `serde` (`healthy` / `attention` / `blocked`), et
un test qui fige les formes exactes.

### D9 — le battement ne retarde pas son propre cycle, et le suivant seulement de façon bornée

Émis **après** le cycle, donc il ne retarde pas ce cycle-ci. Il peut retarder le **suivant** : le
client `reqwest` porte un timeout de **5 s** (aligné sur `probe_executor_health`, même nature —
une sonde de liveness bornée court), soit au pire **1,7 %** d'un `poll_interval` de 300 s. Et
`tokio::time::interval` rattrape un tick retardé (`MissedTickBehavior::Burst` par défaut).

**Alternative écartée :** `tokio::spawn` détaché. Ça rendrait AC1 trivialement vrai, au prix de
perdre l'ordre des battements (deux `poll:<n>` peuvent arriver inversés chez cm) et de rendre AC3
non testable de façon déterministe. Le timeout court suffit, et l'attente est **awaitée** pour que
le test puisse l'observer.

---

## Livrables

### U1 — la variable et le champ de config

- `pub const ENV_LIVENESS_URL: &str = "MIKA_MANAGER_LIVENESS_URL";` (`spawn.rs`, avec les neuf
  autres `ENV_*`).
- `ManagerConfig.liveness_url: Option<String>`, peuplé par `read_string_env` dans
  `manager_config_from_env`.
- **`.env.example`** : la ligne déclarative, dans le bloc `MIKA_MANAGER_*`, avec le commentaire
  qui nomme la collision de R4.
  *Non optionnel :* `mika2267_every_manager_env_const_is_declared_in_env_example` (T7,
  `sink_dir.rs:599`) refuse toute `pub const ENV_*` que `.env.example` ne déclare pas. Son
  assertion d'anti-vacuité (`declared.len() >= 10`) reste satisfaite — on passe à 11.

### U2 — le canal

```rust
/// Corps du battement. Minimal à dessein : la freshness est un instant.
pub struct LivenessBody {
    pub entity: String,          // "mika-manager"
    pub reason: String,          // "poll:<n>" | "delivery:<severity>"
    pub generated_at: String,    // RFC 3339
    pub milestone_ref: MilestoneRef,
}

#[async_trait]
pub trait LivenessSink: Send + Sync {
    async fn beat(&self, url: &str, token: Option<&str>, body: &LivenessBody)
        -> Result<(), LivenessFailure>;
}
```

`HttpLivenessSink` en production (client `reqwest` à timeout 5 s, `bearer_auth` si token) ; un
sink en mémoire en test. Motif exact de `ReportDeliverer` / `HttpReportDeliverer`, qui existe dans
le même fichier et dont les doubles de test sont déjà écrits sur cette forme.

`LivenessFailure { class: LivenessFailureClass, message: String }` où la classe vient du
**statut**, jamais du texte : `CredentialRefused` (401 | 403), `Unreachable`
(`is_connect() || is_timeout()`), `Other`.

**Contrainte de garde à connaître avant d'écrire le POST :** `no_dispatch_test.rs` interdit le
littéral `"POST"` dans tout fichier du module (`FORBIDDEN_TOKENS`, ligne 42). `client.post(url)`
ne le contient pas ; une construction par `Method::POST` ou par chaîne le contiendrait et ferait
rougir la garde LECTURE-SEULE. On écrit `client.post(url)`, comme `HttpReportDeliverer`.

### U3 — l'émetteur : la décision et l'état

```rust
pub struct LivenessEmitter {
    tick: u64,                    // D3 — itérations de boucle
    failing: bool,                // D7 — état de transition
    consecutive_failures: u32,
}
```

Deux fonctions pures et testables sans réseau :

- `liveness_reason(tick: u64, delivered: bool, severity: &Severity) -> String` — le **site unique**
  de composition (D8) ;
- `LivenessEmitter::observe(outcome) -> Option<TransitionLine>` — dit s'il faut émettre un WARN,
  un INFO de rétablissement, ou rien.

Plus `LivenessEmitter::beat(...)`, qui compose les deux et appelle le sink. `tick` est incrémenté
par le **site d'appel** en tête d'itération (D3), pas par `beat` — sinon un cycle en erreur, qui
ne bat pas, n'incrémenterait pas et les trous disparaîtraient.

### U4 — le câblage

Dans `spawn_manager_cycle_task` (`spawn.rs`) :

1. tête d'itération, après le `select!` : `emitter.next_tick()` ;
2. bras `Ok(outcome)` du `match run_manager_cycle_with_auth(...)`, **après** le bloc
   `manager_cycle_delivered` existant : `emitter.beat(&cfg, &sink, &outcome).await` ;
3. bras `Err(_)` : **rien** (D2).

`spawn_manager_cycle_task` gagne un paramètre `liveness_sink: Arc<dyn LivenessSink>`, exactement
comme mika#2013 lui a ajouté `token_resolver: Arc<dyn TokenResolver>`. Appelant unique :
`server::run_server`.

### U5 — la surface de configuration

`emit_delivery_resolved` (`spawn.rs:508`) gagne **un champ**, `liveness_url_set: bool`. Aucun
nouvel événement (D7). Le test négatif existant de cette fonction — aucun matériel de credential
n'atteint un champ — couvre le nouveau champ par construction puisqu'il balaie *tous* les champs.

### U6 — la documentation

- `crates/mika-agent/CLAUDE.md` § *Milestone Manager* : le contrat de sortie passe de « le seul
  effet de bord sortant est un POST de rapport » à « … un POST de rapport **et un battement de
  liveness** », plus la liste des variables (dixième → onzième) ;
- le `CLAUDE.md` racine : une entrée `MIKA_MANAGER_LIVENESS_URL` avec les surfaces, les régimes et
  les haltes ;
- le doc-comment du module `cadence.rs`, dont l'en-tête énonce aujourd'hui le contrat
  LECTURE-SEULE en nommant *un seul* effet de bord sortant. **Atomique avec le code** : la garde
  `no_dispatch_test.rs` prescrit en toutes lettres de mettre à jour le test *et* le docstring
  ensemble, jamais de desserrer l'un sans l'autre.

---

## Fire-Disposition

Ce plan livre des détecteurs : un **scan de source** SOLE WRITER et un **pinning de format de
fil**. Disposition retenue : **(a) exception nommée en allowlist — avec allowlist livrée vide.**

| détecteur | allowlist | raison |
|---|---|---|
| `mika1990_le_battement_a_un_seul_ecrivain` | `LIVENESS_BEAT_SITES_ALLOWED: &[&str] = &[]` | le code est **neuf** : il n'existe aucune violation préexistante à exempter. Doctrine mika#2201 : *quand il tire, on retire le second site, on ne l'allowliste pas.* |
| `mika1990_le_vocabulaire_du_reason_est_un_format_de_fil` | — | pinning de valeurs, pas de population à exempter |

**Assertion auto-nettoyante (obligatoire, et c'est elle qui rend le scan honnête) :** le scan
**rougit si le nom qu'il cherche n'est écrit nulle part**. Un scan visant un nom mort vérifie zéro
chose et se lit exactement comme un arbre propre — classe mika#2205, et la maison l'a payée assez
souvent pour que ce soit un réflexe (`mika2496_the_cost_overrun_name_has_a_single_writer` porte la
même assertion, `mika2420` la porte sur ses deux aiguilles).

**Second terme, contre le rétrécissement silencieux :** le scan assère aussi une **cardinalité** —
exactement **un** site de composition de `reason` et exactement **un** site d'appel au sink en
production. Sans cardinalité, un prédicat devenu trop étroit passe en ne regardant rien
(mika#2496 U3).

**Pourquoi un scan et pas un test de comportement :** un second écrivain de battement ne rend
**aucune décision fausse** le jour où il est écrit — le cycle continue de tourner, chaque
assertion reste verte, et seule la freshness devient incomptable (deux suites de `<n>`
entrelacées chez cm). Invisible à tout test comportemental. C'est très exactement la classe que
`grooming_marker` (mika#2158) a dû fermer dans ce dépôt.

**Contrôle de bonne foi** (le scan rougit-il vraiment ?) : un test compagnon injecte un second
site factice et vérifie que le scan le voit — sans quoi « le scan regarde » et « le scan est
inerte » se lisent pareil.

---

## Vérification

| # | ce qui est établi | forme |
|---|---|---|
| **V1** | **AC3** — un cycle simulé de N+1 ticks émet `poll:1..N` puis exactement un `delivery:<severity>` | émetteur + sink en mémoire, aucun réseau, aucun `tokio::time` |
| **V2** | **contrôle négatif** — URL absente ⇒ **zéro** appel au sink, **zéro** erreur, cycle inchangé | idem |
| **V3** | **AC1** — un sink qui rend `Err`, et un sink qui pend au-delà du timeout, laissent le cycle rendre `Ok` et la boucle continuer | sink en échec / sink lent + `tokio::time::pause` |
| **V4** | **AC2** — les deux motifs sont **distincts** et figés (`poll:7`, `delivery:blocked`, …) | assertion sur les littéraux exacts (D8) |
| **V5** | **D3** — un cycle en erreur crée un **trou** : `poll:5` puis `poll:7`, jamais `poll:5` puis `poll:6` | séquence pilotée |
| **V6** | **D7** — un WARN sur la **première** panne, rien sur les suivantes, un INFO au rétablissement | `LivenessEmitter::observe` sur une séquence sain/cassé/cassé/sain |
| **V7** | Fire-Disposition — les deux scans + leur contrôle de bonne foi | scan de source |
| **V8** | **non-régression** — `no_dispatch_scaffolding_in_milestone_manager` passe (pas de littéral `"POST"`) et `mika2267_every_manager_env_const_is_declared_in_env_example` passe (onzième variable déclarée) | suites existantes |

**Ce que la vérification ne peut PAS établir, écrit plutôt que découvert :** que le battement
*arrive* chez cm. `control-monitor` est hors de ce workspace (R2) ; le contrat côté mika est
*« le POST part, avec le bon motif, au bon rythme, sans bloquer »*, et V1–V6 l'attestent
déterministiquement. La moitié comportementale est la sonde S1 ci-dessous.

**Deuxième limite, nommée :** la boucle `spawn_manager_cycle_task` n'est pas testable de bout en
bout — elle porte un `tokio::time::interval`, un `verify_gh_auth` au démarrage, et
`run_manager_cycle_with_auth` construit son `Reader` en dur (non injectable). C'est **le scan de
cardinalité (V7)** qui couvre « le site d'appel existe et il est unique », pas un test
d'intégration. Rendre la boucle injectable est un refactoring hors périmètre.

---

## Surfaces opérateur

```bash
# 1. Le canal est-il armé ? (une ligne par démarrage)
grep manager_delivery_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq '{milestone, liveness_url_set, route_normal, delivery_token_present}'

# 2. Le canal est-il tombé ? (régime attendu : VIDE)
grep manager_liveness_failed "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{milestone, class, consecutive_failures, reason}'

# 3. S'est-il rétabli ?
grep manager_liveness_recovered "$MIKA_SPIRIT_LOG_FILE"
```

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `manager_delivery_resolved` avec `liveness_url_set: true` | INFO | **1 / démarrage** | le canal est armé. Son **absence** pendant que la cadence tourne = binaire antérieur au correctif (classe mika#2340) — **jamais** « tout va bien » |
| `liveness_url_set: false` | INFO | — | canal désarmé : c'est une **configuration**, pas une panne ; la freshness restera RED et c'est attendu |
| `manager_liveness_failed` | WARN | **vide** | chaque ligne est un battement perdu ; `class` dit lequel des trois remèdes (credential / réseau / autre) |
| `manager_liveness_recovered` | INFO | vide | le pendant du précédent |
| ligne par battement réussi | — | **aucune** | AC4 |

**Pas de ligne `audit_events`, et c'est une décision :** à 288 battements/jour et par milestone,
une ligne par battement serait exactement le churn que la doctrine mika#2131 borne, et
l'information durable (« le canal est tombé à telle heure ») est déjà portée par les deux WARN de
transition. La population des battements **réussis** se lit chez cm — c'est son registre, c'est
tout l'objet du ticket.

---

## Sondes post-déploiement, et leurs haltes

> **Préalable.** Établir que le binaire servi porte le correctif : la ligne
> `manager_delivery_resolved` doit porter le champ `liveness_url_set`. **Sans cette
> vérification, chacune des sondes ci-dessous décrit le binaire d'hier** (classe mika#2340).

**S1 — le symptôme (la sonde qui compte, ~15 min après le déploiement).** Côté cm, la freshness de
l'entité `mika-manager` doit rester verte en continu, avec un battement toutes les ~5 min, et le
nudge-scanner doit cesser de la signaler entre deux deliveries.
**Halte 1 — la freshness reste RED alors que `manager_liveness_failed` est vide.** Le POST part et
n'atteint pas le registre : **ne pas élargir l'émetteur par réflexe** — vérifier d'abord l'URL
posée (sonde 1 : `liveness_url_set`), puis que l'endpoint cm existe et lit ce corps. C'est la
moitié `control-monitor` du travail, et elle a son propre dépôt.

**S2 — contrôle positif de non-vacuité (24 h).** Le registre cm doit montrer les deux motifs :
beaucoup de `poll:*` et quelques `delivery:*`.
**Halte 2 — uniquement des `poll:*`, jamais un `delivery:*`, sur 24 h.** C'est **attendu** si
aucune delivery n'a eu lieu (état stable, moins de 6 h écoulées) — vérifier
`manager_cycle_delivered` **avant** de conclure à un défaut. Zéro des deux ne prouve rien.

**S3 — AC1 sur le terrain (48 h).** Aucune régression de la cadence : `manager_cycle_delivered`
continue d'apparaître à son rythme, aucun `manager_cycle_error` neuf.
**Halte 3 — les cycles ralentissent ou se décalent.** Le battement borne mal : lire
`manager_liveness_failed` — un `class = unreachable` soutenu signifie que le timeout est atteint à
chaque tick, soit 5 s perdues toutes les 5 min. **Désarmer d'abord** (retirer
`MIKA_MANAGER_LIVENESS_URL` de l'environnement du service — le rollback de D4, sans
redéploiement), diagnostiquer ensuite.

**S4 — contrôle négatif de bruit (7 jours).** `manager_liveness_failed` reste vide.
**Halte 4 — flot soutenu.** Ce n'est pas un seuil à régler : c'est l'endpoint cm qui refuse, et
`class` dit lequel des trois remèdes. Un `credential_refused` renvoie au `delivery_token` (D6) et
**pas** à ce code.

**Halte transverse — les trois greps muets et la freshness verte.** On ne peut rien conclure :
vérifier que la cadence a réellement tourné (`manager_cadence_start`, puis au moins un
`manager_cycle_delivered` ou une erreur) avant toute conclusion. *Une garde que personne n'a
exercée se lit exactement comme une garde qui marche* (mika#2205).

---

## Ce que ce travail n'achète PAS

- **Il ne fait pas exister l'endpoint cm.** R2 : `control-monitor` est hors de ce workspace. Si
  `/api/v1/agents/mika-manager/heartbeat` n'existe pas encore, chaque battement partira en 404,
  `manager_liveness_failed` le dira une fois, et la freshness restera RED. **La moitié cm est un
  ticket frère**, et l'ordre est contraint dans l'autre sens que d'habitude : émettre avant que
  l'endpoint existe est **sûr** (un 404 est borné, journalisé, non bloquant), l'inverse ne l'est
  pas non plus — un endpoint sans émetteur est simplement inerte. Les deux moitiés tombent
  indépendamment.
- **Il ne rend pas la delivery plus fréquente.** Le plancher 6 h et le déclencheur `state_changed`
  ne bougent pas d'un octet. Ce qui change est **ce que cm sait de notre vivacité** entre deux
  rapports, pas la fréquence des rapports.
- **Il ne dit rien de la santé de GitHub ni de l'exécuteur.** `manager_cycle_error`,
  `manager_auth_persistent_failure` et `probe_executor_health` gardent chacun leur périmètre.
  Le battement dit « la cadence tourne et a lu le jalon », et rien d'autre.
- **Il n'ajoute aucun compteur et aucune ligne d'audit** (voir *Surfaces opérateur*) : les seuls
  instruments neufs sont les deux WARN de transition et le booléen de configuration, et **leur
  silence ne prouve rien tant que personne n'exécute les sondes**.
- **Il ne rattrape aucun battement perdu.** Pas de file, pas de réessai : un battement raté est
  perdu, et le suivant arrive dans 5 min. C'est le bon arbitrage pour un signal de vivacité —
  réessayer un « je suis vivant » périmé est au mieux inutile, au pire un mensonge daté.

---

## Hors périmètre, délibérément

- **L'endpoint cm et le nudge-scanner** — autre dépôt, ticket frère (ci-dessus).
- **Rendre `spawn_manager_cycle_task` testable de bout en bout** (injecter `Reader`) — refactoring
  réel et utile, sans rapport avec ce défaut, et dont le coût dépasse celui du livrable.
- **Renommer `MIKA_MANAGER_HEARTBEAT_INTERVAL_SECS`** pour dissiper la collision de R4 à la source.
  Ce serait une rupture de configuration en production pour un gain cosmétique ; le nom neuf
  (D5) et la doc suffisent. **Suivi possible**, précondition : qu'un opérateur se soit réellement
  trompé entre les deux.
- **Verser le battement dans le ledger auth-boundary** — refusé avec sa mesure en D6.
- **Un battement sur les ticks en erreur** — refusé en D2. Si une mesure montre un jour qu'on veut
  distinguer « process mort » de « process vivant mais aveugle », le geste est un **troisième
  motif** (`reason = "error:<class>"`), pas un assouplissement du prédicat — et c'est un ticket,
  avec la mesure en précondition.
- **La promotion Phase 2** du manager (autorité de dispatch) : ce travail ne touche ni
  `no_dispatch_test.rs::FORBIDDEN_TOKENS` ni la liste des gates, et le contrat LECTURE-SEULE reste
  entier — ce qui s'ajoute est un effet de bord **sortant de liveness**, de la même famille que le
  POST de rapport qui existe depuis le premier jour.

---

## Definition of Done

1. `MIKA_MANAGER_LIVENESS_URL` lue, portée sur `ManagerConfig`, déclarée dans `.env.example`.
2. `LivenessSink` + `HttpLivenessSink` + `LivenessEmitter` livrés, avec le motif composé à un seul
   site.
3. La boucle bat sur le bras `Ok`, une fois par tick, avec un timeout de 5 s et sans jamais
   propager d'erreur.
4. `manager_delivery_resolved` porte `liveness_url_set` ; `manager_liveness_failed` /
   `manager_liveness_recovered` émettent **sur transition seulement**.
5. V1–V8 verts, y compris les deux gardes préexistantes du module.
6. Les deux scans de Fire-Disposition livrés, allowlist **vide**, avec leur assertion
   d'anti-vacuité et leur contrôle de bonne foi.
7. `CLAUDE.md` racine, `crates/mika-agent/CLAUDE.md` et le doc-comment de `cadence.rs` mis à jour
   **dans le même commit** que le code (prescription de `no_dispatch_test.rs`).
8. `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` verts.

---

## Acceptance criteria

1. **Un POST de heartbeat est émis par poll tick réussi, best-effort et jamais bloquant pour le
   cycle.** Un sink qui rend une erreur, ou qui pend au-delà de son timeout, laisse le cycle rendre
   `Ok` et la boucle poursuivre (V3). Le POST est borné à 5 s (D9).
2. **Les motifs poll et delivery sont distincts.** Un tick sans delivery porte `poll:<n>` ; un tick
   qui a délivré porte `delivery:<severity>` avec `severity` ∈ `{healthy, attention, blocked}`.
   Les deux formes sont figées par test comme un format de fil (V4, D8).
3. **Un cycle simulé émet N ticks poll + 1 delivery.** Une séquence de N+1 ticks dont seul le
   dernier délivre produit exactement `poll:1` … `poll:N` puis un unique `delivery:<severity>`
   (V1).
4. **Le journal est discret :** aucune ligne par battement réussi. Les seules surfaces sont le
   booléen `liveness_url_set` sur l'événement de configuration existant (1 / démarrage) et deux
   lignes de **transition** dont le régime attendu est vide (D7, V6).
5. **URL absente ⇒ désarmé proprement :** zéro POST, zéro erreur, cycle byte-identique au
   comportement d'avant le correctif (V2, D4).
6. **Un cycle en échec ne bat pas**, et le trou dans la suite de `<n>` est conservé comme
   information (D2, D3, V5).
7. **Les deux gardes préexistantes du module restent vertes** : le contrat LECTURE-SEULE
   (`no_dispatch_test`) et la déclaration des variables (`mika2267_…_env_example`) — V8.
8. **Les détecteurs livrés portent leur Fire-Disposition** : allowlist vide, assertion
   d'anti-vacuité, contrôle de bonne foi (V7).
