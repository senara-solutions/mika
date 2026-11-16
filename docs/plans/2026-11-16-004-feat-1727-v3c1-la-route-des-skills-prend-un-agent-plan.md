# mika#1727 — vol V3c-1 : la route des skills prend un agent

- **Ticket** : senara-solutions/mika#1727 — *TUI as thin HTTP client of mika-spirit*
- **Vol** : V3c-1 (le premier des deux vols de V3c — côté serveur ; V3c-2 fera lire la route à la TUI)
- **Branche** : `feat/1727/v3c1-la-route-des-skills-prend-un-agent`, neuve depuis `main`
- **Base mesurée** : `main` @ `850b7acb` (le merge de V3b-2, mika#3099) — la tête sur laquelle MPC a mesuré, et la tête de cette branche
- **Label** : `loop-substrate` (plafond de tours : 200)
- **Borne du vol** : au plus 450 lignes ajoutées, plan exclu, par `git diff --numstat origin/main...HEAD`
- **La PR porte `Refs mika#1727`**, jamais `Closes`, `Fixes` ni `Resolves` — ni dans le corps, ni dans un commit. Le ticket se ferme au merge de V4.

---

## 0. Pourquoi ce vol tient en un seul vol

Tout le changement de production vit dans **une fonction de 45 lignes**,
`handle_skills_list`. Elle prend aujourd'hui le premier agent d'une `DashMap` et
sert six champs ; elle prendra un agent nommé et en servira treize. Il n'y a ni
nouvelle route, ni nouveau type public, ni migration, ni lecteur à migrer —
**personne ne lit cette route aujourd'hui** (§ 1), donc le vol n'a pas de
compatibilité descendante à porter.

Le volume est ailleurs : dans les **fixtures de test**. Le module `tests` de
`server/mod.rs` ne sait construire qu'un état **à un seul agent**, et le bloc
`AgentState { … }` est inliné dans `test_state_full`. Un test qui distingue deux
agents demande d'en sortir une fabrique — c'est ce que la tête du ticket exige,
et c'est le poste le plus cher du vol (§ *Taille estimée*), parce qu'un bloc
déplacé compte comme ajouté dans `--numstat`.

L'estimation est de **398 lignes** dans l'unité de la borne du vol, et de **392**
dans celle de l'architecte (§ *Taille estimée*) : **sous 450 et sous 429, tests
compris**. Le groom ne demande donc aucune coupe, et n'en pratique aucune de
lui-même.

---

## 1. Ce qui est mesuré sur main (@ `850b7acb`)

### `crates/mika-agent/src/server/dashboard.rs`

`SkillsListQuery` (l. 1264-1268) porte deux champs, `source` et `always_on`.

`handle_skills_list` (l. 1270-1314) :

- **l. 1277-1289** — `state.agents.iter().next()` : le premier agent de la
  `DashMap`, dont l'ordre d'itération n'est pas défini. Le commentaire sur place
  explique l'extraction hors du scrutinee (`clippy::sig_drop`, #1724) ; il
  disparaît avec la ligne, puisque `resolve_agent` gère son propre guard.
  Aucun agent ⇒ **500** `{"error": "no agents configured"}`.
- **l. 1291-1294** — `SkillListFilter { source, always_on }`, les deux parsés en
  `Option`, une valeur invalide valant « pas de filtre » (mika#606).
- **l. 1296-1313** — `apply_filter` puis, par skill, six champs : `name`,
  `description`, `origin` (`is_bundled_skill` ⇒ `built-in` / `marketplace`),
  `enabled`, `always_on`, `tools` (**un compte**, `entry.skill_tools.len()`).
- La racine est `{"skills": [...]}` — rien d'autre.

### `crates/mika-agent/src/server/mod.rs`

- **l. 263** — `.route("/skills", get(dashboard::handle_skills_list))`. Elle ne
  change pas : le paramètre voyage en query string, pas dans le chemin.
- **l. 363-370** — la route est sous `require_dashboard_or_internal_token` : le
  jeton interne y passe en superutilisateur (`auth.rs` l. 53-66), et les tests du
  module posent `authorization: Bearer test-token-secret` (motif en place
  l. 4902-4920).
- **l. 2496-2578** — `test_state_full(agent_settings, tier, home_dir, model_drift)`
  crée un `AsyncDatabase`, le clone en `dashboard_db`, **inline** un
  `AgentState { … }` de 48 lignes, l'insère sous la clé `"mika"` dans une
  `DashMap` neuve, puis **inline** un `AppState { … }` de 32 lignes avec
  `default_agent: "mika"`. Deux appelants indirects : `test_state()` et
  `test_state_with_settings_and_tier` (l. 2363, 2475).
- **l. 2582-2584** — `test_app(state) = build_router(state)`.
- **Aucun test ne vise `/api/v1/skills`** : `grep` sur `api/v1/skills` dans le
  module ne rend rien.

### `crates/mika-agent/src/server/state.rs` (lu, **non modifié**)

`AppState::resolve_agent(&self, name: &str) -> Option<Arc<AgentState>>` (l. 246) :

1. `name.is_empty()` ⇒ `self.default_agent`, sinon `name` ;
2. `agent::normalize_agent_name` (`mika-common`, l. 37 : `trim().to_lowercase()`) ;
3. fast path `self.agents.get(&normalized)` ;
4. slow path (#1399) : `identity.toml` sur disque **et** ligne en base, puis
   garde de tier (mika#1962), puis `init_agent`. Absent ⇒ `None`.

Elle rend l'`Arc<AgentState>` et **pas le nom résolu** — ce point commande D2.

### Ce que la TUI lit du registre, et que V3c-2 lira sur cette route

Mesuré dans `crates/mika-cli/src/tui/commands/handlers.rs` :

| surface TUI | champs lus |
|---|---|
| autocomplétion `/skill` | `name`, `description` |
| `/skills` (l. 944-977) | `enabled`, `always_on`, `skill_tools.len()`, `has_override`, `variant_count()`, et `skipped()` au niveau registre |
| `/skill <nom>` (l. 1085-1123) | `version`, `timeout_secs`, `triggers.keywords`, les **noms** d'outils, `entry.dir`, puis `variant_count()` + `variant_providers()` + `variant_models(p).len()` |

Les accesseurs existent tous et sont publics : `SkillEntry::variant_providers()`
(`skills/index.rs` l. 254), `variant_models()` (l. 276), `variant_count()`
(l. 296) ; `SkillRegistry::skipped()` (`skills/mod.rs` l. 363) rend
`&[SkippedSkill]`, dont les champs `name` et `reason` sont publics
(`index.rs` l. 394-400). **Rien à ajouter sous `skills/`.**

### Écarts entre la tête du ticket et la mesure

- **É1 — l'état de test porte trois agents, pas deux.** La tête dit « un état à
  deux agents ». Les champs de `SkillRegistry` sont **privés** (`skills/mod.rs`
  l. 280-287) et l'API publique n'offre **aucun** constructeur qui combine des
  entrées *et* des `skipped` : `from_test_entries(entries)` laisse `skipped`
  vide, `with_skipped(skipped)` laisse `skills` vide. Combiner les deux
  demanderait de toucher `skills/mod.rs`, que la tête place hors périmètre. Le
  vol ajoute donc un **troisième** agent dont le registre ne porte que des
  `skipped`, pour que le critère 4 ne soit pas vacant sur ce champ. Les critères
  1 à 3 n'interrogent que les deux premiers : rien n'est affaibli. Voir D5.
- **É2 — le cas « aucun agent » passe de 500 à 404.** La branche
  `no agents configured` disparaît avec l'itération sur la `DashMap` : un défaut
  introuvable est désormais un agent introuvable. Aucun lecteur n'existe (§ 1),
  donc le changement de statut ne casse rien. Voir D3.

---

## 2. Décisions

### D1 — `agent` est un paramètre de query, pas de chemin

`SkillsListQuery` gagne `pub agent: Option<String>`. La ligne de route
(`server/mod.rs` l. 263) **ne change pas**, et aucune route
`/agents/{id}/skills` n'est ajoutée : la tête la place explicitement hors vol, et
l'ajouter ouvrirait une seconde représentation de la même lecture — exactement la
duplication que mika#1727 ferme.

### D2 — le nom résolu est recalculé dans le handler, et c'est assumé

Le champ racine `agent` doit nommer l'agent **effectivement résolu** (critère 3),
mais `resolve_agent` rend un `Arc<AgentState>` et l'`AgentState` ne porte pas son
nom. Les deux issues propres — faire rendre le nom à `resolve_agent`, ou poser un
champ `name` sur `AgentState` — touchent `server/state.rs`, que la tête déclare
intact.

Le handler rejoue donc la règle de défaut de `resolve_agent`, **en deux lignes** :

```rust
let requested = q.agent.as_deref().unwrap_or("");
let resolved_name = if requested.is_empty() {
    state.default_agent.clone()
} else {
    mika_common::agent::normalize_agent_name(requested)
};
```

C'est une duplication, elle est nommée ici, et un commentaire sur place dit qu'elle
est **la trace d'une contrainte de périmètre** et non un choix de conception : le
jour où `state.rs` s'ouvre, le geste est de faire rendre le nom à `resolve_agent`
et de supprimer ces deux lignes. Le critère 3 la tient : il demande le nom pour un
paramètre absent, pour un paramètre vide **et** pour un autre agent résolu, donc
une dérive entre les deux règles rougit.

`resolve_agent` reste l'unique **résolveur** : le handler lui passe `requested`
brut et ne décide rien de l'agent servi.

### D3 — l'agent introuvable rend 404, dans la forme de `handle_agent_detail`

```json
{"error": "agent '<nom demandé>' not found"}
```

Même formulation que `handle_agent_detail` (`dashboard.rs` l. 157-164) : deux
routes du même fichier qui refusent le même objet doivent le refuser dans la même
forme. Le corps **ne porte pas** de champ `skills` — pas de liste vide, qui se
lirait comme « cet agent n'a pas de skills » alors qu'il n'existe pas (critère 2).
Le nom rapporté est celui **demandé**, pas le normalisé : c'est ce que l'appelant
a écrit, et c'est ce qu'il doit corriger.

Conséquence É2 : le 500 `no agents configured` disparaît. Un serveur sans agent
voit son `default_agent` échouer à résoudre et rend 404 — un diagnostic plus
exact que « erreur serveur », puisque rien n'est cassé.

### D4 — treize champs par skill, et la forme de `variants`

Les six champs actuels restent, **à leur nom actuel** — `tools` reste le compte,
et les noms arrivent à côté sous `tool_names`. Renommer `tools` n'achèterait rien
et casserait le seul contrat que la route a déjà publié.

Les sept ajoutés, un par ligne du tableau du § 1 :

| champ | source |
|---|---|
| `has_override` | `entry.has_override` |
| `version` | `entry.manifest.skill.version` |
| `timeout_secs` | `entry.manifest.skill.timeout_secs` (la valeur **racine**, pas `effective_timeout`) |
| `keywords` | `entry.manifest.triggers.keywords` |
| `tool_names` | `entry.skill_tools[].definition.name` |
| `path` | `entry.dir.display().to_string()` |
| `variants` | `{"total": variant_count(), "providers": {"<p>": <nb modèles>}}` |

**`timeout_secs` est la valeur racine** parce que `effective_timeout(provider,
model)` demande un couple que la route n'a pas : le provider servant la session
appelante n'est pas connu ici, et en inventer un servirait un nombre faux avec
autorité. C'est aussi ce que la TUI affiche aujourd'hui (`handlers.rs` l. 1093,
`m.skill.timeout_secs`).

**`variants` est un objet et non deux champs plats** parce que `/skill <nom>`
affiche « N total (p1 (k models), p2) » : le total et la ventilation sont lus
ensemble, et les séparer obligerait le lecteur à les recoller. `providers` est
une map `provider → nombre de modèles` ; un provider sans modèle y figure avec
`0`, ce qui est la distinction que la TUI rend déjà (« p2 » sans parenthèse).

### D5 — `skipped` à la racine, et le troisième agent (É1)

La racine porte `{"agent": …, "skills": […], "skipped": [{"name", "reason"}]}`.
`skipped` est une propriété du **registre**, pas d'une skill : il nomme ce qui
n'a pas pu être chargé. Il n'est **pas** filtré par `source`/`always_on` — une
skill ignorée n'a pas de manifeste lisible sur lequel un filtre porterait.

Le critère 4 demande que le test pose « une skill ignorée ». Faute de
constructeur public combinant entrées et `skipped` (É1), c'est un **troisième
agent**, de registre `SkillRegistry::with_skipped(…)`, qui porte le cas. Le test
du critère 4 fait donc deux requêtes : l'une sur l'agent aux entrées (les treize
champs, `skipped` vide), l'autre sur l'agent aux ignorées (`skipped` peuplé,
`skills` vide). Le couplage registre → réponse est pinné **des deux côtés** : si
le handler lisait le `skipped` d'un autre agent, la première moitié rougirait.

### D6 — une fabrique d'`AgentState`, et l'état multi-agents dérive de l'existant

Deux gestes, pas trois :

1. Le bloc `AgentState { … }` sort de `test_state_full` dans
   `fn test_agent_state(db, agent_settings, tier, home_dir, model_drift, skills)
   -> AgentState`. `test_state_full` l'appelle avec `SkillRegistry::empty()` et
   garde tout le reste — la `DashMap`, le `default_agent: "mika"`, le bloc
   `AppState`. **Six paramètres**, sous le seuil de `clippy::too_many_arguments`.
2. `test_state_skills_multi()` **ne recopie pas** le bloc `AppState` : il part de
   `test_state()`, insère ses agents dans la `DashMap` déjà là et déplace
   `default_agent`. `AppState` est une struct à champs publics et `agents` est
   une `DashMap` : les deux mutations sont possibles depuis le module.

Une fabrique d'`AppState` aurait coûté une quarantaine de lignes de plus pour
rien. L'agent `"mika"` de `test_state()` survit avec un registre **vide**, et
c'est un bénéfice : il rend le test du critère 1 non vacant même si l'ordre de la
`DashMap` plaçait `"mika"` en tête.

Le nom de l'agent **n'est pas** un paramètre de la fabrique : `budget_record`
reste résolu pour `"mika"` comme aujourd'hui. La route des skills ne lit pas ce
champ, et le paramétrer ajouterait un septième argument sans lecteur. Un
commentaire sur place le dit, pour qu'un futur test de budget multi-agents sache
que c'est ici qu'il faut ouvrir.

Les entrées de skill passent par **une seule** fabrique,
`test_skill_entry(name, always_on, keywords, tool_names)`, qui rend un
`SkillEntry` complet (tous les champs du struct `index.rs` l. 132-159), et le
registre par `SkillRegistry::from_test_entries`. Aucune fixture n'est recopiée.

### D7 — pourquoi ce vol ne porte pas de section `## Fire-Disposition`

La règle mika#2306 arme la section sur la livraison d'un **détecteur** : « tout
code dont le chemin de succès est *aucune violation trouvée* » — lint, garde CI,
scan structurel, garde EndTurn, validateur de schéma.

Ce vol n'en livre aucun. Ses quatre tests sont des tests de **comportement d'une
route** : leur chemin de succès est « la réponse porte ce contenu », et ils
échouent en nommant un champ manquant ou un agent servi à tort, jamais en
dénombrant des violations dans le dépôt. Le critère 1 est un test de régression
négatif — il rougit sur `main` — mais un test négatif n'est pas un scanner : il
n'a ni population à parcourir, ni allowlist à tenir, ni risque de passer vert en
ne lisant rien, puisque chacune de ses assertions nomme une valeur attendue.

Le vol V3b-2 (mika#3099), lui, livrait une garde lexicale sur deux fichiers de
production et portait la section. La différence est celle-là, et elle est la
seule. La règle dit « si le plan ne livre AUCUN détecteur, la section n'est pas
requise : ne l'invente pas » — ce paragraphe est le raisonnement, visible, qui
tient la place d'une section qui serait vide.

### D8 — ce que ce vol ne fait pas

- **Rien côté TUI.** La lecture de la route, le retrait de l'assemblage du
  registre dans le worker, `updated_skills`, l'entrée du manifeste d'egress :
  V3c-2. Aucun fichier de `crates/mika-cli/` ne change (critère 5).
- **Pas le bout-en-bout de `SetModel`** (écart É1 de V3b-1) : il demande
  d'extraire la boucle du worker, et V3c-2 le porte.
- **Pas de route `/agents/{id}/skills`** (D1).

La suite se note `Tracked in: senara-solutions/mika#1727` dans le corps de la PR.
**Ce vol n'ouvre aucun ticket** (gel jusqu'au 01/12) : un manque ou un reste va en
commentaire sur mika#1727 ou dans la PR.

---

## 3. Ordre de commit

Un commit par garde, **après que la garde est verte avec son test**, et non en fin
de vol. Les tests ne se reportent jamais à un vol suivant. C'est l'ordre suggéré
par la tête du ticket, et ce plan le confirme.

### C1 — la route prend un agent (critères 1, 2, 3)

`crates/mika-agent/src/server/dashboard.rs` :

- `SkillsListQuery` gagne `pub agent: Option<String>`, avec la ligne de doc qui
  dit le défaut.
- `handle_skills_list` : les l. 1277-1289 (itération `DashMap` + 500) sont
  remplacées par le bloc `requested` / `resolved_name` / `resolve_agent` / 404 de
  D2 et D3. Le filtre et la boucle restent tels quels pour ce commit ; la racine
  gagne `"agent": resolved_name`.

`crates/mika-agent/src/server/mod.rs`, module `tests` uniquement :

- `test_agent_state(…)` extraite de `test_state_full` (D6), et `test_state_full`
  réécrit pour l'appeler.
- `test_skill_entry(name, always_on, keywords, tool_names)` (D6).
- `test_state_skills_multi()` : trois agents — `skills-a` (deux entrées, dont une
  `always_on`), `skills-b` (une entrée distincte), `skills-skipped`
  (`with_skipped`) — insérés dans l'état de `test_state()`, `default_agent`
  déplacé sur `skills-a`.
- un helper `skills_get(app, query) -> (StatusCode, serde_json::Value)` qui pose
  le `Bearer test-token-secret` et désérialise le corps, pour que les quatre
  tests ne recopient pas seize lignes de `Request::builder` chacun.
- **test du critère 1** — un seul test : `?agent=skills-a` rend les noms de `a`
  et **aucun** de `b`, `?agent=skills-b` l'inverse, puis
  `?agent=skills-a&always_on=true` rend la seule entrée `always_on` de `a`. C'est
  le test négatif du vol : sur `main`, la route ignore `agent` et l'une des deux
  moitiés échoue quel que soit l'agent que la `DashMap` sert en tête.
- **test du critère 2** — `?agent=<nom absent>` rend 404, `error` porte le nom
  demandé, et le corps **n'a pas** de clé `skills`. Le nom choisi est
  invraisemblable sur disque (`agent-absent-v3c1`), parce que le slow path de
  `resolve_agent` consulte `~/.mika/agents/<nom>/identity.toml` sous le
  `global_home_dir` de l'état de test (`/tmp/mika-test`) : un nom banal pourrait
  exister sur une machine de dev et rendre le test vert pour la mauvaise raison.
  Une assertion le dit sur place.
- **test du critère 3** — sans paramètre **et** avec `?agent=`, la réponse porte
  les skills de `skills-a` et `agent == "skills-a"` ; une troisième requête
  `?agent=skills-b` vérifie que `agent` nomme **l'agent résolu** et non le défaut.

Garde verte : `cargo test -p mika-agent server::tests` et
`cargo clippy --all-targets -p mika-agent`.

### C2 — les champs élargis et `skipped` (critère 4)

`dashboard.rs` : la fermeture de `map` gagne les sept champs de D4 ; la racine
gagne `"skipped"` (D5). Les commentaires portent le **pourquoi** : la provenance
des champs (le tableau du § 1 — ce sont ceux que V3c-2 lira, pas un élargissement
spéculatif), la raison du `timeout_secs` racine, et la raison de la forme objet
de `variants`.

`server/mod.rs`, module `tests` : `test_skill_entry` est appelée avec un outil, un
mot-clé et un `provider_overrides` non vide pour que `variants.total` et
`variants.providers` soient non vacants ; **test du critère 4** — deux requêtes
(D5) relisant chacun des treize champs et les deux champs de racine.

Garde verte : mêmes commandes.

### C3 — la doc (critère 7, `claude_md_size` compris)

`crates/mika-agent/CLAUDE.md`, **l. 624**, paragraphe « Skills listing (#606) » :
réécrit pour dire le paramètre `agent`, son défaut (l'agent par défaut du
serveur), le 404 sur agent inconnu, les champs ajoutés et les deux champs de
racine. Le fichier pèse **94 341 octets pour un plafond de 100 000** : le
paragraphe se réécrit, il ne s'ajoute pas, et le vol lui concède **six lignes**
(§ *Taille estimée*). La référence du ticket passe à « (#606, mika#1727 V3c-1) ».

Garde verte : `cargo test -p mika-common --test claude_md_size`.

### Fin de pipeline

Dans cet ordre, avant d'ouvrir la PR :

1. `cargo clippy --all-targets -p mika-agent`
2. `cargo test -p mika-agent`
3. `cargo test -p mika-common --test claude_md_size`
4. `bash scripts/verify-egress-manifest.sh --report`
5. `bash scripts/verify-pipeline.sh origin/main`
6. `git diff --numstat origin/main...HEAD` — la mesure de la borne, plan exclu
7. `git diff --name-only origin/main...HEAD` — critère 5 : la liste doit tenir en
   quatre chemins (`dashboard.rs`, `server/mod.rs`, `CLAUDE.md`, le plan), et
   **aucun** sous `crates/mika-cli/`

**Si (4) ou (5) est rouge, le pilote s'arrête et le dit** — il n'ouvre pas la PR
et ne répare pas un rouge qu'il n'a pas causé.

Le corps de la PR porte `Refs mika#1727`, `Tracked in: senara-solutions/mika#1727`,
`Code review: delegated to mika-qa (mika#2694)`, et **aucun** `Closes`, `Fixes`
ni `Resolves` (critère 8). Si le gabarit de reprise (`wip-rescue`) ouvre la PR, il
écrit `Closes #N` par défaut : celui qui le voit le corrige **avant toute revue**,
puis vérifie que `gh pr view <N> --json closingIssuesReferences` rend `[]`.

### Halte du pilote sur la borne

Après **chaque** commit, le pilote mesure `git diff --numstat origin/main...HEAD`,
plan exclu.

- **Total > 450 ⇒ halte.** Pas de commit suivant, pas de PR. Dernier message :
  « HALTE borne : N lignes pour 450 », avec le compte par commit.
- **Total > 405 après C1 ⇒ halte**, dite de la même façon : le reste ne tiendrait
  pas. C1 est estimé à 265 lignes, donc 405 laisse 140 de dérive avant que la
  halte morde — si elle mord, c'est que les fixtures ont coûté le double du
  prévu, et c'est exactement le signal qu'il faut remonter plutôt qu'absorber.

Les commentaires comptent dans la borne et **ne se rabotent pas** pour la tenir.
Un commentaire dit le pourquoi qu'on ne lit pas dans le code — ici : la
duplication assumée de D2, la provenance des treize champs, la raison du
`timeout_secs` racine, le nom en dur du `budget_record`. Si le code écrit dépasse
la borne, le pilote s'arrête selon la halte ci-dessus ; il ne retire pas après
coup les commentaires qui portent un pourquoi.

### Si le code s'écarte de ce plan

La QA juge contre **ce plan**, pas contre le corps du ticket. Si le pilote doit
s'écarter d'un point — un fichier, un critère, un nom, la forme de `variants`, le
nombre d'agents de la fixture — **il amende ce plan dans un commit `docs(plans)`
avant de pousser le code qui s'en écarte**, en nommant le point et la raison. Un
plan qui dit une chose pendant que le code en fait une autre est bloqué par la QA,
même quand le code est juste.

Si un hook ou un classifieur refuse un geste, au pilote ou à un sous-agent, il
s'arrête et le déclare, même si le refus semble être un faux positif. Il ne le
contourne jamais par un autre outil : pas de heredoc Bash à la place de Write, pas
de reformulation (incident mika#2869 du 2026-10-21).

---

## 4. Definition of Done

- `GET /api/v1/skills?agent=<nom>` sert les skills de l'agent **nommé**, résolu
  par `AppState::resolve_agent`, et plus jamais celles du premier agent de la
  `DashMap`.
- Un paramètre absent ou vide vaut l'agent par défaut du serveur ; la racine
  `agent` nomme l'agent effectivement résolu dans les trois cas.
- Un agent inconnu rend 404 avec le nom demandé dans `error`, sans champ `skills`.
- Les filtres `source` et `always_on` s'appliquent, inchangés, à l'agent résolu.
- Chaque skill servie porte les treize champs de D4 ; la racine porte `skipped`.
- Les quatre tests vivent dans le module `tests` de `server/mod.rs`, passent par
  `test_app()`, et ne recopient aucune fixture : une fabrique d'`AgentState`, une
  fabrique d'entrée de skill, un helper de requête.
- La ligne de route de `server/mod.rs` (l. 263) est inchangée ; aucun fichier de
  `crates/mika-cli/` ne change ; aucun fichier du plan de contrôle n'est touché.
- Le paragraphe « Skills listing (#606) » de `crates/mika-agent/CLAUDE.md` dit le
  paramètre, son défaut, le 404 et les champs ajoutés, sans faire franchir au
  fichier son plafond de 100 000 octets.
- `cargo clippy --all-targets -p mika-agent`, `cargo test -p mika-agent` et
  `cargo test -p mika-common --test claude_md_size` sont verts ;
  `verify-egress-manifest.sh --report` et `verify-pipeline.sh origin/main` ont été
  lancés avant la PR.
- La PR porte `Refs mika#1727` et aucun `Closes`, `Fixes` ni `Resolves`.

---

## 5. Acceptance criteria

Transcrits de la tête de mika#1727 (§ *Critères de V3c-1*).

1. Sur un état à deux agents aux skills distinctes, `GET /api/v1/skills?agent=<a>`
   rend les skills de `a` et aucune de `b`, et `?agent=<b>` l'inverse, dans le
   même test. Avec `?agent=<a>&always_on=true`, le filtre s'applique aux skills
   de `a`. Ce test-ci est le test négatif du vol : sur main, l'une des deux
   moitiés échoue, puisque la route ignore `agent`.
2. `GET /api/v1/skills?agent=<inconnu>` rend 404, avec le nom demandé dans `error`
   et aucun champ `skills`.
3. Sans paramètre `agent`, ou avec `agent=` vide, la route rend les skills de
   l'agent par défaut du serveur, et le champ racine `agent` le nomme, y compris
   quand un autre agent est résolu.
4. Chaque skill servie porte, en plus des six champs actuels, `has_override`,
   `version`, `timeout_secs`, `keywords`, `tool_names`, `path` et `variants`
   (`total` et nombre de modèles par provider). La racine porte `skipped`, une
   liste de `{ name, reason }`. Le test pose une entrée avec au moins un outil,
   un mot-clé et une skill ignorée, et relit chaque champ.
5. Seuls changent les fichiers listés plus haut. La ligne de route de
   `server/mod.rs` (l. 263) est inchangée, et aucun fichier de `crates/mika-cli/`
   ne change.
6. Aucun `allow` ni `expect` neuf n'apparaît dans `crates/mika-agent/`.
7. `cargo clippy --all-targets -p mika-agent` est vert. `cargo test -p mika-agent`
   est vert, de même que `cargo test -p mika-common --test claude_md_size`. Le
   pilote lance `bash scripts/verify-egress-manifest.sh --report` et
   `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il trouve
   l'un des deux rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si le gabarit de reprise l'ouvre.

**Sur les critères 1 et 4, ce plan livre davantage que la lettre de la tête** :
l'état de test porte **trois** agents et non deux (É1/D5), le troisième ne
servant qu'à rendre `skipped` non vacant. Les critères 1 à 3 restent jugés sur les
deux premiers.

---

## Taille estimée

| livrable | lignes estimées |
|---|---|
| `dashboard.rs` — `SkillsListQuery`, résolution, 404, racine `agent` (C1) | 38 |
| `dashboard.rs` — les sept champs, `variants`, `skipped`, commentaires (C2) | 47 |
| `server/mod.rs` tests — `test_agent_state` extraite (bloc + signature + doc) (C1) | 62 |
| `server/mod.rs` tests — `test_state_full` réécrit autour de la fabrique (C1) | 12 |
| `server/mod.rs` tests — `test_skill_entry` (C1) | 45 |
| `server/mod.rs` tests — `test_state_skills_multi` à trois agents (C1) | 28 |
| `server/mod.rs` tests — helper `skills_get` (C1) | 20 |
| test du critère 1 — deux agents, deux sens, filtre `always_on` (C1) | 50 |
| test du critère 2 — 404, `error`, absence de `skills` (C1) | 22 |
| test du critère 3 — absent, vide, et un autre agent résolu (C1) | 28 |
| test du critère 4 — treize champs et `skipped`, deux requêtes (C2) | 40 |
| `crates/mika-agent/CLAUDE.md` — paragraphe « Skills listing » réécrit (C3) | 6 |

Total estimé : 398 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités diffèrent, et les deux tiennent.** L'unité de la borne du vol
(`git diff --numstat origin/main...HEAD`, plan exclu, doc comprise) compte les
**398** lignes ci-dessus, pour une borne de **450**. L'unité de l'architecte
exclut `docs/` et les `CLAUDE.md`, donc les 6 lignes de C3 : elle compte **392**,
pour une borne de **429**. **398 pour 450, et 392 pour 429 : les deux bornes
tiennent, tests compris.** Le groom ne demande donc aucune coupe — et il n'en
pratique aucune de lui-même : une coupe demande le mot de Vincent.

Répartition par commit : **C1 ≈ 265**, **C2 ≈ 87**, **C3 ≈ 6**. C'est ce qui règle
le second seuil de halte : C1 à 265 laisse 140 lignes de dérive avant les 405.

Marges : **52 lignes** sous la borne du vol, **37** sous celle de l'architecte.
Deux choses les rendent tenables.

D'abord, **le poste dominant est du déplacement, pas de l'écriture**. Sur les 265
lignes de C1, 62 sont le bloc `AgentState` qui change de fonction sans changer de
contenu, et 12 la réécriture de `test_state_full` autour : `--numstat` les compte
comme ajoutées parce qu'il ne sait pas lire un déplacement, mais elles ne
demandent aucune décision au pilote et ne peuvent donc pas déraper. C'est aussi
pour cela que la fabrique d'`AppState` n'est pas extraite (D6) : elle aurait
ajouté une quarantaine de lignes de la même espèce, pour rien.

Ensuite, **le poste le plus incertain — les tests — est le plus découpé** : quatre
tests de 22 à 50 lignes, dont trois tombent dans C1. Si l'un d'eux dépasse, la
mesure après C1 le dit avant que C2 soit écrit, et le second seuil de halte (405)
mord là plutôt qu'à l'ouverture de la PR.

Le changement de production, lui, est petit et borné par construction : **85
lignes dans une fonction de 45**, sans nouveau type, sans nouvelle route, sans
migration et sans lecteur à migrer.
