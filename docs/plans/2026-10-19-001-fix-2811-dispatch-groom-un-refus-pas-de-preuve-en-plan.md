# mika#2811 — Un refus « pas de preuve en base » renvoie au groom, et le groom ne lit plus le corps pour refuser

> La boucle de mika#2484 a changé de place sans se fermer. Son propre code le
> disait déjà : le commentaire de `route_for` nomme cette population comme la
> limite D4 du ticket précédent, et la renvoie au budget de re-drive. Ce vol la
> ferme.

Labels : `bug`, `p2-normal`, `dispatch:loop`, `loop-substrate`, `ready`.

Voisins : mika#2484 (le premier correctif de la classe — a déplacé la boucle),
mika#2012 (l'auto-skip `already_groomed`, 25 requeues / 5 tickets / 13 h),
mika#1620 + mika#2287 (la porte de preuve), mika#2020 (le budget de re-drive,
qui a tenu), mika#2590 (le lecteur ancré + `MarkerOutOfPosition`), mika#2545
(l'ESCALATE terminal), mika#2158 (un seul prédicat pour l'état de grooming),
mika#1333 U2 (HEAD inchangé sur re-dispatch de groom est attendu), mika#2470
(auto_pull Phase 2 dispatche par `try_handle_ready_label_dispatch`).

---

## 1. Contexte : deux portes, deux sources, et personne ne fait le travail

### 1.1 La mesure

Le 2026-10-19 entre 00:50Z et 01:50Z, #1381 (3 re-drives, puis `operator-review`)
et #1959 (2 re-drives au moment du retrait manuel) ont tourné à vide. Cinq
tâches de groom mesurées — `63a0f44d`, `48b462e7`, `b6e28109` (#1959),
`99902916`, `8ab2da4b` (#1381) — chacune complétée en ≈ 2 minutes, sans plan et
sans preuve, sous `parent_resolved_from_dispatch_refusal (reason: already_groomed)`.

Le cycle, à chaque tour :

1. `ready` → tâche `ready-label` ;
2. étape 5 du handler : `groomed_state` rend `MarkersWithoutProof` — le corps
   porte les trois callouts, `has_completed_groom_for_issue` rend `Absent`
   (ces plans d'août viennent d'un `/ce:plan` interactif, qui ne frappe aucune
   preuve) ;
3. `route_for` envoie en `groom` (le correctif de mika#2484) ;
4. la porte de mika#2012 dans `_set_up_worktree` voit
   `_committed_plan_on_branch` réussir → `already_groomed` + `exit 0` ;
5. `try_resolve_parent_on_dispatch_refusal` complète le parent ; le reconciler
   re-drive.

### 1.2 Ce que la mesure établit

**Deux portes répondent à « ce ticket est-il groomé ? » sur deux sources
différentes, et quand elles divergent la divergence elle-même devient le moteur
de la boucle.** La porte de dispatch lit la **base** (preuve = un callback groom
terminé portant une convergence, mika#1620/#2287) ; l'auto-skip du groom lit le
**corps** (les trois callouts). Le refus de la première est précisément l'entrée
de la seconde, et la seconde refuse sur une source que la première a déjà
déclarée non probante.

Ce n'est pas un bug de lecture : les deux lectures sont correctes dans leur
propre monde. C'est la classe mika#2158 — *un seul prédicat pour l'état de
grooming* — appliquée à la paire Rust/Bash plutôt qu'à la paire Rust/Rust que ce
ticket-là avait croisée.

### 1.3 Ce que la mesure n'établit pas, et ce que le code disait déjà

Elle n'établit **pas** une surprise. Le commentaire de `route_for`
(`ready_label_handler.rs:540-552`) décrit l'incident avant qu'il arrive :

> *« si le plan résout encore sur la branche de dispatch, le dev-groom répond
> `already_groomed` (mika#2012) et ne frappe **aucune** preuve — le ticket
> repassera au tour suivant. Cette population-là est bornée ailleurs, par le
> budget de re-drive de mika#2020 »*

mika#2484 a nommé la limite et l'a renvoyée au budget. Le budget a tenu — c'est
le constat du ticket — mais « borné » n'est pas « fermé » : trois créneaux de
dispatch et un `operator-review` pour arriver là où un seul passage suffisait.

Elle n'établit pas non plus que le contournement de la nuit (retrait des
callouts de #1381, #1959, #2680) soit une correction : il rend le ticket
indistinguable d'un premier grooming, ce qui marche **parce que** la source
lue est le corps. Le trou reste entier pour le prochain ticket plané à la main.

---

## 2. La décision

### 2.1 Option 1 du ticket, et pourquoi la machine est déjà prête pour elle

Le ticket pose deux options : (1) forcer une passe réelle de groom quand le
groom a été invoqué **parce que** la porte de dispatch a refusé faute de preuve,
ou (2) tenir le ticket tout de suite avec un label et un commentaire nommant la
divergence.

**Option 1 est retenue**, sur quatre constats de code — pas sur une préférence :

- **La passe forcée converge sans rien produire de neuf.** Le pilote
  (`/mika-groom-plan-only`, Phase 2 étape 4) réutilise un plan existant ; HEAD
  reste donc inchangé. Et **HEAD inchangé sur un dev-groom qui porte un plan
  valide n'est pas un échec de pipeline** : `dispatch-lib.sh:6911-6925` écrit
  *« Note: HEAD unchanged on dev-groom re-dispatch — the plan for `REPO#N` is
  already committed. Architect pass will determine outcome. »*, et le
  commentaire juste au-dessus (mika#1333 U2) dit en toutes lettres *« don't
  poison RESULT with PIPELINE FAILURE for the expected re-dispatch state »*.
  `_iterate_groom_loop` tourne donc (`:12144`, conditionné sur `dev-groom` et
  `PILOT_SESSION_TERMINATED != 1`).
- **La convergence frappe exactement la preuve que la porte exige.** Sur
  GROOMED, `:12175` pose `_set_outcome_line "Outcome: PLAN_GROOMED"` — la ligne
  que `task_state::tasks::groom_result_convergence` lit en ancré depuis
  mika#2590, et que `has_completed_groom_for_issue` transforme en `Converged`.
  Le ticket repart `Groomed` au tour suivant.
- **L'objection écrite dans la note de refus ne tient plus.** Elle dit que
  re-groomer *« stacked a second body callout »*. C'était vrai avant mika#1271 :
  depuis, `_write_canonical_callout` (`:9368`) est **idempotent** — « if all
  three dispatch-gate signals are already in the body (branch line + plan path +
  second-pass GROOMED marker), skip writing ». Un corps déjà callouté n'en reçoit
  pas un second.
- **Le désarmement ne peut pas rouvrir mika#2012.** La cause de #2012 était un
  LLM choisissant `dev-groom` sur un ticket qui avait **déjà** sa preuve ; la
  porte protégeait contre une boucle à état fixe. Ici l'état **change après une
  passe** : la preuve est frappée, la cause de forçage disparaît, et le tour
  suivant lit `Groomed`. Un désarmement conditionné par « il n'y a pas de
  preuve » s'éteint par construction le jour où la preuve existe.

**Option 2 est écartée, et son coût est nommé.** Elle demande un label et un
commentaire depuis le chemin de dispatch — donc un `gh issue edit` et un
`gh issue comment` de plus, là où la pose de label sur épuisement appartient
déjà à mika#2020 (`abandon_stuck_ready`). Elle laisse en outre le ticket au
même endroit : callouts sans preuve, et un opérateur à qui on demande de faire
à la main ce que la passe forcée fait seule en un tour. Elle serait le bon
choix si la passe forcée risquait de ne pas converger ; § 2.1 établit qu'elle
converge, et § 6 borne le cas où elle échoue.

### 2.2 Le prédicat : « pourquoi ce groom a-t-il été demandé », et il n'est pas reconstructible

Le travail attendu dit *« parce que la porte de dispatch a refusé faute de
preuve »*. C'est une **provenance**, et aucune des deux moitiés ne peut la
déduire :

- le shell ne peut pas la lire : il voit un `SKILL=dev-groom` et un ticket, pas
  la raison du routage ; interroger la base depuis le shell serait un troisième
  lecteur de l'état de grooming, exactement ce que mika#2158 interdit ;
- le Rust la **connaît** : `groomed_state` l'a calculée à l'étape 5, et
  `routing_note` l'écrit déjà en audit sous deux noms distincts
  (`cause=markers_without_proof`, `cause=proof_unreadable`).

Donc la provenance est **transmise**, pas recalculée : un champ de l'entrée de
dispatch, écrit par le seul site qui la connaît, lu par le seul site qui doit
s'en servir. C'est le motif de mika#2026 (*un fait est estampillé par son
producteur, jamais reconstruit après coup*).

### 2.3 Pourquoi le champ n'est PAS déclaré dans `tools.json` — l'omission EST le refus

Si `run_claude_pilot_groom` déclarait ce champ, un LLM pourrait le poser et
désarmer la porte de mika#2012 sur n'importe quel ticket, en une phrase. La
porte protège contre une boucle que ce même LLM a produite (#2012 : 6 branches
ne contenant que du markdown).

Le dépôt a déjà son précédent exact, et son doc-comment est le contrat à
recopier — `INTERNAL_DEFERRED_DISPATCH_FIELD` (`executor.rs:4307`) :

> *« The `__internal_*` prefix marks this as engine-internal — not part of the
> public `run_claude_pilot` tool schema. The dev-pilot `tools.json` MUST NOT
> advertise this field; correctness depends on the schema's permissive
> `additionalProperties` mode. »*

Même préfixe, même contrat, même vérification : `skills/bundled/dev-groom/tools.json`
ne porte pas le champ, et un test le lit pour le tenir (doctrine mika#2425 —
*l'omission d'une surface d'outil EST le refus*). Le préfixe `__mika_` est
écarté : il désigne les champs que l'exécuteur long-running injecte à la
livraison (`__mika_task_id`, `__mika_agent`), pas un fait porté dans l'entrée
depuis sa construction.

### 2.4 Deux valeurs, pas un booléen

`groom_cause_for` rend `Option<&'static str>` à **deux** valeurs —
`markers_without_proof` et `proof_unreadable` — parce que les deux populations
ont des signes opposés et sont déjà séparées côté Rust par deux `tool_name`
d'audit SOLE-WRITER (`ready_label_handler.rs:498` et `:509`, régimes attendus
« non vide et faible » vs. **vide**). Les fondre en un booléen côté shell
rendrait la ligne de journal du gate incomptable et ferait lire une panne de
base comme la population nominale du ticket (classe mika#2205 : deux
populations sous un seul nom ne sont pas soustractibles).

Le shell, lui, ne décide que sur la **présence** — il n'a aucune disposition
par valeur. La valeur ne sert qu'à sa ligne de journal. Un seul littéral doit
donc rester en phase entre les deux moitiés : **le nom du champ**, et c'est ce
que le scan de parité tient (§ D5).

### 2.5 Périmètre : le chemin moteur, pas le chemin LLM

Le forçage ne concerne que les dispatches où le **moteur** a routé vers le
groom après avoir lu l'absence de preuve. Un `run_claude_pilot_groom` ouvert par
un tour LLM (« groom mika#X ») ne porte aucune cause, et garde le comportement
d'aujourd'hui, porte de mika#2012 comprise — c'est le flux opérateur de
mika#1173 et il n'est pas dans la population mesurée.

**auto_pull Phase 2 est couvert gratuitement** : depuis mika#2470 il dispatche
en process par `try_handle_ready_label_dispatch`, donc le site réparé couvre le
webhook **et** le filet de rattrapage. C'est le chemin par lequel les cinq
tâches mesurées sont passées.

### 2.6 Un seul vol

392 lignes estimées hors `docs/` contre une borne par vol de 429 (§ *Taille
estimée*). Le vol tient parce qu'il n'ouvre aucun chantier : un champ ajouté à
un JSON existant, une fonction pure à côté de deux fonctions pures jumelles
déjà testées, une branche de garde dans une fonction shell existante, un scan
de parité cloné d'un modèle maison, et **aucun changement de signature** —
`validate_dispatch_readiness`, `build_callback_task` et `_set_up_worktree`
gardent la leur.

---

## 3. Les livrables, site par site

### D1 — `ready_label_handler.rs` : `groom_cause_for`, pure et exhaustive

Deux constantes de valeur, puis la fonction, à côté de `route_for` (`:527`) et
`routing_note` (`:573`) dont elle est la troisième sœur :

```rust
/// La cause transmise au groom quand le routage y envoie faute de preuve
/// (mika#2811). Valeur de fil : lue par `dispatch-lib.sh`, jamais par le Rust.
pub(crate) const GROOM_CAUSE_MARKERS_WITHOUT_PROOF: &str = "markers_without_proof";
pub(crate) const GROOM_CAUSE_PROOF_UNREADABLE: &str = "proof_unreadable";

/// Pourquoi ce groom a-t-il été demandé — `None` quand la question ne se pose
/// pas (mika#2811).
///
/// # `match` exhaustif, aucun bras `_ =>`
///
/// Même contrat que [`route_for`], et le même scan le tient : un cinquième
/// `GroomedState` doit faire rougir le compilateur ici, pas hériter en silence
/// d'un `None` qui laisserait la porte de mika#2012 armée sur une cause neuve.
pub(crate) fn groom_cause_for(
    state: &crate::skills::executor::GroomedState,
) -> Option<&'static str>
```

- `Groomed` → `None` : le routage va en `implement`, la porte du groom n'est
  jamais atteinte.
- `MarkersMissing(_)` → `None` : **délibérément**. Un premier grooming n'a
  aucun plan sur la branche, donc la porte de mika#2012 ne tire pas, et la
  désarmer là n'achèterait rien tout en élargissant la population du forçage à
  tout le nominal. Le forçage ne couvre que les deux états que mika#2484 a
  ajoutés.
- `MarkersWithoutProof` → `Some(GROOM_CAUSE_MARKERS_WITHOUT_PROOF)` — la
  population mesurée du § 1.1.
- `ProofUnreadable(_)` → `Some(GROOM_CAUSE_PROOF_UNREADABLE)` — même direction,
  autre nom (§ 2.4).

**Et le commentaire de `route_for` change.** Son paragraphe de limite D4
(`:540-552`) décrit un comportement que ce vol supprime ; le laisser serait la
régression que mika#2484 lui-même nomme à `executor.rs:1873` (*« une phrase de
ce champ est devenue FAUSSE par l'effet de ce ticket »*). Il devient : la
population est désormais fermée par le forçage, et le budget de mika#2020 ne
borne plus que l'échec de convergence (§ 6).

≈ 35 lignes.

### D2 — `executor.rs` : le nom du champ, et son contrat de non-déclaration

À côté de `INTERNAL_DEFERRED_DISPATCH_FIELD` (`:4307`), dont le doc-comment est
le modèle :

```rust
/// Engine-internal sentinel field name (mika#2811).
///
/// Posé dans l'entrée de dispatch par `ready_label_handler` quand le routage
/// envoie au groom faute de preuve en base, et lu par `dispatch-lib.sh` pour
/// désarmer le seul auto-skip `already_groomed` de mika#2012.
///
/// # Le `tools.json` de dev-groom NE DOIT PAS le déclarer
///
/// Un LLM qui pourrait poser ce champ désarmerait la porte de mika#2012 sur
/// n'importe quel ticket — or cette porte existe contre une boucle qu'un LLM a
/// produite. L'omission de la surface d'outil EST le refus (mika#2425) ;
/// la justesse repose sur le mode `additionalProperties` permissif, comme pour
/// `INTERNAL_DEFERRED_DISPATCH_FIELD`. Épinglé des deux côtés par
/// `canonical_tokens` (parité du littéral) et par un test de lecture de
/// `tools.json`.
pub(crate) const INTERNAL_GROOM_CAUSE_FIELD: &str = "__internal_groom_cause";
```

≈ 20 lignes.

### D3 — `ready_label_handler.rs` : `dispatch_input_for`, et l'absence qui reste une absence

L'étape 9c (`:1507`) construit le JSON en ligne dans un handler de ≈ 1100
lignes, donc intestable. Elle sort en fonction pure — troisième application du
même geste que mika#2484 a fait pour `route_for` :

```rust
/// L'entrée de dispatch que le LLM aurait fournie, plus la provenance que
/// lui ne peut pas connaître (mika#2811).
fn dispatch_input_for(
    target_skill: &str,
    repo_name: &str,
    number: u64,
    task_id: &str,
    groom_cause: Option<&str>,
) -> serde_json::Value
```

Le corps est celui d'aujourd'hui, inchangé à l'octet près pour les trois champs
existants — `prompt` reste la référence **nue** `<repo>#<num>` (mika#1593 : une
forme qualifiée par l'owner route silencieusement en mode texte libre
sans worktree). Puis :

```rust
if let Some(cause) = groom_cause {
    map.insert(INTERNAL_GROOM_CAUSE_FIELD.to_string(), json!(cause));
}
```

**Sans cause, la clé est ABSENTE — jamais `""`, jamais `null`.** Absent n'est
pas zéro (mika#2331), et ici la distinction est load-bearing deux fois : le
`jq -r '… // empty'` du shell rendrait la même chaîne vide dans les deux cas,
mais l'entrée est **persistée** par `build_callback_task` (étape 9e) et lue par
un opérateur en forensique ; une clé présente et vide y dirait « le moteur a
considéré le forçage et l'a refusé », ce qui est faux pour un dispatch
`implement`.

Le site d'appel devient une ligne, après `route_for` :

```rust
let dispatch_input =
    dispatch_input_for(target_skill, location.repo_name(), location.number, &task_id,
                       groom_cause_for(&groomed_state));
```

L'ordre du reste de l'étape 9 **ne change pas** : 9d
(`validate_dispatch_readiness`) voit le champ en plus et s'en moque — sa porte
mika#2484 ne bite que sur `extract_skill_from_input == Some("dev-pilot")` ; 9e
le persiste ; 9h injecte les deux `__mika_*` par-dessus.

≈ 30 lignes.

### D4 — `dispatch-lib.sh` : le désarmement, nommé et borné

**(a) La déclaration du littéral, en colonne 0** — la forme que le scan de
parité de mika#2641 épingle, et un site de définition unique côté shell :

```bash
GROOM_FORCE_CAUSE_FIELD="__internal_groom_cause"
```

**(b) La lecture, dans `_parse_input_json` (`:3062`)**, à côté des deux champs
`__mika_*` et par la constante, jamais par le littéral retapé :

```bash
GROOM_FORCE_CAUSE=$(printf '%s\n' "$INPUT" | jq -r --arg f "$GROOM_FORCE_CAUSE_FIELD" '.[$f] // empty')
```

**(c) Le désarmement, dans la porte de mika#2012 (`_set_up_worktree`, `:4533`)** —
un court-circuit **avant** `_committed_plan_on_branch`, parce que la question
« ce groom a-t-il été forcé ? » est à coût nul et que la mesure du plan coûte
du git :

```bash
if [ "$SKILL" = "dev-groom" ] && [ -n "$GROOM_FORCE_CAUSE" ]; then
    echo "dispatch_gate_groom_forced_no_proof: repo=${REPO} issue=${ISSUE_NUM} branch=${BRANCH} cause=${GROOM_FORCE_CAUSE} — … (mika#2811)" >&2
elif [ "$SKILL" = "dev-groom" ]; then
    # … la porte de mika#2012, inchangée
fi
```

Trois propriétés du signal, chacune une doctrine maison :

- **Un troisième nom, distinct des deux existants.** `dispatch_gate_groom_refused`
  (refus) et `dispatch_gate_groom_allowed_stale_callout` (permis, mais
  re-grooming sur revendication périmée) ont chacun leur population ; le forçage
  est une troisième, et la réutilisation d'un nom la rendrait insoustractible
  (mika#2012 U4, qui a créé le second pour cette raison exacte).
- **La ligne est émise sur le chemin sain**, parce qu'un désarmement qu'on ne
  peut pas observer n'est pas un désarmement, c'est un espoir (mika#2293). Elle
  porte `cause=` pour que le `grep` sépare la population nominale de la panne
  de base (§ 2.4).
- **La note ne CITE PAS le marqueur de convergence en toutes lettres.**
  mika#2590 U5b : la porte prouvait le grooming par une sous-chaîne, donc le
  texte qui disait « ceci n'est pas une preuve » **est devenu** la preuve
  (offset 651, 2026-09-29). Le lecteur est ancré depuis, mais la défense en
  profondeur reste, tenue par une assertion de `test-dispatch-lib.sh`.

Le commentaire du site dit ce qui borne le désarmement — l'état change après une
passe (§ 2.1) — et pourquoi l'objection « stacked a second body callout » de la
note voisine ne s'applique pas à lui (`_write_canonical_callout` est idempotent).

≈ 55 lignes.

### D5 — `canonical_tokens.rs` : la parité du littéral Rust ↔ shell

Clone du modèle `mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust`
(`:5463`), dont le doc-comment énonce déjà exactement le risque :

> *« Une divergence ne casse rien le jour où elle est écrite : le shell continue
> d'écrire, le lecteur continue de ne rien trouver, et la population devient
> vide en silence — c'est-à-dire indistinguable d'un champ sain. »*

Ici la divergence est pire que vide : le shell ne lirait **jamais** de cause,
donc la porte de mika#2012 resterait armée, donc `already_groomed` reviendrait —
avec tous les tests comportementaux des deux côtés verts, chacun correct dans
son propre monde.

`mika2811_le_nom_du_champ_de_cause_est_synchronise_shell_a_rust` :

- lit `skills/bundled/_shared/dispatch-lib.sh`, **panique** si illisible (un
  scan qui ne lit rien se lit comme un scan d'accord) ;
- exige la ligne exacte
  `GROOM_FORCE_CAUSE_FIELD="{INTERNAL_GROOM_CAUSE_FIELD}"` ;
- allowlist `GROOM_CAUSE_WIRE_EXCEPTIONS` **livrée vide**, pinnée vide par
  `mika2811_lallowlist_de_parite_est_livree_vide` — excepter le fichier shell
  rendrait le scan vert en cessant de regarder la moitié qu'il existe pour
  confronter (mika#2201) ;
- message de résolution : réaligner les deux littéraux, **jamais** élargir le
  lecteur shell à une seconde orthographe.

≈ 45 lignes.

### D6 — `executor.rs` : la phrase de remède qui devient fausse

Le champ `recovery` de `groom_provenance_verdict` (`:1879`) finit par :

> *« if the plan already resolves on the dispatch branch (hand-groomed ticket),
> dev-groom answers `already_groomed` and mints no proof — remove the plan from
> the branch first so a fresh loop groom can run. »*

Ce vol rend cette phrase fausse et le geste prescrit inutile. Le précédent est
dans le commentaire du site même, posé par mika#2484 sur la phrase
**précédente** : *« une phrase de ce champ est devenue FAUSSE par l'effet de ce
ticket, et la laisser serait livrer la régression que mika#2287 a nommée : un
texte de remède qui prescrit une route morte »*. Seule cette phrase change ; le
reste du payload reste identique à l'octet près, et le commentaire gagne la
troisième strate.

Même geste pour la note de refus de `already_groomed`, dont le texte continue de
nommer le bon geste pour sa population résiduelle — **un groom LLM** (§ 2.5) —
et le dit désormais ainsi plutôt qu'au général.

≈ 12 lignes.

### D7 — tests Rust (`mod mika2811`, voisin de `mod mika2484`)

| test | ce qu'il tient |
|---|---|
| `mika2811_des_callouts_sans_preuve_forcent_la_passe_de_groom` | `MarkersWithoutProof` → `Some("markers_without_proof")` |
| `mika2811_une_preuve_illisible_force_sous_son_propre_nom` | `ProofUnreadable(_)` → `Some("proof_unreadable")`, valeur **distincte** |
| `mika2811_un_ticket_groome_ne_porte_aucune_cause` | `Groomed` → `None` (contrôle négatif) |
| `mika2811_un_premier_grooming_ne_porte_aucune_cause` | `MarkersMissing` → `None` — la porte de mika#2012 reste armée sur le nominal (§ D1) |
| `mika2811_la_cle_de_cause_est_absente_du_json_quand_il_ny_a_pas_de_cause` | `dispatch_input_for(..., None)` : `get(field).is_none()`, pas `== ""` |
| `mika2811_les_trois_champs_historiques_du_json_sont_inchanges` | `skill` / `prompt` nu (mika#1593) / `task_id` à l'octet près, avec et sans cause |
| `mika2811_le_champ_de_cause_nest_pas_declare_dans_le_tools_json_de_dev_groom` | lit `skills/bundled/dev-groom/tools.json`, assert que le nom du champ est **absent** du fichier entier (§ 2.3) |

Le dernier est le test de l'**omission**, et il est structurel : aucune
assertion comportementale ne peut voir qu'un champ a été ajouté au schéma — le
jour où il l'est, tout reste vert et un LLM gagne un désarmement.

Et **une ligne** dans `mika2484_le_routage_n_a_pas_de_bras_joker` (`:3352`) :
`groom_cause_for` rejoint `["route_for", "routing_note"]` dans la boucle du
scan.

≈ 110 lignes.

### D8 — tests bash (`test-dispatch-lib.sh`, § de la porte de groom)

Structurels, sur `SETUP_WT_SRC=$(declare -f _set_up_worktree)` et
`$(declare -f _parse_input_json)` — l'idiome déjà en place aux lignes 1849-1886 :

- `_parse_input_json` lit le champ **par la constante** (`--arg f "$GROOM_FORCE_CAUSE_FIELD"`),
  jamais par le littéral retapé ;
- le littéral est déclaré **une seule fois**, en colonne 0 (la forme que D5
  lit) — compté, pas seulement cherché ;
- la porte porte le court-circuit `[ -n "$GROOM_FORCE_CAUSE" ]`, et il est
  **avant** `_committed_plan_on_branch` ;
- le signal `dispatch_gate_groom_forced_no_proof` existe, porte `cause=`, et
  n'est **ni** `dispatch_gate_groom_refused` **ni**
  `dispatch_gate_groom_allowed_stale_callout` ;
- la note de forçage ne cite pas le marqueur de convergence en toutes lettres
  (mika#2590 U5b, report de l'assertion existante sur le nouveau texte) ;
- comportemental, les deux sens : cause posée ⇒ pas de `already_groomed` dans la
  sortie du gate ; cause absente ⇒ `already_groomed` inchangé, à l'octet près.

**Le croisement contractuel de la ligne 1708 reste vrai et ne change pas :**
*« si la garde refuse pour `already_groomed`, alors le Rust doit dire groomé »*.
La garde ne refuse plus que sans cause, et sans cause le Rust dit `Groomed` ou
`MarkersMissing` — l'implication tient. Un test l'énonce explicitement pour que
le prochain lecteur ne croie pas devoir l'assouplir.

≈ 85 lignes.

---

## Fire-Disposition

Ce plan livre des détecteurs : les tests Rust (D7), les tests bash (D8), le scan
de parité Rust↔shell (D5) et le scan d'exhaustivité étendu (D7, dernière ligne).

**Disposition retenue : (a) exception nommée en allowlist — livrée VIDE, parce
que la population de violations existantes est nulle par construction.**

Détail, détecteur par détecteur :

- **D5 (parité du littéral)** — l'allowlist existe, est nommée
  (`GROOM_CAUSE_WIRE_EXCEPTIONS`), et est livrée **vide**. Le littéral est créé
  par ce vol : il a exactement un site de déclaration de chaque côté le jour de
  la livraison, donc aucune violation préexistante n'est possible. Une assertion
  auto-nettoyante (`mika2811_lallowlist_de_parite_est_livree_vide`) pinne la
  vacuité — y ajouter une entrée fait rougir un test dédié et exige la
  justification par ticket. Pas d'entrée, donc pas d'entrée stale possible.
- **D7 (scan d'exhaustivité)** — pas d'allowlist du tout : le scan boucle sur
  trois noms de fonction et refuse `_ =>` dans chacune. Les trois sont écrites
  sans bras joker par ce vol, donc vert à la livraison. L'ajout d'un nom à la
  liste est une édition d'une ligne, visible en revue.
- **D7, D8 (tests comportementaux et structurels)** — livrés **armés**, sans
  `#[ignore]`, sans exception : ils décrivent le comportement que ce vol
  introduit, donc verts par construction.
- **Le test d'omission de `tools.json`** — armé, et sa population est vérifiée
  nulle avant écriture : le fichier (35 lignes, lu ci-dessus) déclare quatre
  propriétés et n'en portera pas une cinquième.

**Consigne de déclenchement : (c) halte-et-remontée, jamais d'élargissement.**
Si D5 rougit, le remède est de réaligner les deux littéraux — **jamais**
d'élargir le lecteur shell à une seconde orthographe, **jamais** d'ajouter le
fichier shell à l'allowlist (ce serait rendre le scan vert en cessant de
regarder la moitié qu'il confronte, mika#2201). Si le scan d'exhaustivité
rougit, énumérer le nouvel état — jamais l'absorber.

---

## 4. Contrat de vérification

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | un ticket callouté sans preuve en base ne produit plus `already_groomed` | D8, test comportemental « cause posée » |
| V2 | la passe forcée frappe la preuve | D4 + § 2.1 : HEAD inchangé ⇒ `_iterate_groom_loop` ⇒ GROOMED ⇒ `_set_outcome_line "Outcome: PLAN_GROOMED"` ; sonde S1 ci-dessous |
| V3 | le forçage ne couvre QUE les deux états ajoutés par mika#2484 | D7, les deux contrôles négatifs (`Groomed`, `MarkersMissing`) |
| V4 | les deux causes restent comptables séparément | D7 (deux valeurs distinctes) + D8 (`cause=` dans le signal) |
| V5 | un LLM ne peut pas forger le désarmement | D7, test d'omission de `tools.json` |
| V6 | le littéral ne peut pas diverger en silence | D5 + son allowlist vide pinnée |
| V7 | la porte de mika#2012 est intacte sur sa population d'origine | D8, test comportemental « cause absente », octet pour octet |
| V8 | le croisement Rust↔Bash de 2158 tient encore | D8, l'implication de la ligne 1708 énoncée explicitement |
| V9 | les textes de remède ne prescrivent plus de route morte | D6, relecture des deux champs |
| V10 | rien d'autre n'a bougé | `cargo test`, `cargo clippy`, `make lint`, `make fmt`, `bash skills/bundled/_shared/test-dispatch-lib.sh`, `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main` |

**Sondes post-déploiement** (après `make deploy` — rien de tout ceci n'est vivant
avant, mika#2340) :

```
grep -c 'dispatch_gate_groom_forced_no_proof' /var/log/claude-pilot/*.stderr
grep 'dispatch_gate_groom_forced_no_proof' /var/log/claude-pilot/*.stderr | grep -c 'cause=proof_unreadable'
grep -c 'parent_resolved_from_dispatch_refusal (reason: already_groomed)' ~/.mika/logs/mika-spirit.log
```

Lecture attendue : la première > 0 au premier ticket de la classe, la deuxième
= 0 (régime attendu **vide** de `ProofUnreadable`, sonde S4 de mika#2484), la
troisième n'augmente plus pour un ticket dont le corps porte les trois callouts.
Un `cause=proof_unreadable` non nul est une **panne de base**, pas un succès du
correctif, et se lit comme tel.

---

## 5. Ordre de commit

1. **D2** — le nom du champ et son contrat. Compile, aucun lecteur, aucun
   écrivain, suite verte.
2. **D1 + D3 + D7** — `groom_cause_for`, `dispatch_input_for`, leurs tests, et
   la ligne du scan d'exhaustivité. Le champ part dans l'entrée de dispatch ;
   **aucun comportement ne change encore**, puisque le shell l'ignore.
3. **D5** — la parité. Elle rougit tant que le shell ne déclare pas le littéral,
   donc elle part avec l'étape 4 ou juste après dans la même PR.
4. **D4 + D8** — la déclaration shell, la lecture, le désarmement, le signal, et
   leurs tests. **C'est le commit qui change le comportement.**
5. **D6** — les deux textes de remède.

Plancher mergeable : 1–5 entiers. L'étape 2 seule est inerte et l'étape 4 seule
ne lirait rien : les deux moitiés doivent partir dans la même PR, et D5 est
précisément le détecteur qui refuse qu'elles se séparent.

---

## 6. Risques

- **La passe forcée ne converge pas : l'architecte rend ESCALATE.** Aucune preuve
  n'est frappée (mika#2545 préserve `Outcome: ESCALATE` et ne rejoue pas), le
  corps garde ses callouts, et le tour suivant force à nouveau. **Borné par le
  budget de re-drive de mika#2020 — exactement là où la population d'aujourd'hui
  est bornée** : 3 passes puis `operator-review` + retrait de `ready`
  (`auto_pull::abandon_stuck_ready`). Le coût change de nature plutôt que de
  taille : trois passes réelles avec verdict d'architecte et findings préservés
  en `.iterate/escalate-*.md`, au lieu de trois skips de 2 minutes qui ne
  produisaient rien. Le premier passage, lui, est déjà terminal au sens du
  ticket : preuve, ou halte motivée avec forensique. **Parquer un groom ESCALATE
  dès le premier échec est un autre prédicat et un autre ticket** (§ 7).
- **Le littéral divergent.** Le risque silencieux du vol : le shell ne lirait
  jamais de cause et `already_groomed` reviendrait, tous tests verts.
  Mitigation : D5, avec son allowlist vide et son panic sur shell illisible.
- **Un champ ajouté à `tools.json` « pour la symétrie ».** Il rendrait le
  désarmement forgeable par un LLM, sans qu'aucune assertion comportementale ne
  bouge. Mitigation : D7, test d'omission + le doc-comment du § D2 qui écrit la
  raison au site.
- **Un `_ =>` ajouté à `groom_cause_for`.** Un cinquième `GroomedState`
  hériterait d'un `None` et laisserait la porte armée sur une cause neuve, en
  silence. Mitigation : la ligne ajoutée au scan de mika#2484.
- **Élargir le forçage à `MarkersMissing`** par symétrie apparente. Il n'achète
  rien (pas de plan sur la branche, pas de porte qui tire) et élargit la
  population du désarmement à tout le nominal. Mitigation : § D1 l'écrit, D7 le
  pinne par un contrôle négatif.
- **Le forçage lu comme une permission générale.** La porte de mika#2012 reste
  la porte de mika#2012 : son unique désarmement est conditionné par un fait que
  seul le moteur peut poser, sur un état qui s'éteint après une passe.
  Mitigation : V7 compare la branche non forcée octet pour octet.

---

## 7. Hors périmètre

- **Parquer un groom ESCALATE au premier échec.** mika#2545 l'a rendu terminal
  côté replay sans le parquer côté label ; c'est la seule voie par laquelle la
  passe forcée peut consommer plus d'un créneau (§ 6). Prédicat différent (une
  disposition terminale d'architecte, pas une divergence callout/preuve) et
  propriétaire différent (la pose de label sur épuisement est à mika#2020).
  `Tracked in:` — à ouvrir si la sonde S1 observe un `dispatch_gate_groom_forced_no_proof`
  suivi d'un second sur le même ticket.
- **Le chemin LLM de `run_claude_pilot_groom`** (§ 2.5). Hors population
  mesurée, et le soumettre au forçage demanderait au LLM de fournir la cause —
  exactement ce que § 2.3 refuse.
- **Fusionner les deux lecteurs de l'état de grooming en un seul.** C'est l'axe
  de mika#2158 et il est déjà traité : le shell ne lit pas la base, il reçoit un
  fait de celui qui l'a lue. Un troisième lecteur serait la régression.
- **Le contournement de la nuit** (retrait des callouts de #1381, #1959, #2680).
  Il reste en place et devient inutile ; le réécrire n'est pas du code.
- **`scripts/canonical-tokens.tsv`.** Le nom du champ est **matché** par un
  lecteur shell, donc la question se pose — et la réponse est non : ce fichier
  déclare les tokens qu'un lecteur **Rust** matche (mika#2201), et la parité
  shell↔Rust a déjà son instrument dédié (D5, modèle mika#2641, dont les deux
  littéraux n'y figurent pas non plus).
- **`CLAUDE.md`.** Aucun ajout : cap de taille tenu par
  `crates/mika-common/tests/claude_md_size.rs`. Le fait vit dans les
  doc-comments des deux sites et dans ce plan.

---

## Definition of Done

- [ ] `groom_cause_for` existe à côté de `route_for`, `match` exhaustif sans
      `_ =>`, et rend `None` pour `Groomed` et `MarkersMissing`.
- [ ] `INTERNAL_GROOM_CAUSE_FIELD` existe dans `executor.rs` avec le
      doc-comment nommant son écrivain, son lecteur, et l'interdiction de le
      déclarer dans `tools.json`.
- [ ] `skills/bundled/dev-groom/tools.json` ne porte pas le champ, et un test
      le lit pour le tenir.
- [ ] `dispatch_input_for` est pure et testée ; la clé de cause est **absente**
      du JSON quand il n'y a pas de cause, jamais vide.
- [ ] `dispatch-lib.sh` déclare le littéral une seule fois, en colonne 0, le lit
      par la constante dans `_parse_input_json`, et désarme la porte de mika#2012
      — et seulement elle — quand la cause est non vide.
- [ ] `dispatch_gate_groom_forced_no_proof` est émis sur le chemin sain, porte
      `cause=`, et est distinct des deux signaux existants.
- [ ] La note de forçage ne cite pas le marqueur de convergence en toutes
      lettres (mika#2590 U5b).
- [ ] Le scan de parité Rust↔shell est vert, son allowlist est livrée vide et
      pinnée vide, et il panique sur un shell illisible.
- [ ] `groom_cause_for` est entrée dans la boucle de
      `mika2484_le_routage_n_a_pas_de_bras_joker`.
- [ ] Le commentaire de limite D4 de `route_for` et la phrase de remède de
      `groom_provenance_verdict` sont corrigés — aucun texte ne prescrit plus de
      route morte.
- [ ] `cargo test`, `cargo clippy`, `make lint`, `make fmt`,
      `bash skills/bundled/_shared/test-dispatch-lib.sh`,
      `make verify-bundled-skills` et `scripts/verify-pipeline.sh origin/main`
      passent.
- [ ] La PR nomme les trois sondes du § 4 et dit que
      `cause=proof_unreadable` est attendu à zéro.

## Acceptance criteria

1. **Quand le groom est invoqué parce que la porte de dispatch a refusé faute de
   preuve en base, l'auto-skip `already_groomed` ne s'applique pas.** Le groom
   produit une passe réelle et sa convergence frappe la preuve
   (`Outcome: PLAN_GROOMED`). Une seule source de vérité — la base — pour la
   question qui a causé le refus.
2. **La provenance est transmise, jamais recalculée.** Le moteur, seul à
   connaître la cause du routage, la pose dans l'entrée de dispatch ; le shell
   la lit. Aucun troisième lecteur de l'état de grooming n'est ajouté
   (mika#2158).
3. **Un LLM ne peut pas forger le désarmement.** Le champ est absent du schéma
   de `run_claude_pilot_groom`, et un test structurel le tient — l'omission de la
   surface d'outil EST le refus (mika#2425).
4. **Un ticket avec les callouts dans le corps et sans preuve en base, envoyé en
   dispatch, finit groomé avec preuve. Jamais `already_groomed` suivi d'un
   re-drive.** Tenu par un test comportemental dans les deux sens : cause posée
   ⇒ pas d'`already_groomed` ; cause absente ⇒ porte de mika#2012 inchangée.
5. **Les deux causes restent comptables séparément** — `markers_without_proof`
   (régime non vide et faible) et `proof_unreadable` (régime vide) ont deux
   valeurs de fil distinctes et apparaissent dans le signal du gate, pour qu'une
   panne de base ne se lise jamais comme un succès du correctif (mika#2205).
6. **Le désarmement est observable sur le chemin sain**, par un signal nommé
   distinct des deux signaux existants de la porte (mika#2293, mika#2012 U4).
7. **Le littéral partagé ne peut pas diverger en silence** : un scan de parité
   Rust↔shell, allowlist livrée vide et pinnée vide, qui échoue plutôt que de
   rendre une comparaison vide.
8. **Le forçage ne couvre que les deux états ajoutés par mika#2484.** Un premier
   grooming (`MarkersMissing`) et un ticket groomé (`Groomed`) ne portent aucune
   cause, et la porte de mika#2012 reste armée sur sa population d'origine.
9. **Aucun texte de remède ne prescrit plus de route morte** : la phrase de
   `groom_provenance_verdict` qui demandait de retirer le plan de la branche, et
   le paragraphe de limite D4 de `route_for`, sont corrigés dans le même vol.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| D1 — `ready_label_handler.rs` : `groom_cause_for` + 2 constantes + réécriture du commentaire D4 | 35 |
| D2 — `executor.rs` : `INTERNAL_GROOM_CAUSE_FIELD` + contrat de non-déclaration | 20 |
| D3 — `ready_label_handler.rs` : `dispatch_input_for` + câblage à l'étape 9c | 30 |
| D4 — `dispatch-lib.sh` : déclaration, lecture, désarmement, signal | 55 |
| D5 — `canonical_tokens.rs` : scan de parité + allowlist vide + son assertion | 45 |
| D6 — `executor.rs` : les deux textes de remède | 12 |
| D7 — tests Rust (`mod mika2811`) + 1 ligne au scan d'exhaustivité | 110 |
| D8 — tests bash (`test-dispatch-lib.sh`) | 85 |

Total estimé : 392 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 392 < 429, marge 37 lignes (≈ 9 %).
Le vol n'ouvre aucun chantier qui dépense cette marge : la fonction de cause
s'ajoute à côté de deux fonctions pures jumelles déjà testées et partage leur
scan, le champ s'ajoute à un JSON existant sans toucher une signature, le
désarmement est une branche `elif` devant une garde existante, et le scan de
parité est un clone d'un modèle maison (mika#2641) dont la forme est déjà
écrite. **Aucun appel de `validate_dispatch_readiness`, de
`build_callback_task` ni de `_set_up_worktree` ne change de signature**, donc
aucune édition mécanique de site d'appel.

Levier de resserrement si le vol déborde : D7 fusionne ses quatre tests de
`groom_cause_for` en un seul `#[test]` table-driven (110 → ≈ 85), ce qui rend 25
lignes sans rien perdre de la couverture. Le total par PR (392) est très en
dessous du seuil de 1000, donc aucune découpe en phases n'est requise.
