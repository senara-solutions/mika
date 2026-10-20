# mika#2837 — une branche de callout supprimée par le merge de sa PR n'est pas un plan perdu

**Ticket :** senara-solutions/mika#2837 (`bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`)
**Branche :** `fix/2837/auto-pull-une-branche-de-callout-supprim`
**Périmètre :** `crates/mika-agent/src/auto_pull.rs` seul (production + tests in-module)

---

## Le constat, mesuré

`gh_compare_branch` (`auto_pull.rs:2906`) rend `StalenessMeasurement::BranchAbsent`
sur un `404`, et `classify_promotion` (l. 1039-1042) refuse immédiatement. La
porte ne pose jamais la question suivante : **pourquoi** la branche est-elle
absente.

Les deux charges réelles, capturées le 2026-10-20 avec
`gh pr list --repo senara-solutions/mika --head <branche> --state all --json number,state,mergedAt,baseRefName,headRefName`,
tranchent le ticket et dictent le prédicat :

```
# fix/2108/p1-dispatch-lib-le-bac-sable-ne-monte
[{"baseRefName":"main","mergedAt":"2026-10-20T06:10:45Z","number":2834,"state":"MERGED"},
 {"baseRefName":"main","mergedAt":"2026-10-19T06:15:38Z","number":2791,"state":"MERGED"}]

# feat/2680/pr-filtre-webhook-phase-4-tours-vide-sur
[{"baseRefName":"main","mergedAt":null,              "number":2833,"state":"CLOSED"},
 {"baseRefName":"main","mergedAt":"2026-10-19T16:27:54Z","number":2829,"state":"MERGED"},
 … quatre autres MERGED …]
```

Trois faits qu'aucune réécriture de l'énoncé ne remplace :

1. **La preuve existe après la suppression de la branche.** Le `404` de `compare`
   et la réponse de `pr list --head` coexistent : l'enregistrement de la PR
   survit à la branche.
2. **La première entrée de la charge de #2680 est une PR FERMÉE SANS MERGE**
   (#2833, `mergedAt: null`). Un parseur qui lirait `[0]`, ou qui se contenterait
   de « une PR existe », refuserait #2680 — exactement le mauvais verdict. Le
   prédicat porte donc sur `mergedAt`, entrée par entrée, et le contrôle négatif
   de l'AC4 est **une sous-partie d'une charge réelle**, pas une fabrication.
3. **Une même branche porte plusieurs merges** (#2791 puis #2834 sur #2108 : le
   grooming et l'implémentation du même vol). La preuve retenue est donc la plus
   récente par `mergedAt`, jamais l'ordre de la liste.

Et l'argument qui justifie de promouvoir, en une phrase : **la preuve que la
branche a été mergée dans `main` EST la preuve que le plan qu'elle portait est
sur `main`.** C'est pour cette raison que `baseRefName == "main"` est vérifié et
non supposé — c'est le seul maillon qui relie « une PR a été mergée » à « le plan
est atteignable ».

**Contrôle fait, et il lève le dernier doute sur la sûreté de la promotion :**
`dispatch-lib.sh` n'utilise le callout `> - **Branch:**` qu'en **écriture**
(l. 10096 et 10122, `_write_canonical_callout`). Aucun site ne le lit pour un
checkout — le pilote dérive sa branche par `scripts/derive-branch-name`. Promouvoir
un ticket dont le callout nomme une branche disparue ne fait donc repartir aucun
dispatch sur une référence morte.

---

## Architecture retenue

La doctrine du module est déjà écrite et elle décide de la forme :

- `StalenessMeasurement` porte *« Four outcomes, not two, because "I could not
  measure" and "I measured zero" are different facts and collapsing them is how a
  gate starts lying »* (l. 722-727). « Absente parce que mergée » est un
  **cinquième fait mesuré**, pas une nuance de `BranchAbsent`.
- `parse_compare_payload` porte *« Split from the subprocess call so the whole
  decision path is testable against frozen payloads with no network »* (l. 976-980).
  Le nouveau parsing suit la même coupure.

### D1 — un cinquième résultat de mesure, et un résidu nommé dans le quatrième

```rust
pub enum StalenessMeasurement {
    Measured(BranchStaleness),
    /// Absente d'`origin`, et la sonde « une PR mergée la porte-t-elle ? » n'a
    /// pas promu — soit elle a répondu « aucune », soit elle n'a pas pu répondre.
    BranchAbsent { probe: AbsenceProbe },
    /// Absente parce que le merge de sa PR l'a supprimée (mika#2837).
    BranchDeletedByMerge { pr: u64 },
    NoBranchCallout,
    Unavailable,
}

/// Pourquoi une branche absente n'a pas été promue. Les deux prennent la MÊME
/// décision — le refus — sous deux noms distincts, parce qu'un opérateur doit
/// pouvoir compter l'un sans l'autre : `Unreadable` en masse, c'est un jeton
/// expiré qui restaure silencieusement le comportement d'avant ce correctif
/// pour tout le monde (la classe mika#2013).
pub enum AbsenceProbe { NotCarried, Unreadable }
```

`BranchAbsent` devient une variante de structure : trois sites de construction à
ajuster (`gh_compare_branch` l. 2927, le test l. 5403, l'arme de
`classify_promotion`). `RefusalReason::BranchAbsent { branch }` et son slug
`branch_absent_on_origin` sont **inchangés** — c'est l'AC2.

### D2 — le parseur, pur, sur la charge figée

```rust
/// Une PR mergée dont la tête est la branche du callout.
pub struct MergedPrEvidence { pub number: u64, pub merged_at: String }

/// `Err` = charge illisible (la forge n'a pas répondu) ; `Ok(None)` = elle a
/// répondu « aucune PR mergée ne porte cette branche ».
pub fn parse_merged_pr_for_branch(stdout: &str, branch: &str)
    -> Result<Option<MergedPrEvidence>>
```

Trois conditions par entrée, toutes les trois nécessaires :
`mergedAt` non-nul **et** `headRefName == branch` **et** `baseRefName == "main"`.
La deuxième n'est pas redondante avec le `--head` du serveur : le parseur est le
seul endroit qui peut constater que la forge a bien honoré le filtre. Retenue :
l'entrée de `mergedAt` maximal (comparaison lexicographique — ISO 8601 fixe, la
convention `crate::timestamp` du dépôt).

**Asymétrie des deux niveaux d'échec**, reprise telle quelle de
`parse_open_pr_issue_numbers` (l. 2672-2681) : JSON global illisible ⇒ `Err` ;
entrée malformée ⇒ cette entrée ne prouve rien, les autres continuent.

### D3 — la résolution, pure, et le refus sur absence de preuve

```rust
/// Dit, en trois armes, ce que l'absence d'une branche vaut. L'argument est un
/// `&Result` **exprès** : la seule chose que cette fonction a à dire, c'est que
/// `Err` et `Ok(None)` prennent la même décision sous deux noms différents.
pub fn classify_absent_branch(evidence: &Result<Option<MergedPrEvidence>>)
    -> StalenessMeasurement
{
    match evidence {
        Ok(Some(e)) => StalenessMeasurement::BranchDeletedByMerge { pr: e.number },
        Ok(None)    => StalenessMeasurement::BranchAbsent { probe: AbsenceProbe::NotCarried },
        Err(_)      => StalenessMeasurement::BranchAbsent { probe: AbsenceProbe::Unreadable },
    }
}
```

C'est l'AC3, et c'est l'**inverse** du fail-open général de la porte
(*« the three "could not measure" outcomes all promote »*, l. 1014-1018) : ici le
cas de base est déjà le refus, et la sonde ne peut que le **lever**. Ne pas
promouvoir sur une absence de preuve n'est donc pas une exception à la doctrine,
c'est elle appliquée dans le sens où elle mord. Le doc-comment doit le dire, sinon
un futur lecteur « harmonisera » l'arme `Err` avec ses voisines.

### D4 — l'arme de promotion

```rust
StalenessMeasurement::BranchDeletedByMerge { .. } =>
    return PromotionGate::Promote { detail: "branch_deleted_by_merged_pr" },
StalenessMeasurement::BranchAbsent { .. } =>
    return PromotionGate::Refuse(RefusalReason::BranchAbsent { … }),  // inchangé
```

`detail` est `&'static str` : le numéro de PR ne passe pas par là, il passe par
l'audit (D5). AC1 est satisfaite par la distinction du `detail`.

### D5 — le journal, et ce qui reste au nom qu'il avait

`staleness_audit_json` : **aucun paramètre nouveau** — tout se dérive de la
mesure, donc les sept sites d'appel de test restent intacts.

| champ | valeur |
|---|---|
| `measurement` | `"branch_absent"` **inchangé** pour `BranchAbsent{..}` ; `"branch_deleted_by_merge"` pour la nouvelle variante |
| `merged_pr` | le numéro, ou `null` |
| `merged_pr_probe` | `"carried"` \| `"not_carried"` \| `"unreadable"` \| `null` hors chemin d'absence |

`measurement: "branch_absent"` garde son nom parce que `RefusalReason::slug` porte
déjà la règle, citée mot pour mot : *« This is a wire format. (…) a split here is
dated and never retroactive (…) The half that keeps its name stays comparable to
itself »* (l. 807-816). La population qui refuse encore reste comparable à
elle-même ; la population qui promeut porte un nom neuf.

`null`, jamais `0` ni `""`, hors chemin d'absence : motif mika#2331, et c'est déjà
la discipline des trois champs voisins (`behind_by`, `non_plan_files`,
`plan_files_count`).

### D6 — l'I/O, mince

```rust
async fn gh_merged_pr_for_branch(github_token: &str, branch: &str)
    -> Result<Option<MergedPrEvidence>>
```
`run_gh(token, GhSubject::Repo, &["pr","list","--repo",DEFAULT_REPO,"--head",branch,
"--state","all","--json","number,state,mergedAt,baseRefName,headRefName","--limit","30"])`.

`--state all` et non `--state merged` : le discriminant doit vivre **dans notre
code**, là où un test l'atteint. Avec un filtre serveur, le contrôle négatif de
l'AC4 n'inspecterait qu'une liste vide — il testerait la fixture, pas la règle.

Câblage dans `promotion_gate_allows` (l. 2965-2969), après la mesure :

```rust
let measurement = match (measurement, branch.as_deref()) {
    (StalenessMeasurement::BranchAbsent { .. }, Some(b)) =>
        classify_absent_branch(&gh_merged_pr_for_branch(github_token, b).await),
    (m, _) => m,
};
```

Coût : **un** appel `gh` de plus, et seulement sur le chemin d'absence — quatre
occurrences en quatre jours d'après les preuves du ticket. Négligeable.

### D7 — signaler, et où s'arrête ce vol

Le ticket demande « elle signale **ou** réécrit le callout » et met la réécriture
hors périmètre. Signalement retenu : une ligne INFO
`auto_pull_branch_deleted_by_merge` (ticket, branche, `pr`, `merged_at`) plus les
deux champs d'audit de D5, et une WARN `auto_pull_merged_pr_probe_unreadable`
quand la sonde échoue.

**Pas de commentaire sur la forge, et c'est une décision argumentée.** Le
commentaire de refus existe parce qu'un refus exige un geste humain ; cette
promotion n'en exige aucun. La porte repasse à chaque pull tant que le callout est
périmé, donc un commentaire par promotion serait du bruit récurrent sans remède
actionnable autre que l'édition du callout — que le ticket place hors périmètre.
« Journalisée », c'est le mot de l'AC1, et l'audit est le canal qu'un opérateur
`GROUP BY`.

**Résidu, nommé et non masqué :** le callout continue de nommer une branche
disparue, donc la sonde est repayée à chaque pull et un lecteur humain du ticket
lit toujours une branche morte. La réécriture en « Plan antérieur » est le suivi à
ouvrir (`Tracked in:` à renseigner par l'opérateur) ; elle demande une écriture de
corps d'issue, surface que `auto_pull` ne possède aujourd'hui sur aucun chemin
(il n'écrit que des labels et des commentaires) et qui entrerait en course avec
`_write_canonical_callout`.

---

## Tests — un par cas, plus le contrôle négatif (AC4)

**Tout en in-module, et c'est le précédent du fichier qui le décide.**
`OPEN_PR_LIST_FIXTURE` (l. 10626) est déjà une charge `gh pr list` **réelle et
figée** en `const` dans `mod tests`. `tests/fixtures/auto_pull_compare/` ne
contient que des charges `compare` et son `PROVENANCE.md` ne parle que d'elles :
y déposer des charges `pr list` mélangerait deux populations. Les deux charges
capturées ci-dessus deviennent donc deux `const` provenancées (date + commande).

| # | test | AC |
|---|---|---|
| 1 | `mika2837_la_partition_des_trois_sondes` — table nommée, pas comptée : `Carried → Promote{"branch_deleted_by_merged_pr"}`, `NotCarried → Refuse(branch_absent_on_origin)`, `Unreadable → Refuse(branch_absent_on_origin)`. Non-vacuité : les deux côtés non vides | AC1, AC2, AC3 |
| 2 | `mika2837_le_parseur_sur_les_deux_charges_reelles` — #2108 ⇒ `Some(2834)` (le plus récent des deux merges), #2680 ⇒ `Some(2829)` **alors que l'entrée [0] est la PR fermée** | AC1 |
| 3 | `mika2837_une_pr_fermee_sans_merge_ne_porte_pas_la_branche` — l'entrée réelle #2833 seule ⇒ `Ok(None)` ⇒ refus | **AC4** |
| 4 | `mika2837_les_trois_conditions_sont_necessaires` — `baseRefName != "main"` ⇒ `None` ; `headRefName` autre ⇒ `None` ; entrée malformée ⇒ ignorée sans faire échouer le lot | AC1 |
| 5 | `mika2837_un_json_illisible_est_une_erreur_jamais_une_absence` — `Err`, puis `classify_absent_branch(&Err) → BranchAbsent{Unreadable}` | **AC3** |
| 6 | `mika2837_laudit_distingue_les_trois_sondes` — `merged_pr` / `merged_pr_probe` par cas, `measurement == "branch_absent"` préservé sur le refus, et les deux champs `null` hors chemin d'absence | AC1 |

**Le message de refus est réassserté mot pour mot** dans le test 1
(`r.reason(42).contains("n'existe pas sur `origin`")`, comme l'actuel l. 5406) :
l'AC2 dit « inchangé », et c'est la seule façon de le tenir.

**Contrôle structurel gratuit :** `auto_pull_replay_the_promote_refuse_partition_is_unchanged`
(`tests/auto_pull_promotion_gate.rs:353`) et ses sept fixtures `compare` ne sont
**pas touchés**. Leur verdict inchangé est la preuve que ce vol n'a déplacé aucune
autre frontière.

**Piège à ne pas prendre :** ne **pas** tester `gh_merged_pr_for_branch` via le
double de `gh_process` en posant un `CANNED_STDOUT` sur `GhSubject::Repo`. La clé
`("repo", 0)` est globale et le doc du double l'interdit explicitement
(*« la suite tourne en parallèle dans un seul processus, donc un override global
serait lu par les tests voisins »*, `gh_process.rs:220-225`) : `gh_compare_branch`
et `gh_list_open_pr_issue_numbers` s'adressent au même sujet. Toute la décision est
atteignable par les deux fonctions pures — c'est la raison de la coupure D2/D3.

---

## Fire-Disposition

**N/A, et vérifié plutôt que supposé.** Ce plan ne livre aucun détecteur au sens
de mika#2306 : pas de règle de lint, pas de garde CI, pas de scan structurel, pas
de validateur de schéma, pas de garde EndTurn. Les six tests sont behavioraux
(entrée figée ⇒ verdict nommé) ; leur chemin de succès est « le verdict est celui
de la table », jamais « aucune violation trouvée ». La section n'est donc pas
inventée.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `auto_pull.rs` — `AbsenceProbe` + slug (D1) | 25 |
| `auto_pull.rs` — `MergedPrEvidence` (D2) | 18 |
| `auto_pull.rs` — `parse_merged_pr_for_branch` (D2) | 55 |
| `auto_pull.rs` — `classify_absent_branch` (D3) | 30 |
| `auto_pull.rs` — variantes de `StalenessMeasurement` + 3 sites de construction (D1) | 25 |
| `auto_pull.rs` — armes de `classify_promotion` (D4) | 14 |
| `auto_pull.rs` — `staleness_audit_json` : armes de slug + 2 champs (D5) | 18 |
| `auto_pull.rs` — `gh_merged_pr_for_branch` + sondes (D6, D7) | 45 |
| `auto_pull.rs` — câblage `promotion_gate_allows` (D6) | 25 |
| tests in-module (6 tests + 3 `const` de charge réelle) | 150 |

Total estimé : 405 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 405 < 429 et 405 < 1000 : les deux
bornes passent. Un seul fichier de production est touché, aucune migration,
aucun fichier nouveau, aucun `docs/` ; les sept sites d'appel de
`staleness_audit_json` sont épargnés par le choix D5 de ne pas ajouter de
paramètre, ce qui est la décision qui tient l'estimation. Marge : 24 lignes. Si
l'implémentation la dépasse, le levier nommé d'avance est de replier le test 6
(audit) dans la table du test 1 — pas de retirer un cas d'AC.

---

## Ordre de commit

1. D1 + D2 + D3 — les types et les deux fonctions pures, avec les tests 2 à 5.
   Compile et passe sans qu'aucun chemin de production ait changé de verdict.
2. D4 + D5 — armes de `classify_promotion` et champs d'audit, avec les tests 1 et 6.
   C'est le commit qui change le comportement.
3. D6 + D7 — l'I/O, le câblage et les sondes.

Plancher : l'étape 1 seule est mergeable (du code mort testé), mais le ticket
n'est servi qu'à l'étape 3. Les trois vont dans la même PR.

---

## Contrat de vérification

```bash
cargo test -p mika-agent auto_pull            # in-module + mika2837_*
cargo test -p mika-agent --test auto_pull_promotion_gate   # les 7 fixtures compare, inchangées
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/verify-pipeline.sh origin/main
```

Aucune variable d'environnement nouvelle, donc aucune entrée dans
`docs/operator/environment-variables.md`. Aucune entrée `CLAUDE.md` : la doctrine
de ce module vit dans ses doc-comments, et c'est là que celle-ci est écrite.

**Sonde post-déploiement**, parce qu'un correctif qu'on ne peut pas voir n'est pas
un correctif (mika#2293) :

```sql
SELECT json_extract(after_value,'$.measurement')      AS measurement,
       json_extract(after_value,'$.merged_pr_probe')  AS probe,
       json_extract(after_value,'$.outcome')          AS outcome,
       count(*)
FROM audit_events
WHERE target_key = 'auto_pull_staleness_measured'
GROUP BY 1,2,3;
```

Lecture : une ligne `branch_deleted_by_merge / carried / promote` est le correctif
qui sert. Une ligne `branch_absent / unreadable / refuse` en volume est un
problème d'accès à la forge qui restaure le comportement d'avant — c'est
précisément ce que l'AC3 rend visible plutôt que silencieux. L'**absence** du
champ `merged_pr_probe` signifie un binaire antérieur à ce correctif (classe
mika#2340), jamais « aucune branche absente ».

---

## Definition of Done

- Les six tests du tableau passent, et les sept fixtures `compare` de
  `auto_pull_promotion_gate.rs` rendent leurs verdicts inchangés.
- `clippy -D warnings` et `fmt --check` propres ; `verify-pipeline.sh` vert.
- `RefusalReason::BranchAbsent`, son slug `branch_absent_on_origin` et son message
  sont littéralement inchangés.
- `measurement: "branch_absent"` conserve son nom dans l'audit ; les deux champs
  neufs sont `null` hors chemin d'absence.
- Le corps de PR nomme le résidu de D7 (callout périmé non réécrit) avec sa ligne
  `Tracked in:`.

---

## Acceptance criteria

- **AC1.** Une branche du callout absente d'`origin`, mais portée par une PR
  mergée, ne produit ni `operator-gated` ni commentaire de refus. La décision est
  journalisée avec un détail distinct de `branch_absent_on_origin`.
- **AC2.** Une branche absente et portée par aucune PR mergée produit le refus
  actuel, inchangé.
- **AC3.** Si la forge ne permet pas de dire si une PR mergée existe (lecture en
  erreur), le comportement reste celui d'aujourd'hui (refus). On ne promeut pas
  sur une absence de preuve.
- **AC4.** Un test par cas, plus un contrôle négatif : une PR **fermée sans
  merge** sur la branche donne le refus.

## Hors périmètre

- La réécriture automatique du corps du ticket, si elle demande d'écrire sur la
  forge sous une autre identité que celle déjà utilisée par l'auto-pull. La
  signaler suffit pour ce ticket (D7).
- Les autres motifs de refus (`TooFarBehind`, `SalvageWorkOnStaleBranch`,
  `StaleBranchWithoutPlan`).
