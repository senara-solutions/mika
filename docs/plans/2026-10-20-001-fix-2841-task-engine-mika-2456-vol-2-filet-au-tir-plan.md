# mika#2841 — le filet au tir de la porte récurrente niveau-agent (vol 2 de mika#2456)

- **Ticket :** senara-solutions/mika#2841 (`Refs mika#2456`)
- **Branche :** `fix/2841/task-engine-mika-2456-vol-2-filet-au-tir`
- **Plan amont :** `docs/plans/2026-10-19-007-fix-2456-enabled-false-ne-gate-pas-les-recurrentes-plan.md` (sur `main`), § 6 « Vol 2 — le filet au tir et sa surface d'audit »
- **Vol 1 :** PR #2835, mergée en `cf21685b` le 2026-10-20 — la porte à l'**enregistrement**
- **Date :** 2026-10-20

## Contexte

Le vol 1 a livré la porte agent-level : `Identity::recurring_tasks_enabled()` et son
blocage dans `ensure_recurring_task_with_policy`. Cette porte tourne **au boot et au
provision**, c'est-à-dire au moment où une ligne récurrente est (ré-)enregistrée. Elle
ne voit donc pas deux populations :

1. les lignes récurrentes **nées avant** le déploiement de la porte — elles sont déjà
   en base, `status = 'recurring_active'`, et personne ne repasse les enregistrer ;
2. les bascules d'`enabled` **en cours d'exécution** — un opérateur qui pose
   `enabled = false` dans `identity.toml` sans redémarrer.

Le vol 2 est le **filet au tir** : le dernier point de passage avant qu'une récurrente
ne consomme un tour de LLM. Il n'est pas redondant avec la porte. En régime attendu il
ne refuse rien et n'écrit aucune ligne — exactement comme le filet du dispatcher de
mika#2337. **Son zéro est l'assertion que la porte amont tient**, et c'est pour cela
qu'il doit exister même quand il ne mord jamais.

### Pourquoi le filet ne peut pas se contenter de la porte

La porte décide *« faut-il armer cette récurrence ? »*. Le filet décide *« faut-il
tirer cette récurrence-ci, maintenant ? »*. Les deux questions ont des réponses
différentes précisément dans les deux populations ci-dessus, et la seconde est la seule
qui soit vraie au moment où le coût est engagé. Une porte seule est un invariant sur
l'historique des écritures ; un filet est un invariant sur l'état courant.

## Périmètre — ce que CE vol livre

- `refuse_recurring_fire_if_agent_disabled` dans `dispatcher.rs`, câblé **en tête** de
  `dispatch_run_skill`, avant toute résolution de skill ou de trigger.
- L'événement tracing `recurring_fire_refused_agent_disabled` (WARN), avec `agent_id`,
  `label` et `trigger_type`.
- Le compte de lignes annulées : `cancel_recurring_task_by_label -> Result<usize>` sur
  les deux étages (`db/tasks.rs`, `async_db.rs`), remonté jusqu'au log de
  `cancel_recurring_task_for_disabled_agent` dans `task_engine/mod.rs`.
- Les sondes V4 (le filet refuse), V5 (contrôle négatif), V6 (le filet n'usurpe pas la
  décision d'une porte de section) et le scan structurel V8 (R-7).

## Périmètre — ce que CE vol ne livre PAS, et pourquoi

La **surface d'audit en base** — la ligne `audit_events` de `tool_name =
'agent_recurring_gate'`, son rafraîchissement 24 h, son cap de mémoire et son contrat
de SOLE WRITER — part dans un vol suivant. Raison : le mécanisme de déduplication est
indivisible du mécanisme d'audit (sans lui, une récurrente à la minute écrit ~1 440
lignes par jour), et le couple pèse environ 110 lignes de plus que ce que la borne par
vol de ce ticket autorise. La découpe est donc en **vols** et non en phases d'un même
vol, conformément à mika#2693.

**Ce que ce report coûte, dit plutôt que tenu sous silence : jusqu'au vol suivant, la
requête opérateur du § 5 du plan amont ne renvoie rien.**

```sql
SELECT target_key, after_value, count(*)
FROM audit_events
WHERE tool_name = 'agent_recurring_gate'
GROUP BY 1, 2;
```

Un `count(*)` vide sur cette requête ne dit **pas** « aucun refus » : il dit « ce vol
n'écrit pas cette ligne » (doctrine mika#2331 — absent n'est pas zéro). Jusque-là le
refus se lit dans les logs, par `event = "recurring_fire_refused_agent_disabled"`. Le
plan interdit explicitement d'écrire une documentation opérateur promettant la requête
SQL avant que le vol suivant ne la serve.

`Tracked in:` un ticket de suivi à ouvrir sur `senara-solutions/mika` à la fusion de ce
vol, intitulé « la surface d'audit du filet récurrent (vol 3 de mika#2456) », portant :
la constante `RECURRING_GATE_AUDIT_TOOL_NAME = "agent_recurring_gate"` et son contrat
de SOLE WRITER ; `RECURRING_GATE_AUDIT_REFRESH` (24 h) ; le cap
`RECURRING_GATE_AUDIT_SEEN_CAP` ; le `static LazyLock<Mutex<HashMap<…>>>` transcrit de
`auto_pull.rs::EXCLUSION_AUDIT_SEEN` (mika#2131, fail-open sur mutex empoisonné) ;
`recurring_gate_audit_is_due` / `mark_recurring_gate_audited` ; la sonde V4-bis par la
ligne d'audit ; la sonde V4c de la fenêtre de déduplication ; le scan source
SOLE-WRITER ; et la documentation opérateur de la requête ci-dessus. La wip
`backup/2456-wip` (`d9b2f4eb`) en porte déjà la matière.

Hors périmètre également, et nommé pour qu'on ne le cherche pas : la conjonction au
**tir réel** avec la porte de section heartbeat (une récurrente heartbeat effectivement
non tirée sous `[heartbeat] enabled = false`) demande de piloter un `TaskEngine` complet
sur une boucle de ticks ; elle part avec la surface d'audit, dont les sondes paient déjà
ce harnais.

## Invariants (repris du plan amont, § R)

| id | invariant | tenu par |
|---|---|---|
| R-1 | `enabled = false` empêche un **tir** de récurrente, pas seulement son enregistrement | le filet |
| R-3 | la porte agit sur `trigger_type = "recurring"` **seul** — les rappels (`"time"`), le chemin interactif et les callbacks sont intouchés (§ 3.5 du plan amont) | V5 |
| R-5 | le filet ne décide rien pour un agent dont la racine `enabled` est vraie ou absente ; il laisse les portes de section décider | V6 |
| R-7 | `enabled` **n'entre pas** dans `CODE_OWNED_IDENTITY_SECTIONS` — la section reste opérateur-owned par décision | V8 |

R-3 a une conséquence concrète à ne pas perdre : `mika-test` est un banc, et
`mika ask --agent mika-test` doit continuer de répondre après ce vol. Un filet qui
refuserait aussi les rappels ou le chemin interactif casserait le banc en silence.

## Les quatre corrections à la wip de référence

La wip vit sur `backup/2456-wip` (`d9b2f4eb`, 2026-09-21). Le ticket en nomme trois ;
une quatrième a été trouvée en vérifiant les trois contre `main`. Toutes quatre ont été
confirmées sur `main` au grooming.

1. **Les deux signatures du compte annulé.** `cancel_recurring_task_by_label` rend
   `Result<()>` et non un compte, aux deux étages : `async_db.rs:425` et
   `db/tasks.rs:528`. Corps du changement en base :

   ```rust
   pub fn cancel_recurring_task_by_label(&self, agent_id: &str, label: &str) -> Result<usize> {
       let cancelled = self.conn.execute(/* même UPDATE */)?;
       Ok(cancelled)
   }
   ```

   Les deux appelants existants restent compatibles — ils ignorent la valeur.

2. **Le reste de débogage.** `refuse_recurring_fire_if_agent_disabled` contient, dans la
   wip, littéralement :

   ```rust
   if false && task.trigger_type != super::types::trigger_type::RECURRING {
       return false;
   }
   ```

   Le `if false &&` neutralise la garde et fait refuser aussi les rappels, ce que R-3
   interdit. À la reprise, le `if false &&` disparaît : la garde devient inconditionnelle
   et **c'est elle que V5 prouve**.

3. **Aucune plomberie n'est nécessaire.** `TaskDispatcher` porte déjà `home_dir`
   (`dispatcher.rs:508`) et `global_home_dir` (`:523`), tous deux `pub`. Il n'y a donc
   aucun champ à ajouter, aucun constructeur à toucher, et aucun des 22 champs du
   littéral de test à étendre.

4. **`retry_on_busy` est typé sur `Result<()>` — correction que le ticket omet.**
   `task_engine/mod.rs:217-223` :

   ```rust
   async fn retry_on_busy<F, Fut>(
       policy: &RecurringRetryPolicy,
       mut op: F,
   ) -> Result<RetryReport, (RetryReport, bool, anyhow::Error)>
   where
       F: FnMut() -> Fut,
       Fut: std::future::Future<Output = anyhow::Result<()>>,
   ```

   La sortie du futur est **câblée à `()`**. Un `cancel_recurring_task_by_label` qui rend
   `Result<usize>` ne peut donc pas être passé tel quel par ce chemin, et c'est bien par
   ce chemin que `cancel_recurring_task_for_disabled_agent` l'appelle
   (`task_engine/mod.rs:409`). Deux issues, et le plan **choisit la première** :

   - **(retenue) généraliser sur `T`** — `Fut: Future<Output = anyhow::Result<T>>`, le
     succès devenant `Ok((T, RetryReport))`. Coût : la signature, le bras `Ok`, et
     l'ajustement des 6 sites d'appel (`:343`, `:409`, et les tests `:1218`, `:1242`,
     `:1262`, `:1286`), soit ~18 lignes. Retenue parce qu'elle laisse le compte
     traverser le mécanisme de retry au lieu de le contourner : un `usize` capturé hors
     du futur n'a pas de rapport prouvable avec la tentative qui a réussi.
   - (écartée) capturer le compte dans un `Cell` autour de la closure — ~4 lignes, pas
     de churn de signature, mais la valeur devient celle de la **dernière** tentative,
     réussie ou non, ce qui est précisément la classe de défaut qu'un compte est censé
     écarter.

## Implémentation — ordre de commit

Quatre commits, chacun compilable et testable seul.

**C1 — le compte traverse les trois étages.**
`db/tasks.rs` (`-> Result<usize>`, `Ok(cancelled)`), `async_db.rs` (signature),
`task_engine/mod.rs` (`retry_on_busy` généralisé sur `T`, 6 sites d'appel ajustés,
`cancel_recurring_task_for_disabled_agent -> usize` et le compte ajouté au champ du log
existant). `cargo test -p mika-agent task_engine` doit rester vert sans qu'aucune
assertion existante ne change.

**C2 — le filet et son câblage.**
`dispatcher.rs` : la constante `RECURRING_FIRE_REFUSED_EVENT` avec son doc-comment,
`async fn refuse_recurring_fire_if_agent_disabled(&self, task: &Task) -> bool`, et son
appel en tête de `dispatch_run_skill` (`:713`), **avant** le `if let Some(skill_name)` /
`match trigger_name` de `:719-724`. Le corps du filet, dans l'ordre :

1. si `task.trigger_type != types::trigger_type::RECURRING` → `false` (R-3, sans
   `if false &&`) ;
2. résoudre le home de l'agent depuis `self.home_dir` / `self.global_home_dir` ;
3. consulter `task_engine::agent_recurring_tasks_enabled(home_dir, agent_id)`
   (`mod.rs:860`, `pub(crate)`, deux arguments) ;
4. si l'agent est activé → `false` ;
5. sinon : un WARN sous `RECURRING_FIRE_REFUSED_EVENT` portant `agent_id`, `label`,
   `trigger_type`, puis `true`.

Le retour `bool` signifie « j'ai refusé », et l'appelant abandonne le tir.

**C3 — les sondes du filet, inline.**
`dispatcher.rs`, dans le `mod tests` existant. **Pas de nouveau harnais** : le module
possède déjà `test_db()` (`:5275`), `test_dispatcher_with_homes(db, home_dir,
global_home_dir)` (`:5298` — le littéral complet des 22 champs, avec `NoopSender`,
`dummy_provider()` et `SkillRegistry::empty()`) et `no_stop_home()` (`:5294`). Deux
helpers courts à ajouter : `home_with_enabled(bool) -> TempDir` (écrit un
`identity.toml`) et `recurring_task(&db, label) -> Task` (passe par
`create_task(NewTask { … })` puis relit la ligne, comme `curator_task` au `:9201` — les
~35 champs de `Task` ne se construisent pas à la main).

Le témoin du refus est **la valeur de retour `bool` du filet**, assertée directement,
et non une ligne en base. C'est le point où ce vol s'écarte de la wip, dont la V4 pilote
un `TaskEngine` réel sur 120 itérations de 50 ms et lit la ligne d'audit comme unique
témoin. Ce couplage est ce qui rendait la découpe impossible : retirer l'audit d'un vol
retirait aussi l'observable de sa sonde. En assertant le `bool`, la sonde de ce vol ne
dépend plus de l'étage d'audit, et la sonde par la ligne d'audit reste due au vol
suivant comme V4-bis.

**C4 — V8, le scan structurel de R-7.**
`canonical_tokens.rs`, sur le modèle exact de
`RECURRING_REGISTRATION_SCAN_EXCEPTIONS` (`:1164-1280`) : constante d'exceptions livrée
**vide** et assertée vide, aiguille composée à l'exécution (mika#2496), filtrage des
lignes de commentaire, et boucle anti-vacuité qui exige que le fichier propriétaire soit
bien trouvé (mika#2103/#2205).

### Plancher, si le pilote chauffe

L'ordre ci-dessus est aussi l'ordre de priorité. Si le plafond de tours approche :

1. **C4 (V8) part en premier** — R-7 est un invariant sur une constante qui ne bouge pas
   dans ce vol ; son scan rejoint le vol suivant.
2. **Puis V6 seule** dans C3 — V4 et V5 sont le couple minimal (un détecteur et son
   contrôle négatif) et ne se séparent jamais.
3. **C1 + C2 + V4 + V5 sont le plancher dur.** En dessous, le vol ne livre rien de
   prouvé et il vaut mieux qu'il n'atterrisse pas.

## Verification Contract

| id | ce qui est prouvé | où | forme |
|---|---|---|---|
| **V4** | le filet refuse un tir récurrent sur un agent dont la racine porte `enabled = false` | `dispatcher.rs` `mod tests` | `refuse_recurring_fire_if_agent_disabled(&task)` rend `true` |
| **V5** | contrôle négatif, deux branches : (a) `enabled = true` sur une récurrente → `false` ; (b) `enabled = false` sur un `trigger_type = "time"` → `false` (R-3) | idem | deux assertions, même graine, seul le champ varie |
| **V6** | le filet n'usurpe pas la décision d'une porte de section : racine `enabled = true` **et** `[heartbeat] enabled = false` → le filet rend `false` et laisse `heartbeat_enabled_for_agent` (`mod.rs:877`) décider | idem | assertion sur le `bool`, pas sur le tir |
| **V8** | R-7 — `enabled` n'est pas dans `CODE_OWNED_IDENTITY_SECTIONS` (`well_known_agents.rs:709`) | `canonical_tokens.rs` | scan structurel, exceptions vides, anti-vacuité |
| **V-compte** | le compte de lignes annulées est le nombre réel de lignes passées en `cancelled`, et il traverse `retry_on_busy` | `tests/eval/test_agent_disabled_recurring_gate_2456.rs` | deux récurrentes, une annulation, `assert_eq!(2, …)` ; helpers `test_db`/`agent_home`/`home_with_enabled` déjà sur place (vol 1) |

V4 et V5 forment le couple indissociable : sans V5, V4 passe aussi sur un filet qui
refuse tout, et c'est très exactement le défaut que la correction 2 répare.

Commandes : `cargo test -p mika-agent task_engine`,
`cargo test -p mika-agent canonical_tokens`,
`cargo test -p mika-agent --test eval test_agent_disabled_recurring_gate_2456`,
`cargo clippy --all-targets`, `scripts/verify-pipeline.sh origin/main`.

## Fire-Disposition

Ce vol livre cinq détecteurs (V4, V5, V6, V8, V-compte) : la section est requise.

**Option retenue : (a) exception nommée en allowlist, livrée VIDE.**

Le seul détecteur de ce vol qui balaie l'arbre source est **V8**. Il reçoit sa propre
constante d'exceptions,

```rust
const IDENTITY_ENABLED_CODE_OWNED_SCAN_EXCEPTIONS: &[&str] = &[];
```

livrée **vide** et **assertée vide** dans le même test, sur le modèle littéral de
`RECURRING_REGISTRATION_SCAN_EXCEPTIONS` (`canonical_tokens.rs:1164`). La mesure au
grooming est que R-7 tient sur `main` : `CODE_OWNED_IDENTITY_SECTIONS` ne contient pas
`enabled`, et son propre doc-comment déclare la section opérateur-owned. V8 atterrit
donc **armé et vert**, sans aucune violation à excuser.

L'assertion auto-nettoyante est l'assertion de vacuité elle-même : toute entrée ajoutée
plus tard rougit immédiatement le test de vacuité, ce qui oblige à nommer la donnée, le
ticket de suivi et la raison dans le même commit. Et le message de refus porte la
doctrine mika#2201, transcrit du site modèle : *un site qu'on ne veut pas armer est un
site à supprimer*, pas une ligne à ajouter à l'allowlist.

Les quatre autres détecteurs (V4, V5, V6, V-compte) sont des sondes comportementales sur
du code que ce même vol livre ; ils n'ont pas de population préexistante à excuser, donc
pas d'allowlist, et atterrissent armés.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/tasks.rs` (signature + `Ok(cancelled)`) | 5 |
| `crates/mika-agent/src/async_db.rs` (signature) | 3 |
| `crates/mika-agent/src/task_engine/mod.rs` (`retry_on_busy` sur `T`, 6 sites, compte remonté) | 32 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` (constante, filet, câblage) | 52 |
| tests inline `dispatcher.rs` (2 helpers + V4 + V5 + V6) | 90 |
| `crates/mika-agent/src/canonical_tokens.rs` (V8) | 42 |
| tests (`tests/eval/test_agent_disabled_recurring_gate_2456.rs`, V-compte) | 25 |

Total estimé : 249 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

### La re-mesure que le ticket demande

Le ticket pose la question explicitement : *« la wip entière pèse environ 1 064 lignes,
soit environ 3,8 fois la borne, et le vol 1 en a pris la porte. Il faut re-mesurer
contre `main` au groom. »* Mesures successives, contre `main` :

| périmètre | estimé | verdict |
|---|---|---|
| transcription intégrale de la wip | ≈ 633 | 2,3 × la borne |
| sans le compte, V6 et V8 | ≈ 500 | encore au-dessus |
| sans en plus la déduplication 24 h et le scan SOLE-WRITER | ≈ 352 | encore 73 au-dessus |
| **périmètre de ce vol** (harnais inline existant, témoin `bool`) | **249** | **sous la borne** |

Le levier décisif n'est pas un retrait de fonctionnalité mais un retrait de **harnais** :
le `mod tests` de `dispatcher.rs` possède déjà `test_dispatcher_with_homes`, donc les
~77 lignes de harnais d'intégration de la wip (`NoopSender`, `dispatcher_with_home`,
`gate_audit_rows`, `llm_call_rows`, `seed_recurring`) et ses boucles de ticks de 6 s ne
sont pas à transcrire.

### Pourquoi cette estimation est plus fiable que celle du vol 1

Le vol 1 avait estimé 267 et livré 815 insertions réelles, soit ~3 ×. Trois différences
matérielles ici, et une marge :

- le code de production **existe déjà**, mesuré ligne par ligne sur `d9b2f4eb` ; il n'est
  pas à concevoir mais à transcrire moins trois morceaux nommés ;
- le harnais de test n'est pas à écrire — c'est le poste qui avait dérapé au vol 1 ;
- les quatre corrections sont chiffrées à la ligne, pas estimées en bloc.

La marge est de 30 lignes sur 279, ce qui est mince. C'est pourquoi le plancher ci-dessus
est nommé dans l'ordre où il s'applique, plutôt que laissé à l'arbitrage du pilote au
tour 140.

## Definition of Done

- [ ] `cancel_recurring_task_by_label` rend `Result<usize>` aux deux étages, et le compte
      apparaît dans le log de `cancel_recurring_task_for_disabled_agent`.
- [ ] `retry_on_busy` est générique sur `T`, et ses 6 sites d'appel compilent sans
      changement de comportement.
- [ ] `refuse_recurring_fire_if_agent_disabled` existe, est appelée en tête de
      `dispatch_run_skill`, et sa garde `trigger_type` est inconditionnelle (aucun
      `if false &&` dans le diff).
- [ ] `recurring_fire_refused_agent_disabled` est émis en WARN avec `agent_id`, `label`,
      `trigger_type`.
- [ ] V4, V5, V6, V8 et V-compte sont verts, et V8 atterrit armé avec sa constante
      d'exceptions vide et assertée vide.
- [ ] `cargo test`, `cargo clippy --all-targets` et `scripts/verify-pipeline.sh
      origin/main` passent.
- [ ] Le corps de PR porte la ligne `Tracked in:` nommant le ticket de suivi de la
      surface d'audit, et dit que la requête SQL `tool_name = 'agent_recurring_gate'` ne
      renvoie rien avant ce vol-là.
- [ ] Aucune documentation opérateur ajoutée ne promet la requête d'audit.

## Acceptance criteria

Le corps de mika#2841 ne porte pas de section `## Acceptance criteria`. Les critères
ci-dessous sont dérivés de ses exigences et de AC2 / AC4 du plan amont
(`2026-10-19-007-…-plan.md`), bornés au périmètre de ce vol.

- **AC1** — Une ligne récurrente **née avant** le déploiement, appartenant à un agent
  dont `identity.toml` porte `enabled = false` à la racine, n'est pas tirée : le filet
  la refuse au point de dispatch. (Couvre AC2 du plan amont pour la première des deux
  populations ; prouvé par V4.)
- **AC2** — Une bascule d'`enabled` **en cours d'exécution** mord au tir suivant, sans
  redémarrage : le filet relit l'identité à chaque passage. (Seconde population ;
  prouvé par V4, dont le `identity.toml` est écrit après la création de la ligne.)
- **AC3** — Le refus est nommé : `event = "recurring_fire_refused_agent_disabled"` en
  WARN, portant `agent_id`, `label` et `trigger_type`. Un refus muet ne compte pas
  (mika#2293 — un réglage qu'on ne peut pas observer est un espoir).
- **AC4** — Le filet ne touche **que** `trigger_type = "recurring"`. Les rappels
  (`"time"`), le chemin interactif et les callbacks sont intouchés, et
  `mika ask --agent mika-test` répond toujours. (§ 3.5 du plan amont ; prouvé par la
  branche (b) de V5.)
- **AC5** — Sous une racine `enabled` vraie ou absente, le filet ne décide rien et
  laisse les portes de section (dont `[heartbeat] enabled`) décider seules. (Prouvé par
  V6.)
- **AC6** — `cancel_recurring_task_by_label` rend le nombre réel de lignes passées en
  `cancelled`, et ce nombre traverse le mécanisme de retry plutôt que de le contourner.
  (AC4 du plan amont ; prouvé par V-compte.)
- **AC7** — `enabled` n'est pas entré dans `CODE_OWNED_IDENTITY_SECTIONS`, et un scan
  structurel armé le refuse. (R-7 ; prouvé par V8.)
- **AC8** — La portée est dite honnêtement : la PR nomme par `Tracked in:` le vol de la
  surface d'audit, et déclare que `tool_name = 'agent_recurring_gate'` n'est écrit par
  personne avant lui — un `count(*)` vide y signifie « pas encore instrumenté », pas
  « aucun refus » (mika#2331).

## Risques

- **Le câblage n'est pas prouvé par les sondes de ce vol.** V4/V5/V6 assertent le
  **prédicat**, pas sa position dans `dispatch_run_skill`. Une régression qui
  déplacerait l'appel après le `match trigger_name` les laisserait vertes. Le scan de
  câblage existant (`tests/eval/test_recurring_trigger_wiring_2337.rs`, V3.1) s'ancre
  sur l'`action_config` des appels à `ensure_recurring_task` et ne couvre pas ce
  prédicat ; l'étendre appartient au vol de la surface d'audit, dont la sonde V4-bis
  pilote de toute façon un `TaskEngine` réel et observe donc la position.
- **`revert_config_cancel_recurring_task` reste un vecteur de résurrection**
  (mika#2271) : il ne cible que `status = 'cancelled'`. Un refus au tir ne change pas
  le statut de la ligne, donc le filet n'ouvre pas de nouvelle porte de résurrection —
  mais il n'en ferme aucune non plus.
- **Le filet ne mute rien, et c'est voulu** (mika#2156, `phantom_sweep_spared` — un
  filet qui mute efface sa propre preuve). Il refuse et journalise ; l'annulation de la
  ligne reste le travail de la porte à l'enregistrement.
