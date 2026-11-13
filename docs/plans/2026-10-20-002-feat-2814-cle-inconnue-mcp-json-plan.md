# mika#2814 vol 3 — une clé inconnue dans une entrée de serveur `mcp.json` ne passe plus en silence

> - **Ticket:** senara-solutions/mika#2814 (vol 3)
> - **Branch:** `feat/2814/non-publication-des-outils-mcp-testimony`
> - **Base:** `main` (`c5e64079`, qui contient `91a96cd2`)
> - **Plan du vol 1 (livré, PR #2832 → `5570ac1a`):** `docs/plans/2026-10-19-006-feat-2814-non-publication-outils-mcp-testimony-plan.md`
> - **Plan du vol 2 (livré, PR #2845 → `91a96cd2`):** `docs/plans/2026-10-20-001-feat-2814-helper-publie-et-ensemence-mcp-plan.md`

## Contexte — ce qui reste après deux vols

Le vol 1 a livré l'équivalent L2 côté MCP : `mcp::retain_publiables` ampute le
lot publié au modèle de tout outil dont le serveur déclare
`data_grade = "testimony"`. Le vol 2 a soudé publication et ensemencement L4 en
un seul appel (`agent_loop::publier_outils_mcp`) et remplacé la garde textuelle
de mika#1959 par une garde d'inventaire.

Deux résidus restaient, nommés par #2845 (« Tracked in ») :

1. **L'avertissement au chargement sur toute clé inconnue** d'une entrée de
   serveur `mcp.json`. C'est le résidu 2 de la revue de #2816, et c'est aussi la
   dernière ligne `Tracked in:` que porte la doctrine
   (`crates/mika-agent/docs/non-transit-data-grade.md` § *Vigilance surface*).
2. **L'épinglage de `filter_available_required_tools`**, dont l'incohérence est
   aujourd'hui inatteignable.

**Ce vol prend le résidu 1 seul.** Le résidu 2 part par une ligne `Tracked in:` —
voir § *Taille estimée* pour la mesure qui décide de la coupe, et § *Ce que ce
vol ne prend pas* pour ce qui lui est déjà cadré.

## Le fait, mesuré sur le code actuel

`McpServerConfig` ne porte pas `#[serde(deny_unknown_fields)]`, et c'est
délibéré : le chargement de `mcp.json` est fail-open (mika#1764), et un
`mcp.json` écrit pour un autre hôte MCP porte légitimement des clés que Mika
ignore. `serde` jette donc toute clé non reconnue **sans un octet**.

Appliqué au champ qui compte, cela donne :

```json
{ "mcpServers": { "journal-perso": { "transport": "stdio", "command": "…",
                                     "dataGrade": "testimony" } } }
```

`dataGrade` est jeté, `data_grade` prend son `#[serde(default)]`, et le serveur
publie ses outils comme `operational`. L'opérateur a écrit un refus ; le système
a lu une permission. Le `#[default]` de l'enum est porteur — on ne *gagne* pas le
privilège testimony par accident (`config.rs:57-92`) — mais il ne protège que
contre l'oubli, pas contre la faute de frappe : l'opérateur qui a tapé
`dataGrade` croit avoir fermé la porte, et rien ne le contredit.

C'est le mode de panne nommé par le plan de mika#1959 (« serveur testimony laissé
vide ») et assumé à l'époque, relevé par la revue de #2816, puis porté par deux
vols sans être fermé. Aucun chemin du dépôt ne le signale : `McpConfig::load`
(`config.rs:146`) et `McpConfig::load_operator_shell` (`config.rs:207`)
désérialisent directement depuis la chaîne, et `mika doctor` lit le fichier en
`serde_json::Value` pour y **compter** les serveurs (`doctor.rs:613-625`) sans
regarder les clés de leurs entrées.

## Décisions

### D1 — Un constructeur unique, et c'est lui qui audite

`McpConfig::depuis_json(content: &str, source: &Path) -> anyhow::Result<Self>`
parse en `serde_json::Value`, audite les entrées de serveur, puis désérialise par
`serde_json::from_value`. Les deux sites de production (`load:146`,
`load_operator_shell:207`) n'appellent plus `serde_json::from_str` : ils
appellent celui-là.

Un seul chemin parce que c'est la leçon que le vol 2 vient de payer : deux sites
qui doivent faire deux gestes appariés sont deux endroits où oublier le second.
Ici le geste est « parse **et** audite », et un `from_str` nu est exactement la
moitié qu'on oublierait.

Le coût est un `Value` intermédiaire par chargement — une fois par démarrage de
processus, sur un fichier de quelques kilo-octets. Il n'est pas discuté.

`source` n'entre que dans les lignes de journal : un opérateur qui lit un
avertissement sur `dataGrade` doit savoir **quel** fichier le porte, parce que la
cascade de résolution en désigne jusqu'à quatre (`resolve_operator_mcp_config_path`).

**Pas de garde d'appariement par source-scan ici, et la raison est une
asymétrie de conséquence, pas une économie.** Le vol 2 a payé cette garde parce
qu'un site publiant sans ensemencer **laisse partir l'appel distant** — F5
entier. Un site qui désérialiserait sans auditer ne perd qu'un avertissement :
la porte L4 et la non-publication L2 restent entières, et le mode de panne est
une faute de frappe non signalée, pas un transit de donnée. Deux fautes de
nature différente ne méritent pas le même appareil. Ce qui tient l'invariant ici
est que `depuis_json` est le chemin naturel et que les deux seuls sites de
production sont dans la même `impl`, à soixante lignes l'un de l'autre.

### D2 — La liste des clés connues est épinglée sur la sérialisation, pas retapée

`serde` n'expose pas les noms de champs d'une structure à l'exécution, donc
`KNOWN_SERVER_KEYS: &[&str]` est un littéral : les huit champs sérialisés de
`McpServerConfig` (`transport`, `command`, `args`, `env`, `url`, `headers`,
`enabled`, `data_grade`).

Deux spellings du même fait divergent. L'épinglage est donc un test de
**round-trip** : construire un `McpServerConfig` dont **tous** les `Option` sont
`None`, le passer à `serde_json::to_value`, et asserter que l'ensemble des clés
obtenues est exactement `KNOWN_SERVER_KEYS` — dans les **deux sens**, pour qu'un
champ ajouté et une entrée devenue fantôme rougissent chacun.

Le « tous les `Option` à `None` » n'est pas de la décoration : c'est ce qui
asservit la précondition dont dépend la viabilité du round-trip. Aucun champ ne
porte `skip_serializing_if`, donc un `None` sérialise en `null` et la clé
apparaît. Qu'on ajoute un `skip_serializing_if` à un champ et ce test rougit avec
le message qui dit pourquoi — plutôt que de laisser l'audit se mettre à signaler
ce champ comme inconnu sur tous les `mcp.json` de la flotte.

Sans cet épinglage, le mode de panne de l'audit est bruyant et dirigé contre
l'opérateur : un neuvième champ légitime ajouté à `McpServerConfig` ferait
avertir sur une clé parfaitement valide, et l'avertissement perdrait sa crédibilité
le jour où il a raison.

### D3 — Le périmètre est l'entrée de serveur, et le niveau supérieur est nommé hors périmètre

Le ticket dit « une clé inconnue dans l'entrée `mcp.json` d'un serveur ».
L'audit parcourt `mcpServers.<nom>` et rien d'autre.

Une faute au **niveau supérieur** (`"mcpServer"` au singulier) n'est pas
couverte, et pour une raison, pas par omission : elle rend une config **vide**,
donc zéro outil MCP publié — un mode de panne que l'opérateur voit à l'usage dès
le premier tour, là où `dataGrade` est muet par construction. Les deux ne
demandent pas le même remède. Si cela doit être fermé, c'est un ticket avec sa
propre mesure, pas une ligne glissée ici.

### D4 — La suggestion vient d'une normalisation, pas d'une distance d'édition

`cle_connue_equivalente(cle) -> Option<&'static str>` compare la clé inconnue aux
huit connues après normalisation : minuscules, et retrait des `_` et `-`. Donc
`dataGrade` → `datagrade` ← `data_grade` : correspondance trouvée.

Cela couvre la famille réelle — camelCase, kebab-case, SCREAMING_SNAKE — qui est
celle d'un `mcp.json` recopié depuis un autre hôte MCP ou tapé de mémoire. Pas de
Levenshtein : le dépôt n'en a aucune implémentation, et en ajouter une dépendance
pour huit clés serait disproportionné.

Une clé sans correspondance normalisée **n'obtient pas de suggestion** — et le
champ `suggestion` est alors **absent** de la ligne de journal, jamais `""`
(motif mika#2331 : « aucune suggestion » et « suggestion vide » ne sont pas le
même fait). Le message liste de toute façon les huit clés reconnues, donc
l'opérateur a l'information même sans suggestion.

### D5 — Un avertissement, jamais un refus — et un contrôle positif pour que le silence veuille dire quelque chose

**WARN `mcp_config_unknown_server_key`, une ligne par clé inconnue**, portant
`source`, `server`, `key`, `known_keys` et `suggestion` (absente quand il n'y en
a pas, D4). Jamais un refus, jamais une valeur par défaut changée, jamais une
clé réécrite : le chargement reste fail-open (mika#1764), et
`#[serde(deny_unknown_fields)]` reste écarté pour la raison déjà écrite dans le
doc-comment du champ `data_grade`.

**INFO `mcp_config_keys_audited`, une ligne par chargement, sur la branche
saine aussi**, portant `source`, `servers` et `unknown_keys`. Sans elle,
« aucune clé inconnue » et « le binaire servi précède ce correctif » rendent les
mêmes octets — la classe mika#2340, que le dépôt a déjà payée deux fois. Ici
`unknown_keys: 0` est un **vrai zéro** : le comptage a eu lieu et a rendu zéro,
donc le champ est présent et à zéro. C'est l'inverse exact du motif mika#2331
invoqué en D4, et les deux cohabitent dans la même fonction : un compte qui a eu
lieu se dit `0`, un fait qui n'existe pas s'omet.

Une ligne par démarrage de processus n'est pas du bruit, et c'est le seul moyen
de répondre « l'audit tourne-t-il ? » autrement qu'en provoquant une faute.

### D6 — Les deux sites couvrent tous les consommateurs, et c'est mesuré

Aucun appelant n'est à modifier au-delà des deux sites, parce que tous
transitent par eux :

| consommateur | chemin |
|---|---|
| `server/mod.rs:741` (démarrage mika-spirit) | `load_operator_shell_or_empty` → `load_operator_shell` |
| `validate.rs:123` (`mika validate`) | `load_operator_shell` |
| `cli/init.rs:191` (`init_base_for_agent`) | `load_operator_shell` |
| `cli/commands/mcp.rs:40,114,165,206` | `load_operator_shell` |

`save` et `save_operator_shell` sérialisent **depuis le type**, donc ne peuvent
pas produire de clé inconnue : rien à auditer sur le chemin d'écriture.
`migrate_from_agent_home_if_needed` copie des octets sans désérialiser — la
config migrée sera auditée au premier chargement, ce qui est le bon moment.

### D7 — `mika doctor` n'est pas touché, et pourquoi c'est un refus argumenté

`check_mcp` (`crates/mika-cli/src/commands/doctor.rs:602`) serait la surface
opérateur naturelle de cet audit. Il n'est pas touché, pour deux raisons
indépendantes : il lit l'**ancien** chemin per-agent (`agent_home/mcp.json`) et
non celui que résout la cascade operator-shell — un écart préexistant qui n'est
pas de ce ticket — et il vit dans un autre crate. Un audit branché sur le mauvais
fichier serait pire que pas d'audit : il dirait « aucune clé inconnue » sur un
fichier que personne ne charge. Nommé, pas élargi.

### D8 — Rien du vol 1 ni du vol 2 n'est touché

`retain_publiables`, `tool_routing`, `mcp_tool_data_grades`,
`publier_outils_mcp`, la map L4 et la garde
`mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence` restent
exactement où ils sont. Ce vol vit dans `mcp/config.rs` et nulle part ailleurs
dans le code de production — zéro intersection de fichier avec les deux vols
précédents, ce qui est aussi ce qui rend la coupe de § *Taille estimée* propre.

### D9 — La doctrine se met à jour dans le même commit

`crates/mika-agent/docs/non-transit-data-grade.md` § *Vigilance surface* porte
aujourd'hui le paragraphe « **What this still does NOT buy, and where it is
tracked** » avec son `Tracked in: senara-solutions/mika#2814 (vol 3)`. Ce vol le
réécrit en livré : ce que l'audit dit, ce qu'il ne refuse pas, et le fait qu'il
ne dispense ni L2 ni L4. C'est la dernière ligne `Tracked in:` de cette section.

Le fichier est **crate-local seul** — absent du `DOCS=()` de
`scripts/sync-agent-docs.sh` (vérifié : neuf entrées, aucune n'est celle-ci) —
donc aucun doc-sync n'est dû et le job CI `docs-sync` n'est pas concerné.

Le doc-comment du champ `data_grade` (`config.rs:78-90`) perd ses onze lignes de
« **Résidu connu, et non fermé ici** » : elles décrivent un futur devenu présent.
Il garde une ligne disant qu'une clé mal orthographiée avertit désormais au
chargement, et où.

## Quoi

### 1. `KNOWN_SERVER_KEYS` — `crates/mika-agent/src/mcp/config.rs`

Const au niveau module, les huit champs de D2, avec le doc-comment qui dit
pourquoi elle est un littéral (serde n'expose pas les noms de champs) et ce qui
l'empêche de dériver (le test du livrable 5).

### 2. `cle_connue_equivalente` — même fichier

Fonction libre et pure de D4. Doc-comment : la famille de fautes visée, et le
refus explicite d'une distance d'édition.

### 3. `auditer_cles_de_serveur` — même fichier

Fonction libre prenant le `serde_json::Value` déjà parsé et le chemin source ;
émet les lignes de D5 et ne rend rien (`()`), parce qu'un audit ne décide rien —
même forme et même raison qu'`AuthBoundaryLedger` et que le filet d'audit de
mika#2456. Un `mcpServers` absent, non-objet, ou une entrée non-objet : rien à
auditer, aucune ligne, l'erreur de forme sera rendue par `from_value` juste
après.

### 4. `McpConfig::depuis_json` + les deux sites

Le constructeur de D1, et les deux `serde_json::from_str` de `load` et
`load_operator_shell` remplacés par un appel à celui-là.

### 5. Le test de round-trip — module `tests` de `config.rs`

L'épinglage bidirectionnel de D2, avec le message d'erreur qui nomme le geste à
faire (ajouter le champ à la const, ou retirer l'entrée) et la précondition
`skip_serializing_if`.

### 6. Les tests de l'audit — même module, cinq cas

| cas | attendu |
|---|---|
| `"dataGrade": "testimony"` | une clé inconnue signalée, suggestion `data_grade`, et `data_grade` vaut toujours `Operational` — l'audit n'a rien réparé |
| une clé inconnue sans correspondance normalisée | signalée, **sans** champ `suggestion` |
| une entrée dont les huit clés sont connues | aucune clé inconnue ; la ligne de contrôle positif porte `unknown_keys: 0` |
| deux serveurs, une clé inconnue chacun | deux signalements, chacun nommant **son** serveur |
| un JSON malformé | l'erreur rendue par `depuis_json` est celle du parse, inchangée par l'audit |

L'assertion porte sur la valeur rendue par un audit testable plutôt que sur la
capture de `tracing` : `auditer_cles_de_serveur` journalise, et un helper
module-privé rend la liste des `(serveur, clé, suggestion)` qu'elle a vue — c'est
lui que les tests interrogent, et c'est la seule chose que la fonction de
journalisation consomme. Une assertion sur des lignes de log capturées épinglerait
la formulation, pas la propriété.

### 7. Doctrine — `crates/mika-agent/docs/non-transit-data-grade.md`

Le paragraphe de D9, et le doc-comment du champ `data_grade`.

## Ordre de commit

Un seul commit. L'ordre de rédaction est 1, 2, 3, 5 (le détecteur avant son
consommateur, pour que la const soit prouvée exacte avant qu'on bâtisse dessus),
puis 4, 6, 7.

## Ce que ce vol ne prend pas

**L'épinglage de `filter_available_required_tools`** (item 3 du vol 1, renommé
résidu 2 par le vol 2). Son périmètre est déjà cadré par la mesure faite ici,
pour que le vol qui le prend n'ait pas à la refaire :

- **L'incohérence, exactement.** La branche 1 de
  `filter_available_required_tools` (`agent_loop/mod.rs:9070-9072`) demande
  `mcp_manager.is_some_and(|m| m.is_mcp_tool(t))`, qui lit `tool_routing` —
  intact par construction du vol 1 (`mcp/mod.rs:105-112`). Un outil MCP
  testimony y répond donc **disponible**. C'est la branche 2 (`:9083`) qui le
  désarme, parce que `enabled_tool_names` est bâti sur `skill_tool_defs`
  (`:5793`), d'où `retain_publiables` l'a retiré. Résultat : l'outil est bien
  désarmé, mais sous `reason = "not_served"` — « ce tour ne sert pas cet outil »
  — alors que la cause est « son serveur déclare testimony ». Deux populations
  sous les mêmes octets, classe mika#2205.
- **Inatteignable aujourd'hui**, et c'est ce qui autorise le report : aucune
  skill bundlée ne déclare de `mcp__*` dans ses `required_tools`.
- **Ce que coûterait la branche nommée.** `filter_available_required_tools`
  reçoit déjà `mcp_manager: Option<&McpManager>`, donc un accesseur de grade sur
  le manager suffit et **aucune signature ne change** — ce qui épargne la mise à
  jour des ~15 sites d'appel du module de tests. La branche doit se poser
  **avant** la branche 2, sans quoi elle est inatteignable.
- **Le geste qui rendrait le report révocable** : une garde refusant qu'une skill
  bundlée déclare un `mcp__*` en `required_tools`, qui transforme « inatteignable
  aujourd'hui » en « inatteignable, et on le saura ».

Tracked in: senara-solutions/mika#2814 (vol 4 — épinglage de
`filter_available_required_tools` : la raison de désarmement nomme le ban
testimony au lieu de `not_served`)

## Fire-Disposition

Ce vol livre deux livrables dont le chemin de succès est « aucune violation
trouvée », et ils n'ont pas la même disposition. Les deux sont nommées, parce
qu'un seul mot pour deux natures laisserait le lecteur deviner.

**Le test de round-trip du livrable 5 — option (a), exception nommée en
allowlist, allowlist livrée VIDE.** C'est un détecteur de divergence
`KNOWN_SERVER_KEYS` ↔ champs sérialisés de `McpServerConfig`. Il atterrit
**armé** et **vert** : les huit champs sont mesurés sur le code actuel
(`config.rs:36-93`), et la précondition dont il dépend — aucun champ ne porte
`skip_serializing_if` — est vérifiée au même endroit. **Aucune violation
existante n'est à exempter**, donc la liste d'exceptions ne comporte aucune
entrée, et c'est une mesure, pas un espoir. Elle n'a pas non plus de ticket de
suivi à référencer, pour la même raison : il n'y a rien à faire. L'assertion est
bidirectionnelle, ce qui est sa propre propriété auto-nettoyante — une entrée de
la const qui cesse de correspondre à un champ rougit du même geste qu'un champ
qui cesse de correspondre à une entrée. Même forme que
`LECTEURS_NON_PUBLIANTS_MCP` au vol 2, et même raison : le geste qui rend une
exception inutile est celui qui la fait expirer (mika#2309).

**L'audit lui-même (livrables 2–4) — armé, et non bloquant par conception.** Son
chemin de succès est aussi « aucune clé inconnue », mais il ne garde pas la CI :
il parle à un opérateur, au chargement, et son verdict est un `WARN` qui ne
change aucune valeur et ne refuse aucun démarrage (D5). Ce n'est donc pas une
option de mika#1574 à choisir — ni allowlist, ni désarmement, ni halte — et
l'écrire comme tel serait une fausse déclaration. Il n'a aucune violation
préexistante à exempter : aucun `mcp.json` n'est committé dans ce dépôt
(`git ls-files | grep -i 'mcp.*json'` rend le seul
`crates/mika-agent/templates/skills/mcp/tools.json`, qui n'est pas une config de
serveurs), et toutes les configs que les tests chargent sont écrites par `save`,
donc sérialisées depuis le type et sans clé inconnue par construction (D6).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `KNOWN_SERVER_KEYS` + doc, et réécriture du doc-comment de `data_grade` | 30 |
| `cle_connue_equivalente` + doc | 25 |
| `auditer_cles_de_serveur` (WARN par clé, INFO de contrôle) + doc | 60 |
| `McpConfig::depuis_json` + câblage des deux sites | 35 |
| test de round-trip de sérialisation (le détecteur) | 35 |
| tests de l'audit (5 cas) | 70 |

Total estimé : 255 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Convention de comptage, déclarée parce qu'elle décide de la coupe.** Les
doc-comments des fichiers `.rs` sont **comptés** livrable par livrable, comme au
vol 2 et à l'inverse du vol 1. Le paragraphe de doctrine du livrable 7 n'est pas
compté : c'est de la prose sous `docs/`, et la règle compte les lignes de code.

**Calibrage mesuré du vol 2**, parce qu'une estimation qui ne se compare pas à un
réel n'est pas une mesure. `git show --numstat 91a96cd2`, hors `docs/` et hors
`todos/` : **456 lignes ajoutées** dans `agent_loop/mod.rs` (et 219 retirées)
pour **265 estimées** — facteur **1,72×**, en comptant déjà les doc-comments. Le
vol a tenu ses tours. Deux conséquences pour ce vol-ci : l'estimation ci-dessus
est de la même nature que celle du vol 2 (donc comparable à sa borne par la même
règle), et **la marge contre la borne n'est pas la marge réelle** — 255 estimées
projettent ≈ 440 lignes réelles, soit à peu près le volume que le vol 2 a livré
en tenant.

**Pourquoi un seul vol tient, et pourquoi la coupe tombe là.**

- **Un seul fichier de production, et il est petit.** `mcp/config.rs` fait 883
  lignes, contre les ~20 000 d'`agent_loop/mod.rs` où vivaient les vols 1 et 2.
  Le coût en tours de la navigation et de la relecture n'est pas du même ordre,
  et c'est ce qui fait tenir 255 ici alors que 265 était tendu là-bas.
- **Zéro appelant à mettre à jour**, et ce n'est pas une estimation : les sept
  consommateurs de `McpConfig` passent tous par `load` ou `load_operator_shell`,
  dont les signatures ne changent pas (D6, tableau mesuré). Le vol ajoute un
  constructeur, il n'en modifie aucun.
- **Zéro intersection de fichier avec le résidu renvoyé.** L'épinglage de
  `filter_available_required_tools` touche `agent_loop/mod.rs` et `mcp/mod.rs` —
  deux fichiers que ce vol n'ouvre pas. Chacun est donc mergeable seul, et la
  coupe ne crée aucune dépendance d'ordre.
- **Le résidu renvoyé est de nature différente, pas juste de l'excédent.** Celui
  d'ici ferme une porte réelle sur un chemin vivant (une faute de frappe muette
  dans un fichier que l'opérateur écrit à la main) ; celui de là-bas épingle un
  fait inatteignable. Les additionner aurait mis ~85 lignes estimées de plus,
  soit 340 — au-delà de la borne, et pour mêler deux gestes dont l'un n'a aucune
  urgence.

## Verification contract

| ce qui est prouvé | par quoi |
|---|---|
| `KNOWN_SERVER_KEYS` égale les champs réellement sérialisés, dans les deux sens | test de round-trip (livrable 5) |
| un `skip_serializing_if` ajouté à un champ rougit au lieu de rendre l'audit bruyant | même test, par la construction « tous les `Option` à `None` » |
| `dataGrade` est signalé, avec `data_grade` en suggestion | tests de l'audit, cas 1 |
| l'audit ne répare rien — `data_grade` vaut toujours `Operational` après le WARN | même cas, seconde assertion |
| « aucune suggestion » ne se confond pas avec une suggestion vide | tests de l'audit, cas 2 (champ `suggestion` absent) |
| le silence de la branche saine est attesté, pas supposé | tests de l'audit, cas 3 (`unknown_keys: 0` présent) |
| chaque signalement nomme son propre serveur | tests de l'audit, cas 4 |
| l'audit ne change pas l'erreur rendue sur un JSON malformé | tests de l'audit, cas 5 |
| le chargement reste fail-open | aucun chemin de `depuis_json` ne rend `Err` du fait d'une clé inconnue — cas 1 à 4 rendent tous `Ok` |
| rien des vols 1 et 2 n'a régressé | `tests/eval/test_mcp_testimony_non_publication_2814.rs` et `mika2814_publier_des_outils_mcp_passe_par_le_helper_qui_ensemence` passent sans modification de leur corps |
| les configs existantes ne deviennent pas bruyantes | `mcp_operator_shell_config.rs` et `test_mcp_secret_boundary_2281.rs` passent inchangés (ils écrivent par `save`, donc sans clé inconnue) |

## Definition of Done

1. `McpConfig::depuis_json` existe et est le **seul** chemin de désérialisation
   de production : `load` et `load_operator_shell` ne contiennent plus de
   `serde_json::from_str::<McpConfig>`.
2. Une clé inconnue dans une entrée de serveur produit un `WARN`
   `mcp_config_unknown_server_key` portant `source`, `server`, `key`,
   `known_keys`, et `suggestion` **seulement** quand il y en a une.
3. Chaque chargement produit un `INFO` `mcp_config_keys_audited` avec
   `unknown_keys`, y compris quand il vaut `0`.
4. Aucune clé inconnue ne provoque un refus, un démarrage avorté, ou une valeur
   par défaut différente de celle d'aujourd'hui.
5. `KNOWN_SERVER_KEYS` porte les huit champs, et le test de round-trip
   bidirectionnel passe avec un message d'erreur qui nomme le geste à faire.
6. Les cinq cas de test du livrable 6 passent.
7. Le doc-comment du champ `data_grade` ne contient plus le bloc « Résidu connu,
   et non fermé ici », et `non-transit-data-grade.md` § *Vigilance surface* dit
   la moitié « misspelled key » livrée.
8. `cargo test -p mika-agent`, `cargo test -p mika-cli`, `cargo clippy`,
   `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` passent.
9. Le corps de PR porte **Pas de `Closes`** : l'épinglage de
   `filter_available_required_tools` reste ouvert sur mika#2814, nommé par la
   ligne `Tracked in:` de § *Ce que ce vol ne prend pas*.

## Acceptance criteria

Dérivés du corps du ticket, de sa section *Résidus de la revue de #2816* point 2,
et des sections ci-dessus — le ticket ne porte pas de section
`## Acceptance criteria`.

1. **AC1 — Une clé inconnue dans une entrée de serveur `mcp.json` produit un
   avertissement au chargement.** C'est le résidu 2 nommé par la revue de #2816
   et par la doctrine, et `"dataGrade": "testimony"` est le cas qui doit mordre.
2. **AC2 — C'est un avertissement, pas un refus.** Le chargement reste
   fail-open (mika#1764) : aucune clé inconnue ne fait échouer `load`, ni changer
   une valeur, ni avorter un démarrage. `deny_unknown_fields` reste écarté, et le
   plan dit pourquoi.
3. **AC3 — La liste des clés reconnues ne peut pas dériver en silence.** Un champ
   ajouté ou retiré de `McpServerConfig` rougit un test au lieu de rendre l'audit
   faux — dans un sens comme dans l'autre.
4. **AC4 — L'audit est observable sur sa branche saine.** « Aucune clé inconnue »
   et « le binaire servi précède ce correctif » ne rendent pas les mêmes octets
   (classe mika#2340).
5. **AC5 — Une clé inconnue sans correspondance n'invente pas de suggestion.** Le
   champ est absent, pas vide (motif mika#2331), et le message reste utile par la
   liste des clés reconnues.
6. **AC6 — Aucune régression sur les vols 1 et 2.** `retain_publiables`,
   `tool_routing`, la map L4, `publier_outils_mcp` et sa garde sont intacts ;
   leurs tests passent sans modification de leur corps.
7. **AC7 — L'épinglage de `filter_available_required_tools` reste explicitement
   ouvert.** Le plan et le corps de PR le nomment avec son périmètre cadré et
   l'incohérence mesurée ; rien ne laisse croire que mika#2814 est clos.
