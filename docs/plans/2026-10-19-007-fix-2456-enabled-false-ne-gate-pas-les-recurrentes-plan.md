# Plan — `enabled = false` ne gate pas les tâches récurrentes (mika#2456)

> Re-groom du 2026-10-19. Remplace `docs/plans/2026-09-21-004-fix-2456-enabled-false-ne-gate-pas-les-recurrentes-plan.md`,
> dont la substance est reprise ici. La wip de la PR fermée #2468 (`backup/2456-wip`, `d9b2f4eb`)
> reste la référence à copier pour les tests et le filet du dispatcher, **mais elle précède
> trois changements de `main`** et son volume dépasse la borne par vol d'un facteur ~3,8.
> Ce re-groom tranche les trois points nommés par le ticket et découpe le travail en deux vols.

## 1. Ce que le ticket dit, et ce que le code dit

Trois rectifications, déjà établies par le plan du 2026-09-21 et re-vérifiées sur `main` :

**R1 — il n'existe aujourd'hui aucun champ `enabled` au niveau agent.** `Identity`
(`crates/mika-agent/src/prompt.rs:841-871`) porte exactement dix champs : `name`, `emoji`,
`reflection`, `heartbeat`, `kg`, `skills`, `tools`, `context`, `session`, `curator`. Pas de
racine `enabled`. La phrase du corps du ticket — « la porte `enabled` ne couvre que le chemin
interactif » — est donc **fausse dans sa prémisse** : il n'y a pas de porte à étendre, il y a
une porte à créer. Le commentaire 2/5 de l'opérateur dit la même chose autrement, et c'est lui
qui a raison sur le fond : le levier demandé est *une* garde niveau-agent plutôt qu'une
sous-clé par feature.

**R2 — ce que mika-test portait réellement.** Son `identity.toml` déclare `[kg] enabled = false`
et `[skills] nudge_enabled = false`. Aucun des deux n'est une porte niveau-agent. `Identity`
ne porte pas `#[serde(deny_unknown_fields)]`, donc un `enabled = false` écrit à la racine à la
main **se désérialise sans erreur et est ignoré** : c'est la forme exacte d'un réglage qu'on
croit avoir posé. Ce plan le rend lisant.

**R3 — l'état de gatage par feature, mesuré.** `heartbeat` est gaté par `[heartbeat] enabled`
(`server/mod.rs:1896-1904`, branche `else` = `cancel_recurring_task_by_label`), `reflection`
par `[reflection] enabled` via `reflection_cron_for_agent` (`:1908-1916`), et `curator_review`
par **rien du tout**. Les cinq autres sites d'enregistrement de `server/mod.rs` (`:1939`,
`:1961`, `:1995`, `:2028`, `:2047`) sont inconditionnels eux aussi. C'est le `curator_review`
non gaté qui explique la mesure du ticket : il se ré-enregistre à chaque boot complet, quel que
soit l'état de l'agent.

**Coût mesuré, inchangé :** ~5,18 $/22 h sur le tenant canari cloud mika-test, six tours
automatiques/jour, tous des sessions `heartbeat-*`, dont au moins un fini en
`reasoning budget exhausted` — budget de sortie brûlé en raisonnement, zéro texte livré.

## 2. Les trois points que le ticket demande de trancher

### Point 1 — où placer la porte depuis mika#2601

mika#2601 a découpé l'enregistrement en trois fonctions :

```rust
pub async fn ensure_recurring_task(db, label, cron_expr, action_config)              // :230  pub
pub(crate) async fn ensure_recurring_task_with_policy(db, …, policy)
    -> Result<(), RegistrationFailure>                                              // :256  pub(crate)
async fn register_recurring_task_once(db, …) -> anyhow::Result<()>                   // :329  privée
```

**Décision : la porte vit en tête de `ensure_recurring_task_with_policy`, AVANT
`retry_on_busy`, et `home_dir: &Path` entre dans la signature des DEUX fonctions publiques.**

Quatre raisons, dont la première est celle que le ticket demande (« les appels directs ne
doivent pas lui échapper ») :

1. `_with_policy` est la fonction la plus interne qui soit **atteignable de l'extérieur**
   (`pub(crate)` ; `register_recurring_task_once` est privée et n'a qu'un appelant, déjà épinglé
   par le scan mika#2601). Toute arête entrante traverse donc l'une des deux signatures — et
   comme les deux prennent `home_dir`, c'est **le compilateur** qui interdit l'échappement, pas
   un scan structurel qu'il faudrait écrire, maintenir et dont il faudrait prouver qu'il n'est
   pas vacant. La doctrine mika#2201 appliquée en amont : on route le site, on n'ajoute pas une
   ligne d'allowlist.
2. **Avant `retry_on_busy`, pas dedans.** La lecture de `identity.toml` n'est pas de la
   contention : la rejouer une fois par tentative lirait N fois le même fichier pour la même
   réponse, et un `identity.toml` momentanément illisible deviendrait un échec « busy » qu'il
   n'est pas.
3. Placer la porte dans `register_recurring_task_once` est rejeté pour la même raison, plus
   une : son doc-comment porte une contrainte de nommage dure (le nom ne doit pas finir par le
   nom de l'API publique suivi de `(`, sinon la garde par sous-chaîne de mika#2337 s'arrête), et
   on ne touche pas à cette fonction.
4. `retry_on_busy` **n'émet aucun événement** — il est pur, générique sur `Result`, et son
   doc-comment dit que c'est cette généricité qui fait de « une erreur non-busy n'est pas
   rejouée » une propriété de la fonction. Les événements sont écrits par `_with_policy`. On
   peut donc réutiliser `retry_on_busy` au point 2 sans mélanger deux populations sous un nom.

**Échappement résiduel, nommé plutôt que tu :** un appelant qui passerait un `home_dir`
fantaisiste résoudrait `enabled = true` par le défaut d'ouverture (§3.2). C'est un mésusage
délibéré, pas une classe d'accident : la porte ne peut pas distinguer « mauvais chemin » de
« agent sans `identity.toml` », et faire échouer le second fermerait la flotte entière (R-4).

### Point 2 — l'annulation doit-elle réessayer sous contention

**Décision : oui, par réutilisation du `retry_on_busy` existant, sous un nom d'événement
DISTINCT de celui de l'enregistrement.**

Pourquoi réessayer : AC1 est une assertion sur l'**état de la base** (« aucune ligne
`recurring_active` pour un agent désactivé »). Une annulation perdue sous contention rend AC1
intermittemment fausse *sans aucun signal* — la porte a refusé, l'événement de refus est écrit,
et la ligne est toujours là. C'est la pire forme : un instrument qui affirme avoir agi.

Pourquoi un nom distinct : deux populations sous un seul nom ne sont pas soustractibles
(mika#2205), et le scan mika#2601 épingle déjà « un seul écrivain » pour
`recurring_registration_retried` et `recurring_registration_failed`. Les noms de la porte sont
`recurring_gate_cancel_retried` et `recurring_gate_cancel_failed` — le scan ne rougit pas, et
la question « est-ce que la *porte* perd des annulations ? » a son propre grep.

**L'asymétrie avec l'enregistrement doit être écrite, parce qu'elle n'est pas intuitive :**

| | ce qu'une perte coûte | borne |
|---|---|---|
| enregistrement perdu | rien ne tire ce label jusqu'au prochain redémarrage | **non bornée** |
| annulation perdue | une ligne continue de tirer | bornée par le filet du vol 2, et **auto-guérie** au prochain boot ou provision — le bloc gaté tourne à chaque chargement d'agent |

L'annulation est donc le cas *moins* grave, et c'est précisément pourquoi il ne faut pas lui
donner une politique de réessai à elle : on réutilise la `RecurringRetryPolicy` déjà passée en
paramètre, sans nouveau réglage.

### Point 3 — `mika tasks rearm` sur un agent désactivé

**Décision : un refus explicite, par une variante `RearmError::AgentDisabled`, vérifiée à la
même position que `NotRoutable` — avant la trace d'audit et avant le marqueur.**

`rearm_recurring_task(db, label)` (`task_engine/mod.rs:439-649`) finit par appeler
`ensure_recurring_task(db, &target.label, &cron_expr, &target.action_config)` (`:631`) puis
vérifie `get_recurring_task_cron(&target.label).is_none()` → `RearmError::NotRegistered`.

Sans refus nommé, un rearm sur agent désactivé **tombe dans `NotRegistered`**, dont le message
dit *« read the mika#1742 warning in the log »* — un avertissement qui n'a jamais été émis.
Le doc-comment de l'énum interdit exactement ça : *« un geste opérateur refusé sans raison
lisible est un geste qu'on contourne au jugé — en éditant la base, précisément ce que la
commande existe pour rendre inutile. »* Une porte qui envoie l'opérateur lire une garde qui
n'a rien dit est la fabrication d'un contournement.

**Le fil `home_dir` n'est pas un choix, il est forcé par le compilateur :**
`rearm_recurring_task` appelle `ensure_recurring_task`, dont la signature prend désormais
`home_dir` — donc `rearm_recurring_task` doit en recevoir un, variante nommée ou pas. Le coût
côté CLI est nul : `crates/mika-cli/src/commands/tasks.rs:195` tourne sous
`let ctx = init::init_db_only_for_agent(agent_name)?;` et `DbContext` porte déjà
`pub home_dir: PathBuf` (`crates/mika-cli/src/init.rs:30-33`). La variante nommée est donc la
moitié *bon marché* du geste, et son omission serait un défaut qu'on sait introduire.

**Position de la garde — avant l'audit, pas après.** L'ordonnancement actuel est porteur :
`NotRoutable` refuse à `:578-596`, **avant** `log_audit_event` (`:609`) et avant
`mark_recurring_operator_rearm` (`:629`). `AgentDisabled` prend la même place : un geste refusé
ne laisse pas de marqueur d'exécution.

## 3. Le levier : un `enabled` à la racine de `identity.toml`

### 3.1 Pourquoi là, et pas ailleurs

- **Pas `[agent] enabled`** — les sous-clés sœurs (`[heartbeat] enabled`, `[reflection] enabled`,
  `[kg] enabled`) vivent déjà au niveau des sections ; la porte générale est au-dessus d'elles,
  donc à la racine, à côté de `name` et `emoji`.
- **Pas `customer_config`** — trois raisons, et la troisième est dirimante : (a) les siblings
  par feature sont dans `identity.toml` ; (b) le levier doit être posable **hors ligne, sur un
  pod arrêté**, ce qui est le geste d'urgence du ticket ; (c) `customer_config` est
  **modifiable par le modèle** via `SETTABLE_CONFIG_KEYS`, et un agent désactivé ne doit pas
  pouvoir se ré-activer.
- **Pas code-owned** — `enabled` n'entre PAS dans `CODE_OWNED_IDENTITY_SECTIONS`. Cette
  constante est réécrite au démarrage ; y mettre `enabled` écraserait le réglage de l'opérateur
  au prochain boot. C'est le défaut exact que mika#1833 a dû nommer pour `[kg]`, une section
  plus loin. Gardé par V8 (vol 2, cf. §7).

### 3.2 Trois états, un seul défaut

`pub enabled: Option<bool>` avec `#[serde(default)]`. `Identity` ne dérive que
`Debug, Deserialize, Clone` — **pas `Serialize`** — donc l'`Option` ne change aucune
sérialisation existante et ne peut pas fuiter dans un template.

| sur disque | résolu | source |
|---|---|---|
| `enabled = true` | `true` | `identity` |
| `enabled = false` | `false` | `identity` |
| absent | `true` | `default` |

`Option<bool>` est choisi plutôt qu'un `bool` + `#[serde(default = "…")]` parce qu'il rend la
**provenance** gratuite : `self.enabled.is_some()` distingue « l'opérateur a posé `true` » de
« personne n'a rien posé », sans relire le TOML brut et sans enum supplémentaire. C'est ce
champ `source` qui rend la sonde de la halte 1 (§7) interprétable.

```rust
/// `(en vigueur, provenance)` pour la porte récurrente niveau-agent (mika#2456).
pub fn recurring_tasks_enabled(&self) -> (bool, &'static str) {
    match self.enabled {
        Some(v) => (v, "identity"),
        None => (true, "default"),
    }
}
```

### 3.3 L'identité fail-closed reste `enabled = true`

`fail_closed_identity()` (`prompt.rs:1069`) laisse `enabled: None`, donc `true`. C'est un refus
argumenté, pas un oubli. La doctrine mika#2023 dit *« une valeur non reconnue échoue fermé,
une absence non »*, et le fail-closed de mika#2027 ferme les **outils** (allowlist sentinelle,
denylist mutative) pour empêcher l'agent d'**agir**. Fermer aussi les récurrentes ferait d'un
`identity.toml` momentanément illisible — un `rename` interrompu, un disque plein — l'extinction
silencieuse de tout le heartbeat de la flotte. Un commentaire sur le champ le dit, parce que le
prochain lecteur posera la question.

### 3.4 Conjonction avec les leviers par feature

La porte niveau-agent et les sous-clés se composent en **ET** : `enabled = false` coupe tout ;
`enabled = true` ne ré-active rien que `[heartbeat] enabled = false` avait coupé. La porte est
strictement soustractive — elle ne peut jamais enregistrer une tâche qu'un appelant n'aurait
pas demandée.

### 3.5 Portée : les récurrentes, et rien d'autre

La porte agit sur les tâches de `trigger_type = "recurring"` uniquement. Les rappels
(`trigger_type = "time"`), le chemin interactif et les callbacks sont intouchés : mika-test est
un **banc**, et `mika ask --agent mika-test` doit continuer de répondre après le correctif.
Le ticket demande la fin des *tours automatiques*, pas la mise hors service de l'agent.

## 4. Invariants

- **R-1** — la porte est strictement soustractive (§3.4).
- **R-2** — absence de la clé ⇒ `true` ; aucun agent de la flotte ne change de comportement.
- **R-3** — la porte ne vit qu'à un seul endroit atteignable (§2, point 1), et c'est le
  compilateur qui le tient.
- **R-4** — un `identity.toml` illisible résout `true` (§3.3).
- **R-5** — la porte n'écrit jamais `identity.toml`.
- **R-6** — **aucun agent du dépôt ne reçoit `enabled = false`.** `well_known_agents.rs` n'est
  pas touché : la décision runtime appartient à l'opérateur (« Décision runtime = opérateur »,
  corps du ticket). Le travail livre le levier, pas la décision.
- **R-7** — `enabled` n'entre pas dans `CODE_OWNED_IDENTITY_SECTIONS` (§3.1).
- **R-8** — chaque branche de la porte, **la branche saine comprise**, dit ce qu'elle a résolu
  (§5) : un réglage qu'on ne peut pas observer n'est pas un réglage, c'est un espoir
  (mika#2293).
- **R-9** — `docs/configuration.md` documente la clé, ses trois états et la conjonction du
  §3.4. (Hors compte de lignes : `docs/` est exclu.)

## 5. Observabilité

Trois événements au vol 1, un au vol 2. Tous en INFO, `target: "mika::otel"`.

| événement | vol | quand |
|---|---|---|
| `agent_recurring_gate_resolved` | 1 | à **chaque** résolution, les deux branches. Champs : `agent_id`, `enabled`, `source` ∈ {`identity`, `default`} |
| `recurring_registration_refused_agent_disabled` | 1 | la porte a refusé un enregistrement. Champs : `agent_id`, `label` |
| `recurring_gate_cancel_retried` / `recurring_gate_cancel_failed` | 1 | l'annulation de la porte a réessayé / a épuisé son budget (point 2) |
| `recurring_fire_refused_agent_disabled` | 2 | le filet du dispatcher a refusé un tir |

**`agent_recurring_gate_resolved` est émis à chaque appel, sans déduplication — et c'est un
arbitrage, pas un oubli.** La wip portait un `OnceLock<Mutex<HashSet<(String, bool, &str)>>>`
pour n'annoncer qu'un changement d'état. On s'en passe au vol 1 : la répétition est bornée par
le nombre de labels récurrents (sept par agent au boot), jamais par le trafic, et vingt-deux
lignes d'état global mutable pour économiser six lignes de log par agent par boot est le
mauvais côté du marché quand le vol est à douze lignes de sa borne. Si la mesure
post-déploiement montre que le bruit gêne la lecture, la déduplication est un suivi de trois
lignes de plan — pas une dette structurelle.

**Requête opérateur** (sur le `audit_events` du vol 2, `tool_name = 'agent_recurring_gate'`,
dédupliqué à 24 h) :

```sql
SELECT target_key, after_value, count(*)
FROM audit_events
WHERE tool_name = 'agent_recurring_gate'
GROUP BY 1, 2;
```

## 6. Découpage en vols

La wip `d9b2f4eb` pèse **≈ 1 064 lignes de code** hors son document de plan
(`prompt.rs` 103, `dispatcher.rs` 279, `task_engine/mod.rs` 286, `well_known_agents.rs` 25,
le test eval 309, et sept fichiers à ≤ 30). Contre une borne par vol de **279**, c'est ~3,8×.
C'est le fait décisif de ce re-groom, et le seul qui change la forme du travail par rapport au
plan du 2026-09-21 : **il faut deux vols, mergeables séparément.**

### Vol 1 (CE vol) — la porte à l'enregistrement

Ferme la cause mesurée : `curator_review` et les six autres sites qui se ré-enregistrent à
chaque boot. Satisfait AC1, AC3, AC4, AC5, AC6, AC7.

**Ordre de commit :**

1. `prompt.rs` — le champ et son résolveur. Seul, il ne gate rien et ne casse rien.
2. `task_engine/mod.rs` — le résolveur `agent_recurring_tasks_enabled`, la porte en tête de
   `_with_policy`, l'annulation réessayée, `home_dir` sur les deux signatures.
3. Les onze sites d'appel inline des tests de `task_engine` + les dix sites de production
   (`server/mod.rs` ×7, `chat.rs` ×2, `commands/tasks.rs` ×1). Le compilateur énumère la liste.
4. `RearmError::AgentDisabled` + la garde dans `rearm_recurring_task`.
5. `tests/eval/test_agent_disabled_recurring_gate_2456.rs` — V1, V2, V3, V7, V9.
6. `docs/configuration.md` + l'entrée de `crates/mika-agent/CLAUDE.md` (hors compte).

**Plancher, dans cet ordre, si le pilote chauffe :** d'abord la documentation de CLAUDE.md
(R-9 garde `docs/configuration.md`), puis V7. Les étapes 1 à 5 moins V7 sont le plancher dur :
sans V2 il n'y a pas de contrôle négatif, sans V3 l'anti-résurrection n'est pas tenue, sans V9
le refus du point 3 n'est pas attesté.

### Vol 2 — le filet au tir et sa surface d'audit

Couvre AC2 pour les deux populations que la porte du vol 1 ne voit pas : les lignes nées
**avant** le déploiement, et les bascules de `enabled` **en cours d'exécution** (la porte ne
tourne qu'au boot et au provision). Contenu : `refuse_recurring_fire_if_agent_disabled` dans
`dispatch_run_skill`, l'événement `recurring_fire_refused_agent_disabled`, la ligne
`audit_events` `tool_name = 'agent_recurring_gate'` avec son rafraîchissement 24 h, le compte
de lignes annulées (`cancel_recurring_task_by_label -> Result<usize>`), V4, V5, V6 et le scan
structurel V8.

**Le filet n'est pas redondant avec la porte**, et c'est pourquoi il est un vol et non une
coupe : en régime attendu il n'écrit **aucune ligne**, exactement comme le filet du dispatcher
de mika#2337. Son zéro est l'assertion que la porte amont tient.

**Trois corrections à appliquer en reprenant la wip** — elle les porte telles quelles :

1. `cancel_recurring_task_by_label` rend `Result<()>` et **pas** un compte de lignes
   (`async_db.rs:425`, `db/tasks.rs:528`). Le champ `cancelled_rows` de la wip exige donc de
   changer les deux signatures. Les deux appelants existants font `if let Err(e) = …`, donc le
   passage à `Result<usize>` est compatible — mais c'est du travail, et il est ici.
2. `refuse_recurring_fire_if_agent_disabled` contient un reste de débogage :
   `if false && task.trigger_type != …RECURRING { return false; }`. La restriction de
   `trigger_type` était **désactivée** : le filet refusait aussi des rappels, ce que §3.5
   interdit.
3. Le filet n'a besoin d'aucune plomberie : `TaskDispatcher` porte déjà
   `pub home_dir: PathBuf` (`dispatcher.rs:508`) et `pub global_home_dir: PathBuf` (`:523`).

Tracked in: un ticket de suivi « mika#2456 vol 2 — filet au tir des récurrentes pour agent
désactivé », à ouvrir à la fusion du vol 1.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/prompt.rs` — `enabled: Option<bool>`, `recurring_tasks_enabled()`, commentaire §3.3 | 34 |
| `crates/mika-agent/src/task_engine/mod.rs` — résolveur, annonce, porte + annulation réessayée, deux signatures | 64 |
| `crates/mika-agent/src/task_engine/mod.rs` — `RearmError::AgentDisabled` + garde dans `rearm_recurring_task` | 26 |
| sites de production (`server/mod.rs` ×7, `chat.rs` ×2, `commands/tasks.rs` ×1) | 14 |
| sites d'appel inline des tests existants de `task_engine` (×11) + un helper de home | 15 |
| tests (`crates/mika-agent/tests/eval/test_agent_disabled_recurring_gate_2456.rs`) — V1, V2, V3, V7, V9 | 100 |
| test structurel V8 (`canonical_tokens.rs`) | 14 |

Total estimé : 267 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Le vol courant tient **sous** la borne avec ~12 lignes de marge, et il y tient par découpage :
la wip de référence pèse ≈ 1 064 lignes, soit ~3,8× la borne. Le vol 2 est nommé au §6 par sa
ligne `Tracked in:` et son périmètre n'entre pas dans ce total — l'architecte juge le vol
courant, jamais la somme.

Deux leviers de volume déjà exercés, pour que la marge ne soit pas reconquise par inadvertance :
la déduplication process-globale de l'annonce de porte (~22 lignes, arbitrée au §5) et la carte
d'audit à rafraîchissement 24 h de la wip (~88 lignes, partie du vol 2 avec le filet qu'elle
instrumente).

## Fire-Disposition

Ce plan livre des détecteurs : les tests comportementaux V1, V2, V3, V7, V9 (vol 1) et le scan
structurel V8 (vol 2).

**Option (a) — exception nommée en allowlist, avec allowlist livrée VIDE et assertion
auto-nettoyante.**

Le seul détecteur qui a une allowlist est V8, le scan structurel qui refuse l'entrée de
`enabled` dans `CODE_OWNED_IDENTITY_SECTIONS` (R-7). Il est livré avec
`const AGENT_ENABLED_CODE_OWNED_EXCEPTIONS: &[&str] = &[];` — **vide**, par la même idiome que
`RECURRING_REGISTRATION_SCAN_EXCEPTIONS: &[&str] = &[]` de mika#2601 (`canonical_tokens.rs:1164`),
et pour la même raison : quand il rougit, le remède est de router la décision, pas d'ajouter une
ligne (doctrine mika#2201). Il porte l'assertion anti-vacuité du voisinage — attester que le
balayage a réellement vu la constante — et compose son motif à l'exécution pour que le fichier
ne se dénonce pas lui-même (motif mika#2496).

**Zéro violation existante, et c'est démontrable, pas espéré :** R-6 interdit de donner
`enabled = false` à un agent du dépôt, et aucun `identity.toml` de `well_known_agents.rs` ne
porte la clé — elle n'existe pas encore. Les cinq détecteurs comportementaux du vol 1
construisent leurs propres fixtures (`.pilot-scratch/`-style homes temporaires), donc ils n'ont
aucune population préexistante à excepter. Aucun détecteur n'atterrit désarmé, aucun ne requiert
de halte.

## 7. Vérification

**Vol 1 :**

- **V1** — agent avec `enabled = false` : `ensure_recurring_task` n'enregistre rien, et une
  ligne `recurring_active` préexistante du même label est annulée.
- **V2** — **contrôle négatif** : même graine, un seul octet d'écart (`enabled = true`), la
  récurrente s'enregistre normalement. C'est la paire V1/V2 qui sépare « la porte mord » de
  « rien ne s'enregistre jamais ».
- **V3** — **anti-résurrection** : une ligne annulée par la porte ne revient pas au boot
  suivant tant que l'agent est désactivé. Le vecteur est
  `revert_config_cancel_recurring_task` (mika#2271), qui lève le veto mika#1742 sur les lignes
  `cancelled` — c'est exactement ce que la porte ne doit pas déclencher, et c'est pour ça
  qu'elle refuse **avant** d'entrer dans `register_recurring_task_once`.
- **V7** — `identity.toml` absent ou illisible : résolution `true`, source `default`,
  comportement de la flotte inchangé (AC7, §3.3).
- **V9** — `mika tasks rearm` sur un agent désactivé rend `RearmError::AgentDisabled`, **pas**
  `NotRegistered`, et n'écrit ni ligne d'audit ni marqueur de rearm.

**Vol 2 :** V4 (le filet refuse un tir sur agent désactivé), V5 (contrôle négatif du filet),
V6 (conjonction : `[heartbeat] enabled = false` mord toujours sous `enabled = true`),
V8 (scan structurel de R-7).

**Sonde post-déploiement, avec ses haltes nommées.** L'instrument n'est pas vivant avant
`make deploy` (classe mika#2340 : une ligne absente signifie un binaire périmé, jamais une
absence de porte).

1. `grep agent_recurring_gate_resolved` dans `$MIKA_SPIRIT_LOG_FILE` doit rendre une ligne par
   label par agent au boot, `source = "default"` pour toute la flotte.
   **HALTE si `source = "identity"` apparaît sur un agent** : quelqu'un a posé la clé sans que
   R-6 l'autorise.
2. Poser `enabled = false` sur mika-test, redémarrer, vérifier `mika tasks list --agent mika-test`
   vide de `recurring_active`.
   **HALTE si une ligne survit** : la porte ne couvre pas ce site.
3. `mika ask --agent mika-test "ping"` doit toujours répondre (§3.5).
   **HALTE si l'agent est muet** : la portée a fui hors des récurrentes.

## 8. Hors périmètre

- Les rappels, le chemin interactif, les callbacks, les runs d'équipe (§3.5).
- `well_known_agents.rs` et toute décision runtime (R-6).
- Une surface CLI pour poser `enabled` (`mika agents disable`) — le geste est une édition de
  fichier, posable hors ligne, ce qui est la propriété demandée.
- Le nettoyage des lignes `cancelled` historiques.

## 9. Risques

| risque | mitigation |
|---|---|
| un agent de la flotte s'éteint par surprise | R-2 + R-6 + V7 ; la clé n'existe nulle part sur disque, et personne ne l'écrit |
| la porte déclenche la résurrection mika#2271 | V3 ; le refus est avant `register_recurring_task_once` |
| une annulation perdue sous contention | point 2 : réessai + événement dédié ; borne du tableau §2 |
| un rearm refusé sans raison lisible | point 3 : `AgentDisabled` nommé, avant l'audit |
| le vol 1 fusionne sans le filet du vol 2 | les lignes nées avant le déploiement restent exposées jusqu'au vol 2 ; c'est un coût nommé, borné par le fait que le bloc gaté tourne à chaque boot et provision, donc il se résorbe de lui-même agent par agent |

## Definition of Done

- Le champ `enabled` est lu à la racine de `identity.toml`, avec ses trois états du §3.2.
- La porte vit en tête de `ensure_recurring_task_with_policy`, avant `retry_on_busy`, et
  `home_dir` est dans la signature des deux fonctions publiques — l'échappement est interdit
  par le compilateur.
- L'annulation de la porte réessaie sous contention SQLite, sous un nom d'événement distinct de
  celui de l'enregistrement.
- `mika tasks rearm` sur un agent désactivé refuse par `RearmError::AgentDisabled`, avant la
  trace d'audit et avant le marqueur.
- Les trois événements du vol 1 sont émis, la branche saine comprise.
- V1, V2, V3, V7, V9 passent. `cargo test`, `cargo clippy`, `cargo fmt --check`,
  `make verify-bundled-skills` et `scripts/verify-pipeline.sh origin/main` sont verts.
- `docs/configuration.md` documente la clé ; `crates/mika-agent/CLAUDE.md` porte l'entrée de
  doctrine ; `scripts/sync-agent-docs.sh` a tourné si `docs/` a bougé.
- Le vol 2 est ouvert comme ticket de suivi et nommé par la ligne `Tracked in:` du §6.

## Acceptance criteria

- **AC1** — un agent avec `enabled = false` ne porte aucune tâche `recurring_active` après un
  boot ou un provision. *(vol 1)*
- **AC2** — un agent avec `enabled = false` ne produit aucun tour automatique récurrent, y
  compris depuis une ligne née avant le déploiement ou après une bascule en cours d'exécution.
  *(vol 1 pour la cause mesurée ; vol 2 pour ces deux populations résiduelles)*
- **AC3** — un agent avec `enabled = true`, ou sans la clé, porte ses récurrentes normalement.
  *(vol 1, V2)*
- **AC4** — la porte est strictement soustractive : `enabled = true` ne ré-active pas ce qu'une
  sous-clé par feature avait coupé. *(vol 1 par construction, attesté au vol 2 par V6)*
- **AC5** — chaque résolution de la porte est observable, la branche saine comprise, avec la
  provenance de la valeur. *(vol 1, R-8)*
- **AC6** — `mika tasks rearm` sur un agent désactivé rend un refus qui nomme sa cause, et ne
  laisse ni ligne d'audit ni marqueur. *(vol 1, V9)*
- **AC7** — aucun agent existant ne change de comportement : absence de la clé ⇒ `true`, et
  `identity.toml` illisible ⇒ `true`. *(vol 1, V7)*
