# mika#2315 — `ready` se ré-arme tout seul : un park opérateur ne tient pas, et le frein de Phase 2 est aveugle au-delà de 100 événements

> Ticket : senara-solutions/mika#2315 (p1 substrat)
> Type : fix
> Plan : `docs/plans/2026-09-15-004-fix-2315-ready-rearme-sur-ticket-parque-plan.md`

## Le symptôme

Sur mika#2295, le label `ready` est **retiré puis remis ~2 s plus tard** par
l'identité machine, à `:29:47Z` chaque heure (observé 02:29:47Z et 03:29:47Z le
2026-09-15). Chaque remise déclenche le `ready_label_handler` → un dev-groom
neuf, cache-cold, ~600k tokens Opus. Cinq grooms ont été produits ainsi
(`73b09112`, `c8267bd5`, `c2b098cd`, `eccbb543`, `b501fdef`) **pendant le STOP P0
mika#2313**, où l'opérateur croyait la boucle à l'arrêt.

Deux choses sont cassées, et elles sont indépendantes. Ce plan les traite comme
telles.

---

## Ce que le code établit

### L'hypothèse du ticket est réfutée

Le ticket suppose le reaper mtime-worktree (`dba4f7e7`, mika#2249). **Ce reaper
ne touche à aucun label.** `git show dba4f7e7 --stat` donne quinze fichiers, dont
aucun ne contient une écriture de label ; son unique mention de `ready` est un
commentaire qui *délègue* explicitement le re-drive :

> `// re-driven, by `stuck_ready_reconcile`, once the ticket is back`
> — `crates/mika-agent/src/task_engine/engine.rs` (ajouté par dba4f7e7)

Le reaper est donc un **maillon** de la chaîne (il libère le verrou `in_flight`,
voir B1 ci-dessous), jamais l'applicateur. L'applicateur est ailleurs, et c'est
le point suivant.

### Inventaire exhaustif des applicateurs de `ready`

Quatre sites, hors tests, appliquent le label `ready` sur ce dépôt :

| # | Site | Rôle |
|---|------|------|
| 1 | `auto_pull.rs:2897` | Phase 0 — auto-feeder (remplit le pool `ready`) |
| 2 | `auto_pull.rs:3055` | Phase 1 — idle pull (promotion quand la queue est vide) |
| 3 | `auto_pull.rs:3384` | Phase 2 — `stuck_ready_reconcile`, **remove→add** |
| 4 | `server/milestone_context_handler.rs:373` | cascade milestone M4 |

Les trois premiers passent par `gh_apply_label` ; le quatrième par
`github_graphql::add_label_to_issue`. **Aucun des quatre ne consulte l'historique
du label avant d'écrire.** Le site 3 est le seul qui produit un remove→add à
quelques secondes d'intervalle : c'est la signature exacte de l'évidence du
ticket.

### B1 — le frein de Phase 2 est inopérant au-delà de 100 événements de timeline

Phase 2 est censée s'auto-brider. Son commentaire le dit
(`auto_pull.rs:3370` env.) :

> *« The remove→add cycle resets the label-age timestamp, so a rescued-but-still-
> undispatched ticket self-throttles for a full threshold window (D3). »*

Ce frein repose entièrement sur `gh_ready_label_age_secs`
(`auto_pull.rs:2560-2592`), qui lit **une seule page** :

```rust
"repos/{}/issues/{}/timeline?per_page=100"
```

puis prend, dans `parse_last_ready_labeled_at` (`auto_pull.rs:2540-2551`), le
dernier `labeled(ready)` de cette page via `.next_back()`.

Le commentaire qui justifie ce plafond se trompe sur le sens de la pagination :

> *« Single page (`per_page=100`) — timeline events beyond the 100th are not
> inspected; a `ready` re-label is near the tail for any recently-touched ticket,
> so this bound is safe in practice. »*

L'API `GET /repos/{o}/{r}/issues/{n}/timeline` renvoie les événements en ordre
**chronologique ascendant**. La page 1 contient donc les 100 événements les plus
**anciens**, pas les plus récents. Sur une issue qui dépasse 100 événements — et
un ticket déjà re-drivé plusieurs fois les dépasse mécaniquement — le
`labeled(ready)` que Phase 2 vient tout juste d'écrire est en page 2 ou au-delà
et **n'est jamais lu**. L'âge renvoyé est celui d'un `labeled` ancien : toujours
≥ threshold, à chaque tick, pour toujours.

Conséquence : **le self-throttle ne freine rien sur exactement la population
qu'il existe pour freiner.** Le frein s'affaiblit à mesure que le ticket est
re-drivé — c'est un emballement auto-renforçant, pas une dégradation gracieuse.

**Pourquoi une heure, et pas dix minutes** (`AUTO_PULL_CRON = "0 */10 * * * *"`,
`server/mod.rs:95`) : entre deux rescues, le ticket lit `in_flight` et Phase 2
saute (`FILTER_IN_FLIGHT`, `classify_stuck_ready`). Le dispatch produit par le
rescue est refusé `already_groomed` et laisse sa tracking row vieillir en
`phantom_aged_out` — mika#2158 mesure cette durée à **~60 min**. À la mort de la
row, `in_flight` retombe, le tick suivant re-drive, et le cycle recommence. La
période observée n'est pas celle d'un cron : c'est **la durée de vie d'une
tracking row orpheline**. Cela explique aussi un `:29:47` qui ne s'aligne sur
aucune frontière de cron.

### B2 — un retrait de `ready` par l'opérateur n'est pas un park

C'est l'exigence littérale du ticket, et elle vise un défaut distinct de B1.

Aujourd'hui, le seul park qui tient est un **label** : `is_feeder_excluded`
(`auto_pull.rs:1714`) exclut `blocked`, `operator-review` et `operator-gated`.
Retirer `ready` n'est pas un park — c'est même l'inverse : un ticket groomé,
non exclu et **sans** `ready` est précisément le candidat que Phase 0 et Phase 1
cherchent (`auto_pull.rs:1425`, `:1835` filtrent sur `!ready`). Un opérateur qui
retire `ready` pour parquer un ticket le rend **immédiatement éligible à la
re-promotion**, au tick suivant, en ≤ 10 minutes.

Le geste de park existant est donc « poser `operator-review` », qu'il faut
connaître. Le geste intuitif — retirer `ready` — produit l'effet contraire de
celui qu'il vise. C'est ce que le ticket appelle « un STOP opérateur ne tient
pas ».

---

## Ce qui reste inféré, et comment le trancher

Cette session n'a pu lire ni `/var/log/mika/server.log` (absent de la machine du
worktree) ni la timeline GitHub (`gh` non authentifié). Trois points sont donc
**déduits du code**, pas mesurés, et chacun a sa sonde :

| Point | Sonde |
|---|---|
| mika#2295 dépasse 100 événements de timeline | `gh api repos/senara-solutions/mika/issues/2295/timeline --paginate --slurp -q 'length'` |
| La période est bien celle de `phantom_aged_out` | `grep phantom_aged_out $MIKA_SPIRIT_LOG_FILE` corrélé aux cinq `stuck_ready_reconciled` sur #2295 |
| Le budget de re-drive (`MAX_REDRIVES_DEFAULT = 3`) n'a pas borné les 5 grooms | `SELECT redrive_count, redrive_abandoned_at FROM auto_pull_stats WHERE issue_number = 2295` |

La troisième mérite un mot. Cinq rescues avec un budget de trois signifie que le
compteur n'a pas avancé. Trois causes possibles, non départageables d'ici :
`MIKA_AUTO_PULL_MAX_REDRIVES=0` en production (le sentinel qui désactive le
budget), un `increment_auto_pull_redrive` en échec — il n'est qu'un `warn!` que
la boucle ignore (`auto_pull.rs:3400` env.) —, ou un reset par
`SkipAndResetBudget`. **La réparation portée par ce plan ne dépend pas de la
réponse** : B1 supprime la source des rescues, B2 rend le park effectif, et le
budget reste ce qu'il est — une seconde barrière. La sonde est un AC de
déploiement, pas un préalable à l'implémentation. Si elle révèle un compteur
non écrivable, c'est la classe déjà nommée pour les callbacks par mika#2179
(`callback_delivery_counter_unwritable`) et elle mérite son propre ticket.

---

## Décisions

**D1 — Un applicateur canonique unique pour `ready`, et un seul.**
Nouveau module `crates/mika-agent/src/ready_label.rs`, propriétaire exclusif de
l'écriture du label `ready`. Les quatre sites de l'inventaire l'appellent ; aucun
n'écrit `ready` en direct.

Pourquoi pas un simple correctif sur Phase 2 : l'exigence du ticket dit « le
reaper **ou tout re-dispatch auto** ». Un quatrième producteur vit hors
d'`auto_pull` (cascade milestone) et personne ne penserait à le patcher en
traitant ce symptôme-ci. Le repo a déjà payé cette leçon deux fois — la regex de
grooming dupliquée entre `auto_pull` et `executor` qui a divergé des mois en
silence (mika#2158), et l'accesseur étroit à côté du résolveur canonique
(mika#2205, `docs/solutions/architecture-patterns/2026-09-06-accesseur-etroit-a-cote-du-resolveur-canonique.md`).
Modèle retenu : celui de `grooming_marker.rs` — un seul lecteur, plus un test de
scan de source qui refuse l'apparition d'un second. Un test de comportement ne
peut pas attraper cette classe : un cinquième applicateur écrit demain ne rendrait
aucune décision fausse, il la rendrait **non gardée**, et toutes les assertions
existantes resteraient vertes.

**D2 — Le prédicat de park se lit sur la timeline, sur le dernier événement.**

```
parked(issue) :=
  E := événements de timeline dont label.name == "ready"
       et event ∈ {"labeled", "unlabeled"}, ordonnés par created_at
  E vide                                  → non parqué
  dernier(E).event == "labeled"           → non parqué
  dernier(E).actor ∈ identités_machine    → non parqué
  sinon                                   → PARQUÉ
```

`apply_ready()` refuse quand `parked(issue)`.

Les trois propriétés qui rendent ce prédicat correct :

- **Le remove→add de Phase 2 ne se parque pas lui-même.** Entre son `remove` et
  son `add`, le dernier événement est un `unlabeled` **machine** → non parqué.
- **La sortie de park est un geste unique et symétrique** : l'opérateur remet
  `ready` à la main. Le dernier événement devient `labeled` → non parqué. Rien à
  retirer, aucun label à connaître, aucune documentation à avoir lue.
- **Les retraits machine légitimes restent des non-parks** :
  `draft_pr_opened_handler` (retire `ready` à l'ouverture d'une PR draft) et
  `abandon_stuck_ready` (retire `ready`, pose `operator-review`). Le premier
  reste couvert par le filtre `has_open_pr`, le second par `is_feeder_excluded` :
  ni l'un ni l'autre ne perd sa protection.

**D3 — Les identités machine sont déclarées, pas devinées.**
Ensemble résolu une fois par process et mis en cache, depuis trois sources
cumulatives : `MIKA_GITHUB_APP_LOGIN` (variable déjà existante), le login du
token courant (`gh api user -q .login`), et `MIKA_LOOP_BOT_LOGINS` (liste séparée
par virgules, pour les identités qu'aucune des deux premières ne révèle).
Comparaison insensible à la casse, après retrait du suffixe `[bot]` : GitHub
renvoie `mika-platform-dev[bot]` là où la configuration porte
`mika-platform-dev`, et l'évidence du ticket cite les deux formes.

**Ensemble vide → fail-closed** (aucune application de `ready`), avec un WARN
nommant la cause. Sans savoir qui est la machine, tout retrait ressemble à un
park, et appliquer `ready` reviendrait à restaurer le bug en entier, en silence.
Le coût est réel et nommé : une machine mal configurée voit sa boucle s'arrêter.
Il est borné par deux sources de résolution indépendantes et par un signal
opérateur explicite — et c'est la même asymétrie que le repo tranche déjà dans le
même sens ailleurs (`assert_family_tier_env_consistency` refuse de démarrer
plutôt que de servir la mauvaise politique).

**D4 — Timeline illisible → refus, pas application.**
Erreur API, pagination incomplète, JSON illisible : `apply_ready` refuse et
laisse le tick suivant réessayer. L'asymétrie est mesurée : un faux négatif coûte
**10 minutes de latence** (un tick), un faux positif coûte **~600k tokens Opus**
et casse un STOP opérateur. Ce n'est pas le même ordre de grandeur, et le choix
suit la mesure.

Noter que c'est l'**inverse** du fail-open retenu ailleurs dans le même module
(`gh_list_open_pr_closing_issues` échoue en ensemble vide « to preserve pre-fix
behavior on infra glitches »). Diverger ici est délibéré : ce fail-open-là élargit
un filtre dont l'échec fait *rater* une exclusion ; celui-ci gouverne une écriture
dont l'échec fait *agir*.

**D5 — La timeline se lit en entier, avec un plafond.**
`--paginate` jusqu'à la dernière page, plafonné à `MAX_TIMELINE_PAGES` (20, soit
2000 événements). Au-delà, refus (D4). Le plafond existe pour que le prédicat ne
devienne jamais un amplificateur d'appels API sur un ticket pathologique.

Cette lecture **corrige incidemment B1** : `gh_ready_label_age_secs` et le
prédicat de park consomment désormais **une seule lecture de timeline**, complète
et partagée. L'âge devient exact, le self-throttle redevient effectif, et Phase 2
ne paie **aucun appel supplémentaire** (elle en faisait déjà un). Phase 0, Phase 1
et la cascade milestone en paient un chacune par candidat promu, soit ≤ 9 appels
par tick de 10 min (≤ 54/h), contre un plafond GitHub de 5000/h en PAT et
15000/h en App.

**D6 — Le budget de re-drive n'est pas touché.**
`MAX_REDRIVES_DEFAULT` et son ledger restent inchangés. Ce plan supprime la
source des rescues non bornés (B1) et rend le park effectif (B2) ; le budget reste
la seconde barrière, pour le ticket qui est légitimement re-drivé et ne produit
rien. Le modifier dans le même geste mélangerait deux bornages dont on ne saurait
plus lequel a tenu.

---

## Le changement

**Nouveau — `crates/mika-agent/src/ready_label.rs`**

- `pub(crate) async fn apply_ready(...) -> ReadyApplyOutcome` — l'unique écriture
  de `ready`. Consulte le park, puis délègue l'écriture.
- `pub(crate) fn is_parked(events: &[TimelineEvent], machine: &MachineIdentities) -> bool`
  — le prédicat D2, **pur**, entièrement testable sans réseau.
- `pub(crate) async fn read_ready_label_timeline(...)` — lecture paginée D5,
  partagée avec le calcul d'âge.
- `machine_identities()` — résolution D3, cache process (`OnceCell`).
- `ReadyApplyOutcome { Applied, RefusedParked, RefusedUnreadable }` — le refus est
  une valeur retournée, jamais un échec avalé (leçon mika#2199 : un `gh` en échec
  silencieux a re-élu la même PR dix-sept fois).

**Modifié**

- `auto_pull.rs` — les trois `gh_apply_label(.., "ready")` (`:2897`, `:3055`,
  `:3384`) passent par `apply_ready`. `gh_ready_label_age_secs` consomme la
  lecture partagée au lieu de sa page unique ; son commentaire erroné sur
  l'ordre de pagination est corrigé.
- `server/milestone_context_handler.rs:373` — `add_label_to_issue(.., "ready")`
  passe par `apply_ready`.

**Télémétrie** (les noms sont un format de fil : un site d'écriture chacun)

- `ready_apply_refused_parked` — WARN + `audit_events` (`tool_name`
  = `ready_label_park`, `target_key` = `issue:<n>`, `after_value` = phase
  appelante). C'est la ligne qui répond à « pourquoi ce ticket n'est-il plus
  promu ? ».
- `ready_apply_refused_unreadable` — WARN. **Attendu à zéro.** Une série
  soutenue signifie que la boucle est bridée par un problème d'API, pas par un
  park.
- `ready_machine_identities_unresolved` — WARN au démarrage, fail-closed D3.

---

## Contrat de vérification

Tests unitaires purs sur `is_parked`, sur des timelines fixées :

1. dernier = `unlabeled` par un humain → parqué
2. dernier = `unlabeled` par la machine (remove→add en cours) → non parqué
3. dernier = `labeled` par un humain (re-armement manuel = sortie de park) → non parqué
4. dernier = `labeled` par la machine → non parqué
5. aucun événement `ready` → non parqué
6. `unlabeled` humain **puis** `labeled` humain → non parqué (l'ordre décide, pas la présence)
7. `labeled` humain **puis** `unlabeled` humain → parqué
8. acteur `mika-platform-dev[bot]` contre configuration `mika-platform-dev` → reconnu machine
9. acteur en casse différente → reconnu machine
10. événements d'un **autre** label (`blocked`) intercalés → ignorés

Tests de pagination :

11. timeline > 100 événements : le `labeled(ready)` de la dernière page est celui
    qui décide — **la régression B1 nommée, épinglée** ;
12. plafond `MAX_TIMELINE_PAGES` atteint → `RefusedUnreadable`, jamais `Applied`.

Tests structurels :

13. `ready_label::tests::no_ready_label_write_outside_this_module` — scan de
    source refusant tout `"ready"` passé à `gh_apply_label` / `add_label_to_issue`
    hors du module (garde D1) ;
14. `machine_identities` vide → `apply_ready` refuse (garde D3).

Test d'intégration :

15. Rejeu du scénario mika#2295 : un ticket dont `ready` a été retiré par un
    humain traverse les trois phases d'un tick complet d'`auto_pull` sans qu'aucune
    ne le re-promeuve, et le ledger d'exclusion le nomme.

---

## Risques et coûts, nommés

- **Un opérateur agissant via le PAT de la machine sera classé « machine ».** Son
  retrait de `ready` ne parquera pas. C'est le prix du prédicat par acteur ; la
  parade est d'agir sous son propre compte, ou de poser `operator-review`, qui
  reste le park explicite.
- **Fail-closed sur identités non résolues** arrête la promotion. Mitigé par deux
  sources indépendantes et un WARN nommé au démarrage.
- **Un ticket parqué avant le déploiement** (dernier événement = `unlabeled`
  humain) devient parqué rétroactivement. C'est le comportement voulu, mais il
  peut surprendre : des tickets cessent d'être promus sans qu'un geste ait été
  fait. Le WARN `ready_apply_refused_parked` les nomme un par un, et la sortie de
  park est d'un seul geste.
- **La timeline est une lecture réseau sur un chemin qui n'en avait pas** (Phase 0,
  Phase 1, cascade milestone). Volume borné et chiffré en D5.

---

## Hors périmètre, délibérément

- **Le STOP global hot-swappable.** `MIKA_DEV_AUTO_PULL=0` n'est lu qu'une fois,
  dans `init_agent` (`server/mod.rs:1581`) : le couper en pleine incidence exige
  un redémarrage de mika-spirit. C'est un second sens de « le STOP ne tient pas »,
  réel et adjacent, mais c'est un autre mécanisme (un interrupteur lu à chaque
  tick) avec un autre rayon d'action. À ouvrir en ticket de suivi.
- **La cause du refus `already_groomed`** qui laisse une tracking row vieillir
  ~60 min en `phantom_aged_out` — la période du cycle. Ce plan supprime l'entrée
  du cycle, pas le mécanisme qui en fixe le pas (lignée mika#2158 / mika#1934).
- **Le compteur de re-drive**, voir D6 et la sonde de déploiement.
- **`gh_list_open_pr_closing_issues` et son fail-open** : divergence assumée et
  argumentée en D4, pas un oubli.

---

## Acceptance criteria

**AC1** — Aucun chemin automatique n'applique le label `ready` à une issue dont
le dernier événement de timeline portant ce label est un `unlabeled` par un
acteur hors de l'ensemble des identités machine. Vérifié sur les quatre
applicateurs de l'inventaire.

**AC2** — Un opérateur qui retire `ready` à la main parque le ticket sans avoir à
poser le moindre autre label ; le ticket reste hors de Phase 0, Phase 1 et
Phase 2 aussi longtemps que le retrait n'est pas annulé.

**AC3** — Remettre `ready` à la main sort le ticket du park, en un seul geste et
sans redémarrage.

**AC4** — Le remove→add de Phase 2 ne se parque pas lui-même : un rescue légitime
sur un ticket non parqué aboutit toujours.

**AC5** — `gh_ready_label_age_secs` renvoie l'âge du **dernier** `labeled(ready)`
de la timeline complète, y compris au-delà du centième événement. Test 11
épingle la régression.

**AC6** — Toute écriture du label `ready` passe par `ready_label::apply_ready`.
Un test de scan de source échoue si un second site d'écriture apparaît.

**AC7** — Une timeline illisible, une pagination incomplète ou un ensemble
d'identités machine vide produisent un refus d'appliquer `ready`, jamais une
application. Chacun émet son signal opérateur distinct.

**AC8** — Chaque refus pour cause de park écrit une ligne WARN et une ligne
`audit_events` nommant l'issue et la phase appelante.

**AC9** — Non-régression : les retraits machine légitimes
(`draft_pr_opened_handler`, `abandon_stuck_ready`) ne sont pas classés park, et
les protections qui les couvrent (`has_open_pr`, `is_feeder_excluded`) restent
intactes.

---

## Definition of Done

- [ ] `crates/mika-agent/src/ready_label.rs` créé ; les quatre applicateurs de
      l'inventaire y passent ; aucune écriture directe de `ready` ne subsiste.
- [ ] Les quinze tests du contrat de vérification passent, dont les deux gardes
      structurelles (13, 14) et l'épinglage de la régression de pagination (11).
- [ ] `cargo test`, `cargo clippy` et `cargo fmt --check` verts.
- [ ] `make verify-bundled-skills` vert (aucun bundle touché, gate de non-régression).
- [ ] Les trois signaux de télémétrie sont documentés dans `CLAUDE.md`
      (section « Optional (auto-pull… ) ») avec leur valeur attendue en régime
      nominal.
- [ ] `MIKA_LOOP_BOT_LOGINS` documentée dans `CLAUDE.md` et `.env.example`.
- [ ] Le corps de PR porte les trois sondes de la section « Ce qui reste inféré »
      comme vérifications post-déploiement, avec leur critère de lecture.
- [ ] Le ticket de suivi « STOP global hot-swappable » est ouvert et référencé
      (`Tracked in:`) dans le corps de PR.
