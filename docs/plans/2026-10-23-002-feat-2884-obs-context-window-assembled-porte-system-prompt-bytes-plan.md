# mika#2884 — `context_window_assembled` porte la taille du prompt système

> Un champ sur une ligne existante. Le travail n'est pas le champ : c'est de
> démontrer que la valeur portée est **la même** que celle du voisin qui la
> rapporte déjà, et de dire honnêtement ce que le ticket gagne — une jointure
> évitée, pas une mesure sauvée.

## 1. Contexte

`context_window_assembled` (`crates/mika-agent/src/agent_loop/mod.rs:10355`,
brique 0 de mika#2295) est la **ligne d'attribution** d'un tour : elle décompose
ce qui a rempli la fenêtre de contexte — `history_bytes`,
`user_message_bytes`, `tool_defs_bytes`, `message_count`, `distinct_sessions`,
`history_scope`, `oldest_age_secs`, `truncated_messages`, `truncated_bytes`.
Elle omet le prompt système, qui est pourtant le composant le plus lourd et le
seul qu'un changement de dépôt fait bouger sans qu'aucune conversation ne change
(le CLAUDE.md de 825 Ko du 2026-10-13).

mika#2593 avait déjà nommé ce reliquat :
`docs/plans/2026-10-16-005-feat-2593-peser-tool-defs-bytes-arch-plan.md:669`
liste « **(b) `system_prompt_bytes` sur `context_window_assembled`** » parmi les
suites. Ce ticket est ce (b).

### 1.1 Une correction de prémisse, parce que mika-arch la demandera

Le corps du ticket dit que la taille du prompt système « n'existe que sur
`turn_usage` » et qu'elle est « perdue » quand le tour meurt avant.
**Inexact sur la valeur, exact sur le nom du champ.** Trois surfaces la portent
déjà, à des degrés différents :

| surface | champ | quand | couverture |
|---|---|---|---|
| `system_prompt_assembled` (mika#1217, `mod.rs:10079`, appelé `mod.rs:5924`) | **`total_bytes`** | **avant** l'appel au modèle | tous les tours, non conditionné |
| `brief_size_overrun` (mika#2474, `mod.rs:10634`) | `system_prompt_bytes` | avant l'appel au modèle | uniquement au franchissement de seuil |
| `turn_usage` (mika#1889 / mika#2331, `mod.rs:10857`) | `system_prompt_bytes` | **après** l'appel au modèle | tours qui atteignent la réponse |

Donc le scénario « prompt trop long refusé au premier tour » n'efface pas la
mesure : `system_prompt_assembled` est émis avant l'appel et la porte, sous un
**autre nom**. Ce que le ticket décrit vraiment, et qui reste un gain réel :

- **la jointure évitée.** Pour lire ce qui a rempli une fenêtre, il faut
  aujourd'hui joindre deux ou trois événements par `trace_id`. Après ce
  changement, **une seule ligne** porte tous les composants de la fenêtre, et
  l'opérateur qui lit `context_window_assembled` n'a plus de `grep` croisé à
  faire.
- **le nom.** `total_bytes` sur un événement nommé `system_prompt_assembled` est
  déjà la mesure, mais `grep system_prompt_bytes` ne le trouve pas. Le ticket a
  raison sur le champ.

Le plan livre donc le champ demandé **et** corrige au passage l'inexactitude
documentaire qui est à l'origine de la prémisse : `observability.md:25` annonce
`system_prompt_assembled (system_prompt_bytes, …)` là où le code émet
`total_bytes`. C'est très probablement ce que le rédacteur du ticket a lu.

### 1.2 La tension de doctrine, traitée d'avance

`docs/agent-internals/observability.md` refuse **deux fois** un champ sur
l'événement voisin : mika#2474 pour un booléen `oversized`, mika#2783 pour le
recensement par outil (« Why a distinct event rather than a field on the
sibling. It must grep alone »). Les deux refus portaient sur une **nouvelle
mesure ayant besoin de son propre grep**.

Ici le besoin est l'inverse : la valeur ne doit **pas** grepper seule — elle
existe déjà sur trois lignes — elle doit être **sur la ligne d'attribution**.
Le précédent qui gouverne est celui de mika#2474, § « `system_prompt_bytes`
rides on the line » (`observability.md:62-73`), que le doc-comment de
`report_brief_size_overrun` reformule : *« the same value, at the same instant,
on the line that needs it »*. Le même argument, appliqué à la ligne qui
décompose la fenêtre.

Les lignes 28–31 d'`observability.md` mettent en garde contre « two sources of
truth for one value ». La garde tient parce qu'il n'y a **pas** de seconde
source : les trois émissions dérivent toutes de `system.len()` sur le même
`String`, et le § 3.3 démontre qu'elles sont égales par construction, pas par
coïncidence.

## 2. Direction retenue

**Un paramètre de plus sur l'émetteur, nourri par `system.len()` au site
d'appel. Rien dans `ContextWindowFields`.**

```rust
fn emit_context_window_assembled(
    agent_id: &str,
    session_id: &str,
    trace_id: &str,
    mode: &str,
    fields: &ContextWindowFields,
    system_prompt_bytes: usize,   // ← nouveau
)
```

Trois raisons, dans l'ordre où elles mordent :

1. **`build_context_window_fields` est pure sur d'autres entrées.** Sa signature
   est `(history, scope, tool_defs, truncated_messages, truncated_bytes, now)`.
   Le prompt système n'en est pas une, et ne peut pas l'être : il est assemblé
   ailleurs, par un autre chemin. Y faire entrer la valeur serait élargir une
   fonction pure pour y faire transiter une donnée qu'elle ne calcule pas.
2. **Le précédent est à trois lignes du site d'appel.** `mod.rs:6134` passe déjà
   `system.len()` en argument positionnel à `report_brief_size_overrun`, une
   forme que mika#2474 a choisie délibérément. Les deux appels deviennent
   littéralement comparables au diff — même expression, même instant, deux lignes
   d'écart.
3. **Zéro site de test touché.** `ContextWindowFields` est construit par ~14
   tests unitaires (`mod.rs:19546-19718+`). Élargir la struct les fait tous
   bouger pour une valeur qu'aucun d'eux n'a à produire ; élargir l'émetteur
   n'en touche aucun, parce que l'émetteur n'a **qu'un seul appelant**
   (`observability.md:207` : *« `emit_context_window_assembled` has **one**
   caller; the silent and team paths do not build `ContextWindowFields` »*).

### 2.1 Alternative nommée et écartée

Lier le retour de `emit_system_prompt_assembled` — qui rend déjà
`Option<i64>` et dont le site d'appel écrit `let _ = …` (`mod.rs:5924`) —
rendrait l'égalité avec `system_prompt_assembled` **structurelle** plutôt que
démontrée. Écarté : les types divergent (`Option<i64>` contre `usize`), le site
voisin lit `system.len()` directement, et une égalité structurelle avec
`system_prompt_assembled` n'est pas celle qu'AC2 demande — AC2 porte sur
`turn_usage`, qui dérive de `system_prompt_len` (`mod.rs:1694`), un troisième
chemin. Reprendre la **même expression que le voisin immédiat** est ce qui rend
les deux lignes relisables ensemble.

### 2.2 `usize` nu, pas `Option`

Au site d'émission, le prompt système est un `String` assemblé, en main. « Une
mesure qui n'a pas eu lieu » y est inatteignable : le motif mika#2331 (*absent
n'est jamais zéro*) tient **par construction**, sans `Option` pour le dire.
C'est exactement la forme de `report_brief_size_overrun`, et c'est pourquoi
`turn_usage` — lui, émis depuis un chemin où le prompt peut être absent — garde
son `Option<i64>` et son sigil `?`. Les deux formes sont justes à leur place.

## 3. Mécanisme

### 3.1 L'émetteur

`mod.rs:10355`, une ligne de champ ajoutée dans le `info!`, **à côté de**
`tool_defs_bytes` plutôt qu'en fin de liste : les trois composants de la
fenêtre (`history_bytes`, `user_message_bytes`, `tool_defs_bytes`) se lisent
ensemble, et `system_prompt_bytes` est le quatrième.

```rust
        system_prompt_bytes = system_prompt_bytes,
```

Valeur nue, sans sigil — voir § 3.4, c'est load-bearing pour AC2.

Le doc-comment de la fonction reçoit un paragraphe court, sur le modèle du
§ « Why `system_prompt_bytes` rides along » de `report_brief_size_overrun` :
pourquoi le quatrième composant est ici (la jointure évitée, § 1.1), et pourquoi
il arrive en paramètre et non dans `ContextWindowFields` (§ 2).

### 3.2 Le site d'appel

`mod.rs:6092`, appel unique :

```rust
    emit_context_window_assembled(
        &db.agent_id,
        session_id,
        trace_id,
        "conversation",
        &context_window_fields,
        system.len(),
    );
```

Le commentaire déjà présent à `mod.rs:6121-6122` explique pourquoi
`system.len()` est en portée (*« `emit_system_prompt_assembled` above consumed
the prompt by reference »*) ; il couvre les deux appels et n'a pas besoin d'être
dupliqué.

### 3.3 Pourquoi les trois valeurs sont égales — par construction

AC2 n'est un invariant que si l'égalité est démontrable. Elle l'est, sur trois
lectures du même `String` :

| lecture | ligne | expression |
|---|---|---|
| ce que le nouveau champ portera | `mod.rs:6092` | `system.len()` |
| ce qui partira au modèle | `mod.rs:6307-6314` | `request.system = Some(system)` |
| ce que `turn_usage` rapportera | `mod.rs:1694` | `request.system.as_ref().map_or(0, \|s\| s.len())` |

Deux propriétés closent l'égalité :

1. **Rien ne mute `system` entre son assemblage (≈ `mod.rs:5934`) et son
   déplacement dans `LlmRequest` (`mod.rs:6310`)** — vérifié par grep sur la
   fenêtre. Le `system.len()` du nouveau champ et le `request.system.len()`
   du côté provider sont donc le même nombre.
2. **`system_prompt_len` est calculé AVANT la boucle de steps.** `mod.rs:1694`
   est hors du `for step in 0..max_steps` qui démarre à `mod.rs:1697`, donc avant
   le *nudge* de fin de budget qui fait `system.push_str(nudge)` à
   `mod.rs:1750-1764`. `turn_usage` rapporte le prompt **tel qu'envoyé au premier
   appel** — exactement ce que `context_window_assembled` mesure.

C'est cette seconde propriété qui rend AC2 non trivial : sur un tour multi-steps
avec nudge, une implémentation naïve de `turn_usage` aurait divergé. Elle ne
diverge pas, et le test le fige.

### 3.4 L'asymétrie de sigil, qui décide de la forme de l'assertion AC2

Le capteur de tracing des tests transcrit les champs en `String` via
`FieldVisitor`. Deux chemins, deux rendus :

- `turn_usage` émet `system_prompt_bytes = ?fields.system_prompt_bytes`
  (`mod.rs:10857`) — le sigil `?` sur un `Option<i64>` passe par
  `record_debug`, donc le capteur voit la chaîne **`"Some(1234)"`**.
- le nouveau champ émet une valeur nue sur un `usize` — `record_u64`, donc le
  capteur voit **`"1234"`**.

L'assertion AC2 doit donc comparer :

```rust
assert_eq!(
    turn_usage.get("system_prompt_bytes").map(String::as_str),
    Some(format!("Some({cwa_bytes})").as_str()),
    "…"
);
```

C'est écrit ici pour que le vol n'ait pas à le découvrir en rouge. Ne pas
« corriger » le sigil de `turn_usage` au passage : le ticket dit *« aucun autre
changement »*, et le `?` y est juste (§ 2.2).

### 3.5 Le capteur réutilisé, pas réécrit

`test_context_scope_observability_2305.rs` expose `pub(crate) type Captured` et
`pub(crate) fn capture()` **depuis mika#2877, précisément pour un second
consommateur**. Ce ticket est le troisième. Le nouveau fichier les importe via
`use super::test_context_scope_observability_2305::{capture, Captured};` — et
hérite ainsi de `keep_registry_multi_dispatcher()` et de son raisonnement sur le
cache d'intérêt mono-dispatcher de `tracing-core` 0.1.36 (sans quoi 8 runs sur
25 capturent zéro événement sous `--features telemetry`).

Le doc-comment de `Captured` note que l'unification des cinq duplicatas dans un
`tests/eval/tracing_capture.rs` partagé mérite son propre ticket. Ce plan
**n'ouvre pas** ce chantier (§ 6) : il incrémente le compteur de consommateurs,
ce qui est l'argument de ce ticket futur, pas son exécution. Le doc-comment de
`Captured` reçoit une mention du troisième consommateur, deux lignes.

## 4. Livrables et ordre de commit

Un seul commit, mais dans cet ordre d'écriture — le rouge-avant d'AC3 exige que
le test existe avant le champ :

1. `crates/mika-agent/tests/eval/test_context_window_system_prompt_bytes_2884.rs`
   (AC1 + AC2, en-tête `//!` portant la recette de rouge-avant) ;
   `mod test_context_window_system_prompt_bytes_2884;` dans
   `crates/mika-agent/tests/eval.rs`, avec sa ligne de commentaire explicative
   comme ses voisines ; mention du troisième consommateur sur le doc-comment de
   `Captured`.
2. **Exécuter AC1 : il doit rougir** (§ 5.4). C'est AC3, et il n'est satisfait
   qu'une fois, à cet instant.
3. `crates/mika-agent/src/agent_loop/mod.rs` — le paramètre (`:10355`), le champ
   émis, le doc-comment, le site d'appel (`:6092`).
4. `docs/agent-internals/observability.md` — AC4 (§ 6 pour le périmètre exact).
5. Suite complète, clippy, fmt, `scripts/verify-pipeline.sh origin/main`,
   `scripts/check-secrets.sh`.

## 5. Verification Contract

### 5.1 AC1 — le champ est là, et il vaut la bonne chose

`mika2884_la_fenetre_assemblee_porte_la_taille_du_prompt_systeme`

Un tour nominal via `EvalHarness` (`responses(vec![text_response("…")])`), le
capteur installé avant `harness.run(…)`. Deux lectures, une égalité :

- la référence : `trace.captured_requests[0].system.as_ref().expect("le tour doit avoir atteint le provider avec un prompt système").len()` ;
- la mesure : `fields["system_prompt_bytes"]` de l'unique événement
  `context_window_assembled`.

L'assertion est `assert_eq!(mesure, reference.to_string())`, pas un
`assert!(contains)` ni un `> 0` : un prompt système mesuré à zéro passerait un
test de présence, et c'est exactement le mode de panne que le motif
mika#2331 nomme.

Le helper qui isole l'événement reprend la forme de `the_window_event` de
mika#2305 — **exactement un** `context_window_assembled` par tour — parce que la
sonde opérateur du § 7 lit les lignes qu'elle obtient, et qu'un tour qui
émettrait l'événement deux fois rendrait cette sonde ambiguë sans rien faire
rougir.

### 5.2 AC2 — la même valeur que `turn_usage`, sur le même tour

`mika2884_la_valeur_est_celle_que_turn_usage_rapporte_pour_le_meme_tour`

Même tour, même capture, deux événements extraits : `context_window_assembled`
et `turn_usage`. L'assertion est celle du § 3.4, avec le `format!("Some({})")`.

Le test assert **la présence** de `turn_usage` avant de comparer, avec un
message qui nomme la cause probable. Il ne doit jamais se replier
silencieusement sur « pas de `turn_usage`, donc rien à comparer » : un test qui
skippe ce qu'il ne trouve pas est le mode de panne de mika#2340 porté en
suite — vert, et muet sur le fait qu'il n'a rien vérifié.

> **Vérification au vol, à faire avant d'écrire l'assertion :** confirmer que
> `emit_turn_usage` est bien atteint sous `MockLlmProvider` (un tour qui répond
> en texte doit le traverser). S'il ne l'est pas, le repli autorisé est
> d'allonger le tour d'un appel d'outil pour l'atteindre — **pas** de changer la
> cible d'AC2 : le ticket nomme `turn_usage`, et c'est la jointure que ce
> changement supprime.

### 5.3 Pourquoi un test de harnais et pas un test unitaire

Un unitaire sur `emit_context_window_assembled` prouverait que l'émetteur rend
l'argument qu'on lui passe. Il resterait vert si le site d'appel passait
`0`, ou la longueur d'un autre `String`. C'est la forme mika#2205 — *un prédicat
correct jamais appelé* — et la même raison pour laquelle mika#2305 a écrit un
test de harnais plutôt qu'un unitaire sur `build_context_window_fields`. Seul un
tour réel, lu sur l'événement émis **et** sur
`trace.captured_requests[0].system`, relie les deux bouts.

### 5.4 La recette de rouge-avant (AC3)

Écrite dans l'en-tête `//!` du fichier de test, pour qu'elle soit rejouable
après le merge :

1. Revenir au `mod.rs` d'avant l'étape 3 du § 4 (ou retirer la ligne
   `system_prompt_bytes = system_prompt_bytes` du `info!`).
2. `cargo test -p mika-agent --test eval mika2884_la_fenetre_assemblee` →
   **rouge**, sur `None` au lieu de `Some("<n>")`, et non sur une panique de
   compilation.
3. Remettre le champ → vert.

AC2 est rouge au même instant et pour la même raison ; AC1 est le critère
nommé par le ticket, c'est donc lui que la recette cite.

### 5.5 Non-régression

- Les ~14 appels unitaires de `build_context_window_fields`
  (`mod.rs:19546-19718+`) sont **inchangés** — vérifiable au diff : la struct
  ne bouge pas.
- `report_brief_size_overrun` et son appel (`mod.rs:6128-6139`) sont inchangés
  octet pour octet.
- `emit_system_prompt_assembled` et son `total_bytes` sont inchangés — seul le
  document qui le décrit mal est corrigé.
- Les deux tests de mika#2305 restent verts : leur helper assert un unique
  `context_window_assembled`, et ce plan n'en ajoute aucun.
- `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main`,
  `scripts/check-secrets.sh`.

## 6. Hors périmètre

- **Renommer `total_bytes` en `system_prompt_bytes` sur
  `system_prompt_assembled`.** Ce serait un changement de schéma sur un
  événement émis depuis mika#1217, avec des sondes opérateur qui le grepent.
  Le ticket dit *« aucun autre changement »*. Le plan **documente**
  l'écart (§ 1.1, AC4) plutôt que de le résorber.
- **Toucher au sigil `?` de `turn_usage`.** Juste à sa place (§ 2.2, § 3.4).
- **Unifier les cinq capteurs de tracing dupliqués** dans un
  `tests/eval/tracing_capture.rs` partagé. Nommé par le doc-comment de
  `Captured` comme méritant son propre ticket ; ce vol en est le troisième
  consommateur, ce qui renforce l'argument sans l'exécuter.
- **`docs/configuration.md:658`**, qui porte une sonde
  `grep context_window_assembled | jq`. AC4 ne nomme qu'`observability.md`, et la
  sonde existante reste correcte — elle lit des champs, pas leur liste
  exhaustive.
- **Les chemins `silent` et `team`.** Ils ne construisent pas
  `ContextWindowFields` et n'émettent pas l'événement
  (`observability.md:207`) ; rien à y porter.
- **Tout seuil, toute alerte sur la nouvelle valeur.** `brief_size_overrun`
  (mika#2474) est déjà le détecteur de seuil pour cette mesure. En ajouter un
  second serait la seconde source de vérité contre laquelle
  `observability.md:28-31` met en garde.

## 7. Comment on lit la mesure en production

Après `make deploy` — et **pas avant** : l'instrument n'est pas vivant tant que
le binaire servi précède le changement (motif mika#2340 ; une ligne de log
absente dit « binaire en retard », jamais « valeur nulle »).

La décomposition complète d'une fenêtre, en une ligne et sans jointure :

```bash
grep context_window_assembled /var/log/mika/server.log \
  | jq -r 'select(.fields.agent_id == "mika-dev")
           | [.timestamp, .fields.system_prompt_bytes, .fields.history_bytes,
              .fields.tool_defs_bytes, .fields.user_message_bytes,
              .fields.message_count] | @tsv'
```

Ce qui se lit dessus, et qui n'était pas lisible avant :

- **la part du prompt système dans la fenêtre**, tour par tour, sans toucher à
  `turn_usage` ;
- **un saut de `system_prompt_bytes` à histoire constante** — signature d'un
  changement de dépôt (CLAUDE.md, skill bundled), pas d'un changement de
  conversation ;
- **les tours morts avant `turn_usage`** apparaissent désormais avec leur taille
  de prompt sur la ligne d'attribution, au lieu d'exiger un second `grep` sur
  `system_prompt_assembled` et son nom de champ divergent.

Un champ **absent** sur cette ligne signifie « binaire antérieur à ce
changement ». Il ne signifie pas « prompt système vide » : au site d'émission le
prompt est un `String` en main (§ 2.2), donc zéro n'est pas un état atteignable
pour un tour qui a émis l'événement.

## 8. Risques

| risque | lecture |
|---|---|
| Volume de log | Un entier décimal de ≤ 6 chiffres sur une ligne émise 3 041 fois/jour : **≈ 80 Ko/jour** dans le pire cas, sur un `server.log` qui porte déjà 16 351 `turn_usage`. Négligeable. |
| Coût CPU | `system.len()` sur un `String` : lecture d'un champ de longueur, O(1). Le voisin `report_brief_size_overrun` le fait déjà trois lignes plus bas, au même instant. |
| Fuite de secret | Impossible : seule une **longueur** transite, jamais le contenu du prompt. |
| Divergence future des trois mesures | Réelle mais tenue par AC2, qui rougit si `turn_usage` et la ligne d'attribution cessent de s'accorder. Le point de rupture identifié est le *nudge* de `mod.rs:1750-1764` ; le § 3.3 démontre pourquoi il ne mord pas aujourd'hui, et AC2 le surveille. |
| Fragilité du capteur de tracing en test | Réelle et connue — c'est ce que `keep_registry_multi_dispatcher()` traite. Raison exacte de réutiliser le capteur éprouvé de mika#2305 (§ 3.5) plutôt que d'en écrire un sixième. |
| Confusion de nom avec `total_bytes` | C'est l'origine de la prémisse du ticket (§ 1.1). Traitée par l'édition AC4, qui nomme le champ réel de `system_prompt_assembled` au lieu de le laisser mal documenté. |

## 9. Fire-Disposition

**Sans objet pour ce vol, et la section n'est pas inventée** (mika#2306, options
mika#1574). Les candidats au statut de détecteur, refusés un par un :

- **AC1 est un test fonctionnel.** Son chemin de succès est « l'événement émis
  porte la bonne valeur », pas « aucune violation trouvée ». Il n'a aucune
  population à balayer : il exerce un tour unique qu'il a lui-même scripté.
- **AC2 est une assertion d'égalité entre deux champs du même tour**, sur un
  tour composé par le test. Il ne peut rien *découvrir* d'une population
  existante — et la valeur qu'il assert est introduite par cette PR même, donc
  il n'existe aucune violation préexistante à exempter.
- **La sonde du § 7 est une commande d'opérateur dans un document.** Rien ne
  l'exécute, rien ne rougit.
- **Aucun lint, scan structurel, garde CI, validateur de schéma ou garde
  EndTurn n'est ajouté.** `crates/mika-agent/src/canonical_tokens.rs` ne porte
  aucune entrée sur `context_window` ni `system_prompt` (grep : zéro
  correspondance) ; ce plan n'en ouvre pas.

Fabriquer ici une allowlist serait créer *« un endroit où déposer la prochaine
infraction »* (mika#2201 / mika#2323) pour une population vide par construction.

Et `scripts/check-suite-wiring.sh` n'ouvre aucune entrée : le test livré vit
sous `crates/mika-agent/tests/eval/`, que `cargo test -p mika-agent --test eval`
exécute déjà en CI.

## 10. Definition of Done

- [ ] `emit_context_window_assembled` prend `system_prompt_bytes: usize` et émet
      `system_prompt_bytes` **sans sigil**, placé à côté de `tool_defs_bytes`.
- [ ] Son doc-comment dit pourquoi le champ est là (jointure évitée) et pourquoi
      il arrive en paramètre et non dans `ContextWindowFields`.
- [ ] Le site d'appel unique (`mod.rs:6092`) passe `system.len()` — la même
      expression que `report_brief_size_overrun` trois lignes plus bas.
- [ ] `ContextWindowFields` et `build_context_window_fields` sont **inchangés**,
      et aucun des ~14 appels unitaires ne bouge — vérifiable au diff.
- [ ] `test_context_window_system_prompt_bytes_2884.rs` livre AC1 et AC2, avec
      la recette de rouge-avant dans son en-tête `//!`.
- [ ] `mod test_context_window_system_prompt_bytes_2884;` ajouté à
      `tests/eval.rs` avec sa ligne de commentaire ; le doc-comment de
      `Captured` mentionne le troisième consommateur.
- [ ] AC1 vert, sur une égalité avec
      `trace.captured_requests[0].system.…len()`, pas sur une présence.
- [ ] AC2 vert, avec la comparaison `format!("Some({}))"` du § 3.4, et une
      assertion de présence de `turn_usage` qui échoue bruyamment plutôt que de
      skipper.
- [ ] La recette de rouge-avant du § 5.4 a été **exécutée** : AC1 rougit contre
      le code actuel, sur une valeur absente et non sur une erreur de
      compilation.
- [ ] `emit_system_prompt_assembled`, `report_brief_size_overrun` et
      `emit_turn_usage` sont inchangés octet pour octet.
- [ ] Les deux tests de mika#2305 toujours verts.
- [ ] `docs/agent-internals/observability.md` liste `system_prompt_bytes` parmi
      les champs de `context_window_assembled`, porte la lecture opérateur du
      § 7 et le raisonnement du § 1.2 (pourquoi un champ ici alors que le
      document refuse ailleurs un champ sur le voisin), et nomme le champ réel
      de `system_prompt_assembled` (`total_bytes`) à sa ligne 25.
- [ ] `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main`,
      `scripts/check-secrets.sh` verts.

## 11. Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2884 :

- AC1 : un test du harnais assemble un tour et vérifie que
  `context_window_assembled` porte `system_prompt_bytes`, égal à la longueur en
  octets du prompt système envoyé.
- AC2 : sur le même tour, la valeur est égale au `system_prompt_bytes` de
  `turn_usage`.
- AC3 : rouge-avant, AC1 rougit contre le code actuel.
- AC4 : `docs/agent-internals/observability.md` liste le nouveau champ.

## 12. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/mod.rs` (paramètre + champ émis + doc-comment + site d'appel) | 20 |
| `crates/mika-agent/tests/eval/test_context_window_system_prompt_bytes_2884.rs` (en-tête `//!` + helpers + AC1 + AC2) | 130 |
| `crates/mika-agent/tests/eval/test_context_scope_observability_2305.rs` (mention du troisième consommateur) | 3 |
| `crates/mika-agent/tests/eval.rs` (ligne `mod` + commentaire) | 2 |

Total estimé : 155 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 155 est à 44 % sous la borne par
vol (279) et très loin du seuil par PR (1000). Trois traits du périmètre
soutiennent ce chiffre plutôt qu'un optimisme :

1. **La production est un paramètre et un champ.** Pas de nouveau module, pas de
   couche pure, pas de nouvelle struct — le § 2 écarte explicitement
   l'élargissement de `ContextWindowFields`, qui est la seule variante de ce
   ticket capable de faire exploser le compte (~14 sites de test unitaires).
2. **Le site d'appel est unique**, attesté par `observability.md:207` et par le
   grep du § 3.3. Aucune recherche de sites à faire au vol, aucun risque de
   découvrir un deuxième appelant à mi-parcours.
3. **Le capteur de tracing est réutilisé, pas écrit** (§ 3.5) : `capture()` et
   `Captured` sont déjà `pub(crate)` depuis mika#2877. Les ~85 lignes de
   `CapturingLayer` / `FieldVisitor` /
   `keep_registry_multi_dispatcher` — et leurs deux jours de débogage sur le
   cache d'intérêt mono-dispatcher — ne sont ni recopiées ni redécouvertes.

Le poste dominant est le fichier de test, et il l'est pour une raison qui ne
dérive pas : deux `#[tokio::test]` sur le même modèle que mika#2305, plus
l'en-tête qui porte la recette de rouge-avant et l'asymétrie de sigil du § 3.4.
Les éditions `docs/` (AC4) sont hors du compte par la règle, et le § 6 borne
leur périmètre à `observability.md`.
