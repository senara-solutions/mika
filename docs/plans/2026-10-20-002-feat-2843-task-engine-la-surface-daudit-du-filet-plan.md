# mika#2843 — La surface d'audit du filet (vol 3 de mika#2456)

**Ticket :** senara-solutions/mika#2843
**Amont :** mika#2456 (le défaut), mika#2841 (vol 2, livré par #2844)
**Plan du vol 2 :** `docs/plans/2026-10-20-001-fix-2841-task-engine-mika-2456-vol-2-filet-au-tir-plan.md`
**Branche :** `feat/2843/task-engine-la-surface-d-audit-du-filet`

---

## Contexte

mika#2456 se livre en trois vols, et les deux premiers sont sur `main` :

| vol | PR | ce qu'il porte |
|---|---|---|
| 1 | #2835 | les sondes de la porte à l'**enregistrement** (`Identity::recurring_tasks_enabled`, blocage dans `ensure_recurring_task_with_policy`) |
| 2 | #2844 | le **filet au tir** — `refuse_recurring_fire_if_agent_disabled`, appelé en tête de `dispatch_run_skill` |
| 3 | *ce vol* | la **surface d'audit** de ce filet |

Le filet existe parce que la porte à l'enregistrement ne voit pas deux
populations : les lignes **nées avant** son déploiement, et les bascules
**à chaud** de `enabled`. Il est aujourd'hui en place
(`crates/mika-agent/src/task_engine/dispatcher.rs:743-761`) et il refuse
correctement — mais il ne refuse que dans les **logs** :

```rust
warn!(
    target: "mika::otel",
    event = RECURRING_FIRE_REFUSED_EVENT,   // "recurring_fire_refused_agent_disabled"
    agent_id = %self.db.agent_id(),
    label = %task.label,
    trigger_type = %task.trigger_type,
    "mika#2456: agent carries `enabled = false` — recurring fire refused …"
);
true
```

Conséquence mesurable : la requête du § 5 du plan amont ne renvoie **rien**.

```sql
SELECT target_key, after_value, count(*)
FROM audit_events
WHERE tool_name = 'agent_recurring_gate'
GROUP BY 1, 2;
```

Un `count(*)` vide ne dit pas « aucun refus » — il dit « aucun vol n'écrit
encore cette ligne » (**doctrine mika#2331, absent n'est pas zéro**). Et il n'y
a aujourd'hui aucune documentation opérateur qui promette cette requête : le
plan du vol 2 interdisait explicitement d'en écrire une avant ce vol-ci. C'est
la bonne discipline — *une requête documentée qui rend zéro sur une table que
personne n'écrit est un instrument qui ment*.

**Pourquoi un vol séparé et non une phase du vol 2.** Le mécanisme de
déduplication est indivisible du mécanisme d'audit : sans lui, une récurrente à
la minute écrit ≈ 1 440 lignes `audit_events` par jour pour un seul agent
désactivé. Le couple pesait ≈ 110 lignes au-dessus de la borne par vol du
ticket amont, d'où la découpe en **vols** (mika#2693) — chacun mergeable seul.

**Matière déjà écrite.** La wip `backup/2456-wip` (`d9b2f4eb`, 2026-09-21) porte
la matière de cet étage : `.pilot-scratch/wip-dispatcher.rs:129-196` (les
constantes, le `static`, les deux fonctions), `:694-728` (le bloc d'écriture),
`:4286-4342` (V4c et `consulting_does_not_mark`), `:4344-4387` (un scan
SOLE-WRITER inline). Le § *Corrections à la wip* ci-dessous dit ce qui s'y
transcrit tel quel et ce qui change.

---

## Périmètre — ce que CE vol livre

1. **`RECURRING_GATE_AUDIT_TOOL_NAME = "agent_recurring_gate"`** et son contrat
   de **SOLE WRITER** en doc-comment, nommant le test qui le tient.
2. **`RECURRING_GATE_AUDIT_REFRESH`** — la fenêtre de rafraîchissement de 24 h.
3. **`RECURRING_GATE_AUDIT_SEEN_CAP = 10_000`** — le cap de mémoire de la table
   de déduplication.
4. Le **`static LazyLock<Mutex<HashMap<(String, String), Instant>>>`** transcrit
   de `auto_pull.rs::EXCLUSION_AUDIT_SEEN` (mika#2131), **fail-open sur mutex
   empoisonné** (`unwrap_or_else(|p| p.into_inner())`).
5. **`recurring_gate_audit_is_due`** / **`mark_recurring_gate_audited`** —
   consulter et marquer sont deux gestes séparés, et c'est la propriété que
   `mika2456_consulting_does_not_mark` tient.
6. Le **bloc d'écriture** dans la branche de refus de
   `refuse_recurring_fire_if_agent_disabled` : WARN **et** ligne d'audit sous un
   seul prédicat (R-9 du plan amont).
7. La sonde **V4-bis** — le refus par la **ligne d'audit** (le vol 2 ne tient V4
   que par le booléen rendu et par le log).
8. La sonde **V4c** — la fenêtre de déduplication : trois assertions à `t0`,
   `t0 + 23 h`, `t0 + REFRESH + 1 s`.
9. La sonde **`mika2456_consulting_does_not_mark`**.
10. Le **scan source SOLE-WRITER** sur `tool_name = 'agent_recurring_gate'`,
    allowlist **née vide** (mika#2323), dans `canonical_tokens.rs`.
11. La **documentation opérateur** de la requête ci-dessus, dans
    `crates/mika-agent/CLAUDE.md` § *Audit Log*, sur le modèle exact de
    `auth_boundary`. Elle n'a le droit d'exister **qu'avec ce vol**.

## Périmètre — ce que CE vol ne livre PAS

**La conjonction au tir réel avec la porte de section heartbeat.** Le corps du
ticket la range ici avec cette justification : *« demande de piloter un
`TaskEngine` complet sur une boucle de ticks. Les sondes de la surface d'audit
paient déjà ce harnais. »* **Les deux moitiés de cette phrase sont fausses, et
c'est la mesure de ce vol qui le dit** — donc elles sont écrites ici pour que
personne ne rouvre le sujet sur la prémisse d'origine :

- **Les sondes de ce vol ne paient pas ce harnais.** La wip le payait
  (`.pilot-scratch/wip-eval-2456.rs`, ≈ 95 lignes de harnais : `NoopSender`,
  un littéral `TaskDispatcher` à 22 champs, 120 ticks de 50 ms). `main` n'en a
  plus besoin : le vol 2 a livré un harnais **inline** dans
  `dispatcher.rs:5275-5399` (`test_db`, `test_dispatcher_with_homes`,
  `agent_home`, `home_with_enabled`, `seeded_task`), et les sondes de ligne
  d'audit de mika#2496 (`dispatcher.rs:6577-6660`) montrent le modèle bon
  marché : appeler la fonction, puis lire `count_audit_events_by_tool_name` et
  `get_audit_events`. **Aucun `TaskEngine`.** C'est le levier que le plan du vol
  2 nommait déjà : *« le levier décisif n'est pas un retrait de fonctionnalité
  mais un retrait de harnais ».*
- **Une boucle de ticks ne prouverait rien sur cette conjonction.**
  `heartbeat_enabled_for_agent` a **exactement un** lecteur de production,
  `server/mod.rs:1906`, et il agit à l'**enregistrement** : la branche `else`
  appelle `cancel_recurring_task_by_label("heartbeat")`.
  `dispatch_heartbeat` (`dispatcher.rs:1768`) ne le consulte **jamais**. Donc
  « une récurrente `heartbeat` effectivement non tirée sous `[heartbeat] enabled
  = false` » n'est pas un fait du **tir** : c'est un fait de
  l'**enregistrement**, et la forme qui le démontre est **structurelle** (un
  scan de lecteur unique), pas comportementale.

Le suivi porte donc la forme corrigée — un scan « la porte de section est
registration-only par construction », plus la sonde comportementale de
l'annulation à l'enregistrement — et non la boucle de ticks.

`Tracked in:` ticket de suivi à ouvrir à la livraison de ce vol, vol 4 de
mika#2456 : *« la porte de section heartbeat est registration-only par
construction »* — scan de lecteur unique sur `heartbeat_enabled_for_agent`
(allowlist née vide) + sonde comportementale sur la branche `else` de
`server/mod.rs:1906`. Sort de ce vol pour **volume** (≈ 40 lignes au-dessus de
la borne, cf. § *Taille estimée*), et sa prémisse est réécrite ci-dessus.

Également hors périmètre, et déjà hors de celui du vol 2 — inchangé :

- Le remplacement du filet par une mutation de ligne (annuler / suspendre la
  récurrente au tir). **Doctrine mika#2156** (`phantom_sweep_spared`) : un filet
  qui mute efface sa propre preuve. Le filet laisse la ligne telle quelle ; le
  tir reprend le tick suivant le retour de `enabled` à `true`.
- Toute extension de la porte aux autres `trigger_type`.

---

## Corrections à la wip, au-delà des trois du vol 2

Le plan du vol 2 relevait trois corrections. Deux sont consommées par le vol 2
(la signature `-> Result<usize>`, le résidu de débogage `if false && …` de
`.pilot-scratch/wip-dispatcher.rs:684`). La troisième vaut encore ici : **aucune
plomberie n'est nécessaire** — `TaskDispatcher` porte déjà `home_dir` et
`global_home_dir`, et `super::agent_recurring_tasks_enabled` est déjà appelé
par le filet.

Ce vol en ajoute **deux**, toutes deux sur le bloc d'écriture.

### C1 — `session_id` est un sentinel fixe, pas un identifiant par tâche

La wip écrit `&format!("{RECURRING_GATE_AUDIT_TOOL_NAME}-{}", task.id)`. Un
`session_id` qui porte l'id de tâche rend `get_audit_events(session_id)`
inutilisable : il n'existe plus de session sous laquelle **lire la population**.
Le précédent est `auth_boundary`, documenté dans
`crates/mika-agent/CLAUDE.md` § *Audit Log* en ces termes : *« `session_id` :
`auth-boundary` — a boundary failure belongs to no conversation »*. Un refus de
porte n'appartient à aucune conversation non plus.

→ `session_id = "agent-recurring-gate"` (constante
`RECURRING_GATE_AUDIT_SESSION_ID`). L'id de tâche reste porté par `reasoning`
(`task_id=<n>`), où il est une donnée et non une clé de regroupement.

*Note pour le scan du § 10 : ce littéral est en tirets, le `tool_name` en
tirets bas — deux littéraux distincts, et la comparaison du scan est **exacte**
(cf. ci-dessous).*

### C2 — l'`agent_id` du `target_key` vient de `self.db.agent_id()`

La wip construit sa clé depuis `task.agent_id`. Le WARN déjà en place, lui, lit
`self.db.agent_id()`. **R-9 du plan amont exige que les deux surfaces se
soustraient** : un WARN et une ligne d'audit qui nomment des agents différents
ne sont pas comparables, et le `GROUP BY target_key` compterait une population
que le log ne sait pas désigner.

→ une seule source, `self.db.agent_id()`, pour la clé de déduplication, pour le
`target_key` et pour le WARN.

### C3 — à dire fort : le WARN devient dédupliqué, lui aussi

Aujourd'hui le WARN est **inconditionnel**. Le mettre sous `if is_due` —
nécessaire, parce que R-9 veut **un seul prédicat** pour les deux surfaces —
change la sémantique de son absence :

> Après ce vol, l'absence de `recurring_fire_refused_agent_disabled` dans les
> logs signifie **« déjà annoncé dans les 24 h »**, et non « aucun refus ».

C'est un changement de contrat d'un instrument existant. Il va dans le
doc-comment du prédicat **et** dans le § *Audit Log* du `CLAUDE.md` de la crate.
L'alternative — garder le WARN inconditionnel — est refusée : elle rendrait
≈ 1 440 lignes de log par jour et par agent désactivé, et ferait du log et de la
table deux populations non soustractibles (classe mika#2205).

---

## Implémentation — ordre de commit

### Commit 1 — le mécanisme (`dispatcher.rs`)

Juste après `RECURRING_FIRE_REFUSED_EVENT` (`:100-109`), transcrire
`.pilot-scratch/wip-dispatcher.rs:129-196` :

- `RECURRING_GATE_AUDIT_TOOL_NAME: &str = "agent_recurring_gate"` avec le
  doc-comment **SOLE WRITER** nommant
  `mika2456_the_gate_audit_name_has_exactly_one_writer`.
- `RECURRING_GATE_AUDIT_SESSION_ID: &str = "agent-recurring-gate"` (C1), avec
  la raison en doc-comment.
- `RECURRING_GATE_AUDIT_REFRESH: Duration = Duration::from_secs(24 * 60 * 60)`.
- `RECURRING_GATE_AUDIT_SEEN_CAP: usize = 10_000`.
- `static RECURRING_GATE_AUDIT_SEEN: LazyLock<Mutex<HashMap<(String, String),
  Instant>>>` — `use std::sync::LazyLock;` est déjà présent (`:9`).
- `fn recurring_gate_audit_is_due(key, now) -> bool` et
  `fn mark_recurring_gate_audited(key, now)`, **fail-open** sur mutex
  empoisonné via `unwrap_or_else(|p| p.into_inner())`, et vidage du cap avec un
  WARN nommant `cap`.

Le doc-comment du `static` porte trois choses que la transcription seule perd :
l'état est **en mémoire**, donc **perdu au redémarrage par conception** (le
premier refus après un restart réécrit une ligne — c'est voulu, un restart est
une information) ; consulter n'est **pas** marquer ; et le fail-open est un
choix — *un audit qui ne peut pas se dédupliquer écrit plutôt que de refuser le
refus*.

### Commit 2 — le bloc d'écriture (`dispatcher.rs:743-761`)

Dans la branche de refus, remplacer le `warn!` nu par, en transcrivant
`.pilot-scratch/wip-dispatcher.rs:694-728` avec C1 et C2 :

```rust
let agent_id = self.db.agent_id().to_string();
let key = (agent_id.clone(), task.label.clone());
let now = std::time::Instant::now();
if recurring_gate_audit_is_due(&key, now) {
    warn!( /* … inchangé … */ );
    match self.db.log_audit_event(
        RECURRING_GATE_AUDIT_SESSION_ID,
        RECURRING_GATE_AUDIT_TOOL_NAME,
        &format!("{agent_id}@{}", task.label),
        None,
        Some("refused_agent_disabled"),
        Some(&format!("task_id={}", task.id)),
        None,
    ).await {
        Ok(()) => mark_recurring_gate_audited(key, now),
        Err(e) => warn!(target: "mika::otel", error = %e, "…"),
    }
}
true
```

Deux propriétés que la forme porte et qu'un refactor casserait :

- **On marque après que l'écriture a atterri.** `Ok(()) => mark…`. Marquer
  avant, c'est perdre silencieusement la ligne et taire le refus 24 h.
- **Une écriture d'audit en échec ne change pas le verdict.** `true` est rendu
  hors du `match` : le refus reste un refus. Même forme que le
  *fire-and-forget* d'`AuthBoundaryLedger`.

### Commit 3 — V4-bis (`dispatcher.rs`, `mod tests`)

`mika2456_v4bis_le_refus_ecrit_sa_ligne_daudit`. Modèle : les sondes mika#2496
de `:6577-6660` — pas de `TaskEngine`.

1. `test_db()`, `home_with_enabled(false)`, `seeded_task(&db, …,
   trigger_type::RECURRING)`, `test_dispatcher_with_homes(…)`.
2. Appeler `refuse_recurring_fire_if_agent_disabled(&task)` ; assertion sur le
   `bool` (le contrat du vol 2 reste tenu).
3. `db.count_audit_events_by_tool_name(RECURRING_GATE_AUDIT_TOOL_NAME) == 1`.
4. `db.get_audit_events(RECURRING_GATE_AUDIT_SESSION_ID)` : une ligne,
   `after_value == Some("refused_agent_disabled")`, `target_key ==
   "<agent>@<label>"`, `reasoning` contenant `task_id=`, `before_value ==
   None`.

**Le `static` est global au binaire de test — ce test prend un `label`
distinct.** `RECURRING_GATE_AUDIT_SEEN` est un `static` partagé par tout le
binaire. Après ce vol, le V4 du vol 2
(`mika2456_le_filet_refuse_une_recurrente_dun_agent_desactive`, agent `"mika"`,
label `"heartbeat"`) écrira **lui aussi** une ligne, sous la clé
`("mika", "heartbeat")`. Deux tests qui partagent la clé se dédupliquent l'un
l'autre et lisent `0` ou `1` **selon l'ordre d'exécution**. V4-bis, V4c et
`consulting_does_not_mark` prennent donc chacun un label **unique**
(`"v4bis-gate-audit"`, `"v4c-dedup-window"`, `"v4c-consult-only"`), et le
commentaire du test dit pourquoi — c'est la seule chose qui empêche un futur
éditeur de « simplifier » vers un label partagé.

### Commit 4 — V4c et `consulting_does_not_mark` (`dispatcher.rs`, `mod tests`)

Transcription de `.pilot-scratch/wip-dispatcher.rs:4286-4342`.

- `mika2456_v4c_the_refusal_is_deduplicated_on_a_24h_window` — trois
  assertions sur `recurring_gate_audit_is_due` / `mark_recurring_gate_audited`
  avec un `Instant` injecté : `t0` dû, `t0` marqué puis `t0 + 23 h` **non** dû,
  `t0 + REFRESH + 1 s` dû. L'`Instant` est un **paramètre**, pas
  `Instant::now()` — c'est ce qui rend la fenêtre de 24 h testable sans
  attendre 24 h, et c'est pour cela que les deux fonctions le prennent.
- `mika2456_consulting_does_not_mark` — deux `is_due` consécutifs sans `mark`
  rendent `true` tous les deux.

**Le cap de 10 000 n'est pas testé en remplissant la table.** `seen.clear()` est
un effet de bord sur un `static` partagé : un test qui le déclenche viderait la
table de déduplication des tests parallèles et les rendrait flaky. Le cap est
tenu par sa lecture — la constante, le `>=`, le WARN qui nomme `cap` — et ce
refus est écrit dans le commentaire du test, pour qu'il ne soit pas relu comme
un oubli.

### Commit 5 — le scan SOLE-WRITER (`canonical_tokens.rs`) — *plancher*

Sur le modèle de mika#2634 (`:1499-1660`), **en réutilisant** le prédicat déjà
extrait `audit_name_literal_present(content, needle)` (`:1529`) — mika#2653 :
*un prédicat recopié est un prédicat qui peut diverger de son propre contrôle de
bonne foi*.

- `RECURRING_GATE_SOLE_WRITER_EXCEPTIONS: &[&str] = &[]`.
- `mika2456_the_gate_audit_name_has_exactly_one_writer` : needle **composé à
  l'exécution** (`format!("agent_recurring{}gate", "_")` ou équivalent) pour que
  le fichier scannant ne se dénonce pas lui-même (mika#2496) ; boucle sur
  `production_sources()` ; saut de l'allowlist ; **anti-vacuité sur le
  propriétaire** (`dispatcher.rs` doit être trouvé — mika#2103 / mika#2205) ;
  `strangers.is_empty()` avec un message de RÉSOLUTION qui **refuse**
  l'allowlist : *« on retire le site, on ne l'allowliste pas »* (mika#2201).
- `mika2456_lallowlist_du_scan_dagent_recurring_gate_est_livree_vide` — le
  pendant auto-nettoyant.

**La comparaison est exacte sur le littéral entier, et c'est ce qui rend
l'allowlist livrable vide.** `audit_name_literal_present` teste
`lit.trim() == needle`. Deux littéraux existants contiennent la sous-chaîne
`agent_recurring_gate` sans être des écritures de cette ligne :
`task_engine/mod.rs:77` (`AGENT_RECURRING_GATE_RESOLVED_EVENT =
"agent_recurring_gate_resolved"`) et, après C1, le sentinel de session
`"agent-recurring-gate"`. Avec un prédicat en `contains`, le scan serait **rouge
au premier jour** et l'allowlist naîtrait à deux entrées — contredisant
mika#2323. Avec l'égalité exacte, le seul site trouvé est la déclaration de la
constante dans `dispatcher.rs`. Cette phrase va dans le commentaire du test.

**Pas de nouveau contrôle de bonne foi.** `audit_name_literal_present` en a déjà
un, `mika2634_the_sole_writer_scan_still_catches_a_second_site`, qui couvre ses
trois formes bénignes dont un nom d'événement de log préfixé. Le prédicat est
partagé, donc son attestation l'est aussi ; en recopier une troisième serait la
duplication que mika#2653 condamne.

### Commit 6 — la documentation opérateur (`crates/mika-agent/CLAUDE.md`)

Dans § *Audit Log*, une sous-section
`### tool_name = 'agent_recurring_gate'` calquée sur celle d'`auth_boundary` :
le tableau de forme de ligne (les six colonnes, `before_value` toujours `NULL`),
la requête SQL, les deux propriétés load-bearing (*fire-and-forget* ;
déduplication 24 h **en mémoire, perdue au redémarrage**), et la phrase de C3 —
l'absence de WARN signifie désormais « déjà annoncé dans les 24 h ».

Hors du compte de lignes (`docs/` et `CLAUDE.md` sont exclus), mais **pas hors
contrainte** : `crates/mika-common/tests/claude_md_size.rs` plafonne tout
`CLAUDE.md` à 100 000 octets et celui-ci en fait 85 462 — ≈ 14 KB de marge, la
sous-section en pèse ≈ 1,5.

### Plancher

Si le pilote chauffe, **le commit 5 tombe en dernier** et part avec le vol 4 par
une ligne `Tracked in:` ajoutée au corps de la PR. Les commits 1 à 4 sont le
plancher : ils livrent la ligne d'audit, sa déduplication et les sondes qui les
tiennent — c'est-à-dire la raison d'être du ticket. Le commit 6 accompagne
toujours le commit 2 : une table écrite sans requête documentée est un
instrument que personne ne sait lire, et une requête documentée sans table est
un instrument qui mente (mika#2331) — les deux se tiennent, dans cet ordre.

---

## Verification Contract

| # | vérification | commande |
|---|---|---|
| 1 | la suite de la crate passe | `cargo test -p mika-agent` |
| 2 | les sondes du vol 3 nommément | `cargo test -p mika-agent mika2456` |
| 3 | les scans structurels | `cargo test -p mika-agent canonical_tokens` |
| 4 | le plafond des `CLAUDE.md` | `cargo test -p mika-common claude_md_size` |
| 5 | clippy sans avertissement | `cargo clippy --all-targets -- -D warnings` |
| 6 | format | `cargo fmt --check` |
| 7 | pipeline | `scripts/verify-pipeline.sh origin/main` |

**Vérification manuelle, une seule, et elle n'est pas facultative :** lancer la
suite **deux fois** (`cargo test -p mika-agent mika2456` puis
`cargo test -p mika-agent mika2456 -- --test-threads=1`). Le `static` partagé
est le seul risque de flake de ce vol, et un ordre d'exécution différent est ce
qui le révèle. Si un compte d'audit diffère entre les deux passes, c'est une
collision de clé : chercher un label partagé entre deux tests.

---

## Fire-Disposition

Ce vol livre des détecteurs : le scan source SOLE-WRITER du commit 5, et les
assertions anti-vacuité qui l'accompagnent.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist naît
vide.**

- `RECURRING_GATE_SOLE_WRITER_EXCEPTIONS: &[&str] = &[]`, grep-visible,
  déclarée vide au moment de la livraison.
- Elle est vide **par mesure, pas par espoir** : le seul site de production
  portant le littéral exact `"agent_recurring_gate"` est la déclaration de la
  constante dans `dispatcher.rs`, et les deux littéraux voisins
  (`"agent_recurring_gate_resolved"` à `task_engine/mod.rs:77`,
  `"agent-recurring-gate"` le sentinel de session) ne sont pas comptés parce que
  la comparaison est exacte — cf. commit 5.
- **Assertion auto-nettoyante :**
  `mika2456_lallowlist_du_scan_dagent_recurring_gate_est_livree_vide` rougit dès
  qu'une entrée y est ajoutée, sur le modèle des ≈ 25 pendants déjà en place
  dans `canonical_tokens.rs`.
- **Anti-vacuité :** le scan assert que `dispatcher.rs` **est** trouvé. Un scan
  qui ne trouve plus son propriétaire est vert pour la mauvaise raison
  (mika#2103 / mika#2205).
- **Message de résolution :** le `assert!` nomme la gestion attendue d'un second
  site — *router l'écriture par le propriétaire, ne pas ajouter d'entrée*
  (mika#2201). Un second écrivain casserait le `GROUP BY` de la requête
  opérateur, qui ne compterait plus une seule population.

Aucun détecteur de ce vol n'atterrit désarmé, et aucun ne demande de cadrage
opérateur : les options (b) et (c) ne sont pas retenues.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/task_engine/dispatcher.rs` — constantes, `static`, `is_due` / `mark` | 70 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — bloc d'écriture dans la branche de refus | 30 |
| tests (`dispatcher.rs` `mod tests`) — V4-bis | 45 |
| tests (`dispatcher.rs` `mod tests`) — V4c + `consulting_does_not_mark` | 48 |
| `crates/mika-agent/src/canonical_tokens.rs` — scan SOLE-WRITER, allowlist, vacuité | 55 |

Total estimé : 248 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et ce qui a été sorti pour qu'il tienne.** 248 contre
279 : marge de 31 lignes. Le périmètre nominal du ticket en comptait ≈ 288 —
le scan « lecteur unique de `heartbeat_enabled_for_agent` » + sa sonde
comportementale pesant ≈ 40 lignes de plus. Il est sorti en vol 4, nommé par
le `Tracked in:` du § *Périmètre — ce que CE vol ne livre PAS*, **avec sa
prémisse réécrite** : ce n'est pas une boucle de ticks qu'il demande, c'est un
scan structurel. L'architecte juge le périmètre de **ce** vol, pas la somme.

Trois choses **ne coûtent rien** parce que `main` les porte déjà, et c'est ce qui
ramène 358 lignes de wip à 248 :

- le harnais de test (`:5275-5399`, livré par le vol 2) — la wip en portait
  ≈ 95 lignes à elle seule, dont un littéral `TaskDispatcher` à 22 champs ;
- `audit_name_literal_present` (`canonical_tokens.rs:1529`) et son contrôle de
  bonne foi, réutilisés tels quels ;
- `use std::sync::LazyLock` (`dispatcher.rs:9`), `log_audit_event` et
  `count_audit_events_by_tool_name` (`async_db.rs:2576`, `:1224`).

Le commit 6 (documentation opérateur) est hors du compte par la règle
(`CLAUDE.md` exclu) ; il pèse ≈ 45 lignes de markdown.

---

## Definition of Done

- [ ] `RECURRING_GATE_AUDIT_TOOL_NAME`, `RECURRING_GATE_AUDIT_SESSION_ID`,
      `RECURRING_GATE_AUDIT_REFRESH`, `RECURRING_GATE_AUDIT_SEEN_CAP` et le
      `static RECURRING_GATE_AUDIT_SEEN` sont en place, le doc-comment du
      `tool_name` nommant son test de SOLE WRITER.
- [ ] `recurring_gate_audit_is_due` et `mark_recurring_gate_audited` existent,
      prennent un `Instant` en paramètre, et sont **fail-open** sur mutex
      empoisonné.
- [ ] La branche de refus de `refuse_recurring_fire_if_agent_disabled` écrit une
      ligne `audit_events` sous `tool_name = 'agent_recurring_gate'`, sous **le
      même prédicat** que son WARN, et marque la clé **après** que l'écriture a
      réussi.
- [ ] Une écriture d'audit en échec laisse le verdict du filet inchangé
      (`true`) et émet un WARN.
- [ ] V4-bis, V4c et `consulting_does_not_mark` passent, chacune sur un `label`
      **unique**, avec le commentaire disant pourquoi.
- [ ] Le scan SOLE-WRITER passe, son allowlist est vide, son assertion
      d'anti-vacuité trouve `dispatcher.rs`, et son pendant auto-nettoyant
      existe.
- [ ] Le V4 du vol 2 passe toujours — ce vol ne change pas son contrat de
      booléen.
- [ ] `crates/mika-agent/CLAUDE.md` § *Audit Log* documente la requête, la forme
      de ligne, et le changement de sémantique de l'absence du WARN.
- [ ] Les sept vérifications du Verification Contract passent, et la suite
      `mika2456` passe en parallèle **et** en `--test-threads=1`.
- [ ] Le corps de la PR porte une ligne `Tracked in:` pour le vol 4.

## Acceptance criteria

*mika#2843 ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son § Périmètre et du Verification Contract
ci-dessus.*

- **AC1** — Un refus du filet sur un agent `enabled = false` écrit **exactement
  une** ligne dans `audit_events` avec `tool_name = 'agent_recurring_gate'`,
  `session_id = 'agent-recurring-gate'`, `target_key = '<agent_id>@<label>'`,
  `after_value = 'refused_agent_disabled'`, `reasoning` contenant
  `task_id=<id>`, et `before_value = NULL`. Tenu par V4-bis.
- **AC2** — Un second refus **sur la même clé** `(agent_id, label)` dans les
  24 h n'écrit **pas** de seconde ligne ; un refus au-delà de la fenêtre en
  écrit une. Tenu par V4c, à `t0`, `t0 + 23 h`, `t0 + REFRESH + 1 s`.
- **AC3** — Consulter la fenêtre ne la marque pas : deux
  `recurring_gate_audit_is_due` consécutifs sans `mark` rendent tous deux
  `true`. Tenu par `mika2456_consulting_does_not_mark`.
- **AC4** — Un mutex empoisonné ne fait pas perdre le refus : le code reprend
  l'état par `into_inner()` et écrit. Tenu par lecture du site + doc-comment ;
  aucun test n'empoisonne un `static` partagé, et ce refus est écrit.
- **AC5** — `tool_name = 'agent_recurring_gate'` a **exactement un** écrivain
  dans les sources de production. Tenu par
  `mika2456_the_gate_audit_name_has_exactly_one_writer`, dont l'allowlist est
  vide et pinnée vide.
- **AC6** — L'`agent_id` du `target_key` et celui du WARN proviennent de la
  **même** source (`self.db.agent_id()`), de sorte que log et table se
  soustraient (R-9).
- **AC7** — Une écriture d'audit en échec ne change pas le verdict du filet : la
  fonction rend `true` et le refus reste un refus.
- **AC8** — La requête opérateur du corps du ticket renvoie les lignes écrites
  par AC1, et elle est documentée dans `crates/mika-agent/CLAUDE.md` § *Audit
  Log* avec la forme de ligne et les deux propriétés load-bearing.
- **AC9** — La suite `mika2456` est stable en parallèle **et** en
  `--test-threads=1` : aucune sonde de ce vol ne partage une clé de
  déduplication avec une autre.
- **AC10** — La conjonction au tir réel avec la porte de section heartbeat est
  **nommément** renvoyée au vol 4 par une ligne `Tracked in:`, avec sa prémisse
  corrigée écrite dans le plan et dans le corps de la PR.

---

## Risques

| risque | nature | parade |
|---|---|---|
| **Le `static` partagé rend une sonde flaky** | le plus probable de ce vol. `RECURRING_GATE_AUDIT_SEEN` est global au binaire de test, et le V4 du vol 2 va se mettre à écrire sous `("mika", "heartbeat")` | un `label` unique par sonde, le commentaire qui dit pourquoi, et la double passe du Verification Contract |
| **Le scan rouge au premier jour** | deux littéraux voisins contiennent la sous-chaîne (`…_resolved`, le sentinel de session) | égalité **exacte** sur le littéral entier, via le prédicat partagé `audit_name_literal_present` ; la raison est dans le commentaire du test |
| **Le marquage avant l'écriture** | une inversion de deux lignes tait le refus 24 h sans rien rougir | `Ok(()) => mark…` ; la propriété est dans le doc-comment de `mark_recurring_gate_audited` |
| **Le WARN perd son contrat en silence** | son absence change de sens (C3) et personne ne le lit nulle part | écrit dans le doc-comment du prédicat **et** dans § *Audit Log* |
| **La déduplication est perdue au redémarrage** | par conception, mais relisible comme un défaut | dit dans le doc-comment du `static` et dans la doc opérateur : le premier refus après un restart réécrit une ligne, et un restart est une information |
| **Le cap de 10 000 n'est pas testé** | `seen.clear()` sur un `static` partagé viderait la table des tests parallèles | refus assumé et **écrit** dans le commentaire du test, pour qu'il ne se relise pas comme un oubli |
| **Le vol 4 se rouvre sur la prémisse d'origine** | le corps du ticket affirme qu'une boucle de ticks est nécessaire ; la mesure dit le contraire | la prémisse corrigée est dans le § Périmètre, dans AC10 et dans le corps de la PR |
