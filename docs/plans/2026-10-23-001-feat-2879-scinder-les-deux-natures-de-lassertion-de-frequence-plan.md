# mika#2879 — Vol 2/4 : scinder les deux natures de l'assertion de fréquence, et les rendre soustractibles

> **Ce vol ne rattache rien à la classe P0.** Il livre la **précondition
> structurelle** du rattachement — la scission du détecteur en ses deux natures,
> à comportement de détection inchangé *par construction* — et l'instrument qui
> dit, pour la première fois, **laquelle des deux** récidive en production. Le
> rattachement de la moitié accomplie part au vol 3, le fixture de collision au
> vol 4. La raison est la borne par vol de mika#2693, et elle est chiffrée en
> § *Taille estimée*.

---

## Le constat, et ce qui le rend décidable

Le détecteur `detect_unactioned_frequency_promise` (garde 5e, mika#2358) tient
**douze** alternatives d'assertion dans une seule constante
(`FREQUENCY_ASSERTION_ALTERNATION`, `guards.rs:1169-1182`). mika#2878 a établi
qu'elles sont de **deux natures**, et que cette distinction décide de
l'appartenance à la classe P0 `fabrication-detected` :

| nature | alternatives | état système posé ? | classe `fabrication-detected` |
|---|---|---|---|
| **engagement sur l'avenir** (8) | `je vais {corriger,régler,arrêter,réduire,changer,ajuster,limiter,baisser,espacer,couper}`, `je {corrige,règle,…}`, `je ne t'envoie plus`, `je ne t'enverrai plus`, `je n'enverrai plus`, `i'll {fix,stop,…}`, `i will {fix,…}`, `i {won't,will not} send` | **non** — rien n'est affirmé du présent | **hors classe** |
| **état accompli** (4) | `c'est {corrigé,réglé,fait,bon,réparé}`, `i've {fixed,stopped,reduced,changed,adjusted,limited,cut}`, `i have {…}`, `{it's,that's,it is} {fixed,done,sorted}` | **oui** — et le terme (C) du détecteur garantit qu'aucun `set_config` ne le porte | **dans la classe** |

Le critère est celui de `PendingFabricationP0Site` (mika#2872 D1, du bearing de
Prime du 2026-10-22) : *la phrase fautive affirme-t-elle un état du système
qu'aucun résultat d'outil de ce tour ne porte ?* Pour « c'est corrigé », oui — et
le terme (C) (`frequency_actor_called`, `guards.rs:1307-1316`) a déjà **prouvé**
que non.

**Aujourd'hui ces deux populations partagent un nom d'événement.**
`guard.unactioned_frequency_promise_uncorrected` (`agent_loop/mod.rs:3053`) porte
`matched_assertion` — le fragment brut — et rien qui dise sa nature. C'est
exactement la classe mika#2205 : deux populations sous un seul nom ne sont pas
soustractibles, et personne ne peut répondre à « combien de ces récidives sont
des états accomplis ? ». Or c'est **la** question dont dépend l'armement de la
porte (mika#2871), dont la Fire-Disposition de mika#2872 dit en propres termes
que « la fréquence réelle de la récidive n'est pas mesurée ».

Ce vol répond à cette question et ne fait rien d'autre.

---

## D1 — La scission est une **composition**, pas une réécriture

C'est le cœur du vol, et c'est ce qui rend AC2 vrai par construction plutôt que
par échantillonnage.

`FREQUENCY_ASSERTION_ALTERNATION` est découpée en deux constantes, et l'union est
**reconstruite** à partir d'elles en un site unique :

```rust
/// **Nature « état accompli ».** La phrase affirme un état du système — et le
/// terme (C) du détecteur garantit qu'aucun `set_config` ne le porte. C'est la
/// moitié qui appartient à la classe `fabrication-detected` (mika#2879).
const FREQUENCY_ASSERTION_ACCOMPLISHED_STATE: &str = r"(?:
      c'?est\s+(?:corrig[ée]|r[ée]gl[ée]|fait|bon|r[ée]par[ée])
    | i'?ve\s+(?:fixed|stopped|reduced|changed|adjusted|limited|cut)
    | i\s+have\s+(?:fixed|stopped|reduced|changed|adjusted|limited|cut)
    | (?:it'?s|that'?s|it\s+is)\s+(?:fixed|done|sorted)
)";

/// **Nature « engagement sur l'avenir ».** Un engagement de comportement, pas un
/// état. Hors de la classe `fabrication-detected` : rien n'est posé qu'un humain
/// doive aller vérifier.
///
/// Les deux formes au **présent** (`je corrige`, `je ne t'envoie plus`) sont ici
/// et non au-dessus, et le choix est délibéré : c'est un présent de décision, pas
/// un constat. Personne n'écrit « je ne t'envoie plus » pour rapporter un état
/// déjà atteint — la phrase annonce la conduite à venir. Le classer en accompli
/// réveillerait l'opérateur sur un engagement, exactement le faux positif que ce
/// vol existe pour rendre impossible.
const FREQUENCY_ASSERTION_FUTURE_COMMITMENT: &str = r"(?:
      je\s+vais\s+(?:l[ae]\s+|les\s+|l'|t'|te\s+|y\s+|en\s+)?(?:corriger|r[ée]gler|arr[êe]ter|r[ée]duire|changer|ajuster|limiter|baisser|espacer|couper)
    | je\s+(?:l[ae]\s+|les\s+|l'|y\s+|en\s+)?(?:corrige|r[ée]gle|arr[êe]te|r[ée]duis|change|ajuste|limite|espace|coupe)\b
    | je\s+n(?:e\s+t'?|'?)envoie\s+plus
    | je\s+ne?\s*t'?enverrai\s+plus
    | je\s+n'?enverrai\s+plus
    | i'?ll\s+(?:fix|stop|reduce|change|adjust|limit|lower|cut|send)
    | i\s+will\s+(?:fix|stop|reduce|change|adjust|limit|lower|cut|send)
    | i\s+(?:won'?t|will\s+not)\s+send
)";

/// L'union des deux natures, et **le seul site qui les assemble**.
///
/// Les deux groupes nommés imbriqués sont ce qui rend la nature **exacte** : ce
/// n'est pas une re-classification du fragment capturé, c'est le moteur de regex
/// qui dit quelle branche a apparié. Une frontière de mot devinée de travers
/// n'est donc pas un état atteignable.
static FREQUENCY_ASSERTION_ALTERNATION: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| {
        format!(
            r"(?P<assert>(?P<accompli>{FREQUENCY_ASSERTION_ACCOMPLISHED_STATE})\
              |(?P<engagement>{FREQUENCY_ASSERTION_FUTURE_COMMITMENT}))"
        )
    });
```

Les deux regex de détection (`FREQ_ASSERT_THEN_SUBJECT_RE:1184`,
`FREQ_SUBJECT_THEN_ASSERT_RE:1193`) interpolent `&*FREQUENCY_ASSERTION_ALTERNATION`
là où elles interpolaient `(?P<assert>{FREQUENCY_ASSERTION_ALTERNATION})`.

**Trois propriétés de cette forme, toutes les trois load-bearing :**

1. **La détection lit l'union, et l'union EST la concaténation des deux
   moitiés.** Une alternative ne peut donc pas tomber au sol : si elle disparaît
   d'une moitié, elle disparaît de l'union, et les dix tests existants de 5e
   rougissent. AC2 n'est pas un test d'échantillonnage, c'est une propriété de
   la construction.
2. **La nature n'est jamais re-calculée.** `caps.name("accompli").is_some()` lit
   la branche que le moteur a prise. L'alternative — ré-apparier le fragment
   capturé contre une regex ancrée — introduisait un troisième prédicat, une
   frontière à deviner et un cas « inclassable » avec son défaut à nommer. Rien
   de tout cela n'existe ici.
3. **L'ordre d'évaluation de la détection ne bouge pas.** Faire **deux** passes
   (accompli d'abord, engagement ensuite) aurait changé *quelle* violation est
   retournée sur un texte qui porte les deux — donc `matched_assertion`, donc un
   comportement observable. Une seule passe sur l'union, puis lecture de la
   branche : AC2 tient.

### Le type de la nature, et le champ qui la porte

```rust
/// La nature de l'alternative que la couche B a appariée (mika#2879).
///
/// Ce n'est pas une commodité de journal : c'est le discriminant qui décide de
/// l'appartenance à la classe P0 `fabrication-detected` (voir
/// `PendingFabricationP0Site`), et le vol 3 en fait un terme de garde. Il est
/// `Copy` et à deux bras pour que ce terme s'écrive `== AccomplishedState` et
/// qu'un troisième bras ne puisse pas apparaître sans casser les `match`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrequencyAssertionNature {
    /// « c'est corrigé », « i've stopped » — un état du système est posé, et le
    /// terme (C) a prouvé qu'aucun `set_config` ne le porte.
    AccomplishedState,
    /// « je vais réduire », « i'll stop » — un engagement sur l'avenir. Aucun
    /// état posé, donc rien à aller vérifier, donc hors classe.
    FutureCommitment,
}
```

`FrequencyPromiseMatch` (`guards.rs:1121-1126`) gagne un troisième champ
`nature: FrequencyAssertionNature`, rempli dans
`first_surviving_frequency_promise` (`:1347-1350`) :

```rust
return Some(FrequencyPromiseMatch {
    subject: caps.name("subj")?.as_str().to_string(),
    assertion: caps.name("assert")?.as_str().to_string(),
    nature: if caps.name("accompli").is_some() {
        FrequencyAssertionNature::AccomplishedState
    } else {
        FrequencyAssertionNature::FutureCommitment
    },
});
```

---

## D2 — Le consommateur de production est la sonde, et c'est voulu

Un discriminant que rien ne lit en production est du code mort, et le plan de
mika#2878 a refusé nommément « un vol dont le seul livrable est un type sans
second consommateur ». Ce vol a donc son consommateur, et c'est un **instrument**.

Les deux `warn!` de 5e gagnent un champ `assertion_nature` :

| site | événement | ce que le champ ajoute |
|---|---|---|
| `agent_loop/mod.rs:2978-2990` | `guard.unactioned_frequency_promise` | de quelle nature sont les promesses que le re-prompt **corrige** |
| `agent_loop/mod.rs:3044-3056` | `guard.unactioned_frequency_promise_uncorrected` | de quelle nature sont celles qui **sortent quand même** — la population du vol 3 |

```rust
assertion_nature = ?promise.nature,
```

**C'est le remède à la classe mika#2205, pas un ornement.** Avant ce champ, la
requête « combien de récidives de 5e sont des états accomplis ? » n'a pas de
réponse : les deux populations sont sous un nom et ne se soustraient pas. Après,
elle en a une, et c'est la donnée dont dépendent deux décisions déjà ouvertes —
le périmètre du vol 3 (quelle fraction de la population il rattache) et
l'armement de la porte (mika#2871, qui a besoin de la fréquence réelle).

**Sonde post-déploiement** (l'instrument n'est pas vivant avant `make deploy`,
motif mika#2340 — l'absence du champ signifie un binaire antérieur à ce vol, et
jamais « aucune récidive accomplie ») :

```
grep -h 'guard.unactioned_frequency_promise_uncorrected' /var/log/mika/server.log \
  | jq -r '.fields.assertion_nature' | sort | uniq -c
```

---

## D3 — Le détecteur structurel : un seul site d'union

**Le mode de régression que ce vol crée.** Deux constantes publiques dans le
module invitent un futur éditeur à construire une regex « juste pour la moitié
accomplie » — par souci de performance, ou pour un troisième consommateur. Le
jour où cela arrive, la détection se dédouble : deux passes, deux ordres
d'évaluation, et un `matched_assertion` qui change sans qu'aucun test de
comportement ne le voie. C'est précisément la classe que les scans « ce prédicat
a un seul lecteur » du dépôt existent pour refuser (mika#1833, mika#2425).

**Livrable :** `mika2879_les_deux_moities_nont_quun_seul_site_dunion` — un scan
de source sur `crates/mika-agent/src/`, hors fichiers de test (via
[`crate::source_scan::is_test_source_path`], le prédicat déjà en service), qui
refuse toute occurrence de `FREQUENCY_ASSERTION_ACCOMPLISHED_STATE` ou de
`FREQUENCY_ASSERTION_FUTURE_COMMITMENT` ailleurs qu'à leur ligne de déclaration
et dans le `format!` de l'union. **Allowlist livrée vide**, et pinnée vide par
l'assertion elle-même.

Avec son contrôle de bonne foi —
`mika2879_le_scan_dunion_rougit_sur_un_second_site` — parce qu'un scan dont on
n'a pas vu le rouge n'atteste rien : il injecte une chaîne de source contenant un
second usage et vérifie que le prédicat la refuse. Le prédicat s'ancre sur les
lignes **non commentaires**, comme ses aînés : la prose *à propos* des deux
constantes (il y en a trois paragraphes ci-dessus et autant dans les
doc-comments) n'est pas un usage.

---

## D4 — Ce que ce vol ne touche pas

Aucune ligne de :

- `PendingFabricationP0Site` — pas de troisième variante. Le vol 3 la nomme.
- La branche `_uncorrected` de 5e **au-delà du champ de `warn!`** — aucun dépôt
  dans `pending_fabrication_p0`.
- `still_violated` — pas de troisième bras.
- `config.rs` / `SETTABLE_CONFIG_KEYS` / le défaut de
  `MIKA_FABRICATION_P0_NOTIFY`. **La porte reste désarmée** (AC5), et ce vol ne
  l'approche pas : il n'ouvre aucun site qui la lirait.
- Les dix tests de 5e dans `guards.rs` (`:5624-5823`) et les quatre de
  `tests/eval/test_unactioned_frequency_promise_2358.rs` — **verts sans
  modification.** C'est le plancher du vol : un seul de ces quatorze tests touché
  voudrait dire que la scission a changé la détection, et AC2 serait faux.
- Les six fixtures et trois tests de `test_fabrication_detected_p0_2872.rs` et
  `_2867.rs` — intacts, puisque ni le créneau ni le site d'acceptation ne sont
  touchés.

---

## Phase unique — l'ordre de commit

1. **`guards.rs`** — les deux constantes de nature et l'union à site unique (D1,
   premier bloc). `cargo test -p mika-agent evidence::guards::tests` :
   **les dix tests de 5e doivent rester verts sans modification.** C'est le
   plancher, et il est vérifié avant d'ajouter quoi que ce soit.
2. **`guards.rs`** — `FrequencyAssertionNature`, le champ `nature`, sa lecture
   dans `first_surviving_frequency_promise`. `cargo check -p mika-agent`.
3. **`guards.rs` tests** — la table de classification des douze alternatives
   (D1), par nature attendue.
4. **`agent_loop/mod.rs:2978` et `:3044`** — le champ `assertion_nature` sur les
   deux `warn!` (D2).
5. **`guards.rs` tests** — le scan d'union et son contrôle de bonne foi (D3).
6. `cargo fmt`, `cargo clippy`, `cargo test -p mika-agent`, les lints du dépôt,
   `scripts/verify-pipeline.sh origin/main`. Les recettes de rouge-avant de la
   § *Risques* sont jouées **avant** l'ouverture de la PR, et leur résultat — y
   compris celles qui resteraient vertes, avec leur raison — écrit dans l'en-tête
   de doctrine du module de test.

---

## Fire-Disposition

**Option (a) — exception nommée en allowlist, et l'allowlist est livrée VIDE.**

Ce vol livre deux détecteurs au sens de la règle mika#2306 : la table de
classification (D1, chemin de succès « chaque alternative porte la nature
attendue ») et le scan d'union (D3, chemin de succès « aucun second site »).

| détecteur | violations existantes | exception |
|---|---|---|
| table de classification des douze alternatives | **zéro** — les douze sont réparties par ce vol même, donc aucune n'arrive déjà mal classée. Il n'y a pas d'état antérieur à excepter. | aucune |
| scan d'union sur `src/` | **zéro, vérifié** : `grep -n FREQUENCY_ASSERTION_ALTERNATION crates/mika-agent/src/` ne rend que trois lignes — la déclaration (`:1169`) et les deux interpolations (`:1187`, `:1196`), toutes trois réécrites par ce vol. Les deux nouvelles constantes n'existent pas encore, donc leur population d'usages hors-union est vide par construction. | aucune |

**L'allowlist du scan est donc un tableau vide, et c'est une assertion, pas un
défaut.** Un test frère la pinne vide (le motif de
`mika1833_the_zero_budget_predicate_has_a_single_reader`) : le jour où quelqu'un
y ajoute une entrée, il doit nommer la donnée, référencer un ticket de suivi, et
l'assertion auto-nettoyante rougit quand l'exception devient stale.

**Rien n'est livré désarmé, et aucun test ne porte `#[ignore]`.** La raison est
que ce vol n'ouvre aucun site de notification : il n'y a pas de porte à fermer
ici. La porte `MIKA_FABRICATION_P0_NOTIFY` reste à son défaut `false` (AC5) parce
qu'aucune ligne de ce vol ne la lit ni ne la change — c'est le vol 3 qui en aura
besoin, et c'est lui qui livrera son contrôle à porte fermée.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/evidence/guards.rs` (scission en deux constantes de nature + union à site unique + doc-comments du critère) | 58 |
| `crates/mika-agent/src/evidence/guards.rs` (`FrequencyAssertionNature`, groupes nommés, champ `nature` + sa lecture) | 40 |
| `crates/mika-agent/src/evidence/guards.rs` tests (table de classification des douze alternatives) | 55 |
| `crates/mika-agent/src/evidence/guards.rs` tests (scan d'union D3 + son contrôle de bonne foi) | 45 |
| `crates/mika-agent/src/agent_loop/mod.rs` (`assertion_nature` sur les deux `warn!` de 5e) | 10 |

Total estimé : 208 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et pourquoi le ticket entier ne tenait pas.**

Le périmètre complet du ticket — scission, rattachement, trois tests moteur,
fixture de collision — a été estimé livrable par livrable avant ce découpage, et
il vaut **≈ 460 lignes**, soit 1,65 × la borne :

| point du ticket | estimation |
|---|---|
| 1. scission + discriminant + ses attestations (ce vol) | 208 |
| 2. rattachement : variante de site, bras de `still_violated`, dépôt dans la branche 5e, et **trois** tests moteur (positif accompli, négatif engagement, porte fermée) avec leur en-tête de doctrine | ≈ 280 |
| 3. fixture de collision 5d + `assert_grounded` et son rouge-avant sur le verrou de classe | ≈ 110 |

Le point 2 seul frôle la borne : la mesure vient du vol 1 de ce lignage, où
**trois** tests moteur de cette exacte famille ont coûté **369 lignes réelles**
(`test_fabrication_detected_p0_2872.rs`) pour 150 estimées. Ce facteur 2,5 est la
raison pour laquelle l'estimation ci-dessus ne reprend pas les chiffres du plan
de mika#2878 (qui annonçait « ≈ 145 lignes » pour ce vol) : ce plan-là a livré
567 lignes réelles pour 266 estimées, et le recalibrer sur la mesure est
précisément ce que la règle mika#2636 demande.

**Quatre arguments de marge structurelle pour le vol courant**, tous vérifiés
plutôt que supposés :

1. **Les douze alternatives existent déjà** (`guards.rs:1169-1182`). La scission
   les redistribue, elle ne les écrit pas : le coût est en doc-comments, pas en
   regex.
2. **Zéro ligne de test `tests/eval/`.** Tout ce que ce vol atteste est
   atteignable dans le `mod tests` inline de `guards.rs`, sur un détecteur pur
   et un scan de source — pas de harnais, pas de `MessageSender`, pas de
   séquence de récidive. C'est le poste qui a fait exploser le vol 1, et il est
   absent ici.
3. **Zéro ligne de `config.rs`, de harnais ou de déclaration de module.** Aucun
   setter à gagner, aucun `pub mod` à ajouter dans `tests/eval.rs`.
4. **Le prédicat de classification de fichier de test existe** —
   `crate::source_scan::is_test_source_path` (`src/source_scan.rs:79`), consommé
   par dix-neuf modules du crate. Le scan D3 l'appelle au lieu de le réinventer,
   et hérite donc de la prémisse déjà réparée par mika#2335 / mika#2305 /
   mika#2361 (un module de test extrait ne porte pas le littéral `#[cfg(test)]`,
   donc une troncature naïve le lirait comme de la production).

La marge est de 71 lignes, soit 25 %.

**Tracked in: senara-solutions/mika#2879** — le ticket reste ouvert et porte les
deux vols suivants, comme mika#2872 a porté celui-ci. **Vol 3** : le rattachement
de la moitié accomplie à la classe `fabrication-detected` (variante
`PendingFabricationP0Site`, bras de `still_violated`, dépôt dans la branche 5e
derrière `fabrication_p0_notify` **et** `pending_fabrication_p0.is_none()`, et ses
trois tests moteur) — il ferme AC1 au niveau de la classe, AC4 et AC5. **Vol 4** :
le fixture de collision 5d + `assert_grounded` qui attesterait le verrou de
classe — il ferme AC3. Ce dernier est **indépendant des deux premiers** : la
collision 5d + `assert_grounded` est déjà atteignable sur `main` depuis d51da321,
puisque ses deux sites y sont rattachés. Le ticket l'ordonne en dernier, et
l'ordre est le bon — avec le vol 3 mergé, le vol 4 pourra choisir sa collision
parmi trois sites au lieu de deux.

---

## Acceptance criteria

Transcrits verbatim du corps de mika#2879 :

- **AC1** : une réponse d'état accompli que le terme (C) contredit lève la
  récidive de classe `fabrication-detected`. Une réponse d'engagement sur
  l'avenir ne la lève pas. Les deux sont testées.
- **AC2** : l'ensemble des réponses qui font tirer le détecteur est inchangé
  (test de non-régression sur les fixtures existants).
- **AC3** : le fixture de collision 5d + `assert_grounded` émet une seule P0 par
  tour. Retirer le verrou de classe le fait rougir.
- **AC4** : rouge-avant, terme par terme, sur le commit vert.
- **AC5** : la porte reste désarmée par défaut. Son armement relève de mika#2871.

### Répartition sur les vols — ce que le VOL COURANT ferme

| AC | ce vol | vol 3 | vol 4 |
|---|---|---|---|
| AC1 | **la moitié détecteur** : les deux natures sont distinguées, nommées et testées *une par une* sur les douze alternatives. La levée de la récidive de classe n'est pas atteignable sans le site, qui est du vol 3. | la moitié classe (levée / non-levée au transport) | — |
| AC2 | **entier**, et par construction (D1) | plancher reconduit | plancher reconduit |
| AC3 | — | — | **entier** |
| AC4 | **entier, sur le périmètre de ce vol** (§ *Risques*) | sur son périmètre | sur son périmètre |
| AC5 | **entier, et trivialement** : aucune ligne de ce vol ne lit ni ne change la porte (D4) | reconduit, avec son contrôle à porte fermée | reconduit |

### Critères dérivés, concrets et testables, du périmètre de ce vol

- **AC-a** — Les douze alternatives de `FREQUENCY_ASSERTION_ALTERNATION` sont
  réparties entre exactement deux constantes — **quatre** d'état accompli,
  **huit** d'engagement — et **chacune dans une seule**.
  Vérifiable : la table de classification donne, pour un texte représentatif de
  chaque alternative, la nature attendue — une alternative dupliquée dans les deux
  moitiés rend `AccomplishedState` là où la table attend `FutureCommitment` (la
  première branche de l'union gagne), et le test rougit.
- **AC-b** — La détection est **inchangée** : les dix tests de 5e de
  `guards.rs` et `tests/eval/test_unactioned_frequency_promise_2358.rs` restent
  verts **sans modification**. Un seul de ces tests touché infirme AC2.
- **AC-c** — La nature est lue sur la branche appariée par le moteur
  (`caps.name("accompli")`), et non re-calculée par un troisième prédicat.
  Vérifiable par lecture : aucune regex ancrée de classification n'existe dans le
  diff.
- **AC-d** — Les deux `warn!` de 5e portent `assertion_nature`, et sa valeur est
  celle du `FrequencyPromiseMatch` du tour. Vérifiable : la sonde de D2 rend deux
  valeurs distinctes sur un corpus mélangé.
- **AC-e** — Le scan d'union est livré avec une allowlist **vide**, pinnée vide,
  et avec son contrôle de bonne foi qui atteste son rouge (D3).
- **AC-f** — Diff vide sur `PendingFabricationP0Site`, `still_violated`, le
  créneau `pending_fabrication_p0`, le site d'acceptation, `config.rs` et
  `tests/eval/` (D4). Vérifiable : `git diff origin/main --stat` ne nomme que
  `evidence/guards.rs`, `agent_loop/mod.rs` et `docs/plans/`.

---

## Definition of Done

- [ ] `FREQUENCY_ASSERTION_ACCOMPLISHED_STATE` et
      `FREQUENCY_ASSERTION_FUTURE_COMMITMENT` livrées, avec le critère
      d'appartenance à la classe et la justification du classement des deux formes
      au présent dans leurs doc-comments.
- [ ] L'union est reconstruite **à partir d'elles** en un site unique, et les deux
      regex de détection l'interpolent.
- [ ] `FrequencyAssertionNature` livré ; `FrequencyPromiseMatch` porte `nature` ;
      la valeur vient de la branche appariée par le moteur.
- [ ] Les dix tests de 5e de `guards.rs` et le fichier eval de mika#2358 sont
      verts, **non modifiés**.
- [ ] La table de classification couvre les douze alternatives, par nature
      attendue.
- [ ] Les deux `warn!` de 5e portent `assertion_nature`.
- [ ] Le scan d'union et son contrôle de bonne foi sont livrés ; l'allowlist est
      vide et pinnée vide.
- [ ] Diff vide sur les six surfaces de D4 — en particulier **aucun** dépôt dans
      `pending_fabrication_p0` et **aucune** variante ajoutée à
      `PendingFabricationP0Site`.
- [ ] Recettes de rouge-avant jouées, résultats écrits dans l'en-tête de doctrine
      du module de test.
- [ ] `cargo fmt`, `cargo clippy`, `cargo test -p mika-agent`, les lints du dépôt
      et `scripts/verify-pipeline.sh origin/main` verts.
- [ ] Aucune porte armée : `MIKA_FABRICATION_P0_NOTIFY` à son défaut,
      `SETTABLE_CONFIG_KEYS` inchangé.
- [ ] `Code review: delegated to mika-qa (mika#2694)` dans le corps de la PR, et
      `Tracked in: senara-solutions/mika#2879` pour les vols 3 et 4.

---

## Risques

### R1 — Les recettes de rouge-avant, terme par terme (AC4)

Un test qui n'a jamais été rouge n'atteste rien. Chaque recette est une injection
jouée sur l'arbre, le test lancé, l'injection retirée par `git checkout`. Ce qui
est écrit dans l'en-tête du module est **ce qui a été observé**, pas ce qui était
prévu — et une recette restée verte y figure avec sa raison.

| # | injection | ce qui doit rougir |
|---|---|---|
| i | une alternative accomplie (`c'est corrigé`) déplacée dans la moitié engagement | la table de classification, sur cette alternative |
| ii | une alternative accomplie **dupliquée** dans les deux moitiés | la table : la première branche de l'union gagne, donc l'alternative engagement dupliquée rend `AccomplishedState` |
| iii | une alternative **supprimée** d'une moitié sans être replacée | les dix tests de 5e — c'est le contrôle d'AC2, et il prouve que l'union est bien l'unique source de la détection |
| iv | `caps.name("accompli").is_some()` → `caps.name("engagement").is_some()` | toute la table : les deux natures s'inversent en bloc |
| v | `assertion_nature` retiré d'un des deux `warn!` | — **à construire ou à déclarer vert.** Voir R3 |
| vi | un second usage de `FREQUENCY_ASSERTION_ACCOMPLISHED_STATE` ajouté hors du site d'union | le scan de D3 — c'est son contrôle de bonne foi, et il est livré comme test, pas seulement joué |

La recette (iii) est celle qui rend AC2 attesté plutôt que plausible : elle
démontre que retirer une alternative d'une moitié la retire **de la détection**,
donc que la détection n'a pas d'autre source que l'union.

### R2 — La scission change la détection en silence

Le risque n'est pas que les dix tests de 5e rougissent, c'est qu'ils restent
verts alors que l'ensemble des réponses appariées a bougé — par exemple si une
alternative est recopiée avec une classe de caractères légèrement différente
(`r[ée]gl[ée]` → `réglé`), ce qu'aucun des dix fixtures ne traverse.

**Mitigation**, et elle est mécanique plutôt que vigilante : les douze
alternatives sont **déplacées**, jamais retapées. L'étape 1 de l'ordre de commit
isole la scission dans son propre commit, sans le discriminant, et la vérifie sur
le corpus existant avant que quoi que ce soit s'y ajoute. Un `git diff` de ce
commit où une ligne de regex apparaît modifiée plutôt que déplacée est le signal
à relire.

### R3 — Le champ de `warn!` est le seul livrable que ce vol n'attesterait pas

`assertion_nature` sur les deux `warn!` est le consommateur de production du
discriminant (D2), et rien dans le périmètre de ce vol ne le vérifie : les tests
de `guards.rs` ne voient pas `agent_loop`, et ce vol ne livre aucun test
`tests/eval/`.

Deux issues, et **l'implémentation choisit celle qui tient dans la marge** :

1. **Un `tracing-test` sur la branche 5e** — un test moteur qui capture le
   `warn!` et assert sur le champ. Coût réel : il faut atteindre la branche, donc
   le harnais, donc une séquence de récidive — ≈ 60 lignes, et la marge est de
   71. Faisable, mais il mange la marge entière.
2. **Déclarer la recette (v) comme restant verte, avec sa raison**, exactement
   comme le vol 1 a déclaré son verrou de classe livré non attesté : le champ est
   une ligne de journal dont la régression est visible à la relecture et dont
   l'absence est de toute façon le signal d'un binaire ancien (motif mika#2340),
   et le vol 3 traverse cette branche avec de vrais tests moteur qui le
   couvriront.

**La préférence est (1) si et seulement si le compteur de lignes reste sous la
borne au moment de l'écrire ; sinon (2), écrite dans l'en-tête du module.** Ce qui
n'est **pas** une option : la livrer sans dire laquelle des deux a eu lieu.

### R4 — Le découpage en vols est une borne, pas une suggestion

Trois dérives sont à portée de main pendant ce vol, et toutes les trois sont hors
périmètre : ajouter « tant qu'on y est » la variante de `PendingFabricationP0Site`
(c'est le vol 3, et elle ne compile pas sans son bras de `still_violated`, qui ne
se justifie pas sans son dépôt, qui ne s'atteste pas sans ses trois tests —
l'enchaînement complet du vol 3), armer la porte (mika#2871), et rattacher le
site 6 à `recovery-runbook` (mika#2866).

**Mitigation** : AC-f est vérifiable par `git diff origin/main --stat`, qui ne
doit nommer que trois chemins. Un `--stat` qui en nomme un quatrième est un vol
qui a glissé.
