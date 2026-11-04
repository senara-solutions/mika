# mika#3036 — en phase 0, le feeder ne se redonne pas le consentement que le merge vient de rendre

- **Ticket :** senara-solutions/mika#3036
- **Labels :** `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`
- **Dépôt :** mika (un seul)
- **Source :** parking #2983 (commentaires 6624299123 et 6626104054)

---

## 0. Le vol, en une phrase

`classify_promotion` rend `Promote` sur une mesure `BranchDeletedByMerge` **sans
regarder la phase** ; le vol fait entrer la phase dans cette décision, par un
`match` sans bras `_`, de sorte que la phase 0 — celle où le ticket ne porte
**pas** `ready` — décline, tandis que les phases 1 et 2 — celles où il le porte
déjà — promeuvent exactement comme avant.

---

## 1. L'état mesuré

### 1.1 Les deux occurrences

| ticket | PR mergée | `ready` retiré par le merge | mesure du feeder | promotion |
|---|---|---|---|---|
| #3030 | #3032 (`Refs`), 06:12:57Z | 06:13:02Z, `merged_pr_ready_cleanup_handler` | 06:20:02Z, `branch_deleted_by_merge`, `outcome: promote` | 06:20:07Z, `feeder_rank=4, reason=pullable=0<min_ready=3` → groom plan-only que personne n'a demandé |
| #2194 | #3033, 07:58:23Z | oui | 08:00:05Z, `branch_deleted_by_merge` | 08:00:14Z, `feeder_rank=0` → refusé à 08:00:16Z, **seulement** parce qu'un `live_pilot` existait |

Deux lectures importent dans ce tableau. La première : le second cas n'a été
rattrapé par rien de structurel — un pilote vivait, voilà tout. La seconde :
sur 62 mesures `branch_deleted_by_merge` dans `audit_events` depuis le
2026-10-20, seules ces deux-là sont suivies d'un `auto_feeder_promoted` sur le
même ticket dans les deux minutes. Les 60 autres sont des passages de phase 1 ou
2 sur un ticket qui portait **déjà** `ready` — c'est-à-dire la population que
mika#2837 a délibérément autorisée, et que ce vol ne touche pas.

Les deux occurrences suivent le handler de mika#3022
(`merged_pr_ready_cleanup_handler`, mergé le 2026-11-03 à 15:08Z). Avant lui,
`ready` restait posé après le merge et le feeder n'avait aucune raison de le
reposer : la régression est née de la fermeture d'un autre trou, pas d'un
changement dans `auto_pull`.

### 1.2 Le site, lu sur `7e71911aa`

`crates/mika-agent/src/auto_pull.rs` :

| élément | ligne | ce qu'il fait |
|---|---|---|
| `classify_absent_branch` | 1327 | `Ok(Some(evidence))` donne `BranchDeletedByMerge { pr }` |
| `classify_promotion` | 1353 | signature `(measurement, branch, max_behind)` — **la phase n'y entre pas** |
| ↳ le bras fautif | 1376-1380 | `BranchDeletedByMerge { .. } => Promote { detail: "branch_deleted_by_merged_pr" }` |
| `RefusalReason::slug` | 993-1000 | `BranchAbsent { .. } => "branch_absent_on_origin"` — la raison dont AC1 exige d'être distinct |
| `staleness_audit_json` | 1485-1593 | porte déjà `issue`, `merged_pr`, `merged_pr_probe`, `measurement`, `outcome`, `reason` |
| `promotion_gate_allows` | 3858 | reçoit `phase: &str`, **ne le passe pas** à `classify_promotion` (l. 3889) |
| ↳ bloc de réécriture du callout | 3929-3965 | s'exécute **inconditionnellement**, avant le `match decision` de la l. 3967 |
| `ExclusionPhase` | 1700-1714 | les trois mêmes phases, déjà typées, mais sous un **autre** format de fil |
| `FILTER_PROMOTION_GATE` | 1756 | `"promotion_gate_refused"` |
| appels | 5141, 5307, 5922 | `"phase0_feeder"`, `"phase1_idle_pull"`, `"phase2_stuck_rescue"` |

`server/merged_pr_ready_cleanup_handler.rs` (mika#3022) pose l'invariant :
« Retirer, c'est rendre un consentement, jamais s'en donner un » (voie B,
mika#2995). En phases 1 et 2 le ticket porte déjà `ready` : la règle de
mika#2837 y laisse partir le vol suivant d'un ticket à plusieurs vols, et c'est
juste. En phase 0 il ne le porte pas — le moteur se redonne alors le
consentement que le merge venait de rendre.

### 1.3 Trois faits de terrain qui dictent la forme du geste

**(a) `ExclusionPhase` existe déjà, aux trois mêmes variantes, et chaque site
d'appel la nomme déjà deux lignes plus bas.** Aux l. 5152, 5319 et 5933,
`ledger.record(ExclusionPhase::Phase0Feeder | ::Phase1Pull | ::Phase2StuckReady,
…)` est juste sous l'appel qui passe la chaîne. Il n'y a donc **aucun** enum à
créer : il y a une projection à ajouter.

**(b) Les deux projections ne sont pas interchangeables.**
`ExclusionPhase::as_str()` rend `"phase1_pull"` / `"phase2_stuck_ready"` ; la
porte écrit `"phase1_idle_pull"` / `"phase2_stuck_rescue"` dans
`auto_pull_staleness_measured`. Les deux vocabulaires sont **déjà persistés dans
`audit_events`** et un opérateur les `GROUP BY`. Réutiliser `as_str()` pour la
porte renommerait silencieusement la moitié d'une population (mika#2205). D'où
une **seconde** méthode, `gate_phase()`, qui rend les trois chaînes de la porte
telles quelles.

**(c) Un test bout-en-bout de `promotion_gate_allows` est hors d'atteinte du
sceau d'aujourd'hui.** `gh_compare_branch` (l. 3276) et
`gh_merged_pr_for_branch` (l. 3339) passent tous deux par `GhSubject::Repo` :
une seule case de canning globale, deux charges différentes nécessaires dans un
même passage, et le journal d'appels est partagé avec les tests parallèles. Ce
constat n'est pas un détail d'outillage — c'est lui qui impose, pour AC3,
d'**extraire** le bloc de réécriture dans sa propre fonction, qui ne touche que
`GhSubject::Issue(n)` et qui est donc, elle, cannable par ticket.

---

## 2. Le geste

### 2.1 `ExclusionPhase` devient la phase, tout court

Elle passe `pub` (`classify_promotion` est `pub` et le test d'intégration
l'appelle ; un paramètre de type privé dans une fonction publique est refusé par
`private_interfaces`). Elle gagne `gate_phase()` :

```rust
impl ExclusionPhase {
    /// Le format de fil du **ledger d'exclusion**.
    fn as_str(self) -> &'static str { /* inchangé */ }

    /// Le format de fil de la **porte de promotion** — celui que
    /// `auto_pull_staleness_measured.phase` porte depuis mika#2470.
    ///
    /// Deux projections, deux vocabulaires, et ils ne sont PAS
    /// interchangeables : les deux sont déjà dans `audit_events`, et un
    /// opérateur les groupe. Les faire converger renommerait une population
    /// à moitié (mika#2205).
    pub fn gate_phase(self) -> &'static str {
        match self {
            Self::Phase0Feeder => "phase0_feeder",
            Self::Phase1Pull => "phase1_idle_pull",
            Self::Phase2StuckReady => "phase2_stuck_rescue",
        }
    }
}
```

Les trois chaînes sont copiées **à l'octet** depuis les l. 5146, 5312 et 5927.
Aucune ligne d'audit ne change de forme.

### 2.2 Le déclin est un troisième état, pas un refus

`PromotionGate` gagne une variante. Pourquoi pas `Refuse` : `refuse_promotion`
(l. 4017-4113) pose `REFUSAL_LABEL` (`operator-review`), **retire `ready`** et
poste `reason.comment_body()`. Appliqué ici, il escaladerait à l'opérateur un
ticket qui n'a aucun défaut, et retirerait un `ready` qui n'est pas là. Le bon
mot n'est pas « ce ticket est cassé », c'est « ce n'est pas à moi de partir ».

```rust
/// Pourquoi la porte s'abstient — sans rien reprocher au ticket.
///
/// Distinct de [`RefusalReason`] parce que les suites diffèrent : un refus
/// escalade (label `operator-review`, `ready` retiré, commentaire), un déclin
/// n'écrit RIEN sur la forge. Le ticket est sain ; c'est le consentement qui
/// manque, et seul un humain ou une phase qui le voit déjà peut le rendre.
pub enum DeclineReason {
    /// mika#3036 — en phase 0 le ticket ne porte pas `ready`, et la branche a
    /// disparu parce qu'une PR a été mergée. Le merge vient de *rendre* le
    /// consentement (mika#3022) ; le reprendre serait s'en donner un.
    ConsentReturnedByMerge { pr: u64 },
}

impl DeclineReason {
    pub fn slug(&self) -> &'static str {
        match self {
            Self::ConsentReturnedByMerge { .. } => "consent_returned_by_merge",
        }
    }
}
```

`slug()` est **distinct de `branch_absent_on_origin`** (AC1) : les deux mesures
se ressemblent — une branche absente dans les deux cas — mais l'une est une
anomalie et l'autre la trace normale d'un merge. Les compter sous un même nom,
c'est de nouveau mika#2205.

`PromotionGate::Decline(DeclineReason)` rejoint `Promote` et `Refuse`.

### 2.3 La décision, dans un seul site, par un `match` sans `_`

`classify_promotion` prend un quatrième paramètre et le bras de la l. 1376
devient :

```rust
StalenessMeasurement::BranchDeletedByMerge { pr } => {
    // mika#3036 — le seul site où la phase décide. Pas de bras `_` : une
    // quatrième phase ne compilera pas sans qu'on dise ici ce qu'elle fait.
    return match phase {
        // Le ticket ne porte pas `ready`. Le merge l'a retiré (mika#3022) ;
        // le reposer serait se donner le consentement qu'on vient de rendre.
        ExclusionPhase::Phase0Feeder => PromotionGate::Decline(
            DeclineReason::ConsentReturnedByMerge { pr: *pr },
        ),
        // Le ticket porte déjà `ready`. mika#2837 inchangé : c'est ainsi que
        // part le vol suivant d'un ticket à plusieurs vols.
        ExclusionPhase::Phase1Pull | ExclusionPhase::Phase2StuckReady => {
            PromotionGate::Promote { detail: "branch_deleted_by_merged_pr" }
        }
    };
}
```

C'est **tout** l'AC2 : un site, un `match` exhaustif, et aucun deuxième lecteur
de la sonde de merge — rien d'autre dans le vol ne relit
`StalenessMeasurement::BranchDeletedByMerge` ni ne consulte la phase pour
décider.

### 2.4 Un déclin n'écrit pas sur la forge

`PromotionGate` gagne une projection, elle aussi exhaustive :

```rust
impl PromotionGate {
    /// Ce passage a-t-il le droit de réécrire le corps du ticket ?
    ///
    /// `Refuse` garde `true` : le comportement d'avant mika#3036, à
    /// l'identique. Seul `Decline` dit non, et il le dit parce qu'un passage
    /// qui s'abstient ne doit laisser aucune trace d'écriture (AC3).
    pub fn authorizes_body_rewrite(&self) -> bool {
        match self {
            Self::Promote { .. } | Self::Refuse(_) => true,
            Self::Decline(_) => false,
        }
    }
}
```

Le bloc des l. 3929-3965 sort de `promotion_gate_allows` dans
`maybe_rewrite_stale_callout(db, github_token, label_auth, issue, measurement,
decision, phase, trace_id, session_id)` — déplacement, pas réécriture — qui
commence par :

```rust
if !decision.authorizes_body_rewrite() {
    return;
}
```

Deux effets. Côté production : un déclin fait **zéro** appel forge et n'écrit
aucune ligne `auto_pull_callout_rewrite`. Côté mesure : la fonction extraite ne
touche que `GhSubject::Issue(n)` (`gh_issue_body` puis `gh_edit_issue_body`),
donc elle est testable au sceau `gh_process` existant — ce que
`promotion_gate_allows` entière n'est pas (§ 1.3c).

### 2.5 Le déclin se lit dans le journal

`promotion_gate_allows` prend `phase: ExclusionPhase` (les quatre usages en
`&str` deviennent `phase.gate_phase()`), et le `match decision` de la l. 3967
gagne son bras :

```rust
PromotionGate::Decline(reason) => {
    warn!(
        issue = issue.number,
        merged_pr = ?reason.merged_pr(),
        reason = reason.slug(),
        phase = phase.gate_phase(),
        "auto_pull_promotion_declined"
    );
    false
}
```

`staleness_audit_json` (l. 1543) gagne son troisième bras :

```rust
PromotionGate::Decline(r) => ("decline", r.slug()),
```

`"decline"` est une **valeur nouvelle** du vocabulaire `outcome`, pas un
renommage : les agrégats `WHERE outcome='promote'` et `='refuse'` existants
restent sommables, et la nouvelle population est interrogeable séparément. Les
champs qu'AC4 nomme sont déjà là : `issue` et `merged_pr` (rempli `Some(*pr)`
précisément sur `BranchDeletedByMerge`), plus `reason`.

Les trois sites d'appel passent la variante qu'ils nomment déjà :
`"phase0_feeder"` → `ExclusionPhase::Phase0Feeder`, etc. Le `ledger.record(…,
FILTER_PROMOTION_GATE)` sur `false` est **inchangé** — c'est lui qui satisfait
le premier membre d'AC4, et ne pas le toucher est le geste.

### 2.6 Les 41 appels existants, sans 41 éditions

`classify_promotion` est appelée ~32 fois dans `mod tests` et ~9 fois dans
`tests/auto_pull_promotion_gate.rs`. Ces tests mesurent la distance, le salvage
et l'absence de branche — des populations où la phase ne décide rien. Chaque
fichier reçoit **un** helper local du même nom, qui l'emporte sur l'import glob :

```rust
/// Ancre les tests antérieurs à mika#3036 sur la phase dont ce vol ne change
/// RIEN. C'est ce qui en fait leur contrôle de non-régression : s'ils bougent,
/// c'est que les phases 1 et 2 ont bougé. Les tests de la phase appellent
/// `super::classify_promotion` nommément, avec leur phase explicite.
fn classify_promotion(
    m: &StalenessMeasurement,
    branch: Option<&str>,
    max_behind: i64,
) -> PromotionGate {
    super::classify_promotion(m, branch, max_behind, ExclusionPhase::Phase1Pull)
}
```

Reste ce que le compilateur refusera de lui-même : les `match` exhaustifs sur
`PromotionGate` qui n'ont que deux bras (l. 92 et 398-404 de l'intégration,
quelques sites inline). Chacun gagne un bras `Decline` qui `panic!` avec la
décision reçue — un déclin inattendu là est un défaut, et doit se lire comme
tel.

---

## 3. Ce que ce vol ne fait pas

- **Le secours `no-shipping-tail` de dispatch-lib**, qui rouvre une PR déjà
  mergée (#3034, parking 6626104054 point 1). Hors vol par le ticket.
- **Une repose de `ready` par un humain après le merge.** C'est un nouveau
  consentement, et il reste servi par les phases 1 et 2 — inchangées, et dont
  AC1 porte le contrôle négatif.
- **Il ne renomme aucun format de fil.** `"phase1_pull"` et `"phase1_idle_pull"`
  restent deux chaînes différentes dans deux colonnes différentes (§ 2.1b).
- **Il ne fusionne pas `Decline` dans `Refuse`.** `refuse_promotion` garde son
  comportement exact ; `authorizes_body_rewrite()` rend `true` sur `Refuse`
  précisément pour que rien du chemin d'avant ne bouge.
- **Coût accepté, nommé ici parce qu'AC3 le crée.** Le commentaire des
  l. 3790-3793 observe que la population mesurée est « auto-tarissante » :
  après réécriture du callout, `extract_branch_name` rend `None` et la porte ne
  mesure plus rien sur ce ticket. AC3 interdit cette réécriture sur un déclin —
  donc le tarissement disparaît, et un ticket décliné est **re-mesuré à chaque
  tick** (deux appels `gh`), puis re-décliné. Ce n'est pas un livelock : rien
  n'est consommé, aucun vol ne part, aucune écriture n'a lieu, et le feeder
  passe au candidat suivant. C'est un coût de sonde récurrent, et il est
  **intentionnel** : le déclin est une décision *par passage*, pas un état, et
  le ticket attend un consentement humain. Sa taille est comptable sans rien
  ajouter — `SELECT issue, count(*) FROM audit_events WHERE … outcome='decline'
  GROUP BY issue` — et c'est cette même requête qui sert la sonde du § 7. Si la
  population grossit au-delà de quelques tickets, c'est un ticket de suivi, pas
  une raison de réécrire le corps d'un ticket qu'on a choisi de ne pas toucher.

---

## 4. Fire-Disposition

Ce vol livre **un** détecteur : le scan structurel S1, qui tient AC2 et le
premier membre d'AC4. Disposition retenue : **(a) exception nommée en
allowlist**, avec une allowlist **vide** — et c'est une mesure, prise sur
`7e71911aa`, pas un espoir.

| détecteur | violations avant le geste | après | allowlist |
|---|---|---|---|
| S1 — un seul lecteur de la phase dans la décision, et trois `ledger.record` sous `FILTER_PROMOTION_GATE` | 0 (la forme n'existe pas encore) | 0 | `T3036_PHASE_READER_ALLOWLIST: [&str; 0] = []` |

S1 étend le scan déjà en place dans ce fichier
(`test_promotion_gate_never_resolves_conflicts`, l. 6948-6975), qui construit
déjà son `ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"))` et lit
`auto_pull.rs` — il n'y a donc pas un scanner de plus à câbler. Il asserte des
**comptes**, jamais des présences :

- `match phase {` apparaît exactement **1** fois dans la production du fichier ;
- `ledger.record(` suivi de `FILTER_PROMOTION_GATE` apparaît exactement **3**
  fois — les trois phases, dont celle d'AC4 ;
- contrôle positif conservé du scan hôte : `production.contains("fn
  classify_promotion")`, sans quoi une extraction vide rendrait les comptes
  ci-dessus indistinguables d'un fichier non lu.

L'allowlist est un tableau vide suivi de son assertion auto-nettoyante, sur le
précédent maison du dépôt :

```rust
const T3036_PHASE_READER_ALLOWLIST: [&str; 0] = [];
assert!(
    T3036_PHASE_READER_ALLOWLIST.is_empty(),
    "S1: table d'exceptions du scan de lecteur de phase — doit rester vide; \
     toute entrée nomme sa donnée et son ticket de suivi (mika#3036)"
);
```

Le type `[&str; 0]` fait mieux que l'assertion : une entrée ajoutée sans élargir
le type **ne compile pas**, et l'élargir est le geste visible au `grep` que
mika#1574 demande. L'assertion reste parce qu'elle nomme la règle à qui la lira.

Les tests de comportement du § 5 ne sont pas des détecteurs : leur chemin de
succès est « le comportement attendu s'est produit », pas « aucune violation
trouvée ».

---

## 5. Acceptance criteria

Transcrits **verbatim** du corps de senara-solutions/mika#3036.

- **AC1.** En `phase0_feeder`, une mesure `BranchDeletedByMerge` refuse la
  promotion, sous une raison nommée et distincte de `branch_absent_on_origin`.
  Le même test, dans le même appel, porte le contrôle négatif : la même mesure
  en `phase1_idle_pull` et en `phase2_stuck_rescue` promeut toujours, avec
  `branch_deleted_by_merged_pr` (mika#2837 inchangé).
- **AC2.** La décision se prend dans un seul site, par un `match` sans bras `_`
  sur la phase. Aucun deuxième lecteur de la sonde de merge n'est créé.
- **AC3.** Un refus en phase 0 ne réécrit pas le corps du ticket : aucun
  `auto_pull_callout_rewrite` pour ce ticket dans ce passage. Le test porte le
  témoin d'un passage qui promeut.
- **AC4.** Le refus compte dans le ledger d'exclusion de la phase 0 sous
  `FILTER_PROMOTION_GATE`, avec une ligne d'audit relisible (`issue`,
  `merged_pr`, raison).
- **AC5.** Sonde : on relève, sur les 5 merges suivants d'une PR `Refs` dont le
  ticket reste ouvert, les promotions par le feeder, et on l'écrit ici. La cible
  est 0.

### 5.1 Comment chacun est tenu

| AC | ce qui le tient | où |
|---|---|---|
| AC1 | `mika3036_la_phase_decide_de_la_mesure_de_merge` — **un seul** test, **un seul** `let` de mesure (`BranchDeletedByMerge { pr: 3032 }`), trois appels à `super::classify_promotion` avec les trois variantes. Assertions : phase 0 ⇒ `Decline(ConsentReturnedByMerge { pr: 3032 })` et `slug() == "consent_returned_by_merge"` ; phases 1 et 2 ⇒ `Promote { detail: "branch_deleted_by_merged_pr" }`. Plus `assert_ne!(slug, "branch_absent_on_origin")`, qui est l'exigence écrite et non une paraphrase. | `mod tests` inline |
| AC2 | le `match phase` sans bras `_` (§ 2.3) — tenu par le **compilateur**, et attesté en compte par S1 (§ 4). `ExclusionPhase` n'a pas de `Default` : aucune phase ne peut être omise par inadvertance à un site d'appel. | production + S1 |
| AC3 | `mika3036_un_declin_ne_touche_pas_le_corps_du_ticket` — deux appels à `maybe_rewrite_stale_callout` dans **un** test, sur deux tickets de la bande fixture `990_3xx` : le déclin ⇒ `callout_rewrite_rows(db, session)` vide **et** `gh_calls_for(n)` vide ; le témoin (même mesure, `Phase1Pull`, donc `Promote`) ⇒ une ligne `auto_pull_callout_rewrite` et un `issue edit`. Le témoin est ce qui distingue « il n'a rien écrit » de « rien n'a tourné ». | `mod tests` inline, sceau `gh_process` |
| AC4 | le `ledger.record(ExclusionPhase::Phase0Feeder, n, FILTER_PROMOTION_GATE)` de la l. 5152, **inchangé**, atteint parce que le bras `Decline` rend `false` ; S1 en compte les trois occurrences ; et la ligne d'audit est assertée dans le test d'AC1, qui vérifie sur le JSON de `staleness_audit_json` : `outcome == "decline"`, `reason == "consent_returned_by_merge"`, `issue` et `merged_pr == 3032` tous deux présents et non nuls (Signal O, mika#2331). | site d'appel + S1 + test d'AC1 |
| AC5 | § 7 | commentaire sur le ticket |

---

## 6. Definition of Done

1. `cargo test -p mika-agent` vert, y compris les ~41 appels adaptés par les
   deux helpers du § 2.6 et les bras `Decline` ajoutés aux `match` exhaustifs.
2. `cargo clippy --all-targets -- -D warnings` vert
   (`maybe_rewrite_stale_callout` porte `#[allow(clippy::too_many_arguments)]`
   avec sa justification en commentaire : elle hérite les neuf paramètres du
   bloc dont elle est l'extraction).
3. `cargo fmt --check` vert.
4. `scripts/verify-pipeline.sh origin/main` vert.
5. Les trois tests nommés au § 5.1 passent, et le commit 1 de l'ordre du § 8 a
   été constaté **rouge** avant le commit 2 — le rouge exact est recopié dans le
   corps de la PR.
6. Aucune chaîne de format de fil modifiée : un `grep` sur
   `"phase0_feeder"`, `"phase1_idle_pull"`, `"phase2_stuck_rescue"`,
   `"phase1_pull"`, `"phase2_stuck_ready"`, `"branch_deleted_by_merged_pr"`,
   `"branch_absent_on_origin"` et `"promotion_gate_refused"` rend les mêmes
   valeurs qu'avant le vol.
7. Une note sous `docs/solutions/` : « rendre un consentement et s'en donner un
   sont deux gestes, et la phase est ce qui les sépare » — pourquoi la même
   mesure de merge doit décider différemment selon que le ticket porte déjà le
   consentement ou non, et pourquoi une projection de format de fil s'ajoute au
   lieu de se fusionner.
8. Corps de PR : `Closes #3036`, la mesure du § 1.1, le rouge du commit 1, et
   `Code review: delegated to mika-qa (mika#2694)`.

---

## 7. La sonde AC5, et comment elle se ferme

Sur les **5 merges suivants** d'une PR `Refs` dont le ticket reste ouvert, on
relève les promotions par le feeder. La cible est **0**.

La requête, sur `audit_events` :

```sql
-- Les déclins : la sonde qui dit que le geste a mordu.
SELECT issue, merged_pr, count(*)
  FROM audit_events
 WHERE target_key = 'auto_pull_staleness_measured'
   AND json_extract(after_value, '$.outcome') = 'decline'
   AND created_at > '<deploy>'
 GROUP BY issue, merged_pr;

-- Les promotions survivantes : la sonde qui dit qu'il n'a pas mordu.
SELECT issue FROM audit_events
 WHERE target_key = 'auto_feeder_promoted' AND created_at > '<deploy>';
```

Le relevé va **en commentaire sur mika#3036**, pas dans un ticket neuf. Trois
lectures possibles, et elles ne se confondent pas (mika#2340) :

- des lignes `decline` et **zéro** promotion sur ces tickets ⇒ la cible est
  tenue, et on l'a vue tenir ;
- **ni** `decline` **ni** promotion ⇒ la population n'est pas passée, et la
  sonde n'a rien mesuré. On ne l'écrit pas « 0 » : on l'écrit « non mesurée »,
  et on attend cinq merges de plus ;
- une promotion sur un ticket sans `ready` ⇒ le geste a un trou, et la ligne
  d'audit du § 2.5 dit laquelle des trois phases a décidé.

La seconde lecture est la raison d'être de la première requête : sans elle,
l'absence de promotion serait indistinguable d'un binaire qui précède le vol.
La première colonne à vérifier reste le `attested_at` du déploiement.

---

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/auto_pull.rs` — `ExclusionPhase` `pub` + `gate_phase()` + doc (§ 2.1) | 22 |
| `crates/mika-agent/src/auto_pull.rs` — `DeclineReason`, `PromotionGate::Decline`, `authorizes_body_rewrite()` (§ 2.2, 2.4) | 58 |
| `crates/mika-agent/src/auto_pull.rs` — `classify_promotion` : 4e paramètre, `match phase`, doc (§ 2.3) | 28 |
| `crates/mika-agent/src/auto_pull.rs` — `promotion_gate_allows` : phase typée, bras `Decline`, bras d'audit (§ 2.5) | 34 |
| `crates/mika-agent/src/auto_pull.rs` — extraction `maybe_rewrite_stale_callout` (§ 2.4) | 42 |
| `crates/mika-agent/src/auto_pull.rs` — les trois sites d'appel (§ 2.5) | 3 |
| tests inline — helper de phase + bras `Decline` des `match` existants (§ 2.6) | 32 |
| tests inline — AC1 (table trois phases) + assertions d'audit d'AC4 | 75 |
| tests inline — AC3 (déclin muet + témoin promouvant, un seul test) | 70 |
| tests inline — scan S1 + allowlist vide (§ 4) | 18 |
| `crates/mika-agent/tests/auto_pull_promotion_gate.rs` — helper + bras `Decline` | 18 |

Total estimé : 400 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient, et où est la marge.** 400 pour 429 (93 %), et pour
1 000 de seuil par PR : c'est serré, et trois choses le rendent tenable.

*Un seul fichier de production.* Tout le geste vit dans `auto_pull.rs`. Aucun
crate voisin, aucune migration de schéma, aucun `skills/bundled/`, donc une
seule unité de compilation dans la boucle de vérification.

*Le poste le plus gros est un déplacement, pas une écriture.* Les 42 lignes de
`maybe_rewrite_stale_callout` sont, pour 35 d'entre elles, le bloc des
l. 3929-3965 déplacé tel quel ; seuls la signature, le garde-fou de trois lignes
et le doc-comment sont neufs. L'estimation les compte pleines par prudence —
c'est là qu'est la marge réelle.

*Les 41 appels existants coûtent deux helpers, pas 41 éditions.* Le shadowing du
§ 2.6 est ce qui empêche ce vol de mourir en tours plutôt qu'en lignes : 41
éditions `Edit` à des formes d'appel toutes différentes, c'est le mode de mort de
mika#2683 et mika#2654, et il ne se lit pas dans un compte de lignes.

**Pourquoi un découpage en vols serait pire.** Le découpage naturel serait
AC1+AC2+AC4 (la décision) puis AC3 (l'écriture). Mais le vol 1 livrerait alors
un moteur qui décline et **réécrit quand même** le corps du ticket qu'il a choisi
de ne pas toucher — donc une écriture sous le jeton de l'App sur un ticket sain,
qui est la moitié du défaut mesuré au § 1.1 (« dans les deux cas, le feeder a
aussi réécrit le callout »). Les deux moitiés partagent une seule décision,
`decision`, et les séparer demanderait de la lire deux fois, ce qu'AC2 interdit
nommément.

---

## 9. Ordre de commit

Chaque commit laisse l'arbre vert, sauf le premier, dont le rouge est le but.

1. `test(3036): la phase 0 promeut sur une mesure de merge — rouge attendu` —
   le test d'AC1 seul, sur la signature d'aujourd'hui, avec le `Decline` attendu
   écrit en `assert!(matches!(…))` qui échoue. **Rouge**, et c'est la panne du
   ticket reproduite à la demande ; le rouge exact est consigné dans le corps de
   PR.
2. `fix(3036): la phase entre dans la décision de la porte de promotion` —
   § 2.1 à 2.3, 2.5 et 2.6. Le test du commit 1 passe au vert, les ~41 appels
   sont adaptés par les deux helpers, les trois sites d'appel passent la
   variante.
3. `fix(3036): un déclin de la porte ne réécrit pas le corps du ticket` —
   § 2.4 : `authorizes_body_rewrite()`, l'extraction de
   `maybe_rewrite_stale_callout`, et le test d'AC3 avec son témoin promouvant.
4. `test(3036): un scan compte le lecteur de phase et les trois ledgers` — S1
   et son allowlist vide (§ 4), greffés sur le scan hôte de la l. 6948.
5. `docs(solutions): rendre un consentement et s'en donner un sont deux gestes`
   — la note du § 6.7, et la raison pour laquelle une projection de format de
   fil s'ajoute au lieu de se fusionner.
