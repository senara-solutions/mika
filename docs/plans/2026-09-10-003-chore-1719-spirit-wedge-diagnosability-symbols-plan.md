---
module: mika-spirit, build, observability
tags: [wedge, deadlock, symbolication, diagnosability, observability, rt003]
problem_type: observability-gap
category: chore
issue: mika#1719
type: chore
date: 2026-09-10
---

# mika#1719 — rendre la prochaine occurrence diagnosticable : symboles au binaire déployé

## 1. Ce que ce plan traite, et pourquoi il est plus étroit que le corps du ticket

Le corps de mika#1719 (2026-07-02) décrit un wedge process-wide de `mika-spirit` :
29 threads sur 30 parkés au même frame, 8h30 de panne silencieuse, cinq
récurrences en trois jours (n=1 → n=5). Il propose cinq remèdes durables.

**Quatre des cinq sont livrés.** La mesure du 2026-09-10 :

| Remède du corps | État mesuré | Preuve |
|---|---|---|
| 2. Auditer les sites de détention de verrou | **livré** | mika#1723 fermé, PR#1726 mergée le 2026-07-06 (`3f87e9a9`) — `handlers.rs` DashMap `Ref` détenu à travers `.insert()` |
| 3. Interdire structurellement la forme | **livré** | mika#1724 fermé le 2026-08-19 — `mika/Cargo.toml:181-182` porte `[workspace.lints.clippy] significant_drop_in_scrutinee = "warn"` |
| 4. Détecter le wedge au lieu de le subir 8h | **livré autrement** | cm#126 — `control-monitor/scripts/host-probes.sh:86-90`, heartbeat inversé sur `localhost:8081/healthz`, cron */10. Vivant, pas seulement déployé : `cm health --agent mika-spirit-engine` → `last_tick_at 2026-09-10T18:20:00Z`, `freshness green`, `expected_period_secs 1200` |
| 5. Corriger la double-écriture des logs | **livré** | mika#2195 / PR#2210 — régime de doublage clos le 2026-09-06 |
| 1. Reconstruire avec des symboles | **NON FAIT** | `mika/Cargo.toml:169` → `strip = true`. `nm ~/.local/bin/mika-spirit` → `no symbols`. `file` → `stripped` |

Le reliquat de mika#1719 est donc **un seul item, mesurable en une commande** : le
binaire déployé ne peut pas être symbolisé. Ce plan ne traite que celui-là.

## 2. Pourquoi ce reliquat vaut encore un p1

Ce n'est pas de l'hygiène. C'est le facteur qui a transformé un bug d'une ligne en
trois jours d'enquête et cinq pannes.

Le diagnostic de juillet a dû procéder par soustraction d'adresses : cinq captures
gdb rendant `?? ()` sur chaque frame, une base PIE recalculée à la main, des
hypothèses successivement falsifiées (`AgentState.skills` — falsifiée ;
double-init tracing — falsifiée ; `flush_failed_sends` — non concluante), et un
tableau d'invariants lui-même révisé à n=4. Le site fautif (`handlers.rs:245-255`)
n'a été nommé qu'au quatrième jour. Un `addr2line` sur un binaire porteur de
tables de lignes l'aurait rendu au premier.

mika#1722 a déjà fermé la moitié amont de cette chaîne : `scripts/mika-spirit-wedge-capture.sh`
capture `/proc/<pid>/maps` au même instant que la pile, donc la base PIE est
désormais un artefact indépendant et non une valeur dérivée dans l'analyse. La
moitié aval — offset → nom de fonction — reste ouverte, et le commentaire d'en-tête
du script le dit lui-même : la corroboration passe aujourd'hui par un `addr2line`
sur *une reconstruction*, ce qui est « nécessairement indirect ».

Formulé en termes de risque de lancement : la classe de bug précise est morte
(#1723 + #1724), mais **la classe d'enquête ne l'est pas**. Tout futur park
process-wide — sur un hôte ou chez un tenant cloud — repart d'un binaire muet.

## 3. Décision de cadrage

**Le périmètre de ce plan est `mika/Cargo.toml` et la chaîne de build.** Rien d'autre.

Trois choses sont délibérément exclues, chacune avec sa raison :

- **Pas de watchdog in-process.** Le corps du ticket (item 4) le demandait, mais un
  watchdog sur le runtime tokio ne peut structurellement pas voir cette classe :
  `spawn_engine_wedge_watchdog` (`liveness.rs:160`) est un `tokio::spawn`, donc il
  attend un worker que le wedge a précisément consommés ; et ses deux canaux de
  signal étaient eux-mêmes parkés lors de n=1 — le thread `tracing-appender` au
  frame partagé, les dix threads `mika-db` de même. Un observateur qui meurt avec
  ce qu'il observe n'est pas un observateur. La détection appartient au hors-process,
  où cm#126 l'a mise.
- **Pas de redémarrage automatique.** Le corps demandait « dump thread stacks +
  restart ». RT#003, postérieur et ratifié par Vincent, refuse l'auto-respawn :
  cité dans `liveness.rs:156-158`, « auto-recovery breaks the bounded-authority
  principle for process-restart operations. Operator decides remediation. » La
  politique prime sur le texte du corps, qui lui est antérieur de deux mois.
- **Pas le déclenchement automatique de la capture.** Le fil manquant — « la sonde
  constate le silence, donc elle lance `mika-spirit-wedge-capture.sh` » — vit dans
  `host-probes.sh`, c'est-à-dire dans le dépôt `control-monitor`, pas dans `mika`.
  Il est réel et il est faisable (Yama absent sur cet hôte : `/proc/sys/kernel/yama/ptrace_scope`
  n'existe pas, donc gdb attache un process de même uid sans root — ce qui bloquait
  en juillet, où seul root-Claude pouvait capturer). Il part en ticket compagnon
  côté `cm`, conformément à « les branches, issues et PR vont sur le dépôt où le
  code vit ». Ce plan livre ce dont ce fil aura besoin côté `mika` : un binaire
  dont la capture soit exploitable.

## 4. Approche retenue

Modifier `[profile.release]` dans `mika/Cargo.toml` pour conserver les tables de
lignes, en gardant `lto = true` et `codegen-units = 1` inchangés :

```toml
[profile.release]
lto = true
codegen-units = 1
strip = "none"            # était: true
debug = "line-tables-only"
```

`line-tables-only` est le palier retenu plutôt que `strip = "debuginfo"` (qui ne
garderait que `.symtab` : des noms, pas de fichier:ligne) et plutôt que `debug = true`
(DWARF complet : plusieurs centaines de Mo). Il donne exactement ce que le
diagnostic de juillet réclamait — `addr2line -f -C -e mika-spirit <offset>` rendant
un nom démanglé **et** un `fichier:ligne` — pour un surcoût de taille borné.

Le profil reste unique et global, sans profil `release-with-debug` séparé, pour une
raison de fond : un binaire diagnosticable ne sert que s'il est *celui qui a wedgé*.
Un profil parallèle produit un binaire qu'on n'exécute pas, et laisse les tenants
cloud — où un wedge serait tout aussi invisible — sur la même impasse qu'en juillet.

**Repli si la taille dépasse le seuil de l'AC2** : `split-debuginfo = "packed"`,
qui laisse le binaire déployé mince et archive un `.dwp` à côté. Le repli n'est
pris que si la mesure le commande, pas par précaution.

## 5. Critères d'acceptation

**AC1 — Une adresse devient un nom et une ligne, sur le binaire réellement déployé.**
Après `make deploy`, `addr2line -f -C -e ~/.local/bin/mika-spirit <offset>` rend
un symbole Rust démanglé et un `fichier:ligne`, pour au moins une frame prise dans
une capture réelle de `scripts/mika-spirit-wedge-capture.sh` sur le process vivant.
La preuve est la sortie de commande, pas l'inspection du `Cargo.toml`.

**AC2 — Le surcoût est mesuré et borné.** Taille du binaire avant/après reportée
dans la PR (référence mesurée : 34 Mo strippé au 2026-09-10). Si l'augmentation
dépasse **3×** (soit >100 Mo), basculer sur le repli `split-debuginfo = "packed"`
et re-mesurer. Le temps de `cargo build --release --features telemetry` est reporté
lui aussi, avant/après.

**AC3 — Les propriétés d'exécution ne bougent pas.** `lto = true` et
`codegen-units = 1` restent en place ; aucune modification du code produit. Vérifié
par revue du diff : ce ticket ne touche à aucun fichier `.rs`.

**AC4 — Contrôle négatif.** Un `addr2line` sur une adresse hors segment de code
échoue lisiblement (`??`), et non silencieusement en rendant un symbole voisin
plausible. Une sonde a besoin de ses deux contrôles dans le même appel : sans
celui-ci, on ne saura pas distinguer « symbolisé » de « symbolisé n'importe comment ».

**AC5 — Le suivant sait quoi faire.** Une entrée dans `docs/solutions/` donne la
procédure complète de symbolisation d'un wedge : capture → base PIE depuis `maps`
→ offset → `addr2line`, avec la sortie réelle de l'AC1 comme exemple travaillé.
C'est ce document, et non ce plan, qui sera lu à 3h du matin à la prochaine
occurrence.

## 6. Étapes

1. Mesurer l'état de départ : taille du binaire, durée du build release, sortie de
   `nm` (attendu : `no symbols`). Consigner — c'est le contrôle avant/après.
2. Modifier `[profile.release]` dans `mika/Cargo.toml` selon §4.
3. `cargo build --release --features telemetry` ; mesurer taille et durée.
4. Si le seuil de l'AC2 est franchi, appliquer le repli `split-debuginfo = "packed"`
   et re-mesurer avant d'aller plus loin.
5. `make deploy`, puis capturer le spirit **vivant** avec
   `scripts/mika-spirit-wedge-capture.sh $(pidof mika-spirit)` — un process sain
   suffit : ce qu'on teste est la symbolisation, pas le wedge.
6. Calculer la base PIE depuis le `maps` de la capture, soustraire, et symboliser
   une frame par `addr2line`. Contrôle positif (AC1) et contrôle négatif (AC4)
   dans la même session, pas dans deux.
7. Écrire l'entrée `docs/solutions/` (AC5) avec la sortie réelle des étapes 5-6.
8. Ouvrir le ticket compagnon côté `control-monitor` : brancher le déclenchement
   automatique de la capture sur le franchissement « healthz muet » de
   `host-probes.sh`, alert-only, une fois par franchissement, en réutilisant la
   discipline `alert`/`clear_alert` déjà présente (`host-probes.sh:50-62`). Y noter
   la question ouverte du choix de PID : en n=5, trois PID orphelins wedgés
   coexistaient avec un child sain, donc `/run/mika-spirit.pid` ne désigne pas
   nécessairement le process à capturer.

## 7. Risques

- **Les images cloud grossissent.** `[profile.release]` est global, donc les images
  Docker de `mika-cloud` héritent du surcoût. Assumé : c'est le prix pour qu'un
  wedge chez un tenant soit diagnosticable. L'AC2 le chiffre ; le repli `packed`
  existe si le chiffre est mauvais.
- **La capture est plus lourde et plus lente.** gdb charge davantage de tables. Sur
  un process déjà wedgé c'est sans conséquence — il ne va nulle part.
- **Faux sentiment de clôture.** Symboliser ne prévient aucun wedge ; cela raccourcit
  l'enquête. La prévention, elle, est portée par #1723 et #1724, et elle est déjà en
  place. Il ne faudra pas lire la fermeture de ce ticket comme « la classe est morte ».

## 8. Hors périmètre

- Tout watchdog in-process (§3, premier point).
- Tout redémarrage automatique (RT#003).
- Le fil sonde → capture, qui vit dans `control-monitor` (ticket compagnon, étape 8).
- Toute modification de code Rust : ce ticket ne touche pas un seul `.rs` (AC3).

## 9. Question de statut à porter à l'opérateur

mika#1719 a été versé dans le jalon `bloqueurs-lancement` le 2026-09-09T16:01Z. La
mesure ci-dessus montre que la panne qu'il décrit est prévenue (#1723, #1724) et
détectée (cm#126, heartbeat vert mesuré ce jour). Ce qui reste — la symbolisation —
est un raccourcisseur d'enquête, pas un empêcheur de panne.

Ce plan ne retire pas le ticket du jalon : c'est une décision opérateur. Il fournit
la mesure sur laquelle Vincent peut re-décider si ce reliquat gate encore le
lancement, ou s'il redescend d'un cran une fois livré.
