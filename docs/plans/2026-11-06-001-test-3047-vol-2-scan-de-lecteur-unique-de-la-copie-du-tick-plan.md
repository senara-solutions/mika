# test(3047) vol 2 — la copie du tick n'a qu'un lecteur dans la boucle de dispatch de la phase 2

> **Ticket :** senara-solutions/mika#3047 — `bug`, `p1-important`, `dispatch:loop`,
> `loop-substrate`, `ready`.
> **Vol 2 de 2.** Le vol 1 est mergé : `3cafaa9d2` (PR #3048, `Refs #3047`), qui a
> fermé AC1, AC2 et la **première** phrase d'AC3. Ce vol ferme la **seconde**
> phrase d'AC3 — *« un test garde qu'aucun chemin de dispatch de la phase 2
> n'utilise plus la copie du tick »* — explicitement renvoyée ici par le § 6 du
> plan du vol 1, avec sa ligne `Tracked in:`.
> **Plan du vol 1 :** `docs/plans/2026-11-05-006-fix-3047-phase-2-relit-le-ticket-avant-dispatch-plan.md`.

---

## 0. Ce qui reste du ticket, et ce que ce vol peut en fermer

Trois des quatre critères sont déjà atterris. Le reste n'est pas un reliquat
cosmétique : c'est la moitié de la garantie qu'**aucune assertion
comportementale ne peut voir**.

| critère | état après le vol 1 | ce vol |
|---|---|---|
| AC1 — relecture à la forge avant dispatch | fermé (`gh_reread_issue_at_dispatch`, l. 5698) | — |
| AC2 — fail-closed, trois filtres nommés, quatre tests | fermé (T1–T4, l. 12323 ss.) | — |
| AC3 phrase 1 — le fetcher rend la lecture fraîche | fermé (`&fresh` au site, l. 6346 ; discriminant comportemental) | — |
| **AC3 phrase 2 — un test garde qu'aucun chemin de dispatch n'utilise plus la copie du tick** | **dû** | **ce vol** |
| AC4 — sonde sur 14 jours | **dû, et non livrable par du code** | § 7 |

**Pourquoi la phrase 2 n'est pas redondante avec le discriminant du vol 1.** Le
discriminant comportemental (`UNGROOMED_BODY` dans la liste du tick,
`GROOMED_BODY` dans la relecture cannée, observable = la classe préparée) prouve
que le handler reçoit aujourd'hui la lecture fraîche. Il ne voit **rien** d'un
second chemin : une ligne ajoutée plus bas dans la boucle de secours qui relirait
`issue.body` ou `issue.labels` — pour une sonde, pour un log, pour un gate neuf —
laisserait ce test vert et réintroduirait la décision sur une copie de plusieurs
secondes. C'est très exactement la classe `grooming_marker` (mika#2158) : une
seconde lecture ne rend aucune décision fausse le jour où elle est écrite.

Et le vol 1 a rendu cette seconde lecture **plus** tentante, pas moins : la
variable `issue` reste liée dans le corps de la boucle après la relecture, à
portée, nommée comme ce qu'on veut lire. Le vol 1 l'a laissée là délibérément —
le gate de staleness en a besoin, et il mesure une branche, pas un consentement.
Ce vol est la garde qui fait de toute lecture ultérieure un geste visible.

### Gel des tickets (jusqu'au 01/12)

Ce vol n'ouvre aucun ticket de suivi. Le reste — le relevé AC4 — va en
commentaire sur mika#3047 (§ 7), et **ce vol ne porte donc pas `Closes`** mais
`Refs #3047` : le ticket se ferme sur le relevé, pas sur ce code.

---

## 1. Le site, sur `3cafaa9d2`

`crates/mika-agent/src/auto_pull.rs`, dans `phase2_reconcile_stuck_ready`
(l. 5830). La fonction porte **deux** boucles, et c'est le fait structurel qui
décide de tout ce qui suit.

| lieu | lignes | ce qui s'y lit | périmètre d'AC3 ? |
|---|---|---|---|
| boucle de **filtrage** `for issue in &ready` | 5877 – 6120 | la copie du tick, massivement : `classify_stuck_ready_in_memory(issue)` (5882), `classify_stuck_ready(issue, …)` (6010) | **non** — ce sont les filtres 2–8, pas un dispatch |
| sélection pure `select_stuck_ready_candidates` | 6123 | prend `issues` en entier | non |
| boucle de **secours** `for &n in &selected` | 6179 – 6462 | la liaison l. 6251, lue une fois l. 6263 | **oui** — c'est le seul chemin de dispatch |

Dans la boucle de secours, l'état est aujourd'hui exactement celui qu'AC3 décrit :

| l. | forme | lecture ? |
|---|---|---|
| 6251 | `let Some(issue) = issues.iter().find(|i| i.number == n) else {` | la **liaison** de la copie du tick |
| 6263 | `issue,` — argument de `promotion_gate_allows` | **la seule lecture**, et elle précède la relecture |
| 6304 | `gh_reread_issue_at_dispatch(github_token, n)` → `fresh` | la relecture d'AC1 |
| 6346 | `dispatch_rescued_ticket_in_process(db, ctx, &fresh, …)` | le dispatch, sur la lecture fraîche |
| 6362 | `let issue_url = format!(…)` | pas la liaison (`issue_url` est un autre identifiant) |
| ~20 sites | `issue = n` dans `warn!` / `info!` | pas la liaison (champ `tracing`, valeur `n`) |

**Zéro violation existante, et c'est mesuré ici plutôt que supposé** (cf. § 10).

---

## 2. La propriété épinglée, et pourquoi elle ne peut pas l'être par un nom

Le § 4 du plan du vol 1 a tranché une question que ce vol hérite : *« ce scan
garde un **ordre**, jamais le nom d'une variable locale, et une aiguille qui
épingle ce nom est stale au premier renommage sans que la propriété ait bougé »*.
C'est pourquoi il a **resserré** l'aiguille de mika#2470 de
`dispatch_rescued_ticket_in_process(db, ctx, issue` à
`dispatch_rescued_ticket_in_process(db, ctx, `.

Une garde qui chercherait le token `issue` en dur dans la boucle de secours
rouvrirait cette fragilité, un cran plus bas. Elle serait verte le jour où
quelqu'un renomme la liaison en `tick_copy` — et c'est ce jour-là, celui d'une
refonte du site, que la garde a sa seule valeur.

**La propriété est donc formulée sans nom.** Dans la boucle de secours :

> La liaison produite par `issues.iter().find(…)` — quel que soit son nom — est
> lue **exactement une fois**, et cette lecture est l'argument de
> `promotion_gate_allows`, qui précède `gh_reread_issue_at_dispatch`.

Le nom est **lu dans la source** par le prédicat, extrait du motif de liaison
(§ 4.2), jamais écrit dans le test. Un renommage change le nom que le scan
cherche, pas le verdict qu'il rend.

**Pourquoi « exactement une » et non « au plus une ».** Zéro lecture n'est pas un
arbre propre : c'est le gate de staleness débranché, ou la liaison devenue morte
et le scan devenu orphelin — un correctif désarmé qu'un `<= 1` lit comme une
réussite. Le vol 2 de mika#3044 a gravé cette distinction dans sa propre table
d'assertions (`parent_rehabilitation.rs`, T8) ; c'est le même piège, sur un
autre objet.

---

## 3. La fenêtre, et pourquoi elle n'est pas le corps de la fonction

Un scan posé sur le corps entier de `phase2_reconcile_stuck_ready` serait
**rouge par construction** : la boucle de filtrage lit la copie du tick une
dizaine de fois, légitimement. La réaction naturelle serait d'allowlister ces
sites — et une allowlist née pleine est exactement ce que mika#2309 condamne,
parce que sa taille ne dit plus rien et que la vacuité cesse d'être assertable.

La fenêtre est donc **la boucle de secours seule**, bornée par deux ancres
structurelles plutôt que par des numéros de ligne :

| borne | ancre | pourquoi celle-là |
|---|---|---|
| début | `for &n in &selected {` | nomme la **sortie de la sélection pure**, pas une variable de commodité. `for &n in &survivors` (l. 6132) est l'autre boucle `for &n`, et elle est avant : l'ancre complète les distingue |
| fin | la fin du corps de la fonction | le dernier `rescued += 1;` de la boucle est son unique sortie nominale |

**Pourquoi le corps vient de `production_half` et non d'un `find` nu.** Le motif
`async fn phase2_reconcile_stuck_ready(` apparaît **deux** fois dans le fichier :
la définition (l. 5830) et la chaîne littérale du scan mika#2470 (l. 8452). Un
`find` rend aujourd'hui la bonne — la première — mais par accident d'ordre. Le
corps est donc découpé dans `crate::source_scan::production_half(src)`, qui
coupe au `\n#[cfg(test)]\nmod tests` (l. 6469) : la fenêtre ne peut pas, même
par réordonnancement, tomber sur une fixture de test.

**Le terme de non-vacuité**, qui est ce qui sépare une garde d'une décoration.
Trois choses doivent exister pour que le scan vise encore quelque chose, et
chacune est assertée séparément avant le compte :

1. l'ancre de début est présente **une seule fois** dans la moitié de production ;
2. la fenêtre contient le motif de liaison `issues.iter().find(` — sinon le site
   a été refondu et le scan ne mesure plus rien ;
3. la fenêtre contient `gh_reread_issue_at_dispatch` — l'ordre à vérifier n'a
   pas de sens sans elle, et son absence serait le vol 1 défait.

Sans ces trois termes, un renommage ou un déplacement rendrait le scan vert en
ne regardant rien : le mode de panne qu'il existe pour rendre visible.

---

## 4. Le prédicat, et la couture à desserrer d'abord

### 4.1 Pourquoi `occurs_at_boundary` monte dans `source_scan.rs`

La discrimination dont ce scan a besoin est une **frontière de token**, et le
fichier la rend indispensable plutôt que prudente :

| forme dans la fenêtre | sans frontière | avec frontière |
|---|---|---|
| `issues.iter()` | appariée (`issue` ⊂ `issues`) | écartée — `s` est un caractère de mot |
| `issue_url` (l. 6362) | appariée | écartée — `_` est un caractère de mot |
| `timelines_by_issue` (l. 6434) | appariée | écartée à gauche — `_` précède |

Le prédicat existe déjà, écrit hier par le vol 2 de mika#3044, dans
`parent_rehabilitation.rs § tests::occurs_at_boundary` — et il y est **privé à un
`mod tests`**, donc inatteignable depuis ici. La règle écrite de `source_scan.rs`
tranche ce qu'on en fait : *« un seul lecteur, plutôt qu'une copie par garde :
c'est la classe que `grooming_marker` a dû graver une fois (mika#2158, une regex
copiée dont le commentaire disait “Mirrors …” et qui a ensuite raté deux
élargissements) »*.

Il monte donc dans `source_scan.rs` en `pub(crate)`, et
`parent_rehabilitation.rs` devient son **premier** appelant plutôt que son
propriétaire. Trois propriétés de cette extraction, et chacune est une
non-régression à montrer :

- **Le corps ne change pas d'un octet.** L'extraction est un déplacement, pas une
  réécriture : si l'un des tests de mika#3044 rougit, c'est le déplacement qui
  est faux, et non le prédicat.
- **Les deux cas qui justifient ses deux frontières déménagent avec lui**, dans
  son doc-comment : la frontière de droite (`parent_rehabilitation_decided`
  contient `decide`) et celle de gauche (mika#1833, où `per_budget` et
  `total_budget` polluaient la population de `budget`). Un prédicat extrait sans
  les cas qui l'ont fait naître est un prédicat qu'on resimplifiera.
- **Il gagne son test propre dans `source_scan.rs`**, parce qu'un helper
  partagé dont la seule couverture vit chez un appelant est un helper dont le
  second appelant ne sait pas ce qu'il garantit.

### 4.2 Le nom de la liaison, lu plutôt qu'écrit

Le motif de liaison est `let Some(<nom>) = issues.iter().find(`. L'extraction
part de l'ancre de droite — `) = issues.iter().find(` — et remonte au `let Some(`
qui la précède ; `<nom>` est ce qui est entre les deux, élagué.

Deux refus explicites, parce qu'ils sont ce qui rend le scan robuste :

- **L'ancre est `issues.iter().find(`, pas `let Some(`.** `let Some(` apparaît
  plusieurs fois dans la fenêtre (l. 6133 `if let Some(&age)`) ; la liaison de la
  copie du tick est la seule à venir d'une recherche dans `issues`.
- **Si l'extraction échoue, le test échoue** — il ne retombe pas sur `"issue"`
  en dur. Un défaut nommé est une garde ; un défaut silencieux est le nom de
  variable qu'on avait refusé d'épingler, réintroduit par la porte de service.

### 4.3 Ce qui compte comme une lecture

Sur la fenêtre, **commentaires retirés** par
`crate::source_scan::strip_comment_lines` — son doc-comment porte le prédicat et
ses bornes — une occurrence du nom est une lecture quand elle est à frontière de
token des deux côtés **et** que le caractère qui la suit est l'un de `,` `)` `.`
`;`, ou un espace non suivi de `=`.

Ce que chaque clause écarte, mesuré sur la fenêtre d'aujourd'hui :

| clause | écarte | occurrences |
|---|---|---|
| frontière de token | `issues`, `issue_url`, `timelines_by_issue` | 4 |
| espace non suivi de `=` | `issue = n` — un **champ `tracing`**, dont la valeur est `n` et jamais la liaison | ~20 |
| caractère suivant dans l'ensemble clos | `issue:` de `format!("issue:{n}")` | 2 |

**La liaison elle-même n'est pas une lecture**, et c'est une exclusion par
position et non par forme : l'occurrence qui tombe dans le motif de liaison
extrait au § 4.2 est l'origine de la fenêtre de comptage, pas un de ses
habitants.

**Les deux faux positifs que ce prédicat admet encore, nommés plutôt que
tus.** `strip_comment_lines` est *à la ligne*, jamais lexical — son doc-comment le
dit : un commentaire de **fin** de ligne (`let x = 1; // …`) reste dans la
sortie, et l'intérieur d'un bloc `/* … */` dont les continuations ne commencent
pas par `*` y reste aussi. Une prose de fin de ligne écrivant `issue,` ou
`issue.` dans la fenêtre serait donc comptée. Le remède est de déplacer le
commentaire, et le message d'échec le dit : c'est un faux positif réparable, pas
un trou de détection, et le fermer demanderait un strip lexical des littéraux de
chaîne qu'aucune garde de ce dépôt ne possède — hors périmètre d'un ticket dont
la « Taille mesurée » tient en un filtre et une lecture.

---

## 5. Les trois assertions, et ce que chacune seule laisserait passer

| assertion | sans elle |
|---|---|
| chaque entrée de l'allowlist nomme un `mika#` | une exemption que personne ne possède — *how a guard stops guarding* |
| **exactement une** lecture, jamais « au plus une » | **zéro** : le gate de staleness débranché, ou la liaison morte et le scan orphelin — un correctif désarmé qu'un `<= 1` lit comme un arbre propre (§ 2) |
| cette lecture est l'argument de `promotion_gate_allows` | une lecture **déplacée** : même compte, autre consommateur. Un gate neuf lisant la copie du tick passerait, et c'est le défaut de mika#3047 sous un autre nom |
| elle **précède** `gh_reread_issue_at_dispatch` | la lecture unique glissée *après* la relecture : le gate décidant sur la copie pendant que le dispatch part sur la lecture fraîche — deux vérités dans un appel |
| les trois termes de non-vacuité du § 3 | un renommage rend le scan vert en ne regardant rien |

Les quatre premières portent sur le **même** compte ; elles ne sont pas
redondantes parce qu'un compte, un consommateur et un ordre sont trois
propriétés dont chacune survit à la violation des deux autres.

**Le message d'échec nomme la résolution, pas le symptôme** : router la lecture
par la relecture fraîche, jamais ajouter une entrée à l'allowlist (doctrine
mika#2201).

---

## 6. Le contrôle de bonne foi

Le scan pourrait être vert parce qu'il ne regarde rien — le mode de panne qu'il
existe pour rendre visible. On le montre donc **rougir** sur une source
synthétique, et **rester muet** sur les formes qui nomment la liaison sans la
lire. Le patron est écrit deux fois dans ce crate
(`mika2474_the_reader_scan_reddens_on_a_second_reader`,
`mika3044_le_scan_de_lecteur_unique_rougit_sur_un_second_lecteur`) ; ce test est
leur frère sur un troisième objet.

**Rougit — une par classe de violation du § 5 :**

1. **Une seconde lecture** ajoutée après la relecture : `let _ = sonde(issue);`.
2. **Une lecture déplacée** : le même compte, mais consommée par un autre appel
   que `promotion_gate_allows`.
3. **L'ordre inversé** : la lecture unique posée après
   `gh_reread_issue_at_dispatch`.
4. **La liaison renommée** et lue deux fois sous son nouveau nom — c'est ce qui
   prouve que le nom est lu dans la source et non écrit dans le test (§ 4.2),
   et aucune autre assertion ne peut le voir.

**Reste muet :**

5. `issues`, `issue_url`, `timelines_by_issue` — la frontière de token.
6. `issue = n` dans un `warn!` — le champ `tracing`.
7. `format!("issue:{n}")` — le littéral.
8. Une prose de doc-comment citant la liaison — la forme exacte du faux positif
   Signal S de mika#2050, et la fenêtre en porte une trentaine de lignes.

**Le cas qui manquerait si on ne l'écrivait pas :** l'extraction du nom en
échec. Une fenêtre sans motif de liaison doit faire **échouer** le test, et non
le rendre vert sur un défaut tacite (§ 4.2).

---

## 7. AC4 — ce que ce vol ne peut pas produire, et pourquoi le dire ici

AC4 demande un relevé sur **les 14 jours qui suivent le deploy**. Le vol 1 a été
mergé le 2026-11-06 à 07:34Z ; la fenêtre ne se ferme pas avant le ~2026-11-20,
et elle ne s'ouvre qu'au `make deploy` — pas au merge (mika#2340 : une absence de
ligne peut être un binaire d'avant le deploy).

**Aucune ligne de code ne peut fermer AC4.** Ce vol ne tente donc rien sur cet
axe : il n'ajoute ni compteur, ni sonde, ni test. Le relevé est un geste
d'opérateur, et le plan du vol 1 porte déjà ses trois termes (numérateur,
dénominateur, preuve que la garde a mordu) et ses deux pièges de lecture (les
deux `target_key` n'ont pas la même forme ; la fenêtre part du deploy) au § 7.

Deux conséquences pour ce vol, à ne pas inférer :

- **Il porte `Refs #3047`, pas `Closes`.** Le ticket garde AC4 ouvert, et le gel
  des tickets interdit d'en ouvrir un pour le porter : le relevé va en
  commentaire sur mika#3047.
- **Les trois filtres du vol 1 sont le dénominateur de la garde**, et leur
  compte dans `auto_pull_exclusion` est le seul terme qui distingue *« la garde
  a tenu »* de *« la garde n'a jamais été atteinte »* (doctrine mika#2331 :
  absent n'est pas zéro). Un relevé à zéro croisement s'écrit **« aucun
  croisement sur la fenêtre »**, jamais « course corrigée ».

---

## 8. Definition of Done

1. `occurs_at_boundary` vit dans `crate::source_scan`, en `pub(crate)`, avec son
   doc-comment portant les deux cas qui justifient ses deux frontières et son
   propre test ; `parent_rehabilitation.rs` l'appelle au lieu de le définir, et
   **les deux tests de mika#3044 qui en dépendent passent sans modification**.
2. Le scan de lecteur unique de la copie du tick est atterri dans la moitié de
   test d'`auto_pull.rs`, voisin de son frère
   `mika2470_direct_dispatch_precedes_the_label_churn`, avec les trois termes de
   non-vacuité du § 3 et les quatre assertions du § 5.
3. Le contrôle de bonne foi couvre les quatre formes qui rougissent et les
   quatre qui restent muettes (§ 6), **dont** le renommage de la liaison.
4. L'allowlist est livrée vide, et sa vacuité est elle-même assertée par un test
   auto-nettoyant exigeant de toute entrée future qu'elle nomme son `mika#`.
5. `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
   `cargo fmt --check`, `make verify-bundled-skills` et
   `scripts/verify-pipeline.sh origin/main` passent.
6. Le corps de PR porte `Refs #3047` et **pas** `Closes` — il nomme AC4 comme dû
   et renvoie au relevé en commentaire (§ 7). `Code review: delegated to mika-qa
   (mika#2694)`.

---

## 9. Acceptance criteria

Transcrits du corps du ticket. **Les trois premiers sont fermés par le vol 1** et
ne sont pas rouverts ici ; ce vol est jugé sur la seconde phrase d'AC3.

- **AC1.** Juste avant le dispatch en processus, la phase 2 relit l'état du
  ticket à la forge : ouvert, et portant `ready`. Si l'un des deux manque, aucun
  dispatch n'est préparé, aucune parente n'est créée, et une ligne d'audit nomme
  la raison (`ready_absent_at_dispatch`, `issue_closed_at_dispatch`).
  → *fermé par le vol 1 (`3cafaa9d2`).*
- **AC2.** Fail-closed : si la relecture échoue, la phase 2 ne dispatche pas, et
  le ledger du tick l'inscrit sous un filtre nommé. Un test par cas, dans le même
  appel : `ready` retiré, ticket fermé, relecture en erreur. Le contrôle positif
  (le ticket porte toujours `ready`) dispatche comme aujourd'hui.
  → *fermé par le vol 1 (T1–T4).*
- **AC3.** Le fetcher passé au handler rend la lecture fraîche d'AC1, et non la
  liste du tick. **Un test garde qu'aucun chemin de dispatch de la phase 2
  n'utilise plus la copie du tick.**
  → *première phrase fermée par le vol 1 (le discriminant comportemental) ;*
  **seconde phrase : ce vol** — le scan de lecteur unique des § 2 à § 6, avec
  son contrôle de bonne foi.
- **AC4.** Sonde : sur les 14 jours qui suivent le deploy, on compte les
  `stuck_ready_direct_dispatch` précédés d'un retrait de `ready` dans la minute
  sur le même ticket, puis on l'écrit ici. La cible est 0. Un compte de
  croisements nul s'écrit « aucun croisement sur la fenêtre », et non « course
  corrigée ».
  → *dû, et non livrable par du code (§ 7). Relevé en commentaire sur mika#3047
  au terme de la fenêtre (~2026-11-20).*

---

## 10. Fire-Disposition

Ce vol livre **un détecteur** — le scan structurel d'AC3 — plus deux tests qui
le gardent (son contrôle de bonne foi et la vacuité de son allowlist). Le test
d'`occurs_at_boundary` (§ 4.1) n'en est pas un : son chemin de succès est « le
prédicat rend la bonne réponse », pas « aucune violation trouvée ».

**Option retenue : (a) exception nommée en allowlist — et il n'y a aucune
exception à nommer.**

- **Zéro violation existante, mesurée avant d'écrire le code** et pas supposée.
  Le § 1 porte le relevé : dans la boucle de secours (l. 6179 – 6462) la liaison
  de la copie du tick est lue **une** fois, à la l. 6263, comme argument de
  `promotion_gate_allows`, et cette lecture précède `gh_reread_issue_at_dispatch`
  (l. 6304). C'est très exactement l'état que les quatre assertions du § 5
  affirment : le scan atterrit vert par construction. Les ~26 autres occurrences
  du token sont écartées par les trois clauses du § 4.3, chacune avec son compte.
- **Aucun mécanisme d'exception n'est livré** dans le détecteur : ni attribut de
  contournement, ni variable d'environnement de désarmement, ni exemption par
  fichier. Le seul mécanisme est la constante d'allowlist, livrée vide.
- **L'assertion auto-nettoyante est double**, et les deux moitiés sont
  nécessaires. La vacuité de l'allowlist est assertée (DoD 4), *et* toute entrée
  future doit nommer le `mika#` qui la retirera — une exemption que personne ne
  possède est la manière dont une garde cesse de garder. C'est le contrat déjà
  porté par `BRIEF_SIZE_THRESHOLD_READERS_ALLOWED` (mika#2474),
  `CONTEXT_HISTORY_READERS_ALLOWED` (mika#2425),
  `ZERO_BUDGET_READER_EXCEPTIONS` (mika#1833) et
  `REHAB_PREDICATE_READERS_ALLOWED` (mika#3044).
- **Les trois termes de non-vacuité du § 3 sont la moitié anti-stale du
  détecteur.** Une garde dont la cible a été renommée est verte sans rien
  regarder ; les trois termes la font rougir sur ce renommage au lieu de la
  laisser mourir en silence.

**Aucune option (b)** : rien n'atterrit `#[ignore]` ni `#[cfg(skip)]`. Le
détecteur est armé à l'atterrissage, sur du code déjà atterri — c'est la raison
même pour laquelle le § 6 du vol 1 l'avait renvoyé ici plutôt que de le faire
converger par itérations sur du code en train d'être écrit.

**Aucune option (c)** : rien ne remonte à l'opérateur. Le périmètre est tranché
par la « Taille mesurée » et le « Hors vol » du ticket, et le seul reste — AC4 —
est un relevé daté, pas un cadrage (§ 7).

---

## 11. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/source_scan.rs` — `occurs_at_boundary` extrait, son doc-comment (les deux cas des deux frontières) et son test | 42 |
| `crates/mika-agent/src/parent_rehabilitation.rs` — le lecteur re-pointé sur le site extrait, définition locale retirée | 8 |
| `crates/mika-agent/src/auto_pull.rs` (tests) — le prédicat : bornes de fenêtre, les trois termes de non-vacuité, extraction du nom de la liaison, clauses de lecture, doc | 62 |
| `crates/mika-agent/src/auto_pull.rs` (tests) — le scan et ses quatre assertions, avec leurs messages de résolution | 46 |
| `crates/mika-agent/src/auto_pull.rs` (tests) — le contrôle de bonne foi : quatre formes qui rougissent, quatre qui restent muettes | 54 |
| `crates/mika-agent/src/auto_pull.rs` (tests) — l'allowlist vide auto-nettoyante | 14 |

Total estimé : 226 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**226 tient sous la borne par vol, et le plan dit pourquoi — point 4 de
mika#2693.** Trois raisons, chacune mesurable plutôt que rassurante :

- **Le périmètre est un seul fichier de test plus une extraction mécanique.**
  Aucune ligne de production ne change de comportement : le détecteur observe du
  code atterri au vol 1, et l'extraction du § 4.1 est un déplacement dont le
  corps ne bouge pas d'un octet. Il n'y a pas de boucle « implémenter →
  tester → corriger le comportement » à payer en tours.
- **Le patron est écrit trois fois dans ce crate, ligne pour ligne.**
  `mika2474_the_threshold_has_a_single_reader` + son contrôle de bonne foi +
  sa vacuité d'allowlist (`canonical_tokens.rs` l. 5415-5510) donne la forme
  entière des quatre derniers postes ; `mika3044_le_predicat_de_rehabilitation_na_quun_lecteur`
  (`parent_rehabilitation.rs` l. 394-676) donne celle de la table d'assertions et
  du prédicat de frontière. Les deux sont du 2026-11-05 et du mois courant : le
  patron est vivant, pas archéologique.
- **L'estimation compte la doc AVEC le code**, ≈ 45 % du total — le ratio mesuré
  sur le vol 1 de mika#3044 (408 estimées pour ~1 123 réelles, facteur 2,75, dû
  pour l'essentiel aux doc-comments que ce dépôt exige). Le vol 1 de mika#3047 a
  tenu à 409 estimées pour 810 réelles hors plan. Un total « nu » de ~120 lignes
  serait cette erreur répétée.

Le seuil par PR (1000 lignes) n'est pas approché, et **il n'y a pas de vol 3** :
le découpage du § 6 du vol 1 était en deux vols, ce vol est le second, et le
reste du ticket n'est pas du code (§ 7).

---

## 12. Ordre de commit

Deux commits, dans cet ordre, chacun vert seul.

1. **`refactor(3047): le prédicat de frontière de token monte dans source_scan, un seul lecteur`**
   — `source_scan.rs` + `parent_rehabilitation.rs`. Vert seul par construction :
   le corps du prédicat est inchangé et son unique appelant d'aujourd'hui est
   re-pointé dans le même commit, donc les deux tests de mika#3044 qui en
   dépendent passent sans modification. Sans ce commit, le scan du suivant
   devrait recopier le prédicat — la classe mika#2158.
2. **`test(3047): la copie du tick n'a qu'un lecteur dans la boucle de dispatch de la phase 2`**
   — `auto_pull.rs` seul : le prédicat, le scan, le contrôle de bonne foi,
   l'allowlist vide. **C'est le plancher du vol** : un mika-qa qui ne lirait
   qu'un commit doit lire celui-là, parce que c'est lui qui ferme la seconde
   phrase d'AC3.

Le second commit ne touche aucune ligne de production, donc aucun ordre interne
n'est contraint au-delà de celui-là.
