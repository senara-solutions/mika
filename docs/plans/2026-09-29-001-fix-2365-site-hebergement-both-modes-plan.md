---
ticket: mika#2365
kind: fix
scope: site/ (getmika.ai) — copy only
status: ready-for-qa
---

# Site getmika.ai : présenter les deux modes d'hébergement — Plan (mika#2365)

Plan a posteriori (le fix est passé hors pipeline /mika ; on garde la trace, pas d'exemption). Le contenu n'est pas contesté par la QA ; seul le contrôle pipeline (plan absent) échouait.

## Goal Capsule

Le site public getmika.ai présentait Mika comme **auto-hébergé uniquement** et affirmait un absolu faux pour Mika Cloud (« Your data never leaves your machine »). But : présenter les **deux modes** (self-hosted + Mika Cloud), formulation ratifiée Vincent (11:25) validée Prime. Périmètre `site/` seulement — aucun crate Rust, aucun cargo.

## Formulation ratifiée

« Mika runs where you want: on your machine (self-hosted, open source) or on Mika Cloud, operated by Senara on AWS. » (adaptée à chaque surface, anglais).

## Édits (BEFORE → AFTER verbatim)

- **`site/index.html:7`** (meta) : `…runs entirely on your machine. Built in Rust.` → `…runs where you want: on your machine (self-hosted) or on Mika Cloud. Built in Rust.`
- **`site/src/components/Hero.tsx:39`** : `…acts proactively, and runs entirely on your machine.` → `…runs where you want: on your machine or on Mika Cloud.`
- **`site/src/components/OpenSource.tsx:23`** (h2) : `MIT Licensed. Self-hosted. Yours.` → `MIT Licensed. Self-hosted or Cloud. Yours.`
- **`site/src/components/OpenSource.tsx:26`** (para, l'absolu Prime) : `Mika is free and open source. Your data never leaves your machine.` → `Mika is free and open source. Run it where you want: on your machine (self-hosted) or on Mika Cloud, operated by Senara on AWS, by invitation.`

## Balayage Prime (tout `site/`)

grep exhaustif (`your machine|never leaves|your data|self-host|local|stays on|on-device|offline|no cloud|entirely on|only on your …`) → les 4 claims résolues ; post-édit **aucun absolu restant**. `Nav.tsx:39` = commentaire de code (non-claim), laissé.

## Acceptance criteria

- [x] **AC1** — les 4 surfaces nomment les deux modes (self-hosted + Mika Cloud) ; l'absolu « data never leaves your machine » retiré.
- [x] **AC2** — rien inventé sur Mika Cloud au-delà de : operated by Senara, on AWS, by invitation (pas de prix/features/dates).
- [x] **AC3** — pas d'em dash ajouté ; édits chirurgicaux copy-only ; rien hors `site/` (aucun Rust/cargo).
- [x] **AC4** — `npm run build --workspace=site` ✓ ; `eslint` ✓.
- [x] **AC5** — balayage Prime : aucun absolu résiduel.

## Fire-Disposition

- **Défaut** : affirmations d'hébergement local-only + absolu privacy faux pour le Cloud sur un site PUBLIC. **Corrigé** : deux modes présentés, absolu retiré.
- **Vérif de sortie** : re-QA MPC (les 3 fichiers + ce plan) ; **merge = Vincent** (geste souverain sur le site public).
- **Résidu** : aucun. Le narratif Mika complet (articles) reste non publié, hors scope.

## Références

- Ticket : mika#2365. Formulation : Vincent 11:25 + Prime. PR : mika#2585.
