# Plan : aucun tour de LLM abouti ne se perd en silence (mika#2270)

- **Ticket :** senara-solutions/mika#2270
- **Type :** fix (p1, loop-substrate)
- **Surfaces :** `crates/mika-agent/src/server/a2a.rs`, `crates/mika-cli/src/remote_ask.rs`,
  `crates/mika-cli/src/commands/ask.rs`, `crates/mika-agent/src/a2a_db.rs` (commentaire + test de forme)
- **Date :** 2026-09-17

---

## Problème

Onze appels `mika ask` consécutifs sont rentrés vides côté appelant — `.content`
absent, **exit 0** — alors que le moteur avait produit la réponse complète et
l'avait écrite dans `/var/log/mika/server.log` (trace `d5887aa7`, 43 108 tokens
d'entrée, 28 s, `stop_reason=EndTurn`, corps intégral présent). Le verdict
d'architecte de mika#2266 a dû être récolté à la main dans le journal.

Le mode d'échec est silencieux : JSON bien formé, code de sortie zéro, `.content`
simplement absent. **Rien ne distingue « l'agent n'a rien à dire » de « la réponse
a été perdue ».** C'est une sonde qui ment sans échouer, sur le canal de retour
obligatoire de `/mika-groom-ticket` (porte architecte) et de `/mika-ask-prime`
(hook `check-question-routing.sh`).

---

## Mesures — lues dans le code le 2026-09-17, depuis le worktree de grooming

Le ticket donne quatre pistes et les dit non conclusives. Cinq lectures les
déplacent. Aucune n'a demandé la base de production.

**M1 — `message/send` détient la réponse et la jette ; `message/stream` la garde.**
`run_a2a_agent` (`server/a2a.rs:131`) rend `Result<Option<String>, String>` : le
texte du tour, en mémoire. Le chemin **stream** l'utilise directement
(`a2a.rs:791-806` : `Ok(response_text) => { let text = response_text.unwrap_or_else(|| "Task completed.".to_string()); … }`),
le pose dans le `StatusUpdate` et le sert. Le chemin **send** l'ignore
(`a2a.rs:420` : `Ok(_) => { … }`), marque la tâche `completed`, puis **rebâtit
une Task depuis la base** via `a2a_build_task` (`a2a.rs:437-448`). Même boucle,
deux portes, une seule garde la réponse en main. C'est la forme exacte du
« coût LLM payé et jeté » du ticket : la réponse était dans la main du processus
au moment où elle a été perdue.

**M2 — les « trois étages » du rendeur ne sont pas trois sources.**
`render_task_parts` (`remote_ask.rs:44-77`) lit artifacts → history(`role == Agent`)
→ `status.message`, et son commentaire annonce une défense en profondeur. Contre
ce mode de panne elle n'existe pas :

- **Étage 1 mort en production.** `a2a_insert_artifact` (`a2a_db.rs:394`) n'a
  **aucun appelant hors des tests de son propre module** — recherche exhaustive sur
  `crates/mika-agent/src` et `crates/mika-gateway/src`. mika-spirit n'écrit jamais
  d'artifact, donc `task.artifacts` est toujours `None`.
- **Étages 2 et 3 sont la même requête.** `a2a_build_task` (`a2a_db.rs:573-590`)
  calcule `status.message` comme *le dernier message de rôle `Agent` de `messages`*,
  et `messages` vient de `a2a_get_messages`. Si cette requête rend vide, `history`
  est `None` **et** `status.message` est `None`.

Conclusion : **les trois étages reposent sur un unique appel.** Il rend vide → la
Task est structurellement vide, et aucun étage ne peut secourir l'autre. Le rendeur
n'a rien à rendre ; chercher la cause chez lui — la piste principale du ticket —
ne pouvait aboutir.

**M3 — la requête qui décide, et ses deux paramètres.**
`a2a_get_messages` (`a2a_db.rs:289-328`) filtre
`session_id = ?1 AND role IN ('user','assistant') AND (trace_id = ?2 OR json_extract(metadata,'$.a2a_task_id') = ?2)`,
avec `?1 = a2a_task_map.session_id` et `?2 = a2a_task_id`. Son commentaire nomme
« deux producteurs » ; l'un des deux (`a2a_insert_message`) n'a, comme
`a2a_insert_artifact`, aucun appelant de production. **Le seul producteur réel est
la boucle d'agent, via `trace_id`** — `run_a2a_agent` pose
`AgentParams.trace_id = Some(task_id)` (`a2a.rs:194`) et le message assistant est
persisté sous ce `trace_id`. La restitution du contenu tient donc à un seul
prédicat, sur une seule colonne, écrite par un seul chemin.

**M4 — le timeout est hors de cause, et le ticket a raison sur ce point.**
`A2aClient::DEFAULT_TIMEOUT` vaut 300 s, le tour a duré 28 s, et un dépassement
produirait une erreur nommée par `transport_error_message` (mika#2036), pas un
exit 0. Le POST a rendu **200** : le client a reçu une Task, pas une panne.

**M5 — `Ok(None)` de `recover_by_context` n'est pas le chemin emprunté.**
La piste `remote_ask.rs:163` ne s'applique qu'après un échec de transport. Ici il
n'y en a pas eu. La Task est arrivée, `state = completed`, et la validation
d'état terminal (`remote_ask.rs:306-322`) l'a laissée passer — correctement. Le
vide est en amont du client.

---

## Rectification apportée à la direction du ticket

Le ticket désigne `render_task_parts` comme « surface suspecte ». M2 montre qu'elle
est **la victime, pas la cause** : un rendeur ne peut pas rendre ce que la Task ne
porte pas. Corriger le rendeur seul déplacerait le silence d'un cran sans rien
récupérer.

En revanche, la spec minimale du ticket est exactement la bonne, et elle ne dépend
d'aucune hypothèse causale : *si le moteur a rendu 200 avec un contenu, le CLI le
rend ou échoue en le nommant*. Ce plan la tient, et y ajoute la moitié que M1 rend
disponible **et gratuite** : sur `message/send`, le serveur n'a pas à espérer que la
base lui restitue une réponse qu'il tient déjà dans la main.

**Ce plan ne prétend pas expliquer pourquoi `a2a_get_messages` rend vide.** Cela
demande la base de production. Il rend cette perte (a) sans conséquence sur
`message/send`, (b) nommée partout ailleurs, (c) instrumentée pour être tranchée
à la première récidive. Voir § *Hors portée* et § *Conditions d'arrêt*.

---

## Conception

Trois briques. B1 et B2 ferment chacune un trou distinct et ne se remplacent pas :
B1 protège le chemin local `mika ask` → mika-spirit ; B2 protège tout chemin où le
serveur est plus ancien, distant (`--remote` via gateway) ou hors de ce dépôt.

### B1 — `message/send` cesse de jeter la réponse qu'il détient

Après la boucle, dans le bras `Ok` de `handle_message_send`, le texte retourné est
**conservé** au lieu d'être filtré par `Ok(_)`. La Task est rebâtie comme
aujourd'hui, puis, avant sérialisation :

| boucle | Task rebâtie | action |
|---|---|---|
| texte présent | porte du contenu | rien — chemin nominal, sortie **byte-identique** à aujourd'hui |
| texte présent | vide sur les trois tranches | **filet** : le texte est posé en `status.message` ; WARN `a2a_send_task_content_lost` + ligne `audit_events` |
| texte absent (`None`) | vide | `status.message` reçoit le **même littéral que le stream** (`"Task completed."`) ; aucun WARN de perte |
| texte absent | porte du contenu | rien |

**Pourquoi la quatrième ligne du tableau existe, et pourquoi elle n'est pas un
détail.** Elle est ce qui rend B2 utilisable sans faux positif. Aujourd'hui un
`.content` vide est compatible avec deux histoires incompatibles ; le serveur, lui,
sait laquelle — `run_a2a_agent` rend `Option<String>`. En garantissant qu'une Task
terminale servie par `message/send` porte toujours au moins une tranche non vide,
on **supprime la classe entière** « Task completed vide » de cette porte, et le CLI
peut alors traiter tout vide comme une perte sans jamais se tromper sur un tour
légitimement muet. Ce n'est pas une invention : `message/stream` sert ce littéral
depuis toujours (M1). C'est une **unification de deux portes sur une sémantique**,
et le littéral est extrait en une constante partagée par les deux sites.

**Le filet ne masque pas le défaut, et cette précaution est le cœur de la brique.**
Une réparation silencieuse rendrait la boucle verte et rendrait le défaut
définitivement invisible — ce serait construire la prochaine occurrence de
mika#2270. D'où le WARN, qui porte les faits permettant de trancher la cause à la
première récidive, et eux seuls : `task_id`, `context_id`, le `session_id` employé,
le `trace_id` passé à la boucle, le nombre de lignes `messages` pour cette session,
le nombre pour ce `trace_id`, la longueur du texte sauvé. Ces deux derniers comptes
séparent les deux hypothèses de M3 — « rien n'a été persisté » et « quelque chose a
été persisté sous un autre `trace_id` » — sans qu'un opérateur ait à ouvrir la base.

**Sole writer :** `a2a_send_task_content_lost` est écrit à ce seul site, en journal
et en `audit_events`. Son **absence** sous un symptôme est alors une information :
elle dit que la perte n'est pas là. Réutiliser le nom ailleurs détruirait
exactement cette propriété.

`returnImmediately` est **exempt** : cette branche ne fait jamais tourner la boucle
(`a2a.rs:342-344`), donc il n'y a pas de texte en main et pas de contenu à attendre.

### B2 — l'invariant de rendu : rendre, ou échouer en le nommant

`render_task_parts` rend `String`, et une chaîne vide y confond trois situations.
La fonction rend désormais un résultat qui les distingue, et la signature change
plutôt que d'ajouter une variante à côté : **c'est le compilateur qui force les
deux appelants** (`ask.rs:382` et `remote_ask.rs:355`) à traiter le cas, là où une
fonction parallèle laisserait l'ancienne en place pour le prochain lecteur.

- **contenu rendu** → inchangé, sortie byte-identique ;
- **la Task porte des tranches, mais aucune n'est lisible par ce rendeur** — un
  artifact aux `parts` vides, un contenu qui ne vit qu'en `role: User`, une variante
  de `Part` que ce rendeur ne connaît pas → **erreur**, jamais la chaîne vide ;
- **la Task ne porte structurellement rien** → **erreur**.

Les deux derniers cas produisent un exit non nul et un message qui nomme ce qui a
été inspecté (nombre d'artifacts, nombre de messages d'history et leurs rôles,
présence de `status.message`) plus `task_id` et `context_id` — les deux poignées
avec lesquelles retrouver le tour dans `$MIKA_SPIRIT_LOG_FILE`. Un message qui dit
« vide » sans dire ce qui a été regardé reproduirait le défaut à un niveau de bruit
près.

**Fail-closed, et l'asymétrie est mesurée.** Un faux positif coûte un message
d'erreur sur un cas qui n'a jamais été observé — et que B1 supprime sur la porte
locale. Un faux négatif coûte la boucle de développement cassée en silence,
onze fois en une session, plus une récolte manuelle dans un journal de 19 Go.

### B3 — la garde contre le retour de l'illusion

Le commentaire de `render_task_parts` affirme une redondance à trois étages que M2
réfute. Laissé tel quel, il renverra le prochain lecteur exactement où le ticket a
envoyé celui-ci. Il est corrigé pour dire ce qui est vrai : les trois tranches
descendent d'une seule requête, l'étage artifacts n'a pas de producteur côté
mika-spirit, et la redondance apparente ne protège de rien contre un tour perdu.

Plus un test de forme sur `a2a_build_task` qui épingle la dérivation
`status.message = dernier message agent de history`. Elle est aujourd'hui une
propriété du code et non un contrat écrit ; si elle diverge un jour, ce test le dit
au lieu de laisser l'étage 3 redevenir silencieusement un doublon de l'étage 2.

---

## Fire-Disposition

Aucune. Correctif de substrat sur le chemin de retour d'un tour ; pas de tâche
récurrente, pas de dispatch, pas de variable d'environnement nouvelle.

---

## Definition of Done

1. `handle_message_send` conserve le texte de `run_a2a_agent` et garantit qu'une
   Task terminale servie porte au moins une tranche non vide.
2. Le cas « boucle a produit du texte, Task rebâtie vide » sert le texte **et**
   émet `a2a_send_task_content_lost` (journal + `audit_events`), avec les six champs
   de diagnostic nommés en B1.
3. Le littéral de complétion est une constante unique partagée par `message/send`
   et `message/stream`.
4. `render_task_parts` distingue « rendu » de « rien de lisible » ; les deux
   appelants traitent le cas et propagent une erreur nommée.
5. `mika ask` ne sort plus jamais 0 avec `.content` absent, sur les deux formats
   (`text` et `json`) et sur les deux chemins (spirit local et `--remote`).
6. Le commentaire de `render_task_parts` décrit la dépendance réelle ; le test de
   forme sur `a2a_build_task` existe.
7. `cargo test`, `cargo clippy`, `cargo fmt` passent.

---

## Acceptance criteria

*Dérivés de la spec minimale du ticket (points 1 et 2), chacun testable.*

- **AC1** — Un tour dont la boucle a produit du texte et dont la Task rebâtie est
  vide sur les trois tranches fait servir ce texte par `message/send`. Test
  d'intégration serveur, sans réseau.
- **AC2** — Ce même cas émet `a2a_send_task_content_lost` exactement une fois, avec
  les champs `task_id`, `context_id`, `session_id`, `trace_id`, les deux comptes de
  lignes `messages` et la longueur du texte.
- **AC3** — Le cas nominal (Task porteuse de contenu) produit une réponse
  **byte-identique** à celle d'avant le correctif, et n'émet aucun WARN.
- **AC4** — Une `Task` `completed` dont le contenu vit dans une tranche que le
  rendeur ne lit pas fait **échouer** le rendu, pas rendre la chaîne vide. Trois
  tests négatifs distincts : contenu en `role: User` seul ; artifact aux `parts`
  vides ; message agent sans aucune `Part` textuelle. *(C'est la porte #2264 citée
  par le ticket.)*
- **AC5** — `mika ask --format json` sur une Task vide sort non-zéro et écrit une
  erreur nommant les tranches inspectées, `task_id` et `context_id` ; `--format
  text` fait de même. Aucune combinaison ne sort 0 avec `.content` absent.
- **AC6** — Un tour dont la boucle n'a produit aucun texte est servi avec le même
  littéral que `message/stream`, et **n'est pas** compté comme une perte.
- **AC7** — Le test de forme échoue si `a2a_build_task` cesse de dériver
  `status.message` du dernier message agent de `history`.

---

## Hors portée, délibérément

- **La cause racine de la perte** — pourquoi `a2a_get_messages` rend vide sur une
  session où la boucle a tourné. M3 la ramène à un unique prédicat sur `trace_id`,
  mais trancher entre « rien n'a été persisté » et « persisté sous un autre
  `trace_id` » demande la base de production. Ticket de suivi **conditionné à la
  première occurrence** de `a2a_send_task_content_lost`, qui portera précisément les
  deux comptes qui départagent. Ouvrir ce ticket avant d'avoir cette ligne serait
  instruire sans mesure.
- **Le lot 2 du ticket** — « un ping court a débloqué `mika-prime` ». Non
  reproductible, non expliqué par cette chaîne, et le ticket demande lui-même de ne
  pas en faire une conclusion. Aucun élément de ce plan n'en dépend.
- **`metadata.session_id` absent** — ce champ est inconditionnel dès que
  `--verbose` est passé (`ask.rs:420-424`), donc son absence dit soit que le drapeau
  n'y était pas, soit que l'observation portait sur le mode texte. Rien à corriger
  ici sans une trace d'invocation.
- **Écrire de vrais artifacts A2A** — l'étage 1 du rendeur est mort côté
  mika-spirit (M2). Le faire vivre est un travail de conformité au protocole, avec
  son propre périmètre ; ce plan constate l'absence et cesse de s'appuyer dessus.
- **Le chemin `message/stream`** — il garde déjà le texte en main (M1) et n'a pas
  le défaut. Il reçoit la constante partagée, rien de plus.

---

## Vérification

1. `cargo test -p mika-agent` et `cargo test -p mika-cli` — unités de rendu
   (AC4), unités serveur (AC1/AC2/AC3/AC6), test de forme (AC7).
2. `cargo test -p mika-agent --test eval` — non-régression de la boucle.
3. `cargo clippy --all-targets` et `cargo fmt --check`.
4. Contrôle manuel post-déploiement : un `mika ask --agent mika-arch --format json
   --verbose` sur un brief long rend du contenu ; `grep a2a_send_task_content_lost
   $MIKA_SPIRIT_LOG_FILE` est **vide** en régime nominal.
5. Surface SQL :
   `SELECT COUNT(*) FROM audit_events WHERE tool_name = 'a2a_send_task_content_lost';`

---

## Conditions d'arrêt

- **Le symptôme réapparaît et le WARN reste muet** → la perte n'est pas où ce plan
  la situe. **Halte.** Ne pas élargir le filet, ne pas ajouter de tranche au
  rendeur : rouvrir l'instruction côté client ou transport, avec la trace du POST.
- **Le WARN se déclenche en continu** → ce n'est plus un incident, c'est le régime
  nominal, et le filet masque une panne de persistance qui mérite son propre
  correctif. Traiter la cause, pas le seuil.
- **AC3 ne passe pas** — un cas nominal dont la sortie change — → le filet mord là
  où il ne devrait pas. Halte avant déploiement : un correctif de canal de retour
  qui altère les réponses saines est pire que le silence qu'il remplace.

---

## Voisinage

- **mika#2036** — la récupération par `context_id` après échec de transport. Cousin
  direct : même doctrine (« une réponse produite ne se perd pas »), classe de panne
  disjointe (transport échoué contre 200 vide). Ce plan reprend son vocabulaire de
  distinction des issues.
- **mika#2070** — l'adoption de la session de l'appelant, qui a rendu
  `a2a_get_messages` many-to-one et lui a imposé le filtre par `trace_id` de M3.
- **mika#1727** — `mika ask` devenu client mince : c'est ce qui place le tour côté
  spirit, et donc ce qui rend le canal de retour porteur.
- **mika#2069** — la même famille de défaut sur le canal de mesure : un jeu de
  données plus petit, plausible et faux rendu au lieu d'une erreur.
