---
title: Borne de re-drives et refus nommé pour l'auto-pull stuck-ready - Plan
type: fix
date: 2026-08-30
issue: senara-solutions/mika#2020
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# fix(loop-substrate): l'auto-pull borne ses re-drives et nomme son abandon

**Ticket:** mika issue#2020

---

## Goal Capsule

- **Objective.** Un opérateur qui regarde le tracker voit qu'un ticket a cessé d'être secouru, sait pourquoi, et sait quoi faire pour le remettre en jeu — sans compter à la main les événements de l'API GitHub. Corollaire : un slot de dispatch n'est plus consommé indéfiniment par un ticket qui ne peut structurellement pas avancer.
- **Means.** Phase 2 de l'auto-pull compte ses propres re-drives par ticket, refuse immédiatement un ticket dont le plan appartient à une autre issue, et à l'épuisement de la borne pose `operator-review` + un commentaire qui nomme le ticket, la raison et le remède (KTD1, KTD3, KTD4).
- **Authority.** Le corps de mika#2020 fixe la direction (compteur par ticket + arrêt nommé). L'intent de dispatch ajoute la classe du plan mal attribué (mika#1887). Là où les deux se rencontrent, KTD3 porte la résolution : deux refus de sévérité différente, pas un seul.
- **Stop conditions.** Ne pas toucher la sémantique du circuit-breaker existant (`failure_count` = échec d'appel API) ; ne pas modifier Phase 0/Phase 1 au-delà de la garde d'appartenance de plan ; ne pas retirer `ready` d'un ticket simplement parce qu'il n'est pas groomé — c'est l'état d'entrée nominal du pipeline (voir KTD3, Risque R-1). Arrêter et remonter si la correction demande de lire le contenu d'un fichier de plan via l'API GitHub.
- **Execution profile.** Rust uniquement (`crates/mika-agent/`), une migration SQLite v49→v50, tests unitaires purs + tests de migration. Aucun déploiement, aucun run RT-005.
- **Tail ownership.** PR sur `fix/2020/loop-substrate-l-auto-pull-re-drive-un`, **`Closes #2020`**, reviewer `mika-platform-qa`.

---

## Product Contract

### Summary

Donner à un mécanisme de sauvetage la capacité d'abandonner en le disant. Phase 2 de l'auto-pull acquiert trois choses qu'elle n'a pas : un compteur de re-drives par ticket distinct du compteur d'échecs d'API, une garde qui refuse un plan positivement attribué à une autre issue, et un geste terminal — retirer `ready`, poser `operator-review`, commenter le ticket — qui rend le renoncement visible et réversible.

### Problem Frame

`mika#1901` a reçu le label `ready` **16 fois** en ~19 h, un toggle toutes les 70–90 minutes, par le reconciler stuck-ready (mika#1824). Aucun plan produit, aucune PR ouverte, branche identique à `main`. Chaque tour consommait le créneau `groom` qu'un ticket réellement groomable aurait pris.

Trois trous structurels, indépendants, se combinent :

1. **Phase 2 ne filtre pas sur le grooming.** `select_feeder_candidates` (Phase 0) et `select_best_candidate` (Phase 1) appellent `is_groomed()` ; `phase2_reconcile_stuck_ready` ne l'appelle jamais. Le corps de #1901 n'a aucun callout — ni `Branch:`, ni `Plan:`, ni `Grooming history:`. Rien ne distinguait, pour Phase 2, un ticket en attente d'un ticket sans issue.

2. **Le circuit-breaker existant ne peut pas voir cette boucle.** `CIRCUIT_BREAKER_THRESHOLD = 3` s'applique à `auto_pull_stats.failure_count`, qui compte les **échecs d'appel `gh`**. Chaque rescue de #1901 réussissait côté API, et `reset_auto_pull_failure` (auto_pull.rs:1168) remettait donc le compteur à 0 à chaque tour. Le breaker faisait exactement son travail ; son travail n'est pas celui-là.

3. **Un callout de plan peut désigner le plan d'un autre ticket.** `is_groomed()` vérifie la *forme* du callout — la présence de la sous-chaîne `> - **Plan:** \`docs/plans/` — jamais l'*appartenance* du fichier. Le corps de mika#1887 porte aujourd'hui la trace de l'incident : « Le callout précédent pointait `docs/plans/2026-08-21-002-fix-1933-reader-completed-section-avancement-plan.md` ». Le slot `-1933-` sur un ticket #1887. Le fichier existait, le pilote l'a ouvert, l'a lu, et n'avait aucun moyen de savoir qu'il travaillait sur l'intention d'un autre ticket.

Le troisième trou est le plus cher, et c'est ce qui ordonne la sévérité des deux refus construits ici. **Un ticket sans plan est moins dangereux qu'un ticket avec le mauvais plan.** Le premier produit une boucle stérile : coûteuse, visible en creux, réparable par un groom. Le second produit du travail confiant sur une intention devinée — une PR qui compile, qui passe la revue, et qui répond à une question que personne n'a posée. La garde doit refuser dans les deux cas ; elle ne doit pas les traiter au même rythme.

La documentation du mécanisme énonce d'ailleurs son propre seuil d'alarme (`CLAUDE.md:230`) : *« Steady-state expectation: ≤5 rescues/day; >20/day indicates the dispatch-layer primary fix is still needed. »* Seize reprises sur **un seul ticket** en 19 h. Le seuil est formulé globalement, donc cette concentration ne le déclenche pas — c'est précisément l'angle mort.

### Requirements

**Appartenance du plan**

- R1. Un ticket dont le callout `> - **Plan:**` désigne un fichier dont le créneau d'issue canonique porte un numéro **différent** du sien est refusé, dans les trois phases de l'auto-pull.
- R2. Un ticket dont le callout désigne un fichier **sans** créneau d'issue reconnaissable n'est pas refusé pour ce motif. La garde accuse sur la contradiction, jamais sur l'ambiguïté — dev-groom dispose d'un repli sur le contenu du fichier que l'auto-pull ne peut pas évaluer sans I/O.
- R3. En Phase 2, un plan attribué à une autre issue déclenche l'abandon **immédiat**, sans consommer de re-drive.

**Borne de re-drives**

- R4. Chaque re-drive Phase 2 réussi incrémente un compteur par ticket, distinct du compteur d'échecs d'API.
- R5. Un ticket dont le compteur atteint la borne n'est plus re-drivé.
- R6. Le compteur revient à zéro dès qu'un progrès observable apparaît : une PR ouverte fermant le ticket, ou une tâche `self_dev` en vol pour ce ticket.
- R7. Le compteur d'échecs d'API existant conserve sa sémantique et son seuil ; aucun de ses points de reset ne touche le compteur de re-drives.

**Abandon nommé**

- R8. L'abandon retire `ready` et pose `operator-review` — déjà reconnu comme exclusion structurelle par `is_feeder_excluded`.
- R9. L'abandon poste un commentaire sur le ticket qui nomme **le ticket, la raison, et ce qu'il faudrait pour passer**. Un `debug!` ne satisfait pas ce requirement : l'abandon doit être lisible sans grep de l'API events.
- R10. L'abandon émet un log `warn!` structuré et un `log_audit_event`, tous deux greppables par un nom d'événement stable.
- R11. Un ticket abandonné n'est ni re-drivé ni promu tant qu'il porte `operator-review`.
- R12. Retirer `operator-review` d'un ticket abandonné le remet en jeu : au tick suivant, son compteur de re-drives repart de zéro. Le commentaire de R9 énonce ce geste.
- R13. L'abandon est posé une fois par cycle de vie : un ticket déjà abandonné et encore sous `operator-review` ne re-commente pas à chaque tick.

**Observabilité de la borne**

- R14. La borne est réglable par variable d'environnement, avec repli sur le défaut et WARN sur valeur invalide — même contrat que `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS`.
- R15. `CLAUDE.md` documente la nouvelle variable, l'événement d'abandon et le geste de remise en jeu, à côté de la ligne existante du reconciler.

### Key Decisions

- **Deux refus de sévérité différente, pas un seul.** Le plan mal attribué est refusé à zéro re-drive ; toute autre impasse est refusée après la borne. *Gouverne R3, R5.*
- **Fail-open sur l'ambiguïté, fail-closed sur la contradiction.** L'auto-pull ne lit pas le contenu des plans ; il ne peut donc trancher que ce que le nom de fichier affirme positivement. *Gouverne R2.*
- **Le retrait de `operator-review` est le geste de remise en jeu.** Pas un nouveau label, pas une commande : le geste que l'opérateur fait déjà pour débloquer un ticket. *Gouverne R12.*

### Scope Boundaries

- **Hors périmètre — retirer `ready` d'un ticket non groomé au premier passage.** `ready` sur un ticket non groomé est l'état d'entrée nominal : c'est ce label qui déclenche dev-groom, qui groome *puis* dispatche. Un refus immédiat sur ce motif casserait le flux normal. Ce cas est borné, pas interdit.
- **Hors périmètre — lire le contenu d'un fichier de plan.** Le plan est committé sur la branche de grooming, pas dans le checkout de l'agent ; l'évaluer demanderait un appel API par ticket et par tick. R2 est la conséquence assumée de cette limite.
- **Hors périmètre — unifier `priority_rank` et `feeder_rank`.** Divergence connue et documentée dans le module ; sans rapport avec cette classe.
- **Hors périmètre — le seuil global `>20 rescues/day` de `CLAUDE.md:230`.** Il reste tel quel ; ce plan ferme l'angle mort de la *concentration* par ticket, pas le seuil agrégé.

### Sources

- `crates/mika-agent/src/auto_pull.rs` — module complet, 1938 l.
- `crates/mika-agent/src/db.rs:4433` (migration v42, table `auto_pull_stats`), `:9417-9470` (méthodes).
- `crates/mika-agent/src/async_db.rs:791-826` — wrappers async.
- `CLAUDE.md:230` — contrat documenté du reconciler Phase 2.
- Corps de mika#1887 — trace de l'incident de plan mal attribué (`-1933-` sur #1887).
- Mémoire `feedback_dev_groom_find_issue_plan_filename_slot` — contrat de `_find_issue_plan` : créneau `-<issue>-` en primaire, repli sur marqueur de contenu.
- mika#2038 / `docs/plans/2026-08-29-002-fix-2038-...-plan.md` — classe voisine côté dispatch-lib : réfutation d'en-tête au palier 1. Ce plan est son pendant côté auto-pull.

---

## Planning Contract

### Key Technical Decisions

**KTD1 — Compteur dédié `redrive_count`, jamais `failure_count`.**
`failure_count` signifie « l'appel `gh` a échoué » et est remis à zéro sur **chaque** rescue Phase 2 réussie (auto_pull.rs:1168) et **chaque** promotion Phase 1 réussie (:957). Le compteur de re-drives doit s'incrémenter précisément sur cet événement-là. Les deux sémantiques sont opposées sur le même point du code : les fusionner reproduirait à l'identique le bug qu'on ferme. Migration v49→v50 ajoutant à `auto_pull_stats` : `redrive_count INTEGER NOT NULL DEFAULT 0`, `last_redrive_at TEXT`, `redrive_abandoned_at TEXT`. *Gouverne R4, R7.*

**KTD2 — Le créneau d'issue est ancré en position canonique, jamais cherché librement.**
Le nom canonique est `<YYYY-MM-DD>-<seq>-<type>-<issue>-<slug>-plan.md`. La regex est ancrée : `^\d{4}-\d{2}-\d{2}-\d{3,4}-[a-z]+-(\d+)-`. L'ancrage est le point : mika#2038 a documenté qu'un glob permissif sur `*-2026-*` matche `rustsec-2026-0097`, et c'est ce faux positif confiant qui a envoyé un pilote sur un plan d'avril. Un motif non ancré ici commettrait la même faute en sens inverse — refuser un ticket parce qu'un nombre de son slug ressemble à un numéro d'issue. Trois résultats possibles : `Owned` (créneau == numéro), `OwnedByOther(n)` (créneau != numéro), `Unattributable` (pas de créneau canonique — les vieux formats `918-eval-kg-...md`, `2047-disable-...md`). Seul `OwnedByOther` refuse. *Gouverne R1, R2.*

**KTD3 — Deux refus, deux rythmes.**
`OwnedByOther` → abandon immédiat, zéro re-drive : le re-drive est ici *activement nuisible*, puisque `is_groomed()` répond `true` et que dev-groom saute donc le grooming pour dispatcher directement à l'implémentation du mauvais plan. Toute autre impasse (dont l'absence totale de callout, cas #1901) → re-drive permis jusqu'à `MAX_REDRIVES_PER_TICKET`, puis abandon. C'est la lecture fidèle du Problem Frame : refuser dans les deux cas, plus vite là où le coût est plus élevé. *Gouverne R3, R5.*

**KTD4 — L'état d'abandon est persistant, pas déduit de l'absence du label.**
Sans `redrive_abandoned_at`, deux situations deviennent indiscernables : « le compteur vient d'atteindre la borne, le label n'est pas encore posé » et « le compteur est à la borne, l'opérateur a retiré le label ». La colonne les sépare :

| `redrive_count >= borne` | `redrive_abandoned_at` | `operator-review` | Action |
|---|---|---|---|
| oui | `NULL` | absent | **Abandonner maintenant** : retirer `ready`, poser le label, commenter, horodater |
| oui | posé | présent | Écarté au filtre d'exclusion — silence, R13 |
| oui | posé | **absent** | **Remise en jeu** : `redrive_count = 0`, `redrive_abandoned_at = NULL`, le ticket redevient éligible |

La troisième ligne est le geste de R12, et elle tombe naturellement : les tickets portant `operator-review` sont déjà écartés par le filtre 2, donc tout survivant avec `redrive_abandoned_at` posé est, par construction, un ticket remis en jeu. *Gouverne R11, R12, R13.*

**KTD5 — Le reset de progrès se branche sur des filtres déjà calculés.**
« Progrès observable » = une PR ouverte fermant le ticket (`open_pr_issue_numbers`) ou une tâche `self_dev` en vol (`has_active_self_dev_task_for_issue`). Ce sont exactement les filtres 2 et 3 de Phase 2, déjà résolus à chaque tick. Le reset ne coûte donc aucune I/O supplémentaire, et il est juste : un ticket qui a produit une PR ou une session vivante a avancé grâce au re-drive. *Gouverne R6.*

**KTD6 — La garde d'appartenance monte tout en haut de la chaîne de filtres.**
Le module ordonne ses filtres par coût croissant (in-mem → DB → API GitHub). `plan_ownership` est un test de regex sur un corps déjà fetché : coût nul. Il se place avant les appels DB, ce qui économise aussi les filtres suivants pour les tickets qu'il refuse. *Gouverne R3.*

**KTD7 — Phase 0 et Phase 1 écartent sans abandonner.**
Le trou d'appartenance de plan existe identiquement dans `select_best_candidate` et `select_feeder_candidates` : un ticket comme #1887 y serait promu à `ready` puis dispatché droit à l'implémentation. Les resserrer est nécessaire, pas du périmètre en trop. Mais elles *choisissent un candidat parmi N* — un ticket qu'elles écartent n'est pas une impasse annoncée, il n'est simplement pas retenu ce tour-ci. Elles émettent donc un `warn!` structuré et passent au suivant. Le geste fort — label + commentaire — reste à Phase 2, là où le ticket porte déjà `ready` et où le dispatch est imminent. *Gouverne R1.*

**KTD8 — Borne = 3, réglable, sentinelle `0` = illimité.**
3 par cohérence avec `CIRCUIT_BREAKER_THRESHOLD` déjà dans le module, et parce qu'à un seuil d'âge de 900 s, trois re-drives représentent au minimum ~45 min — bien au-delà d'un webhook perdu ou d'un mika-dev occupé au mauvais moment. `MIKA_AUTO_PULL_MAX_REDRIVES` suit le contrat de `parse_stuck_ready_threshold` (repli + WARN sur invalide) ; `0` désactive la borne, comme `AUTO_FEEDER_MIN_READY` utilise déjà `0` en sentinelle. *Gouverne R14.*

### High-Level Technical Design

Chaîne de filtres de `phase2_reconcile_stuck_ready`, après changement — les nouveautés sont marquées `[+]` :

```
pour chaque issue :
  1. in-mem   a le label `ready`                        sinon → territoire Phase 1
  2. in-mem   [+] pas `blocked` / `operator-review`      sinon → skip (R11)
  3. in-mem   [+] plan_ownership != OwnedByOther         sinon → ABANDON IMMÉDIAT (R3)
  4. in-mem   pas d'open PR fermant l'issue              sinon → skip [+] + reset redrive (R6)
  5. DB       pas de self_dev en vol                     sinon → skip [+] + reset redrive (R6)
  6. DB       [+] si redrive_abandoned_at posé           → remise en jeu : reset (R12)
  7. DB       failure_count < CIRCUIT_BREAKER_THRESHOLD  sinon → skip (inchangé)
  8. DB       [+] redrive_count < borne                  sinon → ABANDON NOMMÉ (R5, R8-R10)
  9. API      âge du label `ready` >= seuil              sinon → skip

boucle de rescue (inchangée dans sa mécanique) :
  remove `ready` → add `ready` → record_auto_pull → reset_auto_pull_failure
                                → [+] increment_auto_pull_redrive (R4)
```

L'ordre 6 → 8 est délibéré : la remise en jeu efface le compteur *avant* que la borne ne soit évaluée, donc un ticket que l'opérateur vient de débloquer repart bien de zéro au même tick.

Le geste d'abandon, partagé par les branches 3 et 8 :

```
abandon(issue, raison, remède) :
  gh_remove_label(ready)        → sur échec : increment_auto_pull_failure, on renonce ici
  gh_apply_label(operator-review)
  gh_comment_issue(corps nommant ticket + raison + remède)   → sur échec : warn seul
  db.mark_auto_pull_redrive_abandoned(repo, issue)
  warn!(issue, reason, "auto_pull_redrive_abandoned")
  log_audit_event("auto_pull_redrive_abandoned", ...)
```

Le retrait de `ready` vient en premier parce que c'est lui qui arrête la boucle ; tout le reste est de la mise en visibilité par-dessus un arrêt déjà acquis.

### Assumptions

- A1. La convention de nommage `<date>-<seq>-<type>-<issue>-<slug>-plan.md` est celle des plans produits par le pipeline depuis mi-2026. Vérifiée sur les 9 plans les plus récents de `docs/plans/` ; les formats antérieurs (`918-...`, `2047-...`) tombent en `Unattributable` et sont donc épargnés par R2.
- A2. Le token GitHub dont dispose l'auto-pull a le droit de commenter une issue. Il édite déjà des labels sur le même dépôt avec ce token ; le commentaire ne demande pas de portée supplémentaire.
- A3. Le label `operator-review` existe sur le dépôt. Il est déjà consommé par `is_feeder_excluded` et cité par le corps de mika#2020.
- A4. `no_dispatch_test.rs` interdit `gh issue comment` dans `milestone_manager/` seulement — vérifié : le grep est ancré sur `src/milestone_manager`. `auto_pull.rs` n'est pas dans son périmètre.

### Risques

- **R-1 — Refuser trop tôt un ticket en attente légitime de grooming.** Un `ready` sur ticket non groomé est l'entrée nominale du pipeline. Mitigation : ce cas n'est jamais refusé à zéro re-drive ; il consomme la borne comme les autres (KTD3, Scope Boundaries).
- **R-2 — Faux positif d'appartenance sur un plan légitimement nommé.** Mitigation : ancrage strict en position canonique + `Unattributable` fail-open (KTD2), plus un test par format historique rencontré dans `docs/plans/`.
- **R-3 — Divergence avec le repli sur contenu de `_find_issue_plan`.** Un plan sans créneau mais portant `**Issue:**` en tête est accepté par dev-groom ; la garde le classe `Unattributable` et le laisse passer. Les deux contrats restent compatibles — c'est le sens de R2.
- **R-4 — Le commentaire d'abandon devient du bruit.** Mitigation : `redrive_abandoned_at` rend le geste unique par cycle de vie (R13), et le filtre 2 écarte le ticket dès le tick suivant.

---

## Implementation Units

### U1. `plan_ownership` — le cœur pur de la garde d'appartenance

- **Goal.** Décider, à partir du seul corps d'une issue et de son numéro, si le plan qu'elle désigne lui appartient, appartient à une autre, ou ne se prononce pas.
- **Requirements.** R1, R2.
- **Files.** `crates/mika-agent/src/auto_pull.rs`.
- **Approach.** Ajouter, près de `is_groomed()` (l.141) :
  - `pub enum PlanOwnership { Owned, OwnedByOther(u64), Unattributable }`
  - `pub fn plan_ownership(body: &str, issue_number: u64) -> PlanOwnership` — extrait le chemin du callout via une regex `> - \*\*Plan:\*\* \`(docs/plans/[^\`]+)\``, puis applique la regex de créneau ancrée `^\d{4}-\d{2}-\d{2}-\d{3,4}-[a-z]+-(\d+)-` sur le **basename**. Absence de callout ou absence de créneau → `Unattributable`. `is_groomed()` reste inchangé : forme du callout et appartenance sont deux questions distinctes, et les deux appelants n'ont pas les mêmes besoins.
  - Deux `OnceLock<Regex>`, dans le style des regex existantes du module.
- **Test Scenarios.**
  - Créneau égal au numéro → `Owned` (`2026-08-29-002-fix-2038-...` avec `2038`).
  - Créneau différent → `OwnedByOther(1933)` — le cas mika#1887 littéral, avec le chemin réel du corps de #1887.
  - Seq à 4 chiffres → `Owned` (`2026-08-29-1249-security-2039-...` avec `2039`) : la regex ne confond pas le seq et le créneau.
  - Format historique sans date → `Unattributable` (`2047-disable-release-please-workflow.md`, `918-eval-kg-fixtures-...`).
  - Corps sans callout `Plan:` → `Unattributable`.
  - Piège mika#2038 : un slug contenant `rustsec-2026-0097` ne fait pas passer `2026` pour un créneau — l'ancrage tient.
  - Callout présent mais chemin hors `docs/plans/` → `Unattributable`.
- **Verification.** `cargo test -p mika-agent plan_ownership`.

### U2. Migration v49→v50 et surface DB du compteur de re-drives

- **Goal.** Persister un compteur de re-drives et un horodatage d'abandon, sans toucher à la sémantique de `failure_count`.
- **Requirements.** R4, R7, R12, R13.
- **Files.** `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/async_db.rs`.
- **Approach.**
  - `migrate_v49_to_v50()` : trois `ALTER TABLE auto_pull_stats ADD COLUMN` — `redrive_count INTEGER NOT NULL DEFAULT 0`, `last_redrive_at TEXT`, `redrive_abandoned_at TEXT` ; `INSERT INTO schema_version (version) VALUES (50)`. Câbler dans la chaîne (db.rs:1130) et refléter dans le schéma v1 inline (db.rs:1188 et la définition de table l.1809).
  - Méthodes sync, sur le modèle des quatre existantes (:9417-9470) :
    - `get_auto_pull_redrive_state(repo, issue) -> Result<(i64, bool)>` — `(redrive_count, abandoned)`, `(0, false)` si pas de ligne.
    - `increment_auto_pull_redrive(repo, issue)` — upsert `redrive_count + 1`, `last_redrive_at = now`.
    - `reset_auto_pull_redrive(repo, issue)` — `redrive_count = 0`, `redrive_abandoned_at = NULL`.
    - `mark_auto_pull_redrive_abandoned(repo, issue)` — `redrive_abandoned_at = now`.
  - Wrappers async en miroir dans `async_db.rs` (:791-826).
- **Test Scenarios.**
  - Migration idempotente : appliquée deux fois, pas d'erreur, version = 50.
  - `test_v1_and_incremental_schemas_converge` (db.rs:18418) passe après ajout de `migrate_v49_to_v50()` à la chaîne de test.
  - Une base v49 existante avec des lignes `auto_pull_stats` conserve ses `failure_count` après migration, et lit `redrive_count = 0`.
  - `increment` puis `reset` : le compteur revient à 0 **et** `failure_count` est inchangé — le test qui prouve KTD1.
  - `mark_abandoned` puis `reset` : `redrive_abandoned_at` redevient `NULL`.
- **Verification.** `cargo test -p mika-agent auto_pull_redrive`, `cargo test -p mika-agent schemas_converge`.

### U3. Le geste d'abandon — retirer, marquer, dire

- **Goal.** Un seul chemin de code qui arrête la boucle et rend l'arrêt lisible par un humain.
- **Requirements.** R8, R9, R10, R13.
- **Files.** `crates/mika-agent/src/auto_pull.rs`.
- **Approach.**
  - `async fn gh_comment_issue(github_token, issue_number, body) -> Result<()>` — `gh issue comment <n> --repo <DEFAULT_REPO> --body <body>`, calqué sur `gh_apply_label` (:455) : `GH_TOKEN` en env, stdin null, `kill_on_drop`.
  - `enum AbandonReason { PlanOwnedByOtherIssue { plan: String, owner: u64 }, RedriveBudgetExhausted { redrives: i64, budget: i64 } }` avec deux méthodes pures : `fn reason(&self) -> String` et `fn remedy(&self) -> String`. Rendre le message testable en pur est le point — un refus dont le libellé n'est pas testé se dégrade en silence.
  - `async fn abandon_stuck_ready(db, token, issue_number, reason, session_id, trace_id)` : séquence de la section High-Level Technical Design. Le corps du commentaire nomme les trois choses exigées par R9 — le ticket, la raison, le remède — et se termine par la phrase de remise en jeu : *« Pour remettre ce ticket en jeu : \<remède\>, puis retire le label `operator-review`. »*
  - Nom d'événement stable pour R10 : `auto_pull_redrive_abandoned`, en `warn!` structuré (`issue`, `reason`, `redrives`) et en `log_audit_event`.
- **Test Scenarios.**
  - `reason()` / `remedy()` pour les deux variantes : le texte nomme le numéro de ticket, cite le chemin du plan fautif et son propriétaire réel pour `PlanOwnedByOtherIssue`, cite le compteur et la borne pour `RedriveBudgetExhausted`.
  - Le corps du commentaire contient la mention littérale de `operator-review` — la garde qui empêche le remède de disparaître à la première réécriture.
- **Verification.** `cargo test -p mika-agent abandon`.

### U4. Refonte de la chaîne de filtres Phase 2

- **Goal.** Câbler la garde, la borne, le reset de progrès et la remise en jeu dans `phase2_reconcile_stuck_ready`.
- **Requirements.** R3, R5, R6, R11, R12, R14.
- **Files.** `crates/mika-agent/src/auto_pull.rs`.
- **Dependencies.** U1, U2, U3.
- **Approach.**
  - Consts : `MAX_REDRIVES_DEFAULT: i64 = 3`, `MAX_REDRIVES_ENV: &str = "MIKA_AUTO_PULL_MAX_REDRIVES"`, plus `fn parse_max_redrives(raw: Option<&str>) -> i64` et `fn max_redrives() -> i64`, calqués trait pour trait sur `parse_stuck_ready_threshold` (:60) — `0` = illimité.
  - Insérer les filtres 2, 3, 6 et 8 de la chaîne conçue ci-dessus ; ajouter le reset de progrès aux branches de skip 4 et 5.
  - `phase2_reconcile_stuck_ready` reçoit `session_id` et `trace_id` pour pouvoir auditer l'abandon — Phase 1 les reçoit déjà (:869) ; propager depuis `auto_pull_groomed_ticket` (:597).
  - Dans la boucle de rescue, après `reset_auto_pull_failure` : `increment_auto_pull_redrive`. L'ordre importe et mérite un commentaire — c'est le point exact où l'ancien code effaçait sa propre mémoire.
- **Test Scenarios (purs).** Étendre `select_stuck_ready_candidates` ou extraire un classificateur pur `classify_stuck_ready(issue, ctx) -> StuckReadyVerdict` — préférer l'extraction, le module fait déjà ce split partout :
  - Ticket `ready` + `operator-review` → écarté, aucun abandon.
  - Ticket `ready` + plan `OwnedByOther` → verdict d'abandon immédiat, compteur non consommé.
  - Ticket `ready` non groomé, `redrive_count = 0..2` → éligible ; à `3` → verdict d'abandon.
  - `redrive_count = 3`, `redrive_abandoned_at` posé, sans `operator-review` → verdict de remise en jeu, puis éligible.
  - Ticket avec open PR ou en vol → skip **et** demande de reset.
  - Borne à `0` via env → jamais de verdict d'abandon pour budget épuisé.
  - `MIKA_AUTO_PULL_MAX_REDRIVES` absent / vide / négatif / non numérique → repli sur 3.
- **Verification.** `cargo test -p mika-agent stuck_ready`, `cargo test -p mika-agent max_redrives`.

### U5. Fermer la même classe en Phase 0 et Phase 1

- **Goal.** Un ticket dont le plan appartient à une autre issue n'est ni promu à `ready` ni sélectionné pour dispatch.
- **Requirements.** R1.
- **Files.** `crates/mika-agent/src/auto_pull.rs`.
- **Dependencies.** U1.
- **Approach.** Ajouter le filtre `!matches!(plan_ownership(&i.body, i.number), PlanOwnership::OwnedByOther(_))` à `select_best_candidate` (:208) et `select_feeder_candidates` (:312), juste après leur `is_groomed`. Émettre un `warn!(issue, plan, owner, "auto_pull_plan_ownership_mismatch")` — écarter, pas abandonner (KTD7). Les deux fonctions étant pures, le `warn!` s'y fait directement, comme le module le fait déjà ailleurs.
- **Test Scenarios.**
  - Un candidat par ailleurs parfait (groomé, p0, sans PR) mais au plan `OwnedByOther` n'est pas retenu par `select_best_candidate`.
  - Le même en Phase 0 : il ne compte pas dans les candidats du feeder.
  - Un candidat `Unattributable` reste retenu — R2 tient aussi en amont.
- **Verification.** `cargo test -p mika-agent select_`.

### U6. Documenter le contrat dans `CLAUDE.md`

- **Goal.** Un opérateur qui lit la ligne du reconciler y trouve la borne, l'événement d'abandon et le geste de remise en jeu.
- **Requirements.** R15.
- **Files.** `CLAUDE.md`.
- **Dependencies.** U4.
- **Approach.** Étendre l'entrée `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS` (l.230) et ajouter `MIKA_AUTO_PULL_MAX_REDRIVES` à côté : défaut, sentinelle `0`, événements `auto_pull_redrive_abandoned` et `auto_pull_plan_ownership_mismatch`, et la phrase de remise en jeu. Nommer mika#2020 et l'incident des 16 requeues sur #1901, comme les autres entrées nomment leur incident fondateur.
- **Verification.** Relecture ; le fichier n'a pas de test.

---

## Verification Contract

```bash
# Depuis le worktree mika/
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mika-agent auto_pull
cargo test -p mika-agent plan_ownership
cargo test -p mika-agent schemas_converge
cargo test -p mika-agent          # suite complète du crate
```

Portes de qualité :

- `cargo clippy` sans warning sur le workspace — la CI l'exige.
- La migration v49→v50 doit laisser passer `test_v1_and_incremental_schemas_converge` : c'est le test qui prouve que le schéma construit d'un bloc et le schéma migré pas à pas convergent.
- Chaque nouveau chemin de refus a un test qui le nomme. Un refus non testé est un refus qui redeviendra silencieux — c'est la classe même que ce plan ferme.

Aucun run RT-005. Aucun déploiement.

---

## Definition of Done

- [ ] `plan_ownership` distingue `Owned` / `OwnedByOther` / `Unattributable` et est couvert par les sept scénarios de U1.
- [ ] La migration v49→v50 est en place, idempotente, et la convergence de schéma passe.
- [ ] `redrive_count` est incrémenté sur chaque re-drive réussi et n'est touché par aucun point de reset de `failure_count`.
- [ ] Phase 2 refuse immédiatement un plan attribué à une autre issue, sans consommer de re-drive.
- [ ] Phase 2 cesse de re-driver à la borne et pose `operator-review` + un commentaire nommant ticket, raison et remède.
- [ ] Retirer `operator-review` d'un ticket abandonné remet son compteur à zéro au tick suivant.
- [ ] Phase 0 et Phase 1 écartent un candidat au plan mal attribué avec un `warn!` structuré.
- [ ] `CLAUDE.md` documente `MIKA_AUTO_PULL_MAX_REDRIVES` et les deux nouveaux événements.
- [ ] `cargo fmt --check`, `cargo clippy -D warnings` et `cargo test -p mika-agent` passent.
- [ ] Aucun code d'approche abandonnée ne subsiste dans le diff.
- [ ] PR ouverte avec `Closes #2020` et reviewer `mika-platform-qa`.

---

## Acceptance criteria

- [ ] Un ticket `ready` dont le callout `> - **Plan:**` désigne un fichier au créneau d'issue différent du sien n'est jamais re-drivé : il est abandonné au premier passage, avec label et commentaire.
- [ ] Un ticket `ready` sans callout de grooming est re-drivé au plus `MIKA_AUTO_PULL_MAX_REDRIVES` fois (défaut 3), puis abandonné — la situation de mika#1901 se termine en 3 tours au lieu de 16.
- [ ] Le compteur de re-drives est stocké séparément de `failure_count` et survit aux resets de ce dernier.
- [ ] Le compteur de re-drives revient à zéro dès qu'une PR ouverte ferme le ticket ou qu'une tâche `self_dev` est en vol pour lui.
- [ ] L'abandon est lisible sur le ticket lui-même : un commentaire nomme le numéro, la raison, et le geste de remise en jeu, sans qu'il faille compter les événements de l'API.
- [ ] Un ticket abandonné ne reçoit pas un second commentaire aux ticks suivants.
- [ ] Retirer `operator-review` d'un ticket abandonné suffit à le remettre en jeu avec un compteur neuf.
- [ ] Un plan au nom historique, sans créneau d'issue reconnaissable, n'est refusé par aucune des trois phases.
- [ ] `CLAUDE.md` documente la borne, sa variable d'environnement, sa sentinelle `0`, et les deux nouveaux noms d'événements.
