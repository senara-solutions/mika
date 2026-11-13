# mika#2795 — un callback sans ligne `Outcome:` n'est pas un verdict de groom

**Ticket :** senara-solutions/mika#2795
**Type :** fix
**Labels :** `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`
**Relie :** mika#2545 (le gate), mika#2762 (la mesure), mika#2786 (le précédent conceptuel), mika#2590 (ne jamais pré-filtrer en SQL), mika#2492 (le producteur de la ligne `Outcome:`), mika#2638 (le lecteur unique des variantes d'URL)

---

## Contexte, et ce que la mesure dit exactement

Le gate mika#2545 vit dans `validate_dispatch_readiness`
(`crates/mika-agent/src/skills/executor.rs:3441-3570`) et refuse un re-dispatch
**automatique** de groom quand le dernier callback de groom du ticket porte
`Outcome: ESCALATE`. Trois termes conjonctifs : la classe de dispatch est `groom`,
`originating_message.is_none()` (le discriminant de ré-armement — les deux gestes
de récupération opérateur portent un message et traversent par construction), et le
dernier groom a escaladé.

Ce « dernier groom » est lu par `latest_groom_verdict_for_issue`
(`crates/mika-agent/src/db/tasks.rs:4410-4433`) :

```sql
SELECT child.result FROM tasks child
 JOIN tasks parent ON child.parent_task_id = parent.id
 WHERE child.agent_id = ?1
   AND child.trigger_type = 'callback'
   AND child.dispatch_class = 'groom'
   AND child.status IN ('completed', 'delivered')
   AND parent.reference_url IN (?2, ?3)
 ORDER BY child.created_at DESC, child.id DESC
 LIMIT 1
```

Le `LIMIT 1` retient **tout** callback de groom terminal. Or le lanceur différé en
est un : `promote_next_deferred_callback` et
`promote_next_deferred_callback_for_class`
(`db/tasks.rs:4472-4553`) font passer à `status = 'completed'` une row
`trigger_type = 'callback'`, `dispatch_class = 'groom'`,
`label = 'long_running:run_claude_pilot:deferred'`, de même tâche parente, avec
`result = 'deferred dispatch slot freed'`. Ce texte ne porte **aucune** ligne
`Outcome:`. Le prédicat pur `groom_escalate_verdict` (`executor.rs:2206`) le lit
donc comme `NotEscalated` — « le dernier groom n'a pas escaladé ».

### La mesure du ticket, et ce qu'elle isole

Sur mika#2762, le 2026-10-18, base `~/.mika/data/mika.db`, **même tâche parente
`1936fc26`** :

| heure (Z) | callback de groom en tête de la requête | issue du dispatch |
|---|---|---|
| 18:30:55 | `18:20:02` — lanceur différé, `deferred dispatch slot freed` | **passé** → passe d'architecte `49ffa230`, retenue |
| 18:37:06, 18:37:51 | `18:30:55` — `Outcome: ESCALATE — first-pass` | refusé, `groom_escalate_redispatch_refused` |

Sept minutes, même tâche, même tour d'origine. La seule variable est la ligne en
tête de la requête. Le lanceur de 18:20:02, posé **après** l'ESCALATE de 18:13, l'a
caché ; le re-dispatch de 18:30:55 a coûté un pilote de groom (1,24 $) et un tour
d'architecte. Le gate n'était pas désarmé — il a lu une row qui n'est pas un
verdict.

### Trois faits relevés au grooming, qui resserrent le périmètre

1. **La sœur du lecteur est déjà immunisée, et c'est le modèle de la réparation.**
   `has_completed_groom_for_issue` (`db/tasks.rs:4333-4361`) ne fait **pas** de
   `LIMIT 1` : elle collecte les `result` et décide en Rust via
   `groom_result_convergence` / `aggregate_groom_convergence`
   (`task_state/tasks.rs`). Une row neutre y rend `GroomConvergence::Absent`, que
   l'agrégat ignore. Le défaut de mika#2795 n'existe que du côté `LIMIT 1`.
   **Elle n'est pas touchée par ce vol.**

2. **La ligne `Outcome:` est garantie par construction côté producteur.**
   `_set_outcome_line` (`skills/bundled/_shared/dispatch-lib.sh:2873`, mika#2492)
   garantit « exactement une ligne `Outcome:` ancrée » sur tout RESULT terminal
   produit par un pilote : `_escalate_groom` (9365) écrit
   `Outcome: ESCALATE — ${stage}`, `dispatch_claude_pilot` (12172) écrit
   `Outcome: PLAN_GROOMED`. Un prédicat **ancré en début de ligne** sur `Outcome: `
   est donc sans perte : il ne peut pas écarter un verdict réel. Les rows sans
   cette ligne ne sont pas des verdicts partiels, ce sont des rows d'une autre
   nature.

3. **Le défaut n'est pas propre au lanceur différé.** Toute future row terminale de
   classe `groom` écrite par un producteur non-pilote (note de faucheuse, wrapper,
   promotion de file) reproduira exactement le même masquage. La règle correcte
   porte sur la **forme** de la row — « porte-t-elle un verdict ? » — jamais sur
   une liste de `label`. Une exclusion par `label = 'long_running:…:deferred'`
   fermerait le cas mesuré et laisserait la classe ouverte.

mika#2786 — qui fait sauter les lignes neutres pour compter `anchor_withheld_streak`
— est cité par le ticket comme précédent. Il est **encore OPEN** et son symbole
n'existe pas dans l'arbre : c'est un précédent **conceptuel**, pas une dépendance.
Ce vol ne l'attend pas et ne le consomme pas.

---

## La direction retenue

**Le lecteur rend toutes les rows, la décision passe en Rust, et la règle porte sur
la forme de la row.**

### Pièce 1 — le lecteur DB rend la liste, pas la tête

`Database::latest_groom_verdict_for_issue` devient
`Database::groom_callback_results_newest_first`, perd son `LIMIT 1` et rend
`Result<Vec<Option<String>>>`, ordonné du plus récent au plus ancien. Le `JOIN`, le
scoping par `agent_id`, `issue_url_variants` et la comparaison
`parent.reference_url IN (?2, ?3)` — **sur une seule ligne source** — sont
conservés tels quels.

**Aucun pré-filtre SQL** : ni `result NOT LIKE`, ni `label != …`, ni
`instr(result, 'Outcome: ')`. Doctrine mika#2590 : *une ligne écartée par SQL est
invisible au verdict*. Le compteur de la pièce 4 existe précisément pour mesurer la
population que ce vol répare ; la filtrer en SQL la rendrait incomptable, et
rendrait aussi indistinguables « aucune row neutre » et « des rows neutres jamais
vues ». Le `JOIN` borne déjà le résultat à une poignée de rows par issue — c'est la
même borne que la sœur immunisée exploite depuis mika#2545.

Le doc-comment actuel porte trois raisonnements porteurs qui **migrent mot pour
mot**, parce que chacun se ré-invente faux :

- pourquoi le **dernier** groom et non un `EXISTS` — un ticket re-groomé doit
  redevenir dispatchable ; c'est un frein, pas un mur ;
- pourquoi `created_at DESC, id DESC` — `created_at` est la **naissance** du
  dispatch, qu'aucune faucheuse ne réécrit, à la différence d'`updated_at` que
  touchent le watchdog #959, la faucheuse de blocage silencieux mika#2249 et le
  balayage fantôme mika#1712 ; `id DESC` est un départage déterministe et
  explicitement **non chronologique** (UUID v4) ;
- le contrat des deux niveaux d'`Option` — qui change de forme et doit être
  réécrit : « liste vide » dit *aucun callback de groom terminal* ; `Some(texte)`
  dit *une row avec un `result`* ; `None` dit *une row terminale dont la colonne
  `result` est NULL*.

### Pièce 2 — l'enveloppe asynchrone suit

`AsyncDatabase::latest_groom_verdict_for_issue` (`async_db.rs:1758-1770`) est
renommée et change de type de retour. Même forme, même `with_db`. Un seul appelant
en production (pièce 4) et les tests.

### Pièce 3 — le prédicat de forme, ancré en ligne

Dans `executor.rs`, à côté de `GROOM_ESCALATE_MARKER` (`:2144`) — pas dans
`task_state/tasks.rs`, pour que la grammaire du gate reste à **un** site :

```rust
pub(crate) const GROOM_OUTCOME_LINE_PREFIX: &str = "Outcome: ";

pub(crate) fn carries_groom_outcome_line(result: &str) -> bool {
    result.lines().any(|l| l.starts_with(GROOM_OUTCOME_LINE_PREFIX))
}
```

**Ancré en début de ligne, jamais en sous-chaîne.** C'est la leçon de mika#2050
(Signal S : un pilote qui *parle* d'un jeton se lit comme l'émettant) et la mesure
de mika#2105 (`instr(result, 'Outcome: PLAN_GROOMED')` a rendu 651 sur une note
d'auto-skip qui citait le marqueur pour dire qu'aucune preuve n'avait été frappée).
Le prédicat frère `groom_escalate_verdict` est déjà ancré ligne — celui-ci l'est de
la même façon, par la même construction `lines().any(starts_with(…))`.

### Pièce 4 — la sélection, pure et exhaustive

Une fonction **pure**, testable sans base :

```rust
pub(crate) struct GroomVerdictSelection {
    pub state: GroomVerdictState,
    pub skipped_neutral: usize,
    pub rows_examined: usize,
}

pub(crate) fn groom_verdict_from_rows(rows: Vec<Option<String>>) -> GroomVerdictSelection
```

Parcours du plus récent au plus ancien, quatre cas et **aucun autre** :

| row | décision | raison |
|---|---|---|
| `Some(texte)` sans ligne `Outcome:` ancrée | **sauter**, `skipped_neutral += 1` | ce n'est pas un verdict — lanceur différé, note de faucheuse, tout producteur neutre futur |
| `None` (colonne NULL) | **arrêt**, `Unreadable` | échec fermé : on ne peut pas savoir, donc on ne laisse pas passer |
| `Some(texte)` portant une ligne `Outcome:` | **arrêt**, délégué à `groom_escalate_verdict` **inchangé** | c'est un verdict, et sa lecture ne bouge pas |
| liste épuisée ou vide | `NotEscalated` | même état épistémique que « aucun groom n'a tourné » |

Le `None` s'arrête plutôt que de sauter, délibérément : le contrat mika#2545 dit
*« on n'a pas pu savoir »*, et sa population opérateur doit rester vide et
comptable. Un `None` sauté la viderait en silence au lieu de la refuser.

`groom_escalate_verdict` **n'est pas modifiée**. C'est le contrôle R3 de ce vol :
le comportement se **déplace**, il n'est pas réécrit — ses six tests purs
(`executor.rs:12965-13024`) restent verts octet pour octet.

### Pièce 5 — le lecteur du gate, et sa sonde

```rust
pub(crate) async fn groom_verdict_state(db: &AsyncDatabase, issue_url: &str) -> GroomVerdictState
```

garde sa signature — le site d'appel du gate (`executor.rs:3441-3570`) ne change
pas d'une ligne. Son corps appelle le nouveau lecteur, applique
`groom_verdict_from_rows`, émet la sonde, rend l'état. Une erreur DB reste
`Unreadable(format!("DB error: {e}"))`.

**La sonde, sur toutes les branches.** Événement INFO
`groom_verdict_selected`, champs `issue_url`, `rows_examined`, `skipped_neutral`,
`verdict`. Elle est émise y compris quand `skipped_neutral == 0` : c'est le
contrôle positif. Sans la ligne à zéro, « le tri n'a rien sauté » et « le binaire
servi est antérieur au correctif » rendent les mêmes octets (classe mika#2205 /
mika#2340) ; et un réglage qu'on ne peut pas observer n'est pas un réglage
(mika#2293). `skipped_neutral > 0` **est** la population que ce vol répare, et la
requête opérateur qui dit si le trou se refermait souvent ou une fois.

`verdict` prend trois valeurs : les deux constantes existantes
`GROOM_ESCALATE_VERDICT_ESCALATED` / `GROOM_ESCALATE_VERDICT_UNREADABLE`
(`executor.rs:2159-2160`) — mêmes épellations, un seul site de définition chacune —
plus une nouvelle `GROOM_VERDICT_NOT_ESCALATED = "not_escalated"`.
`ALL_GROOM_ESCALATE_VERDICTS` **reste à deux entrées** : c'est le format de fil des
*motifs de refus*, pinné par `mika2545_the_two_refusal_verdicts_are_a_wire_format`
(`canonical_tokens.rs:3929`), et la troisième valeur n'est pas un motif de refus.
Les y fondre casserait ce garde et mélangerait deux populations opérateur.

### Ce que la direction ferme, et ce qu'elle ne ferme pas

Elle ferme **toute** row terminale de classe `groom` dépourvue de verdict, présente
ou future, par une règle de forme. Elle ne ferme pas le cas d'un producteur qui
écrirait une ligne `Outcome:` fautive — ce n'est pas la classe de ce ticket, et
`_set_outcome_line` est le garde-fou de ce côté-là.

---

## Hors périmètre, et pourquoi

- **`has_completed_groom_for_issue` et `groom_provenance_verdict`** — déjà immunisés
  (fait 1). Les toucher serait un changement sans défaut mesuré, dans la fonction
  voisine, au milieu d'un correctif : exactement le type de diff qui rend une
  revue indécidable.
- **Exclure le lanceur par `label`** — règle par valeur là où une règle par forme
  subsume le cas (fait 3), et qui laisse la classe ouverte au prochain producteur
  neutre.
- **Un pré-filtre SQL** — mika#2590, détaillé en pièce 1.
- **mika#2786** — OPEN, symbole absent de l'arbre. Précédent conceptuel cité, pas
  une dépendance ; aucune mutualisation de code n'est tentée avec un ticket non
  mergé.
- **Un scan de source « un seul lecteur de `GROOM_OUTCOME_LINE_PREFIX` »** — écarté
  explicitement, et la raison est écrite ici pour qu'elle ne soit pas re-tranchée
  à l'aveugle au prochain passage. Il coûterait ~85 lignes (scanner, aiguille,
  contrôle de non-vacuité, allowlist vide) pour une classe que le périmètre rend
  aujourd'hui improbable : le préfixe naît avec son unique lecteur, dans le même
  fichier, à quinze lignes de lui. Le jour où un second site le lit, c'est un
  ticket de suivi à ouvrir, pas une dette silencieuse — et le coût évité est ce qui
  garde ce vol loin de sa borne. Les gardes structurels existants (pièce
  *Verification Contract*) couvrent les classes réellement atteignables.
- **Changer le site d'appel du gate** (`executor.rs:3441-3570`) — le `match`
  exhaustif sans bras joker, le `warn!` sous `GROOM_ESCALATE_REFUSED_EVENT` et le
  `log_audit_event` restent intacts. La signature de `groom_verdict_state` est
  préservée pour cette raison.

---

## Livrables et ordre de commit

1. **`crates/mika-agent/src/db/tasks.rs`** — `latest_groom_verdict_for_issue` →
   `groom_callback_results_newest_first` : `LIMIT 1` retiré, `query_row` →
   `query_map`, retour `Result<Vec<Option<String>>>`, doc-comment réécrit en
   portant les trois raisonnements de la pièce 1. La ligne
   `AND parent.reference_url IN (?2, ?3)` reste **une seule ligne** (voir R2).
2. **`crates/mika-agent/src/canonical_tokens.rs`** — le message du plancher
   `mika2638_…` (`:5696`) nomme l'ancien symbole dans sa prose de panic ;
   il est mis à jour pour nommer le nouveau. Prose, pas prédicat : le compte de
   `reference_url IN (` ne bouge pas.
3. **`crates/mika-agent/src/async_db.rs`** — enveloppe renommée, même type de
   retour que la pièce 1.
4. **`crates/mika-agent/src/skills/executor.rs`** — `GROOM_OUTCOME_LINE_PREFIX`,
   `carries_groom_outcome_line`, `GROOM_VERDICT_NOT_ESCALATED`,
   `GROOM_VERDICT_SELECTED_EVENT`, `GroomVerdictSelection`,
   `groom_verdict_from_rows`, réécriture du corps de `groom_verdict_state` + sonde.
   `groom_escalate_verdict` et le site d'appel du gate : **non touchés**.
5. **`crates/mika-agent/src/db/tests/task_messages_and_groom.rs`** — les 9 tests
   mika#2545 (`:1341-1570`) adaptés au nouveau type de retour, plus le cas mesuré
   (D1 ci-dessous). `GROOM_CALLBACK_ESCALATE` et `set_created_at` sont réutilisés
   tels quels.
6. **`crates/mika-agent/src/skills/executor.rs` (tests)** — `mod
   mika2545_escalate_terminal` étendu : tests purs de `groom_verdict_from_rows`,
   helper `seeded_rows(&[&str])` frère de `seeded`, et le bout-en-bout mika#2762
   (D2).

Chaque étape compile ; 1→3→4 est la séquence obligée (le type remonte), 2 est
indépendante, 5 et 6 ferment.

---

## Verification Contract

| # | vérification | commande |
|---|---|---|
| V1 | suite complète verte | `cargo test` |
| V2 | les 6 tests purs de `groom_escalate_verdict` sont verts **sans avoir été modifiés** (contrôle R3) | `cargo test -p mika-agent mika2545` + `git diff --stat` sur `executor.rs:12965-13024` |
| V3 | la nouvelle sélection : le cas mesuré, le contrôle négatif, le `None`, la liste vide | `cargo test -p mika-agent mika2795` |
| V4 | les quatre gardes mika#2545 restent verts, dont le `match` sans joker et le format de fil à **deux** entrées | `cargo test -p mika-agent mika2545_the_` |
| V5 | les gardes mika#2638 : plancher à ≥ 5 comparaisons et lecteur unique des variantes | `cargo test -p mika-agent mika2638` |
| V6 | le marqueur de convergence n'a toujours qu'un lecteur (mika#2590) | `cargo test -p mika-agent mika2590` |
| V7 | registre des jetons canoniques | `cargo test -p mika-agent canonical_tokens` && `scripts/check-canonical-tokens.sh` |
| V8 | lint + format | `cargo clippy --all-targets -- -D warnings` && `cargo fmt --check` |
| V9 | pipeline | `scripts/verify-pipeline.sh origin/main` |

Preuve manuelle à joindre au corps de PR : la ligne `groom_verdict_selected`
produite par le test D2, montrant `rows_examined: 2`, `skipped_neutral: 1`,
`verdict: "escalated"` — c'est-à-dire la lecture de 18:30:55 rendue correcte.

---

## Risques

**R1 — le `None` fail-closed élargit la population `unreadable`.** Si une row
terminale à `result` NULL se glissait **devant** un verdict, le gate refuserait là
où il laissait passer. C'est le comportement voulu (le contrat mika#2545 est déjà
celui-là pour la tête de liste) et la population reste comptable par la sonde et
par `ALL_GROOM_ESCALATE_VERDICTS`. Le test `mika2545_a_terminal_callback_with_a_null
_result_is_some_none` est adapté, pas supprimé : il reste la preuve que le NULL se
distingue de l'absence.

**R2 — reformater le littéral SQL rougit `mika2638_…` à zéro marge.** Le plancher
exige ≥ 5 comparaisons `reference_url IN (` **comptées ligne par ligne**, et son
propre message le dit : scinder la colonne et son `IN (` sur deux lignes suffit à
le faire baisser. En retirant le `LIMIT 1` on réécrit la requête voisine : la ligne
`AND parent.reference_url IN (?2, ?3)` doit rester intacte. Mitigation : V5 en
cours d'implémentation, pas seulement en fin de pipeline.

**R3 — le renommage touche plus de sites que prévu.** Mesuré au grooming : deux
sites de production (`db/tasks.rs`, `async_db.rs`), un appelant
(`groom_verdict_state`), les 9 tests DB, et **une prose de panic**
(`canonical_tokens.rs:5696`) qui nomme le symbole. Cette dernière est le site qu'un
`grep` sur les appels manque. Mitigation : `grep -rn latest_groom_verdict_for_issue
crates/ scripts/` doit rendre zéro avant V9.

**R4 — la sonde n'existe qu'après `make deploy`.** Son **absence** dans les logs
signifie « binaire antérieur au correctif » (classe mika#2340), jamais « rien n'a
été sauté ». Écrit dans le doc-comment du site d'émission, parce que c'est la
mauvaise lecture par défaut.

**R5 — fausse symétrie avec la sœur immunisée.** Un futur lecteur pourrait croire
que `has_completed_groom_for_issue` a le même défaut et « l'aligner ». Mitigation :
le doc-comment du nouveau lecteur nomme explicitement la sœur et dit pourquoi elle
ne bouge pas (elle décide déjà en Rust).

---

## Definition of Done

- `groom_callback_results_newest_first` rend toutes les rows terminales de classe
  `groom` du ticket, ordonnées du plus récent au plus ancien, sans pré-filtre SQL.
- `groom_verdict_from_rows` est pure, sans accès DB, et traite les quatre cas du
  tableau de la pièce 4 par un `match` exhaustif.
- `groom_escalate_verdict` est inchangée et ses six tests purs sont verts sans
  avoir été édités.
- Le site d'appel du gate (`validate_dispatch_readiness`) est inchangé : même
  `match` exhaustif sans bras joker, même `warn!`, même `log_audit_event`.
- `groom_verdict_selected` est émis sur **toutes** les branches, y compris
  `skipped_neutral == 0`.
- `ALL_GROOM_ESCALATE_VERDICTS` compte toujours deux entrées.
- Aucune occurrence résiduelle de `latest_groom_verdict_for_issue` dans
  `crates/` ni `scripts/`.
- V1 à V9 verts.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son Constat, de sa section *Ce qui manque*, et du
Verification Contract.

**AC1 — le cas mesuré ne se reproduit pas.** Deux callbacks de groom terminaux sous
la même tâche parente : un `Outcome: ESCALATE — first-pass` à T, puis un
`deferred dispatch slot freed` à T+7 min. Un dispatch `dev-groom` avec
`originating_message = None` est **refusé** avec `dispatch_groom_escalated` et
`verdict = "escalated"`. (Test D2 ; sans le correctif ce dispatch passe — c'est la
reproduction exacte de 18:30:55.)

**AC2 — contrôle négatif : une row neutre seule ne refuse rien.** Un unique
callback `deferred dispatch slot freed`, aucun verdict en base : le dispatch
**traverse**. Le tri saute les rows neutres, il ne les traite pas comme un refus.

**AC3 — un ESCALATE suivi d'un vrai PLAN_GROOMED laisse repartir.** Le frein de
mika#2545 reste un frein et non un mur : le test
`mika2545_an_escalate_followed_by_a_groomed_reads_not_escalated` est vert après
adaptation, et le reste **avec** une row neutre intercalée entre les deux.

**AC4 — le NULL se distingue du neutre.** Une row terminale à `result` NULL en tête
rend `Unreadable` et refuse ; une row neutre en tête est sautée. Les deux
populations opérateur restent séparées.

**AC5 — la sonde est le contrôle positif.** `groom_verdict_selected` est émise sur
chaque appel de `groom_verdict_state`, avec `skipped_neutral` et `rows_examined`
renseignés, y compris quand rien n'est sauté.

**AC6 — le prédicat est ancré en ligne.** Un `result` qui *mentionne*
`Outcome: ESCALATE` en milieu de phrase, sans ligne ancrée, est traité comme
neutre et sauté — pas comme un verdict. (Classe mika#2050 / mika#2105 ; le test
frère `mika2545_a_quoted_marker_mid_line_is_not_a_verdict` existe déjà pour l'autre
prédicat et reste vert.)

**AC7 — le comportement a bougé, il n'a pas été réécrit.** Les six tests purs de
`groom_escalate_verdict` sont verts et le diff de `executor.rs:12965-13024` est
vide.

**AC8 — aucun garde structurel n'est ouvert.** V4 à V7 verts, allowlists
inchangées, `ALL_GROOM_ESCALATE_VERDICTS` toujours à deux entrées.

---

## Fire-Disposition

Le plan livre des détecteurs : tous sont **comportementaux** (tests sur les
nouveaux chemins), aucun n'est une règle de lint ni un scan de source.

**Disposition retenue : (a) exception nommée en allowlist — et l'allowlist est
livrée VIDE, parce que la direction a été choisie pour n'avoir besoin d'aucune
exception.** Les détecteurs portent sur du code qui naît avec ce vol : il n'existe
aucune violation préexistante à excepter. C'est l'idiome de la maison (mika#2425,
mika#2457, mika#2759).

1. **`mika2795_le_lanceur_differe_ne_masque_plus_un_escalate`** (D2, nouveau,
   bout-en-bout via `validate_dispatch_readiness`) — la reproduction de 18:30:55.
   Il atterrit **armé et rouge sur le HEAD actuel**, vert après la pièce 4 : c'est
   la démonstration que le détecteur atteint la faute qu'il nomme, et non une
   décoration (classe mika#2638 D5).
2. **`mika2795_une_row_neutre_seule_ne_refuse_rien`** (nouveau) — le contrôle
   négatif d'AC2. Sans lui, un tri qui refuserait *tout* passerait AC1 : la paire
   est ce qui sépare « saute les neutres » de « refuse tout ».
3. **`mika2795_la_couche_db_rend_la_row_neutre`** (D1, nouveau) — l'assertion
   **auto-nettoyante** exigée par (a), appliquée à la doctrine mika#2590 : elle
   affirme que `groom_callback_results_newest_first` **retourne** la row du lanceur
   différé au lieu de la filtrer. Elle rougit le jour où quelqu'un « optimise » la
   requête avec un `WHERE label != …` ou un `instr(result, …)`, c'est-à-dire le
   jour exact où le compteur `skipped_neutral` cesserait de mesurer la population
   qu'il existe pour mesurer.
4. **`mika2795_la_sonde_porte_le_compteur_meme_a_zero`** (nouveau) — le contrôle
   positif d'AC5, qui rougit si la sonde devient conditionnelle. Une sonde qui ne
   parle que quand quelque chose est sauté rend « rien sauté » indistinguable de
   « pas de sonde » (mika#2205).
5. **Détecteurs existants, aucune exception demandée.** Les quatre gardes mika#2545
   (`canonical_tokens.rs:3869-3995`) et les trois gardes mika#2638
   (`:5696` et suivants) restent verts **sans modification de leur prédicat** :
   seule la prose de panic du plancher est mise à jour (livrable 2), et le compte
   `reference_url IN (` est préservé par R2. **Si l'un d'eux rougit,
   l'implémentation s'arrête et remonte (option c)** — ni
   `GROOM_ESCALATE_SOLE_WRITER_EXCEPTIONS`, ni `REFERENCE_URL_LIKE_ALLOWED`, ni le
   plancher de 5 ne s'ouvrent sous l'autorité de ce vol.
6. **Les 9 tests DB mika#2545 sont *migrés*, pas exceptés.** Leur assertion change
   de forme avec le type de retour (`is_none()` → `is_empty()`, `expect(…)` →
   `[0]`) ; aucun n'est désarmé, aucun n'est supprimé, et aucun ne change de
   question. Un test supprimé ici serait une exception non nommée.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/tasks.rs` — requête sans `LIMIT`, retour `Vec<Option<String>>`, doc-comment réécrit | 55 |
| `crates/mika-agent/src/canonical_tokens.rs` — prose du panic du plancher mika#2638 | 3 |
| `crates/mika-agent/src/async_db.rs` — enveloppe renommée | 15 |
| `crates/mika-agent/src/skills/executor.rs` — préfixe, prédicat, sélection pure, lecteur + sonde | 125 |
| `crates/mika-agent/src/skills/executor.rs` — tests (sélection pure + bout-en-bout D2) | 110 |
| `crates/mika-agent/src/db/tests/task_messages_and_groom.rs` — adaptation des 9 tests + D1 | 85 |

Total estimé : 393 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 393 < 429 et 393 < 1000 : les deux
bornes passent, la plus serrée avec 36 lignes de marge. Le périmètre est fermé par
construction : **cinq** fichiers touchés, **zéro** fichier neuf, **zéro** migration
de schéma, **une** signature publique modifiée (`db` + `async_db`, même fonction)
dont l'unique appelant de production est réécrit dans le même vol. Les 9 tests DB
sont **adaptés** mécaniquement, pas réécrits : leur fixture
(`GROOM_CALLBACK_ESCALATE`) et leur helper (`set_created_at`) sont réutilisés tels
quels, et le site d'appel du gate — la partie coûteuse à toucher, 130 lignes de
`match` exhaustif, `warn!` et audit — n'est **pas** touché, parce que la signature
de `groom_verdict_state` est préservée délibérément.

Le poste qui aurait fait déborder a été écarté et nommé : le scan de source
« un seul lecteur de `GROOM_OUTCOME_LINE_PREFIX` », ~85 lignes, écarté en
*Hors périmètre* avec sa raison et sa condition de réouverture. Avec lui le total
serait à 478, au-dessus de la borne, pour une classe que le voisinage immédiat du
préfixe et de son lecteur rend aujourd'hui improbable.
