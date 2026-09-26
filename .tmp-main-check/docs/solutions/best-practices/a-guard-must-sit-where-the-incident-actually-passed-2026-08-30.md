---
title: "Une garde doit se poser là où l'incident est réellement passé, pas là où le défaut se raconte"
date: 2026-08-30
category: best-practices
module: agent-core
problem_type: best_practice
component: dispatch
severity: high
applies_when:
  - Ajouter une garde structurelle sur un chemin qui a plusieurs entrées
  - Un ticket décrit un défaut par sa cause conceptuelle plutôt que par sa trace
  - Choisir entre un prédicat partagé et une garde par site d'appel
  - Écrire les tests d'une garde qui refuse
---

# Une garde doit se poser là où l'incident est réellement passé

## Le contexte

mika#2084. Une étiquette `dispatch:<siège>` dit quel dispatcher possède un ticket.
Personne ne la lisait. Résultat mesuré le 2026-08-30 : mika#2055 portait `dispatch:ssc`,
SSC avait ouvert la PR#2082, et la boucle a créé une seconde tâche sur le même ticket.
Deux écrivains sur une branche.

## Leçon 1 — Le ticket raconte le défaut ; la trace dit par où il est passé

Le ticket décrivait, correctement, un défaut de « chemin de dispatch », et le chemin de
dispatch le plus visible de ce dépôt est le handler ready-label
(`server/ready_label_handler.rs`), qui a déjà une garde de la même forme (mika#2046).
La pente naturelle est d'y poser la nouvelle garde et de s'arrêter là.

Mais la trace de l'incident, citée dans le corps du ticket, dit autre chose :

```
Label:  CI fix: mika#2082 SIGPIPE lint … (issue mika#2055)
source: self_dev   trigger: manual
```

`trigger: manual`, pas `issues.labeled`. La tâche fautive **n'est jamais passée par le
handler ready-label**. Une garde posée uniquement là aurait été verte, revue, mergée — et
n'aurait pas empêché l'incident qu'elle citait en justification.

La couche porteuse était `skills/executor.rs::validate_dispatch_readiness`, la frontière
d'outil que tous les chemins traversent. Le dépôt le savait déjà et l'avait écrit à côté,
pour mika#2046 : *« The pre-LLM ready-label handler refuses the webhook path; this refuses
every path, whatever originated the turn. »*

**À faire** : avant de choisir l'emplacement d'une garde, relire la trace de l'incident et
identifier par quel chemin il est **réellement** passé — pas par quel chemin il *aurait
pu* passer. Si le ticket cite des identifiants de tâche, un `source` et un `trigger`, ce
sont eux qui désignent le site, pas la prose.

## Leçon 2 — Un prédicat partagé couvre trois sites ; une garde par site en couvre un

Le plan initial plaçait la garde de sélection dans `classify_stuck_ready_in_memory`
(`auto_pull.rs`). En remontant les callsites, trois endroits distincts posent le label
`ready` — le feeder, la sélection auto-pull, et le sauvetage de phase 2 — et un seul
d'entre eux traverse ce classifieur.

Le point commun réel était `is_feeder_excluded`, le prédicat d'exclusion que les trois
chemins interrogent. La garde y vit maintenant, et couvre les trois d'un coup.

**À faire** : ne jamais poser une garde sur un site d'appel avant d'avoir gréppé *tous*
les sites de l'action gardée et cherché leur prédicat commun. Ici : `grep -n
'gh_apply_label(.*"ready")'` d'abord, la garde ensuite.

Corollaire tenu ici : quand le prédicat partagé absorbe la décision, le motif de refus
devient celui du prédicat partagé (`operator_review_or_blocked`), qui nomme la mauvaise
cause. La garde a donc été **aussi** testée en amont dans le classifieur, uniquement pour
que la trace nomme `seat_owned_by_other`. Même décision, étiquette honnête.

## Leçon 3 — Distinguer « information absente » de « information non résoluble »

Deux cas qui se ressemblent et ne se tranchent pas pareil :

| Cas | Décision | Pourquoi |
|---|---|---|
| Étiquette `dispatch:zorglub`, `dispatch:`, ou deux étiquettes de siège | **Refus** | Un siège qu'on ne sait pas identifier n'est pas une autorisation |
| Appel `gh` en échec, token absent, issue non résolue | **Passage** + `warn!` | Fail-closed ferait de chaque hoquet réseau un arrêt complet de la boucle |

Les confondre dans le sens « fail-closed partout » produit un correctif qui casse plus que
le défaut qu'il répare. C'est le mode d'échec dominant de ce genre de garde, parce qu'il
se déguise en prudence.

## Leçon 4 — Un test de refus qui n'est pas apparié ne mesure rien

« Une issue étiquetée pour un autre siège est refusée » est satisfait par un refus
universel. Chaque test de refus doit avoir son jumeau positif : « une issue non étiquetée
est toujours dispatchée ».

Vérifié ici par mutation, et le résultat vaut d'être noté :

- Neutraliser la garde (`refuses()` → `false`) : **7 tests rougissent**. Les tests de
  refus mesurent bien la garde.
- Élargir la garde (`refuses()` → `true`) : **12+ tests existants d'`auto_pull` rougissent**,
  la plupart écrits des mois plus tôt pour d'autres raisons.

Le second chiffre est le plus intéressant : la suite existante était déjà un filet
anti-sur-refus, parce qu'elle exerce le chemin nominal sur des tickets ordinaires. Un
correctif qui refuse trop se fait attraper par les tests des autres, pas par les siens.

**À faire** : après avoir écrit une garde, la casser dans les deux sens et compter ce qui
rougit. Si l'élargissement ne casse rien, la couverture du chemin nominal est le vrai trou.

## Leçon 5 — Garder le champ que le sous-système lit réellement, pas celui qui le décrit le mieux

La garde a d'abord jugé le siège d'après `task.reference_url`, au motif que
c'est le champ que le moteur a lui-même écrit et que la tâche de l'incident le
portait. Raisonnement séduisant, et faux.

`dispatch-lib.sh` ne lit **jamais** `reference_url`. Il lit `.prompt`
(`dispatch-lib.sh:769`), en dérive dépôt et numéro (`:1076-1080`), la branche
(`:1131`), le worktree (`:1176`). C'est donc le prompt qui détermine où un second
écrivain atterrirait. Une tâche référençant l'issue A dispatchée avec
`{"prompt": "mika#B"}` passait la garde et écrivait sur la branche de B.

**À faire** : pour une garde qui protège une ressource (une branche, un fichier,
un répertoire), identifier le champ que le composant qui **touche** la ressource
consomme, et garder celui-là. Le champ le plus descriptif n'est pas
nécessairement le champ effectif. Quand les deux existent et peuvent diverger,
les interroger tous les deux et refuser si l'un refuse.

Corollaire observé dans le même dépôt : les gardes voisines avaient déjà chacune
leur règle — l'allowlist mika#2046 juge le prompt seul, la garde de grooming
mika#919 juge `github_ref` seul. Trois gardes, trois définitions de « la cible ».
C'est une dette à connaître avant d'en ajouter une quatrième.

## Leçon 6 — `gh issue view <n>` accepte un numéro de PR et répond sans broncher

Mesuré sur ce dépôt :

```
$ gh issue view 2082 --repo senara-solutions/mika --json labels
{"labels":[]}          # code de retour 0 — or 2082 est une PULL REQUEST
```

Issues et PR partagent un espace de numérotation, et l'endpoint issues sert les
deux. Une garde qui lit des étiquettes par ce chemin ne tombe donc pas en
fail-open bruyant sur une PR : elle rend un verdict **confiant sur le mauvais
objet**, sans aucun signal. Ici, exactement sur la paire issue/PR de l'incident
(#2055 / #2082).

Le discriminant est le champ `pull_request` de la réponse REST
`/repos/{owner}/{repo}/issues/{n}`, absent pour une vraie issue. À utiliser dès
qu'un numéro d'origine incertaine sert à lire des métadonnées d'issue.

Bénéfice adjacent : passer au client REST déjà présent
(`github_graphql::fetch_issue_body`, timeout 10 s) a retiré du chemin chaud du
dispatch un sous-processus `gh` dont le `wait()` n'a **aucun** timeout
(`pr_merge_with_gate.rs:795-798`). Une garde anti-blocage ne doit pas introduire
une nouvelle façon de bloquer.

## Leçon 7 — Un refus placé après une mise en file ne refuse pas, il thésaurise

La garde compilait naturellement après le contrôle de dispatch global, qui met
en file un rappel différé quand le créneau est occupé (mika#1011). Un dispatch
de siège étranger tenté pendant ce créneau était donc **mis en file**, rejoué
plus tard, refusé alors — et chaque rejeu brûlait un wrapper sur un ticket qui ne
pourrait jamais partir.

**À faire** : situer une garde structurelle par rapport aux points de mise en
file, pas seulement par rapport aux effets de bord finaux. Refuser avant la file,
pas après.

## Voir aussi

- `docs/solutions/architecture-patterns/post-hoc-vs-tool-boundary-guard-placement-2026-05-13.md`
  — pourquoi la frontière d'outil, et pas une garde post-hoc
- `docs/solutions/1053-dispatch-trigger-allowlist-config-constant.md` — le raisonnement
  « liste tenue à la main plutôt que dérivée » repris ici pour les sièges
- mika#2046 — le précédent structurel dont ce correctif copie la forme
