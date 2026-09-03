# Plan : dire au pilote où il est, et faire dire la vérité au squelette (mika#2108)

**Ticket :** mika issue#2108 — `p1(dispatch-lib): le bac à sable ne monte que le worktree, mais les consignes citent les chemins de l'hôte — le pilote brûle ses tours à se chercher, puis meurt sur la forme large`
**Labels :** `bug`, `p1-important`
**Type :** issue (bug — ralentit la boucle)
**Palier de priorité :** Tier 2 — *ralentit la boucle*. Ne casse plus rien depuis mika#2146 ; brûle des tours et pousse vers la forme de commande qui tue (cpp#128).

---

## Résumé de la décision

Le ticket propose trois pistes et délègue explicitement l'arbitrage au grooming
(« Je ne choisis pas : la 2 est la moins chère et la 1 la plus propre, à trancher au
grooming »). **La mesure tranche autrement que ne le laissait attendre le corps du
ticket** : la piste 1 n'a presque rien à réécrire, et la piste 3 n'est pas une
commodité — elle est devenue nécessaire depuis mika#2146.

Ce plan livre **la piste 2 et la piste 3**, et réduit la piste 1 à une garde qui
maintient vrai le recensement dans le temps.

---

## Mesures — ce qui a été exécuté, pas déduit

Toutes les lignes ci-dessous ont été produites le 2026-09-03 sur `origin/main` à
`7b4ec10a`, après le merge de mika#2146.

### M1 — Recensement de la surface de consignes *chargée automatiquement*

La consigne qu'un pilote reçoit sans la demander, c'est : le `CLAUDE.md` de son cwd
et de ses répertoires parents, plus les commandes de `.claude/commands/`. Le prompt
lui-même est `mika#<n>` (`dispatch-lib.sh:2072`) et l'entrée est `/mika`
(`dispatch-lib.sh:5280`) — ni l'un ni l'autre ne porte de chemin.

| surface chargée automatiquement | mentions de `/data/workspace/mika-platform` |
|---|---|
| `$WORKTREE_DIR/CLAUDE.md` | **0** |
| `$WORKTREE_DIR/.claude/commands/*.md` (4 fichiers) | **0** |
| `crates/*/CLAUDE.md`, `dashboard/`, `packages/ui/` (7 fichiers) | **6**, toutes dans un seul fichier |
| `~/.claude/CLAUDE.md` | **absent du bac à sable** — `--tmpfs /home` et aucun bind |

Les six mentions sont `crates/mika-agent/CLAUDE.md:499-504`, et ce sont les
`[kg].docs_roots` d'un exemple de `identity.toml` **pour l'agent `mika-arch` côté
hôte**. Ce n'est pas une consigne de navigation adressée au pilote : c'est une
valeur de configuration décrivant un processus qui tourne ailleurs. Correcte telle
qu'écrite.

### M2 — Recensement de la surface *lue à la demande*

```
grep -rl '/data/workspace/mika-platform' --include='*.md' .   →  28 fichiers
```

27 sur 28 sont des `docs/plans/` et `docs/solutions/` **historiques**. Sur les 59
plans écrits depuis le 2026-08-01, **un seul** cite un chemin hôte
(`2026-09-02-001-fix-2141-sandbox-gitdir-bind-plan.md`, 2 mentions — et il parle
précisément du bind, donc le chemin hôte y est le sujet, pas une instruction).

**Conséquence.** L'hypothèse de tête du ticket — « les consignes que le pilote reçoit
citent constamment des chemins absolus » — **n'est pas confirmée sur la surface
chargée automatiquement**. Les consignes livrées sont déjà relatives au worktree.
La piste 1 (« réécrire les chemins dans ce que le pilote reçoit ») n'a, aujourd'hui,
presque aucun texte à réécrire.

### M3 — Correction de prémisse : le squelette existe, et il ment

Le corps du ticket (2026-08-30) affirme :

> `/data/workspace/mika-platform/` **n'existe pas** à l'intérieur.

**C'était vrai le 30/08. Ce ne l'est plus.** Depuis mika#2146, les binds du gitdir
créent leurs répertoires intermédiaires. Sonde `bwrap` réelle, jeu de binds de
`main`, exécutée aujourd'hui :

```
--- /data/workspace/mika-platform ---
drwx------ .claude
drwx------ mika

--- /data/workspace/mika-platform/mika ---
drwx------ .git          ← et rien d'autre
```

`ls /data/workspace/mika-platform/mika` ne rend plus `ENOENT`. Il rend **un
répertoire contenant uniquement `.git`** : la forme exacte d'un dépôt dont le
checkout aurait disparu.

C'est plus dangereux que le silence du 30/08. Un non-répertoire ne répond pas ; ce
squelette **répond, et sa réponse est fausse**. Et c'est très exactement la
conclusion qu'ont tirée les deux sessions du premier commentaire (`2b74a8c6`,
`9f144402`), toutes deux terminées en réclamant de *« remonter/rétablir le dépôt git
backing du worktree »*. Elles n'ont pas halluciné : elles ont lu le squelette et en
ont tiré l'inférence raisonnable.

**Le mécanisme du ticket est donc intact, mais son moteur a changé de nature :** ce
n'était pas « le pilote cherche dans le vide », c'est désormais « le pilote reçoit
une réponse plausible et fausse ». Le remède change avec lui.

### M4 — Le pilote n'est jamais situé

`dispatch-lib.sh:2167` lance `claude-pilot --command /mika --cwd $WORKTREE_DIR -- "$PROMPT"`
avec `PROMPT="mika#2108"`. Aucune ligne, nulle part, ne dit au pilote qu'il est dans
un namespace, ni où sa racine se trouve, ni que ce qu'il voit sous
`/data/workspace/mika-platform/` est un squelette et non le workspace.

Le point d'injection modèle existe déjà et fait précédent :
`claude-pilot/src/claude_pilot/agent.py:570` `_system_prompt_with_hint()` appose
`DENIED_BASH_PATTERNS_HINT` (`tier1.py:299`) au préréglage Claude Code.

---

## Ce que ce plan livre

### Phase 1 — Figer le recensement et le garder vrai

La mesure M1 est vraie aujourd'hui. Rien ne la maintient. Un `CLAUDE.md` ajouté
demain sous `crates/` peut réintroduire un chemin hôte dans la surface
automatiquement chargée sans que personne ne le voie.

1. Écrire `docs/solutions/best-practices/` — un document de recensement qui porte
   les tableaux M1/M2, **la commande exacte qui les a produits**, et la distinction
   *chargé automatiquement* / *lu à la demande*.
2. Ajouter `scripts/check-pilot-instruction-paths.sh` : échoue si un fichier de la
   surface **chargée automatiquement** (tout `CLAUDE.md` du dépôt, tout
   `.claude/commands/*.md`) contient `/data/workspace/mika-platform`, avec une
   liste d'exemptions nommées et justifiées (aujourd'hui : la seule ligne
   `crates/mika-agent/CLAUDE.md` § `[kg].docs_roots`).
3. Annoter cette exemption sur place, en une ligne : ces chemins décrivent la
   configuration d'un agent **côté hôte** et ne sont pas résolvables depuis un bac
   à sable de pilote.

Les 27 fichiers historiques de `docs/` **ne sont pas réécrits** — voir Hors périmètre.

### Phase 2 — Situer le pilote (piste 2)

`dispatch-lib.sh` ajoute au `PROMPT` un bloc d'orientation borné, **en suffixe**,
après la référence de ticket. Il énonce trois faits, pas plus :

- la racine de travail est `$WORKTREE_DIR`, et c'est le cwd ;
- le workspace de l'hôte **n'est pas monté** ; ce qui est visible sous
  `/data/workspace/mika-platform/` est un squelette de points de montage — les
  autres sous-dépôts et les autres worktrees n'y sont pas ;
- le checkout du dépôt n'existe qu'à la racine de travail : y chercher ailleurs ne
  rendra rien d'utile.

**Pourquoi dans `dispatch-lib.sh` et non dans le `system_prompt` de claude-pilot.**
Les deux emplacements sont défendables ; celui-ci est meilleur pour une raison de
fond et une de coût.

*Fond :* les faits d'orientation sont **propres à la dispatche** — quel worktree,
quel dépôt, quels binds. `dispatch-lib.sh` les connaît ; une constante statique dans
`tier1.py` ne peut énoncer que du générique et ne pourra jamais nommer la racine
réelle. Un message qui dit « ta racine est *ici* » vaut mieux qu'un message qui dit
« tu as une racine ».

*Coût :* la correction reste mono-dépôt. Aucune PR compagnon sur `claude-pilot-py`,
aucun `uv tool install` à séquencer avant que le correctif prenne effet.

**Suffixe, jamais préfixe — et c'est une contrainte, pas un goût.** Le `PROMPT` est
passé en argument à `/mika` (`dispatch-lib.sh:2167`), et sa référence de ticket est
analysée par découpage sur son premier `#` (`dispatch-lib.sh:1794-1795`) puis
re-analysée par `/mika` lui-même. Un bloc placé **avant** `mika#2108` déplace ce
premier `#` et casse les deux analyses. La forme correcte existe déjà dans le
fichier : `dispatch-lib.sh:2077` appose `ITERATION CONTEXT` en suffixe, après la
référence, séparé par une ligne vide. Le bloc d'orientation prend exactement cette
forme.

Le bloc est **borné à 12 lignes**. Il entre en concurrence d'attention avec la
référence de ticket ; un préambule qui enfle déplace le problème au lieu de le
résoudre.

### Phase 3 — Faire dire la vérité au squelette (piste 3)

Le squelette de M3 répond déjà. Il faut qu'il réponde juste.

Matérialiser un fichier `README-SANDBOX` **dans** le squelette, via le mécanisme
`--ro-bind-data` déjà présent pour le canal de secrets (`dispatch-lib.sh:993`) :

- `/data/workspace/mika-platform/README-SANDBOX`
- `/data/workspace/mika-platform/<repo>/README-SANDBOX`

Contenu : les trois mêmes faits que le bloc d'orientation, plus la racine de travail
en toutes lettres. Le `ls` qui rend aujourd'hui « un dépôt sans checkout » rendra
« un point de montage, et voici où est le vrai travail ».

**Ceci n'élargit aucun bind.** `--ro-bind-data` matérialise un contenu fourni sur un
descripteur de fichier ; il ne donne accès à rien de l'hôte. La surface passe de
*n* répertoires visibles à *n* répertoires visibles **plus deux fichiers de texte
que nous écrivons nous-mêmes*.

**L'ordre est porteur.** Les deux `--ro-bind-data` doivent venir **après** les binds
du gitdir, dont ils réutilisent les répertoires intermédiaires. Placés avant, bwrap
les matérialiserait puis un bind de répertoire les recouvrirait. C'est la même
règle d'ordre déjà documentée à `dispatch-lib.sh:596-597`.

### Phase 4 — Contrôle négatif

Le commentaire du 2026-09-03 pose la contrainte : *« Toute proposition d'élargir les
binds doit venir avec son contrôle négatif. »* Ce plan n'élargit aucun bind, ce qui
ne dispense pas de le prouver.

Un test lancé dans un `bwrap` **réel** avec le jeu d'arguments produit par la
Phase 3 vérifie, dans le même appel, que restent inaccessibles :

- `/data/workspace/mika-platform/mika/crates` (le checkout du dépôt parent) ;
- tout autre worktree sous `.claude/worktrees/` que le sien ;
- tout autre sous-dépôt (`mika-cloud/`, `mika-skills/`, `claude-pilot/`) ;
- `~/.ssh`, `~/.config/gh`, `~/.gitconfig`.

Le test porte **les deux contrôles dans le même appel** : il affirme aussi que
`README-SANDBOX` *est* lisible et que le worktree *est* accessible en écriture. Une
sonde qui ne montre que des absences ne distingue pas « correctement confiné » de
« bwrap n'a pas démarré ».

---

## Critères d'acceptation

**AC1 — Le recensement est écrit et reproductible.** Un document sous
`docs/solutions/` porte les tableaux M1 et M2, la commande exacte qui les produit,
et la distinction *chargé automatiquement* / *lu à la demande*. Un lecteur peut
relancer la commande et retrouver les mêmes nombres.

**AC2 — Une garde maintient le recensement vrai.**
`scripts/check-pilot-instruction-paths.sh` sort non-zéro si un `CLAUDE.md` ou un
`.claude/commands/*.md` du dépôt introduit `/data/workspace/mika-platform` hors des
exemptions nommées. La garde est exécutée par la CI. Elle rend **vert sur `main`
d'aujourd'hui** et **rouge** sur un fichier de test qui réintroduit un chemin hôte —
les deux sens sont vérifiés.

**AC3 — Le pilote est situé, sans casser l'analyse de la référence.** Le `PROMPT`
composé par `dispatch-lib.sh` **commence toujours par `<repo>#<n>`** et porte
ensuite, en suffixe, un bloc d'orientation de 12 lignes au plus qui nomme
`$WORKTREE_DIR` en toutes lettres et énonce que le workspace hôte n'est pas monté.
Un test sur la composition du prompt vérifie les trois choses dans le même appel :
(a) le prompt commence par la référence de ticket exacte, (b) il contient la racine
réelle du worktree, (c) il contient l'énoncé de non-montage. Vérifier seulement
qu'un bloc existe laisserait passer la régression de préfixe.

**AC4 — Le squelette dit la vérité.** Dans un `bwrap` réel,
`cat /data/workspace/mika-platform/README-SANDBOX` et
`cat /data/workspace/mika-platform/mika/README-SANDBOX` rendent chacun un texte qui
nomme la racine de travail et déclare le workspace hôte non monté.

**AC5 — Aucun bind n'est élargi.** Le diff n'ajoute aucun `--bind` ni `--ro-bind`
de répertoire. Les seuls ajouts d'arguments bwrap sont deux `--ro-bind-data`. Cette
vérification est mécanique : `git diff` sur `dispatch-lib.sh` filtré sur les lignes
ajoutées commençant par `--bind`/`--ro-bind` doit être vide.

**AC6 — Le contrôle négatif passe, dans les deux sens.** Le test de la Phase 4
rend vert : les six surfaces listées sont inaccessibles **et** `README-SANDBOX` est
lisible **et** le worktree est accessible en écriture, le tout dans le même appel
`bwrap`.

**AC7 — L'exemption est annotée.** `crates/mika-agent/CLAUDE.md` § `[kg].docs_roots`
porte une ligne indiquant que ces chemins décrivent une configuration côté hôte et
ne sont pas résolvables dans un bac à sable de pilote.

---

## Hors périmètre

- **Le montage du gitdir.** Clos par mika#2146. Ce plan ne le rouvre pas et ne le
  modifie pas.
- **La létalité de la forme large** (cpp#128) et le veto de chaîne. Ce ticket
  supprime une *raison* d'aller vers la forme large ; il ne touche pas au garde.
- **Le garde-fou d'inactivité** (cpp#147, mergé le 2026-09-03).
- **Élargir les binds** à d'autres dépôts, d'autres worktrees, ou au checkout du
  dépôt parent. L'invariant établi par mika#2146 ne se relâche pas ici.
- **Réécrire les 27 fichiers historiques de `docs/plans/` et `docs/solutions/`.**
  Ce sont des comptes rendus datés ; les chemins hôte qu'ils citent étaient les
  chemins réels au moment où ils ont été écrits. Les réécrire falsifierait le
  registre pour un gain nul — ils ne sont pas chargés automatiquement, et un pilote
  qui en ouvre un le fait délibérément, muni du contexte. La garde de l'AC2 protège
  la surface qui compte ; elle ne s'applique pas à celle-ci, et c'est délibéré.

---

## Risques et limites, nommés

**Le préambule est du prompt : il réduit un taux, il ne ferme pas une classe.**
C'est la même limite que porte déjà `DENIED_BASH_PATTERNS_HINT` dans sa propre note
de clôture honnête (`tier1.py:279-298`). La Phase 3 existe précisément parce que la
Phase 2 ne suffit pas : le marqueur répond à la tentative qui a quand même lieu.
Aucune des deux ne prétend fermer la classe seule ; ensemble elles couvrent le
raisonnement *et* le geste.

**Concurrence d'attention.** Un bloc d'orientation s'ajoute à un prompt dont le
contenu utile tient en une ligne. Borné à 12 lignes il informe ; à 40 il dilue. La
borne est un critère d'acceptation, pas une préférence de style.

**Comptabilité des descripteurs.** Les deux `--ro-bind-data` s'ajoutent au canal de
secrets qui utilise déjà le même mécanisme (`dispatch-lib.sh:959-993`). Les
descripteurs doivent être alloués et refermés sans collision avec ceux des secrets ;
c'est le point le plus susceptible de casser silencieusement, et il est couvert par
l'AC4 qui lit réellement les deux fichiers dans un bwrap.

**Ce plan ne prétend pas que le pilote livrera.** Trois sessions post-#2146 sont
mortes avant l'étape git, tuées par le garde-fou d'inactivité désormais corrigé. Le
prochain pilote ira plus loin que tous ses prédécesseurs. Ce plan supprime une
friction sur ce chemin ; il ne mesure pas ce qui se trouve après elle.

---

## Séquencement

Phase 1 (recensement + garde) → Phase 2 (orientation) → Phase 3 (marqueur) →
Phase 4 (contrôle négatif).

Les phases 2 et 3 sont indépendantes l'une de l'autre et peuvent être écrites dans
n'importe quel ordre, mais la Phase 4 les valide **ensemble** : c'est le même bwrap
qui doit montrer le marqueur présent et les six surfaces absentes.
