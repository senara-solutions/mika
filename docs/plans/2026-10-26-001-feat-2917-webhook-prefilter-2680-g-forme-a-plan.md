# mika#2917 — (g′) forme (a) : écarter les tours `check_suite` dont la tête est déjà revue

> Vol 1 sur 3. Parent : senara-solutions/mika#2680, classe (g′), bearing ratifié
> par Vincent le 2026-10-26 : **filtrer**, dans la forme (a).

## Contexte

Sur 4,9 jours, 51 tours `check_suite_pr_branch` (mika-dev et mika-qa) se
terminent par `stop_verdict_recorded` (35) ou `stop_already_reviewed_sha` (16),
pour 7,06 M tokens d'entrée (#2680, commentaire 6410232454). Chacun paie un tour
complet pour conclure qu'il n'y a rien à faire. Aucun tour attesté n'est sorti en
`state_unreadable`.

Le pré-filtre (`crates/mika-agent/src/server/webhook_prefilter.rs`) ne peut pas
écarter ces tours aujourd'hui, pour une raison matérielle : **le texte que le
gateway transmet ne nomme pas la tête.** `format_event_text` émet
`[GitHub] Check suite {conclusion} on {repo} (branch: {branch})` et jette
`check_suite.head_sha` ainsi que `check_suite.pull_requests[]`. Le pré-filtre
n'a donc aucun moyen de savoir *de quelle tête* l'événement parle, et la seule
façon de l'apprendre serait un appel `gh` — que le bearing refuse (mika#2260).

Ce vol lève cet obstacle, et **rien d'autre** : le gateway transmet ce qu'il
jette, le moteur en lit la forme, et aucune décision ne la lit encore.

## Ce qui est refusé (bearing, non négociable)

- **Aucun appel `gh` depuis `webhook_prefilter.rs`**, sous quelque identité que
  ce soit (mika#2260). Ce vol ne fait qu'ajouter des sites de lecture de texte,
  et il **pose la garde** qui recense les appels `gh` du fichier avant que le
  vol 3 n'y touche.
- **Aucune copie de la logique de dedup du skill dans le Rust** : un seul site
  de vérité. Ce vol n'écrit aucune comparaison ; le vol 3 comparera, sans rien
  recalculer.

## Découpage en vols

Le total des trois pièces de la forme (a) est estimé à ≈ 1 150 lignes hors
`docs/` — au-dessus du seuil par PR (1 000) **et** très au-dessus de la borne
par vol (429, § *Taille estimée*). La borne la plus serrée mord, donc le travail
sort en **trois vols indépendamment mergeables**. L'ordre est celui des trois
pièces du ticket, et il est contraint : la pièce 3 ne peut pas comparer une tête
que la pièce 1 ne transmet pas.

| vol | périmètre | plancher (ce que sa PR doit tenir) | AC servis | taille |
|---|---|---|---|---|
| **1 — ce vol** | Pièce 1 : le gateway transmet `head_sha` et `pull_requests[]`, `mika-common` en déclare la grammaire et le parse, `classify` la transporte, `decide` ne la lit pas. | Les textes anciens **et** nouveaux se lisent par les quatre lecteurs existants ; le texte émis se re-parse en aller-retour ; aucun comportement de porte ne change. | prépare AC1–AC3 (aucun n'est atteint) | **365** |
| **2** | Pièce 2 : l'outil `record_reviewed_head`, le prompt de `qa-review-webhook-success` qui l'appelle, la garde de l'écrivain unique et le recensement des sites `gh`. | L'état est écrit et **personne ne le lit** ; l'écrivain unique est recensé ; la garde `gh` est verte sur `main` avant le diff du vol 3. | AC3 | ≈ 366 |
| **3** | Pièce 3 : la classe (g′), le terme d'état, le décideur, l'interrupteur dédié, le corpus rejoué. | AC1, AC2 et AC3 verts, AC4 mesurable après merge. | AC1, AC2, AC3, AC4 | ≈ 425 |

**Pourquoi la garde AC3 est au vol 2 et non au vol 3.** Une garde qui atterrit
avec le code qu'elle juge n'a jamais rien refusé : elle passe la revue, elle
passe la CI par construction, et le défaut qu'elle aurait attrapé atterrit en
production. Ce dépôt a écrit cette doctrine lui-même (`suite-wiring-lint`,
mika#2578 une fois puis mika#2752 deux fois). La garde doit donc être **verte
sur `main` avant** le diff qu'elle juge — ce que « au vol 2 » garantit et
« au vol 3 » ne garantit pas.

Tracked in: senara-solutions/mika#2680 — le parent de la classe (g′) porte les
vols 2 et 3 jusqu'à l'ouverture de leurs tickets. Vol 2 : « le skill écrit son
état de dedup, et il en est le seul écrivain ». Vol 3 : « le pré-filtre compare
et ne recalcule rien ».

## La pièce 2, choisie et justifiée

Le ticket délègue au groom un choix et lui demande de l'écrire : *« il écrit une
ligne `(repo, PR, head_sha, issue)` à un endroit que le moteur peut lire sans
`gh`. Le groom choisit cet endroit et le justifie par écrit. Le moteur ne
l'écrit jamais. »* Le choix est arrêté ici, parce que les vols 1 et 3 en
dépendent (la clé, et le site de lecture), même si l'implémentation est au
vol 2.

**L'endroit retenu : une ligne d'`audit_events` sous un `tool_name` dédié,
écrite par un outil que le skill appelle, lue par le moteur à travers un seul
terme du trait `PrefilterState`.**

```
tool_name   = QA_REVIEWED_HEAD_TOOL            (nouvelle constante, SOLE WRITER déclaré)
target_key  = ci_success_handler::head_key(repo, pr, head_sha)   →  pr:{repo}#{n}@{sha}
after_value = l'issue du tour : "review" | "stop_already_reviewed_sha" | "stop_verdict_recorded"
reasoning   = "request_id=<uuid>"
agent_id    = porté par la ligne, donc le registre est par agent par construction
```

Les quatre champs du ticket y sont au complet : `repo`, `PR` et `head_sha` dans
la clé, `issue` dans `after_value`.

**Pourquoi là.**

1. **Aucun `gh`.** Le pré-filtre lit la base SQLite de son propre agent, par les
   accesseurs qui existent déjà (`count_recent_audit_events_for_target`).
2. **Aucune table, aucune migration.** Le précédent est explicite et vit dans ce
   dépôt : `crates/mika-agent/src/qa_head_supersession.rs`, dont l'en-tête dit
   *« Pas de table, pas de migration : la lecture passe par
   `count_recent_audit_events_for_target`, et le registre est **par agent** »*,
   et dont `SYNC_OBSERVED_TOOL = "qa_pr_sync_observed"` porte une déclaration
   **SOLE WRITER** en doc-comment. C'est le même mécanisme, au même titre.
3. **Par agent par construction.** Les lignes d'`audit_events` portent
   l'`agent_id`. L'état de mika-qa ne peut pas écarter un tour de mika-dev, et
   réciproquement — ce qui est exactement la prémisse que le seau
   `skipped_by_agent_class` de `measure-webhook-turn-cost` a été ajouté pour
   vérifier (#2680 vol 5).
4. **Aucune grammaire de clé nouvelle.** `head_key` existe, et le pré-filtre la
   lit déjà telle quelle (`pr_prefix`, `decision_core_holds`). Une forme de clé
   retapée dans un second fichier est la classe de divergence programmée que
   mika#2683 évite déjà (mika#2158, mika#2362).
5. **La décision reste entière dans le skill.** Le moteur ne calcule jamais
   « cette tête est revue » : il compare la tête que le payload nomme à une
   ligne que le skill a écrite. Aucune copie de la logique de dedup en Rust.
6. **Le moteur ne l'écrit jamais.** L'écriture Rust n'a lieu que comme effet
   direct d'un appel d'outil du modèle. Aucun chemin du moteur n'écrit cette
   ligne, et la garde de l'écrivain unique (vol 2) refuse qu'un second site
   apparaisse.

**Ce qui a été écarté, et pourquoi.**

- **`store_fact` / `memory_facts`.** Aucun outil nouveau, mais l'état de dedup
  entre dans la mémoire sémantique et ressort dans les résultats de
  `search_memory` de toutes les questions sans rapport. `memory_facts` n'a par
  ailleurs aucune grammaire de clé `(repo, PR, tête)` : il faudrait la retaper,
  ce que (4) refuse.
- **Une table dédiée (schéma v56 → v57).** Une migration pour un registre dont
  le lecteur rend un booléen, quand le précédent du dépôt dit explicitement
  « pas de table, pas de migration ».
- **Un fichier sous `~/.mika/state/`.** Ni transactionnel, ni par agent par
  construction, et le pré-filtre y gagnerait une lecture de système de fichiers
  qu'il n'a nulle part ailleurs.
- **Surcharger l'attestation `Webhook-Outcome:` de mika#2855 et la persister
  depuis `emit_turn_outcome`.** C'est la forme la moins chère en lignes, et elle
  est refusée sur le bearing, pas sur le coût : (i) le site d'écriture devient un
  site **du moteur**, ce que le ticket exclut en une phrase ; (ii) elle
  transforme une ligne d'observabilité en entrée de refus, qui est précisément
  la faute que ce dépôt interdit ailleurs par un scan de sources
  (`LABELED_BY_LINE_PREFIX` : *« Hard invariant — informational only. **No
  refusal decision reads this value.** »*) ; (iii) `emit_turn_outcome` tire sur
  **tous** les tours webhook de tous les agents, donc l'élargir élargirait une
  population que mika#2855 a bornée.
- **Un outil de skill à handler `exec`** (la forme de `qa_pr_view`). Un
  sous-processus shell ne peut écrire dans la base de l'agent qu'en ouvrant
  `mika.db` en direct : un second écrivain hors du type Rust, avec un `agent_id`
  re-dérivé. Refusé pour la même raison que l'operational ledger interdit le SQL
  direct.

**Coût assumé du choix.** `record_reviewed_head` est un builtin, donc sa
définition pèse sur le prompt de **tous** les agents (mika#2593 a mesuré ce
poids). Deux atténuations : la définition est courte (trois champs requis, un
`enum` fermé), et `MIKA_ARCH_ALLOWED_TOOLS` étant une allowlist fermée, un
builtin qui n'y figure pas n'est **pas** servi à mika-arch — l'outil n'entre
donc pas dans le prompt du relecteur.

**Ce que `required_tools` ne fera pas.** `qa-review-webhook-success/skill.toml`
ne déclarera **pas** `record_reviewed_head` dans `required_tools`. La raison est
documentée dans ce dépôt : `docs/solutions/architecture-patterns/`
`required-tools-terminal-failure-bypass-2026-04-20.md` et
`docs/solutions/runtime-errors/mika-qa-duplicate-pr-review-required-tools-gate-2026-04-26.md`
— un `required_tools` non résolu fait échouer le skill de façon terminale. Sur
un handler de webhook, une écriture manquée coûte un tour payé (régime
d'aujourd'hui, fail-safe) tandis qu'un échec terminal coûte une revue perdue.
L'asymétrie décide.

## Requirements

### R1 — Le payload entre dans le gateway

`crates/mika-gateway/src/github.rs:98-102` : `CheckSuite` gagne deux champs, et
rien n'en perd.

```rust
#[derive(Debug, serde::Deserialize)]
pub struct CheckSuite {
    pub conclusion: Option<String>,
    pub head_branch: Option<String>,
    pub head_sha: Option<String>,
    #[serde(default)]
    pub pull_requests: Vec<CheckSuitePullRequest>,
}

#[derive(Debug, serde::Deserialize)]
pub struct CheckSuitePullRequest {
    pub number: u64,
    pub head: Option<GitHubRef>,
}
```

- `#[serde(default)]` sur `pull_requests` : un payload sans le tableau se
  désérialise, il ne fait pas échouer la livraison.
- `GitHubRef` **existe déjà** (il porte `sha`, et `GitHubPullRequest.head`
  l'utilise). On le réemploie ; on n'en écrit pas un second.

### R2 — La grammaire du texte est déclarée dans `mika-common`, une fois

`crates/mika-common/src/github_event_format.rs` est le site de vérité unique de
la grammaire des textes de webhook, et le test cross-crate
`mika-gateway::github::tests::mika2680_chaque_prefixe_declare_est_celui_que_le_gateway_emet`
refuse la dérive producteur/consommateur. Les deux nouvelles lignes y sont
déclarées :

```rust
/// Deuxième ligne d'un texte de `check_suite` : la tête de la suite.
/// Forme complète : `Head-Sha: {sha}`.
pub const CHECK_SUITE_HEAD_LINE_PREFIX: &str = "Head-Sha: ";

/// Troisième ligne : les PR que la suite nomme, `{n}@{sha}` séparés par un
/// espace, ou [`CHECK_SUITE_PRS_NONE`] quand le tableau est vide.
/// Forme complète : `Check-Suite-PRs: 2917@1a2b3c 2918@9f8e7d`.
pub const CHECK_SUITE_PRS_LINE_PREFIX: &str = "Check-Suite-PRs: ";

/// Le tableau `pull_requests` était **présent et vide** — ce qui n'est pas la
/// même chose que la ligne absente (mika#2331, mika#2340).
pub const CHECK_SUITE_PRS_NONE: &str = "none";
```

Écrire ces littéraux ailleurs qu'ici tomberait sous la garde des jetons
canoniques (mika#2201) : les consommateurs font `starts_with(CONST)`, qui ne
porte aucun littéral.

### R3 — Les lignes nouvelles sont des lignes 2 et 3, jamais la ligne 1

Le texte émis devient :

```
[GitHub] Check suite success on senara-solutions/mika (branch: feat/2917/x)
Head-Sha: 1a2b3c4d5e6f…
Check-Suite-PRs: 2917@1a2b3c4d5e6f…
```

**C'est une contrainte, pas une préférence.** Quatre lecteurs du texte
`check_suite` existent, et les quatre ne lisent que la première ligne, qu'ils
terminent sur son `)` :

| lecteur | forme de lecture |
|---|---|
| `ci_success_handler::parse_check_suite_success` | `text.lines().next()` puis `strip_suffix(')')` |
| `ci_failure_handler::parse_check_suite_failure` | `text.lines().next()` puis `strip_suffix(')')` |
| `webhook_queue::CHECK_SUITE_RE` (mika#528) | `^…\(branch: ([^)]+)\)` sur `text.lines().next()` |
| `webhook_queue_v2::CHECK_SUITE_RE` | même motif, `^` sans `(?m)`, pas de `$` |

Aucun n'est ancré en fin de texte, aucun n'utilise `(?m)`, et `[^)]+` s'arrête
au `)`. Ajouter des lignes **après** la première les laisse donc tous intacts —
et c'est précisément ce qu'un test de ce vol épingle, sur un texte ancien et sur
un texte nouveau, pour les quatre.

`truncate_body` ne s'applique pas au bras `check_suite` (aucun corps n'y est
transmis), donc les caps mika#911 ne touchent pas ces lignes.

### R4 — Le bras `check_suite` de `format_event_text`

`crates/mika-gateway/src/github.rs:545-555`, étendu sans rien retirer :

```rust
"check_suite" => {
    let cs = event.check_suite.as_ref();
    let conclusion = cs.and_then(|c| c.conclusion.as_deref()).unwrap_or("unknown");
    let branch = cs.and_then(|c| c.head_branch.as_deref()).unwrap_or("unknown");
    let head = cs.and_then(|c| c.head_sha.as_deref()).unwrap_or("unknown");
    let prs = cs.map(|c| format_check_suite_prs(&c.pull_requests)).unwrap_or_default();
    format!(
        "[GitHub] Check suite {conclusion} on {repo_name} (branch: {branch})\n\
         {CHECK_SUITE_HEAD_LINE_PREFIX}{head}\n\
         {CHECK_SUITE_PRS_LINE_PREFIX}{prs}"
    )
}
```

- `head_sha` absent rend `unknown`, sur le modèle exact de `conclusion` et
  `branch` : le gateway n'invente pas, et le consommateur traite `unknown` comme
  une tête non nommée.
- `format_check_suite_prs` rend `CHECK_SUITE_PRS_NONE` sur un tableau vide, et
  **saute** toute PR dont `head.sha` est absent — une PR sans tête n'est pas une
  PR nommée.
- Les deux lignes sont **toujours émises**, y compris pour un tableau vide.
  L'absence de la ligne et la ligne `none` sont deux populations distinctes :
  l'absence veut dire *binaire d'avant ce vol* (mika#2340), `none` veut dire
  *`pull_requests` vide* — l'état mesuré sur mika#2681. Les confondre, c'est
  additionner un régime de déploiement à un comportement de la forge
  (mika#2205), et c'est exactement ce que le vol 3 ne doit pas faire.

### R5 — Le parse, dans `mika-common`, à côté de sa grammaire

```rust
/// Ce que le texte dit des têtes de la suite. Quatre valeurs, et aucune ne se
/// déduit d'une autre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckSuiteHeads {
    /// La ligne `Check-Suite-PRs:` est absente : binaire d'avant mika#2917
    /// (mika#2340). **Jamais confondu avec `Empty`.**
    LineAbsent,
    /// La ligne est là et dit `none` : `pull_requests` était vide.
    Empty,
    /// Exactement une PR, avec son `head.sha`. Le seul cas que le vol 3
    /// comparera.
    One { pr: u64, head_sha: String },
    /// Plusieurs PR : le payload ne nomme pas LA tête de l'événement.
    Many(usize),
}

pub fn parse_check_suite_heads(text: &str) -> CheckSuiteHeads;
```

- Fonction **pure**, sans état, sans réseau. Elle lit les lignes 2+ par
  `lines().find_map(|l| l.strip_prefix(CONST))`, et n'utilise **aucun
  découpage d'octets** sur `&str` — `byte-slice-lint` (mika#764) refuse la
  forme, et un `sha` est de l'ASCII mais un nom de branche ne l'est pas
  forcément.
- Un jeton `{n}@{sha}` illisible (numéro non numérique, `@` absent) est **sauté**
  et non promu en erreur : le texte vient du gateway, pas de l'utilisateur, et le
  vol 3 doit se comporter comme si le payload n'avait pas nommé la PR.
- Elle vit dans `mika-common` et non dans le pré-filtre pour une raison
  testable : le test du gateway peut alors asserter l'**aller-retour**
  `parse_check_suite_heads(format_event_text(payload))`, qui est une forme bien
  plus forte que deux assertions de chaînes et qui tue la classe de dérive que
  `github_event_format` existe pour tuer.

### R6 — `classify` transporte les têtes, `decide` ne les lit pas

`Candidate::GreenPrBranch` gagne un troisième champ, rempli par `classify` (qui
reste pure) :

```rust
GreenPrBranch {
    repo: String,
    branch: String,
    /// (g′) ce que le payload dit des têtes (mika#2917, vol 1). **Aucun
    /// décideur ne le lit dans ce vol** : il est transporté pour que le vol 3
    /// n'ait qu'un décideur à écrire, et sa présence ne change aucune porte.
    heads: CheckSuiteHeads,
}
```

- `decide` continue d'appeler `decide_head_decided(&repo, &branch, state)` —
  signature inchangée, classe (b) inchangée.
- `Touched::applies_to` et `candidate_kind` filtrent sur `Self::GreenPrBranch { .. }`,
  donc le champ ne demande aucun élargissement de motif. Si un site s'avère
  exhaustif, il est élargi en `{ .. }` et non recopié.
- `Candidate::RedPrBranch` n'est **pas** touché : les arrêts 1 et 2 sont hors de
  ce vol.
- **Zéro terme `Release` nouveau, zéro `PrefilterClass` nouvelle dans ce vol.**
  Les deux arrivent au vol 3, avec le décideur qui les produit. Déclarer une
  classe qu'aucun site ne peut atteindre ferait entrer dans
  `skipped_by_class` un seau qui ne se remplira jamais, et un seau vide se lit
  comme « cette classe n'écarte rien » (mika#2205).

### R7 — La garde du bearing, posée avant le diff qu'elle juge

Déplacée au **vol 2** (voir § *Découpage en vols*). Sa forme est arrêtée ici
parce que l'exception qu'elle porte est un fait d'aujourd'hui, pas du vol 2 :
`webhook_prefilter.rs` contient **un** appel `gh` au moment où ce plan s'écrit,
`LiveState::pr_state` (`gh pr view --json state`, via
`crate::tools::pr_merge_with_gate::run_gh_subprocess`), qui sert la classe (c)
`VerdictPrClosed` et **n'est pas sur le chemin (g′)**. Détail dans
§ *Fire-Disposition*.

## Non-Requirements (hors de ce vol, délibérément)

- **La comparaison.** Aucun décideur ne lit `CheckSuiteHeads` au vol 1.
- **L'écriture de l'état de dedup.** L'outil et le prompt sont au vol 2.
- **Les arrêts 1 et 2** (`stop_no_open_pr`, `stop_out_of_scope` : 4 tours,
  0,64 M). Hors flight par le ticket.
- **La forme (b).** À n'ouvrir que si (a) échoue au groom.
- **Le filage de `head_sha` jusqu'à la clé de coalescence de la file v2.**
  `webhook_queue_v2` documente ce manque comme *« correctness-critical »* et le
  renvoie à un suivi : *« The gateway-formatted text carries `repo` and `branch`
  but NOT `head_sha` … Threading `head_sha` end-to-end (companion `mika-gateway`
  change) is the fully-correct form, deferred to a follow-up. »* Ce vol **rend ce
  suivi réalisable** — le texte portera la tête — et ne le réalise pas :
  `coalescing_key` reste `check_suite:{repo}:{branch}`, et l'atténuation par
  coalescence bornée dans le temps reste celle décrite par le module. Élargir
  ici changerait une clé de dedup de file sous couvert d'un vol de grammaire.
  Tracked in: senara-solutions/mika#2680.
- **Le retrait du dernier appel `gh` de `webhook_prefilter.rs`.** C'est
  l'achèvement de mika#2260, pas de ce vol. L'exception nommée du
  § *Fire-Disposition* le référence.

## Verification Contract

1. `cargo test -p mika-common github_event_format` — le parse, ses quatre
   valeurs, et le saut d'un jeton illisible.
2. `cargo test -p mika-gateway github` — l'aller-retour émission → parse sur
   trois payloads (`pull_requests` absent, vide, une PR, plusieurs PR), et la
   parité cross-crate des préfixes.
3. `cargo test -p mika-agent webhook_prefilter` — `classify` transporte les
   têtes ; les quatre lecteurs de la première ligne lisent un texte ancien **et**
   un texte nouveau à l'identique.
4. `cargo test` (suite complète), `cargo clippy`, `cargo fmt --check`.
5. `scripts/check-byte-slices.sh` — `parse_check_suite_heads` n'introduit aucun
   découpage d'octets sur `&str`.
6. `make verify-bundled-skills` — aucun bundle n'est touché par ce vol ; le gate
   doit rester vert sans exception.
7. `scripts/verify-pipeline.sh origin/main`.
8. **`suite-wiring-lint` : N/A, et c'est vérifiable.** Les livrables de test de
   ce vol sont des `#[cfg(test)] mod tests` compilés dans les crates, atteints
   par `cargo test` que la CI lance. `scripts/check-suite-wiring.sh` ne juge que
   les suites présentes sous `scripts/` ou `skills/bundled/_shared/` ; ce vol
   n'en crée aucune, donc `scripts/suite-wiring.tsv` ne gagne **aucune** entrée
   (et son `unwired=0` ne doit pas bouger).

## Definition of Done

- `CheckSuite` porte `head_sha` et `pull_requests[].{number, head.sha}`, et un
  payload dépourvu de l'un ou l'autre se désérialise toujours.
- Le texte `check_suite` porte ses deux lignes nouvelles, **sans rien retirer**
  de ce qu'il portait.
- Les trois constantes de grammaire et `parse_check_suite_heads` vivent dans
  `mika_common::github_event_format`, et aucun consommateur n'écrit leur
  littéral.
- `LineAbsent` et `Empty` sont deux valeurs distinctes, et un test le dit.
- `parse_check_suite_success` et `parse_check_suite_failure` lisent les anciens
  textes **et** les nouveaux ; les deux `CHECK_SUITE_RE` aussi.
- `classify` remplit `GreenPrBranch.heads` ; aucun décideur ne le lit ; aucune
  porte du pré-filtre ne change de verdict sur aucune entrée existante.
- Aucun terme `Release`, aucune `PrefilterClass`, aucune migration de schéma,
  aucun appel `gh` ajoutés.
- Le § *Verification Contract* passe en entier.

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2917. **Les quatre sont
servis par les vols 2 et 3** ; le vol 1 les prépare et n'en atteint aucun — son
contrat de PR est le § *Definition of Done* et la colonne « plancher » du
§ *Découpage en vols*.

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

**Où chaque AC atterrit.**

| AC | vol | comment il est tenu |
|---|---|---|
| AC1 | 3 | `webhook_prefilter/tests/ac1_corpus_2917.rs` — rejeu **ordonné** (l'état de chaque tour est l'accumulation de ce que les tours précédents ont écrit, jamais un état pré-rempli), plus le contrôle négatif dans le même test, plus une anti-vacuité : le corpus porte au moins un tour de chacune des trois issues. |
| AC2 | 3 | Quatre tests sur l'état bouchonné : `Empty`, `Many`, ligne lue sans la tête, lecture en erreur. Les quatre rendent `Gate::Charged`. |
| AC3 | 2 (garde) + 3 (le diff qu'elle juge) | Deux scans de sources : recensement des sites `gh` de `webhook_prefilter.rs` à exception nommée, et recensement exact des sites qui nomment `QA_REVIEWED_HEAD_TOOL` (un écrivain, un lecteur). |
| AC4 | 3, mesuré après merge | `skipped_by_class` groupe dynamiquement sur `.class` (`scripts/measure-webhook-turn-cost`, seau `skipped_by_class`) : **la classe nouvelle y apparaît sans aucune modification du script**, et `skipped_by_agent_class` la ventile par agent. La moitié « les issues payées baissent » se lit sur `webhook_turn_outcome`, joint par `request_id`. |

## Fire-Disposition

Ce vol livre des détecteurs : les tests de compatibilité des quatre lecteurs
(chemin de succès = « aucune dérive »), l'extension du test de parité
cross-crate des préfixes, et — au vol 2, mais arrêtée ici — la garde AC3.

**Option retenue : (a), exception nommée en allowlist.**

### (a.1) Le recensement des sites `gh` de `webhook_prefilter.rs` — une exception

Le scan parcourt la moitié production du fichier
(`crate::source_scan::production_half`), énumère ses corps de fonctions
(`crate::source_scan::fn_bodies`), et rend la liste de ceux qui nomment un jeton
d'appel `gh` (`run_gh_subprocess`, `run_gh`, le littéral `"gh"`). **Il existe une
violation aujourd'hui**, donc le détecteur atterrit avec une entrée et une seule :

```rust
/// Les sites de `webhook_prefilter.rs` qui appellent `gh`. Une entrée par site,
/// nommant la donnée précise qu'il lit et le ticket qui le retire.
///
/// - `pr_state` — lit le seul champ `state` d'une PR (`gh pr view --json state`)
///   pour la classe (c) `VerdictPrClosed`. **Antérieur à mika#2917** et hors du
///   chemin (g′) : aucun décideur de `GreenPrBranch` ne l'atteint. Son retrait
///   est l'achèvement de mika#2260, suivi par senara-solutions/mika#2260.
const GH_CALL_SITES_ALLOWED: &[&str] = &["pr_state"];
```

Trois propriétés, et aucune n'est optionnelle :

- **L'assertion est auto-nettoyante.** Elle compare l'**ensemble exact** des
  sites trouvés à `GH_CALL_SITES_ALLOWED` — pas un `contains`, pas un `>=`. Le
  jour où `pr_state` cesse d'appeler `gh`, l'entrée devient stale et le test
  rougit en nommant l'entrée à retirer. C'est le geste qui rend l'exception
  inutile qui la fait expirer — ce qui est ce qui empêche cette allowlist de
  devenir celle que mika#2309 condamne.
- **Contrôle négatif.** Le scan tourne sur une source fabriquée contenant un
  appel `gh` dans une fonction hors allowlist, et doit le trouver. Sans lui,
  « le scan n'a rien trouvé » est indistinguable de « le scan n'a rien lu ».
- **Anti-vacuité.** Le test asserte que le scan a bien lu le fichier réel et y a
  énuméré au moins N corps de fonctions (mika#2205 : deux populations sous un
  nom ne sont pas soustractibles, et « zéro violation » sur un fichier non lu
  n'est pas une mesure).

### (a.2) Le recensement de l'écrivain et du lecteur uniques — allowlist **vide**

Le second scan n'a **aucune violation existante** : `QA_REVIEWED_HEAD_TOOL` n'est
créée qu'au vol 2. Il atterrit donc **armé, allowlist vide, et doit rester
vide** — quand il rougit, la résolution est de router la lecture par le terme du
trait, jamais d'ajouter une entrée. Forme : l'ensemble des fichiers de production
sous `crates/mika-agent/src/` qui nomment la constante doit être exactement
`{ tools/record_reviewed_head.rs, server/webhook_prefilter.rs }`, et le site qui
passe la constante à `log_audit_event` doit être unique. Même contrôle négatif,
même anti-vacuité.

### (a.3) Les détecteurs de ce vol — armés, sans exception

Les tests de compatibilité des quatre lecteurs et l'extension de la parité
cross-crate atterrissent verts, sans allowlist : il n'existe aucune violation à
excepter, et en créer une vide serait une entrée inerte qui se lit comme une
autorisation.

**Aucun détecteur n'est livré désarmé** (option b), et rien ne remonte à
l'opérateur (option c).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-common/src/github_event_format.rs` (3 constantes + doc, `CheckSuiteHeads`, `parse_check_suite_heads`) | 95 |
| `crates/mika-common/src/github_event_format.rs` — tests du parse (ligne absente, `none`, une PR, plusieurs, jeton illisible) | 70 |
| `crates/mika-gateway/src/github.rs` (`head_sha`, `CheckSuitePullRequest`, `format_check_suite_prs`, bras `check_suite`) | 45 |
| `crates/mika-gateway/src/github.rs` — tests (aller-retour émission→parse sur 4 payloads, parité cross-crate des préfixes) | 80 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` (`GreenPrBranch.heads`, `classify` le remplit, `decide` ne le lit pas) | 30 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` — tests (`classify` transporte ; les 4 lecteurs lisent ancien et nouveau texte) | 45 |

Total estimé : 365 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient ici.** 365 < 429, avec 64 lignes de marge, et
365 < 1000. Les trois pièces de la forme (a) totalisent ≈ 1 150 lignes et ne
tenaient dans aucune des deux bornes : le § *Découpage en vols* les sort en
trois vols indépendamment mergeables, chacun sous 429, et nomme la suite par la
ligne `Tracked in:` de cette même section. Le vol courant est le seul jugé ici.

**Ce qui tient la marge.** Le vol n'ajoute aucun décideur, aucun terme `Release`,
aucune `PrefilterClass`, aucune migration, aucun outil, et ne touche aucun
bundle — donc aucune fin de pipeline coûteuse (pas de `make deploy`, pas de
`make verify-bundled-skills` à réparer, pas de table à migrer). Le livrable le
plus lourd est un parse pur et ses tests, qui ne demandent ni base, ni réseau, ni
bouchon d'état.

## Ordre de commit

1. `mika-common` : les trois constantes, `CheckSuiteHeads`, `parse_check_suite_heads`,
   et ses tests. **Compile et passe seul**, sans aucun producteur.
2. `mika-gateway` : `CheckSuite.head_sha`, `CheckSuitePullRequest`,
   `format_check_suite_prs`, le bras `check_suite`, et les tests d'aller-retour +
   parité. **À partir d'ici le texte porte la tête**, et rien ne la lit.
3. `mika-agent` : `GreenPrBranch.heads`, `classify` qui le remplit, et les tests
   de compatibilité des quatre lecteurs sur ancien et nouveau texte.

Chaque étape laisse l'arbre vert. L'étape 2 est celle qui change un octet sur le
fil ; l'étape 3 est celle qui prouve que ce changement n'a rien cassé.

## Sonde post-déploiement (vol 1)

Ce vol n'arme aucune porte, donc il n'a pas de halte. Une seule lecture, et elle
sert au vol 3 : après `make deploy`, sur un `check_suite` réel d'une branche de
PR, le texte transmis doit porter ses trois lignes. **Un texte à une seule ligne
alors que le gateway tourne veut dire un binaire d'avant ce vol** (classe
mika#2340), jamais « la forge n'a pas envoyé la tête » — et c'est exactement la
distinction que `CheckSuiteHeads::LineAbsent` rend représentable pour le vol 3.
