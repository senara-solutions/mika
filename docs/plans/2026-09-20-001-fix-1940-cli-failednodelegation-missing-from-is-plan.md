---
ticket: senara-solutions/mika#1940
type: fix
date: 2026-09-20
seq: 001
---

# Un statut terminal d'échec cesse d'être lisible par un prédicat que le compilateur ne vérifie pas — Plan

## Goal Capsule

Les trois surfaces CLI qui lisent `RunStatus` cessent d'énumérer les variantes à
la main. Elles passent par **une disposition exhaustive portée par l'enum**, si
bien qu'une septième variante ne pourra plus être ajoutée sans que chacune des
trois cesse de compiler. Corollaire mesurable et immédiat : `mika ask --team`
rend un code de sortie non nul sur `FailedNoDelegation` **et** sur
`FailedTransport`, laisse `stdout` vide plutôt qu'une réponse plausible, et
`mika chat --team` cesse d'écrire la réponse d'un échec dans l'historique de
conversation sous le rôle `assistant`.

## Product Contract

### Summary

mika#1676 (PR#1939) a ajouté la variante terminale `RunStatus::FailedNoDelegation`
et une revue multi-agents a relevé trois sites CLI qui ne la reconnaissent pas.
Le ticket les décrit comme « 3 correctifs mécaniques ». La lecture du code
déplace ce diagnostic sur deux points, et les deux changent le remède.

**(M1) Ce n'est pas une variante manquante, c'en est deux.** mika#1671 a ajouté
`RunStatus::FailedTransport(String)` — également terminale, également un échec,
également absente des trois prédicats. Le ticket ne la nomme pas. Un correctif
qui ajouterait un bras `FailedNoDelegation` à chacun des trois `matches!`
laisserait donc la moitié du défaut ouverte le jour même où il est déclaré
fermé.

**(M2) La ligne de partage n'est pas « qui a été mis à jour » mais « ce que le
compilateur peut vérifier ».** Recensement exhaustif des lecteurs de `RunStatus`
hors tests, sur tout le workspace :

| site | forme | a suivi les deux ajouts ? |
|---|---|---|
| `teams/types.rs:272` (`Display`) | `match` exhaustif | **oui** |
| `teams/engine.rs:668` (colonne DB) | `match` exhaustif | **oui** |
| `teams/notification.rs:27` | `match` exhaustif, sans bras `_` | **oui** |
| `mika-cli/commands/ask.rs:778` | `matches!(…, Failed(_))` | non |
| `mika-cli/commands/ask.rs:784` | `if let Failed(ref msg)` | non |
| `mika-cli/commands/chat.rs:1104` | `if let Failed(reason)` | non |

Trois `match` exhaustifs, trois mises à jour ; trois motifs écrits à la main,
zéro. Ce n'est pas une corrélation : `matches!` et `if let` sont exactement les
deux formes qui **n'échouent pas à compiler** quand une variante apparaît. Le
défaut est donc le même que celui que mika#2023 M2 a dû nommer par écrit à
propos de `tier == AgentTier::Family` — « une mine qu'aucune erreur de
compilation ne pouvait annoncer ». Le remède qui a la forme du défaut n'est pas
d'ajouter six bras, c'est de **retirer aux trois sites le droit d'énumérer**.

**(M3) Le dégât en mode chat est plus lourd que « affichage ».** Sur
`FailedNoDelegation`, le moteur pose `run.deliverable = Some(retry_reply)`
(`engine.rs:1131`). `chat.rs:1107` prend alors la branche `else`, envoie
`TeamEvent::Deliverable(retry_reply)`, et le TUI — `tui/app.rs:1258` —
**persiste ce texte en base** via `save_message("", "assistant", &text, None)`.
La fausse réponse n'est donc pas seulement affichée : elle devient un tour
`assistant` durable, relu comme contexte par les tours suivants. C'est le seul
des trois sites dont le dégât survit à la session.

**(M4) `--format json` n'était pas défectueux, et ça vaut d'être dit.**
L'enveloppe porte `team_run.status = format!("{}", run.status)`, donc
`"failed_no_delegation"` — parce que `Display` est un `match` exhaustif qui a
suivi. Le mode JSON discriminait déjà ; seuls le mode texte et le code de sortie
mentaient. C'est la même preuve que M2, vue de l'autre côté.

### Problem Frame

Le ticket est étiqueté p3 avec un déclencheur d'escalade écrit : « si le
scripting opérateur rencontre un faux succès en production → passer p2 ». Aucun
consommateur automatisé de `mika ask --team` n'existe aujourd'hui dans le dépôt
(recherche sur `skills/`, `scripts/`, `.claude/` : zéro appel, seulement de la
documentation). Le défaut est donc **latent** : il ne coûte rien tant que
personne ne scripte cette commande, et il coûte un faux succès silencieux le
jour où quelqu'un le fait. C'est exactement la forme qui justifie une réparation
structurelle plutôt qu'un rattrapage : le correctif est bon marché maintenant et
l'incident serait invisible plus tard.

Détail des trois sites, avec leur dérive de numérotation depuis la rédaction du
ticket (#1939 a été mergée le 2026-08-21 ; le fichier a bougé depuis) :

1. **`ask.rs:778`** (ticket : `:619`) —
   `let is_failure = matches!(&run.status, RunStatus::Failed(_));` puis
   `if is_failure { std::process::exit(1) }`. Sur `FailedNoDelegation` et sur
   `FailedTransport`, code de sortie **0**.
2. **`ask.rs:782-786`** (ticket : `:623–627`) —
   ```rust
   if let Some(ref deliverable) = run.deliverable {
       println!("{deliverable}");
   } else if let RunStatus::Failed(ref msg) = run.status {
       eprintln!("Error: {msg}");
   }
   ```
   La branche `Some(deliverable)` gagne **toujours** sur la branche d'erreur.
   Conséquence non nommée par le ticket : c'est déjà vrai pour `Failed(_)`
   lui-même. Un run qui a produit son livrable (`deliver_phase`, `engine.rs:634`)
   puis a échoué plus loin — le chemin `timeout` de `engine.rs:566` en est un —
   imprime aujourd'hui son livrable partiel sur `stdout` **sans aucune ligne
   d'erreur**. Le code de sortie était juste dans ce cas-là ; le rendu, non.
3. **`chat.rs:1104`** (ticket : `:1044`) — voir M3.

**Ce que le ticket appelle « the notification helper's failure text never fires
because message sender is None in CLI context » demande une rectification.**
`teams::notification::build_run_completion_message` traite bien les cinq statuts
terminaux, `FailedNoDelegation` et `FailedTransport` compris, par un `match`
exhaustif **sans bras `_`**. Il n'est pas « jamais déclenché faute de
sender » : il est `pub(crate)` dans `mika-agent` et donc **structurellement
inatteignable depuis `mika-cli`**, qui est un autre crate. La connaissance
existait ; ce qui manquait était un chemin d'accès. Cette rectification décide
un point de conception (cf. D3) : on n'exporte pas ce helper, on expose la
*classification*.

### Requirements

- **R1** — Une septième variante de `RunStatus` doit faire **échouer la
  compilation** de la décision CLI, et non passer en silence pour un succès.
- **R2** — `mika ask --team` sort avec un code non nul sur **tout** statut
  terminal d'échec : `Failed`, `FailedNoDelegation`, `FailedTransport`.
- **R3** — Sur un statut terminal d'échec, `stdout` est **vide** dans les trois
  formats de rendu du mode texte ; le diagnostic et l'éventuel texte partiel
  vont sur `stderr`.
- **R4** — `mika chat --team` route tout statut terminal d'échec vers
  `TeamEvent::RunFailed`, ce qui interdit structurellement la persistance du
  texte sous le rôle `assistant`.
- **R5** — La raison lisible d'un `FailedNoDelegation` (variante sans champ) a
  **un seul site d'écriture** pour le registre opérateur, au lieu des deux
  copies divergentes actuelles.
- **R6** — Aucun changement de format de fil : `team_run.status` reste la chaîne
  produite par `Display`, `content` garde sa sémantique actuelle. Tout ajout est
  additif et omis quand absent.
- **R7** — Le mode `--format json` et `--format yaml` restent byte-identiques
  sur un run réussi.

### Scope Boundaries

**Dans le périmètre**

- Les trois sites nommés, plus `FailedTransport` à chacun d'eux (M1).
- Le prédicat porté par l'enum, dans `mika-agent/src/teams/types.rs`.
- La déduplication de la raison `FailedNoDelegation` entre `engine.rs` et le
  nouveau site canonique.
- Un champ additif `team_run.failure_reason` sur l'enveloppe JSON/YAML : le
  ticket cite explicitement le scripting comme impact, et obliger un script à
  parser `stderr` pour obtenir la raison serait ne fermer que la moitié utile.

**Hors périmètre, avec la raison**

- **(a) `build_run_completion_message_from_row` et son bras `_ => None`.**
  Ce lecteur-là est sur chaîne (`&str`), pas sur enum : un nouveau statut
  persisté y produit silencieusement « aucune notification ». C'est le même
  genre de trou, sur le chemin serveur/asynchrone, avec un rayon d'impact
  différent (notification Telegram d'un run de tenant) et une réparation
  différente (il n'y a pas d'exhaustivité à obtenir sur un `&str` — il faut un
  `parse` faillible). **Ticket de suivi à ouvrir.**
- **(b) Le rendu TUI d'un run `Suspended`.** Aujourd'hui `chat.rs` envoie
  `Deliverable("")` et le TUI affiche « Team completed with no deliverable. » —
  une affirmation fausse (le run n'a pas complété, il s'est suspendu en attente
  de callbacks). La corriger proprement demande une variante `TeamEvent`
  supplémentaire, dont le fan-out touche les deux callbacks CLI, le TUI et les
  sites d'émission du moteur. Disproportionné pour un p3 dont ce n'est pas le
  sujet. La classification livrée **nomme** cet état (`TeamOutcome::Pending`) et
  `chat.rs` le mappe au comportement actuel avec un commentaire qui pointe le
  suivi ; `ask.rs`, qui n'a pas de machine à états d'interface, reçoit la note
  honnête tout de suite. L'asymétrie est assumée et écrite. **Ticket de suivi.**
- **(c) Le code de sortie `75` (`EXIT_TRANSPORT_FAILURE`).** `remote_ask`
  distingue déjà transport (`75`) et contrat (`1`), et `_arch_ask_with_retry`
  réessaie sur `75`. La tentation est de faire sortir `FailedTransport` en `75`.
  **Refusé** : ce budget de réessai est dimensionné pour un `mika ask` simple,
  pas pour un cycle d'équipe complet (décomposition, exécution parallèle, revue,
  livraison) dont un réessai automatique coûte plusieurs minutes et plusieurs
  appels LLM. Faire entrer la population « run d'équipe » dans un budget conçu
  pour une autre serait un couplage que rien ne demande. Tous les échecs
  terminaux d'équipe sortent en `1`.
- **(d) La valeur `content` de l'enveloppe JSON sur un échec.** Elle reste
  `run.deliverable.clone()`. Le mode JSON portait **déjà** le discriminant
  correct (`team_run.status`, M4) : ce n'était pas la surface défectueuse, et la
  vider serait retirer de l'information à des consommateurs sur un format de fil
  qui fonctionnait. Risque résiduel nommé : un consommateur qui lit `.content`
  sans lire `.team_run.status` obtient une réponse plausible. R2 (code de
  sortie) et le nouveau `failure_reason` lui donnent deux moyens de plus de ne
  pas le faire ; le troisième existait déjà.
- **(e) Le texte utilisateur de `notification.rs`.** Il porte un registre
  différent — une phrase adressée à un utilisateur, avec une remédiation
  (« Try rephrasing the goal or check the team's member roster »). Le registre
  opérateur, lui, est court et va dans une colonne de base et sur `stderr`. Deux
  registres pour un fait, séparés délibérément — même arbitrage que mika#2290 et
  mika#2292. Seul le registre **opérateur** est dédupliqué (R5).

## Planning Contract

### Key Technical Decisions

**D1 — La décision vit sur l'enum, en un `match` exhaustif sans bras `_`.**

```rust
/// Terminal disposition of a run, as a single compile-checked decision.
pub enum RunDisposition<'a> {
    /// Still running, or suspended awaiting callbacks.
    NotTerminal,
    /// Terminal success.
    Success,
    /// Terminal failure, with its operator-register reason.
    Failure { reason: &'a str },
}
```

`RunStatus::disposition(&self) -> RunDisposition<'_>` est **le** `match`
exhaustif. `is_terminal_failure()` et la raison en sont **dérivés**, jamais
écrits séparément — ce qui rend le biconditionnel « c'est un échec ⟺ il y a une
raison » vrai *par construction* plutôt que vrai *par test*. Sans bras `_`, à la
lettre du modèle `webhook_queue_v2::coalescing_key`, de
`dispatch_substrate_diagnostic` et de `hosting_ground_truth_line` : le
compilateur, et pas un relecteur, force la prochaine variante à décider.

**D2 — La CLI ne nomme plus aucune variante de `RunStatus`.**

C'est la forme forte de D1, et c'est ce qui permet de livrer la garde
structurelle avec une **allowlist vide**. `team_outcome::classify` n'a besoin
que de `disposition()` : `NotTerminal` → `Pending`, `Failure { reason }` →
`Failed`, `Success` → `Delivered` ou `CompletedWithoutDeliverable` selon
`run.deliverable`. Zéro motif sur une variante, donc zéro site à exempter, donc
aucun endroit où déposer la prochaine violation. Une allowlist née vide est un
emplacement pour la prochaine entorse (mika#2323 le dit en toutes lettres) ;
ici il n'y en a pas.

**D3 — On expose la classification, jamais le texte de `notification.rs`.**

La tentation naturelle, une fois M4 comprise, est de rendre
`build_run_completion_message` public et de s'en servir depuis la CLI. Refusé
pour deux raisons. *Registre* : son texte est déjà préfixé (« Team 'alpha' did
not run: … ») et le TUI le re-préfixerait (« Team error: Team 'alpha' did
not run: … »). *Périmètre* : rendre public un helper de notification
utilisateur pour en extraire une classification, c'est importer le registre avec
la décision — exactement le couplage que mika#2023 a dû défaire entre l'axe
outils et l'axe persona. La classification monte dans l'enum ; le texte reste
où il est.

**D4 — `stdout` vide sur un échec, et c'est la moitié qui ferme le défaut pour
un script qui ne lit pas le code.**

Le code de sortie ferme le cas `if ! mika ask …` et `set -e`. Il ne ferme pas
`RESULT=$(mika ask --team X "goal")`, qui capture `stdout` et obtient
aujourd'hui le `retry_reply` — une réponse plausible à une question qui a
échoué. `stdout` est le canal de succès d'une CLI ; un run en échec doit l'y
laisser vide. Le texte partiel n'est pas jeté pour autant : il part sur `stderr`,
derrière un marqueur qui dit ce qu'il est. Application en miroir de la doctrine
mika#2270 (« une réponse perdue est une erreur, pas une réponse vide ») : une
réponse *échouée* ne doit pas ressembler à une réponse.

**D5 — La raison de `FailedNoDelegation` a un site d'écriture, pas deux.**

La variante est sans champ, donc chaque lecteur qui veut une phrase doit
l'inventer ; deux l'ont déjà fait, avec deux formulations
(`engine.rs:666` pour la colonne `team_runs.failure_reason`, `notification.rs:59`
pour l'utilisateur), et la CLI serait la troisième. Une constante
`NO_DELEGATION_REASON` à côté de la variante, servie par `disposition()`, est
lue par `engine.rs` et par la CLI. Le texte n'est pas modifié : la chaîne
déplacée est celle d'`engine.rs`, à l'octet près, donc la colonne DB ne change
pas de valeur.

**D6 — Un seul classificateur pour les deux surfaces CLI.**

`ask.rs` et `chat.rs` posent la même question et la posaient séparément — c'est
la duplication qui a permis aux deux de manquer les deux variantes
indépendamment. Un module `commands/team_outcome.rs`, lu par les deux. Même
motif que `grooming_marker` (mika#2158) et `live_pilot` (mika#2279) : *un
résolveur écrit deux fois est un résolveur qui peut se contredire*.

**D7 — La garde structurelle est nécessaire parce qu'aucun test comportemental
ne peut voir la régression.**

Si quelqu'un réécrit demain un `matches!(run.status, RunStatus::Failed(_))` dans
`mika-cli`, aucune assertion existante ne rougit : les décisions couvertes
restent justes, c'est un **nouveau** chemin qui redevient muet. C'est
littéralement la classe que `mika2335_no_production_dispatch_transitions_a_parent_without_stamping`
et `mika2205_periodic_scans_do_not_read_the_pat_field_directly` existent pour
attraper. Le dépôt a déjà l'outil et `mika-cli` l'utilise déjà
(`mika1951_the_envelope_never_reads_the_local_flag`, `ask.rs:1198`) :
`mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"))`,
déjà en `dev-dependencies` de ce crate. Aucune dépendance nouvelle.

### High-Level Technical Design

```
mika-agent/src/teams/types.rs
    NO_DELEGATION_REASON: &str                   ← D5, site unique (registre opérateur)
    enum RunDisposition<'a>                      ← D1
    RunStatus::disposition() -> RunDisposition   ← LE match exhaustif, sans `_`
    RunStatus::is_terminal_failure() -> bool     ← dérivé de disposition()

mika-agent/src/teams/engine.rs:666
    le littéral local `no_delegation_reason` → NO_DELEGATION_REASON   (D5, iso-valeur)

mika-cli/src/commands/team_outcome.rs            ← D6, NOUVEAU, lecteur unique
    enum TeamOutcome { Delivered | CompletedWithoutDeliverable | Failed | Pending }
    classify(&TeamRun) -> TeamOutcome             ← ne nomme aucune variante (D2)
    exit_code(&TeamOutcome) -> i32                ← 1 sur Failed, 0 sinon (scope-out c)

mika-cli/src/commands/ask.rs:778-816              ← consomme classify()
mika-cli/src/commands/chat.rs:1103-1110           ← consomme classify()
```

Table de disposition, qui est le livrable de conception :

| `RunStatus` | `disposition()` | `TeamOutcome` | stdout | stderr | exit |
|---|---|---|---|---|---|
| `Running` | `NotTerminal` | `Pending` | — | note | 0 |
| `Suspended` | `NotTerminal` | `Pending` | — | note | 0 |
| `Completed` + livrable | `Success` | `Delivered` | livrable | — | 0 |
| `Completed` sans livrable | `Success` | `CompletedWithoutDeliverable` | — | note | 0 |
| `Failed(m)` | `Failure{m}` | `Failed` | — | `Error: m` (+ partiel) | 1 |
| `FailedNoDelegation` | `Failure{NO_DELEGATION_REASON}` | `Failed` | — | idem | 1 |
| `FailedTransport(m)` | `Failure{m}` | `Failed` | — | idem | 1 |

Les trois dernières lignes sont les trois qui changent ; `Failed(m)` y change de
rendu (`stdout` cesse de porter un livrable partiel) sans changer de code de
sortie.

### Assumptions

- **A1** — `run_team` ne rend jamais un `TeamRun` en statut `Running` : les
  chemins d'`execute` posent `Completed`, `Suspended`, ou une des trois
  variantes d'échec avant de rendre. La classification le traite malgré tout
  comme `Pending` plutôt que de paniquer ou de le confondre avec un succès —
  *un signal illisible n'est jamais un terme satisfait*. Coût d'un `Running`
  observé : un run silencieux en exit 0, soit exactement le comportement
  d'aujourd'hui, jamais un faux succès imprimé.
- **A2** — `mika-cli` peut appeler des méthodes inhérentes sur `RunStatus` :
  `teams::types` est `pub` et les deux fichiers l'importent déjà
  (`ask.rs:667`, `chat.rs:30`). **Vérifié.**
- **A3** — `mika_common::source_guard` est disponible en `dev-dependencies` de
  `mika-cli` et y est déjà utilisé. **Vérifié** (`Cargo.toml:59-60`,
  `ask.rs:1198`, `remote_ask.rs:1597`).
- **A4** — Aucun consommateur automatisé de `mika ask --team` n'existe dans le
  workspace, donc le changement de rendu de `stdout` sur un échec ne casse aucun
  appelant connu. **Vérifié** par recherche sur `skills/`, `scripts/`,
  `.claude/`. Reste un changement observable pour un opérateur humain : il est
  déclaré dans le corps de PR.
- **A5** — Un test de bout en bout satisfaisant AC4 à la lettre (lancer le
  binaire contre une équipe qui échoue à déléguer) n'est **pas** réalisable de
  façon déterministe : il demanderait une équipe réelle, un agent réel et un
  appel LLM réel dont l'actionnabilité déclenche la garde. Ce que livre le plan
  à la place est décrit et justifié en Verification Contract § AC4 ; c'est le
  même arbitrage que mika#1671, dont la garde `all_delegations_transport_failed`
  est couverte par une table de cas sur le prédicat pur et non par un run.

## Implementation Units

### U1. La disposition terminale monte sur l'enum

`crates/mika-agent/src/teams/types.rs`

1. Ajouter `pub const NO_DELEGATION_REASON: &str = …`, avec la chaîne **exacte**
   de `engine.rs:666-667`, et un doc-comment qui dit pourquoi elle est ici :
   variante sans champ, donc tout lecteur voulant une phrase doit l'inventer,
   donc deux l'ont déjà inventée différemment (D5). Le doc-comment nomme aussi
   la frontière avec `notification.rs` (registre utilisateur, scope-out (e)).
2. Ajouter `pub enum RunDisposition<'a> { NotTerminal, Success, Failure { reason: &'a str } }`.
3. Ajouter `impl RunStatus { pub fn disposition(&self) -> RunDisposition<'_> }`,
   `match` exhaustif **sans bras `_`**, avec le doc-comment qui porte le
   raisonnement M2 : les trois `match` exhaustifs du crate ont suivi les deux
   ajouts de variante, les trois motifs écrits à la main n'en ont suivi aucun,
   et c'est la seule différence entre les deux groupes.
4. Ajouter `pub fn is_terminal_failure(&self) -> bool`, **dérivé** de
   `disposition()` via `matches!`, jamais un second `match` (D1).

`crates/mika-agent/src/teams/engine.rs`

5. Remplacer le littéral local `no_delegation_reason` (ligne 666) par
   `NO_DELEGATION_REASON`. Chaîne identique → valeur de la colonne
   `team_runs.failure_reason` inchangée.

### U2. Le classificateur CLI, lecteur unique des deux surfaces

`crates/mika-cli/src/commands/team_outcome.rs` (nouveau), déclaré dans
`commands/mod.rs`.

1. `pub enum TeamOutcome { Delivered { text: String }, CompletedWithoutDeliverable,
   Failed { diagnostic: String, partial: Option<String> }, Pending { note: String } }`.
   Les quatre variantes sont distinctes bien que deux partagent leur disposition
   (`exit 0`, note sur `stderr`) : ce sont deux faits différents et le
   classificateur doit rester lisible et testable variante par variante.
2. `pub fn classify(run: &TeamRun) -> TeamOutcome` — `match run.status.disposition()`,
   **sans nommer une seule variante de `RunStatus`** (D2). Sur `Failure`, le
   texte partiel est `run.deliverable.clone()`.
3. `pub fn exit_code(outcome: &TeamOutcome) -> i32` — `1` sur `Failed`, `0`
   sinon. Doc-comment portant le refus de `75` (scope-out (c)).

### U3. `mika ask --team` consomme le classificateur

`crates/mika-cli/src/commands/ask.rs`, `run_team_ask`.

1. Remplacer `let is_failure = matches!(…)` par
   `let outcome = team_outcome::classify(&run);`.
2. Mode texte : rendre selon la table de disposition. Sur `Failed`, `stdout`
   reste vide, `eprintln!("Error: {diagnostic}")`, puis le texte partiel sur
   `stderr` derrière un marqueur qui dit ce qu'il est (et non un livrable).
3. Modes JSON/YAML : `content` inchangé (scope-out (d)) ; `TeamRunMeta` gagne
   `failure_reason: Option<String>` avec `#[serde(skip_serializing_if = "Option::is_none")]`,
   alimenté par `run.status.disposition()`. Sur un run réussi, sortie
   byte-identique (R7). Le diagnostic part aussi sur `stderr` dans ces modes —
   `stderr` est déjà le canal de progression de cette commande.
4. Remplacer `if is_failure { std::process::exit(1) }` par
   `let code = team_outcome::exit_code(&outcome); if code != 0 { std::process::exit(code) }`.
   `process::exit` reste au **bord** ; la décision est dans la fonction pure, ce
   qui est ce qui la rend testable (précédent : `remote_ask::exit_code_for`,
   `main.rs:348`).

### U4. `mika chat --team` consomme le classificateur

`crates/mika-cli/src/commands/chat.rs`, worker d'équipe (ligne 1103).

1. Remplacer `if let RunStatus::Failed(reason) = run.status { … } else { … }` par
   un `match` sur `team_outcome::classify(&run)`.
2. `Failed` → `TeamEvent::RunFailed(texte composé du diagnostic et, s'il existe,
   du partiel derrière son marqueur)`. C'est ce routage, et lui seul, qui ferme
   M3 : le TUI ne persiste en base que sur la branche `Deliverable`
   (`tui/app.rs:1258`), donc router un échec vers `RunFailed` rend la
   persistance d'une fausse réponse **structurellement** impossible — et non
   interdite par convention.
3. `Delivered` / `CompletedWithoutDeliverable` → `Deliverable(text)` /
   `Deliverable(String::new())` — comportement actuel conservé.
4. `Pending` → `Deliverable(String::new())`, comportement actuel conservé, avec
   un commentaire nommant le texte faux du TUI et renvoyant au suivi
   (scope-out (b)). L'asymétrie avec `ask.rs` est écrite là.

### U5. Les tests

**Sur l'enum** (`teams/types.rs`, `#[cfg(test)] mod tests`)

- `mika1940_disposition_covers_every_variant` — table sur les **six** variantes,
  disposition attendue, et pour les trois échecs la raison attendue. C'est la
  couverture qu'AC1 demande, étendue à `FailedTransport` (M1).
- `mika1940_failure_and_reason_agree` — contrôle de cohérence : pour les six
  variantes, `is_terminal_failure()` ⟺ `matches!(disposition(), Failure{..})`.
  Rendu presque tautologique par D1 ; il tient si quelqu'un réécrit
  `is_terminal_failure` en second `match`, ce qui est précisément le geste à
  refuser.
- `mika1940_no_delegation_reason_is_the_engine_string` — la constante est bien
  celle qu'`engine.rs` écrivait, donc la colonne DB ne change pas de valeur
  (U1.5 est une déduplication, pas une réécriture).

**Sur le classificateur** (`commands/team_outcome.rs`)

- `mika1940_every_terminal_failure_exits_non_zero` — les trois variantes
  d'échec → `exit_code == 1`. **Contrôle négatif obligatoire** : les trois
  variantes non-échec → `0`. Sans lui, « le classificateur décide » serait
  indiscernable de « le classificateur échoue toujours ».
- `mika1940_a_failed_run_leaves_stdout_empty` — sur `Failed*`, aucune variante
  ne porte de texte destiné à `stdout` ; le partiel est bien dans le champ
  `partial`. C'est AC2 exprimé sur la fonction pure.
- `mika1940_a_failure_carrying_a_deliverable_is_still_a_failure` — le cas de la
  branche gagnante d'`ask.rs:782` : `FailedNoDelegation` **avec**
  `deliverable = Some(retry_reply)`, la forme exacte que pose `engine.rs:1131`.
  C'est la régression fondatrice ; elle doit être rouge avant le correctif.
- `mika1940_a_completed_run_with_a_deliverable_is_unchanged` — contrôle de
  non-régression du chemin nominal.

**Garde structurelle** (`commands/team_outcome.rs`, D7)

- `mika1940_no_hand_written_run_status_predicate_in_the_cli` — balaye la moitié
  production de `crates/mika-cli/src/**` avec `ProductionScanner::for_each` et
  refuse toute occurrence de `RunStatus::Failed`, `RunStatus::FailedNoDelegation`
  ou `RunStatus::FailedTransport`. **Allowlist livrée vide** (D2 : le
  classificateur lui-même ne nomme aucune variante, donc rien n'est à exempter),
  et le message d'échec dit la résolution : *utiliser `disposition()`, jamais
  ajouter une entrée*. Contrôle de bonne foi obligatoire — assertion que la
  tranche production scannée contient bien `fn classify`, sinon un découpage
  cassé rendrait la garde verte pour la mauvaise raison (modèle
  `test_promotion_gate_never_resolves_conflicts`, `auto_pull.rs:4462`).

### U6. Documentation

- `crates/mika-cli/CLAUDE.md` § `mika ask` — un paragraphe : contrat de code de
  sortie de `--team`, `stdout` vide sur échec, le champ additif
  `failure_reason`, et le refus de `75` avec sa raison.
- `crates/mika-agent/CLAUDE.md` § Schema Version / teams — deux lignes sur
  `disposition()` comme décision unique et sur la garde CLI.
- Entrée de solution `docs/solutions/best-practices/` : *un prédicat écrit à la
  main à côté d'un `match` exhaustif ne suit pas les variantes*, avec le tableau
  de recensement M2 comme mesure. Troisième occurrence de la classe après
  mika#2023 M2 (`tier == AgentTier::Family`) et mika#2158 (`is_groomed`
  recopié) — c'est ce qui la rend digne d'une entrée plutôt que d'un commentaire.

## Fire-Disposition

- **Aucun événement de journal nouveau, aucune ligne `audit_events`.** Le défaut
  est un code de sortie et un canal de sortie sur une commande interactive ; il
  n'a pas de population à compter côté serveur. Ajouter une télémétrie ici
  mesurerait la fréquence à laquelle un opérateur lance `mika ask --team`, ce
  que personne ne demande.
- **La surface d'observation est le code de sortie lui-même**, qui est
  précisément ce que le ticket veut rendre lisible.
- **Pas de kill-switch.** Il n'y a pas de population dont on puisse craindre un
  faux positif : la classification est totale sur un enum fermé, et les trois
  cas qui changent sont trois cas qui mentaient.

## Verification Contract

| # | Vérification | Comment |
|---|---|---|
| V1 | `cargo test -p mika-agent teams::types` | les trois tests de U5 passent |
| V2 | `cargo test -p mika-cli team_outcome` | les cinq tests + la garde passent |
| V3 | `cargo test -p mika-cli` | aucune régression sur `ask.rs` / `remote_ask` |
| V4 | `cargo clippy --workspace --all-targets` | zéro warning |
| V5 | `cargo fmt --check` | propre |
| V6 | Contrôle d'exhaustivité, **manuel et obligatoire** | ajouter localement une septième variante à `RunStatus`, vérifier que `cargo check -p mika-agent` échoue sur `disposition()` **et nulle part ailleurs dans `mika-cli`**, puis retirer. C'est la seule preuve directe de R1 ; un test ne peut pas l'exprimer. À consigner dans le corps de PR. |
| V7 | Garde structurelle, contrôle négatif | réintroduire temporairement `matches!(run.status, RunStatus::Failed(_))` dans `ask.rs`, vérifier que `mika1940_no_hand_written_run_status_predicate_in_the_cli` rougit, puis retirer |

**AC4 — ce qui est livré, et ce qui ne l'est pas.** Le libellé demande un test
lançant `mika ask --team <équipe-qui-échoue-à-déléguer>`. Ce n'est pas
réalisable de façon déterministe (A5) : la garde de mika#1676 ne se déclenche que
si `detect_actionability` classe *deux* réponses conversationnelles successives
d'un LLM réel comme actionnables. Ce qui est livré à la place, et qui couvre la
même propriété :

1. la décision de code de sortie est une **fonction pure** testée sur les six
   variantes, contrôle négatif compris (`mika1940_every_terminal_failure_exits_non_zero`) ;
2. la régression fondatrice — `FailedNoDelegation` porteur d'un `deliverable` —
   est épinglée telle que le moteur la produit
   (`mika1940_a_failure_carrying_a_deliverable_is_still_a_failure`) ;
3. le bord (`process::exit`) est réduit à `exit_code(&outcome)`, donc la seule
   part non testée est un appel d'une ligne — le même découpage que
   `remote_ask::exit_code_for`, déjà en service.

Même arbitrage, explicitement, que mika#1671 : son prédicat
`all_delegations_transport_failed` est couvert par une table de cas sur la
fonction pure, jamais par un run d'équipe.

## Definition of Done

- [ ] `RunStatus::disposition()` existe, `match` exhaustif sans bras `_`,
      `is_terminal_failure()` en est dérivé.
- [ ] `NO_DELEGATION_REASON` a un site d'écriture ; `engine.rs` le lit ; la
      valeur écrite en base est inchangée.
- [ ] `commands/team_outcome.rs` existe, est le lecteur unique, et ne nomme
      aucune variante de `RunStatus`.
- [ ] `ask.rs` et `chat.rs` passent tous deux par `classify()`.
- [ ] `mika ask --team` sort non nul sur les trois variantes d'échec.
- [ ] `stdout` est vide sur un échec ; diagnostic et partiel sur `stderr`.
- [ ] `chat.rs` route tout échec vers `RunFailed` ; plus aucun chemin ne peut
      persister un texte d'échec en `assistant`.
- [ ] `team_run.failure_reason` est additif et omis quand absent ; run réussi
      byte-identique.
- [ ] Garde structurelle verte, allowlist vide, contrôle de bonne foi présent.
- [ ] V1–V7 passés, V6 consigné dans le corps de PR.
- [ ] Les trois entrées de documentation de U6 sont écrites.
- [ ] Les deux tickets de suivi (scope-out (a) et (b)) sont ouverts et cités
      dans le corps de PR sous une ligne `Tracked in:`.

## Acceptance criteria

Transcrits verbatim du corps de mika#1940 :

- **AC1** — `is_failure` predicate in `ask.rs:619` returns `true` for
  `FailedNoDelegation` (verify via test)
- **AC2** — text mode CLI output on `FailedNoDelegation` distinct from success
  (either failure-shaped message OR non-zero exit code, ideally both)
- **AC3** — `chat.rs:1044` notification helper handles `FailedNoDelegation`
  symmetrically with other terminal failure statuses
- **AC4** — regression test : `mika ask ... --team <team-that-fails-delegation>`
  exit code non-zero + output distinguishable from success

### Lecture des AC contre le code au 2026-09-20

Les AC datent du 2026-08-21 et citent des numéros de ligne qui ont dérivé. Rien
dans leur substance ne change ; la correspondance est écrite ici pour qu'une
relecture n'ait pas à la redécouvrir.

| AC | référence du ticket | site réel | satisfait par |
|---|---|---|---|
| AC1 | `ask.rs:619` | `ask.rs:778` | U1 + `mika1940_disposition_covers_every_variant`. Le prédicat nommé `is_failure` disparaît au profit de `is_terminal_failure()` sur l'enum ; la propriété demandée (« rend `true` pour `FailedNoDelegation` ») est celle qui est testée. |
| AC2 | `ask.rs:623–627` | `ask.rs:782-786` | U3 — les **deux** moitiés du « ideally both » sont livrées : message d'échec sur `stderr` *et* code non nul *et* `stdout` vide. |
| AC3 | `chat.rs:1044` | `chat.rs:1104` | U4. Note : le site réel n'est pas un « notification helper » mais le worker d'équipe ; le vrai helper de notification (`teams/notification.rs`) traitait déjà les cinq statuts terminaux — cf. Problem Frame. |
| AC4 | — | — | Verification Contract § AC4, avec la limite d'exécutabilité nommée. |

**Extension assumée au-delà des AC :** `RunStatus::FailedTransport` (mika#1671)
est traitée partout où `FailedNoDelegation` l'est. Les AC ne la nomment pas
parce que le ticket ne l'avait pas vue (M1) ; la livrer à moitié signifierait
déclarer le défaut fermé en en laissant la moitié armée.

## Risks

| Risque | Gravité | Mitigation |
|---|---|---|
| Un opérateur ou un script capturait `stdout` sur un run en échec et perd ce texte | faible | Le texte part sur `stderr`, il n'est pas jeté. Aucun consommateur automatisé n'existe dans le workspace (A4). Changement déclaré dans le corps de PR — la doctrine pré-1.0 du dépôt autorise une rupture documentée. |
| `team_run.failure_reason` casse un parseur JSON strict | très faible | Champ additif, omis quand absent (`skip_serializing_if`), run réussi byte-identique (R7). |
| La garde structurelle rougit sur un futur site légitime | faible | Sa résolution est écrite dans son message : passer par `disposition()`. D2 rend l'exemption inutile par construction ; si elle devient nécessaire, c'est que quelqu'un a réintroduit un motif, ce qui est exactement ce qu'elle doit signaler. |
| `RunDisposition<'a>` gêne un appelant par sa durée de vie | faible | Les deux appelants tiennent un `TeamRun` possédé et produisent des `String`. Repli disponible si besoin : `failure_reason() -> Option<String>`, au prix d'une allocation par appel. |
| La déduplication de U1.5 change la valeur écrite en base | faible | Test dédié (`mika1940_no_delegation_reason_is_the_engine_string`) sur l'égalité de chaîne. |
| Le scope-out (b) laisse un texte faux dans le TUI sur `Suspended` | faible | Pré-existant, non aggravé, nommé, avec un ticket de suivi. Le classificateur le distingue déjà (`Pending`), donc le correctif futur est un changement de routage et non une ré-analyse. |

## Sources

- `crates/mika-agent/src/teams/types.rs:132-154, 271-282` — l'enum et son `Display` exhaustif
- `crates/mika-agent/src/teams/engine.rs:664-675` — la colonne DB, `match` exhaustif + le littéral à dédupliquer
- `crates/mika-agent/src/teams/engine.rs:1119-1141` — la garde mika#1676, et `deliverable = Some(retry_reply)`
- `crates/mika-agent/src/teams/engine.rs:1567` — `FailedTransport` (mika#1671)
- `crates/mika-agent/src/teams/notification.rs:26-82` — `match` exhaustif sans `_`, `pub(crate)`
- `crates/mika-cli/src/commands/ask.rs:778-818` — sites 1 et 2
- `crates/mika-cli/src/commands/chat.rs:1103-1110` — site 3
- `crates/mika-cli/src/tui/app.rs:1246-1275` — `save_message(…, "assistant", …)` sur la branche `Deliverable` (M3)
- `crates/mika-cli/src/remote_ask.rs:125-160` — `exit_code_for`, `a2a_error_class` (le motif exhaustif sans `_`, et le contrat `75`)
- `crates/mika-cli/src/commands/ask.rs:1186-1221` — `mika1951_…`, garde `ProductionScanner` déjà en service dans ce crate
- `crates/mika-agent/src/auto_pull.rs:4450-4478` — le contrôle de bonne foi d'une garde de scan
- `crates/mika-agent/src/teams/engine.rs:2373-2445` — mika#1671 : le précédent « tester le prédicat pur, pas le run »
- Racine `CLAUDE.md` § `MIKA_AGENT_TIER` (mika#2023 M2) — la classe « comparaison d'égalité qu'aucune erreur de compilation ne peut annoncer »

## Revision history

- 2026-09-20 — v1, rédaction initiale.
