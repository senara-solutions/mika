# Le rescue mika#1282 ne publie plus une sonde de pilote comme si c'était une implémentation (mika#2503)

> **Parent umbrella :** mika#2491 (Défaut 7). Enfant du cadre substrat — 1 PR atomique.

## Le défaut, décomposé par la lecture du code

PR #2502 (impl de #2497) : 28 fichiers, dont `.v1probe/Cargo.toml`,
`.v1probe/src/lib.rs` et tout `.v1probe/target/` (rlib, `incremental/`,
fingerprints). Zéro implémentation. QA block 7/7. Même mécanisme que #2486 la
veille.

La chaîne, site par site :

1. **`.gitignore` ancre `/target/` à la racine.** La règle est `/target/`, pas
   `**/target/`. `.v1probe/target/` n'est donc **pas** un chemin ignoré.
2. **`git add -A` n'a pas désobéi.** Il respecte `.gitignore` — on ne lui avait
   simplement rien dit sur ce chemin-là. Le site est
   `dispatch-lib.sh::_rescue_dirty_worktree`, ligne ~3925.
3. **`RESCUE_EXCLUDE_PATHSPEC` ne porte que du scaffold `.claude/*`**
   (`commands/`, `claude-pilot.json`, `settings.local.json`, `*.local.*` —
   mika#1288, mika#1419, mika#1552). Rien sur les artefacts de build.
4. **La garde « rien à stager » existe déjà et ne voit rien.** Le bloc
   mika#1288/#1419 juste après le `git add` refuse le commit quand l'exclusion
   n'a rien laissé. Mais `.v1probe/**` n'est pas du scaffold : il reste stagé,
   la garde passe.
5. Commit `wip()`, `POST_RUN_HEAD` avancé, `RESCUED_DIRTY_WORKTREE=1`, et
   **Unit 2** (ligne ~7974) ouvre la draft PR.

## Trois rectifications que la lecture apporte au ticket

**(R1) « `source_scan` / pre-commit ne refusent pas `target/` » — le pre-commit
ne tourne pas du tout.** Le commit de rescue porte `--no-verify` **par
conception** (mika#1685, bearing Mika Prime du 2026-06-30 ~16:32Z : un nit
clippy rejetant un rescue avait échoué un pilote de 29 tours, cause modale de
wedge, n≥3). Ni durcir lefthook ni ajouter un hook n'est donc une voie ouverte —
ce serait une garde déguisée en correctif, et ce ticket ne rouvre pas cet
arbitrage. Le remède doit vivre **dans** le rescue.

**(R2) « exit 1 HEAD unchanged = pas d'implémentation à sauver » — faux en
général, et c'est le cœur.** mika#1282 existe précisément parce qu'un pilote
peut mourir exit 1 en laissant de la **vraie** implémentation non committée :
c'est son cas nominal, et la forme la plus fragile de la perte (le contenu
n'existe qu'à un seul endroit, et `_set_up_worktree` force-retire le worktree au
prochain dispatch). Conditionner sur le code de sortie détruirait exactement ce
que le rescue existe pour sauver. **Le discriminant doit porter sur le contenu,
jamais sur le code de sortie.**

**(R3) « un scan pré-PR pourrait refuser `**/target/` » — le pathspec git ne
l'exprime pas.** Mesuré sur ce dépôt (3693 fichiers trackés) :

| forme | fichiers restants | lecture |
|---|---|---|
| `git ls-files` | 3693 | référence |
| `':(glob)!**/src/**'` | **0** | **exclut tout** — un piège qui jetterait le contenu du pilote |
| `':!*src/*'` | 3203 | le `*` traverse les slashes → sur-matche `docs/target/`, `mytarget/`, `retarget/` |

Ce qui exprime **exactement** la règle voulue est `.gitignore`, où `**/target/`
a une sémantique définie par la spec git (« a leading `**` followed by a slash
means match in all directories »). La moitié `target/` va donc au `.gitignore`,
**pas** au pathspec. Le refus de la forme `:(glob)` est à écrire au site : elle
est celle qu'un futur éditeur essaiera en premier, et elle est silencieusement
destructrice.

## L'asymétrie décide toute la conception

- **Faux positif** (on refuse de rescuer du vrai contenu) → perte
  **irréversible** d'implémentation. C'est littéralement le défaut que mika#1282
  existe pour empêcher.
- **Faux négatif** (on rescue du scratch) → une PR fantôme, QA la bloque,
  l'opérateur la ferme. **Réversible**, et c'est l'état du monde aujourd'hui.

D'où deux étages de **natures différentes**, et la différence est le livrable :

- **Ce qui est retiré du staging** est destructif en pratique (le non-stagé
  disparaît au prochain `_set_up_worktree`). Réservé à ce qui est
  **certainement** régénérable : un `target/` de cargo.
- **Ce qui est refusé ne détruit rien** : le commit `wip()` est fait,
  `POST_RUN_HEAD` avancé, `_push_branch` pousse. Seule **l'ouverture de la draft
  PR** est refusée, et l'opérateur peut l'ouvrir d'un geste.

## Étage 1 — `.gitignore` : `/target/` → `**/target/`

Une ligne, et c'est le levier structurel.

- **Mesure de non-régression :** `git ls-files | grep -E '(^|/)target/'` rend
  **zéro** chemin. Aucun `target/` n'est tracké ; dans un workspace Cargo tous
  les crates partagent le `target/` racine. La règle ne peut ignorer que des
  artefacts.
- **Effet de bord recherché, et c'est le motif déjà écrit dans ce fichier pour
  `/pr-body.md` :** `git status --porcelain` — la sonde dirty-worktree que
  `_rescue_dirty_worktree` interroge — **ne rapporte pas les fichiers ignorés**.
  Donc un pilote qui ne laisse que du `target/` derrière lui ne déclenche plus
  de rescue **du tout**. Le `git add -A` est réparé sans être touché, et les
  cinq sites qui consomment `RESCUE_EXCLUDE_PATHSPEC` le sont avec lui.
- Couvre **26 des 28** fichiers de #2502.
- Le commentaire posé au-dessus de la règle doit nommer mika#2503, la sonde
  dirty-worktree, et le refus de la forme `:(glob)` de R3 — sans quoi le
  prochain éditeur refera la mesure.

## Étage 2 — `_rescue_touches_tracked_tree`, le prédicat qui voyage

**La question, et une seule :** *le contenu que cette PR publierait touche-t-il
au moins un endroit que le dépôt connaît ?*

**La mesure.** Sur `origin/main...HEAD` (la forme **trois points** — le jeu de
fichiers que GitHub montre sur la PR ; même base que `_rescue_diff_carries_work`,
par cohérence délibérée), prendre le **premier segment** de chaque chemin ; si
au moins un est présent dans `git ls-tree --name-only HEAD`, répondre oui.

| contenu | premier segment | verdict |
|---|---|---|
| `.v1probe/Cargo.toml`, `.v1probe/src/lib.rs` | `.v1probe` — absent de l'arbre | **non** → pas de PR |
| impl dans `crates/mika-agent/src/…` | `crates` — présent | oui |
| nouvelle crate `crates/mika-newthing/…` | `crates` — présent | oui |
| plan `docs/plans/…` | `docs` — présent | oui |

**Pourquoi ce prédicat et pas « une crate non déclarée dans le workspace ».**
Cette seconde formulation est vraie mais Cargo-spécifique, et dispatch-lib est
déployé dans quatre dépôts dont tous ne sont pas des workspaces Rust. Elle est
aussi plus fragile : un pilote qui écrit une crate légitime et oublie de la
déclarer verrait son implémentation refusée. Le prédicat par segment
**n'exige aucune connaissance de langage** — c'est ce qui lui permet de voyager
là où le `.gitignore` de l'étage 1 ne voyage pas.

**FAIL-OPEN, et c'est l'inverse de son voisin immédiat.** Worktree illisible,
`origin/main` absent, `git ls-tree` vide ou en échec, diff vide → **ouvrir la
PR**. `_rescue_diff_carries_work`, vingt lignes plus haut, est fail-**closed**,
et l'asymétrie inverse est délibérée : son erreur coûteuse est une fermeture
automatique de ticket que personne ne mesure, la nôtre est de **bloquer le
chemin nominal de la boucle** — `no-shipping-tail` (mika#2492, livré il y a
trois jours) passe par le même `if`. Deux prédicats voisins, deux polarités ; la
raison de chacune est écrite à son site, sans quoi un relecteur « harmonisera »
celle qu'il déplace.

**Le prédicat ne compose pas avec la liste d'incident de
`_rescue_diff_carries_work`** (`.claude/groom-verdict-trail.log`, `.iterate/`,
`docs/plans/`, …). Une question par prédicat ; et sur le cas mesuré aucun chemin
d'incident n'est en jeu, donc la composition ne changerait rien tout en rendant
deux fonctions inséparables.

### Le piège central — sans lui le correctif est inerte

Ne **pas** poser `RESCUED_DIRTY_WORKTREE=1` ne suffit pas. Le commit de rescue
avance `POST_RUN_HEAD`, donc `PRE_RUN_HEAD != POST_RUN_HEAD` devient vrai et la
branche `commit-pushed-no-pr` du calcul de `RECOVERY_CLASS` ouvre la PR quand
même. Un correctif posé au seul site du drapeau passerait tous ses tests locaux
et ne changerait rien en production.

Le refus s'applique donc **après** le calcul de `RECOVERY_CLASS`, comme terme
conjonctif du `if [ -n "$RECOVERY_CLASS" ] && … && [ -z "$PR_URL" ]`, pour les
**trois** classes. C'est aussi le bon périmètre : le prédicat porte sur le
contenu, jamais sur la classe.

### Surface opérateur — et le piège du sink, nommé plutôt que découvert

Unit 2 tourne au niveau de `dispatch_claude_pilot`, **dans le même régime que
`_check_pilot_force_push`** : son stderr est le `Stdio::piped()` de
`spawn_long_running_exec`, que l'exécuteur ne lit **que** dans sa branche
`if !status.success()`. Sur un dispatch qui réussit, le tuyau est jeté non lu —
c'est le Signal M, et la classe mika#2050 corrigée **trois fois** sur Signal S.

**Ce plan n'annonce donc aucun grep.** La surface est **`RESULT`** — le corps du
callback, qui atterrit dans `tasks.result` — exactement celle que le rescue
emploie déjà via `_compose_rescue_note`. Elle nomme le refus, la branche, le
commit, et le geste : `gh pr create --head <branch> --draft`. Un `echo … >&2`
est posé en plus, **sans être présenté comme une sonde**.

Pas d'`audit_events` : on est en shell, sans accès base — même borne que
mika#2280 sur `mika-common`.

## Changements, par fichier

1. **`.gitignore`** — `/target/` → `**/target/`, avec le commentaire qui porte
   la mesure (zéro tracké), l'effet recherché sur `git status --porcelain`, et
   le refus de R3.
2. **`skills/bundled/_shared/dispatch-lib.sh`**
   - `_rescue_touches_tracked_tree()` — nouveau prédicat, posé à côté de
     `_rescue_diff_carries_work`, doc-comment portant : la question, la mesure,
     la polarité fail-open **et sa raison**, le refus du discriminant Cargo.
   - Unit 2 — terme conjonctif sur le `if` d'ouverture, avec le commentaire qui
     nomme le piège `commit-pushed-no-pr` ci-dessus (sans quoi un futur
     déplacement vers le site du drapeau réintroduit le défaut).
   - `RESULT` — la note de refus.
   - Le message de la garde mika#1288/#1419 (« dirty worktree contained only
     scaffold paths (.claude/commands/, .claude/claude-pilot.json) ») devient
     **honnête** : il nomme ce qui a réellement été exclu. Un opérateur qui lit
     « only scaffold paths » après qu'un pilote a écrit 28 fichiers de `target/`
     cherche au mauvais endroit.
3. **`skills/bundled/_shared/tests/test_rescue_scratch_not_published.sh`** —
   nouvelle suite, sur le harnais de `test_dev_groom_dirty_rescue.sh` (repo git
   temporaire réel, appel de la **vraie** fonction ; les suites
   `test_auto_rescue_*` réimplémentent le rescue et ne peuvent donc pas
   falsifier le code livré).
4. **`Makefile`** — la suite dans `test` et dans une cible nommée, à côté de
   `test-rescue-closes-guard` / `test-rescue-pipeline-verified`. CI l'exécute
   par `make test`.

## Definition of Done

- Un pilote sorti exit 1 sans implémentation, laissant une crate de sonde et son
  `target/`, **ne produit pas de PR**.
- Son contenu n'est pas perdu : le commit `wip()` existe et la branche est
  poussée.
- Le chemin nominal du rescue (implémentation non committée sous un répertoire
  tracké) ouvre sa PR exactement comme aujourd'hui.
- `make test` passe ; la nouvelle suite est câblée et exécutée en CI.
- Le contrôle négatif a été **vu rouge** avant le correctif.

## Acceptance criteria

1. **AC1 — le cas fondateur ne publie plus.** Sur un worktree reproduisant
   #2502 (`.v1probe/Cargo.toml`, `.v1probe/src/lib.rs`, `.v1probe/target/**`,
   HEAD inchangé), le rescue ne pose pas d'ouverture de PR, pour **aucune** des
   trois valeurs de `RECOVERY_CLASS`.
2. **AC2 — rien n'est détruit.** Sur ce même cas, un commit existe après le
   rescue et `POST_RUN_HEAD != PRE_RUN_HEAD`, de sorte que `_push_branch`
   pousse la branche.
3. **AC3 — `**/target/` est ignoré à toute profondeur.** `git check-ignore`
   répond positivement pour `.v1probe/target/debug/x.rlib` et pour
   `crates/foo/target/y`, et **aucun** chemin actuellement tracké ne devient
   ignoré.
4. **AC4 — le chemin nominal est intact.** Un rescue dont le contenu touche un
   répertoire tracké (`crates/…`) ouvre sa PR ; un rescue `no-shipping-tail`
   (mika#2492) l'ouvre aussi.
5. **AC5 — le prédicat est fail-open sur chaque signal illisible.** Worktree
   vide/illisible, `origin/main` absent, `ls-tree` vide, diff vide → la PR est
   ouverte. Un terme par assertion : un test global laisserait passer une seule
   branche fail-closed.
6. **AC6 — le refus est lisible sans grep.** `RESULT` nomme le refus, la
   branche, et le geste d'ouverture manuelle.
7. **AC7 — la garde de scaffold dit la vérité.** Son message ne prétend plus
   « only scaffold paths » quand ce n'est pas ce qui a été exclu.
8. **AC8 — le piège est épinglé.** Une assertion échoue si le refus est déplacé
   au seul site de `RESCUED_DIRTY_WORKTREE` et cesse de couvrir
   `commit-pushed-no-pr`.

## Fire-Disposition

Ce plan livre des détecteurs au sens de mika#2306 : la suite de tests du point 4
ci-dessus, et les assertions structurelles d'AC8.

**Disposition retenue : (a) exception nommée en allowlist — avec une allowlist
vide, et c'est une mesure, pas une commodité.**

- Ces détecteurs sont des tests de **comportement** sur une fonction, dont la
  population est **construite par le test lui-même** dans un repo git temporaire.
  Ils ne scannent aucune population préexistante du dépôt, donc il n'existe
  aucune violation à excepter et aucune entrée d'allowlist à écrire.
- Le seul détecteur qui touche une population réelle est **AC3**, et sa
  population est mesurée à **zéro** : `git ls-files | grep -E '(^|/)target/'`
  rend zéro chemin. L'implémentation doit **refaire cette mesure** et, si elle
  rend autre chose que zéro, s'arrêter et remonter — c'est la seule branche où
  ce plan bascule en option (c).
- Les assertions structurelles d'AC8 s'ajoutent aux assertions existantes de
  `test-dispatch-lib.sh` (lignes ~1799-1808 et ~3194-3203) sans en modifier
  aucune : elles ne peuvent donc pas hériter d'une population.
- **Livrés armés, et vus rouges d'abord.** Aucun `#[ignore]`, aucune cible de
  test désarmée : l'option (b) supposerait une population inconnue, et ici elle
  est connue et vide. La contrepartie est le contrôle négatif obligatoire du
  contrat de vérification.

## Contrat de vérification

- **Contrôle négatif, vu rouge avant toute chose.** La nouvelle suite doit
  échouer sur le `dispatch-lib.sh` **non modifié** (le rescue de #2502 publie).
  Un test qui n'a jamais été vu rouge n'atteste rien — c'est le geste que
  `test-shell-exec-guard.sh` a dû poser pour la même raison.
- **Contrôle positif de l'étage 1** : `git check-ignore -v
  .v1probe/target/debug/x.rlib` nomme la règle `**/target/`.
- **Contrôle de non-régression de l'étage 1** : le jeu de fichiers trackés est
  identique avant et après (`git ls-files` inchangé, `git status` propre).
- **AC4 est le contrôle positif de l'étage 2** : sans lui, un prédicat qui
  refuserait **tout** passerait AC1, AC2 et AC5.
- `make test` et `make test-dispatch-lib` verts.

## Sondes post-déploiement, et leurs haltes

1. **Symptôme (7 jours).** Aucune PR ouverte par la boucle ne porte de chemin
   sous un `target/` ni de crate de sonde. Croiser avec les PR de la boucle,
   pas avec l'ensemble des PR.
   **Halte 1 — une telle PR réapparaît.** Ne **pas** élargir le prédicat par
   réflexe : établir d'abord si le dispatch a servi un `dispatch-lib.sh`
   antérieur au correctif — `~/.mika/skills/` est une projection du binaire, et
   un fichier édité dans l'arbre est invisible jusqu'au `make deploy`
   (classe mika#2340). *Une garde qu'on n'a pas déployée se lit exactement comme
   une flotte saine.*
2. **Attribution (7 jours).** Les `RESULT` de refus (grep sur `tasks.result`,
   via `mika tasks get` ou la requête d'audit du callback). **Régime attendu :
   non vide et faible.** Chaque ligne est une PR fantôme qui n'a pas été ouverte.
   **Halte 2 — le compte porte du trafic nominal** (plusieurs par jour, sur des
   tickets différents). Le prédicat est alors trop large, ou les pilotes
   travaillent massivement hors de l'arbre tracké : lire **quels** premiers
   segments sont refusés avant de toucher au prédicat. *Ceci est un filet, pas
   un chemin* — s'il porte le trafic nominal, il a en plus effacé le signal qui
   permettrait de le voir.
   **Halte 3 — un refus sur un contenu légitime.** Ouvrir la PR à la main
   (le commit est poussé, rien n'est perdu), puis lire le premier segment en
   cause : c'est une décision de périmètre, pas un seuil à régler.
3. **Contrôle négatif de la boucle (7 jours).** Les PR `no-shipping-tail`
   continuent d'être ouvertes. Zéro sur une semaine active signifie que le terme
   conjonctif mord sur le chemin nominal — **désarmer d'abord** (retirer le
   terme), diagnostiquer ensuite.

## Ce que ce travail n'achète pas

- **Il n'empêche aucun pilote d'écrire une crate de sonde.** Il l'empêche de
  devenir une PR. Le geste du pilote est une question de prompt, et
  `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` dit ce
  que vaut une contrainte de prompt sur le substrat de la boucle.
- **Il ne couvre pas un `target/` sous un répertoire tracké dans un dépôt dont
  le `.gitignore` ne porte pas la règle** (p. ex. `crates/foo/target/` sur
  mika-cloud) : l'étage 1 est par dépôt, et l'étage 2 verrait `crates` tracké et
  ouvrirait. Limite **nommée, non mesurée** — sa sonde est le point 1 ci-dessus,
  et son remède, s'il devient nécessaire, est la même ligne de `.gitignore` dans
  le dépôt concerné, jamais un mécanisme Cargo dans dispatch-lib.
- **Aucun compteur, aucun événement d'audit.** La seule surface est `RESULT`, et
  son silence ne prouve rien tant que personne ne le lit.
- **Il ne touche pas `--no-verify`** (mika#1685, bearing ratifié) ni la
  `RESCUE_EXCLUDE_PATHSPEC` (R3 : la forme pathspec est le mauvais outil pour
  cette règle).

## Hors périmètre, délibérément

- Le durcissement de lefthook / l'installation d'un pre-commit sur la machine de
  dispatch (R1 — arbitrage mika#1685, non rouvert ici).
- `_rescue_diff_carries_work` et sa liste d'incident (mika#2157) : population
  différente, polarité différente, ticket différent.
- `_rescue_verify_pipeline` et le marqueur `rescue-pipeline-verified`
  (mika#2354) : le refus de l'étage 2 est placé **avant** eux, donc ils ne
  tournent pas sur un rescue refusé — c'est une économie, pas une modification.
- Le classement rescue-class de QA (mika#1282/#1618, chaînon nommé dans
  mika#2334) : même voisinage, autre défaut.
- #2486 : cité par le ticket comme « même mécanisme », **non re-mesuré ici**
  (`gh` n'est pas authentifié dans le bac à sable de dispatch). Le correctif est
  dérivé de #2502, qui est mesuré fichier par fichier.
