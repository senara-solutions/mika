---
module: mika-agent/auto_pull
date: 2026-11-04
problem_type: logic_error
component: agent
severity: high
category: logic-errors
ticket: mika#3036
symptoms:
  - "`auto_pull_staleness_measured` à `branch_deleted_by_merge`, `outcome: promote`, puis `auto_feeder_promoted` sur le même ticket dans les 2 minutes, alors que le merge venait de retirer `ready`"
  - "un groom plan-only lancé que personne n'a demandé (#3030 via PR #3032, 06:20:07Z)"
  - "une seconde occurrence arrêtée seulement par la présence d'un pilote vivant (`live_pilot`, #2194 via PR #3033, 08:00:16Z)"
  - "`auto_pull_callout_rewrite` sous le jeton de l'App sur un ticket que le moteur n'aurait pas dû toucher"
root_cause: logic_error
resolution_type: code_fix
tags:
  - auto-pull
  - feeder
  - consentement
  - ready-label
  - phase
  - wire-format
  - exhaustive-match
related:
  - docs/solutions/workflow-issues/verdict-writer-and-gate-must-share-one-vocabulary-2026-08-27.md
  - docs/solutions/logic-errors/un-reglage-a-zero-qui-ne-desarme-quune-moitie-de-la-phase.md
---

# Rendre un consentement et s'en donner un sont deux gestes, et la phase est ce qui les sépare

## Problem

`classify_promotion` (`crates/mika-agent/src/auto_pull.rs`) rendait `Promote`
sur une mesure `BranchDeletedByMerge` **sans regarder la phase**. La règle
venait de mika#2837 et elle était juste pour ce qu'elle visait : en phase 1 et
en phase 2, le ticket porte déjà `ready`, et une branche disparue parce que sa
PR a été mergée ne doit pas bloquer le vol suivant d'un ticket à plusieurs
vols.

Depuis mika#3022, `merged_pr_ready_cleanup_handler` **retire** `ready` au
merge. Le feeder de phase 0, qui promeut précisément les tickets qui ne portent
pas `ready`, a donc lu la trace du merge comme une autorisation : il s'est
redonné le consentement que le merge venait de rendre. La même mesure, la même
ligne de code, deux sens opposés selon ce que le ticket porte au moment où on
la lit.

## Symptoms

Mesuré deux fois le 2026-11-04, sur 62 mesures `branch_deleted_by_merge`
depuis le 2026-10-20 — les 60 autres étaient des passages de phase 1 ou 2 sur
un ticket qui portait déjà `ready`, donc nominales :

| ticket | PR mergée | `ready` retiré | mesure | promotion |
|---|---|---|---|---|
| #3030 | #3032, 06:12:57Z | 06:13:02Z | 06:20:02Z, `outcome: promote` | 06:20:07Z, `feeder_rank=4` |
| #2194 | #3033, 07:58:23Z | oui | 08:00:05Z | 08:00:14Z, `feeder_rank=0` |

Le second n'a pas abouti, et pas grâce à une garde de consentement : un pilote
vivait encore sur le ticket. Dans les deux cas le feeder a aussi réécrit le
callout du corps sous le jeton de l'App.

## What Didn't Work

Deux réflexes se présentent d'abord, et tous deux déplacent le défaut au lieu
de le traiter.

**Ne pas émettre la mesure en phase 0.** La sonde de merge est un fait sur
l'origine — la branche a disparu parce qu'une PR l'a mergée. Le taire en phase
0 enlève l'information à tout le monde, y compris au ledger, et laisse la
mesure retomber sur `branch_absent_on_origin`, qui ne dit pas la même chose.

**Lire la sonde une seconde fois chez l'appelant du feeder.** C'est ce que
AC2 interdit nommément : deux lecteurs de la même sonde, c'est deux endroits
où la quatrième phase sera oubliée. La décision se prend dans un seul `match`,
sans bras `_`, à `auto_pull.rs:1439` — une phase ajoutée ne compile pas tant
qu'on n'a pas dit ici ce qu'elle fait.

## Solution

La phase entre dans la signature de la décision, et la décision gagne une
troisième issue qui n'est pas un reproche :

```rust
StalenessMeasurement::BranchDeletedByMerge { pr } => {
    return match phase {
        ExclusionPhase::Phase0Feeder => {
            PromotionGate::Decline(DeclineReason::ConsentReturnedByMerge { pr: *pr })
        }
        ExclusionPhase::Phase1Pull | ExclusionPhase::Phase2StuckReady => {
            PromotionGate::Promote { detail: "branch_deleted_by_merged_pr" }
        }
    };
}
```

`Decline` se distingue de `Refuse` par une seule question, posée à
`auto_pull.rs:1184` : `authorizes_body_rewrite()`. Un refus reproche quelque
chose au ticket et le callout du corps est le lieu où on le lui dit ; un déclin
ne lui reproche rien — le ticket est à jour, c'est le moteur qui n'a pas de
consentement. La réécriture du callout, jusque-là inline dans
`promotion_gate_allows`, est devenue `maybe_rewrite_stale_callout`
(`auto_pull.rs:3959`), qui sort d'emblée quand la décision ne l'autorise pas.
Un seul site d'écriture, une seule question.

Les trois `ledger.record(…, FILTER_PROMOTION_GATE)` n'ont **pas** été touchés :
c'est l'absence de geste qui satisfait AC4. Le déclin compte déjà dans le
ledger d'exclusion de sa phase, parce que `promotion_gate_allows` rend `false`
et que le ledger est en aval du booléen, pas de la raison.

## Why This Works

### Une projection de format de fil s'ajoute, elle ne se fusionne pas

`ExclusionPhase` portait déjà `as_str()` : `"phase0_feeder"`,
`"phase1_pull"`, `"phase2_stuck_ready"` — le vocabulaire du **ledger
d'exclusion**. La porte de promotion, elle, écrit depuis mika#2470
`"phase0_feeder"`, `"phase1_idle_pull"`, `"phase2_stuck_rescue"` dans
`auto_pull_staleness_measured.phase`. Deux vocabulaires, qui coïncident sur
une valeur des trois.

La tentation est de n'en garder qu'un. C'est exactement ce que mika#2205
interdit : une chaîne persistée dans `audit_events` et groupée par un
opérateur ne se renomme jamais, parce que deux orthographes d'une même
population ne s'additionnent pas. Un `GROUP BY phase` sur six mois de données
rendrait `phase1_pull` et `phase1_idle_pull` comme deux lignes, et aucun
lecteur n'aurait de raison de les sommer. D'où `gate_phase()`
(`auto_pull.rs:1805`) **à côté** de `as_str()` (`auto_pull.rs:1790`), et non à
sa place : deux projections d'un même type, non interchangeables, chacune
nommant le fil qu'elle sert.

`"decline"` suit la même règle dans l'autre sens. Ce n'est pas un
renommage de `"refuse"` : c'est une **valeur nouvelle** du vocabulaire
`outcome`. Les lignes `refuse` d'avant le vol restent vraies et restent
comparables ; une requête qui compte les refus ne compte pas les déclins, et
c'est précisément ce qu'on veut lire.

### Le témoin, parce que l'absence et le silence se ressemblent

Le test d'AC3 (`mika3036_un_declin_ne_touche_pas_le_corps_du_ticket`) assure
qu'un passage déclinant n'écrit aucun `auto_pull_callout_rewrite`. Prise
seule, cette assertion passe aussi si rien n'a tourné — c'est la leçon de
mika#2340. Elle porte donc, dans le même appel, un ticket **témoin** qui
promeut et dont on exige exactement une ligne de réécriture et exactement un
`gh issue edit`.

Et la réponse de `gh issue view` est cannée **des deux côtés** : le déclin
avait l'écriture à portée et ne l'a pas prise. Prouver l'abstention par
l'absence d'une réponse cannée, c'est prouver l'absence de la réponse.

## Prevention

- **S1, greffé sur `test_promotion_gate_never_resolves_conflicts`** : un scan
  de source (`ProductionScanner`, qui coupe le fichier à son premier
  `#[cfg(test)]`) compte exactement **un** `match phase {` et exactement
  **trois** `ledger.record(…FILTER_PROMOTION_GATE)`. Le second lecteur
  qu'AC2 interdit fait rougir le scan ; un quatrième ledger aussi. La table
  d'exceptions est `[&str; 0] = []` (mika#1574) : toute entrée doit compiler,
  donc toute entrée doit se justifier.
- **Le `match` sans bras `_`** est la vraie garde : la quatrième phase ne
  compilera pas sans qu'on dise, à ce seul endroit, si elle se donne un
  consentement ou si elle en a déjà un.
- **La question à se poser devant une règle de phase** : cette mesure
  signifie-t-elle la même chose selon que le ticket porte déjà le
  consentement ou non ? Si la réponse est non, la phase appartient à la
  signature de la décision, pas à son appelant.

## Ce qui reste ouvert

- Une repose de `ready` par un humain après le merge est un **nouveau**
  consentement, et reste servie par les phases 1 et 2. Le déclin ne porte que
  sur le consentement que le merge vient de rendre.
- Le secours `no-shipping-tail` de dispatch-lib, qui rouvre une PR déjà
  mergée, est suivi par #3034 — même famille de symptômes, autre site.
