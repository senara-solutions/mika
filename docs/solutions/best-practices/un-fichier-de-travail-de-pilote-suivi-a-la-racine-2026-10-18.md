---
title: Un fichier de travail de pilote suivi à la racine — deux chemins d'entrée, donc deux gardes
date: 2026-10-18
last_updated: 2026-10-18
category: best-practices
module: skills/bundled/_shared/dispatch-lib
tags:
  - dispatch-lib
  - post-flight-recovery
  - pathspec
  - ci-lint
  - declaration-guard
  - pilot-scratch
  - repo-hygiene
problem_type: best_practice
component: dev-loop
severity: medium
applies_when:
  - Vous voyez un `.md` de travail (brouillon de corps de ticket, brief de groom) suivi à la racine du dépôt
  - Vous ajoutez une exclusion à `RESCUE_EXCLUDE_PATHSPEC` et hésitez entre `:!motif` et `:(exclude,glob)motif`
  - Une prescription de commande demande au pilote d'écrire sous `/tmp` et vous vous demandez où le fichier atterrit vraiment
  - Vous écrivez un garde d'égalité entre ce que le dépôt contient et ce qu'un fichier déclare
---

# Un fichier de travail de pilote suivi à la racine

## Le problème

Le 2026-10-18, trois fichiers de travail de pilote étaient suivis à la racine de
`main`. Aucun fichier du dépôt ne les référençait, et comme ils étaient sur
`main`, **chaque nouveau worktree de dispatch les embarquait**.

| fichier | ce que c'est | entré par |
|---|---|---|
| `groom-brief-mika-789-pass2.md` (127 lignes) | brief de seconde passe architecte | `4dcb2c65`, 2026-05-19 — **cinq mois** à la racine |
| `followup-issue-body.md` (70 lignes) | corps de ticket pour `gh issue create --body-file`, jamais déposé | `b99c6219`, `wip(mika#2317): trailing content after pilot end_turn` |
| `followup-issue.md` (59 lignes) | idem | `50e98793`, `wip(mika#2309): impl staged by post-flight recovery` |

## Les deux chemins d'entrée, et pourquoi une seule garde ne suffit pas

Les deux messages de commit des brouillons de suite **nomment eux-mêmes leur
chemin d'entrée** : ce sont les sites de récupération de `dispatch-lib.sh`
(`_rescue_dirty_worktree`, mika#1282, et le contenu traînant après `end_turn`,
mika#1383). Ils stagent tout ce que le pilote a laissé, et leur seule exclusion
partagée — `RESCUE_EXCLUDE_PATHSPEC` — ne portait que de l'échafaudage
`.claude/`.

Le brief de mai est entré **autrement** : le pilote l'a `git add`é et commité de
sa main, dans sa propre PR. Aucune exclusion de récupération n'aurait pu
l'arrêter, et la revue QA l'a laissé passer.

**D'où la règle.** Une exclusion de récupération ferme le chemin *à la source* ;
elle ne couvre pas le fichier que le pilote commite lui-même. Pour celui-là, il
faut un garde *au bord* : une déclaration du contenu légitime de la racine,
confrontée à `git ls-files` sur chaque PR (`scripts/check-root-files.sh`).

## La cause de production, qui vit ailleurs

Les commandes d'orchestration prescrivent au pilote d'écrire son brief sous
`/tmp` (`mika-groom-ticket.md`, `mika-groom-milestone.md`, `mika-ask.md`). Or la
permission-policy de la sandbox **refuse** toute écriture hors worktree
(`[policy:deny] Write: /tmp/…`, règle de dispatch mika#2211 ; depuis mika#2548 le
brouillon vit sous `.pilot-scratch/`). Un pilote qui suit la prescription reçoit
un refus et **replie son brouillon là où il peut — à la racine du worktree.**

Ces trois fichiers sont des commandes du méta-dépôt, seedées dans le worktree par
`_seed_worktree_slash_commands` et exclues de `git status` (mika#1415) : un ticket
ouvert sur `senara-solutions/mika` ne peut pas les éditer. Suivi en
**senara-solutions/mika-platform#207**.

Conséquence pratique : tant que la prescription vit, le garde de la racine n'est
pas une ceinture de confort — c'est la seule chose qui arrête le symptôme.

## `:(exclude,glob)` et non `:!` — mesuré, pas une préférence de style

Mesuré dans un dépôt de sonde le 2026-10-18 :

| pathspec | `followup-issue.md` (racine) | `sub/followup-issue.md` | `followup-issue-dir/x.md` |
|---|---|---|---|
| `:!followup-issue*.md` | exclu | indexé | **exclu** ← faux positif |
| `:(exclude,glob)followup-issue*.md` | exclu | indexé | indexé |

Sans la magie `glob`, git applique `fnmatch` **sans** `FNM_PATHNAME` : le `*`
traverse les `/`, et `:!followup-issue*.md` avale un répertoire entier dont le nom
commence par `followup-issue` — du contenu de pilote, silencieusement perdu.
`glob` ancre le motif à un seul segment de chemin, ce qui est exactement
l'exclusion « à la racine » demandée.

Le **mélange des deux formes dans une même invocation est vert** : la magie de
pathspec est par-entrée, pas globale. Les quatre entrées historiques du tableau
restent donc inchangées — les reformuler serait un diff sans mesure derrière lui,
sur le chemin le plus coûteux à réparer du dépôt.

## Un test qui charge le tableau réel, et non sa copie

Les cas voisins de `test-dispatch-lib.sh` codent leur pathspec en dur
(`':!.claude/commands/'`). Ils testent **leur propre copie** : si
`dispatch-lib.sh` perdait l'entrée, ils resteraient verts. Le cas de mika#2760
charge la définition depuis le fichier :

```bash
def=$(grep -E '^RESCUE_EXCLUDE_PATHSPEC=\(' "$DISPATCH_LIB" | head -1)
eval "$def"
git -C "$test_dir" add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}"
```

`eval` sur une ligne unique, auto-contenue, extraite par ancrage en colonne zéro :
le fichier entier n'est pas sourcé (cela exécuterait son setup top-level). Une
définition multi-lignes ou renommée rendrait une chaîne vide, le tableau serait
vide, et le cas **rougirait** — la forme qui casse est rouge, pas verte.

Et le contrôle négatif est ce qui donne sa valeur au cas positif : le même
fixture, re-stagé avec le tableau **privé des deux nouvelles entrées**, doit
indexer les trois brouillons. Sans lui, le cas positif passerait aussi si
l'exclusion venait d'un `.gitignore` ou d'un `core.excludesFile` de la machine.

## Le garde de la racine : trois propriétés qui ne sont pas négociables

`scripts/check-root-files.sh` confronte `git ls-files` (restreint à la racine) à
`scripts/root-files-declared.txt`.

1. **Bidirectionnel.** Les deux listes sont le *même* vocabulaire, pas un
   sous-ensemble l'une de l'autre. Un fichier non déclaré rougit ; une entrée
   déclarée orpheline rougit aussi. C'est ce second sens qui rend la liste
   auto-nettoyante : elle ne peut pas survivre à la suppression du fichier
   qu'elle déclare.
2. **Il ne peut pas passer à vide → exit 3.** Côté A vide (zéro fichier suivi à
   la racine, ou répertoire qui n'est pas un dépôt git) et côté B sans entrée
   parseable (fichier vide, ou commentaires seuls) sont des *lectures ratées*, pas
   des accords. Un garde qui n'a rien lu ne doit pas dire « d'accord ».
3. **La liste est générée, jamais écrite de mémoire** —
   `git ls-files | grep -v '/' | LC_ALL=C sort`, et **après** le retrait des
   fichiers fautifs. Générée avant, elle aurait inscrit les trois résidus et le
   garde serait vert sur exactement l'état qu'il existe pour refuser.

Deux détails d'implémentation qui ont chacun coûté une mesure :

- **`ls-files`, pas `ls-tree HEAD`.** L'index compte : un brouillon simplement
  *stagé* est déjà une violation, et c'est le bon moment pour le dire.
- **`LC_ALL=C` sur `comm` autant que sur `sort`.** `comm` compare avec la
  collation de **son propre** environnement : donné des flux triés en C mais
  comparés sous une locale UTF-8, les deux se désynchronisent à la première entrée
  que les deux collations ordonnent différemment (ici `.env.example` face aux noms
  en majuscules), et `comm` rapporte alors une divergence qui n'est réelle dans
  aucun sens. Mesuré : quatre assertions du harnais rouges, le garde sortant
  pourtant 1 — pour la mauvaise raison.
- **`-c core.quotePath=false` + `ls-files -z`.** La population de ce dépôt écrit
  en français ; sous le quoting par défaut, `archivé.md` ressort
  `"archiv\303\251.md"` et n'égale jamais sa ligne déclarée.

## Ce qui n'était pas le bon remède

- **`.gitignore`.** Il n'arrête pas un `git add` explicite, donc ne couvre pas le
  chemin 2 ; et il masquerait le fichier dans `git status`, c'est-à-dire
  exactement le signal qu'on veut voir.
- **Exclure `*.md` à la racine du worktree.** Un pilote écrit légitimement
  `pr-body.md` à la racine (règle mika#2211). Deux motifs nommés, tirés des trois
  incidents réels, valent mieux qu'une règle large dont le premier faux positif
  mangerait du contenu de pilote.
- **Une allowlist d'exceptions.** « On déclare, on n'allowliste pas »
  (mika#2201) : `root-files-declared.txt` est la déclaration du contenu légitime
  de la racine, la même nature que `.github/labels.yml` pour les labels. Il n'y a
  pas de second fichier d'exemptions, et le remède à un run rouge est une ligne
  déclarée ou un fichier déplacé.

## En pratique

Vous ajoutez un fichier à la racine du dépôt :

```
# dans le MÊME commit que le fichier
echo "mon-fichier.toml" >> scripts/root-files-declared.txt   # puis re-trier
bash scripts/check-root-files.sh
```

Vous supprimez un fichier de la racine : retirez sa ligne dans le même commit,
sinon le sens « entrée orpheline » rougit.

Vous êtes un pilote avec un brouillon à écrire : il va sous `.pilot-scratch/`
(règle de dispatch mika#2548), jamais à la racine, jamais sous `/tmp`.

## Références

- mika#2760 — le ticket, les trois fichiers, les deux chemins
- mika-platform#207 — les prescriptions `/tmp` des commandes de groom
- mika#1282, mika#1383 — les deux sites de récupération qui ont commité les brouillons
- mika#2348 — la propriété « une liste, tous les sites de staging »
- mika#2092, mika#2542 — les deux gardes de déclaration dont celui-ci reprend le contrat
- mika#2103 — un garde que personne n'a vu rougir est une décoration
