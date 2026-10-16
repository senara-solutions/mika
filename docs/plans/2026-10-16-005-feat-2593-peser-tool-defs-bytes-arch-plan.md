# mika#2593 — Peser `tool_defs_bytes` devant le plan, et prendre la réduction non destructive qui existe

**Ticket :** senara-solutions/mika#2593
**Type :** feat (agent-core)
**Suivi de :** mika#2474 § 9 (*Hors périmètre*)
**Vol :** un seul — 231 lignes estimées sous la borne de 243 (§ *Taille estimée*)

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

Les trois prompts servis à mika-arch — `mika-arch-groom-ticket` (276 lignes),
`mika-arch-second-review` (238), `mika-arch-groom-milestone` (128) — ne nomment
**que quatre outils**. La mesure est reproductible telle quelle : l'alternance
énumère les **vingt-neuf noms servis** du § 2.2, pas un échantillon, de sorte
qu'un nom présent ne puisse pas échapper au filtre.

```bash
grep -ohE '\b(update_core_memory|store_fact|update_fact|search_memory|list_reminders\
|promote_deferred_callback|get_task|send_message|list_skills|get_config|get_active_llm\
|read_agent_file|list_agent_files|list_scheduled_tasks|list_tasks|check_task\
|resolve_issue_order|query_knowledge_graph|query_timeline|get_session_messages\
|list_audit_events|search_tool_history|record_served_content|check_already_served\
|list_agents|list_teams|get_team_status|get_team_history|gh_read)\b' \
  skills/bundled/mika-arch-*/system_prompt.md | sort | uniq -c
```

Relevé le 2026-10-16 sur `HEAD` :

```
     17 gh_read
      6 query_knowledge_graph
      2 store_fact
      2 update_core_memory
```

`gh_read` et `query_knowledge_graph` sont la lecture ; `store_fact` et
`update_core_memory` sont la mémoire agent-scoped épinglée par #811. **Vingt-cinq
des vingt-neuf outils servis ne sont nommés par aucune instruction du tour**, et
**aucun des neuf noms que ce vol retire n'apparaît une seule fois**. Ils sont là
sur l'hypothèse que le modèle improvisera.

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
| `list_scheduled_tasks` | `create_reminder`, `cancel_reminder` — **déniés** (le doc-comment de la const les range sous *« writes to scheduled_tasks rows »*, c'est la même table) ; plus `create_scheduled_task`, **non enregistré** dans `default_tools()` ni dans `management_tools_if_needed` — il n'existe que construit en interne par `complete_task` (`complete_task.rs:185`), lui-même dénié |
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
basculement complet dénylist → allowlist est hors périmètre, motivé au § 8 (a).

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
3. la lecture n'est **pas** dans la liste constitutive de #811.

Ce qu'elle attrape, et qu'aucun test comportemental ne voit : un futur ticket qui
élargirait la coupe à une lecture dont l'écriture est **encore servie** — la
seule forme sous laquelle cette coupe devient destructive. `promote_deferred_callback`
n'est **pas** dans la table : ce n'est pas une lecture, son motif est la voie, et
le mélanger salirait le critère.

**Le point 3 coûte une extraction, et il faut la nommer parce qu'elle touche la
zone que V3 protège.** Les trois noms de #811 vivent aujourd'hui dans un
tableau **local** au corps du test (`let agent_self_state_tools = [...]`,
`well_known_agents.rs:2083`) : rien ne peut les lire de l'extérieur. Deux voies
seulement, et l'une est exclue :

- **retaper les trois noms dans D2** — refusé : deux listes du même ensemble
  dans le même fichier divergent au premier ticket qui en modifie une, et aucune
  assertion ne rougit (classe mika#2205) ;
- **les extraire en `const MIKA_ARCH_AGENT_SELF_STATE_TOOLS: &[&str]`**, dont le
  test #811 devient le **premier lecteur** et D2 le second. Retenu.

Donc le diff du test #811 n'est **pas** vide, et V3 le dit ainsi plutôt que
d'exiger l'impossible. Ce que V3 borne est précis : le changement autorisé dans
ce test est **exactement** le remplacement du tableau local par la const, sans
qu'un nom soit ajouté, retiré ou réécrit, et sans que l'assertion ni son message
bougent. Le doc-comment de la const reprend mot pour mot le commentaire de trois
lignes que le test porte aujourd'hui — c'est lui qui porte #811, et il doit
monter avec les noms, pas rester derrière eux.

Noter que la const n'est **pas** `pub` : son seul usage est ce fichier, et un
`pub` en ferait une déclaration publique que personne ne lit — exactement
l'inverse de ce que l'extraction achète.

### D3 — le recensement à roster attesté (détecteur)

`mika2593_le_tableau_servi_a_mika_arch_est_un_roster_atteste`, dans le `mod tests`
de `crates/mika-agent/src/tools/mod.rs` — à côté de
`mika1952_gated_tools_match_the_reflection_contract_constant`, dont il reprend la
**forme** : une divergence attrapée dans les deux sens, sur une classe qu'aucun
test comportemental ne voit.

Il n'en reprend **pas** le scanner `declared_tool_names` (`tools/mod.rs:1509`), et
c'est ce qui le rend court : ce scanner existe parce que mika#1952 devait
attribuer un appel à un fichier *source*. Ici l'autorité est le registre
lui-même — `default_tools()` ne prend aucun argument et `definitions()`
(`tools/mod.rs:1256`) est `pub` — donc le test lit ce qui est **construit**,
jamais ce qui est écrit. Une lecture de source serait ici un détour qui pourrait
diverger de l'objet qu'elle décrit.

Il construit `default_tools()`, retire les noms de `MIKA_ARCH_DISABLED_TOOLS`,
et assère l'**égalité ensembliste** avec un roster déclaré
`MIKA_ARCH_SERVED_BUILTINS_FROM_DEFAULT_TOOLS` : **18 noms**. Le compte se
dérive, il ne se devine pas — et il ne vaut pas 19, qui est le nombre d'une
autre grandeur (cf. l'avertissement sous le tableau) :

| | |
|---|---|
| `default_tools()` | 37 |
| − déniés **qui y sont enregistrés** (13 des 23 : les 10 autres sont des outils de management) | − 13 |
| servis de `default_tools()` **aujourd'hui** | 24 |
| − coupe de ce vol **qui y est enregistrée** (6 des 9 : `list_teams` / `get_team_status` / `get_team_history` sont des outils de management) | − 6 |
| **roster** | **18** |

**19 est le `kept_count` du § 6 P2, pas ce roster.** `kept_count` est mesuré sur
le **registre global** (51 − 32 = 19) parce que `apply_agent_tool_visibility`
filtre le tableau assemblé, management inclus ; le roster ne couvre que
`default_tools()`. Les deux nombres sont à un écart d'une unité pour des raisons
sans rapport, et les confondre est l'erreur que ce paragraphe existe pour
empêcher.

Divergence attrapée **dans les deux sens** :

- un `register(...)` nouveau non ajouté au roster → un outil atteint mika-arch
  sans que personne l'ait décidé, et `tool_defs_bytes` remonte en silence. C'est
  la croissance que le § 3.5 décrit, et c'est la raison d'être de ce test ;
- une entrée du roster qui n'est plus enregistrée → le roster décrit un tableau
  qui n'existe plus, et il cesse d'être lisible comme attestation.

**Borne déclarée :** le recensement ne couvre que `default_tools()`, pas les 14
outils de `management_tools_if_needed`, qui exige un `home_dir`, un `Settings`,
un client HTTP et une `GitHubApp`. C'est accepté, et dit : la surface de
croissance est `default_tools()` (c'est là que les `register(...)` s'ajoutent),
et **10 des 14** outils de management sont déjà déniés par #811 — les **4**
restants sont `list_agents` (conservé, § 3.3) et `list_teams` /
`get_team_status` / `get_team_history`, que ce vol dénie et que D2 nomme un par
un. Après ce vol, la dénylist couvre donc 13 des 14 ; le seul outil de
management encore servi à mika-arch est `list_agents`.

### D4 — l'attestation d'octets sur l'événement existant, et le champ qui la rend lisible

`apply_agent_tool_visibility` (`agent_loop/mod.rs:8689`) émet déjà
`applied identity tool-visibility filter` avec `hidden_count` et `disabled_size`.
**Trois** champs s'y ajoutent, **sur l'événement existant plutôt que sous un nom
nouveau** :

- `kept_count` — les outils restants ;
- `kept_bytes` — `serde_json::to_string(&*tool_defs).map(|s| s.len()).unwrap_or(0)`,
  la même expression que `tool_defs_bytes` (`mod.rs:9230`), de sorte que les deux
  nombres soient comparables par construction ;
- `agent` — **de qui** parle la ligne.

Les deux premiers sont la mesure. Sans eux, un opérateur qui lit
`tool_defs_bytes` après déploiement ne peut pas distinguer « la coupe a atterri »
de « la coupe a atterri à moitié » : il lit un total, et un total ne se soustrait
pas.

**Le troisième est ce qui rend les deux premiers lisibles, et il manque
aujourd'hui.** L'événement ne porte **aucun** identifiant d'agent :
`apply_agent_tool_visibility` ne reçoit que `tool_defs` et `disabled`, et
`inject_skills_and_resolve_tools` (`mod.rs:8978`) ne reçoit pas l'agent non plus.
Or la ligne est émise pour **tout** agent dont la dénylist est non vide —
mika-arch, mika-dev, mika-qa, mika-test — *et* sur tout tour Webhook
Fallthrough de n'importe quel agent, puisque `effective_disabled_tools`
(mika#2517) élargit la dénylist de `create_task` à ce moment-là et fait donc
passer `removed > 0`. Une sonde `grep "tool-visibility"` rend aujourd'hui une
population mêlée : c'est la classe mika#2205, *deux populations sous un nom ne se
soustraient pas*, que ce plan invoque ailleurs et qui s'applique ici.

On pourrait croire la valeur auto-identifiante — mika-arch est le seul agent dont
la dénylist vaille 32 — mais c'est précisément une lecture qui ressemble à une
réponse sans en être une : l'élargissement mika#2517 ajoute 1, donc un agent à 31
émettrait aussi 32. Un discriminant qui dépend d'une coïncidence arithmétique
n'est pas un discriminant.

**Le coût, nommé plutôt que passé sous silence : deux signatures.**
`apply_agent_tool_visibility` prend un `agent: &str` en troisième paramètre, et
`inject_skills_and_resolve_tools` le reçoit pour le lui passer — soit **14 sites
d'appel** à compléter (3 en production aux lignes 5496 / 6792 / 7599, 11 dans
`mod tests`). Aux trois sites de production l'agent est déjà sous la main :
`ctx.db.agent_id()` (`async_db.rs:130`, `&str`), la forme employée ailleurs dans
le fichier. Le changement est mécanique — une ligne par site — mais il contredit
la revendication « zéro signature changée » que portait la section *Taille
estimée*, et celle-ci est corrigée en conséquence.

Le point d'émission ne bouge pas : la ligne reste sous le `if removed > 0`, donc
un agent sans dénylist ne paie rien et n'émet rien, comme aujourd'hui.

### D5 — le test de D4

`mika2593_le_filtre_atteste_le_compte_et_les_octets_restants`, dans le `mod tests`
de `agent_loop/mod.rs`, à côté des trois
`test_apply_agent_tool_visibility_*` existants (lignes 12614, 12642, 12658). Il
construit trois `ToolDefinition` dont un dénié et vérifie que `kept_count` vaut 2
et que `kept_bytes` égale la longueur de la sérialisation du vecteur **après**
filtrage — l'égalité, pas un seuil, pour que le champ ne puisse pas dériver vers
« avant filtrage » sans rougir.

Le champ `agent` n'est pas testable par la même assertion : `tracing::info!`
n'expose pas ses champs au test. Ce qui l'est, et ce que le test épingle, est le
**chemin par lequel il arrive** — la signature. Les quatre appels de test
existants à `apply_agent_tool_visibility` passent désormais un nom d'agent, donc
un futur refactor qui retirerait le paramètre ne compile plus. C'est une garde de
type, pas d'assertion, et elle est dite comme telle : l'arbitre du *contenu* du
champ reste la sonde P2 après déploiement.

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
| V3 | #811 reste vert, et son **ensemble** est intouché | `git diff origin/main -- crates/mika-agent/src/well_known_agents.rs` : dans `test_mika_arch_disabled_tools_excludes_agent_self_state`, la seule modification admise est le remplacement du tableau local par `MIKA_ARCH_AGENT_SELF_STATE_TOOLS` (§ 5 D2) — aucun nom ajouté, retiré ou réécrit, assertion et message inchangés. Le bloc *Notably allowed* du doc-comment de la const, lui, ne montre **aucune** modification |
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
grep "tool-visibility" "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.agent == "mika-arch") | {hidden_count, kept_count, kept_bytes}'
```

Attendu : `hidden_count` 23 → **32**, `kept_count` 28 → **19**, `kept_bytes` en
baisse d'environ 4,6 Ko.

Le `select(.agent == …)` est la moitié de la sonde, et il n'est possible que
grâce au champ ajouté par D4 : la ligne est émise par tout agent à dénylist non
vide et par tout tour Webhook Fallthrough (§ 5 D4), donc sans filtre elle rend
une population mêlée. **Si `.agent` est absent de la sortie**, le binaire servi
précède D4 (classe mika#2340) — ce n'est pas un agent sans nom.

Le `kept_count` est celui du **registre global** au point de filtrage — 51
builtins (37 de `default_tools()` + 14 de `management_tools_if_needed`) moins la
dénylist : 51 − 23 = 28 aujourd'hui, 51 − 32 = 19 après. Les outils de skill
(`gh_read`) s'ajoutent **après** ce point et ne sont comptés ni avant ni après.
Ce 19 n'est pas le roster de D3, qui vaut 18 et ne couvre que
`default_tools()` — le § 5 D3 porte la dérivation des deux.

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

Un seul commit. Les neuf entrées, leurs deux gardes, l'extraction en const des
trois noms de #811, les trois champs avec le threading qui les porte, et le test
de ces champs ne sont pas séparables : D1 sans D2/D3 est une coupe sans critère
vérifiable, D2/D3 sans D1 sont deux tests rouges, l'extraction seule est un
refactor sans lecteur, et le threading seul est une signature élargie sans rien
qui la lise.

```
feat(agent): les lectures orphelines de mika-arch quittent le tableau servi — vol 1 (mika#2593)
```

**Plancher :** `cargo test -p mika-agent` vert, `cargo clippy` propre, V3 vérifié
à la main sur le diff.

---

## 8. Hors périmètre

Chaque renvoi nomme sa raison ; aucun n'est un report par fatigue.

**(a) Le basculement dénylist → allowlist pour les outils.** Le
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

**Ce renvoi n'est pas nommé par une ligne `Tracked in:` dans ce plan, et c'est
délibéré.** La règle mika#2693 exige le découpage en vols — et la ligne qui nomme
la suite — quand le total dépasse la borne par vol ; ici il ne la dépasse pas
(231 < 243), donc ce travail tient **en un seul vol** et le basculement est un
hors-périmètre ordinaire, au même titre que les cinq autres de ce § 8. Une ligne
`Tracked in:` pointant vers mika#2593 lui-même — c'est-à-dire vers le ticket
qu'elle est censée prolonger — serait une référence circulaire : elle ressemble à
une réponse sans en être une, exactement le défaut que la DoD de ce plan reproche
à `tool_defs_bytes` lu comme un total. Le ticket de suivi n'existe pas encore ;
l'implémenteur l'**ouvre** à la fin du vol (il a `gh`, ce plan ne l'a pas) et le
nomme alors par un `Tracked in:` dans le corps de PR, avec le relevé P2/P3 en
corps. C'est la DoD 10 qui porte cette obligation, et c'est le seul endroit où
elle peut être tenue honnêtement.

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
C'est l'instrument du basculement (a), pas de celui-ci : ce vol-ci n'a pas besoin de
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
   `MIKA_ARCH_ORPHANED_READS`. Les trois noms constitutifs de #811 vivent dans
   `MIKA_ARCH_AGENT_SELF_STATE_TOOLS`, lue par D2 **et** par le test #811 — aucun
   des deux ne les retape.
4. D3 existe, est armé, et assère l'égalité ensembliste dans les **deux** sens
   entre `default_tools()` moins la dénylist et le roster attesté de **18** noms ;
   sa borne (`management_tools_if_needed` hors portée) est écrite dans son
   doc-comment, ainsi que la raison pour laquelle 18 ≠ le `kept_count` de 19.
5. `MIKA2593_CRITERION_EXCEPTIONS` existe, est **vide**, et porte son assertion
   auto-nettoyante.
6. `applied identity tool-visibility filter` porte `kept_count`, `kept_bytes` et
   `agent` ; D5 épingle l'égalité de `kept_bytes` avec la sérialisation
   post-filtrage, et la signature porte le chemin de `agent` (§ 5 D5). Les 14
   sites d'appel de `inject_skills_and_resolve_tools` et les 4 de
   `apply_agent_tool_visibility` compilent.
7. V1 à V5 passent.
8. `test_mika_arch_disabled_tools_excludes_agent_self_state` est vert, et son
   diff ne contient que l'extraction en const — aucun nom ajouté, retiré ou
   réécrit, assertion et message inchangés (V3).
9. Le corps de PR porte les cinq sondes P1–P5 avec leur attendu, la bande
   3,5–5,5 Ko annoncée comme bande, et la procédure de reversion.
10. Le ticket de suivi du basculement dénylist → allowlist (§ 8 (a)) est
    **ouvert** par l'implémenteur en fin de vol, avec le relevé P2/P3 en corps,
    et le corps de PR le nomme par une ligne
    `Tracked in: senara-solutions/mika#<n>`. Un `Tracked in:` pointant vers
    mika#2593 ne satisfait pas ce critère : il ne nomme aucune suite.

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
  `test_mika_arch_disabled_tools_excludes_agent_self_state` est vert, et son diff
  ne contient **que** l'extraction en const décrite au § 5 D2 — aucun nom ajouté,
  retiré ou réécrit, assertion et message inchangés. Ce critère ne demande pas un
  diff *vide* : l'extraction est ce qui empêche les trois noms d'être retapés une
  seconde fois dans le même fichier (classe mika#2205), et la borne exacte de ce
  qui est autorisé est V3.
- **AC5 — La coupe n'est pas « au plus gros ».** Le plan nomme au moins un outil
  volumineux ou plausible qu'il **conserve** avec son motif — `list_agents`
  (§ 3.3) — et la famille d'instruments d'ancrage conservée par C3.
- **AC6 — La coupe est observable, et attribuable.** `kept_count`, `kept_bytes`
  **et `agent`** sur l'événement existant — le troisième parce que la ligne est
  émise par plusieurs agents et qu'une mesure non attribuable n'est pas une
  mesure (§ 5 D4). P3 lit `tool_defs_bytes` — le champ même de la précondition —
  comme arbitre du gain. Aucun nouveau nom d'événement n'est créé.
- **AC7 — Le gain cesse de s'éroder.** D3 rougit sur le prochain
  `register(...)` qui atteindrait mika-arch sans décision, dans les deux sens de
  divergence.
- **AC8 — La décision atteint un agent déjà provisionné.** `tools.disabled` est
  dans `CODE_OWNED_IDENTITY_SECTIONS`, et P1 le constate après déploiement.
- **AC9 — Ce qui n'est pas pris est nommé avec sa raison.** Le § 8 énumère six
  renvois, dont le basculement vers une allowlist (ticket de suivi ouvert en fin
  de vol, DoD 10) et le
  reliquat `system_prompt_bytes` nommé deux fois par l'opérateur.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/well_known_agents.rs` — D1 : 9 entrées + 2 sous-sections commentées + paragraphe de doc du critère | 52 |
| `crates/mika-agent/src/well_known_agents.rs` — D2 : table `MIKA_ARCH_ORPHANED_READS`, garde C1, allowlist vide + assertion auto-nettoyante | 78 |
| `crates/mika-agent/src/well_known_agents.rs` — D2 : extraction de `MIKA_ARCH_AGENT_SELF_STATE_TOOLS` + son doc-comment, et repointage du test #811 dessus | 5 |
| `crates/mika-agent/src/tools/mod.rs` — D3 : roster attesté de 18 noms + recensement bidirectionnel + doc de sa borne | 30 |
| `crates/mika-agent/src/agent_loop/mod.rs` — D4 : `kept_count` + `kept_bytes` + `agent` sur l'événement existant, les deux signatures et leurs 14 sites d'appel | 34 |
| `crates/mika-agent/src/agent_loop/mod.rs` — D5 : test de l'attestation d'octets + les 4 appels de test complétés | 32 |

Total estimé : 231 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Un seul vol, et voici pourquoi il tient.** 231 < 243, et 231 < 1000 : les deux
bornes passent, la plus serrée (243) avec ~5 % de marge. Trois propriétés
soutiennent l'estimation plutôt qu'un espoir :

- **Aucun site nouveau, et une seule sorte de changement mécanique.** Les
  livrables atterrissent dans trois fichiers qui existent, sur des sites qui
  existent : une const (D1), deux `mod tests` déjà peuplés (D2, D3, D5), et
  **trois champs** sur un `tracing::info!` déjà écrit (D4). Zéro module créé,
  zéro migration. La seule déclaration déplacée est l'extraction en const de D2,
  dont les trois noms sont déjà écrits à la ligne `well_known_agents.rs:2083`.
  **Deux signatures changent** — celles de `apply_agent_tool_visibility` et de
  `inject_skills_and_resolve_tools`, pour faire arriver le champ `agent` — ce qui
  touche 18 sites d'appel à raison d'**une ligne chacun**, dont 15 en test. C'est
  le poste le plus volumineux du vol et le moins risqué : il ne compile pas si
  l'un est oublié, donc il ne peut pas être livré à moitié.
- **Les deux détecteurs ont leur patron en dépôt.** D3 reprend la forme de
  divergence bidirectionnelle de
  `mika1952_gated_tools_match_the_reflection_contract_constant`
  (`tools/mod.rs:1550`) — **sans** son scanner `declared_tool_names`, puisque
  `default_tools()` est appelable sans argument et `definitions()` est `pub`
  (§ 5 D3) : c'est ce qui fait tomber son estimation de 48 à 30. D5 se range à
  côté des trois `test_apply_agent_tool_visibility_*` existants.
- **La part de doc-comment est forte et son volume est connu.** ~75 des 231 lignes
  sont du commentaire de déclaration (motif des deux sous-sections, critère,
  borne de D3), dont le contenu est déjà rédigé aux § 3.1 à § 3.3 de ce plan : il
  reste à le transcrire, pas à le trouver.

Le basculement dénylist → allowlist — qui touche le schéma `Identity.tools`, le
prédicat du filtre, `CODE_OWNED_IDENTITY_SECTIONS`, la réconciliation et leurs
gardes — est **hors périmètre** et motivé au § 8 (a). Il n'est pas un « vol 2 »
de ce plan : ce travail-ci est complet et mergeable seul, et le basculement
**remplace** le prédicat du filtre que celui-ci se contente de nourrir. Son
ticket de suivi est ouvert en fin de vol et nommé par `Tracked in:` dans le corps
de PR (DoD 10).
