# Notifications opérateur à sévérité — vol 2/3 : une seule aide, un argument obligatoire

- **Ticket** : senara-solutions/mika#1381 (`Part of #1381` — ce vol ne ferme pas le ticket)
- **Branche** : `feat/1381/notifications-severity-tiered-operator`
- **Type** : feat
- **Date** : 2026-10-20
- **Plan antérieur** : `docs/plans/2026-10-19-001-feat-1381-notifications-severity-tiered-operator-plan.md` (vol 1, livré par #2813, mergé en `059652e9`)
- **Projet** : *Loop Trustworthiness*, phase 1b — keystone

---

## Ce que le vol 1 a livré, et ce qu'il a laissé ouvert

Le vol 1 a posé le rail et l'a armé :

- `NotificationSeverity { P0, P2 }` et `TargetKind { Normal, Escalation, EscalationUnsetFallback }` dans `crates/mika-agent/src/messaging.rs` ;
- `MessageSender::send_with_severity(text, severity)`, **méthode à défaut** qui retombe sur `self.send(text)` — les ~24 autres impls du trait n'ont pas bougé d'une ligne ;
- `GatewayMessageSender.escalation_chat_id: Option<i64>`, armé par `with_escalation_chat_id()` en chaînage après `new()`, jamais par `new()` ;
- le réglage `operator_escalation_chat_id` / `MIKA_OPERATOR_ESCALATION_CHAT_ID`, documenté dans `docs/operator/environment-variables.md` § *Le P0 passe par un second chat, et un chat d'escalade absent le dit (mika#1381)* ;
- les 7 sites P0 routés, les 10 sites P2 laissés **textuellement intacts**.

Pour obtenir ce dernier point, le vol 1 a livré une **seconde aide** plutôt qu'un paramètre :
`send_notification` (P2) et `send_notification_p0` (P0), dans `verdict_handler.rs` comme dans
`ci_failure_handler.rs`, partageant un `report_notification_outcome` commun. Son propre
doc-comment nomme la dette :

> *Deliberately a **second** helper rather than a `severity` parameter on `send_notification`:
> vol 1 of mika#1381 keeps the ten P2 callsites textually unchanged, so a reviewer's diff shows
> only the sites whose routing moved. The merge into one helper carrying an explicit severity is vol 2.*

Ce plan est ce vol 2. Le corps du ticket le nomme dans les mêmes termes.

## Pourquoi la fusion mérite un vol, et ce qu'elle achète exactement

Deux aides, c'est un **défaut muet**. `send_notification(sender, msg)` compile, et route en P2.
Un nouveau site de rupture écrit dans six mois par quelqu'un qui n'a jamais lu mika#1381 part sur
le chat mutable, et **rien ne le dit** : ni le compilateur, ni un test, ni la revue — le site a
exactement la forme des dix autres. C'est la classe de défaut que ce ticket existe pour fermer
(*alarm into the void*, diagnostic #4) en train de se reconstituer par la porte de derrière.

Une aide unique à paramètre obligatoire supprime ce défaut : **on ne peut pas écrire une
notification opérateur sans choisir sa sévérité**, parce que le code ne compile pas. Le
compilateur devient la garde sur les 16 sites existants et sur tous les suivants. C'est le seul
gain du vol, et il est structurel plutôt que comportemental : aucune notification ne change de
destination.

### Le constat central : l'état interdit se déplace avec la fusion

C'est le point qui gouverne la moitié des décisions ci-dessous.

Avant la fusion, l'état interdit est *« le P0 est passé par la mauvaise aide »*. Il est
observable **par aide** : il suffit d'appeler `send_notification_p0` et de vérifier qu'une
sévérité arrive à la frontière. C'est exactement ce que font les deux tests du vol 1
(`mika1381_tests`, `verdict_handler.rs:5021`), au moyen d'un `SEVERITE_ABSENTE` poussé par
`send` et d'une sévérité réelle poussée par `send_with_severity`.

Après la fusion il n'y a plus qu'une aide, qui appelle toujours `send_with_severity`. L'assertion
« une sévérité est arrivée » devient **tautologique** : elle est vraie par construction, pour les
deux tiers, et resterait verte si tous les sites passaient `P2`. L'état interdit est devenu
*« un site passe la mauvaise sévérité »*, et il n'est plus observable par aide — seulement **par
site**.

Conséquence : la fusion ne peut pas être livrée en gardant les tests du vol 1 tels quels. Les
garder verts serait pire que les supprimer, parce qu'un test tautologique se lit comme une
couverture. Ils doivent être réécrits sur le nouvel état interdit (D4), et au moins un site réel
doit être attesté de bout en bout (D6).

---

## Décisions

### D1 — Une aide unique par handler, signature `(sender, severity, message)`

```rust
async fn send_notification(
    sender: &Arc<dyn MessageSender>,
    severity: NotificationSeverity,
    message: &str,
)
```

Elle appelle **toujours** `sender.send_with_severity(message, severity)`, puis
`report_notification_outcome(outcome, severity)`.

**L'ordre des paramètres est le fond de la décision, pas de la cosmétique.** Douze des treize
sites de `verdict_handler` et un des trois de `ci_failure_handler` sont multi-lignes, de la forme :

```rust
send_notification(
    sender,
    &format!(
        "PR #{} on {} — VERDICT: block[ac] from @{}. …",
        event.pr_number, event.repo, event.reviewer, task_id
    ),
)
.await;
```

Avec la sévérité en dernier, elle atterrit **après** un `format!` de quatre à six lignes : la
classification du site devient invisible au survol du diff, ce qui annule une partie du gain de
lisibilité que deux noms d'aide distincts procuraient. En deuxième position, elle est sur la ligne
qui suit `sender`, toujours au même endroit, sur les seize sites. La revue d'un nouveau site se
fait alors sur deux lignes au lieu d'une lecture jusqu'à la parenthèse fermante.

### D2 — `send_notification_p0` est supprimée, pas dépréciée

Pas de `#[deprecated]`, pas d'alias conservé. Une aide dépréciée qui compile est un défaut muet
remis en service : c'est précisément l'état dont D1 veut sortir. Les deux fonctions sont privées
au module (`async fn`, sans `pub`), donc il n'existe aucun appelant hors des deux handlers — la
suppression est locale et le compilateur la vérifie.

`report_notification_outcome` est **conservée telle quelle** dans les deux handlers. Elle reste
une fonction distincte même avec un seul appelant : son doc-comment dit « one copy, so the two
helpers above cannot drift », et cette raison disparaît avec la fusion, mais sa structure en
quatre bras (`Delivered` / `Failed` / `NoChannel` / `Err`) reste la seule chose qui sépare
l'émission de la journalisation. La fusionner dans l'aide gonflerait l'aide sans rien simplifier.
Seul son doc-comment est corrigé, pour ne pas laisser une justification morte.

### D3 — Les 16 sites portent leur sévérité, et la classification du vol 1 n'est pas relitigée

Le vol 1 a arbitré dix-sept classes de message une par une, sous la règle R6 (*« une surface
"jamais mise en sourdine" qui reçoit dix-sept classes de message est une surface qui sera mise en
sourdine »*), et cette table est dans son plan. **Ce vol la transcrit, il ne la rouvre pas.** Tout
site dont la sévérité changerait ici serait un changement de comportement déguisé en refactor.

| fichier | ligne (au 2026-10-20) | aide actuelle | sévérité portée |
|---|---|---|---|
| `verdict_handler.rs` | 534 | `send_notification` | `P2` |
| `verdict_handler.rs` | 915 | `send_notification` | `P2` |
| `verdict_handler.rs` | 1373 | `send_notification_p0` | `P0` |
| `verdict_handler.rs` | 1442 | `send_notification` | `P2` |
| `verdict_handler.rs` | 1660 | `send_notification_p0` | `P0` |
| `verdict_handler.rs` | 1711 | `send_notification` | `P2` |
| `verdict_handler.rs` | 1868 | `send_notification_p0` | `P0` |
| `verdict_handler.rs` | 1935 | `send_notification` | `P2` |
| `verdict_handler.rs` | 2037 | `send_notification` | `P2` |
| `verdict_handler.rs` | 2162 | `send_notification` | `P2` |
| `verdict_handler.rs` | 2193 | `send_notification` | `P2` |
| `verdict_handler.rs` | 2346 | `send_notification_p0` | `P0` |
| `verdict_handler.rs` | 2634 | `send_notification_p0` | `P0` |
| `ci_failure_handler.rs` | 451 | `send_notification_p0` | `P0` |
| `ci_failure_handler.rs` | 542 | `send_notification` | `P2` |
| `ci_failure_handler.rs` | 720 | `send_notification` | `P2` |

Les numéros de ligne sont des repères à la date du plan, pas un contrat : ce qui est contractuel
est le **compte par fichier et par tier** — 8 P2 + 5 P0 dans `verdict_handler.rs`, 2 P2 + 1 P0
dans `ci_failure_handler.rs` — et c'est sous cette forme qu'AC2 le vérifie.

Le site `agent_loop/mod.rs:4337` (le P0 phare, calage de dispatch) n'appelle aucune aide : il
appelle `sender.send_with_severity(&notification, NotificationSeverity::P0)` directement, parce
qu'il n'est pas dans un handler qui en possède une. **Il n'est pas touché par ce vol.**

### D4 — Les deux tests du vol 1 sont réécrits sur le nouvel état interdit

`mika1381_tests` (`verdict_handler.rs:5021`) devient, pour chacun des deux tiers, une assertion
sur la **valeur** reçue plutôt que sur sa présence :

- `mika1381_un_p0_de_handler_natteint_jamais_la_surface_mutable` — appelle l'aide unique avec
  `P0` et exige `"p0"` à la frontière ;
- `mika1381_un_p2_de_handler_nescalade_jamais` — appelle l'aide unique avec `P2` et exige
  `"p2"`.

`SeverityRecordingSender` et sa constante `SEVERITE_ABSENTE` sont **conservés**, et leur rôle
change utilement. Le double surcharge `send` *et* `send_with_severity`, en poussant
`SEVERITE_ABSENTE` depuis le premier. Après la fusion, l'aide n'appelle plus jamais `send` : un
`SEVERITE_ABSENTE` observé devient donc le témoin d'une **fuite** — un chemin qui a perdu la
sévérité en route. Le contrôle négatif cesse d'être un échafaudage et devient une sonde. Son
doc-comment doit le dire, sans quoi le prochain lecteur le prendra pour un résidu et le
supprimera.

### D5 — Le trait `MessageSender` n'est pas touché, et son défaut reste perdant

`send_with_severity` garde son corps par défaut `self.send(text).await`, qui **perd** la
sévérité. Ce n'est pas une lacune à corriger dans ce vol, c'est la propriété qui rend D4
possible : un double qui n'implémente que `send` ne peut pas distinguer P0 de P2, donc
l'observation d'une sévérité à la frontière n'est jamais un artefact du trait.

Fait vérifié sur l'arbre au 2026-10-20 : `SeverityRecordingSender` (`verdict_handler.rs:5071`)
est le **seul** surchargeur de `send_with_severity` en dehors de `messaging.rs`. Les ~24 autres
impls — `NoopSender`, le rail CLI, et tous les doubles de `tests/eval/` — n'ont que `send` et
passent par le défaut. La fusion est donc **à comportement constant** pour toutes : avant, un P2
arrivait par `send` ; après, il arrive par `send_with_severity`, dont le défaut appelle `send`.
Le seul double qui voit la différence est celui que le vol 1 a écrit pour la voir, et D4 le
réécrit.

Pour `GatewayMessageSender`, même conclusion par un autre chemin : son `send()` délègue déjà à
`send_with_severity(text, P2)` (`messaging.rs:288`), donc les deux routes étaient déjà
confondues en production.

### D6 — Un site réel attesté de bout en bout, et la raison pour laquelle il n'y en a qu'un

D4 ferme le niveau de l'aide. Il faut au moins un site attesté, sinon la classification des
seize reste tenue par la seule revue.

**Le site retenu est `ci_failure_handler.rs:720`**, dans `surface_for_adoption` (définie à
`:603`). C'est le seul des seize appelable depuis un test unitaire : `mod tests` l'appelle déjà
dix fois (T1 à T8). Le nouveau test exerce le chemin qui notifie et exige que la frontière ait
observé `"p2"` — un contrôle négatif de premier intérêt, puisqu'il atteste qu'un accusé de
routine **ne monte pas** en P0 et ne pollue donc pas la surface jamais-muette (règle R6 du vol 1).

**Pourquoi aucun site P0 de ces deux handlers n'est attesté** — ce n'est pas un choix de
périmètre, c'est une impossibilité mesurée :

- `try_handle_ci_failure` (`:269`) exige un `github_token` et émet de vrais appels API
  (`fetch_failure_context` à `:938`, `fetch_job_log` à `:1008`). Son site P0 (`:451`) est en aval
  de ces appels. L'atteindre demanderait un mock HTTP que ce dépôt n'a pas pour ce chemin.
- `verdict_handler` n'a **aucun** point d'entrée testé : `grep` sur
  `try_handle_verdict\|handle_verdict(` dans `crates/mika-agent/tests/` et dans le fichier
  lui-même ne retourne aucun appelant de test. Ses trois modules de test (`tests` à `:3373`,
  `mika2667_tests` à `:4865`, `mika1381_tests` à `:5021`) testent des fonctions internes.

Construire l'un de ces deux harnais est un travail d'infrastructure de test qui dépasse ce vol et
qui n'a pas de rapport avec mika#1381. **Ce plan ne le promet donc pas**, et AC6 énonce la borne
de couverture explicitement plutôt que de la laisser se deviner par absence.

### D7 — `RecordingSender` de `ci_failure_handler` gagne la sévérité **en addition**

Le double (`:1465`) n'implémente aujourd'hui que `send` et ne stocke que le texte
(`sent: Mutex<Vec<String>>`). Il gagne `send_with_severity` et stocke la paire.

Contrainte dure : **`messages()` est conservé à l'identique**. Dix assertions des tests T1 à T8
l'appellent (`:1544`, `:1592`, `:1650`, `:1691`, `:1729`, `:1772`, …). Changer sa signature pour
retourner des paires ferait rougir dix tests sans rapport avec ce vol, et la tentation serait
alors de les ajuster en masse — un diff où la vraie régression se cache. Le double gagne donc un
**second** accesseur (`observe()`, qui retourne `Vec<(String, String)>` sur le modèle de
`SeverityRecordingSender`), et `messages()` reste la projection sur le texte.

### D8 — L'attestation du site de calage `agent_loop` part en vol 3

Le vol 1 a renvoyé au vol 2 l'attestation de son site phare — `agent_loop/mod.rs:4337`, le calage
de dispatch, *« the one line that must cross a mute »* — en la chiffrant : *« ≈ 150 lignes, plus
≈ 90 si l'attestation du site de calage `agent_loop` passe par le harnais eval »*.

Cette attestation est **faisable** : `EvalHarness` expose `.message_sender(Arc<dyn MessageSender>)`
(`crates/mika-agent/tests/eval/harness.rs:459`) et `MockLlmProvider` rend le chemin déterministe
sans réseau. Le voisin le plus proche existe et tient le même déclencheur
`webhook_ready_label_dispatch` : `tests/eval/test_ready_label_blocked_skip.rs`.

Mais ce voisin fait **180 lignes**, non 90 ; `test_ready_label_live_pilot_noop_2279.rs` en fait
davantage. L'estimation du vol 1 était optimiste d'un facteur deux, et la recalibrer sur le
voisin réel fait passer le total du vol 2 de 273 à ~323 lignes — **au-dessus de la borne par vol
de 279**. Le découpage s'impose donc par la mesure, et non par préférence.

Le partage est propre parce que les deux moitiés n'ont aucune dépendance : la fusion ne touche pas
`agent_loop`, et l'attestation du site de calage ne touche aucune aide. Chacune est mergeable
seule.

- **Tracked in: senara-solutions/mika#1381 — vol 3** : attestation bout-en-bout du site de calage
  `agent_loop/mod.rs:4337` via `EvalHarness::message_sender()` + `MockLlmProvider`, sur le modèle
  de `tests/eval/test_ready_label_blocked_skip.rs`. Ferme l'AC9 différée du vol 1. ≈ 180 lignes.

### D9 — La documentation

`crates/mika-agent/CLAUDE.md` § *MessageSender Trait* décrit la forme à deux aides livrée par le
vol 1. Elle devient fausse avec ce vol et doit énoncer la forme finale : une aide, un paramètre
obligatoire, et la raison (un nouveau site ne peut pas omettre sa sévérité). Le fichier pèse
85 462 octets pour un plafond de 100 000 (`crates/mika-common/tests/claude_md_size.rs`) — la
marge couvre largement cette correction.

`docs/operator/environment-variables.md` **n'est pas touché** : le réglage, ses sondes et ses
signaux sont inchangés par ce vol, qui ne modifie aucune destination. Le réécrire serait du
churn sur un fichier que les opérateurs grepent.

---

## Découpage en vols

Trois vols, dont le premier est livré.

| vol | contenu | état |
|---|---|---|
| 1 | Le rail (`messaging.rs`), le réglage (`config.rs`), les deux sites d'armement, les 7 sites P0, et la seconde aide `send_notification_p0` | **livré** — #2813, `059652e9` |
| 2 | **Ce plan.** Fusion des deux aides en une seule à `severity` obligatoire dans les deux handlers, les 16 sites portant leur sévérité, les deux tests du vol 1 réécrits sur le nouvel état interdit, `RecordingSender` étendu, un site P2 réel attesté | ce plan |
| 3 | Attestation bout-en-bout du site de calage `agent_loop/mod.rs:4337` par le harnais eval (AC9 différée du vol 1) | `Tracked in:` ci-dessus (D8) |

Le vol 2 est mergeable seul : il est à comportement constant (D5), le compilateur garantit la
complétude de la migration des seize sites, et il est attesté à deux niveaux (l'aide par D4, un
site réel par D6).

**Ce vol ne ferme pas mika#1381.** Le troisième axe du corps du ticket — relâcher les
`INTENT_GUARDS` pour grouper ou supprimer les accusés P2 sur les webhooks non actionnables,
cité comme `agent.rs:1591` dans le corps et aujourd'hui dans `agent_loop/mod.rs` — est orthogonal
au routage : il porte sur le *volume* d'émission, pas sur la *destination*. Il reste hors
périmètre, comme au vol 1.

- **Tracked in: senara-solutions/mika#1381 — axe INTENT_GUARDS** : relâchement des gardes pour
  grouper/supprimer les accusés P2 sur webhooks non actionnables. Non traité par les vols 1 à 3.

La PR de ce vol porte donc `Part of #1381`, jamais `Closes #1381`.

---

## Fire-Disposition

**Sans objet pour ce vol, et la section n'est pas inventée** (mika#2306, options mika#1574).

Ce vol ne livre aucun détecteur, au sens d'un livrable dont la fonction première est de signaler
une violation et dont le chemin de succès est « aucune violation trouvée ». Les trois candidats
possibles, écartés un par un :

- **Le compilateur** n'est pas un détecteur livré. La fusion le rend contraignant sur les sites de
  notification, mais c'est une propriété du langage obtenue par un changement de signature, pas un
  artefact qu'on arme, dont on mesure la population ni qu'on pourrait décider de livrer désarmé.
  Il n'y a pas de population pré-existante de violations à traiter, puisqu'il ne peut pas exister
  de site non migré : un tel site ne compilerait pas.
- **Les trois tests** de D4 et D6 sont fonctionnels et négatifs-comportementaux — ils exercent un
  chemin et vérifient la valeur qui en sort. Leur chemin de succès est « la sévérité attendue est
  arrivée », pas « rien n'a été trouvé ».
- **Aucun lint `scripts/`, aucun scan de source** n'est ajouté. En particulier, ce vol n'ajoute
  pas de scan du genre de `mika1745_t3_the_only_call_site_is_the_no_task_arm`
  (`ci_failure_handler.rs:1837`), qui lui en est un : il compte les appelants de
  `surface_for_adoption` dans la source. Ce test existe et doit rester vert — ce vol n'ajoute
  aucun appelant à cette fonction — mais il n'est ni livré ni modifié ici.

La contrainte structurelle du vol 1 reste par ailleurs en vigueur et doit être respectée sans
être retouchée : `qa_head_supersession.rs:664` scanne le littéral `"GatewayMessageSender::new("`
dans `run_agent_for_message` et exige qu'il suive `skip_superseded_review_turn(` (test
`mika2671_b2_la_garde_de_revue_precede_le_tour`). Ce vol ne touche pas les sites d'armement, donc
la contrainte est satisfaite par construction ; AC8 le vérifie.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `verdict_handler.rs` — aide unique à `severity`, `send_notification_p0` supprimée, doc-comments refondus, 13 sites portant leur sévérité | 40 |
| `ci_failure_handler.rs` — même fusion, 3 sites portant leur sévérité | 28 |
| `verdict_handler.rs` § `mika1381_tests` — les deux tests réécrits sur la valeur reçue, doc-comment de `SeverityRecordingSender` corrigé | 25 |
| `ci_failure_handler.rs` § `tests` — `RecordingSender` gagne `send_with_severity` + `observe()`, et un test du site P2 `:720` | 50 |

Total estimé : 143 lignes

Sous la borne par vol de 279 lignes, avec une marge réelle de 49 % — et elle tient parce que
l'attestation par le harnais eval, recalibrée à ≈ 180 lignes sur son voisin
`test_ready_label_blocked_skip.rs` (180 lignes mesurées) plutôt que sur l'estimation optimiste de
≈ 90 du vol 1, part en vol 3 (D8). Les garder ensemble donnerait ≈ 323 lignes, au-dessus de la
borne.

`crates/mika-agent/CLAUDE.md` (D9) est hors du compte : la consigne exclut `docs/` et
`CLAUDE.md`.

Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

---

## Acceptance criteria

- **AC1** — `grep -rn "send_notification_p0" crates/` ne retourne **aucune** occurrence. L'aide du
  vol 1 est supprimée des deux handlers, sans `#[deprecated]` ni alias.
- **AC2** — Les comptes par fichier et par tier sont ceux de D3 : dans
  `crates/mika-agent/src/server/verdict_handler.rs`, 13 appels à `send_notification` dont 5
  passent `NotificationSeverity::P0` et 8 passent `NotificationSeverity::P2` ; dans
  `crates/mika-agent/src/server/ci_failure_handler.rs`, 3 appels dont 1 `P0` et 2 `P2`. Aucun site
  n'a changé de tier par rapport à la table du vol 1.
- **AC3** — La signature est `send_notification(sender: &Arc<dyn MessageSender>, severity: NotificationSeverity, message: &str)`
  dans les deux handlers — la sévérité en **deuxième** position (D1), et le corps appelle
  `sender.send_with_severity(message, severity)`.
- **AC4** — `mika1381_un_p0_de_handler_natteint_jamais_la_surface_mutable` passe par l'aide unique
  avec `NotificationSeverity::P0` et assert que la frontière a observé `"p0"`. Rouge-avant
  vérifié : remplacer `P0` par `P2` au site d'appel du test fait rougir le test.
- **AC5** — `mika1381_un_p2_de_handler_nescalade_jamais` passe par l'aide unique avec
  `NotificationSeverity::P2` et assert `"p2"`. Rouge-avant vérifié symétriquement.
- **AC6** — Un test nouveau dans `ci_failure_handler.rs` § `tests` appelle `surface_for_adoption`
  sur le chemin qui notifie et assert que `RecordingSender` a observé la paire
  `(texte, "p2")` pour le site `:720`. **Borne de couverture déclarée** : c'est le seul des seize
  sites attesté à son point d'appel réel ; les sites P0 de ces deux handlers sont hors
  d'atteinte d'un test unitaire (`try_handle_ci_failure` exige un `github_token` et émet des
  appels API ; `verdict_handler` n'a aucun point d'entrée testé — D6). Le site de calage
  `agent_loop/mod.rs:4337` est attesté au vol 3 (D8).
- **AC7** — `RecordingSender` (`ci_failure_handler.rs`) conserve `messages() -> Vec<String>` à
  l'identique et gagne `observe() -> Vec<(String, String)>`. Les dix tests T1 à T8 qui appellent
  `messages()` passent **sans modification** — vérifié en diff : aucune ligne de ces tests ne
  bouge.
- **AC8** — `cargo test -p mika-agent` est intégralement vert, y compris les deux détecteurs
  structurels que ce vol ne doit pas déranger :
  `mika1745_t3_the_only_call_site_is_the_no_task_arm` (`ci_failure_handler.rs:1837`) et
  `mika2671_b2_la_garde_de_revue_precede_le_tour` (`qa_head_supersession.rs:664`).
- **AC9** — `crates/mika-agent/CLAUDE.md` § *MessageSender Trait* décrit la forme à une aide et
  nomme la propriété obtenue (un nouveau site de notification ne compile pas sans choisir sa
  sévérité). Aucune mention résiduelle de `send_notification_p0` ne subsiste dans le fichier.
  `crates/mika-common/tests/claude_md_size.rs` reste vert.
- **AC10** — Aucun doc-comment ne renvoie plus la fusion à un vol futur. En particulier les
  phrases du vol 1 *« The merge into one helper carrying an explicit severity is vol 2 »* et
  *« one copy, so the two helpers above cannot drift »* sont supprimées ou corrigées : une
  justification morte se lit comme une prescription vivante.
- **AC11** — Le doc-comment de `SeverityRecordingSender` énonce le nouveau rôle de
  `SEVERITE_ABSENTE` : l'aide n'appelle plus jamais `send`, donc une observation de cette
  constante est le témoin d'une fuite de sévérité, pas un échafaudage de test (D4).
- **AC12** — `docs/operator/environment-variables.md` est inchangé par ce vol (D9), et le
  comportement observable en production l'est aussi : aucune notification ne change de
  destination.

## Definition of Done

- [ ] `cargo build` et `cargo clippy --all-targets` sans avertissement nouveau.
- [ ] `cargo fmt --check` propre.
- [ ] `cargo test -p mika-agent` vert en intégralité (AC8).
- [ ] `cargo test` vert sur l'espace de travail.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] Les douze AC ci-dessus vérifiés un par un, chacun par la commande ou le diff qu'il nomme.
- [ ] Les deux rouge-avant d'AC4 et AC5 réellement exécutés et constatés rouges avant d'être
      remis au vert — un test de sévérité qui n'a jamais rougi n'atteste rien.
- [ ] Corps de PR portant `Part of #1381` (jamais `Closes`), les deux lignes `Tracked in:` (vol 3
      et axe INTENT_GUARDS), et `Code review: delegated to mika-qa (mika#2694)`.
- [ ] Aucun fichier hors des quatre cibles : `verdict_handler.rs`, `ci_failure_handler.rs`,
      `crates/mika-agent/CLAUDE.md`, et ce plan.

---

## Risques

- **R1 — La migration des seize sites est mécanique, donc elle se relit mal.** Seize appels de la
  même forme changent dans le même diff ; un site passant `P2` au lieu de `P0` a exactement
  l'aspect de ses voisins. *Atténuation* : la table de D3 est le référentiel, AC2 compte les
  tiers par fichier plutôt que de les relire, et la position de la sévérité choisie par D1 la met
  au même endroit sur les seize sites — une anomalie devient une anomalie de colonne.
- **R2 — Treize sites sur seize restent tenus par la seule revue.** AC6 déclare la borne, mais la
  déclarer ne la supprime pas. *Atténuation* : le compilateur garantit qu'aucun site n'est
  *oublié* ; ce qui reste non attesté est seulement le *choix* de tier, et ce choix a déjà été
  arbitré site par site au vol 1 et n'est pas rouvert ici (D3). Le risque résiduel est donc
  celui d'une faute de transcription, que R1 couvre.
- **R3 — Un test tautologique survivant à la réécriture.** Si les tests de D4 étaient portés
  mécaniquement sur la nouvelle signature sans changer leur assertion, ils resteraient verts et ne
  prouveraient plus rien. *Atténuation* : les rouge-avant d'AC4 et AC5 sont exigés explicitement
  dans la DoD ; un test tautologique ne rougit pas quand on inverse la sévérité au site d'appel,
  ce qui le démasque immédiatement.
- **R4 — L'extension de `RecordingSender` faisant rougir les tests T1 à T8.** *Atténuation* : D7
  impose l'addition d'un accesseur plutôt que la modification de `messages()`, et AC7 le vérifie
  en diff — aucune ligne de ces dix tests ne doit bouger.
