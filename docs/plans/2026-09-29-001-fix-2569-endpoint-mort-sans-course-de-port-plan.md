# mika#2569 — Un point de terminaison mort se **réserve**, il ne se libère pas

**Ticket :** `senara-solutions/mika#2569`
**Type :** fix (substrat de test)
**Labels :** `bug`, `p2-normal`, `dispatch:loop`

---

## 1. Le défaut, mesuré (2026-09-28)

`crates/mika-cli/tests/remote_ask_integration.rs:146`
(`dispatch_surfaces_connection_error_for_dead_endpoint`) lie `127.0.0.1:0`,
relève le port, **libère** l'écouteur, puis appelle ce port en supposant que
personne ne l'a repris. Job `Check` de mika#2561 (tête `28dc8916`, run
36416967408, job 108910307046) :

```
thread 'dispatch_surfaces_connection_error_for_dead_endpoint' panicked at
crates/mika-cli/tests/remote_ask_integration.rs:155:10:
should fail when no listener accepts the connection: "artifact-output"
test result: FAILED. 11 passed; 1 failed
```

`"artifact-output"` est la réponse du serveur factice de
`dispatch_prefers_artifacts_over_status_message` (ligne 267), un test du **même
binaire**. L'appel « vers un port mort » a été servi par un voisin qui avait
reçu le port libéré entre le `drop` et l'appel. Le diff de la PR (bump utoipa +
`gateway.yaml` régénéré) ne touche pas ce code.

`Check` est un check **requis** sur `main` : ce rouge bloque des PR saines et
habitue à relancer un rouge sans le lire.

---

## 2. Ce que la lecture du code établit — quatre rectifications, et c'est le premier livrable

### R1 — Le motif n'est pas à un site, il est à **cinq**

Recensement exhaustif (`grep -rn "TcpListener::bind" crates/ --include="*.rs"`,
puis lecture de chaque site pour séparer « lie et sert » de « lie et libère ») :

| # | fichier | test | forme |
|---|---|---|---|
| 1 | `crates/mika-cli/tests/remote_ask_integration.rs:146` | `dispatch_surfaces_connection_error_for_dead_endpoint` | `let listener = …; drop(listener);` |
| 2 | `crates/mika-cli/tests/remote_ask_recovery.rs:250` | `a_refused_port_errors_without_attempting_a_recovery` | bloc `{ let listener = …; listener.local_addr() }` |
| 3 | `crates/mika-cli/tests/remote_ask_recovery.rs:351` | `a_refused_port_is_retryable_and_exits_75` | idem |
| 4 | `crates/mika-a2a/tests/transport_failures.rs:126` | `a_refused_port_is_unreachable_and_forbids_recovery` | idem, commentaire « Bind then drop » |
| 5 | `crates/mika-agent/src/milestone_manager/cadence.rs:1066` | `probe_executor_health_returns_none_on_unreachable_endpoint` | `drop(listener);` |

Les trois premiers vivent dans des binaires de test qui montent chacun plusieurs
serveurs factices sur `127.0.0.1:0` en parallèle — exactement la course mesurée.
Le quatrième partage son binaire avec `a_silent_server_is_abandoned_on_the_clients_budget`,
qui bind `:0`. Le cinquième vit dans le binaire de tests unitaires de
`mika-agent`, le plus parallèle de l'arbre.

**Conséquence sur le périmètre :** un correctif au site 1 seul livre le même
rouge aléatoire pour la semaine prochaine, sur quatre tests dont les noms ne
diront rien à celui qui le recevra.

### R2 — Le remède que le ticket propose en second est déjà **refusé par écrit**, avec sa mesure

Le ticket propose « viser une adresse non routable avec un court délai ».
`crates/mika-agent/src/canonical_tokens.rs` (garde mika#2495) l'interdit
structurellement et dit pourquoi :

> RÉSOLUTION : remplacer la fixture par un port de boucle locale fermé […]. Un
> proxy intercepte une adresse non routable et rend un 400, jamais une erreur de
> transport — le test est alors vert sur le CI et rouge en pilote.

Le bac à sable de dispatch porte un proxy d'egress (mika#2049) ; une adresse non
routable y est interceptée et rend un statut HTTP, donc `TransportFailure::classify`
rendrait `HttpStatus(400)` et non `Unreachable`. Le test serait vert sur le CI et
rouge en pilote — la panne invisible que mika#2495 a payée 6,95 USD de session QA.
**Cette voie est fermée**, et le plan ne la rouvre pas.

### R3 — Le motif fautif est **prescrit** par cette même garde

Le message de résolution de mika#2495 nomme littéralement
``TcpListener::bind("127.0.0.1:0")`` **puis `drop`** comme le remède recommandé.
C'est ce qui explique les cinq occurrences : le motif n'a pas essaimé par
négligence, il est écrit dans la prescription que le dépôt sert à quiconque fait
rougir ce scan. **Un correctif qui ne touche pas ce texte laisse la prochaine
fixture naître fautive**, et la moitié qui tient n'est de toute façon pas le
texte (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`).

### R4 — « Sérialiser ce seul test » ne ferme pas la classe

Troisième option du ticket. `serial_test` est déjà dev-dep de `mika-cli`, donc
le geste est disponible — et insuffisant sur deux axes. (a) Il ne borne que
l'**intra-binaire** : `cargo test` lance les binaires de test en parallèle, et
un `bind(:0)` d'un autre binaire peut recevoir le port libéré. (b) Rien ne force
un futur test du binaire à prendre le mutex : la garantie est une convention, pas
une propriété. Elle coûte en plus le parallélisme sur un binaire de douze tests.

---

## 3. Le remède — un socket **lié et non écoutant**

Le ticket l'effleure (« lier puis fermer côté socket sans `SO_REUSEADDR` ») ;
l'exact est plus simple : **on ne ferme pas du tout**.

Un socket TCP lié à `127.0.0.1:P` sur lequel `listen()` n'a **jamais** été appelé
donne les deux propriétés d'un coup :

- **Le port est réservé.** `inet_bind` insère le socket dans le bind bucket,
  `listen()` ou non. Un `bind("127.0.0.1:0")` concurrent parcourt la plage
  éphémère via `inet_csk_get_port` et saute les buckets occupés. La collision
  n'est levée que si **tous** les sockets liés portent `SO_REUSEADDR` (et aucun
  n'est en `LISTEN`), ou `SO_REUSEPORT` des deux côtés.
- **La connexion est refusée.** Le socket n'étant pas en état `LISTEN`, un SYN
  entrant ne rencontre aucun socket d'écoute : le noyau répond RST, `connect()`
  rend `ECONNREFUSED` **immédiatement**, `reqwest` rend `is_connect() == true`,
  et `TransportFailure::classify` rend `Unreachable`. Aucun délai à attendre,
  aucune classe à deviner.

**Ne pas poser `SO_REUSEADDR` est porteur, et c'est pourquoi `std::net::TcpListener`
ne convient pas** : sa `bind` pose `SO_REUSEADDR` sur Unix, et deux sockets
`SO_REUSEADDR` dont aucun n'est en `LISTEN` peuvent lier la même adresse exacte.
Le garde perdrait précisément la propriété de réservation qu'on lui demande.
D'où `socket2::Socket::new` — qui ne pose rien — plutôt que la bibliothèque
standard.

`socket2` **est déjà dans `Cargo.lock`** (0.6.5, transitivement via
`tokio`/`hyper-util`) : la dépendance ajoutée est dev-only et n'introduit aucune
caisse nouvelle dans l'arbre.

### Ce raisonnement est **asserté**, jamais supposé

Un raisonnement noyau dans un doc-comment est une hypothèse. Les deux propriétés
sont donc vérifiées **à la réservation**, sur tous les sites d'un coup :

- un `std::net::TcpListener::bind(addr)` sur ce port exact doit rendre `Err`
  (réservation effective) ;
- un `std::net::TcpStream::connect_timeout(addr, 2 s)` doit rendre `Err`
  rapidement (refus effectif).

Les deux passent par du TCP brut, qui ne consulte aucun proxy : elles mesurent la
propriété réseau et non le comportement de `reqwest`. Si une plateforme se
comporte autrement, elle rougit **dans le garde**, avec un message qui la nomme,
au lieu de dériver dans un test distant.

---

## 4. Design

### D1 — `mika_common::dead_endpoint`, un module `test-utils`

Motif calqué sur `mika_common::source_guard` (mika#2398), le précédent exact :
`#[cfg(any(test, feature = "test-utils"))]`, *« test-only by construction: it
enters no production binary »*.

```rust
/// Un point de terminaison de boucle locale dont la connexion est REFUSÉE et
/// dont le port ne peut être attribué à personne tant que ce garde vit.
pub struct DeadEndpoint { _socket: socket2::Socket, addr: SocketAddr }

impl DeadEndpoint {
    /// Réserve le port et vérifie les deux propriétés avant de rendre.
    /// Panique en nommant laquelle a lâché — un fixture muet est pire qu'absent.
    pub fn reserve() -> Self;
    pub fn addr(&self) -> SocketAddr;
}
```

Le garde est RAII : tant que la valeur vit, le port est à nous. Un site qui
oublie de la tenir (`let _ = DeadEndpoint::reserve();`) libère immédiatement —
d'où un nom de liaison explicite sur chaque site et le scan de D3 pour le tenir.

**Pourquoi un helper partagé plutôt que quatre lignes recopiées cinq fois :** ce
qui se perd en recopiant n'est pas le code, c'est le *raisonnement* — pourquoi
pas de `listen`, pourquoi pas de `SO_REUSEADDR`, pourquoi le garde doit vivre.
Le motif fautif d'aujourd'hui est lui-même une recopie à cinq exemplaires d'une
prescription qui portait le raisonnement à un seul endroit (R3).

**Coût nommé :** `mika-a2a` gagne `mika-common` en **dev**-dépendance ; elle
n'avait que `tokio`. Pas de cycle (`mika-common` ne dépend pas de `mika-a2a`) et
pas d'inversion de hiérarchie (la chaîne est `common → a2a → agent → cli`). Le
coût réel est un `cargo test -p mika-a2a` isolé qui compile `mika-common` ; sous
`cargo test --workspace` — ce que fait la CI — il est nul.

### D2 — Migration des cinq sites

Chacun passe de trois ou quatre lignes à deux, et **aucune assertion de test ne
change**. Les tests continuent d'asserter ce qu'ils assertaient ; seule la
fixture change de nature.

```rust
let dead = DeadEndpoint::reserve();
let url = format!("http://{}/a2a/cust-5/mika-prime", dead.addr());
```

### D3 — Le scan structurel : un listener dont on prend l'adresse et rien d'autre

**Aucun test comportemental ne peut voir cette classe.** Un sixième site écrit
demain ne rendrait *aucune* décision fausse : il passerait, sauf une fois sur
cent, sur une machine chargée, dans un job dont on relance le rouge sans le lire.
C'est la signature qui justifie un scan de source.

Le prédicat tient en une phrase, et couvre les deux formes présentes (`drop`
explicite **et** bloc d'initialisation) sans les distinguer :

> Pour chaque `let <id> = … TcpListener::bind …`, le fichier doit contenir, après
> la liaison, au moins une occurrence de `<id>` qui ne soit ni `<id>.local_addr…`
> ni `drop(<id>)`.

Un listener légitime est **servi** (`axum::serve(listener, …)`) ou **accepté**
(`listener.accept()`), donc il porte une telle occurrence. Un listener fautif
n'existe que pour rendre son adresse, donc il n'en porte aucune. Vérification sur
l'arbre :

| site | occurrences de `listener` après liaison | verdict |
|---|---|---|
| `spawn_mock` (remote_ask_integration:44) | `local_addr`, `axum::serve(listener, …)` | **vert** |
| `spawn_dropping_server` (remote_ask_recovery:125) | `local_addr`, `listener.accept()` | **vert** |
| `a_silent_server_…` (transport_failures:59) | `local_addr`, `listener.accept()` | **vert** |
| `server/mod.rs:1704` (production) | `axum::serve(listener, …)` | **vert** |
| les cinq de R1 | `local_addr` et/ou `drop` seulement | **rouge** |

Hébergé dans `crates/mika-agent/src/canonical_tokens.rs`, à côté de
`mika2495_aucune_fixture_ne_sonde_une_plage_de_documentation`, dont il réutilise
`all_rust_sources()` (toutes les sources Rust sous `crates/`, donc workspace-wide)
et `source_scan::strip_comment_lines`.

**Population non filtrée, et c'est une décision.** `source_scan::is_test_source_path`
répond `false` pour `cadence.rs` (test inline sous `src/`), donc filtrer sur lui
raterait le site 5. Et un `bind` suivi d'un `drop` en production serait tout aussi
suspect. Le scan regarde donc tout `crates/`.

**Angle mort assumé et écrit :** une liaison dont `let <id> =` et
`TcpListener::bind` sont sur deux lignes distinctes n'est pas vue. Les cinq sites
mesurés sont tous sur une ligne ; un scan qui attrape la forme réelle vaut mieux
qu'un scan qui prétend attraper toutes les formes concevables.

### D4 — Le texte prescripteur de mika#2495 est corrigé

Son message de résolution cesse de prescrire ``bind("127.0.0.1:0")`` puis `drop`
et nomme `DeadEndpoint::reserve()`. Le scan de mika#2495 est par ailleurs
**inchangé** : la boucle locale reste la bonne réponse contre les plages de
documentation, c'est sa *libération* qui ne l'était pas. Les deux gardes se
complètent — l'une refuse une adresse non routable, l'autre refuse un port
libéré — et le texte de chacune renvoie désormais au même helper.

---

## 5. Unités d'implémentation

- **U1** — `socket2 = "0.6"` dans `[workspace.dependencies]` ; dev-dep de
  `mika-common` (le helper le compile sous `test-utils`).
- **U2** — `crates/mika-common/src/dead_endpoint.rs` : le type, ses deux
  assertions de propriété, son doc-comment portant le raisonnement de la § 3.
  Déclaré dans `lib.rs` sous `#[cfg(any(test, feature = "test-utils"))]`.
- **U3** — `mika-common` avec `features = ["test-utils"]` en dev-dep de
  `mika-a2a` (nouvelle) ; déjà présente pour `mika-cli` et `mika-agent`.
- **U4** — Migration des cinq sites de R1.
- **U5** — Le scan D3 : `dead_listener_hits(content) -> Vec<String>` (fonction
  pure, testable sur chaîne) + le test qui l'applique à `all_rust_sources()`,
  son allowlist, sa non-vacuité de population et ses trois contrôles.
- **U6** — D4 : le message de résolution de mika#2495.
- **U7** — Une ligne dans la section **Conventions → Testing** du `CLAUDE.md`
  racine, nommant le helper comme la seule façon d'obtenir un endpoint mort.

---

## Fire-Disposition

Ce plan livre **un détecteur** : le scan structurel de D3 (U5). Un second
livrable a une forme de détecteur sans en être un — les deux assertions de
propriété de `DeadEndpoint::reserve()` (§ 3) sont une **précondition de fixture**,
pas un scan sur le dépôt : elles n'ont pas de population et rien à excepter.

**Disposition retenue : (a) exception nommée en allowlist — livrée VIDE.**

- `ALLOWED_DEAD_LISTENER_FIXTURES: &[&str] = &[]`, à côté de son voisin
  `ALLOWED_DOC_RANGE_FIXTURES`, lui aussi livré vide (mika#2495).
- **Elle peut être vide parce que U4 migre les cinq violations dans le même
  commit.** L'ordre est contraint : le scan atterrit *armé* sur un arbre déjà
  propre. C'est aussi l'argument qui justifie le périmètre de R1 — un scan livré
  avec quatre entrées d'allowlist serait le contraire de la doctrine mika#2201
  que ce module cite déjà : *« on déclare, on n'allowliste pas »*.
- **Quand il tire, on migre le site vers `DeadEndpoint`, on ne l'allowliste
  pas.** Le doc-comment de la constante le dit, sur le modèle de son voisin, et
  le message d'échec du scan nomme le helper comme résolution.
- **Assertion auto-nettoyante :** `mika2569_lallowlist_des_fixtures_est_livree_vide`
  rougit dès que la constante cesse d'être vide, avec pour message l'obligation de
  dater l'exception et de nommer son ticket de suivi. Elle rougit aussi si une
  entrée désigne un chemin qui n'existe plus — une exception périmée exempterait
  silencieusement un futur homonyme (comparaison **double sens**, motif
  `FIRED_AT_LITERAL_WRITERS`).

**Le détecteur est armé, jamais désarmé (option b écartée) :** un `#[ignore]`
sur un scan de source le rend indistinguable d'un arbre propre (classe mika#2205),
et il n'y a rien à mesurer avant de l'armer — sa population résiduelle est zéro,
par construction, au commit même.

---

## 6. Verification Contract

| # | vérification | attendu |
|---|---|---|
| **V1** | `cargo test -p mika-cli --test remote_ask_integration` | vert, les 12 tests |
| **V2** | `cargo test --workspace` | vert |
| **V3** | `cargo clippy --workspace --all-targets -- -D warnings` | vert |
| **V4** | `cargo fmt --check` | vert |
| **V5** | La réservation mord : `DeadEndpoint::reserve()` puis `std::net::TcpListener::bind(dead.addr())` | `Err` |
| **V6** | Le refus mord : `TcpStream::connect_timeout(dead.addr(), 2 s)` | `Err`, en < 1 s |
| **V7** | Le scan est **vu rouge** sur une fixture plantée (les deux formes : `drop` explicite, bloc d'initialisation) | rouge, message nommant le helper |
| **V8** | Le scan est **vu vert** sur une fixture de bonne foi (`axum::serve(listener, …)`, `listener.accept()`) | vert |
| **V9** | Non-vacuité : la population du scan est non vide et contient des sources de test | vert |
| **V10** | L'allowlist est vide et l'assertion la tient | vert |

### V11 — Le contrôle négatif d'AC2, **à voir rouge puis annuler**

Muter `TransportFailure::classify` (`crates/mika-a2a/src/error.rs:57`) pour que
la branche `is_connect()` rende `Interrupted` au lieu de `Unreachable`, puis
relancer V1. `dispatch_surfaces_connection_error_for_dead_endpoint` **doit**
rougir sur son assertion `chain.contains("unreachable")`. Annuler la mutation.

Sans ce passage, « le test assert la bonne chose » et « le test assert une chose
qui ne peut pas être fausse » rendent le même vert.

### V12 — Le contrôle négatif du remède lui-même, **à voir rouge puis annuler**

Remplacer `socket2::Socket` par `std::net::TcpListener` dans `reserve()` (donc
avec `SO_REUSEADDR`, et sans `listen` puisque la std le pose — le garde perd sa
propriété de réservation), puis relancer V5. Le bind concurrent **doit** réussir,
donc V5 rougir.

C'est ce qui sépare *« le helper réserve »* de *« le helper a l'air de
réserver »*, et ce qui attache la décision `socket2` à une mesure plutôt qu'à un
raisonnement noyau.

---

## 7. Surfaces opérateur

**Aucune.** Ni compteur, ni événement de journal, ni ligne `audit_events` : le
défaut est un rouge de CI et le remède vit entièrement dans le substrat de test.
Le seul instrument est le scan, dont le signal est son propre rouge en CI —
inventer une surface de journal ici reproduirait le défaut du Signal M
(mika#2050), un signal publié vers un puits que personne ne lit.

---

## 8. Ce que ce travail n'achète PAS

- **Il ne rend pas le rouge du 2026-09-28 reproductible.** La course dépend de
  l'ordonnancement ; rien ici ne la rejoue. Ce qui est livré est une fixture dont
  la course est **impossible par construction** (le port est tenu), pas une
  fixture dont la course est moins probable. C'est précisément pourquoi V5 assert
  la réservation plutôt que de compter des exécutions vertes : mille passages
  verts ne prouvent rien d'une course, un `EADDRINUSE` prouve la réservation.
- **Il ne ferme pas les autres classes de rouge aléatoire du CI.** Ce plan traite
  une classe nommée — l'endpoint mort obtenu par libération de port — et rien
  d'autre.
- **Il ne borne pas les formes multi-lignes** de la liaison (angle mort de D3,
  écrit au site du scan).
- **Le scan est un filet, pas un chemin.** S'il se met à tirer régulièrement,
  cela signifie que la prescription de D4 n'atteint pas ses lecteurs, et c'est
  **elle** qu'il faudra relire — jamais l'allowlist qu'il faudra allonger.

---

## 9. Definition of Done

- [ ] `socket2` en `[workspace.dependencies]`, dev-dep de `mika-common` (U1).
- [ ] `mika_common::dead_endpoint::DeadEndpoint` livré sous `test-utils`, avec ses
      deux assertions de propriété et son doc-comment (U2).
- [ ] `mika-common` + `test-utils` en dev-dep de `mika-a2a` (U3).
- [ ] Les **cinq** sites de R1 migrés ; aucune assertion de test modifiée (U4).
- [ ] Le scan D3 livré armé, allowlist vide, avec ses trois contrôles (U5).
- [ ] Le message de résolution de mika#2495 ne prescrit plus le motif fautif (U6).
- [ ] Une ligne de convention dans le `CLAUDE.md` racine (U7).
- [ ] V1–V10 verts ; V11 et V12 **vus rouges** puis annulés, et le rapport de PR
      le dit.
- [ ] `## Acceptance criteria` présent dans ce plan (gate `verify-pipeline.sh` U2).

---

## Acceptance criteria

Transcrits du corps de mika#2569 :

- [ ] **AC1 — Le test ne dépend plus d'un port libéré puis réutilisable.**
      Tenu par D1+D2 : le port est tenu par un socket vivant pendant toute la
      durée de l'appel, donc aucun `bind("127.0.0.1:0")` concurrent ne peut le
      recevoir. Vérifié par **V5** (le bind concurrent échoue) et par **V12**
      (vu rouge quand la réservation est retirée).
- [ ] **AC2 — Il reste rouge si `dispatch_remote` cesse de remonter l'erreur de
      connexion (contrôle négatif).** Les assertions du test sont **inchangées**
      (`unreachable` + l'URL + `never left this client`), et **V11** les fait voir
      rouge en mutant `TransportFailure::classify`. `V6` garantit de surcroît que
      le rouge est **rapide** : un endpoint qui pendrait au lieu de refuser ferait
      attendre le budget client (600 s) avant de rougir.

---

## 10. Hors périmètre, délibérément

- **Le scan mika#2495 lui-même**, dont seul le texte de résolution bouge (D4) :
  son prédicat, sa population et son allowlist sont intacts.
- **Les sites qui lient et servent** (`spawn_mock`, `spawn_dropping_server`,
  `a_silent_server_is_abandoned_on_the_clients_budget`, la production) : ils
  tiennent leur listener, ils n'ont pas la course, et les toucher élargirait le
  diff sans fermer quoi que ce soit.
- **`serial_test` sur ces binaires** : R4 établit que la sérialisation ne ferme
  pas la classe et coûte le parallélisme.
- **Toute adresse non routable** : refusée par mika#2495 avec sa mesure (R2).
- **Le budget client A2A** (600 s, `MIKA_A2A_TIMEOUT_SECS`) : ce plan ne le lit
  ni ne le déplace ; V6 le rend simplement hors d'atteinte sur ce chemin.
