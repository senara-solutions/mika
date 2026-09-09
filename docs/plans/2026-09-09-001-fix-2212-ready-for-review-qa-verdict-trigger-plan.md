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
n'est couverte par aucun handler.

**Emplacement tranché (F3) : côté `mika-gateway`, sans dépendance de crate nouvelle.**
Mesuré : `crates/mika-gateway/Cargo.toml` dépend de `mika-common` et `mika-a2a`, **pas** de
`mika-agent` ; `crates/mika-agent/Cargo.toml` ne dépend pas de `mika-gateway`. Aucune crate
ne voit aujourd'hui à la fois `route_event` et les manifestes bundled. La résolution est de
ne pas créer la dépendance : le test vit dans `mika-gateway` (où `route_event` est une `pub
fn` appelable directement) et lit les manifestes **depuis le disque**, par un chemin relatif
à `env!("CARGO_MANIFEST_DIR")` vers `skills/bundled/` — ce répertoire est versionné dans le
dépôt, `include_str!` n'en est qu'un consommateur. Écrire le test côté `mika-agent`
imposerait d'ajouter `mika-gateway` aux dépendances de `mika-agent` pour un seul test :
refusé.

**Terme de couverture (disjonction à deux termes, tous deux lus sur disque) :**
une action `a` est couverte si

- **T1** — un `skills/bundled/*/skill.toml` de la famille qa-review déclare `a` comme
  mot-clé de déclenchement ; **ou**
- **T2** — `skills/bundled/qa-review/system_prompt.md` contient la sous-chaîne littérale
  `pull_request.<a>`.

T2 existe parce que `qa-review` est `always_on = true` : `opened`, `synchronize` et
`review_requested` ne sont couverts par aucun mot-clé de manifeste et n'ont pas à l'être —
ils sont couverts par l'énumération de `qa-review:5`, qui est le contrat qu'ils ont
réellement. Une recherche de sous-chaîne littérale sur `pull_request.opened` est mécanique,
pas de l'analyse de prose : la forme est exacte. C'est précisément le terme qui manquait
pour `ready_for_review` et dont l'absence n'a rien fait échouer.

L'ensemble des actions est **lu depuis `route_event`** en itérant sur une liste d'actions
candidates et en ne retenant que celles qui rendent `Some("mika-qa")` — jamais recopié en
dur : c'est le recopiage qui laisserait passer la prochaine divergence.

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

### D-E — `isDraft == true` et la garde marker répondent à deux questions différentes (F5)

Le handler saute sur `isDraft == true` ; la garde marker de `qa-review:110-131` item 4
répute une PR vérifiée si `marker == yes` **même en draft**. Ces deux règles ne se
contredisent pas — elles portent sur des questions distinctes, et l'incident lui-même le
montre :

- **Éligibilité au dispatch** (question du handler) : *cette transition draft→ready
  justifie-t-elle de dépenser un tour de revue maintenant ?* Si la PR est redevenue draft
  entre l'émission de l'événement et son traitement, l'opérateur a **retiré** son signal de
  disponibilité. Sauter est la bonne réponse, et c'est déjà la règle du précédent :
  `qa-review-webhook-success` step 2 exige `draft: false`.
- **Traitement du boilerplate rescue** (question de `qa-review`) : *une fois la revue
  engagée, la boilerplate « Auto-rescued PR » doit-elle bloquer le verdict ?* L'item 4 dit
  non quand le marker est `yes`. Il ne dit rien sur l'opportunité de déclencher une revue.

**Règle tranchée :** l'éligibilité au dispatch appartient au handler et le draft y est
disqualifiant ; le sort de la boilerplate rescue appartient à `qa-review` et le marker y
est décisif. Le cas rescue-draft `marker: yes` + re-draft n'est donc pas un verdict perdu :
il est un verdict **non déclenché**, et le geste qui le déclenche reste l'undraft — celui
que ce ticket rend enfin opérant. Aucune divergence n'est laissée implicite.

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
`qa-review-webhook-success/`.

**`skill.toml`** : `always_on = false`, `dependencies = ["qa-review"]`, et

```toml
[triggers]
keywords = ["ready_for_review", "PR ready_for_review"]
```

**Mots-clés — tranché (F4), sur mesure du moteur d'appariement.**
`crates/mika-agent/src/skills/matcher.rs:50` (`build_matcher_regex`) et `:119`
(`message_lower = user_message.to_lowercase()`) apparient **avec limites de mots et sans
sensibilité à la casse** ; les tests `:714` (`test_word_boundary_bare_bigram_does_not_collide_on_prose`)
et `:796` (`test_word_boundary_multiword_keyword_requires_adjacent_tokens`) fixent cette
sémantique. Conséquence directe : le candidat `"ready for review"` (avec espaces) **fire sur
de la prose ordinaire** — « this PR is ready for review » suffit — et activerait le handler
sur des tours conversationnels sans rapport. Il est **retiré**. Le candidat `"undraft"` est
également retiré : il n'apparaît dans aucun texte d'événement produit par
`format_event_text`, et sa seule fonction serait d'ouvrir une porte conversationnelle que
personne n'a demandée. Restent les deux formes exactes qui collent à la première ligne
réellement émise : `ready_for_review` et `PR ready_for_review`.

**Enregistrement — les deux endroits, pas un seul.** Un skill bundled n'est matérialisé pour
un agent que s'il figure dans son allowlist. `qa-review-webhook-success` apparaît à **deux**
emplacements de `crates/mika-agent/src/well_known_agents.rs` : `:225` (JSON d'identité de
`mika-qa`) et `:1968` (test de l'allowlist). `qa-review-webhook-ready` doit être ajouté aux
deux, en plus de son enregistrement dans `crates/mika-agent/src/bundled_skills.rs` selon le
motif `BundledSkill` existant. Un oubli de `:225` produit un skill qui existe, se seede dans
la bibliothèque canonique, et n'est **jamais** lié sous
`{global_home}/agents/mika-qa/skills/` — invisible, sans échec.

**`system_prompt.md`** : point d'entrée webhook, impératif « ne termine pas le tour sans
agir », et chemin de décision aligné sur celui de `qa-review-webhook-success` :

1. Corréler le numéro de PR depuis la première ligne de l'événement.
2. Sauter si hors périmètre (dépôt non révisable, auteur humain avec relecteur désigné).
3. Sauter si `isDraft == true` — la PR est redevenue draft entre l'événement et le
   traitement (voir D-E ci-dessous pour pourquoi cela ne contredit pas la garde marker).
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

Le handler **ne réimplémente pas** la garde marker rescue : elle vit dans
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

Test unitaire dans `crates/mika-gateway` (emplacement tranché en D-C — aucune dépendance de
crate nouvelle) :

1. Construire l'ensemble `A` des actions `pull_request` routées vers `mika-qa`, en itérant
   sur une liste d'actions candidates et en filtrant sur
   `route_event("pull_request", Some(a), None) == Some("mika-qa")`. `A` n'est jamais
   recopié en dur.
2. Charger, depuis `env!("CARGO_MANIFEST_DIR")` + chemin relatif vers `skills/bundled/` :
   les `skill.toml` de la famille qa-review (T1) et le texte de
   `qa-review/system_prompt.md` (T2).
3. Pour chaque `a ∈ A`, affirmer T1 ∨ T2 (définis en D-C). Message d'échec nommant l'action
   découverte et les deux termes manquants, pour que la prochaine divergence se lise sans
   enquête.

### Phase 4 — Tests

- `route_event` : `ready_for_review → mika-qa` (déjà couvert, `github.rs:1789`) ; ne pas
  régresser.
- Manifeste : `qa-review-webhook-ready` déclare des mots-clés (sinon `index.rs:1552` le
  déclare « never activate »), déclare `qa-review` en dépendance, et passe
  `verify_bundled_skills`.
- Cohérence phase 3 : contrôle rouge-avant **terme par terme**, protocole en trois mesures
  détaillé sous `## Fire-Disposition`. Ne pas se contenter d'une seule neutralisation : la
  couverture est une disjonction, un seul terme retiré laisse l'assertion satisfaite.
- Allowlist : `qa-review-webhook-ready` présent aux deux emplacements de
  `well_known_agents.rs` (`:225`, `:1968`).
- `cargo test` (pas seulement `clippy`) sur les crates touchées, plus le test de taille
  des prompts bundled.

### Phase 5 — Vérification de bout en bout

Sur une PR de test dans le dépôt, **les deux branches d'AC2 dans la même session** :

1. Ouvrir en draft, marquer non-draft → une review `mika-platform-qa` fraîche est postée au
   SHA de tête. (Contrôle positif.)
2. Re-draft puis re-undraft **sans nouveau commit** → aucun second verdict, la review du
   SHA courant existant déjà. (Contrôle négatif de la déduplication.)
3. Pousser un commit, puis undraft → un verdict frais au nouveau SHA, la review du SHA
   antérieur ne le supprimant pas. (Le cas #2202.)

La mesure porte sur la review postée — source de vérité selon
`qa-review/system_prompt.md:47` — jamais sur un log. Un contrôle positif seul ne
distinguerait pas « le handler marche » de « le handler poste toujours ».

## Acceptance criteria

- **AC1** — Le handler `qa-review-webhook-ready` existe, s'active **par appariement moteur**
  sur la première ligne `[GitHub] PR ready_for_review: …`, déclare exactement les mots-clés
  `["ready_for_review", "PR ready_for_review"]`, dépend de `qa-review`, et son prompt porte
  l'impératif de ne pas terminer le tour sans agir.
  *Vérification :* `skill.toml` + `system_prompt.md` dans le diff ; `verify_bundled_skills`
  passe ; le skill est enregistré dans `bundled_skills.rs` **et** aux deux emplacements de
  `well_known_agents.rs` (`:225` identité `mika-qa`, `:1968` test d'allowlist).

- **AC2** — Déduplication au SHA de tête. Un second `ready_for_review` sur le même
  `headRefOid`, alors qu'une review `mika-platform-qa` existe déjà à ce SHA, ne produit pas
  de second verdict ; une review à un SHA **antérieur** (cas #2202, review du 2026-09-05) ne
  supprime **pas** la revue fraîche.
  *Vérification :* le chemin de décision du handler l'énonce (étape 4, `commit_id ==
  pr.headRefOid`) ; la phase 5 exerce les deux branches.

- **AC3** — Le test de cohérence routage ↔ handlers passe : pour chaque action `a` telle que
  `route_event("pull_request", Some(a), None) == Some("mika-qa")`, `a` est couverte par T1
  (mot-clé d'un `skill.toml` de la famille qa-review) ou T2 (sous-chaîne littérale
  `pull_request.<a>` dans `qa-review/system_prompt.md`). L'ensemble des actions est dérivé
  de `route_event`, jamais recopié en dur.
  *Vérification :* rouge-avant **terme par terme** (voir Fire-Disposition), vert après.

- **AC4** — `qa-review/system_prompt.md:5` énumère `pull_request.ready_for_review` — ce qui
  est aussi le terme T2 qu'AC3 exige pour cette action.
  *Vérification :* lecture du fichier dans le diff ; AC3 échoue si la ligne est retirée.

- **AC5** — Bout en bout : une PR non-draft, marker `rescue-pipeline-verified: yes`, CI
  verte, passée de draft à ready, reçoit une review **postée** par `mika-platform-qa` au SHA
  de tête courant.
  *Vérification :* phase 5, sur la review GitHub — source de vérité selon
  `qa-review/system_prompt.md:47` — et non sur un log.

- **AC6** — La phase 0 a rendu sa disposition (tour-déclenché-sans-post, ou
  aucun-tour-déclenché), écrite en commentaire sur mika issue#2212, et le ticket de suite
  éventuel est fiché avec son évidence.
  *Vérification :* le commentaire existe sur le ticket.

- **AC7** — `DROP_SYNCHRONIZE_NO_DIFF` (`github.rs:894-970`, `audit_events.rs:42`) et
  `DROP_REVIEWER_FILTER` (mika#1655) sont **inchangés**, et la garde marker de
  `qa-review:110-131` est **inchangée**.
  *Vérification :* absents du diff.

## Fire-Disposition

Les détecteurs livrés par ce plan, et ce qui doit arriver quand ils font feu.

| Détecteur | Nature | Disposition |
|---|---|---|
| Test de cohérence routage ↔ handlers (phase 3, AC3) | **Gate CI bloquant** | Un feu signifie qu'une action `pull_request` est routée vers `mika-qa` sans handler ni énumération — exactement la classe mika#1822. Le feu **bloque le merge** ; la remédiation est d'ajouter le terme manquant, jamais de retirer l'action de la table de routage pour faire taire le test. |
| `verify_bundled_skills` sur le nouveau manifeste (AC1) | Gate CI bloquant (existant) | Feu = manifeste invalide ou skill « never activate » (`index.rs:1552`). Bloquant, remédiation dans le manifeste. |
| Test d'allowlist `well_known_agents.rs:1968` (AC1) | Gate CI bloquant (existant) | Feu = le skill n'est pas dans l'allowlist `mika-qa`. Bloquant. |
| Test de taille des prompts bundled (phase 2) | Gate CI, avertissement à 95 % | `qa-review` déclare `max_prompt_size = 65536` et était à ~58 Ko à mika#1729. L'ajout d'AC4 est d'une phrase. Si l'avertissement 95 % fait feu, la remédiation est un relèvement motivé du plafond (sous le plafond dur de 80 Ko), **pas** le retrait de l'énumération — c'est elle le terme T2. |

**Rouge-avant, terme par terme (AC3).** La couverture d'AC3 est une **disjonction** T1 ∨ T2 :
neutraliser un seul terme ne prouve rien, puisque l'autre satisfait encore l'assertion. Le
contrôle rouge exige donc **deux mesures séparées**, chacune avec l'autre terme neutralisé :

1. Retirer `ready_for_review` des mots-clés du handler, `qa-review:5` neutralisée → rouge.
2. Retirer `pull_request.ready_for_review` de `qa-review:5`, mot-clé du handler neutralisé
   → rouge.
3. Les deux termes en place → vert.

Sans ces deux mesures, un test vert n'établit pas que la garde tient : il peut n'avoir
jamais évalué que le terme survivant.

## Hors périmètre

- Router `pull_request.edited` (bascule du marker sans undraft) — ticket de suite (D-D).
- Événement d'audit « tour qa-review terminé sans review postée » — ticket de suite (D-D).
- Toute modification de `DROP_SYNCHRONIZE_NO_DIFF` (D-A) ou de `DROP_REVIEWER_FILTER`.
- La garde marker rescue de `qa-review/system_prompt.md:110-131` — correcte, non touchée
  (fait 4).
- La classe rescue-draft en amont (mika#2211, CLOSED) et le double-logging (mika#2195,
  CLOSED) — livrés, hors de ce diff.
