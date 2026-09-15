# Plan : la revue d'une PR ne doit pas dépendre d'un événement unique et perdable (mika#2334)

**Ticket :** mika issue#2334 — `la demande de revue est sautée quand le pilote meurt post-push`
**Labels :** non lus — `gh` n'est pas authentifié dans le worktree de grooming ; le corps et le commentaire opérateur ont été fournis intégralement dans le brief de dispatch.
**Type :** issue (bug de substrat de boucle — le livrable existe, la revue n'est jamais demandée)
**Palier de priorité :** Tier 2 — *dégrade la boucle sans la casser*. Deux PRs sur deux du drain du 2026-09-15 ont exigé un geste opérateur manuel pour démarrer leur revue.
**Fix retenu par l'opérateur (samidarko, 2026-09-15) :** option 1 du corps — la revue ne doit pas dépendre de la survie du pilote aux pas fragiles.
**Fichiers principaux :** `crates/mika-agent/src/qa_review_reconcile.rs` (nouveau), `crates/mika-agent/src/task_engine/dispatcher.rs`, `crates/mika-agent/src/lib.rs`, `crates/mika-agent/tests/eval/test_qa_review_reconcile_2334.rs` (nouveau), `CLAUDE.md`, `crates/mika-agent/CLAUDE.md`

---

## Problème

Deux PRs du drain du 2026-09-15 (#2332 issue #2296, #2333 issue #2293) sont restées ouvertes sans revue. L'orchestrateur a posé `mika-platform-qa` à la main pour débloquer. Les deux pilotes sont morts après le push, par deux fins distinctes : `error_during_execution:after_deny` dans `/ce-code-review` pour #2293, recovery dirty-worktree mika#1282 pour #2296.

Le ticket en déduit qu'un pas trailing `gh pr edit --add-reviewer` a été sauté, et demande de le déplacer avant les pas fragiles.

## Mesures — exécutées le 2026-09-15 dans le worktree de grooming

Quatre faits lus dans le code, qui déplacent le diagnostic.

**M1 — le pas trailing n'existe pas.** Aucun site du dépôt ne pose de reviewer sur une PR. La recherche exhaustive de `--add-reviewer`, `gh api …/requested_reviewers` et `gh pr review --request` sur `.claude/commands/`, `skills/bundled/`, `crates/`, `scripts/` et `.github/` ne rend qu'une occurrence, et c'est une table d'arité de flags : `crates/mika-agent/src/skills/builtin_handlers.rs:2432` (garde wip-rescue mika#1682). Les huit `gh pr edit` de `dispatch-lib.sh` ne portent que `--add-label`, `--title`, `--body`. **Il n'y a pas de pas à déplacer avant les pas fragiles : aucune PR de la boucle n'a jamais porté de reviewer.**

**M2 — la revue est déclenchée par webhook, indépendamment du pilote.** `crates/mika-gateway/src/github.rs:336-338` route `pull_request.opened` vers mika-qa, sans filtre draft. La création de la PR suffit donc à démarrer la cascade : **la mort du pilote après le push ne peut pas, à elle seule, empêcher la revue.** C'est la prémisse causale du ticket qui ne tient pas.

**M3 — `opened` est un événement unique, non rejouable, et perdable en quatre endroits.** C'est là qu'est le défaut réel.
- Côté agent : la file bornée mika#1870 ne coalesce jamais un `opened` (`crates/mika-agent/src/server/webhook_queue_v2.rs:194-197`) mais le jette en tête de file à saturation (drop-oldest, `MIKA_WEBHOOK_QUEUE_MAX_DEPTH` = 64) ; le handler répond alors 429 (`server/handlers.rs:358-372`).
- Côté gateway : au 3ᵉ 429 consécutif le circuit breaker ouvre et la livraison part directement en DLQ (`crates/mika-gateway/src/circuit_breaker.rs:21-31`) ; après 10 tentatives le statut passe `dead` et **seul un rejeu manuel** le ressort (`crates/mika-gateway/src/dlq.rs:110-113`).
- Côté tour LLM : `webhook_zero_tools` n'est opposé qu'une fois ; au second tour vide l'EndTurn passe et la revue n'est jamais postée.
- **Aucun chemin ne relit une PR ouverte sans revue.** Les trois scans récurrents (`dispatcher.rs:442-444` : `auto_pull_groomed`, `wip_rescue`, `curator_review`) ne couvrent pas ce cas : `auto_pull` travaille sur les *issues*, `wip_rescue` ne voit que les drafts portant le label `wip-rescue`, `curator_review` porte sur les skills. Le seul rattrapage existant est le fan-out `check_suite.completed(success)` → mika-qa (mika#1711), qui exige `draft: false` **et** une CI verte (`skills/bundled/qa-review-webhook-success/system_prompt.md:19-24`) : une PR dont la CI est rouge ou n'a jamais tourné n'est jamais rattrapée.

**M4 — les deux PRs de l'incident relèvent probablement de deux causes différentes.** #2332 vient de la voie recovery mika#1282 : PR ouverte en `--draft` avec `<!-- rescue-pipeline-verified: no -->` (`dispatch-lib.sh:6395-6400`, `_compose_rescue_pr_body:5654-5690`). Pour celle-là, la revue **a pu démarrer et se terminer en `hold[review]`** au Step 1.5 de qa-review (`skills/bundled/qa-review/system_prompt.md:125-132`) — c'est le maillon 2 que le commentaire opérateur décrit, pas une revue jamais demandée. #2333 est une PR non-draft : son `opened` aurait dû suffire, ce qui désigne M3. La distinction n'est pas vérifiable depuis le worktree (`gh` non authentifié) — d'où AC0 ci-dessous, préalable bloquant.

## Rectification apportée à la direction du ticket

Le ticket demande de déplacer la demande de revue avant les pas fragiles. M1 et M2 rendent ce geste sans objet : il n'y a rien à déplacer, et la survie du pilote n'est pas ce dont la revue dépend. Ce que l'incident révèle est plus large et plus grave que ce que le ticket énonce : **la revue de toute PR de la boucle repose sur un unique événement webhook qu'aucun mécanisme ne rejoue.**

Le corps du ticket contient déjà la bonne formulation, en seconde branche de son option 1 : *« ou de façon indépendante (un handler qui, sur draft→ready ou PR ouverte sans reviewer, pose mika-qa) »* — et son test négatif l'exige nommément : *« la PR a QUAND MÊME un reviewer mika-qa demandé (via un handler indépendant) »*. Le commentaire opérateur privilégie la première branche (« AVANT »), mais son intention est explicite et c'est elle qui est honorée ici : *« même si le pilote meurt dans ces pas fragiles, la revue est déjà demandée → la cascade QA démarre sans intervention manuelle »*.

**Pourquoi la lettre (« AVANT, dans le pipeline dev-pilot ») n'est pas retenue — une mesure, pas une préférence.** Un geste inconditionnel posé à la création de la PR produit une **revue en double sur chaque PR de la boucle**, pas seulement sur les PRs sinistrées :

1. `gh pr create` émet `opened` → mika-qa démarre une session de revue.
2. Le geste pose le reviewer quelques secondes plus tard → `review_requested`, non supprimé puisque le reviewer est bien `mika-platform-qa` (`github.rs:312-314`) → mika-qa démarre une **seconde** session.
3. La seconde session ne peut pas constater que la première a déjà revu : `gh pr view` est hors du périmètre outillé de qa-review (`QA_REVIEW_GH_ALLOWED`, `builtin_handlers.rs:1935-1940`), et la première revue n'est de toute façon pas encore postée quand le second événement est mis en file.

Le doublon serait donc le cas nominal, pas l'exception. C'est précisément la classe de défaut que #886 a déjà dû fermer une fois (garde synchronize no-diff, motif : *« Prevents cross-session duplicate APPROVED reviews »*). Doubler le coût QA de chaque PR pour couvrir une perte de webhook rare est un mauvais échange.

Deux corollaires en découlent, et ils décident la conception :
- **Le geste doit être conditionnel à l'absence de revue.** Poser le reviewer n'a de valeur que lorsque rien n'est venu.
- **Il ne peut donc pas vivre dans `dispatch-lib.sh`.** Le tail shell livre son callback et meurt en quelques secondes ; il ne peut pas constater une absence qui ne se mesure qu'après un délai. Un ancrage propre y existe pourtant (`_post_flight_recovery:3649-3670`, où `PR_URL` est résolue et `_stamp_pr_origin` déjà appelé) — il est écarté pour cette raison seule, et non par difficulté.

Le lieu correct est un scan périodique, de la même famille que `auto_pull` et `wip_rescue` : hors LLM, hors session pilote, déclenché par le temps et non par un événement. Il satisfait le test négatif à la lettre et couvre en plus les trois pertes de M3, qui ne sont pas des morts de pilote.

Ce choix suit aussi la doctrine du dépôt : `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` proscrit l'application par prompt sur le substrat de boucle, et mika#2120 en porte la mesure (9 récurrences sous prompt contre 0 quand l'opérateur l'écrivait à la main). Un pas ajouté à `.claude/commands/mika.md` hériterait exactement de la fragilité que le ticket dénonce.

## Conception

### Brique 0 — attribution (préalable bloquant, aucun code)

Établir laquelle des causes de M3/M4 a produit l'incident, avant de poser le remède. Une seule commande, exécutée par l'opérateur sur un shell authentifié :

```bash
gh pr list --repo senara-solutions/mika --state open \
  --json number,isDraft,author,createdAt,reviewRequests,reviews,headRefOid \
  --limit 100 \
  | jq '[.[] | select(.author.login == "mika-platform-dev")
        | {number, isDraft, createdAt,
           requested: [.reviewRequests[].login],
           reviewed_by: [.reviews[].author.login] | unique}]'
```

Ce relevé donne la taille réelle de la population « PR de la boucle, ouverte, ni revue ni demandée » — qui est aussi la charge du premier tick après déploiement (voir la borne d'âge haute, brique 1).

### Brique 1 — le réconciliateur `qa_review_reconcile`

Nouveau module `crates/mika-agent/src/qa_review_reconcile.rs`, câblé comme quatrième scan récurrent à côté de `auto_pull_groomed` et `wip_rescue` dans `crates/mika-agent/src/task_engine/dispatcher.rs:442-444`. Porté par mika-dev, comme ses deux voisins.

**Population retenue** — conjonction, chaque terme fail-safe (une information illisible sort la PR de la population, ne l'y fait jamais entrer) :

| Terme | Raison |
|---|---|
| `state == open` | — |
| `author.login == mika-platform-dev` | une PR humaine n'est pas la boucle et ne se fait pas poser un reviewer par elle |
| `isDraft == false` | les drafts ont leur propre voie (`wip_rescue` → `gh pr ready` → `ready_for_review`, mika#1822) ; et une PR rescue draft est délibérément tenue par `RECOVERY_PENDING: true` |
| `reviewRequests` ne contient pas `mika-platform-qa` | idempotence : une demande déjà posée sort la PR de la population |
| aucune `review` de `mika-platform-qa` | GitHub retire la demande quand la revue est soumise ; sans ce terme, chaque PR revue serait re-demandée en boucle |
| `now − createdAt > MIN_AGE` | laisse au chemin nominal le temps d'aboutir — c'est ce terme qui évite le doublon |
| `now − createdAt < MAX_AGE` | une PR de plusieurs jours sans revue n'est pas un webhook perdu mais une PR abandonnée ; la réveiller n'aide personne |

**Action** : `gh pr edit <n> --repo <repo> --add-reviewer mika-platform-qa`. Le webhook `review_requested` qui en résulte passe `is_suppressed_review_request` (`github.rs:312-314`) puisque le reviewer est exactement `QA_REVIEWER_LOGIN` — le chemin de déclenchement existe déjà et est testé, rien n'est à ajouter côté gateway.

**Découpe pour la testabilité.** Une fonction pure `select_prs_needing_review(prs: &[PrSnapshot], now: DateTime<Utc>, cfg: &ReconcileConfig) -> Vec<PrRef>` porte toute la décision ; l'exécution `gh` est un appelant mince. C'est cette fonction que le test négatif interroge, sans réseau.

**Authentification — à faire correctement du premier coup.** Le résolveur canonique `Settings::resolve_github_token` (PAT-first / App-fallback), jamais `self.github_token`. mika#2205 a mesuré ce que coûte l'accesseur étroit : le 2026-09-05, `auto_pull` et `wip_rescue` sont morts à la même seconde quand le PAT a disparu de l'environnement, alors que le chemin App était sain. Le test structurel `dispatcher::tests::mika2205_periodic_scans_do_not_read_the_pat_field_directly` doit couvrir ce nouveau scan. Poser un reviewer ne fait pas partie des opérations dont GitHub lit l'auteur de façon critique (ADR-008), donc le repli App est légitime ici — au même titre que la bascule de label d'`auto_pull`.

**Bornes** — le premier tick après déploiement voit toute l'arriération d'un coup, et c'est le seul moment où ce scan peut faire du bruit :
- cap par tick (défaut 3), oldest-first, pour étaler ;
- borne d'âge haute (défaut 7 jours) ;
- un seul `gh pr list` par repo et par tick, donc coût API constant.

**Configuration** (trois paliers : absent/vide → défaut ; illisible, `0` ou négatif → défaut + WARN) :

| Variable | Défaut | Rôle |
|---|---|---|
| `MIKA_QA_REVIEW_RECONCILE` | `1` | kill-switch ; `0` annule la tâche récurrente, comme `MIKA_DEV_WIP_RESCUE` |
| `MIKA_QA_REVIEW_RECONCILE_MIN_AGE_SECS` | `3600` | délai avant de conclure à l'absence de revue |
| `MIKA_QA_REVIEW_RECONCILE_MAX_AGE_SECS` | `604800` | au-delà, PR abandonnée |
| `MIKA_QA_REVIEW_RECONCILE_MAX_PER_TICK` | `3` | étalement du premier tick |
| `MIKA_QA_REVIEW_RECONCILE_REPOS` | `senara-solutions/mika` | liste séparée par virgules ; l'extension à mika-cloud est un geste d'une ligne, délibérément non anticipé |

**Le défaut de `MIN_AGE` est un arbitrage asymétrique, pas une rondeur.** Trop court, il recrée le doublon que toute cette conception existe pour éviter ; trop long, la PR attend son rattrapage — alors qu'aujourd'hui elle attend indéfiniment. Le coût d'une heure d'attente est donc très inférieur au coût d'une revue en double sur chaque PR. Une heure couvre largement l'enveloppe d'un tour (`AGENT_TOTAL_TIMEOUT_SECS` = 300 s) et une revue passée par le callback de build. Risque résiduel nommé : une session qa-review en cours depuis plus d'une heure sans avoir rien posté verrait un reviewer posé par-dessus — rare, et borné par les deux termes d'idempotence.

Cadence : `0 */15 * * * *`, fond de file, à l'image de `wip_rescue`.

### Brique 2 — observabilité

- Un `audit_events` par PR réconciliée : `tool_name = 'qa_review_reconciled'`, `target_key = 'pr:<repo>#<n>'`.
- Un INFO `qa_review_reconciled` par PR (le volume est borné par le cap : au plus 3 par tick).
- Un INFO agrégé `qa_review_reconcile_tick` **seulement quand le tick agit** — zéro action, zéro ligne (doctrine mika#2131 : un scan qui journalise tout le monde ne distingue plus personne, et un détail par candidat ne descend jamais en `debug!`, que le filtre de ce serveur ne collecte pas).
- Un WARN `qa_review_reconcile_no_token` quand aucun jeton ne se résout — mika#2205 : un scan silencieusement inactif se lit exactement comme un scan qui n'a rien trouvé à faire.
- Un WARN `qa_review_request_failed` quand `gh pr edit` échoue. Doit rester vide en régime nominal ; l'écriture `requested_reviewers` est de la même famille que l'écriture de label, qui échoue déjà sous PAT (`Resource not accessible by personal access token`, mika#2228) — d'où un événement nommé plutôt qu'une ligne d'erreur générique.

Fail-open de bout en bout : aucun échec de ce scan ne doit faire échouer un tick du moteur.

### Brique 3 — accord sur le login du relecteur

`mika-platform-qa` est défini une fois, en Rust : `crates/mika-common/src/forge_identity.rs:48-55` (`REVIEWER_FORGE_LOGIN`), et le gateway l'importe au lieu de le redéfinir (`github.rs:247`). Le réconciliateur étant écrit en Rust, il l'importe également — **aucune nouvelle écriture du littéral n'est introduite**, et la divergence de constante qui aurait guetté une implémentation shell est évitée par construction. C'est la seconde raison, après le conditionnement, pour laquelle le lieu Rust est le bon.

## Fire-Disposition

- **AC0 — attribution (opérateur, préalable bloquant).** Tir sur : le relevé de la brique 0. Disposition : **halt-and-surface**. Si #2333 apparaît avec une revue de `mika-platform-qa` déjà postée, la cause n'est pas M3 mais le maillon 2 (faux-étiquetage rescue) : ne pas poser la brique 1, écrire la mesure dans mika#2334 et re-groomer sur le maillon 2. Aucune remédiation automatique.
- **AC1 — sélection (CI).** Tir sur : le diff / la CI. Disposition : **gate CI bloquant**. La fonction pure retient la PR du test négatif et écarte les six formes voisines (draft, auteur humain, déjà demandée, déjà revue, trop jeune, trop vieille).
- **AC2 — pas de doublon (CI).** Tir sur : le diff / la CI. Disposition : **gate CI bloquant**. Une PR portant déjà une demande ou une revue de `mika-platform-qa` n'est jamais retenue, quel que soit son âge.
- **AC3 — résolution du jeton (CI, structurel).** Tir sur : `mika2205_periodic_scans_do_not_read_the_pat_field_directly` étendu au nouveau scan. Disposition : **gate CI bloquant**. Un test comportemental resterait vert pendant toute une panne de PAT : le défaut de mika#2205 n'était pas une mauvaise résolution mais un appelant qui ne résolvait pas.
- **AC4 — sonde post-déploiement, volume (opérateur).** Tir sur : `grep qa_review_reconciled` sur les 48 h suivant le déploiement. Disposition : **halt-and-surface**. Régime nominal attendu : l'arriération de la brique 0 absorbée sur les premiers ticks, puis **≤ 1 par jour**. Un volume soutenu supérieur signifie que `opened` se perd systématiquement et que ce scan masque une panne amont (file webhook saturée, DLQ) : c'est cette panne qu'il faut alors traiter, pas ce seuil qu'il faut ajuster.
- **AC5 — sonde post-déploiement, revues en double (opérateur, signal négatif).** Tir sur : deux revues de `mika-platform-qa` au même `headRefOid` sur une PR touchée par le scan. Disposition : **halt-and-surface**, désarmer par `MIKA_QA_REVIEW_RECONCILE=0`. Ne pas rallonger `MIN_AGE` en réflexe : un doublon prouve que le conditionnement lui-même est troué, et c'est lui qu'il faut réparer.
- **Non-action par défaut.** Hors de ces signaux, rien : aucune pose de reviewer sur un draft, aucun réveil de PR hors fenêtre d'âge, aucune retouche de `.claude/commands/mika.md`, aucun geste dans `dispatch-lib.sh`.

## Definition of Done

- Le relevé AC0 est fait et écrit dans mika#2334 **avant** que la brique 1 ne soit posée.
- `qa_review_reconcile` est câblé comme tâche récurrente, désactivable par une variable d'environnement, et n'ouvre qu'un `gh pr list` par repo et par tick.
- La décision est portée par une fonction pure testée sans réseau.
- Le jeton passe par `Settings::resolve_github_token`, et le test structurel mika#2205 couvre le nouveau scan.
- `cargo build`, `cargo clippy`, `cargo fmt --check`, `cargo test -p mika-agent` et `make verify-bundled-skills` passent.
- `CLAUDE.md` documente les cinq variables et les cinq signaux opérateur, dans la forme des sections voisines.
- **Aucun fichier de `skills/bundled/` ni de `.claude/commands/` n'est modifié** — l'absence de diff y est un résultat du plan, pas un oubli.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria`. Les critères ci-dessous dérivent de son test négatif, énoncé littéralement, et de la conception ci-dessus.

- [ ] **AC1 — le test négatif du ticket passe.** Une PR ouverte, non-draft, d'auteur `mika-platform-dev`, sans demande ni revue de `mika-platform-qa`, plus âgée que `MIN_AGE` et plus jeune que `MAX_AGE`, est retenue par `select_prs_needing_review` — quelle que soit la façon dont le pilote qui l'a produite s'est terminé (`after_deny`, dirty-worktree, timeout). La terminaison du pilote n'est pas une entrée de la fonction : c'est la forme structurelle de « indépendant de la survie du pilote ».
- [ ] **AC2 — aucune revue en double.** Une PR portant déjà `mika-platform-qa` dans `reviewRequests`, ou portant déjà une revue de `mika-platform-qa`, n'est jamais retenue.
- [ ] **AC3 — les drafts sont hors population.** Une PR `isDraft: true` n'est jamais retenue, y compris une PR de rescue portant `<!-- rescue-pipeline-verified: no -->`.
- [ ] **AC4 — les PRs humaines sont hors population.** Une PR dont l'auteur n'est pas `mika-platform-dev` n'est jamais retenue.
- [ ] **AC5 — la fenêtre d'âge est respectée des deux côtés.** Une PR plus jeune que `MIN_AGE` et une PR plus vieille que `MAX_AGE` sont toutes deux écartées.
- [ ] **AC6 — le premier tick est borné.** Avec N candidats et un cap de M, au plus M PRs sont traitées par tick, oldest-first.
- [ ] **AC7 — le scan est désactivable et résout son jeton comme ses voisins.** `MIKA_QA_REVIEW_RECONCILE=0` annule la tâche ; le jeton passe par `Settings::resolve_github_token` et le test structurel mika#2205 le pin.
- [ ] **AC8 — l'action est observable.** Chaque pose écrit un `audit_events` `qa_review_reconciled` et une ligne INFO ; un tick sans action n'écrit rien ; l'absence de jeton écrit un WARN nommé.
- [ ] **AC9 — le signal est branché de bout en bout.** Le `review_requested` produit cible exactement `REVIEWER_FORGE_LOGIN`, donc survit à `is_suppressed_review_request` et atteint mika-qa. Assis par un test du côté gateway ou par citation du test existant `github.rs:1850`.

## Rattachement aux critères d'acceptation

| AC | Brique | Vérification |
|---|---|---|
| AC1–AC6 | Brique 1 (fonction pure) | `test_qa_review_reconcile_2334.rs` — gate CI (AC1/AC2 Fire-Disposition) |
| AC7 | Brique 1 (câblage) | test structurel mika#2205 étendu — gate CI (AC3 Fire-Disposition) |
| AC8 | Brique 2 | test unitaire d'émission ; `grep qa_review_reconciled` post-déploiement (AC4) |
| AC9 | Brique 3 | test existant `github.rs:1850` (`is_suppressed_review_request` n'écarte pas le login QA) |
| — | Effet | sondes AC4 et AC5 de la Fire-Disposition |

## Hors portée (repris du ticket, sans extension)

- **Le maillon 2 — le faux-étiquetage rescue-class.** Le commentaire opérateur le nomme comme un correctif distinct (« ne pas flagger rescue quand le push a réussi + l'implement est complet »). Il touche `_compose_rescue_pr_body` et le Step 1.5 de qa-review, et son remède n'a rien à voir avec celui-ci. **Ticket de suivi à ouvrir**, avec les maillons 3 et 4 de la chaîne décrite (geste opérateur de vérification bloqué par le classifier en self-approval).
- **L'option 2 du ticket — sandbox-compatibiliser `/ce-code-review`.** Le ticket demande « une des deux » et l'opérateur a retenu l'option 1. Le skill appartient au plugin `compound-engineering`, hors de ce dépôt.
- **Toute modification de `.claude/commands/mika.md`** — voir la Rectification : le prompt est le mauvais lieu, et `--reviewer` sur `gh pr create` ferait en outre peser sur le livrable le risque qu'un flag refusé annule la création de la PR (classe mika#2211).
- **Tout geste dans `dispatch-lib.sh`** — l'ancrage existe (`_post_flight_recovery:3649-3670`) mais le geste y serait inconditionnel, donc générateur de doublons.
- **Le trou « draft sans label `wip-rescue` »** — réel (aucun scan ne le voit), mais c'est la voie draft, qui a son propre mécanisme et son propre ticket.
- **`isDraft` illisible par qa-review** — `qa_pr_view.sh:34` ne l'expose pas et `QA_REVIEW_GH_ALLOWED` interdit `gh pr view`, ce qui rend le Step 1.5.4 inexécutable dans son propre périmètre. Défaut réel, trouvé en chemin, sans rapport avec la demande de revue. **Ticket de suivi à ouvrir.**
- **Les pertes amont elles-mêmes** (drop-oldest de la file webhook, DLQ `dead`) : ce scan les rend rattrapables, il ne les fait pas disparaître. AC4 est la sonde qui dira si elles sont le vrai sujet.

## Vérification

- `cargo test -p mika-agent qa_review_reconcile` et `cargo test -p mika-agent --test eval test_qa_review_reconcile_2334`.
- `cargo test -p mika-agent dispatcher` — le test structurel mika#2205 étendu.
- `cargo build && cargo clippy && cargo fmt --check && make verify-bundled-skills`.
- `git diff --stat` ne doit toucher **aucun** fichier sous `skills/bundled/` ni `.claude/commands/`.
- Post-déploiement, dans `$MIKA_SPIRIT_LOG_FILE` :
  - `grep qa_review_reconciled | jq '{repo, pr, age_secs}'` — volume et âge des PRs rattrapées (AC4).
  - `grep qa_review_reconcile_no_token` et `grep qa_review_request_failed` — doivent être vides.
- SQL : `SELECT target_key, created_at FROM audit_events WHERE tool_name = 'qa_review_reconciled' ORDER BY created_at DESC;` — la liste des PRs que la boucle a dû rattraper, qui est aussi la mesure de la santé du chemin nominal.
- Recoupement à 48 h : chaque PR de cette liste doit porter une revue de `mika-platform-qa` postée après la pose. Une PR rattrapée mais toujours sans revue signifie que le `review_requested` ne suffit pas non plus, et c'est une halte (le remède n'aurait alors réparé que la visibilité).

## Conditions d'arrêt

- **AC0 infirme la thèse** (#2333 porte déjà une revue) → halte. La cause est le maillon 2 ; poser ce scan ne réparerait rien tout en donnant l'apparence d'un fix.
- **AC5 se déclenche** (revues en double) → désarmer et réparer le conditionnement, pas rallonger `MIN_AGE`.
- **AC4 montre un volume soutenu** → halte. Ce scan est un filet, pas un chemin : s'il porte le trafic nominal, c'est la perte amont qu'il faut traiter, et le filet masque désormais le signal qui permettrait de la voir.
- **La pose de reviewer échoue systématiquement sous le jeton résolu** (`qa_review_request_failed` non vide, classe mika#2228) → halte avant d'élargir des permissions : c'est une question d'identité et de scope de jeton, qui se tranche dans son propre ticket et pas dans celui-ci.

## Voisinage

- mika#1711 — fan-out `check_suite.completed(success)` → mika-qa : le seul rattrapage existant, et la mesure de ce qu'il ne couvre pas (drafts, CI rouge, CI jamais verte).
- mika#1822 — `ready_for_review` ajouté au routage : le même défaut, sur la voie draft, déjà réparé une fois.
- mika#1655 — le filtre `review_requested`, qui rend ce plan possible sans toucher au gateway : demander `mika-platform-qa` déclenche, demander n'importe qui d'autre non.
- mika#1870 — la file webhook bornée, dont le drop-oldest est l'une des trois pertes de M3.
- mika#2205 — l'accesseur étroit à côté du résolveur canonique : la panne à ne pas reproduire sur un quatrième scan.
- mika#1282 / mika#1618 / mika#2157 — la voie recovery et la classe rescue : le maillon 2, délibérément hors périmètre.
- mika#2120 — « un correctif qui vit dans un prompt dure exactement le temps de la mémoire de celui qui écrit le prompt » : la mesure qui écarte `.claude/commands/mika.md`.
- mika#2211 — la classe « un geste ajouté au pas de création fait perdre la PR » : la raison pour laquelle `--reviewer` n'est pas ajouté à `gh pr create`.
- #2331 — autre fragilité de fin, cité par le ticket.
