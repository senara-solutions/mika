# mika#2943 — l'attestation de bout en bout du filet de coupure par pas

> **Vol 2/2.** Le vol 1 a livré le code de production du filet et ses tests
> unitaires ; il est mergé (`e2e9774d1`, PR #2945, 2026-10-28T11:48:59Z). Ce
> plan livre **ce que le vol 1 a nommé et renvoyé** : l'attestation de bout en
> bout des trois décisions du filet — abstention sur revue déjà postée (AC2),
> absence de cible sur un tour non-PR (AC2), abstention sur build en vol (AC3) —
> à travers un `run_agent` réel, et non sur un `TurnConclusion` fabriqué.
> Aucune ligne de production nouvelle. Dernier vol : il n'y a pas de
> `Tracked in:` de suite.

## Contexte

### Ce que le vol 1 a posé, vérifié sur `HEAD` (`7df2c1e1c`)

| livrable du vol 1 | site | état |
|---|---|---|
| le prédicat « un callback de build portera le verdict » | `qa_build_callback::build_callback_dispatched_in_turn` (`qa_build_callback.rs:137`) | présent |
| le fait de coupure dans la sortie du tour | `AgentOutput::max_steps_exceeded: Option<MaxStepsCutOff>` (`agent_loop/mod.rs:439`), posé au seul site `agent_loop/mod.rs:7022` | présent |
| l'abstention AC3 et sa ligne de décision | `qa_max_steps_verdict_withheld`, `cause=build_callback_pending` (`agent_loop/mod.rs:6960-6973`) | présent |
| le convertisseur **total** sur les deux bornes | `TurnConclusion::of_output` (`deadline_verdict.rs:201`) ; `TurnConclusion::from` retiré | présent |
| le motif et son corps | `VerdictReason::WebhookCutOffByMaxSteps` (`deadline_verdict.rs:303`), événement `qa_deadline_verdict`, `cause=cut_off_max_steps` | présent |
| le bras du filet | `deadline_verdict_target`, `match` exhaustif sans bras `_` (`deadline_verdict.rs:465`) | présent |
| 6 contrôles unitaires `mika2943_*` | `deadline_verdict.rs::tests` | présents, verts |
| **1** contrôle de bout en bout (AC1) | `tests/eval/test_max_steps_verdict_2943.rs:193`, `mika2943_un_tour_webhook_coupe_par_ses_pas_ne_laisse_jamais_la_pr_muette` | présent, vert |

Le défaut d'origine est donc **corrigé**, et son chemin nominal est attesté de
bout en bout : le vol 1 devait un contrôle négatif (règle 13, mika#2264) et l'a
payé avec le cas AC1 du vol 2. Ce que le vol 1 n'a pas payé, et qu'il nomme en
propres termes — *« plus le contrôle négatif et le cas du build en vol de bout
en bout »* —, est l'objet de ce plan.

### Pourquoi ces trois-là ne sont pas déjà couverts

Les six `mika2943_*` de `deadline_verdict.rs::tests` construisent leur
`TurnConclusion` **à la main**. Ils attestent la décision du filet, pas le
câblage qui l'alimente. Pour les deux abstentions, le câblage est précisément
l'endroit où une régression serait muette :

| décision | ce que l'unité voit | ce que l'unité ne peut pas voir |
|---|---|---|
| AC2 — revue déjà postée | `session_has_review_for` rend `true` sur une clé **écrite par le test** | que le `run_gh pr review` d'un vrai tour écrive bien cette clé dans le registre que le filet relit |
| AC2 — tour non-PR | `parse_pr_target` rend `None` sur un texte **choisi par le test** | rien de neuf ; c'est le terme le moins risqué des trois, et c'est pourquoi il est le plus court |
| AC3 — build en vol | la table à 4 cas de `build_callback_dispatched_in_turn` | que le site de pose d'`agent_loop/mod.rs:7022` laisse bien `max_steps_exceeded = None` quand ce prédicat est vrai, **et** que `of_output` en tire alors `Concluded` |

Le cas AC3 est le plus porteur. Son invariant tient en trois sites qui doivent
s'accorder — le prédicat, le `.then_some(…)` du constructeur, et le bras
`(None, None)` d'`of_output`. Chacun est juste séparément ; un futur
réarrangement de la continuation (le commentaire du site parle déjà d'un
emprunt de `tool_call_summaries` évalué « AVANT la continuation ») peut les
désaccorder sans qu'aucune unité rougisse, et le symptôme serait un **double
`hold`** sur la même PR — la régression que le vol 1 a refusé de renvoyer au
vol 2 pour cette raison même.

## Requirements

Tout se passe dans `crates/mika-agent/tests/eval/test_max_steps_verdict_2943.rs`,
qui porte déjà le corpus (`burned_budget_corpus`), la fixture d'outils
(`qa_tools`), le joint nommé (`conclusion_of`) et les constantes de l'événement
mesuré. Une exception, R5, hors de ce fichier.

### R1 — la chaîne de production devient un terme partagé

Le test AC1 porte, en ligne, le bloc qui fait passer une sortie de tour par la
chaîne de production : `deadline_verdict_target(conclusion_of(&trace.output), …)`
puis, sur `Some`, `maybe_post_deadline_verdict` avec un poster qui capture et
compte. Les trois tests de ce vol ont besoin **du même** bloc.

Extraire un helper de fichier :

```rust
/// Ce que le call-site webhook exécute, et ce que la PR en reçoit.
///
/// La chaîne entière de production — `conclusion_of` → `deadline_verdict_target`
/// → `maybe_post_deadline_verdict` — plus le poster qui capture. Un terme
/// partagé plutôt qu'une copie par test : quatre copies de la chaîne
/// divergeraient, et un test qui n'appellerait plus `conclusion_of` cesserait
/// de traverser le joint où le défaut vivait sans que rien ne le dise
/// (classe mika#2158).
async fn what_the_pr_receives(
    output: &AgentOutput,
    event_text: &str,
    registry: &DashMap<String, HashSet<String>>,
) -> NetDecision { … }

/// Ce que la chaîne a décidé, et ce qu'elle a émis.
struct NetDecision {
    /// `None` = `deadline_verdict_target` n'a pas armé le filet.
    outcome: Option<DeadlineVerdictOutcome>,
    posts: usize,
    captured: Option<PostReviewRequest>,
}
```

Contraintes :

- **Pas d'`expect()` sur le `Option` de `deadline_verdict_target`.** Le refus du
  call-site est ce que deux des trois tests de ce vol **attendent** ; un
  dépliage y paniquerait sur le chemin nominal. Le `None` remonte dans
  `NetDecision::outcome`, exactement comme le test AC1 le fait aujourd'hui.
- Le test AC1 existant est **réécrit pour appeler ce helper**, et ses assertions
  ne changent pas d'un octet : même précondition sur le décompte d'appels, même
  négation sur le contenu capturé, même message `ÉTAT INTERDIT (mika#2943)`.
  C'est un refactor sans changement de comportement, et il est dans le même
  commit que le helper (sinon le fichier porte deux copies de la chaîne).

### R2 — AC2 de bout en bout : une session qui a déjà revu ne reçoit rien de plus

Nouveau test
`mika2943_une_coupure_apres_revue_postee_najoute_rien_sur_la_pr`.

- **Corpus** : identique à `burned_budget_corpus(ceiling)` à un appel près — le
  premier pas est un `run_gh` de `pr review` sur #2920, suivi de `ceiling - 1`
  `run_shell`. Le tour brûle donc son plafond **après** avoir posté sa revue.
  C'est la forme de la trace `e6e7d8e0` (#2941, 27/10 15:38) que le ticket
  compte hors de la population du défaut.
- **La fixture `run_gh` écrit la clé de déduplication**, comme le handler de
  production le fait sur succès de `gh pr review`. C'est le maillon que
  l'unité ne voit pas : elle écrit la clé elle-même. Une variante de `Recording`
  — `RecordingThatRegisters { name, calls, registry, session_id, key }` — ou un
  champ `on_success: Option<Box<dyn Fn(&DashMap<…>)>>` ajouté à `Recording` ;
  la seconde forme est préférée, elle évite un second `impl Tool`.
- **La clé vient de `format_pr_dedup_key`**, jamais recomposée dans le test
  (voir R5).
- **Préconditions dures** avant la négation, sur le modèle du test AC1 :
  - les `run_shell` servis valent `ceiling - 1` et la sentinelle du corpus n'a
    pas été consommée — le tour a bien été coupé ;
  - `trace.output.max_steps_exceeded.is_some()` — le fait de coupure est posé
    (aucun build n'est en vol dans ce corpus, donc R4 ne masque pas ce cas) ;
  - le registre porte **exactement une** clé pour cette session.
- **L'assertion** : `outcome == Some(DeadlineVerdictOutcome::AlreadyReviewed)`
  (`deadline_verdict.rs:369`) et `posts == 0`. Le filet a **décidé**
  et il n'a rien posté — les deux termes comptent : `posts == 0` seul serait
  aussi vrai d'un filet jamais armé, ce qui est un autre défaut.

### R3 — AC2 de bout en bout : un tour non-PR coupé n'a pas de cible

Nouveau test `mika2943_un_tour_non_pr_coupe_ne_poste_rien`.

- **Corpus** : `burned_budget_corpus(ceiling)`, inchangé.
- **Déclencheur** : un événement GitHub réel qui ne nomme aucune PR —
  `"[GitHub] Check suite success on senara-solutions/mika (branch: main)"`, la
  forme qu'émet `webhook_queue_v2::CHECK_SUITE_RE`, sur la branche par défaut.
  Choisi plutôt qu'un texte Telegram parce que c'est un **webhook**, donc le
  même call-site : le terme sous test est l'absence de PR dans le texte, pas
  l'absence de webhook.
- **Préconditions dures** : le tour est coupé (décompte + sentinelle) et
  `trace.output.max_steps_exceeded.is_some()` — le fait de coupure est bien posé.
  Sans cette seconde précondition, un filet qui s'abstiendrait parce que la
  boucle n'a rien posé passerait pour un filet qui s'abstient faute de cible, et
  le test serait vert pour la mauvaise raison.
- **L'assertion** : `outcome.is_none()` et `posts == 0`. Et un commentaire qui
  dit ce que le ticket dit : **ce n'est pas un défaut**.

### R4 — AC3 de bout en bout : un build en vol fait taire le filet du webhook

Nouveau test
`mika2943_une_coupure_avec_un_build_en_vol_laisse_le_verdict_au_callback`.

- **Corpus** : le premier pas appelle `build_mika` (succès), puis `ceiling - 1`
  `run_shell`, puis le texte de continuation et la sentinelle. C'est la forme
  des traces `11699630` (#2920, 26/10 15:13) et `517380e0` (#2932, 27/10 00:58)
  que le ticket compte hors de la population du défaut.
- **La fixture fournit `build_mika`** par le même `Recording`, sous le nom
  `qa_build_callback::BUILD_MIKA_TOOL` — la constante, jamais le littéral
  `"build_mika"`. Le prédicat du vol 1 lit ce nom ; un littéral recopié ici
  laisserait le test vert si la constante changeait.
- **Préconditions dures** :
  - le tour est coupé (décompte des `run_shell` à `ceiling - 1`, sentinelle non
    consommée) ;
  - `build_mika` a bien été appelé une fois, et la trace le porte dans
    `trace.tool_calls` avec succès — c'est le terme que lit
    `build_callback_dispatched_in_turn`, et il doit être **mesuré**, pas supposé ;
  - `trace.output.deadline_exceeded.is_none()` — l'abstention qu'on mesure est
    celle du plafond de pas, pas un tour mort sur sa deadline.
- **L'assertion centrale** : `trace.output.max_steps_exceeded.is_none()`, donc
  `conclusion_of` rend `Concluded`, donc `outcome.is_none()` et `posts == 0`.
  Le message d'assertion nomme la régression qu'il interdit : **un second
  `hold[review]` sur une PR dont le callback de build portera déjà le verdict**.
- **Ce que ce test ne mesure pas, et le dit** : la ligne INFO
  `qa_max_steps_verdict_withheld`. Le harnais eval ne capture pas le flux
  `tracing`, et l'y brancher pour une ligne serait une infrastructure hors
  périmètre. Le doc-comment du test renvoie à la ligne du site
  (`agent_loop/mod.rs:6960-6973`) : l'abstention est **dite** là, et c'est ce
  que la sonde AC4 relit en production.

### R5 — `format_pr_dedup_key` devient `pub`

`crates/mika-agent/src/skills/builtin_handlers.rs:5480` la déclare
`pub(crate)`. R2 en a besoin depuis `tests/eval`, qui est une **crate externe**.

Changement : `pub(crate)` → `pub`, plus deux lignes au doc-comment existant
disant pourquoi la visibilité s'élargit. Le doc-comment de
`session_has_review_for` (`deadline_verdict.rs:476-487`) a déjà écrit
l'argument : *« le format de clé appartient à `format_pr_dedup_key` […] faute de
quoi la même grammaire vivrait à deux endroits et dériverait en silence dans les
deux sens »*. Un test d'intégration qui recomposerait la clé serait ce
troisième endroit, et son mode de panne est précisément celui que R2 doit
interdire : une clé fabriquée laisse le filet re-poster.

Alternative écartée : un helper `#[cfg(feature = "test-utils")]` qui réexporte
la fonction. Il ajoute un site pour ne rien gagner — la fonction est déjà la
source de vérité, et `pub` est la déclaration honnête de ce qu'elle est.

## Décisions

**D1 — AC3 : le filet du webhook s'abstient. Décision du vol 1, reconduite.**
Quand la coupure survient après un `build_mika` dispatché avec succès, le
callback portera le verdict par son propre filet (mika#2515), et un second
`hold[review]` serait un doublon sur la même PR. Le ticket demande que ce cas
soit *« tranché par écrit dans le plan »* : il l'est, dans le sens de
l'abstention, et le vol 1 a livré le code et la ligne de décision. Ce vol en
livre l'attestation de bout en bout (R4). Un double `hold` n'est **pas**
acceptable : `verdict_handler` serait réveillé deux fois sur une décision qui
n'est pas la sienne.

**D2 — l'abstention de R4 est assertée sur le champ, pas sur le log.** Voir R4.
Le champ `max_steps_exceeded` est la cause ; la ligne INFO est son témoin
opérateur. Asserter la cause dans un test et le témoin dans la sonde est la
répartition juste des deux instruments.

**D3 — le contrôle non-PR de R3 prend un `check_suite` sur la branche par
défaut.** Un webhook, donc le même call-site, et un texte dont l'absence de
numéro de PR est structurelle. La forme `check_suite` **sur une branche de PR**
est un résidu nommé en `## Hors périmètre` ci-dessous, et R3 ne le couvre pas :
sa branche est `main`.

**D4 — aucune ligne de production nouvelle, sauf la visibilité de R5.** Si
l'implémentation découvre qu'un des trois tests exige un changement de
comportement pour passer, elle **s'arrête et remonte** : ce serait un défaut que
le vol 1 a laissé, pas une lacune d'attestation, et son périmètre n'est pas
celui de ce plan.

## Fire-Disposition

Ce plan livre trois détecteurs : les tests R2, R3 et R4.

**Option (a) — exception nommée en allowlist, avec une allowlist vide.**

Les trois détecteurs atterrissent **armés** (ni `#[ignore]`, ni `#[cfg(skip)]`)
et **verts** : le comportement qu'ils mesurent existe sur `HEAD` depuis le vol 1
(`e2e9774d1`). Il n'y a donc aucune violation préexistante à excepter, et
l'allowlist est vide — par **constat**, pas par choix.

Le constat est vérifiable avant d'écrire le premier test, et l'ordre de commit
le rend obligatoire : R1 d'abord (refactor, suite verte), puis chaque détecteur
sur un arbre où sa cible existe déjà. Un détecteur qui rougirait à son
atterrissage ne serait pas une exception à inscrire : ce serait le cas D4 —
halte et remontée.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `tests/eval/test_max_steps_verdict_2943.rs` — R1 (helper + `NetDecision` + réécriture du test AC1) | 55 |
| `tests/eval/test_max_steps_verdict_2943.rs` — R2 (fixture `on_success` + test AC2 revue postée) | 85 |
| `tests/eval/test_max_steps_verdict_2943.rs` — R3 (test AC2 non-PR) | 45 |
| `tests/eval/test_max_steps_verdict_2943.rs` — R4 (corpus build + test AC3) | 75 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` — R5 (visibilité + doc) | 8 |

Total estimé : 268 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient en un seul vol.** 268 contre 429 : la marge est de 161
lignes, et elle est achetée par deux faits. D'abord, le fichier cible **existe
déjà** et porte son en-tête doctrinal (54 lignes), son corpus, sa fixture et ses
constantes d'événement : ce vol n'en paie aucun. Ensuite, R1 est un
investissement qui se rembourse dans le même vol — la chaîne de production est
écrite une fois et lue quatre, là où quatre copies en ligne vaudraient environ
120 lignes de plus et porteraient le total à ~390, soit sous la borne mais sans
marge. Le poste le plus incertain est R2 (la fixture qui écrit la clé) ; même
doublé il laisse le total à ~350.

Aucun découpage n'est nécessaire, et il n'y a pas de suite à nommer : le vol 1
était le code, ce vol est son attestation, et AC4 est une sonde opérateur sans
livrable de code.

## Ordre de commit

1. **R1 + R5** — le helper partagé, `NetDecision`, la réécriture du test AC1, et
   la visibilité de `format_pr_dedup_key`. Refactor pur : la suite est verte
   avant et après, et le test AC1 garde ses assertions à l'octet. R5 est ici
   parce que R2 l'exige et qu'un commit de visibilité seul n'a pas de lecteur.
2. **R3** — le détecteur le plus court, sur le helper fraîchement posé. Il
   atteste le chemin « aucune cible » et sert de première validation du helper
   sur le bras `None` de `deadline_verdict_target`.
3. **R2** — la fixture qui écrit la clé de déduplication, puis le détecteur. Le
   plus gros des trois : son commit est seul.
4. **R4** — le corpus avec `build_mika` et le détecteur d'abstention AC3.

L'ordre 2-avant-3 est délibéré : R3 exerce le bras `None` du helper avec un
corpus déjà connu de la suite, donc une panne au tour 2 est une panne du helper
et non de la fixture de R2. Les trois détecteurs atterrissent après R1, sur un
arbre où leur cible existe — la raison de la section `## Fire-Disposition`.

## Definition of Done

- `cargo test -p mika-agent --test eval mika2943` — les quatre tests du fichier
  verts, dont les trois nouveaux.
- `cargo test` — suite complète verte, le test AC1 réécrit inclus.
- `cargo fmt --check` propre, `cargo clippy --all-targets` propre.
- `bash scripts/verify-pipeline.sh origin/main` → *Pipeline verification passed*.
- Le corps de la PR porte la section `## Sonde post-déploiement` reprise de ce
  plan, et nomme le résidu `check_suite_pr_branch` du `## Hors périmètre`
  ci-dessous comme un **constat de mesure**, non comme un report : il appelle
  son propre ticket, que l'opérateur ouvre.
- `grep -rn 'TurnConclusion::from' crates/` ne rend rien (invariant du vol 1,
  revérifié).

## Acceptance criteria

Repris verbatim de senara-solutions/mika#2943. AC1 est attesté par le vol 1
(`e2e9774d1`) et sa couverture n'est pas rejouée ici ; AC2 et AC3 reçoivent de ce
vol leur attestation de bout en bout ; AC4 est une sonde opérateur.

- **AC1.** Quand un tour webhook mika-qa est coupé par `max_steps` sur un
  événement de PR, sans verdict posté dans la session, il poste par le moteur un
  `VERDICT: hold[review]` sur cette PR, avec `cause=cut_off_max_steps` et le
  nombre de pas. Le corps et l'événement sont ceux du filet de mika#2276. Il n'y
  a pas de seconde grammaire.
- **AC2.** Contrôle négatif dans le même test. Une coupure `max_steps` dont la
  session a déjà posté une revue (le cas e6e7d8e0) ne poste rien de plus : la
  déduplication passe par `session_has_review_for`. Un tour non-PR coupé ne poste
  rien.
- **AC3.** Le cas « build lancé avant la coupure » (11699630, 517380e0) est
  tranché par écrit dans le plan. Soit le filet du webhook s'abstient, parce que
  le callback portera le verdict, soit il poste, et alors le plan dit pourquoi un
  double `hold` est acceptable.
- **AC4.** Sonde après déploiement. Au prochain `max_steps_tool_census` d'un tour
  webhook mika-qa sur une PR, la même trace porte un `deadline_verdict` posté, ou
  une revue de la session.

**Correspondance.** AC1 → vol 1 (R3/R4 de son plan) + le test de bout en bout
déjà présent, revérifié par la réécriture R1. AC2 → R2 (revue déjà postée) et R3
(tour non-PR), tous deux dans le même fichier que le test AC1, comme l'AC le
demande. AC3 → D1 (la décision écrite) + R4 (son attestation). AC4 → la section
`## Sonde post-déploiement` ci-dessous.

## Hors périmètre

- **La raison pour laquelle mika-qa brûle ses pas en `run_shell`** relève de
  mika#2926 (b) : le resserrement du skill. Ce ticket ne pose que le filet.
- **Le relèvement du plafond.**
- **Le résidu `check_suite_pr_branch` — constat de ce grooming, et il appelle son
  propre ticket.** La table de mesure du ticket compte une trace déclenchée par
  `check_suite_pr_branch` (`71a8fa50`, 27/10 01:13, 29 appels pour un plafond de
  20). Le filet ne la couvrira pas, et pour une raison de **forme** du texte
  d'événement, pas d'oubli : `webhook_queue_v2::CHECK_SUITE_RE` lit
  `[GitHub] Check suite {conclusion} on {repo} (branch: {branch})` — une branche,
  **jamais un numéro de PR**. `parse_pr_target` (`deadline_verdict.rs:419`) ne
  connaît que les formes `PR review (…)` et `PR {action}: {repo}#{pr}`, donc elle
  rend `None`, donc le filet s'abstient sans cible.

  Ce résidu **n'est pas propre à mika#2943** : il est partagé par les deux
  populations fondatrices du même filet, mika#2276 (deadline) et mika#2289 (tour
  mort). Le fermer demande de résoudre la PR ouverte d'une branche, donc un appel
  réseau GitHub **sur un chemin de coupure** — une décision d'architecture pour
  trois populations, qui n'a sa place ni dans l'attestation de ce vol ni dans le
  périmètre d'AC1 (« sur un événement de PR », au sens du texte d'événement).

  Il est nommé ici pour que la sonde AC4 ne le lise pas comme un vert : voir la
  dernière ligne de la section suivante.

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

**Ce qui n'est pas un vert non plus, et qu'il faut compter à part.** Une trace de
(1) dont le déclencheur est `check_suite_pr_branch` ne portera aucune des trois
lignes, et ce **n'est pas** l'absence de défaut : c'est le résidu nommé en
`## Hors périmètre`. Le relevé de la sonde doit donc séparer deux quantités, et
non les additionner (classe mika#2205) : les traces de (1) sur un **texte**
d'événement qui nomme une PR — la population d'AC4, dont aucune ne doit être
muette — et les traces `check_suite_pr_branch`, dont le compte est la taille du
résidu et l'argument de son ticket. Le déclencheur se lit sur la ligne
`max_steps_tool_census` elle-même.
