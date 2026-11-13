---
title: Une mesure en `contains` et un prédicat ancré ne comptent pas la même population
date: 2026-10-29
last_updated: 2026-10-30
category: best-practices
module: mika-agent/evidence/guards
problem_type: best_practice
component: dev-loop
severity: high
applies_when:
  - Un ticket chiffre une population de verdicts, de logs ou de corps de revue avec un `contains`
  - On s'apprête à écrire un garde dont le prédicat est « ce corps porte-t-il telle section ? »
  - La mesure qui justifie un vol et le prédicat qui le livre sont écrits par deux mains
  - Une exception mesurée sert de contrôle positif à un test
tags:
  - guards
  - measurement
  - qa-review
  - verification
  - fail-open
related:
  - mika#2961
  - mika#2687
  - mika#2455
  - mika#2205
  - mika#2158
---

# Une mesure en `contains` et un prédicat ancré ne comptent pas la même population

## Le défaut

mika#2961 chiffrait le régime des verdicts `pass` de `mika-platform-qa` qui ne
portent pas la section `PIPELINE:` que leur prompt exige. Sa mesure filtrait sur
`contains("PIPELINE:")` et trouvait **4 verdicts conformes sur 426**, tous sur
mika#2691, le 2026-10-14.

Le prédicat du moteur, lui, est **ancré en début de ligne** :
`pipeline_section_present` cherche une ligne qui *commence* par `PIPELINE:`
(blancs de tête et course d'emphase markdown tolérés, casse repliée). Refait avec
cet ancrage, le relevé donne **0 sur 261** sur une fenêtre plus récente, et sur
les quatre verdicts de #2691 :

```
2026-10-14T14:00:51Z  pass  contains=true  lignes_ancrées=0
2026-10-14T15:05:35Z  pass  contains=true  lignes_ancrées=0
2026-10-14T15:20:47Z  pass  contains=true  lignes_ancrées=0
2026-10-14T16:25:27Z  pass  contains=true  lignes_ancrées=0
```

Le mot y est dans la **prose** : cette PR implémentait mika#2687 et son résumé
parle de *« a `PIPELINE:` section citing a failing guard »*. Le régime réel était
donc **426 sur 426**, pas 422 sur 426 : aucun verdict de la population mesurée ne
portait la section. L'écart entre les deux mesures ne portait pas sur du bruit —
il portait **exactement** sur les quatre cas que le ticket citait comme preuve
que la règle était tenable.

## Pourquoi c'est plus grave qu'une mesure imprécise

Trois conséquences, et aucune n'est cosmétique.

**1. L'écart est du côté fail-open.** Un `contains` *surestime* la conformité. Un
garde écrit sur ce prédicat compterait la prose d'une revue comme sa preuve — un
relecteur qui *parle* de la section passerait le garde sans l'avoir produite.
C'est le fail-open même que le ticket condamnait, réintroduit par le prédicat
censé le fermer.

**2. Le contrôle positif demandé n'existait pas.** L'AC2 du ticket exigeait
qu'« une revue de mika#2691 qui porte `PIPELINE:` passe ». Sous le prédicat réel,
**cette revue n'existe pas** : il n'y a dans la population aucun corps qui
passerait. Écrit naïvement, ce test aurait été vert sous un prédicat en
`contains` — c'est-à-dire vert sous le défaut — et rouge sous le bon. Un contrôle
positif tiré d'une mesure faite avec le mauvais prédicat **valide le mauvais
prédicat**.

**3. Le dimensionnement du risque change.** « 422 sur 426 » et « 426 sur 426 »
n'appellent pas la même décision : la première laisse croire à un régime
majoritairement conforme avec quatre accidents, la seconde dit que la règle n'a
**jamais** été tenue et qu'armer le garde d'emblée changerait le régime de toute
la population d'un coup. C'est ce chiffre-là qui a imposé l'interrupteur désarmé
par défaut.

## La règle

**Mesurez avec le prédicat que le garde va exécuter, pas avec son approximation
en une ligne.** Si le garde est ancré, la requête de mesure est ancrée. Sur `jq`,
cela coûte une ligne :

```sh
# FAUX — compte la prose
--jq '[.[] | .reviews[] | select(.body|contains("PIPELINE:"))] | length'

# JUSTE — compte les lignes ancrées, casse repliée, emphase markdown tolérée
--jq '[.body|split("\n")|.[]
       |select(test("^[ \t*_]*[Pp][Ii][Pp][Ee][Ll][Ii][Nn][Ee]:"))]|length'
```

Et quand le ticket est déjà chiffré par une autre main : **refaites la mesure au
groom**, avant d'écrire le plan. Elle est bon marché, et elle est la seule chose
qui dit si le contrôle positif que les critères d'acceptation demandent existe.

## Ce qui tient la doctrine — un test, pas un commentaire

Un paragraphe de doc expliquant « n'écrivez pas ce prédicat en `contains` » ne
survit pas à la première refactorisation. Ce qui le tient est un test qui assère
les **deux faits sur le même corps mesuré** :

```rust
// guards.rs, mod tests::mika2961
#[test]
fn mika2961_le_corps_de_2691_contient_le_mot_et_na_pas_la_section() {
    assert!(MEASURED_2691.contains(PIPELINE_SECTION_LINE_PREFIX));   // la prose
    assert!(!pipeline_section_present(MEASURED_2691));               // pas la section
    assert_eq!(
        classify_pipeline_block_grounding(NOT_A_BLOCK, IS_A_PASS, MEASURED_2691, GUARD_RAN),
        PipelineGroundingOutcome::RefusedPassSectionAbsent
    );
}
```

Ce test a trois propriétés qu'aucun commentaire n'a. Il **interdit
structurellement** de réécrire le prédicat en `contains` : la première assertion
resterait verte, la troisième rougirait. Il remplace le contrôle positif que
l'AC2 demandait et qui n'existe pas dans la population. Et il rend le fixture
**auto-documenté** : le corps est celui de la revue réelle, et la doc du fixture
porte la mesure du corps entier (39 lignes, 0 ancrée, `contains` vrai), pour que
la troncature du fixture ne puisse jamais être relue comme la cause du refus.

## Reproduit à l'identique sur un second jeton (2026-10-30)

Le vol 2/2 de mika#2961 porte le même garde sur la ligne `NEGATIVE-TEST:`. La
mesure a été **refaite**, et non déduite de celle du vol 1 : 60 dernières PR par
dépôt, prédicat ancré, casse repliée.

| population | `pass` | ligne ancrée | `contains` |
|---|---|---|---|
| 2026-10-29, jeton `PIPELINE` | 261 | 0 | 4 |
| 2026-10-30, jeton `NEGATIVE-TEST` | 260 | 0 | 1 |

L'unique `contains` du second relevé est la revue de **la PR du vol 1
elle-même** : elle cite le jeton dans une case de critère d'acceptation, parce que
le vol 1 avait reporté ce terme par écrit. La forme du faux positif est donc
identique à celle des quatre de #2691 — *une revue qui parle du jeton parce que la
PR qu'elle relit l'implémente* — ce qui en fait moins une coïncidence qu'une
**propriété de la population** : sur un dépôt qui s'auto-développe, les corps de
revue les plus susceptibles de contenir le mot sont ceux des PR qui installent le
garde. Un `contains` est donc structurellement biaisé vers le fail-open exactement
là où la mesure sert à décider.

Le coût de refaire la mesure : une requête, deux minutes. Le coût de l'avoir
déduite : le même contrôle positif faux que le vol 1 avait failli écrire, sur un
jeton où l'écart valait 1 sur 260 — assez petit pour qu'un relevé « ≈ 0, comme le
vol 1 » soit passé sans objection.

## Ce que ça ne dit pas

- **`contains` n'est pas faux en soi** : il est faux comme *estimateur d'un
  prédicat ancré*. Pour une mesure exploratoire — « ce mot apparaît-il quelque
  part ? » — il répond exactement à sa question.
- **Le sens de l'écart n'est pas toujours fail-open.** Ici le `contains`
  surestime la conformité ; un prédicat plus large que la mesure surestimerait
  les refus, ce qui est l'autre côté de l'arithmétique (coût R8 : un faux refus
  tue le tour sans aucune revue postée). Ce qui compte est de **savoir de quel
  côté** l'écart tombe avant de décider du défaut de l'interrupteur.
- **Un fixture tronqué reste un fixture tronqué.** La parade n'est pas de garder
  le corps entier, c'est d'écrire la mesure du corps entier dans la doc du
  fixture — sinon une tête de 10 lignes qui ne porte pas la section prouve
  seulement que la tête ne la porte pas.

## Références

- Vols : mika#2961 vol 1/2 (section `PIPELINE:`) et vol 2/2 (ligne
  `NEGATIVE-TEST:`), tous deux dans `crates/mika-agent/src/evidence/guards.rs`,
  `mod mika2961`. Le second a ajouté un terme au même garde : l'ordre dans lequel
  les deux termes sont lus décide quel motif un corps porte —
  `une-conjonction-de-deux-termes-dans-un-garde-se-lit-dans-un-ordre-2026-10-30.md`.
- Famille du garde : mika#2687 (membre `block[pipeline]`), mika#2455
  (l'abstention CI et son arithmétique fail-open).
- Classe « une garde que rien n'exerce se lit comme une garde qui marche » :
  mika#2205. Classe « le vert d'un test de comportement ne voit pas un câblage
  mort » : mika#2158.
