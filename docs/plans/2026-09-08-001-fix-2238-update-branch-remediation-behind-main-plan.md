---
title: "fix: remédiation update-branch sur une PR BEHIND — fermer le 3e verrou d'autonomie"
issue: 2238
type: fix
depth: Standard
origin: null
created: 2026-09-08
---

# fix: Remédiation update-branch sur une PR BEHIND

## Résumé

Les trois sites de merge autonome **détectent** qu'une PR est derrière `main` (`is_behind_main`, mika#1577) et **déclinent** le merge — mais rien ne **répare** l'état détecté. La seule remédiation existante est une phrase de prose adressée à un LLM (« Rebase the PR onto main before merging »), sans outil nommé, et aucun prompt embarqué n'énumère la variante `behind_main`. Résultat mesuré : une PR APPROVED + CI verte + mergeable reste bloquée jusqu'à une intervention humaine.

Ce plan ajoute l'étape manquante : un helper partagé qui déclenche `gh pr update-branch`, appelé depuis les trois sites, avec **rendez-vous différé sur le webhook CI** plutôt qu'un re-merge immédiat, un plafond anti-thrash, et une trace d'audit.

---

## Cadre du problème

### Ce que le code fait aujourd'hui (mesuré, 2026-09-08)

`is_behind_main(base_ref_oid, repo, token)` — `crates/mika-agent/src/tools/pr_merge_with_gate.rs:710` — compare le `baseRefOid` de la PR au HEAD de `main` via l'API. Elle est appelée depuis exactement trois sites :

| Site | Ligne | Comportement sur BEHIND |
|------|-------|-------------------------|
| `tools/pr_merge_with_gate.rs` (étape 1b) | 155 | `Blocked { reason: BehindMain { pr_base_sha, current_main_sha } }` |
| `server/ci_success_handler.rs` (étape 5b) | 356 | `Passthrough { enrichment: format_behind_main_enrichment(...) }` |
| `server/verdict_handler.rs` | 509 | `Passthrough { enrichment: format_behind_main_enrichment(...) }` |

Les deux `format_behind_main_enrichment` (`ci_success_handler.rs:857`, `verdict_handler.rs:2801`) produisent la même consigne terminale : *« Rebase the PR onto main before merging. »*

### Les trois trous

**T1 — Aucune remédiation structurelle.** `grep -rn "update-branch\|update_branch" crates/` rend **zéro** résultat. Le mot « rebase » n'existe que dans une chaîne de prose. La réparation d'un état purement mécanique est déléguée à un LLM par prompt — la classe empiriquement établie comme insuffisante au niveau substrat (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`).

**T2 — Le LLM n'a aucune consigne pour `behind_main`.** `grep -rn "behind_main" skills/bundled/` rend **zéro** résultat. Les prompts (`self-dev`, `self-dev-webhook-qa`, `self-dev-webhook-ci`) énumèrent cinq `blocked.reason` — `merge_conflict`, `required_check_failed`, `missing_approval`, `pr_closed`, `draft` — et instruisent « branch on these variants **exhaustively** ». L'outil peut en retourner **sept** : les cinq documentées, plus `behind_main` (mika#1577) et `human_gate_required` (mika#1829). Un agent qui reçoit une variante hors de sa liste exhaustive n'a pas d'arme définie ; le comportement observé est l'arrêt silencieux.

**T3 — Le BEHIND est systématique, pas occasionnel.** `is_behind_main` compare des SHAs (`base_ref_oid != current_main_sha`), délibérément — KTD-1 de mika#1577 a rejeté `mergeStateStatus` parce que le ruleset a `strict_required_status_checks_policy: false` et que `mika-platform-dev` a `bypass_mode: always`, donc GitHub rend `CLEAN` même derrière. La conséquence, non tirée en #1577 : **tout merge sur `main` rend BEHIND toutes les PR ouvertes**, immédiatement. Le blocage n'est pas un cas de bord, c'est l'état par défaut dès qu'une seconde PR existe.

### Ce que dit l'évidence

PR mika#2236 : `reviewDecision=APPROVED` par `mika-platform-qa` à 08:25:08Z, `mergeable=MERGEABLE`. `mergedBy = samidarko` (humain) à `2026-09-08T10:00:36Z`. La boucle n'a pas fermé ; l'opérateur a fermé.

---

## Exigences

- **R1** — Un helper partagé `attempt_update_branch(pr_number, repo, token)` vit à côté de `is_behind_main` dans `pr_merge_with_gate.rs` et est le **seul** point d'appel de `gh pr update-branch`.
- **R2** — Les trois sites BEHIND appellent R1 au lieu de décliner sèchement.
- **R3** — Après un update-branch réussi, **aucun merge n'est tenté dans le même tour**. Le tour se termine sur un état « branche mise à jour, CI fraîche attendue ».
- **R4** — Plafond anti-thrash : au plus une tentative d'update-branch par couple (PR, SHA de `main` visé). Une seconde arrivée sur le même SHA cible ne ré-émet pas.
- **R5** — Nouvelle variante `MergeGateResult::BranchUpdated { pr_base_sha, new_main_sha }` distincte de `Blocked` — un BEHIND réparé n'est pas un blocage.
- **R6** — Les prompts embarqués énumèrent les **sept** variantes `blocked.reason`, avec une disposition nommée pour `behind_main` et `human_gate_required`.
- **R7** — Trace : chaque tentative d'update-branch émet un événement structuré (PR, SHA avant, SHA main visé, issue) lisible par le moniteur.
- **R8** — Échec d'update-branch (conflit réel révélé au rebase, permission, 422) dégrade en `Blocked { reason: MergeConflict | ... }` avec le détail — jamais en merge silencieux.

---

## Décisions techniques clés

### KTD-1 — Update-branch, puis **rendez-vous sur le webhook CI**. Jamais de re-merge dans le même tour.

Le ticket propose « déclencher un update-branch puis **re-tenter le merge** ». Ce plan s'en écarte délibérément, et c'est la décision structurante.

`gh pr update-branch` crée un **nouveau commit** sur la branche de la PR. Le `statusCheckRollup` vert que le gate vient de lire porte sur le commit **précédent**. Merger juste après l'update-branch, c'est merger un commit qu'aucune CI n'a validé — c'est-à-dire **exactement le défaut que mika#1577 a été écrit pour fermer** (« two PRs green against the same base can merge in sequence, producing a `main` that neither CI run validated »). Un re-merge immédiat rouvrirait #1577 par la porte que #2238 ouvre.

La séquence correcte est un rendez-vous, pas une boucle :

```
BEHIND détecté
  → attempt_update_branch()          [nouveau commit sur la tête de PR]
  → BranchUpdated / Passthrough      [le tour se termine ICI]
  → GitHub relance la CI sur le nouveau commit
  → webhook check_suite success
  → ci_success_handler ré-entre       [désormais à jour ET CI fraîche verte]
  → merge
```

Le chemin de fermeture existe déjà : `ci_success_handler` est précisément le handler du `check_suite success`. Cette conception n'ajoute aucun mécanisme d'attente — elle branche la réparation sur le rail de réveil déjà en place.

**Conséquence assumée :** la fermeture prend un cycle CI de plus. C'est le prix de l'invariant #1577, et il est payé par la machine, pas par l'opérateur.

### KTD-2 — Le plafond anti-thrash s'ancre sur le SHA cible, pas sur un compteur

Sous concurrence, A et B ouvertes : A merge → B devient BEHIND → B s'update → CI de B → pendant ce temps C merge → B redevient BEHIND → … Un compteur de tentatives est fragile (où vit l'état ? qui le remet à zéro ?). L'ancrage retenu est **le SHA de `main` visé** : on n'émet un update-branch que si l'on n'a pas déjà mis cette PR à jour vers **ce** SHA précis. La clé est `(pr_number, current_main_sha)`, naturellement idempotente et sans remise à zéro.

Le porteur d'état est à trancher en U2 ; le plan retient par défaut la métadonnée de tâche corrélée (déjà le porteur des HOLD dans `self-dev` M4), avec repli sur un cache mémoire borné du processus serveur si aucune tâche n'est corrélée (cas Dependabot, mika#1729).

**Le plafond ne fait pas de la progression une garantie.** Sous un flux de merges soutenu, une PR peut rester derrière indéfiniment sans jamais boucler : chaque nouveau SHA rouvre une tentative légitime. C'est voulu — la famine est visible (R7) plutôt que masquée par un abandon. La borne d'inanition, si elle s'avère nécessaire, est un ticket suivant, pas ce plan.

### KTD-3 — Le SHA lu pour le plafond est celui déjà en main

`is_behind_main` retourne `BehindMainInfo { pr_base_sha, current_main_sha }`. Le `current_main_sha` est lu **avant** l'update-branch ; c'est la clé du plafond et le SHA rapporté. Aucun second appel API n'est ajouté. Il y a une fenêtre TOCTOU (un merge peut atterrir entre la lecture et l'update) ; elle est inoffensive : la PR ressortira BEHIND vers un SHA différent, donc une nouvelle clé, donc une nouvelle tentative légitime.

### KTD-4 — `--auto` de GitHub n'est pas une alternative

Le ticket demande si l'auto-merge GitHub pourrait « update-and-merge » un BEHIND. Non, pour une raison déjà établie en #1577 KTD-1 : `strict_required_status_checks_policy: false` + `bypass_mode: always` sur `mika-platform-dev` font que GitHub **ne considère pas la PR comme derrière**. L'auto-merge n'a donc rien à réparer de son point de vue, et le seul gardien qui lie est celui en code. La remédiation doit être en code pour la même raison que la détection l'est.

### KTD-5 — Échec d'update-branch : dégrader, jamais forcer

`gh pr update-branch` échoue lorsque le rebase révèle un conflit réel, quand le jeton n'a pas le droit d'écriture sur la tête, ou en 422 « already up to date » (course bénigne). Ces trois cas se distinguent au message et se traduisent respectivement en `Blocked { MergeConflict }`, `GateError { CredentialScope }`, et une ré-évaluation immédiate de `is_behind_main`. Aucun chemin ne mène à un merge : un update-branch raté laisse le gate au moins aussi fermé qu'avant.

### KTD-6 — La correction de prompt (R6) est dans le périmètre, pas adjacente

`feedback_implementation_scope_bundling` renvoie le travail adjacent vers un ticket séparé. R6 n'est pas adjacent : les trois sites structurels sont **fail-open** sur erreur d'API (`warn!` + on continue). Quand le chemin structurel dégrade, le LLM est le seul recours — et aujourd'hui il fait face à une variante que sa liste « exhaustive » ne contient pas. Corriger le code sans corriger l'énumération laisserait le repli muet. Les deux couches ferment le même trou.

---

## Unités d'implémentation

### U1 — `attempt_update_branch` + variante `BranchUpdated`

**Exigences :** R1, R5, R8
**Fichiers :** `crates/mika-agent/src/tools/pr_merge_with_gate.rs`

Ajouter, à côté de `is_behind_main` (bloc « Behind-main detection (#1577) », l. 680-724) :

```rust
pub(crate) enum UpdateBranchOutcome {
    Updated,            // nouveau commit créé sur la tête de PR
    AlreadyUpToDate,    // 422 bénin — re-évaluer is_behind_main
    Conflict(String),   // le rebase révèle un conflit réel
    Failed(String),     // permission / API / gh absent
}

pub(crate) async fn attempt_update_branch(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> UpdateBranchOutcome
```

L'implémentation passe par `run_gh_subprocess(&["pr", "update-branch", &pr_str, "--repo", repo], token)` — le même helper que `run_gh_merge` (l. 726-751), donc le même scrub d'environnement et la même ré-injection de jeton (mika#515). La discrimination des sorties se fait sur le texte d'erreur de `gh`.

Ajouter `MergeGateResult::BranchUpdated { pr_base_sha, new_main_sha }` avec `#[serde(rename = "branch_updated")]`, sœur de `Merged` / `AutoMergeEnabled` / `Blocked` (l. 384-401).

**Motif à suivre :** la forme exacte de `is_behind_main` — helper autonome, `async`, retour typé, aucun effet sur les fonctions pures existantes (KTD-3 de #1577).

### U2 — Le plafond par SHA cible

**Exigences :** R4
**Fichiers :** `crates/mika-agent/src/tools/pr_merge_with_gate.rs`, porteur d'état à confirmer

Fonction de garde `should_attempt_update(pr_number, current_main_sha) -> bool`, consultée avant U1 et marquée après. **Le choix du porteur est le point ouvert de ce plan** : métadonnée de tâche corrélée (durable, absente pour Dependabot) contre cache mémoire borné du processus (universel, perdu au redémarrage). Recommandation : cache mémoire borné (`LruCache` de ~256 entrées, clé `(pr, sha)`) — un redémarrage qui autorise une tentative de plus est inoffensif, alors qu'une PR sans tâche non couverte ne l'est pas. À arbitrer par l'architecte.

Sur `false`, le site retombe sur le comportement d'aujourd'hui (bloquer / passthrough), avec le détail « update déjà tenté vers ce SHA ».

### U3 — Câbler les trois sites

**Exigences :** R2, R3
**Fichiers :** `pr_merge_with_gate.rs` (l. 152-179), `ci_success_handler.rs` (l. 351-388), `verdict_handler.rs` (l. 504-540)

Dans chacun, la branche `Ok(Some(info))` devient : consulter U2 → appeler U1 → disposer.

| Sortie U1 | `pr_merge_with_gate` | les deux handlers |
|-----------|----------------------|-------------------|
| `Updated` | `BranchUpdated { .. }` | `Passthrough` + enrichment « branche mise à jour, CI fraîche attendue » |
| `AlreadyUpToDate` | re-évaluer `is_behind_main` ; si à jour, poursuivre le gate | idem, puis poursuivre |
| `Conflict(d)` | `Blocked { MergeConflict, detail: d }` | `Passthrough` + enrichment conflit |
| `Failed(d)` | `Blocked { BehindMain { .. }, detail: d }` | `Passthrough` + enrichment d'échec (comportement d'aujourd'hui) |

Les deux `format_behind_main_enrichment` (`ci_success_handler.rs:857`, `verdict_handler.rs:2801`) sont réécrites : la consigne « Rebase the PR onto main before merging » — qui demandait au LLM une action que le code vient d'exécuter — devient un constat d'état (« la branche a été mise à jour vers `<sha>` ; ne pas merger, la CI fraîche déclenchera le rendez-vous ») **et une interdiction explicite de merger dans ce tour**, R3 étant le point où un LLM zélé peut rouvrir #1577.

> **Garde de rédaction :** les deux fichiers portent un test qui vérifie qu'aucun pre-digest ne déclenche la regex de revendication d'achèvement `(?i)\b(merged|deployed|completed?|shipped)\b` (`ci_success_handler.rs:869`, `verdict_handler.rs:2818`). Les nouveaux textes d'enrichment doivent passer cette garde — écrire « mise à jour » / « updated », jamais « merged ».

### U4 — Énumération des sept variantes dans les prompts

**Exigences :** R6
**Fichiers :** `skills/bundled/self-dev/system_prompt.md` (Rule 6 l. 272-276, dispositions l. 514-526), `skills/bundled/self-dev-webhook-qa/system_prompt.md` (Rule 6 l. 246-250), `skills/bundled/self-dev-webhook-ci/system_prompt.md` (Rule 6 l. 33-37)

Étendre la liste des `blocked.reason` de cinq à sept, et ajouter les dispositions :

- `behind_main` — le code a déjà tenté la réparation. **Ne pas merger. Ne pas rebaser à la main. Ne pas relancer `pr_merge_with_gate`.** Terminer le tour ; le webhook `check_suite success` reprend la main.
- `human_gate_required` — périmètre forge-gate (mika#1829) ; l'opérateur merge. Notifier, ne pas contourner.

Ajouter `branch_updated` (U1) à l'énumération des variantes de premier niveau, au même titre que `merged` / `auto_merge_enabled` / `blocked` / `already_merged` / `gate_errored`.

> **Rappel de déploiement (`feedback_mika_spirit_reextracts_bundled_skills_per_dispatch`) :** les skills embarqués sont ré-extraits à chaque dispatch. Éditer la source **et** reconstruire ; une édition dans `~/.mika/agents/*/skills/` est effacée au dispatch suivant.

### U5 — Trace et sonde

**Exigences :** R7
**Fichiers :** les trois sites de U3

Chaque tentative émet un `info!` structuré — `pr_number`, `pr_base_sha`, `target_main_sha`, `outcome`, `issue = 2238` — sur le modèle des `info!` behind-main déjà en place (`ci_success_handler.rs:359-364`). La sonde « BEHIND depuis > N min sans update » du ticket devient dérivable de cette trace au lieu d'exiger un compteur séparé : une PR qui apparaît en BEHIND sans `outcome=Updated` consécutif est le signal.

> **Rappel de mesure (`feedback_server_log_ecrit_chaque_ligne_deux_fois`) :** `server.log` écrit chaque ligne deux fois. Tout `grep -c` sur cette trace est doublé ; dédupliquer hors horodatage avant de compter.

---

## Tests

| Test | Nature | Ce qu'il pinne |
|------|--------|----------------|
| `attempt_update_branch` — les quatre sorties discriminées depuis des textes d'erreur `gh` réels | unitaire | U1, R8 |
| `BranchUpdated` sérialise en `{"action":"branch_updated", ...}` | unitaire | R5 — même forme que les tests `serialize_blocked_*` existants (l. 1206-1280) |
| BEHIND + update réussi → **aucun** `run_gh_merge` dans le même tour | unitaire, les 3 sites | **R3 — le test le plus important du plan** ; c'est le rouge qui empêche la réouverture de #1577 |
| Deuxième passage sur le même `(pr, main_sha)` → aucun second update | unitaire | R4 |
| `Conflict` → `Blocked{MergeConflict}`, jamais de merge | unitaire | R8 |
| Les nouveaux enrichments ne déclenchent pas la regex d'achèvement | unitaire | garde U3 |

> **Contrôle rouge-avant (`feedback_red_before_control_is_term_by_term`) :** le test R3 doit être vu échouer **avant** U3 — sur le code d'aujourd'hui il ne peut pas échouer, puisque aucun update-branch n'a lieu. Le rouge se construit en neutralisant la garde de non-merge une fois U1+U3 en place, terme par terme, pas en supposant que le vert final prouve quelque chose.

Commande : `cargo test -p mika-agent pr_merge_with_gate ci_success_handler verdict_handler`.

---

## Hors périmètre

- Une borne d'inanition (« après K SHAs successifs, abandonner et notifier ») — KTD-2 explique pourquoi le plafond ne la fournit pas ; à ficher séparément si la famine s'observe.
- Le passage de `is_behind_main` d'une comparaison de SHAs à `mergeStateStatus` — rejeté en #1577 KTD-1 pour une raison qui tient toujours.
- Toute modification du ruleset GitHub ou de `bypass_mode` — surface opérateur.
- La configuration de l'auto-merge GitHub (KTD-4 : elle ne peut pas résoudre ce cas).

---

## Références

- mika#1577 — l'assertion behind-main ; `docs/plans/2026-06-26-004-fix-1577-not-behind-main-assertion-plan.md`. KTD-1 y explique pourquoi la comparaison est faite sur des SHAs ; KTD-3 y donne la forme du helper autonome que U1 reprend.
- mika#1829 / mika#1853 — le périmètre forge-gate, source de la septième variante `human_gate_required`.
- mika#1729 — les PR Dependabot sans tâche corrélée ; le cas qui force U2 à ne pas dépendre d'une tâche.
- mika#515 — la ré-injection de `GH_TOKEN` après scrub, que `run_gh_subprocess` porte déjà.
- mika#2218 (verrou 1, identité de review — fermé), mika#2237 (verrou 2, mémoire > skill — ouvert).
- Évidence : PR mika#2236, APPROVED par `mika-platform-qa` à 08:25:08Z, `mergedBy = samidarko` à 10:00:36Z.
