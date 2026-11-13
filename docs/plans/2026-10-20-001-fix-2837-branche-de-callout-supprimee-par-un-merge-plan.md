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
seul endroit qui peut constater que la forge a bien honoré le filtre.

**Retenue : l'entrée de `mergedAt` maximal, comparé par `crate::timestamp::parse`,
jamais comme deux chaînes** (F6). La rev 1 disait « comparaison lexicographique,
ISO 8601 fixe » — vrai pour la forme Zulu que les deux charges capturées portent
(`2026-10-20T06:10:45Z`), faux dès qu'un décalage apparaît :
`"2026-10-20T08:10:45+02:00" < "2026-10-20T06:10:45Z"` est vrai en
lexicographique et faux dans le temps. Le dépôt n'a pas à parier sur le format de
la forge, parce qu'il possède déjà le parseur qui absorbe les deux :

```rust
// crates/mika-agent/src/timestamp.rs:18-27 — DB_FORMAT ("…Z") puis RFC 3339 en
// repli, donc un décalage horaire est normalisé plutôt que mal ordonné.
pub fn parse(s: &str) -> Result<DateTime<Utc>>
```

L'argmax porte donc sur `timestamp::parse(&e.merged_at).ok()`, et une entrée dont
`mergedAt` ne se parse pas est **malformée au sens de l'asymétrie ci-dessous** :
elle ne prouve rien, les autres continuent. Trois lignes, et la classe de bug
disparaît au lieu d'être documentée comme acceptable.

Pour mémoire, et c'est ce qui rend ce choix peu coûteux plutôt qu'anxieux : la
comparaison lexicographique de ces mêmes champs est déjà une convention tenue et
testée ici — `scripts/pr-origin-report.sh:160-161` filtre sur
`since <= merged < until`, avec ses dix fixtures Zulu dans
`scripts/test-pr-origin-report.sh`. La rev 1 n'inventait donc rien ; elle
reposait simplement sur une propriété du producteur qu'un appel à notre propre
parseur rend inutile à supposer.

**Asymétrie des deux niveaux d'échec** — JSON global illisible ⇒ `Err` ; entrée
malformée ⇒ cette entrée ne prouve rien, les autres continuent.

**Et elle est citée, pas recopiée** (F3). Le doc-comment du nouveau parseur
renvoie à `parse_open_pr_issue_numbers` (l. 2672-2681), qui est l'endroit où
cette asymétrie est **argumentée** (*« le fail-open global est trop grossier pour
servir de traitement d'erreur par entrée »*) — lui-même renvoyant à
`parse_compare_payload`, l'aîné des deux. Un lecteur suit une chaîne de renvois
vers une seule rédaction de la règle.

Ce qui n'est **pas** factorisé, et pourquoi la factorisation serait la mauvaise
réponse ici : la part commune aux deux parseurs est la ligne
`serde_json::from_str::<Vec<Value>>(stdout)?` et le `continue` par entrée. Tout
le reste diverge — champs lus (`closingIssuesReferences`/`headRefName` contre
`mergedAt`/`baseRefName`/`headRefName`), type de retour (`OpenPrLinkage`, un
agrégat à cinq compteurs, contre `Option<MergedPrEvidence>`), et surtout mode
d'accumulation (union d'ensemble contre argmax). Une abstraction partagée devrait
être générique sur l'extracteur d'entrée **et** sur l'accumulateur : c'est
`iter().filter_map(…).max_by_key(…)`, que la bibliothèque standard fournit déjà.
L'envelopper produirait un adaptateur plus épais que ses deux sites d'appel.
DRY porte sur la connaissance, pas sur les frappes : la connaissance dupliquée
était la *règle* d'asymétrie, et c'est elle que le renvoi dé-duplique. Si la
convention change, le site à éditer reste un.

### D3 — la résolution, pure, et le refus sur absence de preuve

```rust
/// Dit, en trois armes **totales et explicites**, ce que l'absence d'une branche
/// vaut.
///
/// La sonde est prise **par valeur** (mika#2837 rev 2) : l'appelant n'a plus
/// rien à en faire après cet appel, et le `&Result` de la rev 1 n'existait que
/// pour éviter un déplacement dont personne n'avait besoin — il forçait un
/// binding temporaire au site d'appel sans rien acheter.
///
/// Deux des trois armes convergent sur le même verdict — le refus — sous deux
/// noms distincts. C'est le **propos** de cette fonction, pas une économie
/// d'écriture : un opérateur doit pouvoir compter `Unreadable` sans
/// `NotCarried`. `Unreadable` en masse, c'est un jeton expiré qui restaure
/// silencieusement le comportement d'avant ce correctif pour tout le monde (la
/// classe mika#2013), et c'est un diagnostic que `NotCarried` ne porte pas.
///
/// **Ne pas « harmoniser » l'arme `Err` avec ses voisines.** Elle refuse là où
/// le reste de la porte promeut (*« the three "could not measure" outcomes all
/// promote »*, l. 1014-1018), et c'est délibéré : voir ci-dessous.
///
/// **Point d'extension, si un quatrième cas d'absence émerge** (timeout avec
/// retry contre échec permanent) : une variante d'[`AbsenceProbe`], pas une
/// signature neuve. `Result<Option<_>>` n'a que trois formes et le `match` les
/// couvre toutes ; la granularité du diagnostic vit dans `AbsenceProbe`, qui est
/// précisément l'axe conçu pour grossir.
pub fn classify_absent_branch(probe: Result<Option<MergedPrEvidence>>)
    -> StalenessMeasurement
{
    match probe {
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
c'est elle appliquée dans le sens où elle mord.

**Pourquoi cette fonction existe, plutôt que trois armes inlinées dans
`promotion_gate_allows`** (F2, seconde branche de la préconisation). Parce que
cette alternative rend l'AC3 **intestable**, et le plan le prouve plus bas :
`promotion_gate_allows` est une `async fn` qui lance un sous-processus, donc
atteindre ses armes depuis un test impose de passer par le double de
`gh_process` — dont la clé `("repo", 0)` est globale et dont le doc interdit
explicitement l'override (§ *Piège à ne pas prendre*). Le cas `Err` de l'AC3 ne
serait alors joignable par aucun test. La coupure D2/D3 n'est donc pas une
préférence de style : c'est la condition pour que le chemin fail-closed soit
vérifiable sans réseau, exactement la raison que `parse_compare_payload` donne
pour la sienne (*« Split from the subprocess call so the whole decision path is
testable against frozen payloads with no network »*, l. 976-980). Et la logique
n'est pas « cachée » dans un auxiliaire : les trois armes sont côte à côte dans
un seul `match` de huit lignes, ce qui est plus lisible que leur dispersion dans
le corps de la fonction d'I/O.

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
        classify_absent_branch(gh_merged_pr_for_branch(github_token, b).await),
    (m, _) => m,
};
```

Passage par valeur (rev 2, F2) : plus de `&` ni de binding temporaire au site
d'appel.

Coût : **un** appel `gh` de plus, et seulement sur le chemin d'absence — quatre
occurrences en quatre jours d'après les preuves du ticket. Négligeable.

### D7 — signaler, et où s'arrête ce vol

Le ticket demande « elle signale **ou** réécrit le callout » et met la réécriture
hors périmètre. Signalement retenu : une ligne INFO
`auto_pull_branch_deleted_by_merge` (ticket, branche, `pr`, `merged_at`) plus les
deux champs d'audit de D5, et une WARN `auto_pull_merged_pr_probe_unreadable`
quand la sonde échoue.

**Les deux lignes sont émises dans `gh_merged_pr_for_branch` (D6), pas dans
`classify_absent_branch`** (rev 2, conséquence de F2). C'est là que vit
`merged_at` — la chaîne verbatim de la forge — et là que l'erreur de lecture
porte encore son contexte `anyhow`. Journaliser depuis la fonction de
classification aurait exigé d'elle un retour composite
(`(StalenessMeasurement, Option<MergedPrEvidence>)`) pour faire remonter une
valeur dont seul le journal a besoin : la fonction pure garde un retour simple,
et le numéro de PR continue de voyager par la variante `BranchDeletedByMerge`.

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
| 4 | `mika2837_les_trois_conditions_sont_necessaires` — `baseRefName != "main"` ⇒ `None` ; `headRefName` autre ⇒ `None` ; entrée malformée ⇒ ignorée sans faire échouer le lot ; `mergedAt` non parsable ⇒ même traitement que malformée (F6) | AC1 |
| 5 | `mika2837_un_json_illisible_est_une_erreur_jamais_une_absence` — `Err`, puis `classify_absent_branch(Err(…)) → BranchAbsent{Unreadable}` (par valeur, rev 2) | **AC3** |
| 6 | `mika2837_laudit_par_mesure_table_complete` — **table sur les six mesures**, détaillée ci-dessous | AC1 |

**Test 6, cas déclencheurs nommés** (F4). La rev 1 disait « par cas » sans dire
lesquels, ce qui laissait le pilote libre de ne vérifier `null` que sur une seule
mesure — et l'invariant qui compte est précisément qu'aucune autre mesure ne
laisse fuiter les deux champs neufs. La table est donc **totale sur
`StalenessMeasurement`** : ses **six lignes** couvrent les **cinq variantes** de
l'énumération, `BranchAbsent` comptant pour deux puisque ses deux sondes doivent
produire le même `measurement` sous deux `merged_pr_probe` distincts.

Les quatre valeurs *(inchangé)* sont relues dans `measurement_slug`
(`auto_pull.rs:1201-1206`), pas supposées : ce test les réasserte telles
qu'elles sont, il ne les redéfinit pas.

| mesure en entrée | `measurement` | `merged_pr` | `merged_pr_probe` |
|---|---|---|---|
| `Measured(BranchStaleness{..})` — branche présente | `"measured"` *(inchangé)* | `null` | `null` |
| `NoBranchCallout` | `"no_branch_callout"` *(inchangé)* | `null` | `null` |
| `Unavailable` | `"unavailable"` *(inchangé)* | `null` | `null` |
| `BranchAbsent { probe: NotCarried }` | `"branch_absent"` *(inchangé)* | `null` | `"not_carried"` |
| `BranchAbsent { probe: Unreadable }` | `"branch_absent"` *(inchangé)* | `null` | `"unreadable"` |
| `BranchDeletedByMerge { pr: 2834 }` | `"branch_deleted_by_merge"` | `2834` | `"carried"` |

Noter `"unavailable"` et non `"staleness_unavailable"` : ce dernier est le
`detail` d'une **promotion** (l. 5390-5395), pas un slug de mesure. Les deux
vocabulaires voisinent dans le même JSON sous `measurement` et `outcome`, et les
confondre est l'erreur que cette table rend impossible à commettre en silence.

Trois lignes
`null`/`null` plutôt qu'une seule parce que la question de F4 — « un refacto
futur pourrait-il faire fuiter `merged_pr_*` hors du chemin d'absence ? » — n'a
de réponse que si la table est totale. Une variante absente de la table est
exactement la fuite non couverte. Et la totalité est tenue par le compilateur :
le corps du test `match` la mesure de façon exhaustive, donc une **sixième**
variante ajoutée demain ne compile pas sans sa ligne.

**Le message de refus est réasserté mot pour mot** dans le test 1 — l'AC2 dit
« inchangé », et c'est la seule façon de le tenir. Voici le message **complet
actuel**, relu dans `RefusalReason::reason` (`auto_pull.rs:883-887`) et reproduit
ici pour que la conformité AC2 soit vérifiable en revue QA sans ouvrir le fichier
(F5) :

```rust
Self::BranchAbsent { branch } => format!(
    "La branche `{branch}` désignée par le callout de #{issue_number} \
     n'existe pas sur `origin`. Le plan est annoncé comme commité dessus ; \
     un pilote dispatché ici repartirait de `main` sans son plan."
),
```

L'assertion du test reste celle de l'actuel l. 5406 —
`r.reason(42).contains("n'existe pas sur `origin`")` — et non une égalité sur la
chaîne entière : `format!` interpole `branch` et `issue_number`, donc une égalité
exigerait de recopier le gabarit dans le test, ce qui en ferait un second
exemplaire du message à maintenir. Le `contains` ancre le fragment stable ; la
citation ci-dessus ancre le reste pour le lecteur humain, qui est le destinataire
réel de l'AC2.

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

La rev 1 s'arrêtait à « N/A » ; F1 a raison de ne pas s'en contenter, parce
qu'une N/A est une affirmation qui doit se vérifier contre la définition du
détecteur, et dire ensuite ce qui se passe si quelque chose tire quand même.
Les deux, ci-dessous.

### Les six tests ne sont pas des détecteurs — et c'est la définition qui le dit

Le critère de mika#1574 n'est pas la **forme** du livrable (« test, assertion,
lint, invariant, validation »), c'est ce que ce livrable **fait aux données
existantes**. La doctrine ouvre sur lui :

> *« the plan must specify what happens when the detector fires on **existing**
> data — pre-existing violations that the new detector now surfaces »*
> — `docs/solutions/best-practices/fire-disposition-doctrine.md:14`

Et son incident fondateur le confirme : `verify-bundled-skills` **scanne l'arbre**
et a trouvé une collision `gh_read` préexistante (ibid. l. 18). C'est cette
classe — le livrable qui parcourt une population qu'il n'a pas écrite — que les
trois options arbitrent.

Les six tests de ce plan sont l'autre classe : chacun prend une **entrée figée
dans le test lui-même** (deux `const` de charge réelle, des `StalenessMeasurement`
construits à la main) et assert un verdict nommé. Aucun ne parcourt
`docs/plans/`, ni l'arbre, ni la base, ni la forge. Leur chemin de succès est
« le verdict est celui de la table », jamais « aucune violation trouvée ».
Il n'existe donc **aucune population préexistante** sur laquelle ils puissent
tirer, et les trois options n'ont pas d'objet : il n'y a ni violation à
allowlister (a), ni détecteur à désarmer (b), ni arbitrage de périmètre à rendre
à l'opérateur (c).

### Les deux détecteurs que ce vol touche **existent déjà**, et leur disposition est nommée

Ce qui mérite une disposition, ce n'est pas ce que le plan livre, c'est ce que le
plan **fait tirer** chez les gardes en place. Ils sont deux, les deux inspectés :

1. **`auto_pull_replay_the_promote_refuse_partition_is_unchanged`**
   (`tests/auto_pull_promotion_gate.rs:353`) et ses sept fixtures `compare`.
   Celui-là est bien un détecteur sur données existantes, et ce vol le laisse
   intact par construction : il n'ajoute que des variantes et ne retire aucune
   arme. **Disposition : (c) halt-and-surface.** S'il rougit, l'implémentation
   s'arrête et remonte à l'opérateur — elle ne retouche ni la fixture ni
   l'assertion. Motif : un verdict déplacé sur une fixture `compare` contredit
   frontalement l'AC2 (« le refus actuel, inchangé ») et l'hypothèse centrale du
   plan (la frontière promote/refuse ne bouge que sur le chemin d'absence).
   Ajuster le détecteur pour le faire taire reviendrait à livrer le contraire de
   l'AC2 en la déclarant tenue. La forme de la résolution — faut-il renoncer à la
   variante, ou l'AC2 était-elle mal posée ? — est exactement la question de
   périmètre que l'option (c) réserve à l'opérateur.

2. **`Canonical Token Lint`** (`scripts/check-canonical-tokens.sh`,
   `canonical_tokens::tests::mika2201_every_match_site_is_declared`), le piège de
   `docs/solutions/best-practices/une-constante-de-refus-neuve-entre-dans-le-recensement-des-jetons-2026-10-14.md`,
   dont le `applies_when` nomme littéralement *« Un plan annonce une
   Fire-Disposition "allowlist vide" pour un nom neuf »*. Ce plan introduit cinq
   jetons neufs (`branch_deleted_by_merged_pr`, `branch_deleted_by_merge`,
   `carried`, `not_carried`, `unreadable`). **Vérifié plutôt que supposé :** le
   recensement énumère les constantes en `*_PREFIX` portant un jeton machine
   (ibid. § Problème), et `grep -n auto_pull scripts/canonical-tokens.tsv` rend
   trois occurrences dont **une seule entrée de données** (l. 202 —
   `> - **Branch:**` / `BRANCH_CALLOUT_RE`, le motif du callout) ; les deux
   autres sont des lignes de commentaire (l. 211 et 219) et ne déclarent rien.
   Ni `branch_absent_on_origin`, ni `measurement: "branch_absent"`, ni
   aucun slug de ce module n'y figure : ce sont des littéraux d'arme de `match`,
   pas des constantes préfixes. Les cinq jetons neufs sont de la même nature que
   leurs aînés non recensés, donc **ne rentrent dans aucun des deux
   recensements** et aucune déclaration TSV n'est due.
   **Disposition si le lint tire malgré cette lecture : (a) named allowlist
   exception — non, déclaration.** Le geste est d'ajouter la ligne au TSV dans le
   **même commit** que la constante, à côté de son aînée, jamais d'allowlister ni
   de renommer pour sortir de la population (ibid. § Solution). Le contrôle
   préalable est nommé dans le contrat de vérification : `bash
   scripts/check-canonical-tokens.sh` avant le push, qui rend en quelques
   secondes ce que le job requis dirait.

Aucun autre garde du dépôt ne lit ce module : `scripts/check-byte-slices.sh`
(pas de slicing d'octets ici), `dispatch-seats-lint`, `suite-wiring-lint`
(aucune suite neuve sous `scripts/` ni `skills/bundled/_shared/`),
`pilot-turn-ceiling-labels-lint` (aucun label neuf) et `landing-tokens-lint`
(aucun fichier sous `site/`) sont tous hors d'atteinte de ce diff.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `auto_pull.rs` — `AbsenceProbe` + slug (D1) | 25 |
| `auto_pull.rs` — `MergedPrEvidence` (D2) | 18 |
| `auto_pull.rs` — `parse_merged_pr_for_branch` (D2, +3 en rev 2 : argmax par `timestamp::parse`) | 58 |
| `auto_pull.rs` — `classify_absent_branch` (D3) | 30 |
| `auto_pull.rs` — variantes de `StalenessMeasurement` + 3 sites de construction (D1) | 25 |
| `auto_pull.rs` — armes de `classify_promotion` (D4) | 14 |
| `auto_pull.rs` — `staleness_audit_json` : armes de slug + 2 champs (D5) | 18 |
| `auto_pull.rs` — `gh_merged_pr_for_branch` + sondes (D6, D7) | 45 |
| `auto_pull.rs` — câblage `promotion_gate_allows` (D6) | 25 |
| tests in-module (6 tests + 3 `const` de charge réelle ; +15 en rev 2 : table totale du test 6) | 165 |

Total estimé : 423 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient encore.** 423 < 429 et 423 < 1000 : les
deux bornes passent. Un seul fichier de production est touché, aucune migration,
aucun fichier nouveau, aucun `docs/` ; les sept sites d'appel de
`staleness_audit_json` sont épargnés par le choix D5 de ne pas ajouter de
paramètre, ce qui est la décision qui tient l'estimation.

**Marge : 6 lignes, et elle est annoncée comme mince plutôt que confortable.**
La rev 2 a consommé 18 des 24 lignes de marge de la rev 1 (F6 : +3 ; F4 : +15).
Six lignes sur une estimation de 423 sont du bruit, pas une réserve — le dire
plutôt que le maquiller.

**Levier, donc pré-autorisé et non plus contingent.** Si l'implémentation dépasse
429, replier la table du test 6 dans celle du test 1 : les deux prennent la même
entrée (`StalenessMeasurement`) et appellent la même paire
`classify_promotion` / `staleness_audit_json`, donc la fusion économise un
échafaudage de fonction de test entier (~15 lignes) sans retirer une seule ligne
de table. L'AC4 (« un test par cas ») reste tenue : le test 1 est **déjà** une
table par conception (*« table nommée, pas comptée »*), et une ligne nommée y est
un cas. Ce qui reste interdit dans tous les cas : retirer un cas d'AC, ou
réduire la table du test 6 à un sous-ensemble des variantes — c'est sa totalité
qui répond à F4.

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
bash scripts/check-canonical-tokens.sh     # § Fire-Disposition, détecteur 2 — avant le push
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
- `clippy -D warnings` et `fmt --check` propres ; `check-canonical-tokens.sh` et
  `verify-pipeline.sh` verts.
- `RefusalReason::BranchAbsent`, son slug `branch_absent_on_origin` et son message
  sont littéralement inchangés — le message étant celui cité verbatim au § Tests,
  relu dans `auto_pull.rs:883-887`.
- `measurement: "branch_absent"` conserve son nom dans l'audit ; les deux champs
  neufs sont `null` sur les **cinq** autres mesures, vérifié par la table totale
  du test 6 et non par échantillon.
- L'ordre de `mergedAt` passe par `crate::timestamp::parse`, et aucune
  comparaison de deux `mergedAt` comme chaînes ne subsiste dans le diff.
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

---

## Revision history

- **rev 1** (2026-10-20) — plan initial.
- **rev 2** (2026-10-20) — six findings de première passe architecte, les six
  traités, aucun laissé sans réponse :
  - **F1** (BLOCKING, Fire-Disposition) — section réécrite en deux temps.
    Primo, la N/A est **ancrée dans la définition** plutôt qu'affirmée : le
    critère de mika#1574 porte sur « fires on **existing** data »
    (`fire-disposition-doctrine.md:14`) et sur son incident fondateur, un
    scanneur d'arbre (ibid. l. 18) ; les six tests prennent une entrée figée
    dans le test lui-même et ne parcourent aucune population préexistante, donc
    les trois options n'ont pas d'objet. Secundo — et c'est le fond de F1 — la
    section nomme désormais une disposition pour les **deux détecteurs
    existants que ce vol fait tirer** : (c) halt-and-surface pour
    `auto_pull_replay_the_promote_refuse_partition_is_unchanged`, et
    déclaration-dans-le-même-commit pour `Canonical Token Lint`, après
    vérification que les cinq jetons neufs n'y entrent pas (`grep -n auto_pull
    scripts/canonical-tokens.tsv` rend la seule ligne `BRANCH_CALLOUT_RE`). Les
    cinq autres gardes du dépôt sont nommés hors d'atteinte du diff.
  - **F2** (BLOCKING, API de `classify_absent_branch`) — signature passée **par
    valeur** (`Result<Option<MergedPrEvidence>>`), le `&` de la rev 1 n'achetait
    rien et imposait un binding temporaire ; câblage D6 mis à jour. La fusion
    `Err`/`Ok(None)` est **conservée et argumentée** dans le doc-comment (un
    opérateur doit pouvoir compter `Unreadable` seul — classe mika#2013), avec
    une interdiction explicite de l'harmoniser et le point d'extension nommé
    (variante d'`AbsenceProbe`, pas signature neuve), ce qui répond à l'objection
    « un troisième cas d'absence demain ». La seconde branche de la
    préconisation — inliner trois armes dans `promotion_gate_allows` — est
    **refusée avec son motif** : elle rendrait l'AC3 intestable, le cas `Err`
    n'étant alors joignable que par le double de `gh_process`, dont la clé
    globale `("repo", 0)` est interdite d'override (`gh_process.rs:220-225`,
    § *Piège à ne pas prendre*). Citation préservée : `parse_compare_payload`
    l. 976-980.
  - **F3** (BLOCKING, duplication du parseur) — justification écrite dans D2
    plutôt que factorisation : la part commune est une ligne de désérialisation
    et un `continue`, tandis que champs lus, type de retour (`OpenPrLinkage` à
    cinq compteurs contre `Option<MergedPrEvidence>`) et mode d'accumulation
    (union d'ensemble contre argmax) divergent — l'abstraction partagée serait
    `filter_map().max_by_key()`, déjà dans la bibliothèque standard, et son
    adaptateur serait plus épais que ses deux sites d'appel. La connaissance
    réellement dupliquée était la **règle** d'asymétrie : le doc-comment du
    nouveau parseur la **cite par renvoi** à `parse_open_pr_issue_numbers`
    (l. 2672-2681), où elle est argumentée, de sorte qu'un changement de
    convention garde un seul site à éditer. Citation DRY préservée.
  - **F4** (sharpening, test 6) — cas déclencheurs nommés : le test devient
    `mika2837_laudit_par_mesure_table_complete`, une table **totale sur
    `StalenessMeasurement`** — six lignes pour cinq variantes (`Measured`,
    `NoBranchCallout`, `Unavailable`, `BranchAbsent` sous ses deux sondes,
    `BranchDeletedByMerge`) — avec un `match` exhaustif qui fait échouer la
    compilation si une sixième variante apparaît sans sa ligne. La totalité est la réponse à l'objection de fuite :
    une variante absente de la table *est* la fuite non couverte. Slugs relus
    dans `measurement_slug` (`auto_pull.rs:1201-1206`), ce qui a corrigé une
    erreur de la rédaction — `"unavailable"`, et non `"staleness_unavailable"`
    qui est un `detail` de promotion.
  - **F5** (sharpening, message AC2) — message de refus **complet** cité
    verbatim au § Tests depuis `auto_pull.rs:883-887`, avec la raison pour
    laquelle l'assertion reste un `contains` sur le fragment stable (le `format!`
    interpole `branch` et `issue_number` ; une égalité recopierait le gabarit et
    créerait un second exemplaire à maintenir). Référence ajoutée dans la DoD.
  - **F6** (investigative, ISO 8601) — la préoccupation est **fondée** et traitée
    par le code plutôt que par une note : l'argmax passe de la comparaison de
    chaînes à `crate::timestamp::parse` (`timestamp.rs:18-27`), qui absorbe
    `…Z` **et** RFC 3339 avec décalage, donc un `+02:00` est normalisé au lieu
    d'être mal ordonné. Une entrée dont `mergedAt` ne se parse pas rejoint la
    classe « malformée » de l'asymétrie de D2 (test 4 étendu). Le précédent
    `scripts/pr-origin-report.sh:160-161` est cité pour situer la convention
    lexicographique du dépôt, mais ce plan ne s'y repose plus. Coût : +3 lignes.
  - **Effet sur l'estimation** — 405 → 423 lignes (F6 +3, F4 +15), marge sur la
    borne de 429 ramenée de 24 à **6 lignes**, annoncée comme mince ; le levier
    de repli (fusionner la table du test 6 dans celle du test 1, ~15 lignes) est
    pré-autorisé, avec l'interdit maintenu de retirer un cas d'AC ou de réduire
    la table à un sous-ensemble.
- **rev 3** (2026-10-20) — trois corrections de rédaction, aucune décision
  d'architecture modifiée, estimation inchangée à 423 lignes :
  - **Dénombrement variantes/cas.** La rev 2 écrivait « six variantes » là où
    `StalenessMeasurement` en porte cinq après ce vol, la table du test 6 ayant
    six *lignes* parce que `BranchAbsent` y figure sous ses deux sondes. Le § Tests
    et l'entrée F4 disent désormais « six lignes pour cinq variantes », et le
    garde-fou du compilateur porte sur l'ajout d'une **sixième** variante, non
    d'une septième. La DoD, qui disait déjà « les cinq autres mesures », était
    juste et reste inchangée.
  - **Résultat du `grep` sur le recensement de jetons.** Le § Fire-Disposition
    annonçait « une seule ligne » ; `grep -n auto_pull scripts/canonical-tokens.tsv`
    rend trois occurrences, dont une seule entrée de données (l. 202) et deux
    lignes de commentaire (l. 211, 219). La conclusion — aucun slug de ce module
    n'est recensé, donc aucune déclaration TSV n'est due — est inchangée ; c'est
    le nombre annoncé qui est aligné sur ce qu'un relecteur verra en rejouant la
    commande.
  - **Coquille** « réassserte »/« réassserté » → « réasserte »/« réasserté »
    (deux occurrences).
