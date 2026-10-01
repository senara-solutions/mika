# mika#2626 — `_push_branch` cesse de déduire l'existence distante d'une ref locale

> **Ticket :** senara-solutions/mika#2626 (`agent-core`, `dispatch:loop`) — tier 1
> **Incident fondateur :** 2026-10-01, pilote `624656b1` (mika#1960 phase 2), rescue `f2cc44bb`, 90,40 USD / ~770 lignes restés local-only
> **Liés :** mika#1364 (le lease `--force-with-lease`), mika#1857 (le retry), mika#1407 (la décision de push est au code, pas au pilote), mika#2158 et mika#2484 (deux prédicats divergents sur un même fait)
> **Consigne de groom :** MPC, 2026-10-01, après la mort du groom `93bac846` — repro sous `.pilot-scratch/`, une commande simple par appel, aucun `rm -rf`
> **Repro exécutée pendant ce groom :** `.pilot-scratch/repro-2626/` (dépôt bare + clone, `exercise.sh`), git 2.53.0 — le défaut est reproduit **au message près** et le remède est validé

## Le défaut, reproduit

Le fixture construit pendant ce groom place un clone dans l'état exact de
l'incident : `origin/test/1960/phase` pointe localement sur `991118307…`, le dépôt
bare n'a plus cette ref (supprimée comme GitHub le fait au merge), et HEAD a été
réécrite par le rescue. `_push_branch` y a été exercée pour de vrai :

```
local origin/test/1960/phase : 991118307cce9657bf8772d5f462d8c73eb3bc6d
local HEAD                   : 6c36b65202791a594b528ba7e6491020f79706d8
remote has the branch        : NO (orphaned local ref)

rev-parse --verify origin/$BRANCH : SUCCEEDS -> not first-push
merge-base --is-ancestor rc        : 1
=> push_mode = diverged  <-- THE DEFECT

 ! [rejected]        test/1960/phase -> test/1960/phase (stale info)
Push: FAILED — remote advanced since fetch (lease aborted); commits remain local-only on test/1960/phase
```

La dernière ligne est **mot pour mot** celle du ticket. Et un push nu sur le même
fixture, immédiatement après, rend `* [new branch]` — ce que MPC avait obtenu à la
main.

Le mécanisme du ticket est donc confirmé en entier : la ligne 5801 de
`skills/bundled/_shared/dispatch-lib.sh` demande à la **ref de suivi locale** si le
distant porte la branche, et une ref orpheline répond oui.

## Trois rectifications que la mesure impose au ticket

Ce sont les premiers livrables du groom : sans elles, l'implémentation suivrait
l'AC1 à la lettre et ne refermerait rien.

### R1 — l'option A de l'AC1 ne fonctionne pas

L'AC1 propose « `git fetch --prune origin "$BRANCH"` **ou**
`git ls-remote --exit-code` ». Les deux ne sont pas équivalentes : la première est
**inopérante**. Mesuré sur git 2.53.0, sur la ref orpheline du fixture :

| forme | rc | prune la ref orpheline ? |
|---|---|---|
| `git fetch origin "$BRANCH"` — le code actuel, l.5783 | 128 | non |
| **`git fetch --prune origin "$BRANCH"` — AC1 option A** | **128** | **NON** |
| `git fetch --prune origin refs/heads/X:refs/remotes/origin/X` | 128 | **NON** |
| `git fetch --prune origin` — nu, refspec par défaut | 0 | **oui** |
| **`git ls-remote --exit-code origin refs/heads/X` — AC1 option B** | **2** | n/a, ne mute rien |

La cause est que `--prune` n'est **jamais atteint** : git échoue d'abord sur
`fatal: couldn't find remote ref test/1960/phase`. Après les deux formes A, la ref
orpheline est encore à `991118307…`, vérifié par `rev-parse`.

Seule la forme **nue** prune — et elle est écartée pour trois raisons, dont la
troisième décide :

1. Elle fetche **toutes** les branches du dépôt à chaque dispatch.
2. Elle **mute les refs de suivi du common dir**, partagé par tous les worktrees du
   checkout : elle prunerait aussi les refs d'autres dispatches en vol. Effet de
   bord hors périmètre, sur la structure même que l'incident a mise en cause.
3. **Elle ne répond pas à la question posée.** La question est « le distant
   porte-t-il cette branche *maintenant* ? ». `ls-remote` la pose directement ;
   pruner est un moyen détourné d'y répondre en modifiant l'état local.

**Le remède retenu est donc l'option B**, et le `fetch origin "$BRANCH"` de la
l.5783 n'est pas touché : il reste utile quand la branche existe, et son `|| true`
absorbe déjà le cas où elle n'existe pas.

### R2 — le prédicat juste existe déjà dans ce fichier, à 2 300 lignes de là

`_set_up_worktree` pose **exactement** la bonne question, l.3457 :

```sh
if git -C "$SUB_REPO_DIR" ls-remote --exit-code origin "refs/heads/$BRANCH" >/dev/null 2>&1; then
```

Dans l'incident, cette question a rendu « absente » — c'est pourquoi le worktree a
été basé sur `origin/main`. Puis `_push_branch`, dans le même fichier et le même
dispatch, a posé la **même** question par un prédicat **différent** et a rendu
« présente ».

**Deux prédicats divergents sur un même fait**, et c'est le motif que la maison a
déjà documenté deux fois : mika#2158 (« promotion et routage de dispatch
répondaient différemment à la même question », pendant des mois, sans que rien
casse) et mika#2484 (« une divergence à quatre étapes d'écart dans le même
handler »). Conséquence de conception : le remède est un **alignement**, pas une
invention, et sa forme est un **lecteur unique** — motif `_extract_plan_path`
(mika#2608), `parse_pr_target` (mika#2368), `grooming_marker` (mika#2158).

### R3 — le retry de mika#1857 aggrave le défaut, il ne le sauve pas

Non mentionné par le ticket, mesuré dans la repro. La sortie complète porte :

```
_push_with_rebase_retry: race-shaped rejection on attempt 1/2 — fetching + rebasing
_push_with_rebase_retry: rebase succeeded on attempt 1 — retrying push
_push_with_rebase_retry: exhausted 2 attempts on race errors — bailing to draft rescue
```

`! [rejected] … (stale info)` matche le grep race-shape (`rejected|fetch first|remote
contains work`), donc le retry fetche `origin/main`, **rebase l'historique local**,
retente le lease et épuise ses deux tentatives. Le défaut coûte donc **deux pushes
et un rebase gratuit**, et il **mute l'historique local** sur la foi d'un faux
diagnostic.

Rien ici ne modifie ce retry : une fois le mode corrigé, cette population ne
l'atteint plus. La mesure est écrite parce qu'elle explique le coût observé et
parce qu'elle est la raison de ne pas y toucher.

## Conception

### R-a — un lecteur unique du fait « le distant porte cette branche »

`_remote_branch_exists <repo_dir> <branch>`, placé près des helpers de push, avec
**trois** issues et non deux — la sémantique du rc est **contractuelle**, pas
observée par accident (`git ls-remote --help` : *« Exit with status "2" when no
matching refs are found in the remote repository. Usually the command exits with
status "0" to indicate it successfully talked with the remote repository »*) :

| rc de `ls-remote --exit-code` | issue | mesuré |
|---|---|---|
| `0` | **présente** | fixture avant suppression |
| `2` | **absente** | fixture après `update-ref -d` dans le bare |
| autre (`128`, …) | **indéterminée** — le distant n'a pas répondu | remote inexistant |

**Disposition de l'issue indéterminée, et elle est nommée plutôt que choisie par
défaut : on retombe sur le prédicat d'aujourd'hui** (la ref locale), avec une ligne
qui le dit. C'est la règle maison « un signal qu'on ne peut pas lire n'est jamais un
terme satisfait » (mika#2277) appliquée dans les deux sens : traiter un réseau coupé
comme « absente » serait affirmer une mesure qu'on n'a pas faite. Le coût est borné
et il est exactement le comportement courant.

Les deux appelants sont l.3457 (`_set_up_worktree`, qui délègue — son prédicat ne
change pas de valeur) et l.5801 (`_push_branch`, nouveau). Le lecteur prend le
répertoire en argument : les deux sites n'opèrent pas sur le même (`$SUB_REPO_DIR`
contre `$WORKTREE_DIR`).

**Ne pas unifier avec les deux autres `ls-remote` du fichier** (l.3682
`PRE_RUN_REMOTE_HEAD`, l.5741 `_check_pilot_force_push`) : ils lisent le **SHA**,
pas l'existence. Question différente, prédicat différent.

### R-b — la décision de mode consulte le distant

À la l.5801, `push_mode` est décidé ainsi :

- distant **présent** → comportement actuel inchangé, y compris la sonde d'ancêtre
  et `diverged` → `--force-with-lease` (c'est l'AC3) ;
- distant **absent** → `first-push`, quoi que dise la ref locale. C'est l'AC1 ;
- **indéterminé** → prédicat local, comme aujourd'hui.

Le court-circuit `ahead == 0 → return 0` (l'état (a) de mika#1407, « rien à
pousser ») doit rester **sous la branche « présent »** : sur un distant absent,
`origin/$BRANCH..HEAD` compte contre une ref orpheline et peut rendre 0 alors
qu'il y a tout à pousser. C'est la seconde moitié silencieuse du défaut, et un
correctif qui ne déplacerait que le choix de mode la laisserait ouverte.

### R-c — le diagnostic cesse d'affirmer une avance distante qui n'a pas eu lieu

`Push: FAILED — remote advanced since fetch` était un **énoncé faux** : le distant
n'avait aucune ref. C'est la classe mika#2304 — un champ qui affirme, avec
autorité, ce qui n'a pas eu lieu. Deux moitiés, et l'AC4 demande la seconde :

1. **À la détection** (chemin de **succès** compris) : une ligne nommant la ref
   orpheline et son SHA. Un checkout partagé qui porte une ref périmée est une
   anomalie d'hygiène qui mérite d'être comptée, même quand le push aboutit.
2. **Dans la branche FAILED** : la classification « remote advanced since fetch »
   est **conditionnée à l'existence distante**. Sur un distant absent, le message
   nomme la ref orpheline au lieu d'inventer une avance.

### Ce qui n'est pas touché, et pourquoi

- **`_push_with_rebase_retry`** (R3) — une fois le mode corrigé, cette population ne
  l'atteint plus.
- **Le passage au lease après un rebase réussi** (l.5962-5964), qui s'applique quel
  que soit le mode initial. Sur une branche absente du distant ce lease
  re-échouerait — mais y arriver demande qu'un push **nu** de first-push soit
  d'abord rejeté en race-shape, ce qu'aucune mesure ne montre. **Limite nommée, non
  traitée** : la traiter demanderait de décider ce qu'un lease veut dire sur une ref
  qui n'existe pas, et aucune population ne le demande.
- **`NO_PR: rescue_pr_create_failed`** (l.10034) — c'est une **cascade** du push
  échoué : branche absente du distant → `gh pr create` échoue → `_pr_list_url` vide
  → ce motif. Il se referme avec le push ; aucune ligne à y écrire.

## Tâches

1. **`_remote_branch_exists`** dans `skills/bundled/_shared/dispatch-lib.sh` :
   trois issues, doc-comment portant la table de rc et la justification de la
   disposition indéterminée.
2. **`_set_up_worktree` l.3457 délègue** au lecteur. Aucun changement de valeur de
   vérité — c'est ce qui rend le pas sûr et ce que son test de non-régression doit
   établir.
3. **`_push_branch` l.5801** : décision de mode par R-b, court-circuit `ahead == 0`
   déplacé sous la branche « présent ».
4. **Diagnostic** par R-c, aux deux sites.
5. **Tests** `Test mika#2626` dans `skills/bundled/_shared/test-dispatch-lib.sh`
   (convention récente du fichier : un bloc nommé par ticket).

### Note d'implémentation sur le fixture — non évidente, et elle décide du test

Les helpers `_fixture_setup` / `_fixture_cleanup` / `_assert_fixture_is_local`
existent déjà (l. ~2196) et sont réutilisés tels quels : rien à construire.

Pour créer la ref orpheline, supprimer la branche **dans le bare** :

```sh
git -C "$FIXTURE_BARE" update-ref -d "refs/heads/$br"
```

**Jamais `git push origin --delete`** depuis le clone : git y supprimerait aussi la
ref de suivi locale, c'est-à-dire précisément le fixture qu'on veut construire. La
forme `update-ref -d` est celle qui a été validée pendant ce groom, et c'est aussi
la plus fidèle à l'incident : GitHub supprime la branche, le checkout partagé ne le
sait pas.

Le harness installe `init.defaultBranch=main` par `GIT_CONFIG_*` (mika#1772). La
repro manuelle de ce groom ne l'avait pas et a obtenu `master` — à l'intérieur du
harness, le couplage est déjà neutralisé.

## Verification contract

**V1 — le test AC2 est vu ROUGE avant le correctif, et pour la bonne raison.**
Écrire le test d'abord, lancer `make test-dispatch-lib`, et vérifier que le rouge
dit `mode=diverged` **ou** `Push: FAILED`. Un rouge pour une autre raison (fixture
cassé, `main` absent, remote non local) ne vaut **pas** confirmation du défaut :
c'est le piège que cette ligne existe pour fermer. Le rouge attendu est celui que la
repro de ce groom a produit, à l'identique.

**V2 — AC2 vert après le correctif :** `push_mode=first-push`, push réussi, et la
tête distante égale à HEAD locale.

**V3 — AC3 non-régression :** les Tests 12a (first-push), 12b (fast-forward), 12c
(diverged → lease), 12d (forme du lease), 12f (dedup-rebase → diverged) et 12h
(stale-main, mika#1407) passent sans modification. Si l'un d'eux doit être édité, la
décision de mode a changé de valeur sur une population qu'il protège — **halte**.

**V4 — AC4 :** sur un distant absent, le `RESULT` ne contient pas « remote advanced
since fetch » ; le diagnostic nomme la ref orpheline. Sur un distant **présent** qui
a réellement avancé, le message est conservé mot pour mot.

**V5 — lecteur unique :** scan de source refusant un second `ls-remote --exit-code`
sur `refs/heads/` hors du lecteur, **plus** une assertion d'anti-vacuité — exactement
deux appelants. Sans ce second terme, un scan devenu aveugle se lit comme un arbre
propre (classe mika#2205).

**V6 — le court-circuit déplacé :** un test où le distant est absent et où
`origin/$BRANCH..HEAD` compte 0 contre la ref orpheline doit **pousser** quand même.
C'est la vérification de la seconde moitié de R-b, et elle est distincte de V2.

**V7 — suite complète :** `make test-dispatch-lib` vert de bout en bout.
L'invocation est une commande simple ; le `rm -rf` de `_fixture_cleanup` vit à
l'intérieur du script, pas dans une ligne de commande du pilote.

## Fire-Disposition

Ce plan livre des détecteurs : les tests du bloc `Test mika#2626` (V1-V6) et le scan
de source de lecteur unique (V5).

**Disposition retenue : (a) exception nommée en allowlist — et l'allowlist est
livrée VIDE.**

Après le pas 2, le seul `ls-remote --exit-code` du fichier hors du lecteur est la
l.3457, qui **devient un appelant** : il n'y a donc aucune violation à excepter, et
rien à inscrire. C'est la doctrine mika#2201 appliquée à sa lettre — *on déclare, on
n'allowliste pas* : quand ce scan tirera, la résolution est de router le site vers
le lecteur, jamais d'ajouter une ligne.

Deux précisions qui font la différence entre une allowlist vide et un détecteur
inerte :

- **Le prédicat porte sur `--exit-code`**, la question d'existence, et non sur
  `ls-remote` en général. Sans cette restriction, les l.3682 et 5741 — qui lisent un
  **SHA** — seraient dénoncées à tort, et un scan qui dénonce le travail légitime se
  fait désarmer.
- **L'anti-vacuité est une assertion, pas un commentaire** : le scan échoue si le
  lecteur compte autre chose que deux appelants. Un prédicat devenu trop étroit
  passerait sinon en ne regardant rien.

Les tests, eux, ne sont pas livrés désarmés : V1 prescrit de les voir rouges avant
le correctif, ce qui est l'inverse d'un `#[ignore]`.

**Aucun interrupteur d'environnement.** Le changement de comportement est borné à la
population du défaut (distant absent, ref locale présente), et son pire cas est un
push **nu** — qui ne peut rien écraser, git refusant un non-fast-forward. Un
kill-switch serait un levier sans population, et un levier de plus à documenter sur
le chemin de publication d'un travail payé.

## Definition of Done

- `_remote_branch_exists` existe, est le seul lecteur de l'existence distante, et
  porte sa table de rc en doc-comment.
- `_push_branch` ne peut plus produire `diverged` sur une ref locale orpheline, et
  son court-circuit `ahead == 0` ne s'applique plus qu'à un distant présent.
- Le diagnostic n'affirme « remote advanced since fetch » que si le distant existe.
- `make test-dispatch-lib` vert, Tests 12a-12h **non modifiés**.
- Le test AC2 a été vu rouge avant le correctif, avec le message attendu.

## Acceptance criteria

Transcrits du corps de mika#2626.

- **AC1.** Avant de choisir le mode, `push_branch` établit l'existence **distante**
  de la branche : `git fetch --prune origin "$BRANCH"` ou
  `git ls-remote --exit-code`. Une ref locale orpheline ne peut plus produire
  `diverged`.
  → Livré par `ls-remote --exit-code` (R-a, R-b). **La première forme est refusée sur
  mesure** : R1 établit qu'elle ne prune pas.
- **AC2.** Test : une ref `origin/$BRANCH` locale périmée et une branche absente du
  distant donnent `push_mode=first-push`, puis un push réussi. Ce test est **vu
  rouge** avant le correctif.
  → V1 (rouge, et le message qu'il doit porter) puis V2 (vert).
- **AC3.** Le cas légitime de divergence (le distant existe et a avancé) garde son
  `--force-with-lease`. Test de non-régression.
  → V3, par la préservation sans édition des Tests 12a-12h.
- **AC4.** Le message « remote advanced since fetch » n'est émis que si le distant
  existe ; sinon le diagnostic nomme la ref orpheline.
  → R-c et V4.

## Hors périmètre, délibérément

- **`_push_with_rebase_retry`** et son passage au lease post-rebase (R3, et la limite
  nommée ci-dessus).
- **Le `fetch origin "$BRANCH"` de la l.5783** : correct quand la branche existe, et
  son `|| true` couvre déjà l'autre cas. R1 montre qu'y ajouter `--prune`
  n'achèterait rien.
- **`NO_PR: rescue_pr_create_failed`** — cascade, se referme avec le push.
- **L'hygiène des refs de suivi du checkout partagé.** Rien ici ne prune quoi que ce
  soit : le correctif cesse de *croire* la ref orpheline, il ne la supprime pas. Un
  `fetch --prune` périodique sur le checkout partagé est une décision distincte, dont
  le rayon de souffle est le common dir partagé par tous les worktrees en vol.
  **Suivi**, et sa précondition est la ligne de diagnostic de R-c : elle compte la
  population avant qu'on décide d'y toucher.
- **La réutilisation d'un nom de branche entre phases**, que le ticket nomme comme le
  déclencheur structurel. C'est une pratique adoptée le 2026-10-01 et elle est
  légitime ; ce qui était cassé est le push, pas la pratique.

## Ce que ce travail n'achète PAS

- **Il ne rattrape pas l'incident du 2026-10-01.** Les commits `f2cc44bb` ont été
  poussés à la main par MPC et la PR #2625 est mergée. Rien ici ne rétro-écrit quoi
  que ce soit ; la sonde est la **prochaine** réutilisation de nom de branche.
- **Il n'ajoute aucun compteur ni aucune ligne d'`audit_events`.** `_push_branch` est
  du shell dans un sous-processus sans accès base, et son stderr d'avant-pilote est
  structurellement perdu sur un dispatch qui réussit (classe mika#2050, Signaux M et
  Q : le `Stdio::piped()` que l'exécuteur ne lit que dans `if !status.success()`).
  Inventer une surface de journal qui ne serait pas lue reproduirait le défaut du
  Signal M. **Les deux surfaces réelles sont le `RESULT` du callback — lisible par
  `mika tasks get <id>` et par `SELECT result FROM tasks` — et le `.stderr`
  par-dispatch sous `${PILOT_LOG_DIR:-/var/log/claude-pilot}/`, à grepper ancré sur
  `^dispatch-lib: ` (mika#2050 a mesuré le faux positif d'un grep nu : la prose d'un
  pilote qui *discute* le signal y est aussi).**
- **Il ne purge aucune ref périmée** et ne rend donc pas le checkout partagé sain —
  il le rend inoffensif pour la décision de push.
- **Il ne surveille rien.** Le seul instrument neuf est le diagnostic de R-c, et
  **son silence ne prouve rien tant que personne ne le lit** : sur une population qui
  n'apparaît qu'à la réutilisation d'un nom de branche, l'absence d'occurrence peut
  simplement vouloir dire qu'aucune phase n'a été rejouée cette semaine.

## Sondes post-déploiement, et leurs haltes

> **Préalable.** `skills/bundled/_shared/` est une projection du **binaire**, pas du
> checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer` doit porter le sha
> qu'on vient de bâtir — sans quoi chacune des sondes décrit le binaire d'hier.

**S1 — le rejeu du défaut fondateur** (première phase 2 sur une branche réutilisée
après merge). Attendu : `Push: pushed to origin/<branche> (mode=first-push)` dans le
`RESULT`, plus la ligne de diagnostic nommant la ref orpheline.
*Halte 1 — `mode=diverged` réapparaît :* ne pas toucher au prédicat par réflexe.
Établir d'abord que le binaire servi porte le correctif, puis lire si `ls-remote` a
rendu `128` (indéterminé) plutôt que `2` — dans ce cas la cause est le réseau ou
l'authentification, et le repli sur le prédicat local est le contrat, pas un défaut.

**S2 — contrôle négatif du lease (7 jours).** `mode=diverged` doit **continuer**
d'apparaître sur les re-dispatches légitimes, où le distant porte la branche.
*Halte 2 — plus aucun `mode=diverged` sur 7 jours :* le prédicat mord trop large et
`first-push` est rendu pour une population qui a besoin du lease. Vérifier d'abord
qu'un re-dispatch a bien eu lieu — **zéro `diverged` et zéro re-dispatch ne prouve
rien** (classe mika#2205).

**S3 — contrôle positif, obligatoire.** Compter les dispatches qui ont poussé, quel
que soit le mode. Sans ce compte, « zéro échec de push » et « aucun push n'a eu
lieu » rendent les mêmes octets.
*Halte 3 — zéro push et zéro échec :* la sonde ne regarde pas là où dispatch-lib
écrit. Comparer `PILOT_LOG_DIR` et `MIKA_PILOT_LOG_DIR`, dont la divergence est déjà
documentée (mika#2249), avant toute conclusion.

**Halte transverse.** Aucune de ces sondes n'est exécutable depuis un bac à sable de
dispatch : elles demandent le `.stderr` de l'hôte ou la base. Ce sont des **gestes
d'opérateur**, déclarés comme tels.
