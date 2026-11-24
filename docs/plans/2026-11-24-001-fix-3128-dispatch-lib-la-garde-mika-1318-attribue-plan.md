# fix(3128) — la garde mika#1318 attribue au pilote la suppression humaine de la branche distante

**Ticket :** senara-solutions/mika#3128
**Origine :** parking mika#2983, commentaire 5 (« supprimer la branche d'un groom en vol
est lu comme un push du pilote »)
**Site :** `skills/bundled/_shared/dispatch-lib.sh::_check_pilot_force_push` (l. 8844),
terme fautif l. 8870
**Occurrence mesurée :** une (#3004, 2026-11-01)

---

## Le défaut, tel qu'il se mesure

`_check_pilot_force_push` est la garde de périmètre d'autorité de mika#1318 : un pilote
`dev-groom` écrit du contenu, et le push appartient à `_push_branch`. Elle lit le SHA de la
ref distante avant le vol (`PRE_RUN_REMOTE_HEAD`, l. 6440) et après (l. 8866), puis conclut
par **un seul terme** :

```sh
if [ "${PRE_RUN_REMOTE_HEAD:-}" = "${post_remote_head:-}" ]; then   # l. 8870
    ... return 0        # propre
fi
PUSH_VIOLATION_DETECTED=1                                            # tout le reste
```

Tout ce qui n'est pas l'égalité est donc attribué au pilote. Or la ref distante peut aussi
**disparaître**, et la disparition n'est pas une avancée.

**Le 2026-11-01, #3004.** Le groom `03e31776` démarre à 16:03:51Z sur
`fix/3004/dispatch-un-tour-pr-review-de-mika-dev`, branche alors présente sur `origin` à
`8da087abd` (draft #3009). À 16:04:57Z — **66 secondes plus tard, le vol en cours** —
l'opérateur ferme #3009 sans merge et supprime la branche. À 16:14:54Z, la garde lit
`post_remote=<none>`, le terme d'égalité est faux, et dispatch-lib rend :

```
STRUCTURAL VIOLATION: pilot push detected (mika#1318) …
Evidence: pre_remote=8da087abd post_remote=<none>
Outcome: PIPELINE_INCOMPLETE — push violation
```

Le pilote n'avait rien poussé. Et le coût n'est pas la phrase fausse : au site d'appel
(l. 14377) la garde **court-circuite `_iterate_groom_loop` et `_push_branch`** puis délivre
le callback. Le plan 005 était commité en local (`22d9f8bc3`) et n'a jamais vu l'architecte.
Un vol de groom entier perdu sur une suppression de branche faite par un humain.

### Pourquoi c'est bien un défaut d'attribution, et pas une politique à durcir

Trois lectures, dans cet ordre :

1. **La question posée par la garde est « la ref distante a-t-elle avancé vers du contenu
   produit par le pilote ? ».** Une suppression ne produit aucun contenu : il n'existe, après
   elle, aucun commit nulle part qu'on puisse attribuer à qui que ce soit. La garde répond
   donc sur un prédicat (« le SHA a changé ») plus large que sa question.
2. **La population où `post_remote=<none>` désigne le pilote est, sous le défaut armé,
   vide.** Le seul geste du pilote qui produirait cette lecture est `git push --delete` /
   `git push origin :<branch>`, et la borne d'egress de mika#2736 le refuse en 403
   `push-ref-delete` (`scripts/mika-pilot-github-auth-addon.py` l. 468, 501 ;
   `MIKA_PILOT_EGRESS_PUSH_BOUND`, **armée par défaut**). C'est un argument **secondaire** et
   il est écrit comme tel : la borne est désarmable, donc elle ne porte pas la décision — elle
   la conforte.
3. **Haleter ne récupère rien et coûte tout.** La ref est déjà partie, et aucune garde
   post-vol ne la ramène. L'arrêt ne protège donc rien ; il détruit le vol. C'est la
   dissymétrie exacte de #3004.

### Ce que l'aval sait déjà faire d'une branche distante absente

Le correctif ne lâche pas le vol dans un état non modélisé : `_push_branch` porte déjà
**deux** lecteurs de ce fait, écrits pour d'autres incidents.

- `remote_known_absent` + la ligne `Stale-ref:` (mika#2626, l. 9052-9056) : la ref de suivi
  locale survit à une branche distante supprimée, et le push repart en first-push en le
  disant.
- `push_branch.skipped_empty` (mika#3038, l. 9058-9089) : quand le distant n'a plus la
  branche **et** que l'arbre de HEAD égale `origin/main`, la branche n'est pas recréée — rien
  à publier.

Laisser le vol continuer l'amène donc dans un chemin déjà conçu, pas dans un trou.

### La contrainte de surface : stderr ne sera pas lu

`_check_pilot_force_push` tourne au niveau de `dispatch_claude_pilot`, **hors** de la
redirection `2>"$STDERR_FILE"`. Son stderr est le `Stdio::piped()` de
`spawn_long_running_exec`, que l'exécuteur ne lit **que** dans `if !status.success()`. Sur un
dispatch qui réussit, le tube est jeté sans être lu — c'est le Signal M de mika#2050,
documenté comme halte dans `docs/operator/environment-variables.md` l. 194 et re-nommé
l. 264. Un fait qui ne vivrait que sur `stderr` ne serait donc lu par personne.

Conséquence portante pour ce plan : **la disparition de la ref va dans `RESULT`**, à côté des
lignes `Stale-ref:` et `Remote-probe:` que `_push_branch` y écrit pour cette raison précise
(l. 9030-9040, l. 9048-9051).

---

## Requirements

- **R1 — La lecture post-vol rend trois issues, pas deux.** `présente` (avec son SHA),
  `absente`, `indéterminée`. Elles viennent du **contrat de `git ls-remote --exit-code`**
  (`rc=0` présente, `rc=2` aucune ref ne correspond, autre : le distant n'a pas répondu), pas
  d'une inférence sur un `stdout` vide.
- **R2 — Une suppression n'est pas un push.** `pre=présente` et `post=absente` ⇒
  `PUSH_VIOLATION_DETECTED` reste vide, la garde rend `0`, le vol continue vers
  `_iterate_groom_loop` puis `_push_branch`.
- **R3 — La suppression est NOMMÉE, dans `RESULT`.** Une ligne de prose qui dit quelle ref a
  disparu, à quel SHA elle était au lancement, et que ce n'est pas attribué au pilote. Plus
  un marqueur `stderr` au format maison `pilot_push_guard.*` (convenance, jamais annoncé
  comme sonde — Signal M).
- **R4 — Le pré-vol distingue « absente » de « non mesurée ».** `PRE_RUN_REMOTE_HEAD` est
  aujourd'hui vide dans les deux cas (`2>/dev/null … || true`, l. 6440), donc un `ls-remote`
  de pré-vol en échec réseau fait lire « la branche est apparue ⇒ violation » à une branche
  qui était là depuis le début. C'est la moitié symétrique du même défaut : la garde ne sait
  pas distinguer « je n'ai pas lu » de « il n'y avait rien ». Sur `non mesurée`, la garde
  rend `0` sans accuser — règle maison mika#2277, un signal qu'on ne peut pas lire n'est
  jamais un terme satisfait.
- **R5 — Ce qui était violation le reste.** `pre=présente` + `post=présente` + SHA différent
  (avancée ou réécriture) ⇒ violation. `pre=absente` + `post=présente` (le pilote a créé la
  branche) ⇒ violation. Le fail-open réseau post-vol ⇒ `0`. Le hors-périmètre (`dev-pilot`,
  mode texte libre) ⇒ `0`. Aucune de ces quatre issues ne change d'avis.
- **R6 — `post_remote=<none>` ne peut plus apparaître dans `PUSH_VIOLATION_EVIDENCE`.**
  C'est la forme exacte de l'evidence de #3004 ; son absence structurelle est l'assertion
  qui dit que le défaut est fermé. Le `pre_remote=<none>` du côté gauche, lui, **reste** —
  il désigne R5 cas 2, qui est une vraie violation.

---

## Design

### D1 — La table des issues, et où elle vit

Un **seul** `ls-remote` post-vol répond aux deux questions (existence et SHA) en une
lecture. C'est délibéré : deux appels ouvriraient une fenêtre TOCTOU entre
« existe-t-elle ? » et « à quel SHA ? » sur exactement la race que ce ticket corrige.

```
rc de `ls-remote --exit-code`  | issue post-vol | SHA
------------------------------ | -------------- | -----------------
0                              | présente       | cut -f1 du stdout
2                              | absente        | — (aucun)
autre (128, …)                 | indéterminée   | — (fail-open)
```

Puis la matrice de décision, pré-vol × post-vol :

| pré-vol \ post-vol | présente, même SHA | présente, autre SHA | absente | indéterminée |
|---|---|---|---|---|
| **présente** | propre (`0`) | **VIOLATION** (`1`) | **suppression** → `0` + `Vanished-ref:` | fail-open (`0`) |
| **absente** | n/a | **VIOLATION** (`1`) | propre (`0`) | fail-open (`0`) |
| **non mesurée** | fail-open (`0`) | fail-open (`0`) | fail-open (`0`) | fail-open (`0`) |

La ligne « non mesurée » est **entièrement** fail-open, et le terme est évalué **avant** la
lecture post-vol : sans pré-vol, il n'y a pas de comparaison à faire, donc pas de
round-trip réseau à dépenser.

### D2 — Le pré-vol gagne son état

Au site de capture (l. 6436-6444, dans la branche `repo#number` et son `else` texte libre),
`PRE_RUN_REMOTE_HEAD` est conservé tel quel — ses deux seuls lecteurs sont la garde et son
evidence — et un **nouveau global** le qualifie :

```sh
PRE_RUN_REMOTE_STATE=present|absent|unread
```

Un global nommé plutôt qu'une sentinelle glissée dans le champ SHA (`"<unread>"`) : un champ
qui porte parfois un SHA et parfois un mot est exactement ce qui fait écrire
`post_remote=<none>` dans une phrase qui parle de push. En mode texte libre, l'état est posé
vide, et la garde traite le vide comme `unread` — un état non initialisé n'accuse personne.

### D3 — La disposition de la suppression

Rendre `0`, et écrire le fait aux deux surfaces :

- `stderr` : `pilot_push_guard.branch_deleted: …` — même préfixe que `.clean` / `.violation`,
  parce que c'est le préfixe que la doc opérateur grep (l. 194) et que rien ne **parse** ces
  lignes (vérifié : les trois seules occurrences hors docs sont les `echo` eux-mêmes ;
  `scripts/canonical-tokens.tsv` ne les déclare pas, et n'a pas à les déclarer — aucun
  lecteur machine, donc aucun site de match).
- `RESULT` : une ligne `Vanished-ref: …` nommant la branche, le SHA de lancement, et
  l'attribution. Prose, pas de token machine : les deux lecteurs de `RESULT` sont
  `grep -qF "PIPELINE FAILURE:"` (l. 8802) et `^Outcome: ` ; une ligne de prose ne touche ni
  l'un ni l'autre. Vérifié aussi contre `RE_NO_PR` de `dispatcher.rs` (classe B) : le
  préfixe n'est ni `PR:` ni `NO_PR:`.

**Ce que la ligne ne fait pas :** elle ne juge pas s'il faut recréer la branche. C'est la
décision de `_push_branch`, qui la prend déjà sur `remote_known_absent` + l'arbre
(mika#3038). Dupliquer ce jugement ici créerait les deux prédicats divergents que mika#2626
a passé un ticket entier à réunir.

### D4 — Le voisinage documentaire

Deux commentaires du fichier **affirment** le comportement corrigé et doivent bouger avec
lui, sinon le prochain lecteur fera confiance au commentaire :

- L'en-tête de la garde (l. 8868-8869) : « *if remote state changed between pre-run and
  post-run, the pilot pushed (any push, not just force-push, is a violation)* ». C'est
  littéralement la phrase que #3004 a démentie.
- Le bloc « **NE PAS unifier** avec les deux autres `ls-remote` du fichier … ils lisent le
  SHA, pas l'existence. Question différente, prédicat différent. » (l. 8930-8932). Après ce
  vol, la garde lit **les deux**. La consigne de non-unification **tient toujours**, mais sa
  raison change et doit être réécrite : non plus « question différente » mais « une seule
  lecture atomique répond aux deux, et appeler `_remote_branch_exists` ici rouvrirait la
  fenêtre TOCTOU ». Un « ne pas faire » dont la raison a cessé d'être vraie se fait défaire.

### D5 — La doc opérateur

`docs/operator/environment-variables.md`, Signal M (l. 194) dit aujourd'hui : « *Any change
indicates the pilot pushed* » et « `pilot_push_guard.violation` … *should never appear* ».
La première phrase est le défaut ; la seconde est démentie par #3004. Le Signal gagne la
troisième sous-issue (`branch_deleted`), la correction de l'invariant, et le pointeur vers la
ligne `Vanished-ref:` du `RESULT` — qui est, elle, la surface que l'opérateur lit vraiment,
la halte Signal M restant entière pour les trois lignes `stderr`. Hors compte de lignes.

---

## Périmètre — ce vol

**Dans le périmètre :** la table des issues post-vol, l'état du pré-vol, le bras suppression
et ses deux surfaces, les deux commentaires de D4, la suite de tests, le Signal M.

**Hors périmètre, nommé :**

- **Lire l'auteur de l'événement de suppression** — la seconde branche de la question du
  parking. Écartée, et pas par économie : un `ls-remote` ne porte aucun auteur, et le lire
  demanderait l'API des événements du dépôt (`gh api`) sur le chemin de la boucle, pour une
  décision déjà prise par la forme du fait. On n'a pas besoin de savoir *qui* a supprimé pour
  savoir qu'**une suppression n'est pas un push**.
- **Le force-push invisible côté proxy** — limite connue et écrite
  (`environment-variables.md` l. 9938) : `receive-pack` ne porte aucun drapeau de force. Ce
  vol ne la touche pas.
- **Le comportement de `_push_branch` face à une branche recréée** — déjà conçu (mika#2626,
  mika#3038), inchangé ici.

---

## Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | `pre=présente`, `post=absente` ⇒ `rc=0`, `PUSH_VIOLATION_DETECTED` vide | nouveau cas dans `test_force_push_guard.sh`, mock `ls-remote` à `rc=2` |
| V2 | le même cas écrit `Vanished-ref:` dans `RESULT`, avec la branche et le SHA de pré-vol | `assert_contains` sur `$RESULT` |
| V3 | **contrôle négatif de V1** : `pre=présente`, `post=présente` SHA différent ⇒ `rc=1`, flag posé | cas 2 existant, **laissé mot pour mot** — c'est ce qui sépare « la garde distingue » de « la garde est désarmée » |
| V4 | **contrôle négatif** : `pre=absente`, `post=présente` ⇒ `rc=1` (R5 cas 2) | cas 6 existant, adapté au nouveau mock, evidence `pre_remote=<none> post_remote=def456` inchangée |
| V5 | R6 : `post_remote=<none>` n'apparaît dans `PUSH_VIOLATION_EVIDENCE` sur **aucun** cas de la suite | assertion d'absence, exécutée après le dernier cas |
| V6 | `pre=non mesurée`, `post=présente` SHA quelconque ⇒ `rc=0`, flag vide | nouveau cas ; sans lui le correctif de R4 n'est pas mesuré |
| V7 | le fail-open réseau post-vol (`rc=128`) rend toujours `0` | cas 7 existant, inchangé |
| V8 | `dev-pilot` et le mode sans worktree sortent toujours en `0` | cas 3 et 4 existants, inchangés |
| V9 | `pre=absente`, `post=absente` ⇒ `0` (branche jamais poussée, vol nominal de premier groom) | cas 5 existant, adapté au nouveau mock |
| V10 | la suite tourne dans le job CI qui la porte déjà | `.github/workflows/ci.yml` l. 719, `dispatch-lib-offline-suites` — aucun câblage à ajouter, donc rien à déclarer dans `scripts/suite-wiring.tsv` |
| V11 | `bash -n skills/bundled/_shared/dispatch-lib.sh` et `shellcheck` restent propres | lints du dépôt |

**Note sur le mock.** Le `git()` de la suite intercepte `ls-remote` et rend
`MOCK_LS_REMOTE_OUTPUT` / `MOCK_LS_REMOTE_EXIT`. Il faut lui ajouter la sémantique `rc=2`
(absente) distincte de `rc=128` (indéterminée) : aujourd'hui tout `MOCK_LS_REMOTE_EXIT`
non nul est un échec indifférencié. C'est le seul changement structurel du harnais.

---

## Definition of Done

1. `_check_pilot_force_push` décide sur la matrice de D1 ; les trois issues post-vol viennent
   du `rc` de `ls-remote --exit-code`, en **un** appel.
2. `PRE_RUN_REMOTE_STATE` est posé aux deux sites de capture (`repo#number` et texte libre) et
   lu par la garde avant toute requête réseau.
3. Le bras suppression rend `0`, pose la ligne `Vanished-ref:` dans `RESULT` et le marqueur
   `pilot_push_guard.branch_deleted` sur `stderr`.
4. Les quatre issues de R5 rendent exactement ce qu'elles rendaient avant ce vol.
5. `test_force_push_guard.sh` passe, 11 points du Verification Contract couverts, dont les
   trois contrôles négatifs V3/V4/V9.
6. Les deux commentaires de D4 sont réécrits ; aucun commentaire du fichier n'affirme plus
   qu'un changement de ref distante implique un push du pilote.
7. Signal M de `docs/operator/environment-variables.md` à jour.
8. `cargo fmt`, `cargo clippy`, `cargo test`, les lints du dépôt et
   `scripts/verify-pipeline.sh origin/main` passent.

---

## Acceptance criteria

Le corps du ticket ne porte pas de section « Critères d'acceptation » ; ceux-ci sont dérivés
des Requirements et du Verification Contract ci-dessus.

- **AC1.** Sur un dispatch `dev-groom` dont la ref distante de la branche existait au
  lancement et a disparu pendant le vol, `_check_pilot_force_push` rend `0`, ne pose pas
  `PUSH_VIOLATION_DETECTED`, et le `RESULT` ne contient ni `STRUCTURAL VIOLATION: pilot push
  detected` ni `Outcome: PIPELINE_INCOMPLETE — push violation`. Le vol atteint
  `_iterate_groom_loop`. (R2 ; V1, V3)
- **AC2.** Le même dispatch porte dans son `RESULT` une ligne nommant la branche, le SHA
  qu'elle portait au lancement, et le fait que la disparition n'est pas attribuée au pilote.
  (R3 ; V2)
- **AC3.** Contrôle négatif, dans la même suite : une ref distante qui **avance** vers un
  autre SHA pendant le vol reste une violation (`rc=1`, flag posé, evidence capturée), et une
  branche **apparue** pendant le vol reste une violation. (R5 ; V3, V4)
- **AC4.** La chaîne `post_remote=<none>` — la forme exacte de l'evidence de #3004 —
  n'apparaît dans `PUSH_VIOLATION_EVIDENCE` sur aucun cas de la suite. (R6 ; V5)
- **AC5.** Un `ls-remote` de **pré-vol** qui n'a pas répondu ne produit plus de violation,
  quel que soit l'état post-vol. (R4 ; V6)
- **AC6.** Les issues hors périmètre — `dev-pilot`, mode texte libre, fail-open réseau
  post-vol, branche absente des deux côtés — rendent `0` comme avant. (R5 ; V7, V8, V9)

---

## Fire-Disposition

Ce vol livre des détecteurs : la suite `test_force_push_guard.sh` (cinq cas conservés, trois
ajoutés, une assertion d'absence) et la garde `_check_pilot_force_push` elle-même.
**Option (a) : exception nommée en allowlist, avec une population d'exceptions vide par
construction.**

Pourquoi vide, et pourquoi par construction : ce vol **rétrécit** un détecteur existant. La
population violante d'un détecteur rétréci est un sous-ensemble de celle du détecteur
d'avant — tout ce que la garde accusait et qu'elle doit continuer d'accuser continue de
l'être (R5, tenu par les contrôles négatifs V3, V4, V9), et la seule issue qui change est
précisément le faux positif que le vol corrige (V1). Il n'existe donc aucune donnée
préexistante à exempter : pas d'entrée d'allowlist à écrire, et aucune ne doit l'être.

L'audit qui le prouve est le Verification Contract lui-même : **aucun** cas vert de la suite
d'aujourd'hui ne devient rouge. Les cas 2, 3, 4, 7 sont repris mot pour mot ; les cas 5 et 6
ne changent que par la sémantique du mock (`rc=2` au lieu d'un `stdout` vide), pas par leur
verdict attendu. Le détecteur atterrit donc **armé**, sans `#[ignore]`, sans suivi à armer,
et sans halte à remonter.

La seconde moitié de R4 — le pré-vol non mesuré — élargit le fail-open, donc rétrécit encore
la population accusée. Même raisonnement, même conclusion.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — `_check_pilot_force_push` : lecture `--exit-code` à trois issues, terme du pré-vol, matrice de décision, commentaire d'en-tête réécrit | 70 |
| `skills/bundled/_shared/dispatch-lib.sh` — bras suppression : marqueur `stderr` + ligne `Vanished-ref:` dans `RESULT`, et pourquoi les deux surfaces | 22 |
| `skills/bundled/_shared/dispatch-lib.sh` — `PRE_RUN_REMOTE_STATE` aux deux sites de capture (l. 6436-6444) | 18 |
| `skills/bundled/_shared/dispatch-lib.sh` — commentaire « NE PAS unifier » de `_remote_branch_exists` réécrit (D4) | 8 |
| tests (`skills/bundled/_shared/tests/test_force_push_guard.sh`) — mock `rc=2` distinct de `rc=128`, helper `assert_contains` | 25 |
| tests (`skills/bundled/_shared/tests/test_force_push_guard.sh`) — trois cas ajoutés (V1/V2, V6, V5) et deux cas adaptés au mock (V4, V9) | 75 |

Total estimé : 218 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 218 < 279, et la marge n'est pas mince par chance : le vol
touche **un** fichier de substrat et **un** fichier de test, aucune compilation Rust n'est
dans son chemin critique (la suite est du bash pur, sur mocks, sans réseau), et les deux
tiers du changement sont concentrés dans une fonction de 37 lignes dont la totalité est déjà
lue ci-dessus. Le Signal M de `environment-variables.md` est hors compte (règle : hors
`docs/`) et ne coûte qu'une édition. Aucune phase, aucune suite, aucun `Tracked in:`.

---

## Ordre de commit

1. **`fix(dispatch-lib): la garde mika#1318 lit trois issues de la ref distante, pas deux`**
   — D1 + D2 + D4 : la matrice, `PRE_RUN_REMOTE_STATE`, les deux commentaires. La garde ne
   change pas encore de verdict sur la suppression ; elle sait seulement la distinguer.
   *Plancher :* `bash -n` propre, suite existante verte (les cinq cas conservés le sont par
   construction à ce commit).
2. **`fix(dispatch-lib): une suppression de ref distante n'est pas un push du pilote (mika#3128)`**
   — D3 : le bras suppression, ses deux surfaces. C'est le commit qui change le verdict de
   #3004.
   *Plancher :* la suite étendue verte, V1 à V9.
3. **`test(dispatch-lib): la suppression, le pré-vol non mesuré et l'absence de post_remote=<none>`**
   — la suite : mock à trois rc, trois cas ajoutés, deux adaptés, l'assertion d'absence de
   R6. Séparé du 2 pour que le diff des tests soit lisible seul en revue ; les deux commits
   partent dans la même PR et la suite est verte à chacun des trois.
   *Plancher :* `bash skills/bundled/_shared/tests/test_force_push_guard.sh` → 0 échec.
4. **`docs(operator): Signal M distingue la suppression de ref du push du pilote (mika#3128)`**
   — D5.
   *Plancher :* `scripts/verify-pipeline.sh origin/main`.
