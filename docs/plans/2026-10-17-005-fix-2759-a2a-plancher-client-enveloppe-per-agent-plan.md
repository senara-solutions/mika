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

Re-vérifié le 2026-10-17 sur `46fdf64d` (re-mesuré au grooming de ce vol) :

- `crates/mika-a2a/src/client.rs:51` — `resolve_timeout_secs(a2a, total)` est
  **privée** et ne reçoit que deux lectures de `std::env` :
  `MIKA_A2A_TIMEOUT_SECS` et `MIKA_AGENT_TOTAL_TIMEOUT_SECS`. Jamais un fichier.
- `crates/mika-agent/src/well_known_agents.rs:1659` — `MIKA_ARCH_CONFIG` porte
  `agent_total_timeout_secs = 900`.
- `crates/mika-a2a/src/client.rs:78-80` — `A2aClient::new` appelle
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

Trois faits de plus, mesurés au re-grooming du 2026-10-17, qui **corrigent** ce que
la première rédaction de ce plan tenait pour acquis. Chacun change un livrable.

4. **Le périmètre du lint a2a ne couvrirait pas le nouveau module.**
   `scripts/check-a2a-timeout-literals.sh:98-103` déclare un `PERIMETER` **fermé** de
   quatre entrées : `crates/mika-a2a/src`, `crates/mika-cli/src/remote_ask.rs`,
   `crates/mika-agent/src/tools/a2a_call.rs`, `crates/mika-common/src/llm`.
   `crates/mika-cli/src/budget_floor.rs` n'en fait pas partie. La conformité de
   `ATTESTATION_TIMEOUT_SECS` serait donc vraie mais **non vérifiée** — le lint
   resterait vert parce qu'il ne regarde pas, ce qui est le faux vert que ce garde
   existe pour refuser. Le commentaire du périmètre dit lui-même sa règle
   d'appartenance : *« These are the files that actually carry an a2a exchange or the
   LLM budget cascade behind it. »* Un module qui borne un `GET` et calcule le
   plancher d'un envoi a2a en est un. **Le périmètre s'étend dans ce vol** (livrable 7).
5. **Le token de l'attestation est déjà en portée, et l'endpoint l'accepte.**
   `remote_ask.rs:473-475` — `send_message_to_agent` lit elle-même
   `MIKA_INTERNAL_TOKEN` (`.filter(|t| !t.is_empty())`) avant de construire le client.
   Et `crates/mika-agent/src/server/mod.rs:163,184` place `/agents/{id}/budget` sur les
   **dashboard routes**, documentées *« read-only, accepts dashboard OR internal
   token »*. L'attestation réutilise donc le token déjà lu : pas de second canal de
   secret, pas d'appel à `commands::dashboard::auth_token()`, et rien de nouveau à
   lire dans l'env. Un token absent devient un arbre de dégradation nommé de plus
   (`status_401`), pas une erreur.
6. **Les tests du `max` existent déjà et sont à étendre, pas à écrire.**
   `client.rs:222-235`, `resolve_send_timeout_floors_the_client_budget_at_the_engine_total`,
   couvre déjà les cinq cas à deux entrées sur le cœur pur. Le troisième candidat
   s'ajoute à ce test existant ; le poste correspondant passe de 45 à 15 lignes
   (§ Taille estimée).

### La contrainte qui tranche la question

Le site le plus commode — faire lire l'enveloppe par `mika-cli` à l'endroit où
`agent_name` est déjà en portée (`crates/mika-cli/src/commands/ask.rs:469-473`, où
`spirit_endpoint` est assemblé en `{spirit_url()}/a2a/{agent_name}`) — est
**interdit structurellement**.
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

3. **L'enveloppe est lue sur l'attestation, à la même origine que l'envoi, avec le
   token déjà en portée.** L'URL d'attestation est construite depuis l'URL cible
   elle-même (`{scheme}://{authority}/api/v1/agents/{nom}/budget`), **pas** depuis un
   `spirit_url()` résolu de son côté : on interroge le serveur qu'on est sur le point
   d'appeler, pas un homonyme. Le bearer est le `auth_token` que
   `send_message_to_agent` **a déjà lu** (`remote_ask.rs:473-475`) — l'endpoint siège
   sur les dashboard routes, qui acceptent le token interne (fait 5). Rien de neuf
   n'est lu dans l'env, et aucun second canal de secret n'est ouvert : le plancher
   s'authentifie auprès du serveur qu'il appelle avec la clé de cet appel-là.
   Budget court nommé (`const ATTESTATION_TIMEOUT_SECS: u64 = 5`, jamais un littéral à
   un site bornant). Le parseur est **pur et séparé du fetch** :
   `envelope_from_attestation(&serde_json::Value) -> Option<u64>` lit
   `budget.agent_total_timeout_secs` — ce que `handle_agent_budget`
   (`crates/mika-agent/src/server/dashboard.rs:327-345`) sert réellement, sous la clé
   `budget`, aux côtés de `model_drift` qu'on ignore. Aucune aiguille de `mika2457_…`
   n'est écrite : le module ne nomme ni `BudgetProvenance`, ni `effective_budget`, ni
   `llm_timeout_budget`. Il lit un champ JSON d'un record que spirit a figé à
   l'`init_agent`.

4. **Le câblage est au seul site qui en a besoin, et ne change aucune signature.**
   `send_message_to_agent` remplace son `A2aClient::new(url, token)`
   (`remote_ask.rs:476`) par
   `A2aClient::with_timeout(url, token, resolve_send_timeout_with_floor(floor))`.
   Pas de septième paramètre positionnel, pas de point d'entrée jumeau, pas un seul
   des ~12 appelants (dont `tests/remote_ask_recovery.rs` et
   `tests/remote_ask_integration.rs`) à retoucher — et le plancher s'applique à tout
   appelant *futur* du chemin a2a local sans que personne ait à y penser.

   **Contrainte de forme sur ce site, et elle est gardée.**
   `mika1951_both_ask_doors_post_the_isolation_key_through_one_site`
   (`remote_ask.rs:1766`) n'est pas un test de comportement : c'est un scan de source
   qui découpe la production sur `"pub async fn send_message_to_agent("` puis sur le
   `"\npub "` suivant, et exige que `build_send_params(` se trouve dans cette tranche —
   et qu'il n'y ait **qu'un** appel en production. Le câblage s'insère donc *avant*
   `build_send_params`, dans le corps, sans introduire de `pub ` en début de ligne
   entre les deux et sans extraire l'envoi vers un helper. Dit autrement : le plancher
   se calcule dans `send_message_to_agent`, il ne la découpe pas. Ce garde ne figure
   pas dans le Verification Contract d'origine ; il y entre (V9).

### Dégradation, nommée plutôt que découverte

Le CLI **rend** « non attesté » sans valeur quand il affiche un budget ; un transport,
lui, a besoin d'un nombre. Tous les arbres de dégradation résolvent donc vers le
comportement d'aujourd'hui — `resolve_send_timeout()`, c'est-à-dire exactement la
valeur servie avant ce changement — et le disent :

| cas | plancher | ligne |
|---|---|---|
| cible non loopback / pas `/a2a/{nom}` | aucun | `debug!` `a2a_floor_not_local` |
| spirit injoignable | aucun | `warn!` `a2a_floor_unattested`, `cause = "unreachable"` |
| 404 (agent non résolu sur ce serveur — `handle_agent_budget` ne résout pas en différé) | aucun | `warn!` idem, `cause = "status_404"` |
| 401 (pas de token lisible, ou pas celui du serveur) | aucun | `warn!` idem, `cause = "status_401"` |
| tout autre statut non-2xx | aucun | `warn!` idem, `cause = "status_<code>"` |
| corps sans `budget.agent_total_timeout_secs` | aucun | `warn!` idem, `cause = "malformed"` |
| attestation lue | `max(client résolu, enveloppe)` | `debug!` `a2a_floor_resolved` |

Les trois statuts sont **un seul arbre** (`status_<code>`) et non trois branches : le
code est porté dans le champ plutôt que dans le nom de l'événement, forme déjà retenue
par `UnattestedCause::Status(u16)` (`commands/agents.rs`). Un token absent emprunte
cette même branche — on envoie la requête sans en-tête `Authorization` et le serveur
répond 401, plutôt que de court-circuiter : c'est le serveur qui décide si la clé lui
convient, pas le client qui le devine.

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
   process) + `resolve_send_timeout_with_floor`. Le test existant
   `resolve_send_timeout_floors_the_client_budget_at_the_engine_total`
   (`client.rs:222-235`) est **étendu** au troisième candidat — il n'est pas remplacé :
   enveloppe seule, enveloppe + env (la plus grande gagne), enveloppe inférieure au
   défaut (ne rabaisse rien), `None` (= comportement d'aujourd'hui, déjà couvert).
2. **`crates/mika-cli/Cargo.toml`** — `url.workspace = true` (déjà déclarée au
   workspace à `url = "2"`, et déjà consommée par `mika-common`, `mika-agent`,
   `mika-gateway` ; `reqwest` et `serde_json` sont déjà là).
   **`crates/mika-cli/src/lib.rs`** — `pub mod budget_floor;` (quatrième module de la
   surface lib, après `ask_message`, `remote_ask`, `supervision`).
   **`crates/mika-cli/src/budget_floor.rs`** — `local_a2a_agent`,
   `envelope_from_attestation`, `attested_envelope_secs` (le seul fetch, qui prend le
   token en paramètre et ne lit pas l'env), les événements. Tests **purs** du prédicat
   (loopback vs hôte distant, chemin à deux segments, chemin sans `/a2a/`, schéma non
   http, IPv6 entre crochets, `/` final) et du parseur (champ présent, absent, nul,
   type faux, corps d'erreur 404).
3. **`crates/mika-cli/src/remote_ask.rs`** — le câblage, inséré avant
   `build_send_params` et sans découper la fonction (contrainte mika#1951, § mécanisme
   pièce 4). Le changement de comportement runtime atterrit ici, et seulement ici.
4. **`crates/mika-cli/src/budget_floor.rs`** (tests) —
   `mika2759_le_plancher_a_un_seul_site` + son contrôle de non-vacuité.
5. **`crates/mika-agent/src/well_known_agents.rs`** — retrait de
   `mika2309_client_default_is_below_the_arch_envelope` (`:3435`) et atterrissage de son
   successeur `mika2759_le_plancher_couvre_lenveloppe_de_mika_arch` (voir
   § Fire-Disposition, point 4).
6. **`scripts/check-a2a-timeout-literals.sh`** — `crates/mika-cli/src/budget_floor.rs`
   entre au `PERIMETER` (fait 4), avec le commentaire d'appartenance des trois autres
   entrées (`# a2a send-floor attestation`). Sans cette ligne, la conformité de
   `ATTESTATION_TIMEOUT_SECS` est vraie et **non vérifiée** ; avec elle, V3 l'atteste.
   `scripts/a2a-timeout-allowlist.txt` n'est **pas** touchée : ses 21 lignes sont
   toutes des commentaires, elle porte **zéro entrée**, et c'est l'état voulu — « vide »
   ici veut dire zéro entrée, pas fichier vide.
7. **Docs** (hors compte) — un paragraphe dans `crates/mika-a2a/CLAUDE.md` (le
   plancher per-agent est tenu à l'envoi, par l'attestation, sur le chemin loopback
   seulement ; `tests/a2a_budget_transitivity_2309.rs` garde son périmètre déclaré et
   ne change pas), dans `crates/mika-cli/CLAUDE.md` (le module, le prédicat, les
   arbres) et la mention du périmètre élargi là où il est déjà décrit
   (`crates/mika-a2a/CLAUDE.md` § policy gardée).

---

## Verification Contract

| # | vérification | commande |
|---|---|---|
| V1 | suite complète verte | `cargo test` |
| V2 | le garde mika#2457 reste vert **et son allowlist reste vide** | `cargo test -p mika-cli mika2457` |
| V3 | aucun littéral à un site bornant sur le chemin a2a, **périmètre élargi au nouveau module**, allowlist toujours à zéro entrée des deux côtés | `scripts/check-a2a-timeout-literals.sh` |
| V3b | le lint a2a n'est pas devenu vacueux en changeant de périmètre | `scripts/test-check-a2a-timeout-literals.sh` |
| V4 | la transitivité mika#2309 est intacte | `cargo test -p mika-agent --test a2a_budget_transitivity_2309` |
| V5 | le plancher couvre réellement l'enveloppe de mika-arch | `cargo test -p mika-agent mika2759` |
| V6 | le scan « un seul site » voit quelque chose et refuse un second site | `cargo test -p mika-cli mika2759` |
| V7 | lint + format | `cargo clippy --all-targets -- -D warnings` && `cargo fmt --check` |
| V8 | pipeline | `scripts/verify-pipeline.sh origin/main` |
| V9 | le câblage n'a pas découpé `send_message_to_agent` (garde mika#1951) | `cargo test -p mika-cli mika1951` |

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
changé. **Aucun prérequis d'authentification nouveau non plus** : la requête d'attestation
porte le `MIKA_INTERNAL_TOKEN` que l'envoi lit déjà, et l'endpoint accepte ce token
(dashboard routes, mika#2457). Un shell qui pouvait envoyer peut attester ; un shell
sans token recevait déjà un 401 sur l'envoi et en reçoit un sur l'attestation, qui est
fail-open.

Un spirit qui n'expose pas l'endpoint (binaire antérieur à mika#2457) sert la
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
  parseur pur, son unique fetch (token reçu en paramètre, pas lu dans l'env), et ses
  quatre arbres (`not_local`, `unreachable`, `status_<code>`, `malformed`) tous testés.
- `crates/mika-cli/src/budget_floor.rs` est **dans** le `PERIMETER` de
  `scripts/check-a2a-timeout-literals.sh`, et le harnais anti-vacuité de ce lint est
  vert.
- `send_message_to_agent` pose le plancher ; aucune de ses signatures ni aucun de ses
  ~12 appelants n'a changé, et le garde mika#1951 qui lit son corps est vert.
- `mika2309_client_default_is_below_the_arch_envelope` est **retiré** avec sa cause,
  et son successeur mika#2759 est vert.
- V1 à V9 passent. Les deux allowlists existantes (`scripts/a2a-timeout-allowlist.txt`,
  `ALLOWED` de `mika2457_…`) portent **encore zéro entrée** : aucune exception demandée,
  et le seul mouvement côté détecteurs est un **élargissement** de périmètre.
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
3. **AC3 — l'absence d'attestation rend exactement la valeur d'avant.** Les arbres de
   dégradation (`not_local`, `unreachable`, `status_<code>` — dont 404 et 401 —,
   `malformed`) résolvent tous vers `resolve_send_timeout()`, et chacun a son test.
4. **AC4 — une cible non loopback ne déclenche aucun appel réseau.** Le prédicat
   refuse avant tout `GET` ; un test l'atteste sur une URL gateway et sur une URL
   `--remote`.
5. **AC5 — le CLI ne résout toujours aucun budget localement.**
   `mika2457_the_cli_resolves_no_budget_locally` est vert avec `ALLOWED` **vide** ;
   aucune de ses sept aiguilles n'apparaît en production dans le nouveau module.
6. **AC6 — aucun budget littéral à un site bornant, et le nouveau module est DANS le
   périmètre.** `crates/mika-cli/src/budget_floor.rs` figure au `PERIMETER` de
   `scripts/check-a2a-timeout-literals.sh`, celui-ci sort 0,
   `scripts/test-check-a2a-timeout-literals.sh` (anti-vacuité) sort 0, et
   `scripts/a2a-timeout-allowlist.txt` porte toujours **zéro entrée** (comparée dans
   les deux sens). Un plancher a2a borné par un fichier hors périmètre ne satisfait
   pas ce critère, même si sa constante est conforme.
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
11. **AC11 — le câblage n'a pas découpé la fonction d'envoi.**
    `mika1951_both_ask_doors_post_the_isolation_key_through_one_site` est vert :
    `build_send_params` a toujours exactement une définition et un appel de
    production, et cet appel vit toujours dans la tranche de source qui commence à
    `pub async fn send_message_to_agent(`.

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
5. **Élargissement du périmètre d'un détecteur existant, et c'est la correction du
   re-grooming.** `a2a-timeout-literal-lint` a deux règles (A1 : aucun littéral à un
   site bornant ; A2 : tout `const … : Duration >= 60 s` est un `DEFAULT_*` adossé à un
   `MIKA_*`) et un **périmètre fermé de quatre entrées** qui ne contient pas
   `budget_floor.rs` (fait 4). `ATTESTATION_TIMEOUT_SECS` y est conforme deux fois —
   `u64` nommé, donc A1 l'admet (`from_secs(IDENT)`) et A2 ne le concerne pas (ce n'est
   pas un `const … : Duration`) — mais tant que le fichier est hors périmètre, cette
   conformité n'est **pas mesurée**. Un garde vert parce qu'il ne regarde pas est
   précisément ce que celui-ci refuse ailleurs. Le périmètre s'élargit donc d'une
   entrée (livrable 6), ce qui **arme** le garde sur le nouveau module au lieu de lui
   accorder une exemption tacite. Ce n'est pas une exception : c'est l'inverse d'une
   exception, et V3b atteste que le garde élargi n'est pas devenu vacueux.
6. **Détecteurs existants, aucune exception demandée** —
   `mika2457_the_cli_resolves_no_budget_locally` reste vert parce qu'aucune de ses sept
   aiguilles n'est écrite : le module lit un champ JSON attesté, il ne résout rien.
   `mika1951_both_ask_doors_post_the_isolation_key_through_one_site` reste vert parce
   que le câblage s'insère dans le corps de `send_message_to_agent` sans l'extraire ni
   la découper (§ mécanisme pièce 4, V9).
   **Si l'une des trois rougit, l'implémentation s'arrête et remonte (option c)** — ni
   l'allowlist `mika2457_…`, ni `scripts/a2a-timeout-allowlist.txt`, ni la contrainte
   mika#1951 ne s'ouvrent sous l'autorité de ce vol.

---

## Taille estimée

Re-mesuré au re-grooming du 2026-10-17 : deux postes bougent. Les tests du `max`
passent de 45 à 15 (le test existe, il est étendu — fait 6) ; le périmètre du lint
ajoute 2 lignes (livrable 6).

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-a2a/src/client.rs` (cœur pur publié + `resolve_send_timeout_with_floor`) | 35 |
| `crates/mika-a2a/src/client.rs` (extension du test du `max` au 3ᵉ candidat) | 15 |
| `crates/mika-cli/src/budget_floor.rs` (prédicat, parseur, fetch, événements) | 95 |
| `crates/mika-cli/src/budget_floor.rs` (tests purs : prédicat, parseur, arbres d'échec) | 85 |
| `crates/mika-cli/src/budget_floor.rs` (scan « un seul site » + contrôle de non-vacuité) | 50 |
| `crates/mika-cli/src/remote_ask.rs` (câblage dans `send_message_to_agent`) | 15 |
| `crates/mika-cli/src/lib.rs` + `Cargo.toml` (déclarations) | 2 |
| `crates/mika-agent/src/well_known_agents.rs` (retrait mika#2309 + successeur) | 35 |
| `scripts/check-a2a-timeout-literals.sh` (une entrée de `PERIMETER` + son commentaire) | 2 |

Total estimé : 334 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 334 < 429 et 334 < 1000 : les deux bornes
passent, la plus serrée avec ~95 lignes de marge. Le périmètre est fermé par
construction — **un** module neuf, **six** fichiers touchés (`client.rs`,
`remote_ask.rs`, `lib.rs`, `Cargo.toml`, `well_known_agents.rs`,
`check-a2a-timeout-literals.sh`), aucune signature
publique modifiée, donc **zéro** appelant à retoucher (c'est le gain principal de la
dérivation depuis l'URL : les ~12 appelants de `send_message_to_agent`, dont deux
fichiers de tests d'intégration, restent intacts). Aucune migration de schéma, aucun
renommage, aucune réécriture de test existant — le test du `max` est **étendu** et
celui de mika#2309 est **retiré** avec sa cause (point 4 de la Fire-Disposition), ce
qui est nommé, pas subi. Les deux postes qui auraient fait déborder ont été écartés
délibérément et pour des raisons de justesse, pas de budget : un serveur HTTP stub
(~60 lignes de harnais) est inutile dès lors que le parseur est séparé du fetch et
testé purement, et un test de bout en bout de `send_message_to_agent` est
**inobservable** — la fonction rend un `Task`, jamais le client, donc la seule couture
assertable est celle que les points 1 à 3 de la Fire-Disposition couvrent déjà.
