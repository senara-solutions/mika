# mika#3141 — le lecteur de consentement de l'auto-fire reçoit l'`owner/repo`, et aucun site ne peut plus lui donner le nom court

Régression de #3139 (`35239e0d`), mesurée le 2026-11-22 à 20:00:33Z sur #3119.
Vol unique.

## Le défaut, tel qu'il se mesure

`try_dispatch_pilot_after_groom_success` lit les coordonnées du ticket parent à
l'étape 3 (`crates/mika-agent/src/task_engine/dispatcher.rs:5685`) :

```rust
let (repo, issue_num) = match parse_repo_issue_from_url(&reference_url) { … };
```

`parse_repo_issue_from_url` (l. 6419) splitte l'URL sur le littéral
`"senara-solutions/"` et ne garde que le segment suivant : il rend le **nom
court** du dépôt, `"mika"`. L'étape 4bis (l. 5814) passe cette valeur à
`autofire_consent_at_dispatch`, dont le paramètre s'appelle `repo`, qui la passe
à `ready_label::reread_issue_consent_via_gh`, dont le paramètre s'appelle
`owner_repo` et qui l'interpole en `gh issue view <n> --repo mika --json
state,labels`. `gh` refuse : *expected the "[HOST/]OWNER/REPO" format*.

Le refus remonte en `Err`, que `autofire_consent_at_dispatch` traduit en
`AutofireConsent::Unreadable`, donc en refus `consent_unreadable_at_autofire`
(`groom_pilot_autofire_consent_refused`). **Le refus est sûr — il est
fail-closed — mais il est inconditionnel** : `ready` posé ou non, ouvert ou
clos, tout auto-fire après groom est refusé. La chaîne « `ready` sur un ticket
non groomé → auto-groom → pilote » (mika#996) est coupée depuis `35239e0d`.

Les deux autres appelants du même lecteur passent un `owner/repo` complet et
n'ont jamais été atteints :

| appelant | ce qu'il passe | provenance |
|---|---|---|
| `dispatcher.rs:1642` (ré-armement, mika#3050) | `parent.repo` | `parse_ready_label_task_label`, qui **exige** un `/` (`ready_label.rs:222`) |
| `executor.rs:5994` | `owner_repo` | résolu en amont |
| **`dispatcher.rs:5814` (étape 4bis)** | **`repo` nu** | `parse_repo_issue_from_url` |

### Pourquoi la suite de tests est restée verte

Les quatre tests de comportement de mika#3118
(`mika3118_consentement_a_lautofire`, l. 11826-12001) partent **déjà** d'une
`reference_url` réelle et traversent `try_dispatch_pilot_after_groom_success` en
entier. Ils passent, et ils passaient avant comme après l'introduction du
défaut, pour une raison unique : le double de `gh` (`gh_process.rs:281`) indexe
ses charges **par sujet** — `GhSubject::Issue(n)` — et `arm_consent` pose la
charge sans motif d'`argv`. Le double sert donc la réponse *quel que soit* le
`--repo` demandé. Les tests mesuraient la traduction des quatre verdicts ; ils
ne mesuraient à aucun moment **ce qui est passé au lecteur**.

C'est la classe exacte que la table de charges par motif (`set_canned_stdout_for`,
mika#3047) sait fermer, et elle existait déjà dans le dépôt au moment du défaut.

### Pourquoi le lecteur ne peut pas rendre les deux formes sous un seul nom

Le nom court n'est pas un accident à supprimer : **il a un consommateur
légitime**, à six lignes du site fautif. Le brief du pilote
(`dispatcher.rs:5975`, `"prompt": format!("{repo}#{issue_num}")`) porte son
propre commentaire : *dispatch-lib's worktree-setup parser requires
`<repo>#<num>`*. Les deux formes coexistent dans la même fonction, et c'est
l'homonymie — un seul mot, `repo`, pour les deux — qui a produit la régression.

## Requirements

- **R1.** Le site 4bis passe au lecteur de consentement un `owner/repo`
  complet, lu dans la `reference_url` du parent et non recomposé à partir d'un
  littéral d'organisation.
- **R2.** Le nom court reste disponible, inchangé, pour le brief du pilote et
  pour les champs `repo` des lignes de log et des motifs d'audit déjà en
  service : aucune population d'opérateur ne change de forme.
- **R3.** Un site qui voudrait reproduire la confusion doit avoir à **nommer**
  laquelle des deux formes il veut : aucun champ ni paramètre accessible depuis
  ce chemin ne s'appelle plus `repo` nu.
- **R4.** Un test part d'une `reference_url` réelle et va **jusqu'à l'appel de
  lecture** : il observe l'`argv` effectivement remis à `gh`, et non seulement
  le verdict rendu. Il rougit sur le code d'avant le correctif.
- **R5.** Le lecteur `reread_issue_consent_via_gh` refuse un argument qui n'a
  pas la forme `owner/repo` **avant** de lancer `gh`, sous un motif qui nomme la
  cause — plutôt que de laisser le `stderr` de `gh` l'expliquer à un lecteur de
  logs Rust.
- **R6.** Les trois scans de source de mika#3118 (`canonical_tokens.rs`) restent
  verts *pour la bonne raison* : leurs ancres de bonne foi sont mises à jour
  avec les formes qu'elles surveillent, jamais élargies.

## Design

### D1 — Un lecteur unique, deux champs nommés, aucun champ `repo`

`parse_repo_issue_from_url` rend un type porteur plutôt qu'un tuple :

```rust
struct IssueCoordinates {
    /// `senara-solutions/mika` — la forme que TOUTE API de la forge exige.
    owner_repo: String,
    /// `mika` — le nom court, exigé par le parseur de worktree-setup de
    /// dispatch-lib (voir le brief de l'étape 5e).
    repo_short: String,
    issue: u64,
}
```

**Aucun des deux champs ne s'appelle `repo`**, et c'est tout le design (R3) :
l'homonymie était la cause, donc le remède est que le mot nu n'existe plus sur
ce chemin. Un futur éditeur qui écrit `coords.owner_repo` pour un appel API ou
`coords.repo_short` pour le brief a, dans les deux cas, dit lequel il voulait.
Un accesseur dérivé (`fn owner_repo(&self) -> String` au-dessus d'un champ
`repo`) aurait laissé le nom nu atteignable, donc la régression reproductible.

L'owner est **lu dans l'URL**, pas recomposé. Le parse devient :

```rust
let after_host = url.split_once("github.com/")?.1;
let mut parts = after_host.split('/');
let owner = parts.next()?;
if owner != GITHUB_ORG { return None; }   // le contrat d'organisation, inchangé
let repo_short = parts.next()?.to_string();
// … `issues`, puis le numéro, et le strip de query string, inchangés
```

La distinction est porteuse : le littéral `senara-solutions` sert de
**contrôle** et non de source. Une URL d'une autre organisation est refusée
(comme aujourd'hui) au lieu d'être réécrite en `senara-solutions/<son repo>` —
ce qu'un re-préfixage aveugle aurait fait. C'est aussi ce que le doc-comment de
`parse_ready_label_task_label` (`ready_label.rs:193-201`) reproche à un
re-préfixage, et sa prose devra être amendée : elle affirme aujourd'hui que ce
lecteur « rend le repo nu ».

L'exigence neuve de `github.com/` dans l'URL est sans population perdue : les
onze `reference_url` du dépôt, production et tests confondus, sont toutes de la
forme `https://github.com/senara-solutions/<repo>/issues/<n>` (relevé
exhaustif par grep sur `dispatcher.rs`). Un test d'aller-retour l'épingle.

### D2 — Un seul appelant change de sens, l'autre change d'une ligne

`parent_issue_coordinates` (l. 6391) garde sa signature
`(Option<String>, Option<u64>)` et rend `c.repo_short` : ses deux appelants
(l. 6173, 6326) écrivent un champ de log `repo=` dont la forme courte est déjà
celle que l'opérateur lit (R2). Une migration vers `owner/repo` y serait un
changement de format de fil sans demandeur.

Au site 4bis, le paramètre de `autofire_consent_at_dispatch` est renommé
`repo` → `owner_repo` : le nom du paramètre est la moitié de la cause racine
(il annonçait un contrat qu'il ne tenait pas), et son corps passe déjà la valeur
à un paramètre nommé `owner_repo`. Les locales de
`try_dispatch_pilot_after_groom_success` deviennent `repo_short` / `issue_num`,
ce qui force le compilateur à faire revoir chacun des six usages de `repo` qui
suivent — dont le brief, qui doit rester court.

### D3 — Le refus de forme vit dans le lecteur, pas dans ses appelants

`reread_issue_consent_via_gh` refuse un `owner_repo` sans `/` avant le spawn
(R5), sous le motif *« `owner_repo` sans organisation »*. Trois raisons, dans
cet ordre :

1. **Elle nomme la cause là où elle est connue.** Aujourd'hui le fait traverse
   `gh`, en ressort en anglais dans un `stderr` concaténé, et atterrit dans un
   `reason` d'audit où rien ne distingue « le dépôt n'a pas d'owner » de « le
   réseau est tombé ». Deux causes sous un motif ne sont pas soustractibles
   (mika#2205).
2. **Elle est en deçà du réseau.** Un appel qui ne peut pas réussir n'est pas
   lancé — l'inverse du `gh issue view --repo mika` qui a tourné 1 fois par
   auto-fire depuis `35239e0d`.
3. **Elle est au bon étage.** `parse_ready_label_task_label` porte déjà
   exactement ce refus pour exactement cette raison (`ready_label.rs:219-224`,
   « un repo nu ferait une URL d'API fausse »). Le lecteur d'état est le frère
   qui ne l'avait pas.

Elle **ne masque pas** le défaut de ce ticket : un site fautif rendrait toujours
un refus, simplement nommé. C'est le correctif de R1, et lui seul, qui rétablit
l'auto-fire.

### D4 — Ce qui n'est délibérément pas fait

- **Pas de newtype `OwnerRepo` traversant.** La fermeture de classe complète —
  un type que le compilateur refuse de confondre avec un `&str` — toucherait
  `ready_label.rs`, `executor.rs`, le ré-armement et leurs tests, pour une
  classe que D1 ferme déjà sur le seul chemin qui l'a produite. Hors périmètre ;
  rien ne le réclame aujourd'hui.
- **Pas de scan structurel « aucun site ne passe un nom court à une API de
  forge ».** Le prédicat porterait sur la *valeur* d'un argument et non sur un
  site, c'est-à-dire la forme d'allowlist que mika#2309 condamne : « nom de
  dépôt sans slash » apparie aussi des slugs, des clés de config et des libellés
  de label. D1 le rend inutile en supprimant le nom ambigu.
- **Pas de changement au vocabulaire d'audit.** Les six valeurs de
  `GROOM_PILOT_*` restent ce qu'elles sont, et `consent_unreadable_at_autofire`
  garde son sens : la lecture n'a pas pu se faire. Ce vol réduit sa population à
  ce qu'elle devait mesurer.

## Périmètre

Un seul vol, un seul commit de travail. Tout tient dans trois fichiers de
`crates/mika-agent/src/` et un mouvement de code borné à une fonction de parse,
un site d'appel et un paramètre renommé. Rien n'est renvoyé à un suivi.

Hors périmètre, nommé plus haut : le newtype traversant (D4), et toute
migration de `parent_issue_coordinates` vers la forme qualifiée (D2).

## Verification Contract

| # | ce qui est vérifié | où |
|---|---|---|
| V1 | L'`argv` remis à `gh` porte `--repo senara-solutions/mika` sur l'auto-fire après groom — lu dans `gh_process::recorded_calls()`, pas déduit du verdict | `dispatcher.rs` mod tests (R4) |
| V2 | Régression : la charge de consentement est armée **par motif** `--repo senara-solutions/mika` (`set_canned_stdout_for`), donc un `--repo mika` ne la reçoit pas, le double rend `b"[]"`, et le verdict tombe en `Unreadable`. Le test exige `Held` et zéro row d'audit : il est rouge sur le code d'avant le correctif | `dispatcher.rs` mod tests (R4) |
| V3 | Aller-retour du lecteur : `https://github.com/senara-solutions/mika/issues/990131` → `owner_repo == "senara-solutions/mika"`, `repo_short == "mika"`, `issue == 990131` ; query string encore strippée | `dispatcher.rs` mod tests (R1) |
| V4 | Le lecteur refuse (`None`) une URL d'une autre organisation, une URL dont le segment 2 n'est pas `issues`, et un numéro non parsable — aucune ne devient une coordonnée fabriquée | `dispatcher.rs` mod tests (R1) |
| V5 | `reread_issue_consent_via_gh("…", "mika", n)` rend `Err` dont le motif nomme la forme attendue, et `recorded_calls()` ne porte **aucun** appel pour ce sujet : le refus est en deçà du réseau | `ready_label.rs` mod tests (R5) |
| V6 | Les quatre tests de verdict de mika#3118 restent verts inchangés — la traduction `Held`/`ReadyAbsent`/`IssueClosed`/`Unreadable` n'est pas touchée par ce vol | suite existante (R2) |
| V7 | Les trois scans de mika#3118 sont verts, ancres mises à jour : `mika3118_le_site_dengagement_du_consentement_est_unique` (1 site), `…_tombe_au_bon_rang` (4 ancres, ordre), `…_nont_aucun_bras_joker` (fenêtre ancrée sur `reread_issue_consent_via_gh(token, owner_repo, issue)`) | `canonical_tokens.rs` (R6) |
| V8 | `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main` | pipeline |

**Ce que V1 et V2 mesurent séparément, et pourquoi les deux.** V2 seul passerait
si quelqu'un remplaçait la charge par motif par une charge de sujet « pour faire
passer le test » — le double redeviendrait complaisant et le test vert sans que
rien ne soit lu. V1 observe le journal d'appels, qui ne dépend d'aucune charge.
V1 seul, lui, ne dirait pas que le verdict s'en trouve changé.

## Definition of Done

- Le site 4bis passe `coords.owner_repo` au lecteur de consentement.
- `IssueCoordinates` remplace le tuple du lecteur ; aucun champ ni paramètre
  nommé `repo` ne subsiste sur le chemin de l'auto-fire.
- Le brief du pilote porte toujours le nom court, et le prouve par la suite
  existante.
- `reread_issue_consent_via_gh` refuse un `owner_repo` sans `/` avant le spawn.
- V1-V8 passent. Les quatre tests de mika#3118 et les trois scans de source
  restent verts, ancres amendées plutôt qu'élargies.
- Le doc-comment de `parse_ready_label_task_label` ne dit plus que
  `parse_repo_issue_from_url` « rend le repo nu ».

## Acceptance criteria

Le corps de mika#3141 ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de son « Attendu » et des Requirements.

- **AC1.** `try_dispatch_pilot_after_groom_success` passe au lecteur de
  consentement l'`owner/repo` complet (`senara-solutions/mika`), et cet
  `owner/repo` est lu dans la `reference_url` du parent.
- **AC2.** Un test part d'une `reference_url` réelle, traverse la fonction
  entière, et assère sur l'**appel de lecture** : l'`argv` observé porte
  `--repo senara-solutions/mika`.
- **AC3.** Ce test est rouge sur le code d'avant le correctif — la charge de
  consentement n'est servie que si le `--repo` est qualifié.
- **AC4.** Un auto-fire après groom sur un ticket ouvert et portant `ready`
  n'écrit **aucune** row d'audit de refus et laisse le parent en
  `dispatch_class = groom` jusqu'au saut de 5a : la chaîne mika#996 est
  rétablie.
- **AC5.** Le brief du pilote porte toujours `<repo-court>#<num>`, forme exigée
  par le parseur de worktree-setup de dispatch-lib.
- **AC6.** Les trois scans de source de mika#3118 passent, avec leurs
  assertions de bonne foi (« l'ancre doit apparaître exactement une fois »)
  satisfaites par les formes neuves.

## Fire-Disposition

Ce plan livre des détecteurs : V1-V5 sont des tests dont le chemin de succès est
« aucune violation », et V5 livre en plus une garde de production (le refus de
forme dans `reread_issue_consent_via_gh`).

**Option (a) — exception nommée en allowlist, allowlist livrée VIDE.**

Détail d'implémentation, et c'est l'absence d'entrée qui est le livrable :

- La **seule** violation existante de la classe est le site 4bis lui-même, et
  elle est corrigée dans le même commit que les détecteurs. Il n'y a donc aucune
  donnée à exempter, et aucune entrée d'allowlist n'est écrite — pas un tableau
  vide « pour plus tard », pas un `#[ignore]`.
- Les deux autres appelants du lecteur (`dispatcher.rs:1642`,
  `executor.rs:5994`) passent déjà un `owner/repo` qualifié ; vérifié par
  lecture, et V5 les couvre désormais par construction puisque la garde est
  dans le lecteur.
- L'auto-nettoyage est porté par le détecteur lui-même plutôt que par une
  assertion de staleness séparée : V2 est un test de comportement sur le chemin
  nominal, donc il rougit le jour où un site repasse un nom court, et il n'a
  rien à périmer. Les trois scans de mika#3118 conservent leurs assertions de
  bonne foi, qui rougissent si l'ancre cesse de décrire la production.
- Les détecteurs atterrissent **armés** : aucun `#[ignore]`, aucun
  `#[cfg(skip)]`, aucun suivi ouvert pour les armer plus tard. Option (b) est
  donc explicitement écartée ; option (c) sans objet, le périmètre étant
  entièrement cadré par le corps du ticket.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/task_engine/dispatcher.rs` — `IssueCoordinates` + réécriture du lecteur + doc | 85 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — site 4bis, locales renommées, `parent_issue_coordinates` | 25 |
| `crates/mika-agent/src/ready_label.rs` — garde de forme + doc + amendement du doc-comment l. 193-201 | 35 |
| `crates/mika-agent/src/canonical_tokens.rs` — ancres V6/V7 amendées | 5 |
| tests (`dispatcher.rs` mod tests — V1 à V4) | 110 |
| tests (`ready_label.rs` mod tests — V5) | 25 |

Total estimé : 285 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Un seul vol, et voici pourquoi il tient : 285 est sous la borne par vol (429) et
sous le seuil par PR (1000). Le périmètre est un changement de **forme** sur un
chemin unique — un type de retour, un site d'appel, un paramètre renommé — et
non l'ajout d'un sous-système. Les deux tiers du volume de test sont des
assertions sur un harnais qui existe déjà (`recorded_calls`,
`set_canned_stdout_for`, `fixture_issue_url`, `refuse_run`), donc il n'y a ni
double à écrire ni fixture à inventer. Aucun découpage en vols n'est nécessaire,
et aucune suite n'est renvoyée à un ticket de suivi.

## Ordre de commit

Un commit de travail, parce que le correctif et son détecteur ne sont
démontrables qu'ensemble : V2 est rouge avant, vert après, et les livrer
séparément laisserait `main` avec un test rouge entre les deux.

1. `fix(3141): l'auto-fire passe l'owner/repo complet au lecteur de consentement`
   — `IssueCoordinates`, site 4bis, garde de forme dans le lecteur, ancres
   amendées, V1-V5.

Plancher : `cargo test -p mika-agent` vert, `cargo clippy` sans warning,
`cargo fmt --check` propre, `make verify-bundled-skills` vert, puis
`bash scripts/verify-pipeline.sh origin/main`.
