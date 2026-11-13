# mika#2924 — la PR qui ferme un ticket se lit depuis le ticket (vol 1/2)

## Contexte

Le rapport 6 h de mika-manager dit faux sur ce qui est livré. Dans le dernier
rapport du sink, **87 lignes** de tickets fermés du milestone 33 portent
`closed (no linked PR)`, et **aucune** ne porte de lien de PR. C'est ce rapport
que Vincent et MPC lisent du manager, et sur un milestone à 162 tickets fermés
il dit « rien n'est livré ».

La cause est localisée, et elle est d'une seule ligne de conception.
`reader.rs::compose_from_gh_outputs` construit l'index inverse
« quelle PR ferme le ticket N ? » **exclusivement** depuis la liste des PR du
milestone :

```rust
// reader.rs:676 — l'index fautif
for pr in &prs {
    if let Some(refs) = pr.get("closingIssuesReferences").and_then(|v| v.as_array()) {
        for r in refs {
            if let Some(issue_num) = r.get("number").and_then(|v| v.as_u64()) {
                pr_by_issue.entry(issue_num).or_insert_with(|| (pr_number, pr_state.clone(), ci.clone()));
            }
        }
    }
}
```

et `prs` vient de `gh pr list --state all --search milestone:N`. Or **la boucle
autonome rattache le milestone au TICKET, jamais à la PR** : le pilote pose le
milestone sur l'issue, `gh pr create` n'en pose aucun. L'ensemble `prs` est donc
quasi vide de PR produites par la boucle, l'index est vide pour elles, et
`write_completed` (`reporter.rs:557`) rend la branche `None` pour chaque ticket
fermé. Le doc-comment du site le disait déjà, en nommant la cause sans la
corriger : « *hand-closed, **or PR not tagged with the milestone*** ».

La direction du lien est le défaut. GitHub tient la même relation **des deux
côtés**, et le côté ticket est celui que le producteur remplit : `Closes #N`
dans le corps de la PR alimente `closedByPullRequestsReferences` **sur
l'issue**, sans qu'aucun milestone n'ait à exister.

### Ce que la mesure a dit, avant d'écrire une ligne

Le ticket exige de choisir le champ en mesurant sur #2886/#2892 et #2698/#2704.
Mesuré le 2026-10-26, sur le dépôt réel :

| question mesurée | réponse |
|---|---|
| `closedByPullRequestsReferences` est-il peuplé pour #2886 ? | **oui** — une entrée, `number: 2892`, `url: https://github.com/senara-solutions/mika/pull/2892` |
| le champ est-il servi par l'appel `gh issue list --json` **déjà émis** ? | **oui** — il s'ajoute à la liste de champs existante ; 1 appel avant, 1 appel après (1,639 s → 1,795 s pour le même appel) |
| combien de tickets fermés du milestone 33 ont une PR de clôture côté issue ? | **73 sur 87** ; les 14 restants sont fermés à la main et n'ont aucune PR |
| le champ répond-il pour #2698 (ouvert, PR de vol mergées) ? | **non** — `closedByPullRequestsReferences: []`, et les PR #2704/#2706 ne déclarent **aucun** `closingIssuesReferences` |
| que reste-t-il pour la forme #2698 ? | le `timeline` REST de l'issue, événements `cross-referenced` filtrés sur `source.issue.pull_request != null` **et** `merged_at != null` → #2698 → 2704, 2706, 2712, 2892. **Un appel `gh` par candidat probé**, et la page par défaut est 30 (#2698 la remplit exactement : `?per_page=100` obligatoire, page pleine = troncature possible, jamais une absence) |

Deux conséquences de cette mesure décident le découpage :

1. **AC1 et AC3 coûtent zéro appel `gh` supplémentaire** et touchent une seule
   surface (le rendu des tickets fermés).
2. **AC2 ne peut pas être servi par un champ de référence de clôture.** Il
   réclame une source de données différente (timeline REST), un coût différent
   (K appels bornés par cycle), une garde de troncature propre et une surface
   différente (`assessor::pick_next_action`). Ce n'est pas la suite du même
   geste, c'est un second vol.

## Ce qui est refusé (bearing, non négociable)

- **Remplacer la liste des PR du milestone par la référence côté issue.** Elle
  reste la **source unique** de `pr_state` et `ci_state`, parce que trois
  consommateurs en dépendent et que la référence de clôture ne les porte pas :
  `is_in_flight()` / la section In-flight, le Tier 1 de `pick_next_action`,
  `is_hard_blocked()`, et surtout `detect_silent_progress`, qui exige
  `state == Open && pr_state == Some("merged")`. Le changement est une
  **union** : la référence côté issue **comble le trou**, elle ne remplace rien.
- **Recomposer l'URL de la PR depuis `{repo}/pull/{number}`** sur le chemin
  issue-side. `closedByPullRequestsReferences` peut porter une PR d'un **autre
  dépôt** ; la recomposition rendrait alors une URL fausse qui ressemble à une
  URL vraie. On rend le champ `url` que la référence porte elle-même. Le chemin
  milestone garde sa recomposition : là, la PR vient de `gh pr list` sur le
  dépôt courant, donc le dépôt est connu par construction.
- **Deviner `pr_state`/`ci_state` pour un lien issue-side.** Pour un ticket
  **fermé**, `In-flight`, Tier 1 et `detect_silent_progress` ne le lisent pas ;
  les laisser à `None`/`CiState::None` est exact et observable, les inventer
  serait une invention. `pr_link_source` est ce qui rend cette asymétrie
  lisible plutôt que surprenante.
- **Déduire le numéro de vol.** Le ticket illustre AC2 par « vol N livré, PR #… ».
  **Aucun champ `gh` n'atteste un numéro de vol** — ni le titre de la PR (texte
  libre), ni un label, ni la timeline. Le vol 2 nommera les PR mergées qui
  **citent** le ticket, jamais un numéro de vol fabriqué. Divergence assumée
  avec la lettre du ticket, signalée ici parce qu'elle appartient au vol 2.
- **Un second appel `gh` dans ce vol.** Le ticket demande que le coût soit
  mesuré avant/après et borné s'il augmente. Dans ce vol il **n'augmente pas**,
  et c'est une propriété à préserver : si une revue propose d'enrichir le lien
  par un appel de plus, elle enrichit le vol 2.

## Où ce vol se situe

Deux vols, séparés par la source de données et non par la commodité.

| vol | périmètre | source de données | coût `gh` ajouté | mergeable seul |
|---|---|---|---|---|
| **1 — celui-ci** | AC1 + AC3 | `closedByPullRequestsReferences`, sur l'appel `gh issue list` **existant** | **0** | oui — fait passer les 87 lignes à 14 et satisfait la sonde AC3 |
| 2 — suivi | AC2 | `timeline` REST par candidat, `?per_page=100`, borné + garde de troncature | K borné, mesuré dans son plan | oui — `pick_next_action` seul |

Le vol 1 est celui qui éteint le mensonge du rapport. Le vol 2 corrige une
proposition « Next » erronée sur une population mesurée à 5 tickets ouverts
porteurs de plan (#2923, #2922, #2921, #2699, #2698), dont un seul présente
aujourd'hui la forme du défaut.

## L'endroit, choisi et justifié

Quatre fichiers, et le choix de chacun est contraint.

**1. `reader.rs::read_with_runner` — la demande du champ.** Le champ s'ajoute à
la liste `--json` de la lecture d'issues déjà émise :

```
gh issue list --milestone N --state all \
  --json number,title,state,body,labels,updatedAt,closedByPullRequestsReferences \
  --limit 100
```

C'est le seul site qui compose un argv `gh`, et c'est ce qui garde le coût à
zéro appel ajouté.

**2. `reader.rs::compose_from_gh_outputs` — l'union, dans la fonction pure.**
Le site de consommation existe déjà (ligne 765) :

```rust
let (pr_number, pr_state, ci_state) = pr_by_issue
    .get(&number).cloned()
    .map(|(n, s, c)| (Some(n), Some(s), c))
    .unwrap_or((None, None, CiState::None));
```

L'union se pose **dans le bras `unwrap_or`** : quand l'index du milestone ne
répond pas, lire `closedByPullRequestsReferences` du JSON de l'issue, prendre
la **première** entrée (premier-écrivain gagne, même règle déterministe que
l'index) et en retenir `number` **et** `url`. La fonction reste pure : tout ce
qu'elle lit est déjà dans ses arguments, donc les tests tournent sur du JSON de
fixture sans sous-processus.

**3. `types.rs::SubIssue` — deux champs, et le second est l'instrument.**

```rust
/// L'URL de la PR de clôture **telle que GitHub la donne**, quand le lien vient
/// de la référence côté issue. `None` sur le chemin liste-du-milestone, où le
/// dépôt est connu par construction et l'URL recomposée.
#[serde(default)]
pub pr_url: Option<String>,

/// D'OÙ vient `pr_number` (mika#2924). Sans ce champ, « aucune PR » et
/// « une PR que la liste du milestone ne voit pas » rendent les mêmes octets
/// (classe mika#2205), et la sonde AC3 ne peut pas distinguer les 14 tickets
/// fermés à la main des tickets que le correctif vient de réparer.
#[serde(default)]
pub pr_link_source: Option<PrLinkSource>,
```

avec `enum PrLinkSource { MilestonePrList, IssueClosingRef }`. `Option` +
`#[serde(default)]` est le motif déjà en place sur `MilestoneState`
(`outside_read_truncated`), et il rend un état d'avant le changement lisible
comme **absent**, jamais comme « liste du milestone » (mika#2331 : absence ≠
valeur par défaut).

**4. `reporter.rs::write_completed` — le rendu, trois cas au lieu de deux.**

```rust
match (s.pr_url.as_deref(), s.pr_number) {
    (Some(url), _)  => writeln!(out, "  - #{n} {title} — PR {url}", …),
    (None, Some(pr)) => writeln!(out, "  - #{n} {title} — PR https://github.com/{repo}/pull/{pr}", …),
    (None, None)     => writeln!(out, "  - #{n} {title} — closed (no linked PR)", …),
}
```

Le troisième bras garde son texte **mot pour mot** : c'est lui que la commande
de comptage d'AC3 grepe, et c'est lui qui doit rester vrai pour les 14 tickets
fermés à la main. Aucune autre section du rapport ne change.

## La perte de revue, et ce qui l'empêche

La régression qui repasse en revue n'est pas « le lien est faux ». C'est **un
second site qui résout une PR de clôture depuis la liste du milestone seule**,
posé six mois plus tard par quelqu'un qui n'a pas lu ce plan : le rapport
resterait vert, l'index resterait vide pour les PR de la boucle, et le défaut
reviendrait sur une nouvelle surface sans qu'aucune assertion comportementale
ne rougisse. C'est la classe mika#2158 — le site correct existe, un second site
le contourne.

Le détecteur livré par ce vol est donc un **scan de source à lecteur unique** :
sous `src/milestone_manager/`, dans la **moitié production** des fichiers,
l'ensemble des corps de fonction qui mentionnent `closingIssuesReferences` ou
`pr_by_issue` doit être **exactement** `{compose_from_gh_outputs}`. Allowlist
livrée **vide** (mesurée : aucun autre site aujourd'hui), contrôle négatif et
assertion d'anti-vacuité — détail en § *Fire-Disposition*.

Il est complémentaire et non redondant avec le test comportemental d'AC1 : le
test dit que le lien est rendu, le scan dit qu'il n'existe qu'un endroit où il
peut cesser de l'être.

## Requirements

- **R1.** `read_with_runner` demande `closedByPullRequestsReferences` sur la
  lecture d'issues **déjà émise** du milestone. Aucun appel `gh`
  supplémentaire : 5 appels fixes + jusqu'à `PLAN_SIZE_READ_MAX` (= 12) lectures
  de plan avant, **identique** après.
- **R2.** `compose_from_gh_outputs` résout la PR de clôture par **union** :
  l'index de la liste du milestone d'abord (inchangé, il seul porte `pr_state`
  et `ci_state`), puis la référence côté issue en comblement. Première entrée
  retenue, déterministe.
- **R3.** Sur le chemin issue-side, l'URL rendue est le champ `url` de la
  référence, jamais une recomposition. `pr_state` et `ci_state` restent
  `None`/`CiState::None`.
- **R4.** `SubIssue` porte `pr_url: Option<String>` et
  `pr_link_source: Option<PrLinkSource>`, tous deux `#[serde(default)]`.
  `pr_link_source` est posé par les **deux** chemins, de sorte que sa valeur
  `None` signifie « état sérialisé par un binaire antérieur au changement »
  (mika#2340) et jamais « liste du milestone ».
- **R5.** `write_completed` rend l'URL portée quand elle existe, la
  recomposition quand seul le numéro existe, et conserve **mot pour mot**
  `closed (no linked PR)` quand ni l'un ni l'autre n'existe.
- **R6.** Un test unique porte **le contrôle positif et le contrôle négatif** :
  #2886 → `PR …/pull/2892`, #1680 → `closed (no linked PR)`. La fixture est une
  **sortie `gh` réelle capturée**, et une seule — les deux contrôles vivent dans
  le même document JSON, car c'est ainsi qu'ils arrivent en production.
- **R7.** Le scan de source à lecteur unique de § *La perte de revue* est livré,
  allowlist vide, avec son contrôle négatif et son anti-vacuité.
- **R8.** Le coût `gh` par cycle est écrit dans le plan avant et après
  (§ *Coût `gh` par cycle* ci-dessous), comme le ticket l'exige.

## Non-Requirements (hors de ce vol, délibérément)

- **AC2** — « Next » ne propose plus un ticket ouvert porteur de PR mergées.
  Source de données différente (timeline REST), coût différent, surface
  différente. Vol 2, voir la ligne `Tracked in:` du § *Taille estimée*.
- **La formule « vol N livré »** du corps du ticket : aucun champ `gh` ne
  l'atteste. Le vol 2 nommera les PR, pas les vols.
- **Un lien de PR dans les sections In-flight, Blocked, Unstarted.** Elles
  lisent `pr_state`/`ci_state`, que la référence côté issue ne porte pas ; les
  toucher demanderait la source du vol 2.
- **Le rattachement du milestone aux PR produites par la boucle.** C'est l'autre
  remède possible au même défaut — modifier `dispatch-lib.sh` pour poser le
  milestone sur la PR — et il est refusé ici : il ne répare que les PR
  **futures**, laisse les 87 lignes historiques fausses, et met le correctif
  dans un fichier que `make deploy` seul propage.
- **Un dénombrement de `no linked PR` dans le rapport lui-même.** La sonde AC3
  se lit par un `grep -c` sur le rapport du sink ; ajouter un compteur rendu
  déplacerait la mesure dans la chose mesurée.

## Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | AC1, les deux contrôles dans un test | `cargo test -p mika-agent milestone_manager::reporter::tests::mika2924` — un test, fixture `gh` réelle, assertion positive sur `pull/2892` **et** assertion négative sur `closed (no linked PR)` pour #1680 |
| V2 | l'union ne régresse pas la source de `pr_state`/`ci_state` | `compose_end_to_end` et `compose_end_to_end_mix_closed_open_blocked_locks_avancement` (ce dernier assère `closed.pr_number == Some(1929)` depuis la liste du milestone) passent **inchangés** |
| V3 | `detect_silent_progress` voit toujours `pr_state == Some("merged")` | la suite `assessor` entière passe inchangée |
| V4 | lecteur unique de la résolution de PR de clôture | le scan de R7 est vert avec allowlist vide ; son contrôle négatif rougit sur un second lecteur injecté ; son anti-vacuité rougit si l'ensemble trouvé est vide |
| V5 | zéro appel `gh` ajouté | le test d'argv de `read_with_runner` : la liste des champs `--json` change, le **nombre** d'invocations du `GhRunner` injecté ne change pas |
| V6 | `pr_link_source` absent ≠ liste du milestone | un `MilestoneState` sérialisé sans le champ se désérialise en `None`, assertion explicite |
| V7 | suite, lints et pipeline | `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` |
| V8 | sonde post-déploiement | § *Sonde post-déploiement* — après `make deploy`, le premier rapport du sink |

### Coût `gh` par cycle (R8 — la mesure que le ticket exige)

| | appels fixes | lectures de plan | total par cycle |
|---|---|---|---|
| **avant** | 5 (milestone, issues du milestone, PR du milestone, vivier `ready`, candidats hors milestone) | ≤ `PLAN_SIZE_READ_MAX` = 12 | **≤ 17** |
| **après (vol 1)** | 5 — le champ s'ajoute à un appel existant | ≤ 12 | **≤ 17**, inchangé |
| vol 2 (pour mémoire) | 5 | ≤ 12 | + K bornés, mesurés et justifiés dans son propre plan |

Latence mesurée de l'appel enrichi : 1,639 s → 1,795 s pour la même lecture
unique. Aucune augmentation à borner, donc aucune justification à produire.

## Definition of Done

- R1–R8 livrés.
- V1–V7 verts localement et sur le `Check` requis de la PR.
- Le corps de PR **ne porte pas** `Closes senara-solutions/mika#2924` : AC2
  reste ouvert, donc le ticket ne se ferme qu'au merge du vol 2. Il porte à la
  place une ligne `Tracked in:` nommant le ticket de suivi du vol 2.
- `Code review: delegated to mika-qa (mika#2694)` dans le corps de PR.
- Le ticket de suivi du vol 2 est ouvert **avant** l'ouverture de la PR et son
  numéro est celui de la ligne `Tracked in:` du corps.
- Le merge revient à Vincent (`loop-substrate`).

## Acceptance criteria

Transcrits mot pour mot du corps de senara-solutions/mika#2924.

- **AC1.** Sur une sortie `gh` capturée pour #2886 (fermé par #2892) et pour un
  ticket fermé à la main sans PR, le rapport rend « PR …/pull/2892 » pour le
  premier et `closed (no linked PR)` pour le second. Le contrôle positif et le
  contrôle négatif sont dans le même test.
- **AC2.** Un ticket ouvert avec des PR mergées (la forme de #2698) n'est plus
  proposé en « Next » comme « no PR ».
- **AC3, la sonde post-déploiement.** Au premier rapport après `make deploy`, le
  compte de `no linked PR` du sink baisse depuis 87. Le plan donne la commande
  qui le compte, et le nombre restant est expliqué ticket par ticket sur un
  échantillon.

**Couverture par vol.** AC1 et AC3 sont servis par ce vol. **AC2 est servi par
le vol 2**, nommé par la ligne `Tracked in:` du § *Taille estimée* : la mesure
du 2026-10-26 établit que `closedByPullRequestsReferences` rend `[]` pour #2698
et que les PR #2704/#2706 ne déclarent aucun `closingIssuesReferences`, donc
aucun champ de référence de clôture ne peut le satisfaire — il réclame la
timeline REST, un appel borné par candidat et sa propre garde de troncature.
L'architecte juge le périmètre du vol courant.

## Fire-Disposition

Ce vol livre **un** détecteur : le scan de source à lecteur unique de R7
(`mika2924_la_resolution_de_pr_de_cloture_a_un_lecteur_unique`). Les tests
comportementaux V1–V3 ne sont pas des détecteurs au sens de mika#2306 — leur
chemin de succès est « le rendu est celui attendu », pas « aucune violation
trouvée ».

**Option retenue : (a) exception nommée en allowlist, livrée VIDE.**

Mesure préalable, qui est ce qui rend l'allowlist vide tenable :
`closingIssuesReferences` et `pr_by_issue` n'apparaissent aujourd'hui, dans la
moitié production de `src/milestone_manager/`, que dans le corps de
`compose_from_gh_outputs` — plus trois lignes de **commentaire** dans
`reader.rs` et `types.rs`. Aucune violation existante, donc aucune entrée à
nommer, donc **l'allowlist est livrée vide et son vide est assérté**.

Implémentation, et chacune des quatre propriétés répond à un mode d'échec
observé dans ce dépôt :

1. **Ensemble exact, auto-nettoyant.** L'assertion est
   `trouvés == {"compose_from_gh_outputs"}` — une **égalité d'ensembles**, pas
   un `contains`. Un site retiré rougit autant qu'un site ajouté : le geste qui
   rend le scan sans objet est le geste qui le fait expirer, ce qui l'empêche de
   devenir l'allowlist que mika#2309 condamne.
2. **Allowlist vide, et son vide épinglé.** `const ALLOWLIST: &[&str] = &[]`,
   plus une assertion `ALLOWLIST.is_empty()` portant en message le ticket à
   ouvrir si une entrée devient nécessaire. Une entrée future doit nommer la
   **donnée précise** (le nom de fonction) et référencer un ticket de suivi ;
   sans ticket, l'assertion de vide refuse l'entrée.
3. **Contrôle négatif de bonne foi.**
   `mika2924_le_scan_rougit_sur_un_second_lecteur` fait tourner le même
   prédicat sur une source **synthétique** portant une seconde fonction qui lit
   `closingIssuesReferences`, et exige que le prédicat la trouve. Sans lui, un
   prédicat qui ne matche plus rien (renommage de champ, reflow rustfmt) serait
   indistinguable d'un dépôt sain.
4. **Anti-vacuité.** L'assertion exige que l'ensemble trouvé soit **non vide**
   avant de le comparer. Un `fn_bodies` qui rend `vec![]` — chemin faux, fichier
   déplacé, `production_half` qui tronque trop tôt — rend un scan vert et muet ;
   l'anti-vacuité est ce qui transforme ce silence en rouge.

Le prédicat ancre sur les lignes **hors commentaire** : la prose *à propos* du
champ n'est pas une lecture du champ, et ce plan comme les doc-comments de R4 en
contiennent plusieurs paragraphes (motif mika#2050, signal S). Le scan passe par
`crate::source_scan::{fn_bodies, production_half}` et
`source_scan::is_test_source_path`, l'API déjà en place, sans en écrire une
seconde.

**Quand il rougit : router le site par `compose_from_gh_outputs`, ne pas ajouter
de ligne à l'allowlist** (doctrine mika#2201). L'allowlist existe pour nommer
une dette déjà présente, et il n'y en a aucune.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/reader.rs` (champ `--json`, union, pose de `pr_link_source`, doc-comments) | 75 |
| tests `reader.rs` (V2, V5, V6 + fixture `gh` réelle) | 110 |
| `crates/mika-agent/src/milestone_manager/types.rs` (`pr_url`, `pr_link_source`, `enum PrLinkSource`, ré-export `mod.rs`) | 40 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (`write_completed` trois bras + test AC1 à deux contrôles) | 45 |
| détecteur : scan de source lecteur unique + contrôle négatif + anti-vacuité | 75 |

Total estimé : 345 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un découpage en vols, et pourquoi celui-ci tient.** Le périmètre
complet d'AC1 + AC2 + AC3 est estimé à **≈ 635 lignes** (345 pour AC1/AC3,
≈ 290 pour AC2 : résolution par timeline REST ≈ 85, garde de troncature et
bornage des lectures ≈ 40, `pick_next_action` et sa formulation ≈ 45, tests
≈ 120). C'est **au-dessus de la borne par vol de 429**, donc le découpage est
imposé par la règle, et non choisi par confort.

La couture est propre, ce qui est la condition pour que chaque vol soit
mergeable seul : source de données différente (champ `--json` déjà payé contre
appel REST par candidat), surface différente (`reporter::write_completed` contre
`assessor::pick_next_action`), profil de coût différent (0 appel contre K
bornés). Le vol 1 **à lui seul** fait passer les 87 lignes fausses à 14 et
satisfait la sonde AC3 ; il ne laisse pas un demi-correctif en production.

Le vol courant, à 345 lignes, repasse sous la borne de 429 **et** sous le seuil
par PR de 1000. C'est la borne par vol qui mord ici, et c'est elle qui a décidé
le découpage.

Tracked in: un ticket de suivi « mika#2924 vol 2 — « Next » ne propose plus
d'implémenter un ticket qui a déjà des PR mergées (AC2, timeline REST bornée) »,
ouvert à l'étape 7 de l'ordre de commit ci-dessous, **avant** l'ouverture de la
PR ; son numéro est la valeur de la ligne `Tracked in:` du corps de PR. Son
périmètre n'entre pas dans le total ci-dessus — l'architecte juge le vol
courant.

## Ordre de commit

Plancher : chaque commit laisse la suite verte.

1. **`types.rs` + `mod.rs`** — `enum PrLinkSource`, `pr_url`, `pr_link_source`,
   ré-export. Inerte : aucun producteur, aucun consommateur. V6 (absent se
   désérialise en `None`) est vert dès ce commit. *Plancher : `cargo test -p
   mika-agent` vert, aucun changement de comportement.*
2. **`reader.rs` — le champ `--json`** seul, plus la pose de
   `pr_link_source: Some(MilestonePrList)` sur le chemin existant. V5 (nombre
   d'invocations `gh` inchangé) est vert ici. *Plancher : V2, V3, V5 verts.*
3. **`reader.rs` — l'union** dans le bras de comblement de
   `compose_from_gh_outputs`, avec `pr_url` et
   `pr_link_source: Some(IssueClosingRef)`. Les tests `compose_end_to_end*`
   restent **inchangés** : c'est la preuve que la source de `pr_state`/`ci_state`
   n'a pas bougé. *Plancher : V2, V3 verts sans retouche.*
4. **Fixture + tests `reader.rs`** — sortie `gh` réelle capturée sous
   `.pilot-scratch/` d'abord pour inspection, puis commitée comme fixture de
   test avec les deux contrôles (#2886 peuplé, #1680 `[]`) dans le **même**
   document. *Plancher : V2, V5, V6 verts.*
5. **`reporter.rs`** — `write_completed` à trois bras + le test AC1 portant les
   deux contrôles. Le texte `closed (no linked PR)` est inchangé. *Plancher :
   V1 vert, tests existants du rapporteur verts.*
6. **Le détecteur** — scan lecteur unique, allowlist vide assérée, contrôle
   négatif, anti-vacuité. Dernier parce qu'il atteste l'état final.
   *Plancher : V4 vert.*
7. **Ouvrir le ticket de suivi du vol 2** (`gh issue create`, label
   `loop-substrate`, corps portant la mesure #2698 / `[]` / #2704–#2706 et le
   coût K à borner), **puis** la PR avec `Tracked in: <son numéro>`.
8. **Fin de pipeline** : `cargo test`, `cargo clippy -- -D warnings`,
   `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main`, commit, push,
   `gh pr create --body-file pr-body.md`, puis `rm pr-body.md`.

## Sonde post-déploiement

**L'instrument n'est pas vivant avant `make deploy`.** Le rapport est produit
par le binaire ; un correctif dans l'arbre de travail ne change aucun rapport.
Un rapport du sink antérieur au déploiement qui porterait encore 87 lignes
signifie « binaire antérieur au changement » (motif mika#2340), jamais « le
correctif ne marche pas ».

**La commande qui compte (AC3).** Le sink résout par
`resolve_offline_sink_dir()` → `$HOME/.mika/manager/sink` (surchargeable par
`ENV_OFFLINE_SINK_DIR`), et le lecteur canonique est la CLI — on ne recompose
pas le chemin à la main :

```bash
# Sur l'hôte, après `make deploy` et le premier cycle manager suivant.
mika milestone reports --target senara-solutions/mika#33 --latest \
  | grep -c 'closed (no linked PR)'
```

`--latest` imprime le Markdown du rapport le plus récent et distingue
« sink absent » de « sink vide » — deux états que `grep -c` sur un chemin
recomposé confondrait en `0`.

**Lecture attendue :** **87 → 14.** Mesuré le 2026-10-26 sur le milestone 33 :
87 sous-tickets fermés, dont **73** portent une PR de clôture côté issue et
**14** n'en portent aucune.

**Les 14 restants, nommés ticket par ticket** — ce sont les tickets fermés à la
main (décision opérateur, doublon, périmètre abandonné), sans aucune PR les
citant. Leur ligne `closed (no linked PR)` est **vraie** et doit rester :

> #2735, #2687, #2127, #2121, #2105, #2077, #2056, #2054, #2048, #2034, #2029,
> #1812, #1713, #1680.

**Contrôle positif ciblé**, sur l'exemple que le ticket cite mot pour mot :

```bash
mika milestone reports --target senara-solutions/mika#33 --latest | grep -n '#2886'
```

doit rendre `PR https://github.com/senara-solutions/mika/pull/2892` et non
`closed (no linked PR)`.

**Lectures qui ne concluent pas, et ce qu'elles veulent dire :**

| observation | lecture |
|---|---|
| le compte reste à 87 | binaire antérieur au vol (mika#2340) — vérifier le sha servi avant de suspecter le correctif |
| le compte tombe sous 14 | un ticket fermé à la main a reçu un lien : régression de R3 (URL recomposée) ou union trop large — à instruire, pas à célébrer |
| le compte est entre 14 et 87 | état attendu **transitoire** si de nouveaux tickets ont été fermés depuis la mesure ; re-dénombrer les tickets fermés avant de conclure |
| « sink vide » / « sink absent » | aucun cycle manager n'a tourné depuis le déploiement — la sonde n'a pas encore d'objet |

## Risques nommés

| # | risque | remède |
|---|---|---|
| **R1** | **Le `gh` de l'hôte ne connaît pas `closedByPullRequestsReferences`.** Un champ `--json` inconnu fait échouer l'appel **entier** (`unknown JSON field`), donc la lecture des issues du milestone, donc le cycle — surfacé en `manager_cycle_error`. La version du `gh` de l'hôte **n'est pas vérifiable depuis le bac à sable** (`gh --version` est refusé par la permission-policy), donc ce risque est nommé plutôt que supposé écarté. | Le champ **est** servi par le `gh` qui a produit la mesure du 2026-10-26 sur ce dépôt, ce qui établit qu'il existe côté API. Si l'hôte diverge, le remède est un revert d'une ligne (la liste `--json` du commit 2) et le cycle redevient celui d'avant. À vérifier en premier si `manager_cycle_error` apparaît après le déploiement. |
| R2 | Une PR de clôture **d'un autre dépôt** rendrait une URL fausse si elle était recomposée. | R3 : on rend le champ `url` porté par la référence. Pas de recomposition sur ce chemin. |
| R3 | L'union masque une régression de `pr_state`/`ci_state` en remplissant `pr_number` par une autre voie. | V2/V3 : les tests `compose_end_to_end*` et la suite `assessor` passent **sans retouche**. Une retouche nécessaire sur l'un d'eux est le signal que l'union a débordé. |
| R4 | `closedByPullRequestsReferences` porte plusieurs entrées (plusieurs PR citant `Closes`). | Première entrée, premier-écrivain gagne — même règle déterministe que l'index existant, documentée au site. |
| R5 | Le scan de source devient sans objet après un renommage de champ par GitHub. | Le contrôle négatif rougit alors sur la source synthétique : le prédicat qui ne matche plus rien est détecté comme tel, et non lu comme un dépôt sain. |
