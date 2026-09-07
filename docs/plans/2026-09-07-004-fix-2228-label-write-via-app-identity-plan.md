---
issue: 2228
type: fix
title: "Écritures de label (operator-gated, ready, human-review-required) via le token App, pas le PAT sous-scopé"
branch: fix/2228/label-write-via-app-identity
---

# Plan — #2228 : pose de label via l'identité App

## Problème (évidence dure, 2026-09-07)

Les chemins de pose de label échouent : `gh issue edit --add-label failed: Resource not accessible by personal access token (addLabelsToLabelable)`. Le token résolu (via `resolve_github_token`, PAT-first — le PAT samidarko fuité dans l'env du spirit, cf #2218) **n'a pas le scope `issues: write`**. Mesuré :
- refusal-marker `operator-gated` (auto_pull.rs:1841) — 29× aujourd'hui.
- promotion `ready` de l'auto-feeder (auto_pull.rs:2417/2560/2866) — 5× aujourd'hui, dont une tentative sur #2218.
- `operator-review` (auto_pull.rs:2028).

Conséquence : le pool `ready` ne s'auto-alimente plus, les refus d'exclusion sont muets.

## Fix (direction opérateur : voie App, pas de nouveau PAT)

Poser un label n'est **pas** une opération dont GitHub lit l'auteur (contrairement à approve/merge de PR) — le repli App y est explicitement acceptable (#2205). **Résoudre le token des écritures de label en App-first** (le token d'installation App porte `issues: write`), fallback PAT quand l'App n'est pas configurée.

Mécanisme : `Settings::resolve_label_write_token(github_app)` — **App-first** (`github_app.installation_token()`), fallback `self.github_token` (PAT). Symétrique inverse de `resolve_github_token` (PAT-first).

### Portée — callsites couverts (F1, explicite)

**In scope (écritures de label, classe non-identitaire) :**
- `auto_pull.rs:1563` `gh_apply_label` (`--add-label`) — 6 appelants : refusal-marker (:1841), operator-review (:2028), ready-promotions (:2417, :2560, :2866).
- `auto_pull.rs:1597` `gh_remove_label` (`--remove-label`) — même classe (:1626), utilisé par la ré-entrée operator-review.
- `wip_rescue.rs:1035/1069` — pose de `human-review-required` via `gh(&add_label, token)`. **Même classe, même défaut** (le token wip_rescue vient aussi de resolve_github_token) → inclus.

**Hors scope, justifié :**
- `gh_comment_issue` (commentaires d'issue) : l'évidence mesurée est **label-write seul** (`addLabelsToLabelable`) ; `createComment` relève d'un scope GitHub distinct et n'a PAS été observé en échec. L'inclure serait élargir sans évidence. Si un `createComment` échoue par scope plus tard, ticket séparé (même helper réutilisable).
- Chemins identitaires (approve/merge de PR) : gardent `resolve_github_token` PAT-first (#2218). Non touchés.

### Token App court-lived (F2, résolu)

`GitHubApp::installation_token()` (`mika-common/src/github_app.rs`) a un **cache `RwLock` avec refresh automatique** (double-checked locking, buffer 5 min avant expiry, persisté `~/.mika/github_app_token.json`). Chaque appel rend un token frais proactivement rafraîchi → **aucun mécanisme additionnel requis** ; pas de piège de token expiré pour des appels ponctuels ni pour un cycle auto_pull long.

### App présente mais sans permission Issues:write (F3)

Le résolveur ne peut pas pré-vérifier le scope d'un token sans l'utiliser. Comportement retenu : **précondition opérateur** (l'installation App DOIT avoir `Issues: Read and write`) + **échec LOUD et distinct**. Si le token App échoue sur `addLabelsToLabelable`, émettre un event nommé distinct (`label_write_app_token_insufficient`) — NE PAS retomber silencieusement sur le PAT (qui échouerait pareil) ni confondre avec « App absente ». Trois états distingués dans les logs : App absente (→ PAT fallback), App présente token OK, App présente permission insuffisante (→ erreur nommée). La checklist opérateur vérifie la permission avant deploy.

## Fire-Disposition (F4)

AC3 s'appuie sur des **détecteurs déjà existants** (les events ERROR `auto_pull_refusal_marker_unavailable` et `auto_feeder: failed to apply ready label`) — ce fix n'en construit pas de nouveau, il les fait **passer de rouge à silencieux**.
- **Disposition** : rouge sur main **maintenant** (les events firent, mesuré 29× + 5× aujourd'hui) → **silencieux après fix** (App configurée avec Issues:write).
- **Garde permanente** : ces events ERROR restent en place — toute réapparition future re-signale la régression.
- **Mécanisme de vérification** : grep `$MIKA_SPIRIT_LOG_FILE` sur un cycle complet post-deploy (PAS un gate CI — le résultat dépend de la config App live, non reproductible en sandbox). Le nouvel event `label_write_app_token_insufficient` (F3) est le détecteur du cas permission-insuffisante.

## Acceptance Criteria

- **AC1** — Écritures de label (auto_pull 6 + remove-label + wip_rescue) via le token App quand l'App est configurée. Test unitaire du résolveur `resolve_label_write_token` : App présent → token App ; App absent → PAT.
- **AC2 (fallback)** — App absente → pose de label via PAT (comportement actuel préservé). Test résolveur.
- **AC3 (Fire-Disposition ci-dessus)** — Zéro `auto_pull_refusal_marker_unavailable` / `auto_feeder: failed to apply ready label` en ERROR sur un cycle post-deploy (App avec Issues:write). Vérif grep.
- **AC4 (non-régression identité)** — Les chemins approve/merge de PR gardent `resolve_github_token` PAT-first, non modifiés.
- **AC5 (permission insuffisante — F3)** — App présente sans Issues:write → event nommé distinct `label_write_app_token_insufficient`, pas de fallback PAT silencieux ni de confusion avec App-absente. Test unitaire du chemin d'erreur.
- **AC6** — `cargo test -p mika-agent` + `cargo build` verts.

## Vérification opérateur pré-déploiement

- L'installation App a la permission `Issues: Read and write` (sinon le token App échoue aussi — AC5 le rend visible).

## Out of scope

- Défaut d'identité self-approve (env-shadow) = #2218.
- `gh_comment_issue` (pas d'évidence d'échec ; scope distinct).
- Minting/rescoping de PAT ; réparer le scope du PAT samidarko (contourné par l'App).

## Risques

- **App sans `issues: write`** : couvert par AC5 (échec loud nommé) + checklist opérateur.
- **Portée** : les 3 chemins de label (auto_pull, auto-feeder, wip_rescue) sont couverts ; comments explicitement exclus faute d'évidence — bordé.
