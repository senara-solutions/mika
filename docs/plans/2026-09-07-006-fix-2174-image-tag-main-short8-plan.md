---
issue: 2174
type: fix
title: "Le workflow d'image pousse main-<short8> (la forme que la rotation consomme), pas le sha nu"
branch: fix/2174/image-tag-main-short8
---

# Plan — #2174 : aligner le tag produit sur la forme consommée en aval

## Problème (évidence, 2026-09-04)

`.github/workflows/agent-image-build-push.yml` pousse `${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:${{ github.sha }}` — le **sha nu, 40 caractères**. Mais la forme réellement consommée en aval (Helm values, historique ECR sur mika-cloud) est `main-<short>` : `main-7cea6d6`, `main-960ab824`, `main-56336b9e`, `main-d7314906`. **Contrôle négatif mesuré :** aucun consommateur ne lit la forme sha-nu (`grep github.sha|GITHUB_SHA|[0-9a-f]{40}` sur mika-cloud helm/+scripts/ = zéro ligne). `rotate-image.sh` ne dérive aucun tag (reçoit `NEW_TAG` en argument opérateur). Donc le workflow ne fait pas échouer la CI, mais son produit est **inutilisable en aval sans traduction manuelle**.

C'est la **disposition pré-spécifiée** de la § Fire-Disposition de #2143 (prémisse P2 confirmée : la ligne pousse le sha nu, aucun consommateur ne le lit).

## Décision de scope : voie A (aligner le producteur)

Le ticket recommande la voie A (le workflow pousse `main-<short8>`) plutôt que la voie B (réécrire tous les fichiers de valeurs déployés + historique ECR hétérogène). **Retenu : voie A** — moins coûteux, ne touche pas à l'état déployé, et le tag reste fonction du sha (satisfait l'immutabilité).

## Fix

Le workflow calcule le short-sha et pousse `main-<short8>`. Pousser **les deux formes** (`${{ github.sha }}` nu ET `main-<short8>`) — toutes deux dérivées du sha, donc compatibles avec l'immutabilité ; garde la forme nue au cas où un futur consommateur la voudrait, ajoute la forme consommée aujourd'hui. GitHub Actions ne fait pas de substring dans `tags:` directement → une étape amont calcule `SHORT_SHA` (ex. `echo "SHORT_SHA=${GITHUB_SHA:0:8}" >> "$GITHUB_ENV"`), puis le bloc `tags:` ajoute `${{ env.ECR_REGISTRY }}/${{ env.ECR_REPOSITORY }}:main-${{ env.SHORT_SHA }}`.

Question de longueur (à confirmer au grooming/impl) : les exemples réels mélangent 7 (`main-7cea6d6`) et 8 (`main-960ab824`) caractères. Le ticket demande `${GITHUB_SHA:0:8}` (8). Retenir **8** (l'ask du ticket) ; le 7-char historique reste consommable (la rotation prend un argument littéral, pas une longueur fixe).

## Contrainte (garde #2143)

Les trois dépôts ECR sont `IMMUTABLE` ; la garde `scripts/check-image-tags-immutable.sh` (job CI `Image Tag Immutability Lint`) échoue sur tout tag non dérivé du sha. `main-<short8>` est dérivé du sha → passe. `latest`/`stable`/`main` échoueraient → interdits. Le fix doit vérifier que `main-<short8>` satisfait cette garde (l'exécuter sur le tag produit).

## Acceptance Criteria

- **AC1** — Le workflow pousse `main-<short8>` (8 premiers caractères du sha), en plus (ou à la place) du sha nu. Vérifiable : le `tags:` du build-push contient la forme `main-${SHORT_SHA}`.
- **AC2 (immutabilité)** — `scripts/check-image-tags-immutable.sh` passe sur le(s) tag(s) produit(s) (`main-<short8>` est sha-dérivé). Vérifiable en exécutant la garde.
- **AC3** — La forme produite correspond à la forme consommée par la rotation (`main-<short>` dans les Helm values). Doc/commentaire dans le workflow le pointe.
- **AC4** — Le workflow reste valide (yaml lint / actionlint si présent).

## Séquencement (du ticket)

Devient bloquant à la réactivation du workflow, avec #2143. Ordre : mika-cloud#220 (OIDC) → mika#2143 (mergé) → **ce ticket** → réactivation → mika#1619. Ce correctif est safe à merger avant réactivation (le workflow n'est pas actif).

## Hors périmètre

- Voie B (réécrire les consommateurs) — écartée.
- La contradiction tag-mouvant/dépôt-immuable = #2143 (mergé).
- La réactivation du workflow elle-même (séquencement aval).

## Risques

- **Longueur 7 vs 8** : les deux coexistent dans l'historique ; 8 est l'ask, le 7 reste consommable (argument littéral). Pas de rupture.
- **Immutabilité** : si `main-<short8>` échouait la garde (improbable, il est sha-dérivé), AC2 l'attrape.
