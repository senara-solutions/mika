# mika#2759 — le plancher `client >= total` doit tenir pour une enveloppe per-agent

**Ticket :** senara-solutions/mika#2759
**Type :** fix
**Labels :** `bug`, `p2-normal`, `agent-core`, `ready`, `dispatch:loop`, `loop-substrate`
**Relie :** mika#2297 (le plancher), mika#2189 (les enveloppes per-agent), mika#2309 (le substrat et le contrôle positif), mika#2293 / mika#2457 (la cascade et son attestation)

---

## Contexte, et ce que la mesure dit exactement

Le plancher `client >= total` posé par mika#2297 est résolu sur **l'environnement du
process** ; les enveloppes per-agent posées par mika#2189 vivent dans le
`config.toml` **de l'agent**. Les deux ne se rencontrent jamais.

Re-vérifié le 2026-10-17 sur `39e6df30` :

- `crates/mika-a2a/src/client.rs:51` — `resolve_timeout_secs(a2a, total)` est
  **privée** et ne reçoit que deux lectures de `std::env` :
  `MIKA_A2A_TIMEOUT_SECS` et `MIKA_AGENT_TOTAL_TIMEOUT_SECS`. Jamais un fichier.
- `crates/mika-agent/src/well_known_agents.rs:1659` — `MIKA_ARCH_CONFIG` porte
  `agent_total_timeout_secs = 900`.
- `crates/mika-a2a/src/client.rs:63` — `A2aClient::new` appelle
  `resolve_send_timeout()` **à la construction**, avant que l'URL ne soit
  décomposée en agent.

Sur un appel vers mika-arch le client résout donc **600 s** face à une enveloppe de
**900 s** : il abandonne une génération que le moteur a encore le droit de finir —
le sinistre du 2026-09-11 qui a motivé le passage de 300 à 600.

### Ce que la mesure ajoute au corps du ticket

Trois faits, mesurés pendant le grooming, qui resserrent le périmètre.

1. **Le rayon de souffle est plus étroit que « le client a2a ».** Il y a deux sites
   de construction en production. `crates/mika-agent/src/tools/a2a_call.rs:123`
   vise, **par conception SSRF**, des agents *externes* : son garde refuse
   `localhost`, `127.0.0.1`, `::1` et tout IP privé/loopback/link-local
   (`LOCAL_TARGET_HINT`). Le `config.toml` de sa cible n'est pas sur cette machine :
   le plancher per-agent y est **structurellement illisible**, et ce site sort du
   périmètre — pas par économie, par impossibilité.
   `crates/mika-cli/src/remote_ask.rs:476` est l'autre, et c'est celui que
   `mika ask --agent <a>` emprunte, donc celui que `_arch_ask` emprunte.
2. **La prémisse de la direction 2 est partiellement périmée.** `mika-a2a` a déjà
   `mika-common` dans son `Cargo.toml` — en dev-dependency, `features =
   ["test-utils"]`, posée par mika#2569, avec le raisonnement de non-cycle écrit sur
   place (« la chaîne est `common → a2a → agent → cli` »). L'objection n'est donc
   pas « aucune dépendance maison » mais « promouvoir une dev-dependency en
   dépendance de runtime », ce qui est une autre question.
3. **Le lecteur canonique de la cascade per-agent existe déjà, et il interdit sa
   propre copie.** `BudgetProvenance::resolve(global_home, agent_home)
   .effective_budget().agent_total_timeout_secs()` (mika#2293) reproduit la cascade
   réelle, ne panique jamais, et son doc-comment se déclare **troisième** parcours de
   cette cascade en disant qu'un quatrième devrait être une primitive partagée, pas
   une quatrième copie.

### La contrainte qui tranche la question

Le site le plus commode — faire lire l'enveloppe par `mika-cli` à l'endroit où
`agent_name` est déjà en portée (`crates/mika-cli/src/commands/ask.rs:466-470`) —
est **interdit structurellement**.
`crates/mika-cli/src/commands/agents.rs:1760`, `mika2457_the_cli_resolves_no_budget_locally`,
est un scan de source sur tout le crate dont les aiguilles sont `BudgetProvenance`,
`ModelProvenance`, `resolve_llm_budget_record`, `effective_budget`,
`effective_llm_http_timeout_secs`, `http_timeout_secs()`, `llm_timeout_budget`. Son
allowlist est **livrée vide**, et son message dit en toutes lettres : *« quand ce
scan tire, on RETIRE le second lecteur, on ne l'allowliste pas — une allowlist qui
cesse d'être vide ici est le faux vert mika#2304 réintroduit »*. Doctrine :
**« Spirit atteste, le CLI rend ; jamais l'inverse »** (mika#2457).

C'est la contrainte, et c'est aussi la solution : elle interdit de **résoudre** un
budget dans le CLI, elle n'interdit pas de **lire celui que spirit a attesté**.
`commands/agents.rs:95-146` fait déjà exactement cette lecture, et il est vert.

---

## La direction retenue

**Le plancher est résolu au moment de l'envoi, l'agent est dérivé de l'URL, et
l'enveloppe est lue sur l'attestation de spirit — jamais résolue localement.**

C'est la direction 1 du ticket, **déplacée chez l'appelant** et avec la source de
l'enveloppe changée. Les deux déplacements sont ce qui annule ses objections.

| ce que le ticket reprochait à la direction 1 | pourquoi ça ne s'applique plus |
|---|---|
| « change la sémantique de `with_timeout` » | `with_timeout` est appelé tel quel, inchangé. La dérivation vit dans `mika-cli`, pas dans `mika-a2a`. |
| « change le partage du `reqwest::Client` » | il n'y a pas de partage à changer : `A2aClient::new` construit déjà un `reqwest::Client` par envoi, et `send_message_to_agent` en construit un par appel. |

### Pourquoi pas les deux autres directions

- **Direction 2 (nom d'agent au constructeur + lecture du `config.toml`)** —
  refusée, et pas pour la raison du ticket. Le défaut n'est pas la dépendance : c'est
  qu'un *client* lisant le `config.toml` de son *serveur* sur le disque local
  n'est correct que si les deux sont co-localisés, ce que `A2aClient` ne peut pas
  vérifier. Sur le chemin `--remote` il lirait le `mika-arch/config.toml` de **cette**
  machine pour un mika-arch distant : un plancher **faux**, posé avec l'autorité
  d'une mesure. Un faux plancher est strictement pire qu'une absence de plancher,
  exactement l'argument que `budget_provenance.rs` écrit sur sa propre
  reconstruction de cascade.
- **Direction 3 (aligner `DEFAULT_TIMEOUT` sur 900)** — refusée par le ticket
  lui-même, et le hors-périmètre de mika#2309 l'interdit sans arbitrage. Le plancher
  redeviendrait faux au prochain agent qui lève son enveloppe, sans que rien ne le
  dise.

### Le mécanisme, en quatre pièces

1. **`mika-a2a` publie son cœur pur et un point d'entrée à plancher.**
   `resolve_timeout_secs(a2a: Option<u64>, total: Option<u64>) -> u64` devient
   `pub` (elle est déjà pure et déjà testée comme telle), et
   `resolve_send_timeout_with_floor(agent_envelope_secs: Option<u64>) -> Duration`
   s'ajoute à côté de `resolve_send_timeout()`. L'enveloppe attestée n'est **pas une
   nouvelle arithmétique** : c'est un troisième candidat au `max` déjà écrit —
   `resolve_timeout_secs(a2a_env, max(total_env, attested))`. Aucun littéral à un
   site bornant, donc `a2a-timeout-literal-lint` ne voit rien de nouveau.
   `resolve_send_timeout()` reste le chemin `None` et ne change pas de comportement.

2. **`mika-cli` gagne un module `budget_floor` avec un prédicat de cible fermé.**
   `local_a2a_agent(url) -> Option<String>` n'accepte une cible que si son hôte est
   loopback (`localhost` / `127.0.0.1` / `::1`) **et** si son chemin a la forme
   `/a2a/{nom}` à un seul segment. C'est le prédicat SSRF de `a2a_call.rs` **inversé**,
   et la symétrie est le fond du raisonnement : `a2a_call` refuse le loopback parce
   qu'une cible locale n'est pas de son ressort ; le plancher n'accepte **que** le
   loopback parce qu'une cible distante n'est pas du ressort de cette machine. Le
   chemin gateway et le chemin `--remote` tombent donc hors population par
   construction, sans appel réseau et sans ligne de log.

3. **L'enveloppe est lue sur l'attestation, à la même origine que l'envoi.**
   L'URL d'attestation est construite depuis l'URL cible elle-même
   (`{scheme}://{authority}/api/v1/agents/{nom}/budget`), **pas** depuis un
   `spirit_url()` résolu de son côté : on interroge le serveur qu'on est sur le point
   d'appeler, pas un homonyme. `GET` avec bearer `MIKA_INTERNAL_TOKEN`, budget court
   nommé (`const ATTESTATION_TIMEOUT_SECS: u64 = 5`, jamais un littéral à un site
   bornant). Le parseur est **pur et séparé du fetch** :
   `envelope_from_attestation(&serde_json::Value) -> Option<u64>` lit
   `budget.agent_total_timeout_secs` — ce que `handle_agent_budget`
   (`crates/mika-agent/src/server/dashboard.rs:327`) sert réellement. Aucune aiguille
   de `mika2457_…` n'est écrite : le module ne nomme ni `BudgetProvenance`, ni
   `effective_budget`, ni `llm_timeout_budget`. Il lit un champ JSON d'un record que
   spirit a figé à l'`init_agent`.

4. **Le câblage est au seul site qui en a besoin, et ne change aucune signature.**
   `send_message_to_agent` remplace son `A2aClient::new(url, token)` par
   `A2aClient::with_timeout(url, token, resolve_send_timeout_with_floor(floor))`.
   Pas de septième paramètre positionnel, pas de point d'entrée jumeau, pas un seul
   des ~12 appelants (dont `tests/remote_ask_recovery.rs` et
   `tests/remote_ask_integration.rs`) à retoucher — et le plancher s'applique à tout
   appelant *futur* du chemin a2a local sans que personne ait à y penser.

### Dégradation, nommée plutôt que découverte

Le CLI **rend** « non attesté » sans valeur quand il affiche un budget ; un transport,
lui, a besoin d'un nombre. Les trois arbres de dégradation résolvent donc vers le
comportement d'aujourd'hui — `resolve_send_timeout()`, c'est-à-dire exactement la
valeur servie avant ce changement — et le disent :

| cas | plancher | ligne |
|---|---|---|
| cible non loopback / pas `/a2a/{nom}` | aucun | `debug!` `a2a_floor_not_local` |
| spirit injoignable | aucun | `warn!` `a2a_floor_unattested`, `cause = "unreachable"` |
| 404 (agent non résolu sur ce serveur — `handle_agent_budget` ne résout pas en différé) | aucun | `warn!` idem, `cause = "status_404"` |
| corps sans `budget.agent_total_timeout_secs` | aucun | `warn!` idem, `cause = "malformed"` |
| attestation lue | `max(client résolu, enveloppe)` | `debug!` `a2a_floor_resolved` |

`attested_envelope_secs` est **absent** du champ, jamais `0`, quand rien n'a été
attesté (motif mika#2331 : absent n'est pas zéro).

**L'arbre nominal est en `debug!` et non en INFO, et c'est un coût nommé, pas un
oubli.** La doctrine mika#2293 (« un réglage qu'on ne peut pas observer n'est pas un
réglage, c'est un espoir ») vise un service de longue vie, dont l'événement de
configuration est émis une fois par agent et dédoublonné. Ici le processus est un CLI
invoqué des milliers de fois par jour par `_arch_ask`, pour une valeur qui ne change
pas entre deux invocations : une ligne INFO par `mika ask` serait du bruit sur l'arbre
sain. La surface d'observation à la demande existe déjà et c'est la bonne —
`mika agents budget --agent <a>` rend l'enveloppe attestée avec son `resolved_at`
(mika#2457, mika#2473). Les arbres **dégradés**, eux, sont en WARN : leur régime
attendu est zéro, et un WARN par invocation sur un spirit injoignable est précisément
le signal voulu.

---

## Hors périmètre, et pourquoi

- **`crates/mika-agent/src/tools/a2a_call.rs`** — cibles externes par conception SSRF.
  Le `config.toml` de la cible n'est pas sur cette machine et son spirit n'expose pas
  `/api/v1/agents/{id}/budget` au public. Impossible, pas différé.
- **Le chemin `--remote` / la gateway** — l'endpoint d'attestation n'est pas exposé à
  travers la gateway, et l'enveloppe d'un agent distant n'est pas une affaire de cette
  machine. Hors population par le prédicat loopback, sans appel réseau.
- **`recover_by_context` (`remote_ask.rs:384`)** — `RECOVERY_TIMEOUT` (30 s) est
  délibéré : une sonde de reprise n'est pas une génération. Aucun plancher ne s'y
  applique, et le texte de mika#2036 reste valable.
- **Toute modification de valeur** : ni `DEFAULT_TIMEOUT`, ni `RECOVERY_TIMEOUT`, ni
  `agent_total_timeout_secs = 900` ne bougent. Le hors-périmètre du ticket est tenu
  littéralement : l'écart entre la constante et l'enveloppe **subsiste**, il est
  désormais franchi à l'envoi.
- **Promouvoir `mika-common` en dépendance de runtime de `mika-a2a`** — non requis
  par cette direction, donc non fait.

---

## Livrables et ordre de commit

Chaque étape est verte seule.

1. **`crates/mika-a2a/src/client.rs`** — `resolve_timeout_secs` publiée avec le
   doc-comment qui dit pourquoi (c'est le cœur pur, testable sans toucher à l'env
   process) + `resolve_send_timeout_with_floor`. Tests du `max` à trois entrées :
   enveloppe seule, env seule, les deux (la plus grande gagne), aucune (= comportement
   d'aujourd'hui), enveloppe inférieure au défaut (ne rabaisse rien).
2. **`crates/mika-cli/Cargo.toml`** — `url.workspace = true` (déjà déclarée au
   workspace, `url = "2"`).
   **`crates/mika-cli/src/lib.rs`** — `pub mod budget_floor;`.
   **`crates/mika-cli/src/budget_floor.rs`** — `local_a2a_agent`,
   `envelope_from_attestation`, `attested_envelope_secs` (le seul fetch), les
   événements. Tests **purs** du prédicat (loopback vs hôte distant, chemin à deux
   segments, chemin sans `/a2a/`, schéma non http) et du parseur (champ présent,
   absent, nul, type faux, corps d'erreur 404).
3. **`crates/mika-cli/src/remote_ask.rs`** — le câblage. Le changement de
   comportement runtime atterrit ici, et seulement ici.
4. **`crates/mika-cli/src/budget_floor.rs`** (tests) —
   `mika2759_le_plancher_a_un_seul_site` + son contrôle de non-vacuité.
5. **`crates/mika-agent/src/well_known_agents.rs`** — retrait de
   `mika2309_client_default_is_below_the_arch_envelope` et atterrissage de son
   successeur `mika2759_le_plancher_couvre_lenveloppe_de_mika_arch` (voir
   § Fire-Disposition, point 4).
6. **Docs** (hors compte) — un paragraphe dans `crates/mika-a2a/CLAUDE.md` (le
   plancher per-agent est tenu à l'envoi, par l'attestation, sur le chemin loopback
   seulement ; `tests/a2a_budget_transitivity_2309.rs` garde son périmètre déclaré et
   ne change pas) et dans `crates/mika-cli/CLAUDE.md` (le module, le prédicat, les
   quatre arbres).

---

## Verification Contract

| # | vérification | commande |
|---|---|---|
| V1 | suite complète verte | `cargo test` |
| V2 | le garde mika#2457 reste vert **et son allowlist reste vide** | `cargo test -p mika-cli mika2457` |
| V3 | aucun littéral à un site bornant sur le chemin a2a, allowlist toujours vide des deux côtés | `scripts/check-a2a-timeout-literals.sh` |
| V4 | la transitivité mika#2309 est intacte | `cargo test -p mika-agent --test a2a_budget_transitivity_2309` |
| V5 | le plancher couvre réellement l'enveloppe de mika-arch | `cargo test -p mika-agent mika2759` |
| V6 | le scan « un seul site » voit quelque chose et refuse un second site | `cargo test -p mika-cli mika2759` |
| V7 | lint + format | `cargo clippy --all-targets -- -D warnings` && `cargo fmt --check` |
| V8 | pipeline | `scripts/verify-pipeline.sh origin/main` |

Preuve manuelle à joindre au corps de PR : la valeur résolue pour une cible
`http://localhost:8080/a2a/mika-arch` avec l'attestation à 900 (**900 s**) et sans
attestation (**600 s**, inchangé).

---

## Note de migration (pre-1.0, changement de comportement runtime)

`mika ask --agent <a>` vers un spirit loopback effectue désormais **une requête
`GET` supplémentaire** (`/api/v1/agents/{a}/budget`, budget 5 s) avant l'envoi, et
peut retenir un timeout client **plus grand** que `MIKA_A2A_TIMEOUT_SECS`.

Ce n'est pas une sémantique nouvelle : depuis mika#2297, `MIKA_A2A_TIMEOUT_SECS` est
un **défaut-et-plancher**, jamais un plafond — `resolve_timeout_secs` fait déjà
`a2a.unwrap_or(DEFAULT).max(total.unwrap_or(0))`, donc
`MIKA_AGENT_TOTAL_TIMEOUT_SECS` pouvait déjà le relever. Ce changement élargit
l'ensemble des entrées du **même** `max`. Un opérateur qui veut un plafond strict n'en
avait pas avant et n'en a pas après.

Rien à migrer : aucune variable renommée, aucune clé de config ajoutée, aucun défaut
changé. Un spirit qui n'expose pas l'endpoint (binaire antérieur à mika#2457) sert la
valeur d'avant, avec un WARN qui la nomme.

---

## Risques

1. **L'endpoint d'attestation ne résout pas en différé.** `handle_agent_budget` lit
   `state.agents.get(&agent_id)` sans passer par `resolve_agent()` (mika#1399) : un
   agent créé après le démarrage du serveur donne 404 jusqu'à ce que quelque chose
   d'autre le résolve. Conséquence acceptée et nommée : plancher non posé, WARN
   `status_404`, comportement d'avant. **Ne pas** « corriger » l'endpoint dans ce vol
   — c'est un autre sujet, sur un autre crate, avec sa propre question d'autorisation.
2. **Un coût réseau sur le chemin le plus chaud.** Une requête loopback de plus par
   `mika ask`, bornée à 5 s. Si le spirit est lent, l'envoi est retardé d'autant.
   Atténuation : budget court et nommé, et l'échec est fail-open, jamais bloquant.
3. **Le prédicat est sur l'URL, donc textuel.** Une forme d'URL non prévue
   (IPv6 entre crochets, port absent, chemin avec `/` final) sort de la population
   plutôt que de planter — le `Option<String>` est la forme de ce refus, et les tests
   du prédicat couvrent chacune de ces formes.
4. **Deux lecteurs du même endpoint dans `mika-cli`** : `commands/agents.rs::budget`
   (qui rend tout le record) et `budget_floor` (qui lit un champ comme plancher de
   transport). Duplication **acceptée et motivée** : `agents.rs` miroite
   délibérément `ResolvedBudgetRecord` plutôt que de l'importer, pour tolérer le
   décalage de version, et les deux consommations n'ont pas la même forme d'échec
   (l'une rend « non attesté », l'autre doit rendre un nombre). Ce qui est tenu
   structurellement, c'est l'unicité du **site de plancher**, par le scan du
   point 1 de la Fire-Disposition.

---

## Definition of Done

- `resolve_timeout_secs` est publique et documentée comme le cœur pur ;
  `resolve_send_timeout_with_floor` existe à côté de `resolve_send_timeout`, qui ne
  change pas de comportement.
- `crates/mika-cli/src/budget_floor.rs` existe, avec son prédicat loopback fermé, son
  parseur pur, son unique fetch, et ses quatre arbres (`not_local`, `unreachable`,
  `status_<code>`, `malformed`) tous testés.
- `send_message_to_agent` pose le plancher ; aucune de ses signatures ni aucun de ses
  ~12 appelants n'a changé.
- `mika2309_client_default_is_below_the_arch_envelope` est **retiré** avec sa cause,
  et son successeur mika#2759 est vert.
- V1 à V8 passent. Les deux allowlists existantes (`scripts/a2a-timeout-allowlist.txt`,
  `ALLOWED` de `mika2457_…`) sont **encore vides** : aucune exception demandée.
- `crates/mika-a2a/CLAUDE.md` et `crates/mika-cli/CLAUDE.md` portent le mécanisme et
  le périmètre loopback.
- Le corps de PR porte la note de migration et la preuve manuelle 900 / 600.

---

## Acceptance criteria

1. **AC1 — le plancher est tenu sur une enveloppe per-agent.** Pour une cible
   `http://localhost:<port>/a2a/mika-arch` dont l'attestation porte
   `agent_total_timeout_secs = 900`, le timeout du `A2aClient` construit par
   `send_message_to_agent` vaut **900 s** et non 600.
2. **AC2 — rien ne baisse jamais.** Une enveloppe attestée *inférieure* au client
   résolu ne rabaisse pas le client : le résultat est un `max`, vérifié par un test
   dédié.
3. **AC3 — l'absence d'attestation rend exactement la valeur d'avant.** Les quatre
   arbres de dégradation résolvent vers `resolve_send_timeout()`, et chacun a son
   test.
4. **AC4 — une cible non loopback ne déclenche aucun appel réseau.** Le prédicat
   refuse avant tout `GET` ; un test l'atteste sur une URL gateway et sur une URL
   `--remote`.
5. **AC5 — le CLI ne résout toujours aucun budget localement.**
   `mika2457_the_cli_resolves_no_budget_locally` est vert avec `ALLOWED` **vide** ;
   aucune de ses sept aiguilles n'apparaît en production dans le nouveau module.
6. **AC6 — aucun budget littéral à un site bornant.**
   `scripts/check-a2a-timeout-literals.sh` sort 0, et
   `scripts/a2a-timeout-allowlist.txt` reste vide (comparée dans les deux sens).
7. **AC7 — l'exception mika#2309 est retirée avec sa cause.**
   `mika2309_client_default_is_below_the_arch_envelope` n'existe plus ; son successeur
   lit les deux mêmes données réelles (`MIKA_ARCH_CONFIG`, le résolveur) et affirme
   que le plancher les couvre.
8. **AC8 — le site de plancher est unique et le scan qui le tient n'est pas vide.**
   `mika2759_le_plancher_a_un_seul_site` refuse un second appel à
   `resolve_send_timeout_with_floor` hors `remote_ask.rs`, allowlist vide ; son
   contrôle de non-vacuité atteste que le scanner voit le crate.
9. **AC9 — aucune valeur de configuration n'a bougé.** `DEFAULT_TIMEOUT` (600),
   `RECOVERY_TIMEOUT` (30) et `agent_total_timeout_secs = 900` sont inchangés dans le
   diff.
10. **AC10 — la garde B de mika#2309 est intacte.**
    `crates/mika-agent/tests/a2a_budget_transitivity_2309.rs` n'est pas modifié et
    reste vert : son périmètre déclaré (défauts + env du process) est toujours exact.

---

## Fire-Disposition

Le plan livre des détecteurs. Disposition retenue : **(a) exception nommée en
allowlist — et les deux allowlists concernées sont livrées VIDES et mesurées vides au
HEAD**, ce qui est l'idiome de la maison (mika#2425, mika#2457) : aucune exception
n'est demandée parce que la direction a été choisie pour n'en avoir besoin d'aucune.

1. **`mika2759_le_plancher_a_un_seul_site`** (nouveau) — scan de source sur
   `crates/mika-cli/src` via `ProductionScanner::for_crate`, aiguille
   `resolve_send_timeout_with_floor`, autorisée au seul chemin `remote_ask.rs`
   (contrainte de **chemin**, pas entrée d'allowlist — forme mika#2425).
   `const ALLOWED: &[&str] = &[]`, **mesuré vide au HEAD** (zéro occurrence
   aujourd'hui). *Quand il tire : on route le site par
   `remote_ask::send_message_to_agent`, on n'ajoute pas de ligne* — doctrine mika#2201
   et message explicite du garde frère mika#2457.
2. **`mika2759_le_scan_du_plancher_nest_pas_vide`** (nouveau) — l'assertion
   auto-nettoyante exigée par (a) : le scanner doit voir > 10 fichiers **et** voir
   `remote_ask.rs`. Une racine dérivée ou une source masquée rendent le point 1
   vacuement vert ; ce contrôle rougit à leur place. Forme et raison :
   `mika2457_the_local_resolution_scan_is_not_vacuous`.
3. **`mika2759_le_plancher_couvre_lenveloppe_de_mika_arch`** (nouveau, successeur) —
   lit `agent_total_timeout_secs` dans `MIKA_ARCH_CONFIG` et affirme
   `resolve_timeout_secs(None, Some(enveloppe)) >= enveloppe`. Il passe par le **cœur
   pur**, donc sans lire l'env process : pas de `#[serial]`, pas de course. Il rougit
   si le plancher régresse, et reste vert si quelqu'un relève `DEFAULT_TIMEOUT`
   (auquel cas le plancher tient par une autre route — ce qui est correct).
4. **Retrait de `mika2309_client_default_is_below_the_arch_envelope`** — c'est la
   discipline mika#1574 appliquée à la lettre, et il faut la dire parce qu'elle
   surprend. Ce contrôle positif auto-nettoyant **ne rougira pas** : le plan ne touche
   pas `DEFAULT_TIMEOUT`, donc l'écart 600/900 subsiste et le test reste vert. Mais sa
   **cause** est close : l'écart n'est plus un défaut non tenu, c'est une constante
   délibérée au-dessus de laquelle le plancher se lève à l'envoi. Un contrôle vert dont
   la cause est close est exactement l'exception qui survit à sa cause — le mode
   d'échec que mika#1574 existe pour empêcher. Il est donc **retiré avec son ticket de
   suivi (celui-ci)** et remplacé par le point 3, qui épingle ce qui est désormais vrai
   et porteur. Il n'est **pas** ajusté.
5. **Détecteurs existants, aucune exception demandée** —
   `a2a-timeout-literal-lint` (A1 : aucun littéral au site bornant ; A2 : tout
   `const … : Duration >= 60 s` est un `DEFAULT_*` adossé à un `MIKA_*`) reste vert
   parce que `ATTESTATION_TIMEOUT_SECS` est un `u64` nommé à 5 s, donc ni littéral au
   site bornant (A1 admet `from_secs(IDENT)`) ni concerné par A2.
   `mika2457_the_cli_resolves_no_budget_locally` reste vert parce qu'aucune de ses sept
   aiguilles n'est écrite : le module lit un champ JSON attesté, il ne résout rien.
   **Si l'une des deux rougit, l'implémentation s'arrête et remonte (option c)** — ces
   deux allowlists ne s'ouvrent pas sous l'autorité de ce vol.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-a2a/src/client.rs` (cœur pur publié + `resolve_send_timeout_with_floor`) | 35 |
| `crates/mika-a2a/src/client.rs` (tests du `max` à trois entrées) | 45 |
| `crates/mika-cli/src/budget_floor.rs` (prédicat, parseur, fetch, événements) | 95 |
| `crates/mika-cli/src/budget_floor.rs` (tests purs : prédicat, parseur, arbres d'échec) | 85 |
| `crates/mika-cli/src/budget_floor.rs` (scan « un seul site » + contrôle de non-vacuité) | 50 |
| `crates/mika-cli/src/remote_ask.rs` (câblage dans `send_message_to_agent`) | 15 |
| `crates/mika-cli/src/lib.rs` + `Cargo.toml` (déclarations) | 2 |
| `crates/mika-agent/src/well_known_agents.rs` (retrait mika#2309 + successeur) | 35 |

Total estimé : 362 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 362 < 429 et 362 < 1000 : les deux bornes
passent, la plus serrée avec ~67 lignes de marge. Le périmètre est fermé par
construction — **un** module neuf, **quatre** fichiers touchés, aucune signature
publique modifiée, donc **zéro** appelant à retoucher (c'est le gain principal de la
dérivation depuis l'URL : les ~12 appelants de `send_message_to_agent`, dont deux
fichiers de tests d'intégration, restent intacts). Aucune migration de schéma, aucun
renommage, aucune réécriture de test existant sauf le retrait nommé du point 4 de la
Fire-Disposition. Les deux postes qui auraient fait déborder ont été écartés
délibérément et pour des raisons de justesse, pas de budget : un serveur HTTP stub
(~60 lignes de harnais) est inutile dès lors que le parseur est séparé du fetch et
testé purement, et un test de bout en bout de `send_message_to_agent` est
**inobservable** — la fonction rend un `Task`, jamais le client, donc la seule couture
assertable est celle que les points 1 à 3 de la Fire-Disposition couvrent déjà.
