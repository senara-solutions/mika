# mika#3178 — scinder `test-dispatch-lib.sh` : **vol 4/7**, les familles 11 à 15

- **Issue :** senara-solutions/mika#3178 (`Refs #3174, #3177`)
- **Branche :** `test/3178/dispatch-lib-scinder-test-dispatch-lib`
- **Base mesurée : `abaa73c9`**, au-dessus de `3849d757` (`main`, le merge du vol 3).
- **Vol courant : le vol 4.** Les vols 1 à 3 sont mergés : `b60bd55d` (PR #3183, l'ossature,
  128 lignes), `8de06d66` (PR #3186, familles 1 à 5, 155 écrites / 5 363 déplacées), `3849d757`
  (PR #3187, familles 6 à 10, 84 écrites / 3 838 déplacées). Ce plan succède à
  `docs/plans/2026-11-28-001-…-plan.md`, qui reste en place comme document du vol 3 ; la table des
  25 familles en est reconduite.
- **Label portant le plafond :** `loop-substrate` (200 tours).

## Ce que ce plan est : le relevé d'un vol **déjà produit**, pas un pari

Le vol 4 est le premier de la série dont le travail de production est **déjà sur la branche** avant
son plan. Deux grooms l'ont précédé le 2026-11-28 ; aucun n'a écrit de plan, et le corps du ticket
en tire la consigne de celui-ci.

- **Premier groom (13:23Z).** Il a extrait au lieu de planifier, puis s'est arrêté sur un refus de
  la politique de permission. La récupération post-vol de mika#2031 a commité son produit en
  `abaa73c9` — un commit `wip()`, donc une **entrée mesurée**, jamais un vol livré.
- **Second groom (14:04Z).** Il a passé ses 18 tours à re-vérifier : les cinq `diff` de plage contre
  les cinq suites sont revenus **exacts**. Il s'est arrêté sur un second refus (substitution de
  processus) avant d'écrire une ligne de plan.

**Conséquence directe sur l'ordre de marche : le dev-pilot du vol 4 ne re-extrait rien et ne
re-vérifie aucune frontière.** Les vérifications sont faites, et refaire un `diff` déjà revenu exact
coûte des tours sans rien établir. Ce que `abaa73c9` contient est acquis ; ce qui reste est le
câblage, un compteur, AC5 et la fin de pipeline.

## Les cinq familles du vol 4, telles qu'elles sont sur la branche

Frontières relevées sur l'agrégat à la base `3849d757`, reprises du corps du ticket sans
re-mesure ; corps et queue extraits par le `sed` de D1(a), en-têtes écrits par `Write`.

| # | suite | plage | lignes du fichier | en-tête | corps + queue | relevé |
|---|---|---|---|---|---|---|
| 11 | `test_dispatch_lib_arch_passes.sh` | 8773–9153 | 408 | 16 | 381 + 11 | **44 PASS / 0 FAIL** |
| 12 | `test_dispatch_lib_worktree_removal_1943.sh` | 9154–9402 | 275 | 15 | 249 + 11 | **à relever** |
| 13 | `test_dispatch_lib_fire_disposition.sh` | 9403–10311 | 940 | 20 | 909 + 11 | **131 PASS / 0 FAIL** |
| 14 | `test_dispatch_lib_halt_notes.sh` | 10312–11204 | 925 | 21 | 893 + 11 | **139 PASS / 0 FAIL** |
| 15 | `test_dispatch_lib_marker_position_2590.sh` | 11205–11402 | 223 | 14 | 198 + 11 | **18 PASS / 0 FAIL** |

**Le compteur de la famille 12 ne figure dans aucun des deux transcripts.** Il est le seul chiffre
manquant d'AC2 pour ce vol, et il est au premier rang de l'ordre de commit — nommé comme absent
plutôt que deviné à partir de ses 249 lignes de corps. Somme partielle des PASS : **332**, plus la
famille 12.

### La bibliothèque de sondes, et pourquoi elle est neuve

Deux dépendances entre familles, nommées à l'exécution comme aux vols 2 et 3 :

| dépendance | définie à | lue par |
|---|---|---|
| `_t2178_line_of`, `_t2178_after` | l. **7419–7431** (famille 9) | **famille 13** |
| `_gate_probe` | l. **6858–6920** (famille 8) | **famille 14** |

Les familles 8 et 9 sont parties au vol 3 : leurs définitions ne sont plus dans aucune plage que le
vol 4 extrait. Remède, celui que le corps du ticket tranche d'avance (« un helper utilisé par
plusieurs familles va dans cette bibliothèque ») : une bibliothèque neuve,
`skills/bundled/_shared/tests/lib-dispatch-lib-shared-probes-fixtures.sh` — 58 lignes d'en-tête
écrites, puis les deux plages extraites par `sed … >>`, forme admise par **claude-pilot#328**. Les
familles 13 et 14 la `source` depuis leur en-tête. Vérifiée octet pour octet contre l'agrégat à
`3849d757` par le premier groom.

Le nom n'est pas libre : claude-pilot#328 n'admet que `lib-dispatch-lib-<nom>-fixtures.sh` en `>>`.
C'est pourquoi des sondes s'appellent `…-fixtures.sh`. Un troisième nom serait **un arrêt à
déclarer, pas un geste à contourner** (corps du ticket, mika#3122 règle 1).

### La mesure de D2(b), par `git diff --numstat 3849d757 abaa73c9`

Le numstat rend **2 905 lignes ajoutées, 0 retirée** sur six fichiers. La décomposition se recoupe
exactement avec les frontières ci-dessus — c'est ce recoupement, et non un `diff` de plus, qui
établit que `abaa73c9` est bien ce que le corps du ticket décrit :

| | lignes |
|---|---|
| 5 corps de famille (381 + 249 + 909 + 893 + 198) | 2 630 |
| 5 queues de résumé (5 × 11) | 55 |
| 2 plages de dépendances (13 + 63) | 76 |
| **déplacées, mot pour mot par `sed`** | **2 761** |
| 5 en-têtes de suite (16 + 15 + 20 + 21 + 14) | 86 |
| en-tête de la bibliothèque de sondes | 58 |
| **écrites** | **144** |
| **total** | **2 905** ✓ |

Les « 2 685 + 76 » de la table du ticket sont confirmés : 2 685 est bien la part des cinq suites
(corps + queues) et 76 celle des deux dépendances. Le total déplacé du vol est **2 761**.

## Ce qui reste à faire

Cinq choses, et le plan n'en invente aucune — ce sont celles que le corps du ticket énumère.

1. **Le compteur de la famille 12** (§ ci-dessus).
2. **Le câblage du `Makefile`** — cinq appels dans la cible `test` et cinq dans `test-dispatch-lib`.
3. **Les cinq faussages d'AC5.**
4. **La note Compound** — enrichissement de
   `docs/solutions/test-failures/scinder-une-suite-une-plage-contigue-nest-pas-auto-suffisante-2026-11-27.md`
   avec la troisième occurrence du motif : au vol 4, la dépendance ne vient plus d'une famille du
   même vol mais d'**une famille déjà partie au vol précédent**, ce qui la rend invisible à la
   relecture de l'agrégat courant. Hors borne (sous `docs/`).
5. **Le corps de PR.**

## Ordre de commit

Le travail de production est acquis en `abaa73c9`. Le dev-pilot reprend sur **cette même branche,
par commits neufs** — il ne rebase pas, ne modifie pas `abaa73c9`, et ne touche ni les suites, ni
les bibliothèques, ni les plans des vols 1 à 3 (présents parce qu'ils viennent de `main`).

1. **`cargo build -p mika-cli --bin mika`** — avant tout lancement de suite. Mesuré au vol 3 : sans
   `target/debug/mika`, les familles rendent des échecs d'environnement et l'une en rendait un de
   plus que l'agrégat sur la même plage.
2. **Lancer les cinq suites une par une**, depuis la racine du worktree en chemin relatif, et
   relever les trois compteurs de chacune. La famille 12 est la seule dont le chiffre manque ; les
   quatre autres sont une **re-confirmation** attendue à 44 / 131 / 139 / 18, et un écart y serait
   un fait à déclarer, pas à corriger en silence.
3. **`Makefile`** — les cinq appels dans `test` et les cinq dans `test-dispatch-lib`, précédés d'une
   ligne de commentaire disant « vol 4/7, familles 11 à 15 » et pourquoi le câblage est direct
   (`check-suite-wiring.sh` ne voit pas un appel fait depuis le corps d'une autre suite —
   mika#2761). **Aucune entrée dans `scripts/suite-wiring.tsv`** : la comparaison du lint est
   bidirectionnelle, une entrée pour une suite que la CI atteint la fait rougir.
4. **AC5** — cinq faussages délibérés, un par suite, chacun mesuré sur le compteur `FAIL`
   (`n` → `n+1` → `n`), puis rétablis.
5. **Les portes** — `bash scripts/check-suite-wiring.sh`,
   `bash scripts/check-secrets.sh --changed origin/main`,
   `bash scripts/canonical-tokens-survey.sh --check`, `make verify-bundled-skills`,
   `bash scripts/verify-pipeline.sh origin/main`.
6. **La note Compound**, puis le corps de PR et la PR (`Refs #3178`, jamais `Closes`).

**Plancher du vol :** `abaa73c9` tel quel, plus le câblage des cinq suites dans les deux cibles et
le compteur de la famille 12 — mergeable seul. AC5 et la note Compound s'ajoutent sans
interdépendance.

## La limite du bac à sable, déclarée et non contournée

Au vol 3, **`make test-dispatch-lib` a été refusé par la politique de permission** du bac à sable de
dispatch. Le câblage s'atteste donc par trois voies que le corps du ticket nomme lui-même :
`check-suite-wiring.sh` (`unwired=0`), le lancement des suites une par une avec leurs compteurs, et
la CI, qui appelle cette cible **hors du bac à sable** (`ci.yml:85`). Ce n'est pas un repli obtenant
le même accès par un autre outil (mika#3122 règle 1) : la cible reste câblée et reste appelée, par
l'appelant qui en a le droit. **Ce refus se déclare dans le corps de PR et ne se contourne pas.**

## Fire-Disposition

Ce vol livre **cinq détecteurs** : les cinq suites d'assertions. La bibliothèque de sondes n'en est
pas un — elle ne porte aucune assertion, seulement trois fonctions. L'option retenue est
**(a) exception nommée en allowlist**, et **l'ensemble des exceptions est vide**, par mesure :

- **pas d'entrée dans `scripts/suite-wiring.tsv`** — les cinq suites sont câblées directement dans
  les deux cibles du `Makefile`, donc atteintes par la CI ; une entrée les ferait rougir ;
- **pas d'entrée dans `LARGE_FILE_ALLOWLIST`** — hors périmètre, et sans objet : la plus grosse
  suite de ce vol est la famille 13 à 940 lignes, du même ordre que la famille 6 du vol 3 (54 692
  octets, soit 5,2 % du `LARGE_FILE_LIMIT`). AC1 tient avec deux ordres de grandeur de marge ;
- **pas de relève de `AGGREGATE_FROZEN_BYTES`** — l'agrégat ne maigrit pas à ce vol, la scission
  restant additive jusqu'au vol 7. Le contrôle **P** du harnais de mika#3179 reste vert par
  construction ;
- **aucune exception pour un échec d'environnement** — quatre des cinq familles sont relevées à
  `FAIL = 0`. Le remède est à l'étape 1 de l'ordre de commit, pas dans une allowlist.

**Les deux assertions auto-nettoyantes :**

1. `scripts/check-suite-wiring.sh` (lint `suite-wiring-lint`) rougit dès qu'une des cinq suites perd
   son appelant — c'est le détecteur qui tient le câblage d'AC4 dans le temps. L'invariant qui mord
   est **`unwired=0`** ; le relevé du vol 3 après-vol était `|D|=125 |C|=123 |R|=2 unwired=0`, et
   l'ajout de cinq suites câblées le porte à `|D|=130 |C|=128 |R|=2`. Le `|D|` attendu est donné à
   titre indicatif et **se relève, ne se suppose pas** : si la bibliothèque de sondes entre dans le
   dénombrement, `|D|` monte de 1 sans changer `unwired=0`.
2. `scripts/test-check-dispatch-lib-aggregate-frozen.sh` (contrôle **P**) rougit si
   `AGGREGATE_FROZEN_BYTES` cesse d'égaler la taille réelle — donc si un vol fait maigrir l'agrégat
   sans le déclarer.

**Un détecteur qui deviendra vacant, et il est traité au vol 7, pas ici.**
`crates/mika-agent/src/bundled_skills.rs::test_support_dirs_exclude_test_files` affirme
`!skills_dir.join("_shared").join("test-dispatch-lib.sh").exists()`. Le jour où l'agrégat disparaît,
elle passe au vert **par absence de sujet**. Ce vol ne la touche pas : l'agrégat existe encore, et
l'assertion mord encore.

**AC5 est l'armement**, et son instrument est le compteur `FAIL`, pas le code de sortie : sur une
suite déjà rouge, un code non nul serait vrai sans rien prouver.

## Taille estimée

Comptage par **D2(b)** (tranchée par l'opérateur le 2026-11-27, corps de mika#3178) : la borne mord
sur les **lignes écrites**, non sur les 2 761 lignes déplacées mot pour mot par le `sed` de D1(a),
qui ne traversent ni le contexte du modèle ni sa rédaction.

| livrable | lignes de code (hors `docs/`) |
|---|---|
| 5 en-têtes de suite sous `skills/bundled/_shared/tests/` (16+15+20+21+14) — **déjà écrites en `abaa73c9`** | 86 |
| `tests/lib-dispatch-lib-shared-probes-fixtures.sh` — en-tête de la bibliothèque de sondes — **déjà écrit en `abaa73c9`** | 58 |
| `Makefile` — câblage des 5 suites dans `test` et `test-dispatch-lib` + commentaires | 14 |
| lignes déplacées par `sed` (hors borne, D2(b)) | 0 |

Total estimé : 158 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et pourquoi il tient mieux que les trois précédents.** 158 lignes écrites,
loin sous la borne de 429 — et **144 de ces 158 sont déjà sur la branche**. Il ne reste à écrire que
les 14 lignes de câblage. Le coût réel est en **tours**, et il est borné par ce que `abaa73c9`
retire du programme : aucune extraction (5 × 3 gestes économisés), aucun `diff` d'AC3 (5 gestes
économisés, les cinq étant revenus exacts au second groom), aucune re-vérification de frontière.
Reste un `cargo build`, cinq lancements, un câblage, cinq faussages, six portes et la fin de
pipeline — de l'ordre de 25 à 30 tours de production. Le re-ciblage des mentions — le seul travail
de la série dont le coût en tours est mal borné — reste hors de ce vol et porté au vol 7.

**Les sept vols, pour mémoire :** lignes écrites 128 (réel), 155 (réel), 84 (réel), **158
(mesuré)**, ≈84, ≈84, ≈130 — chacune sous la borne de 429 ; lignes déplacées 0, 5 363 (réel),
3 838 (réel), **2 761 (mesuré)**, 3 787, 3 990, 0.

Tracked in: senara-solutions/mika#3178 — le ticket reste ouvert jusqu'au vol 7 et porte les vols 5
à 7 ; la table du § *État de la série* de son corps en est l'ordre de marche, et le vol 7 est le
seul à écrire `Closes #3178`.

## Décisions réservées à l'opérateur, et ce vol n'en pratique aucune moitié

**D3 — les mentions de `test-dispatch-lib.sh` dans les commentaires de `dispatch-lib.sh`** (31 au
relevé des vols 2 et 3). Le corps du ticket la réserve explicitement : « D3 … se tranche par
l'opérateur avant le groom du vol 7 ». Elle **ne porte pas sur le vol 4**, et ce vol ne la tranche
ni ne l'anticipe : `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` doit rester
**vide**, et c'est une ligne de la Definition of Done. Les redire ici ne les rendrait pas plus
tranchées (mika#3122 règle 2).

**Aucun refus de permission n'est contourné par ce plan.** Six formes ont été refusées au cours des
deux grooms précédents et du vol 3. Elles sont nommées parce qu'un refus est un arbitrage de
l'opérateur matérialisé en code (mika#3122 règle 1), et **aucune n'est reprise** :

| geste refusé | conséquence |
|---|---|
| `sed` à plusieurs `-e` | aucune : un `sed -n 'N,Mp'` par commande suffit, et c'est la forme que claude-pilot#325/#328 admettent |
| boucle `for` autour de `sed` | aucune : les cinq extractions sont faites, une par commande |
| `echo "rc=${PIPESTATUS[0]}"` après un tube | aucune : ce vol ne lance plus de `diff` de plage |
| substitution de processus `<(…)`, **deux fois** — `diff - <(…)` puis `diff <(echo) -` | chacune a **arrêté une session**. Seule forme admise pour comparer une plage à un fichier : `sed -n 'N,Mp' <agrégat> \| diff - <fichier>`, un seul tube par commande. Ce vol n'en a pas besoin |
| `make test-dispatch-lib` dans le bac à sable | § *La limite du bac à sable* ci-dessus : déclarée au corps de PR, attestée par la CI, non contournée |

Aucun programme jetable, aucune réécriture d'un geste refusé, aucune variante du même accès n'est
inscrite à ce plan.

## Risques

- **Un compteur de famille 12 qui ne serait pas à `FAIL = 0`.** C'est le seul chiffre non relevé du
  vol, et le seul endroit où une dépendance non nommée pourrait encore se cacher. La garde est la
  méthode de la série : lancer avant de croire, et lire l'absence de bloc `Results:` — ou un
  `rc=127`, une `unbound variable` sous `set -u`, une `command not found` — comme le **nom** de la
  dépendance. Si elle est partagée, elle va dans la bibliothèque de sondes, qui existe déjà et dont
  le nom est déjà admis par claude-pilot#328 : c'est la dépense que ce vol a déjà payée.
- **Une quatrième dépendance, dans une famille autre que la 12.** Quatre des cinq familles sont
  relevées vertes, donc le risque est consommé pour elles. Il reste entier pour les vols 5 et 6, et
  son profil a changé : au vol 4 la définition manquante venait d'une famille **déjà partie au vol
  précédent**, pas d'une famille du même vol. Référence :
  `docs/solutions/test-failures/scinder-une-suite-une-plage-contigue-nest-pas-auto-suffisante-2026-11-27.md`.
- **Un dev-pilot qui re-vérifierait ce qui est acquis.** C'est le défaut qui a tué les deux grooms
  précédents : le second a dépensé 18 tours à confirmer cinq `diff` déjà exacts, puis est mort sur
  un refus. L'ordre de commit est écrit pour l'empêcher — il n'y a ni extraction, ni `diff` de
  plage, ni re-mesure de frontière.
- **La double exécution allonge la CI.** L'agrégat et les suites exécutent les mêmes blocs jusqu'au
  vol 7 : ≈ +18 s par PR pour les 5 familles de ce vol, sur les deux cibles. Chiffré, temporaire, et
  c'est le prix de la voie qui n'exige aucune permission supplémentaire.
- **Les compteurs globaux comptent deux fois pendant la série.** `Results:` de l'agrégat reste
  inchangé et chaque suite rend sa part en plus. Aucun compteur global n'est donc un total de la
  série — ce que le relevé par famille rend explicite.

## Definition of Done

Du vol courant (vol 4) :

- [ ] `cargo build -p mika-cli --bin mika` est passé **avant** le premier lancement de suite.
- [ ] Les 5 suites de la table et `tests/lib-dispatch-lib-shared-probes-fixtures.sh` sont présentes,
      **telles que `abaa73c9` les a produites** — ni re-extraites, ni éditées.
- [ ] Les trois compteurs des 5 suites sont relevés dans le corps de PR, **le compteur de la
      famille 12 inclus**, et `FAIL = 0` sur les cinq. Référence des quatre connus : 44 / 131 /
      139 / 18, soit 332 PASS plus la famille 12.
- [ ] Les familles 13 et 14 `source` la bibliothèque de sondes et passent sans `unbound variable`.
- [ ] 5 faussages d'AC5, chacun mesuré sur le compteur `FAIL` (`n` → `n+1` → `n`), notés dans le
      corps de PR.
- [ ] Les 5 suites sont câblées directement dans `test` et `test-dispatch-lib` ; **aucune entrée**
      dans `scripts/suite-wiring.tsv`.
- [ ] `bash scripts/check-suite-wiring.sh` est vert et rapporte **`unwired=0`** ; le `|D|`/`|C|`
      relevé est inscrit au corps de PR tel que mesuré.
- [ ] `bash scripts/check-secrets.sh --changed origin/main`,
      `bash scripts/canonical-tokens-survey.sh --check`, `make verify-bundled-skills` et
      `bash scripts/verify-pipeline.sh origin/main` sont verts.
- [ ] Le corps de PR déclare le **refus de `make test-dispatch-lib`** dans le bac à sable, et les
      trois voies par lesquelles le câblage s'atteste malgré lui.
- [ ] `AGGREGATE_FROZEN_BYTES` est **inchangée**, et l'agrégat garde ses deux appels de `Makefile`.
- [ ] `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` est **vide** (D3 réservée).
- [ ] La note Compound est enrichie de la troisième occurrence du motif.
- [ ] Le corps de PR déclare qu'AC1, AC3 (clause « blocs retirés ») et AC6 ne seront vrais qu'au
      vol 7, et porte `Refs #3178` — **jamais** `Closes #3178`.

De la série (vols 5 à 7, un vol à la fois) :

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
**à chaque vol**, sur les familles de ce vol. Pour le vol 4, **AC3 est déjà mesuré** : les cinq
`diff` de plage sont revenus exacts au second groom, et la commande qui va au corps de PR est

```
sed -n '<début>,<fin>p' skills/bundled/_shared/test-dispatch-lib.sh \
  | diff - skills/bundled/_shared/tests/test_dispatch_lib_<famille>.sh
```

AC2 reste ouvert sur le seul compteur de la famille 12. AC4 est tenu sous la réserve du § *La limite
du bac à sable* : la cible est câblée et appelée, par un appelant hors du bac. AC1 est mesuré
d'avance mais n'est pleinement vrai qu'au vol 7 pour sa première clause, l'agrégat restant entier
jusque-là ; AC3 (clause « blocs retirés ») et AC6 sont dans le même cas.
