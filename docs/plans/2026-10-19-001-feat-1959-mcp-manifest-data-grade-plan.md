# Plan — feat(mcp): `data_grade` sur le manifeste MCP, et L4 qui mord sur les outils MCP (mika#1959)

**Status:** DRAFT
**Date:** 2026-10-19
**Ticket:** mika#1959
**Révision:** rev 2 — re-groom contre `main`. Remplace `docs/plans/2026-08-23-004-feat-1959-mcp-manifest-data-grade-plan.md` (rev 1, 2026-08-23), dont trois prémisses sont fausses aujourd'hui (§ *Ce que le re-groom a changé*).
**Classe:** suite de la doctrine non-transit — F5 de la revue adversariale de mika#1798
**Renvois:** mika#1798 (cuisson de la doctrine, PR#1956 **mergée** le 2026-08-23 — `1f322ecb`), mika#1957 (F3, fermé), mika#2627 (surface *propose*), `feedback_prompt_enforcement_fragile`

## Pourquoi

PR#1956 a livré la doctrine non-transit en quatre couches : L1 prompt, L2 bannissement au registre, L3 validation de sous-commande `run_gws`, L4 garde à l'exécution indexée par `skill_data_grades: HashMap<String, DataGrade>`. La revue adversariale a relevé **F5** : les outils MCP contournent L4 **par construction**. Ils atteignent `execute_tool` par le troisième étage du dispatch (après builtins, après skills), ils ne sont donc pas des `skill_tools`, et la map L4 est bâtie depuis `skill_tools` — donc vide pour eux. Un `mcp__gmail__list_messages` atteint Gmail sans qu'aucune couche ne tire.

Le dépôt dit déjà, en deux endroits indépendants, ce qu'il faut faire :

- `crates/mika-agent/src/tool_execution/dispatch.rs:65-73` — *« The map's shape is forward-compatible: when MCP manifests gain a `data_grade` field and MCP tools are seeded into this map, the L4 predicate will fire on them without any change to `execute_tool`. »*
- `crates/mika-agent/docs/non-transit-data-grade.md:370-375` — *« Forward-compat requires MCP manifests to gain a `data_grade` field AND the MCP dispatch site to consult the same map. »*

Ce ticket exécute cette prescription : **un champ dans le manifeste, un ensemencement de la map existante, zéro ligne changée dans `execute_tool`.**

**Priorité p2-normal, et la raison vaut d'être tenue :** aucun serveur MCP en production n'expose de données testimony aujourd'hui, donc il n'y a **aucun contournement actif**. La garde atterrit avant que la demande arrive. C'est une porte posée sur un couloir encore vide, pas une fuite qu'on colmate.

## Ce que le re-groom a changé

La rev 1 datait du 2026-08-23, avant le merge de PR#1956. Trois de ses prémisses sont fausses contre `main` aujourd'hui, et deux changent la conception.

| prémisse rev 1 | état réel sur `main` | conséquence |
|---|---|---|
| « PR#1956 est OPEN ; implémentation bloquée ; Path A/B à trancher » | **mergée** le 2026-08-23 (`1f322ecb`) | toute la section *Dependency on PR#1956* tombe ; l'AC5 (doc) n'est plus différée |
| « `DataGrade` a trois variantes : `reference`/`evidence`/`testimony` » | **deux** : `Operational` (`#[default]`) et `Testimony` (`skills/manifest.rs:336-347`) | **décision D1** ci-dessous |
| « ajouter une branche MCP parallèle dans `execute_tool` » (§3 rev 1) | la forme prescrite est d'**ensemencer la map existante** | **décision D2** — `execute_tool` reste intouché ; une garde de moins, pas une de plus |

La branche a par ailleurs été rebasée : elle est à 0 commit de retard sur `origin/main` (la porte de promotion de mika#2123 l'avait refusée à 92 de retard le 2026-09-03 ; le motif du refus a disparu).

## Décisions

### D1 — On réutilise l'enum livré à deux variantes ; on n'en crée pas trois

Le *Scope* du ticket écrit `data_grade: testimony | evidence | reference`, défaut `reference`. **Cet enum n'existe pas et n'a jamais été livré.** Celui que PR#1956 a mergé est :

```rust
// crates/mika-agent/src/skills/manifest.rs:336
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DataGrade {
    #[default]
    Operational,
    Testimony,
}
```

L'exigence de fond du ticket — *« Default missing field to `reference` (deny for testimony path) »* — est **exactement** satisfaite par `Operational` en `#[default]` : champ absent ⇒ variante sûre ⇒ L4 ne tire pas ⇒ le serveur fonctionne ; déclaration explicite `testimony` ⇒ L4 refuse. La propriété demandée est le *fail-closed sur le chemin testimony*, et elle est tenue.

Porter l'enum à trois variantes coûterait : `SkillInfo.data_grade` et tous les `skill.toml`, `apply_testimony_grade_ban` (qui compare à `Testimony`), le rédacteur de prompt `write_data_grade_doctrine_section` (×2 variantes de prompt), le prédicat L4, et les tests de chacun — pour une distinction `evidence` vs `reference` que **rien ne consomme** : aucune couche ne traite ces deux grades différemment. C'est du vocabulaire sans conséquence machine, et la doctrine est explicitement binaire (« Mika may NEVER access testimony-grade »).

**Décision : réutiliser `DataGrade` tel quel.** Le `reference` du ticket se lit `operational` dans le manifeste. Le plan le dit, la doc opérateur le dit, et le ticket sera corrigé en ce sens au moment de la PR. Une entrée MCP se déclare donc `"data_grade": "operational"` ou `"data_grade": "testimony"`, et l'absence vaut `operational`.

**Conséquence assumée :** un opérateur qui lit le *Scope* du ticket et écrit `"data_grade": "reference"` obtient une **erreur de parse** du fichier `mcp.json`, pas un défaut silencieux. C'est le bon sens du refus (un grade inconnu ne doit jamais être deviné), et c'est pourquoi un test le pin (AC1).

### D2 — On ensemence la map existante ; `execute_tool` n'est pas touché

La rev 1 ajoutait une seconde branche de refus dans `execute_tool`, dédiée au préfixe MCP. Deux raisons de ne pas le faire.

D'abord la conception livrée le prescrit (les deux citations du § *Pourquoi*). Ensuite l'orthogonalité : L4 est **un** prédicat sur **une** map. Deux branches, c'est deux prédicats à maintenir d'accord, et le jour où l'un gagne une condition que l'autre n'a pas, le trou est silencieux. La map est déjà `HashMap<String, DataGrade>` indexée par **nom d'outil Claude-facing** — et un outil MCP en a un, `mcp__<serveur>__<outil>`. Rien dans la clé n'est propre aux skills.

Donc : `execute_tool` reste **byte pour byte** ce qu'il est. Le champ `TOOL_TESTIMONY_GRADE_FORBIDDEN` et son message restent les mêmes. Le test de refus (AC3) passe par `execute_tool` sans qu'une ligne de `execute_tool` ait changé — c'est la démonstration que la forme était bien « forward-compatible », et pas une affirmation.

**Nuance de message, nommée pour que personne ne la « corrige » :** le texte du refus dit *« This tool's owning skill declares data_grade = "testimony" »*. Pour un outil MCP il n'y a pas de skill propriétaire — c'est le serveur. Le message est donc légèrement inexact sur ce chemin. Le plan **ne le change pas** : `TOOL_TESTIMONY_GRADE_FORBIDDEN` est une constante JSON unique dont la forme est le contrat, l'amender demanderait soit un second message (deux sites, le défaut de D2 sous un autre angle) soit un formatage dynamique (la constante cesse d'être une constante). Le champ `doctrine: "mika#1798"` et le refus structurel portent l'information utile. La doc opérateur dit que « owning skill » couvre aussi « serveur MCP déclarant ».

### D3 — Deux sites d'ensemencement sur trois éventails, et le troisième est hors population par construction

`build_skill_data_grades` est appelé en trois endroits de `agent_loop/mod.rs` (la discipline « 3 of 3 fan-out sites » de mika#2276). Ici **deux** reçoivent l'ensemencement, et il faut dire pourquoi, sinon un lecteur futur « réparera » le troisième :

| mode | map bâtie | outils MCP publiés | ensemencer ? |
|---|---|---|---|
| conversation | `mod.rs:5566` | `mod.rs:5606` (`extend_from_slice(mcp.tool_definitions())`) | **oui** |
| team | `mod.rs:7657` | `mod.rs:7670` (idem) | **oui** |
| silencieux / callback / heartbeat | `mod.rs:6861` | **aucun** — `run_loop` reçoit `None` en place du manager (`mod.rs:7182`, commentaire *« MCP tools excluded from silent mode »*) | **non** |

Le mode silencieux n'a pas de manager MCP du tout : il n'y a donc rien à ensemencer, et un ensemencement y serait du code mort appelé avec `None`. La garde structurelle de la § *Fire-Disposition* tient l'appariement sur la population réelle — les sites qui **publient** — plutôt que sur un compte figé de trois.

### D4 — La population est une fonction pure, parce que `connect_all` n'est pas testable

`McpManager::connect_all` ouvre de vraies connexions (stdio ou HTTP) : aucun test unitaire ne peut l'exécuter, et les tests existants construisent le manager par littéral de struct (`mcp/mod.rs:426`, `empty_manager()`). Mettre la logique de population en ligne dans `connect_all` la rendrait donc **non couverte** — l'AC2 serait vérifiée par un test qui remplit la map à la main, c'est-à-dire par rien.

Donc la jointure config × routage sort en fonction pure et libre :

```rust
pub(crate) fn mcp_tool_data_grades(
    config: &McpConfig,
    tool_routing: &HashMap<String, (String, String)>,
) -> HashMap<String, DataGrade>
```

`connect_all` l'appelle une fois, juste avant de construire `Self`. Les tests l'appellent directement avec un `McpConfig` désérialisé depuis du JSON littéral et un routage bâti à la main : c'est la vraie logique qui est sous assertion.

### D5 — La non-publication (la moitié *propose*) est HORS périmètre, et nommée

Côté skills, la doctrine a **deux** couches : L2 évince la skill du registre (ses outils ne sont donc **jamais publiés** au modèle) et L4 refuse à l'exécution. Côté MCP, ce plan ne livre que l'équivalent de L4.

**Résidu, dit franchement :** un outil MCP déclaré testimony **reste publié** dans `tool_definitions()`, donc le modèle le voit et peut le *proposer* à l'utilisateur. L4 refusera l'appel, mais la doctrine interdit aussi de « proposer » (c'est tout l'objet de mika#1960 phase 2 / mika#2627, garde EndTurn 5h). La symétrie complète serait de ne pas enregistrer du tout les outils d'un serveur testimony dans la boucle de `connect_all`.

Ce n'est **pas** dans le *Scope* du ticket, qui écrit « Gate L4 MCP calls by data_grade » et rien d'autre ; la fiche F5 de la doc doctrine ne demande pas davantage. Élargir ici mélangerait deux garanties dans une PR dont la borne par vol est déjà serrée. L'ensemencement livré est par ailleurs le **prérequis** de cette suite : on ne peut évincer qu'un grade qu'on sait lire.

**Tracked in: senara-solutions/mika#1798** — le ticket parent de la doctrine porte le résidu en attendant. Au moment de la PR : ouvrir le ticket de suite « ne pas publier les outils d'un serveur MCP testimony (équivalent L2 côté MCP) », et remplacer la ligne `Tracked in:` du corps de PR par son numéro. La doc doctrine porte ce résidu en § *Vigilance surface* plutôt que de laisser croire la classe entièrement fermée.

## Quoi

### 1. `McpServerConfig` gagne `data_grade`

**Fichier :** `crates/mika-agent/src/mcp/config.rs`.

```rust
    /// Whether this server is enabled (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Grade des données exposées par les outils de ce serveur (mika#1959,
    /// doctrine non-transit mika#1798).
    ///
    /// Absent ⇒ `DataGrade::Operational` par le `#[default]` de l'enum : c'est
    /// la porte porteuse. Un manifeste qui oublie de déclarer son grade ne peut
    /// pas acquérir par accident un privilège testimony ; il faut le consentement
    /// explicite `"data_grade": "testimony"` — qui vaut refus de L4, pas
    /// permission (voir la doc opérateur).
    ///
    /// Valeurs : `"operational"` (défaut) | `"testimony"`. Le `reference` du
    /// corps de mika#1959 se lit `operational` ici — l'enum livré par PR#1956
    /// est binaire (décision D1 du plan).
    ///
    /// Le grade vaut pour **tous** les outils du serveur. La granularité par
    /// outil est une affaire de v2 : aucun serveur v1 connu n'expose des outils
    /// de grades mêlés.
    #[serde(default)]
    pub data_grade: DataGrade,
```

Import : `use crate::skills::manifest::DataGrade;`. Même crate, aucune question de visibilité.

**`DataGrade` et pas `Option<DataGrade>`** — divergence assumée avec la rev 1, qui proposait l'`Option` pour « préserver le round-trip absent-vs-explicite ». Cette distinction ne sert aucun consommateur : les deux cas produisent `Operational`, et personne ne réécrit `mcp.json`. L'enum dérive `Default`, donc `#[serde(default)]` donne directement `Operational` sans `.unwrap_or()` au point de consommation. Un `Option` ajouterait un `None` à déplier à chaque lecture — un état de plus pour zéro comportement de plus.

Ajouter la ligne correspondante dans l'impl `Debug` manuelle (`config.rs:57-80`) : `s.field("data_grade", &self.data_grade);`. Aucun secret, aucune rédaction.

**Contrat JSON :**

```json
{
  "mcpServers": {
    "journal-perso": {
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@exemple/mcp-journal"],
      "data_grade": "testimony"
    },
    "wikipedia": {
      "transport": "http",
      "url": "https://mcp.wikipedia.org/",
      "data_grade": "operational"
    },
    "serveur-quelconque": {
      "transport": "stdio",
      "command": "some-mcp"
    }
  }
}
```

Le troisième n'a pas de champ : il vaut `operational`.

### 2. `McpManager` porte et expose les grades par outil

**Fichier :** `crates/mika-agent/src/mcp/mod.rs`.

```rust
pub struct McpManager {
    connections: HashMap<String, McpConnection>,
    tool_definitions: Vec<ToolDefinition>,
    /// Maps namespaced tool name -> (server_name, original_tool_name).
    tool_routing: HashMap<String, (String, String)>,
    /// Grade par nom d'outil namespacé, dérivé du `data_grade` de son serveur
    /// (mika#1959). Consommé par la garde L4 de `tool_execution::dispatch` via
    /// l'ensemencement de `skill_data_grades` dans `agent_loop` — pas par
    /// `call_tool`, qui ne refuse rien.
    tool_data_grades: HashMap<String, DataGrade>,
}
```

La fonction pure de D4, et son câblage :

```rust
/// Joint le `data_grade` de chaque serveur à chacun de ses outils namespacés.
///
/// Libre et pure pour être testable : `connect_all` ouvre de vraies connexions,
/// aucun test unitaire ne l'atteint (décision D4 du plan mika#1959). Un outil
/// dont le serveur a disparu de la config retombe sur le défaut de l'enum —
/// cas inatteignable depuis `connect_all` (le routage est bâti depuis cette
/// même config), couvert parce que la signature le permet.
pub(crate) fn mcp_tool_data_grades(
    config: &McpConfig,
    tool_routing: &HashMap<String, (String, String)>,
) -> HashMap<String, DataGrade> {
    tool_routing
        .iter()
        .map(|(namespaced, (server, _tool))| {
            let grade = config
                .mcp_servers
                .get(server)
                .map(|cfg| cfg.data_grade)
                .unwrap_or_default();
            (namespaced.clone(), grade)
        })
        .collect()
}
```

Dans `connect_all`, juste avant le `Self { … }` final (`mcp/mod.rs:139`) :

```rust
        let tool_data_grades = mcp_tool_data_grades(config, &tool_routing);

        Self {
            connections,
            tool_definitions,
            tool_routing,
            tool_data_grades,
        }
```

Accesseur, voisin de `tool_definitions()` :

```rust
    /// Grade par outil namespacé, pour l'ensemencement de la map L4.
    pub fn tool_data_grades(&self) -> &HashMap<String, DataGrade> {
        &self.tool_data_grades
    }
```

`empty_manager()` dans les tests (`mcp/mod.rs:426`) gagne `tool_data_grades: HashMap::new()`.

### 3. Les deux sites d'ensemencement

**Fichier :** `crates/mika-agent/src/agent_loop/mod.rs`.

Un helper, à côté de `build_skill_data_grades` (`mod.rs:7959`) :

```rust
/// Ensemence la map L4 avec les grades des outils MCP (mika#1959, F5 de mika#1798).
///
/// Les outils MCP ne sont pas des `skill_tools`, donc `build_skill_data_grades`
/// ne les voit pas et L4 ne tirait jamais sur eux. La map est indexée par nom
/// d'outil Claude-facing, ce qu'un outil MCP possède (`mcp__<serveur>__<outil>`),
/// donc l'ensemencement suffit : `execute_tool` n'est pas modifié.
///
/// À appeler partout où `mcp.tool_definitions()` est publié au modèle — et
/// seulement là. Le mode silencieux reçoit `None` en place du manager, il est
/// hors population par construction. Appariement tenu par la garde
/// `mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4`.
///
/// Collision : un outil MCP et un outil de skill de même nom sont impossibles —
/// le préfixe `mcp__` est réservé. L'insertion est donc sans écrasement en
/// pratique, et last-write-wins comme le reste de la map si elle le devenait.
fn seed_mcp_data_grades(
    grades: &mut HashMap<String, crate::skills::manifest::DataGrade>,
    mcp: Option<&McpManager>,
) {
    if let Some(mcp) = mcp {
        for (tool, grade) in mcp.tool_data_grades() {
            grades.insert(tool.clone(), *grade);
        }
    }
}
```

Les deux appels, chacun collé à son `extend_from_slice` pour que l'appariement soit visible à l'œil :

```rust
    // mika#1959 — les outils MCP publiés ci-dessous entrent dans la map L4.
    let mut skill_data_grades = build_skill_data_grades(&matched_entries);
    seed_mcp_data_grades(&mut skill_data_grades, params.mcp_manager);
```

en conversation (`mod.rs:5566`) et en team (`mod.rs:7657`). Le `let` devient `let mut` aux deux sites. Le site silencieux (`mod.rs:6861`) **n'est pas touché**.

### 4. Doc doctrine + doc opérateur

**`crates/mika-agent/docs/non-transit-data-grade.md`** (hors comptage de lignes) :

- § *Known bypass classes* — retirer l'entrée **MCP-registered testimony tools** (l. 370-375).
- § *Layer 4*, point 2 *Dynamic MCP registration* (l. 208-214) — réécrire : la forme n'est plus « forward-compatible », elle est **en vigueur**. Dire que l'ensemencement est le mécanisme et que `execute_tool` n'a pas changé.
- § *Ban list (v1)* — ajouter : « Tout outil MCP dont le serveur déclare `data_grade = "testimony"` (L4) ».
- § *Vigilance surface* — porter le résidu de D5 : le grade testimony **refuse l'appel** mais **ne dépublie pas** l'outil ; la surface *propose* reste tenue par la seule garde 5h. C'est ce qui empêche de lire F5 comme entièrement fermée.
- § *File anchors* — ajouter le champ de manifeste MCP et le site d'ensemencement.
- § *Change log* — entrée `2026-10-19 — mika#1959`, avec la correction `reference` → `operational` (D1).

**Doc opérateur** — section « Déclarer le grade d'un serveur MCP » :

> Chaque entrée de `~/.mika/mcp.json` accepte un champ `data_grade` :
>
> - `"operational"` — défaut quand le champ est absent. Le serveur fonctionne normalement.
> - `"testimony"` — données personnelles, confessionnelles, de confiance déléguée. **Déclaration explicite exigée.**
>
> Les outils d'un serveur déclaré `testimony` sont **refusés** par la garde L4 à l'exécution. Un opérateur qui veut réellement une surface testimony doit la déclarer *et* accepter que Mika refuse d'appeler ces outils : la valeur de la déclaration est la traçabilité, pas la permission.
>
> Un grade inconnu — dont le `"reference"` du corps de mika#1959, qui n'existe pas dans l'enum livré — fait **échouer le chargement** de `mcp.json`. Un grade n'est jamais deviné.
>
> **Ce que la déclaration ne fait pas :** l'outil reste visible du modèle, qui peut le mentionner. Seul l'appel est refusé.

## Fire-Disposition

Ce plan livre un détecteur : la garde structurelle d'appariement
`mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4`, dont le chemin
de succès est « aucune violation trouvée ». Elle existe parce que la doc doctrine
nomme déjà cette classe de panne pour le bannissement des skills (§ *Known bypass
classes*, « Registry-ban callsite drift » — ~12 sites appariés à la main) : ajouter
un site qui publie des outils MCP sans l'ensemencement rouvrirait F5 **en
silence**, toutes les assertions comportementales restant vertes.

**Disposition retenue : (a) exception nommée en allowlist — et l'allowlist part
VIDE.** Il n'y a aucune violation préexistante à exempter : les deux seuls sites
de publication (`mod.rs:5606`, `mod.rs:7670`) sont appariés par cette PR même.
Le détecteur atterrit donc **armé**, sans exception, et son vide est lui-même
asserté — idiome du dépôt (mika#2425 : *« allowlist shipped empty, pinned empty
by a sibling test »*). Ni (b) ni (c) : il n'y a rien à désarmer et rien à
remonter.

**Implémentation.** Test `#[test]` dans le `mod tests` en ligne de
`agent_loop/mod.rs`, sur le texte source de `src/agent_loop/**/*.rs` :

1. Lire chaque `.rs` du répertoire, couper à la première occurrence de
   `#[cfg(test)]`, puis retirer les commentaires `//` ligne par ligne — l'idiome
   `production_sans_commentaires()` de `milestone_manager/mod.rs:314`. Les deux
   étapes comptent : la prose *sur* l'ensemencement n'est pas un ensemencement
   (motif mika#2050 signal S), et ce plan en écrit beaucoup.
2. Compter les occurrences de `extend_from_slice(mcp.tool_definitions())` — les
   sites qui publient — et celles de `seed_mcp_data_grades(`, moins sa
   définition.
3. Assertion : les deux comptes sont **égaux**, et valent 2. Message d'échec
   actionnable nommant le site publiant non apparié et renvoyant à la décision
   D3.
4. `ALLOWLIST_PUBLICATION_SANS_ENSEMENCEMENT: &[&str] = &[]`, avec une assertion
   sœur `assert!(ALLOWLIST…is_empty())` portant *pourquoi* elle est vide : toute
   entrée future doit nommer le site, son ticket de suivi, et rougir quand elle
   devient stale.

**Contrôle négatif de bonne foi** —
`mika1959_la_garde_dappariement_rougit_sur_un_site_non_ensemence` : appliquer le
même prédicat à un fragment source littéral portant un `extend_from_slice(mcp.tool_definitions())`
sans ensemencement, et vérifier qu'il est bien compté comme violation. Sans lui,
une garde qui ne trouve jamais rien et une garde cassée rendent les mêmes octets.

**Note de prémisse (mika#2335).** La garde tronque à `#[cfg(test)]`, ce qui est
correct ici : `agent_loop/` n'a que des `mod tests` **en ligne**, pas de fichier
de test extrait, donc `is_test_source_path` n'est pas requis. Si un futur
refactor extrait ces tests dans `agent_loop/tests/`, consulter
`crate::source_scan::is_test_source_path` (`source_scan.rs:79`) — **réparer la
classification, jamais élargir l'aiguille ni ajouter une allowlist de fichiers.**

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/mcp/config.rs` — champ `data_grade`, doc, ligne `Debug` | 22 |
| `crates/mika-agent/src/mcp/mod.rs` — champ, `mcp_tool_data_grades`, accesseur, câblage | 40 |
| `crates/mika-agent/src/agent_loop/mod.rs` — `seed_mcp_data_grades` + 2 sites | 28 |
| tests `mcp/config.rs` (3 tests de parse) | 32 |
| tests `mcp/mod.rs` (2 tests de population) | 42 |
| tests `tool_execution/dispatch.rs` (refus L4 + contrôle négatif) | 42 |
| garde d'appariement + contrôle négatif (`agent_loop/mod.rs`) | 58 |

Total estimé : 264 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 264 < 279 (borne par vol) et
264 < 1000 (seuil par PR) : les deux bornes passent, la plus serrée mordant à
279. Trois choix de conception tiennent le total sous la borne, et aucun n'est un
rognage de la garantie :

- **D2** supprime la plus grosse ligne de la rev 1 — une branche de refus dédiée
  dans `execute_tool` et ses tests. `execute_tool` n'est pas modifié.
- **D1** évite la migration d'enum à trois variantes, qui aurait touché les
  `skill.toml`, `apply_testimony_grade_ban`, deux rédacteurs de prompt et leurs
  tests : plusieurs centaines de lignes, à elle seule au-dessus de la borne.
- **D5** sort la non-publication du périmètre et la renvoie à un ticket de suite
  nommé, plutôt que de la glisser dans ce vol.

La marge est de 15 lignes, soit ~6 % — mince. Le poste le plus susceptible de
déborder est la garde structurelle (58). S'il déborde à l'implémentation, le
rognage se prend sur le **volume des tests de parse** (poste à 32, dont deux cas
sur trois sont du paramétrage de boucle), **jamais** sur le contrôle négatif de
la garde ni sur le test de refus L4 : ce sont les deux seules assertions qui
distinguent « la garantie tient » de « rien ne la contredit ».

## Acceptance criteria

Le corps du ticket n'a pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de la prose du *Scope* (« Add `data_grade` to MCP manifest
schema. Gate L4 MCP calls by data_grade. Default missing field to `reference`. »),
lue à la lumière de la décision D1.

- **AC1 — le manifeste porte le champ, et refuse un grade inconnu.**
  `McpServerConfig` a `data_grade: DataGrade` avec `#[serde(default)]`. Un
  `mcp.json` sans le champ désérialise en `DataGrade::Operational`.
  `"testimony"` et `"operational"` désérialisent en leurs variantes. Une valeur
  hors enum — dont `"reference"` — fait **échouer** la désérialisation.
- **AC2 — le manager expose un grade par outil namespacé.**
  `McpManager::tool_data_grades()` rend un `&HashMap<String, DataGrade>` où
  chaque outil de `tool_routing` a une entrée, portant le grade de son serveur ;
  un serveur sans champ donne `Operational`. La logique est sous assertion via
  la fonction pure `mcp_tool_data_grades` (D4), pas via un remplissage manuel de
  la map.
- **AC3 — L4 refuse un appel d'outil MCP testimony, avant le réseau.**
  `execute_tool` sur un outil MCP dont le serveur déclare `testimony` rend
  `TOOL_TESTIMONY_GRADE_FORBIDDEN` sans atteindre `McpManager::call_tool` : aucun
  appel distant, aucun effet de bord. Contrôle négatif : le même outil en
  `operational` **n'est pas** refusé par L4 (il poursuit le dispatch).
  **`execute_tool` n'est pas modifié par cette PR** — `git diff` sur sa fonction
  est vide, ce qui est la preuve que la forme était bien extensible.
- **AC4 — l'appariement publication/ensemencement est tenu structurellement.**
  La garde de la § *Fire-Disposition* passe, son allowlist est vide et assertée
  vide, et son contrôle négatif rougit sur un site publiant non ensemencé.
- **AC5 — la doctrine et l'opérateur sont à jour.** F5 quitte § *Known bypass
  classes* ; § *Layer 4* point 2 passe de « forward-compatible » à « en
  vigueur » ; § *Vigilance surface* porte le résidu *propose* de D5 ; § *Ban
  list* et § *File anchors* sont complétés ; le change log porte l'entrée du jour
  et la correction `reference` → `operational`. La doc opérateur décrit le champ,
  son défaut, le refus sur grade inconnu, et ce que la déclaration ne fait pas.

## Definition of Done

- [ ] `crates/mika-agent/src/mcp/config.rs` : champ `data_grade: DataGrade`, doc par § 1, ligne ajoutée dans l'impl `Debug` manuelle.
- [ ] `crates/mika-agent/src/mcp/mod.rs` : champ `tool_data_grades`, fonction pure `mcp_tool_data_grades`, accesseur `tool_data_grades()`, câblage dans `connect_all`, `empty_manager()` mis à jour.
- [ ] `crates/mika-agent/src/agent_loop/mod.rs` : `seed_mcp_data_grades` + appel en conversation (`:5566`) et en team (`:7657`). Site silencieux (`:6861`) intouché.
- [ ] `crates/mika-agent/src/tool_execution/dispatch.rs` : **aucune modification de `execute_tool`** ; le commentaire « aspirational, not enforced today » (l. 65-73) est réécrit au présent et renvoie au site d'ensemencement.
- [ ] Tests `mcp/config.rs` : défaut sur champ absent, parse des deux variantes, refus d'un grade inconnu (`"reference"` comme cas nommé).
- [ ] Tests `mcp/mod.rs` : population par `mcp_tool_data_grades` avec grade déclaré ; défaut `Operational` sur serveur sans champ.
- [ ] Tests `tool_execution/dispatch.rs` : refus L4 d'un outil MCP testimony sans atteindre `call_tool` ; contrôle négatif `operational` non refusé.
- [ ] Garde `mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4` + contrôle négatif `mika1959_la_garde_dappariement_rougit_sur_un_site_non_ensemence` + allowlist vide assertée vide.
- [ ] `crates/mika-agent/docs/non-transit-data-grade.md` mis à jour par § 4 (cinq sections + change log).
- [ ] Doc opérateur : section « Déclarer le grade d'un serveur MCP ».
- [ ] Ticket de suite D5 ouvert (non-publication des outils MCP testimony) et nommé en `Tracked in:` dans le corps de PR.
- [ ] `cargo test --workspace` vert.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` vert.
- [ ] `cargo fmt --all --check` vert.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] Corps de PR : (a) « forward-compat, aucun contournement actif » explicite, (b) la correction D1 `reference` → `operational` et le fait que le corps du ticket est corrigé en conséquence, (c) le résidu D5 avec son `Tracked in:`, (d) exemples de manifeste pour l'opérateur.

## Vérification par injection (per `feedback_verify_pipeline_passes_without_the_fix`)

Quatre inversions, une par AC exécutable. Chacune doit faire **rougir le test nommé**, sinon le test ne prouve rien.

1. **AC1** — remplacer `#[serde(default)]` par un `#[serde(default = "…")]` rendant `Testimony` ; le test de défaut sur champ absent rougit. Restaurer.
2. **AC2** — dans `mcp_tool_data_grades`, remplacer le `.map(|cfg| cfg.data_grade)` par `.map(|_| DataGrade::Operational)` ; le test de grade déclaré rougit (le défaut, lui, reste vert — c'est ce qui montre que les deux tests ne sont pas redondants). Restaurer.
3. **AC3** — retirer l'appel `seed_mcp_data_grades` du site conversation ; le test de refus L4 rougit et l'appel atteint `call_tool`. Restaurer.
4. **AC4** — ajouter un troisième `extend_from_slice(mcp.tool_definitions())` sans ensemencement ; la garde d'appariement rougit en nommant le site. Restaurer.

À consigner dans `todos/1959-injection-verification.md`.

## Hors périmètre

- **Non-publication des outils d'un serveur testimony** — décision D5, renvoyée à un ticket de suite nommé en `Tracked in:`. Résidu porté par § *Vigilance surface* de la doc doctrine.
- **Extension de `DataGrade` à trois variantes** — décision D1. Demanderait une migration des `skill.toml`, de `apply_testimony_grade_ban` et de deux rédacteurs de prompt, pour une distinction que rien ne consomme.
- **Granularité par outil dans un serveur** — v1 est par serveur. Un serveur aux grades mêlés est une affaire de v2 (`data_grade_overrides: HashMap<String, DataGrade>`).
- **Dérogation à l'exécution** — pas de `MIKA_MCP_ALLOW_TESTIMONY=1`. La doctrine n'a pas de surface de dérogation en v1, et ce ticket n'en ouvre pas une.
- **Classification automatique** — aucune inférence du grade depuis le nom du serveur ou la description des outils. Déclaration explicite de l'opérateur, uniquement.
- **Avertissement de migration** — aucun WARN au démarrage sur champ absent : ce serait du bruit sur la population modale (tous les serveurs actuels sont operational). La doc porte le défaut.
- **Mode silencieux** — hors population par construction (D3), pas par omission.
- **Surfaces HTTP brutes** — `curl` vers une API Gmail reste une classe ouverte de la doc doctrine, indépendante de MCP.

## Risques et parades

- **Le message de refus dit « owning skill » pour un serveur MCP.** Inexactitude assumée et argumentée en D2 : la constante JSON est le contrat, et l'amender coûterait soit un second message soit la fin de la constante. La doc opérateur lève l'ambiguïté. Si cela gêne un relecteur, la bonne réponse est un ticket sur le *format* du refus, pas une branche de plus.
- **L'opérateur écrit `"reference"` en suivant le corps du ticket.** Le chargement de `mcp.json` échoue — bruyamment, ce qui est le comportement voulu (un grade n'est pas deviné). Parades : le test AC1 nomme `"reference"` comme cas de refus, la doc opérateur le nomme, et le corps du ticket est corrigé au moment de la PR.
- **Dérive d'appariement : un futur site publie des outils MCP sans ensemencer.** C'est exactement la classe « Registry-ban callsite drift » de la doc doctrine. Parade : la garde structurelle de la § *Fire-Disposition*, armée, allowlist vide.
- **Un serveur réellement testimony déclaré `operational` (ou laissé vide).** Il tourne, ses outils marchent, L4 ne tire pas. **C'est le trou que ce ticket ne ferme pas** : il déplace la garantie du code vers la déclaration de l'opérateur, et L2 (bannissement par nom de skill) ne couvre pas les noms de serveurs MCP. Risque résiduel réel, à porter en § *Vigilance surface*, avec l'appel explicite à déclarer le grade sur tout serveur adjacent à Gmail / Drive / données personnelles. Aucune parade technique en v1 — c'est ce que « déclaration explicite » signifie.
- **`DataGrade` partagé entre skills et MCP.** Voulu : un `testimony` dans `mcp.json` veut dire la même chose que dans un `skill.toml`, et l'uniformité de L4 dépend de ce type unique. C'est l'argument de D2 vu depuis le type.

## Solutions apparentées

- `crates/mika-agent/docs/non-transit-data-grade.md` — F5 y est en § *Known bypass classes* (l. 370-375) ; ce ticket l'en retire et porte son résidu en § *Vigilance surface*.
- `feedback_prompt_enforcement_fragile` — la mémoire fondatrice. Un serveur MCP est exactement la classe de « nouvelle surface d'intégration » qu'une consigne de prompt ne peut pas couvrir : la déclaration structurelle est la porte porteuse.
- mika#1957 (F3, fermé) — même forme « porte structurelle à la frontière d'exécution », autre classe d'outil (`shell-exec`).
- mika#2627 — la moitié *propose* de la doctrine, qui est la raison d'être du résidu D5.
- mika#2425 — l'idiome de la garde structurelle à allowlist vide, et sa discipline de contrôle négatif.

## Potentiel de composition

- **Le champ de schéma à consentement explicite** (~40 lignes) : un champ serde dont l'absence vaut le défaut sûr, et dont la déclaration explicite n'accorde pas une permission mais rend un refus traçable. Réutilisable pour toute surface de configuration dont le défaut doit échouer fermé. L'anti-motif à contraster est le champ par défaut permissif, où l'absence vaut le privilège maximal.
- **« Étendre une garde en ensemençant sa map, pas en ajoutant sa branche »** : la démonstration la plus propre de ce motif que le dépôt puisse produire, puisque le `git diff` de `execute_tool` est **vide** alors que sa couverture a crû d'un étage de dispatch entier. Un doc de composition nommant ce motif — map indexée par la clé neutre plutôt que par le type propriétaire — le rend reproductible pour les prochains étages (plugins d'agent, hooks de workflow, scripts externes).
