# Plan : le brief de mika-arch a doublé parce que sa fenêtre d'historique n'est bornée qu'en nombre (mika#2295)

**Ticket :** mika issue#2295 — `fix(prompt,agent-core): le brief d'entrée de mika-arch a doublé le 2026-09-03 (35 829 → 71 007 tokens médians) — latence 15-30 s → 50-77 s, 14 % des tours ≥ 120 s`
**Labels :** `bug`, `p2-normal`
**Type :** issue (bug de substrat — correction de fond, le budget HTTP de mika#2293 étant le pansement)
**Palier de priorité :** Tier 2 — *dégrade la boucle sans la casser*. Depuis le retrait du bypass grooming (08:11, 2026-09-14), une passe arch réussie est obligatoire à chaque dispatch ; un brief à 71 k reste lent et fragile même sous un budget élargi.
**Fichiers principaux :** `crates/mika-agent/src/prompt.rs`, `crates/mika-agent/src/agent_loop/mod.rs`, `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/async_db.rs`, `crates/mika-agent/src/well_known_agents.rs`, `crates/mika-agent/tests/eval/test_context_window_budget_2295.rs` (nouveau)

---

## Problème

Le prompt d'entrée de mika-arch est passé de **35 829 tokens médians** (avant le 30/08) à **71 007** (depuis le 03/09), sans qu'aucun de ses trois `system_prompt.md` ni `prompt.rs` n'ait bougé à cette date. Le doublement est donc dans le **contexte assemblé au tour**, et la question à instruire est : *quelle composante de ce contexte a doublé, et pourquoi le 03/09 précisément ?*

La réponse que le code impose — et que les mesures ci-dessous soutiennent sans encore la prouver — est que **la fenêtre de conversation n'est bornée qu'en nombre de messages, jamais en octets, et son périmètre est l'agent, pas la session** :

```rust
// crates/mika-agent/src/agent_loop/mod.rs:3480
let history = db.rebuild_context(scope_task_id, 20).await?;
```

```sql
-- crates/mika-agent/src/db.rs:11033 (load_recent_messages_filtered)
SELECT … FROM messages m JOIN sessions s ON m.session_id = s.id
 WHERE m.agent_id = ?1 AND m.role != 'summary' AND s.channel_type != 'team'
 ORDER BY m.created_at DESC, m.id DESC LIMIT ?2
```

Deux propriétés se composent ici, et c'est leur produit qui fait le défaut :

1. **`LIMIT 20` borne un compte, pas une taille.** Les éléments comptés sont, pour mika-arch, des plans entiers et des revues d'architecte. Une fenêtre de vingt éléments de taille non bornée n'est pas une fenêtre bornée : c'est vingt fois une quantité inconnue.
2. **Le filtre est `m.agent_id`, pas `m.session_id`.** La fenêtre de mika-arch traverse donc les sessions, et par conséquent **les tickets**. Le brief d'une revue du ticket A contient les plans et verdicts des tickets B, C, D — pour peu qu'ils soient récents.

De là, la date. Le 03/09 est, dit le ticket, le jour des **59 grooms-moteur livrés**. Tant que les grooms passaient par des spawns hors moteur (bypass depuis le 22/07), mika-arch écrivait peu de messages : ses vingt derniers étaient épars, anciens, souvent courts. Le jour où cinquante-neuf grooms traversent le moteur, ces vingt derniers messages deviennent **vingt plans et revues complets, tous récents**. Le saut est en escalier et non en pente parce que la cause est un changement de débit sur une fenêtre à compte fixe — pas une croissance de contenu.

L'ordre de grandeur concorde : les plans de septembre mesurés dans ce dépôt vont de 25 à 61 ko (≈ 6 000 à 15 000 tokens l'unité). Vingt messages dont une part de plans et de revues à ~7 ko moyens font ≈ 140 ko, soit **≈ 35 000 tokens** — exactement le delta observé.

**Conséquence qui dépasse le coût, et qu'il faut nommer :** un architecte qui revoit le plan du ticket A lit, dans le même contexte, les plans des tickets B, C et D. Ce n'est pas seulement cher, c'est une **contamination inter-tickets**. La latence est le symptôme qui a rendu le défaut visible ; elle n'est pas la totalité du défaut.

## Mesures — exécutées le 2026-09-15 dans le worktree de grooming

Ce que le worktree a permis de vérifier, et qui vaut comme fait :

| # | Mesure | Résultat |
|---|---|---|
| M1 | `prompt.rs` modifié entre le 25/08 et le 05/09 ? | **Non.** Dernier commit touchant `prompt.rs` / `skills/index.rs` / `well_known_agents.rs` : #2101, le 30/08 — soit **avant** le saut, et déjà écarté par le ticket. |
| M2 | Taille des plans (= le message *user* envoyé à l'architecte) | Septembre : 25–61 ko. Fin août : 33–44 ko. **La taille unitaire du brief n'a pas doublé.** |
| M3 | Le summary conversationnel est-il en cause ? | **Non** — `build_mika_arch_identity` pose déjà `[context.summary] inject = false` (`well_known_agents.rs`). Ce candidat est éteint avant l'enquête. |
| M4 | Périmètre de la fenêtre | `m.agent_id`, pas `m.session_id` (`db.rs:11033`). La fenêtre traverse les tickets. |
| M5 | Bornage de la fenêtre | `LIMIT 20` uniquement. Aucun plafond d'octets ni de tokens sur aucun chemin. |
| M6 | Nombre de sites d'assemblage | **Un seul** : `agent_loop/mod.rs:3480`. `rewind.rs:102` est un chemin administratif distinct, hors périmètre. |
| M7 | Instruments déjà en place | `llm_calls.system_prompt_bytes` (v38, mika#1217), `llm_calls.request_bytes` (v53, mika#2189), événements `system_prompt_assembled` (`total_bytes`, `per_skill_bytes`, `tool_count`) et `turn_usage` (`input_tokens`). |
| M8 | Précédent maison pour un budget de contexte | `truncate_to_token_budget` + `CHARS_PER_TOKEN_ESTIMATE = 4` + `ContextSummaryConfig.max_tokens` avec `Some(0)` comme sentinelle d'omission (`prompt.rs:411`, `prompt.rs:361`). |
| M9 | Précédent maison pour le point d'application | `load_gated_summary(db, &ctx.identity.context.summary, None)` (`agent_loop/mod.rs:3338`) — une config `[context.*]` lue sur l'identité et appliquée par l'appelant, sans changer la signature de la couche DB. |
| M10 | Portée disponible au site d'appel | `ctx.identity` et `params.session_id` sont tous deux en portée à la ligne 3480. |

**Ce qui n'a PAS pu être mesuré ici, et qui doit l'être en premier à l'implémentation.** Le sandbox du pilote ne monte pas `~/.mika/data/mika.db` (vérifié : le chemin n'existe pas dans le bac). L'attribution du delta aux composantes réelles — *combien d'octets d'historique, combien de message user, combien de définitions d'outils* — **n'est donc pas faite**. Elle repose sur une inférence à partir du code et des tailles de fichiers, pas sur une lecture de la base.

C'est pour cette raison que la première brique de ce plan est un **instrument d'attribution**, et non la correction : une correction posée sur une inférence non vérifiée est un pari, et un pari sur le substrat se paie plus tard, ailleurs, et plus cher. La brique 0 peut **infirmer** la thèse ; la conception dit explicitement ce qu'on fait dans ce cas.

## Rectification apportée à la direction du ticket

Le corps du ticket dirige l'investigation vers `crates/mika-cli/src/remote_ask.rs` et `crates/mika-cli/src/commands/ask.rs`. **Ces fichiers n'assemblent aucun contexte** : depuis mika#1727, `mika ask` est un client A2A mince qui poste le message à mika-spirit sur `{spirit_url}/a2a/{agent}` et rend la `Task` retournée (`render_task_parts`). Le brief — prompt système, mémoire, fenêtre de conversation, définitions d'outils — est assemblé **côté serveur**, dans `agent_loop/mod.rs` et `prompt.rs`.

La direction du ticket reste juste sur le fond (« le bloat est dans le contexte runtime, pas dans les `system_prompt` statiques ») ; seule la **localisation** est à corriger. Ce plan travaille donc mika-agent, pas mika-cli, et ne lit aucun `skills/bundled/*/system_prompt.md` conformément à la consigne du ticket.

## Conception

Trois briques, dans cet ordre, la première pouvant arrêter les deux autres.

### Brique 0 — l'instrument d'attribution (préalable, et seul juge)

Ajouter un événement INFO `context_window_assembled` émis au site unique d'assemblage (`agent_loop/mod.rs:3480`), sur `target: "mika::otel"`, dans la forme exacte de `emit_system_prompt_assembled` (mika#1217) dont il est le frère manquant :

| Champ | Sens |
|---|---|
| `message_count` | nombre de messages retenus dans la fenêtre |
| `history_bytes` | somme des octets des messages **hors** le dernier (= le message *user* du tour) |
| `user_message_bytes` | octets du dernier message, celui qu'on vient de sauver |
| `tool_defs_bytes` | somme des octets des définitions d'outils sérialisées |
| `distinct_sessions` | nombre de `session_id` distincts dans la fenêtre — **c'est le champ qui prouve ou réfute la contamination inter-tickets**, et il ne coûte rien |
| `oldest_age_secs` | âge du plus ancien message retenu |
| `truncated_messages`, `truncated_bytes` | ce que la brique 2 a retiré ; `0` tant qu'elle ne fire pas |

Émis **inconditionnellement**, comme `turn_usage` et pour la même raison (mika#1889) : un instrument qui se tait quand la persistance DB est désactivée n'est pas un instrument, c'est une option.

`distinct_sessions` mérite sa ligne : sans lui, on mesure un coût et on rate une contamination. Avec lui, une seule ligne de log répond aux deux questions.

**Ce que la brique 0 décide.** Sur au moins 20 tours de mika-arch après déploiement :

- `history_bytes` domine (≳ 60 % de `request_bytes − system_prompt_bytes`) **et** `distinct_sessions > 1` → thèse confirmée, briques 1 et 2 s'appliquent telles quelles.
- `history_bytes` est marginal → **thèse infirmée : halte.** On ne pose ni scope ni budget, on écrit ce qu'on a mesuré dans le ticket, et on re-groome sur la composante réellement dominante. Les briques 1 et 2 ne sont pas « appliquées quand même parce qu'elles sont écrites ».

### Brique 1 — le périmètre de la fenêtre devient une propriété d'identité

Nouvelle sous-section `[context.history]` dans l'identité, sur le modèle exact de `[context.summary]` (M8, M9) :

```toml
[context.history]
scope = "session"   # "agent" (défaut, comportement actuel) | "session"
max_tokens = 8000   # optionnel ; absent = pas de plafond
```

`scope = "session"` ajoute `AND m.session_id = ?` à la requête. Le filtre **doit** descendre en SQL et non s'appliquer après chargement : filtrer vingt lignes déjà lues rendrait une fenêtre amputée dès que des messages d'autres sessions se sont intercalés — c'est-à-dire précisément sous les 59 grooms/jour qui ont produit l'incident. Un filtre a posteriori serait correct au repos et faux sous charge, ce qui est la pire des deux.

Signature : `load_recent_messages_filtered(agent_id, session_id: Option<&str>, limit, exclude_internal)`, `None` reproduisant la requête actuelle **octet pour octet**. `rebuild_context` propage. Le défaut est `"agent"` : aucun agent existant ne change de comportement sans qu'on l'ait écrit dans son identité.

**mika-arch reçoit `scope = "session"`** dans `build_mika_arch_identity`. C'est la correction de fond : chaque passe d'architecte est un acte one-shot sur un plan, la première passe part d'une session neuve (donc d'un historique vide), la seconde ne voit que la première — ce qui est exactement ce qu'on veut qu'elle voie. Le régime d'août est retrouvé par construction, non par réglage.

**Ce que ce défaut ne retouche pas, et c'est délibéré :** les agents déjà provisionnés ne sont pas ré-écrits (`write_default_if_missing` ne réécrit jamais un `identity.toml` existant). Pour un mika-arch déjà sur disque, la bascule est une édition d'identité + redémarrage — un geste d'opérateur, énoncé dans la DoD plutôt que découvert après coup.

### Brique 2 — le plafond d'octets, en filet

`max_tokens` convertit en octets via `CHARS_PER_TOKEN_ESTIMATE` (le même 4 que `truncate_to_token_budget`, pas un second estimateur), et la fenêtre est élaguée **du plus ancien vers le plus récent** jusqu'à tenir sous le plafond. Le dernier message — celui du tour — n'est **jamais** élagué : l'élaguer reviendrait à répondre à une question qu'on a effacée. Un marqueur d'élision est inséré, comme le fait déjà `truncate_to_token_budget`, pour que le modèle sache qu'il lui manque quelque chose plutôt que de croire tout savoir.

Le filet a une raison d'être mesurée et non spéculative : `_iterate_groom_loop` enchaîne plusieurs itérations **dans la même session** (plan v1, revue, plan v2, revue…). Le scope par session borne la contamination entre tickets ; il ne borne pas une session qui itère. Les deux leviers couvrent deux axes réellement distincts, et aucun ne rend l'autre inutile.

Sémantique du champ, alignée sur son précédent : absent → pas de plafond ; `Some(0)` → fenêtre vide (sentinelle d'omission, comme `ContextSummaryConfig.max_tokens`). Une valeur aberrante ne panique pas et ne se tait pas : elle retombe sur l'absence de plafond avec un WARN, parce qu'un plafond mal saisi qui viderait silencieusement la fenêtre serait un effacement de contexte déguisé en configuration.

**Valeur pour mika-arch : `max_tokens = 8000`.** Elle se dérive, elle ne se choisit pas : la cible du ticket est < 40 000 tokens d'entrée ; le prompt système de mika-arch et ses skills pèsent ≈ 20 000 tokens (M7 le mesure exactement au déploiement) ; le plan lui-même vaut 6 000 à 15 000 (M2). Il reste ≈ 8 000 pour l'historique avant de toucher les 40 000, dans le cas du plan le plus gros. C'est un plafond, pas une consigne : sous scope-session, la première passe n'en consomme rien.

### Ce que ce plan ne fait pas

Il ne réduit pas le prompt système de mika-arch (54 ko → 59,8 ko le 01/09, explicitement hors périmètre de mika#2189 et sans rapport avec le saut du 03/09). Il ne touche pas au budget HTTP de mika#2293, qui reste le pansement et le reste légitimement. Il ne compacte pas et ne résume pas l'historique : ajouter un appel LLM de compaction pour réduire le coût d'un appel LLM demande d'abord de prouver que le solde est positif, ce que rien ici ne prouve.

## Fire-Disposition

- **AC1 — attribution mesurée (implémentation, préalable bloquant).** Tir sur : `context_window_assembled` sur ≥ 20 tours mika-arch après déploiement de la brique 0. Disposition : **halt-and-surface**. Si `history_bytes` ne domine pas, ou si `distinct_sessions == 1` partout, la thèse est infirmée : **ne pas poser les briques 1 et 2**, écrire la mesure dans mika#2295 et re-groomer. Aucune remédiation automatique.
- **AC2 — non-régression du chemin par défaut (CI).** Tir sur : le diff / la CI. Disposition : **gate CI bloquant**. Un test assied que `scope = "agent"` sans `max_tokens` produit la fenêtre actuelle, octet pour octet. Rouge si le défaut dérive. Pas de remédiation auto.
- **AC3 — plafond et élagage (CI).** Tir sur : le diff / la CI. Disposition : **gate CI bloquant**. Tests unitaires : élagage du plus ancien d'abord ; dernier message jamais élagué ; `Some(0)` → fenêtre vide ; valeur aberrante → pas de plafond + WARN.
- **AC4 — sonde post-déploiement (opérateur).** Tir sur : `turn_usage` filtré `agent_id = "mika-arch"` sur ≥ 24 h et ≥ 50 tours après bascule. Disposition : **halt-and-surface**. `input_tokens` médians < 40 000 → tenu. Toujours ≈ 71 000 → **ne pas élargir le plafond ni re-déployer à l'aveugle** : la composante dominante n'est pas celle qu'on a bornée, et c'est la mesure qui est à refaire, pas le réglage à durcir.
- **AC5 — perte de contexte utile (opérateur, signal négatif).** Tir sur : `truncated_messages` non nul sur mika-arch **en première passe**. Disposition : **halt-and-surface**. Une première passe part d'un historique vide sous scope-session ; y voir de l'élagage signifie que le scope ne s'applique pas, pas que le plafond est trop bas. Ne pas remonter `max_tokens` en réponse à ce signal.
- **Non-action par défaut.** Hors de ces cinq signaux, rien ne se déclenche : aucun ajustement automatique de plafond, aucune bascule d'un autre agent vers `scope = "session"`, aucune purge de messages. Les trois autres agents bien connus gardent le défaut `"agent"` tant qu'une mesure propre à eux ne dit pas le contraire.

## Definition of Done

- La brique 0 est déployée et a produit une attribution sur ≥ 20 tours mika-arch, **avant** que les briques 1 et 2 ne soient posées.
- `[context.history]` existe dans le schéma d'identité avec son défaut inchangé, et `build_mika_arch_identity` l'émet.
- `cargo build`, `cargo clippy`, `cargo fmt --check` et `make verify-bundled-skills` passent.
- **Geste d'opérateur requis, et non automatisable :** `write_default_if_missing` ne réécrit jamais un `identity.toml` existant. Un mika-arch déjà provisionné sur disque **ne bascule pas au déploiement**. Il faut éditer `~/.mika/agents/mika-arch/identity.toml` pour y ajouter le bloc `[context.history]`, puis redémarrer mika-spirit. Sans ce geste, AC6 et AC7 échoueront en donnant l'apparence d'un fix qui n'a pas pris, alors que c'est la bascule qui n'a pas eu lieu — d'où son énoncé ici plutôt que sa découverte après la sonde.
- Les sondes AC6 et AC7 sont relevées après ce redémarrage, pas avant.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les critères ci-dessous sont dérivés de son exigence chiffrée (« ramener sous ~40 k tokens ») et de la conception ci-dessus.

1. **AC1 — l'attribution est mesurable.** Un événement INFO `context_window_assembled` est émis à chaque assemblage de fenêtre, inconditionnellement (non couplé à `MIKA_STORE_LLM_CALLS`), portant au minimum `message_count`, `history_bytes`, `user_message_bytes`, `tool_defs_bytes`, `distinct_sessions`, `oldest_age_secs`, `truncated_messages`, `truncated_bytes`.
2. **AC2 — le défaut est inchangé.** Un agent sans `[context.history]` obtient exactement la fenêtre actuelle : mêmes messages, même ordre, même requête SQL. Assis par un test.
3. **AC3 — le périmètre est configurable et descend en SQL.** `[context.history] scope = "session"` restreint la fenêtre à la session courante **via la requête**, pas par un filtre après chargement. Assis par un test qui intercale des messages d'une autre session au-delà de la limite et vérifie qu'aucun message de la session courante n'est perdu.
4. **AC4 — le plafond d'octets élague correctement.** `max_tokens` élague du plus ancien vers le plus récent ; le dernier message n'est jamais élagué ; un marqueur d'élision est présent ; `Some(0)` vide la fenêtre ; une valeur invalide retombe sur l'absence de plafond avec un WARN.
5. **AC5 — mika-arch est configuré.** `build_mika_arch_identity` émet `[context.history] scope = "session"` et `max_tokens = 8000`. Assis par un test sur l'identité produite.
6. **AC6 — la cible chiffrée est tenue.** Après bascule, sur ≥ 24 h et ≥ 50 tours, la médiane de `turn_usage.input_tokens` pour `agent_id = "mika-arch"` est **< 40 000**.
7. **AC7 — la contamination inter-tickets est fermée.** Après bascule, `context_window_assembled.distinct_sessions` vaut 1 sur les tours mika-arch.

## Rattachement aux critères d'acceptation

| AC | Brique | Vérification |
|---|---|---|
| AC1 | Brique 0 | Test unitaire sur l'émission ; `grep context_window_assembled` post-déploiement |
| AC2 | Brique 1 (défaut) | Test de non-régression de fenêtre — gate CI (AC2 Fire-Disposition) |
| AC3 | Brique 1 | Test d'intercalation inter-sessions — gate CI |
| AC4 | Brique 2 | Tests unitaires d'élagage — gate CI (AC3 Fire-Disposition) |
| AC5 | Brique 1 + 2 | Test sur `build_mika_arch_identity` |
| AC6 | Effet | Sonde post-déploiement (AC4 Fire-Disposition) |
| AC7 | Brique 1 | Sonde post-déploiement `distinct_sessions` |

## Hors portée (repris du ticket, sans extension)

- La taille du prompt système de mika-arch (54 ko → 59,8 ko le 01/09) — hors périmètre de mika#2189, et sans rapport avec le saut du 03/09.
- Le budget HTTP de mika#2293 (300 s) — le pansement, qui reste en place et le reste légitimement.
- La classe de timeout de mika#2280.
- Toute compaction ou résumé LLM de l'historique — non démontré rentable, et ajouterait un appel pour en alléger un autre.
- Les trois autres agents bien connus : ils gardent le défaut `scope = "agent"`.
- `mika-cli/src/remote_ask.rs` et `commands/ask.rs` — désignés par le ticket, mais ils n'assemblent aucun contexte (voir § Rectification).
- La lecture des `skills/bundled/*/system_prompt.md` — explicitement refusée par le ticket, et mauvaise cible.

## Vérification

- `cargo test -p mika-agent context_window` et `cargo test -p mika-agent prompt` (dont les tests AC2/AC3/AC4/AC5).
- `cargo build && cargo clippy && cargo fmt --check`.
- `make verify-bundled-skills` — l'identité de mika-arch change, la cohérence d'allowlist doit rester verte.
- Post-déploiement, dans `$MIKA_SPIRIT_LOG_FILE` :
  - `grep context_window_assembled | jq 'select(.agent_id=="mika-arch") | {history_bytes, distinct_sessions, message_count, truncated_messages}'` — attribution (AC1) et contamination (AC7).
  - `grep turn_usage | jq 'select(.agent_id=="mika-arch") | .input_tokens'` — médiane < 40 000 (AC6).
- SQL de recoupement : `SELECT system_prompt_bytes, request_bytes FROM llm_calls WHERE agent_id = 'mika-arch' ORDER BY created_at DESC LIMIT 50;` — `request_bytes − system_prompt_bytes` doit chuter du même ordre que `history_bytes`. Deux surfaces indépendantes qui doivent bouger ensemble ; si une seule bouge, c'est la mesure qui est en cause.

## Conditions d'arrêt

- **La brique 0 infirme la thèse** (`history_bytes` marginal, ou `distinct_sessions == 1` déjà) → halte, écrire la mesure dans mika#2295, re-groomer sur la composante dominante réelle. Ne pas poser les briques 1 et 2 « puisqu'elles sont écrites ».
- **Le filtre de session ne peut pas descendre en SQL** sans toucher plus de deux appelants → halte et reprise de conception : un filtre après chargement serait faux exactement sous la charge qui a produit l'incident, ce qui est pire que l'absence de correction.
- **AC6 échoue après bascule** (médiane toujours ≈ 71 000) → halte. Ne pas durcir `max_tokens`, ne pas re-déployer : c'est le modèle du défaut qui est faux, et deux réglages successifs sur un modèle faux effacent la trace qui permettrait de le corriger.
- **AC5 de la Fire-Disposition se déclenche** (élagage en première passe) → halte : le scope ne s'applique pas ; réparer le scope, ne pas remonter le plafond.

## Voisinage

- mika#2293 — budget HTTP à 300 s : le pansement dont ce ticket est la correction de fond.
- mika#2280 — classe de timeout.
- mika#2189 — `request_bytes` (v53) et la doctrine « une taille non mesurée sur le chemin d'erreur est une taille perdue » ; c'est cette colonne qui permet le recoupement ci-dessus.
- mika#1217 — `system_prompt_bytes` (v38) et `system_prompt_assembled`, dont `context_window_assembled` est le frère manquant.
- mika#1889 — `turn_usage` : instrument non couplé à la persistance DB, forme reprise ici.
- mika#1019 / mika#1021 — `[context.summary]` : le précédent de forme pour `[context.history]`.
- mika#1727 — le CLI devenu client A2A mince, d'où la rectification de localisation.
