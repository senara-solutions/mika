# Plan : le filet de récupération signale son arrivée dans une PR ouverte, et retire le tampon qui ne couvre plus (mika#2151)

**Ticket :** mika issue#2151 — `fix(dispatch): le filet de récupération commite dans une PR déjà revue, sans le dire — l'approbation couvre alors du code que personne n'a lu`
**Labels :** `bug`, `p2-normal`
**Type :** issue (bug — mode d'échec : une revue qui ment)
**Palier de priorité :** Tier 2 — *ralentit la boucle*. Rien ne casse ; le coût se paie le jour où le code entré après approbation est mauvais. Le ticket fixe lui-même **p2-normal** ; le plan ne le conteste pas.

---

## Problème

### Ce qui s'est produit, relu dans git

`PR#2147` (`fix/2121/task-engine-306-dispatches-marqu-s`) porte cinq commits :

```
746a82dd docs(plans): groom mika issue#2121 — garde structurelle + mesure (AC-…
e3fe1724 wip(mika#2121): impl staged by post-flight recovery (mika#1282)
628099ef wip(mika#2121): impl staged by post-flight recovery (mika#1282)
429b1e9f Merge branch 'main' into fix/2121/task-engine-306-dispatches-marqu-s
74ace2da Merge branch 'main' into fix/2121/task-engine-306-dispatches-marqu-s
```

Deux commits `wip(mika#2121)` de la **même classe de sauvetage** (`mika#1282`, arbre sale), l'un après l'autre — c'est le cas AC4, et il n'est pas théorique : il est déjà arrivé.

### Le tampon était un **commentaire**, pas un `reviewDecision` — et c'est décisif

Le relevé horaire, mesuré et non raconté (tout en Z ; local = Z+2) :

| Heure (Z) | Fait |
|---|---|
| `2026-09-02T19:17:47` | `e3fe1724` — **premier** sauvetage `mika#1282` |
| `2026-09-02T19:18:58` | revue `mika-platform-qa` — état `COMMENTED`, sur `e3fe1724` |
| `2026-09-03T00:46:51` | commentaire `samidarko` : *« # Revue QA formelle — APPROUVÉE pour son périmètre »* — **le tampon** |
| `2026-09-03T04:40:45` | `628099ef` — **second** sauvetage `mika#1282`, +112/−17 |
| `2026-09-03T04:42:53` | commentaire `samidarko` : *« Mon approbation du 2026-09-03 est PÉRIMÉE — ne pas merger sur elle »* — **le contre-geste, à la main** |
| `2026-09-03T04:49:33` | **première** revue à l'état `APPROVED` (`mika-platform-qa`) |
| `2026-09-03T11:02:58` | merge |

Deux lectures s'imposent, et elles orientent la conception :

**1. Au moment du sauvetage, `reviewDecision` n'était PAS `APPROVED`.** La première revue à cet état est postérieure de 9 minutes au commit `628099ef`. Le tampon en vigueur à `04:40` était un **commentaire de revue QA formelle** posté à `00:46`. Une conception qui déclencherait sur `reviewDecision == "APPROVED"` **serait restée muette sur l'incident réel**. Le déclencheur doit donc être *« une PR est ouverte sur cette branche »*, point. Le congédiement d'approbation est un geste **secondaire et conditionnel**, utile quand l'état existe, jamais le pivot.

**2. Le contre-geste existe déjà — il est manuel.** À `04:42:53`, 128 secondes après le sauvetage, un humain a écrit à la main l'avertissement d'invalidation. C'est exactement le commentaire que ce ticket demande d'automatiser : la forme est connue, la latence à battre est mesurée, et le fait qu'un humain ait dû l'écrire est la preuve que le mécanisme, lui, n'a rien dit.

### Trois silences, pas un

Le défaut n'est pas à un seul endroit. Le chemin de sauvetage traverse **trois** points où le code sait quelque chose et ne le dit pas.

**Silence 1 — le commit de sauvetage.** `dispatch-lib.sh:2618` `_rescue_dirty_worktree()` stage et commite (`:2737`, retry `cargo fmt` `:2778`). Il ne consulte jamais l'état des PR de la branche. C'est correct : commiter est sa raison d'être (le ticket l'exige explicitement — cf. « Ce que ce ticket ne demande pas »).

**Silence 2 — la poussée.** `dispatch-lib.sh:3207` `_push_branch()` est le site de poussée canonique. Son raisonnement est entièrement local à git : trois états (`HEAD == origin/$BRANCH`, `ahead`, `divergé`) et un choix de mode de poussée. **Le mot « PR » n'y apparaît pas.** C'est ici que le contenu du sauvetage entre effectivement dans la PR — un commit non poussé ne change rien à ce que voit l'opérateur.

Second site de poussée : `dispatch-lib.sh:2995`, le sauvetage « contenu résiduel » de `mika#1383` (Phase A), qui commite (`:2994`) **et pousse lui-même** avant que `_push_branch` ne s'exécute. Ce site doit être couvert aussi, sans quoi une classe de sauvetage sur deux resterait muette.

**Silence 3 — la création de PR de secours.** `dispatch-lib.sh:5451` :

```bash
RESCUED_PR_URL=$(gh pr create --repo "senara-solutions/$REPO" --head "$BRANCH" … 2>&9 || true)
```

Quand une PR est **déjà ouverte** sur la branche, `gh pr create` échoue (`a pull request for branch … already exists`). Le `|| true` avale l'échec, `RESCUED_PR_URL` reste vide, et le bloc `if [ -n "$RESCUED_PR_URL" ]` (`:5453`) ne s'exécute pas. Rien n'est écrit, ni sur la PR, ni dans `RESULT`. C'est exactement l'instant où le mécanisme *dispose déjà* de la preuve qu'il vient d'atterrir dans une PR existante — et ne fait rien de cette preuve.

### Pourquoi c'est bloquant côté machine aussi

`crates/mika-agent/src/server/ci_success_handler.rs:683` `find_pass_verdict()` cherche « la revue `APPROVED` la plus récente portant `VERDICT: pass` » et rejette toute revue dont `state != "APPROVED"` (`:716`). C'est la porte du merge automatique. Une revue **congédiée** passe à l'état `DISMISSED` dans l'API : la porte se referme d'elle-même. Retirer l'approbation n'est donc pas qu'un signal humain — c'est aussi le frein machine, sans écrire une ligne dans le moteur.

---

## Forme retenue : **A — signaler**, au point de poussée

Le ticket propose trois formes et laisse le choix à la conception. Décision : **A**.

**Pourquoi pas B (dériver le sauvetage sur `<branche>-rescue-<n>`).** Une branche dérivée devrait ouvrir sa propre PR, porter son propre `Closes #`, et cohabiter avec `_check_duplicate_commits` (`:3420`) et avec la reprise automatique `crates/mika-agent/src/wip_rescue.rs` (profondeur `RESCUE_DEPTH`, étiquette `wip-rescue`, chaîne rebase → clippy → un-draft). Elle affaiblit AC3 (« retrouvable ») plutôt qu'elle ne le renforce : le travail atterrirait sur une branche que personne ne regarde. Rayon d'action très supérieur au défaut.

**Pourquoi pas C (refuser le redispatch).** Le ticket la qualifie lui-même de « plus large, touche une autre surface — à peser », et « pas présupposée » dans le hors-périmètre. Elle *empêche* du travail là où le ticket demande de *rendre visible*. Candidate à un ticket distinct ; pas ici.

**Pourquoi pas la comparaison générale `commit_id` ↔ `head_sha` (arbitrage explicite, F2).** `find_pass_verdict()` (`ci_success_handler.rs:683`) lit déjà `commit_id` sur la revue retenue, et `PrInfo` porte `head_sha`. Refuser le merge automatique quand les deux diffèrent fermerait la porte pour **toute** avance post-revue, pas seulement celle du filet — c'est strictement plus général que ce plan. Écartée, pour trois raisons nommées :

1. **Elle ne satisfait pas AC1.** Le critère exige qu'« un opérateur qui regarde la PR » voie le changement. Une garde de merge ne s'écrit nulle part sur la PR : elle refuse, en silence, dans le moteur. C'est l'exacte forme de silence que le ticket condamne.
2. **Elle n'aurait pas tiré sur l'incident.** Mesuré : à `04:40:45Z`, aucune revue à l'état `APPROVED` n'existait. `find_pass_verdict()` aurait rendu `None` et la comparaison n'aurait eu aucun `commit_id` à comparer. La solution « plus structurelle » est, sur le cas réel, **moins couvrante** que le signal local.
3. **Autre surface, autre chemin de livraison.** Moteur Rust (`crates/mika-agent`) contre script empaqueté (`skills/bundled/`), avec un déploiement et un périmètre de test disjoints. Les mêler dans un ticket p2 gonfle le rayon d'action sans rien fermer de plus.

Elle reste une bonne garde en soi — d'où la ligne « piste notée, non construite » du hors-périmètre, à ficher séparément avec la preuve ci-dessus.

**Pourquoi pas « repasser la PR en brouillon » (`gh pr ready --undo`).** Envisagé — c'est le signal le plus fort, GitHub refusant de merger un brouillon. Rejeté : `wip_rescue.rs` scanne les brouillons portant `wip-rescue` et les **re-sort du brouillon** automatiquement après 15 min (`MIN_AGE_DEFAULT_SECS = 900`, chaîne rebase → clippy → `gh pr ready`). Un re-brouillon sur une PR de sauvetage serait donc silencieusement défait par notre propre moteur : le signal serait *moins* durable qu'un congédiement, qui n'a pas d'adversaire. Le retenir imposerait en plus une étiquette d'exclusion à câbler dans `wip_rescue.rs` — deux fichiers, deux couches, pour un signal plus fragile.

---

## Ce qui est construit

### Étape 0 — sonde du droit du jeton, **avant** d'écrire le congédiement

*(F3 de la revue architecte de premier passage — le congédiement ne doit pas dépendre d'une permission non vérifiée.)*

Le congédiement exige `pull_requests: write` sur l'installation GitHub App `mika-platform-bot[bot]` (`_setup_gh_auth`, `:1337`, bascule `gh auth switch --user "mika-platform-bot[bot]"`). Cette permission n'est **pas** vérifiée aujourd'hui.

Sonde, exécutée en premier pas de l'implémentation, **sous le jeton bot** (une sonde sous `samidarko` ne prouverait rien : propriétaire du dépôt, elle réussirait de toute façon — cf. `feedback_a_probe_needs_both_controls_in_the_same_call`) :

1. ouvrir une PR de rebut sur une branche jetable ;
2. la faire approuver ;
3. `gh api --method PUT /repos/senara-solutions/mika/pulls/<n>/reviews/<id>/dismissals -f message=probe -f event=DISMISS` ;
4. relire `state` de la revue : `DISMISSED` attendu ;
5. fermer la PR, supprimer la branche.

**Disposition selon le résultat, pré-spécifiée :**

| Résultat | Suite |
|---|---|
| `DISMISSED` | Étape 2 (c) est construite telle que décrite. |
| `403` / autre échec | Étape 2 (c) est **retirée du périmètre**. Le commentaire et l'étiquette restent — AC1 tient sur eux. Un ticket de suite est fiché pour demander la permission à l'installation, avec la sortie de la sonde comme preuve. Le plan n'est pas re-groomé pour autant : le repli est déjà décidé ici. |

### Étape 1 — accumuler les commits de sauvetage

Deux variables globales, initialisées auprès de `RESCUED_DIRTY_WORKTREE=0` (`dispatch-lib.sh:2061`) :

- `RESCUE_COMMITS` — SHAs (un par ligne) des commits de sauvetage produits pendant ce dispatch.
- `RESCUE_COMMITS_SIGNALLED` — sous-ensemble déjà rapporté sur la PR.

Producteurs (append du `rev-parse HEAD` après un commit réussi) :

- `_rescue_dirty_worktree()` — les deux bras de succès (`:2757` chemin direct, `:2792` chemin retry `cargo fmt`).
- Phase A `mika#1383` — après le commit `:2994`, avant la poussée `:2995`.

**Portée : toutes les compétences, pas seulement `dev-pilot`.** `RESCUED_DIRTY_WORKTREE` est volontairement restreint à `dev-pilot` (il pilote l'ouverture d'une PR de secours). `RESCUE_COMMITS` ne l'est pas : depuis `mika#2031` le filet couvre aussi `dev-groom`, et un regroom redispatché sur une branche portant une PR ouverte produit exactement le même danger.

### Étape 2 — `_signal_rescue_into_open_pr()`

Nouvelle fonction, placée juste après `_push_with_rebase_retry` (voisinage thématique : tout ce qui suit la poussée).

Garde d'entrée — retour `0` immédiat si `REPO`, `BRANCH` ou `WORKTREE_DIR` est vide, ou si l'ensemble non-signalé est vide.

**Cette garde précède l'appel réseau, et c'est ce qui répond à F4 de la revue architecte.** Il y a deux cas « nominaux » et le plan les confondait :

| Cas | Fréquence | Coût ajouté |
|---|---|---|
| **N1 — aucun sauvetage n'a eu lieu** (le pilote a commité lui-même) | le cas majoritaire d'une boucle saine | **zéro** : la garde rend `0` avant tout appel `gh`. Aucune latence, aucune dépendance réseau, aucun quota consommé. |
| **N2 — un sauvetage a eu lieu, aucune PR n'est ouverte** | le cas nominal que décrit AC2 | **une lecture** `gh pr list`, zéro mutation. |

Le coût de N2 est irréductible : on ne peut pas répondre à « une PR est-elle ouverte ? » sans le demander. Il est aussi proportionné — il ne se paie que sur un dispatch qui vient déjà de commiter du travail rescapé, jamais sur un dispatch sain.

Interrogation :

```bash
_open_pr=$(gh pr list --repo "senara-solutions/$REPO" --head "$BRANCH" \
             --state open --json number,reviewDecision --jq '.[0] // empty' 2>&9 || true)
```

**Cas nominal — aucune PR ouverte (AC2).** `_open_pr` vide → **zéro écriture GitHub**, zéro ligne ajoutée à `RESULT`, une seule ligne sur stderr (`rescue_signal.no_open_pr: branch=…`), marquage signalé, retour `0`. Le filet commite et pousse exactement comme aujourd'hui. La seule différence est une lecture d'API — pas de friction, pas de mutation.

**Cas défaut — une PR ouverte (AC1).** Trois gestes, tous *fail-open* (`|| true` + ligne stderr nommant l'échec) :

1. **Commentaire** sur la PR (`gh pr comment <n> --body-file`), contenant :
   - pour chaque SHA non-signalé : `git log -1 --format='%h %s'` et `git diff --shortstat <sha>^ <sha>` ;
   - le delta cumulé `git diff --shortstat origin/main...HEAD` ;
   - la session pilote (`SESSION_ID`), le `LOG_ID`, le ticket ;
   - une phrase non ambiguë : *le contenu de cette PR a changé après revue ; l'approbation antérieure ne couvre pas ce ou ces commits.*
2. **Étiquette** `rescue-after-review` (`gh pr edit <n> --add-label`).
3. **Congédiement**, uniquement si `reviewDecision == "APPROVED"` :
   ```bash
   gh api "/repos/senara-solutions/$REPO/pulls/$_n/reviews" \
     --jq '.[] | select(.state=="APPROVED") | .id'
   # puis, pour chaque id :
   gh api --method PUT "/repos/…/pulls/$_n/reviews/$_id/dismissals" \
     -f message="…" -f event=DISMISS
   ```
   `PUT …/dismissals` est retenu plutôt que `gh pr review --request-changes` : ce dernier refuse d'agir sur une PR dont le jeton est l'auteur (`Can not request changes on your own pull request`), ce qui est précisément le cas de `mika-platform-bot[bot]`. Le congédiement ne dépend que du droit d'écriture.

   **Ce geste est conditionnel et secondaire.** Sur l'incident réel il n'aurait pas eu lieu (`reviewDecision` n'était pas `APPROVED` à `04:40`). Son absence ne doit jamais supprimer le commentaire : c'est le commentaire qui porte AC1.

Enfin : une ligne ajoutée à `RESULT` pour que le rappel (`_deliver_callback`) porte le fait, et `RESCUE_COMMITS_SIGNALLED` avancé.

**Aucune de ces trois étapes ne peut faire échouer le dispatch ni perdre le commit (AC3).** Le commit est déjà sur la branche et déjà poussé quand la fonction s'exécute ; elle ne fait qu'écrire *à côté*.

### Étape 3 — sites d'appel

- Après la poussée en ligne de la Phase A (`:2995`), et seulement si elle a réussi.
- Après `_push_branch` dans `dispatch_claude_pilot()` (autour de `:5355`) — appel inconditionnel, la fonction se garde elle-même.

Deux appels, un seul émetteur : le second ne redit pas ce que le premier a dit (`RESCUE_COMMITS_SIGNALLED`).

### Étape 4 — rompre le silence 3

Au site `gh pr create … || true` (`:5451`), quand `RESCUED_PR_URL` est vide **et** qu'une PR ouverte existe sur la branche, écrire la raison — stderr + une ligne `RESULT` nommant la PR existante — au lieu de ne rien dire. Trois lignes, sur le chemin exact de l'incident.

### Étape 5 — étiquette

Ajouter `rescue-after-review` à `.github/labels.yml` (voisine de `wip-rescue`, `:178`), avec une description qui nomme mika#2151.

### Étape 6 — déploiement

`skills/bundled/` est ré-extrait à chaque dispatch depuis le binaire construit. **Le correctif ne prend effet qu'après `make -C mika deploy`** (reconstruction + installation + redémarrage). Un merge seul ne le déploie pas ; le dire dans le corps de la PR.

---

## Fire-Disposition

*(F1, bloquant, de la revue architecte de premier passage — un détecteur qui entre en service doit dire à l'avance ce que signifie son premier tir.)*

**Ce détecteur n'a pas d'arriéré.** `RESCUE_COMMITS` est vide au démarrage de chaque dispatch et n'est alimenté que par des commits produits **pendant ce dispatch**. Il ne balaye ni l'historique, ni les PR ouvertes, ni les branches existantes. Il n'y a donc aucun tir rétrospectif possible sur des données préexistantes : le premier tir sera un événement vivant.

**Chemin de succès :** un commit de sauvetage atterrit sur une branche portant une PR ouverte → **un commentaire est posté sur cette PR**. C'est l'unique marqueur de succès. Son absence, sur une PR dont la branche a reçu un commit `wip(` après l'ouverture de la PR, est une **violation** — pas un silence acceptable.

**Dispositions par classe de tir, décidées d'avance :**

| Classe | Attendu | Disposition |
|---|---|---|
| Sauvetage → PR ouverte **non** approuvée (le cas de l'incident) | commentaire + étiquette, **pas** de congédiement | vrai positif. Rien à trier. |
| Sauvetage → PR ouverte approuvée | commentaire + étiquette + congédiement | vrai positif. Le congédiement referme aussi `ci_success_handler.rs:716`. |
| Sauvetage → PR ouverte **en brouillon** que la boucle alimente encore (`wip-rescue`) | commentaire + étiquette, pas de congédiement (rien à congédier sur un brouillon en cours) | **Attendu, et volontairement non exempté.** Un commentaire sur un brouillon coûte une ligne et n'a aucun effet bloquant ; l'exempter demanderait de distinguer « brouillon que la boucle nourrit » de « brouillon qu'un humain relit », distinction que rien dans l'état GitHub ne porte. Un bruit inoffensif vaut mieux qu'une exemption qui rouvre le silence. |
| Sauvetage → aucune PR | rien | AC2. Zéro écriture. |
| Aucun sauvetage | rien, et **aucun appel réseau** | cas N1 ci-dessus. |

**Aucune exemption nommée n'est nécessaire**, et c'est la conclusion de fond : la seule classe de tir qu'on pourrait vouloir taire — le brouillon nourri par la boucle — est précisément celle où le tir est gratuit. Si l'exploitation montre que ce bruit gêne (mesure : nombre de commentaires par semaine sur des brouillons `wip-rescue`), l'exemption se décidera alors, sur des chiffres, dans un ticket qui les porte.

## Tests

Nouveau fichier `skills/bundled/_shared/tests/test_rescue_signal_open_pr.sh`, hors-ligne, sans `cargo`. Idiome : source direct de `dispatch-lib.sh` (`test_push_with_rebase_retry.sh` en atteste : la bibliothèque n'a pas de code impératif au niveau supérieur) + stub `gh()` en fonction shell enregistrant ses appels (idiome de `test_finalize_pr_gate.sh:149`) + dépôt git temporaire réel.

| # | Ce qui est prouvé | AC |
|---|---|---|
| T1 | `gh pr list` rend `[]` → **aucun** appel `gh pr comment` / `gh pr edit` / `gh api` enregistré, `RESULT` inchangé, retour 0 | AC2 |
| T1b | `RESCUE_COMMITS` vide (cas N1) → **aucun appel `gh` du tout**, pas même `pr list` ; le stub enregistre zéro invocation | AC2 (F4) |
| T2 | PR ouverte → un commentaire posté contenant le SHA du sauvetage et son `--shortstat` | AC1 |
| T3 | `reviewDecision == APPROVED` → un `PUT …/dismissals` par revue `APPROVED` | AC1 |
| T4 | PR ouverte mais non approuvée → commentaire posté, **aucun** congédiement | AC1 |
| T5 | Deux SHAs accumulés → **un** commentaire les nommant **tous les deux** ; second appel sans nouveauté → aucun commentaire supplémentaire ; nouvel exécutable (`SIGNALLED` réinitialisé) → signale à nouveau | AC4 |
| T6 | `gh pr comment` échoue (exit 1) → la fonction rend 0, stderr nomme l'échec, et le commit de sauvetage reste atteignable dans le dépôt temporaire réel | AC3 |
| T7 | Scénario figé PR#2147, **tel qu'il s'est produit** : branche `fix/2121/task-engine-306-dispatches-marqu-s`, PR ouverte, `reviewDecision` **absent** (le tampon est un commentaire QA du `00:46:51Z`), sauvetage `628099ef` portant `+667/−7` à `+779/−24` → **un commentaire est posté** ; aucun congédiement n'est attendu | AC5 |
| T7b | Même scénario avec `reviewDecision: APPROVED` (le cas qui arrivera dès que la revue passera par l'état GitHub) → commentaire **et** congédiement | AC5, AC1 |
| T8 | Garde statique : les trois sites de commit de sauvetage alimentent `RESCUE_COMMITS`, et `_signal_rescue_into_open_pr` est appelée après chacun des deux sites de poussée | AC1, AC4 |

T8 reprend l'idiome de garde statique de `test_rescue_commit_no_verify.sh` — celui qui empêche qu'un futur site de sauvetage soit ajouté sans câblage.

Câblage `Makefile` : ligne dans la cible `test:` (auprès de `:130`) et cible nommée `test-rescue-signal` (idiome des cibles voisines `test-dispatch-lib`, `test-pr-origin`).

---

## Acceptance criteria

- **AC1** — Étape 2, cas défaut : commentaire sur la PR + étiquette + congédiement de l'approbation. Visible sur la PR elle-même, pas seulement dans un journal. Bloquant : le congédiement referme aussi la porte machine (`ci_success_handler.rs:716` rejette tout `state != "APPROVED"`). Étape 4 couvre le troisième silence. Prouvé par T2, T3, T4, T8.
- **AC2** — Deux paliers, tous deux tenus (F4) : **N1**, aucun sauvetage → la garde d'entrée rend `0` avant tout appel `gh`, coût strictement nul ; **N2**, sauvetage sans PR ouverte → une lecture `gh pr list`, zéro mutation, `RESULT` inchangé, une ligne stderr. Prouvé par T1, qui assert l'**absence** d'appels mutants — et par T1b, qui assert l'absence de **tout** appel `gh` quand `RESCUE_COMMITS` est vide.
- **AC3** — Les trois gestes de signalement sont *fail-open* et s'exécutent **après** que le commit est sur la branche et poussé. Aucun chemin ne peut perdre le travail. Prouvé par T6.
- **AC4** — `RESCUE_COMMITS` est un accumulateur, pas un booléen ; le second sauvetage d'un même dispatch entre dans le même commentaire, et le second sauvetage d'un dispatch **ultérieur** repart d'un `SIGNALLED` vide et signale à nouveau. Prouvé par T5.
- **AC5** — Scénario réel figé en table, dans son état **mesuré** : `reviewDecision` absent au moment du sauvetage. T7 fige ce cas-là ; T7b fige sa variante `APPROVED`. Le déclencheur testé est « PR ouverte », jamais « PR approuvée » — c'est la correction que la mesure du relevé horaire a imposée au plan.

---

## Hors périmètre

- `claude-pilot#145` — le `idleTimeout` qui tue les sessions et rend le filet si souvent nécessaire. Cause amont, ticket distinct.
- La forme **C** (refus de redispatch d'un ticket à PR ouverte) — le ticket la place explicitement hors périmètre.
- **Piste notée, non construite :** `find_pass_verdict()` (`ci_success_handler.rs:683`) lit déjà `commit_id` sur la revue retenue, et `PrInfo` porte `head_sha`. Comparer les deux fermerait la porte machine pour *toute* avance post-revue, pas seulement celle du filet. C'est une garde de merge, pas un signal : elle ne satisfait pas AC1 (« un opérateur qui regarde la PR doit voir »). Candidate à un ticket séparé, à ficher avec la preuve ci-dessus.

---

## Risques

**1. Droit du jeton pour le congédiement — levé par l'Étape 0.** La sonde et sa disposition de repli sont pré-spécifiées ci-dessus ; ce n'est plus un risque ouvert mais une étape avec deux issues décidées. En complément, le chemin reste *fail-open* à l'exécution : le commentaire est posté **avant** le congédiement, donc un 403 en production n'emporte pas AC1.

**2. Le tampon en prose n'est pas neutralisé.** Le tampon réel de l'incident était un commentaire (« Revue QA formelle — APPROUVÉE »), et ce plan ne cherche pas à le détecter : un grep de prose sur des commentaires est fragile et se périmerait au premier changement de formulation QA. Conséquence assumée et nommée : le signal **informe** l'opérateur, il ne **retire** pas ce tampon-là. Ce qu'il retire, c'est l'état GitHub `APPROVED` quand il existe, et c'est le seul des deux que la machine lit.

**3. Le correctif n'est pas déployé par son merge.** Étape 6. Le corps de la PR doit le dire ; sans `make -C mika deploy`, le code est mergé et le filet reste muet.

---

## Références

- `mika#1282` — le filet de récupération post-vol (`_rescue_dirty_worktree`)
- `mika#1383` — le sauvetage de contenu résiduel (Phase A, second site de poussée)
- `mika#1396` / `mika#1679` — la PR de secours (Path B) et le silence 3
- `mika#1852` / `wip_rescue.rs` — la reprise automatique des brouillons, l'adversaire du re-brouillon
- PR#2147, commits `e3fe1724` (`19:17:47Z`) et `628099ef` (`04:40:45Z`) — le cas mesuré
- PR#2147, commentaires `00:46:51Z` (tampon QA) et `04:42:53Z` (invalidation manuelle) — la forme et la latence du geste à automatiser
- `claude-pilot#145` — la raison pour laquelle le filet se déclenche si souvent
