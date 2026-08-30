---
title: dev-groom doit préserver un arbre sale avant de débloquer - Plan
type: fix
date: 2026-08-30
issue: senara-solutions/mika#2031
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# fix(dispatch-lib): dev-groom n'a pas de sauvetage d'arbre sale, donc un plan non commité est perdu

**Ticket:** mika issue#2031 — milestone « Substrat de boucle », p2-normal

---

## Goal Capsule

- **Objective.** Un plan écrit par un pilote `dev-groom` mais jamais commité doit survivre à la fin de la session, et le callback doit dire ce qui a été préservé et où. Un arbre propre ne doit rien déclencher.
- **Means.** Le filet mika#1282 existe déjà et fait exactement le bon travail ; il est simplement gardé sur `SKILL = dev-pilot`. On l'extrait de `_post_flight_recovery` en une fonction nommée, on ouvre sa garde à `dev-groom`, on différencie ses sorties par skill (message de commit, disposition de PR, texte du callback), et on lui fait nommer le commit et la branche en plus des fichiers.
- **Ordre non négociable — préserver d'abord, débloquer ensuite.** Le contenu non commité n'existe nulle part ailleurs : ni sur une branche, ni sur un distant. Le sauvetage commit AVANT que quoi que ce soit d'autre ne juge, ne pousse, ni ne nettoie. Un sauvetage qui commencerait par nettoyer pour pouvoir continuer aurait inversé la priorité.
- **Un sauvetage muet est presque une suppression.** `Saved working directory and index state WIP on main` ne dit à personne qu'il y a quelque chose à récupérer. La note de sauvetage nomme les fichiers, le sha du commit et la branche.
- **Stop conditions.** Ne pas ouvrir de PR pour `dev-groom` — sa sortie est un plan sur la branche, pas une PR ; `RESCUED_DIRTY_WORKTREE` reste réservé à `dev-pilot`. Ne pas toucher au chemin `dev-pilot` autrement que par l'ajout de l'identification commit+branche. Ne pas retirer `--no-verify` (mika#1685). Ne pas élargir les exclusions de scaffold.
- **Execution profile.** Bash uniquement : `skills/bundled/_shared/dispatch-lib.sh` + une suite d'assertions nouvelle dans `skills/bundled/_shared/tests/`. Aucun redéploiement requis pour prouver le comportement — la fonction extraite est appelable en isolation contre un vrai dépôt git temporaire.
- **Tail ownership.** PR sur `fix/2031/dispatch-lib-dev-groom-has-no-dirty`, **`Closes #2031`**, reviewer `mika-platform-qa`.

---

## Product Contract

### Summary

Le sauvetage d'arbre sale de dispatch-lib ne s'exécute que pour `dev-pilot`. Un pilote `dev-groom` tué après avoir écrit son plan mais avant `git commit` n'a rien de commité, rien de poussé — et `_set_up_worktree` force-remove le worktree au dispatch suivant. Le plan disparaît. Ce plan ouvre le filet à `dev-groom` et lui fait dire ce qu'il a sauvé.

### Problem Frame

La garde, telle qu'elle est aujourd'hui (`dispatch-lib.sh`, bloc « Unit 1 (mika#1282) ») :

```bash
if [ "$PRE_RUN_HEAD" = "$POST_RUN_HEAD" ] && [ "$SKILL" = "dev-pilot" ] && [ -n "$WORKTREE_DIR" ]; then
```

Trois faits rendent la perte définitive plutôt que réparable :

| Fait | Conséquence |
|---|---|
| Rien n'est commité | Le contenu n'est sur aucune branche |
| Rien n'est poussé | Le contenu n'est sur aucun distant |
| `_set_up_worktree` force-remove le worktree au dispatch suivant de la même branche | Le contenu n'est plus sur le disque |

`_find_issue_plan` parcourt le **système de fichiers** du worktree, donc à l'intérieur d'un run le plan non commité est bel et bien *trouvé* : `VALID_PLAN` est renseigné. La perte se produit **entre** les runs. C'est aussi ce qui produit le second défaut, adjacent et de la même veine : sur HEAD-inchangé avec `VALID_PLAN` non vide, le callback affirme « the plan for `repo#N` is already committed » — sur la foi d'une mesure du système de fichiers qui ne peut pas répondre à cette question. La phrase est fausse dans exactement le cas où le sauvetage est nécessaire.

Grooming est la phase la plus exposée : c'est le premier dispatch sur une branche neuve, et toute sa production tient en un seul fichier markdown. Le coût d'une perte est un cycle de dispatch complet (~45 min) plus une repasse d'architecte depuis zéro.

**Lignée.** PR#2028 a réduit le danger adjacent — une session terminée qui a laissé du travail ne saute plus `_post_flight_recovery`, donc le filet est désormais *atteint*. Il ne se déclenche simplement pas pour ce skill. Et l'incident du 2026-08-29 (worktree à six commits locaux, sauvé seulement parce qu'une garde comparait HEAD à la tête de la PR) fixe la direction : le travail non commité est la forme la plus fragile de ce risque.

### Ce que « préservé » veut dire

Trois conditions, toutes nécessaires :

| # | Condition | Mesure |
|---|---|---|
| **P1** | Le contenu est dans un commit | `HEAD` a avancé et le fichier est suivi à `HEAD` |
| **P2** | Le commit est joignable | Le commit est sur `$BRANCH`, que `_push_branch` publie ensuite |
| **P3** | Quelqu'un sait qu'il existe | Le callback nomme les fichiers, le sha et la branche |

Un sauvetage qui atteint P1 et P2 mais pas P3 est le « stash muet » : le contenu existe, personne ne va le chercher.

---

## Requirements

- **R1.** Le sauvetage d'arbre sale s'exécute pour `dev-groom` comme pour `dev-pilot`, sur HEAD-inchangé et worktree sale.
- **R2.** Les exclusions de chemins scaffold (`.claude/commands/`, `.claude/claude-pilot.json`, `.claude/settings.local.json`, `.claude/*.local.*`) s'appliquent inchangées.
- **R3.** Un arbre **propre** ne produit aucun commit, aucune note, aucun effet de bord — pour les deux skills.
- **R4.** Le sauvetage `dev-groom` n'ouvre **aucune** PR. `RESCUED_DIRTY_WORKTREE` — le drapeau qui déclenche la draft PR de l'Unit 2 — reste posé pour `dev-pilot` seul.
- **R5.** Après un sauvetage `dev-groom` réussi, `POST_RUN_HEAD` avance, donc `_push_branch` publie le commit et la suite du grooming (convergence architecte) peut s'exécuter. **Débloquer**, après avoir préservé.
- **R6.** La note de sauvetage nomme : les fichiers réellement mis en index, le sha court du commit de sauvetage, et la branche locale. Pour les deux skills.
- **R7.** Le callback `dev-groom` ne préfixe pas un sauvetage réussi par `PIPELINE FAILURE:` — le sauvetage a fonctionné et le grooming continue. Un sauvetage *échoué* reste un `PIPELINE FAILURE:` avec l'arbre laissé sale pour inspection.
- **R8.** La phrase « the plan ... is already committed » n'est émise que si le plan est effectivement commité sur la branche ; sinon le callback dit qu'il est présent dans le worktree mais non commité, ce que le sauvetage traite ensuite.
- **R9.** Le comportement `dev-pilot` est inchangé hors R6 : même message de commit, même `--no-verify`, même `cargo fmt` proactif, même draft PR.

---

## Technical Design

### D1 — Extraire le bloc en `_rescue_dirty_worktree()`

Le bloc de sauvetage vit aujourd'hui inline dans `_post_flight_recovery`, ce qui le rend inatteignable par un test : les suites existantes (`test_auto_rescue_excludes_scaffold_files`, `test_rescue_hook_failure_invariant`) **réimplémentent** la logique dans le test et ne peuvent donc pas falsifier le code réel. On l'extrait tel quel en fonction top-level, avec ses gardes en tête :

```bash
_rescue_dirty_worktree() {
    [ -n "$WORKTREE_DIR" ] || return 0
    [ "${PRE_RUN_HEAD:-}" = "${POST_RUN_HEAD:-}" ] || return 0
    case "$SKILL" in dev-pilot|dev-groom) ;; *) return 0 ;; esac
    DIRTY_FILES=$(git -C "$WORKTREE_DIR" status --porcelain 2>/dev/null | head -20)
    [ -n "$DIRTY_FILES" ] || return 0
    ...
}
```

`_post_flight_recovery` l'appelle à la place du bloc. Le corps est déplacé, pas réécrit : les ancres statiques dont dépendent les suites existantes (`mika-rescue-commit-err`, le décompte à deux de `RESCUED_DIRTY_WORKTREE=1` sur les chemins de succès, la présence de `--no-verify` sur les trois sites de commit) sont préservées littéralement.

La fonction lit/écrit des globals (`WORKTREE_DIR`, `SKILL`, `PRE_RUN_HEAD`, `POST_RUN_HEAD`, `REPO`, `ISSUE_NUM`, `BRANCH`, `SESSION_ID`, `PILOT_EXIT`, `RESULT`, `RESCUED_DIRTY_WORKTREE`) — même contrat qu'avant, la fonction ne change pas le couplage, elle le rend nommable.

### D2 — Différenciation par skill

| Aspect | `dev-pilot` | `dev-groom` |
|---|---|---|
| Sujet du commit | `impl staged by post-flight recovery (mika#1282)` (inchangé) | `plan staged by post-flight recovery (mika#2031)` |
| `RESCUED_DIRTY_WORKTREE=1` | posé | **non posé** — pas de draft PR (R4) |
| Préfixe de RESULT en succès | `PIPELINE FAILURE: ... auto-committed` (inchangé) | note de préservation, sans `PIPELINE FAILURE:` (R7) |
| Préfixe de RESULT en échec | `PIPELINE FAILURE:` (inchangé) | `PIPELINE FAILURE:` — identique |
| Publication | `_push_branch` | `_push_branch` (identique) |

Le `cargo fmt` proactif reste gaté sur la présence de `*.rs` en index : un sauvetage de plan markdown ne paie pas le démarrage de cargo.

### D3 — Nommer ce qui a été préservé (R6)

Après un commit de sauvetage réussi, on capture `RESCUE_SHA=$(git -C "$WORKTREE_DIR" rev-parse --short HEAD)` et on l'inclut, avec `$BRANCH`, dans la note. Forme cible pour `dev-groom` :

```
dispatch-lib (mika#2031): plan preserved before anything else ran.
Rescued into commit <sha> on branch <branch>:
docs/plans/....-plan.md
The pilot wrote this content and never committed it; dispatch-lib committed it
so the next dispatch's worktree removal cannot destroy it.
```

et pour `dev-pilot`, la ligne `Files rescued:` existante gagne `Rescued into commit <sha> on branch <branch>`.

### D4 — Ne plus affirmer « already committed » sans l'avoir mesuré (R8)

Dans la branche `elif [ "$SKILL" = "dev-groom" ] && [ -n "$VALID_PLAN" ]` de `_post_flight_recovery`, on mesure avant d'affirmer :

```bash
if git -C "$WORKTREE_DIR" ls-files --error-unmatch -- "$VALID_PLAN" >/dev/null 2>&1 \
   && [ -z "$(git -C "$WORKTREE_DIR" status --porcelain -- "$VALID_PLAN" 2>/dev/null)" ]; then
    # suivi ET propre → réellement commité
else
    # présent dans le worktree, non commité → le sauvetage ci-dessous s'en charge
fi
```

`VALID_PLAN` est un chemin relatif au worktree tel que `_find_issue_plan` le rend ; la mesure est faite avec `git -C "$WORKTREE_DIR"` pour que les deux parlent du même arbre.

### D5 — Ordre d'exécution (préserver d'abord)

Aucun réordonnancement n'est requis, et c'est un fait à vérifier plutôt qu'à supposer : `_post_flight_recovery` (donc le sauvetage) s'exécute déjà avant `_check_pilot_force_push`, `_iterate_groom_loop` et `_push_branch` dans `dispatch_claude_pilot`, et le force-remove de worktree n'a lieu qu'au dispatch **suivant**. La préservation précède structurellement le déblocage. Le plan n'introduit aucun appel avant le sauvetage.

---

## Alternatives Considered

- **Garder le bloc inline et n'élargir que la garde.** Diff minimal, mais le comportement reste intestable autrement que par grep statique — exactement la forme de test qui ne peut pas falsifier le code. Rejeté : le ticket demande un anti-vacuité réel.
- **Poser `RESCUED_DIRTY_WORKTREE=1` aussi pour `dev-groom`.** Ouvrirait une draft PR contenant un plan, ce que le contrat de grooming exclut (le plan va sur la branche, la PR vient de l'implémentation). Rejeté.
- **Déclarer que le contenu de grooming n'est délibérément pas sauvé** (l'option « ou expliquer pourquoi » du ticket). Indéfendable : le coût de la perte est un cycle complet, le filet existe déjà, et l'ouvrir coûte une garde.

---

## Implementation Steps

1. Extraire le bloc de sauvetage inline en `_rescue_dirty_worktree()` ; `_post_flight_recovery` appelle la fonction. Corps déplacé littéralement, ancres statiques préservées.
2. Ouvrir la garde de skill à `dev-groom` (`case ... dev-pilot|dev-groom`).
3. Différencier message de commit, `RESCUED_DIRTY_WORKTREE`, et texte de RESULT par skill (D2).
4. Capturer et publier le sha de sauvetage + la branche dans la note, pour les deux skills (D3).
5. Mesurer avant d'affirmer « already committed » (D4).
6. Écrire `skills/bundled/_shared/tests/test_dev_groom_dirty_rescue.sh` : suite qui **source** dispatch-lib.sh et appelle la vraie fonction contre un dépôt git temporaire.
7. Exécuter la suite nouvelle + `test-dispatch-lib.sh` + les suites `tests/` existantes ; `shellcheck` sur le fichier modifié.

---

## Verification Contract

La suite `test_dev_groom_dirty_rescue.sh` exerce la fonction réelle, pas une réimplémentation. Cas :

| # | État | Attendu |
|---|---|---|
| V1 | `dev-groom`, arbre **propre**, HEAD inchangé | aucun commit (HEAD identique), `RESULT` inchangé, pas de note de sauvetage |
| V2 | `dev-groom`, plan non commité dans `docs/plans/` | HEAD avance ; le plan est suivi à HEAD ; `RESULT` nomme le fichier, le sha et la branche |
| V3 | `dev-groom`, sauvetage réussi | `RESCUED_DIRTY_WORKTREE` **non** posé (pas de draft PR) et pas de `PIPELINE FAILURE:` dans la note ajoutée |
| V4 | `dev-groom`, seuls des chemins scaffold sales | aucun commit, HEAD inchangé (garde d'index vide) |
| V5 | `dev-pilot`, arbre sale | comportement inchangé : HEAD avance, `RESCUED_DIRTY_WORKTREE=1`, préfixe `PIPELINE FAILURE:` |
| V6 | `dev-pilot`, arbre propre | aucun commit, aucune note |
| V7 | skill inconnu, arbre sale | aucun commit (la garde de skill tient) |

V1/V6 sont l'anti-vacuité : sans eux, un sauvetage qui s'exécuterait tout le temps passerait la suite.

Commandes :

```bash
make test-dispatch-lib          # les deux suites, la cible que la CI invoque
bash skills/bundled/_shared/tests/test_dev_groom_dirty_rescue.sh
shellcheck -S error skills/bundled/_shared/dispatch-lib.sh
```

**Contrôle négatif (obligatoire avant de croire la suite).** Remettre la garde à
`dev-pilot)` seul et relancer : la suite doit passer de 29/29 à 21/29. Mesuré le
2026-08-30 — huit assertions tombent, toutes du côté dev-groom.

**Hors périmètre, constaté :** `tests/test_parse_disposition.sh` sort en code 1
sur `origin/main` comme sur cette branche (coupure silencieuse en fin de tier 1b,
`set -euo pipefail`). Préexistant, non touché ici.

---

## Definition of Done

- [ ] `_rescue_dirty_worktree()` existe, est appelée par `_post_flight_recovery`, et couvre `dev-pilot` et `dev-groom`.
- [ ] Un plan non commité par `dev-groom` est commité sur la branche et le callback le nomme (fichier + sha + branche).
- [ ] Un arbre propre ne déclenche aucun sauvetage, pour les deux skills.
- [ ] `dev-groom` n'ouvre pas de PR de sauvetage.
- [ ] Le chemin `dev-pilot` est inchangé hors ajout du sha+branche.
- [ ] `test_dev_groom_dirty_rescue.sh` passe et exerce la fonction réelle.
- [ ] `test-dispatch-lib.sh` et les suites `tests/` existantes passent sans qu'aucune assertion soit relâchée. Une extraction d'ancre a dû être ré-adressée (voir *Structural anchors*), pas affaiblie.
- [ ] La nouvelle suite est câblée dans `make test` **et** `make test-dispatch-lib` — la cible que la CI invoque.

---

## Acceptance criteria

- [ ] Le sauvetage d'arbre sale s'exécute pour `dev-groom` sur HEAD-inchangé + worktree sale, avec les mêmes exclusions de chemins scaffold que `dev-pilot`.
- [ ] Le contenu sauvé atterrit dans un commit sur la branche de dispatch avant tout autre traitement (préserver d'abord), et `_push_branch` le publie ensuite (débloquer ensuite).
- [ ] Le callback nomme explicitement ce qui a été préservé et où : chemins des fichiers, sha du commit de sauvetage, branche.
- [ ] Un worktree propre ne produit ni commit de sauvetage ni note — vérifié par un test dédié pour chacun des deux skills.
- [ ] Le sauvetage `dev-groom` n'ouvre aucune PR et ne pose pas `RESCUED_DIRTY_WORKTREE`.
- [ ] Un sauvetage `dev-groom` réussi n'est pas rapporté comme `PIPELINE FAILURE:` ; un sauvetage échoué l'est, avec l'arbre laissé sale pour inspection.
- [ ] La suite de test source `dispatch-lib.sh` et appelle la fonction réelle contre un dépôt git temporaire (pas une réimplémentation de la logique dans le test).
- [ ] `test-dispatch-lib.sh` et les suites existantes sous `skills/bundled/_shared/tests/` passent sans que leurs assertions aient été relâchées.
- [ ] La nouvelle suite tourne dans la cible CI (`make test-dispatch-lib`), pas seulement à la main.
