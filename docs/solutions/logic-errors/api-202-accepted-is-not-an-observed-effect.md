---
title: "Un 202 Accepted n'est pas un effet observé : « accepté » vient du code retour, « observé » vient d'une relecture"
date: 2026-09-30
category: logic-errors
module: crates/mika-agent/src/tools/pr_merge_with_gate.rs
problem_type: logic_error
component: tooling
severity: high
symptoms:
  - "update-branch rend exit 0 (HTTP 202) et la boucle rend BranchUpdated { new_main_sha } alors qu'aucun commit n'a atterri"
  - "new_main_sha publié au moniteur et au LLM vaut le SHA VISÉ (info.current_main_sha), jamais lu sur la PR"
  - "le job asynchrone GitHub échoue, la PR reste BEHIND présentée comme réparée jusqu'à six heures (UPDATE_ATTEMPT_TTL)"
  - "le claim anti-thrash reste posé, aucun webhook CI ne vient, l'absence de signal est indistinguable du succès"
root_cause: logic_error
resolution_type: code_fix
tags:
  - async-api
  - http-202
  - accepted-vs-observed
  - update-branch
  - false-success
  - negative-controls
  - tokio-paused-clock
  - exhaustive-match
related_components:
  - crates/mika-agent/src/server/ci_success_handler.rs
  - crates/mika-agent/src/server/verdict_handler.rs
---

# Un 202 Accepted n'est pas un effet observé

## Problem

`attempt_update_branch` émet `PUT repos/{repo}/pulls/{n}/update-branch` et traitait l'exit zéro
comme « la branche est à jour ». L'endpoint répond `202 Accepted` et fait le merge **de façon
asynchrone**. Le zéro prouve que GitHub a pris la requête, jamais qu'un commit existe. Le résultat
rendu (`BranchUpdated { new_main_sha }`) publiait pourtant une donnée que rien n'avait lue
(mika#2252, correctif en PR mika#2604, non mergée au moment de l'écriture).

La classe dépasse cet endpoint. Toute API qui répond « accepté, je m'en occupe » (202, un job
enfilé, une file de dispatch, un auto-merge armé) donne un code retour qui porte sur la *requête*,
pas sur l'*effet*. Nommer la variante d'après l'effet revient à déclarer ce qu'on n'a pas mesuré.

## Symptoms

- `BranchUpdated.new_main_sha` rempli depuis `info.current_main_sha` (le SHA visé), sérialisé
  vers le moniteur et vers le LLM comme s'il avait été lu.
- Quand le job GitHub échoue : ni commit, ni CI, ni webhook. La boucle a consommé son unique
  tentative, et le claim n'était relâché que pour `Failed`. La PR reste donc bloquée jusqu'au TTL
  de six heures ou jusqu'au prochain mouvement de `main`.
- Le fichier se contredisait lui-même. Le doc-comment affirmait « every string this module renders
  for `Updated` says "accepted" ». C'était vrai d'une surface sur cinq : deux doc-comments d'enum,
  le jeton de trace et le champ de donnée disaient autre chose.

## What Didn't Work

- **Comparer les SHA juste après le 202.** Le premier brouillon du ticket le prescrivait. Or
  l'endpoint est asynchrone par contrat : une lecture à t=0 voit l'ancienne base. On obtient un faux
  négatif systématique sur une PR qui atterrit normalement.
- **Corriger seulement la prose.** mika#2250 avait fait dire « accepted » à la chaîne rendue au LLM.
  Le champ de donnée et le jeton de trace n'avaient pas bougé. Corriger la prose sans la donnée
  laisse le mensonge dans la partie que les machines lisent.
- **Prédire « aucun autre fichier touché ».** Le plan affirmait que `ci_success_handler.rs` et
  `verdict_handler.rs` ne bougeraient pas. Ils bougent, mais seulement dans leurs tests : leurs
  fixtures construisent `BehindMainRemediation::Updated`, et la variante a gagné un payload. « Le
  compilateur énumère les sites » inclut les modules de test des autres fichiers. L'écart a été
  relevé et accepté au rescue (session history).

## Solution

Trois niveaux, chacun ne dit que ce qu'il sait.

1. **La variante du code retour dit « accepté ».** `UpdateBranchOutcome::Updated` devient
   `Accepted` (`pr_merge_with_gate.rs:1078`). Son doc-comment ne revendique aucun commit.
2. **Seule une relecture produit « observé ».** `observe_update_branch_landing`
   (`pr_merge_with_gate.rs:1040`) relit la base de la PR. L'effet réseau est injecté sous forme de
   closure :

   ```rust
   for _ in 0..budget.attempts {
       tokio::time::sleep(budget.delay).await;          // attente AVANT la lecture
       if let Ok(observed) = read_base().await
           && !observed.is_empty()                       // champ absent != atterrissage
           && observed != before_base_sha
       {
           return LandingObservation::Landed { observed_base_sha: observed };
       }
   }
   LandingObservation::NotObserved
   ```

   `Landed` donne `BehindMainRemediation::Updated { observed_base_sha }` (`:1098`). `NotObserved`
   donne `AcceptedNotLanded` (`:1109`).
3. **La donnée publiée porte la valeur LUE.** `BranchUpdated.new_main_sha = observed_base_sha`
   (`:1547`), jamais le SHA visé. `AcceptedNotLanded` rend `Blocked { reason: BehindMain }` (la PR
   *est* encore behind) plutôt qu'une septième variante de `MergeGateResult`, qui aurait rendu faux
   les trois prompts embarqués (« six typed variants… exhaustively »).

Le budget est de trois lectures espacées de deux secondes (`UPDATE_LANDING_BUDGET`, `:1002`). C'est
un choix, pas une mesure. Ce qui le rend sûr, c'est qu'un faux négatif est **bénin** :
`AcceptedNotLanded` relâche le claim, et le tour suivant re-mesure `is_behind_main`. Il n'y a pas de
double merge commit, et on perd au plus un tour. Sans cette propriété écrite, un lecteur futur
allongera le budget pour de mauvaises raisons.

## Why This Works

Le défaut venait de ce qu'un seul nom couvrait deux faits : « GitHub a pris la requête » et « la
base a bougé ». En les séparant en deux types, aucun appelant ne peut plus lire « réparé » sur une
issue que rien n'a mesurée. Le type impose la lecture avant la publication. Et la relecture est une
closure injectée, donc la décision est une boucle pure testable sans forge : c'est l'idiome du
`poster` de `server::deadline_verdict`.

## Prevention

Ce qui n'était pas évident et n'a été établi qu'à l'implémentation :

- **Trois contrôles négatifs SÉPARÉS**, un par terme fail-safe :
  `mika2252_an_unreadable_base_is_never_a_landing` (`Err`),
  `mika2252_a_base_that_never_moves_is_not_observed` (SHA identique),
  `mika2252_an_empty_base_is_never_a_landing` (SHA vide). Si on neutralise les trois termes dans un
  seul test, un prédicat qui n'en lit qu'un passe au vert. Le terme du SHA vide est porteur :
  `PrPreflight::base_ref_oid` est `#[serde(default)]` (`:668-669`), donc `""` quand GitHub ne
  renvoie pas le champ. Sans ce terme, `"" != before` serait vrai et un champ absent se lirait comme
  un atterrissage : le même défaut, une couche plus bas.
- **Un compteur d'appels ne prouve pas l'ordre attente → lecture.** Vérifié par mutation : si on
  supprime le `sleep`, tous les autres tests restent verts, parce qu'une boucle
  lecture-puis-attente atteint le même SHA à la même itération. Seule une horloge le distingue :
  `#[tokio::test(start_paused = true)]` et l'assertion que la première lecture a lieu à
  `elapsed == budget.delay` (`mika2252_the_wait_precedes_the_first_read`, `:2856`).
- **La fixture « observé » doit différer de TOUTES les valeurs que l'ancien code pouvait publier.**
  `OBSERVED_SHA` (`:2355`) est distinct de `pr_base_sha` et de `current_main_sha`. Avec
  `observed == current_main_sha`, le test de `new_main_sha` passerait sur le code d'avant le fix.
  C'est ce troisième SHA qui distingue « mesuré » de « déclaré »
  (`mika2252_branch_updated_carries_the_observed_sha_not_the_target`).
- **Sortir la décision d'un `matches!` vers un `match` exhaustif sans `_`.** Le relâchement du claim
  était `if matches!(remediation, Failed(_))`, le seul site de la chaîne que le compilateur ne tenait
  pas (mika#1940). Le plan prévoyait de le couvrir par un test. L'implémentation l'a extrait en
  `releases_claim` (`:1207`), un `match` exhaustif sans bras `_`. Une nouvelle variante devient une
  erreur de compilation là où se prend la décision, au lieu de retomber en silence sur « garder le
  claim ». Même traitement pour les jetons de trace (`behind_main_trace_fields`, `:1467`), qui sont
  un format de fil : `"updated"` est gardé, `"accepted_not_landed"` est ajouté.

Règle générale, pour toute API asynchrone : nommer la variante d'après ce que le code retour prouve,
puis faire produire la variante « effet » par une lecture seulement. Enfin, écrire au moins un test
dont la valeur observée diffère de la valeur visée.

## Related Issues

- mika#2252 (le défaut), PR mika#2604 (le correctif), mika#2238 (le mécanisme update-branch et son
  claim anti-thrash), mika#2250 (la correction de prose seule).
- `docs/solutions/logic-errors/send-message-tool-false-success-on-gateway-error.md` est la classe
  voisine : un non-2xx lu comme un succès. Ici, c'est un 2xx vrai qui ne porte pas sur l'effet.
- `docs/solutions/best-practices/rust-async-deadline-patterns-2026-04-28.md` explique
  `start_paused` et l'auto-avance de l'horloge tokio.
