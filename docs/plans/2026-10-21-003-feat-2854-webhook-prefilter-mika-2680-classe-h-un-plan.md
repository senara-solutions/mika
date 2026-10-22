# mika#2854 — classe (h) du pré-filtre webhook, vol 2 : la moitié `issues` `closed`

> **Vol 2 d'une série de 3.** Le vol 1 (`docs/plans/2026-10-20-003-feat-2854-webhook-prefilter-mika-2680-classe-h-un-plan.md`, PR #2858, mergée le 2026-10-21T11:49:08Z) a livré AC2, AC3, AC5, AC6, AC7 et la moitié mécanique d'AC1. Ce vol-ci porte **AC4**, et lui seul. AC1 sous vrai modèle passe au vol 3 : les deux ensemble pèsent 539 lignes estimées pour une borne par vol de 429 — voir `## Taille estimée`.
>
> Les décisions D1 à D8 du vol 1 sont **acquises**. Ce plan ne les relitige pas ; il les cite quand elles décident une forme.

## Le problème, en une phrase

Une fermeture de ticket sans tâche de boucle garde son tour complet aujourd'hui,
parce que le vol 1 a délibérément retiré cette moitié de `classify` (`2f7c15f1`).
La livrer n'est pas un copier-coller de la moitié commentaire : **le handler qui
nettoie les lignes de suivi tourne avant la porte**, donc une lecture d'état
faite *dans* la porte lirait l'après-nettoyage, et la garde AC7 serait aveugle
exactement là où elle compte.

## Le point dur, vérifié dans le code et non supposé

`crates/mika-agent/src/server/handlers.rs` :

| ligne | ce qui s'y passe |
|---|---|
| 1749 | `original_github_text` est capturé — le texte GitHub d'origine, intact |
| 1994 | `upstream_close_handler::try_handle_upstream_close(&req.text, &a.db, …)` |
| 2029–2061 | le bloc de la porte : `webhook_prefilter::skip_turn(…, &LiveState { …, llm: Some(a.llm.as_ref()) }, …)` |

Et dans `crates/mika-agent/src/server/upstream_close_handler.rs` :

- `try_handle_upstream_close` s'auto-sélectionne sur `ISSUE_CLOSED_PREFIX`
  (`"[GitHub] Issue closed:"`, l. 31) et appelle `handle_issue_closed` ;
- celui-ci extrait l'URL puis appelle `cleanup_rows_for_issue_url`, qui
  `find_active_tracking_rows_by_reference_url_and_variants(base_url)` puis
  `terminal_mark_tracking_row_upstream_closed(&task.id, …)` **sur chaque ligne**.

Donc, pour un `[GitHub] Issue closed:` sur un ticket qui *était* en vol,
`has_active_self_dev_task_for_issue` appelé depuis la porte rend `false` : la
tâche a été terminale-marquée douze lignes plus haut. La garde AC7 — « un ticket
avec tâche de boucle n'est pas touché » — ne verrait jamais le cas qu'elle
existe pour voir.

Vérifié aussi, parce que la conclusion en dépend : **aucun autre handler ne
s'auto-sélectionne sur ce préfixe**. `verdict_handler`, `ci_failure_handler`,
`ci_success_handler`, `ready_label_handler`, `milestone_context_handler` et
`draft_pr_opened_handler` se sélectionnent tous sur d'autres préfixes.
`upstream_close_handler` est le seul nettoyeur. La capture doit néanmoins se
faire **au plus tôt** — juste après la l. 1749 — et non « juste avant la
l. 1994 » : un handler inséré entre les deux dans six mois ne redemanderait
l'avis de personne.

## Décisions

### § D9 — l'état d'avant est un *instantané porté par l'état*, pas un argument de plus

Le terme de trait n'est **pas** `async` et ne prend **pas** d'`issue_url` :

```rust
/// (h) la moitié fermeture : l'état du ticket tel qu'il était AVANT
/// `upstream_close_handler` (mika#2854 AC4). Pas d'`issue_url` en argument et
/// pas d'`async` : la lecture a DÉJÀ eu lieu, dans `handlers.rs`, avant le
/// nettoyage. Ce terme ne fait que la rendre — ce qui rend « lire en direct
/// sur une fermeture » non représentable.
fn closure_task_snapshot(&self) -> &ClosureSnapshot;
```

C'est le cœur du dessin : la signature est ce qui interdit la faute. Un terme
`async fn closure_has_active_task(&self, url: &str)` aurait eu exactement la
même forme que `issue_has_active_task` et se serait laissé implémenter par une
lecture live sans qu'aucun test ne rougisse — la classe de défaut que seule la
forme du type attrape.

**Refusé explicitement** : ajouter un paramètre `pre_handler: bool` à
`issue_has_active_task`. Deux populations sous un même nom ne se soustraient pas
(mika#2205) ; et un drapeau se passe mal aussi facilement qu'il se passe bien.

### § D10 — l'instantané nomme ses quatre états, dont deux absences distinctes

```rust
/// L'état du ticket lu AVANT `upstream_close_handler` (mika#2854 AC4).
///
/// Quatre variantes et non un `Option<bool>` : « pas une fermeture », « triage
/// désarmé » et « lecture en échec » sont trois causes de non-lecture qu'un
/// `None` fusionnerait, et dont deux sont des défauts de câblage tandis que la
/// troisième est le régime nominal. Classe mika#2205.
pub(crate) enum ClosureSnapshot {
    /// Le texte n'est pas une fermeture de (h) : le terme n'a pas de sujet.
    NotAClosure,
    /// Le triage est désarmé — aucun état n'est lu. Tient la promesse § D8 du
    /// vol 1 : « désarmé, aucun appel n'est fait et aucun état n'est lu ».
    Disarmed,
    /// L'état du ticket avant le nettoyage.
    Read(bool),
    /// La lecture a échoué, et l'erreur est nommée.
    Unreadable(String),
}
```

### § D11 — la clé de l'instantané est produite par `classify`, pas par une seconde orthographe

La capture ne ré-extrait rien :

```rust
pub(crate) async fn snapshot_closure_task_state(
    original: Option<&str>,
    db: &AsyncDatabase,
) -> ClosureSnapshot {
    let Some(text) = original else {
        return ClosureSnapshot::NotAClosure;
    };
    let Some(Candidate::HumanTextNoTask {
        issue_url,
        half: TriageHalf::IssueClosed,
        ..
    }) = classify(text)
    else {
        return ClosureSnapshot::NotAClosure;
    };
    if !prefilter_enabled() || !comment_triage::triage_enabled() {
        return ClosureSnapshot::Disarmed;
    }
    match db.has_active_self_dev_task_for_issue(&issue_url).await {
        Ok(v) => ClosureSnapshot::Read(v),
        Err(e) => ClosureSnapshot::Unreadable(e.to_string()),
    }
}
```

`classify` est une fonction pure sur une `&str` : l'appeler deux fois par
webhook de fermeture (une fois ici, une fois dans `skip_turn`) ne coûte rien, et
**c'est ce qui garantit par construction que la clé lue est la clé que le
décideur lira**. Le § D6 du vol 1 fait tronquer le fragment `#issuecomment-…`
une seule fois, à l'extraction ; réutiliser `classify` hérite de cette
troncature au lieu de la recopier.

L'ordre des deux gardes est voulu : la classification d'abord (gratuite, et
c'est elle qui décide si la question a un sujet), le désarmement ensuite. Un
`Disarmed` n'est donc rendu que pour un texte qui *aurait* été trié.

### § D12 — les deux absences, arrivées au décideur, sont *fail-open* et nommées à part

`decide_comment_triage` bifurque sur `half` :

```rust
let active = match half {
    TriageHalf::Comment => match state.issue_has_active_task(issue_url).await {
        Ok(v) => v,
        Err(e) => return state_unreadable("issue_has_active_task", &target, &e),
    },
    TriageHalf::IssueClosed => match state.closure_task_snapshot() {
        ClosureSnapshot::Read(v) => *v,
        ClosureSnapshot::Unreadable(e) => {
            return state_unreadable("closure_task_snapshot", &target, &anyhow::anyhow!(e.clone()));
        }
        // Armé, (h), et pourtant pas d'instantané : un défaut de câblage, et
        // jamais « ce ticket n'a pas de tâche ». Le tour a lieu, et la relâche
        // le dit sous son propre nom.
        absent => return snapshot_absent(absent, &target),
    },
};
if active {
    return Decision::Llm(Release::IssueTaskActive);
}
```

`snapshot_absent` émet un WARN nommé et rend
`Decision::Llm(Release::ClosureSnapshotAbsent)`. **Une sixième relâche et pas un
réemploi de `TriageUnreadable`** : « le triage n'a pas pu se prononcer » et
« l'instantané n'a jamais été pris » sont deux populations, et les fusionner
rendrait la prochaine mesure incapable de distinguer une panne de modèle d'un
site de capture qui a sauté (mika#2205).

L'ordre des trois portes de `decide_comment_triage` reste **exactement** celui du
vol 1 — désarmement, puis état du ticket, puis triage. La bifurcation remplace le
corps de la deuxième porte, pas sa place.

### § D13 — `TriageHalf::IssueClosed` et son orthographe de cible

```rust
pub(crate) enum TriageHalf { Comment, IssueClosed }

impl TriageHalf {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Comment => "issue_comment",
            Self::IssueClosed => "issue_closed",
        }
    }
}
```

`"issue_closed"` est déjà l'orthographe de `TriggerClass::IssueClosed` : les deux
axes nomment la même moitié du même nom, donc un `GROUP BY` croisé reste lisible.
L'arm de `classify` passe de `_ => return None` à l'aiguillage des deux classes ;
le reste de l'arm (ligne 2 du gabarit, `extract_issue_url`) est inchangé — la
ligne 2 porte l'URL pour les deux moitiés, vérifié sur les deux fixtures
(`MIKA2854_COMMENT`, `MIKA2854_ISSUE_CLOSED`).

### § D14 — aucun changement de prompt

`TRIAGE_SYSTEM_PROMPT` dit déjà, dans sa première phrase : *« un unique événement
GitHub — un commentaire de ticket, ou **la fermeture d'un ticket** »*. Le vol 1
l'a écrit pour les deux moitiés. **Zéro ligne de prompt dans ce vol**, et donc
aucune porte de calibration (mika#1190) ouverte.

### § D15 — le refus qui reste un refus

`"[GitHub] Issue closed: senara-solutions/mika#2294 — t"` — une fermeture **sans
URL en ligne 2** — reste hors population. C'est le même refus que la moitié
commentaire (§ D4 du vol 1, « aucune URL d'issue extractible ») et il garde son
entrée dans la liste de `classify(texte).is_none()` à `webhook_prefilter.rs`
l. 2668. Seul `ISSUE_CLOSED_2854` en sort.

## Plan de livraison (ordre de commit)

Chaque commit compile et laisse la suite verte.

1. **`ClosureSnapshot`, la capture, le terme de trait.**
   `webhook_prefilter.rs` : l'enum (§ D10), `snapshot_closure_task_state`
   (§ D11), `PrefilterState::closure_task_snapshot` (§ D9), le champ
   `closure_snapshot: &'a ClosureSnapshot` sur `LiveState` et son impl. Le
   `Stub` de test gagne sa fente. Rien ne lit encore le terme.
2. **La moitié fermeture entre dans la population.**
   `TriageHalf::IssueClosed` + `as_str` (§ D13), l'arm de `classify`, la
   bifurcation de `decide_comment_triage` (§ D12), `Release::ClosureSnapshotAbsent`
   + `as_str` + `snapshot_absent`. Les deux listes « hors population »
   (`webhook_prefilter.rs` l. 2474 et l. 2668) perdent `ISSUE_CLOSED_2854` et
   gardent le reste (§ D15). Tests unitaires V1–V5.
3. **Le câblage de production.**
   `handlers.rs` : la capture juste après la l. 1749, avec le commentaire qui
   nomme le point dur et la l. 1994 ; le champ dans le littéral `LiveState` de
   la l. 2043. `server/mod.rs` : `mika2854_un_issue_closed_garde_son_tour_dans_ce_vol`
   devient la version triée, et le test AC7-avant-nettoyage arrive. Tests V6–V8.
4. **La garde de lecteur unique.**
   `canonical_tokens.rs` : le recensement des lecteurs de
   `has_active_self_dev_task_for_issue` sous `src/server/`. Test V9.

## Contrat de vérification

| # | ce qui est asserté | où |
|---|---|---|
| V1 | `classify(ISSUE_CLOSED_2854)` rend `HumanTextNoTask { half: IssueClosed, issue_url: ISSUE_URL_2854 }` — et une fermeture sans URL en ligne 2 reste `None` | `webhook_prefilter.rs` |
| V2 | `TriageHalf::IssueClosed.as_str() == "issue_closed"`, ajouté au test d'orthographes figées, et la cible rendue par le décideur est `"issue_closed:<url>"` | `webhook_prefilter.rs` |
| V3 | `Read(false)` → le triage est consulté ; verdict `DECISION` → `Release::TriageEscalated`, verdict `NOTE:` → `Decision::SkipWithNote { class: CommentTriagedNote, … }` | `webhook_prefilter.rs` |
| V4 | `Read(true)` → `Release::IssueTaskActive`, **et le bouchon de triage est une fente `None`** — donc le test panique si la porte du triage est consultée, au lieu de passer pour la mauvaise raison | `webhook_prefilter.rs` |
| V5 | `Unreadable(_)` → `Release::StateUnreadable` + `webhook_state_unreadable` portant `closure_task_snapshot` ; `NotAClosure` et `Disarmed` arrivés au décideur → `Release::ClosureSnapshotAbsent`, tour complet, WARN nommé. Les trois sous des noms distincts | `webhook_prefilter.rs` |
| V6 | **AC7 avant le nettoyage** — un ticket avec tâche de boucle vivante, fermé : le tour complet a lieu, release `issue_task_active`, aucune note, aucune ligne `skipped`. C'est le test qui rougit si la capture repasse après `upstream_close_handler` ; il sème la tâche par `reference_url` (via `mika2854_seed_issue_task`), parce qu'une tâche semée sans elle serait invisible à AC7 | `server/mod.rs` (e2e) |
| V7 | Bout en bout, armé, fermeture sur ticket sans tâche : `mock.calls_made() == 1` (le triage a parlé, le tour ne s'est pas ouvert), la note est dans la base et retrouvable par `search_events("Triage webhook")`, l'audit et `webhook_turn_charged` disent la même branche | `server/mod.rs` (e2e) |
| V8 | Désarmé (`MIKA_WEBHOOK_COMMENT_TRIAGE` absent), fermeture : release `triage_disarmed`, **et aucune lecture de base n'a lieu** — l'instantané rend `Disarmed`, asserté sur la variante | `webhook_prefilter.rs` + `server/mod.rs` |
| V9 | `has_active_self_dev_task_for_issue` a exactement **deux** lecteurs sous `src/server/`, nommés par chemin et par symbole : `LiveState::issue_has_active_task` et `snapshot_closure_task_state`. Un troisième est refusé ; la disparition d'un des deux rougit aussi | `canonical_tokens.rs` |

Tout test passant par le harnais e2e porte `#[serial_test::serial]` : trois états
sont globaux au processus — `MIKA_WEBHOOK_PREFILTER`, l'`Interest` du site
`webhook_turn_charged`, et `MIKA_WEBHOOK_COMMENT_TRIAGE`.

## Fire-Disposition

**Option (a) — exception nommée en allowlist, livrée VIDE.**

Le seul détecteur structurel du vol est V9, la garde de lecteur unique de
`has_active_self_dev_task_for_issue` sous `src/server/`. Elle est livrée
**armée**, et son allowlist d'exceptions est **vide** : les deux lecteurs
légitimes ne sont pas des exceptions, ils sont la *définition* du recensement.

- **Entrées grep-visibles** : les deux lecteurs sont nommés en clair dans le
  corps de la garde — `("src/server/webhook_prefilter.rs", "issue_has_active_task")`
  et `("src/server/webhook_prefilter.rs", "snapshot_closure_task_state")` — avec
  la référence mika#2854 AC4 en commentaire.
- **Assertion auto-nettoyante** : l'assertion est une **égalité** au recensement,
  pas un `<=`. Elle rougit sur un troisième lecteur (le défaut à attraper : une
  lecture live rajoutée sur le chemin de la fermeture) **et** sur la disparition
  ou le renommage d'un des deux (l'exception devenue stale). Un seuil `<=`
  laisserait le second cas passer en silence, et c'est lui qui rend une
  allowlist éternelle.
- **Contrôle de bonne foi** : un test jumeau construit une source synthétique
  avec un troisième site et vérifie que le scan rougit — sans lui, une garde qui
  ne trouve jamais rien et une garde cassée rendent les mêmes octets.
- Le prédicat ignore les lignes de commentaire via
  `crate::source_scan::is_test_source_path` et l'exclusion de commentaires déjà
  en usage : la prose *à propos* du champ n'est pas une lecture, et le site de
  décision en porte trois paragraphes (forme mika#2050 signal S).

Zéro violation existante au moment de la livraison, donc aucune entrée
d'exception à écrire — et c'est l'état que l'égalité fige.

Les autres livrables de test (V1–V8) ne sont pas des détecteurs au sens de
mika#1574 : leur chemin de succès est « le comportement livré est celui-ci », pas
« aucune violation trouvée ». Ils atterrissent verts sur le code de ce vol.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/webhook_prefilter.rs` — `ClosureSnapshot`, `snapshot_closure_task_state`, le terme de trait, `TriageHalf::IssueClosed`, l'arm de `classify`, la bifurcation de `decide_comment_triage`, `Release::ClosureSnapshotAbsent` + `snapshot_absent` | 106 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` (tests V1–V5, V8 : fente du `Stub`, bascule des deux listes « hors population », orthographes figées) | 112 |
| `crates/mika-agent/src/server/handlers.rs` — site de capture après la l. 1749 + champ du littéral `LiveState` + commentaire du point dur | 16 |
| `crates/mika-agent/src/server/mod.rs` (tests e2e V6, V7, V8 : bascule de `mika2854_un_issue_closed_garde_son_tour_dans_ce_vol`, AC7-avant-nettoyage) | 85 |
| `crates/mika-agent/src/canonical_tokens.rs` (V9 : garde de lecteur unique + contrôle de bonne foi) | 28 |

Total estimé : 347 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et pourquoi AC1 n'y est pas.** AC4 seul pèse 347, sous
la borne. AC1 sous vrai modèle pèse environ **192** lignes de plus — le corpus
réel en `const` nommées avec leur provenance (≈ 100), l'éval `#[ignore]` + son
analyse de taux (≈ 82), et la couture de prompt dégénéré pour mesurer le rouge
(≈ 10) — soit **539** au total, au-dessus de 429. La borne mord, et la règle
mika#2693 prescrit des **vols**, pas des phases : AC1 part donc au vol 3, qui
est mergeable seul (il n'écrit aucune ligne de moteur — un corpus, un test
désarmé par défaut, une couture) et porte son propre plancher.

Tracked in: senara-solutions/mika#2854 — vol 3 : AC1 sous vrai modèle. Le corpus réel est déjà identifié et vérifié par `gh` (voir `## Corpus du vol 3, déjà établi` ci-dessous) ; le vol 3 le fige en `const` nommées, le joue contre le vrai fournisseur derrière `#[ignore]` + `MIKA_EVAL_TRIAGE_PROVIDERS`, et mesure le rouge avec un prompt dégénéré qui note tout.

## Corpus du vol 3, déjà établi

Noté ici pour que le vol 3 n'ait pas à le redécouvrir, et parce qu'une
divergence avec le corps du ticket doit être écrite une fois.

**Les items porteurs d'une décision** (chacun doit escalader) :

| # | source | instant | ce qu'il décide |
|---|---|---|---|
| D1 | #2784, commentaire de `samidarko` | 2026-10-20T15:14:37Z | tranche les questions 1 et 2 ; porte littéralement « je valide » |
| D2 | #2680, commentaire de `samidarko` | 2026-10-20T15:16:54Z | classe (h) : réponse (c), « retenue et groomable » |
| D3 | #2680, commentaire de `samidarko` | 2026-10-20T15:20:54Z | classe (g′) : réponse (a), ouvrir le vol d'instrument |
| D4 | #2814, **fermeture** | 2026-10-21T10:26:01Z | la moitié AC4 du corpus |
| D5 | #1381, **fermeture** | 2026-10-21T10:26:48Z | la moitié AC4 du corpus |

**Correction de provenance, vérifiée.** Le corps du ticket annonce « les
décisions du 2026-10-20 sur #2784, #2680, #2814 et #1381 ». Mesuré par
`gh issue view` : #2784 et #2680 portent bien des commentaires d'opérateur ce
jour-là (trois en tout), mais **#2814 n'a aucun commentaire de `samidarko`** et
le dernier de #1381 date du 2026-08-22. Sur ces deux tickets, la décision du
2026-10-20 est leur **fermeture**, horodatée au 2026-10-21T10:26Z. C'est une
chance plutôt qu'un manque : elle donne au corpus deux vrais `issues` `closed`,
la moitié même que ce vol-ci livre.

**Le contrôle positif sous vrai modèle** (chacun doit noter) : les trois rapports
machine de `mika-platform-dev` — #2680 2026-10-19T05:30:07Z (« Auto-pull :
promotion refusée »), #2680 2026-10-19T01:03:44Z (« Troisième dispatch sans rien
à écrire — n=3 »), #2814 2026-10-20T06:10:12Z (le doublon verbatim de 06:10:06Z,
qui ne dit rien de neuf).

**Un item limite, mesuré et non asserté** : #2680 2026-10-19T01:10:07Z,
« Auto-pull : re-drive abandonné ». Écrit par une machine, mais **impératif dans
sa forme** (« groome #2680 jusqu'à… », « retire son label de tenue »). Sous la
posture fail-open, l'escalade est la direction sûre ; asserter l'un ou l'autre
figerait l'arbitrage que la mesure existe pour informer. Le vol 3 le **rapporte**
en taux, hors assertion.

## Definition of Done

1. `classify` rend `Candidate::HumanTextNoTask { half: TriageHalf::IssueClosed, … }` sur un `[GitHub] Issue closed:` porteur d'une URL en ligne 2.
2. Une fermeture sans URL extractible en ligne 2 reste hors population.
3. `TriageHalf::IssueClosed.as_str()` vaut `"issue_closed"`, et le test d'orthographes figées le couvre.
4. `ClosureSnapshot` porte ses quatre variantes, et `closure_task_snapshot` est un terme de trait **synchrone et sans argument**.
5. `snapshot_closure_task_state` produit sa clé **par `classify`**, et non par une seconde extraction.
6. `handlers.rs` capture l'instantané **avant** la l. 1994 (`try_handle_upstream_close`), et le commentaire du site nomme pourquoi.
7. Le décideur bifurque sur `half` ; l'ordre des trois portes de `decide_comment_triage` est inchangé.
8. `Unreadable` → `Release::StateUnreadable` ; `NotAClosure`/`Disarmed` arrivés au décideur → `Release::ClosureSnapshotAbsent`. Les deux sont *fail-open*, aucun ne rend une note.
9. Désarmé, aucune lecture de base n'a lieu sur le chemin de la fermeture.
10. V1 à V9 sont verts ; `cargo test -p mika-agent`, `cargo clippy` et `scripts/verify-pipeline.sh origin/main` passent.
11. Aucune ligne de `TRIAGE_SYSTEM_PROMPT` n'est modifiée.
12. `MIKA_WEBHOOK_COMMENT_TRIAGE` reste à `off` par défaut.

## Plancher de ce vol

- **`MIKA_WEBHOOK_COMMENT_TRIAGE` reste `off` par défaut.** Ce vol livre la
  moitié fermeture *sous* l'interrupteur, pas le basculement de l'interrupteur.
- **`MIKA_WEBHOOK_PREFILTER` garde son rôle de désarmement global**, et le
  désarme aussi.
- **Désarmé, rien n'est lu** — ni appel de modèle, ni lecture de base, sur les
  deux moitiés de (h).
- **Aucun ordre d'opérateur n'est dégradé en note.** Une erreur, un délai, un
  verdict illisible, un fournisseur absent, un instantané absent : tous rendent
  le tour complet.
- **Le tour léger n'a aucun outil mutationnel** (`tools: None`, § D3 du vol 1),
  inchangé.
- **Versionnage pré-1.0** : `TriageHalf` et `Release` gagnent des variantes
  (`pub(crate)`, donc sans surface publique). Aucune étape de migration manuelle.

## Hors périmètre

- **AC1 sous vrai modèle** → vol 3, nommé par la ligne `Tracked in:` ci-dessus.
- **Le basculement du défaut de `MIKA_WEBHOOK_COMMENT_TRIAGE` à `on`** — c'est
  une décision d'opérateur qui se prend sur la mesure du vol 3, pas avant elle.
- **(g′)** reste sur mika#2680.
- **Les classes (a) à (g)** restent telles qu'elles sont.
- **Le prompt du triage** — § D14.

## Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#2854 (section
« Critères d'acceptation (ébauche, à affiner au groom) »).

- **AC1 : corpus de contrôle négatif.** Un jeu de commentaires réels porteurs d'une décision : lever `operator-gated`, « je valide », fermer, lancer un vol, trancher Q1/Q2. On les tire des tickets de la file, par exemple les décisions du 2026-10-20 sur #2784, #2680, #2814 et #1381. **Chacun escalade vers le tour complet.** Le rouge est mesuré avec un triage qui note tout.
- **AC2 : contrôle positif.** Un jeu de commentaires sans décision (accusé, note, lien) donne une note, sans tour complet.
- **AC3 : fail-open.** Une erreur, un timeout ou une sortie illisible du triage donne le tour complet. C'est asserté.
- **AC4** : `issues` `closed` sans tâche suit la même règle que `issue_comment`.
- **AC5** : la ligne d'audit et `skipped_by_class` portent la classe (h) et la branche prise.
- **AC6** : le tour léger ne reçoit aucun outil mutationnel. C'est mesuré sur le tableau servi.
- **AC7** : un ticket **avec** tâche de boucle n'est pas touché, et passe par le chemin d'aujourd'hui.

### Couverture par vol

| AC | vol | état |
|---|---|---|
| AC1 | vol 1 (moitié mécanique, V2/V6 sur bouchon) + **vol 3** (vrai modèle) | partiel |
| AC2 | vol 1 | fermé |
| AC3 | vol 1 | fermé |
| AC4 | **vol 2 — ce plan** | ce vol |
| AC5 | vol 1, **étendu par ce vol** à la moitié fermeture (V2, V7) | ce vol |
| AC6 | vol 1 | fermé |
| AC7 | vol 1, **rendu non aveugle par ce vol** sur la moitié fermeture (V6) | ce vol |

AC5 et AC7 figurent en « ce vol » sans rouvrir le vol 1 : leur mécanique est
livrée et inchangée ; ce vol étend la population sur laquelle elles portent.
C'est précisément V6 qui fait d'AC7 une garde qui voit, là où un AC7 lu après
`upstream_close_handler` serait resté vert en ne mesurant rien.

Refs mika#2680, mika#2675, mika#2854, mika#2205, mika#1574, mika#2693
