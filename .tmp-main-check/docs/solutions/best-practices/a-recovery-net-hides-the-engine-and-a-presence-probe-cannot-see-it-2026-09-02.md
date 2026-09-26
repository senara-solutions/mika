---
module: dispatch
tags: [bwrap, containment, sandbox, git-worktree, wip-rescue, canary, probe-vacuity, dispatch-lib, mika-2141, threat-model]
problem_type: silent-failure
category: best-practices
---

# Un filet de secours cache le moteur, et une sonde de présence ne peut pas le voir

Du 2026-08-04 au 2026-09-02, **aucun pilote autonome n'a pu commiter**. La boucle
paraissait livrer. Une suite adverse tournait à chaque déploiement et restait
verte. Le diagnostic a porté un mois durant sur l'aval — rappels honnêtes,
verdicts de re-grooming, létalité du garde-fou — d'un moteur qui ne pouvait pas
écrire. Trois leçons transférables, chacune payée.

## 1. Le `.git` d'un worktree lié est un fichier, pas un répertoire

Le confinement montait `--bind "$WORKTREE_DIR" "$WORKTREE_DIR"` et rien d'autre.
Or :

```
$ cat <worktree>/.git
gitdir: /data/workspace/mika-platform/mika/.git/worktrees/mika24
```

Ce chemin est **hors du bind**. Pas de gitdir, pas de `commondir`, pas de store
d'objets — `fatal: not a git repository` sur *toute* commande git, dans *tous*
les worktrees, alors que le même worktree répond normalement vu de l'hôte.

**À retenir :** confiner un worktree git ne se réduit jamais à monter son
répertoire. Vérifier avec `cat <worktree>/.git` avant de raisonner sur les binds.

## 2. Une sonde de présence de binaire n'est pas une sonde de capacité

L'unique vérification git de la suite « must-work » était :

```bash
git --version 2>&1 | head -1 || echo "FAIL: git missing"
```

Verte un mois durant dans un namespace où aucune opération de dépôt ne marchait.
La suite adverse existait, tournait, et **ne pouvait structurellement pas**
falsifier ce défaut.

**À retenir :** `--version`, `command -v`, `[ -x ... ]` prouvent qu'un outil est
là. Ils ne prouvent rien de ce qu'il peut faire. Pour chaque capacité dont
dépend la livraison, une sonde doit *exercer* la capacité — ici `rev-parse`,
`status`, `add`, `commit`, et la résolution du remote. Un test qui construit
l'argv `bwrap` sans lancer le bac à sable a exactement le même angle mort :
l'argv était « correct » ; c'est le namespace résultant qui n'avait pas de
gitdir.

## 3. Un filet de secours transforme une panne de moteur en succès apparent

La récupération post-vol (mika#1282) commite le worktree sale **côté hôte**
quand le pilote meurt sans commit, puis ouvre une PR brouillon. Résultat : la
boucle *semblait* livrer. Le contrôle qui a fini par trancher est un décompte,
pas une lecture :

| origine | PR mergées depuis le 2026-08-04 |
|---|---|
| `origin:loop` **sans** `wip-rescue` | **aucune** |
| `origin:loop` **avec** `wip-rescue` | trois |

Zéro PR produite par un pilote. Toutes les livraisons de la boucle passaient par
le filet.

**À retenir :** quand un système a un mécanisme de secours, l'artefact final ne
dit plus lequel des deux l'a produit. Il faut **estampiller le producteur** —
c'est ce que fait l'étiquette `origin:*` (mika#2090) — puis compter par
producteur. Devant un pilote qui n'a pas livré, la première question n'est ni
« a-t-il émis des outils » (il en émettait 11 à 35 par session) ni « qu'est-ce
qui l'a tué », c'est **« pouvait-il écrire ? »**.

## 4. Corollaire de sécurité : la surface élargie alimente parfois la décision d'élargir

Le correctif dérive les binds côté hôte à partir du `.git` du worktree et de
`worktrees/<name>/HEAD`. Mais `.git` est dans `$WORKTREE_DIR`, monté rw au
pilote, et `HEAD` est monté rw **par le correctif lui-même** — et les worktrees
persistent entre dispatches. Un `ref: refs/heads/../../../..` écrit depuis le
bac à sable aurait fait monter un chemin hôte arbitraire en rw au dispatch
suivant.

**À retenir :** après avoir élargi un bind, se demander si la nouvelle surface
inscriptible **alimente le calcul des binds**. Si oui, la résolution doit passer
par un composant qui valide (ici `git rev-parse` / `git symbolic-ref`, qui
imposent la grammaire des refs et le rattachement du worktree), ré-affirmer
l'invariant de liaison explicitement, et re-filtrer toute chaîne qui devient un
segment de chemin.

Un piège s'y cache : `git symbolic-ref --quiet` rend vide pour un `HEAD`
**trafiqué** comme pour un `HEAD` **détaché légitime**. Les confondre ne montait
rien de dangereux — mais dégradait en silence vers un pilote qui tourne et ne
peut pas commiter, c'est-à-dire vers la panne d'origine. **Un mode dégradé qui
ressemble au bug qu'on répare doit abandonner bruyamment.**

## 5. Un correctif qui répare `status` peut laisser `push` mort

Trois blocages **indépendants** empêchaient la livraison, et chacun était
atteignable sans s'en apercevoir :

1. le gitdir non monté ;
2. le `config` du parent non monté — git marche, mais le dépôt n'a plus de
   remote ;
3. l'identité committer, qui ne vit que dans le `~/.gitconfig` de l'opérateur,
   effacé par `--tmpfs /home` ;

plus un quatrième pour `push` seul : le remote est en SSH et le bac à sable n'a
ni clé ni agent, délibérément — il fallait la réécriture d'URL vers HTTPS, que
l'injection host-side attendait déjà.

**À retenir :** valider un correctif de confinement sur `status` et `commit`
seuls est un piège. La sonde décisive est celle qui va jusqu'au bout de la
chaîne de livraison — ici `git ls-remote --get-url origin` doit rendre une URL
`https://`, et le commit doit être **visible côté hôte** sur la tête de branche.

## Vérifications qui tiennent maintenant

- `make test-sandbox-git-usable` — lance un `bwrap` réel via `_run_pilot_sandboxed`
  sur un worktree lié jetable ; exige la suite de dépôt verte, les contrôles
  négatifs rouges, et les entrées trafiquées refusées. Sans le correctif,
  10 assertions sur 22 tombent.
- `scripts/canary-pilot-containment` — sonde post-déploiement en bac à sable
  vivant ; `git --version` y reste, mais a cessé d'être la preuve.

## Références

- mika#2141 (ce ticket), `skills/bundled/_shared/dispatch-lib.sh` (`_pilot_gitdir_bind_args`, `_stage_pilot_gitconfig`)
- mika#1282 (récupération post-vol — le filet, à conserver), mika#2090 (étiquette `origin:*` qui rend le décompte possible)
- mika#2039 (canal secret hors argv), mika#2056 (jeton injecté côté hôte) — les deux gardes que ce correctif a dû satisfaire sans exemption
- `e4f24677` / PR#1894 — l'introduction du confinement, 2026-08-04
