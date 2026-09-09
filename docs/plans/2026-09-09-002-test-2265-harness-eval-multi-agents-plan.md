---
issue: 2265
type: test
title: "harness eval multi-agents — N agent_id sur une DB container partagée, pour que la classe fan-out cesse d'être invisible"
branch: test/2265/harness-harness-eval-multi-agents-2
status: groomed
---

# Plan — harness eval multi-agents (mika#2265)

## Invariant (une phrase)

**Un test de `crates/mika-agent/tests/` peut monter N agents distincts (`agent_id`) sur la
même base container, leur faire traiter le même événement, et asserter *quel agent* a atteint
quel callsite — attribution que le harness mono-agent actuel ne peut pas produire, quelle que
soit la réalité mesurée.**

Le mot load-bearing est **attribution**. La classe de bugs du 2026-09-09 (#2248, #2260, #2263)
n'est pas « le mauvais résultat » : c'est « le bon résultat, sous le mauvais acteur » ou « deux
acteurs là où un seul était prévu ». Un harness qui n'a qu'un `agent_id` rend toute assertion de
cette forme *vacuously* satisfaite — elle passe sans rien mesurer. C'est la définition d'une
sonde qui ment sans échouer.

## Ce que le code dit aujourd'hui (vérifié, `file:line`)

### Le harness eval est mono-agent, par construction

- `tests/eval/harness.rs:457-459` — `build()` fait `Database::open_in_memory()` puis
  `AsyncDatabase::new(db)`. `async_db.rs:48-50` : `new()` délègue à
  `new_with_agent(db, "mika")`. Il n'existe **aucun** point d'extension dans `EvalHarnessBuilder`
  (`harness.rs:237-416`, 21 setters `pub fn`) pour un second `agent_id` ni pour une DB partagée.
- Conséquence directe : toutes les lectures d'audit passent par `self.agent_id`
  (`async_db.rs:1028` `count_audit_events_by_tool_name`, `async_db.rs:2282` `get_audit_events`,
  `async_db.rs:1040`/`1055`). Sur un harness mono-agent elles rendent *toujours* une seule
  tranche. La question « lequel des deux ? » n'a pas de forme exprimable.

### La topologie de production est : UNE base, N connexions, N `agent_id`

C'est le point que le corps du ticket ne nomme pas et dont tout le reste dépend.

- `server/mod.rs:445` — `let db_path = home::container_db_path(global_home);` puis
  `server/mod.rs:447` `Database::open(&db_path)` et `server/mod.rs:469`
  `AsyncDatabase::new_with_agent(db, agent_name)`. `home.rs:77-79` :
  `container_db_path` = `{home}/data/mika.db` — **une seule et même** valeur pour tous les agents
  du container.
- Chaque agent ouvre donc sa **propre connexion rusqlite** (donc son propre thread `mika-db`,
  `async_db.rs:53-84`) sur le **même fichier**. `db.rs:922-933` pose `journal_mode = WAL`,
  `busy_timeout = 5000`, `foreign_keys = ON` — les writes d'un agent sont visibles de l'autre.
- `async_db.rs:121-126` — `with_agent()` existe et re-scope un handle *en partageant le même
  `Arc<Inner>`* : une connexion, un thread. Ce n'est **pas** la topologie de production (c'est un
  re-scope intra-process, pas deux agents).

**Ce que cela invalide.** Le montage naïf « deux `AsyncDatabase::new_with_agent(Database::open_in_memory(), …)` »
produit deux mémoires **disjointes** : chaque agent écrit dans son propre néant, aucune écriture
n'est visible de l'autre, et le montage *ressemble* à du multi-agents sans partager quoi que ce
soit. C'est exactement la forme qu'a prise `test_merge_identity_2248.rs:46-53` (`db_for_agent`) —
correct pour ce fichier-là, qui n'assert que des invariants **structurels** sur la source
(`include_str!`, lignes 32-34) et n'a jamais eu besoin d'un état croisé. Généraliser ce helper
tel quel livrerait un harness qui ne voit pas plus que l'actuel.

### Le fan-out lui-même vit dans le gateway, pas dans mika-agent

- `mika-gateway/src/github.rs:358-369` — `secondary_targets()` : `check_suite.completed(success)`
  → primaire `mika-dev` (via `route_event`) + secondaire `&["mika-qa"]`.
- `mika-gateway/src/github.rs:1058-1085` — la diffusion : une `tokio::spawn` pour le primaire,
  puis une boucle sur les secondaires, chacun avec son propre permit/slot.
- Le routage a déjà ses tests (`github.rs:1921`, « secondary_targets tests »).

**Ce que cela borne.** Le harness de ce ticket ne re-teste pas le routage. Il monte l'**aval** :
les deux agents ont reçu l'événement, exécutent leurs handlers contre la base partagée, et le
harness rend l'état croisé qui en résulte. La zone du ticket (`crates/mika-agent/tests/`) est la
bonne zone pour cet aval, et il n'y a pas de raison de la déborder.

### Les sondes croisées existent déjà — aucune nouvelle SQL n'est requise

- `async_db.rs:1028` `count_audit_events_by_tool_name(tool_name)` — scopée sur `self.agent_id`.
  Appelée une fois par handle, elle **est** la mesure fan-out (« combien d'agents ont atteint ce
  callsite »).
- `async_db.rs:2869` `count_audit_events(agent_id)` — prend l'`agent_id` en **paramètre
  explicite**. Un handle peut donc interroger la tranche d'un autre agent : c'est la sonde de
  **partage** (contrôle positif du montage lui-même).
- `db.rs:1626-1642` — `audit_events.agent_id TEXT NOT NULL REFERENCES agents(id)`. Chaque agent
  monté DOIT être `register_agent`é avant d'écrire, sinon la FK rejette.

### `spawn_live_child` existe mais est privé à un fichier

- `tests/eval/test_pilot_silent_stall_reaper.rs:111-125` (`spawn_live_child`), `:127-129`
  (`kill_pid`), `:138-151` (`spawn_and_reap_child`) — `fn` privées de module, non réutilisables.
  `spawn_and_reap_child` porte un commentaire load-bearing (`:131-137`) : un enfant non *reapé*
  devient zombie et `/proc/<pid>/stat` survit, si bien qu'un test bâti dessus assert le contraire
  de ce qu'il prétend. Cette connaissance doit voyager avec le helper, pas rester dans un fichier.

## Décisions de conception

### D1 — Une base FICHIER partagée, pas `open_in_memory`

Le harness alloue un `TempDir`, y place `data/mika.db`, et ouvre **une connexion par agent** via
`Database::open(&db_path)` (WAL, `busy_timeout=5000`, `migrate()` idempotent au second open).
C'est la topologie de `server/mod.rs:445-469`, à l'identique.

*Rejeté :* `with_agent()` sur un handle unique — une seule connexion, un seul thread : ne
reproduit ni la concurrence inter-connexion ni les contentions WAL, c'est-à-dire précisément la
dimension où vivent #2248 et #2260.
*Rejeté :* deux `open_in_memory()` — mémoires disjointes (voir plus haut).

Coût assumé : un fichier temporaire au lieu de la mémoire. Les tests eval sont déjà des tests
d'intégration ; le `TempDir` est possédé par le harness et tombe avec lui.

### D2 — Le harness fournit l'état partagé et les sondes, PAS des combinateurs de flux

Le harness expose les handles (`agents()`, `db(agent_id)`) ; le test compose lui-même la
diffusion — boucle `for` pour un ordre déterministe, `tokio::join!` pour une course.

*Pourquoi.* Un `for_each_agent(closure_async)` générique exige du HRTB sur `&AsyncDatabase` pour
zéro gain de lisibilité face à `for (id, db) in h.agents()`. Et surtout : #2248 est un
**ordre**, #2260 est une **course** — les deux formes sont nécessaires, aucune n'est le défaut de
l'autre. Le harness ne choisit pas à la place du test. Le patron des deux formes est documenté
dans le doc-comment du module.

*Conséquence :* aucune dépendance nouvelle. `futures-util` est déclaré au workspace
(`Cargo.toml:76`) mais avec `default-features = false`, ce qui exclut `future::join_all` (feature
`alloc`) — `tokio::join!` est une macro sans feature et suffit.

### D3 — Zéro modification de `crates/mika-agent/src/`

Toutes les primitives requises sont déjà `pub` (D1, sondes ci-dessus). Le ticket est test-only et
le reste. Si l'implémentation découvre un besoin d'API dans `src/`, c'est un **signal de
conception** à remonter, pas une extension à faire au passage.

### D4 — Le témoin porte deux contrôles dans le même fichier

AC3 dit « échoue AVANT et passe APRÈS » sans nommer l'axe. Un test qui échoue avant *parce que le
helper n'existe pas encore* ne démontre rien : il ne compile pas, ce n'est pas un rouge, c'est une
absence. Et un témoin qui asserterait l'état de fait de #2260 (« deux agents entrent dans le
chemin de merge ») graverait le bug dans la suite et virerait au rouge le jour où #2260 ferme.

Le témoin démontre donc la **capacité de mesure**, avec contrôle positif ET négatif dans le même
fichier (cf. `feedback_a_probe_needs_both_controls_in_the_same_call`) :

- **positif** — deux agents écrivent le même `tool_name` sur le même `target_key` ; le harness
  rend `{mika-dev: 1, mika-qa: 1}` : deux attributions distinctes.
- **négatif** — le même scénario monté sur `EvalHarness` (mono-agent) : les deux écritures
  atterrissent sous l'unique `agent_id`, la carte rend une seule clé. L'assertion « deux
  attributions » y est **structurellement** inatteignable. C'est le rouge-avant, exprimé comme un
  test **vert** qui épingle l'incapacité.
- **partage** — un handle lit la tranche d'un autre agent (`count_audit_events(autre_agent) > 0`).
  Sans ce contrôle, un montage à mémoires disjointes passerait les deux premiers.

## Livrables

### L1 — `crates/mika-agent/tests/eval/multi_agent.rs` (nouveau)

```rust
pub struct MultiAgentHarness { /* TempDir + db_path + Vec<(String, AsyncDatabase)> ordonné */ }

impl MultiAgentHarness {
    pub fn builder() -> MultiAgentHarnessBuilder;      // .agent("mika-dev").agent("mika-qa")
    pub fn db(&self, agent_id: &str) -> &AsyncDatabase; // panique, nommant les agents montés
    pub fn agents(&self) -> impl Iterator<Item = (&str, &AsyncDatabase)>; // ordre du builder
    pub fn session_id(&self, agent_id: &str) -> &str;
    pub fn db_path(&self) -> &Path;

    /// Mesure fan-out : combien d'événements portant `tool_name`, par agent monté.
    pub async fn audit_counts_by_agent(&self, tool_name: &str) -> BTreeMap<String, i64>;

    /// Sonde de partage : ce que `reader` voit de la tranche de `subject`.
    pub async fn cross_read_count(&self, reader: &str, subject: &str) -> u64;

    pub fn shutdown(self);   // shutdown() sur chaque handle, puis drop du TempDir
}
```

`build()` par agent, dans l'ordre déclaré : `Database::open(&db_path)` → `register_agent(id, id, "")`
→ `create_session(session_for(id), id, "github")` → `AsyncDatabase::new_with_agent(db, id)`.
Le premier `open` migre ; les suivants trouvent le schéma à jour (`migrate()` idempotent).
`agents()` préserve l'ordre de déclaration — un fan-out a un primaire et des secondaires, et
l'ordre est parfois ce qu'on assert.

Déclaration : `pub mod multi_agent;` dans le bloc `mod eval` de `tests/eval.rs`.

### L2 — `crates/mika-agent/tests/eval/process_fixtures.rs` (nouveau) — AC2

`spawn_live_child()`, `spawn_and_reap_child()`, `kill_pid()` extraits *verbatim* de
`test_pilot_silent_stall_reaper.rs:111-151`, **commentaire zombie inclus** (`:131-137`) : c'est la
part qui empêche un futur test d'asserter le contraire de ce qu'il croit.
`test_pilot_silent_stall_reaper.rs` importe désormais depuis ce module et perd ses copies —
pas de duplication, et son comportement doit être **inchangé** (voir V2).

### L3 — `crates/mika-agent/tests/eval/test_multi_agent_harness_witness.rs` (nouveau) — AC3

Les trois tests de D4 : positif, négatif (mono-agent), partage. Le scénario est celui de #2248 —
`tool_name = "ci_success_handler_processed"`, `target_key = "senara-solutions/mika#2244"` — pour
que le témoin nomme la classe qu'il rend visible, sans dépendre du handler réel.

### L4 — Doc-comment de module (dans L1)

En tête de `multi_agent.rs` : la topologie de production citée (`server/mod.rs:445-469`,
`home.rs:77-79`), pourquoi le fichier partagé et non `open_in_memory` (D1), et les deux patrons de
diffusion (boucle / `tokio::join!`, D2). Ce module sera lu par le prochain qui écrira un test de
classe fan-out ; ce qu'il doit surtout ne pas refaire, c'est le montage à mémoires disjointes.

## Vérification

| # | Ce qui est vérifié | Comment |
|---|---|---|
| V1 | Le témoin mesure | `cargo test -p mika-agent --test eval multi_agent_harness_witness` — trois tests verts |
| V2 | L'extraction n'a rien changé | `cargo test -p mika-agent --test eval pilot_silent_stall` **avant** et **après** L2 — même liste de tests, même verdict |
| V3 | Rouge-avant réel | Le test négatif retiré, l'assertion positive portée sur `EvalHarness` → **échoue**. Consigné dans le corps du test, pas seulement dans ce plan |
| V4 | Pas de dérive `src/` | `git diff --stat main -- crates/mika-agent/src/` → vide |
| V5 | Hygiène | `cargo clippy -p mika-agent --tests -- -D warnings`, `cargo fmt --check` |

V3 est le seul point où le plan demande une manipulation manuelle : elle se fait une fois,
pendant l'implémentation, et son résultat s'écrit dans le doc-comment du test négatif. Un rouge
qu'on n'a pas vu de ses yeux est une croyance.

## Hors périmètre (explicite)

- **Corriger #2260 ou #2263.** Ce ticket livre l'instrument, pas le remède. Un témoin qui
  asserterait l'état de fait de #2260 le graverait (D4).
- **Tester le routage gateway.** `secondary_targets` a déjà ses tests
  (`mika-gateway/src/github.rs:1921`).
- **Un troisième agent, ou N > 2.** Le builder accepte N par construction ; aucun test de ce
  ticket n'en monte plus de deux. Monter N=3 sans une classe de bug qui le réclame serait de la
  généralité non mesurée.
- **Toute modification de `crates/mika-agent/src/`** (D3).

## Acceptance (tie-back)

| AC | Livrable | Vérif |
|---|---|---|
| AC1 — helper ≥2 agents sur un même event, pilotant leurs handlers | L1 (+ L4) | V1 |
| AC2 — `spawn_live_child` réutilisable | L2 | V2 |
| AC3 — témoin fan-out, rouge avant / vert après | L3 (D4) | V1, V3 |

## Note de grooming — deux points relevés dans le corps du ticket

1. **`BLOQUE #<C>`** : placeholder non résolu. Aucun ticket n'est référencé. Le plan ne suppose
   aucune dépendance sortante ; si `<C>` désignait un ticket réel, il reste à nommer.
2. **« deux `AsyncDatabase`/`agent_id` partageant le même event »** : exact, et le plan le livre —
   mais la condition *sine qua non* que le corps ne nomme pas est que les deux handles partagent
   la même **base**. Sans elle, deux `AsyncDatabase` ne partagent rien (D1). C'est un raffinement
   de la prémisse, pas une contradiction.
