# mika#3178 — scinder `test-dispatch-lib.sh` : **vol 2/7**, les familles 1 à 5

- **Issue :** senara-solutions/mika#3178 (`Refs #3174, #3177`)
- **Branche :** `test/3178/dispatch-lib-scinder-test-dispatch-lib`
- **Base :** `890f3f5f` (`main`)
- **Vol courant : le vol 2.** Le vol 1 — l'ossature partagée — est **mergé** (`b60bd55d`, PR #3183,
  `skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh`, 128 lignes). Ce plan est le plan du vol
  suivant. Il succède à `docs/plans/2026-11-26-003-test-3178-…-plan.md`, qui reste en place comme
  document du vol 1 ; le découpage en 25 familles, les plages et la règle de gabarit en sont
  reconduits **après re-mesure**, et quatre points en sont **révisés par mesure** (§ *Ce que les
  mesures du 2026-11-27 ont changé*).
- **Label portant le plafond :** `loop-substrate` (200 tours).

## Condition de départ : les trois conditions sont remplies

Le plan du vol 1 en laissait une non remplie. Les trois sont vérifiées sur la base `890f3f5f` :

1. **#3177 mergée ou fermée.** ✅ Mergée en `26140f2b`.
2. **Aucune PR ouverte ne modifie l'agrégat.** ✅ Les trois PR ouvertes au 2026-11-27 sont #3185
   (`fix(3135)`), #3147 (`chore(deps): bump sqlx`) et #3055 (`fix(3051) — le canari…`) ; aucune ne
   touche `skills/bundled/_shared/test-dispatch-lib.sh` (mesuré sur la liste de fichiers de chaque PR).
3. **`senara-solutions/claude-pilot#325` mergée ET déployée.** ✅ **Les deux.** L'issue est `CLOSED`
   depuis `2026-11-27T15:30:30Z`, et le déploiement — la seule moitié qu'un `gh issue view` ne dit
   pas — est **mesuré dans le bac à sable de ce groom**, sur les deux formes de la permission :

   | geste exercé | résultat |
   |---|---|
   | `sed -n '19070,19080p' <agrégat> >  skills/bundled/_shared/tests/test_dispatch_lib_probe3178.sh` | ✅ exécuté, 11 lignes / 234 octets écrits |
   | `sed -n '116,130p' <agrégat> >> skills/bundled/_shared/tests/test_dispatch_lib_probe3178.sh` | ✅ exécuté |

   La condition qui tenait les vols 2 à 6 est donc levée **par exercice**, pas par lecture d'un
   statut d'issue. C'est la différence que le plan du vol 1 demandait explicitement de faire
   (« “déployée” veut dire : la politique installée que `_run_pilot_sandboxed` injecte porte la
   règle, pas seulement la branche de claude-pilot qui la déclare »).

## La base n'a pas bougé sous le plan : les 25 plages tiennent

| mesure | valeur sur `890f3f5f` | valeur au plan du vol 1 |
|---|---|---|
| `skills/bundled/_shared/test-dispatch-lib.sh` | **19 080 lignes, 1 041 743 octets** | 19 080 / 1 041 743 — **identique** |
| `AGGREGATE_FROZEN_BYTES` | 1 041 743 — **égal** (contrôle P du harnais vert) | égal |
| `LARGE_FILE_LIMIT` | 1 048 576 octets | idem |
| marge sous le plafond | 6 833 octets | idem |
| lint de câblage | `|D|=115 (floor 85) |C|=113 |R|=2 unwired=0` | identique |
| mentions hors `docs/plans` + `docs/brainstorms` | **186 lignes / 82 fichiers** | 183 / 81 |

Les deux commits qui ont atterri depuis (`890f3f5f` docs(3182), `b60bd55d` le vol 1) n'ont **pas
touché l'agrégat** : la table des 25 plages du vol 1 reste valable telle quelle, et les **six
frontières du vol 2 sont re-vérifiées ligne par ligne** sur cette base, pas reconduites de
confiance :

| frontière | ligne | contenu mesuré |
|---|---|---|
| ouverture famille 1 | 116 / 117 | ligne vide, puis `# --- Test 1: Code structure verification ---` |
| 1 → 2 | 1095 / 1096 | ligne vide, puis `# ====` + `# ESCALATE flow (mika#1271 sub-PR 5)` |
| 2 → 3 | 1930 / 1931 | ligne vide, puis `# ====` + `# mika#2811 — quand le refus de la porte…` |
| 3 → 4 | 2815 / 2816 | ligne vide, puis `# ====` + `# Test 12: Push divergence awareness…` |
| 4 → 5 | 3921 / 3922 | ligne vide, puis `# ====` + `# Pre-flight stale-relic cleanup…` |
| 5 → 6 | 4989 / 4990 | ligne vide, puis `# ===` + `# mika#1772 — an honest dev-groom callback` |
| queue | 19070–19080 | `# --- Summary ---` … `exit 0` — **11 lignes**, confirmées |

**Une correction de prose, et elle évite une erreur d'une ligne en vol.** Le § *Découpage retenu* du
vol 1 énonce que « les plages commencent sur la **ligne vide** qui précède la ligne de règle
`# ====` ». La table, elle, fait finir chaque famille **sur** cette ligne vide et commencer la
suivante **sur la ligne de règle** (famille 1 = 116–1095, famille 2 = 1096–1930 : 1095 est la ligne
vide, 1096 la règle). La table est exhaustive et disjointe — c'est elle qui fait foi ; l'énoncé est
décalé d'une ligne. **Ce plan retient la table et corrige l'énoncé :** une plage finit sur la ligne
vide qui précède la bannière suivante, et la bannière voyage bien avec sa famille. Appliquer
l'énoncé littéral aurait décalé les 25 plages d'une ligne.

## Ce que les mesures du 2026-11-27 ont changé

Cinq mesures nouvelles. Deux confirment une décision du vol 1, trois corrigent le plan.

### 1. L'ossature du vol 1 fonctionne — mesuré de bout en bout, pas relu

Deux familles ont été extraites et **exécutées** dans le bac à sable de ce groom, par les trois
gestes exacts de D1(a) :

| famille extraite | plage | lignes | résultat | code de sortie |
|---|---|---|---|---|
| 1 — `iterate_1271` | 116–1095 | 1001 (10 + 980 + 11) | `Results: 121 passed, 0 failed, SKIPPED: 1` | 0 |
| 5 — `preflight_1472` | 3922–4989 | 1089 (10 + 1068 + 11) | `Results: 128 passed, 17 failed, SKIPPED: 1` | 1 |

`bash -n` vert sur les deux. Le `SCRIPT_DIR` ré-ancré sur `_shared/` résout, `lib-mika-bin.sh` est
trouvée, les `assert_*` comptent. **La décision de conception du vol 1 est validée par exécution.**
Les deux sondes ont été supprimées (`rm`, sans option) : ce groom ne livre que ce plan.

### 2. Le `trap … EXIT` de la ligne 4238 n'écrase ni le résumé ni le code de sortie

C'est la mesure qui avait écarté la voie du `trap` au vol 1, et elle se vérifie **dans l'autre
sens** : la famille 5 — celle qui porte `trap "rm -rf '$POLICY_DENY_FIXTURE_DIR'" EXIT` au premier
niveau — imprime bien son bloc `Results:` et sort en **1** avec ses 17 FAIL. La queue en clair, et
non un `trap` de résumé, est donc le bon choix, et il est maintenant mesuré des deux côtés. Rien à
réviser.

### 3. AC3 se prouve par une commande admise, et elle n'écrit rien

Mesure centrale de ce groom. AC3 demande un `diff` « ne montrant que les lignes d'ossature ». Sur la
famille 1 réellement extraite :

```
sed -n '116,1095p' skills/bundled/_shared/test-dispatch-lib.sh \
  | diff - skills/bundled/_shared/tests/test_dispatch_lib_iterate_1271.sh
```

sort **exactement** `0a1,10` (les 10 lignes d'en-tête) et `980a991,1001` (les 11 lignes de queue), et
**rien d'autre**. C'est la formulation littérale d'AC3, obtenue par un pipe en lecture seule : ni
redirection, ni fichier intermédiaire, ni substitution de processus. La commande part telle quelle
dans le corps de PR, une fois par famille.

**Conséquence sur la force de la preuve, et elle simplifie le contrat.** Les 25 plages sont
disjointes et exhaustives (19 069 − 116 + 1 = 18 954, vérifié), et chaque corps est octet pour octet
sa plage. **Aucune assertion ne peut donc être perdue, ajoutée ou dupliquée** : AC2 découle d'AC3
par construction. Le relevé de compteurs par famille (R1 du vol 1) reste au contrat, mais comme
**mesure de confirmation chiffrée** et non comme preuve principale — ce qui retire du vol la seule
étape dont le vol 1 ne savait pas dire comment elle se pratiquerait (le découpage de la sortie de
l'agrégat sur ses lignes de bannière).

### 4. La cause des échecs du bac à sable n'est pas celle que le vol 1 avançait

Le plan du vol 1 attribue les 85 échecs à `fatal: Not a valid object name origin/main` et à un
`mkdir: command not found`. La mesure du jour en nomme une autre, et c'est la bonne pour les
familles du vol 2 : **le binaire `mika` est absent du bac à sable** (`command -v mika` → introuvable).
Les 17 échecs de la famille 5 sont tous de cette classe — `✗ _label_to_type enhancement`,
`✗ _label_to_type feature`, … — et le test imprime lui-même son remède :

```
délèguent au moteur : sans le binaire ce bloc ne peut RIEN mesurer.
Remède : cargo build -p mika-cli --bin mika
```

Deux conséquences :

- **`FAIL = 0` redevient mesurable localement**, ce que le vol 1 déclarait inatteignable (son R2).
  Le vol construit `target/debug/mika` **avant** de mesurer la parité. Ce n'est pas une réécriture
  d'assertion — interdite par le ticket et impossible sous D1(a) — c'est une préparation
  d'environnement, et elle est prescrite par le message du test. `ci.yml` le fait pour la même
  raison et le dit : l'étape `Dispatch-lib assertion suite (mika#1772)` est placée **après**
  `cargo test` « ici, où `target/debug/mika` existe ». Statut : **prescription fondée sur le message
  du test et sur le commentaire CI**, à confirmer au premier geste du vol — pas un fait déjà mesuré
  par ce groom, qui n'a pas construit le binaire.
- **AC5 ne se mesure pas sur le code de sortie d'une suite déjà rouge.** La famille 5 sort **1**
  sans aucun faussage. Un faussage y produirait donc « code non nul » de façon vacante — l'exacte
  forme de vert-sur-rien que ce dépôt refuse. **AC5 se mesure sur le compteur `FAIL`**, qui doit
  s'incrémenter de exactement 1 au faussage et revenir à sa valeur au rétablissement ; le code de
  sortie non nul n'est l'instrument que sur une suite verte. Le corps de PR cite les deux nombres
  par suite.

### 5. Les étiquettes de bloc ne sont pas uniques — le re-ciblage ne peut pas suivre les familles

Le vol 1 prévoit ~20 lignes de re-ciblage de mentions **par vol**, « au vol de leur famille ». La
mesure dit que ce découpage n'est pas praticable, et pourquoi. Les mentions désignent un bloc par
**étiquette** (`S4`, `S11`, `S14`, `S-c`, `T5`, `T6`, `T12j`, `V4`, `case B`,
`rescue-block extraction`) — et ces étiquettes sont **réutilisées d'une famille à l'autre** :

| étiquette | occurrences dans l'agrégat | familles concernées |
|---|---|---|
| `T12j` | l. 3847 **et** l. 9897–9980 | 4 **et** 13 |
| `T5` | l. 5483, 7612, 9639, 10174, 10387, 13494, 15900, 17198 | ~8 familles |
| `T6` | l. 5490, 7690, 9660, 10214, 10399, 16911 | ~6 familles |
| `V4` | l. 5905, 8433, 12439, 14275, 15003, 16702, 18920 | ~7 familles |
| `S4` | l. 5877, 7912, 10922, 13262 | 4 familles |

Une mention comme `dispatch-lib.sh:5351` — « `test-dispatch-lib.sh` (T12j) qui les tient ensemble »
— ne désigne donc **aucune famille de façon décidable** tant que les 25 suites n'existent pas : il
faut lire l'assertion pour savoir si c'est le bloc 3847 (famille 4) ou le bloc 9897 (famille 13).
Répartir ce travail « au vol de sa famille » demande, à chaque vol, d'**écarter 20 familles pour
chaque mention** — le jugement le plus coûteux par unité, répété cinq fois, et le seul de la série
dont une erreur est silencieuse (une mention re-ciblée vers la mauvaise suite reste une phrase
plausible).

**Révision : le re-ciblage des mentions devient un vol à lui seul, le vol 7.** Au vol où les 25
suites existent, chaque étiquette a exactement une destination, et l'identification cesse d'être un
jugement. Le découpage passe donc de 6 à 7 vols (§ *Découpage en vols*). Les vols 2 à 6 ne touchent
**aucune** mention, et AC6 n'est à aucun moment transitoirement fausse : l'agrégat existe pendant
tout le re-ciblage du vol 7 et disparaît dans le **même commit**.

## Note du vol (2026-11-27) : une sixième livraison, la bibliothèque de fixtures

**Mesuré en vol, pas prévu au groom.** Le § *Les trois gestes qui produisent une suite* suppose
qu'une plage contiguë est auto-suffisante. Deux dépendances inter-familles l'ont démenti, chacune
découverte par une exécution et non par relecture :

1. **L'état de premier niveau que l'agrégat ne pose qu'une fois.** L'agrégat `source`
   `dispatch-lib.sh` à sa l. 730 — dans la **famille 1** — et y définit trois marqueurs
   (l. 737-739) que des blocs des familles 2 et 8 lisent. Extraite seule, la famille 2 meurt au
   premier `_escalate_groom` : commande introuvable, `stderr` déjà vers `/dev/null` par le bloc
   lui-même, donc une panne **silencieuse** sous `set -e`. Remède : ces quatre éléments vivent
   désormais dans `tests/lib-dispatch-lib-suite.sh` (26 lignes ajoutées à l'ossature du vol 1),
   et la l. 730 reste en place dans la famille 1, où un second `source` est sans effet.
2. **La chaîne définition → fixture → appel, répartie sur trois familles.** L'agrégat l'annonce
   lui-même à ses l. 1426-1428 : les douze cas `test_groom_gate_*` sont **définis** en
   famille 2 (l. 1429-1827), `_fixture_setup` en **famille 4** (l. 2824-2855), et la boucle qui
   les **appelle** en **famille 5** (l. 4950-4988). Aucun découpage en plages contiguës ne la
   garde entière : la famille 5 extraite seule rendait **12 `rc=127`** là où l'agrégat en rend 0.

**Livrable ajouté : `skills/bundled/_shared/tests/lib-dispatch-lib-groom-gate-fixtures.sh`** —
42 lignes d'en-tête **écrites**, puis les deux plages `2824-2855` et `1426-1827` (434 lignes)
**extraites par le `sed` de D1(a)**, dans cet ordre (le fixture d'abord, car les douze cas
l'appellent). La famille 5 la `source` depuis son en-tête ; les familles 2 et 4 gardent les mêmes
plages dans leur corps, où une redéfinition de fonction est sans effet — c'est ce qui laisse leur
`diff` d'AC3 exact.

**La permission de D1(a) couvre ce nom, et c'est mesuré.** `claude-pilot#328` — mergée et
déployée — admet une seule forme de plus vers ce fichier :
`sed -n '<N>,<M>p' <agrégat> >> skills/bundled/_shared/tests/lib-dispatch-lib-groom-gate-fixtures.sh`,
en **ajout seulement**. Jamais `>`, jamais une autre forme, jamais un autre nom, et jamais un
`Edit` qui recopierait les plages à la main : le mot pour mot reste prouvé **par construction**,
comme pour les cinq suites.

**Conséquence sur les deux nombres de D2(b) :** lignes écrites **84 → 132** (+42 d'en-tête de
bibliothèque, +6 de `source` et de commentaire dans la famille 5), lignes déplacées
**4 929 → 5 363** (+434). Les deux restent sous la borne de 429 lignes écrites par vol.

**Pour les vols 3 à 6 :** les deux dépendances ci-dessus sont les seules **mesurées** à ce vol.
La leçon générale n'est pas une liste mais une méthode — **une famille extraite se lance avant
d'être crue**, et un `rc=127` ou une panne silencieuse nomme la dépendance. Elle est consignée
sous `docs/solutions/`.

## Découpage en vols : 7, et le vol courant est le 2

La règle de comptage est **D2(b)**, tranchée par l'opérateur le 2026-11-27 (corps de mika#3178) : la
borne de 429 lignes mord sur les **lignes écrites**, pas sur les lignes déplacées mot pour mot par
le `sed` de D1(a). Chaque vol déclare donc deux nombres distincts.

| vol | contenu | familles | **lignes écrites** | lignes déplacées | état |
|---|---|---|---|---|---|
| 1 | `tests/lib-dispatch-lib-suite.sh` — l'ossature | — | 128 (réel) | 0 | ✅ mergé (#3183) |
| **2** ⬅ *vol courant* | **5 suites + câblage** | **1–5** | **84** | **4 929** | à livrer |
| 3 | 5 suites + câblage | 6–10 | ≈ 84 | 3 838 | à venir |
| 4 | 5 suites + câblage | 11–15 | ≈ 84 | 2 685 | à venir |
| 5 | 5 suites + câblage | 16–20 | ≈ 84 | 3 787 | à venir |
| 6 | 5 suites + câblage | 21–25 | ≈ 84 | 3 990 | à venir |
| 7 | **clôture** : re-ciblage de toutes les mentions, `rm` de l'agrégat, retrait du trio de gel, `bundled_skills.rs`, `Closes #3178` | — | ≈ 130 | 0 | à venir |

Les lignes déplacées de chaque vol incluent les 5 copies de la queue (5 × 11 = 55).

**Ce que le vol 2 ne fait pas, et pourquoi c'est volontaire :** il ne retire rien de l'agrégat, ne
touche pas `AGGREGATE_FROZEN_BYTES`, ne touche pas `dispatch-lib.sh`, et ne re-cible aucune mention.
Son `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` est donc **vide** — plus fort
que la clause « seulement des commentaires » du vol 1, et vérifiable d'une commande.

**`Tracked in:`** — la suite est tracée par **#3178 lui-même**, qui reste ouvert jusqu'au vol 7 et
dont la table ci-dessus est l'ordre de marche. Chaque vol porte `Refs #3178` ; le vol 7 est le seul
à écrire `Closes #3178`. Le périmètre du **vol courant** est les cinq familles de la ligne « 2 ».

**Le vol 7 porte son propre groom.** Son estimation ci-dessus (≈ 130 lignes écrites) est un ordre de
grandeur : ~82 fichiers portent une mention, dont 33 sous `docs/solutions/**` que la règle de tri du
vol 1 exclut (un relevé d'incident daté décrit ce qui était vrai à sa date), et `dispatch-lib.sh`
en concentre 31 à lui seul. Le découpage définitif du vol 7 — et son éventuelle scission — se
mesure à son groom, quand les 25 destinations existent. Ce plan ne juge que le vol 2 (mika#2693 §5).

## Les cinq suites du vol 2

Gabarit : aucune suite ne dépasse **1 500 lignes**, le gabarit de la plus grosse suite existante du
répertoire (`test_egress_push_bound.sh`, 1 479 lignes). AC1 n'exige que 50 % de `LARGE_FILE_LIMIT`
(≈ 9 600 lignes) ; la règle retenue est 7 fois plus serrée, parce que le motif du ticket est la
**lisibilité en revue**.

| # | suite (`skills/bundled/_shared/tests/…`) | plage | corps | total fichier | ce qu'elle tient |
|---|---|---|---|---|---|
| 1 | `test_dispatch_lib_iterate_1271.sh` | 116–1095 | 980 | 1001 | structure de la branche issue-fermée, boucle `_iterate_groom_loop`, second-pass ×2 par branche |
| 2 | `test_dispatch_lib_escalate_1271.sh` | 1096–1930 | 835 | 856 | flux ESCALATE (mika#1271 sub-PR 5), note `already_groomed` |
| 3 | `test_dispatch_lib_groom_gate_2811.sh` | 1931–2815 | 885 | 906 | porte de dispatch mika#2811, absence des freins de fabrication mika#1319 |
| 4 | `test_dispatch_lib_push_divergence_1364.sh` | 2816–3921 | 1106 | 1127 | divergence de push + remontée d'échec de rebase (mika#1364), scan `_is_scaffold_path` (mika#2144) |
| 5 | `test_dispatch_lib_preflight_1472.sh` | 3922–4989 | 1068 | 1089 | nettoyage de reliques pré-vol + diagnostic double-échec (mika#1472) ; **porte le `trap … EXIT` de la l. 4238** |

Somme des corps : 980 + 835 + 885 + 1106 + 1068 = **4 874** lignes. Plus 5 queues de 11 lignes :
**4 929 lignes déplacées**. Plus grosse suite produite : 1 127 lignes, **≈ 7 % de `LARGE_FILE_LIMIT`**.

## Les trois gestes qui produisent une suite

Sous D1(a), et ils sont les seuls — mesurés au § *Condition de départ* :

1. `Write` l'en-tête (~14 lignes, gabarit ci-dessous). **Seule** écriture du fichier, et seule chose
   que le modèle rédige.
2. `sed -n '<début>,<fin>p' skills/bundled/_shared/test-dispatch-lib.sh >> <suite>` — le corps, mot
   pour mot par construction.
3. `sed -n '19070,19080p' skills/bundled/_shared/test-dispatch-lib.sh >> <suite>` — la queue de
   résumé, mot pour mot elle aussi, **zéro ligne écrite**.

Aucune ligne du corps ni de la queue ne traverse le contexte du modèle.

**Gabarit d'en-tête** (c'est l'exclusion du `diff` d'AC3) :

```bash
#!/bin/bash
# <titre de la famille> — extrait de test-dispatch-lib.sh par mika#3178, vol 2.
# <deux à quatre lignes disant ce que la famille tient>
#
# Plage d'origine : lignes <N>–<M> de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 890f3f5f. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
```

**Reprise d'une suite ratée :** `rm <suite>` (`rm` sans option), puis les trois gestes depuis le
début. `Write` refuse d'écraser un fichier non lu, et l'en-tête doit précéder le corps.

## Un refus de permission, nommé et sans conséquence sur ce vol

Mesuré pendant ce groom : `sed -n '116,1095p' <agrégat> > .pilot-scratch/ref-famille1.txt` est
**refusé** — `no matching policy rule -- denied by default (production posture)`. La permission
D1(a) est bornée au **seul** répertoire `skills/bundled/_shared/tests/`, ce qui est exactement ce
que l'opérateur a décidé le 2026-11-27.

Ce refus **n'empêche rien de ce plan**, et aucun repli n'est inscrit pour le contourner (mika#3122
règle 1) : la preuve d'AC3 passe par `sed … | diff - <suite>`, un pipe en lecture seule qui n'écrit
aucun fichier et qui a été **exercé avec succès** (§ *Ce que les mesures ont changé*, point 3). Ce
plan ne demande donc **aucun élargissement** de la permission.

## Câblage

Le ticket l'exige directement dans les deux cibles, et dit pourquoi :
`check-suite-wiring.sh` ne voit pas un appel fait depuis le corps d'une autre suite, et
sous-approximer la joignabilité donne des faux rouges, jamais des faux verts (mika#2761).

- **10 lignes** : 5 suites × 2 cibles. Dans `test` (après la ligne 150,
  `@bash …/tests/test_rescue_signal_open_pr.sh`) et dans `test-dispatch-lib` (après la ligne 213,
  `@bash …/tests/test_callout_plan_identity_3174.sh`), plus ~4 lignes de commentaire nommant
  mika#3178 et le vol.
- **L'agrégat garde ses deux appels** (`Makefile:144` et `Makefile:207`) : il n'est retiré qu'au
  vol 7.
- **La cible `test-dispatch-lib` garde son nom.** `.github/workflows/ci.yml:85` lit
  `run: make test-dispatch-lib` et c'est la **cible** qu'il nomme, pas le fichier.
- La joignabilité passe par le hop **H2** (le workflow nomme la cible, le lint suit la recette, ses
  prérequis et la récursion `$(MAKE)`). **Aucune entrée dans `scripts/suite-wiring.tsv`** : les 5
  suites sont atteintes, et le lint est bidirectionnel — une entrée pour une suite atteinte le
  ferait rougir.
- **Population.** `|D|` passe de 115 à 120, `|C|` de 113 à 118, `|R|` reste à 2, `unwired=0`. Le
  plancher `# min-population: 85` est franchi par le haut.

## Le gel de mika#3179 : rien à toucher à ce vol

L'agrégat ne grossit pas (gel, prédicat `≤`) et ne maigrit pas (le vol ne retire aucune plage).
**`AGGREGATE_FROZEN_BYTES` n'est donc pas touchée**, et le contrôle **P** du harnais — qui exige
l'**égalité** entre la constante et la taille réelle, et sortirait en vacuité (exit 3) sur un écart
— reste vert par construction. Le trio de gel (le lint, son harnais, leurs 4 lignes `@bash` et
leurs 8 lignes de commentaire dans `Makefile`) est retiré **au vol 7**, dans le commit qui supprime
l'agrégat : `scripts/test-check-dispatch-lib-aggregate-frozen.sh:82-88` teste `[ ! -r … ]` avant
tout et compterait une vacuité sur un agrégat absent.

## Décision réservée à l'opérateur — elle ne mord pas sur ce vol

**D3 — comment se lit l'exclusion « tout changement de `dispatch-lib.sh` » du périmètre, face à
AC6 ?** Le périmètre du ticket exclut tout changement de `dispatch-lib.sh` ; AC6 exige qu'aucune
mention ne désigne un bloc déplacé. Or les **31** mentions de ce fichier désignent toutes un bloc
qui bouge. Les deux clauses se croisent. Les options, chiffrées :

| option | geste | coût |
|---|---|---|
| **D3(a)** — l'exclusion protège le **comportement**, pas les commentaires | 31 `Edit` de lignes `#` dans `dispatch-lib.sh` au vol 7 ; `git diff` du fichier ne montre que des lignes de commentaire, et le corps de PR le cite | ~1 tour par mention au vol 7 ; AC6 pleinement vraie |
| **D3(b)** — lecture littérale : `dispatch-lib.sh` est intouchable | aucune de ses 31 mentions n'est re-ciblée ; AC6 est explicitement borné aux 81 autres fichiers | 0 ligne ; AC6 reste partiellement fausse, par décision assumée et écrite |

**Ce plan n'en pratique aucune**, et il n'a pas à le faire : **le vol 2 ne touche aucune mention**
(§ *Ce que les mesures ont changé*, point 5), donc son `git diff origin/main -- dispatch-lib.sh` est
**vide** dans les deux options. D3 mord au **vol 7** et se tranche à son groom. Elle est nommée ici
pour qu'elle ne se découvre pas au dernier vol, pas pour être arbitrée maintenant.

## Ordre de commit

Préalable, une fois : `cargo build -p mika-cli --bin mika` — le remède que le test imprime lui-même,
sans quoi `FAIL = 0` n'est pas mesurable localement (§ *Ce que les mesures ont changé*, point 4).

Puis, pour chacune des cinq familles, dans l'ordre de la table :

1. `Write` l'en-tête (~14 lignes, gabarit ci-dessus).
2. `sed -n '<début>,<fin>p' <agrégat> >> <suite>` — le corps.
3. `sed -n '19070,19080p' <agrégat> >> <suite>` — la queue.
4. `bash -n <suite>` — **avant** tout lancement (R4 : une frontière tombée dans un here-document
   laisse un délimiteur non fermé, donc une erreur de syntaxe ; les compteurs pourraient ne pas la
   voir).
5. `sed -n '<début>,<fin>p' <agrégat> | diff - <suite>` — AC3 : ne doit montrer que les deux blocs
   d'ossature (`0a1,<n>` et `<corps>a…`).
6. Exécution de la suite ; relevé des trois compteurs (confirmation d'AC2).
7. AC5 : une assertion faussée → `FAIL` s'incrémente de exactement 1 (et, sur une suite verte, le
   code de sortie passe non nul) ; puis rétablissement et retour de `FAIL` à sa valeur.

puis, une fois les cinq faites :

8. `Makefile` — 10 lignes de câblage + ~4 de commentaire. **L'agrégat garde ses deux appels.**
9. `bash scripts/check-suite-wiring.sh`, `bash scripts/check-secrets.sh --changed origin/main`,
   `bash scripts/canonical-tokens-survey.sh --check`, `make verify-bundled-skills`,
   `make test-dispatch-lib`, `bash scripts/verify-pipeline.sh origin/main`.
10. `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` — doit être **vide**.
11. Corps de PR : les deux nombres de D2(b) (84 écrites / 4 929 déplacées), les 5 `diff` d'AC3, les
    5 relevés de compteurs, la séquence AC5 avec ses deux nombres par suite, et la mention explicite
    qu'**AC1, AC3 (clause « blocs retirés ») et AC6 ne seront vrais qu'au vol 7** — plutôt que de
    laisser un AC non coché passer pour un oubli.

## Fire-Disposition

Option **(a) exception nommée en allowlist**, avec un **ensemble d'exceptions vide**.

Le vol livre 5 détecteurs — les 5 suites — mais **aucun détecteur nouveau** : chaque assertion est
déplacée mot pour mot par le `sed` de D1(a), et son chemin de succès est inchangé. Aucune violation
existante n'a besoin d'une exception, et aucune n'en reçoit :

- **pas d'entrée dans `scripts/suite-wiring.tsv`** — les 5 suites sont câblées dans le `Makefile` et
  atteintes depuis `ci.yml:85` par le hop H2. Le lint étant bidirectionnel, une entrée pour une
  suite atteinte le ferait rougir : l'exception serait sa propre régression ;
- **pas d'entrée dans `LARGE_FILE_ALLOWLIST`** — hors périmètre, et sans objet : la plus grosse
  suite produite pèse ≈ 7 % du plafond ;
- **pas de relève de `AGGREGATE_FROZEN_BYTES`** — l'agrégat ne maigrit pas à ce vol, et le contrôle
  P reste vert par construction.

**Les deux assertions auto-nettoyantes** qui rougissent quand l'état change sous elles :

1. `scripts/check-suite-wiring.sh` (règle D\C==R, lint `suite-wiring-lint`) rougit dès qu'une des 5
   suites perd son appelant — c'est le détecteur qui tient le câblage d'AC4 dans le temps.
2. `scripts/test-check-dispatch-lib-aggregate-frozen.sh` (contrôle P) rougit si
   `AGGREGATE_FROZEN_BYTES` cesse d'égaler la taille réelle — donc si un vol fait maigrir l'agrégat
   sans le déclarer. Il est l'assertion auto-nettoyante de la voie « l'agrégat reste entier ».

**Un détecteur qui deviendra vacant, et il est traité au vol 7, pas ici.**
`crates/mika-agent/src/bundled_skills.rs::test_support_dirs_exclude_test_files` affirme
`!skills_dir.join("_shared").join("test-dispatch-lib.sh").exists()`. Le jour où l'agrégat disparaît,
elle passe au vert **par absence de sujet**. Elle est re-ciblée au vol 7 sur un chemin survivant,
dans le commit qui supprime l'agrégat — jamais après. Ce vol ne la touche pas : l'agrégat existe
encore, et l'assertion mord encore.

**Les échecs d'environnement ne reçoivent pas d'exception.** Les 17 échecs mesurés dans la famille 5
viennent de l'absence du binaire `mika` dans le bac à sable, non d'une assertion fautive. Ils sont
**nommés, chiffrés et attribués** dans le corps de PR, et le remède (`cargo build -p mika-cli --bin
mika`) est au premier rang de l'ordre de commit. Aucune entrée d'allowlist dans le dépôt.

**AC5 est l'armement**, et son instrument est le compteur `FAIL`, pas le code de sortie : 5
faussages délibérés, chacun devant faire passer `FAIL` de `n` à `n+1`, puis revenir à `n`. Sur une
suite déjà rouge, un code de sortie non nul serait vrai sans rien prouver.

## Taille estimée

Comptage par **D2(b)** (tranchée par l'opérateur le 2026-11-27, corps de mika#3178) : la borne mord
sur les **lignes écrites**, non sur les lignes déplacées mot pour mot par le `sed` de D1(a), qui ne
traversent ni le contexte du modèle ni sa rédaction.

| livrable | lignes de code (hors `docs/`) |
|---|---|
| 5 en-têtes de suite sous `skills/bundled/_shared/tests/` (14 lignes chacun) | 70 |
| `Makefile` — câblage des 5 suites dans `test` et `test-dispatch-lib` + commentaire | 14 |
| lignes déplacées par `sed` (hors borne, D2(b)) | 0 |

Total estimé : 84 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et ce n'est pas qu'une affaire de lignes.** 84 lignes écrites, loin sous la
borne de 429. Le coût réel est en **tours**, et il est borné par construction : 5 familles × 7
gestes = 35 tours de production, plus ~8 tours de vérification finale et la fin de pipeline. Les
gestes sont identiques d'une famille à l'autre et ne demandent aucune lecture de corps — c'est ce
que D1(a) achète. Les deux familles déjà extraites pendant ce groom (1 et 5) ont pris **4 tours
chacune**, mesurés, ce qui ancre l'estimation sur une mesure et non sur un pari. Le re-ciblage des
mentions — le seul travail de la série dont le coût en tours est mal borné — est sorti de ce vol et
porté au vol 7 (§ *Ce que les mesures ont changé*, point 5).

**Les sept vols, pour mémoire :** lignes écrites 128 (réel), **84**, ≈84, ≈84, ≈84, ≈84, ≈130 —
chacune sous la borne de 429 ; lignes déplacées 0, 4 929, 3 838, 2 685, 3 787, 3 990, 0.

Tracked in: senara-solutions/mika#3178 — le ticket reste ouvert jusqu'au vol 7 et porte les vols 3
à 7 ; la table du § *Découpage en vols* en est l'ordre de marche, et le vol 7 est le seul à écrire
`Closes #3178`.

## Definition of Done

Du vol courant (vol 2) :

- [ ] Les 5 suites de la table existent sous `skills/bundled/_shared/tests/`, chacune composée
      uniquement de son en-tête, de sa plage et de la queue — les deux dernières extraites par
      `sed` (D1(a)).
- [ ] `bash -n` est vert sur chaque suite **avant** son premier lancement.
- [ ] Le `diff` d'AC3 de chaque suite ne montre **que** les deux blocs d'ossature, et les 5
      commandes figurent dans le corps de PR.
- [ ] Les trois compteurs de chaque suite sont relevés, et `FAIL = 0` partout une fois
      `target/debug/mika` construit.
- [ ] 5 faussages d'AC5, chacun mesuré sur le compteur `FAIL` (`n` → `n+1` → `n`), notés dans le
      corps de PR.
- [ ] Les 5 suites sont câblées directement dans `test` et `test-dispatch-lib` ; aucune entrée dans
      `scripts/suite-wiring.tsv`.
- [ ] `bash scripts/check-suite-wiring.sh` est vert et rapporte `|D|=120 |C|=118 |R|=2 unwired=0`.
- [ ] `bash scripts/check-secrets.sh --changed origin/main`,
      `bash scripts/canonical-tokens-survey.sh --check`, `make verify-bundled-skills`,
      `make test-dispatch-lib` et `bash scripts/verify-pipeline.sh origin/main` sont verts.
- [ ] `AGGREGATE_FROZEN_BYTES` est **inchangée**, et l'agrégat garde ses deux appels de `Makefile`.
- [ ] `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` est **vide**.
- [ ] Le corps de PR déclare qu'AC1, AC3 (clause « blocs retirés ») et AC6 ne seront vrais qu'au
      vol 7.

De la série (vols 3 à 7, un vol à la fois) :

- [ ] Les 25 suites existent, chacune sous 1 500 lignes.
- [ ] Au vol 7 : toutes les mentions re-ciblées, l'agrégat supprimé, le trio de gel de mika#3179
      retiré en le citant, `test_support_dirs_exclude_test_files` re-ciblée sur un chemin survivant
      et non vacante, la cible `test-dispatch-lib` gardant son nom, et **D3 tranchée** par
      l'opérateur à son groom.

## Acceptance criteria

- **AC1.** `skills/bundled/_shared/test-dispatch-lib.sh` n'existe plus, ou n'est plus qu'un
  lanceur de quelques lignes. Aucune suite sous `skills/bundled/_shared/` ne dépasse 50 % de
  `LARGE_FILE_LIMIT`.
- **AC2. Non-perte, prouvée sur le même commit.** La somme des PASS des nouvelles suites égale le
  PASS de l'ancienne, et de même pour SKIPPED, avec FAIL = 0 des deux côtés. Le corps de PR cite
  les deux comptes.
- **AC3. Déplacement prouvé.** Pour chaque famille, la concaténation des blocs retirés, comparée
  par `diff` au corps de sa nouvelle suite (ossature exclue), ne montre que les lignes d'ossature
  retirées. La commande figure dans le corps de PR.
- **AC4. Câblage.** `make test-dispatch-lib` lance toutes les nouvelles suites.
  `bash scripts/check-suite-wiring.sh`, le survey d'inventaire canonique en mode `--check` et
  `bash scripts/check-secrets.sh --changed origin/main` sont verts.
- **AC5. Rouge avant.** Une assertion de chaque nouvelle suite est volontairement faussée et la
  suite rend un code non nul, puis est rétablie. La séquence est notée dans le corps de PR.
- **AC6. Références.** Plus aucune mention de `test-dispatch-lib.sh` ne désigne un bloc qui n'y
  est plus (`git grep` cité dans le corps de PR).

**Comment les six AC se distribuent.** AC2, AC3 (clause « déplacement »), AC4 et AC5 sont vérifiés
**à chaque vol**, sur les familles de ce vol — et pour le vol 2, AC3 est vérifié par la commande
mesurée au § *Ce que les mesures ont changé*, point 3. AC1, AC3 (clause « blocs retirés ») et AC6 ne
sont pleinement vrais qu'au **vol 7**, parce que l'agrégat reste entier jusque-là. AC5 est mesuré sur
le compteur `FAIL` et non sur le code de sortie, pour la raison donnée au point 4 du même paragraphe.

## Risques

- **Une frontière tombée dans un here-document.** Seule classe de défaut qu'un découpage par
  numéros de ligne produit silencieusement. Couverte par `bash -n` avant tout lancement, et doublée
  par le `diff` d'AC3. Les deux familles déjà extraites (1 et 5, dont la plus longue des cinq) ont
  passé `bash -n` du premier coup.
- **La double exécution allonge la CI.** L'agrégat et les suites exécutent les mêmes blocs jusqu'au
  vol 7 : ≈ +30 s par PR pour les 5 familles de ce vol (l'agrégat entier prend 1 min 12 s pour 25
  familles), sur les deux cibles `test` et `test-dispatch-lib`. Chiffré, temporaire, et c'est le
  prix de la voie qui n'exige aucune permission supplémentaire.
- **Les compteurs globaux comptent deux fois pendant la série.** `Results:` de l'agrégat reste
  inchangé et chaque suite rend sa part en plus. Aucun compteur global n'est donc un total de la
  série — ce que le relevé par famille rend explicite.
- **Un en-tête mal rédigé.** Les 14 lignes de chaque en-tête sont les seules que le modèle écrive
  par suite. Une erreur y est rattrapée par `bash -n`, par le `diff` d'AC3 (qui montrerait un
  en-tête plus long que prévu) et par l'exécution. Le gabarit a été exercé deux fois pendant ce
  groom.
