---
module: mika-agent/server
tags: [ci-success-handler, merge-ready, fan-out, agent-identity, dedup, loop-substrate, forge-gate]
problem_type: missing-entry-gate
category: bug
type: fix
issue: senara-solutions/mika#2260
branch: bug/2260/loop-substrate-double-chemin-de-merge-le
status: groomed
---

# fix mika#2260 — la transition évaluer→merger n'appartient qu'au dispatcher : porte à l'ENTRÉE de `ci_success_handler`

**Issue :** senara-solutions/mika#2260 (p2, bug, loop-substrate)
**Branche :** `bug/2260/loop-substrate-double-chemin-de-merge-le`
**Lignée :** mika#1711 (fan-out `check_suite` vers mika-qa) → mika#1869 (dedup burst) → mika#2248/PR#2258 (signal, pas acteur) → **ce ticket** (le relecteur n'entre plus)

> **Re-dérivé le 2026-09-30 contre `main` @ `7455023b`.** Le nom du fichier porte
> la date du premier grooming (2026-09-11, architecte READY en première passe,
> session `868b51f2`) ; le corps est celui de la re-mesure. Le chemin est conservé
> à dessein — un second plan pour la même issue ajouterait une ambiguïté gratuite
> à `_find_issue_plan` et laisserait un plan périmé dans l'arbre. Ce que la
> re-mesure a changé est isolé au § suivant, pour qu'un relecteur n'ait pas à
> diffuser les deux versions.

## Ce que la re-mesure contre `main` change — et c'est le premier livrable

Le commentaire opérateur du 2026-09-30 demande de re-mesurer les prémisses avant
l'architecte. Fait, ancre par ancre. **Le fond du plan tient ; sept points de sa
lettre étaient faux ou périmés, dont un qui l'aurait fait construire sur un
montage que le dépôt condamne par écrit.**

### Les prémisses qui tiennent

| prémisse | mesure du 2026-09-30 |
|---|---|
| `owns_merge_transition` n'existe pas | `grep -rn owns_merge_transition crates/` ⇒ zéro hit hors ce plan. **Non implémenté.** |
| `ci_success_handler.rs` inchangé | dernier commit `189836ca` (2026-09-09, PR#2258) — **antérieur au plan**. Le fichier est **identique** à ce que le plan a lu. |
| `forge_identity` porte la table d'acteurs | `merge_disposition`, `MergeDisposition::Act`, `DISPATCHER_AGENT` intacts. Seul ajout depuis : `DISPATCHER_FORGE_LOGIN` (mika#2334, 16/09), sans effet ici. |
| le dedup est global et sans `agent_id` | `check_suite_dedup.rs:59-61` — **citation exacte à la ligne**. |
| le fan-out vers mika-qa existe | `mika-gateway/src/github.rs:366` — `("check_suite","completed","success") => &["mika-qa"]`. |
| la seconde ceinture tient | `authorize_merge` : `WouldMergeAsReviewer` (`:82`) puis `NotDispatcher` (`:85`). D3 inchangé. |
| `db.agent_id()` est disponible | `async_db.rs:129`, `pub fn agent_id(&self) -> &str`. D2 implémentable tel quel. |
| le helper d'audit existe | `assert_audit_event_name_is_real` — `tests/eval/test_ci_success_handler.rs:73`, `pub`. |

Les deux tickets que le commentaire opérateur nomme n'invalident rien :
**mika#2244** est l'incident *fondateur* de #2248, donc antérieur ; **mika#2338**
(`1732dfcf`, review-anchor) touche le comparateur de mika-arch, pas cette zone —
il change la sévérité de la *revue de ce plan*, pas ses prémisses (voir R2).

### R1 — le montage de test prescrit est celui que le dépôt condamne (rectification principale)

`crates/mika-agent/tests/eval/multi_agent.rs`, **274 lignes livrées le 2026-09-09**
par mika#2265/PR#2274 — donc **deux jours avant le premier grooming**, qui ne le
cite pas. Son doc-comment nomme **ce ticket** :

> « la classe de défauts du 2026-09-09 (mika#2248, **#2260**, #2263) n'est pas
> « le mauvais résultat » : c'est « le bon résultat, sous le mauvais acteur », ou
> « deux acteurs là où un seul était prévu ». Le mot load-bearing est donc
> **attribution**, et c'est exactement ce que ce module rend mesurable. »

Et il porte une section « ## Le montage à NE PAS refaire » qui condamne
littéralement ce que la première version de ce plan prescrivait :

> « Deux `AsyncDatabase::new_with_agent(Database::open_in_memory(), …)` produisent
> deux mémoires **disjointes** [...] C'est la forme qu'a prise `db_for_agent` dans
> `test_merge_identity_2248.rs` — correct là-bas [...] — mais généralisée telle
> quelle elle livrerait un harness qui ne voit pas plus que l'actuel. »

Le plan disait : « base en mémoire portée par `agent_id` (`db_for_agent`, forme
déjà présente dans `merge_ready_handler.rs`) ». **Remplacé** par
`MultiAgentHarness`, qui apporte trois choses que `db_for_agent` ne peut pas :

- `audit_counts_by_agent(tool_name)` — **la sonde d'AC1 et d'AC5** : une carte à
  deux clés dit « deux agents ont atteint ce callsite », une carte à une clé dit
  qu'un seul l'a atteint. C'est très exactement l'énoncé du ticket.
- le patron **course** (`tokio::join!`), documenté avec sa raison : « #2248 est un
  *ordre* et #2260 est une *course* ». Le double chemin *est* une course ; un test
  qui ne l'exerce qu'en séquence ne décrit pas le défaut.
- `cross_read_count` — le contrôle positif du montage lui-même, qui rougit si
  quelqu'un le re-dégrade en mémoires disjointes.

### R2 — AC4 citait deux choses inexistantes

`mika2248_le_relecteur_natteint_jamais_le_merge` est **in-file** dans
`crates/mika-agent/src/server/merge_ready_handler.rs:507`, **pas** dans
`tests/eval/test_merge_identity_2248.rs` comme le plan l'affirmait ; et il
n'existe **aucun** test `ac1_*` dans ce fichier (il porte `ac2_*`, `ac3_*`,
`ac4_*`). Corrigé ci-dessous. Ce n'est pas cosmétique : mika#2338 vient de
durcir le comparateur d'ancrage de mika-arch, et une citation non ancrable est
désormais un motif de refus d'attestation.

### R3 — les ancres sont citées par symbole, plus par numéro

Le plan citait `ci_success_handler.rs:531` pour le commentaire « Aucune logique
d'identité ici » (réellement `:517-518`), `:123-587` pour la fonction (réellement
`:123`–~`:656`), `handlers.rs:1289-1337` pour la chaîne (réellement ~`:1360`–
`:1460`) et `:1299-1301` pour le remplacement de `req.text` (réellement
`:1392-1393`). Le fichier n'a pas bougé : ces numéros étaient approximatifs dès
l'origine. Un plan relu 19 jours plus tard doit citer **le symbole**, et c'est ce
que fait la version ci-dessous.

### R4 — le piège dedup en test, que le plan ne nommait pas

`try_handle_ci_success` appelle `try_dedup_check_suite`, qui écrit dans la map
**process-globale**. `try_dedup_in` est paramétré sur la map « for tests », mais
le chemin de production ne l'est pas : deux tests du même binaire partageant une
clé `(repo, branch, head_sha)` se pollueraient selon l'ordre d'exécution.

Le contrôle positif de T1 ne le rencontre **pas** — le token est exigé avant
`find_open_pr`, qui précède le dedup, donc `github_token = None` sort avant
l'écriture. C'est vérifié, pas supposé. Mais il faut l'écrire : tout test futur
de ce chemin avec un token réel doit porter un `(repo, branch)` unique.

### R5 — la Fire-Disposition n'était pas conforme

Le plan portait bien une section `## Fire-Disposition`, mais sous la forme d'un
**tableau rouge/vert par test**, qui ne nomme aucune des trois options canoniques
de mika#1574. La règle de grooming mika#2306 en fait un ITERATE en première passe
et un ESCALATE en seconde, sans recours. Réécrite ci-dessous.

### R6 — les mesures du ticket ne sont pas re-mesurables ici, et c'est à dire

Les comptes qui fondent ce plan (81 parcours sous mika-qa, 6 tenues DECISION-CORE
en double, la chronologie à 5 ms) viennent d'`audit_events` et de
`/var/log/mika/server.log`. Le bac à sable de dispatch ne monte ni l'un ni
l'autre : `~/.mika/data/mika.db` et `/var/log/mika/` sont **absents** (vérifié).
Ces chiffres datent donc du **2026-09-09/10** et ont trois semaines. Ce qui *est*
établi aujourd'hui, par lecture et non par mesure : le fan-out existe, la chaîne
tourne à l'identique dans les deux agents, et `try_handle_ci_success` ne se
sélectionne que sur le type d'événement — donc **le double chemin est
structurellement toujours là**. La confirmation quantitative est la sonde AC5,
post-déploiement par construction.

### R7 — un doublon de littéral à ne pas créer

D1 ajoute un symbole à `forge_identity`, dont le voisin `DISPATCHER_FORGE_LOGIN`
vient de poser la discipline (mika#2334) : un **seul site de définition**, et
l'argumentaire du périmètre écrit **sur le symbole**. La ligne de journal cite
`DISPATCHER_AGENT` par la constante, jamais par sa valeur.

## Le défaut, mesuré (2026-09-09/10 — voir R6 pour la datation)

PR#2258 a corrigé **qui merge** : `mergedBy = mika-platform-dev` sur #2259,
`human_gate_events = 0`, re-preuve C2 réussie. Il n'a pas corrigé **qui évalue**.
Le relecteur (`mika-qa`) parcourt toujours l'évaluateur complet — jusqu'à émettre
le signal — et n'est retenu qu'à l'acteur.

### Comptes depuis le déploiement de #2258 (`audit_events`, 2026-09-09 → 2026-09-10)

| agent | `ci_success_handler_processed` | `..._human_gate_required` | `ci_success_merge_ready` | `merge_ready_hold_reviewer_is_not_merge_actor` | `ci_success_merge` |
|---|---|---|---|---|---|
| mika-dev | 65 | 7 | 1 | 0 | 1 |
| **mika-qa** | **81** | **6** | **1** | **1** | 0 |

81 parcours complets sous le PAT du relecteur. Six tenues DECISION-CORE — donc six
notifications « Operator must merge manually » **en double** de celles de mika-dev.

### Chronologie #2259, head `047e9eea`

| horodatage (Z) | agent | événement |
|---|---|---|
| 09:07:26 | mika-dev | `deferring webhook: task has in-flight callback` (60 s) |
| 09:08:11 | mika-qa | `evaluating` → `processed` écrit → clé dedup mémoire **enregistrée** → checks encore `pending` |
| 09:08:26 | mika-dev | replay du webhook différé → **`ci_success_dedup.skip`** — l'évaluation du dispatcher est **avalée** par la clé posée 15 s plus tôt par le relecteur |
| 09:12:36.705 | mika-qa | `evaluating` → toutes portes vertes → `ci_success_merge_ready` → **`merge_ready_hold_reviewer_is_not_merge_actor`** |
| 09:12:36.709 | mika-dev | `deferring webhook` (60 s) |
| 09:13:36.710 | mika-dev | replay — **60,005 s** après l'enregistrement de mika-qa, fenêtre dedup = 60 s : passe **à 5 ms près** |
| 09:20:17 | mika-dev | `evaluating` → `ci_success_merge_ready` → `merge_ready_merge_initiated` |

Le « double chemin en ~15 s » observé par Vincent est la paire 09:08:11 / 09:08:26.
Elle montre la conséquence que le ticket redoutait sans la nommer : **le dedup
mémoire est global au processus et clé sur `{repo}:{branch}:{head_sha}` sans
identité d'agent** (`check_suite_dedup::DEDUP_MAP`). Quand le relecteur entre en
premier, il consomme le créneau du dispatcher. À 09:08 c'était sans conséquence
(checks `pending`). À 09:12, toutes portes vertes, le dispatcher n'a évalué que
parce que sa copie est arrivée **5 ms après** la fin de la fenêtre. Un deferral à
59,9 s au lieu de 60,005 s : mika-dev rend `ci_success_dedup.skip`, aucun signal
n'est émis sous une identité qui peut agir, et plus aucun `check_suite` n'arrive
sur ce head (tous les workflows sont terminés). **La PR reste verte et ouverte
sans que rien ne la ferme.** Ce n'est pas un résidu cosmétique : c'est une classe
d'arrêt de boucle à un timing près.

### Les effets de bord que le relecteur exerce AVANT la garde

Dans l'ordre d'exécution de `try_handle_ci_success`, tous vérifiés le 2026-09-30 :

1. **Clé dedup mémoire** (étape 2b, `check_suite_dedup::try_dedup_check_suite`) —
   globale au processus, avale l'évaluation du dispatcher (mesuré ci-dessus).
2. **Cinq appels `gh` sous le PAT du relecteur** — `find_open_pr`,
   `find_pass_verdict`, `run_gh_checks`, `run_gh_pr_view`, `is_behind_main`.
   Quota et latence pour rien : le signal émis par mika-qa n'a **aucun
   consommateur** par construction — il voyage en bande dans `req.text`, relu par
   le handler suivant *du même agent*, où l'acteur le retient.
3. **`PUT /repos/{repo}/pulls/{n}/update-branch` sous le PAT du relecteur**
   (étape 5c, `remediate_behind_main`) — une **écriture** sur la forge, posée par
   l'agent qui ne doit jamais agir sur cette PR. Pas encore observée en audit,
   mais atteignable dès que le relecteur entre avec une PR en retard sur main.
4. **Notification opérateur DECISION-CORE en double** (étape 5b) — 6 mesurées.
5. **Le pré-digest de l'évaluateur REMPLACE `req.text`** dans le tour du
   relecteur (`handlers.rs`, bras `VerdictAction::Handled { pre_digest } =>
   req.text = pre_digest`) — mika-qa reçoit « Merge-ready signal emitted… » à la
   place de l'événement brut que `qa-review-webhook-success` attend (repo +
   branche, pour corréler la PR). Le prompt du relecteur est écrit pour le texte
   brut ; aujourd'hui il ne le voit que quand l'évaluateur rend `Passthrough`.

## La cause, en une phrase

Le fan-out gateway vers mika-qa est **voulu** (mika#1711,
`github.rs::secondary_targets` — c'est ce qui déclenche la revue autonome sur CI
verte), mais la chaîne de handlers structurels de `handlers.rs` tourne à
l'identique dans chaque agent, et `try_handle_ci_success` ne se sélectionne que
sur le **type d'événement** (`parse_check_suite_success`), jamais sur **l'agent**.
#2258 a mis l'identité à la sortie (l'acteur) ; il l'a laissée absente de
l'entrée (l'évaluateur), par une décision explicite — « Aucune logique d'identité
ici, par conception » — qui confond deux questions : *qui merge* (l'acteur,
réglé) et *pour qui cette évaluation existe* (personne, quand c'est le relecteur
qui la fait).

## La conception

**Un seul énoncé, une seule porte :** la transition évaluer→signaler→merger est un
tout qui appartient au dispatcher. Un agent qui ne peut pas consommer le signal ne
l'évalue pas. La porte se pose au premier point où l'agent est connu et avant tout
travail — après la sélection par type d'événement, avant même l'exigence de token.

### D1 — `forge_identity::owns_merge_transition(agent_id) -> bool`

`crates/mika-common/src/forge_identity.rs`. Un nom pour le concept, défini comme
`merge_disposition(agent_id) == MergeDisposition::Act`. La liste blanche reste
unique (`DISPATCHER_AGENT`) ; ajouter un acteur reste un geste explicite à un seul
endroit, et la porte d'entrée en hérite sans seconde table. La doc du module gagne
une quatrième ligne dans le tableau des couches : « la porte d'entrée de
l'évaluateur — seul le dispatcher évalue ; les autres agents restent transparents
à l'événement ».

Pourquoi un symbole plutôt que réutiliser `merge_disposition` en ligne :
l'évaluateur ne décide pas *qui merge* — c'est l'acteur qui le fait, et ça ne
bouge pas. Il décide si *son évaluation a un consommateur*. Deux questions, deux
noms ; une seule table sous-jacente.

Discipline du voisin (R7, mika#2334) : le doc-comment porte l'argumentaire du
périmètre — ici, pourquoi ce prédicat n'est pas `forge_login_for_agent` et pourquoi
il ne se substitue pas à l'acteur.

### D2 — la porte d'entrée dans `try_handle_ci_success`

`crates/mika-agent/src/server/ci_success_handler.rs`, immédiatement après l'étape
1 (`parse_check_suite_success`), **avant** le `match github_token` et avant tout
appel `gh`, toute écriture dedup, toute ligne d'audit existante :

- si `!owns_merge_transition(db.agent_id())` :
  `info!(event = "ci_success_handler_skipped_not_merge_actor", repo, branch,
  agent_id, dispatcher = DISPATCHER_AGENT, …)`, une ligne `audit_events` du même
  nom (clé cible `event:{repo}@{branch}` — le numéro de PR n'est pas connu, et ne
  doit pas l'être : le connaître coûte le premier appel `gh`), puis
  `return VerdictAction::Passthrough { enrichment: None }`.
- sinon : la suite inchangée.

`Passthrough { enrichment: None }` et non `Handled` : le tour du relecteur voit
**l'événement brut**, exactement ce que `qa-review-webhook-success` attend (effet
5). Pas d'enrichissement non plus — le prompt du relecteur sait déjà qu'il ne
merge pas ; lui redire n'ajoute qu'un second texte à suivre.

L'ordre est le point porteur : la porte doit précéder `try_dedup_check_suite`
(effet 1) — c'est ce que le test structurel T3 épingle.

### D3 — la seconde ceinture reste

`merge_ready_handler::authorize_merge` (`WouldMergeAsReviewer` puis
`NotDispatcher`) et le refus outil `pr_merge_with_gate_reviewer_refused` **ne
bougent pas**. Après D2 le hold du relecteur devient inatteignable en production ;
il reste la défense si un signal arrivait par un chemin que l'évaluateur n'a pas
posé (le module le dit déjà). Les tests de #2248 qui l'exercent en appelant
l'acteur directement restent verts tels quels.

### D4 — les commentaires et la doc qui disent l'inverse

Cités par symbole (R3), pas par numéro :

- `ci_success_handler.rs` § invariants d'en-tête : ajouter l'invariant « porte
  d'entrée (mika#2260) ». Étape 2c, la note de portée « check_suite success events
  for a given PR route to the same agent (mika-qa), so the scope is correct » est
  fausse depuis #1711 et devient « seul le dispatcher atteint ce point depuis
  mika#2260 ». Étape 6, la phrase « Aucune logique d'identité ici, par conception
  — ce fichier évalue, il n'agit pas » devient « l'identité décide à l'entrée si
  l'évaluation a un consommateur ; elle ne décide jamais ici qui merge ».
- `crates/mika-agent/CLAUDE.md` § *Structural CI Success Handler* et § *Merge
  Actor (mika#2248)* : un paragraphe « Entry gate (mika#2260) » avec le signal
  opérateur `ci_success_handler_skipped_not_merge_actor` et le fait mesuré du
  dedup global au processus.
- `docs/solutions/architecture-patterns/2026-09-09-un-handler-diffuse-a-deux-agents-ne-peut-pas-etre-acteur.md`
  (présent, 5442 o) : une section « Le résidu (mika#2260) » — une garde tardive
  résout la course sans la supprimer ; les cinq effets que l'évaluateur du
  relecteur touchait quand même ; la règle générale devient *un handler diffusé à
  N agents n'évalue que dans l'agent qui peut consommer son évaluation*.

### Ce que ce plan ne fait pas, et pourquoi

- **Poser la porte dans `handlers.rs` autour de la paire évaluateur+acteur.**
  Écarté : cela garderait l'évaluateur littéralement « sans identité », mais le
  corps de `handle_message` n'est pas testable en unité — la porte ne serait
  épinglable que par un test de source. Dans le handler, l'auto-sélection par
  agent vit au même endroit que l'auto-sélection par type, et T1 l'exerce sans
  réseau. La phrase « Aucune logique d'identité ici » décrivait l'état après
  #2248 (l'évaluateur ne décide pas *qui merge* — toujours vrai) ; ce n'est pas un
  invariant qui interdit une porte d'entrée (validé mika-arch, première passe).
- **Retirer mika-qa du fan-out gateway.** Non : le relecteur a besoin de
  l'événement brut pour déclencher la revue (mika#1711, 14 h de fenêtre QA morte
  avant). La porte est côté agent, pas côté routage.
- **Ajouter `agent_id` à la clé du dedup mémoire.** Écarté : une fois la porte
  posée, un seul agent atteint le dedup ; la clé multi-agent ne servirait qu'à
  ré-autoriser N évaluations si un second acteur était un jour listé — ce que la
  liste blanche rend délibérément coûteux. T3 (porte avant dedup) est la
  protection, pas la clé.
- **Le retard de 8 min de mika-dev sur #2259** (09:12 → 09:20) : file d'attente
  derrière un tour LLM `MaxTokens`, pas ce ticket. Observation à part.

## Acceptance criteria

- **AC1 — le relecteur est transparent à l'événement.** Pour un
  `check_suite.completed(success)` bien formé, dans un agent où
  `owns_merge_transition == false` (`mika-qa`, et tout agent hors liste blanche :
  `mika`, `mika-arch`, un nom inconnu), `try_handle_ci_success` rend
  `Passthrough { enrichment: None }` **sans** appel `gh`, **sans** ligne
  `ci_success_handler_processed`, **sans** clé dedup, et laisse une ligne
  `audit_events` `ci_success_handler_skipped_not_merge_actor`. Contrôle positif
  dans le **même test** : l'agent `mika-dev`, même texte, `github_token = None`,
  rend `Passthrough { enrichment: Some(…no GitHub token…) }` — preuve que la porte
  l'a laissé passer **et** qu'elle a joué avant l'exigence de token.
- **AC2 — l'attribution du fan-out est mesurée, pas déduite.** Sur un
  `MultiAgentHarness` montant `mika-dev` et `mika-qa` sur la **même** base
  container, l'événement diffusé aux deux agents produit
  `audit_counts_by_agent("ci_success_handler_processed")` à **une seule clé**
  (`mika-dev`) et `audit_counts_by_agent("ci_success_handler_skipped_not_merge_actor")`
  à une seule clé (`mika-qa`). C'est l'énoncé du ticket — « les deux agents
  entrent encore » — rendu falsifiable, dans les **deux** patrons : ordre
  déterministe et course `tokio::join!`.
- **AC3 — la porte précède tout travail (structurel).** Dans le corps de
  `try_handle_ci_success`, l'indice de `owns_merge_transition(` est strictement
  inférieur à ceux de `github_token`, `find_open_pr(`, `try_dedup_check_suite(` et
  `count_recent_audit_events_for_target(`. Épinglé par `include_str!`, même forme
  que les trois tests `ac3_*` de `test_merge_identity_2248.rs`.
- **AC4 — le nom d'audit est réel.** `ci_success_handler_skipped_not_merge_actor`
  passe `assert_audit_event_name_is_real` (`tests/eval/test_ci_success_handler.rs:73`)
  et apparaît dans les signaux opérateur de `crates/mika-agent/CLAUDE.md`.
- **AC5 — la seconde ceinture tient encore, sans modification.** Restent verts
  **tels quels** : `merge_ready_handler::tests::mika2248_le_relecteur_natteint_jamais_le_merge`
  (in-file, `merge_ready_handler.rs:507` — corrigé par R2), et les tests `ac2_*`,
  `ac3_*`, `ac4_*` de `tests/eval/test_merge_identity_2248.rs` (il n'y a pas de
  `ac1_*` — corrigé par R2). La porte d'entrée s'ajoute, elle ne remplace pas.
- **AC6 — mesure post-déploiement (une requête, deux contrôles).** Pour tout
  `check_suite.completed(success)` postérieur au déploiement : `audit_events` sous
  `mika-qa` compte `ci_success_handler_skipped_not_merge_actor ≥ 1` et
  `ci_success_handler_processed = 0` ; sous `mika-dev`,
  `ci_success_handler_processed ≥ 1`. Et `$MIKA_SPIRIT_LOG_FILE` ne contient plus
  de paire `evaluating` mika-qa → `ci_success_dedup.skip` mika-dev sur le même
  head.
- **AC7 — rouge avant.** T1 est rouge sur `main` (l'agent `mika-qa` avec
  `token = None` rend aujourd'hui `Passthrough { enrichment: Some(…) }` —
  l'enrichissement « no GitHub token » — et aucune ligne `skipped`). T2 est rouge
  sur `main` (deux clés dans la carte de `ci_success_handler_processed`). Chacun
  échoue pour la bonne raison, terme par terme.

## Périmètre

| fichier | geste |
|---|---|
| `crates/mika-common/src/forge_identity.rs` | D1 : `owns_merge_transition` + doc module + test unitaire |
| `crates/mika-agent/src/server/ci_success_handler.rs` | D2 : porte d'entrée ; D4 : invariants et commentaires |
| `crates/mika-agent/tests/eval/test_ci_success_handler.rs` | T1, T3, T4 |
| `crates/mika-agent/tests/eval/test_ci_success_fanout_2260.rs` | **nouveau** — T2 (attribution multi-agents) |
| `crates/mika-agent/tests/eval.rs` | déclaration du module T2 (garde `test_eval_modules_declared.rs`) |
| `crates/mika-agent/CLAUDE.md` | D4 |
| `docs/solutions/architecture-patterns/2026-09-09-un-handler-diffuse-a-deux-agents-ne-peut-pas-etre-acteur.md` | D4 |

**Zone DECISION-CORE.** Source Rust sous `crates/` : le classifieur de périmètre
est fail-closed (`crates/mika-agent/src/perimeter/rules.rs`), la PR sera tenue par
la forge-gate et mergée par l'opérateur. C'est la forme attendue — le ticket le
dit, et c'est le même chemin que PR#2258.

**Non touchés :** `handlers.rs` (la porte vit dans le handler, pas dans le
câblage — le test d'ordre de #2248 sur `handlers.rs` reste vrai),
`merge_ready_handler.rs`, `pr_merge_with_gate.rs`, `check_suite_dedup.rs`,
`multi_agent.rs` (consommé, pas modifié), `mika-gateway`.

## Tests

Tous sans réseau. `github_token = None` pour que le seul chemin qui passe la porte
s'arrête sur l'enrichissement « no GitHub token » — la première ligne après elle.

**Note dedup (R4) :** avec `token = None`, aucun test n'atteint
`try_dedup_check_suite`, donc la map process-globale n'est pas touchée et l'ordre
d'exécution des tests est sans effet. Tout test futur de ce chemin **avec un token
réel** doit porter un `(repo, branch)` unique, sous peine de pollution croisée
dans le même binaire.

- **T1 (AC1, AC7) `mika2260_le_relecteur_est_transparent_a_levenement`** — même
  texte `[GitHub] Check suite success on senara-solutions/mika (branch: fix/x)`,
  trois agents dans **un seul appel de test** : `mika-qa` →
  `Passthrough { enrichment: None }`, `skipped == 1`, `processed == 0` ; `mika`
  (hors liste) → idem ; `mika-dev` → `Passthrough { enrichment: Some(s) }` avec
  `s` contenant `no GitHub token`, `skipped == 0`. Le contrôle positif et les deux
  négatifs ensemble, sinon un handler qui rendrait toujours `Passthrough { None }`
  passerait.
- **T2 (AC2, AC7) `mika2260_lattribution_du_fanout_est_mesurable`** — nouveau
  fichier `test_ci_success_fanout_2260.rs`, sur `MultiAgentHarness::builder()
  .agent("mika-dev").agent("mika-qa").build()`. Deux sous-cas dans le même test,
  exactement les deux patrons que `multi_agent.rs` documente : **(a) ordre**
  déterministe (`for (id, db) in h.agents()`), **(b) course** (`tokio::join!` sur
  les deux handles). Dans les deux : `audit_counts_by_agent("ci_success_handler_processed")`
  n'a que la clé `mika-dev`, `audit_counts_by_agent("ci_success_handler_skipped_not_merge_actor")`
  n'a que `mika-qa`. Plus `cross_read_count("mika-dev", "mika-qa") > 0` comme
  contrôle positif du partage — sans lui, deux mémoires disjointes rendraient le
  test vert en ne mesurant rien.
- **T3 (AC3) `mika2260_la_porte_precede_tout_travail`** — `include_str!` +
  extraction du corps de `try_handle_ci_success` : indices ordonnés comme dit en
  AC3. Porte son **assertion d'anti-vacuité** : le test échoue si
  `owns_merge_transition(` est absent du corps, sinon un scan qui vise un nom mort
  se lit exactement comme un arbre propre (classe mika#2103/#2205).
- **T4 (AC4)** — `assert_audit_event_name_is_real("ci_success_handler_skipped_not_merge_actor")`.
- **T5 (D1)** — dans `forge_identity.rs` : `owns_merge_transition("mika-dev")`,
  `!owns_merge_transition("mika-qa")`, `!owns_merge_transition("")`,
  `!owns_merge_transition("mika-dev-2")`, plus l'égalité avec
  `merge_disposition(...) == Act` sur ces mêmes entrées (les deux fonctions ne
  doivent pas diverger) et la normalisation (`"  MIKA-DEV "`).
- **AC5** — `cargo test -p mika-agent --test eval` et `cargo test -p mika-common`
  verts, **aucune modification** des tests de #2248.

## Fire-Disposition

Ce plan livre trois détecteurs : **T3** (scan de source sur l'ordre des termes
dans `try_handle_ci_success`), **T4** (réalité du nom d'audit) et **T5** (unité de
non-divergence entre `owns_merge_transition` et `merge_disposition`).

**Option retenue : (a) exception nommée en allowlist — allowlist livrée VIDE**,
pour les trois. C'est la doctrine mika#2201 : quand un de ces détecteurs tire, on
**route le site par la garde** ; on n'ajoute pas de ligne d'exemption. Une
allowlist née vide est un endroit où déposer la prochaine infraction, donc elle
est livrée vide **et** épinglée vide.

Détail d'implémentation, par détecteur :

| détecteur | population de violations existantes | allowlist | assertion auto-nettoyante |
|---|---|---|---|
| T3 (ordre des termes) | **zéro** — un seul site (`try_handle_ci_success`), qui est précisément celui qu'on arme | aucune constante d'exemption n'est créée : il n'y a rien à exempter, donc aucun emplacement pour une future exemption | le test échoue si `owns_merge_transition(` est **absent** du corps — un scan visant un nom mort est indistinguable d'un arbre propre (mika#2103/#2205) |
| T4 (nom d'audit réel) | **zéro** — le nom est neuf | s.o. (le helper `assert_audit_event_name_is_real` n'en porte pas) | le helper échoue si le nom n'est écrit nulle part dans la source |
| T5 (non-divergence) | **zéro** — `owns_merge_transition` n'existe pas encore | s.o. | l'égalité est asserted sur les quatre entrées ; elle rougit le jour où l'une des deux fonctions bouge sans l'autre |

**Aucun détecteur n'est livré désarmé** (option b) et **aucun ne remonte à
l'opérateur** (option c) : les trois populations de violations sont vides par
construction, donc les trois peuvent être armés dans le commit qui les introduit,
et chacun est **vu rouge d'abord** (AC7) — un détecteur qui naît vert n'a jamais
montré qu'il voit.

## Vérification

1. `cargo test -p mika-common forge_identity` ; `cargo test -p mika-agent --test eval ci_success` ;
   `cargo test -p mika-agent --test eval fanout_2260` ; `cargo test -p mika-agent --test eval merge_identity` ;
   `cargo clippy --all-targets -- -D warnings` ; `cargo fmt --check`.
2. **Rouge avant (AC7), un terme à la fois.** Stash la porte D2 : T1 doit rougir
   sur `enrichment` **et** sur `skipped == 1` (deux assertions distinctes, pas
   une) ; T2 doit rougir sur la cardinalité de la carte `processed` (2 clés au lieu
   de 1) ; T3 doit rougir sur l'absence du symbole. Un seul terme neutralisé à la
   fois — une conjonction ne se prouve pas en cassant tout d'un coup.
3. Après déploiement (`make deploy` depuis `main`), attendre **au moins une PR** de
   la boucle qui passe au vert (~10 min pour un cycle CI complet), puis la requête
   d'AC6 :

   ```sql
   select agent_id, tool_name, count(*)
   from audit_events
   where created_at >= '<horodatage du déploiement>'
     and tool_name in ('ci_success_handler_processed',
                       'ci_success_handler_skipped_not_merge_actor',
                       'ci_success_merge_ready')
   group by 1, 2 order by 1, 2;
   ```

   Attendu : mika-qa n'a **que** des `skipped` ; mika-dev **aucun** `skipped` et au
   moins un `processed`.
4. `grep -c ci_success_dedup.skip "$MIKA_SPIRIT_LOG_FILE"` après déploiement :
   chaque occurrence restante doit avoir sa première sighting sous mika-dev (rafale
   de workflows d'un même push — le cas pour lequel le dedup existe), jamais sous
   mika-qa.

### Les haltes

- **Halte 1 — `skipped` reste à zéro sous mika-qa alors que des `check_suite`
  verts sont passés.** Ne pas élargir le prédicat par réflexe : établir d'abord que
  le binaire servi porte le correctif (classe mika#2340), puis que l'événement
  atteint bien mika-qa (`secondary_targets` inchangé, file webhook non saturée).
- **Halte 2 — `processed` non nul sous mika-qa après déploiement.** La porte est
  contournée : lire **quel chemin** a appelé l'évaluateur avant de toucher à
  `owns_merge_transition`. La porte ne couvre que `try_handle_ci_success`.
- **Halte 3 — `ci_success_dedup.skip` continue d'avaler des évaluations de
  mika-dev.** Alors le dedup est consommé par un troisième producteur, pas par le
  relecteur : ne pas ajouter `agent_id` à la clé (décision écartée ci-dessus)
  avant d'avoir nommé ce producteur.
- **Halte transverse — les deux comptes muets.** Zéro `skipped` **et** zéro
  `processed` ne prouve rien : il faut qu'un `check_suite.completed(success)` ait
  réellement eu lieu depuis le déploiement. *Une garde que personne n'a exercée se
  lit exactement comme une garde qui marche* (mika#2205).

## Risques et questions ouvertes

- **Un agent client (`customer-42`) qui recevrait un `check_suite` ne sera plus
  jamais évalué.** C'est voulu par la liste blanche de #2248 (« un agent inconnu
  signale, il ne merge pas ») — ici il ne signale même plus. Si un jour un tenant
  doit auto-merger ses propres PRs, c'est `DISPATCHER_AGENT` qu'il faut faire
  évoluer (une table d'acteurs par tenant), pas la porte.
- **Volume d'audit.** La ligne `skipped` remplace, sous mika-qa, une ligne
  `processed` **plus** jusqu'à cinq appels `gh` par événement ; sur une rafale de 8
  workflows elle remplace 1 `processed` + 7 `dedup.skip` avec chacun son
  `gh pr list`. Le solde est net négatif en écritures et en appels. Pas de dedup
  sur la ligne `skipped` : elle est le compteur qu'AC6 mesure.
- **La classe n'est pas re-mesurée, elle est ré-établie par lecture (R6).** Les
  comptes ont trois semaines et la base n'est pas atteignable depuis le bac à
  sable. Ce qui est établi aujourd'hui est structurel : fan-out présent, chaîne
  identique dans les deux agents, sélection sur le type seul. Si AC6 rend une
  population **vide** sous mika-qa alors que des PR sont passées au vert, la
  prémisse a changé entre le 09/09 et le déploiement — c'est un résultat à écrire,
  pas un défaut du correctif, et la Halte 1 est le chemin.
- **Pas de question ouverte de conception.** Le point d'insertion est dicté par
  l'effet 1 (avant le dedup) et l'effet 5 (`Passthrough`, pas `Handled`) ; les deux
  sont mesurés, pas choisis.

## Ce que ce travail n'achète PAS

- **Il ne supprime pas le fan-out**, et ne doit pas : la revue autonome en dépend.
  Ce qui est retiré est l'**évaluation** dans l'agent qui ne peut pas la consommer.
- **Il ne rend pas le dedup multi-agents.** Après la porte, un seul agent
  l'atteint ; la clé reste volontairement sans `agent_id`, et T3 est ce qui
  protège cette propriété plutôt que la clé.
- **Il ne rattrape aucune PR restée verte et ouverte** pendant la fenêtre du
  défaut. Rien ici ne rétro-ferme une PR : le correctif ferme la classe pour les
  événements **suivants**, et le filet existant pour la population passée est la
  réconciliation `qa_review_reconcile` (mika#2334/#2347), inchangée.
- **Il n'ajoute aucun compteur au-delà de la ligne `skipped`.** Les instruments
  sont la requête d'AC6 et le grep de la vérification 4, et **leur silence ne
  prouve rien tant que personne ne les exécute**.
