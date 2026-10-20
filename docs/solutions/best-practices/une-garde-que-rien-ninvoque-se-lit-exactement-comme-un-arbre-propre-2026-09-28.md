---
title: Une garde que rien n'invoque se lit exactement comme un arbre propre
date: 2026-09-28
category: best-practices
module: scripts, .github/workflows, Makefile
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - "Livrer un script de garde sous `scripts/` (lint de forme, scan structurel, anti-vacuité)"
  - "Relire un PR qui ajoute un détecteur et lire « tous les tests passent »"
  - "Constater qu'une classe de défaut déjà documentée se reproduit malgré sa garde"
  - "Écrire une garde dédiée à protéger une propriété d'une AUTRE garde"
related_components:
  - testing_framework
  - tooling
tags:
  - garde-structurelle
  - cablage-ci
  - anti-vacuite
  - remede-local
  - faux-vert
  - mika-1801
  - mika-2793
---

# Une garde que rien n'invoque se lit exactement comme un arbre propre

## Contexte

mika#1801 (LC.2) livre `<Button>` dans `@samidarko/ui` et migre dix CTA vers lui.
Pour que la migration ne se défasse pas, la branche livre aussi un détecteur :
`scripts/check-cta-primitives.sh`, 357 lignes, qui refuse un fond plein sur un
`<button>` ou un `<a>` hors de `<Button>` — plus son harnais d'anti-vacuité,
`scripts/test-check-cta-primitives.sh`, 32 assertions incluant les contrôles
négatifs vus rouges.

Les deux scripts étaient **corrects**. Lancés à la main, ils font exactement ce
qu'ils annoncent :

```
$ bash scripts/check-cta-primitives.sh
No hand-rolled CTA outside the allowlist
  scanned:   55 file(s) under dashboard/src site/src
  found:     10 CTA element(s) — 10 via <Button>, 0 hand-rolled

$ bash scripts/test-check-cta-primitives.sh | tail -1
check-cta-primitives anti-vacuity: 32 passed, 0 failed
```

Et ils n'étaient référencés **nulle part** : ni dans `.github/workflows/ci.yml`,
ni dans le `Makefile`. Aucune CI ne les exécutait. Aucune cible `make` ne les
nommait. Le détecteur avait un harnais prouvant qu'il sait rougir, et rien au
monde ne le faisait tourner.

Rien n'était rouge. Aucun test ne manquait. La seule façon de voir le défaut
était de se demander *qui appelle ce fichier* — une question que ni un test, ni
une revue de diff, ni une CI verte ne posent.

Et la branche connaissait la règle. Quelques commits plus tôt, le même ticket
avait ajouté une étape `npm run typecheck -w packages/ui` au job *Dashboard*,
sous ce commentaire (`.github/workflows/ci.yml:139-143`) :

> `Button.type-assertions.tsx` is inert unless a typechecker runs it —
> **a guard nobody runs is a decoration** (mika#2103).

Le même PR a écrit la phrase, l'a appliquée à un fichier d'assertions de types,
et a livré à côté un détecteur shell que personne ne lançait. Savoir la règle
n'a pas suffi : elle a été appliquée là où le trou venait d'être remarqué, et
pas là où il n'avait pas été cherché.

## La couche zéro de l'inertie d'une garde

Ce dépôt poursuit déjà cette famille sous plusieurs noms, et chacun décrit une
garde **qui tourne** :

| couche | la garde… | ce qui l'attrape | déjà documenté |
|---|---|---|---|
| **0. jamais invoquée** | n'est appelée par aucun runner | **inventorier** : chaque script de `scripts/` est-il nommé par `ci.yml` ou le `Makefile` ? | *ce doc* |
| 1. prédicat trop étroit | tourne, connaît une seule écriture du défaut | **planter** une vraie violation et voir si elle est vue | [mika#2103](2103-a-guard-that-knows-one-spelling-of-a-defect-protects-nothing-else.md) |
| 2. population vide | tourne, ne regarde rien | **mesurer** le compte, avec un contrôle positif | mika#2205, et [les trois façons](trois-facons-dont-une-verification-passe-au-vert-sans-rien-mesurer-2026-09-22.md) |
| 3. fail-open interne | tourne, regarde, et laisse passer | **muter** le terme et voir si l'assertion rougit | [parser, fixture, harnais](structural-guard-fails-open-parser-fixture-harness.md) |

Les trois techniques des couches 1 à 3 — planter, mesurer, muter — **supposent
toutes que la garde s'exécute**. Aucune ne peut voir la couche 0. C'est ce qui
la rend distincte plutôt que redondante : elle est en amont de l'endroit où
toutes les autres commencent à regarder.

Corollaire qui vaut d'être dit, parce qu'il est contre-intuitif : **un harnais
d'anti-vacuité non invoqué est lui-même la condition qu'il existe pour
détecter.** `test-check-cta-primitives.sh` sait prouver que le scan rougit sur
un CTA hand-rolled. Il ne l'a jamais prouvé à personne, parce que personne ne
l'a lancé — et contrairement à une assertion sautée dans une suite qui tourne,
aucun avis de saut n'est émis : il n'y a pas de runner pour l'émettre.

## Le point non évident : le dépôt connaissait la classe, et l'avait refermée pour un membre

Ce n'est pas un angle mort. `scripts/check-shared-checkout-guard-wiring.sh`
(mika#2107) est une garde dont l'objet **est** de vérifier que le câblage d'une
autre garde tient. Son en-tête nomme la classe mot pour mot :

> POURQUOI IL EXISTE À CÔTÉ DU TEST COMPORTEMENTAL. La régression qu'il attrape
> ne rendrait AUCUNE décision fausse : elle rendrait la garde ABSENTE. Les 94
> assertions de `test-guard-shared-checkout.sh` resteraient vertes pendant que
> plus aucune session ne chargerait le hook — exactement la classe que ce dépôt
> a payée trois fois (mika#2205, un scan silencieusement inactif ; mika#2327, un
> réglage code-owned jamais écrit ; mika#2340, une bibliothèque rafraîchie en
> apparence). Un test comportemental ne peut pas voir cette panne-là.

*(Le « 94 » de cette citation est périmé : le harnais en question rend
aujourd'hui `219 ok, 0 fail`. Un nombre recopié à la main dans un commentaire ne
suit pas son sujet — même famille de silence que celle du doc, une couche plus
bas. Citer la commande vaut mieux que citer le compte.)*

Le diagnostic est juste, daté, et il cite trois paiements antérieurs. Mais le
remède qu'il a produit est **codé en dur sur un membre** : il vérifie le câblage
de `guard-shared-checkout`, et de rien d'autre. Il n'a donc pas protégé la garde
suivante, et il ne protégera pas la prochaine.

**Un remède local ne se généralise pas tout seul.** La classe était générale
(« une garde absente reste verte »), l'implémentation était particulière, et
l'écart entre les deux s'est refermé sur mika#1801, quatre tickets plus loin.

Le deuxième indice que l'angle mort est collectif se lit dans un doc voisin.
`structural-guard-fails-open-parser-fixture-harness.md` décrit trois artefacts
de sûreté et crédite explicitement les deux premiers de leur câblage — puis
écrit du troisième qu'il « is operator-run after a deploy and is referenced by
no Makefile target and no workflow ». La phrase est exacte. Elle énonce un
non-câblage comme une propriété neutre, à côté de deux artefacts dont le
câblage, lui, est mentionné comme une qualité. C'est le défaut de ce doc-ci,
déjà écrit dans l'arbre, sous la forme d'une condition acceptée.

## La règle

**Une garde se câble dans le diff qui la crée.** Un détecteur sans son job CI
n'est pas « livré, à câbler ensuite » : c'est un fichier. Le PR qui ajoute
`scripts/check-*.sh` ajoute dans le même commit sa strophe dans `ci.yml` et sa
cible `make`, ou il ne livre pas de garde.

Le modèle est dans l'arbre, et c'est celui que mika#1801 aurait dû recopier :
`check-byte-slices.sh` + `test-check-byte-slices.sh`, tous deux dans un job CI
**et** derrière une cible `make`. Deux étapes dans le job, jamais une : le scan,
puis son anti-vacuité — parce qu'une garde que personne n'a vue rougir est une
décoration (mika#2103).

**Et le sens de lecture est du runner vers le script, jamais l'inverse.** C'est
la discipline que `verify-which-script-ci-actually-invokes-2026-04-28.md` énonce
pour un workspace multi-dépôts (« the file you find by grep is not necessarily
the file CI executes »). La couche 0 en est le cas nul : CI n'exécute **aucune**
copie. Lire le script et le trouver bon ne dit rien de son exécution.

## Le geste de vérification

Deux commandes, sans substitution ni boucle, dont la comparaison est le seul
instrument qui voit la couche 0 :

```bash
ls -1 scripts/*.sh | sort
grep -oh 'scripts/[A-Za-z0-9._-]*\.sh' .github/workflows/*.yml Makefile | sort -u
```

Tout script de la première liste absent de la seconde est soit un défaut de
câblage, soit un geste d'opérateur — et **c'est la seule distinction qui
compte**.

Mesuré sur cet arbre le 2026-09-28 : **13 scripts non référencés**, dont neuf
sont des gestes d'opérateur ou de développement légitimes, plusieurs documentés
comme tels (`verify-dependabot-topology.sh` appelle `gh api` sur trois dépôts et
son doc écrit « ce n'est pas câblé au CI, à dessein » ; `audit-egress-no-log.sh`
lit une base vivante ; `pr-origin-report.sh` est déclaré dans `CLAUDE.md` §
Commands ; les `smoke-*`, `investigate-*`, `generate-openapi.sh`).

Les quatre autres sont **le même défaut que celui-ci**, et aucun n'est un geste
d'opérateur : ce sont des harnais qui ne demandent ni réseau, ni jeton, ni
service vivant, et que rien ne lance.

| harnais non câblé | ce qui le rend citable |
|---|---|
| `verify-pipeline-test.sh` | le dépôt documente lui-même l'écart (mika#2544), **et il a déjà coûté** : mika#2516 a annoncé une garde à l'intérieur de ce harnais, elle n'a jamais été livrée, et personne ne l'a vu |
| `test-smoke-search-substrate.sh` | son en-tête dit « it lets the suite run in CI with no gateway, no database, no token… » — et aucun job ne la lance |
| `test-measure-pilot-cycle-emptiness.sh` | son **jumeau** `test-measure-empty-turns.sh` est câblé (`Makefile:191`), lui non |
| `test-mika-groom-milestone.sh` | ne lit que des fichiers du dépôt |

Les quinze `scripts/check-*.sh` sont en revanche **tous** câblés :
`check-cta-primitives.sh` était la seule exception de sa famille, ce qui est
précisément pourquoi personne ne l'a cherchée.

## Pourquoi la garde de famille n'est pas livrée ici

La suite logique serait un scan refusant tout script de garde que ni `ci.yml` ni
le `Makefile` ne nomment. Elle n'est pas livrée ici, et **ce n'est pas faute de
population** : l'inventaire ci-dessus en trouve quatre autres, dont un que le
dépôt a déjà payé.

L'obstacle est le **discriminant**. Séparer « garde à câbler » de « geste
d'opérateur » n'a pas de forme lisible dans l'arbre : les deux sont des scripts
shell sous `scripts/`. Le scan aurait donc besoin d'une allowlist nommée, et
l'expérience maison sur les allowlists est écrite (mika#2201 : quand une garde
tire, on route le site, on n'allowliste pas) — ici la moitié « on route » n'a
pas de sens pour un geste d'opérateur, donc l'allowlist serait la sortie
normale, et une allowlist qui est la sortie normale se remplit.

Il y a un discriminant plus prometteur que le nom du fichier, et c'est
l'inventaire qui le suggère : les quatre écarts sont tous des **harnais qui
tournent hors-ligne** (ils simulent `gh`, le gateway, la base), là où les neuf
gestes légitimes ont tous besoin d'un service vivant, d'un jeton ou d'un pid.
« Ce script a-t-il besoin du monde extérieur ? » est une question que le scan ne
sait pas poser, mais qu'un humain tranche en lisant l'en-tête — donc la première
étape du suivi est un recensement annoté, pas un prédicat.

Le précédent maison pour ce genre de remède existe et il donne la méthode :
`docs/solutions/architecture-patterns/2026-09-19-audit-scanners-sources-structurels.md`
a recensé ~30 gardes structurelles avant d'en replier 13 sur une aide canonique.
**Recenser la population avant de réparer un membre** — c'est exactement l'étape
que mika#2107 n'a pas faite. Le suivi, s'il s'ouvre, s'ouvre avec ce recensement
et une décision explicite sur le discriminant, pas avec un scan écrit d'abord.

### Livrée depuis — mika#2761, 2026-10-18

La garde de famille existe : `scripts/check-suite-wiring.sh`, job CI
`suite-wiring-lint`. Elle a été livrée par la méthode que la section ci-dessus
annonçait, dans cet ordre : **le recensement annoté d'abord, le prédicat
ensuite**.

Le discriminant pressenti ici — « ce script a-t-il besoin du monde
extérieur ? » — est devenu le **vocabulaire de la colonne `mode`** de
`scripts/suite-wiring.tsv` : `operator` (réseau, jeton, service vivant, pid, ou
instrument de mesure délibéré) ou `unwired` (tourne hors-ligne, et rien ne le
lance — c'est l'arriéré). La question que le scan ne sait pas poser reste posée
à l'humain, qui remplit le champ ; la machine ne vérifie que la **forme** — mode
dans le vocabulaire fermé, raison non vide, ticket cité. C'est la discipline de
la colonne `tolérance` de `scripts/canonical-tokens.tsv`.

Ce qui ferme le piège de l'allowlist que cette section identifiait comme
l'obstacle, ce n'est pas la bonne volonté : c'est que le registre ne déclare que
le **résidu**, et que le lint rougit (`[D2]`) sur une entrée dont la suite est
devenue atteignable. Le geste qui rend une exception inutile est le geste qui la
fait échouer, donc une entrée ne peut pas survivre à sa raison.

Mesure refaite le 2026-10-18 sur `0170f5cd`, avec une définition plus large que
celle de 2026-09-28 (les deux racines `scripts/` et `skills/bundled/_shared/`,
les cinq préfixes `test-` `test_` `check-` `verify-` `measure-`, et
l'atteignabilité par fermeture transitive du `Makefile` et non par citation
directe) :

| population | compte |
|---|---|
| suites sur disque (`\|D\|`) | 94 |
| atteintes par un workflow, directement ou via `make` (`\|C\|`) | 67 |
| résidu déclaré (`\|R\|`) | 27 — dont **25 `unwired`**, 2 `operator` |

Deux corrections à la lecture ci-dessus, et aucune ne déplace sa conclusion :

- **Les quatre harnais nommés dans le tableau sont tous encore non câblés** au
  2026-10-18, trois semaines plus tard. Ils sont désormais les quatre premières
  lignes citables de l'arriéré, et non plus une observation sans tenue.
- **« Les quinze `scripts/check-*.sh` sont en revanche tous câblés » n'est plus
  vrai.** `check-release-debuginfo.sh` et son harnais sont sur disque, ont une
  cible `make`, et aucun workflow ne la nomme — exactement la situation de
  `check-cta-primitives.sh`, « la seule exception de sa famille, ce qui est
  précisément pourquoi personne ne l'a cherchée ». Une famille propre à une date
  ne reste pas propre : c'est l'argument de la garde structurelle, pas de
  l'inventaire.

### Câblée depuis — mika#2793, 2026-10-19 : et quatre des 25 étaient rouges

mika#2793 est la Fire-Disposition de l'arriéré que mika#2761 avait mesuré. Les
25 `unwired` sont câblées, chaque ligne retirée du registre dans le même commit
que son câblage (le lint l'y force), **les quatre harnais nommés par le tableau
ci-dessus en premier**. La colonne est vide : `unwired=0` dans la ligne de
rapport du lint. Le recensement du 2026-09-28 est clos.

Mais le fait qui vaut d'être écrit n'est pas la fermeture. C'est que **quatre
des 25 ne passaient pas au premier lancement** — et qu'aucune n'avait été
touchée depuis son écriture :

| suite | ce que son premier lancement a dit |
|---|---|
| `test_sandbox_log_dir_bound.sh` | 11/5. Son probe faisait `if : > "$chemin"` dans le sandbox : la redirection est exécutée par le shell avant toute commande, un `sh -c` non interactif quitte sur place quand elle échoue, et la branche `else` n'était jamais atteinte. **Le garde rougissait exactement quand l'isolation fonctionnait** — un chemin non bindé est le contrat qu'il garde |
| `test-mika-groom-milestone.sh` | 24/1. Deux assertions cherchaient un `llm_overrides` par skill que `6ccb79c4` (mika#949) avait supprimé. Le garde avait **perdu son sujet** : plus aucun site à asserter, donc un rouge permanent sur un producteur réaligné |
| `verify-pipeline-test.sh` | 29/0 ici, et `Author identity unknown` sur un runner neuf : ses 29 `git commit` de fixture héritaient de la config globale de la machine |
| `test-measure-pilot-cycle-emptiness.sh` | une assertion sur le bit `+x` d'un script que le dépôt déclarait `100644` |

Deux de ces quatre rougissaient sur du code **juste**, et deux auraient rougi
chez le premier lecteur. Dans les deux cas le résultat est le même : la première
personne à lancer la suite n'aurait pas lu un défaut, elle aurait lu du bruit —
et aurait appris à ignorer sa sortie.

Donc la couche 0 de la table n'est pas « la garde ne refuse rien ». C'est
**« la garde cesse de pouvoir refuser, et personne ne l'apprend »**. Un garde
non câblé ne se conserve pas en attendant son job CI : il dérive avec le code
qu'il observe, et trois semaines ont suffi pour deux de ces quatre. La fenêtre
où le câbler coûte une strophe de workflow se referme sur une fenêtre où il
coûte un diagnostic.

Et un corollaire direct contre la phrase du § *Contexte* : « lancés à la main,
ils font exactement ce qu'ils annoncent » n'est **pas** une preuve de
câblabilité. `verify-pipeline-test.sh` le montre dans l'autre sens — vert sur un
poste de développeur, rouge sur un runner neuf. Un lancement à la main mesure la
machine autant que le code ; seul un runner neuf sépare les deux.

Une dernière propriété, celle qui rend le compte honnête : aucune des 25 n'a été
basculée en `operator` et aucune n'a été supprimée. « Needs bwrap » n'est pas
« needs the outside world » — quatre suites à précondition d'environnement sont
allées dans le job `check`, le seul runner qui porte un `bwrap` réel, parce
qu'ailleurs elles auraient été vertes **par skip permanent**, c'est-à-dire
décoratives une couche plus haut. Un job vert ne distingue pas « 15 assertions
passées » de « ⊘ skipped », donc le câblage ne vaut que lu dans le log.

## Quand l'appliquer

- À l'écriture de tout script de garde : le câblage est dans le même diff. Ce
  n'est plus une consigne mais un refus — `suite-wiring-lint` rougit sur la
  suite qu'aucun workflow n'atteint et que le registre ne déclare pas.
- En revue d'un PR qui ajoute un détecteur : chercher le job CI dans le diff
  avant de lire le prédicat. Un prédicat parfait non câblé vaut zéro ; un
  prédicat imparfait câblé se corrige.
- À la troisième récurrence d'une classe que le dépôt croit gardée : vérifier
  que la garde **tourne** avant de suspecter son prédicat.
- Au câblage d'une garde restée sans runner : prévoir qu'elle soit rouge, et
  mesurer avant de conclure. Quatre des 25 de mika#2793 l'étaient, dont deux sur
  du code juste. Un rouge au premier lancement n'accuse pas le code — il dit
  seulement que la garde a dérivé sans témoin.
- À l'écriture d'une garde qui protège une propriété d'une autre garde :
  se demander si la propriété vaut pour une famille, et si oui, recenser la
  famille — sans quoi le remède ne dépassera pas son premier membre.

## Ce que ce doc n'achète pas

Aucun compteur, aucun détecteur. Le seul instrument est la comparaison
ci-dessus, et **son silence ne prouve rien tant que personne ne la lance** : un
inventaire que nul n'exécute est, très précisément, le défaut que ce doc décrit,
appliqué à lui-même.

## Voisins

- [`2103-a-guard-that-knows-one-spelling-of-a-defect-protects-nothing-else.md`](2103-a-guard-that-knows-one-spelling-of-a-defect-protects-nothing-else.md)
  — la couche 1 (prédicat trop étroit), et dans son remède #4 l'implémentation
  de référence correctement câblée que mika#1801 aurait dû recopier.
- [`trois-facons-dont-une-verification-passe-au-vert-sans-rien-mesurer-2026-09-22.md`](trois-facons-dont-une-verification-passe-au-vert-sans-rien-mesurer-2026-09-22.md)
  — trois mécanismes d'une vérification qui tourne sans mesurer. Sa table
  « mécanisme / ce qui l'attrape » est la taxonomie de cette famille ; la couche
  0 en est la ligne manquante.
- [`structural-guard-fails-open-parser-fixture-harness.md`](structural-guard-fails-open-parser-fixture-harness.md)
  — la couche 3 (fail-open interne), et la phrase qui enregistre un non-câblage
  comme une propriété neutre.
- [`verify-which-script-ci-actually-invokes-2026-04-28.md`](verify-which-script-ci-actually-invokes-2026-04-28.md)
  — le sens de lecture runner → script, dont la couche 0 est le cas nul.
- `docs/solutions/architecture-patterns/2026-09-19-audit-scanners-sources-structurels.md`
  — la méthode du recensement d'une famille de gardes, et l'asymétrie qui la
  motive : une garde trop large casse la CI dans l'heure, une garde qui a cessé
  de regarder reste verte.
- `scripts/check-shared-checkout-guard-wiring.sh` (mika#2107) — la classe
  diagnostiquée en prose, et le remède codé en dur sur un seul membre.
