---
title: Un prédicat écrit à la main à côté d'un match exhaustif ne suit pas les variantes
date: 2026-09-20
last_updated: 2026-09-20
category: best-practices
module: mika-agent/teams/types
problem_type: best_practice
component: dev-loop
severity: medium
applies_when:
  - Vous ajoutez une variante à un enum déjà lu par du code hors de son crate
  - Un `matches!` ou un `if let` décide d'un statut, d'un tier ou d'un verdict
  - Une revue conclut « N correctifs mécaniques » sur N sites qui ont raté la même variante
  - Vous vous demandez si une garde de scan mérite une allowlist
  - Un défaut est latent — il ne coûte rien tant que personne ne scripte la commande
---

# Un prédicat écrit à la main à côté d'un match exhaustif ne suit pas les variantes

## Le fait

`RunStatus` (mika-agent, moteur d'équipe) a reçu deux variantes terminales
d'échec après que la CLI eut été écrite : `FailedNoDelegation` (mika#1676,
2026-08-21) et `FailedTransport` (mika#1671). Recensement exhaustif des lecteurs
hors tests, au 2026-09-20 :

| site | forme | a suivi les deux ajouts ? |
|---|---|---|
| `teams/types.rs` — `Display` | `match` exhaustif | **oui** |
| `teams/engine.rs` — colonne DB | `match` exhaustif | **oui** |
| `teams/notification.rs` | `match` exhaustif, sans bras `_` | **oui** |
| `mika-cli/commands/ask.rs` — code de sortie | `matches!(…, Failed(_))` | non |
| `mika-cli/commands/ask.rs` — rendu texte | `if let Failed(ref msg)` | non |
| `mika-cli/commands/chat.rs` — worker d'équipe | `if let Failed(reason)` | non |

Trois `match` exhaustifs, trois mises à jour. Trois motifs écrits à la main,
zéro. **Ce n'est pas une corrélation entre des auteurs, c'est une propriété des
formes :** `matches!` et `if let` sont exactement les deux constructions qui
**continuent de compiler** quand une variante apparaît. Les trois premiers sites
n'ont pas « été mieux tenus » — leurs auteurs n'ont pas eu le choix.

## Pourquoi c'est la même classe que mika#2023 M2

Le dépôt avait déjà dû nommer ça par écrit, à propos de
`tier == AgentTier::Family` : « une mine qu'aucune erreur de compilation ne
pouvait annoncer ». Introduire `AgentTier::Champion` sous ce prédicat d'égalité
aurait fait refuser le démarrage à toute la population champion, sans qu'aucune
ligne ne rougisse nulle part. Même forme ici, autre enum : le prédicat n'est pas
faux, il est **muet sur ce qu'il ne connaît pas**.

Deux conséquences pratiques, mesurées :

1. **Le ticket sous-compte.** mika#1940 décrivait « 3 correctifs mécaniques » et
   ne nommait qu'une variante sur deux. Un correctif qui aurait ajouté un bras
   `FailedNoDelegation` à chacun des trois `matches!` aurait laissé la moitié du
   défaut armée **le jour même où il est déclaré fermé**.
2. **Le format machine, lui, était juste.** L'enveloppe JSON portait
   `team_run.status = format!("{}", run.status)`, donc `"failed_no_delegation"` —
   parce que `Display` est un `match` exhaustif qui a suivi. Seuls le mode texte
   et le code de sortie mentaient. C'est la même preuve, vue de l'autre côté.

## La leçon

**Le remède qui a la forme du défaut n'est pas d'ajouter des bras, c'est de
retirer aux sites le droit d'énumérer.**

```rust
// teams/types.rs — LE match exhaustif, sans bras `_`
pub fn disposition(&self) -> RunDisposition<'_> {
    match self {
        RunStatus::Running | RunStatus::Suspended => RunDisposition::NotTerminal,
        RunStatus::Completed => RunDisposition::Success,
        RunStatus::Failed(reason) => RunDisposition::Failure { reason },
        RunStatus::FailedNoDelegation => RunDisposition::Failure {
            reason: NO_DELEGATION_REASON,
        },
        RunStatus::FailedTransport(reason) => RunDisposition::Failure { reason },
    }
}

// Le prédicat est DÉRIVÉ, jamais un second match.
pub fn is_terminal_failure(&self) -> bool {
    matches!(self.disposition(), RunDisposition::Failure { .. })
}
```

Dériver plutôt que réécrire rend le biconditionnel « c'est un échec ⟺ il y a une
raison » vrai **par construction** plutôt que vrai **par test**. Un test qui
l'assure reste utile — il rougit le jour où quelqu'un réécrit
`is_terminal_failure` en second `match`, ce qui est précisément le geste à
refuser — mais il ne porte plus la propriété tout seul.

## Trois corollaires qui ne se devinent pas

### 1. Une allowlist naît vide ou pas du tout

La garde de scan (`ProductionScanner`) qui refuse un nouveau motif à la main
dans `mika-cli` est livrée **avec une allowlist vide**, et c'est une conséquence
de la conception, pas une discipline : le classificateur lui-même ne nomme
aucune variante (il lit `disposition()`), donc il n'y a rien à exempter. Une
allowlist née vide est un emplacement pour la prochaine entorse (mika#2323 le
dit en toutes lettres) ; ici il n'y en a pas. Le message d'échec de la garde dit
la résolution : *passer par `disposition()`, jamais ajouter une entrée.*

### 2. Aucun test comportemental ne peut voir la régression

Si quelqu'un réécrit demain un `matches!(run.status, RunStatus::Failed(_))` dans
la CLI, **aucune assertion existante ne rougit** : les décisions couvertes
restent justes, c'est un *nouveau* chemin qui redevient muet. C'est ce qui rend
la garde de scan nécessaire plutôt que confortable — même famille que
`mika2335_no_production_dispatch_transitions_a_parent_without_stamping` et
`mika2205_periodic_scans_do_not_read_the_pat_field_directly`.

### 3. La preuve directe demande un geste manuel, une fois

Un test ne peut pas exprimer « une septième variante fait échouer la
compilation ». La vérification est manuelle, à la livraison : ajouter une
variante localement, lancer `cargo check`, compter les sites. Résultat consigné
pour mika#1940 — **quatre** sites (`disposition()`, `Display`, la colonne DB,
`notification.rs`) et **zéro** dans `mika-cli`.

## Le contexte où ça se décide : un défaut latent

Aucun consommateur automatisé de `mika ask --team` n'existait dans le dépôt
(recherche sur `skills/`, `scripts/`, `.claude/` : zéro appel, seulement de la
documentation). Le ticket était étiqueté p3 avec un déclencheur d'escalade
écrit : « si le scripting opérateur rencontre un faux succès en production →
p2 ».

C'est **exactement la forme qui justifie une réparation structurelle plutôt
qu'un rattrapage** : le correctif est bon marché maintenant, et l'incident
serait invisible plus tard — un code de sortie 0 sur un échec ne produit aucune
ligne de journal, aucune alerte, rien à greper. On ne peut pas instrumenter une
absence de plainte.

## Voir aussi

- `crates/mika-agent/CLAUDE.md` § *Management Tools* — la disposition et sa garde
- `crates/mika-cli/CLAUDE.md` § `mika ask` — le contrat de code de sortie de `--team`
- Racine `CLAUDE.md` § `MIKA_AGENT_TIER` (mika#2023 M2) — la première occurrence de la classe
- mika#2158 — `is_groomed` recopié, la deuxième : *un résolveur écrit deux fois est un résolveur qui peut se contredire*
