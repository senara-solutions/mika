# Notifications opérateur à sévérité — vol 3/3 : le P0 phare est attesté bout en bout

- **Ticket** : senara-solutions/mika#1381 (`Part of #1381` — ce vol ne ferme pas le ticket)
- **Branche** : `feat/1381/notifications-severity-tiered-operator`
- **Type** : feat
- **Date** : 2026-10-20
- **Plans antérieurs** :
  - `docs/plans/2026-10-19-001-feat-1381-notifications-severity-tiered-operator-plan.md` (vol 1, livré par #2813, mergé en `059652e9`)
  - `docs/plans/2026-10-20-002-feat-1381-notifications-severity-tiered-operator-plan.md` (vol 2, livré par #2846, mergé en `c5e64079`)
- **Projet** : *Loop Trustworthiness*, phase 1b — keystone

---

## Ce que les vols 1 et 2 ont livré, et la seule chose qu'ils ont laissée ouverte

Le vol 1 a posé le rail (`NotificationSeverity`, `send_with_severity`, `escalation_chat_id`,
`MIKA_OPERATOR_ESCALATION_CHAT_ID`) et routé les 7 sites P0. Le vol 2 a fusionné les deux aides
en une seule à sévérité obligatoire, de sorte qu'un site de notification écrit demain ne peut plus
partir en P2 par défaut muet : le compilateur réclame le tier.

Les deux vols ont attesté les sites de handler par des tests unitaires en place
(`verdict_handler.rs § mika1381_tests`, `ci_failure_handler.rs § tests`). Un site, et un seul, est
resté hors de portée : **le P0 phare**, le calage de dispatch, `agent_loop/mod.rs:4337`. Le vol 1
l'a dit explicitement et a daté sa dette :

> **AC9** — `agent_loop/mod.rs:4260`, le P0 phare, appelle `send_with_severity(_, NotificationSeverity::P0)`. **Son attestation comportementale est renvoyée au vol 2** : le chemin de calage n'a aucun test aujourd'hui (`grep -rn ready_label_dispatch_stall crates/mika-agent/tests/` ⇒ vide) et en construire un est un scénario de harnais eval. C'est une lacune préexistante que le vol 1 n'aggrave pas et ne ferme pas.

Le vol 2 ne l'a pas fermée non plus — il l'a mesurée et renvoyée en vol 3, parce que
l'estimation du vol 1 (≈ 90 lignes) était optimiste d'un facteur deux et que garder les deux
moitiés dans le même vol donnait ≈ 323 lignes, au-dessus de la borne de 279. Son D3 est net sur
la raison structurelle pour laquelle le site ne pouvait pas monter dans le vol 2 :

> Le site `agent_loop/mod.rs:4337` (le P0 phare, calage de dispatch) n'appelle aucune aide : il appelle `sender.send_with_severity(&notification, NotificationSeverity::P0)` directement, parce qu'il n'est pas dans un handler qui en possède une. **Il n'est pas touché par ce vol.**

La vérification tient toujours au 2026-10-20 :
`grep -rn "ready_label_dispatch_stall\|dispatch stalled" crates/mika-agent/tests/` ⇒ **vide**.

Ce plan est le vol 3. Il ferme l'AC9 et rien d'autre.

## L'état interdit que ce vol attrape

Le site est la ligne du ticket entier. Son commentaire le dit lui-même
(`agent_loop/mod.rs:4330-4333`) :

> *The flagship P0 of mika#1381: the `ready` label is gone and dispatch never ran, so the loop is dead and no further webhook will say so. This is the one line that must cross a mute.*

L'état interdit a donc deux formes, et les deux doivent rougir :

1. **La notification ne part pas.** Le site est atteint, la boucle est morte, et rien ne sort.
2. **La notification part sur la surface mutable.** Elle sort, mais en P2 — ou, pire, par `send()`
   sans tier du tout. L'opérateur a un chat muet et un diagnostic #4 (*alarm into the void*)
   intact : le ticket aurait été livré trois vols pour rien.

La seconde forme est celle qu'aucun test existant n'attrape, et elle est **silencieuse par
construction**, parce que `send_with_severity` est une méthode à défaut dont le corps est
`self.send(text).await`. Une régression qui remplacerait l'appel du site par `sender.send(...)`
compile, passe toute la suite, et dégrade le P0 phare en P2 sans qu'une seule ligne de log ne
change. C'est exactement la classe que le vol 2 a fermée sur les handlers *par le compilateur*, et
que le compilateur ne peut pas fermer ici : il n'y a pas d'aide à signature contraignante sur ce
chemin.

## Décisions

### D1 — Le modèle cité par le vol 2 n'est pas le bon modèle, et on ne rejoue pas la citation

Le corps du ticket et le D8 du vol 2 nomment `tests/eval/test_ready_label_blocked_skip.rs` comme
modèle de ce vol. **Lecture faite, la citation ne tient pas structurellement.** Ce fichier
n'utilise ni `EvalHarness` ni `MockLlmProvider` : il appelle
`try_handle_ready_label_dispatch_with_fetcher(...)` en direct, avec un `NoopSender` local et une
`AsyncDatabase` en mémoire, et assertionne sur `task_count` et `VerdictAction`. Il a été cité pour
sa **taille mesurée** (180 lignes), qui est ce dont le vol 2 avait besoin pour recalibrer son
estimation — pas pour sa forme.

Ce vol nomme les deux fichiers qui sont réellement ses modèles, chacun pour une moitié :

- **`tests/eval/grounding_regressions/engine_correction_rejection.rs`** (306 lignes) pour
  *atteindre le site*. Son `test_regression_engine_correction_rejection_pre_fix_shape` produit
  déjà, verbatim, les trois conditions du prédicat de `:4297` : un message d'entrée qui déclenche
  `ready_label_dispatch_trigger`, un tour 1 qui appelle `run_gh` + `send_message` mais **aucun
  outil de dispatch** (donc la garde tire et insère son label dans `intent_guard_retries`), et un
  tour 2 en `text_response` nu (donc `ready_label_dispatch_satisfied` reste faux). Ce test n'est
  pas `#[ignore]` : le chemin tourne en CI aujourd'hui.
- **`tests/eval/test_undelivered_send_2136.rs`** (429 lignes) pour *observer la frontière*. Son
  `ScriptedSender` est injecté par `.message_sender(sender.clone())` sur le builder, puis
  `harness.run(...)`, et les assertions portent sur `sender.sent()`. Il porte aussi la discipline
  clippy à respecter : relâcher le garde du `Mutex` avant tout point d'await (`sig_drop`, #1724).

Ce que `engine_correction_rejection.rs` ne fait pas, c'est appeler `.message_sender(...)` : donc
`tool_ctx.message_sender` y vaut `None` et le corps `if let Some(ref sender)` de `:4323` ne
s'exécute jamais. **La distance entre le chemin déjà couvert et l'attestation manquante est
d'une ligne de builder.** C'est la raison pour laquelle ce vol est petit.

Chaîne d'injection vérifiée de bout en bout : `EvalHarnessBuilder::message_sender`
(`harness.rs:459`) → `AgentParams.message_sender` (`harness.rs:116`) → `tool_ctx.message_sender`
(`agent_loop/mod.rs:6006`) → le corps de `:4323` → `send_with_severity(_, P0)` à `:4337`.

### D2 — Les stubs d'outils sont réutilisés, pas recopiés

`engine_correction_rejection.rs` porte quatre stubs (`StubSendMessage`, `StubRunClaudePilot`,
`StubRunGh`, `StubCreateTask`) et le constructeur `tools_with_dispatch_stubs()` qui part de
`default_tools()` et les enregistre. Ce sont exactement les outils dont ce vol a besoin, pour la
raison exacte : ils rendent le chemin de dispatch scriptable sans toucher au monde.

Leur module est déjà `pub mod engine_correction_rejection;` (`grounding_regressions/mod.rs:50`) et
le module englobant porte `#[allow(dead_code)]` (`tests/eval.rs:5`). Les cinq items deviennent
`pub(crate)` — cinq mots-clés sur des lignes existantes, aucune logique touchée — et un
doc-comment nomme le second consommateur, pour que la prochaine personne qui touche ces stubs
sache qu'un test de notification en dépend.

Recopier les 95 lignes de stubs dans le nouveau fichier serait la version DRY-négative du même
test, et elle dériverait au premier changement de `ToolDefinition`.

`StubSendMessage` est un no-op pur (`Ok(ToolOutput::success("Message sent"))`) : il ne touche pas
`message_sender`. C'est ce qui rend l'observation du double **propre** — l'unique chose que le
double verra pendant le run est la notification de `:4337`, jamais un `send_message` d'outil.

### D3 — Le double surcharge les deux méthodes, et la sentinelle est un témoin de fuite

Le double d'observation surcharge **`send` *et* `send_with_severity`**, sur le patron établi par
le D4 du vol 2 :

- `send_with_severity(text, severity)` pousse `(text, severity)` dans son journal ;
- `send(text)` pousse `(text, SEVERITE_ABSENTE)`.

`SEVERITE_ABSENTE` n'est pas de l'échafaudage : c'est un **témoin de fuite**. Un double qui
n'implémenterait que `send` ne pourrait pas distinguer un P0 d'un P2 — le défaut du trait
dégraderait l'appel avant qu'il n'arrive, et le test resterait vert sur la régression qu'il est
censé attraper. Avec la double surcharge, observer `SEVERITE_ABSENTE` **est** le rouge : il dit
« le site est repassé par `send()` », c'est-à-dire précisément la seconde forme de l'état interdit.

Le double ne réutilise pas `SeverityRecordingSender` de `verdict_handler.rs:5071` : celui-là vit
sous `#[cfg(test)]` dans la crate source et n'est pas accessible depuis le binaire de test `eval`.
Le patron est réutilisé, pas le type.

### D4 — Le contrôle négatif est obligatoire, et il porte sur le prédicat, pas sur le sender

Un test positif seul est satisfait par un notificateur inconditionnel. Le second test scripte donc
un run où le tour 2 **appelle un outil de dispatch** (`create_task` + `run_claude_pilot`, la forme
post-fix que `engine_correction_rejection.rs` script déjà), de sorte que
`ready_label_dispatch_satisfied` devienne vrai et que le prédicat de `:4297` soit faux. L'assertion
est alors : le double a observé **zéro** émission.

C'est le contrôle qui mesure que le test positif teste le prédicat de calage, et non la simple
présence d'un sender.

### D5 — Rouge-avant, et la recette est écrite dans le fichier

Un test de sévérité qui n'a jamais été rouge n'atteste rien. Deux recettes d'injection sont
exécutées avant le commit et consignées dans le header du fichier, chacune rougissant une forme
distincte de l'état interdit :

1. Remplacer `NotificationSeverity::P0` par `::P2` à `:4337` ⇒ le test positif échoue sur le tier.
2. Remplacer l'appel par `sender.send(&notification)` à `:4337` ⇒ le test positif échoue en
   observant `SEVERITE_ABSENTE`. **Cette injection compile** — c'est tout l'argument du D3 et du
   vol 2 réuni, et c'est la seule preuve que la méthode à défaut est bien l'angle mort qu'on
   prétend couvrir.

Et pour le contrôle négatif : retirer la conjonction `&& !ready_label_dispatch_satisfied(...)` du
prédicat de `:4297` ⇒ le contrôle négatif échoue sur une émission non attendue.

### D6 — La borne de couverture du vol 2 n'est pas levée, et ce vol ne la conteste pas

Le D6 du vol 2 a déclaré que **aucun** site P0 de `verdict_handler.rs` / `ci_failure_handler.rs`
n'est joignable par un test de bout en bout : `try_handle_ci_failure` exige un `github_token` et
émet de vrais appels API, et `verdict_handler` n'a aucun point d'entrée testé. Ce vol ne change
rien à cela. Il atteste **un** site — celui qui est joignable, et qui se trouve être le phare.

La borne reste donc : 1 site P0 attesté bout en bout sur 7, et 6 attestés unitairement à la
frontière de l'aide par les vols 1 et 2. Dire autre chose dans le corps de PR serait une
surenchère.

### D7 — Ce vol ne touche pas le site de production

Le fichier `agent_loop/mod.rs` n'est pas modifié. Ce vol ajoute un test et rend cinq items
`pub(crate)` ; c'est tout. Si l'attestation révélait que le site ne fait pas ce qu'il prétend, ce
serait un défaut à part entière, à ouvrir en ticket plutôt qu'à corriger sous couvert d'un vol
d'attestation — et le rouge-avant du D5 est précisément ce qui permettrait de le dire sans
ambiguïté.

### D8 — L'axe INTENT_GUARDS reste hors des trois vols

La seconde moitié du corps du ticket — *batch/suppress P2 acks via INTENT_GUARD relaxation (let
mika-dev EndTurn without send_message on non-actionable webhooks)* — est orthogonale au rail de
sévérité : elle porte sur les gardes de post-condition de l'EndTurn, pas sur le routage des
notifications. Elle n'est pas dans ce vol, et la PR porte donc `Part of #1381`, jamais `Closes`.

**Tracked in: senara-solutions/mika#1381 — axe INTENT_GUARDS** : relâchement de la garde de
post-condition pour laisser mika-dev clore un tour sans `send_message` sur un webhook non
actionnable (batching / suppression des acks P2). Hors vols 1 à 3, à groomer séparément.

---

## Phase unique — l'attestation

Un seul vol, une seule phase. Les trois gestes sont indissociables : le double sans le test
n'atteste rien, et le test positif sans le contrôle négatif atteste une émission inconditionnelle.

1. `engine_correction_rejection.rs` — les quatre stubs et `tools_with_dispatch_stubs()` passent
   `pub(crate)`, avec un doc-comment nommant le second consommateur (D2).
2. `tests/eval/test_dispatch_stall_p0_1381.rs` — header de doctrine + recettes de rouge-avant
   (D5), `SeverityRecordingSender` à double surcharge + `SEVERITE_ABSENTE` (D3), le test positif,
   le contrôle négatif (D4).
3. `tests/eval.rs` — déclaration du module, avec le commentaire d'une ligne qui est la convention
   du fichier.
4. `crates/mika-agent/CLAUDE.md` § *MessageSender Trait* — une phrase nommant le site désormais
   attesté et son test, à côté des deux gardes du vol 2. Hors du compte de lignes.

---

## Fire-Disposition

**Sans objet pour ce vol, et la section n'est pas inventée** (mika#2306, options mika#1574).

Ce vol ne livre aucun détecteur, au sens d'un livrable dont la fonction première est de signaler
une violation et dont le chemin de succès est « aucune violation trouvée ». Les candidats, écartés
un par un :

- **Le test positif** est fonctionnel. Il scripte un run, atteint un site, et vérifie la valeur qui
  en sort. Son chemin de succès est « la paire `(texte de calage, P0)` est arrivée », pas « rien
  n'a été trouvé ».
- **Le contrôle négatif du D4** est le candidat sérieux, parce que son assertion est une absence :
  zéro émission. Il n'est pourtant pas un détecteur. Il n'a **aucune population** : il exerce un
  chemin unique, entièrement scripté par le plan du test lui-même, et l'absence d'émission y est
  la *valeur attendue d'un comportement mesuré*, non le résultat d'une recherche sur un corpus
  pré-existant. Un détecteur au sens de mika#1574 balaie quelque chose qu'il n'a pas écrit — des
  sites de source, des fichiers, des entrées de registre — et peut donc découvrir des violations
  antérieures qu'il faut disposer. Celui-ci ne peut rien découvrir : il n'y a rien à découvrir
  dans un run qu'il a lui-même composé. C'est la même lecture que celle qu'a tenue le vol 2 sur
  ses trois tests négatifs-comportementaux.
- **La sentinelle `SEVERITE_ABSENTE`** n'est pas un détecteur non plus : c'est une valeur de
  journal, interne au double, qui rend une fuite *observable par le test positif*. Elle n'a ni
  armement, ni population, ni chemin de succès propre.
- **Aucun lint `scripts/`, aucun scan de source, aucune garde CI** n'est ajouté. En particulier ce
  vol n'ajoute rien du genre de `mika1745_t3_the_only_call_site_is_the_no_task_arm`
  (`ci_failure_handler.rs:1837`), qui compte les appelants d'une fonction dans la source et qui
  *est* un détecteur. Ce test existe, doit rester vert, et n'est ni livré ni modifié ici.
- **`scripts/check-suite-wiring.sh`** n'ouvre pas d'entrée : le fichier ajouté vit sous
  `crates/mika-agent/tests/`, que le binaire `eval` exécute déjà en CI
  (`cargo test -p mika-agent --test eval`). Aucune ligne `unwired` n'est créée (mika#2793).

La contrainte structurelle héritée du vol 1 reste par ailleurs en vigueur sans être retouchée :
`qa_head_supersession.rs:664` scanne le littéral `"GatewayMessageSender::new("` dans
`run_agent_for_message`. Ce vol ne touche aucun site d'armement, donc la contrainte est satisfaite
par construction ; AC8 le vérifie.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `tests/eval/test_dispatch_stall_p0_1381.rs` — header de doctrine + recettes de rouge-avant, imports, `SeverityRecordingSender` à double surcharge + `SEVERITE_ABSENTE`, le test positif, le contrôle négatif | 185 |
| `tests/eval/grounding_regressions/engine_correction_rejection.rs` — `pub(crate)` sur les quatre stubs et `tools_with_dispatch_stubs()`, doc-comment nommant le second consommateur | 6 |
| `tests/eval.rs` — déclaration du module + son commentaire | 3 |

Total estimé : 194 lignes

Sous la borne par vol de 279 lignes, avec une marge de 30 %. Elle tient pour une raison
structurelle et non par découpage : le chemin qui atteint `:4337` est **déjà** écrit et vert en CI
(`engine_correction_rejection.rs`, D1), et l'injection manquante est un appel de builder. Ce vol
n'écrit donc que l'instrument d'observation et ses deux assertions — pas le scénario. L'estimation
de ≈ 180 lignes du vol 2, calibrée sur les 180 lignes mesurées de
`test_ready_label_blocked_skip.rs`, se confirme à 194 après lecture des deux vrais modèles : le
double à double surcharge coûte une quinzaine de lignes de plus qu'un `NoopSender`, et les stubs
sont réutilisés au lieu d'être recopiés (D2), ce qui compense les 95 lignes qu'une copie aurait
ajoutées.

`crates/mika-agent/CLAUDE.md` est hors du compte : la consigne exclut `docs/` et `CLAUDE.md`.

Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

---

## Acceptance criteria

- **AC1** — `crates/mika-agent/tests/eval/test_dispatch_stall_p0_1381.rs` existe, est déclaré dans
  `crates/mika-agent/tests/eval.rs`, et **n'est pas** `#[ignore]` ni gardé par une variable
  d'environnement : il tourne dans `cargo test -p mika-agent --test eval` en CI.
- **AC2** — `mika1381_le_calage_de_dispatch_part_en_p0_sur_la_surface_jamais_muette` passe. Il
  injecte le double par `.message_sender(...)` sur `EvalHarness`, scripte un tour 1 avec `run_gh` +
  `send_message` (aucun outil de dispatch) puis un tour 2 en `text_response` nu, et assertionne sur
  les observations du double : **exactement une** paire, dont le texte contient
  `"Ready-label dispatch stalled"` et dont la sévérité **est** `NotificationSeverity::P0`.
- **AC3** — Le même test assertionne que la sévérité observée n'est **pas** `SEVERITE_ABSENTE`,
  avec un message d'échec qui nomme la cause : le site est repassé par `send()` et la méthode à
  défaut a mangé le tier.
- **AC4** — Le même test assertionne `trace.llm_call_count > 1`, ce qui atteste que la garde
  `webhook_ready_label_dispatch` a bien tiré son unique re-prompt correctif. Sans cet appel, le
  prédicat de `:4297` n'aurait pas pu être vrai et le test serait vert pour la mauvaise raison.
- **AC5** — `mika1381_un_dispatch_qui_a_eu_lieu_ne_notifie_rien` passe. Même injection, tour 2
  appelant `create_task` + `run_claude_pilot`, et le double a observé **zéro** émission.
- **AC6** — Le double surcharge `send` **et** `send_with_severity` (D3). `grep -n "async fn send"
  crates/mika-agent/tests/eval/test_dispatch_stall_p0_1381.rs` retourne les deux méthodes. Aucun
  garde de `Mutex` n'est tenu à travers un point d'await (`clippy::sig_drop`, #1724) :
  `cargo clippy --all-targets` est propre.
- **AC7** — Les trois recettes de rouge-avant du D5 ont été exécutées et sont consignées verbatim
  dans le header du fichier, chacune avec le test qu'elle rougit et le message d'échec obtenu. En
  particulier la recette 2 (`send()` au lieu de `send_with_severity`) **compile** et rougit en
  `SEVERITE_ABSENTE` — c'est elle qui atteste que la méthode à défaut est bien l'angle mort couvert.
- **AC8** — `agent_loop/mod.rs` n'est pas modifié (D7), et aucun site d'armement de
  `GatewayMessageSender` ne bouge : `mika2671_b2_la_garde_de_revue_precede_le_tour`
  (`qa_head_supersession.rs:664`) reste vert.
- **AC9** — `engine_correction_rejection.rs` : les quatre stubs et `tools_with_dispatch_stubs()`
  sont `pub(crate)`, et ses deux tests restent verts. Aucune ligne de leur logique n'a changé — le
  diff sur ce fichier est le mot-clé de visibilité et un doc-comment.
- **AC10** — `crates/mika-agent/CLAUDE.md` § *MessageSender Trait* nomme le site `:4337` comme
  désormais attesté et cite le test, à côté des deux gardes du vol 2. Le cap de taille de
  `CLAUDE.md` reste respecté (`crates/mika-common/tests/claude_md_size.rs` vert).
- **AC11** — `grep -rn "ready_label_dispatch_stall\|Ready-label dispatch stalled" crates/mika-agent/tests/`
  ne retourne plus le vide. C'est la mesure littérale de la fermeture de l'AC9 du vol 1.

---

## Definition of Done

- [ ] `cargo test -p mika-agent --test eval` vert, y compris les deux tests neufs et les deux tests
      de `engine_correction_rejection.rs`.
- [ ] `cargo test` vert sur l'ensemble du workspace.
- [ ] `cargo clippy --all-targets` sans avertissement, `sig_drop` compris.
- [ ] `cargo fmt --check` propre.
- [ ] `scripts/verify-pipeline.sh origin/main` vert — y compris U2, la garde structurelle qui exige
      que ce plan porte à la fois `## Acceptance criteria` et `## Definition of Done`
      (mika#1600, mika#1627).
- [ ] Les trois recettes de rouge-avant du D5 exécutées, et le site de production restauré à
      l'identique après chacune (`git diff crates/mika-agent/src/agent_loop/mod.rs` ⇒ vide).
- [ ] AC1 à AC11 vérifiés un par un, chacun par la commande ou le nom de test qu'il nomme.
- [ ] Corps de PR portant `Part of #1381` (jamais `Closes` — l'axe INTENT_GUARDS reste ouvert, D8),
      la ligne `Tracked in: senara-solutions/mika#1381 — axe INTENT_GUARDS`, la borne de couverture
      du D6 énoncée sans surenchère, et `Code review: delegated to mika-qa (mika#2694)`.

---

## Risques

- **Un garde de post-condition voisin consomme des tours et change le compte d'appels LLM.** La
  boucle générique des INTENT_GUARDS fait `continue` dès qu'un garde tire
  (`agent_loop/mod.rs:3376`), donc le site `:4292` n'est pas atteint au step où la garde tire —
  c'est ce qui garantit **une seule** notification, et c'est aussi ce qui rend le compte d'appels
  sensible à un garde voisin. `webhook_zero_tools` est satisfait par le `run_gh` du tour 1,
  exactement comme dans le modèle. Si un autre garde tire malgré tout, le symptôme est une séquence
  scriptée épuisée, pas une assertion fausse : la remédiation est d'ajouter une réponse scriptée et
  de lire `trace.llm_call_count`, jamais d'affaiblir une assertion.
- **La réutilisation des stubs couple deux fichiers de test.** Mitigé par le doc-comment du D2 et
  par AC9 : la prochaine personne qui touche ces stubs voit nommé le second consommateur, et les
  deux tests du modèle sont dans la DoD.
- **L'attestation pourrait révéler que le site ne fait pas ce qu'il prétend.** C'est le résultat le
  plus utile possible, et il ne se corrige pas dans ce vol (D7) : il s'ouvre en ticket, avec le
  rouge-avant du D5 comme preuve que l'instrument était calibré avant la mesure.
- **Le ticket a trois plans antérieurs et quatre refus d'auto-pull.** Ce vol est petit et sa branche
  porte déjà deux vols livrés ; le risque de dérive est dans le corps de PR plutôt que dans le code.
  Mitigé par la DoD : `Part of #1381`, la borne du D6 énoncée telle quelle, et l'axe INTENT_GUARDS
  tracé explicitement.
