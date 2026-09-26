# mika#2520 — Le push de `resolve-pr-conflicts` porte une refspec explicite, et le pilote ne pousse plus

**Ticket :** senara-solutions/mika#2520 (P0, enfant de la milestone mika#2491)
**Type :** fix — substrat de la boucle autonome
**Incident fondateur :** 2026-09-24 nuit — le pilote `resolve-pr-conflicts` dispatché
pour #2435 (task `357bb36e`) a **rembobiné `origin/main`** de `24f25e99` (#2489, mergé
19:30:25Z) vers `e851d72b` (#2514). Restauré en fast-forward sous GO opérateur.

---

## 0. Ce que la lecture du code déplace dans le ticket

Six constats. Les cinq premiers sont mesurés dans l'arbre ; le sixième est une mesure
que ce worktree **ne peut pas** faire, et c'est écrit comme tel plutôt que deviné.

### R1 — Le handler connaît DÉJÀ la branche, et ne la donne jamais au prompt

`skills/bundled/resolve-pr-conflicts/handlers/run.sh:100` appelle déjà
`gh pr view "$PR_NUMBER" --repo "$REPO_FULL" --json headRefName -q .headRefName` et
range le résultat dans `BRANCH`. Cette variable sert **uniquement** à dériver le
chemin du worktree (ligne 107) et n'est jamais interpolée dans `PROMPT` (lignes
177–185). Le pilote, lui, re-dérive sa branche à l'étape 2 par
`git branch --show-current`.

**AC4 n'est donc pas un manque d'infrastructure, c'est un trou de propagation.** La
valeur juste est déjà calculée, au bon endroit, par le bon appel — elle ne traverse
pas la frontière. Le correctif est de la faire traverser, pas d'en construire une
seconde.

Corollaire qui compte pour le diagnostic : `git branch --show-current` rend une chaîne
**vide** sur un HEAD détaché, et rend `main` si HEAD est sur `main`. L'étape 2 du
prompt bâtit donc son `gh pr list --head <vide|main>` sur cette valeur, et l'étape 6
pousse ce que `push.default` veut bien pousser. Aucune des deux ne consulte la branche
que le handler tient en main.

### R2 — Le prompt annule sa propre garantie à la ligne 1 (constat central)

`git push --force-with-lease` **sans** `=<ref>:<expect>` compare au *remote-tracking
ref* local (`refs/remotes/origin/<branche>`). Or l'étape 1 du même prompt est
`git fetch origin`, qui rafraîchit précisément ce ref.

> Depuis l'instant où l'étape 1 tourne, le bail de l'étape 6 ne protège plus rien :
> il compare le distant à une photo du distant prise après coup.

C'est la nuance que le corps du ticket ne nomme pas. Il impute le défaut à l'absence
de **refspec** (« sans refspec, sans branche cible explicite »), ce qui est vrai et
insuffisant : la forme nue est cassée sur **deux** axes indépendants — *où* ça pousse
(refspec) et *contre quoi* ça se protège (expect). AC1 les ferme tous les deux en
prescrivant `--force-with-lease=<branche>:<sha attendu>` ; ce plan retient le point
important qui en découle : **le `<sha attendu>` doit être capturé AVANT que le pilote
ne tourne**, sans quoi on réécrit la même annulation un cran plus loin.

### R3 — Ce skill lance claude-pilot HORS bac à sable

`run.sh:196` invoque `claude-pilot` directement. Il ne traverse **pas**
`_run_pilot_sandboxed` de `dispatch-lib.sh` : ni `bwrap`, ni coupure réseau, ni relais
d'egress, ni prologue de secrets. La posture fail-closed de mika#2049 ne s'applique
pas ici.

**Conséquence de conception, et elle décide du remède :** aucune interception
structurelle d'un `git push` arbitraire n'est disponible *à l'intérieur* de la
session. Un pilote qui décide de taper `git push` le fait. Le seul levier structurel
disponible est donc **de retirer le push de la session**, pas d'essayer de le filtrer.
(La mise sous bac à sable de ce skill est réelle, adjacente et **hors périmètre** —
suivi nommé au § 9.)

### R4 — Le précédent maison existe, né du même incident, et n'a jamais été appliqué ici

`skills/bundled/dev-groom/system_prompt.md:17-23` :

> The dev-groom pilot's scope is **content-only** […] All git push operations are
> dispatch-lib's responsibility (`_push_branch`). The pilot MUST NOT execute any of:
> `git push --force`, `git push --force-with-lease`, `git push -f` […]
> (mika#1318 founding incident: a dev-groom pilot ran `git push --force-with-lease`
> from inside its worktree, destroying substrate-fix work on the remote.)

mika#1318 → mika#1407 ont retiré le push du pilote et fait de `_push_branch` **l'unique
site de push** pour les dispatches dev-groom. `resolve-pr-conflicts` est le skill
frère qui n'a jamais reçu ce traitement. Ce plan applique le précédent plutôt que d'en
inventer un.

### R5 — Le défaut frère, mesuré, et il n'est pas corrigé ici

```
$ grep -rn "git push" skills/bundled/*/handlers/*.sh
skills/bundled/resolve-pr-conflicts/handlers/run.sh:183:6. Push: git push --force-with-lease
skills/bundled/address-pr-comments/handlers/run.sh:236:5. Push with: git push
```

Deux occurrences sur **7 scripts** de handler. `address-pr-comments` porte un
`git push` nu **et ne résout aucune branche** (zéro `headRefName` dans le fichier).
Même classe, un cran moins létal — le push n'est pas forcé, donc il ne peut pas
rembobiner.

**Mais il peut atterrir.** La sortie de l'incident le prouve :

> `Bypassed rule violations for refs/heads/main: Changes must be made through a pull request.`

L'identité du pilote porte un **bypass admin**. Un `git push` nu, non forcé, qui
viserait `main` y atterrirait en fast-forward, hors PR, hors revue. Population réelle,
sévérité moindre, correctif différent (ce skill n'a pas de `pr_url` → `headRefName` à
brancher, il faudrait le construire). **Non corrigé dans cette PR** — entrée
d'allowlist nommée et ticket de suivi, § 6.

### R6 — Ce que ce worktree ne peut PAS établir

Le mécanisme exact du rembobinage. Deux lectures restent en lice :

| lecture | ce qu'il faudrait mesurer |
|---|---|
| `push.default = matching` → le push nu a poussé **toutes** les branches locales de même nom, dont un `main` local périmé, pendant que HEAD était bien sur `fix/2135` | `git config --get push.default` sur l'hôte de dispatch |
| HEAD avait bougé sur `main` (rebase avorté, `checkout`) → le push nu a poussé `main:main` | le log claude-pilot de la task `357bb36e` |

`git config --get push.default` est **refusé par la politique de permissions** dans ce
bac à sable (`no matching policy rule -- denied by default`), et le log de la task
n'est pas lisible d'ici. Les deux mesures sont des gestes d'opérateur sur l'hôte.

**Le correctif ne dépend d'aucune des deux**, et c'est délibéré : la refspec explicite
ferme la première lecture (on ne pousse plus que `HEAD:refs/heads/<branche>`), le terme
R4 du § 2.2 ferme la seconde (on refuse si HEAD n'est pas sur la branche attendue).
Écrire ici « la cause est X » serait une affirmation que rien dans ce worktree ne
soutient.

---

## 1. Requirements

| # | Exigence | AC couverte |
|---|---|---|
| U1 | Le push du skill porte une refspec de destination **explicite** et un bail **pinné sur un SHA capturé avant la session** | AC1 |
| U2 | Le push est **refusé** quand la cible résolue est une branche protégée, la branche de base de la PR, ou quand le worktree n'est pas dans l'état attendu | AC2 |
| U3 | Le pilote ne pousse plus : le push sort de la session et devient un site de code, unique et gardé | AC1, AC2 |
| U4 | La branche cible vient de `gh pr view --json headRefName`, jamais de la config de push locale ni de `git branch --show-current` | AC4 |
| U5 | Tests négatifs : une résolution qui viserait `main` est **refusée, pas poussée** — attesté sur l'absence d'invocation de `git push`, pas sur un code de retour | AC3 |
| U6 | Un scan de source refuse le retour d'une instruction de push dans un prompt de handler | — (garde de classe) |
| U7 | Documentation : `system_prompt.md` du skill + entrée `CLAUDE.md` racine | — |

---

## 2. Conception — la résolution et ses cinq refus

### 2.1 Un fichier partagé, sourçable, sans code impératif de haut niveau

Nouveau fichier **`skills/bundled/_shared/pr-push-guard.sh`**.

Pourquoi `_shared/` et pas dans `run.sh` : `run.sh` porte du code impératif en tête
(`set -e`, `trap`, lecture de stdin) — il n'est **pas** sourçable, donc rien de ce
qu'il contient n'est testable sans le lancer pour de bon. `_shared/` est le mécanisme
prévu exactement pour ça (mika#923 : « shared support libraries […] so sibling skills
can source them at runtime via relative path »), il est découvert par `build.rs`
(`discover_support_dirs`) et **seedé inconditionnellement**, y compris sous
`MIKA_DISABLE_BUNDLED_SKILLS`.

Contrainte reprise du test de `dispatch-lib` : **aucun code impératif de haut niveau**
dans ce fichier — que des définitions de fonctions et des constantes. C'est ce qui rend
l'audit d'isolation inutile et le harnais de test trivial.

Shebang : `run.sh` passe de `#!/bin/sh` à `#!/bin/bash`, comme ses frères
`dev-pilot/handlers/run.sh` et `dev-groom/handlers/run.sh` qui sourcent déjà
`dispatch-lib.sh` en bash. Ça retire une classe entière de doute de portabilité
(`local`, `[[ ]]`) sans rien acheter d'exotique : bash est déjà une dépendance de
handler dans ce dépôt.

### 2.2 Les cinq refus, tous fail-closed

L'asymétrie qui décide du sens de chaque refus, écrite une fois pour les cinq :

> **Un refus à tort coûte un dispatch — visible, rattrapable, borné.
> Un passage à tort peut rembobiner une branche protégée — irréversible.**

Donc : tout signal illisible **refuse**. C'est l'inverse de la règle fail-safe du
faucheur mika#2420 (où un signal illisible *conserve*), et l'inversion est raisonnée :
là-bas l'action détruisait du travail, ici l'action *est* l'écriture distante.

| # | Terme | Refuse quand |
|---|---|---|
| R1 | Branche résolue | `headRefName` vide, illisible, ou `gh` en échec |
| R2 | Branche protégée | branche ∈ `{main, master}` |
| R3 | Branche de base | branche == `baseRefName` de la PR ; **ou `baseRefName` illisible** |
| R4 | HEAD du worktree | `git symbolic-ref --short HEAD` ≠ branche résolue (détaché, ou sur une autre branche) |
| R5 | État publiable | rebase en cours, arbre sale, ou `git ls-remote origin refs/heads/<branche>` vide |

**R3 est la généralisation que l'AC2 demande entre parenthèses** (« ou toute branche
protégée / branche de base d'une PR ») : une PR empilée sur `feat/x` a pour base
`feat/x`, qui n'est dans aucune liste statique. Le champ vient du **même** appel
`gh pr view` que `headRefName` — un seul aller-retour, `--json headRefName,baseRefName`.

**R4 ferme la seconde lecture de R6 sans avoir à la trancher.** Si HEAD a bougé sur
`main`, on refuse au lieu de pousser le tip de `main` sur la branche de la PR.

**R5 avec `ls-remote` vide refuse au lieu de faire un premier push.** Une PR ouverte
implique que sa branche existe sur le distant ; son absence signifie que la résolution
regarde autre chose. Pousser là serait créer une branche sur une prémisse fausse.

La liste `{main, master}` est un **doublon assumé** de
`GIT_OPS_PROTECTED_BRANCHES` (`crates/mika-agent/src/skills/builtin_handlers.rs:834`).
Aucun single-source inter-langage n'existe ici et en inventer un serait
disproportionné ; la constante shell porte un commentaire nommant sa jumelle Rust, et
c'est le prix — nommé — plutôt qu'un mécanisme.

### 2.3 Le bail est capturé AVANT la session (constat R2 rendu opérationnel)

```
EXPECTED_SHA = $(git -C "$WORKTREE" ls-remote origin refs/heads/<BRANCH> | cut -f1)
```

capturé **avant** `claude-pilot`, et porté jusqu'au push. C'est strictement meilleur
que la forme `--force-with-lease=$BRANCH:origin/$BRANCH` de `_push_branch` : un
`git fetch` du pilote rafraîchit `origin/$BRANCH` et voide ce bail-là, alors qu'un SHA
littéral capturé avant la session ne bouge pas.

Si quelqu'un pousse sur la branche de la PR pendant la session, le push est **refusé
par git**, ce qui est exactement ce qu'on veut : le travail du tiers n'est pas écrasé,
et le rebase reste dans le worktree.

### 2.4 Pas de retry, et le coût est nommé

L'étape 7 actuelle du prompt (« If push fails, fetch and retry once ») est retirée sans
remplacement.

Le seul échec légitime de ce push est « quelqu'un a poussé sur la branche pendant la
session ». Re-baîller sur le nouveau SHA écraserait son travail — c'est-à-dire
reproduirait, sous une forme polie, le défaut qu'on ferme. Et un retry **dans le
prompt** est précisément la surface d'improvisation où un `git push` nu se tape : après
un échec, sous pression.

**Coût assumé :** un échec réseau transitoire coûte un dispatch. Les commits du rebase
restent dans le worktree, rien n'est perdu, le callback le dit. Si une mesure montre que
cette population est non négligeable, le remède est d'adopter la chaîne bornée
`_push_with_rebase_retry` de `dispatch-lib` — **ticket de suivi**, précondition : cette
mesure.

---

## 3. Conception — le push sort de la session

### 3.1 L'ordre des opérations dans `run.sh`

```
 1. résoudre        : gh pr view --json headRefName,baseRefName   (un seul appel)
 2. REFUSER (R1-R3) : AVANT tout spawn — zéro coût LLM sur un refus
 3. capturer        : EXPECTED_SHA via ls-remote                  (avant la session)
 4. prompt          : sans aucune instruction de push
 5. claude-pilot    : le pilote rebase, résout, teste. Il ne pousse pas.
 6. REFUSER (R4-R5) : état du worktree après la session
 7. ré-affirmer     : R2-R3 sur les mêmes variables shell
 8. pousser         : site unique et gardé
 9. rapporter
```

Le refus à l'étape 2 **précède le spawn**, ce qui est plus fort que la lettre d'AC3
(« refusé, pas poussé ») : aucun tour LLM n'est dépensé.

L'étape 7 ré-affirme R2/R3 sur des variables shell que le pilote ne peut pas toucher.
C'est presque gratuit et ça garde le site de push honnête devant une future édition qui
muterait `BRANCH` entre-temps.

### 3.2 Le site de push, unique, et son argv exact

```bash
git -C "$WORKTREE_PATH" push \
    --force-with-lease="$BRANCH:$EXPECTED_SHA" \
    origin "HEAD:refs/heads/$BRANCH"
```

C'est **mot pour mot** la forme qu'AC1 prescrit. Le test V3 assertera sur **l'argv
réellement construit**, jamais sur l'intention du code : c'est la leçon de mika#2304
(un champ qui affirme, avec autorité, l'override qui n'a pas eu lieu) et la forme que
`pilot_budget_armed` (mika#2496) a dû adopter — *lire l'argv, jamais le résolveur*.

### 3.3 Le nouveau prompt

```
Resolve merge conflicts on this branch.

Branch: <BRANCH>        (resolved from the PR head — do NOT re-derive it)
Base:   <BASE>

Steps:
1. git fetch origin
2. git rebase origin/<BASE>
3. If conflicts arise, resolve each one: read both sides, choose the correct
   resolution, git add the file, git rebase --continue
4. After the rebase completes, run the repo's test suite if one exists
   (check for Makefile, cargo, npm, etc.)
5. Stop there. DO NOT PUSH.

The push is performed by the handler after this session exits, with an explicit
refspec and a lease captured before you started. You MUST NOT execute any of:
  - git push (plain)
  - git push --force, git push --force-with-lease, git push -f
  - git remote set-url, git config push.default, or any other command that
    changes where a push lands.

If the rebase has conflicts you cannot confidently resolve, run
git rebase --abort and report what conflicts were found.
```

Deux dérivations LLM disparaissent : la branche (étape 2 actuelle,
`git branch --show-current` + `gh pr list --head`) et la refspec (étape 6 actuelle).

**La liste d'interdictions est du prompt, donc elle ne tient pas seule** — c'est
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` (mika#2120 :
neuf récurrences sous application par prompt contre zéro quand le fait est posé par le
code). Elle est reprise de dev-groom parce qu'elle est **gratuite** et qu'elle retire
le modèle de commande à imiter ; la moitié qui tient est que le prompt ne contient plus
aucun push, et que le scan du § 4 l'y maintient.

### 3.4 Le refus va dans `RESULT`, jamais dans le seul stderr

Piège de la classe mika#2050, un fichier plus loin : le stderr que `run.sh` écrit
**avant** de lancer le pilote n'est pas capturé dans `$STDERR_FILE` (créé plus bas) ;
il hérite du `Stdio::piped()` de l'exécuteur, que celui-ci ne lit que dans la branche
`if !status.success()`.

Le chemin de refus **doit** donc :

1. poser `RESULT="REFUSED (mika#2520) — <terme>: <détail>. <geste de levée>"`,
2. `echo` sur stderr (utile, non suffisant),
3. `exit 1` — le trap `EXIT` délivre `RESULT` via `mika ask --task-complete`.

`RESULT` est la surface durable : elle atterrit dans `tasks.result`, lisible par
`mika tasks get` et par le dashboard. Un refus qui ne nomme pas son geste de levée est
un refus qu'on contourne au jugé, donc chaque terme nomme le sien.

### 3.5 Le chemin `worktree_path` déprécié

`worktree_path` sans `pr_url` ne donne **ni** `headRefName` **ni** `baseRefName` : AC4
y est inatteignable par construction.

Mesure : **zéro appelant** dans l'arbre passe `worktree_path` à `resolve_pr_conflicts`
(les occurrences de `worktree_path` visent `address_pr_comments` —
`self-dev-iterate/system_prompt.md:41`). Le seul appelant mesuré de
`resolve_pr_conflicts` est `self-dev-callback/system_prompt.md:123`, sans argument
explicite.

Décision : la résolution des conflits continue de fonctionner sur ce chemin, **le push
y est refusé** sous un motif nommé (`no_pr_url`). Population mesurée vide, donc coût nul
en pratique, et on ne casse aucun contrat d'outil. Rendre `pr_url` obligatoire dans le
schéma serait plus net mais change la surface d'outil pour un gain nul.

---

## 4. Conception — le scan de source

**`scripts/check-pilot-push-sites.sh`**

**Règle, une phrase :** aucun `git push` ne peut apparaître dans
`skills/bundled/*/handlers/*.sh`.

Le prédicat est **positionnel et lexical**, pas sémantique, et c'est ce qui le rend
tenable :

- **Population** : `skills/bundled/*/handlers/*.sh` — **7 fichiers** mesurés.
- **Hors population par construction** : `_shared/` (qui porte le site de push gardé,
  légitime) et `system_prompt.md` (qui porte les *interdictions* de dev-groom, lignes
  19-20 — un scan sémantique devrait les distinguer des prescriptions, celui-ci n'a
  même pas à poser la question).
- **Lignes de commentaire retirées avant l'examen** (motif du scan mika#2496).

Un handler qui a besoin de pousser appelle l'aide partagée gardée. **Un site de push
qu'on ne veut pas garder est un site à router, jamais à exempter** (doctrine
mika#2201).

Allowlist : `scripts/pilot-push-allowlist.txt`, comparée **dans les deux sens** — une
entrée qui ne matche plus rien fait rougir le build, exactement comme un site non
listé. C'est l'assertion auto-nettoyante : le jour où `address-pr-comments` est
réparé, le build rougit et l'entrée doit partir.

Câblage : job CI `pilot-push-lint` (modèle `a2a-timeout-literal-lint`), plus une cible
`make test-pilot-push-guard`, plus l'entrée dans la cible `test`.

---

## 5. Contrat de vérification

### 5.1 Ce qui est attestable, et par quoi

`skills/bundled/_shared/tests/test_pr_push_guard.sh` — bash, sourçage direct de
`pr-push-guard.sh`, `git` et `gh` remplacés par des fonctions qui **enregistrent leurs
appels**.

| # | Assertion | Couvre |
|---|---|---|
| V1 | `main` refusé ; `master` refusé | AC2, R2 |
| V2 | branche == base refusée (y compris base ≠ `main`, p.ex. `feat/x`) | AC2, R3 |
| V3 | `baseRefName` illisible ⇒ **refusé** (fail-closed) | R3 |
| V4 | branche vide / `gh` en échec ⇒ refusé | R1 |
| V5 | HEAD sur une autre branche ⇒ refusé ; HEAD détaché ⇒ refusé | R4 |
| V6 | rebase en cours ⇒ refusé ; arbre sale ⇒ refusé ; `ls-remote` vide ⇒ refusé | R5 |
| V7 | **argv exact** : `push --force-with-lease=<b>:<sha> origin HEAD:refs/heads/<b>` | **AC1** |
| V8 | le `<sha>` du bail est celui capturé **avant** la session, pas un relu après | AC1, R2 (§ 0) |
| V9 | **aucun `git push` n'est invoqué** sur chacun des chemins de refus V1–V6 | **AC3** |
| V10 | le refus pose `RESULT` (pas seulement stderr) et rend un code non nul | § 3.4 |

**V9 est la forme qui compte, et elle n'est pas interchangeable avec V1–V6.** Asserter
le seul code de retour passerait sur une fonction qui retourne 1 **après** avoir
poussé. L'assertion porte donc sur le journal d'appels du faux `git` : le jeton `push`
n'y apparaît pas.

**V7 porte sur l'argv construit**, pas sur une variable d'intention (§ 3.2).

### 5.2 Contrôles négatifs, vus rouges

Sans eux, « le test vérifie la forme » est indistinguable de « le test ne vérifie
rien ».

| # | Fixture | Attendu |
|---|---|---|
| N1 | une variante du site de push qui construit `git push --force-with-lease` **nu** | V7 rouge |
| N2 | une variante qui retire le terme R2 | V1 rouge |
| N3 | une variante qui retire le terme R4 | V5 rouge |
| N4 | un handler fixture portant `git push` nu | scan rouge |
| N5 | un handler fixture portant `git push --force-with-lease` nu | scan rouge — **pas de cas particulier pour la forme « presque bonne »** |
| N6 | un handler fixture sans aucun `git push` | scan **vert** |
| N7 | un `system_prompt.md` fixture portant les interdictions de dev-groom | scan **vert** (hors population) |
| N8 | une entrée d'allowlist ne matchant aucun fichier | scan rouge (deux sens) |

N6 et N7 sont les contrôles de bonne foi : sans eux, un scan devenu rouge en permanence
serait désarmé à la première gêne.

### 5.3 Ce qui n'est PAS testable ici, écrit plutôt que découvert

- **« Le pilote n'a pas poussé. »** Le pilote exécute un modèle réel dans un autre
  processus. Le contrat *côté mika* est : le prompt ne contient aucune instruction de
  push (attesté par le scan) et le handler pousse (attesté par V7). La moitié
  comportementale est la sonde S2 du § 8.
- **Le mécanisme du rembobinage** (R6) — deux gestes d'opérateur sur l'hôte.
- **Le bypass admin de l'identité du pilote** — c'est le ticket compagnon « durcissement
  ruleset no-bypass main » nommé dans les *Liens* de mika#2520.

### 5.4 Pré-vol, à exécuter avant de livrer le scan

```bash
grep -rn "git push" skills/bundled/*/handlers/*.sh
```

Attendu après le correctif : **une seule ligne**, `address-pr-comments/handlers/run.sh`.
Si la commande en rend d'autres, l'allowlist du § 6 est incomplète et **c'est
l'inventaire qu'il faut refaire, pas l'allowlist qu'il faut gonfler.**

---

## 6. Fire-Disposition

Ce plan livre des détecteurs : les tests négatifs V1–V10 / N1–N8 et le scan de source
du § 4. La section est donc requise.

**Option retenue : (a) exception nommée en allowlist.**

**Ce qui tire au moment de l'atterrissage :** le scan du § 4, sur
`skills/bundled/address-pr-comments/handlers/run.sh:236` (`5. Push with: git push`).
Violation réelle, mesurée, de la même classe (§ 0 R5) — et **pas** corrigée par cette
PR : `address-pr-comments` ne résout aucune branche, donc lui donner une refspec
explicite demande d'y brancher la même chaîne `gh pr view` → refus → push handler, soit
l'architecture entière appliquée à un second skill. Sur un P0 dont les AC portent sur
`resolve-pr-conflicts`, c'est une expansion de périmètre qui achèterait un second risque.

Les tests V1–V10 ne tirent sur rien : ils portent sur du code neuf.

**L'entrée, `scripts/pilot-push-allowlist.txt` :**

```
# Une entrée = un site de push non gardé, sa raison, son suivi.
# Comparée dans LES DEUX SENS : une entrée qui ne matche plus rien fait rougir
# le build. C'est l'assertion auto-nettoyante — le jour de la réparation, pas
# des mois après.
#
# skills/bundled/address-pr-comments/handlers/run.sh
#   `git push` nu (ligne ~236), dans le PROMPT du pilote. Même classe que
#   mika#2520, un cran moins létal : le push n'est pas forcé, donc il ne peut
#   pas rembobiner. Il peut en revanche ATTERRIR sur une branche protégée —
#   l'identité du pilote porte un bypass admin, mesuré dans la sortie de
#   l'incident du 2026-09-24 (« Bypassed rule violations for refs/heads/main »).
#   Ce handler ne résout AUCUNE branche (zéro `headRefName`) : lui donner une
#   refspec explicite demande d'y brancher la chaîne complète du § 2.
#   Suivi : <numéro à ouvrir> — appliquer pr-push-guard.sh à address-pr-comments.
skills/bundled/address-pr-comments/handlers/run.sh
```

**Pourquoi (a) et pas (b) « livrer désarmé ».** Un scan désarmé sur la population qu'il
vient de réparer ne protège pas `resolve-pr-conflicts` contre le retour du défaut dès
le lendemain, et « désarmé + suivi pour l'armer » est la forme sous laquelle une garde
reste désarmée. L'allowlist garde le scan **armé sur le site réparé dès le jour un**, et
rend le résidu **visible et compté** au lieu de toléré en silence.

**Ce qu'il faut savoir avant de toucher à ce fichier :** quand le scan tire sur un
*nouveau* site, la résolution est de **router ce site vers l'aide gardée**, jamais
d'ajouter une ligne ici. L'allowlist n'a pas de place libre : elle a une entrée, datée,
avec son ticket.

---

## 7. Documentation

1. **`skills/bundled/resolve-pr-conflicts/system_prompt.md`** — trois phrases y
   affirment aujourd'hui que le skill « pushes with `--force-with-lease` » (lignes 7,
   48, 55). Elles deviennent fausses et sont réécrites : le pilote résout, le handler
   pousse, le push porte une refspec explicite et un bail pré-session. Le tableau
   *Expected Outcomes* gagne la ligne **Refusal** (cible protégée / base / HEAD
   déplacé / état non publiable), avec le geste de levée.
2. **`CLAUDE.md` racine** — section courte dans le voisinage des skills de dispatch :
   les cinq refus, la propriété « le pilote ne pousse plus », la surface opérateur
   (`tasks.result`, pas un log — § 3.4), la halte de déploiement (§ 8), et le suivi
   nommé du § 6. Proportionnée : un P0 sur un skill, pas une nouvelle sous-système.
3. **`tools.json`** — la description de `worktree_path` gagne « le push est refusé sans
   `pr_url` » (§ 3.5).

Aucun fichier sous le périmètre de `scripts/sync-agent-docs.sh` n'est touché (le job CI
`docs-sync` reste vert).

---

## 8. Surfaces opérateur et sondes

### Lecture

```bash
# Pourquoi ce dispatch n'a-t-il rien poussé ?
mika tasks get <task-id>          # le motif est dans `result`, préfixé REFUSED (mika#2520)
```

```sql
SELECT id, result FROM tasks
 WHERE result LIKE 'REFUSED (mika#2520)%' ORDER BY created_at DESC;
```

**Pas de nouvel événement de journal, et c'est une décision.** Le handler est un
sous-processus shell sans accès base ; son stderr d'avant-pilote est structurellement
perdu sur un dispatch qui réussit (§ 3.4, classe mika#2050). Inventer une surface de
log qui ne serait pas lue reproduirait le défaut du Signal M. `tasks.result` est la
surface durable, et c'est celle que l'opérateur consulte déjà.

| motif dans `result` | régime attendu | lecture |
|---|---|---|
| `REFUSED … protected_branch` | **vide** | toute occurrence est un rembobinage évité — et une anomalie amont : pourquoi la PR a-t-elle `main` en head ? |
| `REFUSED … branch_is_base` | **vide** | idem |
| `REFUSED … head_mismatch` | **proche de zéro** | le pilote a laissé HEAD ailleurs. Non nul soutenu ⇒ lire le log pilote **avant** de toucher au prédicat |
| `REFUSED … no_pr_url` | **vide** | population mesurée vide (§ 3.5) ; une occurrence nomme un appelant à migrer |
| `REFUSED … not_publishable` | faible | rebase avorté ou arbre sale — le travail est dans le worktree |

### Sondes, et leurs quatre haltes

**S1 — le chemin nominal tient (première PR en conflit après déploiement).**
La PR est rebasée et poussée ; `result` porte la ligne de push avec la refspec.
*Halte 1 — rien n'est poussé et `result` ne porte aucun `REFUSED`.* Ne pas retoucher le
prédicat : établir d'abord que le binaire servi porte le correctif. `_shared/` est une
projection du **binaire**, pas du checkout — `cat ~/.mika/skills/.manifest-writer` et
comparer le sha à celui du dernier `make deploy` (classe mika#2340). Un fichier neuf
dans `_shared/` n'atteint aucun agent avant `make deploy` → seed.

**S2 — le pilote ne pousse plus (48 h).** Sur le log claude-pilot d'un dispatch
`resolve-pr-conflicts`, zéro `git push`.
*Halte 2 — le pilote pousse quand même.* C'est la moitié prompt qui ne tient pas, et
c'est **attendu comme possible** (§ 3.3). Ne pas durcir le prompt par réflexe : le
levier structurel disponible est la mise sous bac à sable du skill (§ 0 R3), et c'est
le ticket de suivi qu'il faut ouvrir, avec cette occurrence comme précondition.

**S3 — contrôle négatif (7 jours).** Aucune écriture non-PR sur `main` de
`senara-solutions/mika` : `git log --first-parent origin/main` ne doit porter que des
merges de PR.
*Halte 3 — une occurrence.* Désarmer d'abord (`resolve_pr_conflicts` retiré de
l'allowlist de skills de l'agent), diagnostiquer ensuite. Et **lire l'auteur** : si ce
n'est pas `resolve-pr-conflicts`, la piste est `address-pr-comments` (§ 6) ou le ticket
compagnon ruleset, pas ce correctif.

**S4 — le scan est armé et il regarde quelque chose.**
```bash
bash scripts/check-pilot-push-sites.sh; echo "rc=$?"     # attendu : 0
wc -l < scripts/pilot-push-allowlist.txt                  # attendu : une entrée
```
*Halte 4 — le scan rend 0 avec une allowlist vide.* Il ne regarde plus rien : la
population a bougé (renommage de répertoire, changement d'extension) et un scan
silencieusement inerte se lit exactement comme un arbre propre (classe mika#2205).
Vérifier `ls skills/bundled/*/handlers/*.sh | wc -l` ≥ 7 avant toute conclusion.

### Ce que ce travail n'achète PAS

- **Aucun compteur, aucun événement de journal.** Le seul instrument est la lecture de
  `tasks.result` ci-dessus, et **son silence ne prouve rien tant que personne ne la
  lance** — sur un skill dispatché quelques fois par semaine, l'absence d'occurrence
  peut simplement signifier qu'aucun conflit n'a été résolu.
- **Aucune protection contre un `git push` que le pilote taperait de lui-même** (§ 0 R3).
  Ce qui est retiré est l'instruction et le modèle à imiter, pas la capacité.
- **Aucune correction rétroactive.** Le rembobinage du 2026-09-24 a été réparé à la main
  sous GO opérateur ; rien ici ne le rejoue ni ne le documente au-delà de ce plan.
- **`address-pr-comments` reste exposé** (§ 6), avec son ticket.

---

## Definition of Done

- [ ] `skills/bundled/_shared/pr-push-guard.sh` livré : cinq refus fail-closed, site de
      push unique, aucun code impératif de haut niveau.
- [ ] `resolve-pr-conflicts/handlers/run.sh` : résolution `headRefName,baseRefName` en
      un appel, refus avant spawn, `EXPECTED_SHA` capturé avant la session, prompt sans
      push, push après session, refus posés dans `RESULT`.
- [ ] `scripts/check-pilot-push-sites.sh` + `scripts/test-check-pilot-push-sites.sh` +
      `scripts/pilot-push-allowlist.txt` (une entrée) + job CI `pilot-push-lint` +
      `make test-pilot-push-guard` câblée dans `make test`.
- [ ] `skills/bundled/_shared/tests/test_pr_push_guard.sh` : V1–V10 verts, N1–N3 **vus
      rouges** avant d'être remis verts.
- [ ] Scan : N4–N8 verts, dont N6/N7 comme contrôles de bonne foi.
- [ ] `make verify-bundled-skills` vert (structure du bundle).
- [ ] Documentation : `system_prompt.md`, `tools.json`, entrée `CLAUDE.md` racine.
- [ ] Pré-vol du § 5.4 exécuté et son résultat reporté dans le corps de la PR.
- [ ] Corps de PR : la section Fire-Disposition citée, avec le numéro du ticket de suivi
      `address-pr-comments` **ouvert** (ligne `Tracked in:` requise par
      `pr-body-validation.yml`).

---

## Acceptance criteria

Transcrites du corps de senara-solutions/mika#2520.

- **AC1** — Le push du skill utilise une **refspec EXPLICITE** :
  `git push --force-with-lease=<branche>:<sha attendu> origin HEAD:refs/heads/<branche>`,
  où `<branche>` est le head de la PR (jamais nu, jamais `HEAD` seul).
- **AC2** — **Refus dur** si la branche cible résolue est `main` (ou toute branche
  protégée / branche de base d'une PR). Le skill ne doit JAMAIS pousser sur `main`.
- **AC3** — **Test négatif** : un worktree dont la résolution viserait `main` (branche
  mal configurée, upstream = main) est refusé, pas poussé.
- **AC4** — La branche cible est dérivée du head de la PR
  (`gh pr view --json headRefName`), pas de la config de push locale du worktree.

**Couverture :**

| AC | Où c'est livré | Où c'est attesté |
|---|---|---|
| AC1 | § 3.2 — site de push unique, argv mot pour mot ; § 2.3 — le `<sha attendu>` est capturé avant la session | V7, V8 ; N1 vu rouge |
| AC2 | § 2.2 — R2 (protégée) et R3 (base de la PR, y compris base ≠ `main`), tous deux fail-closed | V1, V2, V3 ; N2 vu rouge |
| AC3 | § 3.1 — refus **avant le spawn** | V9 (absence d'invocation de `git push` sur les six chemins de refus) |
| AC4 | § 2.1–2.2 — `gh pr view --json headRefName,baseRefName`, un appel ; § 3.3 — la branche est interpolée, le prompt ne la re-dérive plus | V4 ; § 5.4 (aucun `git branch --show-current` ni `push.default` sur le chemin de décision) |

---

## 9. Hors périmètre, délibérément

- **La mise sous bac à sable de `resolve-pr-conflicts`** (§ 0 R3) — ce skill lance
  claude-pilot sans `bwrap`, sans coupure réseau, sans relais d'egress, contrairement à
  tout pilote passant par `dispatch-lib`. C'est le seul levier qui empêcherait
  structurellement un `git push` spontané. Blast radius large (containment mika#2049),
  décision distincte. **Ticket de suivi**, précondition : la halte 2 de la sonde S2.
- **`address-pr-comments`** (§ 0 R5, § 6) — allowlisté, ticket de suivi nommé.
- **Le durcissement du ruleset « no-bypass main »** — ticket compagnon déjà nommé dans
  les *Liens* de mika#2520. C'est la seconde moitié de l'incident : sans le bypass
  admin, le force-push aurait été refusé par GitHub. Ce plan ferme le côté client ; le
  côté forge est ailleurs et ce n'est pas une redondance, c'est une défense en
  profondeur dont les deux moitiés tombent séparément.
- **Le gap iterate-rebase (#2435)** — voisin nommé dans les *Liens*, population et
  remède distincts.
- **Le retry de push** (§ 2.4) — retiré ; ré-adoption via `_push_with_rebase_retry`
  conditionnée à une mesure.
- **Le mécanisme exact du rembobinage** (§ 0 R6) — deux gestes d'opérateur sur l'hôte,
  et le correctif n'en dépend pas.
