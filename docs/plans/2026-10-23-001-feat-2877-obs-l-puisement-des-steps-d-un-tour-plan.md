# mika#2877 — l'épuisement des steps d'un tour nomme les outils qui ont consommé le budget

**Ticket :** senara-solutions/mika#2877
**Labels :** `enhancement`, `p2-normal`, `ready`, `observability`, `origin:manual`, `dispatch:loop`, `loop-substrate`
**Type :** vol de mesure (`feat(obs)`) — aucune correction de comportement.

---

## 1. Contexte

Mesuré le 2026-10-22 sur `/var/log/mika/server.log`, moteur `7eb63e43` :
« agent exceeded max tool steps » apparaît **204 fois**, presque exclusivement sur
**mika-qa**. Zéro avant le 2026-10-12, puis 14, 18, 22, 8, 11, 35, 17, 8 et 4 par
jour du 14 au 22/10, à `max_steps=20`, en mode conversation **et** en mode
silencieux. Les tours de QA coupés là finissent en `hold[review]`.

Le WARN se lit aujourd'hui, verbatim (`crates/mika-agent/src/agent_loop/mod.rs:5110-5113`) :

```rust
warn!(
    label = mode.label(),
    max_steps, "agent exceeded max tool steps"
);
```

Deux champs : `label` et `max_steps`. Ils disent **que** le budget a été épuisé
et **quel** était le plafond. Ils ne disent pas **par quoi**. Or les trois
corrections candidates — relever le plafond, corriger un outil qui boucle,
raccourcir le prompt QA — se départagent exactement sur cette donnée :

| ce que la mesure montrerait | la correction que ça désigne |
|---|---|
| 20 appels répartis sur 6 outils distincts, aucun dominant | le travail est réel, le plafond est trop bas |
| 14 appels sur le même outil | cet outil boucle (ou le modèle le relance sur un échec) |
| carte **vide** ou quasi vide | le budget a brûlé hors outils — re-prompts de garde, prompt QA trop long |

Les trois lectures sont mutuellement exclusives et aucune n'est atteignable
depuis le WARN actuel. **Corriger maintenant serait deviner.** Ce vol livre
l'instrument, pas la correction.

## 2. Direction retenue

Un **événement distinct**, émis au site de coupure, immédiatement après le WARN
existant — lequel reste **inchangé, octet pour octet**.

Le ticket laisse le choix entre enrichir le WARN et porter un événement
distinct. L'argument décisif est la **comparabilité de la série déjà mesurée** :
le constat du ticket est un `grep "agent exceeded max tool steps"` qui a rendu
204 lignes sur dix jours. Cette ligne est le dénominateur de la mesure. Y
ajouter des champs — et, pour la rendre filtrable par `event`, un champ `event`
qu'elle n'a pas — réécrirait la ligne même que la série time-série suit. Un
événement voisin n'y touche pas : l'ancienne courbe reste lisible, la nouvelle
commence à la date du déploiement, et les deux se comparent.

Second effet, gratuit : les deux lignes étant émises au **même site, sans
condition**, leurs comptes sont égaux par construction sur tout binaire
postérieur à ce vol. D'où la sonde de § 7.

Nom de l'événement : `max_steps_tool_census`, suivant `tool_defs_census`
(mika#2783). Niveau **WARN**, pour accompagner le WARN voisin : un épuisement
est anormal, et un opérateur qui filtre son log au niveau WARN doit voir les
deux lignes ou aucune.

### Trois propriétés de lecture, posées avant le code

**« Absent n'est jamais zéro » (mika#2331).** Un outil non appelé est **absent**
de la carte, jamais présent à `0`. La carte énumère ce qui a consommé, pas
l'univers des outils.

**Une carte vide est une mesure, pas une absence.** Un tour qui épuise 20 steps
sans aucun appel d'outil rend `per_tool = {}` et `total_calls = 0` — et c'est
peut-être *la* réponse pour mika-qa (budget brûlé par les re-prompts de garde).
L'émission est donc **inconditionnelle** au site : aucun `if !summaries.is_empty()`.
Une garde là serait précisément celle qui rendrait muet le cas le plus
intéressant.

**L'absence de la ligne signifie un binaire antérieur (mika#2340).** Et ici cette
lecture est *exacte et unique*, parce que le WARN voisin, lui, existe déjà :

- `grep -c "agent exceeded"` > 0 et `grep -c max_steps_tool_census` == 0 ⇒ binaire antérieur à ce vol. **Jamais** « aucun outil appelé ».
- les deux à 0 ⇒ aucun épuisement sur la fenêtre.

## 3. Mécanisme

### 3.1 Où le code atterrit, et pourquoi pas dans `mod.rs`

`crates/mika-agent/src/agent_loop/mod.rs` pèse **989 858 octets**. Le gate CI
`scripts/check-secrets.sh` plafonne tout fichier à 1 048 576 octets
(`LARGE_FILE_LIMIT`, ligne 23) et son `LARGE_FILE_ALLOWLIST` est **vide**
(lignes 42-44). Il reste 58 718 octets de marge. Ce n'est pas un risque abstrait :
une couche pure + un émetteur + leurs tests inline y tiendraient, mais chaque
vol qui prend cette marge rapproche le jour où un vol la dépasse et doit
s'arrêter pour découper un fichier de 1 Mo.

Le nouveau code va donc dans un **module frère**,
`crates/mika-agent/src/agent_loop/max_steps_census.rs`, déclaré à côté de ses
trois précédents (`mod.rs:56-58`) :

```rust
pub mod context_history;
pub mod max_steps_census;   // ← ajouté
pub mod review_anchor;
pub mod skill_nudge;
```

`mod.rs` ne reçoit que la déclaration et le site d'appel.

### 3.2 La couche pure

Source de la mesure : `all_tool_summaries: Vec<ToolCallSummary>`, déclaré à
`mod.rs:1593`, étendu à `mod.rs:2103` à chaque step. Le type
(`crates/mika-agent/src/tool_execution/types.rs:7-18`) :

```rust
pub struct ToolCallSummary {
    pub step: u32,
    pub name: String,
    pub input_summary: String,
    pub output_summary: String,
    pub success: bool,
    pub non_zero_exit: bool,
}
```

La fonction pure prend `&[ToolCallSummary]` et rend un agrégat :

```rust
pub struct MaxStepsToolCensus {
    /// Appels par nom d'outil. `BTreeMap` et non `HashMap` : l'ordre de
    /// sérialisation doit être déterministe, sinon le champ `per_tool` n'est
    /// ni assertable en test ni comparable entre deux lignes de log.
    /// Un outil non appelé est ABSENT, jamais à 0 (mika#2331).
    pub per_tool: BTreeMap<String, u32>,
    /// `all_tool_summaries.len()`. Peut DÉPASSER `max_steps` : un step porte
    /// autant d'appels que le modèle a émis de blocs `tool_use`
    /// (`tool_execution/dispatch.rs:105-125`). Ce n'est pas une anomalie, c'est
    /// du parallélisme — et le distinguer est l'objet de `steps_with_tools`.
    pub total_calls: u32,
    /// Nombre de valeurs de `step` distinctes ayant produit au moins un appel.
    pub steps_with_tools: u32,
    pub distinct_tools: u32,
    /// Appels à `success == false`. Un échec consomme du budget et compte dans
    /// `per_tool` : c'est le motif « un outil qui boucle sur son erreur ».
    pub failed_calls: u32,
}

pub fn census(summaries: &[ToolCallSummary]) -> MaxStepsToolCensus;
```

**Pourquoi `steps_with_tools` est porteur et non décoratif.** Sans lui,
`total_calls = 20` recouvre deux populations non soustractibles (classe
mika#2205) : « 20 steps × 1 appel » (le modèle travaille, le plafond est trop
bas) et « 5 steps × 4 appels parallèles + 15 steps sans appel » (le budget brûle
ailleurs). Ces deux lectures appellent des corrections opposées. Trois lignes de
code les séparent.

**Pas de contrôle de réconciliation à l'exécution.** L'identité
`per_tool.values().sum() == total_calls` est tenue par construction : un seul
parcours de la tranche construit les deux. Un WARN de réconciliation (le geste
de mika#2783 § 2.1) n'aurait ici **aucune population** — il surveillerait deux
sorties du même `for`. L'identité est assertée dans le test unitaire de la
couche pure, où elle protège contre une refonte future de la fonction, et nulle
part ailleurs.

### 3.3 L'émetteur

Même module, suivant la forme canonique du dépôt (`mod.rs:10336-10352`,
`context_window_assembled`) :

```rust
pub const EVENT_MAX_STEPS_TOOL_CENSUS: &str = "max_steps_tool_census";

pub fn emit_max_steps_tool_census(
    summaries: &[ToolCallSummary],
    agent_id: &str,
    session_id: &str,
    trace_id: &str,
    label: &str,
    max_steps: usize,
) {
    let c = census(summaries);
    warn!(
        target: "mika::otel",
        event = EVENT_MAX_STEPS_TOOL_CENSUS,
        agent_id = %agent_id,
        session_id = %session_id,
        trace_id = %trace_id,
        label = %label,
        max_steps,
        total_calls = c.total_calls,
        steps_with_tools = c.steps_with_tools,
        distinct_tools = c.distinct_tools,
        failed_calls = c.failed_calls,
        per_tool = %serde_json::to_string(&c.per_tool).unwrap_or_default(),
        "max steps exhausted; tool call census for the turn"
    );
}
```

Trois points de forme :

- `label` porte la **même** valeur que le WARN voisin (`mode.label()` →
  `"agent"` / `"silent agent"` / `"team agent"`, `mod.rs:817-856`). Même terme,
  donc les deux populations se soustraient ; un synonyme les rendrait
  incomparables.
- `per_tool` est du **JSON compact** sous clés triées : un objet et non une
  liste de paires, parce que l'opérateur le lit au `jq`
  (`jq '.per_tool | to_entries | sort_by(-.value)'`) et que l'ordre par coût est
  le travail de `jq`, pas celui de l'émetteur.
- La carte ne porte que des **noms d'outils** — jamais `input_summary` ni
  `output_summary`. Aucune valeur d'entrée ou de sortie n'y transite, donc aucun
  passage par `secret_scrubber` n'est requis, et c'est une propriété du choix de
  champs, pas une vérification à refaire.

Les identités `agent_id` / `session_id` / `trace_id` sont déjà disponibles au
site : `mod.rs:5087-5094` les y utilise (`tool_ctx.trace_id`, `db.agent_id()`,
`session_id` de la signature de `run_loop`, `mod.rs:1397`).

### 3.4 Le site d'appel

`mod.rs`, immédiatement après le WARN existant (ligne 5113), **avant** que
`all_tool_summaries` ne soit déplacé dans `LoopResult::MaxStepsExceeded` à la
ligne 5125 :

```rust
warn!(
    label = mode.label(),
    max_steps, "agent exceeded max tool steps"
);   // ← inchangé, octet pour octet

// mika#2877 — le décompte par outil du tour coupé. Inconditionnel : une carte
// vide est la mesure « le budget a brûlé hors outils », pas une absence de
// mesure. Doit précéder le déplacement de `all_tool_summaries` ci-dessous.
crate::agent_loop::max_steps_census::emit_max_steps_tool_census(
    &all_tool_summaries,
    db.agent_id(),
    session_id,
    &tool_ctx.trace_id,
    mode.label(),
    max_steps,
);
```

**Un seul site suffit pour les trois modes.** `LoopResult::MaxStepsExceeded` a
**une** construction (`mod.rs:5125`) et trois consommations
(`mod.rs:6468` conversation, `7603` silencieux, `8267` équipe). Le ticket nomme
deux populations (conversation et silencieux) ; toutes deux passent par cette
unique construction. Aucun fan-out n'est nécessaire — et le second WARN
`"silent agent exceeded max tool steps"` (`mod.rs:7608-7613`, dans
`run_silent_agent`) est une **re-journalisation en aval** de ce même
`LoopResult`, pas un second épuisement : l'instrumenter aussi doublerait chaque
tour silencieux dans le décompte. Il n'est pas touché.

### 3.5 La garde structurelle voisine reste verte par construction

`mika2515_every_loop_exit_decides_about_the_verdict_signal` (`mod.rs:13169`)
assert `count_in_production("Ok(LoopResult::") == 6`, `mark_unmet_after_retry() == 3`
et `mark_cut_off( == 2`. L'ajout n'introduit aucune ligne de production portant
l'un de ces trois motifs. Par ailleurs `production_lines_of_this_module()`
(`mod.rs:13127`) filtre `!l.trim_start().starts_with("//")`, donc les
doc-comments `///` du nouveau code sont hors scan de toute façon. Rien à mettre
à jour — et c'est **vérifié**, pas supposé : si le compte bougeait, cette garde
rougirait au premier `cargo test`, ce qui est son métier.

## 4. Livrables et ordre de commit

| # | fichier | contenu |
|---|---|---|
| 1 | `crates/mika-agent/src/agent_loop/max_steps_census.rs` | const du nom d'événement, `MaxStepsToolCensus`, `census()`, `emit_max_steps_tool_census()`, en-tête `//!` portant les trois propriétés de lecture du § 2 |
| 2 | `crates/mika-agent/src/agent_loop/mod.rs` | `pub mod max_steps_census;` (§ 3.1) + le site d'appel (§ 3.4) |
| 3 | `crates/mika-agent/src/agent_loop/max_steps_census.rs` | les 4 tests unitaires inline (§ 5.1) |
| 4 | `crates/mika-agent/tests/eval/test_context_scope_observability_2305.rs` | `Captured` et `capture()` passent `pub(crate)`, avec un doc-comment nommant le second consommateur (§ 5.3) |
| 5 | `crates/mika-agent/tests/eval/test_max_steps_tool_census_2877.rs` | AC1 (positif) + AC2 (contrôle négatif) |
| 6 | `crates/mika-agent/tests/eval.rs` | `mod test_max_steps_tool_census_2877;` dans le bloc `mod eval` |

Commits 1-2 ensemble (le module ne compile utilement qu'avec son appelant),
puis 3, puis 4-5-6 ensemble. Le livrable 6 n'est pas un détail d'intendance :
`test_eval_modules_declared.rs` échoue si un `.rs` de `tests/eval/` n'a pas sa
ligne `mod` — sans elle, le fichier n'est jamais compilé et AC1 serait **vert
pour la mauvaise raison** (zéro test exécuté).

## 5. Verification Contract

### 5.1 Tests unitaires de la couche pure (`max_steps_census.rs`)

| # | ce qu'il épingle |
|---|---|
| U1 | décompte exact sur une tranche mélangée ; ordre des clés déterministe ; `per_tool.values().sum() == total_calls` (l'identité du § 3.2) |
| U2 | tranche **vide** ⇒ carte vide et tous les scalaires à 0 — pas de panique, pas de `None`, une mesure |
| U3 | plusieurs appels au même `step` ⇒ `total_calls > steps_with_tools`, et `total_calls` peut dépasser `max_steps` sans que rien ne soit faux |
| U4 | `failed_calls` compte les `success == false`, **et** ces appels restent comptés dans `per_tool` (un échec consomme du budget) |

### 5.2 AC1 — le test du harnais (positif)

`tests/eval/test_max_steps_tool_census_2877.rs`, sur le patron de
`test_max_steps_continuation.rs` : 20 réponses d'appel d'outil puis un
`text_response` pour le tour de continuation.

La composition est **délibérément mixte** :
`tool_call_response("search_memory", …)` pour la majorité, et au moins un
`multi_tool_response(vec![("search_memory", …), ("store_fact", …)])`
(`mika-common/src/llm/mock.rs:299`) pour qu'un step porte deux appels. Sans ce
step double, un décompte qui compterait les **steps** au lieu des **appels**
passerait le test : les deux nombres seraient égaux. Avec lui, `total_calls`
dépasse `steps_with_tools` et seul un décompte par appel est vert.

Assertions, sur l'unique événement `max_steps_tool_census` du tour :

- `per_tool` est **égal** à la carte attendue en entier — pas « contient ». Un
  `contains` laisserait passer un outil fantôme.
- `total_calls`, `steps_with_tools`, `distinct_tools` aux valeurs calculées à la
  main depuis la séquence scriptée.
- `max_steps == 20`, `label == "agent"`.
- **exactement un** événement de ce nom pour le tour (helper de la forme
  `the_window_event` de mika#2305) : un tour qui en émettrait deux rendrait la
  commande opérateur du § 7 ambiguë sans rien faire rougir.

### 5.3 AC2 — le contrôle négatif

Un tour qui **ne s'épuise pas** : deux appels d'outil réels puis un
`text_response` qui conclut en `EndTurn`. Assertion : **zéro** événement
`max_steps_tool_census`, lu par le **même** filtre que AC1.

Le contrôle est non-dégénéré : le tour appelle de vrais outils, donc le
décompte aurait de la matière à émettre s'il était émis hors coupure. Un
contrôle sans appel d'outil ne distinguerait pas « pas émis parce que pas
d'épuisement » de « pas émis parce que rien à compter ».

**Réutilisation du capteur de tracing.** `Captured` et `capture()` de
`test_context_scope_observability_2305.rs` passent `pub(crate)` et sont importés
par `super::test_context_scope_observability_2305::{…}`. Le précédent est
textuel : `test_dispatch_stall_p0_1381.rs:66` importe déjà
`super::grounding_regressions::engine_correction_rejection::tools_with_dispatch_stubs`
(`pub(crate)`, ligne 173). On hérite ainsi de `keep_registry_multi_dispatcher()`
et de son raisonnement sur le cache d'intérêt mono-dispatcher de `tracing-core`
0.1.36 — sans quoi ce test serait vert ou rouge selon l'ordre des threads du
binaire de test.

L'alternative — créer `tests/eval/tracing_capture.rs` et y migrer les **cinq**
duplicatas existants (`test_context_scope_observability_2305`,
`test_continuation_response_chars_1910`, `test_context_history_per_tenant_2425`,
`test_qa_review_submission_2727`, `test_session_isolation_1951`) — est le bon
geste et n'est **pas** celui de ce vol : c'est un refactor de cinq fichiers de
test pour un vol de mesure S, et il mérite son propre ticket. Y toucher ici
mettrait la mesure en otage du refactor.

### 5.4 AC3 — recette de rouge-avant

Écrite verbatim dans l'en-tête `//!` de
`test_max_steps_tool_census_2877.rs`, exécutable par le relecteur :

1. Commenter la **seule** ligne d'appel de `emit_max_steps_tool_census(` dans
   `mod.rs` (§ 3.4). C'est l'état du code actuel, à l'octet près du reste.
2. `cargo test -p mika-agent --test eval max_steps_tool_census`
3. **AC1 rougit** : `expected exactly one max_steps_tool_census event for the turn, got 0`.
4. **AC2 reste vert** — il atteste une absence, et sous la recette l'absence est
   totale. C'est l'information qui dit qu'**AC2 seul n'atteste rien** : c'est la
   paire qui est le livrable, et la recette le démontre au lieu de l'affirmer.
5. Décommenter ; les deux passent.

Pourquoi commenter l'appel plutôt que `git stash` : retirer le module entier ne
compilerait pas, et « ne compile pas » n'est pas un rouge-avant — il ne dit pas
*quelle* assertion portait la charge.

### 5.5 Pipeline

`cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check`, `scripts/verify-pipeline.sh origin/main`.
`scripts/check-secrets.sh` est le gate qui motive le § 3.1 et doit rester vert :
après le vol, `mod.rs` aura grossi d'environ 400 octets, pas de 10 000.

## 6. Hors périmètre

- **Toute correction.** Ni le plafond (`MAX_TOOL_STEPS = 20`,
  `MAX_TEAM_TOOL_STEPS = 20`, `crates/mika-agent/src/planning/policy.rs:10,14`),
  ni le comportement à l'épuisement, ni le tour de continuation, ni le prompt
  QA. Le ticket est explicite : « Le choix d'une correction viendra dans un
  second vol, après lecture de la mesure en production. »
- **Enrichir le WARN existant** de `trace_id` / `agent_id` / `session_id`. Ce
  serait un changement de contrat non demandé sur la ligne qui porte la série
  des 204 occurrences (§ 2). L'événement nouveau est auto-suffisant : il porte
  ses propres identités, donc aucune jointure entre les deux lignes n'est
  requise.
- **Le WARN de `run_silent_agent`** (`mod.rs:7608-7613`), pour le motif du § 3.4.
- **Un décompte sur les tours qui ne s'épuisent pas.** Le ticket demande la
  mesure à la coupure ; AC2 en fait une exigence.
- **Le refactor du capteur de tracing en module partagé** (§ 5.3). Mérite son
  ticket ; n'est pas tracké aujourd'hui.
- **Un test end-to-end du cas « tour épuisé sans aucun appel d'outil ».**
  Composer un tel tour dans le mock exigerait de piloter les gardes de re-prompt
  (`text_tool_call_retry`, `prose_tool_call_retry`), ce qui mesurerait les
  gardes et non le recensement. La propriété est tenue par deux faits
  vérifiables séparément : U2 (la couche pure rend une carte vide sans broncher)
  et l'**absence de garde** au site d'appel, visible dans un diff de six lignes.

## 7. Comment on lit la mesure en production

Après `make deploy`, sur une fenêtre d'une semaine :

```sh
# Les outils qui consomment le budget, par coût décroissant, sur mika-qa
grep max_steps_tool_census /var/log/mika/server.log \
  | jq -r 'select(.agent_id == "mika-qa") | .per_tool' \
  | jq -s 'reduce (.[] | fromjson | to_entries[]) as $e ({}; .[$e.key] += $e.value)
           | to_entries | sort_by(-.value)'

# Parallélisme ou longueur : total_calls vs steps_with_tools
grep max_steps_tool_census /var/log/mika/server.log \
  | jq -r '[.label, .total_calls, .steps_with_tools, .distinct_tools, .failed_calls] | @tsv'
```

**Sonde de conformité de l'instrument.** Les deux lignes étant émises au même
site sans condition (§ 2), sur tout binaire postérieur à ce vol :

```sh
grep -c "agent exceeded max tool steps" /var/log/mika/server.log
grep -c max_steps_tool_census           /var/log/mika/server.log
```

Les deux comptes doivent être **égaux**. Une inégalité n'est pas un détail de
log : premier > second signifie qu'un chemin de coupure échappe à l'émission
(ou que le log couvre le déploiement, § 2 dernière puce) ; second > premier
signifie une double émission, ce que le test de § 5.2 interdit en harnais.

## 8. Risques

| risque | lecture |
|---|---|
| Volume de log | ≈ 20 épuisements/jour × ~250 o = **5 Ko/jour**. Négligeable devant un `server.log` qui porte déjà le WARN voisin. |
| Carte large | Plafond structurel : le nombre d'outils distincts du registre (~51 builtins), soit ~1,3 Ko dans le pire cas théorique. En pratique un tour épuisé touche 1 à 6 outils distincts. **Pas de troncature** — en ajouter une introduirait un seuil silencieux là où le pire cas est borné et petit. |
| Coût CPU au site | Un parcours de ≤ ~40 `ToolCallSummary` + une sérialisation JSON, une fois par tour **épuisé** (20/jour en production). Hors de tout chemin chaud. |
| Fuite de secret | Impossible par construction : seuls des **noms** d'outils transitent (§ 3.3). |
| Fragilité du capteur de tracing en test | Réelle et connue ; c'est exactement ce que `keep_registry_multi_dispatcher()` traite, et la raison de réutiliser le capteur éprouvé de mika#2305 plutôt que d'en écrire un sixième (§ 5.3). |

## 9. Fire-Disposition

**Sans objet pour ce vol, et la section n'est pas inventée** (mika#2306, options
mika#1574). Les quatre candidats au statut de détecteur, refusés un par un :

- **AC1** est un test **fonctionnel** : son chemin de succès est « l'événement
  émis porte les bonnes valeurs », pas « aucune violation trouvée ». Il n'a pas
  de population à balayer.
- **AC2** est un contrôle négatif dont la population est **composée par le test
  lui-même** — il exerce un chemin qu'il a scripté, donc il ne peut rien
  *découvrir*. C'est le motif exact du précédent mika#1381
  (`docs/plans/2026-10-20-003-feat-1381-notifications-severity-tiered-operator-plan.md` § Fire-Disposition).
- **La sonde du § 7** est une commande d'opérateur dans un document, pas du code
  livré. Rien ne l'exécute, rien ne rougit.
- **Aucun lint, scan structurel, garde CI ou validateur de schéma n'est ajouté.**
  La couche pure ne balaie aucune source ; l'émetteur ne refuse rien. Le § 3.2
  écarte explicitement le contrôle de réconciliation à l'exécution, faute de
  population.

Et `scripts/check-suite-wiring.sh` n'ouvre aucune entrée : les tests livrés
vivent sous `crates/mika-agent/src/` et `crates/mika-agent/tests/`, que
`cargo test -p mika-agent` et `cargo test -p mika-agent --test eval` exécutent
déjà en CI.

## 10. Definition of Done

- [ ] `max_steps_census.rs` livre `census()` et `emit_max_steps_tool_census()`, avec les trois propriétés de lecture du § 2 écrites dans son en-tête `//!`.
- [ ] Le site d'appel est dans `mod.rs`, inconditionnel, avant le déplacement de `all_tool_summaries`, et le WARN existant est inchangé octet pour octet.
- [ ] `pub mod max_steps_census;` déclaré ; `mod test_max_steps_tool_census_2877;` ajouté à `tests/eval.rs`.
- [ ] U1-U4 verts.
- [ ] AC1 vert, avec `per_tool` assertée par égalité complète et un step à appels multiples dans la séquence.
- [ ] AC2 vert, sur le même filtre qu'AC1.
- [ ] La recette de rouge-avant du § 5.4 est écrite dans l'en-tête du fichier de test et a été exécutée : AC1 rougit, AC2 reste vert.
- [ ] `mika2515_every_loop_exit_decides_about_the_verdict_signal` toujours vert.
- [ ] `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main`, `scripts/check-secrets.sh` verts.
- [ ] Le plafond, le comportement à l'épuisement et le prompt QA sont inchangés — vérifiable au diff : aucun fichier sous `planning/` n'est touché.
- [ ] La lecture opérateur du § 7 est reportée dans `docs/agent-internals/observability.md`.

## 11. Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2877 :

- AC1 : un test du harnais pousse un tour jusqu'à `MaxSteps` avec des appels d'outils connus, et vérifie que le décompte émis par outil est exact.
- AC2 : un tour qui ne s'épuise pas n'émet pas ce décompte.
- AC3 : rouge-avant, AC1 rougit contre le code actuel.

## 12. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/max_steps_census.rs` (couche pure + émetteur) | 115 |
| tests unitaires inline du même fichier (U1-U4) | 80 |
| `crates/mika-agent/src/agent_loop/mod.rs` (déclaration + site d'appel) | 15 |
| `crates/mika-agent/tests/eval/test_context_scope_observability_2305.rs` (2 items en `pub(crate)` + doc-comment) | 8 |
| `crates/mika-agent/tests/eval/test_max_steps_tool_census_2877.rs` (AC1 + AC2 + recette rouge-avant) | 130 |
| `crates/mika-agent/tests/eval.rs` (ligne `mod`) | 2 |

Total estimé : 350 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 350 est sous la borne par vol (429)
avec 18 % de marge, et très loin du seuil par PR (1000). Trois traits du
périmètre soutiennent ce chiffre plutôt qu'un optimisme :

1. **Le site d'appel est unique** (§ 3.4) — une construction de
   `LoopResult::MaxStepsExceeded`, vérifiée, couvrant les trois modes. Un
   fan-out par mode aurait triplé la surface de production et de test.
2. **Le capteur de tracing est réutilisé et non réécrit** (§ 5.3) : 8 lignes de
   changement de visibilité au lieu de ~70 lignes de boilerplate dupliqué. C'est
   le poste qui, seul, aurait rapproché le total de la borne.
3. **Rien n'est touché en dehors de l'ajout** : le WARN existant, le plafond et
   le comportement à l'épuisement sont hors périmètre (§ 6), donc aucun test
   existant n'a de raison de bouger. Le poste « mise à jour de tests
   collatéraux » — celui qui fait déraper une estimation — est vide, et le § 3.5
   le vérifie pour la seule garde structurelle qui aurait pu l'ouvrir.
