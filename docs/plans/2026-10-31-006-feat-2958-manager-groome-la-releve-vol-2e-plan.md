# mika#2958 — ce que le cycle a fait de la relève se lit dans le rapport (vol 2e)

> **Périmètre touché hors de la zone déclarée par le corps du ticket — et pourquoi.**
> La « zone attendue » du corps (`groom_trigger.rs`, `cadence.rs`, `spawn.rs`)
> datait du **vol 2b**. Ce vol en sort par **un** fichier :
> `crates/mika-agent/src/milestone_manager/reporter.rs`. C'est inévitable et
> c'est l'objet même d'AC4 : le livrable est un rendu, et le rendu du rapport n'a
> qu'un site. `spawn.rs` et `server/mod.rs`, eux, ne sont **pas** touchés : la
> chaîne d'injection est complète depuis le vol 2d et ce vol ne consomme aucune
> dépendance neuve.
>
> Les trois interdits de périmètre du corps sont **levés** depuis le merge de
> mika#2972 (2026-10-30T08:21:55Z) : `reader.rs`, `types.rs` et le bloc
> `pub use types::{…}` ne sont plus en vol. Ce plan ne les touche pas davantage,
> et il n'ajoute **aucun** littéral `MilestoneState { … }` — les tests de rendu
> passent par `report_with_pool_and_relief` et `base_state()`, les constructeurs
> existants. La zone de mika#2961 (`evidence/guards.rs`,
> `skills/builtin_handlers.rs`, aujourd'hui la PR #2993) n'est pas approchée.
>
> Au 2026-10-31, aucune PR ouverte ne touche `reporter.rs` ni `cadence.rs`.

## Contexte

Le vol 1 (PR #2740) nomme la relève en mode ombre. Les vols 2a → 2d ont construit
le geste et l'ont rendu idempotent :

- **vol 2a** (#2970, `9d6c1225`) — `GroomSender` / `A2aGroomSender`, le
  `message/send` déclaré à plat, les trois refus de fil, `GROOMS_PER_CYCLE = 1`,
  l'exception nommée de `no_dispatch_test.rs`.
- **vol 2b** (#2975) — `resolve_groom_rail`, l'injection `CycleContext::groom_sender`,
  `trigger_cycle_grooms` branché au corps de cycle.
- **vol 2c** (#2988, `43a7499fd`) — `GroomInFlightSource`, `GroomInFlightReading`,
  `groom_in_flight.rs` : la lecture container-scopée des tâches moteur de classe
  `groom`.
- **vol 2d** (#2992, `aca2fcf47`) — le câblage de cette lecture au candidat, la
  fenêtre d'idempotence durable `triggered_grooms` portée par le checkpoint, et
  `GroomTriggerOutcome` **lié** au site d'appel plutôt que jeté.

**Ce qui reste donc dû, et c'est ce vol : la moitié ON d'AC4.** Le résultat du
geste est lié (`cadence.rs:1263`) et consommé par la fenêtre durable, mais rien ne
le rend : le commentaire du site le dit en clair — « Ce qu'il n'a toujours pas,
c'est une destination dans le rapport : c'est le vol 2e (AC4, moitié ON) »
(`cadence.rs:1257-1262`). Aujourd'hui le rapport porte la section du vol 1,
titrée « mode ombre — aucun groom déclenché », et ce titre reste vrai parce que
l'interrupteur est OFF partout. Le jour où Vincent l'ouvre, il devient **faux sans
rien le dire** : des grooms partiraient et le rapport continuerait d'affirmer
qu'aucun n'est déclenché.

Ce vol **clôt le ticket** : AC1 est fermé au vol 2a, AC2 au vol 2d, AC3 aux vols
2a/2b, et AC4 a sa moitié OFF depuis le vol 1. Le corps de PR porte donc
`Closes #2958`, et non un `Tracked in:`.

`MIKA_MANAGER_GROOM_WRITE` reste OFF par défaut. Ce vol ne l'ouvre pas.

## Requirements

- **R1.** Le rapport reçoit ce que le cycle a fait de la relève par un
  **paramètre**, jamais par une relecture d'environnement.
- **R2.** Un appelant qui n'a pas mesuré le geste rend **exactement** la section
  du vol 1, au caractère près — les 97 tests actuels de `reporter.rs` passent sans
  amendement.
- **R3.** Avec l'interrupteur OFF, la section est **inchangée** : même titre, mêmes
  lignes. C'est la moitié OFF d'AC4, et elle est épinglée par un test de cycle, pas
  seulement par un test de rendu.
- **R4.** Avec l'interrupteur ON, le rapport dit **quels** grooms ont été
  déclenchés et **lesquels** ont été sautés, avec leur motif.
- **R5.** Un candidat que le cycle n'a **ni** déclenché **ni** sauté — le cas que
  `AgentBusy` crée en cassant la boucle — n'est présenté ni comme l'un ni comme
  l'autre.
- **R6.** Un interrupteur ouvert dont le geste n'a pas eu lieu (rail absent,
  source de lecture absente) ne se lit **pas** « mode ombre » : le rapport nomme la
  cause.
- **R7.** Aucun littéral de dépôt n'entre dans `milestone_manager/`
  (`no_repo_list_test.rs`), et aucune exception n'est ajoutée ni amendée dans
  `no_dispatch_test.rs` — `EXEMPTED_OCCURRENCES` reste à **trois** entrées.

## Décisions

### D1 — le canal est un paramètre du rapporteur, pas une lecture de l'interrupteur

Nouvelle méthode, sur le patron exact de `report_with_auth_notes` (mika#1949 U4),
qui est l'idiome du dépôt pour étendre ce rapporteur :

```rust
/// Comme [`Reporter::report_with_auth_notes`], plus ce que le cycle a fait de la
/// relève (mika#2958 vol 2e, AC4 moitié ON). `None` rend identiquement à
/// `report_with_auth_notes`.
pub fn report_with_grooms(
    &self,
    state: &MilestoneState,
    assessment: &Assessment,
    auth_notes: &[AuthBoundaryNote],
    grooms: Option<&GroomTriggerOutcome>,
) -> String
```

et `report_with_auth_notes` délègue avec `None`, comme `report` délègue déjà avec
`&[]`. Un seul site de production change d'appel : `cadence.rs:1265`.

**Pourquoi pas `groom_write_is_armed()` lu dans le rapporteur**, qui aurait l'air
plus court — deux raisons, et la seconde est une correction :

1. Le rendu redeviendrait fonction de l'environnement du processus. Les 97 tests
   de `reporter.rs` tournent aujourd'hui en parallèle sans `serial_test` ; chacun
   qui toucherait à la relève devrait prendre le harnais `GroomArme` et le verrou
   sériel. Le rendu est une fonction pure de ses entrées, et il le reste.
2. Ce serait un **second lecteur** du même interrupteur (mika#2201), et il pourrait
   mentir : le cycle lit l'armement **une fois**, à son site
   (`trigger_cycle_grooms`, décision D4 du vol 2d), et le rapport est composé après.
   Un interrupteur ouvert entre les deux instants ferait dire au rapport « ON »
   d'un cycle qui s'est déroulé OFF. Le rapport dit **ce qui a eu lieu**, jamais ce
   que l'environnement vaut à l'instant du rendu.

### D2 — `None` veut dire « le geste n'a pas été mesuré », et c'est le rendu du vol 1

Trois appelants n'ont pas d'outcome et n'en auront pas : `Reporter::report`
(délégation), `reader.rs:3707` et `db/tests/vols_termines_du_manager.rs:908`. Pour
eux, `None` — motif mika#2331 : l'absence de mesure s'écrit par l'absence, jamais
par un zéro qui ressemble à une réponse.

**`None` et `Some(outcome dont le refus est `disarmed`)` rendent la même section,
et c'est l'AC, pas une confusion.** AC4 est explicite : « Interrupteur OFF : la
section « Relève à grommer » est **inchangée** ». Un futur éditeur tenté de
« séparer les deux populations » (mika#2205) séparerait précisément ce que l'AC
demande d'identifier. C'est écrit au site, avec la citation de l'AC, pour que la
tentation ait une réponse.

### D3 — `GroomTriggerOutcome` gagne le motif de refus de cycle, parce que « vide » ne distingue pas « désarmé » de « armé sans geste »

C'est la décision centrale du vol, et elle est la raison pour laquelle il pèse plus
que les ≈ 120 lignes que le vol 2d avait prospectées.

Cinq chemins rendent aujourd'hui `GroomTriggerOutcome::default()` — vide, vide :

| chemin | motif journalisé | section rendue ? |
|---|---|---|
| `trigger_cycle_grooms`, `relief == None` | `no_relief_verdict` | **non** (`if let Some(relief)`) |
| `cycle_refusal`, `!armed` | `disarmed` | oui |
| `trigger_cycle_grooms`, source absente | `no_in_flight_source` | oui |
| `cycle_refusal`, émetteur absent | `no_sender` | oui |
| `cycle_refusal`, relève vide | `no_relief` | **non** (`!relief.next.is_empty()`) |

Trois chemins visibles au rendu, et un rendu piloté par le seul « vide » les
confondrait sous le titre « mode ombre ». Pour deux d'entre eux ce titre serait un
**mensonge** : l'interrupteur est ouvert, rien ne part, et l'opérateur qui vient
justement de l'ouvrir ne lit nulle part pourquoi. C'est mika#2205 sur l'axe même où
AC4 se lit. R6 l'exige.

Le champ, troisième du type :

```rust
/// Le refus qui a arrêté le **cycle entier**, ou `None` si le geste a eu lieu.
///
/// Le vocabulaire est celui des sondes, et il est fermé : `disarmed`,
/// `no_sender`, `no_relief` (les trois de [`cycle_refusal`]), plus
/// `no_relief_verdict` et `no_in_flight_source` (les deux du site d'appel, dans
/// `cadence.rs`). `None` ne dit pas « tout est parti » : il dit que la boucle par
/// candidat a tourné, et ce sont alors `declenches` et `sautes` qui portent la
/// réponse.
///
/// Sans ce champ, un `GroomTriggerOutcome` vide a cinq causes possibles dont
/// trois sont visibles au rapport (mika#2958 vol 2e, D3).
pub refus_de_cycle: Option<&'static str>,
```

Rempli à quatre sites, tous existants :

- `trigger_relief_grooms`, bras `(Some(reason), _)` → `Some(reason)`, le motif
  qu'il journalise déjà : aucun second calcul, la valeur est la même variable.
- `trigger_relief_grooms`, bras `(None, None)` inatteignable → `Some("no_sender")`,
  le motif que `cycle_refusal` aurait rendu pour cet état. Un `None` y serait
  l'affirmation « le geste a eu lieu » dans la branche qui existe précisément pour
  ne pas paniquer un cycle.
- `trigger_cycle_grooms`, ses deux refus propres → `Some("no_relief_verdict")` et
  `Some("no_in_flight_source")`.
- `envoyer` → `None`, par `Default` : le geste a eu lieu.

**Trois assertions existantes comparent à `GroomTriggerOutcome::default()` sur une
branche `armed = false`**, où le champ vaut désormais `Some("disarmed")`. Elles
sont amendées dans le même commit, en **renforcement** et non en relâchement — la
forme nommée épingle le motif en plus du vide :

```rust
GroomTriggerOutcome {
    refus_de_cycle: Some("disarmed"),
    ..Default::default()
}
```

Les trois sites : `groom_trigger.rs:789`
(`mika2958_off_naucun_envoi_et_son_controle_negatif`), `groom_trigger.rs:1288`
(`mika2958_le_rail_arme_a_false_nemet_aucune_requete`) et
`cadence.rs:3762` (`mika2958_une_releve_non_mesuree_ne_declenche_rien` — celui-là
attend `Some("no_relief_verdict")`, et il devient ainsi le test qui **épingle** ce
motif, que rien n'épinglait).

Le bloc `pub use groom_trigger::{…}` de `mod.rs:319` réexporte déjà
`GroomTriggerOutcome` : aucune réexportation neuve, et le bloc n'est pas touché.

### D4 — le segment de titre : trois formes, et une seule est l'existante au caractère près

```
### Relève à grommer (<segment>)
```

| état de `grooms` | segment |
|---|---|
| `None`, ou `refus_de_cycle == Some("disarmed")` | `mode ombre — aucun groom déclenché` *(inchangé, R2/R3)* |
| `refus_de_cycle == Some(r)`, `r != "disarmed"` | `aucun groom déclenché — <r>` |
| `refus_de_cycle == None`, `n` déclenchés, `n > 0` | `<n> groom<pluriel> déclenché<pluriel>` |
| `refus_de_cycle == None`, `n == 0` | `aucun groom déclenché` |

Le littéral `mode ombre — aucun groom déclenché` n'existe **qu'une fois** en
production : les trois autres formes se composent ailleurs, et aucune ne le
recopie. La sonde S2 le vérifie.

Le pluriel se dérive comme celui de `relief.beyond` le fait déjà à trois lignes de
là (`let pluriel = if n == 1 { "" } else { "s" }`). Le rendu **ne suppose pas**
`GROOMS_PER_CYCLE = 1` : la borne est une constante qui peut monter, et un rendu
qui écrirait « 1 groom déclenché » en dur serait faux ce jour-là sans que rien ne
rougisse.

### D5 — l'annotation par candidat, et la troisième catégorie qu'`AgentBusy` crée

Quand `refus_de_cycle == None`, chaque ligne de candidat reçoit un suffixe :

```
- #1802 (p1-important) — à grommer : plan groomé absent — groom déclenché
- #1803 — à grommer : plan groomé absent — sauté : budget_spent
- #1804 — à grommer : plan groomé absent — non atteint
```

`declenches ∪ sautes` **n'est pas** `relief.next` : sur `Err(AgentBusy)`, `envoyer`
casse la boucle (`groom_trigger.rs:677`), et les candidats restants n'entrent dans
aucune des deux listes. Les annoter « sauté » leur donnerait un motif qu'ils n'ont
pas, et choisir l'un des cinq jetons existants serait inventer une mesure
(mika#2331). Ils sont donc **non atteints** : le cycle s'est arrêté avant eux.

L'ordre de consultation est `declenches`, puis `sautes`, puis « non atteint ». La
double appartenance est impossible par construction d'`envoyer` (`continue` exclut
le `push` de succès) ; l'ordre est écrit pour être déterministe le jour où cette
construction changerait, pas pour arbitrer un cas d'aujourd'hui.

Quand `refus_de_cycle.is_some()` ou que `grooms` est `None`, **aucune annotation** :
la ligne est exactement celle du vol 1. Un suffixe « non atteint » sur chacune des
quatre lignes d'un cycle désarmé serait du bruit qui répète le titre.

La ligne « N autres candidats au-delà des 4 nommés » n'est pas annotée non plus :
`relief_candidates` ne porte que `relief.next` (`cadence.rs:1072`), donc le cycle
n'a rien tenté sur ce reste et n'a rien à en dire.

### D6 — le motif rendu est le jeton de sonde, nu

Le bassin passe par `RefusalReason::label()` parce que `RefusalReason` est un enum
dont le libellé a déjà un site unique. Ici le motif **est** un `&'static str`, et
c'est le mot même que le journal porte : `manager_groom_skipped reason=…`,
`manager_groom_not_triggered reason=…`. Une table de traduction jeton → libellé
français serait une seconde orthographe du même fait (mika#2201), elle divergerait
le jour où un jeton est renommé, et surtout elle ôterait à l'opérateur ce que le
jeton nu lui donne : lire `agent_busy` au rapport, puis grep le journal avec ce
mot. Le jeton est rendu tel quel, et un test le fige.

C'est le même arbitrage que le vol 2a a déjà rendu pour le fil : `GroomSendError`
ne porte pas de message, il porte `classe()`, et le motif au rapport est celui de
la classe (`groom_trigger.rs:426-429`).

### D7 — aucune section nouvelle : l'annotation vit dans la section du vol 1

AC4 dit « la section « Relève à grommer » » et « le rapport dit quels grooms ont
été déclenchés, et lesquels ont été sautés » — une seule section, enrichie. Une
seconde section redirait la même liste de numéros avec son propre suffixe et son
propre ordre, et les deux divergeraient : c'est exactement ce que le commentaire du
vol 1 interdit déjà pour `candidate_suffix` (« deux rendus de la même paire
(numéro, provenance) dans un même rapport se contrediraient le jour où l'un
change », `reporter.rs:468-471`).

### D8 — pas de `## Fire-Disposition` : ce vol ne livre aucun détecteur

Les livrables sont : un champ de données, une méthode de rendu, un segment de titre,
un suffixe de ligne, et des tests **de rendu et de comportement** — le rapport
porte tel titre, telle ligne ; le cycle désarmé rend la section du vol 1 ; un
candidat non atteint n'est pas dit sauté. Le chemin de succès de chacun est « le
rendu attendu est là », jamais « aucune violation trouvée ».

Aucune garde structurelle n'est livrée ni amendée :
`no_dispatch_test.rs::EXEMPTED_OCCURRENCES` reste à **trois** entrées — ce vol
n'ajoute aucun geste d'écriture, et son assertion auto-nettoyante (`len() == 3`,
`no_dispatch_test.rs:173-178`) resterait verte parce que rien n'y touche ;
`no_repo_list_test.rs` reste satisfait — aucune URL n'est composée ici, le rendu
ne porte que des numéros ; et
`canonical_tokens::mika2700_le_vol_1_nouvre_aucun_chemin_decriture` n'est pas
retouché (il l'a été au vol 2a). Le gate est **N/A**, et la section n'est pas
inventée (mika#2306).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/groom_trigger.rs` — champ `refus_de_cycle`, doc, remplissage aux 3 sites | 28 |
| `crates/mika-agent/src/milestone_manager/groom_trigger.rs` — amendement des 2 assertions (D3) + 1 test du vocabulaire de cycle | 40 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` — les 2 refus propres posent leur motif, passage de l'outcome au rapporteur, 1 assertion amendée | 18 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` — `report_with_grooms`, délégation, segment de titre (D4), annotation par candidat (D5), doc | 85 |
| tests (`reporter.rs`, `mod tests` inline — les quatre formes de titre, les trois annotations, l'identité `None` ↔ vol 1) | 112 |
| tests (`cadence.rs`, `mod tests` inline — E2E ON : le rapport livré nomme le déclenché ; E2E OFF : section inchangée) | 75 |

Total estimé : 358 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 358 < 429, et il n'y a rien à renvoyer : ce vol est
le dernier du ticket, son périmètre est un rendu, et il n'ouvre aucune dépendance
neuve — ni injection, ni requête SQL, ni fichier, ni paramètre de `spawn.rs`.

**Pourquoi 358 et non les ≈ 120 prospectés par le vol 2d.** Le vol 2d avait estimé
« une ligne de passage vers `report_with_auth_notes` plus sa mise en forme et ses
tests de rendu ». La ligne de passage est bien une ligne — mais le rendu ne peut pas
dire la vérité sans D3 : sans le motif de refus de cycle, un interrupteur ouvert
dont le rail manque se lirait « mode ombre », et R6 tombe. Le champ, son remplissage,
les trois assertions à amender et le test qui épingle le vocabulaire, c'est
≈ 90 lignes que la prospection n'avait pas vues. L'écart est nommé ici plutôt que
découvert en vol : le vol 2b avait été estimé à 374 lignes pour ≈ 652 réelles, et
c'est cette calibration qui tient 358 à ~83 % de la borne plutôt qu'à son ras.

## Ordre de commit

Chaque étape compile et la suite passe. Le plancher de chaque étape est ce sans quoi
la suivante n'a pas de sens.

1. **`GroomTriggerOutcome::refus_de_cycle` et son remplissage dans
   `groom_trigger.rs`** (D3), plus l'amendement des deux assertions de ce fichier.
   *Plancher* : `cargo test -p mika-agent milestone_manager::groom_trigger` vert, et
   un test qui épingle les trois motifs de cycle sur l'outcome — le vocabulaire
   voyage désormais par une valeur, donc il devient épinglable autrement que par la
   sonde. Rien ne lit encore le champ : l'étape est isolée exprès, c'est elle qui
   porte la décision.
2. **Les deux refus propres de `trigger_cycle_grooms` posent leur motif**, et
   l'assertion de `cadence.rs:3762` attend `Some("no_relief_verdict")`.
   *Plancher* : `cargo test -p mika-agent milestone_manager::cadence` vert.
3. **`report_with_grooms` et sa délégation, sans aucun changement de rendu** (D1,
   D2). *Plancher* : les 97 tests de `reporter.rs` verts **sans amendement** — c'est
   la preuve de R2, et elle ne vaut qu'à cette étape, avant que le rendu bouge.
4. **Le segment de titre (D4) et l'annotation par candidat (D5)**, avec leurs tests
   de rendu. *Plancher* : les quatre formes de titre et les trois annotations
   figées ; `None` rend toujours la section du vol 1 au caractère près.
5. **Le site d'appel** : `cadence.rs:1265` passe `Some(&grooms)`, et le commentaire
   « c'est le vol 2e » disparaît — il est honoré. *Plancher* : les deux tests de
   cycle, ON et OFF, sur le `report_markdown` que le cycle a **livré**.
6. **Fin de pipeline.** `cargo test`, `cargo clippy --all-targets -- -D warnings`,
   `cargo fmt --check`, les lints du dépôt, et
   `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- `report_with_grooms` existe, `report_with_auth_notes` et `report` y délèguent, et
  le seul site de production qui passe un `Some` est `cadence.rs`.
- Avec `MIKA_MANAGER_GROOM_WRITE` absent, le `report_markdown` livré par un cycle
  porte la section du vol 1, titre et lignes **inchangés** — épinglé par un test de
  cycle, pas seulement par un test de rendu.
- Avec l'interrupteur posé en test, le `report_markdown` livré nomme le groom
  déclenché, et un candidat sauté y porte son motif.
- Un candidat ni déclenché ni sauté se lit « non atteint », et un cycle armé sans
  rail se lit `aucun groom déclenché — no_sender`.
- `GroomTriggerOutcome` porte `refus_de_cycle`, rempli aux quatre sites, et les
  trois assertions nommées en D3 sont amendées dans le même commit.
- `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
  verts ; `scripts/verify-pipeline.sh origin/main` vert.
- `no_dispatch_test.rs`, `no_repo_list_test.rs` et
  `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` passent **sans amendement**.
- Le corps de PR porte `Closes #2958` : AC1 à AC4 sont tous fermés, et l'ouverture
  de l'interrupteur reste le geste de l'opérateur.

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2958.

- **AC1.** Avec l'interrupteur OFF, aucun groom n'est déclenché. C'est une garantie structurelle, avec un contrôle négatif dans le même test. Le geste entre comme **exception nommée** au scan de `no_dispatch_test.rs`, sur le patron des entrées de mika#2698 vol 2 et de mika#2699 vol 4. Le scan n'est pas assoupli. Le test `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` est amendé dans le même commit, et la PR dit ce qui le faisait rougir.
- **AC2.** Sélection et idempotence : un ticket déjà groomé, ou dont un groom est en cours, n'est pas relancé. Le plan dit par quelle lecture le manager voit un groom en cours : les tâches moteur `Groom …` et `ready-label`, ou autre chose. Le motif est la collision d'un groom manuel avec un groom moteur.
- **AC3.** La voie est celle du moteur : la même que `mika ask --agent mika-dev "groom mika#N"`, ou son équivalent interne. Elle n'ouvre ni spawn ni dispatch d'implémentation. Le plan dit comment le geste reste borné : combien de grooms par cycle, et ce qu'il fait quand mika-dev a déjà un pilote en vol, puisque le moteur n'en fait tourner qu'un à la fois.
- **AC4.** Ce qui reste de l'ombre se lit dans le rapport. Interrupteur OFF : la section « Relève à grommer » est inchangée. Interrupteur ON, en test seulement : le rapport dit quels grooms ont été déclenchés, et lesquels ont été sautés, avec leur motif.
- **Taille** : à estimer tests compris, et à découper en vols au-delà de la borne.

### Couverture par ce vol

| AC | état | ce que ce vol y ajoute |
|---|---|---|
| AC1 | **fermé au vol 2a**, préservé ici | l'exception nommée et l'amendement de `mika2700_…` ne sont pas retouchés ; `EXEMPTED_OCCURRENCES` reste à trois entrées (D8), et la sonde S4 l'atteste. Le rendu n'ouvre aucun chemin d'écriture. |
| AC2 | **fermé au vol 2d**, préservé ici | `refus_de_cycle` n'entre dans aucune décision de sélection : il est posé par les sites qui refusent déjà, et lu par le seul rapporteur. La fenêtre durable et la lecture en base sont inchangées. |
| AC3 | **fermé aux vols 2a/2b**, préservé ici | `GROOMS_PER_CYCLE` n'est pas lu par le rendu (D4) et `envoyer` n'est pas touché, donc ni la borne ni l'`AgentBusy ⇒ break` ne bougent. Ce vol **rend visible** ce que ce break laisse : la catégorie « non atteint » (D5). |
| AC4 | **fermé par ce vol** | moitié OFF : R2/R3/D2/D4 — la section du vol 1 est rendue au caractère près, et un test de cycle le montre sur le rapport livré. Moitié ON : R4/R5/R6/D4/D5/D6 — les déclenchés, les sautés avec leur motif, les non atteints, et un interrupteur ouvert sans geste qui nomme sa cause au lieu de dire « mode ombre ». |
| Taille | — | section `## Taille estimée` ci-dessus, 358 lignes, un vol, et aucune suite : ce vol clôt le ticket. |

## Hors périmètre

- **L'ouverture de `MIKA_MANAGER_GROOM_WRITE`.** Geste opérateur, au seuil écrit
  dans mika#2700 : `MIKA_MANAGER_EVALUATE_WRITE` et `MIKA_MANAGER_POOL_WRITE`
  ouverts et stables sur un cycle complet. Les deux sont OFF aujourd'hui.
- **Le rendu du geste du bassin** (`write_pool_admissions`, lié en `_admitted`
  depuis mika#2699 vol 4, `cadence.rs:1256`). Même forme de besoin, même site de
  rendu — mais c'est le vol 5 de mika#2699, avec son propre interrupteur et son
  propre AC. L'y mêler ferait un vol qui ferme deux tickets à moitié.
  **Tracked in: senara-solutions/mika#2699**
- **La canonicalisation de `reference_url` au point d'écriture** (mika#2638 § 8 S1).
- **`reader.rs`, `types.rs`, `spawn.rs`, `server/mod.rs`, `evidence/guards.rs`,
  `skills/builtin_handlers.rs`** — non touchés, et aucun littéral
  `MilestoneState { … }` ajouté.

## Sonde post-déploiement

Statiques — aucune n'exige l'interrupteur ouvert.

- **S1.** `grep -n "report_with_grooms" crates/mika-agent/src/milestone_manager/reporter.rs crates/mika-agent/src/milestone_manager/cadence.rs`
  rend la définition, la délégation depuis `report_with_auth_notes`, et **un seul**
  site d'appel de production.
- **S2.** `grep -c "mode ombre — aucun groom déclenché" crates/mika-agent/src/milestone_manager/reporter.rs`
  — le littéral n'apparaît qu'une fois hors du `mod tests` : les trois autres
  formes de titre se composent, elles ne le recopient pas (D4).
- **S3.** `grep -c "refus_de_cycle" crates/mika-agent/src/milestone_manager/groom_trigger.rs`
  ≥ 5 (déclaration, les trois remplissages, les assertions amendées).
- **S4.** `cargo test -p mika-agent no_dispatch_test no_repo_list_test` vert, et le
  compte d'entrées d'`EXEMPTED_OCCURRENCES` inchangé : trois, pas quatre.
- **S5.** `grep -n "vol 2e" crates/mika-agent/src/milestone_manager/cadence.rs` ne
  rend plus le commentaire « c'est le vol 2e (AC4, moitié ON) » du site d'appel : il
  est honoré, donc il disparaît.
- **S6.** Après `make deploy`, un cycle manager avec l'interrupteur OFF livre un
  rapport dont la section porte exactement
  `### Relève à grommer (mode ombre — aucun groom déclenché)`, et le journal ne
  porte aucun `manager_groom_triggered`.
