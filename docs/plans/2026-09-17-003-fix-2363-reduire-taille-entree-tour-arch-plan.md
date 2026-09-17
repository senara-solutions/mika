# Plan — mika#2363 : réduire la taille d'entrée d'un tour mika-arch

**Ticket:** mika issue#2363

- **Ticket :** `mika issue#2363`
- **Type :** fix
- **Priorité :** p2 — substrate ; lever de fond de `mika issue#2362` D2, en amont de `mika issue#2361`
- **Repos touchés :** `mika` (mika-agent, mika-cli, mika-a2a, `skills/bundled/`)

---

## Le besoin, et ce que la lecture du code y déplace

Un tour mika-arch qui heurte le plafond HTTP ne rend pas de verdict, le groom échoue, le
budget de re-drive s'épuise et le ticket cale. Le ticket demande de **réduire la taille
d'entrée d'un tour arch** pour que l'appel finisse sous le plafond.

Le ticket décrit correctement le symptôme et sa lignée. Six lectures du code le
**déplacent**, et ce sont elles qui décident la conception. Chacune est vérifiable dans
l'arbre, avec sa référence.

### T1 — Trois prompts de skill arch sont injectés à chaque tour ; un seul sert

Les trois skills de mika-arch déclarent `always_on = true` :

| skill | `skills/bundled/<name>/system_prompt.md` |
|---|---|
| `mika-arch-groom-ticket` | **16 269 o** |
| `mika-arch-second-review` | **14 290 o** |
| `mika-arch-groom-milestone` | **9 239 o** |
| **total injecté à chaque tour** | **39 798 o** |

`skills::matcher::match_skills` (`matcher.rs:130`) retient toute skill `always_on` sans
condition, et `inject_skills_and_resolve_tools` concatène le `prompt_snippet` de chacune
dans le system prompt (`agent_loop/mod.rs:3565`). L'allowlist d'identité de mika-arch
(`MIKA_ARCH_SKILL_ALLOWLIST`, `well_known_agents.rs:347`) contient exactement ces trois-là,
donc les trois passent la phase -1 et les trois sont injectées.

Un tour `_arch_ask` exécute **une** passe : première passe *ou* seconde passe *ou* milestone.
**23 529 octets — 59 % de la portion skill du system prompt — décrivent à chaque tour deux
tâches que le tour ne fait pas.** C'est une constante, déterministe, sans aucun coût de
justesse : on retire des instructions pour un travail qui n'a pas lieu.

### T2 — `--enable-skill` n'atteint pas le moteur, et `always_on` est la seule raison pour laquelle le bon prompt est là

`_arch_ask` (`dispatch-lib.sh:4516`) passe `--enable-skill "$skill"`. Depuis mika#1727,
`mika ask` est un client mince et le drapeau est validé localement puis **abandonné**.
`crates/mika-cli/src/commands/ask.rs:315-318` l'écrit mot pour mot :

> `--enable-skill` / `--disable-skill` / `--model` configure the *local* registry/LLM, which
> is no longer the execution surface. Their arg-level validation is preserved, but they do
> not yet reach spirit — that needs a config channel threaded through `message/send`.

Conséquence, et c'est un piège : **on ne peut pas se contenter de passer les trois skills à
`always_on = false`.** Le mot-clé ne les rattraperait pas (T2b), donc le tour arch
n'injecterait **aucun** prompt arch — et avec lui disparaîtraient en silence le contrat de
sortie (`required_suffix_lines`), la liste de findings (`required_finding_list_prefixes`) et
l'attestation d'ancrage (mika#2037). Le pipeline déclare déjà la passe qu'il veut ; c'est la
déclaration qui se perd en route. **Le correctif rétablit le canal, il ne retire pas le
drapeau.**

### T2b — Le routage par mots-clés ne peut pas remplacer le canal

`mika-arch-groom-ticket` déclare le mot-clé `"plan review"` ; `mika-arch-second-review`
déclare `"second pass"`. Les deux tournures apparaissent naturellement dans le corps d'un
plan — celui-ci en est un exemple. Router sur le contenu du plan chargerait la mauvaise
skill, ou les deux. Le contenu revu ne peut pas être le sélecteur de la skill qui le revoit.

### T3 — Tronquer le plan est refusé par un contrat déjà en place (mika#2037)

Le levier littéral du ticket — « résumer ou tronquer le plan + contexte review au-delà d'un
seuil » — porte sur **l'objet même de la revue**, pas sur son contexte. Les trois `skill.toml`
arch déclarent :

```toml
required_review_anchor_prefixes = ["A1:", …]
review_anchor_min_count = 3
review_anchor_min_quote_chars = 40
```

Une disposition `READY` / `GROOMED` doit citer **verbatim** le brief revu, à trois positions
distinctes, 40 caractères minimum. Un brief tronqué par la queue laisse les trois ancres se
regrouper dans la tête conservée, et la garde ne peut plus distinguer *une revue du tout*
d'*une revue de la tête*. Le verdict garde la même forme et perd son fondement — pendant
qu'il commande la porte de dispatch (mika#919).

**Tronquer l'objet d'une revue n'est pas une optimisation de taille, c'est une fabrication de
verdict.** Le plan ne tronque donc rien du plan. Voir § *Ce que ce plan ne fait PAS*.

### T4 — La cause n'est pas mesurée, et les trois instruments existent déjà

Le ticket affirme « la différence = taille du brief ». C'est une hypothèse, pas une mesure.
Trois événements **non gated** portent déjà la réponse et aucun n'a été lu :

| événement | champs utiles | ticket |
|---|---|---|
| `system_prompt_assembled` | `total_bytes`, `per_skill_bytes`, `active_skill_count`, `tool_count` | mika#1217 |
| `context_window_assembled` | `history_bytes`, `user_message_bytes`, `tool_defs_bytes`, `truncated_messages` | mika#2295 |
| `turn_usage` | `request_bytes`, `system_prompt_bytes`, `latency_ms`, `status`, **`step`** | mika#1889 + mika#2331 AC1 |

`request_bytes` sur la branche d'erreur a été livré **la veille** (mika#2331, mergé
2026-09-16, `5e971013`) précisément pour cette classe, avec sa table de décision à quatre
branches — dont la branche 4 est *« `request_bytes` indiscernable entre tours sains et tours
en échec → ne pas borner ; rouvrir le diagnostic »*. Ce plan ouvre donc par la lecture, pas
par le remède.

### T5 — `step` sépare deux causes que « taille du brief » confond

Une coupure au **step 0** accuse le brief. Une coupure au **step ≥ 2** accuse ce que le tour
a accumulé : les trois skills arch déclarent `required_fetches_for_quoted_resources = true`,
donc l'architecte doit aller chercher chaque ressource que le plan cite, et `gh_read`
`file_view` plafonne à **1 MiB par résultat** — chacun restant dans la liste de messages pour
tous les steps suivants. Un plan plus gros cite plus de ressources, donc un step **tardif**
franchit le plafond. C'est un autre défaut, avec un autre remède, et `turn_usage.step` les
sépare aujourd'hui sans une ligne de code.

### T5b — Aucune borne n'existe sur la taille totale d'une requête, et le chemin arch est le moins gardé des trois

`LlmRequest::payload_bytes()` (`mika-common/src/llm/types.rs:48`) est **purement mesuré** :
ses quatre appelants l'écrivent dans un log ou une colonne, aucun ne le compare à un seuil.
Il n'existe aucun compteur de jetons réel dans l'arbre — `CHARS_PER_TOKEN_ESTIMATE = 4`
(`prompt.rs:114`) ne sert qu'aux deux caps *locaux* (résumé, historique). Aucun rail provider
ne teste la taille de l'entrée avant l'envoi.

Et les gardes d'entrée sont asymétriques entre chemins :

| chemin | borne applicative |
|---|---|
| HTTP `/message` (gateway → agent) | **50 000 o**, rejet 400 (`server/handlers.rs:158`) |
| callback `/tasks/{id}/result` | 100 000 chars, rejet 400 ; puis tronqué à `CALLBACK_RESULT_MAX_BYTES = 10 240` à l'injection (`planning/policy.rs:48`) |
| **`/a2a/{agent}` `message/send` — le chemin de `_arch_ask`** | **aucune**. Seul plafond : le corps JSON-RPC à 2 MiB (`server/mod.rs:337`). `extract_text_from_parts` (`a2a_db.rs:70`) concatène sans cap. |

Deux composants du system prompt sont par ailleurs non bornés au rendu : `soul.md`
(`write_soul_section`, `prompt.rs:920`) et la core memory
(`write_core_memory_section`, `prompt.rs:1114-1128` — la boucle n'a ni cap ni troncature ; la
limite de ~2 500 jetons est tenue à l'écriture, pas au rendu). Pour mika-arch ce sont des
constantes modestes et le résumé de conversation vaut **zéro** (`[context.summary] inject =
false`, `well_known_agents.rs:405-406`) — mais B0 doit les attribuer plutôt que les supposer,
et c'est exactement ce que `system_prompt_assembled.total_bytes` moins la somme de
`per_skill_bytes` donne en une soustraction.

### T6 — La géométrie annoncée n'est peut-être pas celle en vigueur

`MIKA_ARCH_CONFIG` (`well_known_agents.rs:1477`) pose `llm_http_timeout_secs = 240` et
`agent_total_timeout_secs = 900`. Le ticket mesure des coupures à **300 s** et nomme 300/600
puis 300/660. Ce ne sont pas les mêmes nombres. `llm_budget_resolved` (mika#2293) dit lequel
est en vigueur **et par quelle porte** (`http_source`). Si une variable de service écrase le
`config.toml` per-agent, une part du comportement observé est un fait de configuration — et
réduire le brief se verrait créditer d'une correction qui appartient ailleurs.

---

## Requirements

- **R1** — Un tour mika-arch n'injecte que le prompt de la skill correspondant à la passe
  qu'il exécute. Les deux autres ne sont pas injectées.
- **R2** — Le mécanisme est **strictement soustractif** : il ne peut activer aucune skill
  qu'un tour aurait sinon laissée inactive, et ne crée donc aucune surface de privilège sur
  `/a2a/{agent}`.
- **R3** — Un appelant qui ne déclare rien obtient le comportement d'aujourd'hui, octet pour
  octet. Aucun tour existant (`/mika-ask-arch`, re-prompt correctif mika#1823, webhook) ne
  change de forme sans qu'on l'ait écrit.
- **R4** — La restriction est **per-tour** : elle ne mute jamais le `SkillRegistry` mis en
  cache sur `AgentState`, que deux tours concurrents partagent.
- **R5** — Aucun contenu revu n'est tronqué, résumé, ni élidé. La fenêtre d'historique
  (mika#2295/#2330) et le message utilisateur restent exactement ce qu'ils sont.
- **R6** — La réduction est **mesurable après coup sans nouvelle instrumentation** :
  `system_prompt_assembled.per_skill_bytes` et `active_skill_count` l'attestent.
- **R7** — Le piège T2 est épinglé : une bascule des trois `always_on` à `false` sans le canal
  doit faire rougir un test, pas partir en silence.
- **R8** — La décision de borner quoi que ce soit d'autre est **conditionnée à la mesure B0**,
  avec ses branches et ses haltes écrites.

---

## Conception

### B0 — Lire les trois instruments (aucun code)

Sur au moins **20 tours mika-arch postérieurs au déploiement de mika#2331**, dont au moins
5 coupés.

```sh
# 1. De quoi le tour est fait, et si la triple injection est bien là.
grep system_prompt_assembled $MIKA_SPIRIT_LOG_FILE \
  | jq 'select(.agent_id == "mika-arch") | {total_bytes, active_skill_count, per_skill_bytes, tool_count}'

# 2. Le partage brief / historique / outils.
grep context_window_assembled $MIKA_SPIRIT_LOG_FILE \
  | jq 'select(.agent_id == "mika-arch") | {user_message_bytes, history_bytes, tool_defs_bytes, truncated_messages, message_count}'

# 3. Le couple (taille, latence, step, issue) — la corrélation elle-même.
grep turn_usage $MIKA_SPIRIT_LOG_FILE \
  | jq 'select(.agent_id == "mika-arch") | {step, status, latency_ms, request_bytes, system_prompt_bytes}'

# 4. La géométrie réellement en vigueur (T6).
grep llm_budget_resolved $MIKA_SPIRIT_LOG_FILE \
  | jq 'select(.agent_id == "mika-arch") | {http_timeout_secs, agent_total_timeout_secs, max_attempts, http_source, total_source}'
```

**Attribution du reste du system prompt (T5b)** — `total_bytes` moins la somme des valeurs de
`per_skill_bytes` donne en une soustraction ce que pèsent `soul.md`, la core memory et le
corps constant. Si ce résidu est lui-même important, il se traite séparément ; B1 ne le touche
pas et ne prétend pas le toucher.

**Table de décision, avec ses haltes :**

| observation | lecture | suite |
|---|---|---|
| `http_source = process_env`, plafond ≠ 240 | la géométrie annoncée n'est pas en vigueur (T6) | **corriger l'environnement du service d'abord** ; re-mesurer avant de créditer quoi que ce soit à la taille |
| `active_skill_count = 3`, `per_skill_bytes` porte les trois | T1 confirmé en production | **B1 s'applique** |
| coupures concentrées au **step 0**, `request_bytes` des tours coupés nettement supérieur | le brief décide | B1, puis re-mesure ; le résidu relève de la taille du plan (§ *hors périmètre*) |
| coupures au **step ≥ 2** | ce sont les résultats d'outils accumulés (T5), pas le brief | **halte** : B1 reste bon à prendre mais ne referme pas le défaut — ouvrir le ticket « borner les résultats `gh_read` du tour arch » |
| `request_bytes` indiscernable entre tours sains et coupés | la corrélation du ticket est **infirmée** | **halte** : B1 reste bon à prendre (T1 ne dépend d'aucune corrélation), mais ne rien borner d'autre et rouvrir le diagnostic |
| aucune ligne `turn_usage` sur les tours coupés | le blocage est en amont de l'appel HTTP | **halte** : hors de cette lignée, voir mika#2342 |

**B1 ne dépend d'aucune branche.** Retirer 23,5 Ko d'instructions décrivant un travail qui
n'a pas lieu est juste quelle que soit la cause de la coupure. Ce que B0 décide, c'est si
autre chose doit suivre — et quoi.

### B1 — Un seul prompt de skill arch par tour

**Forme : un canal de restriction, strictement soustractif, sur `message/send`.**

1. **Protocole** — `message/send` accepte dans ses métadonnées de requête un champ
   `only_skills: Vec<String>` (absent par défaut). Même emplacement et même discipline que
   le `session_id` que mika#2070 y a déjà fait passer : une métadonnée facultative que le
   serveur peut refuser sans casser l'appel.

2. **Sémantique, et c'est elle qui tient R2** — quand `only_skills` est présent et non vide,
   le serveur applique `SkillRegistry::apply_transient_disable` (`skills/mod.rs:866`) à
   **toute skill du registre dont le nom n'est pas dans la liste**. Jamais
   `apply_transient_always_on`. Une skill nommée qui n'aurait pas été active reste inactive :
   le champ ne peut que retirer. Un nom inconnu est un no-op journalisé, jamais une erreur —
   un appelant d'une version antérieure ne doit pas pouvoir faire échouer un tour.

   *Pourquoi seulement la moitié soustractive.* La moitié additive
   (`--enable-skill` → `apply_transient_always_on`) ouvrirait à tout appelant de
   `/a2a/{agent}` la capacité de forcer une skill de l'agent en `always_on`. Le registre est
   déjà borné par l'allowlist d'identité, donc le risque est contenu — mais il est non nul,
   et **il n'achète rien ici** : la skill dont `_arch_ask` a besoin est déjà `always_on`.
   Elle reste différée, avec sa raison.

3. **Per-tour, jamais sur le cache (R4)** — `server/a2a.rs:140-163` résout un
   `Arc<SkillRegistry>` partagé sur `AgentState`. La restriction produit un clone per-tour
   (`Arc::new(registry_restreint)`) qui n'est **jamais** réécrit dans
   `*agent_state.skills.lock()`. Si `SkillRegistry` ne dérive pas `Clone`, l'ajouter est
   préférable à une vue filtrée : le registre est déjà un agrégat de données possédées, et
   une seconde représentation serait un lecteur de plus à faire diverger.

4. **CLI** — `mika ask --only-skill <name>` (répétable), transmis dans les métadonnées
   `message/send`. Exclusif avec `--enable-skill` / `--disable-skill` dans la même
   invocation : trois sémantiques de sélection sur un tour est un état dont personne ne veut
   déboguer la composition. Le refus est une erreur d'argument, avant tout appel.

5. **Appelant** — `_arch_ask` ajoute `--only-skill "$skill"` à `args`. Le `--enable-skill`
   existant est **conservé** : il reste la déclaration correcte côté local, et son abandon en
   transit est le défaut mika#1727 que ce plan ne referme qu'à moitié.

   `_arch_ask` ne nomme **que la skill qu'il veut**, jamais la liste de ses sœurs — le
   vocabulaire des trois skills arch vit dans `MIKA_ARCH_SKILL_ALLOWLIST` et ne doit pas être
   réécrit dans du shell.

**Effet mesuré sur le system prompt arch** (T1) :

| passe | avant | après | delta |
|---|---|---|---|
| première passe (`groom-ticket`) | 39 798 o | 16 269 o | **−23 529 o** |
| seconde passe (`second-review`) | 39 798 o | 14 290 o | **−25 508 o** |
| milestone (`groom-milestone`) | 39 798 o | 9 239 o | **−30 559 o** |

**Ce que ça ne prétend pas.** ~23 Ko ≈ 6 000 jetons. Sur un tour dont l'entrée mesurée en B0
se compte en dizaines de milliers de jetons, c'est une réduction **réelle, permanente et sans
contrepartie**, pas une garantie de passer sous 200 s. La cible du ticket est une cible, et
B0 dit ce qu'il reste à faire pour l'atteindre.

### B2 — Ce que B1 remet en état au passage, et qu'il faut dire

L'union des `required_suffix_lines` des skills `AlwaysOn` (`collect_required_suffix_lines`)
fait aujourd'hui, sur **tout** tour arch, un ensemble accepté de cinq lignes :
`Disposition: READY|ITERATE|ESCALATE` **et** `Verdict: GROOMED|ESCALATE`. Une première passe
peut donc satisfaire la garde de suffixe en émettant `Verdict: GROOMED` — un contrat de
sortie que `_parse_disposition` ne lira pas, et qui remonte en `UNPARSED`.

B1 resserre cet ensemble au contrat de la passe réellement en cours. **C'est un
durcissement, donc un changement de comportement à surveiller** : un modèle qui s'appuyait
sur la tolérance se fera reprendre une fois par la garde (budget d'un re-prompt) au lieu de
passer. La sonde post-déploiement le regarde explicitement (§ Verification contract, V4).

### B3 — Épingler le piège T2

Un test dans `skills/` ou `well_known_agents.rs` asserte que les trois skills arch portent
`always_on = true`, **avec sa raison écrite dans le message d'échec** : tant que le canal
`only_skills` est le seul sélecteur et que les mots-clés sont inutilisables (T2b), passer ces
drapeaux à `false` supprime en silence tout contrat de revue arch.

Un test comportemental ne peut pas attraper cette classe : la régression ne rend aucune
décision fausse, elle rend le tour **muet de contrat**, et toutes les assertions existantes
restent vertes pendant que `_parse_disposition` renvoie `UNPARSED` sur chaque groom.

---

## Verification contract

- **V1 (unitaire, `skills/mod.rs`)** — `apply_transient_disable` appliqué au complément d'une
  liste `only_skills` laisse exactement les skills nommées ; une skill nommée mais désactivée
  en base n'est pas ressuscitée ; un nom inconnu est un no-op.
- **V2 (unitaire, protocole)** — `only_skills` absent ⇒ le registre per-tour est
  **identiquement** celui du cache (R3) ; `only_skills` vide ⇒ traité comme absent, jamais
  comme « aucune skill ».
- **V3 (intégration, `tests/eval/`)** — un tour A2A `only_skills = ["mika-arch-groom-ticket"]`
  sur un registre portant les trois skills arch produit un `system_prompt_assembled` avec
  `active_skill_count = 1` et un `per_skill_bytes` ne nommant que `mika-arch-groom-ticket`.
  Le même tour sans le champ en produit trois. **Les deux moitiés sont nécessaires** : la
  première seule passerait sur une implémentation qui n'injecte jamais rien.
- **V4 (intégration)** — sous `only_skills = ["mika-arch-groom-ticket"]`, l'ensemble accepté
  par la garde de suffixe est `Disposition: READY|ITERATE|ESCALATE` **seul** — `Verdict:`
  n'y est plus (B2).
- **V5 (concurrence, R4)** — après un tour restreint, le `SkillRegistry` de `AgentState`
  porte toujours ses trois skills. C'est l'assertion qui distingue un clone per-tour d'une
  mutation partagée, et rien d'autre ne la fait.
- **V6 (structurel, B3)** — les trois `always_on` arch sont épinglés, message d'échec
  explicatif.
- **V7 (shell, `test-dispatch-lib.sh`)** — `_arch_ask` émet `--only-skill <skill>` pour chacune
  des trois skills, et n'énumère jamais les deux sœurs.
- **V8 (post-déploiement, 48 h)** — sur mika-arch :
  1. `system_prompt_assembled | jq .active_skill_count` vaut **1** sur les tours de groom ;
  2. `total_bytes` baisse de ≈ 23 à 30 Ko selon la passe, et `turn_usage.system_prompt_bytes`
     bouge de la même quantité — **deux surfaces indépendantes qui doivent bouger ensemble ;
     si une seule bouge, c'est la mesure qui est en cause, pas le système** ;
  3. `turn_usage status=error latency_ms ≈ plafond` sur mika-arch : compter avant / après. Une
     baisse est le résultat attendu ; **zéro n'est pas promis** (§ B1, *ce que ça ne prétend
     pas*) ;
  4. `_parse_disposition` renvoyant `UNPARSED` : le compte ne doit **pas** monter. S'il monte,
     c'est B2 qui mord — désarmer par un retrait du `--only-skill` dans `_arch_ask` (un
     caractère de shell, aucun redéploiement du binaire) et traiter le contrat de sortie dans
     son propre ticket.

---

## Definition of Done

- `only_skills` traverse `message/send` et restreint le registre per-tour, strictement par
  soustraction.
- `mika ask --only-skill` existe, est répétable, et refuse la combinaison avec
  `--enable-skill` / `--disable-skill`.
- `_arch_ask` déclare la passe qu'il exécute.
- V1–V7 passent ; `cargo test`, `cargo clippy`, `cargo fmt --check` verts.
- `crates/mika-agent/CLAUDE.md` (§ Skills System, overrides transitoires) et
  `crates/mika-cli/CLAUDE.md` (§ `mika ask`) décrivent le canal, sa demi-portée soustractive
  et la raison pour laquelle la moitié additive reste différée.
- `docs/openapi/mika-spirit.yaml` décrit le champ de métadonnées.
- Le corps de PR porte la mesure B0 et le delta attendu par passe.

## Acceptance criteria

*(Le ticket ne porte pas de section `## Acceptance criteria` ; ceux-ci sont dérivés de ses
sections « Lever » et « Vérification » et des Requirements ci-dessus.)*

- **AC1** — Un tour mika-arch exécutant une passe n'injecte que le prompt de skill de cette
  passe. Attesté par `system_prompt_assembled.active_skill_count = 1` et par
  `per_skill_bytes` (V3, V8.1).
- **AC2** — La réduction de `system_prompt_bytes` sur un tour de première passe est d'au moins
  **20 000 octets** par rapport à la mesure B0 du même type de tour (V8.2).
- **AC3** — Aucun contenu revu n'est tronqué, résumé ni élidé : `user_message_bytes` et
  `history_bytes` sont inchangés à configuration égale, et le diff ne touche ni
  `truncate_history_to_token_budget` ni `ContextHistoryConfig` (R5).
- **AC4** — Le canal est strictement soustractif : aucun chemin de code n'appelle
  `apply_transient_always_on` depuis le serveur (V1, R2).
- **AC5** — Un appelant qui ne déclare rien obtient le comportement antérieur octet pour
  octet (V2, R3).
- **AC6** — Le registre partagé n'est jamais muté par un tour restreint (V5, R4).
- **AC7** — La mesure B0 est **portée dans le corps de la PR** : répartition
  brief / historique / system prompt / outils, distribution des `step` sur les tours coupés,
  et provenance de la géométrie (`http_source`). Une PR sans ces chiffres ne satisfait pas ce
  critère — la table de décision B0 ne peut pas être rejouée sans eux.
- **AC8** — Le piège T2 est épinglé par un test dont le message d'échec l'explique (V6, R7).
- **AC9** — Le désarmement est d'un geste sans redéploiement du binaire : retirer
  `--only-skill` de `_arch_ask` (V8.4).

---

## Risques et hors périmètre

### Risques

- **B2 est un durcissement.** Resserrer l'ensemble accepté par la garde de suffixe est
  correct et peut coûter un re-prompt là où la tolérance passait. Sonde V8.4, désarmement
  AC9.
- **Une surface de protocole de plus.** `only_skills` est une métadonnée que tout appelant
  authentifié de `/a2a/{agent}` peut poser. Elle ne peut que retirer (R2), donc le pire cas
  est un tour privé de ses skills — bruyant, pas silencieux : `active_skill_count = 0` sur
  `system_prompt_assembled`, et les gardes de contrat de sortie ne s'arment plus.
- **Le registre per-tour clone une structure partagée.** Coût mémoire par tour restreint,
  négligeable devant le prompt qu'il économise, mais réel ; à ne pas étendre aux chemins
  chauds sans mesure.

### Hors périmètre, délibérément

- **Tronquer, résumer ou élider le plan revu** — refusé par mika#2037 (T3). Le résidu de
  taille, si B0 le désigne, se traite **à la source** (un budget de taille sur la production
  de plans par `/ce:plan`), ce qui est une décision produit et mérite son ticket. Un plan de
  74 Ko n'est pas un défaut de transport.
- **Borner les résultats `gh_read` accumulés dans le tour** (T5) — ticket propre, ouvert si
  et seulement si B0 place les coupures au step ≥ 2.
- **La moitié additive du canal** (`--enable-skill` vers spirit) — différée avec sa raison
  (B1.2). mika#1727 reste à moitié ouvert et le dit.
- **La géométrie de mika-arch** (plafond / enveloppe) — si B0 montre que `240/900` est écrasé
  par une variable de service, le remède est un geste d'environnement (mika#2293), pas une
  ligne de ce plan. Ce plan réduit la cause ; il ne touche aucune valeur de budget.
- **Une borne agrégée sur la requête totale** — il n'en existe aucune dans l'arbre (T5b) et
  ce plan n'en ajoute pas. Une borne globale sans compteur de jetons réel serait une
  troncature aveugle, donc la classe T3 généralisée à tous les agents. Si elle doit exister,
  c'est avec un vrai compteur, un ordre d'éviction explicite, et son propre ticket.
- **Les composants non bornés partagés** (`soul.md`, core memory au rendu) — réels (T5b), mais
  modestes pour mika-arch et communs à tous les agents : ils ne se traitent pas dans un ticket
  d'agent.
- **La cause fournisseur des latences** — hors de portée ici, comme dans mika#2189 et
  mika#2342.
