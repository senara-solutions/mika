---
issue: 2266
type: test
title: "fan-out ci_success sur 2 agents — le relecteur court-circuité à l'entrée"
branch: test/2266/eval-3-tests-d-int-gration-multi-agents
status: groomed
---

# Plan — fan-out `ci_success` sur deux agents (mika#2266)

## Invariant (une phrase)

**Le même événement `check_suite.completed(success)`, délivré aux deux scopes
d'agent, produit deux dispositions distinctes — et le test le prouve en nommant
*quel* agent a fait quoi, à l'entrée du chemin, avant tout appel réseau.**

Le mot load-bearing est **entrée**. La garde de #2258 (`pr_merge_with_gate` refuse
le relecteur juste avant l'appel `gh`) est une garde *tardive* : elle corrige la
conséquence en laissant la course exister — c'est exactement ce que #2260 mesure
et veut supprimer. Un test qui n'observe que le `mergedBy` final ne distingue pas
le monde d'avant du monde d'après : il passe dans les deux. Seule une assertion
d'entrée les sépare.

## Ce que le code dit aujourd'hui (vérifié, `file:line`, 2026-09-09)

### `try_handle_ci_success` n'interroge jamais l'identité de son agent

`server/ci_success_handler.rs:123-270`, dans l'ordre :

1. `:132` `parse_check_suite_success(text)` — pur, aucun effet de bord.
2. `:143` exigence du token. Sans token :
   `Passthrough { enrichment: Some("[ci_success_handler] CI checks passed but no GitHub token configured…") }`.
3. `:161` `find_open_pr(...)` — **premier appel réseau** (`gh`).
4. `:186` dédup couche 1 (mémoire), `:213-256` dédup couche 2 (audit-durable,
   marqueur `ci_success_handler_processed`), `:266` `find_pass_verdict(...)`.

Aucun `db.agent_id()` sur ce chemin. `is_reviewer_agent` / `is_dispatcher_agent`
existent (`mika-common/src/forge_identity.rs:67,72`) et servent à l'aval
(`merge_ready_handler.rs:125,138`), mais rien ne les consulte à l'entrée. Le
relecteur entre donc réellement dans le chemin — le fait que #2260 rapporte.

Le commentaire `:211-214` porte d'ailleurs la prémisse à corriger : *« check_suite
success events for a given PR route to the same agent (mika-qa), so the scope is
correct »*. Le routage réel est un **fan-out** — `mika-gateway/src/github.rs:358-369`
`secondary_targets()` : primaire `mika-dev` **plus** secondaire `mika-qa`. Les deux
agents reçoivent l'événement, chacun avec son propre `agent_id` et sa propre
tranche d'audit. La dédup couche 2 est donc scopée par agent et ne peut pas, par
construction, empêcher le second agent d'entrer.

### Le fix de #2260 n'existe nulle part, et son nom d'audit non plus

`grep -rn ci_success_reviewer_noop` sur le dépôt → **0 occurrence**. Aucune branche
ne porte de garde d'identité à l'entrée de `ci_success_handler.rs`. Ce plan livre
la sonde ; le remède est #2260.

`ci_success_handler.rs` est zone **décision-core → CODEOWNERS @samidarko** (corps de
#2260) : la PR de #2260 est human-gatée au merge par conception, et ce ticket ne
change pas cela.

### Le harness #2265 est groomé, non implémenté

`test/2265/harness-harness-eval-multi-agents-2` @ `38c14c1a` ne porte que son plan.
`MultiAgentHarness` (base **fichier** partagée, N `agent_id`, `agents()`,
`audit_counts_by_agent`, `cross_read_count`) n'existe pas encore. Ce ticket ne peut
pas compiler avant que #2265 ait mergé, et la sonde de partage (T3 ci-dessous) est
ce qui vérifie que le montage partage réellement une base — sans elle, deux
`AsyncDatabase` en mémoire disjointe passeraient les deux autres assertions sans
rien mesurer (D1 du plan #2265).

### Ce que #2269 a déjà retiré du périmètre

PR **#2269** (branche `fix/2263/…`) livre les deux autres tests que ce ticket
portait à l'ouverture : `test_supersede_kills_live_pilot.rs` (2 tests),
`test_dispatch_fired_at_stamped.rs` (3 tests) et `test_ready_label_blocked_skip.rs`
(3 tests, assertant `Handled` — pas `Passthrough`). Chacun naît **vert** à côté de
son fix, avec sa recette de rouge-avant écrite dans l'en-tête. Le corps du ticket
porte le tableau de disposition ; ce plan n'y revient pas.

## Décisions de conception

### D1 — La paire de contrôles tient AVANT le premier `gh`

Le levier est l'étape 2 (`:143`) : avec `github_token = None`, les deux scopes
rendent des `VerdictAction` **distinguables** sans qu'aucun appel réseau ne parte.

| scope | `github_token` | disposition attendue après le fix #2260 |
|---|---|---|
| `mika-qa` (relecteur) | `None` | `Passthrough { enrichment: None }` + 1 audit de no-op |
| `mika-dev` (dispatcheur) | `None` | `Passthrough { enrichment: Some("…no GitHub token configured…") }` + **0** audit de no-op |

*Pourquoi cela suffit.* La propriété que #2260 veut est « le relecteur ne **traite**
pas le merge ». « Ne traite pas » est observable comme : sortie sans enrichment
(la porte a rendu avant l'étape 2) **et** zéro marqueur `ci_success_handler_processed`
écrit (le relecteur n'a pas atteint `:244`). Le dispatcheur, lui, doit dépasser la
porte — son enrichment de token le prouve. Aucune de ces deux observations ne
nécessite de franchir `find_open_pr`.

*Rejeté :* poser un joint d'injection pour `find_open_pr` / `find_pass_verdict`.
Zone décision-core, CODEOWNERS, PR human-gatée — un coût réel pour une capacité
dont ce test n'a pas besoin.
*Rejeté :* asserter le `mergedBy` final. C'est la garde tardive de #2258, déjà
correcte aujourd'hui : l'assertion passerait avant comme après le fix #2260. Sonde
qui ment sans échouer.

### D2 — Boucle `for`, pas `tokio::join!`

#2260 est une question d'**entrée**, pas de course : la garde d'entrée *supprime* la
course plutôt que de l'arbitrer. Diffuser en `join!` introduirait une
non-déterminance qui ne mesure rien de plus et rendrait un échec difficile à
attribuer. Le plan #2265 (D2) laisse explicitement ce choix au test ; celui-ci
choisit l'ordre.

### D3 — Le nom de l'événement d'audit appartient à #2260

Le test lit le nom depuis la constante que le fix introduit, plutôt que de le
répéter en littéral. Un test qui pré-nomme l'identifiant d'un fix non écrit fige
une décision qui n'est pas la sienne — et `ci_success_reviewer_noop`, proposé par
le corps d'origine, n'existe dans aucun code.

*Conséquence de séquencement :* le fichier ne compile pas avant #2260. C'est
assumé, et c'est pourquoi D4.

### D4 — Le fichier land AVEC le fix de #2260

Dans la PR de #2260, ou empilé dessus par `Companion PR`. Il n'atterrit **jamais**
rouge dans la suite : un test-témoin rouge en attente d'un autre ticket bloque la
CI de tout le monde et n'observe jamais « la cause est partie » — il campe sur la
cause présente. C'est la discipline que #2264/PR#2268 a rendue obligatoire à la
porte, et que les trois tests de #2269 respectent.

### D5 — Zéro modification de `crates/mika-agent/src/`

Ce ticket est test-only. Si l'implémentation découvre un besoin d'API dans `src/`,
c'est un signal de conception à remonter à #2260, pas une extension à faire au
passage.

## Livrables

### L1 — `crates/mika-agent/tests/eval/test_ci_success_fanout_2260.rs` (nouveau)

Monté sur `MultiAgentHarness::builder().agent("mika-dev").agent("mika-qa")` (#2265),
un seul texte d'événement `check_suite.completed(success)` bien formé, diffusé en
boucle aux deux handles avec `github_token = None`.

- **T1 `reviewer_short_circuits_at_entry`** — handle `mika-qa` :
  `Passthrough { enrichment: None }` ; `audit_counts_by_agent(<audit du no-op>)`
  rend `{mika-qa: 1, mika-dev: 0}` ; **et** `audit_counts_by_agent("ci_success_handler_processed")`
  rend `0` pour les deux — la forme observable de « zéro effet de bord », puisque
  ce marqueur est écrit à `:244`, en aval de la porte.
- **T2 `dispatcher_passes_the_entry_gate`** (contrôle négatif) — handle `mika-dev` :
  l'`enrichment` contient `no GitHub token configured` (le flux a dépassé la porte
  d'identité et atteint `:143`) ; 0 audit de no-op sous `mika-dev`.
- **T3 `the_two_scopes_are_one_database`** (sonde de partage) —
  `cross_read_count("mika-dev", "mika-qa") > 0`.

Sans T2, T1 est vacuously vraie (un handler qui refuserait *tout le monde* la
passerait) ; sans T3, un montage à mémoires disjointes passerait T1 et T2 sans rien
partager. Les trois contrôles vivent dans le même fichier, cf.
`feedback_a_probe_needs_both_controls_in_the_same_call`.

### L2 — Déclaration du module

`mod test_ci_success_fanout_2260;` dans le bloc `mod eval` de `tests/eval.rs`. Non
négociable : un `.rs` posé dans `tests/eval/` sans sa ligne `mod` n'est pas compilé
— il ne casse rien, ne rapporte rien, et aucune CI ne le réclame. La porte
`test_eval_modules_declared` (L5 du plan #2265) le vérifie mécaniquement.

### L3 — En-tête du fichier : le rouge-avant constaté

L'en-tête consigne le rouge **vu de ses yeux** pendant l'implémentation (V2), pas
une recette théorique : sur le code sans la garde d'entrée, le relecteur atteint
`:143` et rend l'`enrichment` du token — T1 échoue sur `enrichment == None`. Le
rouge disparaît quand la **cause** part (la garde d'identité), pas quand un symptôme
est rattrapé — directive Prime du 2026-09-09 sur ce ticket.

## Vérification

| # | Ce qui est vérifié | Comment |
|---|---|---|
| V1 | Le fan-out est mesuré | `cargo test -p mika-agent --test eval ci_success_fanout` — 3 tests verts (sur la branche portant le fix #2260) |
| V2 | Rouge-avant réel | Le fix #2260 neutralisé (garde d'entrée commentée) → T1 échoue sur `enrichment == None`, T2 et T3 restent verts. Constaté et consigné dans l'en-tête |
| V3 | Le rouge est **term-by-term** | T1 porte deux assertions (`enrichment`, marqueur de dédup) ; neutraliser la garde doit faire tomber les **deux**, pas seulement la première — cf. `feedback_red_before_control_is_term_by_term` |
| V4 | Pas de dérive `src/` | `git diff --stat main -- crates/mika-agent/src/` → vide (D5) |
| V5 | Aucun livrable invisible | `cargo test -p mika-agent --test eval eval_modules_declared` vert **et** `grep -c 'mod test_ci_success_fanout_2260;' crates/mika-agent/tests/eval.rs` = 1 |
| V6 | Hygiène | `cargo clippy -p mika-agent --tests -- -D warnings`, `cargo fmt --check` |

## Fire-Disposition

Schéma canonique (mika#1574) : **(a) exception nommée en liste blanche / (b)
posé-désactivé / (c) halte-et-remontée**.

**1. T1 + T2 → (c) halte-et-remontée, gate CI bloquant.**
Ils tirent quand la garde d'identité d'entrée disparaît, cesse de discriminer, ou
se déplace en aval du marqueur de dédup. Aucune liste blanche : une exception
nommée ici rouvrirait précisément la course que #2260 ferme. Violations
préexistantes : **une, connue et voulue** — sans le fix #2260 le test est rouge,
raison pour laquelle il land avec le fix (D4) et jamais avant.

**2. T3 → (c) halte-et-remontée.**
Elle tire si le harness #2265 régresse vers des mémoires disjointes. Disposition :
réparer le montage, jamais assouplir la sonde — une sonde de partage assouplie
laisse T1 et T2 passer sans rien mesurer.

**Ce qui ne tire pas.** L2 et L3 sont de la déclaration et de la documentation ;
ils ne rendent aucun verdict. Ce ticket n'a pas d'outillage propre — il consomme
celui de #2265.

## Hors périmètre (explicite)

- **Écrire le fix de #2260** (la garde d'identité et son événement d'audit). Ce
  ticket livre la sonde, pas le remède ; le merge de #2260 reste sous garde humaine
  (CODEOWNERS @samidarko).
- **Poser un joint d'injection dans `ci_success_handler.rs`** (D1, rejeté).
- **Corriger la prémisse du commentaire `:211-214`** (« route to the same agent »).
  Elle est fausse au regard de `secondary_targets()`, mais la corriger est une
  édition de `src/` qui appartient à la PR de #2260 (D5).
- **Tester le routage gateway.** `secondary_targets` a ses tests
  (`mika-gateway/src/github.rs:1921`).
- **Les deux tests livrés par #2269.**

## Acceptance (tie-back)

| AC (corps) | Livrable | Vérif |
|---|---|---|
| AC1 — 3 assertions : court-circuit relecteur, contrôle négatif dispatcheur, sonde de partage | L1 (T1/T2/T3) | V1, V5 |
| AC2 — rouge-avant réel, consigné dans l'en-tête | L3 | V2, V3 |
| AC3 — le nom de l'audit vient de #2260, pas de ce ticket | D3 | V1 |
| AC4 — land avec le fix #2260, jamais rouge en suite | D4 | V1 |
