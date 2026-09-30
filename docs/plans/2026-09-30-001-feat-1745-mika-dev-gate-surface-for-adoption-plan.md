# mika#1745 — surface-for-adoption sur un événement sans tâche, dans un repo dispatchable

> Ticket : `senara-solutions/mika#1745` (ouvert 2026-07-08, relocalisé depuis
> `mika-platform#185`). Labels : `enhancement`, `p2-normal`, `dispatch:loop`.
> Zéro commentaire.

## Résumé

mika-dev écarte en silence les événements webhook qui n'ont pas de tâche
correspondante. Le ticket demande de remplacer ce silence par un **signal
d'adoption** adressé à l'opérateur — sans adopter automatiquement.

Ce plan ferme **un** trou (le chemin CI-failure), établit que **deux** des trois
cas de vérification du ticket sont déjà fermés ou hors périmètre, et refuse d'en
ouvrir un troisième dont la décision opérateur du 2026-08-29 dit l'inverse.

---

## 1. Ce que la lecture du code déplace dans le ticket

Le ticket a près de trois mois. Quatre de ses prémisses sont fausses aujourd'hui,
et chacune change le remède. **C'est le premier livrable.**

### R1 — Le site n'est plus le prompt

Le ticket localise la porte dans `mika_agent::skills::self-dev-webhook-*`, à la
règle *« No matching task found → STOP »*. Cette ligne existe toujours
(`skills/bundled/self-dev-webhook-ci/system_prompt.md:12`), mais depuis #594 elle
n'est plus ce qui décide : `server::ci_failure_handler` intercepte
`check_suite.completed(failure|timed_out)` **avant le tour LLM**, et le silence
est posé là :

```rust
// crates/mika-agent/src/server/ci_failure_handler.rs:191-202
let task = match find_active_task(db, &pr_url, &event.branch).await {
    Some(t) => t,
    None => {
        info!(…, "CI failure on PR with no matching work item — passing through");
        return VerdictAction::Passthrough { enrichment: None };   // ← le silence
    }
};
```

`enrichment: None` est très exactement la forme du défaut : le LLM reçoit le
texte brut du webhook, sans un mot du moteur, puis son prompt lui dit d'ignorer.

**Conséquence sur le remède.** Un correctif écrit dans le prompt ne tiendrait pas
(`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, mika#2120 :
neuf récurrences sous prompt contre zéro quand le fait est posé par le code). La
moitié qui tient doit être écrite par le **moteur**, avant le tour LLM.

### R2 — Le cas de vérification n°1 porte sur un repo que la boucle n'a pas le droit de toucher

Le ticket demande qu'un « cpp Dependabot QA-pass event » produise un
surface-for-adoption. Or `claude-pilot-py` est **délibérément absent** de
`DISPATCHABLE_REPOS`, et le doc-comment de cette constante nomme la décision :

> `control-monitor` et `claude-pilot` **sont** des dépôts git voisins de `mika`
> dans le workspace, et la décision opérateur du 2026-08-29 est qu'ils sont
> spawn-CC-only et ne doivent jamais être atteints par la boucle.

Surfacer ce repo pour adoption proposerait une action que la porte mika#2046
refuse structurellement. **Ce cas relève d'AC4 (silent-stop), pas d'AC1.**

Par ailleurs, la moitié *Dependabot* du cas est déjà fermée depuis mika#1729, et
plus fortement que ce que le ticket demande : sur un repo dispatchable, une PR
Dependabot sans tâche **merge** par le chemin `pass`, `pr_merge_with_gate` servant
de garde dure (voir `self-dev-webhook-qa/system_prompt.md`, encadré *Task-less
Dependabot PRs*).

### R3 — Le cas de vérification n°2 est déjà couvert

« Un CI success sur un repo que mika-dev possède mais n'a pas dispatché » :
`ci_success_handler` traite déjà l'absence de tâche sans s'arrêter —

```rust
// ci_success_handler.rs:527-540
let task_id = task.as_ref().map(|t| t.id.as_str()).unwrap_or("none").to_string();
let signal = MergeReadySignal { … task_id, … };
```

Le merge-ready signal est émis avec `task_id = "none"`. Aucun silence à lever.

### R4 — Il existe déjà deux listes, et ce ticket ne doit pas en créer une troisième

| repo | `INTERNAL_REPOS` (gateway) | `DISPATCHABLE_REPOS` (agent, mika#2046) |
|---|---|---|
| `mika`, `mika-cloud`, `mika-skills`, `mika-platform` | ✅ | ✅ |
| `claude-pilot-py` | ✅ | ❌ |
| `wizzard` | ✅ | ❌ |

Le gateway **route** les événements de `claude-pilot-py` et `wizzard` vers
mika-dev ; la boucle n'a pas le droit d'y **dispatcher**. L'« ownership list »
d'AC1 est donc `DISPATCHABLE_REPOS` et son prédicat `is_dispatchable_repo` —
parce que c'est celle qui répond à la question que le surface pose réellement :
*l'adoption proposée est-elle exécutable ?*

Une troisième liste serait la divergence programmée que ce dépôt a déjà payée
deux fois (sièges de dispatch mika#2092, `DISPATCHABLE_REPOS` ↔ `labels.yml`).

### Ce qu'il reste, après rectification

Un seul trou réel : **un CI failure, sur un repo dispatchable, avec une PR
ouverte, et aucune tâche.** C'est la population de ce plan.

---

## 2. Décisions de conception

### D1 — Le surface est écrit par le moteur ; le prompt n'en porte que l'intention

Trois surfaces, et seules les deux premières tiennent indépendamment du modèle :

| surface | écrite par | tient si le LLM ignore tout ? |
|---|---|---|
| ligne `audit_events` | le handler, avant le tour | **oui** |
| notification opérateur (`send_message`) | le handler, avant le tour | **oui** |
| `enrichment` sur le `Passthrough` | le handler, lue par le LLM | non — moitié intention |

Le handler détient déjà `message_sender` (`ci_failure_handler.rs:121`) et s'en
sert à deux endroits (escalade l.262, dispatch l.353). Aucun canal nouveau.

### D2 — `Passthrough`, jamais `Handled`

`Handled` remplace le message et informe le LLM qu'une action a eu lieu. AC3
interdit l'adoption automatique : le tour ne doit **rien** déclencher.
`Passthrough { enrichment: Some(…) }` laisse le tour se dérouler tel qu'il se
déroule aujourd'hui, avec un fait de plus sous les yeux du modèle.

### D3 — La dédup est une condition de viabilité, pas un raffinement

Un seul push produit jusqu'à **8** `check_suite.completed` (un par workflow) —
mesuré et documenté par mika#1869, qui a dû fermer la même classe côté succès.
Sans dédup, un CI failure sans tâche produirait jusqu'à 8 notifications Telegram
pour un seul fait, et une notification qui arrive en rafale est une notification
qu'on finit par museler.

Clé : `pr:{repo}#{n}@{head_sha}` — la forme exacte que `ci_success_handler`
(mika#1869) et `qa_review_reconcile` (mika#2347) emploient déjà. `head_sha` est
porté par `PrInfo`, que `find_open_pr` rend déjà à ce handler. Lecture par
`Database::count_recent_audit_events_for_target` ; **aucune méthode DB nouvelle,
aucune migration.**

La dédup est **fail-open** : une base illisible laisse passer le surface. Un
doublon de notification coûte une ligne ; un surface perdu rouvre le défaut.

### D4 — Détection inconditionnelle, disposition gatée

Motif mika#2249/#2272, cité parce qu'il a été payé : *la détection est
inconditionnelle, seule la disposition est gatée*. Ici :

- **ligne d'audit + log : toujours.** Ce sont la mesure, et elle doit exister
  même quand on a coupé le bruit.
- **notification opérateur : derrière `MIKA_SURFACE_FOR_ADOPTION`** (défaut
  **armé** ; `0`/`false`/`off`/`no` désarment ; absent, vide ou **non reconnu**
  laisse armé avec un WARN nommant la valeur entre guillemets — un désarmement
  par coquille sur un signal de sûreté est la panne silencieuse qu'on ferme).

Désarmé, le handler mesure toujours et n'écrit rien à l'opérateur. C'est le mode
d'observation que la sonde S2 prescrit d'employer si le volume surprend.

### D5 — Tous les termes sont fail-safe vers le silence

Le surface est **additif** : il ne bloque aucun dispatch, ne crée aucune tâche,
ne signale aucun processus, ne change aucun statut. Toute information illisible
(repo vide, `head_sha` absent, base en panne sur la dédup) retombe sur le
comportement d'aujourd'hui.

L'asymétrie qui le justifie : un surface manqué coûte un événement qu'un humain
ne voit pas — visible et rattrapable ; un surface de trop coûte une ligne de
journal. Aucune des deux erreurs n'est destructive, ce qui est précisément
pourquoi ce mécanisme peut être livré armé.

### D6 — Un seul écrivain du nom

`surface_for_adoption` est **SOLE WRITER**, dans le journal et dans
`audit_events`, tenu par un scan de source. C'est ce qui rend
`SELECT count(*) … GROUP BY target_key` exact plutôt qu'un nombre sur lequel deux
sites peuvent diverger — et le compte est la précondition de la décision
d'adoption automatique qu'AC3 diffère explicitement (« until we have enough n »).

---

## 3. Implémentation

### U1 — `crates/mika-agent/src/server/ci_failure_handler.rs`

Au step 4, remplacer le bras `None` par un appel à une fonction dédiée :

```rust
None => {
    return surface_for_adoption(
        db, &event, &pr, github_token_agent_id, message_sender,
        session_id, trace_id,
    ).await;
}
```

`surface_for_adoption` (même fichier, privée) :

1. `crate::webhook_dispatch::is_dispatchable_repo(&event.repo)` → **faux** :
   `info!` court (motif `repo_not_dispatchable`) et `Passthrough { enrichment: None }`.
   C'est AC4, byte pour byte le comportement d'aujourd'hui.
2. `head_sha` vide ou illisible → `Passthrough { enrichment: None }` + `info!`
   (motif `unreadable_head_sha`). Sans clé, pas de dédup possible.
3. Dédup : `count_recent_audit_events_for_target(agent_id, "surface_for_adoption",
   "pr:{repo}#{n}@{sha}", now - SURFACE_DEDUP_WINDOW)` > 0 → `Passthrough` avec
   l'enrichment mais **sans** notification ni seconde ligne d'audit.
4. Sinon : ligne `audit_events` + `info!` + (si armé) `send_notification` +
   `Passthrough { enrichment: Some(…) }`.

Constante `SURFACE_DEDUP_WINDOW_SECS = 3600` — une heure couvre largement le
fan-out d'un push (secondes) et la re-livraison GitHub (minutes), et un second
CI failure sur le **même** `head_sha` une heure plus tard n'est pas un fait neuf.

**Forme de l'enrichment** (AC2 (a)–(d), les quatre éléments) :

```
[surface_for_adoption] CI <conclusion> sur <repo>#<n> (branche : <branch>).
Aucune tâche mika ne correspond à cette PR — ce travail n'est pas passé par
le pipeline self-dev. Le dépôt EST dispatchable.
Adoption proposée : créer une tâche à partir de cet événement et engager.
NE PAS adopter de votre propre chef : l'opérateur ratifie au cas par cas.
```

La dernière ligne est nécessaire : sans elle, le modèle qui lit
« adoption proposée » sous un prompt qui lui dit d'ignorer est laissé devant une
contradiction, et c'est le genre d'écart que la maison a mesuré comme coûteux.

### U2 — Notification opérateur

Même forme que ses deux voisines du fichier, via `send_notification` :

```
Travail non adopté : CI <conclusion> sur <repo>#<n> (branche : <branch>).
Aucune tâche mika — non dispatché par la boucle. <url PR>
Adopter : poser `ready` sur l'issue liée, ou dispatcher à la main.
```

Nomme le geste de levée. Un signal qui ne dit pas quoi en faire est un signal
qu'on apprend à ignorer.

### U3 — `skills/bundled/self-dev-webhook-ci/system_prompt.md`

Ligne 12, la moitié **intention** : la règle d'ignorance devient conditionnelle
et renvoie à l'enrichment moteur. Aucune injonction d'agir — AC3 tient parce que
rien ne demande au modèle d'adopter.

Rappel de discipline : `skills/bundled/` est une projection du **binaire**
(mika#2340). Cette moitié n'atteint aucun agent avant `make deploy` → seed.

### U4 — `crates/mika-common/src/…` : rien

Aucune constante partagée n'est créée. `is_dispatchable_repo` est
`pub(crate)` dans `mika-agent` et `server::ci_failure_handler` est dans le même
crate.

### U5 — Kill-switch

`Settings` gagne `surface_for_adoption: Option<String>` /
`MIKA_SURFACE_FOR_ADOPTION`, lu par un helper suivant la forme maison
(absent/vide → armé ; valeurs de désarmement reconnues → désarmé ; non reconnu →
armé + WARN nommant la valeur entre guillemets). `Option<String>` et **jamais
`bool`** : sous config-rs un `bool` fait d'une coquille une erreur dure de
`Settings::load`, donc un arrêt de démarrage sur un drapeau d'observabilité
(motif `MIKA_TELEGRAM_HTML_RENDER`, mika#2291).

### U6 — `docs/architecture/mika-dev-work-assignment.md` (AC6)

Document neuf. Contenu : les deux portes (détection de dépôt, corrélation de
tâche), les **deux listes** de R4 avec ce que chacune décide, la doctrine
surface-for-adoption et son refus de l'auto-adoption, la règle que les dépôts
hors `DISPATCHABLE_REPOS` restent en silent-stop et **pourquoi** (décision
opérateur du 2026-08-29), et le tableau des trois cas de vérification du ticket
avec leur statut rectifié.

---

## 4. Fire-Disposition

Ce plan livre **un détecteur** : le scan de source SOLE WRITER de U7 ci-dessous
(son chemin de succès est « aucune violation trouvée »).

**Option retenue : (a) exception nommée en allowlist — livrée VIDE.**

`SURFACE_FOR_ADOPTION_WRITERS_ALLOWED: &[&str] = &[]`.

Justification : le nom `surface_for_adoption` est **créé par ce plan**, donc la
population des violations préexistantes est vide par construction. Il n'y a rien
à exempter, et une allowlist née non vide serait un emplacement où déposer la
prochaine infraction (doctrine mika#2323).

Deux assertions accompagnent l'allowlist :

1. **Auto-nettoyante** — le test échoue si l'allowlist cesse d'être vide sans que
   son entrée nomme un ticket de suivi. Elle rougit le jour où quelqu'un exempte
   au lieu de retirer.
2. **Anti-vacuité** — le test échoue si le nom n'est écrit **nulle part** dans le
   fichier visé. Un scan qui vise un nom mort ne vérifie rien et se lit exactement
   comme un arbre propre (classe mika#2205).

**Résolution quand il tire : on retire le second site d'écriture, on n'ajoute pas
d'entrée.**

### U7 — le scan

`canonical_tokens::tests::mika1745_the_surface_name_has_a_single_writer` — scan
de source sur `crates/mika-agent/src/`, refusant un second site écrivant le nom
(journal ou `audit_events`) hors de `ci_failure_handler.rs`. Scan de source et
non test comportemental : un second écrivain ne rend **aucune décision fausse**
le jour où il est écrit — il rend le compte inexact, en silence, avec toutes les
assertions au vert.

---

## 5. Definition of Done

- [ ] `ci_failure_handler` émet un surface-for-adoption sur un CI failure sans
      tâche dans un dépôt dispatchable, et conserve le silence ailleurs.
- [ ] Ligne `audit_events` (`tool_name = 'surface_for_adoption'`) et `info!` du
      même nom, écrits avant le tour LLM.
- [ ] Notification opérateur dédupliquée par `(repo, PR, head_sha)`, derrière le
      kill-switch armé par défaut.
- [ ] Aucune tâche créée, aucun dispatch déclenché, aucun statut modifié.
- [ ] `MIKA_SURFACE_FOR_ADOPTION` documenté dans `.env.example` et dans le
      `CLAUDE.md` racine.
- [ ] `docs/architecture/mika-dev-work-assignment.md` créé (AC6).
- [ ] Prompt `self-dev-webhook-ci` aligné (moitié intention).
- [ ] Scan SOLE WRITER livré, allowlist vide, deux assertions.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt` verts.
- [ ] `make verify-bundled-skills` vert (le prompt bundled est touché).
- [ ] `bash scripts/verify-pipeline.sh` vert.

## 6. Acceptance criteria

Transcrites du corps du ticket, avec leur statut après les rectifications de § 1.

- [ ] **AC1 — Détecter un dépôt possédé sans tâche correspondante.** Un événement
      webhook portant un signal de dépôt possédé (`is_dispatchable_repo`) et pour
      lequel la corrélation de tâche ne rend rien produit un signal
      *surface-for-adoption* au lieu d'un EndTurn terminal.
      *Périmètre rectifié (R1) : le chemin CI-failure, à `ci_failure_handler`.
      « Ownership list » = `DISPATCHABLE_REPOS` (R4).*
- [ ] **AC2 — Forme du signal.** Le signal porte (a) l'événement arrivé, (b) le
      contexte dépôt/branche/PR, (c) le motif « aucune tâche n'existe », (d) une
      action d'adoption proposée. Il atteint l'opérateur par un canal qu'il peut
      relire (notification + `audit_events`).
- [ ] **AC3 — PAS d'auto-adoption.** Aucune tâche n'est créée, aucun dispatch
      n'est déclenché. `Passthrough`, jamais `Handled` (D2). Le prompt ne
      prescrit aucune action.
- [ ] **AC4 — Comportement inchangé pour les dépôts non possédés.** Un événement
      hors `DISPATCHABLE_REPOS` conserve le bras « pas notre travail » et
      s'arrête. **Inclut `claude-pilot-py` et `wizzard`** (R2/R4) — contrôle
      négatif explicite, vu rouge.
- [ ] **AC5 — Test de régression.** Fixture : événement CI-failure, dépôt
      dispatchable, PR ouverte, aucune tâche → surface-for-adoption émis, pas de
      silence, aucune tâche créée, aucun dispatch. Plus les contrôles négatifs de
      § 7.
- [ ] **AC6 — Documentation de doctrine.**
      `docs/architecture/mika-dev-work-assignment.md` : les deux portes, la
      doctrine surface-for-adoption, la règle du silent-stop hors périmètre.
- [ ] **AC-R (ajoutée) — Les rectifications sont écrites, pas seulement
      appliquées.** Le document d'AC6 porte le tableau des trois cas de
      vérification du ticket avec leur statut réel, pour qu'un futur lecteur ne
      redécouvre pas R2 et R3 à ses frais.

---

## 7. Contrat de vérification

### Tests

`crates/mika-agent/tests/eval/test_surface_for_adoption_1745.rs` — aucun
`test_ci_failure_handler.rs` n'existe aujourd'hui ; ce fichier est neuf.

| # | cas | attendu |
|---|---|---|
| T1 | dépôt dispatchable, PR ouverte, pas de tâche | surface émis, ligne d'audit, `Passthrough` avec enrichment |
| T2 | **contrôle négatif** — `claude-pilot-py`, même forme | aucun surface, `enrichment: None` (AC4) |
| T3 | **contrôle négatif** — dépôt dispatchable **avec** tâche | chemin nominal intact, aucun surface |
| T4 | second événement, même `head_sha` | une seule notification, une seule ligne d'audit |
| T5 | `head_sha` différent | un second surface (le fait est neuf) |
| T6 | kill-switch désarmé | ligne d'audit **présente**, notification absente (D4) |
| T7 | base en panne sur la dédup | surface émis quand même (fail-open, D3) |
| T8 | AC3 structurel | aucune tâche créée, aucun dispatch, aucun statut modifié |

T2 et T3 sont porteurs : sans eux, « le handler décide » est indistinguable de
« le handler surface tout ». Chaque terme du prédicat doit être **vu rouge** par
mutation individuelle — une conjonction de termes ne se prouve pas en les
neutralisant tous à la fois (leçon mika#2277).

### Surfaces opérateur

```bash
grep surface_for_adoption "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{repo, pr_number, branch, head_sha, dedup_skipped, notified}'
```

```sql
SELECT target_key, count(*) FROM audit_events
 WHERE tool_name = 'surface_for_adoption' GROUP BY 1 ORDER BY 2 DESC;
```

| surface | régime attendu | lecture |
|---|---|---|
| `surface_for_adoption` | **non vide, faible** | chaque ligne est du travail légitime que la boucle ignorait |
| une clé `target_key` comptée > 1 | **anomalie** | la dédup ne mord pas |
| `surface_for_adoption_audit_failed` | **vide** | le WARN est passé, la ligne d'audit non — le `GROUP BY` sous-compte |

### Sondes post-déploiement, et leurs quatre haltes

> **Préalable.** `skills/bundled/` est une projection du **binaire**, pas du
> checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer` doit porter le sha
> qu'on vient de bâtir — sinon chaque sonde décrit le binaire d'hier.

**S1 — le surface mord (premier CI failure sans tâche).** Une ligne, une
notification, aucune tâche créée.
*Halte 1 — aucune ligne alors qu'un tel événement a eu lieu :* ne pas élargir le
prédicat par réflexe. Établir d'abord que le binaire servi porte le correctif
(classe mika#2340), puis lire le motif `repo_not_dispatchable` — le dépôt peut
légitimement être hors périmètre (AC4 fait son travail).

**S2 — dimensionnement (30 jours).** **Cette mesure n'était pas disponible à
l'implémenteur** : `~/.mika/data/mika.db` n'est pas lisible depuis le bac à sable
de dispatch, donc le volume réel de CI failures sans tâche est **inconnu** à la
livraison. C'est un geste d'opérateur sur l'hôte.
*Halte 2 — le compte porte du trafic nominal* (plusieurs par jour, sur des PR
différentes) : ce n'est pas le prédicat qui est trop large, c'est qu'une part
significative du travail des dépôts dispatchables ne passe pas par le pipeline —
et **c'est le résultat que le ticket cherchait**. Noter le compte, désarmer la
notification (`MIKA_SURFACE_FOR_ADOPTION=0`) pour garder la mesure sans le bruit,
et c'est ce compte qui ouvre la décision d'auto-adoption qu'AC3 diffère.

**S3 — contrôle négatif de bruit (7 jours).** Aucun surface sur un événement dont
la PR porte une tâche.
*Halte 3 — une occurrence :* la corrélation de tâche est cassée en amont
(`find_active_task` : `pr_url` puis branche) et c'est **là** qu'il faut chercher,
pas dans le seuil de dédup.

**S4 — AC4 tient.** Zéro surface sur `claude-pilot-py` et `wizzard`.
*Halte 4 — une occurrence :* le prédicat lit `INTERNAL_REPOS` au lieu de
`DISPATCHABLE_REPOS`. Désarmer, corriger la liste lue — ne pas ajouter de filtre
en aval.

**Halte transverse — les deux sondes muettes.** Zéro surface **et** zéro refus ne
prouve rien : il faut qu'un CI failure sans tâche ait réellement eu lieu depuis le
déploiement. *Une garde que personne n'a exercée se lit exactement comme une
garde qui marche* (mika#2205).

---

## 8. Ce que ce travail n'achète PAS

- **Il n'adopte rien.** C'est AC3, et c'est le point du ticket : la porte devient
  un point de décision, pas une action.
- **Il ne couvre qu'un chemin d'événement.** Le CI-failure. Les autres portes
  webhook sans tâche (`pull_request.closed` sans corrélation, `issue_comment`)
  gardent leur comportement — population non mesurée, et instruire avant de
  mesurer est ce que ce plan refuse ailleurs.
- **Il ne rattrape rien rétroactivement.** Les événements déjà écartés n'auront
  jamais leur ligne : fabriquer une ligne d'audit datée d'un fait qu'on n'a pas
  observé est l'inverse de ce que ce travail défend. La sonde est la **prochaine**
  occurrence.
- **Il rend le fait lisible, pas surveillé.** Les instruments sont le grep et la
  requête ci-dessus, et **leur silence ne prouve rien tant que personne ne les
  exécute**.
- **Il ne dimensionne pas la population** — voir S2, qui est la mesure et sa
  halte.

## 9. Hors périmètre, délibérément

- **Élargir `DISPATCHABLE_REPOS` à `claude-pilot-py`** — c'est une décision
  opérateur du 2026-08-29, pas un défaut de prédicat (R2). Un ticket qui voudrait
  la rouvrir doit la peser, pas la contourner par un surface.
- **L'auto-adoption** — explicitement différée par AC3 « until we have enough n ».
  La précondition est le compte de S2.
- **Les cas de vérification n°1 (moitié Dependabot) et n°2** — fermés par
  mika#1729 et `ci_success_handler` (R2, R3). Les rouvrir serait livrer un second
  chemin pour un besoin déjà servi.
- **Le prompt `self-dev-webhook-qa`** — son chemin sans tâche est déjà traité
  (R2) et son remède n'a rien à voir avec celui-ci.
- **Une notification agrégée** (un digest quotidien plutôt qu'une ligne par
  événement) — plus confortable si le volume est élevé, mais la forme se décide
  **sur** la mesure de S2, pas avant.

## 10. Risques

| risque | mitigation |
|---|---|
| Volume inconnu à la livraison | dédup (D3) + kill-switch (D4) + sonde S2 avec sa halte |
| Le modèle adopte malgré AC3 | le prompt ne prescrit aucune action ; T8 asserte structurellement l'absence de tâche et de dispatch |
| Le surface masque une panne amont de corrélation | S3 et sa halte 3 |
| Le prédicat lit la mauvaise liste | S4 et sa halte 4, plus le contrôle négatif T2 |
| Le prompt n'atteint pas les agents | rappel mika#2340 dans U3 et dans le préalable des sondes |
