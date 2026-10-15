# mika#2699 — le manager tient le bassin `ready` entre 2 et 4, en ombre puis derrière un interrupteur

## Contexte

Deuxième ticket de la Phase 2 du manager (ordre b → a → c), après mika#2698 pour
(b). Aujourd'hui l'orchestrateur tient le bassin `ready` à la main : il garde
entre 2 et 4 tickets prêts, les p1 avant les p2, et ne laisse jamais le rail
vide. Le ticket demande que le manager le *dise* à chaque cycle, puis — derrière
un interrupteur OFF par défaut — qu'il le *fasse*.

Deux contraintes de substrat encadrent tout ce qui suit.

**Le manager est LECTURE SEULE, et c'est structurel.** La charte vit dans le
docstring de `crates/mika-agent/src/milestone_manager/mod.rs` et est tenue par un
scan de jetons dans `no_dispatch_test.rs` (22 jetons interdits, dont
`gh issue edit`, `"POST"`, `"PATCH"`, `run_claude_pilot`). Toute écriture de la
Phase 2 est une **exception nommée** à ce scan, jamais un relâchement du scan.
Son docstring l'impose : on modifie le test ET le docstring du module dans le
même commit, ou on ne les modifie pas.

**Le bassin `ready` n'est pas une propriété du milestone cible.** Le webhook
`ready` dispatche à l'échelle du dépôt, pas du milestone. Un bassin compté sur
les seules `sub_issues` du milestone serait sous-compté, et la recommandation
fausse exactement le jour où elle compte. Mesuré le 2026-10-15 :
2 issues `ready` ouvertes (mika#2699, mika#2687), **les deux** portant
`dispatch:loop` — profondeur disponible 0, bande *famine*. Un bassin lu sur le
seul milestone aurait affiché « 2, nominal ».

## Ce que le code offre déjà — relevé, parce qu'il change le périmètre

- **`SubIssue` porte déjà tout ce qu'il faut pour trier les candidats** :
  `number`, `state`, `priority_rank: Option<u8>`, `plan_present: bool`,
  `labels: Vec<String>`. Le vol 1 n'a donc **aucun** parse de corps d'issue à
  écrire pour juger « plan groomé présent » : `plan_callout_present` le calcule
  déjà dans `reader.rs`, et son docstring mika#2608 explique pourquoi la forme
  lâche est délibérément conservée.
- **`priority_from_labels`** (`reader.rs`) donne l'ordre p0 → p1 → p2 sous forme
  numérique (`p0-critical` → 0 … `p3-nice-to-have` → 4, sinon `None`). Le tri du
  vol 1 le réutilise tel quel : **aucune seconde table de priorités**.
- **`DISPATCH_SEAT_LABEL_PREFIX` (`"dispatch:"`) existe déjà**, `pub(crate)` dans
  `webhook_dispatch.rs`, avec `KNOWN_DISPATCH_SEATS` lintée des deux côtés contre
  `.github/labels.yml` (`scripts/check-dispatch-seats-declared.sh`). « Un vol est
  en cours » se lit donc sur la forge, sans base de données, sans nouvel appel
  réseau, et **sans dépendre de mika#2698** (dont les PR #2704 et #2706 sont
  encore OUVERTES au moment où ce plan est écrit — en dépendre rendrait ce vol
  non-mergeable seul). `_release_issue_seat` (dispatch-lib) retire
  `dispatch:loop` en fin de vol et à nouveau dans son trap EXIT : le signal
  retombe tout seul.
- **`ready` n'est jamais retiré par la boucle.** `_release_issue_seat` ne touche
  que `dispatch:<seat>`. L'interdit « jamais de retrait de `ready` pendant un
  vol » du point 3 est donc déjà vrai du côté boucle ; le vol 3 doit seulement ne
  pas l'introduire.
- **Les sections conditionnelles du rapport ont un précédent** :
  `### Dispatch contention` et `### Auth boundary` ne s'impriment que lorsqu'il y
  a quelque chose à dire. Le bassin suit cette forme.
- **`#[serde(default)]` sur un champ d'`Assessment` a un précédent canonique** :
  `contention_events`, avec ses deux tests (un blob antérieur au champ se
  désérialise encore ; vide sérialise `[]` et non `null`). Le puits hors-ligne
  contient de tels blobs : sans `default`, le manager cesse de relire son propre
  historique.
- **`state_digest` ne hache que `sub_issues`** (`cadence.rs`) : numéro, état,
  état de PR, état CI, bloqueurs. Un champ de bassin posé à côté est donc
  **par construction** hors de la détection de changement. C'est le comportement
  voulu au vol 1 — le bassin est rendu, il ne déclenche pas de cycle — mais il
  est aujourd'hui vrai par accident de rédaction. Le vol 1 l'épingle par un test
  nommé, sinon le prochain qui enrichit le digest le change sans le savoir.
- **Pas de répertoire `tests/` imbriqué sous `milestone_manager/`** : interdit par
  `MECHANICAL_CONTAINS`, qui accorde MECHANICAL (auto-mergeable) à tout chemin
  contenant `/tests/`. Les tests du nouveau module vivent **en ligne**.

## Découpage en vols

La borne par vol de ce ticket (431 lignes estimées) mord avant le seuil par PR.
Un vol unique couvrant les trois points du ticket a été estimé à ≈ 570 lignes —
au-dessus. Le découpage suit les trois points de l'attendu, ce qui donne trois
vols dont chacun est mergeable seul et observable seul.

| vol | contenu | AC couverts |
|---|---|---|
| **1 (ce plan)** | Le manager voit le bassin et dit qui devrait y entrer. Profondeur (`ready` ouverts, vols en cours), bandes 2–4, hystérésis, tri par priorité puis numéro, filtre « plan groomé présent ». Mode ombre : le verdict entre dans le rapport et nulle part ailleurs. | point 1 entier ; AC2 pour le tri et les bornes 2 à 4 |
| **2** | Le refus de l'admission mal formée, dit nominativement. Structure `AdmissionRefusal` à deux motifs (`NoGroomedPlan`, `EstimateOverFlightBound`), lecture du plan au ref de branche (`gh api /repos/…/contents/<chemin>?ref=<branche>`), parse de `Total estimé :`, détection d'un découpage explicite. Candidats hors milestone cible, après ceux du milestone. | point 2 ; solde de AC2 |
| **3** | `MIKA_MANAGER_POOL_WRITE` (OFF par défaut) et le geste groupé unique : poser `ready` + rattacher au milestone, idempotent. Exception nommée dans `no_dispatch_test.rs` au jeton `gh issue edit`, avec son contrôle négatif, et amendement atomique du docstring de `mod.rs`. | point 3 ; AC1 et AC3 |

Tracked in: les vols 2 et 3 sont suivis par mika#2699 lui-même — ce ticket reste
ouvert après le vol 1 et son corps porte l'état d'avancement par vol. Si
l'opérateur préfère un ticket par vol, les deux périmètres ci-dessus sont déjà
rédigés pour être découpés tels quels.

**Pourquoi le vol 1 s'arrête là.** Le refus par estimation (vol 2) exige un appel
`gh` **par candidat** pour aller lire le plan au ref de sa branche — le fichier de
plan vit sur la branche du ticket, pas sur `main`. C'est une classe de coût
différente de tout ce que le reader fait aujourd'hui (trois appels à volume fixe),
et elle mérite son propre vol, son propre plafond de candidats et sa propre
garde. L'écriture (vol 3) exige l'exception structurelle, qui ne se livre pas
dans le même commit qu'un module neuf sans rendre la revue illisible.

## Conception — vol 1

### `crates/mika-agent/src/milestone_manager/pool.rs` (nouveau)

Déclaré `pub mod pool;` dans `mod.rs`, types re-exportés comme les autres,
docstring du module amendé pour nommer le bassin comme lecture. **Tests en ligne**
(`#[cfg(test)] mod tests`), jamais de répertoire `tests/`.

```rust
/// Un ticket `ready` ouvert quelque part dans le dépôt, vu depuis la forge.
pub struct ReadyTicket {
    pub number: u64,
    pub labels: Vec<String>,
}

/// Profondeur du bassin à l'instant du cycle.
pub struct PoolDepth {
    pub ready_total: usize,  // tickets portant `ready`
    pub in_flight: usize,    // ... dont un siège `dispatch:<seat>` est posé
    pub available: usize,    // ready_total - in_flight
}

pub enum PoolBand { Starved, Nominal, Saturated }

pub struct PoolVerdict {
    pub depth: PoolDepth,
    pub band: PoolBand,
    /// Numéros à faire entrer, dans l'ordre d'admission. Vide hors famine.
    pub admit: Vec<u64>,
    /// Combien de candidats du milestone ont été écartés faute de plan groomé.
    pub withheld_no_plan: usize,
}

pub const POOL_MIN: usize = 2;
pub const POOL_MAX: usize = 4;

pub fn assess_pool(pool: &[ReadyTicket], candidates: &[SubIssue]) -> PoolVerdict;
```

Quatre décisions, chacune pour une raison nommable :

1. **`available = ready_total − in_flight`, et c'est `available` qui décide la
   bande.** Un ticket `ready` déjà en vol n'est pas du rail disponible. Le
   prédicat « en vol » est `labels.iter().any(|l| l.starts_with(DISPATCH_SEAT_LABEL_PREFIX))`,
   en réutilisant la constante de `webhook_dispatch.rs` — pas une seconde
   épellation de `"dispatch:"`.
2. **Hystérésis : on ne recommande une admission que sous `POOL_MIN`, et on
   recommande alors de remonter à `POOL_MAX`.** Sans elle, à 3 disponibles, le
   manager recommanderait un ticket à chaque cycle jusqu'à 4, puis à nouveau dès
   qu'un vol démarre : un geste par cycle, soit exactement le bruit que
   l'orchestrateur évite à la main. Avec elle, la recommandation est muette entre
   2 et 4 et ne parle qu'en famine.
3. **Le tri est `(priority_rank.unwrap_or(u8::MAX), number)`** — la même clé que
   les quatre paliers de `pick_next_action` dans `assessor.rs`. Un ticket sans
   label de priorité passe en dernier, jamais en premier : l'absence de priorité
   n'est pas p0.
4. **Candidats = les `sub_issues` ouvertes du milestone cible**, filtrées : pas
   déjà `ready`, pas en vol, `plan_present == true`. Le « avant le reste » du
   ticket (candidats hors milestone) est le vol 2, qui a de toute façon besoin du
   même élargissement de lecture pour le refus par estimation. Ce que le vol 1
   livre n'est donc pas faux : c'est la moitié haute de l'ordre demandé, et le
   rapport le dit.

Le compte `withheld_no_plan` existe pour une seule raison : un filtre muet est
indistinguable d'un bassin vide. Le vol 1 ne nomme pas *quels* tickets sont
écartés (c'est le point 2, donc le vol 2) mais il dit **combien**, pour que
l'opérateur sache qu'il y a eu un filtre.

### `reader.rs` — un quatrième appel `gh`

```
["issue", "list", "--repo", R, "--state", "open", "--label", "ready",
 "--json", "number,labels", "--limit", "100"]
```

Volume fixe, même forme que les trois appels existants, même discipline INV-2
(`GhRunner`, jamais de `std::process`). `--label ready` borne la réponse au
bassin lui-même : 2 lignes aujourd'hui, pas les 60 issues ouvertes du dépôt. Un
composeur pur `compose_ready_pool(json) -> Vec<ReadyTicket>` fait la conversion,
testable sans réseau comme les autres composeurs du module.

Pas de garde de troncature ici, et c'est délibéré : la page ne peut se remplir
que si le bassin atteint 100 tickets `ready`, ce qui est 25 fois la borne haute —
un état où la recommandation « ne rien admettre » est correcte quelle que soit la
troncature. (Le vol 2, qui élargira la lecture à tout le dépôt, en aura besoin et
reprendra le motif de `pr-origin-report.sh` : refuser plutôt que mal répondre.)

### `types.rs` — deux champs

- `MilestoneState::ready_pool: Vec<ReadyTicket>` — la lecture brute.
- `Assessment::pool: Option<PoolVerdict>`, `#[serde(default)]`.

`Option` et non `Vec` : `None` signifie « le bassin n'a pas été lu » (binaire
antérieur, lecture en échec), `Some(verdict)` avec `admit` vide signifie « lu, et
rien à admettre ». Un null n'est jamais un 0 (motif mika#2331) — et le puits
hors-ligne contient des blobs écrits avant ce champ, qui doivent continuer à se
relire.

### `assessor.rs` — un appel

`Assessor::assess` appelle `pool::assess_pool(&state.ready_pool, &state.sub_issues)`
et pose le résultat sur `Assessment::pool`. Rien d'autre : `assess_pool` est une
fonction pure, l'assessor reste un moteur de règles sans E/S.

### `reporter.rs` — une section conditionnelle

Rendue seulement quand `assessment.pool` est `Some` :

```
### Bassin ready (mode ombre — aucune écriture)

- Profondeur : 2 ready, 2 en vol, 0 disponible — famine (cible 2–4)
- À admettre : #2711, #2708 (p1, p2)
- Écartés : 3 candidats sans plan groomé
```

La mention « mode ombre » est **dans le titre**, pas en note de bas de section :
l'opérateur qui compare la recommandation à son propre geste doit savoir d'un
coup d'œil que le manager n'a rien fait (forme retenue par mika#2698).

### `cadence.rs` — rien, et un test qui le dit

`state_digest` ne hache que `sub_issues` ; `ready_pool` en est donc exclu sans
une ligne de code. Un test nommé épingle la propriété : deux `MilestoneState`
identiques sauf le bassin produisent le même digest. Sans lui, le prochain qui
enrichit le digest fait du bassin un déclencheur de cycle et personne ne le voit.

## Ce que le vol 1 ne livre pas, et pourquoi c'est dit ici

- **Aucune écriture, d'aucune sorte.** `no_dispatch_test.rs` n'est pas touché et
  reste vert sur le nouveau module : c'est la démonstration, pas une promesse.
- **Aucun interrupteur.** `MIKA_MANAGER_POOL_WRITE` n'est **pas** déclaré au vol 1.
  Un interrupteur dont la branche ON ne fait rien est un mensonge de surface ; il
  arrive au vol 3 avec son geste.
- **Aucun refus nommé par ticket.** Le vol 1 compte les candidats écartés faute de
  plan ; il ne dit pas lesquels ni ne distingue le second motif. Vol 2.
- **Aucun candidat hors milestone cible.** Vol 2.
- **`## Fire-Disposition` : N/A pour ce vol.** Le vol 1 ne livre aucun détecteur —
  aucun livrable dont la fonction primaire est de signaler une violation, aucun
  scan structurel, aucune garde CI. Les tests livrés sont des tests de
  comportement sur une fonction pure, pas des détecteurs au sens mika#1574. La
  section n'est donc pas inventée (mika#2306 : « si le plan ne livre AUCUN
  détecteur, ne l'invente pas »). **Le vol 3 en livrera un** — l'amendement de
  `no_dispatch_test.rs` — et son plan portera la section en **option (a)**,
  exception nommée au jeton `gh issue edit`, entrée grep-visible citant le site
  exact, assertion auto-nettoyante qui rougit quand l'exception devient stale, et
  contrôle négatif prouvant que le scan amendé rougit encore sur une source
  fautive.

Tracked in: senara-solutions/mika#2699 (vols 2 et 3, périmètres détaillés dans la
section *Découpage en vols* ci-dessus).

## Contrat de vérification

Tests en ligne, tous sans réseau et sans base de données.

**`pool.rs`**

1. Bandes aux cinq points : 0, 1 → famine ; 2, 3, 4 → nominal ; 5 → saturé.
   C'est le test littéral de « entre 2 et 4 ».
2. `available` retranche bien les vols : trois `ready` dont deux portent
   `dispatch:loop` → `available = 1`, bande famine. Le cas mesuré du
   2026-10-15 (2 ready, 2 en vol, famine) est le fixture.
3. Un label `dispatch:` d'un siège inconnu compte quand même comme vol en cours —
   le prédicat est sur la famille, pas sur `KNOWN_DISPATCH_SEATS` ; un siège neuf
   ne doit pas rendre le bassin optimiste le jour de son introduction.
4. Hystérésis : à `available = 3`, `admit` est vide. À `available = 1`, `admit`
   contient de quoi atteindre 4, soit 3 numéros au plus.
5. Tri : p0 avant p1 avant p2 ; à priorité égale, numéro croissant ; un candidat
   sans label de priorité passe après tous les autres.
6. Filtre : un candidat `plan_present == false` n'apparaît jamais dans `admit` et
   incrémente `withheld_no_plan`.
7. Un candidat déjà `ready`, ou déjà en vol, ou fermé, n'est pas recommandé.
8. Bassin vide et aucun candidat : `admit` vide, bande famine, pas de panique.

**`reader.rs`**

9. La forme de l'appel : `--label ready`, `--state open`, le champ `labels`
   présent — via un `GhRunner` de test qui capture les arguments.
10. `compose_ready_pool` sur une sortie `gh` réelle (fixture) et sur `[]`.

**`types.rs`**

11. Un blob d'`Assessment` **antérieur au champ** se désérialise (`pool == None`).
12. Un `Assessment` sans bassin sérialise `"pool":null` et jamais un objet vide ;
    un `Assessment` avec bassin fait l'aller-retour.

**`cadence.rs`**

13. `state_digest` est insensible à `ready_pool` : deux états identiques sauf le
    bassin donnent le même digest.

**`assessor.rs`**

14. `assess` pose `Some` quand le bassin est lu, et le verdict porte la
    profondeur attendue.

**`reporter.rs`**

15. Section absente quand `pool` est `None`.
16. Section présente, titre portant « mode ombre », quand `pool` est `Some`.
17. Le rendu de famine nomme la profondeur, la cible 2–4 et les numéros admis.

Plus, inchangés et devant rester verts : `no_dispatch_test.rs` (le nouveau module
est scanné et ne porte aucun jeton interdit),
`milestone_manager_has_no_nested_tests_directory`, et la Porte 1 forge-gate
(`pool.rs` doit classer `DecisionCore`, comme tout fichier sous
`milestone_manager/**`).

Commande : `cargo test -p mika-agent milestone_manager`, puis `cargo clippy`,
`cargo fmt`, et `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- `pool.rs` existe, est déclaré et re-exporté dans `mod.rs`, dont le docstring
  nomme le bassin comme une **lecture** de la Phase 2 (a) vol 1.
- Le reader fait son quatrième appel `gh` via `GhRunner` et compose `ready_pool`.
- `Assessment::pool` est `#[serde(default)]` et le puits hors-ligne relit ses
  anciens blobs.
- Le rapport porte la section conditionnelle, « mode ombre » dans le titre.
- Les 17 tests ci-dessus passent ; `no_dispatch_test.rs` est **inchangé** et vert.
- `cargo clippy` sans warning, `cargo fmt` propre, `verify-pipeline.sh` vert.
- Le corps du ticket mika#2699 dit ce qui reste aux vols 2 et 3.

## Acceptance criteria

- [ ] **AC1** : avec l'interrupteur OFF, aucune écriture (garantie structurelle
      avec contrôle négatif). — *Vol 3 pour sa forme nominale (l'interrupteur
      n'existe pas avant lui). Le vol 1 en livre le cas dégénéré et le plus fort :
      il n'y a pas d'interrupteur du tout, aucun chemin d'écriture n'est écrit, et
      `no_dispatch_test.rs` — inchangé — le vérifie sur le nouveau module.*
- [ ] **AC2** : tests du tri, des bornes 2 à 4, et du refus des mal formés. —
      *Vol 1 pour le tri (test 5) et les bornes 2 à 4 (tests 1 et 4), et pour le
      refus des tickets sans plan groomé en tant que filtre (test 6). Vol 2 pour
      le refus dit nominativement et pour le second motif (estimation au-dessus de
      la borne par vol sans découpage).*
- [ ] **AC3** : avec l'interrupteur ON, une seule mutation par décision,
      idempotente. — *Vol 3.*
- [ ] **Taille** : à estimer tests compris, et à découper en vols au-delà de la
      borne. — *Fait : section ci-dessous, trois vols, le vol courant sous la
      borne.*

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/pool.rs` (types, `assess_pool`, prédicat « en vol », bandes, tri, hystérésis) | 105 |
| tests en ligne de `pool.rs` (cas 1 à 8) | 80 |
| `crates/mika-agent/src/milestone_manager/reader.rs` (4e appel `gh`, `compose_ready_pool`) | 35 |
| tests en ligne de `reader.rs` (cas 9 et 10) | 30 |
| `crates/mika-agent/src/milestone_manager/types.rs` (`MilestoneState::ready_pool`, `Assessment::pool`) | 18 |
| tests en ligne de `types.rs` (cas 11 et 12) | 27 |
| `crates/mika-agent/src/milestone_manager/assessor.rs` (appel de `assess_pool`) | 15 |
| tests en ligne de `assessor.rs` (cas 14) | 22 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (section conditionnelle) | 35 |
| tests en ligne de `reporter.rs` (cas 15 à 17) | 25 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` (test du digest, cas 13) | 12 |
| `crates/mika-agent/src/milestone_manager/mod.rs` (déclaration, re-exports, docstring) | 18 |

Total estimé : 422 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

Le vol courant passe sous la borne avec 9 lignes de marge, ce qui est serré et se
dit. Le poste à sortir en premier si la mesure dérive est le test 3 (siège
inconnu) puis le test 8 (bassin vide), qui couvrent des cas dégénérés et non le
cœur des AC ; le tri, les bornes et le filtre ne sortent pas.

Mesure de référence pour calibrer la lecture de ce total : le vol 1 de mika#2698
avait estimé 970 lignes et produit 2 183 insertions hors `docs/`, soit un facteur
d'optimisme de ≈ 2,25. La borne par vol divise déjà par 2 ; 422 lignes estimées
correspondent donc à ≈ 850 lignes réelles, pour un plafond de 200 tours dont la
fin de pipeline en consomme 50.
