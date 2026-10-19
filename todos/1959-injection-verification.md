# mika#1959 — vérification par injection (AC1, AC2, AC3, AC4)

Doctrine `feedback_verify_pipeline_passes_without_the_fix` ; précédent
`todos/mika-2267-injection-verification.md`.

**Ce que ce document établit :** les quatre acceptance criteria de mika#1959
rougissent quand on retire le correctif qu'ils prétendent tenir. Le ticket ferme
le contournement F5 de la revue adverse de PR#1956 — un outil MCP atteint
`execute_tool` par le troisième étage du dispatch, n'est pas un `skill_tool`,
donc la map L4 `skill_data_grades` était **vide pour lui**. Un contournement qui
ne casse rien de visible ne peut être tenu que par des tests dont on a mesuré le
rouge.

Les quatre inversions ont été appliquées une par une, mesurées, puis **restaurées
et re-vérifiées au vert** (§ Retour au vert, en fin de document).

---

## AC1 — `mika1959_un_manifeste_sans_grade_vaut_operational`

**Ce qu'il tient :** un manifeste MCP sans `data_grade` vaut `Operational`. Le
corps du ticket demande « default missing field to `reference` (deny for
testimony path) » ; `reference` se lit `operational` dans l'enum binaire livré
par PR#1956 (décision D1 du plan). Le défaut est **porteur** : un défaut
`Testimony` refuserait tous les serveurs MCP installés avant ce ticket, et un
ticket p2 de forward-compat casserait la production au déploiement.

**Inversion** — `mcp/config.rs`, champ `McpServerConfig.data_grade` :
`#[serde(default)]` → `#[serde(default = "grade_injection_ac1")]` avec un helper
rendant `DataGrade::Testimony`.

**Mesure :**

```
---- mcp::config::tests::mika1959_un_manifeste_sans_grade_vaut_operational stdout ----
panicked at crates/mika-agent/src/mcp/config.rs:809:9:
assertion `left == right` failed: mika#1959 — champ absent ⇒ `Operational`. Un défaut `Testimony` refuserait tous les serveurs installés avant ce ticket.
  left: Testimony
 right: Operational

---- mcp::tests::mika1959_un_serveur_sans_champ_ou_absent_vaut_operational stdout ----
panicked at crates/mika-agent/src/mcp/mod.rs:739:9:
assertion `left == right` failed: champ absent ⇒ défaut sûr : le serveur tourne, L4 ne tire pas
  left: Testimony
 right: Operational

test result: FAILED. 7 passed; 2 failed
```

**Deux tests rougissent, sur deux modules**, et c'est la propriété qui compte :
le défaut est lu une fois au parse (`config.rs`) et une seconde fois à la
jointure (`mcp/mod.rs`, le `unwrap_or_default()` du serveur hors config). Un
correctif qui ne corrigerait qu'un des deux chemins laisserait l'autre rouge.

**Les deux autres tests AC1 ne sont pas injection-vérifiés, et c'est délibéré.**
`mika1959_les_deux_variantes_se_declarent_explicitement` épingle le mapping
sérde des deux valeurs ; son inversion est le renommage d'un littéral et
n'apprend rien. `mika1959_un_grade_inconnu_dont_reference_fait_echouer_le_parse`
tient une **absence** — qu'aucun grade inconnu ne soit deviné, `"reference"`
compris. L'inverser demanderait d'ajouter un `#[serde(other)]`, c'est-à-dire
d'écrire le défaut devinant qu'il refuse : l'inversion *est* le défaut, pas une
mesure de celui-ci.

---

## AC2 — `mika1959_chaque_outil_porte_le_grade_de_son_serveur`

**Ce qu'il tient :** la jointure config × routage. Chaque outil namespacé
`mcp__<serveur>__<outil>` porte le grade **de son serveur**, et deux serveurs du
même manifeste ne se contaminent pas.

**Inversion** — `mcp/mod.rs`, `mcp_tool_data_grades` : le corps de la jointure
remplacé par `(namespaced.clone(), DataGrade::Operational)`, c'est-à-dire la
forme qu'aurait un correctif qui ensemence la map sans la joindre — map peuplée,
tous les outils au grade permissif. C'est le contournement F5 déguisé en
correctif : la garde d'appariement AC4 reste verte, la map n'est plus vide, et
aucun outil n'est gardé.

**Mesure :**

```
---- mcp::tests::mika1959_chaque_outil_porte_le_grade_de_son_serveur stdout ----
panicked at crates/mika-agent/src/mcp/mod.rs:705:9:
assertion `left == right` failed: le grade doit venir du serveur qui route l'outil, pas d'une constante
  left: Operational
 right: Testimony

test result: FAILED. 8 passed; 1 failed
```

**Ce que l'inversion a corrigé dans le test lui-même.** Au premier passage les
trois `assert_eq!` finaux de ce test n'avaient **pas de message** : le rouge
affichait `left: Operational / right: Testimony` sans dire quelle propriété
cassait. Les trois portent maintenant leur message — le premier nomme
littéralement le défaut injecté (« pas d'une constante »). C'est l'injection qui
l'a révélé, pas une relecture : un test dont on n'a jamais vu le rouge n'a pas de
message utile par construction.

---

## AC3 — `mika1959_un_outil_mcp_testimony_est_refuse_avant_call_tool`

**Ce qu'il tient :** un outil MCP testimony est refusé **avant** le troisième
étage du dispatch — avant `McpManager::call_tool`, donc avant tout appel
distant : le serveur ne voit pas la requête.

**Inversion** — `tool_execution/dispatch.rs`, garde L4 : ajout de
`&& !name.starts_with("mcp__")` à la condition. C'est F5 écrit en une ligne : la
garde existe, elle est au bon endroit, elle ne couvre pas les outils MCP.

**Mesure :**

```
---- tool_execution::dispatch::tests::mika1959_un_outil_mcp_testimony_est_refuse_avant_call_tool stdout ----
panicked at crates/mika-agent/src/tool_execution/dispatch.rs:746:9:
assertion `left == right` failed: le corps de refus doit être celui de la garde L4
  left: "MCP server 'journal-perso' is not connected."
 right: "{\"error\":\"testimony_grade_forbidden\",\"doctrine\":\"mika#1798\",\"reason\":\"This tool's owning skill declares data_grade = \\\"testimony\\\". …\"}"

test result: FAILED. 8 passed; 1 failed
```

**Le `left` de ce rouge est la mesure même du contournement.**
`"MCP server 'journal-perso' is not connected."` est la phrase que rend
`McpManager::call_tool` quand il est **atteint** sur un serveur non connecté —
le témoin d'atteinte du troisième étage, construit exprès pour cela par
`McpManager::sans_connexion_pour_tests`. Elle sort ici parce que le dispatch a
traversé la garde et appelé le manager. Sans ce témoin le test serait
tautologique : un `mcp_manager: None` sauterait l'étage de toute façon et le
refus L4 ne prouverait rien.

La première assertion du test (`sortie.is_error`) **reste verte sous
l'injection** — « pas connecté » est aussi une erreur. C'est pourquoi l'assertion
porteuse est celle sur le *corps* du refus, et pourquoi le contrôle positif
jumeau existe.

**Contrôle négatif associé, non injecté mais porteur :**
`mika1959_le_meme_outil_en_operational_nest_pas_refuse_par_l4`. Même montage, seul
le grade change ; il assère le témoin **positivement**
(`assert_eq!(sortie.content, temoin_call_tool_atteint())`). Un correctif qui
refuserait *tous* les outils MCP rendrait AC3 vert en cassant MCP entièrement —
ce test est ce qui l'interdit. Les deux s'assertent mutuellement.

---

## AC4 — `mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4`

**Ce qu'il tient :** tout site de production qui publie `mcp.tool_definitions()`
au modèle appelle `seed_mcp_data_grades`. Deux sites, deux ensemencements
(conversation `:5608`/`:5611`, équipe `:7676`/`:7679`) ; le site
silencieux/callback/heartbeat reçoit `None` pour le manager et est hors
population par construction (décision D3).

**Inversion** — `agent_loop/mod.rs` : l'appel `seed_mcp_data_grades` du site
conversationnel (`:5611`) retiré, c'est-à-dire l'état exact d'un futur
contributeur qui ajoute une publication sans l'ensemencer.

**Mesure :**

```
---- agent_loop::tests::mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4 stdout ----
panicked at crates/mika-agent/src/agent_loop/mod.rs:19091:9:
assertion `left == right` failed: mika#1959 — chaque site publiant `mcp.tool_definitions()` au modèle doit appeler `seed_mcp_data_grades`. Publications (2) :
agent_loop/mod.rs:5608: skill_tool_defs.extend_from_slice(mcp.tool_definitions());
agent_loop/mod.rs:7673: skill_tool_defs.extend_from_slice(mcp.tool_definitions());

Ensemencements (1) :
agent_loop/mod.rs:7676: seed_mcp_data_grades(&mut skill_data_grades, params.mcp_manager);

Si vous venez d'ajouter un site de publication, ensemencez-le — ne l'allowlistez pas : un outil MCP testimony publié sans grade dans la map L4 est le contournement F5 de mika#1798, et rien de visible ne casse.
  left: 2
 right: 1

test result: FAILED. 8 passed; 1 failed
```

*(Les numéros de ligne du site équipe sont ceux de l'état **injecté**, qui porte
trois lignes de moins au-dessus. Restauré, ce site est à `:7676`/`:7679`.)*

**La mesure la plus importante de ce document est ce qui N'A PAS rougi.** Sous
cette inversion, les huit autres tests mika#1959 restent **verts** — y compris
les deux tests comportementaux AC3, qui bâtissent leur map à la main et ne
peuvent donc pas voir que la production ne la peuple plus. Le site
conversationnel est le chemin de **tous** les tours d'utilisateur ; un outil MCP
testimony y redevient appelable, en silence, et aucune assertion de comportement
ne le dit.

C'est la classe que mika#2158 (`grooming_marker`) a nommée, et la raison d'être
de ce test : sa régression ne rend aucune décision fausse, elle la rend
**invisible**. La sortie du rouge est d'ailleurs la réparation : elle liste les
deux sites et le seul ensemencement, donc elle nomme la ligne à écrire.

**Pourquoi cette garde est tout de même injection-vérifiée, contrairement aux
scans de source de mika#2267** (§ *Ce qui n'est PAS injection-vérifié* de ce
document-là) : l'inversion n'est pas ici « ajouter un second site », un geste
trivial par construction. C'est **retirer une ligne du correctif de ce
ticket-ci** — l'une des deux lignes qui ferment F5. La mesure porte donc sur le
correctif, pas sur la forme du scan.

**Contrôle de bonne foi associé, non injecté :**
`mika1959_la_garde_dappariement_rougit_sur_un_site_non_ensemence`, cinq cas sur
fragments de source littéraux — un troisième site non ensemencé (pubs=1,
seeds=0), le même ensemencé (1=1), une lecture nue de `tool_definitions()` qui
n'est pas une publication, la **définition** `fn seed_mcp_data_grades(` qui n'est
pas un appel, et de la prose nommant les deux aiguilles qui n'est ni l'un ni
l'autre. Ce dernier cas est la forme Signal S de mika#2050 ; les deux premiers
sont ce qui distingue « la garde compte » de « la garde compte juste ».

---

## Retour au vert

Les quatre inversions ont été restaurées à l'identique — la seule différence
avec l'état d'avant injection est l'ajout des trois messages d'assertion révélés
par AC2. État final :

```
cargo test -p mika-agent --lib mika1959
running 9 tests
test mcp::config::tests::mika1959_un_manifeste_sans_grade_vaut_operational ... ok
test mcp::config::tests::mika1959_les_deux_variantes_se_declarent_explicitement ... ok
test mcp::config::tests::mika1959_un_grade_inconnu_dont_reference_fait_echouer_le_parse ... ok
test mcp::tests::mika1959_un_serveur_sans_champ_ou_absent_vaut_operational ... ok
test mcp::tests::mika1959_chaque_outil_porte_le_grade_de_son_serveur ... ok
test agent_loop::tests::mika1959_la_garde_dappariement_rougit_sur_un_site_non_ensemence ... ok
test tool_execution::dispatch::tests::mika1959_le_meme_outil_en_operational_nest_pas_refuse_par_l4 ... ok
test tool_execution::dispatch::tests::mika1959_un_outil_mcp_testimony_est_refuse_avant_call_tool ... ok
test agent_loop::tests::mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4 ... ok
test result: ok. 9 passed; 0 failed
```

Le détail de la suite complète (workspace, clippy, lints dépôt,
`scripts/verify-pipeline.sh`) est dans le corps de la PR.

## Ajout post-sauvetage — collision de nom dans `seed_mcp_data_grades`

La revue (`/ce:review`, sécurité + adversarial) a relevé que l'insertion
last-write-wins pouvait rabattre un `Testimony` posé par une skill déclarant un
outil `mcp__<s>__<t>` — le préfixe n'est réservé qu'à la résolution des
`required_tools`, rien n'empêche cette collision. Correctif : le grade le plus
strict l'emporte. Test : `mika1959_lensemencement_verse_les_grades_mcp_sans_ecraser_les_skills`.

**Inversion** — le corps redevient `grades.insert(tool.clone(), *grade);`.

**Mesure :**

```
assertion `left == right` failed: sur collision, le grade le plus strict l'emporte — un serveur MCP muet ne rabat jamais un `Testimony` de skill
  left: Some(Operational)
 right: Some(Testimony)
test result: FAILED. 0 passed; 1 failed
```

Restauré, vert (`1 passed`).

## Refs

- Parent : mika#1798 (bake de l'invariant non-transit)
- Preuve F5 : https://github.com/senara-solutions/mika/pull/1956#issuecomment-5382232239
- Résidu D5 (publication, pas appel) : mika#2814
- Doctrine : `crates/mika-agent/docs/non-transit-data-grade.md` § *Layer 4* point 2
