# mika#2923 — le bassin refuse, et nomme, un candidat que l'opérateur tient

> **Ticket :** senara-solutions/mika#2923 (`bug`, `p1-important`, `dispatch:loop`,
> `loop-substrate`, milestone *Substrat de boucle*)
> **Portée :** `crates/mika-agent/src/milestone_manager/{pool,relief}.rs` + leurs tests
> et ceux de `reporter.rs`. Aucune autre couche touchée.
> **Le merge revient à Vincent** (`loop-substrate`).

## Contexte

Le bassin de mika-manager (mika#2699) recommande, dans son dernier rapport de sink
du 2026-10-26, d'admettre `#2698`, `#2699` et `#1806` — trois tickets qui portent
`operator-gated`. Le prédicat d'admission ne lit pas ce label :
`git grep -n -iE 'operator-gated|"blocked"|operator-review' -- crates/mika-agent/src/milestone_manager/`
ne rend aujourd'hui que quatre lignes, toutes étrangères à l'admission
(`Severity::Blocked => "blocked"` dans `liveness.rs:280` et `reporter.rs:144`, une
note de `reader.rs:1152`, un nom de section de test `reporter.rs:1349`).

Conséquence le jour où `MIKA_MANAGER_POOL_WRITE` s'ouvre : `pool_writer` ne pose
que ce que `PoolVerdict::admit` nomme — c'est la propriété qui tient le poseur
court — donc il poserait `ready` sur un ticket que l'opérateur retient. C'est
exactement l'autorité que l'interrupteur devait borner, et le seuil écrit de
mika#2699 l'interdit déjà par la phrase « qu'aucun `ready` mal formé n'ait été
admis en ombre ». Ce vol le rend structurel, **avant** toute ouverture de
l'interrupteur (bearing de Prime du 2026-10-26, « go » de Vincent).

## L'endroit, et la source des deux littéraux

Le point 2 du ticket demande que les littéraux viennent d'une source existante du
dépôt « s'il en existe une ». **Il en existe une, et elle est déjà exactement ce
qu'il faut** :

```rust
// crates/mika-agent/src/webhook_dispatch.rs:535
pub(crate) const OPERATOR_HELD_LABELS: &[&str] = &["blocked", "operator-review", "operator-gated"];

// :542
pub(crate) fn operator_held_label<'a>(labels: impl IntoIterator<Item = &'a str>) -> Option<&'a str>
```

Quatre faits qui en font le bon point d'ancrage, et pas seulement une
commodité :

1. **C'est déjà « la liste unique derrière deux surfaces »** (mika#2263) — le
   feeder d'`auto_pull` et la porte du `ready_label_handler`. Le bassin est la
   troisième surface qui pose la même question ; une quatrième orthographe de la
   liste serait la divergence que mika#2263 a déjà payée une fois (« un label qui
   tient un ticket sur un chemin et pas sur l'autre ne tient pas le ticket »).
2. **`operator_held_label` rend *lequel*, jamais un booléen** — pour la raison
   écrite dans son docstring, qui est mot pour mot celle que le point 2 de
   mika#2923 réclame : l'opérateur doit savoir quel label retirer.
3. **`pool.rs` importe déjà de ce module** (`use crate::webhook_dispatch::{DISPATCH_SEAT_LABEL_PREFIX, READY_LABEL};`),
   au même titre que `READY_LABEL` que le ticket cite en exemple. L'import
   s'étend d'un nom, il ne s'invente pas.
4. **La liste est tenue déclarée** : `auto_pull::tests::test_exclusion_labels_are_declared_in_labels_yml`
   (mika#2790) itère sur *cette* constante contre `.github/labels.yml`. Un
   troisième lecteur hérite donc de cette garantie gratuitement.

### Le sur-ensemble est voulu, et il est dit

`OPERATOR_HELD_LABELS` compte **trois** membres là où le ticket en nomme deux :
`operator-review` vient en plus. Ce n'est pas un élargissement discret du
périmètre — c'est le même fait (« quelqu'un d'autre tient ce ticket »), et admettre
un ticket sous `operator-review` serait le même défaut sous un autre nom. Le vol
le rend donc explicite plutôt que de filtrer la liste à deux membres : filtrer
serait créer une quatrième liste, c'est-à-dire exactement ce que le point 2
interdit.

### Un refus argumenté : la comparaison n'est pas normalisée ici

`operator_held_label` compare par **égalité exacte** (`OPERATOR_HELD_LABELS.contains(l)`),
là où `labels_carry_ready` et `labels_hold_dispatch_seat` de `pool.rs` *trim*ment
et ignorent la casse. Ce vol **ne corrige pas** cette asymétrie, et ce n'est pas
un oubli :

- La normaliser **dans `pool.rs`** donnerait une troisième sémantique de
  comparaison sur la même liste — le feeder, la porte `ready` et le bassin
  répondraient différemment à « ce ticket est-il tenu ? », sans qu'aucune
  assertion de comportement rougisse (classe mika#2201).
- La normaliser **dans `operator_held_label`** changerait les trois lecteurs
  d'un coup, dont deux hors périmètre de ce ticket (AC4 demande que rien d'autre
  ne bouge).
- Le risque résiduel est faible et mesurable : les trois noms sont déclarés en
  minuscules dans `.github/labels.yml` et GitHub rend le nom tel qu'il est
  déclaré. Les deux lecteurs existants vivent sur cette prémisse depuis
  mika#2263.

Si la normalisation est un jour voulue, son site est `operator_held_label` — site
unique — et c'est son propre ticket.

## Requirements

### R1 — Le variant : `RefusalReason::OperatorHeld { label }`

Dans `crates/mika-agent/src/milestone_manager/pool.rs`, un troisième variant de
l'`enum` existant — c'est précisément ce que cette forme achetait (le docstring
de `RefusalReason` le dit du vol 3 : « le second motif arrive comme un
**variant**, pas comme une refonte du rapport ») :

```rust
/// Le candidat porte un label par lequel l'opérateur — ou une autre instance —
/// le tient (mika#2923).
///
/// `label` dit **lequel**, pour la raison que porte déjà
/// [`crate::webhook_dispatch::operator_held_label`] : le geste correctif est de
/// retirer *ce* label-là, et un compteur doit pouvoir distinguer « tenu par
/// `blocked` » de « tenu par `operator-gated` ».
OperatorHeld { label: String },
```

Et son libellé, composé au site unique de l'`enum` comme les deux autres :

```rust
RefusalReason::OperatorHeld { label } => {
    format!("tenu par l'opérateur — label `{label}` à retirer")
}
```

**Conséquence de type, assumée : `RefusalReason` perd `Copy`.** Le label est une
`String` parce que `RefusalReason` est `Serialize + Deserialize` (le verdict va
dans le puits hors-ligne du sink) et qu'un `&'static str` ne se désérialise pas.
Le dépôt a déjà ce précédent exact une porte plus loin : `auto_pull::RefusalReason`
est `#[derive(Debug, Clone, PartialEq, Eq)]` **sans `Copy`** et porte des `String`
pour la même raison. Le coût est borné à trois sites, tous nommés au § *Ordre de
commit* :

| site | geste |
|---|---|
| `relief.rs:92` (`motive: r.reason`) | `r.reason.clone()` |
| `pool.rs:1207` (assertion de test sur `v.refusals[0].reason`) | comparaison par référence |
| `RefusalReason::label(self)` | `label(&self)` — les quatre appelants (`reporter.rs:383`, `reporter.rs:438`, `pool.rs:1141`, `relief.rs:364`) compilent inchangés par auto-ref |

### R2 — Le filtre, et sa position dans `well_formed_candidates`

Dans `well_formed_candidates` — site unique du filtre d'admission, partagé par
`assess_pool` et `numbers_needing_size` — le refus s'insère **après** les trois
filtres silencieux (fermé / déjà `ready` / siège `dispatch:`) et **avant** le
refus `NoGroomedPlan` :

```rust
// Premier des refus nommés, et l'ordre des deux est la priorité du motif :
// `AdmissionRefusal` n'en porte qu'un. Un ticket que l'opérateur tient n'a pas
// besoin d'un plan — le geste correctif est de retirer le label — donc le
// nommer `NoGroomedPlan` le ferait entrer dans la relève à grommer
// (`relief::name_relief`) et proposerait un travail que rien ne débloque.
if let Some(held) =
    crate::webhook_dispatch::operator_held_label(s.labels.iter().map(String::as_str))
{
    refusals.push(AdmissionRefusal {
        number: s.number,
        origin,
        reason: RefusalReason::OperatorHeld { label: held.to_string() },
    });
    continue;
}
```

Trois propriétés tombent de cette position, et chacune doit être dite :

- **Un ticket tenu quitte `eligible`**, donc il n'entre jamais dans `admit` quelle
  que soit la bande — c'est l'AC3 par construction, pas par une seconde
  vérification dans le rapporteur.
- **Il quitte aussi `numbers_needing_size`**, donc le cycle cesse de payer un
  appel `gh` pour lire le plan d'un ticket qu'il ne recommandera pas. Effet
  collatéral cohérent, et non une optimisation ajoutée.
- **Après les filtres silencieux, pas avant.** Un ticket déjà `ready` *et* tenu
  est une anomalie réelle, mais elle n'appartient pas à l'admission : poser
  `ready` n'y est plus en jeu, et cette population est déjà la charge de la porte
  du `ready_label_handler` (mika#2263). Un refus nommé ici serait du bruit sur un
  ticket que le bassin ne peut pas corriger.

### R3 — La relève ne propose pas de grommer un ticket tenu

`relief::name_relief` dérive la relève des refus du bassin et « ne refiltre
rien » (sa décision 1). Ce vol ne contredit pas cette décision : il ajoute que
**tout refus n'est pas un motif de grooming**, et il le place au site unique de
l'`enum` pour qu'un variant neuf ne puisse pas y échapper.

```rust
impl RefusalReason {
    /// Ce refus se corrige-t-il par un **grooming** ?
    ///
    /// `match` exhaustif, sans bras `_` : un variant neuf ne compile pas sans
    /// que son auteur ait répondu. C'est ce qui empêche la relève de proposer
    /// un travail que rien ne débloque — « à grommer : tenu par l'opérateur »
    /// serait un faux conseil dans le rapport, et le lecteur agirait dessus.
    pub fn remediable_by_grooming(&self) -> bool {
        match self {
            RefusalReason::NoGroomedPlan => true,
            // Le geste correctif est un découpage chiffré du plan, donc un
            // re-groom : comportement d'aujourd'hui, inchangé.
            RefusalReason::EstimateOverFlightBound { .. } => true,
            RefusalReason::OperatorHeld { .. } => false,
        }
    }
}
```

Dans `name_relief`, le filtre passe **avant** le calcul de `beyond` :

```rust
let mut candidates: Vec<ReliefTicket> = verdict
    .refusals
    .iter()
    .filter(|r| r.reason.remediable_by_grooming())
    .map(|r| ReliefTicket { /* … motive: r.reason.clone() */ })
    .collect();
```

Filtrer après aurait fait compter à `beyond` — « 7 autres candidats », le chiffre
qui dit à l'opérateur si le bassin a du fond — des tickets que la relève ne
nommera jamais. Le docstring du module reçoit une **décision 5** qui porte ce
point ; l'amendement est atomique avec le code, jamais différé.

### R4 — Le rapport : aucune ligne de code

`reporter.rs` rend déjà un refus par ligne avec son motif, par
`RefusalReason::label()` (`reporter.rs:377-385`), et un ticket tenu n'est jamais
dans `pool.admit`. L'AC3 est donc tenu **par la forme déjà en place** : le
livrable de ce requirement est son **test**, pas une modification du rapporteur.
Une ligne de rendu écrite ici serait une seconde orthographe du même fait — ce
que le commentaire de `reporter.rs:430` refuse déjà pour la relève.

Forme attendue de la ligne, telle que le test l'épingle :

```
- Refusé : #2698 (p1-important) — tenu par l'opérateur — label `operator-gated` à retirer
```

### R5 — `MIKA_MANAGER_POOL_WRITE` : zéro ligne touchée (AC4)

Aucun fichier de ce vol ne lit l'interrupteur. Trois propriétés existantes le
tiennent, et le vol n'en relâche aucune :

- `pool_writer::pool_write_is_armed` reste **le seul lecteur** de
  `ENV_POOL_WRITE` du dépôt (son docstring le dit, `pool_writer.rs:87`), et
  `pool_writer.rs` n'est pas dans le périmètre.
- `no_dispatch_test.rs` balaie tout `milestone_manager/**` pour les jetons
  d'écriture ; `pool.rs` et `relief.rs` ne reçoivent aucune exception et n'en
  demandent aucune — les deux restent des fonctions **pures**.
- `cadence.rs:2686` tient déjà « aucune écriture `gh` avec
  `MIKA_MANAGER_POOL_WRITE` fermé », avec son contrôle négatif armé.

L'AC4 reçoit quand même **son propre test**, parce que « rien n'a changé » doit
être une assertion et pas une déclaration : voir V5.

## Non-Requirements (hors de ce vol, délibérément)

1. **Normaliser la comparaison de labels** (trim / casse) — refus argumenté
   ci-dessus, § *Un refus argumenté*. Son site serait `operator_held_label`, qui
   a deux autres lecteurs hors périmètre.
2. **Un scan de source « aucun littéral de label retapé sous `milestone_manager/` »**.
   Tentant, et non livré : la propriété est tenue par construction (le vol
   n'écrit aucun littéral, il appelle `operator_held_label`), V3 la tient par
   itération sur la constante elle-même, et le scan coûterait ~70 lignes qui
   feraient passer le total au-dessus de la borne par vol. Si un littéral
   réapparaît un jour sous ce sous-arbre, c'est un ticket de garde structurelle,
   pas une extension de celui-ci.
3. **Le cas « déjà `ready` *et* tenu »** — population de la porte du
   `ready_label_handler` (mika#2263), pas de l'admission. Voir R2.
4. **Ouvrir `MIKA_MANAGER_POOL_WRITE`.** Ce vol est la précondition de cette
   ouverture, jamais l'ouverture.

## Verification Contract

| id | ce qui est vérifié | où | AC |
|---|---|---|---|
| **V1** | Le corpus du rapport du 2026-10-26 : `#2698`, `#2699` (milestone, p1) et `#1806` (hors milestone, p1) tous trois `operator-gated` rendent trois `AdmissionRefusal::OperatorHeld` nommés, **et** un quatrième candidat sans label gaté, groomé, est dans `admit`. Bande *famine* pour que `admit` soit non vide. Contrôle positif et négatif dans **le même** `#[test]`. | `pool.rs § tests` — `mika2923_le_corpus_du_2026_10_26_refuse_les_trois_gates_et_admet_le_non_gate` | AC1 |
| **V2** | `operator-gated` **seul** refuse ; `blocked` **seul** refuse. Deux `#[test]` distincts, chacun avec son candidat jumeau non labellisé comme contrôle, pour que neutraliser un terme du prédicat ne puisse pas laisser l'autre vert. | `pool.rs § tests` — `mika2923_operator_gated_seul_suffit_a_refuser`, `mika2923_blocked_seul_suffit_a_refuser` | AC2 |
| **V3** | **Chaque** membre de `webhook_dispatch::OPERATOR_HELD_LABELS` refuse à l'admission — itération sur la constante, pas sur une liste recopiée — donc `operator-review` est couvert aujourd'hui et un membre ajouté demain l'est le jour de son arrivée. Porte l'assertion de non-vacuité (`!OPERATOR_HELD_LABELS.is_empty()`) sans laquelle la boucle garderait zéro chose. | `pool.rs § tests` — `mika2923_chaque_label_tenu_refuse_a_ladmission` | AC2 |
| **V4** | Un refus `OperatorHeld` n'apparaît **pas** dans `ReliefVerdict::next` et n'est pas compté dans `beyond` ; son contrôle positif dans le même test — un `NoGroomedPlan` du même verdict y est, lui, nommé. | `relief.rs § tests` — `mika2923_un_ticket_tenu_nentre_pas_dans_la_releve_a_grommer` | R3 |
| **V5** | Le rapport du sink porte une ligne `- Refusé : #<n>… — tenu par l'opérateur — label …` par ticket gaté (trois lignes), et **aucune** occurrence de `2698`, `2699`, `1806` sur la ligne `- À admettre :`. Le test lit le rapport rendu, pas le verdict. | `reporter.rs § tests` — `mika2923_le_rapport_nomme_chaque_refus_de_porte_et_nen_admet_aucun` | AC3 |
| **V6** | `ENV_POOL_WRITE == "MIKA_MANAGER_POOL_WRITE"` et `pool_write_is_armed()` rend `false` sous variable absente — épinglage par valeur, dans le même commit que le filtre, pour que « lu exactement comme avant » soit une assertion. | `pool.rs § tests` — `mika2923_linterrupteur_decriture_reste_lu_comme_avant` | AC4 |
| **V7** | `cargo test -p mika-agent milestone_manager`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main` | CI | — |

Les fixtures de V1 portent les **vrais numéros** du rapport du 2026-10-26
(`#2698`, `#2699`, `#1806`) et leurs vrais labels. Un corpus inventé testerait le
prédicat sans attester la cause du ticket.

## Fire-Disposition

Ce vol livre un détecteur : **V3**, dont le chemin de succès est « aucun membre de
`OPERATOR_HELD_LABELS` n'échappe au refus d'admission ». Les autres livrables
(V1, V2, V4, V5, V6) sont des assertions de comportement sur une fonctionnalité
neuve et atterrissent **armées, sans exception**.

**Option retenue : (a) exception nommée en allowlist — livrée vide.**

```rust
/// Membres de `OPERATOR_HELD_LABELS` qui, délibérément, **n'excluent pas** un
/// candidat du bassin. **Livrée vide, et c'est l'état nominal** : les trois
/// membres d'aujourd'hui tiennent tous le ticket au même titre (mika#2263).
///
/// Une entrée future doit nommer le label, dire pourquoi l'admission l'ignore,
/// et référencer son ticket de suivi — jamais un nom seul.
const EXCEPTIONS_ADMISSION: &[(&str, &str)] = &[];
```

Et son assertion auto-nettoyante, dans le même test :

```rust
for (label, _raison) in EXCEPTIONS_ADMISSION {
    assert!(
        OPERATOR_HELD_LABELS.contains(label),
        "exception stale : `{label}` n'est plus dans OPERATOR_HELD_LABELS — \
         retire l'entrée, l'exception ne protège plus rien"
    );
}
```

Elle rougit dès qu'une exception survit au label qu'elle exemptait, ce qui est la
façon dont une allowlist meurt sans bruit. Aucune violation existante n'est
exemptée : le détecteur atterrit vert sur les trois membres.

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2923 :

- **AC1.** Le corpus du rapport du 2026-10-26 (les trois candidats ci-dessus et
  au moins un candidat sans label gaté) rend : les trois gatés en refus nommés,
  le non-gaté admis. Le contrôle positif et le contrôle négatif sont dans le même
  test.
- **AC2, terme par terme.** `operator-gated` seul et `blocked` seul suffisent
  chacun à refuser, chacun avec son test : neutraliser l'un ne doit pas laisser
  l'autre vert par accident.
- **AC3.** Le rapport du sink porte une ligne de refus par ticket gaté, avec la
  raison. Aucun ticket gaté n'apparaît dans « À admettre ».
- **AC4.** `MIKA_MANAGER_POOL_WRITE` reste lu exactement comme avant. Aucun label
  n'est posé par ce ticket.

## Definition of Done

1. `RefusalReason::OperatorHeld { label: String }` existe, avec son libellé
   composé au site unique de l'`enum` ; `RefusalReason` est `Clone` sans `Copy`,
   et les trois sites dépendants de R1 sont ajustés.
2. `well_formed_candidates` refuse tout candidat dont
   `webhook_dispatch::operator_held_label` nomme un label, **avant** le refus
   `NoGroomedPlan`, et aucun littéral de label n'est écrit dans
   `milestone_manager/`.
3. `RefusalReason::remediable_by_grooming` existe avec un `match` exhaustif sans
   bras `_`, `name_relief` filtre dessus avant de calculer `beyond`, et la
   décision 5 du docstring de `relief.rs` est écrite dans le même commit.
4. V1 à V6 passent ; V3 atterrit vert avec `EXCEPTIONS_ADMISSION` vide.
5. `reporter.rs` ne reçoit **aucune** ligne de code — seulement son test.
6. `pool_writer.rs`, `cadence.rs` et `no_dispatch_test.rs` sont **inchangés** ;
   `git diff --stat` le montre.
7. V7 vert : suite `mika-agent`, clippy `-D warnings`, `fmt --check`,
   `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`.
8. Le docstring du module `pool.rs` nomme le troisième motif de refus et son
   ticket, à côté des deux existants — un motif que le module ne déclare pas se
   lit comme un effet de bord.

## Ordre de commit

Un seul commit de travail, parce que le retrait de `Copy` et le variant qui le
cause ne compilent pas séparément. Les gestes, dans l'ordre où ils sont écrits :

1. `pool.rs` — variant `OperatorHeld`, bras de `label()`,
   `remediable_by_grooming()`, retrait de `Copy`, filtre dans
   `well_formed_candidates`, amendement du docstring du module (DoD 8).
2. `relief.rs` — `.filter(…)` avant `beyond`, `r.reason.clone()`, décision 5 du
   docstring.
3. `pool.rs:1207` et les assertions de test rendues par le retrait de `Copy`.
4. Les tests V1 à V6.

**Plancher de ce vol** : l'étape 1 seule ferme AC2 et AC4 ; sans l'étape 2 le
rapport proposerait de grommer un ticket tenu, donc elle n'est pas optionnelle.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/pool.rs` (variant, libellé, prédicat de grooming, filtre, docstrings, retrait de `Copy`) | 100 |
| `crates/mika-agent/src/milestone_manager/relief.rs` (filtre + décision 5 + `clone`) | 30 |
| ajustements rendus par le retrait de `Copy` (3 sites) | 10 |
| tests `pool.rs` (V1, V2 ×2, V3 + allowlist, V6) | 150 |
| tests `relief.rs` (V4 + contrôle positif) | 50 |
| tests `reporter.rs` (V5) | 55 |

Total estimé : 395 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** Le périmètre est un variant ajouté à
un `enum` qui existe pour recevoir des variants, plus un `if let … continue` dans
une fonction **pure** déjà écrite, plus un `.filter` d'une ligne. Aucun module
neuf, aucun appel `gh`, aucune migration de schéma, aucun interrupteur, aucune
ligne dans le rapporteur, aucun fichier de skill ni de CI. Les deux tiers du
total sont des tests, qui ne portent ni refactor ni plomberie : six `#[test]`
dans trois `mod tests` existants, avec des `fn` d'aide (`ready`, `candidate`)
déjà en place. La marge sous la borne est de 34 lignes, et le § *Non-Requirements*
nomme le livrable (le scan de source, ~70 lignes) qui l'aurait dépassée —
délibérément non livré plutôt que livré à moitié.
