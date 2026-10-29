# fix(manager) — le bassin admet sur le prédicat du dispatch, pas sur un lecteur plus lâche

**Ticket :** senara-solutions/mika#2956 (`bug`, `p1-important`, `dispatch:loop`, `loop-substrate`, jalon 33 « Substrat de boucle »)
**Branche :** `fix/2956/manager-le-bassin-admet-sur-un-lecteur`
**Base mesurée :** `origin/main` à `dcd0dba04`
**Lié :** mika#2952 (même classe : « vol livré », « dormant »), mika#2608 (le lecteur lâche), mika#2699 (POOL_WRITE), mika#2120 (alignement de `is_groomed` sur son écrivain), mika#2205 (deux populations sous un nom), mika#2340/mika#2331 (l'absence n'est pas zéro), mika#2923 (`match` exhaustif sans bras `_`)

---

## 1. Ce qui est mesuré

### 1.1 Le bassin admet sur un seul des trois signaux

`pool.rs`, queue de l'échelle d'admission dans `well_formed_candidates` :

```rust
if s.plan_reading == Some(PlanReading::PriorOnly) { … RefusalReason::PriorPlanOnly; continue; }
if !s.plan_present { … RefusalReason::NoGroomedPlan; continue; }
eligible.push((s, origin));
```

`plan_present` est alimenté par `reader::plan_callout_present` (`reader.rs:769-782`), qui rend vrai sur **toute** ligne de citation contenant la sous-chaîne `docs/plans/` :

```rust
if t.starts_with("> ") && t.contains("docs/plans/") { return true; }
```

Donc une admission du bassin n'atteste **aucun** des trois signaux que le gate du dispatch exige. L'écart n'est pas d'un cheveu : il est de deux signaux sur trois.

### 1.2 Le prédicat du dispatch en exige trois

`executor.rs:1822` :

```rust
pub fn check_grooming_markers(issue_body: &str) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !issue_body.contains("> - **Branch:**") { missing.push("branch_callout"); }
    if !issue_body.contains("docs/plans/") { missing.push("plan_callout"); }
    if !crate::grooming_marker::has_groomed_verdict(issue_body) { missing.push("groomed_verdict"); }
    missing
}
```

`auto_pull.rs:504`, le prédicat du feeder — l'autre écrivain de `ready` :

```rust
pub fn is_groomed(body: &str) -> bool {
    let readable = strip_fenced_blocks(body);
    crate::grooming_marker::has_groomed_verdict(&readable)
        && extract_branch_name(&readable).is_some()
        && extract_plan_path(&readable).is_some()
}
```

Les deux exigent les trois mêmes **axes** et diffèrent par la **rigueur** sur deux d'entre eux : `is_groomed` est ancré (`(?m)^> - \*\*Branch:\*\* \`([^\`]+)\``, `extract_plan_path` ancré avec préfixe de dépôt optionnel — mika#2120) et dépouille les blocs clôturés ; `check_grooming_markers` se contente de sous-chaînes non ancrées. Sur l'axe du verdict ils sont **identiques** (même appel `has_groomed_verdict`).

Conséquence qui décide tout le plan : **`is_groomed(b) ⟹ check_grooming_markers(b).is_empty()`**, et l'inverse est faux. Un corps que `is_groomed` accepte est accepté par le gate ; il existe des corps que le gate accepte et que `is_groomed` refuse (orthographes non canoniques, callout dans un bloc clôturé).

### 1.3 L'asymétrie est ratifiée par doc-comment, et le resserrement est interdit

`grooming_marker.rs`, en-tête `//!` :

> Elles restent volontairement asymétriques sur la rigueur […] L'écart va dans le sens sûr : **le lecteur strict est celui qui promeut.**
> `auto_pull` décide d'une promotion, `executor` d'un routage : […] un prédicat commun leur imposerait la rigueur du plus strict ou la tolérance du plus lâche sans que personne ait tranché lequel.

`auto_pull.rs`, doc-comment de `is_groomed` :

> `executor::check_grooming_markers` se contente de la sous-chaîne `docs/plans/`, non ancrée. **Ne le resserrez pas pour « harmoniser » les deux** […] C'est le lecteur qui rejoint son écrivain, pas l'inverse.

`plan_callout.rs:48`, table des quatre tolérances, ligne `executor` : *« intouché — resserrement **interdit** »*. La même table nomme `reader::plan_callout_present` comme le lecteur lâche **ratifié** par mika#2608, dont le doc-comment (`reader.rs:725-782`) enregistre les quatre raisons pour lesquelles la forme lâche est prescrite et conclut : *« La levée a pris la forme d'une séparation, et ce prédicat est inchangé. »*

Ce plan ne resserre donc **aucun** des deux lecteurs lâches. Il ajoute une lecture orthogonale — le motif mika#2952, déjà appliqué deux fois dans ce module (`plan_reading`, `dormancy`).

### 1.4 `plan_reading` ne rattrape pas le défaut

`plan_callout.rs:397-455` : `PlanReading::Live` ne dit que ceci — *il existe un callout `> - **Plan:** …` strict hors bloc clôturé*. Il ne dit rien du callout de branche ni du verdict GROOMED. Aucun des sept champs actuels de `PoolCandidate` (`number`, `state`, `priority_rank`, `plan_present`, `plan_reading`, `dormancy`, `labels`) ne mesure ces deux axes. Le défaut n'est donc pas réparable par un réordonnancement de l'échelle : **il manque une mesure.**

### 1.5 L'état de #2194 au 2026-10-29, et pourquoi la sonde ne porte pas sur elle

Mesuré aujourd'hui : #2194 est OPEN, sans label, hors jalon. Son corps porte depuis le 2026-10-29 une ligne 5 `> **Condition de réveil :** …` et une ligne 7 de prose `> Plan de référence : \`docs/plans/2026-10-19-005-…-plan.md\` (sur \`main\`, § 1). La branche du vol 3 n'existe plus.` Ni `**Branch:**`, ni `**Plan:**` strict, ni `Grooming history`.

Donc **#2194 est aujourd'hui captée par le bras `Dormant`** (mika#2952 vol 2/2), en amont du bras défaillant. Deux conséquences opératoires :

- le test d'AC1 utilise une **fixture synthétique** de la forme « lâche-vrai / strict-faux », pas #2194 ;
- la sonde d'AC4 porte sur la **classe** (toute ligne « À admettre »), jamais sur ce numéro.

La cause nommée par le ticket reste entière : elle valait pour #2194 avant l'ajustement de son corps, et elle vaut pour tout ticket dans la même situation.

### 1.6 La relève et le motif unique

`relief.rs:60-140` : `name_relief` filtre `.filter(|r| r.reason.remediable_by_grooming())` **avant** de calculer `beyond`. `AdmissionRefusal` porte **exactement un** motif, donc l'ordre des bras de l'échelle **est** la priorité des motifs. Réponses actuelles de `remediable_by_grooming` : `NoGroomedPlan` vrai, `EstimateOverFlightBound` vrai, `OperatorHeld` faux, `PriorPlanOnly` faux, `Dormant` faux.

### 1.7 Le « Next » ne lit pas `PoolCandidate`

`assessor.rs:240` (`tier2_pick`) et `assessor.rs:374` (tier 3) lisent `SubIssue.plan_present`, pas `PoolCandidate`. `is_in_governed_progress` de même. C'est le fondement de la fermeture d'AC3 **par non-modification** (§ 4).

### 1.8 L'instrument de parité existe déjà

`grooming_marker.rs` ~l. 560-680 : `FIXTURES` (six corps gelés via `include_str!`), `fixture_table`, et

```rust
#[test]
fn ac7_both_rust_predicates_agree_on_the_frozen_bodies() {
    for (ticket, body, _) in FIXTURES {
        let p1 = crate::auto_pull::is_groomed(body);
        let p2 = crate::skills::executor::check_grooming_markers(body).is_empty();
        assert_eq!(p1, p2, …);
    }
}
```

Plus `mika2120_repo_prefixed_plan_callout_is_read_by_both_predicates` et la garde structurelle `no_grooming_regex_outside_this_module`. **Les deux prédicats sont donc déjà d'accord sur les six corps gelés** : l'implication du § 1.2 y est vraie sans exception, ce qui est la mesure qui rend la Fire-Disposition verte à la livraison (§ Fire-Disposition).

---

## 2. La décision : ce que le bassin lit

### 2.1 Retenu — le bassin lit le jeu de signaux de `auto_pull::is_groomed`

Quatre raisons, dans l'ordre de leur force :

1. **AC4 est littéralement cette lecture.** La sonde demandée relit chaque ligne « À admettre » contre `is_groomed`. Un bassin qui admet sur un autre prédicat livrerait un invariant que sa propre sonde ne mesure pas.
2. **L'implication va dans le bon sens.** Par le § 1.2, `is_groomed ⟹ gate accepte`. Donc *admis par le bassin* ⟹ *accepté par le gate du dispatch* : exactement l'invariant qu'AC4 veut rendre vrai, et il est **structurel**, pas constaté.
3. **La doctrine écrite du dépôt tranche.** *« Le lecteur strict est celui qui promeut. »* Le bassin **promeut** — il pose `ready`. Il doit donc lire le strict.
4. **`auto_pull` est l'autre écrivain de `ready`.** Les deux écrivains partagent désormais un seul prédicat d'admission, ce qui est la forme la plus forte de la liaison qu'AC2 demande.

### 2.2 Écarté — lire `executor::check_grooming_markers`

Donnerait l'égalité exacte avec le gate, donc zéro dispatch vide par construction. Écarté parce qu'il est lâche sur l'axe du plan (sous-chaîne non ancrée) : un candidat admis sur ce prédicat pourrait **échouer à la sonde d'AC4 elle-même**, et le bassin promouvrait sur un lecteur que la doctrine réserve au routage. Le sens sûr (§ 1.3) est celui que ce plan prend.

### 2.3 Écarté — resserrer `plan_callout_present`

Interdit par mika#2608 et par la table des quatre tolérances (§ 1.3), et il changerait `tier2_pick` et le tier 3 (§ 1.7) sans que personne ait tranché. Non fait. Ce que le plan fait à la place est dit en § 4.

### 2.4 Écarté — étendre `PlanReading` d'une variante

`plan_reading` répond à *« quelle forme de callout de plan ce corps porte-t-il ? »*. Le callout de branche et le verdict ne sont pas des formes de callout de plan : les y verser ferait deux populations sous un nom (mika#2205) et rendrait `PriorOnly` non soustrayable.

### 2.5 La forme livrée

**Un lecteur, dans `auto_pull.rs`, et `is_groomed` devient sa face booléenne** — une seule computation dans l'arbre, donc pas de second site à faire dériver :

```rust
/// Les trois signaux que le grooming écrit et que le dispatch exige.
///
/// Le `snake_case` de serde rend exactement les trois jetons que
/// `executor::check_grooming_markers` pousse dans son `missing_signals` : la
/// parité de vocabulaire est donc une propriété du type, pas d'une convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroomingSignal { BranchCallout, PlanCallout, GroomedVerdict }

impl GroomingSignal {
    /// Le mot posé sur le fil (rapport, journal). Format de fil.
    pub fn token(self) -> &'static str { … }
}

/// Lesquels des trois signaux manquent — dans l'ordre de
/// `check_grooming_markers`, pour que les deux listes soient comparables
/// élément par élément.
pub fn missing_grooming_signals(body: &str) -> Vec<GroomingSignal> { … }

pub fn is_groomed(body: &str) -> bool { missing_grooming_signals(body).is_empty() }
```

L'ordre d'évaluation change (branche, plan, verdict au lieu de verdict, branche, plan) : sans effet sur le booléen, et c'est ce qui rend les deux listes `missing` alignables.

**Un champ, sur `SubIssue` et sur `PoolCandidate`**, en discipline tri-état (mika#2340/mika#2331) :

```rust
/// Lesquels des trois signaux de grooming manquent au corps.
///
/// `None` = **ce binaire n'a pas lu le corps** (blob d'un binaire antérieur à
/// ce champ), jamais « rien ne manque ». `Some(vec![])` = mesuré et groomé.
/// `Some([…])` = mesuré, et voici ce qui manque.
#[serde(default)]
pub missing_grooming_signals: Option<Vec<GroomingSignal>>,
```

Comme `plan_reading` et `dormancy`, il est dérivé du corps, donc **gratuit sur les deux chemins d'alimentation** — ce qui le distingue de `pr_state`, `ci_state` et `branch_present`, mesurés sur un seul chemin.

**Un motif de refus, distinct :**

```rust
UngroomedForDispatch { missing: Vec<GroomingSignal> },
```

`remediable_by_grooming() == true` — **c'est la réponse à la question explicite d'AC1 : oui, ce candidat entre dans `relief::name_relief`.** Un groom écrit les trois signaux (`_write_canonical_callout` est leur écrivain), donc nommer ce ticket dans la relève est un conseil vrai, contrairement à `PriorPlanOnly` et `Dormant`, qu'aucun groom ne débloque.

Distinct de `NoGroomedPlan` pour deux raisons : deux populations sous un nom ne sont pas soustractables (mika#2205) ; et « plan groomé absent » sur un corps qui cite visiblement `docs/plans/` est précisément la ligne qui a fait **mécroire le rapport** à l'opérateur le 2026-10-29.

### 2.6 La place dans l'échelle, et ce qu'y devient le lecteur lâche

Ordre des bras après ce vol :

| rang | motif | remédiable par un groom |
|---|---|---|
| 1 | `OperatorHeld` | non |
| 2 | `Dormant` | non |
| 3 | `PriorPlanOnly` | non |
| 4 | **`UngroomedForDispatch { missing }`** ou **`NoGroomedPlan`** (voir ci-dessous) | **oui** |
| 5 | `NoGroomedPlan` (plancher : corps non mesuré) | oui |

`PriorPlanOnly` reste **au-dessus** : `PlanReading::PriorOnly` implique l'absence de callout de plan strict, donc implique l'ungroomed — sans cet ordre, le motif le plus informatif serait mangé par le plus générique.

Le bras 4 :

```rust
// Le prédicat STRICT décide seul de l'admission. Le lecteur lâche de
// mika#2608 n'y a plus voix : il choisit le LIBELLÉ, et rien d'autre.
if let Some(missing) = s.missing_grooming_signals.as_deref()
    && !missing.is_empty()
{
    let reason = if s.plan_present {
        // Le corps CITE `docs/plans/` et échoue quand même au prédicat du
        // dispatch : c'est la population de mika#2956, celle dont le libellé
        // « plan groomé absent » a fait mécroire le rapport.
        RefusalReason::UngroomedForDispatch { missing: missing.to_vec() }
    } else {
        // Rien ne cite de plan : « plan groomé absent » est honnête et complet.
        RefusalReason::NoGroomedPlan
    };
    refusals.push(AdmissionRefusal { number: s.number, origin, reason });
    continue;
}
if !s.plan_present { … RefusalReason::NoGroomedPlan; continue; }
```

Deux propriétés à retenir, parce qu'elles sont le cœur de la décision :

- **L'admission ne dépend que du prédicat strict.** `plan_present` est **démis** de décideur à choisisseur-de-libellé. Il n'est ni resserré, ni retiré, ni contourné : c'est exactement ce qu'il sait faire de mieux — nommer la population où le lâche et le strict divergent.
- **La scission ne coûte rien en comportement.** Les deux motifs du bras 4 répondent `true` à `remediable_by_grooming`, donc la relève est identique quel que soit le libellé. La scission n'achète qu'une chose : un rapport lisible. Et les deux populations restent soustractables, puisqu'elles sont deux variantes et non un nom partagé.
- **`NoGroomedPlan` garde un plancher.** Quand `missing_grooming_signals == None` — blob sérialisé par un binaire antérieur à ce champ — le bras 5 décide comme aujourd'hui. L'absence du champ signifie « pas lu », jamais « rien ne manque » : le plancher est ce qui empêche l'absence de valoir admission.

### 2.7 Effet de bord, nommé plutôt que découvert

`numbers_needing_size` passe par le **même** `well_formed_candidates` que `assess_pool` (site de filtrage unique). Le nouveau bras rétrécit donc aussi la population « Taille non vérifiée » du rapport, et rend `UnreadCause::NoCallout` plus rare : un candidat refusé en amont ne demande plus d'estimation. C'est voulu — estimer un ticket non groomé est du travail jeté — et c'est ce que le test de § 5.4 ligne 4 constate.

---

## 3. Ce que le plan ne touche pas

- `reader::plan_callout_present` — **inchangé** (mika#2608 reste en vigueur, ses deux tests restent verts sans édition).
- `executor::check_grooming_markers` — **inchangé** (resserrement interdit, § 1.3).
- `plan_callout::plan_reading`, `PlanReading` — inchangés.
- `assessor.rs:240` (`tier2_pick`), `assessor.rs:374` (tier 3), `is_in_governed_progress` — **inchangés**, et c'est la fermeture d'AC3 : voir § 4.
- `grooming_marker::has_groomed_verdict` et ses regex — inchangés ; la garde `no_grooming_regex_outside_this_module` reste verte parce que le nouveau lecteur passe par `extract_branch_name` / `extract_plan_path` / `has_groomed_verdict` et n'écrit aucune regex.
- `assess_pool` reste **pure** (aucune E/S) ; la moitié écrivante reste dans `pool_writer.rs`. `no_dispatch_test.rs` ne gagne aucun nom.

---

## 4. AC3, par écrit

**Le « Next » garde son comportement, et rien dans ce vol ne peut le changer.**

`tier2_pick` (`assessor.rs:240`, `&& s.plan_present`) et le tier 3 (`assessor.rs:374`, `!s.plan_present`) lisent `SubIssue.plan_present`, alimenté par `reader::plan_callout_present`, qui n'est pas édité. Le nouveau champ est **additif** : `SubIssue` gagne `missing_grooming_signals`, aucun champ existant ne change de valeur ni de sens, et **aucun consommateur du « Next » ne lit le nouveau champ** — son seul lecteur est `pool.rs`.

Le choix de design qui garantit cela est celui du § 2.6 : parce que le bassin ajoute une lecture au lieu de resserrer le lecteur partagé, le « Next » et le bassin peuvent avoir des réponses différentes à *« ce ticket a-t-il un plan ? »* — le « Next » répond « il y a de quoi lire », le bassin répond « il y a de quoi dispatcher ». Ce sont deux questions, et mika#2608 dit laquelle `plan_callout_present` répond.

La propriété est **tenue par un test**, pas seulement par la prose : § 5.4 ligne 5.

---

## 5. Livrables

### 5.1 `crates/mika-agent/src/auto_pull.rs`

- `GroomingSignal` + `token()`, avec le doc-comment du § 2.5 (la parité de vocabulaire est une propriété de `serde(rename_all)`).
- `missing_grooming_signals(body) -> Vec<GroomingSignal>` : `strip_fenced_blocks`, puis les trois tests dans l'ordre de `check_grooming_markers`.
- `is_groomed` réécrit en face booléenne. Son doc-comment conserve **intégralement** l'interdiction de resserrer `check_grooming_markers` et la phrase *« C'est le lecteur qui rejoint son écrivain, pas l'inverse »*, et gagne un paragraphe : *ce prédicat a désormais deux consommateurs de promotion — le feeder et le bassin du manager — et c'est voulu : les deux écrivains de `ready` partagent un seul prédicat d'admission (mika#2956).*
- Les deux appelants de production (`auto_pull.rs:1968` → `FILTER_NOT_GROOMED`, `auto_pull.rs:4688`) sont inchangés.

### 5.2 `crates/mika-agent/src/milestone_manager/types.rs`

- `SubIssue` gagne `missing_grooming_signals: Option<Vec<GroomingSignal>>` avec `#[serde(default)]` et le doc-comment tri-état du § 2.5.

### 5.3 `crates/mika-agent/src/milestone_manager/reader.rs`

- `compose_outside_candidates` (~l. 647) et `compose_from_gh_outputs` (~ll. 963 / 1029) alimentent le champ depuis le corps, à côté de `plan_reading` et `dormancy`.
- Un paragraphe au doc-comment de `plan_callout_present` : le prédicat est **inchangé** et reste le lecteur de mika#2608 ; depuis mika#2956 le bassin ne l'utilise plus pour **décider** d'une admission, seulement pour choisir entre deux libellés de refus. Un futur éditeur qui vient le resserrer « pour aligner le bassin » n'a plus de motif de le faire.
- Les tests d'épinglage ll. 1267-1290 restent verts sans édition.

### 5.4 `crates/mika-agent/src/milestone_manager/pool.rs`

Production :

- Une **sixième section `//!`** pour `UngroomedForDispatch`, sur le gabarit des cinq autres : la mesure qui l'a produite (rapport du puits du 2026-10-29T10-11-41Z, #2194 hors jalon), ce qu'elle refuse, et pourquoi elle est distincte de `NoGroomedPlan`.
- `RefusalReason::UngroomedForDispatch { missing: Vec<GroomingSignal> }`, son bras de `label()` (`« grooming incomplet — manque \`branch_callout\`, \`groomed_verdict\` ; à (re)groomer »`) et son bras de `remediable_by_grooming()` (`true`, commenté : AC1). Les deux `match` restent **exhaustifs sans bras `_`** (mika#2923), donc une septième variante ne compilera pas sans que son auteur réponde à la question.
- `PoolCandidate` gagne le champ ; son doc-comment passe de « les six champs » à « les sept champs ci-dessous sont exactement ceux que `assess_pool` consulte », et le champ rejoint la liste de ceux qui sont dérivés du corps donc gratuits sur les deux chemins.
- `impl From<&SubIssue> for PoolCandidate` propage le champ.
- Le bras 4 de l'échelle, avec les deux commentaires du § 2.6.
- Le helper de test `fn cand(…)` gagne le champ (défaut : `Some(vec![])`, « mesuré et groomé », pour que les tests existants restent verts sans édition de leurs assertions).

Tests (dans `mod tests`, via `assess_pool_sans_tailles`) :

1. **AC1** — `mika2956_un_corps_qui_cite_docs_plans_sans_branche_ni_verdict_nest_pas_admis` : fixture synthétique de la forme du § 1.5 (`> Plan de référence : \`docs/plans/…-plan.md\``, ni `**Branch:**`, ni verdict), passée par le vrai `missing_grooming_signals`. Assertions : absent de `admit` ; présent dans `refusals` avec `UngroomedForDispatch` ; `missing` contient `BranchCallout` et `GroomedVerdict` et **pas** `PlanCallout` (le gate est lâche sur cet axe, et le test le dit plutôt que de le taire).
2. **AC1, relève** — `mika2956_un_refus_de_grooming_entre_dans_la_releve` : `name_relief` nomme ce ticket dans `next` ou `beyond`. Contrôle négatif dans le même test : un `Dormant` n'y entre pas.
3. **AC2, contrôle négatif** — même fonction de test que (1), second candidat : corps canoniquement groomé (les trois signaux) ⇒ **reste admis**, et `refusals` ne le contient pas. C'est la paire qui sépare « le bras refuse la bonne population » de « le bras refuse tout ».
4. **Effet de bord** — `mika2956_un_candidat_non_groome_ne_demande_plus_destimation` : `numbers_needing_size` ne contient pas le candidat de (1).
5. **AC3** — `mika2956_le_next_ne_lit_pas_le_nouveau_champ` : un `SubIssue` dont `missing_grooming_signals` vaut `Some([…])` non vide mais dont `plan_present` est vrai reste choisi par `tier2_pick` et absent du tier 3. Le test est le garde-fou de la prose du § 4 : il rougit le jour où quelqu'un câble le nouveau champ dans l'assesseur.
6. **Plancher** — `mika2956_un_corps_non_mesure_retombe_sur_lancien_plancher` : `missing_grooming_signals == None` + `plan_present == false` ⇒ `NoGroomedPlan`, comme avant. `None` ne vaut jamais admission.

### 5.5 `crates/mika-agent/src/grooming_marker.rs` — la liaison d'AC2

Le test de parité existant (`ac7_…`) assure **l'égalité** sur les six corps gelés. L'égalité est fausse en général **par conception** (§ 1.3) et son harmonisation est interdite. Le lien qu'AC2 demande est donc posé sur l'**implication**, qui est vraie partout :

```rust
/// Le bassin du manager admet sur `is_groomed` (mika#2956). Cette implication
/// est ce qui rend « admis par le bassin ⇒ accepté par le gate du dispatch »
/// structurel et non constaté. Elle ne demande PAS l'égalité : le résidu
/// (`gate` ∖ `is_groomed` — orthographes non canoniques, callout dans un bloc
/// clôturé) est refusé par le bassin dans le sens sûr, et c'est voulu.
#[test]
fn mika2956_is_groomed_implique_le_gate_du_dispatch() { … }
```

Plus `mika2956_le_vocabulaire_des_signaux_est_commun_aux_deux_predicats` : sur un corps vide, `check_grooming_markers` rend ses trois jetons dans l'ordre, et `missing_grooming_signals` rend les trois variantes dont les `token()` sont ces jetons, dans le même ordre. Une renomination d'un côté rougit : la parité de vocabulaire devient un fait testé, et le `missing` du rapport du bassin se lit avec le même grep que le `missing_signals` du refus de dispatch.

### 5.6 Essaimage littéral

`SubIssue { … }` : **28** sites (`assessor.rs`, `pool.rs`, `spawn.rs`, `db/tests/vols_termines_du_manager.rs`, `reporter.rs`, `cadence.rs`, `reader.rs`, `types.rs`, `evaluator.rs`, `tests/manager_delivery_observability_2267.rs`). `PoolCandidate { … }` : **30** sites (`assessor.rs`, `pool.rs`, `relief.rs`, `reporter.rs`, `reader.rs`). ≈ **58 ajouts d'une ligne**, mécaniques, en une passe.

---

## 6. Ordre de commit

1. `auto_pull.rs` — `GroomingSignal`, `missing_grooming_signals`, `is_groomed` réécrit, ses tests. *Compile et passe seul ; aucun consommateur encore.*
2. `grooming_marker.rs` — test d'implication + test de parité de vocabulaire. *Vert sur l'état mesuré du § 1.8.*
3. `types.rs` + `reader.rs` + les 58 sites littéraux — le champ existe et est alimenté, **personne ne le lit**. *Aucun changement de comportement ; la suite entière doit rester verte à ce commit, et c'est ce qui prouve que le vol est additif.*
4. `pool.rs` — la sixième section `//!`, la variante, le bras 4, le plancher, les six tests. *C'est le seul commit qui change un comportement.*

---

## 7. Vérification

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test -p mika-agent milestone_manager::pool
cargo test -p mika-agent auto_pull
cargo test -p mika-agent grooming_marker
cargo test -p mika-agent milestone_manager
cargo test -p mika-agent
scripts/check-byte-slices.sh
scripts/verify-pipeline.sh origin/main
```

Les trois dernières lignes ne sont pas décoratives : `check-byte-slices.sh` (mika#764) parce que le nouveau lecteur manipule des `&str` ; `verify-pipeline.sh` parce que son U2 exige la section `## Acceptance criteria` de ce plan.

---

## 8. Sonde après déploiement (AC4)

**Instant zéro :** `make deploy`, noter le sha bâti. Le bassin tourne en mode ombre (`MIKA_MANAGER_POOL_WRITE` non posé) : la sonde lit un rapport, elle n'observe aucune écriture.

**Le rapport :** premier fichier `senara-solutions-mika-33-<instant>.md` postérieur au déploiement, dans le répertoire que rend `resolve_offline_sink_dir` — `$MIKA_MANAGER_OFFLINE_SINK_DIR` si posée et non vide, sinon `$HOME/.mika/manager/sink` (`mika milestone reports` le nomme, et dit par quelle porte).

| étape | geste | lecture attendue |
|---|---|---|
| 1 | section `### Bassin ready (mode ombre — aucune écriture)`, relever chaque ligne `- À admettre : #N` | la liste des numéros à vérifier ; **liste vide ⇒ sonde non concluante**, pas sonde verte (voir ci-dessous) |
| 2 | pour chaque `#N` : `gh issue view N --repo senara-solutions/mika --json body` | le corps tel que le binaire l'a lu |
| 3 | relire ce corps contre `is_groomed` | **vrai pour chaque numéro** — c'est AC4 |
| 4 | section du bassin, lignes `- Refusé :` | au moins une ligne `grooming incomplet — manque …` si la population existe ce jour-là ; son `missing` se lit avec le même grep que le `missing_signals` d'un refus `dispatch_no_grooming_marker` |
| 5 | lignes `- Taille non vérifiée :` | population rétrécie (§ 2.7) ; aucune entrée dont le corps échoue à `is_groomed` |

**Deux mises en garde, parce qu'une sonde qui mentirait ici est pire que pas de sonde.**

- *Un rapport sans aucune admission ne vérifie rien.* L'invariant « aucune admission mal formée » est vrai par vacuité sur une liste vide. Dans ce cas, la lecture concluante est celle de l'étape 4 : un refus `grooming incomplet` prouve que le bras est atteint. Si ni admission ni refus de ce motif n'apparaissent, la sonde est à relire au rapport suivant.
- *L'étape 3 doit passer par `is_groomed`, pas par un grep.* Un `grep 'docs/plans/'` sur le corps est précisément le lecteur lâche que ce ticket corrige : il rendrait vert exactement le défaut mesuré. La forme juste est un appel au prédicat — un test `#[ignore]` prenant le corps en fixture, ou `mika` relisant le corps ; jamais une approximation textuelle.

**L'absence de la ligne `grooming incomplet` sur un rapport antérieur au déploiement n'est pas une absence de la population** : c'est un binaire qui ne savait pas la nommer (motif mika#2340).

---

## Definition of Done

- `missing_grooming_signals` et `GroomingSignal` livrés dans `auto_pull.rs`, `is_groomed` réécrit en leur face booléenne, un seul site de computation.
- Le champ `missing_grooming_signals: Option<Vec<GroomingSignal>>` (`#[serde(default)]`) porté par `SubIssue` et `PoolCandidate`, alimenté par les **deux** chemins de `reader.rs`, avec le doc-comment tri-état.
- `RefusalReason::UngroomedForDispatch { missing }` livré, `remediable_by_grooming() == true`, `label()` et `remediable_by_grooming()` toujours exhaustifs sans bras `_`.
- Le bras 4 de l'échelle décide **sur le prédicat strict seul** ; `plan_present` n'y choisit que le libellé ; `NoGroomedPlan` conserve son plancher pour un corps non mesuré.
- Sixième section `//!` de `pool.rs` écrite, avec sa mesure et sa raison de distinction.
- Les six tests du § 5.4 verts, dont le contrôle négatif d'AC2 dans la même fonction que le cas d'AC1.
- Le test d'implication et le test de parité de vocabulaire verts dans `grooming_marker.rs`.
- `plan_callout_present`, `check_grooming_markers`, `plan_reading`, `tier2_pick` et le tier 3 **non édités** ; le test d'AC3 (§ 5.4 ligne 5) le tient.
- Suite complète, clippy, `check-byte-slices.sh` et `verify-pipeline.sh origin/main` verts.
- La sonde du § 8 écrite dans le corps de la PR, avec ses deux mises en garde.

## Acceptance criteria

- **AC1.** Un candidat dont le corps cite `docs/plans/` dans une citation, mais qui n'a ni callout de branche ni marqueur GROOMED, n'est pas admis. Il porte un refus nommé. Le plan dit lequel : `NoGroomedPlan` ou un motif distinct. Il dit aussi si ce candidat entre dans la relève à grommer (`relief::name_relief`), ce qui serait cohérent ici puisqu'un groom le débloque.
- **AC2.** Contrôle négatif dans le même test : un ticket groomé selon le prédicat du dispatch (les trois signaux) reste admis. Le prédicat du bassin et celui du dispatch sont liés par un test, pour qu'ils ne puissent plus diverger en silence.
- **AC3.** Le « Next » (`tier2_pick` et le tier 3) garde son comportement, ou le plan dit par écrit ce qui change et pourquoi.
- **AC4.** Sonde après déploiement : le premier rapport du sink sous le nouveau binaire ne porte aucun « À admettre » dont le corps échoue au prédicat du dispatch. La lecture se fait en relisant chaque ligne « À admettre » contre `is_groomed` de son corps.

**Où chacun est fermé :** AC1 → § 2.5, § 2.6, § 5.4 tests 1 et 2 (motif **distinct**, `UngroomedForDispatch`, **et** il entre dans la relève). AC2 → § 5.4 test 3 (contrôle négatif, même test) + § 5.5 (la liaison, posée sur l'implication). AC3 → § 4 (par écrit) + § 5.4 test 5. AC4 → § 8.

## Fire-Disposition

Ce plan livre deux détecteurs : `mika2956_is_groomed_implique_le_gate_du_dispatch` et `mika2956_le_vocabulaire_des_signaux_est_commun_aux_deux_predicats` (§ 5.5). Les six tests de `pool.rs` sont des assertions de comportement, pas des détecteurs.

**Option retenue : (a) — exception nommée en allowlist, livrée vide.**

```rust
/// Corps du jeu gelé pour lesquels l'implication `is_groomed ⇒ gate` est
/// connue fausse. **Livrée vide** : mesuré le 2026-10-29 sous `dcd0dba04`, le
/// test d'égalité `ac7_both_rust_predicates_agree_on_the_frozen_bodies` est
/// vert sur les six corps, donc l'implication y est vraie sans exception.
/// Une entrée ici doit nommer le ticket de son corps et sa raison.
const IMPLICATION_EXCEPTIONS: &[&str] = &[];
```

Avec son assertion auto-nettoyante, dans le test lui-même :

```rust
// Le geste qui rend une exception inutile est celui qui la fait expirer.
for t in IMPLICATION_EXCEPTIONS {
    let (_, body, _) = FIXTURES.iter().find(|(x, _, _)| x == t).expect("ticket inconnu");
    assert!(!implication_holds(body), "exception stale pour {t} : l'implication y tient — retirez l'entrée");
}
```

L'allowlist est livrée vide **et** pinnée vide par le fait qu'une entrée stale rougit : l'instrument ne peut pas devenir le dépotoir que mika#2309 condamne. Aucune violation existante à excepter, donc aucun ticket de suivi n'est ouvert par cette section.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/auto_pull.rs` (lecteur + enum + doc-comments) | 78 |
| `crates/mika-agent/src/auto_pull.rs` (tests du lecteur) | 40 |
| `crates/mika-agent/src/grooming_marker.rs` (implication, vocabulaire, allowlist vide + staleness) | 28 |
| `crates/mika-agent/src/milestone_manager/pool.rs` (section `//!`, variante, `label`, `remediable`, bras, champ) | 105 |
| `crates/mika-agent/src/milestone_manager/pool.rs` (six tests) | 64 |
| `crates/mika-agent/src/milestone_manager/reader.rs` (deux sites d'alimentation + doc-comment) | 18 |
| `crates/mika-agent/src/milestone_manager/types.rs` (champ + doc-comment) | 16 |
| essaimage littéral (28 `SubIssue {` + 30 `PoolCandidate {`) | 58 |

Total estimé : 407 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 407 < 429 et très en dessous du seuil de 1000 par PR. Deux raisons de ne pas découper :

1. **Le livrable est indivisible en vols mergeables.** Découper donnerait un vol 1 livrant un champ que personne ne lit (aucun AC fermé, rien à vérifier) et un vol 2 changeant un comportement sans la mesure qui le fonde. La lecture, son consommateur et les quatre AC sont une seule chaîne causale.
2. **Le poste dominant ne coûte pas de tours.** 58 des 407 lignes sont l'essaimage littéral du § 5.6 : des ajouts d'une ligne, mécaniques, produits en une passe. La borne convertit des tours en lignes **rédigées** ; ces 58-là en consomment une fraction, ce qui laisse la marge réelle au-dessus des 22 lignes nominales.

L'ordre de commit du § 6 est construit pour que ce soit vrai même si un tour manque : les commits 1 à 3 sont verts et additifs, et seul le commit 4 change un comportement — un vol interrompu après le 3 laisse l'arbre sain.

## Hors périmètre

- **Ouvrir `MIKA_MANAGER_POOL_WRITE`** — mika#2699 le gouverne, et sa condition (« aucun `ready` mal formé admis en ombre ») est ce que ce ticket rend atteignable, jamais ce qu'il décide.
- **Harmoniser `is_groomed` et `check_grooming_markers`** — interdit par doc-comment (§ 1.3). Le résidu `gate ∖ is_groomed` est refusé par le bassin dans le sens sûr, et nommé en § 5.5.
- **Resserrer `plan_callout_present`** — mika#2608 ; conséquences sur le « Next » en § 2.3 et § 4.
- **Le quatrième lecteur du callout**, la sous-chaîne de prompt dans `self-dev/system_prompt.md` — non touché ; il n'admet rien.
- **Les orthographes de dormance autres que `Condition de réveil`** — l'ajustement du corps de #2194 le 2026-10-29 les a mises hors sujet ici ; si la classe reparaît, elle relève de mika#2952 et non de ce ticket.
- **Un rapport qui dirait pourquoi un candidat refusé l'a été sur un axe plutôt qu'un autre au-delà de `missing`** — le `missing` du libellé suffit à AC1 et à la sonde ; toute lecture plus fine est un ticket d'observabilité.
