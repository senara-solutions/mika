# Un hold explicite tient contre `wip_rescue` (mika#2597)

> **Ticket :** senara-solutions/mika#2597 (p1, `loop-substrate`)
> **Classe :** un geste d'opérateur qu'un mécanisme de reprise annule en silence.
> **Précédent direct :** mika#2315 (`ready_label.rs`) — même classe, même forme de
> remède (prédicat de timeline, sortie auto-libérante, fail-closed sur illisible).

---

## Constat

`wip_rescue` a repris une PR maintenue en **brouillon de hold**, l'a rebasée, puis
l'a **sortie du brouillon** — deux fois en 35 minutes sur mika#2589, la seconde
après une remise en draft explicite de l'orchestrateur. Le merge autonome n'a été
empêché que parce que le diff touchait des fichiers decision-core : sur un autre
jeu de fichiers la PR était CLEAN et APPROVED, et le moteur l'aurait mergée en
contournant une décision de renoncement consignée dans le plan.

`wip_rescue` n'a aujourd'hui **aucun moyen** de distinguer :

| | forme | produit par | ce que le démon doit en faire |
|---|---|---|---|
| **draft de rescue** | état de **départ** | `dispatch-lib` (`gh pr create --draft`) | le promouvoir — c'est sa raison d'être (mika#1852) |
| **draft de hold** | état **remis après** le travail | un humain ou l'orchestrateur | ne pas y toucher |

---

## Trois rectifications que la lecture du code impose au ticket

C'est le premier livrable : deux des trois pistes du ticket sont réfutées par le
code, et la troisième est déjà fermée sur la population mesurée.

### R1 — rien n'a « repris une tâche morte » ; l'`updated_at` est une *conséquence*

La ligne `21:01:05 · la tâche morte 63348699 … est reprise (updated_at)` se lit
comme un déclencheur. Ce n'en est pas un. `wip_rescue` ne lit une tâche qu'à
`resolve_parent_depth` (PR → `closingIssuesReferences` → issue → `find_active_task_by_ref_url`),
et la **seule** écriture qu'il y fait est `update_task_metadata` à la toute fin de
`resume_chain` — l'incrément de `wip_rescue.depth`, après l'un-draft.

> **Conséquence pour le correctif :** il n'y a pas de « reprise de tâche » à
> bloquer. En excluant la PR **à la sélection**, l'`updated_at` ne bouge pas non
> plus, puisque le bump vit après l'un-draft dans la même chaîne. AC2 (« ni reprise
> ni promue ») est donc satisfaite par un seul terme, et non par deux.

### R2 — « `ConvertToDraftEvent` postérieur au dernier push du pilote » est un prédicat qui s'annule lui-même

La piste (b) du ticket compare l'instant du hold au dernier push. Deux mesures la
refusent :

1. **`wip_rescue` pousse lui-même** — `prepare_branch` fait rebase + push
   `--force-with-lease` à l'étape 4, **avant** l'un-draft de l'étape 7, dans la même
   chaîne. Un prédicat « hold postérieur au dernier push » serait donc voidé par le
   push du démon qui est en train de violer le hold. *Auto-annulation.*
2. **Un push ultérieur ne lève pas une décision d'opérateur.** Un hold de
   renoncement tient jusqu'à ce que l'opérateur le lève, pas jusqu'au prochain
   commit. La comparaison ouvrirait un trou dont personne n'aurait connaissance.

> **Le prédicat retenu ne compare aucun instant** — voir § *Le prédicat* : la
> **présence** d'un `ConvertToDraftEvent` suffit, et le `--draft` du listing est ce
> qui la rend suffisante.

### R3 — le second chemin d'un-draft est déjà fermé sur la population mesurée

AC1 demande « tout chemin qui sort une PR du brouillon ». Recensement exhaustif
(`grep` sur `crates/`, `skills/`, `scripts/`) :

| # | chemin | statut |
|---|---|---|
| 1 | `wip_rescue::resume_chain` étape 7 (`gh pr ready`, Rust) | **le défaut mesuré — fermé par ce plan** |
| 2 | `run_gh` → `gh pr ready <N>` par un LLM | **déjà refusé** par `validate_pr_ready_undraft_scope` (mika#1682) dès que la PR porte le label `wip-rescue` ou un commit de tête `wip(` |
| 3 | les quatre prompts `self-dev*` | interdisent déjà le geste, en renfort du refus 2 |
| 4 | `gh pr ready --undo` (sens inverse) | **aucun appelant**, ni code ni prompt — voir § *Le prédicat* |

mika#2589 porte `wip-rescue`, donc le chemin 2 était **déjà** couvert au moment de
l'incident : ce n'est pas par là que la PR est sortie du brouillon. Élargir
mika#1682 à « toute PR portant un `ConvertToDraftEvent » est possible, coûte un
appel timeline sur **chaque** `gh pr ready` du modèle, et porte sur une population
dont **aucune violation n'est mesurée**. Armer un détecteur sur une population vide
est ce que mika#2520 refuse. → **suivi nommé**, précondition écrite.

---

## Le prédicat

```text
held(pr) :=
  pr ∈ listing `gh pr list --draft --state open --label wip-rescue`   (déjà le cas)
  ∧ ∃ au moins un ConvertToDraftEvent dans la timeline de pr
```

**Pourquoi la simple présence suffit, et c'est le cœur du raisonnement.** Une PR
de rescue est **créée** en brouillon (`gh pr create --draft`), donc sa timeline ne
porte **aucun** `ConvertToDraftEvent` à la naissance. Les deux seules transitions
d'état draft sont `ConvertToDraftEvent` et `ReadyForReviewEvent`. Comme le listing
filtre déjà `--draft`, une PR qui porte ≥ 1 `ConvertToDraftEvent` **et** qui est
brouillon *maintenant* a forcément eu son dernier basculement vers le brouillon :
c'est un hold. Aucune date à comparer, aucun ordre à établir, aucune pagination.

> **Fragilité nommée :** retirer `--draft` du listing casserait cette équivalence.
> Le sens de la casse est l'inertie (une PR ready porterait un hold fantôme et
> serait exclue d'une population dont elle ne fait de toute façon pas partie), pas
> la violation. Écrit au site, et épinglé par un test.

**Pas de filtre d'acteur machine, et c'est une décision.** mika#2315 en porte un
(`identités_machine`) parce que son propre `remove→add` se parquerait lui-même.
Ici, **aucun** chemin n'écrit de `ConvertToDraftEvent` (ligne 4 du tableau R3) :
le terme aurait une population vide, et si une machine s'y mettait un jour, lire
son geste comme un hold est le sens **sûr**. L'acteur et l'instant sont donc
**rapportés sur la ligne d'observabilité** sans décider de rien — ce qui rend
visible le jour où une machine y apparaît, sans armer un prédicat sur du vide.

**Sortie du hold : aucun geste à apprendre, aucun état à nettoyer.** L'opérateur
sort la PR du brouillon ; elle quitte le listing `--draft` et donc la population
entière. Le hold n'existe que tant que la PR *est* un brouillon — il se lève de
lui-même, exactement comme le park de mika#2315 (« geste unique et symétrique »).
C'est ce qui distingue ce remède d'un marqueur durable : il n'y a rien à effacer.

### Lecture : GraphQL, un appel, et c'est la leçon B1 qui l'impose

```graphql
query($owner:String!,$repo:String!,$number:Int!) {
  repository(owner:$owner,name:$repo) {
    pullRequest(number:$number) {
      timelineItems(itemTypes:[CONVERT_TO_DRAFT_EVENT], last:1) {
        nodes { ... on ConvertToDraftEvent { createdAt actor { login } } }
      }
    }
  }
}
```

La décision porte sur **`nodes` non vide**, jamais sur `totalCount` : la sémantique
de `totalCount` sous filtre `itemTypes` n'a pas besoin d'être supposée pour que ce
prédicat soit juste.

**Pourquoi pas la timeline REST**, qui a pourtant un lecteur maison
(`ready_label::TimelinePageFetcher`) : c'est mika#2315 lui-même qui l'écarte. Son
défaut **B1 mesuré** est que la timeline REST rend ses événements en ordre
**ascendant**, donc un événement récent vit en dernière page — d'où une pagination
explicite, un plafond de 20 pages, et un refus au-delà. Transposé ici, cela ferait
jusqu'à **20 appels `gh api` par candidat et par tick** (cron 5 min). Le `last: 1`
de GraphQL retire d'un coup le piège de l'ordre, la pagination et le plafond.
Réutilisation : une fonction dans `github_graphql.rs`, dans la forme exacte de
`fetch_open_blockers` (même client `reqwest`, même classification d'erreur
401/403/429). `wip_rescue` appelle `run_gh_subprocess` / `reqwest` directement et
ne traverse pas `GH_API_ALLOW_MATRIX` — le matrice de `run_gh` ne s'y applique pas.

---

## Où le prédicat vit, et pourquoi le placement EST le livrable

**Dans `select_eligible`, comme troisième exclusion — jamais dans `resume_chain`.**

`auto_resume_wip_rescue_drafts` traite **au plus un brouillon par tick** (AC6 de
mika#1852, cap = 1, le plus vieux d'abord). Un hold placé dans `resume_chain`
consommerait donc ce créneau unique **à chaque tick**, indéfiniment, en affamant
tout ce qui est derrière — c'est très exactement la forme de livelock que mika#2199
a mesurée (14 bails sur une seule PR en six heures) et que mika#2286 a dû fermer
une seconde fois. Un brouillon de hold est **vieux par nature** : il reste donc le
plus ancien candidat pour toujours.

Ordre des trois exclusions, du moins cher au plus cher :

| rang | terme | coût | existant |
|---|---|---|---|
| 1 | marqueur de bail (mika#2199) | 1 lecture DB | oui |
| 2 | corps `rescue-pipeline-verified: yes` puis marqueur de park (mika#2286) | test de chaîne gratuit, puis 1 lecture DB | oui |
| 3 | **hold** | 1 appel réseau | **neuf** |

Le court-circuit sur le premier candidat non exclu est déjà la propriété de
`select_eligible` : en régime nominal (aucun hold), le coût ajouté est **un** appel
GraphQL par tick, sur le candidat retenu.

**`select_eligible` prend déjà ses prédicats en injection** — son doc-comment dit
pourquoi : « it makes the selection falsifiable without a database, and lets a test
supply the always-false predicate that reproduces `main`'s livelock ». Le troisième
prédicat suit le même contrat, ce qui rend l'intégralité du correctif testable
**sans réseau**.

---

## Sens d'échec : fail-closed vers « tenu »

| observation | verdict | conséquence |
|---|---|---|
| `nodes` vide | `NotHeld` | comportement d'aujourd'hui, octet pour octet |
| `nodes` non vide | `Held { since, actor }` | exclu, `hold_respected` |
| appel/JSON illisible, timeout, 401/403/429 | `Unreadable` | **exclu**, `hold_unreadable` |

L'asymétrie est mesurée, pas supposée :

- **faux « tenu »** → un brouillon de rescue attend un geste d'opérateur. Visible,
  rattrapable, et c'est le comportement d'avant mika#1852.
- **faux « non tenu »** → le démon sort du brouillon une PR dont la fusion est
  interdite, et la défense en profondeur (forge-gate decision-core) ne tient que
  **par coïncidence de périmètre** — c'est le constat du ticket.

*Un terme qu'on ne peut pas lire n'est jamais un terme satisfait* (mika#2277), ici
appliqué au terme « ce brouillon n'est **pas** tenu ». C'est aussi la politique
**uniforme du module** : `has_bailed_marker`, `has_parked_marker`, `classify_route`
et `fresh_pipeline_verified` sont tous les quatre fail-closed, chacun avec sa
raison écrite. Diverger ici serait la seule exception, et elle irait dans le sens
de l'action sortante.

**Coût nommé :** une panne durable de l'API GitHub gèle `wip_rescue` en entier.
C'est pourquoi `Unreadable` porte **son propre nom d'événement** — l'inertie doit
être greppable et ne pas se confondre avec un hold nominal (deux causes, deux
remèdes, deux comptes : motif `phantom_aged_out` / `phantom_sweep_spared`,
mika#2156).

---

## Ce qui est refusé, avec son motif

- **Un nouveau label `hold` sur la PR** (piste (a) du ticket). Il exigerait que
  l'opérateur apprenne un **nouveau** geste, alors que le geste qui a échoué deux
  fois est « je remets en draft ». Un remède qui repose sur la mémoire de qui
  l'applique dure aussi longtemps que cette mémoire — mesuré par mika#2120 (neuf
  récurrences sous consigne contre zéro quand le geste était fait à la main). Le
  correctif structurel fait tenir le geste **existant**. Il faudrait en plus le
  déclarer dans `.github/labels.yml`, faute de quoi `delete-other-labels: true` le
  supprime en silence du dépôt **et de chaque PR qui le porte** (classe à cinq
  occurrences mesurées).
- **Réutiliser `human-review-required`** comme label de hold. Il porte **sa propre
  cause** (« le démon a bailé vers un humain : conflit/erreur ») ; y router un hold
  d'opérateur rendrait le nom faux et fusionnerait deux populations que le dépôt
  compte séparément. Il reste disponible comme escalade explicite, et il exclut
  déjà (terme existant de `select_eligible`) — c'est le palliatif que le
  commentaire n=2 décrit, et ce plan ne le retire pas.
- **`blocked` sur le ticket** (piste (a), seconde moitié). Orthogonal : `blocked`
  est lu par `is_feeder_excluded` (`auto_pull`) et par la porte 4c du
  `ready_label_handler`. Il ne gouverne pas `wip_rescue`, qui part d'une **PR** et
  non d'une issue. Le brancher ici demanderait la traversée PR → issue
  (`closingIssuesReferences`), un appel de plus, pour un signal que l'opérateur
  pose sur un autre objet.
- **Un kill-switch d'environnement.** Le module n'en a aucun pour ses quatre
  autres termes fail-closed, et un interrupteur sur la garde restrictive serait la
  seule façon de réarmer le défaut par une variable. La garde **ne détruit rien** :
  elle s'abstient. mika#2420 a déjà tranché ce sens pour un scan *destructif* ;
  à plus forte raison pour une abstention.

---

## Requirements

- **R1 — le prédicat.** `wip_rescue::hold` : un enum à trois états
  (`NotHeld` / `Held { since, actor }` / `Unreadable`), jamais un `bool` — deux des
  trois excluent mais appellent des remèdes opposés.
- **R2 — la lecture.** `github_graphql::fetch_convert_to_draft_events(owner, repo,
  number, token)` → `Result<Vec<ConvertToDraftEvent>, String>`, forme de
  `fetch_open_blockers`, `last: 1`, timeout 10 s.
- **R3 — le branchement.** `select_eligible` prend un quatrième prédicat injecté
  `is_held`, consulté **en dernier** (après bail et park), avec le même
  court-circuit par ordre d'âge.
- **R4 — AC3, l'observabilité.** Deux noms distincts, INFO **et** ligne d'audit
  (jamais `debug!` : ce module en a dix, et le dépôt a mesuré qu'un `debug!`
  d'exclusion apparaît **0 fois** sur 200 Mo — mika#2131).
  - `wip_rescue_hold_respected` — champs `pr_number`, `held_since`, `held_by`,
    `trace_id`. **Régime attendu : non vide** ; chaque ligne est un hold tenu.
  - `wip_rescue_hold_unreadable` (WARN) — **régime attendu : vide**.
- **R5 — la déduplication.** Une ligne d'audit par `(PR, motif)` et par **24 h**
  (`tool_name = "wip_rescue_hold_respected"`, `target_key = "pr:{repo}#{n}@{motif}"`),
  lue par `count_recent_audit_events_for_target`. Sans elle, un hold de trois
  semaines écrit 288 lignes/jour — le churn que mika#2131 borne, et la forme
  exacte de `worktree_reap_skipped` (mika#2420). Le séparateur `@` est ce qui rend
  un `LIKE` de préfixe sûr (mika#2361 : `#234` ne doit pas apparier `#2343`).
  **La ligne INFO suit la même porte** : elle sort à la *première* observation puis
  au plus une fois par 24 h — l'opérateur voit donc son hold reconnu au tick
  suivant (≤ 5 min), sans payer 288 lignes/jour ensuite.
- **R6 — co-location.** L'un-draft de l'étape 7 et la sélection ne peuvent pas
  divorcer : un scan de source refuse un second site d'un-draft en production.
- **R7 — aucune valeur de réglage ne bouge.** Ni `MIKA_WIP_RESCUE_MIN_AGE_SECS`, ni
  `MAX_DEPTH`, ni le cap de 1, ni `MIKA_WIP_RESCUE_REPO_DIR`. Aucune migration,
  aucune colonne, aucune variable d'environnement neuve.

---

## Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/src/github_graphql.rs` | +1 fonction + 1 struct (R2) |
| `crates/mika-agent/src/wip_rescue.rs` | enum `HoldVerdict`, prédicat, 4ᵉ argument de `select_eligible`, deux écrivains d'observabilité, dédup 24 h, tests |
| `crates/mika-agent/CLAUDE.md` | § *Auto-resume WIP-rescue drafts* — le troisième terme d'exclusion et son sens d'échec |
| `CLAUDE.md` (racine) | § Signal N (`wip_rescue`) — surfaces opérateur, régimes, sondes, haltes |

**Non touchés, et c'est vérifiable par un diff vide :** `dispatch-lib.sh`,
`builtin_handlers.rs` (la garde mika#1682 reste intacte, cf. R3 du § rectifications),
`ready_label.rs` (son scan `no_ready_label_write_outside_this_module` et son
correctif B1 ne sont pas approchés), `auto_pull.rs`, `.github/labels.yml`.

---

## Verification Contract

Tous les tests vivent dans `wip_rescue.rs::tests` et `github_graphql.rs::tests` —
aucun réseau, par le contrat d'injection que `select_eligible` porte déjà.

| # | test | doit rougir sur `main` ? |
|---|---|---|
| V1 | `mika2597_un_brouillon_tenu_nest_pas_selectionne` — prédicat `Held`, `select_eligible` rend `None` | **oui** (AC2) |
| V2 | `mika2597_un_brouillon_tenu_ne_bloque_pas_le_suivant` — deux candidats, le plus vieux tenu : le second est rendu **au même tick** | oui |
| V3 | `mika2597_un_brouillon_de_rescue_reste_selectionne` — **contrôle négatif** : `NotHeld` ⇒ sélection inchangée | non (doit rester vert) |
| V4 | `mika2597_une_timeline_illisible_exclut` — `Unreadable` ⇒ exclu | oui |
| V5 | `mika2597_les_trois_exclusions_composent` — bail + park + hold sur trois PR distinctes | oui |
| V6 | `mika2597_le_verdict_a_trois_etats_et_deux_noms` — `Held` et `Unreadable` n'écrivent pas le même nom | oui |
| V7 | `mika2597_la_dedup_borne_a_une_ligne_par_24h` — deux ticks consécutifs, une seule ligne ; motif différent ⇒ ligne distincte | oui |
| V8 | `mika2597_un_seul_site_dundraft_en_production` — scan de source, allowlist **livrée vide** | non (vert à la livraison) |
| V9 | `mika2597_le_listing_filtre_toujours_draft` — scan de source : `--draft` est la prémisse du prédicat simple | non (vert) |
| V10 | `fetch_convert_to_draft_events` — parse d'une réponse GraphQL réelle, vide, et malformée (→ `Err`) | oui |

**Le contrôle négatif V3 est porteur** : sans lui, « la garde décide » est
indistinguable de « la garde bloque tout », et le mécanisme mika#1852 serait mort
avec tous les tests au vert. Chaque terme est vérifié **rouge par mutation** avant
livraison, un à la fois — une conjonction de termes fail-safe ne se prouve pas en
les neutralisant tous ensemble (leçon mika#2277).

**V9 n'est pas décoratif** : il épingle la prémisse écrite au § *Le prédicat*. Un
futur éditeur qui retire `--draft` du listing pour élargir la population fait
rougir un test au lieu de rendre le prédicat faux en silence.

---

## Fire-Disposition

Ce plan livre des détecteurs (une garde d'exclusion à l'exécution, deux scans de
source, dix tests). Disposition retenue : **(a) exception nommée en allowlist**,
et les deux allowlists sont **livrées vides et épinglées vides**.

- **V8 (site unique d'un-draft)** — population actuelle : **un** site,
  `resume_chain` étape 7, celui-là même qu'on garde. Le validateur
  `validate_pr_ready_undraft_scope` inspecte un argv et n'un-drafte rien : il est
  hors population. **Zéro violation existante**, donc allowlist vide, plus un test
  frère qui refuse qu'elle cesse de l'être (mika#2323 : une allowlist née vide est
  un tiroir où déposer la prochaine infraction). Quand le scan tire, **on route le
  nouveau site par la garde ; on n'ajoute pas de ligne** (doctrine mika#2201).
- **V9 (prémisse `--draft`)** — même disposition, même raison.
- **La garde d'exécution est livrée ARMÉE, sans interrupteur.** Elle va mordre dès
  le déploiement sur mika#2589, qui est tenue sur trois couches : **ce n'est pas une
  violation à allowlister, c'est le livrable**. Régime attendu de
  `wip_rescue_hold_respected` : non vide. La livrer désarmée (option b) reproduirait
  l'erreur que mika#2272 a dû corriger sur mika#2249 — une condition d'armement
  portant sur un compteur qu'une population vide ne pouvait pas faire monter. Et
  l'asymétrie ci-dessus dit que l'état sûr est l'abstention, pas l'action.
- **Aucune halte-et-remontée (option c)** n'est nécessaire : le périmètre est
  entièrement établi par lecture, et les trois rectifications sont écrites.

---

## Acceptance criteria

1. **AC1 — un hold explicite est respecté par `wip_rescue`.** Une PR `wip-rescue`
   ouverte, brouillon, portant au moins un `ConvertToDraftEvent`, n'est ni
   sélectionnée, ni rebasée, ni poussée, ni sortie du brouillon, et ne voit pas son
   `wip_rescue.depth` incrémenté. **Pour les autres chemins :** le recensement R3
   est publié dans le corps de PR, le chemin 2 est établi comme **déjà fermé** sur
   la population mesurée par mika#1682, et l'élargissement est un suivi nommé avec
   sa précondition.
2. **AC2 — test négatif qui rougit sur `main`.** V1, et V2/V4/V5/V6/V7 avec elle.
   Chaque rougeur est **observée**, pas déduite.
3. **AC3 — le motif de skip est porté par l'audit.** `hold_respected` est lisible
   en SQL et en `grep`, à un niveau collecté (INFO), dédupliqué 24 h, avec son
   frère `hold_unreadable` **comptable séparément**.
4. **AC4 — aucune régression du mécanisme mika#1852.** V3 vert : un brouillon de
   rescue nominal est toujours promu, sur la même route et avec le même verdict.
5. **AC5 — aucune valeur de réglage déplacée, aucune migration** (R7).

---

## Definition of Done

- [ ] `github_graphql::fetch_convert_to_draft_events` + tests de parse (V10).
- [ ] `HoldVerdict` à trois états + son lecteur, dans `wip_rescue.rs`.
- [ ] `select_eligible` prend `is_held` et le consulte en dernier.
- [ ] Deux noms d'observabilité, INFO + audit, dédupliqués 24 h (R4, R5).
- [ ] Dix tests, dont le contrôle négatif V3 et les deux scans à allowlist vide.
- [ ] Chaque terme vu **rouge** par mutation, un à la fois ; trace dans le corps de PR.
- [ ] `cargo test -p mika-agent`, `cargo clippy --tests -- -D warnings`, `cargo fmt`.
- [ ] `crates/mika-agent/CLAUDE.md` et `CLAUDE.md` racine mis à jour (surfaces,
      régimes, sondes, haltes).
- [ ] Corps de PR : le recensement R3, les deux réfutations R1/R2, et le fait que
      l'incident fondateur **n'est pas rattrapé** (§ ci-dessous).

---

## Surfaces opérateur

```bash
# 1. Un hold a-t-il été tenu ? (régime attendu : NON VIDE)
grep wip_rescue_hold_respected "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{pr_number, held_since, held_by}'

# 2. La timeline est-elle lisible ? (régime attendu : VIDE)
grep wip_rescue_hold_unreadable "$MIKA_SPIRIT_LOG_FILE"

# 3. CONTRÔLE POSITIF — le scan tourne-t-il seulement ?
grep -c wip_rescue_resume_attempt "$MIKA_SPIRIT_LOG_FILE"
```

```sql
-- Les holds tenus, datés, un par (PR, motif) et par 24 h
SELECT target_key, created_at, before_value FROM audit_events
 WHERE tool_name = 'wip_rescue_hold_respected' ORDER BY created_at DESC;

-- Contrôle négatif : les promotions continuent-elles ?
SELECT count(*) FROM audit_events
 WHERE tool_name = 'wip_rescue' AND target_key = 'wip_rescue_success';
```

| surface | niveau | régime attendu | lecture |
|---|---|---|---|
| `wip_rescue_hold_respected` | INFO | **non vide** | chaque ligne est un hold que le démon n'a pas violé |
| `wip_rescue_hold_unreadable` | WARN | **vide** | toute occurrence gèle `wip_rescue` par fail-closed |
| `wip_rescue_success` | INFO | non vide, faible | le mécanisme mika#1852 vit toujours — sonde S3 |
| `held_by` ≠ une identité humaine | INFO | **vide** | une machine remet en draft : population inexistante aujourd'hui (R3, ligne 4) |

---

## Sondes post-déploiement, et leurs quatre haltes

> **Préalable.** `wip_rescue` tourne dans **mika-spirit**, pas dans un handler
> seedé : la sonde décrit le binaire servi. Établir le déploiement avant toute
> conclusion (classe mika#2340).

**S1 — le hold tient (premier tick après déploiement).** mika#2589 est tenue sur
trois couches : la sonde 1 doit rendre une ligne la nommant, et aucun
`ReadyForReviewEvent` ne doit apparaître sur sa timeline.
*Halte 1 — aucune ligne et la PR ressort du brouillon :* ne pas élargir le
prédicat par réflexe. Lire d'abord la sonde 3 (contrôle positif) : si le scan ne
tourne pas du tout, la question n'est pas le prédicat. Vérifier ensuite que la PR
est bien dans le listing (`--label wip-rescue`, `--state open`, `--draft`).

**S2 — le mécanisme n'est pas gelé (7 jours).** La sonde SQL de contrôle négatif
doit **continuer à croître** : un brouillon de rescue nominal est toujours promu.
*Halte 2 — le compte est figé et `wip_rescue_hold_unreadable` est non vide :* le
fail-closed gèle tout. La cause est le jeton ou l'API, **pas le prédicat** : lire
`wip_rescue_no_token` (mika#2205) avant de toucher quoi que ce soit.

**S3 — contrôle négatif de faux positif (7 jours).** Aucun
`wip_rescue_hold_respected` sur une PR de rescue que personne n'a remise en draft.
*Halte 3 — une occurrence :* la prémisse « une PR de rescue naît sans
`ConvertToDraftEvent` » est fausse — donc `dispatch-lib` ne crée plus la PR en
brouillon, ou un chemin machine re-drafte. Lire `held_by` : il nomme l'auteur.
**Désarmer d'abord** (revert du terme), diagnostiquer ensuite — un brouillon de
rescue gelé à tort casse le mécanisme mika#1852 en entier.

**S4 — coût.** Un seul appel GraphQL par tick en régime nominal.
*Halte 4 — le quota GitHub se dégrade :* le court-circuit par âge ne fonctionne
pas (la boucle interroge tous les candidats au lieu de s'arrêter au premier non
exclu). Réparer l'ordre de la boucle, **pas** mettre le prédicat en cache — un
cache réintroduirait la fenêtre pendant laquelle un hold fraîchement posé n'est
pas vu.

**Halte transverse — les deux sondes muettes.** Zéro hold **et** zéro promotion ne
prouve rien : il faut qu'un brouillon `wip-rescue` ait existé depuis le
déploiement. *Une garde que personne n'a exercée se lit exactement comme une garde
qui marche* (mika#2205).

---

## Ce que ce travail n'achète PAS

- **Il ne rattrape pas l'incident fondateur.** mika#2589 a été sortie du brouillon
  deux fois, et **rien ici ne réécrit une ligne d'audit datée d'un hold qu'on n'a
  pas observé** — ce serait l'inverse de ce que ce plan défend. La sonde est la
  **prochaine** occurrence.
- **Il ne ferme pas le chemin 2 de R3** pour les PR **hors** signature
  `wip-rescue`. Établi comme déjà fermé sur la population mesurée ; l'élargissement
  est un suivi conditionné à une mesure.
- **Il ne retire pas le palliatif à trois couches.** Le label
  `human-review-required` et le `blocked` sur mika#2105 restent en place et
  continuent d'exclure ; l'opérateur peut les retirer quand la sonde S1 est verte,
  c'est son geste, pas celui de ce plan.
- **Il ne rend pas le hold surveillé, seulement lisible.** Le seul instrument neuf
  est un nom d'événement, et **son silence ne prouve rien tant que personne
  n'exécute les sondes** — sur une population de quelques brouillons, l'absence de
  ligne peut simplement vouloir dire qu'aucun hold n'a été posé.
- **Il ne touche pas le bypass admin** de l'identité du pilote, seconde moitié de
  la défense en profondeur que le ticket mentionne (« la forge-gate a tenu par
  coïncidence de périmètre »). Les deux moitiés tombent séparément.

---

## Hors périmètre, délibérément

- **Élargir mika#1682 à toute PR portant un `ConvertToDraftEvent`** — suivi,
  **précondition écrite** : une mesure montrant qu'un `gh pr ready` de modèle a
  franchi un hold sur une PR hors signature `wip-rescue`.
- **Un filtre d'acteur machine sur le `ConvertToDraftEvent`** — population vide
  aujourd'hui (R3 ligne 4) ; l'acteur est **rapporté** pour que le jour où elle
  cesse de l'être soit visible. Suivi conditionné à une occurrence de la dernière
  ligne du tableau des surfaces.
- **Le faux-étiquetage rescue-class** (mika#1282/#1618) qui fait refuser QA en
  `hold[review]` sur du travail complet — autre défaut, autre remède, nommé par
  mika#2334 comme suivi déjà identifié.
- **La route MECHANICAL non vérifiée** de `undraft_decision` — mika#2286 a
  explicitement ratifié son chemin auto (« closing the mechanical path too is a
  separate ticket with its own measurement »). Ce plan ne le rouvre pas : le hold
  s'applique **avant** la classification, donc aux deux routes également.
- **Le rétro-remplissage du registre d'audit** et la réécriture de l'historique de
  mika#2589.
