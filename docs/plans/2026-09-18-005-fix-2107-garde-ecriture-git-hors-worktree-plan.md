# mika#2107 — La garde entre « une session tape une commande git » et « elle s'exécute dans l'arbre partagé »

> Plan de grooming — `senara-solutions/mika#2107`
> Mesures prises le 2026-09-18 depuis une dispatche `dev-groom` (sandbox bwrap), git 2.53.0, machine `gentux`.
> Re-mesurées le 2026-09-18 au soir depuis une seconde dispatche (re-groom après non-convergence
> du dispatch précédent) : les cinq constats du § 2 tiennent, et trois d'entre eux se renforcent
> d'une mesure directe — elles sont citées à leur place.

---

## 1. Le fait que porte le ticket

Quatre occurrences en vingt-six heures, deux répertoires, trois mécanismes :

| # | quand | répertoire | mécanisme | conséquence |
|---|---|---|---|---|
| 1 | 30/08 matin | `mika/` | `git reset --hard` | travail indexé d'une autre session effacé |
| 2 | 30/08 16:54 | `mika/` | `git checkout <branche>` | checkout de déploiement détaché 3 h 15 |
| 3 | 30/08 20:56 | `claude-pilot/` | fichiers indexés (`git add` / `stash` / `-C`) | **12 min de dispatches sur un classifieur de permissions non revu** |
| 4 | 31/08 09:31 | `claude-pilot/` | `git checkout <sha>` | artefact déployé hors `main` |

Cause unique, écrite par le ticket : **la session croit être dans son worktree et ne l'est
pas.** Le worktree est créé correctement ; c'est le **répertoire courant au moment de la
commande** qui dérive, et rien ne le recoupe.

Le ticket a déjà réfuté la réponse documentaire par l'expérience : `docs/operator/claude-pilot-deploy-path.md`
a été écrit après l'occurrence 3, et l'occurrence 4 s'est produite **onze heures plus tard sur
le même répertoire**. Ce plan ne rouvre pas ce débat.

---

## 2. Ce que la mesure déplace — cinq constats, chacun vérifiable

Le ticket propose deux surfaces « sans préjuger de la bonne ». Les mesures ci-dessous
disqualifient la surface 2, montrent que la surface 1 est **déjà livrée** pour la population
qu'elle visait, et désignent une troisième surface que le ticket n'avait pas.

### M-1. La dispatche est déjà contenue — plus fortement qu'une marque de worktree

`/proc/self/mountinfo`, lu depuis l'intérieur d'une dispatche `dev-groom`, 2026-09-18 :

```
/data/workspace/mika-platform/mika/.git/worktrees/mika23      rw
/data/workspace/mika-platform/mika/.git/objects                rw
/data/workspace/mika-platform/mika/.git/refs                   ro
/data/workspace/mika-platform/mika/.git/config                 ro
/data/workspace/mika-platform/mika/.git/packed-refs            ro
/data/workspace/mika-platform/mika/.git/refs/heads/fix/2107    rw   (imbriqué)
/data/workspace/mika-platform/claude-pilot/src                 ro
```

Et ce qui n'y figure pas est le fait :

- **L'arbre de travail de `mika/` n'est pas monté du tout** (`ls` du répertoire ne rend que `.git`).
- **`.git/HEAD` du checkout principal n'est pas monté** — occurrences 2 et 4 inatteignables.
- **`claude-pilot/src` est `ro`** — occurrence 3 inatteignable, sur la surface que le ticket
  qualifie de « la plus dangereuse des deux ».

C'est le travail de mika#2141, dont `dispatch-lib.sh:720` porte le raisonnement écrit
(« WHAT IS DELIBERATELY NOT BOUND. `hooks/` — the sandbox runs no repo hooks »).
**Aucune des quatre occurrences n'est reproductible depuis une dispatche.** La surface 1 du
ticket (« le worktree du dispatch porte une marque ») demande une marque pour une population
qui n'a déjà plus accès à l'arbre partagé.

**Re-mesure directe du soir, qui ne passe par aucune lecture de `mountinfo`** — deux `ls` depuis
l'intérieur de la dispatche :

```
$ ls /data/workspace/mika-platform/
claude-pilot   mika                    # et rien d'autre ; aucun des deux n'a d'arbre de travail
$ ls /data/workspace/mika-platform/.claude/
worktrees                              # ni skills/, ni settings*.json, ni commands/
```

Le second `ls` est un fait neuf et il a un effet sur ce plan : **la définition de `/mika-spawn`
n'est pas lisible depuis une dispatche.** C'est ce qui rend M0-e (§ 4) nécessaire plutôt
qu'ornemental — la précondition du premier terme du prédicat ne peut pas être établie ici.

**Conséquence pour le périmètre :** la population résiduelle est celle des sessions
**non sandboxées** — les spawns (`/mika-spawn`, tenant `claude` nu en workspace Hyprland 4) et
l'orchestrateur. Les quatre occurrences le confirment : 1, 2 et 3 sont attribuées nommément à
des spawns par le ticket lui-même ; 4 est non attribuée.

### M-2. Les hooks `pre-checkout` et `pre-reset` n'existent pas

Liste complète des hooks de git 2.53.0 (`man githooks`) :

```
applypatch-msg   pre-applypatch   post-applypatch  pre-commit     pre-merge-commit
prepare-commit-msg  commit-msg    post-commit      pre-rebase     post-checkout
post-merge       pre-push         pre-receive      update         proc-receive
post-receive     post-update      reference-transaction            push-to-checkout
pre-auto-gc      post-rewrite     sendemail-validate  fsmonitor-watchman  post-index-change
```

Ni `pre-checkout`, ni `pre-reset`. `post-checkout` et `post-index-change` existent et
**s'exécutent après**. La surface 2 du ticket, prise à la lettre, n'est pas implémentable.

### M-3. Le seul hook qui peut refuser couvre 2 occurrences sur 4, et trop tard

`reference-transaction` est le seul hook dont un code de retour non nul en phase `prepared`
avorte la transaction (`man githooks`, section `reference-transaction`). Mais :

- il **ne se déclenche pas pour `git add`** (aucune mise à jour de référence) → occurrence 3
  non couverte ;
- `git reset --hard` **sans argument** ne déplace pas `HEAD` (il réinitialise *vers* `HEAD`),
  donc aucune transaction → occurrence 1 non couverte ;
- pour `git checkout`, git met à jour l'arbre de travail **avant** de déplacer `HEAD`. Avorter
  la transaction laisserait les fichiers basculés et `HEAD` inchangé — un état pire que
  l'occurrence qu'on prétend empêcher. *(Ordre à re-mesurer en M0-c ; il conditionne le rejet,
  pas la conclusion, qui tient déjà sur les deux points précédents.)*

**Couverture maximale d'un hook git : 2/4, et dégradante sur ces 2.**

### M-4. Les mécanismes de hook de ce dépôt ont un taux d'installation de zéro

**Il y en a deux, pas un** — et aucun n'est installé. C'est le constat le plus dur du plan,
parce qu'il disqualifie toute une famille de réponses d'un seul coup.

`.githooks/pre-commit` est **suivi dans le dépôt** et `CONTRIBUTING.md:18` demande
`git config core.hooksPath .githooks`. `lefthook.yml` est suivi lui aussi et déclare un bloc
`pre-commit` (`rust-fmt`, `rust-clippy`, `no-secrets`, `no-large-files`). Mesuré ici :

```
$ git config --show-origin --get-all core.hooksPath
(exit 1 — non défini dans aucune portée)
$ ls .githooks/
pre-commit   pre-commit.old          # deux fichiers, dont un vestige non nettoyé
```

Deux mécanismes concurrents, tous deux suivis, tous deux documentés, tous deux inertes — et un
`.old` que personne n'a supprimé, ce qui est la trace d'un arbre que rien n'exerce.

Confirmation indépendante et datée d'hier, dans `dispatch-lib.sh:3152-3153` (mika#2348) :

> « Le gate `lefthook` `rust-fmt` […] **N'EST PAS INSTALLÉ sur la machine de dispatch (pas de
> `.git/hooks/`, pas de `core.hooksPath` dans aucune portée), donc il n'a jamais tourné.** »

Un ticket dont la thèse est « la prose ne ferme pas cette classe » ne peut pas être clos par un
mécanisme dont l'activation **est elle-même un geste manuel documenté, jamais accompli**.

Et pour la dispatche, le point est doublement clos : `dispatch-lib.sh:721` — « **WHAT IS
DELIBERATELY NOT BOUND. `hooks/`** — the sandbox runs no repo hooks ».

### M-5. Rien n'intercepte git dans un spawn, et la place pour le faire est libre

- `~/.claude/settings.json` : `permissions.defaultMode: "auto"`, et les hooks déclarés sont
  `SessionStart`, `PreCompact`, `UserPromptSubmit`. **Aucun `PreToolUse`.** `~/.claude/hooks/`
  est vide (mesuré : répertoire présent, zéro entrée).
- Le `.claude/settings.local.json` copié dans chaque worktree par `dispatch-lib.sh:2400` porte
  `Bash(git:*)`, `Bash(git checkout:*)`, `Bash(git add:*)` — git est autorisé en bloc, sur
  138 entrées d'allow-list.
- Le dépôt `mika` ne suit **aucun** `.claude/settings.json` (`git ls-files .claude/` rend quatre
  `commands/*.md` et `claude-pilot.json`).

**Deux mesures directes sur le `settings.local.json` de ce worktree, et chacune décide d'un
point du plan :**

```
$ python3 -c "import json; d=json.load(open('.claude/settings.local.json')); print(list(d.keys()))"
['permissions']
```

1. **Il ne porte aucune clé `hooks`.** La cohabitation de M2 est donc plus qu'un choix de noms de
   fichiers distincts : les deux fichiers ne se disputent pas la même clé, et le hook posé par
   `settings.json` n'a rien à fusionner ni à écraser.
2. **`Bash(git:*)` y est présent.** C'est la mesure qui fait de M0-b le point de rupture réel et
   non une précaution de principe : dans une session de worktree, git est explicitement autorisé
   avant même que le hook ait son mot à dire. Si un `deny` de hook ne prime pas sur un `allow`
   de permissions, la garde est inerte précisément là où elle doit mordre.

Donc : dans un spawn, une commande git passe sans aucune interposition, et **le créneau
`PreToolUse` d'un `settings.json` suivi par le dépôt est libre et à coût de geste nul** — il
arrive avec le checkout, sans étape d'installation. C'est exactement ce que M-4 exige.

---

## 3. Décision d'architecture

> **La garde est un hook `PreToolUse` sur `Bash`, livré par un `.claude/settings.json` suivi dans
> le dépôt, qui refuse toute commande git mutante dont l'arbre cible n'est pas le worktree de la
> session.**

Trois propriétés, chacune adossée à une mesure ci-dessus :

1. **Elle s'interpose avant l'exécution**, donc elle couvre les quatre mécanismes mesurés —
   `add`, `reset`, `checkout`, indexation par `-C` — là où un hook git n'en voit que deux, et
   trop tard (M-2, M-3).
2. **Elle n'a pas d'étape d'installation.** Elle est dans l'arbre ; cloner ou créer un worktree
   suffit. C'est la seule forme que M-4 n'a pas déjà réfutée.
3. **Elle vise la population résiduelle** — les sessions non sandboxées — et pas celle que
   mika#2141 a déjà fermée (M-1).

### Le prédicat, et pourquoi il est formé ainsi

```
REFUSER  ssi   le project-dir de la session est un worktree lié
          ET   la commande est une invocation git
          ET   le verbe n'est PAS dans l'allow-list de lecture
          ET   l'arbre cible effectif ≠ le project-dir de la session
```

**Allow-list de lecture, pas deny-list de mutation — et c'est la mesure qui tranche.** Le ticket
nomme `checkout` et `reset` ; une deny-list bâtie sur les mécanismes connus aurait manqué
**l'occurrence 3 (`git add`) et l'occurrence 1**, soit la moitié de la population mesurée, dont
la seule qui ait réellement expédié du code non revu en production. Qui aurait classé `git add`
comme destructeur avant de le mesurer ? Donc : on énumère ce qui lit (`status`, `log`, `diff`,
`show`, `rev-parse`, `rev-list`, `ls-files`, `cat-file`, `blame`, `for-each-ref`, `describe`,
`reflog`, `shortlog`, `grep`, `branch`/`tag`/`stash`/`worktree` en forme de listage,
`remote -v`, `config --get*`), et **tout verbe non classé est refusé** quand il vise hors du
worktree. Un verbe git inconnu est précisément la forme du prochain incident.

**Le premier terme exempte l'opérateur et l'humain, par construction.** Une session dont le
project-dir est le checkout principal ou la racine de l'espace de travail — l'orchestrateur qui
exécute `/mika-platform-sync-main`, un humain dans son shell — n'est jamais dans la population.
Le prédicat encode la signature de l'accident telle que le ticket l'écrit (« la session croit
être dans son worktree »), pas « toute écriture git dans un arbre partagé », qui casserait la
maintenance légitime dès le premier tick.

**Résolution de la cible effective** : le hook reçoit le `cwd` de la session et la chaîne de
commande. Il doit honorer `cd X && git …`, `git -C X …`, `git --git-dir=…`, `GIT_DIR=… git …`,
et le cas nu (dérive du `cwd`). La cible est le `rev-parse --show-toplevel` de ce répertoire.
**Cette garde défend contre l'accident, pas contre un adversaire** — c'est écrit dans le script
et cela borne le périmètre : un chemin obfusqué n'est pas la classe mesurée, et exiger une
étanchéité de bac à sable ici serait demander à un garde-fou d'être un mur (le mur, c'est
mika#2141, et il est déjà posé).

### La garde tombe en marche — fail-open, et ce que ça coûte

Un hook `PreToolUse` s'exécute avant **chaque** appel Bash de **chaque** session enracinée dans
ce dépôt. Un script absent, non exécutable, ou rendant du JSON invalide doit donc être décidé,
pas découvert : c'est la question que ce dépôt a dû trancher explicitement à chaque garde
(`MIKA_DEPLOYMENT` → `Unknown` n'asserte rien ; la garde de budget refuse au démarrage mais pas
sur un réglage seulement sous-optimal).

> **Décision : fail-open.** Un script qui ne rend pas une décision lisible laisse passer la
> commande.

Le motif n'est pas la prudence, c'est l'asymétrie des coûts, et elle est sévère ici : un
fail-closed sur un script cassé **couche toutes les sessions du dépôt**, y compris les dispatches
et l'orchestrateur en incident, pour un défaut de garde qui ne protège qu'une population
résiduelle. Un faux négatif rouvre une classe d'accident dont on a quatre occurrences en
vingt-six heures ; un faux positif généralisé arrête la boucle. Un cas concret et non
hypothétique le rend obligatoire : **tout worktree créé sur une branche antérieure à ce
correctif ne porte pas le script**, et il n'y a aucune raison qu'une session y perde Bash.

**Ce que fail-open coûte, écrit plutôt que découvert : une garde cassée se lit exactement comme
une garde qui n'a jamais eu à firer.** C'est la classe que ce dépôt a déjà payée trois fois
(mika#2205 — un scan silencieusement inactif ; mika#2327 — un réglage code-owned jamais écrit ;
mika#2340 — une bibliothèque de skills rafraîchie en apparence). C'est ce coût, et lui seul, qui
rend la ligne d'armement `SessionStart` de M4 **porteuse et non décorative** : elle est la seule
chose qui distingue « rien à refuser » de « rien n'est armé ». Sans elle, fail-open serait un
désarmement silencieux ; avec elle, c'est un arbitrage observable.

### La garde tournera aussi dans les dispatches, et c'est sans effet utile

Le `settings.json` étant suivi, il est présent dans **chaque** worktree de dispatche : le pilote
exécutera donc le hook avant chacun de ses appels Bash. Trois conséquences, toutes nommées :

- **Aucun gain de sécurité.** mika#2141 a déjà rendu l'arbre partagé inatteignable depuis la
  sandbox (M-1). La garde y est redondante par construction, jamais nuisible.
- **Un coût par appel Bash**, qui devient le vrai budget de conception du script : sortie `allow`
  immédiate dès que le premier terme est faux, sh pur, aucun appel réseau, au plus un `rev-parse`.
- **Une écriture de journal potentiellement impossible** : `~/.mika/state/` n'est pas garanti
  accessible en écriture depuis la sandbox. L'échec d'écriture du journal **ne doit jamais
  changer la décision** — journaliser est une observation, pas un terme du prédicat. Une garde
  qui refuserait parce qu'elle n'a pas pu écrire son log serait un fail-closed déguisé, qui
  contredirait la décision ci-dessus par une porte dérobée.

### Ce qui a été écarté, et pourquoi

| Option | Motif de rejet |
|---|---|
| Hooks `pre-checkout` / `pre-reset` (surface 2 du ticket) | N'existent pas (M-2) |
| Hook `reference-transaction` | 2/4 de couverture, et après la bascule de l'arbre (M-3) |
| Marque sur le worktree de dispatch (surface 1 du ticket) | Déjà livré, et plus fortement, par mika#2141 (M-1) |
| Installation via `core.hooksPath` | Taux d'installation mesuré : zéro (M-4) |
| Hook au niveau utilisateur (`~/.claude/settings.json`) **comme remède principal** | Hors de tout dépôt, donc geste d'installation manuel — la classe que M-4 réfute. Reste possible en **commodité opérateur**, jamais comme porteur du correctif |
| Durcir la permission-policy (`[policy:deny]`) | Elle gouverne le `canUseTool` des dispatches, pas les spawns — donc pas la population résiduelle |

---

## 4. Livrables

### M0 — Sonde de faisabilité (bloquante, ~45 min)

Le correctif entier repose sur cinq propriétés — quatre du harnais Claude Code installé, une du
mode de lancement des spawns. Elles sont **vérifiées avant d'écrire la garde**, pas supposées.
M0-e demande de lancer un spawn réel, d'où l'enveloppe un peu plus large.

- **M0-a** — un hook `PreToolUse` avec `matcher: "Bash"` se déclenche bien sur l'outil Bash.
- **M0-b** — **la plus critique** : un `permissionDecision: "deny"` du hook bloque la commande
  *même quand* `permissions.allow` la couvre (`Bash(git:*)`) et que `defaultMode` vaut `auto`.
  Si c'est faux, l'approche meurt ici.
- **M0-c** — la charge utile du hook porte bien `cwd` et `tool_input.command`.
- **M0-d** — un hook déclaré dans un `settings.json` **suivi par le dépôt** s'exécute dans une
  session fraîche d'un worktree frais, et à quelles conditions de confiance.
- **M0-e** — **le project-dir d'un spawn est bien son worktree.** Lancer `/mika-spawn`, et lire
  dans la session obtenue le `cwd` que reçoit le hook ainsi que `$CLAUDE_PROJECT_DIR`.

**Pourquoi M0-e est bloquante et n'était pas dans la première rédaction.** Le premier terme du
prédicat est « le project-dir de la session est un worktree lié ». Si un spawn démarre enraciné
ailleurs — la racine de l'espace de travail, ou le checkout principal — alors ce terme est faux
pour **toute la population mesurée**, et la garde est inerte précisément sur les quatre
occurrences qui motivent le ticket, tout en paraissant livrée. Le ticket établit que les spawns
*travaillent* dans des worktrees (occurrence 3 : le spawn de cpp#128 « travaille normalement
dans `.claude/worktrees/…/claude-pilot` »), ce qui rend l'hypothèse plausible — mais
« travailler dans » n'est pas « être enraciné dans », et c'est exactement le genre d'écart que
ce ticket punit. La mesure n'est pas faisable depuis une dispatche : `/data/workspace/mika-platform/.claude/`
n'y expose que `worktrees` (M-1), donc la définition de `/mika-spawn` est hors de portée.

**Critères d'arrêt — deux, et ils ne se règlent pas au même endroit.**

- Si **M0-b** échoue, on ne bricole pas : le plan bascule sur le repli nommé — hook au niveau
  utilisateur plus `autoMode.soft_deny`, avec le coût d'installation assumé et écrit — et cela
  **change le ticket**, donc remonte à l'opérateur avant M1.
- Si **M0-e** échoue, ce n'est pas le code qui change mais **le prédicat** : le premier terme
  doit alors être reformulé sur une propriété que les spawns satisfont réellement (par exemple
  « la session n'est pas celle de l'opérateur », dont l'exemption devra être obtenue autrement
  que par l'enracinement). Remontée opérateur avant M1 également — réécrire le prédicat sans le
  dire reviendrait à changer qui la garde protège.

**Sous-produit de M0-c** : mesurer l'ordre arbre-de-travail / `HEAD` d'un `git checkout` dans un
dépôt jetable, pour clore M-3 par la mesure et non par le raisonnement.

### M1 — `scripts/guard-shared-checkout`

Exécutable POSIX, **agnostique du dépôt** (aucun chemin `mika` en dur), fonction pure de
`(project-dir, cwd, chaîne de commande)` → `allow` | `deny + motif`. Lit le JSON du hook sur
l'entrée standard, écrit la décision sur la sortie standard.

Le message de refus suit le modèle mika#1475 (`Makefile:86-98`) — **il nomme le remède et
l'échappatoire**, parce qu'un refus qui ne nomme pas sa levée est un refus qu'on contourne au
jugé :

```
REFUS: cette commande git vise /data/workspace/mika-platform/mika
       alors que la session travaille dans .../worktrees/<slug>/mika.
  Remède:      git -C "$CLAUDE_PROJECT_DIR" <commande>
  Dérogation:  MIKA_GUARD_SHARED_CHECKOUT=0 (geste explicite, journalisé)
```

### M1b — `scripts/test-guard-shared-checkout.sh`

Table-driven. **Le nom suit la convention majoritaire de `scripts/`**, mesurée :
`test-check-byte-slices.sh`, `test-check-dispatch-seats-declared.sh`,
`test-check-image-tags-immutable.sh`, `test-pr-origin-report.sh`, `test-dispatch-symmetry.sh` —
préfixe `test-`, suffixe `.sh`. (La forme `<script>-test` existe aussi, minoritaire ; on prend la
majoritaire pour que le job CI de M3 se lise comme ses voisins.)

**Les quatre occurrences mesurées sont les fixtures**, nommées et datées :
`reset --hard` nu (occ. 1), `checkout <branche>` (occ. 2), `add` via `-C` (occ. 3),
`checkout <sha>` (occ. 4).

**Contrôles négatifs, au moins aussi importants que les positifs** — la garde ne doit rien
coûter au nominal :
`git add` / `commit` / `checkout` dans le worktree de la session ; `git -C <son propre worktree>` ;
tous les verbes de l'allow-list de lecture visant l'arbre partagé ; une session dont le
project-dir **n'est pas** un worktree lié (orchestrateur, humain) ; un dépôt sans `.git`
(harnais de test de la sandbox, cf. `dispatch-lib.sh:733-737`) ; `MIKA_GUARD_SHARED_CHECKOUT=0`.

### M2 — Câblage : `.claude/settings.json` suivi

Un `settings.json` **nouveau et suivi** portant le hook `PreToolUse` vers
`$CLAUDE_PROJECT_DIR/scripts/guard-shared-checkout`. Il ne collisionne pas avec le
`settings.local.json` que `dispatch-lib.sh:2400` copie — noms de fichiers distincts, **et**
clés disjointes (M-5 : ce fichier ne porte que `permissions`, jamais `hooks`). Ce dernier est par
ailleurs exclu du staging de rescue par `RESCUE_EXCLUDE_PATHSPEC` (`dispatch-lib.sh:3104`), ce
qui n'est pas le cas du `settings.json` suivi : il est **censé** être versionné, et c'est toute
la propriété de M-4 qu'on achète.

### M3 — CI : `shared-checkout-guard-lint`

Job dédié sur le modèle de `loop-select-lint` (`.github/workflows/ci.yml:216-222`), exécutant
`M1b`. Plus un **test structurel** refusant qu'un futur `settings.json` perde le hook — la
régression ne rendrait aucune décision fausse, elle rendrait la garde absente, et toutes les
assertions comportementales resteraient vertes.

### M4 — Observabilité et anti-inertie

Deux exigences, et la seconde est celle que ce dépôt a dû réapprendre plusieurs fois
(mika#2205, mika#2327, mika#2340) : **une garde silencieusement non installée se lit exactement
comme une garde qui n'a jamais eu à firer.**

- **Chaque refus** écrit une ligne dans `~/.mika/state/shared-checkout-guard.log`
  (horodatage, project-dir, cible, commande tronquée, motif). **Régime attendu : non nul mais
  faible.** Un flux soutenu ne se traite pas en élargissant la garde — il dit que les consignes
  de spawn font dériver le `cwd`, et c'est *ça* qu'il faut traiter.
- **Preuve d'armement** : un hook `SessionStart` dans le même fichier écrit une ligne
  `shared-checkout-guard: armed`. **Son absence est l'information** — elle dit que le hook n'est
  pas chargé, et non que rien n'a eu à être refusé.
- `CONTRIBUTING.md` documente la garde, la dérogation et la sonde d'armement — en **complément**
  du mécanisme, jamais à sa place (M-4).

---

## 5. Couverture réelle, dite plutôt que déduite

| # | répertoire | couvert par ce ticket ? |
|---|---|---|
| 1 | `mika/` | **oui** — session enracinée dans un worktree `mika` |
| 2 | `mika/` | **oui** — idem |
| 3 | `claude-pilot/` | **non** — session enracinée dans un worktree `claude-pilot` |
| 4 | `claude-pilot/` | **non** — idem |

**Ce livrable ferme 2 des 4 occurrences mesurées.** C'est inconfortable et c'est le fait : le
hook d'un dépôt ne gouverne que les sessions enracinées dans ce dépôt. Les deux autres — dont
l'occurrence 3, la plus grave — exigent la **même** garde dans `claude-pilot`, d'où l'exigence
d'agnosticisme de M1 : le ticket de suivi y est un vendoring du script plus trois lignes de
`settings.json`, pas une réécriture.

Ordre contraint, sur le précédent mika#2023 → mika-cloud#242 : **`mika` d'abord**, parce que
c'est lui qui produit et éprouve le script que l'autre consommera.

---

## 6. Hors périmètre, délibérément — et tickets de suivi à ouvrir

- **`senara-solutions/claude-pilot`** — la même garde, vendorée. Ferme les occurrences 3 et 4.
  **C'est le suivi le plus urgent** : c'est là que l'arbre de travail *est* l'artefact déployé.
- **Sessions de l'orchestrateur** (project-dir = racine de l'espace de travail). Hors du premier
  terme du prédicat, à dessein — les y faire entrer casserait `/mika-platform-sync-main`. Si
  l'occurrence 4 est imputable à l'orchestrateur, elle appelle un prédicat distinct (« l'arbre
  partagé ne quitte `main` que par un geste nommé »), c'est-à-dire un autre ticket.
- **Shells humains.** Aucun hook d'outil ne les voit. Le seul levier serait un hook git, que
  M-2/M-3/M-4 disqualifient ; si le besoin se mesure, il se traitera pour lui-même.
- **La réparation des quatre incidents** — faite, et le ticket l'exclut explicitement.
- **Durcir la containment de dispatch** — déjà fait (mika#2141), rien à ajouter ici.

---

## 7. Risques

| Risque | Parade |
|---|---|
| **M0-b est faux** : le hook ne prime pas sur `permissions.allow` | Sonde bloquante avant toute écriture ; repli nommé ; remontée opérateur |
| **M0-e est faux** : le project-dir d'un spawn n'est pas son worktree → garde **inerte sur la population mesurée tout en paraissant livrée** | Sonde bloquante M0-e ; l'échec change le **prédicat**, pas le code, et remonte à l'opérateur avant M1 |
| **Garde cassée = garde silencieuse** (conséquence assumée du fail-open) | Ligne d'armement `SessionStart` (M4), qui est la seule chose distinguant « rien à refuser » de « rien n'est armé » |
| **L'échec d'écriture du journal fait refuser** (fail-closed déguisé, `~/.mika/state/` non inscriptible en sandbox) | Journaliser n'est jamais un terme du prédicat ; test dédié en M1b |
| **Faux positif** bloquant une commande légitime et calant une dispatche | Prédicat à quatre termes conjoints, tous positifs ; contrôles négatifs en M1b ; dérogation `MIKA_GUARD_SHARED_CHECKOUT=0` sans redéploiement. Coût borné : un refus n'est pas une destruction, la session retente autrement |
| **Le hook n'est pas chargé** (confiance projet, chemin, JSON invalide) → garde inerte et invisible | Ligne d'armement `SessionStart` (M4). Son absence est un signal, pas un silence |
| **Le script devient le point faible** — il tourne avant chaque Bash | Sh pur, pas d'appel réseau, au plus un `rev-parse` ; sortie `allow` immédiate dès que le premier terme est faux (le cas nominal) |
| **Faux sentiment de complétude** — croire les 4 occurrences fermées | § 5 l'écrit dans le plan, et le ticket de suivi `claude-pilot` est nommé |

---

## Definition of Done

- `scripts/guard-shared-checkout` et `scripts/test-guard-shared-checkout.sh` existent, sont
  exécutables, et le second passe.
- `.claude/settings.json` est suivi et déclare les hooks `PreToolUse` (garde) et `SessionStart`
  (preuve d'armement).
- Le job CI `shared-checkout-guard-lint` est déclaré dans `.github/workflows/ci.yml` et vert.
- Les quatre occurrences mesurées sont des fixtures nommées du test ; les contrôles négatifs du
  § M1b passent.
- `CONTRIBUTING.md` documente la garde, la dérogation et la sonde d'armement.
- Les résultats de M0 (a–e) sont consignés dans la description de la PR, y compris la mesure
  d'ordre de `git checkout` (M0-c) et le `$CLAUDE_PROJECT_DIR` observé d'un spawn (M0-e).
- La PR nomme explicitement la couverture 2/4 et le ticket de suivi `claude-pilot`.

## Acceptance criteria

1. **AC1 — La garde refuse les mécanismes mesurés.** Depuis une session dont le project-dir est
   un worktree lié, chacune des quatre formes mesurées (`reset --hard` nu, `checkout <branche>`,
   `add` via `-C`, `checkout <sha>`) visant le checkout principal est refusée. Le refus nomme
   l'arbre visé, le worktree de la session, le remède et la dérogation.
2. **AC2 — La garde ne coûte rien au nominal.** `git add`, `git commit`, `git checkout` visant
   le worktree de la session sont autorisés ; tous les verbes de lecture de l'allow-list sont
   autorisés même vers l'arbre partagé ; une session dont le project-dir n'est pas un worktree
   lié n'est jamais refusée. Aucune dispatche `dev-pilot` / `dev-groom` ne régresse.
3. **AC3 — Un verbe git non classé visant hors du worktree est refusé** (allow-list de lecture,
   pas deny-list de mutation), avec un test le démontrant sur un verbe absent des deux listes.
4. **AC4 — La garde est armée sans geste d'installation.** Dans un worktree fraîchement créé par
   `dispatch-lib.sh::_set_up_worktree`, sans `core.hooksPath` ni édition de
   `~/.claude/settings.json`, la ligne d'armement `SessionStart` est présente et AC1 tient.
5. **AC5 — La dérogation fonctionne et se voit.** `MIKA_GUARD_SHARED_CHECKOUT=0` laisse passer
   la commande ; l'événement est journalisé.
6. **AC6 — Tout refus est observable.** Chaque refus écrit une ligne exploitable dans
   `~/.mika/state/shared-checkout-guard.log` ; le régime attendu (faible mais non nul) et la
   halte associée (un flux soutenu se traite en amont, pas en élargissant la garde) sont écrits
   dans `CONTRIBUTING.md`.
7. **AC7 — La régression « garde absente » est attrapée par la CI.** Un test structurel échoue
   si `.claude/settings.json` perd le hook `PreToolUse` ou si le script cesse d'être exécutable.
8. **AC8 — Le script est agnostique du dépôt.** Aucun chemin ni nom de dépôt en dur ; le test le
   démontre en l'exerçant sur une arborescence de worktree jetable portant un autre nom.
9. **AC9 — La garde tombe en marche du bon côté, et le dit.** Un script absent, non exécutable
   ou rendant une sortie illisible **laisse passer** la commande (fail-open) — démontré par un
   test, et notamment sur le cas réel d'un worktree créé sur une branche antérieure au correctif.
   L'impossibilité d'écrire `~/.mika/state/shared-checkout-guard.log` **ne change jamais la
   décision** ; un test l'exerce avec un chemin de journal non inscriptible. Le coût de cet
   arbitrage — une garde cassée se lit comme une garde qui n'a pas eu à firer — est écrit dans
   `CONTRIBUTING.md` à côté de la sonde d'armement qui le compense.
10. **AC10 — La garde fire sur la population qu'elle vise.** M0-e est consignée : le
    `$CLAUDE_PROJECT_DIR` observé d'une session de spawn est un worktree lié, donc le premier
    terme du prédicat est vrai pour la population mesurée. Si la mesure dit l'inverse, le
    prédicat est reformulé et l'écart est remonté à l'opérateur avant M1 — livrer une garde
    inerte sur les quatre occurrences du ticket serait le pire résultat possible, puisqu'elle
    aurait l'apparence d'un correctif.
