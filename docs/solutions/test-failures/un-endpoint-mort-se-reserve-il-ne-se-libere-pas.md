---
title: Un point de terminaison mort se réserve, il ne se libère pas
date: 2026-09-29
category: test-failures
module: mika-common
problem_type: test_failure
component: testing_framework
symptoms:
  - "Un test qui attend une erreur de transport reçoit la réponse du serveur factice d'un test voisin (\"artifact-output\")"
  - "Rouge aléatoire sur le job Check, requis sur main, sans lien avec le diff de la PR"
root_cause: test_isolation
resolution_type: test_fix
severity: medium
tags: [tests, flakiness, tcp, port-race, so-reuseaddr, socket2, dead-endpoint, source-scan, mika-2569, mika-2495]
---

# Un point de terminaison mort se réserve, il ne se libère pas

## Problem

Pour obtenir un « endpoint mort », cinq tests liaient `127.0.0.1:0`, relevaient
le port, **libéraient** l'écouteur, puis appelaient ce port en supposant que
personne ne l'avait repris. Les tests d'un même binaire tournent en parallèle et
montent chacun des serveurs factices sur `127.0.0.1:0` : le port libéré peut être
attribué au voisin entre le `drop` et l'appel.

## Symptoms

Job `Check` de mika#2561 (run 36416967408), 2026-09-28, sur une PR qui ne
touchait pas ce code :

```
thread 'dispatch_surfaces_connection_error_for_dead_endpoint' panicked at
crates/mika-cli/tests/remote_ask_integration.rs:155:10:
should fail when no listener accepts the connection: "artifact-output"
```

`"artifact-output"` est la réponse du serveur factice de
`dispatch_prefers_artifacts_over_status_message`, un test du **même binaire**.
L'échec ne tombe pas sur le coupable et ne se reproduit pas : il se lit comme un
flake, et `Check` étant requis, il habitue à relancer un rouge sans le lire.

## What Didn't Work

Trois remèdes se présentent d'eux-mêmes ; aucun ne ferme la classe.

- **Viser une adresse non routable** (`203.0.113.x`, RFC 5737). Déjà refusé par
  la garde mika#2495, avec sa mesure : le bac à sable de dispatch porte un proxy
  d'egress (mika#2049) qui intercepte une telle adresse et rend un statut HTTP,
  jamais une erreur de transport. Le test serait vert sur le CI et rouge en pilote.
- **Sérialiser le test** (`serial_test`). Ne borne que l'intra-binaire —
  `cargo test` lance les binaires en parallèle, un `bind(:0)` d'un autre binaire
  reçoit aussi le port — et rien ne force un futur test à prendre le mutex. Même
  famille que [`#[serial]` ne protège pas contre les tests parallèles](serial-ne-protege-pas-contre-les-tests-paralleles-2026-09-05.md).
- **Tenir le port avec `std::net::TcpListener`**. Sa `bind` pose `SO_REUSEADDR`
  sur Unix, et deux sockets `SO_REUSEADDR` dont aucun n'est en `LISTEN` peuvent
  lier la même adresse : le garde perd la réservation qu'on lui demande. Vu rouge
  à la livraison : poser `SO_REUSEADDR` sur le garde fait rougir son assertion de
  réservation (`crates/mika-common/src/dead_endpoint.rs`).

## Solution

`mika_common::dead_endpoint::DeadEndpoint` (feature `test-utils`) : un socket
`socket2` **lié, jamais `listen()`, jamais `SO_REUSEADDR`**, tenu en RAII.

```rust
// Avant — le port est rendu au pool éphémère avant l'appel
let addr = {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
};

// Après — le port est tenu pendant tout l'appel
let dead = DeadEndpoint::reserve();
let url = format!("http://{}/a2a/cust-5/mika-prime", dead.addr());
// `dead` doit rester vivant : `let _ = DeadEndpoint::reserve();` relâche aussitôt.
```

`reserve()` **asserte** les deux propriétés avant de rendre, au lieu de les
supposer : un `TcpListener::bind` sur le port exact doit échouer (réservation),
un `TcpStream::connect_timeout` doit échouer (refus). Une plateforme qui se
comporte autrement rougit dans le garde, avec un message qui la nomme.

Les cinq sites sont migrés sans qu'une assertion de test change. Un scan de
source dans `crates/mika-agent/src/canonical_tokens.rs`
(`mika2569_aucune_fixture_nobtient_un_endpoint_mort_en_liberant_un_port`) accuse
tout `let <id> = … TcpListener::bind …` dont l'identifiant n'est ensuite utilisé
que pour `.local_addr` ou `drop(<id>)`. Le message de résolution de la garde
mika#2495, qui prescrivait le motif fautif, nomme désormais le helper.

## Why This Works

Un socket TCP lié mais non écoutant donne deux propriétés d'un coup :

- **le port est réservé** — le noyau l'insère dans le bind bucket, `listen()` ou
  non, et un `bind(:0)` concurrent saute les buckets occupés ;
- **la connexion est refusée** — sans socket en `LISTEN`, le SYN reçoit un RST,
  `connect()` rend `ECONNREFUSED` immédiatement, et `TransportFailure::classify`
  rend `Unreachable`. Aucun délai d'attente, aucun proxy traversé.

La course n'est pas rendue moins probable : elle est **impossible par
construction** tant que le garde vit. C'est pourquoi la preuve est un
`EADDRINUSE` asserté, pas un compte d'exécutions vertes — mille passages verts ne
disent rien d'une course.

## Prevention

Ce qui n'est lisible dans aucun des fichiers pris isolément :

- **Le motif fautif avait été prescrit par une garde.** Le message de
  résolution de mika#2495 disait littéralement « `TcpListener::bind("127.0.0.1:0")`
  puis `drop` ». Les cinq occurrences n'étaient pas de la négligence : c'était
  la prescription servie à quiconque faisait rougir ce scan. **Quand un remède
  s'avère fautif, corriger aussi le texte qui le prescrit** — message d'échec,
  doc-comment, `CLAUDE.md` — sinon la prochaine fixture naît fautive.
- **Un helper `test-utils` a besoin d'une dépendance régulière optionnelle, pas
  d'une dev-dépendance.** Le plan déclarait `socket2` en dev-dep de
  `mika-common` ; c'est inapplicable. Quand `mika-a2a`, `mika-agent` ou
  `mika-cli` consomment `mika-common` avec `features = ["test-utils"]`, elle est
  compilée comme bibliothèque ordinaire et ses dev-deps n'existent pas. D'où
  `socket2 = { workspace = true, optional = true }` et
  `test-utils = ["dep:socket2"]` dans `crates/mika-common/Cargo.toml`.
- **Un recensement à la main en rate.** Le plan recensait cinq sites ; le scan
  en a trouvé un sixième, `free_port` dans `crates/mika-agent/tests/smoke.rs`.
  Il a l'intention **inverse** — un port qu'un `mika-spirit` lancé à côté devra
  lier — donc tenir le port lui est interdit. Il est déclaré dans
  `DEAD_LISTENER_CENSUS` (comparé dans les deux sens), plutôt que de rétrécir le
  prédicat jusqu'à ne plus le voir. Sur un site **neuf**, on migre ; on n'ajoute
  pas d'entrée.
- **Un scan de source doit être vu rouge sur son propre site fondateur.** Deux
  termes du prédicat n'étaient pas dans le plan et viennent de là : ignorer les
  occurrences dans un littéral de chaîne (sans quoi
  `.expect_err("should fail when no listener accepts …")` comptait comme un
  usage et le site d'origine passait vert), et borner la fenêtre d'un site à la
  prochaine liaison du **même** identifiant (sans quoi, dans
  `transport_failures.rs`, le `listener.accept()` d'un site légitime suivant
  blanchissait le site fautif).
- **Angle mort écrit :** une liaison où `let <id> =` et `TcpListener::bind`
  sont sur deux lignes n'est pas vue.

## Related Issues

- mika#2569 (ce correctif, PR #2582) ; mika#2561 (le rouge mesuré) ;
  mika#2495 (la garde des plages de documentation, dont le texte est corrigé) ;
  mika#2049 (le proxy d'egress du bac à sable).
- [`#[serial]` ne protège pas contre les tests parallèles](serial-ne-protege-pas-contre-les-tests-paralleles-2026-09-05.md)
  — même famille : une ressource globale au processus partagée entre tests
  parallèles.
- Plan : `docs/plans/2026-09-29-001-fix-2569-endpoint-mort-sans-course-de-port-plan.md`.
