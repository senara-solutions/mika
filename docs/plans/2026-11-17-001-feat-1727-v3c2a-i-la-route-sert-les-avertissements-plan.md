# Plan — mika#1727, vol V3c-2a-i : la route sert les avertissements

- **Ticket** : senara-solutions/mika#1727
- **Vol** : V3c-2a-i — la route sert les avertissements (côté serveur seulement)
- **Branche** : `feat/1727/v3c2a-i-la-route-sert-les-avertissements`
- **Base** : `main` @ `210f8dd8` (« feat(1727): la route des skills prend un agent (vol V3c-1) », PR #3100)
- **Label** : `loop-substrate` (plafond de tours 200)
- **Borne** : 170 lignes ajoutées dans l'unité du vol, 155 estimées dans l'unité de l'architecte
- **PR** : `Refs mika#1727`. **Jamais** `Closes`, `Fixes` ni `Resolves` — ni dans le corps, ni dans un message de commit. Le ticket se ferme au merge de V4.

Ce plan part du plan 005
(`docs/plans/2026-11-16-005-feat-1727-v3c2a-la-route-des-skills-a-son-client-plan.md`,
sur `feat/1727/v3c2a-la-route-des-skills-a-son-client`), **sans nouvelle passe de
mesure côté serveur**, comme le ticket le demande. Il en reprend la mesure de
`dashboard.rs` et de `skills/`, l'écart É-A, les décisions D1, D2 et D8, et le
commit C1. Il n'en reprend **ni** le client, **ni** l'egress, **ni** la halte de
son § 0, **ni** les coupes de son § 4. Les numéros de ligne sont recalés sur
`main` — et, mesure faite, **aucun n'a bougé** : `main` est à `210f8dd8`, qui est
la base sur laquelle le plan 005 avait mesuré.

---

## § 1. Ce qui est sur main

### 1.1 `handle_skills_list` (`crates/mika-agent/src/server/dashboard.rs` l. 1274-1381)

Le handler Axum prend `State(AppState)` + `Query(SkillsListQuery)` et rend
`impl IntoResponse`. Trois faits portent ce vol :

- **Le registre est cloné hors du verrou** (l. 1320) :
  `let registry = agent_state.skills.lock().unwrap().clone();`. Tout ce qui suit
  lit une valeur locale, donc ajouter une lecture ne rallonge pas la section
  critique et ne touche pas `server/state.rs`.
- **`skills` est l'itérateur filtré** (l. 1321-1364) : `apply_filter(registry.skills().iter(), &filter)` puis
  `.map(…)` sur treize champs (`name`, `description`, `origin`, `enabled`,
  `always_on`, `tools`, `has_override`, `version`, `timeout_secs`, `keywords`,
  `tool_names`, `path`, `variants`).
- **`skipped` n'est pas filtré, et son commentaire dit pourquoi** (l. 1366-1374) :

  ```rust
  // `skipped` is a property of the REGISTRY, not of a skill: it names what
  // could not be loaded. It is deliberately NOT filtered by `source` /
  // `always_on` — a skipped skill has no readable manifest for a property
  // filter to bear on.
  ```

- **La racine ne porte que trois clés** (l. 1376-1380) :
  `{"agent": resolved_name, "skills": skills, "skipped": skipped}`. **C'est le
  trou que ce vol comble**, et le test du critère 1 est négatif sur `main` à
  cause de cette ligne-là.

`registry.validated_warnings()` est disponible exactement là où
`registry.skipped()` est déjà lu, sur la même valeur clonée.

### 1.2 D'où viennent les avertissements (`crates/mika-agent/src/skills/`)

- `SkillRegistry::validated_warnings()` — `skills/mod.rs` l. 379 — rend
  `&[SkillValidationWarning]`.
- `SkillValidationWarning { skill_name: String, diagnostics: Vec<SkillDiagnostic> }`
  — `skills/index.rs` l. 415-418. Chaque `SkillDiagnostic` porte un `level`
  (`DiagnosticLevel::{Ok, Warn, Fail}`) et un `message`.
- **Aucun constructeur de `SkillRegistry` ne pose d'avertissement** : `from_dir()`
  (l. 294), `empty()` (l. 305), `from_test_entries()` (l. 315) et
  `with_skipped()` (l. 325) écrivent tous `validated_warnings: Vec::new()`.
- **Le seul producteur est `apply_load_safety_check()`** — `skills/mod.rs`
  l. 396-475 —, **qui lit le disque**. Sa phase 1 répartit chaque skill entre
  `to_skip` et `to_warn` ; sa branche `else` (l. 451-465) log chaque diagnostic
  puis `to_warn.push(SkillValidationWarning { skill_name, diagnostics: issues })` ;
  sa phase 2 écrit `self.validated_warnings = to_warn;` (l. 474).
- **`issues` est déjà filtré à `Warn | Fail`** (l. 417-420) : les `Ok` n'y
  entrent pas. Une skill qui part dans `to_warn` reste **chargée** — c'est la
  différence avec `skipped`.

### 1.3 Écart É-A — `test_skill_entry` ne peut pas produire un avertissement

Le banc de V3c-1 construit ses registres par `SkillRegistry::from_test_entries`
sur des `test_skill_entry` (`server/mod.rs` l. 2629), dont le `dir` vaut
`/tmp/mika-test/skills/<name>` — un chemin qui n'existe pas. Si l'on faisait
passer un tel registre par `apply_load_safety_check()`, `validate_skill` rendrait
`skill.toml not found`, un `Fail` que `is_skip_worthy_failure` (`skills/index.rs`
l. 439-466) retient : la skill partirait dans **`skipped`**, pas dans
`validated_warnings`. **Un avertissement non fatal exige un répertoire réel.**
C'est ce que D2 résout, et c'est le seul point dur du vol.

### 1.4 Le banc de V3c-1 (`crates/mika-agent/src/server/mod.rs`, module `tests`)

- `test_agent_state` l. 2558, `test_app` l. 2611, `test_skill_entry` l. 2629.
- `test_state_skills_multi()` l. 2707 : `home = /tmp/mika-test`,
  `tier = AgentTier::Default`, un tableau `[(&str, SkillRegistry); 3]` —
  `skills-a` (`alpha-always` always_on + `alpha-on-demand`), `skills-b`
  (`beta-only`), `skills-skipped` (`with_skipped([SkippedSkill { name: "broken-skill",
  reason: "manifest parse error" }])`) —, une boucle d'insertion dans
  `state.agents`, et `state.default_agent = "skills-a"` (l. 2752).
- `skills_get(app, query)` l. 2758 : `GET /api/v1/skills{query}` avec
  `authorization: Bearer test-token-secret`, rend `(StatusCode, serde_json::Value)`.
- `skill_names(body)` l. 2776.
- **Les quatre tests de V3c-1 (l. 2801-2924) visent `skills-a`, `skills-b` et
  `skills-skipped` nommément** : ajouter un quatrième agent au banc ne touche
  aucune de leurs assertions (critère 3).

### 1.5 Le cap de `CLAUDE.md`

`CLAUDE_MD_MAX_BYTES = 100 000`, tenu par
`crates/mika-common/tests/claude_md_size.rs`. `crates/mika-agent/CLAUDE.md` pèse
**94 977 o** : marge de **5 023 o**. L'ajout du § 3 pèse ~200 o. Le pilote lance
tout de même `cargo test -p mika-common --test claude_md_size` (critère 7).

---

## § 2. Décisions

### D1 — `warnings` à la racine, forme `{ name, messages }`, non filtrée

La clé s'ajoute **à la racine**, à côté de `skipped`, et vaut
`[{ "name": <String>, "messages": [<String>, …] }]` :

- `name` est `w.skill_name` ;
- `messages` est la suite des `message` des diagnostics **dans l'ordre**, **sans
  `level`**. Pas de `level`, pour deux raisons mesurées : `apply_load_safety_check`
  ne met dans `diagnostics` que des `Warn | Fail` (§ 1.2), donc tout ce qui
  atterrit là est un avertissement **par construction** ; et c'est déjà ce que la
  TUI affiche au démarrage (`chat.rs` l. 605 ne rend que le message).
- **Non filtrée par `source` ni `always_on`**, comme `skipped` et **pour la même
  raison** : un avertissement de validation est une propriété du **registre**, pas
  d'une skill.

**Le commentaire pointe celui de `skipped` au lieu de le recopier.** Deux énoncés
jumeaux du même raisonnement dérivent, et celui qui est déjà écrit est le bon.

### D2 — Le fixture monte une skill avertie sur un répertoire temporaire

Dans le module `tests` de `server/mod.rs`, un helper `test_registry_avertie()` :

1. `tempfile::tempdir()` — dépendance déjà en place, déjà employée dans ce même
   module (l. 2323, 5443, 5523, 5685) : **aucune dépendance neuve** (critère 5).
2. un sous-répertoire `avertie/` portant un `skill.toml` **valide** :

   ```toml
   [skill]
   name = "avertie"
   description = "Skill chargee avec un avertissement de validation"
   always_on = false

   [triggers]
   keywords = ["registre"]

   [constraints]
   required_tools = ["outil_absent"]
   ```

3. `SkillRegistry::from_dir(<tmp>)` puis `.apply_load_safety_check()`.

Quatre points que le pilote ne doit pas avoir à redécouvrir :

- **`required_tools = ["outil_absent"]` est le producteur de l'avertissement.**
  `skills/index.rs` l. 946-970 : l'outil n'est ni dans un `tools.json` (il n'y en
  a pas), ni dans `BUILTIN_TOOL_NAMES`, ni préfixé par une dépendance déclarée
  (il n'y en a pas) — donc `SkillDiagnostic::warn`, et **aucun** `Fail`.
- **`keywords` ne porte pas le nom de la skill.** `skills/index.rs` l. 825-835
  en fait un `Fail`, qui enverrait la skill dans `skipped`. D'où `["registre"]`.
- **`always_on = false` n'est pas un détail de remplissage** : il rend le second
  volet du critère 1 non vacant. Sous `?always_on=true` la skill quitte `skills`,
  et `warnings` doit rester **entier** — l'assertion échoue si quelqu'un câble
  `warnings` sur l'itérateur filtré plutôt que sur le registre.
- **Le `TempDir` meurt à la sortie du helper, délibérément.** Après
  `apply_load_safety_check` le registre est en mémoire et la route ne relit
  jamais le disque ; le champ `path` servi pointe vers un chemin disparu et
  **aucune assertion ne le lit**.

**Pourquoi pas un constructeur de test.** Un `SkillRegistry::with_validation_warnings(…)`
serait plus court, mais toucherait `skills/mod.rs`, que le ticket déclare intact.
Le critère **tient** sans élargir : pas de halte, pas d'élargissement.

### D3 — Le fixture produit DEUX avertissements, et c'est ce qui rend l'ordre testable

**C'est le seul point où ce plan va plus loin que le plan 005**, qui ne comptait
qu'un diagnostic. Mesure faite à la source sur `main` : un répertoire sans
`tools.json` **ni** `system_prompt.md` déclenche aussi l'étape 8 de
`validate_skill` (`skills/index.rs` l. 1544-1550) :

```rust
// 8. Warnings for no-op or never-activates skills
if !has_tools && !has_snippet {
    diags.push(SkillDiagnostic::warn(
        "no-op skill: no tools.json and no system_prompt.md",
    ));
}
```

Le fixture de D2 produit donc exactement **deux** `Warn`, dans l'ordre où
`validate_skill` les empile :

1. étape 5b (l. 963) — `"[constraints] required_tools references 'outil_absent' which is not in this skill's tools.json — this is OK if it's an MCP tool or a dependency tool whose name does not follow the dep-prefix convention"`
2. étape 8 (l. 1548) — `"no-op skill: no tools.json and no system_prompt.md"`

et **aucun** `Fail` : la skill reste chargée, et part dans `validated_warnings`.

**Le choix est de les garder tous les deux.** Le critère 2 demande « les messages
**dans l'ordre** des diagnostics » : à un seul message, cette assertion ne teste
rien. Le second avertissement est gratuit — il naît de l'absence de deux fichiers
que le fixture n'a aucune raison d'écrire — et il rend l'ordre réellement
vérifié. Le pilote assertionne donc **deux** messages, dans cet ordre, et non une
inclusion.

Ce que cela coûte, dit franchement : si quelqu'un ajoute plus tard un diagnostic
qui tire sur ce fixture, le test rougit. C'est un test de comportement sur un
fixture nommé, pas un scanner de population : une rougeur y dit « un diagnostic
neuf tire sur une skill valide-mais-avertie », ce qui mérite d'être vu.

Le troisième avertissement de l'étape 8 — `"skill will never activate: not
always_on and no trigger keywords"` — **ne tire pas**, parce que
`keywords = ["registre"]` n'est pas vide. Ce qui confirme que le choix de D2
était déjà délibéré sur cet axe.

### D4 — Pas de section `## Fire-Disposition` (gate N/A)

Ce vol **ne livre aucun détecteur**, donc la section n'est pas requise, et ce
plan ne l'invente pas. Même disposition que D8 du plan 005, et que V3c-1 avant
lui (D7 de son plan) : ses tests sont des tests de **comportement** sur un
fixture nommé. Un test négatif n'est pas un scanner — il n'a ni population à
parcourir, ni allowlist à tenir, ni risque de passer vert en ne lisant rien : le
critère 1 rougit sur `main` précisément parce que la clé n'y existe pas.

---

## § 3. Ordre de travail

**Ce vol est une garde : la route sert `warnings`, avec le test du critère 1 et la
doc de `crates/mika-agent/CLAUDE.md`, dans un seul commit.** Chaque garde arrive
avec ses tests dans le même commit ; les tests ne se reportent jamais à un vol
suivant.

### C1 — unique commit : `feat(1727): la route des skills sert les avertissements (vol V3c-2a-i)`

1. **`crates/mika-agent/src/server/dashboard.rs`**, dans `handle_skills_list`
   **seulement** : le bloc `warnings` juste après celui de `skipped`, avec son
   commentaire renvoyant à celui de `skipped` (D1), puis la clé `"warnings"` dans
   le `serde_json::json!` de la racine.
2. **`crates/mika-agent/src/server/mod.rs`**, dans son module `tests`
   **seulement** :
   - `test_registry_avertie()` selon D2, avec un doc-comment qui dit pourquoi un
     répertoire réel est nécessaire (§ 1.3) et pourquoi le `TempDir` peut mourir ;
   - un quatrième agent `skills-avertie` dans le tableau de
     `test_state_skills_multi()` (l. 2707), le `; 3]` passant à `; 4]` ;
   - le test du critère 1 via `skills_get`, **en deux requêtes** —
     `?agent=skills-avertie` puis `?agent=skills-avertie&always_on=true` —
     assertionnant que `warnings` porte `avertie` avec ses deux messages dans
     l'ordre (D3), et que la seconde requête vide `skills` de cette skill en
     laissant `warnings` **entier** ;
   - le test du critère 2 : sur `skills-a`, `warnings` vaut `[]`, et `agent`,
     `skills` et `skipped` sont inchangés.
3. **`crates/mika-agent/CLAUDE.md`**, paragraphe « Skills listing (#606,
   mika#1727 V3c-1) » **seulement** : la racine devient
   `{ "agent", "skills", "skipped", "warnings" }`, et quelques mots disent la
   forme de `warnings` (`{ name, messages }`, messages dans l'ordre, non
   filtrée). Pas de réécriture du paragraphe.

**Vérification du commit** : `cargo build -p mika-cli --bin mika`, puis
`cargo test -p mika-agent`, `cargo test -p mika-common --test claude_md_size`, et
`cargo clippy --all-targets -p mika-agent`.

### Fin de pipeline

Avant d'ouvrir la PR, le pilote lance `bash scripts/verify-pipeline.sh origin/main`.
**S'il le trouve rouge, il s'arrête et le dit** — il ne contourne pas.

Le corps de PR porte `Refs mika#1727` et **aucun** `Closes`, `Fixes` ni
`Resolves`. Si le gabarit de reprise (`wip-rescue`) ouvre la PR, il écrit
`Closes #N` par défaut : celui qui le voit le corrige **avant toute revue**, puis
vérifie que `gh pr view <N> --json closingIssuesReferences` rend `[]`. La suite
se note `Tracked in: senara-solutions/mika#1727`.

Il porte aussi `Code review: delegated to mika-qa (mika#2694)`, et le pilote
n'invoque ni `ce-simplify-code` ni `ce-code-review`.

### Halte du pilote sur la borne

Après le commit, le pilote mesure `git diff --numstat origin/main...HEAD`, plan
exclu. **Si le total dépasse 170, il s'arrête** : pas de PR. Son dernier message
dit « HALTE borne : N lignes pour 170 », avec le compte par commit.

**Les commentaires comptent dans la borne, mais ils ne se rabotent pas pour la
tenir.** Un commentaire dit le pourquoi qu'on ne lit pas dans le code. Si le code
écrit dépasse la borne, le pilote s'arrête selon la halte ci-dessus ; il ne
retire pas après coup les commentaires qui portent un pourquoi.

### Gel des tickets (jusqu'au 01/12)

Ce vol n'ouvre **aucun** ticket, pas même de suivi ni de sous-ticket. Un manque,
ou un reste, va en commentaire sur mika#1727 ou dans la PR.

### Si le code s'écarte de ce plan

La QA juge contre ce plan, pas contre le corps du ticket. Si le pilote doit
s'écarter d'un point — un fichier, un critère, un nom —, **il amende ce plan sur
la branche dans un commit `docs(plans)` avant de pousser le code qui s'en
écarte**, en nommant le point et la raison. Un plan qui dit une chose pendant que
le code en fait une autre est bloqué par la QA, même quand le code est juste.

### Périmètre de fichiers

**Seuls trois fichiers changent** :

| fichier | portée |
|---|---|
| `crates/mika-agent/src/server/dashboard.rs` | `handle_skills_list` seulement |
| `crates/mika-agent/src/server/mod.rs` | module `tests` seulement |
| `crates/mika-agent/CLAUDE.md` | paragraphe « Skills listing (#606, mika#1727 V3c-1) » |

**Restent intacts** : tout `crates/mika-cli/`, `crates/mika-common/`,
`crates/mika-a2a/`, `crates/mika-gateway/`, `dashboard/`, `docs/egress/`, et le
reste de `crates/mika-agent/src/` — `server/state.rs` et `skills/` compris.
**Aucun fichier du plan de contrôle** : ni `.github/workflows/`, ni `.claude/`,
ni `skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni
`.mika/`, ni `docs/egress/`, ni aucun chemin de CODEOWNERS.

Hors de ce vol, et nommés pour qu'ils ne glissent pas dedans : le client de la
route, son miroir, ses refus, le témoin de requête du factice et l'entrée
d'egress (V3c-2a-ii et V3c-2a-iii) ; tout câblage dans la TUI, le retrait de
l'assemblage, `updated_skills` et `migrate_disabled_markers_async` (V3c-2b) ; le
test É1 (V3c-2c).

---

## Definition of Done

- `GET /api/v1/skills?agent=<a>` sert `warnings` à la racine, sous la forme
  `[{ name, messages }]`, non filtrée par `source` ni `always_on`.
- Le test du critère 1 existe, passe, et rougit sur `main`.
- Les quatre tests de V3c-1 passent sans modification.
- Seuls les trois fichiers du tableau ci-dessus changent.
- `cargo build -p mika-cli --bin mika`, `cargo test -p mika-agent`,
  `cargo test -p mika-common --test claude_md_size` et
  `cargo clippy --all-targets -p mika-agent` sont verts.
- `bash scripts/verify-pipeline.sh origin/main` est vert.
- Le diff reste sous 170 lignes ajoutées, plan exclu.
- La PR porte `Refs mika#1727` et aucun `Closes`, `Fixes` ni `Resolves`.

## Acceptance criteria

1. Sur un agent dont le registre porte une skill chargée avec au moins un diagnostic d'avertissement, `GET /api/v1/skills?agent=<a>` rend à la racine `warnings`, avec le nom de cette skill et le message de son diagnostic. La même requête avec `always_on=true` retire la skill de `skills` et laisse `warnings` entier. Ce test est le test négatif du vol : sur main, la clé n'existe pas.
2. `warnings` a la forme `[{ "name": <String>, "messages": [<String>, …] }]`, les messages dans l'ordre des diagnostics, sans `level`. Sur un agent sans avertissement, `warnings` vaut `[]`, et `agent`, `skills`, `skipped` sont inchangés.
3. Aucun test existant de `server/mod.rs` ne change. Les quatre tests de V3c-1 passent tels quels.
4. Seuls changent les trois fichiers listés plus haut. Tout `crates/mika-cli/` et tout `skills/` sont inchangés.
5. Aucune attente ni aucun `allow` neuf.
6. Le diff reste sous 170 lignes ajoutées, plan exclu.
7. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-agent` et `cargo test -p mika-common --test claude_md_size` sont verts. `cargo clippy --all-targets -p mika-agent` est vert. Le pilote lance `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il le trouve rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si le gabarit de reprise l'ouvre.

## Taille estimée

| livrable | lignes |
|---|---|
| `server/dashboard.rs` — bloc `warnings` + clé racine + commentaire | 15 |
| `server/mod.rs` — `test_registry_avertie()` et son doc-comment | 28 |
| `server/mod.rs` — quatrième agent dans `test_state_skills_multi()` | 3 |
| `server/mod.rs` — test du critère 1 (deux requêtes) | 40 |
| `server/mod.rs` — test du critère 2 (`warnings == []`) | 15 |
| `crates/mika-agent/CLAUDE.md` — paragraphe « Skills listing » | 1 |

Total estimé : 102 lignes
Estimation dans l'unité de l'architecte (hors `docs/` et hors `CLAUDE.md`) : 101 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Les deux bornes du ticket sont tenues : **102 pour 170** dans l'unité du vol,
**101 pour 155** dans celle de l'architecte. Le § 4 du plan 005 chiffrait déjà ce
vol (sa coupe B, vol B-i) à 93 / 85 ; l'écart vient du test du critère 2, compté
ici à part, et des commentaires de D2 et D3. Aucune halte de borne, donc, et
aucune coupe à demander.

## Revision history

- **2026-11-17** — rédaction initiale (groom autonome, mika#1727 V3c-2a-i).
  Reprise du plan 005 : mesure de `dashboard.rs` et de `skills/`, écart É-A,
  décisions D1, D2 et D8 (D8 renumérotée D4), commit C1. Une décision neuve,
  **D3** : mesure à la source du § 8 de `validate_skill`, qui ajoute un second
  avertissement au fixture de D2 — le plan 005 n'en comptait qu'un. Les deux
  sont gardés, parce que l'assertion d'ordre du critère 2 est vacante à un seul
  message.
