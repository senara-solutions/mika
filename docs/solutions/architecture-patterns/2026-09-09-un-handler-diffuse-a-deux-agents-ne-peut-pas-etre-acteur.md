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

## Le résidu (mika#2260) — une garde tardive résout la course, elle ne la supprime pas

Le correctif ci-dessus a corrigé **qui merge**. Il n'a pas corrigé **qui évalue**. La
re-preuve du 2026-09-09 a réussi (`mergedBy = mika-platform-dev` sur mika#2259,
`human_gate_events = 0`) et pourtant les deux agents entraient encore dans
l'évaluateur : le relecteur le parcourait en entier — jusqu'à émettre le signal — et
n'était retenu qu'à l'acteur. Comptes sur `audit_events` entre le déploiement et le
lendemain : **81** parcours complets sous mika-qa, **6** tenues DECISION-CORE (donc six
notifications opérateur en double), un signal émis, puis l'arrêt.

Une course résolue par une garde **tardive** est fragile parce qu'elle dépend de
l'ordre, et le prix de l'ordre n'était pas cosmétique. Cinq effets étaient exercés
**avant** la garde, dans l'ordre d'exécution :

1. **La clé de dedup mémoire**, globale au processus et clé sur
   `{repo}:{branch}:{head_sha}` **sans `agent_id`**. Le relecteur entrant en premier
   consommait le créneau du dispatcher. Sur #2259, head `047e9eea` : mika-qa évalue à
   09:08:11 et enregistre la clé, le replay différé de mika-dev rend
   `ci_success_dedup.skip` à 09:08:26. Sans conséquence là (checks encore `pending`) —
   mais à 09:12:36, toutes portes vertes, le dispatcher n'a évalué que parce que sa
   copie est arrivée **60,005 s** après l'enregistrement du relecteur contre une
   fenêtre de 60 s : **5 ms**. Un deferral à 59,9 s, et plus aucun signal n'est émis
   sous une identité qui peut agir, plus aucun `check_suite` n'arrive sur ce head
   (tous les workflows sont terminés), et **la PR reste verte et ouverte sans que rien
   ne la ferme**. C'est une classe d'arrêt de boucle à un timing près.
2. **Cinq appels `gh` par événement sous le PAT du relecteur** — quota et latence pour
   un signal qui, par construction, n'a aucun consommateur : il voyage en bande dans
   `req.text`, relu par le handler suivant *du même agent*, où l'acteur le retient.
3. **Une écriture sur la forge** — `PUT /repos/{repo}/pulls/{n}/update-branch` — posée
   par l'agent qui ne doit jamais agir sur cette PR, dès qu'elle est en retard sur main.
4. **La notification opérateur DECISION-CORE en double** (six mesurées).
5. **Le pré-digest de l'évaluateur remplaçait `req.text`** dans le tour du relecteur,
   qui recevait « Merge-ready signal emitted… » à la place de l'événement brut que son
   propre prompt attend pour corréler la PR.

Le remède est une **porte d'entrée** au premier point où l'agent est connu et avant
tout travail : après la sélection par type d'événement, avant l'exigence de token,
avant le premier appel `gh`, avant l'écriture de dedup. Elle rend
`Passthrough { enrichment: None }` — l'événement **brut**, ce que l'effet 5 avait
emporté — et laisse une ligne d'audit. La ceinture de l'acteur n'est pas retirée : elle
devient inatteignable en production et reste la défense d'un signal arrivé par un
chemin que l'évaluateur n'a pas posé.

**Deux noms, une seule liste blanche.** `merge_disposition` répond *qui merge* — une
décision d'acteur. `owns_merge_transition` répond *est-ce que mon évaluation a un
consommateur* — une décision d'entrée. Deux questions, deux sites d'appel, une table :
ajouter un acteur reste un geste explicite à un seul endroit.

**Et la garde qui tient ce nom unique est un scan de source, pas un test de
comportement** — parce que réécrire le prédicat en ligne (`merge_disposition(agent) !=
Act`) laisse tous les tests comportementaux **verts** et ne casse que le scan, ce qui a
été vérifié par mutation. Même famille que la divergence que `grooming_marker` a dû
engraver une fois : *un lecteur écrit une seconde fois est un lecteur qui peut
diverger, en silence, avec toutes les assertions au vert.*

## La règle, généralisée

Le correctif de 2026-09-09 disait : *un handler diffusé à deux agents ne peut pas être
acteur.* C'était vrai et insuffisant. La forme complète est :

> **Un handler diffusé à N agents n'évalue que dans l'agent qui peut consommer son
> évaluation.**

Un agent qui ne peut pas consommer le signal n'a aucune raison de l'émettre, et le
prix de le lui laisser émettre n'est pas nul : ce sont les cinq effets ci-dessus. Mettre
l'identité à la sortie borne le **résultat** ; la mettre à l'entrée borne le **travail**.

## Où chercher la même forme

Tout handler structurel appelé depuis la chaîne `channel == "github"` de
`handlers.rs`, croisé avec `secondary_targets` du gateway. Aujourd'hui un seul
événement fait l'objet d'un fan-out ; le jour où un second apparaît, les questions à
poser sont **deux**, et la seconde est celle que mika#2260 a coûtée : *ce handler
agit-il sur une surface qui enregistre son auteur ?* et *son évaluation a-t-elle un
consommateur dans l'agent qui la fait ?*
