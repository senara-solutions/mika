# Plan : rendre git utilisable dans le bac à sable du pilote sans rouvrir `/data/workspace` (mika#2141)

**Ticket :** mika issue#2141 — `fix(dispatch,containment): le bac à sable ne monte que $WORKTREE_DIR, donc .git pointe hors du namespace — aucun pilote ne peut commiter depuis le 2026-08-04`
**Labels :** `bug`, `p1-important` *(le corps du ticket revendique **p0** — voir « Écart d'étiquette » plus bas ; à trancher par l'opérateur, sans effet sur le contenu du plan)*
**Type :** issue (bug — casseur de boucle)
**Palier de priorité :** Tier 1 — *casse la boucle*. Depuis le 2026-08-04 aucun pilote n'a pu livrer ; tout ce que la boucle a produit est passé par le filet `wip-rescue` (mika#1282).

---

## Problème

`skills/bundled/_shared/dispatch-lib.sh:639` (branche Phase 2b) et `:717` (repli Phase 2a) montent le worktree et rien d'autre :

```bash
--bind "$WORKTREE_DIR" "$WORKTREE_DIR" \
```

Le `.git` d'un worktree git lié n'est pas un répertoire mais un fichier contenant un chemin absolu vers le dépôt parent :

```
$ cat <worktree>/.git
gitdir: /data/workspace/mika-platform/mika/.git/worktrees/mika24
```

Ce chemin est hors du bind. Il n'existe pas dans le namespace. Git ne trouve ni gitdir de rattachement, ni `commondir` (`../..`), ni store d'objets.

## Mesures — ce qui a été exécuté, pas déduit

Toutes les lignes ci-dessous proviennent de `bwrap` réellement lancé le 2026-09-02, avec le jeu de binds de `main` puis avec chaque forme candidate. Aucune n'est une lecture d'argv.

### M1 — reproduction du défaut (jeu de binds actuel)

```
rev-parse: fatal: not a git repository: /data/workspace/mika-platform/mika/.git/worktrees/mika25
status:   fatal: not a git repository: …
log:      fatal: not a git repository: …
add:      fatal: not a git repository: …
commit:   fatal: not a git repository: …
```

Six commandes sur six échouent. Le diagnostic du ticket est confirmé à la ligne.

### M2 — le jeu minimal qui répare la lecture *et* l'écriture

En ajoutant `worktrees/<name>` rw + `objects` rw + `refs` :

```
rev-parse: /data/workspace/…/probe-2141-scratch/mika
status:   ## probe/2141/scratch
log:      15795b66 chore(deps): bump rand from 0.9.5 to 0.10.2 (#2005)
commit:   (succès)
after:    724e694e probe
```

### M3 — `config` du dépôt parent est **obligatoire**, contrairement à ce que M2 laisse croire

Sans `config`, git fonctionne mais le dépôt n'a plus de remote — `push` devient impossible :

```
sans config : remote -v : []          get-url : error: No such remote 'origin'
avec config : get-url  : git@github.com:senara-solutions/mika.git
```

C'est le piège de ce ticket : un correctif validé sur `status`/`commit` seuls passe le test et laisse `push` mort.

### M4 — deuxième blocage indépendant : le remote est en SSH

Le remote résolu est `git@github.com:senara-solutions/mika.git`. Or le bac à sable ne monte ni `~/.ssh` ni `$SSH_AUTH_SOCK` — délibérément, per le modèle de menace. `git push` ne peut donc pas aboutir en SSH, **même une fois le gitdir monté**.

Le design visait pourtant déjà git-over-HTTPS : `mika-pilot-github-auth-addon.py:16` injecte `Authorization: Basic` sur `github.com` « pour git smart-HTTP push/fetch », et `$_PILOT_SECRET_PROLOGUE` exporte déjà `GIT_SSL_CAINFO`. **Il manque uniquement la réécriture d'URL.** Vérifiée dans le bac à sable :

```
ls-remote --get-url                        : git@github.com:senara-solutions/mika.git
+ GIT_CONFIG_COUNT=1 url.…insteadOf        : https://github.com/senara-solutions/mika.git
```

`GIT_CONFIG_COUNT`/`KEY_0`/`VALUE_0` conviennent : pas d'écriture de fichier, pas de secret dans l'argv (AC3 intact).

### M5 — `refs` en rw entier n'est pas nécessaire, et il est dangereux

Avec `refs` en **ro** plus le seul répertoire de la branche en rw, `commit` et le reflog fonctionnent, et la suppression d'une branche d'un autre worktree échoue :

```
commit :                  (succès)
head   :                  01d29c83 probe3
reflog :                  01d29c83 HEAD@{0}: commit: probe3
supprimer refs/heads/main : error: cannot lock ref 'refs/heads/main':
                            Unable to create '…/refs/heads/main.lock': Read-only file system
```

C'est strictement plus étroit que « `refs` en rw », pour la même fonction.

### M6 — contrôle négatif (AC2), sous la forme retenue

```
autre worktree ls    : No such file or directory
autre worktree git   : fatal: cannot change to '…' : No such file or directory
autre dépôt (cloud)  : No such file or directory
racine du méta-dépôt : mika          ← rien d'autre
gitdir des autres wt : mika25        ← seulement le sien
```

`bwrap` matérialise les répertoires intermédiaires vides : monter `worktrees/<name>` **ne** révèle **pas** `worktrees/*`.

### M7 — `git fetch` exige `refs/remotes` en écriture

```
update-ref refs/remotes/origin/probe-2141 : cannot lock … Read-only file system
```

`fetch` (donc tout rebase sur `main` depuis le bac à sable) échoue tant que `refs/remotes` reste en ro.

### M8 — deux cas limites que l'implémentation doit traiter, pas supposer

- **Refs empaquetées.** `packed-refs` du dépôt parent contient **316** refs. Le répertoire `refs/heads/<type>/<issue>/` n'existe donc pas forcément au moment du dispatch : `bwrap --bind` exige que la source existe. Le dispatcher doit le créer côté hôte avant de construire l'argv.
- **Branches sans slash.** Le dépôt en porte (`main`, `feat-2066-binaries-report-commit-stamp`, `pr-1509-review`…). Pour celles-là `dirname(refs/heads/$BRANCH)` vaut `refs/heads` : le bind étroit dégénère en rw sur **toutes** les têtes. `scripts/derive-branch-name` produit toujours `<type>/<issue>/<slug>`, mais le dispatcher ne doit pas élargir en silence si la branche arrive d'ailleurs.

### M9 — pourquoi un mois sans le voir : le canary teste la mauvaise chose

`scripts/canary-pilot-containment:342` est l'unique vérification git de la suite « must-work » :

```bash
git --version 2>&1 | head -1 || echo "FAIL: git missing"
```

C'est une sonde de **présence de binaire**. Elle réussit dans un namespace où aucune opération de dépôt ne marche. La suite adversariale existait, tournait, et ne pouvait structurellement pas falsifier ce défaut — c'est précisément le trou que l'AC4 nomme.

---

## Décision

**Forme A du ticket (binder le strict nécessaire), dans sa variante la plus étroite mesurée (M5).** La forme B (dépôt autonome dans `$WORKTREE_DIR`) est écartée : elle impose un clone ou une réécriture de gitdir par dispatch, casse la relation worktree↔parent dont dépend la récupération post-vol (mika#1282, protégée par AC6), et n'achète rien que M5 n'obtienne déjà — M6 montre que la forme A ne révèle pas les autres worktrees.

Jeu de binds retenu, dérivé par chemin, jamais `/data/workspace` en bloc :

| Chemin | Mode | Pourquoi | Mesure |
|---|---|---|---|
| `$PARENT_GIT/worktrees/<name>` | rw | `HEAD`, `index`, `commondir`, refs de ce worktree | M2 |
| `$PARENT_GIT/objects` | rw | `commit` écrit des objets | M2 |
| `$PARENT_GIT/refs` | **ro** | lecture des refs ; interdit la suppression | M5 |
| `$PARENT_GIT/refs/heads/<dirname(branche)>` | rw | mise à jour de la tête de *cette* branche | M5 |
| `$PARENT_GIT/refs/remotes` | rw | `fetch` met à jour les refs de suivi | M7 |
| `$PARENT_GIT/logs/refs/heads/<dirname(branche)>` | rw | reflog | M5 |
| `$PARENT_GIT/config` | **ro** | résolution du remote | M3 |
| `$PARENT_GIT/packed-refs` | ro | 316 refs y vivent | M8 |

Plus la réécriture d'URL par `GIT_CONFIG_*` (M4).

### Surface résiduelle — à assumer explicitement

Partager `objects` et lire `refs` donne au pilote la lecture du **contenu de toutes les branches de `mika`** (`git show <autre-branche>:fichier`). C'est inhérent à tout partage de store d'objets, forme B incluse dès lors que le clone est complet. Ce que l'AC2 ferme — l'accès *fichier* aux autres worktrees et aux autres dépôts — reste fermé (M6). La distinction est nommée ici pour être arbitrée, pas contournée.

### Écart d'étiquette

Le corps revendique `p0 — casseur de boucle, le plus haut` ; les étiquettes portent `p1-important`. Aucune conséquence sur le plan ; l'ordre de tirage appartient à l'opérateur.

---

## Phases

### Phase 1 — Dériver les chemins gitdir côté hôte

1. Dans `dispatch-lib.sh`, ajouter une fonction `_pilot_gitdir_bind_args` qui, à partir de `$WORKTREE_DIR` et `$BRANCH`, émet le tableau d'arguments `bwrap` du tableau ci-dessus.
   - Lire `$WORKTREE_DIR/.git`, en extraire `gitdir:` → `$WT_GITDIR`.
   - `$PARENT_GIT = $(cd "$WT_GITDIR/$(cat "$WT_GITDIR/commondir")" && pwd)` — résoudre `commondir`, ne pas présumer `../..`.
   - Si `$WORKTREE_DIR/.git` est un **répertoire** (dépôt non-worktree, cas `mika-platform` méta-dépôt), n'émettre aucun argument : le `.git` est déjà dans le bind du worktree. Le chemin doit rester correct dans les deux formes.
2. `mkdir -p` côté hôte, avant la construction de l'argv (M8) :
   `$PARENT_GIT/refs/heads/<dirname>` et `$PARENT_GIT/logs/refs/heads/<dirname>`.
3. Garde sur les branches sans slash (M8) : si `dirname(refs/heads/$BRANCH)` vaut exactement `refs/heads`, **abandonner le dispatch** avec un message nommant la branche, plutôt que d'élargir le bind en silence. Élargir serait donner rw sur toutes les têtes du dépôt — l'inverse de l'AC2.

### Phase 2 — Câbler les deux constructions `bwrap`

4. Insérer `"${gitdir_bind_args[@]}"` **après** `--bind "$WORKTREE_DIR" "$WORKTREE_DIR"` dans la branche Phase 2b (`:639`) **et** dans le repli Phase 2a (`:717`). Les deux, sans exception : le repli est le chemin dégradé, pas un chemin mort.
5. Ajouter à `setenv_args` la réécriture d'URL (M4) :
   `GIT_CONFIG_COUNT=1`, `GIT_CONFIG_KEY_0=url.https://github.com/.insteadOf`, `GIT_CONFIG_VALUE_0=git@github.com:`.
   Si `GIT_CONFIG_COUNT` est déjà positionné ailleurs, composer plutôt qu'écraser.
6. Ajouter `GIT_TERMINAL_PROMPT=0`. Sans lui, un `push` dont l'injection host-side échoue attend une saisie sur un terminal absent — la classe de blocage silencieux exactement décrite au § « Pourquoi ça a été si long à voir ».
7. Mettre à jour l'en-tête du modèle de menace (`:60-83`). La ligne `NOT bound … /data/workspace outside the branch worktree` devient fausse telle quelle : la réécrire pour dire ce qui est monté, pourquoi, et ce qui reste fermé. Une doctrine périmée dans un en-tête est ce qui a fait passer ce défaut pour une décision.

### Phase 3 — Les tests qui peuvent falsifier (AC4)

8. `scripts/canary-pilot-containment` — remplacer `git --version` (`:342`) par une suite de dépôt réellement exécutée dans le confinement : `rev-parse --show-toplevel`, `status -sb`, `log --oneline -1`, `add`, `commit` sur un fichier jetable, puis `ls-remote --get-url` (doit rendre une URL **https://**, cf. M4). `git --version` reste, mais cesse d'être la preuve.
9. Ajouter au bloc « must-fail » du canary les contrôles négatifs de M6, chacun exigeant l'échec : `ls` d'un autre worktree, `git -C` d'un autre worktree, `ls` de `mika-cloud`, et `update-ref -d refs/heads/main` (doit rendre `Read-only file system`, M5).
10. Ajouter `tests/test_sandbox_git_usable.sh` : lance un vrai `bwrap` via `_run_pilot_sandboxed` sur un worktree jetable, exige la suite de l'étape 8 verte et celle de l'étape 9 rouge, puis détruit le worktree. **Un test qui construit l'argv sans lancer le bac à sable est explicitement hors contrat** — c'est la forme qui a laissé passer le défaut.
11. Ajouter au canary une assertion AC3 : depuis le bac à sable, `~/.gitconfig`, `~/.config/gh` et `~/.ssh` restent absents, et `$PARENT_GIT/config` est en lecture seule et ne contient aucune URL à jeton.

### Phase 4 — Preuve de bout en bout (AC5)

12. Déployer (`make -C mika deploy`) puis dispatcher un ticket réel via l'étiquette `ready`.
13. Vérifier que la PR produite porte `origin:loop` **sans** `wip-rescue` :
    `gh pr list --repo senara-solutions/mika --state all --label origin:loop --json number,labels,createdAt`.
    C'est le seul critère qui sépare « le pilote a commité » de « le secours a rattrapé ».
14. Vérifier que la récupération post-vol (mika#1282) est intacte (AC6) : aucun de ses points de déclenchement n'est touché par le diff. Le prouver par `git diff` sur les chemins de mika#1282, pas par lecture.

---

## Rattachement aux critères d'acceptation

| AC | Traité par | Preuve exigée |
|---|---|---|
| AC1 — git utilisable dans le bac à sable | Phases 1–2 (étapes 1–6) | Étape 8 exécutée dans le confinement réel ; `push` couvert par M4 (URL https), pas seulement par le montage |
| AC2 — contrôle négatif non négociable | Étape 3 (garde), étape 9 | Quatre tentatives d'accès exigeant l'échec, dont `update-ref -d` |
| AC3 — aucun secret par le nouveau bind | `config` en ro (M3), `GIT_CONFIG_*` sans secret (M4), étape 11 | Assertion d'absence de `~/.gitconfig`, `~/.config/gh`, `~/.ssh` |
| AC4 — test dans le confinement réel | Étapes 8–10 | `test_sandbox_git_usable.sh` lance `bwrap` ; la construction d'argv seule est hors contrat |
| AC5 — preuve de bout en bout | Étapes 12–13 | PR `origin:loop` sans `wip-rescue` |
| AC6 — mika#1282 intact | Étape 14 | `git diff` sur les chemins de la récupération post-vol |

## Hors périmètre

Repris du ticket, sans extension : le statut de sortie `Success` sur session bloquée (cpp#144), la létalité d'`idleTimeout` (mika#2125), le confinement réseau Phase 2b et l'injection de jeton host-side (inchangés), et toute réduction du modèle de menace au-delà du minimum nommé en AC2.

**Ajouté par ce plan, à ne pas traiter ici :** la réécriture d'URL de l'étape 5 rend `push` *possible*. Si un dispatch réel révèle un défaut dans l'injection host-side de `mika-pilot-github-auth-addon.py`, c'est un ticket distinct — ce plan ne touche pas l'addon.

## Risques

- **Le correctif répare `status` et laisse `push` mort.** C'est le mode d'échec le plus probable, et M3+M4 montrent qu'il est atteignable sans s'en apercevoir. L'étape 8 exige `ls-remote --get-url` en `https://` pour cette raison.
- **Le bind étroit dégénère.** M8 : branche sans slash, ou refs empaquetées. L'étape 3 abandonne plutôt que d'élargir ; l'étape 2 crée le répertoire manquant.
- **Le repli Phase 2a est oublié.** Deux constructions `bwrap` existent ; ne corriger que `:639` laisse le chemin dégradé cassé. L'étape 4 les nomme toutes les deux.

## Références

- `skills/bundled/_shared/dispatch-lib.sh:639`, `:717` — le bind ; `:60-83` — l'en-tête à corriger ; `:177` — `$_PILOT_SECRET_PROLOGUE` exportant déjà `GIT_SSL_CAINFO`
- `scripts/canary-pilot-containment:342` — la sonde `git --version` qui ne pouvait pas falsifier (M9)
- `scripts/mika-pilot-github-auth-addon.py:15-16` — l'injection host-side visant `github.com` en smart-HTTP
- `e4f24677` / PR#1894 — l'introduction du confinement, 2026-08-04
- mika#1282 (récupération post-vol, AC6), mika#2090 (étiquette `origin:*`), mika#2056 (jeton host-side), cpp#144, mika#2125

---

## Registre de grooming — ce que l'architecte a signé, et ce qu'il n'a pas tranché

Consigné après le commit du plan, pour que la lignée reste lisible : le commit précédent porte l'état signé ; celui-ci porte le compte rendu de la signature.

**Premier appel** (session `9c974b42-ebed-4d62-a876-d9caf7621608`) — rejeté par le garde moteur `required_review_anchor_prefixes` : `Disposition-Withheld: REVIEW-ANCHOR-MISSING`. Les trois ancres citaient le **corps du ticket GitHub**, que l'architecte n'avait pas reçu, et non le brief. La réponse mentionnait en outre une « Phase 1.5 ajoutant la reconnaissance des trois formes de verdict », absente du plan comme du brief — contamination d'un autre sujet. Le garde a fait exactement son travail.

**Second appel** (session `d61ac474-190f-49ab-b10c-9a61d3dfe8cc`), même brief plus un rappel du contrat d'ancrage — `Disposition: READY`, ancres cette fois verbatim et sur trois régions distinctes.

**Ce que ce READY n'atteste pas.** La réponse fait environ 900 octets sur un brief d'environ 14 Ko, elle restitue le tableau de décision du brief sans le contredire ni l'étendre, elle ouvre par « toutes les trouvailles de la première passe sont résolues » alors que la première passe n'a produit aucune trouvaille — et **aucune des cinq incertitudes numérotées du brief n'est arbitrée**. Le garde vérifie l'attestation mécanique, pas la substance.

**Conséquence pour l'implémenteur : les cinq questions restent ouvertes.** Elles ne sont pas tranchées par l'architecte ; elles sont à traiter comme des décisions de conception à prendre pendant l'implémentation, avec mesure à l'appui.

1. **La surface résiduelle satisfait-elle l'AC2 ?** L'accès *fichier* aux autres worktrees est fermé (M6), mais `git show <autre-branche>:fichier` réussira. Arbitrage au jugement, non confirmé.
2. **La garde « branche sans slash » abandonne le dispatch.** Fail-closed volontaire, mais il suppose que seul le chemin canonique `derive-branch-name` est en usage — supposition non prouvée.
3. **`refs/remotes` en rw entier** n'a pas été resserré (`refs/remotes/origin` seul ?). Seul bind du tableau non minimisé.
4. **`objects` en rw** : la variante `objects` ro + `objects/info/alternates` a été écartée par raisonnement, pas par mesure. Si le raisonnement AC6 est faux, elle serait strictement meilleure.
5. **Ce que `push` exige réellement** n'a pas été mesuré — aucun `push` réel n'a été exécuté. C'est le trou le plus large de la preuve, et c'est précisément l'objet de l'AC5.

Cette classe de défaut (verdict mécaniquement attesté, substantiellement vide) est celle que suit mika#2037 ; cet échange en est une occurrence datée, pas un nouveau ticket.
