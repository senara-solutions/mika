---
issue: 2142
type: fix
title: "Exclure sqlite-vec du dependabot cargo — l'upstream 0.1.10-alpha.4 est incomplet (build cassé)"
branch: fix/2142/dependabot-ignore-sqlite-vec
status: groomed
---

> **Grooming (orchestrateur, 2026-09-08 nuit).** p1 propre hors-core (feed réserve). Plan par la main.

# Plan — #2142 : dependabot n'introduit plus le sqlite-vec cassé

## Cause (mesurée, ticket)

`sqlite-vec 0.1.10-alpha.4` publié est **incomplet** : son `sqlite-vec.c` inclut `sqlite-vec-diskann.c`, non livré dans le crate → `fatal error: sqlite-vec-diskann.c: No such file or directory` → `Check` + `Docker Build (Dockerfile.agent)` échouent (mesuré PR #2114, #2224). Dependabot le remonte via le groupe `cargo-minor-patch` (`.github/dependabot.yml`, ecosystem cargo L18-25).

## Fix

Dans `.github/dependabot.yml`, ecosystem `cargo`, ajouter un `ignore` pour `sqlite-vec` couvrant la (les) version(s) alpha cassée(s) — au minimum `0.1.10-alpha.4`, idéalement un `version-update:semver-patch`/pré-release sur `sqlite-vec` tant que l'upstream ne re-livre pas le fichier. Objectif : dependabot n'ouvre plus de PR bumpant sqlite-vec vers une alpha incomplète (qui rouge la CID de toute PR groupée, comme #2224).

## Acceptance
- **AC1** — `.github/dependabot.yml` ecosystem cargo porte un `ignore` sur `sqlite-vec` (version alpha cassée / pré-releases).
- **AC2** — Une prochaine passe dependabot cargo ne propose plus sqlite-vec 0.1.10-alpha.4 (vérifiable : la PR groupée cargo-minor-patch ne le contient plus).
- **AC3** — Note : la version pinnée effective dans `Cargo.toml`/`Cargo.lock` reste celle qui build (hors périmètre : ce ticket empêche la RE-introduction par dependabot, il ne change pas le pin actuel).

## Hors périmètre
- Le pin sqlite-vec dans Cargo.toml (si déjà bon, ne pas toucher).
- La cause upstream (crate incomplet) — non-nôtre ; à revoir quand sqlite-vec re-livre.

## Surface
- `.github/dependabot.yml` (ecosystem cargo, groupe cargo-minor-patch / ignore). **Hors zone décision-core merge-gate** — mais `.github/` n'est pas mécanique (perimeter) → PR human-gated au merge (tap). Aucun risque de collision (#2238/#2248 sont dans crates/) ni de classifier (pas de code merge/token).
