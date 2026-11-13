# mika#2814 vol 2 — le rouge mesuré du helper et de sa garde

Doctrine `feedback_verify_pipeline_passes_without_the_fix` ; précédent immédiat
`todos/2814-injection-verification.md` (vol 1), dont ce document est la suite.

**Ce que ce document établit.** Trois choses, mesurées le **2026-10-20** plutôt
que raisonnées :

1. Ce qu'un site publiant **sans** ensemencer rend réellement — et que la prose
   du vol 1 s'était trompée sur ce point ;
2. que la garde d'appariement de mika#1959 **rougit** quand les deux sites
   passent par le helper, donc qu'elle devait bouger dans ce commit et non être
   contournée ;
3. que la garde refondue rougit sur chacune de ses trois assertions, y compris
   sur la forme que l'ancienne aiguille ne voyait pas.

Aucun de ces trois cas ne laisse de test permanent derrière lui : les cas 1 et 2
portent sur un état qui n'existe plus, et le cas 3 est tenu en permanence par
`mika2814_la_garde_du_helper_rougit_sur_une_publication_hors_chemin`. Ce qui est
durable, ce sont les **messages** d'assertion cités ci-dessous, pas les numéros
de ligne — ils appartiennent à l'état injecté et ont encore bougé au
`cargo fmt` final.

---

## Cas 1 — ce qu'un site non ensemencé rend **réellement**

**Ce qu'il mesure.** Le vol 1 avait écrit, dans le doc-comment de
`seed_mcp_data_grades` et dans celui de la garde, qu'un site publiant sans
ensemencer ne coûtait qu'une dégradation d'observabilité : « un appel halluciné
rend *outil inconnu* au lieu de *banni* ». Si c'était vrai, le helper de ce vol
serait de la cosmétique. Il fallait donc le mesurer avant de le réparer.

**Inversion.** Un test temporaire dans le module de test de
`crates/mika-agent/src/tool_execution/dispatch.rs`, reproduisant exactement
l'état d'un site qui publie sans ensemencer : un `ToolDispatchCtx` dont
`skill_data_grades` est **vide**, et un `McpManager` (via
`McpManager::sans_connexion_pour_tests`) portant `mcp__journal-perso__read_entry`
dans son routage avec le grade `Testimony`. L'assertion attendait la réponse que
la prose annonçait — `Unknown tool: …` — et rendait ce qu'`execute_tool`
retourne vraiment.

**Mesure :**

```
panicked at crates/mika-agent/src/tool_execution/dispatch.rs:783:9:
assertion `left == right` failed: MESURE : voici ce que rend reellement un site non ensemence
  left: "MCP server 'journal-perso' is not connected."
 right: "Unknown tool: mcp__journal-perso__read_entry"

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 6601 filtered out
```

**Ce que ce `left` dit.** La cascade ne s'est pas arrêtée. `tool_routing` est
resté intact par construction (le filtre L2 du vol 1 n'ampute ni le routage ni
les grades — c'est
`mcp::tests::mika2814_le_routage_et_les_grades_ne_sont_pas_amputes`), donc à
l'étage 3 d'`execute_tool` `mcp.is_mcp_tool(name)` répond **vrai** sans consulter
la map L4, et `mcp.call_tool` **part**. Le message rendu est celui du transport —
le serveur n'est pas connecté *dans ce test* — ce qui veut dire qu'avec un
serveur connecté l'appel distant aurait abouti.

**Donc la prose du vol 1 est fausse, et mesurée fausse.** Un site publiant sans
ensemencer ne dégrade pas l'observabilité : il laisse partir l'appel distant sur
un outil testimony, pour tout nom obtenu autrement que par l'inventaire publié —
hallucination, mention en prose par l'utilisateur, historique d'avant la
connexion du serveur, résumé de conversation. C'est le contournement F5 de
mika#1798 **entier**. La correction de la phrase fait partie de ce commit
(§ *La porte que ce helper tient est une porte, pas une trace* sur
`publier_outils_mcp`), et c'est cette mesure qui l'exigeait.

---

## Cas 2 — la garde de mika#1959 rougit sur le helper

**Ce qu'il mesure.** Que les livrables 2 (les deux sites appellent le helper) et
4 (la garde refondue) ne pouvaient **pas** être commités séparément. Le plan le
prédit ; il reste à le voir.

**Inversion.** L'état intermédiaire réel, pendant la rédaction : les deux sites
de publication réduits à `publier_outils_mcp(…)`, le helper écrit, et la garde
de mika#1959 encore en place avec son aiguille
`extend_from_slice(mcp.tool_definitions())`.

**Commande :** `cargo test -p mika-agent --lib mika1959`

**Mesure :**

```
thread 'agent_loop::tests::mika1959_tout_site_publiant_des_outils_mcp_ensemence_la_map_l4'
panicked at crates/mika-agent/src/agent_loop/mod.rs:20168:9:
assertion `left == right` failed: mika#1959 — exactement deux sites publient les outils MCP au modèle : la conversation et l'agent d'équipe. Le mode silencieux est hors population (il reçoit `None`). Trouvés :
agent_loop/mod.rs:8235: defs.extend_from_slice(mcp.tool_definitions());
  left: 1
 right: 2

test result: FAILED. 9 passed; 1 failed; 0 ignored; 0 measured; 6591 filtered out; finished in 1.02s
```

**Écart avec la prédiction du plan, enregistré plutôt que lissé.** Le plan
annonçait « les deux aiguilles disparaissent des sites, `pubs.len()` tombe à
`0` ». Le compte mesuré est **1**, pas 0 : l'aiguille continue de matcher **dans
le corps du helper lui-même**, qui est la seule ligne du dépôt à encore écrire
`defs.extend_from_slice(mcp.tool_definitions())`. La garde est rouge comme prévu,
mais pour un compte différent — et c'est la raison pour laquelle le prédicat
refondu exclut le corps du helper **par portée** (`bornes_du_helper_de_publication`)
plutôt que par fichier : la ligne légitime existe, elle est unique, et c'est
cette unicité que le vol 2 achète.

**Les neuf verts de ce cas** sont la confirmation que D6 a été tenu : les tests
de `seed_mcp_data_grades`, du filtre L2 et du routage ne bougent pas. Seule la
garde d'appariement rougit, parce que c'est la seule chose dont la propriété a
changé.

**État de départ, pour que le `9 passed` soit lisible** — avant le livrable 2 :

```
$ cargo test -p mika-agent --lib mika1959
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 6591 filtered out; finished in 1.02s
```

---

## Cas 3 — la garde refondue rougit, y compris là où l'ancienne était verte

La garde porte trois assertions. Une assertion dont on n'a pas vu le rouge n'est
pas une assertion, donc les trois ont été tirées, et **séparément** : la
première qui tombe masque les suivantes.

### 3a — assertion 2 : un troisième site publie hors du helper, en boucle

**Ce qu'il mesure.** Le résidu nommé de la revue de #2816 : l'ancienne garde
comparait des **chaînes**, donc une publication écrite autrement la laissait
verte. La forme choisie pour l'injection est celle qui échappait à l'ancienne
aiguille — une boucle `push`, sans `extend_from_slice` — et elle est injectée
dans `agent_loop/mod.rs`, c'est-à-dire **le fichier qui porte déjà un lecteur
déclaré**, pour que la mesure tranche aussi la question de la granularité.

**Inversion** — après l'appel au helper du site agent d'équipe :

```rust
if let Some(m) = params.mcp_manager {
    for d in m.tool_definitions() {
        skill_tool_defs.push(d.clone());
    }
}
```

**Commande :**
`cargo test -p mika-agent --lib mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence`

**Mesure :**

```
panicked at crates/mika-agent/src/agent_loop/mod.rs:20223:9:
mika#2814 — ces lignes de production lisent `mcp.tool_definitions()` hors du corps de `publier_outils_mcp` et hors de `LECTEURS_NON_PUBLIANTS_MCP` :
agent_loop/mod.rs:7887: for d in m.tool_definitions() {

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 6602 filtered out; finished in 1.08s
```

**Trois choses dans ce rouge.** (1) L'ancienne aiguille ne contient pas
`extend_from_slice`, donc l'ancienne garde serait restée **verte** sur cette
injection — c'est le trou que ce vol ferme, et il est aussi asservi en permanence
par l'assertion `la fixture doit bien être invisible à l'ancienne aiguille` du
contrôle négatif. (2) Le rouge **nomme le fichier et la ligne**, ce qui est la
réparation. (3) Le fichier incriminé est celui d'un lecteur déclaré, et l'entrée
de ce lecteur ne couvre pas la nouvelle ligne : le droit est attaché à une
**ligne**, pas à un fichier. Une allowlist par fichier aurait laissé cette
injection passer, à l'endroit exact où elle serait écrite — là où vivent les
boucles d'agent.

### 3b — assertion 3 : une entrée qui ne correspond plus à rien

**Ce qu'il mesure.** Que l'inventaire des lecteurs non-publiants ne peut pas
survivre à ce qu'il décrit (mika#2309). Mis-orthographier l'aiguille d'une entrée
existante ne l'isole pas — l'assertion 2 tombe d'abord, puisque la ligne réelle
devient non couverte :

```
panicked at crates/mika-agent/src/agent_loop/mod.rs:20218:9:
mika#2814 — ces lignes de production lisent `mcp.tool_definitions()` hors du corps de `publier_outils_mcp` et hors de `LECTEURS_NON_PUBLIANTS_MCP` :
kg/domain_builder.rs:768: for tool_def in mcp.tool_definitions() {
```

C'est déjà la bonne nouvelle — une entrée réécrite échoue **fermé** — mais ce
n'est pas le rouge de l'assertion 3. L'isolation est une **troisième** entrée qui
ne décrit aucun lecteur existant, les deux vrais restant couverts.

**Inversion** — entrée ajoutée à `LECTEURS_NON_PUBLIANTS_MCP` :
`("telemetry/mod.rs", "let compte = mcp.tool_definitions().len()", "un lecteur retiré depuis, dont l'entrée est restée")`.

**Mesure :**

```
panicked at crates/mika-agent/src/agent_loop/mod.rs:20243:9:
mika#2814 — ces entrées de `LECTEURS_NON_PUBLIANTS_MCP` ne correspondent plus à aucune ligne que le balayage retrouve :
telemetry/mod.rs :: let compte = mcp.tool_definitions().len()

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 6602 filtered out; finished in 1.08s
```

C'est ce qui remplace, en mieux, l'assertion « la liste est vide » de mika#1959.
Cette liste est née **non vide** et le reste : elle inventorie deux droits
légitimes, et aucun ticket de suite n'est dû pour eux. Ce qui expire n'est pas
l'exception, c'est l'entrée qui a cessé d'être vraie.

### 3c — assertion 1 : le compte d'appels

Elle n'a pas eu besoin d'une injection dédiée : c'est l'assertion qui a rougi au
**cas 2** ci-dessus sous sa forme mika#1959 (`left: 1, right: 2`), et l'état
intermédiaire où un seul des deux sites avait été converti l'a tirée pour la même
raison. Son contrôle de bonne foi permanent est le point 3c du contrôle négatif
(`un appel réel doit être compté`), qui interdit le `0 == 0` symétrique — un
prédicat qui ne verrait aucun appel laisserait l'assertion rouge, pas verte, donc
la direction de panne est déjà la bonne.

---

## Retour au vert

Les trois inversions ont été restaurées à l'identique — le test temporaire du
cas 1 retiré, le site d'équipe du cas 3a rendu à son seul appel au helper, la
troisième entrée du cas 3b retirée.

```
$ cargo test -p mika-agent --lib mika2814
test agent_loop::tests::mika2814_sans_manager_le_helper_ne_touche_a_rien ... ok
test mcp::tests::mika2814_seuls_les_outils_testimony_sont_depublies ... ok
test mcp::tests::mika2814_une_definition_hors_map_est_publiee ... ok
test mcp::tests::mika2814_controle_une_table_sans_testimony_ressort_intacte ... ok
test mcp::tests::mika2814_le_routage_et_les_grades_ne_sont_pas_amputes ... ok
test agent_loop::tests::mika2814_un_seul_appel_publie_et_ensemence ... ok
test agent_loop::tests::mika2814_la_garde_du_helper_rougit_sur_une_publication_hors_chemin ... ok
test agent_loop::tests::mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 6595 filtered out; finished in 1.08s

$ cargo test -p mika-agent --lib "agent_loop::tests::mika"
test result: ok. 111 passed; 0 failed; 0 ignored; 0 measured; 6492 filtered out; finished in 1.98s
```

Le détail de la suite complète (workspace, clippy, `cargo fmt --check`, lints
dépôt, `scripts/verify-pipeline.sh origin/main`) est dans le corps de la PR.

## Refs

- Parent : mika#1798 (bake de l'invariant non-transit), F5
- Vol 1 de ce ticket : `todos/2814-injection-verification.md` (#2832)
- Résidu de revue qui ouvre ce vol : #2816, point 1 (« la garde d'appariement
  compare des chaînes »)
- Précédent : mika#1959 (décision D5), dont la garde est ici remplacée
- Doctrine de l'expiration des exemptions : mika#2309
- Doctrine : `crates/mika-agent/docs/non-transit-data-grade.md` § *Vigilance
  surface* et § *Layer 4*
- Reste ouvert sur mika#2814 (vol 3) : l'avertissement sur une clé inconnue
  d'une entrée `mcp.json`, et l'épinglage de `filter_available_required_tools`
