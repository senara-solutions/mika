# mika#1910 — l'axe B n'est pas mesurable : le tour de continuation ne dit pas ce qu'il a produit

**Ticket :** senara-solutions/mika#1910
**Type :** fix (instrumentation + mesure)
**Branche :** `bug/1910/llm-glm-5-2-silent-empty-output-au-max`
**Date :** 2026-09-21

---

## Goal Capsule

Le ticket demande cinq investigations sur la sortie vide silencieuse de
`glm-5.2` à `max_turns`, et son commentaire du 2026-08-30 fixe sa condition de
fermeture : *« une cause racine corrigée, ou une mesure fraîche montrant la
classe disparue — pas une bascule de modèle »*.

**Huit faits, lus dans le dépôt et sur l'hôte, déplacent le travail.** L'axe A
est mesuré et clos par mika#1950 ; ce qui reste dû est l'axe B, que ce même
travail a explicitement laissé ouvert ; les rejeux prescrits restent
inexécutables ; et — c'est le constat central — **l'axe B n'est mesurable par
aucun instrument existant**, parce que le tour de continuation, très exactement
là où la classe mika#1910 se manifeste dans le moteur mika, enregistre
`response_text = NULL` **inconditionnellement, succès compris**.

Ce plan livre donc ce qui manque *avant* la mesure : l'instrumentation qui rend
la vacuité lisible, le geste reproductible qui la compte, et le refus explicite
de produire un verdict qu'il ne peut pas fonder. **Aucun modèle ne bouge.
mika#1910 n'est pas fermé.**

---

## Product Contract

### La rectification est le premier livrable

Un plan qui exécuterait la lettre du ticket produirait un rapport faux avec
l'autorité d'une mesure. Les huit constats ci-dessous sont donc du contenu, pas
du préambule ; chacun est vérifiable par la commande ou le site qui l'accompagne.

#### R1 — L'axe A est mesuré et clos ; il ne se refait pas

mika#1950 a livré, et son résultat est dans l'arbre à ce commit :
`scripts/measure-pilot-cycle-emptiness` (287 lignes), son test (433 lignes,
35 assertions), `docs/eval/mika-1950/measurement-2026-09-20.jsonl` et son
`README.md`. Le verdict de l'axe A est **SANS OBJET, prémisse réfutée** : sur
2353 sessions `claude-pilot` (2026-05-15 → 2026-09-20), l'événement
`[guardrail] empty_response:` survient **0 fois**, et **zéro** de ces sessions ne
tourne sous GLM (`claude-opus-4-8[1m]` 1376, `claude-opus-5[1m]` 288, 686 sans
modèle rapporté).

```bash
jq -s 'map(select(.empty_event)) | length' docs/eval/mika-1950/measurement-2026-09-20.jsonl   # → 0
```

La correction d'attribution a également atterri :
`no-substrate-on-open-failure-mode-2026-08-30.md` porte désormais, dans le corps
de sa règle 3, la rectification du 2026-09-20 (« all 120 of those sessions ran
under `claude-opus-4-8[1m]` and none under any GLM »). **Rien de cet axe n'est à
reprendre.**

#### R2 — Ce qui reste dû est l'axe B, et mika#1950 le dit par écrit

Son `README.md` §9 :

> **mika#1910 n'est pas fermé ici.** […] La moitié GLM du symptôme — les tours de
> `mika-dev` — n'est pas mesurée ici : elle vit dans `$MIKA_SPIRIT_LOG_FILE` et la
> procédure est en `docs/eval/mika-1950/README.md` §5.

Le périmètre de ce plan est donc **exactement** cet axe B, et rien d'autre.

#### R3 — Les rejeux prescrits restent inexécutables

Les investigations 1, 2 et 3 du ticket reposent sur
`docs/eval/hy3-v0-2026-08-18/traces/run_glm-5.2_{1878,1817}.jsonl`. Les cinq
commandes de `docs/eval/mika-1950/README.md` §1 établissent leur absence de tout
l'historique atteignable et du workspace ; elles sont **rejouées en précondition**
de ce plan (Verification Contract §1), avec la même halte. Fabriquer des fixtures
« équivalentes » reste refusé : une fixture construite depuis la définition du
symptôme ne peut pas réfuter cette définition.

#### R4 — L'axe B n'est mesurable par aucun instrument existant. C'est le constat central

**Où la classe atterrit dans le moteur mika.** Le ticket décrit 30 tours brûlés
puis un message final vide. Traduit dans l'architecture mika : `max_steps` (20)
épuisé → `LoopResult::MaxStepsExceeded` → `attempt_continuation_turn` (outils
désactivés, budget écrêté) → si ce tour ne rend rien, le moteur sert
`EMPTY_RESPONSE_FALLBACK`, littéralement `"Done."`
(`crates/mika-agent/src/agent_loop/mod.rs:85`). **Le symptôme terminal de
mika#1910 vit donc sur le tour de continuation**, et nulle part ailleurs.

**Ce que ce tour enregistre.** `save_continuation_llm_call`
(`agent_loop/mod.rs:882`) appelle `save_llm_call` en passant `None, None` aux
positions `response_text` et `reasoning` — **sur toutes ses branches, succès
compris** (`mod.rs:957-958`). L'asymétrie est dans le même fichier, à soixante
lignes :

| site | chemin | `response_text` écrit ? |
|---|---|---|
| `mod.rs:1500` | boucle, bras `Ok` | **oui** — `response_text.as_deref()` |
| `mod.rs:1528` | boucle, bras `Err` | non — *correct*, aucune réponse n'existe |
| `mod.rs:942` | **continuation, toutes branches** | **non** — la lacune |

**Conséquence, et elle est dure :** sur le tour de continuation,
`response_text IS NULL` est vrai **à 100 %**, qu'il ait produit un résumé ou
rien. La colonne qui porterait la vacuité est inconditionnellement nulle sur la
seule ligne qui compte. **La base ne peut pas mesurer la classe mika#1910.**

**Et le journal non plus.** `turn_usage` (Signal O) porte `step`, `stop_reason`,
`output_tokens`, `tool_use_in_turn`, `status`, `request_bytes`,
`system_prompt_bytes` — **aucune mesure du texte produit**. Le champ
`response_chars` n'existe pas ; `TurnUsageFields` (`mod.rs:8396`) ne le déclare
pas.

Les deux surfaces que la procédure §5 de mika#1950 prescrit sont donc muettes
sur la question même que le ticket pose. **Ce n'est pas un défaut de cette
procédure : c'est une lacune d'instrumentation en amont d'elle.**

#### R5 — Le piège autour duquel ce travail est construit : trois populations nulles, dont une n'est pas vide

`response_text` est NULL pour trois raisons distinctes :

| population | `response_text` | est-ce une sortie vide ? |
|---|---|---|
| (a) lignes antérieures à la v31 | NULL | **inconnu** — la colonne n'existait pas |
| (b) appels en erreur (`mod.rs:1528`) | NULL | **non** — aucune réponse n'existe |
| (c) **tout** tour de continuation, succès compris | NULL | **non déterminé** — la lacune R4 |

Un prédicat qui lirait la vacuité comme `response_text IS NULL` compterait les
trois. C'est le pendant, pour l'axe B, du piège `emptyResponseThreshold` que le
script de mika#1950 existe pour ne pas retomber dedans — **et il est pire** : là
2349 faux positifs sur 2352 étaient détectables par écart à l'événement réel,
ici le taux de faux positifs est de **100 % sur exactement la population
d'intérêt**. Le nombre produit serait faux, plausible, et porté par l'autorité
d'une mesure.

**Corollaire de conception :** la mesure ne peut pas être dérivée de l'absence
d'une valeur. Il faut une valeur **posée**, et c'est U1.

#### R6 — mika#2357 est une autre classe et doit rester comptable à part

`docs/dormeurs.md` inscrit mika#2357 : *« callback-\* mika-dev muets sur glm-5.3
(tours vides, `stop_reason` error, input 0) »*, dormant *« quand le modèle actif
de mika-dev redevient glm-5.3 »*. Les deux formes sont distinctes sur des champs
que le journal porte :

| | mika#1910 | mika#2357 |
|---|---|---|
| modèle | `glm-5.2` | `glm-5.3` |
| `status` | `success` | `error` |
| `input_tokens` | *normaux* (« tokens consommés normalement ») | **0** |
| `stop_reason` | `ToolUse` / `EndTurn` | `error` |

Confondre les deux ferait compter une population glm-5.3 sous un ticket
glm-5.2. Le prédicat doit les séparer **par construction**, pas par prudence de
lecture — c'est le contrôle négatif N2 du Verification Contract.

#### R7 — Le discriminant de mika#1665 existe déjà en Rust et ne se re-dérive pas

`calibration::failure::classify_failure`
(`crates/mika-agent/src/calibration/failure.rs:79`) porte la règle, avec son
raisonnement au-dessus :

> texte vide **+** `finish_reason_is_length` **+** `output_tokens > 0`
> ⇒ `ReasoningBudgetExhausted` (relever `max_tokens`), **et non** `EmptyResponse`
> (régression modèle).

C'est la règle qui distingue « le modèle a brûlé son budget de sortie en
raisonnement interne » — sain, remédié par un réglage — de la classe mika#1910.
Sur le rail mika, le signal de plafond est `StopReason::MaxTokens`
(`mika-common/src/claude.rs:283`), sérialisé tel quel dans `stop_reason`.

**Une règle, deux consommateurs, épinglés par test.** La calibration la lit sur
un `ScenarioOutcome` ; l'analyseur de l'axe B la lit sur une ligne de journal.
Ce plan ne déplace pas la règle et n'en écrit pas une seconde : il pose
`failure.rs:79` comme **site de définition** et fait assertionner les trois mêmes
cas au test de U3 (Verification Contract §4). Deux orthographes de la même règle
couperaient une population en deux sans le dire.

#### R8 — Quel modèle tourne réellement n'est pas lisible depuis le dépôt

`MIKA_DEV_CONFIG` (`well_known_agents.rs:185`) déclare
`openrouter_model = "z-ai/glm-5.2"`, et son propre doc-comment nomme la dérive :
*« this constant's source has DRIFTED from its runtime […] the plans of mika#2179
and mika#2189 measure mika-dev on `z-ai/glm-5.3` »*. Le registre des dormeurs
affirme de son côté `glm-5.2` aujourd'hui. Les deux ne peuvent pas être vraies
en même temps, et **aucune ligne du dépôt ne tranche** : la bascule du
2026-08-26 vit hors du dépôt, dans le `config.toml` de l'agent.

L'instrument qui tranche est `llm_budget_resolved` avec sa moitié modèle
(mika#2328), et sa lecture est un **geste opérateur** (§ procédure de U4).

**Conséquence de périmètre, et elle est contraignante :** la mesure doit
**établir le modèle par la donnée** — le champ `model` de chaque ligne — jamais
le supposer. C'est la discipline AC3 de mika#1950, et c'est elle qui a fait
tomber la prémisse de l'axe A.

### Contraintes d'accès, nommées plutôt que contournées

Trois surfaces sont hors d'atteinte depuis une session de dispatch, et chacune
est vérifiée :

```bash
echo "[$MIKA_SPIRIT_LOG_FILE]"     # → []            variable non définie
ls /var/log/mika/                  # → No such file or directory
ls ~/.mika/data/                   # → pilot-transcripts  (PAS de mika.db)
gh issue view 1910 --repo …        # → aucune sortie (non authentifié)
```

`~/.mika/data/` ne contient que `pilot-transcripts` : **la base n'est pas plus
lisible que le journal**, donc la route DB de l'axe B est fermée ici aussi — ce
que mika#1950 n'avait pas eu à établir, ayant renvoyé l'axe B au journal seul.

**Conséquence assumée : ce plan ne produit pas l'artefact de mesure**, et ne
peut structurellement pas le produire — le champ qu'il mesure n'existe qu'après
son propre déploiement (D6). L'instrument est écrit et testé ici sur fixtures ;
la mesure est un geste opérateur post-déploiement.

---

## Planning Contract

### D1 — La lacune d'instrumentation se ferme d'abord, parce que rien en aval n'est possible sans elle

L'ordre n'est pas une préférence. Tant que `response_chars` n'existe pas, le
geste de U3 n'a rien à lire, la procédure de U4 n'a rien à prescrire, et le
verdict du ticket n'a rien sur quoi se fonder. U1 est la précondition des trois.

### D2 — La surface mesurée est le JOURNAL, pas la base, et la classification vit dans l'analyseur

Trois raisons, dans l'ordre du poids.

1. **Non-gated.** `turn_usage` est émis indépendamment de `MIKA_STORE_LLM_CALLS`
   — Signal O l'énonce : *« le flux de journal est le canal de mesure primaire
   RT-005 et ne se tait pas quand la persistance DB est désactivée »*. Une mesure
   dont la source peut être coupée par un réglage de bruit n'est pas une mesure.
2. **Un seul puits, `mika ask` compris** (mika#2069). `$MIKA_SPIRIT_LOG_FILE`
   porte tous les tours quelle que soit la porte d'entrée.
3. **La doctrine de Signal O prescrit la classification hors de l'instrument** :
   *« la frontière est définie par l'analyseur hors-ligne, pas cuite dans le
   thermomètre »*. Un analyseur shell sur champs RAW est donc la forme
   **doctrinalement correcte**, pas une duplication — et c'est ce qui réconcilie
   D2 avec R7 : la *règle* a un site de définition en Rust, l'*analyse* d'un
   corpus de journal est un consommateur, et le test de U3 épingle leur accord.

**Prime hard condition #1 respectée.** `response_chars` est un **compte**, donc
une dimension RAW, jamais une classification : ni `is_empty`, ni `phase`, ni
`role`. Le seuil qui fait d'un compte une classe est posé par l'analyseur.

**`null` n'est jamais `0`** — la règle de mika#2331, écrite mot pour mot sur
`request_bytes` dans le même struct : `null` dit « non mesuré », `0` dit
« mesuré et vide ». C'est très exactement la distinction dont R5 montre que son
absence produit un nombre faux.

### D3 — La moitié base est secondaire, gated, et sert le diagnostic — pas la mesure

Écrire `response_text` sur la ligne de continuation (U2) ne sert pas à compter :
il sert à **lire une occurrence** quand le compte en signale une, ce qui est
l'investigation 2 du ticket (« turn-by-turn analysis »). Elle est gated par
`MIKA_STORE_LLM_CALLS` et n'est donc jamais la source du verdict.

Elle est quasi gratuite : U1 doit déjà faire voyager le texte produit jusqu'à
`save_continuation_llm_call`, et U2 est le geste de le passer à `save_llm_call`
au lieu de `None`.

### D4 — Aucune migration de schéma

Le réflexe serait d'ajouter `llm_calls.response_chars` (v53 → v54). **Refusé :**
la colonne `response_text` existe déjà et son compte en est dérivable, donc une
colonne de comptage serait une **seconde mesure de la même grandeur** — la
duplication que R7 refuse par ailleurs. Le compte vit sur le journal, le contenu
sur la base, chacun à un seul endroit.

### D5 — Trois issues sont prévues, aucune n'est promise

Les trois conduites sont fixées **avant** la mesure, pour que le résultat ne
choisisse pas sa lecture.

1. **La classe est absente sur les tours GLM de mika-dev** → c'est la « mesure
   fraîche montrant la classe disparue » que le commentaire du 2026-08-30 pose
   comme condition, et mika#1910 devient **fermable par l'opérateur**. Ce plan
   ne le ferme pas (D6).
2. **La classe est présente** → le travail de cause racine s'ouvre **avec un
   compte** au lieu d'une intuition, et la règle 2 du doc de doctrine
   (« switch, gate, or accept in writing ») devient une décision opérateur
   instruite. Le swap reste hors périmètre (mika#1190 : pas d'échange de modèle
   sans calibration passante).
3. **Il n'y a pas de population** — mika-dev ne produit aucun tour de
   continuation sur la fenêtre → c'est un **résultat**, pas un échec : il dit
   que la classe n'est pas atteignable par ce chemin et nomme quoi instrumenter
   ensuite. Ce cas est explicitement prévu parce que `max_steps = 20` n'est pas
   les `max_turns = 30` du harnais d'origine.

### D6 — Rien n'est promu, rien n'est fermé, et l'artefact est un geste post-déploiement

Aucun `openrouter_model` / `zai_model` ne bouge ; aucun fichier sous
`docs/eval/calibration/mika-qa-2328/` n'est touché (la promotion de mika-qa est
gouvernée par mika#2328, dont la condition est conjonctive et plus stricte).
mika#1910 n'est pas fermé : KTD5 du plan mika#1996 l'a déjà tranché, et la
fermeture reste un geste opérateur.

**Délai nommé :** la fenêtre de mesure commence au déploiement d'U1. Le README
d'U4 porte ce délai et le nombre de tours minimal, pour qu'un compte pris trop
tôt ne se lise pas comme une classe absente — qui est la forme la plus
dangereuse du faux vert ici.

---

## Implementation Units

### U1 — `response_chars` sur `turn_usage`, sur les trois sites (la lacune)

Ajouter `response_chars: Option<i64>` à `TurnUsageFields`
(`agent_loop/mod.rs:8396`), en dernier champ, avec un doc-comment calqué sur
celui de `request_bytes` juste au-dessus (mika#2331) et portant la règle
`null ≠ 0`. Nouveau paramètre de `build_turn_usage_fields`, nouveau champ de
`emit_turn_usage`.

**Sémantique, énoncée plutôt que devinée :** `response_chars` est le nombre de
caractères de la sortie de `mika_common::llm::serialize_response_text` — **le
même sérialiseur qui alimente `llm_calls.response_text`**. Deux mesures de « la
réponse » qui divergeraient seraient un second lecteur ; l'identité de source est
donc la propriété, pas un détail d'implémentation.

Deux conséquences de ce choix, qu'il faut écrire au site :

- Ce sérialiseur **inclut les appels d'outil** sous forme
  `[Tool Call: nom(args)]`. Sur un tour de boucle, `response_chars > 0` n'implique
  donc pas « du texte a été produit » — l'analyseur doit le lire **avec**
  `tool_use_in_turn`. Sur le tour de continuation les outils sont désactivés, donc
  la mesure y est bien du texte seul : **c'est là que la classe mika#1910 vit, et
  c'est là que la sémantique est non ambiguë.**
- `serialize_response_text` applique `strip_internal_tags` et rend `None` si le
  résultat est vide. Une réponse composée **uniquement** de balises internes
  compte donc `0`. Faux positif réel, borné, nommé au site et repris en Risks.

Les trois sites de production :

| site | ce qui est passé |
|---|---|
| `mod.rs:1570` (boucle, `Ok`) | le compte du `response_text` déjà calculé à `mod.rs:1496` |
| `mod.rs:1595` (boucle, `Err`) | `None` — aucune réponse n'existe, et `0` y serait un mensonge lisible |
| `mod.rs:907` (continuation) | le compte du texte produit, `None` sur les bras erreur/timeout |

Le troisième exige de faire voyager le texte jusqu'à
`save_continuation_llm_call`, qui ne le reçoit pas aujourd'hui — même
trajectoire que celle que mika#2331 a ouverte pour `request_bytes` et
`system_prompt_bytes`, et pour la même raison.

**Garde structurelle.** Un scan de source refusant un appel à
`build_turn_usage_fields` en production qui passe un `response_chars` littéral
`None` **hors du bras erreur**. Motif : retirer la mesure ne casse aucune
assertion existante — la boucle continue de fonctionner, tous les tests restent
verts, et seule la ligne redevient muette. C'est très exactement la classe que
`mika2342_every_llm_call_is_wrapped_in_a_timeout` a dû fermer par un scan, avec
le même raisonnement écrit. **Allowlist livrée vide** (voir Fire-Disposition).

### U2 — `response_text` sur la ligne de continuation (secondaire, gated)

À `mod.rs:957-958`, remplacer les deux `None` par le texte produit et son
`reasoning`. Aucune nouvelle classe de donnée n'est introduite : c'est le même
contenu, par le même sérialiseur, sous le même plafond de 50 000 caractères,
déjà écrit par le bras `Ok` de la boucle.

### U3 — `scripts/measure-empty-turns` (le geste reproductible)

Script de lecture seule sur `$MIKA_SPIRIT_LOG_FILE`, sans dépendance hors `jq`,
sur la forme de `scripts/measure-pilot-cycle-emptiness` — et pour la raison que
son en-tête énonce : *« la mesure du 2026-08-29 n'a laissé aucun geste
reproductible […] c'est pourquoi le ticket est resté ouvert un mois »*. La
procédure §5 de mika#1950 est deux `grep | jq` qui **déversent** des lignes sans
définir de prédicat : l'opérateur reçoit un tuyau et doit inventer la règle sur
place. C'est la moitié où mika#1950 a commis, en petit, la faute qu'il
diagnostique.

**Le tour est l'unité, pas l'appel.** Un tour est un `trace_id` ; sa ligne de
continuation est celle dont `step == 4294967295` (le sentinelle `u32::MAX`,
partagé par le journal et la base). Un `trace_id` sans ligne de continuation
n'a pas atteint `max_steps` et **n'entre pas dans la population** — ce n'est pas
un tour vide, c'est un tour normal.

**Classification par ligne de continuation, dans cet ordre :**

| ordre | condition | classe | est-ce mika#1910 ? |
|---|---|---|---|
| 1 | `status == "error"` | `error` | non — famille mika#2357 (R6) |
| 2 | `response_chars == null` | `undetermined` | **non** — non mesuré (R5) |
| 3 | `stop_reason == "MaxTokens"` et `output_tokens > 0` et `response_chars == 0` | `reasoning_budget_exhausted` | non — mika#1665 (R7) |
| 4 | `response_chars == 0` | `empty_response` | **oui — la classe** |
| 5 | `response_chars > 0` | `produced` | non |

L'ordre est portant : intervertir 2 et 4 rend la classe fausse sur toute ligne
antérieure au déploiement, et intervertir 1 et 4 fait compter mika#2357 sous
mika#1910.

**Sortie :** une ligne JSON par tour (`trace_id`, `session_id`, `date`, `model`,
`provider`, `stop_reason`, `output_tokens`, `response_chars`, `status`,
`tool_use_in_turn`, `classe`), plus une agrégation **par modèle × mois ×
classe**. Le champ `model` est rapporté, jamais supposé (R8).

**Invariants :**

- **Fail-safe vers `undetermined`.** Champ absent, ligne illisible, JSON
  invalide → `undetermined`, jamais `empty_response` ni `produced`. Un signal
  qu'on ne peut pas lire n'est jamais un terme satisfait.
- **Le vocabulaire des classes est un format de fil**, d'un seul lieu : deux
  orthographes d'une classe couperaient une population en deux sans le dire.
- **Lecture seule**, aucun écrit hors du fichier de sortie passé en argument.

### U4 — `docs/eval/mika-1910/README.md` (protocole, procédure, haltes)

Sur la forme de `docs/eval/mika-1950/README.md` : un document qui **produit** sa
conclusion sur une sortie machine et porte ses haltes. Sections :

1. Ce que le ticket prescrivait, et pourquoi les rejeux ne sont pas exécutables
   (R3, avec les cinq commandes et leur halte).
2. Le renvoi à l'axe A **déjà clos** (R1) — pour qu'aucune relecture ne le
   refasse.
3. **La lacune R4**, avec le tableau des trois sites et la raison pour laquelle
   ni la base ni le journal ne pouvaient répondre avant U1.
4. Le prédicat de U3, son ordre, et les trois classes qu'il sépare (R5/R6/R7).
5. **La procédure opérateur**, non exécutée ici :

```bash
# 1. Quel modèle tourne réellement pour mika-dev, et par quelle porte (mika#2328)
grep llm_budget_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-dev")
        | {provider, model, model_source, model_config_key,
           llm_max_tokens, max_tokens_source}'

# 2. La mesure
scripts/measure-empty-turns --agent mika-dev --out measurement.jsonl

# 3. Lire une occurrence signalée (gated par MIKA_STORE_LLM_CALLS, cf. D3)
#    SELECT model, stop_reason, output_tokens, response_text
#      FROM llm_calls WHERE trace_id = '<trace_id>' AND step = 4294967295;
```

6. **Les haltes**, dont les trois qui comptent :
   - `model_source` vaut `process_env` → une variable fleet-wide écrase le
     `config.toml` per-agent ; le remède est de la **retirer**, pas de toucher
     une valeur.
   - Zéro ligne de continuation sur la fenêtre → **issue 3 de D5**, pas « classe
     absente » : `max_steps = 20` n'est pas `max_turns = 30`, et conclure ici
     serait le faux vert que ce plan existe pour éviter.
   - Le modèle mesuré n'est ni `glm-5.2` ni `glm-5.3` → la mesure ne dit rien du
     ticket ; **établir le déploiement avant toute conclusion** (classe
     mika#2340).
7. Ce que ce travail n'achète pas.

### U5 — Note de statut sur mika#1910 (texte fourni, pose = opérateur)

Le README fournit le texte : ce qui a été instrumenté, ce qui reste à mesurer,
le geste qui le mesure. **Pas de fermeture** (D6).

---

## Fire-Disposition

Deux livrables de classe détecteur, et leur disposition diffère parce que leur
population existante diffère.

**La garde structurelle d'U1 → (a) exception d'allowlist nommée, livrée VIDE.**
Les trois sites de production migrent dans le même commit ; aucun n'est exempté.
L'allowlist est donc livrée vide, et un test assertionne qu'elle le reste — *une
allowlist née vide est l'endroit où la prochaine violation serait déposée*
(mika#2323). **Résolution quand la garde tire : retirer le site fautif, jamais
l'allowlister.** Un quatrième site d'émission est un `halt-and-surface` : savoir
s'il mesure une réponse est une question que la garde ne peut pas trancher pour
son auteur.

**Le prédicat de U3 → propriété auto-nettoyante, sans liste d'exceptions.** La
« donnée existante » est ici l'ensemble des lignes `turn_usage` antérieures au
déploiement d'U1. Elles ne portent pas `response_chars`, donc la règle 2 du
tableau les classe `undetermined` **par construction** — pas par exception, pas
par date codée en dur, pas par une liste à maintenir. La propriété se nettoie
d'elle-même à mesure que la fenêtre avance, et le README nomme le délai (D6).
Aucune ligne historique n'est réécrite.

---

## Verification Contract

1. **Précondition R3 — halte si la prémisse porteuse tombe.** Les cinq commandes
   de `docs/eval/mika-1950/README.md` §1 sont rejouées **avant toute autre
   unité** ; leur résultat attendu est vide. Si l'une rend non-vide : **halte**,
   les fixtures existent, les rejeux redeviennent exécutables et le recadrage
   doit être **re-plaidé**, pas contourné. La raison de cette halte est écrite :
   le corps du ticket affirme des traces « mergé[e]s sur mika-platform via
   PR#192 », et une PR d'un autre dépôt n'est pas couverte par un `git log --all`
   local.
2. **La lacune R4 est prouvée avant d'être fermée.** Un test assertionne qu'un
   tour de continuation **productif** rend un `response_chars > 0` — et un
   contrôle négatif, exécuté contre l'état d'avant U1, montre que la même
   fixture ne portait aucun champ. Sans ce contrôle, « la lacune est fermée » est
   indistinguable de « la lacune n'existait pas ».
3. **Anti-vacuité du prédicat, dans les deux sens.** Un tour de continuation à
   `response_chars: 0` / `status: "success"` / `stop_reason: "EndTurn"` rend
   `empty_response` ; un tour à `response_chars: 240` rend `produced`. La
   direction positive est portante : un script qui ne saurait dire que
   `empty_response` passerait tous les contrôles négatifs et ne vaudrait rien.
4. **Les trois contrôles négatifs, un par confusion, et séparément.**
   - **N1 (R5)** — `response_chars: null` rend `undetermined`, jamais
     `empty_response`. C'est le test dont l'absence rendrait la mesure fausse
     sans la rendre rouge.
   - **N2 (R6)** — `status: "error"`, `input_tokens: 0` rend `error`, jamais
     `empty_response` : mika#2357 ne se compte pas sous mika#1910.
   - **N3 (R7)** — `stop_reason: "MaxTokens"`, `output_tokens: 1200`,
     `response_chars: 0` rend `reasoning_budget_exhausted`, jamais
     `empty_response`. Les trois mêmes cas sont assertionnés contre la règle de
     `failure.rs:79`, pour que l'accord des deux consommateurs soit épinglé
     plutôt qu'espéré.

   **Séparément, et non par une fixture qui neutralise les trois à la fois** :
   une conjonction de termes fail-safe ne se prouve pas en les invalidant
   ensemble (leçon mika#2277).
5. **N4 — la granularité du tour.** Une ligne de boucle (`step != 4294967295`)
   à `response_chars: 0` ne fait pas, à elle seule, de son `trace_id` un tour de
   la classe : un tour qui n'émet que des appels d'outil à l'étape 3 est
   nominal.
6. **Aucune valeur de production n'est modifiée.** `git diff` ne touche ni
   `well_known_agents.rs`, ni un `config.toml`, ni
   `docs/eval/calibration/mika-qa-2328/`, ni `docs/eval/mika-1950/`.
7. **Aucune migration.** `git diff` ne touche pas `db/migrations.rs` et le
   `schema_version` reste **v53** (D4).

---

## Definition of Done

- [ ] Précondition R3 rejouée et vide (Verification Contract §1) ; sinon halte.
- [ ] `response_chars: Option<i64>` sur `TurnUsageFields`, renseigné sur les
      trois sites de production, `None` sur le bras erreur et **jamais `0`** là.
- [ ] Le texte produit voyage jusqu'à `save_continuation_llm_call` ; la ligne de
      continuation écrit `response_text` et `reasoning` (U2).
- [ ] Garde structurelle livrée, **allowlist vide**, avec son test de
      non-régression de vacuité.
- [ ] `scripts/measure-empty-turns` livré, lecture seule, avec ses tests :
      anti-vacuité dans les deux sens, N1/N2/N3 séparés, N4.
- [ ] `docs/eval/mika-1910/README.md` : R1/R3/R4 avec leurs commandes et sites,
      le prédicat et son ordre, la procédure opérateur, les trois haltes, ce que
      le travail n'achète pas.
- [ ] Texte de la note mika#1910 fourni ; ticket **non fermé**.
- [ ] Aucun modèle promu ; `MIKA_DEV_CONFIG` et `MIKA_QA_CONFIG` inchangés ;
      `schema_version` inchangé.
- [ ] Le corps de PR nomme que l'axe B n'était pas mesurable, pourquoi, et que
      la mesure est un geste post-déploiement (mika#2211 : corps écrit sous le
      worktree).

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de ses cinq investigations numérotées, de la
condition de fermeture posée par son commentaire du 2026-08-30, et des faits
R1–R8.

- **AC1** — L'inexécutabilité des investigations 1 à 3 (rejeu des deux fixtures)
  est **établie et vérifiable** : le README porte les commandes, elles sont
  rejouées avant toute autre unité, et la halte est nommée si l'une rend
  non-vide.
- **AC2** — La **lacune d'instrumentation** est établie par les sites de code
  (`mod.rs:942` contre `mod.rs:1500`) et **fermée** : un tour de continuation
  rapporte désormais ce qu'il a produit, sur le journal non-gated.
- **AC3** — Le modèle réellement en service dans la population mesurée est
  **rapporté par la donnée** (champ `model`), jamais supposé — et la porte de la
  cascade qui le pose est lisible par `llm_budget_resolved`.
- **AC4** — Les **trois classes sont séparées** par le prédicat, chacune avec son
  contrôle négatif : mika#1910 (`empty_response`), mika#2357 (`error`),
  mika#1665 (`reasoning_budget_exhausted`). Un `null` n'est jamais compté comme
  vide.
- **AC5** — La mesure est **reproductible** : une commande unique la refait, sur
  la surface non-gated.
- **AC6** — La règle de mika#1665 a **un seul site de définition**
  (`failure.rs:79`) et l'accord de ses deux consommateurs est épinglé par test.
- **AC7** — **Aucune promotion, aucune fermeture, aucune migration** : ni
  `glm-5.3` vers mika-dev ou mika-qa, ni fermeture de mika#1910, ni
  `schema_version` déplacé. La procédure mika#2328 n'est pas touchée.
- **AC8** — Le fait que **l'artefact de mesure ne soit pas produit par ce
  travail** est écrit, avec sa raison structurelle (le champ n'existe qu'après le
  déploiement) et le délai minimal avant qu'un compte soit lisible.
- **AC9** — Les investigations 4 et 5 du ticket (comparaison OpenRouter ↔ Z.AI
  native, sensibilité au prompt système) sont **nommées hors périmètre** avec
  leur précondition : elles comparent des populations, et il n'y a pas de
  population avant AC2.

---

## Risks

| Risque | Conduite |
|---|---|
| **Il n'y a pas de population** : mika-dev ne produit aucun tour de continuation sur la fenêtre (`max_steps = 20` ≠ `max_turns = 30`). | Issue 3 de D5, prévue et nommée comme **résultat**. La halte du README interdit de lire ce zéro comme « classe absente ». |
| **Un compte pris trop tôt se lit comme une classe absente.** | Le README porte le délai et un nombre de tours minimal ; AC8 l'exige. C'est le faux vert le plus dangereux de ce travail. |
| **Faux positif de `strip_internal_tags`** : une réponse faite uniquement de balises internes compte `0`. | Borné et nommé au site. Si la mesure en produit, le discriminant est `output_tokens > 0` avec `response_chars == 0` **hors** `MaxTokens` — une population à ouvrir en ticket de suivi, pas à absorber dans le prédicat. |
| **`response_chars` inclut les appels d'outil** sur les tours de boucle. | Sémantique écrite au site ; la classe ne se décide que sur le tour de continuation, où les outils sont désactivés. N4 l'épingle. |
| **Second lecteur de la règle mika#1665.** | R7 : un site de définition, deux consommateurs, accord épinglé par test (Verification Contract §4). |
| **Confusion avec mika#2357** — compter une population glm-5.3 sous un ticket glm-5.2. | Règle 1 du prédicat, contrôle négatif N2, et le champ `model` rapporté. |
| **La garde structurelle tire sur un quatrième site d'émission.** | Halt-and-surface (Fire-Disposition) : l'allowlist reste vide. |
| **`MIKA_STORE_LLM_CALLS` désactivé** rend U2 muet. | Par conception (D3) : la mesure est sur le journal non-gated ; seule l'inspection d'une occurrence est gated, et le README le dit. |
| **Empiéter sur mika#2328.** | Aucun fichier sous `docs/eval/calibration/mika-qa-2328/` n'est touché ; le README y renvoie pour la promotion de mika-qa. |
| **Refaire l'axe A.** | R1 : l'axe A est clos, ses artefacts sont dans l'arbre, et `docs/eval/mika-1950/` n'est pas touché. |

---

## Ce que ce travail n'achète pas

- **Il ne mesure pas.** Il rend l'axe B **mesurable** et livre le geste ; la
  mesure est un geste opérateur post-déploiement, et le dire est AC8.
- **Il ne corrige pas la cause racine.** Si la classe est présente, le travail
  de cause s'ouvre avec un compte — ce qui est strictement plus que ce que le
  ticket a aujourd'hui.
- **Il ne ferme pas mika#1910** et ne promeut aucun modèle (D6).
- **Il ne compare pas les fournisseurs** (investigation 4) ni les variantes de
  prompt système (investigation 5). Les deux comparent des populations, et il
  n'y a pas de population avant AC2 — précondition écrite, ticket de suivi.
- **Il ne réconcilie pas la dérive dépôt ↔ runtime de mika-dev** (R8) : c'est son
  propre travail, que mika#2296 D2 nomme et n'ouvre pas.
- **Il ne reconstitue pas la campagne hy3-v0.** Ces artefacts sont perdus (R3).

---

## Sources

- `docs/eval/mika-1950/README.md` — l'axe A mesuré et clos, la procédure §5 de
  l'axe B, et sa §9 qui laisse mika#1910 ouvert.
- `scripts/measure-pilot-cycle-emptiness` + son test — la forme d'un geste
  reproductible, et l'en-tête qui dit pourquoi il existe.
- `docs/solutions/best-practices/no-substrate-on-open-failure-mode-2026-08-30.md`
  — la règle, et sa rectification d'attribution du 2026-09-20.
- `crates/mika-agent/src/agent_loop/mod.rs:882-968` —
  `save_continuation_llm_call` et ses deux `None` : **le site de la lacune**.
- `crates/mika-agent/src/agent_loop/mod.rs:1495-1554` — les deux sites de la
  boucle, dont le bras `Ok` qui écrit `response_text` : **le contraste**.
- `crates/mika-agent/src/agent_loop/mod.rs:8386-8460` — `TurnUsageFields`, le
  doc-comment `request_bytes` de mika#2331 et la règle `null ≠ 0`.
- `crates/mika-agent/src/agent_loop/mod.rs:85` — `EMPTY_RESPONSE_FALLBACK`.
- `crates/mika-agent/src/calibration/failure.rs:70-133` —
  `classify_failure` : **le site de définition** du discriminant mika#1665.
- `crates/mika-common/src/llm/mod.rs:370-404` — `serialize_response_text`, son
  plafond, son inclusion des appels d'outil et son `strip_internal_tags`.
- `crates/mika-common/src/claude.rs:280-285` — `StopReason::MaxTokens`.
- `crates/mika-agent/src/well_known_agents.rs:165-188` — `MIKA_DEV_CONFIG` et le
  doc-comment qui nomme la dérive dépôt ↔ runtime.
- `docs/dormeurs.md` — l'entrée mika#2357 et sa condition de réveil (R6).
- Racine `CLAUDE.md` § Signal O — `turn_usage`, son caractère non-gated, la
  Prime hard condition #1 et la règle `null ≠ 0` de mika#2331.

---

## Revision history

- **2026-09-21** — v1. Plan initial. L'axe A est établi clos (mika#1950) ; le
  périmètre est recadré sur l'axe B seul ; et le constat central — **l'axe B
  n'est mesurable par aucun instrument existant, le tour de continuation
  n'enregistrant pas ce qu'il a produit** — déplace le travail de « mesurer »
  vers « rendre mesurable, puis livrer le geste ». Aucun modèle ne bouge, aucune
  migration, le ticket n'est pas fermé.
