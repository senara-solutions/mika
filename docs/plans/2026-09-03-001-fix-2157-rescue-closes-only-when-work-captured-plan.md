# Plan : le filet ne peut écrire `Closes #N` que s'il porte du travail (mika#2157)

**Ticket :** mika issue#2157 — `fix(dispatch-lib): le filet écrit \`Closes #N\` sans regarder ce qu'il a capturé — une PR de 2 lignes de journal, approuvée, fermait un p1`
**Labels :** `bug`, `p1-important`
**Type :** issue (bug — prévention, pas nettoyage)
**Palier de priorité :** Tier 2 — *ralentit la boucle*. Le balayage de l'opérateur (commentaire du 2026-09-03T21:05:59Z) a mesuré 62 PR de récupération sur quatre dépôts : **une seule** est creuse (`mika-cloud#202`, fermée non mergée), et **aucun ticket n'a été fermé silencieusement à ce jour**. Le défaut est armé, il n'a pas tiré.

---

## Problème

`skills/bundled/_shared/dispatch-lib.sh:5516`, dans le corps de la PR de récupération :

```
Closes #${ISSUE_NUM}
```

Posé **sans condition**. Le filet ne regarde jamais ce qu'il a capturé.

Quand un pilote meurt sans commiter, `dispatch-lib` commite ce que le worktree contient. Or un worktree de grooming contient au minimum l'effet de bord du grooming lui-même : deux lignes ajoutées à `.claude/groom-verdict-trail.log` (écrites par dispatch-lib à `:4121`/`:4129`). Le filet emballe ça, ouvre une PR, et y écrit une instruction que GitHub exécute **automatiquement** au merge.

L'asymétrie est du mauvais côté, et c'est le cœur du ticket :

| Surface | Nature | Révocable par |
|---|---|---|
| `--draft` (`:5499`) | procédurale | un geste humain |
| `<!-- rescue-pipeline-verified: no -->` (`:5504`) | procédurale | un geste humain |
| `Closes #${ISSUE_NUM}` (`:5516`) | **automatique** | rien — GitHub l'exécute au merge |

Deux protections qu'un humain lève d'un clic, contre une conséquence qu'aucun humain n'a besoin de déclencher.

## Mesures — ce qui a été lu dans l'arbre, pas déduit

Base de mesure : `origin/main` @ `7b4ec10a`.

### M1 — les occurrences de `Closes #` dans dispatch-lib

```
$ grep -n 'Closes #' skills/bundled/_shared/dispatch-lib.sh
3694:    #          H1, `Closes #N`, YAML `number: N`, etc. Wider zone (50 lines)
5516:Closes #${ISSUE_NUM}
```

Deux occurrences, **une seule productrice** (`:5516`), l'autre étant un commentaire (`:3694`). Le décompte du ticket est confirmé ; ses numéros de ligne (`:5449` / `:3627`) ont dérivé de ~67 lignes entre le dépôt du ticket et `7b4ec10a`. La dérive porte sur les repères, pas sur le fait.

### M2 — le bloc de récupération est réservé à `dev-pilot`

```
2816:            case "$SKILL" in dev-pilot) RESCUED_DIRTY_WORKTREE=1 ;; esac
2851:            case "$SKILL" in dev-pilot) RESCUED_DIRTY_WORKTREE=1 ;; esac
5442:    elif [ -z "$PR_URL" ] && … && [ "$SKILL" = "dev-pilot" ]; then
```

Les deux classes de récupération (`dirty-worktree` mika#1282, `commit-pushed-no-pr` mika#1396) sont **exclusivement** `dev-pilot`. Conséquence load-bearing pour AC1 : un fichier de plan sous `docs/plans/` est, dans ce bloc, un artefact d'incident — jamais un livrable. Le livrable d'un `dev-pilot` est du code. (Pour `dev-groom`, dont le livrable *est* le plan, ce bloc ne s'exécute pas.)

### M3 — ce que dispatch-lib écrit lui-même dans un worktree

Relevé exhaustif des chemins que dispatch-lib crée ou modifie dans le worktree, hors travail du pilote — la liste d'artefacts d'incident se dérive de cette mesure, pas d'une intuition :

| Chemin | Producteur | Référence |
|---|---|---|
| `.claude/groom-verdict-trail.log` | `_append_groom_verdict_trail` | `:4121`, `:4129` |
| `docs/plans/` | `/ce:plan` du grooming, réinitialisé au rebase | `:1477` |
| `.iterate/` | boucle d'itération, supprimée au rebase | `:1476` |
| `.claude/commands/` | `_seed_worktree_slash_commands` (écrase depuis `make deploy`) | `:1478` |
| `.claude/*.local.json` | config recopiée depuis `$PLATFORM_DIR` | `:1445-1450` |

Les cinq lignes de `_clean_worktree_for_rebase` (`:1475-1478`) sont la déclaration existante, dans le code, de « ce que dispatch-lib possède et peut réinitialiser sans rien perdre ». La liste d'artefacts d'incident **est** cette liste. Ce plan ne l'invente pas ; il la réutilise.

### M4 — `origin/main..HEAD` est déjà la base de mesure du diff

```
4983:        stat_output=$(git -C "$wt_dir" diff --stat origin/main..HEAD 2>&1)
```

`_ac6_verbatim_stats_block` mesure déjà le diff de la branche contre `origin/main`. Le prédicat de ce plan réutilise cette base (en forme trois-points, cf. Décision D3).

### M5 — `REQUEST_CHANGES` n'existe nulle part dans les compétences

```
$ grep -rn 'request-changes\|request_changes\|REQUEST_CHANGES' skills/bundled/
$ echo $?
0        # aucune ligne
```

Contrôle positif sur le même corpus : `grep -rn 'hold\[review\]' skills/bundled/qa-review/system_prompt.md` → 14 occurrences. La sonde voit ce qui existe ; elle ne voit pas `REQUEST_CHANGES` parce qu'il n'y est pas.

Le vocabulaire de verdict de `mika-qa` est `pass` / `hold[review]` / `block[ac]` / `block[pipeline]` / `block[dependency]` (`system_prompt.md:40-44`, `:363-371`), et la table de publication (`:542-546`) ne connaît que deux formes GitHub : `pass` → `--approve`, **tout le reste** → `--comment`. Il n'y a aucun chemin `--request-changes` dans le produit.

**Conséquence pour AC5 : voir la Décision D5.** L'intention de l'AC5 du ticket est portée intacte ; seul le jeton est lié au vocabulaire qui existe.

## Décision

### D1 — le prédicat regarde le diff, pas la classe de récupération

Un nouveau prédicat `_rescue_diff_carries_work <wt_dir>` répond à une seule question : *le diff capturé contient-il au moins un chemin hors de la liste d'artefacts d'incident ?*

```bash
# Renvoie 0 (vrai) si le diff porte au moins un fichier non-incident.
# Renvoie 1 dans TOUS les autres cas : diff entièrement incident, diff vide,
# ou diff non mesurable. Voir D2 pour pourquoi l'échec tombe de ce côté.
_rescue_diff_carries_work() {
    local wt_dir="$1" files f
    files=$(git -C "$wt_dir" diff --name-only origin/main...HEAD 2>/dev/null) || return 1
    [ -n "$files" ] || return 1
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        case "$f" in
            .claude/groom-verdict-trail.log) ;;
            .claude/commands/*)              ;;
            .claude/*.local.json)            ;;
            .iterate/*)                      ;;
            docs/plans/*)                    ;;
            *) return 0 ;;
        esac
    done <<<"$files"
    return 1
}
```

La liste `case` est la transcription de M3. Elle est volontairement **close et courte** : tout chemin non énuméré compte comme du travail. Élargir la liste désarme le filet dans son cas utile (AC4) ; c'est le sens du risque R1.

### D2 — l'échec de mesure tombe du côté `Refs`, pas `Closes`

Si `git diff` échoue (pas de `origin/main` fetché, worktree détruit, dépôt cassé), le prédicat renvoie faux et le corps porte `Refs #N`.

C'est l'argument du ticket appliqué à son propre correctif : la conséquence de se tromper vers `Refs` est un ticket qui reste ouvert et qu'un opérateur ferme à la main — visible, réversible, une ligne dans une liste. La conséquence de se tromper vers `Closes` est une fermeture silencieuse que personne ne mesure. On ne place pas un échec de mesure du côté automatique.

Un diff **vide** tombe aussi de ce côté : une PR de récupération sans aucun fichier changé ne peut, par construction, satisfaire aucun AC.

### D3 — base trois-points `origin/main...HEAD`

`_ac6_verbatim_stats_block` (M4) utilise la forme deux-points. Le prédicat utilise **trois points** (`origin/main...HEAD`, diff depuis la base de fusion) parce que c'est exactement l'ensemble de fichiers que GitHub affichera dans la PR — la question posée est « que porte cette PR », pas « qu'est-ce qui diffère de la pointe de main ». Sur une branche fraîchement rebasée les deux formes coïncident ; sur une branche en retard, seule la forme trois-points répond à la bonne question. La divergence avec `:4983` est délibérée et documentée en commentaire sur le site.

### D4 — le corps se compose dans une fonction, pas dans un heredoc en ligne

Le heredoc `RESCUEBODY` (`:5497-5517`) est inséré directement dans l'argument `--body` de `gh pr create`. Sous cette forme, AC3 et AC4 ne sont testables qu'en simulant `gh` et en relisant son argv.

Extraction de `_compose_rescue_pr_body <wt_dir> <recovery_class> <class_fact> <issue_num>`, sur le modèle de `_derive_recovery_pr_title` (`:4799`) qui a déjà fait ce choix pour le titre. La fonction lit `SESSION_ID` / `TURNS` / `COST` de l'environnement, comme aujourd'hui, et appelle `_rescue_diff_carries_work` en interne. `gh pr create` reçoit `--body "$(_compose_rescue_pr_body …)"`.

Les tests appellent alors la fonction sur de **vrais dépôts git temporaires** — pas sur un `gh` simulé. C'est ce qui rend AC3 falsifiable : la sonde traverse le vrai `git diff`, pas une reconstruction du diff écrite depuis le plan.

### D5 — AC5 : l'intention est portée, le jeton est lié au vocabulaire qui existe

Le ticket écrit : « `mika-qa` refuse (`REQUEST_CHANGES`) ». M5 mesure que ce verdict n'existe pas — ni dans `mika-qa`, ni ailleurs dans `skills/bundled/`.

L'intention d'AC5 est sans ambiguïté et se lit dans son propre motif : *« avec pour motif que le diff ne peut satisfaire aucun AC du ticket »*. C'est mot pour mot la sémantique de `block[ac]` (`system_prompt.md:363-371`) : verdict **bloquant**, non-approbateur, publié en `--comment`, et qui exige une section `Plan amendment required:` que le gestionnaire de verdicts de `mika-dev` route vers l'opérateur **sans réessai automatique**.

Ce plan porte donc AC5 comme `block[ac]`. C'est une traduction de jeton à intention constante, pas une réduction de portée : le refus est gatant, l'approbation devient impossible, et le routage vers l'opérateur est plus fort que ce qu'un `REQUEST_CHANGES` GitHub aurait produit seul.

### D6 — le contrôle d'incident passe AVANT la retenue-brouillon de l'étape 1.5

L'étape 1.5 de `qa-review/system_prompt.md:108-123` détecte déjà les PR de récupération et, quand le marqueur vaut `no` **et** que la PR est encore en brouillon, émet `hold[review]` puis termine la revue.

Cette branche n'a pas retenu `mika-cloud#202` : la PR était en brouillon, le marqueur valait `no`, et le tampon `APPROVED` a été posé quand même le 2026-08-31T14:03:05Z. Quelle qu'en soit la cause d'adhérence, le contrôle d'incident ne doit pas dépendre d'elle : il est inséré comme **item 3 de l'étape 1.5, avant la branche vérifié/non-vérifié**, et il termine la revue sur `block[ac]`.

Ordre de priorité assumé : un diff qui ne peut satisfaire aucun AC est une information plus forte qu'un brouillon non vérifié. `block[ac]` prime sur `hold[review]`.

### D7 — un marqueur lisible par machine, écrit par le producteur

`_compose_rescue_pr_body` écrit `<!-- rescue-diff: incident-only -->` ou `<!-- rescue-diff: carries-work -->`, sur le modèle de `<!-- rescue-pipeline-verified: -->` déjà en place.

Le côté revue **lit** ce marqueur au lieu de rejuger. Le producteur mesure une fois, le consommateur lit — au lieu de deux jugements indépendants qui peuvent diverger, ce qui est exactement le motif que `mika#1618` avait déjà réglé pour l'autre marqueur.

Repli obligatoire pour les PR antérieures au correctif (aucun marqueur) : mesurer depuis la liste de fichiers déjà rendue par `qa_pr_view` et appliquer la même liste d'artefacts d'incident. Absence de marqueur ne vaut pas « porte du travail ».

## Phases

### Phase 1 — le prédicat et la composition du corps (AC1, AC2)

1. Ajouter `_rescue_diff_carries_work` dans `dispatch-lib.sh`, à côté de `_derive_recovery_pr_title` (~`:4784`), avec l'en-tête de commentaire du fichier : args, valeur de retour, et la raison du fail-closed (D2).
2. Extraire `_compose_rescue_pr_body` du heredoc `:5497-5517`. Le corps produit :
   - **branche « porte du travail »** : identique à aujourd'hui, plus `<!-- rescue-diff: carries-work -->`, et `Closes #${issue_num}` en dernière ligne ;
   - **branche « entièrement incident »** : un bloc de citation en **première ligne** disant en clair que cette récupération ne contient aucun correctif et n'existe pas pour être mergée, puis `<!-- rescue-diff: incident-only -->`, puis le corps existant, et `Refs #${issue_num}` en dernière ligne.
3. Remplacer le heredoc en ligne par `--body "$(_compose_rescue_pr_body "$WORKTREE_DIR" "$RECOVERY_CLASS" "$_rescue_class_fact" "$ISSUE_NUM")"`.

### Phase 2 — la suite de tests (AC3, AC4)

4. Créer `skills/bundled/_shared/tests/test_rescue_closes_guard.sh`, sur le patron de `tests/test_stamp_pr_origin.sh` (source direct de `dispatch-lib.sh`, compteurs `PASS`/`FAIL`, `assert_contains` / `assert_not_contains`, sortie non nulle si `FAIL>0`). Chaque cas construit un vrai dépôt git temporaire avec une `origin/main` locale et une branche.

| Cas | Contenu du diff | Attendu |
|---|---|---|
| T1 (**AC3**) | `.claude/groom-verdict-trail.log` seul | `Refs #N` présent, `Closes #N` **absent**, marqueur `incident-only`, phrase AC2 en première ligne |
| T2 (**AC4**) | un vrai fichier source (`crates/…/foo.rs`) | `Closes #N` présent, marqueur `carries-work`, **pas** de bloc « aucun correctif » |
| T3 | plan + fichier source | `Closes #N` — un artefact d'incident accompagnant du vrai travail ne désarme rien |
| T4 | `docs/plans/…-plan.md` seul | `Refs #N` — un `dev-pilot` qui n'a produit qu'un plan n'a rien corrigé (M2) |
| T5 | `origin/main` absent → diff non mesurable | `Refs #N` (fail-closed, D2) |
| T6 | diff vide | `Refs #N` |

5. Câbler dans le `Makefile` : ligne dans la cible `test` (à côté des autres suites `_shared/tests/`) **et** cible nommée `test-rescue-closes-guard`, sur le patron de `test-pr-origin` / `test-sandbox-git-usable`.

### Phase 3 — le côté revue (AC5)

6. `skills/bundled/qa-review/system_prompt.md`, étape 1.5 : insérer l'item 3 « diff entièrement incident » **avant** la branche vérifié/non-vérifié (D6), et renuméroter les items suivants de l'étape 1.5 uniquement. Ne pas toucher aux numéros des étapes 1.6+ — `:798` cite « Step 1.6 » nommément.

   Contenu de l'item : lire `<!-- rescue-diff: incident-only -->` → `VERDICT: block[ac]`, motif nommant que le diff est entièrement composé d'artefacts de grooming et ne peut satisfaire aucun AC du ticket ; section `Plan amendment required:` obligatoire ; publication `--comment` par l'étape 5 ; fin de revue. Repli de mesure si le marqueur est absent (D7). Une phrase explicite : **une approbation sur un diff entièrement incident est un faux positif, pas une opinion** — reprise verbatim de l'intention du ticket.

7. Ajouter au test de Phase 2 un contrôle de contrat de prompt : le fichier `qa-review/system_prompt.md` contient bien la chaîne `rescue-diff: incident-only` et le verdict `block[ac]` dans cet item. C'est une garde contre la suppression silencieuse, pas une preuve d'adhérence — voir R2, qui le dit sans l'enjoliver.

## Definition of Done

- `_rescue_diff_carries_work` et `_compose_rescue_pr_body` existent dans `dispatch-lib.sh`, et le heredoc en ligne a disparu du site `gh pr create`.
- `bash skills/bundled/_shared/tests/test_rescue_closes_guard.sh` sort à 0 avec les six cas T1–T6 verts.
- `make test-rescue-closes-guard` existe et la suite est appelée par `make test`.
- `bash skills/bundled/_shared/test-dispatch-lib.sh` et `bash skills/bundled/_shared/tests/test_dev_groom_dirty_rescue.sh` restent verts (non-régression sur le filet).
- `qa-review/system_prompt.md` porte l'item d'incident dans l'étape 1.5, et aucune référence « Step 1.6 »/« Step 2.5 » n'a bougé.
- `bash -n skills/bundled/_shared/dispatch-lib.sh` passe ; `shellcheck` ne régresse pas sur le fichier.

## Fire-Disposition

Ce plan livre des artefacts de classe détecteur — la suite T1–T6 et le contrôle de contrat de prompt de la Phase 3.7 — dont le chemin de succès est « aucune violation trouvée ». La porte de mika#1574 exige de dire ce que fait l'implémentation quand un détecteur tire sur des données **préexistantes**, et non sur le code que la PR ajoute. Les trois détecteurs de ce plan n'ont pas le même rapport à l'existant ; ils sont traités séparément plutôt que sous une disposition unique qui en masquerait deux.

**Détecteur 1 — la suite T1–T6 : disposition (c) halte-et-remontée, sans exception nommée.**
Chaque cas construit son propre dépôt git temporaire (D4) et n'inspecte aucun corpus du dépôt. Il n'existe donc **aucune donnée préexistante** sur laquelle ces tests puissent tirer : l'option (a) est vide de contenu ici, et l'option (b) n'aurait rien à protéger. Un échec de T1–T6 signale une régression du prédicat ou de la composition du corps, jamais une violation héritée. Il fait échouer la CI, sans exception nommée et sans `#[ignore]`. La liste d'allowlist attendue par (a) est vide **par construction**, pas par indulgence.

**Détecteur 2 — le contrôle de contrat de prompt (Phase 3.7) : disposition (c), même raison.**
Il lit `qa-review/system_prompt.md` et exige la présence de `rescue-diff: incident-only` et de `block[ac]`. La conformité qu'il vérifie est **créée par la même PR** (Phase 3.6) : au moment où il atterrit, l'artefact qu'il scanne est déjà conforme. Il ne peut donc pas tirer sur de l'existant non conforme. Son tir futur signifie une seule chose — quelqu'un a retiré l'étape 1.5 item 3 — et c'est exactement ce contre quoi il existe. Échec de CI, pas d'exception.

**Détecteur 3 — `_rescue_diff_carries_work` en production : ne peut pas tirer sur l'existant.**
C'est le seul des trois qui a un corpus réel — les PR de récupération déjà ouvertes. Il ne tire pas dessus, et pas par choix de ce plan : le prédicat n'est appelé **qu'au moment de composer le corps d'une PR nouvellement créée** (Phase 1.3, site `gh pr create`). Rien dans ce plan ne relit ni ne réécrit un corps de PR existant. Les PR déjà ouvertes gardent le corps qu'elles portent.

La mesure borne la conséquence de ce choix : le balayage de l'opérateur (62 PR de récupération, quatre dépôts, tous états) a trouvé **une seule** PR creuse, `mika-cloud#202`, déjà fermée non mergée, et la plus petite PR de récupération **mergée** est `mika#1637` (+163, 2 fichiers). L'ensemble d'exceptions que l'option (a) demanderait est donc vide **par mesure**, pas par hypothèse. Le désarmement rétroactif reste hors périmètre pour cette raison, et non par omission.

**Ce qui ferait bouger cette disposition.** Si une PR de récupération creuse est trouvée ouverte après ce correctif, la disposition ne tient plus et le traitement rétroactif devient dû — avec son propre ticket, sa propre mesure, et une exception nommée par PR concernée. La condition est datable et vérifiable : `gh pr list --search "Auto-rescued in:body" --state open` sur les quatre dépôts, croisé avec un diff entièrement incident.

## Acceptance criteria

Transcrits du corps de mika#2157. AC5 porte la liaison de vocabulaire décidée en D5 — l'intention du ticket, dans le jeton qui existe (M5).

- [ ] **AC1** — Le filet n'écrit `Closes #N` que si le diff capturé contient au moins un fichier **hors de la liste des artefacts incidents** (au minimum `.claude/groom-verdict-trail.log`, les fichiers de plan sous `docs/plans/`, et tout autre journal écrit par le grooming lui-même — liste dérivée en M3). Sinon il écrit une référence non fermante (`Refs #N`).
- [ ] **AC2** — Quand le diff capturé est **entièrement** incident, le corps de la PR le dit en clair en première ligne : cette récupération ne contient aucun correctif, et la PR existe pour ne pas perdre l'état, pas pour être mergée.
- [ ] **AC3** — Un test sur `dispatch-lib` couvre le cas mesuré ici : worktree ne contenant que `.claude/groom-verdict-trail.log` modifié → le corps produit contient `Refs #N` et **pas** `Closes #N`. (Cas T1.)
- [ ] **AC4** — Le cas symétrique reste couvert : worktree contenant un vrai fichier source modifié → `Closes #N` est écrit comme aujourd'hui. Le correctif ne doit pas désarmer le filet dans son cas utile. (Cas T2, renforcé par T3.)
- [ ] **AC5** — Côté revue : `mika-qa` refuse toute PR dont le diff est entièrement incident, par un verdict **bloquant et non-approbateur**, avec pour motif que le diff ne peut satisfaire aucun AC du ticket. Le jeton est `block[ac]` — le vocabulaire de `mika-qa` ne contient pas `REQUEST_CHANGES` (M5) ; `block[ac]` est le verdict gatant, publié en `--comment`, qui route vers l'opérateur via `Plan amendment required:` sans réessai automatique. Une approbation sur un diff entièrement incident est un faux positif, pas une opinion.

## Rattachement aux critères d'acceptation

| AC | Porté par | Vérifié par |
|---|---|---|
| AC1 | D1 + Phase 1.1/1.2 | T1, T4, T5, T6 (`Refs`) et T2, T3 (`Closes`) |
| AC2 | D7 + Phase 1.2 branche incident | T1 (première ligne + marqueur) |
| AC3 | Phase 2.4 | T1 |
| AC4 | D1 (liste `case` close) + Phase 2.4 | T2, T3 |
| AC5 | D5 + D6 + Phase 3.6 | contrôle de contrat de prompt (Phase 3.7) ; adhérence non prouvable en test — R2 |

## Hors périmètre

- Le cycle de vie des brouillons `wip-rescue` qui s'éternisent — **mika#1713**, distinct : là il s'agit de PR qui ne se ferment jamais, ici d'une PR qui fermerait quelque chose qu'elle ne corrige pas.
- Le fait que le filet commite dans une PR déjà revue — **mika#2151**, en cours.
- La cause amont (les pilotes qui ne commitaient pas) : corrigée par **mika#2146**. Ce ticket porte sur ce que le filet produit, pas sur pourquoi il se déclenche.
- La retenue-brouillon de l'étape 1.5 elle-même (pourquoi elle n'a pas retenu `mika-cloud#202`). D6 la contourne par priorité au lieu de la réparer ; si elle a un défaut d'adhérence propre, il mérite son propre ticket avec sa propre mesure — pas un élargissement de celui-ci.
- Le désarmement rétroactif des PR de récupération déjà ouvertes. Le balayage de l'opérateur a mesuré qu'aucune n'est concernée hors `mika-cloud#202`, déjà fermée.

## Risques

**R1 — la liste d'artefacts d'incident est un désarmement latent.** Chaque chemin ajouté à la liste `case` retire du poids au filet dans son cas utile (AC4). La liste est close, courte, et dérivée d'une mesure (M3) plutôt que d'une intuition ; T2 et T3 la gardent par le bas. Toute extension future doit venir avec son cas de test symétrique.

**R2 — AC5 est une garde de prompt, et un prompt n'est pas une structure.** `feedback_prompt_enforcement_fragile` et `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` disent tous deux que l'application par prompt échoue au substrat de la boucle. Le contrôle de contrat de Phase 3.7 pin la présence du texte, **pas** l'adhérence du modèle. Ce plan ne prétend pas le contraire.

Ce que la structure porte réellement, c'est AC1 : après ce correctif, même si `mika-qa` approuve à tort une PR entièrement incidente et qu'un humain la merge, **aucun ticket ne se ferme** — le corps ne porte plus l'instruction. C'est l'argument d'asymétrie du ticket retourné du bon côté : la conséquence automatique est désarmée par la structure, l'opinion est gardée par le prompt. Si l'architecte estime que l'opinion mérite aussi une garde structurelle, le point d'accroche existe (`self-dev-webhook-qa` lit déjà des marqueurs du corps de PR) — mais c'est une extension de portée que ce ticket ne demande pas, et qui appelle son propre ticket.

**R3 — dérive entre le prédicat et le nettoyage de rebase.** `_clean_worktree_for_rebase` (`:1475-1478`) et la liste `case` du prédicat expriment la même notion — « ce que dispatch-lib possède » — en deux endroits. Elles peuvent diverger. Un commentaire croisé sur chaque site nomme l'autre ; ce plan ne les fusionne pas, parce que leurs sémantiques diffèrent (l'un réinitialise, l'autre classe) et qu'une abstraction prématurée coûterait plus que la duplication de cinq motifs.

## Références

- `skills/bundled/_shared/dispatch-lib.sh:5516` — le site du défaut ; `:5497-5517` le heredoc extrait en D4 ; `:4799` `_derive_recovery_pr_title`, le précédent d'extraction ; `:1475-1478` la liste dont M3 dérive ; `:4983` la base de diff existante.
- `skills/bundled/qa-review/system_prompt.md:108-123` — étape 1.5, point d'insertion ; `:363-371` la sémantique de `block[ac]` ; `:542-546` la table de publication qui montre l'absence de `--request-changes`.
- `skills/bundled/_shared/tests/test_stamp_pr_origin.sh` — patron de suite réutilisé en Phase 2.
- mika#1282 (classe `dirty-worktree`), mika#1396 (classe `commit-pushed-no-pr`), mika#1618 (le marqueur lisible par machine dont D7 reprend la forme).
- mika#1713, mika#2151, mika#2146 — les trois tickets voisins que le corps met hors périmètre.
- Commentaire de l'opérateur du 2026-09-03T21:05:59Z sur mika#2157 — le balayage 62 PR qui borne le dégât à zéro fermeture silencieuse et établit que ce ticket est de la prévention.

## Registre de grooming — ce que l'architecte a signé, et ce qu'il n'a pas vu

Première passe `mika-arch`, session `5a0533d8-f80f-44ea-8a14-8de116af3805`, disposition **ITERATE**, deux trouvailles bloquantes.

**F2 — `## Fire-Disposition` absente (mika#1574). Fondée, appliquée.** La section ci-dessus est la réponse. L'architecte suggérait la disposition (c) en bloc ; le plan la retient pour les détecteurs 1 et 2, et documente séparément le détecteur 3, qui ne peut pas tirer sur l'existant par construction. Découper plutôt que signer une disposition unique : trois détecteurs sous une seule ligne auraient caché que deux d'entre eux n'ont aucun corpus et que le troisième n'est jamais rejoué.

**F1 — « section `## Acceptance criteria` absente ». Réfutée, et la cause est de mon côté.** Le plan porte cette section depuis sa première rédaction, avec les cinq AC en puces `- [ ] **ACn**`. L'architecte ne l'a pas vue parce que **je ne lui ai pas envoyé le plan** : la première passe ne transportait que le brief de revue par les pairs, qui résume les décisions sans reproduire la section d'AC. La trouvaille mesure fidèlement ce qu'il avait sous les yeux ; elle ne mesure pas le plan. Correction de procédure, pas de contenu : la seconde passe transporte le fichier de plan intégral. Aucune modification n'est faite au plan au titre de F1 — ajouter une section qui existe déjà aurait gravé l'erreur de mesure dans l'artefact.

**Les cinq incertitudes du brief, tranchées.**

| # | Incertitude | Réponse de l'architecte | Effet sur le plan |
|---|---|---|---|
| U1 | La liste `case` est-elle au bon niveau de fermeture ? | Le raisonnement par conséquence tient : ces artefacts sont éphémères (le rebase les écrase), donc jamais des livrables légitimes. `:1475-1478` est l'autorité établie. | D1 inchangée, confirmée |
| U2 | D6 contourne l'étape 1.5 au lieu de la réparer | Découpage acceptable — défense en profondeur valable sur un mécanisme qui a échoué ; réparer 1.5 mérite son ticket si la cause est complexe | D6 inchangée ; hors-périmètre confirmé |
| U3 | AC5 reste une garde de prompt | D7 (marqueur lisible par machine) **est** la bonne réponse structurelle : après le correctif, une revue défaillante ne peut plus merger un `Closes #N` puisque l'instruction n'est plus là. Webhook hors périmètre acceptable à ce stade. | D7 confirmée comme le porteur structurel ; R2 inchangé |
| U4 | Trois-points vs deux-points | Divergence acceptable ; la forme trois-points mesure bien « ce que la PR introduit relativement à la base ». Citer le risque sur branche en retard suffit. | D3 inchangée |
| U5 | Duplication `_clean_worktree_for_rebase` ↔ liste `case` | Le commentaire croisé est la bonne réponse minimale ; la fusion créerait une mauvaise abstraction (nettoyer vs classifier) | R3 inchangé |

**Ce que l'architecte n'a pas tranché**, et qui reste donc au jugement de l'implémenteur : rien sur le fond n'a été renvoyé. Les deux trouvailles portaient sur la forme du plan (une section due, une section crue absente). Le contenu technique — D1 à D7 — est passé sans contestation, U1 à U5 comprises.

**Seconde passe — `Verdict: GROOMED`** (même session `5a0533d8-f80f-44ea-8a14-8de116af3805`). F1 réfuté avec preuve et clos sans modification du plan ; F2 fondé et résolu, l'architecte retenant explicitement que l'absence d'allowlist nommée pour les détecteurs 1 et 2 « n'est pas une indulgence : leur corpus est vide par construction ». Aucune décision non résolue, aucune nouvelle trouvaille.

La seconde passe a dû être redemandée une fois : le premier appel a rendu `stop_reason: EndTurn`, `output_tokens: 177`, `status: success` — et aucun contenu, aucun message assistant persisté (trace `fa002cef-46bb-4706-b40c-971a53e6a542`). Un tour qui réussit et ne livre rien. Ce n'est pas une troisième passe : la seconde n'avait rendu aucun verdict. Consigné ici parce que c'est la même classe de faux positif de livraison que mika#1996 et mika#2121 traitent côté callback, et que le prochain groomer qui la rencontre doit pouvoir la nommer au lieu de la redécouvrir.
