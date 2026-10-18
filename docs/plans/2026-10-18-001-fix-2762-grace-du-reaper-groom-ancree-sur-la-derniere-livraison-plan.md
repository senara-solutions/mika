# mika#2762 — La grâce du reaper groom s'ancre sur la **dernière** livraison, et un lanceur différé n'ancre rien

- **Ticket :** senara-solutions/mika#2762 (`bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`)
- **Précédents :** mika#2750 (PR #2754, le reaper groom), mika#2762 vol 1 (PR #2765, le rollback ciblé), mika#2405 (`find_settleable_dispatch_parents`, le précédent de forme SQL)
- **Branche :** `fix/2762/engine-le-reaper-groom-de-mika-2750-fait`

## 1. Où en est le ticket : le rollback est merged, le correctif ne l'est pas

Le vol 1 de ce ticket est déjà sur `main` (`6ab22e9e`, PR #2765). Il a retiré **l'appel**,
pas le prédicat :

- `task_engine/engine.rs:1352` appelle `reap_orphaned_parent_tasks("implement")` et
  **rien** pour `"groom"` ; les lignes 1353–1356 portent le commentaire de rollback.
- `reap_orphaned_parent_tasks` est restée `#[doc(hidden)] pub` *uniquement* pour que les
  deux tests de la classe `groom` de `tests/eval/test_groom_parent_reap_2750.rs` continuent
  d'épingler le balayage en l'appelant directement (lignes 221 et 264).
- Le header de ce fichier de test dit textuellement : *« Quand le correctif complet de
  mika#2762 rétablit l'appel groom dans le tick, ces deux tests repassent par
  `run_one_scan` »*.
- `mika2762_un_groom_reussi_pendant_quune_implementation_tient_la_place_garde_son_parent`
  (ligne 335) **existe déjà**, sème exactement la scène mesurée, et traverse `tick()`.
  Sous le rollback il est vert **pour la mauvaise raison** : le tick ne balaie pas `groom`.
  Rétablir l'appel est ce qui le rend significatif.

Ce vol ferme donc trois choses d'un bloc : le prédicat, le câblage, et la signification du
test déjà posé.

## 2. Le prédicat, et le seul piège du correctif

`Database::find_orphaned_parent_tasks` (`src/db/tasks.rs:2030`) aujourd'hui :

```sql
JOIN tasks child ON parent.id = child.parent_task_id
WHERE …
  AND COALESCE(child.dispatch_class, 'implement') = ?3
  AND child.trigger_type = 'callback'
  AND child.action_type  = 'resume_agent'
  AND child.status       = 'delivered'
  AND child.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)   -- ⬅ le trou
  AND NOT EXISTS (… sibling.status IN ('pending','in_progress'))
GROUP BY parent.id
```

Cette clause est **par ligne**. Il suffit donc qu'**un** enfant livré soit vieux pour que
le parent entre dans la population — l'enfant livré il y a une seconde est filtré *avant*
l'agrégat et ne proteste pas. Dans le cas mesuré, l'enfant vieux était le lanceur différé
`long_running:run_claude_pilot:deferred` (classe `groom`, `trigger_type=callback`,
`action_type=resume_agent`), livré 11 min plus tôt **quand il a lancé le groom**. Un groom
en deux passes a de surcroît toujours un enfant de passe 1 ancien.

**Le piège, et c'est le seul.** Le correctif n'est pas « ajouter un `HAVING MAX(...)` » : il
est de **retirer la clause `WHERE child.updated_at < …` et de la remplacer** par le
`HAVING`. Garder les deux reproduit exactement le défaut d'aujourd'hui — la `WHERE`
écarterait l'enfant récent avant l'agrégat, et le `MAX` ne verrait que les vieux. Le
précédent de forme est `find_settleable_dispatch_parents` (mika#2405, `tasks.rs:2549`) :
aucune clause `WHERE` sur `updated_at`, un `HAVING MAX(child.updated_at) < …`, et son
doc-comment explique déjà que cette différence de forme avec ses deux voisins est
intentionnelle. Ce vol aligne un des deux voisins sur elle.

SQLite refuse une fonction d'agrégat en `WHERE` : le `HAVING` n'est pas un choix de style.

### 2.1 Le second trou de la cause, et ce que ce vol en fait

Le ticket nomme une seconde cause : *le frère qui protège le parent n'existe pas encore*
quand la place implement est tenue (`global_dispatch_active`). Ce vol ne la ferme pas
frontalement — il la **désarme pour la fenêtre où elle détruisait quelque chose** :

- À T+1 s (le cas mesuré, deux fois), le dernier callback groom a livré il y a 1 s. La grâce
  n'est pas écoulée, le parent est épargné, et le tour de reprise a 600 s pour écrire ses
  callouts. C'est le dégât du ticket, et il est fermé.
- À T+600 s sans frère, le parent est fauché. **Résidu assumé, et il ne détruit rien :** à
  cet instant le tour de reprise a tourné, les callouts et le verdict sont dans le corps du
  ticket, le plan est poussé. Le parent passe `failed` à tort (verdict comptable faux), mais
  le ticket n'est plus ungroomed — le feeder le reprendra en **implement**, pas en groom. Le
  vol de groom n'est pas perdu.
- Fermer ce résidu demanderait un critère d'artefact de groom (« les callouts sont écrits »)
  que la table `tasks` ne porte pas, donc un livrable d'une autre nature. Hors périmètre
  (§ 7), recommandé en ticket séparé.

C'est le compromis que mika#2750 avait déjà nommé en D1 (*« callback délivré, pas de frère
actif, grâce écoulée **est** le critère »*) ; mika#2762 corrige lequel des callbacks ancre
la grâce, pas le critère.

## 3. Les livrables, site par site

### D1 — Le prédicat « ce n'est pas un lanceur différé », un seul site

Un `const` à côté de `DEFERRED_DISPATCH_LABEL` (`agent_loop/mod.rs:6391`, valeur
`long_running:run_claude_pilot:deferred`) :

```rust
/// Motif SQL (`LIKE`) des **lanceurs différés** — les enfants callback qui ne
/// disent rien de l'état d'un dispatch (mika#2762).
pub const DEFERRED_LAUNCHER_LIKE_PATTERN: &str = "%:deferred";
```

Un **suffixe**, pas la constante exacte, parce que le producteur du label est susceptible
de se spécialiser par classe (`…_groom:deferred`) et qu'un lanceur différé d'un autre nom
rouvrirait le trou en silence. Le lien entre les deux est **asserté**, pas espéré — dans un
`#[test]`, sibling de `test_deferred_dispatch_label_string_drift_guard` (mika#1124,
`dispatcher.rs:7380`) et posé à côté de lui :

```rust
/// Garde de dérive mika#2762 : la constante exacte doit rester couverte par le
/// motif `LIKE` que le SQL du reaper utilise pour l'exclure.
#[test]
fn test_deferred_launcher_like_pattern_couvre_le_label() {
    let suffixe = DEFERRED_LAUNCHER_LIKE_PATTERN.trim_start_matches('%');
    assert!(
        DEFERRED_DISPATCH_LABEL.ends_with(suffixe),
        "DEFERRED_DISPATCH_LABEL ne matche plus DEFERRED_LAUNCHER_LIKE_PATTERN — \
         répare le prédicat SQL de mika#2762, n'excepte pas le label"
    );
}
```

Un `#[test]` et non un `const { assert!(…) }` : `str::ends_with` n'est pas une `const fn`,
donc la vérification ne peut pas être au build. Le suffixe est **dérivé** du motif
(`trim_start_matches('%')`) plutôt que retapé, sinon la garde elle-même peut dériver du
prédicat qu'elle protège et rester verte sur une valeur périmée.

### D2 — `find_orphaned_parent_tasks` : ancrage sur `MAX`, lanceurs différés exclus

```sql
SELECT parent.id, parent.agent_id, parent.created_at,
       MIN(child.id)          AS callback_task_id,
       MAX(child.updated_at)  AS last_callback_at
FROM tasks parent
JOIN tasks child ON parent.id = child.parent_task_id
WHERE parent.agent_id = ?1
  AND parent.status = 'in_progress'
  AND parent.source = 'self_dev'
  AND parent.trigger_type = 'manual'
  AND COALESCE(child.dispatch_class, 'implement') = ?3
  AND child.trigger_type = 'callback'
  AND child.action_type  = 'resume_agent'
  AND child.status       = 'delivered'
  AND child.label NOT LIKE ?4            -- mika#2762 : un lanceur différé n'ancre rien
  AND (parent.metadata IS NULL
       OR json_extract(parent.metadata, '$.claude_pilot.pr_url') IS NULL)
  AND NOT EXISTS (
    SELECT 1 FROM tasks sibling
    WHERE sibling.parent_task_id = parent.id
      AND sibling.id != child.id
      AND sibling.status IN ('pending', 'in_progress'))
GROUP BY parent.id
HAVING MAX(child.updated_at) < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
ORDER BY parent.id
```

Trois propriétés que le SQL ne dit pas, à écrire dans le doc-comment :

- **`MAX` porte sur les enfants que la `WHERE` a retenus**, donc sur les callbacks *de la
  classe balayée*, non-differés. Un callback d'une autre classe livré à l'instant ne
  protège pas le parent : la classe est le périmètre du balayage, pas une préférence.
- **`NOT LIKE` est une exclusion des deux côtés, et c'est voulu.** Il retire une ancre trop
  *vieille* (le trou mesuré) **et** une ancre trop *récente* — un lanceur différé fraîchement
  promu ne doit pas masquer un vrai orphelin dont le seul vrai callback est ancien. Le
  prédicat est plus juste dans les deux sens, pas seulement plus permissif.
- **`callback_task_id` reste `MIN(child.id)` et n'est donc PAS l'ancre.** Il est un id
  arbitraire parmi les enfants retenus, conservé tel quel parce qu'il est lu ailleurs ;
  `last_callback_at` est l'ancre. Le dire, sinon le prochain lecteur croira que
  `callback_task_id` désigne le callback dont la grâce est écoulée.

Le nouveau champ va sur `OrphanedParentTask` (`src/task_state/tasks.rs:450`), sur le modèle
de `SettleableDispatchParent.last_child_at` juste en dessous.

### D3 — Le sibling de succès, par contrat de couplage

Le doc-comment de `find_orphaned_parent_tasks` porte un **contrat explicite** :

> **Coupled pair:** […] Any filter change here (agent_id, status, source, trigger_type,
> sibling guard, **grace window**) MUST be applied symmetrically there.

La fenêtre de grâce change : `find_completable_parent_tasks_on_pr_url` (`tasks.rs:2437`)
reçoit donc le **même** `NOT LIKE` et le **même** `HAVING MAX(...)`, à l'identique. La
classe y reste littérale (`'implement'`) — c'est la seule asymétrie déclarée de la paire, et
elle ne bouge pas.

Effet sur cette voie : un parent portant un `pr_url` dont le dernier callback implement
non-differé vient de livrer est complété un tour plus tard. Un **retard**, pas une perte ;
le chemin nominal reste l'inline de `dispatch_resume_agent`, ce backstop ne sert que les
reprises après crash.

### D4 — Le reaper du moteur : l'appel rétabli, la surface refermée

Dans `engine.rs` :

1. **Rétablir** `self.reap_orphaned_parent_tasks("groom").await;` à la ligne 1353, à la
   place du commentaire de rollback, immédiatement après l'appel `"implement"`.
2. **Retirer `#[doc(hidden)] pub`** → `async fn`. La visibilité était la dette explicite du
   rollback (doc-comment lignes 4320–4323) ; une fois les tests eval repassés par `tick()`,
   `tick()` redevient le seul appelant. Le test inline `engine.rs:7495` est dans le même
   crate et n'a pas besoin de `pub`.
3. **Aligner le garde TOCTOU mika#1126 AC-3** (lignes 4393–4419) sur le même prédicat : un
   lanceur différé ne compte pas parmi les `delivered_callback_children` que le garde
   examine. Sinon les deux lectures de « qu'est-ce qu'un callback de dispatch » divergent
   dans la même fonction.
4. **Porter `last_callback_at` sur la ligne `task_engine_reaper.evaluated`** (ligne 4375).
   C'est ce qui rend l'ancrage *lisible* : l'opérateur voit sur quoi la grâce a été ancrée et
   peut constater que ce n'est pas un `:deferred`. Sans ce champ, un futur faux positif de
   la même famille serait à nouveau indistinguable d'un vrai orphelin sur la ligne de log.

### D5 — Les tests

**`src/db/tests/reapers.rs`** — cinq cas, tous sur la classe `groom` sauf le dernier :

| test | scène | attendu |
|---|---|---|
| `mika2762_un_lanceur_differe_nancre_pas_la_grace` | lanceur `:deferred` livré à T−11 min **+** callback groom livré à T | population **vide** |
| `mika2762_un_lanceur_differe_seul_ne_rend_aucun_candidat` | lanceur `:deferred` livré à T−11 min, **aucun** autre enfant | population **vide** (le `NOT LIKE` vide le JOIN) |
| `mika2762_la_grace_sancre_sur_le_dernier_callback` | deux callbacks groom, T−11 min et T−20 min, aucun récent | population = `[parent]`, `last_callback_at` = le plus récent |
| `mika2762_un_lanceur_differe_recent_ne_masque_pas_un_vrai_orphelin` | lanceur `:deferred` livré à T **+** callback groom livré à T−11 min | population = `[parent]` — le détecteur n'est pas émoussé |
| `mika2762_la_voie_de_succes_sancre_pareil` | `find_completable_parent_tasks_on_pr_url`, lanceur `:deferred` vieux + callback implement récent, `pr_url` présent | population **vide** (symétrie D3) |

**`tests/eval/test_groom_parent_reap_2750.rs`** — recâblage, pas de nouveau test :

- `un_groom_dont_le_callback_a_delivre_quitte_in_progress` et
  `un_groom_dont_le_pilote_vit_encore_ne_bouge_pas` repassent par `run_one_scan(&mut engine)`
  à la place de l'appel direct, comme le header du fichier l'annonce.
- `mika2762_un_groom_reussi_pendant_quune_implementation_tient_la_place_garde_son_parent`
  est **inchangé** : il traverse déjà `tick()`. Il devient le test du prédicat le jour où
  l'appel est rétabli — c'est le cœur de la vérification de ce vol.
- Header du module : retirer le paragraphe « Rollback mika#2762 » et dire ce qui tient
  désormais (l'appel est dans le tick, les trois tests y passent).

### D6 — Documentation (hors compte de lignes)

- `docs/solutions/dev-loop/` — une note : *une grâce par ligne n'est pas une grâce sur la
  population*, le lanceur différé comme faux ancre, et le piège `WHERE` vs `HAVING`.
- `crates/mika-agent/CLAUDE.md` n'a pas de section sur ce reaper ; rien à y ajouter. Le
  doc-comment de `find_orphaned_parent_tasks` est le site de vérité et porte déjà la prose
  de mika#2750 — il est amendé, pas doublé.

## Fire-Disposition

Ce plan livre des détecteurs (D5 : cinq tests DB ; D1 : une assertion de dérive de
constante). **Option (a) — exception nommée en allowlist, allowlist vide et épinglée vide.**

Détail d'implémentation : il n'y a **aucune violation préexistante à excepter**, et c'est
vérifiable plutôt que postulé. Les deux cas mesurés du ticket (mika#2752 à 07:37:20Z,
mika#2751 à 07:55:20Z) sont des lignes de production déjà terminales ; les détecteurs
portent sur des scènes semées en mémoire, et le correctif D2/D3 atterrit dans le même
commit qu'eux (§ 5, commit 1), donc ils sont verts à l'atterrissage sans exception d'aucune
sorte. Aucun fichier d'allowlist n'est créé : une allowlist vide dans ce vol serait un
fichier mort.

L'assertion de dérive de D1 est l'unique détecteur qui peut rougir plus tard. Quand elle
rougit, la consigne est écrite dans son message : **réparer le prédicat SQL, jamais excepter
le label** — un lanceur différé qui échappe au `NOT LIKE` rouvre exactement le défaut de ce
ticket.

## 4. Contrat de vérification

1. `cargo test -p mika-agent` — suite entière. Les onze
   `test_find_orphaned_parent_tasks_*` existants doivent rester verts **sans
   modification** : ils ont tous zéro ou un enfant livré par classe, où `MAX(x) = x`, et
   `test_find_orphaned_parent_tasks_mixed_children_groom_and_null` (ligne 700) a ses deux
   enfants dans des classes différentes — la `WHERE` n'en retient qu'un. Si l'un d'eux
   rougit, le prédicat a été écrit plus large que ce plan ne le dit : relire le § 2.
2. `cargo test -p mika-agent --test eval test_groom_parent_reap` — les quatre tests du
   fichier, les trois premiers désormais à travers `tick()`.
3. **Contrôle de bonne foi du câblage.** Avant d'écrire D2/D3, rétablir **seulement**
   l'appel de D4.1 et vérifier que
   `mika2762_un_groom_reussi_pendant_quune_implementation_tient_la_place_garde_son_parent`
   **rougit**. S'il reste vert, l'appel n'est pas branché et le reste du vol est non testé.
   Puis écrire le prédicat et le voir repasser vert.
4. `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
5. `scripts/verify-pipeline.sh origin/main`.
6. **Sonde post-deploy :**
   `sqlite3 ~/.mika/data/mika.db "SELECT target_key, reasoning, created_at FROM audit_events WHERE tool_name='groom_parent_orphaned' ORDER BY created_at DESC LIMIT 10"`
   — toute ligne apparue moins de 600 s après la livraison du callback groom de son parent
   est une régression de ce vol. Le champ `last_callback_at` de
   `task_engine_reaper.evaluated` dit sur quoi la grâce s'était ancrée. L'**absence** de ce
   champ sur la ligne signifie un binaire antérieur à ce vol (classe mika#2340), jamais une
   absence d'ancrage.

## 5. Ordre de commit

1. `fix(db)` — D1 + D2 + D3 + le champ `last_callback_at` + les cinq tests de D5.
   **Plancher : mergeable seul.** Le prédicat est corrigé, le tick ne balaie toujours pas
   `groom` (rollback intact), donc aucune row de production ne change de verdict. Le
   correctif et ses détecteurs arrivent ensemble : des tests seuls seraient rouges, un
   prédicat seul serait non testé.
2. `fix(engine)` — D4 (appel rétabli, visibilité refermée, garde AC-3 aligné, champ sur la
   ligne d'audit) + le recâblage des tests eval. C'est le commit qui **rearme** le
   balayage ; il vient après que le prédicat soit vert, jamais avant.
3. `docs` — D6.

## 6. Risques

| risque | parade |
|---|---|
| `WHERE` et `HAVING` tous deux présents → le défaut est intact et la suite verte | § 2 le nomme comme *le* piège ; le contrôle de bonne foi § 4.3 l'attrape (le test mesuré resterait rouge après D2) |
| Le `NOT LIKE` émousse le reaper : un parent dont le seul enfant est un `:deferred` n'est plus jamais fauché | Exact, et c'est correct — un lanceur différé livré ne dit rien d'un dispatch. Cette row reste au balayage des parents sans enfant (mika#1687) et au balayage fantôme (mika#1712). Épinglé par `mika2762_un_lanceur_differe_seul_ne_rend_aucun_candidat`, qui **documente** l'exclusion au lieu de la subir |
| La symétrie D3 retarde la complétion d'un parent à `pr_url` | Un tour de retard sur un backstop de reprise après crash ; le chemin nominal est l'inline de `dispatch_resume_agent` |
| Le résidu T+600 s (§ 2.1) est pris pour le défaut fermé | § 2.1 le nomme, le borne (verdict comptable faux, aucun travail perdu) et le renvoie au § 7 |
| Le rollback est réarmé sans que le prédicat soit corrigé (commits 1 et 2 inversés) | § 5 ordonne : le plancher est le prédicat, le réarmement vient après |

## 7. Hors périmètre

- **Le résidu T+600 s de § 2.1.** Fermer la fenêtre où aucun frère n'apparaît pendant
  qu'une implémentation tient la place demande un critère d'artefact de groom (« les
  callouts sont écrits ») que `tasks` ne porte pas : un livrable d'une autre nature.
  Recommandé en ticket séparé ; ce vol le laisse en l'état documenté, où il ne détruit rien.
- **`REAPER_GRACE_SECONDS`** (600 s) n'est pas touché. Le défaut est l'ancre, pas la durée ;
  rallonger la grâce masquerait le symptôme sans corriger le prédicat.
- **Les deux autres balayages** (`find_childless_parent_tasks` mika#1687,
  `find_phantom_tracking_tasks` mika#1712) ont leur propre ancrage sur
  `parent.updated_at` et ne joignent aucun enfant livré : ce défaut ne les concerne pas.
- **`find_settleable_dispatch_parents`** (mika#2405) porte déjà la forme correcte ; elle est
  le modèle de ce vol, pas sa cible.

## Definition of Done

- [ ] `find_orphaned_parent_tasks` ancre la grâce sur `MAX(child.updated_at)` en `HAVING`,
      et la clause `WHERE child.updated_at < …` a **disparu**.
- [ ] Les enfants dont le `label` matche `%:deferred` sont exclus du JOIN des deux requêtes
      de la paire couplée, via un prédicat déclaré en un seul site.
- [ ] `DEFERRED_DISPATCH_LABEL` est assertée conforme au motif, avec le message qui dit quoi
      réparer.
- [ ] `OrphanedParentTask.last_callback_at` existe et sort sur
      `task_engine_reaper.evaluated`.
- [ ] `tick()` appelle à nouveau `reap_orphaned_parent_tasks("groom")`, et la méthode n'est
      plus `pub`.
- [ ] Le garde TOCTOU mika#1126 AC-3 ignore les lanceurs différés.
- [ ] Les cinq tests de D5 sont verts ; les trois tests `groom` de
      `test_groom_parent_reap_2750.rs` passent par `tick()`.
- [ ] Le contrôle de bonne foi § 4.3 a été exécuté et son résultat est écrit dans le corps
      de la PR.
- [ ] `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` : verts.
- [ ] Note `docs/solutions/dev-loop/` écrite.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les critères ci-dessous
sont dérivés de sa section **Correctif attendu** et du § 4 de ce plan.

1. **AC1 — La grâce est ancrée sur la dernière livraison.** Un parent dont un callback groom
   non-differé a livré il y a moins de `REAPER_GRACE_SECONDS` n'est jamais rendu par
   `find_orphaned_parent_tasks(…, "groom")`, quel que soit l'âge de ses autres enfants
   livrés.
2. **AC2 — Un lanceur différé n'ancre rien.** Un enfant callback dont le `label` matche
   `%:deferred` ne compte pas parmi les callbacks de dispatch, ni pour la requête, ni pour
   le garde TOCTOU du moteur.
3. **AC3 — Le cas mesuré est épinglé à travers `tick()`.** Un parent avec lanceur différé
   livré à T−11 min, callback groom livré à T, aucun frère actif, reaper à T+1 s : le parent
   reste `in_progress` et aucune ligne `groom_parent_orphaned` n'est écrite. Vérifié par le
   test déjà présent, rendu significatif par le rétablissement de l'appel.
4. **AC4 — Le détecteur n'est pas émoussé.** Un parent dont tous les callbacks groom
   non-differés ont livré avant la grâce, sans frère actif, est toujours fauché, avec le
   motif `groom_callback_delivered_without_redispatch` mot pour mot et son événement d'audit
   `groom_parent_orphaned`.
5. **AC5 — La classe `implement` ne régresse pas.** Les onze tests
   `test_find_orphaned_parent_tasks_*` existants et
   `la_classe_implement_garde_son_motif_mot_pour_mot` restent verts sans modification.
6. **AC6 — Le balayage `groom` est à nouveau câblé.** `tick()` appelle
   `reap_orphaned_parent_tasks("groom")`, et le commentaire de rollback de la ligne 1353 a
   disparu.
7. **AC7 — L'ancrage est lisible en production.** `task_engine_reaper.evaluated` porte
   `last_callback_at`, et la sonde § 4.6 permet de dire pour chaque fauche sur quoi la grâce
   s'était ancrée.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/tasks.rs` — D2 (SQL + doc-comment amendé) | 70 |
| `crates/mika-agent/src/db/tasks.rs` — D3 (sibling de succès symétrisé) | 30 |
| `crates/mika-agent/src/task_state/tasks.rs` — champ `last_callback_at` + doc | 12 |
| `crates/mika-agent/src/agent_loop/mod.rs` — D1 (prédicat) | 10 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — D1 (`#[test]` de dérive) | 20 |
| `crates/mika-agent/src/task_engine/engine.rs` — D4 (appel, visibilité, garde AC-3, champ d'audit) | 40 |
| tests (`crates/mika-agent/src/db/tests/reapers.rs`) | 155 |
| tests (`crates/mika-agent/tests/eval/test_groom_parent_reap_2750.rs`) | 40 |

Total estimé : 377 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 377 sous la borne de 429 et très loin du seuil
de 1000 par PR. Trois raisons concrètes : (a) le test du cas mesuré **existe déjà** et n'est
pas à écrire — il est à rendre significatif, ce qui coûte une ligne d'appel dans `tick()` ;
(b) les deux requêtes de la paire couplée reçoivent le **même** diff de trois clauses, donc
D3 est une copie et non une conception ; (c) la forme SQL cible est déjà écrite dans ce même
fichier (`find_settleable_dispatch_parents`, mika#2405) avec son doc-comment justificatif —
il n'y a pas de forme à inventer. Les 155 lignes de tests DB sont le plus gros poste et
suivent le gabarit des onze tests voisins du même fichier.
