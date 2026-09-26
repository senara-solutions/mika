---
issue: 2195
type: fix
module: crates/mika-common/src/logging.rs
tags: [telemetry, logging, double-write, observability, rt005]
problem_type: dual-sink-writes-same-file
status: groomed
---

# Plan — mika#2195 : double-écriture du log spirit (deux layers JSON → même fichier)

## Cause racine, relue dans le code + le service

`mika_common::logging::init` (branche `(Some(path), LogFormat::Json)`) compose **deux** layers
`fmt` JSON :
```rust
tracing_subscriber::registry()
    .with(otel_layer).with(filter)
    .with(fmt::layer().json().flatten_event(true))                       // (A) → STDOUT
    .with(fmt::layer().json().flatten_event(true).with_writer(non_blocking)) // (B) → FICHIER (server.log)
    .init();
```
Layer (A) écrit chaque événement sur **stdout** ; layer (B) l'écrit dans le **fichier**
(`MIKA_SPIRIT_LOG_FILE` = `/var/log/mika/server.log`).

**Et le service redirige stdout vers ce MÊME fichier.** Le supervise-daemon OpenRC lance :
`supervise-daemon mika-spirit --stdout /var/log/mika/server.log --stderr /var/log/mika/server.log`
(vérifié sur /proc). Donc stdout (layer A) **→ OpenRC → server.log**, en plus de l'écriture directe
du layer (B). **Chaque événement atterrit DEUX fois dans server.log.**

## Portée : GÉNÉRALE, pas turn_usage

Le défaut est structurel aux layers, pas propre à `turn_usage`. Mesuré au grooming de mika#2209 :
`mika-spirit listening` (le serveur bind son port UNE fois) apparaît **deux fois** au restart
06:42:48Z ; `domain_rebuild_complete` (un par boot) apparaît **quatre** fois. Tout consommateur du
log qui compte des événements sur-compte ×2 — c'est ce qui a fabriqué le fantôme « double-init » de
mika#2209 et le facteur-2 de RT-005 (−7,43 → −3,71 dédoublonné).

## Approche (HOW) — Option A tranchée : un seul écrivain du fichier

**Décision (tranchée, mika#1244) : Option A, self-contained dans mika.** Quand `log_file` est `Some`,
`init` n'installe **PAS** le layer stdout JSON (A) ; il ne garde que le layer fichier (B). Sous
OpenRC, la redirection `--stdout server.log` ne reçoit alors plus les lignes JSON, et le fichier
n'est écrit qu'une fois, par (B).

**Pourquoi A et pas la correction OpenRC (rejetée) :** le producteur (mika) ne doit pas dépendre de
la configuration du lanceur pour ne pas se dédoubler. Corriger la redirection côté mika-cloud/OpenRC
laisserait le défaut latent — un autre lanceur (systemd, docker, un futur init-script) qui redirige
stdout vers le même fichier le rouvrirait. Le fix appartient au point qui compose les layers.
Couplage cross-repo évité, robustesse au lanceur acquise. Un commentaire au-dessus des layers
documente la contrainte (log_file set ⇒ pas de layer stdout JSON, sinon double-écriture sous toute
redirection stdout→fichier).

**Non-régression dev (AC4) :** en `LogFormat::Pretty` et/ou sans `log_file`, la sortie console reste
inchangée — on ne supprime le layer stdout QUE dans la branche `(Some(path), Json)`, la seule où le
double-écrit se produit.

## Acceptance criteria

- **AC1** — Un événement (`turn_usage` et tout autre) produit **UNE** ligne dans `server.log` sous
  la configuration de prod (log_file set + stdout redirigé vers ce fichier). Cause nommée au
  file:line : les deux layers JSON (logging.rs, branche Json) + la redirection `--stdout` OpenRC.
- **AC2** — Test de non-régression : pour un tour connu, compter les lignes du canal = 1 (pas 2).
  Le test doit être rouge sur le code actuel (contrôle de non-vacuité). Comme un test unitaire ne
  peut pas reproduire la redirection OpenRC, il vérifie l'invariant côté producteur : `init` avec un
  `log_file` n'installe **pas** deux sinks JSON écrivant la même destination (un seul writer fichier,
  et pas de layer stdout JSON dupliquant quand log_file est set).
- **AC3** — Vérifier les autres event types du même canal (le défaut est structurel aux layers) :
  après fix, `mika-spirit listening`, `domain_rebuild_complete`, `turn_usage` apparaissent chacun
  une seule fois par occurrence réelle. Sonde post-déploiement sur état vivant.
- **AC4** — Non-régression dev : en mode `LogFormat::Pretty` / sans `log_file`, la sortie console
  reste présente (on ne casse pas l'observabilité de dev en supprimant stdout).

## Fire-Disposition

- **AC2 — test de non-vacuité (CI).** Tir sur : le diff / la CI. Disposition : **gate CI bloquant**,
  rouge sur le code actuel, vert après fix. Pas de remédiation auto.
- **AC3 — sonde post-déploiement (opérateur).** Tir sur : l'état runtime après restart. Disposition :
  **halt-and-surface** — si un événement single-occurrence (`mika-spirit listening`) réapparaît en
  double, le fix n'a pas pris ; ne pas re-déployer aveuglément.

## Hors périmètre

- Correction rétroactive des valeurs RT-005 : explicitement NON (−7,4 reste la valeur étiquetée du
  batch ; pas d'estimation concurrente — cf. note du ticket).
- Le batch physique RT-005 suivant : déverrouillé par ce fix, mais lancé séparément.

## Vérification

- `cargo test -p mika-common logging` (dont le test AC2).
- `cargo build` + `cargo clippy`.
- Post-déploiement : `grep -c '"message":"mika-spirit listening"' server.log` sur une fenêtre
  post-restart = 1 par restart (pas 2). `grep turn_usage` dédup = compte réel.
