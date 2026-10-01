---
title: Un plancher d'anti-vacuité peut se rembourrer avec le trou qu'il garde
date: 2026-10-01
category: best-practices
module: crates/mika-agent/tests/eval
problem_type: best_practice
component: testing
severity: medium
applies_when:
  - "Écrire une porte d'exhaustivité (« tout X doit être déclaré / câblé / compilé ») et son anti-vacuité"
  - "Poser un plancher de population (dirs_scanned >= N, files_scanned >= N) sur un scan"
  - "Asserter qu'une garde EndTurn a tiré dans un éval MockLlm"
  - "Relire une porte dont le régime attendu est « rien à signaler »"
related_components:
  - testing_framework
  - development_workflow
tags:
  - anti-vacuite
  - porte-exhaustivite
  - faux-vert
  - faux-rouge
  - eval-mock
  - attribution-garde
---

# Un plancher d'anti-vacuité peut se rembourrer avec le trou qu'il garde

## Context

mika#1960 phase 2 (PR #2625) a ajouté une porte à
`crates/mika-agent/tests/eval/test_eval_modules_declared.rs` : tout `.rs` d'un
sous-répertoire de `crates/mika-agent/tests/eval/` portant un `mod.rs` doit y être déclaré, sinon il
n'est jamais compilé. Comme son régime attendu est « vide », la porte portait deux
planchers d'anti-vacuité (`dirs_scanned`, `files_scanned`) pour qu'un renommage ne
la rende pas inerte en silence (classe mika#2205).

La revue (`/ce:code-review`, relecteur adversarial) a trouvé deux défauts dans ces
planchers, et un troisième de même nature dans l'éval qui l'accompagnait.

1. **Le plancher comptait la population que la porte devait refuser.** Un
   sous-répertoire hypothétique `foo/` au `mod.rs` complet mais **absent de `crates/mika-agent/tests/eval.rs`** n'est
   jamais compilé. La porte du premier niveau ne voit pas les répertoires (elle
   filtre `extension == "rs"`), et la nouvelle porte énumérait le répertoire,
   l'ajoutait à `dirs_scanned` et à `files_scanned`, puis vérifiait seulement ses
   fichiers contre **son propre** `mod.rs` — cohérent avec lui-même. Résultat : un
   sous-répertoire entièrement mort **augmentait** les compteurs qui étaient censés
   prouver que la porte regardait quelque chose. Mesuré : avec un répertoire
   synthétique `zz_orphelin_rouge/` (un `mod.rs` + un `scenario_x.rs` contenant un
   `panic!()`), les quatre tests de la porte étaient **verts**.
2. **Le plancher était un compte exact déguisé.** `dirs_scanned >= 8` alors que 8
   sous-répertoires portaient un `mod.rs` à l'écriture, et le commentaire promettait
   des « planchers larges, pas des comptes exacts ». Trois relecteurs indépendants
   l'ont relevé : fusionner légitimement un sous-répertoire rougissait la porte.
3. **Le « la garde a tiré » ne nommait pas la garde.** Le scénario mesuré assertait
   `trace.llm_call_count > 1`, que **n'importe quelle** garde EndTurn satisfait. Une
   garde ajoutée plus tard et appariant le texte du mock aurait gardé le test vert
   avec la garde 5h débranchée.

## Guidance

**Un compteur d'anti-vacuité ne doit compter que des éléments que la porte a
réellement jugés, après les avoir qualifiés comme membres légitimes de la
population.** Si un élément peut être hors population pour une raison que la porte
ne vérifie pas (ici : « le répertoire n'est pas compilé »), il faut soit vérifier
cette raison, soit ne pas le compter. Correctif appliqué : la porte vérifie d'abord
que chaque sous-répertoire est déclaré dans `crates/mika-agent/tests/eval.rs`, avec le même parseur
`declared_modules` que la porte du premier niveau.

```rust
let top_level = declared_modules(EVAL_RS); // eval.rs, épinglé par include_str!
for orphan in undeclared_stems(&top_level, &dir_refs) {
    reports.push(format!("{orphan}/ → absent de tests/eval.rs"));
}
```

**Un plancher porte une présence nommée et une borne lâche, jamais le compte du
jour.** L'élément pour lequel la porte existe est exigé nommément, et la borne
numérique reste loin sous le compte réel :

```rust
assert!(dir_names.iter().any(|d| d == "doctrine_regressions"), "…");
assert!(dirs_scanned >= 4, "…"); // 8 à l'écriture : un plancher, pas un compte
```

**Une assertion « la garde a tiré » s'attache à ce que seule cette garde écrit.**
Dans un éval `EvalHarness` / `MockLlmProvider`, la requête qui suit un re-prompt se
termine sur la correction `[mika-engine]` injectée par la garde. C'est elle qu'on
asserte, via `trace.captured_requests`, pas le nombre d'appels :

```rust
let correction = trace.captured_requests.get(1).unwrap().messages.last().unwrap();
assert!(format!("{:?}", correction.content)
    .contains("Your response proposes to open access"));
```

## Why This Matters

Une anti-vacuité existe pour distinguer « la porte a regardé et n'a rien trouvé »
de « la porte n'a rien regardé ». Si la population hors-contrat gonfle le compteur,
**c'est le défaut lui-même qui satisfait la preuve d'anti-vacuité**. Le faux vert
est alors double : le test qui aurait dû rougir est vert, et la garde censée
détecter qu'il ne regarde rien est verte elle aussi. Aucun test comportemental
ne le voit, parce que rien n'échoue.

Le compte exact déguisé est l'erreur inverse, et elle a son propre coût. Une porte
qui rougit sur une réorganisation légitime finit désarmée ou recalibrée à chaque
fusion, et un plancher qu'on baisse par réflexe ne protège plus rien.

L'attribution compte pour la même raison que l'anti-vacuité : `llm_call_count > 1`
mesure qu'**une** correction a eu lieu, pas laquelle. La note
`mock-eval-tautological-assertion-trap` désigne déjà ce compteur comme la seule
assertion de l'éval qui dépende du moteur. Mais ce compteur ne nomme pas la garde.

## When to Apply

- Toute porte « tout X doit être déclaré/câblé » dont l'énumération et la
  déclaration vivent à deux niveaux différents (fichier vs répertoire, module vs
  crate, label vs table).
- Tout plancher numérique d'anti-vacuité : se demander « un élément invalide
  peut-il faire monter ce compteur ? » et « ce nombre est-il le compte d'aujourd'hui ? ».
- Tout éval qui prouve qu'une garde tire alors que plusieurs gardes partagent le
  même chemin (`intent_guard_retries`, chaîne EndTurn).

## Examples

**Le contrôle qui l'a prouvé** est un orphelin réel posé sur disque, pas seulement
synthétique en mémoire. Le contrôle négatif synthétique existant
(`la_porte_des_sous_repertoires_voit_un_orphelin_synthetique`) exerçait la décision
*par fichier* et ne pouvait pas voir le défaut *par répertoire* :

| état | porte des sous-répertoires |
|---|---|
| avant correctif + `zz_orphelin_rouge/` non déclaré | **verte** (le défaut) |
| après correctif + `zz_orphelin_rouge/` | rouge, « `zz_orphelin_rouge/ → absent de tests/eval.rs` » |
| après correctif, orphelin retiré | verte |

Le contrôle synthétique couvre désormais aussi le niveau répertoire. Pour
l'attribution, le contrôle a été une **mutation** : le libellé de la correction
de 5h a été modifié dans `crates/mika-agent/src/agent_loop/mod.rs`. L'assertion rougit alors en citant
le dernier message, et redevient verte une fois le libellé restauré.

## Related

- `une-case-manquante-dans-une-taxonomie-force-a-encoder-un-mensonge-2026-09-30.md`
  (mika#1984) — le constat fondateur : la porte du premier niveau ne descend pas
  dans les sous-répertoires. Cette note est sa suite : l'extension a été faite par
  mika#1960 phase 2, et ce sont ses planchers qui se sont révélés rembourrés.
- `trois-facons-dont-une-verification-passe-au-vert-sans-rien-mesurer-2026-09-22.md`
  — la famille ; ceci en est une quatrième façon : *le plancher compte le trou*.
- `une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md`
  — la couche « population vide » ; ceci est le cas inverse, une population
  **gonflée par ses propres invalides**.
- `un-cfg-test-ditem-aveugle-un-scan-qui-tronque-au-premier-litteral-2026-09-29.md`
  — y préfère `assert_eq!(n, 1)` à `n <= 1`. Pas de contradiction : là, on compte
  des **sites** dont la cardinalité est une propriété du contrat ; ici, on borne la
  **taille d'une population scannée**, qui varie légitimement avec l'arbre.
- `mock-eval-tautological-assertion-trap-2026-04-29.md` — y fait de
  `llm_call_count > 1` la règle des tests « la garde tire » ; ce compte est
  nécessaire, mais il n'attribue pas le re-prompt à une garde.
