# mika#2814 vol 4 — la raison de désarmement nomme le ban testimony, pas `not_served`

> - **Ticket:** senara-solutions/mika#2814 (vol 4, dernier)
> - **Branch:** `feat/2814/non-publication-des-outils-mcp-testimony`
> - **Base:** `main` (`dc96ad5e`, qui contient `5a93d507`)
> - **Plan du vol 1 (livré, PR #2832 → `5570ac1a`):** `docs/plans/2026-10-19-006-feat-2814-non-publication-outils-mcp-testimony-plan.md`
> - **Plan du vol 2 (livré, PR #2845 → `91a96cd2`):** `docs/plans/2026-10-20-001-feat-2814-helper-publie-et-ensemence-mcp-plan.md`
> - **Plan du vol 3 (livré, PR #2852 → `5a93d507`):** `docs/plans/2026-10-20-002-feat-2814-cle-inconnue-mcp-json-plan.md`

## Contexte — ce qui reste après trois vols

Le vol 1 a livré l'équivalent L2 côté MCP (`mcp::retain_publiables`). Le vol 2 a
soudé publication et ensemencement L4 en un seul appel
(`agent_loop::publier_outils_mcp`) et remplacé la garde textuelle de mika#1959
par une garde d'inventaire. Le vol 3 a fermé la clé inconnue de `mcp.json`.

Il reste **un** résidu, nommé par le vol 3 (« Tracked in ») et par la dernière
ligne `Tracked in:` que porte encore la doctrine
(`crates/mika-agent/docs/non-transit-data-grade.md:442`) : l'épinglage de
`filter_available_required_tools`.

**Ce vol le prend, et il est le dernier.** Les deux résidus de #2832, les deux
points de la revue de #2816 et la décision D5 de mika#1959 sont alors tous
fermés — le corps de PR porte donc `Closes #2814`, à la différence des trois
précédents.

## Le fait, mesuré sur le code actuel

`filter_available_required_tools` (`agent_loop/mod.rs:9054-9110`) décide quelles
exigences `required_tools` le tour peut encore honorer, et nomme chaque retrait.
Trois branches, dans cet ordre :

| ordre | prédicat | journal |
|---|---|---|
| 1 | `!available` (registre, skills, MCP) | `WARN`, message libre, pas de champ `reason` |
| 2 | `!served.contains(t)` | `INFO event = "required_tool_disarmed", reason = "not_served"` |
| 3 | `turn_forbids_dispatch && is_dispatch_tool(t)` | `INFO … reason = "turn_forbids_dispatch"` |

Appliqué à un outil MCP dont le serveur déclare `testimony` :

- **La branche 1 le déclare disponible.** Elle demande
  `mcp_manager.is_some_and(|m| m.is_mcp_tool(t))`, qui lit `tool_routing`
  (`mcp/mod.rs:287-289`) — intact par construction du vol 1
  (`retain_publiables` n'ampute que `tool_definitions`, précisément pour qu'un
  appel halluciné soit refusé comme `TOOL_TESTIMONY_GRADE_FORBIDDEN` et non
  comme « outil inconnu »).
- **C'est la branche 2 qui le désarme**, parce que `served` est
  `enabled_tool_names`, bâti sur `skill_tool_defs` (`agent_loop/mod.rs:5793`),
  d'où `publier_outils_mcp` ne l'a jamais versé.
- **Donc l'outil est bien désarmé, mais sous `reason = "not_served"`** — « ce
  tour-ci ne sert pas cet outil » — alors que la cause est « son serveur déclare
  testimony, et la doctrine interdit de le publier ». Deux populations sous les
  mêmes octets, classe mika#2205.

Le doc-comment de la fonction écrit lui-même pourquoi c'est un défaut : *« les
branches 1 et 2 ne sont pas redondantes, et confondre les deux ferait lire une
restriction normale comme une erreur de configuration »*. Un ban doctrinal lu
comme une restriction de tour est la même faute, d'un cran plus loin.

**Inatteignable aujourd'hui, et c'est mesuré** :
`grep -rn 'required_tools' skills/bundled/ --include=*.toml` rend onze
déclarations, aucune ne porte de `mcp__*`. La population est vide — ce qui fait
de ce vol un épinglage, pas une correction d'incident.

## Décisions

### D1 — La décision est extraite en fonction pure, parce que c'est la seule façon de la tester

**Le problème de testabilité est le cœur de ce vol, pas un détail
d'implémentation.** `filter_available_required_tools` rend un
`HashSet<String>` : la branche 2 et la branche testimony **retirent le même
outil**, donc rendent le **même ensemble**. Aucune assertion sur la valeur
rendue ne peut distinguer « désarmé comme non servi » de « désarmé comme banni »
— c'est-à-dire qu'aucune assertion sur la valeur rendue ne peut prouver l'AC de
ce ticket.

Et le dépôt **n'a pas de harnais de capture `tracing`** : la seule note du sujet
(`memory/mod.rs:982-993`) omet explicitement une assertion sur un `warn!` pour
cette raison, et aucun `tracing-test` ni `TestWriter` n'existe dans l'arbre.

Donc :

```rust
enum RaisonDesarmement { Inexistant, BanTestimony, NonServi, TourInterditDispatch }

fn raison_de_desarmement(
    outil: &str,
    tools: &ToolRegistry,
    skill_tool_map: &HashMap<String, &ResolvedSkillTool>,
    mcp_manager: Option<&McpManager>,
    served: &HashSet<String>,
    turn_forbids_dispatch: bool,
) -> Option<RaisonDesarmement>
```

Pure, module-privée, sans journalisation. `filter_available_required_tools`
l'appelle et `match` sur le résultat pour journaliser puis filtrer. **Même forme
et même raison que le livrable 6 du vol 3** : les tests interrogent la fonction
qui *décide*, et la fonction qui *journalise* en est le seul consommateur. Une
assertion sur des lignes de log capturées épinglerait la formulation ; une
assertion sur la variante épingle la propriété.

`turn_forbids_dispatch` entre en `bool` déjà calculé, et non en `&str` :
`is_unauthorized_webhook_dispatch` est évalué **une fois pour l'ensemble des
exigences** (mika#2757 — « le domaine est une propriété du tour, pas de
l'outil »), et faire entrer `turn_message` dans la fonction pure inviterait à le
réévaluer par outil.

### D2 — Pas de `as_str()` sur l'enum : les littéraux restent dans leurs bras

L'enum ne porte **aucune** méthode de conversion. Les littéraux
`reason = "not_served"`, `reason = "turn_forbids_dispatch"` et le nouveau
`reason = "mcp_testimony_banned"` restent écrits dans leurs bras respectifs du
`match` de journalisation, et les tests interrogent **les variantes**, jamais les
chaînes.

C'est ce qui évite le défaut que le vol 2 a payé ailleurs : deux spellings du
même fait. Avec un `as_str()`, la chaîne existerait deux fois (dans la méthode et
dans l'attente du test) ou le test épinglerait la méthode au lieu de la décision.
Ici la chaîne existe une fois, à son seul site d'émission, et la variante est le
contrat.

**Le `WARN` de la branche 1 est préservé littéralement**, message et champs
compris — il ne porte aujourd'hui ni `event` ni `reason`, et son doc-comment
dit que c'est voulu (*« celui-là accuse une configuration, pas un tour »*). Le
`match` le réécrit tel quel ; aucune harmonisation n'est faite au passage, parce
qu'un changement de forme de journal non demandé casse les `GROUP BY` d'un
opérateur sans rien acheter.

### D3 — La branche se pose entre 1 et 2, et cet ordre est la seule chose qui la rend atteignable

Ordre final : **1** l'outil n'existe pas → **1.5** son serveur déclare
`testimony` → **2** le tour ne le sert pas → **3** le tour interdit le dispatch.

Ce n'est pas une préférence de lisibilité. Un outil MCP testimony **n'est jamais
servi** — `retain_publiables` l'a retiré du lot, donc de `enabled_tool_names` —
si bien qu'une branche testimony posée **après** la 2 serait inatteignable à
100 %, pour toujours, avec tous les tests verts. Le test 1 du livrable 5 est
exactement l'épinglage de cet ordre : *testimony **et** non servi* rend
`BanTestimony`.

Elle se pose **après** la 1 parce qu'un outil qui n'existe pas n'a pas de grade,
et parce que la branche 1 accuse une configuration de skill cassée — un diagnostic
plus élémentaire, qui doit rester premier.

### D4 — Le prédicat de grade est un accesseur nommé sur le manager, pas une lecture de map au site

`McpManager::is_testimony_tool(&self, name: &str) -> bool`, à côté de
`is_mcp_tool`. La map est déjà accessible (`tool_data_grades()`, `mcp/mod.rs:257`),
donc la fonction pure pourrait écrire
`m.tool_data_grades().get(outil) == Some(&DataGrade::Testimony)`.

Elle ne le fait pas, pour deux raisons. La première : **aucune signature ne
change**, puisque `filter_available_required_tools` reçoit déjà
`mcp_manager: Option<&McpManager>` — donc les ~15 sites d'appel du module de
tests ne bougent pas, et l'accesseur ne coûte que lui-même. La seconde : le
prédicat « cet outil est-il banni ? » devient une question qu'on pose au
manager, dans le même vocabulaire que `retain_publiables` (`mcp/mod.rs:126`) ;
une comparaison de `DataGrade` recopiée dans `agent_loop` serait un second
endroit à mettre à jour le jour où un troisième grade apparaît.

**Un outil absent de la map n'est pas banni** — le défaut est de publier, pas de
censurer, exactement comme `retain_publiables` l'écrit. Donc `None` → `false`.

### D5 — `WARN`, et c'est l'inverse de l'argument de mika#2757

Les branches 2 et 3 sont des `INFO` **parce que leur population est nominale** :
le doc-comment dit qu'elles tireront « des dizaines de fois par jour pour un
comportement correct », et qu'un `WARN` à ce volume est un `WARN` qu'on apprend à
ignorer (leçon des « 25+ documented misfires » de mika#1469).

La branche testimony a la propriété **inverse, et elle est mesurée** : sa
population est **vide** aujourd'hui (zéro `mcp__*` en `required_tools`, § *Le
fait*). Elle ne peut donc pas noyer quoi que ce soit, et son apparition est un
fait qu'un opérateur doit voir — deux déclarations légitimes se contredisent : une
skill exige un outil, et l'opérateur a classé son serveur testimony. Personne n'a
tort ; la skill ne s'activera jamais utilement sur cet hôte, et c'est ce que le
`WARN` dit.

`event = "required_tool_disarmed"` est **conservé** (la famille reste
soustractible par `GROUP BY reason`) avec `reason = "mcp_testimony_banned"` et
les champs `tool`, `server` — ce dernier dérivé du **nom** par
`mcp::serveur_du_nom_namespace`, jamais d'un second parcours de la config, pour
la raison déjà écrite dans son doc-comment. La fonction est aujourd'hui privée au
module `mcp` : ce vol la passe en `pub(crate)` (un mot, pas un déplacement).

### D6 — Pas de contrôle positif runtime, et le refus est argumenté

La classe mika#2340 demanderait une ligne sur la branche saine, sans quoi « aucun
désarmement testimony » et « le binaire servi précède ce vol » rendraient les
mêmes octets. Elle n'est pas livrée, pour une raison de nature, pas d'économie :

- `filter_available_required_tools` tourne **à chaque tour**, et une ligne par
  tour disant « rien à bannir » est précisément le bruit que D5 refuse par
  ailleurs.
- Les précédents mika#2340 cités par le dépôt observent un fait **variable** —
  un hash de bibliothèque de skills, un budget KG en vigueur — où seule une
  attestation runtime répond. Ici le fait est **le code**, et la question « cette
  branche est-elle dans le binaire ? » se répond par la version, pas par un
  journal.
- Ce qui tient l'atteignabilité est le test 1 du livrable 5, pas une ligne de
  log : sans lui, l'ordre des branches pourrait régresser en silence — avec lui,
  il rougit.

Nommé pour que ce ne soit pas une omission.

### D7 — La garde proposée par le vol 3 est refusée, parce que la branche livrée la rend fausse

Le vol 3 nommait *« le geste qui rendrait le report révocable : une garde refusant
qu'une skill bundlée déclare un `mcp__*` en `required_tools` »*. Ce vol ne la
livre pas, et ce n'est pas un report :

- Sa fonction était de **protéger le report** — transformer « inatteignable
  aujourd'hui » en « inatteignable, et on le saura ». Le report est levé ; il n'y
  a plus rien à protéger.
- Et elle serait **fausse**. Elle interdirait tout `mcp__*`, y compris les
  `operational`, qui sont parfaitement légitimes en `required_tools`. Une garde
  qui refuse un cas correct pour attraper un cas désormais nommé est une garde
  qui sera désarmée par le premier développeur qu'elle gêne.

Ce que la branche achète est mieux que l'interdiction : le cas devient
**possible et nommé** plutôt qu'impossible.

### D8 — Rien des trois vols précédents n'est touché

`retain_publiables`, `tool_routing`, `tool_data_grades`, `mcp_tool_data_grades`,
`publier_outils_mcp`, `seed_mcp_data_grades`, la map L4, la garde
`mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence`,
`KNOWN_SERVER_KEYS` et `McpConfig::depuis_json` restent exactement où ils sont.
Ce vol **lit** la map du vol 1 ; il n'en écrit aucune.

Un seul mot change dans un livrable antérieur : la visibilité de
`serveur_du_nom_namespace` (D5).

### D9 — La doctrine se met à jour dans le même commit

`crates/mika-agent/docs/non-transit-data-grade.md` :

1. § *Vigilance surface*, paragraphe « **What this still does NOT buy, and where
   it is tracked** » (`:437-442`) — sa dernière clause et la ligne
   `Tracked in: senara-solutions/mika#2814 (vol 4).` sont réécrites en **livré** :
   la raison de désarmement nomme le ban, et l'ordre des branches est ce qui la
   rend atteignable. Le paragraphe **garde** sa première clause (la faute au
   niveau supérieur de `mcp.json`, hors périmètre depuis le vol 3 D3) : c'est un
   non-périmètre déclaré, pas un report de ce vol, et il ne porte aucune ligne
   `Tracked in:` à faire disparaître.
2. § *Where the pieces live* (`:560-566`) — une ligne qui décrit faux depuis le
   vol 2 : elle dit que la map est *« consumed by `seed_mcp_data_grades` at the
   two sites that publish `mcp.tool_definitions()` »*, alors que les deux sites
   appellent désormais `publier_outils_mcp`, qui appelle `seed_mcp_data_grades`.
   Corrigée (une ligne), et l'entrée nouvelle de la branche de désarmement
   ajoutée à côté.
3. § *Change log* — une entrée pour ce vol, qui clôt mika#2814.

Le fichier est **crate-local seul** — absent du `DOCS=()` de
`scripts/sync-agent-docs.sh` — donc aucun doc-sync n'est dû et le job CI
`docs-sync` n'est pas concerné.

## Quoi

### 1. `McpManager::is_testimony_tool` — `crates/mika-agent/src/mcp/mod.rs`

L'accesseur de D4, à côté de `is_mcp_tool`. Doc-comment : ce que le défaut
« absent de la map ⇒ publiable » garantit, et pourquoi le prédicat vit sur le
manager plutôt qu'au site de lecture.

Même fichier : `serveur_du_nom_namespace` passe `pub(crate)` (D5).

### 2. `RaisonDesarmement` — `crates/mika-agent/src/agent_loop/mod.rs`

L'enum de D1/D2, quatre variantes, aucune méthode. Doc-comment : pourquoi il
n'existe pas de `as_str()`, et pourquoi la variante est le contrat que les tests
lisent.

### 3. `raison_de_desarmement` — même fichier

La fonction pure de D1. Son doc-comment porte **l'ordre et sa justification**
(D3) : qu'une branche testimony posée après `NonServi` est inatteignable à 100 %
avec tous les tests verts, et que le test 1 du livrable 5 est ce qui l'empêche.

### 4. `filter_available_required_tools` — corps recâblé

Le corps devient : `turn_forbids_dispatch` calculé une fois, puis
`.filter(|t| match raison_de_desarmement(…) { … })` avec quatre bras
journalisants. Le `WARN` de la branche 1 et les deux `INFO` existants sont
recopiés **littéralement** (D2). Le doc-comment de la fonction gagne la
quatrième raison dans sa liste numérotée et une phrase sur l'ordre ;
la signature ne change pas.

### 5. Les tests de `raison_de_desarmement` — module `tests` de `agent_loop/mod.rs`

Sept cas, sur le harnais existant (`make_registry_with_tool`,
`make_resolved_skill_tool`, `served`, `McpManager::sans_connexion_pour_tests`) :

| cas | attendu |
|---|---|
| outil MCP testimony, **non servi** | `Some(BanTestimony)` — l'épinglage de l'ordre (D3), et le cœur de l'AC |
| outil MCP testimony, **servi** | `Some(BanTestimony)` — la branche ne dépend pas de `served` ; état inatteignable en production, atteignable par la signature |
| outil MCP **operational**, servi | `None` — contrôle négatif : le ban ne mord pas sur un grade operational |
| outil MCP **operational**, non servi | `Some(NonServi)` — la raison de mika#2757 est intacte sur la population qui lui revient |
| outil inexistant (manager `None`) | `Some(Inexistant)` |
| outil de dispatch servi, message webhook fallthrough | `Some(TourInterditDispatch)` — mika#2757 préservé |
| outil builtin servi, message neutre | `None` |

### 6. Le test de bout en bout — même module

Un cas par `filter_available_required_tools` : l'outil MCP testimony requis est
absent de l'ensemble rendu, et un builtin requis y reste. Il ne prouve pas la
raison (il ne peut pas, D1) ; il prouve que le recâblage du livrable 4 n'a pas
changé le comportement observable, ce que les sept cas du livrable 5 ne voient
pas.

### 7. Doctrine — `crates/mika-agent/docs/non-transit-data-grade.md`

Les trois points de D9.

## Ordre de commit

Un seul commit. Ordre de rédaction : 1, 2, 3, **5** (la fonction pure et ses
tests avant son consommateur — c'est l'ordre qui fait constater le rouge du
cas 1 avant que le câblage l'efface), puis 4, 6, 7.

**Le rouge à constater, nommément.** Avant le livrable 3, écrire le cas 1 contre
l'ordre *actuel* des branches — testimony après `NonServi` — et le voir rendre
`NonServi` au lieu de `BanTestimony`. C'est la mesure qui prouve que l'ordre de
D3 est la propriété, et non une préférence d'écriture.

## Ce que ce vol ne prend pas

**Une faute au niveau supérieur de `mcp.json`** (`"mcpServer"` au singulier),
hors périmètre depuis le vol 3 D3 et pour la raison qu'il a écrite : elle rend
une config vide, donc zéro outil MCP publié — visible à l'usage dès le premier
tour, là où `dataGrade` était muet par construction. Ce n'est **pas un report de
ce vol** : rien dans mika#2814 ne l'a jamais porté, la doctrine le déclare hors
périmètre sans ligne `Tracked in:`, et ce vol ne change pas ce statut.

**La garde refusant un `mcp__*` en `required_tools`** — refusée, pas reportée
(D7).

## Fire-Disposition

**Il n'y a pas de détecteur au sens strict dans ce vol** — aucun scan de source,
aucune règle de lint, aucune garde CI dont le chemin de succès est « aucune
violation trouvée ». La section est écrite quand même, et pour une raison
précise : **le cas 1 du livrable 5 est un détecteur de régression d'ordre**, et
c'est le seul livrable dont la fonction est de rougir plutôt que de mesurer un
comportement.

**Cas 1 du livrable 5 — option (a), exception nommée en allowlist, allowlist
livrée VIDE.** Il atterrit **armé** et **vert** : il détecte qu'une branche
`BanTestimony` posée après `NonServi` est inatteignable, et il n'existe
aujourd'hui **aucune violation à exempter** — c'est une mesure, pas un espoir :
`grep -rn 'required_tools' skills/bundled/ --include=*.toml` rend onze
déclarations, aucune avec un préfixe `mcp__` (§ *Le fait*). La liste
d'exceptions ne comporte donc aucune entrée et n'a aucun ticket de suivi à
référencer, parce qu'il n'y a rien à faire. Sa propriété auto-nettoyante est
portée par la paire avec le cas 3 (`operational` → `None`) et le cas 4
(`operational` non servi → `NonServi`) : le geste qui élargirait le ban à une
population qui ne lui revient pas rougit du même coup. Même forme que
`LECTEURS_NON_PUBLIANTS_MCP` au vol 2 et que le round-trip du vol 3, et même
raison : le geste qui rend une exception inutile est celui qui la fait expirer
(mika#2309).

**Les six autres cas et le livrable 6 — des tests de comportement, armés.** Leur
chemin de succès est « la fonction rend la bonne raison », pas « aucune violation
trouvée » : ce ne sont pas des options de mika#1574 à choisir, et les écrire comme
telles serait une fausse déclaration. Aucun n'a de violation préexistante à
exempter.

**Le `WARN` de la branche nouvelle n'est pas un détecteur non plus.** Son verdict
ne refuse rien, ne garde aucune CI, ne change aucune valeur : il parle à un
opérateur au moment d'un tour, et sa population attendue est vide (D5).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `McpManager::is_testimony_tool` + doc, et `pub(crate)` sur `serveur_du_nom_namespace` | 20 |
| `RaisonDesarmement` (4 variantes, aucune méthode) + doc | 25 |
| `raison_de_desarmement` (fonction pure, 4 prédicats dans l'ordre) + doc | 50 |
| `filter_available_required_tools` : corps recâblé en `match` journalisant + maj du doc-comment | 50 |
| tests de `raison_de_desarmement` (7 cas) | 90 |
| test de bout en bout par `filter_available_required_tools` | 25 |

Total estimé : 260 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**260 < 279 : un seul vol, et voici pourquoi il tient.** Trois mesures, pas trois
espoirs :

- **Aucune signature ne change, et c'est mesuré** (D4) : `is_testimony_tool` vit
  sur un manager que `filter_available_required_tools` reçoit déjà, donc les ~15
  sites d'appel du module de tests ne sont pas touchés. Les plans qui meurent au
  plafond meurent sur la propagation ; il n'y en a aucune ici.
- **Le plus gros livrable est un remplacement, pas un ajout.** Le livrable 4
  retire les ~45 lignes du corps actuel pour en poser 50, dont les trois blocs de
  journalisation **recopiés littéralement** (D2 — aucune formulation à écrire, à
  relire, ni à arbitrer). Le comptage brut que la règle impose surévalue
  structurellement ce livrable ; c'est le seul des six dans ce cas.
- **Deux fichiers, et les deux sites sont déjà localisés.** `mcp/mod.rs`
  (20 lignes) et une région contiguë d'`agent_loop/mod.rs` (`:9054-9110` pour le
  code, `:15234-15345` pour le harnais de tests, qui **existe** et n'est pas à
  bâtir : `make_registry_with_tool`, `make_resolved_skill_tool`, `served`,
  `McpManager::sans_connexion_pour_tests`).

**Calibrage mesuré, parce qu'une estimation qui ne se compare pas à un réel n'est
pas une mesure.** `git show --numstat 91a96cd2` (vol 2), hors `docs/` et
`todos/` : **456 ajoutées** pour **265 estimées**, facteur **1,72×**,
doc-comments déjà comptés. Appliqué ici, 260 estimées projettent ≈ 445 réelles —
sous les 456 que le vol 2 a livrées **en tenant ses tours**, dans ce même
`agent_loop/mod.rs`. C'est la comparaison la plus proche que ce ticket possède,
et elle est favorable.

**Convention de comptage, déclarée parce qu'elle décide de la marge.** Les
doc-comments des fichiers `.rs` sont **comptés** livrable par livrable, comme aux
vols 2 et 3 — ce vol en écrit beaucoup (D1, D2 et D3 vivent chacune dans un
doc-comment), et les exclure maquillerait l'estimation. Les trois points de
doctrine du livrable 7 ne sont **pas** comptés : c'est de la prose sous `docs/`,
et la règle compte les lignes de code.

**Pas de découpage en vols, et la raison n'est pas la marge.** Les six livrables
sont **un seul geste** : livrer l'enum sans la fonction pure ne compile sur rien,
livrer la fonction pure sans le recâblage laisse deux décisions concurrentes dans
le fichier (le défaut même que le vol 2 a payé), et livrer le recâblage sans les
tests livre la branche **sans la seule chose qui prouve qu'elle est atteignable**
(D3). Aucune coupe ne produit deux PR mergeables seules ; un vol « enum +
fonction pure + tests, câblage au suivant » serait du code mort derrière une
garde verte. Le total tient sous la borne, donc la question ne se pose pas — mais
elle ne se poserait pas autrement.

## Verification contract

| ce qui est prouvé | par quoi |
|---|---|
| un outil MCP testimony requis est désarmé sous une raison qui **nomme le ban**, pas `not_served` | livrable 5, cas 1 |
| l'ordre des branches est la propriété, pas une préférence — une branche posée après `NonServi` est inatteignable | même cas, constaté **rouge** avant le livrable 3 (§ *Ordre de commit*) |
| la branche ne dépend pas de `served` | livrable 5, cas 2 |
| le ban ne mord pas sur un grade `operational` | livrable 5, cas 3 |
| `reason = "not_served"` reste rendu sur la population qui lui revient (mika#2757 intact) | livrable 5, cas 4 |
| `reason = "turn_forbids_dispatch"` reste rendu (mika#2757 intact) | livrable 5, cas 6 |
| le `WARN` de la branche 1 est inchangé | livrable 5, cas 5 + relecture du bras recopié (D2) |
| un outil absent de la map des grades n'est pas banni | `is_testimony_tool` rend `false` — livrable 5, cas 7 |
| le recâblage n'a pas changé le comportement observable | livrable 6 |
| aucune signature n'a changé | les ~15 sites d'appel existants de `filter_available_required_tools` passent **sans modification de leur corps** |
| rien des vols 1 à 3 n'a régressé | `tests/eval/test_mcp_testimony_non_publication_2814.rs`, `mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence`, `mika2814_un_seul_appel_publie_et_ensemence`, `mika1959_lensemencement_verse_les_grades_mcp_sans_ecraser_les_skills` et `mika2814_les_cles_connues_egalent_les_champs_serialises` passent sans modification de leur corps |
| mika#2757 n'a pas régressé bout en bout | `tests/fallthrough_withheld_tools_required_2517.rs` et les tests `mika2757_*` d'`agent_loop/mod.rs` passent inchangés |

## Definition of Done

1. `raison_de_desarmement` existe, est pure, et est le **seul** site qui décide
   d'un désarmement : `filter_available_required_tools` ne contient plus de
   prédicat de retrait, seulement un `match` journalisant.
2. `RaisonDesarmement` porte quatre variantes et **aucune** méthode de
   conversion ; les littéraux `reason = …` vivent dans leurs bras.
3. La variante `BanTestimony` est évaluée **avant** `NonServi` et **après**
   `Inexistant`.
4. Un outil MCP testimony requis produit un `WARN`
   `event = "required_tool_disarmed", reason = "mcp_testimony_banned"` portant
   `tool` et `server`.
5. Le `WARN` de la branche 1 et les deux `INFO` de mika#2757 sont inchangés —
   mêmes niveaux, mêmes champs, mêmes messages.
6. `McpManager::is_testimony_tool` existe et rend `false` pour un outil absent de
   la map des grades.
7. La signature de `filter_available_required_tools` est inchangée, et aucun de
   ses sites d'appel existants n'est modifié.
8. Les sept cas du livrable 5 et le cas du livrable 6 passent.
9. Le doc-comment de `filter_available_required_tools` liste quatre raisons et
   dit pourquoi l'ordre est load-bearing.
10. `non-transit-data-grade.md` ne porte plus de ligne
    `Tracked in: senara-solutions/mika#2814`, sa ligne § *Where the pieces live*
    décrivant l'ensemencement est corrigée, et son § *Change log* porte l'entrée
    de ce vol.
11. `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` et
    `scripts/verify-pipeline.sh origin/main` passent.
12. Le corps de PR porte **`Closes #2814`** : c'est le dernier vol, et les deux
    résidus de #2832 comme les deux points de la revue de #2816 sont tous fermés.

## Acceptance criteria

Dérivés du corps du ticket (point 3 de ses *Points à trancher*, et la section
*Résidus de la revue de #2816*), du périmètre cadré par le vol 3 § *Ce que ce vol
ne prend pas*, et des sections ci-dessus — le ticket ne porte pas de section
`## Acceptance criteria`.

1. **AC1 — La raison de désarmement nomme le ban testimony.** Un outil MCP dont
   le serveur déclare `testimony` et qu'une skill exige en `required_tools` est
   désarmé sous une raison qui dit *pourquoi* : le ban doctrinal, et non
   `not_served`. Deux populations ne rendent plus les mêmes octets
   (classe mika#2205).
2. **AC2 — La branche est atteignable, et c'est prouvé plutôt qu'affirmé.** Un
   outil MCP testimony n'est jamais servi, donc une branche posée après
   `not_served` serait inatteignable avec tous les tests verts. Un test épingle
   l'ordre, et le rouge est constaté avant le câblage.
3. **AC3 — La propriété est testable sans capture de journal.** La décision est
   extraite en fonction pure rendant la raison ; les tests interrogent la
   variante, pas une chaîne de log — le dépôt n'a pas de harnais `tracing`, et
   une assertion sur des lignes capturées épinglerait la formulation.
4. **AC4 — mika#2757 est intact.** Les trois raisons existantes gardent leurs
   niveaux, leurs champs et leurs messages ; leurs tests passent sans
   modification de leur corps.
5. **AC5 — Le ban ne s'élargit pas.** Un outil MCP `operational` n'est jamais
   désarmé pour cette raison, et un outil absent de la map des grades est
   publiable — le défaut est de publier, pas de censurer.
6. **AC6 — Aucune propagation de signature.** `filter_available_required_tools`
   garde sa signature ; aucun de ses ~15 sites d'appel n'est touché.
7. **AC7 — Aucune régression sur les vols 1 à 3**, et la doctrine ne porte plus
   de ligne `Tracked in: senara-solutions/mika#2814` : le ticket est clos par ce
   vol.
