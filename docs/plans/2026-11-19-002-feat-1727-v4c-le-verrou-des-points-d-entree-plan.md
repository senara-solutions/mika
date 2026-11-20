# mika#1727 — vol V4-c : le verrou des points d'entrée

- **Ticket :** senara-solutions/mika#1727
- **Branche :** `feat/1727/v4c-le-verrou-des-points-d-entree`
- **PR :** à ouvrir — `Refs mika#1727`, sans `Closes` / `Fixes` / `Resolves`
- **Suite :** aucune. Ce vol ferme le ticket à son merge.
- **Label de borne :** `loop-substrate` (plafond de tours 200)

---

## HALTE — la borne du groom est dépassée

**`Total estimé : 312 lignes` ajoutées, pour une borne du groom de `300`.**

Le corps du ticket pose, pour ce vol : *« Si ce total dépasse `300`, le groom
s'arrête et le dit. Il ne recoupe pas lui-même. »* (Vincent, 2026-11-19). Le
groom s'arrête donc ici. **Il ne choisit aucune coupe, ne rabote aucun
commentaire et n'ajoute aucune halte de son cru.** Le plan ci-dessous est écrit
en entier — il est exécutable tel quel si Vincent lève la borne — mais il ne doit
pas être exécuté tant qu'elle n'est pas levée ou que le vol n'est pas recoupé par
Vincent.

**Où va le dépassement.** Un seul poste en porte l'essentiel : la relocalisation
de `teams::run_team` et `teams::resume_team_run` dans un module privé, **109
lignes ajoutées** sur 312. Les six autres postes totalisent 203 lignes, soit sous
la borne. La raison est mécanique et n'est pas un choix de confort :

1. Rust ne sait pas **élargir** une visibilité par réexport. `pub use` d'un item
   `pub(crate)` est une erreur dure (E0364/E0365). Il n'existe donc que deux
   formes natives pour la double visibilité : (i) deux déclarations fines sous
   `#[cfg]` qui délèguent à un corps privé renommé, (ii) un module privé portant
   les items `pub`, plus deux `use` réexportés sous `#[cfg]`.
2. Pour `teams/mod.rs`, la forme (i) est **exclue par le critère 7**. Les deux
   fonctions portent **12 paramètres** chacune et déjà un
   `#[allow(clippy::too_many_arguments)]`. Chaque bras de `#[cfg]` redéclare la
   signature de 12 paramètres et réécrit le transfert de 12 arguments — environ
   30 lignes par bras, quatre bras, ≈ 120 lignes — **et** chaque bras aurait
   besoin de son propre `#[allow(clippy::too_many_arguments)]`, c'est-à-dire des
   `allow` neufs, que le critère 7 interdit. `clippy.toml` ne relève pas
   `too-many-arguments-threshold` : le seuil par défaut de 7 s'applique.
3. Reste la forme (ii), qui coûte la **réindentation** des 96 lignes non vides de
   la région 19–122 de `teams/mod.rs`, plus ≈ 13 lignes d'échafaudage. Les
   retraits ne comptent pas dans l'unité, mais les 96 lignes réindentées
   comptent comme ajoutées.

La variante la plus maigre qu'on puisse écrire sans rogner un commentaire
porteur — pas de ligne de doc par bras de `#[cfg]` — atterrit vers 298–302 : elle
ne tient la borne qu'à la marge d'erreur de l'estimation, et seulement en
supprimant les commentaires que la tête du ticket interdit justement de raboter
(*« Les commentaires comptent dans la borne, mais ils ne se rabotent pas pour la
tenir. »*). Le groom ne la propose donc pas comme solution.

La seconde borne, celle de mika#2693 — **429 lignes par vol** — n'est pas
atteinte. C'est la borne de Vincent, plus serrée, qui mord.

**La décision appartient à Vincent** : lever la borne, recouper le vol, ou
arbitrer autrement le mécanisme de `teams/mod.rs`. Le groom n'en choisit aucune.

---

## Contexte

Depuis V4-b2 (#3111, mergée et déployée le 2026-11-19), aucune ligne de
production de `mika-cli` ne lance un agent, une équipe ou le moteur de tâches.
Ce sont des **gardes textuelles** qui le tiennent : des scans de source dans
`crates/mika-cli/src/commands/chat_turn.rs` et
`crates/mika-cli/src/commands/team_runs_remote.rs`, qui rougissent sur un
aiguillage lexical.

Ce vol remplace la ceinture textuelle par une **bretelle structurelle** : rendre
ces appels incompilables hors de `mika-agent`. C'est l'AC5 d'origine du ticket —
*« structural enforcement, not prompt-level »*. Il ne change aucun comportement.
Les gardes textuelles existantes **ne bougent pas** : elles restent la ceinture.

### Ce qui est sur main (mesuré sur `f958c36e`)

- `crates/mika-agent/src/lib.rs` déclare `agent_loop`, `task_engine`, `teams` et
  `tool_execution` en `pub mod`, et réexporte `agent_loop` entier par
  `pub mod agent { pub use crate::agent_loop::*; }` (l. 4–7).
- `tool_execution::process_tool_calls` est **déjà** `pub(crate)`.
- Les onze fonctions du défaut 44 sont `pub`. Leurs seuls appelants hors de
  `mika-agent` sont les tests d'intégration du crate, sous
  `crates/mika-agent/tests/`.
- `mika-agent` n'a qu'une feature, `telemetry`.
- `crates/mika-cli/Cargo.toml` déclare `mika-agent.workspace = true` **nu**, sans
  clé `features` : il n'existe aujourd'hui **aucune** violation de la garde du
  critère 5.
- `clippy.toml` ne configure que `await-holding-invalid-types` : le seuil
  `too-many-arguments` reste à 7.
- `.github/workflows/ci.yml` l. 295 lance `cargo check --locked --target
  "$TARGET" -p mika-agent -p mika-cli`, **sans feature et sans `-D warnings`** ;
  l. 76 lance `cargo clippy --all-targets --all-features -- -D warnings`.

### Les treize points d'entrée du défaut 44, localisés

> **Onze au groom, treize depuis l'amendement du 2026-11-19 (bis)** ci-dessous.
> Les lignes 12 et 13 ont été ajoutées par le bearing de Prime ; partout ailleurs
> dans ce plan, « les onze » se lit « les treize ».

| # | fonction | fichier | ligne | appelants internes |
|---|---|---|---|---|
| 1 | `run_agent` | `agent_loop/mod.rs` | 6059 | `server/a2a.rs:368`, `server/handlers.rs:2222` |
| 2 | `run_agent_with_deadline` | `agent_loop/mod.rs` | 6174 | **aucun** |
| 3 | `run_silent_agent` | `agent_loop/mod.rs` | 7513 | `task_engine/dispatcher.rs:1171, 2172, 2599, 3375, 4654` |
| 4 | `run_silent_agent_with_deadline` | `agent_loop/mod.rs` | 7544 | **aucun** |
| 5 | `run_team_agent` | `agent_loop/mod.rs` | 8445 | `teams/engine.rs:1839, 2299`, `tools/delegate_task.rs:323`, test unitaire `agent_loop/tests/heritage_dequipe.rs:98` |
| 6 | `teams::run_team` | `teams/mod.rs` | 28 | `server/state.rs:441`, `tools/run_team.rs:128` |
| 7 | `teams::resume_team_run` | `teams/mod.rs` | 70 | `task_engine/dispatcher.rs:2445` |
| 8 | `TaskEngine::new` | `task_engine/engine.rs` | 949 | `server/mod.rs:770, 2378` + ~60 sites du `mod tests` du même fichier |
| 9 | `TaskEngine::startup_recovery` | `task_engine/engine.rs` | 1005 | `server/mod.rs:2225` |
| 10 | `TaskEngine::spawn_tick_loop` | `task_engine/engine.rs` | 1386 | `server/mod.rs:2247` |
| 11 | `TaskEngine::tick` | `task_engine/engine.rs` | 1407 | `engine.rs:1394`, depuis `spawn_tick_loop` |
| 12 | `TaskDispatcher::check_and_dispatch_parent` | `task_engine/dispatcher.rs` | 847 | `server/handlers.rs:759, 812, 819`, `task_engine/engine.rs:1674, 5632` |
| 13 | `TaskDispatcher::dispatch` | `task_engine/dispatcher.rs` | 867 | `dispatcher.rs:862` (depuis la 12), `task_engine/engine.rs:5572` |

`heartbeat`, `enqueue` et `cancel` de `TaskEngine` ne sont **pas** des treize et
ne bougent pas. Les quatre binaires de `mika-agent` (`mika-spirit`, `calibrate`,
`eval-diff`, `verify_bundled_skills`) n'appellent **aucune** des onze — vérifié
par scan de `crates/mika-agent/src/bin/` — donc `cargo build -p mika-agent --bin
mika-spirit` reste vert sans feature, bien qu'un binaire du même paquet soit un
crate externe qui ne voit pas les items `pub(crate)` de la lib.

---

## Requirements

1. Sans la feature `test-support`, les onze fonctions ne sont pas joignables
   depuis un crate externe ; avec elle, elles le sont.
2. Aucun corps de fonction ne change, aucun appelant interne ne change.
3. Les structures de paramètres, les types, les constantes et les aides pures
   restent publics.
4. La feature est éteinte par défaut, et allumée pour les tests du crate par une
   dev-dépendance de `mika-agent` sur lui-même.
5. Une garde de `mika-cli` refuse que son `[dependencies]` allume `test-support`
   sur `mika-agent`, avec témoin et contre-témoin.
6. Aucune dépendance externe neuve, aucun `allow` neuf, aucune attente
   (`#[ignore]`, `#[should_panic]` différé, TODO reporté).
7. Aucun fichier sous `crates/mika-agent/tests/` ne change.

---

## Périmètre — ce qui change, et rien d'autre

**Fichiers touchés, et aucun autre :**

- `crates/mika-agent/Cargo.toml`
- `Cargo.lock` si la dev-dépendance l'exige
- `crates/mika-agent/src/agent_loop/mod.rs`
- `crates/mika-agent/src/teams/mod.rs`
- `crates/mika-agent/src/task_engine/engine.rs`
- `crates/mika-agent/src/task_engine/tick_cli_guard_scan.rs` (**amendement du
  2026-11-19, voir ci-dessous**)
- `crates/mika-agent/src/task_engine/dispatcher.rs` (**amendement du 2026-11-19
  (bis), voir ci-dessous**)
- `crates/mika-cli/tests/garde_cargo_feature_test_support.rs` (**neuf**)
- `crates/mika-agent/CLAUDE.md`
- `crates/mika-cli/CLAUDE.md`
- ce plan

**Hors du vol, et non négociable dans le plan :** tout changement de
comportement ; `.github/workflows/` ; `docs/egress/` ; les fichiers sous
CODEOWNERS ; les quatre accès locaux du défaut 47 ; les gardes textuelles
existantes, qui ne bougent pas ; tout fichier sous `crates/mika-agent/tests/`.

### Amendement du 2026-11-19 — `tick_cli_guard_scan.rs` entre au périmètre

**Le point.** Le plan ne nommait pas
`crates/mika-agent/src/task_engine/tick_cli_guard_scan.rs`, et le critère 10 ne
l'autorisait pas (il ne porte aucune des quatre méthodes). Il est ajouté au
périmètre, et le critère 10 est lu comme l'incluant.

**La raison, mesurée.** Ce fichier porte la garde de mika#3042 : elle extrait le
corps de `tick` par scan de source, ancrée sur la signature littérale
`TICK_SIGNATURE = "pub async fn tick(&mut self) {"`, et vérifie qu'aucun
balayage terminal de parente `self_dev` n'y vit hors de `if
!self.dispatcher.cli_mode`. La forme (i) renomme le corps en `tick_verrouille`
et laisse sous la signature d'origine une coquille d'une ligne. La garde a donc
rougi — `ac3_le_scan_echoue_ferme_plutot_que_de_se_taire`, sur l'ancre « le
corps extrait doit aller jusqu'au bas du bloc de scan DB ». **C'est son travail
exact** : elle est fail-closed précisément pour qu'un `tick` renommé ne la laisse
pas scanner un texte vide en silence (classe mika#2205).

**Ce qui change, et ce qui ne change pas.** `TICK_SIGNATURE` désigne désormais
`async fn tick_verrouille(&mut self) {`, c'est-à-dire **le même corps réel**
qu'avant le vol. La fixture de contrôle `TICK_SANS_GARDE` et les deux assertions
qui citent la signature sont réalignées sur ce nom. Le prédicat, le recensement
fermé `TERMINAL_PARENT_SWEEPS`, les six sites du contrôle et les deux moitiés
fail-closed ne bougent pas : la garde surveille après le vol exactement ce
qu'elle surveillait avant. Ce n'est pas un désarmement, et ce n'est pas une
exception ajoutée — c'est le réancrage qu'imposait son propre rouge.

**Ce que ce n'est pas.** Ce n'est pas une halte. Les haltes de ce vol sont la
borne de lignes et un `verify-pipeline` rouge ; un écart de périmètre se traite
par cet amendement, conformément à la tête du ticket (*« il amende le plan sur la
branche dans un commit `docs(plans)` avant de pousser le code qui s'en
écarte »*). Ce n'est pas non plus une des « gardes textuelles existantes qui ne
bougent pas » : celles-là sont les scans de `mika-cli` (`chat_turn.rs`,
`team_runs_remote.rs`) qui tiennent la ceinture du vol, et ils ne sont pas
touchés.

**Coût.** 11 lignes ajoutées, toutes du commentaire qui porte la raison du
réancrage ; aucune assertion retirée. Poste R8 au tableau de taille.

### Amendement du 2026-11-19 (bis) — `TaskDispatcher::dispatch` et `check_and_dispatch_parent` entrent au verrou

**Le point.** Le verrou s'étend à deux méthodes de
`crates/mika-agent/src/task_engine/dispatcher.rs` — `TaskDispatcher::dispatch`
(l. 867) et `TaskDispatcher::check_and_dispatch_parent` (l. 847) — sous la forme
(i), celle déjà employée pour `TaskEngine::new`. Les onze points d'entrée
deviennent **treize**, et `dispatcher.rs` entre au périmètre des fichiers
touchés. Source : le **bearing de Prime du 2026-11-19, qui vaut le mot de
Vincent**, porté au corps du ticket sous *« Complément du 2026-11-19 »*.

**L'écart à la liste d'origine, nommé.** La tête du ticket nomme, pour le moteur
de tâches, `TaskDispatcher::dispatch` et `TaskDispatcher::check_and_dispatch_parent`.
Le plan avait retenu à leur place `TaskEngine::startup_recovery` et
`TaskEngine::tick`, par un raisonnement sur *ce qui lance le moteur* plutôt que
sur *ce qui lance une tâche* : `new`, `startup_recovery`, `spawn_tick_loop` et
`tick` sont le cycle de vie du moteur, et c'est ce cycle que la TUI faisait
tourner dans son propre processus (`chat.rs:183-269`, mesure du corps du
ticket). Les deux méthodes du dispatcher ne sont pas ce cycle : elles sont le
geste unitaire qu'il appelle. **Le bearing tranche que les deux lectures sont
dues** — le verrou ne porte pas seulement sur le démarrage du moteur mais aussi
sur le lancement d'une tâche, qui est joignable sans lui. La liste n'est donc
pas corrigée mais **élargie** : les quatre méthodes de `TaskEngine` restent au
verrou, les deux du dispatcher s'y ajoutent.

**Ce qui change, et ce qui ne change pas.** Chacune des deux porte désormais
deux déclarations sous `#[cfg]` déléguant à un corps privé `*_verrouille`
inchangé, byte pour byte. **La structure `TaskDispatcher` et tous ses champs
restent `pub`** — le verrou porte sur deux méthodes, jamais sur le type, que
`mika-cli` construit encore. Aucun appelant interne ne change : les trois sites
de `server/handlers.rs` (l. 759, 812, 819) et les trois de `task_engine/engine.rs`
(l. 1674, 5572, 5632) sont intra-crate et voient le bras `pub(crate)`. Les deux
fichiers de `crates/mika-agent/tests/` qui appellent `dispatch`
(`eval/test_credit_hold_2944.rs`, `eval/test_callback_delivery_starvation.rs`)
restent couverts par la dev-dépendance sur soi-même, qui leur allume
`test-support` : **aucun fichier sous `crates/mika-agent/tests/` ne change**, le
critère 4 tient.

**Pourquoi la forme (i) et pas la (ii).** `dispatch` prend un paramètre,
`check_and_dispatch_parent` un aussi : aucune des deux n'approche le seuil
clippy de 7, donc aucun `#[allow]` neuf n'est dû. C'est exactement le cas des
quatre méthodes de `TaskEngine` (§ *Mécanisme*, R5), et la raison invoquée pour
la forme (ii) dans `teams/mod.rs` — 12 paramètres — ne s'applique pas ici.

**Pas de `dead_code`.** Les deux ont des appelants internes (tableau ci-dessus),
donc leur bras `pub(crate)` est joint. Le traitement à déclaration unique réservé
à `run_agent_with_deadline` et `run_silent_agent_with_deadline` (§ R3) ne les
concerne pas.

**Lecture du critère 10.** Il autorise *« les fichiers de `task_engine/` qui
portent les quatre méthodes »*. `dispatcher.rs` est sous `task_engine/` et porte
deux des treize : le critère est lu comme l'incluant, l'élargissement venant du
bearing qui l'a produit.

**Ce que ce n'est pas.** Ce n'est pas une halte. Les bornes ne changent pas :
halte du pilote à `480`, mesurée après chaque commit. Ce n'est pas non plus une
réouverture du groom — le bearing est une décision opérateur, pas une
réestimation du périmètre groomé.

**Coût.** 24 lignes ajoutées (poste R9), plus la mise à jour du compte « onze »
dans les deux `CLAUDE.md` (R7 passe de 28 à 30).

---

`crates/mika-agent/src/lib.rs` n'est **pas** touché, et c'est une conclusion, pas
un oubli. Le réexport glob `pub mod agent { pub use crate::agent_loop::*; }`
n'erre pas sur un item `pub(crate)` : il ne l'exporte simplement pas. Le critère
1 (*« joignable par aucun chemin public, `mika_agent::agent::*` compris »*) tient
donc sans y toucher. Passer le module entier en `pub(crate)` aurait au contraire
violé le critère 2, qui exige que les types et les structures de paramètres
restent publics.

---

## Mécanisme, fichier par fichier

### Le choix de forme, et pourquoi il n'y en a pas de troisième

Rust ne permet pas d'**élargir** une visibilité par réexport : `pub use` d'un
item `pub(crate)` est E0364/E0365. Deux formes natives restent :

- **(i) Deux déclarations fines sous `#[cfg]`**, déléguant à un corps privé
  renommé. Coût ≈ 11 lignes ajoutées par fonction, corps byte-pour-byte
  identique. Convient aux signatures courtes.
- **(ii) Un module privé portant les items `pub`**, plus deux `use` réexportés
  sous `#[cfg]`. Coût = la réindentation de la région, plus ≈ 13 lignes
  d'échafaudage. Convient quand les signatures sont longues ou portent des
  attributs qu'il faudrait dupliquer.

### `crates/mika-agent/Cargo.toml` — la feature et la dev-dépendance

```toml
[features]
telemetry = ["mika-common/telemetry"]
# Ouvre les onze points d'entrée du défaut 44 aux crates externes (mika#1727
# vol V4-c). Éteinte par défaut : hors de mika-agent, un lancement d'agent,
# d'équipe ou du moteur de tâches ne compile pas. Allumée pour les tests du
# crate par la dev-dépendance sur soi-même ci-dessous.
test-support = []
```

```toml
[dev-dependencies]
# Dev-dépendance sur soi-même : la seule façon d'allumer une feature pour les
# cibles de test du paquet sans l'allumer pour sa lib ni pour ses binaires.
# Un `#[cfg(any(test, feature = "..."))]` ne suffirait pas : un fichier de
# `tests/` est un crate séparé qui ne voit pas le `cfg(test)` de la lib dont il
# dépend. Sous les résolveurs v2/v3 (edition 2024), les features d'une
# dev-dépendance s'unifient dans les builds qui incluent les cibles de test
# (`cargo test`, `--all-targets`) et PAS dans `cargo check -p mika-agent` ni
# dans `cargo build`.
mika-agent = { path = ".", features = ["test-support"] }
```

### `agent_loop/mod.rs` — trois doubles visibilités, et deux cas à part

`run_agent`, `run_silent_agent` et `run_team_agent` prennent **un** paramètre
chacune : forme (i).

```rust
#[cfg(not(feature = "test-support"))]
pub(crate) async fn run_agent(params: &AgentParams<'_>) -> Result<AgentOutput> {
    run_agent_verrouille(params).await
}

#[cfg(feature = "test-support")]
pub async fn run_agent(params: &AgentParams<'_>) -> Result<AgentOutput> {
    run_agent_verrouille(params).await
}

/// Corps historique, inchangé. Le renommage existe pour que les deux
/// déclarations ci-dessus ne dupliquent pas 115 lignes de corps.
async fn run_agent_verrouille(params: &AgentParams<'_>) -> Result<AgentOutput> {
    // … corps inchangé …
```

Un commentaire d'ouverture, posé une seule fois dans le fichier, porte la raison
de la forme (E0364/E0365, et pourquoi pas de réexport).

**Les deux fonctions sans appelant interne — micro-écart nommé au critère 1.**
`run_agent_with_deadline` et `run_silent_agent_with_deadline` n'ont aucun
appelant dans le crate. Écrire leur bras `pub(crate)` créerait une fonction
jointe par personne : `dead_code` est `warn` par défaut, et il n'y a ni
`RUSTFLAGS` global ni `-D warnings` sur le `cargo check` sans feature — mais
`cargo clippy --all-targets --all-features -- -D warnings` (ci.yml l. 76) tourne
**avec** `--all-features`, donc avec `test-support`, donc sur le bras `pub`. Le
risque n'est pas le rouge de CI ; c'est d'introduire un `#[allow(dead_code)]`
pour le taire, ce que le critère 7 interdit.

Ces deux fonctions portent donc une **unique** déclaration :

```rust
#[cfg(feature = "test-support")]
pub async fn run_agent_with_deadline(
```

Sans la feature, la fonction **n'existe pas** — strictement plus fort que
`pub(crate)`, aucun avertissement, aucun `allow`. C'est un écart littéral au
critère 1, qui dit « `pub(crate)` sans la feature ». Il est nommé ici, avec sa
raison, et il satisfait la propriété que le critère protège (*« n'est joignable
par aucun chemin public »*) a fortiori.

**Le docstring de `run_agent_with_deadline` est périmé par ce vol** et doit être
réécrit dans le même commit. Il dit aujourd'hui que le `cfg`-gating a été
*rejeté* parce qu'il aurait demandé « an awkward self-dev-dep ». V4-c livre
exactement cette dev-dépendance : le paragraphe affirme le contraire de l'état du
dépôt. Un plan qui dit une chose pendant que le code en fait une autre est bloqué
par la QA.

### `teams/mod.rs` — forme (ii), module privé

`run_team` et `resume_team_run` portent **12 paramètres** chacune et déjà un
`#[allow(clippy::too_many_arguments)]`. La forme (i) demanderait quatre bras
redéclarant 12 paramètres et transférant 12 arguments — et **quatre `allow`
neufs**, que le critère 7 interdit, le seuil clippy étant à 7.

```rust
mod verrou {
    // … les lignes 19 à 122 d'aujourd'hui, réindentées d'un cran,
    //     `run_team` et `resume_team_run` restant `pub` ici …
}

// Deux visibilités pour un seul corps (mika#1727 V4-c) : la forme à deux
// déclarations `#[cfg]` coûterait quatre redéclarations de 12 paramètres et
// autant d'`#[allow(clippy::too_many_arguments)]` neufs.
#[cfg(not(feature = "test-support"))]
pub(crate) use verrou::{resume_team_run, run_team};
#[cfg(feature = "test-support")]
pub use verrou::{resume_team_run, run_team};
```

Les deux `#[allow(clippy::too_many_arguments)]` existants restent sur les
fonctions, dans le module : ce ne sont pas des `allow` neufs.

Région concernée : lignes 19 à 122, soit 104 lignes dont **96 non vides** (les
blanches sont aux lignes 46, 62, 91, 95, 99, 101, 106, 120). Ce sont ces 96
lignes qui comptent comme ajoutées.

### `task_engine/engine.rs` — quatre doubles visibilités

Les quatre méthodes prennent 0 à 2 paramètres : forme (i), dans le bloc `impl`.
Les ~60 appels à `TaskEngine::new` du `mod tests` du même fichier sont
intra-crate et ne sont pas affectés.

### `crates/mika-cli/tests/garde_cargo_feature_test_support.rs` — la garde

Un fichier **neuf** sous `crates/mika-cli/tests/`, découvert automatiquement par
Cargo. `scripts/check-suite-wiring.sh` ne gouverne que `scripts/` et
`skills/bundled/_shared/` : aucune entrée de registre n'est due.

Le prédicat est **pur** — `fn allume_test_support(manifeste: &str) -> bool` — ce
qui permet au contre-témoin de lui passer un `Cargo.toml` d'essai sous forme de
littéral, sans fixture sur disque, sans `tempfile`, sans
`.pilot-scratch/`.

Trois tests :

- **La garde.** Lit `crates/mika-cli/Cargo.toml` par
  `env!("CARGO_MANIFEST_DIR")`, affirme `!allume_test_support(...)`.
- **Le témoin.** Affirme que l'entrée `mika-agent` de `[dependencies]` a bien été
  **trouvée** — un prédicat qui rend `false` parce qu'il n'a rien lu n'est pas
  une garde. Le parseur doit accepter les deux formes : `mika-agent.workspace =
  true` (la forme d'aujourd'hui, nue) et la forme table en ligne
  `mika-agent = { workspace = true, features = [...] }`.
- **Le contre-témoin.** Un manifeste littéral portant `mika-agent = { workspace
  = true, features = ["test-support"] }` dans `[dependencies]` : le prédicat
  rend `true`.

**Aucune dépendance neuve.** `toml.workspace = true` est déjà dans le
`[dependencies]` de `crates/mika-cli/Cargo.toml` (l. 36), et Cargo rend les
`[dependencies]` d'un paquet disponibles à ses cibles de test. Critère 7 tenu.

---

## Ordre de commit

Chaque commit compile et passe ses tests. Chaque changement arrive avec ses
tests dans le même commit. Après chaque commit, le pilote mesure.

1. **`feat(1727): le verrou des onze points d'entrée (vol V4-c)`**
   `crates/mika-agent/Cargo.toml` (feature + dev-dépendance sur soi),
   `Cargo.lock` si nécessaire, les onze doubles visibilités dans
   `agent_loop/mod.rs`, `teams/mod.rs` et `task_engine/engine.rs`, et la
   réécriture du docstring périmé de `run_agent_with_deadline`.
   Vert attendu : `cargo check -p mika-agent -p mika-cli`,
   `cargo build -p mika-agent --bin mika-spirit`, `cargo test -p mika-agent`.
2. **`test(1727): une garde refuse test-support dans les dépendances de mika-cli (vol V4-c)`**
   `crates/mika-cli/tests/garde_cargo_feature_test_support.rs`, avec son témoin
   et son contre-témoin.
   Vert attendu : `cargo test -p mika-cli`.
3. **`docs(1727): le défaut 48 (vol V4-c)`**
   `crates/mika-agent/CLAUDE.md` et `crates/mika-cli/CLAUDE.md`.
   Vert attendu : `cargo test -p mika-common --test claude_md_size`.

---

## Verification Contract

| commande | attendu | ce qu'elle prouve |
|---|---|---|
| `cargo check -p mika-agent -p mika-cli` | vert | critère 3 — le verrou ne casse pas la compilation sans feature |
| `cargo build -p mika-agent --bin mika-spirit` | vert | critère 3 — les binaires du paquet, crates externes de la lib, ne touchent pas aux onze |
| `cargo test -p mika-agent` | vert | critère 4 — la dev-dépendance rallume bien la feature pour les tests d'intégration |
| `git diff --stat origin/main...HEAD -- crates/mika-agent/tests/` | **vide** | critère 4 — aucun fichier de `tests/` ne change |
| `cargo test -p mika-cli` | vert | critère 5 — garde, témoin, contre-témoin |
| `cargo test -p mika-common --test claude_md_size` | vert | critère 9 — les deux `CLAUDE.md` tiennent sous 100 000 o |
| `cargo clippy --all-targets --all-features -- -D warnings` | vert | critère 11 — y compris le bras `pub` sous `test-support` |
| `bash scripts/verify-egress-manifest.sh --report` | vert | critère 8 |
| `bash scripts/verify-pipeline.sh origin/main` | vert, **avant** d'ouvrir la PR | critère 11 ; rouge ⇒ le pilote s'arrête et le dit |
| `git diff --shortstat origin/main...HEAD -- . ':!docs/plans'` | ≤ 420 après chaque commit | halte du pilote |

**Les trois mutations du critère 6**, chacune faite seule dans un worktree
détaché, sortie collée dans la PR :

1. Ajouter `mika_agent::agent::run_silent_agent(` dans la production de
   `mika-cli` ⇒ `cargo check -p mika-cli` **rouge** (E0603, item privé).
2. Ajouter `mika_agent::task_engine::TaskEngine::new(` ⇒ idem.
3. Ajouter `features = ["test-support"]` sur `mika-agent` dans `[dependencies]`
   de `mika-cli` ⇒ la garde du critère 5 **rouge**.

**Quatrième mutation, due par l'amendement du 2026-11-19 (bis).** Un appel à
`mika_agent::task_engine::TaskDispatcher::dispatch` depuis
`crates/mika-cli/src/lib.rs` ⇒ `cargo check -p mika-cli` **rouge**. Le code
attendu est **E0624** (*« method is private »*), et non E0603 comme aux
mutations 1 et 2 : une méthode `pub(crate)` d'un type `pub` se refuse à
l'appel, là où une fonction libre `pub(crate)` se refuse au chemin. Cette
différence de code est ce qui atteste que c'est bien la **méthode** qui est
verrouillée et non le type, lequel reste public par décision du bearing. Sortie
recopiée dans la PR.

**Portée honnête du verrou, dite plutôt que surestimée.** Le verrou mord sur le
jeu de features par défaut : `cargo check -p mika-cli`, qui est exactement ce que
ci.yml l. 295 lance et ce que le critère 6 nomme. Sous `--all-features` ou sous
`cargo test`, les dev-dépendances sont construites et les features s'unifient :
un appel fautif y compilerait. Le critère 6 est donc tenu au sens littéral de son
énoncé, et les gardes textuelles existantes — qui ne bougent pas — restent la
ceinture qui couvre ces configurations.

---

## Definition of Done

- Les onze fonctions ne sont joignables depuis aucun crate externe sans
  `test-support`, et le sont avec.
- Aucun corps de fonction et aucun appelant interne n'a changé.
- `crates/mika-agent/Cargo.toml` déclare `test-support`, éteinte par défaut, et
  la dev-dépendance sur soi-même qui l'allume pour les tests du crate.
- `crates/mika-cli/tests/garde_cargo_feature_test_support.rs` existe, est armé,
  et porte son témoin et son contre-témoin.
- Les deux `CLAUDE.md` portent le défaut 48, sous leurs plafonds d'octets.
- Les dix commandes du Verification Contract sont vertes, et les trois mutations
  du critère 6 sont exécutées et collées dans la PR.
- La PR porte `Refs mika#1727` et aucun verbe de fermeture.

---

## Acceptance criteria

Transcrits mot pour mot depuis le corps du ticket.

1. Sans `test-support`, les onze fonctions du défaut 44 sont `pub(crate)` ; avec
   elle, `pub`. Leurs corps ne changent pas. Une fonction ainsi verrouillée n'est
   joignable par aucun chemin public, `mika_agent::agent::*` compris.
2. Les structures de paramètres, les types, les constantes et les aides pures
   restent publics. `mika tasks`, `tui/app.rs`, `chat.rs` et `team_outcome.rs`
   compilent sans changement.
3. `cargo check -p mika-agent -p mika-cli`, sans feature, est vert.
   `cargo build -p mika-agent --bin mika-spirit` aussi.
4. `cargo test -p mika-agent` est vert, et aucun fichier sous
   `crates/mika-agent/tests/` ne change.
5. Une garde de `mika-cli` échoue si son `Cargo.toml` allume `test-support` sur
   `mika-agent` dans `[dependencies]`, avec un témoin et un contre-témoin.
6. Trois mutations, chacune faite seule dans un worktree détaché, avec la sortie
   dans la PR :
   - un appel à `mika_agent::agent::run_silent_agent(` ajouté dans la production
     de `mika-cli` fait échouer `cargo check -p mika-cli` ;
   - un appel à `mika_agent::task_engine::TaskEngine::new(` fait de même ;
   - `features = ["test-support"]` ajouté sur `mika-agent` dans `[dependencies]`
     de `mika-cli` fait rougir la garde du critère 5.
7. Aucune dépendance externe neuve, aucun `allow` neuf, aucune attente.
8. `.github/workflows/`, `docs/egress/` et les fichiers sous CODEOWNERS ne
   changent pas. `bash scripts/verify-egress-manifest.sh --report` est vert.
9. `crates/mika-agent/CLAUDE.md` porte le paragraphe (1 500 octets au plus) et
   `crates/mika-cli/CLAUDE.md` la phrase (600 octets au plus).
   `cargo test -p mika-common --test claude_md_size` est vert.
10. `git diff origin/main...HEAD` ne touche que `crates/mika-agent/Cargo.toml`,
    `Cargo.lock` si la dev-dépendance le demande, `agent_loop/mod.rs`, les
    fichiers de `task_engine/` qui portent les quatre méthodes, `teams/mod.rs`,
    le fichier de la garde dans `mika-cli`, les deux `CLAUDE.md` et le plan.
11. `cargo clippy --all-targets --all-features -- -D warnings` est vert. Le
    pilote lance `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la
    PR. S'il le trouve rouge, il s'arrête et le dit.
12. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
    compris si le gabarit de reprise l'ouvre. La fermeture est le geste de
    samidarko au merge, avec le commentaire du défaut 47.

**Écart nommé au critère 1.** `run_agent_with_deadline` et
`run_silent_agent_with_deadline` n'ont aucun appelant interne. Elles portent une
**unique** déclaration `#[cfg(feature = "test-support")] pub` au lieu du couple
`pub(crate)` / `pub` : sans la feature, la fonction n'existe pas, ce qui est
strictement plus fort que `pub(crate)` pour la propriété que le critère protège,
et évite l'`#[allow(dead_code)]` que le critère 7 interdit. Raison complète au
§ *Mécanisme*.

---

## Fire-Disposition

Ce plan livre un **détecteur** : la garde du `Cargo.toml` de `mika-cli`
(critère 5). Disposition retenue, parmi les trois formes canoniques de
mika#1574 : **(a) exception d'allowlist nommée — livrée avec zéro exception, la
garde armée.**

**Mesure sur `f958c36e` : il n'existe aucune violation préexistante.**
`crates/mika-cli/Cargo.toml` l. 22 déclare `mika-agent.workspace = true` **nu**,
sans clé `features`. La garde ne rougit donc sur rien au merge, et aucune
exception n'est à écrire.

**Aucune constante d'allowlist n'est introduite, et c'est une conclusion.** Le
domaine du prédicat est **une entrée de dépendance**, pas une population de
fichiers : une allowlist y serait une liste sans membre possible, du cargo-culte
du motif plutôt que le motif. Les scans de source du dépôt
(`chat_turn.rs § V1_ALLOWED`, `V2A_ALLOWED`) en portent une parce qu'ils
balaient des fichiers ; ici il n'y a qu'un datum.

**Ce qu'on fait si la garde rougit un jour.** On s'arrête et on escalade : une
entrée d'exception sur ce prédicat reviendrait à allumer `test-support` en
production de `mika-cli`, c'est-à-dire à désarmer le verrou que ce vol existe
pour poser. Le contre-témoin est ce qui garantit que le rouge est réel et non une
panne de lecture.

---

## Taille estimée

| livrable | lignes ajoutées |
|---|---|
| R1 — feature `test-support`, dev-dépendance sur soi, leurs commentaires (`crates/mika-agent/Cargo.toml`) | 13 |
| R2 — trois doubles visibilités dans `agent_loop/mod.rs` (`run_agent`, `run_silent_agent`, `run_team_agent`) + commentaire de forme | 44 |
| R3 — les deux fonctions sans appelant interne + réécriture du docstring périmé de `run_agent_with_deadline` | 13 |
| R4 — `teams/mod.rs` : module privé `verrou` + deux `use` sous `#[cfg]` (96 lignes réindentées + 13 d'échafaudage) | 109 |
| R5 — quatre doubles visibilités dans `task_engine/engine.rs` + commentaire de forme | 51 |
| R6 — `crates/mika-cli/tests/garde_cargo_feature_test_support.rs` : prédicat, garde, témoin, contre-témoin | 54 |
| R7 — le défaut 48 : `crates/mika-agent/CLAUDE.md` et `crates/mika-cli/CLAUDE.md` | 30 |
| R8 — réancrage de la garde mika#3042 sur le corps réel de `tick` (`tick_cli_guard_scan.rs`), amendement du 2026-11-19 | 11 |
| R9 — deux doubles visibilités dans `task_engine/dispatcher.rs` (`dispatch`, `check_and_dispatch_parent`) + commentaire de forme, amendement du 2026-11-19 (bis) | 24 |

Total estimé : 312 lignes au groom ; 349 avec R8 et R9

**Ni R8 ni R9 ne rouvrent le groom.** Le `Total estimé : 312` est le chiffre sur
lequel Vincent a tranché le 2026-11-19 en levant la borne du groom à 312. R8 est
un surcoût **constaté à l'exécution** — la garde de mika#3042 a rougi sur le
renommage du corps de `tick` — et non une réestimation du périmètre groomé. R9
est un **élargissement décidé par le bearing de Prime du 2026-11-19**, qui vaut
le mot de Vincent et qui dit explicitement que les bornes ne changent pas. Les
deux sont portés ici pour que le compte mesuré et le compte annoncé se
répondent. La halte du pilote (480, levée du 2026-11-19) n'est pas approchée.
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Unité.** La ligne **ajoutée**, plan exclu, code, commentaires, doc et tests
compris. Les retraits ne comptent pas. La commande est, seule sur sa ligne et
sans rien après elle :

```
git diff --shortstat origin/main...HEAD -- . ':!docs/plans'
```

Le compte est le nombre devant `insertions(+)`. Si cette forme est refusée par la
politique des pilotes, le pilote s'arrête et le dit : il ne cherche pas une autre
forme.

Le tableau **inclut** la ligne des deux `CLAUDE.md`, contrairement au gabarit
générique de mika#2636, parce que la borne de ce vol est posée sur une unité qui
compte la doc (*« avec le code, les commentaires, la doc et les tests »*) — et
parce que c'est le précédent déjà retenu pour ce ticket, au vol V4-b2
(`docs/plans/2026-11-19-001-…`, ligne `crates/mika-cli/CLAUDE.md`).

**Borne du groom : 300, posée par Vincent pour ce vol (2026-11-19).**
`Total estimé : 312` la dépasse. Conformément à la tête du ticket — *« Si ce
total dépasse `300`, le groom s'arrête et le dit. Il ne recoupe pas lui-même. »*
— **le groom s'arrête**. Voir la section HALTE en tête de ce plan. Le groom ne
choisit aucune coupe et n'en propose aucune : la décision appartient à Vincent.

**Halte du pilote : 420, posée par Vincent (2026-11-19).** Le plan ne la change
pas et n'en ajoute aucune. Après chaque commit, le pilote mesure ; au-dessus de
420 il s'arrête — pas de commit suivant, pas de push, pas de PR — et son dernier
message dit « HALTE borne : N lignes ajoutées pour `420` », avec le compte après
chaque commit.

**Écarts nommés aux indications du MPC** (indicatives, non liantes : 160 à 275
au total) :

- *Onze doubles visibilités : 90 à 140 indiqué, 217 estimé* (R2 + R4 + R5). Tout
  l'écart vient de `teams/mod.rs`. L'indication suppose une forme à ≈ 11 lignes
  par fonction, vraie pour neuf des onze ; les deux de `teams/` portent 12
  paramètres et un `allow` existant, ce qui exclut cette forme (voir
  § *Mécanisme*).
- *Traitement des deux fonctions sans appelant interne : 0 à 10 indiqué, 13
  estimé* (R3). Le surplus est la réécriture du docstring de
  `run_agent_with_deadline`, que ce vol rend faux et qu'il doit donc corriger.
- Les quatre autres postes tiennent dans leurs fourchettes.

---

## Risques et notes de dérive

- **La réindentation de `teams/mod.rs` est le poste qui porte le dépassement.**
  Si Vincent arbitre autrement — par exemple en autorisant des `allow` neufs, ou
  en relevant `too-many-arguments-threshold` dans `clippy.toml` — le poste tombe
  à ≈ 30 lignes et le total sous la borne. Le groom ne tranche pas : relever le
  seuil clippy touche un fichier que le périmètre ne nomme pas, et autoriser des
  `allow` neufs contredit le critère 7.
- **La dev-dépendance d'un paquet sur lui-même est une forme supportée par
  Cargo**, pas un contournement. Elle n'introduit aucune version neuve au
  verrou ; `Cargo.lock` peut gagner une arête, ce que le critère 10 autorise
  explicitement.
- **`cargo clippy --all-features` voit le bras `pub`.** C'est voulu : c'est la
  seule configuration où les deux bras sont lintés. Si un bras `pub(crate)`
  devait un jour déclencher `dead_code`, la réponse est la forme à déclaration
  unique du § R3, jamais un `allow`.
- **Les ~60 appels à `TaskEngine::new` du `mod tests` de `engine.rs` sont
  intra-crate** et ne sont pas affectés par `pub(crate)`. Aucun d'eux ne change.
- **Le test unitaire `agent_loop/tests/heritage_dequipe.rs:98` appelle
  `run_team_agent`** : il vit sous `src/`, pas sous `crates/mika-agent/tests/`,
  donc il est intra-crate et le critère 4 n'est pas en jeu.
- **`crates/mika-cli/src/commands/chat.rs:1239` définit un `pub async fn
  run_team` local à la TUI** (mode équipe), homonyme mais distinct de
  `teams::run_team`. Ne pas le confondre en lisant un grep.
- **Marge des deux `CLAUDE.md`**, mesurée : `crates/mika-agent/CLAUDE.md`
  95 266 o (+ 1 500 ⇒ 96 766), `crates/mika-cli/CLAUDE.md` 93 994 o (+ 600 ⇒
  94 594). Plafond 100 000 o. Les deux passent.
- **Gel des tickets jusqu'au 01/12.** Ce vol n'ouvre aucun ticket, pas même de
  suivi ni de sous-ticket. Un manque ou un reste va en commentaire sur ce ticket
  ou dans la PR, noté `Tracked in: senara-solutions/mika#1727`.
- **Une halte ne se lève jamais par un amendement du plan.** Si une revue
  d'architecte demande de recouper ou d'étendre ce vol, le groom s'arrête et le
  dit, au lieu de réviser le plan dans ce sens.
