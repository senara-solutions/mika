# mika#2866 — Un site d'émission P0 pour la classe `recovery-runbook`

> Suivi du vol 4/4 de mika#1381. Le rail de sévérité est livré et attesté ; ce
> plan n'y touche pas. Il **crée** le site qui manque.

## Contexte

`crates/mika-agent/src/messaging.rs` porte `NotificationSeverity::{P0, P2}` et
`MessageSender::send_with_severity`. Quatre classes P0 sont nommées par le corps
de mika#1381 : `dispatch-stall` (site : `agent_loop/mod.rs:4337`),
`block[security]` (site : `server/verdict_handler.rs`), `recovery-runbook` et
`fabrication-detected`. Les deux dernières **n'ont aucun site d'émission** —
rien à classifier, un site à créer. C'est une extension de périmètre, et c'est
pourquoi mika#1381 l'a sortie de son vol.

Ce plan traite `recovery-runbook` seule. `fabrication-detected` reste sans site
et sans ticket d'implémentation nommé ici — voir § *Hors périmètre*.

## Arbitrage de la décision 1 du ticket — quel événement ouvre la classe

« Un runbook de récupération devrait tourner » n'est pas un état que le moteur
porte. Le ticket offre quatre candidats ; aucun n'était le bon, et l'arbitrage
est ce que ce plan apporte avant la première ligne de code.

| candidat | lu | écarté parce que |
|---|---|---|
| une tâche `blocked` au-delà d'un seuil | `db/tasks.rs:1351-1620`, signal `stale_blocked` de `get_task_health_summary` | déjà injecté comme `<task-health>` dans le prompt du heartbeat : l'opérateur n'est pas le destinataire, l'agent l'est. Et `blocked > 24 h` n'est pas terminal — la tâche peut se débloquer seule. **Aucun runbook n'est dû.** |
| un `dispatch_failures` répété | signaux #7/#8 de `get_task_health_summary` (`dispatch_failures`, `dispatch_stale`, mika#980) | **échoue la décision 2.** Cette population est celle que la classe `dispatch-stall` couvre déjà (`agent_loop/mod.rs:4292-4342`). Deux classes P0 sur un même wedge rendent le compte de l'opérateur illisible — exactement la classe mika#2205 que la décision 2 interdit. |
| un échec de `deploy_mika` | `self-dev-callback` | l'échec est déjà rendu dans le fil de la conversation que l'opérateur a lui-même ouverte en demandant le déploiement. Il n'y a pas de dormance : personne n'attend d'être réveillé pour un déploiement qu'il vient de lancer. |
| une base en migration coincée | `db/migrations.rs` | une migration coincée tue le démarrage du processus. Le moteur n'est alors pas en état d'émettre quoi que ce soit, et `GatewayMessageSender` n'existe pas encore. Non atteignable par construction. |
| verdicts de vol du milestone-manager (`FlightRecommendation::ManualResumption`) | `milestone_manager/evaluator.rs:1-330` | module read-only en mode shadow, dont l'escalade a **son propre rail** (`MIKA_MANAGER_ESCALATION_URL`). Mélanger ce rail avec celui de mika#1381 est une décision d'architecture distincte, et non ce ticket. |
| `park_unverified` (mika#2286) | `wip_rescue.rs:1595-1673` | l'exclusion est **ré-armable** et se lève d'elle-même quand le marqueur passe à `yes`. Rien n'a mal tourné. Aucun runbook n'est dû. |

**Retenu : `wip_rescue::bail_to_human`** — le démon d'auto-reprise rend la main à
un humain sur un brouillon `wip-rescue`. Cinq propriétés, et chacune est une
raison séparée :

1. **Terminal par construction.** Le marqueur durable `wip_rescue_bailed` est
   écrit *avant* tout appel réseau (`bail_to_human:1548`, `mark_bailed:1683`) et
   le filtre d'éligibilité le lit (`has_bailed_marker:844`). Plus aucune
   automatisation ne touchera ce brouillon. C'est la définition d'un état qui
   attend un humain.
2. **Le contenu existe.** Le brouillon est né du sauvetage de worktree sale
   (mika#1282) : il porte du travail qui n'est nulle part ailleurs.
3. **Un runbook nommé existe déjà** —
   `.claude/commands/mika-rescue-drafts-process.md`, Phase 3 (vérifier, puis
   un-draft / tenir / remonter). Le site ne demande donc pas d'écrire un runbook,
   il demande de le **déclencher**.
4. **Les seuls signaux actuels sont un label et un commentaire GitHub** — et
   `bail_to_human` n'a aucun `sender` en portée. Un `wip_rescue_bail_to_human` en
   WARN dans le journal n'a jamais réveillé personne : c'est le diagnostic #4 de
   *Loop Trustworthiness*, l'alarme dans le vide. Mesuré : 17 bails le
   2026-09-05, dont 14 sur la seule PR #2197 entre 09:43 et 16:05
   (`wip_rescue.rs` § doc de module).
5. **La population est déjà soustractible** (décision 2, ci-dessous) : elle
   existe, en table, avec un écrivain unique. Aucune seconde table à ouvrir.

## Arbitrage de la décision 2 — le prédicat se soustrait

La question de l'opérateur est « combien de P0 `recovery-runbook` cette
semaine ? », et la réponse doit être **exactement** une population, jamais un
mélange.

**La table.** Elle existe et n'est pas à créer :

```sql
SELECT target_key, count(*)
FROM audit_events
WHERE tool_name = 'wip_rescue_bailed'
GROUP BY 1;
```

Écrivain unique `mark_bailed` (`wip_rescue.rs:1683`), une ligne par PR, écrite
avant tout appel réseau — donc une ligne même quand GitHub est injoignable. La
déduplication est **gratuite** : le marqueur est le filtre d'éligibilité, donc un
second bail sur la même PR n'est pas atteignable. Aucune table, aucune fenêtre de
rafraîchissement et aucun verrou en mémoire ne sont nécessaires, contrairement à
`agent_recurring_gate` (mika#2456 vol 3) dont la cadence n'était pas bornée.

**Le nom d'événement.** Le site émet `recovery_runbook_notified` (INFO, compteur),
portant `class = "recovery-runbook"`, `pr_number`, `severity = "p0"`. Il se
soustrait de `wip_rescue_bail_to_human` (déjà émis par `bail_to_human:1546`), et
c'est délibéré : les deux lignes répondent à deux questions différentes.

| ligne | répond à |
|---|---|
| `wip_rescue_bail_to_human` (WARN) | *le démon a-t-il bailé ?* |
| `recovery_runbook_notified` (INFO) | *la notification P0 est-elle partie ?* |

En régime nominal les deux comptes sont **égaux**, et **leur écart est le
signal** : un bail sans notification est précisément le diagnostic #4 que ce
ticket ferme. Un seul nom pour les deux faits rendrait cet écart inobservable
(classe mika#2205). L'écart a son propre nom quand il est causé par l'absence de
rail : `recovery_runbook_notification_unroutable` (WARN), § D5.

## Arbitrage de la décision 3 — le site nomme son étage, sans défaut

La forme à reprendre est celle de `verdict_handler.rs:2722-2752` et de
`ci_failure_handler.rs` : une aide **privée-module**, une par module, sévérité en
**seconde** position et sans défaut.

```rust
async fn send_notification(
    sender: &Arc<dyn MessageSender>,
    severity: NotificationSeverity,
    message: &str,
) {
    report_notification_outcome(sender.send_with_severity(message, severity).await, severity);
}
```

Trois propriétés, reprises telles quelles :

- **Pas de défaut** ⇒ « oublié de classifier » n'est pas un état atteignable ;
  un futur site de ce module ne compile pas sans nommer son étage.
- **Seconde position** ⇒ le message est le dernier argument, parce qu'il est un
  `format!` multi-lignes et qu'un argument en fin de bloc est une ligne qu'un
  relecteur survole.
- **Une seule aide par module.** Pas de seconde aide, pas d'enveloppe qui
  remplirait l'étage : ajouter l'une ou l'autre rend l'état interdit atteignable
  à nouveau.

`report_notification_outcome` est repris aussi : `Delivered` ⇒ rien ;
`Failed`/`NoChannel`/`Err` ⇒ un WARN nommé portant `severity.as_str()`. Un P0 qui
n'est pas délivré ne doit pas se taire.

**Décision négative, explicite :** les deux aides existantes **ne sont pas**
promues en fonction partagée. Elles sont privées-module par doctrine
(`crates/mika-agent/CLAUDE.md` § *MessageSender Trait* : « each of
`verdict_handler.rs` and `ci_failure_handler.rs` therefore owns **exactly one**
module-private `send_notification` »). Une troisième copie privée est le geste
conforme ; une extraction est une refonte du rail, hors périmètre.

## Arbitrage de la décision 4 — attestation au transport

`send_with_severity` a un **corps par défaut** qui délègue à `self.send(text)`.
Un site qui retomberait sur `send()` perdrait son étage sans aucune erreur, et
`GatewayMessageSender::send` délègue à `send_with_severity(.., P2)` — donc le P0
atterrirait sur le chat que l'opérateur peut muter, en rendant `Delivered`.
**Aucune assertion d'issue ne peut tenir cet invariant.**

Le motif existe (`tests/eval/test_dispatch_stall_p0_1381.rs`) : un double
surchargeant les **deux** entrées, et une sentinelle `SEVERITE_ABSENTE` poussée
par `send` — observer la sentinelle est le témoin d'une fuite de sévérité.

**Ce plan atteste au niveau unitaire, dans `wip_rescue.rs § tests`, et non
dans `tests/eval/`.** Raison : la chaîne de sauvetage entière (sélection →
`RESCUE_DEPTH` → garde git ≥ 2.38 → `merge-tree` → rebase → clippy → push →
périmètre) exige `gh`, un `git` réel et un checkout — un test `eval` ne peut pas
la traverser sans réseau. Le découpage de D2 est ce qui rend l'attestation
possible **sans** réseau : l'aide prend une valeur `ChainOutcome` déjà construite.
Le chemin attesté est donc exactement « cet état de chaîne ⇒ cette notification
portant cet étage au transport », et le seul segment non couvert — la production
de `ChainOutcome::Bailed` par `bail_to_human` — l'est déjà par trois tests
existants (`wip_rescue.rs:2401`, `:2439`, `:2468`).

## Décisions

**D1 — Le rail atteint déjà la surface jamais muette ; rien à câbler.**
`server/mod.rs:659-672` construit `engine_sender` avec
`.with_escalation_chat_id(agent_settings.operator_escalation_chat_id)` et le passe
en `message_sender: Some(engine_sender)` au `TaskDispatcher` (`:703`) — le
commentaire en place dit pourquoi (« the task-engine rail carries the
dispatch-stall P0 … so it must know the escalation chat too (mika#1381) »). Le
scan `wip-rescue` tourne sur ce dispatcher. Aucune variable d'environnement,
aucune construction de sender, aucune migration : c'est ce qui rend ce vol petit.

**D2 — L'émission vit dans `auto_resume_wip_rescue_drafts`, pas dans
`bail_to_human`.** Faire descendre un `sender` jusqu'à `bail_to_human` traverse
`resume_chain` et touche plusieurs signatures pour plusieurs sites d'appel.
À la place, on capture l'issue **avant** le `match` existant (`:690-730`) :

```rust
    let outcome = resume_chain(db, github_token, label_auth, trace_id, session_id, &pr, age_secs).await;

    notify_if_bailed(sender, pr.number, &outcome).await;

    match outcome {
        // les quatre bras, inchangés
    }
```

Trois conséquences voulues : le type de retour reste `Option<usize>` (tous les
tests existants restent intacts), `notify_if_bailed` reçoit une **valeur** donc
est testable sans réseau, et le `match` par étage de l'aide est **exhaustif** sur
`ChainOutcome` — un cinquième état d'issue ne compilera pas sans que son auteur
décide s'il réveille l'opérateur.

**D3 — `auto_resume_wip_rescue_drafts` gagne exactement un paramètre :**
`sender: Option<&Arc<dyn MessageSender>>`. Un seul appelant de production,
`task_engine/dispatcher.rs:2338`, qui passe `self.message_sender.as_ref()`
(`pub message_sender: Option<Arc<dyn MessageSender>>`, `:640`). `Option` et non
`&Arc` : les agents du moteur d'équipes portent délibérément
`message_sender: None`, et le CLI aussi.

**D4 — Le message nomme le runbook, la PR, la raison, et dit si le label a
atterri.** `ChainOutcome::Bailed.parked` dit si l'écriture GitHub a réussi
(`:628-637`) — un opérateur qui part chercher le label sur GitHub ne le trouvera
pas quand `parked == false`, donc le message le dit plutôt que de le ravaler.
Forme :

```
🚨 recovery-runbook — wip-rescue a rendu la main

PR #<n> — le démon d'auto-reprise a bailé vers un humain. Terminal : plus aucune
automatisation ne touchera ce brouillon (mika#2199).

Raison : <reason>
Label `human-review-required` : posé | NON POSÉ (l'écriture GitHub a échoué)

Runbook : /mika-rescue-drafts-process
```

Le commentaire déjà posé sur la PR par `bail_to_human:1558` n'est pas dupliqué :
il porte le détail, la notification porte le réveil.

**D5 — Un `sender` absent est nommé, jamais silencieux.**
`notify_if_bailed` avec `sender == None` sur un `Bailed` émet
`recovery_runbook_notification_unroutable` (WARN, champs `pr_number`,
`class = "recovery-runbook"`). Sans cette ligne, « aucun bail » et « un bail dont
la notification n'avait aucun rail » rendent des octets identiques (classe
mika#2205), et c'est précisément le défaut que ce ticket ferme.

**D6 — Aucune ligne d'audit nouvelle, et c'est une décision.** La table de la
classe est `wip_rescue_bailed`, déjà écrite par un écrivain unique avant tout
appel réseau. Une ligne `recovery_runbook_notified` en `audit_events` ouvrirait
une seconde population pour le même fait — ce que la décision 2 interdit. Le
compteur vit dans le journal structuré ; la population vit dans la table qui
existe.

**D7 — Pas de constante publique pour le nom de classe.**
`const RECOVERY_RUNBOOK_CLASS: &str = "recovery-runbook"` reste privée au module :
un seul site l'écrit, et une constante publique inviterait un second émetteur
sans que personne arbitre la décision 2 pour lui.

## Implémentation

Un seul vol, un seul ordre de commit.

### Pas 1 — `crates/mika-agent/src/wip_rescue.rs`

1. Imports : `use crate::messaging::{MessageSender, NotificationSeverity};` et
   `use std::sync::Arc;`.
2. `const RECOVERY_RUNBOOK_CLASS: &str = "recovery-runbook";`, avec la requête
   opérateur de la décision 2 dans son doc-comment.
3. `send_notification(sender, severity, message)` + `report_notification_outcome`,
   privées-module, dans la forme verbatim de `verdict_handler.rs:2722-2752`
   (§ décision 3).
4. `notify_if_bailed(sender: Option<&Arc<dyn MessageSender>>, pr_number: u64,
   outcome: &ChainOutcome)` — `match` **exhaustif** : `Bailed { reason, parked }`
   construit le message de D4 et appelle
   `send_notification(s, NotificationSeverity::P0, &msg)`, puis émet
   `recovery_runbook_notified` ; `Resumed(_) | ParkedUnverified | Skipped(_)` ⇒
   `return`, chaque bras commenté avec la raison pour laquelle aucun runbook
   n'est dû (§ arbitrage de la décision 1). `sender == None` sur `Bailed` ⇒ D5.
5. Signature de `auto_resume_wip_rescue_drafts` : un paramètre
   `sender: Option<&Arc<dyn MessageSender>>`.
6. Corps : `let outcome = resume_chain(…).await;` puis
   `notify_if_bailed(sender, pr.number, &outcome).await;` puis le `match outcome`
   existant, inchangé.
7. Doc de module : une ligne au diagramme de chaîne disant que le bail émet
   désormais un P0 de classe `recovery-runbook`, et que le marqueur durable est
   la population de cette classe.

### Pas 2 — `crates/mika-agent/src/task_engine/dispatcher.rs`

8. `:2338` — passer `self.message_sender.as_ref()` en dernier argument.

### Pas 3 — Attestation (`wip_rescue.rs § tests`)

9. `SEVERITE_ABSENTE` + `SeverityRecordingSender` surchargeant **les deux**
   entrées du trait (motif `verdict_handler.rs:5071`, `test_dispatch_stall_p0_1381.rs`).
10. `mika2866_un_bail_reveille_loperateur_au_palier_p0` — un `Bailed` ⇒
    exactement un franchissement, dont la sévérité observée **vaut** `"p0"`.
    Assertion sur la **valeur**, pas sur deux `!=` : seul `send_with_severity`
    rend `"p0"`, donc un passage par `send()` rend la sentinelle et rougit.
11. `mika2866_une_reprise_ne_reveille_personne`,
    `…_un_park_ne_reveille_personne`, `…_un_skip_ne_reveille_personne` — zéro
    franchissement. C'est la forme exécutable de la décision 2 : la classe se
    soustrait des trois autres issues de chaîne.
12. `mika2866_un_label_non_pose_est_dit_dans_le_message` — `parked: false` ⇒ le
    message porte la mention « NON POSÉ » (D4).
13. `mika2866_un_bail_sans_rail_est_nomme` — `sender == None` sur un `Bailed` ⇒
    `notify_if_bailed` rend la main sans panique (le WARN de D5 est la trace ; le
    test tient l'absence de perte silencieuse).
14. En-tête du module de test : la recette *rouge-avant* — remplacer
    `send_with_severity` par `send` dans l'aide, ou `P0` par `P2`, et nommer quel
    test rougit.

## Fire-Disposition

**Sans objet pour ce vol — aucun détecteur n'est livré.** La section n'est pas
inventée (mika#2306), et les candidats sont écartés un par un :

- **Les cinq tests du pas 3 ne sont pas des détecteurs.** Un détecteur a une
  population qu'il balaie et dont il peut découvrir une violation inconnue. Ces
  tests composent eux-mêmes l'état qu'ils examinent (un `ChainOutcome` littéral)
  et n'exercent que le chemin qu'ils ont écrit. Doctrine établie au vol 3 de
  mika#1381 : un contrôle négatif comportemental n'a **pas** de population.
- **La sentinelle `SEVERITE_ABSENTE` est une valeur de journal, pas un
  détecteur** — c'est un témoin observé dans un test, pas un balayage.
- **Aucun scan de source n'est livré ni modifié.** Rien ne compte les sites de
  notification aujourd'hui, et ce vol n'ouvre pas ce compte. Le scan le plus
  proche, `qa_head_supersession.rs:664`, cherche le littéral
  `"GatewayMessageSender::new("` : ce vol n'en construit aucun (D1), donc il
  n'est ni touché ni à exempter.
- **Aucune règle de lint, aucune garde CI, aucun validateur de schéma.** Rien
  sous `scripts/`.
- **Aucune ligne `unwired`.** Les tests atterrissent dans
  `crates/mika-agent/src/wip_rescue.rs § tests`, une cible déjà atteinte par
  `cargo test -p mika-agent`, donc `scripts/check-suite-wiring.sh` ne gagne
  aucune entrée (le champ `unwired=<n>` reste à zéro depuis mika#2793).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/wip_rescue.rs` (aide + site + doc de module, hors tests) | 70 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` (un argument au seul appelant) | 3 |
| tests (`crates/mika-agent/src/wip_rescue.rs § tests`) | 140 |

Total estimé : 213 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 213 < 279, et la marge est réelle plutôt que
rhétorique pour trois raisons structurelles : aucune migration de schéma (D6),
aucune nouvelle construction de sender ni variable d'environnement (D1), et
aucune signature modifiée au-delà d'un paramètre pour un appelant unique (D3) —
donc aucun tour dépensé à réparer des sites d'appel. Les 140 lignes de test sont
cinq cas sur un module de test déjà outillé, dont le double de sender est une
reprise verbatim d'un motif existant.

## Acceptance criteria

1. **AC1** — `wip_rescue.rs` porte une aide **unique** privée-module
   `send_notification(sender, severity, message)`, sévérité en seconde position et
   **sans valeur par défaut** ; aucune seconde aide, aucune enveloppe remplissant
   l'étage, aucun défaut d'argument.
2. **AC2** — un `ChainOutcome::Bailed` produit **exactement une** notification,
   émise par `send_with_severity` au palier `NotificationSeverity::P0`, et le test
   l'observe **au transport** (double surchargeant `send` *et*
   `send_with_severity`), avec assertion sur la **valeur** `"p0"` — pas sur une
   inégalité.
3. **AC3** — `Resumed`, `ParkedUnverified` et `Skipped` produisent **zéro**
   notification, chacun couvert par un test nommé ; le `match` de
   `notify_if_bailed` est exhaustif sur `ChainOutcome` (aucun bras `_`).
4. **AC4** — le message nomme la PR, la raison, le runbook
   `/mika-rescue-drafts-process`, et dit si le label `human-review-required` a
   atterri ; un `parked: false` est couvert par un test.
5. **AC5** — la population de la classe est **exactement**
   `audit_events WHERE tool_name = 'wip_rescue_bailed'` ; **aucune table n'est
   créée**, aucune ligne d'audit nouvelle n'est écrite, et la requête opérateur
   figure dans le doc-comment de `RECOVERY_RUNBOOK_CLASS`.
6. **AC6** — le site émet `recovery_runbook_notified` (INFO) portant `class`,
   `pr_number`, `severity` ; un `sender` absent sur un `Bailed` émet
   `recovery_runbook_notification_unroutable` (WARN) et ne panique pas.
7. **AC7** — `auto_resume_wip_rescue_drafts` conserve son type de retour
   `Option<usize>` et sa sémantique (`Some(1)` repris, `Some(0)` examiné non
   repris, `None` rien d'éligible) ; **tous les tests existants de `wip_rescue.rs`
   passent sans modification de leurs assertions**.
8. **AC8** — `task_engine/dispatcher.rs:2338` passe `self.message_sender.as_ref()`,
   et reste le **seul** appelant de production.
9. **AC9** — aucun `GatewayMessageSender` n'est construit par ce vol ; aucune
   variable d'environnement n'est ajoutée ; aucune migration de schéma.
10. **AC10** — l'en-tête du module de test porte la recette *rouge-avant* : quelle
    mutation (`send_with_severity` → `send`, `P0` → `P2`) fait rougir quel test.
11. **AC11** — la doc de module de `wip_rescue.rs` dit que le bail émet un P0 de
    classe `recovery-runbook` et que le marqueur durable **est** la population de
    cette classe.

## Definition of Done

- [ ] `cargo build` vert.
- [ ] `cargo test -p mika-agent` vert, en particulier le module
      `wip_rescue.rs § tests` en entier (anciens tests inclus, non modifiés).
- [ ] `cargo clippy --all-targets -- -D warnings` vert (`sig_drop` inclus).
- [ ] `cargo fmt --check` vert.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] `make verify-bundled-skills` vert (aucun bundle touché, le gate reste vert).
- [ ] La recette *rouge-avant* a été **exécutée** une fois à la main : l'aide
      repassée par `send()` fait rougir AC2, et `P2` à la place de `P0` aussi.
- [ ] Les onze AC sont énumérées une par une dans le corps de la PR, chacune avec
      son élément de preuve (nom de test, chemin:ligne).
- [ ] `Code review: delegated to mika-qa (mika#2694)` dans le corps de la PR.
- [ ] Aucune invocation de `ce-simplify-code` ni de `ce-code-review` (mika#2694).

## Risques

| risque | portée | parade |
|---|---|---|
| Le P0 réveille l'opérateur plus souvent que voulu | 17 bails mesurés sur une seule journée (2026-09-05), dont 14 sur une même PR | la déduplication est **structurelle** : le marqueur `wip_rescue_bailed` est le filtre d'éligibilité, donc une PR ne baile qu'une fois. Les 14 de ce jour-là précèdent mika#2199 et ne sont plus atteignables. Le volume attendu est un P0 par PR baillée, jamais par tick. |
| L'étage se perd silencieusement à une refonte future de l'aide | le défaut de `send_with_severity` délègue à `send` et rend `Delivered` | AC2 observe au transport, avec assertion sur la valeur `"p0"` ; la sentinelle `SEVERITE_ABSENTE` rougit sur une fuite. Recette *rouge-avant* exécutée (DoD). |
| Un cinquième état de `ChainOutcome` arrive et personne ne décide s'il réveille | `ChainOutcome` est privé-module et peut gagner un bras | le `match` de `notify_if_bailed` est exhaustif, sans bras `_` : le compilateur refuse le nouvel état jusqu'à ce que son auteur tranche (AC3). |
| Le `sender` est `None` en production et la classe ne part jamais | agents du moteur d'équipes et CLI portent `None` par conception | D1 établit par lecture que le `TaskDispatcher` du serveur porte `Some(engine_sender)` **avec** le chat d'escalade (`server/mod.rs:659-703`), et D5 nomme le cas `None` au lieu de le taire. |
| Deux populations non soustractibles dans le journal | `wip_rescue_bail_to_human` et `recovery_runbook_notified` décrivent le même bail | les deux noms répondent à deux questions distinctes (§ décision 2) et leur **écart** est le signal recherché. Un seul nom rendrait cet écart inobservable. |

## Hors périmètre

- **Le rail de sévérité lui-même** : livré et attesté par mika#1381 vols 1 à 4.
- **La classe `fabrication-detected`** (#11 de l'inventaire mika#1381) reste sans
  site d'émission. Elle n'est pas traitée ici et aucune ligne de ce vol ne la
  prépare : son prédicat est une décision de produit distincte, de la même nature
  que l'arbitrage de la décision 1 ci-dessus. Tracked in: senara-solutions/mika#1381
  (vol 4/4, dont ce ticket est le premier des deux suivis).
- **Le `GatewayMessageSender` de `server/handlers.rs:2369`**, construit **sans**
  `with_escalation_chat_id` contrairement à `:1731-1743`. Observé en lisant D1, et
  délibérément non corrigé : aucun site P0 n'emprunte ce sender aujourd'hui, et le
  réparer au passage mêlerait une correction de câblage non attestée à la création
  d'un site. À ouvrir en ticket propre si un P0 doit un jour partir de ce chemin.
