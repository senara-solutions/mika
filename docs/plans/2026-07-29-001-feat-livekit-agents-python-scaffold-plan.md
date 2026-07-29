# Plan — LiveKit Agents SDK (Python) scaffold : `Assistant(Agent)` avec silero VAD + cloud STT + Mika brain + ElevenLabs TTS

**Ticket:** mika issue#1787 (sub-issue of #1785 — Mika Voice milestone, Phase 1 conversation MVP)
**Type:** feat (voice, p1.2)
**Priority:** p2-normal
**Repo:** senara-solutions/mika

---

## Context

Phase 1 de la milestone Voice (#1785) livre la **conversation lane** (cloud STT/TTS OK,
rien de sensible ne transite ; la testimony lane sovereign est Phase 2). Ce ticket
scaffolde l'agent LiveKit Python : un process séparé, co-localisé avec le gateway,
qui monte le pipeline `VAD (local silero) → STT (cloud) → Mika brain → TTS (ElevenLabs)`.

Décisions déjà tranchées en amont (milestone #1785 — **ne pas relitiguer**) :

- **Transport :** LiveKit Cloud (Q1 ratifié). Credentials livrés par P1.1 (blocker).
- **Placement :** module dans `crates/mika-gateway/` (Q4 ratifié) — pas un nouveau service.
  Concrètement : un sous-répertoire Python `crates/mika-gateway/voice/` sous gestion `uv`,
  démarré par `supervise-daemon` en process séparé du binaire Rust.
- **Mika brain reste Mika :** le LLM adapter appelle le brain Mika via son endpoint A2A
  local. **Jamais** de swap vers un LLM vendeur (OpenAI/Anthropic direct depuis le plugin
  LiveKit) — Mika reste Mika, c'est un invariant de la milestone.
- **VAD local :** `silero.VAD.load()` — le trafic VAD ne quitte jamais la box (souveraineté).
- **TTS voice_id :** bloqué sur Q2 Prime bearing (cloner Vincent vs voix Mika propre). Ce
  scaffold utilise une voix **placeholder ElevenLabs** ; le `voice_id` sera swappé après
  ratification Prime. Le placeholder est lu depuis une env var — swap = édition config,
  zéro changement de code.

### Contraintes d'intégration découvertes

- **Brain adapter → A2A.** `mika-agent` expose `POST /a2a/{agent_name}` (JSON-RPC), avec
  deux méthodes pertinentes : `message/send` (synchrone, retourne un `Task`) et
  `message/stream` (SSE, `TaskStatusUpdateEvent`/`TaskArtifactUpdateEvent`). L'interface
  `livekit.agents.llm.LLM` attend un flux de tokens incrémental → **`message/stream` est le
  chemin cible** pour minimiser le TTFT. `message/send` sert de fallback non-streaming.
  Le gateway proxie aussi A2A (`/a2a/{customer_id}/{agent_name}`) ; le scaffold cible
  l'endpoint agent **local** (loopback, pas de hop réseau) via env var.
- **Latency doctrine (P1.3, hors scope ici mais à ne pas bloquer).** Le brief impose
  « ack rapide en foreground + réponse profonde en background ». Ce scaffold n'implémente
  **pas** le bridge-line masking (c'est P1.3) mais structure l'adapter pour qu'il puisse
  émettre des tokens dès le premier chunk SSE (pas d'attente de complétion) — condition
  nécessaire pour que P1.3 se greffe sans refonte.
- **Déploiement gated.** Aucun prod deploy sans les mains de Vincent (Prime line). Ce
  ticket = scaffold + AC harness local. Le fichier `supervise-daemon` / service unit est
  livré **prêt mais non activé** ; l'activation runtime est une étape opérateur manuelle.

---

## Requirements

### R1 — Structure du module Python

- Nouveau répertoire `crates/mika-gateway/voice/` géré par `uv` :
  - `pyproject.toml` — deps : `livekit-agents`, `livekit-plugins-openai`,
    `livekit-plugins-elevenlabs`, `livekit-plugins-silero`, `httpx` (pour l'adapter A2A),
    `pydantic` (config typée). Python `>=3.11`.
  - `uv.lock` committé (reproductibilité).
  - `README.md` — comment lancer localement, quelles env vars, comment tourner l'AC harness.
  - `.python-version` optionnel.
- Layout package :
  ```
  crates/mika-gateway/voice/
  ├── pyproject.toml
  ├── uv.lock
  ├── README.md
  ├── src/mika_voice/
  │   ├── __init__.py
  │   ├── config.py          # pydantic settings, MIKA_VOICE_* env vars
  │   ├── agent.py           # Assistant(Agent) + entrypoint(ctx)
  │   ├── brain.py           # MikaBrainLLM(llm.LLM) — A2A adapter
  │   ├── metrics.py         # TTFT/TTFB/EOU hooks (AC4)
  │   └── worker.py          # WorkerOptions + cli.run_app entrypoint
  └── tests/
      ├── test_config.py
      ├── test_brain_adapter.py    # A2A adapter unit tests (httpx mock)
      └── test_ac_harness.py       # round-trip + barge-in (AC5, mockable)
  ```

### R2 — `Assistant(Agent)` + entrypoint (AC1)

- `agent.py` : classe `Assistant(Agent)` construite avec le shape du brief :
  - `instructions` : Mika system prompt (placeholder concis pointant vers le brain — le
    vrai system prompt vit côté Mika brain, pas dupliqué ici ; l'instruction locale
    cadre juste le rôle STT/TTS).
  - `stt = openai.STT()` (cloud, Phase 1 OK).
  - `llm = MikaBrainLLM(...)` (adapter A2A — R3).
  - `tts = elevenlabs.TTS(voice_id=<from config>)` (voix placeholder).
  - `vad = silero.VAD.load()` (local).
- `entrypoint(ctx: JobContext)` : `await ctx.connect()`, crée `AgentSession`, `await
  session.start(room=ctx.room, agent=Assistant())`.
- `worker.py` : `WorkerOptions(entrypoint_fnc=entrypoint)` + `cli.run_app(...)` pour lancer
  le worker qui s'attache aux rooms LiveKit Cloud.

### R3 — Mika brain LLM adapter (le cœur — AC5 plug-and-play)

- `brain.py` : `MikaBrainLLM` implémente l'interface `livekit.agents.llm.LLM` (sous-classe
  `llm.LLM`, retourne un `LLMStream` sur `chat()`).
- Transport : `httpx.AsyncClient` vers `POST {MIKA_VOICE_BRAIN_URL}/a2a/{agent_name}` avec
  JSON-RPC `message/stream` (SSE). Parse les `TaskStatusUpdateEvent`/artifact chunks et les
  yield comme deltas de contenu vers LiveKit dès réception (streaming incrémental — pas
  d'attente de complétion).
- Fallback : si `message/stream` échoue/indisponible, bascule sur `message/send` (synchrone)
  et yield la réponse complète en un chunk. Log warn sur le fallback.
- Auth : `Authorization: Bearer {MIKA_VOICE_BRAIN_TOKEN}` (l'`internal_token` de l'agent).
- Mapping conversation : le `ChatContext` LiveKit (historique multi-tours) est sérialisé
  en `MessageSendParams` A2A. Session continuity via un `context_id`/`task_id` A2A stable
  par room.
- **Invariant testable :** le module ne référence **aucun** LLM provider vendeur pour la
  génération (seul le brain A2A). Un test asserte l'absence d'import `openai`/`anthropic`
  dans `brain.py` côté génération (STT `openai` reste autorisé, c'est un étage distinct).

### R4 — Métriques TTFT/TTFB/EOU (AC4, préparation P1.4)

- `metrics.py` : hooks légers exposant les trois latences clés, en s'abonnant aux events
  du pipeline LiveKit (`AgentSession` / `metrics.MetricsCollectedEvent` du SDK) :
  - **TTFT** (time-to-first-token) : premier delta reçu du brain adapter.
  - **TTFB** (time-to-first-byte audio) : premier chunk audio TTS émis.
  - **EOU** (end-of-utterance) : détection fin de tour via VAD/turn-detection.
- Émission : structured logging JSON (aligné sur la discipline control-monitor) + un hook
  d'extension pour que P1.4 branche l'export métriques. Ne pas sur-construire — P1.4 possède
  le pipeline métriques complet ; ici on **expose** les trois valeurs de façon consommable.

### R5 — Barge-in (AC3)

- Vérifier/activer l'interruption : le VAD silero détecte la parole utilisateur pendant que
  le TTS joue → `AgentSession` interrompt la synthèse en cours. C'est un comportement natif
  du SDK LiveKit ; le scaffold doit le **configurer explicitement** (allow interruptions) et
  le **couvrir par un test** dans l'AC harness (R6), pas le laisser implicite.

### R6 — AC harness (AC2 + AC3 + AC5)

- `tests/test_ac_harness.py` : test round-trip **mockable sans credentials LiveKit réels**
  (STT/brain/TTS stubés) qui valide la topologie du pipeline : audio in → VAD → STT →
  brain adapter → TTS → audio out, et un cas barge-in (parole pendant TTS → interruption).
- Ce harness tourne en CI sans réseau (mocks). Un mode `--live` (gated env var, skip par
  défaut) documente comment lancer contre LiveKit Cloud réel une fois P1.1 livré — mais
  **non exécuté en CI** (dépend de credentials).

### R7 — Déploiement prêt-mais-non-activé

- Fichier service `supervise-daemon` (ou fragment documenté dans le README) décrivant le
  lancement du worker Python, avec les env vars requises, **livré désactivé**. Activation =
  étape opérateur manuelle (Prime line : aucun prod deploy sans Vincent).
- Documenter dans `crates/mika-gateway/CLAUDE.md` (section courte « Voice module ») le
  placement, les env vars `MIKA_VOICE_*`, et le statut « scaffold, non activé ».

### R8 — Perimeter / CI

- Le nouveau répertoire Python ne doit pas casser les gates Rust existants. Vérifier :
  - Le module Python est ignoré par les jobs `cargo` (hors du workspace Cargo — un
    sous-répertoire non listé dans `crates/mika-gateway/Cargo.toml` ni membre du workspace).
  - Ajouter un job CI léger (lint/test Python via `uv`) **ou** documenter explicitement
    pourquoi il est différé à P1.4/P1.5 si l'ajout CI dépasse le scope scaffold. Décision
    par défaut : ajouter un job minimal `uv run pytest` scoping `crates/mika-gateway/voice/`
    pour que R6 soit exécuté en CI.
  - Confirmer que `.gitignore` couvre `.venv/`, `__pycache__/`, `*.pyc` sous `voice/`.

---

## Env vars (nouvelles, préfixe `MIKA_VOICE_`)

| Var | Rôle | Défaut |
|-----|------|--------|
| `MIKA_VOICE_LIVEKIT_URL` | WebRTC/worker URL LiveKit Cloud (de P1.1) | — (requis runtime) |
| `MIKA_VOICE_LIVEKIT_API_KEY` | LiveKit API key (de P1.1) | — |
| `MIKA_VOICE_LIVEKIT_API_SECRET` | LiveKit API secret (de P1.1) | — |
| `MIKA_VOICE_BRAIN_URL` | Base URL de l'endpoint A2A mika-agent local | `http://localhost:8080` |
| `MIKA_VOICE_BRAIN_AGENT` | `agent_name` A2A cible | `mika` |
| `MIKA_VOICE_BRAIN_TOKEN` | Bearer token (internal_token agent) | — |
| `MIKA_VOICE_TTS_VOICE_ID` | ElevenLabs voice_id (placeholder jusqu'à Q2 Prime) | placeholder public |
| `MIKA_VOICE_STT_PROVIDER` | Sélecteur STT (`openai` Phase 1) | `openai` |
| `OPENAI_API_KEY` | STT cloud | — |
| `ELEVEN_API_KEY` | TTS cloud | — |

---

## Approach / Steps

1. **Scaffold `uv` project** (R1) — `pyproject.toml`, layout package, deps, `uv.lock`,
   README, `.gitignore` entries. Vérifier `uv sync` + `uv run python -c "import
   livekit.agents"` localement (ou documenter si les deps ne sont pas résolvables hors
   réseau — le scaffold committé n'exige pas d'exécution réseau en CI).
2. **`config.py`** (R1) — pydantic `Settings` lisant les `MIKA_VOICE_*` + tests
   `test_config.py`.
3. **`brain.py`** (R3) — `MikaBrainLLM` adapter A2A streaming + fallback + tests
   `test_brain_adapter.py` (httpx mock, assertions sur le shape JSON-RPC + invariant
   no-vendor-LLM).
4. **`agent.py` + `worker.py`** (R2) — `Assistant(Agent)` avec les cinq étages, `entrypoint`,
   `WorkerOptions`. Barge-in configuré explicitement (R5).
5. **`metrics.py`** (R4) — hooks TTFT/TTFB/EOU + structured logging.
6. **AC harness** (R6) — `test_ac_harness.py` round-trip + barge-in, mockable, CI-safe ;
   mode `--live` documenté/skip.
7. **Déploiement prêt-non-activé** (R7) — service fragment + section CLAUDE.md.
8. **CI + perimeter** (R8) — job `uv run pytest` scopé, confirmer non-régression des gates
   Cargo, `.gitignore`.

---

## Verification Contract

- `uv run pytest crates/mika-gateway/voice/tests/` passe (config + brain adapter + AC harness
  mockés, sans réseau ni credentials).
- `test_brain_adapter.py` prouve : (a) le adapter émet un JSON-RPC `message/stream` bien
  formé vers l'endpoint A2A, (b) les chunks SSE sont yieldés incrémentalement, (c) fallback
  `message/send` sur échec stream, (d) **invariant** — aucune génération LLM ne passe par un
  provider vendeur (seul le brain A2A).
- `test_ac_harness.py` prouve : round-trip pipeline topologie (VAD→STT→brain→TTS) + un cas
  barge-in (interruption TTS sur détection VAD).
- `cargo build -p mika-gateway` et les jobs Cargo existants restent verts (le module Python
  n'est pas dans le workspace Cargo).
- `crates/mika-gateway/CLAUDE.md` documente le module voice (placement, env vars, statut
  non-activé).
- Le service de déploiement est livré **désactivé** — aucune activation runtime automatique.

---

## Non-goals (hors scope, sub-issues dédiées)

- **P1.1** — setup LiveKit Cloud (credentials). Ce ticket **consomme** les credentials via
  env vars mais ne les provisionne pas. Blocker déclaré.
- **P1.3** — bridge-line latency masking (« attends, je vérifie… ») + prompt discipline
  tours courts. Le scaffold structure l'adapter pour un TTFT bas mais n'implémente pas le
  masking.
- **P1.4** — pipeline métriques complet (export control-monitor). Ce ticket **expose** TTFT/
  TTFB/EOU de façon hookable ; P1.4 possède l'export.
- **P1.5** — AC harness voice complet contre LiveKit réel. Ce ticket livre un harness
  **mockable CI-safe** + un mode `--live` documenté mais non exécuté.
- **Phase 2 (testimony lane)** — Whisper local, non-transit invariant build-time, turn
  detection generous, Book coordinator. Aucun code sovereign ici.
- **Swap du `voice_id` définitif** — bloqué Q2 Prime bearing ; placeholder configurable.
- **Prod deploy** — gated main Vincent (Prime line). Service livré désactivé.

---

## Risks / Open questions

- **Résolution des deps LiveKit hors réseau.** Si CI ne peut pas résoudre `livekit-agents`
  et plugins, le job Python doit soit utiliser un cache/mirror, soit être marqué
  `continue-on-error` en attendant P1.5 — décision à confirmer au moment de l'implémentation
  selon l'environnement CI réel. Défaut : tenter le job réel ; documenter le fallback.
- **Shape exact de l'interface `llm.LLM` LiveKit.** L'API du SDK `livekit-agents` évolue
  (Agent/AgentSession sont récents). L'implémenteur doit vérifier la signature exacte de
  `LLM.chat()` / `LLMStream` contre la version pinnée dans `uv.lock` et adapter `brain.py`
  en conséquence — le plan fixe le contrat (streaming A2A → deltas), pas la signature ligne-à-ligne.
- **`context_id`/session continuity A2A.** Le mapping room LiveKit ↔ session Mika doit être
  stable pour préserver l'historique multi-tours. Choix par défaut : un `context_id` dérivé
  du `room.name`. À valider que le brain A2A honore la continuité de contexte.

---

## Definition of Done

- Le module `crates/mika-gateway/voice/` existe, géré par `uv`, avec le layout R1.
- `Assistant(Agent)` + `entrypoint` implémentés avec les cinq étages (silero VAD local,
  cloud STT, Mika brain via A2A, ElevenLabs TTS placeholder).
- Brain adapter A2A streaming (+ fallback) avec invariant no-vendor-LLM testé.
- Barge-in configuré + testé.
- TTFT/TTFB/EOU exposés (hookable).
- AC harness mockable passe en CI ; mode `--live` documenté.
- Service de déploiement livré désactivé + doc CLAUDE.md.
- Jobs Cargo existants non régressés ; job Python CI vert (ou fallback documenté).

## Acceptance criteria

1. **Scaffold Python démarre, s'attache à room LiveKit.** `Assistant(Agent)` + `entrypoint`
   + `WorkerOptions` sont implémentés et le worker peut s'attacher à une room (validé mockable
   en CI ; validé live une fois P1.1 livré via le mode `--live`).
2. **Round-trip minimal :** client envoie audio → VAD detects → STT transcrit → LLM
   (brain adapter, placeholder-mockable d'abord) répond → TTS synthesize → client entend.
   Couvert par l'AC harness (topologie du pipeline prouvée mockée).
3. **Barge-in :** l'utilisateur peut interrompre le TTS mid-speech via VAD. Configuré
   explicitement et couvert par un test dédié.
4. **Métriques hookables :** TTFT/TTFB/EOU exposés (préparation P1.4).
5. **Structure plug-and-play :** l'organisation du code permet de swapper les providers
   STT/LLM/TTS ; le Mika brain reste derrière l'adapter A2A (jamais un LLM vendeur pour la
   génération), invariant testé.
