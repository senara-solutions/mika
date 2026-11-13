# Plan — feat(mcp) : un outil MCP testimony n'est jamais publié au modèle, équivalent L2 côté MCP (mika#2814, vol 1)

## Pourquoi

mika#1959 a mis la troisième tier de dispatch sous la garde L4 : un serveur MCP
déclare `data_grade = "testimony"` dans `mcp.json`, le grade est joint à chacun
de ses outils namespacés (`mcp_tool_data_grades`), la map `skill_data_grades`
est ensemencée aux deux sites qui publient `mcp.tool_definitions()`, et
`tool_execution::dispatch::execute_tool` refuse l'appel **avant** les trois
tiers. Aucun appel distant, aucun effet de bord.

Le refus porte sur **l'appel**, pas sur la **publication**. L'outil reste dans
`McpManager::tool_definitions()`, donc dans le lot envoyé au modèle, donc le
modèle peut encore le **proposer** — « je peux lire ton journal avec
`mcp__journal__read_entry`, veux-tu ? ». La doctrine mika#1798 interdit de
proposer autant que d'accéder :

> Mika may NEVER access **nor propose accessing** testimony-grade data.

Côté skills cette moitié est structurelle depuis L2
(`SkillRegistry::apply_testimony_grade_ban`, `skills/mod.rs:677`) : l'outil est
**évincé du registre**, donc jamais publié. Il n'existe aucun équivalent côté
MCP. C'est la décision D5 du plan de mika#1959, prise hors périmètre et tracée
plutôt que laissée implicite.

Le filet existant — la garde EndTurn 5h `testimony_access_proposal` (mika#1960
phase 2, étendue par mika#2627) — lit le texte sortant. C'est une couche 1,5,
probabiliste sur la formulation. Ce vol livre la porte structurelle : le modèle
ne reçoit pas la définition, donc il n'a rien à formuler.

## Ce que le grooming a tranché

Les trois points que le corps du ticket laissait ouverts sont tranchés en D1,
D3 et D5. Les deux résidus de la revue de #2816 sont tranchés aussi — le
**choix** de D7 est fait ici, comme le ticket l'exige (« c'est donc ici que le
choix doit être fait ») ; son implémentation et l'audit de clés inconnues
partent en **vol 2**, pour une raison mécanique seule : la borne par vol. Voir
§ *Taille estimée* et la ligne `Tracked in:`.

## Décisions

### D1 — Le filtre vit dans `connect_all`, pas aux sites de publication

Le ticket pose l'alternative : filtrer à `connect_all` (le manager ne porte
jamais ces définitions) ou aux deux sites de publication (l'inventaire reste
entier pour les lecteurs non-publiants). **C'est `connect_all`**, et la raison
n'est pas la simplicité : les deux lecteurs dits « non-publiants » sont eux
aussi des vecteurs de proposition.

- `agent_loop/mod.rs:~1554` — `available_tool_names` réunit builtins, outils de
  skills et les **noms** de `mcp.tool_definitions()` pour la détection d'appel
  d'outil en prose (#569). Un nom laissé là ne « renseigne » pas le modèle : il
  convertit une mention en **appel réel**. L4 refuserait l'appel, mais la
  proposition aurait déjà eu lieu, et c'est elle que mika#1798 interdit.
- `kg/domain_builder.rs:765-785` — la projection KG écrit un `tool:*` par
  définition. Un outil testimony y devient **récupérable par
  `query_knowledge_graph`**, c'est-à-dire re-proposable un tour plus tard, hors
  de tout contexte MCP.

Filtrer aux deux sites de publication laisserait ces deux-là ouverts. Filtrer à
`connect_all` les ferme tous les quatre d'un seul geste, présents et futurs : un
cinquième lecteur de `tool_definitions()` écrit demain hérite de la propriété
sans rien savoir d'elle.

### D2 — Le filtre consomme la map que L4 lit, pas une seconde lecture du config

`mcp_tool_data_grades(config, &tool_routing)` est déjà calculé dans
`connect_all` (`mcp/mod.rs:169`) et c'est cette map qui, ensemencée dans
`skill_data_grades`, arme L4. Le filtre prend **cette map** en entrée, jamais
`config.mcp_servers` à nouveau.

Deux lectures indépendantes du même fait peuvent divergir : une qui nomme le
serveur par sa clé, une qui le nomme par le segment du namespace, et un jour
l'une dit testimony là où l'autre dit operational. Dans ce sens-là la
divergence est muette et grave — publié mais refusé à l'appel est un bug
visible ; non publié mais autorisé à l'appel ne l'est pas.

### D3 — `tool_routing` et `tool_data_grades` restent intacts : dépublier ne désarme pas L4

Vérifié sur pièce (`tool_execution/dispatch.rs:495-545`) : le refus L4 est clé
uniquement sur `dispatch.skill_data_grades.get(name)` et court avant les trois
tiers ; `execute_tool` n'a pas été modifié par mika#1959. Donc **seul**
`tool_definitions` est amputé ; `tool_routing` et `tool_data_grades` gardent
l'inventaire complet.

C'est la condition pour qu'un appel halluciné sur un nom dépublié soit refusé
**comme banni testimony** (`TOOL_TESTIMONY_GRADE_FORBIDDEN`, `event =
"tool_testimony_ban"`) et non comme outil inconnu. Sans cette précaution,
« cet outil n'existe pas » et « cet outil est interdit par doctrine » rendraient
les mêmes octets — classe mika#2205, et ici c'est la seconde qui est l'alerte.
`filter_available_required_tools` (`agent_loop/mod.rs:~8959`) lit déjà
`is_mcp_tool`, donc le routage, et reste cohérent par construction.

### D4 — La logique est une fonction libre, parce que `connect_all` est intestable

`connect_all` ouvre de vraies connexions stdio/HTTP ; c'est le même mur que
mika#1959 D4 a rencontré, et la même parade :

```rust
pub(crate) fn retain_publiables(
    tool_definitions: Vec<ToolDefinition>,
    tool_data_grades: &HashMap<String, DataGrade>,
) -> Vec<ToolDefinition>
```

Pure, sans `self`, testable en table. `connect_all` l'appelle en une ligne entre
le calcul des grades et la construction de `Self`.

Une définition **absente** de la map des grades est **publiée**. Depuis
`connect_all` ce cas est inatteignable (les deux structures naissent du même
`tool_routing`), mais la signature l'autorise, donc il est couvert par un test :
un défaut futur qui ferait divergir les deux inventaires doit rendre un
comportement nommé, pas un comportement de hasard. Le défaut est *publier*
plutôt que *censurer* parce que la censure muette d'un outil operational serait
un incident sans message, là où une publication de trop reste tenue par L4.

### D5 — Observabilité : une ligne par outil retiré, et trois compteurs sur la ligne du manager

Un serveur entièrement testimony devient invisible. Deux surfaces, aucune
nouvelle structure de données (même arbitrage que `apply_testimony_grade_ban`,
dont la surface v1 est délibérément un WARN et rien d'autre — un champ de
registre sans consommateur décrit n'est pas une surface) :

1. **Un `tracing::warn!` par outil retiré** — `event =
   "mcp_tool_testimony_unpublished"`, champs `tool` (nom namespacé), `server`,
   `doctrine = "mika#1798"`. Un WARN et non un INFO : ce n'est pas un état
   nominal silencieux, c'est une amputation du lot d'outils, et le pendant
   côté skills est un WARN.
2. **La ligne `"MCP manager initialized"`** gagne `discovered`, `published`,
   `unpublished` (`published` remplaçant l'actuel `tools`, qui comptait
   l'inventaire avant filtrage).

Table de lecture, parce que c'est elle qui fait la différence que le ticket
demande :

| observation | lecture |
|---|---|
| aucune ligne `connected to MCP server` pour S | S ne s'est pas connecté |
| `connected to MCP server` S, puis N lignes `mcp_tool_testimony_unpublished` pour S, `discovered > published` | S est connecté et ses outils sont bannis |
| `discovered == published`, `unpublished = 0` | aucun serveur testimony déclaré |
| aucune ligne `mcp_tool_testimony_unpublished` **et** `unpublished` absent de la ligne du manager | binaire antérieur à ce changement (mika#2340), jamais « rien n'a été banni » |

`unpublished = 0` est écrit, pas omis : absent ≠ zéro (mika#2331). L'instrument
n'est vivant qu'après `make deploy`.

### D6 — La doctrine est mise à jour dans le même commit

`crates/mika-agent/docs/non-transit-data-grade.md` est **crate-local seul** —
absent du tableau `DOCS=()` de `scripts/sync-agent-docs.sh` — donc l'éditer
n'entraîne aucun doc-sync et le job CI `docs-sync` n'est pas concerné. Quatre
régions mentent après ce vol si on ne les touche pas :

- § *Layer 4* point 2 (l. 208-230), dont le paragraphe de résidu (228-230) est
  exactement ce que ce vol ferme ;
- § *Ban list (v1)*, ligne MCP (l. 247-250) : « Layer 4 only — there is no
  registry-eviction equivalent on the MCP side » devient faux ;
- § *Vigilance surface* (l. 297-326), dont le bloc « **What that does NOT buy,
  on this same axis (mika#2814).** … Tracked in mika#2814 » ;
- § *Known bypass classes* (l. 390-396) et § *Change log* (l. 452+).

Hors décompte de taille (`docs/`), mais dans le même commit : une doctrine qui
décrit l'état d'avant est un piège pour le prochain lecteur.

### D7 — Le choix du résidu 1 est fait ici ; son implémentation est au vol 2

Le résidu 1 de la revue de #2816 observe que la garde d'appariement
(`mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4`) **compare des
chaînes** : une publication écrite autrement la laisse verte. Deux remèdes
étaient proposés ; le choix est **le helper unique qui publie et ensemence
ensemble** — une fonction qui prend `&mut skill_tool_defs`, `&mut
skill_data_grades` et `params.mcp_manager`, appelée aux deux sites, de sorte
qu'un troisième site ne *puisse* pas publier sans ensemencer. La garde
fonction-par-fonction est rejetée : elle resterait un détecteur à rattrapage là
où le helper rend la faute inécrivable, et elle paierait un scan de source pour
une propriété qu'un type peut porter.

Deux raisons de ne pas l'écrire dans ce vol :

1. **Mécanique** — la borne par vol. Voir § *Taille estimée*.
2. **D1 en abaisse l'urgence** — avec le filtrage à `connect_all`, un troisième
   site non ensemencé publierait désormais un inventaire **déjà sans
   testimony**. Le trou qui restait était « publie et n'ensemence pas, donc L4
   est aveugle sur ces outils » ; après ce vol c'est « publie un lot propre et
   n'ensemence pas, donc un appel halluciné rend *outil inconnu* au lieu de
   *banni* » — une dégradation d'observabilité, plus une porte ouverte.

**Le code de la garde de mika#1959 n'est pas touché par ce vol** : D1 ne déplace
aucun site de publication, les deux needles et le compte `== 2` restent exacts.
Seul son doc-comment gagne une phrase disant que l'inventaire qu'elle compte est
désormais pré-filtré. La règle du ticket — « un filtrage qui déplace la
publication doit la mettre à jour dans le même commit, pas la contourner » — est
tenue par le fait que rien ne bouge, et le DoD exige de le prouver en la faisant
passer.

Tracked in: senara-solutions/mika#2814 (vol 2)

## Quoi

### 1. `retain_publiables`, fonction libre dans `crates/mika-agent/src/mcp/mod.rs`

Prend les définitions et la map des grades (D2), rend les définitions dont le
grade n'est pas `DataGrade::Testimony` (défaut : publier, D4), émet un
`warn!(event = "mcp_tool_testimony_unpublished", …)` par retrait (D5). Le champ
`server` est extrait du nom namespacé par `MCP_PREFIX`/`MCP_SEP`, pas d'un
second parcours du config.

### 2. Câblage dans `connect_all`

Entre le calcul `mcp_tool_data_grades` (l. 169) et la construction de `Self` :
`let tool_definitions = retain_publiables(tool_definitions, &tool_data_grades);`
`tool_routing` et `tool_data_grades` passent inchangés (D3).

### 3. Les trois compteurs sur la ligne du manager

`"MCP manager initialized"` : `discovered` (avant filtrage), `published`
(`tool_definitions.len()`, remplace `tools`), `unpublished` (différence, écrite
même à zéro). La ligne par serveur `"connected to MCP server"` garde son `tools`
— c'est un compte de découverte, et c'est précisément lui qui distingue
« non connecté » de « connecté et banni » dans la table de D5.

### 4. Tests unitaires dans `mcp/mod.rs`

Sur `retain_publiables`, avec `routage()` et les helpers existants :

- un serveur testimony voit ses outils retirés, un serveur operational garde
  les siens — **dans la même table**, parce que séparés ce sont deux tests dont
  l'un peut passer sur un filtre qui retire tout ;
- une définition absente de la map est publiée (D4) ;
- `tool_routing` et `tool_data_grades` ne sont pas amputés — l'inventaire du
  refus survit (D3), assertion portée sur le manager construit par
  `sans_connexion_pour_tests` ;
- contrôle négatif de bonne foi : une table sans aucun outil testimony rend
  `unpublished == 0` et la liste inchangée, pour qu'un filtre dégénéré (« retire
  tout ») ne puisse pas passer en vert sur les cas positifs seuls.

### 5. Le test end-to-end — ce que le modèle reçoit vraiment

`crates/mika-agent/tests/eval/test_mcp_testimony_non_publication_2814.rs`, plus
sa déclaration dans `crates/mika-agent/tests/eval.rs`.

C'est la pièce décisive, et la seule qui mesure l'AC telle qu'elle est écrite.
Les tests unitaires prouvent que la fonction filtre ; le câblage de l'étape 2
(une ligne, dans une fonction intestable) est exactement le genre d'endroit où
l'on calcule un filtre et où l'on jette son résultat. Le substrat existe déjà :
`test_mcp_secret_boundary_2281.rs` monte un serveur MCP factice réel
(`tests/fixtures/mcp_secret_boundary_server.py`, deux outils) et son helper
`all_llm_request_bytes` concatène **tout ce qui a été soumis au modèle** —
prompt système, messages, résultats d'outils, définitions d'outils.

Le nouveau fichier réutilise ce fixture avec `data_grade:
DataGrade::Testimony` et assert, sur un tour `run()` complet :

1. `manager.has_connections()` — sans quoi un fixture cassé rendrait un manager
   vide et l'assertion négative serait verte sans avoir rien mesuré (le mode de
   panne que ce dépôt refuse, et que le test voisin nomme déjà) ;
2. les deux noms d'outils du fixture n'apparaissent **nulle part** dans
   `all_llm_request_bytes` ;
3. **contrôle positif, même fixture en `Operational`** : les deux noms
   apparaissent bien. C'est lui qui sépare « le filtre mord » de « le harnais ne
   soumet jamais d'outils MCP » — deux causes du même vert.

`test_mcp_secret_boundary_2281.rs` est laissé **intact** : son serveur est
`data_grade: Default::default()` (operational), ses assertions de découverte
survivent au filtrage. Le DoD exige de le rejouer.

### 6. La doctrine (D6)

Quatre régions de `crates/mika-agent/docs/non-transit-data-grade.md` et son
change log. Hors décompte.

## Fire-Disposition

Ce vol livre des détecteurs : les quatre tests unitaires de l'étape 4 et le test
e2e de l'étape 5 (chemin de succès = « aucun outil testimony publié »).

**Option (a) — exception nommée en allowlist — avec une allowlist vide par
construction.** L'unique violation de production existante est *le défaut
lui-même*, corrigé par les étapes 1 à 3 **dans le même commit** que les
détecteurs. Il n'y a donc aucune donnée à excepter, aucune entrée à rendre
grep-visible, et le détecteur atterrit **armé**. Ce vol ne livre ni allowlist ni
`#[ignore]`.

Les deux assertions qui tiennent le rôle de l'auto-nettoyage :

- **Le contrôle positif de l'étape 5.3** (fixture operational → les noms
  apparaissent) rougit si quelqu'un « corrige » un jour par un filtre trop
  large. Une exception stale, ici, ce serait un filtre qui mange de
  l'operational ; c'est cette assertion qui l'expire.
- **Le contrôle négatif de l'étape 4** (table sans testimony → liste inchangée,
  `unpublished == 0`) rougit sur un filtre dégénéré, donc sur un détecteur qui
  passerait au vert pour la mauvaise raison.

La vacuité de l'allowlist n'est pas à asserter : il n'y a pas d'allowlist à
tenir vide. Si l'implémentation découvre une violation qu'elle ne peut pas
fermer dans ce commit, la disposition bascule en (c) halte-et-remontée — pas en
`#[ignore]` silencieux.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/mcp/mod.rs` (prod : `retain_publiables`, câblage, compteurs) | 50 |
| `crates/mika-agent/src/mcp/mod.rs` (tests unitaires, 4 cas) | 70 |
| `crates/mika-agent/tests/eval/test_mcp_testimony_non_publication_2814.rs` | 75 |
| `crates/mika-agent/tests/eval.rs` (déclaration du module) | 1 |

Total estimé : 196 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 196 < 279, avec ~83 lignes (30 %) de marge — et
cette marge est voulue, parce que le poste risqué de ce vol n'est pas le code de
production (50 lignes, une fonction pure et une ligne de câblage) mais le test
e2e : harnais eval, séquence de mock LLM, fixture Python. Un vol serré sur ce
poste-là meurt en fin de pipeline, pas en écriture.

**Ce qui est renvoyé au vol 2, et pourquoi.** Les deux résidus de la revue de
#2816 coûtent ≈ 136 lignes estimées (audit de clés inconnues dans
`mcp/config.rs` : 74 prod + 62 tests), et le helper publie+ensemence de D7 avec
le retrait de la garde d'appariement qu'il rend inutile. 196 + 136 = 332 >
279 : la borne mord, donc découpage **en vols**, pas en phases. Chaque vol est
mergeable seul — le vol 1 ferme le résidu D5 de mika#1959 (le sujet du ticket)
sans rien devoir au vol 2 ; le vol 2 est un durcissement du même site qui ne
dépend du vol 1 que par le fichier qu'il touche.

**Périmètre du vol 2**, nommé pour que la suite ne se réinvente pas :

1. L'audit de clés inconnues de `mcp.json` (résidu 2) : un
   `parse_with_key_audit` soudé aux deux sites `serde_json::from_str` de
   `config.rs` (`load`, l. 128, et `load_operator_shell`, l. 189), un const
   `KNOWN_SERVER_KEYS` épinglé par un test d'aller-retour de sérialisation qui
   rougit quand un champ typé est ajouté, et un champ `did_you_mean` qui attrape
   `dataGrade` → `data_grade`. **WARN et non erreur** : `mcp.json` est un schéma
   partagé avec d'autres hôtes MCP et le chemin de chargement est
   délibérément fail-open (mika#1764 — un fichier operator-shell corrompu ne
   doit pas coincer `init_agent`). `#[serde(deny_unknown_fields)]` est rejeté
   pour la même raison, et `#[serde(flatten)]` l'est pour son coût : 22 sites de
   construction littérale de `McpServerConfig` dans le crate.
2. Le helper publie+ensemence de D7, et le retrait de la garde d'appariement
   que ce helper rend structurellement inutile.
3. L'incohérence de disponibilité de `filter_available_required_tools`, qui lit
   `is_mcp_tool` (le routage, intact par D3) et considérerait donc disponible un
   outil dépublié. **Actuellement inatteignable** : vérifié par grep, aucune
   skill bundlée ne déclare un `mcp__*` dans ses `required_tools`. Un cas de
   test l'épinglera au vol 2.

Tracked in: senara-solutions/mika#2814 (vol 2 — résidus de la revue de #2816)

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de son § *Scope proposé* et de ses trois points
à trancher.

1. **AC1 — Un outil MCP dont le serveur déclare `testimony` n'atteint jamais le
   modèle.** Mesuré sur les octets réellement soumis, pas sur un `Vec` :
   `test_mcp_testimony_non_publication_2814.rs` assert que les noms des outils
   du fixture sont absents de la concaténation de toutes les requêtes LLM d'un
   tour complet, serveur connecté attesté.
2. **AC2 — Un outil operational reste publié.** Même fixture, même harnais,
   grade operational : les deux noms sont présents. Un filtre qui retire tout ne
   satisfait pas l'AC.
3. **AC3 — Dépublier ne désarme pas L4.** `tool_routing` et `tool_data_grades`
   portent toujours l'inventaire complet après `connect_all`, donc un appel sur
   un nom dépublié est refusé comme `TOOL_TESTIMONY_GRADE_FORBIDDEN` et non
   comme outil inconnu.
4. **AC4 — Un serveur entièrement banni est lisible dans les journaux.** Une
   ligne `mcp_tool_testimony_unpublished` par outil retiré (champs `tool`,
   `server`, `doctrine`), et `discovered`/`published`/`unpublished` sur la ligne
   `MCP manager initialized`, `unpublished` écrit même à zéro. « Serveur non
   connecté » et « outils bannis » ne rendent pas les mêmes octets.
5. **AC5 — La garde d'appariement de mika#1959 passe sans avoir été modifiée
   dans son code.** `mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4`
   et son contrôle négatif à 5 cas restent verts ; seul le doc-comment change.
6. **AC6 — Le choix du résidu 1 est écrit.** Le plan nomme le remède retenu
   (helper unique publie+ensemence) avec la raison du rejet de l'alternative, et
   renvoie son implémentation par une ligne `Tracked in:`.
7. **AC7 — La doctrine ne décrit plus l'état d'avant.** Les quatre régions de
   `non-transit-data-grade.md` nommées en D6 sont à jour et le change log porte
   l'entrée.

## Definition of Done

- [ ] `retain_publiables` existe comme fonction libre dans `mcp/mod.rs`, prend
      la map des grades (D2) et publie par défaut une définition absente de la
      map (D4).
- [ ] `connect_all` l'appelle ; `tool_routing` et `tool_data_grades` sont passés
      inchangés à `Self` (D3).
- [ ] Un `warn!(event = "mcp_tool_testimony_unpublished", tool, server, doctrine)`
      par outil retiré ; `discovered`/`published`/`unpublished` sur
      `"MCP manager initialized"`, `unpublished` présent même à zéro.
- [ ] Les quatre tests unitaires de l'étape 4 passent, contrôle négatif inclus.
- [ ] `test_mcp_testimony_non_publication_2814.rs` passe : AC1 (absence), AC2
      (contrôle positif operational), attestation de connexion. Déclaré dans
      `crates/mika-agent/tests/eval.rs`.
- [ ] `test_mcp_secret_boundary_2281.rs` rejoué et vert sans modification (son
      serveur est operational).
- [ ] `mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4`,
      `mika1959_lensemencement_verse_les_grades_mcp_sans_ecraser_les_skills`,
      `mika1959_la_garde_dappariement_rougit_sur_un_site_non_ensemence`,
      `mika1959_chaque_outil_porte_le_grade_de_son_serveur` et
      `mika1959_un_serveur_sans_champ_ou_absent_vaut_operational` : verts, code
      inchangé (AC5).
- [ ] `crates/mika-agent/docs/non-transit-data-grade.md` : les quatre régions de
      D6 et le change log. Pas de `scripts/sync-agent-docs.sh` à lancer (le
      fichier n'est pas dans son `DOCS=()`), donc rien à attendre du job CI
      `docs-sync`.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
      `make lint`, et `scripts/verify-pipeline.sh origin/main`.
- [ ] `todos/2814-injection-verification.md` écrit (voir section suivante).
- [ ] Corps de PR : `Closes senara-solutions/mika#2814` **non** — le ticket
      reste ouvert pour le vol 2. Le corps porte la ligne
      `Tracked in: senara-solutions/mika#2814` pour le report, que le gate
      `pr-body-validation` exige.

## Vérification par injection (per `feedback_verify_pipeline_passes_without_the_fix`)

Un détecteur dont on n'a pas vu le rouge n'est pas un détecteur. Avant d'écrire
le correctif, ou en le retirant temporairement, enregistrer dans
`todos/2814-injection-verification.md` :

1. **Sans l'étape 2 (filtre écrit mais non câblé)** — le test e2e doit rougir en
   nommant les deux outils trouvés dans la requête. C'est le mode de panne le
   plus probable de ce vol : une fonction pure testée et un câblage oublié.
2. **Avec un filtre dégénéré (`retain_publiables` rendant `vec![]`)** — AC2
   (contrôle positif operational) doit rougir, et le contrôle négatif unitaire
   aussi.
3. **Sans l'étape 3 (compteurs)** — aucun test ne rougit, et c'est attendu : la
   ligne de journal n'est pas assertée par un test (même arbitrage que
   `apply_testimony_grade_ban`). Le noter explicitement vaut mieux que le
   laisser croire couvert ; la vérification est opérateur, par `grep` après
   `make deploy`.
4. **Sur `origin/main` (sans aucune des étapes)** — les deux nouveaux fichiers
   de test rougissent, les tests de mika#1959 restent verts. Ce dernier point
   est ce qui prouve que le vol ajoute une garantie au lieu d'en déplacer une.

## Hors périmètre

- **Le helper publie+ensemence et l'audit de clés inconnues** — vol 2, périmètre
  nommé en § *Taille estimée*, `Tracked in:` ci-dessus.
- **La garde EndTurn 5h `testimony_access_proposal`** (mika#1960, mika#2627) —
  inchangée. Elle reste utile après ce vol : elle couvre le texte, pas le lot
  d'outils, donc une proposition formulée depuis une connaissance a priori du
  modèle.
- **Un troisième grade, ou un `DataGrade` non binaire** — `DataGrade` reste
  `Operational` / `Testimony`.
- **Le filtrage par outil plutôt que par serveur** — le grade est déclaré au
  niveau du serveur dans `mcp.json` ; un grade par outil est une évolution du
  schéma, pas un résidu de mika#1959.
- **Les lignes déjà écrites en base par la projection KG avant ce déploiement**
  — un `tool:*` testimony persisté par un binaire antérieur reste interrogeable
  jusqu'au prochain `rebuild()` du domain builder, qui tourne à chaque boot et
  **élague** les entités devenues absentes (DELETE type-scopé). Le premier
  redémarrage après déploiement les retire donc ; aucune migration n'est
  nécessaire, et le dire ici évite qu'on en écrive une.

## Risques et parades

| risque | parade |
|---|---|
| Le filtre est calculé et son résultat jeté (`connect_all` intestable) | Le test e2e de l'étape 5, qui mesure les octets soumis au modèle et non le `Vec` — et l'injection n°1 qui prouve son rouge |
| Un filtre trop large mange de l'operational | AC2 / contrôle positif, dans le même fichier, sur le même fixture |
| Un test tout-vert parce que le harnais ne soumet aucun outil MCP | L'attestation `has_connections()` **et** le contrôle positif : deux causes du même vert, séparées |
| Dépublier rend un appel halluciné « outil inconnu » au lieu de « banni » | D3 : `tool_routing` et `tool_data_grades` intacts, assertés |
| Un serveur testimony devient silencieusement invisible | D5 : un WARN par outil, trois compteurs, table de lecture, `unpublished = 0` écrit |
| Un cinquième lecteur de `tool_definitions()` écrit plus tard | D1 : le filtre est en amont, donc tout lecteur hérite de la propriété |
| L'absence de ligne de journal lue comme « rien n'a été banni » | Dernière ligne de la table de D5 : absence ⇒ binaire antérieur (mika#2340) |

## Solutions apparentées

- `docs/plans/2026-10-19-001-feat-1959-mcp-manifest-data-grade-plan.md` — le vol
  précédent, dont D4 (fonction libre pour contourner `connect_all`) et D5 (ce
  résidu) sont repris tels quels.
- `crates/mika-agent/src/skills/mod.rs:677` — `apply_testimony_grade_ban`, le
  précédent L2 dont ce vol est le miroir, y compris son arbitrage
  d'observabilité (un WARN, pas un champ de registre).
- `crates/mika-agent/docs/non-transit-data-grade.md` — les quatre couches et le
  § *Vigilance surface*, qui nomme déjà ce ticket.

## Potentiel de composition

La forme « une fonction pure sur l'inventaire, câblée en un point, et un test
qui mesure les octets soumis au modèle plutôt que la structure intermédiaire »
se transpose à toute autre amputation du lot d'outils (grades à venir,
restrictions par tenant, outils désarmés par label). Le couple
*assertion de connexion* + *contrôle positif de même fixture* est le patron
réutilisable : il sépare « la garde mord » de « l'instrument ne mesure rien »,
qui sont les deux causes du même vert.
