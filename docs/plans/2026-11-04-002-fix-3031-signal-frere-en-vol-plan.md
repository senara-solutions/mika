# mika#3031 — le signal « pilote frère en vol » cesse de lire un fichier qui naît à la mort du pilote

- **Ticket :** senara-solutions/mika#3031
- **Labels :** `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`
- **Dépôt :** mika (un seul)
- **Source :** parking #2983 (commentaire 6560380011, re-mesuré)

---

## 0. Le vol, en une phrase

`_pilot_sibling_pilot_in_flight` cherche la preuve qu'un pilote vole dans un
fichier que le pilote n'écrit qu'**en mourant** ; le vol remplace cette preuve
par celle que le ticket a mesurée — un `.log` **sans** `.stderr` apparié — et la
fenêtre de temps cesse d'être le détecteur pour devenir ce qu'elle est vraiment,
un plafond d'ancienneté contre les `.log` orphelins.

---

## 1. L'état mesuré

Le corps du ticket établit la cause A. Le groom en a mesuré deux autres, sur le
même hôte et avec un vol réel en cours (ce groom lui-même, `4cf19f14`), et la
troisième change la valeur d'un paramètre que le ticket ne proposait pas de
toucher. Les trois sont indépendantes : corriger A seule laisse le signal faux.

### M1 — le prédicat actuel est faux pendant un vol, le candidat est vrai

Mesure prise pendant ce groom, vol `4cf19f14` né à 08:29:27Z, âge 322 s au
moment du tir, fenêtre `2026-11-04T08:27:52Z` → `08:32:52Z` :

| prédicat | sortie | lecture |
|---|---|---|
| `find … -name '*.stderr' -newermt <iso>` | vide | **faux** — alors qu'un pilote vole |
| `find … -name '*.log' -newermt <iso>` | `4cf19f14-…log` | **vrai** |

C'est la cause A du ticket, reproduite à la demande sur un vol de plus de 300 s.

### M2 — `find`, ici, refuse l'offset relatif que le prédicat lui passe

```
$ find /var/log/claude-pilot -maxdepth 1 -name '*.log' -newermt "-7200 seconds"
bfs: error: … -newermt "-7200 seconds"
bfs: error: Invalid timestamp.
Supported timestamp formats are ISO 8601-like, e.g.
  - 2026-11-04T08:31:38Z
```

Le `find` de l'environnement de dispatch est **`bfs`**, et il refuse
`-newermt "-300 seconds"` — la forme exacte de la ligne 1300. Le `2>/dev/null`
avale le refus, la substitution rend la chaîne vide, et `[ -n "" ]` se lit
« aucun frère en vol ». **Le prédicat échoue donc avant même de regarder son
motif** : corriger `*.stderr` → `*.log` sans toucher à la forme du timestamp
laisserait le signal faux à l'identique.

Ce que la mesure n'établit **pas** : que le `find` résolu par le `bash` non
interactif du démon soit le même. Elle n'en a pas besoin — la forme retenue
en M4 est correcte sous les deux, et c'est la seule qui le soit.

### M3 — `@epoch` est accepté par `bfs` et mal interprété

La sortie de secours évidente — passer un epoch plutôt qu'une date — est un
piège, et c'est la mesure qui le dit :

| coupure | attendu | obtenu |
|---|---|---|
| `-newermt "@1762244000"` (= 2026-11-04T07:33:20Z) | rien d'avant 07:33Z | un `.log` du **2026-03-18** |
| `-newermt "@1793780000"` (un an dans le futur) | vide | le `.log` du vol en cours |

Aucune erreur, aucun code de retour non nul : `bfs` **accepte** la forme et lui
donne un sens faux, dans les deux directions. Forme éliminée — et c'est
précisément la classe de panne que ce ticket existe pour éteindre, donc elle
part avec son propre contrôle de bonne foi (T4).

### M4 — la forme ISO 8601 absolue est validée dans les deux sens

| coupure | sortie | lecture |
|---|---|---|
| `-newermt "2026-11-04T08:27:52Z"` | le `.log` du vol en cours, seul | vrai quand il faut |
| `-newermt "2027-01-01T00:00:00Z"` | vide | faux quand il faut |

C'est la seule forme mesurée correcte sous `bfs`, et c'est celle que GNU `find`
documente. `date -u +%Y-%m-%dT%H:%M:%SZ` a **12 précédents** dans
`dispatch-lib.sh` ; seul le `-d "-N seconds"` est nouveau, sur le même binaire.

### M5 — le `.log` est écrit jusqu'à la sortie du pilote

Le ticket mesure la **naissance** des deux fichiers. Le groom y ajoute leur
dernière écriture, qui est ce dont le prédicat dépend :

| vol | mtime final `.log` | mtime `.stderr` | écart |
|---|---|---|---|
| e71ada95 | 20:59:41.230 | 20:59:44.542 | +3,3 s |
| bde03347 | 21:11:26.366 | 21:11:27.014 | +0,6 s |

Le `.log` suit le vol de bout en bout. C'est ce qui rend l'appariement
`.log` sans `.stderr` exact plutôt que seulement plausible.

### M6 — mais son mtime suit les **tours**, pas le temps mural

Intervalles entre deux lignes du `.log`, extraits des horodatages que le pilote
y écrit lui-même, sur cinq vols :

| vol | évts | durée | gap max | cinq plus grands |
|---|---|---|---|---|
| 54a43843 | 12 328 | 3 384 s | **602 s** | 602, 481, **301**, 253, 163 |
| 0069b6f0 | 8 085 | 1 458 s | 227 s | 227, 206, 181, 30, 30 |
| 671abf8e | 5 374 | 445 s | 30 s | 30, 30, 30, 28, 3 |
| fcc5a955 | 4 361 | 657 s | 55 s | 55, 54, 54, 52, 40 |
| 4cf19f14 | 1 903 | 423 s | 105 s | 105, 5, 4, 4, 2 |

**Trois gaps dépassent 300 s sur un seul vol.** Un prédicat « `.log` écrit dans
les 300 dernières secondes » serait donc faux pendant trois fenêtres de ce
vol-là — la panne du ticket, déplacée plutôt qu'éteinte. C'est la mesure qui
interdit de s'arrêter à « remplacer `.stderr` par `.log` ».

Elle a aussi un contrôle d'instrument : le gap de 105 s du vol `4cf19f14` est un
`sleep 105` que le groom a exécuté délibérément, et il apparaît à la seconde.
L'extraction mesure bien ce qu'elle prétend.

### M7 — la fenêtre de 300 s ne tient pas du nombre qu'elle cite

Le commentaire de `_PILOT_DAEMON_SIBLING_WINDOW_SECONDS=300` dit : « Le budget
d'inactivité de claude-pilot (`idle_timeout`, watchdog cpp#170). Ce n'est pas un
nombre inventé ». Le pilote écrit ses garde-fous dans son propre `.log` :

```
[guardrails] maxTurns=200 stallThreshold=5 emptyResponseThreshold=5
  idleTimeout=480.0s rateLimitCeiling=1800.0s toolWaitCeiling=1800.0s
  modelWaitCeiling=900.0s promptCacheDead=3x>50000tok
```

Ni la valeur ni la borne ne sont celles qu'il nomme : `idleTimeout` vaut **480 s**,
et ce qui borne réellement le silence d'un pilote **vivant** est
`toolWaitCeiling` / `rateLimitCeiling` = **1 800 s**. Le gap de 602 s de M6 vit
dans cet intervalle, pas dans celui que le commentaire décrit.

### M8 — 878 `.log` orphelins existent, et un seul a moins de onze jours

L'appariement seul ne suffit pas : un vol tué (SIGKILL, crash) laisse un `.log`
sans `.stderr` pour toujours, et un signal nu resterait vrai à jamais — la
relance gelée définitivement, c'est-à-dire mika#2780 réintroduit par le
correctif de mika#3031.

| grandeur | valeur |
|---|---|
| `.log` | 3 539 |
| `.stderr` | 3 166 |
| `.log` **sans** `.stderr` | **878** |
| orphelins de moins de 1 jour | **1** — le vol en cours |
| orphelin mort le plus récent | **11,5 jours** |

Le fossé est de trois ordres de grandeur : 602 s de silence maximal mesuré d'un
côté, 995 000 s pour l'orphelin mort le plus jeune de l'autre. Un plafond
d'ancienneté se place dedans sans arbitrage délicat.

### M9 — un seul site, un seul `-newermt`

| scan | occurrences aujourd'hui |
|---|---|
| `_pilot_sibling_pilot_in_flight` défini | 1 (l. 1297) ; appelé 1 fois (l. 1324) |
| `-newermt` dans `skills/` + `scripts/` + `Makefile` | 1 (l. 1300) |

Les deux détecteurs d'AC4 et de la cause B atterrissent donc **verts, allowlist
vide** — voir § 4.

Le relais et le `mitmdump` passent **déjà** par ce signal unique, via
`_pilot_daemon_freshness` (l. 1446-1456 et 1594-1609). AC4 ne demande pas de les
y faire passer : il demande d'**épingler** qu'ils y restent.

---

## 2. Le geste

### 2.1 Le détecteur de vol devient l'appariement, et perd sa fenêtre

`_pilot_sibling_pilot_in_flight` (l. 1297-1301) est réécrit. Le signal est :

> il existe un `<id>.log` dont le `<id>.stderr` n'existe pas.

Sans fenêtre, donc **vrai pendant toute la durée du vol**, quel que soit son âge
(M1) et quelle que soit la durée du tour en cours (M6). Le `.stderr` quitte le
prédicat : il naît à la sortie, il ne témoigne d'aucun vol, et sa seule
contribution actuelle est de différer la relance 300 s après chaque vol **fini** —
un pilote sorti n'a plus de tunnel CONNECT à couper.

### 2.2 La fenêtre devient un plafond d'ancienneté, à 1 800 s

`_PILOT_DAEMON_SIBLING_WINDOW_SECONDS` passe de `300` à `1800`, et son
commentaire est réécrit sur ce qu'il borne réellement : non plus « un vol est
récent », mais « un `.log` orphelin cesse de compter ». Sa dérivation, à écrire
dans le commentaire :

- **plancher** — le plus grand plafond de silence du pilote vivant,
  `toolWaitCeiling` = `rateLimitCeiling` = 1 800 s (M7). Trois fois le gap
  maximal mesuré, 602 s (M6).
- **plafond** — très loin sous l'orphelin mort le plus jeune, 11,5 jours (M8) :
  553 fois la fenêtre.

Le renommage de la constante est **hors vol** : son nom reste le même pour que le
diff porte sur le comportement, pas sur un sed.

### 2.3 Le timestamp passe en ISO 8601 absolu

L'instant de coupure est calculé puis passé en littéral :

```sh
_cut=$(date -u -d "-${_PILOT_DAEMON_SIBLING_WINDOW_SECONDS} seconds" \
         +%Y-%m-%dT%H:%M:%SZ 2>/dev/null)
```

Forme validée dans les deux sens (M4), et jamais `@epoch` (M3). `find` reste le
pré-filtre — un seul processus pour 6 705 entrées, contre 878 `stat` forkés si
l'appariement passait en premier — puis l'appariement s'applique aux quelques
candidats que la fenêtre laisse passer.

### 2.4 Un `date` qui échoue est nommé, pas deviné

`date -u -d` n'a aucun précédent dans ce fichier (M10 : seul le format en a 12).
Sur un `date` non-GNU, il échouerait. Les deux replis muets sont tous deux faux :
rendre « frère en vol » gèle la relance pour toujours ; rendre « aucun frère »
réintroduit la panne en silence.

Le prédicat gagne donc une **troisième sortie** : rc 2, « indécidable ». Le
verdicteur la mappe sur `_PILOT_DAEMON_UNDECIDABLE`, jeton qui existe déjà et qui
fait exactement le bon geste — démon conservé, ligne nommée écrite. C'est le
précédent D2 du fichier, dix lignes plus haut : « un `case` sans branche par
défaut rendrait `fresh` EN SILENCE sur une faute de frappe, c'est-à-dire la panne
de ce ticket réintroduite par son propre correctif. »

La co-localisation `_pilot_log_dir; … "$_PILOT_LOG_DIR"` est tenue sur chaque
ligne qui lit la variable — contrainte épinglée par un scan existant de la suite.

### 2.5 Les fixtures de report cessent de fabriquer un vol par un `.stderr`

`_daemon_freshness_probe` (l. 15078) et `_relay_freshness_probe` (l. 15274)
remplacent `touch "$tmp/pilotlogs/frere.stderr"` par l'état mesuré d'un vol : un
`<id>.log` **sans** `<id>.stderr`. La sonde du `mitmdump` gagne trois états de
plus ; celle du relais n'en prend qu'un, par le motif déjà écrit dans le
commentaire de R4 — « il est une propriété du VERDICTEUR PARTAGÉ, déjà tenue par
V4 ».

| état | fixture | verdict attendu | ce qu'il épingle |
|---|---|---|---|
| `deferred` | `.log` sans `.stderr`, mtime maintenant | `stale_deferred` | le report nominal |
| `deferred_old` | idem, mtime à **−600 s** | `stale_deferred` | **AC1** — un vol de plus de 300 s |
| `exited` | `.log` **et** `.stderr`, tous deux frais | `stale` | le `.stderr` ne diffère plus (§ 2.1) |
| `orphan` | `.log` sans `.stderr`, mtime à **−3 jours** | `stale` | le plafond d'ancienneté est armé (M8) |

`deferred_old` est la fixture d'AC1 **et** celle d'AC3 : à −600 s, elle est hors
de l'ancienne fenêtre de 300 s, donc tout prédicat fenêtré à 300 s y rend faux.

---

## 3. Ce que ce vol ne fait pas

- **Il n'installe rien.** Ce que le canari (`scripts/canary-pilot-containment`
  l. 81) et `make install` (`Makefile` l. 46-48) écrivent reste tel quel ; la
  règle « canari seulement sur moteur vide » reste en vigueur.
- **Il n'identifie pas les 4 lanceurs « hors dispatch »** de la table Mesure. Ils
  restent nommés et non identifiés, et le report ne les couvre pas : rien dans ce
  vol ne les fait passer par `_pilot_daemon_freshness`.
- **Il ne renomme pas la constante** (§ 2.2) et ne touche pas aux trois autres
  lancements de pilote (l. 10194, 10399, 10534) : leurs `.stderr` sont des
  `mktemp`, hors de `$_PILOT_LOG_DIR`, donc hors du prédicat.
- **Il ne tranche pas quel `find` le démon résout** (M2) : la forme retenue est
  correcte sous les deux, et un scan refuse le retour de celle qui ne l'est pas.
- **Il n'ouvre aucun ticket de suivi** (gel jusqu'au 01/12). Le reste de la sonde
  AC5 va en commentaire sur mika#3031.

---

## 4. Fire-Disposition

Ce vol livre des détecteurs : les deux scans structurels ci-dessous, plus les
tests du § 5. Disposition retenue : **(a) exception nommée en allowlist**, avec
une allowlist **vide** — et c'est une mesure, pas un espoir (M9).

| détecteur | violations avant | après le geste | allowlist |
|---|---|---|---|
| S1 — un seul prédicat « frère en vol » | 1 (le site légitime) | 1 | `T3031_SIBLING_PREDICATE_ALLOWLIST=()` |
| S2 — aucun `-newermt` à offset relatif | 1 (l. 1300) | 0 | `T3031_RELATIVE_NEWERMT_ALLOWLIST=()` |

Chaque allowlist est un tableau bash vide suivi de son assertion
auto-nettoyante, sur le précédent maison déjà en place dans ce même fichier
(`T2701_SOURCE_SCAN_ALLOWLIST`, l. 14980-14982) :

```sh
T3031_RELATIVE_NEWERMT_ALLOWLIST=()
assert_eq "S2: table d'exceptions du scan -newermt relatif — zero entries" \
    "0" "${#T3031_RELATIVE_NEWERMT_ALLOWLIST[@]}"
```

L'assertion rougit le jour où quelqu'un y ajoute une entrée sans en nommer la
donnée et son ticket — c'est elle qui empêche l'allowlist de devenir celle que
mika#2309 condamne.

Les deux scans sont **comptés, jamais cherchés** : un `grep -c` qui rend `0`
serait aussi ce que rendrait une extraction vide. S1 asserte `1 1` (une
définition, un appel) ; S2 asserte `0`, et porte en regard un compte du
`-newermt` **total** qui doit valoir 1 — sans quoi un scan vert signerait la
disparition de la ligne plutôt que sa correction.

Les deux scans tournent déjà en CI sans câblage nouveau :
`.github/workflows/ci.yml:85` → `make test-dispatch-lib` →
`bash skills/bundled/_shared/test-dispatch-lib.sh`. Aucune entrée
`scripts/suite-wiring.tsv` à créer.

---

## 5. Acceptance criteria

Transcrits du corps de mika#3031, avec le site qui les tient.

- **AC1.** Le signal « frère en vol » est vrai pendant toute la durée d'un
  pilote vivant, quel que soit l'âge du vol. Test dans `test-dispatch-lib.sh` sur
  une fixture qui reproduit l'état mesuré d'un vol : un `.log` qui s'écrit, aucun
  `.stderr`. Le test couvre un vol de plus de 300 s.
  → fixture `deferred_old` (§ 2.5), mtime à −600 s, verdict `stale_deferred`,
  sur les deux sondes (V3b et R3).
- **AC2.** Contrôle négatif dans le même test : aucun pilote vivant et aucun
  fichier récent rend `stale`, et le démon est relancé.
  → `V2`/`R2` existants (répertoire vide) restent verts, plus `orphan` (§ 2.5),
  qui est le contrôle négatif qui compte vraiment : 878 orphelins existent (M8)
  et aucun ne doit différer une relance.
- **AC3.** Les fixtures de V3 et R3 (l. 15078 et l. 15274) ne fabriquent plus un
  vol par un `.stderr` frais. Un test de bonne foi montre que l'ancien prédicat
  rend `stale`, donc faux, sur la fixture d'AC1.
  → § 2.5 pour les fixtures ; `T3` ci-dessous pour la bonne foi.
- **AC4.** Le relais et le helper `mitmdump` passent par le même signal. Un scan
  épinglé par un test refuse un second prédicat « frère en vol ».
  → scan `S1` (§ 4), qui asserte `1 1`.
- **AC5.** Sonde : sur les 10 démarrages du relais qui suivent le deploy, le
  compte de démarrages tombés pendant un vol (même jointure que la table Mesure,
  `fired_at` + 60 s) est écrit sur mika#3031. La cible est 0.
  → § 7. Non livrable par la PR ; la PR écrit la requête et la fenêtre.

### Tests de bonne foi

Trois, et ils sont le cœur de la valeur de ce vol : chacun reproduit à la demande
une des trois causes, donc chacun prouve que les assertions ci-dessus mesurent le
correctif et non le décor.

- **T3** (AC3) — sur la fixture `deferred_old`, le **prédicat d'avant**
  (`-name '*.stderr'`, fenêtre 300 s) rend faux, donc le verdict `stale`. Écrit
  en ligne dans la suite, sans lire une autre révision : le prédicat d'avant tient
  en un `find`, et l'inliner garde le test vert en archive détachée — la raison
  pour laquelle V5 et R4 ne sont, eux, pas assertés en permanence.
- **T4** (M3) — contrôle de forme du timestamp. Sur un `.log` fabriqué à un
  mtime connu, `-newermt "@<epoch>"` et `-newermt "<iso>"` sont tirés sur le
  **même** instant de coupure ; la forme ISO est exacte dans les deux sens, et le
  test asserte que la source n'emploie pas la forme epoch. Sans lui, le prochain
  lecteur « simplifierait » vers `@epoch` et obtiendrait un signal faux sans
  aucune erreur (M3).
- **T5** (M2) — le prédicat tiré avec un `find` qui refuse les offsets relatifs
  reste vrai sur la fixture de vol. Monté par un `find() { … }` de surcharge dans
  le sous-shell de la sonde, qui refuse tout argument `-newermt` commençant par
  `-` ou `+` et délègue le reste. C'est la cause B tenue sans dépendre du `find`
  de l'hôte de CI — sinon le test serait vert sous `bfs` et vide sous GNU.

---

## 6. Definition of Done

- [ ] `_pilot_sibling_pilot_in_flight` réécrit : appariement `.log` sans
      `.stderr`, coupure ISO 8601 absolue, troisième sortie « indécidable ».
- [ ] `_PILOT_DAEMON_SIBLING_WINDOW_SECONDS = 1800`, commentaire réécrit sur ce
      qu'il borne (plafond d'ancienneté) avec sa dérivation et ses deux marges.
- [ ] `_pilot_daemon_freshness` mappe rc 2 sur `_PILOT_DAEMON_UNDECIDABLE`.
- [ ] L'assertion existante « le verdicteur écrit 3 lignes nommées, aucune pour
      `fresh` » reste verte (le mapping n'ajoute aucune ligne nommée).
- [ ] Fixtures V3/R3 remplacées ; états `deferred`, `deferred_old`, `exited`,
      `orphan` sur la sonde `mitmdump`, `deferred_old` sur celle du relais.
- [ ] Scans `S1` et `S2` + leurs deux allowlists vides et leurs assertions
      auto-nettoyantes.
- [ ] Tests de bonne foi `T3`, `T4`, `T5`.
- [ ] `bash -n skills/bundled/_shared/dispatch-lib.sh` (assertion déjà dans la
      suite) et `make test-dispatch-lib` verts.
- [ ] `make verify-bundled-skills`, `cargo clippy`, `cargo fmt --check`,
      `cargo test`, les lints du dépôt et
      `scripts/verify-pipeline.sh origin/main` verts.
- [ ] `docs/solutions/dev-loop/` : une note sur la leçon transférable — *un
      signal de vivacité ne se lit pas dans un fichier écrit à la mort*, plus le
      piège `@epoch` sous `bfs` (M3), qui n'est pas propre à ce site.
- [ ] Corps de PR : la table des trois causes, les mesures M1-M9, et
      `Code review: delegated to mika-qa (mika#2694)`.
- [ ] La fenêtre de la sonde AC5 nommée dans la PR (§ 7).

---

## 7. La sonde AC5, et comment elle se ferme

Elle n'est pas livrable par la PR : elle demande dix démarrages du relais après
le deploy. Au rythme mesuré par le ticket — 34 démarrages du 2026-10-27 au
2026-11-03 — dix tombent en deux à trois jours, donc bien avant le 01/12.

Jointure, identique à celle de la table Mesure du ticket :

- démarrages : `pilot_egress_startup.begin` dans
  `/var/log/mika/pilot-egress-proxy.log`, les dix premiers après l'instant du
  deploy ;
- vols : tâches `long_running:run_claude_pilot*` dont l'intervalle
  [`fired_at` + 60 s, fin] contient le démarrage ;
- la mesure publiée est le **compte** de démarrages tombés pendant un vol, cible
  **0**, écrit en commentaire sur mika#3031.

Un démarrage tombé pendant un vol **après** ce deploy est un reste réel : il va
en commentaire sur mika#3031, pas dans un ticket neuf (gel jusqu'au 01/12).

---

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` (prédicat, constante + commentaires, mapping rc 2) | 52 |
| `skills/bundled/_shared/test-dispatch-lib.sh` (fixtures + 4 états, 5 assertions) | 70 |
| `skills/bundled/_shared/test-dispatch-lib.sh` (scans S1, S2 + allowlists) | 40 |
| `skills/bundled/_shared/test-dispatch-lib.sh` (bonne foi T3, T4, T5) | 58 |

Total estimé : 220 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 220 pour 429, soit 51 % de la borne, et pour
1 000 de seuil par PR. Deux fichiers, tous deux dans `skills/bundled/_shared/`,
aucun crate Rust touché donc aucune compilation dans la boucle de vérification.
Le site de production est **une fonction de cinq lignes** et une constante ; tout
le reste est de la mesure, dont l'exécution est un `bash` de quelques secondes.
Les trois causes partagent un seul site, donc elles ne se découpent pas en vols
sans écrire trois fois le même prédicat.

---

## 9. Ordre de commit

Chaque commit laisse l'arbre vert, et les deux premiers sont l'ordre qui compte :
la mesure avant le remède, pour que le rouge soit constaté plutôt que raconté.

1. `test(3031): les fixtures de report reproduisent l'état mesuré d'un vol` —
   fixtures V3/R3 et les quatre états (§ 2.5). **Rouge attendu**, et c'est la
   panne du ticket reproduite à la demande ; le rouge exact est consigné dans le
   corps de PR.
2. `fix(3031): le signal « frère en vol » lit l'appariement, pas un fichier
   d'après-vol` — § 2.1 à 2.4. Les assertions du commit 1 passent au vert.
3. `test(3031): bonne foi — les trois causes sont reproductibles` — `T3`, `T4`,
   `T5`.
4. `test(3031): deux scans refusent le retour du prédicat et de l'offset relatif`
   — `S1`, `S2` et leurs allowlists vides (§ 4).
5. `docs(solutions): un signal de vivacité ne se lit pas dans un fichier écrit à
   la mort` — la note du § 6, et le piège `@epoch` sous `bfs`.
