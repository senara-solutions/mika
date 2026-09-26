---
issue: 2040
type: fix
title: "claude-pilot-py écrit le transcript pilote (1 JSONL/appel LLM) dans ANTHROPIC_LOG_FILE — le maillon lecteur de mika#1705"
branch: fix/2040/pilot-transcript-capture-reader
repos: [claude-pilot-py (primaire), mika (garde/ingestion)]
---

# Plan — #2040 : construire le côté écriture du transcript pilote

## Cause racine (confirmée)

Le hook de transcript pilote (mika#1705) a livré son **producteur** et son **ingestion**, jamais son **écrivain** :
- **Producteur (existe)** : `skills/bundled/_shared/dispatch-lib.sh:2410-2416` — l'executor mika-spirit injecte `ANTHROPIC_LOG_FILE=~/.mika/data/pilot-transcripts/<task-id>.jsonl` ; le dossier est monté en écriture dans le bwrap (:1054/:1137) ; la var passe dans l'allowlist de passthrough.
- **Ingestion (existe)** : `mika/crates/mika-agent/src/async_db.rs:3243` `insert_pilot_transcripts_batch`, `:3253` `prune_old_pilot_transcripts`, `:3259` `count_pilot_transcripts_for_task` — le moteur mika ingère les fichiers finis dans la table `pilot_transcripts`.
- **Écrivain (MANQUE)** : le commentaire dispatch-lib:2415 promet « claude-pilot-py appends one JSONL line per LLM call » — **ce code n'existe pas** : zéro occurrence de `ANTHROPIC_LOG_FILE` dans `claude-pilot-py/` (source + outil installé), et aucune capture LLM alternative (transcript/jsonl/callback). Dossier `pilot-transcripts/` vide.

**Décision de scope (tranchée) : OPTION (a) — construire l'écrivain**, PAS l'option (b) « retirer l'injection ». Justification : le producteur ET l'ingestion sont déjà bâtis ; retirer gâcherait cette infra et laisserait Vincent sans l'artefact de post-mortem que Prime classe **prérequis-racine** (bearing 2026-09-07 : « rien avant lui »). L'artefact est voulu, pas superflu.

## Fix (primaire : claude-pilot-py)

claude-pilot-py wrappe `claude-agent-sdk`. Le SDK expose le flux de messages / événements d'appel LLM. **Implémenter : si `ANTHROPIC_LOG_FILE` est défini dans l'env, claude-pilot-py y append une ligne JSONL par appel LLM** (via le hook/callback de message du SDK), au format que l'ingestion mika attend (`insert_pilot_transcripts_batch` — vérifier le schéma de ligne attendu).

**Mécanisme COMMITTÉ (F1 — pas « à mesurer ») :** le pilote headless (`claude-pilot/src/claude_pilot/agent.py`) consomme **déjà** le flux de messages SDK — il construit un `ClaudeSDKClient` (`claude_agent_sdk`) et itère `AssistantMessage`/`ResultMessage` dans sa boucle de stream (le même objet que `can_use_tool` relaie ; cf. le point de consommation existant, jumeau de `shell.py:159` `async for message in client.receive_response()`). **Le hook est ce point de consommation existant** : à chaque message SDK, écrire une ligne JSONL dans `$ANTHROPIC_LOG_FILE`. Pas de monkey-patch, pas d'interception de bas niveau, pas de wrapper de `messages.create` — on réutilise la boucle que le pilote parcourt déjà. Granularité : une ligne JSONL par message SDK (AssistantMessage = blocs de contenu ; ResultMessage = usage/terminal_reason), ce qui couvre tous les appels puisque le pilote les voit tous là.

**Spike borné (impl, pas décision) :** confirmer la sérialisation exacte de chaque message SDK (les objets `AssistantMessage`/`ResultMessage` portent le contenu + l'usage) et l'accorder au schéma attendu par `insert_pilot_transcripts_batch`. Ce n'est pas un choix de conception ouvert (le hook est fixé) mais une vérification de forme. Note : que claude-code honore ou non `ANTHROPIC_LOG_FILE` nativement est **moot** — c'est claude-pilot qui écrit, à ce point-là.

## Garde anti-reconduction (détecteur — ticket point 2)

Un producteur qui pose une variable dont aucun consommateur ne dépend doit **échouer visiblement**, pas silencieusement. **Test de bout en bout** : une session pilote courte → le fichier `<task-id>.jsonl` attendu existe → assertion **anti-vacuité** sur son contenu (≥1 ligne JSONL valide). Placé dans claude-pilot-py (test d'intégration) ; optionnellement une garde côté mika (le tick d'ingestion émet un WARN si un dispatch fini n'a produit aucun transcript — silent-failure → signal).

## Ingestion aval (ticket point 3)

Vérifier que `pilot_transcripts` ingère effectivement une fois l'écrivain en place (le chemin `insert_pilot_transcripts_batch` est appelé par le tick sur les fichiers finis). Confirmer qu'un lecteur aval (le cas échéant) reçoit enfin des données.

## Acceptance Criteria

- **AC1** — Quand `ANTHROPIC_LOG_FILE` est défini, claude-pilot-py écrit ≥1 ligne JSONL par appel LLM dans ce fichier. Test unitaire/intégration claude-pilot-py.
- **AC2 (format + version — F4)** — Le JSONL écrit correspond au schéma que `insert_pilot_transcripts_batch` (mika) ingère, ET porte un champ **`schema_version: "v1"`** obligatoire. mika **rejette explicitement** une version inconnue (message d'erreur clair nommant la version reçue), plutôt que d'ingérer silencieusement des données non conformes. Le schéma est ainsi versionné à une seule source de vérité.
- **AC3 (détecteur écrivain — Fire-Disposition)** — Test e2e : session courte → fichier attendu → anti-vacuité (≥1 ligne). Gate CI claude-pilot. Rouge aujourd'hui (dossier vide), vert après.
- **AC4 (ingestion)** — Après une session, `count_pilot_transcripts_for_task` > 0 pour la tâche.
- **AC5** — Tests claude-pilot + `cargo build`/test mika verts.
- **AC6 (intégration cross-repo — F2)** — Le changement claude-pilot est effectivement pris par mika : l'install éditable (`uv tool install --editable ./claude-pilot`, via `make deploy`) reflète le code, ET une vérification e2e post-deploy montre `count_pilot_transcripts_for_task > 0` sur un **vrai dispatch** (pas seulement un test unitaire). Le ticket mika ne se ferme pas sur le seul merge claude-pilot : il se ferme quand un dispatch réel produit un transcript ingéré.
- **AC7 (détecteur ingestion — F3)** — Un WARN nommé (`pilot_transcript_empty_after_dispatch`) est émis côté mika quand un dispatch fini avait `ANTHROPIC_LOG_FILE` défini mais 0 ligne ingérée. Test.

## Fire-Disposition (détecteurs des DEUX côtés de la frontière — F3)

Le défaut est **silencieux** (var posée, dossier monté, zéro fichier = indistinguable de « pas de session »). Le silence exige un détecteur de chaque côté :

1. **Côté écrivain (claude-pilot)** — détecteur AC3 : test e2e anti-vacuité, **gate CI bloquant** : session courte → fichier `<task-id>.jsonl` attendu → ≥1 ligne JSONL valide. Rouge aujourd'hui (aucun fichier), vert après, garde permanente.
2. **Côté ingestion (mika) — F3, AC7** : quand un dispatch se termine avec `ANTHROPIC_LOG_FILE` défini pour son `task_id` mais que l'ingestion (`insert_pilot_transcripts_batch`) insère **0 ligne**, émettre un **WARN nommé** (`pilot_transcript_empty_after_dispatch`) — un fichier introuvable / droits / parse-error côté ingestion est invisible pour claude-pilot ; ce WARN le rend visible. Disposition : halt-and-surface (WARN + `audit_events`), rouge tant que l'écrivain manque, silencieux une fois les transcripts ingérés. C'est le détecteur qui aurait fait fire le défaut d'origine dès le premier dispatch au lieu de le laisser 25+ jours.

## Surface / cross-repo

- **claude-pilot-py (primaire)** : l'écrivain JSONL + le test e2e. Branche `fix/2040/pilot-transcript-capture-reader` (même nom).
- **mika (secondaire)** : ingestion déjà présente (vérifier) ; optionnellement le WARN silent-failure côté tick. Le plan vit ici (ticket mika).
- Companion PR entre les deux si les deux repos changent.

## Hors périmètre

- L'horodatage de pilot-egress-proxy.log (mika#2030) et `mika ask` perd-réponses (mika#2036) — instruments muets voisins, tickets séparés.
- Le diagnostic #2029 (décrochage pilote) qui a besoin de cet artefact — débloqué par ce fix, pas traité ici.

## Risques

- **Point de hook SDK introuvable/instable** : si le SDK n'expose pas proprement chaque appel LLM, le format/complétude du transcript en souffre. Mitigé : mesurer le hook disponible à l'impl avant de figer le format (AC2 le contraint contre l'ingestion).
- **Cross-repo** : le producteur (mika) et l'écrivain (claude-pilot-py) doivent s'accorder sur le chemin + format ; l'ingestion mika est la source de vérité du format (AC2).
