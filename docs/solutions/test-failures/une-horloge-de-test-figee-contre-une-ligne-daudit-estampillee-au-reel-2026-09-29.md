---
module: crates/mika-agent/src/worktree_reaper.rs, crates/mika-agent/src/db.rs
tags: [testing, time-dependence, dedup, audit-events, worktree-reaper, flakiness, mika-2482, mika-2420, mika-2497]
problem_type: test_failure
category: test-failures
date: 2026-09-29
ticket: mika#2482
applies_when:
  - Testing a 24 h dedup guard that reads its window from `audit_events`
  - Writing an `async` test around `log_audit_event` / `count_recent_audit_events_for_target`
  - A test module defines a frozen `fn now() -> DateTime<Utc>` for the pure functions
  - A dedup assertion passes and its mirror ("past the window") fails
---

# Une horloge de test figée contre une ligne d'audit estampillée au réel

## Le symptôme

Un test de déduplication à trois assertions, dont la première et la deuxième
passent et la troisième échoue :

```rust
let now0 = now();                              // 2026-09-20T12:00:00Z — FIGÉ
assert!(!stale_probe_recently_done(&db, path, now0, "t").await);   // ok
record_stale_probed(&db, "s", &t, STALE_PROBE_NO_PR, "t").await;
assert!( stale_probe_recently_done(&db, path, now0, "t").await);   // ok
assert!(!stale_probe_recently_done(                                // ÉCHOUE
    &db, path, now0 + TimeDelta::seconds(REFUSAL_DEDUP_SECS + 60), "t").await);
```

Message : « passé 24 h, la sonde est de nouveau permise ». Le prédicat répond
« déjà sondé » pour une borne postérieure de plus de 24 h à l'écriture.

## La cause : deux horloges, et elles s'éloignent d'un jour par jour

Le module de test définit une horloge **figée**, ce qui est correct et voulu pour
les fonctions pures (un screening doit être reproductible) :

```rust
fn now() -> DateTime<Utc> {
    crate::timestamp::parse("2026-09-20T12:00:00Z").unwrap()
}
```

Mais `log_audit_event` estampille `created_at` à **`Utc::now()`**, l'horloge
réelle. Et le prédicat compare les deux :

```sql
SELECT COUNT(*) FROM audit_events
 WHERE agent_id = ?1 AND tool_name = ?2 AND target_key = ?3 AND created_at > ?4
```

avec `?4 = now - REFUSAL_DEDUP_SECS`, `now` étant le paramètre **figé**. Le
2026-09-29, la ligne écrite porte `2026-09-29`, la borne calculée vaut
`2026-09-19` : la ligne est **neuf jours** au-delà de la borne.

| appel | borne `since` calculée | `created_at` de la ligne | `>` ? | verdict | attendu |
|---|---|---|---|---|---|
| avant écriture | 2026-09-19T12:00Z | — | — | `false` | `false` ✓ |
| après écriture | 2026-09-19T12:00Z | 2026-09-29 | oui | `true` | `true` ✓ **par accident** |
| +24 h + 60 s | 2026-09-20T12:01Z | 2026-09-29 | **oui** | `true` | `false` ✗ |

## Ce qui rend le défaut coûteux plutôt que gênant

**La deuxième assertion passe pour la mauvaise raison.** Elle est censée
établir « une ligne fraîche est dans la fenêtre » ; elle établit en réalité « une
ligne de neuf jours dans le futur est dans la fenêtre ». Un test à deux
assertions au lieu de trois serait donc **vert et sans valeur** — la forme la
plus chère, parce qu'elle se lit comme une couverture.

**L'erreur croît d'un jour par jour écoulé.** Le jour où le test est écrit,
l'écart entre l'horloge figée et le réel peut être de quelques heures, et la
troisième assertion peut passer. Elle casse ensuite pour une raison que rien dans
le diff n'explique — un test qui pourrit au calendrier, pas au code.

**Le sens du décalage est celui qui masque.** L'horloge figée est dans le
**passé**, donc la borne est toujours en retard sur la ligne, donc le prédicat
répond toujours « déjà sondé ». Un test de dédup qui répond toujours « déjà fait »
ne peut pas détecter une dédup qui ne se relâche jamais — très exactement le
défaut qu'on veut éviter en production, où il produirait un scan muet pour
toujours.

## La règle

> **Une assertion qui compare une borne calculée à une ligne écrite doit calculer
> cette borne sur l'horloge qui a estampillé la ligne.**

Concrètement, dans ce dépôt : un test `async` qui traverse `audit_events` prend
`Utc::now()`, pas le `now()` figé du module — et les tests de dédup voisins le
faisaient déjà, ce qui est le signal qu'il fallait lire avant d'écrire :

```rust
record_refusal(&db, "session-2420", &dirty, Utc::now(), "trace-1").await;   // mika#2420
probe_main_checkout(&db, "session-2449", tmp.path(), Utc::now(), "trace-1").await;  // mika#2449
```

Le `now()` figé reste le bon outil **pour les fonctions pures** du même module
(`screen_worktrees`, `screen_target_purges`, `classify_branch_staleness`) : rien
n'y est comparé à une ligne, tout y est comparé à des valeurs que le test
fabrique lui-même.

## Le correctif

```rust
// L'horloge RÉELLE, et pas le `now()` figé du module : `log_audit_event`
// estampille la ligne à `Utc::now()`, donc une borne calculée sur une date
// fixe s'éloigne de la ligne écrite d'un jour par jour écoulé — la sonde
// répondrait « déjà sondé » pour toujours.
let now0 = Utc::now();
```

## Comment le reconnaître

Trois signes, dans l'ordre de coût de diagnostic :

1. Le test est `async` et touche `audit_events` (ou toute table dont l'écriture
   est estampillée serveur).
2. Le module de test définit un `fn now()` figé, et le test l'appelle.
3. **L'assertion « dans la fenêtre » passe et l'assertion « hors fenêtre »
   échoue** — la signature du décalage. Le cas inverse (horloge figée dans le
   *futur*) donne le symptôme miroir : « dans la fenêtre » échoue d'emblée.

## Population de ce dépôt

Tout garde dont la fenêtre est lue en base, et il y en a une dizaine :
`REFUSAL_DEDUP_SECS` (refus du faucheur, mika#2420), le ledger d'exclusions
(mika#2131), le marqueur de bail (mika#2199), le ledger de revue QA (mika#2347),
la sonde de saleté (mika#2449), la sonde stale (mika#2482). Chacun a un test de
dédup, et chacun est exposé à cette confusion.

## Pièges voisins, à ne pas confondre

- **`created_at > since`, jamais `>=`.** Une ligne écrite exactement à la borne
  est hors fenêtre. Sans conséquence ici (les deux instants ne coïncident pas),
  mais c'est ce qui rend un test « à la seconde près » fragile.
- **Le jeu en mémoire n'est pas la base.** Plusieurs dédups de ce dépôt ont un
  jeu en mémoire *devant* la lecture en base (mika#2131) ; un test qui n'exerce
  que le premier ne prouve rien sur le second, et inversement.

## Voir aussi

- `docs/solutions/test-failures/serial-ne-protege-pas-contre-les-tests-paralleles-2026-09-05.md`
  — l'autre famille de dépendance d'environnement dans les tests de ce crate.
- `docs/solutions/database-issues/iso8601-timestamp-migration.md` — pourquoi
  l'ordre lexicographique des timestamps est correct, ce qui est la propriété
  dont ces comparaisons SQL dépendent.
