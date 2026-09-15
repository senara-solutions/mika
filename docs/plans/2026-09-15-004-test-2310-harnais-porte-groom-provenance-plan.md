# Harnais isolé de la porte de provenance de grooming (mika#2310)

> - **Ticket :** senara-solutions/mika#2310
> - **Relie :** mika#2287 (la porte), mika#2288 (la campagne d'essais), mika#1620 (la classification de dispatch), mika#2286 (essai 5)
> - **Type :** test — aucun changement de comportement de production
> - **Ordre :** après mika#2295 (fait, `0a946b95`), avant l'essai 5 (mika#2286)

## L'objet, et ce que la spec ne savait pas

Le ticket demande de prouver la porte **loop-résistante par construction**, sans
groom live, pour que les essais live cessent d'être la preuve de la porte et ne
prouvent plus que les couches autour (budgets, fournisseur, worktree).

La spec (`~/.claude/plans/harnais-f-porte-2288-spec.md`, lue sur main `891004c9`)
énumère 9 cas et déclare le manque : « cas 2-9 au niveau `db.rs` + cas 9 bout en
bout ». **Le code a divergé de cette lecture, et c'est la première chose que ce
plan doit dire** : mika#2287 a livré dix tests au niveau `db.rs`
(`db.rs:25580-25832`) et trois au niveau verdict (`executor.rs:8065-8102`). Sept
des neuf cas sont déjà couverts. Le ticket prévoyait cette divergence
(« corriger si le code a divergé ») ; voici l'inventaire, cas par cas, établi par
lecture du code et non par relecture de la spec.

| Cas | État réel | Preuve existante |
|---|---|---|
| 1 — nominal → `true` + verdict `Ok` | **couvert** | `test_groom_cross_check_completed_callback_returns_true` (db.rs:25686) + `test_groom_provenance_verdict_proof_present_allows` (executor.rs:8065) |
| 2 — marqueur absent → `dispatch_grooming_not_verified` | **couvert** | `test_groom_cross_check_plan_iterate_returns_false` (25759), `…_pending_callback_returns_false` (25744) + `test_groom_provenance_verdict_no_proof_refuses` (8073) |
| 3 — mauvaise classe (`implement`) → `false` | **couvert** | `test_groom_cross_check_implement_class_callback_returns_false` (25769) |
| 4 — mauvais statut (`failed` / `cancelled`) | **MANQUE** | `pending` est testé (25744) ; `failed` et `cancelled` ne le sont pas |
| 5 — suffixe legacy `?phase=groom` → `true` | **couvert** | `test_groom_cross_check_legacy_suffixed_parent_url_returns_true` (25729) |
| 6 — parent d'une autre issue → `false` | **couvert** | `test_groom_cross_check_different_issue_returns_false` (25803) |
| 7 — agent différent → `false` | **couvert** | `test_groom_cross_check_different_agent_returns_false` (25786) |
| 8 — fail-closed : DB cassée → `Err` | **MANQUE à moitié** | le verdict est testé sur un `Err` **synthétique** (8092) ; que `has_completed_groom_for_issue` produise réellement un `Err` n'est prouvé nulle part |
| 9 — bout en bout (chemin de dispatch réel) | **MANQUE** | `test_dispatch_no_grooming_marker_guard.rs:250` le dit en toutes lettres : *« The full `validate_dispatch_readiness` integration path for the cross-check requires `fetch_issue_body` (HTTP call to GitHub API) which is not mocked »* |

Le manque réel est donc **3 cas sur 9**, pas 8. C'est une bonne nouvelle pour le
coût et une mauvaise pour le confort : les trois qui restent sont précisément les
trois durs — le statut terminal négatif (le cas des essais 3/4), le maillon
`Err` réel, et le bout en bout que la porte n'a jamais eu.

**Ce tableau est une prémisse portante, pas un constat de passage.** Il est établi
par lecture du code sur la branche de ce ticket ; ses numéros de ligne datent de
cette lecture et bougeront. Un relecteur qui n'a pas de shell — l'architecte en
première passe, par exemple — ne peut ni le ratifier ni le contester. Deux
conséquences, tenues ailleurs dans ce plan : il doit être recoupé avant merge
(§ Vérification, étapes 1-2, qui le mesurent par le vert de la suite existante), et
il doit être inscrit dans le corps de mika#2310 (AC9), parce que c'est lui qui
rend fausse la phrase « Le manque = cas 2-9 au niveau `db.rs` + cas 9 bout en
bout » que le ticket porte encore, et que c'est le ticket, non ce fichier, que
mika#2288 relira.

## Pourquoi les trois qui restent sont ceux qui comptent

**Cas 4 — le statut est le seul terme du prédicat qu'un essai live a réfuté.**
Les essais 3/4 de mika#2288 ont échoué sur un callback qui n'a jamais atteint
`completed`. Le prédicat filtre `status IN ('completed','delivered')`, et le seul
statut non-terminal testé aujourd'hui est `pending`. Un `failed` et un
`cancelled` portent un `result` **non vide** — c'est là toute la différence avec
`pending`, qui n'en porte aucun. Un prédicat qui aurait perdu son filtre de
statut resterait vert sur les dix tests actuels et faux sur le seul cas mesuré en
production.

**Cas 8 — un `Err` synthétique ne prouve pas qu'un `Err` arrive.** Le test 8092
construit `Err(anyhow!(…))` à la main et vérifie que le verdict le refuse. C'est
la moitié utile du contrat ; l'autre moitié — *le lecteur DB propage bien une
erreur plutôt que de rendre `Ok(false)`* — n'est attestée par rien. La distinction
n'est pas académique : `Ok(false)` et `Err` produisent deux rejections
différentes (`dispatch_grooming_not_verified` contre `dispatch_check_failed`),
et l'une dit à l'opérateur « ce ticket n'a pas été groomé » quand l'autre dit
« je n'ai pas pu lire ma preuve ». Confondre les deux, c'est envoyer l'opérateur
regroomer un ticket dont la base est en panne.

**Cas 9 — c'est la preuve que le ticket demande, et elle n'existe pas.** Le
commentaire de `test_dispatch_no_grooming_marker_guard.rs:250` renvoie au niveau
DB en disant que le chaînage n'y est pas couvert ; les tests DB, eux, ne savent
rien du site d'appel. Entre les deux, le segment
`executor.rs:1771-1785` — construction de l'URL, appel DB, verdict,
`record_dispatch_rejection` — n'est exercé par aucun test. C'est exactement le
segment qu'un essai live exerce, et c'est pour cela que les essais live sont
aujourd'hui la seule preuve de la porte.

## La conception

### D1 — Extraire le segment post-fetch, sans toucher à ce qu'il fait

Le bout en bout bute sur un fait de structure : `fetch_issue_body`
(`github_graphql.rs:161`) écrit `https://api.github.com/...` en dur, sans base-URL
injectable, et `validate_dispatch_readiness` l'appelle inconditionnellement dès
qu'un token est présent. Sans token, le bloc entier est sauté (fail-open) et le
cross-check n'est jamais atteint : il n'y a aucun réglage de `Some`/`None` qui
donne un bout en bout hors réseau.

Deux sorties, et une seule est proportionnée.

- **Rendre la base-URL de `github_graphql` configurable** et servir un faux corps
  d'issue depuis un serveur local. Cela touche dix fonctions d'un module partagé
  et introduit en production un point de configuration dont personne n'a besoin,
  pour le bénéfice d'un test. Rejeté.
- **Extraire le segment qui suit le fetch** en une fonction nommée, à qui le
  corps de l'issue est *donné* :

  ```rust
  pub(crate) async fn evaluate_grooming_gate(
      db: &AsyncDatabase,
      task_id: &str,
      owner: &str,
      repo: &str,
      number: u64,
      issue_body: &str,
  ) -> Result<(), serde_json::Value>
  ```

  Elle enchaîne `check_grooming_markers` → rejection `dispatch_no_grooming_marker`
  si manquant, sinon construction de l'URL → `has_completed_groom_for_issue` →
  `groom_provenance_verdict`. Le site d'appel `executor.rs:1735-1785` devient un
  appel à cette fonction suivi du `record_dispatch_rejection` + `return Err(…)`
  qu'il fait déjà. En production le corps vient de `fetch_issue_body` ; en test,
  d'une fixture. Retenu.

C'est bien ce que le cas 9 demande littéralement — *« marqueurs de grooming en
fixture (pas GitHub), appeler le chemin de dispatch réel »* : le seul maillon qui
sort du test est le transport HTTP, que le ticket exclut lui-même (« zéro
réseau »).

**L'extraction doit être neutre, et c'est vérifiable.** Les deux rejections du
bloc actuel appellent `record_dispatch_rejection` puis retournent `Err` ; en
remontant cet appel d'un cran dans l'appelant, l'ordre et le contenu sont
identiques. Le diff de production attendu est un déplacement de lignes, pas une
réécriture : si la revue y lit un changement de condition, c'est une erreur
d'implémentation, pas une intention.

**Une asymétrie préexistante est laissée telle quelle, et nommée pour qu'elle ne
soit pas corrigée par accident** : le bras `Err(e)` du fetch (`executor.rs:1787`)
retourne `dispatch_check_failed` **sans** appeler `record_dispatch_rejection`,
là où les deux autres rejections l'appellent. C'est peut-être un défaut ; ce n'est
pas celui de ce ticket. Un harnais qui répare silencieusement ce qu'il mesure
cesse d'être un harnais.

### D2 — Le contrôle négatif jumeau du cas 9 est obligatoire

Le cas 9 tel que rédigé demande de « vérifier aucune rejection
`dispatch_grooming_not_verified` ». **Une assertion d'absence passe aussi quand
le chemin n'évalue rien** : une `evaluate_grooming_gate` qui retournerait `Ok(())`
sans rien lire satisferait le cas 9 intégralement. Le harnais livrerait alors
exactement la fausse assurance qu'il est censé remplacer.

Le harnais ajoute donc **9b**, son jumeau : même chemin, même DB temporaire,
mêmes marqueurs en fixture, mais **sans** la paire parent+enfant — et l'attente
est cette fois positive : une `Err` dont le champ `error` vaut
`dispatch_grooming_not_verified`. 9 et 9b se lisent ensemble ; pris séparément,
aucun des deux n'atteste la porte.

### D3 — Où vivent les tests, et pourquoi pas dans `tests/eval/`

`validate_dispatch_readiness` est `pub(crate)` et `groom_provenance_verdict` est
privée : ni l'une ni l'autre n'est atteignable depuis `tests/eval/`, qui est une
cible externe au lib. Deux options : élargir la visibilité pour un test — le
précédent existe dans ce fichier même, `check_grooming_markers` est `pub` pour
cette raison — ou loger le harnais dans le lib, où tout est déjà accessible.

Le harnais va dans le lib, dans deux sous-modules nommés `harnais_porte` :

- `src/db.rs` → `mod tests` → `mod harnais_porte` : cas 4 et 8 (niveau prédicat).
- `src/skills/executor.rs` → `mod tests` → `mod harnais_porte` : cas 9 et 9b
  (niveau chemin de dispatch).

Zéro élargissement de visibilité, et le critère de sortie du ticket est satisfait
tel quel : `cargo test -p mika-agent harnais_porte` filtre sur le chemin complet
du test, donc `db::tests::harnais_porte::*` et
`skills::executor::tests::harnais_porte::*` sont tous deux sélectionnés.

Le sous-module porte le nom de la campagne, pas celui du mécanisme, parce que
c'est le nom que le critère de sortie du ticket interroge et que ce nom doit
rester lisible depuis mika#2288.

### D4 — Comment les cas 4 et 8 sont construits

**Cas 4 — construit intégralement par l'API d'écriture de production, et la
question de la machine à états est tranchée ici, pas à l'implémentation.**
Réutiliser les fixtures existantes de `db.rs` — `groom_parent`, `groom_callback`,
`completed_groom_pair` (25596-25674) — telles quelles, sans les dupliquer : c'est
ce que le ticket demande (« à réutiliser, ne pas dupliquer »). Le chemin est :

```rust
let (_, callback_id) =
    completed_groom_pair(&db, "mika", GROOM_ISSUE_URL, GROOM_CALLBACK_PLAN_GROOMED);
db.update_task_status(&callback_id, "failed").unwrap();   // puis "cancelled" dans le jumeau
```

`completed_groom_pair` écrit le `result` porteur de `GROOM_SUCCESS_MARKER` par
`update_task_completed`, c'est-à-dire par la voie de production ; `update_task_status`
(`db.rs:6552`) est un `UPDATE tasks SET status = ?1 WHERE id = ?2` **public et sans
aucune garde de transition**, et il ne touche pas `result`. La ligne obtenue porte
donc le marqueur *et* le statut visé : le statut est le seul terme qui la sépare du
cas nominal, ce qui est exactement la propriété que le cas 4 doit discriminer.

**La première rédaction de ce plan prévoyait, en repli, « l'écriture directe via
`db.conn` » si la machine à états refusait la transition. Ce repli est supprimé,
et il l'aurait été même s'il avait été nécessaire** : le GO ratifié de mika#2287
porte, condition 5, « Zéro INSERT SQL brut dans les tests : la preuve naît de
l'API d'écriture de prod » (condition 1 : aucun `INSERT`/`UPDATE`/`DELETE`). Un
harnais qui se construit hors de l'API de prod atteste d'un état que la prod
n'écrit pas.

Il n'était de toute façon pas nécessaire, et le précédent le prouve dans le même
fichier : `test_groom_cross_check_delivered_callback_returns_true` (db.rs:25692)
fait déjà `completed_groom_pair(…)` puis `update_task_status(&callback_id,
"delivered")`. Le cas 4 est ce geste, avec deux autres statuts. Il n'y a pas de
machine à états à franchir sur ce chemin — c'est la réponse à R2, qui cesse d'être
une question ouverte.

**Si l'implémentation découvre que ce chemin ne produit pas la ligne visée** (par
exemple si une garde était ajoutée à `update_task_status` entre ce grooming et le
commit), c'est un **halt-and-surface** : consigner la mesure dans mika#2310 et
remonter à l'opérateur. Ce n'est en aucun cas une autorisation d'écrire en SQL.

**Cas 8.** `Database::open_in_memory()` puis `db.conn.execute("DROP TABLE tasks",
[])` : le `SELECT` du prédicat lève alors un `rusqlite::Error` (« no such table »)
que le `?` propage. C'est déterministe, sans I/O sale et sans fichier corrompu.
L'assertion est `.is_err()` — pas le texte du message, qui appartient à SQLite et
non à nous.

**Substitution assumée, à consigner dans le ticket (AC9).** Le ticket écrit le cas 8
« DB fermée/corrompue → `Err` → `dispatch_check_failed`, jamais Ok » ; la
construction retenue est « table absente ». Les trois situations produisent un
`Err` par des mécanismes distincts (`rusqlite::Error` de variantes différentes) :
le test atteste donc d'un **voisin** du cas énoncé, pas du cas énoncé. « Fermée »
n'est pas constructible — `Database` ne expose pas la fermeture de sa connexion
sans la consommer ; « corrompue » demande un fichier sali, donc de l'I/O
non déterministe. « Table absente » est le plus proche voisin déterministe, et
c'est précisément parce que la substitution est invisible dans le nom du test
qu'elle doit vivre dans le corps du ticket et pas seulement ici — sans quoi la
relecture de mika#2288 conclura que le cas 8 du ticket est attesté tel quel.

Le `DROP TABLE` est une écriture SQL de **mise en scène de la panne**, pas une
écriture de preuve : la condition 5 de mika#2287 interdit de fabriquer par SQL
l'état que le prédicat doit lire. Ici le SQL détruit le support de lecture, il
n'écrit aucune ligne que le prédicat compte. La distinction doit être écrite en
commentaire au-dessus du test, faute de quoi le lecteur suivant y verra un
précédent d'écriture brute.

**Limite assumée, écrite plutôt que découverte :** le fail-closed n'est *pas*
prouvé bout en bout. `AsyncDatabase::new` consomme la `Database` et la déplace sur
son thread dédié, donc la table ne peut être détruite qu'avant construction — et
alors `db.get_task(task_id)` échoue en amont, dans `validate_dispatch_readiness`,
bien avant d'atteindre le cross-check. Le chaînage `Err → dispatch_check_failed`
reste attesté par la conjonction de deux tests (cas 8 : le lecteur produit `Err` ;
`executor.rs:8092` : le verdict refuse sur `Err`) sur la fonction que le site
d'appel utilise littéralement. C'est une preuve en deux morceaux, pas en un ; le
dire est moins coûteux que de laisser quelqu'un le redécouvrir en croyant le cas 8
plus fort qu'il n'est.

## Acceptance criteria

- **AC1** — Cas 4 : un callback de classe `groom`, sous un parent portant l'URL de
  l'issue, portant `Outcome: PLAN_GROOMED` dans son `result`, mais de statut
  `failed`, fait rendre `false` à `has_completed_groom_for_issue`. Idem pour
  `cancelled`. Deux tests distincts, pas un seul paramétré : les deux statuts sont
  deux populations que l'opérateur compte séparément.
- **AC2** — Cas 8 : sur une base dont la table `tasks` n'existe plus,
  `has_completed_groom_for_issue` rend `Err` — jamais `Ok(false)`. L'assertion
  porte sur la variante, pas sur le message.
- **AC3** — Cas 9 : sur une `AsyncDatabase` temporaire portant la paire
  parent+enfant du cas nominal, et un corps d'issue en fixture portant les trois
  marqueurs canoniques (`> - **Branch:**`, `docs/plans/`, un marqueur
  `second-pass`), le chemin de dispatch réel rend `Ok(())` — aucune rejection.
- **AC4** — Cas 9b : même chemin, même fixture de marqueurs, **sans** la paire
  parent+enfant → `Err` dont le champ `error` vaut exactement
  `dispatch_grooming_not_verified`. AC3 sans AC4 n'atteste rien.
- **AC5** — Les quatre tests sont sélectionnés par
  `cargo test -p mika-agent harnais_porte`, et ce filtre en sélectionne au moins
  quatre. Un filtre qui ne sélectionne rien sort en succès : le compte est donc
  vérifié, pas seulement le code de retour.
- **AC6** — Le diff sur le code de production se limite à l'extraction D1. Aucune
  condition, aucun message de rejection, aucun ordre d'appel ne change ; en
  particulier l'asymétrie du bras `Err(e)` du fetch (pas de
  `record_dispatch_rejection`) est préservée à l'identique.
- **AC7** — Les fixtures `groom_parent` / `groom_callback` / `completed_groom_pair`
  de `db.rs` sont réutilisées, pas dupliquées. `TEST_ISSUE_URL` et la ligne
  synthétique de `dispatcher.rs:5469` ne sont pas recopiées.
- **AC8** — `cargo test -p mika-agent` et `cargo clippy` passent ; la suite
  complète reste verte, y compris les dix tests mika#2287 existants, qui ne sont
  ni modifiés ni déplacés.
- **AC9** — Le corps de mika#2310 est amendé et porte (a) le tableau d'inventaire
  cas-par-cas avec ses numéros de ligne, qui remplace la phrase « Le manque =
  cas 2-9 au niveau `db.rs` + cas 9 bout en bout » devenue fausse, et (b) la
  substitution du cas 8 (« DB fermée/corrompue » → « table absente ») avec sa
  justification. Un commentaire d'avis d'édition signale l'amendement. Sans cet
  amendement, la trace du narrowing 8→3 ne vit que dans un fichier de plan et la
  relecture de mika#2288 devra relire le code pour la retrouver.
- **AC10** — La sortie rouge des deux mutations du contrôle négatif (Vérification,
  étape 5) est collée, horodatée, dans le corps de la PR — la mutation elle-même
  n'est pas commitée. Un contrôle négatif conduit et non consigné ne se distingue
  pas d'un contrôle négatif non conduit.

## Périmètre

**Dans le périmètre :** les cas 4, 8, 9, 9b ; l'extraction D1 ; la mise à jour du
commentaire `test_dispatch_no_grooming_marker_guard.rs:242-260`, qui affirme
aujourd'hui que le bout en bout n'est pas couvert et deviendrait faux — il doit
pointer vers le nouveau sous-module ; **l'amendement du corps de mika#2310**
(AC9), qui porte l'inventaire et la substitution du cas 8 là où mika#2288 ira les
chercher.

**Hors périmètre, délibérément :**

- Les cas 1, 2, 3, 5, 6, 7 : déjà couverts, et les réécrire sous le nom
  `harnais_porte` coûterait un diff pour zéro assurance nouvelle. Le tableau
  d'inventaire ci-dessus est la trace de cette décision, et AC9 la fait voyager
  jusqu'au corps du ticket : la relecture de mika#2288 doit pouvoir la retrouver
  sans relire le code, et un fichier de plan n'est pas l'endroit où elle
  regardera.
- L'injection de `fetch_issue_body` / la base-URL configurable de
  `github_graphql` (rejeté en D1).
- L'asymétrie `record_dispatch_rejection` du bras `Err(e)` du fetch (nommée en D1).
- La rétention 30 j (`prune_completed_tasks`) : le ticket la mentionne comme
  propriété du prédicat (« preuve élaguée = refus ») et non comme un cas à tester ;
  elle se réduit d'ailleurs au cas 2 — une preuve élaguée est une preuve absente.
- Tout changement de comportement de la porte. Ce ticket mesure ; il ne corrige
  rien.

## Fire-Disposition

La totalité du livrable est de classe détecteur (mika#1574) : quatre tests, une
assertion d'absence, et un contrôle négatif manuel, tous braqués sur du code
**déjà livré** par mika#2287. « Ce ticket mesure ; il ne corrige rien » est une
affirmation de périmètre ; elle ne devient une disposition qu'énoncée ici, faute
de quoi l'implémenteur arbitrera seul entre corriger la porte, affaiblir
l'assertion et escalader — et le troisième terme est le seul autorisé.

**Disposition, pour les quatre cas et pour le contrôle négatif :
halt-and-surface.** Un détecteur qui rougit sur le code existant arrête
l'implémentation. La mesure est consignée dans mika#2310 (commentaire : le test,
la sortie, la ligne du prédicat en cause) et remontée à l'opérateur. Ni la
correction de la porte ni l'affaiblissement de l'assertion ne sont autorisés dans
ce ticket.

Cas par cas, pour que la disposition ne se lise pas comme une formule :

- **Cas 4 rouge** — le prédicat aurait perdu son filtre
  `child.status IN ('completed','delivered')` (`db.rs:9552`). Ce serait la
  réfutation directe des essais 3/4 de mika#2288 : un groom mort compterait comme
  une preuve. Halt. La correction est un ticket de porte, pas une ligne de ce
  harnais.
- **Cas 8 rouge** (`Ok(false)` là où `Err` est attendu) — le fail-closed serait
  inversé : une base illisible rendrait « ce ticket n'a pas été groomé » au lieu
  de « je n'ai pas pu lire ma preuve », et l'opérateur irait regroomer un ticket
  dont la base est en panne. Halt. Contredit la condition 2 du GO mika#2287
  (« aucun chemin d'exception ne rend `true` par défaut ») par son autre versant :
  aucun chemin d'exception ne doit rendre un verdict de contenu.
- **Cas 9 rouge** — le chemin de dispatch refuserait un ticket réellement groomé.
  Halt : c'est la porte qui ferme sur du légitime, exactement ce que les essais
  live cherchaient.
- **Cas 9b rouge** — le chemin de dispatch accepterait sans preuve. Halt, et
  c'est la forme la plus grave : la porte serait ouverte.
- **Contrôle négatif rouge à l'envers** (une mutation qui ne fait rien rougir) —
  ce n'est pas la porte qui est en cause mais le harnais : le détecteur est mort.
  Halt aussi, et la réparation est alors dans le périmètre, puisqu'elle porte sur
  le livrable de ce ticket.

**Ce qui n'est pas un halt :** un test qui ne compile pas, une fixture mal
branchée, un nom de sous-module qui ne passe pas le filtre AC5. Ce sont des
défauts du harnais en cours d'écriture, pas des mesures.

## Risques et questions ouvertes

- **R1 — L'extraction est le seul risque de production du ticket.** Un harnais qui
  casse ce qu'il mesure est le pire résultat possible. Atténuation : AC6 fait du
  caractère « déplacement de lignes » du diff un critère explicite, et les dix
  tests mika#2287 plus les scénarios de
  `test_dispatch_no_grooming_marker_guard.rs` encadrent les deux extrémités du
  segment extrait.
- **R2 — clos au grooming, plus un risque.** La question « la machine à états
  peut-elle refuser `completed → failed` ? » est tranchée par lecture :
  `update_task_status` (`db.rs:6552`) est `pub`, sans clause de garde, et
  mika#2287 l'utilise déjà pour amener un callback `completed` à `delivered`
  (db.rs:25692). Le chemin du cas 4 est celui-là, entièrement par API de prod
  (D4). Aucune écriture SQL de statut n'est autorisée par ce plan — condition 5
  du GO mika#2287. Une décision portante laissée à l'implémenteur est une
  décision non résolue (mika#1244) ; elle est résolue ici.
- **Q1 — ouverte, et laissée ouverte :** faut-il un cas `delivered` + `failed`
  simultané ? Non — les statuts sont exclusifs sur une ligne. La question ne se
  pose que si le prédicat change de forme, auquel cas elle se posera avec lui.

## Vérification

1. `cargo test -p mika-agent harnais_porte -- --nocapture` — les quatre tests
   passent, et le compte affiché est ≥ 4 (AC5).
2. `cargo test -p mika-agent` — suite complète verte, dix tests mika#2287 inclus
   (AC8).
3. `cargo clippy --all-targets` — sans avertissement nouveau.
4. `git diff` sur `src/skills/executor.rs` hors `mod tests` — relire le segment
   extrait ligne à ligne contre l'original `1735-1785` : même ordre, mêmes
   conditions, mêmes charges JSON (AC6).
5. Contrôle négatif du harnais lui-même, **conduit à la main, la mutation non
   commitée, la sortie collée** : retirer le filtre
   `AND child.status IN ('completed','delivered')` du prédicat `db.rs:9552`,
   vérifier que le cas 4 rougit ; puis rendre `evaluate_grooming_gate`
   inconditionnellement `Ok(())` et vérifier que 9b rougit. Un harnais qu'on n'a
   jamais vu échouer n'a pas encore été mesuré.

   **La sortie rouge des deux mutations va dans le corps de la PR, horodatée**
   (AC10). « Sans commiter » borne correctement la mutation ; cela ne doit pas
   emporter la preuve avec elle. La chaîne applique déjà cette forme — condition 4
   du GO mika#2287 : rouge-avant démontré, sortie collée. Un contrôle négatif
   conduit et non consigné se lit exactement comme un contrôle négatif sauté, et
   c'est le seul geste du ticket qui atteste que les détecteurs sont vivants.

6. Amendement du corps de mika#2310 (AC9) : inventaire cas-par-cas + substitution
   du cas 8, avec commentaire d'avis d'édition.

## Revision history

- **rev 2 (2026-09-15)** — révision sur findings de première passe architecte
  (`.iterate/findings-1.md`, `Disposition: ITERATE`). Les cinq findings sont
  adressés ; aucun ne reste ouvert.
  - **F1 (bloquant)** — ajout d'une section `## Fire-Disposition` : halt-and-surface
    pour les cas 4, 8, 9, 9b et pour le contrôle négatif, avec la conséquence
    nommée cas par cas et la liste de ce qui n'est *pas* un halt. Citations
    conservées : gate Fire-Disposition mika#1574, condition 2 du GO mika#2287.
  - **F2 (bloquant)** — le repli « écriture directe via `db.conn` » est supprimé de
    D4, et la construction du cas 4 est tranchée au grooming plutôt que déléguée à
    l'implémentation (mika#1244) : `completed_groom_pair` + `update_task_status`,
    intégralement par API de production. La question de la machine à états est
    close par lecture — `update_task_status` (`db.rs:6552`) n'a aucune garde de
    transition et mika#2287 s'en sert déjà pour le statut `delivered`
    (db.rs:25692). La condition 5 du GO mika#2287 (« Zéro INSERT SQL brut ») est
    désormais citée dans D4, et le halt-and-surface est nommé comme la seule
    sortie si le chemin par API venait à ne plus produire la ligne. R2 passe de
    risque ouvert à question close.
  - **F3** — AC9 : l'amendement du corps de mika#2310 (inventaire + numéros de
    ligne) devient un critère, entre au périmètre et à la Vérification (étape 6).
    Une note en § L'objet dit explicitement que le tableau est une prémisse non
    attestable par un relecteur sans shell, et où elle est recoupée.
  - **F4** — la substitution du cas 8 (« DB fermée/corrompue » → « table absente »)
    est consignée dans D4 avec la raison pour laquelle les deux autres formes ne
    sont pas constructibles, et voyage jusqu'au corps du ticket via AC9. Ajout de
    la distinction « SQL de mise en scène de la panne » vs « SQL de fabrication de
    preuve », à commenter dans le test pour qu'il ne fasse pas précédent contre la
    condition 5.
  - **F5** — AC10 : la sortie rouge des deux mutations du contrôle négatif est
    collée horodatée dans le corps de PR, la mutation restant non commitée.
    Citation conservée : condition 4 du GO mika#2287 (rouge-avant démontré).
