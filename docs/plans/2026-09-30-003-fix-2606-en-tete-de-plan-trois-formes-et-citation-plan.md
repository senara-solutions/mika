# Plan : `_plan_header_claimed_issues` lit l'en-tête que nos plans écrivent réellement — trois formes typographiques, la citation, et les deux motifs couplés (mika#2606)

**Ticket :** mika issue#2606 — `fix(dispatch-lib): _plan_header_claimed_issues ne lit pas « Ticket : » (espace avant :) ni un en-tête en citation « > » — garde de réfutation #2038 muette sur 99/161 plans récents`
**Labels :** `bug`, `p2-normal`, `dispatch:loop`, `loop-substrate`
**Type :** fix (substrat de la boucle — faux négatif silencieux d'une garde, classe mika#2205)
**Fichiers principaux :** `skills/bundled/_shared/dispatch-lib.sh`, `skills/bundled/_shared/tests/test_find_issue_plan.sh`, `skills/bundled/_shared/test-dispatch-lib.sh`, `scripts/measure-plan-header-coverage.sh` (nouveau), `Makefile`

---

## Problème

`_plan_header_claimed_issues` (`dispatch-lib.sh`) est le lecteur unique de
l'en-tête d'un plan : il rend tout numéro d'issue que les 20 premières lignes
**réclament**. `_plan_header_refutes_issue` s'en sert pour refuser un plan qui
appartient à un **autre** ticket, et rend « ne réfute pas » dès que la
réclamation est vide — un fail-open délibéré de mika#2038, parce que 95 des
745 plans d'alors ne portaient aucun marqueur et qu'exiger une correspondance
positive les rendait tous indécouvrables.

Son motif exige les deux-points **collés** au label et n'admet aucun préfixe de
citation. Nos plans, écrits en français, mettent une espace avant les
deux-points et posent souvent leur en-tête dans un bloc de citation. Le lecteur
est donc muet sur la majorité des plans récents, et la garde qu'il alimente ne
peut rien réfuter : elle n'échoue pas, elle ne voit rien.

C'est la forme exacte de la classe mika#2205 — *une garde que personne n'a
exercée se lit exactement comme une garde qui marche*. Aucune panne n'a été
observée, et c'est précisément ce qui rend le défaut coûteux à trouver.

## Ce que la mesure établit, et les trois rectifications qu'elle impose au ticket

Toutes les mesures ci-dessous ont été prises sur ce worktree, à `e6c40a80`, en
sourçant la vraie fonction plutôt qu'une copie.

### M1 — Le défaut est réel, et plus large que mesuré

Sur les **163** plans `docs/plans/2026-09-1*` à `2026-09-3*` (le ticket en
comptait 161 ; deux plans ont atterri depuis) :

| lecteur | plans dont l'en-tête de ticket est lu |
|---|---|
| actuel | **38** |
| motif complet de ce plan | **147** |

Donc **109** plans portent une ligne d'en-tête réelle que le lecteur actuel ne
lit pas — et non 99. L'écart de dix est expliqué par M2, et il n'est pas une
imprécision du ticket : sa sonde a utilisé **son propre motif proposé** pour
décider « ce plan porte un en-tête », donc les dix formes que ce motif rate lui
étaient invisibles aussi.

### M2 — Il existe une TROISIÈME forme typographique, que le ticket n'identifie pas

Recensement des formes réelles sur la population (label normalisé en `LABEL`,
nombres en `N`) :

| forme | n | actuel | motif **littéral du ticket** | motif complet |
|---|---|---|---|---|
| `LABEL: …`, `**LABEL:** …`, `- **LABEL:** …` | 39 | ✓ | ✓ | ✓ |
| `**LABEL :** …` (et variantes backtick/lien) | 50 | ✗ | ✓ | ✓ |
| `- **LABEL :** …` | 29 | ✗ | ✓ | ✓ |
| `> LABEL : …` | 14 | ✗ | ✓ | ✓ |
| `> **LABEL :** …`, `> - **LABEL :** …`, `> **LABEL:** …` | 14 | ✗ | ✓ | ✓ |
| **`- **LABEL** : …`, `> **LABEL** : …`** | **9** | ✗ | **✗** | ✓ |
| prose enroulée (`ticket.`, `ticket est …`) | 5 | ✗ | ✗ | ✗ *(correct)* |

La forme en gras se décline de deux façons et le ticket n'en voit qu'une. Son
`(\s*:\*\*|\s*:)` lit `**Ticket :**` — l'espace **dans** le gras — mais pas
`**Ticket** :`, où **le gras se ferme avant les deux-points**. Neuf en-têtes
réels sont de cette seconde façon, tous des réclamations légitimes :

```
- **Ticket** : senara-solutions/mika#2331
- **Ticket** : senara-solutions/mika#2446
- **Ticket** : senara-solutions/mika#2133
- **Ticket** : senara-solutions/mika#2155
- **Ticket** : senara-solutions/mika#2149
- **Ticket** : senara-solutions/mika#2152
- **Ticket** : `mika issue#2575`
> **Ticket** : senara-solutions/mika#2523 — « … »
> **Ticket** : senara-solutions/mika#2522 — « … »
```

**Conséquence pour l'AC3 du ticket, et c'est pourquoi cette rectification n'est
pas cosmétique.** Implémenté à la lettre, le motif proposé fait tomber le compte
des en-têtes non lus de ~125 à **25**, et l'AC3 (« 0, ou des cas nommés »)
serait rapportée satisfaite — alors que 9 de ces 25 ne sont pas des exceptions
nommées mais **le même défaut, un cran plus loin**. Le motif complet le fait
tomber à **16**, dont aucun n'est un en-tête (M6).

### M3 — La fonction a DEUX motifs, ils sont couplés, et le ticket n'en nomme qu'un

`_plan_header_claimed_issues` lit par deux branches :

1. un `grep -oE '#[0-9]+'` sur les lignes admises — la forme `#N` ;
2. un `sed -E` qui **dépouille le préfixe et le label**, pour qu'un
   `grep -oE '^[0-9]+'` lise la valeur **numérique nue** (`issue: 1679`,
   `number: 3030`), la forme que mika#1617 a enseignée.

Le `sed` porte sa propre copie du préfixe. Élargir le `grep` seul laisse donc le
`sed` aveugle sur les lignes que le `grep` vient d'admettre. Mesuré, `grep`
élargi et `sed` d'origine :

| ligne | attendu | obtenu |
|---|---|---|
| `issue : 1679` | `1679` | **vide** |
| `> issue: 1679` | `1679` | **vide** |
| `> issue : 1679` | `1679` | **vide** |
| `**Ticket :** mika issue#2295` | `2295` | `2295` ✓ |

La dernière ligne est le piège : la branche `#N` continue de fonctionner. **Un
demi-correctif est donc vert sur chaque fixture en `#N` et cassé en silence
uniquement sur les formes YAML numériques-nues** — c'est-à-dire la classe de
panne silencieuse que ce ticket existe pour fermer, reproduite un cran plus
tard. C'est ce qui décide la forme d'implémentation (R1) : **un seul site de
définition**, pas deux motifs tenus synchrones par la vigilance.

### M4 — Aucun faux positif sur le corpus entier

Sur les **983** plans de `docs/plans/`, en comparant chaque plan à sa cible :
exactement **une** réfutation d'un plan contre son propre créneau de nom de
fichier, et elle est **préexistante** (le lecteur actuel la produit aussi)
**et correcte**. `2026-05-20-001-fix-847-followup-…` porte en frontmatter
`ticket: mika#852` : c'est un suivi de #847 qui **appartient** à #852, et le
`847` de son slug est une citation. Le discriminant de ma sonde (le créneau du
nom) est un proxy imparfait, pas le lecteur — et ce cas est mot pour mot
l'assertion déjà présente dans `test-dispatch-lib.sh` (« a plan whose slug cites
another ticket is refuted by its header »).

**Le motif complet n'introduit donc zéro faux positif sur 983 plans.**

### M5 — La mesure décisive : la sélection du tier 1 ne change pas

Le proxy de M4 ne suffit pas, parce que le glob du tier 1 est
`*-${ISSUE_NUM}-*-plan.md` : il attrape un numéro **n'importe où** dans le nom,
pas seulement dans le créneau canonique. La mesure qui tranche est de **rejouer
la sélection complète du tier 1** — réfutation, partition `in_slot`/`off_slot`,
`head -1` — pour chaque numéro d'issue, avant et après.

Sur les **482** créneaux d'issue canoniques, c'est-à-dire la population
réellement dispatchée :

```
  numéros d'issue rejoués ........... 482
  sélections qui CHANGENT ........... 0
  sélections PERDUES (régression) ... 0
```

**Aucun plan ne devient indécouvrable.** C'est le contrôle qui borne le risque
réel de ce correctif — la classe de faux négatif que mika#1421 (n=2),
mika#1602 (n=3) et mika#1617 (N=5) ont chacun été ouverts pour fermer.

Rejoué sur un ensemble plus large de 538 nombres délimités par tirets (qui
ramasse aussi les numéros de **séquence** et de **date**, jamais des
`ISSUE_NUM`), sept sélections changent. Cinq sont des artefacts de la sonde
(`001`, `02`, `03`, `004`, `005`). Les deux autres portent des numéros crédibles
et sont **la garde qui mord correctement** :

| `ISSUE_NUM` | plan que l'actuel rend | ce que son en-tête réclame |
|---|---|---|
| `1837` | `2026-09-28-1837-fix-2572-…` | `**Ticket :** senara-solutions/mika#2572` |
| `2137` | `2026-09-26-2137-fix-2548-…` | `**Ticket :** senara-solutions/mika#2548` |

Les deux noms portent un créneau de **séquence** à quatre chiffres que le glob
du tier 1 attrape, et les deux numéros existent dans le dépôt. Aujourd'hui le
lecteur ne voit pas leur en-tête français, donc ne réfute pas, donc rendrait le
plan de #2572 comme plan de #1837 — **exactement le défaut que mika#2038 existe
pour fermer**. Ces deux cas sont le bénéfice, pas une régression.

### M6 — Ce qui reste non lu, et pourquoi c'est correct

16 des 163, en deux familles, et les deux doivent rester silencieuses :

- **14** ne portent **aucune** ligne à label `ticket|issue|number` en tête de
  ligne. C'est la population documentée de mika#2038, et le fail-open est ce qui
  les garde découvrables.
- **2** sont de la **prose française enroulée**, dont la ligne commence par le
  mot sans deux-points : `ticket.` et
  `ticket est reconstruit à partir de trois sources présentes dans le dépôt :`.
  L'exigence de deux-points collés au label les exclut, et c'est le
  rétrécissement n°1 de mika#2038 qui fait son travail — le même qui lit
  `The issue: 3 phases remain` comme de la prose et non comme une réclamation
  de l'issue 3.

Ce sont les « cas nommés » que l'AC3 autorise, et ils sont nommés ci-dessus.

### M7 — Un titre transcrit importe ses propres `#N`, et c'est neutre

Mesuré sur **ce plan même**, qui sert de démonstration : son en-tête est
`**Ticket :** mika issue#2606 — « … garde de réfutation #2038 muette … »`, et le
lecteur complet réclame donc **`2038 2606`**. La transcription du titre du
ticket importe chaque `#N` que ce titre contient — c'est le troisième
rétrécissement de mika#2038 (« tous les `#N` d'une ligne comptent »), appliqué à
une ligne que l'élargissement rend nouvellement lisible. Environ 50 plans
emploient cette forme.

**L'effet est neutre, et la raison vaut d'être écrite parce qu'elle est ce qui
rend tout cet élargissement sûr :** élargir le motif ne peut qu'**ajouter** des
lignes, donc qu'agrandir l'ensemble réclamé ; et la réfutation exige qu'**aucun**
numéro réclamé ne soit la cible. Un ensemble plus grand rend donc la réfutation
**moins** probable, jamais plus. Le numéro propre du plan y est toujours, donc un
plan légitime ne peut pas se faire réfuter par l'élargissement — ce que M5
confirme empiriquement sur 482 créneaux, et ce que cet argument explique.

Le cas dual est vide lui aussi : avant le fix, l'en-tête non lu donnait un
ensemble réclamé **vide**, donc pas de réfutation ; après, pour un
`ISSUE_NUM` que le titre cite, ce numéro est dans l'ensemble, donc pas de
réfutation non plus. **Comportement identique**, et non pas une garde
affaiblie — ce qui est déjà la décision écrite de mika#2038 (« un plan pour un
ticket peut légitimement en citer d'autres dans son titre »).

---

## Requirements

### R1 — Le lecteur admet les trois formes et la citation, depuis UN site de définition

Dans `_plan_header_claimed_issues`, le préfixe de ligne d'en-tête et le
séparateur label↔valeur deviennent **deux variables locales, définies une fois,
interpolées dans les deux motifs** :

```bash
local hdr='^[[:space:]]*(>[[:space:]]*)*(-[[:space:]]+)?(\*\*)?'
local sep='(\*\*)?[[:space:]]*(:\*\*|:)'

label_lines=$(head -n 20 "$candidate" 2>/dev/null \
    | grep -iE "${hdr}(ticket|issue|number)${sep}")
…
    | sed -E "s/${hdr}[A-Za-z]+${sep}[[:space:]]*//"
```

Trois ajouts au motif actuel, et rien d'autre :

| ajout | ferme |
|---|---|
| `(>[[:space:]]*)*` | l'en-tête en bloc de citation |
| `(\*\*)?` **après** le label | la troisième forme, `**Ticket** :` (M2) |
| `[[:space:]]*` avant les deux-points | la typographie française, `Ticket :` |

`[[:space:]]` et non `\s` : la fonction est déjà écrite en classes POSIX, et
l'homogénéité vaut mieux qu'une extension GNU dans un motif que deux outils se
partagent.

**Le site unique est le livrable, pas une élégance.** M3 montre qu'un préfixe
recopié se désynchronise en rendant la moitié `#N` verte ; une variable
consommée par les deux motifs rend la désynchronisation **inexprimable** plutôt
que détectée après coup. C'est la doctrine du lecteur unique que la maison a dû
graver pour `grooming_marker` (mika#2158) et pour `parse_log_llm_bodies`
(mika#2220).

### R2 — Les trois rétrécissements de mika#2038 sont intacts, et on le prouve

Aucun des trois ne bouge, et chacun a son assertion :

1. **Le label ancre la ligne.** `Related issue: #456` et
   `The issue: 3 phases remain` restent non réclamants — l'ancre `^` et les seuls
   préfixes admis (citation, tiret, gras) ne laissent passer aucun mot devant le
   label.
2. **`id` n'est pas un label réfutant.** L'alternance reste
   `(ticket|issue|number)`. `groom_session_id: 557a7808` ne réclame rien.
3. **Tous les `#N` d'une ligne comptent.** `**Ticket:** mika#1772/#1773` rend
   toujours les deux, donc ne réfute pas 1772.

Un négatif **nouveau** est ajouté, parce que l'élargissement le rend pensable :
`**Ticket** de fond : mika#111` ne réclame rien. `[[:space:]]*` ne peut pas
enjamber des lettres, donc la propriété tient par construction — mais c'est
exactement le genre de propriété qu'on croit tenir et qu'on n'a pas vérifiée.

### R3 — Les fixtures sont des en-têtes RÉELS, et elles sont gelées

Dans `skills/bundled/_shared/tests/test_find_issue_plan.sh`, un bloc mika#2606
assertant les lignes d'en-tête **mesurées** des plans cités :

| fixture (ligne réelle) | attendu |
|---|---|
| `**Ticket :** mika issue#2295 — …` | `2295` |
| `> Ticket : senara-solutions/mika#2315 (p1 substrat)` | `2315` |
| `> - **Ticket :** senara-solutions/mika#2310` | `2310` |
| `- **Ticket** : senara-solutions/mika#2331` | `2331` |
| `> **Ticket** : senara-solutions/mika#2523 — « … »` | `2523` |
| `- **Ticket** : ` + backticks + `mika issue#2575` | `2575` |
| `**Ticket :** mika issue#2606 — « … réfutation #2038 … »` (M7) | `2038 2606` |

La dernière est la forme dominante — le titre du ticket transcrit — et elle est
assertée **avec ses deux numéros**, pas seulement le bon : c'est ce qui épingle
le troisième rétrécissement de mika#2038 sur une ligne que l'élargissement rend
nouvellement lisible, et ce qui ferait rougir une lecture gloutonne qui ne
retiendrait que le dernier `#N` (le défaut que ce rétrécissement a été écrit pour
fermer, `**Ticket:** mika#1772/#1773` n'ayant alors rendu que 1773).

Ce sont des **lignes**, pas des copies des fichiers de plan : copier six plans
entiers ajouterait ~100 Ko de bruit et dériverait du jour où quelqu'un édite un
plan. Et elles sont **gelées, jamais rafraîchies** — la doctrine que mika#2158 a
dû écrire pour ses propres fixtures : les régénérer depuis le dépôt effacerait
les formes mêmes que le prédicat doit reconnaître. Un commentaire au-dessus du
bloc dit de quel plan chaque ligne vient et que le rafraîchissement est interdit.

Les cinq positifs préexistants (`**Issue:** #539`, `issue: 1679`,
`number: 3030`, `issue: senara-solutions/mika#1772`,
`**Ticket:** mika#1772/#1773`) sont assertés dans le même bloc : c'est la
non-régression, et elle doit être visible à côté de ce qui change.

### R4 — Le couplage est couvert par comportement ET par forme

Deux couches, parce qu'aucune des deux ne suffit.

**Comportement** — quatre fixtures numériques-nues dans les formes
**nouvellement admises**, c'est-à-dire précisément celles que M3 montre cassées
par un demi-correctif : `issue : 1679`, `> issue: 1679`, `> issue : 1679`,
`- **issue** : 1679`. Sans elles, la suite reste verte sur un demi-correctif.

**Forme** — une assertion de source dans `test-dispatch-lib.sh` sur le corps de
`_plan_header_claimed_issues` (`declare -f`, le motif déjà employé pour
`PLAN_GATE_SRC_2034` juste à côté) :

- le corps définit les deux variables de motif ;
- le corps **n'inline aucun littéral de préfixe** en dehors de leur définition —
  c'est-à-dire qu'un futur éditeur qui recopie le préfixe dans l'un des deux
  motifs fait rougir le build au lieu de rouvrir M3 ;
- **terme d'anti-vacuité** : l'assertion échoue si le corps est vide ou
  introuvable. Sans lui, un renommage de la fonction rend le scan
  silencieusement inerte — et *un scan qui ne regarde plus rien se lit
  exactement comme un arbre propre* (classe mika#2205, qui est littéralement la
  classe de ce ticket).

### R5 — Le commentaire dit les trois formes et le couplage

Le bloc de commentaire de la fonction est le lieu où les rétrécissements de
mika#2038 vivent, et il est la raison pour laquelle ce défaut a survécu : il
décrit avec soin ce que le motif refuse, et pas une fois ce qu'il **n'atteint
pas**. Il gagne les trois formes, le couplage des deux motifs, et le renvoi à
la mesure.

**Contrainte de forme :** `dispatch-lib.sh` est dans la population S1 du
`canonical-tokens-lint` (gate CI bloquant). Vérifié : `Ticket` n'est ni un
jeton tout-capitales ni `Disposition`/`Verdict`, donc hors de la population de
la règle L4 ; et ce n'est pas une clé de callout canonique, donc hors de L3.
Les formes citées le sont **entre backticks** par sûreté — la doctrine mika#2201
(« un jeton cité comme du code est une mention, jamais une instruction »), née
du cas mesuré où un document expliquant le lint le faisait rougir.

### R6 — Le contrôle de population est une MESURE re-jouable, pas une garde CI

`scripts/measure-plan-header-coverage.sh` **source `dispatch-lib.sh` et appelle
la vraie fonction** — aucune seconde définition du lecteur, sans quoi la sonde
divergerait du code au premier changement (la leçon que mika#2293 a dû épingler
pour sa reconstruction de cascade). Il rend, sur un glob passé en argument :
le compte lu / non lu, et **chaque** plan non lu avec ses premières lignes.
Sortie de rapport uniquement, **toujours exit 0** : ce n'est pas un détecteur,
c'est un instrument.

Câblé à `make measure-plan-header-coverage`, et **délibérément pas à la CI.**

**Pourquoi pas une garde CI**, alors que la tentation est forte et que l'AC3
ressemble à une demande de test permanent :

- elle asserterait une propriété d'un **corpus de documents** en croissance
  (983 fichiers, `2026-10-*` le mois prochain), pas une propriété du **code** ;
- elle rougirait sur les 14 plans légitimement sans marqueur (M6), donc
  demanderait une allowlist de 14 noms de fichiers historiques qui devient
  obsolète immédiatement ;
- une nouvelle forme d'en-tête dans un plan est une **variance rédactionnelle**,
  pas une régression de code : rougir la CI pour elle pousserait les auteurs à
  reformuler leur prose plutôt qu'à réparer le lecteur — l'incitation
  exactement inverse ;
- les fixtures de R3/R4 protègent déjà contre la seule régression qui compte,
  celle du code.

L'AC3 est donc satisfaite **comme mesure** : les comptes de M1/M5/M6 sont
rapportés dans le corps de la PR, et la commande qui les reproduit est livrée.

---

## Fire-Disposition

Ce plan livre deux détecteurs :

1. les assertions de `test_find_issue_plan.sh` (R3, R4 comportemental, R2) ;
2. l'assertion de forme de `test-dispatch-lib.sh` (R4 forme).

`scripts/measure-plan-header-coverage.sh` (R6) **n'en est pas un** : il rapporte
et sort toujours en 0, sans mode d'échec — c'est pourquoi il est conçu ainsi
plutôt qu'avec un `--check`.

**Disposition retenue : (a) exception nommée en allowlist — allowlist livrée
VIDE.**

La population de violations existantes est **mesurée nulle** : les 21
assertions de la conception passent contre le motif complet (négatifs de
mika#2038, cinq positifs préexistants, six formes nouvelles, quatre formes
numériques-nues), et M4/M5 établissent zéro faux positif et zéro sélection
perdue sur 983 plans et 482 créneaux d'issue. Il n'y a donc rien à excepter, et
c'est le motif dominant de la maison : livrer armé avec une allowlist vide, et
**quand le scan tire, on répare le site — on n'ajoute pas de ligne** (doctrine
mika#2201).

(b) *livrer désarmé* est écarté : le détecteur **est** le correctif de ce
ticket — un `#[ignore]` laisserait la garde muette, c'est-à-dire ne fermerait
rien. (c) *halte-et-remontée* est écarté : il n'y a rien à cadrer, la mesure est
complète et sans ambiguïté.

**L'assertion auto-nettoyante** est le terme d'anti-vacuité de R4 : le scan de
forme échoue si le corps de la fonction est introuvable ou vide, donc un
renommage ou une suppression le fait rougir au lieu de le rendre inerte. C'est
la protection dont l'absence **est** le sujet de ce ticket.

---

## Implementation plan

1. **R1 + R5** — réécrire `_plan_header_claimed_issues` avec `hdr`/`sep` et
   mettre son commentaire à jour (formes entre backticks).
2. **R2 + R3 + R4 comportemental** — ajouter le bloc mika#2606 à
   `tests/test_find_issue_plan.sh` : négatifs, positifs préexistants, six formes
   réelles gelées, quatre formes numériques-nues.
3. **R4 forme** — ajouter l'assertion de source + anti-vacuité à
   `test-dispatch-lib.sh`, à côté de `PLAN_GATE_SRC_2034`.
4. **R6** — écrire `scripts/measure-plan-header-coverage.sh` (source la vraie
   fonction, rapport seul) et sa cible `make`.
5. Rejouer la mesure et reporter M1/M5/M6 dans le corps de la PR.

## Verification Contract

| # | vérification | commande | attendu |
|---|---|---|---|
| V1 | non-régression de la découverte | `bash skills/bundled/_shared/tests/test_find_issue_plan.sh` | les 44 assertions préexistantes passent, **plus** le bloc mika#2606 |
| V2 | non-régression de dispatch-lib | `make test-dispatch-lib` | vert, assertion de forme R4 incluse |
| V3 | le gate de jetons ne rougit pas | `bash scripts/check-canonical-tokens.sh .` | vert (baseline établie verte avant le changement) |
| V4 | **contrôle négatif du demi-correctif** | élargir le `grep` seul, laisser le `sed` | les quatre fixtures numériques-nues de R4 **rougissent** |
| V5 | **contrôle négatif de la troisième forme** | retirer le `(\*\*)?` de R1 | les fixtures `**Ticket** :` **rougissent** |
| V6 | **contrôle négatif de l'anti-vacuité** | renommer la fonction dans une copie | l'assertion de forme **rougit** |
| V7 | contrôle de population | `make measure-plan-header-coverage` | 147/163 lus ; les 16 non lus sont ceux de M6 |
| V8 | sélection du tier 1 inchangée | rejouer M5 sur les 482 créneaux | 0 changement, 0 perte |

V4, V5 et V6 sont les vérifications porteuses : **une suite qui ne rougit sur
aucune des trois n'atteste rien**. V4 en particulier est la seule qui distingue
« les deux motifs bougent ensemble » de « la moitié `#N` marche ».

## Acceptance criteria

1. **AC1 — le motif lit les formes réelles.** `_plan_header_claimed_issues`
   admet la typographie française (espace avant les deux-points, dans le gras
   **et** hors du gras) et un préfixe de bloc de citation, sans perdre aucun des
   trois rétrécissements de mika#2038 : le label ancre la ligne, `id` n'est pas
   un label réfutant, tous les `#N` d'une ligne comptent.
2. **AC2 — les en-têtes réels sont assertés, les négatifs conservés.** Les
   en-têtes mesurés des plans `2295`, `2315`, `2310`, `2293` sont lus avec leur
   bon numéro, ainsi que ceux de la troisième forme (`2331`, `2523`, `2575`).
   `Related issue: #456` et `The issue: 3 phases remain` restent non réclamants,
   et `groom_session_id: 557a…` ne réclame rien.
3. **AC3 — contrôle de population.** Rejoué sur `docs/plans/2026-09-*`, le
   nombre d'en-têtes non lus tombe de ~125 à **16 cas nommés** : 14 plans sans
   aucune ligne à label (population fail-open de mika#2038) et 2 lignes de prose
   française enroulée sans deux-points. La commande qui reproduit la mesure est
   livrée.
4. **AC4 — aucun plan ne devient indécouvrable.** La sélection du tier 1 de
   `_find_issue_plan` est inchangée sur les 482 créneaux d'issue canoniques :
   zéro sélection modifiée, zéro perdue.
5. **AC5 — le couplage des deux motifs est couvert.** Un correctif qui élargit
   le `grep` sans le `sed` fait rougir la suite (V4).

## Definition of Done

- [ ] R1 : `hdr`/`sep` définis une fois, consommés par les deux motifs.
- [ ] R2 : les trois rétrécissements de mika#2038 assertés, plus le négatif
      nouveau `**Ticket** de fond :`.
- [ ] R3 : six en-têtes réels gelés, avec leur provenance et l'interdiction de
      rafraîchir en commentaire.
- [ ] R4 : quatre fixtures numériques-nues + assertion de forme + anti-vacuité.
- [ ] R5 : commentaire à jour, formes entre backticks.
- [ ] R6 : `scripts/measure-plan-header-coverage.sh` + cible `make`, hors CI.
- [ ] V1–V3 verts ; V4–V6 **vus rouges** puis rétablis.
- [ ] M1/M5/M6 reportés dans le corps de la PR.

---

## Ce que ce travail n'achète PAS

- **Il ne rend pas la garde SAINE, seulement non muette.** Un plan sans en-tête
  ne réclame rien et reste accepté — c'est le fail-open délibéré de mika#2038, et
  **14 des 163 plans de la population sont dans cet état**. Ce qui change est que
  109 plans qui *déclarent* leur ticket sont désormais lus.
- **Il ne rattrape rien.** Aucun plan n'est réécrit, aucun callout de corps n'est
  réparé. Les deux réfutations que M5 nomme (`1837`, `2137`) ne se produiront que
  si ces numéros sont un jour dispatchés comme issues ; rien ici ne les corrige
  rétroactivement.
- **Aucune télémétrie nouvelle.** Les diagnostics existants
  (`dispatch_gate_groom_plan_refuted`, `_find_issue_plan: tier 1 discarded …`)
  sont inchangés. Et leur surface est inégale, ce qu'il faut dire plutôt que
  laisser découvrir : `FIND_ISSUE_PLAN_REFUTED` n'atteint `tasks.result` que par
  `PLAN_REFUTED_NOTE`, consommé **uniquement** sur les trois branches
  `PIPELINE FAILURE` — donc une réfutation qui **perd** le plan est lisible, une
  réfutation qui **change** la sélection ne l'est que sur un stderr que
  l'exécuteur laisse tomber quand le dispatch réussit (classe mika#2050, Signaux
  M et Q). M5 mesure cette seconde population **vide** aujourd'hui : c'est la
  raison de ne pas ajouter de surface, pas un oubli.
- **Il ne surveille rien.** Le seul instrument neuf est la mesure de R6, et
  **son silence ne prouve rien tant que personne ne l'exécute.**

## Hors périmètre, délibérément

- **Une garde CI sur le corpus de plans** — refusée avec ses quatre motifs en
  R6. **Suivi conditionnel**, dont la précondition est une mesure montrant
  qu'une *quatrième* forme d'en-tête est apparue sans que les fixtures ne la
  voient.
- **Le fail-open de mika#2038 lui-même** (un en-tête absent ne réfute pas) —
  décision datée et mesurée de ce ticket-là, dont l'inversion rendrait 14 plans
  de la seule population de septembre indécouvrables.
- **Normaliser la forme d'en-tête que les plans écrivent.** La tentation
  inverse : au lieu d'élargir le lecteur, imposer `**Ticket:**` aux
  prescripteurs. Écartée — le lecteur doit lire **l'historique**, que rien ne
  réécrit, et `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
  dit ce qu'un correctif qui ne vit que dans un prompt vaut au substrat de la
  boucle. Les deux moitiés ne sont pas exclusives ; seule la structurelle est
  livrée ici.
- **Le motif du tier 2** (`^(\*\*Ticket:\*\*|\*\*Issue:\*\*|ticket:|issue:)\s+mika…`)
  et celui du **tier 3**. Ils cherchent une raison d'**accepter**, où une erreur
  est rattrapable, et le tier 2 n'accepte qu'un en-tête ancré nommant *cette*
  issue — un candidat qu'il accepte ne peut jamais être réfuté. Ils portent les
  mêmes formes typographiques et donc la même cécité, mais leur direction de
  risque est inverse : **population et rayon de souffle distincts, ticket
  distinct.** **Suivi nommé**, précondition : une mesure montrant qu'un plan
  légitime a échappé aux trois tiers faute d'en-tête lisible au tier 2.
- **Le glob du tier 1** (`*-${ISSUE_NUM}-*-plan.md`), qui attrape un numéro de
  séquence ou de date aussi bien qu'un numéro d'issue — c'est ce qui rend les
  deux cas de M5 possibles. Propriété préexistante, non aggravée par ce
  correctif, et la resserrer changerait quels candidats entrent au tier 1.
- **Le stderr non persisté du chemin `_find_issue_plan`** (classe mika#2050) —
  inhérent au placement de `_iterate_groom_loop` hors de la redirection de
  `_run_claude_pilot`, nommé ci-dessus, non refermé ici.
