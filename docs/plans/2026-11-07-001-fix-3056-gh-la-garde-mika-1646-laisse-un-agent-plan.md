# mika#3056 — la garde refuse toute fermeture d'issue, et ne la refuse sur aucune lecture

Ticket : senara-solutions/mika#3056 (`bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`)
Base : `4fb4a4dde` (mesure du ticket) / `e125b95c2` (HEAD de la branche)

---

## 0 — Le défaut, en une phrase

La garde mika#1646 demande à une fermeture d'être **fondée**. La doctrine du
2026-11-06 demande à une fermeture d'issue d'être **portée par un merge**. Ce ne
sont pas deux degrés de la même exigence : ce sont deux exigences de nature
différente. La première est satisfaite par une lecture et une citation — un fait
d'état. La seconde ne peut être satisfaite par aucune lecture, parce qu'elle
porte sur l'origine du geste et non sur sa justesse. Fermer est un verdict.

C'est pourquoi le correctif n'est pas un durcissement de la couche A. Une couche
A plus stricte reste une couche A : elle continue de dire « montre-moi que tu as
lu », et la mesure du ticket montre que l'agent finit toujours par le montrer —
trois fois sur neuf, après un à trois refus, au tour suivant, avec une citation
exacte. La garde n'a pas été contournée : elle a été **satisfaite**. Un gardien
qu'on satisfait en lisant mieux ne garde pas un verdict.

Le correctif est donc un bras qui ne lit rien.

## 1 — La mesure, et ce qu'elle dit du placement

Trois fermetures autorisées, trois fois après des refus de couches A/B :

| ticket | refus avant | autorisé (audit) | `ClosedEvent` (forge) |
|---|---|---|---|
| #2467 | 2 | 2026-09-21T14:21:24Z | 2026-09-21T14:21:02Z |
| #2573 | 2 | 2026-09-30T13:25:32Z | 2026-09-30T13:25:35Z |
| #3002 | 2 | 2026-11-01T14:46:20Z | 2026-11-01T14:46:22Z |

Le motif est le même aux trois lignes : **le refus n'est pas une protection, il
est une itération**. Deux refus, un ajustement de citation, une autorisation.
C'est une boucle d'apprentissage, et son point fixe est la fermeture.

Conséquence directe sur le placement du nouveau bras : il doit tomber **avant**
les deux couches, pas après et pas entre. Non pas pour économiser deux requêtes,
mais parce qu'un refus placé après une lecture hérite du mode de défaillance de
cette lecture — la base indisponible, l'historique illisible, la persistance des
appels désarmée. Un refus qui dépend d'une lecture est un refus qui peut, un
jour, ne pas tomber. AC2 nomme exactement cette propriété, et le placement est
le seul moyen de l'obtenir : un bras en tête ne peut pas défaillir pour une
raison qu'il n'a pas.

## 2 — L'écart de #2467, lu

Le ticket demande au vol de lire les 22 s qui séparent le `ClosedEvent`
(14:21:02Z) de l'autorisation inscrite à l'audit (14:21:24Z), et le hors-vol
précise que si cet écart passait par un chemin non gardé, AC4 devrait le
couvrir. Lecture faite à la forge (`issues/2467/timeline`, `issues/2467/comments`) :

- un seul `ClosedEvent`, acteur `mika-platform-dev`, `commit_id: null` — ni
  commit ni PR ;
- aucun `reopened` ;
- un commentaire du même compte à **14:21:01Z**, une seconde avant la
  fermeture, dont le texte est : *« Prior close attempt was blocked by doctrine
  guards (no read, then repeat-unreviewed). This close is founded on the issue
  body read (state: OPEN, no comments) and two consecutive main-branch CI
  successes. »*

Ce commentaire **est** le `--comment` de l'appel qui a fermé : il cite la
relecture (couche A) et déclare la répétition (couche B), c'est-à-dire exactement
les deux conditions que la garde exige, et il nomme ses deux refus antérieurs
dans l'ordre que l'audit enregistre (`no read`, puis `repeat-unreviewed`). La
fermeture est donc **passée par la garde, autorisée**.

Il reste que l'audit date cette autorisation 23 s après le commentaire qu'elle a
rendu possible. Un seul offset — l'horloge qui écrit `audit_events.created_at`
en avance de ~23 s sur celle de la forge — aligne les trois horodatages d'un
coup : autorisation 14:21:24 → 14:21:01 côté forge (le commentaire), et les deux
refus à 14:21:14 et avant → avant 14:20:51. Aucune autre hypothèse n'explique
les trois avec un seul paramètre.

Ce que le vol doit faire de cette lecture : **la vérifier, et la consigner**.
Elle est une inférence à un paramètre, pas une mesure de l'offset. Le critère
qui tranche est dans la base : si les deux refus et l'autorisation de #2467
portent **le même `trace_id`** et tiennent dans une fenêtre de 10 s, il s'agit
d'un seul tour, l'ordre interne est celui de la garde, et l'écart est un écart
d'horodatage — pas un chemin. Requête, à exécuter sur l'hôte, hors sandbox :

```sql
SELECT created_at, trace_id, after_value, reasoning FROM audit_events
WHERE tool_name = 'destructive_action_grounding' AND target_key = 'issue:close:2467'
ORDER BY created_at;
```

Si la base n'est pas atteignable depuis le vol, la conclusion reste celle
ci-dessus avec sa réserve nommée. Dans les deux cas elle s'écrit dans le corps de
la PR, et **pas** sous forme de certitude si la requête n'a pas tourné. Elle
borne AC4 : aucun chemin non gardé n'est en cause pour #2467, donc AC4 reste un
recensement et n'a pas à élargir la garde.

## 3 — AC1 : le bras, son libellé, son remède

Site : `crates/mika-agent/src/skills/builtin_handlers.rs`,
`validate_destructive_action_grounding` (l. 3649). Le bras s'insère juste après
la détection fail-open (`let Some(action) = detect_destructive_action(args)`) et
le calcul de `target_key`, **avant** l'appel à
`find_recent_destructive_actions` qui ouvre la couche B.

```rust
// mika#3056 — une fermeture d'issue ne se fonde pas, elle se mérite par un
// merge. Ce bras est en tête de la garde et ne lit rien : il ne peut donc
// défaillir pour aucune des raisons qui font défaillir les couches A et B.
if action.kind == DestructiveTargetKind::Issue {
    return refuse(
        ctx,
        &target_key,
        "block[issue_close_not_by_merge]",
        "issue close is not an engine gesture — no reading of the target founds it",
        /* remède : voir ci-dessous */,
        None,
    )
    .await;
}
```

`DestructiveTargetKind` s'ajoute à l'import `use crate::evidence::guards::{…}`
en tête de la fonction.

Trois propriétés tenues par cette forme, et chacune est une raison :

- **La ligne d'audit est la même.** Le bras passe par la closure `refuse`
  existante, qui écrit `audit_events` sous
  `tool_name = "destructive_action_grounding"` avec le libellé en `after_value`.
  Un second site d'écriture aurait fait du nouveau refus une population séparée,
  invisible au relevé qui a produit la mesure du ticket — et c'est ce relevé
  qu'AC5 rejouera.
- **Le `diagnostic` est `None`.** Il n'y a aucune cause technique à router vers
  l'opérateur : rien n'est indisponible, rien n'est illisible. Le refus est un
  refus de doctrine, et il est complet dans le canal du modèle.
- **Le libellé est distinct.** `block[issue_close_not_by_merge]` ne se confond
  ni avec `block[destructive_grounding]` ni avec
  `block[destructive_repeat_unreviewed]`, de sorte que le compte d'AC5 se lit
  par un `GROUP BY after_value` sans démêlage.

Le remède, dont la contrainte est de ne proposer **aucun** contournement :

> La fermeture d'un ticket n'est pas un geste du moteur. Elle vient du `Closes`
> d'une PR dont le merge a été approuvé, ou elle vient de Vincent. Aucune
> relecture, aucune citation et aucune reformulation de ce commentaire ne lève ce
> refus : ce qui manque n'est pas une preuve, c'est un merge. Si le travail est
> fait, c'est la PR qui le porte et c'est son merge qui ferme. Si le ticket doit
> être fermé sans PR, c'est à Vincent de le faire — dis-le-lui et laisse le
> ticket ouvert.

Ce qu'il ne dit pas, délibérément : aucune mention d'un autre outil, d'un autre
verbe, d'un autre tour. La leçon mika#1964 vaut ici en négatif — le modèle garde
la **conduite** (ce refus ne se lève pas), et il ne reçoit aucune piste qui
ressemblerait à une porte.

Et ce que le bras ne touche pas : `gh pr create` / `gh pr edit`, donc écrire
`Closes #N` dans un corps de PR reste permis. C'est la voie que le remède
prescrit, et il faut qu'elle reste ouverte pour que le remède soit vrai.

Le docstring d'en-tête de la garde (l. 3600–3648) est mis à jour dans le même
commit : il décrit aujourd'hui « les deux couches » comme la totalité du
mécanisme, et une garde dont la prose ne nomme pas son bras le plus fort est une
garde qu'on relâche par inadvertance au prochain passage. Une section brève —
*le bras issue, et pourquoi il précède les couches* — y entre, avec le report du
§ *Which way it fails* : la détection reste fail-open, et le fail-closed
commence désormais **avant** la première lecture pour le cas issue.

## 4 — AC3 : la fermeture d'une PR ne change pas

Le bras est conditionné sur `DestructiveTargetKind::Issue`. Une PR traverse
jusqu'aux couches A et B inchangées. Deux faits rendent ce non-changement
vérifiable sans effort :

- le fixture de test existant, `mika1964_ungrounded_close_args` (l. 7297), porte
  déjà `["pr", "close", "42", …]` — les deux tests mika#1964 qui s'appuient
  dessus (l. 7303 et l. 7360) restent verts **sans modification**, et c'est la
  première preuve qu'AC3 tient ;
- un test neuf l'affirme dans le même appel qu'AC1 (voir § 5).

Une conséquence à garder explicitement : le diagnostic `MIKA_STORE_TOOL_CALLS`
de la couche A (routé vers le canal opérateur sur tier `Family`) ne se produit
plus pour une fermeture d'**issue**, puisque la couche A n'est plus atteinte. Il
continue de se produire pour une PR. Ce déplacement est correct — il n'y a plus
rien d'indisponible à signaler quand le refus ne dépend de rien — mais il est
assez contre-intuitif pour mériter une assertion plutôt qu'une découverte.

## 5 — Les tests

Tous dans le bloc de la garde, `builtin_handlers.rs` (voisinage de la l. 7297),
avec `TestHarness` et `ctx()`.

**Un helper, quatre cas.** AC2 demande que le refus tombe dans quatre états du
monde. Les quatre partagent le même appel et la même assertion ; seul l'état
préalable change. Un helper paramétré porte l'invariant une fois :

```rust
/// Les quatre états que AC2 énumère, et l'invariant qui ne dépend d'aucun :
/// le refus tombe, et il porte le libellé du bras issue — jamais celui d'une
/// couche que le bras précède.
async fn assert_issue_close_refused(h: &TestHarness, etat: &str) { … }
```

- **relecture présente et citée** — enregistre, par `save_tool_call` (l'écrivain
  de production, motif mika#2565, cf. `record_a_shell_call` l. 14041 pour la
  forme), une ligne `run_gh` sur le `trace_id` du contexte avec
  `{"command":["issue","view","42","--json","comments"]}`, et passe un
  `--comment` qui cite la lecture. C'est le cas qui **passait** avant ce vol :
  c'est le test central du ticket.
- **relecture absente** — harness nu.
- **répétition** — enregistre une ligne `run_gh` antérieure portant
  `{"command":["issue","close","42",…]}` pour que
  `find_recent_destructive_actions` la trouverait. L'assertion porte sur le
  libellé : `block[issue_close_not_by_merge]` et **non**
  `block[destructive_repeat_unreviewed]`. C'est la preuve du placement en tête,
  et la seule façon de la faire est de rendre la couche B *prête à tirer* puis
  de constater qu'elle n'a pas tiré.
- **persistance des appels désactivée** — harness nu, tier `Family` : le refus
  tombe, et **aucune** ligne `substrate_unavailable` n'est écrite (le § 4
  ci-dessus). Sans cette moitié, « le bras précède la couche A » reste
  indistinguable de « la couche A a refusé en premier ».

Chaque cas assert, outre le libellé : `is_error`, un corps JSON parseable,
l'absence de la chaîne `MIKA_STORE_TOOL_CALLS` dans le contenu, et la présence
d'une ligne `audit_events` sous `tool_name = "destructive_action_grounding"`
dont `after_value` est le nouveau libellé — la même ligne que les autres, ce que
seul un `get_audit_events` peut attester.

**Le remède ne propose pas de contournement.** Une assertion sur le texte du
remède : il contient `Closes` et le mot *merge*, et il ne contient aucune des
formes qui ressembleraient à une porte (`retry`, `read the`, `cite`,
`--comment`). Un détecteur sur une chaîne de prose est faible ; il est tenu
quand même, parce que la seule régression plausible ici est un remède réécrit
avec de bonnes intentions pédagogiques.

**AC3, dans le même appel.** Un test unique enchaîne les deux argv sur le même
harness : `["issue","close","42",…]` refusé avec le libellé issue, puis
`["pr","close","42",…]` refusé avec `block[destructive_grounding]`. Deux tests
séparés prouveraient deux comportements ; un seul prouve qu'ils coexistent, et
c'est la formulation d'AC3 (« dans le même appel qu'AC1 »).

## 6 — AC4 : le recensement, et pourquoi il mesure au lieu de déclarer

AC4 veut la garantie qu'aucun **autre** outil servi ne fait passer un ticket à
l'état fermé. Un recensement qui énumère la surface servie et déclare, outil par
outil, « celui-là n'écrit pas à la forge » serait une liste d'affirmations
humaines : il garderait la complétude de la liste, jamais la vérité de ses
lignes. La forme retenue inverse la charge — **la table déclare les écrivains, le
test mesure le complément.**

Nouveau fichier `crates/mika-agent/src/evidence/forge_close_census.rs`, déclaré
dans `evidence/mod.rs` à côté de `guards` et `audit`.

**La table.** Une entrée par outil servi qui atteint la forge ou délègue à une
exécution arbitraire — ~18 lignes, pas 70 :

```rust
enum CloseAuthority {
    /// Atteint la forge et porte un verbe de fermeture d'issue. Doit être gardé.
    ClosesIssueDirectly,
    /// Ferme une issue, mais seulement par le `Closes` d'un merge : la voie de
    /// la doctrine.
    ClosesByMergeOnly,
    /// Atteint la forge, en lecture seule.
    ForgeReadOnly,
    /// Écrit à la forge sans jamais fermer (revue, commentaire, label).
    ForgeWriteNotClose,
    /// Exécute du code arbitraire ou délègue à une session : la fermeture, s'il
    /// en passe une, n'est pas la sienne.
    ArbitraryExecution,
}
const FORGE_CLOSE_CENSUS: &[(&str, CloseAuthority, &str)] = &[ … ];
```

La population, telle que le site la donne aujourd'hui :

- `run_gh` → `ClosesIssueDirectly`, gardé par ce vol (AC1) ;
- `pr_merge_with_gate` → `ClosesByMergeOnly` : **le seul** fichier de
  `crates/mika-agent/src/tools/` qui spawne `gh` (`Command::new("gh")`,
  `pr_merge_with_gate.rs:2284`) ;
- `gh_read`, `qa_pr_view`, `qa_head_reviews` → `ForgeReadOnly` ;
- `run_claude_pilot`, `run_claude_pilot_groom`, `address_pr_comments`,
  `resolve_pr_conflicts`, `build_mika`, `deploy_mika` → `ArbitraryExecution` ;
- `run_shell` → `ArbitraryExecution`, avec sa particularité inscrite dans la
  raison : il est **servi** (token `required_tools` de `qa-review`,
  `skills/bundled/qa-review/skill.toml:181`) mais **fourni hors de ce dépôt**
  par un skill communautaire, donc aucune mesure locale ne peut parler de lui.
  Il entre par déclaration, et la raison le dit.

**Les deux directions, et la mesure.**

1. *Pas de faux négatif, et c'est mesuré.* Le test parcourt la surface servie —
   `tools::BUILTIN_TOOL_NAMES` ∪ `bundled_skills::all_bundled_tool_names()` ∪
   les tokens `required_tools` des `skill.toml` bundled — et, pour chaque outil,
   cherche une **couture forge** dans sa source : le fichier
   `crates/mika-agent/src/tools/<nom>.rs` quand il existe (le nom d'outil est le
   nom de fichier : `pr_merge_with_gate.rs`, `a2a_call.rs`), sinon les fichiers
   du bundle qui le déclare, via `bundled_skill_tool_map()` et
   `skill.files` — lesquels embarquent les `handlers/*.sh` (`build.rs` les
   inclut), donc la mesure se fait sur des constantes compilées et non sur un
   checkout. Coutures cherchées : `Command::new("gh")`, `gh_process`,
   `octocrab`, `github_graphql`, `api.github.com`, `gh issue`, `gh pr`. Tout
   outil à couture absent de la table fait rougir. Le complément
   (« n'atteint pas la forge ») n'est pas énuméré : il est **défini** par
   l'absence de couture, ce qui est une mesure et non une affirmation.
2. *Auto-nettoyant.* Toute entrée dont le nom n'est plus dans la surface servie
   fait rougir. Une table qui survit à l'outil qu'elle décrit est le mode de
   mort habituel de ce genre de garde (mika#2092, mika#2309).
3. *L'invariant d'AC4, nommé.* L'ensemble des entrées en `ClosesIssueDirectly`
   est exactement `{"run_gh"}`, et celui des `ClosesByMergeOnly` exactement
   `{"pr_merge_with_gate"}`. Un troisième fermeteur arrive donc par une
   assertion rouge et non par une relecture, et la règle qu'AC4 énonce
   (« ce chemin reçoit le même refus ») s'applique à lui au ticket qui l'ajoute.

**Ce que le recensement constate, et qui n'est pas réparé par ce vol.**
`run_shell` et les six outils `ArbitraryExecution` peuvent, par construction,
exécuter `gh issue close` sans passer par la garde argv de `run_gh`. Le vol ne
les garde pas, pour deux raisons mesurées :

- la mesure du ticket est nette — les 9 tentatives vues par la garde sont toutes
  passées par `run_gh`, et le § 2 ci-dessus montre que #2467, le seul cas dont
  l'horodatage suggérait un autre chemin, est passé par la garde lui aussi.
  Aucune fermeture mesurée n'a emprunté une exécution arbitraire ;
- un détecteur sur une ligne de shell se tromperait sur le chemin le plus chaud
  de la boucle : les appels `run_shell` de `qa-review` **embarquent le corps de
  PR verbatim** (`skills/executor.rs:136`), de sorte qu'une PR dont le corps
  contient la chaîne `gh issue close 1234` provoquerait un refus sur un geste
  qui n'en est pas un. Garder ce chemin demande une tokenisation qui distingue
  un argv d'un heredoc — un périmètre, pas un bras.

Ce reste va en commentaire sur mika#3056 et dans le corps de la PR, le gel des
tickets interdisant d'ouvrir un suivi. Le recensement le rend visible et daté,
ce qui est précisément ce qu'AC4 demande d'un recensement.

## 7 — AC5 : la sonde

Hors code. À +14 jours du deploy, relevé et écrit en commentaire sur mika#3056 :

- les `ClosedEvent` d'issue sur `mika`, `mika-skills`, `mika-cloud`,
  `claude-pilot`, dont l'acteur est un compte d'agent et dont le `closer` n'est
  pas une PR (`timeline` + `closed_by`) — cible **0** ;
- à côté, le compte des refus `block[issue_close_not_by_merge]` :
  `SELECT COUNT(*) FROM audit_events WHERE tool_name = 'destructive_action_grounding' AND after_value = 'block[issue_close_not_by_merge]' AND created_at >= '<deploy>'`.

Un compte nul **sans aucune tentative** s'écrit « aucune tentative sur la
fenêtre » : zéro refus sur zéro tentative mesure le silence, pas la garde, et
les deux ne doivent pas se lire pareil.

## Fire-Disposition

Le plan livre des détecteurs : les tests d'AC1–AC3, et le recensement d'AC4 dont
le chemin de succès est « aucun outil servi à couture forge n'est absent de la
table ».

**Option (a) — exception nommée en allowlist.** La table `FORGE_CLOSE_CENSUS`
**est** l'allowlist : une entrée par outil, chacune nommant la donnée précise
(le nom d'outil servi), sa classe, et sa raison en clair. Grep-visible par
construction (`FORGE_CLOSE_CENSUS`, un outil par ligne).

- **Population des violations à l'atterrissage : zéro.** Le vol classe la
  surface entière avant de poser l'assertion, donc le détecteur atterrit **armé
  et vert**. Il n'y a pas d'exception « en attente » : chaque entrée décrit un
  état voulu du site, pas une dette.
- **Assertion auto-nettoyante.** Direction 2 du § 6 : une entrée dont l'outil
  n'est plus servi fait rougir le test. Une exception ne peut donc pas devenir
  stale en silence — le geste qui retire l'outil est le geste qui périme
  l'entrée.
- **Référence de ticket.** Le gel des tickets (jusqu'au 01/12) interdit d'ouvrir
  un suivi : chaque entrée référence `mika#3056` et, pour celles qui portent une
  limite connue (`run_shell`, `ArbitraryExecution`), la raison cite la mesure
  qui la borne. C'est une dérogation assumée au « référence un ticket de suivi »
  de mika#1574, imposée par le gel et nommée ici pour ne pas être lue comme un
  oubli.

## Definition of Done

- `cargo build` et `cargo clippy --all-targets -- -D warnings` propres.
- `cargo test -p mika-agent` vert, dont les deux tests mika#1964 préexistants
  (l. 7303, l. 7360) **inchangés**.
- `make verify-bundled-skills` vert (aucun bundle touché, gate de non-régression).
- `bash scripts/verify-pipeline.sh origin/main` vert.
- La lecture de #2467 (§ 2) consignée dans le corps de la PR, avec sa réserve si
  la requête `audit_events` n'a pas pu tourner.
- Le reste d'AC4 (`run_shell`, exécutions arbitraires) écrit en commentaire sur
  mika#3056 et rappelé dans le corps de la PR.
- Corps de PR portant `Closes #3056` et
  `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

- **AC1.** Une fermeture d'issue reconnue par le détecteur est refusée, quelle
  que soit la relecture. Le refus porte un libellé de bloc distinct
  (`block[issue_close_not_by_merge]`), écrit sur la même ligne d'audit que les
  autres. Son remède dit, sans proposer de contournement, que la fermeture vient
  du `Closes` d'un merge approuvé, ou sinon d'un geste de Vincent.
- **AC2.** Fail-closed. Le refus ne dépend d'aucune lecture : pas de base, pas
  de forge, pas d'historique des appels. Un test garde chacun de ces cas :
  relecture présente et citée ; relecture absente ; répétition ; persistance des
  appels désactivée. Dans les quatre cas, le refus tombe.
- **AC3.** La fermeture d'une PR (`pr close`) garde son comportement actuel,
  couches A et B. Un test le garde dans le même appel qu'AC1.
- **AC4.** Aucun autre outil servi aux agents ne fait passer un ticket à l'état
  fermé. Un test énumère les outils servis qui écrivent à la forge, et garde ce
  fait. Si le vol en trouve un, ce chemin reçoit le même refus.
- **AC5.** Sonde : sur les 14 jours qui suivent le deploy, on compte les
  `ClosedEvent` d'issue dont l'acteur est un compte d'agent et dont le `closer`
  n'est pas une PR, sur les dépôts de la boucle, puis on l'écrit ici. La cible
  est 0. On écrit à côté le nombre de refus
  `block[issue_close_not_by_merge]`. Un compte nul sans aucune tentative s'écrit
  « aucune tentative sur la fenêtre ».

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/skills/builtin_handlers.rs` — bras AC1 + docstring de la garde | 40 |
| `crates/mika-agent/src/evidence/forge_close_census.rs` — table + mesure + tests (AC4) | 175 |
| tests AC2/AC3 (`builtin_handlers.rs`, bloc de la garde) | 150 |

Total estimé : 365 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Un seul vol, et voici pourquoi il tient : 365 < 429, et la part de code *neuf à
concevoir* est petite — le bras AC1 est un `if` de six lignes dans une fonction
existante qui fournit déjà sa closure `refuse` et sa ligne d'audit, et les tests
AC2 réutilisent le harness et les deux helpers déjà écrits (`TestHarness`,
`save_tool_call`). Le seul livrable réellement neuf est le recensement d'AC4
(175), dont 22 lignes sont une table de données et dont la mesure réutilise
`BUILTIN_TOOL_NAMES`, `all_bundled_tool_names()` et `bundled_skill_tool_map()`
sans les étendre. Aucun découpage en vols n'est requis.

## Ordre de commit

1. `fix(3056): la garde refuse toute fermeture d'issue, et ne lit rien pour le
   faire` — le bras AC1, le libellé, le remède, le docstring de la garde mis à
   jour. **Plancher :** `cargo build` + `clippy` propres ; les deux tests
   mika#1964 préexistants verts sans modification (c'est déjà AC3 en négatif).
2. `test(3056): les quatre états fail-closed, et la PR qui garde son
   comportement` — le helper, les quatre cas d'AC2, le test conjoint d'AC3, les
   assertions d'audit et de remède. **Plancher :**
   `cargo test -p mika-agent skills::builtin_handlers` vert.
3. `test(3056): le recensement des outils servis qui atteignent la forge` — le
   module `forge_close_census`, la table, les deux directions, l'invariant
   d'AC4. **Plancher :** `cargo test -p mika-agent forge_close_census` vert, et
   `cargo test -p mika-agent` entier vert.

Chacun des trois est mergeable seul : (1) livre le refus, (2) le prouve, (3)
borne la surface. Si le vol devait s'arrêter, il s'arrête sur un plancher vert
et le ticket dit lequel.
