# Plan : la porte de promotion sépare les deux populations par ce que les commits **touchent**, pas par leur nombre (mika#2140)

**Ticket :** mika issue#2140 — `fix(auto_pull): la porte de promotion lit `ahead_by > 1` comme du travail de pilote, alors que le grooming produit légitimement 2-3 commits de plan`
**Labels :** `bug`, `p1-important`
**Type :** issue (bug — casseur de boucle : la porte retire du bassin `ready` les tickets les plus travaillés)
**Palier de priorité :** Tier 1 — *casse la boucle*. Bassin `ready` à 1 sur un plancher de 3, avec deux candidats groomés gatés à tort.
**Fichier principal :** `crates/mika-agent/src/auto_pull.rs`

---

## Problème

`classify_promotion` (`auto_pull.rs:680`) sépare « branche de grooming pur » de « branche portant du travail d'un pilote mort » par un **compte de commits** :

```rust
if staleness.ahead_by > 1 {
    return PromotionGate::Refuse(RefusalReason::SalvageWorkOnStaleBranch { … });
}
```

L'hypothèse est écrite au module (`auto_pull.rs:37-39`) :

> `ahead_by` separates the two populations for free: a branch carrying only its plan has `ahead_by == 1`; every branch that died on 2026-08-31 carried more.

Elle est fausse sur le **chemin nominal** du grooming. `.claude/commands/mika-groom-ticket.md` commite le plan à trois sites distincts — Phase 3 étape 10, Phase 4 étape 12, Phase 5 étape 17 — et c'est délibéré : la lignée doit rester lisible entre « l'architecte a signé » et « l'opérateur a rédigé » (Phase 2 étape 7). Tout ticket ayant demandé un aller-retour architecte porte donc `ahead_by ∈ {2,3}` sans qu'aucun pilote ne l'ait touché.

**L'énoncé le plus court du défaut**, tel que le commentaire du 2026-09-02 12:08Z le formule :
*le prédicat pénalise exactement la propriété qu'il devrait récompenser.* Plus un plan est
retravaillé, plus il porte de commits, plus la porte le classe « travail partiel d'un pilote mort ».
Un ticket groomé en une passe passe ; un ticket groomé en trois est gaté à vie.

C'est la troisième récidive en 48 h de la même forme — *un garde encode une hypothèse sur ce que son producteur produit, et le producteur produit légitimement autre chose* — après `is_groomed` et `dispatch-lib.sh:4405` (mika#2120). Précédent applicable : `docs/solutions/architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md`.

## Mesures — exécutées le 2026-09-03, pas déduites

Toutes les lignes ci-dessous viennent d'appels réels à `GET /repos/senara-solutions/mika/compare/main...<branch>`.

### M1 — les deux faux positifs, et le fait que l'API porte déjà le signal manquant

```
fix/2118/skills-cloud-sur-un-tenant-cloud-google
  {"status":"diverged","ahead_by":3,"behind_by":8,"total_commits":3,
   "files":["docs/plans/2026-09-01-003-fix-2118-gws-cloud-design-limit-plan.md"]}

fix/2120/auto-pull-is-groomed-exige-docs-plans
  {"status":"diverged","ahead_by":2,"behind_by":8,"total_commits":2,
   "files":["docs/plans/2026-09-01-004-fix-2120-is-groomed-repo-prefix-plan.md"]}
```

`behind_by > 0` et `ahead_by > 1` → la porte actuelle rend `SalvageWorkOnStaleBranch` sur les deux. Le diff complet de chacune contre `main` est **un seul fichier**, sous `docs/plans/`. Les deux tickets portent encore `operator-gated` et sont encore `OPEN` (vérifié 2026-09-03).

### M2 — le contrôle négatif existe et il est net

```
fix/1680/mika-dev-tui-broken-glyph-rendering-in
  {"status":"diverged","ahead_by":2,"behind_by":197, "files":[
     "crates/mika-agent/src/agent_loop/mod.rs",
     "crates/mika-agent/src/evidence/guards.rs",
     "crates/mika-agent/src/evidence/mod.rs",
     "crates/mika-agent/src/well_known_agents.rs",
     "docs/plans/2026-06-30-016-fix-1680-mika-dev-cn-output-bleed-plan.md"]}
```

Quatre fichiers de code **plus** un fichier de plan. C'est exactement la branche morte du 2026-08-31 dont le vrai `git rebase origin/main` conflit sur `agent_loop/mod.rs` et `evidence/guards.rs` (mesuré le 2026-09-01, `tests/fixtures/auto_pull_compare/PROVENANCE.md`). Le prédicat par fichiers la refuse toujours.

### M3 — le préfixe est bien `docs/plans/`, sans préfixe de dépôt

Le point qui a fait tomber le garde frère (mika#2120) : `is_groomed` exigeait `docs/plans/` là où la spec écrivait `mika/docs/plans/`. Ici la question est tranchée par mesure, pas par lecture de spec — **l'endpoint `compare` rend des chemins relatifs à la racine du dépôt** : `docs/plans/2026-09-01-004-…`, jamais `mika/docs/plans/…`. Le préfixe littéral `docs/plans/` est donc correct *à cette frontière-là*, et le plan écrit pourquoi à côté de la constante pour que le prochain lecteur n'ait pas à refaire l'appel.

### M4 — la forme des objets de `files`

```
$ gh api …/compare/main...fix/2120/… --jq '.files[0]|keys'
["additions","blob_url","changes","contents_url","deletions","filename","patch","raw_url","sha","status"]
```

Le seul champ lu sera `filename`. Les fixtures gelées ne garderont que celui-là (les `patch` font des mégaoctets — c'est déjà la raison pour laquelle `files` avait été retiré des fixtures de mika#2123).

### M5 — l'état actuel des fixtures gelées, qui est un piège

Les quatre fixtures de `tests/fixtures/auto_pull_compare/` ont été gelées **sans** la clé `files` (PROVENANCE : *« `commits`, `files`, `base_commit` and `merge_base_commit` were dropped »*). Sous le nouveau prédicat, une fixture sans `files` tombe dans le chemin « liste indisponible » → jamais `salvage`. Le test d'intégration `auto_pull_replay_1680_is_refused_by_name` deviendrait vert **pour la mauvaise raison** (il basculerait sur `branch_too_far_behind`, 197 > 50) et son assertion `slug() == "salvage_work_on_stale_branch"` passerait au rouge. Les fixtures doivent donc être ré-enrichies, pas laissées telles quelles. C'est un livrable, pas un détail.

### M6 — le tir du nouveau prédicat sur le backlog vivant, compté

Le nouveau prédicat n'est **pas** strictement plus permissif que l'ancien, et il faut le dire avant
de l'écrire : une branche à `ahead_by == 1` dont l'unique commit touche du **code** promouvait sous
`ahead_by > 1` et **refuse** désormais. Cette population n'est pas hypothétique par construction —
elle est mesurable. Mesurée le 2026-09-03 sur les **11** tickets ouverts de `senara-solutions/mika`
portant un callout `> - **Branch:**`, en évaluant les deux prédicats sur le même appel `compare` :

| ticket | branche | ahead | behind | ancien → nouveau | fichiers |
|---|---|---|---|---|---|
| #2126 | `fix/2126/…` | 1 | 13 | distance → distance | 1 plan |
| #2121 | `fix/2121/…` | — | — | `BranchAbsent` (branche absente d'`origin`) | — |
| #2120 | `fix/2120/…` | 2 | 8 | **`salvage` → promotion** | 1 plan |
| #2118 | `fix/2118/…` | 3 | 8 | **`salvage` → promotion** | 1 plan |
| #2117 | `research/2117/…` | 1 | 17 | distance → distance | 1 plan |
| #2108 | `fix/2108/…` | 1 | 0 | promotion → promotion | 1 plan |
| #2036 | `bug/2036/…` | 2 | 60 | **`salvage` → `TooFarBehind`** *(latent : porte `ready`)* | 1 plan |
| #1959 | `feat/1959/…` | 1 | 92 | distance → distance | 1 plan |
| #1949 | `feat/1949/…` | — | — | `BranchAbsent` | — |
| #1727 | `feat/1727/…` | 3 | 185 | `salvage` → `salvage` | 1 plan + 1 **doc** hors `docs/plans/` |
| #1381 | `feat/1381/…` | 1 | 97 | distance → distance | 1 plan |

Quatre faits que cette table établit et que le ticket n'avait pas :

1. **La direction dangereuse est vide : zéro bascule promotion → refus sur 11.** Aucune branche
   ouverte ne porte un unique commit de code. Le rétrécissement théorique du chemin d'acceptation
   n'a, aujourd'hui, aucun sujet.
2. **Un troisième cas du même défaut, non compté par le ticket : #2036 — mais latent, pas déjà tiré.**
   `ahead_by = 2`, un seul fichier, sous `docs/plans/`. Il porte aujourd'hui `ready`, pas
   `operator-gated` : la porte ne l'a donc pas encore refusé. Elle le ferait au premier passage par
   le sauvetage `stuck-ready` de la Phase 2, qui est une re-promotion et traverse `promotion_gate_allows`
   comme les deux autres — et elle dirait `salvage` sur une branche qui ne porte que son plan. Après
   correctif il reste refusé — 60 commits de retard, au-delà du seuil de 50 — mais sous
   `TooFarBehind`, le motif **honnête**. La mesure du ticket (« n = 2 sur 2 ») portait sur les
   `operator-gated` ouverts, ce qui était exact pour sa question ; #2036 dit que la population du
   défaut est plus large que celle où il a déjà laissé une trace. Corriger un motif faux compte même
   quand la décision ne change pas : c'est le motif qui dit à l'opérateur quoi faire.
3. **Un contrôle positif vivant : #1727**, qui reste `salvage` après correctif — la porte ne devient
   pas passe-partout sur le backlog réel.
4. **#1727 est aussi le cas-limite du préfixe étroit**, et il est instructif : son unique fichier
   hors plan est `crates/mika-cli/docs/2026-07-06-tui-thin-client-phase-1-audit-and-plan.md` — un
   document d'audit, pas du code. Traité en § Fire-Disposition.

Les deux `BranchAbsent` (#2121, #1949) désignent des branches qui n'existent plus sur `origin`.
C'est le troisième chemin de refus, inchangé par ce ticket, et ce n'est pas son sujet — noté ici
pour que la table soit complète, pas pour élargir le périmètre.

### M7 — deux mesures venues des commentaires du ticket, et ce qu'elles changent

Les commentaires du 2026-09-02 (10:14Z et 12:08Z) portent deux mesures que le corps n'a pas, et qui
touchent l'une le périmètre, l'autre AC6. Elles sont reprises ici parce qu'un plan qui ne les
nomme pas laisse son implémenteur les redécouvrir.

**M7a — le faux positif est auto-régénérant ; un correctif manuel ne tient pas.** Le 2026-09-02, les
deux branches ont été rebasées à la main (`behind_by = 0`, donc promotion par le court-circuit
d'amont `up_to_date`), `ready` reposé, `operator-gated` retiré. Les deux ont été re-gatées dans
l'heure : `#2120` à 11:10:02Z (**63 min** de survie), `#2118` à 11:20:09Z (~73 min). La raison est
mécanique : le rebase met `behind_by` à zéro *à cet instant* ; dès qu'un commit atterrit sur `main`,
`behind_by` repasse au-dessus de zéro, le court-circuit ne s'applique plus, et `ahead_by > 1`
reprend la main. Un ticket groomé en deux passes oscille donc entre `ready` et `operator-gated` au
rythme des merges, sans jamais être dispatché. Le bassin `ready` était à **2** à 12:10Z, sous le
plancher de 3, avec trois candidats groomés dehors.

**M7b — le prédicat `ahead_by` est faux dans les deux sens, et le nouveau ne corrige qu'un sens.**
Un pilote tué avant son premier commit ne modifie pas `ahead_by` : le travail partiel qu'il laisse,
s'il en laisse, est **non commité**, donc invisible à un compte de commits — et tout aussi invisible
à une liste de fichiers `compare`, qui ne lit que ce qui est commité. Mesuré : deux dispatches ont
réellement travaillé sur `fix/2118/…` le 2026-09-02 (sessions `16e51db9`, 31 appels d'outils ;
`478b7b1c`, 34 appels), tous deux tués par `idle_timeout`, et le `reflog` montre que `HEAD` n'a
jamais bougé — huit `checkout`, zéro `commit`.

| population | `ahead_by > 1` | prédicat par fichiers |
|---|---|---|
| branche groomée en 2–3 passes, sans pilote | **refuse** (faux positif) | promeut ✔ |
| branche portant du travail de pilote **commité** | refuse ✔ | refuse ✔ |
| pilote mort **avant** son premier commit | promeut (faux négatif) | promeut (faux négatif, inchangé) |

Ce ticket ferme la première ligne. Il ne ferme pas la troisième, et ne le prétend pas : une porte
qui verrait le travail non commité devrait lire un état git local que ce module n'a pas (il n'a
aucun checkout — c'est `auto_pull.rs:1327-1333`). Le faux négatif reste ouvert, hors périmètre.

**Et il est aujourd'hui sans sujet.** Tant que mika#2141 tient — le bac à sable ne monte pas le
gitdir, donc aucune commande git ne fonctionne à l'intérieur d'un pilote — la population « travail
partiel **commité** par un pilote » que cette porte existe pour protéger **n'existe pas du tout**.
Cela ne justifie pas de retirer la règle `salvage` (elle protège des branches historiques comme
#1680 et #1727, qui sont réelles), mais cela borne le coût accepté en § Décision de conception : la
branche-à-liste-`files`-indisponible qui promeut désormais devrait, pour nuire, porter du travail de
pilote commité — ce qu'aucun pilote ne peut produire tant que #2141 n'est pas résolu.

### M8 — ré-exécution du 2026-09-04 à l'implémentation : le déclencheur ne tire pas, et le défaut a grossi

Le § Fire-Disposition attache un **déclencheur de halte-et-remontée** à M6 : si, au moment
d'implémenter, une seule branche ouverte bascule promotion → refus, l'implémenteur s'arrête. La
mesure a donc été refaite le **2026-09-04**, sur les **18** tickets ouverts portant un callout
`> - **Branch:**` (contre 11 la veille), les deux prédicats évalués sur le même appel `compare`.

**Le déclencheur ne tire pas : zéro bascule promotion → refus sur 18.** L'implémentation continue.

Trois faits que la ré-exécution ajoute :

1. **Le défaut a grossi d'un facteur cinq en un jour.** Dix branches basculent aujourd'hui
   `salvage → promotion` — #2160, #2158, #2157, #2156, #2143, **#2140 elle-même**, #2127, #2120,
   #2118, #1772 — contre deux le 2026-09-03. Toutes portent un unique fichier, sous `docs/plans/`.
   La branche de ce ticket (`ahead = 3`, `behind = 1`) est un exemplaire du défaut qu'elle corrige :
   la porte refusait le correctif de la porte.
2. **#1727 tient son rôle de contrôle positif vivant**, et son refus reste surdéterminé — 190 de
   retard (185 la veille), toujours au-delà du seuil de 50. L'assertion auto-nettoyante est donc
   satisfaite.
3. **Un cas hors périmètre de M6 apparaît dans le jeu de fixtures, et il bascule dans la direction
   dangereuse : `ci/2048-re-enable-release-please`.** 17 de retard, `ahead = 1`, trois fichiers de
   configuration et **aucun plan**. Elle promouvait ; elle refuse désormais, et le préfixe en est la
   cause **unique** — c'est-à-dire exactement la forme que le § Fire-Disposition dit de rouvrir au
   lieu de la laisser se répondre en silence. M6 ne l'avait pas vue parce que M6 compte les tickets
   **ouverts** et que #2048 est fermé.

#### Ce qui a été décidé sur #2048, et pourquoi ce n'est pas un élargissement du préfixe

Le préfixe **n'est pas élargi** : le plan interdit à l'implémenteur de le faire de sa propre
autorité, et cette borne est respectée. La fixture est conservée et son test **retourné en contrôle
nommé**, qui asserte le nouveau refus *et* que la distance ne le refuse pas (17 < 50) — l'assertion
auto-nettoyante, dans l'autre sens.

Le fait est borné par mesure, pas par argument : **cette branche ne peut pas atteindre la porte.**

| chemin | filtre en amont | atteint la porte ? |
|---|---|---|
| Phase 0 `phase0_feed_ready_pool` | `is_groomed` (`auto_pull.rs:1938`) | non |
| Phase 1 `phase1_promote_groomed` | `is_groomed` via `select_best_candidate` (`:818`) | non |
| Phase 2 `phase2_reconcile_stuck_ready` | label `ready` seul | **oui, en principe** |

`is_groomed` (`:301-314`) exige littéralement ``> - **Plan:** `docs/plans/`` ; une branche sans
fichier de plan n'a pas de ticket qui passe ce filtre. Reste la Phase 2, dont la population vivante
est **vide** : sur les 18 tickets ouverts à callout du 2026-09-04, **zéro** désigne une branche sans
plan. Et #2048 lui-même est **fermé** et ne porte aucun callout de grooming — il n'atteint la porte
par aucun des trois chemins.

Le contrôle négatif que #2048 portait — « en retard, mais promue » — n'est pas perdu : il passe aux
replays #2118/#2120, sur des branches qui sont réellement des branches de grooming, c'est-à-dire la
population que cette porte existe pour juger.


## Décision de conception

**Le prédicat de `salvage` devient : « la branche modifie au moins un fichier hors `docs/plans/` par rapport à `main` ».** `ahead_by` cesse d'être le discriminant et redevient ce qu'il est — une distance, journalisée, jamais interprétée.

### Ordre des règles, et pourquoi la troncature ne demande pas de branche à part

```
non_plan = files.filter(|f| !f.starts_with("docs/plans/"))
si non_plan est non vide            → Refuse(SalvageWorkOnStaleBranch { non_plan })
sinon                                → pas de salvage ; on tombe sur la règle de distance
```

La troncature de l'API est **déjà** traitée par cet ordre, et c'est ce qui rend AC4 gratuit plutôt qu'ajouté :

- Si la liste tronquée contient déjà un fichier hors plan, le fait recherché est acquis — la troncature ne peut pas le retirer. Refus.
- Si tous les fichiers visibles sont sous `docs/plans/`, la seule ignorance possible est « il y avait peut-être du code plus loin » → la branche n'est pas classée `salvage` → **elle promeut**, conformément à l'invariant déjà écrit au module (`auto_pull.rs:25-29` : *« Every "could not measure" outcome promotes »*) et au fait que le vrai rebase tranche de toute façon au dispatch.

Une liste `files` absente ou non-tableau prend exactement le même chemin, pour la même raison.

**Ce que ça coûte, dit franchement.** Une branche portant du travail de pilote dont l'API ne rendrait pas les fichiers, et qui serait sous le seuil de distance, promeut désormais. C'est le prix explicite de l'invariant fail-open de ce module ; le refuser reviendrait à faire de `auto_pull` la seule porte du dépôt qui se ferme quand GitHub hoquette.

### AC6 — la levée d'`operator-gated` reste **manuelle**, et voici pourquoi

Trois raisons, dont deux sont structurelles :

1. **L'auto-levée s'auto-contredit.** `is_feeder_excluded` (`auto_pull.rs`) exclut tout ticket portant `operator-gated` des **trois** phases. Une porte qui se relit elle-même devrait d'abord ré-évaluer des tickets que sa propre exclusion lui interdit de regarder — c'est-à-dire retourner l'exclusion contre son objet.
2. **La machine ne peut pas distinguer son label de celui de l'opérateur.** `operator-gated` n'est pas un label machine : sa description déclarée (`.github/labels.yml:106`) est *« Groomed work requiring operator-host-time. Distinct from parked/blocked. No ready label. »* — un geste d'opérateur légitime. Une porte qui le retirerait dé-gaterait silencieusement du travail qu'un humain a gaté. Ce serait très exactement la faute que ce ticket corrige : un lecteur qui suppose ce que son producteur a produit.
3. **Le canal existe déjà.** Le commentaire de refus dit déjà *« puis retire le label `operator-gated` »* (`RefusalReason::comment_body`). Ce qui manquait n'était pas le geste mais sa *raison lisible* — AC5 la fournit en nommant les fichiers.

**L'objection d'idempotence, et sa réponse.** Le commentaire du 2026-09-02 12:08Z pose la seule
condition qui rende une levée manuelle acceptable : *« si la levée reste manuelle, elle doit au
moins être **idempotente vis-à-vis de l'avancée de `main`**, sinon elle est un travail de Sisyphe
déguisé en remède. »* La condition est juste, et elle est **remplie par le correctif lui-même** —
pas par une garantie ajoutée à côté :

- **Avant.** La levée manuelle passait par un rebase mettant `behind_by` à `0`, donc par le
  court-circuit `up_to_date`. Elle survivait jusqu'au prochain merge sur `main` : 63 minutes,
  mesurées (M7a). Sisyphe, exactement.
- **Après.** Une branche ne portant que son plan n'entre plus jamais dans `SalvageWorkOnStaleBranch`,
  quel que soit `behind_by`. La levée ne dépend donc plus d'une valeur que l'avancée de `main`
  détruit. Elle tient **tant que la branche reste sous le seuil de distance** — et si elle le
  franchit un jour, le refus qui arrive est `TooFarBehind`, un motif différent, avec un remède
  différent et légitime (rebaser). Ce n'est pas la même levée à refaire : c'est une autre question.

C'est ce qui rend le choix « manuel » tenable ici alors qu'il ne l'était pas hier. Dit autrement :
l'auto-levée serait un remède au symptôme d'un prédicat faux ; le prédicat corrigé retire le
symptôme.

Ce choix est écrit **dans le module**, à côté de `REFUSAL_LABEL`, pas seulement ici : un label posé par la machine et levable seulement à la main est une dette d'opérateur silencieuse tant qu'elle n'est pas nommée à l'endroit où on la lit.

**Remédiation des deux tickets déjà gatés à tort** (`#2118`, `#2120`) : geste d'opérateur unique après merge — retirer `operator-gated`. Listé en § Suite opératoire ; hors du diff, parce qu'un correctif de code n'a pas à muter l'état de tickets tiers pour prouver qu'il marche. Les fixtures AC3 sont ce qui le prouve.

## Fire-Disposition

*(Exigée par la Fire-Disposition Gate, mika#1574 — première passe mika-arch, F1 bloquant. Les
livrables D7/D8 sont de classe détecteur : leur chemin de succès est « aucune violation ».)*

Les détecteurs de ce plan sont les tests de D8 et les fixtures de D7. La question de la porte est :
**que fait l'implémentation quand un détecteur tire sur des données préexistantes ?** Il y a deux
surfaces de tir, et elles n'appellent pas la même disposition.

### Surface 1 — le jeu de fixtures gelées (CI)

**Tir certain, pas probable.** Les quatre fixtures de mika#2123 ont été gelées sans la clé `files`
(M5). Dès que le prédicat change, `auto_pull_replay_1680_is_refused_by_name` passe au rouge : la
fixture tombe dans le chemin « liste indisponible », le refus bascule sur `branch_too_far_behind`,
et l'assertion de slug échoue.

**Disposition : réparation dans le périmètre (D7), ni liste blanche ni `#[ignore]`.**

Le choix mérite sa justification, parce que la disposition par défaut de la doctrine est (a) la
liste blanche nommée. Elle ne convient pas ici : une liste blanche existe pour **isoler une
violation réelle** que le correctif ne traite pas. Or ces fixtures ne portent aucune violation —
elles sont des **captures incomplètes** d'un appel dont ce ticket a précisément besoin du champ
manquant. Les allowlister reviendrait à figer l'aveuglement que le ticket corrige, dans le fichier
même qui sert à prouver qu'il est corrigé. La ré-capture est un acte d'une ligne par fixture, sur
des branches qui existent encore sur `origin` (vérifié 2026-09-03), et le diff à trois points rend
la liste `files` invariante à l'avance de `main` (D7). C'est donc une réparation, pas une exception.

### Surface 2 — le backlog vivant (production, au tick suivant)

Mesurée, pas supposée : M6, 11 tickets, table complète.

**Direction dangereuse — promotion devenue refus : n = 0 sur 11.** Aucune disposition n'est due pour
une population vide ; ce qui est dû, c'est de dire qu'elle a été comptée et à quelle date.

**Cas-limite mesuré — #1727, n = 1 sur 11.** La branche porte un fichier hors `docs/plans/` qui est
un **document d'audit** (`crates/mika-cli/docs/…-audit-and-plan.md`), pas du code. Le prédicat
étroit le classe donc « travail qui n'est pas du grooming », ce qui est littéralement vrai et
sémantiquement discutable.

**Disposition : (a) exception nommée, mesurée, avec assertion auto-nettoyante.** Concrètement :

- #1727 est gelé comme **troisième fixture d'intégration** (`1727-diverged-185-behind-3-ahead.json`)
  et son test asserte `Refuse(salvage)` — le contrôle positif vivant.
- Le test asserte **aussi**, dans le même corps, que `behind_by > THRESHOLD`. C'est l'assertion
  auto-nettoyante : elle dit à voix haute que le refus de #1727 est **surdéterminé** — la règle de
  distance le refuserait de toute façon — donc que le préfixe étroit ne décide, ici, que du *motif*
  et jamais du *sort*. Le jour où quelqu'un gèle un cas-limite dont le préfixe serait la cause
  **unique** d'un refus qui aurait autrement promu, cette assertion échoue et rouvre la question du
  préfixe au lieu de la laisser se répondre en silence.
- Suivi : aucun ticket ouvert n'est dû tant que `n_cause-unique = 0`. La condition de réveil est
  exactement l'échec de l'assertion ci-dessus — concrète, datable, et portée par le test plutôt que
  par la mémoire de quelqu'un.

### Déclencheur de halte-et-remontée (option (c), bornée)

L'option (c) n'est pas la disposition générale — elle est le **déclencheur** attaché à la mesure M6,
parce qu'une mesure a une date et que l'implémentation arrive après :

> Si, au moment d'implémenter, la ré-exécution de la mesure M6 fait apparaître **une seule** branche
> ouverte qui bascule promotion → refus, l'implémenteur **s'arrête et remonte** avec le nom de la
> branche et de ses fichiers. Il n'élargit pas le préfixe de sa propre autorité et ne pose pas
> d'exception non nommée.

La raison est celle de la doctrine : quand la résolution de la violation préexistante *est* la
décision de périmètre, elle revient à l'opérateur. Élargir `PLAN_PATH_PREFIX` change ce que la porte
appelle « travail de pilote » — c'est une décision de politique, pas un ajustement d'implémentation.

## Deliverables

### D1 — `BranchStaleness` porte la liste des fichiers

`crates/mika-agent/src/auto_pull.rs` (~`:419`) :

```rust
pub struct BranchStaleness {
    pub behind_by: i64,
    pub ahead_by: i64,
    pub status: String,
    /// Les chemins que la branche modifie par rapport à `main`, tels que
    /// l'endpoint `compare` les rend (relatifs à la racine du dépôt — mesuré,
    /// voir le plan mika#2140 M3). `None` quand la clé `files` est absente ou
    /// n'est pas un tableau : « je n'ai pas pu lire », jamais « il n'y a rien ».
    pub changed_files: Option<Vec<String>>,
}
```

`parse_compare_payload` remplit le champ à partir de `files[].filename`, en ignorant les entrées sans `filename` (et en rendant `None` si `files` manque). **Les trois champs existants restent obligatoires** — leur absence reste une erreur de parse, contrat inchangé.

### D2 — la constante du préfixe, documentée à sa mesure

```rust
/// Le seul préfixe qu'une branche de grooming pur modifie (mika#2140).
/// Relatif à la racine du dépôt : `compare` rend `docs/plans/…`, jamais
/// `mika/docs/plans/…` — mesuré 2026-09-03, et c'est précisément l'écart
/// qui a fait tomber `is_groomed` (mika#2120).
const PLAN_PATH_PREFIX: &str = "docs/plans/";
```

Plus une fonction pure et testable :

```rust
pub fn non_plan_files(changed: Option<&[String]>) -> Vec<String>
```

`None` → vecteur vide (le fail-open, par construction et non par branche `if` séparée).

### D3 — `classify_promotion` change de prédicat, pas de forme

Le bloc `if staleness.ahead_by > 1` (`:680`) devient :

```rust
let non_plan = non_plan_files(staleness.changed_files.as_deref());
if !non_plan.is_empty() {
    return PromotionGate::Refuse(RefusalReason::SalvageWorkOnStaleBranch {
        branch,
        behind_by: staleness.behind_by,
        ahead_by: staleness.ahead_by,
        non_plan_files: non_plan,
    });
}
```

Position dans l'ordre des règles **inchangée** : avant la règle de distance, pour la raison déjà écrite (`:625-627`) — c'est le fait le plus spécifique sur la même branche, et son remède est différent.

### D4 — AC5 : le refus nomme les fichiers

`RefusalReason::SalvageWorkOnStaleBranch` gagne `non_plan_files: Vec<String>`. `reason()` cesse de parler de compte et parle de contenu :

> La branche `<b>` de #`<n>` est en retard de **N commits** et modifie **K fichier(s) hors `docs/plans/`** — du travail qui n'est pas du grooming : `a.rs`, `b.rs`, …

La liste est bornée à **10 chemins** suivis de « … et P autres » : un commentaire GitHub ne doit pas devenir un `git diff --stat`. `remedy()` garde sa formulation actuelle (le choix porte sur du travail, pas sur git) — elle reste exacte.

### D5 — AC4/AC1 : l'audit dit ce qu'il a vu

`staleness_audit_json` gagne deux champs sur les décisions `Measured` :

- `"changed_files_count"` — `null` quand la liste est indisponible, jamais `0` (même discipline que `behind_by` sur les issues non mesurées).
- `"non_plan_files"` — le tableau des chemins hors plan (borné à 10, comme le commentaire).

`ahead_by` **reste émis** : il n'est plus un discriminant, il redevient une mesure, et la promesse de KTD2c (réviser le seuil depuis une distribution réelle) en dépend.

### D6 — le module cesse d'affirmer une chose fausse

Le paragraphe `auto_pull.rs:37-39` (*« `ahead_by` separates the two populations for free »*) est la phrase qui a causé le défaut. Elle est remplacée par l'énoncé du nouveau prédicat, la mention explicite que le grooming produit 2–3 commits de plan par conception, et la référence à mika#2140. La partie qui reste **vraie** — *pourquoi* une branche portant du travail ne doit pas être rebasée en silence (deux résolutions légitimes, jugement sur du travail et non sur git) — est conservée telle quelle : ce ticket ne conteste pas la porte, il corrige son prédicat.

### D7 — fixtures : ré-enrichissement + deux nouvelles

`crates/mika-agent/tests/fixtures/auto_pull_compare/` :

| fixture | action | `files` (filename seul) |
|---|---|---|
| `1680-diverged-180-behind-2-ahead.json` | **enrichir** | 4 fichiers `crates/**` + 1 `docs/plans/**` (M2) |
| `1959-diverged-75-behind-1-ahead.json` | enrichir | 1 `docs/plans/**` |
| `2048-diverged-17-behind-1-ahead.json` | enrichir | liste réelle capturée |
| `2123-ahead-0-behind-1-ahead.json` | enrichir | liste réelle capturée |
| `2118-diverged-8-behind-3-ahead.json` | **nouvelle** | 1 `docs/plans/**` (M1) — **trois** commits de plan, le cas « plusieurs passes architecte » explicitement demandé par le commentaire du 2026-09-02 12:08Z |
| `2120-diverged-8-behind-2-ahead.json` | **nouvelle** | 1 `docs/plans/**` (M1) |
| `1727-diverged-185-behind-3-ahead.json` | **nouvelle** | 1 `docs/plans/**` + 1 doc hors plan (M6, cas-limite) |

`PROVENANCE.md` est mis à jour et doit porter **une honnêteté explicite** : pour les quatre fixtures existantes, les compteurs datent du 2026-09-01 et la liste `files` du 2026-09-03. Ce n'est pas une incohérence, et la raison est vérifiable : `compare/main...branch` est un diff à trois points, donc `files` est relatif à la **base de fusion**, qui ne bouge pas quand `main` avance — seul `behind_by` bouge (1680 : 180 → 197 le 2026-09-03, `ahead_by` inchangé à 2, liste inchangée). La ligne doit être dans le fichier, pas seulement dans ce plan.

### D8 — tests

**Unitaires** (`auto_pull::tests`). L'aide `measured(behind, ahead, status)` conserve sa signature et rend `changed_files: None` ; une aide `measured_files(behind, ahead, status, &[…])` est ajoutée. Trois tests existants doivent migrer vers `measured_files` — ils reposaient sur `ahead_by` pour obtenir `salvage` :

1. `test_promotion_gate_salvage_work_refuses_independently_of_threshold` (deux `classify_promotion` : `measured(1,2,…)` et `measured(180,2,…)` avec seuil `0`)
2. `test_staleness_audit_json_is_structured_on_promote_and_refuse` (le bloc `measured(180, 2, "diverged")` qui attend `reason == "salvage_work_on_stale_branch"`)

Non affectés : `behind_but_within_threshold` (`ahead=1`), `too_far_behind` (`ahead=1`), `threshold_zero_disables`, `fails_open`, `absent_branch`, `refusal_label_is_declared`.

Nouveaux tests unitaires :

- `test_non_plan_files_partitions` — `None` → vide ; que du plan → vide ; code seul → code ; code + plan → code seul. Contrôle négatif inclus : un chemin qui *ressemble* (`docs/plansible/x.md`) compte comme hors plan (le préfixe est littéral, `starts_with` sur `docs/plans/` avec la barre finale).
- `test_promotion_gate_multi_commit_plan_only_promotes` — **AC1**, le cœur : `measured_files(8, 3, "diverged", &["docs/plans/x.md"])` → `Promote { detail: "behind_within_threshold" }`. Le même appel avec l'ancien prédicat refusait.
- `test_promotion_gate_code_on_stale_branch_still_refuses` — **AC2** : `measured_files(8, 3, "diverged", &["crates/a.rs", "docs/plans/x.md"])` → `Refuse(salvage)`, y compris avec le seuil désactivé (`0`), donc indépendamment de la distance.
- `test_promotion_gate_missing_file_list_promotes` — **AC4** : `measured(8, 3, "diverged")` (liste `None`) → `Promote`, et l'audit porte `changed_files_count: null`.
- `test_salvage_refusal_names_the_offending_files` — **AC5** : `reason()` et `comment_body()` contiennent `crates/mika-agent/src/agent_loop/mod.rs` ; la troncature à 10 est exercée avec 12 chemins et l'assertion porte sur « … et 2 autres ».
- `test_parse_compare_payload_files_absent_is_none` + `…_reads_filenames` — le contrat de parse dans les deux sens.

**Intégration** (`crates/mika-agent/tests/auto_pull_promotion_gate.rs`) — **AC3**, sur des corps réels :

- `auto_pull_replay_2118_promotes` et `auto_pull_replay_2120_promotes` : chaque fixture rend `Promote`.
- **Non-vacuité**, exigée parce qu'un test de régression qui aurait aussi passé avant ne prouve rien : chacun des deux asserte d'abord que la fixture est bien dans la zone de refus de l'ancien prédicat — `behind_by > 0 && ahead_by > 1` — puis que la décision est `Promote`. Le test échoue donc si quelqu'un remplace la fixture par une branche à `ahead_by == 1`.
- `auto_pull_replay_1680_is_refused_by_name` : conservé, et **renforcé** — l'assertion passe de « le slug est `salvage` » à « le slug est `salvage` **et** le refus nomme `agent_loop/mod.rs` ». C'est la fixture dérivée d'une branche morte du 2026-08-31 que demande AC3.
- `auto_pull_replay_1727_is_the_measured_boundary_case` — **Fire-Disposition, surface 2** : la
  fixture rend `Refuse(salvage)` (contrôle positif vivant) **et** le test asserte `behind_by >
  THRESHOLD`, l'assertion auto-nettoyante qui déclare le refus surdéterminé. Elle échoue si
  quelqu'un fige un cas-limite dont le préfixe serait la cause unique du refus.
- Le test à seuil `0` (`tests/auto_pull_promotion_gate.rs:96`) est conservé : il prouve que les deux règles restent indépendantes.

### D9 — compound

Une entrée `docs/solutions/architecture-patterns/` étendant le précédent nommé par le ticket, sur la forme récidivante (3 occurrences en 48 h) : **un garde ne doit pas encoder une hypothèse sur la *forme* de ce que son producteur produit ; il doit lire la *substance*.** Le compte de commits est une forme, la liste des fichiers est une substance. Rédigée en `/ce:compound` à la fin du pipeline, pas ici.

## Hors périmètre

Repris du ticket, sans extension :

- `MAX_BEHIND` et `TooFarBehind` — hors cause, les deux branches sont à 8 de retard.
- L'aveuglement de préfixe d'`is_groomed` — c'est mika#2120, déjà groomé.
- Le fait que `/mika-groom-ticket` produise plusieurs commits de plan — **pas** un défaut : c'est le lecteur qui doit tolérer ce que son producteur produit.
- **Le faux négatif symétrique** — un pilote tué avant son premier commit laisse du travail non
  commité, invisible au compte de commits comme à la liste `compare` (M7b). Le fermer demanderait un
  état git local que ce module n'a pas. Reste ouvert, hors de ce ticket, et sans sujet tant que
  mika#2141 tient.
- La réparation de `operator-review` (48 lignes `not found` en production) — chemin mika#2020, cité par `REFUSAL_LABEL` et laissé où il est.

## Risques et contre-mesures

| Risque | Contre-mesure |
|---|---|
| **Rendre la porte permissive rouvre la porte qu'elle ferme** (la faute que l'AC2 de mika#2120 nomme sur l'autre garde) | AC2 est un test, pas une intention : `crates/**` + plan sur branche stale → `Refuse`, y compris seuil désactivé. Et la fixture #1680 est un corps réel dont le rebase a **réellement** conflit. |
| Les fixtures existantes sans `files` verdissent les tests pour la mauvaise raison | D7 les ré-enrichit ; D8 renforce l'assertion #1680 pour qu'elle porte sur les fichiers nommés, ce qu'aucune fixture sans `files` ne peut satisfaire. |
| Un test de régression AC3 vacieux (la fixture promeut aussi sous l'ancien code) | Assertion de non-vacuité explicite : `behind_by > 0 && ahead_by > 1` avant la décision. |
| Un préfixe trop étroit refuse une branche de grooming légitime touchant autre chose | **Compté, plus seulement argumenté** (M6, 11 tickets ouverts) : zéro bascule promotion → refus, un seul cas-limite (#1727, un doc hors plan) dont le refus est **surdéterminé** par la distance. Disposition nommée en § Fire-Disposition avec assertion auto-nettoyante, et déclencheur de halte-et-remontée si la mesure ne tient plus à l'implémentation. |
| `changed_files` ajouté à une struct publique casse un site de construction | Trois sites au total dans le dépôt (`:419` déclaration, `:609` parse, `:2545` aide de test) — grep exécuté le 2026-09-03, aucun consommateur hors `auto_pull.rs`. |

## Acceptance criteria

Transcrits verbatim du corps de mika#2140 (`## Critères d'acceptation`). La colonne
de preuve est en § Critères d'acceptation — traçabilité ci-dessous.

- [ ] **AC1** — La porte ne rend `SalvageWorkOnStaleBranch` que si la branche modifie **au moins un fichier hors `docs/plans/`** par rapport à `main`. Une branche dont tous les commits ne touchent que `docs/plans/` n'est jamais classée « travail partiel », quel que soit `ahead_by`.
- [ ] **AC2** — Contrôle négatif, explicite : une branche stale portant **du code** (`crates/**`, `scripts/**`, `.github/**`, `Cargo.toml`…) **plus** un commit de plan est **toujours** refusée. Rendre la porte permissive ne doit pas rouvrir la porte qu'elle existe pour fermer.
- [ ] **AC3** — Test de régression sur les corps réels des deux branches mesurées : `fix/2118/…` et `fix/2120/…` rendent `Promote`, et une fixture dérivée de l'une des branches mortes du 2026-08-31 portant du code rend `Refuse`.
- [ ] **AC4** — Le signal de comparaison utilisé est celui déjà disponible ou une extension minimale : l'appel `compare` qui fournit `behind_by` / `ahead_by` fournit aussi la liste des fichiers. Si la liste est tronquée par l'API ou indisponible, la porte **promeut**.
- [ ] **AC5** — Le message de refus, quand il tire, **nomme les fichiers hors plan** qui ont motivé la décision.
- [ ] **AC6** — Les tickets déjà mal gatés ne restent pas coincés : soit la porte retire `operator-gated` d'elle-même quand la condition ne tient plus, soit le ticket documente explicitement que la levée est manuelle. Choisir et l'écrire.

## Critères d'acceptation — traçabilité

| AC | Livrable | Preuve |
|---|---|---|
| AC1 — refus seulement si ≥1 fichier hors `docs/plans/` | D2, D3 | `test_promotion_gate_multi_commit_plan_only_promotes` ; replays #2118/#2120 |
| AC2 — contrôle négatif : code + plan sur branche stale → refus | D3 | `test_promotion_gate_code_on_stale_branch_still_refuses` (seuil 50 **et** 0) ; replay #1680 |
| AC3 — régression sur les corps réels | D7, D8 | fixtures `2118-…`, `2120-…` → `Promote` (avec non-vacuité) ; `1680-…` → `Refuse` |
| AC4 — liste tronquée/indisponible → promotion | D1, D2, D5 | `test_promotion_gate_missing_file_list_promotes` ; `non_plan_files(None) == []` par construction |
| AC5 — le refus nomme les fichiers hors plan | D4 | `test_salvage_refusal_names_the_offending_files` (+ troncature à 10) |
| Fire-Disposition (mika#1574, F1 première passe) | § Fire-Disposition ; D7, D8 | surface 1 → réparation dans le périmètre ; surface 2 → n=0 mesuré + fixture #1727 auto-nettoyante + déclencheur (c) |
| AC6 — levée choisie et écrite | Décision de conception ; D6 | la levée est **manuelle**, justifiée en trois points, écrite dans le module à côté de `REFUSAL_LABEL` — et l'objection d'idempotence du commentaire 2026-09-02 12:08Z est traitée : le correctif rend la levée idempotente vis-à-vis de l'avancée de `main`, là où elle survivait 63 min avant (M7a) |

## Suite opératoire (hors diff, après merge)

Geste d'opérateur unique, à faire une fois le correctif déployé : retirer `operator-gated` de `#2118` et `#2120`. Les deux branches restent en retard de 8 commits — sous le seuil de 50 — et ne portent que leur plan ; la porte corrigée les promeut alors d'elle-même au tick suivant, sans intervention supplémentaire. Poser `ready` reste, comme toujours, une action d'opérateur.

**#2036 n'est pas dans ce geste, et c'est voulu.** Le correctif lui rend le motif honnête
(`TooFarBehind` au lieu de `salvage`) mais ne le dé-gate pas : 60 commits de retard, au-delà du
seuil de 50. Son remède est celui que la porte nomme déjà — rebaser la branche ou re-groomer sur une
branche neuve — et il appartient à l'opérateur, pas à ce ticket.

## Vérification

```bash
cargo test -p mika-agent auto_pull
cargo test -p mika-agent --test auto_pull_promotion_gate
cargo clippy -p mika-agent --all-targets -- -D warnings
cargo fmt --check
```

Contrôle de non-vacuité à exécuter et à reporter dans le corps de la PR, pas à supposer : rétablir `if staleness.ahead_by > 1` à la place du nouveau prédicat doit faire **rougir** `test_promotion_gate_multi_commit_plan_only_promotes` et les deux replays #2118/#2120 — et laisser vert le replay #1680.
