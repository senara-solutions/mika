# mika#2917 — le pré-filtre compare la tête et n'écarte que ce qui est déjà revu (vol 3/3)

> Ticket : `senara-solutions/mika#2917`, enfant de mika#2680, classe (g′), bearing Prime ratifié par Vincent le 2026-10-26 : **filtrer**, forme (a).
> Vols antérieurs, tous deux mergés dans `main` :
> - vol 1 — `docs/plans/2026-10-26-001-feat-2917-webhook-prefilter-2680-g-forme-a-plan.md`, PR #2918 (`4502306c`) : le texte du `check_suite` porte la tête que le gateway jetait.
> - vol 2 — `docs/plans/2026-10-26-002-feat-2917-webhook-prefilter-2680-g-forme-a-vol-2-plan.md`, PR #2919 (`25e3b0f1`) : le registre des têtes revues, et son écrivain unique.

## Où ce vol se situe

Le vol 2 a déclaré le périmètre du vol 3 mot pour mot :

> *Pièce 3 : la classe (g′), le terme de lecture du trait `PrefilterState`, le décideur, l'interrupteur dédié, le corpus rejoué.*
> Plancher : *AC1, AC2, AC3 verts ; AC4 mesurable après merge.*

Ce plan livre exactement cette pièce, et rien d'autre. Les deux moitiés qu'il
compare existent déjà sur `main` :

| moitié | livrée par | ce qu'elle porte |
|---|---|---|
| la tête, dans le texte | vol 1 | `CHECK_SUITE_HEAD_LINE_PREFIX`, `CHECK_SUITE_PRS_LINE_PREFIX`, `parse_check_suite_heads` → `CheckSuiteHeads::{LineAbsent, Empty, One{pr,head_sha}, Many(n)}` (`crates/mika-common/src/github_event_format.rs`) ; `Candidate::GreenPrBranch { repo, branch, heads }` la transporte jusqu'à `decide`, où elle est délibérément ignorée |
| la tête, au registre | vol 2 | `tool_name = QA_REVIEWED_HEAD_TOOL` (`"qa_reviewed_head"`), `target_key = head_key(repo, pr, head_sha)` → `pr:{repo}#{n}@{sha}`, `after_value` ∈ `REVIEWED_HEAD_OUTCOMES`, écrivain unique `tools/record_reviewed_head.rs` (`crates/mika-agent/src/qa_reviewed_head.rs`) |

Le vol 3 est **la comparaison**, et elle seule. Aucune des deux moitiés n'est
retouchée : ni le gateway, ni la grammaire, ni le prompt du skill, ni l'écrivain.

## Ce qui est refusé (bearing, non négociable)

- **Aucun appel `gh` depuis `webhook_prefilter.rs`**, sous quelque identité que
  ce soit (mika#2260). Le site préexistant `pr_state` (classe (c)) reste le seul,
  et reste inscrit tel quel dans `GH_CALL_SITES_ALLOWED` : **le vol 3 n'y ajoute
  aucune entrée**, ce qui est l'audit point 3 du ticket et une assertion de ce
  plan, pas un espoir.
- **Aucune copie de la logique de dedup du skill dans le Rust.** Un seul site de
  vérité par donnée : la clé vient de `head_key`, l'issue canonique de
  `reviewed_head_outcome`, le nom de l'outil de `QA_REVIEWED_HEAD_TOOL`. Le
  vol 2 porte déjà la garde qui l'impose
  (`mika2917_le_module_ne_retape_pas_la_grammaire_de_cle`).
- **Aucune écriture au registre par le moteur.** `record_reviewed_head` reste
  SOLE WRITER ; le vol 3 est un lecteur, et le seul.

## Requirements

### R1 — La classe (g′)

Une variante neuve de `PrefilterClass`, dans `webhook_prefilter.rs` :

```rust
/// (g′) `check_suite` verte d'une branche de PR dont la tête nommée par le
/// texte figure déjà au registre des têtes revues (mika#2917, vol 3).
GreenCheckSuiteHeadAlreadyReviewed,
```

avec `as_str() => "green_check_suite_head_already_reviewed"`. C'est un **format
de fil** : `measure-webhook-turn-cost` groupe dynamiquement sur `.class`, donc la
classe apparaît dans `skipped_by_class` et dans `skipped_by_agent_class` sans
aucune modification du script (AC4). Une orthographe qui change scinde une
population sans le dire (mika#2205) — d'où l'assertion de littéral, sur le modèle
de `mika2917_lissue_rendue_est_le_litteral_canonique`.

Le `target` du skip prend la forme que (b) emploie déjà :
`check_suite:{repo}#{n}@{sha}`, obtenue en retirant le préfixe `pr:` de la clé.
Même forme, classes distinctes : c'est `class` qui sépare les deux populations,
jamais `target`.

### R2 — Le terme de lecture du trait

Un neuvième terme sur `PrefilterState` :

```rust
/// (g′) l'issue attestée au registre pour CETTE tête, telle que
/// `record_reviewed_head` l'a écrite — `None` quand le registre ne porte pas la
/// tête. La valeur est rendue BRUTE : la validation contre
/// [`crate::qa_reviewed_head::REVIEWED_HEAD_OUTCOMES`] appartient au décideur,
/// pour qu'une valeur hors contrat soit une relâche nommée et non un silence.
async fn reviewed_head(&self, head_key: &str) -> anyhow::Result<Option<String>>;
```

L'implémentation sur `LiveState` est une ligne de base, sans `gh` :

```rust
async fn reviewed_head(&self, head_key: &str) -> anyhow::Result<Option<String>> {
    let since = crate::timestamp::now_minus(REVIEWED_HEAD_WINDOW);
    Ok(self
        .db
        .latest_audit_event_for_target(QA_REVIEWED_HEAD_TOOL, head_key, &since)
        .await?
        .and_then(|p| p.after_value))
}
```

Trois propriétés de cet appel, chacune décidée et non héritée :

- **`latest_…` et non `count_…`.** La décision a besoin de l'issue, pas d'un
  cardinal ; `LatestAuditEventProjection` (mika#2242) projette `after_value`,
  `reasoning` et `created_at`, ordonne `created_at DESC, id DESC LIMIT 1`, et
  rend `after_value` comme l'`Option<String>` que la colonne est réellement —
  jamais coercé en `""` (mika#2331). Un `and_then` propage donc l'absence de
  valeur comme une absence, et le décideur la nomme.
- **Le scope par `agent_id` est structurel, pas ajouté.**
  `AsyncDatabase::latest_audit_event_for_target` clone `self.agent_id` et le
  passe en paramètre SQL 1 (`WHERE agent_id = ?1 AND tool_name = ?2 AND
  target_key = ?3 AND created_at > ?4`) ; l'appelant ne peut pas le passer, donc
  ne peut pas le passer de travers. C'est l'audit point 1 du ticket : une ligne
  écrite par mika-qa est hors du scope de mika-dev **par construction**. Le test
  croisé que le ticket exige (R7) **atteste** cette propriété au lieu de
  l'implémenter — et c'est pour cela qu'il doit mordre au niveau de la base, pas
  du bouchon.
- **`REVIEWED_HEAD_WINDOW` est une fenêtre et non l'éternité.** La signature
  exige un `since`. 30 jours : un `check_suite` sur une tête plus ancienne est
  hors régime, et le repli d'une fenêtre trop courte est du bon côté (pas de
  ligne ⇒ le tour a lieu). Constante dans `qa_reviewed_head.rs`, à côté du nom
  de l'outil, pas dans le pré-filtre : c'est une propriété du registre.

### R3 — Le décideur

Le site exact que le vol 2 a laissé en attente, dans `decide` :

```rust
// `heads` est délibérément ignoré : dans ce vol la tête traverse et
// rien ne la lit (mika#2917).
Candidate::GreenPrBranch { repo, branch, .. } => {
    decide_head_decided(&repo, &branch, state).await
}
```

devient :

```rust
Candidate::GreenPrBranch { repo, branch, heads } => {
    decide_green_pr_branch(&repo, &branch, &heads, state).await
}
```

et `decide_green_pr_branch` a **une seule forme**, répétée sur chacun de ses
quatre bras : (g′) tente l'écart ; si elle ne l'obtient pas, **(b) est consultée
entière** ; si (b) ne mord pas non plus, la relâche rendue est celle que (g′) a
nommée.

| le texte dit | (g′) | sinon |
|---|---|---|
| `CheckSuiteHeads::One { pr, head_sha }`, interrupteur armé, registre porte une issue canonique | `Skip { GreenCheckSuiteHeadAlreadyReviewed, target }` | — |
| `One`, registre muet | — | repli (b), relâche `ReviewedHeadAbsent` |
| `One`, registre porte une valeur hors des trois littéraux | — | repli (b), relâche `ReviewedHeadOutOfContract` |
| `One`, lecture en erreur | — | WARN `webhook_prefilter_state_unreadable`, repli (b), relâche `StateUnreadable` |
| `One`, interrupteur désarmé | — | repli (b), relâche `ReviewedHeadSwitchDisarmed` |
| `LineAbsent`, `Empty`, `Many(_)` | — | (b) **entière, relâches comprises** |

**Pourquoi le repli sur (b) est systématique.** (b)
(`GreenCheckSuiteHeadDecided`, mika#2680) écarte aujourd'hui une partie de cette
population sur la trace DECISION-CORE du même événement. Le vol 3 **ne doit rien
lui retirer sur aucune entrée** : un tour que (b) écarte aujourd'hui doit rester
écarté, quelle que soit la réponse de (g′) et quel que soit l'état de
l'interrupteur. C'est la propriété de non-régression du vol, et R8 la teste
directement. Elle est aussi ce qui rend l'interrupteur dédié sans danger : le
désarmer rend exactement le régime de `main` avant ce vol, pas un régime neuf.

**Pourquoi les trois premières lignes du tableau ne consultent pas (g′) et (b)
dans l'autre ordre.** Sur la population `One`, (g′) est la porte de référence —
son critère nomme la tête, donc il est strictement plus proche de la cause que
celui de (b), qui lit une redécision. Quand les deux mordraient, c'est (g′) qui
est inscrite au registre.

**La conséquence mesurable de cet ordre, écrite parce qu'elle sera lue comme une
régression sinon.** Au déploiement, sur la population `One` :

- `skipped_by_class.green_check_suite_head_decided` **baisse**, et cette baisse
  est un **transfert** vers `green_check_suite_head_already_reviewed`, jamais une
  perte. La grandeur conservée est la **somme** des deux classes, et c'est elle
  qu'on compare avant/après.
- `charged_by_release.no_decision_attested` et
  `charged_by_release.head_not_previously_decided` baissent au profit de
  `reviewed_head_absent`. Même nature : un transfert, pas une disparition. Les
  deux termes de (b) restent intégralement en service sur `LineAbsent`, `Empty`
  et `Many`, qui sont les populations que (g′) ne touche pas.

Les deux populations `Empty` et `Many` sont **séparées par la grammaire du vol 1
et ne sont jamais sommées** (mika#2205) : `Empty` est « la forge ne nomme aucune
tête » (mesuré sur mika#2681), `Many` est « plusieurs PR ». Elles tombent toutes
deux dans le même bras du décideur — (b) entière — mais elles sont deux variantes
distinctes du type, donc deux cas distincts d'AC2, donc deux tests.

### R4 — Les termes de relâche

Trois variantes neuves de `Release`, avec leur `as_str()` :

| variante | `as_str()` | régime attendu |
|---|---|---|
| `ReviewedHeadAbsent` | `reviewed_head_absent` | le gros de la population au début, puis décroissant ; c'est le contrôle négatif d'AC1 — une tête neuve n'est pas au registre, donc le tour a lieu |
| `ReviewedHeadOutOfContract` | `reviewed_head_out_of_contract` | **zéro**. `record_reviewed_head` valide par `reviewed_head_outcome` avant d'écrire, donc une valeur hors contrat au registre est un défaut d'écrivain. Le terme existe pour que ce défaut soit lisible plutôt que silencieux |
| `ReviewedHeadSwitchDisarmed` | `reviewed_head_switch_disarmed` | **zéro** en régime armé. C'est le contrôle positif du désarmement : sans lui, « l'interrupteur est baissé » et « la classe ne mord sur rien » rendraient des octets identiques (mika#2205) |

`Release::StateUnreadable` est **réutilisé** et non dédoublé : il est
délibérément UN terme pour tous les sites de lecture en erreur, et (g′) en est le
sixième. La seule modification à ce mécanisme est l'extraction de sa ligne WARN :

```rust
fn warn_state_unreadable(what: &str, target: &str, e: &anyhow::Error) { … }

fn state_unreadable(what: &str, target: &str, e: &anyhow::Error) -> Decision {
    warn_state_unreadable(what, target, e);
    Decision::Llm(Release::StateUnreadable)
}
```

parce que (g′) doit émettre le WARN **puis** replier sur (b), au lieu de rendre
la `Decision` immédiatement. Un seul site de vérité du WARN, cinq appelants
inchangés.

### R5 — L'interrupteur dédié

```rust
/// L'interrupteur dédié de la classe (g′). Voir [`reviewed_head_enabled`].
pub(crate) const REVIEWED_HEAD_ENV: &str = "MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD";
```

`reviewed_head_enabled()` a **exactement** la table de vérité de
`prefilter_enabled` — **l'absence vaut `on`**, valeur non reconnue laisse armé
avec son WARN, via `qa_head_supersession::parse_switch`. C'est la ligne où il
diverge de `triage_enabled` (classe (h), absence vaut `off`), et la divergence
est argumentée : (h) livre un appel de modèle et doit être armé par décision ;
(g′) ne livre qu'une lecture de base dont tous les échecs laissent passer le
tour, donc son coût d'erreur est un tour payé, celui d'aujourd'hui.

Ce que l'interrupteur est, et pourquoi le vol 2 l'a annoncé : le vol 2 a nommé un
risque résiduel qu'aucun de ses trois filets ne couvrait — *une revue dépêchée
qui meurt sans émettre `review_failed` laisse une ligne `review` sur une tête non
revue* — et a écrit que le troisième filet serait **l'interrupteur dédié du
vol 3, qui désarme la classe entière**. C'est celui-ci. Il retire (g′) de service
sans redéploiement et sans toucher aux autres classes ; `MIKA_WEBHOOK_PREFILTER`
le désarme en amont, et cette fonction n'a pas à le savoir.

Il est lu **dans le décideur**, pas dans `skip_turn` : un désarmement de (g′)
doit laisser (b) décider, ce qui serait impossible depuis la porte.

### R6 — Le corpus rejoué (AC1)

Les 51 tours de la mesure du 2026-10-26 ne sont pas rejouables tels quels — leurs
payloads n'ont pas été capturés. Le corpus est donc **construit**, table-driven,
chaque ligne dérivée d'une issue attestée de la mesure, et il vit dans le module
de test de `webhook_prefilter.rs` :

```rust
struct Cas {
    nom: &'static str,
    texte: &'static str,                 // un check_suite complet, grammaire du vol 1
    registre: Option<&'static str>,      // l'issue au registre pour la tête du texte
    attendu: Attendu,                    // Ecarte(class) | Passe(release)
}
```

Les lignes, toutes dans **un seul** `#[tokio::test]` comme AC1 l'exige :

| ligne | registre | attendu |
|---|---|---|
| tête nommée, un tour antérieur a posté une revue | `review` | écarté (g′) |
| tête nommée, un tour antérieur a conclu au verdict déjà enregistré | `stop_verdict_recorded` | écarté (g′) |
| tête nommée, un tour antérieur a conclu à la tête déjà revue | `stop_already_reviewed_sha` | écarté (g′) |
| **contrôle négatif** : même PR, tête NEUVE, la PR porte déjà une ligne sur son ANCIENNE tête | aucune ligne pour cette tête | passe, `reviewed_head_absent` |

Le contrôle négatif est **structurel et non conventionnel**, et c'est la propriété
qui ferme AC1 : un nouveau push produit un nouveau `head_sha`, donc une clé que le
registre ne porte pas. Le fixture le matérialise en écrivant la ligne sur
`pr:{repo}#{n}@{sha_ancien}` et en servant un texte portant `{sha_neuf}` — aucune
assertion ne porte sur une convention de nommage, seulement sur la clé.

« Aucun tour terminé en `review` n'est écarté » tient par la même structure : un
tour qui se *terminerait* en `review` est un tour sur une tête que le registre ne
porte pas encore. Les trois issues au registre font donc toutes écarter, et c'est
le bon arbitrage — une ligne `review` dit « un tour a déjà posté la revue de
cette tête », dont le successeur se terminerait en `stop_already_reviewed_sha`.

### R7 — Le test croisé `agent_id` (audit point 1)

Au niveau de la **base**, dans le module de test de `qa_reviewed_head.rs`, pas au
niveau du bouchon :

1. `AsyncDatabase::open_in_memory`, un seul fichier (un conteneur = une base).
2. `db.with_agent("mika-qa")` écrit la ligne sur `head_key(repo, pr, sha)` via
   `log_audit_event` — la forme exacte que `record_reviewed_head` produit.
3. `db.with_agent("mika-dev").latest_audit_event_for_target(QA_REVIEWED_HEAD_TOOL,
   &key, &since)` rend **`None`**.
4. Contrôle positif dans le même test :
   `db.with_agent("mika-qa")` sur la même clé rend `Some("review")`. Sans lui,
   « le scope filtre » et « le fixture n'a rien écrit » rendraient le même `None`.

`with_agent` est un clone partageant l'`Arc` interne — aucune seconde connexion,
aucun second thread, et surtout pas deux bases en mémoire, ce qui rendrait la
propriété intestable au lieu de la faire échouer (motif mika#1926).

Un bouchon ne peut pas porter ce test : `Stub::reviewed_head` rend ce qu'on lui
dit, donc il atteste le décideur, jamais le scope.

### R8 — La non-régression de (b)

Un test dédié, et non un corollaire : pour chaque entrée du corpus où (b) écarte
aujourd'hui, l'écart **subsiste** après le vol 3 — sous la classe (g′) quand elle
mord, sous la classe (b) sinon — et il subsiste aussi **interrupteur (g′)
désarmé**, où la classe rendue est exactement celle de `main`.

C'est le test qui rend l'interrupteur crédible : il dit que le baisser rend le
régime d'avant, et pas un troisième régime que personne n'a mesuré.

## Non-Requirements

Hors de ce vol, et pour chacun la raison :

- **Les arrêts 1 et 2** (`stop_no_open_pr`, `stop_out_of_scope` : 4 tours,
  0,64 M tokens) — hors périmètre par le ticket. Ils ne passent pas par le
  registre des têtes.
- **La forme (b)** — à n'ouvrir que si (a) échoue au groom. Elle ne l'a pas fait.
- **Toute retouche du gateway, de la grammaire, du prompt du skill ou de
  l'écrivain.** Les deux moitiés comparées sont sur `main` et sont tenues par
  leurs propres gardes.
- **AC4** — post-merge et non bloquant, par le ticket lui-même. Ce plan en porte
  la procédure (§ Sonde post-déploiement) sans en faire une condition de merge.
- **Toute écriture au registre par le moteur**, et toute seconde source d'état
  de dedup. Un seul site de vérité, un seul écrivain, un seul lecteur.
- **Une entrée neuve dans `GH_CALL_SITES_ALLOWED`.** Le vol 3 n'appelle pas `gh`.

## Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | AC1 : les trois issues au registre font écarter, et la tête neuve passe | `cargo test -p mika-agent --lib webhook_prefilter::tests::mika2917_vol3_le_corpus_rejoue` — un seul test, contrôle négatif inclus |
| V2 | AC2 : `LineAbsent`, `Empty`, `Many`, registre absent, registre illisible | cinq tests, un par cas (`…_ac2_ligne_absente`, `…_ac2_aucune_tete`, `…_ac2_plusieurs_pr`, `…_ac2_registre_muet`, `…_ac2_registre_illisible`) |
| V3 | AC3 : aucun appel `gh` dans `webhook_prefilter.rs`, `GH_CALL_SITES_ALLOWED` inchangé | `mika2917_a1_les_sites_gh_du_prefiltre_sont_recenses` (vol 2) reste vert **sans nouvelle entrée** — assertion explicite sur le contenu de la constante |
| V4 | AC3 : un seul lecteur du registre | `mika2917_a2_un_seul_ecrivain_et_au_plus_un_lecteur` resserré de `readers.len() <= 1` à un exact-set `vec!["server/webhook_prefilter.rs"]` |
| V5 | le scope `agent_id` (audit point 1) | `qa_reviewed_head::tests::mika2917_vol3_une_ligne_de_mika_qa_necarte_pas_le_tour_de_mika_dev`, au niveau `AsyncDatabase`, avec son contrôle positif |
| V6 | non-régression de (b) | `mika2917_vol3_aucun_ecart_de_b_nest_perdu`, interrupteur armé et désarmé |
| V7 | l'interrupteur | `mika2917_vol3_linterrupteur_desarme_rend_le_regime_anterieur` + la relâche `reviewed_head_switch_disarmed` |
| V8 | les formats de fil | assertions de littéral sur `GreenCheckSuiteHeadAlreadyReviewed.as_str()` et sur les trois `Release::as_str()` |
| V9 | pas de découpage d'octets | `scripts/check-byte-slices.sh` (mika#764) — le décideur ne manipule que des `&str` entiers et `strip_prefix` |
| V10 | la chaîne complète | `cargo test`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main` |

## Ordre de commit

Chaque commit laisse la suite verte, et le détecteur précède ce qu'il juge quand
c'est possible (doctrine garde-avant-diff ; les deux gardes structurelles de
l'AC3 ont été délibérément posées au vol 2 pour être vertes sur `main` **avant**
le diff du vol 3).

1. **`REVIEWED_HEAD_WINDOW` + le terme de trait + son impl sur `LiveState` + le
   `Stub` étendu.** Aucun décideur ne lit encore le terme : le vol ajoute une
   lecture avant d'ajouter une décision, donc ce commit ne change aucun verdict.
   Vert par construction.
2. **Le test croisé `agent_id` (R7).** Il porte sur le terme du commit 1 et sur
   rien d'autre ; il part vert, parce qu'il atteste une propriété structurelle de
   `AsyncDatabase` plutôt qu'une ligne neuve.
3. **La classe (g′), les trois `Release`, l'extraction de `warn_state_unreadable`,
   l'interrupteur.** Du vocabulaire et un interrupteur ; aucun décideur ne les
   lit encore. Les assertions de littéral (V8) arrivent ici.
4. **Le décideur `decide_green_pr_branch`, branché dans `decide`.** C'est le seul
   commit qui change un verdict. Il arrive avec le corpus rejoué (V1), les cinq
   cas d'AC2 (V2), la non-régression (V6) et l'interrupteur (V7).
5. **Le resserrement de la garde (a.2)** de `readers.len() <= 1` à l'exact-set
   `vec!["server/webhook_prefilter.rs"]`, plus l'assertion que
   `GH_CALL_SITES_ALLOWED` est inchangé (V3). En dernier, parce que l'exact-set
   ne peut être écrit qu'une fois le lecteur posé — et il doit être **vu rouge**
   avant le commit 4 pour attester qu'il voit quelque chose.
6. **`docs/operator/environment-variables.md`** : l'entrée
   `MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD` (défaut, table de vérité, sa
   divergence argumentée avec `MIKA_WEBHOOK_COMMENT_TRIAGE`, la sonde
   d'opérateur). Hors du compte de lignes.

## Definition of Done

- Les quatre critères du ticket sont traités : AC1, AC2, AC3 verts dans la PR ;
  AC4 a sa procédure écrite et mesurable après merge.
- `cargo test`, `cargo clippy`, `cargo fmt --check`,
  `scripts/check-byte-slices.sh`, `make verify-bundled-skills` et
  `scripts/verify-pipeline.sh origin/main` passent.
- Les trois points de l'audit MPC de #2919 sont traités dans la PR : le scope
  `agent_id` est attesté au niveau base (R7/V5) ; la procédure de `diff` du
  prompt déployé est écrite avant AC4 (§ Sonde post-déploiement) ;
  `GH_CALL_SITES_ALLOWED` est vert **sans nouvelle entrée** (V3).
- La garde (a.2) est resserrée à un exact-set nommant le lecteur unique, et a été
  vue rouge avant que le lecteur n'existe.
- Le corps de PR porte `Closes #2917`, `Code review: delegated to mika-qa
  (mika#2694)`, et la section « conséquence mesurable » du § R3 — le transfert de
  population entre les classes (b) et (g′) — pour que mika-qa ne lise pas la
  baisse de `green_check_suite_head_decided` comme une régression.

## Acceptance criteria

Transcrits mot pour mot du corps de `senara-solutions/mika#2917` :

- **AC1** : sur un corpus rejoué, chaque tour qui se serait terminé en
  `stop_verdict_recorded` ou `stop_already_reviewed_sha` est écarté, et **aucun**
  tour terminé en `review` ne l'est. Le contrôle négatif, une nouvelle tête sur
  une PR déjà revue, doit passer, dans le même test.
- **AC2** : `pull_requests` vide, plusieurs PR, état absent ou illisible : le
  tour passe. Un test par cas.
- **AC3** : le diff ne contient aucun appel `gh` dans `webhook_prefilter.rs`, et
  le Rust ne lit qu'une seule source pour l'état de dedup. Une garde de test
  l'impose.
- **AC4** : après déploiement, `measure-webhook-turn-cost` montre la classe
  nouvelle dans `skipped_by_class`. Les issues `stop_verdict_recorded` et
  `stop_already_reviewed_sha` des tours payés baissent, et `review` ne baisse
  pas. Cette mesure se fait après le merge et ne bloque pas la PR.

## Fire-Disposition

Le vol livre des détecteurs : les cinq cas d'AC2, le corpus d'AC1, le test de
non-régression, le test croisé `agent_id`, et surtout le **resserrement** de la
garde structurelle (a.2). Disposition retenue : **(a) exception nommée en
allowlist**, sur les deux gardes structurelles, aucune des deux n'étant livrée
désarmée.

**(a.1) — le recensement des sites `gh` (`GH_CALL_SITES_ALLOWED`).** Posée au
vol 2, exact-set contre `&["pr_state"]`, anti-vacuité `enumerated >= 20`. Le
vol 3 **n'y ajoute aucune entrée**, ce qui est l'audit point 3 du ticket. La
donnée nommée par l'exception est précise — `pr_state`, le seul site `gh` du
pré-filtre, classe (c), mika#2260 — et l'assertion est auto-nettoyante par sa
forme même : un exact-set rougit aussi bien sur une entrée en trop que sur une
entrée manquante, donc le jour où `pr_state` disparaît, l'exception expire d'elle-même.
Ce plan ajoute une assertion explicite sur le **contenu** de la constante, pour
que le passage du vol 3 ne puisse pas y glisser une entrée en restant vert.

**(a.2) — un seul écrivain, un seul lecteur.** Posée au vol 2 avec
`writers == vec!["tools/record_reviewed_head.rs"]` et `readers.len() <= 1`, le
`<=` étant délibéré pour qu'elle morde aux deux vols sans rougir par construction
au vol 2, où aucun lecteur n'existait. **Le vol 3 la resserre** :
`readers == vec!["server/webhook_prefilter.rs"]`, exact-set. Chaque moitié nomme
sa donnée précise, référence mika#2917, et l'exact-set est l'assertion
auto-nettoyante : un second lecteur du registre — la forme exacte de ce que le
bearing refuse, « aucune copie de la logique de dedup dans le Rust » — la fait
rougir, et un lecteur renommé la fait rougir aussi. L'anti-vacuité du vol 2
(`files.len() >= 100`) est conservée : sans elle, un scan qui ne lit rien rend un
exact-set vide et reste vert.

**Le resserrement doit être vu rouge avant d'être cru.** L'exact-set est écrit au
commit 5, après le lecteur ; mais il est **exécuté une fois avant le commit 4**,
sur un arbre où le lecteur n'existe pas encore, et il doit échouer. Un détecteur
qui démarre vert n'a jamais montré qu'il voit — c'est la raison même pour
laquelle les deux gardes ont été posées au vol 2 plutôt qu'ici.

Aucune violation existante n'est portée en exception par ce vol : les deux
exact-sets sont exacts, et il n'y a pas de résidu à tracker.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/webhook_prefilter.rs` — classe (g′), 3 variantes `Release`, terme de trait + impl `LiveState`, `decide_green_pr_branch`, interrupteur, extraction de `warn_state_unreadable` | 145 |
| `crates/mika-agent/src/qa_reviewed_head.rs` — `REVIEWED_HEAD_WINDOW`, resserrement de la garde (a.2) en exact-set | 40 |
| tests (`webhook_prefilter.rs § tests`) — corpus rejoué AC1 + contrôle négatif, AC2 ×5, non-régression de (b), interrupteur, assertions de littéral, `Stub` étendu | 160 |
| tests (`qa_reviewed_head.rs § tests`) — test croisé `agent_id` au niveau `AsyncDatabase`, avec contrôle positif | 45 |

Total estimé : 390 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 390 est sous la borne par vol de 429 et très en
deçà du seuil par PR de 1000, donc aucun découpage n'est requis et il n'y a pas
de `Tracked in:` à écrire. Trois raisons de croire l'estimation, et non une
seule :

- **Les deux moitiés comparées existent déjà.** Le vol ne construit ni grammaire
  ni état : il ajoute une lecture, un vocabulaire et un décideur. Aucun des
  quatre livrables n'ouvre un fichier neuf.
- **Le précédent du vol lui-même.** Le vol 1 était estimé à 365 lignes et a
  mergé en un seul pilote ; le vol 2 à 381. Le vol 3 est de la même grandeur, sur
  le même fichier, avec le même outillage de test (`Stub`, `skip`,
  `source_scan`), déjà en place.
- **Aucune migration, aucune table, aucun fichier d'état.** La décision du vol 2
  — une ligne d'`audit_events` sous un `tool_name` dédié — est ce qui rend ce
  vol-ci petit : le lecteur est un appel d'accesseur existant.

Le poste le plus lourd est le test (205 des 390). C'est voulu : AC1 et AC2
exigent un corpus et cinq cas, et le test croisé d'`agent_id` est la seule
attestation d'une propriété que le bouchon ne peut pas porter.

## Sonde post-déploiement (vol 3)

`make deploy` est **obligatoire** avant toute mesure : rien ne lit
`skills/bundled/` au runtime, et le binaire est ce qui porte le décideur
(mika#2340).

**Ordre imposé — le prompt d'abord, la mesure ensuite.** C'est l'audit point 2 du
ticket, et l'ignorer rend AC4 ininterprétable : si l'écrivain du vol 2 est resté
muet parce que son prompt n'a pas atterri, le vol 3 ne filtre rien, et
`skipped_by_class` sans la classe neuve ne dira pas lequel des deux vols est en
cause.

1. **Attester le binaire :** `cat ~/.mika/skills/.manifest-writer` — le
   `git_hash` doit être celui qu'on vient de construire. `git_hash: "unknown"`
   signifie un binaire bâti hors checkout, pas un déploiement raté.
2. **Attester le prompt, par `diff` et non par confiance :** comparer le
   `system_prompt.md` résolu de `qa-review-webhook-success` (sous
   `~/.mika/agents/mika-qa/skills/`, à travers le lien) au fichier du dépôt. Une
   mise à jour de skill peut ne rien faire (mika#2340, mesuré le 2026-09-16) :
   `mika skills --agent mika-qa update` depuis un binaire périmé réextrait une
   bibliothèque périmée et rend succès honnêtement. Un `diff` non vide ⇒
   retourner à l'étape 1, ne pas mesurer.
3. **Attester l'écrivain :** sur un `check_suite` success réel d'une branche de
   PR routée vers mika-qa, une ligne d'`audit_events` sous
   `tool_name = 'qa_reviewed_head'` doit apparaître avec
   `target_key = 'pr:{repo}#{n}@{sha}'` et un `after_value` égal à la ligne
   `Webhook-Outcome:` du même tour (mika#2855). C'est la sonde du vol 2, et elle
   est la précondition de la nôtre : sans ligne au registre, (g′) n'a rien à
   comparer.
4. **Attester le lecteur (AC4) :** `measure-webhook-turn-cost` sur une fenêtre
   couvrant le déploiement. Trois lectures, et la troisième est celle qu'il faut
   écrire d'avance :
   - `skipped_by_class.green_check_suite_head_already_reviewed` **apparaît**
     (le script groupe dynamiquement sur `.class`, donc sans modification) ;
     `skipped_by_agent_class` la ventile par agent, et les deux agents doivent y
     figurer séparément.
   - `charged_by_outcome.stop_verdict_recorded` et
     `charged_by_outcome.stop_already_reviewed_sha` **baissent** ;
     `charged_by_outcome.review` **ne baisse pas**. C'est AC4 mot pour mot.
   - `skipped_by_class.green_check_suite_head_decided` **baisse aussi**, et ce
     n'est **pas** une régression : c'est le transfert de population décrit au
     § R3. La grandeur à comparer avant/après est la **somme** des deux classes,
     jamais l'une seule. Même chose côté relâches entre
     `no_decision_attested` / `head_not_previously_decided` et
     `reviewed_head_absent`.
5. **Les deux zéros qui doivent rester zéro :**
   `charged_by_release.reviewed_head_out_of_contract` (un écrivain qui écrirait
   hors contrat) et `charged_by_release.reviewed_head_switch_disarmed` (un
   interrupteur baissé par accident). Ce sont des zéros qui attestent le
   câblage, donc leur **absence** du rapport est l'anomalie à lire, pas leur
   valeur nulle (mika#2205).

**Le geste de retrait, si une perte de revue est observée :** poser
`MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD=off` et redémarrer. La classe sort de
service, (b) reprend l'intégralité de la population, et
`reviewed_head_switch_disarmed` monte — ce qui rend le retrait lisible dans le
même rapport que la mesure. `MIKA_WEBHOOK_PREFILTER=off` reste le désarmement
global, en amont.
