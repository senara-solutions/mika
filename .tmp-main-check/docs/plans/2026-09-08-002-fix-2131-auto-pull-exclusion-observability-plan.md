---
issue: 2131
type: fix
title: "auto_pull : les décisions d'exclusion sont observables (stuck_ready_reconcile, AC6/AC7)"
branch: fix/2131/auto-pull-stuck-ready-reconcile
status: groomed
---

> **Grooming (orchestrateur, 2026-09-08) :** plan RÉUTILISÉ du plan umbrella déjà groomé par mika-arch (session `814dcefd`, PR #2226 fermée à la décomposition). L'umbrella fermée a dé-groomé ses sous-tickets (le plan vivait sur la branche umbrella, pas sur l'issue). Réutilisé ici pour #2131 **standalone**, adapté par la main de l'orchestrateur (hors sandbox — les 3 pilotes groom échouaient sur `policy:deny [bash-git-readonly]` en redirigeant `git show <rev>:<plan> > fichier`, voir le ticket systémique fiché). Invariant + ACs inchangés (déjà validés en umbrella). #2132 = frère sur la même surface (superset AC1-5 ⊇ #2131 AC6/AC7) : le même changement de code peut le satisfaire ; à lier/fermer au moment de la PR si la couverture est effective, sinon suivi séparé.

# Plan umbrella — auto_pull : rendre les décisions d'exclusion observables (#2131 + #2132)

## Invariant (une phrase)

**Toute décision d'exclusion d'`auto_pull` est observable — émise à un niveau effectivement
collecté (INFO ou `audit_events`), nommant le ticket ET le filtre qui l'a écarté — jamais
silencieusement abandonnée en `debug!` sur une cible non collectée.**

## Pourquoi une unité (cohésion causale, pas un lot)

#2131 (AC6/AC7) et #2132 (AC1-AC5) posent le **même** invariant sur la **même** surface
(`auto_pull.rs`, le logging des décisions d'exclusion). #2131 l'aborde depuis
`stuck_ready_reconcile` (#1651/#1403 écartés sans trace, 6 jours de silence) ; #2132 depuis la
mesure d'observabilité (exclusions en `debug!` sur cible non collectée). Les AC se recouvrent
terme à terme (#2131 AC6 ≡ #2132 AC1 ; #2131 AC7 ≡ #2132 AC2). #2132 est le sur-ensemble.
Un seul changement de code satisfait les deux → PR atomique, un échec → tout reflue.

**Écarté du bundle :** #2161 (`auto_feeder_no_backlog` message trompeur) — invariant DIFFÉRENT
(exactitude d'un message, pas observabilité des exclusions). L'inclure ferait un lot.

## Acceptance criteria (union #2132 AC1-5 ⊇ #2131 AC6/AC7)

- **AC1** — Chaque ticket écarté d'une phase d'`auto_pull` produit une trace au niveau
  **effectivement collecté** (INFO, ou une entrée `audit_events`), nommant **le ticket** et **le
  filtre** qui l'a écarté. Un `debug!` sur cible non collectée ne compte pas. (≡ #2131 AC6)
- **AC2** — **Contrôle négatif** : un ticket qui traverse tous les filtres ne produit **aucune**
  trace d'exclusion. (≡ #2131 AC7)
- **AC3** — La trace nomme le **filtre** (« écarté par <filtre> »), pas seulement le fait.
- **AC4** — **Preuve de non-vacuité** : un test rejoue un tick avec #1651 et #1403 dans le bassin
  et **assert** deux traces nommant leur filtre (rouge-avant sur le code actuel, vert après).
- **AC5** — La **politique journal-vs-`audit_events` est écrite** (dans le plan + en commentaire
  au point de code) : quel volume d'exclusion va en INFO, quel volume en `audit_events`, pour
  qu'un tick touchant des dizaines de tickets ne noie pas le journal.

## Politique journal-vs-audit_events (AC5, décidée — canal VÉRIFIÉ)

**Fait vérifié (mika-arch F1) :** `auto_pull.rs` émet DÉJÀ des `log_audit_event` (9 sites
existants). Le canal `audit_events` existe et auto_pull l'utilise — aucun ajout d'architecture.

Décision : **le détail par-ticket des exclusions va en `log_audit_event`** (structuré, requêtable,
sans noyer le journal), **plus un résumé agrégé unique par tick en INFO** (« N tickets écartés ce
tick : <compte par filtre> »). Rationale : un tick peut écarter des dizaines de tickets ; le détail
par-ticket appartient à `audit_events` (interrogeable a posteriori sur #1651/#1403), le journal
INFO ne porte que l'agrégat actionnable. Le canal existant rend cette décision immédiate, pas un
changement de surface.

## Fire-Disposition

Livrable détecteur = le test AC4 (non-vacuité) dans les tests d'`auto_pull` : **rouge sur `main`
actuel** (les exclusions sont en `debug!` non collecté → aucune trace INFO/audit → assert échoue),
**vert après** le fix. Gate CI `Check` bloquant. Garde permanente : toute régression re-silenciant
les exclusions refait échouer `Check`.

## Phases

1. **Repérer les points d'exclusion** dans `auto_pull.rs` — la chaîne de `.filter(...)` (~lignes
   988-996 : `is_feeder_excluded`, `is_groomed`, open-PR, `ready`) + les filtres de phases 2-3 et
   `stuck_ready_reconcile`. Ces `.filter()` écartent aujourd'hui sans trace collectée.
2. **Émettre la trace observable** (AC1/AC3) : par exclusion → entrée `audit_events` nommant
   ticket+filtre ; par tick → un résumé agrégé INFO (AC5). Contrôle négatif préservé (AC2).
3. **Test non-vacuité** (AC4) : rejouer un tick avec #1651/#1403 → 2 traces nommant le filtre.
4. **Vérif** : le contrôle négatif (AC2) ne produit aucune trace pour un ticket qui passe.

## Hors périmètre

- La **politique** d'exclusion elle-même (quels filtres écartent quoi) — inchangée ; ce ticket
  rend les décisions OBSERVABLES, pas différentes.
- #2161 (message auto_feeder_no_backlog) — invariant distinct, ticket séparé.

## Fichiers probables

- `mika/crates/mika-agent/src/auto_pull.rs` (points d'exclusion + traces)
- tests d'auto_pull (AC4 non-vacuité)

## Closes

Closes #2131
Closes #2132
