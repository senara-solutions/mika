---
issue: 2248
type: fix
title: "Le merge autonome fire sous l'identité reviewer — séparer signal (handler) et acteur (mika-dev)"
branch: fix/2248/merge-under-dispatcher-identity-not-reviewer
status: groomed
---

> **Grooming (orchestrateur, 2026-09-08).** Cause + fix mesurés/recommandés par mika-qa (doctrine ask-agent, code-grounded — voir commentaires #2248). Plan par la main.

# Plan — #2248 : le merge est posé par le dispatcher (mika-dev), jamais par le reviewer

## Cause racine (mesurée, mika-qa code-grounded)

`ci_success_handler` est traité par **mika-qa ET mika-dev** ; les deux atteignent `run_gh_merge`. Le token du merge est résolu **PAT-first** (`config.rs:1469-1476` → PAT de l'agent courant), injecté `GH_TOKEN` (`pr_merge_with_gate.rs:763`), appelé depuis `ci_success_handler.rs:504-506`, câblé à `handlers.rs:1165/1204`. Quand la course est gagnée par mika-qa, le merge tourne sous **son** PAT → `mergedBy=mika-platform-qa` (le reviewer merge sa propre approbation — juge-et-partie, pas de handoff).

## Fix — option (c) « signal, pas acteur » (recommandée par mika-qa)

Le handler `ci_success_handler` **ne merge plus directement**. Il **marque la PR merge-ready** (signal déterministe : label dédié OU métadonnée DB sur le PR/task). **Seul mika-dev** (le dispatcher, propriétaire du cycle dispatch→review→merge) consomme ce signal et appelle `pr_merge_with_gate` **avec SON propre token**.

- **D1** — `ci_success_handler` : sur CI success + verdict pass + perimeter Mechanical, remplacer `run_gh_merge(...)` par l'émission d'un signal merge-ready (label `merge-ready` déclaré dans `.github/labels.yml`, OU flag DB). Ne PAS merger dans le handler.
- **D2** — mika-dev consomme merge-ready (son webhook handler / le tick) → `pr_merge_with_gate` sous le token de mika-dev.
- **D3** — le reviewer (mika-qa) ne doit JAMAIS atteindre le chemin de merge : gater côté qa (le handler qa émet le signal au plus, ne merge pas). Vérifier l'allowlist de skills / le routage webhook qui fait que mika-qa traite ci_success_handler.

**Options rejetées (mika-qa)** : (a) délégation LLM = non-déterministe ; (b) token de mika-dev dans le handler qa = fuite de boundary (un agent lit le token d'un autre).

## Acceptance Criteria

- **AC1** — Le merge autonome d'une PR mécanique montre `mergedBy = mika-dev` (identité App/PAT dev), **jamais** le reviewer.
- **AC2 (test, gate)** — Test asserting `mergedBy != reviewer` : après une fermeture autonome, l'identité du merge ≠ l'auteur de la review. Rouge sur le code actuel (mergedBy=mika-platform-qa sur #2244), vert après.
- **AC3** — `ci_success_handler` n'appelle plus `run_gh_merge` ; il émet un signal merge-ready déterministe. mika-dev est le seul acteur du merge.
- **AC4 (contrôle négatif)** — Une PR décision-core reste human-gated (le signal merge-ready ne fire pas ; perimeter DecisionCore inchangé).

## Hors périmètre / notes
- La détection perimeter Mechanical/DecisionCore (correcte, mika#1829) — inchangée.
- La preuve C2 propre (mergedBy=mika-dev) se refait après ce fix déployé (journal n°2).

## Surface (DÉCISION-CORE — CODEOWNERS @samidarko → PR human-gated par conception)
- `crates/mika-agent/src/server/ci_success_handler.rs` (:504-506, retirer le merge, émettre le signal).
- `crates/mika-agent/src/server/handlers.rs` (:1165/1204, le token).
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` (:763, consommé par mika-dev).
- Le consommateur merge-ready côté mika-dev (webhook/tick).
- `.github/labels.yml` si label merge-ready.
- Test AC2 (mergedBy != reviewer).
