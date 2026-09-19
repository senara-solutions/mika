# mika#1951 — La mémoire agent traverse les sessions : ce n'est pas un défaut, c'est l'absence d'un levier par appel

> Ticket : `senara-solutions/mika#1951`
> Type : fix (substrat + validité de banc)
> Date : 2026-09-19

---

## 1. Ce que le ticket a mesuré, et ce que le code dit de sa cause

**La mesure est juste et reproductible.** Dix `mika ask --agent mika-test
--session-id <uuid-frais>` avec le même prompt ; GLM-5.3 répond « Six. Answer
unchanged. » au premier appel d'un session-id flambant neuf. La batterie de
latence est invalidée : le modèle refuse de re-répondre à un prompt qu'il a vu
« ailleurs ».

**Les deux hypothèses du ticket sont fausses, et la rectification est le premier
livrable.** Le ticket propose `agents.<name>.memory` / `core_memory` interrogés
sans filtre de session, ou `load_recent_messages` sans filtre. Ni l'un ni
l'autre :

- **`core_memory` n'est pas en cause.** Elle est agent-scoped par conception et
  injectée dans le prompt système ; elle ne contient pas la réponse d'un tour
  précédent à moins que le modèle ne l'y écrive explicitement, ce que la batterie
  ne fait pas.
- **Il n'existe pas de table `agents.<name>.memory`.**
- **Le chemin réel est `rebuild_context`**, appelé une fois par tour à
  `agent_loop/mod.rs:4470` :

  ```rust
  let scoped_session_id = match history_config.scope {
      prompt::HistoryScope::Session => Some(session_id),
      prompt::HistoryScope::Agent   => None,
  };
  let mut history = db.rebuild_context(scoped_session_id, scope_task_id, 20).await?;
  ```

  `HistoryScope::Agent` est le **défaut** (`prompt.rs:536`, `#[default]`), et
  sous ce défaut la fenêtre tire les 20 derniers messages de l'agent **toutes
  sessions confondues**. `mika-test` ne déclare pas `[context.history]`
  (`MIKA_TEST_IDENTITY`, `well_known_agents.rs:1434`), donc il est sous le
  défaut. C'est la contamination mesurée, à la ligne près.

**Et il y a un second canal que le ticket ne nomme pas.**
`Database::load_conversation_summary` (`db.rs:3139`) filtre sur `m.agent_id`
**seul** : le résumé de compaction traverse toutes les sessions par
construction. `MIKA_TEST_IDENTITY` ne pose pas `[context.summary]`, donc
`inject` vaut son défaut `true`. Sous le seuil de compaction (50 messages) ce
canal n'explique pas l'incident du 22/08 — dix appels ne compactent pas — mais
**il rouvrirait le même symptôme sur un banc plus long**, et sous une forme
strictement identique. Fermer le scope seul livrerait une isolation partielle
dont la partialité serait invisible : exactement le mode de panne que ce ticket
existe pour clore.

Le substrat qui manquait au ticket existe déjà : `HistoryScope` et
`[context.history]` ont été posés par mika#2295, mis en vigueur sur disque par
mika#2330, et instrumentés par mika#2305 (`context_window_assembled` porte
`history_scope` **et** `distinct_sessions`). Ce plan n'invente aucun mécanisme —
il tranche la décision produit que mika#2330 a laissée ouverte en toutes lettres :
*« giving a `session` window to a non-one-shot role is a product decision nobody
has taken »*.

---

## 2. AC3 tranché, et la réponse n'est pas « strictement par session »

AC3 demande : la mémoire agent est-elle voulue inter-session, ou strictement
par session ? **La réponse est mesurée, et elle interdit la bascule du défaut de
flotte.**

`server/handlers.rs:1319` — pour tout agent **non-singleton**, chaque message
entrant sur `/message` mint une session neuve :

```rust
let session_id = if let Some(ref canonical) = a.canonical_session_id {
    /* … singleton : session canonique … */
} else {
    let id = uuid::Uuid::new_v4().to_string();   // ← un UUID PAR MESSAGE
```

**Donc sur Telegram, chaque message de l'utilisateur est une session neuve.** La
notion de session n'a aucun ancrage transport : le gateway ne porte pas
d'identifiant de fil, et rien ne recolle deux messages consécutifs. Poser
`scope = "session"` par défaut ferait perdre à un tenant famille la mémoire de
la question qu'il vient de poser — il faudrait tout redire à chaque message.

**`HistoryScope::Agent` n'est donc pas un bug : c'est le mécanisme qui porte la
continuité conversationnelle là où la session n'existe pas comme fait
transport.** Le ticket raisonne depuis un banc, où l'agrégation est une
contamination ; sur le canal principal, la même agrégation est la conversation.
Une seule des deux lectures peut devenir le défaut, et ce n'est pas celle qui
casse le produit.

**Réponse par population :**

| population | scope voulu | porté par |
|---|---|---|
| banc / one-shot (`mika-test`, passes architecte) | `session` | l'identité, code-owned |
| assistant conversationnel (famille, opérateur, Telegram) | `agent` | le défaut, inchangé |
| un appel donné qui veut être isolé | `session`, **pour ce tour** | **manquant — c'est le livrable** |

**Le vrai défaut est donc étroit et précis : le scope est une propriété de
l'identité — permanente, par agent — et jamais une propriété de l'appel.** Un
banc qui veut N appels indépendants n'a aucun levier : il devrait éditer
l'`identity.toml` de l'agent, redémarrer le démon, mesurer, rééditer,
redémarrer. Et `mika-test`, dont la raison d'être écrite est d'être un banc
(« a bare engine exerciser », mika#963), porte le défaut conversationnel.

---

## 3. Trois unités

### U1 — `mika-test` est isolé par son identité (ferme le symptôme mesuré)

`MIKA_TEST_IDENTITY` gagne deux blocs :

```toml
[context.history]
scope = "session"

[context.summary]
inject = false
```

Les **deux** canaux de §1, fermés ensemble. `inject = false` est le
`load-prevention` de mika#1009, déjà en vigueur sur mika-arch pour une raison
voisine : un banc doit partir d'un état connu, pas d'un résumé dont il ignore la
composition.

**Rien d'autre à faire pour que ça atterrisse :** `context.history` et
`context.summary` sont **déjà** tous deux dans `CODE_OWNED_IDENTITY_SECTIONS`
(`well_known_agents.rs:542`), donc `reconcile_well_known_identity` écrit les
sections sur un `mika-test` déjà provisionné, au prochain démarrage de
mika-spirit, y compris sous `MIKA_DISABLE_AGENT_PROVISIONING=1` (mika#2330). Pas
de migration, pas de geste opérateur, pas de re-provisionnement. C'est
précisément la leçon que mika#2327 a payée en restant inerte : sans l'entrée
code-owned, le correctif aurait été mergé et invisible sur l'agent mesuré.

**Coût nommé :** une édition à la main de `[context.history]` ou
`[context.summary]` dans le `identity.toml` de mika-test sera écrasée au
démarrage suivant, et `reconciled_paths` sur `identity_reconcile.complete` le
dira. C'est le contrat des sections code-owned, pas une régression.

**Ce que U1 ne ferme pas :** tout autre agent. La batterie aurait pu tourner
sur mika-dev ; un opérateur voulant une passe isolée sur un tenant n'a toujours
rien. D'où U2.

### U2 — Un levier par appel : `mika.session_isolated`

Quatrième clé de la famille `mika.*` de request-metadata A2A, aux côtés de
`mika.caller_session_id` (mika#2070), `mika.only_skills` (mika#2363) et
`mika.model_override` (mika#2304). Déclarée dans `mika_a2a::params`, qui est le
site où la maison écrit ses contrats de fil — `mika-cli` et `mika-agent` ne
partagent aucune arête de dépendance, donc ni l'un ni l'autre ne peut renommer
la clé seul.

**Booléen, jamais un enum — et c'est une contrainte de sûreté, pas un choix de
style.** `/a2a/{agent}` est joignable par tout appelant authentifié. Une clé
portant un `HistoryScope` permettrait de demander `"agent"` sur un agent
configuré en `"session"` : un appelant pourrait **élargir** la fenêtre de
mika-arch et lui faire relire les plans d'autres tickets, c'est-à-dire rouvrir
mika#2295 et mika#2305 par la porte réseau. Un booléen dont seul `true` a un
effet rend l'élargissement **inexprimable par construction** plutôt que refusé
par un prédicat qu'un futur éditeur assouplirait. C'est le raisonnement
« strictement soustractif » de `apply_only_skills`, transposé.

Sémantique : `true` ⇒ ce tour lit `HistoryScope::Session` **et** n'injecte aucun
résumé de compaction, quoi que dise l'identité. Absent, `null`, non-booléen,
`false` ⇒ aucune restriction, et le tour est celui d'avant au bit près.

**Lecture fail-soft, application fail-closed** — l'asymétrie de
`mika.model_override`, pour sa raison mot pour mot : une restriction de skills
silencieusement perdue rend le tour *plus large*, ce qui est visible et ne
falsifie aucune mesure ; une **isolation** silencieusement perdue rend la mesure
fausse tout en produisant une réponse plausible. C'est littéralement le défaut de
mika#1951 : la batterie a produit des données contaminées qui avaient l'air
valides. Un no-op silencieux ici *est* le défaut.

Mécanisme côté serveur : `AgentParams` gagne `session_isolated: bool`, exactement
la forme de `caller_model_override: bool` (`agent_loop/mod.rs:4054`). `ctx.identity`
est une référence partagée ; le bool évite d'en cloner une copie mutée, et écarte
par construction la classe « écrit dans le cache » que mika#2363 a dû tenir par
un test lexical. Le bool est lu à deux sites, et deux seulement :

1. la résolution de `scoped_session_id` (`mod.rs:4466`) ;
2. l'appel à `load_gated_summary` (`mod.rs:4291`).

CLI : `mika ask --isolated`. Les metadata sont construites au site unique
`crates/mika-cli/src/remote_ask.rs` (`request_metadata`), qui sert **les deux
portes** de `mika ask` — depuis mika#1727 le chemin par défaut passe lui aussi
par A2A, et c'est la moitié que mika#2304 a dû corriger après que le ticket
d'origine n'eut visé que `--remote`. Poser la clé au site unique la donne aux
deux ; l'implémentation doit le vérifier plutôt que le supposer.

### U3 — L'attestation, sans quoi U2 reproduit mika#2304

Le serveur pose sur `Task.metadata` ce qu'il a **réellement** fait :
`mika.session_isolated_applied` (booléen), écrit sur **chaque** tour de
`message/send`, isolé ou non — au point d'intervention mika#2270.
`mika ask --verbose` rend cette valeur, jamais la valeur locale.

**Pourquoi c'est la moitié qui compte.** Sans elle, un banc lancé contre un
mika-spirit antérieur au correctif verrait `--isolated` accepté par son CLI,
ignoré par le serveur, et afficherait l'isolation avec autorité. C'est
exactement le faux vert que mika#2304 a mesuré (`model:` affichait le modèle
demandé pendant que le tour tournait sous celui du `config.toml`) et c'est
exactement le défaut de mika#1951 : croire mesurer isolé et ne pas l'être. Livrer
U2 sans U3 remplacerait une contamination par une confiance.

Absence de la clé dans la réponse ⇒ « ce serveur n'a rien attesté », **jamais**
« la valeur locale est bonne ». En texte : `isolated: (not attested by the
server)` ; en JSON : champ absent.

---

## 4. Ce qui est refusé, et pourquoi

- **Basculer le défaut de flotte à `session`.** Casse la continuité
  conversationnelle sur Telegram (§2), où chaque message mint une session. Ce
  n'est pas un arbitrage de prudence : c'est une mesure sur
  `handlers.rs:1319`.
- **Une clé A2A portant un `HistoryScope`.** Permettrait d'élargir la fenêtre
  d'un agent depuis le réseau, donc de rouvrir mika#2295/#2305 (§U2).
- **Dériver l'isolation d'un `--session-id` frais.** C'est exactement ce que la
  batterie a fait, et c'est la preuve du ticket : dix UUID neufs, dix fenêtres
  contaminées. La session neuve est déjà là ; ce qui manque est que la fenêtre
  la respecte.
- **Une variable d'environnement.** Elle serait per-process, pas per-appel, et
  relèverait de la famille « not hot-swappable » (mika#2329 a dû écrire pourquoi
  en faire une exception invisible est pire que le défaut).
- **Fermer la mémoire agent-scoped** (`store_fact`, `update_fact`,
  `search_memory`, `update_core_memory`). Voir §5.

---

## 5. Canal résiduel, nommé et laissé intact

`scope = "session"` ne borne **pas** la mémoire agent-scoped : `store_fact`,
`update_fact`, `search_memory` et `update_core_memory` traversent toutes les
sessions **par conception**, et c'est épinglé comme constitutif d'être un agent
(`test_mika_arch_disabled_tools_excludes_agent_self_state` : *« agent self-state,
not platform side-effect »*). Si le tour 1 d'une batterie écrit un fait, le tour
2 peut le retrouver.

**Pourquoi ce n'est pas fermé ici.** (a) Rien dans l'évidence du 22/08 ne
l'implique : la contamination mesurée s'explique en entier par la fenêtre, et
GLM-5.3 n'a appelé aucun outil de mémoire pour répondre « Six ». (b) `mika-test`
n'a aucune skill (`allowlist = ["__mika_test_no_skills__"]`), ce qui réduit sans
l'annuler la surface. (c) Les fermer serait amputer la mémoire d'un agent contre
un pin explicite — une décision produit hors du périmètre d'un ticket substrat.
**Ticket de suivi**, avec pour préalable une mesure : ces outils sont-ils seulement
appelés pendant une passe de banc ?

---

## 6. Vérification

### Tests

- **U1** — `well_known_agents::tests` : l'identité mika-test parse et rend
  `scope == Session` et `summary.inject == false` (le pendant de
  `mika2295_mika_arch_identity_bounds_its_conversation_window`) ; et une reprise
  du motif `mika2330_code_owned_identity_is_written_at_boot_when_provisioning_is_disabled`
  pour mika-test, qui **dépouille** les deux sections d'un `identity.toml` déjà
  écrit et vérifie qu'elles reviennent. Ce second test est le porteur : le
  premier prouve la constante, seul le second prouve que la correction atteint
  l'agent mesuré.
- **U2, contrôle positif + contrôle négatif au site de production.** Sur le
  modèle de `tests/eval/test_context_scope_observability_2305.rs`, qui porte
  déjà les deux moitiés : avec la clé, `context_window_assembled` rend
  `history_scope = "session"`, `distinct_sessions = 1`, et le message de l'autre
  session est absent ; **sans** la clé, sur le même agent en scope `agent`, il
  rend `"agent"` / `2` et l'autre message est présent. Le contrôle négatif est
  ce qui distingue « le filtre filtre » de « le champ est une constante ».
- **U2, les deux canaux séparément.** Un test où seul le résumé porterait la
  contamination (historique vide, résumé présent) : sans lui, une isolation qui
  ne fermerait que la fenêtre passerait pour complète — c'est le piège de §1.
- **U2, fail-closed.** Une clé déclarée avec une valeur non booléenne refuse le
  tour en `INVALID_PARAMS` plutôt que de la lire comme `false`.
- **U2, non-élargissement.** `mika.session_isolated = false` sur mika-arch (scope
  `session`) laisse le scope à `session`. La clé ne peut pas élargir — asserté,
  pas supposé.
- **U3** — un `Task` servi par un chemin sans attestation rend l'absence, et le
  CLI affiche « not attested » au lieu de la valeur locale.

### Sondes post-déploiement, et leurs haltes

**Sonde A — AC1, le rejeu de l'incident.** Trois `mika ask --agent mika-test
--session-id <uuid-frais>` avec le même prompt, **sans** `--isolated`. Puis :

```bash
grep context_window_assembled "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "mika-test")
        | {history_scope, distinct_sessions, message_count}'
```

Attendu après U1 : `"session"` / `1` sur les trois. **Halte 1 —
`history_scope: "agent"`** : la réconciliation n'a pas eu lieu ; lire
`identity_reconcile` **avant de toucher au code**, c'est la classe mika#2330 et
le remède est un déploiement. **Halte 2 — `"session"` avec
`distinct_sessions > 1`** : le filtre ne filtre pas, la fuite est sous
`rebuild_context` et pas dans le réglage ; ne pas rétrécir `max_tokens` par
réflexe, c'est l'autre axe et il masquerait le symptôme.

**Sonde B — U2 sur un agent conversationnel.** Même rejeu avec `--isolated` sur
un agent laissé en scope `agent` : `history_scope: "session"` sur la ligne, et
`isolated: true` attesté par le serveur dans `--verbose`. **Halte 3 — le CLI
affiche l'isolation et le journal porte `"agent"`** : l'attestation n'est pas
lue depuis le serveur ; ne pas ajuster l'affichage, c'est U3 qui est cassé et
c'est la seule panne qui se déguise en succès.

**Sonde C — non-régression conversationnelle, et c'est celle qu'on oublie.** Sur
un tenant Telegram, deux messages consécutifs dont le second dépend du premier
(« Combien font 3+3 ? » puis « et en doublant ? »). Attendu : la continuité est
intacte, `history_scope: "agent"`, `distinct_sessions > 1`. **Halte 4** — si la
continuité est perdue, U1 a débordé de sa population : vérifier qu'aucune
section n'a été posée hors de `MIKA_TEST_IDENTITY`.

**Ce que ce travail n'achète pas.** Aucun test déterministe ne peut établir
qu'un LLM cessera de répondre « answer unchanged » : la moitié comportementale
se mesure en rejouant la batterie. Et **le silence ne prouve rien si personne ne
rejoue** — vérifier `message_count` non nul avant de conclure quoi que ce soit
(mika#2205).

---

## 7. Definition of Done

- `MIKA_TEST_IDENTITY` déclare `[context.history] scope = "session"` et
  `[context.summary] inject = false`, et un test prouve que les deux sections
  atteignent un `identity.toml` déjà sur disque.
- `mika_a2a::params::SESSION_ISOLATED_KEY` et son pendant
  `SESSION_ISOLATED_APPLIED_KEY` existent, documentés au même niveau que leurs
  trois sœurs, avec l'asymétrie lecture/application écrite sur la constante.
- `AgentParams.session_isolated` est lu aux deux sites de §U2 et nulle part
  ailleurs.
- `mika ask --isolated` pose la clé sur **les deux** portes ; `--verbose` rend
  l'attestation du serveur et jamais la valeur locale.
- Les sondes A, B et C ont tourné ; leurs haltes sont écrites dans le corps de
  la PR avec le résultat obtenu.
- `cargo test`, `cargo clippy`, `cargo fmt` passent.
- Le canal résiduel de §5 est nommé dans le corps de la PR, avec son ticket de
  suivi.

---

## 8. Acceptance criteria

Transcrits du corps de `senara-solutions/mika#1951` :

- **AC1** — reproduce the leak : fresh 3 session-ids × same identical prompt on
  `mika-test`, verify contamination (should be trivial)
- **AC2** — identify the load path : grep `load_recent_messages` / `agent_memory`
  / `core_memory` for session-agnostic queries
- **AC3** — determine expected behavior : is agent-level memory intended
  cross-session (persona shape, doctrine) OR strictly per-session (conversation
  state) ?
- **AC4** — if scoping fix warranted : add session_id filter to appropriate load
  path
- **AC5** — regression test : bench workflow with per-session isolation
  guaranteed

**Où chacun est satisfait :**

| AC | satisfait par |
|---|---|
| AC1 | Sonde A (§6), rejeu à trois session-ids ; la contamination est reproduite avant U1 et absente après |
| AC2 | §1 — `rebuild_context` sous `HistoryScope::Agent` (`mod.rs:4470`), **plus** `load_conversation_summary` keyé `agent_id` seul (`db.rs:3139`), que le ticket ne nommait pas. Ni `core_memory` ni `agent_memory` : les deux hypothèses du ticket sont écartées avec leur raison |
| AC3 | §2 — tranché par population, sur la mesure `handlers.rs:1319` : inter-session est **voulu** pour le conversationnel, per-session pour le banc et le one-shot. Le défaut de flotte ne bouge pas |
| AC4 | U1 (identité de `mika-test`) + U2 (levier par appel). Le filtre existe déjà ; ce qui est ajouté, c'est qui peut le demander et quand |
| AC5 | §6 — contrôle positif **et** contrôle négatif au site de production, un test par canal, plus le non-élargissement et le fail-closed |

---

## 9. Hors périmètre, délibérément

- **La mémoire agent-scoped** (§5) — ticket de suivi, préalable = une mesure.
- **Le défaut de flotte `HistoryScope`** (§2) — une bascule casserait Telegram.
- **La moitié additive du canal A2A** — refusée par mika#2363 avec sa raison, et
  ce ticket n'y touche pas : la clé posée ici est strictement restrictive.
- **Le déterminisme d'un verdict LLM** — une session isolée garantit un même
  *état de départ*, jamais un même *verdict* ; c'est une question d'évaluation
  (famille `calibrate-*`), pas de substrat.
- **`mika chat`** — in-process, session stable par construction, hors du
  symptôme.
