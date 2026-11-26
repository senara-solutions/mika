# mika#3179 — geler la taille de `test-dispatch-lib.sh`, et dire où va le prochain bloc

## Le fait, mesuré sur la branche

```
$ wc -c skills/bundled/_shared/test-dispatch-lib.sh
1041079 skills/bundled/_shared/test-dispatch-lib.sh
$ grep -n 'LARGE_FILE_LIMIT=' scripts/check-secrets.sh
23:LARGE_FILE_LIMIT=1048576  # 1 MB, in bytes
```

Marge restante : **7 497 octets**. Le fichier a pris ~200 Ko en un mois.

Le défaut n'est pas la taille — c'est **le moment où le pilote l'apprend**. Sur #3177, un
bloc de 238 lignes ajouté à l'agrégat a fait rougir `Secret Scan` (run 45856244085), après
le push, sur un message qui parle d'un blob trop gros et ne dit rien de l'endroit où placer
un test. Une itération entière a servi à déplacer le bloc dans
`skills/bundled/_shared/tests/test_callout_plan_identity_3174.sh`. Rien dans le dépôt ne
prévient le prochain vol : il refera le même détour, quel que soit son ticket.

La scission complète de l'agrégat est **#3178**, ouverte en dormeur, en attente de deux
décisions de l'opérateur. Ce vol ne la remplace pas et ne la préempte pas : il rend le défaut
non reproductible pendant qu'elle attend.

## Ce qui est déjà en place, et ne doit pas être refait

- **La destination existe.** `skills/bundled/_shared/tests/` porte 39 suites, une par maillon,
  et `test_callout_plan_identity_3174.sh` y est le précédent exact : son en-tête explique en
  toutes lettres *pourquoi* elle ne vit pas dans l'agrégat (lignes 12–18).
- **Le câblage existe.** `Makefile:200` `test-dispatch-lib` est la cible que `ci.yml:85` nomme
  (`Dispatch-lib assertion suite (mika#1772)`), et elle porte déjà, depuis #3174, la ligne
  `tests/test_callout_plan_identity_3174.sh` avec son commentaire de quatre lignes qui raconte
  le même incident. **Aucun nouveau step de `ci.yml` n'est dû** — mika#2578 vaut dans l'autre
  sens aussi : une cible qu'un workflow nomme déjà est une porte de merge.
- **La paire `check-*` / `test-check-*` est un motif établi**, avec une forme stable :
  `check-pilot-turn-ceiling-labels.sh` (217 l.) / `test-check-pilot-turn-ceiling-labels.sh`
  (222 l.), `check-root-files.sh` (166 l.) / `test-check-root-files.sh` (204 l.). Le garde
  accepte des arguments optionnels pour que le harnais le pointe vers des fixtures ; le harnais
  bâtit ses fixtures **depuis les vraies données** pour qu'une donnée réelle vidée par accident
  rougisse au lieu de passer.
- **`scripts/check-suite-wiring.sh` est vert** (`|D|=113 (floor 85) |C|=111 |R|=2 unwired=0`).
  Les deux fichiers livrés portent les préfixes `check-` et `test-`, donc ils entrent dans son
  énumération : s'ils ne sont pas joignables depuis un workflow, il rougit. Le câblage dans
  `test-dispatch-lib` est ce qui les rend joignables.

## Le défaut qui reste ouvert

Rien, en local, ne mesure la taille de l'agrégat. Le seul lecteur est `check-secrets.sh`, qui
tourne en CI sur le diff, et dont le message est fait pour un fixture binaire, pas pour un
fichier de tests shell :

```
ERROR: <f> is 1028KB — exceeds 1MB limit (add to LARGE_FILE_ALLOWLIST with a named reason if legitimate)
```

Lu par un pilote, ce message **suggère l'exact contraire de ce qu'il faut faire** : il propose
l'allowlist. Un pilote qui l'écoute ajoute une entrée, la PR passe, l'agrégat continue de
croître, et #3178 devient d'autant plus coûteuse. C'est le vrai coût du message actuel, et il
justifie un lint dédié plus qu'un simple gain de latence.

## La décision de forme : un PLAFOND STATIQUE, pas un diff contre la base

Deux formes étaient disponibles.

| forme | ce qu'elle mesure | pourquoi elle est écartée / retenue |
|---|---|---|
| diff contre `origin/main` | la croissance *introduite par la PR* | dépend de la profondeur du checkout et de la présence de la ref ; elle ne dit rien en local sur une branche fraîche, et elle est muette dans `make test` hors d'un clone complet |
| **plafond statique** (retenue) | la taille absolue du fichier | **une seule donnée, aucune dépendance** : la même mesure en local, dans `make test`, et en CI. Le fichier suffit à se juger |

C'est aussi la forme qui survit à #3178 : le jour où l'agrégat est scindé et supprimé, le lint
sort en 0 et **se dénonce lui-même comme devenu inutile** dans son message vert, au lieu de
rougir sur une ref absente.

### Les quatre sorties du lint, et la direction du fail-safe

| état | sortie | raison |
|---|---|---|
| taille ≤ `AGGREGATE_FROZEN_BYTES` | 0 | nominal ; le message vert nomme la mesure ET la constante |
| taille > `AGGREGATE_FROZEN_BYTES` | **1** | la violation que le lint existe pour refuser |
| fichier absent | 0 | #3178 a pu le supprimer. Le message vert **dit** que c'est cette branche-là qui a été prise, et que le lint peut être retiré |
| fichier présent, mesure impossible | **2** | un garde qui ne peut pas mesurer ne passe pas : il le dit (`docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md`) |

La distinction entre les deux verts est la part la plus importante de ce tableau. Un lint qui
sort 0 sans dire *lequel* des deux verts il a rendu est un lint qui passe vert sur un fichier
renommé — et c'est précisément la panne silencieuse de mika#2578.

## Ce que livre ce vol

### 1. `scripts/check-dispatch-lib-aggregate-frozen.sh`

```
Usage: check-dispatch-lib-aggregate-frozen.sh [aggregate-path]
```

L'argument par défaut est le vrai fichier ; il n'existe **que** pour que le harnais pointe le
garde vers une copie temporaire (AC2 et les quatre cas d'AC3 en dépendent). La constante reste
**interne au script** : si le harnais pouvait la passer, les cas de test ne mesureraient plus
la vraie constante, et le cas vert deviendrait une tautologie.

```bash
AGGREGATE_FROZEN_BYTES=<taille mesurée au dernier commit du vol>
```

En-tête du script, dans la forme des gardes voisins : le fait mesuré (taille, plafond, marge),
l'incident fondateur (#3177, le run), **pourquoi le message de `check-secrets.sh` ne suffit
pas** (il propose l'allowlist), le renvoi à #3178, et la consigne « on ne relève pas la
constante : on crée une suite dans `tests/` ».

Le message d'échec (exit 1) nomme, dans cet ordre :

1. la taille mesurée et la constante, avec le delta en octets ;
2. **le geste** — placer le bloc dans une suite neuve
   `skills/bundled/_shared/tests/test_<sujet>_<ticket>.sh`, sur le modèle de
   `tests/test_callout_plan_identity_3174.sh`, puis la câbler dans les cibles `test` et
   `test-dispatch-lib` du Makefile ;
3. **le non-geste** — ne pas relever la constante, ne pas ajouter d'entrée à
   `LARGE_FILE_ALLOWLIST` ;
4. #3178, comme la seule sortie légitime du plafond.

### 2. `scripts/test-check-dispatch-lib-aggregate-frozen.sh`

Harnais anti-vacuité, dans la forme de `test-check-root-files.sh` : compteurs `PASS`/`FAIL`,
`assert_exit`, `assert_output_contains`, une racine `mktemp -d` unique nettoyée par `trap`.
Il ne touche **jamais** le vrai fichier : chaque cas travaille sur une copie.

Les quatre cas du ticket :

| cas | fixture | attendu |
|---|---|---|
| **C1** taille égale | copie telle quelle du vrai agrégat | exit 0 |
| **C2** un octet de plus | copie + 1 octet | exit **1**, et le message contient la taille, la constante, `#3178`, `skills/bundled/_shared/tests/` |
| **C3** fichier plus petit | copie tronquée | exit 0 |
| **C4** fichier absent | chemin inexistant sous la racine de fixtures | exit 0, et le message vert nomme la branche « absent » |

Deux contrôles en sus, qui sont ce qui empêche le harnais d'être vert sur rien :

- **P — population.** `wc -c` du vrai agrégat est lu, et comparé à `AGGREGATE_FROZEN_BYTES`
  extrait du garde par `sed`. Égalité exigée. Ce contrôle est la raison pour laquelle C1 est
  un vrai vert et pas une tautologie : si l'extraction ne rend rien, ou si la constante a
  dérivé de la taille réelle, le harnais sort en **3** (vacuité) au lieu de passer.
- **D — dérive.** `AGGREGATE_FROZEN_BYTES` est comparée au `LARGE_FILE_LIMIT` que
  `scripts/check-secrets.sh` déclare, **lu par `sed`, jamais sourcé** (le motif de
  `check-pilot-turn-ceiling-labels.sh`, qui parse `dispatch-lib.sh` sans l'exécuter). Si la
  constante gelée atteint ou dépasse le plafond, le harnais rougit : à ce point le gel ne
  protège plus de rien, puisque `Secret Scan` rougirait d'abord. `check-secrets.sh` est **lu
  et non modifié** — le hors-périmètre du ticket porte sur sa modification.

  C'est l'unique mode de défaillance réel du lint : **personne ne peut ajouter 1 octet, mais
  n'importe qui peut relever la constante.** Le contrôle D est le seul endroit du dépôt où
  cette relève cesse d'être invisible. Il sort en 3 si l'une des deux constantes n'est plus
  extractible — un rouge sur un renommage est le lockstep voulu, pas un faux positif.

### 3. L'en-tête de `test-dispatch-lib.sh` — et la marge qu'il consomme

Un bloc de commentaire ajouté **après la ligne `#!`**, qui dit : le fichier est gelé, le lint
qui le tient (`scripts/check-dispatch-lib-aggregate-frozen.sh`), #3178, et le chemin des
suites neuves (`skills/bundled/_shared/tests/test_<sujet>_<ticket>.sh`).

**Ce bloc mange la marge, et c'est pourquoi l'ordre de commit n'est pas libre.** La marge est
de 7 497 octets ; un bloc de 8 à 10 lignes en coûte ~700. `AGGREGATE_FROZEN_BYTES` est posée
**après** son écriture, sur la taille finale, et vérifiée une dernière fois avant le push :
une constante posée avant l'en-tête rend le lint rouge sur son propre vol.

### 4. Le câblage

Dans `Makefile` :

- `test-dispatch-lib` (la cible que `ci.yml:85` nomme) reçoit les deux lignes, avec un
  commentaire court renvoyant à #3179 et #3178 — dans la forme du commentaire #3174 déjà
  présent juste au-dessus ;
- `test` reçoit les deux mêmes lignes, parmi les autres paires `check-*` / `test-check-*` qui
  y figurent déjà (`check-pilot-turn-ceiling-labels.sh` + son harnais, lignes 186–187).

Aucun nouveau step `ci.yml`, aucune entrée `scripts/suite-wiring.tsv` : les deux fichiers sont
joignables depuis un workflow, donc une entrée dans le registre **ferait rougir**
`check-suite-wiring.sh` (il refuse une entrée pour une suite que la CI atteint).

## Tests

`bash scripts/test-check-dispatch-lib-aggregate-frozen.sh` — 6 contrôles (C1–C4, P, D), aucun
réseau, aucun jeton, aucun binaire, et le vrai agrégat lu en lecture seule.

**Chaque cas doit avoir été vu rouge une fois** (AC3), par faussage temporaire puis
rétablissement. La trace va dans le corps de PR :

| cas | faussage | rouge attendu |
|---|---|---|
| C1 | constante du garde − 1 | C1 rouge (et P rouge) |
| C2 | prédicat de comparaison inversé (`>` → `<`) | C2 rouge |
| C2 (message) | `#3178` retiré du message d'échec | C2 rouge sur l'assertion de contenu |
| C3 | prédicat `≤` → `==` | C3 rouge |
| C4 | branche « absent » renvoyant 1 | C4 rouge |
| P | `sed` d'extraction pointé sur un nom de variable inexistant | P rouge (exit 3) |
| D | constante du garde posée à 2 000 000 | D rouge |

Et la vérification de bout en bout, sur une copie sous `.pilot-scratch/` (AC2) :

```
cp skills/bundled/_shared/test-dispatch-lib.sh .pilot-scratch/aggregate-plus-one.sh
printf '#' >> .pilot-scratch/aggregate-plus-one.sh
bash scripts/check-dispatch-lib-aggregate-frozen.sh .pilot-scratch/aggregate-plus-one.sh  # exit 1
```

Le brouillon reste en place (mika#2548).

Enfin : `make test-dispatch-lib`, `bash scripts/check-suite-wiring.sh`,
`bash scripts/check-secrets.sh --changed origin/main`, `bash scripts/verify-pipeline.sh origin/main`.

## Ce que ce vol ne corrige PAS

- **La scission de l'agrégat (#3178).** Le fichier reste à ~1,04 Mo et à 39 suites voisines
  près de lui. Le gel borne la dette, il ne la rembourse pas.
- **`check-secrets.sh`, `LARGE_FILE_LIMIT`, `LARGE_FILE_ALLOWLIST`.** Aucun octet modifié. Le
  contrôle D **lit** `LARGE_FILE_LIMIT` ; il ne l'écrit pas, et l'allowlist reste vide.
- **Le message de `check-secrets.sh`,** qui continuera de proposer l'allowlist si un autre
  fichier dépasse 1 Mo. Le lint livré ici ne couvre qu'un chemin.
- **`dispatch-lib.sh`.** Intouché.
- **Les 39 autres suites de `tests/` et toutes les suites de `scripts/`.** AC5 : aucun autre
  fichier de suite modifié.
- **La relève de la constante par une PR future.** Le contrôle D la borne par le plafond de
  `check-secrets.sh`, mais ne l'interdit pas en deçà : une PR peut relever la constante de
  quelques Ko et passer. Ce qu'elle ne peut pas faire, c'est le faire **sans que le diff le
  montre** — et c'est tout ce qu'un plafond statique peut promettre.

## Ordre de commit

Un seul commit, mais un ordre d'écriture contraint — la constante dépend de l'en-tête.

1. **L'en-tête de `test-dispatch-lib.sh`** (le fichier grossit).
2. **`wc -c`** sur le fichier obtenu → c'est la valeur de `AGGREGATE_FROZEN_BYTES`.
3. **`scripts/check-dispatch-lib-aggregate-frozen.sh`**, avec cette valeur. Vérification
   immédiate : vert sur le vrai fichier (AC1), rouge sur la copie +1 octet (AC2).
4. **`scripts/test-check-dispatch-lib-aggregate-frozen.sh`**, puis la série de faussages
   d'AC3, chacun rétabli.
5. **`Makefile`** — les deux cibles.
6. `make test-dispatch-lib`, `check-suite-wiring.sh`, `check-secrets.sh --changed origin/main`,
   `verify-pipeline.sh origin/main`. **Si l'en-tête a été retouché entre-temps, revenir à
   l'étape 2** : la constante doit valoir la taille du dernier commit du vol (AC1).

Plancher : le lint seul, sans le Makefile, ne refuse rien (mika#2578). Le commit ne part pas
sans le câblage.

## Risques

| risque | conséquence | mitigation |
|---|---|---|
| Constante posée avant l'écriture de l'en-tête | le lint rouge sur son propre vol | l'ordre de commit l'interdit explicitement ; AC1 le vérifie en fin de vol |
| En-tête trop long | `Secret Scan` rougit sur ce vol même | 8–10 lignes (~700 o) contre 7 497 o de marge ; `check-secrets.sh --changed origin/main` est dans la liste de vérification |
| Un `cp` du fichier de 1 Mo dans chaque cas du harnais | 4 copies de 1 Mo par run, ~4 Mo et quelques dizaines de ms | acceptable ; une seule racine `mktemp -d`, un seul `trap`. Les cas C2/C3 dérivent de la copie de C1 plutôt que de recopier |
| `sed` d'extraction de la constante trop laxiste | le contrôle P vert sur une ligne de commentaire | ancrage en colonne zéro sur `^AGGREGATE_FROZEN_BYTES=`, et exit 3 si l'extraction rend autre chose qu'un entier > 0 |
| L'agrégat supprimé par #3178, le lint oublié | un lint vert qui ne mesure rien | la branche « absent » le **dit** dans son message vert et nomme son propre retrait |
| Une suite future câblée dans `test` mais pas dans `test-dispatch-lib` | la CI ne la lance pas (`ci.yml` ne nomme pas `test`) | le message d'échec du lint nomme **les deux** cibles |

## Definition of Done

- `scripts/check-dispatch-lib-aggregate-frozen.sh` existe, est exécutable par `bash`, accepte
  un chemin optionnel, et porte `AGGREGATE_FROZEN_BYTES` égale à la taille de l'agrégat au
  dernier commit du vol.
- Ses quatre sorties sont implémentées et distinguables : 0 (≤), 1 (>), 0 (absent, message
  dédié), 2 (illisible).
- Le message d'exit 1 nomme la taille mesurée, la constante, le chemin
  `skills/bundled/_shared/tests/`, le modèle `test_callout_plan_identity_3174.sh`, les deux
  cibles du Makefile, #3178, et le non-geste (ne pas relever la constante, ne pas allowlister).
- `scripts/test-check-dispatch-lib-aggregate-frozen.sh` existe et passe ses 6 contrôles
  (C1–C4 + P + D), sur des copies temporaires uniquement.
- Les 7 faussages du tableau de tests ont été exécutés et rétablis ; la table est dans le corps
  de PR.
- `Makefile` : `test-dispatch-lib` et `test` lancent le lint et son harnais.
- `test-dispatch-lib.sh` porte son bloc d'en-tête, et **aucune autre modification**.
- `make test-dispatch-lib`, `bash scripts/check-suite-wiring.sh`,
  `bash scripts/check-secrets.sh --changed origin/main` et
  `bash scripts/verify-pipeline.sh origin/main` sont verts.
- Aucune entrée ajoutée à `scripts/suite-wiring.tsv`, aucun step ajouté à `ci.yml`, aucun octet
  modifié dans `check-secrets.sh` ni dans `dispatch-lib.sh`.

## Acceptance criteria

- **AC1.** `bash scripts/check-dispatch-lib-aggregate-frozen.sh` est vert sur la branche.
  `AGGREGATE_FROZEN_BYTES` est égale à la taille de l'agrégat dans le dernier commit du vol.
- **AC2. Rouge avant.** Avec un octet ajouté à une copie de l'agrégat, le lint sort en 1, et
  son message nomme la taille, la constante, #3178 et le chemin
  `skills/bundled/_shared/tests/`. La séquence est notée dans le corps de PR.
- **AC3.** Les quatre cas du test du lint passent, et chacun a été vu rouge une fois (contrôle
  faussé puis rétabli).
- **AC4. Câblage.** `make test-dispatch-lib` lance le lint et son test.
  `bash scripts/check-suite-wiring.sh` et `bash scripts/check-secrets.sh --changed origin/main`
  sont verts.
- **AC5.** Aucun autre fichier de suite n'est modifié, et l'agrégat ne change que par la phrase
  d'en-tête.

## Fire-Disposition

Ce vol livre **deux détecteurs** : le lint (`check-dispatch-lib-aggregate-frozen.sh`, chemin de
succès = « le fichier n'a pas grossi ») et son harnais anti-vacuité.

**Option retenue : (a) exception nommée en allowlist — avec une population d'exceptions VIDE,
et cette vacuité est MESURÉE, pas supposée.**

Détail d'implémentation :

- **Zéro violation à l'atterrissage, par construction.** `AGGREGATE_FROZEN_BYTES` est calibrée
  sur la taille du fichier **à la fin de ce vol**. L'unique objet que le lint surveille est donc
  conforme au moment où le lint arrive. Le détecteur atterrit **armé et vert** : ni `#[ignore]`,
  ni désarmement, ni suivi à ouvrir pour l'armer — l'option (b) est sans objet ici.
- **Aucune structure d'allowlist n'est livrée, et c'est délibéré** (« on déclare, on
  n'allowliste pas », mika#2201). La seule sortie légitime du plafond est la scission #3178, pas
  une exemption. Le message d'échec le dit en toutes lettres : il **refuse** le geste que
  `check-secrets.sh` suggère.
- **L'assertion auto-nettoyante que (a) réclame est le contrôle P du harnais.** Une allowlist
  vide est indistinguable d'une allowlist morte si rien ne la mesure. P compare
  `AGGREGATE_FROZEN_BYTES` à la taille réelle de l'agrégat et exige l'égalité : le jour où
  l'une dérive de l'autre, le harnais rougit en exit 3 au lieu de laisser C1 passer vert sur
  une tautologie. C'est ce contrôle qui fait que « population vide » est un fait vérifié à
  chaque run de CI, et non une phrase dans ce plan.
- **Le contrôle D est le garde de la relève.** Il borne la constante par le `LARGE_FILE_LIMIT`
  de `check-secrets.sh` (lu, jamais modifié) : une constante relevée jusqu'au plafond rend le
  gel sans effet, et rougit. C'est la forme auto-nettoyante appliquée au seul mode de
  défaillance que (a) laisse ouvert.
- **L'option (c) halte-et-remontée est sans objet.** Le ticket ne réserve aucune décision à
  l'opérateur : les deux décisions en attente appartiennent à #3178, que ce vol laisse
  explicitement intacte et dormante. Il n'y a ici ni refus de politique de permission à
  contourner, ni coupe de périmètre à arbitrer (mika#3122).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/check-dispatch-lib-aggregate-frozen.sh` | 115 |
| `scripts/test-check-dispatch-lib-aggregate-frozen.sh` | 190 |
| `Makefile` (2 cibles, 2 lignes + commentaire chacune) | 8 |
| `skills/bundled/_shared/test-dispatch-lib.sh` (bloc d'en-tête) | 10 |

Total estimé : 323 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 323 lignes, soit 75 % de la borne par vol et 32 % du seuil par
PR. Les deux tiers du volume sont les deux fichiers neufs, calibrés sur la paire voisine
mesurée (`check-pilot-turn-ceiling-labels.sh` 217 l. / son harnais 222 l. ; `check-root-files.sh`
166 l. / 204 l.) — l'estimation est un peu en deçà parce que le prédicat ici est **une
comparaison d'entiers**, pas un parseur awk à six codes de sortie. Il n'y a aucun code Rust,
aucune migration, aucun appel réseau, et le câblage est de deux lignes dans une cible qui porte
déjà le même motif. Le poste le plus coûteux en **tours** n'est pas l'écriture mais les 7
faussages d'AC3 : un `Edit` aller, un run, un `Edit` retour, soit ~21 tours à prévoir au
budget, confortablement sous les 143 tours de travail que la dérivation ci-dessus suppose.
