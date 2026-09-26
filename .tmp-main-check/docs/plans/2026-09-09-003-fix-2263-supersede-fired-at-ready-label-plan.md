---
issue: 2263
type: fix
---

# fix(mika#2263) — la 3e classe de zombie : supersede, fired_at fantôme, re-dispatch d'un ticket tenu

## Contexte
Investigation cascade 2026-09-09 : trois défauts du chemin dispatch/merge de la boucle, distincts du silent-stall (#2249/D1) et de l'identité-merge (#2248). Ensemble ils fabriquent des zombies invisibles.

## Défauts et fixes
1. **Supersede ne tue pas le process.** `tracking_cleanup.rs` annulait la task row superseded mais laissait le process pilote (bwrap) vivant → zombie. Fix : tuer le process (pgid) via `task_engine/process_kill.rs` à la supersession.
2. **`fired_at` fantôme.** Un dispatch vivant restait `pending`/`fired_at=NULL` → invisible au faucheur stuck-pending (qui filtre `fired_at IS NULL`). Fix : stamper `fired_at` au démarrage du process (`webhook_dispatch.rs`/`db.rs`).
3. **`ready_label_handler` ignore `blocked`.** Le handler re-dispatchait un ticket portant `blocked` (il n'excluait que le feeder auto_pull). Fix : skip si `blocked` (miroir de `auto_pull::is_feeder_excluded`) → `VerdictAction::Passthrough`, 0 task.

## Acceptance criteria
- **AC1** : une task row superseded avec un process pilote VIVANT → le process (pgid) est tué + row cancelled. Test : `tests/eval/test_supersede_kills_live_pilot.rs` (rouge-avant/vert-après, invariant « superseded ⇒ pgid mort »).
- **AC2** : un dispatch dont le process démarre a `fired_at` non-NULL (jamais fantôme). Test : `tests/eval/test_dispatch_fired_at_stamped.rs`.
- **AC3** : un event `ready` sur un ticket portant `blocked` ne crée AUCUNE task (Passthrough). Test : `tests/eval/test_ready_label_blocked_skip.rs`.
- **AC4** : `cargo build` + `clippy --all-targets -D warnings` + les 3 tests VERTS ; sortie rouge-avant/vert-après collée au corps de PR (porte #2264).

## Note zone
Décision-core (`task_engine`, `server/ready_label_handler.rs`) → PR human-gated (samidarko/Vincent).
