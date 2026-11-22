# mika#3117 — `auto_pull` ne lit aucun verdict de grooming terminal

**Vol 1 de 2 — l'ESCALATE de groom.** Type : `fix`. Branche :
`fix/3117/auto-pull-apr-s-un-escalate-une-halte-ou`.

## Le défaut, tel qu'il se mesure

Trois portes existent pour tenir un ticket dont le grooming s'est arrêté sur un
verdict terminal. Sur le chemin de la phase 2 de l'auto-pull, **les trois sont
désarmées par construction**, et aucune ne l'est par accident : chacune a été
écrite pour laisser passer le geste de reprise d'un opérateur, et la phase 2
fabrique un geste qui en est indiscernable.

1. **La porte mika#2545** (`skills/executor.rs:4343-4449`) refuse un re-dispatch
   automatique de groom quand le dernier groom du ticket a escaladé. Son
   deuxième terme conjonctif est `originating_message.is_none()` — *le
   discriminant de ré-armement*, et son propre commentaire (l.4360-4369) nomme
   ce qui le désarme : « `ready_label_handler` carries the webhook marker […] So
   the two canonical recovery gestures — re-posting `ready` as remove→add
   (mika#2323) […] traverse this gate BY CONSTRUCTION. »
2. **Le park mika#2315** (`ready_label.rs:324-332`) refuse d'appliquer `ready`
   quand le dernier événement de la timeline est un `unlabeled` par un acteur
   **non machine**. La phase 2 retire puis repose sous l'identité machine, et le
   commentaire du site le dit en clair (`auto_pull.rs:6675-6678`) : « Between the
   remove above and this add, the last `ready` event is a *machine* `unlabeled`
   — the rescue does not park itself (AC4). »
3. **La porte `is_groomed`** ne front pas la phase 2, et c'est délibéré
   (`auto_pull.rs:6447-6458`, mika#2140 / mika#2170 F4) : la phase 2 *rejoue* un
   `ready` qu'un autre a posé, elle ne *décide* pas de l'engager, et la gater
   contredirait un opérateur qui pose `ready` à la main.

Résultat : `auto_pull.rs` ne lit **aucun** verdict de grooming. Ses seules tenues
sont des labels (`blocked`, `operator-review`, label de refus — doc de
`FILTER_OPERATOR_HELD` à `auto_pull.rs:1876`) et un booléen `is_groomed` absent
de la phase 2. Un ESCALATE terminal n'est donc, pour cette boucle, pas un fait.

**Ce que ça a coûté, et ce que la forge corrobore.**

| date | mesure | corroboration |
|---|---|---|
| 2026-11-10 | quatre grooms sur le même ESCALATE de `mika#1727` (15:20, 15:40, 16:00Z), ≈ 12 $ ; arrêt par `auto_pull_redrive_abandoned` | parking mika#2983, commentaire 42 |
| 2026-11-16 | `ESCALATE second-pass` à 22:36:59Z ; `phase2_stuck_rescue` à 22:40:15Z ; remove→add machine à **22:40:21Z / 22:40:26Z** ; nouveau groom escaladant pour la même cause (1,27 $) | `gh api repos/senara-solutions/mika/issues/1727/events` — les deux événements portent `actor: mika-platform-bot[bot]` |
| 2026-11-19 | pilote `fa982bb6` relancé **2 min 25 s** après l'arrêt sans commit de `137111d7`, sur un plan en HALTE ; 1,34 $ pour deux pilotes, zéro commit | parking mika#2983, commentaires 58b / 59b |

Le troisième cas est traité au **vol 2** (voir § Périmètre).

**Un fait du ticket que la forge ne corrobore pas, et que ce plan ne corrige
donc pas.** Le défaut n°4 du commentaire 55 (« 17/11 : `ready` retiré à la main
vers 13:04Z, reposé par le feeder à 13:10:32Z ») n'apparaît pas sur la timeline
de `mika#1727` : le dernier mouvement de `ready` avant le 22/11 est un `labeled`
par `samidarko` le **2026-11-17T07:27:59Z**, et aucun `unlabeled` ni aucun
`operator-review` ne suit. Le code dit la même chose : les phases 0 et 1 passent
`timeline: None` à `apply_ready_label` (`auto_pull.rs:5546`, `:5775`), donc
`ready_label::apply_ready` lit la timeline elle-même et le park mika#2315 mord
sur un retrait humain aux **trois** phases. Un correctif « faire du retrait
humain une tenue pour le feeder » serait donc un correctif sans défaut.

Ce qui reste vrai et qui est, lui, dans le périmètre : le commentaire 55 note que
le ticket « portait encore le callout de groom de V3c-2b ». Sur un ticket
multi-vol, **un callout `GROOMED` ancien satisfait `is_groomed` et masque un
ESCALATE récent** — les phases 0 et 1 ne sont donc pas couvertes par
`FILTER_NOT_GROOMED` dans ce cas précis, et le filtre de ce plan les couvre.

## Requirements

- **R1.** `auto_pull` lit l'état terminal du dernier grooming d'un ticket **par
  le lecteur existant**, `skills::executor::groom_verdict_state`, et n'en écrit
  pas un second. La grammaire du marqueur (`GROOM_ESCALATE_MARKER`, l'ancrage en
  début de ligne, la sélection de la ligne décisive) reste à son site unique.
- **R2.** L'exclusion porte un **nom propre** et n'est fondue avec aucune
  voisine : le remède de `not_groomed` est « groomer », celui de cet état est
  « l'opérateur tranche, puis lève la tenue ». Doctrine du filtre nommé
  (précédents `below_threshold` / `no_ready_label_event`,
  `ready_parked` / `ready_absent_at_dispatch`).
- **R3.** Les quatre états de `GroomVerdictState` sont traités par un `match`
  **exhaustif sans bras `_ =>`**, comme au site d'`executor.rs:4406` : un
  cinquième état devra être décidé par le compilateur, aux deux sites.
- **R4.** `AnchorWithheld` **passe** — même raisonnement que `executor.rs:4408-4427`
  (mika#3027) : une retenue d'ancrage est un retrait du moteur, pas un jugement
  de l'architecte ; la passe suivante converge et la dépense est bornée en amont
  par le frein mika#2786. Refuser ici facturerait à l'opérateur un retrait/repose
  de `ready` pour obtenir ce que le tick suivant rendrait.
- **R5.** `Unreadable` **refuse**, sous son propre nom, et pas sous celui de
  l'ESCALATE : même disposition, deux lectures opérateur, et les fondre rendrait
  la population du fail-closed incomptable (mika#2205). Le coût est nommé — un
  re-groom légitime attend — et convergent : le tick suivant relit.
- **R6.** Les lectures de base sont **bornées par tick** : aucune interrogation
  sur la population, une seule par candidat effectivement engagé. En phase 2, le
  site est **après `attempted += 1`** (`auto_pull.rs:6531`), donc plafonné par
  `MAX_STUCK_RESCUE_PER_TICK` — précédent mika#3047 pour
  `gh_reread_issue_at_dispatch`.
- **R7.** Aucune des trois portes existantes n'est modifiée. En particulier, la
  porte mika#2545 garde son discriminant `originating_message.is_none()` : le
  remove→add **humain** reste la seule issue après un ESCALATE terminal, et le
  refuser serait enfermer l'opérateur. La décision va chez le **producteur**.
- **R8.** L'exclusion est **comptable** : elle passe par `ExclusionLedger`
  (mika#2131 / mika#2132), donc une ligne de détail par ticket dans
  `audit_events` et l'agrégat par tick, dédupliquée par `(phase, issue, filter)`.

## Design

### Le lecteur, réutilisé tel quel

`skills::executor::groom_verdict_state(db, issue_url) -> GroomVerdictState` est
déjà `pub(crate)` et `crate::skills` est `pub` — aucun changement de visibilité.
Il émet déjà, **sur toutes ses branches**, la sonde `groom_verdict_selected`
portant `rows_examined` / `skipped_neutral` / `verdict` (mika#2795). C'est ce qui
rend observable le seul mode de panne silencieuse de ce correctif : une URL
d'issue mal formée rendrait `rows_examined = 0`, donc `NotEscalated`, donc un
filtre qui ne mord **jamais** sans rien dire (classe mika#2331). Le contrôle
positif est déjà là ; AC3 l'exige comme preuve.

### Le filtre, côté `auto_pull`

Deux constantes nouvelles, voisines des vingt-et-une existantes :

```rust
/// Le dernier grooming de ce ticket a rendu un verdict TERMINAL.
const FILTER_GROOM_ESCALATED: &str = "groom_escalate_terminal";
/// On n'a pas pu établir si le dernier grooming a escaladé. Fail-closed.
const FILTER_GROOM_VERDICT_UNREADABLE: &str = "groom_verdict_unreadable";
```

Un seul traducteur, `async fn groom_halt_exclusion(db, n) -> Option<&'static str>` :
il construit `https://{DEFAULT_REPO}/issues/{n}` sous la même forme que le site
d'`executor.rs:4402`, appelle le lecteur, et `match`e exhaustivement —
`NotEscalated` et `AnchorWithheld` rendent `None`, `Escalated` et `Unreadable`
rendent leur filtre.

### Les trois sites d'insertion

| phase | site | position |
|---|---|---|
| 2 (`phase2_stuck_rescue`) | après `gh_reread_issue_at_dispatch` (`:6557`), **avant** `dispatch_rescued_ticket_in_process` (`:6599`) et donc avant le remove→add (`:6668`) | borné par `MAX_STUCK_RESCUE_PER_TICK` |
| 1 (`phase1_idle_pull`) | sur le candidat élu, avant `apply_ready_label` (`:5775`) | un appel par candidat engagé ; un refus consomme un point du `PHASE1_DECLINE_BUDGET` et la boucle descend le classement |
| 0 (`phase0_feeder`) | sur le candidat retenu, avant `apply_ready_label` (`:5546`) | un appel par promotion, pas par candidat examiné |

`groomed_candidate_exclusion` (`:2455-2476`) et
`classify_stuck_ready_in_memory` (`:2548`) restent **synchrones et in-memory** :
le nouveau filtre lit la base, il ne peut pas y vivre, et les y faire entrer
transformerait deux fonctions pures en fonctions à effet. Le refus s'enregistre
donc au site d'engagement, exactement comme `FILTER_READY_PARKED` et
`FILTER_READY_ABSENT_AT_DISPATCH` le font déjà.

### Le détecteur structurel

Un test lit le source d'`auto_pull.rs` et assert que **chacun** des sites qui
engage un ticket — les deux `apply_ready_label` des phases 0/1 et le
`dispatch_rescued_ticket_in_process` de la phase 2 — est précédé d'un appel à
`groom_halt_exclusion` dans la même fonction, et que leur cardinalité est
exactement 3. Précédent de forme dans ce même fichier : les tests de
`:8758` et `:8775`, qui lisent déjà le source pour pinner un ordre.

## Périmètre — ce vol et le suivant

**Dans ce vol (vol 1) :** l'état *ESCALATE de groom* (y compris la variante
illisible), aux trois phases, avec son filtre nommé, son ledger et son détecteur.
C'est l'état le plus cher des trois mesurés : ≈ 12 $ sur une seule journée.

**Renvoyé au vol 2, nommé et non implémenté ici :** l'état *arrêt sans commit sur
un plan en HALTE* (commentaires 58b / 59b, 2026-11-19). Il appelle un lecteur
différent — l'issue du dernier pilote, pas le verdict du dernier groom — et donc
un filtre distinct ; le livrer ici doublerait le vol et le ferait passer la borne.
Le vol 1 est mergeable seul : il ferme l'ESCALATE sans rien promettre sur la
halte de plan.

**Hors du ticket, déjà relevé ailleurs :** l'auto-lancement mika#1614 du pilote
après un groom en `success` dont le plan porte une section HALTE (commentaires
58a / 59a) — c'est un défaut de `mika#1614`, pas de la phase 2 ni du feeder, et
le titre de ce ticket l'exclut. De même, le motif faux servi par
`ready_label_degroomed` le 16/11 (`pr#3069`, une PR d'un vol antérieur, là où la
cause réelle est l'absence de marqueur `groomed_verdict` sur la nouvelle branche)
est un défaut de classe mika#2331 dans le handler de label, pas dans `auto_pull`.
Enfin, le mécanisme qui a posé `operator-review` sur `mika#1727` à 16:20:03Z le
10/11 sans geste humain reste **sans nom** : ce plan ne prétend pas l'avoir
identifié.

Tracked in: senara-solutions/mika#2983

## Verification Contract

1. `cargo test -p mika-agent auto_pull` — vert, nouveaux tests compris.
2. `cargo test -p mika-agent` — suite entière verte (le `match` exhaustif
   d'`executor.rs` est inchangé ; sa table de vérité ne doit pas bouger).
3. `cargo clippy --all-targets -- -D warnings` — propre.
4. `cargo fmt --check` — propre.
5. `scripts/verify-pipeline.sh origin/main` — vert.
6. `make verify-bundled-skills` — vert (aucun bundle touché ; contrôle de
   non-régression).
7. Table de vérité unitaire des quatre états × trois phases, sans base réelle :
   le traducteur est testé aux bornes comme fonction de `GroomVerdictState` vers
   `Option<&'static str>`.
8. Test de non-régression du geste humain : un remove→add posé par une identité
   **non machine** sur un ticket escaladé n'est refusé par aucun des trois sites
   nouveaux (la porte mika#2545 d'`executor.rs` le laisse déjà passer, et ce vol
   ne doit pas le reprendre par la fenêtre).
9. Mesure post-déploiement, sur le premier ticket réel dont un groom escalade :
   une ligne `auto_pull_exclusion` portant `after_value = groom_escalate_terminal`
   et une ligne `groom_verdict_selected` dont `rows_examined > 0`. Les deux, pas
   une : la première dit que le filtre a mordu, la seconde que la jointure a vu
   des lignes.

## Definition of Done

- [ ] `FILTER_GROOM_ESCALATED` et `FILTER_GROOM_VERDICT_UNREADABLE` existent dans
      `auto_pull.rs`, chacune avec sa doc nommant son remède propre.
- [ ] `groom_halt_exclusion` est le site unique de traduction, avec un `match`
      exhaustif sans bras `_ =>`.
- [ ] Les trois sites d'insertion sont en place, aux positions du tableau
      § Design, et la phase 2 est après `attempted += 1`.
- [ ] Les refus passent par `ExclusionLedger::record` avec la bonne
      `ExclusionPhase`.
- [ ] Aucune modification de `skills/executor.rs:4343-4449`, de
      `ready_label.rs::is_parked`, ni du commentaire de conception
      `auto_pull.rs:6447-6458`.
- [ ] Le détecteur structurel est vert et sa cardinalité attendue est 3.
- [ ] Les sept premiers points du Verification Contract passent ; le point 9 est
      une mesure post-déploiement, pas une condition de merge.
- [ ] Le corps de la PR porte `Code review: delegated to mika-qa (mika#2694)` et
      la ligne `Tracked in: senara-solutions/mika#2983` pour le vol 2.

## Acceptance criteria

Le corps de `mika#3117` ne porte pas de section `## Acceptance criteria` : les
critères ci-dessous sont dérivés des Requirements et du Verification Contract
ci-dessus.

- **AC1.** Un ticket dont le dernier callback terminal de groom porte une ligne
  `Outcome: ESCALATE` d'architecte n'est engagé par **aucune** des trois phases :
  la phase 2 ne le re-dispatche pas et ne touche pas son label, la phase 1 ne
  l'élit pas, la phase 0 ne lui pose pas `ready`. Chaque refus laisse une ligne
  `auto_pull_exclusion` portant `after_value = groom_escalate_terminal` et la
  phase dans `reasoning`.
- **AC2.** Un ticket dont le verdict est illisible (erreur de base, ou `result`
  NULL sur un callback terminal) est refusé de la même façon mais sous
  `groom_verdict_unreadable`, **jamais** sous `groom_escalate_terminal` : les deux
  populations sont séparables par un `grep` sur `after_value`.
- **AC3.** Sur chaque refus, la sonde `groom_verdict_selected` porte
  `rows_examined > 0`. Un filtre qui refuserait avec `rows_examined = 0` serait
  une URL mal formée, pas un ESCALATE — et l'absence totale de la ligne signifie
  un binaire antérieur à ce correctif (classe mika#2340), jamais zéro lecture
  (classe mika#2331).
- **AC4.** Un ticket dont le dernier groom a rendu `AnchorWithheld` (ESCALATE
  écrit par le moteur, `Engine reason:` présent) **passe** les trois phases, et
  aucune ligne d'exclusion n'est écrite pour lui. Le frein mika#2786 reste la
  seule borne de cette population.
- **AC5.** Un ticket dont le dernier groom a rendu `GROOMED` — ou qui n'a jamais
  été groomé — traverse les trois phases exactement comme avant ce correctif :
  aucune ligne d'exclusion nouvelle, aucun appel supplémentaire à la forge.
- **AC6.** Un remove→add de `ready` posé par une identité **non machine** sur un
  ticket escaladé continue de dispatcher un groom. C'est le geste de reprise de
  l'opérateur, et il reste sa seule issue : ce vol ne le reprend ni par la porte
  mika#2545, ni par le park mika#2315, ni par ses trois sites nouveaux.
- **AC7.** Le nombre de lectures de base ajoutées par tick est borné :
  au plus `MAX_STUCK_RESCUE_PER_TICK` en phase 2, au plus une par candidat
  engagé en phases 0 et 1, et **zéro** pour un ticket écarté plus tôt par un
  filtre in-memory.
- **AC8.** Le détecteur structurel rougit si un quatrième site d'engagement
  apparaît dans `auto_pull.rs` sans passer par `groom_halt_exclusion`.

## Fire-Disposition

Ce vol livre un détecteur : le scan structurel d'AC8.

**Option retenue : (a) exception nommée en allowlist — avec une population
d'exceptions vide, mesurée, et une assertion de cardinalité qui la tient.**

Le détecteur atterrit **armé et vert**, sans aucune entrée d'allowlist, parce que
les trois sites d'engagement qu'il connaît sont exactement les trois que ce vol
met en conformité : les deux `apply_ready_label` des phases 0 et 1 et le
`dispatch_rescued_ticket_in_process` de la phase 2. Il n'y a pas de violation
préexistante à excepter — la mesure est le `grep` du § Design, et elle rend 3.

L'assertion auto-nettoyante est l'**assertion de cardinalité** : le test déclare
`EXPECTED_ENGAGEMENT_SITES = 3` et rougit dans les deux sens — un quatrième site
non conforme le fait échouer (c'est AC8), et la disparition d'un des trois le
fait échouer aussi, avec le message qui dit lequel. Une exception ne peut donc pas
devenir stale sans que le test le dise, puisqu'il n'y en a aucune à laisser
pourrir : c'est le cas limite sain de l'option (a), pas un contournement. Si
l'implémentation découvre un quatrième site qu'elle ne peut pas mettre en
conformité dans ce vol, elle lui ouvre une entrée nommée référençant
`senara-solutions/mika#2983` — elle ne relâche pas l'assertion.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/auto_pull.rs` — deux constantes de filtre + doc | 25 |
| `crates/mika-agent/src/auto_pull.rs` — `groom_halt_exclusion` (`match` exhaustif + doc) | 35 |
| `crates/mika-agent/src/auto_pull.rs` — insertion phase 2 (après `attempted += 1`) | 20 |
| `crates/mika-agent/src/auto_pull.rs` — insertion phase 1 (budget de déclin) | 20 |
| `crates/mika-agent/src/auto_pull.rs` — insertion phase 0 (feeder) | 20 |
| `crates/mika-agent/src/skills/executor.rs` — doc du second consommateur du lecteur | 10 |
| tests — table de vérité des 4 états × 3 phases | 60 |
| tests — ledger (phase, filtre, dédup) et borne de lectures par tick | 80 |
| tests — non-régression du geste humain et du cas `GROOMED` | 30 |
| tests — détecteur structurel des trois sites d'engagement | 60 |

Total estimé : 360 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 360 estimées sous une borne de 429 et sous le
seuil par PR de 1000, avec une marge de 69 lignes. Le vol ne touche qu'un module
en écriture (`auto_pull.rs`) et n'en modifie aucune fonction existante : il ajoute
deux constantes, une fonction et trois appels. Les deux portes voisines
(`skills/executor.rs`, `ready_label.rs`) sont lues et citées, jamais modifiées —
c'est la raison pour laquelle la marge tient : il n'y a pas de table de vérité
existante à re-dérouler. L'état *arrêt sans commit sur un plan en HALTE*, qui
aurait coûté un second lecteur et sa propre table, part au vol 2 sous la ligne
`Tracked in:` ci-dessus.

## Ordre de commit

1. Les deux constantes et `groom_halt_exclusion`, avec sa table de vérité
   unitaire. Compile et verdit seul, sans appelant.
2. L'insertion en phase 2 et ses tests de ledger et de borne — c'est le site du
   défaut mesuré le 2026-11-16.
3. Les insertions en phases 1 et 0 et leurs tests, dont le cas du callout
   `GROOMED` ancien qui masque un ESCALATE récent.
4. Le détecteur structurel et l'assertion de cardinalité.
5. La doc du second consommateur dans `skills/executor.rs`.

Le plancher mergeable est le commit 2 : la phase 2 est le producteur du geste
machine indiscernable, et c'est elle qui a brûlé les quatre grooms.
