# mika#2515 — Un `build_mika` vert ne laisse plus une PR muette en silence

> Ticket : `senara-solutions/mika#2515` (loop-substrate, casseur-de-débit).
> Lignée : mika#2355 (garde `qa_build_callback_verdict`), mika#2368 (filet
> `qa_callback_verdict`), mika#2276 (filet deadline webhook), mika#2289 (motif
> `TurnFailed`), mika#2179 (bornes de livraison de callback), mika#2334
> (réconciliateur de demandes de revue).

## Le défaut, mesuré

Le 2026-09-24 sur la PR #2458 (head `6314fb77`) : une revue QA classe les 9 ACs,
met les ACs comportementaux en attente et dispatche `build_mika`. **Deux builds
réussissent** — callbacks `97979b49` (14:49:00Z) et `27560bc1` (15:02:27Z), tous
deux `Build succeeded`. Vingt minutes plus tard la PR est toujours
`reviewDecision=REVIEW_REQUIRED`, dernière revue `mika-platform-qa` =
`COMMENTED`, **aucun verdict final posté**. Et le filet censé rattraper ce
silence n'a rien émis : dernière occurrence de `qa_callback_verdict` /
`qa_deadline_verdict` dans `$MIKA_SPIRIT_LOG_FILE` = **2026-09-18**.

n = 2 : même forme sur la 1ʳᵉ tentative QA de #2465 (build dispatché 3×, non
rendu pendant le tour, verdict capé `hold[review]`), résolue **par hasard** sur
re-QA quand le build a fini avant le tour. #2458 n'a pas eu cette chance.

## Ce que la lecture du code a déplacé

### La piste du ticket est réfutée

Le ticket propose « à investiguer : trigger conjonctif
`[callback: long_running:build_mika]` + `qa-review` dans `loaded_skill_names` ».
**Les deux moitiés tiennent.** `run_silent_agent` compose le message par
`format!("[callback: {label}]")` avec `label = "long_running:build_mika"`, donc
`is_build_callback` mord (égalité de préfixe, épinglée par
`mika2355_the_marker_is_the_shape_the_engine_actually_emits`) ; et
`qa-review/skill.toml` porte `always_on = true`, or `callback_safe_skills()`
amorce précisément sur les skills `always_on`, donc `qa-review` est dans
`loaded_skill_names` de tout tour de callback de mika-qa. `qa_verdict_due` est
vrai. **Le déclencheur n'est pas le défaut.**

### Le défaut est que le filet n'a de population que sur une branche

`post_callback_verdict_net` (`task_engine/dispatcher.rs`) lit
`SilentTurnOutcome.qa_verdict_unmet`, que `run_silent_agent` ne rend que sur la
branche `Ok`. Et `run_loop` ne **pose** ce drapeau qu'aux **deux** chemins de
sortie `LoopResult::Done` (texte non vide, miroir texte vide). Trois façons d'y
échapper, dont deux sont **lisibles dans la source sans reproduire l'incident** :

**(1) `AgentBusy` — le tour ne tourne jamais, et personne ne le dit.**
`dispatch_resume_agent` prend le verrou d'agent par `try_lock()` et rend
`Err(DispatchError::AgentBusy)` **avant** de créer la session et avant
`run_silent_agent`. Le filet est donc structurellement hors d'atteinte. Et le
refus est **muet par construction** : l'appelant
(`engine.rs::dispatch_undelivered_callbacks`) supprime explicitement le WARN
(`if !matches!(e, DispatchError::AgentBusy(_))`), et `record_callback_delivery_failure`
— la télémétrie mika#2179, ses compteurs, sa quarantaine — n'est appelée que dans
la branche `Err` de `run_silent_agent`, c'est-à-dire **après** que le tour a
tourné. Un `AgentBusy` n'incrémente donc `delivery_attempts` **ni** ne déclenche
la quarantaine : la ligne est re-sélectionnée à chaque balayage de 60 s,
indéfiniment, sans compteur, sans ligne d'audit, sans ligne de journal.
**C'est la signature du contexte de contention que le ticket décrit.**

**(2) Le tour est coupé — trou dans les DEUX filets.**
`run_silent_agent` termine ses bras `LoopResult::MaxStepsExceeded` et
`LoopResult::DeadlineExceeded` par `return Ok(SilentTurnOutcome::default())`,
donc `qa_verdict_unmet: false` — **le fait qu'un verdict était dû est
explicitement jeté**. Et le filet mika#2276, qui couvre pourtant exactement le
motif « coupé par la deadline », est câblé au call-site **webhook**
(`handlers.rs`, via `deadline_verdict_target`) : un tour de callback ne le
traverse pas. Donc : mika#2276 couvre *deadline sur tour webhook*, mika#2368
couvre *conclusion muette sur tour de callback*, et **personne ne couvre
deadline sur tour de callback** — alors que c'est le tour le plus chargé de la
chaîne (relire le plan, exécuter les ACs, composer la revue, poster).

**(3) `run_silent_agent` rend `Err`.** Population **nommée par écrit comme hors
périmètre par mika#2368**, dans le `CLAUDE.md` racine : « le tour de callback qui
**échoue** … son silence est déjà borné et instrumenté par mika#2179 … Population
nommée, non couverte, **ticket de suivi** si les sondes montrent qu'elle est non
négligeable. » **mika#2515 est ce ticket de suivi**, plus la population (1).

### Il n'y a aucun rattrapage en aval — et c'est ce qui rend le silence définitif

Le réconciliateur mika#2334 exige, parmi ses six termes conjonctifs, **aucune
revue de `REVIEWER_FORGE_LOGIN`**. Or la revue partielle du 1ᵉʳ tour a posté un
`COMMENTED` sous `mika-platform-qa`. Ce terme est donc **faux pour toujours** :
#2458 est sortie de la population du réconciliateur à l'instant où la QA a posté
sa classification d'ACs. Aucun mécanisme existant ne peut redemander cette revue.
Ce n'est pas un retard, c'est un cul-de-sac.

### Taxonomie — cinq populations, deux couvertes

| # | population | tour tourne ? | filet fire ? | télémétrie aujourd'hui |
|---|---|---|---|---|
| P1 | `AgentBusy` : le verrou d'agent n'est jamais gagné | **non** | non (exige `Ok`) | **rien du tout** (WARN supprimé, compteur mika#2179 non incrémenté) |
| P2 | `run_silent_agent` rend `Err` | partiellement | non (exige `Ok`) | mika#2179 (`callback_delivery_failed`, quarantaine) — ne dit rien d'une PR muette |
| P3 | tour coupé (`DeadlineExceeded` / `MaxStepsExceeded`) | **oui** | **non** (drapeau posé aux seules sorties EndTurn) | `agent deadline exceeded` — ne dit rien d'une PR muette |
| P4 | tour conclu muet, filet s'abstient | oui | abstention **nommée** | `qa_callback_verdict outcome=no_*` ✅ |
| P5 | tour conclu muet, filet poste | oui | oui | `qa_callback_verdict outcome=posted` ✅ |

P1, P2 et P3 laissent un build vert et une PR muette **sans aucune ligne
attribuant la perte**. C'est le périmètre.

### Le trou de l'estampille, nommé et NON refermé ici

`resolve_qa_review_pr_target` (`skills/executor.rs`) estampille la cible PR au
**spawn** du `build_mika`, résolue depuis `originating_message` par le lecteur
unique `deadline_verdict::parse_pr_target`, qui ne connaît que deux grammaires,
toutes deux produites par le gateway (`[GitHub] PR review (…) on {repo}#{n}` et
`[GitHub] PR {action}: {repo}#{n}`). Une revue QA **dispatchée en texte libre**
(`mika ask --agent mika-qa "review PR #2458"`) ne produit donc **aucune
estampille** — seulement un WARN `qa_review_pr_target_unresolved
reason=not_a_pr_event`. Or le ticket dit « QA **directe** dispatchée sur #2458 ».

**Conséquence de conception, et elle décide la forme de U2 :** la population de
l'alerte est **le callback de build**, jamais l'estampille. L'estampille
*enrichit* la ligne (elle nomme la PR) ; elle ne conditionne pas la détection.
Sans quoi ce travail serait silencieux sur exactement le dispatch mesuré.

Élargir `parse_pr_target` au texte libre est **refusé** : ce serait une seconde
grammaire de fil pour une même question — la classe que mika#2158 a dû refermer —
et elle ne pourrait de toute façon pas résoudre le dépôt. Suivi nommé plus bas.

## Surface retenue — trois unités, aucune migration

Le principe : **fermer les deux trous structurellement prouvables au plus près de
leur site, et rendre toute la population attribuable.**

- **U1 — P3 : le tour coupé poste son verdict.** Le filet mika#2368 existe, la
  cible est déjà estampillée, le registre anti-double-post et la classification
  du 422 sont déjà là. Ce qui manque est le **signal** : `run_loop` le pose à
  deux sorties sur quatre. Armé, sans nouveau levier (il roule sur
  `MIKA_QA_CALLBACK_VERDICT_NET`).
- **U2 — P1/P2 : une alerte nommée et bornée.** Un balayage périodique sur les
  callbacks de build non livrés au-delà d'une fenêtre, qui émet
  `qa_build_verdict_undelivered` **en attribuant la perte au chemin**.
  Détection seule ; le POST est un suivi conditionné (voir *Hors périmètre*).
- **U3 — rendre l'attribution positive plutôt qu'inférée.** Aujourd'hui la
  signature de P1 est « `delivery_attempts` absent », c'est-à-dire une
  **absence** — et la règle de la maison est qu'une absence n'est pas une
  preuve. Le refus `AgentBusy` sur un callback de build devient **compté**.

## U1 — Le tour coupé pose son verdict

### Le signal

`run_loop` reçoit aujourd'hui `qa_verdict_unmet: Option<&AtomicBool>`. Il reçoit
désormais `Option<&qa_build_callback::VerdictSignal>` — **même arité**, un struct
groupant les deux faits, déclaré dans `qa_build_callback.rs` à côté des prédicats
qu'il sert :

```rust
pub struct VerdictSignal {
    /// Sorties EndTurn : budget de la garde épuisé, rien de posté (mika#2368).
    unmet_after_retry: AtomicBool,
    /// Sorties coupées : un verdict était dû, rien de posté (mika#2515).
    cut_off: AtomicBool,
    cut_off_exit: AtomicU8,      // discriminant de `CutOffExit`
    cut_off_steps: AtomicUsize,
}
```

Les deux moitiés sont **mutuellement exclusives par construction** — un tour sort
par exactement un chemin — et un test l'épingle plutôt que de le tolérer en
silence.

### Le prédicat, et pourquoi il n'a PAS le terme de budget

```rust
/// Un verdict était dû sur ce tour coupé, et rien n'a été posté (mika#2515).
pub fn verdict_unmet_at_cut_off(
    qa_verdict_due: bool,
    summaries: &[ToolCallSummary],
) -> bool {
    qa_verdict_due && !pr_review_posted_in_turn(summaries)
}
```

Frère de `verdict_unmet_after_retry`, **moins** le terme
`intent_guard_retries.contains(QA_VERDICT_REQUIRED_LABEL)`, et c'est toute la
différence : sur un tour coupé la garde peut n'avoir **jamais eu l'occasion de
firer** (la coupure se produit en tête d'itération, avant tout EndTurn). Exiger
un budget dépensé rendrait le prédicat **structurellement insatisfiable sur
exactement la population qu'il vise** — la leçon mika#2272 : une condition
insatisfiable se lit exactement comme une flotte saine. Les deux prédicats
partagent `pr_review_posted_in_turn`, donc un tour coupé **après** avoir posté sa
revue ne déclenche rien (contrôle négatif porteur).

### Le motif, et pourquoi il ne réutilise pas `CutOffByDeadline`

Quatrième variante de `VerdictReason` :

```rust
/// mika#2515 — le tour de callback de build QA a été **coupé** avant de poster.
CallbackCutOffWithoutVerdict { exit: CutOffExit, steps_completed: usize },
```

Trois raisons de ne pas router sur `VerdictReason::CutOffByDeadline` :

1. son `event_name()` est `DEADLINE_VERDICT_EVENT` (`qa_deadline_verdict`), dont
   la **sonde de contrôle négatif de mika#2355** dit en toutes lettres « le filet
   mika#2276 ne doit pas se mettre à firer » — y router une population neuve
   rendrait cette sonde fausse ;
2. son corps est **figé octet pour octet** par
   `mika2368_the_deadline_body_is_frozen_byte_for_byte` (AC5c) ;
3. son texte dit « a atteint la limite de son enveloppe de temps », ce qui est
   **faux** d'un `MaxStepsExceeded`.

`event_name()` rend donc `CALLBACK_VERDICT_EVENT` (`qa_callback_verdict`) — c'est
une perte du chemin callback, elle appartient à cette population — **discriminée
par le champ `cause`**, exactement le geste que mika#2289 a déjà fait
(`cause = "error"` sur `DEADLINE_VERDICT_EVENT`). Les lignes mika#2368
existantes ne portent **aucun** `cause` et restent identiques : la soustraction
opérateur est `select(.cause == null)` contre `select(.cause | startswith("cut_off"))`.

`cause` est un **format de fil** (l'opérateur en fait des `GROUP BY`) : trois
valeurs, un seul site de définition, `match` exhaustif **sans bras `_`**, épinglé
par test. `cut_off_deadline` / `cut_off_max_steps` / (`error`, hérité).

### Le corps

Nouvelle branche de `build_verdict_body`, qui nomme **quelle** borne a été
franchie et le nombre de steps, et ne parle ni de « conclusion » (le tour n'a pas
conclu) ni de la garde (elle n'a peut-être jamais firé). Même obligation que ses
trois sœurs : `DEADLINE_VERDICT_LINE` et rien d'autre, asserté **et** en passant
le corps produit à `server::verdict::parse_verdict` — la constante seule ne
prouve pas ce que la machine d'état lira, et c'est elle qui merge sur `pass`.

### Le câblage

`post_callback_verdict_net` sélectionne son motif par un `match` sur le couple,
quatre bras explicites :

| `unmet_after_retry` | `cut_off` | motif |
|---|---|---|
| `false` | `None` | retour immédiat (chemin nominal, gratuit) |
| `true` | `None` | `CallbackConcludedWithoutVerdict` (mika#2368) |
| `false` | `Some` | `CallbackCutOffWithoutVerdict` (mika#2515) |
| `true` | `Some` | inatteignable par construction — **le plus spécifique gagne** (`cut_off`), jamais un `unreachable!()` : paniquer dans le dispatcher sur un champ d'observabilité serait le pire des échanges |

Tout le reste du filet est inchangé : estampille, abstentions nommées, token
PAT-first (ADR-008), registre, 422 idempotent, « ne jamais rendre d'erreur ».

## U2 — L'alerte `qa_build_verdict_undelivered`

### Site et population

Troisième bras du bloc de balayage à 60 ticks d'`engine.rs`, **après**
`dispatch_undelivered_callbacks` (une ligne que ce balayage vient de tenter de
livrer n'est pas à alerter avant qu'il ait essayé). Il **réutilise
`get_undelivered_callback_tasks`** — aucune requête neuve, aucune migration — et
filtre **dans l'application**, là où le refus peut se journaliser (leçon
mika#2184 : le proxy filtre en SQL, la mesure directe tranche dans le code).

Quatre termes conjonctifs :

1. `task.label == qa_build_callback::BUILD_CALLBACK_LABEL` — c'est un callback de
   build ;
2. le statut est dans la population non livrée (`completed` / `failed`, jamais
   `delivered`) — garanti par la requête réutilisée ;
3. `completed_at` plus vieux que la fenêtre ;
4. la ligne n'a pas déjà été alertée pour **cette** cause dans les 24 h.

**L'estampille n'est pas un terme.** Elle enrichit la ligne (`target`), et son
absence vaut `target=unresolved` — voir *Le trou de l'estampille* ci-dessus.

### L'attribution — quatre valeurs, exhaustives, format de fil

Lue sur la ligne, sans une requête de plus : les clés que mika#2179 écrit déjà
plus le compteur d'U3.

| `cause` | signature sur la ligne | lecture |
|---|---|---|
| `agent_busy_starvation` | `verdict_delivery_deferrals > 0`, `delivery_attempts` absent/0 | **la contention** : le verrou n'a jamais été gagné, le tour n'a jamais tourné |
| `turn_failed` | `delivery_attempts > 0`, pas de quarantaine | mika#2179 réessaie ; la PR attend |
| `quarantined` | `delivery_quarantined_at` présent | budget épuisé, réessai horaire — le verdict est effectivement perdu |
| `never_attempted` | les deux compteurs absents | **anomalie** : rien n'a jamais tenté de livrer cette ligne. Bras anti-vacuité — sans lui, une ligne que personne n'a touchée se lirait comme une famine |

Le quatrième bras est ce qui empêche l'instrument de mentir : c'est la même
raison qui a fait séparer `below_threshold` de `no_ready_label_event`
(mika#2131) et `in_flight_self_dev` de `live_pilot_orphaned_parent` (mika#2279) —
deux états qui se ressemblent, deux remèdes différents.

### Déduplication

Une ligne d'audit par `(task, cause)` sur un horizon de 24 h (doctrine
mika#2131), via `count_recent_audit_events_for_target` — la primitive que
`ci_success_handler` et `qa_review_reconcile` lisent déjà, donc aucune méthode DB
neuve. Un **changement** de cause réécrit : c'est un changement d'état. Le WARN
suit la même déduplication : à 60 s de cadence et 9 min de fenêtre, une ligne par
passe par PR serait le churn que la doctrine borne. **Zéro alerte ⇒ zéro ligne.**

Lecture **fail-open** : un `audit_events` illisible alerte quand même, et le dit
sous son propre nom. L'inverse de `wip_rescue` (mika#2199) et pour la raison
opposée : là un faux positif rejouait une revue ; ici le pire d'un faux positif
est **une ligne de journal en trop**, et le pire d'un faux négatif est le silence
que tout ce ticket ferme.

### Détection seule — et ce n'est pas de la timidité

Le critère d'acceptation dit « **SOIT** un verdict posté **SOIT** une alerte
nommée ». L'alerte satisfait l'AC. Poster est **refusé ici**, sur une mesure et
non par prudence de principe : pour P1 le verdict n'est pas perdu, il est **en
file** — la ligne réessaie toutes les 60 s et postera le vrai verdict dès que
mika-qa se libère. Poser un `hold[review]` sur une PR dont le verdict est
seulement *en attente* serait une affirmation activement trompeuse, et une revue
postée par `mika-platform-qa` sort la PR de la population de mika#2334 **pour de
bon** (coût écrit dans `deadline_verdict.rs`). Pour P2-quarantaine, en revanche,
poster serait juste — population distincte, suivi nommé, **précondition : que la
première distribution montre cette population non vide.**

## U3 — Le refus `AgentBusy` devient compté

Au site du `try_lock()` échoué de `dispatch_resume_agent`, **et seulement pour un
callback de build** (un `task.label ==` sur le chemin nominal, gratuit) :

- `verdict_delivery_deferrals` — compteur incrémenté à chaque refus ;
- `verdict_delivery_first_deferred_at` — instant, écrit **NULL-only**, idiome
  `FIRED_AT_STAMP_IF_NULL` de mika#2133 : la première fois où ce verdict a
  commencé à attendre.

Écriture par `set_delivery_metadata`, la voie que mika#2179 emploie déjà, avec sa
mise en garde reprise : `json_set(COALESCE(metadata,'{}'))` échoue **dur** sur un
`metadata` non-JSON. Le `metadata` d'un callback de build est écrit par
`build_callback_task` et est du JSON valide ; un échec d'écriture est
journalisé et n'empêche **jamais** le refus `AgentBusy` de rendre — U3 est une
mesure, pas une décision.

**Aucun nouvel événement de journal pour U3**, délibérément : un WARN par refus
serait une ligne par minute par PR affamée. Le compteur atterrit dans
`mika tasks get` et dans la ligne d'U2, qui est déjà bornée. Une mesure, un
instrument.

## Réglages

Trois clés, forme maison à trois paliers (absent/vide → défaut ; illisible, `0`
ou négatif → défaut **plus** un WARN nommant la valeur entre guillemets), champs
`Option<…>` sur `Settings` avec accesseur `effective_*`, déclarées dans
`.env.example`.

- **`MIKA_QA_BUILD_VERDICT_ALERT_AGE_SECS`** — fenêtre avant alerte, défaut
  **`540`**. **L'arithmétique, pas une rondeur :** l'AC demande « dans les 10
  minutes », le balayage tourne à 60 s, donc `540 + 60 = 600 s` est la borne
  haute effective. Le knob existe parce que le nombre est **posé contre l'AC, pas
  mesuré** : mika#2179 a mesuré la latence de livraison de *tous* les callbacks
  (`p50 = 377 s`, `p90 = 9585 s`), mais la sous-population « callback de build
  dont un tour QA attend le retour » n'a jamais été mesurée. Asymétrie assumée,
  celle que mika#2496 écrit pour son propre seuil de coût : **un seuil d'alerte
  ne coupe rien** — un faux positif coûte une ligne de WARN. Et la population est
  minuscule (un callback de build n'existe que quand une revue QA dispatche un
  build), donc même un taux de déclenchement élevé fait quelques lignes par jour,
  chacune nommant une PR qui attend réellement son verdict. **Halte si la
  distribution montre du trafic nominal : c'est le seuil qui monte, jamais
  l'alerte qu'on désarme.**
- **`MIKA_QA_BUILD_VERDICT_ALERT`** — kill-switch de l'alerte, défaut **armé**.
  `0`/`false`/`off`/`no` désarment sans redéploiement ; une valeur non reconnue
  est **dite** et laisse armé — un désarmement par coquille sur un instrument de
  sûreté serait la panne silencieuse que ce ticket ferme. Désarmé, le balayage
  n'écrit **rien** : il ne « s'abstient » pas, il n'existe pas ce tick, et une
  ligne INFO au démarrage le dit (sinon un instrument désarmé se lit exactement
  comme un instrument sain — mika#2205).
- **`MIKA_QA_BUILD_VERDICT_ALERT_LOOKBACK_DAYS`** — profondeur du `since` passé à
  `get_undelivered_callback_tasks`, défaut **`7`** (la valeur que
  `dispatch_undelivered_callbacks` emploie déjà, pour que les deux bras voient la
  même population).

Aucune valeur existante ne bouge : ni `MIKA_QA_CALLBACK_VERDICT_NET`, ni les
budgets mika#2179, ni les enveloppes LLM, ni le budget d'attente #2163.

## Unités d'implémentation

| U | fichier | contenu |
|---|---|---|
| **U1a** | `src/qa_build_callback.rs` | `VerdictSignal`, `CutOffExit` (+ `as_cause()`, `match` sans `_`), `CallbackCutOff`, `verdict_unmet_at_cut_off` |
| **U1b** | `src/agent_loop/mod.rs` | paramètre `run_loop` → `Option<&VerdictSignal>` ; pose aux **quatre** sorties ; `SilentTurnOutcome` gagne `qa_verdict_cut_off: Option<CallbackCutOff>` ; les bras `MaxStepsExceeded` / `DeadlineExceeded` de `run_silent_agent` lisent le signal au lieu de rendre `default()` |
| **U1c** | `src/server/deadline_verdict.rs` | variante `CallbackCutOffWithoutVerdict`, son corps, `event_name()`, le champ `cause` sur la ligne postée |
| **U1d** | `src/task_engine/dispatcher.rs` | sélection du motif dans `post_callback_verdict_net` (match à quatre bras) |
| **U2a** | `src/qa_build_callback.rs` | `UndeliveredVerdictCause` (4 variantes, format de fil), `classify_undelivered_verdict(metadata) -> UndeliveredVerdictCause` — **fonction pure**, testable aux bornes sans base |
| **U2b** | `src/task_engine/engine.rs` | `alert_undelivered_build_verdicts()` dans le bloc à 60 ticks, après `dispatch_undelivered_callbacks` ; dédup 24 h ; WARN + ligne `audit_events` |
| **U3** | `src/task_engine/dispatcher.rs` | compteur + instant NULL-only au site `AgentBusy`, borné aux callbacks de build |
| **R** | `crates/mika-common/src/config.rs`, `.env.example` | les trois clés, leurs constantes `DEFAULT_*` documentées, les accesseurs |
| **T** | tests + scans | voir *Vérification* |
| **D** | `CLAUDE.md` racine, `crates/mika-agent/CLAUDE.md` | entrée opérateur, taxonomie, sondes, haltes |

## Fire-Disposition

Ce plan livre des détecteurs (scans de source et tests d'assertion). Disposition
retenue : **(a) exception nommée en allowlist — et les allowlists sont livrées
VIDES**, parce qu'il n'y a rien à exempter : les trois scans portent sur des noms
et des sites que ce travail **crée**. Détail :

- `mika2515_the_undelivered_alert_has_a_single_writer` — scan de source, SOLE
  WRITER de `qa_build_verdict_undelivered` (journal **et** `audit_events`).
  Allowlist `const ALLOWED: &[&str] = &[]`, plus un test frère
  `…_the_allowlist_is_empty` qui refuse qu'elle cesse de l'être (mika#2323 : une
  allowlist née vide est un endroit où déposer la prochaine infraction). Porte sa
  propre **assertion anti-vacuité** : le scan échoue si le nom est écrit **nulle
  part** — un scan qui visait un nom mort se lit exactement comme un scan propre
  (mika#2103 / mika#2205).
- `mika2515_the_cut_off_signal_is_posed_at_every_loop_exit` — scan de source sur
  `agent_loop/mod.rs` : **cardinalité assertée à 4** sites de sortie de
  `run_loop` posant le signal. La cardinalité est le seul terme qu'aucune fixture
  ne peut voir : un prédicat devenu trop étroit passerait en ne regardant rien.
  Allowlist vide.
- `mika2515_the_cause_values_are_a_wire_format` — fige les valeurs de `cause`
  (`cut_off_deadline`, `cut_off_max_steps`, `error`) et les quatre valeurs de
  `UndeliveredVerdictCause`. Pas d'allowlist (ce n'est pas un scan de sites).
- Les tests comportementaux (V1–V7) sont neufs sur du comportement neuf : aucune
  population préexistante, donc aucune exception, et **aucun n'est livré
  `#[ignore]`**.

**Quand un scan tire, la résolution est de retirer le second écrivain ou d'armer
le site manquant — jamais d'ajouter une entrée** (doctrine mika#2201). Aucun test
comportemental ne peut voir ces classes : un second écrivain ne rend **aucune
décision fausse** le jour où il est écrit, il rend seulement deux populations
inséparables.

## Vérification

Tout est déterministe et hors réseau (`MockLlmProvider`, `EvalHarness`, poster
injecté).

- **V1 — le prédicat de coupure.** `verdict_unmet_at_cut_off` : vrai sur
  (dû, rien de posté) ; faux sur (dû, revue postée) ; faux sur (non dû, rien de
  posté). Et **le contrôle qui porte tout U1** : vrai **sans** que
  `intent_guard_retries` contienne le label — la différence avec
  `verdict_unmet_after_retry`, sans quoi le prédicat serait insatisfiable sur sa
  propre population.
- **V2 — le signal sort des quatre sorties (chemin de production).** Quatre
  scénarios `run_silent_agent` sur un message
  `[callback: long_running:build_mika]` avec `qa-review` chargé : sortie EndTurn
  muette → `qa_verdict_unmet` ; sortie EndTurn miroir (texte vide) → idem ;
  deadline courte (via `run_silent_agent_with_deadline`) → `qa_verdict_cut_off =
  Some(Deadline)` avec ses steps ; max-steps → `Some(MaxSteps)`. Plus **deux
  contrôles négatifs** : un tour qui a posté sa revue puis est coupé ne pose
  rien ; un tour de callback **non-build** (`run_claude_pilot`) ne pose rien.
- **V3 — le filet poste sur le motif neuf.** Exactement un POST, cible = la
  cible estampillée, corps ouvrant sur `DEADLINE_VERDICT_LINE`, nommant la borne
  franchie et les steps, ne contenant ni `pass` ni `block[*]`, **et relu par
  `parse_verdict`** → `Hold("review")`. Plus : les quatre abstentions héritées
  (`no_metadata`, `metadata_unreadable`, `no_target_stamp`, `target_unreadable`)
  valent aussi pour ce motif, **chacune neutralisée séparément** — une
  conjonction de termes fail-safe n'est pas prouvée en les neutralisant tous
  d'un coup (leçon mika#2277).
- **V4 — `cause` est sur la ligne, et les lignes mika#2368 n'ont pas bougé.** Le
  corps `CutOffByDeadline` reste **octet pour octet** identique (le test AC5c
  existant doit passer sans modification) ; une ligne du motif mika#2368 ne porte
  toujours aucun `cause`.
- **V5 — le classificateur d'U2, aux quatre bornes.** `classify_undelivered_verdict`
  rend les quatre valeurs sur les quatre formes de `metadata`, **chaque terme
  muté et vu rouge** (leçon mika#2420 : une assertion de non-vacuité est
  insensible aux termes qu'un autre absorbe en aval). Y compris le bras
  `never_attempted`, sans lequel une ligne jamais tentée se lirait comme une
  famine.
- **V6 — le balayage sélectionne et refuse.** Callback de build non livré
  au-delà de la fenêtre → une alerte avec la bonne `cause` ; `delivered` → rien ;
  dans la fenêtre → rien ; label non-build → rien ; **sans estampille → une
  alerte quand même**, `target=unresolved` (c'est la propriété qui rend ce
  travail non silencieux sur le dispatch mesuré). Dédup : deux passes → une
  ligne ; changement de cause → une seconde ligne.
- **V7 — U3 compte, et borne son écriture.** Un refus `AgentBusy` sur un callback
  de build incrémente le compteur et pose l'instant ; un second refus incrémente
  sans réécrire l'instant (NULL-only) ; un refus sur un callback **pilote** n'écrit
  **rien**.
- **V8 — les trois paliers des trois clés**, et le contrôle négatif du
  kill-switch : désarmé, le balayage n'écrit ni WARN ni ligne d'audit.
- **V9 — les scans de source** de la section *Fire-Disposition*, avec leur
  contrôle de bonne foi (un second écrivain planté fait rougir ; le scan est vu
  rouge avant que le code existe, par anti-vacuité).

**Ce qui n'est PAS testable ici, écrit plutôt que découvert :** que la contention
mika-qa produise réellement un `AgentBusy` sur un callback de build — cela exige
deux tours concurrents sur un agent réel — et que GitHub accepte le POST. Le
contrat côté mika est *le motif est choisi, le corps est juste, le POST est
émis une fois*, et V2/V3 l'attestent déterministiquement. La moitié
comportementale est la sonde S2.

Plus : `cargo fmt`, `cargo clippy` sans avertissement neuf, `cargo test`,
`make verify-bundled-skills` (aucun skill touché, donc non-régression).

## Surfaces opérateur

```bash
# 1. Un build vert a-t-il laissé une PR muette ? (l'alerte d'U2)
grep qa_build_verdict_undelivered "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{task_id, target, cause, age_secs, deferrals, attempts}'

# 2. Le filet a-t-il posté sur un tour COUPÉ ? (le motif neuf d'U1)
grep qa_callback_verdict "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.cause != null) | {cause, repo, pr, steps_completed, outcome}'

# 3. CONTRÔLE NÉGATIF — le filet mika#2276 ne doit PAS se mettre à firer
grep qa_deadline_verdict "$MIKA_SPIRIT_LOG_FILE" | jq 'select(.outcome == "posted")'

# 4. La population mika#2368 historique, inchangée et toujours soustractible
grep qa_callback_verdict "$MIKA_SPIRIT_LOG_FILE" | jq 'select(.cause == null)'

# 5. Le trou de l'estampille — à lire AVANT toute conclusion (voir Halte 1)
grep qa_review_pr_target_unresolved "$MIKA_SPIRIT_LOG_FILE" | jq -c '{reason, trace_id}'
```

```sql
-- La population que le suivi « poster sur quarantaine » devra dimensionner
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'qa_build_verdict_undelivered' GROUP BY 1 ORDER BY 2 DESC;

-- Les deux motifs du filet callback, comptables séparément
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'qa_callback_verdict' GROUP BY 1;
```

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `qa_build_verdict_undelivered` | WARN | **non vide, faible** | chaque ligne est un build vert dont le verdict attend ; `cause` dit lequel des trois chemins |
| `qa_callback_verdict` avec `cause` | WARN | **non vide, faible** | chaque ligne est une PR qu'un tour coupé aurait laissée muette |
| `qa_callback_verdict` sans `cause` | WARN | inchangé | la population mika#2368, intacte |
| `qa_deadline_verdict outcome=posted` | WARN | **vide** | contrôle négatif de mika#2355 — ce travail ne doit pas y ajouter une ligne |
| `qa_build_verdict_alert_disabled` | INFO | vide hors intervention | l'instrument est désarmé, et il le dit |
| `qa_build_verdict_alert_ledger_unreadable` | WARN | **vide** | fail-open : l'alerte est partie quand même, la dédup ne tient plus |

## Sondes post-déploiement — **gestes opérateur, jamais du pilote**

La base et les journaux de production ne sont pas lisibles depuis le bac à sable
de dispatch : ces mesures sont structurellement post-déploiement.

**S0 — établir la cause de #2458 AVANT de conclure quoi que ce soit.** Lire la
surface 5 sur la fenêtre du 2026-09-24. Si `qa_review_pr_target_unresolved
reason=not_a_pr_event` porte le dispatch de #2458, la perte vient du **trou de
l'estampille** et non de P1/P2/P3 — ce travail rend alors l'alerte (la population
d'U2 est le callback, pas l'estampille) mais **pas** le verdict, et le suivi
nommé plus bas devient prioritaire.

**S1 — l'alerte mord (48 h).** `qa_build_verdict_undelivered` non vide, et la
distribution de `cause` dit laquelle des trois populations porte le défaut. C'est
la mesure qui n'existait pas : avant ce travail, P1 était **invisible par
construction**.

**S2 — le motif neuf d'U1 mord (30 j).** Au moins une ligne `qa_callback_verdict`
portant un `cause` de coupure, avec sa PR effectivement porteuse d'un
`hold[review]` du moteur.

**S3 — non-régression, deux sens.** La surface 3 reste **vide** (le filet
mika#2276 n'a pas été détourné) et la surface 4 continue de produire les lignes
mika#2368 sans `cause`.

**S4 — le symptôme (30 j).** Aucune PR de la boucle ne reste `REVIEW_REQUIRED`
sans qu'une ligne nomme la raison. C'est la formulation exacte de l'AC.

### Les cinq haltes

**Halte 1 — S1 est vide alors qu'une PR est visiblement muette.** Ne pas élargir
le prédicat par réflexe. Trois lectures, dans cet ordre : (a) la surface 5 —
c'est le trou de l'estampille, autre défaut, autre remède ; (b) le binaire servi
porte-t-il le correctif ? `cat ~/.mika/skills/.manifest-writer` et la version de
mika-spirit — classe mika#2340, *une ligne absente ne prouve rien tant qu'on n'a
pas établi que le binaire qui tourne sait l'écrire* ; (c) `qa_build_verdict_alert_disabled`
— l'instrument est désarmé.

**Halte 2 — S1 porte du trafic nominal** (plusieurs par heure, PR différentes).
Le seuil est sous la latence de livraison saine. **Monter
`MIKA_QA_BUILD_VERDICT_ALERT_AGE_SECS` avec la distribution**, jamais désarmer
l'alerte : c'est la mesure qui est fausse, pas l'instrument. Et noter la
distribution — c'est la précondition du suivi « poster sur quarantaine ».

**Halte 3 — la surface 3 se met à firer.** Une population a été détournée vers
`qa_deadline_verdict`. **Désarmer par revert avant tout diagnostic** : la sonde
de contrôle négatif de mika#2355 est cassée, donc deux populations ne sont plus
soustractibles, et c'est la propriété dont dépendent toutes les autres lectures.

**Halte 4 — une PR reçoit un `hold[review]` du moteur alors que son vrai verdict
arrivait.** `MIKA_QA_CALLBACK_VERDICT_NET=0` **d'abord**, diagnostic ensuite. Le
prédicat d'U1 exige un tour **coupé** ; un faux positif signifie que le signal
est posé sur un chemin qui n'est pas une coupure, et aucun réglage de seuil ne le
corrige.

**Halte 5 — `cause = "never_attempted"` domine.** Ce n'est pas la contention :
c'est que rien ne tente de livrer ces lignes. Établir si
`dispatch_undelivered_callbacks` tourne (`cli_mode`, fenêtre `since`) **avant**
de toucher à l'attribution.

## Ce que ce travail n'achète PAS

**Il ne réduit pas la contention mika-qa** — le ticket la met explicitement hors
périmètre. P1 reste une famine ; ce qui change est qu'elle est **nommée, datée et
comptée** au lieu d'être indistinguable d'un callback perdu.

**Il ne poste rien pour P1/P2.** Une PR dont le verdict est en file reste
`REVIEW_REQUIRED` — avec, désormais, une ligne qui dit pourquoi. Le POST est un
suivi conditionné à une mesure que ce travail produit.

**Il ne referme pas le trou de l'estampille.** Une revue QA dispatchée en texte
libre alerte (U2) mais ne peut pas recevoir de verdict du moteur (U1 et le filet
mika#2368 exigent la cible estampillée).

**Aucun compteur global, aucun tableau de bord.** Les seuls instruments sont les
greps et les requêtes ci-dessus, et **leur silence ne prouve rien tant que
personne ne les exécute** — limite que mika#2290 a déjà dû écrire pour sa propre
sonde.

**Les lignes historiques ne sont pas rétro-attribuées.** #2458 n'aura jamais son
alerte : les compteurs d'U3 n'existaient pas quand elle a starvé, et fabriquer
une ligne d'audit datée d'un événement qu'on n'a pas observé serait l'inverse de
tout ce que ce travail défend. La sonde est la **prochaine** occurrence.

## Definition of Done

1. `run_loop` pose le signal de verdict-dû-non-posté aux **quatre** sorties, pas
   deux ; `SilentTurnOutcome` porte le fait ; les bras `MaxStepsExceeded` /
   `DeadlineExceeded` de `run_silent_agent` ne le jettent plus.
2. `post_callback_verdict_net` poste un `hold[review]` sur le motif de coupure,
   avec un corps qui nomme la borne franchie, sous `qa_callback_verdict` +
   `cause`, sans toucher une ligne de la population mika#2368 ni de mika#2276.
3. Un balayage périodique émet `qa_build_verdict_undelivered` (WARN + ligne
   d'audit, SOLE WRITER, dédupliqué 24 h) pour tout callback de build non livré
   au-delà de la fenêtre, **en attribuant** la perte à l'une des quatre causes.
4. Le refus `AgentBusy` sur un callback de build est compté et daté sur la ligne.
5. Trois réglages `Settings` + env, forme maison à trois paliers, déclarés dans
   `.env.example` ; aucune valeur existante modifiée.
6. Les trois scans de source de *Fire-Disposition*, allowlists **vides**, avec
   leurs assertions anti-vacuité et leurs contrôles de bonne foi.
7. V1–V9 verts ; `cargo fmt`, `cargo clippy`, `cargo test` propres ; le test
   AC5c de mika#2368 passe **sans modification**.
8. `CLAUDE.md` racine (entrée opérateur : taxonomie, surfaces, régimes attendus,
   cinq haltes) et `crates/mika-agent/CLAUDE.md` (§ *Verdict Net* étendu, §
   *Unified Task Engine* pour le balayage) à jour.
9. Corps de PR nommant explicitement ce qui n'est **pas** livré : le POST pour
   P1/P2, la contention, le trou de l'estampille — chacun avec sa précondition.

## Acceptance criteria

Transcrits du ticket, et complétés là où le ticket n'énumère qu'un critère.

1. **Un `build_mika` qui réussit produit, dans les 10 minutes, SOIT un verdict QA
   posté sur la PR, SOIT une alerte nommée grep-visible
   (`qa_build_verdict_undelivered`) attribuant l'échec du post au chemin.**
   Borne effective : fenêtre (540 s) + un intervalle de balayage (60 s) = 600 s.
2. **Aucun build vert ne laisse une PR `REVIEW_REQUIRED` en silence** : chacune
   des populations P1, P2 et P3 produit au moins une ligne nommée.
3. **P3 est fermé côté verdict** : un tour de callback de build coupé par sa
   deadline ou par sa limite de steps, qui devait un verdict et n'en a posté
   aucun, produit un `hold[review]` du moteur sur la PR estampillée.
4. **L'attribution est positive, jamais inférée d'une absence** : un
   `AgentBusy` sur un callback de build est compté et daté sur la ligne, et
   `cause = agent_busy_starvation` repose sur ce compteur.
5. **Les populations restent soustractibles** : `qa_deadline_verdict` (mika#2276)
   est inchangé et sa sonde de contrôle négatif reste valide ;
   `qa_callback_verdict` sans `cause` désigne toujours exactement la population
   mika#2368 ; le corps du motif mika#2276 reste identique octet pour octet.
6. **Fail-safe partout où le signal manque** : absence d'estampille, metadata
   illisible, cible non résoluble, token absent, filet désarmé ⇒ zéro POST et une
   ligne nommant l'abstention. Une estampille absente **n'empêche pas** l'alerte.
7. **Zéro churn** : zéro alerte ⇒ zéro ligne ; une ligne d'audit par
   `(task, cause)` par 24 h ; le chemin nominal (verdict posté, ou verdict non
   dû) n'écrit rien et ne paie ni lecture de metadata ni résolution de token.
8. **Réversible sans redéploiement** : `MIKA_QA_BUILD_VERDICT_ALERT=0` désarme
   l'alerte, `MIKA_QA_CALLBACK_VERDICT_NET=0` désarme le POST, et désarmé chacun
   **le dit**.
9. **Aucun réglage de dispatch, de concurrence, de budget d'attente ou
   d'enveloppe LLM n'est modifié** ; aucune migration de schéma.

## Hors périmètre, délibérément

- **La contention mika-qa** et le budget d'attente #2163 — mis hors périmètre par
  le ticket lui-même. Ce travail la **mesure** (`cause = agent_busy_starvation`,
  `verdict_delivery_deferrals`), il ne la réduit pas.
- **Poster sur P1/P2.** Refus raisonné ci-dessus. **Suivi**, périmètre étroit :
  poster le `hold[review]` sur les seules lignes **quarantinées** (budget
  mika#2179 épuisé ⇒ réessai horaire ⇒ le verdict est effectivement perdu, et
  rien n'est plus en course). **Précondition : que S1 montre cette population non
  vide**, faute de quoi ce serait une garde sans population.
- **La file de livraison des callbacks.** `dispatch_undelivered_callbacks` est une
  **loterie, pas une file** : chaque ligne est `tokio::spawn`ée et se dispute le
  verrou par `try_lock`, sans équité ni vieillissement — là où `/message` a une
  file bornée avec un drain worker (mika#1870) et `/a2a` une ligne d'attente FIFO
  bornée (mika#2163), ce chemin a gardé le motif d'avant. Défaut réel, trouvé en
  chemin, **rayon d'action très différent** (élargir cette population change qui
  marque une tâche `failed` — périmètre que mika#2272 a déjà borné par écrit).
  **Suivi**, précondition : que S1 montre `agent_busy_starvation` dominant.
- **Le trou de l'estampille.** Élargir `parse_pr_target` au texte libre est
  refusé (seconde grammaire de fil, classe mika#2158 ; et elle ne résoudrait pas
  le dépôt). **Suivi**, et le remède est côté **appelant** — faire porter au
  dispatch opérateur un message d'origine de forme événement-PR — pas côté
  parseur. Précondition : que S0 montre cette population non vide.
- **La cause des échecs de transport** de P2 (openrouter, taille de requête) —
  mika#2179 borne et instrumente, ce travail attribue ; aucun des deux ne la fait
  disparaître.
- **`qa_review_reconcile`** (mika#2334) : sa population est disjointe par
  construction (il exige l'absence de revue de `mika-platform-qa`, faux dès la
  première revue partielle). Aucune ligne de ce travail ne le touche.
- **Les cinq autres flux `long_running`** : le discriminant est
  `qa_build_callback::BUILD_CALLBACK_LABEL`, dont l'égalité stricte les exclut
  chacun (mika#2355 AC4b).
