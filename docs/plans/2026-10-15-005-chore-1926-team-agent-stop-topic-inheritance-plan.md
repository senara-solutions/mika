# chore(prompt) — un enfant d'équipe hérite des `stop_topic_*` de l'agent à qui l'utilisateur a parlé

**Ticket :** senara-solutions/mika#1926
**Type :** chore (prompt / agent-core)
**Décision amont :** option **A — propager**, actée dans le corps du ticket par
l'orchestrateur le 2026-10-15. Ce plan l'implémente ; il ne la relitige pas.

---

## Contexte

mika#1813 (PR#1924) a livré le signal d'arrêt : sur refus de l'utilisateur, le
modèle persiste une préférence `stop_topic_<slug>`, et
`load_agent_context` (`crates/mika-agent/src/agent_loop/mod.rs:578`) la recharge
à **chaque** tour via `search_preferences(STOP_TOPIC_PREFIX)`, la filtre par
`prompt::filter_stop_topic_preferences`, et l'injecte en bloc
`<stopped-topics>` (`prompt.rs:1906` pour le prompt complet, `prompt.rs:2387`
pour le compact MikaModel, `prompt.rs:2529` pour le silencieux).

Les trois boucles passent par ce même `load_agent_context`. Pour un tour
d'équipe, `run_team_agent_inner_impl` écrit
`stopped_topics: &ctx.stopped_topics` (`agent_loop/mod.rs:7252`) — et `ctx`
est le contexte de **l'enfant** : ses propres lignes, sous son propre
`agent_id`. Un « arrête sur X » dit à l'agent conversationnel ne franchit donc
jamais la délégation.

Deux faits de substrat rendent le correctif court, et il faut les avoir en main
avant de lire la conception :

1. **Il n'y a qu'une base.** `init_resources` ouvre N+1 connexions sur
   `home::container_db_path(global_home)` (`teams/engine.rs:301`), une par
   membre plus celle du run ; `delegate_task` ouvre la même
   (`tools/delegate_task.rs:117`). Lire les préférences d'un autre agent n'est
   pas un accès inter-base : c'est le même fichier avec un autre `agent_id`.
2. **Le scope est déjà un paramètre.** `Database::search_preferences(agent_id,
   query)` prend l'`agent_id` en argument (`db.rs:3815`) et
   `AsyncDatabase::with_agent` rend un handle re-scopé partageant l'`Arc`
   interne (`async_db.rs:122`) — aucune connexion neuve, aucun thread neuf.

Et il existe un **précédent exact de propagation descendante** sur ce chemin :
`delegate_task` lit déjà `chat_id` dans la base de l'orchestrateur
(`ctx.db.get_customer_config("chat_id")`, `delegate_task.rs:176`) et le passe en
override explicite au délégué, avec le commentaire « the delegate's agent-scoped
`customer_config` won't have it ». La volonté de l'utilisateur est posée là où
l'utilisateur parle ; ce vol applique la même lecture à un second axe.

**Rayon de souffle, tel que le ticket le borne.** `message_sender` est `None`
pour les membres d'équipe (`engine.rs:1828`, `engine.rs:2284`) et
`NoopSender`/absent sur le chemin délégué, donc l'enfant ne parle pas à
l'utilisateur en direct. Mais son texte remonte dans l'agrégation de
l'orchestrateur — le deliverable, la synthèse, la réponse que l'utilisateur
lit. C'est ce canal-là que le correctif ferme.

---

## Décisions de grooming

### D1 — AC1 est satisfait en amont, et la rationale tient en une ligne

Un « arrête sur X » est la volonté de l'utilisateur, pas un état privé de
l'orchestrateur : puisque le texte d'un enfant remonte dans la réponse que
l'utilisateur lit, un enfant qui ignore le stop peut le lui re-servir. AC1 est
clos par la section « Décision (orchestrateur, 2026-10-15) » du corps du ticket.
Rien dans ce vol ne réexamine l'option B.

### D2 — Le « parent », c'est l'agent dont la base porte la conversation — pas le membre d'équipe nommé `orchestrator`

C'est la seule ambiguïté réelle du ticket, et s'en tromper livrerait un
mécanisme qui propage les mauvaises lignes.

Deux candidats portent le mot « orchestrateur » dans ce code :

| candidat | ce que c'est | exemple |
|---|---|---|
| `TeamDefinition.team.orchestrator` | un **membre de l'équipe**, celui qui décompose et synthétise | `"planner"` |
| `self.team_db.agent_id()` | l'**agent conversationnel** qui a lancé le run | `"mika"` |

Le raisonnement de D1 désigne le second sans ambiguïté : l'utilisateur ne parle
pas à `planner`, il parle à l'agent dont `team_db` porte l'`agent_id` — posé par
`run_team` depuis le handle de l'appelant (`teams/mod.rs:34`), et re-posé
identiquement sur le chemin de reprise
(`AsyncDatabase::new_with_agent(resume_db, &db.agent_id)`, `teams/mod.rs:102`).
C'est aussi l'agent dont `delegate_task` lit déjà le `chat_id`.

**Conséquence de nommage, délibérée :** le champ s'appelle `parent_agent_id` et
non `orchestrator_agent_id`. Un lecteur futur qui verrait
`orchestrator_agent_id` dans `teams/engine.rs` câblerait
`self.team.team.orchestrator` sans hésiter une seconde — le mot serait le piège.

### D3 — Union, l'enfant garde le sien sur collision, et le dédup est insensible à la casse

Le corps du ticket dit « en plus des leurs » : union, pas remplacement. Trois
sous-décisions en découlent, dont une non évidente.

- **L'enfant gagne les collisions.** La liste de l'enfant est déjà en force dans
  son propre tour ; le parent **ajoute** des axes. C'est la lecture littérale de
  « en plus des leurs », et elle évite qu'une propagation puisse *modifier* une
  donnée que l'enfant porte en propre.
- **Le dédup compare sans la casse.** `preferences.category` est déclaré
  `COLLATE NOCASE` avec `PRIMARY KEY (agent_id, category)`
  (`db/migrations.rs:699`). Deux agents peuvent donc porter
  `stop_topic_Web-Config` et `stop_topic_web-config` : ce sont la **même** clé au
  sens de la base, et un dédup sensible à la casse les rendrait toutes deux dans
  le bloc — un doublon dans le prompt, sur l'axe exact qu'on essaie de faire
  respecter. `str::eq_ignore_ascii_case` est l'équivalent exact de `COLLATE
  NOCASE`, qui est ASCII-only en SQLite.
- **Tri final par `category`.** `search_preferences` ordonne déjà
  (`ORDER BY category`, `db.rs:3820`), mais la fusion de deux listes ordonnées
  détruit cet ordre. Sans re-tri, le bloc rendu dépendrait de l'ordre
  d'insertion : prompt non déterministe, donc test non déterministe et cache de
  prompt cassé pour rien.

### D4 — L'échec de la lecture parent porte un nom d'événement propre

Le corps du ticket écrit : « Le chargement reste fail-open (WARN
`stop_topic_load_failed`), comme pour le tour parent. » La **politique** est
reprise à l'identique — aucune nouvelle voie d'échec, un tour ne meurt pas d'une
préférence illisible. Le **nom**, lui, est distinct :
`stop_topic_parent_load_failed`, avec le champ `parent_agent_id`.

La raison est déjà écrite dans ce fichier, à vingt lignes du site :
`read_optional_customer_config` porte le doc-comment « a distinct event name per
key rather than a shared one, for the reason mika#2205 had to write down […] two
populations under one name are not subtractable ». Sous un nom partagé, « la
base de l'enfant est tombée » et « la lecture du parent est tombée » rendraient
des octets identiques, alors que la première coupe la suppression propre de
l'enfant et la seconde seulement l'héritage. Divergence assumée par rapport à la
lettre du corps, conforme à son intention.

### D5 — Une ligne INFO positive, parce qu'une propagation rompue est muette

C'est la décision la plus importante du vol après D2.

Si l'orchestrateur porte deux stops et l'enfant zéro, un câblage rompu produit
un prompt enfant **sans bloc `<stopped-topics>`** — exactement les octets que
produit aussi « l'orchestrateur n'avait rien à propager ». Les deux états sont
indistinguables de l'extérieur (classe mika#2205), et c'est un réglage de
suppression : *un réglage qu'on ne peut pas observer n'est pas un réglage, c'est
un espoir* (mika#2293).

Une seule ligne, `event = "team_stop_topic_inheritance"`, `target:
"mika::otel"`, émise **sur toutes les branches, y compris quand tout est à
zéro** :

| champ | valeur |
|---|---|
| `agent` | l'enfant |
| `parent_agent_id` | le parent — **absent** quand aucun parent n'est déclaré |
| `own` | lignes propres à l'enfant |
| `inherited` | lignes ajoutées par le parent après dédup |
| `effective` | taille de la liste injectée |

`parent_agent_id` **absent** et non `""` : « aucun parent déclaré » n'est pas
« un parent dont le nom est vide » (motif mika#2331 — absent signifie que le fait
n'a pas eu lieu, jamais qu'il a rendu zéro). Pas de déduplication par agent à la
`report_resolved` : la valeur varie par tour selon la liste réelle, donc une
mémoïsation masquerait justement la transition qui intéresse.

### D6 — Un seul site de résolution, et `TeamAgentParams` ne porte qu'un fait

Deux formes étaient possibles. La retenue : `TeamAgentParams` gagne
`parent_agent_id: Option<&'a str>` — **l'identité du parent, un fait** — et
`run_team_agent_inner_impl` fait la lecture et la fusion — **la décision**. La
rejetée : un champ `stopped_topics` déjà résolu dans `TeamAgentParams`, que les
trois producteurs rempliraient. Trois producteurs, c'est trois façons de
calculer la même chose, dont deux qui divergeront ; et `delegate_task` n'a de
toute façon pas la liste en main (`ToolContext` ne porte pas les
`stopped_topics` du tour appelant).

Les trois sites producteurs ne font donc que **nommer leur parent** :

| site | valeur |
|---|---|
| `teams/engine.rs:1804` (spawn parallèle des spécialistes) | `Some(team_db.agent_id())` — `team_db` est déjà cloné par itération, l. 1759 ; zéro clone ajouté |
| `teams/engine.rs:2262` (`TeamEngine::run_agent`, decompose/review/deliver) | `Some(self.team_db.agent_id())` |
| `tools/delegate_task.rs:282` | `Some(current_agent_id)` — déjà en main à la l. 75 |

`Option` plutôt que `&str` : un appelant futur qui n'a pas de parent (un tour
d'équipe déclenché par aucune conversation) doit pouvoir le dire, et `None` est
alors lisible dans l'instrument de D5. Le `match` d'`Option` à l'intérieur de la
résolution est le seul endroit où l'absence se traduit en comportement.

### D7 — Court-circuit quand le parent EST l'enfant

`delegate_task` refuse déjà l'auto-délégation (`delegate_task.rs:76`), mais le
team engine n'a pas cette garde : l'agent conversationnel peut figurer parmi les
membres de son propre run (c'est la forme que `TeamRun.solo_absorption`
mesure). Dans ce cas la seconde lecture rendrait exactement les lignes déjà en
main. Court-circuit explicite — comparaison sur `parent_agent_id ==
db.agent_id()` — plutôt que de compter sur le dédup de D3 pour l'absorber : le
dédup rendrait le bon résultat, mais au prix d'une requête par tour et en
rendant `inherited = 0` indistinguable de « le parent n'avait rien ».

### D8 — Pas de plafond sur la taille du bloc, et c'est nommé plutôt que corrigé

`prompt.rs:1906` ne borne pas le nombre de lignes rendues, et la fusion peut au
pire doubler la liste de l'enfant. Aucun plafond n'est ajouté : le cardinal est
borné par ce que l'utilisateur a réellement refusé (ordre de grandeur : quelques
unités), et un plafond posé ici serait un *refus silencieux* d'une suppression
que l'utilisateur a demandée — strictement pire que le coût en octets qu'il
économise. Si la population devient un jour un problème, c'est le rendu de
`prompt.rs` qu'il faut borner, pour les trois boucles à la fois, pas la fusion
d'une seule.

---

## Conception

### L1 — `TeamAgentParams::parent_agent_id` (`crates/mika-agent/src/agent_loop/mod.rs`, ~l. 7138)

Un champ, `Option<&'a str>`, avec un doc-comment qui porte D2 : que c'est
l'agent **conversationnel** (`team_db.agent_id()` / `ctx.db.agent_id()`), pas
`TeamDefinition.team.orchestrator`, et que le mot « orchestrateur » du ticket
désigne le premier.

### L2 — `resolve_team_stop_topics` (`agent_loop/mod.rs`, à côté de `load_agent_context`)

```rust
async fn resolve_team_stop_topics(
    db: &AsyncDatabase,
    agent_name: &str,
    parent_agent_id: Option<&str>,
    own: Vec<crate::db::Preference>,
) -> Vec<crate::db::Preference>
```

Corps, dans cet ordre :

1. **Court-circuit (D7)** — `None`, ou un parent égal à `db.agent_id()` : émettre
   la ligne de D5 (`parent_agent_id` absent dans le premier cas, présent avec
   `inherited = 0` dans le second) et rendre `own`.
2. **Lecture re-scopée** — `db.with_agent(parent).search_preferences(prompt::STOP_TOPIC_PREFIX)`,
   puis `prompt::filter_stop_topic_preferences` sur le résultat. Le filtre n'est
   pas optionnel : `search_preferences` fait un `LIKE '%…%'` sur `category` **et**
   `value`, et le `_` de `stop_topic_` est un joker SQLite — les deux fuites que
   le doc-comment du filtre détaille (`prompt.rs:355`).
3. **Fail-open (D4)** — bras `Err` : `warn!(event = "stop_topic_parent_load_failed",
   parent_agent_id, error)`, puis `own` rendu tel quel.
4. **Fusion (D3)** — partir de `own`, étendre des lignes du parent dont la
   `category` n'est présente dans `own` sous aucune casse
   (`eq_ignore_ascii_case`), puis `sort_by` sur `category` en comparaison
   insensible à la casse pour retrouver l'ordre de `search_preferences`.
5. **Instrument (D5)** — la ligne INFO, avec les cinq champs.

Fonction unique, un seul appelant. Elle n'est pas découpée en un helper pur
`merge` + un helper d'I/O : la fusion sans la lecture est triviale au point que
son test ne dirait rien, et c'est la paire *lecture re-scopée + dédup NOCASE*
qui porte le risque réel (D3).

### L3 — Le câblage (`agent_loop/mod.rs::run_team_agent_inner_impl`, ~l. 7222–7252)

Après `load_agent_context` (qui a déjà rempli `ctx.stopped_topics`), avant le
littéral `PromptContext` :

```rust
let stopped_topics = resolve_team_stop_topics(
    params.db,
    params.agent_name,
    params.parent_agent_id,
    ctx.stopped_topics,
).await;
```

puis `stopped_topics: &stopped_topics` à la place de `&ctx.stopped_topics`.
`ctx.stopped_topics` est **déplacé** dans l'appel, donc rien ne peut lire
l'ancienne liste après coup : le compilateur tient l'invariant à notre place.

Le commentaire déjà en place à la l. 7257 — « the team child reads its OWN
agent's declaration, the same rule mika#1926 settled for `stopped_topics` » —
est **amendé** : il cite une règle qui n'est plus celle de `stopped_topics`.
`tenant_language` reste délibérément sur la lecture propre de l'enfant (c'est un
registre de langue, pas une volonté de l'utilisateur sur un sujet) ; le
commentaire doit dire la dissymétrie au lieu de l'adosser à mika#1926.

### L4 — Les trois producteurs

`teams/engine.rs:1804`, `teams/engine.rs:2262`, `tools/delegate_task.rs:282` :
une ligne chacun, par la table de D6. Un commentaire court sur le site de
`delegate_task`, qui renvoie au `chat_id` lu dix lignes plus haut depuis la même
base de l'orchestrateur — c'est le précédent qui rend la ligne évidente.

### L5 — Documentation (hors comptage)

`crates/mika-agent/CLAUDE.md` : le paragraphe **Team-agent inheritance
(mika#1926)** dit aujourd'hui « the decision […] is open under mika#1926 ». Il
est remplacé par la sémantique arrêtée : union avec les lignes du parent
conversationnel, l'enfant gagnant les collisions, dédup NOCASE, fail-open sous
`stop_topic_parent_load_failed`, instrument `team_stop_topic_inheritance`, et la
dissymétrie assumée avec `tenant_language`. Ce paragraphe est AC4.

---

## Étapes d'implémentation

1. L1 — le champ et son doc-comment.
2. L4 — les trois producteurs (`cargo check` doit passer dès ici : le champ
   n'est pas `Option`-par-défaut, les trois sites sont obligatoires).
3. L2 — la résolution.
4. L3 — le câblage et l'amendement du commentaire de la l. 7257.
5. Les tests W1–W4.
6. L5 — `CLAUDE.md`.
7. `cargo fmt`, `cargo clippy`, `cargo test -p mika-agent`, puis
   `scripts/verify-pipeline.sh origin/main`.

Plancher de commit : les étapes 1 à 4 forment le plus petit ensemble qui
compile ; les tests et la doc suivent dans le même commit.

---

## Contrat de vérification

Un fixture partagé porte les quatre cas :
`async fn team_turn_system_prompt(parent: Option<&str>) -> String` monte une
`Database::open_in_memory`, enregistre les deux agents (la FK
`preferences.agent_id → agents(id)` l'exige), crée la session d'équipe comme le
fait `engine.rs:1789`, pose les préférences, puis appelle `run_team_agent` avec
un `MockLlmProvider` et rend
`mock.captured_requests()[0].system.unwrap()`.

Deux propriétés de ce fixture sont ce qui rend le contrat sérieux :

- **Il traverse le site de production.** Le prompt observé est celui que
  `run_team_agent_inner_impl` a réellement remis au provider, pas un
  `PromptContext` reconstruit à la main. Un test qui enchaînerait
  base → `resolve_team_stop_topics` → `build_system_prompt` à la main serait vert
  même si le littéral de la l. 7252 n'avait jamais été modifié — c'est la classe
  de défaut exacte que ce ticket corrige.
- **La base est partagée par clone, et c'est fidèle.** En production les deux
  agents ont deux connexions sur **un** fichier ; en test, `with_agent` rend un
  handle re-scopé sur le même `Arc`. Deux `open_in_memory` distincts seraient
  deux bases et le test serait vert pour la mauvaise raison (il ne mesurerait
  plus rien). À écrire dans le doc-comment du fixture.

### W1 — le stop du parent atteint le prompt de l'enfant

`mika` porte `stop_topic_web-config`, l'enfant n'a rien.
`team_turn_system_prompt(Some("mika"))` contient `<stopped-topics>` et
`stop_topic_web-config`.

### W2 — le contrôle négatif porteur

Même base, `parent: None`. Le prompt ne contient **ni** `<stopped-topics>` **ni**
le slug. Sans ce cas, « la propagation marche » serait indistinguable de « le
bloc est rendu de toute façon », et W1 passerait sur un builder qui injecte tout
ce qui traîne dans la base.

### W3 — l'union, le dédup insensible à la casse, et l'ordre

L'enfant porte `stop_topic_Web-Config` (valeur `"enfant"`) et
`stop_topic_zebra` ; le parent porte `stop_topic_web-config` (valeur `"parent"`)
et `stop_topic_alpha`. Le prompt rendu doit porter **trois** lignes, dans l'ordre
`alpha`, `Web-Config`, `zebra`, et la ligne de collision doit porter `"enfant"`
(D3). Trois assertions en une : union, l'enfant gagne, et le tri est
déterministe. Ce test est la raison d'être de D3 — sans le `eq_ignore_ascii_case`
il rend quatre lignes.

### W4 — parent égal à l'enfant

`team_turn_system_prompt(Some("<l'enfant lui-même>"))` avec une ligne propre :
une seule ligne rendue, aucun doublon (D7).

### W5 — la suite complète

`cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`, puis
`scripts/verify-pipeline.sh origin/main`. Aucun test existant n'est attendu
rouge : les trois producteurs sont les seuls appelants de `TeamAgentParams`, et
le seul fixture de test qui en construit un (`engine_with_mock`,
`teams/engine.rs:3030`) passe par `TeamEngine`, donc par L4.

### Gate Fire-Disposition (mika#2306) — N/A, et la raison plutôt que le silence

Ce plan ne livre **aucun détecteur**. W1–W4 sont des assertions de comportement
sur un chemin neuf : pas de balayage d'arbre, aucune population de violations
existantes à exempter, et aucun chemin de succès « aucune violation trouvée » —
leur chemin de succès est « le slug du parent est dans le prompt de l'enfant ».
La section `## Fire-Disposition` n'est donc pas requise et n'est pas inventée.
Si la lecture littérale était retenue (le mot « test » figure dans la liste de
la règle), la disposition serait **(a), avec allowlist vide** : les trois sites
producteurs sont modifiés dans cette PR, donc aucun code préexistant ne peut
violer la propagation le jour de l'atterrissage.

**Une garde structurelle a été envisagée et écartée, avec sa raison.** Un scan
refusant `stopped_topics: &ctx.stopped_topics` sous `run_team_agent_inner_impl`
viserait le trou classique « le site lit le bon champ et ne consulte jamais la
base ». Ce trou est fermé autrement, et mieux : W1 observe le prompt
effectivement remis au provider, donc il rougit sur ce défaut-là. Ajouter le
scan serait de la machinerie par-dessus une garde comportementale qui couvre
déjà la classe — doctrine mika#2201.

---

## Risques nommés

- **R1 — se tromper de parent (D2).** Le risque de régression le plus réaliste
  du vol : un relecteur qui « corrige »
  `Some(self.team_db.agent_id())` en `Some(&self.team.team.orchestrator)` par
  cohérence de vocabulaire. Le prompt resterait rempli — avec les stops de
  `planner`, qui n'en a jamais — donc **tous les tests resteraient verts** : W1
  pose ses lignes sous l'`agent_id` de `team_db`, pas sous un nom de rôle. C'est
  le doc-comment de L1 qui porte la garde, et c'est pour cela qu'il doit nommer
  les deux candidats.
- **R2 — le bloc de l'enfant grossit du double (D8).** Accepté, cardinal borné
  par les refus réels de l'utilisateur, et un plafond ici serait un refus
  silencieux d'une suppression demandée.
- **R3 — l'instrument de D5 n'est pas en service au merge.** La ligne INFO vit
  dans le binaire ; sans `make deploy`, la sonde post-déploiement ci-dessous
  rend vide. Absence de la ligne = binaire antérieur au correctif, **jamais**
  « rien à propager » (classe mika#2340). À nommer dans le corps de PR.
- **R4 — un enfant déjà hors conversation.** Les tours d'équipe n'ont pas de
  `message_sender` : ce vol ne change donc rien à ce que l'utilisateur **reçoit
  directement**, seulement à ce que l'agrégation de l'orchestrateur peut
  contenir. Annoncer davantage dans le corps de PR serait surévaluer l'effet.

---

## Hors périmètre

- **L'option B** (isolation explicite) — tranchée en amont, D1.
- **`tenant_language`** reste sur la lecture propre de l'enfant (L3) : un
  registre de langue n'est pas une volonté de l'utilisateur sur un sujet.
  Seul le commentaire qui l'adossait à mika#1926 est amendé.
- **Un plafond de rendu du bloc `<stopped-topics>`** (D8) — si un jour
  nécessaire, c'est `prompt.rs` pour les trois boucles, pas la fusion.
- **La propagation sur deux niveaux** (petit-enfant d'une délégation en
  cascade) : `delegate_task` interdit l'auto-délégation et ne construit pas de
  chaîne de parents ; un enfant qui délègue à son tour hérite du parent qu'on
  lui nomme, et rien de plus. Aucun chemin actuel ne produit ce cas.
- **Le chemin silencieux** (`SilentPromptContext`) : il n'a pas de notion de
  parent, et aucun tour silencieux n'est un tour d'équipe.

---

## Mesure post-déploiement

1. **La sonde.** Après `make deploy`, sur un run d'équipe ou une délégation
   lancée par un agent portant au moins un `stop_topic_*` :
   `grep 'team_stop_topic_inheritance' "$MIKA_SPIRIT_LOG_FILE"` doit rendre une
   ligne par tour d'enfant, avec `parent_agent_id` présent et `inherited ≥ 1`.
   Lecture exacte : *l'héritage a été calculé et vaut tant de lignes.*
2. **Ce qu'elle ne mesure pas.** Elle n'atteste pas que le modèle a **obéi** au
   bloc. L'obéissance au contrat de suppression relève de la calibration
   (mika#1190), hors de ce vol comme elle l'était de mika#1813.
3. **Halte 1 — aucune ligne alors que des runs d'équipe tournent.** Vérifier
   d'abord le SHA servi (`cat ~/.mika/skills/.manifest-writer` ne dit rien ici,
   c'est la bibliothèque de skills ; utiliser la version du binaire) : R3 avant
   toute autre hypothèse.
4. **Halte 2 — `inherited = 0` avec un `parent_agent_id` présent et des stops
   connus côté parent.** Chercher `stop_topic_parent_load_failed` dans le même
   log : si présent, c'est la lecture qui tombe (D4) ; si absent, c'est D2 —
   le `parent_agent_id` de la ligne n'est pas l'agent où l'utilisateur a parlé.

---

## Definition of Done

- `TeamAgentParams` porte `parent_agent_id`, documenté par D2, et les trois
  producteurs le remplissent.
- `run_team_agent_inner_impl` injecte la liste résolue, et plus
  `&ctx.stopped_topics`.
- L'échec de la lecture parent est fail-open sous
  `stop_topic_parent_load_failed`.
- `team_stop_topic_inheritance` est émis sur toutes les branches, avec
  `parent_agent_id` absent quand aucun parent n'est déclaré.
- W1–W4 passent ; `cargo test -p mika-agent`, `cargo clippy`,
  `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` sont verts.
- `crates/mika-agent/CLAUDE.md` nomme la sémantique d'équipe (AC4).
- Le corps de PR porte R1 et R3.

---

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#1926 :

1. A decision (A or B) is chosen with a one-line rationale.
2. The chosen decision is implemented (either the load-from-parent code OR the
   explicit `&[]` + comment).
3. A test in `prompt.rs` or `agent_loop/tests/` asserts the chosen behavior
   end-to-end.
4. `crates/mika-agent/CLAUDE.md` "Stop-signal convention" paragraph names the
   team-agent semantics explicitly so a future engineer does not have to infer.

**Correspondance :** AC1 → D1 (clos en amont par le corps du ticket). AC2 → L1,
L2, L3, L4. AC3 → W1–W4, dans le `#[cfg(test)] mod tests` de
`crates/mika-agent/src/agent_loop/mod.rs` — le dossier `agent_loop/tests/` que
l'AC nomme n'existe pas (les tests de ce module sont inline depuis la l. 10467),
et W1 traverse `run_team_agent_inner_impl` en observant le prompt réellement
remis au provider, ce qui est le sens « end-to-end » de l'AC. AC4 → L5.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/mod.rs` — L1 champ + doc | 14 |
| `crates/mika-agent/src/agent_loop/mod.rs` — L2 `resolve_team_stop_topics` | 58 |
| `crates/mika-agent/src/agent_loop/mod.rs` — L3 câblage + amendement du commentaire | 16 |
| `crates/mika-agent/src/teams/engine.rs` — L4, deux sites | 10 |
| `crates/mika-agent/src/tools/delegate_task.rs` — L4, un site | 8 |
| tests (`agent_loop/mod.rs`, `mod tests`) — fixture partagé | 48 |
| tests — W1, W2 | 30 |
| tests — W3, W4 | 48 |

Total estimé : 232 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Un seul vol, et voici pourquoi il tient.** 232 < 243 et 232 < 1000 : les deux
bornes passent. Le vol est petit parce que le substrat porte déjà les trois
pièces coûteuses — une base unique par conteneur, un `search_preferences`
paramétré par `agent_id`, et un `with_agent` qui re-scope sans ouvrir de
connexion — de sorte que la production se réduit à un champ, une fonction et
quatre lignes de câblage. La moitié du total est du test, et c'est le bon
partage pour un ticket dont le livrable est une **sémantique**.

**La marge est mince (11 lignes), et voici la soupape si elle est franchie.**
W3 et W4 fusionnent en un seul test piloté par table sur le fixture partagé
(~30 lignes au lieu de 48, soit un total de ~214). Cette compression est
nommée ici pour qu'elle ne soit pas improvisée en vol ; elle ne retire aucune
assertion, seulement l'échafaudage de l'un des deux cas.
