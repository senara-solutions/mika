# mika#2242 — un dé-groomage est un fait estampillé par son producteur, jamais reconstruit

- **Ticket :** `mika issue#2242`
- **Type :** fix
- **Date :** 2026-09-22
- **Branche :** `fix/2242/loop-groom-fermer-une-umbrella-qui`

---

## Ce que la lecture du code déplace dans le ticket

C'est le premier livrable. Le ticket propose deux pistes ; la lecture du substrat
en réfute une, déplace l'autre, et retire au passage la partie de l'énoncé qui
n'est plus vraie. Chaque rectification change le remède, donc aucune n'est
cosmétique.

### R-a — L'option 2 (« détection au routage ») n'est pas implémentable telle qu'écrite

Le ticket propose que « le sélecteur groom-vs-impl détecte *ce ticket était
closingIssue d'une umbrella désormais fermée* ». Cette détection **depuis le
sous-ticket** n'a pas de prise :

1. **Le nom du plan umbrella ne porte aucun numéro de sous-ticket.** La
   convention canonique est `<YYYY-MM-DD>-<seq>-<type>-<issue>-<slug>-plan.md`,
   et le plan mesuré est `2026-09-07-002-fix-umbrella-auto-pull-exclusion-…` :
   le créneau `<issue>` contient le mot `umbrella`. Un plan d'umbrella couvre
   N sous-tickets et n'en nomme aucun. Donc ni `_find_issue_plan`
   (`dispatch-lib.sh:4909`, dont le palier 1 exige `*-${ISSUE_NUM}-*-plan.md`),
   ni un `git log --all -- 'docs/plans/*-2131-*'`, ne peuvent retrouver le plan
   depuis le sous-ticket.
2. **GitHub n'expose pas la direction issue → PR fermantes.** La direction
   inverse existe (`PullRequest.closingIssuesReferences`, employée par
   `wip_rescue`) ; depuis l'issue il n'y a que `timelineItems`, qui mélange
   références croisées et liens manuels. `github_graphql.rs` (832 lignes,
   10 fonctions) n'a aucun lecteur de cette direction, et en écrire un serait
   une reconstruction *a posteriori* sur un signal ambigu.

Conclusion, et c'est la charnière de tout le plan : **le lien umbrella →
sous-ticket n'est lisible qu'à un seul instant, celui de la fermeture de la PR**,
où le corps de la PR est sous la main. C'est exactement la règle que la maison a
déjà écrite deux fois : *« PR origin is a fact stamped by its producer, never
reconstructed afterwards »* (mika#2026) et *« How the engine learns where a
dispatch writes. It is told, never derived »* (mika#2249).

### R-b — L'option 1 a bien un déclencheur, déjà câblé — mais c'est un événement unique

`route_event("pull_request", Some("closed"))` rend `Some("mika-dev")`
(`crates/mika-gateway/src/github.rs:339`), et côté agent
`server::upstream_close_handler::handle_pr_closed` **existe déjà** et fait
littéralement les trois quarts du travail que l'option 1 demande :

- il distingue merged de unmerged (`MERGED_TRUE_LINE` → `UPSTREAM_PR_MERGED` vs
  `UPSTREAM_PR_CLOSED_UNMERGED`) ;
- il parse `Closes #N` / `Fixes #N` / `Resolves #N` du corps de la PR
  (`parse_closing_issue_refs`), donc **il connaît l'ensemble exact des
  sous-tickets** ;
- il résout chaque `#N` en URL d'issue.

La PR #2226 est passée par ce chemin le 2026-09-08. Il a annulé les rows de
suivi de #2131 et **n'a rien dit du dé-groomage**.

Mais `pull_request.closed` est un **événement unique non rejouable**, perdable en
quatre endroits que la maison a déjà mesurés (file bornée mika#1870 → 429 →
disjoncteur → DLQ `dead`). C'est la classe dont la couche C de #1694 est morte, et
que mika#2334 a dû refermer pour `pull_request.opened`. Donc on n'y accroche pas
une **action** : on y écrit un **enregistrement durable** que des lecteurs
ultérieurs consultent. *Un filet, pas un chemin.*

### R-c — La boucle « indéfiniment » est déjà bornée ; ce n'est plus ce qui reste ouvert

Le ticket écrit « chaque dispatch re-route vers groom au lieu d'implement,
**indéfiniment** ». C'était vrai au 2026-09-08 et ne l'est plus :

- **mika#2020** a posé `MIKA_AUTO_PULL_MAX_REDRIVES` (défaut `3`) : la phase 2
  d'`auto_pull` abandonne le ticket après trois re-drives, pose
  `operator-review`, retire `ready` et **commente le ticket**.
- **mika#2279** a posé la porte 2c : un `labeled ready` reposé pendant qu'un
  pilote tourne est un NO-OP, ce qui retire le mécanisme de supersession qui
  tuait les pilotes de grooming (les `cancelled` / `blocked/tué` mesurés).

**Ce plan ne prétend donc pas fermer une boucle infinie.** Ce qui reste
mesurablement ouvert est double, et les deux moitiés se ferment par le même
couple producteur/lecteur :

1. **Rien ne nomme la cause.** Le commentaire d'abandon dit
   `redrive_budget_exhausted`, le registre d'exclusion dit `not_groomed`. Les
   deux sont vrais et ni l'un ni l'autre n'est la cause. C'est littéralement la
   classe que le ticket revendique (« *rien ne nomme pourquoi il n'est plus
   groomé* ») — et, ironie utile, la classe que **#2131 lui-même** a fermée pour
   les exclusions d'`auto_pull`.
2. **Le travail revu est jeté.** Un plan plus deux passes architecte vivent sur
   une branche que rien ne désigne. L'opérateur l'a récupéré à la main
   (« plan réutilisé ») — une archéologie que ce plan doit rendre inutile.

### R-d — Le `prompt` du dispatch est un format strict : interdiction d'y glisser le pointeur

La tentation évidente serait de faire voyager le pointeur dans l'entrée du
dispatch. `ready_label_handler.rs:1096` l'interdit, commentaire à l'appui : le
`prompt` est le `<repo>#<num>` **nu**, parce que le parseur de mise en place de
worktree de `dispatch-lib` n'accepte que cette forme — *« an owner-qualified
prompt silently routes the dispatch into no-worktree free-text mode
(mika#1593) »*. Y ajouter une phrase ferait basculer le dispatch en mode
texte-libre sans worktree, c'est-à-dire casserait le grooming pour l'améliorer.
**Le pointeur ne voyage pas par le prompt.**

### R-e — Le producteur hérite d'un angle mort de 2000 caractères, sur la population même qu'il vise

`format_event_text` tronque le corps de PR à
`DEFAULT_GITHUB_BODY_TRUNCATION_CHARS = 2_000`
(`crates/mika-gateway/src/github.rs:275`) et appose `\n\n[truncated]`. Or
`parse_closing_issue_refs` lit **ce corps tronqué**. Un corps d'umbrella est
typiquement long : si ses lignes `Closes #N` tombent au-delà de 2000 caractères,
le handler existant n'en parse aucune et se tait en DEBUG (*« PR body carries no
Closes/Fixes issue refs »*).

C'est un angle mort **préexistant** du handler, pas une régression de ce plan —
mais il pourrait rendre le producteur **silencieusement inerte pour exactement la
population visée**, et un détecteur silencieusement inerte se lit exactement
comme un détecteur sain (mika#2205). Il est donc **nommé, instrumenté, et non
corrigé ici** : voir R6 et § Surfaces opérateur.

---

## Requirements

- **R1 — Le producteur estampille le dé-groomage à la fermeture.** Dans
  `upstream_close_handler::handle_pr_closed`, sur la branche **unmerged
  seulement**, pour chaque `Closes #N` parsé, écrire une ligne `audit_events`
  durable et une ligne INFO nommant : le sous-ticket, le numéro de la PR, son URL
  et **sa branche de tête** (celle qui porte le plan groomé).
- **R2 — Zéro changement de comportement sur le nettoyage existant.** Les
  transitions de rows de suivi (`cleanup_rows_for_issue_url`), leurs statuts,
  leurs résultats et leur événement d'audit sont inchangés au byte près. Le
  handler reste **side-effect-only** et rend toujours `Passthrough` — son
  contrat documenté.
- **R3 — Le lecteur nomme la cause au routage.** Dans `ready_label_handler`,
  quand la décision groom-vs-impl choisit `groom` (`is_groomed == false`),
  consulter le registre et, si le ticket porte un marqueur, écrire une ligne
  `audit_events` dédiée et une ligne INFO nommant la PR et la branche qui porte
  le plan antérieur.
- **R4 — Le routage ne change pas.** Un ticket dé-groomé continue de partir en
  `groom`. Aucun halt, aucune classe de dispatch modifiée, aucune porte ajoutée
  (justification en § Ce qui est refusé).
- **R5 — Fail-open sur toute lecture.** Un registre illisible, un marqueur
  absent, un `reasoning` non exploitable : le routage est identique à
  aujourd'hui. Le seul effet possible d'une défaillance de ce plan est de
  **perdre une explication**, jamais d'en fabriquer une fausse et jamais de
  changer une décision.
- **R6 — L'inertie est dite.** Une fermeture unmerged dont le corps est tronqué
  **et** dont aucun `Closes #N` n'est parsé émet une ligne nommée. C'est à la
  fois l'aveu de l'angle mort R-e et la mesure qui conditionnera son suivi.
- **R7 — Deux noms, deux populations, chacun à écrivain unique.** Les deux noms
  d'audit sont distincts et scannés `SOLE WRITER`, pour que les requêtes SQL de
  § Surfaces opérateur soient soustractibles.
- **R8 — Aucune variable d'environnement, aucun interrupteur.** Ce travail
  n'ajoute aucune décision à un opérateur : il ne fait qu'écrire ce qui était tu.
  Un kill-switch sur une attribution serait un interrupteur pour éteindre une
  explication.

---

## Conception

### Le producteur — `server/upstream_close_handler.rs`

Un bloc ajouté dans `handle_pr_closed`, **après** le parsing des refs et
**à l'intérieur** de la boucle `for number in issue_numbers`, encadré par
`if !merged`.

```text
si merged            → rien (une PR mergée ferme son issue ; il n'y a pas de dé-groomage)
si !merged           → pour chaque Closes #N :
                         audit_events row  (durable, interrogeable en SQL)
                         info!             (greppable)
                       puis le nettoyage de rows existant, inchangé
```

- **`tool_name = "closing_pr_closed_unmerged"`.** Le nom dit ce qui a été
  **mesuré**, jamais son interprétation : le producteur ne peut pas savoir si #N
  était groomé (il n'a pas son corps d'issue, et aller le chercher serait une
  lecture réseau dans un handler qui n'en fait aucune). « Dé-groomé » est une
  interprétation, et c'est le lecteur — qui a le corps — qui a le droit de
  l'écrire.
- **`target_key = "issue:{owner}/{repo}#{n}"`.** La clé que le lecteur
  interrogera, en **égalité exacte** — jamais un `LIKE`, qui ferait apparier
  `#234` sur `#2343` (le piège mika#2347).
- **`after_value = "pr#{m}"`.** La colonne de format de fil : c'est elle qu'un
  opérateur `GROUP BY`, et le regroupement utile est « quelle umbrella a
  dé-groomé quoi ».
- **`reasoning = "pr_url={} head_branch={} merged=false"`.** Un enregistrement
  `clé=valeur` séparé par des espaces — **la convention déjà en vigueur dans ce
  fichier même** (`event_type={} reference_url={}`) et dans `ready_label_handler`
  (`repo={} number={} target_skill={} groomed={}`). Ce n'est pas une invention de
  format, c'est le format de la maison.
- **La branche vient du webhook, gratuitement.** `format_event_text` écrit
  `(branch: {branch})` dans l'en-tête de tout message `pull_request`
  (`github.rs:496`), et le test existant `test_extract_pr_url_valid` en porte la
  forme. Un extracteur pur `extract_head_branch(text) -> Option<String>`
  s'ajoute à côté des trois extracteurs pareillement purs du module. Branche
  illisible ⇒ le marqueur est écrit **sans** elle : la décision du lecteur ne
  dépend pas de la branche, seul le confort de l'opérateur en dépend.
- **Ordre :** l'écriture du marqueur ne doit pas pouvoir faire échouer le
  nettoyage existant (R2). Elle est donc `fire-and-forget` avec `warn!` sur
  échec, à l'identique du `log_audit_event` déjà présent dans
  `cleanup_rows_for_issue_url`.

**R6 — la ligne d'inertie.** Là où `handle_pr_closed` fait aujourd'hui son
`debug!` « no Closes/Fixes issue refs », ajouter, **si et seulement si**
`!merged` **et** le corps est tronqué : un `warn!(event =
"closing_pr_body_truncated_no_refs", …)`. Le test de troncature est
`text.trim_end().ends_with(TRUNCATED_BODY_MARKER)` avec
`TRUNCATED_BODY_MARKER = "[truncated]"` — le corps est le **dernier** segment que
`format_event_text` appose pour un `closed`, donc le marqueur est en fin de
texte. Le couplage à la mise en forme du gateway est celui que ce module
**assume déjà** (`ISSUE_CLOSED_PREFIX`, `PR_CLOSED_PREFIX`, `MERGED_TRUE_LINE`),
et il est fail-open : si la forme change, on perd l'instrument, pas le
comportement. La constante est **déclarée** dans `scripts/canonical-tokens.tsv`,
où ce fichier a déjà deux entrées (lignes 195–196) — *on déclare, on n'allowliste
pas*.

### Le lecteur — `server/ready_label_handler.rs`

Un bloc ajouté **entre l'étape 5 et l'étape 6**, donc après
`check_grooming_markers` et avant le choix `(target_tool, target_skill,
dispatch_class)`. Il ne participe pas à ce choix.

```text
si is_groomed        → rien (contrôle négatif : le chemin nominal reste muet)
si !is_groomed       → lire le registre pour issue:{owner_repo}#{n}
                         marqueur présent   → audit row + info!  (« dé-groomé »)
                         marqueur absent    → rien (premier grooming nominal)
                         registre illisible → warn! nommé, et on continue
```

- **`tool_name = "ready_label_degroomed"`**, `target_key` identique à celui du
  producteur, `after_value = "pr#{m}"` (mêmes `GROUP BY` de part et d'autre),
  `reasoning = "head_branch={} missing_markers={} recorded_at={}"`.
- **Deux noms plutôt qu'un, et c'est ce qui rend les sondes lisibles.** Les deux
  populations sont réellement différentes : le producteur compte *toute*
  fermeture unmerged × ref fermante, y compris les anodines (une PR de brouillon
  fermée, une PR remplacée) ; le lecteur compte le sous-ensemble qui a
  **effectivement** reflué en groom en portant le marqueur, c'est-à-dire le
  défaut. Le compte du producteur doit dominer largement celui du lecteur : c'est
  le régime sain, et il est soustractible. Cinquième emploi du motif après
  `phantom_aged_out`/`phantom_sweep_spared` (mika#2156),
  `in_flight_self_dev`/`live_pilot_orphaned_parent` (mika#2279),
  `below_threshold`/`no_ready_label_event` (mika#2131) et
  `operator_review_or_blocked`/`abandoned_operator_held` (mika#2361).
- **Une lecture neuve, minimale.** `count_recent_audit_events_for_target`
  (`db.rs:1839`) existe mais ne rend qu'un compte : il permettrait de dire
  « dé-groomé » et **pas** « par la PR #2226, branche `fix/umbrella-…` » — or le
  pointeur est la moitié qui sauve le travail revu. On ajoute donc
  `Database::latest_audit_event_for_target(agent_id, tool_name, target_key,
  since) -> Result<Option<(String, Option<String>, String)>>`
  (`after_value`, `reasoning`, `created_at`, `ORDER BY created_at DESC LIMIT 1`)
  plus son enveloppe `AsyncDatabase`. Une seule méthode, une seule requête, et le
  même prédicat que la méthode voisine.
- **Le dépliage du `reasoning` est une fonction pure**
  (`parse_degroom_marker(after_value, reasoning) -> DegroomMarker`), testable
  sans base. **Granularité du fail-soft, et elle est porteuse :** la *décision* du
  lecteur ne dépend que de la **présence** de la ligne ; la branche est un
  enrichissement. Un `reasoning` non exploitable produit donc « dé-groomé par
  pr#2226, branche inconnue », jamais un silence.
- **`since` = la fenêtre de rétention du registre.**
  `compact_old_audit_events(90)` purge à 90 jours, exactement la demi-vie que
  mika#2199 a dû nommer pour `wip_rescue_bailed`. Le lecteur lit donc sur
  90 jours, via une constante dérivée du même nombre pour que les deux ne
  puissent pas dériver. **Coût nommé :** un ticket dé-groomé il y a plus de
  90 jours perd son attribution et se relit comme « non groomé » —
  c'est-à-dire le comportement d'aujourd'hui. Fail-open dans le sens sûr.

### La portée agent est une condition de fonctionnement, pas un détail

`audit_events` est scopé par `agent_id`, et
`count_recent_audit_events_for_target` filtre `agent_id = ?1`. **Le producteur et
le lecteur doivent donc tourner sur le même agent.** Ils le font : `route_event`
rend `mika-dev` pour `pull_request.closed` **et** pour `issues.labeled`. Cette
condition est écrite au site d'émission, sur le modèle du « Scope note » que
`ci_success_handler.rs:212` porte déjà pour la même primitive — parce que si elle
cessait d'être vraie, le marqueur deviendrait invisible **sans qu'aucun test ne
rougisse**.

**Limite corollaire, nommée :** la résolution de siège (mika#2084) peut router
les événements d'un dépôt vers un `dispatch:<seat>` distinct. Si les deux
événements d'un même dépôt atterrissaient sur des sièges différents, le marqueur
serait invisible et le lecteur retomberait sur le comportement d'aujourd'hui
(fail-open, aucune attribution fausse). La population mesurée est
`senara-solutions/mika`, dont les deux événements vont à `mika-dev`.

---

## Ce qui est refusé, et pourquoi

Chaque refus porte sa raison, parce que chacun est la piste évidente.

- **Halter le dispatch sur un ticket dé-groomé** (la lettre de l'option 2 :
  « surfacer plutôt que boucler en groom »). Refusé sur deux mesures. (a) La
  prémisse a changé : la boucle est bornée à trois re-drives depuis mika#2020
  (R-c), donc « boucler » n'est plus le mal à éviter. (b) Re-groomer un ticket
  dé-groomé est un **travail correct**, pas une erreur : le plan est
  inatteignable depuis l'issue, et un groom qui aboutit rétablit précisément les
  callouts qui manquent — c'est d'ailleurs le remède que l'opérateur a appliqué à
  la main. Halter échangerait une perte d'autonomie contre un gain d'attribution
  qu'on obtient sans elle. *Un filet, pas un chemin* (mika#2334).
- **Copier le plan vers chaque sous-ticket à la fermeture** (la lettre de
  l'option 1). Refusé ici, nommé en suivi. Cela veut dire : créer une branche par
  sous-ticket, y `cherry-pick` le plan, réécrire son en-tête pour qu'il réclame
  le bon ticket (sinon le palier 1 de `_find_issue_plan` le **rejette**
  explicitement — `dispatch-lib.sh:4974`), et poser trois callouts sur chaque
  issue. C'est-à-dire réimplémenter `_write_canonical_callout` plus une chirurgie
  git, accroché à un événement unique non rejouable, pour **une seule occurrence
  mesurée**. mika#2358 a refusé un mécanisme pour un défaut non mesuré ;
  mika#2420 a refusé d'élargir un prédicat pour un problème de dimensionnement.
  Même arbitrage. **Précondition du suivi : une seconde occurrence**, que le
  compteur du § Surfaces opérateur fournira.
- **Faire voyager le pointeur dans le `prompt` du dispatch.** Impossible sans
  casser le grooming : voir R-d (mika#1593).
- **Commenter le sous-ticket à la fermeture.** Ce serait la surface la plus
  visible, et elle est refusée pour trois raisons. Le handler est
  **side-effect-only** par contrat écrit et ne fait aujourd'hui aucune écriture
  GitHub ; le producteur **ne peut pas savoir** si le ticket est réellement
  dé-groomé (il n'a pas le corps de l'issue), donc il commenterait aussi les
  fermetures anodines — une PR de brouillon fermée, une PR remplacée — et un
  commentaire faux sur un ticket sain est un dommage du même ordre que le silence
  qu'il remplace ; enfin le lecteur, qui *sait*, tourne à chaque re-drive, donc y
  commenter demanderait une déduplication par ledger (motif mika#2361) pour un
  bénéfice que les deux surfaces de § Surfaces opérateur donnent déjà.
- **Aller chercher le corps complet de la PR en GraphQL** pour fermer l'angle
  mort R-e. Ce serait une lecture réseau ajoutée à un handler qui n'en fait
  aucune, sur **toute** fermeture de PR, et cela changerait la population du
  nettoyage de rows existant — donc un blast radius que R2 interdit. Suivi, avec
  pour précondition la mesure que R6 produit.
- **Scinder `FILTER_NOT_GROOMED` d'`auto_pull`** en deux noms de filtre. Le motif
  serait le bon, mais le prédicat ne l'atteindrait pas : `not_groomed` est posé
  par le filtre de candidature des phases 0/1, qui travaillent le bassin
  `!ready`, alors que le ticket mesuré **a conservé `ready`** et relève de la
  phase 2. Le nom serait juste et la population vide.
- **Un kill-switch.** Voir R8.

---

## Definition of Done

1. `upstream_close_handler` écrit le marqueur sur la branche unmerged, pour
   chaque ref fermante, avec la branche de tête quand elle est lisible.
2. `upstream_close_handler` émet la ligne d'inertie R6 quand un corps tronqué n'a
   livré aucune ref sur une fermeture unmerged.
3. `ready_label_handler` nomme le dé-groomage quand il route vers `groom` et que
   le marqueur est présent, et reste muet sinon.
4. `Database::latest_audit_event_for_target` + enveloppe `AsyncDatabase`.
5. Fonctions pures `extract_head_branch` et `parse_degroom_marker`, unitairement
   testées.
6. Les deux noms d'audit déclarés à écrivain unique, tenus par un scan de source.
7. `TRUNCATED_BODY_MARKER` déclaré dans `scripts/canonical-tokens.tsv`.
8. `cargo test`, `cargo clippy`, `cargo fmt --check` verts ; `make lint`.
9. L'entrée opérateur de § Surfaces opérateur reportée dans `CLAUDE.md`, au
   voisinage des entrées `auto_pull` / `ready_label` (c'est là que cherche
   l'opérateur qui lit un ticket refluant en groom).

---

## Acceptance criteria

- **AC1** — À la fermeture **sans merge** d'une PR déclarant `Closes #N`, une
  ligne `audit_events` durable existe pour `#N`, nommant la PR et sa branche de
  tête.
- **AC2** — À la fermeture **avec** merge, aucune ligne de ce nom n'est écrite
  (contrôle négatif).
- **AC3** — Quand un ticket portant ce marqueur est routé vers `groom`, une ligne
  `audit_events` et une ligne INFO nomment le dé-groomage, la PR et la branche.
- **AC4** — Un ticket **groomé** (callouts présents) et un ticket simplement
  **non groomé sans marqueur** ne produisent aucune de ces lignes (double
  contrôle négatif : ni le chemin nominal `implement`, ni le premier grooming
  nominal, ne sont bruités).
- **AC5** — Le comportement de routage est inchangé : même `dispatch_class`, même
  `target_skill`, même `ReadyLabelGate`, avec et sans marqueur.
- **AC6** — Le nettoyage de rows de suivi existant est inchangé : mêmes rows
  transitionnées, mêmes statuts, mêmes résultats, même événement d'audit.
- **AC7** — Un registre illisible ne change ni le routage ni le nettoyage, et
  émet une ligne nommée.
- **AC8** — Une fermeture unmerged à corps tronqué et sans ref parsée émet la
  ligne d'inertie ; une fermeture unmerged à corps **non** tronqué et sans ref ne
  l'émet pas.
- **AC9** — Chacun des deux noms d'audit n'a qu'un seul site d'écriture dans
  `crates/`.
- **AC10** — Une requête SQL d'une ligne répond à « pourquoi ce ticket n'est-il
  plus groomé, et quelle branche porte son plan ? ».

---

## Surfaces opérateur, sondes et haltes

### SQL

```sql
-- « Pourquoi ce ticket n'est-il plus groomé, et où est son plan ? » (AC10)
SELECT created_at, after_value, reasoning FROM audit_events
 WHERE tool_name = 'closing_pr_closed_unmerged'
   AND target_key = 'issue:senara-solutions/mika#2131';

-- Quelle umbrella a dé-groomé quoi (population du producteur)
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'closing_pr_closed_unmerged' GROUP BY 1 ORDER BY 2 DESC;

-- Le défaut réellement vécu (population du lecteur) — doit être TRÈS inférieure
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'ready_label_degroomed' GROUP BY 1 ORDER BY 2 DESC;
```

### Journal (`$MIKA_SPIRIT_LOG_FILE`)

| événement | niveau | régime attendu | lecture |
|---|---|---|---|
| `closing_pr_closed_unmerged` | INFO | **non vide, faible** | une fermeture sans merge a délié un ticket de sa PR fermante. Fréquent et souvent anodin. |
| `ready_label_degroomed` | INFO | **proche de zéro** | chaque ligne est du grooming revu qu'on est en train de re-payer. |
| `closing_pr_body_truncated_no_refs` | WARN | **inconnu, à mesurer** | l'angle mort R-e. Non vide ⇒ des fermetures unmerged passent sous le radar du producteur. |
| `ready_label_degroom_ledger_unreadable` | WARN | **vide** | toute occurrence est une attribution perdue (jamais fausse). |

### Sondes, et leurs haltes

1. **Contrôle positif du producteur (7 jours).** `closing_pr_closed_unmerged`
   non vide. **Halte 1 —** si cette ligne est vide alors que des PR ont été
   fermées sans merge, **ne pas élargir le prédicat** : lire d'abord
   `closing_pr_body_truncated_no_refs`. Si *elle* est non vide, le producteur est
   inerte par troncature (R-e) et le remède est son suivi, pas ce prédicat. Si
   les deux sont vides, établir le déploiement (classe mika#2340) avant toute
   conclusion — *une ligne absente ne prouve rien tant qu'on n'a pas établi que
   le binaire qui tourne sait l'écrire*.
2. **Attribution du lecteur (30 jours).** `ready_label_degroomed` doit rester
   proche de zéro. **Halte 2 —** si ce compte porte du trafic nominal, ce n'est
   pas le lecteur qui est trop large : c'est que des tickets dé-groomés sont
   dispatchés en série, et **c'est la décision de refus de § Ce qui est refusé
   qu'il faut rouvrir** (copie du plan à la décomposition) — avec ce compte comme
   précondition, qui est exactement ce que le suivi attendait.
3. **Attribution du défaut fondateur.** Le marqueur de #2131 n'existe pas et
   n'existera pas : la PR #2226 a été fermée avant ce correctif, et **rien ici ne
   rétro-remplit le registre** (fabriquer une ligne d'audit datée d'un événement
   qu'on n'a pas observé serait l'inverse de tout ce que ce plan défend). La
   sonde est donc la **prochaine** occurrence, et c'est une limite à écrire.
4. **Contrôle négatif de bruit (7 jours).** Aucun `ready_label_degroomed` sur un
   ticket dont le corps porte ses trois callouts. **Halte 3 —** une occurrence
   signifie que le lecteur est atteint sur le chemin `implement`, donc mal
   placé : le corriger, ne pas filtrer en aval.

**Ce que ce travail n'achète pas.** Il ne réutilise aucun plan automatiquement :
il rend le plan **désignable** (un numéro de PR, une branche) là où il fallait une
archéologie. Il ne rétro-remplit rien (sonde 3). Et `ready_label_degroomed` ne
peut pas voir un ticket dont personne ne repose le label : *une attribution que
personne ne déclenche reste un silence*.

---

## Fire-Disposition

Ce plan livre des détecteurs : les tests unitaires et comportementaux, et **un
scan de source**. Disposition retenue : **(a) exception nommée en allowlist,
allowlist livrée VIDE**.

- **Détecteur 1 —
  `canonical_tokens::tests::mika2242_the_two_audit_names_have_a_single_writer`.**
  Scan de source sur `crates/`, refusant plus d'un site d'écriture pour
  `"closing_pr_closed_unmerged"` et `"ready_label_degroomed"` (hors
  `#[cfg(test)]` et hors les requêtes SQL citées en documentation). Motif :
  `worktree_reaped` (mika#2420) et `qa_callback_verdict` (mika#2368), dont les
  requêtes SQL publiées reposent sur la même propriété.
  - **Violations existantes : zéro, vérifié** — les deux noms sont nouveaux
    (`grep -rn 'closing_pr_closed_unmerged\|ready_label_degroomed' crates/
    scripts/ skills/` rend vide à HEAD `2b5456cc`). L'allowlist
    `SOLE_WRITER_EXCEPTIONS` est donc **livrée vide**, et le test **asserte
    qu'elle l'est** : le jour où quelqu'un y ajoute une entrée, l'assertion
    auto-nettoyante rougit. *On déclare, on n'allowliste pas.*
  - **Bascule en (c) halte-et-remontée si le scan rougit à l'arrivée.** Une
    violation au premier `cargo test` signifierait une **collision de nom** avec
    un écrivain préexistant — c'est-à-dire que la propriété SOLE WRITER dont
    dépendent les requêtes de § Surfaces opérateur est fausse avant d'être posée.
    Dans ce cas l'implémentation **s'arrête et remonte** : le remède est de
    renommer, jamais d'excepter, parce qu'excepter rendrait les `GROUP BY`
    silencieusement faux.
- **Détecteur 2 — les tests comportementaux et unitaires.** Verts à l'arrivée par
  construction : ils portent sur du code neuf et sur des fonctions pures, et
  aucun n'inspecte de donnée préexistante. Aucune disposition requise ; c'est dit
  pour que l'absence d'exception soit une constatation et non un oubli.
- **Détecteur 3 — l'entrée `TRUNCATED_BODY_MARKER` dans
  `scripts/canonical-tokens.tsv`**, qui arme le gate CI `canonical-tokens-lint`
  sur un jeton de plus. Vérification avant commit :
  `scripts/check-canonical-tokens.sh` et `scripts/canonical-tokens-survey.sh
  --check` verts. Si la déclaration fait accuser un texte préexistant, la
  résolution est une exception **nommée** dans
  `scripts/canonical-tokens-exceptions.tsv` — quatre champs, ticket de suivi
  référencé, assertion auto-nettoyante — et **jamais** le retrait de la
  déclaration.

---

## Verification contract

### Tests unitaires — `server/upstream_close_handler.rs`

- `extract_head_branch` sur la forme canonique du gateway, sur un en-tête sans
  `(branch: …)`, sur un texte vide.
- Troncature : un texte finissant par `[truncated]` est détecté ; un texte qui
  **contient** le mot dans le corps de la PR sans le porter en fin ne l'est pas
  (le faux positif de prose, classe mika#2050).

### Tests unitaires — `server/ready_label_handler.rs`

- `parse_degroom_marker` : record complet ; `reasoning` absent ; `reasoning`
  inexploitable ⇒ marqueur présent, branche `None` (la granularité du fail-soft
  de § Conception).

### Tests comportementaux — `crates/mika-agent/tests/eval/test_degroom_attribution_2242.rs`

Sur une base réelle, via `EvalHarness` (motif
`test_qa_review_reconcile_2347.rs`) :

1. **AC1** — fermeture unmerged `Closes #N` ⇒ une ligne, portant `pr#M` et la
   branche.
2. **AC2** — même texte avec `Merged: true` ⇒ **zéro** ligne.
3. **AC3** — marqueur posé puis `labeled(ready)` sur un corps sans callouts ⇒ une
   ligne `ready_label_degroomed` portant `pr#M` et la branche.
4. **AC4a** — corps **avec** les trois callouts ⇒ zéro ligne.
5. **AC4b** — corps sans callouts et **sans** marqueur ⇒ zéro ligne.
6. **AC5** — la tâche créée porte `dispatch_class = "groom"` et le même
   `ReadyLabelGate` avec et sans marqueur.
7. **AC6** — les rows de suivi transitionnées et leur ligne
   `tracking_row_upstream_closed` sont identiques avec et sans le nouveau bloc.
8. **AC8** — corps tronqué sans ref ⇒ ligne d'inertie ; corps court sans ref ⇒
   pas de ligne.

**Le test 2 et les tests 4a/4b sont porteurs, pas décoratifs** : sans eux, un
marqueur écrit inconditionnellement et un lecteur firant sur tout ticket non
groomé passeraient tous les tests positifs, en produisant un bruit qui noierait
exactement le signal que ce plan existe pour lever.

### Vérifications non automatisables

- La condition de portée agent de § Conception est une **propriété de la table de
  routage du gateway**, pas de ce code. Elle est épinglée par une assertion sur
  `route_event("pull_request", Some("closed")) == route_event("issues",
  Some("labeled"))` — l'égalité, pas la valeur `mika-dev`, parce que c'est
  l'égalité qui est la condition de fonctionnement.

---

## Hors périmètre, délibérément

- **La copie du plan aux sous-tickets à la décomposition** (option 1 littérale) —
  refus raisonné en § Ce qui est refusé, **suivi à ouvrir**, précondition : la
  sonde 2 (une seconde occurrence mesurée).
- **L'angle mort de troncature à 2000 caractères** (R-e) — nommé, instrumenté,
  non corrigé. **Suivi à ouvrir**, précondition :
  `closing_pr_body_truncated_no_refs` non vide.
- **Le halt au routage** (option 2 littérale) — refus raisonné en § Ce qui est
  refusé.
- **La réutilisation automatique du plan par le pilote de groom** — demanderait
  de récupérer une branche étrangère, d'y localiser un plan dont le nom ne porte
  pas le ticket, et de réécrire son en-tête pour passer le palier 1 de
  `_find_issue_plan`. Suivi, même précondition que le premier point.
- **`claude-pilot#166`** (l'échec du groom-reuse au niveau bac à sable) — le
  ticket le nomme comme complémentaire, et il l'est : il vit dans un autre dépôt,
  hors de l'allowlist de dispatch de la boucle.
- **Le rétro-remplissage du registre pour #2131** — voir sonde 3.
- **Le vocabulaire de filtres d'`auto_pull`** — refus raisonné en § Ce qui est
  refusé (le nom serait juste, la population vide).
