# mika#2761 — rendre structurellement visible une suite de test que le pipeline ne lance pas

**Ticket :** senara-solutions/mika#2761
**Type :** feat
**Labels :** `enhancement`, `p2-normal`, `infrastructure`, `ready`, `dispatch:loop`, `loop-substrate`
**Relie :** mika#2578 (la doctrine), mika#2752 (le cas qui a motivé le ticket), mika#2092 (le lint bidirectionnel), mika#2201 / mika#2309 (le piège de l'allowlist et le précédent déclaratif), mika#1801 / mika#2103 / mika#2107 (la couche 0 et le remède codé sur un membre)

---

## Contexte, et ce que la mesure dit exactement

Le ticket demande explicitement de **refaire la mesure au moment du grooming**. Elle a
été refaite le 2026-10-18 sur `0d415f44` (scripts de mesure sous
`.pilot-scratch/measure-2761.sh` et `.pilot-scratch/classify-2761.sh`). Elle contredit
le chiffre central du corps du ticket, et cette contradiction **change la forme à
livrer**. Il faut donc commencer par là.

### Ce que le ticket mesurait, et le hop qu'il a manqué

Le corps compte « cibles `make …` invoquées par `.github/workflows/ci.yml` : 15 » et en
déduit que la couverture CI est d'environ 15 suites, le reste vivant dans `make test`
que CI n'appelle pas — d'où « un lint naïf classerait ~45 suites en violation ».

Le hop manqué est que **CI n'invoque pas ses gardes principalement par `make`** : il les
invoque *directement*, par `bash scripts/<x>.sh` et `python3 -B scripts/<x>.py`. Les
jobs `byte-slice-lint`, `canonical-token-lint`, `dispatch-seat-declaration-lint`,
`egress-manifest-lint` et une quinzaine d'autres sont tous de cette forme
(`.github/workflows/ci.yml:240`, `:381`, `:403`, `:599`, …). Compter les cibles `make`
nommées par un workflow ne mesure donc qu'une petite fraction de la surface réelle
d'invocation.

### La mesure refaite

| population | compte |
|---|---|
| fichiers `.sh`/`.py` sous `scripts/` + `skills/bundled/_shared/` | 112 |
| dont **suites** (préfixe `test-`/`test_`/`check-`/`verify-`/`measure-`) | **94** |
| dont non-suites (bibliothèques, outils : `lib/`, `fixtures/`, `dispatch-lib.sh`, …) | 18 |
| suites nommées **directement** par un `.github/workflows/*.yml` | 51 |
| suites atteintes via une cible `make` nommée par un workflow | 46 |
| **union atteignable depuis CI** | **78** |
| **suites sur disque que CI n'atteint pas** | **20** |
| suites invoquées par la cible `make test` | 44 |
| … d'entre elles que CI n'atteint par aucun autre chemin | **0** |
| occurrences de `make test` dans `.github/workflows/*.yml` (hors commentaires) | **0** |

Trois conclusions, et chacune élimine une des trois options du ticket.

**1. L'arriéré est de 20 suites, pas de ~45.** C'est un nombre qu'une Fire-Disposition
peut porter nommément sans devenir l'allowlist que mika#2309 condamne.

**2. L'option 1 (fermeture transitive, faire appeler `make test` par CI) n'achèterait
rien.** Les 44 suites de `make test` sont **toutes** déjà atteintes par CI par un autre
chemin — directement, ou par une cible dédiée (`make test-dispatch-lib`,
`make test-egress-push-bound`, …). Le constat littéral de mika#2578 (« CI n'appelle pas
`make test` ») reste vrai, mais il est aujourd'hui **sans conséquence** : l'intersection
`make test` ∩ arriéré est vide. Câbler `make test` dans CI coûterait du temps de job
pour zéro couverture nouvelle. L'option 1 est écartée par la mesure, pas par un
jugement.

**3. L'option 3 (borne sur la croissance) est strictement dominée par l'option 2 pour
le même coût.** Le poste de code dominant est le calcul de l'atteignabilité, identique
dans les deux formes. Mais un compteur ne dit pas *lesquelles* des 20, ni *pourquoi*
l'une d'elles n'est pas câblée ; il est satisfait par la suppression d'une suite autant
que par son câblage ; et il n'a aucune réponse pour les instruments délibérément non
câblés (`measure-plan-header-coverage.sh`, que le `Makefile:200` et `CLAUDE.md`
qualifient tous deux de « instrument, deliberately NOT a CI gate »), qui gonfleraient le
compteur à perpétuité.

### Le document qui a déjà arbitré la décision de forme

Le ticket présente la décision de forme comme non écrite. Elle l'est en partie, et le
corps ne la cite pas :
`docs/solutions/best-practices/une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md`
(issu de mika#1801) est le document de doctrine de cette classe. Il nomme la classe
« couche 0 » d'une taxonomie à quatre couches de l'inertie d'une garde, et il établit
trois choses que ce plan reprend telles quelles :

- **La règle** : « Une garde se câble dans le diff qui la crée. […] Deux étapes dans le
  job, jamais une : le scan, puis son anti-vacuité. »
- **Pourquoi le scan naïf n'a pas été livré** : ce n'est *pas* faute de population,
  c'est le **discriminant**. « Séparer "garde à câbler" de "geste d'opérateur" n'a pas
  de forme lisible dans l'arbre : les deux sont des scripts shell sous `scripts/`. Le
  scan aurait donc besoin d'une allowlist nommée […] et une allowlist qui est la sortie
  normale se remplit. »
- **Le discriminant qu'il propose** : « les quatre écarts sont tous des **harnais qui
  tournent hors-ligne** […] là où les neuf gestes légitimes ont tous besoin d'un service
  vivant, d'un jeton ou d'un pid. "Ce script a-t-il besoin du monde extérieur ?" est une
  question que le scan ne sait pas poser, mais qu'un humain tranche en lisant l'en-tête
  — donc la première étape du suivi est **un recensement annoté, pas un prédicat**. »

Et il se clôt sur l'impératif que le ticket reformule au point 2 de son travail attendu :
« son silence ne prouve rien tant que personne ne la lance : un inventaire que nul
n'exécute est, très précisément, le défaut que ce doc décrit, appliqué à lui-même. »

Son recensement du 2026-09-28 nommait quatre écarts citables. **Les quatre sont encore
dans mes 20 vingt jours plus tard** : `verify-pipeline-test.sh`,
`test-smoke-search-substrate.sh`, `test-measure-pilot-cycle-emptiness.sh`,
`test-mika-groom-milestone.sh`. Un recensement que rien ne tient ne garde rien — c'est
le ticket, appliqué à son propre document de doctrine.

### Les 20, classées

Mesuré le 2026-10-18. Dix sous `scripts/`, dix sous `skills/bundled/_shared/`.

**Groupe A — citée par une cible `make` que CI n'appelle pas (6).** Le câblage s'arrête
à mi-chemin : la cible existe, aucun workflow ne la nomme.

| suite | cible `make` qui la nomme |
|---|---|
| `scripts/check-release-debuginfo.sh` | `check-release-debuginfo` (`Makefile:327`) |
| `scripts/test-check-release-debuginfo.sh` | `check-release-debuginfo` |
| `scripts/measure-plan-header-coverage.sh` | `measure-plan-header-coverage` (`Makefile:200`) |
| `scripts/test-measure-empty-turns.sh` | `test-measure-empty-turns` (`Makefile:191`) |
| `skills/bundled/_shared/tests/test_rescue_cause_token.sh` | `test-rescue-cause-token` (`Makefile:222`) |
| `skills/bundled/_shared/tests/test_rescue_commit_no_verify.sh` | `test-rescue-cause-token` |

**Groupe B — aucune cible `make`, aucun workflow (14).** Et pour huit d'entre elles, la
seule trace de la façon de les lancer est **leur propre commentaire `# Run:`** — vérifié
par grep : `test_parse_disposition.sh`, `test_finalize_pr_gate.sh`,
`test_force_push_guard.sh`, `test_push_with_rebase_retry.sh`, `test_rescue_fmt_clean.sh`,
`test_rescue_scratch_not_published.sh`, `test_seed_worktree_slash_commands.sh`,
`test-trace-redaction.sh` ne sont citées par rien d'autre que ce commentaire et par de
la prose dans des suites voisines. **Personne ne les a lancées depuis leur merge.**
Les six autres : `scripts/verify-pipeline-test.sh`,
`scripts/test-smoke-search-substrate.sh`,
`scripts/test-measure-pilot-cycle-emptiness.sh`, `scripts/test-mika-groom-milestone.sh`,
`scripts/verify-dependabot-topology.sh`, `scripts/test-verify-dependabot-topology.sh`.

Ce `# Run:` est le fait le plus parlant de la mesure : la convention d'invocation existe,
l'auteur l'a écrite, et elle est en prose — donc aucune machine ne la lit.

---

## La direction retenue

**Un registre déclaratif du résidu, comparé aux deux réalités dans les deux sens**
(option 2 du ticket), avec **le discriminant de mika#1801 comme vocabulaire de mode** et
**la bidirectionnalité comme assertion auto-nettoyante**.

### Trois écarts assumés par rapport à l'option 2 telle que le ticket l'écrit

**1. Le registre déclare le résidu, pas chaque suite.** Le ticket écrit « chaque suite
déclare son mode d'exécution ». Ce serait 94 entrées, et ce serait une erreur de forme :
pour les 74 suites que CI atteint, **l'arbre est déjà la source de vérité** — le workflow
les nomme. Les déclarer créerait un second site à maintenir pour un fait déjà prouvé, et
un registre qu'il faut éditer à chaque changement de câblage CI est un registre qui
dérive : exactement la classe de mika#2092, auto-infligée.

Le précédent `scripts/canonical-tokens.tsv` déclare *tous* ses tokens parce que **rien
d'autre dans l'arbre n'énonce la tolérance d'un lecteur** — le registre y est la seule
source. Ici CI *est* la source pour « est-ce câblé ». Le principe déclaratif ne
s'applique donc qu'au résidu : les suites dont le statut est ambigu.

**2. C'est structurellement une allowlist, et le plan le dit plutôt que de l'esquiver.**
Une liste de suites exemptes de « doit être atteinte par CI » est une allowlist, et
mika#2309 est cité par le ticket précisément comme le piège. Ce qui rend l'allowlist de
mika#2309 dangereuse est double : ses entrées ne portent **aucun contenu positif** (elles
disent « saute-moi »), et **rien ne les périme** (l'entrée reste vraie pour toujours).

Les deux défauts sont refermés par construction, et le second l'est gratuitement par la
bidirectionnalité : **si CI vient à atteindre une suite déclarée non câblée, le lint
rougit.** L'entrée ne peut donc pas survivre à son obsolescence — au moment où quelqu'un
câble la suite, la déclaration devient un rouge et doit être supprimée. C'est
littéralement l'« assertion auto-nettoyante qui rougit quand l'exception devient stale »
que la règle (a) de mika#1574 exige, et elle ne coûte pas une ligne de plus que la
direction inverse qu'on écrit de toute façon.

**3. Le lint ne prouve pas l'atteignabilité par le shell, et il le déclare.** Voir
§ *Ce que le lint ne prouve pas* ci-dessous — c'est une sous-approximation délibérée,
et la taire serait livrer un faux vert.

### Le vocabulaire de mode

Trois états, dont **un seul n'est pas écrit** :

| mode | sens | absent du registre ? |
|---|---|---|
| *(câblée)* | CI l'atteint. Prouvé par l'arbre. | **oui** — ne s'écrit jamais |
| `operator` | a besoin du monde extérieur (réseau, jeton, service vivant, pid) ou est un instrument de mesure délibéré. Légitimement pas une garde CI. | non |
| `unwired` | tourne hors-ligne, et rien ne la lance. **L'arriéré.** | non |

La frontière `operator` / `unwired` est exactement le discriminant de mika#1801 : *ce
script a-t-il besoin du monde extérieur ?* Le document dit que la machine ne sait pas
poser cette question. Elle ne la pose donc pas : **l'humain tranche en remplissant le
champ, la machine vérifie la forme** — mode dans le vocabulaire, raison non vide, ticket
présent. C'est la discipline de `canonical-tokens.tsv`, dont la colonne `tolérance` est
un format de fil validé contre une liste close de valeurs, le contenu restant humain.

`unwired` est un mode **distinct** de `operator` et non une nuance de prose, pour une
raison opérationnelle : l'arriéré devient `grep -c 'unwired'`, donc un nombre, donc
quelque chose dont un suivi peut mesurer la décroissance. Un arriéré noyé dans un champ
raison libre n'est pas comptable.

### Les quatre directions

Soit **D** l'ensemble des suites sur disque, **C** l'ensemble atteignable depuis CI,
**R** l'ensemble des chemins déclarés au registre.

| # | direction | ce qu'elle attrape | le défaut du ticket |
|---|---|---|---|
| **D1** | `d ∈ D`, `d ∉ C`, `d ∉ R` | une suite que rien ne lance et que personne n'a déclarée | **oui — c'est mika#2752** |
| **D2** | `r ∈ R`, `r ∈ C` | une déclaration périmée : la suite a été câblée, l'entrée reste | l'auto-nettoyage |
| **D3** | `r ∈ R`, `r ∉ D` | une entrée qui nomme un fichier supprimé | la dérive du registre |
| **D4** | `r ∈ R` dont le mode, la raison ou le ticket est mal formé | une déclaration qui n'en est pas une | l'hygiène du résidu |

D1 est la direction qui aurait refusé `make test-egress-push-bound` et
`make test-measure-webhook-turn-cost` le jour de mika#2736 et de mika#2680 vol 2 : les
deux suites étaient sur disque, hors de CI, et non déclarées. D2 est ce qui rend le
registre non-inerte. Ensemble, D1 et D2 sont la bidirectionnalité de mika#2092.

### Comment C est calculé, et ce que le lint ne prouve pas

**C** est prouvé sur **deux hops, tous deux syntaxiques** :

- **H1** — un `.github/workflows/*.yml` nomme littéralement le chemin de la suite
  (`bash scripts/x.sh`, `python3 -B scripts/x.py`), y compris dans un bloc `run: |`.
- **H2** — un workflow nomme `make <t>`, et la recette de `<t>` — plus ses prérequis,
  transitivement, plus la récursion `$(MAKE) <u>` — nomme littéralement le chemin.

**Ce que le lint ne prouve pas, et c'est délibéré :** une suite invoquée *uniquement*
depuis le corps shell d'une autre suite. Le cas existe dans l'arbre
(`test-dispatch-lib.sh:7985` lance `check-pilot-turn-ceiling-labels.sh`), et le prouver
demanderait d'interpréter du shell — ce qui se trompe dans les deux sens, et dont le sens
dangereux est le **faux vert** : une suite qui *se lit* comme câblée et ne l'est pas,
c'est-à-dire le défaut même.

Le lint **sous-approxime donc C**, ce qui ne produit que des faux *rouges* — bruyants,
jamais silencieux. Conséquence à assumer : une suite atteinte seulement par un runner
shell se lit comme non atteinte, et son auteur doit choisir — la câbler directement, ou
la déclarer `operator` en nommant le runner dans la raison. **Aucune suite n'est dans cet
état aujourd'hui** (mesuré : `C = reach-direct ∪ reach-make` couvre les 74, et les 20
orphelines n'ont aucun invocateur du tout), donc aucun mode `via-runner` n'est livré —
ce serait du YAGNI pour un ensemble vide. Le jour où une telle suite apparaît, son auteur
voit un rouge et tranche.

### La population D, et pourquoi elle est définie par préfixe

`D` = les fichiers `.sh`/`.py` sous `scripts/` et `skills/bundled/_shared/` dont le nom
de base porte un préfixe `test-`, `test_`, `check-`, `verify-` ou `measure-`. C'est 94
des 112 fichiers de ces deux arbres ; les 18 écartés sont des bibliothèques et des
outils (`lib/`, `fixtures/`, `dispatch-lib.sh`, `generate-openapi.sh`, les
`mika-pilot-*`), qui ne sont pas des détecteurs et dont le chemin de succès n'est pas
« aucune violation trouvée ».

Le préfixe est une convention **déjà universelle** dans ces deux arbres — pas une règle
inventée ici. Le lint doit néanmoins porter un **contrôle positif sur la taille de D**
(mika#2205, et la couche 2 de la taxonomie mika#1801 : « population vide ») : si `|D|`
tombe sous un plancher déclaré, le lint rougit au lieu de passer au vert en ne regardant
rien. Le plancher est écrit en tête du registre avec sa date de mesure.

### Le lint se câble lui-même

Le ticket l'exige (« sous peine d'être son propre premier contre-exemple ») et
mika#1801 en donne la forme : **deux étapes dans le job, jamais une** — le scan, puis son
anti-vacuité. Et, par construction, le nouveau job est le premier à être vérifié par le
nouveau lint : `check-suite-wiring.sh` et `test-check-suite-wiring.sh` entrent dans `D`,
doivent donc être dans `C`, et D1 rougirait sur eux si la strophe `ci.yml` manquait. Le
lint est son propre premier sujet.

---

## Hors périmètre, et pourquoi

- **Câbler les 20 de l'arriéré.** C'est du triage de test, pas du travail de lint : il
  faut lire 20 en-têtes, décider pour chacune si elle doit garder quelque chose, et pour
  les `unwired` écrire la strophe CI et constater qu'elles passent — dont quatre que
  mika#1801 nommait déjà il y a vingt jours. Ça n'entre pas sous la borne par vol, et ça
  se fait sous un ticket dont l'arriéré `grep -c unwired` est la liste de travail. Voir
  § Fire-Disposition.
- **Faire appeler `make test` par CI.** Écarté par la mesure (zéro couverture nouvelle).
- **Étendre `D` aux tests Rust ou aux suites `npm`.** `cargo test` et `npm test` sont
  déjà invoqués par CI et agrègent leurs suites par découverte du compilateur et du
  runner — la classe de défaut du ticket (« écrite, mergée, jamais appelée ») n'existe
  pas pour un `#[test]`. Le périmètre est le shell, où l'agrégation est manuelle.
- **Corroborer la revendication `operator`** en grepant `gh api` / `curl` / `systemctl`
  dans la suite. Tentant, et c'est la famille d'heuristique dont mika#1801 dit que la
  machine ne sait pas la poser. Le lint vérifie la **forme** de la déclaration, pas sa
  véracité.
- **Les autres dépôts du workspace** (`mika-platform`, `mika-cloud`, `mika-skills`).
  Même classe, autre arbre, autre CI.

---

## Livrables et ordre de commit

Un seul commit logique, dans cet ordre — chaque étape est vérifiable avant la suivante.

1. **`scripts/suite-wiring.tsv`** — le registre. En-tête en prose : la règle qu'il
   encode, le vocabulaire de mode comme format de fil, le plancher de `|D|` avec sa date
   de mesure, et le renvoi à mika#1801 pour le discriminant. Puis 20 lignes de données,
   quatre colonnes séparées par tabulation : `chemin`, `mode`, `raison`, `ticket`.
   Les six du groupe A et les quatre que mika#1801 nommait sont seedées avec la raison
   que leur en-tête ou leur doc donne déjà (p. ex. `measure-plan-header-coverage.sh` →
   `operator`, instrument délibéré, `Makefile:200` + `CLAUDE.md` ;
   `verify-dependabot-topology.sh` → `operator`, `gh api` sur trois dépôts, son doc écrit
   « pas câblé au CI, à dessein »). Les huit du `# Run:` → `unwired`, ticket de suivi.

2. **`scripts/check-suite-wiring.sh`** — le lint. Énumération de `D` + contrôle positif
   de plancher ; calcul de `C` par H1 et H2 ; lecture de `R` ; les quatre directions par
   arithmétique d'ensembles (`comm`) ; un rapport qui imprime les trois cardinalités et
   le compte `unwired`, parce qu'un lint qui n'imprime que « ok » ne permet pas de voir
   qu'il a cessé de regarder. En-tête : pourquoi il existe, et la sous-approximation de
   `C` énoncée noir sur blanc.

3. **`scripts/test-check-suite-wiring.sh`** — l'anti-vacuité. Les quatre directions vues
   **rouges** et vues **vertes**, sur un arbre-fixture temporaire sous `.pilot-scratch/`
   (faux `scripts/`, faux `.github/workflows/`, faux `Makefile`) — jamais sur l'arbre
   réel. Plus le contrôle de plancher de population vu rouge sur un `D` vidé, et le
   contrôle négatif du hop H2 (une cible `make` nommée par un workflow dont la recette
   nomme la suite ⇒ vert ; la même cible non nommée par le workflow ⇒ rouge).

4. **`Makefile`** — cible `check-suite-wiring` lançant le scan puis son anti-vacuité
   (forme de `check-release-debuginfo`, `Makefile:327`), plus l'entrée `.PHONY`.

5. **`.github/workflows/ci.yml`** — job `suite-wiring-lint`, deux étapes
   (`bash scripts/check-suite-wiring.sh`, puis
   `bash scripts/test-check-suite-wiring.sh`), sur le patron de
   `dispatch-seat-declaration-lint` (`:395`).

6. **Docs (hors compte).** Refermer la section « Pourquoi la garde de famille n'est pas
   livrée ici » de
   `docs/solutions/best-practices/une-garde-que-rien-ninvoque-se-lit-exactement-comme-un-arbre-propre-2026-09-28.md`
   — elle dit que le suivi s'ouvre par un recensement annoté et une décision explicite
   sur le discriminant ; c'est ce que ce vol livre, et le document doit cesser d'annoncer
   une absence. Plus la ligne `suite-wiring-lint` dans la liste CI de `CLAUDE.md`
   § Architecture Summary.

---

## Verification Contract

| # | vérification | commande | attendu |
|---|---|---|---|
| V1 | le lint est vert sur l'arbre après seed du registre | `bash scripts/check-suite-wiring.sh` | exit 0, rapport imprimant `\|D\|=94 \|C\|=78 \|R\|=20` et `unwired=…` |
| V2 | l'anti-vacuité passe, les quatre directions vues rouges | `bash scripts/test-check-suite-wiring.sh` | exit 0, `N passed, 0 failed` |
| V3 | D1 rougit sur une suite neuve non câblée non déclarée | fixture : ajouter `scripts/check-zorglub.sh` à l'arbre-fixture | exit ≠ 0, message nommant `check-zorglub.sh` |
| V4 | D2 rougit sur une entrée périmée | fixture : déclarer `unwired` une suite que le workflow-fixture nomme | exit ≠ 0, message nommant l'entrée à supprimer |
| V5 | D3 rougit sur une entrée orpheline | fixture : entrée nommant un chemin absent | exit ≠ 0 |
| V6 | D4 rougit sur un mode hors vocabulaire / raison vide / ticket absent | fixture : trois entrées mal formées | exit ≠ 0, trois messages distincts |
| V7 | le contrôle positif de population rougit sur un `D` vide | fixture : arbre `scripts/` sans suite | exit ≠ 0, message nommant le plancher |
| V8 | le hop H2 est réellement prouvé | fixture : cible `make` nommée par le workflow, recette nommant la suite | vert ; puis retirer le `make <t>` du workflow ⇒ rouge |
| V9 | **le lint se câble lui-même** | `grep -n 'check-suite-wiring' .github/workflows/ci.yml Makefile` | les deux étapes du job + la cible `make` |
| V10 | le lint est son propre sujet | retirer la strophe `ci.yml` et relancer V1 | rouge, nommant `check-suite-wiring.sh` et `test-check-suite-wiring.sh` |
| V11 | la cible `make` fonctionne | `make check-suite-wiring` | exit 0 |
| V12 | pas de régression de pipeline | `make test && cargo clippy --all-targets --all-features -- -D warnings && bash scripts/verify-pipeline.sh origin/main` | exit 0 |
| V13 | hygiène shell du dépôt | `bash -n` sur les deux scripts + `bash scripts/verify-no-sigpipe-grep.sh` | exit 0 |

V10 est la vérification qui distingue ce vol de mika#1801 : le lint doit rougir **sur
lui-même** si on le décâble.

---

## Risques

| risque | effet | mitigation |
|---|---|---|
| le calcul de `C` sous-approxime et produit un faux rouge sur une suite atteinte par un runner shell | bruit à l'atterrissage, puis tentation d'allowlister | mesuré : zéro suite dans cet état aujourd'hui (V1 est vert sur l'arbre réel avant merge). La sous-approximation est écrite dans l'en-tête du lint, et sa sortie nommée est une déclaration `operator` avec le runner — pas une exception muette |
| le parsing du `Makefile` (prérequis transitifs, `$(MAKE)`) rate un chemin | faux rouge | V8 prouve le hop H2 dans les deux sens sur fixture ; V1 le prouve sur les 46 chemins réels atteints par H2 |
| un `run: \|` multi-ligne de `ci.yml` échappe à H1 | faux rouge | la mesure a trouvé les 5 suites du bloc multi-ligne `:125-131` ; V1 le confirme sur l'arbre |
| le registre devient la sortie normale (la crainte de mika#1801) | l'allowlist se remplit | D2 le périme automatiquement ; `unwired` est un mode distinct et comptable, donc l'arriéré a un nombre qu'un suivi peut faire décroître ; et D4 exige raison + ticket, donc une entrée coûte une décision écrite |
| la convention de préfixe cesse d'être universelle | une suite neuve échappe à `D` | le contrôle de plancher voit une chute de `\|D\|`, pas une suite isolée. Limite assumée et écrite : c'est une convention de nommage, pas une propriété prouvée |
| le volume dépasse l'estimation | mort au plafond de tours | § Taille estimée nomme une ligne de coupe pré-identifiée qui ne touche pas l'anti-vacuité |

---

## Definition of Done

- `scripts/suite-wiring.tsv` existe, porte son en-tête de règle et 20 entrées valides.
- `scripts/check-suite-wiring.sh` implémente `D`, `C` (H1+H2), `R`, les quatre
  directions et le contrôle positif de population ; en-tête énonçant la
  sous-approximation.
- `scripts/test-check-suite-wiring.sh` voit les quatre directions rouges **et** vertes
  sur arbre-fixture, plus le plancher et le hop H2.
- Cible `make check-suite-wiring` + `.PHONY`.
- Job `suite-wiring-lint` dans `ci.yml`, **deux** étapes.
- V1–V13 passent.
- Le document mika#1801 ne déclare plus la garde de famille comme non livrée ;
  `CLAUDE.md` liste le nouveau job.
- Le corps de PR porte la Fire-Disposition et la ligne `Tracked in:` du suivi
  d'arriéré.

---

## Acceptance criteria

Le corps du ticket n'a pas de section `## Acceptance criteria`. Les critères suivants
sont dérivés de son § *Travail attendu* (trois points) et du Verification Contract.

- **AC1 — la décision de forme est groomée, et motivée par une mesure refaite.** Le plan
  refait la mesure du corps, nomme le hop qu'elle avait manqué (l'invocation directe
  `bash scripts/…`, 51 suites), corrige l'arriéré de ~45 à 20, et écarte les options 1 et
  3 du ticket **par la mesure** et non par préférence. Il cite le document de doctrine
  mika#1801 que le corps ne citait pas, et adopte son discriminant.
- **AC2 — le lint retenu est livré.** `scripts/check-suite-wiring.sh` refuse une suite
  sur disque que CI n'atteint pas et que le registre ne déclare pas (D1), et refuse une
  déclaration périmée (D2), orpheline (D3) ou mal formée (D4). V1, V3–V6.
- **AC3 — sa suite anti-vacuité est livrée, au motif du dépôt.**
  `scripts/test-check-suite-wiring.sh` voit chacune des quatre directions **rouge** et
  **verte**, sur arbre-fixture. V2.
- **AC4 — le lint se câble lui-même, dans le même diff.** Job `suite-wiring-lint` dans
  `ci.yml` avec ses deux étapes, et cible `make`. V9, V11.
- **AC5 — et il est son propre premier sujet.** Décâblé de `ci.yml`, le lint rougit en
  nommant ses deux propres scripts. V10.
- **AC6 — le registre n'est pas l'allowlist de mika#2309.** Toute entrée porte mode +
  raison + ticket (D4), et aucune entrée ne survit à son câblage (D2). Les deux sont
  vues rouges par l'anti-vacuité. V4, V6.
- **AC7 — le lint ne peut pas passer au vert en ne regardant rien.** Contrôle positif
  sur `|D|` avec plancher daté ; rapport imprimant les trois cardinalités. V1, V7.
- **AC8 — Fire-Disposition explicite sur l'arriéré mesuré.** Les 20 sont nommées une par
  une au registre, classées `operator` ou `unwired` ; l'arriéré est `grep -c unwired` ;
  un ticket de suivi le porte, nommé par `Tracked in:`.
- **AC9 — aucune régression de pipeline.** V12, V13.

---

## Fire-Disposition

Ce vol livre un détecteur (`check-suite-wiring.sh`, chemin de succès « aucune suite non
câblée non déclarée »). Section requise.

**Option retenue : (a) exception nommée en allowlist**, sous la forme du registre
`scripts/suite-wiring.tsv`, pour les **20 suites** mesurées le 2026-10-18.

Le détail d'implémentation, point par point contre les exigences de la règle (a) :

1. **Chaque violation existante reçoit une entrée grep-visible qui nomme la donnée
   précise.** 20 entrées, une par chemin, `chemin\tmode\traison\tticket`. Aucune entrée
   générique, aucun glob, aucun répertoire entier : le chemin exact de la suite.
   `grep -n 'test_parse_disposition' scripts/suite-wiring.tsv` donne la ligne.

2. **Chaque entrée référence un ticket de suivi.** Les `unwired` référencent le ticket de
   câblage de l'arriéré (nommé par `Tracked in:` ci-dessous). Les `operator` référencent
   le ticket ou le document qui établit leur caractère délibéré — pour les quatre déjà
   recensées par mika#1801 et pour `measure-plan-header-coverage.sh`, cette référence
   existe déjà dans l'arbre et est reprise, pas inventée. D4 refuse une entrée sans
   ticket.

3. **L'assertion auto-nettoyante est D2, et elle est structurelle.** Si CI vient à
   atteindre une suite déclarée au registre, **le lint rougit** et exige la suppression de
   l'entrée. Une exception ne peut donc pas survivre à son obsolescence : le geste même
   qui la rend inutile (câbler la suite) la rend rouge. D3 ajoute le cas symétrique de la
   suppression du fichier. Les deux sont vues rouges par `test-check-suite-wiring.sh`
   (V4, V5) — l'auto-nettoyage est lui-même sous anti-vacuité.

4. **Et ce n'est pas l'allowlist que mika#2309 condamne, pour une raison nommable.**
   L'entrée de mika#2309 ne portait aucun contenu positif et rien ne la périmait. Ici
   l'entrée porte un mode issu d'un vocabulaire clos, une raison humaine et un ticket
   (contenu positif, exigé par D4), et elle est périmée par D2. Deuxième différence :
   l'allowlist de mika#2309 était la **sortie normale** du lint — la crainte explicite de
   mika#1801 (« une allowlist qui est la sortie normale se remplit »). Ici la sortie
   normale d'une suite neuve est d'être **câblée** : c'est un bloc de 3 lignes dans
   `ci.yml`, à côté d'une déclaration qui coûte une raison écrite et un ticket. Le coût
   est du bon côté.

5. **L'arriéré n'est pas refermé par ce vol, et son compte est un invariant lisible.**
   Les suites `unwired` restent non câblées : les câbler est du triage de test (lire 20
   en-têtes, décider, écrire 20 strophes, constater que chacune passe), hors borne par
   vol. Ce que ce vol garantit est que l'arriéré est **nommé, compté et borné** :
   `grep -c 'unwired' scripts/suite-wiring.tsv` est un nombre, D1 interdit qu'il
   croisse en silence, et D2 fait que chaque câblage le fait décroître de façon visible.
   Le compte constaté à l'atterrissage est écrit dans le corps de PR.

6. **Aucune Fire-Disposition (c).** Rien dans ce vol ne demande d'arbitrage opérateur :
   la décision de forme est tranchée ci-dessus par la mesure et par le document de
   doctrine existant.

**Tracked in:** un ticket de suivi « câbler ou retirer les suites `unwired` de
`scripts/suite-wiring.tsv` » est ouvert à l'atterrissage de ce vol, et sa liste de
travail est `grep 'unwired' scripts/suite-wiring.tsv`. Il porte explicitement les quatre
suites que mika#1801 recensait le 2026-09-28 et qui sont toujours non câblées
(`verify-pipeline-test.sh`, `test-smoke-search-substrate.sh`,
`test-measure-pilot-cycle-emptiness.sh`, `test-mika-groom-milestone.sh`), dont
`verify-pipeline-test.sh` **a déjà coûté** : mika#2516 a annoncé une garde à l'intérieur
de ce harnais, elle n'a jamais été livrée, et personne ne l'a vu.

---

## Taille estimée

Estimé le 2026-10-18, calibré sur les lints bidirectionnels comparables du dépôt plutôt
qu'à vue : `verify-egress-manifest.sh` 81 lignes (manifeste ↔ code, le plus proche en
*forme* — comparaison d'un registre à une réalité), `check-pilot-turn-ceiling-labels.sh`
217, `check-dispatch-seats-declared.sh` 223, `check-canonical-tokens.sh` 444 (le plus
gros, et sa taille vient de ses neuf classes de tolérance par-règle — poste que ce lint
n'a pas). Côté anti-vacuité : 222 et 271 pour les deux lints bidirectionnels, 883 pour
`test-verify-egress-manifest.sh` (hors-norme).

Ce lint est de l'arithmétique d'ensembles (`comm` sur trois listes) plus un calcul
d'atteignabilité sur deux hops : plus que les 81 de `verify-egress-manifest.sh` à cause
du parsing `Makefile` + workflows, nettement moins que les 444 faute de logique
par-règle.

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `scripts/suite-wiring.tsv` (en-tête de règle ~35 + 20 entrées + plancher) | 58 |
| `scripts/check-suite-wiring.sh` (population + plancher, `C` par H1/H2, `R`, 4 directions, rapport) | 165 |
| `scripts/test-check-suite-wiring.sh` (4 directions × rouge/vert sur arbre-fixture, plancher, hop H2) | 175 |
| `.github/workflows/ci.yml` (job `suite-wiring-lint`, 2 étapes + commentaire de cause) | 14 |
| `Makefile` (cible `check-suite-wiring` + `.PHONY`) | 5 |

Total estimé : 417 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient — mais la marge est mince et c'est dit.**
417 < 429 et 417 < 1000 : les deux bornes passent, la plus serrée avec **12 lignes de
marge seulement**. C'est assumé plutôt que maquillé, et deux choses le rendent tenable.

D'abord, le périmètre est fermé par construction : **trois fichiers neufs**, **deux
fichiers touchés** (`ci.yml`, `Makefile`), aucun code Rust, aucune migration de schéma,
aucun test existant réécrit, aucun appelant à retoucher. Le poste qui aurait fait
déborder — déclarer les 94 suites au registre plutôt que les 20 du résidu — a été écarté
pour une raison de conception (§ écart 1), pas pour tenir le nombre : il aurait ajouté
~74 lignes de données **et** un second site de vérité pour un fait que l'arbre prouve
déjà.

Ensuite, **la ligne de coupe est pré-identifiée**, pour que le vol atterrisse plutôt que
de mourir au plafond. Si `check-suite-wiring.sh` dépasse son estimation, l'implémentation
retire **la direction D4** (validation de forme des champs : mode dans le vocabulaire,
raison non vide, ticket présent) du lint et de son anti-vacuité — environ 30 + 40 lignes
— et la renvoie au ticket de suivi d'arriéré nommé en Fire-Disposition. D4 est le bon
sacrifice : une entrée mal formée est **inerte**, jamais un faux vert, là où D1 et D2
portent la classe de défaut du ticket et l'auto-nettoyage de l'allowlist.

**Ce qui ne se coupe pas, sous aucun budget :** `test-check-suite-wiring.sh`. Livrer le
détecteur sans son anti-vacuité serait la décoration de mika#2103, condamnée mot pour mot
par le document de doctrine que ce vol vient refermer — et ce serait le ticket, appliqué
à son propre remède.
