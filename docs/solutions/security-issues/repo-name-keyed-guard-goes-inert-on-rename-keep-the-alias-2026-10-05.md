---
module: forge_identity
tags: [merge-gate, population, repo-rename, fail-open, type-witness, mika-2617]
problem_type: inert-guard
category: security-issues
---

# Une garde indexée par nom de dépôt devient inerte au renommage — garder le nom courant ET l'alias

## Le problème

mika#2617 (phase C) pose une précondition de merge propre à un dépôt : la trace
de gate MPC. Le plan, écrit le 2026-10-02, désignait la population par
`senara-solutions/claude-pilot-py`. Entre le plan et l'implémentation, le dépôt a
été **renommé** `senara-solutions/claude-pilot` (mesuré le 2026-10-05 :
`gh repo view senara-solutions/claude-pilot-py` rend l'URL du nouveau nom).

GitHub envoie le **nom courant** dans les webhooks et dans `headRepository`. Une
liste qui ne porte que l'ancien nom ne correspond donc **jamais** : la
précondition ne se lit plus, et une garde qui ne se lit pas est **ouverte**. Rien
ne rougit — aucun test ne voit un terme qui ne s'évalue jamais (classe mika#2205).

## Pourquoi l'ancien nom ne peut pas simplement être remplacé

GitHub **redirige** un dépôt renommé : `gh pr merge <n> --repo <ancien-nom>`
atteint la même PR. Un appelant qui écrit encore l'ancien nom — un prompt
périmé, une constante non migrée (`INTERNAL_REPOS` du gateway le liste encore)
— passerait par une porte ouverte si la liste ne porte que le nouveau nom.

D'où la forme retenue dans `MPC_GATE_REQUIRED_REPOS` :

- le **nom courant**, épinglé par un test (`mika2617_the_current_repo_name_is_in_the_population`) ;
- l'**ancien nom en alias**, parce que la redirection en fait une seconde clé
  vers la même ressource ;
- une comparaison **sans casse**, parce que GitHub résout `owner/repo` sans casse
  (même raisonnement : une seconde clé vers la même PR).

Pour une liste qui **restreint** (population d'une garde), garder un alias ne
coûte rien. Pour une liste qui **autorise**, la règle s'inverse : un alias élargit
l'autorisation, et il faut le peser.

## La garde elle-même : un témoin de type, pas un scan

La précondition devait tenir sur trois sites de merge, dont deux n'appellent pas
l'outil (plan R8). Plutôt qu'un scan de source, `run_gh_merge` exige un
`MergeClearance` (champ privé, défini dans `mika-common` donc privé d'une crate à
l'autre, seul constructeur `from_mpc_verdict`). Le compilateur a forcé les trois
sites. Un scan complémentaire n'épingle qu'une chose que le type ne peut pas
voir : que personne n'écrive `from_mpc_verdict(&MpcGateVerdict::NotRequired)`
pour sauter la lecture.

## Comment l'appliquer

- Avant d'implémenter une garde dont la population est un nom de dépôt écrit dans
  un plan, **re-mesurer le nom** (`gh repo view`) : un plan a une demi-vie.
- Écrire un test qui épingle le **nom courant** dans la population — c'est le
  test qui rougit le jour où la liste dérive.
- Chercher les autres listes du même nom dans l'arbre
  (`grep -rn '<ancien-nom>"' crates`) et les **nommer** dans la PR, même hors
  périmètre.
