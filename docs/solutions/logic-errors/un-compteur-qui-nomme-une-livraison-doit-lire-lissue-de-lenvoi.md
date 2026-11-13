---
title: Un compteur qui nomme une livraison doit lire l'issue de l'envoi — une aide fire-and-forget le rend aveugle
date: 2026-10-21
category: logic-errors
module: crates/mika-agent/src/wip_rescue.rs
problem_type: logic_error
component: tooling
symptoms:
  - "L'INFO recovery_runbook_notified part même quand le P0 revient Failed, NoChannel ou Err"
  - "Les comptes recovery_runbook_notified et wip_rescue_bail_to_human restent égaux alors que personne n'a été réveillé"
  - "Toute la suite est verte, rouge-avant sur l'étage compris (send_with_severity vers send, P0 vers P2)"
root_cause: logic_error
resolution_type: code_fix
severity: high
tags:
  - notification
  - send-outcome
  - fire-and-forget
  - observability
  - mika-2205
  - test-double
  - wip-rescue
---

# Un compteur qui nomme une livraison doit lire l'issue de l'envoi — une aide fire-and-forget le rend aveugle

## Problem

mika#2866 (PR mika#2869) crée le site d'émission P0 de la classe
`recovery-runbook` : quand le démon wip-rescue rend la main à un humain,
`notify_if_bailed` réveille l'opérateur. Le site émet un compteur INFO,
`recovery_runbook_notified`, que le plan et le doc-comment de
`RECOVERY_RUNBOOK_CLASS` définissent comme la réponse à *la notification P0
est-elle partie ?*. Son **écart** avec le WARN `wip_rescue_bail_to_human` est
le signal d'un bail qui n'a réveillé personne. Le wip sauvé émettait ce
compteur **après** une aide qui ne disait pas si l'envoi avait réussi, si bien
que l'écart ne pouvait jamais apparaître.

## Symptoms

- Un P0 revenu `Failed`, `NoChannel` ou `Err` comptait comme parti : un WARN
  de livraison partait bien, mais le compteur montait quand même.
- Les deux comptes étaient égaux par construction. « Tout bail a réveillé
  quelqu'un » et « aucun bail n'a réveillé personne » rendaient les mêmes
  octets au compteur : c'est la classe mika#2205.
- Rien ne rougissait. Les six tests initiaux du site passaient, et les mutations
  rouge-avant exécutées sur l'étage (`send_with_severity` → `send`, `P0` →
  `P2`, site retiré) rougissaient toutes comme prévu.

## What Didn't Work

- **Le rouge-avant sur l'étage.** Il était juste et il a été exécuté, mais il
  attestait *quel étage* franchit le transport, pas *ce que le transport a
  rendu*. Le double de test (`SeverityRecordingSender`) ne savait rendre que
  `Ok(SendOutcome::Delivered)`, donc aucun test n'atteignait les trois bras
  d'échec de `report_notification_outcome`. Une mutation sur un chemin
  qu'aucun test n'emprunte ne peut pas rougir.
- **Lire la doctrine voisine comme une permission.**
  `docs/solutions/best-practices/observability-side-channel-must-not-be-able-to-change-the-verdict-2026-08-31.md`
  prescrit une signature sans `Result` (`fn record(&self, err: AuthBoundaryError);`)
  pour qu'un canal d'observation ne puisse pas changer le verdict qu'il observe.
  L'aide `send_notification` avait la même forme, `async fn … ()`. La forme
  était correcte pour l'*appelant qui décide*, et fausse pour le *compteur qui
  rapporte* : il ne pouvait plus rien savoir de l'issue.

Le défaut a été trouvé en revue (correctness et adversarial, indépendamment),
puis confirmé par le validateur, et non par la suite de tests.

## Solution

L'aide rend l'issue, et le compteur ne part que sur une livraison réelle.
`crates/mika-agent/src/wip_rescue.rs` :

```rust
// avant
async fn send_notification(sender: &Arc<dyn MessageSender>, severity: NotificationSeverity, message: &str) {
    report_notification_outcome(sender.send_with_severity(message, severity).await, severity);
}
// …
send_notification(sender, NotificationSeverity::P0, &message).await;
info!(pr_number, class = RECOVERY_RUNBOOK_CLASS, /* … */ "recovery_runbook_notified");

// après (wip_rescue.rs:746, :757, :857)
async fn send_notification(/* … */) -> bool {
    report_notification_outcome(sender.send_with_severity(message, severity).await, severity)
}
fn report_notification_outcome(/* … */) -> bool {
    match outcome {
        Ok(SendOutcome::Delivered) => true,
        Ok(SendOutcome::Failed { reason }) => { warn!(/* … */); false }
        Ok(SendOutcome::NoChannel) => { warn!(/* … */); false }
        Err(e) => { warn!(/* … */); false }
    }
}
// …
if send_notification(sender, NotificationSeverity::P0, &message).await {
    info!(pr_number, class = RECOVERY_RUNBOOK_CLASS, /* … */ "recovery_runbook_notified");
}
```

L'aide reste fire-and-forget pour le **flux** : l'appelant
`auto_resume_wip_rescue_drafts` ne propage rien, et son type de retour et ses
quatre bras sont inchangés. Seul le compteur lit le booléen.

Côté tests :

- le double prend un champ `issue: Option<SendOutcome>`, et
  `double_qui_echoue()` (`wip_rescue.rs:3634`) rend `Failed` ;
- `journal_de()` (`wip_rescue.rs:3685`) capture les octets rendus par un
  souscripteur `fmt`, installé via
  `test_utils::test_helpers::install_capturing_subscriber` (`test_utils.rs:64`),
  le site d'installation unique du crate (mika#2646) ;
- `mika2866_un_p0_non_delivre_ne_compte_pas_comme_notifie` (`:3944`) porte
  dans le même test un cas positif (délivré, donc la ligne est présente) et un
  cas négatif (`Failed`, donc la ligne est absente et le WARN de livraison est
  présent) ;
- `mika2866_seul_delivered_compte_comme_delivre` (`:3980`) passe les quatre
  issues, une par une ;
- chaque test qui atteint ces callsites porte `#[serial_test::serial]`, qu'il
  capture ou non.

Contrôle exécuté : avec la garde `if` retirée, le test du P0 non délivré
rougit ; une fois restauré, il est vert.

## Why This Works

Le compteur affirme un fait sur l'issue de l'envoi. Ce fait n'existe qu'à un
endroit, le `SendOutcome` rendu par le transport, et l'ancienne signature le
détruisait avant que le compteur ne s'exécute. Faire rendre un booléen à
l'aide remet l'information en portée du seul lecteur qui en a besoin, sans
rendre au flux d'appel la possibilité d'échouer. Les deux préoccupations sont
distinctes, et c'est ce qui les rend compatibles :

| question | qui la pose | ce que la signature doit permettre |
|---|---|---|
| l'échec d'envoi peut-il changer la décision ? | l'appelant (flux de chaîne) | **non** : rien à propager |
| l'envoi est-il parti ? | le compteur | **oui** : l'issue doit être lisible |

Un `()` répond à la première question et rend la seconde muette.

## Prevention

- **Un compteur dont le nom ou la doc affirme une livraison est gardé par
  l'issue de cette livraison**, jamais émis à la suite d'un appel qui rend
  `()`. Au moment d'écrire `info!("…_notified" | "…_sent" | "…_delivered")`,
  lire la ligne au-dessus : si l'appel ne rend rien, le compteur ment sur les
  chemins d'échec.
- **Un double de transport doit savoir échouer.** Un double qui ne rend que le
  chemin heureux rend tous les bras d'échec inatteignables, et toute mutation
  sur ces bras vacue. Paramétrer l'issue rendue (`Option<SendOutcome>` ici) dès
  la première écriture du double.
- **Le rouge-avant couvre chaque affirmation, pas seulement la plus visible.**
  Ici il y en avait deux : l'étage qui part, et le fait qu'il est parti. Seule
  la première avait sa mutation. C'est la règle
  `feedback_red_before_control_is_term_by_term` (auto memory [claude]) :
  contrôler terme par terme.
- **Asserter l'événement dans les octets rendus**, pas son absence de panique.
  Le test voisin `mika2866_un_bail_sans_rail_est_nomme`, corrigé dans le même
  vol, n'assertait rien. Ce cas est déjà documenté par
  `docs/solutions/best-practices/a-count-assertion-on-an-event-name-nothing-emits-is-always-green-2026-08-31.md`.
- **La capture tracing exige `#[serial]` sur les empoisonneurs comme sur les
  capturants.** Un test voisin qui atteint le callsite sans abonné en éteint
  l'`Interest` globalement. Voir
  `docs/solutions/test-failures/tracing-capture-la-fenetre-qui-casse-est-posterieure-a-linstallation-2026-10-02.md`.

## Related Issues

- senara-solutions/mika#2866 (le site), PR senara-solutions/mika#2869 (ce vol,
  ouvert en draft au moment de l'écriture).
- senara-solutions/mika#1381 : le rail de sévérité dont c'est le suivi.
- Classe mika#2205 (deux faits, mêmes octets) :
  `docs/solutions/architecture-patterns/2026-09-06-accesseur-etroit-a-cote-du-resolveur-canonique.md`,
  `docs/solutions/architecture-patterns/la-note-dun-refus-ne-peut-pas-valoir-preuve-2026-09-29.md`.
- Converse non contradictoire :
  `docs/solutions/best-practices/observability-side-channel-must-not-be-able-to-change-the-verdict-2026-08-31.md`.
- Même famille de faux vert :
  `docs/solutions/best-practices/trois-facons-dont-une-verification-passe-au-vert-sans-rien-mesurer-2026-09-22.md`.
