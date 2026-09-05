---
issue: 2188
repo: senara-solutions/mika
type: fix
module: mika-agent/grooming_marker
tags: [grooming, dispatch-gate, auto-pull, loop-substrate]
problem_type: predicate-ordering
status: groomed
---

# mika#2188 — un `ESCALATE` résolu écrase le `READY` qui le suit

**Issue :** senara-solutions/mika#2188
**Branche :** `fix/2188/grooming-marker-un-escalate-r-solu-crase`
**Palier :** Tier 2 — ralentit la boucle.

---

## 1. Le fait, relu dans le code

`grooming_verdict` (`crates/mika-agent/src/grooming_marker.rs:145-165`) applique ses règles
dans cet ordre :

```rust
if let Some(last) = VERDICT_TOKEN_RE.find_iter(&text).last() {   // règle 2 — rend TOUJOURS
    return if last.as_str().starts_with("GROOMED") { Groomed } else { Escalated };
}

if FIRST_PASS_READY_RE.is_match(&text) && !LATER_PASS_RE.is_match(&text) {  // règle 3 (AC1 #2158)
    return Groomed;
}
```

La règle 3 n'est **pas** une règle de position : c'est un **repli**, atteignable seulement
quand le callout ne contient aucun token de verdict. Dès qu'un `GROOMED` ou un `ESCALATE`
apparaît *n'importe où* dans la ligne, la règle 3 devient inatteignable.

`VERDICT_TOKEN_RE` vaut `\b(GROOMED|ESCALATE[DS]?)\b`. Le tiret de `ESCALATE-divergence`
satisfait la frontière de mot finale : le token `ESCALATE` **matche à l'intérieur** de
`ESCALATE-divergence`.

Combinés, ces deux faits rendent indispatchable tout ticket qui emprunte le chemin
**prescrit** par `/mika-groom-ticket` :

```
/ce:plan → checkpoint Phase 2.5 (ESCALATE-divergence, résolu par l'opérateur)
        → réconciliation → mika-arch first-pass (READY)
```

Ordre des marqueurs sur la ligne : `ESCALATE` (dans `ESCALATE-divergence`), puis `READY`
(dans `first-pass (READY)`). Le dernier **token de verdict** est `ESCALATE` ; `READY` n'en
est pas un ; la règle 2 rend `Escalated` avant que la règle 3 ne soit lue.

Phase 2.5 est un chemin nominal de la spec de grooming — pas un cas limite. Un grooming qui
s'est **bien** passé, dont l'escalade a été **résolue par l'opérateur**, et que l'architecte
a ensuite approuvé, est lu comme escaladé.

### Le cas mesuré

**mika-cloud#205**, sonde compilée contre `origin/main` @ `500cadfb`, contrôles positif et
négatif dans le même appel (relevé porté au corps du ticket) :

```
CONTROLE-POSITIF  verdict=Groomed    groomed=true
CONTROLE-NEGATIF  verdict=Absent     groomed=false
MC205             verdict=Escalated  groomed=false      ← et NON pas Absent
```

Callout réel de mika-cloud#205, ligne 3 de son corps, relevé le 2026-09-05 :

```
> - **Grooming history:** /ce:plan → checkpoint Phase 2.5 (ESCALATE-divergence, résolu par l'opérateur) → réconciliation → mika-arch first-pass (READY)
```

`Escalated` — et non `Absent` — est la signature qui distingue cette cause de toutes les
autres : le prédicat ne dit pas « je ne sais pas lire », il dit « ce ticket est escaladé ».

---

## Acceptance criteria

Transcription littérale des six critères du corps de mika#2188. Ils sont le contrat ; §3-§4
disent *comment* on les atteint, cette section dit *ce qui doit être vrai* à la fin.

**AC1 — un `ESCALATE` antérieur résolu n'écrase plus le `READY` qui le suit.**
`grooming_verdict` rend `Groomed` sur un callout où une escalade antérieure est suivie d'une
passe architecte aboutie exprimée en disposition `READY` :
`… (ESCALATE-divergence, résolu…) → … first-pass (READY)` → `Groomed`.
Le choix entre les deux formes candidates est tranché et **argumenté** (§2 : forme (a)
retenue, (b) et (c) écartées avec leurs raisons), et l'argument est porté dans la doc-comment
du module — pas seulement dans ce plan.

**AC2 — non-régression d'ordre.** Une escalade **non** suivie d'une passe aboutie reste
`Escalated`. `escalate_without_later_groomed_is_escalated` et
`groomed_then_escalate_is_escalated_order_counts_both_ways` restent verts **sans
modification** (diff vide, attesté par `git diff --stat`).

**AC3 — non-régression de la règle AC1 de mika#2158.**
`ac1_first_pass_ready_without_second_pass_is_groomed`,
`ac1_french_first_pass_ready_without_second_pass_is_groomed`,
`word_continuation_after_groomed_is_not_a_verdict` et `iterate_alone_is_absent` restent verts
sans modification. En particulier `first-pass (READY) → second-pass (GROOMEDLY)` doit rester
`Absent` : une seconde passe annoncée dont le verdict est illisible n'est pas rattrapée par sa
première passe.

**AC4 — les six fixtures figées gardent leur verdict.** `grooming_marker::tests::FIXTURES`
reste à six entrées, `fixture_table` vert, et **aucun fichier de
`crates/mika-agent/tests/fixtures/grooming_bodies/` n'est ajouté, modifié ou supprimé.**

**AC5 — rejeu.** Une fixture portant le callout réel de mika-cloud#205 rend
`has_groomed_verdict = true` après correctif et `false` avant. Le corps **entier** de
mika-cloud#205 ne peut pas servir de fixture d'`is_groomed` : son callout `Plan` est préfixé
par le dépôt (`mika-cloud/docs/plans/…`), ce qui relève de mika#2120 et non de ce ticket — la
fixture doit **isoler la ligne `Grooming history`** (d'où la forme inline via `body_with()`,
§4.5). Le « false avant » est **mesuré et cité**, et il doit valoir `Escalated`, pas `Absent`.

**AC6 — le correctif ne fuit pas.** Il vit dans `grooming_marker.rs` et nulle part ailleurs ;
`no_grooming_regex_outside_this_module` reste vert.

---

## 2. La décision de forme, argumentée

L'AC1 du ticket laisse deux formes candidates et exige que le choix soit argumenté.

### Forme (a) — retenue : `READY` devient un marqueur d'état positionnel

Construire **une seule liste ordonnée de marqueurs d'état** sur le texte du callout, dans
l'ordre du document : les deux tokens de verdict (`GROOMED`, `ESCALATE[DS]?`) **et** les
occurrences de `READY` de première passe, ces dernières ne comptant que lorsque la condition
de désarmement AC1 de mika#2158 tient (aucune marque de passe ultérieure dans le callout).
Le **dernier** marqueur de la liste est l'état.

### Forme (b) — écartée : reconnaître le motif « escalade résolue »

Détecter `ESCALATE…` suivi d'une passe architecte aboutie, et traiter ce motif comme
neutralisant l'escalade.

### Pourquoi (a)

**1. (a) répare la cause ; (b) répare le symptôme.** La cause n'est pas que `ESCALATE-divergence`
soit mal orthographié, ni que le motif « escalade résolue » manque : c'est que la règle 3 est
un *repli* là où le module déclare, dans son propre en-tête, que le discriminateur est
**positionnel** — « la ligne de callout `Grooming history` est le contexte, et le **dernier**
token de verdict qu'elle contient est l'état ». La règle 3 introduit un état (`Groomed` par
première passe READY) qui ne participe pas à cet ordre. (a) supprime l'exception : tous les
marqueurs d'état sont dans le même ordre, et le dernier fait foi. Le module retrouve **une**
règle au lieu de deux qui se marchent dessus.

**2. (a) traite correctement le cas symétrique que (b) ne voit pas.** Un callout
`… first-pass (READY) → revue opérateur (ESCALATE)` — une escalade **postérieure** à un READY —
doit rendre `Escalated`. Sous (a) c'est automatique : le dernier marqueur est `ESCALATE`. Sous
(b), le prédicat cherche un motif « escalade puis passe aboutie » et ne dit rien de
« passe aboutie puis escalade » ; il faudrait une seconde règle. (b) coûte deux concepts là
où (a) en supprime un.

**3. (b) doit définir « une passe architecte aboutie ».** Cette définition est exactement
`FIRST_PASS_READY_RE` plus les tokens `GROOMED` — c'est-à-dire la liste de marqueurs de (a),
reconstruite pour un usage unique et local. (b) est (a) restreinte à un motif, plus le coût
de nommer le motif.

**4. Le désarmement AC1 de mika#2158 survit intact sous (a), et c'est ce qui protège l'AC3.**
La condition « aucune marque de passe ultérieure » reste une **porte globale** sur la
participation de `READY` à la liste. Elle continue de distinguer *« la seconde passe n'a pas
eu lieu parce qu'elle était inutile »* de *« la seconde passe a eu lieu et son verdict est
illisible »*. Concrètement, `first-pass (READY) → second-pass (GROOMEDLY)` : `LATER_PASS_RE`
matche, donc `READY` n'est pas un marqueur, donc la liste est vide, donc `Absent`. Sans cette
porte, (a) déclarerait ce corps groomé sur la foi de sa première passe — la régression
exacte que l'AC3 interdit.

### Forme (c) — écartée en une ligne

Resserrer `VERDICT_TOKEN_RE` pour que `ESCALATE-divergence` ne matche plus : écartée parce
qu'elle fait dépendre le verdict de l'orthographe d'un mot composé plutôt que de la
chronologie, et parce qu'une escalade Phase 2.5 **non** résolue doit rester lisible comme
escalade.

---

## 3. Le changement, fichier par fichier

Tout tient dans `crates/mika-agent/src/grooming_marker.rs`. **Aucun autre fichier de `src/`
n'est touché** — c'est l'AC6, et `no_grooming_regex_outside_this_module` la vérifie.

### 3.1 `FIRST_PASS_READY_RE` — capturer le token `READY`

La regex actuelle matche depuis `first-pass`, pas depuis `READY`. Pour ordonner correctement,
c'est la position du **token `READY` lui-même** qui compte. Ajouter un groupe :

```rust
static FIRST_PASS_READY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:first-pass|première passe|premiere passe)\s*\(\s*(READY)")
        .expect("first-pass READY regex must compile")
});
```

Le groupe 1 est désormais `READY`. Le groupe englobant devient non-capturant. Aucune autre
lecture de cette regex n'existe dans le dépôt (garde structurelle) ; le changement de
numérotation de groupe est donc local.

**Pourquoi la position du token et non celle de la phrase :** un callout de la forme
`first-pass (READY) → … (ESCALATE)` doit rendre `Escalated` ; ancrer sur `first-pass`
donnerait le même résultat ici, mais ancrer sur le token est ce qui rend l'ordonnancement
défendable sans dépendre de la longueur du préfixe de phrase.

### 3.2 `grooming_verdict` — une liste ordonnée, un dernier marqueur

Remplacer les deux branches (règle 2 / règle 3) par une construction unique :

```rust
pub fn grooming_verdict(issue_body: &str) -> GroomingVerdict {
    let Some(text) = callout_text(issue_body) else {
        return GroomingVerdict::Absent;
    };

    // Les marqueurs d'état, dans l'ordre du document : (offset, est_groomé).
    let mut markers: Vec<(usize, bool)> = VERDICT_TOKEN_RE
        .find_iter(&text)
        .map(|m| (m.start(), m.as_str().starts_with("GROOMED")))
        .collect();

    // Une première passe READY est un marqueur d'état — mais seulement si aucune passe
    // ultérieure n'est annoncée (règle AC1 de mika#2158, désarmement inchangé).
    if !LATER_PASS_RE.is_match(&text) {
        markers.extend(
            FIRST_PASS_READY_RE
                .captures_iter(&text)
                .filter_map(|c| c.get(1))
                .map(|m| (m.start(), true)),
        );
    }

    markers.sort_by_key(|(offset, _)| *offset);

    match markers.last() {
        Some((_, true)) => GroomingVerdict::Groomed,
        Some((_, false)) => GroomingVerdict::Escalated,
        None => GroomingVerdict::Absent,
    }
}
```

**Points de vigilance de l'implémentation :**

- `sort_by_key` sur l'offset seul suffit : les deux regex ne peuvent pas produire deux
  marqueurs au même offset (`VERDICT_TOKEN_RE` ne matche ni `READY` ni ses préfixes). Si
  l'implémenteur préfère une garantie explicite, `sort_by` avec un `then` déterministe est
  acceptable, mais il ne doit pas *sembler* trancher une ambiguïté qui n'existe pas.
- Les offsets sont ceux du texte **concaténé** par `callout_text`, qui joint les callouts
  empilés par `\n` dans l'ordre du document. L'ordonnancement global reste donc correct pour
  `stacked_callouts_read_in_document_order` — un callout empilé plus bas a un offset plus
  grand.
- `LATER_PASS_RE` s'évalue sur le texte concaténé entier, comme aujourd'hui. Un second
  callout portant `second-pass` désarme donc `READY` dans le premier. C'est le comportement
  actuel, préservé volontairement : le ticket ne demande pas de le changer, et le modifier
  ouvrirait une question de périmètre que l'AC ne couvre pas.

### 3.3 La documentation de la règle

La doc-comment de `grooming_verdict` et l'en-tête du module portent la règle en toutes
lettres et doivent être réécrits, pas rafistolés. Ce qui change :

- L'en-tête dit aujourd'hui « le **dernier** token de verdict qu'elle contient est l'état »
  et « Deux tokens seulement sont des verdicts […] à une exception près, la règle AC1 ».
  La formulation devient : le dernier **marqueur d'état** fait foi ; les marqueurs sont les
  deux tokens de verdict, plus le `READY` de première passe lorsqu'aucune passe ultérieure
  n'est annoncée. Il n'y a plus d'exception hors-ordre.
- La règle numérotée de `grooming_verdict` passe de quatre points (dont un repli) à trois :
  (1) pas de callout → `Absent` ; (2) le dernier marqueur d'état fait foi ; (3) aucun
  marqueur → `Absent`.
- La section « Pourquoi AC1 est désarmée par une marque de passe ultérieure » reste — le
  désarmement est inchangé et sa justification vaut mot pour mot. Y ajouter que le
  désarmement gouverne désormais la *participation* de `READY` à l'ordre, pas un repli.
- Ajouter le motif fondateur, avec sa référence : mika#2188, callout de mika-cloud#205,
  `ESCALATE-divergence` matché par `\b`, chemin nominal Phase 2.5.

---

## 4. Les tests, AC par AC

Tous dans `crates/mika-agent/src/grooming_marker.rs`, module `tests`.

### AC1 — le cas fondateur, avec le callout réel

Nouveau test, utilisant le callout **verbatim** de mika-cloud#205 :

```rust
/// mika#2188 — le chemin nominal Phase 2.5 : une escalade de réconciliation résolue par
/// l'opérateur, suivie d'un first-pass READY. Callout relevé sur mika-cloud#205 le
/// 2026-09-05, verbatim.
#[test]
fn ac1_escalate_divergence_resolved_then_first_pass_ready_is_groomed() {
    let history = "/ce:plan → checkpoint Phase 2.5 (ESCALATE-divergence, résolu par \
                   l'opérateur) → réconciliation → mika-arch first-pass (READY)";
    assert_eq!(grooming_verdict(&body_with(history)), GroomingVerdict::Groomed);
    assert!(has_groomed_verdict(&body_with(history)));
}
```

Ce test **échoue avant le correctif** (`Escalated`) et passe après. C'est aussi le porteur de
l'AC5 — voir §4.5.

### AC2 — non-régression d'ordre, dans les deux sens

`escalate_without_later_groomed_is_escalated` et
`groomed_then_escalate_is_escalated_order_counts_both_ways` restent **verts sans
modification**. Les modifier serait l'aveu que le correctif a changé la sémantique d'ordre ;
ils ne doivent pas être touchés.

Ajouter le cas symétrique que la forme (a) rend possible et que (b) manquerait :

```rust
/// mika#2188 — l'ordre compte aussi quand READY précède l'escalade. Une escalade
/// POSTÉRIEURE à une passe aboutie reste une escalade.
#[test]
fn first_pass_ready_then_later_escalate_is_escalated() {
    assert_eq!(
        grooming_verdict(&body_with(
            "mika-arch first-pass (READY) → revue de périmètre opérateur (ESCALATE)"
        )),
        GroomingVerdict::Escalated
    );
}
```

### AC3 — non-régression de la règle AC1 de mika#2158

Restent verts **sans modification** :
`ac1_first_pass_ready_without_second_pass_is_groomed`,
`ac1_french_first_pass_ready_without_second_pass_is_groomed`,
`word_continuation_after_groomed_is_not_a_verdict`,
`iterate_alone_is_absent`.

`word_continuation_after_groomed_is_not_a_verdict` est le test le plus exposé du lot : c'est
lui qui vérifie que `first-pass (READY) → second-pass (GROOMEDLY)` reste `Absent`. Sa verdeur
est ce qui atteste que la porte `LATER_PASS_RE` gouverne bien la participation de `READY` à
l'ordre, et non plus seulement un repli. Si l'implémenteur se trouve tenté de le modifier,
c'est le signe que la porte a été perdue dans la réécriture.

Restent verts également, sans modification : `legacy_parameterized_and_annotated_groomed_forms_still_match`
(les cinq formes héritées), `canonical_second_pass_groomed`, `ac2_french_second_pass_is_groomed`,
`ac3_groomed_after_escalate_and_arbitration_is_groomed`,
`verdict_is_read_from_any_producer_not_only_second_pass`,
`stacked_callouts_read_in_document_order`, `ungroomed_is_not_a_verdict`,
`prose_groomed_outside_the_callout_is_absent`, `empty_body_is_absent`,
`body_without_grooming_history_callout_is_absent`.

### AC4 — les six fixtures figées gardent leur verdict

`fixture_table` reste vert sans modification, y compris son assertion `FIXTURES.len() == 6`.
`ac7_both_rust_predicates_agree_on_the_frozen_bodies` et, côté `auto_pull.rs`,
`test_is_groomed_six_frozen_bodies` restent verts.

**Aucun fichier de `crates/mika-agent/tests/fixtures/grooming_bodies/` n'est ajouté, modifié
ou supprimé.** Le README de ce répertoire interdit le rafraîchissement des six ; ce ticket
n'y touche pas et n'en ajoute pas de septième — voir §4.5 pour pourquoi la fixture de l'AC5
ne peut pas y vivre.

### AC5 — le rejeu, et pourquoi il est inline et non un fichier

L'AC5 exige un rejeu portant le callout réel de mika-cloud#205, `true` après correctif et
`false` avant, et interdit d'utiliser le corps entier de mika-cloud#205 comme fixture
d'`is_groomed`. Trois raisons convergentes imposent la forme **inline via `body_with()`**
(le test de §4.1), et non un fichier dans `grooming_bodies/` :

1. **La contrainte que l'AC5 nomme.** Le callout `Plan` de mika-cloud#205 est préfixé par le
   dépôt (`mika-cloud/docs/plans/…`). `auto_pull::is_groomed` exige
   `> - **Plan:** \`docs/plans/` collé au backtick ; le corps entier échouerait sur la
   condition `Plan`, qui est **mika#2120** et non ce ticket. `body_with()` fournit un callout
   `Plan` en forme nue, ce qui isole strictement la ligne `Grooming history`.
2. **`fixture_table` fige le compte à six.** Un fichier de plus dans `FIXTURES` fait échouer
   `assert_eq!(FIXTURES.len(), 6)` — c'est-à-dire l'AC4. Ajouter un septième fichier
   *hors* de `FIXTURES` ferait de `grooming_bodies/` un répertoire à deux régimes, dont le
   README ne décrit qu'un.
3. **La provenance reste honnête sans le répertoire.** Ce que l'AC5 demande de mesurer est la
   **ligne de callout**, relevée verbatim ; le doc-comment du test porte sa provenance
   (mika-cloud#205, relevé le 2026-09-05). Le régime « corps historique figé, ne pas
   rafraîchir » du répertoire s'applique à des corps entiers reconstruits, pas à une ligne
   citée.

**Le « false avant » est vérifié explicitement, pas supposé.** Avant de modifier
`grooming_verdict`, l'implémenteur écrit d'abord le test de §4.1 et **le voit échouer** en
rendant `Escalated` — pas `Absent`. `Escalated` est la signature de la cause décrite ici ;
`Absent` signalerait une autre cause et invaliderait le diagnostic du plan. Le rapport de
travail doit citer la sortie de cet échec. Un test qui n'a jamais été rouge n'atteste rien.

### AC6 — le correctif ne fuit pas hors du module

`no_grooming_regex_outside_this_module` reste vert. Le seul fichier de `crates/mika-agent/src/`
modifié est `grooming_marker.rs`. `auto_pull.rs` et `skills/executor.rs` ne sont pas touchés :
ils appellent déjà `has_groomed_verdict` et héritent du correctif sans changer d'une ligne.

**Le troisième porteur — la garde Bash de `dispatch-lib.sh` — n'est pas concerné, et c'est
mesuré, pas supposé.** `_committed_plan_on_branch` ne lit **aucun** marqueur de verdict : elle
vérifie les callouts `Branch`/`Plan` et l'existence du plan commité sur la branche. Les
occurrences de `GROOMED`/`first-pass` dans `dispatch-lib.sh` (lignes ~4149-4330) parsent les
**réponses de l'architecte**, pas les corps d'issue — une population de textes disjointe.
`test_groom_gate_refusal_implies_rust_says_groomed` reste donc vert sans modification.

---

## 5. Vérification

```bash
cargo test -p mika-agent grooming
cargo test -p mika-agent auto_pull
cargo clippy -p mika-agent --all-targets -- -D warnings
cargo fmt --check
bash skills/bundled/_shared/test-dispatch-lib.sh
```

Attendu : tous verts. Les quatre tests nommés en AC2/AC3 comme « verts sans modification »
doivent apparaître verts **et** leur diff doit être vide — le rapport de travail cite
`git diff --stat` pour l'attester.

**Preuve du rouge-avant (obligatoire).** Le rapport cite la sortie de l'exécution du test de
§4.1 sur le code non modifié, montrant `Escalated`. Sans elle, le correctif n'a pas de
contrôle négatif.

---

## Fire-Disposition

Ce plan livre des **détecteurs** : des tests qui, en devenant rouges, accusent quelque chose.
Cette section dit d'avance quoi faire quand ils tirent — pour qu'aucune décision ne soit prise
sous la pression d'une suite rouge.

| détecteur | s'il tire | disposition |
|---|---|---|
| `fixture_table` — une des six fixtures figées bascule de verdict | le correctif a changé la sémantique sur des corps réels | **(c) halte-et-remontée.** Ne pas ajuster le tableau attendu, ne pas rafraîchir la fixture. Le README de `grooming_bodies/` le dit déjà pour le rafraîchissement (« S'il bouge, ne corrigez pas le tableau ») ; c'est ici la règle symétrique côté code. Arrêter, remonter à l'opérateur avec la fixture et le verdict obtenu. |
| `ac7_both_rust_predicates_agree_on_the_frozen_bodies` — désaccord entre `auto_pull::is_groomed` et `executor::check_grooming_markers` | le correctif a atteint un seul des deux appelants | **(c) halte-et-remontée.** Ce croisement est l'AC7 de mika#2158 ; un désaccord signifie que la centralisation a été défaite. |
| `mika2120_divergence_is_still_open_and_this_test_pins_it` — devient rouge | le correctif a débordé sur la condition `Plan`, qui est **sous arbitrage opérateur** | **(c) halte-et-remontée.** Ne pas supprimer ce test « puisqu'il gêne » : sa suppression est prévue, mais dans le commit qui rend mika#2120, pas ici. |
| `no_grooming_regex_outside_this_module` — devient rouge | une regex de marqueur a été recopiée hors du module | **(c) halte-et-remontée.** C'est l'AC6, et la garde a fait exactement son travail. |
| les quatre tests d'AC3 / les deux d'AC2 — deviennent rouges | la porte `LATER_PASS_RE` ou la sémantique d'ordre a été perdue dans la réécriture | **(c) halte-et-remontée.** Ces tests ne doivent **pas** être modifiés pour redevenir verts : leur modification serait l'aveu que le correctif a changé une sémantique que les AC interdisent de changer. |
| le test AC5 (`ac1_escalate_divergence_resolved_then_first_pass_ready_is_groomed`) — rouge **avant** correctif | attendu, c'est le contrôle négatif | **Poursuivre — mais lire la valeur.** `Escalated` confirme le diagnostic ; **`Absent` l'invalide** et impose la halte : la cause serait autre que celle décrite au §1, et le plan devrait être re-groomé plutôt qu'implémenté. |

**Allowlist d'exceptions : vacante.** Aucun test existant n'est autorisé à être modifié,
désactivé, `#[ignore]`é ou vu son attente ajustée dans le périmètre de ce ticket. Le seul
fichier de `src/` modifié est `grooming_marker.rs`, et les seuls tests ajoutés sont les deux
nommés au §4.

---

## 6. Ce que ce plan ne fait pas

- **mika#2120** — le préfixe dépôt dans le callout `Plan`. C'est la **seconde** cause pour
  laquelle mika-cloud#205 reste invisible ; elle est sous arbitrage opérateur et pinnée par
  `mika2120_divergence_is_still_open_and_this_test_pins_it`, qui doit rester vert. Corriger
  mika#2188 ne rend **pas** mika-cloud#205 dispatchable à lui seul : les deux sont
  nécessaires. Ne pas prétendre le contraire dans le corps de la PR.
- Le choix de `N` / du pool de l'auto-feeder.
- Les conditions `Branch` / `Plan` des deux appelants, qui restent chez eux et divergentes.
- Le comportement de `LATER_PASS_RE` sur les callouts empilés (§3.2), préservé tel quel.

---

## 7. Definition of Done

- [ ] AC1 — le callout Phase 2.5 « escalade résolue → first-pass READY » rend `Groomed`, testé
      sur le callout verbatim de mika-cloud#205.
- [ ] AC2 — `escalate_without_later_groomed_is_escalated` et
      `groomed_then_escalate_is_escalated_order_counts_both_ways` verts, diff vide ; cas
      symétrique `first_pass_ready_then_later_escalate_is_escalated` ajouté et vert.
- [ ] AC3 — les quatre tests de non-régression de mika#2158 verts, diff vide ; en particulier
      `first-pass (READY) → second-pass (GROOMEDLY)` reste `Absent`.
- [ ] AC4 — `fixture_table` vert, `FIXTURES.len() == 6`, aucun fichier de
      `grooming_bodies/` touché.
- [ ] AC5 — rejeu inline vert après correctif, **et sortie du rouge-avant citée dans le
      rapport**, montrant `Escalated` (pas `Absent`).
- [ ] AC6 — `no_grooming_regex_outside_this_module` vert ; `grooming_marker.rs` est le seul
      fichier de `src/` modifié.
- [ ] Le choix de forme (a) et le rejet de (b) sont portés dans la doc-comment du module, pas
      seulement dans ce plan — la prochaine divergence doit être une régression, pas une
      redécouverte.
- [ ] `cargo clippy -D warnings`, `cargo fmt --check`, `test-dispatch-lib.sh` verts.
