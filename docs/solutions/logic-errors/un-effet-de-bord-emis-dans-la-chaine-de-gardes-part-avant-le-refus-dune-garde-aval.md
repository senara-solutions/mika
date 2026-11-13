---
module: crates/mika-agent/src/agent_loop/mod.rs, crates/mika-agent/tests/eval/test_fabrication_detected_p0_2867.rs
date: 2026-10-21
problem_type: logic_error
category: logic-errors
component: agent-core
severity: high
ticket: mika#2867
symptoms:
  - "Une notification P0 « Fabrication detected (uncorrected) … The answer went out » part pour une réponse qui n'est jamais sortie"
  - "Le test positif d'émission reste vert alors que la P0 porte sur une réponse remplacée par un re-prompt ultérieur"
  - "Une recette de rouge-avant prévue (retirer un terme du prédicat) reste verte"
root_cause: logic_error
resolution_type: code_fix
tags: [agent-loop, endturn, guard-chain, side-effect, notification, severity, p0, uncorrected, mock-fixture, red-before, mika-2867, mika-1381]
related_components: [testing_framework, messaging]
---

# Un effet de bord émis dans la chaîne de gardes part avant le refus d'une garde aval

## Problème

Les gardes `EndTurn` de `run_loop` sont évaluées en séquence, et chacune peut
refuser la clôture (`continue`, re-prompt). Un effet de bord **extérieur** posé au
milieu de cette chaîne, au moment où une garde *constate* quelque chose, part même
si une garde **plus bas** refuse ensuite la clôture. Le texte constaté n'est alors
jamais livré, et l'effet de bord atteste un fait faux.

Cas mesuré (mika#2867) : la branche `guard.assert_grounded_uncorrected`, placée
juste après la garde 6d, envoyait la P0 opérateur dès qu'elle constatait la
récidive. Six gardes en aval peuvent encore refuser la clôture : 6e
(equivalence-claim), 6f (send-failure non reconnu), #1313, #864, #901, #2037. La
relance de persistance `store_fact` peut faire de même. La notification disait
« The answer went out » pour une réponse que l'utilisateur n'a jamais reçue.

## Symptômes

- Une P0 part pour un tour dont la réponse finale ne contient plus la
  revendication.
- Le test positif d'origine était **vert pour une mauvaise raison** : la P0 qu'il
  observait portait sur une réponse jamais sortie.

## Ce qui n'a pas marché

- **Relire le prédicat de la branche.** La branche était correcte : la récidive
  avait bien eu lieu. Le défaut tient à *l'instant* de l'effet de bord, pas à ce
  qu'il constate. Aucune relecture locale de la branche ne le montre. Il faut
  lister les `continue` qui la suivent.
- **Se fier au test positif.** La fixture `MockLlmProvider` se terminait par un
  troisième tour bénin, ajouté « au cas où une garde voisine reprendrait la
  main ». C'est arrivé : après la récidive, la relance `store_fact` (déclenchée
  par « verified that ») a re-prompté, et la réponse acceptée a été ce tour
  bénin. La P0 était déjà partie, le test comptait « exactement une émission » et
  restait vert. Le filet de sécurité de la fixture cachait exactement le défaut.
- **Le contrôle négatif R3 prévu par le plan.** Retirer
  `intent_guard_retries.contains(ASSERT_GROUNDED_LABEL)` du prédicat devait
  produire une seconde émission. Le test est resté vert, parce que le terme est
  redondant sur tout chemin atteignable : la garde 6d, juste au-dessus et dans le
  même bloc, porte les mêmes conjoints à `!contains` près (la branche y ajoute
  seulement son verrou par tour), elle fait `continue`
  quand elle tire, et ses prédicats sont purs. Aucune mutation ne peut pincer ce
  terme. La réponse honnête est de prouver la redondance et de garder le terme en
  défense en profondeur, pas d'inventer un contrôle.

## Solution

Garder au constat ce qui *compte*, et différer à l'acceptation ce qui *agit*.

- Le `warn!` `guard.assert_grounded_uncorrected` reste au constat, comme dans
  toute la famille `_uncorrected`. C'est l'instrument de comptage, et un compte
  de constats est correct même si le tour est ensuite repris.
- La notification est mise de côté (`pending_fabrication_p0`, déclaré à
  `crates/mika-agent/src/agent_loop/mod.rs:1658` et rempli à `:3743`). Elle n'est envoyée qu'au point
  unique où l'`EndTurn` est accepté, juste avant la télémétrie #953
  `guard.correction_accepted` (`:4452`). La revendication est **revérifiée sur le
  texte accepté** :

```rust
if let Some(notification) = pending_fabrication_p0.take()
    && detect_affirmative_state_claim(&text)
        .is_some_and(|c| !assert_grounded_satisfied(&c, &all_tool_summaries))
    && let Some(ref sender) = tool_ctx.message_sender
{
    let _ = sender
        .send_with_severity(&notification, NotificationSeverity::P0)
        .await;
}
```

Côté tests (`test_fabrication_detected_p0_2867.rs`) :

- La fixture du test positif réaffirme au **troisième** tour aussi, pour que la
  réponse acceptée soit bien la récidive.
- Un nouveau test, `mika2867_une_garde_aval_qui_reprend_la_main_annule_la_p0`,
  joue la séquence suivante : un `send_message` refusé pour longueur, la
  revendication (6d tire), la récidive (6f reprend la main), puis une clôture
  sans revendication. Il exige zéro P0 et `llm_call_count == 4`. Il était rouge
  sur le site d'avant et il est vert après. La recette R5 (retirer la
  revérification) le refait rougir.

## Pourquoi ça marche

Le seul endroit de `run_loop` où « la réponse sort » est vrai est le bloc
d'acceptation qui précède `return Ok(LoopResult::Done …)`. Toutes les gardes ont
passé, et le `text` est celui que `save_message_with_task_context` persiste.
Avant ce point, chaque garde aval peut encore remplacer le texte. Un effet de bord
qui affirme quelque chose sur la réponse livrée doit donc partir de là, et
revérifier son prédicat sur ce texte. La revérification est nécessaire parce que
le re-prompt d'une garde aval peut produire une réponse corrigée.

## Prévention

- **Avant de poser un effet de bord dans la chaîne `EndTurn`, lister les
  `continue` qui suivent sa ligne.** S'il y en a, l'effet de bord ne peut
  affirmer que le constat (log, compteur), jamais la livraison. Tout ce qui sort
  du processus (notification, écriture durable qui dit « livré ») part au bloc
  d'acceptation.
- **Une réponse de queue dans une fixture doit préserver la propriété
  assertée.** Un tour « bénin au cas où » n'est pas neutre : si une garde reprend
  la main, c'est lui qui devient la réponse livrée, et le test atteste un état
  que la production n'a pas. Soit la queue répète la propriété, soit le test
  épingle `llm_call_count` à la valeur exacte attendue.
- **Écrire un test « une garde aval reprend la main ⇒ rien ne part » pour tout
  site d'émission placé dans la chaîne.** 6f est la garde aval la plus facile à
  déclencher depuis le harness : un `send_message` trop long suffit.
- **Une recette de rouge-avant qui reste verte est une information.** Avant de la
  « réparer », se demander si le terme muté est atteignable. Si une garde amont
  porte les mêmes conjoints et fait `continue`, le terme est redondant : le
  prouver par écrit (plan et en-tête du test), le garder si on veut, et ne pas
  fabriquer de contrôle.

## Voir aussi

- `docs/solutions/architecture-patterns/post-hoc-vs-tool-boundary-guard-placement-2026-05-13.md`
  traite le placement des gardes pour les outils à effet de bord. Ici, l'effet de
  bord est celui du moteur, et c'est sa position dans la chaîne qui compte.
- `docs/solutions/best-practices/mock-eval-tautological-assertion-trap-2026-04-29.md` :
  même famille de tests verts pour une mauvaise raison, et même remède en partie
  (épingler `llm_call_count`).
- `docs/operator/environment-variables.md` § *La récidive d'ancrage se nomme* :
  surface opérateur et condition d'armement de `MIKA_FABRICATION_P0_NOTIFY`.
