---
module: dispatch-lib
tags: [dispatch-lib, claude-pilot, guardrail, drift, enumeration, test-harness, observability]
problem_type: drift
category: best-practices
ticket: mika#2149
---

# Une énumération en commentaire se périme au rythme de l'amont ; une table avec branche par défaut se signale à la première dérive

## Le fait mesuré

`dispatch-lib.sh` affirmait en commentaire que le `subtype` d'une session
`terminated` était « one of stall_detected / empty_response / idle_timeout ».
C'était vrai à l'écriture (mika#1772). Quand mika#2149 a été déposé le
2026-09-03, la liste amont (`GuardrailAbortReason.guardrail`,
`claude-pilot/src/claude_pilot/types.py`) en comptait six. Quand le ticket a
été groomé le 2026-09-21, elle en comptait **huit** — `watchdog_error`
(cpp#168) et `prompt_cache_dead` (cpp#185) s'étaient ajoutés *pendant que le
ticket dormait*. Cinq valeurs de dérive en dix-huit jours, sans qu'un seul
test rougisse, parce qu'aucun test ne lit un commentaire.

Le second symptôme est la conséquence du premier : un `awaiting_model` (« le
modèle n'a jamais rendu le premier jeton du tour suivant ») atteignait
l'opérateur comme `Halt: awaiting_model …`, une chaîne dans une phrase, et
rien de plus. Un arrêt qui distingue huit causes en amont retombait en une
seule en aval, et l'opérateur rouvrait le journal pour savoir s'il fallait
relancer.

## Le remède, et pourquoi ce n'est pas « corriger le commentaire »

Corriger la liste l'aurait remise à jour jusqu'au prochain `cpp#`. Le remède
est de **remplacer l'énumération en prose par une table exécutée** :

1. `_halt_family <subtype>` est un `case` — une ligne par valeur, la branche
   `*)` en dernier. La table *est* l'énumération aval ; le commentaire de tête
   ne liste plus rien, il pointe vers `types.py` (amont) et vers la table.
2. La branche `*)` **dit** ce qu'elle ne connaît pas :
   `dispatch-lib: halt_family.unknown subtype=<x>` sur stderr — donc dans le
   `.stderr` persisté et dans la queue de 10 Ko du callback. La prochaine
   valeur ajoutée en amont laisse une trace à sa **première** occurrence, au
   lieu de rejoindre silencieusement la prose.
3. Un test lit le `Literal` amont et exige une famille ≠ `unknown` pour
   chaque valeur (`test-dispatch-lib.sh`, T6). C'est ce test, pas le
   commentaire, qui rougit le jour où cpp ajoute un motif.

Le callback porte désormais deux lignes à préfixe stable après `Halt:` :

```
Halt: rate_limited (HTTP 429) — backoff exhausted
Halt class: quota_throttled — the API refused (429) and the SDK exhausted its backoff; …
Retry hint: transient — the cause is outside the session; a re-run has a fair chance …
```

Les trois indices (`transient` / `deterministic` / `investigate`) sont des
**annotations, pas des décisions** : rien ne les lit encore pour gater une
relance (hors périmètre du ticket, condition de réveil n≥3 haltes d'une même
famille). Ils sont écrits à la hauteur de ce que les commentaires amont
affirment, source citée par ligne, et pas plus — là où l'amont ne tranche pas,
l'indice est `investigate`.

## La garde sonde son propre armement (F1)

Un test qui lit un fichier d'un **autre dépôt** ne s'arme que là où ce dépôt
est présent. En CI, `claude-pilot/` n'existe pas ; sur le poste de dispatch,
il existe toujours (c'est lui qu'on dispatche). Un `SKIP` noyé dans plusieurs
milliers de lignes de sortie est indiscernable d'un vert pour l'œil qui lit
la dernière ligne — l'architecte l'a posé en bloquant à la première passe.

Trois mécanismes, ensemble :

- La garde émet **exactement un** marqueur sur stdout, en tête de bloc :
  `DRIFT-GUARD: armed against <path> (<n> values)` ou
  `DRIFT-GUARD: SKIP — types.py unreachable (set CLAUDE_PILOT_TYPES)`.
- Le résumé porte une troisième colonne : `Results: N passed, M failed,
  SKIPPED: K`. Un `SKIPPED: 1` sur le poste de dispatch est une halte, pas
  un vert.
- Une assertion compagne (T6-arm) relit le stdout capturé du bloc et exige
  `grep -c '^DRIFT-GUARD: ' == 1`. Rouge-avant constaté : retirer l'`echo`
  du marqueur fait rougir cette assertion. **Un vert nu sans marqueur est
  impossible.**

Le détail bash qui rend ça possible sans perdre les compteurs : le bloc tourne
dans le shell courant avec sa sortie **dupliquée** par substitution de
processus (`_t6_drift_guard > >(tee "$T6_CAPTURE")` puis `wait $!`). Un
`$( )` aurait capturé la sortie *et* jeté les incréments de `PASS`/`FAIL`.

## Ce que le repli ne doit pas casser (R-6)

Quand le JSON n'a pas de `subtype`, la voie de secours gratte la ligne
`[guardrail] <x>: …` du stderr — après le strip ANSI, parce que `ui.py:113`
écrit `\033[38;5;208m[guardrail]\033[0m \033[1m<x>\033[0m: …`. Le nom extrait
alimente **la même table** : une halte n'est jamais classée `unknown` parce
qu'elle est arrivée par le mauvais canal (T4, contrôle négatif compris).

Un subtype **vide** (ni JSON ni `[guardrail]`) est classé `unknown` mais
n'émet **pas** la ligne stderr : il est déjà dit par « cause not recorded »
et n'est pas une dérive amont. Émettre `halt_family.unknown subtype=` avec
une valeur vide aurait pollué le signal exact que la ligne existe pour porter.

## Règle générale

Une liste de valeurs qu'un autre dépôt possède ne se recopie pas en
commentaire. On écrit une **table exécutée avec branche par défaut**, la
branche par défaut **nomme** ce qu'elle reçoit, et un test **lit la source
amont** pour confronter les deux. Si ce test peut ne pas s'armer (dépôt
absent), il le dit par un marqueur unique et compté — jamais par un silence.

## Sonde post-déploiement

Sur le poste de dispatch, après `make deploy` :

```bash
make -C mika test-dispatch-lib 2>&1 | grep -E '^DRIFT-GUARD: |SKIPPED'
```

doit rendre `DRIFT-GUARD: armed against …/claude-pilot/src/claude_pilot/types.py (8 values)`
et `SKIPPED: 0`. Un `SKIP` ici est une halte : la garde n'est pas armée sur
la machine qu'elle protège — poser `CLAUDE_PILOT_TYPES` ou rétablir le
checkout avant de dire « couvert ». Sur la prochaine session `terminated`
réelle, le callback dans mika-dev porte `Halt class:` et `Retry hint:` ;
sinon, `diff` le fichier résolu en prod contre le dépôt avant de conclure
(classe mika#2340 — binaire vert ≠ skill déployé).
