# mika#3178 — scinder `test-dispatch-lib.sh` : **vol 3/7**, les familles 6 à 10

- **Issue :** senara-solutions/mika#3178 (`Refs #3174, #3177`)
- **Branche :** `test/3178/dispatch-lib-scinder-test-dispatch-lib`
- **Base :** `8de06d66` (`main`)
- **Vol courant : le vol 3.** Le vol 1 — l'ossature partagée — est mergé (`b60bd55d`, PR #3183,
  128 lignes). Le vol 2 — les familles 1 à 5, la bibliothèque de fixtures et le câblage — est mergé
  (`8de06d66`, PR #3186, 155 lignes écrites, 5 363 déplacées). Ce plan est celui du vol suivant. Il
  succède à `docs/plans/2026-11-27-003-test-3178-…-plan.md`, qui reste en place comme document du
  vol 2 ; la table des 25 familles en est reconduite **après re-mesure sur `8de06d66`**, et quatre
  points en sont **révisés par mesure** (§ *Ce que les mesures du 2026-11-28 ont changé*).
- **Label portant le plafond :** `loop-substrate` (200 tours).

## Condition de départ : les trois conditions sont remplies, et deux le sont par exercice

1. **#3177 mergée ou fermée.** ✅ Mergée en `26140f2b` (relevé du vol 2, inchangé).
2. **Aucune PR ouverte ne modifie l'agrégat.** ✅ Mesuré sur la liste de fichiers des trois PR
   ouvertes au 2026-11-28 — #3185 (`fix(3135)`), #3147 (`chore(deps): bump sqlx`) et #3055
   (`fix(3051)`) : aucune ne touche `skills/bundled/_shared/test-dispatch-lib.sh`, ni le `Makefile`,
   ni `skills/bundled/_shared/tests/`. Le vol 3 n'a donc aucun conflit annoncé — et il ne modifie
   pas l'agrégat, ce qui retire la cause de conflit elle-même.
3. **`senara-solutions/claude-pilot#325` déployée.** ✅ **Exercée pendant ce groom**, pas lue sur un
   statut d'issue : les trois gestes de D1(a) ont produit les cinq familles de ce vol, en-tête par
   `Write`, corps et queue par `sed -n 'N,Mp' … >> …/tests/test_dispatch_lib_<famille>.sh`. La
   permission tient sur les cinq noms de ce vol.

## La base n'a pas bougé sous le plan : les 25 plages tiennent

| mesure | valeur sur `8de06d66` | valeur au plan du vol 2 |
|---|---|---|
| `skills/bundled/_shared/test-dispatch-lib.sh` | **19 080 lignes, 1 041 743 octets** | 19 080 / 1 041 743 — **identique** |
| `AGGREGATE_FROZEN_BYTES` (`scripts/check-dispatch-lib-aggregate-frozen.sh:53`) | `1041743` | `1041743` — **identique** |
| queue de résumé, lignes 19070–19080 | 11 lignes (`# --- Summary ---` … `exit 0`) | 11 lignes |

Les trois commits intercalaires — `b60bd55d` (vol 1), `890f3f5f` (#3182) et `8de06d66` (vol 2) — ne
touchent pas l'agrégat : les vols 1 et 2 n'y retirent rien, puisque la scission est **additive**
jusqu'au vol 7. Le contrôle **P** du harnais de mika#3179 (`AGGREGATE_FROZEN_BYTES` doit **égaler**
la taille réelle) reste donc vert par construction, sans relève de constante.

## Les cinq familles du vol 3, re-mesurées et **toutes lancées**

Les cinq plages sont re-vérifiées ligne à ligne sur `8de06d66` : chaque borne basse tombe sur la
bannière `# ====` du bloc suivant, chaque borne haute juste avant. Les cinq suites ont été
**extraites et lancées pendant ce groom** — c'est la méthode que le corps du ticket impose au vol 3
(« lancer chaque famille extraite avant de la croire »), et elle a nommé deux dépendances que la
relecture n'aurait pas vues.

| # | suite | plage | lignes de la suite | ce que la famille tient réellement | relevé |
|---|---|---|---|---|---|
| 6 | `test_dispatch_lib_groom_callback_1772.sh` | 4990–5987 | 1019 (14 + 998 + 11) | le callback dev-groom honnête (mika#1772) : parseurs, enveloppe de relance, trail, `_escalate_groom` ; le sixième argument de mika#2149 (`.api_error_status`, cpp#54) ; le pilote tué compté par son nom (mika#2029) ; une ligne par callback livré (mika#2121) ; le corps à scanner devenu argument (mika#3118) | **167 PASS / 0 FAIL** |
| 7 | `test_dispatch_lib_gate_arms_3118.sh` | 5988–6679 | 713 (14 + 692 + 11) | les deux bras qui portent la porte et le scan qui refuse qu'un bras la perde (mika#3118), d'abord sur le source puis **rejoués contre la boucle réelle** ; la signature du 2026-07-04 de mika#1772 rejouée de même | **57 PASS / 0 FAIL** |
| 8 | `test_dispatch_lib_output_gate_1996.sh` | 6680–7321 | 663 (14 + 642 + 11) | l'allowlist de dépôts dispatchables en défense de profondeur shell (mika#2062) ; la **porte de sortie non vide** (mika#1996) ; l'ancre élargie au début de COMMANDE de mika#2165 ; les compteurs de relances de mika#2306 et mika#2636 | **65 PASS / 0 FAIL** |
| 9 | `test_dispatch_lib_pr_state_2121.sh` | 7322–8004 | 704 (14 + 683 + 11) | le callback nomme toujours son état de PR (mika#2121 U1) ; le contrat de mika#138 et la lecture ancrée de `_set_up_worktree` ; le motif parti de `_detect_plan_on_branch` (mika#2120) et la délégation de `_extract_plan_path` (mika#2194) ; la règle de confinement du corps de PR (mika#2211) ; le here-string obligatoire (mika#2055/#3130) ; le scratch désigné du pilote (mika#2548) | **109 PASS / 0 FAIL** |
| 10 | `test_dispatch_lib_plan_callout_2120.sh` | 8005–8772 | 789 (14 + 768 + 11) | le second lecteur du callout `Plan` tolérant le préfixe de dépôt (mika#2120) ; les fixtures inline parties au corpus commun (mika#2194) ; `_pilot_log_dir` et son ancre (mika#2165) ; le défaut de flotte armé à 150 (mika#2542) ; l'ancrage obligatoire du `.stderr` (mika#2050) ; `_pilot_max_turns` (mika#2039/#2496) et l'injection de la règle de taille (mika#2693) | **100 PASS / 0 FAIL, SKIPPED: 0** |

**Totaux mesurés du vol :** 3 783 lignes de corps + 5 × 11 lignes de queue = **3 838 lignes
déplacées**, exactement le chiffre de la table du ticket. Le total des cinq suites est de
**3 888 lignes**, soit 3 838 déplacées + 50 lignes d'en-tête de sonde (les en-têtes livrés en
comptent 14, d'où les 70 lignes écrites de la section *Taille estimée*). Somme des PASS : **498**,
`FAIL = 0` sur les cinq.

**AC1 est tenu avec deux ordres de grandeur de marge.** La plus grosse suite produite est la
famille 6 : **54 692 octets**, soit **5,2 %** du `LARGE_FILE_LIMIT` de 1 048 576 — très loin du
plafond de 50 % qu'AC1 pose. Les cinq pèsent 210 699 octets au total. Aucune n'approche les
1 500 lignes du gabarit de la série.

## Ce que les mesures du 2026-11-28 ont changé

### 1. Deux dépendances entre familles, nommées à l'exécution — et c'est la leçon du vol 2 qui paie

Le vol 2 avait trouvé deux dépendances de ce genre. Le vol 3 en trouve deux autres, et **aucune des
deux n'était visible à la relecture** : les deux suites concernées meurent sur une `unbound
variable` sous `set -u`, sans imprimer de bloc `Results:`.

| dépendance | définie à | lue à | symptôme mesuré |
|---|---|---|---|
| `GATE_SRC_3118=$(declare -f _groom_plan_halt_gate)` | l. **5977** (famille 6) | l. 6086, 6088, 6101 (**famille 7**) | `line 110: GATE_SRC_3118: unbound variable`, aucun `Results:` |
| `REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"` | l. **6758** (famille 8) | l. 6759, 6798, 6813 (famille 8), **7862, 7876 (famille 9)**, 9936/9950/9969 (famille 13), 13004 (famille 18), 18352 (famille 24) | `line 554: REPO_ROOT: unbound variable`, aucun `Results:` |

**Remède, et c'est celui que le corps du ticket tranche d'avance** (« un helper utilisé par
plusieurs familles va dans cette bibliothèque ») : les deux lignes vont dans
`skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh`, **écrites** (elles ne sont donc pas
exemptes de la borne), et les lignes 5977 et 6758 **restent en place** dans le corps de leur famille
d'origine, où la ré-assignation est sans effet. C'est ce qui garde le `diff` d'AC3 exact : la suite
reste mot pour mot sa plage, plus l'ossature.

`REPO_ROOT` n'est pas une dépense du seul vol 3 : l'agrégat le dit lui-même à sa ligne 7862
(« Checked here because this suite already resolves REPO_ROOT »), et ses lecteurs couvrent les
familles 13, 18 et 24. Le promouvoir maintenant sert donc aussi les vols 4, 5 et 6, qui n'auront
pas à le redécouvrir par une panne.

Les deux lignes sont placées **après** les trois marqueurs de mika#3178 vol 2 et **avant** les
`assert_*`, dans un bloc de commentaire qui nomme leur ticket, leur ligne d'origine et leurs
lecteurs — même forme que le bloc du vol 2, pour la même raison : le jour où une de ces lignes
change dans l'agrégat, le commentaire dit où regarder.

### 2. Le binaire `mika` : la prescription du vol 2 devient un **fait mesuré**

Le plan du vol 2 portait la cause des 17 échecs de sa famille 5 comme une *prescription* — « statut :
prescription fondée sur le message du test et sur le commentaire CI, à confirmer au premier geste du
vol ». Elle est **confirmée par mesure**, et de deux côtés.

Sur un bac à sable sans `target/debug/mika`, la famille 7 rendait **25 échecs** là où l'agrégat n'en
rend que 24 sur la même plage. L'écart tenait à une assertion :
`B: a double miss is a non-convergence (loop returns non-zero)` (l. 6371), ✓ dans l'agrégat et ✗
dans la suite extraite. Diagnostic : le sous-shell de `_groom_signature_probe_1772` imprime
`rc=1\narch_calls=1` dans l'agrégat — `rc=1` y passe donc **par accident** — et n'imprime rien dans
la suite extraite (`actual:` vide). Cause commune : `command -v mika` introuvable.

Après `cargo build -p mika-cli --bin mika` (1 min 21 s), l'écart **disparaît** et les cinq familles
sont vertes : 167/57/65/109/100, `FAIL = 0` partout. Donc :

- aucun des échecs mesurés n'est une assertion fautive ni un artefact de la scission ;
- `cargo build -p mika-cli --bin mika` est au **premier rang** de l'ordre de commit ;
- AC2 (parité des compteurs, `FAIL = 0` des deux côtés) est **localement mesurable** sur ce vol,
  famille par famille, et non reporté à la CI.

### 3. AC3 est prouvé exact, par une commande qui n'écrit rien

Mesuré sur les cinq familles. Forme générique, et la commande qui va au corps de PR :

```
sed -n '<début>,<fin>p' skills/bundled/_shared/test-dispatch-lib.sh \
  | diff - skills/bundled/_shared/tests/test_dispatch_lib_<famille>.sh
```

Sur la famille 10 (8005–8772), la sortie entière est `0a1,10` et `768a779,789` : les lignes
d'en-tête et les 11 lignes de queue, **et rien d'autre**. Même forme sur les quatre autres. Le pipe
est en lecture seule — il n'écrit aucun fichier, donc la preuve ne demande aucune permission
au-delà de D1(a).

### 4. Un nom de suite qui ne décrit que la seconde moitié de sa plage — assumé, pas corrigé

La famille 8 s'appelle `output_gate_1996`, mais sa plage **ouvre** sur l'allowlist de dépôts
dispatchables de mika#2062 (l. 6681) et n'atteint mika#1996 qu'à la ligne 6841. Les noms de la table
du vol 1 sont **conservés** : ils sont l'ordre de marche des vols 3 à 6, le vol 7 re-ciblera les
mentions sur eux, et les renommer en cours de série coûterait une table de correspondance que rien
ne lit. Ce que porte vraiment chaque suite est dit **dans son en-tête**, en deux à quatre lignes —
c'est précisément la fonction des 14 lignes écrites par suite.

## Ordre de commit

1. **`cargo build -p mika-cli --bin mika`** — avant tout lancement de suite. Mesuré : sans lui, les
   cinq familles rendent des échecs d'environnement, et la famille 7 en rend un de plus que
   l'agrégat sur la même plage (§ *Ce que les mesures ont changé*, point 2).
2. **`lib-dispatch-lib-suite.sh`** — promotion de `GATE_SRC_3118` et de `REPO_ROOT`, avec leur bloc
   de commentaire. Fait **avant** les suites, sinon les familles 7 et 9 meurent sans `Results:`.
3. **Pour chacune des cinq familles, dans l'ordre 6, 7, 8, 9, 10 — exactement trois gestes, et
   jamais un quatrième :**
   a. `Write` de l'en-tête (14 lignes : shebang, titre, deux à quatre lignes de contenu, plage
      d'origine et base, `set -euo pipefail`, `_SUITE_DIR`, `source` de l'ossature) ;
   b. `sed -n '<début>,<fin>p' skills/bundled/_shared/test-dispatch-lib.sh >> <suite>` — le corps ;
   c. `sed -n '19070,19080p' skills/bundled/_shared/test-dispatch-lib.sh >> <suite>` — la queue.
   Puis `bash -n <suite>`, le `diff` d'AC3, et le lancement. **Aucune ligne de corps ni de queue ne
   traverse le contexte du modèle** : c'est ce que D1(a) achète, et c'est ce qui rend le mot pour
   mot vrai par construction plutôt que par relecture.
4. **`Makefile`** — les cinq appels dans la cible `test` et les cinq dans `test-dispatch-lib`,
   chacun précédé d'une ligne de commentaire disant « vol 3/7, familles 6 à 10 » et pourquoi le
   câblage est direct (`check-suite-wiring.sh` ne voit pas un appel fait depuis le corps d'une autre
   suite — mika#2761). **Aucune entrée dans `scripts/suite-wiring.tsv`** : la comparaison du lint
   est bidirectionnelle, une entrée pour une suite que la CI atteint le fait rougir.
5. **AC5** — cinq faussages délibérés, un par suite, chacun mesuré sur le compteur `FAIL`
   (`n` → `n+1` → `n`), puis rétablis.
6. **Les portes** — `bash scripts/check-suite-wiring.sh`,
   `bash scripts/check-secrets.sh --changed origin/main`,
   `bash scripts/canonical-tokens-survey.sh --check`, `make verify-bundled-skills`,
   `make test-dispatch-lib`, `bash scripts/verify-pipeline.sh origin/main`.

**Plancher du vol :** la famille 6 seule, câblée dans les deux cibles, avec la promotion de
`GATE_SRC_3118` — mergeable seule, et déjà mesurée verte (167/0). Les quatre autres s'ajoutent par
le même gabarit, sans interdépendance entre elles une fois l'étape 2 faite.

## Fire-Disposition

Ce vol livre **cinq détecteurs** : les cinq suites d'assertions. L'option retenue est
**(a) exception nommée en allowlist**, et **l'ensemble des exceptions est vide** — par mesure, non
par choix de commodité :

- **pas d'entrée dans `scripts/suite-wiring.tsv`** — les cinq suites sont câblées directement dans
  les deux cibles du `Makefile`, donc atteintes par la CI (`ci.yml:85`) ; une entrée les ferait
  rougir, le lint comparant dans les deux sens ;
- **pas d'entrée dans `LARGE_FILE_ALLOWLIST`** — hors périmètre, et sans objet : la plus grosse
  suite produite pèse **5,2 %** du plafond (54 692 octets sur 1 048 576), mesuré ;
- **pas de relève de `AGGREGATE_FROZEN_BYTES`** — l'agrégat ne maigrit pas à ce vol, et le contrôle
  P reste vert par construction ;
- **aucune exception pour les échecs d'environnement** — il n'y en a plus un seul : les cinq
  familles rendent `FAIL = 0` une fois `target/debug/mika` construit, et c'est le point 2 du §
  *Ce que les mesures ont changé*. Le remède est à l'étape 1 de l'ordre de commit, pas dans une
  allowlist.

**Les deux assertions auto-nettoyantes** qui rougissent quand l'état change sous elles :

1. `scripts/check-suite-wiring.sh` (règle D\C==R, lint `suite-wiring-lint`) rougit dès qu'une des
   cinq suites perd son appelant. C'est le détecteur qui tient le câblage d'AC4 dans le temps.
   Relevé courant sur `8de06d66` : `|D|=120 (floor 85) |C|=118 |R|=2 unwired=0` ; après ce vol,
   `|D|=125 |C|=123 |R|=2 unwired=0`.
2. `scripts/test-check-dispatch-lib-aggregate-frozen.sh` (contrôle **P**) rougit si
   `AGGREGATE_FROZEN_BYTES` cesse d'égaler la taille réelle — donc si un vol fait maigrir l'agrégat
   sans le déclarer. Il est l'assertion auto-nettoyante de la voie « l'agrégat reste entier ».

**Un détecteur qui deviendra vacant, et il est traité au vol 7, pas ici.**
`crates/mika-agent/src/bundled_skills.rs::test_support_dirs_exclude_test_files` affirme
`!skills_dir.join("_shared").join("test-dispatch-lib.sh").exists()`. Le jour où l'agrégat
disparaît, elle passe au vert **par absence de sujet**. Elle est re-ciblée au vol 7 sur un chemin
survivant, dans le commit qui supprime l'agrégat — jamais après. Ce vol ne la touche pas :
l'agrégat existe encore, et l'assertion mord encore.

**AC5 est l'armement**, et son instrument est le compteur `FAIL`, pas le code de sortie : cinq
faussages délibérés, chacun devant faire passer `FAIL` de `n` à `n+1`, puis revenir à `n`. Sur une
suite déjà rouge, un code de sortie non nul serait vrai sans rien prouver — et les cinq de ce vol
sont vertes, donc `n = 0` et le faussage est lisible sans ambiguïté.

## Taille estimée

Comptage par **D2(b)** (tranchée par l'opérateur le 2026-11-27, corps de mika#3178) : la borne mord
sur les **lignes écrites**, non sur les lignes déplacées mot pour mot par le `sed` de D1(a), qui ne
traversent ni le contexte du modèle ni sa rédaction.

| livrable | lignes de code (hors `docs/`) |
|---|---|
| 5 en-têtes de suite sous `skills/bundled/_shared/tests/` (14 lignes chacun) | 70 |
| `skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh` — promotion de `GATE_SRC_3118` et `REPO_ROOT` + bloc de commentaire | 18 |
| `Makefile` — câblage des 5 suites dans `test` et `test-dispatch-lib` + commentaires | 14 |
| lignes déplacées par `sed` (hors borne, D2(b)) | 0 |

Total estimé : 102 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et ce n'est pas qu'une affaire de lignes.** 102 lignes écrites, loin sous
la borne de 429. Le coût réel est en **tours**, et il est borné par construction : 5 familles ×
7 gestes = 35 tours de production, plus ~8 tours de vérification finale et la fin de pipeline. Les
gestes sont identiques d'une famille à l'autre et ne demandent aucune lecture de corps — c'est ce
que D1(a) achète. **Les cinq familles de ce vol ont été extraites et lancées pendant ce groom**,
donc l'estimation n'est pas un pari : elle est le relevé d'un vol déjà effectué une fois, dont il ne
reste à l'implémentation que les mêmes gestes plus le câblage et les faussages d'AC5. Le re-ciblage
des mentions — le seul travail de la série dont le coût en tours est mal borné — reste hors de ce
vol et porté au vol 7.

**Les sept vols, pour mémoire :** lignes écrites 128 (réel), 155 (réel), **102**, ≈84, ≈84, ≈84,
≈130 — chacune sous la borne de 429 ; lignes déplacées 0, 5 363 (réel), **3 838 (mesuré)**, 2 685,
3 787, 3 990, 0.

Tracked in: senara-solutions/mika#3178 — le ticket reste ouvert jusqu'au vol 7 et porte les vols 4
à 7 ; la table du § *État de la série* de son corps en est l'ordre de marche, et le vol 7 est le
seul à écrire `Closes #3178`.

## Décision réservée à l'opérateur, et ce vol n'en pratique aucune moitié

**D3 — les mentions de `test-dispatch-lib.sh` dans les commentaires de `dispatch-lib.sh`** (31 au
relevé du vol 2). Le corps du ticket la réserve explicitement : « D3 … se tranche par l'opérateur
avant le groom du vol 7 ». Elle **ne porte pas sur le vol 3**, et ce vol ne la tranche ni ne
l'anticipe : `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` doit rester **vide**,
et c'est une ligne de la Definition of Done. Les options et leur coût restent celles du plan du
vol 2 ; les redire ici ne les rendrait pas plus tranchées (mika#3122 règle 2).

**Aucun refus de permission n'a été contourné pendant ce groom.** Deux formes ont été refusées, et
elles sont nommées parce qu'un refus est un arbitrage de l'opérateur matérialisé en code
(mika#3122 règle 1) :

| geste refusé | refus | conséquence |
|---|---|---|
| `cd skills/bundled/_shared && timeout 300 bash tests/<suite>` | `no matching policy rule -- denied by default` | aucune : la même mesure est obtenue depuis la racine du worktree, en chemin relatif |
| `timeout 600 bash …/test-dispatch-lib.sh > .pilot-scratch/agg-run.txt 2>&1` | `no matching policy rule -- denied by default` | aucune : la mesure est obtenue par un pipe en lecture seule qui n'écrit rien |

Dans les deux cas le refus **n'empêche rien de ce que le plan demande** : ce n'est pas un repli qui
obtiendrait le même accès par un autre outil, c'est la même mesure prise par un geste déjà admis.
Aucun programme jetable, aucune réécriture d'un geste refusé, aucune variante du même accès n'est
inscrite à ce plan.

## Definition of Done

Du vol courant (vol 3) :

- [ ] `cargo build -p mika-cli --bin mika` est passé **avant** le premier lancement de suite.
- [ ] `GATE_SRC_3118` et `REPO_ROOT` sont dans `tests/lib-dispatch-lib-suite.sh`, avec leur bloc de
      commentaire nommant ticket, ligne d'origine et lecteurs ; les lignes 5977 et 6758 de l'agrégat
      sont **inchangées**.
- [ ] Les 5 suites de la table existent sous `skills/bundled/_shared/tests/`, chacune composée
      uniquement de son en-tête, de sa plage et de la queue — les deux dernières extraites par
      `sed` (D1(a)), et par aucun autre geste.
- [ ] `bash -n` est vert sur chaque suite **avant** son premier lancement.
- [ ] Le `diff` d'AC3 de chaque suite ne montre **que** les deux blocs d'ossature, et les 5
      commandes figurent dans le corps de PR.
- [ ] Les trois compteurs de chaque suite sont relevés dans le corps de PR, et `FAIL = 0` sur les
      cinq. Référence mesurée à ce groom : 167 / 57 / 65 / 109 / 100 PASS, soit **498**.
- [ ] 5 faussages d'AC5, chacun mesuré sur le compteur `FAIL` (`0` → `1` → `0`), notés dans le corps
      de PR.
- [ ] Les 5 suites sont câblées directement dans `test` et `test-dispatch-lib` ; aucune entrée dans
      `scripts/suite-wiring.tsv`.
- [ ] `bash scripts/check-suite-wiring.sh` est vert et rapporte `|D|=125 |C|=123 |R|=2 unwired=0`.
- [ ] `bash scripts/check-secrets.sh --changed origin/main`,
      `bash scripts/canonical-tokens-survey.sh --check`, `make verify-bundled-skills`,
      `make test-dispatch-lib` et `bash scripts/verify-pipeline.sh origin/main` sont verts.
- [ ] `AGGREGATE_FROZEN_BYTES` est **inchangée**, et l'agrégat garde ses deux appels de `Makefile`.
- [ ] `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` est **vide** (D3 réservée).
- [ ] Le corps de PR déclare qu'AC1, AC3 (clause « blocs retirés ») et AC6 ne seront vrais qu'au
      vol 7, et porte `Refs #3178` — **jamais** `Closes #3178`.

De la série (vols 4 à 7, un vol à la fois) :

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
**à chaque vol**, sur les familles de ce vol — et pour le vol 3, AC2 et AC3 sont déjà mesurés au §
*Les cinq familles du vol 3* et au point 3 du § *Ce que les mesures ont changé*. AC1 est **mesuré
d'avance** sur les cinq suites de ce vol (5,2 % du plafond au maximum) mais n'est pleinement vrai
qu'au vol 7 pour sa première clause, l'agrégat restant entier jusque-là ; AC3 (clause « blocs
retirés ») et AC6 sont dans le même cas. AC5 est mesuré sur le compteur `FAIL` et non sur le code de
sortie, pour la raison donnée au § *Fire-Disposition*.

## Risques

- **Une frontière tombée dans un here-document.** Seule classe de défaut qu'un découpage par
  numéros de ligne produit silencieusement. **Écartée par mesure pour ce vol** : les cinq familles
  ont passé `bash -n` du premier coup pendant ce groom, et les cinq `diff` d'AC3 ne montrent que
  l'ossature. Le risque résiduel ne porte que sur une re-extraction fautive à l'implémentation, que
  les mêmes deux gardes rattrapent.
- **Une troisième dépendance entre familles.** Deux ont été trouvées, toutes deux à l'exécution. La
  garde est la méthode elle-même : lancer chaque famille avant de la croire, et lire l'absence de
  bloc `Results:` comme le nom de la dépendance. Les cinq familles de ce vol sont désormais
  **lancées et vertes**, donc le risque est consommé pour le vol 3 ; il reste entier pour les
  vols 4 à 6. Référence :
  `docs/solutions/test-failures/scinder-une-suite-une-plage-contigue-nest-pas-auto-suffisante-2026-11-27.md`.
- **Un helper partagé demandant une cible que la permission n'admet pas.** Les deux promotions de ce
  vol vont dans `lib-dispatch-lib-suite.sh`, écrit par `Write` et non par `sed` — hors du périmètre
  de claude-pilot#325/#328, qui ne régit que les deux noms en `>>`. Si un vol ultérieur avait besoin
  d'un `sed` vers un troisième nom, c'est **un arrêt à déclarer, pas un geste à contourner** (corps
  du ticket, et mika#3122 règle 1).
- **La double exécution allonge la CI.** L'agrégat et les suites exécutent les mêmes blocs jusqu'au
  vol 7 : ≈ +25 s par PR pour les 5 familles de ce vol (l'agrégat entier prend 1 min 12 s pour 25
  familles), sur les deux cibles `test` et `test-dispatch-lib`. Chiffré, temporaire, et c'est le
  prix de la voie qui n'exige aucune permission supplémentaire.
- **Les compteurs globaux comptent deux fois pendant la série.** `Results:` de l'agrégat reste
  inchangé et chaque suite rend sa part en plus. Aucun compteur global n'est donc un total de la
  série — ce que le relevé par famille rend explicite.
- **Un en-tête mal rédigé.** Les 14 lignes de chaque en-tête sont les seules que le modèle écrive
  par suite. Une erreur y est rattrapée par `bash -n`, par le `diff` d'AC3 (qui montrerait un
  en-tête plus long que prévu) et par l'exécution. Le gabarit a été exercé cinq fois pendant ce
  groom.
