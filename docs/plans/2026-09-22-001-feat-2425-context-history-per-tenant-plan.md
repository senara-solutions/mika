# mika#2425 — le réglage `[context.history]` est per-tenant, et la DB ne peut que rétrécir

**Ticket :** senara-solutions/mika#2425
**Type :** feat
**Statut :** plan v2 (contenu seul — architecte en aval)

> **Divergence plan ↔ ticket, ratifiée et non silencieuse.** Ce plan remplace les
> exigences 1 et 2 du corps de mika#2425 (« un verbe CLI/console qui écrit la
> section nested `[context.history]` de l'`identity.toml` » ; « l'interaction avec
> le réconciliateur (#2330) ») par un mécanisme `customer_config` — refus 1 et
> point 1 du § *Pourquoi « rétrécir seulement »*. Le corps du ticket porte encore
> la formulation d'origine ; **U10 en prescrit la rectification**, selon la
> convention issue-as-versioned-contract établie sur mika#2169/#2158 : corps édité
> avec encadré daté, avis d'édition en commentaire, annotation de clôture ici. Un
> lecteur aval qui lit le ticket avant ce plan doit trouver la divergence écrite
> dans le ticket, jamais la déduire de l'écart.

---

## Ce que la lecture du code déplace dans le ticket

Trois mesures relues à HEAD (`b7e12f73`). Chacune change le livrable, et la
troisième change la **conduite** que ce ticket peut prescrire.

### R1 — le réconciliateur ne touche AUCUN tenant

`WELL_KNOWN_AGENTS` vaut `[&MIKA_DEV, &MIKA_TEST, &MIKA_QA, &MIKA_ARCH]`
(`well_known_agents.rs:504`), et `provision_well_known_agents` boucle sur cette
constante et rien d'autre. `reconcile_well_known_identity` n'a donc **jamais**
été exécuté contre un agent client. L'exigence #2 du ticket — « une valeur
opérateur dans une section code-owned ne doit pas être écrasée au restart » — ne
décrit pas la population que L8 a mesurée : elle décrit les quatre agents
d'ingénierie, dont un seul (mika-arch) pose un non-défaut.

Ce qui écrase réellement l'`identity.toml` d'un tenant est `mika agents
reprovision` — un geste explicite, différentiel, qui sauvegarde en `.bak.<ts>`
et que personne ne déclenche par surprise (mika#2230).

**Conséquence :** il n'y a pas de conflit réconciliateur à résoudre pour la
population visée. Il y en a un si et seulement si l'on choisit d'écrire dans
l'`identity.toml` — voir le refus 1 ci-dessous.

### R2 — l'identité est relue à CHAQUE tour, donc le réglage est hot

`load_agent_context` (`agent_loop/mod.rs:478`) appelle
`prompt::load_identity_async(home_dir)` à chaque tour, et c'est le **funnel des
trois boucles** (conversation, silencieuse, équipe) — le commentaire du site le
dit en toutes lettres. Un réglage de fenêtre n'a donc pas le contrat
*not hot-swappable* de `MIKA_AGENT_TIER` ou `MIKA_DEPLOYMENT` : il prend effet au
tour suivant, sans redémarrage, quelle que soit la porte par laquelle il est
posé. Le ticket ne le dit pas, et c'est ce qui rend le mécanisme ci-dessous
rentable — un réglage per-tenant qui exigerait un restart du daemon partagé
serait inutilisable sur une flotte de tenants.

### R3 — sur le chemin Telegram, `scope = session` ne retire pas 90 % : il vide

**C'est la mesure décisive, et le ticket ne la porte pas.**

`MessageRequest` (`server/types.rs:12`) ne porte **aucun** `session_id`. Le
handler `/message` résout la session ainsi (`server/handlers.rs:1310`) : si
l'agent est singleton, la session canonique ; **sinon `Uuid::new_v4()`, une par
message inbound**. Et ni `DEFAULT_IDENTITY` ni `FAMILY_IDENTITY`
(`mika-common/src/home.rs:677`, `:761`) ne déclarent `[session] singleton`.

Donc pour un tenant Telegram provisionné par tier — la population entière de la
charte — une « session » **est** un message. `scope = session` y produit une
fenêtre conversationnelle vide : pas 3 220 → 330 tokens, mais 3 220 → le seul
message du tour. Y compris le tour précédent de la conversation en cours.

La mesure L8 (résiduel médian de 330 tok) est donc **incompatible** avec un
tenant Telegram par-message et a été prise sur une population dont les sessions
portent plusieurs messages (CLI `mika chat`, appels A2A à `--session-id` stable,
ou agents singleton). Rien n'invalide la mesure ; ce qui est invalide est de
transporter sa conclusion sur la population Telegram sans l'établir.

**Deux atténuations, réelles mais partielles, à ne pas confondre avec une
réfutation.** Le résumé conversationnel est keyé sur `agent_id` seul
(`db.rs:3219` — `load_conversation_summary(agent_id)`) et survit au
rétrécissement ; `search_memory`, la core memory et les faits structurés sont
agent-scoped et survivent aussi — c'est exactement ce que le dépôt écrit déjà
pour mika-arch. Ce qui disparaît est l'historique **brut** des derniers tours.

**Conséquence sur le livrable :** ce ticket livre le **mécanisme** et le
**défaut inchangé** (exigence #3, qui devient la protection principale), plus un
avertissement à l'écriture et une sonde préalable. Il ne prescrit `session` sur
aucun tenant : cette décision demande d'abord la mesure du § *Sondes*.

---

## Le mécanisme : une clé `customer_config`, et une cascade qui ne peut que rétrécir

Deux clés dans la table `customer_config` (per-agent par construction —
`set_customer_config(agent_id, …)`, `db.rs:4119`) :

| clé | valeurs | neutre |
|---|---|---|
| `context_history_scope` | `agent` \| `session` | `agent` |
| `context_history_max_tokens` | `none` \| entier `>= 500` | `none` |

Résolution, au site unique qui décide déjà :

```
plancher de rôle   = identity.toml [context.history]     (inchangé)
rétrécissement     = customer_config                      (nouveau)
effectif           = le PLUS ÉTROIT des deux
```

- **`scope`** — `session` gagne toujours ; `agent` en DB est le neutre et ne
  relâche jamais un `session` déclaré en identité.
- **`max_tokens`** — `min` des deux bornes, `None` valant « pas de plafond ».

### Pourquoi « rétrécir seulement », et le précédent est dans le même fichier

`agent_loop/mod.rs:4898` porte déjà cette asymétrie, sur ce champ exact, pour
mika#1951 : *« The caller may narrow this turn's scope to its own session; it may
never widen it. […] That asymmetry is why the wire key is a bool: the widening
request has no spelling. »*

Elle achète ici quatre choses d'un coup :

1. **L'exigence #2 du ticket est dissoute, pas implémentée.** La valeur opérateur
   ne vit pas dans une section code-owned, donc le réconciliateur ne la voit
   jamais. Aucune ligne de `CODE_OWNED_IDENTITY_SECTIONS` ne bouge. *Dissoudre
   n'est pas ratifier* : le corps du ticket continuerait d'exiger cette
   interaction tant que U10 n'est pas exécuté.
2. **mika#2295 ne peut pas se rouvrir.** Personne ne peut remettre mika-arch en
   `scope = agent` depuis la DB.
3. **La réversibilité est gratuite.** `delete_customer_config` **n'existe pas**
   (vérifié : `db.rs` n'expose que `set_customer_config`), donc une clé posée ne
   s'efface pas. Poser le neutre (`agent` / `none`) est l'annulation, et c'est
   exactement ce que la règle rend sûr.
4. **Le jour où la clé deviendra settable par le modèle (suivi 2), il ne pourra
   pas élargir sa propre fenêtre.** La sûreté est structurelle, pas
   conventionnelle.

**Coût nommé :** un opérateur ne peut pas élargir la fenêtre de mika-arch depuis
la DB. C'est refusé et **dit** (`context_history_widening_refused`, WARN). Le
remède est une édition de `well_known_agents.rs`, ce qui est correct puisque le
code déclare cette borne comme une propriété du rôle.

---

## Trois refus raisonnés

### Refus 1 — écrire la section nested de l'`identity.toml` (la lettre du ticket)

Le ticket prescrit « un verbe CLI/console qui écrit la section nested
`[context.history]` de l'`identity.toml` ». C'est le **moyen**, pas la fin ; les
trois exigences (exposé per-tenant, non écrasé au restart, défaut inchangé) sont
satisfaites par la DB et mieux. Trois mesures :

- **Aucun backend n'écrit `identity.toml`.** `ConfigBackend::File` vise
  `config.toml` (`config.rs:128`, `write_config_toml`), et ce writer pose des
  clés **plates** via `doc[key] = …`. Un backend `Identity` serait à construire
  entièrement, avec son atomicité, ses permissions, et son interaction avec le
  parse fail-closed de `load_identity` (mika#2027 : un `identity.toml` corrompu
  évince toutes les skills de l'agent — un writer bogué y coûte l'agent, pas le
  réglage).
- **Pour les quatre well-known, satisfaire #2 inverserait mika#2330.** Ce ticket
  a écrit noir sur blanc : *« une édition à la main dans une section code-owned
  est maintenant écrasée au prochain démarrage… la perte est lisible plutôt que
  silencieuse »*. Préserver une valeur opérateur dans `context.history`
  demanderait un marqueur de provenance dans le TOML et rendrait le
  réconciliateur conditionnel — pour une population de un (mika-arch).
- **`customer_config` est le site que la maison a déjà choisi pour cette
  classe**, avec sa raison écrite (mika#2358, `config_keys.rs:16`) : *« it was
  chosen for one measured property: nothing rewrites it at startup »*. Et
  `load_agent_context` le lit déjà deux fois (`timezone`, `language`) à la ligne
  d'à côté de `load_identity_async` : le lecteur atterrit au bon endroit sans
  nouveau chemin d'accès.

**Ce refus est une divergence avec la lettre du ticket, et il ne se ratifie pas
ici.** Un plan qui réfute une exigence sans que le ticket le dise laisse deux
contrats contradictoires en circulation, dont un seul est lu par l'opérateur qui
arrive par la recherche GitHub. La rectification du corps est prescrite par
**U10**, gatée par **AC9** et par une case de DoD.

### Refus 2 — exposer les clés au modèle (`SETTABLE_CONFIG_KEYS`) en v1

`SETTABLE_CONFIG_KEYS` **est** la surface d'outil (`SetConfigTool::definition`
construit son `enum` et sa description depuis la constante), donc **l'omission
est le geste** : ne pas y ajouter la clé suffit, il n'y a rien à écrire.

Raison : la seule population mesurée est un besoin **opérateur** (L8), pas une
demande utilisateur. Et poser une amnésie conversationnelle par inférence sur une
phrase ordinaire (« oublie ce qu'on a dit ») est un mode de panne dont la mesure
est absente — le motif maison est de mesurer avant d'instruire. **Suivi nommé**,
précondition : une demande utilisateur mesurée, plus la sonde R3 verte sur la
population concernée.

### Refus 3 — un troisième scope `channel`

Pour un tenant Telegram, l'unité conversationnelle n'est ni la session (un
message, R3) ni l'agent (toutes les conversations) : c'est le **chat**. Un scope
`channel` serait la réponse juste à la charte. Il n'est pas livré ici : il change
la signature de `rebuild_context`, demande une colonne ou un dérivé sur
`sessions`, et le ticket demande deux valeurs nommées (`Agent|Session`).
**Suivi nommé**, précondition : la sonde R3 établissant que la population
Telegram est bien celle que L8 vise.

---

## Requirements

- **U1** — Deux clés `customer_config` (`context_history_scope`,
  `context_history_max_tokens`), déclarées dans `CONFIG_KEYS`
  (`mika-common/src/config.rs`) en `ConfigBackend::Database`, validées par
  `config_keys::validate_config_value`, et **absentes** de
  `SETTABLE_CONFIG_KEYS`.
- **U2** — Un résolveur unique produisant
  `ResolvedContextHistory { scope, scope_source, max_tokens, max_tokens_source }`
  à partir du couple (identité, DB), appliquant la règle « rétrécir seulement ».
- **U3** — Le site décisionnel existant (`scoped_session_id`) consomme le scope
  **résolu** et non `ctx.identity.context.history.scope`. Idem pour
  `truncate_history_to_token_budget` et pour le champ `history_scope` de
  `context_window_assembled` — sans quoi l'instrument de mika#2305 rapporterait
  le scope déclaré pendant qu'un autre s'applique.
- **U4** — `context_history_resolved` (INFO, ungated, dédupliqué sur le couple
  résolu, per-agent).
- **U5** — `context_history_widening_refused` (WARN) quand la DB tente
  d'élargir.
- **U6** — Avertissement à l'écriture CLI quand `context_history_scope=session`
  est posé sur un agent **non singleton** — nommant R3 et la sonde. Avertit,
  **ne refuse pas**.
- **U7** — `validate_config_value` refuse `0` pour `context_history_max_tokens`
  et impose un plancher de `500`. Raison écrite au site : le sentinel
  d'omission (`Some(0)` = fenêtre vide) est une décision de rôle portée par
  l'identité, pas une préférence tenant, et un `0` posé par erreur est *un
  effacement de contexte déguisé en configuration* — la phrase que
  `deserialize_history_max_tokens` porte déjà.
- **U8** — Défaut strictement inchangé : sans clé DB, chaque agent résout
  exactement ce qu'il résout aujourd'hui.
- **U9** — Documentation : entrée dans `crates/mika-agent/docs/configuration.md`
  et `docs/configuration.md` (sync via `scripts/sync-agent-docs.sh`), plus la
  section d'exploitation dans `CLAUDE.md` (surfaces, sondes, haltes).
- **U10** — **Rectification du corps de mika#2425**, parce que ce plan réfute deux
  de ses trois exigences et qu'une divergence qu'aucune des deux surfaces ne porte
  n'est pas ratifiée : elle est seulement invisible. Trois gestes, dans cet ordre,
  et aucun n'est optionnel :
  1. **Corps du ticket édité**, avec un encadré daté en tête constatant que les
     exigences 1 (verbe écrivant la section nested `[context.history]` de
     l'`identity.toml`) et 2 (interaction avec le réconciliateur #2330) sont
     **remplacées** par le mécanisme `customer_config`, et nommant les trois
     mesures qui l'imposent (R1 — le réconciliateur ne touche aucun tenant ;
     refus 1 — aucun backend n'écrit `identity.toml`, et satisfaire #2 sur les
     well-known inverserait mika#2330 ; mika#2358 — `customer_config` est le site
     que la maison a déjà choisi pour cette classe). Le texte d'origine est
     **conservé** sous l'encadré, jamais réécrit en place : un contrat versionné
     garde ce qu'il a dit.
  2. **Avis d'édition en commentaire** sur le ticket, nommant la date, la PR ou le
     plan qui motive l'édition, et ce qui a changé — sans quoi l'édition est un
     fait que seul l'historique GitHub porte, invisible à qui lit le fil.
  3. **Annotation de clôture dans ce plan** — l'encadré en tête de ce fichier,
     déjà posé, plus la mention explicite dans le corps de PR que la rectification
     a été faite et où la lire.

  L'exigence 3 du ticket (défaut inchangé) est **satisfaite telle quelle** par U8
  et ne fait l'objet d'aucune rectification. Citation : convention
  issue-as-versioned-contract établie sur mika#2169 et mika#2158 (corps édité +
  avis d'édition + annotation de clôture) ; `docs/architecture/review-guide.md`
  § divergence plan ↔ spec — un plan ne ratifie pas unilatéralement une divergence
  avec le ticket qui le commande.

## Non-requirements (hors périmètre, délibérément)

- Un backend `ConfigBackend::Identity` ou tout writer de TOML nested (refus 1).
- Toute modification de `CODE_OWNED_IDENTITY_SECTIONS` ou du réconciliateur.
- Toute modification d'une valeur en vigueur : mika-arch reste
  `session`/`8000`, tout le reste reste `agent`/`None`.
- Un scope `channel` (refus 3) ; l'exposition au modèle (refus 2).
- `delete_customer_config` — le neutre rend la suppression inutile ici, et
  ajouter un verbe de suppression change la surface de toutes les clés DB.
- Un chemin console : `mika-cloud` est hors de ce workspace. La route lecture
  existe déjà en forme (`GET /api/v1/agents/{id}/budget`) et un sibling
  `context_history` est un ticket console.

---

## Implémentation

### B1 — les deux clés (`mika-common/src/config.rs`, `mika-agent/src/config_keys.rs`)

Deux `ConfigKeyInfo { backend: Database, env_var: None, secret: false }` à côté
de `timezone` et `thinking_level`. Deux constantes `CONTEXT_HISTORY_SCOPE_KEY` /
`CONTEXT_HISTORY_MAX_TOKENS_KEY` dans `config_keys.rs`, deux bras dans
`validate_config_value`, avec le message d'erreur nommant le neutre (`agent`,
`none`) — un opérateur qui découvre la clé doit lire comment l'annuler dans le
message qui refuse sa faute de frappe.

**Ne pas toucher `SETTABLE_CONFIG_KEYS`** (refus 2), avec un commentaire sur la
constante disant pourquoi l'absence est le geste.

### B2 — le résolveur (`crates/mika-agent/src/agent_loop/context_history.rs`)

Module neuf **sous `agent_loop/`** — le choix est contraint par D1, voir
*Fire-Disposition*.

```rust
pub struct ResolvedContextHistory {
    pub scope: prompt::HistoryScope,
    pub scope_source: &'static str,        // format de fil
    pub max_tokens: Option<usize>,
    pub max_tokens_source: &'static str,   // format de fil
}

pub fn resolve(
    declared: &prompt::ContextHistoryConfig,
    db_scope: Option<&str>,
    db_max_tokens: Option<&str>,
    agent_id: &str,
) -> ResolvedContextHistory
```

Valeurs de source (constantes d'un seul lieu, épinglées par D2) :
`"identity"`, `"customer_config"`, `"default"`.

Trois paliers maison sur chaque lecture DB : absent ou vide → le déclaré ;
illisible → le déclaré **plus un WARN nommant la valeur entre guillemets** ;
valide → la règle du plus étroit. Une DB illisible **ne peut pas** faire échouer
le tour : la fonction ne rend pas de `Result`.

`report_resolved_context_history(agent_id, &resolved)` émet `context_history_resolved`
dédupliqué sur le couple résolu — modèle exact de
`report_resolved_tenant_language` (mika#2247) et de `llm_budget_resolved`
(mika#2293), y compris la règle « une répétition à l'identique est tue, un
changement est ré-émis ».

### B3 — les deux lectures DB (`load_agent_context`)

Deux `db.get_customer_config(…)` à la suite de `timezone` et `language`, même
fail-open, même `warn!` nommé. Les valeurs brutes voyagent sur `AgentContext` ;
la résolution a lieu au site décisionnel (B4) pour que le résolveur et le
consommateur ne soient jamais séparés par une frontière de fonction.

### B4 — le site décisionnel (`agent_loop/mod.rs`, ~4896)

```rust
let declared = &ctx.identity.context.history;
let resolved = context_history::resolve(declared, ctx.db_history_scope.as_deref(),
                                        ctx.db_history_max_tokens.as_deref(), agent_id);
context_history::report_resolved(agent_id, &resolved);

// mika#1951 reste en amont et inchangé : l'appelant peut rétrécir ce tour,
// jamais l'élargir. Les deux asymétries composent dans le même sens.
let effective_scope = if params.session_isolated { Session } else { resolved.scope };
```

`truncate_history_to_token_budget` lit `resolved.max_tokens`.
`build_context_window_fields` reçoit `effective_scope` — c'est déjà le cas
aujourd'hui, l'appel ne bouge pas, seule sa source change.

### B5 — l'avertissement d'écriture (`crates/mika-cli/src/commands/config.rs`)

Dans le bras `ConfigBackend::Database` de `run_set`, après la validation : si la
clé est `context_history_scope`, la valeur `session`, et que l'agent ne déclare
pas `[session] singleton = true`, imprimer sur stderr que chaque message inbound
de cet agent ouvre une session neuve, donc que ce réglage supprime l'historique
conversationnel et non 90 % de lui — avec le nom de la sonde.

Avertit et ne refuse pas : un agent piloté en CLI ou en A2A avec des
`--session-id` stables est une population légitime, et refuser ferait de ce
ticket un interdit là où il doit être un levier.

### B6 — documentation

`configuration.md` (les deux copies, sync par script — le job CI `docs-sync` le
gate) + une section `CLAUDE.md` portant les surfaces, les sondes et leurs
haltes.

---

## Verification contract

### Comportemental

- **V1 — table de vérité du scope.** Les quatre croisements
  (identité ∈ {agent, session}) × (DB ∈ {agent, session}) rendent
  `session` sauf `(agent, agent)`, et `scope_source` nomme la porte qui a
  décidé.
- **V2 — contrôle négatif de V1.** Sans la cascade (identité seule), le
  croisement `(session, agent)` rend `agent`. Sans ce test, V1 pourrait être vert
  parce que le résolveur rend une constante.
- **V3 — `max_tokens` prend le min**, `None` valant l'infini : `(None, 8000) →
  8000`, `(8000, 12000) → 8000`, `(8000, none) → 8000`, `(None, none) → None`.
- **V4 — le défaut est byte-identique.** Un agent sans clé DB résout exactement
  le couple qu'il résout aujourd'hui, sur les deux champs, pour les cinq formes
  d'identité du dépôt (défaut, famille, mika-dev, mika-qa, mika-arch).
- **V5 — l'instrument ne ment pas.** Un agent déclarant `agent` avec
  `context_history_scope=session` en DB produit un `context_window_assembled`
  portant `history_scope: "session"` et `distinct_sessions: 1` ; le contrôle
  négatif (sans la clé) produit `agent` et `> 1`. Modèle :
  `tests/eval/test_context_scope_observability_2305.rs`, qui porte déjà
  exactement ce couple.
- **V6 — l'élargissement est refusé ET dit.** `(session, agent)` émet
  `context_history_widening_refused` et résout `session`.
- **V7 — `0` est refusé à la porte** (`validate_config_value`), `none` et `500`
  acceptés, `499` refusé.
- **V8 — fail-open sur une DB illisible** : le tour se déroule sur le déclaré.

### Structurel

- **D1** — `mika2305_the_scope_has_a_single_decisional_reader` amendé (voir
  *Fire-Disposition*).
- **D2** — `mika2425_source_names_are_a_wire_format` : égalités littérales sur
  les trois valeurs de `scope_source` / `max_tokens_source`. Elles atterrissent
  dans un champ que l'opérateur agrège ; deux orthographes couperaient une
  population en deux sans le dire (motif `mika2131_filter_names_are_a_wire_format`).
- **D3** — `mika2425_identity_context_history_has_a_single_reader` : scan de
  source (`ProductionScanner` + `source_guard::mask_test_regions`) refusant toute
  lecture de `identity.context.history` hors du résolveur. **Allowlist livrée
  vide.** Mesuré à HEAD : un seul lecteur de production
  (`agent_loop/mod.rs:4896`).
- **D3b — contrôle de bonne foi de D3.** Le prédicat doit rougir sur une lecture
  ajoutée ailleurs et rester muet sur les trois formes qui nomment le chemin
  sans le lire. Sans lui, D3 peut être vert parce qu'il ne regarde rien — le mode
  de panne exact que D3 existe pour rendre visible.
- **D4** — `mika2425_the_db_half_is_absent_from_the_tool_surface` : assertion que
  les deux clés ne sont **pas** dans `SETTABLE_CONFIG_KEYS`. Le refus 2 est une
  décision, pas un oubli ; sans cette assertion, un ajout ultérieur serait
  invisible.

---

## Fire-Disposition

Ce plan livre quatre détecteurs (D1–D4) et un contrôle de bonne foi (D3b).
**Option retenue : (a) — exception nommée en allowlist, livrée VIDE.**

### D3 — allowlist vide, et elle le reste

`CONTEXT_HISTORY_READERS_ALLOWED: &[&str] = &[]`. À HEAD il existe **un seul**
lecteur de production de `identity.context.history` (`agent_loop/mod.rs:4896`,
mesuré par `grep -rn "context\.history" crates/mika-agent/src/ crates/mika-cli/src/`),
et ce site **devient** le résolveur. Il n'y a donc aucune violation préexistante
à excepter.

**La résolution quand D3 tire est de retirer le second lecteur, jamais de
l'allowlister** — même contrat que `ACTOR_READING_PREDICATES_ALLOWED`
(mika#2323) et que le scan de `LlmUsage::accumulate` (mika#1883). Une assertion
auto-nettoyante accompagne la constante : elle rougit si l'allowlist cesse d'être
vide sans qu'un ticket de suivi soit nommé à côté de l'entrée.

### D1 — amendement raisonné d'un détecteur existant, pas une exemption

`mika2305_the_scope_has_a_single_decisional_reader` porte aujourd'hui deux
assertions : **exactement 2 sites** lecteurs de `HistoryScope`, et **tous dans
`agent_loop/mod.rs`**. B2 ajoute un troisième site (le résolveur, qui compare
deux scopes) et le place dans un module frère.

**Ce n'est pas une violation à excepter : c'est l'invariant à ré-énoncer.** Ce
que la garde protège est écrit dans son propre message — *« two answers to
"which rows may this window draw from?" can drift apart without breaking
anything visible »*. Le résolveur n'est pas une seconde réponse : il **produit**
la réponse unique que le site décisionnel consomme, et U3 garantit que ce site ne
lit plus rien d'autre.

L'amendement, en deux gestes tous deux grep-visibles :

1. Le compte passe de `2` à `3`, et le message nomme le troisième site
   (`context_history::resolve`) et sa fonction, comme il nomme déjà les deux
   autres.
2. La seconde assertion passe de `agent_loop/mod.rs` à **`agent_loop/`** —
   l'invariant écrit est « les lecteurs vivent dans la boucle », et
   `agent_loop/context_history.rs` est dans la boucle. La contrainte de chemin
   n'est pas relâchée au-delà du répertoire.

**Contournement explicitement refusé.** Le prédicat de D1 est positionnel : il
cherche `HistoryScope::` à gauche d'un `=>`. Écrire la comparaison en
`matches!(scope, HistoryScope::Session)` échapperait au scan sans changer la
sémantique — donc ferait passer la garde sans la satisfaire. Le résolveur écrit
un `match` franc et le compte est mis à jour. Un relecteur qui trouve cette forme
plus verbeuse doit lire cette ligne avant de la « simplifier ».

### Pré-vol obligatoire, avec sa halte

Poser D1–D4 et D3b **avant** B2–B5, et les lancer :

```bash
cargo test -p mika-agent mika2425_ mika2305_the_scope
```

- D2, D4 doivent être **verts** (ils n'assertent que sur du code neuf).
- D3 doit être **vert avec allowlist vide** contre l'arbre à HEAD.
- D1 doit être **rouge** de façon attendue une fois B2 posé, et **vert** après
  l'amendement décrit ci-dessus.

**Halte.** Si D3 tire au jour 1, c'est qu'un second lecteur de
`identity.context.history` existe que cette relecture n'a pas vu :
**s'arrêter, le nommer, et remonter** — ne pas l'allowlister, et ne pas écrire
la cascade tant que le nombre de lecteurs n'est pas établi, puisque toute la
conception repose sur ce nombre valant un.

---

## Surfaces opérateur

```bash
# Quel couple est réellement en vigueur pour ce tenant, et par quelle porte ?
grep context_history_resolved "$MIKA_SPIRIT_LOG_FILE" \
  | jq '{agent_id, scope, scope_source, max_tokens, max_tokens_source, session_minting}'

# Un élargissement a-t-il été tenté ? (régime attendu : vide)
grep context_history_widening_refused "$MIKA_SPIRIT_LOG_FILE"

# La fenêtre réellement assemblée — instrument mika#2305, INCHANGÉ
grep context_window_assembled "$MIKA_SPIRIT_LOG_FILE" \
  | jq 'select(.agent_id == "<tenant>")
        | {history_scope, distinct_sessions, message_count, history_bytes}'
```

`session_minting` ∈ `{singleton, per_message}` est le champ qui rend R3 lisible
sans lire le code : c'est lui qui dit si « session » veut dire *une conversation*
ou *un message* pour cet agent.

**`context_window_assembled` n'est pas élargi** — il porte déjà `history_scope`,
`distinct_sessions`, `message_count` et `history_bytes` depuis mika#2295/#2305,
c'est-à-dire exactement les quatre nombres dont la vérification a besoin.
Inventer un second instrument dupliquerait la mesure et créerait deux vérités.

### Table de lecture

| `scope` | `scope_source` | lecture |
|---|---|---|
| `agent` | `default` | **nominal** — rien n'est posé, comportement d'aujourd'hui |
| `session` | `identity` | un rôle déclare la borne (mika-arch) |
| `session` | `customer_config` | l'opérateur a posé le rétrécissement — **lire `session_minting` avant de conclure quoi que ce soit sur le gain** |
| `session` | `identity` **avec** un `agent` en DB | un élargissement a été tenté et refusé — voir le WARN |

---

## Sondes post-déploiement, et leurs haltes

**S1 — le défaut n'a pas bougé (immédiat).** Avant toute écriture de clé, les
quatre agents bien connus et le tenant local produisent un
`context_history_resolved` dont le couple est identique à ce que le dépôt
déclare : mika-arch `session`/`8000` en `identity`, les autres `agent`/`null` en
`default`. *Halte* — un couple qui diffère signifie que la cascade a changé une
valeur en vigueur, ce que U8 interdit : **désarmer avant tout autre diagnostic.**

**S2 — l'établissement de R3, préalable à toute décision de réglage.** Sur un
tenant Telegram représentatif, `grep context_window_assembled | jq
'{message_count, distinct_sessions}'` sur 24 h de trafic réel. *Si
`distinct_sessions` vaut 1 et `message_count` vaut 1 sur la quasi-totalité des
lignes*, alors une session **est** un message pour ce tenant et
`scope = session` n'a rien à y retirer — il n'y a pas de gain, seulement une
perte. **Halte : ne poser la clé sur aucun tenant de cette forme**, et ouvrir le
suivi « scope `channel` » avec ce compte en précondition. La mesure L8 ne
dispense pas de cette sonde : elle a porté sur une population dont ce plan
établit qu'elle n'est pas celle-là.

**S3 — le gain, si S2 l'autorise.** Sur un tenant dont les sessions portent
plusieurs messages, comparer `history_bytes` avant et après. Attendu :
l'ordre de grandeur annoncé par L8. *Halte* — un `history_scope: "session"` avec
`distinct_sessions > 1` signifie que le filtre ne filtre pas : la fuite est sous
`rebuild_context` et non dans le réglage, et c'est la halte que mika#2305 a déjà
écrite pour cet instrument.

**S4 — contrôle négatif du refus.** `context_history_widening_refused` doit
rester vide. Une occurrence est un opérateur qui croit avoir élargi et ne l'a pas
fait ; la ligne nomme l'agent et le geste qui marche.

**S5 — halte de déploiement.** Aucune ligne `context_history_resolved` alors que
le tenant a tourné ⇒ le binaire servi est antérieur au correctif (classe
mika#2340) : **établir le déploiement avant toute conclusion sur les valeurs.**

---

## Ce que ce travail n'achète PAS

- **Aucun gain de tokens par lui-même.** Il livre le levier ; le gain dépend
  d'une décision de réglage que S2 conditionne. Un plan qui annoncerait la
  réduction de 90 % comme livrée transporterait la mesure L8 sur une population
  que R3 établit comme différente.
- **Aucune protection contre `mika agents reprovision`**, qui reste le geste qui
  réécrit un `identity.toml` de tenant. Il ne touche pas la DB, donc le
  rétrécissement posé lui survit — ce qui est un effet du choix de site, pas une
  garantie ajoutée.
- **Aucune borne sur la mémoire agent-scoped.** `search_memory`, la core memory,
  les faits et le résumé conversationnel traversent toutes les sessions par
  conception. Le ticket parle d'isolation de session pour l'**historique** ; ces
  canaux-là sont un autre axe, déjà nommé dans le dépôt pour mika-arch.
- **Aucune surface console.** `mika-cloud` est hors de ce workspace.

---

## Definition of Done

- [ ] B1–B6 implémentés ; `cargo build`, `cargo clippy`, `cargo fmt --check`
      propres.
- [ ] V1–V8 verts ; D1–D4 et D3b verts après l'amendement D1.
- [ ] Le pré-vol de *Fire-Disposition* a été exécuté et son résultat consigné
      dans le corps de PR (D3 vert à allowlist vide, D1 rouge-puis-vert).
- [ ] S1 exécuté avant merge sur l'arbre de test : le défaut est byte-identique.
- [ ] `docs/configuration.md` et `crates/mika-agent/docs/configuration.md`
      synchronisés (job CI `docs-sync`).
- [ ] Section `CLAUDE.md` portant les surfaces, la table de lecture, les cinq
      sondes et leurs haltes.
- [ ] Les deux suivis (exposition au modèle ; scope `channel`) ouverts avec leur
      précondition écrite, ou nommés dans le corps de PR avec `Tracked in:`.
- [ ] **U10 exécuté avant merge** : corps de mika#2425 édité (encadré daté en
      tête, texte d'origine conservé dessous), avis d'édition posté en
      commentaire, et corps de PR nommant la rectification avec le lien du
      commentaire. *Cette case est un gate, pas une formalité* — le mécanisme
      livré ne correspond pas à la lettre du ticket, et un merge sans elle laisse
      un contrat périmé faisant autorité sur la surface la plus consultée.

## Acceptance criteria

- **AC1** — Un opérateur pose et annule le réglage par tenant sans redémarrer
  mika-spirit : `mika config set context_history_scope session --agent <t>` prend
  effet au tour suivant, et `… agent` l'annule.
- **AC2** — Le réglage survit à un redémarrage de mika-spirit et à une
  réconciliation des sections code-owned, pour un agent bien connu **comme** pour
  un tenant.
- **AC3** — Le défaut reste `scope = Agent`, `max_tokens = None` : aucun agent ne
  change de comportement sans qu'une clé ait été posée pour lui (V4, S1).
- **AC4** — La moitié DB ne peut **jamais** élargir la fenêtre au-delà de ce que
  l'identité déclare ; une tentative est refusée et journalisée (V6, S4).
- **AC5** — Le couple en vigueur et sa provenance sont lisibles à l'instrument,
  par agent, sans lire la base ni le code (`context_history_resolved`).
- **AC6** — `CODE_OWNED_IDENTITY_SECTIONS` et `reconcile_well_known_identity`
  sont inchangés ; mika-arch continue de résoudre `session`/`8000` depuis son
  identité.
- **AC7** — Poser `scope = session` sur un agent non singleton produit un
  avertissement nommant la conséquence (R3) et la sonde, sans refuser le geste.
- **AC8** — Les deux clés ne sont pas atteignables par le modèle via
  `set_config`, et cette absence est tenue par une assertion (D4).
- **AC9** — **La divergence avec le ticket est ratifiée sur le ticket, pas
  seulement dans le plan.** Le corps de mika#2425 porte, au merge, un encadré daté
  constatant que ses exigences 1 et 2 sont remplacées par le mécanisme
  `customer_config` et nommant les mesures qui l'imposent ; son texte d'origine
  est conservé sous l'encadré ; un commentaire d'avis d'édition existe sur le fil.
  Vérification : lire le corps du ticket et le fil — un lecteur qui n'ouvre que le
  ticket doit repartir avec le bon contrat. **Non délégable au corps de PR seul**
  (une PR se referme et ne s'indexe pas comme le ticket) et non délégable au plan
  seul (rien ne mène du ticket au plan avant que le callout `Plan:` ne soit posé).
  Citation : mika#2169, mika#2158.

---

## Revision history

- **rev 2 (2026-09-22)** : addressed F1 en prescrivant la rectification du corps
  de mika#2425 — le plan réfutait les exigences 1 et 2 du ticket sans qu'aucune
  exigence, case de DoD ni AC ne demande de mettre le ticket à jour, laissant la
  divergence non ratifiée pour tout lecteur aval. Quatre gestes : (a) **U10**
  ajouté aux *Requirements*, prescrivant les trois gestes de la convention
  issue-as-versioned-contract (corps édité avec encadré daté et texte d'origine
  conservé, avis d'édition en commentaire, annotation de clôture dans le plan) et
  notant que l'exigence 3 du ticket est satisfaite telle quelle par U8 ; (b)
  **AC9** ajoutée, gatant la ratification sur le ticket lui-même et nommant
  pourquoi ni le corps de PR ni le plan ne peuvent en tenir lieu ; (c) une **case
  de DoD** faisant de U10 un gate avant merge ; (d) l'**annotation de clôture** —
  encadré daté en tête du plan, plus deux renvois courts vers U10 depuis le refus 1
  et depuis le point 1 du § *Pourquoi « rétrécir seulement »*, aux deux endroits où
  le texte dissolvait une exigence sans dire qu'elle restait écrite ailleurs.
  Citations préservées : mika#2169 / mika#2158 (convention), review-guide
  § divergence plan ↔ spec. Aucune AC affaiblie, aucun mécanisme modifié : R1–R3,
  les trois refus, U1–U9, V1–V8, D1–D4/D3b, les surfaces et les sondes S1–S5 sont
  inchangés.
