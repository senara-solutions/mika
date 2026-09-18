# mika#2328 — mika-qa sur glm-5.3 : le dormeur ne peut pas se réveiller tant que rien ne dit ce qui tourne

**Ticket :** senara-solutions/mika#2328
**Type :** fix (substrat d'observabilité + enregistrement d'une décision opérateur)
**Date :** 2026-09-18

---

## Contexte

Le ticket est un **dormeur**. Il constate (2026-09-15) que mika-qa sous
**glm-5.3 / Z.AI direct** a dépassé son enveloppe de revue sur la PR #2327 —
tours de 258–315 s, `hold[review]` posé par le moteur après 3 steps, verdict
perdu — impute la cause à la **latence par step** de 5.3 sur une tâche
multi-step profonde (et non au transport, distinct de mika#2326), et enregistre
le remède déjà appliqué : retour à **glm-5.2 / Z.AI direct**.

La décision Prime du 2026-09-15 15:07 précise la condition de réveil, et elle est
conjonctive :

1. **mika#2296 mergé** — `llm_max_tokens = 8192` étouffe un modèle à
   raisonnement ; sans ce correctif 5.3 ne tient pas l'enveloppe QA.
2. **Un rejeu de la revue #2327 sous glm-5.3 qui CONCLUT** — verdict posé dans
   l'enveloppe, pas un `hold[review]` moteur.

« Tant que les deux ne sont pas remplies, QA reste sur 5.2. »

Ce plan ne swappe pas. Il livre ce sans quoi la condition 2 n'est ni exécutable
ni décidable, et il inscrit le dormeur là où le prochain qui touchera
`MIKA_QA_CONFIG` le lira.

---

## Ce qui est établi, et comment le vérifier

### E1 — La condition 1 est remplie

`git log --oneline -S'llm_max_tokens' -- crates/mika-agent/src/well_known_agents.rs`
→ **`09bc473c`** *fix: mika#2296 — `llm_max_tokens = 8192` étouffe les modèles à
raisonnement* (PR #2332). Mergé. mika#2295 (AC2-7, la latence arch, que le corps
du ticket pose aussi comme préalable) l'est également : `5a7a50fb` / PR #2327.

### E2 — Le dépôt n'a JAMAIS porté glm-5.3 pour mika-qa

```
git log --all -S'zai_model' -p -- crates/mika-agent/src/well_known_agents.rs \
  | grep -E "^[+-]zai_model" | sort -u
→ +zai_model = "glm-5.2"          # une seule ligne, un seul commit (b4219ce3, #1673)
```

Les seules occurrences de `glm-5.3` dans ce fichier sont des lignes `///` — la
note de dérive de `MIKA_DEV_CONFIG` et la phrase qui renvoie déjà à ce ticket.
**Aucune déclaration `zai_model = "glm-5.3"` n'a jamais existé.**

Trois conséquences, et elles déplacent la question du ticket :

- **(a)** Le 5.3 qui a produit l'incident était une édition **hors dépôt** du
  `config.toml` sur disque. Le « remède appliqué (config, restart 15:02) » est un
  retour à ce que la source disait déjà — pas un changement de source.
- **(b)** Cette édition survit aux redémarrages parce que
  `reconcile_well_known_config` (`well_known_agents.rs:706`) réécrit le fichier
  **entier** dès qu'il diffère, et n'est sautée que sous
  `MIKA_DISABLE_AGENT_PROVISIONING` — le drapeau dont le `CLAUDE.md` racine dit
  que c'est précisément le but (« un `llm_provider` / `openrouter_model` choisi à
  la main serait écrasé à chaque déploiement »). C'est la dérive que
  `MIKA_DEV_CONFIG` documente déjà en toutes lettres pour mika-dev
  (« this constant's source has DRIFTED from its runtime »). mika-qa a la même, et
  rien ne l'écrit.
- **(c)** **Aucune calibration mika-qa n'a jamais tourné sur 5.3.**
  `docs/eval/calibration/mika-qa-1632/` ne contient qu'un artefact
  `mika-qa-glm-5.2-post-1632.json` (`"model": "zai/glm-5.2"`). Le swap qui a cassé
  a enfreint mika#1190 — *aucun swap de modèle sans run de calibration passant* —
  et **rien dans le système ne l'a dit**.

### E3 — La calibration ne peut pas, seule, répondre à la question du ticket

`calibration/roles/mika_qa.rs` fait **un appel LLM par scénario**
(`provider.send_message(&request)`, aucune boucle d'outils, aucune enveloppe de
tour). Les fixtures font 500–1200 tokens d'entrée ; le baseline 5.2 mesure
**14–23 s par appel**. Le défaut de mika#2328 est le **cumul multi-step** d'une
revue réelle (diff + vérification d'AC + build) contre une enveloppe.

Donc : **un modèle peut passer la calibration à 100 % et faire perdre le verdict
en production.** La calibration reste *nécessaire* — c'est le gate mika#1190, et
elle mesure la latence par appel, exactement la grandeur que le ticket accuse —
mais elle n'est pas *suffisante*. C'est pourquoi Prime a posé **deux** conditions
et non une : la seconde couvre ce que la première ne voit pas.

### E4 — L'enveloppe de mika-qa n'est pas déclarée

`MIKA_QA_CONFIG` (`well_known_agents.rs:198`) porte `llm_provider`, `zai_model`,
`llm_max_tokens`, `log_level` — **ni `llm_http_timeout_secs` ni
`agent_total_timeout_secs`**. Le dépôt dit donc 120/300 (défaut de flotte,
mika#2189), pendant que mika#2347 relève qu'un chiffre de 600 s circule pour
mika-qa « que ni `MIKA_QA_CONFIG` ni les défauts du dépôt ne portent », et que
mika#2342 a mesuré 420/600 posés sur le process par l'environnement du service.

Conséquence directe : **« 5.3 ne tient pas l'enveloppe » est une phrase dont le
référent n'est pas établi.** Les tours de 258–315 s mesurés par le ticket ne sont
comparables à aucun seuil connu tant que la provenance n'est pas lue.
`llm_budget_resolved` (mika#2293) est l'instrument exact, et il existe déjà.

### E5 — Le baseline de calibration mika-qa est périmé

L'artefact du 2026-06-30 porte **5** scénarios (`verdict_format_precision`,
`per_ac_enumeration`, `absence_claim_grounding`, `wip_rescue_skip`,
`no_fabricated_fix`). `SCENARIOS` en déclare **8** aujourd'hui : s'y sont ajoutés
`duplicate_claim_grounded`, `negative_test_invariant_gate` (mika#2264) et
`verdict_format_canonical_shape`. Une comparaison 5.3-contre-baseline
comparerait 8 scénarios à 5.

---

## Décisions

### D1 — Cette PR ne swappe pas, et ce n'est pas de la prudence

La condition Prime 2 exige une **exécution mesurée en conditions réelles**. Ni
une session de grooming ni une PR ne peuvent la produire. Livrer le swap ici
serait passer outre une décision opérateur explicite. Le swap est le **ticket de
suivi**, conditionné aux deux sondes de U4.

Corollaire : `llm_max_tokens = 16384` **ne bouge pas non plus**. mika#2296 l'a
délibérément laissé « au ticket qui porte le swap de modèle et sa calibration ».
Ce ticket ne le porte pas. Monter le budget de sortie pour un 5.2 dont la
calibration passe à 16384, c'est changer une valeur en service sans mesure —
exactement ce que la note de `MIKA_ARCH_CONFIG` refuse.

### D2 — Ce que la PR livre : l'appareil qui rend la condition 2 décidable

Quatre unités, par ordre de nécessité décroissante. Chacune est justifiée par la
condition qu'elle rend exécutable, et U3 porte son propre critère de renoncement.

### D3 — La provenance du modèle est la moitié manquante de mika#2293

`llm_budget_resolved` dit aujourd'hui quel **couple de timeouts** est en vigueur
et d'où il vient. Il ne dit pas quel **modèle** tourne. Or E2 établit que le
modèle en service peut différer de celui du dépôt, indéfiniment et en silence —
et E4 que l'enveloppe le peut aussi. Les deux moitiés de la question « pourquoi
ce tour a-t-il été coupé ? » sont là, et une seule est instrumentée.

`turn_usage` porte bien `provider` et `model`, donc l'information existe — mais
par tour, dans 19 Go de log, sans provenance, et sans jamais dire si elle
contredit le dépôt. Un événement de *configuration* est ce qui manque, et le site
d'émission existe déjà.

**Étendre `llm_budget_resolved` plutôt que créer un événement jumeau** : les deux
faits se lisent ensemble (un tour coupé se diagnostique avec le modèle *et*
l'enveloppe), le site est le même (`server::init_agent`, `teams::engine`), et la
déduplication a déjà été étendue une fois pour cette raison exacte (mika#2362 y a
ajouté `effective_max_attempts`).

### D4 — Le garde de calibration couvre une moitié, et il faut le dire

Un test structurel qui refuse un modèle déclaré sans artefact de calibration
correspondant transforme la discipline mika#1190 en assertion. **Il n'aurait pas
attrapé mika#2328**, puisque le swap fautif était hors dépôt. Il attrape le swap
*par le dépôt* ; la moitié qui a mordu est couverte par U2 (la dérive devient
visible) et non par lui.

C'est l'unité dont la valeur est la plus discutable, et son critère de
renoncement est écrit en U3 plutôt que caché.

### D5 — Deux runs de calibration, pas un

E5 impose un run **5.2 de contrôle** sur la suite courante à 8 scénarios, le même
jour et avec la même suite que le run 5.3. Sans lui, toute différence observée est
inattribuable : elle peut venir du modèle, des trois scénarios ajoutés depuis
juin, ou d'une dérive du fournisseur en deux mois et demi.

### D6 — Où vit le protocole

`docs/eval/calibration/mika-qa-2328/README.md` — à côté des artefacts qu'il
produira, sur le modèle de `docs/eval/calibration/2264/README.md`. Pas dans
`docs/solutions/` : ce n'est pas une leçon, c'est une procédure dont les sorties
atterrissent dans ce répertoire.

---

## Volets d'implémentation

### U1 — Enregistrer le dormeur dans le constant (`well_known_agents.rs`)

Réécrire le doc-comment de `MIKA_QA_CONFIG` pour porter, en une fois :

- les **deux** conditions de réveil, avec l'état de chacune (condition 1 :
  remplie, `09bc473c` ; condition 2 : non remplie, procédure en
  `docs/eval/calibration/mika-qa-2328/`) ;
- le fait qu'**aucune calibration mika-qa n'a jamais tourné sur 5.3**, et la
  commande qui la produit (`make calibrate-mika-qa MODEL=zai/glm-5.3`) ;
- pourquoi `llm_max_tokens` reste à 16384 (D1) ;
- la **dérive dépôt ↔ runtime** (E2b), en miroir de la note que `MIKA_DEV_CONFIG`
  porte déjà : ce constant peut ne pas décrire ce qui tourne, et `llm_budget_resolved`
  est ce qui le dit.

Aucune valeur ne change. Le test `test_mika_qa_config_toml_is_valid_toml`
(`:2187`) reste vert tel quel — et c'est la vérification que U1 n'a rien touché
d'autre que de la prose.

### U2 — `llm_budget_resolved` nomme le modèle et sa provenance

Ajouter `model` et `model_source` à l'événement, au même site d'émission, et les
intégrer à la signature de déduplication (motif mika#2362 : un changement qui ne
bouge que ce champ doit être ré-émis, pas tu).

**Point d'implémentation à vérifier, avec sa halte.** `budget_provenance`
reconstruit la cascade pour **deux clés fixes** (`llm_http_timeout_secs`,
`agent_total_timeout_secs`). La clé du modèle dépend du provider (`zai_model`,
`openrouter_model`, `anthropic_model`, …), ce qui n'est pas la même forme. Deux
issues :

- le lecteur se généralise proprement sur un nom de clé résolu depuis
  `ProviderKind::config_prefix()` → on l'étend ;
- il ne se généralise pas sans réécrire la cascade → **halte** : n'émettre que
  `model` sans `model_source`, et ouvrir le suivi. Un modèle nommé sans
  provenance vaut déjà mieux qu'un modèle tu ; une provenance **fausse** est
  strictement pire que pas de provenance (mika#2293 le dit pour ses deux clés,
  et la raison vaut ici mot pour mot).

L'ordre de la cascade reste celui de mika#2218 (`.env` per-agent > env du process
> `config.toml` per-agent > `config.toml` global > constante), et l'épinglage
existant (`mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`)
est le gabarit du test à écrire pour la clé modèle.

### U3 — Garde : un modèle déclaré sans calibration ne compile pas vert

Test structurel sur `WELL_KNOWN_AGENTS` : pour chaque `config_toml` déclarant un
modèle, exiger qu'un artefact JSON sous `docs/eval/calibration/` porte ce couple
`provider/model` pour ce rôle.

- Population = les **déclarations**, jamais les absences (motif
  `mika2296_no_well_known_config_declares_an_output_budget_below_8192`) : un agent
  sans `config_toml` n'a pris aucune décision de modèle.
- **Aucune liste d'exemption.** La population est comptée à l'écriture (mika-dev
  `z-ai/glm-5.2`, mika-qa `zai/glm-5.2`, mika-arch `moonshotai/kimi-k2.5`). Un
  test qui rougit plus tard se résout par un run de calibration, pas par une
  dispense — c'est tout son objet.

**Critère de renoncement, à évaluer avant d'écrire le test.** Si les artefacts
existants ne portent pas le couple sous une forme lisible sans heuristique pour
**les trois** agents, U3 tombe. Un garde qui exige une convention de nommage que
le dépôt ne tient pas déjà déplacerait la discipline au lieu de l'asserter, et
serait contourné au premier rouge. Vérification : `mika-dev-1633/`,
`mika-qa-1632/` et le champ `providers.<role>.model` de chaque JSON.
Si U3 tombe, le dire dans le corps de PR et ouvrir le suivi — pas le remplacer
par une variante affaiblie.

### U4 — Le protocole de re-test (`docs/eval/calibration/mika-qa-2328/README.md`)

Quatre étapes ordonnées, chacune avec sa halte. **Aucune ne modifie la
production.**

**Étape 0 — lire l'enveloppe en vigueur (précondition absolue, E4).**

```bash
grep llm_budget_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-qa")
        | {http_timeout_secs, agent_total_timeout_secs, http_source, total_source,
           max_attempts, effective_max_attempts, model, model_source}'
```

| `http_source` / `total_source` | Lecture | Geste |
|---|---|---|
| `agent_config` | le dépôt fait loi | on peut déclarer le couple dans `MIKA_QA_CONFIG` — **suivi**, pas ici |
| `process_env` | une variable de service écrase le per-agent | **halte** : déclarer dans le constant serait **inerte**. Le remède est dans l'environnement du service |
| `default` / `global_config` | la clé per-agent n'est pas lue ou n'existe pas | geste de provisionnement (classe mika#2330) |

**Sans cette lecture, les étapes suivantes ne mesurent rien d'interprétable** :
« tenir l'enveloppe » n'a pas de seuil.

**Étape 1 — contrôle 5.2 sur la suite courante (D5).**
`make calibrate-mika-qa MODEL=zai/glm-5.2`, artefact archivé sous
`mika-qa-2328/control-5.2/`. Halte : un scénario rouge **sur 5.2** invalide toute
la campagne — le problème n'est pas le modèle candidat, et il faut le traiter
avant d'aller plus loin.

**Étape 2 — candidat 5.3 (gate mika#1190).**
`make calibrate-mika-qa MODEL=zai/glm-5.3`, artefact sous `candidate-5.3/`.
Critère du gate, inchangé : **100 % des scénarios passent**. Critère additionnel
propre à ce ticket, et c'est la grandeur que le ticket accuse : comparer
`latency_ms` scénario par scénario aux 14–23 s du contrôle. **Ce n'est pas un
seuil, c'est une mesure** — un facteur 2 ou 3 par appel sur un brief de 1 000
tokens prédit mal un brief de revue réel, et le dire vaut mieux que de fabriquer
un seuil qui n'a pas de fondement.

**Étape 3 — le rejeu de #2327 (condition Prime 2), sur créneau calme.**
Le corps du ticket l'exige (« ne re-tester que sur créneau calme, avec mesure des
tours »). C'est la seule étape qui mesure le cumul multi-step (E3), et la seule
qui touche un agent en service — **donc la seule qui exige une fenêtre annoncée
et un retour arrière d'une ligne.**

Conclusion lue dans le log, pas dans une impression :

```bash
grep turn_usage "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-qa" and .model == "glm-5.3")
        | {step, stop_reason, latency_ms, input_tokens, output_tokens,
           request_bytes, system_prompt_bytes, status}'
grep qa_deadline_verdict "$MIKA_SPIRIT_LOG_FILE" | jq 'select(.outcome == "posted")'
```

- **Succès** = un `run_gh pr review` posté par le tour **et** zéro
  `qa_deadline_verdict` `posted` sur cette PR. Le filet moteur (mika#2276) est
  exactement le détecteur du symptôme : s'il fire, la revue n'a pas conclu, quelle
  que soit l'impression laissée.
- **Halte `MaxTokens`** : un `stop_reason = MaxTokens` avec `output_tokens` collé
  à 16384 signifie que la condition 1 n'a **pas** suffi pour mika-qa et que le
  budget de sortie est bien le facteur suivant — c'est la branche que D1 a laissée
  ouverte, et elle se traite par une mesure, pas par un doublement réflexe.
- **Halte enveloppe** : des tours qui finissent proprement sous une enveloppe dont
  l'étape 0 a montré qu'elle vient de `process_env` ne prouvent rien sur 120/300.
  Le rejeu mesure l'enveloppe qui tournait, et il faut nommer laquelle.

---

## Verification contract

| # | Quoi | Comment |
|---|---|---|
| V1 | U1 ne change aucune valeur | `test_mika_qa_config_toml_is_valid_toml` vert **sans modification**, et `git diff` sur `MIKA_QA_CONFIG` ne touche que des lignes `#` / `///` |
| V2 | `llm_budget_resolved` porte `model` (+ `model_source` si U2 aboutit) | test unitaire sur la construction de l'événement ; grep sur un démarrage local |
| V3 | La provenance du modèle est exacte | test sur le modèle de `mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`, sur chaque position de la cascade. **Rouge ⇒ n'émettre que `model`** (halte U2) |
| V4 | La déduplication ré-émet sur changement de modèle | deux résolutions ne différant que par le modèle produisent deux lignes |
| V5 | U3 attrape un modèle non calibré | contrôle négatif : un `config_toml` de test déclarant un modèle inconnu fait rougir le garde |
| V6 | U3 ne rougit pas sur l'existant | le test passe sur les trois déclarations réelles — **si non, U3 tombe (D4/U3)** |
| V7 | Le protocole est exécutable tel qu'écrit | les deux commandes `jq` de U4 rendent des lignes sur un log réel ; les cibles `make` existent (`Makefile:119`) |

**Contrôle négatif porteur (V5) :** sans lui, U3 peut être vert parce qu'il
n'assert rien — un garde structurel qui ne rougit jamais est indistinguable d'un
garde absent, et c'est la classe que `mika2131_exclusion_skips_never_return_to_an_uncollected_debug`
existe pour nommer.

---

## Fire-Disposition

- **U2 émet et n'empêche rien.** Un événement de configuration ne peut pas
  refuser un démarrage. Une provenance illisible se journalise, elle ne fait pas
  tomber l'agent (motif mika#2293 : la garde de budget refuse le démarrage sur une
  demi-configuration *fonctionnellement cassée*, jamais sur une observabilité
  indisponible).
- **U3 rougit en CI, jamais en production.** C'est un test, pas une garde de
  démarrage. Un modèle non calibré déjà en service n'est pas cassé par ce test —
  il est nommé au prochain PR qui touche la déclaration.
- **U4 ne dispose de rien** : c'est une procédure, et son étape 3 est la seule à
  toucher un agent, sous fenêtre annoncée et retour arrière d'une ligne
  (`zai_model` sur disque + restart).

---

## Definition of Done

- [ ] `MIKA_QA_CONFIG` porte les deux conditions de réveil, leur état, la commande
      de calibration, la raison du maintien de `llm_max_tokens = 16384` et la note
      de dérive dépôt ↔ runtime — **sans qu'aucune valeur change**.
- [ ] `llm_budget_resolved` porte `model`, et `model_source` ou la halte de U2
      documentée dans le corps de PR avec son suivi.
- [ ] La déduplication de l'événement intègre le nouveau champ.
- [ ] U3 livré avec son contrôle négatif, **ou** son abandon motivé dans le corps
      de PR et son suivi ouvert (critère V6).
- [ ] `docs/eval/calibration/mika-qa-2328/README.md` porte les quatre étapes, les
      trois tables de lecture et les trois haltes.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` verts.
- [ ] Le corps de PR dit explicitement que **le swap n'est pas livré**, nomme la
      condition Prime non remplie, et nomme le ticket de suivi qui le portera.

---

## Acceptance criteria

*Le corps du ticket n'a pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de la condition de réveil Prime et des Requirements.*

- **AC1** — La condition 1 est tracée dans le code : `MIKA_QA_CONFIG` nomme
  mika#2296, son SHA de merge, et le fait qu'elle est remplie.
- **AC2** — La condition 2 est **exécutable** : une procédure écrite, ordonnée,
  avec ses commandes littérales, permet à un opérateur de produire le rejeu de
  #2327 sous 5.3 et de trancher « conclut / ne conclut pas » sur une sortie
  machine (`run_gh pr review` posté **et** `qa_deadline_verdict` silencieux), pas
  sur une impression.
- **AC3** — L'enveloppe en vigueur pour mika-qa et sa **provenance** sont lisibles
  en une commande, et l'absence de provenance `agent_config` est traitée comme une
  halte et non comme un détail.
- **AC4** — Le **modèle** en vigueur est lisible dans un événement de
  configuration, pas seulement dans `turn_usage` par tour : une divergence entre
  le modèle du dépôt et le modèle en service cesse d'être silencieuse.
- **AC5** — `zai_model` et `llm_max_tokens` de `MIKA_QA_CONFIG` sont **inchangés**
  à l'issue de cette PR ; mika-qa reste sur glm-5.2, conformément à la décision
  Prime.
- **AC6** — Le protocole exige un run de contrôle 5.2 sur la suite courante à 8
  scénarios avant tout run 5.3, et dit pourquoi le baseline de juin ne peut pas en
  tenir lieu.
- **AC7** — La limite de la calibration est écrite : elle est mono-appel et ne
  mesure pas le cumul multi-step, donc elle ne peut pas à elle seule autoriser le
  retour à 5.3.
- **AC8** — Aucune régression : les tests existants sur `MIKA_QA_CONFIG`, sur
  `llm_budget_resolved` et sur la reconstruction de la cascade restent verts.

---

## Surfaces opérateur et sonde post-déploiement

**Lecture, en une commande** (U2 livré) :

```bash
grep llm_budget_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq '{agent_id, model, model_source, http_timeout_secs, agent_total_timeout_secs,
         http_source, total_source}'
```

**Sonde post-déploiement, avec sa halte.** Au premier démarrage après déploiement,
les quatre agents bien connus doivent produire une ligne portant un `model` non
vide.

- `model` de mika-qa ≠ `glm-5.2` → **la dérive de E2 est confirmée et mesurée** :
  c'est un résultat, pas une panne, et c'est le premier fait que le ticket de suivi
  devra porter. Ne pas « corriger » en éditant le disque avant d'avoir noté la
  valeur, sa provenance et la date.
- `model_source = process_env` → une variable de service fixe le modèle ; le
  dépôt n'est pas autoritaire et `MIKA_QA_CONFIG` est décoratif pour cette clé.
- **Aucune ligne** → l'événement n'est pas émis sur ce chemin ; ne pas élargir
  l'émission par réflexe, établir d'abord quel site d'initialisation a servi cet
  agent (angle mort connu de mika#2293 : le chemin per-skill n'émet rien).

Cadence : celle de `llm_budget_resolved`, c'est-à-dire **une par agent et par
couple résolu**, ré-émise au changement. Un grep qui ne rend qu'une poignée de
lignes après des jours d'exécution est le régime normal.

---

## Hors périmètre (suivi à ouvrir)

- **Le swap lui-même** (`zai_model = "glm-5.3"` + budget de sortie solidaire +
  mise à jour de `test_mika_qa_config_toml_is_valid_toml` + artefact de
  calibration + rollback). **Conditionné aux étapes 1–3 de U4.** C'est la
  décision Prime, pas une préférence de ce plan.
- **Déclarer le couple de timeouts dans `MIKA_QA_CONFIG`.** Conditionné à
  l'étape 0 : sous `process_env`, la déclaration serait inerte et donnerait
  l'illusion d'un réglage. Le remède serait alors dans l'environnement du service,
  ce qui est une autre décision et un autre blast radius.
- **La dérive dépôt ↔ runtime en général** (les trois agents, pas seulement
  mika-qa, et le fait que `MIKA_DISABLE_AGENT_PROVISIONING` gèle `config.toml`
  entier). U2 la rend visible ; la résoudre — réconcilier par section plutôt que
  par fichier, comme mika#2330 l'a fait pour `identity.toml` — est un travail à
  part entière.
- **Un scénario de calibration multi-step** qui mesurerait le cumul contre une
  enveloppe et ferait du gate mika#1190 un prédicteur de la classe mika#2328
  (E3). Réel et attirant ; c'est un sous-système, pas une ligne, et ce ticket
  n'a pas la mesure qui dimensionnerait son enveloppe.
- **Le transport OpenRouter** (mika#2326), distinct et nommé comme tel par le
  corps du ticket.
