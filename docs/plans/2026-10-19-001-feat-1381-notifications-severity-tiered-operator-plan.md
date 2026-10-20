---
title: "feat(notifications): notifications opérateur à niveaux de sévérité — le P0 « la boucle est morte » sur une surface jamais coupée"
type: feat
status: active
date: 2026-10-19
ticket: senara-solutions/mika#1381
branch: feat/1381/notifications-severity-tiered-operator
milestone: senara-solutions/mika#30 — Loop Trustworthiness
supersedes: docs/plans/2026-08-22-001-feat-notifications-severity-tiered-operator-plan.md
---

# Notifications opérateur à niveaux de sévérité — le P0 sur une surface jamais coupée

## Overview

Aujourd'hui toute notification opérateur du runtime mika-spirit part sur **un seul
canal** : le chat Telegram du client. `send_notification()` — **13 sites** dans
`crates/mika-agent/src/server/verdict_handler.rs` (aide à `:2699`) et **3 sites**
dans `crates/mika-agent/src/server/ci_failure_handler.rs` (aide à `:1060`) —
prend un `&str` nu et appelle `MessageSender::send()` sans aucun signal de
sévérité. Un dix-septième site, le plus important, appelle `sender.send()`
directement : `crates/mika-agent/src/agent_loop/mod.rs:4260`, la notification de
**calage du dispatch ready-label**.

Sous charge réelle, le volume d'accusés de routine pousse l'opérateur à couper le
chat — et les signaux P0 (« la boucle est morte ») sont coupés avec eux.

Ce plan ajoute un axe `NotificationSeverity` aux notifications opérateur et route
les **événements P0** vers une **surface structurellement séparée** (un second
chat Telegram, configuré par `MIKA_OPERATOR_ESCALATION_CHAT_ID`). Les accusés de
routine (P2) continuent d'aller au chat normal, que l'opérateur peut couper sans
faire taire les alarmes de panne.

Le motif est repris tel quel de la couche cadence de **mika-manager**, ratifiée le
2026-08-21 par Vincent + Prime et câblée dans
`crates/mika-agent/src/milestone_manager/cadence.rs::select_route` :
`Severity::Blocked` → `MIKA_MANAGER_ESCALATION_URL`,
`Severity::Attention`/`Healthy` → `MIKA_MANAGER_DELIVERY_URL`.

## Ce que ce re-groom a changé par rapport au plan d'août

Le plan d'août (`2026-08-22-001-…`, READY première passe, session
`39503637-…`) n'a jamais été implémenté : la porte d'auto-pull a refusé la
promotion le 2026-09-03 (`branch_too_far_behind`, 97 commits de retard), puis le
budget de re-drive s'est épuisé le 2026-10-19 (`redrive_budget_exhausted`). La
branche est **désormais rebasée** sur `origin/main` (`git rev-list --count
--left-right origin/main...HEAD` ⇒ `0 1`), donc le motif du refus du 2026-09-03
n'existe plus.

Le ticket demandait explicitement une revalidation contre `main`. Elle a produit
cinq corrections, dont trois auraient coûté un pilote :

| point | plan d'août | aujourd'hui (2026-10-19) |
|---|---|---|
| aide `verdict_handler::send_notification` | `:2268`, **11** sites | **`:2699`, 13 sites** |
| aide `ci_failure_handler::send_notification` | `:593`, **2** sites | **`:1060`, 3 sites** |
| site de calage dispatch | `agent_loop/mod.rs:2381` | **`agent_loop/mod.rs:4260`** |
| cible de documentation | `crates/mika-agent/CLAUDE.md § Environment Variables` | **`docs/operator/environment-variables.md`** — la section n'existe plus dans le `CLAUDE.md` de la crate, et la racine interdit d'y documenter une variable (cap de taille tenu par `crates/mika-common/tests/claude_md_size.rs`) |
| justification du repli P0 (D4 d'août) | « reflète le repli de mika-manager vers le puits hors-ligne » | **faux** : `select_route` refuse explicitement la route normale pour `Blocked`. La justification honnête est autre — voir D4 ci-dessous |

Deux ajouts de fond, absents d'août :

- une **borne de volume par vol** (mika#2693) que le périmètre d'août dépasse :
  ≈ 420 lignes contre 279. Le plan est donc découpé en **deux vols**, et ce
  découpage est la décision structurante de ce re-groom (voir § *Découpage en
  vols*) ;
- une **contrainte structurelle non vue en août** :
  `qa_head_supersession.rs:664` scanne le littéral `"GatewayMessageSender::new("`
  dans `run_agent_for_message` et assertionne qu'il suit
  `skip_superseded_review_turn(`. Un refactor qui remplacerait `new()` par un
  point d'entrée de builder fait rougir
  `mika2671_b2_la_garde_de_revue_precede_le_tour` — voir D5.

## Process note (lève l'ESCALATE du 2026-06-26)

Le 2026-06-26 mika-arch a rendu `Verdict: ESCALATE` (session `a26ffb5c`) avec un
blocage F2 : *« la cible de la surface jamais coupée est un choix de design —
chat Telegram séparé / drapeau de contournement de mute / pager externe »*.
L'architecte recommandait l'option 1 mais la traitait comme territoire opérateur.

**Ce choix est tranché par précédent, pas par jugement neuf.** Le brief
mika-manager (`docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`,
écrit par Vincent, ratifié par Prime le 2026-08-21, verdicts 2 et 3) a déjà
établi, pour la même classe de signal :

- deux endpoints (normal + escalade) est la forme correcte ;
- `Blocked` part **exclusivement** sur l'endpoint d'escalade — aucun repli vers
  l'endpoint normal, pour qu'un signal critique ne puisse pas se mettre
  silencieusement en file derrière le trafic de routine ;
- endpoint d'escalade non réglé ⇒ puits hors-ligne, rien n'est perdu.

Ce plan applique la même forme à la couche notification opérateur. Le choix de
design est une application de précédent ; ne remonter à Prime que si Vincent veut
explicitement diverger du motif ratifié.

## Problem Frame

Preuve fondatrice, plan de milestone #30
(`mika-platform/docs/plans/2026-06-03-001-project-loop-trustworthiness-sequenced-plan.md`) :

> « **Alarm into the void** — le moteur journalise `operator notification fired`
> pour les 15 calages, et aucun travail de suite n'a atteint l'opérateur. Le canal
> est coupé à destination, ou la destination n'a jamais été la surface réellement
> surveillée. Dans les deux cas le signal est tiré-et-oublié. »

> « Phase 1 — pierre de voûte d'observabilité : notifications à niveaux ⟸
> BRISE-CYCLE. Les phases 2–3 sont invérifiables sans elle — on réparerait le
> dispatch en étant aveugle à la question de savoir si le dispatch marche. »

La mécanique est intacte aujourd'hui : `agent_loop/mod.rs:4243` écrit
`ready_label_dispatch_stall_total`, `:4249` écrit
`ready_label_dispatch_stalled — operator notification fired`, et `:4260` envoie
la notification **sur le même canal que les accusés de routine**.

## Requirements Trace

| id | exigence | source | où elle atterrit |
|---|---|---|---|
| R1 | un axe de sévérité existe sur le chemin de notification | corps du ticket | D1, vol 1 |
| R2 | le P0 part sur une surface structurellement distincte du chat normal | corps du ticket + F2 du 2026-06-26 | D2/D3, vol 1 |
| R3 | la surface P0 est réglable par l'opérateur, sans recompilation | précédent mika-manager | D6, vol 1 |
| R4 | le site de calage du dispatch (le P0 phare du ticket) est classé P0 | corps du ticket, diagnostic #4 | vol 1 |
| R5 | les accusés de routine restent sur le chat normal, inchangés | corps du ticket | D7 (`send()` délègue en P2), vol 1 |
| R6 | la surface P0 reste **à faible volume**, sinon elle est coupée à son tour | déduit — c'est la seule façon dont R2 reste vrai dans six mois | règle de classification, § *Classification des 17 sites* |
| R7 | le P0 est vérifié comme **livré**, pas seulement comme tiré | corps du ticket : *« vérifier que le P0 atterrit sur un canal que l'opérateur regarde »* | AC6 + sonde post-déploiement |
| R8 | la variable est documentée là où l'opérateur la cherche | CLAUDE.md racine | vol 1, `docs/operator/environment-variables.md` |

R6 n'est pas dans le corps du ticket. Elle est posée ici parce que sans elle R2
est vrai au déploiement et faux au premier mois : une surface « jamais coupée »
qui reçoit dix-sept classes de messages est une surface qui sera coupée. Elle est
la règle qui tranche la classification ci-dessous.

## Classification des 17 sites

**Règle, et elle est la conséquence de R6 :** est P0 ce qui dit *« la machine est
cassée ou calée, un humain doit intervenir »*. Est P2 tout le reste, y compris les
remises de main nominales où la machine a fait son travail.

### P0 — 7 sites

| site | signal |
|---|---|
| `verdict_handler.rs:1373` | `block[ac]` — `escalated_loop_limit` : les tentatives de correction AC sont épuisées |
| `verdict_handler.rs:1660` | `block[ci]` — `escalated_loop_limit` : les tentatives de correction CI sont épuisées |
| `verdict_handler.rs:1868` | `block[{reason}]` escaladé — générique, couvre `security` et `pipeline` |
| `verdict_handler.rs:2346` | coupe-circuit de PR déclenché |
| `verdict_handler.rs:2634` | coupe-circuit de diff identique déclenché |
| `ci_failure_handler.rs:451` | tentatives de correction épuisées — *« Escalating for manual intervention »* |
| `agent_loop/mod.rs:4260` | **calage du dispatch ready-label** — le P0 phare du ticket |

### P2 — 10 sites

| site | signal | pourquoi pas P0 |
|---|---|---|
| `verdict_handler.rs:534` | pass, mais porte humaine DECISION-CORE | la machine a fait son travail et attend une décision ; remise de main nominale |
| `verdict_handler.rs:915` | pass / merge initié | succès |
| `verdict_handler.rs:1442` | `block[ac]`, correction dispatchée | la boucle tourne ; sa forme terminale est `:1373`, déjà P0 |
| `verdict_handler.rs:1711` | `block[ci]`, correction dispatchée | idem, forme terminale `:1660` |
| `verdict_handler.rs:1935` | `hold[review]` sans tâche active (mika#2667) | état d'attente, pas de panne |
| `verdict_handler.rs:2037` | `hold[review]` normal | idem |
| `verdict_handler.rs:2162` | VERDICT illisible | la passe webhook suivante ré-essaie ; si les ré-essais ne convergent pas, `:2634` (diff identique) ou `:2346` (coupe-circuit) tirent en P0 — la forme terminale est déjà couverte |
| `verdict_handler.rs:2193` | VERDICT illisible, pas de tâche | idem |
| `ci_failure_handler.rs:542` | correction structurelle dispatchée, tentative N/MAX | la boucle tourne ; forme terminale `:451`, déjà P0 |
| `ci_failure_handler.rs:720` | `surface_for_adoption` | signal d'observabilité, déjà derrière son propre interrupteur (`settings.surface_for_adoption_notifications_armed()`) |

Les deux sites « VERDICT illisible » sont le seul arbitrage discutable de ce
tableau, et il est posé plutôt qu'implicite : un verdict illisible *est* une
faute machine, mais c'est une faute **ré-essayée**, et sa forme non convergente a
déjà deux sites P0. Les y mettre ferait entrer sur la surface jamais coupée une
population dont le volume suit la dérive de modèle — exactement ce que R6
interdit.

## Scope Boundaries

**Dans le périmètre (vol 1) :** l'axe de sévérité, le rail de routage dans
`messaging.rs`, le réglage `MIKA_OPERATOR_ESCALATION_CHAT_ID`, les deux sites
d'armement, les 7 sites P0 basculés, les tests unitaires du sélecteur, la doc
opérateur.

**Hors périmètre, délibérément :**

- **Le second axe du ticket — relâchement d'INTENT_GUARDS pour supprimer les
  accusés P2.** Le corps demande *« batch/suppress P2 acks via INTENT_GUARD
  relaxation »*. C'est un axe orthogonal (il touche la garde EndTurn de
  `agent_loop`, pas le routage), il n'est pas nécessaire pour que R2 tienne, et
  son porteur a changé depuis août (`agent.rs:1591` → `agent_loop/mod.rs`). Il
  reste à mika#1381 après le vol 2.
- **Les classes `recovery-runbook` et `fabrication-detected`** nommées dans le
  corps : elles n'ont **aucun site de notification existant**. Les créer est une
  extension de périmètre (un nouveau site d'émission, pas une classification).
- **`crates/mika-gateway/.../github.rs`** : le corps le cite comme locus de
  « format ». Il traite les webhooks **entrants**, pas les notifications
  sortantes. Le plan ne touche pas la gateway.
- **Un recensement structurel des 17 sites.** Le compilateur est le détecteur :
  l'aide gagne un paramètre de sévérité au vol 2, donc un site non classé ne
  compile pas. Et une sévérité *fausse* est un jugement qu'aucun scan ne tranche.
- **Le repli du P0 vers un puits hors-ligne.** mika-manager en a un ; ce chemin
  n'en a pas, et en construire un est un sous-système (persistance + rejeu), pas
  une ligne. Voir D4.

**`## Fire-Disposition` : gate N/A, section non écrite** (mika#2306). Ce plan ne
livre aucun détecteur : les tests du vol 1 sont des tests fonctionnels de routage
dont le chemin de succès est *« le P0 est parti sur le chat d'escalade »*, pas
*« aucune violation trouvée »*. Le recensement des 17 sites, qui aurait été un
détecteur, est explicitement refusé ci-dessus. Il n'y a donc pas de population de
violations préexistantes à disposer, et la section n'est pas inventée.

## Design decisions

### D1 — `NotificationSeverity` à **deux** niveaux, pas trois

mika-manager porte trois niveaux (`Healthy`/`Attention`/`Blocked`) parce qu'il
produit des *rapports de cadence* dont le ton varie. Ici il n'y a qu'une
question : *est-ce que cette ligne doit traverser un mute ?* Deux variantes,
`P0` et `P2`, qui sont le vocabulaire du corps du ticket et du plan de milestone
#30. Un `P1` sans destination distincte serait un troisième nom pour le chat
normal.

### D2 — l'axe vit sur le trait, avec une **méthode par défaut**

```rust
#[async_trait]
pub trait MessageSender: Send + Sync {
    async fn send(&self, text: &str) -> Result<SendOutcome>;

    /// Défaut : la sévérité est ignorée. Un rail sans second canal
    /// (`NoopSender`, les doubles de test, le rail CLI) garde exactement son
    /// comportement.
    async fn send_with_severity(
        &self,
        text: &str,
        _severity: NotificationSeverity,
    ) -> Result<SendOutcome> {
        self.send(text).await
    }
}
```

Le défaut est ce qui rend le changement non invasif : les quatre
`impl MessageSender` de production et les cinq doubles de test
(`ready_label_handler.rs:3081`, `ci_failure_handler.rs:1449`,
`dispatcher.rs:4959` et `:9178`, `engine.rs:5831`, `send_message.rs:303`) ne sont
pas touchés. Seul `GatewayMessageSender` surcharge.

### D3 — `GatewayMessageSender` surcharge, et `send()` délègue en P2

```rust
impl MessageSender for GatewayMessageSender {
    async fn send(&self, text: &str) -> Result<SendOutcome> {
        self.send_with_severity(text, NotificationSeverity::P2).await
    }

    async fn send_with_severity(&self, text: &str, severity: NotificationSeverity)
        -> Result<SendOutcome> { /* … */ }
}
```

Ce sens de délégation (et non l'inverse) est ce qui garantit **zéro duplication
du chemin d'envoi** : il n'existe qu'un `build_payload` → `try_send` → retry →
`save_failed_send`, donc une correction future du transport ne peut pas rater la
moitié du trafic. Le corps existant de `send()` (`messaging.rs:175`) se déplace
dans `send_with_severity` ; la seule ligne qui change à l'intérieur est la
résolution de la cible.

### D4 — un P0 sans chat d'escalade réglé **retombe sur le chat normal**, et dit pourquoi

C'est l'endroit où ce plan **diverge** de mika-manager, et la justification
d'août était fausse. `cadence.rs::select_route` refuse explicitement la route
normale pour `Blocked` :

```rust
// H1 review fix: Blocked severity uses ONLY the escalation URL — no
// fallback to `delivery_url`. … When the escalation URL is unset,
// fall through to the offline sink …
```

Ce refus est correct **là-bas** parce que `delivery_url` est un relais
Prime→sami→Vincent, c'est-à-dire **quelqu'un d'autre** : y router un `Blocked`
l'enfouirait dans le trafic d'un tiers. Ici le chat normal est **le même humain**,
simplement sur un canal qu'il peut couper. Et il n'existe pas de puits hors-ligne
sur ce chemin : refuser la route normale signifierait **jeter** le P0.

Donc : chat d'escalade réglé ⇒ le P0 y part, exclusivement. Non réglé ⇒ le P0
part sur le chat normal **avec un WARN nommé**
`operator_escalation_chat_unset`, parce qu'un repli silencieux est exactement le
« alarm into the void » que ce ticket existe pour fermer. L'ancre
`blocked_severity_with_no_escalation_url_falls_to_offline_sink_not_delivery_url`
(`cadence.rs:1160`) reste verte : ce plan ne touche pas mika-manager.

### D5 — un **builder**, et `new()` ne bouge pas

`GatewayMessageSender::new()` a 8 arguments, un `#[allow(clippy::too_many_arguments)]`
et **16 sites d'appel** dont 10 dans des tests. Un neuvième argument les touche
tous les 16 pour n'en concerner que 2.

```rust
impl GatewayMessageSender {
    /// Arme le second canal. Chaînable derrière `new()` pour que l'arité de
    /// `new()` — et ses 16 sites — ne bougent pas.
    pub fn with_escalation_chat_id(mut self, chat_id: Option<i64>) -> Self { … }
}
```

**Contrainte structurelle, non vue en août :**
`qa_head_supersession.rs:664` cherche le littéral `"GatewayMessageSender::new("`
dans le corps de `run_agent_for_message` (`server/handlers.rs:1657`) et
assertionne qu'il **suit** `skip_superseded_review_turn(`. Un builder *chaîné*
derrière `new()` laisse ce littéral en place et le test vert. Un point d'entrée
de type `GatewayMessageSender::builder()` le ferait rougir — et le message
d'erreur (« `GatewayMessageSender::new(` absent de `run_agent_for_message` »)
n'expliquerait pas pourquoi. C'est une raison de plus de prendre le builder
chaîné plutôt que de réécrire le constructeur.

### D6 — la valeur passe par `Settings`, pas par une lecture d'env directe

mika-manager lit son endpoint d'escalade directement dans l'environnement
(`spawn.rs:205–271`, `read_string_env(ENV_ESCALATION_URL)`), parce que
`ManagerConfig` n'est pas construit depuis `Settings`. Ici les deux sites
d'armement ont `Settings` **en portée** (`handlers.rs:1731` lit déjà
`a.settings.customer_id`, `server/mod.rs:658` a `agent_settings`), et
`config.rs` est l'endroit où un opérateur cherche une variable `MIKA_*`.

```rust
/// Chat d'escalade pour les notifications P0 (mika#1381).
///
/// Override d'env : `MIKA_OPERATOR_ESCALATION_CHAT_ID`.
#[serde(default)]
pub operator_escalation_chat_id: Option<i64>,
```

`Environment::with_prefix("MIKA")` (`config.rs:2563–2574`) peuple le champ sans
câblage supplémentaire ; neuf champs `Option<i64>/<u64>/<u32>` existants donnent
le précédent exact (`:888`, `:979`, `:1115`, `:1151`, …). Trois sites à mettre à
jour avec le champ : la liste du `Debug` redacteur (`:2839` — valeur **en clair**,
un identifiant de chat n'est pas un secret) et `test_defaults()` (`:2654`).

`Option<i64>` et non `i64` : l'absence est l'état nominal (un déploiement sans
second chat), et `0` est déjà le sentinelle documentée « pas de canal de réponse »
— voir D8.

### D7 — l'aide `send_notification` gagne son paramètre au **vol 2**, pas au vol 1

Changer la signature de l'aide est un changement qui touche ses 16 sites d'un
coup (le compilateur l'exige). Pour que le vol 1 soit sous la borne **et** livre
un P0 vivant, le vol 1 ajoute une **seconde** aide, `send_notification_p0`, dans
chacun des deux handlers, et ne bascule que les 6 sites P0 qui passent par une
aide (le septième, `agent_loop:4260`, appelle `sender` directement). Le vol 2
fusionne les deux aides en une seule portant un paramètre `severity` explicite,
et c'est ce qui rend le compilateur détecteur sur les 16 sites.

Coût nommé : entre le vol 1 et le vol 2, deux aides coexistent dans chaque
handler. C'est la forme transitoire, et elle est bornée par le vol 2.

### D8 — `escalation_chat_id == Some(0)` est refusé comme route

`chat_id == 0` est la sentinelle documentée « pas de canal de réponse » :
`send()` court-circuite dessus et rend `SendOutcome::NoChannel` avant tout POST
(`messaging.rs:175`, mika#650/#1090). Un chat d'escalade réglé à `0` produirait
donc un P0 **silencieusement avalé** — le mode de panne précis que ce ticket
ferme. Le sélecteur traite `Some(0)` comme non réglé : repli sur le chat normal,
même WARN que D4. C'est le cas de test qui ne doit pas manquer (AC5).

### D9 — observabilité : un champ, pas un événement

Le chemin d'envoi journalise déjà (`send_message_nochannel`, les classes
d'erreur, `save_failed_send`). Ce plan n'ajoute **pas** d'événement de
notification : il ajoute un champ `severity` sur les lignes existantes du chemin
et **un seul** événement neuf, le WARN de repli `operator_escalation_chat_unset`
de D4 — qui est le seul fait que les lignes existantes ne peuvent pas dire.

## Découpage en vols

Le périmètre complet pèse ≈ 420 lignes de code hors `docs/`, au-dessus de la
borne par vol de 279 lignes. Il est donc découpé en deux vols, **chacun
fusionnable seul**.

**Vol 1 — ce plan, cette PR.** Le rail, son réglage, et les 7 sites P0. À la
fusion du vol 1 : le P0 route réellement, les 10 sites P2 gardent exactement leur
comportement d'aujourd'hui (ils passent par `send_notification`, qui appelle
`send()`, qui délègue en P2). R1–R6 et R8 sont satisfaits. La PR ne ferme pas
mika#1381.

**Vol 2 — la suite, portée par mika#1381 qui reste ouvert.** Fusion des deux
aides en une seule à paramètre `severity` explicite (le compilateur devient
détecteur sur les 16 sites), classification explicite des 10 sites P2,
`RecordingSender` étendu pour enregistrer la sévérité, et attestation
bout-en-bout handler→sender. ≈ 150 lignes, plus ≈ 90 si l'attestation du site de
calage `agent_loop` passe par le harnais eval.

Tracked in: senara-solutions/mika#1381 — vol 2 (fusion des aides, classification
P2 explicite, attestation bout-en-bout). Le ticket reste ouvert après la fusion
du vol 1.

## Implementation plan (vol 1)

Ordre de commit, chaque étape compilant et testant seule.

### Phase A — le rail (`crates/mika-agent/src/messaging.rs`)

1. `pub enum NotificationSeverity { P0, P2 }` — `Copy`, `Debug`, `PartialEq`, et
   un `as_str()` pour les champs de log.
2. Méthode par défaut `MessageSender::send_with_severity` (D2).
3. Champ `escalation_chat_id: Option<i64>` sur `GatewayMessageSender` +
   `with_escalation_chat_id` chaînable (D5).
4. Sélecteur `fn resolve_notification_target(&self, severity, normal_chat_id)
   -> (i64, TargetKind)` — pur hors de la résolution de `chat_id` déjà faite,
   donc testable **sans serveur HTTP**, exactement comme `resolve_chat_id` et
   `build_payload` le sont déjà (`messaging.rs:97`, `:116`). `TargetKind`
   distingue `Normal`, `Escalation` et `EscalationUnsetFallback` — c'est le
   discriminant que les tests assertionnent et que le WARN de D4 lit.
5. Le corps de `send()` se déplace dans `send_with_severity` ; `send()` délègue
   en P2 (D3).

### Phase B — le réglage (`crates/mika-common/src/config.rs`)

6. Champ `operator_escalation_chat_id: Option<i64>` + doc-comment (D6).
7. Entrée dans la liste du `Debug` redacteur (`:2839`), en clair.
8. Entrée dans `test_defaults()` (`:2654`), `None`.

### Phase C — l'armement

9. `server/handlers.rs:1731` — chaîner
   `.with_escalation_chat_id(a.settings.operator_escalation_chat_id)` derrière
   `new()`. **Le littéral `GatewayMessageSender::new(` reste en place** (D5).
10. `server/mod.rs:658` — idem avec `agent_settings`.

Le troisième `GatewayMessageSender::new` de `handlers.rs` (`:2333`) est le
chemin `flush_failed_sends` : il rejoue des envois déjà classés, il n'émet pas de
notification, il n'est pas armé.

### Phase D — les 7 sites P0

11. `verdict_handler.rs` — aide `send_notification_p0` (D7), puis `:1373`,
    `:1660`, `:1868`, `:2346`, `:2634`.
12. `ci_failure_handler.rs` — aide `send_notification_p0`, puis `:451`.
13. `agent_loop/mod.rs:4260` — `sender.send_with_severity(&notification,
    NotificationSeverity::P0)`. Une ligne, et c'est le P0 phare du ticket.

### Phase E — les tests et la doc

14. Cinq cas dans `messaging.rs mod tests`, sur le sélecteur et le défaut de
    trait (AC2–AC5, AC7).
15. `docs/operator/environment-variables.md` — section
    `### Le P0 passe par un second chat, et un chat d'escalade absent le dit (mika#1381)` :
    la variable, son défaut, la surface opérateur, la sonde post-déploiement
    (R7/AC6) et ce que ce travail n'achète pas. Hors du compte de lignes.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/messaging.rs` — enum, défaut de trait, champ + builder, sélecteur, override, délégation de `send()` | 95 |
| `crates/mika-common/src/config.rs` — champ, entrée `Debug`, `test_defaults` | 30 |
| sites d'armement — `server/handlers.rs:1731`, `server/mod.rs:658` | 10 |
| aides `send_notification_p0` (deux handlers) + 7 sites P0 basculés | 27 |
| tests (`crates/mika-agent/src/messaging.rs mod tests`, 5 cas) | 110 |

Total estimé : 272 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient ici.** 272 ≤ 279, avec sept lignes de marge. La
marge est mince et c'est délibéré : le périmètre complet pesait ≈ 420 lignes, et
le découpage du § *Découpage en vols* est ce qui ramène le vol courant sous la
borne. Les trois leviers qui ont fait la différence, chacun vérifiable dans le
plan plutôt que postulé :

- le **builder chaîné** (D5) laisse 14 des 16 sites de `new()` intacts, dont les
  10 sites de test ;
- la **délégation `send()` → `send_with_severity`** (D3) déplace le corps d'envoi
  au lieu de le dupliquer — net ≈ 0 ligne pour le chemin HTTP ;
- le **sélecteur pur** (phase A.4) rend le routage testable sans serveur HTTP, du
  même coup que `resolve_chat_id` et `build_payload` le sont déjà : cinq cas à
  ≈ 22 lignes au lieu de cinq fixtures de transport à ≈ 60.

Si l'implémentation dépasse pendant le vol, le livrable à décrocher est le
**paramètre `severity` sur les lignes de log existantes** (D9) — l'événement de
repli `operator_escalation_chat_unset` reste, lui, obligatoire, car AC4 en
dépend.

## Definition of Done

- [ ] `NotificationSeverity { P0, P2 }` existe et est exporté depuis `messaging.rs`.
- [ ] `MessageSender::send_with_severity` existe avec une méthode par défaut ; aucun des neuf autres `impl MessageSender` du dépôt n'est modifié.
- [ ] `GatewayMessageSender` porte `escalation_chat_id` et `with_escalation_chat_id` ; l'arité de `new()` est inchangée et ses 16 sites ne sont pas touchés.
- [ ] `MIKA_OPERATOR_ESCALATION_CHAT_ID` est lu par `Settings` et atteint les deux sites d'armement.
- [ ] Les 7 sites P0 envoient en P0 ; les 10 sites P2 ont un comportement **inchangé par rapport à `main`**.
- [ ] Un P0 sans chat d'escalade réglé part sur le chat normal et émet `operator_escalation_chat_unset`.
- [ ] `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check` verts.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] `mika2671_b2_la_garde_de_revue_precede_le_tour` vert (D5).
- [ ] `blocked_severity_routes_to_escalation_url` et `blocked_severity_with_no_escalation_url_falls_to_offline_sink_not_delivery_url` verts — ce plan ne touche pas mika-manager.
- [ ] `docs/operator/environment-variables.md` porte la variable, la sonde et ses haltes.
- [ ] La PR dit `Part of #1381`, **pas** `Closes #1381`, et porte la ligne `Tracked in:` du vol 2.

## Acceptance criteria

- **AC1** — `NotificationSeverity` a exactement deux variantes, `P0` et `P2`. Un troisième niveau sans destination distincte est refusé (D1).
- **AC2** — Test `mika1381_un_p0_part_sur_le_chat_descalade` : `escalation_chat_id = Some(999)`, `chat_id = Some(111)`, sévérité `P0` ⇒ la cible est `999` et le discriminant est `Escalation`.
- **AC3** — Test `mika1381_un_p2_ignore_le_chat_descalade` : mêmes réglages, sévérité `P2` ⇒ la cible est `111`, discriminant `Normal`. **C'est le contrôle négatif d'AC2** : sans lui, un override inconditionnel passerait AC2.
- **AC4** — Test `mika1381_un_p0_sans_chat_descalade_retombe_sur_le_chat_normal` : `escalation_chat_id = None`, sévérité `P0` ⇒ la cible est `111` et le discriminant est `EscalationUnsetFallback`, celui que lit le WARN `operator_escalation_chat_unset` (D4).
- **AC5** — Test `mika1381_un_chat_descalade_a_zero_est_refuse_comme_route` : `escalation_chat_id = Some(0)`, sévérité `P0` ⇒ `EscalationUnsetFallback`, **jamais** `SendOutcome::NoChannel`. Un P0 ne peut pas être avalé par la sentinelle `chat_id == 0` (D8).
- **AC6** — Test `mika1381_le_defaut_du_trait_ignore_la_severite` : `NoopSender::send_with_severity(_, P0)` rend `Ok(Delivered)` — un rail sans second canal garde son comportement (D2).
- **AC7** — `grep -c 'GatewayMessageSender::new(' crates/mika-agent/src/server/handlers.rs` rend `2` après le changement comme avant, et `mika2671_b2_la_garde_de_revue_precede_le_tour` est vert (D5).
- **AC8** — Les 7 sites P0 du tableau § *Classification des 17 sites* passent une sévérité `P0` explicite ; les 10 sites P2 sont **inchangés textuellement** par rapport à `origin/main` (vérifiable au `git diff`). Le vol 1 ne touche pas un site P2.
- **AC9** — `agent_loop/mod.rs:4260`, le P0 phare, appelle `send_with_severity(_, NotificationSeverity::P0)`. **Son attestation comportementale est renvoyée au vol 2** : le chemin de calage n'a aucun test aujourd'hui (`grep -rn ready_label_dispatch_stall crates/mika-agent/tests/` ⇒ vide) et en construire un est un scénario de harnais eval. C'est une lacune préexistante que le vol 1 n'aggrave pas et ne ferme pas.
- **AC10** — `docs/operator/environment-variables.md` porte la variable, son défaut (`None` = un seul canal), le WARN `operator_escalation_chat_unset`, et la sonde R7 ci-dessous.

## Verification plan

**Avant la PR :** `cargo test -p mika-agent --lib messaging`, `cargo test -p mika-common --lib config`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main`.

**Sonde post-déploiement (R7 — le P0 est-il *livré*, pas seulement *tiré* ?).** L'instrument n'est vivant qu'après `make deploy`. Régler `MIKA_OPERATOR_ESCALATION_CHAT_ID` sur le second chat, puis :

1. Le chat d'escalade reçoit un message de test envoyé en P0.
2. `grep operator_escalation_chat_unset` dans le log **ne rend rien** — le réglage a pris. S'il rend quelque chose, la variable n'a pas traversé la cascade `MIKA_`, et c'est le chemin D6 qui est en cause, pas le routage.
3. Couper le chat normal. Un P0 doit encore arriver. **C'est la seule halte qui atteste R2** : les deux précédentes sont vraies même si les deux chats sont le même.
4. Dérégler la variable. Un P0 arrive sur le chat normal **et** `operator_escalation_chat_unset` apparaît. Un repli muet est le mode de panne que ce ticket ferme ; sa détection est cette halte.

**Absence de ligne ≠ absence de repli** : une absence peut signifier un binaire antérieur au changement (classe mika#2340). La halte 2 ne vaut qu'après confirmation du sha par `cat ~/.mika/skills/.manifest-writer`.

## Rollback

Le vol 1 est réversible en une variable : laisser `MIKA_OPERATOR_ESCALATION_CHAT_ID`
non réglée ramène le comportement bit pour bit à celui de `main` — tout part sur
le chat normal, au prix d'un WARN par P0. Un revert de code est un
`git revert` du merge de la PR ; aucune migration de schéma, aucun état
persistant, rien à nettoyer.

## Related

- Plan superseded : `docs/plans/2026-08-22-001-feat-notifications-severity-tiered-operator-plan.md` (READY première passe, session `39503637-…`, jamais implémenté — voir § *Ce que ce re-groom a changé*).
- Précédent ratifié : `crates/mika-agent/src/milestone_manager/cadence.rs::select_route`, brief `docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`.
- Milestone : senara-solutions/mika#30 — Loop Trustworthiness, phase 1b (pierre de voûte).
- Projet : `mika-platform/docs/plans/2026-06-03-001-project-loop-trustworthiness-sequenced-plan.md`.
- Contrainte structurelle : `crates/mika-agent/src/qa_head_supersession.rs:639` (`mika2671_b2_la_garde_de_revue_precede_le_tour`).
- Sentinelle `chat_id == 0` : mika#650, mika#1090, `crates/mika-agent/CLAUDE.md § MessageSender Trait`.
