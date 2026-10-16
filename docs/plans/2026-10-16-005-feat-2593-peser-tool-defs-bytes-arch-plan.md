# mika#2593 — Peser `tool_defs_bytes` devant le plan, et prendre la réduction non destructive qui existe

**Ticket :** senara-solutions/mika#2593
**Type :** feat (agent-core)
**Suivi de :** mika#2474 § 9 (*Hors périmètre*)
**Vol :** 1 de 2 — `Tracked in:` au § 8

---

## 1. Contexte : la précondition est remplie, et elle a été mesurée deux fois

Le ticket naît de la DoD de mika#2474, qui a pesé le brief d'architecte composant
par composant et conclu que *le plan est le levier*. Le quatrième composant — les
**définitions d'outils** — n'avait jamais été pesé. Le ticket posait une
précondition écrite : `tool_defs_bytes` doit être **non négligeable devant
`user_message_bytes`**, sinon le ticket se ferme sans travail.

L'opérateur a relevé la mesure deux fois, le 2026-10-15 puis le 2026-10-16. Les
deux relevés concordent, et le second porte la population du seuil :

| population | n | `tool_defs_bytes` | `user_message_bytes` | rapport |
|---|---|---|---|---|
| `brief_size_overrun`, mika-arch, 2026-10-16 | 3 | 21 175 (constant) | 50 068 – 52 237 | 0,41 |
| `context_window_assembled`, mika-arch, depuis le 07/10 | 75 | 21 175 (59 tours) / 20 947 (16) | 27 874 (moyenne) | 0,76 |

Deux propriétés de ce relevé commandent tout ce plan.

**Le poids est constant.** 21 175 octets, tour après tour, indépendamment du
ticket revu. Ce n'est pas une queue : c'est un **péage**, payé à chaque tour
d'architecte, y compris sur les questions ad-hoc de `/mika-ask-arch` et sur le
re-prompt correctif de la récupération UNPARSED (mika#1823), dont le message
utilisateur fait ~480 caractères et où les définitions d'outils pèsent alors
quarante fois la question.

**Le poids ne dépend d'aucune entrée.** Il dépend du **registre d'outils**, qui
est global : `server/mod.rs:979` construit un seul `Arc<ToolRegistry>`
(`default_tools()` + `management_tools_if_needed`) que **tous** les agents du
conteneur partagent. mika-arch n'a pas son registre ; il a un **filtre de
présentation** par-dessus celui de tout le monde.

La conséquence est la question de ce ticket, et elle n'est pas de taille : la
réduction, si elle existe, se prend sur ce filtre — donc sur ce que l'architecte
**peut faire**.

### 1.1 Le reliquat d'instrument que l'opérateur a nommé, et que ce vol ne prend pas

Les deux commentaires le disent : `context_window_assembled` ne porte pas
`system_prompt_bytes`, donc la formule de la précondition — qui le lit —
surévalue la part tant que le champ manque. C'est exact, c'est un défaut
d'instrument et non d'arbitrage, et il est **hors du périmètre de ce vol**
(§ 8) : il ne change aucune des deux décisions ci-dessous, parce que la grandeur
sur laquelle elles reposent est le **rapport à `user_message_bytes`**, que
l'opérateur a lui-même désigné comme « la grandeur à retenir », et qui ne lit pas
le prompt système.

---

## 2. Ce que mika-arch reçoit réellement — le compte exact, et sa source

Le relevé dit « 21 175 octets » ; il ne dit pas *de quoi*. Cette section le
reconstruit depuis le code, parce que l'arbitrage se fait outil par outil et
qu'un agrégat ne se soustrait pas (motif mika#2205).

### 2.1 La chaîne d'assemblage

`inject_skills_and_resolve_tools` (`crates/mika-agent/src/agent_loop/mod.rs:8978`) :

1. `tool_defs = tools.definitions().to_vec()` — **tout** le registre global ;
2. `apply_agent_tool_visibility(&mut tool_defs, disabled_tools)` (ligne 8996) —
   évince les noms de `Identity.tools.disabled`, soit
   `MIKA_ARCH_DISABLED_TOOLS` pour mika-arch ;
3. puis les outils de skill (`tools.json`), dédoublonnés.

`tool_defs_bytes` est `serde_json::to_string(tool_defs).len()`
(`agent_loop/mod.rs:9230`) — donc exactement le tableau servi après l'étape 3.

### 2.2 Le compte

| source | outils |
|---|---|
| `default_tools()` (`tools/mod.rs:1375`) | 37 |
| `management_tools_if_needed` (agents > 1, vrai ici) | 14 |
| **registre global** | **51** |
| − `MIKA_ARCH_DISABLED_TOOLS` (`well_known_agents.rs:391`) | − 23 |
| **builtins servis à mika-arch** | **28** |
| + `gh_read` (les trois `tools.json` d'arch, 970 o chacun, dédoublonnés) | + 1 |
| **tableau servi** | **29** pour 21 175 o, soit ~730 o/outil |

Les 28 builtins : `update_core_memory`, `store_fact`, `search_memory`,
`update_fact`, `list_reminders`, `promote_deferred_callback`, `get_task`,
`send_message`, `list_skills`, `get_config`, `get_active_llm`, `read_agent_file`,
`list_agent_files`, `list_scheduled_tasks`, `list_tasks`, `check_task`,
`resolve_issue_order`, `query_knowledge_graph`, `query_timeline`,
`get_session_messages`, `list_audit_events`, `search_tool_history`,
`record_served_content`, `check_already_served`, `list_agents`, `list_teams`,
`get_team_status`, `get_team_history`.

### 2.3 La mesure qui tranche, et elle est en dépôt

Les trois prompts servis à mika-arch — `mika-arch-groom-ticket`,
`mika-arch-second-review`, `mika-arch-groom-milestone` — ne nomment **que quatre
outils** :

```
$ grep -c "" skills/bundled/mika-arch-groom-ticket/system_prompt.md
276
$ grep -ho "gh_read\|query_knowledge_graph\|store_fact\|update_core_memory\|list_tasks\|get_task\|…" \
    skills/bundled/mika-arch-*/system_prompt.md | sort -u
gh_read
query_knowledge_graph
store_fact
update_core_memory
```

`gh_read` et `query_knowledge_graph` sont la lecture ; `store_fact` et
`update_core_memory` sont la mémoire agent-scoped épinglée par #811. **Vingt-cinq
des vingt-neuf outils servis ne sont nommés par aucune instruction du tour.** Ils
sont là sur l'hypothèse que le modèle improvisera.

Cette mesure ne suffit **pas** à autoriser une coupe : le prompt système de base
(`prompt.rs`) instruit l'usage d'outils génériquement, et un outil non nommé par
le prompt de skill peut rester atteignable par le contrat de base — c'est le cas
de `search_memory` (couche 3) ou de `read_agent_file`. Elle est la **seconde
condition** d'un critère qui en compte trois.

---

## 3. La décision : oui, il y a une réduction non destructive, et voici son critère

### 3.1 Le critère, en trois conjonctions

Un outil servi à mika-arch est **retirable sans perte de capacité** si et
seulement si les trois conditions tiennent ensemble :

**(C1) — sa famille d'écriture est entièrement hors d'atteinte.** Chaque outil
qui écrit la table que cette lecture lit est soit dans
`MIKA_ARCH_DISABLED_TOOLS`, soit absent du registre global. Une lecture dont
toute la famille d'écriture est déniée **n'informe aucune action disponible** :
elle prépare une décision que l'agent ne peut pas prendre. C'est la condition
structurelle, et c'est celle qui se vérifie par du code.

**(C2) — aucun prompt servi ne le nomme.** Mesure du § 2.3, lue sur la table
générée de `bundled_skills.rs` — le contenu *compilé*, qui est ce qui est servi
(doctrine mika#2340), jamais une lecture disque.

**(C3) — il n'est ni constitutif ni auto-connaissance.** Ni `send_message` ni les
écritures mémoire agent-scoped (#811, épinglé par
`test_mika_arch_disabled_tools_excludes_agent_self_state`), ni la famille
d'auto-connaissance (#740 : `get_config`, `get_active_llm`, `list_skills`,
`read_agent_file`, `list_agent_files`), ni les instruments d'ancrage dont dépend
la garde assert-grounded (#1331 : `query_timeline`, `get_session_messages`,
`list_audit_events`, `search_tool_history`).

C'est le raisonnement de `--only-skill` (mika#2363) transposé : mika#2363 n'a pas
*supprimé* des skills, il a remplacé un « tout est allumé » implicite par une
**déclaration positive de ce qui est servi**. Ici, (C1) est l'équivalent
structurel de cette déclaration : on ne coupe pas au plus gros, on coupe ce que
la dénylist existante a **déjà** décidé d'interdire sans en tirer la conséquence
sur les lectures orphelines.

### 3.2 Les neuf noms, et la famille qui les orpheline

| lecture retirée | famille d'écriture, et son état |
|---|---|
| `list_reminders` | `create_reminder`, `cancel_reminder` — **déniés** |
| `list_scheduled_tasks` | `create_scheduled_task` — **non enregistré** dans `default_tools()` ; construit en interne par `complete_task`, lui-même dénié |
| `list_tasks` | `create_task`, `update_task_status`, `complete_task`, `cancel_task` — **les quatre déniés** |
| `get_task` | idem |
| `check_task` | idem |
| `list_teams` | `create_team`, `delete_team`, `update_team`, `add_team_member`, `remove_team_member`, `run_team` — **les six déniés** |
| `get_team_status` | idem |
| `get_team_history` | idem |

Plus **un neuvième, qui n'est pas une lecture** et qui relève donc d'un autre
motif, déjà présent dans la dénylist :

| écriture retirée | motif |
|---|---|
| `promote_deferred_callback` | écriture sur le registre de créneaux de dispatch différé — une **voie que mika-arch ne possède pas**, exactement la catégorie *Cross-agent invocation* qui porte déjà `a2a_call`, `delegate_task`, `run_team`. Son oubli dans la dénylist de #811 est un trou, pas une décision. |

### 3.3 Le borderline, retenu délibérément

`list_agents` satisfait (C1) — `create_agent` est dénié — et (C2). Il est
**conservé** quand même : il ne prépare pas une action, il porte une
**connaissance de topologie** (qui d'autre existe), et le contrat
d'auto-connaissance de mika#740 peut s'y appuyer hors prompt de skill. Le nommer
ici est la moitié honnête de la décision : le critère seul l'aurait coupé, et le
critère n'est pas un automate.

### 3.4 Ce que la coupe pèse — en bande, pas en chiffre faux

La mesure exacte en octets sérialisés n'est pas faisable depuis le worktree (pas
d'accès à `/var/log/mika/`, pas de binaire déployé). Ce qui l'est : peser les
régions `fn definition` en source, numérateur et dénominateur de la même façon,
et transporter la **proportion**.

```bash
cd crates/mika-agent/src/tools
# les 9 retirés
sed -n '/fn definition/,/^    }$/p' list_reminders.rs list_scheduled_tasks.rs get_task.rs \
  check_task.rs list_tasks.rs list_teams.rs get_team_status.rs get_team_history.rs \
  promote_deferred_callback.rs | wc -c          # → 8 019
# les 28 servis aujourd'hui
sed -n '/fn definition/,/^    }$/p' update_core_memory.rs store_fact.rs … | wc -c   # → 35 362
```

8 019 / 35 362 = **22,7 %**. Appliqué aux ~20 205 o de builtins sérialisés (21 175
moins ~970 pour `gh_read`), l'attente est de **≈ 4,6 Ko, soit ~21–22 % de
`tool_defs_bytes`** — et ~3,5 % du brief entier sur la population du seuil.

**Le plan ne prétend pas à ce chiffre : il prétend à la bande 3,5–5,5 Ko.** Le
rapport source↔JSON n'est pas 1:1 (l'échappement ajoute, l'indentation retire).
L'arbitre est `tool_defs_bytes` après déploiement, et il est déjà instrumenté :
c'est le champ même de la précondition du ticket. Le § 6 en fait la sonde.

### 3.5 Pourquoi ce n'est pas « une coupe au plus gros », et pourquoi ça vaut quand même d'être pris

Un cinquième d'un sixième du brief, ce n'est pas le levier — mika#2474 a déjà
établi que le levier est le plan. Deux raisons le rendent dû malgré tout.

D'abord **il est constant** : il est payé 100 % des tours, y compris les tours
courts où il domine la question, alors que le dépassement de seuil est une queue.

Ensuite — et c'est l'acquis structurel — **une dénylist croît en silence**. Chaque
`registry.register(...)` ajouté à `default_tools()` est servi à mika-arch par
défaut, sans qu'aucune assertion ne rougisse : c'est la posture inverse de celle
que l'allowlist de skills applique déjà au même agent (*denied by default*). Le
recensement à roster attesté du § 5 D3 est ce qui arrête cette croissance ; le
basculement complet dénylist → allowlist est le vol 2 (§ 8).

---

## 4. Ce que la décision ne casse pas

**#811 reste entier.** Les neuf noms ne touchent ni `send_message`, ni
`update_core_memory`, ni `store_fact`, ni `update_fact`.
`test_mika_arch_disabled_tools_excludes_agent_self_state` reste vert et
intouché ; D2 l'invoque comme borne du critère.

**La garde 6c `asserted_unavailability` compose.** Elle lit
`enabled_tool_names`. Un architecte qui dirait désormais « je n'ai pas accès à
l'état des tâches » dirait quelque chose de **vrai**, et la garde resterait
silencieuse — c'est la propriété que #811 a écrite pour la dénylist (*« le modèle
ne voit jamais les outils déniés, ne peut pas les appeler, ne peut pas se faire
injecter l'envie d'essayer »*), et elle s'étend sans modification.

**La porte `required_tools` ne bouge pas.** Les trois skills d'arch exigent
`gh_read`, et rien d'autre ; `gh_read` est servi par leurs `tools.json`.

**La fenêtre de conversation n'est pas touchée** (mika#2295 / #2330 / #2425), ni
le plan lui-même, dont les quatre transformations concevables sont refusées avec
leur motif au § 3 du plan mika#2474.

**Rien ne change pour les autres agents.** Le registre global est partagé, mais le
filtre est `Identity.tools.disabled`, par agent. mika, mika-dev, mika-qa,
mika-test reçoivent leur tableau octet pour octet.

### 4.1 La chaîne de déploiement, et pourquoi elle mord bien ici

C'est le piège de mika#1833 / mika#2330 : une déclaration de code qui n'atteint
jamais un agent déjà provisionné. Il ne se referme pas ici, et c'est vérifié :
`tools.disabled` **est** dans `CODE_OWNED_IDENTITY_SECTIONS`
(`well_known_agents.rs:574`), donc `reconcile_well_known_identity` réécrit le
sous-arbre au démarrage quand le disque diffère de la spec, et émet
`identity_reconcile.applied` avec `path = "tools.disabled"`. La chaîne est
`make deploy` → redémarrage → réconciliation → tour suivant. Elle est la sonde P1
du § 6.

---

## 5. Les livrables, site par site

### D1 — `crates/mika-agent/src/well_known_agents.rs` : les neuf entrées et leur motif

Deux sous-sections nouvelles dans `MIKA_ARCH_DISABLED_TOOLS`, chacune commentée
par son motif (la const est lue comme une déclaration, pas comme une liste) :

```rust
    // Orphaned reads (mika#2593) — a read whose entire write family is denied
    // above informs no action this agent can take. Criterion C1, pinned by
    // `mika2593_chaque_lecture_orpheline_a_sa_famille_decriture_hors_datteinte`.
    "list_reminders",
    "list_scheduled_tasks",
    "list_tasks",
    "get_task",
    "check_task",
    "list_teams",
    "get_team_status",
    "get_team_history",
    // Deferred-dispatch slot ledger (mika#2593) — a WRITE, in the lane this
    // agent does not own. Same category as `a2a_call` / `delegate_task` /
    // `run_team` above; its absence from #811's list was a gap, not a decision.
    "promote_deferred_callback",
```

Le doc-comment de la const gagne un paragraphe nommant le critère C1/C2/C3, le
borderline `list_agents` retenu (§ 3.3), et la bande attendue (§ 3.4). Le bloc
*Notably allowed* existant est **conservé mot pour mot** : c'est lui qui porte
#811.

`MIKA_ARCH_DISABLED_TOOLS.len()` passe de 23 à 32 —
`test_build_mika_arch_identity` assertant l'égalité avec `identity.tools.disabled.len()`
reste vert sans retouche (il compare la const à elle-même).

### D2 — la garde du critère C1 (détecteur)

`mika2593_chaque_lecture_orpheline_a_sa_famille_decriture_hors_datteinte`, dans
le `mod tests` de `well_known_agents.rs`. Une table déclarée porte la
correspondance lecture → famille :

```rust
const MIKA_ARCH_ORPHANED_READS: &[(&str, &[&str])] = &[ … ];  // les 8 lignes du § 3.2
```

La garde assère, pour chaque paire :

1. la lecture **est** dans `MIKA_ARCH_DISABLED_TOOLS` ;
2. chaque membre de sa famille est **soit** dans `MIKA_ARCH_DISABLED_TOOLS`,
   **soit** absent des noms déclarés par `tools::default_tools().definitions()` ;
3. la lecture n'est **pas** dans la liste constitutive de #811 — la garde
   réutilise les trois noms de
   `test_mika_arch_disabled_tools_excludes_agent_self_state` plutôt que de les
   retaper, pour que les deux tests ne puissent pas diverger.

Ce qu'elle attrape, et qu'aucun test comportemental ne voit : un futur ticket qui
élargirait la coupe à une lecture dont l'écriture est **encore servie** — la
seule forme sous laquelle cette coupe devient destructive. `promote_deferred_callback`
n'est **pas** dans la table : ce n'est pas une lecture, son motif est la voie, et
le mélanger salirait le critère.

### D3 — le recensement à roster attesté (détecteur)

`mika2593_le_tableau_servi_a_mika_arch_est_un_roster_attesté`, dans le `mod tests`
de `crates/mika-agent/src/tools/mod.rs` — là où vit déjà le scanner
`declared_tool_names` et son consommateur `mika1952_gated_tools_match_the_reflection_contract_constant`,
dont la garde reprend la forme.

Elle construit `default_tools()`, retire les noms de `MIKA_ARCH_DISABLED_TOOLS`,
et assère l'**égalité ensembliste** avec un roster déclaré
`MIKA_ARCH_SERVED_BUILTINS_FROM_DEFAULT_TOOLS` (les 19 noms issus de
`default_tools()` après coupe). Divergence attrapée **dans les deux sens** :

- un `register(...)` nouveau non ajouté au roster → un outil atteint mika-arch
  sans que personne l'ait décidé, et `tool_defs_bytes` remonte en silence. C'est
  la croissance que le § 3.5 décrit, et c'est la raison d'être de ce test ;
- une entrée du roster qui n'est plus enregistrée → le roster décrit un tableau
  qui n'existe plus, et il cesse d'être lisible comme attestation.

**Borne déclarée :** le recensement ne couvre que `default_tools()`, pas les 14
outils de `management_tools_if_needed`, qui exige un `home_dir`, un `Settings`,
un client HTTP et une `GitHubApp`. C'est accepté, et dit : la surface de
croissance est `default_tools()` (c'est là que les `register(...)` s'ajoutent),
et 11 des 14 outils de management sont déjà déniés — les 3 restants
(`list_agents`, et `list_teams` / `get_team_status` / `get_team_history` que ce
vol dénie) sont nommés un par un par D2.

### D4 — l'attestation d'octets sur l'événement existant

`apply_agent_tool_visibility` (`agent_loop/mod.rs:8710`) émet déjà
`applied identity tool-visibility filter` avec `hidden_count` et `disabled_size`.
Deux champs s'y ajoutent, **sur l'événement existant plutôt que sous un nom
nouveau** :

- `kept_count` — les outils restants ;
- `kept_bytes` — `serde_json::to_string(&*tool_defs).map(|s| s.len()).unwrap_or(0)`,
  la même expression que `tool_defs_bytes` (`mod.rs:9230`), de sorte que les deux
  nombres soient comparables par construction.

Sans ces deux champs, un opérateur qui lit `tool_defs_bytes` après déploiement ne
peut pas distinguer « la coupe a atterri » de « la coupe a atterri à moitié » :
il lit un total, et un total ne se soustrait pas. Avec eux, la ligne est
auto-suffisante — elle dit ce qui a été retiré **et** ce qui reste.

Le point d'émission ne bouge pas : la ligne reste sous le `if removed > 0`, donc
un agent sans dénylist ne paie rien et n'émet rien, comme aujourd'hui.

### D5 — le test de D4

`mika2593_le_filtre_atteste_le_compte_et_les_octets_restants`, dans le `mod tests`
de `agent_loop/mod.rs`, à côté des trois
`test_apply_agent_tool_visibility_*` existants. Il construit trois
`ToolDefinition` dont un dénié et vérifie que `kept_count` vaut 2 et que
`kept_bytes` égale la longueur de la sérialisation du vecteur **après** filtrage —
l'égalité, pas un seuil, pour que le champ ne puisse pas dériver vers « avant
filtrage » sans rougir.

---

## Fire-Disposition

Ce plan livre **deux détecteurs** : D2 (garde du critère C1) et D3 (recensement à
roster attesté).

**Option retenue : (a) exception nommée en allowlist — avec zéro entrée.**

Les deux détecteurs sont **verts à la naissance par construction**, et c'est une
propriété vérifiable plutôt qu'un espoir :

- **D2** est vert parce que la table `MIKA_ARCH_ORPHANED_READS` et les neuf
  entrées de D1 sont écrites dans le même commit depuis le même relevé (§ 3.2).
  Aucune violation préexistante n'est possible : la table ne contient que des
  noms que D1 vient de dénier.
- **D3** est vert parce que son roster est, par définition, l'ensemble
  `default_tools()` moins `MIKA_ARCH_DISABLED_TOOLS` tel qu'il sort de D1 — il est
  dérivé de l'état d'arrivée, pas d'un état souhaité.

Les deux arrivent donc **armés, sans `#[ignore]`, et avec une liste d'exceptions
vide**. La liste existe quand même, sous le nom
`MIKA2593_CRITERION_EXCEPTIONS: &[(&str, &str)] = &[];` — un couple (nom d'outil,
ticket de suivi) — pour la raison que ce dépôt a déjà écrite ailleurs : une
allowlist qui n'existe que le jour où elle se remplit se remplit sans revue. Elle
porte son **assertion auto-nettoyante** :

```rust
assert!(
    MIKA2593_CRITERION_EXCEPTIONS.is_empty(),
    "mika#2593 — chaque exception doit nommer son ticket de suivi et disparaître \
     avec lui ; retirer l'entrée quand le ticket est clos, ne pas la laisser \
     périmer : {MIKA2593_CRITERION_EXCEPTIONS:?}"
);
```

Elle rougit dès qu'une entrée y est ajoutée sans que le test de référence soit
mis à jour, et elle est la ligne grep-visible qui nomme ce qui a été excepté.

Ni (b) *livrer désarmé* — il n'y a rien à désarmer, aucune violation n'existe —
ni (c) *halte-et-remontée* — l'arbitrage est tranché au § 3, avec son critère et
son borderline nommés.

---

## 6. Contrat de vérification

### Avant merge (en CI, sans déploiement)

| # | vérification | commande |
|---|---|---|
| V1 | la suite passe, dont D2, D3, D5 | `cargo test -p mika-agent` |
| V2 | lint et format | `cargo clippy && cargo fmt --check` |
| V3 | #811 reste vert **et intouché** | `git diff origin/main -- crates/mika-agent/src/well_known_agents.rs` ne doit montrer aucune modification dans `test_mika_arch_disabled_tools_excludes_agent_self_state` ni dans le bloc *Notably allowed* |
| V4 | la structure des skills bundlés tient | `make verify-bundled-skills` |
| V5 | le pipeline | `scripts/verify-pipeline.sh origin/main` |

### Après déploiement (sondes opérateur)

**P1 — la décision a atteint le disque.** Sans elle, tout le reste est muet.

```bash
grep identity_reconcile "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.agent == "mika-arch") | {event, path}'
```

Attendu : `identity_reconcile.applied` portant `tools.disabled`. Au démarrage
suivant, `identity_reconcile.in_sync`. **Si la ligne manque** : le binaire servi
précède le correctif (classe mika#2340), ce n'est pas une réconciliation qui a
échoué.

**P2 — le filtre a retiré ce qu'il devait, et le dit.**

```bash
grep "tool-visibility" "$MIKA_SPIRIT_LOG_FILE" | jq -c '{hidden_count, kept_count, kept_bytes}'
```

Attendu : `hidden_count` 23 → **32**, `kept_count` 28 → **19** (`default_tools()`
moins coupe ; les outils de skill s'ajoutent après ce point), `kept_bytes` en
baisse d'environ 4,6 Ko.

**P3 — l'arbitre : `tool_defs_bytes`, le champ même de la précondition.**

```bash
grep context_window_assembled "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.agent_id == "mika-arch") | {tool_defs_bytes}' | sort | uniq -c
```

Attendu : une **nouvelle valeur constante** dans la bande **15,6 – 17,6 Ko**
(21 175 − 3 500 à 5 500). La constance est la moitié de l'attendu : une valeur qui
varierait d'un tour à l'autre dirait que le filtre dépend d'une entrée, ce qu'il
ne doit pas.

**P4 — contrôle négatif : aucun autre agent n'a bougé.**

```bash
grep context_window_assembled "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.agent_id != "mika-arch") | {agent_id, tool_defs_bytes}' | sort -u
```

Attendu : inchangé. C'est la sonde qui sépare « un filtre par agent » de « un
registre amputé ».

**P5 — la capacité n'a pas été entamée.** Sur les dix dispatches de grooming
suivants :

```bash
grep -c "Disposition: ESCALATE" "$MIKA_SPIRIT_LOG_FILE"
grep asserted_unavailability "$MIKA_SPIRIT_LOG_FILE" | jq -c 'select(.agent == "mika-arch")'
```

Attendu : aucun ESCALATE **motivé par un outil manquant**, et zéro déclenchement
de la garde 6c sur mika-arch — un architecte qui dirait « je n'ai pas accès à
l'état des tâches » dirait vrai, donc la garde doit rester silencieuse. **Si elle
fire**, c'est que l'architecte revendique l'absence d'un outil qu'il a encore :
la coupe a raté sa cible, pas sa taille.

**Reversion :** retirer les neuf entrées de la const, `make deploy`, redémarrer.
`reconcile_well_known_identity` remet `tools.disabled` à l'état précédent au
démarrage suivant — aucune migration, aucune donnée, aucun état à défaire.

---

## 7. Ordre de commit

Un seul commit. Les neuf entrées, leurs deux gardes, les deux champs et le test
de ces champs ne sont pas séparables : D1 sans D2/D3 est une coupe sans critère
vérifiable, et D2/D3 sans D1 sont deux tests rouges.

```
feat(agent): les lectures orphelines de mika-arch quittent le tableau servi — vol 1 (mika#2593)
```

**Plancher :** `cargo test -p mika-agent` vert, `cargo clippy` propre, V3 vérifié
à la main sur le diff.

---

## 8. Hors périmètre

Chaque renvoi nomme sa raison ; aucun n'est un report par fatigue.

**(a) Le basculement dénylist → allowlist pour les outils — c'est le vol 2.** Le
doc-comment d'`apply_agent_tool_visibility` l'invite déjà en ces termes : *« Future
migration (when well-known agents move from denylist to allowlist for tools):
extend this hook to handle both shapes — same call site, different predicate, no
caller changes. »* C'est ce qui rendrait le gain **permanent** au lieu de
réérodable par le prochain `register(...)`, et c'est l'analogue exact de ce que
`[skills].allowlist` fait déjà pour cet agent. Ce n'est pas dans ce vol parce que
cela touche le schéma `Identity.tools` (un champ `allowed`), le prédicat du
filtre, `CODE_OWNED_IDENTITY_SECTIONS`, la réconciliation et leurs gardes — soit
largement au-delà de la borne par vol. D3 en livre la moitié qui tient : le
recensement attesté arrête la croissance silencieuse sans changer le schéma.
**Tracked in: senara-solutions/mika#2593** (vol 2, à ouvrir à la clôture de ce
vol avec le relevé P2/P3 en corps).

**(b) `system_prompt_bytes` sur `context_window_assembled`.** Le reliquat
d'instrument nommé par l'opérateur (§ 1.1). C'est un défaut de mesure, pas
d'arbitrage, et il ne change aucune décision de ce plan — la grandeur retenue est
le rapport à `user_message_bytes`. À ouvrir séparément : il sert toute lecture
future de la part d'un composant, pas celle-ci.

**(c) `list_agents`.** Conservé, et dit au § 3.3. Le rouvrir demanderait de
trancher si le contrat d'auto-connaissance de mika#740 s'appuie dessus, ce qui est
une question sur #740, pas sur la taille.

**(d) Un recensement par outil de `tool_defs_bytes`.** Un événement portant les
octets **de chaque** définition rendrait le prochain arbitrage mesurable au lieu
d'estimé (§ 3.4 a dû transporter une proportion depuis la source faute de l'avoir).
C'est l'instrument du vol 2, pas de celui-ci : ce vol-ci n'a pas besoin de
granularité pour décider, puisque son critère est structurel et non budgétaire.

**(e) Le registre global lui-même.** `server/mod.rs:979` construit un registre
partagé ; un registre par agent supprimerait le filtre de présentation au profit
d'une construction ciblée. Beaucoup plus gros, et pas nécessaire : le filtre est
déjà le bon point d'application (le modèle ne voit jamais l'outil, #811).

**(f) La fenêtre de conversation et le plan.** mika#2295 / #2330 / #2425 pour la
première ; § 3 du plan mika#2474 pour le second, qui refuse ses quatre
transformations concevables avec leur motif.

---

## 9. Risques

**R1 — La coupe est destructive sur un usage que le dépôt ne montre pas.** Le
critère C2 lit les prompts servis ; il ne lit pas l'historique d'appels, qui vit
dans `tool_calls` et n'est pas atteignable depuis un worktree. Un architecte qui
appellerait `list_tasks` sur une improvisation non instruite perdrait cet appel.
**Mitigation :** C1 borne le dégât — l'appel perdu ne préparait aucune action
disponible. P5 le mesure sur dix dispatches. La reversion est une suppression de
neuf lignes et un redémarrage, sans état à défaire.

**R2 — La bande 3,5–5,5 Ko est fausse.** La proportion source↔JSON peut ne pas
transporter. **Mitigation :** c'est pourquoi le plan annonce une bande et nomme
P3 comme arbitre. Une valeur hors bande n'invalide pas la décision — le critère
du § 3.1 ne repose pas sur le gain — elle invalide l'estimateur, et c'est alors
(d) qui est dû.

**R3 — D3 rougit sur un ticket sans rapport.** Le prochain `register(...)` dans
`default_tools()` fera rougir le recensement, et son auteur n'aura pas lu ce
plan. **Mitigation :** le message d'assertion doit dire *quoi faire* — ajouter le
nom au roster si l'outil doit atteindre mika-arch, l'ajouter à
`MIKA_ARCH_DISABLED_TOOLS` sinon — et nommer les deux fichiers. C'est la forme que
`mika1952_gated_tools_match_the_reflection_contract_constant` emploie déjà et
qu'il faut reprendre.

**R4 — La réconciliation n'atteint pas le disque.** Classe mika#1833 / mika#2330.
**Mitigation :** `tools.disabled` est vérifié présent dans
`CODE_OWNED_IDENTITY_SECTIONS` (§ 4.1), et P1 est la sonde qui le constate plutôt
que de le supposer.

---

## Definition of Done

1. Les neuf noms du § 3.2 sont dans `MIKA_ARCH_DISABLED_TOOLS`, dans deux
   sous-sections commentées portant chacune son motif.
2. Le doc-comment de la const nomme le critère C1/C2/C3, le borderline
   `list_agents` et la bande attendue ; le bloc *Notably allowed* est intact.
3. D2 existe, est armé, et assère les trois conjonctions du critère sur la table
   `MIKA_ARCH_ORPHANED_READS`, en réutilisant les noms constitutifs de #811 plutôt
   qu'en les retapant.
4. D3 existe, est armé, et assère l'égalité ensembliste dans les **deux** sens
   entre `default_tools()` moins la dénylist et le roster attesté ; sa borne
   (`management_tools_if_needed` hors portée) est écrite dans son doc-comment.
5. `MIKA2593_CRITERION_EXCEPTIONS` existe, est **vide**, et porte son assertion
   auto-nettoyante.
6. `applied identity tool-visibility filter` porte `kept_count` et `kept_bytes` ;
   D5 épingle l'égalité de `kept_bytes` avec la sérialisation post-filtrage.
7. V1 à V5 passent.
8. `test_mika_arch_disabled_tools_excludes_agent_self_state` est vert et son
   diff est vide (V3).
9. Le corps de PR porte les cinq sondes P1–P5 avec leur attendu, la bande
   3,5–5,5 Ko annoncée comme bande, et la procédure de reversion.
10. Le corps de PR nomme le vol 2 par une ligne `Tracked in:` (§ 8 (a)).

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria`. Les critères
suivants sont dérivés de ses trois exigences — la précondition, la décision, et
l'arbitrage de capacité — et de la section *Contrat de vérification* ci-dessus.

- **AC1 — La précondition est reprise et tranchée par écrit.** Le plan et le
  corps de PR reprennent le relevé de l'opérateur (n = 3 sur la queue, n = 75 sur
  la population entière, `tool_defs_bytes` constant à 21 175 o, rapport 0,41 à
  `user_message_bytes`) et concluent explicitement que le ticket **ne se ferme pas
  sans travail**.
- **AC2 — La réduction est décidée, pas seulement instrumentée.** Neuf noms
  précis quittent le tableau servi à mika-arch dans ce vol. Un plan qui se
  contenterait de mesurer ne satisfait pas ce critère.
- **AC3 — La non-destructivité repose sur un critère écrit, pas sur une absence
  d'usage.** Les trois conjonctions C1/C2/C3 sont écrites, et C1 est **vérifiée
  par du code** (D2), pas seulement affirmée en prose.
- **AC4 — L'arbitrage de capacité respecte #811.** `send_message`,
  `update_core_memory`, `store_fact`, `update_fact` restent servis.
  `test_mika_arch_disabled_tools_excludes_agent_self_state` est vert et son diff
  est vide.
- **AC5 — La coupe n'est pas « au plus gros ».** Le plan nomme au moins un outil
  volumineux ou plausible qu'il **conserve** avec son motif — `list_agents`
  (§ 3.3) — et la famille d'instruments d'ancrage conservée par C3.
- **AC6 — La coupe est observable.** `kept_count` et `kept_bytes` sur
  l'événement existant, et P3 lit `tool_defs_bytes` — le champ même de la
  précondition — comme arbitre du gain. Aucun nouveau nom d'événement n'est créé.
- **AC7 — Le gain cesse de s'éroder.** D3 rougit sur le prochain
  `register(...)` qui atteindrait mika-arch sans décision, dans les deux sens de
  divergence.
- **AC8 — La décision atteint un agent déjà provisionné.** `tools.disabled` est
  dans `CODE_OWNED_IDENTITY_SECTIONS`, et P1 le constate après déploiement.
- **AC9 — Ce qui n'est pas pris est nommé avec sa raison.** Le § 8 énumère six
  renvois, dont le basculement vers une allowlist (vol 2, `Tracked in:`) et le
  reliquat `system_prompt_bytes` nommé deux fois par l'opérateur.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/well_known_agents.rs` — D1 : 9 entrées + 2 sous-sections commentées + paragraphe de doc du critère | 52 |
| `crates/mika-agent/src/well_known_agents.rs` — D2 : table `MIKA_ARCH_ORPHANED_READS`, garde C1, allowlist vide + assertion auto-nettoyante | 78 |
| `crates/mika-agent/src/tools/mod.rs` — D3 : roster attesté + recensement bidirectionnel + doc de sa borne | 48 |
| `crates/mika-agent/src/agent_loop/mod.rs` — D4 : `kept_count` + `kept_bytes` sur l'événement existant | 12 |
| `crates/mika-agent/src/agent_loop/mod.rs` — D5 : test de l'attestation d'octets | 28 |

Total estimé : 218 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Un seul vol, et voici pourquoi il tient.** 218 < 243, et 218 < 1000 : les deux
bornes passent, la plus serrée (243) avec ~10 % de marge. Trois propriétés
soutiennent l'estimation plutôt qu'un espoir :

- **Aucun site nouveau.** Les cinq livrables atterrissent dans trois fichiers qui
  existent, sur des sites qui existent : une const (D1), deux `mod tests` déjà
  peuplés (D2, D3, D5), et **deux champs** sur un `tracing::info!` déjà écrit
  (D4). Zéro module créé, zéro signature changée, zéro migration.
- **Les deux détecteurs ont leur patron en dépôt.** D3 reprend la forme de
  `mika1952_gated_tools_match_the_reflection_contract_constant`
  (`tools/mod.rs:1550`), scanner `declared_tool_names` inclus — un patron suivi
  coûte moins qu'un patron inventé. D5 se range à côté des trois
  `test_apply_agent_tool_visibility_*` existants.
- **La part de doc-comment est forte et son volume est connu.** ~70 des 218 lignes
  sont du commentaire de déclaration (motif des deux sous-sections, critère,
  borne de D3), dont le contenu est déjà rédigé aux § 3.1 à § 3.3 de ce plan : il
  reste à le transcrire, pas à le trouver.

Le vol 2 — le basculement dénylist → allowlist, qui touche le schéma
`Identity.tools` et la réconciliation — est nommé au § 8 (a), avec sa ligne
`Tracked in:`. Il est mergeable seul et après celui-ci : il **remplace** le
prédicat du filtre que ce vol-ci se contente de nourrir.
