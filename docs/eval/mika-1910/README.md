# mika#1910 — l'axe B n'était mesurable par aucun instrument ; il l'est

**Ticket :** senara-solutions/mika#1910
**Geste :** `scripts/measure-empty-turns`
**Artefact de mesure :** **aucun, et c'est structurel** — voir §7.

> **En une ligne.** Le tour de continuation — très exactement là où la classe
> mika#1910 se manifeste dans le moteur mika — n'enregistrait **pas ce qu'il
> avait produit**. Ni la base ni le journal ne pouvaient compter la classe. Ce
> travail pose la valeur manquante, livre le geste qui la lit, et **refuse de
> rendre un verdict qu'il ne peut pas fonder** : la mesure est un geste opérateur
> post-déploiement, parce que le champ qu'elle lit n'existe qu'après lui.

**Aucun modèle n'est promu. mika#1910 n'est pas fermé.**

---

## 1. Ce que le ticket prescrivait, et pourquoi les rejeux ne sont pas exécutables

Les investigations 1 à 3 du corps du ticket (reproductibilité, analyse tour par
tour, `max_turns=60`) reposent toutes sur deux fichiers :
`docs/eval/hy3-v0-2026-08-18/traces/run_glm-5.2_{1878,1817}.jsonl`.

Les cinq commandes ci-dessous **doivent toutes retourner vide**. Elles ont été
rejouées avant toute autre unité de ce travail (Verification Contract §1 du
plan), et elles l'étaient — `ls docs/eval/` rend `calibration` et `mika-1950`,
jamais `hy3-v0-2026-08-18`.

```bash
ls docs/eval/                                                  # → pas de hy3-v0-*
git log --all --oneline -- 'docs/eval/hy3-v0-2026-08-18/**'    # → vide
git log --all --oneline --grep="hy3"                           # → vide
find /data/workspace/mika-platform -name "run_glm*" -not -path "*/target/*"  # → vide
find /data/workspace/mika-platform -name "*hy3*"   -not -path "*/target/*"   # → vide
```

> **HALTE.** Si l'une de ces commandes retourne non-vide, cette section n'est
> plus fondée : le rejeu redevient exécutable et le recadrage doit être
> **re-plaidé**, pas contourné. Le corps de mika#1910 affirme des traces
> « mergé[e]s sur mika-platform via PR#192 » — une PR d'un **autre dépôt** n'est
> pas couverte par un `git log --all` local, donc l'espace de recherche peut
> être incomplet.

**Fabriquer des fixtures « équivalentes » est refusé.** Une fixture construite
depuis la définition du symptôme ne peut pas réfuter cette définition
(règle 6 de
[`cycle-non-empty-detector-2026-08-30.md`](../../solutions/best-practices/cycle-non-empty-detector-2026-08-30.md)).

**Investigations 4 et 5 — hors périmètre, avec leur précondition.** La
comparaison OpenRouter ↔ Z.AI native et la sensibilité au prompt système
comparent des **populations** ; il n'y a pas de population avant que la §3
ci-dessous soit déployée et qu'un compte existe. Ticket de suivi, pas un oubli.

---

## 2. L'axe A est mesuré et clos — ne pas le refaire

mika#1950 a livré, et le résultat est dans l'arbre :
[`../mika-1950/README.md`](../mika-1950/README.md),
`scripts/measure-pilot-cycle-emptiness`, et
[`../mika-1950/measurement-2026-09-20.jsonl`](../mika-1950/measurement-2026-09-20.jsonl).

Verdict de l'axe A : **sans objet, prémisse réfutée**. Sur 2353 sessions
`claude-pilot` (2026-05-15 → 2026-09-20), l'événement
`[guardrail] empty_response:` survient **0 fois**, et **zéro** de ces sessions ne
tourne sous GLM (`claude-opus-4-8[1m]` 1376, `claude-opus-5[1m]` 288, 686 sans
modèle rapporté).

```bash
jq -s 'map(select(.empty_event)) | length' ../mika-1950/measurement-2026-09-20.jsonl   # → 0
```

**Deux axes, et les confondre est la faute que mika#1950 a dû nommer :**

| Axe | Population | Source | Statut |
|---|---|---|---|
| **A** — cycles pilote vides | sessions `claude-pilot` (**implémentation**, tourne sous Claude) | `/var/log/claude-pilot/*.stderr` | **clos** (mika#1950) |
| **B** — tours GLM vides | tours de `mika-dev` (**dispatcheur**, porte GLM) | `$MIKA_SPIRIT_LOG_FILE` | **le périmètre de ce document** |

Le `README.md` §9 de mika#1950 le dit par écrit : *« mika#1910 n'est pas fermé
ici. […] La moitié GLM du symptôme — les tours de `mika-dev` — n'est pas mesurée
ici. »* Rien de l'axe A n'est à reprendre, et `docs/eval/mika-1950/` n'est pas
touché.

---

## 3. La lacune : l'axe B n'était mesurable par aucun instrument existant

C'est le constat central, et il a déplacé le travail de « mesurer » vers « rendre
mesurable, puis livrer le geste ».

### Où la classe atterrit dans le moteur

Le ticket décrit 30 tours brûlés puis un message final vide. Traduit dans
l'architecture mika : `max_steps` (20) épuisé → `LoopResult::MaxStepsExceeded` →
`attempt_continuation_turn` (outils désactivés, budget écrêté) → si ce tour ne
rend rien, le moteur sert `EMPTY_RESPONSE_FALLBACK`, littéralement `"Done."`.
**Le symptôme terminal de mika#1910 vit donc sur le tour de continuation**, et
nulle part ailleurs.

### Ce que ce tour enregistrait

`save_continuation_llm_call` passait `None, None` aux positions `response_text`
et `reasoning` — **sur toutes ses branches, succès compris**. L'asymétrie était
dans le même fichier, à soixante lignes :

| site | chemin | `response_text` écrit ? |
|---|---|---|
| boucle, bras `Ok` | `run_loop` | **oui** |
| boucle, bras `Err` | `run_loop` | non — *correct*, aucune réponse n'existe |
| **continuation, toutes branches** | `attempt_continuation_turn` | **non — la lacune** |

Conséquence, et elle est dure : sur le tour de continuation,
`response_text IS NULL` était vrai **à 100 %**, qu'il ait produit un résumé ou
rien. **La colonne qui porterait la vacuité était inconditionnellement nulle sur
la seule ligne qui compte.**

### Et le journal non plus

`turn_usage` (Signal O) portait `step`, `stop_reason`, `output_tokens`,
`tool_use_in_turn`, `status`, `request_bytes`, `system_prompt_bytes` — **aucune
mesure du texte produit**. Le champ `response_chars` n'existait pas.

Les deux surfaces que la procédure §5 de mika#1950 prescrit étaient donc muettes
sur la question même que le ticket pose. Ce n'était pas un défaut de cette
procédure : **une lacune d'instrumentation en amont d'elle.**

### Le piège autour duquel ce travail est construit

`response_text IS NULL` recouvre **trois populations distinctes** :

| population | `response_text` | est-ce une sortie vide ? |
|---|---|---|
| (a) lignes antérieures à la v31 | NULL | **inconnu** — la colonne n'existait pas |
| (b) appels en erreur | NULL | **non** — aucune réponse n'existe |
| (c) **tout** tour de continuation, succès compris | NULL | **non déterminé** — la lacune |

Un prédicat qui lirait la vacuité comme « la valeur est absente » compterait les
trois, avec un taux de faux positifs de **100 % sur exactement la population
d'intérêt**. Le nombre produit serait faux, plausible, et porté par l'autorité
d'une mesure.

**Corollaire de conception : la mesure ne peut pas être dérivée de l'absence
d'une valeur.** Il faut une valeur **posée**, et c'est `response_chars`.

### Ce qui est livré

- **`response_chars` sur `turn_usage`**, sur les trois sites d'émission, sur la
  surface **non gatée** par `MIKA_STORE_LLM_CALLS` — le canal de mesure primaire
  ne doit pas se taire quand on coupe la persistance DB pour réduire le bruit.
  `null` veut dire « non mesuré », `0` veut dire « mesuré, et rien n'est sorti »
  (la règle mika#2331 écrite mot pour mot sur `request_bytes`, un champ plus
  haut). C'est **un compte**, donc une dimension RAW, jamais une classification :
  le seuil qui en fait une classe est posé par l'analyseur (Prime hard
  condition #1).
- **`response_text` et `reasoning` sur la ligne de continuation** — secondaire,
  gaté, pour **lire** une occurrence quand le compte en signale une
  (investigation 2 du ticket). Jamais la source du verdict.
- **Une garde structurelle** : `agent_loop::tests::mika1910_every_unmeasured_site_declares_itself`
  refuse un `None` nu à la position `response_chars`, et son voisin
  `…_the_inventory_of_emission_sites_is_closed` fige l'inventaire à **trois**
  sites. Retirer la mesure ne casse **aucune assertion** — la boucle continue de
  fonctionner, tous les tests restent verts, et seule la ligne redevient muette.
  Une régression qui ne rend rien faux, seulement quelque chose d'invisible, est
  la classe que cette maison garde par scan de source. **Allowlist vide :** un
  quatrième site est un `halt-and-surface`, jamais une entrée d'exception —
  savoir s'il a une réponse à mesurer est une question que la garde ne peut pas
  trancher pour son auteur.

**Aucune migration** (`schema_version` reste **v53**) : `response_text` existe
déjà et son compte en est dérivable, donc une colonne de comptage serait une
seconde mesure de la même grandeur. Le compte vit sur le journal, le contenu sur
la base, chacun à un seul endroit.

---

## 4. Le prédicat, et pourquoi son ordre est portant

`scripts/measure-empty-turns` lit `$MIKA_SPIRIT_LOG_FILE` en lecture seule, sans
dépendance hors `jq`.

**Le tour est l'unité, et la clé est `trace_id` — jamais `session_id`.**
`run_agent` pose un `trace_id` par invocation, et une **session** en contient
autant que de tours. Une session `mika-dev` dont deux tours épuisent `max_steps`
porte donc légitimement **deux** lignes de continuation ; un analyseur qui
dédupliquerait par session sous-compterait exactement les sessions où la classe
se manifeste le plus. Un `trace_id` sans ligne de continuation n'a pas atteint
`max_steps` et **n'entre pas dans la population** — c'est un tour normal, pas un
tour vide.

**La ligne de continuation est celle dont `step == 4294967295`** (le sentinelle
`u32::MAX`, partagé par le journal et la base).

### Les six règles, dans cet ordre

| ordre | condition | classe | est-ce mika#1910 ? |
|---|---|---|---|
| **0** | plus d'une ligne de continuation pour ce `trace_id` | `undetermined` *(+ compteur `duplicate_continuation`)* | non |
| 1 | `status == "error"` | `error` | non — famille mika#2357 |
| 2 | `response_chars` nul / absent / illisible | `undetermined` | **non** — non mesuré |
| 3 | `stop_reason == "MaxTokens"` **et** `output_tokens > 0` **et** `response_chars == 0` | `reasoning_budget_exhausted` | non — mika#1665 |
| 4 | `response_chars == 0` | `empty_response` | **oui — la classe** |
| 5 | `response_chars > 0` | `produced` | non |

**L'ordre est portant, et voici par quoi.** Intervertir 2 et 4 rend la classe
fausse sur **toute** ligne antérieure au déploiement (elles ne portent pas le
champ, donc la règle 2 les classe `undetermined` **par construction** — sans
allowlist, sans date codée en dur ; la propriété se nettoie d'elle-même à mesure
que la fenêtre avance, et aucune ligne historique n'est réécrite). Intervertir 1
et 4 fait compter une population **glm-5.3** sous un ticket **glm-5.2** :

| | mika#1910 | mika#2357 |
|---|---|---|
| modèle | `glm-5.2` | `glm-5.3` |
| `status` | `success` | `error` |
| `input_tokens` | *normaux* | **0** |
| `stop_reason` | `ToolUse` / `EndTurn` | `error` |

**La règle 3 n'est pas ré-dérivée ici.** Elle appartient à
`calibration::failure::classify_failure` — texte vide **+** plafond de sortie
atteint **+** `output_tokens > 0` ⇒ budget de raisonnement épuisé (remédié par un
réglage), **et non** une régression modèle. Un site de définition, deux
consommateurs, accord épinglé par test : deux orthographes de la même règle
couperaient une population en deux sans le dire.

**La règle 0 n'est pas de la paranoïa.** Au plus une ligne de continuation par
tour est une garantie du **moteur** (trois sites d'appel mutuellement exclusifs
d'`attempt_continuation_turn`, une sortie de boucle par invocation), pas du
**format de fil** : `params.trace_id` est fourni par l'appelant, et rien dans le
type n'interdit de le réutiliser. Un doublon est donc classé `undetermined` et
compté à part — jamais deux `empty_response` (ce qui gonflerait la classe d'un
facteur 2 sur exactement la population d'intérêt), jamais une sélection
silencieuse du premier ou du dernier (ce qui absorberait l'anomalie dans un
nombre d'apparence saine). **Régime attendu : zéro.** Toute occurrence nomme un
appelant qui réutilise un `trace_id`, ce qui est un défaut **en amont** de la
mesure et non un réglage de l'analyseur.

**Fail-safe, toujours vers `undetermined`.** Champ absent, valeur illisible,
ligne JSON invalide, `trace_id` manquant — chacun rend `undetermined`, jamais
`empty_response` ni `produced`. Un signal qu'on ne peut pas lire n'est jamais un
terme satisfait.

**Note sur `status == "timeout"`.** Le bras d'écrêtage de deadline de la
continuation écrit `status = "timeout"` et `response_chars = null`, donc il tombe
sous la règle 2 et se lit `undetermined` — correct : l'écrêtage a coupé l'appel,
rien n'est revenu à mesurer. Il n'est **pas** replié dans la règle 1, dont la
population est la famille mika#2357 et doit rester comptable seule.

### Le faux positif nommé, et la requête qui l'isole

`response_chars` compte la sortie de `serialize_response_text`, qui applique
`strip_internal_tags` et rend `None` si le résultat est vide. **Une réponse
composée uniquement de balises internes compte donc `0`** et tombe sous la
règle 4. C'est un faux positif réel et borné ; il n'est **pas** séparable par le
prédicat (les deux produisent exactement les mêmes champs), mais il est
**isolable depuis la sortie** :

```bash
jq 'select(.classe == "empty_response" and .stop_reason == "EndTurn" and .output_tokens > 0)' \
   measurement.jsonl
```

**Conduite : population à ouvrir en ticket de suivi** — la trancher demande le
`response_text` de l'occurrence, donc la route base de l'étape 3 ci-dessous.
**Jamais à absorber dans le prédicat.**

---

## 5. La procédure opérateur (non exécutée ici)

```bash
# 1. Quel modèle tourne RÉELLEMENT pour mika-dev, et par quelle porte (mika#2328)
grep llm_budget_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-dev")
        | {provider, model, model_source, model_config_key,
           llm_max_tokens, max_tokens_source}'

# 2. Le dénominateur : tours de continuation de mika-dev par jour, par modèle.
#    Mesurable AVANT le déploiement — `step` et `model` sont déjà là depuis
#    Signal O, seul `response_chars` est nouveau.
grep turn_usage "$MIKA_SPIRIT_LOG_FILE" \
  | jq -r 'select(.step == 4294967295 and .agent_id == "mika-dev")
           | "\(.timestamp[0:10]) \(.model)"' \
  | sort | uniq -c

# 3. La mesure
scripts/measure-empty-turns --agent mika-dev --out measurement.jsonl

# 4. Lire UNE occurrence signalée (gaté par MIKA_STORE_LLM_CALLS)
#    Vérifier d'abord que le gate est actif : sous un gate désactivé, un
#    résultat vide dit « non stocké » et JAMAIS « non survenu » (halte 5).
#    SELECT model, stop_reason, output_tokens, response_text
#      FROM llm_calls WHERE trace_id = '<trace_id>' AND step = 4294967295;
```

**Le modèle est établi par la donnée, jamais supposé** — c'est la discipline AC3
de mika#1950, et c'est elle qui a fait tomber la prémisse de l'axe A. Le dépôt ne
tranche pas : `MIKA_DEV_CONFIG` déclare `z-ai/glm-5.2` et son propre doc-comment
nomme la dérive ; le registre des dormeurs affirme autre chose. Les deux ne
peuvent pas être vraies en même temps, et la bascule du 2026-08-26 vit **hors du
dépôt**, dans le `config.toml` de l'agent. L'étape 1 est ce qui tranche.

### Quand un compte devient lisible — et ce qu'un zéro réfute

**Critère conjoint : 14 jours ET ≥ 20 tours de continuation sous un modèle GLM**,
le plus tardif des deux.

La moitié **mesurée** est le nombre de tours, et elle est dérivée : à la
fréquence que le corps du ticket affirme (2/5 runs, soit p ≈ 0,40), observer
**zéro** occurrence sur 20 tours a une probabilité de 0,6²⁰ ≈ **3,7 × 10⁻⁵**. Un
zéro sur 20 tours réfute donc la fréquence historique alléguée.

**Ce qu'il ne réfute pas :** à p = 0,05, le même zéro a une probabilité de
0,95²⁰ ≈ **36 %** — il ne dit rien. Réfuter 5 % demande `ln(0,05)/ln(0,95)` ≈
**59 tours**. *« Aucune occurrence sur 20 tours »* et *« la classe a disparu »*
sont deux énoncés différents, et les confondre **est** le faux vert.

La moitié **déclarée** est le plancher de 14 jours : aucune mesure ne le fonde.
Il couvre la variété opérationnelle (semaine ouvrée, week-end, formes de tickets
différentes) qu'une rafale de 20 tours en une heure sur un seul ticket ne couvre
pas. L'opérateur peut le resserrer **une fois le dénominateur connu** — la
requête de l'étape 2 dit en combien de jours 20 tours sont atteints.

### Les trois issues, fixées AVANT la mesure

Pour que le résultat ne choisisse pas sa lecture :

1. **La classe est absente sur les tours GLM de mika-dev** → c'est la « mesure
   fraîche montrant la classe disparue » que le commentaire du 2026-08-30 pose
   comme condition de fermeture, et mika#1910 devient **fermable par
   l'opérateur**. Ce document ne le ferme pas.
2. **La classe est présente** → le travail de cause racine s'ouvre **avec un
   compte** au lieu d'une intuition. Le swap reste hors périmètre (mika#1190 :
   pas d'échange de modèle sans calibration passante).
3. **Il n'y a pas de population** → c'est un **résultat**, pas un échec : voir la
   halte 2.

---

## 6. Les cinq haltes

1. **`model_source` vaut `process_env`** → une variable fleet-wide écrase le
   `config.toml` per-agent. Le remède est de la **retirer de l'environnement du
   service**, pas de toucher une valeur.

2. **Zéro ligne de continuation sur la fenêtre** → c'est l'**issue 3**, pas
   « classe absente ». `max_steps = 20` n'est pas les `max_turns = 30` du
   harnais d'origine, donc la classe peut n'être pas atteignable par ce chemin.
   Conclure ici serait le faux vert que ce travail existe pour éviter. Ce qu'il
   faut noter : la classe n'est pas atteignable par ce chemin, et quoi
   instrumenter ensuite.

3. **Le modèle mesuré n'est ni `glm-5.2` ni `glm-5.3`** → la mesure ne dit rien
   du ticket. **Établir le déploiement avant toute conclusion** (classe
   mika#2340) : un binaire antérieur au correctif n'émet pas `response_chars`
   du tout, et son silence se lit exactement comme une classe absente.

4. **Fenêtre non atteinte** (moins de 14 jours, **ou** moins de 20 tours de
   continuation GLM) → le compte n'est **pas** un verdict, quelle que soit sa
   valeur. Un zéro s'y lit « pas encore mesuré », jamais « classe disparue ».
   Relire les deux nombres du §5.

5. **Occurrences signalées ET `MIKA_STORE_LLM_CALLS` désactivé** → ce n'est
   **pas** une halte du verdict : la mesure vit sur le journal non gaté, donc le
   compte reste valide comme verdict de présence/absence. Ce qui est perdu est
   l'**inspection d'occurrence** (étape 4), et la conduite est nommée pour
   qu'elle ne s'invente pas sur place :
   - *Le `SELECT` rend zéro ligne.* **Il ne dit pas que l'occurrence n'existe
     pas.** `MIKA_STORE_LLM_CALLS=false` n'écrit **aucune** ligne `llm_calls` —
     pas même une ligne à `response_text` nul — donc un résultat vide y signifie
     « non stocké ». Le confondre avec « non survenu » serait, au sein même de
     la procédure, le faux vert que ce document existe pour refuser.
   - *Réarmer le gate ne récupère rien.* La télémétrie n'est pas rétroactive :
     l'occurrence déjà comptée restera non inspectable. Le gate s'arme pour les
     **prochaines**. Conduite : noter le `trace_id` compté, armer
     `MIKA_STORE_LLM_CALLS`, attendre l'occurrence suivante.
   - *Vérifier l'état du gate avant de conclure quoi que ce soit* — un `SELECT`
     vide sous un gate désactivé et un `SELECT` vide sous un gate actif sont
     deux faits opposés qui produisent les mêmes octets (classe mika#2205).

---

## 7. Ce que ce travail n'achète pas

- **Il ne mesure pas.** Il rend l'axe B **mesurable** et livre le geste ; la
  mesure est un geste opérateur post-déploiement. La raison est structurelle et
  non un manque de zèle : **le champ qu'elle lit n'existe qu'après le
  déploiement de ce correctif.** Aucune ligne `turn_usage` déjà écrite ne porte
  `response_chars`, et aucune ne peut être réécrite.
- **Il ne corrige pas la cause racine.** Si la classe est présente, le travail de
  cause s'ouvre avec un compte — strictement plus que ce que le ticket a
  aujourd'hui.
- **Il ne ferme pas mika#1910** et ne promeut aucun modèle. Aucun
  `openrouter_model` / `zai_model` ne bouge, aucun fichier sous
  `docs/eval/calibration/mika-qa-2328/` n'est touché (la promotion de mika-qa est
  gouvernée par mika#2328, dont la condition est conjonctive et plus stricte).
- **Il ne compare pas les fournisseurs** (investigation 4) ni les variantes de
  prompt système (investigation 5) — voir §1.
- **Il ne réconcilie pas la dérive dépôt ↔ runtime de mika-dev.** C'est son
  propre travail ; l'étape 1 de la procédure la **rend lisible**, elle ne la
  corrige pas.
- **Il ne reconstitue pas la campagne hy3-v0.** Ces artefacts sont perdus (§1).
- **Il n'ajoute aucun compteur ni événement de journal nouveau au-delà de
  `response_chars`.** Le seul instrument est le geste du §5, et **son silence ne
  prouve rien tant que personne ne l'a lancé.**

---

## Sources

- [`../mika-1950/README.md`](../mika-1950/README.md) — l'axe A mesuré et clos,
  et sa §9 qui laisse mika#1910 ouvert.
- `scripts/measure-empty-turns` — le geste, et son en-tête qui porte le piège
  des trois populations nulles.
- `scripts/measure-pilot-cycle-emptiness` — la forme d'un geste reproductible.
- [`../../solutions/best-practices/no-substrate-on-open-failure-mode-2026-08-30.md`](../../solutions/best-practices/no-substrate-on-open-failure-mode-2026-08-30.md)
  — la règle, et sa rectification d'attribution du 2026-09-20.
- `crates/mika-agent/src/agent_loop/mod.rs` — `save_continuation_llm_call` (le
  site de la lacune), `TurnUsageFields::response_chars`,
  `RESPONSE_CHARS_UNMEASURED`, et la garde structurelle.
- `crates/mika-agent/src/calibration/failure.rs` — `classify_failure` : **le
  site de définition** du discriminant mika#1665.
- `crates/mika-common/src/llm/mod.rs` — `serialize_response_text`, son plafond,
  son inclusion des appels d'outil et son `strip_internal_tags`.
- Racine `CLAUDE.md` § Signal O — `turn_usage`, son caractère non gaté, la Prime
  hard condition #1 et la règle `null` ≠ `0` de mika#2331.
