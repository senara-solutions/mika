---
issue: 2027
type: fix
title: "identity.toml absent → fail-closed (sentinel, comme le malformé), universel ; + distinguer absent/malformé dans les logs"
branch: fix/2027/absent-identity-toml-fail-closed
---

# Plan — #2027 : un identity.toml absent ne doit jamais valoir « tout permis »

## Problème (évidence, 2026-08-28)

`load_identity` (`crates/mika-agent/src/prompt.rs:494`) et `load_identity_async` (:504) :

```rust
match std::fs::read_to_string(&path) {
    Ok(content) => parse_identity_or_fail_closed(&content, home_dir, &path),  // malformé → fail-closed (sentinel)
    Err(_)      => Identity::default(),                                       // ABSENT → permissif (BUG)
}
```

`Identity::default()` porte `skills.allowlist: None`, que `apply_identity_allowlist` traite en **no-op** → **toutes les skills bundled actives** (`shell-exec`, `git-ops`, `github`, `tmux`). **Supprimer l'identity.toml d'un agent l'ÉLARGIT.** Atteignable par un geste de remédiation de tier (delete + restart ; bootstrap ne re-tourne jamais car `is_initialized` rend `true` dès que `data/mika.db` existe). Fenêtre réelle ~2 min sur le tenant champion. Classe mika#1596 dans sa forme la plus nue.

## Décisions tranchées (F1, F2)

**F1 — fail-closed UNIVERSEL (option a).** Tout agent sans identity.toml échoue closed, qu'il soit bien-connu/provisionné OU user-défini. Un agent user-défini qui a besoin de skills DOIT avoir un identity.toml explicite (même minimal). Justification : AC1 (« absent ne donne pas permissif ») ne tolère pas d'exception ; distinguer provisionné/user-défini rouvrirait une classe d'ambiguïté (« quel agent devrait avoir un fichier ? ») que le fail-closed universel ferme nettement.

**F2 — mécanisme = SENTINEL fail-closed (allowlist qui n'assortit aucune skill), PAS « refuser de démarrer ».** L'absent rend le **même sentinel** que le malformé (`parse_identity_or_fail_closed`, prompt.rs:559 « Sentinel allowlist matches no real skill (evicts all bundled) ») → l'agent démarre avec **zéro skill**. 

> **Divergence assumée vs recommandation architecte first-pass (qui suggérait « refuser de démarrer ») — raison :** (1) « refuser de démarrer » dans un spirit multi-agents down le process ENTIER pour un seul agent sans fichier, prenant tous les agents sains avec lui ; (2) le tier_guard mika#1962 **tolère explicitement un fichier persona manquant** (« a missing persona file is not an error — a half-bootstrapped agent legitimately has none ») — refuser de démarrer sur absent contredirait cette tolérance établie ; (3) A1 dit « aligner l'absent sur le malformé », et le malformé utilise le sentinel, pas le refus. Le sentinel est per-agent, fail-closed, cohérent, et ne down pas le spirit. (À valider au 2nd pass.)

## Déliverables

### D1 — Absent → sentinel fail-closed (AC1)
Distinguer `Err(ErrorKind::NotFound)` (absent) des autres erreurs de lecture. Sur l'absent, rendre le **sentinel fail-closed** (le même que le malformé), pas `Identity::default()`. Les deux versions (sync `load_identity` + async `load_identity_async`).

### D2 — Distinguer absent / malformé dans les logs (AC2)
Un `WARN`/`ERROR` **distinct** : « identity.toml absent » (NotFound) vs « présent mais malformé » (parse/validation, déjà loggé par parse_identity_or_fail_closed). Nommer l'agent + le chemin. Deux causes, deux remédiations.

### D3 — Documenter le chemin de re-provision MANUEL (AC4, partiel)
Documenter dans `docs/operator/` la remédiation **manuelle** supportée aujourd'hui (éditer identity.toml depuis le template du tier + restart), en attendant le chemin outillé. **Le chemin CLI de re-provision est extrait en #2230** (F3 : un fix de sécurité ne doit pas embarquer une feature CLI).

## Fire-Disposition (F4)

AC1 est un **détecteur d'invariant de sécurité** (le comportement « élargissant » ne doit pas se reproduire).
- **Disposition** : **gate CI bloquant**. Test unitaire : construire un home sans identity.toml pour un agent → asserter que l'allowlist résultante est le sentinel fail-closed (zéro skill), PAS toutes-skills.
- **Rouge avant fix** (comportement actuel permissif : allowlist None → no-op → toutes skills) → **vert après** (sentinel).
- **Garde permanente** : le test reste en CI ; toute régression vers `Identity::default()` permissif sur absent le re-casse.

## Surface

- `crates/mika-agent/src/prompt.rs` — `load_identity` (:494) / `load_identity_async` (:504) ; réutiliser le sentinel de `parse_identity_or_fail_closed`.
- Docs (D3) : `docs/operator/`.

## Acceptance Criteria

- **AC1 (fail-closed universel — F1/F2)** — Un identity.toml absent (tout agent) → allowlist sentinel (zéro skill), JAMAIS permissive. Test unitaire (Fire-Disposition ci-dessus).
- **AC2 (logs distincts)** — Absent (NotFound) et malformé émettent des events nommés distincts. Test.
- **AC3 (doc re-provision manuelle)** — La remédiation manuelle (édition + restart) est documentée dans docs/operator/. Le chemin outillé = #2230.
- **AC4** — `cargo test -p mika-agent` + `cargo build` verts.

## Hors périmètre

- Chemin CLI de re-provision → **#2230** (extrait).
- Tier champion (mika#2023, mika-cloud#209/#210) ; persona champion (question produit).

## Risques

- **Régression user-défini** (F1 option a) : un agent user-défini sans identity.toml perd ses skills (démarre au sentinel). Assumé et voulu — c'est la posture fail-closed. Le remède est un identity.toml explicite (documenté D3).
- **Sentinel vs refus-démarrage** : divergence assumée vs l'architecte (raisons ci-dessus) ; si le 2nd pass tranche pour le refus, bascule facile (mais contredit #1962).
