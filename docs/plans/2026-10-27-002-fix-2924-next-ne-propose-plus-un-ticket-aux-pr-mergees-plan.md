# mika#2924 vol 2/2 — « Next » ne propose plus d'implémenter un ticket que des PR mergées citent déjà

**Ticket :** senara-solutions/mika#2924 (AC2) — périmètre détaillé dans
senara-solutions/mika#2930
**Milestone :** 33 — Substrat de boucle
**Labels :** `bug`, `p2-normal`, `dispatch:loop`, `loop-substrate`
**Le merge revient à Vincent** (`loop-substrate`).

## Contexte

Le **vol 1/2** est livré (PR #2931, commit `206e73e67`, mergée le 2026-10-27) et
ferme AC1 + AC3 de mika#2924 : la PR qui **ferme** un ticket se lit depuis le
ticket, par `closedByPullRequestsReferences`, sur l'appel `gh issue list` **déjà
émis** — zéro appel `gh` ajouté, et les 87 lignes `closed (no linked PR)` du
rapport du sink tombent à 14.

Ce que le vol 1 a posé et que ce vol réutilise tel quel :

- `PrLinkSource` (`types.rs:99`) — deux variantes, `MilestonePrList` et
  `IssueClosingRef`, avec le contrat « `None` dit *blob d'un binaire antérieur*,
  jamais *liste du milestone* » (motif mika#2340).
- `SubIssue.pr_url` — parce qu'une référence côté issue peut porter une PR d'un
  autre dépôt : la recomposition `{repo}/pull/{n}` est refusée.

**AC2 reste ouvert, et aucun champ de référence de clôture ne peut le fermer.**
Le défaut qui reste, mot pour mot dans le rapport du sink pour le milestone 33 :

> `- Next: #2698 — #2698 has a plan but no PR — implementation is the next step`

alors que les PR #2704 et #2706 (mika#2698, vol 1) sont mergées depuis le
2026-10-16. Le manager propose donc d'implémenter un ticket dont le travail est
déjà en production.

La cause est structurelle, et le vol 1 l'a mesurée : `closedByPullRequestsReferences`
rend `[]` pour un ticket **ouvert**, et les PR #2704/#2706 ne déclarent aucun
`closingIssuesReferences`. Le champ que le vol 1 exploite est donc **muet sur
cette population**. Autre source de données, autre surface, autre coût — donc un
second vol.

## Ce que la mesure a dit, avant d'écrire une ligne

Mesuré le 2026-10-27 sur `senara-solutions/mika`, dépôt réel, avant toute
décision de code. Chaque ligne est reproductible par la commande donnée.

| question mesurée | réponse | commande |
|---|---|---|
| le `timeline` REST répond-il pour #2698 ? | **oui** — 6 PR mergées citées : 2704, 2706, 2712, 2892, 2929, 2931 | `gh api "repos/senara-solutions/mika/issues/2698/timeline?per_page=100" --jq '[.[] \| select(.event=="cross-referenced") \| select(.source.issue.pull_request != null) \| {n: .source.issue.number, merged: .source.issue.pull_request.merged_at}]'` |
| la page par défaut suffit-elle ? | **non** — #2698 porte **36** événements ; sans `per_page` l'API en rend **30** | même appel, avec et sans `?per_page=100` → `--jq 'length'` : 36 contre 30 |
| une page de 100 peut-elle être pleine ? | **oui** sur la population visée — #1727 porte déjà 50 événements, et il est le second ticket ouvert le plus ancien du vivier | `gh api ".../issues/1727/timeline?per_page=100" --jq 'length'` → 50 |
| le filtre `merged_at != null` sert-il à quelque chose ? | **oui** — #1727 cite 10 PR dont **2 non mergées** (#1730, #1882), #2934 cite **une seule** PR et elle n'est pas mergée (#2936) | même appel sur 1727 et 2934 |
| combien de tickets ouverts sur le milestone 33 ? | **10** (2934, 2930, 2924, 2922, 2854, 2699, 2698, 2680, 1727, 1721) | `gh issue list --repo senara-solutions/mika --milestone "Substrat de boucle" --state open --limit 100` |

### La mesure qui change le libellé, et qu'il ne faut pas re-découvrir en revue

`cross-referenced` est **plus large** que « PR qui livre ce ticket ». Dans la
liste de #2698 ci-dessus, **#2929 et #2931 ne livrent pas #2698** : ce sont les
PR de mika#2923 et de mika#2924 vol 1, qui ont cité #2698 depuis leur corps ou
un commentaire. Le même élargissement vaut pour #1727, où 8 PR mergées citent un
ticket qui n'est pas livré.

Conséquence directe sur ce que le rapport peut **dire** : la timeline atteste
*« des PR mergées citent ce ticket »*, elle n'atteste pas *« ce ticket est
livré »*. Le libellé retenu en R3 dit donc exactement la première chose. C'est
l'extension du refus déjà tranché au vol 1 (« ne pas déduire un numéro de vol ») :
on ne déduit pas davantage « livré ».

Affiner par lecture du corps de chaque PR citée (chercher un `Closes`) coûterait
un appel `gh` **par PR citée** — 6 pour #2698, 10 pour #1727 — pour une
information que la ligne « Next » n'a pas besoin d'affirmer. Refusé, et nommé
en Non-Requirements.

### Le coût `gh` par cycle, avant et après

Le `Reader` est le seul site d'appels `gh` du cycle de lecture. Avant ce vol :

| lecture | appels |
|---|---|
| 1. métadonnées du milestone (`gh api /repos/…/milestones/N`) | 1 |
| 2. énumération des sous-tickets (`gh issue list --milestone`) | 1 |
| 3. PR du milestone (`gh pr list --search milestone:N`) | 1 |
| 4. bassin `ready` (`gh issue list --label ready`) | 1 |
| 5. candidats hors milestone (`gh issue list`) | 1 |
| 6. taille annoncée des plans, bornée par `PLAN_SIZE_READ_MAX` (= `3 × POOL_MAX` = 12) | ≤ 12 |
| **total avant** | **≤ 17** |

Après ce vol : **≤ 18**, soit **+1 appel au plus, et +5,9 %**.

Le bornage n'est pas une constante arbitraire posée sur une population : il
tombe de la forme de ce qui consomme la sonde. **La ligne « Next » ne nomme
qu'un seul ticket.** Donc un seul candidat a besoin d'être sondé — celui que le
palier 2 nommerait — et `TIMELINE_PROBE_MAX = 1` exprime cette propriété plutôt
qu'un budget. Mieux : quand le palier 1 l'emporte (un ticket du milestone porte
une PR ouverte), **zéro** appel part, parce que le palier 2 ne décide pas ce
cycle-là.

## Ce qui est refusé (bearing, non négociable)

Trois refus tranchés au vol 1 ou par la mesure ci-dessus, à ne pas rouvrir sans
leur prémisse.

1. **Ne pas remplacer la liste des PR du milestone.** Elle reste la source
   unique de `pr_number`, `pr_state` et `ci_state` : `is_in_flight()`, le
   **palier 1** de `pick_next_action`, `is_hard_blocked()` et surtout
   `detect_silent_progress` (qui exige `state == Open && pr_state ==
   Some("merged")`) en dépendent. La sonde de ce vol **comble un trou**, elle ne
   remplace rien, et elle n'écrit dans **aucun** des champs ci-dessus.
   Y écrire ferait basculer #2698 au palier 1 et le rapport dirait « has an open
   PR (#2704) » — faux d'une autre manière.
2. **Ne pas déduire un numéro de vol.** Le corps de mika#2924 illustre AC2 par
   « vol N livré, PR #… ». Aucun champ `gh` n'atteste un numéro de vol — ni le
   titre de la PR (texte libre), ni un label, ni la timeline. Divergence assumée
   avec la lettre du ticket parent, et étendue par la mesure : on ne dit pas non
   plus « livré ».
3. **Ne pas écarter un ticket du « Next » parce que des PR le citent.** Un
   ticket ouvert dont des PR sont mergées est précisément la forme d'un ticket
   multi-vols dont le vol suivant reste à faire — **mika#2924 lui-même** en est
   l'exemple vivant aujourd'hui. L'écarter le rendrait invisible, ce qui est le
   défaut inverse. Il reste le « Next » ; c'est la **raison** qui cesse de
   mentir. C'est la lecture littérale de l'AC2 de mika#2924 (« n'est plus
   proposé […] **comme “no PR”** ») et de l'AC1 de mika#2930 (« la ligne “Next”
   **nomme les PR mergées** au lieu de réclamer une implémentation »).

## La règle choisie, et celle qui est rejetée

### Rejetée — sonder tous les candidats du palier 2

Un appel par candidat porteur de plan, borné par une constante de type
`PLAN_SIZE_READ_MAX`. C'est le patron du vol 3 de mika#2699, et il est légitime
**là-bas** parce que le bassin juge *plusieurs* candidats et que chacun a besoin
de sa taille. Ici il paierait jusqu'à 10 appels par cycle pour une ligne qui
nomme un ticket — 9 réponses jetées par construction.

### Retenue — un seul candidat sondé, celui que le palier 2 nommerait

La désignation se fait **au même endroit** qu'au jugement, par la fonction que
`pick_next_action` utilise elle-même pour choisir. C'est le patron déjà en force
dans ce fichier pour la lecture 6 (`read_plan_sizes`), dont le commentaire porte
la raison mot pour mot :

> Qui est candidat se décide **au même endroit** qu'au jugement […]. Rejouer le
> filtre ici rendrait les deux réponses identiques aujourd'hui et muettes le
> jour où l'une des deux change.

C'est aussi ce qui tient la singularité du prédicat **par la forme** : le
`Reader` ne reçoit qu'un `Option<u64>` et n'a aucun moyen de réimplémenter le
filtre sans le réécrire entièrement. Le précédent immédiat dans le même fichier
est tenu de la même manière, par la même propriété, et sans scan de source — ce
vol ne change pas ce régime (voir Non-Requirements, point 5).

## Requirements

### R1 — `crates/mika-agent/src/milestone_manager/types.rs` : le vocabulaire

Un enum, quatre variantes, un champ. **Inerte** à ce commit : aucun producteur,
aucun consommateur.

```rust
/// Ce que la timeline REST d'un ticket **ouvert** a dit de ses PR (mika#2924,
/// vol 2).
///
/// Quatre variantes et non deux : « aucune PR mergée ne cite ce ticket » et
/// « la lecture n'a pas conclu » ont des remèdes opposés, et les confondre est
/// la classe mika#2205. Les deux causes d'indétermination sont distinctes pour
/// la même raison — une page pleine se corrige en paginant, un échec de lecture
/// se corrige à la forge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergedPrReading {
    /// Des PR mergées citent ce ticket, triées par `merged_at` **décroissant**.
    /// Dit « citent », jamais « livrent » : `cross-referenced` est plus large
    /// que la livraison (mesure du 2026-10-27, #2698 cite #2929 et #2931 qui
    /// ne le livrent pas).
    Citing(Vec<u64>),
    /// La timeline a été lue **en entier** et aucune PR mergée ne cite ce
    /// ticket.
    NoneCiting,
    /// La page est pleine : la timeline est possiblement tronquée, donc
    /// l'absence de PR n'est **pas** attestée (mika#2930 AC2).
    IndeterminatePageFull,
    /// L'appel `gh` a échoué. Un échec ne refuse pas le cycle et ne se tait pas.
    IndeterminateFetchFailed,
}
```

Sur `SubIssue`, un champ et son contrat :

```rust
/// Ce que la sonde de timeline a dit des PR de ce ticket (mika#2924, vol 2).
///
/// `None` signifie **la sonde n'a pas été émise** — palier 1 vainqueur, ticket
/// non désigné, ou blob d'un binaire antérieur au vol (motif mika#2340) — et
/// jamais « aucune PR ». C'est pourquoi `None` rend le libellé **historique**
/// en R3 : un binaire antérieur garde son texte, il n'hérite pas d'une
/// incertitude qu'il n'a pas mesurée.
#[serde(default)]
pub merged_prs: Option<MergedPrReading>,
```

`#[serde(default)]` parce que `MilestoneState` est sérialisé et relu : le
contrat « un blob antérieur se relit » est déjà tenu et testé pour `pr_url` /
`pr_link_source` (`types.rs:847`, `types.rs:893`), et ce champ s'y range.

**Tests (inline `mod tests` de `types.rs`) :**

- un blob sans `merged_prs` se relit, et le champ rend `None` — extension du
  test d'arriéré existant ;
- aller-retour serde des quatre variantes, et la forme JSON de `Citing` porte
  bien un tableau de numéros ;
- un `SubIssue` neuf sérialise `"merged_prs":null`, aux côtés des deux
  assertions `pr_url` / `pr_link_source` déjà présentes en `types.rs:923`.

### R2 — `crates/mika-agent/src/milestone_manager/assessor.rs` : la désignation, site unique

Extraire du corps de `pick_next_action` les deux sélections de palier, sans
changer leur prédicat ni leur tri :

```rust
/// Le sous-ticket que le **palier 1** nommerait, ou `None`.
fn tier1_pick(state: &MilestoneState) -> Option<&SubIssue>;

/// Le sous-ticket que le **palier 2** nommerait, ou `None`.
fn tier2_pick(state: &MilestoneState) -> Option<&SubIssue>;
```

`pick_next_action` les appelle à la place de ses deux blocs de `filter` +
`sort_by_key`. Les sept tests existants de `pick_next_action` restent inchangés,
ce qui est la preuve que le prédicat et l'ordre n'ont pas bougé.

Puis **la** fonction que le `Reader` consulte :

```rust
/// Le ticket dont la sonde de timeline décidera du libellé « Next » de ce
/// cycle — au plus un, et `None` quand le palier 2 ne décide pas (mika#2924,
/// vol 2).
///
/// Désigné **ici**, au même endroit qu'au jugement, et pour la même raison que
/// la lecture 6 du `Reader` : rejouer le filtre côté lecture rendrait les deux
/// réponses identiques aujourd'hui et muettes le jour où l'une des deux change.
///
/// `None` quand le palier 1 l'emporte — un milestone en vol ne paie **aucun**
/// appel de plus.
pub(super) fn timeline_probe_target(state: &MilestoneState) -> Option<u64> {
    if tier1_pick(state).is_some() {
        return None;
    }
    tier2_pick(state).map(|s| s.number)
}
```

### R3 — `assessor.rs` : le palier 2 lit la sonde, et le mot « no PR » disparaît pour cette population

Le palier 2 de `pick_next_action` dérive son `rationale` de
`s.merged_prs`. Le rapport est rendu en anglais (`reporter.rs:203`), donc les
libellés le sont aussi.

| `merged_prs` | `rationale` |
|---|---|
| `Some(Citing(prs))`, non vide | `#{n} has a plan and merged PRs already cite it ({liste}) — check what remains before implementing` |
| `Some(IndeterminatePageFull)` | `#{n} has a plan; its timeline is truncated ({TIMELINE_PAGE_SIZE} events read) — PR state indeterminate, verify by hand before implementing` |
| `Some(IndeterminateFetchFailed)` | `#{n} has a plan; its timeline could not be read — PR state indeterminate, verify by hand before implementing` |
| `Some(NoneCiting)` **ou** `None` | **inchangé** : `#{n} has a plan but no PR — implementation is the next step` |
| `Some(Citing(vide))` | traité comme `NoneCiting` — la variante vide n'est pas produite par R4, le `match` ne la laisse pas décider |

`liste` est bornée : les **trois** PR les plus récemment mergées, `PR #a, #b,
#c`, suivies de ` (+N more)` au-delà. Borné parce que la mesure le demande —
#1727 cite 8 PR mergées, et une ligne « Next » qui en nomme huit n'est plus lue.

**La dernière ligne du tableau est une décision, pas un oubli.** `NoneCiting` et
`None` rendent le **même** texte parce que la conclusion est la même ; ce qui
doit rester distinguable, c'est l'**instrument**, et c'est la sonde de R5 qui
l'est. Rendre un texte d'incertitude sur `None` ferait dire au rapport d'un
binaire antérieur une chose que ce binaire n'a pas mesurée.

**Tests (inline `mod tests` de `assessor.rs`) :**

- `mika2924_next_nomme_les_pr_mergees_au_lieu_de_reclamer_une_implementation` —
  un candidat de palier 2 portant `Citing(vec![2931, 2929, 2892, 2712, 2706, 2704])`
  (les six numéros **mesurés** sur #2698) : le `rationale` nomme les trois plus
  récentes, porte `(+3 more)`, et **ne contient pas** `no PR`. Contrôle négatif
  **dans le même test** : un second candidat portant `NoneCiting` garde le texte
  historique mot pour mot. Les deux assertions dans le même test, par AC1 de
  mika#2930 ;
- `mika2924_une_page_pleine_ne_se_lit_jamais_comme_une_absence_de_pr` —
  `IndeterminatePageFull` rend le libellé d'indétermination et **ne contient
  pas** `no PR` ; `IndeterminateFetchFailed` rend l'autre libellé, et les deux
  diffèrent ;
- `mika2924_un_blob_sans_sonde_garde_le_libelle_historique` — `merged_prs:
  None` rend le texte d'avant, à la lettre (motif mika#2340) ;
- `mika2924_la_cible_de_sonde_est_vide_quand_le_palier_1_lemporte` — avec un
  candidat de palier 1 présent, `timeline_probe_target` rend `None` ; sans lui,
  il rend le numéro que le palier 2 nommerait, **et c'est le même numéro** que
  celui du `Recommendation` rendu par `pick_next_action` sur le même état. Cette
  dernière égalité est ce qui attache la désignation au jugement.

### R4 — `crates/mika-agent/src/milestone_manager/reader.rs` : la lecture 7, bornée

Deux constantes, à côté de `OUTSIDE_READ_LIMIT` :

```rust
/// Taille de page demandée au `timeline` REST (mika#2924, vol 2).
///
/// La page par défaut est de **30** et #2698 en portait **36** le 2026-10-27 :
/// le ticket le plus actif du vivier est précisément celui sur lequel la
/// réponse serait silencieusement fausse sans ce paramètre.
pub const TIMELINE_PAGE_SIZE: usize = 100;

/// Nombre de tickets dont la timeline est lue par cycle (mika#2924, vol 2).
///
/// Vaut 1 parce que la ligne « Next » ne nomme qu'**un** ticket : la borne
/// exprime la forme de ce qui consomme la sonde, pas un budget posé sur une
/// population. Coût mesuré : ≤ 17 appels `gh` par cycle avant, ≤ 18 après.
pub const TIMELINE_PROBE_MAX: usize = 1;
```

Dans `read_with_runner`, après la lecture 6 :

```rust
// 7. L'état des PR du candidat que le palier 2 nommerait (mika#2924, vol 2).
//    Au plus `TIMELINE_PROBE_MAX` lecture, et zéro quand le palier 1
//    l'emporte. La cible est désignée par `assessor::timeline_probe_target`
//    sur l'état qu'on vient de composer — même site qu'au jugement, pour la
//    même raison que la lecture 6.
```

- cible : `super::assessor::timeline_probe_target(&state)`, puis `.take(TIMELINE_PROBE_MAX)` sur
  l'itérateur de cibles, pour que la borne soit lue au site de l'appel et pas
  seulement dans le type ;
- appel : `gh api /repos/{repo}/issues/{n}/timeline?per_page={TIMELINE_PAGE_SIZE}`.
  `gh api` en **lecture** est déjà le geste des lectures 1 et 6 ; aucun verbe
  mutant, donc rien à ajouter à l'allowlist de `no_dispatch_test.rs`
  (vérifié : sa table interdit `-X PATCH|POST|DELETE|PUT`, pas le GET) ;
- `Err(_)` → `IndeterminateFetchFailed`. Rien n'est propagé en `Err` : un
  échec sur **un** candidat ne prive pas l'opérateur du rapport sur tous les
  autres — même politique que `read_one_plan_size` ;
- le résultat est posé sur le `SubIssue` de ce numéro dans `state.sub_issues`.

Et la fonction **pure** qui lit la sortie :

```rust
/// Réduit une sortie `gh api …/timeline` aux PR **mergées** qui citent le
/// ticket — fonction pure, pendant de `compose_ready_pool` pour la lecture 7
/// (mika#2924, vol 2).
///
/// Trois filtres, chacun mesuré le 2026-10-27 :
/// - `event == "cross-referenced"` ;
/// - `source.issue.pull_request != null` — une citation peut venir d'une issue
///   (#2934 en porte) ;
/// - `source.issue.pull_request.merged_at != null` — #1727 cite 10 PR dont 2
///   non mergées, #2934 n'en cite qu'une et elle n'est pas mergée.
///
/// **Une page pleine rend `IndeterminatePageFull`, même quand aucune PR n'a été
/// trouvée** : à `TIMELINE_PAGE_SIZE` événements lus, l'absence n'est pas
/// attestée (mika#2930 AC2). Une page pleine **avec** des PR trouvées rend
/// `Citing` — la réponse est déjà sûre, et la troncature ne peut que l'allonger.
pub fn compose_merged_prs(timeline_json: &str) -> Result<MergedPrReading>;
```

Tri par `merged_at` décroissant, dédoublonnage par numéro de PR.

**Tests (inline `mod tests` de `reader.rs`) :**

- `mika2924_la_timeline_rend_les_pr_mergees_qui_citent_le_ticket` — fixture
  issue de la sortie **réellement capturée** le 2026-10-27 pour #2698, réduite
  aux trois chemins JSON que le compositeur lit (la commande `--jq` de capture
  est en en-tête de la fixture ; les quarante champs par événement que l'API
  rend n'attesteraient rien de plus et rendraient le test illisible). Attendu :
  `Citing([2931, 2929, 2892, 2712, 2706, 2704])`. Contrôle négatif **dans le
  même test** : la fixture de #2934 — un seul `cross-referenced`, vers la PR
  #2936 non mergée — rend `NoneCiting` ;
- `mika2924_une_page_pleine_est_une_troncature_et_jamais_une_absence` — une page
  de `TIMELINE_PAGE_SIZE` événements sans aucune PR mergée rend
  `IndeterminatePageFull` ; la **même** page avec une PR mergée rend `Citing`.
  Les deux branches dans le même test, parce que c'est leur différence qui est
  la propriété ;
- `mika2924_un_evenement_qui_nest_pas_une_pr_ne_compte_pas` — un
  `cross-referenced` vers une issue (`pull_request` absent) et un événement
  d'un autre type sont ignorés ;
- `mika2924_la_sonde_est_bornee_a_une_lecture_par_cycle` — le runner
  enregistreur compte ses invocations : **exactement une** cible de sonde, et
  l'URL porte `per_page=100`. Deuxième moitié dans le même test : avec un
  candidat de palier 1 dans l'état, **aucun** appel `timeline` ne part. C'est
  le contrôle négatif du bornage — sans lui, « borné à 1 » et « émis
  systématiquement » rendent le même compte sur un état où le palier 2 décide.

### R5 — `reader.rs` : la sonde d'observabilité

Une ligne INFO par cycle, `target: "mika::milestone_manager"`, sur **toutes** les
branches y compris celle où rien n'est émis :

`event = "manager_timeline_probe"`, champs :

| champ | valeur |
|---|---|
| `outcome` | `citing` \| `none_citing` \| `indeterminate_page_full` \| `indeterminate_fetch_failed` \| `not_emitted` |
| `candidate` | le numéro sondé — **absent**, jamais `0`, quand `outcome = not_emitted` (motif mika#2331) |
| `events_read` | nombre d'événements lus — **absent** quand rien n'a été lu |
| `citing_prs` | nombre de PR mergées trouvées |

Sans cette ligne, « la sonde n'a pas trouvé de PR » et « la sonde n'est pas
partie » rendraient les mêmes octets (classe mika#2205) — et c'est exactement la
paire que R3 fait converger sur un même texte. Son **absence** signifie un
binaire antérieur au vol (motif mika#2340), jamais une absence de sonde.

### R6 — `mod.rs` : la réexportation

`MergedPrReading`, `TIMELINE_PAGE_SIZE`, `TIMELINE_PROBE_MAX` et
`compose_merged_prs` rejoignent les réexports existants du module
(`mod.rs:305-327`), au même endroit et dans le même ordre alphabétique que
`PrLinkSource` et `OUTSIDE_READ_LIMIT`.

## Non-Requirements (hors de ce plan, délibérément)

1. **`reporter::write_completed` n'est pas touché.** Le rendu des tickets
   **fermés** est livré par le vol 1 et il est juste. Ce vol ne touche que la
   ligne « Next », c'est-à-dire `Recommendation.rationale`, que le reporter rend
   déjà tel quel (`reporter.rs:203-214`) — **aucune ligne de `reporter.rs`**.
2. **`detect_silent_progress` n'est pas touché.** Il exige `pr_state ==
   Some("merged")`, qui vient de la liste des PR du milestone. L'alimenter
   depuis la timeline ferait tirer l'alerte sur tout ticket multi-vols — un
   défaut nouveau, et d'un autre ticket.
3. **Les paliers 1, 3 et 4 ne changent pas**, ni leur prédicat, ni leur ordre,
   ni leur texte. Les sept tests existants de `pick_next_action` sont la preuve.
4. **Aucune lecture du corps des PR citées.** Distinguer « cite » de « livre »
   demanderait un appel `gh` par PR citée (6 pour #2698, 10 pour #1727) pour une
   information que la ligne « Next » n'affirme pas. C'est le prix du libellé
   retenu en R3, et il est assumé.
5. **Aucun scan de source sur la singularité du prédicat de palier.** Le
   précédent immédiat dans le même fichier — la lecture 6, dont la désignation
   passe par `assessor::milestone_pool_candidates` — tient la même propriété par
   la même forme et sans scan. Ce vol ne change pas ce régime pour son propre
   compte. Si la classe doit être armée, elle doit l'être pour **les deux**
   désignations à la fois, ce qui est un ticket d'hygiène du module et non une
   moitié de celui-ci.
6. **Aucune pagination au-delà de la première page.** Une seconde page coûterait
   un appel de plus et, sur la population mesurée, le maximum observé est de 50
   événements contre une page de 100. L'indétermination explicite est la réponse
   retenue : elle est honnête, elle est attestée, et elle coûte zéro appel.

## Verification Contract

```bash
cargo test -p mika-agent milestone_manager::                 # unités du module
cargo test -p mika-agent mika2924                            # les tests de ce vol
cargo test -p mika-agent --test eval manager                  # eval loop-résistance
cargo test -p mika-agent milestone_manager::no_dispatch       # la garde d'écriture
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/verify-pipeline.sh origin/main
```

## Definition of Done

- Les six requirements livrés, les trois commits de l'ordre ci-dessous poussés
  sur `fix/2924/manager-le-rapport-dit-no-linked-pr-sur`.
- Suite verte, clippy sans avertissement, `fmt` propre, `verify-pipeline.sh`
  vert, `Check` requis vert sur la PR.
- Le corps de la PR porte le coût `gh` avant/après (≤ 17 → ≤ 18) et la sonde
  post-déploiement ci-dessous.
- La PR porte `Closes #2924` **et** `Closes #2930` : ce vol ferme le dernier AC
  du parent et la totalité du ticket de périmètre.
- Le merge revient à Vincent.

## Acceptance criteria

**De senara-solutions/mika#2924 — le seul AC qui reste ouvert, verbatim :**

- **AC2.** Un ticket ouvert avec des PR mergées (la forme de #2698) n'est plus
  proposé en « Next » comme « no PR ».

**De senara-solutions/mika#2930, le ticket de périmètre de ce vol, verbatim :**

- **AC1.** Un ticket ouvert porteur de PR mergées (la forme de #2698) n'est plus
  proposé en « Next » comme « no PR ». Sur une sortie `timeline` capturée pour
  #2698, la ligne « Next » nomme les PR mergées au lieu de réclamer une
  implémentation. Le contrôle positif (#2698) et le contrôle négatif (un ticket
  ouvert réellement sans PR) sont dans le **même** test.
- **AC2.** La garde de troncature est attestée par un test : une page pleine
  (30 événements sans `?per_page`, ou 100 avec) est lue comme une troncature
  possible et jamais comme une absence de PR.
- **AC3.** Le coût `gh` par cycle est écrit avant et après dans le plan, K est
  borné par une constante nommée, et le bornage est attesté par un test qui
  compte les invocations.
- **AC4, la sonde post-déploiement.** Au premier rapport après `make deploy`, la
  ligne « Next » du milestone 33 ne propose plus #2698 — et si elle propose un
  autre ticket, c'est un ticket sans PR mergée, vérifié à la main.

Où chacun est servi : AC1 et l'AC2 du parent par les tests de R3 et R4 ; l'AC2
de #2930 par `mika2924_une_page_pleine_est_une_troncature_et_jamais_une_absence`
(la mesure 36 contre 30 événements de #2698 est ce qui rend la garde nécessaire,
et elle est écrite dans la doc de `TIMELINE_PAGE_SIZE`) ; AC3 par le tableau de
coût ci-dessus, `TIMELINE_PROBE_MAX` et
`mika2924_la_sonde_est_bornee_a_une_lecture_par_cycle` ; AC4 par la section
*Sonde post-déploiement*.

## Fire-Disposition

Ce plan livre **un** détecteur :
`mika2924_la_sonde_est_bornee_a_une_lecture_par_cycle`, dont le chemin de succès
est « aucun appel `gh` au-delà de la borne ».

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est livrée
VIDE.**

- La borne est `TIMELINE_PROBE_MAX = 1`, et aucun site existant n'émet d'appel
  `timeline` : la population des violations préexistantes est **vide par
  construction**, donc l'allowlist n'a aucune entrée à porter et il n'y a aucune
  dette à reporter.
- L'assertion est **auto-nettoyante** : le test lit `TIMELINE_PROBE_MAX` plutôt
  qu'un littéral `1`, donc une recalibration de la borne ne le rend pas stale,
  et un second site d'appel le fait rougir immédiatement au compte.
- Le détecteur atterrit **armé** : il est vert sur le code de ce vol, mesuré
  avant livraison, donc ni (b) ni (c) n'ont lieu de s'appliquer.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/types.rs` (R1, production + tests d'arriéré) | 60 |
| `crates/mika-agent/src/milestone_manager/assessor.rs` (R2 + R3, production) | 70 |
| `crates/mika-agent/src/milestone_manager/reader.rs` (R4 + R5 + R6, production) | 85 |
| tests `reader.rs` (composition, troncature, filtres, fixtures mesurées) | 80 |
| tests `assessor.rs` (quatre libellés, cible de sonde) | 70 |
| test de bornage (compte des invocations + contrôle négatif palier 1) | 40 |

Total estimé : 405 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 405 est sous la borne de 429 et très loin du
seuil par PR de 1000. Trois raisons structurelles, pas un pari :

- **aucun fichier nouveau, aucune migration, aucun schéma** — quatre fichiers
  existants touchés, dont `mod.rs` pour quatre lignes de réexport ;
- **le vocabulaire du vol 1 est réutilisé tel quel** — `PrLinkSource`, `pr_url`
  et le contrat `None = binaire antérieur` sont déjà là, déjà testés, et ce vol
  ne fait qu'y ranger un champ de plus avec le même `#[serde(default)]` ;
- **un seul appel `gh` ajouté, et une seule fonction pure pour le lire** — le
  patron (lecture bornée, compositeur pur, politique d'échec log-and-skip) est
  copié trait pour trait sur la lecture 6, qui est dans le même fichier à
  quarante lignes de là.

Le poste le plus lourd est le test, et il est irréductible : les quatre
propriétés attestées (le libellé positif, le contrôle négatif, la troncature, le
bornage) sont chacune une des quatre AC.

## Ordre de commit

Trois commits, chacun avec son plancher de suite verte.

1. **`feat(2924): le vocabulaire de l'état des PR d'un ticket ouvert`** — R1.
   Inerte : aucun producteur, aucun consommateur. Plancher : les tests d'arriéré
   de `types.rs` passent, le blob antérieur se relit.
2. **`feat(2924): la sonde de timeline, bornée à une lecture par cycle`** — R2,
   R4, R5, R6. Produit l'information ; **personne ne la lit encore**, donc le
   rapport ne change pas d'un octet. Plancher : les tests de composition, le
   test de bornage et son contrôle négatif passent ; les sept tests de
   `pick_next_action` sont **inchangés** et verts, ce qui atteste que l'extraction
   de R2 n'a pas bougé le prédicat.
3. **`fix(2924): « Next » nomme les PR mergées au lieu de réclamer une
   implémentation`** — R3. Le seul commit qui change le texte du rapport.
   Plancher : les quatre tests de libellé, dont le contrôle négatif et le blob
   sans sonde.

Pourquoi cet ordre : le commit 2 est mesurable en production **avant** que le
rapport n'en dépende, par la sonde de R5 seule. Si le compte d'appels ou le taux
d'indétermination y était mauvais, le commit 3 serait révisé sans qu'un rapport
faux ait été servi entre-temps. C'est le même découpage que le vol 1, dont le
premier commit est explicitement « inerte — aucun producteur, aucun
consommateur ».

## Sonde post-déploiement (AC4)

L'instrument n'est pas vivant avant `make deploy` : `milestone_manager` est
compilé dans le binaire, donc aucune des trois mesures ci-dessous ne peut être
prise avant.

**1. La ligne « Next » du premier rapport après le déploiement.**

```bash
grep -h '^- Next:' "$(ls -t ~/.mika/manager/sink/senara-solutions-mika-33-*.md | head -1)"
```

Attendu : la ligne ne porte **pas** `no PR` si elle nomme un ticket cité par des
PR mergées. Sur l'état du 2026-10-27, le candidat le plus probable est #2698 lui-même,
et la ligne attendue est de la forme :

```
- Next: #2698 — #2698 has a plan and merged PRs already cite it (PR #2931, #2929, #2892 (+3 more)) — check what remains before implementing
```

Si la ligne nomme **un autre** ticket, AC4 demande de vérifier à la main que ce
ticket est bien sans PR mergée :

```bash
gh api "repos/senara-solutions/mika/issues/<N>/timeline?per_page=100" \
  --jq '[.[] | select(.event=="cross-referenced") | select(.source.issue.pull_request != null) | select(.source.issue.pull_request.merged_at != null) | .source.issue.number]'
```

Attendu dans ce cas : `[]`.

**2. Le compte d'appels, mesuré et non supposé.**

```bash
journalctl -u mika-spirit --since "-7h" | grep -c '"event":"manager_timeline_probe"'
```

Attendu : **une ligne par cycle**, toutes branches confondues — c'est l'invariant
« émise sur toutes les branches ». Et la répartition :

```bash
journalctl -u mika-spirit --since "-7h" \
  | grep '"event":"manager_timeline_probe"' \
  | python3 -c 'import sys,json,collections; print(collections.Counter(json.loads(l).get("fields",{}).get("outcome") for l in sys.stdin))'
```

Attendu : `citing` ou `none_citing` en régime nominal, `not_emitted` quand un
ticket du milestone porte une PR ouverte. **`indeterminate_page_full` non nul est
un signal à remonter** : il voudrait dire qu'un ticket du vivier a dépassé 100
événements, et c'est la prémisse du refus n° 6 des Non-Requirements qui tombe.

**3. Le non-effet sur le vol 1, qui doit rester exact.**

```bash
grep -c 'closed (no linked PR)' "$(ls -t ~/.mika/manager/sink/senara-solutions-mika-33-*.md | head -1)"
```

Attendu : **14**, inchangé depuis le vol 1 — ce vol ne touche pas le rendu des
tickets fermés. Une autre valeur signifie une régression sur le vol 1, pas un
progrès de celui-ci.
