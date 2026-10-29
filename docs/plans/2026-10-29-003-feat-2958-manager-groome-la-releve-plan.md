# feat(manager) — Phase 2 (c) vol 2 : le manager groome la relève d'avance, derrière `MIKA_MANAGER_GROOM_WRITE` OFF

**Ticket :** senara-solutions/mika#2958 (`enhancement`, `p1-important`, `dispatch:loop`, `loop-substrate`)
**Branche :** `feat/2958/manager-phase-2-c-vol-2-le-manager`
**Base mesurée :** `origin/main` à `74e7254fc`
**Lié :** mika issue#2700 (vol 1, PR #2740), mika issue#2698 (EVALUATE_WRITE), mika issue#2699 (POOL_WRITE), mika issue#2956, mika issue#2267 (où vit une const d'env), mika issue#1990 (une URL sortante est DÉCLARÉE), mika issue#2201 (lecteur unique), mika issue#2205 (deux populations sous un nom), mika issue#2331 (absent ≠ zéro), mika issue#2309 (une dérogation expire par le geste qui la rend inutile), mika issue#1727 (`mika ask` ne fait tourner aucune boucle), mika issue#2163 (la file d'attente A2A refuse, elle ne pend pas)

---

## 1. Ce qui est mesuré

### 1.1 Le vol 1 nomme la relève et n'a aucun moyen d'agir

`reporter.rs:425-470` rend la section, et son titre **porte le fait** plutôt qu'une précaution :

```rust
        if let Some(relief) = &assessment.relief
            && !relief.next.is_empty()
        {
            writeln!(out, "### Relève à grommer (mode ombre — aucun groom déclenché)").unwrap();
```

Le commentaire qui précède (`reporter.rs:408-423`) l'écrit en clair : « Au vol 1, aucun
interrupteur n'existe — la mention n'est pas une précaution, c'est le fait. »

### 1.2 Le docstring qui engage ce vol, et les deux gardes qui rougissent

`mod.rs:256-265` :

> Le geste de groom est le **vol 2** : derrière cet interrupteur, OFF par défaut, par la voie
> moteur existante (le groom passe par mika-dev, jamais par un spawn), et il arrivera comme
> **exception nommée** au scan de `no_dispatch_test.rs`, amendée dans le même commit que ce
> paragraphe — jamais comme un assouplissement du scan. Les deux moitiés de
> `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` sont ce qui rougit ce jour-là.

`mod.rs:407` — la garde, ses deux moitiés :

| moitié | ce qu'elle affirme aujourd'hui | ce qui la fait rougir au vol 2 |
|---|---|---|
| 1 | le fichier de garde ne nomme ni `relief.rs`, ni `reporter.rs`, ni `mika#2700` | rien — le vol 2 ne touche pas `relief.rs` ; cette moitié est **réaffirmée**, pas levée |
| 2 | aucun fichier de production ne déclare `concat!("MIKA_MANAGER_", "GROOM_WRITE")` | **la déclaration de l'interrupteur**, au premier commit de ce vol |

Le message de la moitié 2 dit déjà ce que ce commit doit porter : *« Si c'est le vol 2 qui
arrive, il arrive avec son `write_is_armed()`, sa table de vérité et son contrôle négatif, et ce
test change dans le même commit. »*

### 1.3 Le manager n'a ni base de données, ni `AgentState` — et c'est ce qui décide AC3

`spawn.rs` : `spawn_manager_cycle_task` est une tâche tokio nue lancée par `server::run_server`.
Elle ne reçoit **ni `AsyncDatabase`, ni `AppState`, ni `resolve_agent`**. Ses deux seuls rails
sortants sont :

- `GhRunner` / `ProcessGhRunner` (`reader.rs`) — la CLI `gh` ;
- des POST HTTP vers des **URL déclarées** (`HttpReportDeliverer`, `HttpLivenessSink`,
  `probe_executor_health`).

Conséquence directe : « l'équivalent interne de `mika ask --agent mika-dev "groom mika#N"` »
n'est **pas** un appel en process. Il n'en existe pas.

### 1.4 Ce que `mika ask` fait réellement depuis mika issue#1727

`crates/mika-cli/src/commands/ask.rs:370-420` :

```
// --- #1727 TUI/CLI thin-client slice ---
// The one-shot agent loop no longer runs in-process here. It is delegated to
// the local mika-spirit daemon over A2A (`message/send`) […] this process is now
// a thin client that ships the prompt and renders the returned Task.
```

Le site d'envoi unique est `remote_ask.rs::send_message_to_agent` : un `A2aClient` borné en
temps, un `context_id` frappé **avant** l'envoi (mika issue#2036), un JSON-RPC `message/send`.
Donc **la voie du moteur, pour le manager, est exactement la même porte que celle de `mika ask`** :
`POST /a2a/mika-dev` sur le spirit local (`server/mod.rs:388-407`), méthode `message/send`.

### 1.5 Le piège à écrire noir sur blanc : `returnImmediately` ne fait tourner aucune boucle

`server/a2a.rs:1229-1236` :

> **`returnImmediately` is exempt, and the exemption is new.** That branch creates the task row
> and returns it in `submitted`; **it never runs the agent loop**, so it never needs the lock that
> exists to serialise turns.

Un manager qui enverrait `configuration.returnImmediately = true` créerait une tâche `submitted`
qui **ne groome rien**, et le rapport dirait « groom déclenché ». C'est un vert mensonger, de la
classe exacte que ce module passe son temps à refuser. **Le geste est synchrone.** Il tient
parce que le tour `groom` de mika-dev *dispatche* un claude-pilot d'arrière-plan et rend la main :
le tour est court, ce n'est pas le groom entier qui est attendu.

---

## 2. Exigences

| # | exigence | AC |
|---|---|---|
| R1 | un interrupteur `MIKA_MANAGER_GROOM_WRITE`, OFF par défaut, dont l'état OFF est une garantie **structurelle** et non une discipline d'appelant | AC1 |
| R2 | le geste entre comme **exception nommée** au scan de `no_dispatch_test.rs`, au fichier et au jeton près ; le scan n'est pas assoupli | AC1 |
| R3 | les deux moitiés de `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` amendées **dans le même commit** que la déclaration de l'interrupteur | AC1 |
| R4 | sélection et idempotence : un ticket déjà groomé, ou dont un groom est en cours, n'est pas relancé — et le plan **dit par quelle lecture** | AC2 |
| R5 | la voie est celle du moteur (A2A `message/send` vers mika-dev), ni spawn ni dispatch d'implémentation | AC3 |
| R6 | le geste est **borné** : un nombre de grooms par cycle, et une issue nommée quand mika-dev a déjà un pilote en vol | AC3 |
| R7 | OFF : la section « Relève à grommer » est **inchangée**, titre compris. ON (en test seulement) : le rapport dit les déclenchés et les sautés, avec leur motif | AC4 |
| R8 | taille estimée tests compris, découpée en vols au-delà de la borne | Taille |

---

## 3. Découpage en vols

Le total honnête du ticket est de **765 lignes estimées** (§ *Taille estimée*), au-dessus de la
borne par vol de 429. Découpage en **deux vols**, chacun mergeable seul.

**Vol 1 — ce plan, cette PR : le geste, son interrupteur, son exception nommée.**
C'est exactement ce que le docstring de `mod.rs:256-265` engage : l'interrupteur arrive *avec*
son `write_is_armed()`, sa table de vérité et son contrôle négatif. Périmètre : AC1 entier, plus
les constantes de bornage qu'AC3 exige (`GROOMS_PER_CYCLE`), plus le prédicat de sélection
qu'AC2 exige (`deja_groome` / `groom_en_cours` passés en **paramètre**, pas lus).

**Vol 2 — `Tracked in:` ci-dessous : le rail, le câblage, le rapport.**
Le rail HTTP (`A2aGroomSender`), ses deux constantes d'URL et de jeton, le câblage dans
`cadence.rs`, la lecture « groom en cours » sur le rail `gh`, le champ de checkpoint, et le rendu
AC4 dans `reporter.rs`.

**Pourquoi la frontière est là, et pas ailleurs.** Déclarer `ENV_GROOM_URL` au vol 1 serait
déclarer une URL que rien ne lit : exactement l'« interrupteur dont la branche ON ne fait rien »
que ce module refuse. Le rail arrive donc **avec son site d'appel**, au vol 2. Le vol 1, lui, est
complet et attestable de bout en bout contre un `GroomSender` enregistreur — ce qui est la seule
chose dont AC1 a besoin.

**Tracked in:** un ticket de suivi à ouvrir, intitulé *« feat(manager) : Phase 2 (c) vol 2/2 — le
rail A2A du groom, son câblage de cycle et son rendu de rapport (suite de mika issue#2958) »*,
portant AC2 (la lecture), AC3 (le rail et son refus `agent_busy`) et AC4 (le rapport). Il ne peut
être ouvert qu'après merge de ce vol, dont il dépend par compilation.

---

## 4. Conception du vol 1

### 4.1 Nouveau fichier `crates/mika-agent/src/milestone_manager/groom_trigger.rs`

Le patron est `pool_writer.rs` (mika issue#2699 vol 4), à la ligne près, parce que c'est ce que
l'exception nommée du scan sait déjà lire.

```rust
/// MIKA_MANAGER_GROOM_WRITE — troisième et dernière exception nommée à l'invariant
/// LECTURE SEULE du module. OFF par défaut : l'absence de la variable EST l'état normal.
pub const ENV_GROOM_WRITE: &str = "MIKA_MANAGER_GROOM_WRITE";

/// Le geste, à plat. Il est à plat pour une raison : c'est cette chaîne-là que le scan de
/// `no_dispatch_test.rs` cherche, et c'est elle qui est nommée dans l'exception. Une forme
/// éclatée (`.method("message").arg("send")`) échapperait au grep et ferait de l'exception
/// une fiction.
const GESTE: &str = "message/send";

/// Un seul groom par cycle. Le nombre n'est pas une prudence : le moteur ne tient qu'UN
/// créneau `dispatch_class = 'groom'` par agent (schéma v34), donc un second envoi dans le
/// même cycle serait différé par le moteur et compté comme déclenché par le manager — deux
/// populations sous un nom (mika#2205).
pub const GROOMS_PER_CYCLE: usize = 1;
```

**La table de vérité, et pourquoi `armed` est un paramètre.**

```rust
pub fn groom_write_is_armed() -> bool { armed_from_value(std::env::var(ENV_GROOM_WRITE).ok().as_deref()) }

pub(crate) fn armed_from_value(v: Option<&str>) -> bool {
    matches!(v.map(str::trim), Some("1") | Some("true") | Some("TRUE") | Some("True"))
}
```

`armed` traverse ensuite l'API comme **paramètre**, jamais comme lecture d'environnement au point
d'usage. C'est ce qui rend la table OFF/ON testable sans `unsafe { set_var }` et donc en
parallèle — le même choix qu'au vol 4 de mika issue#2699.

**L'émetteur, injectable.**

```rust
#[async_trait::async_trait]
pub trait GroomSender: Send + Sync {
    /// `instruction` est le corps du message utilisateur, déjà formé : `"groom mika#N"`.
    async fn send(&self, instruction: &str) -> Result<(), GroomSendError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroomSendError {
    /// mika-dev tient déjà un verrou de tour : la file d'attente A2A refuse (mika#2163).
    AgentBusy,
    /// Le jeton a été refusé (401/403).
    CredentialRefused,
    /// Rien n'a répondu.
    Unreachable,
    Other,
}
```

`GroomSendError` est dérivé du **statut HTTP**, jamais d'une sous-chaîne de message
(mika issue#2179) — même forme que `LivenessFailureClass`. Le vol 1 fixe l'énumération et la
fige par un test de vocabulaire de fil (mika issue#1990 D8) : les opérateurs `GROUP BY` dessus.

**Le geste borné.**

```rust
pub struct GroomCandidate { pub number: u64, pub deja_groome: bool, pub groom_en_cours: bool }

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GroomTriggerOutcome {
    pub declenches: Vec<u64>,
    /// `(numéro, motif)` — motif dans le vocabulaire fermé de § 4.4.
    pub sautes: Vec<(u64, &'static str)>,
}

pub async fn trigger_relief_grooms(
    armed: bool,
    sender: Option<&dyn GroomSender>,
    candidats: &[GroomCandidate],
) -> GroomTriggerOutcome
```

Les refus sont **ordonnés et exhaustifs**, sur le patron des trois refus de
`write_pool_admissions` :

1. `!armed` → `disarmed`, et **retour immédiat** : `sender` n'est pas touché. C'est la garantie
   structurelle d'AC1 : il n'y a pas un chemin où `armed == false` et où l'émetteur est appelé.
2. `sender.is_none()` → `no_sender` (le rail n'est pas configuré ; au vol 1, c'est le cas nominal).
3. `candidats.is_empty()` → `no_relief`.
4. par candidat : `deja_groome` → `already_groomed` ; `groom_en_cours` → `groom_in_flight`.
5. au-delà de `GROOMS_PER_CYCLE` envois réussis → `budget_spent`, pour **tous** les restants.
6. `Err(AgentBusy)` → `agent_busy`, et le cycle s'arrête là (le créneau est pris, le suivant
   échouerait identiquement) ; les autres `Err` → `send_failed`, et le candidat n'entre **pas**
   dans `declenches`.

Le point 6 est la réponse d'AC3 à « ce qu'il fait quand mika-dev a déjà un pilote en vol » :
**il ne pend pas et ne retente pas** — il nomme `agent_busy` et laisse le cycle suivant reprendre.
Le refus existe déjà côté A2A (`a2a_wait_queue::try_take_slot` / `wait_for_agent_lock` /
`refuse_busy`, mika issue#2163), donc le manager n'invente pas une issue : il la transcrit.

### 4.2 L'exception nommée, et le jeton qui la rend réelle

`no_dispatch_test.rs` aujourd'hui :

```rust
    assert_eq!(EXEMPTED_OCCURRENCES.len(), 2,
        "la Phase 2 du manager n'a reçu que deux gestes d'écriture : …");
```

**Le geste de ce vol n'est pas une commande `gh`.** Il n'existe pas de `gh` qui déclenche un
groom. Donc, laissé tel quel, le scan **ne verrait pas** le nouveau geste, et une « exception
nommée » à un scan qui ne détecte rien serait une fiction — pire que son absence.

Le vol 1 **resserre** le scan plutôt que de l'assouplir, en deux gestes :

1. **Ajout de `"message/send"` à `FORBIDDEN_TOKENS`.** Le jeton devient interdit dans tout le
   module, sauf au site nommé. Vérifié avant rédaction de ce plan :
   `grep -rn "message/send" crates/mika-agent/src/milestone_manager/` ne rend **rien** — donc
   zéro violation préexistante, et le détecteur arrive propre (§ *Fire-Disposition*).
2. **Troisième entrée dans `EXEMPTED_OCCURRENCES`** :
   `("groom_trigger.rs", "message/send", "mika#2958 — le manager déclenche le groom de la relève par la voie moteur (A2A message/send vers mika-dev), derrière MIKA_MANAGER_GROOM_WRITE, OFF par défaut")`.
   La raison nomme le ticket **et** la variable, comme les deux entrées existantes, parce que
   c'est ce que les trois assertions `raison.contains(…)` exigent.

Le pin passe de 2 à 3 et le test est **renommé**, pas étendu :
`mika2699_lallowlist_porte_les_deux_gestes_et_pas_un_de_plus` →
`mika2958_lallowlist_porte_les_trois_gestes_et_pas_un_de_plus`, avec son message réécrit (« la
Phase 2 du manager a reçu trois gestes d'écriture, un par vol »). Un nom de test qui dit « les
deux gestes » en vérifiant trois entrées est un commentaire faux à l'endroit le plus coûteux.

**L'auto-nettoyage couvre la nouvelle entrée gratuitement**, et c'est ce qui en fait une
dérogation qui expire (mika issue#2309) : la boucle finale de ce test lit chaque fichier nommé,
le tronque à son premier `#[cfg(test)]`, et rougit si le jeton n'y est plus — donc le jour où le
geste disparaît, l'exception rougit au lieu de rester un trou dans le grep.

### 4.3 Les deux gardes amendées, et ce que la PR doit dire

`mod.rs:407`, `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` :

- **Moitié 1** (le fichier de garde ne nomme pas `relief.rs` / `reporter.rs` / `mika#2700`) :
  **conservée telle quelle**. Le vol 2 ne touche ni `relief.rs` ni, à ce vol, `reporter.rs`.
  Elle reste verte, et c'est une information : le geste n'a pas été glissé dans le lecteur.
- **Moitié 2** (aucune production ne déclare `concat!("MIKA_MANAGER_", "GROOM_WRITE")`) :
  **inversée**. Elle devient `mika2958_le_vol_2_declare_son_interrupteur_avec_sa_table_de_verite`,
  qui affirme trois choses à la fois, parce que c'est ce que le message de l'ancienne garde
  promettait : (a) `groom_trigger.rs` déclare bien l'interrupteur ; (b) il est **le seul** à le
  déclarer ; (c) le fichier porte aussi `armed_from_value` et `GROOMS_PER_CYCLE`. Le test garde
  `production_sans_commentaires()` comme lecteur, donc de la prose *à propos* du jeton ne la
  satisfait pas (mika issue#2050 signal S).

Le renommage est volontaire : un test nommé `le_vol_1_nouvre_aucun_chemin_decriture` qui atteste
que le vol 2 en ouvre un est illisible à six mois.

**La PR dit ce qui rougissait** — c'est une exigence littérale d'AC1. Le corps de PR portera un
paragraphe nommant : la moitié 2 de `mika2700_…` (la déclaration de l'interrupteur), le pin
`EXEMPTED_OCCURRENCES.len() == 2`, le nom du test d'allowlist, et
`mika2267_every_manager_env_const_is_declared_in_env_example` (déclencheur : toute nouvelle
`pub const ENV_*`, d'où la ligne de `.env.example` dans le même commit).

### 4.4 Sondes — vocabulaire fermé, et une ligne sur la branche zéro

| événement | niveau | champs | quand |
|---|---|---|---|
| `manager_groom_not_triggered` | INFO | `reason`, `candidates` | `disarmed`, `no_sender`, `no_relief` — **y compris la branche OFF nominale** |
| `manager_groom_triggered` | INFO | `issue`, `geste` | un envoi réussi |
| `manager_groom_skipped` | INFO | `issue`, `reason` | `already_groomed`, `groom_in_flight`, `budget_spent` |
| `manager_groom_failed` | WARN | `issue`, `class` | `agent_busy`, `send_failed` |

La ligne sur la branche OFF est délibérée : sans elle, « le manager tourne et ne groome rien » et
« le manager ne tourne pas » rendent les mêmes octets (classe mika issue#2205). `candidates` est
**absent**, jamais `0`, quand la relève n'a pas été mesurée (mika issue#2331). Et l'absence de la
ligne signifie un binaire antérieur à ce vol, pas une absence de geste (mika issue#2340).

### 4.5 `mod.rs` — ré-export et docstring

```rust
pub use groom_trigger::{
    ENV_GROOM_WRITE, GROOMS_PER_CYCLE, GroomCandidate, GroomSendError, GroomSender,
    GroomTriggerOutcome, groom_write_is_armed, trigger_relief_grooms,
};
```

Le paragraphe `mod.rs:256-265` est réécrit : il cesse d'annoncer le vol 2 au futur, et dit ce qui
est livré, ce qui ne l'est pas (le rail et le câblage, renvoyés au ticket de suivi), et que
l'ouverture de l'interrupteur reste un geste opérateur au seuil de mika issue#2700 —
`MIKA_MANAGER_EVALUATE_WRITE` et `MIKA_MANAGER_POOL_WRITE` ouverts et **stables sur un cycle
complet**. Aujourd'hui, les deux sont OFF ; **ce ticket n'en ouvre aucun.**

### 4.6 `.env.example`

Un bloc sur le patron de celui de mika issue#2699 (l. 446) : OFF par défaut, l'absence de la
variable EST l'état normal ; armée (`1` ou `true`), elle autorise exactement un geste — un
`message/send` vers mika-dev — ni spawn ni dispatch d'implémentation ; troisième et dernière
exception nommée à l'invariant LECTURE SEULE, épinglée au fichier et au jeton près par
`no_dispatch_test.rs` ; seuil d'ouverture (geste opérateur, jamais bascule automatique) ; et les
deux sondes de lecture :

```
#   grep manager_groom_triggered "$MIKA_SPIRIT_LOG_FILE"
#   grep manager_groom_not_triggered "$MIKA_SPIRIT_LOG_FILE"   # reason: disarmed | no_sender | no_relief
```

### 4.7 La lecture d'AC2, décidée ici et implémentée au vol 2

**Décision : la lecture est structurelle, sur le rail `gh` que le manager a déjà — trois couches.**

| fait à voir | lecture | d'où elle vient |
|---|---|---|
| déjà groomé | le callout `Plan: docs/plans/` dans le corps du ticket | `reader.rs` le lit déjà (`plan_present` / `plan_reading`), aligné depuis mika issue#2956 sur `check_grooming_markers` |
| groom moteur en cours | le label `dispatch:<siège>` posé par le moteur sur le ticket | `relief.rs` l'exploite déjà : un ticket en groom porte un siège, qui l'a sorti du décompte |
| groom déclenché par le manager au cycle précédent, label pas encore posé | un champ de checkpoint `grooms_declenches`, borné | patron de `posted_flight_task_ids` dans `MilestoneCheckpoint` (`POSTED_FLIGHTS_RETAINED`) |

La troisième couche n'est pas un luxe : la pose du label n'est pas instantanée, donc entre le
`message/send` et le label il existe une fenêtre où les deux premières lectures rendent « pas
groomé, pas en vol ». C'est exactement la collision que le ticket nomme comme motif.

**Pourquoi pas les tâches moteur `Groom …` et `ready-label`, que le ticket propose.** Elles
vivent dans le SQLite que le manager n'atteint pas (§ 1.3). Les lire exigerait une lecture HTTP
authentifiée de l'API tableau de bord (`GET /api/v1/tasks` avec `MIKA_INTERNAL_TOKEN`), c'est-à-dire
un **second lecteur** de « un pilote est-il vivant ? » en face de `live_pilot`, qui est le lecteur
unique de ce fait. C'est la doctrine du lecteur unique (mika issue#2201) qui tranche, pas un coût :
deux réponses à une même question divergent le jour où la réponse compte. Refusé, et nommé ici
pour que le refus ne soit pas à re-découvrir.

### 4.8 Ce que le rapport fera au vol 2 (AC4), figé ici

`run_manager_cycle_in` rend le rapport **après** les aides d'écriture, donc l'issue du geste est
disponible au moment du rendu — il suffit de la faire passer dans l'appel au `Reporter`.

- **OFF** : la section est **inchangée**, titre compris — `### Relève à grommer (mode ombre —
  aucun groom déclenché)`. Les quatre tests du vol 1 de mika issue#2700 (`reporter.rs:2267`,
  `:2362`, `:2404`, `:2436`) restent verts **sans modification**, et c'est la vérification d'AC4
  moitié OFF.
- **ON, en test seulement** : le titre devient `### Relève à grommer` — garder « mode ombre —
  aucun groom déclenché » quand un groom a été déclenché serait un mensonge dans un titre — suivi
  d'une ligne par ticket : `— groom déclenché` ou `— groom sauté : <motif>`, le motif dans le
  vocabulaire fermé de § 4.4.

---

## 5. Ordre de commit du vol 1

Le vol tient en **un seul commit**, et ce n'est pas un raccourci : le docstring de `mod.rs` et la
moitié 2 de `mika2700_…` exigent littéralement que la déclaration de l'interrupteur et l'amendement
des gardes arrivent ensemble. Un commit intermédiaire serait rouge par construction.

1. `groom_trigger.rs` (production + `mod tests`), `mod.rs` (ré-export + docstring + garde
   renommée et inversée), `no_dispatch_test.rs` (jeton + 3ᵉ exception + pin 2→3 + renommage),
   `.env.example`.

**Plancher** (ce qui doit être vert avant la PR) : `cargo test -p mika-agent milestone_manager`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`scripts/verify-pipeline.sh origin/main`.

**Contrainte de structure à ne pas enfreindre :** le module ne peut pas porter de répertoire
`tests/` imbriqué — `perimeter::tests::milestone_manager_has_no_nested_tests_directory` le refuse.
Tous les tests de ce vol sont donc en `#[cfg(test)] mod tests` inline dans `groom_trigger.rs`.

---

## 6. Contrat de vérification

| test | affirme | sans lui |
|---|---|---|
| `la_table_de_verite_de_linterrupteur` | `None`/`""`/`"0"`/`"no"` ⇒ désarmé ; `"1"`/`"true"`/`"TRUE"`/`" 1 "` ⇒ armé | l'interrupteur s'arme sur une valeur arbitraire |
| `off_naucun_envoi_et_son_controle_negatif` | OFF : l'enregistreur a **zéro** appel ; **dans le même test**, ON avec les mêmes candidats : exactement un appel | « zéro appel » et « le harnais n'enregistre rien » rendent les mêmes octets (mika issue#2205) |
| `on_envoie_le_geste_exact` | le corps envoyé est `groom mika#N`, et le geste nommé est `message/send` | une forme voisine passerait, et l'exception nommée serait une fiction |
| `un_seul_groom_par_cycle` | 3 candidats propres ⇒ 1 `declenches`, 2 `sautes` avec `budget_spent` | le manager saturerait le créneau unique du moteur |
| `deja_groome_et_en_vol_ne_sont_pas_relances` | les deux motifs apparaissent, et l'enregistreur n'est pas touché pour eux | AC2 ne serait vérifié que par lecture |
| `agent_busy_arrete_le_cycle_et_ne_compte_pas_comme_declenche` | `Err(AgentBusy)` ⇒ `declenches` vide, `failed` nommé | un groom jamais fait serait rapporté comme fait |
| `le_vocabulaire_des_motifs_est_un_format_de_fil` | les sept motifs, à la chaîne près | un renommage casserait les `GROUP BY` opérateur en silence |
| `mika2958_lallowlist_porte_les_trois_gestes_et_pas_un_de_plus` | 3 entrées, la 3ᵉ nommant `groom_trigger.rs`, `message/send`, `mika#2958`, `MIKA_MANAGER_GROOM_WRITE` | une 4ᵉ écriture entrerait sans son propre seuil |
| `mika2958_le_vol_2_declare_son_interrupteur_avec_sa_table_de_verite` | `groom_trigger.rs` est le **seul** déclarant, et porte `armed_from_value` + `GROOMS_PER_CYCLE` | l'interrupteur pourrait arriver nu, sans sa table |
| `mika2698_le_scan_rougit_sur_une_source_fautive` (existant) | le scan rougit toujours sur une source fautive, jeton `message/send` compris | le resserrement du § 4.2 serait non attesté |

---

## Definition of Done

- [ ] `groom_trigger.rs` livré : `ENV_GROOM_WRITE`, `groom_write_is_armed` / `armed_from_value`,
      `const GESTE = "message/send"` à plat, `GroomSender` + `GroomSendError`, `GROOMS_PER_CYCLE = 1`,
      `trigger_relief_grooms` avec ses six refus ordonnés, les quatre sondes.
- [ ] `armed` traverse l'API en **paramètre** ; aucun `unsafe { set_var }` dans les tests.
- [ ] `"message/send"` ajouté à `FORBIDDEN_TOKENS` ; le scan est **resserré**, jamais assoupli.
- [ ] 3ᵉ entrée dans `EXEMPTED_OCCURRENCES`, nommant fichier, jeton, ticket et variable ; pin
      `len() == 3` ; test d'allowlist renommé et son message réécrit.
- [ ] `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` amendé **dans ce commit** : moitié 1
      conservée, moitié 2 inversée et renommée.
- [ ] docstring `mod.rs:256-265` réécrit ; ré-exports ajoutés.
- [ ] `.env.example` porte le bloc `MIKA_MANAGER_GROOM_WRITE` commenté (OFF), avec son seuil
      d'ouverture et ses deux sondes `grep` —
      `mika2267_every_manager_env_const_is_declared_in_env_example` vert.
- [ ] Les dix tests du § 6 verts, dont le contrôle négatif OFF/ON **dans le même test**.
- [ ] Les quatre tests de rapport du vol 1 de mika issue#2700 verts **sans modification**.
- [ ] `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
      `scripts/verify-pipeline.sh origin/main` verts.
- [ ] Le corps de PR nomme **ce qui rougissait** : la moitié 2 de `mika2700_…`, le pin `len() == 2`,
      et la garde `.env.example`.
- [ ] Le corps de PR porte la ligne `Tracked in:` du vol 2, et dit explicitement que
      **ce ticket n'ouvre aucun interrupteur**.

## Acceptance criteria

Transcrits verbatim de senara-solutions/mika#2958 :

- **AC1.** Avec l'interrupteur OFF, aucun groom n'est déclenché. C'est une garantie structurelle,
  avec un contrôle négatif dans le même test. Le geste entre comme **exception nommée** au scan de
  `no_dispatch_test.rs`, sur le patron des entrées de mika#2698 vol 2 et de mika#2699 vol 4. Le
  scan n'est pas assoupli. Le test `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` est amendé
  dans le même commit, et la PR dit ce qui le faisait rougir.
- **AC2.** Sélection et idempotence : un ticket déjà groomé, ou dont un groom est en cours, n'est
  pas relancé. Le plan dit par quelle lecture le manager voit un groom en cours : les tâches
  moteur `Groom …` et `ready-label`, ou autre chose. Le motif est la collision d'un groom manuel
  avec un groom moteur.
- **AC3.** La voie est celle du moteur : la même que `mika ask --agent mika-dev "groom mika#N"`, ou
  son équivalent interne. Elle n'ouvre ni spawn ni dispatch d'implémentation. Le plan dit comment
  le geste reste borné : combien de grooms par cycle, et ce qu'il fait quand mika-dev a déjà un
  pilote en vol, puisque le moteur n'en fait tourner qu'un à la fois.
- **AC4.** Ce qui reste de l'ombre se lit dans le rapport. Interrupteur OFF : la section « Relève à
  grommer » est inchangée. Interrupteur ON, en test seulement : le rapport dit quels grooms ont été
  déclenchés, et lesquels ont été sautés, avec leur motif.
- **Taille** : à estimer tests compris, et à découper en vols au-delà de la borne.

**Couverture par vol.** AC1 est entièrement dans ce vol. AC2 et AC3 y ont leur **décision écrite**
(§ 4.7, § 4.1) et leurs prédicats et constantes livrés et testés ; leur lecture et leur rail sont
au vol 2. AC4 y a sa forme figée (§ 4.8) et sa moitié OFF **déjà attestée** par les quatre tests
existants laissés intacts ; sa moitié ON est au vol 2.

## Fire-Disposition

Ce plan livre des détecteurs : le scan structurel de `no_dispatch_test.rs` **resserré** d'un jeton
(`message/send`), la garde `mika2958_le_vol_2_declare_son_interrupteur_avec_sa_table_de_verite`, et
le pin d'allowlist à trois entrées.

**Option retenue : (a) exception nommée en allowlist — et le détecteur arrive propre.**

- **Violations préexistantes : zéro, mesuré.**
  `grep -rn "message/send" crates/mika-agent/src/milestone_manager/` ne rend rien sur `74e7254f`.
  Le nouveau jeton n'a donc **aucune** violation à amnistier, et aucune entrée d'allowlist n'est
  créée pour couvrir de l'existant. C'est le cas le plus favorable de mika issue#1574 : le
  détecteur est armé dès son arrivée, sans dette.
- **La seule exception créée est le geste de ce vol lui-même**, et elle nomme la donnée précise
  — le fichier `groom_trigger.rs`, le jeton `message/send` — plus le ticket (`mika#2958`) et la
  variable (`MIKA_MANAGER_GROOM_WRITE`) dans sa raison, grep-visibles tous les quatre.
- **L'assertion auto-nettoyante est déjà écrite et couvre la nouvelle entrée sans ligne
  supplémentaire** : la boucle finale de l'allowlist lit chaque fichier nommé, le tronque à son
  premier `#[cfg(test)]`, et **panique** si le jeton n'y est plus — « une dérogation sans site est
  un trou permanent dans le grep ». Le geste qui rend l'exception inutile (retirer le groom) est
  donc le geste qui la fait expirer : mika issue#2309, et non l'allowlist que mika issue#2309
  condamne.
- **Rien n'est livré désarmé** : pas de `#[ignore]`, pas de `#[cfg(skip)]`. Le contrôle de bonne
  foi existant (`mika2698_le_scan_rougit_sur_une_source_fautive`) est étendu au nouveau jeton, donc
  « le scan voit le jeton » est attesté et non supposé.
- **Aucune halte n'est requise** : rien dans ce vol n'ouvre d'interrupteur, et le seuil
  d'ouverture reste un geste opérateur.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/groom_trigger.rs` — production | 150 |
| `crates/mika-agent/src/milestone_manager/groom_trigger.rs` — `#[cfg(test)] mod tests` | 140 |
| `crates/mika-agent/src/milestone_manager/no_dispatch_test.rs` — jeton, 3ᵉ exception, pin 2→3, renommage | 35 |
| `crates/mika-agent/src/milestone_manager/mod.rs` — ré-exports, docstring, garde renommée et inversée | 50 |
| `.env.example` — bloc `MIKA_MANAGER_GROOM_WRITE` | 20 |

Total estimé : 395 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient.** 395 < 429, avec 34 lignes de marge. Le vol est contenu dans un seul
module nouveau plus trois amendements ponctuels, sans câblage de cycle, sans rail HTTP et sans
rendu de rapport — les trois postes qui portent l'essentiel du reste.

**Le total du ticket entier, pour mémoire :** 765 lignes estimées. Le vol 2 en porte 370 — rail
HTTP `A2aGroomSender` et ses deux constantes (60), câblage dans `cadence.rs` (120), lecture
« groom en cours » et champ de checkpoint (100), rendu AC4 dans `reporter.rs` (90) — ce qui le
place lui aussi sous la borne, et ne demandera donc pas un troisième découpage.

**Tracked in:** ticket de suivi à ouvrir après merge de ce vol — *« feat(manager) : Phase 2 (c)
vol 2/2 — le rail A2A du groom, son câblage de cycle et son rendu de rapport (suite de
mika issue#2958) »*, portant AC2 (la lecture de § 4.7), AC3 (le rail et son refus `agent_busy`) et
AC4 moitié ON (§ 4.8).
