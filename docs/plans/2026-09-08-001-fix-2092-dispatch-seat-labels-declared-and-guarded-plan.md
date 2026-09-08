---
issue: 2092
type: fix
title: "dispatch:loop déclarée dans labels.yml + contrôle KNOWN_DISPATCH_SEATS ↔ labels.yml (un siège non déclaré ne doit plus désarmer en silence)"
branch: fix/2092/dispatch-seat-labels-declared-and-guarded
---

# Plan — #2092 : les étiquettes de siège dispatch:* déclarées ET gardées

## Problème (mesuré, corrigé au 2026-09-08)

`KNOWN_DISPATCH_SEATS = &["loop", "ssc", "mpc"]` (`crates/mika-agent/src/webhook_dispatch.rs:251`) est la source de vérité des sièges. Mais `.github/labels.yml` ne déclare que **`dispatch:ssc`** (L123) et **`dispatch:mpc`** (L127). **`dispatch:loop` n'est PAS déclarée** → `label-sync` (workflow `labels.yml`, `delete-other-labels: true`) ne la crée pas / la supprimerait, donc la branche `SeatVerdict::OwnedByCurrentSeat` (webhook_dispatch.rs:362) pour le siège `loop` est **inatteignable** : un label qui n'existe pas ne peut être posé.

**AC3 (ouvert) :** aucun contrôle ne compare `KNOWN_DISPATCH_SEATS` (code) aux `dispatch:*` de `.github/labels.yml` (`grep -rn KNOWN_DISPATCH_SEATS .github/ scripts/` → rien). Donc un siège ajouté au code sans son label glisse en silence — c'est la classe « un label d'enforcement non déclaré échoue en silence » (voir `docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md`, 3e occurrence après dispatch:ssc et operator-review/blocked).

## Fix

### D1 — Déclarer `dispatch:loop` dans `.github/labels.yml` (AC1/AC2)
Ajouter l'entrée `dispatch:loop` (couleur/description cohérentes avec ssc/mpc). Les trois sièges de `KNOWN_DISPATCH_SEATS` ont alors un label déclaré → la branche `OwnedByCurrentSeat` du siège `loop` devient atteignable. (Vérifier qu'aucun autre siège réel n'existe ; `dispatch:zorglub` vu dans le code est une fixture de test, PAS un siège — à confirmer, ne pas le déclarer.)

### D2 — Contrôle KNOWN_DISPATCH_SEATS ↔ labels.yml (AC3, détecteur)
Ajouter un script `scripts/check-dispatch-seats-declared.sh` (sur le modèle des `check-*` existants) qui : extrait `KNOWN_DISPATCH_SEATS` de `webhook_dispatch.rs`, extrait les `dispatch:*` de `.github/labels.yml`, et **échoue (exit non-zéro) si un siège du code n'a pas de label déclaré** (nommant le siège manquant). Le câbler comme job CI (dans `ci.yml`, à côté des autres lints structurels). Ainsi un futur siège ajouté au code sans label casse la CI au lieu de désarmer en silence.

## Fire-Disposition (détecteur AC3)

Le défaut est **silencieux** (un siège non déclaré ne pose aucun label → la branche est morte sans erreur). Le détecteur D2 est un **gate CI bloquant** :
- **Rouge maintenant** (dispatch:loop non déclarée → le contrôle échoue tant que D1 n'est pas fait) → **vert après D1+D2**.
- **Garde permanente** : tout siège ajouté à `KNOWN_DISPATCH_SEATS` sans son label dans labels.yml re-casse la CI, nommant le siège. C'est précisément la garde anti-reconduction que la classe exige.

## Acceptance Criteria (tie-back ticket, inchangés)

- **AC1** — `dispatch:loop` est déclarée dans `.github/labels.yml` ; les trois `KNOWN_DISPATCH_SEATS` ont un label déclaré.
- **AC2** — La branche `OwnedByCurrentSeat` du siège `loop` est atteignable (le label peut être posé). Vérifiable : `gh label list | grep dispatch:loop` après label-sync.
- **AC3 (détecteur)** — Un contrôle CI compare `KNOWN_DISPATCH_SEATS` à `.github/labels.yml` et échoue si un siège du code n'a pas de label. Test : retirer temporairement un label → CI rouge nommant le siège.
- **AC4** — `scripts/check-dispatch-seats-declared.sh` a son test unitaire (positif : tous déclarés → vert ; négatif : un manquant → rouge nommant le siège).

## Surface

- `.github/labels.yml` — ajouter `dispatch:loop`.
- `scripts/check-dispatch-seats-declared.sh` (neuf) + son test.
- `.github/workflows/ci.yml` — câbler le job.
- Source de vérité lue : `crates/mika-agent/src/webhook_dispatch.rs:251` (`KNOWN_DISPATCH_SEATS`).

## Hors périmètre

- `dispatch:ssc`/`dispatch:mpc` déjà déclarées (438d28a4/#2078).
- `dispatch:zorglub` (fixture de test, pas un siège).
- La logique de dispatch-par-siège elle-même (correcte ; c'est la déclaration + la garde qui manquent).

## Risques

- **Parser fragile** : l'extraction de `KNOWN_DISPATCH_SEATS` depuis le `.rs` doit être robuste (le tableau peut être reformaté). Ancrer sur le nom de la const + parser le littéral tableau ; AC4 négatif le teste.
- **label-sync delete-other-labels** : `dispatch:loop` doit être dans labels.yml AVANT que le contrôle passe, sinon la garde échoue sur son propre dépôt (c'est voulu : rouge jusqu'à D1).
