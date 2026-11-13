# mika#2943 — un tour webhook mika-qa coupé par `max_steps` pose son verdict

> **Vol 1/2.** Ce plan borne son périmètre au **code de production du filet et à
> ses tests unitaires**. L'attestation de bout en bout — `run_agent` réel →
> coupure au plafond → champ posé → POST — part au vol 2.
> `Tracked in: senara-solutions/mika#2943 — vol 2/2`.

## Contexte

### Le défaut, et sa forme exacte

Un tour **webhook** de mika-qa qui épuise son budget de pas d'outil se termine
sans verdict et sans que rien ne le rattrape. Le ticket en compte **6 depuis le
26/10**, dont `b22e6ea0` sur #2920 à sa sortie de draft, au plafond de 30 que
mika#2926 venait de poser.

Le filet existe — `server::deadline_verdict` — mais il ne voit pas ce chemin,
pour une raison de **forme**, pas d'oubli :

| site | ce qu'il fait aujourd'hui |
|---|---|
| `agent_loop/mod.rs:5202-5234` | en fin de budget, `run_loop` rend `LoopResult::MaxStepsExceeded`. Elle pose `CutOffExit::MaxSteps` **sur `qa_verdict_unmet`** — le signal du chemin *callback* (mika#2515). Rien pour le chemin webhook. |
| `agent_loop/mod.rs:6579-6667` | le bras `MaxStepsExceeded` de `run_agent` lance une continuation et construit `AgentOutput { deadline_exceeded: None, … }`. Le commentaire est explicite : *« Max-steps continuation, not a deadline overrun: the turn produced a summary. mika#2276's net must not fire here. »* |
| `server/handlers.rs:1560` | `conclusion_of` lit **un seul champ** : `Ok(output) => TurnConclusion::from(output.deadline_exceeded)`. |
| `server/deadline_verdict.rs:405` | `deadline_verdict_target` : `TurnConclusion::Concluded => return None`. |

Le tour coupé arrive donc au call-site webhook sous l'apparence d'un tour qui a
**conclu** — parce qu'il a produit un texte de continuation. Ce texte n'atteste
rien, d'où le `webhook_turn_outcome = outcome_absent` mesuré ; mais il est, pour
le filet, indiscernable d'une vraie réponse. C'est **exactement** le mécanisme
que mika#2276 avait dû nommer pour la deadline (*« `text` cannot answer the
question »*), un champ plus loin.

### Un demi-chemin est déjà couvert, et il ne faut pas y toucher

Le bras `MaxStepsExceeded` a **deux sorties**, et une seule est le trou :

1. **Continuation sautée** (`agent_loop/mod.rs:6590-6617`) — la deadline est trop
   proche pour payer les 60 s de la continuation. Le code appelle
   `persist_deadline_fallback(…, max_steps, …)`, qui est *« the single place that
   stamps `AgentOutput::deadline_exceeded` »*. Le filet mika#2276 **s'arme déjà**
   sur cette sortie. Rien à faire.
2. **Continuation effectuée** (`agent_loop/mod.rs:6620-6667`) — `deadline_exceeded:
   None`. **C'est le trou**, et c'est la sortie que les 6 cas mesurés ont prise :
   `b22e6ea0` a brûlé 46 appels pour 30 pas en 5 s, très loin de son enveloppe de
   temps.

### Ce que les 9 cas disent sur la population à couvrir

Le ticket a trié les 9 tours coupés en trois classes, et chacune commande une
décision différente :

| classe | n | ce que la PR a reçu | ce que le filet doit faire |
|---|---|---|---|
| revue postée avant la coupure (`e6e7d8e0`) | 1 | sa revue | **rien** — AC2 |
| build `long_running` lancé avant la coupure (`11699630`, `517380e0`) | 2 | sa revue, par le callback et son filet | **rien** — AC3, tranché ci-dessous |
| rien laissé | 6 | **rien** | poster — AC1 |

## Le remède : un second champ qui dit « coupé au plafond de pas », et un motif de plus

Le geste est celui de mika#2276 M2, répété un champ plus loin : **le fait voyage
dans `AgentOutput`, la décision vit dans `deadline_verdict`.**

```text
run_loop → MaxStepsExceeded   (budget de pas épuisé)
  ↓  pré-calcul sur les tool summaries : un build QA est-il en vol ?
     oui → rien n'est posé, l'abstention est journalisée sur place
     non → AgentOutput.max_steps_exceeded = Some(MaxStepsCutOff { steps_completed })
  ↓  conclusion_of → TurnConclusion::MaxStepsExceeded { steps_completed }
  ↓  deadline_verdict_target → VerdictReason::WebhookCutOffByMaxSteps { steps_completed }
  ↓  maybe_post_deadline_verdict → session_has_review_for ? → POST gh pr review
```

Quatre décisions de conception portent ce plan, et chacune a son précédent dans
le même code.

### D1 — Un second champ, pas une réutilisation de `deadline_exceeded`

Confondre les deux couperait une population que l'opérateur agrège, et ferait
dire au corps du verdict *« a atteint la limite de son enveloppe de temps »* sur
un tour qui a brûlé 46 appels en 5 s. Le doc-comment de `deadline_exceeded` dit
déjà le contraire en toutes lettres (*« a turn that ran out of **steps** did
produce a summary and is not this class »*) : ce commentaire reste vrai, il
cesse d'être la fin de l'histoire.

### D2 — Même événement que mika#2276, borne discriminée par `cause`

`event_name()` rend `DEADLINE_VERDICT_EVENT` (`qa_deadline_verdict`) et
`cause()` rend `qa_build_callback::CAUSE_CUT_OFF_MAX_STEPS`
(`"cut_off_max_steps"`), la constante **déjà existante** — aucun second littéral.

C'est le geste de mika#2289 (`cause=error` sur le même événement), et il est
**compatible avec la doctrine mika#2515**, qui refusait l'inverse : mika#2515
refusait de router une population *callback* sur l'événement *webhook*, parce que
la sonde de contrôle négatif de mika#2355 dit « le filet mika#2276 ne doit pas se
mettre à firer ». Ici la population **est** celle du chemin webhook, celle que
`qa_deadline_verdict` nomme. La soustraction opérateur reste exacte :
`select(.cause == null)` isole la population fondatrice mika#2276,
`select(.cause == "cut_off_max_steps")` la nouvelle.

C'est la lecture d'AC1 *« l'événement est celui du filet de mika#2276, il n'y a
pas de seconde grammaire »* : pas de nom neuf, pas de seconde ligne de verdict,
pas de second mécanisme.

### D3 — Le corps est de la grammaire mika#2276, avec le mot juste sur la borne

AC1 dit *« le corps est celui du filet de mika#2276 »*. Ce qui est réutilisé est
la **grammaire** : la ligne canonique `DEADLINE_VERDICT_LINE`, le préambule
*« Ce verdict est posté par le moteur, pas par le tour de revue »*, la mention du
nombre de pas, le conseil de relance, la ligne `Trace :`, le footer `<sub>`.

Ce qui change est **une phrase** : « la limite de son budget de steps d'outil »
au lieu de « la limite de son enveloppe de temps ». Deux raisons, et chacune
suffit :

1. la phrase mika#2276 est **fausse** d'une coupure par pas — `b22e6ea0` avait
   295 s d'enveloppe devant lui ;
2. la branche `CutOffByDeadline` de `build_verdict_body` est **figée octet pour
   octet** par `tests::mika2368_the_deadline_body_is_frozen_byte_for_byte`
   (AC5c de mika#2368). On ne peut pas la réemployer sans soit la casser, soit
   mentir.

mika#2515 a tranché exactement ce point pour le chemin callback, avec exactement
ce raisonnement, et a produit une cinquième branche de `build_verdict_body`
plutôt qu'un corps partagé mensonger. On suit ce précédent.

### D4 — AC3 : sur un build en vol, le filet webhook **s'abstient**, et le dit

**Décision : abstention.** Les deux cas mesurés (`11699630` sur #2920,
`517380e0` sur #2932) ont reçu leur revue par le callback. Trois raisons :

1. **Un `hold[review]` y serait faux.** Le corps dit *« la revue n'a pas
   abouti »* ; elle aboutissait, quelques minutes plus tard.
2. **La déduplication existante ne peut pas le rattraper.**
   `session_has_review_for` lit `AppState.pr_reviews_posted` indexé **par
   `session_id`**, et le tour de callback tourne dans une autre session. Le
   double-post ne serait filtré par rien.
3. **Le `hold` entrerait dans le registre des verdicts** (mika#2927, mika#2934) :
   la même tête porterait deux verdicts du même auteur machine, dont un faux.

Le **coût** de l'abstention est nommé : si le spawn du build échoue, ou si la
tâche callback est perdue, personne ne poste sur cette PR. Ce coût est **couvert
en aval** par `qa_review_reconcile` (mika#2334), le scan périodique qui repose un
relecteur sur une PR ouverte restée sans revue — et il l'est *parce que* le filet
s'abstient : un `hold` posté sortirait au contraire la PR de la population du
réconciliateur.

**Le signal, et où il est lu.** `tasks.metadata.qa_review_pr_target` existe déjà
(mika#2368 C2) et dit « un callback de build portera un verdict sur cette PR »,
mais il vit en base et le filet webhook ne doit pas y faire d'I/O. Le même fait
se lit **localement** sur les `tool_call_summaries` du tour : un appel réussi à
`qa_build_callback::BUILD_MIKA_TOOL` est précisément ce qui a déclenché le stamp.
C'est le voisin exact de `pr_review_posted_in_turn`, et le prédicat se pose au
même endroit que `verdict_unmet_at_cut_off` : **dans la boucle, sur la sortie
coupée**. Ce n'est pas une garde du filet déplacée hors du filet — c'est la
**condition de pose du fait**, le geste que `run_loop` fait déjà deux lignes plus
haut pour `qa_verdict_unmet`.

L'abstention **est journalisée sur place**, au site qui émet déjà
`max_steps_tool_census` : absence n'est jamais preuve, et une abstention muette
est indiscernable d'un filet cassé.

## Requirements

### R1 — `qa_build_callback` : le prédicat « un callback de build portera le verdict »

`crates/mika-agent/src/qa_build_callback.rs`.

```rust
/// Un callback de build QA a-t-il été dispatché dans ce tour ?
///
/// Voisin exact de [`pr_review_posted_in_turn`], et pour le même emploi : un
/// terme partagé plutôt qu'une conjonction recopiée (leçon mika#2158).
///
/// Lu par le chemin webhook de mika#2943 sur sa sortie coupée. Un `true` dit
/// qu'une tâche callback existe, porteuse du stamp
/// `deadline_verdict::QA_REVIEW_PR_TARGET_KEY` que
/// `skills::executor::resolve_qa_review_pr_target` pose sur ce même nom d'outil
/// — donc que le filet mika#2368/#2515 de ce callback portera le verdict si le
/// callback revient muet ou coupé. Le filet webhook s'abstient alors (AC3).
pub fn build_callback_dispatched_in_turn(summaries: &[ToolCallSummary]) -> bool {
    summaries
        .iter()
        .any(|s| s.name == BUILD_MIKA_TOOL && s.success)
}
```

Le `name == BUILD_MIKA_TOOL` est la même lecture que `resolve_qa_review_pr_target`
fait (`tool_name != BUILD_MIKA_TOOL → None`) : les deux moitiés du raisonnement
s'ancrent sur la même constante, qui est déjà le site unique.

### R2 — `agent_loop` : le fait de coupure par pas remonte dans `AgentOutput`

`crates/mika-agent/src/agent_loop/mod.rs`.

**R2a — le type.** À côté de `DeadlineOverrun` :

```rust
/// Le tour a épuisé son **budget de pas d'outil** sans conclure sur ce qu'il
/// devait conclure (mika#2943).
///
/// Symétrique de [`DeadlineOverrun`], et distinct à dessein : confondre les deux
/// ferait dire au corps du verdict « a atteint la limite de son enveloppe de
/// temps » d'un tour qui avait encore 295 s devant lui (trace `b22e6ea0`), et
/// couperait en deux une population que l'opérateur agrège par `cause`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxStepsCutOff {
    /// Le plafond **servi**, transporté par `LoopResult::MaxStepsExceeded`
    /// (mika#2926 R1b) — jamais relu depuis une constante, qui annoncerait 20
    /// pour un tour de mika-qa qui en a consommé 30.
    pub steps_completed: usize,
}
```

**R2b — le champ.** Sur `AgentOutput`, immédiatement après `deadline_exceeded`,
avec le doc-comment qui dit les deux bornes de sa `Some`-ité :

- `Some` **uniquement** sur la sortie « continuation effectuée » du bras
  `LoopResult::MaxStepsExceeded`, et seulement quand aucun callback de build QA
  n'a été dispatché dans le tour (R1, AC3) ;
- `None` partout ailleurs — **y compris sur la sortie « continuation sautée »**,
  qui passe par `persist_deadline_fallback` et pose déjà `deadline_exceeded`.
  Les deux champs sont **mutuellement exclusifs par construction**, chaque sortie
  de `run_loop` n'en posant qu'un.

Les trois autres constructions d'`AgentOutput` (`mod.rs:6570`, `mod.rs:6747`,
`tests/eval/grounding_assertions/mod.rs:817`) reçoivent `None`.

**R2c — le site de pose.** Dans le bras `LoopResult::MaxStepsExceeded` de
`run_agent`, le prédicat R1 est évalué **avant** `attempt_continuation_turn` —
qui emprunte `tool_call_summaries` — exactement comme le fait déjà
`classify_delegation_from_summaries` dans le bras jumeau du team agent
(commentaire mika#1671 D1/D2, `mod.rs:8415`).

```rust
// mika#2943 — le fait que mika#2276 ne pouvait pas voir : un tour coupé au
// plafond de pas produit un texte de continuation, donc indiscernable d'une
// réponse pour le call-site. Évalué AVANT la continuation, qui emprunte les
// summaries.
let build_callback_pending =
    crate::qa_build_callback::build_callback_dispatched_in_turn(&tool_call_summaries);
if build_callback_pending {
    info!(
        event = "qa_max_steps_verdict_withheld",
        trace_id = %trace_id,
        session_id,
        max_steps,
        cause = "build_callback_pending",
        "mika#2943 : tour coupé au plafond de pas avec un dispatch de build QA \
         en vol — le filet de ce callback portera le verdict, aucun fait de \
         coupure n'est posé ici"
    );
}
```

puis, dans la construction finale du bras :

```rust
max_steps_exceeded: (!build_callback_pending)
    .then_some(MaxStepsCutOff { steps_completed: max_steps }),
```

### R3 — `deadline_verdict` : une conclusion, un motif, un corps

`crates/mika-agent/src/server/deadline_verdict.rs`.

**R3a — `TurnConclusion::MaxStepsExceeded { steps_completed: usize }`**, quatrième
variante. Le `match` de `deadline_verdict_target` est **sans bras `_`** — c'est
le contrat du type, et il force la décision plutôt qu'un silence par défaut.

**R3b — le convertisseur devient total.** `impl From<Option<DeadlineOverrun>> for
TurnConclusion` est **remplacé** par :

```rust
impl TurnConclusion {
    /// La traduction d'un `AgentOutput` : un tour qui a rendu `Ok` a conclu, ou
    /// a été coupé par l'une de ses deux bornes — il n'est pas mort.
    ///
    /// Remplace l'`impl From<Option<DeadlineOverrun>>` de mika#2276, qui ne
    /// lisait qu'un champ : le garder à côté de celui-ci en ferait un piège, un
    /// futur lecteur l'appellerait et perdrait la moitié « coupé par les pas ».
    /// Un seul convertisseur, et il est total.
    ///
    /// Les deux champs sont mutuellement exclusifs par construction (R2b). Le
    /// croisement est quand même traité, en faisant gagner la deadline — jamais
    /// par un `unreachable!()` : paniquer sur un champ d'observabilité serait le
    /// pire des échanges (doctrine mika#2515).
    pub fn of_output(output: &crate::agent_loop::AgentOutput) -> Self { … }
}
```

`conclusion_of` (`handlers.rs:1558`) et sa copie locale dans
`tests/eval/test_error_verdict_2289.rs:71` passent à `TurnConclusion::of_output(output)`.

**R3c — `VerdictReason::WebhookCutOffByMaxSteps { steps_completed: usize }`**,
cinquième motif. `event_name()` → `DEADLINE_VERDICT_EVENT` ; `cause()` →
`Some(crate::qa_build_callback::CAUSE_CUT_OFF_MAX_STEPS)`. Les deux `match` sont
exhaustifs sans bras `_` : le doc-comment de `cause()` l'exige déjà
(*« un cinquième motif devra décider de son mot »*) — ce plan est ce cinquième
motif, et il décide.

**R3d — la branche de `build_verdict_body`**, grammaire mika#2276 (D3), phrase de
la borne juste, footer `<sub>mika#2943</sub>`, et la ligne `Trace :` qui nomme
`qa_deadline_verdict` **et** `cause=cut_off_max_steps` — comme le fait déjà la
branche mika#2515, parce que c'est le `cause` qui rend la ligne retrouvable.

**R3e — le bras de `deadline_verdict_target`** :

```rust
TurnConclusion::MaxStepsExceeded { steps_completed } => {
    VerdictReason::WebhookCutOffByMaxSteps { steps_completed }
}
```

Rien d'autre ne bouge : la garde non-PR (`parse_pr_target(event_text)?`) et la
déduplication (`session_has_review_for`) couvrent AC2 **telles quelles**, sans
une ligne de plus. C'est la moitié d'AC2 que le code porte déjà ; ce plan livre
le test qui l'atteste sur cette population.

### R4 — `handlers.rs` : le call-site lit le convertisseur total

`conclusion_of` devient :

```rust
fn conclusion_of(run: &anyhow::Result<agent::AgentOutput>) -> TurnConclusion {
    match run {
        Ok(output) => TurnConclusion::of_output(output),
        Err(e) => { /* inchangé */ }
    }
}
```

Aucune autre ligne de `handlers.rs` ne change : `post_verdict_if_turn_did_not_conclude`
est déjà générique sur le motif.

### R5 — Tests unitaires (AC1, AC2, AC3)

Dans `deadline_verdict.rs::tests`, nommés `mika2943_*` :

1. **`of_output` est total** — table des trois formes d'`AgentOutput` (ni l'un ni
   l'autre → `Concluded` ; `deadline_exceeded` seul → `DeadlineExceeded` ;
   `max_steps_exceeded` seul → `MaxStepsExceeded`), plus le croisement, qui rend
   `DeadlineExceeded` sans paniquer.
2. **AC1 — le POST** — `deadline_verdict_target(MaxStepsExceeded { 30 }, <événement
   `ready_for_review` de #2920>)` rend le motif et la cible ; `maybe_post_deadline_verdict`
   avec un `poster` capturant rend `Posted` ; **un seul** POST, sur
   `senara-solutions/mika#2920` ; le corps porte `DEADLINE_VERDICT_LINE`, le
   nombre `30`, le `trace_id`, et **ne porte pas** « enveloppe de temps ».
3. **AC1 — le fil** — `event_name() == DEADLINE_VERDICT_EVENT` et
   `cause() == Some("cut_off_max_steps")`, cette dernière assertée **contre
   `qa_build_callback::CAUSE_CUT_OFF_MAX_STEPS`**, pas contre un littéral recopié.
4. **AC2 — déduplication** — registre pré-chargé via `format_pr_dedup_key` pour
   cette session et cette PR (les **deux** formes, avec et sans `--repo`, comme le
   fait le test mika#2276 existant) : `AlreadyReviewed`, **zéro** POST. C'est le
   cas `e6e7d8e0`.
5. **AC2 — tour non-PR** — `deadline_verdict_target(MaxStepsExceeded { 30 }, "Bonjour")`
   rend `None` : zéro POST, et ce n'est pas un défaut.
6. **La population fondatrice est intacte** — `mika2368_the_deadline_body_is_frozen_byte_for_byte`
   et le `cause() == None` de `CutOffByDeadline` / `CallbackConcludedWithoutVerdict`
   restent verts, non modifiés. C'est le contrôle qui dit que D2 n'a pas déplacé
   une population existante.

Dans `qa_build_callback.rs::tests`, un test paramétré pour R1 (AC3) : un
`build_mika` réussi → `true` ; un `build_mika` **échoué** → `false` ; un autre
`long_running` → `false` ; aucun outil → `false`.

## Non-Requirements (hors de ce plan, délibérément)

- **Pourquoi mika-qa brûle ses pas en `run_shell`** — mika#2926 (b), le
  resserrement du skill. Ce plan ne pose que le filet.
- **Le relèvement du plafond** — hors périmètre par le ticket.
- **La sortie « continuation sautée »** du bras `MaxStepsExceeded` — elle pose
  déjà `deadline_exceeded` et arme déjà le filet mika#2276, avec le corps
  « enveloppe de temps ». Ce corps est partiellement juste (c'est bien la
  proximité de la deadline qui a fait sauter la continuation) et le changer
  casserait le figement AC5c de mika#2368. Rien ne bouge là.
- **Le chemin callback** (mika#2368, mika#2515) — aucune ligne, aucun corps,
  aucun `cause` de cette population ne change.
- **Le test de bout en bout** (`tests/eval/test_webhook_max_steps_verdict_2943.rs`)
  — vol 2, voir `## Taille estimée`.

## Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | une coupure par pas sur un événement PR produit un POST, un seul, sur la bonne PR, avec `cause=cut_off_max_steps` et le nombre de pas | R5.2 + R5.3 |
| V2 | une coupure par pas dont la session a déjà revu ne poste rien | R5.4 |
| V3 | une coupure par pas sur un tour non-PR ne poste rien | R5.5 |
| V4 | un build QA en vol empêche la pose du fait, et l'abstention est journalisée | R5 (`qa_build_callback`) + grep `qa_max_steps_verdict_withheld` |
| V5 | les populations mika#2276 / #2368 / #2515 sont inchangées | R5.6 — tests existants verts, non modifiés |
| V6 | aucun convertisseur partiel ne subsiste | `grep -rn 'TurnConclusion::from'` sur `crates/` ne trouve rien |
| V7 | le dépôt compile, lint, et passe sa suite | `cargo test`, `cargo clippy`, `make lint`, `scripts/verify-pipeline.sh origin/main` |

## Definition of Done

- `cargo test -p mika-agent` vert, `cargo clippy --all-targets` sans warning,
  `cargo fmt --check` propre.
- `cargo test` complet vert (la suite entière, ~3463 tests).
- `scripts/verify-pipeline.sh origin/main` vert.
- V6 : `grep -rn 'TurnConclusion::from' crates/` ne rend aucune ligne.
- `docs/agent-internals/` mis à jour là où le filet est décrit (hors compte de
  lignes).
- PR ouverte, corps portant la sonde post-déploiement (AC4) et
  `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2943.

- **AC1.** Quand un tour webhook mika-qa est coupé par `max_steps` sur un
  événement de PR, sans verdict posté dans la session, il poste par le moteur un
  `VERDICT: hold[review]` sur cette PR, avec `cause=cut_off_max_steps` et le
  nombre de pas. Le corps et l'événement sont ceux du filet de mika#2276. Il n'y
  a pas de seconde grammaire.
- **AC2.** Contrôle négatif dans le même test. Une coupure `max_steps` dont la
  session a déjà posté une revue (le cas `e6e7d8e0`) ne poste rien de plus : la
  déduplication passe par `session_has_review_for`. Un tour non-PR coupé ne poste
  rien.
- **AC3.** Le cas « build lancé avant la coupure » (`11699630`, `517380e0`) est
  tranché par écrit dans le plan. Soit le filet du webhook s'abstient, parce que
  le callback portera le verdict, soit il poste, et alors le plan dit pourquoi un
  double `hold` est acceptable.
- **AC4.** Sonde après déploiement. Au prochain `max_steps_tool_census` d'un tour
  webhook mika-qa sur une PR, la même trace porte un `deadline_verdict` posté, ou
  une revue de la session.

**Où chaque AC est satisfait.** AC1 → R2, R3, R4, R5.2, R5.3. AC2 → R3e (le code
existant) + R5.4, R5.5. AC3 → D4 (la décision écrite) + R1 + R2c + R5. AC4 → la
section `## Sonde post-déploiement` ci-dessous.

## Fire-Disposition

Ce plan livre des détecteurs : sept tests unitaires dont le chemin de succès est
« aucune violation trouvée » (R5), plus la vérification structurelle V6. La
section est due.

**Option retenue : (a) exception nommée en allowlist, livrée VIDE.**

Il n'y a **aucune** violation préexistante à exempter, et ce n'est pas une
coïncidence : chaque détecteur de R5 porte sur un comportement que le même commit
introduit (R2, R3) ou sur une population que le même commit laisse intacte
(R5.6). Un détecteur posé avant son correctif atterrirait rouge et exigerait une
entrée d'allowlist à retirer au commit suivant — l'ordre de commit ci-dessous
l'évite structurellement.

L'allowlist de V6 est la seule qui aurait pu être non vide, et elle est
**auto-nettoyante par construction** plutôt que par assertion : le détecteur est
`grep -rn 'TurnConclusion::from' crates/`, et R3b **supprime le symbole**. Un
`TurnConclusion::from` qui réapparaîtrait ne compilerait pas — l'`impl From`
n'existe plus. L'exception expire par le geste qui la rend inutile, ce que
mika#2309 exige, et ici le compilateur en est le gardien : il n'y a pas de
littéral à laisser pourrir dans une liste.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/qa_build_callback.rs` (R1 + ses tests) | 46 |
| `crates/mika-agent/src/agent_loop/mod.rs` (R2a/b/c) | 86 |
| `crates/mika-agent/src/server/deadline_verdict.rs` (R3a–e) | 129 |
| `crates/mika-agent/src/server/handlers.rs` (R4) | 15 |
| `crates/mika-agent/tests/eval/test_error_verdict_2289.rs` (R3b, 2 lignes) | 2 |
| tests (`deadline_verdict.rs::tests`, R5.1–R5.5) | 136 |

Total estimé : 414 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et ce qu'il renvoie.** 414 contre 429 : la marge est de
15 lignes, mince, et le découpage est ce qui l'achète. Le poste qui ne tenait pas
est le test de bout en bout : sur le modèle de `test_deadline_verdict_2276.rs`
(296 lignes) et de `test_qa_cut_off_verdict_2515.rs` (648), un
`test_webhook_max_steps_verdict_2943.rs` complet — en-tête doctrinal, harness
`run_until_max_steps_exceeded`, test AC1, contrôle négatif AC2, cas du build en
vol AC3 — vaut **≈ 210 lignes**, ce qui porterait le total à 624 et tuerait le
vol au plafond de tours.

**Vol 1 (ce plan)** est mergeable seul et corrige le défaut : le fait remonte, le
filet poste, l'abstention AC3 est en place — sans elle le déploiement produirait
un double `hold` dès le premier build en vol, ce qui interdit de la renvoyer au
vol 2. Son plancher de vérification est R5 : la décision du filet est attestée
sur les quatre formes d'entrée, et le câblage `AgentOutput → TurnConclusion` par
R5.1. Ce qu'il ne couvre pas, et qu'il nomme : que `run_loop` pose bien le champ
sur une coupure **réelle**. Ce maillon-là, seul un test eval le voit — c'est
précisément ce que le doc-comment de `test_deadline_verdict_2276.rs` dit de son
propre maillon (*« le maillon que les unités ne peuvent pas voir »*).

**Vol 2** livre `tests/eval/test_webhook_max_steps_verdict_2943.rs` : la chaîne
complète, `run_agent` réel → 21 réponses `tool_call` sans EndTurn → champ posé →
filet → POST capturé, plus le contrôle négatif et le cas du build en vol de bout
en bout.
`Tracked in: senara-solutions/mika#2943 — vol 2/2`

## Ordre de commit

1. **R1** — le prédicat et ses tests. Autonome, aucun appelant encore : vert seul.
2. **R2** — le type, le champ, le site de pose. Le dépôt compile ; le champ est
   posé et personne ne le lit encore. Les trois `None` des autres constructions
   sont dans ce commit, faute de quoi il ne compile pas.
3. **R3** — la conclusion, le motif, le corps, le bras de `deadline_verdict_target`.
   `TurnConclusion::from` disparaît ici, donc **R4 et la ligne de
   `test_error_verdict_2289.rs` sont dans ce même commit** : un `impl From`
   retiré sans ses deux appelants ne compile pas.
4. **R5** — les détecteurs, en dernier, sur un arbre où le comportement qu'ils
   mesurent existe. Armés et verts, allowlist vide.

L'ordre 3-avant-4 n'existe pas : c'est un seul commit, et c'est le compilateur
qui l'impose. L'ordre R5-en-dernier est volontaire, pour la raison de la section
`## Fire-Disposition`.

## Sonde post-déploiement (AC4)

À exécuter **après `make deploy`**, sur l'hôte. La sonde est une **jointure par
`trace_id`** : le ticket mesure un défaut dont la forme est « la trace existe,
mais rien d'autre ne la porte ».

```sh
# 1. Les tours webhook mika-qa coupés au plafond depuis le déploiement.
grep 'max_steps_tool_census' /var/log/mika/server.log | tail -20

# 2. Pour chaque trace de (1), le filet doit avoir laissé une ligne.
grep -E 'qa_deadline_verdict|qa_max_steps_verdict_withheld' /var/log/mika/server.log | tail -20
```

**Régime attendu.** Toute trace de (1) qui est un tour webhook de mika-qa sur un
événement de PR porte, en (2), **exactement une** des trois lignes suivantes :

| ligne | lecture |
|---|---|
| `qa_deadline_verdict` + `cause=cut_off_max_steps` + `outcome=posted` | AC1 — le filet a posé le `hold[review]` |
| `qa_deadline_verdict` + `cause=cut_off_max_steps` + `outcome=already_reviewed` | AC2 — la session avait déjà revu |
| `qa_max_steps_verdict_withheld` + `cause=build_callback_pending` | AC3 — abstention, le callback portera le verdict |

**Ce qui fait rougir la sonde** : une trace de (1) sur une PR qui ne porte
**aucune** des trois lignes. C'est le symptôme d'origine, et il est alors
toujours là.

**Ce qui n'est pas un rouge** : un `outcome=no_token` ou un `outcome=failed` — le
filet a décidé et l'a dit ; la panne est à la résolution d'identité ou chez
GitHub, pas dans ce correctif. Ni un `qa_max_steps_verdict_withheld` sur une
trace non-PR : la boucle ne sait pas si le tour portait sur une PR, elle dit
seulement ce qu'elle sait, et c'est la lecture honnête de ce site.
