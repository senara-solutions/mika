# fix(ci-reader) — le lecteur CI lit `statusCheckRollup` avec un jeton d'installation de l'App, le PAT en repli

Ticket : senara-solutions/mika#2963
Labels : `bug`, `p1-important`, `dispatch:loop`, `loop-substrate`
Vol 1 de 2. Vol 2 : senara-solutions/mika#2965.

---

## 1. Ce qui est mesuré, et ce qui est vérifié dans l'arbre

### 1.1 Le défaut

`run_gh_checks_raw` (`crates/mika-agent/src/tools/pr_merge_with_gate.rs:1111`)
compose son argv au site unique `gh_checks_args` (l. 1073) :
`gh pr checks <n> --repo <r> --json name,state,bucket,link`. `gh` résout ce
`--json` par le champ GraphQL `statusCheckRollup`, qu'un PAT fine-grained ne
peut pas lire. Mesure du corps du ticket (`/var/log/mika/server.log`,
2026-10-07 → 2026-10-29) : 65 échecs sur mika-cloud, 11 sur mika-platform, 1 sur
control-monitor, tous sur
`GraphQL: Resource not accessible by personal access token (node.statusCheckRollup…)`.
Même après l'élargissement du PAT par Vincent le 2026-10-29 à 14:45Z,
`check-runs` rend 403.

Le token arrive par `run_gh_subprocess` (l. 2214), qui scrube `MIKA_*` et
`GH_TOKEN` puis réinjecte `GH_TOKEN = <token>`. Un jeton d'installation d'App
s'y substitue sans une ligne de plus : `gh` l'accepte comme n'importe quel
`GH_TOKEN`, et l'installation 123032070 porte `checks=read`.

### 1.2 Les consommateurs, recensés dans l'arbre

Le ticket en cite quatre. Il y en a **cinq** :

| # | site | comment il obtient son token aujourd'hui |
|---|---|---|
| 1 | `tools/pr_merge_with_gate.rs:261` (`execute`) | `ctx.github_token` (l. 169) |
| 2 | `skills/builtin_handlers.rs:4384` (garde mika#2455, via `validate_qa_ci_coherence`) | `ctx.github_token` |
| 3 | `server/verdict_handler.rs:612` | paramètre `github_token`, résolu en `server/handlers.rs:1790` |
| 4 | `server/ci_success_handler.rs:431` | le même, passé en `handlers.rs:1826` |
| 5 | `server/ci_failure_handler.rs:949` (`fetch_failure_context`) | paramètre `token` |

Le cinquième est `ci_failure_handler` : il ne décide pas d'un merge, il bâtit un
contexte de réparation. Il hérite pourtant du même échec de lecture, et c'est la
raison pour laquelle le recensement doit être complet avant de choisir où le
token change.

Les 55 `qa_ci_coherence_abstained` avec `reason "gh_failed"` viennent du
**consommateur 2** — c'est lui, et lui seul, qui porte la sonde AC5.

### 1.3 Le patron, tel qu'il est écrit

`crates/mika-common/src/config.rs` porte déjà **deux** cascades, écrites à la
main, voisines, et sa doc dit pourquoi :

- `resolve_github_token` (l. 1923) — PAT d'abord, App en repli. Opérations
  **identitaires** : la forge expose l'auteur (approve, merge, review, comment).
  Sa doc : « Symétrique inverse de `resolve_label_write_token` : les deux
  résolveurs existent parce que la priorité dépend de la classe d'opération,
  pas de l'appelant. »
- `resolve_label_write_token` (l. 1981, mika#2228) — App d'abord, PAT en repli.
  Écritures **non identitaires** (labels, status checks). Rend un
  `mika_common::label_write::LabelWriteToken` qui porte sa source et un `Debug`
  rédigé.

Lire la CI est une opération non identitaire : aucune règle de la forge ne lit
l'auteur d'une lecture de checks.

### 1.4 Ce qui n'est pas un chemin réseau neuf

`GitHubApp::installation_token()` (`crates/mika-common/src/github_app.rs:218`)
est déjà déclaré dans `docs/egress/egress-manifest.toml` (l. 80-86 :
`client_site`/`call_site` = `crates/mika-common/src/github_app.rs`, `data` = « un
JWT signé par la clé privée de la GitHub App, échangé contre un jeton
d'installation »). Ce plan n'ouvre aucun sink : **le manifeste d'egress n'est pas
touché**, et le lockstep `verify-egress-manifest.sh` reste vert par construction.
Le cache mémoire double-checked de `installation_token()` fait qu'une lecture CI
ne coûte pas un échange de jeton.

---

## 2. Les trois décisions, par écrit

### D1 — Le point d'identité : voie (i), lire avec l'App, poster avec le PAT (AC2)

**Retenu : (i).** Portée la plus étroite, et la seule qui ne déplace aucune
identité observable par la forge.

Ce qui reste intact, et pourquoi c'est le critère : la chaîne de post de la QA
lit le login `mika-platform-qa` en plusieurs endroits —
`qa_review_reconcile`, `qa_head_supersession`, `server::verdict`,
`deadline_verdict`, `skills::matcher`, `task_engine::dispatcher`,
`server::sync_debounce`. Basculer toute la QA sur l'App (voie (ii)) ferait poster
les revues par `mika-platform-bot[bot]`, et chacun de ces sites lirait un login
qu'il ne reconnaît pas. Le défaut mesuré, lui, est une **lecture**. Changer
l'identité d'écriture pour réparer une lecture est un élargissement que rien
dans la mesure ne demande.

Deuxième raison, structurelle : `resolve_github_token` documente que deux PAT
distincts existent précisément pour que `mika-qa` puisse approuver les PR de
`mika-dev` — sous une identité App partagée, GitHub refuse avec
`Review Can not approve your own pull request`. La voie (ii) rouvrirait ce défaut.

**Le site d'exécution de `pr review` ne change pas** : `run_gh`
(`skills/builtin_handlers.rs:5139`) exécute avec `ctx.github_token`
(l. 5383) — le PAT résolu par `resolve_github_token`. Le `ci_read_token` ajouté
par ce plan n'est lu que par le **garde** `validate_qa_ci_coherence`, jamais par
l'exécution. Pour que cette séparation soit observable et non seulement vraie,
le plan extrait le site unique du token d'exécution (R6 ci-dessous) et
l'épingle (T1).

### D2 — Le résolveur est **neuf**, et il n'est pas factorisé (AC1)

`Settings::resolve_ci_read_token(&self, github_app) -> Option<CiReadToken>`.

**Pourquoi neuf plutôt que `resolve_label_write_token`.** La cascade est la
même ; le **type rendu** ne l'est pas. `LabelWriteToken` n'a qu'une méthode
métier, `report_write_failure(scope, target, label, error)` — l'échec d'une
**écriture de label**, avec son numéro de cible et son nom de label. Le réutiliser
pour une lecture de checks rendrait ce type menteur sur deux points à la fois :
le `scope = "label_write"` que son résolveur écrit dans le WARN
`gh_app_token_exchange_failed`, et le vocabulaire de son rapport d'échec. AC4
veut compter les lectures CI par jeton ; mika#2228 veut compter les refus
d'écriture de label. Les faire sortir sous le même nom d'événement et le même
`scope` rend les deux populations inséparables — exactement le défaut d'instrument
que mika#2205 nomme.

**Pourquoi pas de helper factorisé.** La tentation est d'extraire la cascade
commune (`resolve_app_first(app, scope)`) et d'habiller les deux résolveurs.
Le plan ne le fait pas, et c'est un choix aligné sur ce que `config.rs` fait
déjà : `resolve_github_token` et `resolve_label_write_token` sont **déjà** deux
cascades écrites à la main, et la doc du second dit pourquoi il faut les lire
ensemble plutôt que les fondre (« Toucher l'un sans lire l'autre est la façon
dont cette règle se perdra »). Un troisième résolveur explicite suit ce
précédent. Factoriser reviendrait à toucher le chemin `label_write` — en
production depuis mika#2228, avec 34 refus mesurés derrière lui — pour un gain
de ~15 lignes et un risque de régression sur un chemin que ce ticket n'a aucune
raison de bouger.

**Aucun autre site ne touche la clé privée** (AC1, seconde phrase). La clé
n'est lue que par `github_app.rs` ; `resolve_ci_read_token` prend un
`Option<&GitHubApp>` déjà construit, comme ses deux voisins. Ce plan n'ajoute
aucun `from_settings` ni aucun accès à `MIKA_GITHUB_APP_PRIVATE_KEY*`.

### D3 — Le repli REST du corps d'origine est **retiré**

Le corps recadré demande au plan de dire s'il garde la voie par
`GET /repos/{r}/commits/{sha}/status` + `GET /repos/{r}/actions/runs`. **Il la
retire**, pour trois raisons mesurées :

1. **Les trois dépôts concernés sont dans l'installation.** Vincent l'a vérifié
   le 2026-10-29 à 15:16Z : control-monitor, mika-cloud et mika-platform sont
   dans les dépôts sélectionnés de l'installation 123032070. Les 77 échecs
   mesurés sont donc tous couverts par le jeton App. Le repli REST n'aurait
   aucune occurrence à réparer.
2. **Il serait un lecteur partiel, et la contrainte de samidarko l'interdit.** Le
   corps d'origine le dit lui-même : « Un check posé par une App tierce (check
   run hors Actions, sans commit status) reste invisible au repli. » Un lecteur
   qui ne voit qu'une partie des checks, hérité par `pr_merge_with_gate`, est
   exactement le « tout vert » que la contrainte refuse — et mika#2617 a déjà
   payé ce défaut une fois, avec `main` rouge sur `41a4a20e`.
3. **Pour un dépôt hors installation, le comportement correct existe déjà.** Le
   PAT reste le repli (D4), la lecture `statusCheckRollup` échoue, et le gate
   bloque sur `GhCliFailure`. C'est fermé, c'est dit dans le log, et c'est le
   régime d'aujourd'hui. Remplacer un refus honnête par une lecture partielle
   serait une régression de sûreté déguisée en amélioration de couverture.

Le matériau mesuré du corps d'origine (control-monitor#217, tête `8ab600b1`)
reste dans le ticket comme trace ; aucun code ne le consomme.

---

## 3. Le geste

### R1 — `crates/mika-common/src/ci_read.rs` (neuf)

```rust
pub enum CiReadTokenSource { App, Pat }   // + as_str() -> "app" | "pat"
pub struct CiReadToken { token: String, source: CiReadTokenSource }
impl CiReadToken {
    pub fn new(token: String, source: CiReadTokenSource) -> Self
    pub fn token(&self) -> &str
    pub fn source(&self) -> CiReadTokenSource
    pub fn report_read(&self, repo: &str, pr: u64)   // AC4, voir R4
}
impl fmt::Debug for CiReadToken   // rédigé : jamais le token
```

`Clone` dérivé — R5 en a besoin. `Debug` manuel sur le modèle de
`LabelWriteToken` (`label_write.rs:126`) : il imprime la source, jamais le
jeton. Déclaré dans `crates/mika-common/src/lib.rs`.

### R2 — `crates/mika-common/src/config.rs` : `resolve_ci_read_token`

Corps identique en forme à `resolve_label_write_token` : App d'abord, et sur
échec d'échange un WARN `gh_app_token_exchange_failed` portant
`scope = "ci_read"` et `has_pat_fallback`, puis le PAT. `None` quand ni l'un ni
l'autre n'est disponible.

La doc de la fonction porte trois choses, parce qu'elles ne se redécouvrent
pas : la décision D2 (pourquoi un troisième résolveur et pas une factorisation),
le renvoi croisé vers les deux voisins, et la règle d'AC1 (la clé privée reste
chez `github_app.rs`). Les docs de `resolve_github_token` et
`resolve_label_write_token` gagnent chacune une ligne de renvoi vers le
troisième — c'est la seule modification de ces deux fonctions.

### R3 — Le lecteur prend un `&CiReadToken`

`run_gh_checks_raw(pr_number, repo, token: &CiReadToken)` et
`run_gh_checks(pr_number, repo, token: &CiReadToken)`. `run_gh_subprocess`
reste sur `&str` et reçoit `token.token()` : le site d'injection de `GH_TOKEN`
ne bouge pas.

`gh_checks_args` n'est pas touché — ni le `--json`, ni l'absence de
`--required` (mika#2617). Ce plan change **par quel jeton** la lecture passe,
jamais **ce qu'elle lit**.

La doc de `run_gh_checks_raw` gagne un paragraphe : elle est le seul lecteur de
« quel check bloque » pour cinq consommateurs, et depuis ce ticket elle est
aussi le seul site qui dit quel jeton a lu.

### R4 — Le log de provenance (AC4)

`CiReadToken::report_read(repo, pr)` émet, **à chaque lecture**, sur
`target: "mika::github_auth"` :

```
event = "ci_read_token_source", source = "app" | "pat", repo, pr
```

Appelé depuis `run_gh_checks_raw`, le site unique — donc les cinq consommateurs
l'héritent, vol 2 compris, sans une ligne chez eux.

Niveau `info!`, et c'est un choix contre la doctrine mika#2131 (« rien n'est
émis sur le chemin nominal »), assumé parce qu'AC4 l'exige nommément et cite
mika#2205 : le silence d'un repli doit se distinguer du silence d'un lecteur
sain. Un `debug!` ne serait pas comptable par la sonde AC5. Le jeton n'apparaît
jamais — `report_read` n'a accès qu'à `self.source()`, et le `Debug` rédigé de
R1 ferme la seconde porte.

Les deux populations restent séparables : `scope = "ci_read"` sur l'échec
d'échange (R2) contre `scope = "label_write"` de mika#2228, et
`event = "ci_read_token_source"` pour la lecture réussie.

### R5 — Le repli de câblage, à un seul site

```rust
// crates/mika-agent/src/tools/pr_merge_with_gate.rs
pub(crate) fn ci_read_token_or_pat(resolved: Option<&CiReadToken>, pat: &str) -> CiReadToken
```

Rend le `CiReadToken` résolu s'il existe, sinon un `CiReadToken` de source `Pat`
bâti sur le PAT de l'appelant.

**Pourquoi ce repli, et pourquoi il ne contredit pas AC3.** Sans lui, un site de
production dont le câblage serait oublié — ou un `ToolContext` de test — ferait
refuser le gate sur une cause de configuration qui n'existe pas : une panne
inventée par le câblage, pas par la forge. Avec lui, un tel site dégrade vers
**exactement** le comportement d'avant ce ticket : lecture au PAT, échec sur
`statusCheckRollup`, gate fermé sur `GhCliFailure`. Le fail-closed d'AC3 n'est
pas porté par ce repli mais par le cas « ni App ni PAT », déjà refusé en tête de
`pr_merge_with_gate::execute` (l. 169) et pinné par T2.

C'est aussi le site que le détecteur de la section Fire-Disposition protège :
toute autre construction d'un `CiReadToken` de source `Pat` dans
`crates/mika-agent/src` serait un contournement silencieux de la résolution
App-first.

### R6 — `ToolContext` porte le jeton de lecture

`crates/mika-agent/src/tools/mod.rs` : nouveau champ, voisin immédiat de
`github_token` (l. 127), avec la doc qui dit la séparation :

```rust
/// Le jeton des **lectures CI** (mika#2963) — App d'abord, PAT en repli.
/// Jumeau non identitaire de `github_token`, qui reste le jeton
/// d'**exécution** (`pr review`, `pr merge`, `pr comment`).
pub ci_read_token: Option<&'a mika_common::ci_read::CiReadToken>,
```

20 littéraux `ToolContext { … }` existent dans `crates/mika-agent/src`
(`agent_loop/mod.rs` 3, `tools/send_message.rs` 6, `test_utils.rs` 5,
`tools/toggle_skill.rs` 2, `skills/builtin_handlers.rs` 2,
`server/investigate.rs` 1, `tools/pr_merge_with_gate.rs` 1). Les trois de
`agent_loop/mod.rs` sont les sites de production (l. 6629, 7856, 8597) ; les
autres reçoivent `None` et dégradent par R5. Aucun littéral n'existe sous
`crates/mika-agent/tests/` — les tests passent par `test_utils.rs`.

**Le site unique du token d'exécution, extrait pour être observable (D1) :**

```rust
// crates/mika-agent/src/skills/builtin_handlers.rs
fn gh_exec_token<'a>(ctx: &'a ToolContext<'_>) -> Option<&'a str> { ctx.github_token }
```

`run_gh` l'appelle à la place de `ctx.github_token` (l. 5383). Même valeur,
mais désormais un nom que T1 peut épingler — le même geste que
`gh_checks_args`, extrait par mika#2617 pour la même raison : rendre décidable
sans réseau « avec quoi cette commande part-elle ».

### R7 — La résolution, aux trois sites de production

`crates/mika-agent/src/agent_loop/mod.rs` résout déjà
`resolved_github_token = settings.resolve_github_token(params.github_app)` à
trois endroits (l. 6268, 7807, 8460), puis le pose dans `ToolContext`
(l. 6629, 7856, 8597). Un `resolved_ci_read_token =
settings.resolve_ci_read_token(params.github_app)` s'y ajoute, strictement
symétrique — même `params.github_app`, même portée, même `else` sur
`params.github_token` quand `settings` est `None`.

### R8 — Les deux consommateurs de ce vol

- `pr_merge_with_gate::execute` (l. 261) :
  `run_gh_checks(pr_number, repo, &ci_read_token_or_pat(ctx.ci_read_token, token))`.
  Le refus en tête sur `ctx.github_token == None` (l. 169) **ne bouge pas** :
  c'est lui qui porte AC3.
- `validate_qa_ci_coherence` (`builtin_handlers.rs:4380`) : la clôture injectée
  dans `validate_qa_ci_coherence_with_reader` passe le même
  `ci_read_token_or_pat(ctx.ci_read_token, &token)`. La seam du garde reste le
  **stdout brut** — le choix de mika#2455 n'est pas touché, donc ses tests
  existants continuent d'exercer le vrai parseur et le vrai classifieur.

Les trois handlers webhook restent sur `ci_read_token_or_pat(None, token)` —
leur comportement d'aujourd'hui, inchangé, et le log R4 le dira honnêtement
(`source = "pat"`). Vol 2 : mika#2965.

---

## 4. Ce que le plan ne touche pas

- `gh_checks_args` : ni le `--json`, ni l'absence de `--required` (mika#2617).
- `resolve_github_token` et le PAT de `mika-platform-qa` : approve, merge,
  review et comment restent dessus (D1).
- `classify_checks`, `decide_merge_gate`, `classify_ci_coherence` : aucune
  classification ne change. Le jeton change, la décision non.
- `docs/egress/egress-manifest.toml` : aucun sink neuf (§ 1.4).
- `label_write.rs` et `resolve_label_write_token` : seule une ligne de renvoi
  croisé dans la doc (D2).
- Le contenu de la CI de control-monitor et la protection de branche de cm :
  hors périmètre déclaré par le ticket.

---

## 5. Les tests

### T1 — AC2 : `pr review` part avec le PAT

`crates/mika-agent/src/skills/builtin_handlers.rs`, module de tests. Un
`ToolContext` portant **deux valeurs distinctes** (`github_token =
Some("pat-mika2963")`, `ci_read_token = Some(CiReadToken::new("app-mika2963",
App))`). Assertions :

- `gh_exec_token(&ctx) == Some("pat-mika2963")` — le token d'exécution de
  `run_gh`, donc du `pr review`, est le PAT même quand l'App est disponible ;
- la clôture lecteur de `validate_qa_ci_coherence` reçoit `"app-mika2963"` —
  capturée par un `read_checks` injecté dans
  `validate_qa_ci_coherence_with_reader`, qui enregistre le token reçu.

Les deux dans le même test : c'est leur **divergence** qui est l'observable, et
deux tests séparés passeraient tous les deux sur un arbre où les deux chemins
lisent le même jeton.

### T2 — AC3 : le gate refuse quand l'échange App échoue **et** le PAT aussi

`crates/mika-agent/tests/eval/`, sur le patron de
`test_qa_ci_coherence_2455.rs::harness_without_token`. `settings.github_token =
None`, pas d'App : `resolve_ci_read_token` rend `None`, `ctx.github_token` rend
`None`, et `pr_merge_with_gate::execute` refuse en tête. L'observable est le
refus, jamais un merge.

Second cas, celui qui compte le plus : échange App échoué, PAT présent et
incapable de lire. `ci_read_token_or_pat` rend un jeton de source `Pat`, la
lecture échoue, et le gate rend `GateError { GhCliFailure }`. Assertion :
aucune branche de `decide_merge_gate` ne voit une liste de checks vide, donc
`AllPassed` n'est jamais atteint par un échec de lecture. C'est la phrase « il
ne lit jamais un échec comme du vert », écrite comme assertion.

### T3 — AC1 : la cascade, aux trois issues

`crates/mika-common/src/config.rs`, module de tests, sur le patron de
`test_resolve_github_token_*` (l. 3481+) :

- App disponible (`GitHubApp::new_with_test_token`) + PAT configuré → source
  `App` ;
- pas d'App, PAT configuré → source `Pat` (le repli) ;
- ni l'un ni l'autre → `None`.

Plus un test que le `Debug` de `CiReadToken` n'imprime jamais le jeton, sur le
patron de `label_write.rs::debug_never_prints_the_token` (l. 196), et un test
que les libellés `"app"` / `"pat"` sont stables — ils sont un format de fil, lu
par la sonde AC5.

### T4 — AC4 : la provenance est dite, et elle distingue les deux populations

Capture de souscripteur `tracing` autour d'un `run_gh_checks_raw` à lecteur
injecté : l'événement `ci_read_token_source` est émis, porte
`source = "app"`, le `repo` et le `pr`, et **ne contient pas** la chaîne du
jeton. Puis le même test avec un jeton de source `Pat` : `source = "pat"`.

### T5 — le détecteur (voir Fire-Disposition)

`mika2963_la_source_pat_ne_se_construit_qu_au_site_du_repli` : scan structurel
de `crates/mika-agent/src/**/*.rs` refusant toute construction
`CiReadToken::new(` hors du corps de `ci_read_token_or_pat`.

---

## 6. Ordre de commit

1. `crates/mika-common/` — `ci_read.rs`, `lib.rs`, `resolve_ci_read_token` + T3.
   Compile et passe seul ; `mika-agent` n'en dépend pas encore.
2. `pr_merge_with_gate.rs` — signatures de `run_gh_checks*`, `report_read` au
   site unique, `ci_read_token_or_pat`. Casse la compilation des cinq
   consommateurs : c'est voulu, le compilateur énumère le recensement du § 1.2.
3. `tools/mod.rs` + les 20 littéraux `ToolContext` + les trois résolutions de
   `agent_loop/mod.rs` + `gh_exec_token`. L'arbre recompile ici.
4. Les deux consommateurs du vol (R8) + les trois handlers passés à
   `ci_read_token_or_pat(None, token)`.
5. T1, T2, T4, T5.

Chaque commit laisse `cargo build` vert sauf le 2, dont le 3 est l'inséparable
suite — ils peuvent fusionner si la revue le préfère.

---

## 7. Vérification

```
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
make verify-bundled-skills
scripts/verify-pipeline.sh origin/main
```

Plus les lints du dépôt que ce diff peut réveiller :
`scripts/check-byte-slices.sh`, `scripts/check-suite-wiring.sh` (T5 vit dans
`cargo test`, pas sous `scripts/` — rien à déclarer), et
`scripts/verify-egress-manifest.sh` (vert par construction, § 1.4).

---

## 8. Sonde après déploiement (AC5)

Après `make deploy`, sur 7 jours, dans `/var/log/mika/server.log` :

```sh
# AC5 — abstentions du garde mika#2455, par motif et par dépôt
grep -h 'qa_ci_coherence_abstained' /var/log/mika/server.log \
  | jq -r '[.fields.reason // .reason, .fields.repo // .repo] | @tsv' \
  | sort | uniq -c | sort -rn

# AC4 — provenance des lectures CI, par source et par dépôt
grep -h 'ci_read_token_source' /var/log/mika/server.log \
  | jq -r '[.fields.source // .source, .fields.repo // .repo] | @tsv' \
  | sort | uniq -c | sort -rn
```

Cible : **0** pour `reason = "gh_failed"` sur `senara-solutions/mika-cloud`,
`senara-solutions/mika-platform` et `senara-solutions/control-monitor` — les
trois dépôts dont Vincent a vérifié le 2026-10-29 qu'ils sont dans
l'installation 123032070. Référence à battre : 55 abstentions `gh_failed` du
2026-10-21 au 2026-10-29.

La seconde requête est ce qui rend la première lisible. Un `0` sur `gh_failed`
accompagné d'un compte nul de `ci_read_token_source` ne prouve rien — c'est un
lecteur qui ne tourne pas. Un `0` accompagné de `source = "app"` sur les trois
dépôts est la réparation. Un `0` accompagné de `source = "pat"` seulement
signifie que l'échange App échoue en silence et que le défaut dort : lire alors
les WARN `gh_app_token_exchange_failed` portant `scope = "ci_read"`.

Le compte des deux sources sur les trois handlers du vol 2 restera `"pat"` : ce
n'est pas une anomalie, c'est le périmètre de mika#2965.

---

## 9. CODEOWNERS — la PR ne ferme pas en autonome

`.github/CODEOWNERS` attribue `/crates/mika-agent/src/tools/pr_merge_with_gate.rs`
à `@samidarko` (forge-gate perimeter, DECISION-CORE). Ce plan le touche
nécessairement (R3, R5, R8). Conséquences à anticiper :

- `pr_merge_with_gate::execute` classera la PR `DecisionCore` et bloquera son
  propre merge (`HumanGateRequired`) — comportement correct, pas un défaut ;
- la PR porte `Closes #2963` et attend l'approbation puis le merge de Vincent ou
  samidarko ;
- le label `human-review-required` est posé à l'ouverture.

---

## Definition of Done

- `crates/mika-common/src/ci_read.rs` existe, déclaré dans `lib.rs`, avec un
  `Debug` qui ne peut pas imprimer le jeton.
- `Settings::resolve_ci_read_token` existe, App d'abord, PAT en repli, WARN
  `scope = "ci_read"` sur échec d'échange.
- `run_gh_checks` et `run_gh_checks_raw` prennent un `&CiReadToken` et émettent
  `ci_read_token_source` au site unique.
- `ToolContext::ci_read_token` existe, peuplé aux trois sites de production de
  `agent_loop/mod.rs`, `None` ailleurs.
- `pr_merge_with_gate::execute` et le garde mika#2455 lisent par
  `ci_read_token_or_pat(ctx.ci_read_token, token)`.
- Les trois handlers webhook compilent et lisent au PAT, inchangés
  fonctionnellement.
- T1 à T5 passent. `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`,
  `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main` passent.
- Le corps de PR porte `Closes #2963`, `Tracked in: senara-solutions/mika#2965`,
  et `Code review: delegated to mika-qa (mika#2694)`.

---

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2963 (section « Critères
d'acceptation (remplacent ceux du corps d'origine) »).

- **AC1.** Le lecteur CI obtient son jeton par un résolveur App-first, sur le
  patron de `resolve_label_write_token`, avec le PAT en repli. Le plan dit si ce
  résolveur est neuf ou si c'est l'existant, et pourquoi. Aucun autre site ne
  touche la clé privée. → D2, R2, T3.
- **AC2.** Le point d'identité est tranché par écrit entre (i) et (ii). Si c'est
  (i), un test montre que le `pr review` est toujours posté avec le jeton PAT.
  → D1, R6, T1.
- **AC3.** Gate de merge : un test montre que `pr_merge_with_gate` refuse le
  merge quand l'échange de jeton App échoue et que le repli PAT échoue aussi. Il
  ne lit jamais un échec comme du vert. → R5, R8, T2.
- **AC4.** Chaque lecture CI dit dans le log quel jeton l'a faite (App ou PAT),
  sans jamais écrire le jeton. Le silence d'un repli doit se distinguer du
  silence d'un lecteur sain (mika#2205). → R4, T4, § 8.
- **AC5.** Sonde après déploiement, sur 7 jours : le compte de
  `qa_ci_coherence_abstained` par `reason` et par dépôt est écrit dans le ticket,
  avec la requête. La cible est 0 pour `gh_failed` sur mika-cloud, mika-platform
  et control-monitor, ces dépôts étant dans l'installation. → § 8.

---

## Fire-Disposition

Le plan livre un détecteur : **T5**,
`mika2963_la_source_pat_ne_se_construit_qu_au_site_du_repli`. Scan structurel
de `crates/mika-agent/src/**/*.rs` qui refuse toute construction
`CiReadToken::new(` hors du corps de `ci_read_token_or_pat`
(`tools/pr_merge_with_gate.rs`). C'est bien un détecteur au sens de mika#2306 :
son chemin de succès est « aucune violation trouvée ».

Ce qu'il protège : la régression qui ramène silencieusement le PAT. Un site qui
bâtirait son propre `CiReadToken::new(pat, Pat)` contournerait
`resolve_ci_read_token`, lirait au PAT, échouerait sur `statusCheckRollup` — et
le log R4 dirait honnêtement `source = "pat"`, ce qui ressemble au repli
nominal. Le défaut serait donc **invisible dans l'instrument même qui le mesure**.
Seule la structure le décide.

**Disposition retenue : (a), exception nommée en allowlist — et l'allowlist est
vide à la livraison.**

Détail d'implémentation :

- la liste des sites autorisés est un `const` grep-visible du test,
  `SITES_AUTORISES: &[&str] = &["ci_read_token_or_pat"]`, avec en commentaire le
  ticket qui le justifie (mika#2963) ;
- le recensement au moment de l'écriture de ce plan donne **zéro** construction
  de `CiReadToken::new` dans `crates/mika-agent/src` — le type n'existe pas
  encore. Le détecteur atterrit donc **armé**, avec aucune exception par donnée
  à inscrire ;
- l'assertion auto-nettoyante porte sur l'allowlist elle-même : le test échoue
  aussi quand un nom de `SITES_AUTORISES` ne correspond plus à aucune
  construction trouvée. Une entrée devenue stale rougit au lieu de dormir —
  c'est ce qui empêche la liste de devenir l'allowlist que mika#2309 condamne ;
- les constructions sous `#[cfg(test)]` (T1, T3, T4 en bâtissent) sont exclues
  par chemin de lecture : le scan ne lit que le code hors module de tests, la
  frontière étant le `#[cfg(test)] mod tests` de chaque fichier. La raison est
  que l'objet protégé est le **chemin de production** : un test qui bâtit un
  jeton de source `Pat` est précisément ce que T2 doit faire.

Le détecteur vit dans `cargo test`, pas sous `scripts/` ni
`skills/bundled/_shared/` : `scripts/check-suite-wiring.sh` n'a rien à en
connaître, et `scripts/suite-wiring.tsv` n'est pas touché.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-common/src/ci_read.rs` (type, source, `report_read`, `Debug` rédigé) | 55 |
| tests de `ci_read.rs` (`Debug` ne fuit pas, libellés stables) | 25 |
| `crates/mika-common/src/config.rs` (`resolve_ci_read_token` + doc + renvois croisés) | 45 |
| tests de `config.rs` (T3 — trois issues de la cascade) | 40 |
| `crates/mika-agent/src/tools/pr_merge_with_gate.rs` (signatures R3, log R4, `ci_read_token_or_pat` R5) | 45 |
| `crates/mika-agent/src/tools/mod.rs` (champ `ToolContext` + doc) | 25 |
| `crates/mika-agent/src/agent_loop/mod.rs` (3 résolutions + 3 peuplements) | 30 |
| `crates/mika-agent/src/skills/builtin_handlers.rs` (garde mika#2455, `gh_exec_token`) | 10 |
| autres littéraux `ToolContext` (`send_message`, `toggle_skill`, `test_utils`, `investigate`, `pr_merge_with_gate`) + 3 handlers webhook | 15 |
| `crates/mika-common/src/lib.rs` | 1 |
| tests T1, T2, T4 et détecteur T5 | 110 |

Total estimé : 401
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient, et pourquoi la découpe tombe là.** 401 < 429, avec
28 lignes de marge. La découpe est au **recensement des consommateurs**, pas au
milieu d'un mécanisme : le vol 1 livre le résolveur, le type, le lecteur, son
instrument, et les **deux** consommateurs qui portent AC3 (le gate) et AC5 (le
garde mika#2455 — l'unique producteur des 55 abstentions mesurées). Les cinq AC
sont donc satisfaits par ce vol seul.

Le vol 2 n'élargit que la couverture App aux trois handlers webhook, qui
restent d'ici là sur leur comportement d'aujourd'hui, mot pour mot — repli par
`ci_read_token_or_pat(None, token)`, et le log R4 le dit (`source = "pat"`)
plutôt que de le taire. Ce n'est pas une régression différée, c'est un périmètre
non encore élargi, et l'instrument le déclare.

Tracked in: senara-solutions/mika#2965

---

## Hors périmètre

- Le repli par `commits/{sha}/status` et `actions/runs` du corps d'origine :
  **retiré**, raisons en D3.
- La voie (ii), bascule de toute la QA sur l'identité App : **écartée**, raisons
  en D1.
- Le contenu de la CI de control-monitor (un `ci.yml` est confié à SSC) et la
  protection de branche de cm : hors périmètre déclaré par le ticket.
- La liste des dépôts de l'installation 123032070 : vérifiée par Vincent le
  2026-10-29 à 15:16Z, prise comme donnée.
- Le câblage App des trois handlers webhook : mika#2965.
- `scripts/`, les lints CI, `skills/bundled/` : aucun fichier touché.

---

Lié : mika#2455 (le garde qui s'abstient), mika#2228 (`resolve_label_write_token`,
le patron), mika#2617 (le retrait de `--required`, et le `main` rouge qui a
fixé la direction du fail-closed), mika#2961, mika#2205 (un instrument inerte
se lit comme un instrument sain), mika#2965 (vol 2).
