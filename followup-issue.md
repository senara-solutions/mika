## Objet

Le plancher `client >= total` posé par mika#2297 est lu sur **l'environnement du
process** ; les enveloppes per-agent posées par mika#2189 vivent dans le
`config.toml` **de l'agent**. Les deux ne se rencontrent jamais, donc le plancher
n'est pas tenu pour un agent dont l'enveloppe est per-agent.

## La mesure (18/09, `e1342dfa`)

- `crates/mika-a2a/src/client.rs:51` — `resolve_timeout_secs` lit
  `MIKA_A2A_TIMEOUT_SECS` et `MIKA_AGENT_TOTAL_TIMEOUT_SECS` dans l'env du
  process, et **jamais** le `config.toml` per-agent.
- `crates/mika-agent/src/well_known_agents.rs` (`MIKA_ARCH_CONFIG`) — mika-arch
  porte `agent_total_timeout_secs = 900` (mika#2189).

Sur un appel vers mika-arch, le client résout donc **600 s** face à une enveloppe
de **900 s**. Le plancher n'est pas tenu : le client abandonne une génération que
le moteur a encore le droit de finir — exactement le sinistre du 11/09 qui a
motivé le passage de 300 à 600 (mika#2297).

## Ce qui est déjà en place, et ce qui ne l'est pas

mika#2309 a livré la Garde B (`crates/mika-agent/tests/a2a_budget_transitivity_2309.rs`),
qui affirme `client >= total > http` sur la cascade que le client sait réellement
lire (défauts + env du process). Elle ne prétend pas couvrir le per-agent, et le
dit.

Un **contrôle positif auto-nettoyant** épingle l'écart plutôt que de le taire
(`well_known_agents.rs`, test `mika2309_client_default_is_below_the_arch_envelope`) :
il lit les deux valeurs réelles et **rougit le jour où l'écart disparaît**, ce qui
force à retirer l'exception au lieu de la laisser survivre à sa cause.

## La question à trancher, et pourquoi elle n'était pas tranchable dans mika#2309

Corriger revient à apprendre au client a2a à lire la cascade per-agent. C'est un
**changement de comportement runtime**, que mika#2309 s'interdisait en tête
(« substrat borné, pas de changement de comportement runtime »). Et la forme même
est la question : **le client ne connaît pas l'agent visé au moment où il résout
son budget** — `A2aClient::new(base_url, token)` construit son `reqwest::Client`
avant que l'URL ne soit décomposée en agent.

Trois directions, aucune évidente :

1. Résoudre le budget **au moment de l'envoi** plutôt qu'à la construction, en
   dérivant l'agent de l'URL. Change la sémantique de `with_timeout` et le
   partage du `reqwest::Client`.
2. Faire porter au constructeur un nom d'agent optionnel, et lire le
   `config.toml` correspondant. Fait dépendre `mika-a2a` d'une notion de home
   qu'il ignore aujourd'hui (aucune dépendance maison dans son `Cargo.toml`).
3. Aligner les valeurs — remonter `DEFAULT_TIMEOUT` au-dessus de la plus grande
   enveloppe per-agent en service. Le moins cher, le plus fragile : le plancher
   redeviendrait faux au prochain agent qui lève son enveloppe.

## Hors périmètre

Toute modification de valeur faite sans trancher ci-dessus. mika#2309 s'est
explicitement interdit d'aligner `DEFAULT_TIMEOUT` sur 900 de sa propre autorité.

Relie mika#2297, mika#2189, mika#2309.
