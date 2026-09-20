# mika#1950 — le rejeu prescrit n'est pas exécutable ; la mesure qu'il visait l'est

**Ticket :** senara-solutions/mika#1950
**Type :** chore (eval)
**Branche :** `chore/1950/eval-mika-1910-silent-empty-jsonl`
**Date :** 2026-09-20

---

## Goal Capsule

Le ticket demande de rejouer deux fixtures JSONL sous `glm-5.2` puis `glm-5.3`
et de rendre un verdict **FIXED / IGNORED / AGGRAVER** sur le symptôme mika#1910
(sortie vide silencieuse sur boucle d'outils longue), en bloquant la promotion de
`glm-5.3` vers mika-dev et mika-qa jusqu'à ce verdict.

**Sept faits, lus dans le dépôt et sur l'hôte, déplacent le ticket.** Les
fixtures n'existent nulle part ; la condition bloquante a été franchie des deux
côtés ; un verdict par une autre voie existe déjà ; trois changements de substrat
rendent l'attribution au modèle impossible par rejeu ; la population dont la
mesure fondatrice est tirée tourne sous **Claude**, pas sous GLM ; une cause
concurrente domine cette population ; et l'instrument que le ticket réclame a été
livré depuis et tourne en production.

Ce plan livre donc ce que le ticket **visait** — une mesure du symptôme,
décidable sur sortie machine, reproductible — et refuse explicitement de
fabriquer le verdict attribuable qu'il **prescrivait**, en nommant pourquoi. Il
ne promeut aucun modèle, ne swappe rien, et ne ferme pas mika#1910.

---

## Product Contract

### La rectification est le premier livrable

Un plan qui exécuterait la lettre du ticket produirait un rapport faux avec
l'autorité d'une mesure. Les sept constats ci-dessous sont donc du contenu, pas
du préambule ; chacun est vérifiable par la commande qui l'accompagne.

#### R1 — Les fixtures n'existent nulle part

Le ticket localise les fixtures en
`docs/eval/hy3-v0-2026-08-18/traces/run_glm-5.2_{1878,1817}.jsonl`. Le répertoire
`docs/eval/` contient **une seule** entrée, `calibration/`.

```bash
ls docs/eval/                                            # → calibration
git log --all --oneline -- 'docs/eval/hy3-v0-2026-08-18/**'   # → vide
git log --all --oneline --grep="hy3"                     # → vide
find /data/workspace/mika-platform -name "run_glm*" -not -path "*/target/*"  # → vide
```

Elles n'ont jamais été versées : `--all` couvre toute branche et tout historique
atteignable, et la recherche workspace couvre les deux dépôts présents
(`claude-pilot`, `mika`). **Le ticket n'est pas exécutable à la lettre**, et
aucune reconstruction n'est possible : ce sont des artefacts d'une campagne d'août
qui n'a laissé aucune trace ici.

#### R2 — La condition bloquante a été franchie des deux côtés

Le ticket pose : *« Do NOT promote GLM-5.3 to mika-dev OR mika-qa UNTIL this
replay verdict lands. »* Les deux moitiés sont caduques, pour des raisons
différentes.

- **mika-dev** a été basculé sur `glm-5.3` le **2026-08-26**, sans ce verdict. La
  bascule vit **hors du dépôt** : `MIKA_DEV_CONFIG` déclare toujours
  `openrouter_model = "z-ai/glm-5.2"` et son propre doc-comment nomme la dérive
  (`well_known_agents.rs:165-180`, « its source has DRIFTED from its runtime …
  the plans of mika#2179 and mika#2189 measure mika-dev on `z-ai/glm-5.3` »).
- **mika-qa** y est passé, a cassé son enveloppe de revue le **2026-09-15**
  (PR #2327, tours de 258–315 s, `hold[review]` moteur, verdict perdu), et est
  revenu à `glm-5.2`. Sa promotion est désormais gouvernée par **mika#2328**, dont
  la condition Prime est **conjonctive** et plus stricte que celle de ce ticket,
  avec une procédure déjà écrite (`docs/eval/calibration/mika-qa-2328/README.md`).

**Conséquence de périmètre :** ce plan ne peut pas « débloquer » une promotion
déjà faite d'un côté et déjà gouvernée ailleurs de l'autre. Toucher à la seconde
serait empiéter sur mika#2328.

#### R3 — Un verdict par une autre voie existe déjà, et il est gravé

`docs/solutions/best-practices/no-substrate-on-open-failure-mode-2026-08-30.md`
porte la mesure, dans le corps d'une règle :

> *mika-dev was swapped from glm-5.2 to glm-5.3 on 2026-08-26; the 2026-08-29
> measurement still found 102 of 120 sessions silent.*

Dans le vocabulaire du ticket, c'est **IGNORED** : le symptôme persiste après la
bascule. Ce verdict est postérieur au swap, il est daté, et il est déjà la
prémisse d'une doctrine en vigueur. Le rejeu prescrit produirait au mieux une
seconde version du même verdict — au prix de fixtures qui n'existent pas.

#### R4 — L'attribution demandée n'est plus atteignable par rejeu

Le ticket veut attribuer une différence **au modèle**. Or au moins trois autres
variables ont bougé depuis la mesure de référence :

| Changement | Effet sur la grandeur mesurée |
|---|---|
| **mika#1665** | La classification d'un « empty » a été affinée : `ReasoningBudgetExhausted` vs `EmptyResponse`. Un modèle à raisonnement dont le budget de sortie est épuisé rend un `content` vide **en étant sain** — le harnais l'affamait. |
| **mika#1996** | Le détecteur de cycle non-vide a été livré (`_measure_cycle_output` / `_gate_non_empty_cycle`), verdict à trois valeurs `produced`/`empty`/`undetermined`. |
| **mika#2296** | `llm_max_tokens` relevé — le facteur que mika#2328 nomme comme étouffant un modèle à raisonnement. |

Un rejeu aujourd'hui rendant « FIXED » ne dirait **pas** si c'est `glm-5.3` qui
répare ou l'un de ces trois changements. C'est la définition d'un résultat
inattribuable, et le ticket demande précisément une attribution.

#### R5 — La population mesurée ne tourne pas sous le modèle accusé

C'est le constat le plus lourd, et il est direct à vérifier. La mesure fondatrice
porte sur « les 120 dernières sessions claude-pilot ». Ces sessions tournent sous
**Claude**, pas sous GLM :

```bash
grep -o 'model [a-z0-9/.-]*' /var/log/claude-pilot/<recent>.log
# → model claude-opus-5           (session du 2026-09-20)
```

Et à la période même de la mesure :

```
[init] Session bb3fb047, model claude-opus-4-8[1m], task 8be926d9-…
                              ^^^^^^^^^^^^^^^^^^^   (session du 2026-08-21)
```

`claude-pilot` est la session d'**implémentation** ; `mika-dev` est l'agent
**dispatcheur**, et c'est lui qui porte GLM. Le ticket — et la formulation de la
mesure fondatrice — confondent les deux. **Le symptôme compté ne s'est pas
produit sous le modèle qu'on se propose d'innocenter ou d'accuser.**

#### R6 — Une cause concurrente domine cette population

Sur les sessions vides d'août, la forme observée n'est pas un silence du modèle
mais un refus d'outil suivi d'une erreur d'exécution :

```
[tool:request] Bash: echo "git-dir: $(git rev-parse --git-dir)"; …
[policy:deny]  Bash: echo "git-dir: $(git rev-parse --git-dir)"; …
[error] error_during_execution: [ede_diagnostic] result_type=user … stop_reason=tool_use
```

Ordres de grandeur sur le corpus complet (**2352 `.stderr`, recomptés le
2026-09-20** avec le motif ancré sur le producteur défini en U1) :

| Signal | Sessions |
|---|---|
| `[policy:deny]` présent | **1182** (~50 %) |
| `[guardrail] idle_timeout` | 402 |
| `[guardrail] stall_detected` | 10 |
| `[guardrail] error_max_turns` | 4 |
| `[guardrail] prompt_cache_dead` | 1 |
| **`[guardrail] empty_response`** | **0** |
| Ligne de configuration `emptyResponseThreshold=` | 2349 (~100 %) |

**La dernière ligne est le piège à graver.** Un script qui grepperait `empty`
sans distinguer la constante de l'événement rapporterait ~2349 « sorties vides »
— un nombre faux, plausible, et porté par l'autorité d'une mesure. C'est
exactement la forme de défaut que ce ticket existe pour ne pas reproduire. Le
piège a deux autres gueules, mesurées ce tour : `grep -i 'EmptyResponse'` matche
`emptyResponseThreshold` (2349 faux positifs), et `grep -F 'empty_response'`
matche le **contenu** des sessions — le symbole Rust `empty_response_result()`
lu ou édité par un pilote (3 sessions, toutes fausses).

**Rectification du « 4 » de la v1 de ce plan.** La v1 annonçait « événement
empty response réel — 4 sessions (~0,17 %) ». Ce chiffre **n'est reproductible
par aucun motif ancré sur le producteur** : les 4 sessions que le compte de
guardrails laisse hors des familles connues sont des `error_max_turns`
(2026-09-17 → 09-18), pas des `empty_response`. Le compte producteur-ancré de
l'événement est **0 sur 2352**. La v1 avait donc commis en petit la faute
qu'elle décrit — un nombre plausible tiré d'un grep non ancré — ce qui est
précisément la raison d'être de F1 et la justification de l'invariant U1
ci-dessous.

**Conséquence sur la conduite, pas seulement sur le chiffre :** un compte à zéro
sur l'événement rend l'**issue 1 de D4** (prémisse réfutée sur l'axe A) nettement
plus probable, sans la trancher — c'est U1, sur la population entière et avec sa
troisième valeur, qui conclut. Il ne faut surtout pas lire ce zéro comme « le
symptôme n'existe pas » : il dit que le symptôme **compté par ce motif-là** est
absent de cette population, ce qui est exactement la séparation d'axes de D3.

**Statut épistémique, énoncé sans le surjouer :** ces comptes restent des greps
d'orientation, pas la mesure — ancrés sur le producteur, donc bien meilleurs que
ceux de la v1, mais non fail-safe (ils n'ont pas de `undetermined`). Ils
suffisent à établir qu'une cause concurrente massive existe et que l'attribution
du ticket est fragile ; ils ne suffisent pas à conclure sur la cause de chaque
session. C'est U1 qui produit la mesure.

#### R7 — L'instrument réclamé existe et tourne

Le ticket demande de savoir si un cycle produit quelque chose. mika#1996 a livré
exactement cela, dans la boucle, sur **chaque** cycle, avec un verdict à trois
valeurs dont `undetermined` — la protection contre le faux rouge — et une garde
statique `CONTROL-MUST-BE-UNAVOIDABLE` qui refuse un troisième site de livraison
sans la porte. Il est falsifié contre le monde réel (40/40 sessions réelles) et
non contre sa propre définition.

**Ce qui manque n'est donc pas un détecteur, c'est une lecture datée et
reproductible de ce qu'il voit** — et c'est là que le corpus est décisif.

### Le corpus qui existe est plus fort que les fixtures qui manquent

| | Fixtures prescrites | Corpus disponible |
|---|---|---|
| Existence | **aucune** | `/var/log/claude-pilot/` — **5015 fichiers** |
| Étendue | 2 sessions | 2352 sessions avec `.stderr` (au 2026-09-20) |
| Période | 2026-08-18 | **2026-03-18 → 2026-09-20** |
| Couverture calendaire | un point | **de part et d'autre du 2026-08-26** — une couverture, pas un contraste de régime : par R5 la population tourne sous Claude des deux côtés (F2) |
| Nature | proxy, jugé inadéquat par le ticket lui-même | boucle d'outils **réelle**, en production |

Le ticket reproche au proxy de ne pas solliciter la boucle à 30 tours. Le corpus
**est** cette boucle, sur trois mois, en production. La mesure que le rejeu
devait approcher est disponible sur une population trois ordres de grandeur plus
grande.

**Ce que le corpus n'est pas, et il faut le dire ici plutôt que le laisser
espérer :** il ne contient **pas** « le contrôle et le candidat dans le même jeu
de données ». Par R5, ces sessions tournent sous Claude de part et d'autre du
2026-08-26 ; le candidat GLM n'y figure sur aucune ligne. Ce que le corpus offre
est une mesure **du symptôme** sur une population réelle et étendue, pas une
comparaison de modèles — et c'est précisément la raison d'être de la séparation
d'axes (D3) et de l'avertissement calendaire (F2, dans U1).

### Contrainte d'accès, nommée plutôt que contournée

La moitié **GLM** de la question — les tours de `mika-dev` lui-même, via
`turn_usage` et `llm_budget_resolved` — vit dans `$MIKA_SPIRIT_LOG_FILE`, qui
n'est **pas lisible depuis cette session** (`/var/log/mika/` n'existe pas ici, et
`~/.mika/` n'expose que `data` et `state`). Sa lecture est un **geste opérateur**
sur l'hôte. Le plan la prescrit et ne la simule pas.

---

## Planning Contract

### D1 — Aucun rejeu n'est tenté, et le refus est motivé, pas commode

Les fixtures n'existent pas (R1) et ne sont pas reconstructibles. Même
disponibles, leur rejeu serait inattribuable (R4) et porterait sur le mauvais
modèle (R5). **Fabriquer des fixtures « équivalentes » est explicitement
refusé** : une fixture construite depuis la définition du symptôme ne peut pas
réfuter cette définition — c'est la règle 6 de
`cycle-non-empty-detector-2026-08-30.md` (*falsifier contre le monde, pas contre
la définition*), et le corpus réel est précisément ce qu'elle prescrit.

### D2 — Le livrable est une mesure reproductible, et c'est pourquoi il y a un script

Le réflexe serait de mesurer à la main et d'écrire le résultat. **Refusé**, sur
un précédent mesuré : la mesure du 2026-08-29 n'a laissé **aucun geste
reproductible** — elle est citée dans un doc de principe, sans commande, sans
artefact. C'est très exactement ce qui a laissé mika#1950 ouvert un mois : la
question « le symptôme a-t-il bougé ? » n'était pas re-posable sans refaire le
travail.

Le coût est nommé : un script versé est un engagement de maintenance, et il lit
un format de log produit par un dépôt tiers (`claude-pilot`). Il est donc
délibérément **borné à la lecture**, sans dépendance, et **fail-safe** : une
session dont un signal est illisible est classée `undetermined`, jamais rangée
dans l'une des deux autres catégories — la troisième valeur de mika#1996, pour la
raison qu'elle y porte déjà.

### D3 — Le verdict est rendu **par axe**, et un axe non mesurable est dit tel

Le ticket demande un verdict unique. La mesure en supporte deux populations
distinctes, et les confondre serait refaire l'erreur R5 :

| Axe | Population | Source | Mesurable ici ? |
|---|---|---|---|
| **A** — cycles pilote vides | sessions `claude-pilot` | `/var/log/claude-pilot/*.stderr` | **oui** |
| **B** — tours GLM vides | tours de `mika-dev` | `$MIKA_SPIRIT_LOG_FILE` | **non — geste opérateur** |

L'axe A est mesuré et conclu. L'axe B est **prescrit** avec ses commandes et ses
haltes. Rendre un verdict global en ne mesurant que A, et en le présentant comme
un verdict sur GLM, serait la faute que ce plan documente.

### D4 — Trois issues sont prévues, et aucune n'est promise

Un plan qui promet « le verdict sera FIXED/IGNORED/AGGRAVER » promet un résultat
de mesure. Les trois issues et leur conduite sont fixées **avant** la mesure :

1. **Le symptôme a une cause dominante non-modèle** (attendu si R6 se confirme) →
   le verdict demandé est **sans objet** sur l'axe A, et il faut le dire ainsi :
   « la population ne discrimine pas le modèle ». Le ticket est répondu par une
   réfutation de sa prémisse, ce qui est une réponse.
2. **Le symptôme discrimine les périodes** → verdict FIXED/IGNORED/AGGRAVER rendu
   sur l'axe A, avec sa réserve d'attribution (R4) écrite dans la même phrase.
   **Condition ajoutée en rev 2 (F2) :** la discrimination périodique ne suffit
   pas à elle seule. L'axe temporel est calendaire, pas un axe de régime, donc
   une discontinuité au 2026-08-26 sans canal causal établi retombe dans
   l'issue 1 — et le canal candidat (pilote comme capteur aval du dispatcheur)
   n'est prouvable que sur l'axe B, non mesuré ici.
3. **Le corpus ne tranche pas** (signaux illisibles, `undetermined` majoritaire) →
   c'est un résultat, pas un échec : il nomme ce qu'il faudrait instrumenter.

### D5 — Rien n'est promu, rien n'est fermé

Aucun `zai_model` / `openrouter_model` ne bouge. mika#1910 ne se ferme pas :
KTD5 du plan mika#1996 l'a déjà tranché (*« Fermer sur la foi d'une mitigation que
la mesure contredit, c'est refaire le geste que ce ticket corrige »*), et la
fermeture reste un geste opérateur. La procédure mika#2328 n'est pas touchée.

### D6 — La correction des docs est **conditionnée** à la mesure

`no-substrate-on-open-failure-mode-2026-08-30.md` attribue le symptôme à
`glm-5.2` sur une population qui tourne sous Claude (R5). C'est une correction
qui s'impose **si** U1 le confirme sur la population entière. Corriger un doc de
principe sur la foi d'un grep exploratoire serait commettre en petit la faute
qu'on corrige. U4 est donc conditionnel et porte son critère.

---

## Implementation Units

### U1 — `scripts/measure-pilot-cycle-emptiness` (le geste reproductible)

Script de lecture seule sur `/var/log/claude-pilot/*.stderr`. Pour chaque session,
une ligne JSON :

| Champ | Source | Rôle |
|---|---|---|
| `task_id` | nom de fichier | jointure |
| `date` | mtime | **axe calendaire** (voir la note ci-dessous — ce n'est *pas* un axe « régime 5.2 / 5.3 ») |
| `model` | `[init] … model <X>` | **le discriminant de R5** |
| `tool_calls` | `grep -c '\[tool:request\]'` | la grandeur de la mesure fondatrice |
| `policy_denies` | `grep -c '\[policy:deny\]'` | **le discriminant de R6** |
| `empty_event` | ligne `[guardrail] empty_response:` — règle littérale ci-dessous | le symptôme |
| `guardrail_aborts` | liste des `<type>` de toutes les lignes `[guardrail]` | les causes concurrentes, nommées plutôt qu'agrégées |
| `error_during_execution` | présence | cause concurrente |
| `verdict` | `produced` / `empty` / `undetermined` | vocabulaire mika#1996 |

#### La règle d'extraction de `empty_event`, écrite littéralement (F1)

**Elle est dérivée du producteur, jamais rétro-ingénierée depuis le log.** Le
seul site qui produit cet événement est, dans le dépôt `claude-pilot` :

- `src/claude_pilot/guardrails.py` — `self._abort("empty_response", f"{n} consecutive trivial responses (<10 chars)")`,
  déclenché par `_consecutive_empty_turns >= config.emptyResponseThreshold` ;
- `src/claude_pilot/ui.py:112-113` — `log_guardrail(type_, detail)` écrit
  `f"\n{ORANGE}[guardrail]{RESET} {BOLD}{type_}{RESET}: {detail}"`.

La ligne brute sur disque est donc, échappements visibles :

```
^[[38;5;208m[guardrail]^[[0m ^[[1mempty_response^[[0m: 5 consecutive trivial responses (<10 chars)
```

**Règle en deux temps, et l'ordre est portant :**

1. **Dépouiller les séquences ANSI CSI de la ligne** (`\x1b\[[0-9;]*m`) *avant*
   toute mise en correspondance. Un motif écrit sur la ligne colorée devrait
   coder `^[[1m` entre `[guardrail]` et le type — fragile au moindre changement
   de thème de `ui.py`, et illisible dans le script.
2. Sur la ligne dépouillée, apparier l'ancre
   **`[guardrail] empty_response:`** (crochets littéraux, un espace, deux-points
   collés au type). `empty_event` est vrai si et seulement si au moins une ligne
   de la session apparie cette ancre.

**Ce que l'ancre exclut, et c'est le contrôle négatif d'AC9 :**
`emptyResponseThreshold=5` (la constante, 2349 sessions), `EmptyResponse`
(variante de casse de la même constante), et `empty_response_result()` (symbole
Rust apparaissant dans le *contenu* d'une session, 3 sessions). Aucun des trois
ne porte le préfixe `[guardrail] ` ni les deux-points collés.

**`guardrail_aborts` est extrait par la même ancre généralisée**
(`[guardrail] <type>:`), le `<type>` étant capturé et non présumé. C'est ce qui
rend le champ honnête : les familles observées au 2026-09-20 sont
`idle_timeout`, `stall_detected`, `error_max_turns`, `prompt_cache_dead` — et
`error_max_turns` **n'est pas** dans la liste `Literal[...]` de `_abort`, donc il
existe au moins un second site d'émission dans `claude-pilot`. Un type inconnu
est **enregistré tel quel**, jamais rangé dans une famille connue ni jeté
silencieusement.

**Invariants portants :**

- **La constante n'est jamais comptée comme l'événement.** Un test négatif est
  livré avec le script : un log ne portant que `emptyResponseThreshold=5` doit
  rendre `empty_event: false`. Sans lui, le script rapporte ~2349 faux positifs
  et personne ne le voit (R6).
- **Le test négatif borne une frontière connue, pas une frontière devinée**
  (F1). Il porte les **trois** leurres mesurés ci-dessus — `emptyResponseThreshold=5`,
  `EmptyResponse`, `empty_response_result()` — et un **contrôle positif** sur la
  ligne littérale ANSI complète copiée du producteur. Le négatif seul serait
  satisfait par un script qui ne dit jamais `true` ; c'est la même asymétrie que
  le §1 du Verification Contract.
- **Fail-safe vers `undetermined`.** Modèle illisible, fichier tronqué, mtime
  absent → `undetermined`, jamais `produced` ni `empty`. Un signal qu'on ne peut
  pas lire n'est jamais un terme satisfait.
- **Lecture seule.** Aucun écrit hors du fichier de sortie passé en argument.
- **Le vocabulaire des verdicts est celui de mika#1996**, pas un troisième
  dialecte : deux orthographes d'un même verdict couperaient une population en
  deux sans le dire.

#### L'axe temporel est calendaire, et le 2026-08-26 n'y est pas une frontière causale (F2)

La v1 étiquetait `date` comme axe « régime 5.2 / 5.3 », frontière au 2026-08-26.
**C'était une contradiction interne avec R5** : la population de l'axe A tourne
sous Claude avant *et* après cette date, donc la bascule GLM n'a changé le modèle
d'aucune de ces sessions. Étiqueter la date « régime » ré-assumait en silence
exactement ce que R5 refuse d'assumer, et AC5 aurait pu rendre
FIXED/IGNORED/AGGRAVER sur une frontière décorative.

**Le champ est donc une pure dérive calendaire.** Le script le produit comme un
axe de découpage neutre ; il ne porte aucune sémantique de régime, et le README
(U3 §3) doit le dire dans les mêmes termes.

**Le canal causal qui rendrait le 2026-08-26 significatif est nommé comme
hypothèse, et il n'est pas établi ici :** une session pilote est un **capteur
aval** du dispatcheur — si `mika-dev` (qui, lui, porte GLM) se tait ou dispatche
mal, le pilote peut démarrer sans travail et paraître « vide » sans qu'aucun de
ses propres tours ne le soit. Sous cette hypothèse seulement, une discontinuité
au 2026-08-26 sur l'axe A serait imputable au swap. **Trois raisons de ne pas la
poser comme acquise :** elle est non mesurée ici ; elle prédit un silence côté
*dispatch*, donc sa preuve vit sur l'**axe B** (`$MIKA_SPIRIT_LOG_FILE`), qui
n'est pas lisible depuis cette session ; et la cause concurrente de R6
(`policy:deny`, ~50 %) suffirait à produire une discontinuité calendaire pour des
raisons de politique de permissions, sans aucun rapport avec le modèle.

**Conduite qui en découle, et elle est contraignante :** une discontinuité
observée au voisinage du 2026-08-26 sur l'axe A **ne vaut pas** verdict
FIXED/IGNORED/AGGRAVER. Elle relève de l'**issue 1 de D4** — la population ne
discrimine pas le modèle — et le README doit la rapporter comme une corrélation
calendaire dont le canal causal reste à établir sur l'axe B. Rendre un verdict
sur cette seule discontinuité serait la faute R5 commise une seconde fois, par
la date au lieu du modèle.

### U2 — L'artefact de mesure daté

`docs/eval/mika-1950/measurement-2026-09-20.json` (sortie brute) et sa lecture
agrégée. Agrégations minimales : distribution par **mois × modèle × verdict**, et
la part de `policy_denies > 0` parmi les `empty`. C'est cette dernière colonne qui
tranche entre l'issue 1 et l'issue 2 de D4.

### U3 — `docs/eval/mika-1950/README.md` (protocole + verdict)

Sur le modèle de `docs/eval/calibration/mika-qa-2328/README.md` : un document
qui **produit** sa conclusion sur une sortie machine et porte ses haltes. Sections :

1. Ce que le ticket prescrivait, et pourquoi ce n'est pas exécutable (R1–R7,
   avec les commandes de vérification).
2. **Le chemin de retour posé par l'opérateur, et pourquoi il n'est pas pris**
   (voir ci-dessous — F4).
3. La désambiguïsation des deux axes (D3), **et l'avertissement F2** : l'axe
   calendaire n'est pas un axe de régime, une discontinuité au 2026-08-26 ne
   vaut pas verdict.
4. **Axe A** — la mesure, l'artefact, le verdict rendu ou la prémisse réfutée.
5. **Axe B** — la procédure opérateur (commandes ci-dessous), non exécutée ici.
6. Les haltes.
7. Ce que ce travail **n'achète pas**.

**§2 — le chemin de retour de l'opérateur, nommé plutôt que contourné (F4).**
Le commentaire opérateur du 2026-09-20 sur #1950 pose un retour explicite :
*« Retour = reposer `post-launch`, retirer `ready` »* — c'est-à-dire re-parquer
le ticket plutôt que l'exécuter. Ce plan **ne le prend pas**, et le README doit
dire pourquoi plutôt que de laisser le lecteur croire que l'option a été
ignorée. La raison : re-parquer ne produit rien. Le ticket est resté ouvert un
mois précisément parce que la question « le symptôme a-t-il bougé ? » n'était
pas re-posable sans refaire le travail (D2) ; le re-parquer une seconde fois
reconduirait cet état en le datant. La rectification, elle, produit la mesure
que le ticket visait — sur une population trois ordres de grandeur plus grande
que les fixtures absentes — et laisse le ticket dans un état où sa question est
**décidable par une commande**. Le choix est réversible et le README le dit :
si l'opérateur juge que la rectification sort du périmètre qu'il avait en tête,
son geste reste disponible à l'identique (reposer `post-launch`, retirer
`ready`), et les artefacts U1/U2 gardent leur valeur indépendamment — ils ne
présupposent rien du statut du ticket.

Procédure de l'axe B, à exécuter sur l'hôte :

```bash
# Quel modèle tourne réellement pour mika-dev, et par quelle porte
grep llm_budget_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-dev")
        | {provider, model, model_source, model_config_key,
           http_timeout_secs, agent_total_timeout_secs, http_source}'

# Les tours vides : contenu vide AVEC budget de sortie saturé = mika#1665,
# pas un silence du modèle. Le discriminant est stop_reason, pas le vide.
grep turn_usage "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-dev")
        | {step, stop_reason, output_tokens, tool_use_in_turn, status,
           model, request_bytes}'
```

### U4 — Correction d'attribution (**conditionnelle**, cf. D6)

**Si et seulement si** U2 confirme sur la population entière que les sessions
comptées tournent sous Claude : corriger la phrase d'attribution de
`no-substrate-on-open-failure-mode-2026-08-30.md` pour qu'elle nomme la
population réellement mesurée. La **règle** du document ne bouge pas — elle est
juste et indépendante du modèle ; c'est son exemple chiffré qui doit dire de quoi
il est l'exemple. Si U2 ne confirme pas, U4 n'est pas exécutée et le README le dit.

### U5 — Note de statut sur mika#1910 (geste opérateur, non exécuté ici)

Le README fournit le texte de la note : ce qui a été mesuré, sur quelle
population, ce qui reste ouvert. **Pas de fermeture** (D5).

---

## Fire-Disposition

| Unité | Livrée par ce plan | Nature |
|---|---|---|
| U1 script + test négatif | oui | code (shell, lecture seule) |
| U2 artefact | oui | donnée datée |
| U3 README | oui | doc décisionnel |
| U4 correction d'attribution | **conditionnelle** | doc, sous critère U2 |
| U5 note mika#1910 | texte fourni, **pose = opérateur** | — |
| Axe B (tours GLM) | **prescrit, non exécuté** | geste opérateur |
| Swap de modèle | **non** | hors périmètre (D5) |

---

## Verification Contract

1. **Anti-vacuité du script, dans les deux sens.** Un log avec 33 tool calls rend
   `produced` ; un log sans aucun rend `empty` ; un log tronqué rend
   `undetermined`. Les trois sont testés. La direction positive est portante :
   un script qui ne saurait que dire `empty` serait satisfait par « toujours
   empty » et ne vaudrait rien.
2. **Le contrôle négatif de R6 est un test.** Un log ne portant que
   `emptyResponseThreshold=5` rend `empty_event: false`. C'est le test dont
   l'absence rendrait la mesure fausse sans la rendre rouge.
3. **Le script est falsifié contre le monde** (règle 6 de mika#1996) : ses
   `tool_calls` sont recoupés par un `grep -c` direct sur un échantillon de
   sessions réelles, dont au moins une session productive connue.
4. **Recoupement de la mesure fondatrice.** Restreint aux 120 sessions les plus
   récentes du 2026-08-29, le script doit retrouver un ordre de grandeur
   compatible avec 102/120. **Halte si non** : c'est le script qui est faux, pas
   l'histoire — le réparer avant toute conclusion.
5. **Aucune valeur de production n'est modifiée** : `git diff` ne touche ni
   `well_known_agents.rs`, ni un `config.toml`, ni `docs/eval/calibration/mika-qa-2328/`.
6. **Précondition R1 — halte si la prémisse porteuse tombe (F3).** Les quatre
   commandes de vérification de R1 sont rejouées **avant toute autre unité**, et
   leur résultat attendu est le vide. **Si l'une d'elles retourne non-vide :
   halte.** Les fixtures existent, donc le rejeu prescrit redevient exécutable,
   donc la rectification n'est plus fondée et doit être **re-plaidée** avant U1 —
   pas contournée, pas notée en passant. Cette halte existe parce que l'espace
   de recherche peut être incomplet : le corps de mika#1910 affirme que des
   traces ont été « mergé[e]s sur mika-platform via PR#192 », et une PR d'un
   autre dépôt du workspace n'est pas couverte par un `git log --all` local.
   C'est la même discipline que le §4 ci-dessus, appliquée à la prémisse au lieu
   du script : sans elle, un R1 faux découvert à mi-implémentation n'échouerait
   qu'implicitement, au moment d'écrire un corps de PR devenu impossible à
   rédiger honnêtement.

---

## Definition of Done

- [ ] Précondition R1 rejouée et vide (Verification Contract §6) ; sinon halte.
- [ ] `scripts/measure-pilot-cycle-emptiness` livré, lecture seule, `empty_event`
      extrait par l'ancre `[guardrail] empty_response:` après dépouillement ANSI,
      avec ses tests : négatif sur les trois leurres, positif sur la ligne
      littérale du producteur.
- [ ] `docs/eval/mika-1950/measurement-2026-09-20.json` versé.
- [ ] `docs/eval/mika-1950/README.md` : R1–R7 avec leurs commandes, le chemin de
      retour opérateur et la raison de ne pas le prendre, les deux axes séparés,
      l'avertissement sur l'axe calendaire, le verdict de l'axe A **ou** la
      réfutation motivée de la prémisse, la procédure de l'axe B, les haltes.
- [ ] U4 exécutée **ou** son non-déclenchement écrit dans le README avec le
      critère qui ne s'est pas réalisé.
- [ ] Texte de la note mika#1910 fourni ; ticket **non fermé**.
- [ ] Aucun modèle promu ; `MIKA_DEV_CONFIG` et `MIKA_QA_CONFIG` inchangés.
- [ ] Le corps de PR nomme que la lettre du ticket n'est pas exécutable, et
      pourquoi (mika#2211 : corps écrit sous le worktree).

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de ses trois demandes numérotées, de sa
condition bloquante, et des faits R1–R7.

- **AC1** — L'inexécutabilité du rejeu prescrit est **établie et vérifiable** :
  le README porte les commandes qui montrent l'absence des fixtures dans le
  dépôt, dans `git log --all`, et dans le workspace. Les quatre commandes sont
  **rejouées avant toute autre unité**, et le README nomme la halte si l'une
  d'elles retourne non-vide (Verification Contract §6).
- **AC2** — Les deux axes (cycles pilote / tours GLM) sont **nommés et séparés**,
  et le README dit lequel est mesuré ici et lequel est un geste opérateur.
- **AC3** — Le modèle réellement en service dans la population mesurée est
  **établi par la donnée** (champ `model` de l'artefact), pas supposé.
- **AC4** — La part des sessions vides portant un `policy:deny` est **chiffrée** :
  c'est elle qui décide si le verdict demandé a un objet.
- **AC5** — Un verdict FIXED/IGNORED/AGGRAVER est rendu sur l'axe A **ou** la
  prémisse est réfutée par écrit ; dans les deux cas la réserve d'attribution
  (R4 : trois changements de substrat) figure dans la même section. **Un verdict
  ne peut pas reposer sur la seule discontinuité calendaire du 2026-08-26** :
  l'axe temporel est calendaire et non un axe de régime (F2), et une
  discontinuité sans canal causal établi relève de l'issue 1 de D4.
- **AC6** — La mesure est **reproductible** : une commande unique la refait.
- **AC7** — La condition bloquante est traitée explicitement : le README constate
  son franchissement des deux côtés et renvoie mika-qa à mika#2328.
- **AC8** — **Aucune promotion, aucune fermeture** : ni `glm-5.3` vers mika-dev ou
  mika-qa, ni fermeture de mika#1910.
- **AC9** — Le script ne confond jamais la constante avec l'événement, et des
  tests le prouvent sur une **frontière connue** (F1) : l'ancre est
  `[guardrail] empty_response:` après dépouillement ANSI, dérivée du producteur
  (`claude-pilot` `guardrails.py::_abort` → `ui.py::log_guardrail`) ; le test
  négatif porte les trois leurres mesurés (`emptyResponseThreshold=5`,
  `EmptyResponse`, `empty_response_result()`) et un contrôle positif porte la
  ligne ANSI littérale du producteur.
- **AC10** — Le chemin de retour posé par l'opérateur (« reposer `post-launch`,
  retirer `ready` ») est **nommé dans le README** avec la raison de ne pas le
  prendre et le constat qu'il reste disponible à l'identique (F4).

---

## Risks

| Risque | Conduite |
|---|---|
| **Le verdict demandé n'a pas d'objet sur l'axe A** (issue 1 de D4). | C'est le résultat le plus probable et il **répond** au ticket en réfutant sa prémisse. Ne pas fabriquer un verdict pour remplir la case. |
| **Le format du log claude-pilot dérive** (dépôt tiers). | Le script échoue vers `undetermined`, jamais vers une valeur fausse. Un taux d'`undetermined` élevé est lui-même le signal, et il est compté. |
| **Attribution abusive dans l'autre sens** — conclure « GLM est innocent » alors que l'axe B n'est pas mesuré. | D3 l'interdit structurellement : le verdict est par axe, et l'axe B est déclaré non mesuré ici. |
| **Le recoupement avec 102/120 échoue.** | Halte du Verification Contract §4 : réparer le script avant toute conclusion. Un script qui ne retrouve pas une mesure connue ne peut pas en produire une nouvelle. |
| **U4 corrige un doc de principe à tort.** | Conditionnée (D6) à une confirmation sur la population entière ; la règle du doc ne bouge jamais, seulement son exemple chiffré. |
| **Empiéter sur mika#2328.** | Aucun fichier sous `docs/eval/calibration/mika-qa-2328/` n'est touché ; le README y renvoie. |
| **Lire la date comme un régime** (F2) — conclure au modèle sur une discontinuité au 2026-08-26 alors que la population tourne sous Claude des deux côtés. | Le champ est étiqueté calendaire dans le script, dans le README et dans AC5 ; le canal causal candidat est écrit comme hypothèse non établie, et sa preuve est renvoyée à l'axe B. |
| **L'ancre `empty_event` sur- ou sous-matche sans rougir** (F1) — un motif choisi par rétro-ingénierie passerait le test négatif tout en comptant faux. | L'ancre est dérivée du producteur `claude-pilot` et citée avec son site ; le test négatif porte les trois leurres **mesurés**, et un contrôle positif porte la ligne littérale. |
| **R1 est faux** — les fixtures existent quelque part (p. ex. via PR#192 sur un autre dépôt du workspace). | Verification Contract §6 : précondition rejouée avant toute unité, halte et re-plaidoirie de la rectification. Jamais un contournement silencieux. |

---

## Ce que ce travail n'achète pas

- **Il ne corrige pas la cause du symptôme.** Il la mesure et la départage.
- **Il ne rend pas `glm-5.3` promouvable.** Pour mika-qa, la porte est mika#2328
  et sa condition conjonctive ; pour mika-dev, la bascule est déjà faite et sa
  réconciliation dépôt ↔ runtime est son propre travail (mika#2296 D2).
- **Il ne mesure pas les tours GLM.** L'axe B est prescrit, pas exécuté : le
  journal spirit n'est pas lisible depuis une session de dispatch.
- **Il ne reconstitue pas la campagne hy3-v0.** Ces artefacts sont perdus.

---

## Sources

- `docs/solutions/best-practices/no-substrate-on-open-failure-mode-2026-08-30.md`
  — la mesure du 2026-08-29 et la règle qu'elle fonde.
- `docs/solutions/best-practices/cycle-non-empty-detector-2026-08-30.md`
  — le détecteur, son vocabulaire à trois valeurs, et la règle 6 (falsifier
  contre le monde).
- `docs/solutions/best-practices/calibration-reasoning-budget-exhaustion-false-empty-2026-06-30.md`
  — un « empty » peut être une famine de budget sur un modèle à raisonnement.
- `docs/eval/calibration/mika-qa-2328/README.md` — la forme d'un protocole
  décidable sur sortie machine, et la porte qui gouverne mika-qa.
- `crates/mika-agent/src/well_known_agents.rs:165-258` — les deux dérives
  dépôt ↔ runtime, documentées dans le code.
- `skills/bundled/_shared/dispatch-lib.sh` — `_measure_cycle_output`,
  `_gate_non_empty_cycle`, `PILOT_RAN`.
- `/var/log/claude-pilot/` — 5015 fichiers, 2026-03-18 → 2026-09-20 ; 2352
  `.stderr`.
- `claude-pilot` `src/claude_pilot/guardrails.py` (`_abort("empty_response", …)`,
  seuil `emptyResponseThreshold`) et `src/claude_pilot/ui.py:112-113`
  (`log_guardrail` — le format littéral de la ligne) — **le producteur dont
  l'ancre de `empty_event` est dérivée** (F1).
- Commentaire opérateur du 2026-09-20 sur mika#1950
  (`IC_kwDORWsgGM8AAAABVsWYCg`) — le chemin de retour « reposer `post-launch`,
  retirer `ready` » (F4).
- Plan mika#1996 (`docs/plans/2026-08-30-002-feat-1996-cycle-non-vide-detecteur-plan.md`)
  — KTD5 : ne pas fermer mika#1910 sans mesure fraîche.

---

## Revision history

- **2026-09-20** — v1. Plan initial. Le rejeu prescrit est établi inexécutable
  (fixtures absentes de tout l'historique) ; le ticket est recadré sur la mesure
  qu'il visait, avec séparation des deux axes et refus explicite de produire un
  verdict inattribuable.
- **2026-09-20** — rev 2. Quatre findings de la première passe architecte
  adressés, tous les quatre.
  - **F1 (bloquant)** — la règle d'extraction d'`empty_event` est désormais
    écrite littéralement dans U1 et **dérivée du producteur**
    (`claude-pilot` `guardrails.py::_abort("empty_response", …)` →
    `ui.py::log_guardrail`), avec la ligne ANSI brute, l'ancre en deux temps
    (dépouillement CSI puis `[guardrail] empty_response:`), et les trois leurres
    qu'elle exclut. Le test négatif d'AC9 borne donc une frontière **connue**, et
    un contrôle positif sur la ligne littérale est ajouté pour couvrir
    l'asymétrie. Effet de bord porteur : en établissant l'ancre, **le « 4 » de
    R6 s'est révélé non reproductible** — les 4 sessions concernées sont des
    `error_max_turns`, et le compte producteur-ancré de l'événement est **0 sur
    2352**. R6 est rectifié, recompté au 2026-09-20, et la v1 est nommée comme
    ayant commis en petit la faute qu'elle décrivait.
  - **F2** — l'axe temporel est **réétiqueté en dérive calendaire** (branche 2
    du change required). Le canal causal candidat (pilote comme capteur aval du
    dispatcheur) est énoncé explicitement, mais **comme hypothèse non établie**,
    avec les trois raisons de ne pas la poser comme acquise et le renvoi de sa
    preuve à l'axe B. AC5 est durci en conséquence : une discontinuité au
    2026-08-26 ne vaut pas verdict et relève de l'issue 1 de D4.
  - **F3** — Verification Contract §6 : la précondition R1 est rejouée avant
    toute unité, avec **halte et re-plaidoirie** si une commande retourne
    non-vide ; la raison (PR#192, espace de recherche possiblement incomplet)
    est écrite. AC1 et la Definition of Done le portent, et un risque nommé y
    renvoie.
  - **F4** — U3 gagne une section §2 dédiée nommant le chemin de retour
    opérateur (« reposer `post-launch`, retirer `ready` », commentaire
    `IC_kwDORWsgGM8AAAABVsWYCg`), la raison de ne pas le prendre (re-parquer ne
    produit rien et reconduirait l'état qui a laissé le ticket ouvert un mois),
    et le constat qu'il reste disponible à l'identique. Nouvel **AC10**.
  - Aucun AC n'est affaibli ; AC1, AC5 et AC9 sont resserrés, AC10 est ajouté.
