---
title: Trois façons dont une vérification passe au vert sans rien mesurer — et la falsification qui attrape chacune
date: 2026-09-22
category: best-practices
module: crates/mika-agent/src/agent_loop, crates/mika-common/src/llm, docs/plans
problem_type: best_practice
component: testing
severity: high
applies_when:
  - "Écrire ou relire un Verification Contract dans un plan"
  - "Livrer un scan de source structurel, surtout avec une allowlist vide"
  - "Placer un contrôle négatif sur un test existant plutôt que d'en écrire un"
  - "Lire un « tout est vert » sur une garde qu'on vient d'ajouter"
related_components:
  - testing_framework
  - source_guard
tags:
  - faux-vert
  - controle-negatif
  - scan-structurel
  - verification-contract
---

# Trois façons dont une vérification passe au vert sans rien mesurer

mika#2473 livre une garde d'observabilité. Son plan était groomé, validé par
l'architecte en deux passes, puis relu par quatre relecteurs documentaires. Son
code a été écrit en discipline *proof-first*, chaque contrôle négatif vu rouge,
et la suite est passée verte à 1700 tests.

Trois vérifications de cet ensemble étaient **vertes sans rien mesurer**. Les
trois mécanismes sont différents, et c'est ce qui rend le lot intéressant : il
n'y a pas une vigilance à avoir, il y en a trois.

## 1. Le filtre qui ne matche rien — et sort 0

Le contrat de vérification du plan portait :

```bash
cargo test -p mika-agent well_known_agents server:: agent_loop::mika2473
```

Deux défauts, et **le second est silencieux**. `cargo test` n'accepte qu'un seul
`TESTNAME` positionnel : un second est refusé par `error: unexpected argument`.
Bruyant, donc sans danger. Mais une fois les filtres passés derrière `--`,
`agent_loop::mika2473` **ne matche aucun test** — les tests de ce module vivent
dans `#[cfg(test)] mod tests`, donc leur chemin réel est
`agent_loop::tests::mika2473_…`, dont le filtre n'est pas une sous-chaîne.
libtest filtre par sous-chaîne, ne trouve rien, et **sort 0**.

Mesuré, les deux contrôles dans le même appel :

```
agent_loop::mika2342          → 0 passed; 5185 filtered out    exit 0
agent_loop::tests::mika2342   → 2 passed
```

Le critère d'acceptation qui reposait sur cette ligne aurait été signé par une
commande qui n'exécute aucun test.

**La règle :** un `cargo test` qui rapporte `0 passed` est un **échec**, pas un
succès. Lire le compte, jamais le code de sortie. Et dans un dépôt dont les
tests sont inline, un filtre qui nomme un module sans son `tests::` ne matchera
jamais rien.

## 2. Le scan structurel qui ne scanne qu'un fichier

La garde d'AC7 devait établir qu'une primitive n'est appelée **qu'une fois**,
parce que deux sites partageraient une clé de déduplication et se feraient taire
l'un l'autre. Elle était écrite ainsi :

```rust
let production = scanner.production_of(&scanner.src_root().join("agent_loop/mod.rs"));
```

Elle scanne **un fichier**. Un second appel posé dans n'importe quel autre
fichier du crate la laisse verte — c'est-à-dire précisément la violation qu'elle
existe pour interdire.

Son contrôle de bonne foi livré avec elle injectait un second site **dans le même
fichier** : le seul cas que le scan couvrait. Un contrôle qui ne teste que le cas
couvert atteste la moitié rassurante du prédicat.

Démontré plutôt qu'affirmé : un vrai second appel planté dans `init_agent` a
laissé le scan **vert**, et l'a fait rougir une fois le scan élargi à la marche
du crate.

**La règle :** un scan structurel porte sur une **population**, pas sur un
fichier. Nommer sa population, la parcourir entière, et faire porter le contrôle
de bonne foi sur le cas que le scan pourrait **ne pas** couvrir — jamais sur
celui qu'il couvre sûrement.

## 3. Le contrôle négatif posé là où il ne peut pas rougir

Le plan prescrivait un contrôle : *« le test asserte que le record noté est
inchangé après detect »*. L'assertion était correcte. Elle était posée sur le
test du `touch`.

Sur un `touch`, le record re-résolu est identique au record du boot **jusqu'à
`resolved_at`**, qui est estampillé à la seconde et revient donc identique pour
deux résolutions dans la même seconde. Muter le mécanisme pour qu'il écrase la
note de boot laissait le test **vert**.

L'assertion a été déplacée sur le test d'édition de modèle, où la mutation
rougit.

**La règle :** un contrôle négatif n'est pas prouvé par sa présence. Le
neutraliser et le voir rouge est ce qui l'établit — et l'endroit où on le pose
décide s'il *peut* rougir. Deux tests peuvent porter la même assertion et un
seul la mesurer.

## Ce que les trois ont en commun, et ce qui les sépare

Toutes trois sont des instruments qui rapportent « sain » sans avoir regardé —
la classe que ce dépôt poursuit déjà sous plusieurs noms (mika#2205 : *un scan
silencieusement inactif se lit exactement comme un scan qui n'a rien trouvé à
faire* ; mika#2131 ; l'entrée voisine sur les assertions de compte).

Ce qui les sépare est la technique qui les attrape, et aucune des trois ne se
substitue aux deux autres :

| mécanisme | ce qui l'attrape |
|---|---|
| filtre qui ne matche rien | **mesurer** — lire le compte, avec un contrôle positif dans le même appel |
| scan trop étroit | **planter** — introduire une vraie violation et voir si elle est vue |
| contrôle mal placé | **muter** — neutraliser le terme et voir si l'assertion rougit |

Un plan relu par quatre relecteurs et un code écrit proof-first ont produit les
trois. Ce n'est pas un défaut de rigueur : c'est que la rigueur portait sur ce
que les vérifications *disent*, et jamais sur ce qu'elles *touchent*.
