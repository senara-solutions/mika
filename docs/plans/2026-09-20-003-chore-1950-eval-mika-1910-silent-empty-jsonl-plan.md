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

Ordres de grandeur sur le corpus complet (2351 `.stderr`) :

| Signal | Sessions |
|---|---|
| `policy:deny` présent | **1181** (~50 %) |
| Événement « empty response » réel | **4** (~0,17 %) |
| Ligne de configuration `emptyResponseThreshold=` | 2348 (~100 %) |

**La troisième ligne est le piège à graver.** Un script qui grepperait `empty`
sans distinguer la constante de l'événement rapporterait ~2348 « sorties vides »
— un nombre faux, plausible, et porté par l'autorité d'une mesure. C'est
exactement la forme de défaut que ce ticket existe pour ne pas reproduire.

**Statut épistémique, énoncé sans le surjouer :** ces comptes sont des greps
d'orientation, pas la mesure. Ils suffisent à établir qu'une cause concurrente
massive existe et que l'attribution du ticket est fragile ; ils ne suffisent pas
à conclure sur la cause de chaque session. C'est U1 qui produit la mesure.

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
| Étendue | 2 sessions | 2351 sessions avec `.stderr` |
| Période | 2026-08-18 | **2026-03-18 → 2026-09-20** |
| Régimes couverts | un | **les deux** (avant/après le 2026-08-26) |
| Nature | proxy, jugé inadéquat par le ticket lui-même | boucle d'outils **réelle**, en production |

Le ticket reproche au proxy de ne pas solliciter la boucle à 30 tours. Le corpus
**est** cette boucle, sur trois mois, avec le contrôle et le candidat dans le même
jeu de données. La mesure que le rejeu devait approcher est disponible sur une
population trois ordres de grandeur plus grande.

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
| `date` | mtime | axe temporel (régime 5.2 / 5.3) |
| `model` | `[init] … model <X>` | **le discriminant de R5** |
| `tool_calls` | `grep -c '\[tool:request\]'` | la grandeur de la mesure fondatrice |
| `policy_denies` | `grep -c '\[policy:deny\]'` | **le discriminant de R6** |
| `empty_event` | événement réel, **jamais** `emptyResponseThreshold=` | le symptôme |
| `error_during_execution` | présence | cause concurrente |
| `verdict` | `produced` / `empty` / `undetermined` | vocabulaire mika#1996 |

**Invariants portants :**

- **La constante n'est jamais comptée comme l'événement.** Un test négatif est
  livré avec le script : un log ne portant que `emptyResponseThreshold=5` doit
  rendre `empty_event: false`. Sans lui, le script rapporte ~2348 faux positifs
  et personne ne le voit (R6).
- **Fail-safe vers `undetermined`.** Modèle illisible, fichier tronqué, mtime
  absent → `undetermined`, jamais `produced` ni `empty`. Un signal qu'on ne peut
  pas lire n'est jamais un terme satisfait.
- **Lecture seule.** Aucun écrit hors du fichier de sortie passé en argument.
- **Le vocabulaire des verdicts est celui de mika#1996**, pas un troisième
  dialecte : deux orthographes d'un même verdict couperaient une population en
  deux sans le dire.

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
2. La désambiguïsation des deux axes (D3).
3. **Axe A** — la mesure, l'artefact, le verdict rendu ou la prémisse réfutée.
4. **Axe B** — la procédure opérateur (commandes ci-dessous), non exécutée ici.
5. Les haltes.
6. Ce que ce travail **n'achète pas**.

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

---

## Definition of Done

- [ ] `scripts/measure-pilot-cycle-emptiness` livré, lecture seule, avec son test
      négatif sur la constante de configuration.
- [ ] `docs/eval/mika-1950/measurement-2026-09-20.json` versé.
- [ ] `docs/eval/mika-1950/README.md` : R1–R7 avec leurs commandes, les deux
      axes séparés, le verdict de l'axe A **ou** la réfutation motivée de la
      prémisse, la procédure de l'axe B, les haltes.
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
  dépôt, dans `git log --all`, et dans le workspace.
- **AC2** — Les deux axes (cycles pilote / tours GLM) sont **nommés et séparés**,
  et le README dit lequel est mesuré ici et lequel est un geste opérateur.
- **AC3** — Le modèle réellement en service dans la population mesurée est
  **établi par la donnée** (champ `model` de l'artefact), pas supposé.
- **AC4** — La part des sessions vides portant un `policy:deny` est **chiffrée** :
  c'est elle qui décide si le verdict demandé a un objet.
- **AC5** — Un verdict FIXED/IGNORED/AGGRAVER est rendu sur l'axe A **ou** la
  prémisse est réfutée par écrit ; dans les deux cas la réserve d'attribution
  (R4 : trois changements de substrat) figure dans la même section.
- **AC6** — La mesure est **reproductible** : une commande unique la refait.
- **AC7** — La condition bloquante est traitée explicitement : le README constate
  son franchissement des deux côtés et renvoie mika-qa à mika#2328.
- **AC8** — **Aucune promotion, aucune fermeture** : ni `glm-5.3` vers mika-dev ou
  mika-qa, ni fermeture de mika#1910.
- **AC9** — Le script ne confond jamais `emptyResponseThreshold=` avec un
  événement, et un test le prouve.

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
- `/var/log/claude-pilot/` — 5015 fichiers, 2026-03-18 → 2026-09-20.
- Plan mika#1996 (`docs/plans/2026-08-30-002-feat-1996-cycle-non-vide-detecteur-plan.md`)
  — KTD5 : ne pas fermer mika#1910 sans mesure fraîche.

---

## Revision history

- **2026-09-20** — v1. Plan initial. Le rejeu prescrit est établi inexécutable
  (fixtures absentes de tout l'historique) ; le ticket est recadré sur la mesure
  qu'il visait, avec séparation des deux axes et refus explicite de produire un
  verdict inattribuable.
