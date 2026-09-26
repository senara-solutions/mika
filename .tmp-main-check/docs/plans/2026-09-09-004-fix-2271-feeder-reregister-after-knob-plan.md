---
issue: 2271
type: fix
---

# fix(mika#2271) — un cancel par knob n'est pas une mort : le feeder se ré-inscrit au retour

## Contexte
`MIKA_DEV_AUTO_PULL=0` **annule** (cancelled) la task récurrente feeder. Au retour (knob retiré + restart), une garde lignée mika#1742 — pensée pour ne pas ressusciter une task volontairement tuée — refuse la ré-inscription, laissant le feeder mort. Mesuré 2026-09-09 : restart 19:20 sans knob, aucun tick auto_feeder, ready manuel obligatoire.

## Fix
Exemption « config-cancel » : au boot avec knob absent/=1, marquer les rows récurrentes `cancelled` du feeder (par `(agent_id, label)`) comme réversibles et les ré-inscrire. Un `failed` récent continue de bloquer (la garde #1742 tient pour la vraie mort).

## Acceptance criteria
- **AC1** : une row feeder `cancelled` par knob est marquée réversible et ré-inscrite au boot knob-on → `recurring_active`. Test inline (db) : `assert_eq!(marked, 1)`.
- **AC2** : un `failed` récent NE ré-inscrit PAS (la garde #1742 tient) — distinction cancel-par-config vs mort. Test inline.
- **AC3** : `cargo build` + `clippy -D warnings` + tests verts.

## Note zone
Décision-core (`task_engine/mod.rs`, `db.rs`) → PR human-gated (Vincent, à QA-pass).
