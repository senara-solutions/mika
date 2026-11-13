---
module: skills, scripts, dispatch-lib
tags: [pilote, bac-a-sable, bwrap, consignes, chemins-absolus, recensement, structural-guard, anti-vacuity, mika-2108, mika-2146, mika-2141]
problem_type: instruction-surface-cites-unreachable-paths
category: best-practices
---

# La surface de consignes qu'un pilote reçoit sans la demander

## Le problème (mika#2108)

Un pilote de dispatch tourne sous `bwrap` avec `--chdir "$WORKTREE_DIR"` et un jeu
de binds qui monte le worktree plus le strict minimum de son gitdir
(`dispatch-lib.sh::_pilot_gitdir_bind_args`, mika#2141/#2146). **La racine du
workspace hôte n'est pas montée.**

Le ticket posait l'hypothèse que « les consignes que le pilote reçoit citent
constamment des chemins absolus », et que ces chemins ne mènent nulle part. La
seconde moitié a cessé d'être vraie le jour de mika#2146 : les binds du gitdir
créent leurs répertoires intermédiaires, donc

```
ls /data/workspace/mika-platform/mika
```

ne rend plus `ENOENT`. Il rend **un répertoire ne contenant que `.git`** — la forme
exacte d'un dépôt dont le checkout aurait disparu. C'est plus dangereux que le
silence : un non-répertoire ne répond pas ; ce squelette répond, et sa réponse est
fausse. Les deux sessions du premier commentaire du ticket (`2b74a8c6`, `9f144402`)
ont toutes deux conclu qu'il fallait « rétablir le dépôt git backing du worktree ».
Elles n'ont pas halluciné : elles ont lu le squelette et en ont tiré l'inférence
raisonnable.

La première moitié de l'hypothèse, elle, demandait à être mesurée plutôt que crue.

## La distinction qui porte tout : *chargé automatiquement* vs *lu à la demande*

| | ce que c'est | qui décide de l'ouvrir |
|---|---|---|
| **chargé automatiquement** | tout `CLAUDE.md` du cwd et de ses parents, tout `.claude/commands/*.md` | personne — Claude Code les charge seul |
| **lu à la demande** | `docs/plans/`, `docs/solutions/`, tout le reste | le pilote, délibérément, muni du contexte |

Un chemin hôte dans la première colonne est une consigne de navigation
impossible à suivre, reçue sans l'avoir cherchée. Dans la seconde, c'est une
ligne dans un compte rendu daté qu'on est allé ouvrir exprès. **Seule la première
mérite une garde**, et c'est pourquoi la garde ne s'applique pas à la seconde.

## Les mesures

Reproductibles telles quelles depuis la racine du dépôt. Chiffres relevés le
**2026-09-05** sur la branche `fix/2108/p1-dispatch-lib-le-bac-sable-ne-monte`
(rebasée sur `main` à `83464bdd`).

### M1 — la surface chargée automatiquement

```sh
grep -rc '/data/workspace/mika-platform' --include='CLAUDE.md' . | sort -t: -k2 -rn
git ls-files '.claude/commands/*.md' | xargs grep -c '/data/workspace/mika-platform'
```

| surface chargée automatiquement | fichiers | mentions de `/data/workspace/mika-platform` |
|---|---|---|
| `CLAUDE.md` (racine + 7 crates/dashboard/packages) | 8 | **6**, toutes dans un seul fichier |
| `.claude/commands/*.md` **suivis par ce dépôt** | 4 | **0** |
| `~/.claude/CLAUDE.md` | — | **absent du bac à sable** — `--tmpfs /home`, aucun bind |

Les six mentions sont `crates/mika-agent/CLAUDE.md` § *Multi-Corpus Support (#798)*,
et ce sont les `[kg].docs_roots` d'un exemple d'`identity.toml` **pour l'agent
`mika-arch` côté hôte**. Ce n'est pas une consigne de navigation adressée au
pilote : c'est une valeur de configuration décrivant un processus qui tourne
ailleurs. Correcte telle qu'écrite — annotée sur place depuis mika#2108, et seule
exemption déclarée de la garde.

### M2 — la surface lue à la demande

```sh
grep -rl '/data/workspace/mika-platform' --include='*.md' . | wc -l
```

**32 fichiers** (28 au 2026-09-03 ; la croissance est celle des plans et des
comptes rendus, pas celle de la surface chargée). Tous sauf `crates/mika-agent/CLAUDE.md`
sont des `docs/plans/` et `docs/solutions/` **historiques**. Sur les plans écrits
depuis le 2026-08-01, un seul cite un chemin hôte
(`2026-09-02-001-fix-2141-sandbox-gitdir-bind-plan.md`), et il parle précisément du
bind : le chemin hôte y est le sujet, pas une instruction.

**Ces 32 fichiers ne sont pas réécrits.** Ce sont des comptes rendus datés ; les
chemins hôte qu'ils citent étaient les chemins réels au moment où ils ont été
écrits. Les réécrire falsifierait le registre pour un gain nul.

### M3 — la correction que la mesure a imposée au plan

Le plan de mika#2108 mesurait `$WORKTREE_DIR/.claude/commands/*.md` à « 4 fichiers,
0 mention ». **C'est vrai des 4 commandes que ce dépôt suit, et faux de ce que le
pilote reçoit.**

`_seed_worktree_slash_commands` (mika#1173/#1415) copie dans
`$WORKTREE_DIR/.claude/commands/` **toutes** les commandes du méta-dépôt
`mika-platform`, puis les masque de `git status` via le `info/exclude` du
common-dir. Un worktree de dispatch en porte donc ~26, pas 4. Deux d'entre elles
citent la racine hôte :

| commande semée | mentions | nature |
|---|---|---|
| `mpc-inbox.md` | 4 | **navigation** — `ls -1t /data/workspace/mika-platform/.claude/inbox/` |
| `mika-onboarding.md` | 2 | dérivation d'un slug de mémoire à partir du chemin canonique |

`mpc-inbox.md` est exactement la forme que le ticket décrivait : une instruction
qui dit d'aller regarder à un endroit qui, dans le bac à sable, est un squelette.
**L'hypothèse de tête du ticket est donc confirmée — mais sur des fichiers qui
appartiennent à `mika-platform`, pas à ce dépôt.**

## Ce qui maintient M1 vraie : `scripts/check-pilot-instruction-paths.sh`

La garde échoue si un fichier de la surface **chargée automatiquement et suivie par
ce dépôt** contient un chemin absolu sous la racine du workspace hôte.

Quatre décisions de conception, chacune payée par un incident antérieur :

1. **La règle est la racine, pas un dépôt.** La garde refuse `/data/workspace/`,
   pas la chaîne `/data/workspace/mika-platform`. Un checkout voisin est le même
   défaut sous un autre nom. C'est la leçon de mika#2103 : *une garde qui ne
   connaît qu'une écriture d'un défaut ne protège de rien d'autre.*

2. **La surface est ce que le dépôt *suit* (`git ls-files`), pas ce que le
   système de fichiers contient.** C'est la conséquence directe de M3 : globber
   `.claude/commands/` rendrait la garde **rouge dans tout worktree de dispatch**,
   sur des fichiers d'un autre dépôt que celui-ci ne peut pas corriger. Une garde
   rouge là où elle ne peut pas être obéie se fait désactiver, pas obéir. La borne
   est réelle et laisse un trou réel — celui de M3 — nommé ici plutôt que laissé à
   redécouvrir. **Ne pas le combler en élargissant le glob.**

3. **L'exemption est la surface d'attaque**, donc elle est ancrée sur *le fichier
   **et** la forme de la ligne*, et elle doit rester **vivante** : si elle cesse de
   matcher quoi que ce soit, la garde sort en 4. Une exemption qui ne protège plus
   rien élargit quand même la garde pour toute ligne future qui grandirait dans sa
   forme.

4. **Ne rien avoir à vérifier n'est pas être propre.** Surface vide → sortie 3.
   Un parse silencieusement vide est indiscernable d'un arbre propre, et c'est
   comme ça qu'un check vert finit par ne plus rien vouloir dire.

Les deux sens sont épinglés par `scripts/test-check-pilot-instruction-paths.sh`
(20 assertions) : le dépôt réel passe, et chaque forme de réintroduction échoue —
y compris la ligne exemptée placée dans un autre fichier, un autre chemin hôte
dans le fichier exempté lui-même, et le passage d'une commande semée de
*non-suivie* à *suivie*.

## Ce qui répond à la question que le recensement ne ferme pas

Le recensement borne un texte. Il ne situe pas le pilote, et il ne peut rien
contre les fichiers d'un autre dépôt (M3). Deux mécanismes complètent, tous deux
livrés par mika#2108 :

- **le bloc d'orientation** apposé en *suffixe* au `PROMPT` par `dispatch-lib.sh` —
  il nomme la racine de travail réelle et déclare le workspace hôte non monté.
  Suffixe et jamais préfixe : la référence de ticket est analysée par découpage sur
  son premier `#`, et un bloc placé avant la déplacerait ;
- **`README-SANDBOX`**, matérialisé *dans* le squelette via `--ro-bind-data`. Le
  `ls` qui rendait « un dépôt sans checkout » rend désormais « un point de montage,
  et voici où est le vrai travail ».

Aucun des deux ne ferme la classe seul. Le préambule est du prompt : il réduit un
taux. Le marqueur répond à la tentative qui a quand même lieu. Ensemble ils
couvrent le raisonnement *et* le geste — et ils fonctionnent quel que soit le
dépôt propriétaire du texte fautif, ce que la garde, elle, ne peut pas.

## Voisins

- `docs/solutions/best-practices/2103-a-guard-that-knows-one-spelling-of-a-defect-protects-nothing-else.md`
- `docs/solutions/best-practices/an-exempted-disposition-is-the-attack-surface-2026-08-30.md`
- `docs/solutions/best-practices/a-guard-must-sit-where-the-incident-actually-passed-2026-08-30.md`
- `docs/plans/2026-09-02-001-fix-2141-sandbox-gitdir-bind-plan.md` — le bind du gitdir
