# La TUI passe par mika-spirit (mika#1727)

Déplacé mot pour mot hors de `crates/mika-cli/CLAUDE.md` le 2026-11-25. Ce fichier est chargé en entier dans le prompt d'un pilote dès que le pilote lit quoi que ce soit sous `crates/mika-cli/`, et `crates/mika-common/tests/claude_md_size.rs` plafonne chaque `CLAUDE.md` du dépôt à 100 000 octets ; `main` l'avait dépassé (100 387 octets). Les titres gardent leur niveau d'origine, et les renvois « voir plus haut » ou « voir plus bas » se lisent dans l'ordre de `crates/mika-cli/CLAUDE.md` § TUI Features.

### Un tour de la TUI exige mika-spirit (mika#1727, vol V1 sur 4)

Depuis ce vol, un message tapé dans `mika` / `mika chat` **part sur le fil** :
`chat::spawn_agent_worker`, bras `AgentRequest::Message`, envoie le tour par
`commands/chat_turn.rs::send_turn_to_spirit` vers `{spirit_url}/a2a/{agent}`, et
rend ce que le serveur a produit. La boucle, la session de boucle, les outils,
les permissions et le dispatch LLM sont chez mika-spirit — qui les possédait
déjà pour `mika ask` depuis mika#1881. **Sans daemon, la TUI n'a plus de mode
autonome** : le démarrage le dit (voir plus bas) et le tour échoue en nommant
l'endpoint, jamais en rendant une réponse vide (mika#2270).

`chat_turn.rs` est l'**unique** site d'envoi, et c'est structurel :
`mika1727_v1_le_tour_de_la_tui_ne_fait_plus_tourner_la_boucle` balaie la moitié
production du crate et refuse `agent::run_agent(` comme `AgentParams`
**partout**, allowlist (`V1_ALLOWED`) livrée vide et assertie vide depuis le vol
V3a-2b. Quand elle rougit, le remède est de router le nouvel appelant par
l'envoi existant, jamais d'y ajouter une ligne.

**Il ne reste aucun appel local de la boucle dans ce crate (vol V3a-2b).** Le
bras `CallbackResult` était le dernier ; il passe désormais par
`chat_turn::livrer_tour_de_callback`, qui est le seul site à poser
`mika.callback_turn` à `true` — la porte A2A le lit en booléen strict depuis le
vol V3a-1, et le client sait le poser depuis V3a-2a, par
`remote_ask::build_send_params`. **Le poll des callbacks de `tui/app.rs`, lui,
reste** : c'est ce qui livre le résultat au bras, et il part plus tard. Le
`TaskEngine` est déjà parti (vol V2a, voir plus bas), et l'exception
d'allowlist avec le bras. Le périmètre des vols est dans le corps de mika#1727.

Ce que le tour de callback **perd** en passant sur le fil est dit sur
`livrer_tour_de_callback` : pas de bloc `thinking:` (un `Task` A2A n'en porte
pas de part), et aucun avis d'inertie — personne ne *tape* un tour de callback.
Ce qu'il **garde** : sur toute erreur la tâche revient à son statut d'origine,
pour que le poll suivant la reprenne. Une erreur peut arriver **après** que
spirit a fait le tour, donc la livraison suivante le rejoue : la règle préfère
un tour en double à un résultat perdu.

**Trois entrées de la TUI n'atteignent plus le tour, et se disent à voix haute.**
`chat_turn::inertia_notice` rend un avis **à chaque tour qui les exerce** — pas
une fois par session — et chacun nomme le geste qui l'éteint ; c'est la forme de
`cli_skill_flag_inert` (mika#1883), pour la même raison : un réglage qui ne fait
plus ce qu'il annonce doit se voir chaque fois qu'on s'en sert.

| entrée | ce qui se passe | cause |
|---|---|---|
| `/think <budget>` | le budget ne voyage pas | aucune clé de fil ; le serveur ne lit pas `thinking_level` |
| bloc `thinking:` en réponse | il n'apparaît plus | un `Task` A2A ne porte pas de part de pensée |
| image collée (Ctrl+V) | elle est abandonnée | `remote_ask::build_send_params` ne bâtit que des parts texte |

L'avis est émis dans le **worker**, pas à la saisie : c'est l'unique point où
atterrissent les **deux** expéditeurs d'`AgentRequest::Message` (`tui/app.rs` et
le coup unique de `/think` dans `tui/commands/handlers.rs`). Il voyage par
`AgentResponse::Notice`, rendu en `ChatRole::System` **sans toucher au statut** :
un avis n'est pas un échec et ne doit pas éteindre le spinner du tour qu'il
annote. L'avis des images nomme ce qui est tombé — types MIME dédupliqués et
charge base64 transportée pour rien — et c'est aussi ce qui garde un
consommateur à `ImageAttachment::media_type` et `base64_data`, que le tour texte
seul n'envoie plus.

**Le démarrage dit un spirit muet avant le premier tour.** `chat::run` lit
`dashboard::query_dashboard_status()` et non `is_dashboard_running()` : la même
sonde répond désormais à deux questions, et le `bool` les écrasait — un daemon à
terre y était indiscernable d'un tableau de bord désactivé. Un `None` pousse un
message `System` qui nomme l'endpoint. **Il ne nomme plus le jeton depuis le vol
V2d-4a** (voir plus bas) : ce message disait les **deux** causes possibles, parce
que la sonde rend la même absence pour un daemon injoignable et pour un shell
sans `MIKA_INTERNAL_TOKEN`, et en désigner une seule aurait été une fausse
provenance (motif mika#2331). Depuis que l'écran n'existe que si la session s'est
ouverte sur spirit, le jeton **a été lu** à ce point du démarrage : la seule
cause restante est un daemon qui s'est tu entre l'ouverture et la sonde. Le
chemin `--team` garde `is_dashboard_running()` — son tour ne passe pas par ce
bras, et la sonde lui reste après le vol V4-b2 (défaut 42).

### La TUI ne possède plus de moteur de tâches (mika#1727, vol V2a sur 4)

`commands/chat.rs` construisait un `TaskDispatcher`, un `TaskEngine`, les deux
paires de récurrentes intégrées (`heartbeat` et `reflection`),
`startup_recovery`, la purge des tâches anciennes et une boucle de tick. Tout
cela est parti : **le daemon (`mika-agent/src/server/mod.rs`) est désormais le
seul enregistreur des récurrentes d'un agent.** Le refus est structurel —
`mika1727_v2a_la_tui_ne_construit_plus_de_moteur_de_taches` (module `tests` de
`commands/chat_turn.rs`) balaie la moitié production du crate et refuse
`TaskEngine::new(`, `TaskDispatcher {`, `spawn_tick_loop(` et
`startup_recovery(` **partout**, allowlist livrée vide et pinnée vide. Le scan
porte sur le geste de construction et jamais sur `task_engine::` :
`commands/tasks.rs` importe légitimement neuf utilitaires de ce module pour lire
et écrire des tâches par la base. Quand il rougit, le remède est un endpoint de
mika-spirit, jamais une entrée d'allowlist.

**Effet de lecture, à retenir avant de diagnostiquer `auto_pull`.** Jusqu'à ce
vol, le moteur de la TUI disputait aux daemons les récurrentes de son agent
(mika#2351) ; le 2026-11-08 c'était lui qui exécutait `auto_pull` pour mika-dev.
La course est éteinte par construction, donc **l'absence de lignes `auto_pull_*`
dans le journal CLI par agent (`~/.mika/agents/<nom>/logs/mika.log.YYYY-MM-DD`)
est le régime nominal après ce merge, et non une absence d'`auto_pull`** (classe
mika#2340). Ces lignes se lisent côté daemon, dans
`$MIKA_SPIRIT_LOG_FILE` (`mika logs paths --agent <a>`), et la récurrente
elle-même dans `mika tasks list`.

**Les deux entrées de recensement de `canonical_tokens.rs` ont bougé avec le
code.** `HEARTBEAT_GATE_REGISTRATION_SITES` (mika#2847) et
`REFLECTION_GATE_REGISTRATION_SITES` (mika#2850) perdent
`crates/mika-cli/src/commands/chat.rs` — nécessairement dans le même commit,
leurs boucles `seen == len` visant sinon un nom mort. L'entrée n'est **pas**
passée dans `*_READER_ALLOWED` : le retrait **durcit** les deux scans, puisque
toute réapparition d'une lecture de porte sous `crates/mika-cli/` est désormais
un `strangers`.

### Le rembobinage de la TUI est appliqué par mika-spirit (mika#1727, vol V2c-1b)

`/undo`, `/rewind <n>` et `/rewind to <id>` ne touchent plus la base dans le
processus de la TUI. `tui/commands/handlers.rs::rewind_par_spirit` traverse les
**trois portes** de `commands/rewind_remote.rs` — `resolve` (l'ancrage :
session visée et `after_message_id`), `preview` (ce qui serait défait), puis
`execute` — toutes sur `{spirit_url}/api/v1/rewind/*`. `/rewind to <id>` saute
`resolve` : l'ancrage est déjà donné.

**Le refus structurel.**
`mika1727_v2c1b_la_tui_ne_rembobine_plus_la_base_en_local` (module `tests` de
`handlers.rs`) balaie la moitié production du fichier par `ProductionScanner` et
refuse `find_recent_exchanges(`, `preview_rewind(` et `execute_rewind(`, chacun
avec le nom de sa porte de remplacement. Son **allowlist est livrée vide et
assertie vide** : quand elle rougit, le remède est de router l'appelant par la
porte nommée, jamais d'y ajouter une ligne. Elle ne refuse ni `rewind::` ni
`format_preview(` — le formateur reste légitime, et `app.db` reste lu pour le
rechargement.

**`MIKA_INTERNAL_TOKEN` est exigé, et le refus tombe avant tout appel.**
`jeton_interne` lit la variable et refuse en la nommant : **`MIKA_DASHBOARD_TOKEN`
ne suffit pas** — les routes rewind ne l'acceptent pas. Le refus est posé avant
`resolve`, parce que l'appel qui suivrait n'est pas sans effet : un 401 tomberait
sur la première des trois portes et nommerait le transport là où la cause est la
configuration du shell.

### Le démarrage de la TUI passe par mika-spirit (mika#1727, vol V2d-4a)

`mika` / `mika chat` **ne crée plus sa session en local**.
`chat::ouvrir_session_par_spirit` remplace les deux appels de
`spawn_agent_worker` (`get_or_create_canonical_session` pour un singleton,
`create_session` sinon), dont l'échec ne donnait qu'un `tracing::warn!` : la TUI
démarrait sans session et la panne se disait au premier tour, là où elle ne se
lit plus.

**Ce que le démarrage exige.** `MIKA_INTERNAL_TOKEN`, lu avant tout appel, sur
les **deux** chemins (avec et sans `--session-id`) — le premier tour part sur le
fil de toute façon. Le refus nomme la variable et dit que
`MIKA_DASHBOARD_TOKEN` **ne suffit pas** : `dashboard::auth_token()` l'accepte
pour la sonde, les routes de session non. C'est l'angle mort que ce vol ferme —
un shell qui ne portait que le jeton du tableau de bord passait la sonde, puis
prenait un 401 à chaque tour. Sans jeton, ou si `ouvrir` échoue (401, 404,
injoignable), le `?` de `run` sort en non-zéro **avant** le hook de panique et
`enable_raw_mode` : l'écran ne s'affiche jamais. Au changement d'agent et à
`/restart`, les chemins d'erreur existants poussent le message dans la
conversation sans quitter la TUI.

**D'où vient `is_singleton_session`.** Du **fil**, et non de l'identité locale :
la route rend la session canonique de l'agent au lieu d'en créer une, donc le
serveur est la seule autorité sur ce fait. `resolve_canonical_session_id` ne sert
plus qu'au bras de reprise.

**Ce que le démarrage refuse.** Sous `--session-id`, une session que la base
locale ne connaît pas — la route ne prend aucun identifiant dans son corps, donc
elle ne sait pas en créer une à identifiant choisi, et le refus nomme
l'identifiant. Une session étrangère reste refusée au texte inchangé, et une base
illisible ne se lit plus comme « session inconnue » (cette erreur était avalée).
Une session connue et à l'agent est reprise **sans appeler `ouvrir`**.

**Le refus structurel.**
`mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local` (module `tests` en fin
de `commands/chat.rs`) balaie la moitié production du fichier par
`ProductionScanner` et refuse `create_session(` et
`get_or_create_canonical_session(`. Son **allowlist est livrée vide et assertie
vide** : quand elle rougit, le remède est de passer par `session_remote::ouvrir`,
jamais d'y ajouter une ligne. Elle refuse aussi `end_session(` **depuis le vol
V2d-4c** (remède : `session_remote::clore`) — c'est une garde **par fichier**, pas
par vol, d'où un nom qui ne bouge plus et deux vols qui la citent. La garde de
`handlers.rs` refuse le même motif pour son propre fichier depuis V2d-4b ; les
deux ne sont pas fondues : elles balaient deux fichiers et une garde fondue
rougirait sous un seul message.

### `/clear` passe par mika-spirit (mika#1727, vol V2d-4b)

`/clear` **ne clôt plus et ne crée plus sa session en local**.
`tui/commands/handlers.rs::clear_par_spirit` traverse deux portes de
`commands/session_remote.rs` : **`ouvrir` d'abord, `clore` ensuite — et cet ordre
est le livrable.** L'inverse perdrait la session courante avant de savoir si une
nouvelle peut naître, et laisserait la TUI sans session sur un spirit qui refuse.
`handle_clear` charge le `.env` de la home, lit `dashboard::spirit_url()` et
délègue, comme `handle_rewind_impl`.

Ce que chaque issue rend, et pourquoi elle est distincte :

| cas | effet sur l'état | message |
|---|---|---|
| singleton | rien n'est appelé, pas même la lecture du jeton ; la session est gardée | « Display cleared… » (inchangé) |
| sans `MIKA_INTERNAL_TOKEN` | refus **avant tout appel**, état intact, aucun `NewSession` | le refus commun, qui nomme la variable |
| `ouvrir` échoue | rien n'est clos, `session_id` ne change pas, aucun `NewSession` | `Error opening a new session: <issue>` |
| `clore` échoue | la **nouvelle** session est gardée, le worker l'a | dit que l'ancienne n'a pas été close, et nomme la cause |
| `ended: false`, `canonical: true` | la nouvelle session est gardée | dit que l'ancienne est la session canonique de l'agent (mika#1401) et reste ouverte |

`is_singleton_session` prend le `canonical` rendu par `ouvrir` : la route rend la
session canonique de l'agent au lieu d'en créer une, donc le serveur est la seule
autorité sur ce fait (même raison qu'au vol V2d-4a). Et `clore` rend
`{ended, canonical}` en **200** sur une session canonique, qui ne se clôt jamais
— d'où la lecture des **deux** champs, et non d'`ended` seul : un `ended: false`
canonique est le régime nominal de mika#1401, un `ended: false` non canonique est
un refus sans raison donnée, et les confondre dirait à l'opérateur qu'un geste a
échoué alors qu'il n'avait pas lieu d'être.

**Le refus structurel.**
`mika1727_v2d4b_la_tui_ne_clot_plus_la_base_en_local` (module `tests` de
`handlers.rs`) balaie la moitié production du fichier par `ProductionScanner` et
refuse `create_session(` (remède : `session_remote::ouvrir`) et `end_session(`
(remède : `session_remote::clore`). Son **allowlist est livrée vide et assertie
vide**. Elle porte un témoin *et* un contre-témoin : `production_of` masque les
régions `#[cfg(test)]` sans jamais tronquer, donc le premier atteste que le
balayage a lu la production (un chemin faux serait vert par cécité) et le second
qu'il a masqué la moitié test — sans lui, les appels des tests de ce même fichier
feraient rougir la garde, et on l'aurait « réparée » par une entrée d'allowlist.
Depuis le vol V2d-4d elle refuse un **troisième** motif, `maybe_compact(`
(remède : `session_remote::compacter`). Il n'était pas un motif de ce vol-ci :
une garde qui refuse un geste que rien ne remplace encore ne laisse d'autre
issue qu'une entrée d'allowlist, donc le motif est arrivé dans le commit qui a
câblé `/compact`, et pas avant. Le nom de la garde n'a pas changé : c'est une
garde **par fichier**, pas par vol.

**Une seule lecture du jeton dans tout le crate.**
`session_remote::jeton_interne(SourceEnv)` remplace les deux copies que portaient
`chat.rs` (refus en français nommant l'ouverture de session) et `handlers.rs`
(refus en anglais nommant le rembobinage) : elles avaient déjà divergé, et c'était
l'argument de leur mise en commun. Le refus nomme `MIKA_INTERNAL_TOKEN`, dit que
`MIKA_DASHBOARD_TOKEN` **n'est pas accepté** par les routes de session et de
rembobinage, et **ne nomme aucun geste** — trois sites l'appellent, et aucun n'a
autorité sur les deux autres. Il rend un `Err(String)` et non un
`SessionRemoteIssue` : une variable absente n'a ni statut ni URL, et un
`Unexpected { status: None }` se lirait « spirit injoignable » (motif mika#2331).
`grep -rn "fn jeton_interne" crates/mika-cli/src` rend **une** ligne.

### Les sorties de la TUI passent par mika-spirit (mika#1727, vol V2d-4c)

Les **trois** sorties de `commands/chat.rs` — changement d'agent, `/restart`, fin
de session — **ne closent plus la base en local**. Chacune appelle
`chat::clore_session_de_sortie`, qui prend l'URL de base et la source
d'environnement **par argument** (la forme d'`ouvrir_session_par_spirit` et de
`clear_par_spirit`) et passe par `session_remote::clore`.

**L'ordre du corps est le contrat**, et il se mesure : singleton → jeton →
appel.

| cas | ce qui part sur le fil | ce qui est rendu |
|---|---|---|
| singleton | rien, pas même la lecture du jeton | `Ok(())` |
| sans `MIKA_INTERNAL_TOKEN`, non singleton | rien | `Err` qui nomme la variable |
| 401, 404, injoignable | l'appel a eu lieu | `Err` qui porte le `Display` de `SessionRemoteIssue` |
| `ended: false`, `canonical: true` en 200 | l'appel a eu lieu | `Ok(())` + un `warn!` qui dit le régime nominal de mika#1401 |
| `ended: false`, `canonical: false` en 200 | l'appel a eu lieu | `Ok(())` + un `warn!` qui dit « sans raison donnée » |

Un agent singleton ne clôt rien **et ne lit pas le jeton** : la canonique ne se
clôt jamais (mika#1401), donc il n'y a rien à demander au fil et aucune raison
d'exiger une variable d'environnement pour ne rien faire. Le test l'atteste avec
une source d'environnement **vide** — le seul montage qui distingue « on n'a pas
lu » de « on a lu puis renoncé ». Le `!app.is_singleton_session` a disparu des
trois sites : la décision vit **dans** la fonction, et la laisser aussi chez
l'appelant en ferait un second juge du même fait.

**Une clôture en échec ne retient jamais la sortie.** Les trois sites
journalisent un `tracing::warn!` qui nomme **leur** geste, et continuent : la
TUI ne doit pas dépendre de spirit pour quitter, changer d'agent ou redémarrer.
C'est la raison pour laquelle l'erreur monte à l'appelant au lieu d'être
journalisée dans la fonction — trois lignes identiques diraient « la clôture a
échoué » sans dire **laquelle des trois sorties** l'a demandée. Au dernier site,
le terminal est déjà rendu et le lecteur d'événements arrêté : il n'y a plus
d'écran où porter autre chose qu'un journal.

**L'agent quitté est capturé, pas relu.** Au changement d'agent,
`let agent_quitte = app.agent_name.clone();` précède immédiatement l'appel :
`app.agent_name` est réaffecté une soixantaine de lignes plus bas, et une clôture
au nom du **nouvel** agent clôrait une session qui n'est pas la sienne — le corps
porte l'agent, et le contrôle d'appartenance du serveur ne rattraperait pas ce
défaut puisque `resolve_agent` aurait réussi. La capture **supprime** le mode de
panne, là où une garde qui aurait comparé deux numéros de ligne n'aurait fait que
le surveiller, et aurait rougi sur tout déplacement innocent.

**Le miroir est lu dans la fonction**, qui rend `()`. Aucun des trois sites n'a
d'écran où porter la disjonction de mika#1401 ; le leur rendre les obligerait à
rejouer trois fois la même lecture sans destinataire. `/clear` la rend à l'écran
parce que `/clear` **a** un écran.

Ce vol ne câble pas `compacter` ; le suivant le fait, et il ne reste alors plus
aucune attente `dead_code` dans `session_remote.rs`.

### `/compact` passe par mika-spirit (mika#1727, vol V2d-4d)

`/compact` **ne compacte plus la conversation en local**. `handle_compact` n'est
plus qu'une enveloppe — le `.env` de la home et `spirit_url()`, la forme de
`handle_clear` — et délègue à `compact_par_spirit`, qui prend l'URL de base et la
source d'environnement **par argument** et passe par `session_remote::compacter`.

La fonction prend **`&App` et non `&mut App`**, seule différence de forme avec
`clear_par_spirit` : `/compact` ne modifie aucun champ, la conversation étant
recomposée côté serveur et relue au prochain poll comme n'importe quelle autre
écriture de la base. Un `&mut` que le corps n'utilise pas annoncerait une
mutation dans la signature et laisserait un appelant futur en glisser une sans
que la revue la voie.

**L'ordre du corps est le contrat**, et il se mesure : pas-au-repos → jeton →
appel.

| cas | ce qui part sur le fil | ce qui est rendu |
|---|---|---|
| `app.status != Idle` | rien, pas même la lecture du jeton | « Cannot compact while agent is busy. » |
| sans `MIKA_INTERNAL_TOKEN`, au repos | rien | le refus de `jeton_interne`, qui nomme la variable |
| `{compacted: false, messages: n}` en 200 | l'appel a eu lieu | « Nothing to compact (*n* messages). » |
| `{compacted: true, messages: n}` en 200 | l'appel a eu lieu | « Compaction requested (*n* messages before)… » |
| 409 | l'appel a eu lieu | un message qui dit l'occupation **par un autre client**, et le nouvel essai |
| 401, 404, 500, injoignable | l'appel a eu lieu | le `Display` de `SessionRemoteIssue` |

**Pourquoi le seuil n'est plus côté client.** Avant ce vol, `handlers.rs`
comparait `count_messages()` à `50` écrit en dur, et affichait `50` comme
dénominateur, tandis que le serveur **lit** `compaction::COMPACTION_THRESHOLD`.
Deux vérités sur le même seuil, dont l'une était déjà une copie figée : le jour
où la constante bouge, le client refuse ou accepte au mauvais rang et affiche un
dénominateur faux, en silence. Le client ne compare donc plus rien et ne connaît
plus de seuil ; le serveur rend le fait **et** le compte. Le message de
`compacted: false` porte le compte rendu et **aucun dénominateur** — en afficher
un redemanderait au client de connaître le seuil qu'il vient de cesser de
connaître. `count_messages()` reste appelé par `/status`, ce qui est pourquoi il
ne peut pas devenir un motif de garde.

**Pourquoi `compacted: true` veut dire « demandée ».** La porte déclare que le
booléen signifie « la compaction a été **demandée et n'a pas échoué** », jamais
« une ligne `summary` a été écrite » : les deux booléens sont des 200, et `true`
dit que `maybe_compact` a rendu `Ok` — mais elle relit le seuil elle-même et peut
légitimement n'avoir rien fait. Un message qui dirait « Compacted conversation »
— ce que disait le code d'avant — affirmerait un état du système que le serveur
s'est gardé d'attester : c'est la grounding rule du dépôt, et un test l'atteste
par un `assert!` négatif sur « compacted » et « compactée », pas par relecture.

**Pourquoi le refus « pas au repos » reste local.** On pourrait croire qu'il
double le 409, que mika-spirit sait rendre. Tant que V1 n'est pas livré, le tour
de la TUI tourne **dans le processus de la TUI** (`chat.rs` appelle
`agent::run_agent`), donc il ne détient pas le verrou d'agent que
`/api/v1/compact` tente de prendre : un `/compact` lancé pendant ce tour
obtiendrait un **200**, et la compaction remplacerait par un résumé les messages
que le tour local est en train de relire. De ce fait-là, le refus local est le
**seul** juge, et il le restera jusqu'à V1. Les deux refus disent donc deux
choses différentes et gardent deux messages distincts : le local dit « ton propre
tour tourne », le 409 dit « un autre client tient le verrou de cet agent ». Les
fondre enverrait l'opérateur attendre la fin de son propre tour alors qu'un autre
client est en cause, ou l'inverse.

**V2d-4 est fini.** Le démarrage (V2d-4a), `/clear` (V2d-4b), les trois sorties
(V2d-4c) et `/compact` (V2d-4d) passent tous par mika-spirit ; les trois portes
de `session_remote.rs` sont câblées et il n'y reste **aucune** attente
`dead_code`, ce que `cargo clippy -p mika-cli --all-targets` mesure. `App.llm`
est parti avec le dernier lecteur local. La suite du ticket est V3 puis V4.

### Le worker de la TUI ne porte plus de provider (mika#1727, vol V3b-2)

Depuis V3b-1 le modèle choisi voyage sur le fil sous `mika.model_override`
(`mika_a2a::params::MODEL_OVERRIDE_KEY`), posé par `AgentRequest::SetModel` et
lu par `chat_turn`. Ce vol retire ce qui restait derrière : **`spawn_agent_worker`
prend un `DbContext`** — le worker n'a jamais eu besoin que de la base — et
`init.rs` perd `AppContext`, son `Deref`, `override_model`, `init_for_agent` et
`connect_mcp`. `init_db_only_for_agent` est désormais la **seule** porte d'entrée
des commandes interactives, `mika ask` comprise.

**MCP quitte la TUI en entier.** Aucun outil de ce processus n'y touchait depuis
V1 : le tour tourne chez spirit, qui monte son propre `McpManager`. Les deux
rechargements de skills à chaud partent avec `skills_dirty`, un drapeau que rien
ne posait ; depuis V3c-2b-A1 l'assemblage de démarrage part aussi, et la TUI
lit le miroir de la route des skills, relu à chaque respawn.

**Ce qui reste local est la seule part indépendante du provider.**
`mika chat --model ""` est refusé au lancement, avec le message d'`ask.rs` —
porté par `commands::ask::MODEL_MUST_NOT_BE_EMPTY`, une constante partagée et non
deux littéraux. Un `--model` non vide part au worker **tel que tapé** et c'est
lui qui s'affiche au démarrage : afficher le modèle configuré pendant qu'un
override le remplace serait une fausse provenance (mika#2331). Bâtir un provider
ici pour valider davantage ne pourrait plus que **refuser** un tour que spirit
sait servir, sur une clé absente de ce shell (mika#2293).

**`/provider` devient honnête**, et c'est le second livrable du vol : voir la
puce `/provider` plus bas pour ce qu'il fait, et `handlers.rs` pour le pourquoi
mika#1591 — un override préfixé d'un provider que le spirit **en cours** n'a pas
configuré est refusé fermement et couperait tous les tours suivants.

**Le `SetModel` se mesure de bout en bout (vol V3c-2c).** L'écart É1 du plan de
V3b-1 est fermé : `mika1727_v3c2c_apres_un_setmodel_le_tour_suivant_porte_loverride`
(`commands/chat.rs`) appelle `spawn_agent_worker` en entier contre le factice
de `rewind_remote`, et lit l'override sur le corps de chaque tour. Pour qu'un
test l'atteigne sans muter l'environnement, l'adresse de spirit et le lecteur
du jeton arrivent au worker **par le spawn** : `run` passe `spirit_url()` et
`std::env::var`, et le worker ne relit plus l'adresse à chaque tour. Une
bascule d'agent et `/restart` repassent par le spawn, et la relisent.

**Le refus structurel.** La garde de V3b-1
(`mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider`, module `tests`
de `commands/chat.rs`) **garde son nom** — elle est par fichier, pas par vol — et
passe à deux sites (`chat.rs`, `ask.rs`) et douze motifs, dont `connect_mcp`,
`McpManager`, `init_for_agent`, `AppContext` et `skills_dirty`. Allowlist livrée
vide et assertie vide, un témoin par site, le contre-témoin sur `chat.rs` : quand
elle rougit, le remède est de router l'appelant, jamais d'ajouter une ligne.

### La route des skills est la source de la TUI (mika#1727, vols V3c-2a-ii à V3c-2b)

`commands/skills_remote.rs::lire(base_url, token, agent)` fait le GET de
`/api/v1/skills` — nom d'agent en paramètre de requête **encodé par reqwest**,
jeton interne pris par `chat::lire_les_skills` sur `session_remote::jeton_interne`
et posé en `Authorization` — et rend un miroir des quatre clés de la route,
treize champs par skill. **Son appelant est `spawn_agent_worker`** : au
démarrage, à la bascule d'agent et au `/restart`. Pas de rechargement à chaud —
la route est **relue au respawn**.

`app.skills` porte un `Arc<EtatSkills>`, soit `Result<SkillsMirror, String>` : le
miroir lu, ou la cause du refus **en une ligne**, figée à l'instant de la
lecture. Une lecture en échec **n'est pas fatale** — la TUI démarre. Cinq refus
distincts, et pas le vocabulaire de `session_remote` : `Injoignable` (aucun
statut, par construction du variant), `AgentInconnu` (404 — ici un 404 ne peut
vouloir dire qu'agent inconnu, et le message nomme l'agent **demandé**, jamais
une session), `JetonRefuse` (401 — le message nomme `MIKA_INTERNAL_TOKEN`),
`Statut` (tout autre statut que 200, rendu avec son corps), `CorpsIllisible` (un
200 au corps illisible est un refus, jamais un miroir à champs vides).

**Ce que disent les quatre surfaces sur échec.** Le démarrage rend la ligne de
cause telle quelle (`chat::message_de_cause`), sans cadre qui la dirait deux
fois. `/skills` et `/skill <nom>` (`handlers.rs`) rendent la cause et **jamais**
« No skills loaded. » ni « No skill found with name '…' » — ces deux phrases
enverraient chercher la panne du côté des skills, la provenance fausse que
mika#2331 refuse ; « No skills loaded. » n'appartient qu'au miroir **lu** dont
les deux listes sont vides. L'autocomplétion de `/skill` (`completers.rs`) ne
propose **rien** : une liste vide s'y lirait « cet agent n'a pas de skill », et
la cause est déjà dite par les trois autres surfaces. Seuls `SkillsMirror::agent`
et `SkillMirror::origin` gardent une attente `dead_code`, faute de surface qui
les affiche.

**Le refus structurel (V3c-2b-A2).**
`mika1727_v3c2b_a2_la_tui_ne_nomme_plus_le_registre_des_skills`, module `tests`
de `tui/app.rs`, interdit `SkillRegistry` dans la moitié production de **chaque**
fichier de `tui/`, balayé en entier — par dossier et non par fichier, pour qu'un
fichier neuf soit couvert sans qu'on y pense ; par dossier et non par crate,
parce que `commands/skills.rs` construit légitimement le registre sur le disque.
Allowlist livrée vide et assertie vide, un témoin sur `app.skills`, un
contre-témoin de masquage et un contre-témoin de détection. Quand elle rougit, le
remède est `skills_remote::lire` par `app.skills`, jamais une ligne d'allowlist.

### Le mode équipe de la TUI passe par mika-spirit (mika#1727, vol V4-b2)

`mika chat --team` **ne fait plus tourner l'équipe dans son propre processus**.
`chat::servir_les_buts_dequipe` — le worker sorti de `run_team` — envoie chaque
but par `team_runs_remote::lancer_run_equipe` (`POST /api/v1/teams/{name}/runs`,
puis le flux SSE jusqu'à sa trame finale) et pousse chaque `TeamEvent` reçu sur
le canal de la TUI, dans l'ordre. Le `TeamRun` rendu passe par le même
`team_outcome::classify` qu'avant ; un refus du client devient un
`TeamEvent::RunFailed` qui porte son `Display`.

**`MIKA_INTERNAL_TOKEN` est lu avant l'écran.** `acces_du_mode_equipe` prend
`dashboard::spirit_url()` et `session_remote::jeton_interne` **avant** les
canaux, le hook de panique et `enable_raw_mode` : sans jeton, le `?` de
`run_team` sort en non-zéro qui nomme la variable, et l'écran ne s'affiche pas.

**Aucun repli local.** `Settings::load`, l'App GitHub, `AgentTier::from_env()`
et `Deployment::from_env()` ne sont plus lus par ce worker : il n'y a pas de
chemin qui relancerait l'équipe ici si spirit est à terre.

**La porte refuse un nom invalide sans aucune requête.** `lancer_run_equipe`
appelle `mika_common::team::validate_team_name` **avant** de composer l'URL : le
nom entre dans le chemin par `format!`, donc un caractère réservé ferait un
*autre* chemin — un 404 — là où le juge serveur aurait rendu son 400.

**Ce qui reste local** (défaut 42) : le résumé du run de référence de `--run-id`,
lu dans la base du conteneur ; la sonde `is_dashboard_running()` ; et la
persistance des tours par `tui/app.rs`.
