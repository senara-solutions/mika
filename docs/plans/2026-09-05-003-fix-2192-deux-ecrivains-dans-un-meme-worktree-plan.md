---
issue: 2192
repo: senara-solutions/mika
type: fix
module: mika-agent/skills/executor, mika-agent/db, skills/bundled/_shared/dispatch-lib
tags: [dispatch, worktree, concurrency, attribution, loop-substrate, mika-2192]
problem_type: unclaimed-shared-resource
status: groomed
---

# mika#2192 — deux écrivains dans un même worktree

**Issue :** senara-solutions/mika#2192
**Branche :** `fix/2192/dispatch-lib-worktree-deux-crivains-dans`
**Palier :** Tier 2 — ralentit la boucle, versant tier 1 assumé (perte de travail non commité).

---

## 1. Le fait, relu dans le code

Le corps du ticket décrit trois conséquences. Les trois se lisent dans le code, et le
détail que la lecture ajoute est **quel geste** a détruit le travail.

### 1.1 Le geste destructeur n'est pas un `reset` nu

Le reflog dit `reset: moving to HEAD`. C'est la signature de `git stash push`, et
`dispatch-lib` en appelle **deux** dans le chemin d'entrée d'un worktree :

| site | ligne | ce qu'il fait au travail d'un tiers |
|---|---|---|
| pré-vol « relique non canonique » | `dispatch-lib.sh:1939` | `stash push --include-untracked` puis `worktree remove --force` |
| nettoyage avant rebase | `dispatch-lib.sh:1492`, appelé en `:2023` | `stash push --include-untracked` sur le worktree réutilisé |

Les deux **préservent** dans le stash — ce n'est pas une destruction, c'est un
déplacement silencieux. Mais du point de vue de l'écrivain vivant, un fichier qu'il
vient d'écrire disparaît entre deux appels d'outil, sans message, dans une pile de
stash partagée par tous les worktrees du dépôt. C'est ce que l'orchestrateur a
d'abord diagnostiqué comme « écriture non persistée ».

**Conséquence pour la conception : le refus doit précéder `:1921`.** Un garde placé
après le pré-vol arrive après le premier `stash push`.

### 1.2 Rien ne réclame le répertoire

`_set_up_worktree` (`dispatch-lib.sh:1790`) dérive `WORKTREE_DIR` en `:1919` par
`scripts/derive-worktree-path`, puis entre. Entre la dérivation et l'entrée, aucune
lecture d'état : le chemin est une **fonction** de la branche, jamais une allocation.
Deux acteurs sur le même ticket obtiennent le même chemin par construction.

`dispatch_slot_leases` (`db.rs:8608`, clé `(agent_id, dispatch_class, slot_index)`
depuis la migration v52, `db.rs:4894`) arbitre le créneau d'exécution et le fait
correctement. Il n'a jamais prétendu arbitrer un répertoire, et une session
orchestrateur en mode 2 n'y apparaît pas : elle n'est pas un dispatch.

### 1.3 L'attribution est écrite sans preuve

`_rescue_dirty_worktree` (`dispatch-lib.sh:2692`) compose son message de commit en
`:2813` et `:2858` :

```
Content written by pilot session ${SESSION_ID:-unknown} but git commit was never invoked.
```

`SESSION_ID` est la session **du dispatch en cours**. Rien n'est lu du contenu ni du
répertoire : la phrase affirme un auteur au lieu de le constater. C'est la même
faute de forme que la doctrine `origin:*` interdit déjà — *« an unmarked PR reads
"unknown", never "by hand" »*.

---

## 2. Où vit le refus qui *re-diffère*

AC2 demande deux choses qui ne vivent pas au même étage :

1. « `dispatch-lib` refuse d'entrer dans un worktree… » — un site.
2. « Le refus doit être une **classe de refus existante** (le dispatch se re-diffère) » —
   un mécanisme.

Le mécanisme n'existe pas dans `dispatch-lib`. Son vocabulaire de sortie est terminal :
`auto_skipped` (`:1858`, `:1898`), `PIPELINE FAILURE`, `STATUS=REBASE_CONFLICT` (`:2045`).
Aucune de ces sorties ne remet la tâche en file.

La re-diffusion vit **en amont du handler**, côté Rust :

```
validate_dispatch_readiness (executor.rs:1101)
  └─ refus "global_dispatch_active" (executor.rs:1490)
       └─ register_deferred_callback (executor.rs:2415)
            └─ parent → blocked + wrapper différé
                 └─ reap_stale_blocked_dispatch_tasks (engine.rs:1088) ré-arme
```

`verdict_handler.rs:781` montre le même branchement du côté verdict.

**Décision : les deux étages, et c'est le contrat à deux couches que le dépôt pratique
déjà** (garde méta-dépôt + garde sous-dépôt du `make deploy`). La couche Rust porte le
refus qui gouverne la file ; `dispatch-lib` porte le filet qui couvre le contournement
de la frontière d'outil (handler invoqué à la main, chemin de reprise). AC2 est
satisfait dans sa lettre — `dispatch-lib` refuse bien d'entrer — et dans son mécanisme —
la classe qui re-diffère est celle qui existe.

---

## 3. La forme retenue : un registre de réclamations, jumeau de `dispatch_slot_leases`

### 3.1 Clé

`(repo, issue_number)`, pas le chemin du worktree.

C'est équivalent et strictement moins cher. L'invariant
`worktree_path_slug == sanitize(branch_ref)` et
`branch_ref == derive-branch-name(titre, issue, labels)` font du couple
`(repo, issue)` une clé **suffisante** : deux acteurs sur le même ticket collident,
deux acteurs sur des tickets distincts jamais. Et surtout, `(repo, issue)` est ce que
la frontière d'outil possède déjà — `tool_input.prompt` vaut `mika#2192`, lu par
`parse_repo_ref_from_dispatch_prompt`. Clé par chemin ⇒ il faudrait dériver la branche
en Rust (appel `gh` + `derive-branch-name`) pour poser le garde : une duplication de la
dérivation, exactement la dérive que mika-platform#58 a fermée.

### 3.2 Table

`worktree_claims`, migration **v52 → v53** (`CURRENT_SCHEMA_VERSION`, `db.rs:30`) :

| colonne | rôle |
|---|---|
| `repo`, `issue_number` | clé primaire composite |
| `owner_kind` | `pilot` \| `orchestrator` \| `spawn` |
| `owner_id` | `task_id` du dispatch, ou `mika-orchestrator-id`, ou `MIKA_SPAWN_ID` |
| `owner_label` | texte lisible pour l'attribution AC3 |
| `claimed_at`, `expires_at` | horodatages ISO-8601 `%Y-%m-%dT%H:%M:%SZ`, comme le bail |

Vivacité = `expires_at > now`. **Pas de `pid`, pas de `pgrep`, pas de `/proc`** : le
harnais rend `pgrep -f` vacuant (mémoire `feedback_pgrep_f_is_vacuous_in_this_harness`)
et l'agent peut tourner conteneurisé, donc `/proc` de l'hôte n'est pas une surface sur
laquelle bâtir. On reprend exactement la propriété que `try_acquire_dispatch_slot`
documente en `db.rs:8600-8607` : *« un réclamant qui meurt en cours bloque sa classe au
plus un TTL, pas pour toujours »*.

TTL : `pilot` = durée estimée du dispatch + marge (même politique que le bail) ;
`orchestrator` / `spawn` = 2 h, renouvelable par re-réclamation idempotente.

### 3.3 API

`db.rs`, en miroir de `try_acquire_dispatch_slot` :

- `try_claim_worktree(repo, issue, owner_kind, owner_id, owner_label, ttl) -> WorktreeClaim`
  — transaction `Immediate` ; re-réclamation par le même `owner_id` = rafraîchissement
  idempotent ; réclamation par un autre propriétaire vivant = refus nommant le tenant.
- `worktree_claim_holder(repo, issue) -> Option<WorktreeClaim>` — filtre `expires_at > now`,
  donc `None` **est** la réponse « expirée ou absente ».
- `release_worktree_claim(repo, issue, owner_id)` — ne supprime que sa propre ligne.
- balayage des lignes expirées dans le même passage que celui des baux (`db.rs:8742`).

### 3.4 CLI

`mika worktree claim|release|show <repo>#<issue>` (crate `mika-cli`), pour que
l'orchestrateur et les spawns aient le même geste que le pilote. Combler la façade CLI
plutôt que laisser l'orchestrateur écrire en SQL : `feedback_dont_shrug_off_cli_gaps`.

---

## 4. Alternatives écartées, et pourquoi

| écartée | raison |
|---|---|
| **fichier marqueur dans le worktree** (`.claude/worktree-owner.json`) | salit `git status`, donc déclenche le sauvetage `_rescue_dirty_worktree` qu'il est censé corriger ; l'exclusion via `info/exclude` du répertoire git **commun** est partagée par tous les worktrees (`dispatch-lib.sh:1545-1552`) ; et la frontière d'outil Rust devrait dériver la branche pour trouver le fichier. |
| **`git worktree lock --reason`** | porteur git-natif séduisant, mais illisible depuis la frontière d'outil sans dériver la branche, et il change le comportement des `worktree remove --force` existants (`:1955`, `:1971`) — un effet de bord non demandé sur un chemin de nettoyage qui marche. |
| **`pid` + `boot_id` pour la vivacité** | plus précis, mais adossé à `/proc` de l'hôte depuis un agent potentiellement conteneurisé, et sans précédent dans le dépôt. Le TTL a le sien. |
| **clé = chemin du worktree** | duplique la dérivation de branche côté Rust (§3.1). |
| **refus uniquement dans `dispatch-lib`** | aucune classe de refus qui re-diffère n'y existe (§2) ; le seul relais serait le LLM mika-dev lisant un `RESULT` — application par prompt au niveau du substrat, empiriquement défaillante (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`). |

---

## 5. Livrables

### Phase A — le registre (mika)

**A1.** `db.rs` : migration `migrate_v52_to_v53`, table `worktree_claims`,
`CURRENT_SCHEMA_VERSION = 53`. Table créée aussi dans le schéma frais (`db.rs:1388`
voisine).
**A2.** `db.rs` / `async_db.rs` : les quatre fonctions de §3.3.
**A3.** `mika-cli` : sous-commande `worktree claim|release|show`.

*Tie-back : AC1 (le marqueur existe et porte propriétaire + horodatage).*

### Phase B — le refus qui re-diffère (mika, Rust)

**B1.** `validate_dispatch_readiness` (`executor.rs:1101`) : après la porte
`repo_not_dispatchable` et avant la garde de classe, lire
`worktree_claim_holder(repo, issue)`. Si un tenant **vivant** et **différent** du
`task_id` courant existe → refus `worktree_claimed_by_other`, portant `owner_kind`,
`owner_id`, `owner_label`, `claimed_at`, `expires_at`, et un `reason` qui dit que le
dispatch sera repris.
**B2.** Brancher ce refus sur `register_deferred_callback` (`executor.rs:2415`) —
même branchement que `global_dispatch_active` en `executor.rs:1490` et
`verdict_handler.rs:781`. Événement d'audit `worktree_claim_refused` via
`log_audit_event`, à côté de `deferred_dispatch_registered`.
**B3.** `reap_stale_blocked_dispatch_tasks` (`engine.rs:1088`) : ajouter une
vérification (2 bis) sur `worktree_claim_holder`, en miroir exact de la vérification du
bail en `engine.rs:1144`. Sans elle, le réveil ré-arme, se fait refuser, ré-arme —
tournoiement borné mais bruyant.

*Tie-back : AC2 (refus nommé, classe existante, événement d'audit).*

### Phase C — `dispatch-lib` : réclamer, refuser, attribuer (mika, shell)

**C1.** `_set_up_worktree` : juste après `:1919` (dérivation de `WORKTREE_DIR`) et
**avant** le pré-vol `:1921`, appeler `mika worktree show`. Tenant vivant et différent
→ `echo "worktree_claim_refused: repo=… issue=… holder_kind=… holder_id=… expires_at=…" >&2`
puis sortie sans toucher au répertoire. Aucun `stash push`, aucun `worktree remove`,
aucun rebase ne s'exécute.
**C2.** Réclamer immédiatement après le garde : `mika worktree claim` avec
`owner_kind=pilot`, `owner_id=$TASK_ID`, `owner_label` portant `$SKILL` et `$SESSION_ID`.
**C3.** Libérer dans le piège `EXIT` existant (`dispatch-lib.sh:1216` voisin) :
`mika worktree release`. Le TTL reste le filet.
**C4.** `_rescue_dirty_worktree` (`:2692`) : lire le tenant avant de composer le message
(`:2813`, `:2858`). Trois branches, jamais un nom faux :
  - tenant = cette session → texte actuel, inchangé ;
  - tenant ≠ cette session → `Content written by <owner_kind> <owner_label> (claim held since <claimed_at>); this dispatch (session <SESSION_ID>) only staged it.` ;
  - aucun tenant → `Content owner unknown — no live worktree claim covered this directory when the rescue ran.`

*Tie-back : AC1 (écrit à l'entrée, retiré à la sortie), AC2 (`dispatch-lib` refuse), AC3.*

### Phase D — les autres écrivains (companion mika-platform)

Réclamer / libérer autour des gestes qui créent ou réutilisent un worktree hors boucle :
`.claude/commands/mika.md` (méta), `.claude/commands/mika-groom-ticket.md` (phase 2 §5 / §5a),
`scripts/mika-platform-spawn`, `scripts/mika-platform-worktree-cleanup` (libération).

PR compagnon `senara-solutions/mika-platform#<N>`, référencée croisée dans les deux corps
de PR selon la convention méta-dépôt.

*Tie-back : AC1 pour le propriétaire « orchestrateur » et « spawn » — c'est-à-dire
précisément l'acteur de l'incident fondateur.*

---

## 6. Anti-vacuité (AC4)

Deux rejeux, un par étage. Chacun porte son **contrôle négatif dans le même appel**
(`feedback_a_probe_needs_both_controls_in_the_same_call`).

**D1 — Rust, `executor.rs` (suite de tests existante).** Poser une réclamation vivante
sur `("mika", 2179)` par `owner_id = "orchestrator-X"`, appeler
`validate_dispatch_readiness` avec `prompt = "mika#2179"` et un `task_id` différent.
Attendu : `Err`, `error == "worktree_claimed_by_other"`, wrapper différé enregistré.
*Contrôle négatif dans le même test :* la même réclamation **expirée** (`expires_at`
dans le passé) laisse passer, et une réclamation vivante sur `("mika", 2180)` ne bloque
pas `mika#2179`.

**D2 — shell, `test-dispatch-lib.sh`.** Sonde au niveau processus, dans la forme déjà
employée par le bloc mika#1772 du fichier : un faux `mika` sur `PATH` rend un tenant
vivant, `_set_up_worktree` est appelé sur un worktree jetable portant un fichier
non commité. Attendu : sortie non nulle, ligne `worktree_claim_refused:` sur stderr,
**et le fichier non commité toujours présent, `git stash list` vide** — c'est cette
dernière assertion qui mesure la perte évitée plutôt que le message.
*Contrôle négatif :* tenant expiré ⇒ le dispatch entre normalement.

**Rouge sur `main`, terme par terme.** Les deux sondes sont lancées sur `main` avant la
correction ; la sortie rouge est collée dans le corps de la PR. Les contrôles négatifs
sont lancés **séparément** des contrôles positifs, jamais dans une assertion disjonctive
(`feedback_red_before_control_is_term_by_term`).

**D3 — anti-régression d'attribution.** Test dédié sur `_rescue_dirty_worktree` : les
trois branches de C4, et une assertion `assert_not_contains` que le message ne nomme
jamais `$SESSION_ID` comme auteur quand le tenant est un tiers ou absent.

---

## 7. Portée

**Dans la portée.** AC1–AC4 ci-dessus, sur `mika` (phases A–C) et `mika-platform`
(phase D).

**Hors portée, conformément au corps du ticket.**
- Un worktree distinct par acteur — casse `slug == sanitize(branch)`, autre conception.
- Le mécanisme d'estampille `origin:*` lui-même. `_stamp_pr_origin` (`dispatch-lib.sh:1289`,
  `:3216`) n'est pas touché : il a menti parce que la provenance était mal lue, et
  AC2 + AC3 ferment la lecture fausse à la source. Un incident de la même forme ne peut
  plus produire de PR étiquetée `origin:loop` sur du travail manuel, puisque le dispatch
  ne sera pas entré.
- Lever la sérialisation à N > 1 pilotes (mika#2160) — indépendant.

---

## 8. Résidus nommés

**R1 — un `git worktree add` à la main ne réclame rien.** Après la phase D, les gestes
*sanctionnés* réclament ; un `git worktree add` tapé directement reste invisible au
garde. Le fermer demanderait un unique geste de création (`scripts/mika-worktree create`)
que tous les appelants traversent — un remaniement distinct, à ficher en suivi si un
second incident se présente hors des chemins sanctionnés. **Ce n'est pas une hypothèse
neutre** : l'incident fondateur est passé par `/mika` (chemin sanctionné), donc la
phase D le couvre.

**R2 — l'orchestrateur ne bat pas le cœur.** Une session interactive qui meurt sans
libérer laisse une réclamation vivante jusqu'au TTL (2 h). Pendant ce temps le dispatch
se re-diffère au lieu d'entrer : le mauvais côté est le côté sûr, et c'est le même
compromis que le bail de créneau assume depuis mika#1948.

**R3 — le stash partagé reste partagé.** Ce plan empêche `dispatch-lib` de stasher le
travail d'un tiers ; il ne rend pas la pile de stash per-worktree. Hors portée.

---

## 9. Risques

| risque | mitigation |
|---|---|
| Une réclamation orpheline fige un ticket | TTL + `worktree claim show` + `release` en CLI ; précédent du bail. |
| Le garde B1 refuse une reprise légitime du **même** dispatch | La re-réclamation par le même `owner_id` est idempotente (§3.3) ; testée en D1. |
| Le tournoiement ré-arme / refuse | Fermé par B3 (miroir de `engine.rs:1144`) ; `rearm_count` borne le reste. |
| Migration v53 sur une base vivante | Suivre la forme de `migrate_v51_to_v52` (`db.rs:4908`) : table créée, aucune donnée réécrite, aucun `DROP` sur une table existante. |
| Phase D pousse deux PR qui doivent atterrir ensemble | mika d'abord (le CLI doit exister avant que mika-platform l'appelle) ; la phase D dégrade proprement si `mika worktree` est absent (`|| true`). |

---

## 10. Critères d'acceptation — traçabilité

| AC | livrables | preuve |
|---|---|---|
| AC1 — worktree occupé déclaré | A1, A2, A3, C2, C3, D | `mika worktree show mika#2192` rend propriétaire + horodatage |
| AC2 — refus nommé, classe existante, audit | B1, B2, B3, C1 | D1 (Rust), D2 (shell), événement `worktree_claim_refused` |
| AC3 — attribution sans nom faux | C4 | D3, trois branches |
| AC4 — rejeu anti-vacuité rouge sur `main` | D1, D2 | sortie rouge collée dans la PR |
