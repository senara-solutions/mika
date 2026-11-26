# mika#3178 — scinder `test-dispatch-lib.sh` en suites par familles

- **Issue :** senara-solutions/mika#3178 (`Refs #3174, #3177`)
- **Branche :** `test/3178/dispatch-lib-scinder-test-dispatch-lib`
- **Base :** `26140f2b` (`main`)
- **Condition de départ :** **remplie.** #3177 est mergée (`26140f2b`), et aucune des deux PR
  ouvertes restantes (#3147, #3055) ne touche `skills/bundled/_shared/test-dispatch-lib.sh`.
- **Label portant le plafond :** `loop-substrate` (200 tours).

---

## Le fait mesuré, re-mesuré sur la branche

| mesure | valeur |
|---|---|
| `skills/bundled/_shared/test-dispatch-lib.sh` | 1 041 079 octets, 19 070 lignes |
| `LARGE_FILE_LIMIT` (`scripts/check-secrets.sh`) | 1 048 576 octets |
| marge restante | 7 497 octets |
| densité mesurée | 54,6 octets/ligne → ~137 lignes de marge |
| bannières de section de premier niveau | 46 |
| exécution dans le bac à sable de dispatch | `Results: 2036 passed, 85 failed, SKIPPED: 5` |
| plus grosse suite existante sous `tests/` | `test_egress_push_bound.sh`, 1 479 lignes |

Les 85 échecs sont **induits par l'environnement** et préexistent au déplacement : ils sortent
sur `fatal: Not a valid object name origin/main` dans les dépôts-fixtures du bac à sable. L'étape
`Dispatch-lib assertion suite (mika#1772)` de `.github/workflows/ci.yml:84-85` passe sur `main`.
Conséquence directe sur AC2, traitée au § *Preuve de non-perte*.

## Ce que la mesure a fermé

Le corps du ticket nomme quatre points « à confirmer, pas à supposer ». Les quatre sont
confirmés par lecture du code, pas par raisonnement :

1. **Le survey d'inventaire canonique exclut les nouvelles suites.** `collect_files()` de
   `scripts/canonical-tokens-survey.sh` écarte `*/tests/*`, `test-*` et `test_*` sous
   `skills/bundled`. Une suite nommée `skills/bundled/_shared/tests/test_dispatch_lib_*.sh`
   tombe dans **deux** de ces trois exclusions.
2. **Le seeding build-time ne les installe pas.** `collect_support_dir_files()` de
   `crates/mika-agent/build_support/bundled_skills_discover.rs` saute toute entrée dont le nom
   contient `test` en minuscules — ce qui écarte le répertoire `tests/` entier, donc son contenu.
3. **Les deux fuites d'état inter-familles suspectées n'en sont pas.** Le `PATH="$base_dir/empty-path"`
   de la ligne 18184 vit dans un sous-shell `( … )` ouvert en 18179 et refermé en 18189 ; les deux
   redéfinitions de `mika()` (917 et 938) sont annulées par `unset -f mika mika_stdin_capture` en 942.
   Aucune des deux ne traverse une frontière de famille. La preuve mécanique reste exigée au § suivant :
   796 affectations de premier niveau ne s'énumèrent pas, elles se mesurent.
4. **L'appelant de `check-pilot-turn-ceiling-labels.sh` est déjà double.** Le corps de la suite
   l'appelle en 8713–8717, mais `Makefile:186` l'appelle **directement**. La dernière contrainte de
   périmètre du ticket est donc déjà satisfaite structurellement : rien à créer, seul le bloc
   déplace son appel avec lui.

Deux conséquences de code s'ajoutent, mesurées et non évoquées par le ticket :

- `crates/mika-agent/src/bundled_skills.rs` (~3051–3064) porte
  `test_support_dirs_exclude_test_files`, qui affirme
  `!skills_dir.join("_shared").join("test-dispatch-lib.sh").exists()`. Si l'agrégat disparaît,
  **cette assertion devient vacante et passe au vert par absence de sujet.** Elle doit être
  re-ciblée sur un chemin survivant.
- `scripts/check-suite-wiring.sh:49` et `scripts/test-check-suite-wiring.sh:267` nomment en prose
  l'agrégat comme « la seule suite atteinte depuis le corps d'une autre suite ». Après le
  déplacement, la phrase désigne un fichier qui n'existe plus.

## Découpage retenu

**Règle de gabarit.** Aucune nouvelle suite ne dépasse **1 500 lignes**, le gabarit de la plus
grosse suite existante du répertoire. AC1 n'exige que 50 % de `LARGE_FILE_LIMIT` (524 288 octets
≈ 9 600 lignes) ; la règle retenue est 7 fois plus serrée, parce que le motif du ticket est la
**lisibilité en revue**, pas le plafond. La plus grosse suite produite pèse ~70 Ko, soit **13 %**
du plafond.

**Règle de frontière.** Une famille est un ensemble de **bannières de premier niveau contiguës**.
Une famille n'est jamais coupée entre deux suites. Les plages ci-dessous commencent sur la **ligne
de règle** `# ====…` qui ouvre la bannière, pour que la bannière voyage avec sa famille.

| # | suite (`skills/bundled/_shared/tests/…`) | plage dans l'agrégat | lignes |
|---|---|---|---|
| 1 | `test_dispatch_lib_iterate_1271.sh` | 106–1085 | 980 |
| 2 | `test_dispatch_lib_escalate_1271.sh` | 1086–1920 | 835 |
| 3 | `test_dispatch_lib_groom_gate_2811.sh` | 1921–2805 | 885 |
| 4 | `test_dispatch_lib_push_divergence_1364.sh` | 2806–3911 | 1106 |
| 5 | `test_dispatch_lib_preflight_1472.sh` | 3912–4979 | 1068 |
| 6 | `test_dispatch_lib_groom_callback_1772.sh` | 4980–5977 | 998 |
| 7 | `test_dispatch_lib_gate_arms_3118.sh` | 5978–6669 | 692 |
| 8 | `test_dispatch_lib_output_gate_1996.sh` | 6670–7311 | 642 |
| 9 | `test_dispatch_lib_pr_state_2121.sh` | 7312–7994 | 683 |
| 10 | `test_dispatch_lib_plan_callout_2120.sh` | 7995–8762 | 768 |
| 11 | `test_dispatch_lib_arch_passes.sh` | 8763–9143 | 381 |
| 12 | `test_dispatch_lib_worktree_removal_1943.sh` | 9144–9392 | 249 |
| 13 | `test_dispatch_lib_fire_disposition.sh` | 9393–10301 | 909 |
| 14 | `test_dispatch_lib_halt_notes.sh` | 10302–11194 | 893 |
| 15 | `test_dispatch_lib_marker_position_2590.sh` | 11195–11392 | 198 |
| 16 | `test_dispatch_lib_token_parity_2608.sh` | 11393–12239 | 847 |
| 17 | `test_dispatch_lib_push_remote_2626.sh` | 12240–12965 | 726 |
| 18 | `test_dispatch_lib_plan_size_gate_2636.sh` | 12966–13900 | 935 |
| 19 | `test_dispatch_lib_per_flight_bound_2693.sh` | 13901–14608 | 708 |
| 20 | `test_dispatch_lib_arch_disposition_2641.sh` | 14609–15124 | 516 |
| 21 | `test_dispatch_lib_plan_size_estimate_2701.sh` | 15125–16413 | 1289 |
| 22 | `test_dispatch_lib_revise_termination_2077.sh` | 16414–16883 | 470 |
| 23 | `test_dispatch_lib_branch_resume_2976.sh` | 16884–17691 | 808 |
| 24 | `test_dispatch_lib_unparsed_relaunch_2996.sh` | 17692–18741 | 1050 |
| 25 | `test_dispatch_lib_launch_gate_3121.sh` | 18742–19070 | 329 |

25 suites, 18 965 lignes déplacées, maximum 1 289 lignes. Les plages sont exhaustives et
disjointes : leur somme est exactement 19 070 − 105, les 105 premières lignes étant le prélude
qui devient la bibliothèque partagée.

Les bannières qui donnent les noms sont celles des lignes 709, 1005, 1087, 1922, 2807, 3913,
4981, 5644, 5979, 6094, 6235, 6671, 6831, 7313, 7793, 7869, 7996, 8764, 8848, 8963, 9114, 9145,
9394, 9769, 10303, 10828, 10985, 11167, 11196, 11302, 11394, 11800, 11937, 12181, 12241, 12967,
13902, 14610, 15126, 16415, 16885, 17693, 17829 et 18743.

## L'ossature partagée

**Fichier :** `skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh`.

Il n'est **pas** une suite : son nom ne porte aucun des cinq préfixes de `SUITE_PREFIX_RE`
(`test-`, `test_`, `check-`, `verify-`, `measure-`), exactement comme ses deux voisins
`lib-mika-bin.sh` et `lib-fake-egress-relay.sh`. Il n'a donc rien à déclarer dans
`scripts/suite-wiring.tsv`.

Ce qu'il porte, extrait des lignes 1–105 et de la queue de l'agrégat :

| contenu | origine |
|---|---|
| `set -euo pipefail` | ligne 16 |
| les 5 surcharges `GIT_CONFIG_*` d'herméticité (gpgsign, tag.gpgsign, init.defaultBranch, identité du committer) | lignes 29–39 |
| `SCRIPT_DIR` | ligne 41, **ré-ancré** — voir la décision ci-dessous |
| `DISPATCH_LIB="$SCRIPT_DIR/dispatch-lib.sh"` | ligne 42, orthographe inchangée |
| `PASS=0`, `FAIL=0` | lignes 44–45 |
| `SKIPPED=0` | ligne 48 (mika#2149) |
| `source "$SCRIPT_DIR/tests/lib-mika-bin.sh"` | ligne ~67, orthographe inchangée |
| `assert_eq()`, `assert_contains()`, `assert_not_contains()` | lignes 68, 81, 94 |
| `_suite_summary_and_exit()` | queue : `Results: …`, `if [ "$FAIL" -gt 0 ]; then exit 1; fi`, `exit 0` |

**Décision de conception, et c'est celle qui rend AC3 littéralement vraie.** La bibliothèque
calcule `SCRIPT_DIR` comme le répertoire `_shared/`, **pas** `tests/` :

```bash
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
```

`${BASH_SOURCE[0]}` désigne la bibliothèque elle-même, donc `dirname` donne `tests/` et `/..`
donne `_shared/`. Raison : l'agrégat contient **26 chemins relatifs à `$SCRIPT_DIR`** ancrés sur
`_shared/`, répartis dans 11 familles différentes — `PROMPT_FILE="$SCRIPT_DIR/../self-dev/system_prompt.md"`
(215), `"$SCRIPT_DIR/../../../.."` (672), `GROOMING_FIXTURE_DIR` (1754), `REPO_ROOT` (6748),
`M2194_CORPUS` (11322), `M2608_TOKENS_TSV` (12199), `t2636_pf` (13665), `t2693_pf` (14293),
`M2641_ANCHOR_MANIFEST` (15003), `M3_CORPUS` (16275), `t3122_pf` (18660). Ré-ancrer `SCRIPT_DIR`
sur `_shared/` les laisse **octet pour octet identiques** : aucune de ces 26 lignes n'est réécrite,
et le `diff` d'AC3 ne montre que des lignes d'ossature. L'alternative — un `SCRIPT_DIR` pointant
sur `tests/` — exigerait de réécrire 26 chemins, ce que le ticket interdit (« les blocs sont
déplacés mot pour mot »).

**La seule exception connue** est `test_callout_plan_identity_3174.sh`, qui vit déjà sous `tests/`
et épelle `DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"`. Il n'est pas migré vers la bibliothèque
par ce plan : il est hors périmètre et sa migration serait un changement de code sans défaut à
l'appui.

**Ossature par suite**, ~12 lignes, et c'est l'exclusion du `diff` d'AC3 :

```bash
#!/bin/bash
# <titre de la famille> — extrait de test-dispatch-lib.sh par mika#3178.
# <deux à quatre lignes disant ce que la famille tient>
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"

# <corps : la plage, mot pour mot>

_suite_summary_and_exit
```

**Règle de placement des helpers**, telle que le ticket la pose : un helper appelé par plusieurs
familles va dans la bibliothèque ; un helper propre à une famille reste avec elle. La mesure dit
que le second cas est le seul cas réel : hors les trois `assert_*`, **aucune** des 208 fonctions
de l'agrégat n'est appelée depuis plus d'une famille. Les candidates vérifiées — `mika()` /
`mika_stdin_capture` (917, 938, annulées en 942), `_fixture_setup` / `_fixture_cleanup` /
`_assert_fixture_is_local` (2816, 2832, 2837), `_t2144_scaffold_decl_scan` (3843),
`_test_policy_deny_check` (4207), `_parse_prompt` (4527), `_fip_probe` (4566),
`_iterate_*_probe` (5063, 5079), `_classify_probe` (5107), `_pfr_probe_1772` (5238),
`_t3030_seed_and_read` (18172) — sont toutes confinées à une plage unique. La bibliothèque reste
donc minimale : 95 lignes, et pas une fonction de fixture.

## Preuve de non-perte

AC2 demande l'égalité des sommes PASS et SKIPPED, et `FAIL = 0` des deux côtés. Deux
renforcements sont nécessaires pour que la preuve morde réellement.

**R1 — la parité se mesure PAR FAMILLE, pas seulement en somme.** Une somme globale masque une
paire compensatoire : une assertion perdue dans la famille 7 et une double-comptée dans la 19
laissent le total inchangé. Pour chaque suite `k`, les trois compteurs rendus doivent égaler les
trois compteurs que la **même plage** produit dans l'agrégat. Le relevé par plage s'obtient en
exécutant l'agrégat et en découpant sa sortie sur les lignes de bannière, avant tout déplacement :
c'est la mesure de référence, prise une fois, sur un seul commit.

Cette mesure par famille est aussi la **preuve mécanique de l'absence de fuite d'état** que le
point 3 du § *Ce que la mesure a fermé* laisse ouverte. Elle ne demande pas d'énumérer les 796
affectations de premier niveau : `set -u` est vivant, donc une variable amont non résolue est une
erreur dure et non une dégradation silencieuse ; et une fonction amont manquante fait rougir
l'`assert_*` qui compare contre une capture vide, ce qui déplace `FAIL`. Toute fuite se lit dans
l'écart de compteurs d'une famille.

**R2 — `FAIL = 0` n'est attestable que là où l'agrégat est vert.** Mesuré aujourd'hui dans le bac
à sable de dispatch : `Results: 2036 passed, 85 failed, SKIPPED: 5`. La clause `FAIL = 0 des deux
côtés` d'AC2 est donc **inatteignable dans l'environnement du pilote**, et aucune réécriture
d'assertion ne peut la rendre atteignable — ce serait exactement ce que le ticket interdit. Le
contrat de vérification retenu :

- **l'égalité des comptes** (PASS, SKIPPED, et FAIL par famille) se mesure dans **un seul**
  environnement, des deux côtés : c'est ce que « prouvée sur le même commit » veut dire, et elle
  tient dans le bac à sable malgré les 85 échecs, puisque les 85 se retrouvent à l'identique du
  côté des suites ;
- **`FAIL = 0`** est attesté par l'étape `Dispatch-lib assertion suite (mika#1772)` de
  `ci.yml:84-85`, verte sur `main` et exécutant `make test-dispatch-lib` ;
- le corps de PR cite **les deux mesures séparément**, avec leur environnement nommé. Un unique
  compte sans son environnement serait un chiffre qui ne dit pas d'où il vient.

**R3 — AC5, rouge avant.** Une assertion par suite est faussée, la suite doit rendre un code non
nul, puis l'assertion est rétablie. 25 suites, donc 25 faussages. L'ordre est noté dans le corps
de PR, nom de suite et numéro de ligne de l'assertion faussée. C'est le seul instrument qui
prouve que les 25 suites sont **armées** et pas seulement présentes : une suite dont le corps ne
s'exécuterait pas rendrait 0 sur une assertion fausse.

## Câblage

- Chaque suite est câblée **directement** dans les cibles `test` et `test-dispatch-lib` du
  `Makefile` — 2 lignes `@bash skills/bundled/_shared/tests/test_dispatch_lib_<f>.sh` par suite,
  50 lignes au total. Le ticket dit pourquoi : `check-suite-wiring.sh` ne voit pas un appel fait
  depuis le corps d'une autre suite, et sous-approximer la joignabilité donne des faux rouges,
  jamais des faux verts (mika#2761).
- L'appel `@bash skills/bundled/_shared/test-dispatch-lib.sh` de `Makefile:144` est retiré, ainsi
  que l'entrée correspondante de la cible `test-dispatch-lib` (`Makefile:200`).
- `.github/workflows/ci.yml:84-85` nomme `make test-dispatch-lib` : la joignabilité des 25 suites
  passe par le hop H2 (le workflow nomme la cible, le lint suit la recette, ses prérequis et la
  récursion `$(MAKE)`). **Aucune entrée dans `scripts/suite-wiring.tsv`** : les 25 suites sont
  atteintes, et le lint est bidirectionnel — une entrée pour une suite que la CI atteint le fait
  rougir.
- `# min-population: 85` de `suite-wiring.tsv` : la population passe de 94 à 118 suites préfixées.
  Le plancher est franchi par le haut, aucune modification requise.

## Références (AC6)

`git grep -n "test-dispatch-lib"` hors `docs/plans` et `docs/brainstorms` : **78 fichiers, 170
lignes**. La règle de tri retenue, et le plan la pose explicitement parce qu'AC6 ne la pose
pas :

> Une mention se re-cible quand elle dit à un lecteur **d'aujourd'hui** où regarder. Un relevé
> d'incident daté sous `docs/solutions/**` reste écrit tel quel : il décrit ce qui était vrai à sa
> date, et le réécrire falsifierait un témoignage.

Cette règle suit l'énumération du ticket, qui nomme `crates/mika-agent`, `crates/mika-cli`, les
README et TSV de `crates/mika-agent/tests/fixtures/`, le TSV d'inventaire canonique et plusieurs
scripts de `scripts/` — et **ne nomme pas** `docs/solutions`. Les **32 fichiers** de
`docs/solutions/**` (**61 lignes**) sont donc hors re-ciblage, et le corps de PR le dit avec sa
raison.

Se re-ciblent **95 lignes hors `docs/`** (les 4 mentions internes à l'agrégat partent avec lui) et
**10 lignes sous `docs/` hors `docs/solutions`** — `docs/architecture/dispatch-lib-migration.md`
(4), `docs/operator/environment-variables.md` (5), `docs/agent-internals/exec-handlers.md` (1) :
ce sont des références vivantes, pas des relevés datés, donc la règle ci-dessus les prend.

| site | lignes | nature |
|---|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` | 31 | commentaires nommant un bloc précis (`S4`, `S11`, `T5`, `T6 drift guard`, `T12j`, `case B`, `rescue-block extraction`, …) |
| `crates/mika-agent/src/**` (8 fichiers) | 13 | commentaires de parité bash↔Rust, dont `bundled_skills.rs` (2) |
| `crates/mika-agent/tests/**` (4 fichiers) | 5 | commentaires de parité et d'ancrage |
| `crates/mika-agent/tests/fixtures/**` (4 fichiers) | 8 | README et `expectations.tsv` |
| `crates/mika-cli/src/commands/arch_disposition.rs` | 1 | commentaire de parité |
| `scripts/` (9 fichiers) | 11 | `canonical-tokens.tsv` (2), `canonical-tokens-survey.sh` (2), `check-suite-wiring.sh:49`, `test-check-suite-wiring.sh:267`, `verify-pipeline.sh`, `verify-no-sigpipe-grep.sh` + sa suite, `test-guard-shared-checkout.sh`, `test-shell-exec-guard.sh` |
| `skills/bundled/_shared/tests/**` (10 fichiers) | 14 | commentaires de voisinage |
| `skills/bundled/mika-arch-*/system_prompt.md` | 4 | prose d'orientation de l'architecte |
| `.github/workflows/ci.yml`, `Makefile`, `crates/mika-agent/CLAUDE.md` | 8 | câblage et index |

**Hypothèse déclarée, et elle demande à être lue.** Le périmètre du ticket exclut « tout
changement de `dispatch-lib.sh` », et AC6 exige que plus aucune mention ne désigne un bloc
déplacé. Les 31 commentaires de `dispatch-lib.sh` désignent tous un bloc qui bouge : les deux
clauses se croisent. Le plan tranche par la lecture la plus étroite possible — **l'exclusion
protège le comportement, pas les commentaires** — et la rend mécaniquement vérifiable :

> `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` ne montre que des lignes
> commençant par `#` (ou des commentaires de fin de ligne). Aucune ligne exécutable ne change.
> Le corps de PR cite ce `diff`.

Si l'opérateur lit l'exclusion littéralement, ces 31 occurrences sortent du périmètre et AC6 est
borné aux 29 autres, avec un ticket de suivi. Le plan ne pratique pas ce second choix ; il le
signale, parce qu'il est bon marché à trancher et qu'il ne bloque rien d'autre.

---

## Décisions réservées à l'opérateur

Les deux décisions ci-dessous sont chiffrées et **aucune n'est pratiquée** (mika#3122). Le travail
préparatoire est complet : le découpage est exhaustif, l'ossature est inventoriée ligne par ligne,
le câblage est tracé, les quatre « à confirmer » sont confirmés, et les deux fuites suspectées sont
mesurées comme contenues. Ce qui reste n'est pas une incertitude d'analyse.

### D1 — l'instrument par lequel 18 965 lignes se déplacent mot pour mot

**Le refus.** Le geste naturel d'un déplacement mot pour mot est une extraction de plage vers un
fichier. Dans une session dispatchée, la politique de permission le refuse :

- une redirection `>` ou `>>` dont la cible est un fichier de travail du worktree est **refusée**
  (recouvrable) ; la seule forme admise est `git show <ref>:<chemin> > .pilot-scratch/<chemin>`,
  seule sur sa ligne, et elle ne produit que des copies de fichier **entier** ;
- `sed -i` est refusé ; `bash -c`, `sh -c` et `eval` sont refusés et **terminaux** ;
- les seuls instruments d'écriture admis sont les outils `Write` et `Edit`.

Ce refus est un arbitrage de l'opérateur matérialisé en code. Le plan **ne prévoit donc aucun
repli qui obtiendrait le même effet par un autre outil** : pas de programme de découpage jetable,
pas de `csplit`/`split`/`tee` substitué à la redirection, pas de « variante » du même accès. La
décision est nommée et laissée entière.

**Ce que le refus empêche, chiffré.** Via `Write`/`Edit`, chaque ligne déplacée traverse le
contexte du modèle au moins trois fois : une lecture dans l'agrégat, une écriture dans la suite
d'arrivée, une écriture comme `old_string` de l'`Edit` qui la retire de l'agrégat. Enveloppe
mesurée dans ce dépôt : le plus gros fichier shell qu'un pilote y ait produit en un commit est de
**944 lignes** (PR #2648), suivantes 747, 741, 702. Au-delà de ~1 500 lignes de contenu de fichier
accumulées, la compaction de contexte intervient en cours de transfert — sur une exigence « mot
pour mot », c'est la classe de défaut la plus coûteuse à détecter.

| option | coût | ce qu'elle produit |
|---|---|---|
| **D1(a)** — l'opérateur ouvre dans la politique `claude-pilot` une permission nommée et bornée pour l'extraction de plage vers `skills/bundled/_shared/tests/` | 1 geste hors de ce dépôt (la politique n'y vit pas : son texte injecté est `_PILOT_SCRATCH_RULE`, `dispatch-lib.sh:5200-5235`) | le déplacement devient 1 à 2 vols ; AC3 est prouvé par construction |
| **D1(b)** — déplacement via `Write`/`Edit` uniquement, à ~900–1 300 lignes par vol | **15 à 21 vols**, chacun mergeable, AC1/AC3/AC6 satisfaits seulement au dernier | aucun geste hors dépôt ; risque de dérive mot-pour-mot sur chaque vol, couvert par la parité par famille (R1) |
| **D1(c)** — un instrument de découpage versionné dans le dépôt, appelé par le pilote | 1 vol, ~80 lignes | **c'est la forme que mika#3122 règle 1 interdit d'inscrire au plan** : elle obtient par un outil suivi l'effet que le refus empêche. Nommée ici pour être écartée, pas pour être choisie. |
| **D1(d)** — rouvrir le périmètre exclu : entrée `LARGE_FILE_ALLOWLIST` ou plafond par chemin | 1 vol, ~20 lignes | débloque la boucle immédiatement et ne scinde rien. Le ticket argumente contre (« une suite d'un mégaoctet reste illisible en revue ») ; c'est l'alternative permanente de l'opérateur, pas celle du plan |

### D2 — l'unité dans laquelle un déplacement se compte contre la borne par vol

**Le fait arithmétique.** La borne par vol de ce ticket vaut 429 lignes estimées. **14 des 25
familles dépassent 429 lignes à elles seules** (la plus grosse, la 21, en fait 1 289). Comptées à
leur valeur faciale, les 18 965 lignes déplacées valent **46 vols**, et chaque vol porte alors une
**fraction** de famille : l'agrégat et la suite d'arrivée tiennent chacun une moitié du bloc
pendant toute la série. Conséquence : **45 des 46 vols ne peuvent satisfaire ni AC1 (l'agrégat
existe encore), ni AC3 (la concaténation de la famille est incomplète), ni AC6 (les mentions
désignent encore des blocs présents).** Un découpage dont 45 vols sur 46 échouent aux critères du
ticket n'est pas un découpage.

**Pourquoi le plan ne tranche pas.** La borne a un site de vérité unique,
`_plan_size_per_flight_bound` dans `skills/bundled/_shared/dispatch-lib.sh`, et sa justification
(`docs/solutions/dev-loop/une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite.md`)
calibre le facteur ×3 estimé→livré sur du code **écrit**. Un déplacement mot pour mot n'a pas de
coût de conception, et le document mesure précisément l'inverse sur ce fichier : PR #2648, 205
lignes estimées pour 944 livrées, ×4,6. Savoir si une relocalisation se compte dans l'unité de la
borne, et à quel taux, est une recalibration de la borne — la consigne de grooming dit
explicitement « ne les recalcule pas toi-même ».

| option | nombre de vols | ce qu'il faut écrire |
|---|---|---|
| **D2(a)** — valeur faciale, 429 lignes déplacées par vol | **46** | rien à changer ; 45 vols ne passent pas les AC du ticket |
| **D2(b)** — une relocalisation se compte au taux de son coût d'écriture, pas de conception ; la borne mord sur les 540 lignes **écrites** (bibliothèque, en-têtes, câblage, re-ciblages) | **6** : 1 vol d'ossature + 5 vols de 5 suites (≤ ~4 900 lignes déplacées par vol, sous l'enveloppe D1(a)) ou **21** sous D1(b) | une ligne de doctrine dans `_plan_size_per_flight_bound` et dans le document de dérivation — hors périmètre de ce ticket |
| **D2(c)** — un vol par famille, borne dépassée et assumée au cas par cas | **25** | une dérogation nommée, portée par l'opérateur, vol par vol |

**`Tracked in:` — non nommable avant D2.** Le nombre de vols va de 1 à 46 selon D1 et D2 combinées.
Ouvrir la série de tickets de suivi sur la mauvaise hypothèse, c'est une série à refermer. Le plan
fournit les deux découpages chiffrés et laisse l'opérateur ouvrir la série en tranchant ; les noms
de suite de la table du § *Découpage retenu* sont les titres de ces tickets, dans cet ordre.

**Ordre de commit du vol courant, valable sous toute réponse à D1 et D2.** Il est écrit ici pour
que le premier vol parte sans re-grooming :

1. `skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh` — l'ossature, `SCRIPT_DIR` ré-ancré.
2. Les suites du vol, dans l'ordre de la table, chacune avec sa plage retirée de l'agrégat dans
   le **même** commit — jamais un commit où la ligne vit aux deux endroits.
3. `Makefile` — câblage des suites du vol dans `test` et `test-dispatch-lib`.
4. Mesure de parité par famille (R1) pour les suites du vol.
5. AC5 — faussage/rétablissement, une assertion par suite du vol.
6. Au **dernier** vol seulement : retrait de l'agrégat des deux cibles du `Makefile`,
   re-ciblage de `test_support_dirs_exclude_test_files` sur
   `skills/bundled/_shared/tests/test_dispatch_lib_launch_gate_3121.sh`, re-ciblage des 60
   mentions, prose de `check-suite-wiring.sh:49` et `test-check-suite-wiring.sh:267`,
   `canonical-tokens.tsv`.
7. `bash scripts/check-secrets.sh --changed origin/main`, `bash scripts/check-suite-wiring.sh`,
   `bash scripts/canonical-tokens-survey.sh --check`, `make verify-bundled-skills`,
   `bash scripts/verify-pipeline.sh origin/main`.

---

## Fire-Disposition

Option **(a) exception nommée en allowlist**, avec un **ensemble d'exceptions vide**.

Le plan livre 25 détecteurs, mais **aucun détecteur nouveau** : les 1 809 lignes d'assertion sont
déplacées mot pour mot, et leur chemin de succès est inchangé. Aucune violation existante n'a donc
besoin d'une exception, et aucune n'en reçoit :

- **pas d'entrée dans `scripts/suite-wiring.tsv`** — les 25 suites sont câblées dans le `Makefile`
  et atteintes depuis `ci.yml:85` par le hop H2. Le lint étant bidirectionnel, une entrée pour une
  suite atteinte le ferait rougir : l'exception serait sa propre régression ;
- **pas d'entrée dans `LARGE_FILE_ALLOWLIST`** — hors périmètre, et sans objet : la plus grosse
  suite produite pèse 13 % du plafond.

**Les deux assertions auto-nettoyantes** qui rougissent quand l'état change sous elles :

1. `scripts/check-suite-wiring.sh` (règle D\C==R, lint `suite-wiring-lint`) rougit dès qu'une des
   25 suites perd son appelant — c'est le détecteur qui tient le câblage d'AC4 dans le temps.
2. `crates/mika-agent/src/bundled_skills.rs::test_support_dirs_exclude_test_files`, **re-ciblée**
   sur `skills/bundled/_shared/tests/test_dispatch_lib_launch_gate_3121.sh`. Laissée sur
   `test-dispatch-lib.sh`, elle devient vacante le jour où l'agrégat disparaît et passe au vert
   par absence de sujet — l'exacte forme de défaut que le dépôt a documentée deux fois. Le
   re-ciblage est la condition pour que l'exclusion de seeding reste **prouvée** et non supposée.

**Une exception nommée, et elle est environnementale, pas en allowlist.** Les 85 échecs mesurés
aujourd'hui dans le bac à sable de dispatch préexistent sur `main` `946c30d7` (`Results: 2036
passed, 85 failed, SKIPPED: 5`) et sortent sur `fatal: Not a valid object name origin/main`. Ils
ne reçoivent **pas** d'exception dans le dépôt : ils sont nommés, chiffrés et attribués dans le
corps de PR, et `FAIL = 0` est attesté là où il est vrai — l'étape `Dispatch-lib assertion suite
(mika#1772)` de `ci.yml:84-85`. Leur traitement est au § *Preuve de non-perte*, R2.

**AC5 est l'armement.** 25 faussages délibérés, un par suite, chacun devant produire un code de
sortie non nul : c'est la seule preuve qu'un détecteur déplacé est encore armé et pas seulement
présent.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh` (ossature extraite) | 95 |
| 25 en-têtes de suite (shebang, prose, `source`, `_suite_summary_and_exit`) | 300 |
| `Makefile` (câblage des 25 suites dans `test` et `test-dispatch-lib`) | 50 |
| `crates/mika-agent/src/bundled_skills.rs` (re-ciblage de l'assertion) | 4 |
| `scripts/check-suite-wiring.sh` + `scripts/test-check-suite-wiring.sh` (prose) | 6 |
| `scripts/canonical-tokens.tsv` (2 lignes re-ciblées) | 2 |
| mentions re-ciblées hors `docs/`, hors lignes déjà comptées ci-dessus | 83 |
| blocs déplacés mot pour mot depuis `test-dispatch-lib.sh` | 18965 |

Total estimé : 19505 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Le total est **45 fois** la borne par vol, et les lignes écrites — 540 — tiennent largement
dessous. L'écart entre ces deux chiffres **est** la décision D2 : il ne se résout pas par un
découpage que le plan choisirait. Les deux découpages en vols sont chiffrés au § D2 (6 vols,
21 vols, 25 vols ou 46 vols selon D1 et D2), l'ordre de commit du vol courant est écrit au même
endroit, et `Tracked in:` ne peut nommer la suite avant que D2 soit tranchée : le nombre de
tickets à ouvrir varie de 1 à 46 selon la réponse.

## Definition of Done

- [ ] `skills/bundled/_shared/tests/lib-dispatch-lib-suite.sh` existe et porte l'ossature
      inventoriée, avec `SCRIPT_DIR` ancré sur `_shared/`.
- [ ] Les 25 suites de la table existent, chacune sous 1 500 lignes, chacune ne contenant que sa
      plage et son ossature.
- [ ] Aucune ligne de l'agrégat ne vit à deux endroits dans un même commit.
- [ ] Les 25 suites sont câblées directement dans `test` et `test-dispatch-lib`, et
      `bash scripts/check-suite-wiring.sh` est vert sans entrée nouvelle dans `suite-wiring.tsv`.
- [ ] La parité des trois compteurs est mesurée **par famille** et non seulement en somme (R1).
- [ ] `FAIL = 0` est attesté par `ci.yml:84-85`, et les 85 échecs du bac à sable sont nommés et
      attribués dans le corps de PR (R2).
- [ ] 25 faussages d'AC5 sont exécutés et leur séquence est notée dans le corps de PR.
- [ ] `test_support_dirs_exclude_test_files` cible un chemin survivant et n'est pas vacante.
- [ ] `git diff origin/main -- skills/bundled/_shared/dispatch-lib.sh` ne montre que des
      commentaires.
- [ ] `bash scripts/check-secrets.sh --changed origin/main`,
      `bash scripts/canonical-tokens-survey.sh --check`, `make verify-bundled-skills` et
      `bash scripts/verify-pipeline.sh origin/main` sont verts.
- [ ] D1 et D2 sont tranchées par l'opérateur avant le premier vol.

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

## Verdict attendu

`ESCALATE`, sortie nominale (mika#3122 règle 3). Le découpage, l'ossature, le câblage, les quatre
confirmations et le contrat de vérification sont écrits et mesurés : ce plan est prêt à partir. Ce
qui l'arrête n'est pas un manque de préparation, ce sont deux décisions que le code et la doctrine
réservent à l'opérateur — un refus de la politique de permission (D1) et l'unité de la borne par
vol (D2) — chiffrées ici, et dont aucune n'est pratiquée.
