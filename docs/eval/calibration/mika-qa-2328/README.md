# mika#2328 — protocole de re-test de glm-5.3 pour mika-qa

Le ticket mika#2328 est un **dormeur**. Il constate (2026-09-15) que mika-qa sous
glm-5.3 / Z.AI direct a dépassé son enveloppe de revue sur la PR #2327 — tours de
258–315 s, `hold[review]` posé par le moteur après 3 steps, verdict perdu —
enregistre le remède (retour à glm-5.2), et pose sa **condition de réveil**.

La décision Prime du 2026-09-15 15:07 est **conjonctive** :

1. **mika#2296 mergé.** `llm_max_tokens = 8192` étouffe un modèle à raisonnement.
   **Remplie** — `09bc473c` (PR #2332), 2026-09-16.
2. **Un rejeu de la revue #2327 sous glm-5.3 qui CONCLUT** — verdict posé dans
   l'enveloppe, pas un `hold[review]` moteur. **Non remplie.**

Ce document est la procédure qui produit la condition 2 et la rend **décidable sur
une sortie machine**. Il ne swappe rien et **aucune de ses étapes ne modifie la
production**, à l'exception de l'étape 3, qui est la seule à toucher un agent en
service et porte pour cela une fenêtre annoncée et un retour arrière d'une ligne.

Les artefacts produits par ce protocole atterrissent dans ce répertoire
(`control-5.2/`, `candidate-5.3/`).

---

## Ce que la calibration ne peut pas dire — à lire avant de la lancer

`crates/mika-agent/src/calibration/roles/mika_qa.rs` fait **un appel LLM par
scénario** (`provider.send_message`, aucune boucle d'outils, aucune enveloppe de
tour). Les fixtures pèsent 500–1200 tokens d'entrée ; le baseline 5.2 de juin
mesure **9,5–23,5 s par appel** — relevé sur l'artefact lui-même, pas repris de
mémoire :

```
$ jq -r '.providers["mika-qa"].scenarios | to_entries[] | "\(.key)\t\(.value.latency_ms)"' \
    docs/eval/calibration/mika-qa-1632/mika-qa-glm-5.2-post-1632.json
absence_claim_grounding  23487
no_fabricated_fix        20313
per_ac_enumeration       14221
verdict_format_precision  9492
wip_rescue_skip          16597
```

Le défaut de mika#2328 est le **cumul multi-step** d'une revue réelle (analyse de
diff + vérification d'AC + build) contre une enveloppe. Donc : **un modèle peut
passer la calibration à 100 % et faire perdre le verdict en production.** La
calibration reste *nécessaire* — c'est la porte mika#1190, et elle mesure la
latence par appel, exactement la grandeur que le ticket accuse — mais elle n'est
pas *suffisante*. C'est pourquoi Prime a posé deux conditions et non une : la
seconde couvre ce que la première ne voit pas.

---

## Étape 0 — lire l'enveloppe en vigueur (précondition absolue)

`MIKA_QA_CONFIG` ne déclare **ni** `llm_http_timeout_secs` **ni**
`agent_total_timeout_secs` : le dépôt dit donc 120/300 (défaut de flotte,
mika#2189), pendant que mika#2347 relève qu'un chiffre de 600 s circule pour
mika-qa et que mika#2342 a mesuré 420/600 posés sur le process par
l'environnement du service.

**« 5.3 ne tient pas l'enveloppe » est donc une phrase dont le référent n'est pas
établi.** Les tours de 258–315 s ne sont comparables à aucun seuil connu tant que
la provenance n'est pas lue.

```bash
grep llm_budget_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-qa")
        | {http_timeout_secs, agent_total_timeout_secs, http_source, total_source,
           max_attempts, effective_max_attempts,
           provider, model, model_source, model_config_key}'
```

| `http_source` / `total_source` | Lecture | Geste |
|---|---|---|
| `agent_config` | le dépôt fait loi | on peut déclarer le couple dans `MIKA_QA_CONFIG` — **suivi**, pas ici |
| `process_env` | une variable de service écrase le per-agent | **halte** : déclarer dans le constant serait **inerte**. Le remède est dans l'environnement du service |
| `default` / `global_config` | la clé per-agent n'est pas lue ou n'existe pas | geste de provisionnement (classe mika#2330) |

Les champs `model` / `model_source` de la même ligne (mika#2328) répondent à
l'autre moitié de la question — *quel modèle tourne réellement* :

| `model_source` | Lecture |
|---|---|
| `agent_config` avec `model` ≠ `glm-5.2` | **la dérive dépôt ↔ runtime de E2 est confirmée et mesurée** : c'est un résultat, pas une panne. Noter la valeur, sa provenance et la date **avant** de corriger quoi que ce soit sur le disque |
| `process_env` | une variable de service fixe le modèle ; le dépôt n'est pas autoritaire pour cette clé et `MIKA_QA_CONFIG` est décoratif |
| `default` | aucune porte ne porte la clé `zai_model` : c'est le défaut du provider qui s'applique |
| `unknown_provider` | `llm_provider` porte une valeur illisible — état normalement inatteignable (`Settings::load_for_agent` refuse le fichier), à traiter avant tout le reste |

**Aucune ligne du tout** → l'événement n'est pas émis sur ce chemin. Ne pas
élargir l'émission par réflexe : établir d'abord quel site d'initialisation a
servi cet agent (angle mort connu de mika#2293 — le chemin per-skill n'émet rien).

**Sans cette lecture, les étapes suivantes ne mesurent rien d'interprétable** :
« tenir l'enveloppe » n'a pas de seuil.

---

## Le piège de la cible `make`, à connaître avant l'étape 1

```
make calibrate-mika-qa MODEL=zai/glm-5.2
```

passe `--baseline docs/eval/calibration/baselines/latest.json`, **et ce répertoire
n'existe pas dans le dépôt**. La porte sort alors en **exit 2** — *gate not
enforceable: no usable baseline* — ce qui n'est **pas** un échec du modèle. Omettre
`--baseline` ne répare rien : `BaselineState::NotProvided` sort en 2 également
(`calibration/gate.rs:110`) ; c'est délibéré — « rien à comparer » ne doit pas
ressembler à « comparé et bon ».

Le protocole ci-dessous invoque donc le binaire directement et **fabrique sa propre
base de comparaison à l'étape 1**, ce qui est aussi ce que D5 demande : le contrôle
5.2 n'est pas un préalable administratif, c'est le repère contre lequel 5.3 est
jugé.

---

## Étape 1 — contrôle 5.2 sur la suite courante

L'artefact de juin (`docs/eval/calibration/mika-qa-1632/`) porte **5** scénarios ;
`SCENARIOS` en déclare **8** aujourd'hui — s'y sont ajoutés
`duplicate_claim_grounded`, `negative_test_invariant_gate` (mika#2264) et
`verdict_format_canonical_shape`. Une comparaison 5.3-contre-baseline-de-juin
comparerait 8 scénarios à 5, et toute différence observée serait inattribuable :
elle pourrait venir du modèle, des trois scénarios ajoutés, ou d'une dérive du
fournisseur en deux mois et demi.

D'où un run de contrôle **le même jour, sur la même suite**, qui sert de baseline :

```bash
cargo run --bin calibrate --release -- \
  --role mika-qa --model zai/glm-5.2 \
  --establish-baseline \
  --baseline docs/eval/calibration/mika-qa-2328/control-5.2/baseline.json \
  --output   docs/eval/calibration/mika-qa-2328/control-5.2/artifact.json
```

**Halte.** `--establish-baseline` refuse d'écrire une baseline sous le plancher de
100 % (`GateOutcome::BaselineRefused`, exit 1). Ce refus **est** la halte de D5 : un
scénario rouge **sur 5.2** invalide toute la campagne — le problème n'est pas le
modèle candidat, et il faut le traiter avant d'aller plus loin. Ne pas passer
`--force-failing-baseline` pour « débloquer » : une baseline défaillante abaisse la
barre de toutes les comparaisons suivantes.

Relever au passage les `latency_ms` par scénario dans l'artefact : ils sont le point
de comparaison de l'étape 2.

---

## Étape 2 — candidat 5.3 (porte mika#1190)

```bash
cargo run --bin calibrate --release -- \
  --role mika-qa --model zai/glm-5.3 \
  --baseline docs/eval/calibration/mika-qa-2328/control-5.2/baseline.json \
  --output   docs/eval/calibration/mika-qa-2328/candidate-5.3/artifact.json
```

Critère de la porte, inchangé : **100 % des scénarios passent** (et pas de
régression contre le contrôle de l'étape 1 — la porte l'applique elle-même,
exit 1 sur `FailFloor` comme sur `FailBaseline`).

Critère additionnel propre à ce ticket, et c'est la grandeur que le ticket accuse :
comparer `latency_ms` **scénario par scénario** à ceux du contrôle (9,5–23,5 s sur
l'artefact de juin ; le contrôle de l'étape 1 donne la valeur du jour, qui est la
seule comparable).

```bash
jq -r '.providers["mika-qa"].scenarios | to_entries[] | "\(.key)\t\(.value.latency_ms)"' \
  docs/eval/calibration/mika-qa-2328/control-5.2/artifact.json
jq -r '.providers["mika-qa"].scenarios | to_entries[] | "\(.key)\t\(.value.latency_ms)"' \
  docs/eval/calibration/mika-qa-2328/candidate-5.3/artifact.json
```

**Ce n'est pas un seuil, c'est une mesure.** Un facteur 2 ou 3 par appel sur un
brief de 1 000 tokens prédit mal un brief de revue réel, et le dire vaut mieux que
de fabriquer un seuil sans fondement. Le chiffre sert à l'étape 3 : il dit
*combien* de marge l'enveloppe lue à l'étape 0 laisse encore.

---

## Étape 3 — le rejeu de #2327 (condition Prime 2), sur créneau calme

Le corps du ticket l'exige : « ne re-tester que sur créneau calme, avec mesure des
tours ». C'est la seule étape qui mesure le cumul multi-step, et la seule qui
touche un agent en service.

1. Annoncer la fenêtre.
2. Noter la valeur courante de `zai_model` **sur le disque**
   (`~/.mika/agents/mika-qa/config.toml`) — c'est le retour arrière.
3. Poser `zai_model = "glm-5.3"` sur ce fichier, redémarrer mika-spirit.
4. Re-déclencher la revue de la PR #2327 (`gh pr edit 2327 --add-reviewer
   mika-platform-qa`, ou tout déclencheur de revue habituel).
5. Lire la conclusion **dans le journal, pas dans une impression**.
6. Retour arrière : restaurer la ligne notée en 2, redémarrer.

```bash
grep turn_usage "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-qa" and .model == "glm-5.3")
        | {step, stop_reason, latency_ms, input_tokens, output_tokens,
           request_bytes, system_prompt_bytes, status}'

grep qa_deadline_verdict "$MIKA_SPIRIT_LOG_FILE" | jq 'select(.outcome == "posted")'
grep qa_callback_verdict "$MIKA_SPIRIT_LOG_FILE" | jq 'select(.outcome == "posted")'
```

**Succès** = un `run_gh pr review` posté par le tour **et** zéro filet moteur
`posted` sur cette PR. Le filet (mika#2276, généralisé par mika#2368) est
exactement le détecteur du symptôme : s'il fire, la revue n'a pas conclu, quelle
que soit l'impression laissée.

**Halte `MaxTokens`** — un `stop_reason = MaxTokens` avec `output_tokens` collé à
16384 signifie que la condition 1 n'a **pas** suffi pour mika-qa et que le budget de
sortie est le facteur suivant. Cela se traite par une mesure et un ticket, pas par
un doublement réflexe : `llm_max_tokens` est une valeur en service.

**Halte enveloppe** — des tours qui finissent proprement sous une enveloppe dont
l'étape 0 a montré qu'elle vient de `process_env` ne prouvent **rien** sur 120/300.
Le rejeu mesure l'enveloppe qui tournait ; il faut nommer laquelle dans le compte
rendu.

**Halte modèle** — si la ligne `llm_budget_resolved` lue après le redémarrage de
l'étape 3.3 ne montre pas `model: "glm-5.3"`, le rejeu ne mesure pas ce qu'on croit :
`reconcile_well_known_config` a pu réécrire le fichier. Vérifier avant de conclure.

---

## Si les deux conditions sont remplies

Le swap lui-même est **hors du périmètre de mika#2328** et relève de son ticket de
suivi : `zai_model = "glm-5.3"` dans `MIKA_QA_CONFIG`, budget de sortie solidaire,
mise à jour de `test_mika_qa_config_toml_is_valid_toml`, l'artefact de calibration
de l'étape 2 versé au dépôt, et la procédure de rollback. C'est la décision Prime,
pas une préférence de ce protocole.

Tant que les deux ne sont pas remplies, **mika-qa reste sur glm-5.2**.
