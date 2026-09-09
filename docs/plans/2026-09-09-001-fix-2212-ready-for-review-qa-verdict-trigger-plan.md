---
issue: 2212
type: fix
title: "`pull_request.ready_for_review` est routé vers mika-qa mais aucun skill ne le prend — le demi-livrable de mika#1822"
branch: fix/2212/qa-review-routing-aucun-verdict-qa-frais
---

# Plan — #2212 : le verdict QA qui ne vient jamais sur un rescue-draft pipeline-vérifié

## Problème (mesuré 2026-09-06, re-vérifié sur le code à `037fe33d`)

Le ticket rapporte deux PR, trois déclenchements, zéro verdict `mika-platform-qa` frais.
Les deux moitiés ont des mécanismes **différents**, et une seule est un défaut.

### Fait 1 — re-mesuré : zéro review QA sur #2210, une review périmée sur #2202

`gh pr view … --json reviews` (2026-09-09) :

- **PR #2210** (undraft → `ready_for_review`, marker `yes`, CI 20/0, mergée 13:29:51Z) :
  **aucune** review de `mika-platform-qa`. Les trois seules reviews sont `samidarko`
  (10:37:08Z, 12:27:42Z, 12:41:28Z), toutes `COMMENTED`.
- **PR #2202** (commit vide → `synchronize`) : une seule review `mika-platform-qa`, au
  **2026-09-05T19:27:01Z** — le `hold[review]` périmé d'avant la danse marker. Rien après.

Les faits du ticket tiennent. Ce plan ne les re-litige pas ; il nomme les mécanismes.

### Fait 2 — moitié #2202 : `DROP_SYNCHRONIZE_NO_DIFF`, comportement correct, pas un défaut

`crates/mika-gateway/src/github.rs:894-970` : sur `pull_request.synchronize`, le gateway
appelle `commits_have_file_changes(before, after)` et, sur `Ok(false)`, journalise
`DROP_SYNCHRONIZE_NO_DIFF` (`audit_events.rs:42`) puis `return StatusCode::OK` — la
qa-review n'est jamais dispatchée. Un commit vide ne produit aucun diff : le drop est
exactement ce que la garde #886 promet.

**Ce n'est pas la cause à réparer.** Un `synchronize` vide n'est pas un geste de
re-verdict ; c'est un contournement que l'opérateur a inventé parce que le vrai geste
ne marchait pas. Le vrai geste est le passage draft → ready — la moitié suivante.

### Fait 3 — moitié #2210 : `ready_for_review` est routé, mais aucun skill ne le prend

C'est la cause. Elle est **un demi-livrable de mika#1822**, mesurable dans le code :

1. **Le gateway route.** `github.rs:335-338` :
   ```rust
   ("pull_request", Some("opened" | "synchronize" | "review_requested" | "ready_for_review"))
       => Some("mika-qa"),
   ```
   `crates/mika-gateway/CLAUDE.md:31` dit pourquoi `ready_for_review` a été ajouté
   (mika#1822) : « draft→ready transitions were previously dropped as "not routable",
   **stranding every wip-rescue-shaped PR without an autonomous review** ». Le ticket
   présent est cette même classe, revenue.
2. **La file ne le mange pas.** `crates/mika-agent/src/server/webhook_queue_v2.rs:172-174` :
   `opened / closed / review_requested / ready_for_review` → `WebhookEventKind::Other`,
   et `coalescing_key(Other) = None` (`:203`) — « never coalesce ». L'événement atteint
   l'agent individuellement.
3. **Le texte livré est reconnaissable.** `format_event_text` (`github.rs`, bras
   `"pull_request"`) produit en première ligne :
   `[GitHub] PR ready_for_review: senara-solutions/mika#<n> — <titre> (branch: <b>)`.
4. **Mais le skill qui devrait agir ne le déclare pas.**
   `skills/bundled/qa-review/system_prompt.md:5` énumère ses déclencheurs :
   > « You are triggered by GitHub webhook events (`pull_request.opened`,
   > `pull_request.synchronize`, `pull_request.review_requested`) routed through the
   > gateway. »

   `ready_for_review` **est absent de cette énumération**. mika#1822 a modifié la table de
   routage du gateway et n'a pas touché le contrat côté agent.
5. **Et aucun handler dédié ne le couvre.** La seule classe d'événement qui a reçu un
   handler-webhook avec impératif d'action est `check_suite.completed(success)` :
   `skills/bundled/qa-review-webhook-success/` (mika#1711), dont le prompt porte
   « **CRITICAL: DO NOT end your turn without acting** » et un chemin de décision en
   6 étapes. `ready_for_review` n'a pas d'équivalent. Le tableau des handlers mika-qa :

   | skill | déclencheur couvert | impératif d'action |
   |---|---|---|
   | `qa-review` (`always_on = true`) | `opened`, `synchronize`, `review_requested` | non (prompt de revue, pas d'entrée webhook) |
   | `qa-review-webhook-success` | `check_suite.completed(success)` | **oui** (mika#1711) |
   | `qa-review-build-callback` | reprise après `build_mika` | oui |
   | `ready_for_review` | — | **aucun skill** |

6. **Et l'undraft ne relance pas la CI**, donc aucun `check_suite` ne vient rattraper par
   le chemin mika#1711 : le seul événement émis par la transition est celui que personne
   ne prend.

L'activation d'un skill est **structurelle** — `crates/mika-agent/src/skills/matcher.rs`
apparie les `[triggers] keywords` du manifeste contre le message ; `index.rs:1552` refuse
même un skill « not always_on and no trigger keywords » (« skill will never activate »).
Un handler dédié avec le mot-clé `ready_for_review` s'active donc par appariement moteur,
pas par jugement du LLM. C'est le même levier que mika#1711, dont on sait empiriquement
qu'il tient (les reviews sur CI verte partent).

### Fait 4 — la garde marker n'est pas en cause

`qa-review/system_prompt.md:110-131`, item 4 : la PR est réputée pipeline-vérifiée si
`marker == yes` **OU** `isDraft == false` **OU** aucun marker. #2202 et #2210 satisfaisaient
les deux premières. Une qa-review **fraîche** aurait procédé. Le défaut est strictement en
amont : la revue fraîche n'a jamais lieu.

### Incertitude nommée, à trancher en phase 0

La table `tasks` porte une ligne `mika-qa` du 2026-09-06T10:30:48 :
« QA review mika#2210 — build verification for behavioral ACs (logging.rs double-write fix,
mika#2195) », `trigger_type = manual`, `status = completed` — et aucune review n'a été
postée. Son libellé et son `trigger_type` désignent une vérification de build demandée à la
main sur le **contenu** de #2210 (la PR qui corrige #2195), pas une qa-review déclenchée par
le webhook `ready_for_review`. C'est l'interprétation retenue, **elle n'est pas prouvée**, et
elle est désormais mesurable : #2195 est CLOSED, sa PR #2210 est mergée, le log spirit ne
double plus. La phase 0 tranche avant d'écrire du code.

## Décisions de grooming (tranchées)

### D-A — Le geste canonique de re-verdict est l'undraft, pas le commit vide

`DROP_SYNCHRONIZE_NO_DIFF` **n'est pas modifié**. Réparer #2202 en faisant passer les
`synchronize` sans diff rendrait re-dispatchable tout amend de trailer, ce que la garde #886
existe précisément pour empêcher. La moitié #2202 se referme **par** la moitié #2210 :
une fois `ready_for_review` pris en charge, l'opérateur a un geste qui marche et n'a plus
besoin d'inventer le commit vide.

### D-B — Un handler dédié, pas une phrase de plus dans `qa-review`

Ajouter `ready_for_review` à l'énumération de `qa-review/system_prompt.md:5` est
**nécessaire mais insuffisant** : ce prompt est une procédure de revue, pas une entrée
webhook, et il ne porte aucun impératif « n'arrête pas le tour sans agir ». C'est de
l'enforcement par prompt au niveau substrat, la classe que ce dépôt a déjà mesurée comme
défaillante. Le livrable structurel est un handler dédié — `qa-review-webhook-ready` —
dont l'**activation** est un appariement moteur (`matcher.rs`), sur le modèle exact et
déjà éprouvé de `qa-review-webhook-success` (mika#1711).

### D-C — Une garde de cohérence pour que la classe ne revienne pas

mika#1822 a changé la table de routage sans changer le contrat côté agent, et rien n'a
échoué. C'est le défaut fondateur, et il est plus général que ce ticket. Le plan ajoute un
test qui échoue si une action `pull_request` routée vers `mika-qa` par `route_event`
n'est déclarée par **aucun** skill de la famille qa-review (mot-clé de déclenchement ou
énumération de `qa-review`). La prochaine action ajoutée à la table de routage sans
handler ne compilera pas verte.

### D-D — Hors périmètre, avec ticket de suite nommé

- **`pull_request.edited`** (bascule du marker `no → yes` **sans** undraft) : `route_event`
  ne route pas `edited` du tout (`_ => None`). Un opérateur qui édite seulement le corps
  n'émet aucun événement pris. Non traité ici : l'undraft est le geste canonique (et
  `isDraft == false` suffit à la garde marker, item 4), et router `edited` ouvre un
  robinet d'événements bien plus large qui mérite son propre arbitrage.
  **Ticket de suite à ficher** après merge, avec l'évidence de ce plan.
- **Observabilité « tour qa-review terminé sans review postée »** : un événement d'audit
  quand un tour mika-qa déclenché par un webhook éligible se termine sans appel
  `run_gh pr review`. C'est la couche de détection qui aurait rendu cette classe visible
  sans trois mesures à la main. Surface différente (`agent_loop`), classe différente
  (dispatch-FAIL en général). **Ticket de suite à ficher.**
- **`DROP_REVIEWER_FILTER`** (mika#1655) : inchangé, et sans effet ici — #2210 était
  `ready_for_review`, pas `review_requested`.

## Phases

### Phase 0 — Trancher l'incertitude nommée (mesure, pas code)

**Objectif :** confirmer que le webhook `ready_for_review` de #2210 a bien atteint mika-qa
et que le tour s'est terminé sans `run_gh pr review` — ou établir qu'aucun tour n'a été
déclenché du tout.

1. Chercher dans le log spirit de mika-qa la livraison du 2026-09-06 correspondant à
   `[GitHub] PR ready_for_review: senara-solutions/mika#2210`. Le log ne double plus
   (#2195 corrigé par PR #2210, mergée) : les comptes sont désormais fiables sur les
   lignes postérieures au 2026-09-06.
2. Corréler avec `audit_events` (`agent_id = 'mika-qa'`, fenêtre 2026-09-06) et avec la
   tâche `9e66dab8-…` du 10:30:48 pour établir si elle est le tour webhook ou une demande
   manuelle distincte.
3. **Disposition pré-spécifiée :**
   - **Le tour a été déclenché et s'est terminé sans post** → le défaut est bien
     l'absence d'impératif d'action ; le plan continue **inchangé** (le handler dédié
     apporte exactement cet impératif).
   - **Aucun tour n'a été déclenché** → le défaut est plus en amont que le skill (livraison
     ou file). Le plan continue **inchangé sur les phases 1-3** (le handler reste requis),
     et la constatation est **ajoutée au corps du ticket** ; si elle désigne une panne de
     livraison distincte, elle est fichée en ticket séparé — elle ne s'ajoute pas au
     périmètre de celui-ci.

   Dans les deux cas la phase 0 **ne bloque pas** les phases suivantes : elle qualifie
   l'évidence et peut engendrer un ticket voisin, elle ne peut pas invalider le fait 3
   (qui est lu dans le code, pas dans les logs).

### Phase 1 — Le handler `qa-review-webhook-ready`

Nouveau skill bundled `skills/bundled/qa-review-webhook-ready/`, calqué sur
`qa-review-webhook-success/` :

- `skill.toml` : `always_on = false`, `dependencies = ["qa-review"]`,
  `[triggers] keywords = ["ready_for_review", "PR ready_for_review", "ready for review", "undraft"]`.
  Enregistrement dans `crates/mika-agent/src/bundled_skills.rs` selon le motif existant.
- `system_prompt.md` : point d'entrée webhook, impératif « ne termine pas le tour sans
  agir », et chemin de décision aligné sur celui de `qa-review-webhook-success` :
  1. Corréler le numéro de PR depuis la première ligne de l'événement.
  2. Sauter si hors périmètre (dépôt non révisable, auteur humain avec relecteur désigné).
  3. Sauter si `isDraft == true` — un `ready_for_review` suivi d'un re-draft.
  4. Sauter si une review de `mika-platform-qa` existe déjà **au SHA de tête courant**
     (`commit_id == pr.headRefOid`) — la déduplication au SHA, pas au numéro de PR.
     C'est ce qui empêche l'undraft répété de produire des verdicts en double, et c'est
     ce qui fait que la review périmée de #2202 (2026-09-05, autre SHA) **ne** supprime
     **pas** la revue fraîche.
  5. Sinon, appeler `qa-review` — qui possède seule le diff, la vérification plan-AC, la
     vérification de build et l'émission du verdict. Le handler ne duplique aucune de ces
     règles.
  6. Discipline de tour : sur impossibilité de procéder, `send_message` à l'opérateur avec
     la raison précise, jamais de fin de tour silencieuse.
- Le handler **ne réimplémente pas** la garde marker rescue : elle vit dans
  `qa-review/system_prompt.md:110-131` et s'applique telle quelle une fois `qa-review`
  appelée. Un rescue-draft passé non-draft y est réputé vérifié par l'item 4.

### Phase 2 — Fermer l'énumération de `qa-review`

`skills/bundled/qa-review/system_prompt.md:5` : ajouter `pull_request.ready_for_review`
à la liste des déclencheurs, avec la citation mika#1822 / mika#2212. Cette ligne est le
contrat lu par le modèle quand `qa-review` est appelée depuis le handler ; la laisser
incomplète maintient la contradiction qui a produit l'incident.

Vérifier la budgétisation : `qa-review` déclare `max_prompt_size = 65536` et le commentaire
du manifeste note ~58 Ko atteints à mika#1729. L'ajout est d'une phrase ; contrôler malgré
tout que la porte d'alerte à 95 % ne fait pas feu (le test de taille des skills bundled
existe déjà et couvre ce point).

### Phase 3 — La garde de cohérence routage ↔ handlers (D-C)

Test unitaire, côté `mika-gateway` ou `mika-agent` selon l'accès aux manifestes bundled :
pour chaque action `a` telle que `route_event("pull_request", Some(a), None) == Some("mika-qa")`,
affirmer qu'`a` est couverte — soit par un mot-clé de déclenchement d'un skill de la famille
qa-review, soit par l'énumération de `qa-review/system_prompt.md:5`. L'ensemble des actions
est lu depuis `route_event`, jamais recopié en dur dans le test : c'est le recopiage qui
laisserait passer la prochaine divergence.

### Phase 4 — Tests

- `route_event` : `ready_for_review → mika-qa` (déjà couvert, `github.rs:1789`) ; ne pas
  régresser.
- Manifeste : `qa-review-webhook-ready` déclare des mots-clés (sinon `index.rs:1552` le
  déclare « never activate »), déclare `qa-review` en dépendance, et passe
  `verify_bundled_skills`.
- Cohérence phase 3 : rouge si l'on retire `ready_for_review` du manifeste du handler,
  vert avec. **Le rouge-avant se mesure terme par terme** — neutraliser le mot-clé du
  handler ET, séparément, l'énumération de `qa-review:5`, pour que la garde ne soit pas
  satisfaite par un seul des deux termes d'une disjonction.
- `cargo test` (pas seulement `clippy`) sur les crates touchées, plus le test de taille
  des prompts bundled.

### Phase 5 — Vérification de bout en bout

Sur une PR de test dans le dépôt : ouvrir en draft, marquer non-draft, et constater qu'une
review `mika-platform-qa` fraîche est postée au SHA de tête. La mesure porte sur la review
postée (source de vérité selon `qa-review/system_prompt.md:47`), pas sur un log.

## Critères d'acceptation

- **AC1** — `pull_request.ready_for_review` sur une PR non-draft, éligible et non encore
  revue au SHA de tête déclenche une qa-review qui **poste** une review GitHub.
  Vérification : phase 5, review de `mika-platform-qa` au `headRefOid` courant.
- **AC2** — Le skill `qa-review-webhook-ready` existe, s'active par mot-clé sur la première
  ligne `[GitHub] PR ready_for_review: …`, dépend de `qa-review`, et porte l'impératif
  de ne pas terminer le tour sans agir. Vérification : manifeste + `verify_bundled_skills`.
- **AC3** — `qa-review/system_prompt.md:5` énumère `pull_request.ready_for_review`.
  Vérification : lecture du fichier dans le diff.
- **AC4** — Un test échoue si une action `pull_request` routée vers `mika-qa` par
  `route_event` n'est couverte par aucun handler ni par l'énumération de `qa-review`.
  Vérification : rouge-avant **terme par terme** (mot-clé du handler neutralisé seul, puis
  énumération neutralisée seule), vert après.
- **AC5** — Déduplication au SHA : un second `ready_for_review` sur le même SHA de tête,
  après une review déjà postée par `mika-platform-qa` à ce SHA, ne produit pas de second
  verdict ; une review à un SHA **antérieur** (cas #2202) ne supprime pas la revue fraîche.
  Vérification : le chemin de décision du handler l'énonce, et la phase 5 l'exerce.
- **AC6** — `DROP_SYNCHRONIZE_NO_DIFF` et `DROP_REVIEWER_FILTER` sont **inchangés**.
  Vérification : absents du diff.
- **AC7** — La phase 0 a rendu sa disposition, écrite en commentaire sur le ticket #2212
  (tour-déclenché-sans-post, ou aucun-tour-déclenché), et le ticket de suite éventuel est
  fiché avec son évidence.

## Hors périmètre

- Router `pull_request.edited` (bascule du marker sans undraft) — ticket de suite (D-D).
- Événement d'audit « tour qa-review terminé sans review postée » — ticket de suite (D-D).
- Toute modification de `DROP_SYNCHRONIZE_NO_DIFF` (D-A) ou de `DROP_REVIEWER_FILTER`.
- La garde marker rescue de `qa-review/system_prompt.md:110-131` — correcte, non touchée
  (fait 4).
- La classe rescue-draft en amont (mika#2211, CLOSED) et le double-logging (mika#2195,
  CLOSED) — livrés, hors de ce diff.
