# Plan : la porte de promotion sépare les deux populations par ce que les commits **touchent**, pas par leur nombre (mika#2140)

**Ticket :** mika issue#2140 — `fix(auto_pull): la porte de promotion lit `ahead_by > 1` comme du travail de pilote, alors que le grooming produit légitimement 2-3 commits de plan`
**Labels :** `bug`, `p1-important`
**Type :** issue (bug — casseur de boucle : la porte retire du bassin `ready` les tickets les plus travaillés)
**Palier de priorité :** Tier 1 — *casse la boucle*. Bassin `ready` à 1 sur un plancher de 3, avec deux candidats groomés gatés à tort.
**Fichier principal :** `crates/mika-agent/src/auto_pull.rs`

---

## Problème

`classify_promotion` (`auto_pull.rs:680`) sépare « branche de grooming pur » de « branche portant du travail d'un pilote mort » par un **compte de commits** :

```rust
if staleness.ahead_by > 1 {
    return PromotionGate::Refuse(RefusalReason::SalvageWorkOnStaleBranch { … });
}
```

L'hypothèse est écrite au module (`auto_pull.rs:37-39`) :

> `ahead_by` separates the two populations for free: a branch carrying only its plan has `ahead_by == 1`; every branch that died on 2026-08-31 carried more.

Elle est fausse sur le **chemin nominal** du grooming. `.claude/commands/mika-groom-ticket.md` commite le plan à trois sites distincts — Phase 3 étape 10, Phase 4 étape 12, Phase 5 étape 17 — et c'est délibéré : la lignée doit rester lisible entre « l'architecte a signé » et « l'opérateur a rédigé » (Phase 2 étape 7). Tout ticket ayant demandé un aller-retour architecte porte donc `ahead_by ∈ {2,3}` sans qu'aucun pilote ne l'ait touché.

C'est la troisième récidive en 48 h de la même forme — *un garde encode une hypothèse sur ce que son producteur produit, et le producteur produit légitimement autre chose* — après `is_groomed` et `dispatch-lib.sh:4405` (mika#2120). Précédent applicable : `docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md`.

## Mesures — exécutées le 2026-09-03, pas déduites

Toutes les lignes ci-dessous viennent d'appels réels à `GET /repos/senara-solutions/mika/compare/main...<branch>`.

### M1 — les deux faux positifs, et le fait que l'API porte déjà le signal manquant

```
fix/2118/skills-cloud-sur-un-tenant-cloud-google
  {"status":"diverged","ahead_by":3,"behind_by":8,"total_commits":3,
   "files":["docs/plans/2026-09-01-003-fix-2118-gws-cloud-design-limit-plan.md"]}

fix/2120/auto-pull-is-groomed-exige-docs-plans
  {"status":"diverged","ahead_by":2,"behind_by":8,"total_commits":2,
   "files":["docs/plans/2026-09-01-004-fix-2120-is-groomed-repo-prefix-plan.md"]}
```

`behind_by > 0` et `ahead_by > 1` → la porte actuelle rend `SalvageWorkOnStaleBranch` sur les deux. Le diff complet de chacune contre `main` est **un seul fichier**, sous `docs/plans/`. Les deux tickets portent encore `operator-gated` et sont encore `OPEN` (vérifié 2026-09-03).

### M2 — le contrôle négatif existe et il est net

```
fix/1680/mika-dev-tui-broken-glyph-rendering-in
  {"status":"diverged","ahead_by":2,"behind_by":197, "files":[
     "crates/mika-agent/src/agent_loop/mod.rs",
     "crates/mika-agent/src/evidence/guards.rs",
     "crates/mika-agent/src/evidence/mod.rs",
     "crates/mika-agent/src/well_known_agents.rs",
     "docs/plans/2026-06-30-016-fix-1680-mika-dev-cn-output-bleed-plan.md"]}
```

Quatre fichiers de code **plus** un fichier de plan. C'est exactement la branche morte du 2026-08-31 dont le vrai `git rebase origin/main` conflit sur `agent_loop/mod.rs` et `evidence/guards.rs` (mesuré le 2026-09-01, `tests/fixtures/auto_pull_compare/PROVENANCE.md`). Le prédicat par fichiers la refuse toujours.

### M3 — le préfixe est bien `docs/plans/`, sans préfixe de dépôt

Le point qui a fait tomber le garde frère (mika#2120) : `is_groomed` exigeait `docs/plans/` là où la spec écrivait `mika/docs/plans/`. Ici la question est tranchée par mesure, pas par lecture de spec — **l'endpoint `compare` rend des chemins relatifs à la racine du dépôt** : `docs/plans/2026-09-01-004-…`, jamais `mika/docs/plans/…`. Le préfixe littéral `docs/plans/` est donc correct *à cette frontière-là*, et le plan écrit pourquoi à côté de la constante pour que le prochain lecteur n'ait pas à refaire l'appel.

### M4 — la forme des objets de `files`

```
$ gh api …/compare/main...fix/2120/… --jq '.files[0]|keys'
["additions","blob_url","changes","contents_url","deletions","filename","patch","raw_url","sha","status"]
```

Le seul champ lu sera `filename`. Les fixtures gelées ne garderont que celui-là (les `patch` font des mégaoctets — c'est déjà la raison pour laquelle `files` avait été retiré des fixtures de mika#2123).

### M5 — l'état actuel des fixtures gelées, qui est un piège

Les quatre fixtures de `tests/fixtures/auto_pull_compare/` ont été gelées **sans** la clé `files` (PROVENANCE : *« `commits`, `files`, `base_commit` and `merge_base_commit` were dropped »*). Sous le nouveau prédicat, une fixture sans `files` tombe dans le chemin « liste indisponible » → jamais `salvage`. Le test d'intégration `auto_pull_replay_1680_is_refused_by_name` deviendrait vert **pour la mauvaise raison** (il basculerait sur `branch_too_far_behind`, 197 > 50) et son assertion `slug() == "salvage_work_on_stale_branch"` passerait au rouge. Les fixtures doivent donc être ré-enrichies, pas laissées telles quelles. C'est un livrable, pas un détail.

## Décision de conception

**Le prédicat de `salvage` devient : « la branche modifie au moins un fichier hors `docs/plans/` par rapport à `main` ».** `ahead_by` cesse d'être le discriminant et redevient ce qu'il est — une distance, journalisée, jamais interprétée.

### Ordre des règles, et pourquoi la troncature ne demande pas de branche à part

```
non_plan = files.filter(|f| !f.starts_with("docs/plans/"))
si non_plan est non vide            → Refuse(SalvageWorkOnStaleBranch { non_plan })
sinon                                → pas de salvage ; on tombe sur la règle de distance
```

La troncature de l'API est **déjà** traitée par cet ordre, et c'est ce qui rend AC4 gratuit plutôt qu'ajouté :

- Si la liste tronquée contient déjà un fichier hors plan, le fait recherché est acquis — la troncature ne peut pas le retirer. Refus.
- Si tous les fichiers visibles sont sous `docs/plans/`, la seule ignorance possible est « il y avait peut-être du code plus loin » → la branche n'est pas classée `salvage` → **elle promeut**, conformément à l'invariant déjà écrit au module (`auto_pull.rs:25-29` : *« Every "could not measure" outcome promotes »*) et au fait que le vrai rebase tranche de toute façon au dispatch.

Une liste `files` absente ou non-tableau prend exactement le même chemin, pour la même raison.

**Ce que ça coûte, dit franchement.** Une branche portant du travail de pilote dont l'API ne rendrait pas les fichiers, et qui serait sous le seuil de distance, promeut désormais. C'est le prix explicite de l'invariant fail-open de ce module ; le refuser reviendrait à faire de `auto_pull` la seule porte du dépôt qui se ferme quand GitHub hoquette.

### AC6 — la levée d'`operator-gated` reste **manuelle**, et voici pourquoi

Trois raisons, dont deux sont structurelles :

1. **L'auto-levée s'auto-contredit.** `is_feeder_excluded` (`auto_pull.rs`) exclut tout ticket portant `operator-gated` des **trois** phases. Une porte qui se relit elle-même devrait d'abord ré-évaluer des tickets que sa propre exclusion lui interdit de regarder — c'est-à-dire retourner l'exclusion contre son objet.
2. **La machine ne peut pas distinguer son label de celui de l'opérateur.** `operator-gated` n'est pas un label machine : sa description déclarée (`.github/labels.yml:106`) est *« Groomed work requiring operator-host-time. Distinct from parked/blocked. No ready label. »* — un geste d'opérateur légitime. Une porte qui le retirerait dé-gaterait silencieusement du travail qu'un humain a gaté. Ce serait très exactement la faute que ce ticket corrige : un lecteur qui suppose ce que son producteur a produit.
3. **Le canal existe déjà.** Le commentaire de refus dit déjà *« puis retire le label `operator-gated` »* (`RefusalReason::comment_body`). Ce qui manquait n'était pas le geste mais sa *raison lisible* — AC5 la fournit en nommant les fichiers.

Ce choix est écrit **dans le module**, à côté de `REFUSAL_LABEL`, pas seulement ici : un label posé par la machine et levable seulement à la main est une dette d'opérateur silencieuse tant qu'elle n'est pas nommée à l'endroit où on la lit.

**Remédiation des deux tickets déjà gatés à tort** (`#2118`, `#2120`) : geste d'opérateur unique après merge — retirer `operator-gated`. Listé en § Suite opératoire ; hors du diff, parce qu'un correctif de code n'a pas à muter l'état de tickets tiers pour prouver qu'il marche. Les fixtures AC3 sont ce qui le prouve.

## Deliverables

### D1 — `BranchStaleness` porte la liste des fichiers

`crates/mika-agent/src/auto_pull.rs` (~`:419`) :

```rust
pub struct BranchStaleness {
    pub behind_by: i64,
    pub ahead_by: i64,
    pub status: String,
    /// Les chemins que la branche modifie par rapport à `main`, tels que
    /// l'endpoint `compare` les rend (relatifs à la racine du dépôt — mesuré,
    /// voir le plan mika#2140 M3). `None` quand la clé `files` est absente ou
    /// n'est pas un tableau : « je n'ai pas pu lire », jamais « il n'y a rien ».
    pub changed_files: Option<Vec<String>>,
}
```

`parse_compare_payload` remplit le champ à partir de `files[].filename`, en ignorant les entrées sans `filename` (et en rendant `None` si `files` manque). **Les trois champs existants restent obligatoires** — leur absence reste une erreur de parse, contrat inchangé.

### D2 — la constante du préfixe, documentée à sa mesure

```rust
/// Le seul préfixe qu'une branche de grooming pur modifie (mika#2140).
/// Relatif à la racine du dépôt : `compare` rend `docs/plans/…`, jamais
/// `mika/docs/plans/…` — mesuré 2026-09-03, et c'est précisément l'écart
/// qui a fait tomber `is_groomed` (mika#2120).
const PLAN_PATH_PREFIX: &str = "docs/plans/";
```

Plus une fonction pure et testable :

```rust
pub fn non_plan_files(changed: Option<&[String]>) -> Vec<String>
```

`None` → vecteur vide (le fail-open, par construction et non par branche `if` séparée).

### D3 — `classify_promotion` change de prédicat, pas de forme

Le bloc `if staleness.ahead_by > 1` (`:680`) devient :

```rust
let non_plan = non_plan_files(staleness.changed_files.as_deref());
if !non_plan.is_empty() {
    return PromotionGate::Refuse(RefusalReason::SalvageWorkOnStaleBranch {
        branch,
        behind_by: staleness.behind_by,
        ahead_by: staleness.ahead_by,
        non_plan_files: non_plan,
    });
}
```

Position dans l'ordre des règles **inchangée** : avant la règle de distance, pour la raison déjà écrite (`:625-627`) — c'est le fait le plus spécifique sur la même branche, et son remède est différent.

### D4 — AC5 : le refus nomme les fichiers

`RefusalReason::SalvageWorkOnStaleBranch` gagne `non_plan_files: Vec<String>`. `reason()` cesse de parler de compte et parle de contenu :

> La branche `<b>` de #`<n>` est en retard de **N commits** et modifie **K fichier(s) hors `docs/plans/`** — du travail qui n'est pas du grooming : `a.rs`, `b.rs`, …

La liste est bornée à **10 chemins** suivis de « … et P autres » : un commentaire GitHub ne doit pas devenir un `git diff --stat`. `remedy()` garde sa formulation actuelle (le choix porte sur du travail, pas sur git) — elle reste exacte.

### D5 — AC4/AC1 : l'audit dit ce qu'il a vu

`staleness_audit_json` gagne deux champs sur les décisions `Measured` :

- `"changed_files_count"` — `null` quand la liste est indisponible, jamais `0` (même discipline que `behind_by` sur les issues non mesurées).
- `"non_plan_files"` — le tableau des chemins hors plan (borné à 10, comme le commentaire).

`ahead_by` **reste émis** : il n'est plus un discriminant, il redevient une mesure, et la promesse de KTD2c (réviser le seuil depuis une distribution réelle) en dépend.

### D6 — le module cesse d'affirmer une chose fausse

Le paragraphe `auto_pull.rs:37-39` (*« `ahead_by` separates the two populations for free »*) est la phrase qui a causé le défaut. Elle est remplacée par l'énoncé du nouveau prédicat, la mention explicite que le grooming produit 2–3 commits de plan par conception, et la référence à mika#2140. La partie qui reste **vraie** — *pourquoi* une branche portant du travail ne doit pas être rebasée en silence (deux résolutions légitimes, jugement sur du travail et non sur git) — est conservée telle quelle : ce ticket ne conteste pas la porte, il corrige son prédicat.

### D7 — fixtures : ré-enrichissement + deux nouvelles

`crates/mika-agent/tests/fixtures/auto_pull_compare/` :

| fixture | action | `files` (filename seul) |
|---|---|---|
| `1680-diverged-180-behind-2-ahead.json` | **enrichir** | 4 fichiers `crates/**` + 1 `docs/plans/**` (M2) |
| `1959-diverged-75-behind-1-ahead.json` | enrichir | 1 `docs/plans/**` |
| `2048-diverged-17-behind-1-ahead.json` | enrichir | liste réelle capturée |
| `2123-ahead-0-behind-1-ahead.json` | enrichir | liste réelle capturée |
| `2118-diverged-8-behind-3-ahead.json` | **nouvelle** | 1 `docs/plans/**` (M1) |
| `2120-diverged-8-behind-2-ahead.json` | **nouvelle** | 1 `docs/plans/**` (M1) |

`PROVENANCE.md` est mis à jour et doit porter **une honnêteté explicite** : pour les quatre fixtures existantes, les compteurs datent du 2026-09-01 et la liste `files` du 2026-09-03. Ce n'est pas une incohérence, et la raison est vérifiable : `compare/main...branch` est un diff à trois points, donc `files` est relatif à la **base de fusion**, qui ne bouge pas quand `main` avance — seul `behind_by` bouge (1680 : 180 → 197 le 2026-09-03, `ahead_by` inchangé à 2, liste inchangée). La ligne doit être dans le fichier, pas seulement dans ce plan.

### D8 — tests

**Unitaires** (`auto_pull::tests`). L'aide `measured(behind, ahead, status)` conserve sa signature et rend `changed_files: None` ; une aide `measured_files(behind, ahead, status, &[…])` est ajoutée. Trois tests existants doivent migrer vers `measured_files` — ils reposaient sur `ahead_by` pour obtenir `salvage` :

1. `test_promotion_gate_salvage_work_refuses_independently_of_threshold` (deux `classify_promotion` : `measured(1,2,…)` et `measured(180,2,…)` avec seuil `0`)
2. `test_staleness_audit_json_is_structured_on_promote_and_refuse` (le bloc `measured(180, 2, "diverged")` qui attend `reason == "salvage_work_on_stale_branch"`)

Non affectés : `behind_but_within_threshold` (`ahead=1`), `too_far_behind` (`ahead=1`), `threshold_zero_disables`, `fails_open`, `absent_branch`, `refusal_label_is_declared`.

Nouveaux tests unitaires :

- `test_non_plan_files_partitions` — `None` → vide ; que du plan → vide ; code seul → code ; code + plan → code seul. Contrôle négatif inclus : un chemin qui *ressemble* (`docs/plansible/x.md`) compte comme hors plan (le préfixe est littéral, `starts_with` sur `docs/plans/` avec la barre finale).
- `test_promotion_gate_multi_commit_plan_only_promotes` — **AC1**, le cœur : `measured_files(8, 3, "diverged", &["docs/plans/x.md"])` → `Promote { detail: "behind_within_threshold" }`. Le même appel avec l'ancien prédicat refusait.
- `test_promotion_gate_code_on_stale_branch_still_refuses` — **AC2** : `measured_files(8, 3, "diverged", &["crates/a.rs", "docs/plans/x.md"])` → `Refuse(salvage)`, y compris avec le seuil désactivé (`0`), donc indépendamment de la distance.
- `test_promotion_gate_missing_file_list_promotes` — **AC4** : `measured(8, 3, "diverged")` (liste `None`) → `Promote`, et l'audit porte `changed_files_count: null`.
- `test_salvage_refusal_names_the_offending_files` — **AC5** : `reason()` et `comment_body()` contiennent `crates/mika-agent/src/agent_loop/mod.rs` ; la troncature à 10 est exercée avec 12 chemins et l'assertion porte sur « … et 2 autres ».
- `test_parse_compare_payload_files_absent_is_none` + `…_reads_filenames` — le contrat de parse dans les deux sens.

**Intégration** (`crates/mika-agent/tests/auto_pull_promotion_gate.rs`) — **AC3**, sur des corps réels :

- `auto_pull_replay_2118_promotes` et `auto_pull_replay_2120_promotes` : chaque fixture rend `Promote`.
- **Non-vacuité**, exigée parce qu'un test de régression qui aurait aussi passé avant ne prouve rien : chacun des deux asserte d'abord que la fixture est bien dans la zone de refus de l'ancien prédicat — `behind_by > 0 && ahead_by > 1` — puis que la décision est `Promote`. Le test échoue donc si quelqu'un remplace la fixture par une branche à `ahead_by == 1`.
- `auto_pull_replay_1680_is_refused_by_name` : conservé, et **renforcé** — l'assertion passe de « le slug est `salvage` » à « le slug est `salvage` **et** le refus nomme `agent_loop/mod.rs` ». C'est la fixture dérivée d'une branche morte du 2026-08-31 que demande AC3.
- Le test à seuil `0` (`tests/auto_pull_promotion_gate.rs:96`) est conservé : il prouve que les deux règles restent indépendantes.

### D9 — compound

Une entrée `docs/solutions/architecture-patterns/` étendant le précédent nommé par le ticket, sur la forme récidivante (3 occurrences en 48 h) : **un garde ne doit pas encoder une hypothèse sur la *forme* de ce que son producteur produit ; il doit lire la *substance*.** Le compte de commits est une forme, la liste des fichiers est une substance. Rédigée en `/ce:compound` à la fin du pipeline, pas ici.

## Hors périmètre

Repris du ticket, sans extension :

- `MAX_BEHIND` et `TooFarBehind` — hors cause, les deux branches sont à 8 de retard.
- L'aveuglement de préfixe d'`is_groomed` — c'est mika#2120, déjà groomé.
- Le fait que `/mika-groom-ticket` produise plusieurs commits de plan — **pas** un défaut : c'est le lecteur qui doit tolérer ce que son producteur produit.
- La réparation de `operator-review` (48 lignes `not found` en production) — chemin mika#2020, cité par `REFUSAL_LABEL` et laissé où il est.

## Risques et contre-mesures

| Risque | Contre-mesure |
|---|---|
| **Rendre la porte permissive rouvre la porte qu'elle ferme** (la faute que l'AC2 de mika#2120 nomme sur l'autre garde) | AC2 est un test, pas une intention : `crates/**` + plan sur branche stale → `Refuse`, y compris seuil désactivé. Et la fixture #1680 est un corps réel dont le rebase a **réellement** conflit. |
| Les fixtures existantes sans `files` verdissent les tests pour la mauvaise raison | D7 les ré-enrichit ; D8 renforce l'assertion #1680 pour qu'elle porte sur les fichiers nommés, ce qu'aucune fixture sans `files` ne peut satisfaire. |
| Un test de régression AC3 vacieux (la fixture promeut aussi sous l'ancien code) | Assertion de non-vacuité explicite : `behind_by > 0 && ahead_by > 1` avant la décision. |
| Un préfixe trop étroit refuse une branche de grooming légitime touchant autre chose | Mesuré : les deux branches groomées ne touchent **que** `docs/plans/` (M1), et la spec de grooming ne commite que le fichier de plan (Phase 2 étape 7, Phase 4 étape 12, Phase 5 étape 17). Le coût d'une étroitesse est un refus lisible qui nomme le fichier — pas un dispatch perdu. |
| `changed_files` ajouté à une struct publique casse un site de construction | Trois sites au total dans le dépôt (`:419` déclaration, `:609` parse, `:2545` aide de test) — grep exécuté le 2026-09-03, aucun consommateur hors `auto_pull.rs`. |

## Critères d'acceptation — traçabilité

| AC | Livrable | Preuve |
|---|---|---|
| AC1 — refus seulement si ≥1 fichier hors `docs/plans/` | D2, D3 | `test_promotion_gate_multi_commit_plan_only_promotes` ; replays #2118/#2120 |
| AC2 — contrôle négatif : code + plan sur branche stale → refus | D3 | `test_promotion_gate_code_on_stale_branch_still_refuses` (seuil 50 **et** 0) ; replay #1680 |
| AC3 — régression sur les corps réels | D7, D8 | fixtures `2118-…`, `2120-…` → `Promote` (avec non-vacuité) ; `1680-…` → `Refuse` |
| AC4 — liste tronquée/indisponible → promotion | D1, D2, D5 | `test_promotion_gate_missing_file_list_promotes` ; `non_plan_files(None) == []` par construction |
| AC5 — le refus nomme les fichiers hors plan | D4 | `test_salvage_refusal_names_the_offending_files` (+ troncature à 10) |
| AC6 — levée choisie et écrite | Décision de conception ; D6 | la levée est **manuelle**, justifiée en trois points et écrite dans le module à côté de `REFUSAL_LABEL` |

## Suite opératoire (hors diff, après merge)

Geste d'opérateur unique, à faire une fois le correctif déployé : retirer `operator-gated` de `#2118` et `#2120`. Les deux branches restent en retard de 8 commits — sous le seuil de 50 — et ne portent que leur plan ; la porte corrigée les promeut alors d'elle-même au tick suivant, sans intervention supplémentaire. Poser `ready` reste, comme toujours, une action d'opérateur.

## Vérification

```bash
cargo test -p mika-agent auto_pull
cargo test -p mika-agent --test auto_pull_promotion_gate
cargo clippy -p mika-agent --all-targets -- -D warnings
cargo fmt --check
```

Contrôle de non-vacuité à exécuter et à reporter dans le corps de la PR, pas à supposer : rétablir `if staleness.ahead_by > 1` à la place du nouveau prédicat doit faire **rougir** `test_promotion_gate_multi_commit_plan_only_promotes` et les deux replays #2118/#2120 — et laisser vert le replay #1680.
