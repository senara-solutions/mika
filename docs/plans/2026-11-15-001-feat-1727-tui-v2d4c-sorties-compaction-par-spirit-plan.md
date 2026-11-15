# mika#1727 — vol V2d-4c : les trois sorties de la TUI passent par `clore`

- **Ticket :** senara-solutions/mika#1727
- **Branche :** `feat/1727/tui-v2d4c-sorties-compaction-par-spirit`
- **Base :** `origin/main` @ `e4597a7b` (merge de V2d-4b, #3089)
- **Label :** `loop-substrate` (plafond de tours 200)
- **PR :** `Refs mika#1727`. Jamais `Closes`, `Fixes` ni `Resolves`, ni dans le corps ni dans un commit.

---

## 0. La coupe de périmètre, et pourquoi elle est en tête de ce plan

**Ce plan groome une moitié du vol V2d-4c décrit par le ticket : les trois
sorties de `chat.rs` par `session_remote::clore`. `/compact` par `compacter`, le
retrait des deux `expect(dead_code)` et celui d'`App.llm` ne sont pas dans ce
vol.** La suite se note `Tracked in: senara-solutions/mika#1727`.

Ce n'est pas un arbitrage de goût, c'est une mesure. Le ticket borne ce vol à
**600 lignes ajoutées** dans l'unité `git diff --numstat origin/main...HEAD`
(doc comprise, plan exclu). La règle de dispatch mika#2636 / mika#2693 en pose
une seconde, **429 lignes estimées hors `docs/` et hors `CLAUDE.md`**, et c'est
la plus serrée qui mord. Le vol entier, estimé poste par poste au § *Taille
estimée* ci-dessous, pèse **454 lignes** dans la seconde unité — la moitié
« sorties » 200, la moitié « `/compact` + nettoyage » 254.

Deux faits rendent cette estimation crédible plutôt que prudente :

| vol | estimé au plan | **réel**, hors `CLAUDE.md` | réel, unité du ticket |
|---|---|---|---|
| V2d-4a (#3088) | — | 462 (`chat.rs` 447 + `session_remote.rs` 15) | 513 |
| V2d-4b (#3089) | 392 | **496** (`handlers.rs` 420 + `session_remote.rs` 67 + `chat.rs` 9) | 556 |

Mesuré par `git diff --numstat a64a961c 748ad33d` et
`git diff --numstat 748ad33d e4597a7b`. Le plan de V2d-4b annonçait 392 et le
vol en a coûté 496 : l'écart est de **+27 %**, et il est entièrement dans les
tests. Ce vol-ci est structurellement **plus cher** que V2d-4b à périmètre égal,
pour une raison qui est elle aussi une mesure : **`/compact` n'a aucun test
aujourd'hui** (`grep -n "handle_compact\|compact_par" handlers.rs` ne rend que le
dispatch l. 41 et la fonction l. 262), et les trois sorties de `chat.rs` vivent
dans `run()`, qui n'est pas testable unitairement. Là où V2d-4b rebranchait onze
tests existants, ce vol écrit **douze tests neufs** et n'en rebranche aucun.

Le ticket dit « *Si le total dépasse 600, le groom s'arrête et le dit. Il ne
recoupe pas lui-même.* » — cette clause est conditionnée à **600**, que le vol
entier ne dépasse pas (≈ 480 dans l'unité du ticket). Elle n'est donc pas
déclenchée, et elle n'arbitre pas la borne de 429, qui l'est. La règle qui mord
dit : *« Au-dessus de la borne, découpe en vols — pas en phases d'un même vol.
Chaque vol est mergeable seul, et le vol courant doit repasser sous la borne. »*
C'est ce que fait ce plan.

**La coupe suit une ligne que le code dessine déjà**, et c'est ce qui la rend
sûre plutôt qu'arbitraire :

- la moitié « sorties » touche **`commands/chat.rs` et `crates/mika-cli/CLAUDE.md`,
  et rien d'autre** — pas même `session_remote.rs`, puisque `clore` est câblée
  depuis V2d-4b et ne porte plus d'`expect` ;
- les deux `expect(dead_code)` restants sont posés sur `compacter` et
  `CompactedMirror` : ils ne peuvent tomber **que** dans le commit qui câble
  `compacter`. Les retirer sans câblage ferait rougir la CI en
  `dead_code` — `cargo clippy --all-targets --all-features -- -D warnings`
  (`.github/workflows/ci.yml:76`) ;
- `App.llm` n'a **qu'un lecteur**, `handlers.rs:273`, dans le corps de
  `handle_compact`. Le champ ne devient mort qu'à l'instant où `/compact` passe
  par `compacter` ; dans ce vol-ci, il reste lu et n'est pas touché.

Autrement dit : la moitié « `/compact` » est un bloc indivisible (câblage, les
deux `expect`, `App.llm`, la garde `maybe_compact(`), et la moitié « sorties »
en est **indépendante dans les deux sens**. Aucun livrable n'est coupé en deux.

Le nom de la branche, `…-sorties-compaction-par-spirit`, nomme encore les deux
moitiés : il est fixé par le callout du ticket et par le worktree, et ce plan ne
le change pas. La PR dira en une phrase que la compaction part à la suite.

**Ce que ce plan ne fait pas :** il ne rouvre aucun ticket (gel jusqu'au
01/12), ne redécoupe pas V3 ni V4, et ne touche à aucun fichier du plan de
contrôle.

---

## 1. Ce qui est mesuré sur `main` (@ `e4597a7b`)

Tout ce qui suit a été lu, pas supposé. Les absences sont dans la seconde table :
elles portent autant de décisions que les présences.

| fait | site | ce qu'il décide |
|---|---|---|
| Site 1 — changement d'agent : `if !app.is_singleton_session && let Err(e) = worker._ctx.async_db.end_session(&app.session_id).await { tracing::warn!(…) }` | `chat.rs:794-798` | l'un des trois sites à re-router |
| La réaffectation `app.agent_name = target_name.clone();` est **59 lignes plus loin** | `chat.rs:857` | c'est tout le critère 3 : voir D3 |
| Cette affectation est **unique dans le fichier** (`grep -n "app.agent_name = "` → une ligne) | `chat.rs:857` | aucune autre réaffectation à ordonner |
| Site 2 — `/restart`, même forme | `chat.rs:908-917` | deuxième site |
| Site 3 — fin de session, **après** `restore_terminal()?` et `events.shutdown()` | `chat.rs:1006-1017` | troisième site ; plus aucun écran pour rendre un message, d'où D2 |
| Le modèle d'appel aux portes : `ouvrir_session_par_spirit(db, base_url, lire_env, agent, …) -> Result<(String, bool)>`, avec `jeton_interne(lire_env).map_err(anyhow::Error::msg)?` | `chat.rs:50-104` | la forme et le type de retour de la fonction neuve (D1) |
| `session_remote::clore(base_url, token, agent, session_id) -> Result<EndedSessionMirror, SessionRemoteIssue>`, câblée depuis V2d-4b, **sans `expect`** | `session_remote.rs` | ce vol ne touche pas ce fichier |
| `clore` rend `{ended, canonical}` en **200** sur une session canonique, qui ne se clôt jamais (mika#1401) | `session_remote.rs` | D2 et le test T-F |
| `jeton_interne(lire: SourceEnv) -> Result<String, String>` — lecture unique du jeton depuis V2d-4b | `session_remote.rs` | aucune seconde lecture n'est écrite ici |
| Garde `mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local` : `ALLOWED` vide + `assert!(…is_empty())`, deux motifs, témoin `async fn spawn_agent_worker(`, un seul `assert!` final par `flat_map` | `chat.rs:1588` | la garde à étendre (D5) |
| Son doc-comment dit « `end_session(` y reste jusqu'à V2d-4b » | `chat.rs:1584-1586` | commentaire **faux après ce vol** : à réécrire |
| Garde de `handlers.rs` : mêmes propriétés, **plus un contre-témoin** | `handlers.rs:3026` | le contre-témoin est le modèle de D5 |
| Banc `rewind_remote::tests::fake_spirit(&[(chemin, statut, corps)])`, `fallback` → **501**, `RequeteRecue{methode, chemin, autorisation, corps}` sans `Debug` | `rewind_remote.rs:540-592` | emprunté, jamais recopié ; on compte par `vues.len()` |
| Le `mod tests` de `chat.rs` importe déjà `fake_spirit`, `StatusCode`, `DeadEndpoint`, et porte `base_en_memoire()`, `jeton_pose(valeur)`, `ROUTE_SESSIONS`, `CORPS_OUVERT` | `chat.rs:1313-1340` | **zéro ligne d'import à ajouter** ; les tests neufs réutilisent les fixtures |
| Le motif du contrôle positif est déjà écrit dans ce module (« *sans lui, un refus tombé pour toute autre raison rendrait ce test vert en mesurant autre chose* ») | `chat.rs:1342-1398` | reconduit en T-C et T-B |
| `DeadEndpoint::reserve()` est le point injoignable du crate (mika#2569) | `chat.rs:1498` | T-E ; jamais un `bind(:0)` suivi d'un `drop` |
| CI : `cargo clippy --all-targets --all-features -- -D warnings` | `.github/workflows/ci.yml:76` | un warning rougit la CI ; aucun `allow`, aucun `expect` n'est ajouté par ce vol |

**Et les faits qui sont des absences :**

| absence mesurée | comment | conséquence |
|---|---|---|
| Aucun test ne couvre les trois sorties | `grep -n "end_session" chat.rs` → l. 798, 915, 1014 (production) et l. 1585 (doc-comment). Rien dans le `mod tests` | les cinq tests de D1 sont **neufs** ; rien à rebrancher |
| `run()` n'est pas testable unitairement | elle prend le terminal, pose le hook de panique et `enable_raw_mode` | le critère 3 ne s'atteste pas par un test de bout en bout : voir D3 |
| `/compact` n'a aucun test | `grep -n "compact" handlers.rs` : dispatch l. 41, fonction l. 262, rien sous `mod tests` (l. 1463+) | c'est le poste qui fait déborder le vol entier (§ 0) |
| `lib.rs` n'expose pas `tui` (seuls `ask_message`, `budget_floor`, `remote_ask`, `supervision`) | lecture de `lib.rs` | `App` est binaire-privé, donc `dead_code` s'applique à ses champs — le fait qui **force** le retrait d'`App.llm` au vol suivant |
| `App.llm` a **un seul** lecteur | `handlers.rs:273`. `chat.rs:605, 854, 954` n'y écrivent | le champ reste lu dans ce vol : intouché |
| `App::new(` a **quatre** appelants, dont `tui/input.rs:1181` qui porte son propre `NoopLlmProvider` | `chat.rs:598`, `app.rs:2354`, `handlers.rs:1533`, `input.rs:1181` | une tension de périmètre, reportée avec la moitié `/compact` : voir § 8 |
| `docs/egress/` : l'entrée `cli-spirit-sessions` désigne `session_remote.rs` | corps du ticket, non contredit par la mesure | `chat.rs` appelle une porte, il ne construit aucun client : le manifeste ne change pas |

---

## 2. Décisions

### D1 — une fonction, cinq paramètres, `anyhow::Result<()>`

```rust
/// Clôt la session du fil sur mika-spirit, au nom de l'agent qu'on quitte.
async fn clore_session_de_sortie(
    base_url: &str,
    lire_env: SourceEnv<'_>,
    agent: &str,
    session_id: &str,
    is_singleton_session: bool,
) -> Result<()>
```

Même forme que `ouvrir_session_par_spirit` (`chat.rs:50-104`) et que
`clear_par_spirit` (`handlers.rs:137`) : **l'URL de base et la source
d'environnement arrivent par argument**, pour qu'un test les pilote. Un client
qui résout ses propres accès ne se pilote pas depuis un test, et un chargement
caché derrière le paramètre ferait mentir la signature.

L'ordre du corps est le contrat du critère 2, et il est **mesurable** :

1. `if is_singleton_session { return Ok(()); }` — **avant** toute lecture du
   jeton. La canonique ne se clôt jamais (mika#1401), donc il n'y a rien à
   demander et aucune raison d'exiger une variable d'environnement pour ne rien
   faire. T-B l'atteste avec une source d'environnement **vide**, ce qui est le
   seul montage qui distingue « on n'a pas appelé » de « on a lu le jeton puis
   on n'a pas appelé » ;
2. `let jeton = jeton_interne(lire_env).map_err(anyhow::Error::msg)?;` — le refus
   nomme `MIKA_INTERNAL_TOKEN` parce que c'est `jeton_interne` qui le nomme.
   **Aucune seconde lecture du jeton n'est écrite dans ce vol** : la lecture
   unique de V2d-4b reste unique, et `grep -rn "fn jeton_interne"
   crates/mika-cli/src` doit toujours rendre une ligne ;
3. `clore(base_url, &jeton, agent, session_id).await` — l'erreur monte.

Le retour est `anyhow::Result<()>` et non `Result<_, SessionRemoteIssue>` : les
trois appelants sont dans `run()`, qui est en `anyhow`, et le modèle voisin fait
déjà `.map_err(anyhow::Error::msg)?` sur le refus de jeton. Un type d'erreur qui
mélange « variable absente » et « spirit a répondu 401 » n'est pas
`SessionRemoteIssue` : une variable absente n'a ni statut ni URL, et
`Unexpected { status: None }` se lirait « spirit injoignable » (motif mika#2331,
la raison pour laquelle `jeton_interne` rend `String`).

### D2 — le miroir est lu **dans** la fonction, et le retour est `()`

`clore` rend `EndedSessionMirror { ended, canonical }`, et un `ended: false`
canonique est le régime nominal de mika#1401 tandis qu'un `ended: false` non
canonique est un refus sans raison donnée. V2d-4b rend cette disjonction à
l'écran, parce que `/clear` **a** un écran.

Ici, aucun des trois sites n'en a : le site 3 a déjà fait `restore_terminal()?`
et `events.shutdown()`, le site 2 détruit son worker, le site 1 change d'agent.
Rendre le miroir aux appelants les obligerait à rejouer la même disjonction
**trois fois, sans destinataire**. La fonction la journalise donc elle-même, en
deux `tracing::warn!` distincts (canonique / sans raison donnée), et rend
`Ok(())` dans les deux cas : **une clôture refusée sur la canonique n'est pas un
échec**, et la rendre en `Err` ferait dire au `warn!` de l'appelant qu'un geste
a échoué alors qu'il n'avait pas lieu d'être. T-F l'atteste.

### D3 — l'agent quitté est **capturé**, pas relu

Le critère 3 exige que l'agent passé au changement d'agent soit celui qu'on
quitte, « *l'appel précède la réaffectation d'`app.agent_name`* ». Aujourd'hui
cette distance est de **59 lignes** (l. 798 → l. 857), et `run()` n'est pas
testable unitairement : il n'y a pas de test de bout en bout à écrire.

Deux remèdes étaient possibles. Une **garde d'ordre** qui compare les index de
ligne des deux motifs dans la moitié production — ~22 lignes, et fragile : elle
mesure une position dans un fichier, pas un fait du programme, et rougirait sur
tout déplacement innocent. Ou bien **supprimer le mode de panne** :

```rust
// L'agent qu'on quitte, lu ici et non dans la clôture : `app.agent_name` est
// réaffecté 59 lignes plus bas (l. 857), et une clôture au nom du nouvel agent
// clôrait une session qui n'est pas la sienne. La capture ramène à deux lignes
// une distance qu'aucun test de `run()` ne peut surveiller.
let agent_quitte = app.agent_name.clone();
if let Err(e) = clore_session_de_sortie(
    &base_url, &|nom| std::env::var(nom).ok(), &agent_quitte,
    &app.session_id, app.is_singleton_session,
).await {
    tracing::warn!(error = %e, agent = %agent_quitte,
        "la session n'a pas été close sur spirit au changement d'agent");
}
```

**On retient la capture.** Un détecteur qui surveille une distance coûte plus
cher que la suppression de la distance, et laisse le défaut atteignable entre
deux rouges. Le commentaire porte le pourquoi — c'est exactement ce que la borne
du ticket protège quand elle dit que les commentaires ne se rabotent pas.

### D4 — le `warn!` nomme le **geste**, et c'est la valeur du retour à l'appelant

Les trois sites font un `tracing::warn!` sur échec **et le geste continue** : la
sortie, le changement d'agent et `/restart` ne dépendent jamais de spirit. Si la
fonction journalisait elle-même l'échec, les trois lignes seraient identiques et
l'opérateur lirait « la clôture a échoué » sans savoir **laquelle des trois
sorties** l'a demandée. C'est précisément ce que l'`Err` qui monte achète : trois
messages, trois gestes nommés, une cause portée par `error = %e`.

### D5 — la garde de `chat.rs` gagne un motif **et** un contre-témoin

```rust
const ALLOWED: &[&str] = &[];
assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
const MOTIFS: &[(&str, &str)] = &[
    ("create_session(", "session_remote::ouvrir"),
    ("get_or_create_canonical_session(", "session_remote::ouvrir"),
    ("end_session(", "session_remote::clore"),
];
```

Trois propriétés sont conservées telles quelles, parce qu'elles sont le livrable
de la garde et non sa décoration : l'**allowlist vide et assertée vide** (quand
la garde rougit, le remède est la porte que son message nomme, jamais une
entrée) ; le **témoin** `async fn spawn_agent_worker(` (sans lui, un chemin faux
ou un balayeur qui masque tout serait vert **par cécité**) ; et le **`flat_map`
avec un seul `assert!` final**, pour que le rouge nomme tous les sites à
déplacer et pas une moitié.

Une propriété est **ajoutée** : le **contre-témoin**, sur le modèle de la garde
de `handlers.rs`.

```rust
assert!(
    !production.contains("fn mika1727_v2d4a_un_point_injoignable_nomme_lurl"),
    "la garde n'a pas masqué la moitié test de chat.rs"
);
```

`production_of()` **masque** les régions `#[cfg(test)]` — leurs lignes
deviennent vides — et ne **tronque jamais** ; une région infermable est laissée
**visible** exprès (`SliceReport::unclosed`, mika#2398). Jusqu'à ce vol, les
deux motifs de cette garde n'apparaissaient nulle part dans la moitié test de
`chat.rs`, donc un masquage perdu serait resté invisible. Ce vol ajoute cinq
tests qui exercent la clôture ; le contre-témoin est ce qui garantit qu'un
masquage perdu rougira **en nommant le masquage**, au lieu de faire rougir le
motif et d'inviter à « réparer » la garde par une entrée d'allowlist. La cible
est un nom de test **existant** (l. 1493), choisi pour ne pas dépendre des tests
que ce vol écrit.

Le doc-comment de la garde est réécrit : il dit aujourd'hui que `end_session(`
« y reste jusqu'à V2d-4b », ce qui est faux dans les deux sens après ce vol.

**Aucun test neuf de ce vol ne contiendra la chaîne `end_session(`.** La route
est `/api/v1/sessions/{id}/end` et les noms de tests sont en français
(`…clot…`) : le motif n'a pas de raison d'apparaître. Si un rédacteur futur en
introduit un dans la moitié test, le contre-témoin dira lequel des deux défauts
s'est produit.

### D6 — le rouge-avant est **montré**, motif par motif

Le critère 7 exige du pilote qu'il montre « le rouge-avant de chaque motif neuf
sur le code de `main`, puis le vert ». Pour ce vol, un seul motif est neuf
(`end_session(`), et son rouge-avant est **gratuit** : il suffit d'ajouter le
motif avant de re-router les trois sites, et de lancer la garde. Elle doit
nommer les trois lignes 798, 915 et 1014. C'est la première chose du commit 1, et
la sortie est collée dans le corps de la PR.

### D7 — ce vol ne touche ni `session_remote.rs`, ni `app.rs`, ni `input.rs`

Conséquence directe de la coupe du § 0, et la raison pour laquelle le critère 9
est **plus strict** pour ce vol que pour le ticket : seuls `commands/chat.rs` et
`crates/mika-cli/CLAUDE.md` changent. `bash scripts/verify-egress-manifest.sh
--report` est lancé avant la PR et doit être vert sans qu'aucune entrée bouge ;
s'il est rouge, le pilote s'arrête et le dit.

### D8 — la doc dit les sorties, et **annonce** ce qui reste

`crates/mika-cli/CLAUDE.md`, section TUI, gagne une sous-section
`### Les sorties de la TUI passent par mika-spirit (mika#1727, vol V2d-4c)` :
les trois sites, le fait qu'une clôture en échec **ne retient pas la sortie**
(et pourquoi : la TUI ne doit pas dépendre de spirit pour quitter), la capture de
l'agent quitté, et le cas canonique. Deux phrases existantes sont réconciliées
parce qu'elles sont fausses après ce vol : celle du § V2d-4a (« `end_session(`
n'est **pas** un motif de *cette* garde ») et celle du § V2d-4b (« `maybe_compact(`
n'est pas un motif : il part au vol V2d-4c »), qui devient « au vol suivant ».
La ligne `/clear` de la liste des slash commands (la « phrase de la l. 573 » du
ticket) est **relue et laissée telle quelle si elle reste vraie** — elle décrit
`/clear`, que ce vol ne touche pas ; `/compact` n'y a pas de ligne à mettre à
jour puisque ce vol ne le change pas. Le pilote le dit dans la PR plutôt que
d'écrire une mise à jour qui n'en est pas une.

### D9 — aucun ticket n'est ouvert

Gel jusqu'au 01/12. Ce qui reste va dans le corps de la PR, sous
`Tracked in: senara-solutions/mika#1727`. Pas de sous-ticket, pas de ticket de
suivi, pas même pour la tension `App.llm` / `input.rs` du § 8.

---

## 3. Livrables, un par un

1. **`clore_session_de_sortie`** dans `commands/chat.rs`, à côté de
   `ouvrir_session_par_spirit` : signature de D1, ordre de corps de D1, miroir
   journalisé selon D2.
2. **Les trois sites re-routés** (l. 794-798, 908-917, 1006-1017) : capture de
   l'agent (site 1 seulement — les deux autres n'ont pas de réaffectation à
   précéder, et y poser une capture inutile dirait un pourquoi faux), appel,
   `tracing::warn!` nommant le geste. Le `!app.is_singleton_session` disparaît
   des trois sites : la décision est **dans** la fonction, et la laisser aussi
   chez l'appelant en ferait un second juge du même fait.
3. **La garde** `mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local` : motif
   `end_session(` → `session_remote::clore`, contre-témoin, doc-comment réécrit.
   Le nom de la garde ne change pas — c'est une garde par fichier, pas par vol,
   et la renommer casserait la trace des deux vols qui la citent.
4. **Six tests** (§ 4), dans le `mod tests` de `chat.rs`, avec les fixtures qui
   y sont déjà.
5. **La doc** : `crates/mika-cli/CLAUDE.md` selon D8.

---

## 4. Ordre des commits

Trois commits. Le pilote mesure **après chacun** :
`git diff --numstat origin/main...HEAD`, plan exclu.

| # | contenu | fichiers | critères |
|---|---|---|---|
| 1 | Le motif `end_session(` ajouté à la garde **seul**, le rouge-avant montré (il doit nommer l. 798, 915, 1014), puis `clore_session_de_sortie` + les trois sites + le contre-témoin + le doc-comment, jusqu'au vert | `commands/chat.rs` | 3, 7 (moitié `chat.rs`) |
| 2 | Les six tests de la clôture de sortie | `commands/chat.rs` | 1, 2 |
| 3 | La doc | `crates/mika-cli/CLAUDE.md` | 8 (la part « la PR le dit ») |

Le commit 1 porte la garde **avec** son test — la garde *est* un test, et son
rouge-avant est la preuve qu'elle mesure quelque chose. Les six tests de
comportement suivent dans le commit 2 plutôt que d'être fondus dans le 1 :
« chaque garde arrive avec ses tests, dans le même commit » est tenu par le
commit 1, et deux commits plus petits rendent la halte sur la borne plus fine,
comme le demande le ticket. **Les tests ne se reportent jamais à un vol
suivant** : si le budget ne tenait pas, c'est le périmètre qui se réduirait —
c'est exactement ce que fait le § 0, et c'est pourquoi il n'y a plus de tension
de budget ici.

**Halte du pilote sur la borne.** Après chaque commit, si le total dépasse
**600** lignes ajoutées : arrêt. Pas de commit suivant, pas de PR. Dernier
message : « HALTE borne : N lignes pour 600 », avec le compte par commit. À 226
lignes attendues, cette halte ne devrait pas se déclencher ; si elle le fait,
c'est que l'estimation du § *Taille estimée* est fausse, et le pilote s'arrête
plutôt que de raboter des commentaires (le ticket l'interdit nommément).

---

## 5. Jeu de tests

Tous dans le `mod tests` de `commands/chat.rs`, avec `fake_spirit`,
`base_en_memoire()`, `jeton_pose()` et `DeadEndpoint` **déjà importés**. Le banc
est emprunté, jamais recopié : un second témoin de socket ouvrirait une seconde
vérité sur ce que le serveur reçoit, libre d'en diverger. Les requêtes se
comptent par `vues.len()` — `RequeteRecue` ne dérive pas `Debug`.

| id | nom | ce qu'il mesure | critère |
|---|---|---|---|
| T-A | `mika1727_v2d4c_la_cloture_de_sortie_poste_sur_la_route_de_fin_avec_son_jeton` | sur la socket du banc : `POST /api/v1/sessions/{id}/end`, `Authorization: Bearer <jeton>`, et un corps qui porte **l'agent passé** — pas celui de la fixture | 1 |
| T-B | `mika1727_v2d4c_un_singleton_ne_clot_rien_et_ne_lit_pas_le_jeton` | `is_singleton_session = true` + source d'environnement **vide** → `Ok(())`, `vues.len() == 0`. La source vide est le montage : avec un jeton posé, le test ne saurait pas distinguer « on n'a pas lu » de « on a lu puis renoncé » | 2 |
| T-C | `mika1727_v2d4c_sans_jeton_la_cloture_refuse_avant_tout_appel_en_nommant_la_variable` | non singleton, source vide → `Err` contenant `MIKA_INTERNAL_TOKEN`, `vues.len() == 0`. **Contrôle positif obligatoire** : le même montage avec un jeton posé doit atteindre la socket, sinon un refus tombé pour toute autre raison rendrait ce test vert en mesurant autre chose | 2 |
| T-D | `mika1727_v2d4c_un_401_est_rendu_en_erreur` | banc répondant `401` sur la route de fin → `Err`, et la socket a bien été touchée (`vues.len() == 1`) | 2 |
| T-E | `mika1727_v2d4c_un_point_injoignable_est_rendu_en_erreur` | `DeadEndpoint::reserve()`, garde tenue pendant tout l'appel → `Err`. Jamais un `bind(:0)` suivi d'un `drop` (mika#2569) | 2 |
| T-F | `mika1727_v2d4c_une_session_canonique_refusee_nest_pas_un_echec` | corps `{"ended":false,"canonical":true}` en **200** → `Ok(())`. C'est le contrat de D2 : la canonique ne se clôt jamais (mika#1401) et le geste n'a pas échoué | — (contrat de D2) |
| T-G | `mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local` (**existante, étendue**) | trois motifs, allowlist vide et assertée vide, témoin, **contre-témoin** | 7 |

T-F n'est exigé par aucun critère du ticket ; il est écrit parce que D2 introduit
un comportement (un `ended: false` qui n'est pas une erreur) qu'aucun autre test
ne tient. Les deux `tracing::warn!` de D2 ne sont **pas** attestés : un `warn!`
n'est pas observable sans capture de traces, et monter un abonné pour deux lignes
de journal coûterait plus que ce qu'il prouve. C'est dit au § 8.

---

## 6. Definition of Done

- [ ] `clore_session_de_sortie` existe dans `commands/chat.rs`, à la signature de
      D1, et son corps refuse dans l'ordre : singleton → jeton → appel.
- [ ] Les trois sites (changement d'agent, `/restart`, fin de session) l'appellent ;
      aucun n'appelle `async_db.end_session(` ; chacun fait un `tracing::warn!`
      qui nomme son geste, et le geste continue.
- [ ] Au changement d'agent, `let agent_quitte = app.agent_name.clone();`
      précède immédiatement l'appel, et le commentaire dit pourquoi.
- [ ] `!app.is_singleton_session` n'apparaît plus aux trois sites.
- [ ] La garde de `chat.rs` refuse `create_session(`,
      `get_or_create_canonical_session(` et `end_session(` ; `ALLOWED` est vide
      et `assert!(ALLOWED.is_empty(), …)` est présent ; le témoin et le
      contre-témoin sont présents ; le doc-comment ne parle plus de V2d-4b.
- [ ] Le rouge-avant de `end_session(` a été montré sur le code de `main`,
      nommant les l. 798, 915 et 1014, et la sortie est dans le corps de la PR.
- [ ] Les six tests T-A à T-F passent. T-C porte son contrôle positif.
- [ ] `cargo test -p mika-cli` vert.
- [ ] `cargo clippy -p mika-cli --all-targets` vert, **sans `allow` ni `expect`
      ajouté**, et sans attente insatisfaite.
- [ ] `cargo fmt --check` vert.
- [ ] `grep -rn "fn jeton_interne" crates/mika-cli/src` rend **une** ligne.
- [ ] `grep -c "expect(dead_code"
      crates/mika-cli/src/commands/session_remote.rs` rend **2**, inchangé — ce
      vol ne câble pas `compacter`, et les retirer rougirait la CI en `dead_code`.
- [ ] `crates/mika-cli/CLAUDE.md` décrit les sorties sur spirit et pourquoi une
      clôture en échec ne retient pas la sortie ; les deux phrases qui
      annonçaient ce vol sont réconciliées.
- [ ] `git diff --name-only origin/main...HEAD` (plan exclu) ne rend que
      `crates/mika-cli/src/commands/chat.rs` et `crates/mika-cli/CLAUDE.md`.
- [ ] `bash scripts/verify-egress-manifest.sh --report` vert, aucune entrée
      modifiée.
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] `git diff --numstat origin/main...HEAD` (plan exclu) ≤ **600** lignes
      ajoutées.
- [ ] Le corps de la PR porte `Refs mika#1727` et **aucun** `Closes`, `Fixes` ni
      `Resolves` ; `gh pr view <N> --json closingIssuesReferences` rend `[]`.
- [ ] Le corps de la PR porte
      `Code review: delegated to mika-qa (mika#2694)`, la coupe du § 0 en une
      phrase, et `Tracked in: senara-solutions/mika#1727` pour la suite.
- [ ] Aucun ticket ouvert.

---

## 7. Acceptance criteria

Les dix critères du ticket, transcrits **verbatim**, avec pour chacun ce que ce
vol en tient. La coupe du § 0 en reporte cinq ; les reportés sont nommés ici
plutôt qu'omis, pour que la revue compare au ticket et non à une liste
raccourcie.

1. **« Pour une session non singleton, la clôture de sortie envoie
   `POST {base}/api/v1/sessions/{id}/end`, avec `Authorization: Bearer <jeton>`
   et un corps qui porte l'agent passé. Un test lit ces faits sur la socket du
   banc. »** — Tenu : T-A.

2. **« Pour un singleton, la clôture de sortie n'envoie rien, même sans jeton.
   Sans jeton, sur une session non singleton, elle n'envoie rien et rend une
   erreur qui nomme `MIKA_INTERNAL_TOKEN`. Sur un 401 ou un point injoignable,
   elle rend une erreur. Un test par cas. »** — Tenu : T-B, T-C, T-D, T-E.

3. **« Les trois sites de `chat.rs` (changement d'agent, `/restart`, fin) passent
   par la clôture de sortie. Un échec y donne un `tracing::warn!` et le geste
   continue. Au changement d'agent, l'agent passé est celui qu'on quitte :
   l'appel précède la réaffectation d'`app.agent_name`. »** — Tenu : livrable 2,
   garde T-G pour la première phrase (plus aucun `end_session(` en production
   dans ce fichier), D3 pour la troisième. `run()` n'étant pas testable
   unitairement, la troisième phrase est tenue en **supprimant** la distance
   (capture locale) et non en la détectant ; le ticket ne demande pas « un test
   par cas » sur ce critère, contrairement aux critères 1, 2, 4 et 5.

4. **« `/compact` sur un agent pas au repos refuse sans appel, et le banc ne
   reçoit rien. Sans jeton, il refuse avant tout appel. Un test par cas. »** —
   **Reporté** (§ 0). `Tracked in: senara-solutions/mika#1727`.

5. **« `/compact` envoie `POST {base}/api/v1/compact`, avec `Bearer` et `agent`.
   Sur `compacted: false`, le message dit qu'il n'y a rien à compacter, avec le
   compte rendu. Sur `compacted: true`, le message dit que la compaction a été
   demandée, avec le compte d'avant, et ne contient pas « compacted » au passé ni
   « compactée ». Sur un 409, le message dit que l'agent est occupé. Un test par
   cas. »** — **Reporté** (§ 0).

6. **« Aucun seuil de compaction n'est écrit dans `handlers.rs` : la moitié
   production ne compare plus `count_messages()` à une constante pour
   `/compact`. »** — **Reporté** (§ 0). Mesure à verser au vol suivant : les
   seuls `50` de production sont `handlers.rs:270-271` ; `count_messages()` reste
   légitimement appelé l. 444 par une autre commande, donc il ne peut pas devenir
   un motif de garde, et le critère se tiendra par le motif `maybe_compact(`
   plus un test sur le compte **rendu par le serveur**.

7. **« La garde de `chat.rs` refuse `create_session(`,
   `get_or_create_canonical_session(` et `end_session(`. Celle de `handlers.rs`
   refuse `create_session(`, `end_session(` et `maybe_compact(`. Allowlists vides
   et assertées vides, témoins présents. Le pilote montre le rouge-avant de
   chaque motif neuf sur le code de main, puis le vert. »** — **Tenu pour la
   moitié `chat.rs`** : D5, D6, T-G. La moitié `handlers.rs` (`maybe_compact(`)
   est **reportée** avec le câblage de `compacter` : un motif de garde sans son
   câblage rougirait sur du code que rien ne remplace, ce qui est le détecteur
   livré armé que la Fire-Disposition interdit.

8. **« Le pilote lance `cargo clippy -p mika-cli --all-targets`. Le résultat est
   vert, sans `allow`, et sans attente insatisfaite.
   `grep -c "expect(dead_code" crates/mika-cli/src/commands/session_remote.rs`
   rend 0, et la doc du module le dit. Si `App.llm` a été retiré, la PR le dit,
   avec l'avertissement qui l'a demandé. »** — **Tenu pour la première phrase**
   (clippy vert, aucun `allow` ni `expect` ajouté). Le compte à **0** et le
   retrait d'`App.llm` sont **reportés** : les deux `expect` portent sur
   `compacter` et `CompactedMirror`, et `App.llm` a encore son lecteur
   (`handlers.rs:273`) tant que `/compact` n'est pas câblé. La PR dit que le
   compte reste à 2 et pourquoi.

9. **« Seuls `commands/chat.rs`, `tui/commands/handlers.rs`,
   `commands/session_remote.rs`, `crates/mika-cli/CLAUDE.md` et, pour le seul
   retrait d'`App.llm`, `tui/app.rs` changent. Le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le
   trouve rouge, il s'arrête et le dit. »** — Tenu, et **plus strictement** : ce
   vol ne touche que `commands/chat.rs` et `crates/mika-cli/CLAUDE.md`. Le
   manifeste d'egress est lancé et doit être vert sans qu'aucune entrée bouge.

10. **« La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
    compris si le gabarit de reprise l'ouvre. »** — Tenu. Si le label
    `wip-rescue` ouvre la PR, son gabarit écrit `Closes #N` par défaut (corrigé à
    la main sur #3077 et #3081) : le pilote le corrige **avant toute revue**,
    puis vérifie que `gh pr view <N> --json closingIssuesReferences` rend `[]`.

**Et la clause de halte du ticket :** « *Si le groom juge qu'un critère ne tient
pas sans toucher un fichier exclu ci-dessus, il s'arrête et le dit, au lieu
d'élargir.* » Elle n'est pas déclenchée pour ce vol : les critères 1, 2, 3, 7
(moitié `chat.rs`), 9 et 10 se tiennent entièrement dans `commands/chat.rs` et
`crates/mika-cli/CLAUDE.md`. Elle **l'aurait été** pour le vol entier, et c'est
le § 8 qui le dit.

---

## Fire-Disposition

Ce vol livre un détecteur : la garde
`mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local` gagne le motif
`end_session(`. Section obligatoire (mika#2306).

**Option retenue : (a) — allowlist nommée, livrée à zéro entrée, et assertée
vide.**

```rust
const ALLOWED: &[&str] = &[];
assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
```

Les quatre propriétés de mika#1574 :

1. **Le détecteur est livré désarmé-par-vacuité, pas désarmé-par-exception.**
   `ALLOWED` est vide parce qu'il n'y a **aucun** site légitime : après le
   livrable 2, la moitié production de `chat.rs` ne contient plus
   `end_session(`. Le rouge-avant de D6 est la preuve que le détecteur mesure
   quelque chose, et le vert d'après la preuve qu'il n'y a rien à excepter.
2. **Le remède nommé est une porte, jamais une entrée.** Le message d'échec porte
   `remède : session_remote::clore` pour chaque ligne fautive. Un futur appelant
   se route par la porte existante (doctrine mika#2201).
3. **L'allowlist ne peut pas cesser d'être vide en silence.**
   `assert!(ALLOWED.is_empty(), …)` rougit à la première entrée ajoutée, avant
   même le balayage : une allowlist qui se remplit est le faux-vert que cette
   forme existe pour refuser (motif mika#2309 / mika#2457).
4. **Le détecteur ne peut pas être vert par cécité.** Témoin
   (`async fn spawn_agent_worker(` — la garde a lu la production) **et**
   contre-témoin (`fn mika1727_v2d4a_un_point_injoignable_nomme_lurl` absent — la
   garde a masqué la moitié test). `production_of()` masque sans tronquer et
   laisse une région infermable **visible** exprès (mika#2398) : les deux témoins
   distinguent les deux pannes, là où un seul `assert!` les confondrait.

**`expect(dead_code)` : reconduit, deux restants, aucun ajouté ni retiré.** La
Fire-Disposition de V2d-3a avait choisi `expect` plutôt qu'`allow` parce qu'au
câblage l'attribut devient `unfulfilled_lint_expectation` et que le build le dit
— l'attribut est posé sur l'**item**, donc il couvre ses champs, et lire un seul
champ d'un miroir laisserait l'attente satisfaite. Ce vol ne câble pas
`compacter` : les deux `expect` de `session_remote.rs` restent, le compte reste
**2**, mesuré sous `cargo clippy -p mika-cli --all-targets`, et la doc du module
reste vraie sans retouche. Les retirer ici rougirait la CI en `dead_code`
(`-D warnings`), ce qui est l'inverse de ce que la section protège.

**Aucun détecteur n'est livré armé sur du code que ce vol ne corrige pas** : le
motif `maybe_compact(` de la garde de `handlers.rs` est reporté avec son
câblage, pour cette raison exacte (critère 7).

---

## 8. Ce que ce vol ne fait pas

- **`/compact` ne passe pas par `compacter`.** `handle_compact`
  (`handlers.rs:262-276`) garde son seuil `50` écrit en dur (l. 270-271) et son
  `mika_agent::compaction::maybe_compact(&app.db, app.llm.as_ref())` (l. 273).
  Le vol suivant écrit `compact_par_spirit` à la forme de `clear_par_spirit`
  (base + source d'environnement), ses quatre bras (`compacted: false` / `true` /
  `Busy` / `Display` de `SessionRemoteIssue`), ses six tests, et le motif
  `maybe_compact(` de la garde. `Tracked in: senara-solutions/mika#1727`.
- **La grounding rule reste à tenir au vol suivant** : le message de
  `compacted: true` doit dire que la compaction a été **demandée**, jamais
  qu'elle « a été compactée » — `CompactedMirror.compacted` veut dire « demandée
  et pas en échec », jamais « un résumé a été écrit ».
- **Les deux `expect(dead_code)` de `session_remote.rs` restent** (`compacter`,
  `CompactedMirror`), et la doc du module reste telle quelle : elle annonce déjà
  que le chiffre se mesure et qu'il tombera au câblage de la dernière porte.
- **`App.llm` n'est pas touché, et il reste lu.** Son unique lecteur est
  `handlers.rs:273`.

  **Une tension de périmètre à trancher au vol suivant, nommée ici parce qu'elle
  est mesurée ici.** Le retrait d'`App.llm` sera **forcé**, non facultatif :
  `lib.rs` n'expose pas `tui`, donc `App` est binaire-privé et `dead_code`
  s'applique à ses champs ; la CI fait `cargo clippy --all-targets
  --all-features -- -D warnings` (`ci.yml:76`) ; et le ticket interdit `allow`
  comme `expect` sur ce champ. Dès que `/compact` ne le lit plus, la CI rougit si
  le champ reste. Mais `App::new(` a **quatre** appelants, dont
  `tui/input.rs:1181`, qui porte son propre `NoopLlmProvider` (l. 1127-1155) et
  son `use mika_common::llm::{…}` (l. 1121) : retirer le paramètre y rend ~34
  lignes mortes, dans un fichier que le critère 9 n'énumère pas. Le § *Périmètre*
  du ticket dit « constructeurs de test » au **pluriel** et la mesure en trouve
  **trois** (`app.rs:2354`, `handlers.rs:1533`, `input.rs:1181`) là où le critère
  9 n'en nomme qu'un — un défaut de mesure, pas une intention. `tui/input.rs`
  n'est pas dans la liste nominale « Restent intacts », donc la clause de halte
  « fichier exclu » n'est pas déclenchée, et le changement est en **retraits
  seuls**, dans la moitié test, à zéro changement de comportement et à coût quasi
  nul sur une borne qui compte les lignes ajoutées. Le vol suivant devra
  l'isoler dans son **dernier** commit, pour rester livrable si la tension doit
  remonter à l'opérateur.
- **Les deux `tracing::warn!` de D2 ne sont pas attestés par un test.** Un
  `warn!` n'est pas observable sans capture de traces, et monter un abonné pour
  deux lignes de journal coûterait plus que ce qu'il prouve. Ce qui **est**
  attesté est le fait qui compte : un `ended: false` canonique rend `Ok(())`
  (T-F).
- **`commands/ask.rs` n'est pas la TUI** et n'est pas touché. `rewind_remote.rs`,
  `dashboard.rs`, `crates/mika-agent/` et `crates/mika-a2a/` restent intacts.
- **Aucun fichier du plan de contrôle** : ni `.github/workflows/`, ni `.claude/`,
  ni `skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni
  `.mika/`, ni aucun chemin de CODEOWNERS. `docs/egress/`, `docs/openapi/` et
  `docs/slash-commands.md` ne sont pas touchés.
- **Aucun ticket n'est ouvert** (gel jusqu'au 01/12), pas même de suivi.
- **V3 et V4 ne sont pas redécoupés.** #1727 se ferme au merge de V4, et de là
  seulement.

---

## Taille estimée

| poste | lignes ajoutées |
|---|---|
| `clore_session_de_sortie` + son doc-comment (`chat.rs`) | 34 |
| Les trois sites re-routés (capture, appel, `warn!`) | 27 |
| La garde : motif, contre-témoin, doc-comment réécrit | 11 |
| Les six tests T-A…T-F + deux aides locales (`ROUTE_FIN`, `CORPS_CLOS`) | 128 |
| **sous-total hors `docs/` et hors `CLAUDE.md`** | **200** |
| `crates/mika-cli/CLAUDE.md` (D8) | 26 |
| **total dans l'unité du ticket** (`git diff --numstat`, plan exclu) | **226** |

Total estimé : 200 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les deux unités, et laquelle mord.** Le total ci-dessus est écrit dans l'unité
de mika#2636 — **lignes de code, hors `docs/` et hors `CLAUDE.md`** —, celle que
la borne par vol (429) compare. La borne du ticket (600) compte
`git diff --numstat origin/main...HEAD` plan exclu, **doc et `CLAUDE.md`
compris** : 226. Les deux unités sont écrites plutôt que l'une devinée, parce que
c'est le seul moyen pour que la halte du pilote et le jugement de l'architecte
portent sur le même nombre. La borne par PR (1000) ne mord pas.

Marge sur la borne par vol : **229 lignes (53 %)**. Marge sur la borne du ticket
(600, doc comprise) : **374 lignes**.

Cette marge est large et elle est **voulue**. Les deux vols jumeaux ont dépassé
leur estimation de +27 % (§ 0) ; appliqué à 200, l'écart porterait le réel à
~254, toujours très sous la borne. Elle n'est pas une invitation à reprendre un
morceau de `/compact` : la moitié reportée est un bloc indivisible de 254 lignes
estimées (câblage, les deux `expect`, `App.llm`, la garde, six tests), et en
prendre un fragment livrerait soit un détecteur armé sur du code que rien ne
remplace, soit un `expect` retiré sous une CI en `-D warnings`.
