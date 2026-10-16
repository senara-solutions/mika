# fix(test,auto_pull) — un test de la suite `auto_pull` écrit sur la forge réelle (mika#2713)

## Contexte

Le 2026-10-03 puis le 2026-10-14, le ticket **fermé** mika#2636 a reçu
**29 commentaires** « Auto-pull : promotion refusée pour #2636 », sous l'identité
du moteur, plus le label `operator-gated` posé le 03/10 à 03:16:42Z. Chacun
nomme la branche `feat/123/x`, qui n'existe nulle part sur `origin` — elle
n'existe que dans `crates/mika-agent/src/auto_pull.rs:6379`, la fixture
`GROOMED_BODY`.

### La chaîne, établie site par site

Le producteur est le test `mika2634_une_fenetre_propre_ne_freine_pas_le_tick`
(`auto_pull.rs:9875`). Il pose `let n = 2636u64;`, construit
`make_issue(n, GROOMED_BODY, &["ready"], "t")`, puis appelle
`phase2_reconcile_stuck_ready(…, "fake-token", …)`. De là :

1. le ticket survit aux filtres d'exclusion ;
2. `read_ready_label_timeline(github_token, DEFAULT_REPO, n)` (`auto_pull.rs:4514`)
   lit la timeline du **vrai** #2636 ;
3. `promotion_gate_allows` (`auto_pull.rs:2683`) extrait `feat/123/x` du corps de
   la fixture et appelle `gh_compare_branch` (`auto_pull.rs:2631`) ;
4. GitHub répond **404** — la branche n'existe pas — ce qui donne
   `StalenessMeasurement::BranchAbsent`, le seul cas où la porte refuse ;
5. `refuse_promotion` (`auto_pull.rs:2776`) applique `REFUSAL_LABEL`, retire
   `ready`, puis appelle `gh_comment_issue` (`auto_pull.rs:2588`).

Le commentaire publié et le label posé sont exactement les deux écritures de
l'étape 5. Le périmètre est un test unitaire, pas le moteur : `server.log` ne
porte aucune écriture `auto_pull` sur #2636 aux instants des commentaires.

### La « piste non prouvée » du ticket est maintenant PROUVÉE, par le code

Le ticket demande de confirmer en premier l'hypothèse du proxy, « car la
correction en dépend ». Elle est confirmée, et par lecture de code plutôt que
par sonde — preuve plus forte que l'empirique.
`scripts/mika-pilot-github-auth-addon.py` est l'addon mitmproxy de l'egress de
la sandbox. Son en-tête dit « The sandbox NEVER holds the GitHub token », et son
corps fait deux choses :

- il **retire** tout `Authorization` / `Proxy-Authorization` venu du client
  (« the sandbox has none ») ;
- il **injecte** le jeton lu côté **hôte** (`~/.mika/pilot-gh-token`, 0600, jamais
  bind-monté ; repli `GH_TOKEN` / `MIKA_GITHUB_TOKEN`) :
  `flow.request.headers["Authorization"] = _auth_header_for(host, token)`
  (ligne 145), `Bearer <tok>` pour `api.github.com`, `Basic
  base64("x-access-token:<tok>")` pour `github.com`.

**Conséquence :** le `GH_TOKEN="fake-token"` que `gh_comment_issue` pose sur
l'enfant est sans effet. Il est effacé par le proxy et remplacé par le jeton du
moteur. Tout `cargo test` lancé depuis une sandbox de pilote écrit sur la forge
réelle, sous l'identité du moteur, **quel que soit le jeton du test**. La
réponse à la question du ticket est donc : la correction est **côté test**
(AC1–AC3), et la piste côté proxy est écartée pour la raison donnée au § *Hors
périmètre* — elle n'est pas implémentable telle qu'énoncée.

### Pourquoi « un seul test » est trompeur

La fuite n'est pas propre à #2636. Mesuré sur la forge le 2026-10-15 :
#2636 → 52 commentaires ; #2360 → 2 commentaires auto-pull ; #2635 → **0**. Le
zéro s'explique : le vrai #2635 ne porte pas le label `ready`, donc la lecture
de timeline rend `Ok(None)`, l'exclusion `FILTER_NO_READY_LABEL_EVENT` tombe, et
la porte de promotion n'est jamais atteinte. **Qu'une fixture fuie ou non dépend
de l'état réel du ticket réel homonyme** — c'est-à-dire d'une donnée que
personne ne contrôle. C'est précisément la fragilité que ferme AC3.

Douze fixtures de la suite atteignent l'I/O, sur les numéros
2049, 2276, 2360 (×3), 2470 (×4), 2634, 2635, 2636. Toutes sont des numéros de
vrais tickets.

## Décisions de grooming

**D1 — La correction est une couture unique, pas six.** Les six sites `gh` de
`auto_pull.rs` (2383, 2448, 2498, 2540, 2588, 2631) et les deux de
`ready_label.rs` (378, 504) portent un boilerplate **identique** :
`Command::new("gh")`, `args`, `env("GH_TOKEN", …)`, `stdin(null)`,
`stdout(piped)`, `stderr(piped)`, `kill_on_drop(true)`, `output().await`. On les
réduit à un seul point de passage, `gh_process::run_gh`. Chaque site converti
**perd** environ six lignes : la couture se paie en partie elle-même.

**D2 — Le double de test n'échoue pas, il réussit à vide.** Rejeté : rediriger
vers un binaire inexistant pour obtenir un ENOENT garanti. Deux raisons, toutes
deux mesurées dans le code :

- `gh_compare_branch` ne mappe en `BranchAbsent` que le **404** ; *tout autre*
  échec devient `Unavailable`, et `Unavailable` **promeut** (fail-open). Un
  ENOENT ne serait donc pas neutre : il changerait le verdict de la porte.
- `enact_stuck_ready_skip` (`auto_pull.rs:3190`) n'écrit la ligne d'audit
  `stuck_ready_reentry_blocked` **que si le commentaire a été posté** (`return`
  anticipé sur `Err`). Le test
  `mika2361_the_reentry_comment_is_posted_once_per_abandonment`
  (`auto_pull.rs:8017`) dépend donc aujourd'hui du succès de l'appel `gh`. Une
  couture qui fait échouer `gh` le rendrait rouge.

Le double rend donc `exit 0` avec `stdout = "[]"` : la seule constante qui fasse
rendre « rien » à chaque site qui lit du JSON et « succès » à chaque site qui
écrit. Vérifié contre les deux tests ci-dessus :
`mika2634_une_fenetre_propre_ne_freine_pas_le_tick` n'assert que l'absence de
`FILTER_LAUNCHER_BRAKED`, décidée en amont de la porte, donc verte sur `[]` ;
le test n=2360 reste vert parce que le commentaire « réussit ».

**D3 — AC3 se tient à la couture, pas dans les 109 fixtures.** Renuméroter les
125 appels `make_issue(` (dont 109 à littéral) coûte ~185 lignes d'édition et de
réparation d'assertions, ce qui ferait franchir la borne par vol. Mais AC3 n'a
pas besoin de ça : un numéro de fixture ne nuit **que** s'il atteint `gh`. On
donne donc à `run_gh` un sujet typé (`GhSubject::Issue(n) | Pr(n) | Repo`) et le
double **panique** sur `Issue(n)` avec `n < FIXTURE_ISSUE_FLOOR` (990 000), en
nommant le numéro. La propriété est posée au seul endroit par lequel un numéro
peut sortir, elle est auto-armée, et elle couvre les fixtures futures sans qu'on
y pense. Les douze fixtures qui atteignent l'I/O sont renumérotées en
990 101…990 112 ; les autres gardent leur numéro, inoffensif, et si un
changement futur leur fait atteindre `gh`, le double rougit en les nommant. La
bande `990_00x` a déjà un précédent dans le fichier (`auto_pull.rs:8266`,
« Issue numbers well outside the real range »).

**D4 — AC2 et AC3 se renforcent.** Des numéros **distincts** par test sont ce
qui permet à un test de filtrer le journal d'appels process-global sur ses
propres appels sous exécution parallèle. La renumérotation d'AC3 est donc une
précondition des contrôles comportementaux d'AC2, pas un travail parallèle.

**D5 — Un seul vol.** 412 lignes estimées, sous la borne de 431. Le périmètre
tient parce que D1 réduit le coût des conversions et D3 réduit AC3 de ~185
lignes à ~45. Pas de découpage, donc pas de ligne `Tracked in:` au titre de
mika#2693.

**D6 — Les cinq autres sites `gh` du crate restent hors périmètre, en
allowlist nommée.** Le crate compte 13 sites `Command::new("gh")` ; huit sont
dans le périmètre du ticket (« la suite `auto_pull` »), cinq non :
`skills/builtin_handlers.rs:1881` (`gh_read`), `:2967`, `:5336`,
`tools/pr_merge_with_gate.rs:2214` (`run_gh_subprocess`),
`milestone_manager/reader.rs:87`. Les deux derniers ont **déjà** leur point de
passage unique — `run_gh_subprocess` pour le premier, le trait `GhRunner`
(`reader.rs:32`) pour le second, avec un double de test à `reader.rs:1058`.
Aucun n'est atteint par la suite `auto_pull`. Voir `## Fire-Disposition`.

## Conception

**L1 — `crates/mika-agent/src/gh_process.rs` (nouveau), le seul site
`Command::new("gh")` du périmètre.**

```rust
/// À quoi l'appel `gh` s'adresse. Le sujet n'est pas décoratif : sous test,
/// c'est lui qui permet au double de refuser un numéro d'issue réel (mika#2713).
pub(crate) enum GhSubject {
    Issue(u64),
    Pr(u64),
    Repo,
}

pub(crate) async fn run_gh(
    token: &str,
    subject: GhSubject,
    args: &[&str],
) -> std::io::Result<std::process::Output>
```

En production : le boilerplate commun, une fois (`env("GH_TOKEN", token)`,
`stdin(null)`, `stdout(piped)`, `stderr(piped)`, `kill_on_drop(true)`). Sous
`#[cfg(test)]`, `run_gh` **ne spawne jamais** ; elle :

1. refuse `GhSubject::Issue(n)` avec `n < FIXTURE_ISSUE_FLOOR` par un `panic!`
   qui nomme `n` et les args — AC3 ;
2. pousse `(subject, args.join(" "))` dans un journal process-global
   (`Mutex<Vec<GhCall>>`) — le substrat d'AC2 ;
3. rend `exit 0` / `stdout = "[]"` — D2 ;
4. accepte un override par test (`set_canned_stdout`), nécessaire aux contrôles
   comportementaux d'AC2 et à tout test futur qui aurait besoin d'une timeline
   fabriquée.

Le journal est lisible par `recorded_calls()` ; un test filtre sur **son** numéro
de fixture, ce qui le rend indifférent à l'exécution parallèle (D4).

**L2 — Conversion des huit sites.** Dans `auto_pull.rs` : `gh_list_open_issues`
(`Repo`), `gh_list_open_pr_closing_issues` (`Repo`), `gh_apply_label`
(`Issue(n)`, jeton `label_auth.token()`), `gh_remove_label` (`Issue(n)`),
`gh_comment_issue` (`Issue(n)`), `gh_compare_branch` (`Repo`). Dans
`ready_label.rs` : `read_ready_label_timeline` (`Issue(n)`), `gh_current_login`
(`Repo`). Le mapping des erreurs reste **inchangé** à chaque site : la couture
rend le même `io::Result<Output>` que `output().await`, donc le traitement du 404
de `gh_compare_branch` et le fail-open sur `Unavailable` ne bougent pas. C'est
la condition pour que la conversion soit neutre en comportement (D2).

Le plancher d'AC3 mord donc sur exactement quatre sites : les trois écritures de
`refuse_promotion` et la lecture de timeline — les quatre qui ont fui.

**L3 — Le scan structurel (AC2).** Dans le `mod tests` de `gh_process.rs`, sur
l'idiome `mika_common::source_guard::ProductionScanner` :
`for_crate(env!("CARGO_MANIFEST_DIR"))`, `src_root()`, `for_each(|path,
production|)`, exclusion des chemins de test par
`crate::source_scan::is_test_source_path(path)`. L'aiguille est épelée par
concaténation (`["Command::new(\"", "gh\")"].concat()`) pour que le scan ne se
trouve pas lui-même, comme
`mika2361_reset_auto_pull_redrive_has_exactly_two_production_callers`
(`auto_pull.rs:8195`). Chaque contrevenant est collecté en `path:line: text`.
L'assertion est `offenders ⊆ ALLOWLIST ∪ {gh_process.rs}`, précédée d'une
assertion de **non-vacuité** (le scan a vu au moins un site, donc il scanne
vraiment).

**L4 — Le contrôle de bonne foi (AC2).** Un second test applique la même
fonction de scan à un contenu **planté** en mémoire (une ligne portant
l'aiguille, dans un chemin de production fictif) et assert qu'elle le signale.
Sans lui, « aucun contrevenant » et « le scan ne regarde rien » sont
indistinguables — c'est l'idiome mika#2425.

**L5 — Les contrôles comportementaux (AC2).** Trois tests, dans le `mod tests`
de `gh_process.rs` :

- un appel `run_gh` apparaît bien dans `recorded_calls()`, avec son sujet et ses
  args (le journal n'est pas un décor) ;
- `run_gh(…, GhSubject::Issue(2636), …)` **panique**, et le message nomme 2636
  (le plancher d'AC3 rougit sur le numéro exact qui a fui) ;
- sous le double, zéro processus n'est spawné : `run_gh` rend son `Output` canné
  même avec un `PATH` vidé de `gh`, ce qui est la forme « `PATH` sans `gh` »
  suggérée par AC2, vérifiée sans dépendre de l'environnement de la machine.

**L6 — Renumérotation des douze fixtures qui atteignent l'I/O.** `const
FIXTURE_ISSUE_FLOOR: u64 = 990_000;` plus un helper `assert_fixture_issue(n)`
appelé par les tests concernés. Les numéros 2049, 2276, 2360, 2470, 2634, 2635,
2636 deviennent 990 101…990 112, **distincts** (D4). Les assertions qui citent
l'ancien numéro en littéral sont réparées dans le même mouvement ; les noms de
tests, qui portent le numéro de **ticket** et non de fixture, ne changent pas.

### Ce que ce plan NE touche pas

- Le comportement de production : aucun verdict, aucun mapping d'erreur, aucun
  ordre d'écriture ne change. `refuse_promotion` applique toujours le label
  d'abord et vérifie son issue, puis retire `ready`, puis commente.
- `scripts/mika-pilot-github-auth-addon.py` et l'egress de la sandbox.
- Les cinq sites `gh` hors périmètre (D6).
- Les 109 fixtures `make_issue(` à littéral qui n'atteignent pas l'I/O : D3 les
  rend inoffensives sans les éditer.
- `Cargo.toml` : rien à ajouter. `#[cfg(test)]` couvre le harnais unitaire de la
  lib, qui est là où vit la fuite.

## Étapes d'implémentation

0. **Ouvrir le ticket de suivi** « généraliser `run_gh` aux cinq sites `gh`
   restants du crate » et noter son numéro : les cinq entrées d'allowlist de L3
   le citent (mika#1574 (a) exige qu'une exception référence un suivi), et le
   corps de la PR le nomme par une ligne `Tracked in:`.
1. Créer `crates/mika-agent/src/gh_process.rs` (L1) ; déclarer
   `pub(crate) mod gh_process;` dans `lib.rs`, auprès de
   `pub(crate) mod source_scan;` (`lib.rs:52`). `cargo build`.
2. Convertir les six sites de `auto_pull.rs` (L2). `cargo test -p mika-agent
   auto_pull` — la suite doit être verte **avant** toute renumérotation ; c'est
   la preuve que la couture est neutre.
3. Convertir les deux sites de `ready_label.rs` (L2). `cargo test -p mika-agent
   ready_label`, dont la garde existante
   `no_ready_label_write_outside_this_module`.
4. Renuméroter les douze fixtures et réparer les assertions (L6), **sans
   plancher actif** d'abord, puis activer `assert_fixture_issue` : un test resté
   sur un vrai numéro se nomme de lui-même.
5. Ajouter le scan structurel et son allowlist (L3), puis le contrôle de bonne
   foi (L4), puis les trois contrôles comportementaux (L5).
6. `cargo test`, `cargo clippy`, `cargo fmt`, les lints du dépôt, et
   `scripts/verify-pipeline.sh origin/main`.

Ordre de commit : (1) couture, (2) conversions, (3) renumérotation, (4)
détecteurs. Plancher mergeable : les étapes 1–3, qui ferment AC1 à elles seules.

## Contrat de vérification

**W1 — AC1, par construction.** Après l'étape 2, plus aucun chemin de
`phase2_reconcile_stuck_ready` ne peut atteindre `gh` depuis un test : le seul
`Command::new("gh")` du périmètre est dans `gh_process.rs`, et il est remplacé
sous `#[cfg(test)]`. Le jeton passé n'a plus besoin d'être « celui utilisé » —
l'alternative que l'AC autorise explicitement, la couture injectable, est celle
retenue, parce que le proxy rend la première branche inatteignable (§ Contexte).

**W2 — AC1, par mesure.** Un test rejoue la fixture historique exacte — corps
`GROOMED_BODY`, label `ready`, numéro renuméroté — appelle
`phase2_reconcile_stuck_ready`, puis assert sur `recorded_calls()` filtré sur
son numéro : les appels sont enregistrés, aucun n'est parti. C'est la
reproduction du bug, devenue garde.

**W3 — AC2, structurel.** `Command::new("gh")` n'apparaît hors
`gh_process.rs` qu'aux cinq entrées d'allowlist (L3), avec non-vacuité (L3) et
bonne foi (L4).

**W4 — AC2, comportemental.** Les trois contrôles de L5, dont celui qui vaut la
forme « `PATH` sans `gh` » de l'AC et celui qui vaut la forme « un stub qui
enregistre puis panique ».

**W5 — AC3.** `run_gh` panique sur `GhSubject::Issue(n < 990_000)`, message
nommant `n` ; les douze fixtures d'I/O sont dans la bande et distinctes.

**W6 — Non-régression.** `cargo test -p mika-agent` entier vert, en particulier
`mika2361_the_reentry_comment_is_posted_once_per_abandonment` (qui exige un
commentaire **réussi**, D2) et `mika2634_une_fenetre_propre_ne_freine_pas_le_tick`
(le test fuiteur, qui doit rester vert sans fuir).

### Les gardes existantes, inchangées et vertes

- `ready_label::tests::no_ready_label_write_outside_this_module` — la
  conversion garde les écritures de label dans leur module ; la couture est un
  détail d'exécution, pas un site d'écriture.
- `mika2361_reset_auto_pull_redrive_has_exactly_two_production_callers`
  (`auto_pull.rs:8195`) — dont ce plan reprend l'idiome.
- `canonical_tokens::tests::mika2569_aucune_fixture_nobtient_un_endpoint_mort_en_liberant_un_port`
  — aucun port n'est réservé ici ; le double ne spawne ni n'écoute.
- `mika-common`'s `claude_md_size` — aucun `CLAUDE.md` n'est touché.

## Risques nommés

**R1 — Un site qui lit du JSON non-liste.** Si un des huit sites attend un objet
JSON et non une liste, `"[]"` le fait échouer au parse au lieu de rendre
« rien ». Mitigation : l'étape 2 passe la suite au vert **avant** la
renumérotation, donc un tel site se signale immédiatement ; le remède est
l'override `set_canned_stdout` de L1, par test.

**R2 — Un test qui dépendait de l'état réel de la forge.** Symétrique de R1 :
un test aujourd'hui vert parce qu'un vrai ticket est dans un certain état
devient rouge sous le double. C'est un **gain** — il révèle une dépendance
illégitime — mais il coûte des lignes non prévues. Borne : la suite ne compte
que douze tests atteignant l'I/O, tous lus.

**R3 — `#[cfg(test)]` ne couvre pas les cibles d'intégration.** `mika-agent`
n'a pas de feature `test-utils` propre (`Cargo.toml:31`), donc un futur test
sous `tests/` verrait le vrai `run_gh`. Hors périmètre ici : la fuite constatée
est unitaire, et aucun test de `tests/` n'appelle ces fonctions
(`pub(crate)`, donc inatteignable depuis une cible d'intégration). Noté pour que
l'extension soit un choix et non une surprise.

**R4 — L'allowlist de cinq entrées devient stale.** Traité par l'assertion
auto-nettoyante du § Fire-Disposition.

## Hors périmètre

**La correction côté proxy, écartée pour une raison de fond.** Le ticket la
propose : « refuser une requête dont le jeton n'est pas un vrai jeton de la
sandbox ». Elle n'est **pas implémentable telle qu'énoncée** : par conception,
la sandbox ne détient **aucun** jeton — l'addon retire l'`Authorization` du
client précisément parce qu'il n'y en a pas (§ Contexte). Un `gh pr create`
légitime d'un pilote n'en porte donc pas davantage qu'un `cargo test`. La
distinguer exigerait de **doter** chaque sandbox d'une sentinelle que le proxy
réclame — c'est-à-dire de changer le substrat d'egress de tous les pilotes, avec
son propre mode de défaillance : un bug d'injection ferait silencieusement 403
tous les appels légitimes, et la boucle s'arrêterait sans diagnostic. La
correction côté test ferme la classe (AC1 par construction, AC2 et AC3 en
défense en profondeur) sans ce risque. Le durcissement proxy est un travail
d'infrastructure distinct, à ouvrir en suivi et à nommer par `Tracked in:` dans
le corps de la PR.

**La généralisation de `run_gh` aux cinq sites restants** (D6), suivi ouvert à
l'étape 0 et cité par l'allowlist.

**Le nettoyage de la forge** — les 29 commentaires et le label `operator-gated`
sur #2636 fermé. C'est un geste opérateur sur des données, pas du code ; ce plan
n'y touche pas.

## Definition of Done

- [ ] `crates/mika-agent/src/gh_process.rs` existe et est le seul
      `Command::new("gh")` du périmètre, déclaré dans `lib.rs`.
- [ ] Les huit sites `gh` de `auto_pull.rs` (6) et `ready_label.rs` (2) passent
      par `run_gh`, avec leur mapping d'erreur inchangé.
- [ ] Sous `#[cfg(test)]`, `run_gh` ne spawne aucun processus, enregistre l'appel
      et rend `exit 0` / `"[]"`, avec override par test.
- [ ] Les douze fixtures atteignant l'I/O portent des numéros distincts
      ≥ 990 000 ; `assert_fixture_issue` est actif.
- [ ] Le scan structurel, son contrôle de non-vacuité et son contrôle de bonne
      foi sont verts ; l'allowlist a cinq entrées nommées citant le suivi.
- [ ] Les trois contrôles comportementaux de L5 sont verts.
- [ ] W2 reproduit la fixture historique et assert l'absence de fuite.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`, les lints du dépôt et
      `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Le corps de PR porte `Code review: delegated to mika-qa (mika#2694)` et
      les lignes `Tracked in:` des deux suivis nommés au § Hors périmètre.

## Acceptance criteria

Transcrits verbatim du corps de mika#2713 :

- [ ] **AC1.** Le chemin d'écriture (commentaire et label) de
      `phase2_reconcile_stuck_ready` ne peut pas atteindre `gh` réel depuis un
      test. Le jeton passé (`"fake-token"`) doit être celui utilisé, ou bien
      l'écriture doit passer par une couture injectable.
- [ ] **AC2.** Un test de contrôle échoue (vu rouge) si un appel `gh` réel part
      pendant la suite `auto_pull`. Par exemple : `PATH` sans `gh`, ou un stub
      qui enregistre puis panique.
- [ ] **AC3.** Les numéros d'issue des fixtures `auto_pull` ne désignent pas de
      vrais tickets du dépôt. Un numéro hors plage, comme 999999, ferme la
      classe même si une fuite revenait.

Couverture : AC1 par W1 et W2 ; AC2 par W3 et W4 ; AC3 par W5. Le ticket demande
en outre de confirmer d'abord l'hypothèse du proxy, « car la correction en
dépend » : c'est fait au § Contexte, par lecture de
`scripts/mika-pilot-github-auth-addon.py`, et la conclusion est « côté test ».

## Fire-Disposition

Ce plan livre trois détecteurs : le scan structurel (L3), le contrôle de bonne
foi (L4) et le plancher de numéro de fixture (L1 point 1 / L6).

**Option (a) — exception nommée en allowlist — pour le scan structurel (L3).**
Cinq sites `gh` existent hors du périmètre du ticket et ne sont pas convertis
par ce vol. Chacun reçoit une entrée grep-visible qui nomme le fichier, la
fonction porteuse et la raison, et cite le suivi ouvert à l'étape 0 :

```rust
/// Sites `gh` hors du périmètre de mika#2713 (« la suite auto_pull »).
/// Chaque entrée nomme la fonction porteuse et la raison. Généralisation
/// suivie par <suivi de l'étape 0>.
const GH_SPAWN_ALLOWLIST: &[(&str, &str)] = &[
    ("skills/builtin_handlers.rs", "gh_read"),
    ("skills/builtin_handlers.rs", "run_gh_pr_view_author_title"),
    ("skills/builtin_handlers.rs", "run_gh"),
    ("tools/pr_merge_with_gate.rs", "run_gh_subprocess"),
    ("milestone_manager/reader.rs", "GhRunner::run"),
];
```

**Assertion auto-nettoyante.** Le test assert l'**égalité** des ensembles, pas
l'inclusion : `offenders.len() == GH_SPAWN_ALLOWLIST.len()`, et chaque entrée
doit avoir été vue. Donc (i) un **nouveau** site non listé rougit, et (ii) une
entrée dont le site a été converti ou supprimé rougit aussi, avec le message
« entrée d'allowlist stale : retire-la ». L'exception ne peut pas survivre à sa
cause. Le chemin de succès reste « aucun site hors allowlist ».

**Disposition si le scan rougit sur un nouveau site :** convertir le site vers
`run_gh`. Ajouter une entrée d'allowlist est réservé aux sites qu'une mesure
montre hors d'atteinte de toute suite de tests — et cette mesure va dans le
corps de PR, pas dans un commentaire.

**Le plancher de numéro de fixture (L1/L6) atterrit ARMÉ**, et c'est le point :
sa valeur est d'être armé pour toute fixture future, pas seulement pour les
douze d'aujourd'hui. Aucune allowlist, aucune exception. **Disposition s'il
rougit : renuméroter la fixture dans la bande 990 000+.** Il ne rougit que si un
test tente d'adresser un vrai ticket — ce qui est exactement le bug de mika#2713.
Son vide est l'invariant, comme pour la garde `reset_auto_pull_redrive`
(`auto_pull.rs:8195`), et l'étape 4 l'active après la renumérotation pour que
sa première exécution soit verte.

**Le contrôle de bonne foi (L4)** est son propre détecteur inversé : son chemin
de succès est « le scan a bien signalé le site planté ». S'il rougit, le scan ne
regarde plus rien et la disposition est **halte-et-remontée** : un scan aveugle
qui se présente comme vert est pire que pas de scan.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/gh_process.rs` (couture + sujet typé + double enregistreur) | 160 |
| `crates/mika-agent/src/lib.rs` (déclaration de module) | 2 |
| `crates/mika-agent/src/auto_pull.rs` (6 conversions + 12 renumérotations + réparations d'assertions) | 95 |
| `crates/mika-agent/src/ready_label.rs` (2 conversions) | 15 |
| tests AC2 (`gh_process.rs` : scan structurel + allowlist + bonne foi + 3 contrôles comportementaux) | 85 |
| tests AC1/AC3 (`auto_pull.rs` : W2 + plancher de fixture) | 55 |

Total estimé : 412 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

Un seul vol, et voici pourquoi il tient. Les deux postes qui menaçaient la borne
ont été réduits par mesure, pas par optimisme :

- **les conversions**, parce que le boilerplate des huit sites est identique
  (D1) : chaque site converti perd ~6 lignes, donc 8 sites coûtent ~85 lignes
  nettes au lieu de ~140 ;
- **AC3**, parce que le plancher se tient à la couture (D3) : ~45 lignes au lieu
  des ~185 qu'aurait coûté la renumérotation des 109 fixtures à littéral — qui,
  seule, aurait porté le total à ~552 et forcé un second vol.

Le reste du budget de tours va à la fin de pipeline, qui coûte des tours et non
des lignes (mika#2693).
