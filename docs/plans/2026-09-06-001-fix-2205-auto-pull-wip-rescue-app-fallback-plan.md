---
issue: 2205
type: fix
title: "auto_pull + wip_rescue s'authentifient via resolve_github_token (PAT-first, App-fallback)"
class: mika#2013
status: groomed
---

# Plan — mika#2205 : auto_pull + wip_rescue via resolve_github_token

## Contexte (mesuré)

Deux scans périodiques du `TaskDispatcher` gardent sur `self.github_token`
(`crates/mika-agent/src/task_engine/dispatcher.rs`) :

- `dispatch_auto_pull_groomed` — garde à **:1049** (`match self.github_token.as_deref()`), skip debug si None.
- `dispatch_wip_rescue` — garde à **:1104**, même forme.

`self.github_token` vient de `agent_github_token()` (`crates/mika-common/src/config.rs:1414` =
PAT seul, aucun fallback App). Le dispatcher détient pourtant déjà **tout le nécessaire** :
`settings: Settings` (struct :187) et `github_app: Option<Arc<GitHubApp>>` (:175), et le
convertisseur canonique `Settings::resolve_github_token(github_app)` (config.rs:**1432**,
PAT-first puis App-fallback) existe.

Précédent identique : mika#2013 a corrigé exactement cette forme pour le cycle mika-manager
(`crates/mika-agent/src/milestone_manager/spawn.rs:164` :
`settings.resolve_github_token(github_app).await` ; `SettingsTokenResolver` :692-710). Ce plan
applique le même correctif aux deux scans du dispatcher.

Preuve du défaut (2026-09-05) : PAT retiré de l'env à ~16:20 → les deux scans meurent à la même
seconde (`auto_pull: running groomed ticket selection` dernier 16:20:00.320868Z ;
`wip_rescue: running auto-resume scan` dernier 16:20:00.320882Z), zéro depuis — alors que le chemin
App était sain (`manager_token_refreshed` jusqu'à 23:17Z, zéro `gh_app_token_exchange_failed`).

## Approche (HOW)

Remplacer, dans les deux fonctions, la garde PAT-seule par une résolution via
`resolve_github_token`, en réutilisant `self.settings` et `self.github_app` déjà présents. Aucun
nouveau champ, aucune nouvelle dépendance, aucun changement de signature de la struct.

### Changement 1 — `dispatch_auto_pull_groomed` (dispatcher.rs:1048-1055)

Avant :
```rust
let github_token = match self.github_token.as_deref() {
    Some(t) => t,
    None => {
        debug!(task_id = %task.id, "auto_pull: no github_token configured, skipping");
        return Ok(());
    }
};
```
Après :
```rust
let resolved = self
    .settings
    .resolve_github_token(self.github_app.as_deref())
    .await;
let github_token = match resolved.as_deref() {
    Some(t) => t,
    None => {
        // Ni PAT ni App résolus — fail-safe, mais WARN (visible) : le scan
        // ne traite rien tant qu'aucun token n'est disponible.
        warn!(
            task_id = %task.id,
            event = "auto_pull_no_token",
            "auto_pull inactif : aucun github_token résolu (PAT absent ET App indisponible) ; \
             aucune sélection de ticket groomé ne s'exécute"
        );
        return Ok(());
    }
};
```
Le reste de la fonction est inchangé (`github_token` reste un `&str` passé à
`auto_pull::auto_pull_groomed_ticket`).

### Changement 2 — `dispatch_wip_rescue` (dispatcher.rs:1103-1110)

Transformation identique, message `event = "wip_rescue_no_token"` (voir coordination mika#2203
ci-dessous pour ne pas dupliquer le WARN).

### Coordination avec mika#2203 (AC3)

mika#2203 monte déjà le skip wip_rescue de debug→WARN. Deux issues sont en vol sur le même skip.
Résolution retenue : **ce ticket (mika#2205) est le sur-ensemble** — il change le mécanisme
(PAT-seul → resolve) ET pose le WARN au même endroit, pour auto_pull ET wip_rescue. Donc :
- Si mika#2205 est mergé avant mika#2203 : mika#2203 devient un no-op (le WARN wip_rescue existe
  déjà) → le fermer en le référençant, ou réduire sa portée à un test de non-régression du niveau
  de log.
- Si mika#2203 mergé d'abord : mika#2205 réutilise/étend son WARN sans le dupliquer.
L'implémenteur vérifie l'état de mika#2203 au moment du merge et ajuste (une ligne dans la PR).

### Identité (AC4, ADR-008)

Les opérations de ces deux chemins n'exigent PAS d'identité PAT machine distincte :
- `auto_pull` : toggle du label `ready` (`gh issue edit --add-label`), lectures `gh`.
- `wip_rescue` : rebase, push sur une branche de brouillon, `gh pr ready` (undraft), commentaire PR.
Aucune n'est une auto-approbation de PR (le cas qu'ADR-008 protège : `mika-qa` approuvant une PR
`mika-dev`). L'identité bot de l'App est donc acceptable en fallback. Les chemins qui EXIGENT
l'identité machine (PR review/merge via des PAT par agent) restent hors périmètre et ne sont pas
touchés par ce plan. À documenter en commentaire de code au-dessus de chaque appel `resolve_github_token`.

## Acceptance criteria

- **AC1** — `dispatch_auto_pull_groomed` acquiert son token via
  `self.settings.resolve_github_token(self.github_app.as_deref()).await` au lieu de lire
  `self.github_token`. Un App sain suffit quand le PAT est absent.
- **AC2** — `dispatch_wip_rescue` : idem.
- **AC3** — Fail-safe préservé : ni PAT ni App résolus → skip (comme aujourd'hui) mais au niveau
  **WARN**, coordonné avec mika#2203 (pas de double WARN).
- **AC4** — Commentaire de code au-dessus de chaque appel documentant qu'ADR-008 n'exige pas
  d'identité PAT distincte pour ces opérations (pas d'auto-approbation) → App acceptable.
- **AC5** — Test de régression prouvant que le chemin App-fallback est emprunté quand le PAT est
  absent. Réutiliser/adapter la forme du test manager mika#2013
  (`test_...resolve_github_token...`) ; idéalement une sonde partagée couvrant les deux scans.
  Le test existant `test_auto_fire_skips_without_github_token` (dispatcher.rs:5283) doit être mis
  à jour : « sans PAT ET sans App » (pas « sans PAT » seul), sinon il verrouille l'ancien
  comportement.

## Hors périmètre

- Restauration du PAT dans l'env du spirit (geste opérateur/root, fichier 600 sourcé par conf.d,
  pattern gh-cron-token) — c'est le fix immédiat ; ce ticket est le fix durable.
- Tout chemin exigeant l'identité machine PAT (PR review/merge).
- La cause de la disparition du PAT de l'env (couverte par le geste opérateur).

## Vérification

- `cargo build -p mika-agent`
- `cargo test -p mika-agent dispatcher` (dont le test AC5 mis à jour)
- `cargo clippy -p mika-agent`
- Sonde post-déploiement : après un restart SANS PAT mais avec App sain, un
  `auto_pull: running groomed ticket selection` et un `wip_rescue: running auto-resume scan`
  doivent réapparaître dans `$MIKA_SPIRIT_LOG_FILE` au tick suivant (preuve que l'App-fallback
  est emprunté).
