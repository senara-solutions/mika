# mika#2814 vol 2 — publier des outils MCP et ensemencer L4 deviennent un seul appel

> - **Ticket:** senara-solutions/mika#2814 (vol 2)
> - **Branch:** `feat/2814/non-publication-des-outils-mcp-testimony`
> - **Base:** `main` (`cf21685b`, qui contient `5570ac1a`)
> - **Plan du vol 1 (livré):** `docs/plans/2026-10-19-006-feat-2814-non-publication-outils-mcp-testimony-plan.md`

## Contexte — ce que le vol 1 a fermé, et ce qu'il a laissé

Le vol 1 (PR #2832, mergée en `5570ac1a`) a livré l'équivalent L2 côté MCP :
`mcp::retain_publiables` filtre les outils testimony dans
`McpManager::connect_all`, de sorte que le manager ne porte jamais ces
définitions. Un outil MCP dont le serveur déclare `data_grade = "testimony"`
n'est donc plus publié au modèle, qui ne peut plus le *proposer* — la moitié de
la doctrine mika#1798 (« nor propose accessing ») que mika#1959 avait laissée
ouverte en D5.

Le ticket reste ouvert pour les **deux résidus de la revue de #2816**, que #2832
a explicitement refusé de clore (« Pas de `Closes` ») :

1. **Les deux sites d'ensemencement à déplacer**, avec leur propre rouge mesuré.
   Le choix du remède a été fait au vol 1 (D7) et tracé dans le doc-comment de
   `seed_mcp_data_grades` : un helper unique qui publie *et* ensemence.
2. **Un avertissement au chargement sur toute clé inconnue** d'une entrée de
   serveur `mcp.json` — `"dataGrade": "testimony"` vaut aujourd'hui
   `operational` sans rien dire. Un avertissement, **pas un refus**.

Le vol 1 avait aussi nommé un troisième item pour la suite : l'incohérence de
disponibilité de `filter_available_required_tools`, actuellement inatteignable.

**Ce vol prend le résidu 1 seul.** Les résidus 2 et 3 sont renvoyés par une
ligne `Tracked in:` — voir § *Taille estimée* pour la mécanique de la coupe.

## Le fait qui ouvre ce vol : la prémisse de D7 point 2 est fausse

Le vol 1 a justifié le report du résidu 1 par deux raisons. La première est la
borne par vol — elle tient. La seconde affirme que D1 **abaisse l'urgence** :

> après ce vol c'est « publie un lot propre et n'ensemence pas, donc un appel
> halluciné rend *outil inconnu* au lieu de *banni* » — une dégradation
> d'observabilité, plus une porte ouverte.

C'est inexact, et son propre D3 en porte la raison : **`tool_routing` est resté
intact.** La cascade de `tool_execution::dispatch::execute_tool` se lit dans cet
ordre (`dispatch.rs:535-632`) :

1. L4 — `dispatch.skill_data_grades.get(name)`. Sans entrée dans la map, **ne
   tire pas**.
2. builtins, puis skills — un nom `mcp__*` n'y est pas.
3. MCP — `mcp.is_mcp_tool(name)` lit `tool_routing` (`mcp/mod.rs:287-289`), que
   le filtrage de publication ne touche pas. Il répond **vrai**, et
   `mcp.call_tool(name, input)` part.

Donc un site de publication non ensemencé ne dégrade pas l'observabilité : il
**laisse l'appel distant partir** sur un outil testimony, pour tout nom obtenu
autrement que par l'inventaire publié — hallucination, mention en prose par
l'utilisateur, historique d'avant la connexion du serveur, résumé de
conversation. C'est le contournement F5 entier, pas sa trace.

Le vol 1 a eu raison sur la conclusion (le filtrage à `connect_all` est le bon
geste, et il est livré) et tort sur ce corollaire. La même erreur de modèle
figure dans le doc-comment du test de garde (`agent_loop/mod.rs:20080`, « tombe
sur `TOOL_TESTIMONY_GRADE_FORBIDDEN` et non sur « outil inconnu » ») et dans
celui de `seed_mcp_data_grades`. Ce vol la corrige là où elle est écrite, et la
mesure plutôt que de la réaffirmer (§ *Rouge mesuré*).

Conséquence sur le périmètre : la garde textuelle de mika#1959 est aujourd'hui
**la seule chose** qui tient cette porte. Elle compare des chaînes. C'est
exactement la raison de faire ce vol maintenant.

## Décisions

### D1 — Le helper vit dans `agent_loop/mod.rs`, avec la signature déjà tracée

`seed_mcp_data_grades` ne bouge pas de fichier et le helper se pose à côté
d'elle, avec la signature que le doc-comment de `seed_mcp_data_grades` a rendue
publique au vol 1 :

```rust
fn publier_outils_mcp(
    defs: &mut Vec<ToolDefinition>,
    grades: &mut HashMap<String, DataGrade>,
    mcp: Option<&McpManager>,
)
```

Un plan antérieur a nommé cette signature dans un commentaire que la revue de
#2816 lira ; la changer sans raison coûterait la traçabilité du choix. Le corps
fait les deux gestes de la paire actuelle, dans le même ordre, et `None` (mode
silencieux) reste hors population par construction — il ne publie ni n'ensemence
rien.

`seed_mcp_data_grades` **n'est plus appelée que par le helper** et devient son
auxiliaire privé. Elle n'est pas inlinée : son test
(`mika1959_lensemencement_verse_les_grades_mcp_sans_ecraser_les_skills`) prouve
la règle de collision « le grade le plus strict l'emporte », qui est une
propriété de l'ensemencement et non de la publication. Les fondre rendrait ce
test plus gros pour prouver la même chose.

### D2 — Les deux sites appellent le helper et rien d'autre

Aux lignes `5775-5780` (conversation) et `7878-7883` (agent d'équipe),
l'actuel :

```rust
if let Some(mcp) = params.mcp_manager {
    skill_tool_defs.extend_from_slice(mcp.tool_definitions());
}
seed_mcp_data_grades(&mut skill_data_grades, params.mcp_manager);
```

devient un appel unique. Le `if let` disparaît : le helper prend l'`Option`,
donc le site n'a plus de branche à écrire — et une branche en moins est une
branche de moins où oublier la seconde moitié.

### D3 — La garde garde ses deux moitiés, parce que compter les appels ne suffit pas

Le doc-comment du vol 1 annonce que « la garde se réduit à compter les appels
d'une fonction, ce qu'un renommage casse bruyamment au lieu de silencieusement ».
C'est vrai du renommage et faux de l'ajout : un troisième site qui publie en
écrivant `defs.extend_from_slice(mcp.tool_definitions())` laisse le compte
d'appels du helper à `2` et la garde **verte**. Le helper rend la faute
inécrivable *dans le chemin qui passe par lui*, pas en dehors.

La garde refondue porte donc deux assertions :

1. **Le compte d'appels** — `publier_outils_mcp(` apparaît exactement deux fois
   hors de sa définition, dans les fichiers de production.
2. **L'absence de contournement** — aucune ligne de production ne lit
   `mcp.tool_definitions()` en dehors du corps du helper, sauf les **lecteurs
   non-publiants** déclarés, qui ont le droit de connaître l'inventaire sans le
   publier.

La moitié 2 est ce que le vol 1 ne pouvait pas écrire : avant le helper, il
n'existait aucun site unique où la lecture était légitime, donc l'aiguille
devait viser la forme `extend_from_slice(…)` — la fragilité même qu'on corrige.
Après le helper, l'aiguille vise `tool_definitions()` **nue**, et la liste des
lecteurs autorisés est finie, courte et nommée. L'aiguille cesse d'être une
reconnaissance de forme et devient un inventaire.

Le prédicat réutilise les trois mécaniques de l'existant sans les réinventer :
`ProductionScanner::for_crate`, l'exclusion par
`crate::source_scan::is_test_source_path` (mika#2321), le masquage des régions
de test par `mask_test_regions` (mika#2398), et le filtre de prose
(`!line.starts_with("//") && !line.starts_with('*')`) — porteur ici, puisque
`tool_definitions()` figure en clair dans une dizaine de doc-comments et dans
les fixtures du contrôle négatif de ce même fichier.

### D4 — Les deux lecteurs non-publiants sont nommés, et leur allowlist s'auto-nettoie

Inventaire mesuré (`grep -rn "tool_definitions()" --include="*.rs" crates/`,
hors définitions, hors tests, hors prose) : exactement deux lecteurs de
production ne publient pas.

| lecteur | ce qu'il fait |
|---|---|
| `agent_loop/mod.rs:1554` | collecte les **noms** d'outils, ne les publie pas au modèle |
| `kg/domain_builder.rs:768` | projette les outils dans le graphe de connaissance |

`LECTEURS_NON_PUBLIANTS_MCP` les porte, une entrée par site, chacune nommant
son fichier **et** la raison. Contrairement à
`ALLOWLIST_PUBLICATION_SANS_ENSEMENCEMENT` qu'elle remplace, cette liste naît
**non vide** — c'est un inventaire de droits légitimes, pas un emplacement pour
le prochain oubli — et ce changement de nature est la raison de la renommer
plutôt que de recycler l'ancienne.

L'assertion auto-nettoyante va donc dans l'autre sens que celle du vol 1 : au
lieu d'asservir la vacuité, elle exige que **chaque entrée corresponde à un
usage réellement trouvé par le scan**. Une entrée dont le site a disparu rougit,
avec le message qui dit de la retirer. C'est la propriété qui empêche la liste
de devenir l'allowlist que mika#2309 condamne : le geste qui rend une exception
inutile est celui qui la fait expirer.

### D5 — Le rouge se mesure dans `todos/`, pas en test permanent

Le ticket exige « leur propre rouge mesuré ». La doctrine
(`feedback_verify_pipeline_passes_without_the_fix`) demande de prouver que le
pipeline passe **sans** le correctif ; le vol 1 l'a consignée dans
`todos/2814-injection-verification.md` (4 cas). Ce vol écrit son frère.

Un test permanent est **écarté** pour la porte de D5 : épingler « sans
ensemencement, l'appel part » serait épingler un bug dont le helper supprime la
population. Après ce vol il n'existe plus de chemin qui publie sans ensemencer,
donc le test n'aurait rien à mesurer et se dégraderait en commentaire exécutable.
Les trois mesures sont historiques et vivent en prose :

- **Cas 1 — la porte est réelle.** Sur `main`, un `McpManager` monté par
  `sans_connexion_pour_tests` avec `tool_routing` portant
  `mcp__journal-perso__read_entry`, `skill_data_grades` **vide** :
  `execute_tool` rend `"MCP server 'journal-perso' is not connected."`, pas
  `"Unknown tool: …"`. L'étage 3 a été atteint et `call_tool` est entré. Avec le
  grade ensemencé, il rend `TOOL_TESTIMONY_GRADE_FORBIDDEN`. Deux populations,
  deux octets différents (classe mika#2205) — et la mesure réfute la prose du
  vol 1 citée plus haut.
- **Cas 2 — la garde d'appariement actuelle devient rouge sous D2.** Les deux
  aiguilles disparaissent des sites, `pubs.len()` tombe à `0` et
  `assert_eq!(publications.len(), 2)` échoue. C'est la preuve que la refonte
  n'est pas optionnelle : la règle du ticket (« un filtrage qui déplace la
  publication doit la mettre à jour dans le même commit, pas la contourner »)
  est tenue par la mécanique, pas par la discipline.
- **Cas 3 — la garde refondue mord sur ce que l'ancienne laissait passer.** Le
  troisième site écrit sans `extend_from_slice` : vert avant, rouge après.

### D6 — Le vol ne touche ni `retain_publiables`, ni `tool_routing`, ni L4

Le filtrage de publication livré par le vol 1 reste exactement où il est. Ce vol
ne déplace aucun filtre : il soude la paire publie/ensemence. `tool_routing`
reste intact (le refus L4 doit rendre *banni* et non *inconnu* — c'est la raison
de D3 du vol 1 et elle est inchangée), et le corps d'`execute_tool` n'est pas
modifié.

### D7 — La doctrine se met à jour dans le même commit

`crates/mika-agent/docs/non-transit-data-grade.md` § *Vigilance surface*
(l.383-393) annonce le remède comme « chosen » et non livré. Ce vol réécrit ce
paragraphe : la moitié « helper » devient livrée, la moitié « misspelled key »
reste ouverte avec son `Tracked in:`. Le fichier est **crate-local seul**
(absent du `DOCS=()` de `scripts/sync-agent-docs.sh`), donc aucun doc-sync n'est
dû et le job CI `docs-sync` n'est pas concerné.

## Quoi

### 1. `publier_outils_mcp` — `crates/mika-agent/src/agent_loop/mod.rs`

Fonction libre posée juste avant `seed_mcp_data_grades`, signature de D1. Corps :
publie `mcp.tool_definitions()` dans `defs`, puis délègue l'ensemencement à
`seed_mcp_data_grades`. Doc-comment : pourquoi un seul appel (l'appariement est
une propriété de l'appel), ce que la garde de D3 ajoute par-dessus, et la porte
mesurée de D5 — avec la correction explicite de la prose du vol 1, puisqu'un
lecteur qui croit « outil inconnu » sous-estime ce que le helper protège.

### 2. Les deux sites — `agent_loop/mod.rs:5775-5780` et `:7878-7883`

Remplacés par l'appel unique (D2). Le commentaire `// mika#1959 — les outils MCP
publiés juste au-dessus entrent dans la map L4.` disparaît : il décrivait un
appariement que le lecteur devait vérifier à l'œil, et il n'y a plus deux
appels à apparier.

### 3. `seed_mcp_data_grades` — doc-comment réécrit

Le bloc « # Le remède à la faiblesse de cette garde est **choisi** (mika#2814,
vol 2) » et son « Non implémenté dans ce commit » sortent : ils décrivent un
futur devenu présent. Reste : ce que la fonction fait, la règle de collision, et
une ligne disant qu'elle n'est appelée que par le helper et pourquoi elle n'y
est pas fondue (D1).

### 4. La garde refondue — région `agent_loop/mod.rs:19983-20149`

- `ALLOWLIST_PUBLICATION_SANS_ENSEMENCEMENT` (vide) est retirée ;
  `LECTEURS_NON_PUBLIANTS_MCP` la remplace avec ses deux entrées de D4.
- `sites_publication_et_ensemencement_mcp` devient
  `usages_des_definitions_mcp(path, src)` : rend les lignes de production qui
  lisent `tool_definitions()` et celles qui appellent `publier_outils_mcp(`
  (hors définition), mêmes exclusions qu'aujourd'hui.
- `mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4` devient
  `mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence`, portant
  les deux assertions de D3 plus l'assertion auto-nettoyante de D4. Le nom
  change parce que la propriété change : on n'apparie plus deux textes, on
  prouve qu'un seul chemin existe.

### 5. Le contrôle négatif — région `agent_loop/mod.rs:20222-20295`

Réécrit sur la nouvelle surface, trois cas (l'actuel en a cinq pour deux
aiguilles ; une aiguille en demande moins) :

| cas | attendu |
|---|---|
| un site publiant via `extend_from_slice(mcp.tool_definitions())` hors helper | **rouge** — c'est le contournement que l'ancienne garde laissait passer |
| une lecture nue (`for def in mcp.tool_definitions()`) dans un fichier non déclaré | **rouge** |
| la définition du helper elle-même, et les deux lecteurs déclarés | **vert** — ni la déclaration ni un droit nommé ne sont une faute |

### 6. Tests du helper

Deux cas, dans le module de tests qui porte déjà
`mika1959_lensemencement_verse_les_grades_mcp_sans_ecraser_les_skills` (lequel
reste, inchangé dans son corps, requalifié dans son doc-comment) :

- un appel publie **et** ensemence : `defs` contient les définitions du manager
  et `grades` porte leurs grades, après un seul appel ;
- `None` laisse les deux arguments intacts — le mode silencieux reste hors
  population par construction, et la propriété est asservie plutôt que supposée.

### 7. `todos/2814-vol2-rouge-mesure.md`

Les trois cas de D5, chacun avec sa commande, sa sortie observée et sa date.
Prose, pas de code.

### 8. Doctrine — `crates/mika-agent/docs/non-transit-data-grade.md`

Le paragraphe de D7.

## Ordre de commit

Un seul commit. L'ordre interne est contraint par le Cas 2 de D5 : D2 rend la
garde actuelle rouge, donc les livrables 2, 4 et 5 ne peuvent pas être séparés
sans laisser l'arbre rouge entre deux commits. L'ordre de **rédaction** est
néanmoins 7 (mesurer avant de corriger), puis 1, 2, 3, 4, 5, 6, 8.

## Fire-Disposition

Ce vol livre un détecteur (la garde refondue du livrable 4) et retire celui de
mika#1959. Option **(a) — exception nommée en allowlist**, avec un sens inversé
par rapport à l'allowlist qu'elle remplace :

- `LECTEURS_NON_PUBLIANTS_MCP` porte **deux entrées**, grep-visibles, nommant
  chacune son fichier et la raison de son droit : `agent_loop/mod.rs:1554`
  (collecte de noms) et `kg/domain_builder.rs:768` (projection KG). Ce ne sont
  pas des violations tolérées mais des lecteurs légitimes de l'inventaire, et
  aucun ticket de suivi ne leur est dû : rien n'est à faire à leur sujet. Le
  commentaire de la const le dit, pour qu'un lecteur ne cherche pas le `mika#`
  manquant.
- **L'assertion auto-nettoyante** rougit quand une entrée devient stale, c'est-à-
  dire quand le scan ne trouve plus d'usage de `tool_definitions()` dans le
  fichier qu'elle nomme. Une exception qui a cessé de correspondre à quoi que ce
  soit est la façon dont une liste de droits devient une liste de passe-droits ;
  elle expire par le geste même qui la rend inutile.
- Le détecteur atterrit **armé** : l'inventaire de D4 est mesuré et complet, donc
  il est vert à l'atterrissage sans aucune exemption à poser. Aucune violation
  existante n'est à exempter — c'est précisément ce que la mesure a vérifié.
- Le contrôle négatif du livrable 5 prouve que le détecteur **mord**, y compris
  sur le cas que son prédécesseur laissait passer (Cas 2 de D5).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `publier_outils_mcp` + doc-comment (`agent_loop/mod.rs`) | 55 |
| les deux sites de publication (`:5775`, `:7878`) | 10 |
| `seed_mcp_data_grades` — privatisation + doc-comment réécrit | 25 |
| garde refondue (const, scanner, test, assertions) | 90 |
| contrôle négatif de la garde (3 cas) | 45 |
| tests du helper (publie+ensemence, `None` inerte) | 40 |

Total estimé : 265 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Deux conventions de comptage, déclarées parce qu'elles décident de la coupe.**
`todos/2814-vol2-rouge-mesure.md` (≈ 50 lignes) et le paragraphe de doctrine ne
sont pas comptés : ce sont de la prose, et la règle compte les lignes de code. Le
vol 1 a traité `todos/` à l'identique (196 estimées hors `docs/`, pour 227 lignes
de `todos/` livrées en plus). Les **doc-comments des fichiers `.rs`**, eux, sont
comptés — c'est le choix inverse du vol 1 et la raison principale du présent
calibrage.

**Calibrage mesuré du vol 1** (`git show --numstat 5570ac1a`, hors `docs/` et
hors `todos/`) : 196 lignes estimées pour ≈ 514 réelles, soit un facteur ≈ 2,6×.
L'écart n'est pas une dérive de périmètre — c'est le poids des doc-comments
idiomatiques du dépôt, absents de l'estimation (`mcp/mod.rs` seul : +219 lignes
pour une fonction de 15). Les 265 ci-dessus les incluent livrable par livrable,
ce qui rend ce total comparable au **réel** du vol 1 et non à son estimation.

**Pourquoi un seul vol tient.** 265 < 279, avec une marge mince mais un
périmètre qui ne s'étend pas : un fichier de production touché
(`agent_loop/mod.rs`), aucune API publique changée, aucun appelant hors crate,
et une suppression nette (la région `19983-20149` et le contrôle négatif
`20222-20295`, soit ≈ 313 lignes retirées) qui ne coûte pas de tours de
conception. Les deux chantiers qui feraient déborder sont renvoyés :

- L'**audit de clés inconnues de `mcp.json`** (résidu 2) est indépendant — il
  touche `mcp/config.rs` seul, qui n'est pas dans ce vol. Le plan du vol 1 l'a
  estimé à 136 lignes ; recalibré, il est au-delà de 200, et sa somme avec ce
  vol dépasse la borne. Son périmètre est déjà cadré : `KNOWN_SERVER_KEYS` (les
  8 champs sérialisés de `McpServerConfig`), épinglage par round-trip de
  sérialisation (viable parce qu'aucun champ ne porte `skip_serializing_if`),
  `did_you_mean` par normalisation lowercase + retrait des `_` (ce qui attrape
  `dataGrade` → `data_grade` sans dépendance Levenshtein, dont le dépôt n'a
  aucune), aux deux `serde_json::from_str` de `load` (`:146`) et
  `load_operator_shell` (`:207`). **WARN jamais refus** : `mcp.json` est un
  schéma partagé avec d'autres hôtes MCP et son chargement est délibérément
  fail-open (mika#1764) — ce qui écarte aussi `#[serde(deny_unknown_fields)]`,
  et `#[serde(flatten)]` pour son coût mesuré de 23 sites de construction
  littérale.
- L'**incohérence de `filter_available_required_tools`** (item 3 du vol 1)
  reste inatteignable : aucune skill bundlée ne déclare de `mcp__*` dans ses
  `required_tools`. Elle part avec le résidu 2, dont elle partage la nature
  (épingler un fait, pas corriger un chemin vivant).

Tracked in: senara-solutions/mika#2814 (vol 3 — avertissement sur clé inconnue
de `mcp.json`, et épinglage de `filter_available_required_tools`)

## Verification contract

| ce qui est prouvé | par quoi |
|---|---|
| un seul chemin publie les outils MCP, et il ensemence | `mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence`, assertion 1 (compte d'appels `== 2`) |
| aucun site ne contourne le helper | même test, assertion 2 (aucun `tool_definitions()` hors helper et hors lecteurs déclarés) |
| les deux lecteurs déclarés existent encore | même test, assertion auto-nettoyante de D4 |
| le détecteur mord sur le cas que son prédécesseur manquait | contrôle négatif, cas 1 |
| le helper fait les deux gestes | test du livrable 6, cas 1 |
| le mode silencieux reste hors population | test du livrable 6, cas 2 |
| l'ensemencement ne rabat jamais un `Testimony` de skill | `mika1959_lensemencement_verse_les_grades_mcp_sans_ecraser_les_skills` (existant, passe inchangé) |
| les outils testimony restent non publiés | `tests/eval/test_mcp_testimony_non_publication_2814.rs` (existant, passe inchangé) |
| la porte était réelle avant ce vol | `todos/2814-vol2-rouge-mesure.md`, cas 1 |
| la refonte n'était pas contournable | `todos/2814-vol2-rouge-mesure.md`, cas 2 |

## Definition of Done

1. `publier_outils_mcp` existe avec la signature de D1 ; les deux sites de
   publication l'appellent et ne contiennent plus ni `extend_from_slice(mcp.tool_definitions())`
   ni d'appel direct à `seed_mcp_data_grades`.
2. La garde de mika#1959 est **remplacée** (pas contournée, pas allowlistée) par
   celle de D3, dans le même commit que le déplacement — et le Cas 2 de D5
   mesure qu'elle serait rouge sinon.
3. `LECTEURS_NON_PUBLIANTS_MCP` porte exactement les deux entrées de D4, chacune
   nommant son fichier et la raison de son droit, et l'assertion
   auto-nettoyante passe.
4. Le contrôle négatif à trois cas du livrable 5 passe, cas rouges inclus.
5. `todos/2814-vol2-rouge-mesure.md` porte les trois cas de D5 avec leurs
   sorties observées et leurs dates.
6. Les doc-comments de `seed_mcp_data_grades` et du test de garde ne
   contiennent plus l'affirmation « rend *outil inconnu* » réfutée en tête de
   plan.
7. `crates/mika-agent/docs/non-transit-data-grade.md` § *Vigilance surface* dit
   la moitié « helper » livrée et la moitié « misspelled key » ouverte, avec son
   `Tracked in:`.
8. `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` et
   `scripts/verify-pipeline.sh origin/main` passent.
9. Le corps de PR porte **Pas de `Closes`** : le résidu 2 et l'item 3 restent
   ouverts sur mika#2814, nommés par la ligne `Tracked in:` de § *Taille
   estimée*.

## Acceptance criteria

Dérivés du corps du ticket et des sections ci-dessus — le ticket ne porte pas de
section `## Acceptance criteria`.

1. **AC1 — Les deux sites d'ensemencement sont déplacés dans le helper.** Un
   seul appel, aux deux sites, publie et ensemence ; il n'existe plus dans le
   code de production d'appel qui ne fasse qu'une des deux moitiés.
2. **AC2 — Le déplacement porte son propre rouge mesuré.** Les trois cas de D5
   sont consignés avec la sortie observée, pas affirmés. Le Cas 1 réfute
   explicitement la prose du vol 1 sur « outil inconnu ».
3. **AC3 — La garde d'appariement est mise à jour dans le même commit, pas
   contournée.** Aucun `#[ignore]`, aucune entrée d'allowlist posée pour faire
   passer la garde existante, aucune suppression de garde sans remplaçante.
4. **AC4 — La garde refondue mord là où la textuelle ne mordait pas.** Un
   troisième site qui publie sans écrire `extend_from_slice` est rouge ; le
   contrôle négatif le prouve.
5. **AC5 — Les lecteurs non-publiants gardent leur droit de lire l'inventaire.**
   `agent_loop/mod.rs:1554` et `kg/domain_builder.rs:768` ne sont pas modifiés
   et ne rendent pas la garde rouge.
6. **AC6 — Aucune régression sur le vol 1.** Les tests de non-publication de
   `tests/eval/test_mcp_testimony_non_publication_2814.rs` et le test de
   collision de grades passent sans modification de leur corps.
7. **AC7 — Le résidu 2 reste explicitement ouvert.** Le plan et le corps de PR
   le nomment avec son périmètre cadré ; rien ne laisse croire que mika#2814
   est clos.
