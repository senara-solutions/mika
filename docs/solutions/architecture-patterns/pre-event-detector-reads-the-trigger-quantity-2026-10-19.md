---
title: "Un détecteur qui précède un événement doit lire la grandeur sur laquelle l'événement tire"
date: 2026-10-19
category: architecture-patterns
module: crates/mika-agent/src/compaction_handsoff.rs
component: agent_loop
problem_type: design_decision
severity: medium
symptoms:
  - "AC1 de mika#1748 demande un seuil « d'usage du contexte (défaut 85 %) », sous-entendu en tokens"
  - "mika-agent n'instrumente aucun ratio d'occupation de fenêtre : `maybe_compact` tire sur `db.count_messages() > COMPACTION_THRESHOLD` (50)"
  - "l'observation fondatrice (100 % → 12 %) vient de Claude Code, dont la compaction remplace tout l'historique, alors que `maybe_compact` préserve les 20 derniers messages"
root_cause: design_decision
resolution_type: design_decision
related_components:
  - compaction
  - system_prompt
tags:
  - compaction
  - handsoff
  - threshold
  - detector-alignment
  - single-source-of-truth
---

# Un détecteur qui précède un événement doit lire la grandeur sur laquelle l'événement tire

## Contexte

mika#1748 (vol 1, PR mika#2809) ajoute une cérémonie de témoignage — le bloc
`<compaction-imminent>` du prompt système — proposée à l'agent juste avant que
sa conversation soit condensée. Le ticket décrivait le seuil comme un
pourcentage d'usage du contexte, à 85 % par défaut.

## Le piège

Un pourcentage de tokens paraît être la lecture naturelle de « 85 % du
contexte ». Mais la compaction de mika-agent ne regarde pas les tokens. Elle
compte les messages persistés et condense au-delà de 50. Un seuil en tokens
serait un **second instrument**, qui mesure une autre grandeur que celle qui
déclenche l'événement qu'il prétend annoncer. Il peut :

- rester sous son seuil au moment où la compaction tire (cinquante messages
  courts) : la cérémonie n'a jamais lieu ;
- franchir son seuil trois fois sans que la compaction approche (quelques
  messages énormes) : la cérémonie devient du bruit.

Un détecteur désaligné de son déclencheur ment précisément le jour où la
réponse compte, et aucun test unitaire du détecteur seul ne le voit, puisque
chaque instrument est correct pour sa propre grandeur.

## La forme retenue

1. **Exprimer le seuil dans l'unité du déclencheur.** `ceremony_due` compare
   `db.count_messages()` à un pourcentage de `COMPACTION_THRESHOLD` : 85 % de
   50 donne 43 messages. Le « 85 % » de l'AC est conservé ; c'est sa base qui
   change.
2. **Un seul site de vérité pour la constante.** `COMPACTION_THRESHOLD` passe
   de `const` privée à `pub(crate)`, et le détecteur l'importe. Une copie de
   « 50 » dans le module du détecteur dériverait en silence le jour où la
   compaction serait réglée autrement.
3. **Calculer en entiers, multiplier plutôt que diviser.**
   `message_count * 100 >= pct * COMPACTION_THRESHOLD` franchit exactement à 43.
   Une division entière ou un flottant ferait franchir à 42 ou à 44 selon la
   troncature.
4. **Évaluer avant l'événement, pas après.** Le prédicat tourne dans
   `load_agent_context` (début de tour), alors que `maybe_compact` tourne en fin
   de tour. Le décalage, d'au moins un message, est assumé et borné par la
   marge 43 → 50. Un avertissement émis après le tour arriverait après la
   condensation qu'il annonce.
5. **Vérifier ce qui meurt dans *ce* substrat.** `maybe_compact` garde les 20
   derniers messages : la cérémonie atterrit dans la fenêtre préservée et n'est
   pas avalée par la compaction qui la suit. Ce qu'elle doit sauver, c'est le
   résidu des messages `1..N−20`. L'intuition importée de Claude Code (tout
   disparaît) aurait conduit à avancer le seuil sans raison.

## Comment l'appliquer

Avant d'écrire un seuil d'alerte, de pré-avertissement ou de cérémonie,
trouver la ligne de code qui déclenche l'événement visé et lire **ce qu'elle
compare**. Si le ticket nomme une autre grandeur, traduire le seuil dans celle
du déclencheur et noter la traduction dans le module (ici, la section « D2 »
de `compaction_handsoff.rs`) et dans la PR comme réponse explicite à l'AC.
Importer la constante du déclencheur plutôt que la recopier.

Les tests épinglent les deux côtés du franchissement : l'eval
`test_compaction_handsoff_1748` vérifie que le bloc atteint le prompt au
franchissement et qu'il en est absent un message plus tôt.

## Voir aussi

- `crates/mika-agent/src/compaction_handsoff.rs` : sections D2 à D4.
- `docs/architecture/compaction-handsoff.md` : description du mécanisme.
- mika#2293 : un réglage refusé qui se tait est indiscernable d'un réglage
  appliqué, d'où le WARN de `resolve_threshold_pct` sur une valeur hors
  `1..=100`.
