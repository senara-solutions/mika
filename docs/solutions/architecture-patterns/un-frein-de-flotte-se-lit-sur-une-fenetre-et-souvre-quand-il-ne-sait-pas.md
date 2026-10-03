---
title: "Un frein de flotte se lit sur une fenêtre et s'ouvre quand il ne sait pas — trois états, un 0 qui ne désarme pas, un budget qu'il ne dépense pas"
date: 2026-10-03
category: architecture-patterns
module: crates/mika-agent/src/pilot_launcher_health.rs
problem_type: architecture_pattern
component: dev-loop
severity: high
ticket: mika#2634
applies_when:
  - "Ajouter un frein qui suspend TOUS les dispatches sur une panne d'hôte (lanceur, relais, jeton, disque)"
  - "Choisir entre un compteur de consécutifs persistant et un compte sur une fenêtre glissante"
  - "Décider si une garde est fail-open ou fail-closed quand sa lecture échoue"
  - "Exposer un seuil ou une fenêtre en variable d'environnement avec un kill-switch à côté"
  - "Compter des lignes audit_events dont le target_key est unique par dispatch"
  - "Brancher un refus de dispatch dans une boucle qui re-drive les tickets avec un budget"
related_components:
  - crates/mika-agent/src/skills/executor.rs
  - crates/mika-agent/src/auto_pull.rs
  - crates/mika-agent/src/db.rs
tags: [loop-substrate, brake, fail-open, rolling-window, kill-switch, audit-events, re-drive-budget, three-state-verdict, mika2634, claude-pilot]
related:
  - mika#2634
  - mika#2279
  - mika#2597
  - mika#2158
  - mika#2205
  - mika#2199
---

# Un frein de flotte se lit sur une fenêtre et s'ouvre quand il ne sait pas

## Context

mika#2634 : le 2026-10-02, le point d'entrée `~/.local/bin/claude-pilot` a été
réécrit avec le shebang du Python système. Trois dispatches sont morts au
lancement en 2 h 45, sans alarme. La phase A (PR #2644) a rendu cette mort
**visible** : exit 79 estampillé par le pré-flight, une ligne `audit_events`
`pilot_launcher_health = dead`. Sa leçon (le diagnostic émis avant l'armement du
trap EXIT) est dans
`docs/solutions/best-practices/un-diagnostic-emis-avant-le-trap-exit-natteint-personne.md`
et n'est pas reprise ici.

La phase B (PR #2656, AC2) devait **borner** la panne : à la deuxième mort, le
moteur cesse de dispatcher des pilotes. Énoncé ainsi, cela tient en un booléen et
un compteur. Les quatre choix ci-dessous montrent pourquoi ni l'un ni l'autre ne
suffit. Aucun ne se devine en lisant seulement le code final, parce que chacun
écarte une forme plus simple qui avait l'air correcte.

## Guidance

### 1. Un compte sur une fenêtre, jamais un compteur persistant

« ≥ `THRESHOLD` lignes `dead` dans les `WINDOW` dernières secondes » plutôt
qu'un compteur de morts consécutives.

- **Le frein se lève seul.** Un hôte réparé sort de la fenêtre sans qu'on ait
  rien à effacer. Un compteur persistant demande un site de remise à zéro, et
  mika#2158 a mesuré le prix d'un compteur remis à zéro par l'action qu'il
  compte : 31 re-drives qui affichaient 1.
- **Le pire cas est borné.** Si personne ne répare l'hôte, le frein brûle **un**
  dispatch par fenêtre, pas tous.
- **La fenêtre se calibre sur l'incident, des deux côtés.** Les morts sont
  tombées à 03:03Z, 03:05Z et 04:00Z : avec 30 minutes, la troisième repartait à
  neuf. Au-delà d'une heure, la levée cesse d'être un délai qu'un opérateur
  accepte d'attendre une fois l'hôte réparé
  (`pilot_launcher_health.rs`, `BRAKE_WINDOW_DEFAULT_SECS = 3600`).

### 2. La population est l'hôte, pas la tâche : compter par valeur

La requête sœur existante, `count_recent_audit_events_for_target`
(`db.rs:1872`), compte **un sujet** sur une fenêtre. C'est la bonne forme pour
une dédup ou un circuit-breaker par PR. Les lignes `pilot_launcher_health` sont
clées `task:<id>`, une par dispatch et toutes distinctes. Scopée par
`target_key`, la requête rend donc **1 quelle que soit la casse**, et le frein
ne mord jamais. Il fallait une requête qui compte **un résultat sur tous les
sujets** : `count_recent_audit_events_by_value` (`db.rs:1905`, filtre sur
`after_value`, servie par `idx_audit_agent_created`, sans migration).

Avant de réutiliser un compteur d'audit, demandez-vous sur quoi porte la
question : un ticket, une PR, ou l'hôte. Si c'est l'hôte, un `target_key` par
tâche fait une population d'un seul élément.

### 3. Trois états, et le troisième s'ouvre (fail-open)

```rust
pub enum LauncherHealth {
    Healthy,
    Braked { dead_count: i64, since: String },
    Unreadable { reason: &'static str },
}
impl LauncherHealth {
    pub fn is_braked(&self) -> bool { matches!(self, Self::Braked { .. }) }
}
```

(`pilot_launcher_health.rs:192`, `:219`.) `Braked` et `Unreadable` appellent
des conduites opposées : « réparez l'hôte » pour l'un, « la base ne répond pas,
le frein est inerte » pour l'autre. Un booléen ferait lire le second comme le
premier.

Le sens du fail-open se **raisonne**, il ne se copie pas d'une garde voisine.
`wip_rescue` (mika#2199) et `run_gh pr ready` (mika#2624) sont fail-**closed**,
parce que là-bas un faux négatif fait attendre **une** PR. Ici, un faux positif
gèle **toute** la flotte, alors qu'un faux négatif coûte un dispatch brûlé,
visible (une ligne `dead`) et rattrapé au tour suivant. La règle : comparez le
rayon d'un faux positif à celui d'un faux négatif, et ouvrez du côté où l'erreur
coûte le moins.

`is_braked()` existe pour qu'aucun appelant n'écrive `!matches!(v, Healthy)`.
Cette forme transformerait `Unreadable` en blocage, la seule inversion qui
gèlerait la boucle sur une supposition. Les deux surfaces passent par elle
(`executor.rs:2789`, `auto_pull.rs:4289`). Côté classification, un compte
illisible est `None`, jamais `0` (`classify_launcher_health`, `:230`) : « je
n'ai pas pu compter » ne vaut pas « j'ai compté zéro ».

### 4. Le `0` ne désarme pas ; un kill-switch désarme, et désarmé il mesure encore

Trois variables : `MIKA_PILOT_LAUNCHER_BRAKE_WINDOW_SECS`, `_THRESHOLD` et
`MIKA_PILOT_LAUNCHER_BRAKE` (le kill-switch). Pour les deux bornes, une valeur
absente ou vide prend le défaut. Une valeur illisible, `0` ou négative prend le
défaut **et** produit un WARN qui cite la valeur (`parse_positive_i64`, `:117`).

Le `0` est le piège, parce que ses deux lectures sont fausses en sens opposés :

- un **seuil** à 0 freine *avant la première mort*, donc gèle la flotte sur un
  hôte sain ;
- une **fenêtre** à 0 ne couvre aucun instant, donc rend le frein inerte.

Aucune des deux n'est honorée. Le désarmement passe par une variable dédiée, et
une valeur non reconnue de cette variable **laisse le frein armé**, avec un WARN
(`parse_brake_enabled`, `:143`). Une coquille ne doit pas désarmer un frein de
coût.

Une fois désarmé, le frein **compte toujours**. S'il aurait mordu, il émet
`pilot_launcher_brake_disarmed` (`launcher_health`, `:295`). Sans cette ligne,
« frein désarmé » et « hôte sain » produiraient les mêmes octets : c'est la
classe mika#2205, appliquée cette fois au frein. La détection reste
inconditionnelle et seule la disposition dépend du kill-switch (patron
mika#2249/#2272). Pour la même raison, la ligne `recovered` (§16) est écrite
même frein désarmé, et seulement sur **transition** : elle n'est posée que si la
fenêtre portait déjà une mort, sinon le régime sain émettrait une ligne à chaque
dispatch (mika#2131).

### 5. Un frein d'hôte ne dépense pas le budget du ticket

La porte de readiness (surface A, `validate_dispatch_readiness`) refuse avec
`pilot_launcher_braked`. Mais le ticket garde son label `ready`, et la Phase 2
d'`auto_pull` le re-drive. Sans surface B, chaque re-drive refusé consomme le
budget du ticket, et au troisième un ticket **sain** part en
`operator-review`, abandonné à cause d'une panne qui n'est pas la sienne.

D'où `classify_stuck_ready → Skip { reason: "pilot_launcher_braked" }`
(`auto_pull.rs:1875`) :

- **avant** tout bras qui mute l'état du ticket (`ReEntry` efface un budget,
  `Eligible` en dépense un, `Abandon` le parque) ;
- **après** le bras `in_flight`, pour qu'un ticket dont un pilote travaille déjà
  garde ce nom, plus précis ;
- résolu **une fois par tick** (le compte porte sur l'hôte, pas sur le ticket),
  comme son voisin `egress_relay_down`.

Il fait aussi borner la surface A aux skills de pilote (`PILOT_DISPATCH_SKILLS`,
`executor.rs:2325`). Un lanceur mort ne dit rien d'un `deploy_mika`, et le
déploiement est justement l'un des gestes qui réparent l'hôte.

## Why This Matters

Chacune des formes simples rate dans un sens précis :

| Forme simple | Ce qu'elle fait en vrai |
|---|---|
| compteur persistant | demande un site de remise à zéro ; s'il manque, frein collé ; s'il est mal placé, compteur qui ment (mika#2158) |
| `count_…_for_target` réutilisé | rend 1 à chaque fenêtre, le frein ne mord jamais (et les tests verts si on pose une seule ligne) |
| `bool` / `!matches!(v, Healthy)` | une base indisponible gèle la flotte |
| `0` = désarmé | une coquille désarme le frein de coût, en silence |
| kill-switch qui coupe la mesure | « désarmé » indiscernable de « sain » |
| refus sans surface B | des tickets sains abandonnés en `operator-review` derrière une panne d'hôte |

## When to Apply

Pour toute garde dont la disposition est **globale à l'hôte** (lanceur, relais
d'egress, jeton, disque) et dont le signal est déjà une ligne d'audit par
événement. Avant d'écrire la garde, répondez à quatre questions : quelle est la
population (§2) ? qui paie un faux positif et qui paie un faux négatif (§3) ?
que veut dire `0` pour chaque borne (§4) ? quelle boucle de re-drive le refus
va-t-il nourrir (§5) ?

## Examples

Tests qui épinglent ces choix (tous dans le diff de la PR #2656) :

- `mika2634_an_unreadable_ledger_is_not_a_brake`,
  `mika2634_an_unreadable_count_is_not_a_zero_count`
  (`pilot_launcher_health.rs:456`, `:475`) : §3.
- `mika2634_zero_and_negative_take_the_default_and_do_not_disarm` (`:506`) : §4.
- `mika2634_a_braked_launcher_skips_without_resetting_the_budget`
  (`auto_pull.rs:9622`) : §5.
- `mika2634_the_sole_writer_scan_still_catches_a_second_site`
  (`canonical_tokens.rs:1409`) : le scan « un seul écrivain » est passé d'une
  comparaison par sous-chaîne à une comparaison exacte, parce que
  `pilot_launcher_health_unreadable`, un nom d'**événement**, comptait comme un
  second écrivain de la ligne d'audit. Le test vérifie que la garde resserrée
  attrape **encore** un vrai second site : resserrer une garde sans le montrer,
  c'est la laisser devenir inerte sans bruit.
