---
title: Une conjonction de deux termes dans un garde se lit dans un ordre, et cet ordre est une décision de produit
date: 2026-10-30
last_updated: 2026-10-30
category: best-practices
module: mika-agent/evidence/guards
problem_type: best_practice
component: dev-loop
severity: medium
applies_when:
  - On ajoute un second terme de refus à un garde qui en portait déjà un, sur la même population
  - Deux conditions indépendantes refusent le même geste sous deux motifs distincts
  - Un plan dit « refuser si A ou B manque » sans dire lequel est évalué d'abord
  - Une sonde opérateur compte les corps qui manquent d'un terme, ou des deux
tags:
  - guards
  - refusal
  - observability
  - measurement
  - qa-review
related:
  - mika#2961
  - mika#2687
  - mika#2131
  - mika#2205
---

# Une conjonction de deux termes dans un garde se lit dans un ordre

## Le contexte

mika#2961 refuse, avant tout sous-processus, un `pr review --approve` dont le
corps porte `VERDICT: pass` sans la preuve que le prompt de qa-review exige. Le
vol 1 a livré un terme — aucune ligne `PIPELINE:` ancrée. Le vol 2 en ajoute un
second dans **le même** garde — aucune ligne `NEGATIVE-TEST:` ancrée — avec son
propre motif (`pass_negative_test_absent` à côté de `pass_section_absent`), parce
que deux populations sous un seul nom ne sont pas soustractables (mika#2131).

Le plan disait « refuser si l'un des deux manque ». Écrite telle quelle, cette
phrase ne décide rien : elle admet deux implémentations, et elles ne produisent
pas les mêmes chiffres.

## Ce que l'ordre décide

Un corps qui manque des **deux** termes — le régime mesuré, 260 `pass` sur 260 —
peut sortir sous l'un ou l'autre motif. L'ordre choisi est **section d'abord,
ligne ensuite**. Trois conséquences, toutes observables, aucune cosmétique.

**1. La non-régression du vol précédent est acquise, ou elle est à payer.** Les
trois corps mesurés du vol 1 (`control-monitor#217`, mika#2960, mika#2691)
manquent des deux termes. Avec la section évaluée d'abord, ils restent classés
`RefusedPassSectionAbsent` : **zéro édition de fixture, zéro test réécrit**. Avec
l'ordre inverse, les trois fixtures changent de classe, et le vol 2 devient une
régression du vol 1 à ré-attester — pour un gain nul, puisque les deux motifs
refusent le même geste.

**2. Le second motif ne devient visible qu'une fois le premier acquis.** Sous cet
ordre, `pass_negative_test_absent` ne se lit **que** sur une revue déjà à moitié
conforme. Sa courbe monte donc avant de descendre, et cette montée mesure le
progrès du premier terme. Une table de lecture opérateur qui n'écrit pas cela
invite au contresens exact : « le second motif augmente » lu comme une
dégradation, alors que c'est le signe que les revues commencent à porter leur
section.

**3. Les deux sondes ne se somment pas.** `sans_pipeline` et
`sans_negative_test`, comptés indépendamment, se recouvrent sur `sans_les_deux` :

```sh
# Les trois chiffres, car deux ne suffisent pas à en déduire le troisième
--jq '{pass:length,
       sans_pipeline:([.[]|select(.pipe==0)]|length),
       sans_negative_test:([.[]|select(.neg==0)]|length),
       sans_les_deux:([.[]|select(.pipe==0 and .neg==0)]|length)}'
```

Sur la population du 2026-10-30 les trois valent 260 : l'intersection *est* la
population, donc la somme des deux premiers surestime du double. Publier deux
compteurs d'une conjonction sans leur intersection, c'est publier une addition
que personne ne peut refuser de faire.

## La règle

**Quand un garde gagne un second terme sur la même population : écrire l'ordre,
le justifier par la non-régression, et l'épingler par un test.** Les deux
premières tiennent dans un commentaire de trois lignes au site de la conjonction.
La troisième est la seule qui survive :

```rust
if verdict_is_pass {
    // La conjonction ordonnée : la section d'abord, la ligne ensuite.
    // L'ordre est load-bearing — il garde les trois corps mesurés du vol 1
    // dans leur classe d'origine, à zéro édition de fixture.
    if !pipeline_section_present(body) {
        return PipelineGroundingOutcome::RefusedPassSectionAbsent;
    }
    if !negative_test_line_present(body) {
        return PipelineGroundingOutcome::RefusedPassNegativeTestAbsent;
    }
    return PipelineGroundingOutcome::Allowed;
}
```

Le test n'est pas un cas de plus, c'est une **table** : chaque corps, l'état des
deux prédicats, et la sortie attendue.

```rust
// Les quatre cases, dont une que la population ne contient pas
for (label, body, section, ligne, attendu) in [
    ("ni l'un ni l'autre", MEASURED_CM217,            false, false, RefusedPassSectionAbsent),
    ("section seule",      STEP_2E_GREEN,             true,  false, RefusedPassNegativeTestAbsent),
    ("ligne seule",        PASS_LIGNE_SANS_SECTION,   false, true,  RefusedPassSectionAbsent),
    ("les deux",           PASS_CONFORME,             true,  true,  Allowed),
] { … }
```

**Le cas pivot est celui qui n'existe pas dans la population.** « Ligne seule »
n'a aucun représentant mesuré : sa seule raison d'être est d'épingler l'ordre, car
c'est la seule des quatre lignes qui change de valeur attendue si quelqu'un
inverse les deux `if`. Un test de table sans cette ligne reste vert sous les deux
ordres — c'est-à-dire qu'il ne teste pas ce qu'il a l'air de tester.

## Ce que ça ne dit pas

- **Ce n'est pas un plaidoyer pour un motif unique.** Deux conditions
  indépendantes gardent deux motifs : fusionner les `reason` rendrait les deux
  populations non soustractables (mika#2131), et le remède à écrire dans le refus
  n'est pas le même — l'un demande une section, l'autre une ligne, et les deux
  remèdes nomment des formes différentes du prompt.
- **L'ordre inverse n'est pas faux en soi.** Il est plus coûteux *ici*, parce que
  l'antériorité du vol 1 a créé des fixtures qui en dépendent. Sur un garde neuf à
  deux termes, l'ordre se choisit sur le motif le plus actionnable pour le
  destinataire du refus, et se documente pour la même raison.
- **La conjonction n'est pas un plafonnement.** Les deux termes refusent ; aucun
  ne réécrit le verdict en `hold[review]`. Le choix refus-contre-plafonnement est
  antérieur et vit dans le plan du vol 1, avec son arithmétique fail-open.

## Références

- Vol : mika#2961 vol 2/2 — `crates/mika-agent/src/evidence/guards.rs`
  (`classify_pipeline_block_grounding`, `mod tests::mika2961`), table d'ordre dans
  `mika2961_la_table_des_deux_termes_epingle_lordre`.
- Mesure et prédicat ancré :
  `une-mesure-en-contains-et-un-predicat-ancre-ne-comptent-pas-la-meme-population-2026-10-29.md`.
- Un nom d'événement par membre, discriminé par `reason` : mika#2131. Contrôle
  positif sur **toutes** les issues, autorisation comprise : mika#2205.
- Surfaces opérateur, tables de lecture et sonde d'ouverture :
  `docs/operator/environment-variables.md` § *Un `pass` dit ce que la garde du
  dépôt a fait*.
