# mika#2484 — Un callout de corps sans preuve en base route vers `groom`, et une intention de grooming ne peut plus dispatcher un implement

**Ticket :** mika issue#2484
**Type :** fix (substrat boucle — routage `ready_label_handler`, porte-outil `validate_dispatch_readiness`)
**Date :** 2026-09-22
**Branche :** `fix/2484/feeder-dispatch-callouts-de-corps-sans`

---

## Problème

### Ce que le ticket mesure (2026-09-22, reprise substrat de #2471)

**Défaut 1.** #2471 porte les callouts de grooming dans son **corps** — posés par
un re-groom de spawn orchestrateur, hors moteur, qui ne frappe aucune preuve en
base. Il est vu « groomé », (re)promu `ready`, la tâche `ready-label 0c49ce05`
part en dev-pilot, et la porte mika#1620 la refuse
`dispatch_grooming_not_verified`. Commentaire d'issue : « Promu ready par la
garde 13:35 ». Chaque tour de boucle recommence.

**Défaut 2.** `mika ask --agent mika-dev "groom mika issue#2471"` — **après** le
retrait du plan de la branche mais **avant** le retrait des callouts du corps —
a produit le callback **implement** `5460a97f`, qui a ouvert la PR #2483 (draft
`wip-rescue`, fermée depuis sur ratification). Le **même** `mika ask groom`,
**après** retrait des callouts du corps, a correctement dispatché un dev-groom
(`117d7a20`, `dispatch_class=groom`). Une intention de grooming explicite a donc
produit une implémentation sur un grooming que le chemin moteur n'a jamais
vérifié — **contournement de la porte de preuve**.

### Ce que la lecture du code confirme — et ce qu'elle déplace

**Le défaut 1 ne vit pas dans le feeder. Il vit dans le routage, et le code
affirme par écrit l'invariant qu'il a perdu.**

`server/ready_label_handler.rs:1073-1107`, étapes 5 et 6 :

```rust
// 5. Determine groomed-state via the canonical predicate. Same code path as
//    `validate_dispatch_readiness` gate (#919) — drift between the two
//    sites would re-introduce the bug class this handler closes.
let missing_markers = crate::skills::executor::check_grooming_markers(&body);
let is_groomed = missing_markers.is_empty();
…
let (target_tool, target_skill, dispatch_class) = if is_groomed {
    ("run_claude_pilot", "dev-pilot", "implement")
} else {
    ("run_claude_pilot_groom", "dev-groom", "groom")
};
```

Ce commentaire était vrai en #919 et ne l'est plus depuis mika#1620 /
mika#2287 : `validate_dispatch_readiness` porte désormais **deux** couches —
la forme (`check_grooming_markers`) **et** la preuve
(`has_completed_groom_for_issue`, via `evaluate_grooming_gate`,
`executor.rs:1620-1665`) — tandis que l'étape 5 n'en porte qu'une.

Le résultat est une divergence **à l'intérieur du même handler, à quatre étapes
d'écart** : l'étape 5 répond « groomé » et l'étape 9d
(`ready_label_handler.rs:1291`, qui appelle `validate_dispatch_readiness`)
répond « pas de preuve ». Le handler choisit dev-pilot puis refuse le dev-pilot
qu'il vient de choisir. C'est mot pour mot la classe que mika#2158 a dû fermer
une fois (« promotion et routage du dispatch répondaient différemment à la même
question »), un cran plus bas.

**Le défaut 2 a une cause mesurable de plus que celle que le ticket nomme.** La
table de routage de `skills/bundled/self-dev/system_prompt.md:11-18` reconnaît
`implement <repo> issue#<n>` — la forme typée canonique que
`feedback_task_reference_format` prescrit et que l'opérateur emploie — mais pour
le grooming elle ne connaît que `groom <repo>#<n>` et `groom ticket <repo>#<n>`.
**`groom mika issue#2471` ne matche exactement aucune ligne.** Le modèle
raisonne alors sur la ligne la plus proche (`implement <repo> issue#<n>`), et le
contexte injecté — un corps portant un callout `Plan:` — le confirme dans
l'implémentation. Les deux causes composent ; aucune ne suffit seule à
expliquer que la même commande ait donné deux résultats opposés selon l'état du
corps.

### Ce qui existe déjà, et qu'il suffit de partager

- `executor::evaluate_grooming_gate` (`:1620`) fait **exactement** la décision
  qui manque à l'étape 5 : forme, puis preuve, puis verdict à trois bras. Elle
  ne touche jamais `tasks.result` (son doc-comment le dit), donc elle est
  ré-employable hors du chemin de refus.
- `db.has_completed_groom_for_issue(issue_url)` (`async_db.rs:1631`) est une
  requête SQLite locale, scopée agent, lecture seule.
- Les deux sites construisent **la même** URL : le handler
  `https://github.com/{owner_repo}/issues/{n}` (`:705`), la porte
  `https://github.com/{owner}/{repo}/issues/{n}` (`:1655`). Identiques.
- `validate_dispatch_readiness` reçoit déjà `originating_message` et porte déjà
  deux gardes de message en tête, pures chaînes, avant tout I/O :
  `is_unauthorized_webhook_dispatch` (#933) et l'allowlist de dépôt (mika#2046).
- `webhook_dispatch.rs:43-65` est le module des prédicats de message
  (`is_unauthorized_webhook_dispatch`, `is_ready_label_dispatch_marker`).
- `db/tests/harnais_porte.rs` (mika#2310) fournit `completed_groom_pair`,
  l'écriture de production d'une paire parent/callback portant la preuve —
  condition 5 du GO mika#2287 (« zéro `INSERT` SQL brut dans les tests »).
- Depuis mika#2470 (mergé ce jour, `ff02386f`), la Phase 2 d'`auto_pull`
  dispatche **in-process** en appelant `try_handle_ready_label_dispatch`. Le
  routage de l'étape 5 est donc devenu le lecteur **unique** pour le webhook
  **et** pour le filet de sauvetage : un seul site à réparer couvre les deux.

### Ce que la lecture du code **écarte** du DoD du ticket

Le DoD propose « soit le feeder gate `ready` sur la preuve DB (défaut 1) ».
**Ce remède est contre-productif une fois le routage réparé, et il faut
l'écrire avant le reste.**

Le feeder (Phase 0/1) ne promeut que des tickets que `is_groomed` juge
calloutés (`auto_pull.rs:1589`, `FILTER_NOT_GROOMED`). Avec le routage réparé,
promouvoir `ready` sur un ticket callouté-sans-preuve **déclenche le dev-groom
qui produit la preuve manquante**. Gater le feeder sur la preuve retirerait à
ce ticket le seul mécanisme capable de le réparer : il resterait dans le backlog
sans callout de sortie, pour toujours, sous une ligne d'exclusion
`auto_pull_exclusion` que personne ne relit quotidiennement.

**Le churn que le ticket mesure n'est pas un excès de promotions : c'est une
promotion qui aboutit au mauvais dispatch.** On répare l'aboutissement.

---

## Requirements

- **R1** — Un ticket dont le corps porte les trois callouts mais pour lequel
  aucune preuve `Outcome: PLAN_GROOMED` n'existe en base est routé vers
  **`dev-groom` / `dispatch_class=groom`**, et non vers `dev-pilot`. Preuve
  primaire : la ligne `audit_events` `ready_label_markers_without_proof` ;
  conséquence topologique : la tâche callback créée porte
  `dispatch_class='groom'`.
- **R2** — Un seul lecteur décisionnel de `has_completed_groom_for_issue`. Le
  routage (étape 5) et la porte (étape 9d, et tous les autres appelants de
  `validate_dispatch_readiness`) descendent du **même** prédicat. Tenu par un
  scan de source, allowlist livrée **vide**.
- **R3** — Les quatre JSON de refus de `validate_dispatch_readiness`
  (`dispatch_no_grooming_marker`, `dispatch_grooming_not_verified`,
  `dispatch_check_failed`, et le chemin `Ok`) sont **inchangés à l'octet près**.
  Ce ticket déplace une décision, il n'en change aucune formulation.
- **R4** — Le bras dégradé (`has_completed_groom_for_issue` rend `Err`) route
  vers **`groom`**. Un signal illisible ne satisfait jamais un terme : au
  routage, la direction sûre est le travail le moins dangereux, pas le plus
  avancé. Il est **nommé sous son propre événement**, jamais confondu avec
  l'absence de preuve.
- **R5** — Le chemin nominal reste **silencieux**. Un ticket groomé par la
  boucle (preuve présente) n'écrit aucune ligne nouvelle : une observabilité qui
  enregistre tout le monde ne distingue personne (doctrine mika#2131 AC7).
- **R6** — Aucune porte n'est ajoutée, retirée, élargie ou resserrée dans le
  `ready_label_handler`. Aucune valeur de `ReadyLabelGate` n'est ajoutée ni
  renommée : un ticket callouté-sans-preuve **part** en dispatch, comme
  aujourd'hui, seulement vers l'autre outil. R6 de mika#2323 (aucune surface
  retirée) tient.
- **R7** — Un tour dont le message d'origine porte une **intention de grooming
  explicite** ne peut pas dispatcher `run_claude_pilot` / `dev-pilot`. Le refus
  nomme l'outil correct et est actionnable dans le même tour.
- **R8** — La garde R7 ne peut pas mordre sur un tour de callback, ni sur
  l'auto-fire moteur, ni sur le chemin webhook. Propriété **structurelle** et
  non prudentielle : elle lit `originating_message`, qui est `None` sur ces
  trois chemins.
- **R9** — La table de routage de `self-dev` reconnaît la forme typée
  canonique pour le grooming. Moitié **intention** : elle ne remplace pas R7 et
  n'est jamais invoquée comme sa justification.
- **R10** — Le texte de sortie `already_groomed` de `dispatch-lib.sh` cesse de
  prescrire une route morte.

---

## Décisions

### D1 — Un prédicat à quatre bras, lecteur unique, et la porte l'appelle

Nouvelle fonction dans `skills/executor.rs`, à côté de
`evaluate_grooming_gate` :

```rust
pub(crate) enum GroomedState {
    /// Callouts présents ET preuve en base. Un dev-pilot peut partir.
    Groomed,
    /// Un ou plusieurs callouts manquent. Le cas nominal d'un premier grooming.
    MarkersMissing(Vec<&'static str>),
    /// Callouts présents, aucune preuve. Grooming hors moteur, ou preuve
    /// purgée par la rétention de 30 jours.
    MarkersWithoutProof,
    /// La preuve n'a pas pu être lue (erreur DB).
    ProofUnreadable(String),
}

pub(crate) async fn groomed_state(
    db: &AsyncDatabase,
    owner: &str,
    repo: &str,
    number: u64,
    issue_body: &str,
) -> GroomedState
```

`evaluate_grooming_gate` devient un **traducteur** : elle appelle
`groomed_state` et rend les quatre mêmes JSON qu'aujourd'hui. Aucun de ses
appelants ne change, aucune de ses formulations ne bouge (R3).

Le routage de l'étape 5 appelle `groomed_state` et route :

| bras | outil | classe | ligne écrite |
|---|---|---|---|
| `Groomed` | `run_claude_pilot` | `implement` | — (R5) |
| `MarkersMissing` | `run_claude_pilot_groom` | `groom` | comportement d'aujourd'hui, `note_degroomed_ticket` inchangé |
| `MarkersWithoutProof` | `run_claude_pilot_groom` | `groom` | `ready_label_markers_without_proof` |
| `ProofUnreadable` | `run_claude_pilot_groom` | `groom` | `ready_label_groom_proof_unreadable` |

**Pas de `task_id` dans la signature**, contrairement à
`evaluate_grooming_gate` : l'étape 5 tourne **avant** la pré-création de la
parente (étape 7), et le `task_id` n'est employé par la porte que pour remplir
son JSON de refus. C'est le traducteur qui l'ajoute.

**Pourquoi un `enum` et pas un `bool`.** Trois causes distinctes mènent au même
outil, et elles appellent trois lectures opérateur différentes : « ce ticket n'a
jamais été groomé » (nominal), « il a été groomé hors du moteur » (le défaut de
#2484), « la base ne répond pas » (une panne). Les fondre dans un booléen
rendrait la population de #2484 incomptable — exactement le motif de
`below_threshold` / `no_ready_label_event` (mika#2131) et de
`in_flight_self_dev` / `live_pilot_orphaned_parent` (mika#2279). `match`
exhaustif, **aucun bras `_ =>`** aux deux sites de consommation : le compilateur
force un cinquième état à décider.

### D2 — `check_grooming_markers` n'est pas touchée, et c'est structurel

`grooming_marker.rs:601-625` porte un test de parité : `auto_pull::is_groomed`
et `executor::check_grooming_markers(..).is_empty()` doivent rendre le **même**
verdict sur un corpus partagé. Modifier `check_grooming_markers` pour y intégrer
la preuve casserait ce test — et à raison : `is_groomed` répond de la **forme**
du callout, question à laquelle la base n'a rien à dire, et que le feeder pose
légitimement sans elle.

**Deux questions, deux noms.** `check_grooming_markers` = « la forme est-elle
là ? ». `groomed_state` = « un dev-pilot peut-il partir ? ». La seconde appelle
la première ; l'inverse serait une régression de #2120.

### D3 — La rétention de 30 jours change de conséquence, et c'est un progrès

`prune_completed_tasks(THIRTY_DAYS_SECS)` purge parent et callback, et
`parent_task_id … ON DELETE SET NULL` casse la jointure dès que l'une des deux
part (conséquence écrite de mika#2287). Un ticket groomé il y a plus de 30 jours
perd sa preuve.

- **Aujourd'hui** : routé `implement` → refusé `dispatch_grooming_not_verified`
  → le ticket reste `ready` → Phase 2 le re-drive → abandon
  `redrive_budget_exhausted` après trois tours (mika#2020).
- **Après** : routé `groom` → le dev-groom tourne. Si le plan n'est plus sur la
  branche, il converge et frappe la preuve. S'il y est, `already_groomed` (voir
  D4) et le même budget de re-drive borne la boucle au même endroit.

**Le pire cas est identique à l'état actuel ; le meilleur cas converge.** Aucun
chemin n'est dégradé. Cette lecture est ce qui autorise à ne pas exempter les
lignes `PLAN_GROOMED` du prune ici — suivi nommé de mika#2287, laissé où il est.

### D4 — `already_groomed` : la limite est nommée, bornée, et son texte corrigé

`dispatch-lib.sh:2551` rend un `auto_skipped` / `already_groomed` quand le plan
résout sur la branche de dispatch. Ce JSON **ne porte pas** `Outcome:
PLAN_GROOMED` : aucune preuve n'est frappée. Un ticket callouté-sans-preuve dont
le plan est encore sur sa branche sera donc routé `groom`, répondra
`already_groomed`, et repassera au tour suivant.

**Ce n'est pas une boucle nouvelle, et elle est déjà bornée** : le ticket reste
`ready`, Phase 2 le re-drive, et `MIKA_AUTO_PULL_MAX_REDRIVES` (défaut 3) le
parque en `operator-review` avec un commentaire (mika#2020). Même borne,
au même endroit, qu'avec le refus d'aujourd'hui.

Ce que ce plan corrige, c'est la **note** de ce JSON, qui prescrit aujourd'hui :
« *Dispatch dev-pilot to implement, or remove the plan from the branch to force
a fresh groom* ». Depuis mika#2287, la première moitié est une route morte —
elle mène droit à `dispatch_grooming_not_verified`. Elle est remplacée par le
geste qui marche, celui que `groom_provenance_verdict` nomme déjà dans son champ
`recovery` : retirer le plan de la branche **et** les callouts du corps, puis
re-groomer par la boucle. Un texte de remède qui nomme une route morte coûte un
tour de boucle et une lecture — c'est la leçon de prévention de mika#2287,
appliquée au seul endroit du dépôt où elle n'avait pas été portée.

**Frapper la preuve sur `already_groomed` est refusé**, et le refus est de
principe : la preuve atteste d'une convergence architecte passée par le moteur ;
la frapper sur le constat qu'un plan existe rendrait la porte tautologique —
une garde qui lit sa preuve de la revendication ne peut pas la réfuter
(`a-guard-that-reads-its-evidence-from-the-claim-cannot-refute-it-2026-08-30`).

### D5 — La garde d'intention : porte-outil, pure chaîne, ancrée sur le mot

`webhook_dispatch::is_grooming_intent_message(msg) -> bool` : après `trim_start`,
le message commence par `groom` suivi d'une espace ou d'une tabulation,
insensible à la casse.

**L'ancrage sur le mot est ce qui rend le prédicat sûr, pas un détail de
regex.** `starts_with("groom")` nu mordrait sur « grooming report for mika#N ».
L'espace obligatoire sépare `groom ` de `grooming`, et le contrôle négatif est
un test nommé.

Branchée dans `validate_dispatch_readiness`, **en tête**, avec les deux autres
gardes de message : pure chaîne, aucun I/O, avant le fetch de tâche. Elle ne
mord que sur un `skill` valant `dev-pilot` (via `extract_skill_from_input`) —
un `run_claude_pilot_groom` sous intention de grooming est le chemin nominal.

Refus :

```json
{
  "error": "dispatch_grooming_intent_mismatch",
  "task_id": "…",
  "reason": "This turn was opened by an explicit grooming request …",
  "recovery": "Call `run_claude_pilot_groom` with `skill: \"dev-groom\"` …"
}
```

**La garde refuse le tour entier, pas seulement le ticket nommé**, et c'est un
arbitrage explicite : dériver le numéro d'issue du message pour ne refuser que
lui ajouterait un second parseur là où le seul cas légitime — la chaîne
dev-groom → dev-pilot — ne passe pas par ce chemin (D6). Un tour ouvert par
« groom X » qui dispatche un implement sur Y est déjà un dérapage.

### D6 — Pourquoi la garde D5 ne peut pas tuer le chemin nominal

Propriété **structurelle**, établie par lecture, pas par prudence :

| chemin | `originating_message` | source |
|---|---|---|
| auto-fire post-groom (mika#1614) | `None`, explicitement | `dispatcher.rs:4004-4009` |
| tour de callback | absent — `lr_ctx` vaut `None` | `agent_loop/mod.rs:5111-5140` |
| webhook ready-label | `[GitHub] Issue labeled ready on …` | `ready_label_handler.rs:1288` |
| relance de verdict (block[ac] / block[ci]) | texte de la revue PR | `verdict_handler.rs:847` |
| `mika ask "groom …"` | `groom mika issue#N` | le défaut |

Aucun des quatre premiers ne commence par `groom `. Le cinquième est le seul que
la garde voit — et c'est exactement la population du défaut 2.

La cascade milestone du prompt self-dev (`:521-535`) dispatche son dev-pilot
**dans le tour de callback** du dev-groom (« *Wait for the dev-groom callback.
Do NOT poll* ») : ligne 2 du tableau, `originating_message` absent.

### D7 — La moitié prompt est livrée, et elle n'est jamais le remède

`self-dev/system_prompt.md` gagne la forme typée dans sa table de routage :

| User message contains | Route to |
|---|---|
| `groom <repo> issue#<n>`, `groom <repo>#<n>` ou `groom ticket <repo>#<n>` | **Grooming Dispatch** |

Par `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
(mika#2120 : neuf récidives sous prompt contre zéro écrit à la main), cette
moitié **ne tient pas seule** — c'est D5 qui ferme le contournement. Elle est
livrée parce qu'un modèle à qui l'on donne la bonne route n'a pas besoin d'être
refusé, et que la garde compte alors zéro ligne : le régime attendu.

**Aucun autre changement de prompt.** En particulier, rien n'est ajouté pour
dire au modèle « un ticket callouté peut ne pas être groomé » : cette décision
appartient au moteur depuis D1.

### D8 — Le feeder n'est pas touché

Écarté avec sa raison (§ *Ce que la lecture du code écarte du DoD*). Aucune
ligne d'`auto_pull.rs`, aucun nouveau `FILTER_*`, aucune modification de
`is_groomed` ni de `groomed_candidate_exclusion` ni de `promotion_gate_allows`.

---

## Scope Boundaries

**Dans le périmètre :**
- `crates/mika-agent/src/skills/executor.rs` — `GroomedState`, `groomed_state`,
  `evaluate_grooming_gate` devenue traductrice, la garde D5.
- `crates/mika-agent/src/server/ready_label_handler.rs` — étapes 5/6, les deux
  nouveaux événements.
- `crates/mika-agent/src/webhook_dispatch.rs` — `is_grooming_intent_message`.
- `skills/bundled/_shared/dispatch-lib.sh` — la `note` de `already_groomed`.
- `skills/bundled/self-dev/system_prompt.md` — la ligne de la table de routage.
- Tests, `CLAUDE.md` du crate, `CLAUDE.md` racine.

**Hors périmètre, délibérément :**
- `auto_pull.rs` dans son entier (D8).
- `check_grooming_markers` et `auto_pull::is_groomed` (D2).
- Le contenu des quatre JSON de refus existants (R3).
- La rétention de 30 jours sur les lignes `PLAN_GROOMED` — suivi ouvert par
  mika#2287, laissé chez lui (D3).
- Faire frapper une preuve à `/mika-groom-ticket` ou à `already_groomed` (D4) —
  différé à Vincent/Prime par mika#2287, et ce plan ne le préempte pas.
- `--enable-skill`, `--only-skill`, et toute autre surface du canal `mika.*`.

---

## Implementation Units

### U1 — `GroomedState` et `groomed_state` (D1, D2)

`executor.rs`, juste au-dessus d'`evaluate_grooming_gate`. L'enum, la fonction,
et leurs doc-comments : pourquoi quatre bras et non un booléen, pourquoi
`check_grooming_markers` n'est pas touchée, pourquoi il n'y a pas de `task_id`.

### U2 — `evaluate_grooming_gate` devient traductrice (D1, R3)

Le corps devient un `match` sur `groomed_state`, produisant les quatre mêmes
sorties. Les JSON sont **déplacés, jamais réécrits** : la revue doit pouvoir les
lire comme un `git mv` de lignes.

### U3 — Le routage de l'étape 5/6 (D1, R1, R4, R5, R6)

`ready_label_handler.rs`. Le commentaire de l'étape 5 est **réécrit** : il
affirme aujourd'hui une parité perdue, et le laisser tel quel après l'avoir
rétablie serait garder la phrase qui a rendu la divergence invisible. Il dit
désormais quel prédicat est appelé et pourquoi il est le seul.

`note_degroomed_ticket` reste branché sur le seul bras `MarkersMissing` : la
dé-groomage de mika#2242 est une absence de callout, pas une absence de preuve —
les fondre rendrait la population de #2242 incomptable.

Deux nouveaux événements (INFO + ligne `audit_events`), sur le patron de
`note_degroomed_ticket` : `target_key = "<owner/repo>#<n>"`, dédupliqué par rien
(la population est de quelques événements par jour, chacun un fait daté qu'on
veut **compter** — même arbitrage que `ready_label_outcome`, mika#2323).

### U4 — `is_grooming_intent_message` et son branchement (D5, R7, R8)

`webhook_dispatch.rs` pour le prédicat ; `executor.rs` pour la garde, en tête de
`validate_dispatch_readiness`, après `is_unauthorized_webhook_dispatch` et avant
l'allowlist de dépôt (l'ordre entre gardes pures est libre ; celui-ci groupe les
deux lectures de `originating_message`).

Constante nommée pour le jeton de refus — il atterrit dans `tasks.result` et un
opérateur le `grep`.

### U5 — La `note` de `already_groomed` (D4, R10)

Une ligne de `dispatch-lib.sh:2551`. Assertion correspondante dans
`skills/bundled/_shared/test-dispatch-lib.sh` : la note ne prescrit plus
« dispatch dev-pilot ».

### U6 — La table de routage de `self-dev` (D7, R9)

Une ligne de `system_prompt.md`. **Rappel de déploiement** : un changement sous
`skills/bundled/` n'atteint un agent que par `make deploy` → seed → lecture
(§ *Deploying a bundled-skill change*, mika#2340). Le prompt édité dans l'arbre
est invisible jusque-là.

### U7 — Documentation

`crates/mika-agent/CLAUDE.md` : le prédicat unique et ses quatre bras, à côté de
la section de la porte de provenance. `CLAUDE.md` racine : les deux nouveaux
signaux opérateur et leurs haltes, dans le voisinage de la porte mika#1620.

---

## Verification Contract

Tous les tests sont hors réseau. La preuve se construit par l'API d'écriture de
production (`completed_groom_pair`, `harnais_porte.rs`) — **jamais** un `INSERT`
SQL brut : un fixture SQL peut fabriquer exactement la ligne que la production ne
produit pas, ce qui est la forme même du bug de mika#2287.

**Comportemental — le rouge du ticket (`tests/eval/` ou `executor::tests`) :**

1. `mika2484_callouts_sans_preuve_routent_vers_groom` — corps portant les trois
   callouts, aucune paire groom en base → `groomed_state` rend
   `MarkersWithoutProof`, et le routage choisit `run_claude_pilot_groom` /
   `groom`. **Rouge avant le correctif** (le routage choisit `dev-pilot`).
2. `mika2484_controle_positif_preuve_presente_route_vers_implement` — même
   corps, `completed_groom_pair` posée → `Groomed`, routage `dev-pilot` /
   `implement`. Ce contrôle est porteur : sans lui, un prédicat cassé rendant
   toujours `MarkersWithoutProof` passerait le test 1.
3. `mika2484_callouts_absents_restent_le_chemin_de_mika2242` — corps sans
   callout → `MarkersMissing`, routage `groom`, et `note_degroomed_ticket` est
   toujours atteint.
4. `mika2484_preuve_illisible_route_vers_groom` — erreur DB simulée →
   `ProofUnreadable`, routage `groom`, événement **distinct** du bras sans
   preuve.

**Équivalence de la porte (R3) :**

5. `mika2484_les_quatre_verdicts_de_la_porte_sont_inchanges` — pour chacun des
   quatre états, `evaluate_grooming_gate` rend le JSON attendu, champ `error`
   **et** champs `predicate` / `recovery` / `reason` compris. Les trois tests
   existants (`test_groom_provenance_verdict_*`) restent verts sans
   modification ; si l'un d'eux doit changer, R3 est violée.

**Garde d'intention (R7, R8) :**

6. `mika2484_une_intention_de_grooming_refuse_un_dev_pilot` — `originating_message
   = "groom mika issue#2471"`, `skill: "dev-pilot"` → refus
   `dispatch_grooming_intent_mismatch`.
7. `mika2484_la_meme_intention_laisse_passer_un_dev_groom` — même message,
   `skill: "dev-groom"` → pas de refus.
8. `mika2484_grooming_report_n_est_pas_une_intention` — contrôle négatif du mot :
   « grooming report for mika#2471 » ne mord pas.
9. `mika2484_les_quatre_chemins_moteur_ne_mordent_pas` — paramétré sur
   `None`, le marqueur ready-label, un texte `[claude-pilot]`, et
   `implement mika issue#N`.
10. Insensibilité à la casse et tolérance au blanc de tête : « Groom … », «  groom … ».

**Structurel (détecteurs — voir § Fire-Disposition) :**

11. `mika2484_un_seul_lecteur_decisionnel_de_la_preuve` — scan de source :
    `has_completed_groom_for_issue` n'est appelée, hors définition, wrapper async
    et code de test, que depuis `groomed_state`. Allowlist **vide**.
12. `mika2484_les_noms_d_evenement_sont_un_format_de_fil` — les deux nouveaux
    `tool_name` sont écrits à un seul site chacun (SOLE WRITER), leurs littéraux
    épinglés.
13. `mika2484_le_routage_n_a_pas_de_bras_joker` — les deux `match` sur
    `GroomedState` n'ont pas de bras `_ =>`.

**Non-régression :**

14. `grooming_marker::tests` — la parité `is_groomed` ↔ `check_grooming_markers`
    reste verte **sans modification**. Si elle rougit, D2 a été violée.
15. `scripts/verify-pipeline.sh`, `make verify-bundled-skills`,
    `scripts/test-dispatch-lib.sh`, `cargo clippy`, `cargo fmt`.

---

## Definition of Done

- Les quatorze tests ci-dessus passent ; les tests 1 et 4 ont été **vus rouges**
  avant le correctif et le PR body le dit.
- `cargo test -p mika-agent harnais_porte` reste vert (critère de sortie de
  mika#2310, non régressé).
- Aucun diff dans `auto_pull.rs` (D8), aucun dans `check_grooming_markers` (D2).
- Les quatre JSON de refus sont inchangés à l'octet près (R3), vérifié par
  `git diff` autant que par le test 5.
- `CLAUDE.md` du crate et racine à jour ; `docs/` synchronisé si touché
  (`docs-sync` CI).
- PR ouverte avec le corps écrit sous le worktree (`pr-body.md`, mika#2211).

---

## Acceptance criteria

Dérivés des Requirements et du Verification Contract — le corps de mika#2484
porte un DoD mais pas de section `## Acceptance criteria`.

- **AC1** — Un ticket dont le corps porte les trois callouts de grooming et pour
  lequel aucune ligne callback `Outcome: PLAN_GROOMED` n'existe en base est
  dispatché en `dev-groom` avec `dispatch_class='groom'`, par le chemin webhook
  **et** par le filet Phase 2. Vérifiable par le test 1 et par la sonde S1.
- **AC2** — Le même ticket, une fois la preuve présente, est dispatché en
  `dev-pilot` avec `dispatch_class='implement'`. Test 2.
- **AC3** — `validate_dispatch_readiness` rend exactement les mêmes verdicts
  qu'avant ce ticket sur les quatre états. Test 5, et les trois tests
  préexistants inchangés.
- **AC4** — `mika ask --agent mika-dev "groom mika issue#N"` ne peut pas aboutir
  à un dispatch `dev-pilot`, quel que soit l'état du corps du ticket. Tests 6 et
  8 ; sonde S2.
- **AC5** — Les quatre chemins moteur (auto-fire, callback, webhook ready-label,
  relance de verdict) dispatchent `dev-pilot` sans être refusés par la garde
  d'intention. Test 9.
- **AC6** — Le chemin nominal n'écrit aucune ligne de journal nouvelle. Test 2
  assorti d'une assertion d'absence.
- **AC7** — Un échec de lecture de la preuve route vers `groom` et est
  journalisé sous un nom distinct de l'absence de preuve. Test 4.
- **AC8** — Le texte de sortie `already_groomed` ne prescrit plus « dispatch
  dev-pilot ». Assertion dans `test-dispatch-lib.sh`.
- **AC9** — La table de routage de `self-dev` reconnaît `groom <repo> issue#<n>`.

---

## Fire-Disposition

Ce plan livre **trois détecteurs** (tests 11, 12, 13 — deux scans de source et
une garde d'exhaustivité de `match`) plus une garde de porte-outil (U4).

**Option retenue : (a) exception nommée en allowlist — livrée VIDE.**

- **Test 11** (`un_seul_lecteur_decisionnel_de_la_preuve`) : constante
  `GROOM_PROOF_READERS_ALLOWED: &[&str] = &[]`. La population a été relevée
  avant rédaction — `has_completed_groom_for_issue` a aujourd'hui exactement un
  appelant décisionnel (`executor.rs:1657`), que U1/U2 remplacent par
  `groomed_state`. **Quand ce scan tire, la résolution est de retirer la
  lecture, jamais d'ajouter une entrée** (doctrine mika#2201).
- **Test 12** (`noms_d_evenement_format_de_fil`) : allowlist vide par
  construction — les deux noms sont nouveaux, donc aucune violation
  préexistante n'est possible.
- **Test 13** : garde d'exhaustivité, sans population à exempter — un bras
  `_ =>` est une violation, pas un existant.

**Assertion auto-nettoyante** : les trois allowlists sont vides et un test de
bonne foi (`…_l_allowlist_est_livree_vide`) rougit le jour où l'une cesse de
l'être sans que ce fichier de plan soit mis à jour. Une allowlist vide qui se
remplit en silence est une exception permanente déguisée en état transitoire.

**Aucun détecteur n'est livré désarmé.** L'option (b) est écartée : le défaut
que ce plan ferme est un **contournement de porte de sûreté**, et la condition
d'armement de mika#2272 (« zéro était l'absence de mesure, pas la présence de
prudence ») s'applique — une garde livrée désarmée sur un contournement de
porte se lit exactement comme une garde qui n'a rien à refuser.

---

## Sondes post-déploiement, et leurs haltes

### S1 — Le routage mord (48 h)

```bash
grep ready_label_markers_without_proof "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{repo, num, trace_id}'
```
```sql
SELECT target_key, count(*) FROM audit_events
 WHERE tool_name = 'ready_label_markers_without_proof' GROUP BY 1 ORDER BY 2 DESC;
```

**Régime attendu : non vide et faible.** Chaque ligne est un ticket qui serait
parti en implement sur un grooming non vérifié, et qui part en groom.

**Halte 1 — vide alors que `dispatch_grooming_not_verified` continue
d'apparaître.** Le refus se produit donc **ailleurs** que sur le chemin
ready-label — `mika ask`, relance de verdict, ou auto-fire. **Ne pas élargir le
routage** : établir d'abord quel appelant de `validate_dispatch_readiness` a
refusé, les trois remèdes diffèrent.

**Halte 2 — vide, et `dispatch_grooming_not_verified` aussi, et aucun dispatch
n'a eu lieu.** Établir le déploiement avant toute conclusion (classe mika#2340) :
une ligne absente ne prouve rien tant qu'on n'a pas établi que le binaire qui
tourne sait l'écrire.

**Halte 3 — le compte porte du trafic nominal** (plusieurs par heure, sur des
tickets différents). Le prédicat n'est pas trop large : la flotte a une
population massive de tickets groomés hors moteur, ou la rétention de 30 jours
purge plus vite qu'on ne croyait. **Ne pas désarmer** — mesurer la répartition
entre les deux causes, qui décide du suivi (exemption de prune vs geste de
re-grooming).

### S2 — La garde d'intention (48 h)

```bash
grep dispatch_grooming_intent_mismatch "$MIKA_SPIRIT_LOG_FILE"
```
```sql
SELECT count(*) FROM tasks WHERE result LIKE '%dispatch_grooming_intent_mismatch%';
```

**Régime attendu : zéro ou très proche de zéro** — D7 donne au modèle la bonne
route, et la garde est le filet. Chaque ligne est un contournement de la porte
de preuve intercepté.

**Halte 4 — flot soutenu.** La moitié intention n'atteint pas ce chemin :
**vérifier le déploiement du prompt bundled avant de toucher à la garde**
(`cat ~/.mika/skills/.manifest-writer`, mika#2340) — un `system_prompt.md` édité
dans l'arbre est invisible jusqu'au `make deploy`.

**Halte 5 — une régression sur un chemin moteur** (un dev-pilot légitime
refusé). Le tableau de D6 est faux quelque part : lire quel chemin peuple
`originating_message` avec un texte commençant par `groom `, **ne pas ajouter
d'exception au prédicat** avant de l'avoir établi.

### S3 — Rejeu du défaut fondateur

Sur un ticket portant les trois callouts et sans preuve :

1. `mika ask --agent mika-dev "groom mika issue#<n>"` → la tâche callback doit
   porter `dispatch_class='groom'`. Le rejeu du défaut 2.
2. Poser `ready` (remove → add, mika#2323 : GitHub n'émet `labeled` que sur une
   transition) → même attendu. Le rejeu du défaut 1.

**Halte 6 — le ticket part en groom et revient `already_groomed` en boucle.**
C'est la limite nommée en D4, pas une régression : vérifier que le budget de
re-drive se consomme (`auto_pull_redrive_abandoned` après trois tours) et que le
commentaire d'abandon est posé. Si le budget **ne** se consomme pas, c'est
mika#2158 qui a rouvert (un compteur remis à zéro par l'action qu'il compte) et
c'est **là** qu'il faut chercher.

### S4 — Contrôle négatif (7 jours)

`ready_label_groom_proof_unreadable` doit rester **vide**. Toute occurrence est
une base qui ne répond pas — et un routage qui bascule toute la flotte en groom
sur une panne DB, ce que D1/R4 accepte par sûreté mais qui n'est pas un régime.

---

## Suivi (hors périmètre, nommé)

- **Exempter les lignes `PLAN_GROOMED` du prune de 30 jours.** Suivi déjà ouvert
  par mika#2287 ; D3 montre que son absence ne dégrade plus aucun chemin, ce qui
  en baisse la priorité sans l'annuler. **Précondition** : que S1 montre une
  part significative d'occurrences imputables à la rétention plutôt qu'au
  grooming hors moteur.
- **`/mika-groom-ticket` doit-il frapper une ligne-preuve ?** Différé à
  Vincent/Prime par mika#2287, inchangé ici. Ce plan ne le préempte pas : il
  rend seulement le cul-de-sac convergent au lieu de terminal.
- **`already_groomed` devrait-il être terminal plutôt que de consommer le budget
  de re-drive ?** Question réelle, ouverte par D4. **Précondition** : que la
  halte 6 montre que cette population existe en volume.
- **La garde d'intention devrait-elle vérifier le numéro d'issue ?** Arbitrage
  de D5, à rouvrir si la halte 5 montre un faux positif.

---

## Ce que ce travail n'achète PAS

- **Il ne produit aucune preuve manquante.** Un ticket groomé hors moteur reste
  sans preuve ; ce plan change ce que le moteur en **fait**, pas ce qu'il **en
  sait**.
- **Il ne ferme pas le cas `already_groomed`** (D4). Il le borne au même endroit
  qu'aujourd'hui et corrige le texte qui prescrivait une route morte.
- **Il ne garantit pas que le modèle appelle le bon outil** — il garantit qu'il
  ne peut pas appeler le mauvais. La garde refuse ; elle ne dispatche pas à la
  place du modèle.
- **Aucun compteur de « combien de tickets sont groomés hors moteur »** n'est
  livré : S1 compte ceux que la boucle **rencontre**, et un ticket que personne
  ne relance reste invisible. Une attribution que personne ne déclenche est un
  silence.
- **Aucune valeur de réglage ne bouge** : ni `MIKA_AUTO_PULL_MAX_REDRIVES`, ni
  `MIKA_AUTO_FEEDER_MIN_READY`, ni la rétention.

---

## Références

- mika#2484 (ce ticket) ; mika#2471 (le ticket mesuré) ; mika#2483 (la PR
  produite par le défaut 2, fermée sur ratification).
- mika#1620 (la porte de provenance) ; mika#2287 (la porte réparée, la preuve
  sur la ligne callback) ; mika#2310 (le harnais de la porte) ; mika#1614
  (réutilisation de tâche, bascule groom→implement) ; mika#1572 (le handler
  structurel) ; #919 (le marqueur de dispatch).
- mika#2158 (la divergence de prédicat, un cran plus haut) ; mika#2120 (le
  callout préfixé par le dépôt, et la mesure prompt-vs-substrat) ; mika#2279
  (deux noms pour deux populations qui se ressemblent) ; mika#2131 (l'exclusion
  observable, et « zéro exclusion → zéro ligne ») ; mika#2323 (les noms de porte
  comme format de fil) ; mika#2470 (Phase 2 dispatche in-process — ce qui fait
  du routage le lecteur unique).
- mika#2020 (le budget de re-drive) ; mika#2012 (`already_groomed`) ; mika#2046
  (l'allowlist de dépôt au même site) ; #933 (la première garde de message au
  même site) ; mika#2201 (« on déclare, on n'allowliste pas ») ; mika#1574 /
  mika#2306 (la disposition de tir) ; mika#2272 (pourquoi on ne livre pas
  désarmé par réflexe) ; mika#2340 (rebuild → seed → lecture) ; mika#2211 (le
  corps de PR sous le worktree).
- `docs/solutions/logic-errors/grooming-provenance-gate-reads-a-parent-row-a-sibling-mechanism-flips.md`
  — le récit de mika#2287, dont ce plan applique la leçon de prévention (« un
  texte de `recovery` nomme une route qui marche ») au dernier site qui ne
  l'avait pas reçue.
- `docs/solutions/best-practices/a-guard-that-reads-its-evidence-from-the-claim-cannot-refute-it-2026-08-30.md`
  — pourquoi `already_groomed` ne peut pas frapper la preuve.
- `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`,
  `feedback_task_reference_format`.

---

## Revision history

- **v1 (2026-09-22)** — rédaction initiale. Deux rectifications du corps du
  ticket portées en tête : (a) le défaut 1 vit dans le routage du
  `ready_label_handler`, pas dans le feeder, et le feeder-gate proposé par le
  DoD serait contre-productif une fois le routage réparé ; (b) le défaut 2 a une
  seconde cause mesurable — la table de routage de `self-dev` ne connaît pas la
  forme de référence typée que l'opérateur emploie.
