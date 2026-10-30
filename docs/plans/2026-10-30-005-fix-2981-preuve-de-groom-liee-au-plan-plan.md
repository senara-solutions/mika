# La preuve de groom porte le plan qu'elle a groomé — Plan

> Ticket : senara-solutions/mika#2981 (`bug`, `p1-important`, `dispatch:loop`,
> `loop-substrate`, milestone « Substrat de boucle »)
> Liés : mika#2287 (la porte), mika#2590 (le marqueur ancré), mika#2980
> (l'orphelinage), mika#2968 (les trois preuves mesurées).

## Goal Capsule

Une preuve de groom cesse d'être un laissez-passer pour le ticket et devient un
laissez-passer pour **un plan**. La porte de provenance n'accepte un callback de
groom que si la ligne de verdict de ce callback nomme le plan que le callout
`Plan:` du corps courant nomme. Sur un ticket à plusieurs vols, la preuve du vol
1 ne laisse plus partir dev-pilot pour le vol N.

## Product Contract

### Summary

`has_completed_groom_for_issue` joint un callback de groom terminal à une parente
portant l'URL du ticket, et rien de plus. Aucun terme ne lit le plan. Le vol 1
d'un ticket à N vols reste donc une preuve valide pour le vol N — la classe que
mika#2287 existe pour refuser (« markers may be pre-stamped by hand »). Ce plan
lie la preuve au plan, chez le producteur **et** chez le lecteur.

### Problem Frame

Mesuré le 2026-10-30 sur mika#2968 : trois preuves valides simultanément, une par
vol (`e53f9ae7` vol 1, `19aea690` vol 2a, `6b7695ce` vol 2b). Le dispatch de
15:53:14Z a consommé la bonne, `6b7695ce`. Il aurait consommé `e53f9ae7` tout
aussi bien. Pas d'incident à ce jour : ticket de sûreté avant débit.

**Le fait qui commande tout le reste, et qui n'est pas dans le ticket.** Le chemin
du plan n'est **pas** récupérable depuis la base. Il y entre une fois, en
`dispatch-lib.sh:7855` (`Outcome: PLAN_COMMITTED — ${VALID_PLAN}`), et il est
effacé à la convergence : `dispatch-lib.sh:12996` appelle
`_set_outcome_line "Outcome: PLAN_GROOMED"`, et `_set_outcome_line`
(`:3237-3247`) fait `sed '/^Outcome: /d'` avant d'ajouter sa ligne. La preuve et
le nom du plan sont **mutuellement exclusifs** dans `tasks.result`. Les autres
champs ne le rattrapent pas : `reference_url` est `None` sur l'enfant,
`input_context`/`action_config` portent `{prompt, skill, task_id, branch}` — et
sur un premier groom le plan n'existe pas encore quand le prompt est construit —
et `extract_callback_fields` (`dispatcher.rs:5393`) ne remonte que `session_id`,
`turns`, `cost_usd`, `duration_ms`, `pr_url`.

Donc AC1 n'est pas un changement de requête. C'est un changement de **ce que le
producteur écrit**, puis de ce que le lecteur en exige.

### Key Decisions

**D1 — Le terme retenu est le PLAN, pas l'instant du dernier merge.** AC1 offre
les deux et demande d'arbitrer. Quatre raisons, dans l'ordre où elles mordent :

1. Le plan est l'objet du groom. L'instant du merge est un *proxy* pour « un
   nouveau vol a commencé ». Un plan posé à la main **après** le dernier merge
   passe un test purement temporel ; il ne passe pas le test du plan. Le terme
   temporel est nécessaire-mais-pas-suffisant, le terme du plan est suffisant
   seul.
2. Le terme temporel exige un lecteur « issue → PR liée → dernier merge » qui
   **n'existe pas** : `github_graphql.rs` n'a ni ce lecteur ni rien d'approchant
   (`gh_merged_pr_for_branch` est clé **branche**, `reader.rs` lit
   `closedByPullRequestsReferences` sous garde de source mika#2924, scopé
   milestone). C'est un livrable à lui seul.
3. `evaluate_grooming_gate` prend `issue_body` **en paramètre** précisément pour
   tourner hors-ligne, et son doc-comment le revendique. Le terme du plan se lit
   dans cet argument, à coût réseau nul. Le terme temporel y ajouterait une
   dépendance réseau sur un chemin qui **fail-open** déjà quand le token est
   absent (`executor.rs` ~l. 4340, bras `None`) : une seconde porte réseau élargit
   cette ouverture au lieu de la refermer.
4. La porte reste donc testable sans token et sans réseau, ce qui est la
   condition pour qu'AC2 tienne « dans le même appel ».

**Ce que D1 ne ferme pas, et c'est nommé.** Une édition manuelle qui repointe le
callout vivant vers un plan **déjà rétrogradé** en `> Plan antérieur :` par un
merge reste acceptée : la preuve du vol 1 nomme bien ce plan. Le détecteur
hors-ligne de ce résidu (refuser un callout vivant dont le chemin figure aussi en
ligne antérieure) est écrit mais **non livré ici** : il rougit faussement sur un
re-groom légitime du **même** fichier de plan après un merge, et un faux refus sur
un flux réel coûte plus que le résidu qu'il ferme. Un extracteur de chemin depuis
les lignes antérieures n'existe pas encore (`plan_reading` rend `Live` /
`PriorOnly` / `Absent`, jamais un chemin) et devrait vivre dans `plan_callout.rs`
sous la garde mika#2194.

Tracked in: senara-solutions/mika#2981 — ouvrir un ticket de suivi « le callout
vivant ne peut pas nommer un plan déjà rétrogradé en ligne antérieure », avec le
faux-positif du re-groom comme contrainte de conception.

**D2 — Le producteur publie le plan qu'il a validé ; le contrat de
`_set_outcome_line` l'attend déjà.** Son doc-comment (`:3225-3236`) dit « `$1` is
the full line body, e.g. `"Outcome: PR_OPENED — <url>"` », et deux de ses trois
appelants portent déjà un suffixe ` — …`. Le seul qui n'en porte pas est
`PLAN_GROOMED`.

Le chemin fiable à cet instant est celui que `_write_canonical_callout` a déjà
résolu et écrit dans le corps de l'issue (`plan_relpath`, `:10240`) — et **pas**
`VALID_PLAN`, qui est vide sur une re-dispatch (`:7431` porte un
`|| VALID_PLAN=""`). Mais `plan_relpath` est `local` et meurt au `return`. Le
geste est donc : `_write_canonical_callout` publie `plan_relpath` dans une
globale, les deux bras GROOMED de `_iterate_groom_loop` (`:10679`, `:10822`)
l'héritent par construction puisque le writer tourne **avant** leur `return 0`,
et la frappe en `:12996` l'ajoute en suffixe.

**Tous les lecteurs de production survivent au suffixe** — vérifié, pas supposé :
`groom_result_convergence` fait `line.starts_with(GROOM_SUCCESS_MARKER)` sans
ancrage de fin (`task_state/tasks.rs:177`), `carries_groom_outcome_line` teste le
préfixe `"Outcome: "`, le classifieur shell est
`grep -m1 -E '^Outcome: (PR_OPENED|PLAN_COMMITTED|PLAN_GROOMED|ESCALATE)'`
(`:6170`, pas de `$`), et `_set_outcome_line` strippe par préfixe. Aucune égalité
stricte sur `PLAN_GROOMED` en production. Un seul rouge existe, dans les tests :
`test-dispatch-lib.sh:10702` assert `'_set_outcome_line "Outcome: PLAN_GROOMED"'`
guillemet fermante incluse.

**D3 — Une preuve SANS suffixe reste acceptée, sous grâce nommée et comptée.**
Trois populations la produisent légitimement, et refuser sèchement les
refuserait toutes le jour du déploiement :

- **Toutes les preuves déjà en base.** Elles sont toutes nues.
- **La fenêtre de déploiement.** `skills/bundled/` est lu au *build*
  (`build.rs` → `BUNDLED_SKILL_MANIFESTS`), donc le producteur corrigé n'atteint
  aucun agent avant `make deploy` + seed. Entre le merge et le déploiement, les
  grooms de la boucle continuent de frapper des preuves nues.
- **Le flux opérateur `/mika-groom-ticket`**, qui écrit son callout lui-même et
  ne passe pas par `_write_canonical_callout`.

La grâce **se vide d'elle-même** : `prune_completed_tasks` supprime les lignes
terminales au-delà de 30 jours (`db/tasks.rs:4855`, délai posé par
`THIRTY_DAYS_SECS` dans `task_engine/mod.rs:36`), et les grooms de boucle
post-déploiement portent le suffixe. Elle est **comptée** par un événement
d'audit dédié, ce qui est la sonde d'AC4. Le gain net est strict : la porte
devient plus sévère sur toute preuve suffixée et jamais plus laxiste qu'aujourd'hui.

**D4 — Un cinquième état de convergence, mais toujours quatre verdicts de
porte.** `GroomConvergence` gagne `PlanMismatch` ; `groomed_state` la mappe sur
`GroomedState::MarkersWithoutProof`, donc le refus est bien
`dispatch_grooming_not_verified` (ce qu'AC2 exige) et `GroomedState` garde ses
quatre bras — `mika2484_les_quatre_verdicts_de_la_porte_sont_inchanges` reste
verte sans y toucher. C'est exactement le raisonnement déjà écrit pour
`MarkerOutOfPosition` (`executor.rs:1988-1993`) : même disposition, lecture
opérateur différente, l'information neuve est portée par l'événement d'audit et
non par le type. Ordre d'agrégation : `Converged` > `PlanMismatch` >
`MarkerOutOfPosition` > `Absent` — une seule preuve liée suffit, et à défaut la
cause la plus informative gagne.

**D5 — Le vol unique ne change pas, et c'est démontré plutôt qu'affirmé.** Une
preuve, aucun merge : le groom qui a frappé la preuve est celui qui a écrit le
plan **et** le callout, donc son suffixe nomme le plan que le callout nomme →
`Converged`. Si cette preuve précède le déploiement, elle est nue → grâce D3 →
`Converged`. Les deux chemins acceptent. C'est un cas d'AC2 à part entière, pas
un corollaire.

### Requirements

- **R1.** `_write_canonical_callout` publie le chemin repo-relatif qu'il vient de
  valider dans une globale `GROOM_CALLOUT_PLAN`, après la résolution de
  `plan_relpath` et avant son `return 0`. La globale est remise à `""` en tête de
  `dispatch_claude_pilot`, pour qu'une dispatch ne puisse pas hériter du plan de
  la précédente dans le même processus.
- **R2.** La frappe de `dispatch-lib.sh:12996` devient
  `_set_outcome_line "Outcome: PLAN_GROOMED — ${GROOM_CALLOUT_PLAN}"` quand la
  globale est non vide, et reste la forme nue sinon. Le repli nu est la grâce D3
  côté producteur, pas un oubli : il couvre le bras `canonical_callout_failed`,
  où le writer a échoué et n'a donc rien de validé à nommer.
- **R3.** `task_state/tasks.rs` gagne le séparateur en constante nommée, un
  extracteur pur `groom_proof_plan(result) -> Option<String>` qui lit le suffixe
  **sur la ligne ancrée** (jamais ailleurs dans le texte), et la décision de
  liaison. Un seul site, comme le marqueur lui-même : la garde
  `mika2590_le_marqueur_de_convergence_na_quun_lecteur` reste verte — elle porte
  sur l'emploi du **symbole** `GROOM_SUCCESS_MARKER` avec `.contains(` / `instr(`
  / `.find(` hors de ce fichier, et le propriétaire est ce fichier. **Ne rien
  ajouter à `GROOM_MARKER_LOOSE_READERS_ALLOWED`**, que son test frère épingle
  vide.
- **R4.** `has_completed_groom_for_issue` prend `expected_plan: Option<&str>`. Le
  SQL est **inchangé** : la liaison est un filtre Rust sur `child.result`, parce
  qu'un `instr` en SQL est exactement ce que mika#2590 a retiré de cette requête.
  `async_db.rs` passe l'argument.
- **R5.** `groomed_state` extrait le callout vivant par
  `plan_callout::plan_callout(issue_body, FenceHandling::Strip)` — site unique du
  motif, garde mika#2194 — et le passe à la porte. `None` est impossible en
  pratique à ce point (`check_grooming_markers` a déjà exigé `docs/plans/` dans
  le corps et a rendu la main) mais il est traité explicitement : callout
  illisible ⇒ aucune liaison exigible ⇒ comportement d'aujourd'hui, sous
  l'événement de grâce. Pas de `unwrap`.
- **R6.** Deux événements d'audit, écrivains uniques, sur le modèle de
  `note_groom_proof_out_of_position` (`executor.rs:2098`, fire-and-forget,
  `GROOM_PROOF_SESSION_ID`) : `groom_proof_plan_mismatch` (une preuve existait,
  elle nommait un autre plan — le refus neuf) et `groom_proof_unbound_to_plan`
  (acceptée sur la grâce D3 — la population d'AC4). Deux noms, parce que deux
  populations sous un nom ne se soustraient pas (mika#2205).
- **R7.** Le remède du refus gagne une **quatrième strate**, dans la mécanique
  des trois précédentes : elle nomme la cause neuve (« la preuve existe mais
  nomme un autre plan ») et la route **vivante** — ré-appliquer `ready` route au
  groom avec la provenance du refus (mika#2811), donc la passe est réelle et
  frappe une preuve portant le plan courant. Les phrases de mika#2484 et de
  mika#2811 **restent vraies** sous ce changement : c'est précisément ce routage
  qui produit la preuve liée. Aucune route morte prescrite (AC3).

### Scope Boundaries

Dans le périmètre : le producteur shell, l'extracteur pur, la requête, le pont
async, la porte, les deux événements, le remède, et les tests des trois niveaux.

#### Deferred to Follow-Up Work

- Le détecteur « callout vivant nommant un plan déjà rétrogradé » (D1, résidu) —
  `Tracked in:` ci-dessus.
- Le basculement de la grâce D3 en refus, quand la sonde AC4 lit 0 — `Tracked
  in:` ci-dessus, même ticket de suivi.
- Exempter les lignes `PLAN_GROOMED` de `prune_completed_tasks` : suivi
  préexistant, cité par
  `docs/plans/2026-09-22-002-fix-2484-feeder-dispatch-callouts-de-corps-sans-plan.md`.
  Hors périmètre ici — la rétention est ce qui fait **drainer** la grâce D3.
- `fetch_merged_prs_for_issue` dans `github_graphql.rs` : non écrit, par D1.

### Sources

- `crates/mika-agent/src/db/tasks.rs:4388` — `has_completed_groom_for_issue`,
  requête et doctrine mika#2287/#2590.
- `crates/mika-agent/src/task_state/tasks.rs:45,126,162,200` —
  `GROOM_SUCCESS_MARKER`, `GroomConvergence`, `groom_result_convergence`,
  `aggregate_groom_convergence`.
- `crates/mika-agent/src/skills/executor.rs:1856,1976,2033,2098,2555` — verdict,
  `GroomedState`, `groomed_state`, l'écrivain d'audit, la porte.
- `crates/mika-agent/src/skills/executor.rs:1873-1907` — les strates mika#2484 et
  mika#2811 du remède (AC3).
- `crates/mika-agent/src/plan_callout.rs:218,231` — la regex unique et
  `plan_callout`.
- `skills/bundled/_shared/dispatch-lib.sh:3237,10240,10679,10822,12996` —
  `_set_outcome_line`, `plan_relpath`, les deux bras GROOMED, la frappe.
- `crates/mika-agent/src/canonical_tokens.rs:646-745` — la garde du lecteur
  unique et son allowlist vide.
- `crates/mika-agent/src/gh_process.rs:200,261,399` — `FIXTURE_ISSUE_FLOOR`.
- `docs/solutions/logic-errors/grooming-provenance-gate-reads-a-parent-row-a-sibling-mechanism-flips.md`.

## Planning Contract

### Key Technical Decisions

- La requête SQL **ne bouge pas**. Le filtre de liaison est en Rust. Remettre un
  `instr` dans ce `SELECT` est la régression que mika#2590 a fermée.
- Le suffixe est lu **sur la ligne ancrée uniquement**. Un `result.contains(" — ")`
  global attraperait une mention en prose et rendrait la liaison falsifiable par
  le texte du modèle — la classe même du marqueur hors position (mika#2590).
- La comparaison de chemins est sur la chaîne **normalisée** que `plan_callout`
  rend déjà (`PlanCallout.normalized`), pas sur le brut : le callout peut porter
  un préfixe de sous-dépôt là où le producteur écrit un chemin repo-relatif.
- Numéros d'issue de fixture ≥ `FIXTURE_ISSUE_FLOOR` (990 000) dès qu'un test
  traverse `gh_process::run_gh` ; prendre un offset neuf, la suite tourne en
  parallèle dans un seul processus.
- `docs/solutions/` porte la doctrine longue ; les commentaires aux cinq sites
  restent des pointeurs. C'est ce qui tient l'estimation sous la borne.

### Assumptions

- `_write_canonical_callout` tourne sur **les deux** bras GROOMED avant leur
  `return 0` — vérifié (`:10681`, `:10822`), donc la globale est posée dès que la
  convergence a lieu.
- `test-dispatch-lib.sh` est atteint par un workflow (doctrine mika#2578/#2752).
  À confirmer par `scripts/check-suite-wiring.sh` au premier tour
  d'implémentation ; si la suite est `unwired`, l'assertion auto-nettoyante de la
  Fire-Disposition doit migrer vers une garde Rust lisant la source shell, comme
  `canonical_tokens.rs` le fait déjà.

## Implementation Units

### U1. Le producteur publie le plan (`dispatch-lib.sh`)

`GROOM_CALLOUT_PLAN=""` en tête de `dispatch_claude_pilot` ; affectation dans
`_write_canonical_callout` juste après `plan_relpath` ; frappe suffixée en
`:12996` avec repli nu. Mettre à jour `test-dispatch-lib.sh:10702` (l'assertion
porte la guillemet fermante) et le doc-comment périmé de
`dispatcher.rs:4682` (« contains the literal », faux depuis mika#2590) au
passage. Vérifier `scripts/canonical-tokens.tsv:306` : le tier reste
`exact:line-anchored`, un suffixe ne change pas l'ancrage de tête.

### U2. L'extracteur et le cinquième état (`task_state/tasks.rs`)

Constante de séparateur, `groom_proof_plan`, variante `PlanMismatch`, décision de
liaison, bras d'agrégation. Tests unitaires purs : suffixe lu, suffixe en prose
ignoré, absence de suffixe = grâce, plan différent = `PlanMismatch`, ordre
d'agrégation.

### U3. La requête et le pont (`db/tasks.rs`, `async_db.rs`)

`expected_plan: Option<&str>` traversant les deux signatures. SQL inchangé.

### U4. La porte, les deux événements, le remède (`executor.rs`)

Extraction du callout vivant, passage, nouveau bras, les deux écrivains d'audit,
la quatrième strate du remède.

### U5. Les tests de la porte (contrôle négatif d'AC2)

Trois cas, dans `db/tests/harnais_porte.rs` et le `mod harnais_porte` de
`executor.rs`, sur la même fixture :

| cas | preuves | corps | attendu |
|---|---|---|---|
| vol 2 sans preuve | preuve vol 1 portant plan A | callout nomme plan B | refus `dispatch_grooming_not_verified` |
| vol 2 avec preuve | preuves vol 1 (plan A) + vol 2 (plan B) | callout nomme plan B | accepte |
| vol unique | une preuve portant plan A | callout nomme plan A | accepte |
| grâce D3 | une preuve **nue** | callout nomme plan A | accepte + `groom_proof_unbound_to_plan` |

`GROOMED_ISSUE_BODY` (`executor.rs`, `mod harnais_porte`) porte déjà un callout
`Plan:` bien formé : le cas « vol unique » s'y appuie sans fixture neuve. Semer
par `completed_groom_pair` / `VolReel::seme_la_forme_reelle` (`db/tests/mod.rs`),
jamais par `INSERT` direct — la preuve doit passer par le chemin d'écriture de
production.

## Fire-Disposition

Le livrable est une porte : son chemin de succès est « aucune violation
trouvée ». Option **(a) — exception nommée en allowlist**, instanciée sur une
population de **données** et non de sites de code.

- **L'exception, grep-visible.** Une constante
  `GROOM_PROOF_UNBOUND_GRACE` dans `task_state/tasks.rs`, dont le doc-comment
  nomme la donnée précise — une preuve dont la ligne ancrée ne porte **aucun**
  suffixe de plan — et ses trois producteurs légitimes (preuves antérieures au
  déploiement ; fenêtre entre merge et `make deploy`, puisque `skills/bundled/`
  est lu au build ; flux opérateur `/mika-groom-ticket`, qui n'appelle pas
  `_write_canonical_callout`). Elle référence le ticket de suivi de la § *Deferred
  to Follow-Up Work* comme site de son retrait.
- **L'assertion auto-nettoyante.** Dans `test-dispatch-lib.sh` : la forme **nue**
  `_set_outcome_line "Outcome: PLAN_GROOMED"` (guillemet fermante immédiate) est
  **absente** de `dispatch-lib.sh`, et la forme suffixée est présente une fois.
  Elle rougit le jour où quelqu'un ramène le producteur à une frappe nue — le
  seul chemin par lequel la grâce peut devenir la voie nominale au lieu de se
  vider. C'est le contrôle positif de la § *Assumptions* qui dit où elle doit
  vivre si la suite shell n'est pas câblée.
- **Le compteur qui la fait expirer.** `groom_proof_unbound_to_plan` dans
  `audit_events`, écrivain unique, donc
  `SELECT count(*) … WHERE tool_name = 'groom_proof_unbound_to_plan'` est exact
  plutôt qu'un nombre sur lequel deux écrivains divergent. C'est la sonde d'AC4 ;
  à 0, le ticket de suivi retire la grâce et l'assertion avec elle.

Aucun détecteur n'atterrit désarmé : AC2 exige un refus effectif, donc la porte
est armée au merge.

## Verification Contract

- `cargo test -p mika-agent harnais_porte` — le critère de sortie déjà documenté
  pour ce harnais.
- `cargo test -p mika-agent task_state::tasks` et
  `cargo test -p mika-agent db::tests::task_messages_and_groom`.
- `cargo test -p mika-agent canonical_tokens` — les gardes mika#2590 et mika#2484
  restent vertes **sans** entrée d'allowlist neuve.
- `cargo test -p mika-agent` complet, `cargo clippy`, `cargo fmt --check`.
- `skills/bundled/_shared/test-dispatch-lib.sh` en entier.
- `make verify-bundled-skills` (mika#1575).
- `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- La porte refuse un dispatch dev-pilot dont la seule preuve nomme un autre plan
  que le callout courant, sous `dispatch_grooming_not_verified`.
- Le producteur frappe `Outcome: PLAN_GROOMED — <plan repo-relatif>` sur les deux
  bras de convergence, et nu sur le seul bras où il n'a rien de validé à nommer.
- Le cas d'un vol unique accepte, par les deux chemins (preuve suffixée, preuve
  nue sous grâce), et c'est épinglé par deux tests distincts.
- Les deux populations — refus par plan discordant, acceptation par grâce — sont
  comptables séparément dans `audit_events`.
- Le remède porte sa quatrième strate et ne prescrit aucune route morte.
- Aucune allowlist de garde n'a gagné d'entrée.

## Acceptance criteria

- **AC1.** La porte n'accepte une preuve que si elle porte le plan nommé par le
  callout `Plan:` du corps courant, ou si elle est postérieure au dernier merge
  d'une PR liée au ticket. Le plan tranche lequel des deux, ou les deux, et dit
  pourquoi. Il dit aussi ce que rend la porte pour un ticket sans merge
  antérieur, c'est-à-dire le cas d'un vol unique, qui ne doit pas changer.
  → Tranché en **D1** (le plan, pas l'instant du merge ; quatre raisons et le
  résidu nommé) et **D5** (le vol unique, inchangé par les deux chemins).
- **AC2.** Test dans le même appel, avec contrôle négatif. Un ticket porte une
  preuve de groom du vol 1 (plan A), un merge, puis un callout de corps qui nomme
  un plan B sans preuve de groom : la porte refuse
  (`dispatch_grooming_not_verified`). Même fixture, avec une preuve du vol 2
  portant le plan B : la porte accepte. Le cas d'un vol unique (une preuve, aucun
  merge) accepte toujours.
  → **U5**, table des quatre cas.
- **AC3.** Le texte de remède du refus reste vrai, sans prescrire de route morte
  (strates mika#2484 et mika#2811 du payload, `executor.rs` l. 1873-1889).
  → **R7**.
- **AC4.** Sonde après déploiement : sur les 10 dispatches de dev-pilot suivants
  pour un ticket à plusieurs vols, le compte de ceux dont la seule preuve
  acceptée est antérieure au dernier merge du ticket est écrit ici. La cible est
  0.
  → Instrument : `groom_proof_unbound_to_plan` dans `audit_events`, écrivain
  unique (**R6**, § *Fire-Disposition*). À relever après `make deploy` — avant le
  déploiement l'absence de la ligne signifie « binaire antérieur au vol »
  (mika#2340), jamais « compte nul ».

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` | 30 |
| `crates/mika-agent/src/task_state/tasks.rs` | 85 |
| `crates/mika-agent/src/db/tasks.rs` | 30 |
| `crates/mika-agent/src/async_db.rs` | 10 |
| `crates/mika-agent/src/skills/executor.rs` | 90 |
| tests Rust (`db/tests/`, `mod harnais_porte`, `task_state/tasks.rs`) | 125 |
| tests shell (`test-dispatch-lib.sh`) | 32 |

Total estimé : 402 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Un seul vol, et il tient pour une raison précise : c'est **un** chemin de donnée
— le chemin du plan — enfilé à travers cinq sites existants, sans module neuf,
sans migration, sans DDL, et sans changement de requête SQL. Le découper
laisserait un vol 1 inerte (le producteur écrit un suffixe que personne ne lit)
ou dangereux (le lecteur exige un suffixe que personne n'écrit, donc tout refuse
sauf la grâce). La marge de 27 lignes est tenue par la décision de la § *Key
Technical Decisions* : la doctrine longue va dans `docs/solutions/`, hors du
compte, et les commentaires aux cinq sites restent des pointeurs.
