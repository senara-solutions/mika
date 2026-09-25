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

### Le défaut est que le filet n'a de population que sur deux sorties sur six

`post_callback_verdict_net` (`task_engine/dispatcher.rs:836`) lit
`SilentTurnOutcome.qa_verdict_unmet`, que `run_silent_agent` ne rend que sur la
branche `Ok`. Et `run_loop` ne **pose** ce drapeau qu'à deux sites
(`agent_loop/mod.rs:3959` et `:4155`) — alors qu'il a **six** sorties.

#### Les six sorties de `run_loop`, et qui les couvre

Numéros relevés à HEAD `bb198d2e`, et **à re-vérifier par ancre avant d'éditer** —
ils ont déjà dérivé une fois entre deux passes de ce plan (cf. *Re-localiser les
six sites*).

| ligne | sortie | atteignable en `Silent` ? | signal posé aujourd'hui |
|---|---|---|---|
| `3971` | `Done` — texte non vide | oui | ✅ mika#2368 (site 1/2) |
| `4167` | `Done` — texte vide, **`Silent` uniquement** | oui | ✅ mika#2368 (site 2/2) |
| `4214` | `Done` — texte vide **après follow-up** | **non** | — (hors population, voir ci-dessous) |
| `4356` | `Done` — **« Force EndTurn » après `send_message`** | **oui** | ❌ **trou de mika#2368** |
| `1492` | `DeadlineExceeded` | oui | ❌ périmètre mika#2515 |
| `4373` | `MaxStepsExceeded` | oui | ❌ périmètre mika#2515 |

`4214` sort de la population **par construction, pas par chance** :
`LoopMode::follow_up_on_empty()` rend `true` pour `Conversation | Team`
seulement, et un tour de callback est `Silent`. Le commentaire du site `4167` le
dit déjà (« Silent-mode-only exit (`!follow_up_on_empty`) »), et
`test_loop_mode_silent_properties` l'épingle. Une sortie exclue **pour une
raison lisible** n'est pas une sortie oubliée — mais l'exclusion doit être
écrite, sans quoi la prochaine lecture la recompte.

#### Le trou de mika#2368 découvert au grooming — `4356`

Le site `4356` est un `return Ok(LoopResult::Done)` pris depuis la branche
`LlmStopReason::ToolUse`, quand un `send_message` a été livré : le tour est
**conclu** sans que le modèle ait jamais émis d'EndTurn. Il est atteignable en
mode `Silent` (aucun garde de mode ne le protège), donc un tour de callback de
build QA qui envoie un message — « le build a réussi, je note » — au lieu de
poster sa revue **conclut par ce chemin, et le filet mika#2368 ne fire pas**.
C'est très exactement le symptôme du ticket.

Ce n'est pas une inférence : mika#2136 a rencontré ce site et l'a documenté en
propres termes, dans le code, à trois lignes de là — *« This `return` is a
FOURTH exit from `run_loop`, and it does not traverse the EndTurn guard chain at
all: it fires on `stop_reason == ToolUse`, before any of the eleven
post-conditions are evaluated »*. mika#2136 a posé son miroir 3/3 ici ;
**mika#2368 ne l'a pas suivi.** Conséquence de conception : ce site relève du
motif `CallbackConcludedWithoutVerdict` (le tour a **conclu**), pas du motif de
coupure — c'est une **réparation de mika#2368**, pas une population neuve, et
elle est traitée comme telle (aucun `cause` sur la ligne, cf. U1e).

#### Trois façons d'échapper au filet, dont deux lisibles sans reproduire l'incident

**(1) `AgentBusy` — le tour ne tourne jamais, et personne ne le dit.**
`dispatch_resume_agent` (`dispatcher.rs:1003`) prend le verrou d'agent par
`try_lock()` et rend `Err(DispatchError::AgentBusy)` **avant** de créer la
session et avant `run_silent_agent`. Le filet est donc structurellement hors
d'atteinte. Et le refus est **muet par construction**, à deux titres : le site
n'écrit qu'un `debug!` — or le filtre de journal de ce serveur ne collecte en
DEBUG que la cible `mika::llm_debug`, la classe même que mika#2131 a dû
refermer (`stuck_ready_reconcile_skipped`, **0 occurrence** sur 200 Mo de
journal contre 184 pour un `info!` du même module), donc cette ligne **n'existe
pas en production** ; l'appelant
(`engine.rs:1150::dispatch_undelivered_callbacks`) supprime par ailleurs
explicitement le WARN (`if !matches!(e, DispatchError::AgentBusy(_))`), et
`record_callback_delivery_failure`
— la télémétrie mika#2179, ses compteurs, sa quarantaine — n'est appelée que dans
la branche `Err` de `run_silent_agent`, c'est-à-dire **après** que le tour a
tourné. Un `AgentBusy` n'incrémente donc `delivery_attempts` **ni** ne déclenche
la quarantaine : la ligne est re-sélectionnée à chaque balayage de 60 s,
indéfiniment, sans compteur, sans ligne d'audit, sans ligne de journal.
**C'est la signature du contexte de contention que le ticket décrit.**

**(2) Le tour est coupé — trou dans les DEUX filets.**
Le fait est perdu, mais **pas par le même mécanisme sur les deux bras**, et la
distinction décide du câblage d'U1b :

- `LoopResult::DeadlineExceeded` (`mod.rs:6605`) termine par
  `return Ok(SilentTurnOutcome::default())` — le fait est **explicitement jeté**,
  avec un commentaire mika#2368 qui le motive par « c'est le périmètre de l'autre
  motif (`CutOffByDeadline`, mika#2276) ». Or ce motif est câblé au call-site
  **webhook** : pour un callback, ce renvoi ne mène nulle part.
- `LoopResult::MaxStepsExceeded` (bras à `mod.rs:6531`) ne rend `default()` que
  dans sa **sous-branche** « deadline trop proche pour tenter la continuation »
  (`6560`) ; le chemin nominal **tombe à travers** vers la construction finale
  (`mod.rs:6650`), qui **lit** le drapeau. Le fait n'y est donc pas jeté par le
  bras : il est absent parce que `run_loop` ne l'a jamais posé.

  *Ce numéro-ci a été trouvé faux au relevé* : les versions antérieures de ce
  plan écrivaient `6451`, soit **80 lignes** avant le bras réel, au milieu d'une
  autre fonction. Troisième repère fautif de ce document — et, à la différence
  des deux premiers, **ce n'est pas une dérive mais une erreur d'origine** (cf.
  *Re-localiser les six sites*). Elle est signalée ici plutôt qu'effacée parce
  qu'elle fonde la table d'ancres de `run_silent_agent` plus bas : la moitié
  `run_loop` de ce plan avait ses ancres, la moitié `run_silent_agent` n'avait
  que des numéros nus, et c'est exactement là que la faute a survécu à six
  passes de relecture.

**Trois sites à toucher, pas deux** — et deux natures différentes : un renvoi
prématuré à réparer, et une pose manquante en amont. Un correctif qui ne
traiterait que les `default()` laisserait le chemin max-steps nominal muet.

Un précédent de forme existe à 900 lignes de là, et l'asymétrie est le défaut :
`AgentOutput` porte `deadline_exceeded: Option<DeadlineOverrun>`, commenté
« mika#2276 M2: the one place that says "cut off, not concluded" », et
`DeadlineOverrun` porte déjà exactement `steps_completed`. **`SilentTurnOutcome`
n'a jamais reçu son équivalent** — c'est le trou, nommé par sa symétrie.

Et le filet mika#2276, qui couvre pourtant exactement le
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

### Taxonomie — six populations, deux couvertes

| # | population | tour tourne ? | filet fire ? | télémétrie aujourd'hui |
|---|---|---|---|---|
| P0 | tour **conclu** par le « Force EndTurn » de `send_message` (`mod.rs:4356`) | **oui** | **non** (site jamais instrumenté) | rien — indistinguable d'un tour sain |
| P1 | `AgentBusy` : le verrou d'agent n'est jamais gagné | **non** | non (exige `Ok`) | **rien du tout** (`debug!` non collecté, WARN supprimé, compteur mika#2179 non incrémenté) |
| P2 | `run_silent_agent` rend `Err` | partiellement | non (exige `Ok`) | mika#2179 (`callback_delivery_failed`, quarantaine) — ne dit rien d'une PR muette |
| P3 | tour coupé (`DeadlineExceeded` / `MaxStepsExceeded`) | **oui** | **non** (drapeau posé aux seules sorties EndTurn) | `agent deadline exceeded` — ne dit rien d'une PR muette |
| P4 | tour conclu muet, filet s'abstient | oui | abstention **nommée** | `qa_callback_verdict outcome=no_*` ✅ |
| P5 | tour conclu muet, filet poste | oui | oui | `qa_callback_verdict outcome=posted` ✅ |

P0, P1, P2 et P3 laissent un build vert et une PR muette **sans aucune ligne
attribuant la perte**. C'est le périmètre.

**P0 est la plus insidieuse des quatre** et mérite d'être lue à part : le tour a
tourné normalement, il a appelé un outil, il a même parlé à quelqu'un — rien,
dans aucun journal, ne le distingue d'un tour qui a fait son travail. P1 laisse
au moins une absence de compteur, P2 une ligne mika#2179, P3 un
`agent deadline exceeded`. P0 ne laisse **rien**. C'est aussi la seule des
quatre qui se referme sans nouvelle machinerie : un `if let` au site, identique
aux deux que mika#2368 a déjà posés.

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

- **U1 — P0 et P3 : les sorties non instrumentées posent leur signal.** Le filet
  mika#2368 existe, la cible est déjà estampillée, le registre anti-double-post
  et la classification du 422 sont déjà là. Ce qui manque est le **signal** :
  `run_loop` le pose à deux sorties sur six. Armé, sans nouveau levier (il roule
  sur `MIKA_QA_CALLBACK_VERDICT_NET`). Deux moitiés de natures différentes —
  **P0 est une réparation de mika#2368** (le tour a conclu, motif et ligne
  inchangés, aucun `cause`), **P3 est le motif neuf** (le tour a été coupé). Les
  garder distinctes est ce qui laisse la population mika#2368 soustractible.
- **U2 — P1/P2 : une alerte nommée et bornée.** Un balayage périodique sur les
  callbacks de build non livrés au-delà d'une fenêtre, qui émet
  `qa_build_verdict_undelivered` **en attribuant la perte au chemin**.
  Détection seule ; le POST est un suivi conditionné (voir *Hors périmètre*).
- **U3 — rendre l'attribution positive plutôt qu'inférée.** Aujourd'hui la
  signature de P1 est « `delivery_attempts` absent », c'est-à-dire une
  **absence** — et la règle de la maison est qu'une absence n'est pas une
  preuve. Le refus `AgentBusy` sur un callback de build devient **compté**.

## U1 — Les sorties non instrumentées posent leur signal

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

`run_loop` a **trois** appelants : `run_agent` (`:5319`, `Conversation`),
`run_silent_agent` (`:6400`, `Silent`) et `run_team_agent` (`:6975`, `Team`).
Seul le second passe autre chose que `None`, et l'arité étant conservée, les deux
autres sites changent d'un type dans une position déjà occupée par `None` —
aucun comportement conversationnel ni d'équipe n'est touché. C'est ce qui rend ce
changement de signature sûr, et c'est pourquoi il est préféré à un second
paramètre : ajouter une arité ferait porter à trois appelants le coût d'un besoin
qui n'en concerne qu'un.

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

### U1e — P0 : le troisième miroir de mika#2368

Au site `mod.rs:4356`, **le même `if let` que les deux sites mika#2368
existants**, appelant `verdict_unmet_after_retry` — et non le prédicat de
coupure :

```rust
// mika#2515 — chemin de sortie 3/3. Ce `return` conclut le tour depuis la
// branche `ToolUse`, sans traverser la chaîne de gardes EndTurn : mika#2136 l'a
// nommé ici même comme sa troisième glace, mika#2368 ne l'a pas suivi. Un tour
// de callback qui envoie un message au lieu de poster sa revue conclut par ici,
// et le filet restait aveugle sur exactement ce chemin.
if let Some(flag) = qa_verdict_unmet
    && crate::qa_build_callback::verdict_unmet_after_retry(
        qa_verdict_due, &intent_guard_retries, &all_tool_summaries,
    )
{
    flag.unmet_after_retry.store(true, Ordering::Relaxed);
}
```

**Le terme de budget de garde est conservé ici**, à l'inverse d'U1a, et c'est la
différence qui compte : ce site est un **EndTurn forcé**, donc la garde
`qa_build_callback_verdict` a bien eu son occasion de firer et de dépenser son
budget au tour précédent. Sur un tour coupé elle ne l'a pas eue — c'est toute la
raison pour laquelle le prédicat de coupure laisse tomber ce terme. Employer le
mauvais prédicat au mauvais site produirait, dans un sens, un filet insatisfiable
et, dans l'autre, un `hold[review]` sur un tour que la garde n'a jamais interrogé.

Conséquence sur la ligne : **aucun `cause`**. P0 est une conclusion muette, donc
la population mika#2368 stricto sensu ; lui donner un `cause` la sortirait de la
soustraction `select(.cause == null)` que les sondes de mika#2368 emploient déjà,
et ferait passer une **réparation** pour une population neuve.

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

Au site du `try_lock()` échoué de `dispatch_resume_agent`
(`dispatcher.rs:1003`), **et seulement pour un callback de build** (un
`task.label ==` sur le chemin nominal, gratuit). Le **second** site `AgentBusy`
du fichier — `dispatch_skill_by_name` (`:724`) — est hors population et le
reste : il ne livre pas de callback, donc aucun verdict n'y attend. Les deux
sites sont nommés ici parce qu'un implémenteur qui grep `AgentBusy` en trouve
deux et doit savoir lequel, sans avoir à trancher lui-même.

**Et il ne peut PAS les discriminer par le grep évident**, ce qui est la raison
d'être de ce paragraphe : les deux lignes de sortie sont **littéralement
identiques** — `return Err(DispatchError::AgentBusy(task.id.clone()));` aux deux
endroits, au caractère près. Seul le `debug!` qui les précède les sépare, et
c'est donc lui qui fait office d'ancre : `agent busy, deferring resume_agent`
pour la cible d'U3 (`1002`, écart −1), `agent busy, deferring skill run` pour le
site hors population (`723`). Exactement le piège déjà relevé pour les deux
sites de pose mika#2368 dans `agent_loop/mod.rs`, et la même parade : ancrer sur
ce qui distingue, jamais sur ce qui se répète. Se tromper de site ici ne casse
rien de visible — le compteur se poserait sur des dispatches de skill, la
`cause = agent_busy_starvation` d'U2 ne trouverait jamais son compteur, et
l'attribution redeviendrait l'inférence-par-absence qu'U3 existe pour retirer.

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

## Re-localiser les six sites — l'ancre, jamais le numéro

Ce plan désigne ses sites par numéro de ligne parce que c'est lisible à la
lecture. **Un numéro de ligne pourrit en silence** — c'est très exactement ce que
mika#2201 refuse pour son TSV de jetons (« jamais un numéro de ligne, qui pourrit
en silence ») — et `agent_loop/mod.rs` fait plus de dix mille lignes, donc un
implémenteur qui arrive après un autre correctif trouvera du code étranger à
`4356`. Chaque ancre ci-dessous a été **relevée et vérifiée unique** dans
`agent_loop/mod.rs`, au dernier relevé à HEAD `bb198d2e` ; l'implémenteur les
re-vérifie avant d'éditer plutôt que de se fier aux numéros. **Ce plan a déjà vu
ses numéros pourrir une fois entre deux passes de grooming** — la suite de cette
section est le récit de cette péremption, gardé parce qu'il vaut plus que les
numéros qu'il corrige.

**Deux erreurs opposées se sont succédé ici, et les deux sont conservées plutôt
qu'effacées, parce que chacune attend le prochain lecteur.**

*La première* : une version antérieure annonçait une dérive qui n'existait pas.
Elle avait comparé les numéros des **sorties** à ceux des **ancres** — les
commentaires et les logs qui annoncent ces sorties — et lu l'écart entre deux
objets distincts comme le déplacement d'un seul. L'arithmétique suffisait à la
trancher sans rouvrir le fichier : une dérive décale tout dans le **même sens**,
or l'un des six écarts est positif quand les cinq autres sont négatifs.

*La seconde, à l'inverse* : la version suivante a corrigé en affirmant les
numéros « re-relevés, inchangés ». **Cette affirmation est morte.** Re-relevé à
HEAD `bb198d2e`, les six `Ok(LoopResult::…)` sont à `1492`, `3971`, `4167`,
`4214`, `4356`, `4373` — **+59 sur chacun**, dérive uniforme, donc réelle cette
fois. Et la dérive est **locale à `agent_loop/mod.rs`** : les deux `AgentBusy`
(`724`, `1003`) et `post_callback_verdict_net` (`824`, lecture du drapeau `836`)
dans `dispatcher.rs` sont, eux, inchangés. Une dérive ne se propage pas d'un
fichier à l'autre ; il n'y a donc pas un décalage global à appliquer de tête,
mais un relevé à refaire par fichier.

*La troisième n'est pas une dérive et c'est ce qui la rend instructive* : le
`6451` corrigé plus haut n'a jamais désigné le bras `MaxStepsExceeded` de
`run_silent_inner`, qui est à `6531` — 80 lignes plus loin, et le `6451` tombait
au milieu d'une **autre** fonction. Ce n'est donc pas un numéro qui a vieilli,
c'est un numéro qui était faux à l'écriture, et aucune re-lecture des six
premières passes ne l'a vu parce qu'il vivait dans la seule moitié du plan qui
n'avait pas d'ancres. Une dérive se détecte par l'uniformité de son décalage ;
une erreur de relevé ne se détecte que par confrontation au fichier.

**Relevé re-vérifié à HEAD `757cde95`** : les six sorties de `run_loop`, les
quatre renvois de `run_silent_agent`, les deux `AgentBusy` et
`post_callback_verdict_net` sont **tous inchangés** par rapport au relevé
`bb198d2e`. Cette stabilité est **expliquée, pas constatée** — et c'est la seule
forme sous laquelle elle vaut d'être écrite, la version antérieure de ce plan
ayant déjà vu mourir un « re-relevés, inchangés » posé sans raison :
`git diff --stat bb198d2e..757cde95` ne touche que `Cargo.toml`, `Cargo.lock` et
ce plan lui-même, donc **aucun commit de la fenêtre n'a modifié
`crates/mika-agent/src/`**. Le jour où un commit y touche, cette phrase redevient
caduque et la table d'ancres reprend seule la charge — ce qui est son rôle.

Ce que la succession des trois erreurs enseigne, et qui vaut mieux que l'une ou
l'autre : **les écarts ancre↔sortie sont stables, les numéros absolus ne le sont
pas.** Les six écarts relevés à `bb198d2e` (−2, −16, **+4**, −5, −21, −2) sont
identiques à ceux relevés à la conception. C'est la colonne « écart » de la table
ci-dessous qui survit aux réécritures du fichier, et la colonne « sortie » qui
pourrit — ce qui est exactement pourquoi la table fait foi par ses **ancres** et
non par ses numéros.

**Le piège que cette dérive a armé, à connaître avant d'éditer quoi que ce soit :
un numéro périmé ne désigne pas du vide, il désigne autre chose.** La sortie que
les versions antérieures de ce plan appelaient `4155` porte aujourd'hui, à ce
numéro exact, **l'un des deux sites de pose de signal de mika#2368**
(`if let Some(flag) = qa_verdict_unmet`). Un implémenteur qui ouvrirait `4155` en
croyant y trouver une sortie à instrumenter y trouverait du code mika#2368
plausible, déjà armé, et pourrait le prendre pour sa cible ou le croire
redondant. C'est la forme la plus coûteuse de la péremption : non pas une adresse
qui ne résout plus, mais une adresse qui résout vers un voisin crédible.

**Ce que le piège coûte à l'implémenteur, et c'est la raison d'être de ce
paragraphe :** `grep -n '<ancre>'` ne rend **pas** la ligne du `return`, il rend
celle de son commentaire — deux lignes plus haut pour la deadline, vingt et une
pour le Force EndTurn, et quatre plus **bas** pour la sortie Silent, dont le
commentaire suit son `return` au lieu de le précéder. On ancre pour **retrouver
le site**, puis on lit le voisinage pour trouver la sortie ; on ne convertit
jamais une ligne d'ancre en adresse d'édition.

Autres relevés au même HEAD, **par fichier** puisque la dérive ne se propage pas :
dans `agent_loop/mod.rs`, les deux sites de pose mika#2368 (`3959`, `4155`) et le
commentaire mika#2136 (`4289`) ; dans `dispatcher.rs`, **inchangés**, les deux
`AgentBusy` (`724`, `1003`) et `post_callback_verdict_net` (`824`, sa lecture du
drapeau `836`).

Et, dans `run_silent_agent`, **quatre** sites de renvoi de `SilentTurnOutcome` —
là où les versions antérieures de ce plan n'en comptaient que trois. Le
quatrième est traité dans sa propre section ci-dessous (*Le quatrième renvoi*) ;
il est nommé ici parce qu'un implémenteur qui grep `SilentTurnOutcome::default()`
en trouve **trois** et doit savoir que le compte attendu est quatre avec la
construction finale, sans avoir à trancher lui-même lequel manque.

**Ces quatre-là ont leurs ancres au même titre que les six de `run_loop`**, et
ce n'était pas le cas des versions antérieures : elles les désignaient par
numéro nu tout en prêchant l'ancre pour la fonction voisine. L'incohérence n'est
pas cosmétique — c'est très exactement dans cette moitié non ancrée que le
troisième repère fautif a survécu à six passes (le `6451` corrigé plus haut,
faux d'origine et non périmé). La DoD 1b fait de
`run_silent_agent` une moitié aussi critique que `run_loop` ; elle a droit au
même invariant.

| renvoi | écart | site | ancre `grep` (unique) |
|---|---|---|---|
| `6443` | −2 | prélude de deadline (**exclu**, mika#2515-a) | `mika#2368 : le tour n'a pas eu lieu` |
| `6560` | −7 | `default()` — sous-branche « deadline trop proche » | `silent agent max-steps exceeded but deadline too close` |
| `6605` | −4 | `default()` — bras `DeadlineExceeded` | `c'est le périmètre de l'autre motif` |
| `6650` | 0 | construction finale (**lit** le drapeau) | `Ok(SilentTurnOutcome {` |

Le quatrième est son propre repère : `Ok(SilentTurnOutcome {` est unique dans le
fichier, donc écart nul — l'ancre **est** le site. C'est le seul des dix repères
de ce plan dont la colonne « écart » ne sert à rien, et le dire évite qu'un
lecteur cherche un décalage qui n'existe pas.

**Deux pièges de plus, de la même famille que les deux déjà relevés pour
`run_loop`.** `max-steps exceeded but deadline too close for continuation`
**seul** rend **trois** résultats — `run_agent` (`5486`), `run_silent_agent`
(`6553`) et `run_team_agent` (`7142`), trois fonctions qui portent la même
phrase à un préfixe près — d'où le `silent agent ` dans l'ancre ci-dessus. Et
`LoopResult::MaxStepsExceeded {` en rend **quatre** (`4373`, `5469`, `6531`,
`7129`), donc le bras de `run_silent_inner` n'est **pas** ancrable par ce motif :
c'est son voisinage `silent agent` qui le discrimine, jamais le nom de la
variante. Un implémenteur qui ancrerait sur la variante éditerait la boucle
d'équipe en croyant éditer la boucle silencieuse — et les deux compilent.

Les numéros de tout ce plan restent à lire comme **des repères de lecture, jamais
comme des adresses** : la table ci-dessous est ce qui fait foi.

**Un dernier écart de vocabulaire, à connaître avant d'éditer le site `4356` :**
le commentaire mika#2136 qui s'y trouve l'appelle *« a FOURTH exit from
`run_loop` »* là où ce plan en compte **six**. Les deux sont justes et ne
comptent pas la même chose — mika#2136 numérotait les sorties qu'il avait à
couvrir, ce plan énumère toutes celles du corps de `run_loop`. C'est la
cardinalité **6** que le scan asserte, et elle est relevée, jamais mémorisée.

Les deux colonnes de numéros sont données **séparément** et leur écart est
explicite : c'est ce que la version antérieure avait confondu, et l'écart signé
est ce qui rend la confusion impossible à refaire.

Numéros relevés à HEAD `bb198d2e`. **La colonne « écart » est le seul invariant ;
les deux colonnes de numéros sont un confort de lecture, l'ancre est l'adresse.**

| sortie (`return`) | ancre | écart | sortie | ancre `grep` (unique) |
|---|---|---|---|---|
| `1492` | `1490` | −2 | `DeadlineExceeded` | `agent deadline exceeded — exiting loop gracefully` |
| `3971` | `3955` | −16 | `Done` — texte non vide | `mika#2368 — chemin de sortie 1/2` |
| `4167` | `4171` | **+4** | `Done` — texte vide, `Silent` | `Silent-mode-only exit` |
| `4214` | `4209` | −5 | `Done` — après follow-up (**exclu**) | `agent returned empty text after follow-up` |
| `4356` | `4335` | −21 | `Done` — Force EndTurn (**P0**) | `Force EndTurn — return Done directly` |
| `4373` | `4371` | −2 | `MaxStepsExceeded` | `max_steps, "agent exceeded max tool steps"` |

La ligne `4167` est celle qui porte la démonstration : son ancre **suit** son
`return`. Un implémenteur qui traiterait la colonne « ancre » comme une adresse
éditerait, à ce site précis, du code situé **après** la sortie qu'il visait.

Les six ancres ont été re-vérifiées **uniques** (`grep -c` = 1 chacune) à ce même
HEAD. C'est la propriété qui autorise à les traiter comme des adresses ; elle est
à re-vérifier, pas à supposer, et son coût est une commande.

Deux pièges relevés au passage, qui coûteraient chacun une mauvaise édition.
`agent exceeded max tool steps` **seul** rend trois résultats — un doc-comment de
`attempt_continuation_turn`, le site `4373`, et le `silent agent exceeded max tool
steps` de `run_silent_agent` — d'où le préfixe `max_steps,` dans l'ancre. Et le
`if let Some(flag) = qa_verdict_unmet` des deux sites mika#2368 est
**littéralement identique** aux deux endroits : seuls les commentaires qui les
précèdent les distinguent, ce qui est aussi la raison pour laquelle U1e doit
recopier le **bon** prédicat plutôt que le bloc d'à côté (cf. V2b).

## Le quatrième renvoi — et pourquoi il est exclu d'une AUTRE manière

Trouvé en re-vérifiant ce plan contre le code : `run_silent_agent` a **quatre**
sites de renvoi, et les versions antérieures n'en décidaient que trois. Le
quatrième est le **prélude de deadline** (`6443`, mika#848 F3b) — un
`return Ok(SilentTurnOutcome::default())` pris **avant** `run_loop`, quand la
deadline est déjà dépassée à l'entrée.

**Un plan qui compte trois sites sur quatre reproduit d'un cran le défaut qu'il
ferme.** Tout ce travail part de « `run_loop` pose son signal à deux sorties sur
six » ; laisser un renvoi non décidé dans la fonction qui *lit* ce signal serait
la même faute, un étage plus haut, et elle serait invisible pour exactement la
même raison — aucune décision ne devient fausse, une population devient muette.

Son commentaire mika#2368 y motive le renvoi en propres termes : *« le tour n'a
pas eu lieu. Un tour qui n'a pas conclu ne "conclut pas sans verdict" — le filet
ne s'arme pas ici. »* **Ce raisonnement est juste, et il est exactement aussi
périmable que celui du bras `DeadlineExceeded`** que ce plan répare déjà (`6605`,
dont le commentaire renvoie au motif `CutOffByDeadline` de mika#2276, câblé
webhook, qui ne mène nulle part pour un callback). Dans les deux cas mika#2368 a
écarté une population au motif qu'elle relevait d'un *autre* motif ; dans les
deux cas mika#2515 crée le motif « coupé » et rend l'écart caduc. La symétrie est
le point, et c'est elle qui interdit de laisser ce site sans décision.

**Il n'est pourtant pas armable à son site, et la raison est structurelle.**
`qa_verdict_due` est calculé **dans** `run_loop` (`1472`) depuis
`loaded_skill_names`, lui-même calculé à `6471` — soit **après** le prélude.
L'armer exigerait de remonter la correspondance de skills en amont du contrôle de
deadline, ce qui est à la fois un changement d'ordre des effets et une absurdité
lisible : faire le travail de correspondance précisément après avoir établi qu'il
ne reste plus de temps pour s'en servir.

**Et sa population est quasi certainement vide, sans l'être par construction.**
`run_silent_agent` pose `deadline = Instant::now() + enveloppe` puis
`run_silent_inner` teste `Instant::now() >= deadline` après seulement deux appels
de base (`load_agent_context`, `list_commitments`) : il faudrait que ces deux
appels consomment l'enveloppe **entière** (300 s au défaut de flotte). Improbable,
pas impossible.

**C'est cette nuance qui décide la forme de l'exclusion, et elle interdit la
solution évidente.** Ce site ne peut **pas** entrer dans
`LOOP_EXITS_WITHOUT_SIGNAL`, dont le doc-comment exige « structurellement
inatteignable depuis un tour de callback » — ce qui est vrai de `4214`
(`follow_up_on_empty()` est faux pour `Silent`, propriété *booléenne* et épinglée)
et **faux** d'un site qui dépend d'une course entre une horloge et deux requêtes.
Y déposer une entrée en la déclarant inatteignable écrirait une fausseté **dans la
garde même qui existe pour empêcher les faussetés** — et c'est précisément le
geste que la doctrine mika#2201 refuse (« on déclare, on n'allowliste pas » : une
déclaration doit être vraie, sans quoi elle est une exemption déguisée).

Donc une table **sœur et distincte**, dans le scan d'U1, avec son vocabulaire
propre — l'exclusion y est motivée par l'**indisponibilité du prédicat au site**,
jamais par l'inatteignabilité du site :

```rust
/// Renvois de `run_silent_agent` qui ne lisent délibérément aucun fait de
/// verdict. Vocabulaire DISTINCT de `LOOP_EXITS_WITHOUT_SIGNAL` : là l'exclusion
/// dit « ce site est inatteignable », ici elle dit « ce site n'a pas le prédicat
/// sous la main ». Confondre les deux ferait passer une course d'horloge pour une
/// impossibilité (mika#2515).
const SILENT_RETURNS_WITHOUT_VERDICT_READ: &[(&str, &str)] = &[(
    "prelude deadline check (mika#848 F3b)",
    "`qa_verdict_due` dérive de `loaded_skill_names`, calculé APRÈS ce site : \
     le prédicat n'existe pas encore. Population bornée par le fait que \
     l'enveloppe entière devrait s'écouler dans `load_agent_context` + \
     `list_commitments`. NON inatteignable — suivi mika#2515-a.",
)];
```

Son **assertion auto-nettoyante** ne peut pas être booléenne comme celle de sa
sœur (il n'y a pas de propriété à interroger) : c'est un scan d'**ordre** — le
calcul de `loaded_skill_names` doit rester **après** le prélude dans
`run_silent_inner`. Le jour où quelqu'un remonte la correspondance de skills,
l'exclusion cesse d'être vraie et le test rougit en nommant le site devenu
armable, au lieu de laisser un renvoi muet derrière une raison morte.

**Suivi nommé (mika#2515-a), précondition écrite :** armer ce site en remontant
le prédicat. À n'ouvrir **que** si une mesure montre la population non vide —
`grep 'silent agent deadline exceeded during prelude' "$MIKA_SPIRIT_LOG_FILE"`
croisé avec un `trigger_label = "callback"`. Le WARN existe déjà à ce site, donc
la mesure est disponible **sans rien livrer** : c'est la raison pour laquelle ce
plan peut se contenter de déclarer l'exclusion au lieu de la deviner. Zéro ligne
sur ce grep ⇒ **ne pas ouvrir le suivi**, ce serait un site armé pour une
population qui n'existe pas.

## Unités d'implémentation

**Les numéros de cette table sont des repères de lecture, jamais des adresses.**
C'est la première table qu'un implémenteur ouvre, et c'est donc celle où la
mise en garde doit se trouver plutôt que cent lignes plus haut : les ancres de
*Re-localiser les six sites* et de *U3* font foi, et trois des repères de ce
plan se sont déjà révélés faux — deux par dérive, un par erreur d'origine.

| U | fichier | contenu |
|---|---|---|
| **U1a** | `src/qa_build_callback.rs` | `VerdictSignal`, `CutOffExit` (+ `as_cause()`, `match` sans `_`), `CallbackCutOff`, `verdict_unmet_at_cut_off` |
| **U1b** | `src/agent_loop/mod.rs` | paramètre `run_loop` → `Option<&VerdictSignal>` ; pose de la moitié **coupure** aux sorties `1492` et `4373` ; `SilentTurnOutcome` gagne `qa_verdict_cut_off: Option<CallbackCutOff>` (symétrique de `AgentOutput.deadline_exceeded`) ; **quatre** renvois dans `run_silent_agent`, dont **trois à câbler** — le `default()` de la sous-branche « deadline trop proche » (`6560`), le `default()` du bras `DeadlineExceeded` (`6605`), la construction finale (`6650`) qui lit déjà le drapeau et doit lire le nouveau champ — **plus un à déclarer exclu**, le prélude de deadline (`6443`), cf. *Le quatrième renvoi* |
| **U1c** | `src/server/deadline_verdict.rs` | variante `CallbackCutOffWithoutVerdict`, son corps, `event_name()`, le champ `cause` sur la ligne postée |
| **U1d** | `src/task_engine/dispatcher.rs` | sélection du motif dans `post_callback_verdict_net` (match à quatre bras) |
| **U1e** | `src/agent_loop/mod.rs` | P0 : troisième miroir mika#2368 au site `4356`, `verdict_unmet_after_retry` (terme de budget **conservé**), aucun `cause` |
| **U2a** | `src/qa_build_callback.rs` | `UndeliveredVerdictCause` (4 variantes, format de fil), `classify_undelivered_verdict(metadata) -> UndeliveredVerdictCause` — **fonction pure**, testable aux bornes sans base |
| **U2b** | `src/task_engine/engine.rs` | `alert_undelivered_build_verdicts()` dans le bloc à 60 ticks, après `dispatch_undelivered_callbacks` ; dédup 24 h ; WARN + ligne `audit_events` |
| **U3** | `src/task_engine/dispatcher.rs` | compteur + instant NULL-only au site `AgentBusy`, borné aux callbacks de build |
| **R** | `crates/mika-common/src/config.rs`, `.env.example` | les trois clés, leurs constantes `DEFAULT_*` documentées, les accesseurs |
| **T** | tests + scans | voir *Vérification* |
| **D** | `CLAUDE.md` racine, `crates/mika-agent/CLAUDE.md` | entrée opérateur, taxonomie, sondes, haltes |

## Fire-Disposition

Ce plan livre des détecteurs (scans de source et tests d'assertion). Disposition
retenue : **(a) exception nommée en allowlist**. Deux des trois scans portent sur
des noms et des sites que ce travail **crée**, donc leurs allowlists sont livrées
**vides** et un test frère refuse qu'elles cessent de l'être. Le troisième porte
sur du code préexistant et livre **une** exception, nommée, motivée par un
prédicat démontrable et assortie de son assertion auto-nettoyante. Détail :

- `mika2515_the_undelivered_alert_has_a_single_writer` — scan de source, SOLE
  WRITER de `qa_build_verdict_undelivered` (journal **et** `audit_events`).
  Allowlist `const ALLOWED: &[&str] = &[]`, plus un test frère
  `…_the_allowlist_is_empty` qui refuse qu'elle cesse de l'être (mika#2323 : une
  allowlist née vide est un endroit où déposer la prochaine infraction). Porte sa
  propre **assertion anti-vacuité** : le scan échoue si le nom est écrit **nulle
  part** — un scan qui visait un nom mort se lit exactement comme un scan propre
  (mika#2103 / mika#2205).
- `mika2515_every_loop_exit_decides_about_the_verdict_signal` — scan de source sur
  `agent_loop/mod.rs`. **Cardinalité assertée à 6** — c'est le nombre de
  `return Ok(LoopResult::…)` / `Ok(LoopResult::…)` du corps de `run_loop`, relevé
  à HEAD `bb198d2e` (`1492`, `3971`, `4167`, `4214`, `4356`, `4373`), et le plan
  l'a écrit **4** avant que le relevé ne soit fait : une cardinalité posée de
  mémoire est exactement ce que ce scan existe pour empêcher. La cardinalité est
  le seul terme qu'aucune fixture ne peut voir — un prédicat devenu trop étroit
  passerait en ne regardant rien (mika#2205). **Le scan asserte la cardinalité,
  jamais les numéros** : ceux-ci ont déjà dérivé de +59 entre deux passes de ce
  plan, et un scan qui les figerait rougirait à chaque réécriture du fichier sans
  qu'aucune sortie ne soit devenue muette — un détecteur qu'on désarme parce qu'il
  crie à tort.

  **Le motif du scan est `Ok(LoopResult::`, jamais `return Ok(LoopResult::`, et
  la nuance décide de la cardinalité.** La sortie `MaxStepsExceeded` (`4373`)
  est l'**expression finale** de `run_loop` : elle n'a pas de `return`. Un
  prédicat ancré sur `return` en trouve **cinq**, et l'implémenteur a alors deux
  façons de se tromper — faire rougir un scan correct, ou « corriger » la
  cardinalité à 5 et sortir en silence la sortie max-steps de la population que
  ce plan existe pour fermer. Inversement, ancrer sur `LoopResult::` nu rend
  **vingt-quatre** résultats (les motifs de `match` des trois boucles), donc un
  scan permanemment rouge, c'est-à-dire désarmé. Le motif `Ok(LoopResult::` rend
  exactement les six, sans exception à déclarer : relevé et vérifié.

  **`Ok(SilentTurnOutcome` porte le piège jumeau**, et pour la même raison
  structurelle : trois renvois sont des `return … ::default()`, le quatrième
  (`6650`) est la construction finale sans `return` — et c'est précisément celui
  qui **lit** le drapeau, donc celui dont l'omission serait la plus coûteuse. Un
  scan ancré sur `return` y compterait trois, et trois est aussi le compte que
  les versions antérieures de ce plan portaient : le mauvais prédicat aurait
  **confirmé** la mauvaise cardinalité. C'est la forme d'erreur que deux
  instruments qui se trompent ensemble produisent, et la seule parade est que la
  cardinalité soit relevée contre le fichier, jamais dérivée d'un grep qu'on
  s'est choisi.

  Le scan asserte que **chacune des six sorties décide**, au sens où elle est
  soit suivie d'une pose de signal, soit **déclarée exclue avec sa raison** dans
  une table `LOOP_EXITS_WITHOUT_SIGNAL` d'une seule entrée :

  ```rust
  /// Sorties de `run_loop` qui ne posent délibérément aucun signal de verdict.
  /// Ce n'est PAS une allowlist d'infractions : chaque entrée nomme une sortie
  /// structurellement inatteignable depuis un tour de callback, avec le
  /// prédicat qui l'établit. Quand le scan tire, on arme le site — on n'ajoute
  /// une entrée QUE si l'inatteignabilité est démontrable comme celle-ci.
  const LOOP_EXITS_WITHOUT_SIGNAL: &[(&str, &str)] = &[(
      "Done — empty text after follow-up",
      "LoopMode::follow_up_on_empty() est false pour Silent (mika#2515) : \
       inatteignable depuis un tour de callback. Épinglé par \
       test_loop_mode_silent_properties.",
  )];
  ```

  L'entrée porte son **assertion auto-nettoyante** : un test frère asserte
  `!LoopMode::Silent{..}.follow_up_on_empty()`, donc le jour où cette propriété
  change, l'exclusion rougit au lieu de laisser une sortie s'ouvrir en silence.

  **Le même scan couvre les quatre renvois de `run_silent_agent`** — cardinalité
  assertée à **4** — via la table sœur `SILENT_RETURNS_WITHOUT_VERDICT_READ` d'une
  entrée, dont le vocabulaire est **délibérément distinct** (indisponibilité du
  prédicat, jamais inatteignabilité du site : cf. *Le quatrième renvoi*). Son
  assertion auto-nettoyante n'est pas booléenne mais **ordinale** : le calcul de
  `loaded_skill_names` doit rester **après** le prélude dans `run_silent_inner`.
  Sans cette moitié, le scan couvrirait la fonction qui *pose* le signal et
  laisserait libre celle qui le *lit* — c'est-à-dire exactement la moitié par
  laquelle le fait se perd.
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
- **V2 — le signal sort des cinq sorties atteignables (chemin de production).**
  Cinq scénarios `run_silent_agent` sur un message
  `[callback: long_running:build_mika]` avec `qa-review` chargé : sortie EndTurn
  texte non vide → `qa_verdict_unmet` ; sortie EndTurn texte vide → idem ;
  **`send_message` puis Force EndTurn (P0) → `qa_verdict_unmet`** — celui-ci est
  vu **rouge avant U1e**, ce qui est la preuve que le trou existait et non
  qu'on a écrit un test autour du code ; deadline courte (via
  `run_silent_agent_with_deadline`) → `qa_verdict_cut_off = Some(Deadline)` avec
  ses steps ; max-steps → `Some(MaxSteps)`. Plus **trois contrôles négatifs** :
  un tour qui a posté sa revue puis est coupé ne pose rien ; un tour qui a posté
  sa revue puis sort par le Force EndTurn ne pose rien ; un tour de callback
  **non-build** (`run_claude_pilot`) ne pose rien.
- **V2b — le prédicat de P0 est bien le frère, pas celui de la coupure.** Sur le
  site `4356`, un tour dont la garde n'a **jamais** firé (budget non dépensé) ne
  pose **rien** — c'est le contrôle qui distingue U1e d'U1a et qui rougirait si
  quelqu'un « harmonisait » les deux sites sur un seul prédicat.
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
  rouge avant que le code existe, par anti-vacuité). Y compris les **deux**
  cardinalités (6 sorties de `run_loop`, 4 renvois de `run_silent_agent`) et
  l'assertion **ordinale** du quatrième renvoi : un `loaded_skill_names` déplacé
  avant le prélude fait rougir l'exclusion. Contrôle de bonne foi de cette
  moitié-là : le déplacement est simulé sur une fixture, vu rouge, puis défait —
  sans quoi « le scan asserte l'ordre » est indistinguable de « le scan lit un
  fichier qui se trouve être dans le bon ordre ».

**Le voisin à lire avant d'écrire V7 :**
`crates/mika-agent/tests/eval/test_callback_delivery_starvation.rs` couvre
mika#2179, c'est-à-dire **P2 et P2 seulement** — le chemin où
`run_silent_agent` rend `Err`. Il ne touche **pas** P1 : un `AgentBusy` est
refusé avant que ce fichier n'ait un tour à observer. Ne pas le lire comme
« la famine est déjà couverte ». En revanche sa **recette d'injection-vérification**
(commenter le `log_audit_event` du site, voir rougir, restaurer) est le modèle
exact à reprendre pour V7 et pour le `log_audit_event` d'U2b : elle prouve que
le chemin d'écriture est porteur plutôt qu'incident.

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

1. **Chacune des six sorties de `run_loop` décide** du signal de
   verdict-dû-non-posté : cinq le posent (les deux de mika#2368, le Force
   EndTurn de P0, les deux coupures), la sixième est déclarée exclue avec son
   prédicat d'inatteignabilité. `SilentTurnOutcome` porte le fait de coupure,
   symétriquement d'`AgentOutput.deadline_exceeded`.
1b. **Chacun des quatre renvois de `run_silent_agent` décide** aussi : trois
   lisent le fait au lieu de le jeter, le quatrième (prélude de deadline) est
   déclaré exclu dans une table **au vocabulaire distinct** — indisponibilité du
   prédicat, non inatteignabilité — avec son assertion d'ordre. Couvrir la
   fonction qui pose le signal sans couvrir celle qui le lit laisserait ouverte
   la moitié par laquelle le fait se perd.
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
   des populations P0, P1, P2 et P3 produit au moins une ligne nommée.
3. **P3 est fermé côté verdict** : un tour de callback de build coupé par sa
   deadline ou par sa limite de steps, qui devait un verdict et n'en a posté
   aucun, produit un `hold[review]` du moteur sur la PR estampillée.
3b. **P0 est fermé côté verdict, et compté avec mika#2368** : un tour de
   callback qui conclut par le « Force EndTurn » de `send_message` sans avoir
   posté sa revue produit un `hold[review]` sur la ligne `qa_callback_verdict`
   **sans `cause`** — la réparation entre dans la population qu'elle répare,
   elle n'en crée pas une sixième.
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
- **Le prélude de deadline de `run_silent_agent`** (mika#2515-a). Quatrième renvoi,
  déclaré exclu avec son assertion d'ordre plutôt qu'armé : le prédicat n'existe
  pas encore à ce site, et sa population exigerait que l'enveloppe entière
  s'écoule dans deux appels de base. **Précondition explicite :**
  `grep 'silent agent deadline exceeded during prelude' "$MIKA_SPIRIT_LOG_FILE"`
  croisé avec `trigger_label = "callback"` doit être **non vide**. Le WARN existe
  déjà, donc la mesure ne coûte rien à produire — et zéro ligne signifie **ne pas
  ouvrir le suivi**, un site armé pour une population vide étant une garde sans
  population (mika#2272).
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
