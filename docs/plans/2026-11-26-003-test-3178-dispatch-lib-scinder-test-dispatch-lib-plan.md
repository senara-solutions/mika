# mika#3178 — scinder `test-dispatch-lib.sh` en suites par familles

- **Issue :** senara-solutions/mika#3178 (`Refs #3174, #3177`)
- **Branche :** `test/3178/dispatch-lib-scinder-test-dispatch-lib`
- **Base :** `7b7ce6eb` (branche) sur `2727e477` (`main`)
- **Révision du 2026-11-27.** Ce plan est la révision du plan du 2026-11-26 (commit `4db65ae7`),
  après que l'opérateur a tranché les deux décisions que la première passe laissait entières. Le
  découpage en 25 familles, les mesures et les critères sont conservés ; sont révisés l'instrument
  du déplacement (D1), l'unité de la borne par vol (D2), les plages (re-mesurées sur la nouvelle
  base), l'ossature (une mesure du jour a fermé une voie) et le traitement du gel de mika#3179.
- **Label portant le plafond :** `loop-substrate` (200 tours).

## Condition de départ

Trois conditions, toutes vérifiables sans jugement :

1. **#3177 mergée ou fermée.** ✅ Remplie — mergée en `26140f2b`.
2. **Aucune PR ouverte ne modifie l'agrégat.** ✅ Remplie au 2026-11-27 : les deux PR ouvertes sont
   #3147 (`chore(deps): bump sqlx`) et #3055 (`fix(3051) — le canari…`), et ni l'une ni l'autre ne
   touche `skills/bundled/_shared/test-dispatch-lib.sh`.
3. **`senara-solutions/claude-pilot#325` mergée ET déployée.** ⛔ Non remplie au 2026-11-27 :
   l'issue est `OPEN`. C'est la permission nommée de D1(a) (§ *D1(a)* ci-dessous) ; **tout** vol de
   ce plan l'utilise, du vol 2 au vol 6. Un vol parti sans elle verrait son premier `sed` refusé et
   n'aurait aucun repli : ce plan n'en prévoit aucun (mika#3122 règle 1). Le vol 1 (ossature) est
   le seul qui n'en dépend pas — il n'écrit qu'avec `Write`.

Le merge de claude-pilot#325 se fait sur mot de l'opérateur, hors de ce dépôt. « Déployée » veut
dire : la politique installée que `_run_pilot_sandboxed` injecte porte la règle, pas seulement la
branche de claude-pilot qui la déclare.

---

## Le fait mesuré, re-mesuré sur la base du jour

| mesure | valeur |
|---|---|
| `skills/bundled/_shared/test-dispatch-lib.sh` | **1 041 743 octets, 19 080 lignes** |
| `AGGREGATE_FROZEN_BYTES` (`scripts/check-dispatch-lib-aggregate-frozen.sh`) | 1 041 743 octets — **égal**, le contrôle P du harnais l'exige |
| `LARGE_FILE_LIMIT` (`scripts/check-secrets.sh`) | 1 048 576 octets |
| marge restante sous le plafond | 6 833 octets (~125 lignes à 54,6 octets/ligne) |
| exécution dans le bac à sable de dispatch | `Results: 2036 passed, 85 failed, SKIPPED: 5` |
| **durée de cette exécution** | **1 min 11,8 s** (13,9 s user, 11,5 s system) |
| plus grosse suite existante sous `tests/` | `test_egress_push_bound.sh`, 1 479 lignes |
| population du lint de câblage | `|D|=115 (floor 85) |C|=113 |R|=2 unwired=0` |

**Ce qui a bougé depuis le 2026-11-26 :** `f3bd907e` (mika#3179) a ajouté **10 lignes** de
commentaire d'avertissement **en tête** de l'agrégat (lignes 2–11), et rien d'autre. Toutes les
plages du plan du 26/11 se décalent donc de **+10**, uniformément. Le décalage a été vérifié
mécaniquement, pas supposé : les 24 frontières de famille qui tombaient sur une ligne de règle
`# ====` y tombent encore après +10 (la 25ᵉ, la famille 1, s'ouvre sur `# --- Test 1:` et non sur
une bannière, comme dans le plan d'origine). Les trois compteurs sont inchangés : les 10 lignes
sont du commentaire.

Les 85 échecs sont **induits par l'environnement** et préexistent au déplacement : ils sortent sur
`fatal: Not a valid object name origin/main` dans les dépôts-fixtures du bac à sable, et sur un
`mkdir: command not found` à la ligne 18195 (le `PATH` vidé du sous-shell de la famille 24).
L'étape `Dispatch-lib assertion suite (mika#1772)` de `.github/workflows/ci.yml:85` passe sur
`main`. Conséquence directe sur AC2, traitée au § *Preuve de non-perte*.

## Ce que la mesure a fermé

Le corps du ticket nomme quatre points « à confirmer, pas à supposer ». Les quatre sont confirmés
par lecture du code, pas par raisonnement :

1. **Le survey d'inventaire canonique exclut les nouvelles suites.** `collect_files()` de
   `scripts/canonical-tokens-survey.sh` écarte `*/tests/*`, `test-*` et `test_*` sous
   `skills/bundled`. Une suite nommée `skills/bundled/_shared/tests/test_dispatch_lib_*.sh` tombe
   dans **deux** de ces trois exclusions.
2. **Le seeding build-time ne les installe pas.** `collect_support_dir_files()` de
   `crates/mika-agent/build_support/bundled_skills_discover.rs` saute toute entrée dont le nom
   contient `test` en minuscules — ce qui écarte le répertoire `tests/` entier, donc son contenu.
3. **Les deux fuites d'état inter-familles suspectées n'en sont pas.** Le `PATH="$base_dir/empty-path"`
   de la ligne 18194 vit dans un sous-shell `( … )` ouvert en 18189 et refermé en 18199 ; les deux
   redéfinitions de `mika()` (927 et 948) sont annulées par `unset -f mika mika_stdin_capture` en
   952. Aucune des deux ne traverse une frontière de famille. La preuve mécanique reste exigée au
   § *Preuve de non-perte* (R1) : 796 affectations de premier niveau ne s'énumèrent pas, elles se
   mesurent.
4. **L'appelant de `check-pilot-turn-ceiling-labels.sh` est déjà double.** Le corps de la suite
   l'appelle en 8723–8727, mais `Makefile:186` l'appelle **directement**. La dernière contrainte de
   périmètre du ticket est donc déjà satisfaite structurellement : rien à créer, seul le bloc
   déplace son appel avec lui.

Deux conséquences de code s'ajoutent, mesurées et non évoquées par le ticket :

- `crates/mika-agent/src/bundled_skills.rs:3058-3059` porte
  `test_support_dirs_exclude_test_files`, qui affirme
  `!skills_dir.join("_shared").join("test-dispatch-lib.sh").exists()`. Si l'agrégat disparaît,
  **cette assertion devient vacante et passe au vert par absence de sujet.** Elle doit être
  re-ciblée sur un chemin survivant.
- `scripts/check-suite-wiring.sh:49` et `scripts/test-check-suite-wiring.sh:267` nomment en prose
  l'agrégat comme « la seule suite atteinte depuis le corps d'une autre suite ». Après le
  déplacement, la phrase désigne un fichier qui n'existe plus.

**Trois mesures nouvelles du 2026-11-27, et la première ferme une voie de conception :**

5. **Il y a un `trap … EXIT` au premier niveau de l'agrégat**, ligne 4238 :
   `trap "rm -rf '$POLICY_DENY_FIXTURE_DIR'" EXIT`, dans la famille 5. Conséquence : une ossature
   qui imprimerait son résumé depuis un `trap _suite_summary_and_exit EXIT` armé par la
   bibliothèque **serait écrasée** par cette ligne dans la suite de la famille 5 — le résumé ne
   sortirait pas et le code de sortie serait 0 **même avec des FAIL**. Une panne silencieuse, dans
   la seule suite où elle ne se verrait pas. La voie du `trap` est donc écartée par mesure, et la
   queue de résumé reste du code en fin de suite (§ *L'ossature partagée*).
6. **Les quatre `exit` en colonne zéro de l'agrégat (2620, 2637, 6483, 6613) sont tous à
   l'intérieur de here-documents** — des fixtures de scripts que les tests écrivent (`<< 'HOOK'`,
   `<< 'STUB'`, `<<'FAKE_PROXY'`). Il n'existe donc **aucune sortie précoce** de premier niveau
   avant la queue : aucune suite extraite ne risque de court-circuiter son propre résumé.
7. **Un here-document peut contenir une ligne qui ressemble à une frontière.** C'est la seule
   classe de défaut qu'un découpage par numéros de ligne peut produire silencieusement. Elle est
   couverte par R4 (§ *Preuve de non-perte*) : `bash -n` sur chaque suite extraite, avant de la
   lancer. Une frontière tombée dans un here-document laisse un délimiteur non fermé, donc une
   erreur de syntaxe — détectée en une commande, et non par un test qui passerait à côté.

## Le gel de mika#3179, et ce que chaque vol lui doit

`f3bd907e` a mergé un plafond statique sur l'agrégat. Trois pièces, et elles contraignent ce plan :

| pièce | ce qu'elle fait | où elle est câblée |
|---|---|---|
| `scripts/check-dispatch-lib-aggregate-frozen.sh` | refuse toute **croissance** (prédicat `≤`). Agrégat absent → vert explicite qui **nomme son propre retrait** ; agrégat illisible → exit 2 | `Makefile` cibles `test` (l. 194) et `test-dispatch-lib` (l. 219) |
| `scripts/test-check-dispatch-lib-aggregate-frozen.sh` | harnais anti-vacuité C1–C4 + contrôle **P** (`AGGREGATE_FROZEN_BYTES` **égale** la taille réelle) + contrôle **D** (la constante reste sous `LARGE_FILE_LIMIT`) | `Makefile` cibles `test` (l. 195) et `test-dispatch-lib` (l. 220) |
| l'avertissement de 10 lignes en tête de l'agrégat | dit où va le prochain bloc | dans l'agrégat, part avec lui |

**Les deux règles que ce plan en tire :**

- **Un vol qui fait maigrir l'agrégat abaisse `AGGREGATE_FROZEN_BYTES` dans le MÊME commit**, par
  `Edit`, à la nouvelle taille exacte, et **jamais vers le haut**. Le lint lui-même resterait vert
  (son prédicat est `≤`), mais le contrôle **P** du harnais exige l'égalité et sortirait en
  **vacuité (exit 3)** : `make test-dispatch-lib` rougirait sur une constante dérivée, pas sur une
  croissance.
- **Le vol qui supprime l'agrégat supprime le trio**, dans le même commit : `rm` du lint, `rm` du
  harnais, et retrait de leurs **4 lignes `@bash`** (l. 194, 195, 219, 220) plus les **8 lignes de
  commentaire** qui les introduisent (l. 190–193 dans `test`, l. 215–218 dans `test-dispatch-lib`).
  Raison mécanique, et c'est le harnais et non le
  lint qui l'impose : `scripts/test-check-dispatch-lib-aggregate-frozen.sh:82-88` teste
  `[ ! -r "$AGGREGATE_REAL" ]` **avant tout**, compte une vacuité et sort en **3**. Un agrégat
  supprimé sans son harnais fait donc rougir `make test-dispatch-lib` et `make test`. Le message
  vert du lint prescrit exactement ce geste (« retirez `scripts/check-dispatch-lib-aggregate-frozen.sh`,
  son harnais, et leurs lignes des cibles `test` et `test-dispatch-lib` du Makefile »).

**Ce que la voie retenue par D1(a) rend vrai, et c'est un bénéfice non prévu :** l'agrégat ne
maigrit **jamais** pendant la série (§ *Comment les blocs sortent de l'agrégat*). Donc
`AGGREGATE_FROZEN_BYTES` n'est **pas touchée une seule fois** des vols 1 à 5, le contrôle P reste
vert par construction, et le seul geste de gel de toute la série est le retrait du trio au vol 6.

**Un deuxième bénéfice, et il porte sur la permission elle-même :** l'agrégat étant gelé contre la
croissance **et** intact pendant la série, les 25 plages de la table ci-dessous restent valables du
vol 1 au vol 6. Les 25 commandes `sed` sont écrites une fois, dans ce plan, et aucune ne se
re-mesure. Un découpage qui retirerait les plages vol par vol décalerait les familles restantes à
chaque vol et exigerait une re-mesure des 20, puis 15, puis 10, puis 5 plages survivantes — avec
une classe de défaut (plage décalée d'une ligne) que ni `bash -n` ni les compteurs ne voient
nécessairement.

## Décisions tranchées le 2026-11-27

Les deux décisions que la première passe laissait entières ont été tranchées. **Source :** le corps
de senara-solutions/mika#3178, bloc « **Réponses (2026-11-27, bearing Prime sur délégation de
l'opérateur)** ». Elles ne se rediscutent pas ici ; cette section les applique.

### D1(a) — une permission nommée, bornée au seul répertoire `tests/`

**Décision retenue (2026-11-27) :** D1(a). *Raison donnée par l'opérateur :* « le mot pour mot doit
être prouvé par construction, pas par une parité relue sur 15 à 21 vols ; l'autorité sur la règle
vit hors dépôt ». D1(c) — un outil de découpage versionné dans le dépôt — reste écarté
(mika#3122 règle 1).

**La permission est demandée par `senara-solutions/claude-pilot#325`**, et ce plan n'en connaît
**que la forme exacte suivante** :

```
sed -n '<N>,<M>p' skills/bundled/_shared/test-dispatch-lib.sh >  skills/bundled/_shared/tests/test_dispatch_lib_<nom>.sh
sed -n '<N>,<M>p' skills/bundled/_shared/test-dispatch-lib.sh >> skills/bundled/_shared/tests/test_dispatch_lib_<nom>.sh
```

avec quatre bornes : le script `sed` est **seulement** `N,Mp` ; la source est **seulement**
l'agrégat ; la cible correspond **seulement** à `tests/test_dispatch_lib_[a-z0-9_]+.sh` ; **sans
chaînage** (ni `;`, ni `&&`, ni `|`, ni substitution de commande).

Ce plan **ne prévoit aucune autre forme, aucun repli si la permission manque, et aucun outil de
découpage versionné.** Si le `sed` est refusé, le vol s'arrête et le refus se déclare.

**Les trois gestes qui produisent une suite**, et ils sont les seuls :

1. `Write` `skills/bundled/_shared/tests/test_dispatch_lib_<f>.sh` — l'en-tête seul (~14 lignes,
   § *L'ossature partagée*). C'est la **seule** écriture du fichier, et elle est la seule chose que
   le modèle rédige.
2. `sed -n '<début>,<fin>p' skills/bundled/_shared/test-dispatch-lib.sh >> …` — le corps de la
   famille, **mot pour mot par construction**.
3. `sed -n '19070,19080p' skills/bundled/_shared/test-dispatch-lib.sh >> …` — la queue de résumé
   (11 lignes : `# --- Summary ---`, les quatre `echo`, le `if [ "$FAIL" -gt 0 ]` et `exit 0`),
   **mot pour mot par construction elle aussi, et zéro ligne écrite**.

Aucune ligne du corps ni de la queue ne traverse le contexte du modèle. C'est précisément ce que
D1(a) achète : AC3 n'est plus une relecture, c'est une propriété de la commande.

**Reprise d'une suite ratée.** `Write` refuse d'écraser un fichier non lu, et le plan n'a pas besoin
d'un `>` pour tronquer : la reprise est `rm skills/bundled/_shared/tests/test_dispatch_lib_<f>.sh`
(**`rm` sans option**, instrument admis) puis les trois gestes depuis le début. La forme `>` de la
permission reste disponible mais ce plan ne s'en sert pas : l'en-tête doit précéder le corps.

### Ce que D1(a) n'ouvre pas, et comment les blocs sortent de l'agrégat

**La permission ne retire rien de l'agrégat.** `sed -i` reste refusé, et aucun geste de ce plan ne
le contourne. Les seuls instruments de retrait admis sont `Write`, `Edit` et `rm` sans option. Avec
eux, exactement deux voies existent :

| voie | geste | ce qu'elle coûte |
|---|---|---|
| **retrait de plage pendant la série** | `Edit` dont l'`old_string` est la plage entière (jusqu'à 1 289 lignes), donc lecture **et** écriture de chaque ligne dans le contexte | annule tout le bénéfice de D1(a) : le mot pour mot redevient une relecture. Et chaque retrait décale les plages survivantes, et fait maigrir l'agrégat, donc exige un `Edit` de `AGGREGATE_FROZEN_BYTES` au même commit |
| **l'agrégat reste entier jusqu'au vol 6** ✅ **retenue** | aucun retrait de plage ; au vol 6, `rm skills/bundled/_shared/test-dispatch-lib.sh` | les blocs tournent **deux fois** pendant 4 vols ; AC1, AC3 (clause « retirés ») et AC6 ne sont pleinement vrais qu'au vol 6 |

**La voie retenue est la seconde, et elle ne réserve aucune décision à l'opérateur** : elle
n'emploie que des instruments déjà admis, et ne demande donc ni permission nouvelle ni dérogation.
Ce plan **n'a pas besoin** d'une suppression de plage pendant la série ; il n'en pratique aucune et
n'en demande aucune.

**Le coût de la voie retenue, chiffré :**

- **Double exécution.** L'agrégat prend **1 min 11,8 s** mesurées (bac à sable de dispatch). Les
  suites extraites exécutent les mêmes blocs : le temps total de `make test-dispatch-lib` monte
  d'environ **+72 s** à partir du vol 2 et jusqu'au vol 6 — et autant sur `make test`, qui appelle
  aussi l'agrégat (`Makefile:144`). Soit **≈ 2 min 24 s** de CI supplémentaires par PR, sur quatre
  PR. C'est le prix entier, et il est borné dans le temps.
- **Blocs comptés deux fois dans les compteurs globaux.** `Results:` de l'agrégat reste à 2036/85/5
  ; chaque suite rend sa part en plus. Aucun compteur global ne doit donc être lu comme un total de
  la série — ce que R1 corrige en mesurant la parité **par famille**.
- **AC1, AC3 et AC6 satisfaits au vol 6 seulement.** Chaque vol intermédiaire est mergeable et
  n'ajoute aucune dette de câblage ni aucun rouge, mais il ne peut pas prétendre à AC1 (l'agrégat
  existe encore). Les vols 2 à 5 déclarent cela explicitement dans leur corps de PR.
- **Ce que la double présence achète, et ce n'est pas rien :** AC2 devient **directement
  mesurable à chaque vol**, puisque l'agrégat est encore là pour fournir le relevé de référence sur
  le **même commit** — ce que « prouvée sur le même commit » demande. Avec un retrait au fil de
  l'eau, la référence aurait vécu dans un commit antérieur.

### D2(b) — la borne mord sur les lignes écrites

**Décision retenue (2026-11-27) :** D2(b). *Raison donnée par l'opérateur :* « la borne plafonne le
travail écrit (raisonnement, surface de revue, risque de défaut) ; un bloc déplacé n'en est pas, et
D1(a) garantit déjà le mot pour mot ».

**La règle, écrite une fois et appliquée partout dans ce plan :**

> La borne par vol de 429 lignes mord sur les **lignes écrites** — bibliothèque d'ossature,
> en-têtes de suite, câblage du `Makefile`, re-ciblages de mentions, retraits de prose. Elle ne
> mord **pas** sur les lignes **déplacées** mot pour mot par le `sed` de D1(a), qui ne traversent
> ni le contexte du modèle ni sa rédaction. Chaque vol déclare **deux nombres distincts**, et le
> `Total estimé :` de la section *Taille estimée* porte les **lignes écrites**.

**Découpage : 6 vols.** 1 vol d'ossature, puis 5 vols de 5 suites, chacun sous l'enveloppe D1(a).

| vol | contenu | familles | **lignes écrites** | lignes déplacées |
|---|---|---|---|---|
| **1** ⬅ *vol courant* | `tests/lib-dispatch-lib-suite.sh` — l'ossature | — | **≈ 95** | 0 |
| 2 | 5 suites + câblage + mentions des familles 1–5 | 1–5 | ≈ 102 | 4 874 |
| 3 | 5 suites + câblage + mentions des familles 6–10 | 6–10 | ≈ 102 | 3 783 |
| 4 | 5 suites + câblage + mentions des familles 11–15 | 11–15 | ≈ 102 | 2 630 |
| 5 | 5 suites + câblage + mentions des familles 16–20 | 16–20 | ≈ 102 | 3 732 |
| 6 | 5 suites + câblage + clôture (`rm` de l'agrégat, retrait du trio de gel, re-ciblages résiduels) | 21–25 | ≈ 140 | 3 935 |

Somme des lignes écrites sur les six vols : **≈ 643**, sous le seuil par PR de 1 000 **et** chaque
vol sous la borne par vol de 429. Somme des lignes déplacées : **18 954** (plus la queue de 11
lignes recopiée 25 fois, qui est de l'ossature et n'est ni écrite ni comptée comme déplacée).

**Pourquoi le vol 1 ne porte aucune suite**, et ce qu'il faut savoir en le relisant : la
bibliothèque est le **seul** contenu réellement rédigé de toute la série — 95 lignes qui portent
une décision de conception (le ré-ancrage de `SCRIPT_DIR`) dont les 25 suites dépendent. La faire
relire seule évite de rejouer une erreur d'ossature sur 25 fichiers. Conséquence assumée : au merge
du vol 1, `tests/lib-dispatch-lib-suite.sh` n'est sourcée par **aucune** suite. Aucun lint ne s'en
plaint, et c'est mesuré : son nom ne porte aucun des cinq préfixes de `SUITE_PREFIX_RE` de
`scripts/check-suite-wiring.sh` (`test-`, `test_`, `check-`, `verify-`, `measure-`), exactement
comme ses deux voisins `lib-mika-bin.sh` et `lib-fake-egress-relay.sh`, et rien dans le dépôt
n'exige qu'un `lib-*.sh` ait un `source`. Le vol 1 se vérifie par `bash -n` et `shellcheck` ; sa
preuve fonctionnelle arrive au vol 2, où la parité par famille (R1) la rend mécanique.

**`Tracked in:` — tranché.** La suite des vols 2 à 6 est tracée par **#3178 lui-même**, qui reste
ouvert jusqu'au vol 6 et dont la table du § *Découpage retenu* est l'ordre de marche. Ce plan ne
fait pas ouvrir cinq tickets : chaque vol porte `Refs #3178`, et le vol 6 est le seul à écrire
`Closes #3178`. Raison : les cinq vols partagent un plan unique, un ordre fixe et un découpage que
ce document fige ; cinq tickets ne porteraient que leur numéro de vol, et ce numéro est déjà dans
la table. Le `Tracked in:` du corps de PR de chaque vol nomme donc `senara-solutions/mika#3178`.

## Découpage retenu

**Règle de gabarit.** Aucune nouvelle suite ne dépasse **1 500 lignes**, le gabarit de la plus
grosse suite existante du répertoire. AC1 n'exige que 50 % de `LARGE_FILE_LIMIT` (524 288 octets
≈ 9 600 lignes) ; la règle retenue est 7 fois plus serrée, parce que le motif du ticket est la
**lisibilité en revue**, pas le plafond. La plus grosse suite produite pèse ~70 Ko, soit **13 %**
du plafond.

**Règle de frontière.** Une famille est un ensemble de **bannières de premier niveau contiguës**.
Une famille n'est jamais coupée entre deux suites. Les plages commencent sur la **ligne vide** qui
précède la ligne de règle `# ====` ouvrant la bannière, pour que la bannière voyage avec sa famille.
**Les plages sont exprimées sur l'agrégat de la base `7b7ce6eb`, et elles restent valables sur toute
la série** (§ *Le gel de mika#3179* : l'agrégat ne grossit pas — gel — et ne maigrit pas — voie
retenue).

| # | vol | suite (`skills/bundled/_shared/tests/…`) | plage dans l'agrégat | lignes |
|---|---|---|---|---|
| 1 | 2 | `test_dispatch_lib_iterate_1271.sh` | 116–1095 | 980 |
| 2 | 2 | `test_dispatch_lib_escalate_1271.sh` | 1096–1930 | 835 |
| 3 | 2 | `test_dispatch_lib_groom_gate_2811.sh` | 1931–2815 | 885 |
| 4 | 2 | `test_dispatch_lib_push_divergence_1364.sh` | 2816–3921 | 1106 |
| 5 | 2 | `test_dispatch_lib_preflight_1472.sh` | 3922–4989 | 1068 |
| 6 | 3 | `test_dispatch_lib_groom_callback_1772.sh` | 4990–5987 | 998 |
| 7 | 3 | `test_dispatch_lib_gate_arms_3118.sh` | 5988–6679 | 692 |
| 8 | 3 | `test_dispatch_lib_output_gate_1996.sh` | 6680–7321 | 642 |
| 9 | 3 | `test_dispatch_lib_pr_state_2121.sh` | 7322–8004 | 683 |
| 10 | 3 | `test_dispatch_lib_plan_callout_2120.sh` | 8005–8772 | 768 |
| 11 | 4 | `test_dispatch_lib_arch_passes.sh` | 8773–9153 | 381 |
| 12 | 4 | `test_dispatch_lib_worktree_removal_1943.sh` | 9154–9402 | 249 |
| 13 | 4 | `test_dispatch_lib_fire_disposition.sh` | 9403–10311 | 909 |
| 14 | 4 | `test_dispatch_lib_halt_notes.sh` | 10312–11204 | 893 |
| 15 | 4 | `test_dispatch_lib_marker_position_2590.sh` | 11205–11402 | 198 |
| 16 | 5 | `test_dispatch_lib_token_parity_2608.sh` | 11403–12249 | 847 |
| 17 | 5 | `test_dispatch_lib_push_remote_2626.sh` | 12250–12975 | 726 |
| 18 | 5 | `test_dispatch_lib_plan_size_gate_2636.sh` | 12976–13910 | 935 |
| 19 | 5 | `test_dispatch_lib_per_flight_bound_2693.sh` | 13911–14618 | 708 |
| 20 | 5 | `test_dispatch_lib_arch_disposition_2641.sh` | 14619–15134 | 516 |
| 21 | 6 | `test_dispatch_lib_plan_size_estimate_2701.sh` | 15135–16423 | 1289 |
| 22 | 6 | `test_dispatch_lib_revise_termination_2077.sh` | 16424–16893 | 470 |
| 23 | 6 | `test_dispatch_lib_branch_resume_2976.sh` | 16894–17701 | 808 |
| 24 | 6 | `test_dispatch_lib_unparsed_relaunch_2996.sh` | 17702–18751 | 1050 |
| 25 | 6 | `test_dispatch_lib_launch_gate_3121.sh` | 18752–19069 | 318 |

25 suites, **18 954 lignes déplacées**, maximum 1 289 lignes. Les plages sont exhaustives et
disjointes, et leur somme se vérifie : 19 069 − 116 + 1 = **18 954**. Ce qui reste est le **prélude**
(lignes 1–115, qui devient la bibliothèque) et la **queue** (lignes 19070–19080, recopiée dans
chaque suite par le troisième geste de D1(a)).

## L'ossature partagée

**Fichier :** `skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh`, écrit au vol 1.

Il n'est **pas** une suite : son nom ne porte aucun des cinq préfixes de `SUITE_PREFIX_RE`, comme
ses deux voisins `lib-mika-bin.sh` et `lib-fake-egress-relay.sh`. Il n'a donc rien à déclarer dans
`scripts/suite-wiring.tsv`.

Il est **écrit à la main** (`Write`) et non extrait : la permission de D1(a) ne cible que
`tests/test_dispatch_lib_*.sh`. Ses 95 lignes sont donc les seules de la série que le modèle
transcrit, et la preuve qu'elles n'ont pas dérivé du prélude est la parité par famille du vol 2
(R1) : un `assert_*` modifié décale un compteur.

Ce qu'il porte, extrait des lignes 1–115 (hors les 10 lignes d'avertissement de gel, qui restent
sur l'agrégat) :

| contenu | origine dans l'agrégat |
|---|---|
| `set -euo pipefail` | ligne 26 |
| les 5 surcharges `GIT_CONFIG_*` d'herméticité (gpgsign, tag.gpgsign, init.defaultBranch, identité du committer) | lignes 39–49 |
| `SCRIPT_DIR` | ligne 51, **ré-ancré** — voir la décision ci-dessous |
| `DISPATCH_LIB="$SCRIPT_DIR/dispatch-lib.sh"` | ligne 52, orthographe inchangée |
| `PASS=0`, `FAIL=0` | lignes 54–55 |
| `SKIPPED=0` | ligne 58 (mika#2149) |
| `source "$SCRIPT_DIR/tests/lib-mika-bin.sh"` | ligne ~77, orthographe inchangée |
| `assert_eq()`, `assert_contains()`, `assert_not_contains()` | lignes 78, 91, 104 |

**Pas de fonction de résumé, et c'est une révision du 2026-11-27.** Le plan du 26/11 prévoyait un
`_suite_summary_and_exit()` dans la bibliothèque, appelé en dernière ligne de chaque suite. Deux
mesures l'ont remplacé par mieux :

- la queue de l'agrégat (lignes 19070–19080) est extractible par le **troisième geste de D1(a)**,
  donc recopiée **mot pour mot** dans chaque suite, pour **zéro ligne écrite** et zéro lecture ;
- l'autre voie envisageable — un `trap _suite_summary_and_exit EXIT` armé par la bibliothèque —
  est **écartée par mesure** : le `trap … EXIT` de la ligne 4238 (famille 5) l'écraserait, et la
  suite de la famille 5 sortirait **0 malgré ses FAIL**, sans imprimer son résumé. Le plan ne livre
  pas une ossature dont une suite sur 25 mentirait en silence.

**Décision de conception, et c'est celle qui rend AC3 littéralement vraie.** La bibliothèque calcule
`SCRIPT_DIR` comme le répertoire `_shared/`, **pas** `tests/` :

```bash
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
```

`${BASH_SOURCE[0]}` désigne la bibliothèque elle-même, donc `dirname` donne `tests/` et `/..` donne
`_shared/`. Raison : l'agrégat contient **26 chemins relatifs à `$SCRIPT_DIR`** ancrés sur
`_shared/`, répartis dans 11 familles différentes — `PROMPT_FILE="$SCRIPT_DIR/../self-dev/system_prompt.md"`
(225), `"$SCRIPT_DIR/../../../.."` (682), `GROOMING_FIXTURE_DIR` (1764), `REPO_ROOT` (6758),
`M2194_CORPUS` (11332), `M2608_TOKENS_TSV` (12209), `t2636_pf` (13675), `t2693_pf` (14303),
`M2641_ANCHOR_MANIFEST` (15013), `M3_CORPUS` (16285), `t3122_pf` (18670). Ré-ancrer `SCRIPT_DIR`
sur `_shared/` les laisse **octet pour octet identiques** : aucune de ces 26 lignes n'est réécrite,
et le `sed` de D1(a) ne saurait pas les réécrire de toute façon. L'alternative — un `SCRIPT_DIR`
pointant sur `tests/` — exigerait de réécrire 26 chemins, ce que le ticket interdit (« les blocs
sont déplacés mot pour mot ») et ce que D1(a) rend structurellement impossible.

**La seule exception connue** est `test_callout_plan_identity_3174.sh`, qui vit déjà sous `tests/`
et épelle `DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"`. Il n'est pas migré vers la bibliothèque
par ce plan : il est hors périmètre et sa migration serait un changement de code sans défaut à
l'appui.

**Ossature par suite**, ~14 lignes, et c'est l'exclusion du `diff` d'AC3 :

```bash
#!/bin/bash
# <titre de la famille> — extrait de test-dispatch-lib.sh par mika#3178, vol <n>.
# <deux à quatre lignes disant ce que la famille tient>
#
# Plage d'origine : lignes <N>–<M> de skills/bundled/_shared/test-dispatch-lib.sh
# à la base <sha>. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178 D1(a),
# permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
```

suivi du corps (plage `N`–`M`) puis de la queue (plage 19070–19080), tous deux ajoutés par `sed`.

**Règle de placement des helpers**, telle que le ticket la pose : un helper appelé par plusieurs
familles va dans la bibliothèque ; un helper propre à une famille reste avec elle. La mesure dit
que le second cas est le seul cas réel : hors les trois `assert_*`, **aucune** des 208 fonctions de
l'agrégat n'est appelée depuis plus d'une famille. Les candidates vérifiées — `mika()` /
`mika_stdin_capture` (927, 948, annulées en 952), `_fixture_setup` / `_fixture_cleanup` /
`_assert_fixture_is_local` (2826, 2842, 2847), `_t2144_scaffold_decl_scan` (3853),
`_test_policy_deny_check` (4217), `_parse_prompt` (4537), `_fip_probe` (4576),
`_iterate_*_probe` (5073, 5089), `_classify_probe` (5117), `_pfr_probe_1772` (5248),
`_t3030_seed_and_read` (18182) — sont toutes confinées à une plage unique. La bibliothèque reste
donc minimale : 95 lignes, et pas une fonction de fixture.

## Preuve de non-perte

AC2 demande l'égalité des sommes PASS et SKIPPED, et `FAIL = 0` des deux côtés. Quatre
renforcements sont nécessaires pour que la preuve morde réellement.

**R1 — la parité se mesure PAR FAMILLE, pas seulement en somme.** Une somme globale masque une
paire compensatoire : une assertion perdue dans la famille 7 et une double-comptée dans la 19
laissent le total inchangé. Pour chaque suite `k`, les trois compteurs rendus doivent égaler les
trois compteurs que la **même plage** produit dans l'agrégat. Le relevé par plage s'obtient en
exécutant l'agrégat et en découpant sa sortie sur les lignes de bannière : avec la voie retenue,
l'agrégat est **encore présent dans le commit du vol**, donc ce relevé est pris sur le même commit
que la suite — ce que « prouvée sur le même commit » demande, sans référence à un commit antérieur.

Cette mesure par famille est aussi la **preuve mécanique de l'absence de fuite d'état** que le
point 3 du § *Ce que la mesure a fermé* laisse ouverte. Elle ne demande pas d'énumérer les 796
affectations de premier niveau : `set -u` est vivant, donc une variable amont non résolue est une
erreur dure et non une dégradation silencieuse ; et une fonction amont manquante fait rougir
l'`assert_*` qui compare contre une capture vide, ce qui déplace `FAIL`. Toute fuite se lit dans
l'écart de compteurs d'une famille.

**R2 — `FAIL = 0` n'est attestable que là où l'agrégat est vert.** Mesuré le 2026-11-27 dans le bac
à sable de dispatch : `Results: 2036 passed, 85 failed, SKIPPED: 5`. La clause « `FAIL = 0` des deux
côtés » d'AC2 est donc **inatteignable dans l'environnement du pilote**, et aucune réécriture
d'assertion ne peut la rendre atteignable — ce serait exactement ce que le ticket interdit, et
D1(a) l'interdit par construction. Le contrat de vérification retenu :

- **l'égalité des comptes** (PASS, SKIPPED, et FAIL par famille) se mesure dans **un seul**
  environnement, des deux côtés : elle tient dans le bac à sable malgré les 85 échecs, puisque les
  85 se retrouvent à l'identique du côté des suites ;
- **`FAIL = 0`** est attesté par l'étape `Dispatch-lib assertion suite (mika#1772)` de
  `ci.yml:85`, verte sur `main` et exécutant `make test-dispatch-lib` ;
- le corps de PR cite **les deux mesures séparément**, avec leur environnement nommé. Un unique
  compte sans son environnement serait un chiffre qui ne dit pas d'où il vient.

**R3 — AC5, rouge avant.** Une assertion par suite est faussée, la suite doit rendre un code non
nul, puis l'assertion est rétablie — **5 faussages par vol**, 25 au total, notés dans le corps de PR
avec nom de suite et numéro de ligne. C'est le seul instrument qui prouve que les suites sont
**armées** et pas seulement présentes : une suite dont le corps ne s'exécuterait pas rendrait 0 sur
une assertion fausse. Le faussage et son rétablissement passent par `Edit` : ils portent sur **une**
ligne, jamais sur une plage.

**R4 — `bash -n` sur chaque suite, avant de la lancer.** Nouveau, et il couvre la seule classe de
défaut qu'un découpage par numéros de ligne produit silencieusement : une frontière tombée **à
l'intérieur d'un here-document** (l'agrégat en contient de nombreux, dont quatre portant un `exit`
en colonne zéro — § *Ce que la mesure a fermé*, point 6). Un délimiteur non fermé est une erreur de
syntaxe, donc `bash -n` la voit ; les compteurs, eux, pourraient ne pas la voir. L'ordre est :
`bash -n`, puis exécution, puis parité, puis AC5.

## Câblage

- Chaque suite est câblée **directement** dans les cibles `test` et `test-dispatch-lib` du
  `Makefile` — 2 lignes `@bash skills/bundled/_shared/tests/test_dispatch_lib_<f>.sh` par suite,
  50 lignes au total, **10 par vol**. Le ticket dit pourquoi : `check-suite-wiring.sh` ne voit pas
  un appel fait depuis le corps d'une autre suite, et sous-approximer la joignabilité donne des
  faux rouges, jamais des faux verts (mika#2761).
- **Au vol 6 seulement :** l'appel `@bash skills/bundled/_shared/test-dispatch-lib.sh` de
  `Makefile:144` est retiré, ainsi que celui de la cible `test-dispatch-lib` (`Makefile:207`), et le
  trio de gel de mika#3179 est retiré des deux cibles (§ *Le gel de mika#3179*).
- **La cible `test-dispatch-lib` garde son nom** après la disparition du fichier homonyme :
  `.github/workflows/ci.yml:85` lit `run: make test-dispatch-lib`, et c'est la **cible** qu'il
  nomme, pas le fichier. Renommer la cible casserait la seule porte de merge de ces suites.
- La joignabilité des 25 suites passe par le hop **H2** (le workflow nomme la cible, le lint suit la
  recette, ses prérequis et la récursion `$(MAKE)`). **Aucune entrée dans
  `scripts/suite-wiring.tsv`** : les 25 suites sont atteintes, et le lint est bidirectionnel — une
  entrée pour une suite que la CI atteint le fait rougir.
- **Population.** `check-suite-wiring.sh` rapporte aujourd'hui `|D|=115 (floor 85) |C|=113 |R|=2
  unwired=0`. Les 25 suites portent `|D|` à 140 ; le vol 6 en retire 2 (le lint de gel et son
  harnais, tous deux préfixés) et l'agrégat, soit **137**. Le plancher `# min-population: 85` du
  TSV est franchi par le haut à chaque étape, aucune modification requise.

## Références (AC6)

`git grep -n "test-dispatch-lib"` hors `docs/plans` et `docs/brainstorms` au 2026-11-27 :
**81 fichiers, 183 lignes** (le 26/11 : 78 et 170 ; mika#3179 a ajouté les 13 lignes de son lint,
de son harnais et de l'avertissement). La règle de tri retenue, et le plan la pose explicitement
parce qu'AC6 ne la pose pas :

> Une mention se re-cible quand elle dit à un lecteur **d'aujourd'hui** où regarder. Un relevé
> d'incident daté sous `docs/solutions/**` reste écrit tel quel : il décrit ce qui était vrai à sa
> date, et le réécrire falsifierait un témoignage.

Cette règle suit l'énumération du ticket, qui nomme `crates/mika-agent`, `crates/mika-cli`, les
README et TSV de `crates/mika-agent/tests/fixtures/`, le TSV d'inventaire canonique et plusieurs
scripts de `scripts/` — et **ne nomme pas** `docs/solutions`. Les **33 fichiers** de
`docs/solutions/**` (**64 lignes**) sont donc hors re-ciblage, et le corps de PR le dit avec sa
raison.

**Ventilation des 109 lignes hors `docs/`**, re-mesurée :

| site | lignes | nature | traitement |
|---|---|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` | 31 | commentaires nommant un bloc précis (`S4`, `S11`, `T5`, `T6 drift guard`, `T12j`, `case B`, `rescue-block extraction`, …) | re-ciblés au vol de leur famille — voir l'hypothèse déclarée ci-dessous |
| `Makefile` | 8 | câblage + prose de mika#3174 et mika#3179 | réécrit au fil des vols, soldé au vol 6 |
| `skills/bundled/_shared/test-dispatch-lib.sh` | 5 | mentions internes | partent avec le fichier au vol 6 |
| `skills/bundled/_shared/tests/**` (10 fichiers) | 14 | commentaires de voisinage, dont `test_callout_plan_identity_3174.sh` (4) | re-ciblés au vol de leur famille |
| `scripts/check-dispatch-lib-aggregate-frozen.sh` + son harnais | 7 | prose du gel | partent avec les deux fichiers au vol 6 |
| `crates/mika-agent/src/**` (8 fichiers) | 13 | commentaires de parité bash↔Rust, dont `bundled_skills.rs:3058-3059` | re-ciblés au vol de leur famille ; `bundled_skills.rs` au vol 6 |
| `crates/mika-agent/tests/**` (4 fichiers) | 5 | commentaires de parité et d'ancrage | re-ciblés au vol de leur famille |
| `crates/mika-agent/tests/fixtures/**` (4 fichiers) | 8 | README et `expectations.tsv` | re-ciblés au vol de leur famille |
| `scripts/` (9 autres fichiers) | 11 | `canonical-tokens.tsv` (2), `canonical-tokens-survey.sh` (2), `check-suite-wiring.sh:49`, `test-check-suite-wiring.sh:267`, `verify-pipeline.sh`, `verify-no-sigpipe-grep.sh` + sa suite, `test-guard-shared-checkout.sh`, `test-shell-exec-guard.sh` | re-ciblés au vol de leur famille ; la prose des deux `*-suite-wiring*` au vol 6 |
| `skills/bundled/mika-arch-*/system_prompt.md` | 4 | prose d'orientation de l'architecte | vol 6 |
| `crates/mika-cli/src/commands/arch_disposition.rs` | 1 | commentaire de parité | vol de sa famille |
| `crates/mika-agent/CLAUDE.md` | 1 | index | vol 6 (hors comptage de lignes, mika#2636) |
| `.github/workflows/ci.yml` | 1 | `run: make test-dispatch-lib` — nomme la **cible** | **rien à faire** |

S'y ajoutent **10 lignes sous `docs/` hors `docs/solutions`** — `docs/architecture/dispatch-lib-migration.md`
(4 : l. 278, 329, 455, 846), `docs/operator/environment-variables.md` (5 : l. 1362, 1426, 3655,
7442, 7927), `docs/agent-internals/exec-handlers.md` (1 : l. 231) : ce sont des références
vivantes, pas des relevés datés, donc la règle ci-dessus les prend. Vol 6.

**Quand un re-ciblage se fait, et pourquoi pas tout au vol 6.** Une mention qui dit « le bloc
mika#2194 de `test-dispatch-lib.sh` » devient re-ciblable dès que le bloc **existe** dans sa
nouvelle suite : la phrase « le bloc mika#2194 de `tests/test_dispatch_lib_token_parity_2608.sh` »
est vraie au vol où la famille 16 est extraite, même si l'agrégat en garde encore une copie. Les
mentions suivent donc **leur famille**, ~20 lignes par vol. Le vol 6 porte le reste : les mentions
qui nomment le fichier sans nommer de bloc, les deux proses de `*-suite-wiring*`, le TSV canonique,
`bundled_skills.rs`, les prompts de l'architecte, les 10 lignes sous `docs/`, et le `git grep`
final d'AC6. Répartir ainsi est aussi ce qui tient la borne : 114 `Edit` d'une ligne concentrés sur
un seul vol coûtent ~114 tours sur les 143 utilisables, et la borne en lignes ne le verrait pas.

**Hypothèse déclarée, et elle demande à être lue.** Le périmètre du ticket exclut « tout changement
de `dispatch-lib.sh` », et AC6 exige que plus aucune mention ne désigne un bloc déplacé. Les 31
commentaires de `dispatch-lib.sh` désignent tous un bloc qui bouge : les deux clauses se croisent.
Le plan tranche par la lecture la plus étroite possible — **l'exclusion protège le comportement,
pas les commentaires** — et la rend mécaniquement vérifiable :

> `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` ne montre que des lignes
> commençant par `#` (ou des commentaires de fin de ligne). Aucune ligne exécutable ne change.
> Le corps de PR de chaque vol cite ce `diff`.

Si l'opérateur lit l'exclusion littéralement, ces 31 occurrences sortent du périmètre et AC6 est
borné aux autres. Le plan ne pratique pas ce second choix ; il le signale, parce qu'il est bon
marché à trancher et qu'il ne bloque rien d'autre.

## Ordre de commit

**Vol 1 — l'ossature.**

1. `Write` `skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh` (95 lignes, `SCRIPT_DIR`
   ré-ancré sur `_shared/`, pas de fonction de résumé).
2. `bash -n` et `shellcheck` sur le fichier.
3. `bash scripts/check-suite-wiring.sh`, `bash scripts/check-secrets.sh --changed origin/main`,
   `bash scripts/canonical-tokens-survey.sh --check`, `make verify-bundled-skills`,
   `bash scripts/verify-pipeline.sh origin/main`.
4. Le corps de PR dit que la bibliothèque n'est encore sourcée par aucune suite, et pourquoi aucun
   lint ne s'en plaint.

**Vols 2 à 5 — cinq suites chacun.** Pour chaque famille du vol, dans l'ordre de la table :

1. `Write` l'en-tête de la suite (~14 lignes).
2. `sed -n '<début>,<fin>p' skills/bundled/_shared/test-dispatch-lib.sh >> <suite>` — le corps.
3. `sed -n '19070,19080p' skills/bundled/_shared/test-dispatch-lib.sh >> <suite>` — la queue.
4. `bash -n <suite>` (R4), puis exécution de la suite.
5. Parité des trois compteurs contre la même plage dans l'agrégat (R1), relevée sur ce commit.
6. AC5 : une assertion faussée, code non nul constaté, assertion rétablie (R3).

puis, une fois les cinq faites :

7. `Makefile` — 10 lignes de câblage (5 suites × 2 cibles). **L'agrégat garde ses deux appels.**
8. Re-ciblage des mentions qui nomment un bloc des cinq familles (~20 lignes).
9. La batterie de vérification du point 3 du vol 1, au complet, plus `make test-dispatch-lib`.
10. Le corps de PR : les deux nombres de D2(b) (lignes écrites / lignes déplacées), les relevés de
    parité par famille, la séquence AC5, le `diff` de `dispatch-lib.sh`, et la mention explicite
    qu'AC1, AC3 (clause « retirés ») et AC6 ne seront vrais qu'au vol 6.

**Vol 6 — les cinq dernières suites, puis la clôture.** Les étapes 1 à 8 ci-dessus pour les
familles 21 à 25, puis, **dans le même commit** :

9. `rm skills/bundled/_shared/test-dispatch-lib.sh` (`rm` sans option).
10. `rm scripts/check-dispatch-lib-aggregate-frozen.sh` et
    `rm scripts/test-check-dispatch-lib-aggregate-frozen.sh`, et retrait de leurs 4 lignes `@bash`
    (l. 194, 195, 219, 220) et des 8 lignes de commentaire qui les introduisent (l. 190–193 et
    215–218) — **en citant mika#3179**, dont le message vert du lint prescrit exactement
    ce geste. Sans cela, `make test-dispatch-lib` rougit : le harnais sort en vacuité (exit 3) sur
    un agrégat absent (`test-check-dispatch-lib-aggregate-frozen.sh:82-88`).
11. `Makefile` — retrait des deux appels de l'agrégat (l. 144 et 207) et de la prose de mika#3174
    qui le nomme. **La cible `test-dispatch-lib` garde son nom** (`ci.yml:85`).
12. `crates/mika-agent/src/bundled_skills.rs` — re-ciblage de `test_support_dirs_exclude_test_files`
    sur `skills/bundled/_shared/tests/test_dispatch_lib_launch_gate_3121.sh`, pour qu'elle ne
    devienne pas vacante.
13. Prose de `scripts/check-suite-wiring.sh:49` et `scripts/test-check-suite-wiring.sh:267`,
    `scripts/canonical-tokens.tsv` (l. 253 et 306), les prompts `mika-arch-*`, les 10 lignes sous
    `docs/`, `crates/mika-agent/CLAUDE.md`, et toute mention résiduelle.
14. `git grep -n "test-dispatch-lib"` cité dans le corps de PR : plus aucune mention ne désigne un
    bloc absent (AC6). `Closes #3178`.

## Fire-Disposition

Option **(a) exception nommée en allowlist**, avec un **ensemble d'exceptions vide**.

Le plan livre 25 détecteurs, mais **aucun détecteur nouveau** : les 1 809 lignes d'assertion sont
déplacées mot pour mot par le `sed` de D1(a), et leur chemin de succès est inchangé. Aucune
violation existante n'a donc besoin d'une exception, et aucune n'en reçoit :

- **pas d'entrée dans `scripts/suite-wiring.tsv`** — les 25 suites sont câblées dans le `Makefile`
  et atteintes depuis `ci.yml:85` par le hop H2. Le lint étant bidirectionnel, une entrée pour une
  suite atteinte le ferait rougir : l'exception serait sa propre régression ;
- **pas d'entrée dans `LARGE_FILE_ALLOWLIST`** — hors périmètre, et sans objet : la plus grosse
  suite produite pèse 13 % du plafond ;
- **pas de relève de `AGGREGATE_FROZEN_BYTES`** — la voie retenue ne fait pas maigrir l'agrégat
  avant le vol 6, où il disparaît avec son lint. La constante n'est touchée à aucun vol, et le
  contrôle P du harnais reste vert par construction.

**Les deux assertions auto-nettoyantes** qui rougissent quand l'état change sous elles :

1. `scripts/check-suite-wiring.sh` (règle D\C==R, lint `suite-wiring-lint`) rougit dès qu'une des
   25 suites perd son appelant — c'est le détecteur qui tient le câblage d'AC4 dans le temps.
2. `crates/mika-agent/src/bundled_skills.rs::test_support_dirs_exclude_test_files`, **re-ciblée**
   au vol 6 sur `skills/bundled/_shared/tests/test_dispatch_lib_launch_gate_3121.sh`. Laissée sur
   `test-dispatch-lib.sh`, elle devient vacante le jour où l'agrégat disparaît et passe au vert par
   absence de sujet — l'exacte forme de défaut que le dépôt a documentée deux fois. Le re-ciblage
   est la condition pour que l'exclusion de seeding reste **prouvée** et non supposée.

**Une exception nommée, et elle est environnementale, pas en allowlist.** Les 85 échecs mesurés le
2026-11-27 dans le bac à sable de dispatch préexistent sur la base (`Results: 2036 passed, 85
failed, SKIPPED: 5`) et sortent sur `fatal: Not a valid object name origin/main` et un `mkdir:
command not found`. Ils ne reçoivent **pas** d'exception dans le dépôt : ils sont nommés, chiffrés
et attribués dans le corps de PR, et `FAIL = 0` est attesté là où il est vrai — l'étape
`Dispatch-lib assertion suite (mika#1772)` de `ci.yml:85`. Leur traitement est au § *Preuve de
non-perte*, R2.

**AC5 est l'armement.** 5 faussages délibérés par vol, 25 au total, chacun devant produire un code
de sortie non nul : c'est la seule preuve qu'un détecteur déplacé est encore armé et pas seulement
présent.

## Taille estimée

**La règle de comptage, et elle vient d'une décision d'opérateur.** Par **D2(b)** (tranchée le
2026-11-27, corps de mika#3178), la borne par vol mord sur les **lignes écrites** — bibliothèque,
en-têtes, câblage, re-ciblages — et **non** sur les lignes déplacées mot pour mot par le `sed` de
D1(a), qui ne traversent ni le contexte du modèle ni sa rédaction. Le total ci-dessous porte donc
les lignes écrites du **vol courant**, le vol 1.

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh` (l'ossature, écrite) | 95 |
| lignes déplacées par ce vol | 0 |

Total estimé : 95 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les six vols, pour mémoire** (le détail est au § *D2(b)*) : lignes écrites 95, 102, 102, 102, 102,
140 — chacune sous la borne de 429 ; lignes déplacées 0, 4 874, 3 783, 2 630, 3 732, 3 935, soit
18 954 au total. La somme des lignes **écrites** sur toute la série est de **643**, sous le seuil
par PR de 1 000.

Tracked in: senara-solutions/mika#3178 — le ticket reste ouvert jusqu'au vol 6 et porte les vols 2
à 6 ; la table du § *Découpage retenu* en est l'ordre de marche, et le vol 6 est le seul à écrire
`Closes #3178`.

## Definition of Done

Du vol courant (vol 1) :

- [ ] `skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh` existe et porte l'ossature
      inventoriée, avec `SCRIPT_DIR` ancré sur `_shared/` et **sans** fonction de résumé.
- [ ] `bash -n` et `shellcheck` sont verts sur ce fichier.
- [ ] `bash scripts/check-suite-wiring.sh` est vert sans entrée nouvelle dans `suite-wiring.tsv`.
- [ ] `bash scripts/check-secrets.sh --changed origin/main`,
      `bash scripts/canonical-tokens-survey.sh --check`, `make verify-bundled-skills`,
      `make test-dispatch-lib` et `bash scripts/verify-pipeline.sh origin/main` sont verts.
- [ ] Le corps de PR dit que la bibliothèque n'est pas encore sourcée, et pourquoi c'est sûr.

De la série (vols 2 à 6, mêmes critères, un vol à la fois) :

- [ ] Les 25 suites de la table existent, chacune sous 1 500 lignes, chacune ne contenant que son
      en-tête, sa plage et la queue — les deux dernières extraites par `sed` (D1(a)).
- [ ] `bash -n` est vert sur chaque suite **avant** son premier lancement (R4).
- [ ] La parité des trois compteurs est mesurée **par famille** et non seulement en somme (R1).
- [ ] `FAIL = 0` est attesté par `ci.yml:85`, et les 85 échecs du bac à sable sont nommés et
      attribués dans le corps de PR (R2).
- [ ] 5 faussages d'AC5 par vol, 25 au total, avec leur séquence notée dans le corps de PR.
- [ ] Les suites du vol sont câblées directement dans `test` et `test-dispatch-lib`.
- [ ] `AGGREGATE_FROZEN_BYTES` n'est touchée à aucun vol ; au vol 6, le lint, son harnais et leurs
      lignes de `Makefile` sont retirés en citant mika#3179.
- [ ] Au vol 6 : l'agrégat est supprimé, `test_support_dirs_exclude_test_files` cible un chemin
      survivant et n'est pas vacante, et la cible `test-dispatch-lib` garde son nom.
- [ ] `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` ne montre que des
      commentaires, à chaque vol.

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

**Comment les six AC se distribuent sur les six vols.** AC2, AC3 (clause « déplacement »), AC4 et
AC5 sont vérifiés **à chaque vol**, sur les familles de ce vol. AC1, AC3 (clause « blocs retirés »)
et AC6 ne sont pleinement vrais qu'au **vol 6**, parce que l'agrégat reste entier jusque-là
(§ *Ce que D1(a) n'ouvre pas*). Chaque corps de PR intermédiaire le déclare, plutôt que de laisser
un AC non coché passer pour un oubli.

## Risques

- **La permission de D1(a) n'est pas encore mergée.** Les vols 2 à 6 ne partent pas sans elle, et
  ce plan n'a aucun repli : c'est une condition de départ, vérifiable d'un `gh issue view`, pas un
  jugement à porter en vol.
- **Une frontière dans un here-document.** Couvert par R4 (`bash -n`), qui la transforme en erreur
  de syntaxe immédiate au lieu d'un test muet.
- **Une dérive de l'ossature transcrite au vol 1.** Les 95 lignes de la bibliothèque sont les seules
  que le modèle retape ; une dérive d'un `assert_*` se lit dans la parité par famille du vol 2 (R1),
  avant toute généralisation aux 24 autres suites. C'est la raison d'être du vol 1 séparé.
- **La double exécution allonge la CI de ≈ 2 min 24 s par PR pendant quatre PR.** Chiffré,
  temporaire, et c'est le prix de la voie qui n'exige aucune permission supplémentaire.
