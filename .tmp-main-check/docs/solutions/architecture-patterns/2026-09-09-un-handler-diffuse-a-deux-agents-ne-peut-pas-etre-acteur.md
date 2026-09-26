---
module: mika-agent
date: 2026-09-09
problem_type: architecture_pattern
component: tooling
severity: high
tags:
  - webhook-handler
  - ci-success-handler
  - fan-out
  - agent-identity
  - pr-merge
  - forge-gate
  - signal-vs-actor
  - structural-enforcement
related_issues:
  - 2248
  - 2244
  - 1711
  - 1853
  - 2228
---

# Un handler diffusé à deux agents ne peut pas être acteur

## Le symptôme

mika#2244 s'est fermée le 2026-09-08 avec `mergedBy = mika-platform-qa` : le relecteur
a mergé sa propre approbation. Ni le verdict, ni les portes, ni le classifieur de
périmètre n'étaient en cause — tous ont rendu la bonne valeur. C'est **l'acteur** qui
était faux.

## La cause, en une phrase

`ci_success_handler` appelait `gh pr merge` directement, et ce handler tourne dans
**tous** les agents que le fan-out de l'événement atteint.

Le routage de `check_suite.completed(success)` a deux destinataires depuis mika#1711 :
`mika-dev` en primaire (`route_event`) et `mika-qa` en secondaire
(`secondary_targets`). Les deux exécutent la même chaîne de handlers structurels dans
`handlers.rs`, et le token du merge est celui de l'agent courant
(`Settings::resolve_github_token`). Le merge tournait donc **sous l'identité de celui
qui gagnait la course** — une loterie, pas une décision.

## La forme générale

> Un handler diffusé à N agents peut **évaluer** ; il ne peut pas **agir** sur une
> surface qui enregistre l'identité de l'acteur. Évaluation et action doivent être
> séparées par un signal, et l'action gardée par une liste blanche d'identité.

Les surfaces concernées sont celles où la forge (ou tout système externe) inscrit
*qui* a fait le geste : merger, approuver, fermer, publier. Un label ou une écriture
en base, non : personne ne lit leur auteur.

## Ce qui ne marchait pas, et pourquoi

- **Couche prompt : déjà correcte, et impuissante.** `qa-review-webhook-success` dit
  à mika-qa, textuellement, « never dispatch claude-pilot or merge tools », et
  l'allowlist de mika-qa ne contient ni `self-dev-webhook-ci` ni
  `self-dev-webhook-qa`. Le défaut est passé quand même : un handler structurel tourne
  **avant le tour LLM**, et aucune instruction de prompt ni aucune allowlist de skill
  ne l'atteint. Énième confirmation que l'application par prompt échoue au niveau du
  substrat de la boucle.
- **Déléguer la décision au LLM** (« demande à l'agent s'il a le droit ») :
  non-déterministe sur une action irréversible.
- **Injecter le token du dispatcher dans le handler du relecteur** : fuite de
  frontière — un agent lirait le credential d'un autre. Rejeté par mika-qa à la
  mesure.

## La forme retenue

Trois couches, chacune à usage unique, dans `mika_common::forge_identity` +
`server::merge_ready_handler` :

1. **L'évaluateur n'appelle plus le merge.** Il émet un `MergeReadySignal` et rend la
   main. Épinglé par un test de source (`run_gh_merge(` absent du fichier) — le défaut
   était une question de *callsite*, et un callsite se mesure sur la source.
2. **L'acteur est une liste blanche, pas une liste noire.** `merge_disposition` rend
   `Act` pour le seul dispatcher ; tout autre agent — y compris un agent qui n'existe
   pas encore — signale. Une liste noire aurait donné le droit de merger à chaque
   nouveau nom d'agent par défaut.
3. **Le contrôle d'égalité d'identité est une seconde ceinture, indépendante.**
   `would_merge_as_reviewer(agent, reviewer_login_du_signal)` refuse même si la liste
   blanche s'égarait. C'est l'énoncé de l'AC posé à l'exécution, pas seulement dans
   une table de routage.

Et l'acteur **revérifie le périmètre lui-même**, fail-closed : le signal dit « les
portes étaient vertes », mais celui qui écrit sur la forge répond de ce qu'il écrit.
C'est aussi la seule défense si un signal arrivait par un chemin que l'évaluateur n'a
pas posé.

## Le signal n'est pas un label — deux raisons mesurées

Le choix évident (« poser un label `merge-ready` ») a été écarté :

- **Permission.** Une écriture de label sous le PAT résolu échoue
  (`Resource not accessible by personal access token (addLabelsToLabelable)`, 29 refus
  mesurés le 2026-09-07, mika#2228). Le signal aurait exigé un second token, donc un
  second mode de panne, sur le chemin critique du merge.
- **Autorité.** Un label est écrivable par un humain. En faire le porteur de
  l'autorisation transformait un geste d'interface en permission de merge capable de
  contourner la porte décision-core.

Le signal vit donc dans la trace du tour (pré-digest, relu par le handler suivant) et
dans l'audit. L'autorité reste dans le code.

## Le coût, nommé

Le handoff passe par `req.text`. Un handler inséré **entre** l'évaluateur et l'acteur
réécrirait ce texte et emporterait le signal — sans qu'aucun test de comportement ne
bouge. D'où une assertion structurelle sur l'ordre de câblage dans `handlers.rs`.
C'est la contrepartie assumée d'un handoff en bande ; elle est bon marché tant qu'elle
est épinglée.

## Où chercher la même forme

Tout handler structurel appelé depuis la chaîne `channel == "github"` de
`handlers.rs`, croisé avec `secondary_targets` du gateway. Aujourd'hui un seul
événement fait l'objet d'un fan-out ; le jour où un second apparaît, la question à
poser est : *ce handler agit-il sur une surface qui enregistre son auteur ?*
