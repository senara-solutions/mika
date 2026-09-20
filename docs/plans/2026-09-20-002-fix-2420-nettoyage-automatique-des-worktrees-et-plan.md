# Plan — mika#2420 : les worktrees de PR mergées ne survivent plus à leur PR

- **Ticket :** senara-solutions/mika#2420
- **Type :** fix (substrat — hygiène de disque de la boucle autonome)
- **Date :** 2026-09-20
- **Branche :** `fix/2420/nettoyage-automatique-des-worktrees-et`

---

## Goal Capsule

**Objectif.** Le disque de la machine qui porte la boucle autonome cesse de se
remplir du fait de travaux terminés. Un opérateur n'a plus à purger à la main
pour qu'un build passe.

**Moyen.** Un scan périodique retire les worktrees de dispatch dont la PR est
terminale, sous conjonction de garde-fous tous fail-safe vers *conserver*
(KTD/D3).

### Ce que le plan rectifie du ticket, et c'est le premier livrable

Le ticket pose que l'art antérieur **#1694 est « fermé mais inefficace »** et
propose trois hypothèses : la logique a régressé, elle ne couvre pas les
`target/`, ou elle ne se déclenche qu'à la création du worktree. **Les trois sont
fausses, et il n'y a aucune régression à chercher.**

| affirmation du ticket | ce que le dépôt dit |
|---|---|
| #1694 est fermé | #1694 est un **dormeur**, ligne vivante de `docs/dormeurs.md` : *« dette de worktrees et de branches — audit et nettoyage automatisés »*, condition de réveil *« une branche `origin/*/1694/*` existe et porte un plan commité »* |
| sa logique a régressé | elle n'a **jamais atteint `main`** : aucun `worktree_reaper` n'existe dans l'arbre courant |
| elle ne couvre pas les `target/` | elle ne couvre rien : il n'y a pas de code à couvrir |

**#2420 est donc le réveil de #1694, pas son successeur correctif.** Sa condition
de réveil est remplie. Le contrat du registre l'écrit : *« Réveil. Quand la
condition est remplie, rouvrir le ticket GitHub cité et retirer la ligne d'ici »*
— d'où U6.

### Ce qui existe vraiment, et pourquoi son déclencheur est le défaut

Le commit `097cc66c` (2026-07-26), *« wip(mika#1694): impl staged by post-flight
recovery (mika#1282) »*, porte une implémentation réelle : `worktree_reaper.rs`
(378 lignes) et `docs/operator/worktree-hygiene.md` (145 lignes), sauvés en
`wip()` par la recovery dirty-worktree et **jamais promus**. Son architecture est
à trois couches, et son propre doc les décrit :

| couche | quoi | déclencheur |
|---|---|---|
| A — audit | visibilité en lecture seule | **opérateur, à la demande** |
| B — clean | retrait en masse des worktrees mergés/fermés | **opérateur, à la demande** |
| C — reap | retrait automatique par PR | webhook `pull_request.closed` |

Et son admission, en toutes lettres : *« Layer C prevents new debt; layers A/B
clean up whatever slips through (a webhook missed while mika-spirit was down, a
worktree created outside the loop) »*.

**Cette phrase est le défaut.** La couche C est un handler sur un **événement
unique, non rejouable**, perdable en quatre endroits déjà mesurés par la maison :
la file webhook bornée qui jette la tête de file à saturation (mika#1870), le
429, le circuit breaker du gateway qui livre en DLQ puis `dead` que seul un rejeu
manuel ressort, et le tour LLM vide dont `webhook_zero_tools` n'est opposé
qu'une fois. C'est **exactement la classe** que mika#2334 a dû fermer pour
`pull_request.opened`. Le rattrapage était délégué à A et B — qui sont manuels.

**Le geste manuel dont Vincent mesure la demi-vie de trois heures *est* la couche
B.** Ce qui manque n'est donc ni A, ni B, ni C : c'est un **rattrapage
automatique**.

### La décision centrale : scan seul, pas de hook

Le ticket propose « un reaper périodique **ou** un hook post-merge ». Tranché :
**scan périodique seul**, pour quatre raisons de poids décroissant.

1. **Le scan couvre la population déjà accumulée** — les onze worktrees mesurés.
   Un hook ne rattrape jamais ce qu'il a raté ; c'est la propriété qui a mis
   #1694 en échec.
2. **La latence du hook est négligeable** devant le rythme mesuré : le disque
   monte d'environ 8 %/h, un tick de dix minutes coûte ~1,3 % de disque.
3. **Un seul mécanisme est un seul endroit à déboguer.** Livrer un chemin *et*
   son filet quand le filet couvre tout le domaine du chemin est du YAGNI.
4. **Doctrine maison** : *un filet, pas un chemin* (mika#2334). Un hook resterait
   possible plus tard **par-dessus** ce scan sans rien invalider — l'ordre n'est
   pas contraint dans ce sens, et il l'est dans l'autre : livrer le hook d'abord
   laisserait la dette accumulée intacte.

Le code de #1694 reste largement réutilisable — parsing de
`git worktree list --porcelain`, garde de chemin sur `/.claude/worktrees/`, refus
du dirty, no-op en production conteneurisée. Ce qui change est son déclencheur.
Il ne mesurait **ni** l'espace récupéré (garde-fou 3 du ticket) **ni** les
processus actifs (garde-fou 1).

### Condition d'arrêt

Une suppression fautive est irréversible. Toute ambiguïté s'arrête sur
*conserver*, et l'opérateur dispose d'un STOP à chaud sans redémarrage (D6).

---

## Product Contract

### Symptôme observable aujourd'hui

Le 2026-09-20, trois vagues de purge manuelle dans la même journée :

| heure (UTC) | population purgée | espace |
|---|---|---|
| ~08:35 | `fix-1951` (PR #2411, mergée le 19/09), `fix-1952` (PR #2412, mergée le 19/09) | ~67 Go |
| ~09:xx | onze worktrees de PR mergées/fermées | 25→14 worktrees, `/data` 77 %→55 % |
| 12:08 | `feat-1883`, `fix-2413`, `fix-1925` | ~93 Go |

Entre deux vagues, `/data` remonte à 80-82 % en environ trois heures — chaque
build telemetry pèse 25 à 44 Go et deux pilotes tournent. Au-delà de 85 %, les
builds s'arrêtent : la boucle entière se bloque.

### Comportement visé

Un worktree dont la PR est mergée ou fermée disparaît sans geste humain, dans le
tick qui suit sa fenêtre de grâce. Un worktree dont la PR est ouverte, ou sans PR
connue, ou portant du travail non livré, ou avec un processus dedans, n'est
**jamais** touché.

### Requirements

**Retrait**

- R1. Un worktree sous `.claude/worktrees/` dont toutes les PR connues pour sa
  branche sont `MERGED` ou `CLOSED` est retiré, avec son `target/`.
- R2. Le retrait libère aussi le répertoire parent `<slug>/` s'il devient vide,
  et l'entrée du registre git.

**Garde-fous** (garde-fous 1 et 2 du ticket)

- R3. Un worktree dont au moins une PR est `OPEN` est conservé.
- R4. Un worktree sans PR résolvable est conservé — cela inclut le cas « groomé,
  pas encore implémenté » que le ticket nomme.
- R5. Un worktree contenant le répertoire de travail courant d'un processus
  vivant est conservé, même si sa PR est mergée.
- R6. Un worktree portant du travail non livré — modifications non committées ou
  commits absents de `origin/<branche>` — est conservé et **compté**.
- R7. Toute information illisible, absente ou ambiguë conserve le worktree. Il
  n'existe aucune exception à cette règle.

**Observabilité** (garde-fou 3 du ticket)

- R8. Chaque retrait écrit une ligne d'audit portant le chemin, la branche, le
  numéro de PR et l'espace récupéré.
- R9. Chaque refus est attribuable à un motif nommé, dédupliqué, interrogeable en
  SQL.
- R10. L'inaction du scan pour cause d'environnement — checkout absent — est
  **dite**, jamais silencieuse.

**Réversibilité**

- R11. L'opérateur peut arrêter le scan à chaud, sans redémarrer mika-spirit.
- R12. Le scan peut tourner en observation : il mesure et journalise sans rien
  supprimer.

### Périmètre

| | dans le périmètre | hors périmètre |
|---|---|---|
| Worktrees sous `.claude/worktrees/` | ✅ | |
| Checkout primaire, worktree créé à la main hors de cette racine | | ❌ jamais touché (R7) |
| `target/` d'une PR **ouverte** | | ❌ travail vivant — voir angle mort (d) |
| Branches distantes, branches locales orphelines | | ⚠️ suppression locale best-effort seulement (U3) |
| Couches A et B de #1694 (`worktrees-audit` / `worktrees-clean`, dépôt mika-platform) | | ❌ inchangées, elles restent le geste manuel |
| Cause du volume de `target/` (absence de `CARGO_TARGET_DIR` partagé) | | ❌ ticket de suivi (angle mort (d)) |

---

## Planning Contract

### D1 — L'asymétrie décide tout, et elle s'écrit avant le reste

Ce scan supprime. Les deux erreurs n'ont pas le même prix :

| erreur | coût | réversible ? |
|---|---|---|
| faux négatif (on garde un worktree mort) | quelques dizaines de Go pendant dix minutes | oui — rattrapé au tick suivant |
| faux positif (on supprime un worktree vivant) | des heures de travail détruites dans un worktree de décision | **non** |

**Conséquence : tous les termes du prédicat sont fail-safe vers *conserver*, sans
exception (R7).** C'est la direction **inverse** du fail-closed de `wip_rescue`
(mika#2199), et l'inversion est raisonnée : là-bas un signal illisible devait
exclure la PR parce qu'un rejeu coûtait toute la file ; ici un signal illisible
doit conserver parce qu'un retrait fautif ne se répare pas. **Cet arbitrage est
local ; il ne se transporte pas.**

### D2 — Un scan périodique, cinquième de sa famille, calqué sur `qa_review_reconcile`

Le lieu est `crates/mika-agent/src/worktree_reaper.rs` — **pas** sous `server/`,
puisque ce n'est plus un handler webhook. Le scan est porté par mika-dev, hors
LLM et hors session pilote, exactement comme `qa_review_reconcile` (mika#2334).

Ce qui est repris du patron, terme à terme :

- une **fonction pure** `select_worktrees_to_reap`, testable sans réseau ni
  système de fichiers, dont la conjonction des termes est documentée en tableau ;
- une struct de configuration avec `Default`, et le **parse à trois paliers** :
  absent ou vide → défaut ; illisible, `0` ou négatif → défaut avec un `warn!`
  nommant la valeur. Le `0` ne désarme pas — c'est le rôle du kill-switch, et
  l'inverse ferait d'une coquille un désarmement silencieux ;
- un `gh pr list` **par dépôt et par tick**, coût API constant ;
- la **troncature du cap chez l'appelant, après filtrage** (leçon mika#2347 : un
  cap posé en amont plafonne les *sauts* au lieu des *écritures*, et des
  candidats refusés consommeraient le tick à la place de candidats traitables) ;
- la résolution de token par `resolve_periodic_scan_token` (mika#2205), **jamais**
  `self.github_token`. Lire l'état d'une PR n'est pas une opération dont GitHub
  lit l'auteur au sens d'ADR-008, donc le repli App est légitime, au même titre
  que la bascule de label d'`auto_pull`.

### D3 — Le prédicat : conjonction de sept termes

| # | terme | source de vérité | lecture illisible ⇒ |
|---|---|---|---|
| T1 | le chemin contient `/.claude/worktrees/` | le chemin lui-même | conserver |
| T2 | le worktree a une branche attachée | `git worktree list --porcelain` | conserver |
| T3 | au moins une PR connue pour cette branche | `gh pr list --state all` | conserver (R4) |
| T4 | **aucune** PR ouverte pour cette branche | idem | conserver (R3) |
| T5 | la PR la plus récemment close l'est depuis plus que la grâce | `closedAt` | conserver |
| T6 | aucun processus vivant n'a son cwd sous le worktree | `/proc/*/cwd` | conserver (R5) |
| T7 | rien de non livré : ni dirty, ni commits hors de `origin/<branche>` | `git status --porcelain`, `git rev-list origin/<b>..HEAD` | conserver (R6) |

**T2 lit le registre git, jamais une dérivation de chemin.** Le registre est la
vérité terrain et `CLAUDE.md` pose la règle : *« the worktree path is declared,
never derived »* — re-dériver est la duplication que mika-platform#58 a fermée.
Un worktree en `detached HEAD` n'a pas de ligne `branch` : il sort de la
population.

**T4 est formulé en négatif à dessein.** Deux PR peuvent partager une même
`headRefName` (une fermée, une rouverte). « Il existe une PR mergée » serait vrai
dans ce cas et conduirait à supprimer un worktree dont une PR est ouverte.
« Aucune PR n'est ouverte » est le prédicat correct.

**T6 remplace le `pgrep` du ticket, et le remplace par mieux.** `pgrep` matche un
nom de commande, pas une localisation : un `cargo` appartenant à un *autre*
worktree le satisferait. `/proc/<pid>/cwd` répond à la question réellement posée.

### D4 — Ce que T6 ne couvre pas, nommé plutôt que masqué

Un processus peut travailler dans un worktree **sans y avoir son cwd** :
`git -C <chemin>`, `cargo --manifest-path`, un éditeur lancé d'ailleurs. T6 seul
ne les voit pas.

Ce qui les couvre est la **conjonction** : un tel processus travaille sur une
branche dont la PR est ouverte (exclu par T4), ou produit des modifications non
committées (exclu par T7). Le terme T6 est une garde supplémentaire, jamais la
garde unique — et le plan ne prétend pas le contraire.

**Granularité du fail-safe de T6, qui est un piège.** Le fail-safe porte sur
l'**énumération globale** : si `/proc` est illisible en entier, le scan ne peut
rien affirmer et conserve tout. Il ne porte **pas** sur un `readlink` individuel
refusé — sur une machine il y a toujours des processus d'autres utilisateurs, et
conserver dès le premier `EACCES` rendrait le scan définitivement inerte, ce qui
est un désarmement déguisé en prudence.

### D5 — Le walker de taille est distinct de celui qui existe, et c'est essentiel

`task_engine/worktree_activity.rs` exclut `target/` de sa marche, et cette
exclusion est **porteuse pour son propre prédicat** : un `cargo build` en arrière-
plan ne doit pas faire paraître vivant un pilote coincé.

**Ici l'exigence est exactement inverse : `target/` *est* la mesure** — c'est le
consommateur de 25 à 44 Go que le ticket veut voir. Réutiliser ce walker
rapporterait quelques mégaoctets là où il y en a trente-quatre gigaoctets, c'est-
à-dire un chiffre faux avec l'autorité d'une mesure.

D'où un walker dédié, avec ses propres bornes et son propre budget de temps. Ses
règles :

- `bytes_reclaimed` est un `Option<u64>` : **`null` n'est jamais `0`** (doctrine
  mika#2331 — un zéro serait un mensonge lisible) ;
- la mesure est **best-effort** : son échec ou son dépassement de budget
  n'empêche pas le retrait, il rend `None` ;
- une marche tronquée rend un **minorant explicitement étiqueté**, jamais une
  mesure.

**Alternative refusée :** lire l'espace libre du point de montage avant et après.
Moins chère, mais elle attribuerait à ce retrait l'activité concurrente de deux
pilotes qui buildent — un chiffre bruité présenté comme exact.

### D6 — Un STOP à chaud, et c'est ce scan qui justifie enfin l'extension

`auto_pull_stop::is_stopped(global_home, scan)` est **déjà paramétré par nom de
scan**, et mika#2329 a écrit que l'extension « est une ligne chacun » tout en la
refusant faute de besoin mesuré : *« arrêter la revue QA n'est pas la même
décision qu'arrêter le feeder »*.

**Une opération destructive est précisément ce besoin.** Pendant un incident, on
veut arrêter un reaper qui supprime sans redémarrer mika-spirit — le redémarrage
étant ce qu'on veut le moins faire avec des dispatches en vol. Le scan prend donc
le STOP, sous `~/.mika/state/worktree-reap-stop`, avec la même sémantique :
l'existence vaut STOP, le contenu n'est jamais lu.

### D7 — Trois leviers, trois portées, et aucun n'est redondant

| levier | effet | quand |
|---|---|---|
| `MIKA_WORKTREE_REAP=0` | **annule la row récurrente** | au démarrage, désactivation durable |
| `MIKA_WORKTREE_REAP_DISPOSITION=observe` | le scan mesure et journalise, **ne supprime rien** | validation d'une nouvelle machine |
| `~/.mika/state/worktree-reap-stop` | court-circuite le tick | **pendant un incident, à chaud** |

Le kill-switch **annule** la row plutôt que de sauter son enregistrement : sinon
une row posée par un démarrage antérieur survivrait au désarmement — la forme
exacte du défaut que mika#2271 a dû réparer.

La disposition suit le patron de mika#2249 : *la détection est inconditionnelle,
seule la disposition est gardée*. En observation, les lignes d'audit sont écrites
avec `disposition: "observe"`, ce qui donne à l'opérateur la population exacte
qui *serait* supprimée, avant de l'armer.

### D8 — Livré armé, et l'argument est mesuré

Le réflexe serait de livrer désarmé « par prudence ». **Refusé**, sur le
précédent mika#2272 : mika#2249 avait livré derrière une condition d'armement —
trois rows relues sans faux positif — qui s'est révélée **insatisfaisable, pas
seulement non satisfaite**, parce que la population scannée était vide par
construction. *« Zéro était l'absence de mesure, pas la présence de caution. »*

Ici le défaut est p1, la purge manuelle a une demi-vie de trois heures, et livrer
désarmé ne ferme rien. Ce qui paie la prudence est ailleurs, et c'est concret :
les sept termes fail-safe (D1/D3), le mode observation (D7), le STOP à chaud
(D6), et un **contrôle négatif** obligatoire en test (V2).

Une différence de fond avec mika#2249 justifie l'écart : ce prédicat repose sur
l'**état externe et vérifiable d'une PR**, pas sur l'inférence d'un silence.

### D9 — Pas de ledger anti-rejeu, et il faut dire pourquoi

`qa_review_reconcile` a dû se doter d'un ledger relu (mika#2347) parce que ses
termes d'idempotence étaient des états GitHub *qui n'existent qu'après
aboutissement* : une revue qui mourait sans rien poster remettait la PR dans la
population, à l'identique, jusqu'à 96 fois par jour.

**Ce scan n'a pas ce défaut : son action est idempotente par construction.** Un
worktree retiré n'apparaît plus dans `git worktree list` ; il sort de la
population de lui-même.

Les **refus**, eux, se répètent à chaque tick — un worktree dirty de PR mergée
produirait une ligne toutes les dix minutes. D'où la déduplication par
`(worktree, motif)` avec horizon de 24 h, doctrine mika#2131 : l'information
durable est « ce worktree est tenu par ce motif », pas « il l'était encore à
14 h 32 ». La vivacité est le rôle de l'agrégat par tick.

### D10 — Les motifs de refus sont un format de fil

Ils atterrissent dans `audit_events.after_value` et l'opérateur en fait des
`GROUP BY` : deux orthographes d'un même motif couperaient une population en deux
sans le dire.

| motif | ce qu'il signifie | régime attendu |
|---|---|---|
| `pr_open` | au moins une PR ouverte (T4) | fréquent — cas nominal |
| `pr_unknown` | aucune PR résolvable (T3) | fréquent — groomé non implémenté |
| `too_young` | PR close depuis moins que la grâce (T5) | transitoire, se résout seul |
| `live_process` | un processus a son cwd dedans (T6) | rare |
| `dirty` | modifications non committées (T7) | **doit rester rare — voir HALTE 3** |
| `unpushed_commits` | commits absents d'`origin` (T7) | **doit rester rare — voir HALTE 3** |
| `detached_head` | pas de branche attachée (T2) | rare |
| `outside_managed_root` | chemin hors de `.claude/worktrees/` (T1) | **doit rester vide** |

Constantes d'un seul lieu, épinglées par test. `pr_open` et `pr_unknown` sont
délibérément distincts alors qu'ils mènent au même verdict : le premier est un
travail vivant, le second un travail en attente, et les confondre effacerait la
mesure de deux populations différentes.

### Alternatives examinées et refusées

| piste | pourquoi refusée |
|---|---|
| Promouvoir le handler webhook de #1694 tel quel | Événement unique non rejouable (Goal Capsule) ; ne rattrape pas les onze worktrees déjà là. |
| Hook + scan, les deux | Le scan couvre entièrement le domaine du hook. Deux mécanismes pour une population, c'est deux endroits à déboguer et une attribution ambiguë quand ça rate. |
| Un `cron` système ou un `Makefile` appelé par l'opérateur | C'est la couche B de #1694, déjà là, et c'est *elle* dont on mesure la demi-vie de trois heures. |
| Un pas ajouté au prompt d'une skill | `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — mesuré neuf récurrences sous prompt contre zéro écrit à la main (mika#2120). |
| Supprimer seulement `target/` et garder le worktree | Laisse la dette de registre git et de branches que #1694 visait, et un `target/` se reconstitue au prochain build dans un worktree que plus rien n'utilise. |
| Se fier à la suppression automatique de la branche distante | Non garantie par la configuration du dépôt ; un signal qu'on ne contrôle pas ne peut pas décider d'une suppression irréversible. |
| Réutiliser le walker de `worktree_activity.rs` | D5 — son exclusion de `target/` est porteuse pour lui et disqualifiante ici. |
| Livrer désarmé | D8 — précédent mika#2272. |

---

## Implementation Units

### U1 — Le module et la décision pure

**Fichier :** `crates/mika-agent/src/worktree_reaper.rs` (nouveau).

Contient la fonction pure `select_worktrees_to_reap`, la struct de configuration,
les constantes de motifs (D10) et la constante `SOLE WRITER` du `tool_name`
d'audit. Aucune E/S : elle reçoit un instantané déjà collecté — entrées du
registre, instantanés de PR, ensemble des cwd vivants, état de propreté — et rend
les candidats plus les refus motivés.

Les sept termes de D3 sont documentés en tableau au doc-comment, chacun avec sa
direction de fail-safe. L'ordre d'évaluation va du moins cher au plus cher : T1 et
T2 sont des lectures de chaîne, T7 coûte deux sous-processus git.

Le doc-comment porte l'invariant en une phrase : *un worktree dont un seul terme
est illisible est conservé ; il n'existe aucune exception.*

**Configuration** (trois paliers de parse) :

| variable | défaut | justification du défaut |
|---|---|---|
| `MIKA_WORKTREE_REAP` | `1` | D8 |
| `MIKA_WORKTREE_REAP_DISPOSITION` | `armed` | D7 ; `observe` est l'autre valeur reconnue, une valeur non reconnue reste `armed` avec un WARN la nommant entre guillemets |
| `MIKA_WORKTREE_REAP_REPO_DIRS` | le checkout `mika` de la station de dev | liste de checkouts séparés par `:` ; le `owner/repo` est dérivé de `git remote get-url origin`, ce qui évite une seconde liste à tenir synchronisée |
| `MIKA_WORKTREE_REAP_GRACE_SECS` | `900` | trois fois l'enveloppe d'un tour (300 s) — couvre un callback de merge encore en vol ; coûte ~2 % de disque à 8 %/h |
| `MIKA_WORKTREE_REAP_MAX_PER_TICK` | `3` | à dix minutes de tick, absorbe les onze worktrees mesurés en moins d'une heure tout en étalant l'I/O de suppression. En régime stationnaire (2 à 4 PR mergées/jour) le cap n'est jamais atteint |

### U2 — La collecte, et sa contrainte d'exécution

**Fichier :** `crates/mika-agent/src/worktree_reaper.rs`.

Par checkout configuré, et dans cet ordre :

1. `git worktree list --porcelain` — le registre. Les entrées marquées `prunable`
   relèvent de `git worktree prune`, pas d'un retrait.
2. Un seul `gh pr list --repo <r> --state all --json headRefName,state,number,url,closedAt`
   par dépôt, indexé par `headRefName`.
3. L'ensemble des cwd vivants, par lecture de `/proc/*/cwd` (D4).
4. Pour les seuls candidats survivant à T1-T6 : `git status --porcelain` et
   `git rev-list --count origin/<branche>..HEAD`.

**Contrainte d'exécution à respecter, sous peine de tenir le tick du moteur.**
Tout sous-processus passe par `tokio::process::Command` — le patron de `run_git`
dans le code de #1694 — et la marche de taille porte un budget de temps strict.
Un `rm -rf` de 34 Go est une tempête d'I/O ; c'est le cap par tick qui la borne,
et c'est pourquoi ce cap n'est pas cosmétique.

`GH_TIMEOUT` de 30 s, aligné sur `wip_rescue` et `qa_review_reconcile`.

### U3 — La disposition

**Fichier :** `crates/mika-agent/src/worktree_reaper.rs`.

Pour chaque candidat retenu, dans l'ordre : mesurer la taille (D5), puis
`git worktree remove --force`, puis retirer le répertoire parent `<slug>/`
**s'il est vide**, puis `git worktree prune`.

La suppression de la branche locale (`git branch -D`) est **best-effort et
journalisée** : le worktree est parti de toute façon, et son échec ne doit pas
faire échouer le retrait. C'est ce que faisait #1694 et il n'y a pas de raison de
changer.

En disposition `observe`, tout est mesuré et journalisé, **rien n'est supprimé**.

**Point de rupture le plus probable de cette unité :** le retrait du parent
`<slug>/`. `dispatch-lib.sh` le gère déjà de son côté (via
`derive-worktree-path --no-repo`), et un worktree multi-dépôt peut partager un
parent avec un autre dépôt encore vivant. **Ne retirer le parent que s'il est
vide**, jamais récursivement — et le tester en premier.

### U4 — Le branchement dans la famille des scans périodiques

**Fichiers :** `crates/mika-agent/src/task_engine/dispatcher.rs`,
`crates/mika-agent/src/server/mod.rs`, `crates/mika-agent/src/lib.rs`.

1. Une variante `WorktreeReap` sur `enum PeriodicScan`, avec son
   `no_token_event()` (`worktree_reap_no_token`) et son `idle_consequence()`.
   **`ALL_PERIODIC_SCANS` et son assertion de longueur doivent être mis à jour** —
   `mika2334_every_scan_variant_is_covered` casse la compilation sinon, et c'est
   son rôle.
2. Le routage par trigger `"worktree_reap"` et `dispatch_worktree_reap`, calqué
   sur `dispatch_qa_review_reconcile`.
3. `WORKTREE_REAP_CRON = "0 */10 * * * *"` — aligné sur `AUTO_PULL_CRON` et
   justifié en Goal Capsule (point 2).
4. L'enregistrement de la row avec son kill-switch qui **annule** (D7).
5. La lecture du STOP en tête de dispatch (D6), avec sa ligne INFO par tick
   court-circuité et sa transition écrite en audit.

### U5 — Surfaces opérateur

| événement | niveau | régime attendu |
|---|---|---|
| `worktree_reaped` | INFO + audit, **SOLE WRITER** | non vide au premier tick, puis quelques-uns par jour |
| `worktree_reap_tick` | INFO, agrégat | **émis seulement quand le tick agit** — zéro action, zéro ligne |
| `worktree_reap_skipped` | audit dédupliqué 24 h | porte le motif de D10 |
| `worktree_reap_failed` | WARN | doit rester vide |
| `worktree_reap_no_checkout` | WARN | R10 — **dire** le no-op, ne pas se taire |
| `worktree_reap_stop_armed` / `_lifted` | INFO | une ligne par tick court-circuité |

`worktree_reaped` porte `worktree_path`, `branch`, `pr_number`, `pr_state`,
`bytes_reclaimed` (nullable), `bytes_reclaimed_truncated`, `disposition`.
`target_key` = `worktree:<chemin>`.

**R10 est porteur et pas décoratif.** En production conteneurisée les worktrees
ne vivent pas sur le système de fichiers de l'agent : le scan n'a rien à faire,
et sans cette ligne son silence se lirait exactement comme celui d'un scan qui
tourne et ne trouve rien (classe mika#2205).

SQL de lecture, à inscrire dans le corps de PR :

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'worktree_reap_skipped' GROUP BY 1 ORDER BY 2 DESC;
SELECT target_key, created_at FROM audit_events
 WHERE tool_name = 'worktree_reaped' ORDER BY created_at DESC;
```

### U6 — Le réveil du dormeur

**Fichiers :** `docs/dormeurs.md`, `CLAUDE.md`.

Retirer la ligne #1694 du registre : sa condition de réveil est remplie et le
contrat du fichier prescrit le retrait au réveil. Documenter les cinq variables
et les surfaces dans `CLAUDE.md`, au voisinage des autres scans périodiques.

**Ne pas** rouvrir ni modifier #1694 depuis le pilote — c'est un geste
d'orchestrateur, et le corps de PR doit le signaler plutôt que le faire.

---

## Verification Contract

### V1 — Contrôle positif : les trois tests négatifs du ticket

Sur la fonction pure, avec des instantanés en fixtures :

- PR mergée, aucun processus, propre → **retiré** ;
- PR ouverte → **conservé**, motif `pr_open` ;
- `cargo` actif avec son cwd dans le worktree, PR mergée → **conservé**, motif
  `live_process`.

### V2 — Contrôle négatif, obligatoire

Chaque terme de D3, neutralisé un par un, doit faire apparaître le worktree dans
la population de retrait. **Sans ce contrôle, V1 ne prouve rien** : il passerait
aussi contre un prédicat qui ne lit rien et conserve tout.

### V3 — Fail-safe exhaustif (R7)

Pour chacun des sept termes, une entrée illisible — PR sans `headRefName`,
`closedAt` absent, `git status` en échec, `/proc` illisible, `detached HEAD` —
rend `conserver`. Un test par terme, avec son motif attendu.

### V4 — Les gardes structurelles

- Aucun chemin hors de `.claude/worktrees/` n'atteint la disposition, y compris
  par un lien symbolique ou un `..` dans le chemin du registre.
- Les motifs de D10 ont un seul lieu de définition (scan de source, format de fil).
- `ALL_PERIODIC_SCANS` couvre `WorktreeReap` (le test maison le force).
- `mika2205_periodic_scans_do_not_read_the_pat_field_directly` couvre la nouvelle
  fonction de dispatch.

### V5 — Le cap est un cap sur les écritures

Avec un cap de 1 et deux candidats dont le premier est refusé, le second **doit**
être traité dans le même tick. C'est la leçon mika#2347 transposée : un cap
appliqué avant le filtre plafonnerait les sauts.

### V6 — Ce qui n'est pas testable ici, et pourquoi

**La session pilote tourne en bwrap, où `/data` est un tmpfs de 31 Go ne montrant
qu'un seul worktree** — vérifié pendant ce grooming. Aucun test d'intégration sur
l'arbre réel n'est possible depuis un pilote, et le scan lui-même ne tourne que
dans mika-spirit côté hôte.

**Conséquence sur la stratégie de test : fonction pure plus fixtures, plus un
harnais sur un dépôt git temporaire** pour U2/U3 (créer un vrai worktree jetable,
le retirer, vérifier le registre et le parent). Jamais d'assertion sur l'arbre de
travail réel.

### V7 — Sondes post-déploiement, et leurs haltes

**Sonde de volume**, premier tick puis 24 h : le nombre de worktrees de PR
terminale tend vers zéro et y reste. **Sonde de disque** : `/data` cesse de
franchir 80 % en régime nominal.

Commencer en `observe` pendant un tick, lire la population qui *serait*
supprimée, puis armer. C'est la vérification que le mode existe pour offrir.

**HALTE 1 — un worktree de PR ouverte, ou un worktree humain, a été retiré.**
Poser `~/.mika/state/worktree-reap-stop` **immédiatement**, puis diagnostiquer.
Un faux positif est irréversible et ne se règle pas en ajustant un seuil. Lire
`worktree_reaped` pour établir quel terme a rendu vrai ce qui ne l'était pas.

**HALTE 2 — le disque se remplit encore alors qu'aucun worktree mergé ne
subsiste.** La cause est ailleurs : c'est l'angle mort (d), le `target/` des PR
**ouvertes**. **Ne pas élargir le prédicat** — ce serait supprimer du travail
vivant pour un problème de dimensionnement. Ouvrir le ticket de suivi
(`CARGO_TARGET_DIR` partagé, ou `cargo clean` sélectif sur les worktrees
inactifs).

**HALTE 3 — `dirty` ou `unpushed_commits` dominent la distribution des motifs.**
Cette population est du travail non livré sur des PR déjà mergées, c'est-à-dire
un défaut **en amont** — la post-flight recovery mika#1282 n'a pas fait son
travail. Ce n'est pas un réglage de ce scan. Ouvrir le ticket de suivi avec la
distribution en pièce jointe ; ne pas retirer ces worktrees à l'aveugle.

**HALTE 4 — `outside_managed_root` est non vide.** Un chemin hors de la racine
gérée est arrivé jusqu'au prédicat. Désarmer et établir comment avant toute
autre chose.

---

## Definition of Done

- Les sept termes de D3 sont implémentés dans une fonction pure, chacun avec sa
  direction de fail-safe écrite au site, et l'invariant R7 au doc-comment.
- V1 à V5 passent ; V2 est vérifié rouge terme par terme, et le corps de PR le
  dit.
- Les trois leviers de D7 fonctionnent et sont documentés avec leur portée
  respective — en particulier le fait que le kill-switch **annule** la row.
- La mesure d'espace respecte `null ≠ 0` et n'empêche jamais un retrait.
- Les motifs de D10 ont un lieu unique et un test qui les épingle comme format de
  fil.
- `worktree_reap_no_checkout` est émis quand le checkout est absent (R10).
- La ligne #1694 est retirée de `docs/dormeurs.md` ; `CLAUDE.md` documente les
  cinq variables, les six événements et les deux requêtes SQL.
- Le corps de PR porte : la rectification sur #1694 (le ticket croit à une
  régression, il n'y en a pas), la décision « scan seul, pas de hook » avec ses
  quatre raisons, et les quatre haltes.
- `cargo test`, `cargo clippy`, `cargo fmt` propres.
- Aucun code expérimental abandonné ne subsiste dans le diff.

## Acceptance criteria

Transcrits des garde-fous et des tests négatifs du ticket, avec la réponse que le
grooming leur apporte.

1. **Un mécanisme retire, pour chaque worktree sous `.claude/worktrees/` dont la
   PR est MERGED ou CLOSED, le worktree — ce qui reclame aussi son `target/`.**
   → R1/R2, implémenté par U1-U3 comme **scan périodique** (cron dix minutes) et
   non comme hook post-merge, pour les quatre raisons de la Goal Capsule. Le
   `target/` part avec le worktree puisque `git worktree remove --force` supprime
   le répertoire.

2. **Garde-fou 1 — ne jamais toucher un worktree avec un processus actif
   (cargo/pilote) : vérifier `pgrep` + le cwd du process.**
   → R5, terme T6, par lecture de `/proc/<pid>/cwd`. **Le `pgrep` du ticket est
   remplacé par mieux** : il matche un nom de commande et non une localisation,
   donc un `cargo` d'un autre worktree le satisferait (D3). L'angle mort d'un
   processus sans cwd dans le worktree est nommé en D4 et couvert par la
   conjonction avec T4 et T7.

3. **Garde-fou 2 — ne jamais toucher un worktree dont la PR est OPEN, ni un
   worktree sans PR (groomé pas encore implémenté).**
   → R3/R4, termes T4 et T3. T4 est formulé en négatif (« aucune PR ouverte »)
   pour couvrir le cas de deux PR sur une même branche (D3). Les deux refus ont
   des motifs **distincts** (`pr_open`, `pr_unknown`) pour rester comptables
   séparément (D10).

4. **Garde-fou 3 — journaliser chaque retrait (worktree, PR#, espace récupéré)
   pour audit.**
   → R8, événement `worktree_reaped` (SOLE WRITER) portant chemin, branche,
   numéro et état de PR, et `bytes_reclaimed`. Cette mesure exige un walker
   **distinct** de celui qui existe, parce que celui-ci exclut `target/` et
   rapporterait donc un chiffre faux (D5). `null` signifie « non mesuré », jamais
   zéro.

5. **Test négatif — worktree PR mergée + aucun process → RETIRÉ.**
   → V1, premier cas.

6. **Test négatif — worktree PR OPEN → CONSERVÉ.**
   → V1, deuxième cas, plus V2 (le terme T4 neutralisé doit faire basculer le
   verdict, sinon le test ne prouve rien).

7. **Test négatif — worktree avec cargo actif dans son cwd → CONSERVÉ même si PR
   mergée.**
   → V1, troisième cas, plus V3 (une énumération `/proc` illisible conserve
   également).

8. **La purge cesse d'être manuelle et récurrente.**
   → V7 : sonde de volume et sonde de disque sur 24 h, avec quatre haltes dont la
   première impose de **désarmer d'abord** en cas de faux positif.

## Sources

Citations **par symbole** (doctrine mika#2397 : un fichier faux avec un symbole
juste se répare par un `grep` ; un numéro de ligne faux rend du code plausible et
sans rapport).

| Symbole / chemin | Fichier (2026-09-20) | Rôle |
|---|---|---|
| ligne `#1694` du registre | `docs/dormeurs.md` | **La rectification centrale** : #1694 n'est pas fermé, c'est un dormeur dont la condition de réveil est remplie |
| `worktree_reaper.rs` | commit `097cc66c`, chemin `crates/mika-agent/src/server/` | L'implémentation jamais mergée — réutilisable pour le parsing porcelain, la garde de chemin, le refus du dirty |
| `worktree-hygiene.md` | commit `097cc66c`, chemin `docs/operator/` | Les trois couches A/B/C et l'admission que le rattrapage est manuel |
| `select_prs_needing_review` | `qa_review_reconcile.rs` | Le patron de fonction pure et la forme de la conjonction fail-safe |
| `reconcile_qa_review_requests` | `qa_review_reconcile.rs` | Le patron d'orchestration d'un scan : un `gh` par dépôt, cap chez l'appelant |
| `PeriodicScan`, `ALL_PERIODIC_SCANS` | `task_engine/dispatcher.rs` | L'enum à étendre et le test qui force l'extension |
| `resolve_periodic_scan_token` | `task_engine/dispatcher.rs` | PAT d'abord, App en repli (mika#2205) |
| `dispatch_qa_review_reconcile` | `task_engine/dispatcher.rs` | Le calque du site de dispatch |
| `QA_REVIEW_RECONCILE_CRON`, `AUTO_PULL_CRON` | `server/mod.rs` | Cadences voisines et site d'enregistrement de la row |
| `is_stopped`, `stop_file_path`, `AUTO_PULL_SCAN` | `auto_pull_stop.rs` | Lecteur du STOP **déjà paramétré par nom de scan** (D6) |
| `WorktreeActivity`, `EXCLUDED_DIR_NAMES` | `task_engine/worktree_activity.rs` | Le walker à **ne pas** réutiliser, et pourquoi (D5) |
| `is_same_process_alive`, `read_process_start_time` | `task_engine/process_liveness.rs` | Le patron de vivacité par `/proc`, anti-réutilisation de PID |
| `_set_up_worktree`, `derive-worktree-path` | `skills/bundled/_shared/dispatch-lib.sh` | Le producteur des worktrees, et le traitement du parent `<slug>/` |
| `run_gh_subprocess` | `tools/pr_merge_with_gate.rs` | L'appel `gh` maison avec son timeout |

Tickets de lignée : mika#1694 (le dormeur réveillé), mika#1282 (post-flight
recovery — l'amont de la population `dirty`), mika#1870 (file webhook bornée —
pourquoi un événement unique se perd), mika#2131 (déduplication des refus,
agrégat par tick), mika#2205 (résolution de token, et un scan muet se lit comme
un scan oisif), mika#2249 / mika#2272 (détection inconditionnelle, disposition
gardée, et le coût d'une condition d'armement insatisfaisable), mika#2277 (une
inactivité n'est pas une mort), mika#2329 (STOP à chaud paramétré par scan),
mika#2331 (`null` n'est jamais `0`), mika#2334 (un filet, pas un chemin ; un scan
plutôt qu'un geste à la création), mika#2347 (le cap est un cap sur les
écritures), mika-platform#58 (le chemin de worktree est déclaré, jamais dérivé).
