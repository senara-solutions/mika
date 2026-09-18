---
issue: 2368
type: fix
module: server/deadline_verdict, skills/executor, task_engine/dispatcher, agent_loop
tags: [loop-breaker, qa-gate, callback, substrate, mika-2355, mika-2276, filet]
---

# mika#2368 — le filet moteur : une PR de la boucle n'est plus jamais muette

## Le défaut que ce travail ferme

mika#2355 (PR #2359) a livré trois briques sur le flux `long_running:build_mika` :
B1 (le prompt de reprise devient atteignable), B2 (le contrat terminal self_dev est
retiré de ce flux) et la **garde positive** `qa_build_callback_verdict` — un callback
de build QA qui atteint EndTurn sans `run_gh pr review` réussi est re-prompté.

Une brique du même plan (§ B3, AC5–AC7) n'a pas été livrée : le filet moteur. Et le
trou qu'elle laisse est écrit, mot pour mot, dans le test que #2359 a lui-même posé :

> `crates/mika-agent/tests/eval/test_qa_build_callback_verdict_2355.rs::the_verdict_guard_fires_once_and_does_not_loop`
> — après le re-prompt unique, un second EndTurn sans revue est **accepté** et
> `run_gh` n'a pas été appelé. Le test le dit en toutes lettres :
> *« nothing was posted — the net's job »*.

Ce n'est pas un défaut de la garde : c'est son contrat. Toutes les gardes
`intent_guard_retries` ont un budget d'un coup, délibérément — une garde qui
re-prompte indéfiniment transforme un silence en boucle. Le budget épuisé, il reste
une injonction que le modèle a ignorée deux fois, et une PR muette.

C'est exactement la doctrine que ce dépôt applique déjà
(`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, mesurée par
mika#2120 : neuf récurrences sous enforcement par prompt contre zéro quand
l'opérateur écrivait la consigne à la main), et que le module qu'on vient
généraliser énonce en propres termes :

> `crates/mika-agent/src/server/deadline_verdict.rs:26` — *« M1 sans ce filet laisse
> la prochaine cause de dépassement muette. »*

## Ce que la lecture du code établit, avant d'écrire une ligne

Huit faits, tous vérifiés dans l'arbre **post-merge de #2359**, qui contraignent la
forme du correctif. F8 est celui que la relecture a ajouté, et il change une
prescription de conception — voir C4.

**(F1) La porte d'entrée du filet existant refuse précisément notre cas.**
`deadline_verdict.rs:231` :

```rust
let Some(overrun) = input.overrun else {
    return DeadlineVerdictOutcome::NotApplicable("turn_completed");
};
```

`overrun` vaut `None` exactement quand le tour s'est terminé proprement,
c'est-à-dire dans **tout** le périmètre de ce ticket. Appeler la fonction telle
quelle rendrait `NotApplicable` à chaque fois. Le motif doit devenir énuméré.

**(F2) Le corps et la ligne de journal mentent sur notre chemin.**
`build_verdict_body` (`deadline_verdict.rs:171`) écrit *« Le tour de revue QA a
atteint la limite de son enveloppe de temps »* et *« il s'est arrêté après {steps}
step(s) d'outil »* ; le `warn!` de succès dit *« tour de revue coupé par sa deadline
— verdict hold[review] posté par le moteur »*. Sur un tour qui a conclu sans poster,
les deux sont faux. Le corps et la ligne dépendent donc du motif, pas seulement le
déclenchement.

**(F3) `build_callback_task` a une signature unique et cinq appelants de production.**
`skills/executor.rs:2963` (la définition, `metadata: None` en dur ligne 3004), et les
appelants : `executor.rs:3175`, `server/ready_label_handler.rs:700`,
`task_engine/dispatcher.rs:3134`, `task_engine/dispatcher.rs:5881`,
`server/verdict_handler.rs:833`. Plus quatre tests d'eval qui l'appellent
**délibérément** (`test_reaper_liveness_all_surfaces_2277.rs:218`,
`test_reaper_reaps_live_pending_pilot_2272.rs:217`, et deux qui documentent sa forme)
— mika#2272 a payé le prix d'une fixture qui écrivait un statut que la production
n'écrit pas, et le doc-comment de la fonction (`executor.rs:2948-2962`) interdit
explicitement la dérive entre sites de construction.

**(F4) Le texte de l'événement d'origine est déjà porté jusqu'au site de
construction.** `LongRunningContext.originating_message: Option<String>`
(`executor.rs:370`), peuplé depuis le dernier message user en mode conversation
(mika#933), est dans la portée d'`execute_long_running` au moment exact où
`build_callback_task` est appelé (`executor.rs:3101` le lit déjà pour un autre
garde). Le tour QA qui lance `build_mika` est un tour conversationnel déclenché par
un webhook PR, donc `originating_message` porte le texte que `parse_pr_target` sait
lire.

**(F5) Le chemin silencieux ne porte pas le registre anti-double-post, et le code
dit déjà que c'est une anomalie.** `agent_loop/mod.rs:4994` :
`pr_reviews_posted: None, // Silent mode: no session-scoped dedup needed`. Or
`skills/builtin_handlers.rs:2924` porte :

```rust
debug_assert!(
    ctx.pr_reviews_posted.is_some(),
    "pr_reviews_posted must be threaded for production pr review calls"
);
```

Un callback QA qui poste sa revue est un « production pr review call ». Le
commentaire de la ligne 4867 et cette assertion se contredisent depuis que le
callback de build est devenu un flux qui poste des revues. AC7 corrige une
incohérence déjà déclarée dans le source, elle n'en introduit pas.

**(F6) Le dispatcher tient déjà tout ce qu'il faut pour exécuter le POST.**
`TaskDispatcher` porte `settings: Settings` (`dispatcher.rs:312`),
`github_app` (`:300`) et `pr_reviews_posted` (`:315`) ; `resolve_periodic_scan_token`
(`:112`) est le convertisseur PAT-first déjà utilisé sur ce fichier. Et il évince le
registre par `session_id` **après** le run (`dispatcher.rs:911-913`) — le filet doit
donc être appelé avant cette éviction.

**(F7) `run_silent_agent` retourne `Result<()>` et le signal ne remonte pas.**
`agent_loop/mod.rs:4524`. Les tool summaries du tour vivent dans `run_loop` et
n'en sortent pas. Le call-site du filet (le dispatcher) ne peut donc pas savoir, en
l'état, qu'un verdict était dû et n'a pas été posté.

**(F8) La garde de #2359 a DEUX sites d'enforcement, et le second est précisément
notre population.** `agent_loop/mod.rs:2317` (chemin texte non vide) et `:3146`
(miroir sur la sortie à texte vide). Les deux portent la même conjonction
(`EndTurn` + `qa_verdict_due` + budget non consommé + `!pr_review_posted_in_turn`).
Le commentaire du second le décrit lui-même :

> *« a bare EndTurn is exactly the shape a turn that has nothing to say takes, and
> it is the one the registry never sees. »*

Un tour de callback qui conclut sans rien dire **est** le cas nominal de ce ticket.
Un signal posé au seul site texte-non-vide laisserait le filet aveugle sur la moitié
la plus probable de sa population — et rien ne le signalerait : le filet resterait
silencieux, ce qui est indistinguable d'un filet qui n'a rien à faire.

Second point, structurel : **le site où le budget est « déjà consommé » n'existe pas
comme branche.** La condition `!intent_guard_retries.contains(…)` est *dans* le `if`
du re-prompt ; budget épuisé, on tombe à travers le `if`, sans `else`. Le signal ne
se pose donc pas « dans la garde » mais sur les deux chemins de sortie, sous le
prédicat complémentaire — voir C4.

## Précondition : levée, et les points d'accroche re-vérifiés

**PR #2359 (mika#2355) est mergée** — `5091b525`, ancêtre d'`origin/main`, et
`crates/mika-agent/src/qa_build_callback.rs` est dans l'arbre. La rédaction
précédente de ce plan affirmait le contraire ; c'était vrai à l'heure où elle a été
écrite et ne l'est plus. Le ticket est actionnable sans attendre.

Ce plan prescrivait de relire ses trois points d'accroche si #2359 bougeait. Fait,
sur l'arbre mergé :

| point d'accroche | état |
|---|---|
| `qa_build_callback::pr_review_posted_in_turn` | présent (`qa_build_callback.rs:112`), signature `(&[ToolCallSummary]) -> bool` |
| `qa_build_callback::qa_verdict_required` | présent (`:93`), conjonctif (message **et** skills chargées) |
| le point où la garde consomme son budget | **deux sites, pas un** — voir F8 ci-dessous, c'est la correction de fond de cette relecture |

L'implémentation ne réécrit aucun des trois : les recopier serait exactement la
classe de duplication que `qa_build_callback.rs` a été créé pour fermer (voir son
doc-comment, qui cite mika#2158).

## Conception

Le filet réutilise `server::deadline_verdict` en le **généralisant** — même forme,
même registre anti-double-post, même traitement idempotent du 422. Cinq pièces.

### C1 — le motif devient énuméré

`DeadlineVerdictInput.overrun: Option<DeadlineOverrun>` devient
`reason: VerdictReason`, un enum à deux variantes :

```rust
pub enum VerdictReason {
    /// mika#2276 — le tour a été coupé par son enveloppe.
    CutOffByDeadline(crate::agent_loop::DeadlineOverrun),
    /// mika#2368 — le tour de callback de build QA a conclu sans poster de verdict.
    CallbackConcludedWithoutVerdict,
}
```

Ce qui dépend du motif : **le corps posté**, **la ligne de journal**, et **le nom
d'événement** (voir C5). Ce qui n'en dépend pas : la résolution de cible, le registre
anti-double-post, la classification du 422, la discipline « ne jamais rendre
d'erreur ». La garde d'entrée `NotApplicable("turn_completed")` disparaît — elle
était l'expression, dans le type `Option`, du fait qu'il n'y avait qu'un motif.

Le call-site existant (`server/handlers.rs:1151`, helper
`post_deadline_verdict_if_cut_off`) suit mécaniquement :
`overrun: output.deadline_exceeded` devient une construction de
`VerdictReason::CutOffByDeadline`, et son test de sortie précoce
(`handlers.rs:1131`) reste identique.

**AC5c est la contrainte la plus dure de cette pièce** : sur un tour coupé par sa
deadline, le corps posté, la ligne de journal, le nom d'événement et le registre
doivent être **identiques à avant**, octet pour octet sur le corps. Les tests
mika#2276 existants (`deadline_verdict.rs` tests + `test_deadline_verdict_2276.rs`)
sont le gabarit : ils doivent passer sans qu'aucune assertion soit affaiblie.

### C2 — la cible PR est dite, jamais dérivée par le filet

`DeadlineVerdictInput` gagne la possibilité de recevoir une cible **déjà résolue**
plutôt qu'un `event_text` à parser. La forme retenue : remplacer
`event_text: &'a str` par un `target: PrTarget` que l'appelant fournit, et laisser
`parse_pr_target` public pour les appelants qui, comme `handlers.rs`, ont un texte
d'événement sous la main.

Le stamp, côté producteur : `execute_long_running` (`executor.rs`, juste avant
l'appel à `build_callback_task` ligne 3175) résout la cible depuis
`ctx.originating_message` via `deadline_verdict::parse_pr_target` — **le lecteur
unique de cette grammaire, jamais une regex recopiée** — et la stampe dans le
`metadata` de la tâche callback, sous une clé nommée par une constante
(`QA_REVIEW_PR_TARGET_KEY`), à la manière de `metadata.dispatch_worktree_file`
(mika#2249) et `metadata.pilot_transcript_expected` (mika#2040).

**Pourquoi résoudre au stamp et pas plus tôt.** Deux lectures de « dite, jamais
dérivée » étaient ouvertes. (a) Porter une `PrTarget` résolue depuis
`run_agent_for_message` jusqu'au `ToolContext` puis au `LongRunningContext` : le plus
littéral, au prix d'un champ de plus sur trois structs et sur tous leurs sites de
construction — une trentaine, presque tous sans rapport avec le flux QA. (b) La
résoudre au site du stamp depuis `originating_message`, qui est déjà là (F4). Ce qui
est condamné par le ticket, c'est la dérivation **tardive** — celle qui se fait au
moment du filet, quand l'échec n'est plus rattrapable et ne se journalise nulle part.
Ici la résolution a lieu au moment du spawn, son échec est journalisé sur-le-champ,
et **le filet, lui, ne parse rien** : il lit un stamp. C'est la même trajectoire que
les deux précédents cités, où la valeur est également calculée par le producteur au
moment du spawn. Le coût de (b), nommé : si `originating_message` était un jour
peuplé autrement, le stamp deviendrait faux — d'où l'assertion de forme dans les
tests (§ Tests, T5).

**Fail-safe dans le sens de la maison (AC6).** Pas de `metadata`, pas de clé, clé
illisible, cible non parsable, `originating_message` absent → **zéro POST**, et une
ligne nommant l'abstention et son motif. Le filet ne devine jamais une PR. Un signal
qu'on ne peut pas lire n'est jamais un terme satisfait.

### C3 — un paramètre de plus sur la signature unique, jamais un second constructeur

`build_callback_task` gagne un paramètre `metadata: Option<String>` en dernière
position, qui remplace le `None` en dur de la ligne 3004. **Sept** sites d'appel à
toucher, six passent `None` :

| site | valeur |
|---|---|
| `executor.rs:3175` (`execute_long_running`) | le stamp, quand la cible se résout |
| `ready_label_handler.rs:700` | `None` |
| `dispatcher.rs:3134` | `None` |
| `dispatcher.rs:5881` | `None` |
| `verdict_handler.rs:833` | `None` |
| `test_reaper_liveness_all_surfaces_2277.rs:218` | `None` |
| `test_reaper_reaps_live_pending_pilot_2272.rs:217` | `None` |

Deux autres fichiers d'eval (`test_ready_label_live_pilot_noop_2279.rs`,
`test_supersede_kills_live_pilot.rs`) **nomment** la fonction en doc-comment pour
décrire la forme de la row qu'ils fabriquent, sans l'appeler : ils ne compilent pas
contre la signature et ne sont pas à toucher. La distinction vaut d'être écrite —
elle change ce qu'un `cargo build` cassé signifie.

Un second constructeur est explicitement exclu : le doc-comment de la fonction
interdit la dérive entre sites de construction, et c'est la classe de bug que sa
consolidation a fermée.

### C4 — le tour silencieux doit pouvoir dire ce qu'il a fait, et ce qu'il n'a pas fait

Deux propagations, de sens inverse.

**Vers le bas — le registre (AC7).** `SilentAgentParams` gagne
`pr_reviews_posted: Option<&'a Arc<DashMap<String, HashSet<String>>>>` ;
`agent_loop/mod.rs:4867` le lit au lieu de poser `None`, et le commentaire qui
affirme le contraire est corrigé. Les cinq constructions de `SilentAgentParams` dans
`dispatcher.rs` (`:517`, `:729`, `:1135`, `:1545`, `:2310`) passent
`self.pr_reviews_posted.as_ref()` ; les sites hors dispatcher passent `None`. La
session du callback est neuve, donc le registre porte exactement ce que **ce tour-là**
a posté — c'est la granularité voulue, et c'est ce qui rend AC7 vrai sans ajouter
d'état.

**Vers le haut — le fait que le verdict était dû et n'a pas été posé.** `run_loop`
gagne un out-param `qa_verdict_unmet: Option<&AtomicBool>`, posé aux **deux** sites
de sortie que F8 établit — texte non vide (`:2317`) et miroir texte vide (`:3146`) —
sous le prédicat complémentaire de la garde : `qa_verdict_due` **et**
`!pr_review_posted_in_turn(&all_tool_summaries)` **et** budget déjà consommé
(`intent_guard_retries.contains(QA_VERDICT_REQUIRED_LABEL)`), immédiatement après le
`if` du re-prompt, sur le chemin où l'EndTurn est accepté. `run_silent_inner` le lit
après `run_loop` et le rend dans la valeur de retour ; `run_silent_agent` passe de
`Result<()>` à `Result<SilentTurnOutcome>` avec un seul champ pour l'instant.

**Deux sites et non un — c'est la correction que la relecture post-merge a apportée,
et elle n'est pas cosmétique.** Le miroir texte vide est la forme que prend « un tour
qui n'a rien à dire », c'est-à-dire la moitié la plus probable de la population visée.
Le couvrir à moitié produirait un filet silencieux, indistinguable d'un filet qui n'a
rien à faire — exactement le mode de panne que ce ticket existe pour fermer. D'où
**T10** (§ Tests) : un contrôle positif par site, et non un test qui n'exercerait que
le chemin texte non vide.

Le prédicat lui-même n'est pas recopié : il est extrait dans
`qa_build_callback::verdict_unmet_after_retry(qa_verdict_due, retries, summaries)`,
appelé aux deux sites. Deux copies d'une conjonction à trois termes divergent — c'est
la leçon que `grooming_marker` a dû engraver une fois (mika#2158), et les deux sites
de la garde #2359 sont déjà une duplication qu'on n'aggrave pas.

Trois choix, et leurs raisons :

- **Pas une variante de `LoopResult`.** L'enum à trois variantes sans
  `#[non_exhaustive]` est un contrat : son exhaustivité force les trois handlers
  externes à traiter chaque mode de terminaison. « Le tour a conclu sans poster »
  n'est pas un mode de terminaison alternatif — un tour peut être `Done` *et* muet.
- **Pas un champ sur `ToolContext`.** C'est la surface des outils, pas celle du loop.
- **Un out-param par référence** est le motif déjà employé dans ce fichier
  (`pr_review_posted`, `tool_arg_suffix_rejected`, `skills_dirty`).

Le changement de type de retour est peu coûteux : quatre des cinq appelants écrivent
`if let Err(e) = run_silent_agent(...)`, qui compile inchangé quel que soit le type
du `Ok`. Seul `dispatcher.rs:1567` (`match ... { Ok(()) => ... }`, la réflexion) a un
binding à ajuster.

### C5 — deux motifs, deux noms d'événement (et c'est ce qui sauve les sondes)

`DEADLINE_VERDICT_EVENT = "qa_deadline_verdict"` reste attaché au motif
`CutOffByDeadline`. Le motif `CallbackConcludedWithoutVerdict` écrit sous un nom
**distinct** : `qa_callback_verdict`.

Ce n'est pas du confort de nommage. La sonde de contrôle négatif du plan mika#2355
est littéralement :

> `grep qa_deadline_verdict $MIKA_SPIRIT_LOG_FILE | jq 'select(.outcome == "posted")'`
> — le filet mika#2276 ne doit pas se mettre à firer.

Un nom partagé fusionnerait deux populations qui doivent rester comptables
séparément, et cette sonde ne dirait plus rien : un `posted` du nouveau motif se
lirait comme une régression de l'ancien. Même principe que les paires
`phantom_aged_out` / `phantom_sweep_spared` (mika#2156), `auto_pull_no_token` /
`wip_rescue_no_token` (mika#2205), et `wip_rescue_bailed` /
`wip_rescue_parked_unverified` (mika#2286) : *deux noms distincts pour que le grep
discrimine.*

Chacun est **SOLE WRITER** de son nom, en journal comme en `audit_events`.

### Le token du filet : `hold[review]` et rien d'autre

Contrainte de sûreté, pas de registre. `server/verdict_handler.rs` intercepte
`pull_request_review.submitted` **avant** le tour LLM et route déterministement sur
la ligne `VERDICT:` : `pass` → **merge** via `pr_merge_with_gate` ;
`block[ac]`/`block[ci]` → dispatch d'un pilote de correction ; `hold[review]` →
notifier l'opérateur, laisser la tâche `in_progress` (`verdict_handler.rs:1583`).

Un filet qui poserait `pass` au motif que *le build a réussi* **mergerait une PR dont
aucun diff n'a été revu**. C'est un dégât strictement pire que le silence qu'il
remplace. `DEADLINE_VERDICT_LINE` reste la seule ligne que le filet sait écrire, et
l'assertion porte sur la constante **et** sur le parseur (AC5b).

Corollaire positif, qui est la raison d'être du filet : le verdict n'est pas
décoratif. Il est lu par la machine d'état, qui sort la PR du silence et la remet
devant un humain. C'est ce qui en fait un filet et non une trace.

### Câblage et kill-switch

Le filet est appelé depuis `dispatch_resume_agent` (`dispatcher.rs`), sur la branche
`else if is_callback` où `run_silent_agent` a rendu `Ok`, **avant** l'éviction du
registre (`:911-913`). Il y résout le token par `resolve_periodic_scan_token`
(PAT-first, ADR-008 : poster une review est une opération dont GitHub lit l'auteur —
le verdict doit apparaître sous l'identité machine de la QA) et injecte
`run_gh_subprocess` comme `poster`, exactement comme `handlers.rs:1160`.

`MIKA_QA_CALLBACK_VERDICT_NET` (défaut armé ; `0` désarme sans redéploiement).
Il n'est pas là par prudence de principe : la sonde 2c du plan mika#2355 prescrit
« désarmer le filet avant tout diagnostic » en cas de PR mergée sans revue — une
prescription qui n'est exécutable que si le levier existe. Trois paliers habituels de
la maison pour la lecture ; absence → armé.

Le filet ne rend jamais d'erreur et ne fait jamais échouer le tick : un filet qui
tombe remplacerait un silence par une panne.

## Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/src/server/deadline_verdict.rs` | C1 (motif énuméré, corps et journal par motif), C2 (cible reçue résolue), C5 (deux noms d'événement) |
| `crates/mika-agent/src/server/handlers.rs` | suit C1 au call-site mika#2276 (`post_deadline_verdict_if_cut_off`) |
| `crates/mika-agent/src/skills/executor.rs` | C2 (résolution + stamp), C3 (paramètre `metadata`) |
| `crates/mika-agent/src/server/ready_label_handler.rs` | C3 (`None`) |
| `crates/mika-agent/src/server/verdict_handler.rs` | C3 (`None`) |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | C3 (`None` ×2), C4 (registre ×5), câblage du filet + kill-switch |
| `crates/mika-agent/src/agent_loop/mod.rs` | C4 (registre en silent, out-param `run_loop` posé aux **deux** sites de sortie `:2317`/`:3146`, `SilentTurnOutcome`), commentaire `:4994` corrigé |
| `crates/mika-agent/src/qa_build_callback.rs` | `verdict_unmet_after_retry` (le prédicat complémentaire, lecteur unique) ; doc-comment `:38` : le filet n'est plus « à venir » |
| 2 tests d'eval appelant `build_callback_task` (#2272, #2277) | C3 (`None`) |
| `crates/mika-agent/CLAUDE.md`, `CLAUDE.md` racine | § Deadline Verdict Net généralisée ; env var ; signaux opérateur |

## Tests

Tous rouge-avant. Le fichier neuf : `crates/mika-agent/tests/eval/test_qa_callback_verdict_net_2368.rs`.

- **T1 (AC5) — le contrat central.** Un tour de callback de build QA qui conclut sans
  verdict, avec une cible stampée, produit **exactement un** POST `pr review` dont le
  corps s'ouvre sur `VERDICT: hold[review]`. `poster` injecté et compteur d'appels,
  comme `deadline_verdict::tests::deadline_on_a_pr_turn_posts_a_verdict`.
- **T2 (AC5b) — le filet ne peut pas merger.** Sur **tous** les chemins du filet et
  pour les deux motifs : le corps ne contient ni `VERDICT: pass`, ni `block[ac]`, ni
  `block[ci]`, ni `block[security]`, ni `block[pipeline]`. Asserté sur la constante
  `DEADLINE_VERDICT_LINE` **et** en passant le corps produit au parseur de
  `server::verdict` — la constante seule ne prouve pas ce que la machine d'état lira.
- **T3 (AC5c) — mika#2276 figé.** Le motif `CutOffByDeadline` produit un corps
  **identique octet pour octet** à celui d'avant le correctif (chaîne attendue en
  dur dans le test, pas re-générée par le code sous test), la même ligne de journal
  et le même nom d'événement. Les tests mika#2276 existants passent sans qu'aucune
  assertion soit affaiblie.
- **T4 (AC6) — fail-safe, quatre contrôles négatifs séparés.** `metadata` absent ;
  `metadata` présent sans la clé ; clé présente mais illisible ; cible non parsable
  au stamp. Chacun : **zéro** POST et une ligne d'abstention nommant son motif.
  Quatre tests et non un seul : une conjonction de termes fail-safe ne se prouve pas
  en les neutralisant tous à la fois — un test unique passerait sur une
  implémentation qui n'en lit qu'un (la leçon de mika#2277, dont les quatre négatifs
  à un terme sont le gabarit).
- **T5 (C2) — la forme du stamp est celle que la production écrit.** La cible stampée
  est produite par le **chemin d'écriture de production** (`build_callback_task` +
  `create_task`), jamais par un `NewTask` écrit à la main — mika#2272 :
  *« la fixture doit venir du site de construction de production ou elle se mesure
  elle-même »*. Et le test reconstruit la cible attendue par `parse_pr_target` sur un
  texte d'événement réel, de sorte qu'un changement de `originating_message` rougisse
  ici plutôt que de désarmer le filet en silence.
- **T6 (AC7) — anti-double-post.** Un tour de callback qui **a** posté son verdict ne
  reçoit pas de verdict de secours. Deux formes de clé, comme
  `deadline_verdict::tests` : `{repo}|{n}` et `__default__|{n}` (l'appel `gh pr review`
  sans `--repo`). Manquer la seconde ferait re-poster le filet sur une PR déjà revue.
- **T7 (C4) — le registre atteint bien le tour silencieux.** Sur le chemin de
  production (`run_silent_agent` piloté comme dans
  `test_qa_build_callback_verdict_2355.rs`), un `run_gh pr review` réussi inscrit sa
  clé dans le registre passé par le dispatcher. Sans ce test, AC7 tiendrait dans le
  filet et serait faux dans la vraie vie.
- **T8 (C5) — les deux noms sont distincts et chacun a un seul écrivain.** Scan de
  source : `qa_deadline_verdict` et `qa_callback_verdict` sont écrits chacun à un
  seul site. Un test comportemental ne peut pas voir cette classe — un second
  écrivain ne rendrait aucune décision fausse, il rendrait les deux populations
  inséparables, et toutes les assertions resteraient vertes pendant que les sondes
  cesseraient de discriminer.
- **T9 (kill-switch)** — désarmé, le filet ne poste rien et le dit ; armé par défaut
  en l'absence de variable.
- **T10 (F8/C4) — un contrôle positif PAR SITE DE SORTIE.** Deux tests distincts
  pilotant `run_silent_agent` comme le fait
  `test_qa_build_callback_verdict_2355.rs` : (a) second EndTurn **avec** texte, (b)
  second EndTurn **à texte vide**. Chacun doit produire son POST. Deux tests et non
  un paramétré sur la forme du texte : c'est la seule construction qui rougisse quand
  un seul des deux sites est câblé, et le site vide est le plus probable en
  production. Contrôle négatif commun : un tour dont le budget n'est **pas** consommé
  (la garde re-prompte) ne doit pas armer le signal — sinon le filet doublerait le
  re-prompt au lieu de lui succéder.

**Fire-Disposition (mika#1574).** Population pré-existante : **vide par
construction** pour chacun des dix détecteurs. T1/T2/T4/T6/T7/T9/T10 portent sur du
code qui n'existe pas encore ; T3 fige un comportement en place et ne peut donc pas
révéler d'arriéré ; T5 et T8 scannent des sites que ce ticket crée. Aucune
disposition à prendre, et l'énoncé de cette absence est ce qui la distingue d'un
oubli.

## Definition of Done

- C1–C5 livrées ; `cargo test -p mika-agent` vert ; `cargo clippy` propre ;
  `cargo fmt` appliqué.
- Les tests mika#2276 (`deadline_verdict.rs` + `test_deadline_verdict_2276.rs`)
  passent sans qu'aucune assertion ait été affaiblie ou supprimée.
- `crates/mika-agent/CLAUDE.md` § *Deadline Verdict Net* réécrite pour décrire les
  deux motifs ; `CLAUDE.md` racine : `MIKA_QA_CALLBACK_VERDICT_NET` et les signaux
  opérateur des deux noms d'événement.
- Le doc-comment de `qa_build_callback.rs` qui annonce ce filet comme « à venir sous
  mika#2368 » est corrigé — il devient faux le jour où ceci merge.

## Acceptance criteria

Reprises verbatim du ticket mika#2368, elles-mêmes reprises du plan mika#2355 § B3.

- **AC5** — Un tour de callback de build QA terminé sans verdict, avec une cible PR
  stampée, produit exactement **un** POST `pr review` portant `VERDICT: hold[review]`.
- **AC5b** — Le filet ne peut produire ni `pass` ni `block[*]`. Asserté sur
  `DEADLINE_VERDICT_LINE` **et** sur le parseur.
- **AC5c** — Le motif `CutOffByDeadline` conserve mika#2276 à l'identique : corps
  posté, ligne de journal, nom d'événement et registre anti-double-post inchangés.
- **AC6** — Fail-safe : stamp absent, illisible, ou cible non résoluble ⇒ **zéro**
  POST et une ligne de journal nommant l'abstention et son motif.
- **AC7** — Anti-double-post : un tour de callback qui a posté son propre verdict
  n'en reçoit pas un second (le registre `pr_reviews_posted` est propagé au chemin
  silencieux).

## Sondes post-déploiement, et leurs haltes

- **Sonde 1 (symptôme, 48 h).** Toute PR de la boucle ayant reçu une demande de revue
  `mika-platform-qa` porte une revue soumise. Contrôle direct du loop-breaker.
- **Sonde 2 (attribution).**
  `SELECT count(*) FROM audit_events WHERE tool_name = 'qa_callback_verdict';`
  — **doit rester proche de zéro.** Un filet qui porte le trafic nominal a remplacé
  un silence par un `hold[review]` systématique : B1/B2/la garde de #2359 n'ont alors
  pas pris, et c'est **là** qu'il faut chercher, pas dans le réglage du filet. *Ce
  filet est un filet, pas un chemin* — s'il porte le trafic nominal, il a en plus
  effacé le signal qui permettrait de le voir.
- **Sonde 2c (aucun merge non revu).** Sur les premières PR touchées, aucune n'est
  mergée sans revue soumise portant une `DIFF ANALYSIS`. `verdict_handler` merge sur
  `pass`, donc une régression du corps du filet se lirait en production comme des PR
  qui passent toutes seules. **Halte immédiate** si une PR est mergée sans revue :
  `MIKA_QA_CALLBACK_VERDICT_NET=0` d'abord, diagnostic ensuite.
- **Sonde 3 (contrôle négatif, mika#2276).**
  `grep qa_deadline_verdict $MIKA_SPIRIT_LOG_FILE | jq 'select(.outcome == "posted")'`
  — le filet mika#2276 ne doit pas se mettre à firer. Ce correctif ne rallonge aucun
  tour. Cette sonde n'a de sens que grâce à C5 ; si elle se met à compter des
  `posted` du nouveau motif, c'est que les deux noms ont fusionné.
- **Sonde 4 (abstentions).** `grep qa_callback_verdict $MIKA_SPIRIT_LOG_FILE | jq
  'select(.outcome | startswith("no_"))'` — une abstention soutenue sur le même motif
  (`no_target_stamp` en particulier) signifie que la trajectoire du stamp est cassée
  en amont, pas que le filet est trop strict. **Ne pas élargir la résolution de
  cible** : établir d'abord pourquoi `originating_message` ne porte plus le webhook.
- **Halte générale.** Si les PR restent muettes alors que les sondes 2 et 4 sont
  propres, c'est que le tour de callback n'a pas lieu du tout — un autre défaut. Le
  discriminer en une commande :
  `grep 'resuming agent for callback' $MIKA_SPIRIT_LOG_FILE` filtré sur le label
  `long_running:build_mika`. Aucune ligne ⇒ la piste est la livraison des callbacks
  (quarantaine mika#2179, file), pas le contrat de reprise.

## Rollback

Revert du commit : le filet cesse de poster, `deadline_verdict` retrouve son motif
unique, le registre redevient `None` en silent. C'est l'état d'avant, sans autre
changement de comportement. Désarmement seul, sans redéploiement :
`MIKA_QA_CALLBACK_VERDICT_NET=0`.

## Hors périmètre, délibérément

- **Le tour de callback qui échoue (`run_silent_agent` rend `Err`).** Il laisse aussi
  la PR muette, mais son silence n'est pas définitif de la même façon : la livraison
  est réessayée puis quarantainée avec bornes et télémétrie (mika#2179). Le signal
  `concluded_without_verdict` n'existe que sur la branche `Ok` — un tour qui a planté
  n'a pas « conclu ». Population nommée, non couverte, **ticket de suivi** si les
  sondes montrent qu'elle est non négligeable.
- **Les quatre autres flux `long_running`** (`run_claude_pilot`,
  `run_claude_pilot_groom`, `deploy_mika`, `address_pr_comments`,
  `resolve_pr_conflicts`) : ni leur framing, ni leur garde, ni leur contrat terminal
  ne bougent. Le discriminant de flux est `qa_build_callback::qa_verdict_required`,
  dont la conjonction exclut chacun d'eux (mika#2355 AC9).
- **`self-dev-callback` inatteignable** (mika#2355 F1) : contrat porté en dur par le
  moteur, ticket de suivi propre, déjà compté par le détecteur d'atteignabilité.
- **Le trou de rattrapage mika#2334** (une PR portant une demande de revue en place
  n'est jamais re-demandée) : ce filet le rend sans objet pour cette classe, mais
  l'élargir demande son propre arbitrage — une demande en place pendant qu'une revue
  tourne est le cas nominal. **Ticket de suivi.**
- **Le littéral `120s` du rail Anthropic, le volume du prompt QA, la cause des
  timeouts** : sans rapport.
