# Vol V4-b1 — `mika ask --team` passe par spirit (mika#1727)

- **Ticket :** senara-solutions/mika#1727, vol **V4-b1**
- **Branche :** `feat/1727/v4b1-mika-ask-team-passe-par-spirit`, neuve depuis `main` (`9da388ac`)
- **PR :** `Refs mika#1727`, jamais `Closes` / `Fixes` / `Resolves`
- **Suite :** `Tracked in: senara-solutions/mika#1727`
- **Label de borne :** `loop-substrate`

## Contexte

V4-a (#3109, `9da388ac`) a livré la route : `POST /api/v1/teams/{name}/runs` lance une
équipe côté spirit et en diffuse les événements en SSE, sous `require_internal_token`.
Elle n'a **aucun client**. `mika ask --team` lance encore l'équipe dans son propre
processus : `commands::ask::run_team_ask` appelle `mika_agent::teams::run_team`
(`crates/mika-cli/src/commands/ask.rs:852`), avec les `Settings`, l'App GitHub, la base
d'équipe et le tier/déploiement lus du process CLI.

Ce vol écrit le client de la route et y branche `mika ask --team`. **Le mode équipe de
la TUI vient en V4-b2 ; le verrou structurel vient en V4-c.** Le ticket se ferme au
merge de V4-c, et seulement là.

### Ce qui est mesuré sur main (`9da388ac`)

| fait | site |
|---|---|
| `TeamRunFrame`, `#[serde(tag = "event", rename_all = "snake_case")]`, trois variantes `team_event` / `run_completed` / `run_error`, **`pub` et `Deserialize`** | `crates/mika-agent/src/server/team_runs.rs:60`, exporté par `pub mod team_runs` (`server/mod.rs:35`) et `pub mod server` (`lib.rs:62`) |
| `TeamEvent` : `#[serde(tag = "kind", content = "data")]`, `Deserialize`, **sans `#[serde(other)]`** — un `kind` inconnu fait donc échouer le décodage de la trame entière | `crates/mika-agent/src/teams/types.rs:301` |
| `TeamRun` : `Deserialize`, tous les champs sauf `run_id` / `team_name` / `goal` en `#[serde(default)]` | `crates/mika-agent/src/teams/types.rs:71` |
| `drain_events(buffer, on_payload)` — découpeur SSE, **privé** | `crates/mika-cli/src/commands/tasks.rs:1039` |
| `TaskEventStream::open` / `consume` : URL et jeton **par argument**, statut lu avant le corps, aucun délai global, lecture par `Response::chunk()` | `crates/mika-cli/src/commands/tasks.rs:512-640` |
| `jeton_interne(SourceEnv)` — seule lecture de `MIKA_INTERNAL_TOKEN` du crate ; son refus nomme la variable | `crates/mika-cli/src/commands/session_remote.rs:177` |
| `spirit_url()` | `crates/mika-cli/src/commands/dashboard.rs:18` |
| `fake_spirit(&[(chemin, statut, corps)])` → `(base_url, Arc<Mutex<Vec<RequeteRecue>>>)`, `pub(crate)` ; `RequeteRecue` relève méthode, chemin, `authorization`, corps | `crates/mika-cli/src/commands/rewind_remote.rs:580` et `:535` |
| Garde textuelle par `ProductionScanner` + allowlist **livrée vide** | `crates/mika-cli/src/commands/chat_turn.rs:1176` et `:1215` |
| Sinks voisins d'egress ; le scanner coupe la production au premier `#[cfg(test)]` | `docs/egress/egress-manifest.toml:374` et `:385` |
| `crates/mika-cli/CLAUDE.md` : **89 486 octets** pour un plafond de 100 000 | `crates/mika-common/tests/claude_md_size.rs` |

### Trois réutilisations, et c'est elles qui font tenir le vol

Un client SSE écrit de zéro coûte un découpeur, un miroir de types et un banc de test.
Ce vol n'en écrit aucun des trois :

1. **Les types du flux voyagent tels quels.** `TeamRunFrame`, `TeamEvent` et `TeamRun`
   sont `pub` et `Deserialize` ; `mika-cli` dépend déjà de `mika-agent` et `ask.rs`
   importe déjà `mika_agent::teams::types::TeamEvent`. Pas de miroir local — un miroir
   serait une seconde copie libre de divergier sur le nom d'une variante, et le
   décodage est précisément ce qu'on veut lockstep avec le producteur.
2. **Le découpeur SSE est celui de `tasks.rs`.** `drain_events` passe en `pub(crate)` :
   un seul changement, et les deux flux SSE du crate partagent un découpeur dont la
   propriété de re-raboutage est déjà attestée là où elle vit.
3. **Le banc de test est `fake_spirit`.** Un corps SSE fixe par chemin suffit : ce vol
   ne mesure aucune déconnexion serveur.

C'est pourquoi ce vol tient sous sa borne sans que le périmètre soit recoupé.

## Requirements

### R1 — Le client (`crates/mika-cli/src/commands/team_runs_remote.rs`, fichier neuf)

Une porte, `pub(crate) async fn lancer_run_equipe`, qui reçoit `base_url`, le jeton,
le nom d'équipe, le but, le `reference_run_id` et un rappel, **et ne lit rien de
l'environnement** — même raison qu'écrite sur `TaskEventStream::open` : un consommateur
qui résout ses propres accès ne se pilote pas depuis un test.

- `POST` vers `{base}/api/v1/teams/{name}/runs`, en-tête `Authorization: Bearer <jeton>`,
  corps JSON `{"goal": …, "reference_run_id": …}` (le second omis quand absent).
- **Client `reqwest` construit au site de l'appel** — c'est ce que déclare l'entrée
  d'egress, qu'un client passé en argument rendrait aveugle (motif `session_remote.rs`).
- **Aucun délai global** : un run d'équipe dure des minutes. Le commentaire le dit, pour
  qu'un futur `.timeout(…)` « par hygiène » se lise comme la régression qu'il serait.
- **Le statut se lit avant le corps.** 400, 401, 404 et tout autre statut non 200 rendent
  chacun un refus nommant le statut, l'URL et le corps ; le 401 rappelle
  `MIKA_INTERNAL_TOKEN`. Une erreur de transport rend un refus nommant l'URL.
- Lecture : `Response::chunk()` dans un `while let`, `drain_events` sur le tampon,
  décodage de chaque `data:` en `TeamRunFrame`.
  - `team_event` → le `TeamEvent` passe au rappel, **dans l'ordre reçu** ;
  - `run_completed` → la porte rend le `TeamRun` de la trame et s'arrête ;
  - `run_error` → la porte rend l'erreur du run ;
  - **trame indécodable** → une ligne sur stderr, et la lecture continue. Un `kind`
    inconnu de `TeamEvent` tombe ici par construction (`types.rs:295` : pas de
    `#[serde(other)]`) ;
  - **flux fermé sans trame finale** → un échec **distinct**, jamais un succès
    (défaut 33). C'est le seul chemin où la boucle sort sans trame terminale, donc le
    distinguer est structurel, pas une garde à armer.
- Un refus typé, `TeamRunRemoteIssue`, à **cinq** variantes et un `Display` à la main
  (motif `SessionRemoteIssue`, `session_remote.rs:105`) : `Denied { url }`, `Refus { status,
  url, body }`, `Transport { url, cause }`, `SansTrameFinale { url }`, `RunError { error }`.
  Un enum et non un `anyhow::Error` nu : le test 4 doit distinguer « fermé sans finale »
  de tout autre échec, et un `contains` sur un message mesurerait la phrase, pas le fait.

  *Amendé par le pilote (vol V4-b1), sur deux points et pour deux raisons.* Le compte
  d'abord : cette phrase disait « quatre » tout en en énumérant cinq, et c'est le compte
  qui était faux — l'énumération est celle que le périmètre du ticket exige (un refus par
  statut non-200, un pour le transport, un pour le flux sans finale, un pour l'échec du
  run). Le champ de `Denied` ensuite : le critère 4 de la tête exige que **tout** refus
  nomme l'URL, et `base_url` étant un argument, « quel serveur a refusé » est la moitié de
  la question dès que deux daemons tournent. `Denied` ne porte **que** l'URL — ni statut ni
  corps — sur le précédent de `skills_remote::SkillsRemoteIssue::JetonRefuse` : le 401 est
  déjà dans le nom, et le corps d'un 401 d'ici est vide ou générique.

### R2 — L'entrée d'egress, dans le même commit que le client

```toml
[[sink]]
id                 = "cli-spirit-team-runs"
destination        = "MIKA_SPIRIT_URL (défaut http://localhost:8080)"
destination_source = "config"
class              = "internal"
data               = "un POST vers /api/v1/teams/{name}/runs : le nom d'équipe dans le chemin, le but et le run de référence facultatif dans le corps JSON, le jeton interne en en-tête Authorization ; la réponse est un flux SSE"
logged             = false
client_site        = "crates/mika-cli/src/commands/team_runs_remote.rs"
call_site          = "crates/mika-cli/src/commands/team_runs_remote.rs"
owner              = "@samidarko"
```

Le lint est un lockstep bidirectionnel (`scripts/verify-egress-manifest.sh`, mika#2408) :
le `reqwest::Client::new()` du fichier neuf **exige** cette ligne, et la ligne ne peut
pas précéder le sink. D'où « même commit », qui n'est pas une préférence de découpage.

### R3 — `drain_events` en `pub(crate)`

Un seul changement dans `crates/mika-cli/src/commands/tasks.rs:1039`, plus deux lignes
de doc disant **pourquoi** la visibilité s'élargit : deux flux SSE dans le crate, un
découpeur. Rien d'autre de ce fichier ne bouge.

### R4 — `run_team_ask` branché

- La vérification du but et de `--run-id` **reste telle quelle** (défaut 35) : elle
  ouvre la base locale pour valider la référence, et c'est hors du périmètre de ce vol.
- L'URL et le jeton se résolvent **à l'entrée de la fonction**, par `spirit_url()` et
  `jeton_interne(&|cle| std::env::var(cle).ok())`. Une absence de jeton échoue **avant
  tout appel**.
- Le rappel imprime les **mêmes lignes stderr** qu'aujourd'hui (`ask.rs:816-847`) :
  le bloc `match event` est inchangé, seul son type d'accueil change.
- Le `TeamRun` reçu passe par le **même** `team_outcome::classify`, le même `eprintln!`
  de diagnostic, et les trois formats écrivent ce qu'ils écrivent aujourd'hui.
- **Aucun repli local** (défaut 32). Un refus, une erreur de transport, un `run_error`
  ou un flux sans finale sortent en échec : `bail!` avant tout `println!`, donc stdout
  reste vide **par conséquence de l'ordre des gestes**, non par politique — même
  argument qu'écrit sur `get_task_detail_from_spirit_into` (`tasks.rs:1016`).
- **Ce qui ne sert plus part avec l'appel** : `Settings::load`, `GitHubApp::from_settings`,
  `teams::open_container_db_async` + son `shutdown`, `AgentTier::from_env` et
  `Deployment::from_env`. Le tier et le déploiement sont désormais ceux de l'agent par
  défaut de spirit (défaut 34), résolus par `state.resolve_agent("")` côté route.
- Les deux bras `Json` et `Yaml` construisent aujourd'hui **la même**
  `AskTeamJsonResponse`, dupliquée. Un seul constructeur,
  `enveloppe_equipe(&run, failure_reason) -> AskTeamJsonResponse`, remplace les deux :
  c'est le point de mesure du test 7, et le compilateur tient qu'il n'y en a pas d'autre.

### R5 — La garde textuelle

Dans les tests d'`ask.rs`, sur le modèle de `chat_turn.rs:1176` : `ProductionScanner`
sur `ask.rs`, aiguille `teams::run_team(` composée par `concat!` (sans quoi le test
contiendrait sa propre aiguille), **sans allowlist**. Le message de refus nomme le
remède — router par `team_runs_remote::lancer_run_equipe` — et dit qu'il n'y a pas
d'exception à élargir.

### R6 — La doc

Un paragraphe de `crates/mika-cli/CLAUDE.md` : `mika ask --team` passe par spirit, quel
jeton il demande, qu'il n'a **aucun repli local**, que le tier et le déploiement sont
désormais ceux de l'agent par défaut de spirit (défaut 34), et qu'un flux fermé sans
trame finale **ne désigne pas le run à relire** (défaut 33). Le fichier reste sous
100 000 octets (89 486 aujourd'hui).

## Périmètre — ce qui change, et rien d'autre

**Seuls ces fichiers :**

- `crates/mika-cli/src/commands/team_runs_remote.rs` (neuf)
- `crates/mika-cli/src/commands/mod.rs` (une déclaration)
- `crates/mika-cli/src/commands/tasks.rs` (R3, et rien d'autre)
- `crates/mika-cli/src/commands/ask.rs` (`run_team_ask` et ses tests)
- `crates/mika-cli/CLAUDE.md` (un paragraphe)
- `docs/egress/egress-manifest.toml` (l'entrée `cli-spirit-team-runs`)
- `docs/plans/` (ce plan)

**Restent intacts :** tout `crates/mika-agent/`, `crates/mika-cli/src/commands/chat.rs`,
tout `crates/mika-cli/src/tui/`, `commands/team_outcome.rs`, `crates/mika-common/`,
`crates/mika-a2a/`, `crates/mika-gateway/`, `dashboard/`.

**Aucun fichier du plan de contrôle** : ni `.github/workflows/`, ni `.claude/`, ni
`skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni `.mika/`, ni
aucun chemin de CODEOWNERS autre que `docs/egress/egress-manifest.toml`, touché pour
cette entrée et pour elle seule.

**Hors de ce vol :** le mode équipe de la TUI (`chat.rs`, `tui/`) → V4-b2 ; le verrou
structurel → V4-c ; toute retouche de la route ou de `crates/mika-agent/`.

## Ordre de commit

Chaque commit compile et passe ses tests. Après chacun, le pilote mesure avec

```
git diff --shortstat origin/main...HEAD -- . ':!docs/plans'
```

1. **`feat(1727): le client des runs d'équipe de spirit (vol V4-b1)`** — `team_runs_remote.rs`,
   sa déclaration dans `mod.rs`, `drain_events` en `pub(crate)`, l'entrée d'egress, et les
   tests 1 à 6. L'egress est dans **ce** commit : le lint bidirectionnel l'exige (R2).
2. **`feat(1727): mika ask --team passe par spirit (vol V4-b1)`** — `run_team_ask` branché,
   le test 7 et la garde textuelle (test 8).
3. **`docs(cli): mika ask --team est un client de spirit (vol V4-b1)`** — le paragraphe de
   `crates/mika-cli/CLAUDE.md`.

## Verification Contract

### Les huit tests, contre `fake_spirit`, sans fournisseur réel

Un helper de test unique compose le corps SSE (`flux(&["…", "…"]) -> String`, trames
jointes par `\n\n`) et un second lance la porte contre le factice en collectant les
événements reçus. Les six cas du client sont alors courts, et c'est ce partage qui tient
le chiffrage du banc.

| # | ce qui est mesuré | comment |
|---|---|---|
| 1 | la requête vise `/api/v1/teams/{name}/runs` en `POST`, porte `Bearer <jeton>`, et son corps porte le but et le run de référence | `RequeteRecue` relevée par le factice — la socket mesure, pas une relecture du code |
| 2 | trois `team_event` puis `run_completed` : le rappel les reçoit **dans l'ordre**, et le `TeamRun` rendu est celui de la trame | le rappel pousse dans un `Vec`, comparé en séquence |
| 3 | `run_error` rend l'erreur du run | `matches!` sur `RunError` + le texte |
| 4 | un flux fermé **sans trame finale** rend l'échec distinct, et non un succès | corps SSE à deux `team_event` et rien d'autre ; `matches!(…, SansTrameFinale { .. })` |
| 5 | une trame indécodable **au milieu** n'arrête pas la lecture : la finale est lue | `data: {"event":"inconnu"}` entre deux trames valides ; le `Ok` final l'atteste |
| 6 | 401, 404 et une erreur de transport rendent chacun leur refus ; le 401 nomme `MIKA_INTERNAL_TOKEN` | trois cas ; l'erreur de transport par `DeadEndpoint::reserve()` (mika#2569) — **jamais** un `bind(:0)` relâché, et jamais une adresse non routable, que le proxy du bac à sable traduirait en statut HTTP |
| 7 | `run_team_ask`, au format JSON, rend pour un `run_completed` la **même forme** `AskTeamJsonResponse` qu'aujourd'hui | `enveloppe_equipe` sur le `TeamRun` décodé d'une trame, sérialisée et comparée au littéral re-tapé de la sortie d'avant — non à une re-sérialisation de la même structure, qui ne prouverait rien (motif `mika1940_team_envelope_is_byte_identical_on_a_successful_run`) |
| 8 | `ask.rs` ne contient plus `teams::run_team(` | R5 |

Les tests vont **en fin de fichier** : le scanner d'egress coupe la production au premier
`#[cfg(test)]`.

### Le test rougit sans le client — trois mutations

Dans un **worktree détaché**, sur un commit vert, chacune **seule** :

| mutation | test attendu rouge |
|---|---|
| un flux fermé sans finale rend `Ok` au lieu de l'échec | test 4 |
| l'en-tête `Authorization` est retiré de la requête | test 1 (et le 401 du test 6) |
| le rappel n'est pas appelé pour les trames `team_event` | test 2 |

La PR donne la sortie de chacune.

### Commandes

```
cargo build -p mika-cli --bin mika
cargo test -p mika-cli
cargo clippy --all-targets -p mika-cli
cargo test -p mika-common --test claude_md_size
bash scripts/verify-egress-manifest.sh --report
bash scripts/verify-pipeline.sh origin/main
```

`build` **avant** `test` (critère 12). `verify-pipeline.sh` avant d'ouvrir la PR ; rouge,
le pilote s'arrête et le dit.

## Definition of Done

- Les trois commits sont poussés sur `feat/1727/v4b1-mika-ask-team-passe-par-spirit`.
- Les six commandes ci-dessus sont vertes.
- Les trois mutations sont faites et leur sortie est dans le corps de la PR.
- `git diff origin/main...HEAD` ne touche **que** les fichiers listés au périmètre.
- La PR porte `Refs mika#1727` et `Tracked in: senara-solutions/mika#1727`, et aucun
  `Closes` / `Fixes` / `Resolves` — y compris si le gabarit de reprise (`wip-rescue`)
  l'ouvre, auquel cas `gh pr view <N> --json closingIssuesReferences` doit rendre `[]`.
- Aucune attente (`#[ignore]`) ni aucun `allow` neuf.
- Aucun ticket ouvert (gel jusqu'au 01/12) : un manque ou un reste va en commentaire
  sur mika#1727 ou dans la PR.

## Acceptance criteria

Transcrits du corps de mika#1727, § *Critères de V4-b1*.

1. Le client fait le `POST /api/v1/teams/{name}/runs` avec le jeton interne en `Bearer`,
   le but et le run de référence dans le corps, **sans délai global**, et lit le statut
   avant le corps.
2. Il passe chaque `TeamEvent` au rappel **dans l'ordre reçu**, et rend le `TeamRun` de
   `run_completed` ou l'erreur de `run_error`.
3. Un flux fermé sans trame finale est un échec **distinct**, jamais un succès. Une trame
   indécodable se dit et n'arrête pas la lecture.
4. 400, 401, 404, tout autre statut non 200 et une erreur de transport rendent chacun un
   refus qui **nomme l'URL** ; le 401 rappelle `MIKA_INTERNAL_TOKEN`.
5. `run_team_ask` n'appelle plus `teams::run_team`, et une garde textuelle le tient. Il
   n'a **aucun repli local**.
6. La vérification du but et de `--run-id` est inchangée. Les lignes stderr des
   événements, la sortie des trois formats et les codes de sortie sont ceux d'aujourd'hui
   pour un même `TeamRun`. **Tout échec côté client laisse stdout vide.**
7. L'entrée `cli-spirit-team-runs` est dans `docs/egress/egress-manifest.toml`, **dans le
   même commit que le client**, et `bash scripts/verify-egress-manifest.sh --report` est vert.
8. Les trois mutations du périmètre rougissent chacune un test. La PR donne la sortie de
   chaque mutation, faite **seule** dans un worktree détaché.
9. `crates/mika-cli/CLAUDE.md` porte le paragraphe, et
   `cargo test -p mika-common --test claude_md_size` est vert.
10. `crates/mika-agent/`, `chat.rs`, `tui/` et `team_outcome.rs` ne changent pas.
    `git diff origin/main...HEAD` ne touche que les fichiers listés au périmètre.
11. Aucune attente ni aucun `allow` neuf.
12. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-cli` est vert,
    et `cargo clippy --all-targets -p mika-cli` aussi. Le pilote lance
    `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il le trouve
    rouge, il s'arrête et le dit.
13. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si
    le gabarit de reprise l'ouvre.

## Fire-Disposition

Ce vol livre **un** détecteur : la garde textuelle du test 8 (R5), dont le chemin de
succès est « aucune violation trouvée ».

**Option (a) — exception nommée en allowlist, livrée avec une allowlist vide.**

- Le commit qui **arme** la garde est celui qui **retire** la seule violation existante
  (`ask.rs:852`, l'appel à `teams::run_team`). Il n'y a donc rien à excepter : la garde
  atterrit armée, verte, et **sans allowlist du tout**.
- Précédent exact dans le même crate : `chat_turn.rs:1176` et son assertion compagne
  `:1215` — « `V1_ALLOWED` est livrée vide, et le reste ». Ici l'allowlist n'existe même
  pas comme constante, ce qui est l'état auto-nettoyant maximal : il n'y a pas d'entrée
  à devenir stale, et pas de liste à élargir.
- Le doc du test nomme le remède — router le nouvel appelant par
  `team_runs_remote::lancer_run_equipe` — et dit qu'ajouter une exception n'est pas le
  remède. C'est ce texte qui tient la discipline, puisqu'il n'y a pas de liste.
- Ni (b) ni (c) : rien à désarmer (le détecteur est vert à la livraison), et rien à
  remonter (aucune violation ne subsiste).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-cli/src/commands/team_runs_remote.rs` — production (en-tête, refus typé + `Display`, URL, porte) | 150 |
| `crates/mika-cli/src/commands/team_runs_remote.rs` — tests 1 à 6 (deux helpers partagés + six cas) | 150 |
| `crates/mika-cli/src/commands/ask.rs` — `run_team_ask` branché, `enveloppe_equipe` extraite | 50 |
| `crates/mika-cli/src/commands/ask.rs` — tests (cas 7) et garde textuelle (cas 8) | 60 |
| `crates/mika-cli/src/commands/mod.rs` + `tasks.rs` (`drain_events` en `pub(crate)` et sa raison) | 8 |

Total estimé : 418 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 418 passe sous la borne par vol (429) et
sous la borne du groom posée par Vincent pour ce vol (**520 lignes ajoutées, doc
comprise** — cette borne est citée ici comme la tête du ticket le demande ; avec les
12 lignes de l'entrée d'egress et les ~16 du paragraphe de `CLAUDE.md`, le total dans
cette seconde unité est d'environ **446**). La plus serrée des deux est 429, dans l'unité
hors `docs/` — c'est elle qui est mesurée par la ligne `Total estimé :` ci-dessus.

Le chiffre tient parce que les **trois réutilisations** du § *Contexte* retirent
l'essentiel de ce qu'un client SSE coûte d'ordinaire : pas de miroir de types (`TeamRunFrame`,
`TeamEvent` et `TeamRun` voyagent tels quels, ~25 lignes évitées), pas de second
découpeur SSE (`drain_events` partagé, ~30 évitées), pas de banc de test neuf
(`fake_spirit`, ~40 évitées), et les trames JSON des tests s'écrivent à la main plutôt
qu'en construisant des `TeamRun` Rust, puisque tous leurs champs sauf trois sont en
`#[serde(default)]` (~40 évitées).

Le banc de test est chiffré **à part** (150 + 60 = 210), sur la mesure que la tête du
ticket donne : les vols clients de #1727 ont pesé 279 à 447 lignes tests compris
(V2b-2b-1, V2d-3a, V2c-1a-1, V3c-2a-ii). 418 est dans cette fourchette, vers son milieu.

À titre indicatif, MPC estime sur le ticket : client 140–180, ses tests 170–220,
egress 12, `run_team_ask` 30–50, ses tests et la garde 70–100, doc 10–20, total 450–550
(doc comprise). Le chiffrage ci-dessus est dans ces fourchettes, au bas de celle du
banc de test, et la raison en est les quatre réutilisations nommées juste au-dessus.
Ces chiffres de MPC ne lient pas le plan.

**Halte du pilote (posée par Vincent, le plan ne la change pas).** Après chaque commit,
le pilote mesure par `git diff --shortstat origin/main...HEAD -- . ':!docs/plans'` et lit
le nombre devant `insertions(+)`. **Si le compte dépasse 730, il s'arrête** : pas de
commit suivant, pas de push, pas de PR. Son dernier message dit « HALTE borne : N lignes
ajoutées pour `730` », avec le compte après chaque commit. Si la forme de la commande est
refusée malgré tout, le pilote s'arrête et le dit — il ne cherche pas une autre forme.

## Risques et notes de dérive

- **Un `kind` de `TeamEvent` inconnu fait échouer le décodage de la trame entière**, pas
  seulement du `payload` : `types.rs:295` refuse délibérément `#[serde(other)]`, et la
  note qui y est écrite renvoie à « le vol avec un consommateur en main ». Ce vol est ce
  vol, et il décide : un `kind` inconnu tombe dans le chemin « trame indécodable » — dit
  sur stderr, lecture poursuivie. Aucune retouche de `crates/mika-agent/` pour autant.
- **Le `mpsc` de 256 de la route perd des trames en `try_send` sans marqueur**
  (`crates/mika-agent/docs/architecture.md` § 14.2). Le client ne peut pas le détecter
  et n'essaie pas : la doc de la porte le dit, pour qu'une perte d'événement ne se lise
  pas plus tard comme un défaut du client.
- **Le run survit au client.** Un échec côté client ne tue pas le run côté spirit. C'est
  pourquoi le paragraphe de `CLAUDE.md` doit dire qu'un flux fermé sans finale **ne
  désigne pas le run à relire** (défaut 33) : l'opérateur n'a pas d'identifiant en main
  dans ce cas, et le croire disponible serait la mauvaise lecture.
- **Si un critère ne tient pas sans toucher un fichier exclu**, le pilote s'arrête et le
  dit, au lieu d'élargir le périmètre.
- **Si le plan doit s'écarter de la tête du ticket** sur un fichier, un critère ou un nom,
  le pilote amende ce plan sur la branche dans un commit `docs(plans)` **avant** de
  pousser le code qui s'en écarte, en nommant le point et la raison.
- **Une halte ne se lève jamais par un amendement du plan.** Si une halte se déclenche,
  le pilote s'arrête et le dit, même s'il juge qu'elle ne vaut pas ici.
- Si un hook ou un classifieur refuse un geste, le pilote s'arrête et le déclare, même si
  cela lui semble être un faux positif, et ne le contourne jamais par un autre outil
  (incident mika#2869).
