# Plan : le plafond HTTP par appel coupe des générations vivantes — et le réglage censé l'avoir corrigé n'est pas observable (mika#2293)

**Ticket :** mika issue#2293 — `MIKA_LLM_HTTP_TIMEOUT_SECS` (120 s) coupe une génération LLM en cours de streaming
**Labels :** `bug`, `p1-important`, `agent-core`
**Type :** issue (bug de substrat — budget de temps)
**Palier de priorité :** Tier 2 — *dégrade la boucle sans la casser*. Un tour coupé échoue, il ne corrompt rien ; mais le grooming moteur étant obligatoire à chaque dispatch depuis le 14/09, une passe arch qui échoue bloque le dispatch.
**Fichiers principaux :** `crates/mika-common/src/llm/budget.rs`, `crates/mika-common/src/llm/mod.rs`, `crates/mika-common/src/config.rs`, `crates/mika-agent/src/server/mod.rs`, `crates/mika-agent/src/well_known_agents.rs`

---

## Problème

Le ticket établit un mécanisme réel, et il faut commencer par le confirmer sans réserve : `reqwest::ClientBuilder::timeout` borne **la requête entière, lecture du corps comprise**. Quand une génération dépasse le plafond alors qu'elle streame encore, elle est tuée ; l'échec porte le message `failed to read response body: … operation timed out` et n'a rien d'un problème de fournisseur. Le ticket a raison de refuser cette lecture-là, et la mesure qu'il apporte — même signature à 120 s pile sur deux fournisseurs distincts (kimi/OpenRouter et z.ai/glm-5.3) — est exactement l'argument qui la réfute : deux fournisseurs ne tombent pas en panne à la même seconde.

Le remède proposé — `MIKA_LLM_HTTP_TIMEOUT_SECS = 300`, `MIKA_AGENT_TOTAL_TIMEOUT_SECS = 600` — est en revanche calibré sur une charge qui n'existe plus, emprunte une voie qui écrase le réglage per-agent censé résoudre ce problème, ne donne aucun retry supplémentaire, et comporte un mode de panne totale si l'une des deux variables est posée sans l'autre. Les quatre points sont mesurés ci-dessous.

Mais le défaut central est ailleurs, et il est plus grave que le réglage : **mika#2189 a déjà donné à mika-arch le couple 240/900, cinq jours avant la mesure du ticket — et la mesure voit malgré tout des coupures à 120 s pile.** Quelque chose annule ce réglage. Aucune ligne de journal ne permet de dire quoi, parce que **rien dans le code n'expose le couple (plafond, enveloppe) effectivement en vigueur pour un agent donné**. C'est ce trou-là qui a laissé un réglage livré le 06/09 échouer en silence jusqu'au 11/09, et c'est lui qui, inchangé, ferait échouer en silence le réglage suivant.

## Mesures — exécutées le 2026-09-15 dans le worktree de grooming

Le sandbox du pilote n'a pas accès au `~/.mika` de production : **aucune mesure runtime n'a pu être prise**. Tout ce qui suit est établi sur le code et l'historique git du dépôt, et les hypothèses runtime sont nommées comme telles.

- **M1 — le réglage d'arch existe depuis le 06/09.** `crates/mika-agent/src/well_known_agents.rs:1415-1416` porte `llm_http_timeout_secs = 240` / `agent_total_timeout_secs = 900`, introduit par `51ac562d` (2026-09-06, mika#2189). La mesure du ticket date du 11/09. Le code était en place, et `reconcile_well_known_config` (`well_known_agents.rs:689`) réécrit le `config.toml` d'un agent existant dès qu'il diffère du spec — le réglage n'est donc pas bloqué par la non-réécriture qui affecte `identity.toml`.
- **M2 — l'environnement bat le `config.toml` per-agent.** `Settings::load_for_agent` (`config.rs:2254-2271`) empile : global `config.toml` → `config.toml` per-agent → **env `MIKA_*`** → `.env` per-agent. Une variable `MIKA_LLM_HTTP_TIMEOUT_SECS` posée pour le service **écrase** le 240 de mika-arch. C'est l'explication mécaniquement suffisante de M1, et c'est aussi l'effet de bord garanti du remède proposé par le ticket.
- **M3 — le couple effectif n'est journalisé nulle part.** Aucun `event = "llm_budget…"` ni équivalent dans `crates/`. On ne peut répondre à « sous quel plafond ce tour a-t-il tourné ? » que par déduction. La validation elle-même est cold-path, à la construction du provider (`create_provider_with_budget`, `llm/mod.rs:550`), et le module l'écrit noir sur blanc : *« a `mika` that starts is not proof that its budgets are valid. The first LLM call is. »*
- **M4 — 300/600 ne donne pas un seul retry de plus.** `max_attempts = floor(envelope / cap)`, borné à `[1, MAX_ATTEMPTS_HARD_CAP=4]` (`budget.rs:270`, `openai.rs:150`). Aujourd'hui `floor(300/120) = 2` ; proposé `floor(600/300) = 2`. **Identique.** Le ticket demande « envisager >1 retry sur transport » : cela exige `envelope ≥ 3 × cap`, que 300/600 ne satisfait pas. Ce que 300/600 change, en revanche, c'est le coût d'un tour qui échoue : de 240 s à **600 s**, soit ×2,5.
- **M5 — poser le plafond sans l'enveloppe met la flotte par terre.** `MIKA_DEV_CONFIG` et `MIKA_QA_CONFIG` (`well_known_agents.rs:167`, `:180`) ne portent **aucune** clé de budget : leur enveloppe est le défaut 300. Poser `MIKA_LLM_HTTP_TIMEOUT_SECS=300` seul donne `cap=300 >= envelope=300` → `CapNotContained` → `create_provider_with_budget` échoue → **plus aucun appel LLM** pour mika-dev et mika-qa. La panne est différée au premier appel, donc elle ne ressemble pas à une erreur de configuration mais à une panne d'agent.
- **M6 — la moitié « client » de l'incident du 11/09 est déjà corrigée.** `A2aClient::DEFAULT_TIMEOUT` vaut 600 s depuis mika#2297 (`289ea6fe`, 2026-09-14), et `resolve_send_timeout` (`a2a/src/client.rs:35`) plancher le client sur `MIKA_AGENT_TOTAL_TIMEOUT_SECS` — le commentaire cite explicitement l'incident : *« the arch, on a bloated brief, ran past 300 s while the engine's own 600 s budget had not yet expired »*. Deux conséquences : le client n'abandonne plus une génération vivante, et **le moteur tournait déjà sous une enveloppe de 600 s le 11/09**, ce qui rend l'enveloppe à 600 demandée par le ticket déjà acquise par voie d'environnement.
- **M7 — la cause de la charge a été corrigée aujourd'hui.** mika#2295 (`5a7a50fb`, 2026-09-15) borne la fenêtre d'historique de mika-arch, dont le brief avait doublé (35 829 → 71 007 tokens médians) avec une latence passée de 15-30 s à 50-77 s et **14 % des tours ≥ 120 s**. Son plan qualifie mika#2293 de « **pansement** » et le laisse explicitement en place. La mesure du 11/09 qui fonde le présent ticket a donc été prise **sous brief doublé**.
- **M8 — le read-timeout est déjà classé retryable.** `response.text()` échoue en `LlmError::Transport` (`openai.rs:293`), seuil de retry transport `0,5 × cap` contre `1,0 × cap` pour le reste (`budget.rs:241-258`). Rien à réparer de ce côté : la classification est correcte, c'est le nombre de tentatives que l'enveloppe borne.

## Rectification apportée à la direction du ticket

Le ticket demande de monter deux nombres. Ce plan ne refuse pas le réglage — il refuse **l'ordre** dans lequel le ticket le pose, pour trois raisons mesurées.

D'abord, M7 : recalibrer un plafond sur une latence produite par un brief doublé dont la cause vient d'être corrigée, c'est figer dans le substrat la trace d'un défaut déjà réparé. La bonne valeur du plafond après mika#2295 n'est pas connue ; elle est mesurable, et elle ne l'a pas encore été.

Ensuite, M1+M2+M3 : le réglage per-agent livré le 06/09 n'a pas produit son effet, et personne ne peut dire pourquoi parce que le couple en vigueur n'est visible nulle part. Poser un nouveau réglage sans fermer ce trou revient à rejouer exactement la même partie. **Un réglage qu'on ne peut pas observer n'est pas un réglage, c'est un espoir.**

Enfin M5 : la voie que le ticket emprunte — deux variables d'environnement fleet-wide — écrase les réglages per-agent (dont le 240/900 d'arch) et, si l'une des deux manque, coupe les appels LLM de la flotte entière avec une panne différée. La voie sûre existe déjà et c'est celle que mika#2189 a choisie : le `config.toml` per-agent, qui n'écrase personne.

Le plan livre donc, dans cet ordre : **l'observabilité du budget effectif**, **une garde contre la demi-configuration**, puis **le recalibrage**, conditionné à une mesure post-#2295.

## Conception

### Brique 0 — le couple effectif devient observable (préalable, et seul juge)

À la construction du provider, émettre un événement INFO `llm_budget_resolved` portant `agent_id`, `http_timeout_secs`, `agent_total_timeout_secs`, `max_attempts`, `worst_case_failure_secs`, et — c'est le champ qui tranche M1 — **la provenance de chaque valeur** : `default`, `agent_config`, `process_env` ou `agent_dotenv`.

La provenance ne se devine pas après coup : `Settings` a déjà fusionné les sources quand on lit le champ. Elle doit être déterminée là où la cascade est encore visible. Deux options, à trancher à l'implémentation : soit `load_for_agent` conserve la source retenue pour ces deux clés, soit la résolution est refaite explicitement pour ces deux clés seulement (lecture du `config.toml` per-agent et de l'env, comparaison avec la valeur fusionnée). La seconde est plus étroite et n'engage pas config-rs ; elle est à préférer sauf obstacle.

Ungated : ce champ ne doit pas dépendre de `MIKA_STORE_LLM_CALLS`. C'est un événement de configuration, pas de télémétrie d'appel, et il doit rester lisible quand on a précisément coupé la télémétrie pour réduire le bruit.

**Pourquoi cette brique est bloquante et non cosmétique :** elle est le seul moyen de distinguer les deux hypothèses de M1 — « l'env écrase le 240 » contre « le config.toml d'arch ne porte pas la clé ». Ces deux mondes appellent des corrections différentes, et une seule ligne de journal les sépare.

### Brique 1 — la demi-configuration échoue bruyamment, au démarrage

M5 décrit une panne totale, différée, et qui ne ressemble pas à sa cause. La corriger n'est pas un durcissement gratuit : c'est exactement la classe de défaut que le tier-guard a déjà fermée pour le tier famille (`server::tier_guard::assert_family_tier_env_consistency`, appelé depuis `server/mod.rs:721`) — un précédent à suivre plutôt qu'à réinventer.

Ajouter, dans `run_server`, une garde qui résout le couple de **chaque agent bien connu** et refuse le démarrage quand l'un d'eux est invalide, en nommant l'agent, les deux valeurs et leur provenance. Le message doit dire quelle variable poser, pas seulement laquelle est fautive.

Refuser le démarrage plutôt qu'avertir : une flotte qui démarre pour découvrir au premier appel qu'aucun agent ne peut parler est un pire état qu'un refus immédiat avec un message actionnable. C'est la doctrine que `create_provider_with_budget` énonce déjà sans pouvoir l'appliquer — elle n'a que le cold-path, pas le boot.

**Ce que la garde ne fait pas :** elle ne corrige aucune valeur et n'en invente aucune. Un couple invalide est une erreur d'opérateur ; la réparer silencieusement reproduirait le défaut que ce plan instruit.

### Brique 2 — le recalibrage, conditionné à une mesure post-#2295

Une fois la brique 0 déployée et mika#2295 en production, relever sur mika-arch et mika-qa la distribution des latences d'appel **réussi** (p50, p99, max) et le nombre d'appels par passe. Le plafond se dérive de cette distribution, il ne se choisit pas : il doit couvrir le max observé avec marge, et l'enveloppe doit valoir au moins `attentes × plafond` pour le nombre de tentatives qu'on veut autoriser.

Trois contraintes encadrent le choix et doivent être posées explicitement plutôt que subies :

1. **Nombre de tentatives.** `max_attempts = floor(envelope/cap)`. Deux tentatives exigent `envelope ≥ 2 × cap` ; trois exigent `≥ 3 × cap`. Si la demande « >1 retry » du ticket est retenue, elle se paie en enveloppe, et l'enveloppe se paie en latence d'un tour qui échoue.
2. **Coût d'un échec.** `worst_case_failure_secs = max_attempts × cap`. Il croît avec le plafond. Un plafond généreux n'est pas gratuit : il rend les pannes lentes.
3. **Budget des appelants.** Le relais `canUseTool` de claude-pilot est à **120 s** (`.claude/claude-pilot.json:4`) et couvre l'attente *et* le tour. Un plafond per-call au-dessus de 120 s ne rend pas ces tours-là plus utiles — l'appelant a déjà abandonné. Le client A2A, lui, est planché sur l'enveloppe depuis mika#2297 (M6) et suit.

**Voie d'application : le `config.toml` per-agent**, via le spec de `well_known_agents.rs`, comme mika#2189. Pas de variable d'environnement fleet-wide : M2 montre qu'elle écraserait les réglages per-agent, à commencer par celui qu'on vient de mesurer.

### Ce que ce plan ne fait pas

Il ne change pas le défaut de flotte `120/300` tant que la mesure post-#2295 ne l'a pas justifié — et si elle le justifie, le changement se fera par agent mesuré, pas par défaut global. Il ne touche pas à la classification des erreurs de transport (M8 : elle est correcte). Il ne touche pas au littéral `120s` de `claude.rs:382`, inconsistance réelle mais sur un rail — Anthropic — qu'aucune des occurrences mesurées n'emprunte ; mika#2189 l'a déjà nommée hors périmètre et ce plan ne la ramasse pas. Il ne cherche pas la cause des lenteurs fournisseur : elle est hors d'atteinte du code, et ce plan borne un budget, il n'accélère rien.

## Fire-Disposition

- **AC1 — provenance mesurée (implémentation, préalable bloquant).** Tir sur : `llm_budget_resolved` pour `mika-arch` après déploiement de la brique 0. Disposition : **halt-and-surface**. Si la provenance du plafond est `agent_config` avec la valeur 240, alors M2 est infirmé et la cause des 120 s est ailleurs : écrire la mesure dans mika#2293 et re-groomer. **Ne pas poser la brique 2 « puisqu'elle est écrite ».**
- **AC2 — garde de démarrage (CI).** Tir sur : le diff / la CI. Disposition : **gate CI bloquant**. Un test assied qu'un couple valide démarre et qu'un couple invalide est refusé en nommant l'agent — les deux contrôles dans le même test, faute de quoi la sonde ne distingue pas une garde qui marche d'une garde qui refuse tout.
- **AC3 — non-régression du défaut (CI).** Tir sur : le diff / la CI. Disposition : **gate CI bloquant**. Le couple 120/300 reste accepté, `max_attempts` y vaut toujours 2, et les fractions de seuil restent inchangées au plafond par défaut.
- **AC4 — recalibrage conditionné (opérateur).** Tir sur : distribution des latences d'appel réussi sur ≥ 24 h et ≥ 100 appels mika-arch **après** déploiement de mika#2295. Disposition : **halt-and-surface**. Si le p99 est retombé sous 120 s, **le plafond ne bouge pas** : la cause était la charge, elle est corrigée, et élargir le budget ne ferait qu'installer une marge qu'aucune mesure ne soutient.
- **AC5 — sonde de signature (opérateur, signal d'arrêt).** Tir sur : la colonne latence des appels en échec après recalibrage. Disposition : **halt-and-surface**. Une signature nette au nouveau plafond, ou à son double, est un **arrêt** et non une invitation à remonter : deux plafonds franchis d'affilée disent que le modèle de la panne est faux. C'est la règle que mika#2189 a écrite pour lui-même et qui s'applique ici sans modification.
- **Non-action par défaut.** Hors de ces signaux, rien ne se déclenche : aucun ajustement automatique de plafond, aucune variable d'environnement fleet-wide, aucun réglage d'un agent que la mesure n'a pas couvert.

## Definition of Done

- La brique 0 est déployée et a produit un `llm_budget_resolved` exploitable pour les quatre agents bien connus, **avant** toute modification de valeur.
- La brique 1 refuse le démarrage sur un couple invalide, en nommant l'agent, les deux valeurs et leur provenance.
- `cargo build`, `cargo clippy`, `cargo fmt --check` et `cargo test -p mika-common llm::budget` passent.
- **Geste d'opérateur requis, et non automatisable :** si M2 se confirme, une variable `MIKA_LLM_HTTP_TIMEOUT_SECS` ou `MIKA_AGENT_TOTAL_TIMEOUT_SECS` posée dans l'`EnvironmentFile` du service continuera d'écraser tout `config.toml` per-agent, y compris celui que ce plan recalibrerait. Il faut **retirer ces variables de l'environnement du service** pour que les réglages per-agent reprennent la main, puis redémarrer mika-spirit. Sans ce geste, AC4 mesurera un agent qui n'a jamais reçu le réglage et donnera l'apparence d'un fix qui n'a pas pris.
- La sonde AC4 est relevée après ce geste et après le déploiement de mika#2295, pas avant.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les critères ci-dessous sont dérivés de son exigence (« calibrer sur la charge arch/qa réelle ») et de la conception ci-dessus.

1. **AC1 — le budget effectif est observable.** Un événement INFO `llm_budget_resolved` est émis à la construction de chaque provider, inconditionnellement, portant `agent_id`, `http_timeout_secs`, `agent_total_timeout_secs`, `max_attempts`, `worst_case_failure_secs`, et la **provenance** de chacune des deux valeurs parmi `default` / `agent_config` / `process_env` / `agent_dotenv`.
2. **AC2 — la demi-configuration est refusée au démarrage.** Un couple invalide sur un agent bien connu empêche `run_server` de démarrer, avec un message nommant l'agent, les deux valeurs, leur provenance et la variable à corriger. Assis par un test portant les deux contrôles.
3. **AC3 — le défaut de flotte est inchangé par ce PR.** `120/300` reste le couple par défaut, `max_attempts` y vaut 2, et les trois seuils dérivés restent identiques au plafond par défaut. Assis par les tests existants de `budget.rs`, qui doivent rester verts sans modification.
4. **AC4 — la recalibration est fondée sur une mesure post-#2295.** Toute modification de plafond ou d'enveloppe est accompagnée, dans la description du PR, de la distribution mesurée (p50/p99/max des appels réussis, appels par passe) relevée après déploiement de mika#2295, et la valeur retenue en est dérivée explicitement.
5. **AC5 — le réglage passe par le `config.toml` per-agent.** Aucun ajout de `MIKA_LLM_HTTP_TIMEOUT_SECS` ni `MIKA_AGENT_TOTAL_TIMEOUT_SECS` à l'environnement du service n'est prescrit par ce travail.
6. **AC6 — le nombre de tentatives est une décision, pas un effet de bord.** Si la demande « >1 retry » du ticket est retenue, le couple choisi satisfait `envelope ≥ 3 × cap` et le PR le dit ; s'il ne l'est pas, le PR dit pourquoi le nombre de tentatives reste à 2.

## Rattachement aux critères d'acceptation

| AC | Brique | Vérification |
|---|---|---|
| AC1 | Brique 0 | Test unitaire sur l'émission ; `grep llm_budget_resolved` post-déploiement |
| AC2 | Brique 1 | Test à deux contrôles — gate CI (AC2 Fire-Disposition) |
| AC3 | — (non-régression) | Tests existants `budget.rs` inchangés — gate CI (AC3 Fire-Disposition) |
| AC4 | Brique 2 | Sonde post-déploiement (AC4 Fire-Disposition) |
| AC5 | Brique 2 | Revue du diff : `well_known_agents.rs`, pas d'`EnvironmentFile` |
| AC6 | Brique 2 | Arithmétique `floor(envelope/cap)` explicitée dans le PR |

## Hors portée (repris du ticket, sans extension)

- La cause des lenteurs fournisseur — hors d'atteinte du code ; ce plan borne un budget, il n'accélère rien.
- Le littéral `120s` de `crates/mika-common/src/llm/claude.rs:382` — inconsistance réelle, déjà nommée hors périmètre par mika#2189, sur un rail qu'aucune occurrence mesurée n'emprunte.
- La taille du prompt système de mika-arch (54 ko → 59,8 ko le 01/09) — hors périmètre de mika#2189, et distincte du doublement de brief que mika#2295 corrige.
- La classification des erreurs de transport (M8) — correcte en l'état.
- Le budget du relais `canUseTool` de claude-pilot (120 s) — une contrainte d'entrée pour le choix du plafond, pas une cible de ce travail.
- mika#2280 (classe de timeout) et mika#2289 (revue QA) — voisinage, pas périmètre.

## Vérification

- `cargo test -p mika-common llm::budget` et `cargo test -p mika-common config` — AC2, AC3.
- `cargo build && cargo clippy && cargo fmt --check`.
- Post-déploiement, dans `$MIKA_SPIRIT_LOG_FILE` :
  - `grep llm_budget_resolved | jq '{agent_id, http_timeout_secs, http_source, agent_total_timeout_secs, total_source, max_attempts}'` — AC1, et **la réponse à M1** : la provenance dit si l'env écrase le `config.toml` d'arch.
  - Attendu si M2 se confirme : `http_source: "process_env"` sur mika-arch avec une valeur ≠ 240.
- SQL de recoupement, avant toute modification de valeur, pour établir la ligne de base post-#2295 :
  `SELECT model, latency_ms, error FROM llm_calls WHERE agent_id = 'mika-arch' AND created_at > '<date de déploiement de #2295>' ORDER BY created_at DESC;`
  Relever p50/p99/max des appels réussis, et la distribution des latences d'échec. **Capturer cette base avant de déployer le recalibrage** : `prune_old_llm_calls` purge les anciennes lignes, et une base perdue ne se reconstitue pas.

## Conditions d'arrêt

- **AC1 infirme M2** (la provenance du plafond d'arch est `agent_config` à 240) → halte : la cause des coupures à 120 s est ailleurs, écrire la mesure dans mika#2293 et re-groomer sur la cause réelle.
- **AC4 montre un p99 retombé sous 120 s après mika#2295** → halte, et c'est une **bonne** issue : la charge était la cause, elle est corrigée, le pansement n'est plus nécessaire. Ne pas élargir un budget pour installer une marge qu'aucune mesure ne soutient.
- **Une signature nette apparaît au nouveau plafond ou à son double après recalibrage** → halte. Ne pas remonter : deux plafonds franchis d'affilée disent que le modèle de la panne est faux, et un troisième réglage effacerait la trace qui permettrait de le corriger.
- **La provenance ne peut pas être déterminée sans modifier la cascade de config-rs** → halte et reprise de conception. Une provenance approximative est pire que pas de provenance : elle donnerait une réponse fausse à la seule question que la brique 0 existe pour trancher.

## Voisinage

- mika#2189 — la paire `(plafond, enveloppe)` et l'invariant de containment ; c'est ce ticket qui a posé le 240/900 d'arch dont M1 constate l'inefficacité.
- mika#2295 — la correction de fond de la charge d'arch, mergée le 15/09 ; son plan qualifie mika#2293 de « pansement » et le laisse en place.
- mika#2297 — le plancher `client ≥ total` du client A2A, mergé le 14/09 ; il a déjà fermé la moitié « client » de l'incident du 11/09.
- mika#2218 — l'inversion de cascade `.env` per-agent > process env, qui achève la hiérarchie que M2 décrit.
- mika#1660 — le plancher `MIN_HTTP_TIMEOUT_SECS` et la panique cold-path, dont la brique 1 est le pendant au démarrage.
- mika#2280, mika#2289 — voisinage cité par le ticket.
