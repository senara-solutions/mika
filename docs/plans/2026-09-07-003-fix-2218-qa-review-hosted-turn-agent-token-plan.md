---
issue: 2218
merges: 2229
type: fix
title: "Tour de review hébergé : le .env per-agent doit primer sur l'env process du spirit pour les secrets identitaires"
branch: fix/2218/qa-review-hosted-turn-agent-token
---

# Plan — #2218 (fusionne #2229) : en mode serveur, le .env per-agent prime sur l'env process

## Cause racine (vérifiée opérateur 2026-09-07)

Les tours de review des agents famille (mika-qa) s'exécutent **hébergés dans le process spirit**. `Settings::load_for_agent` (`crates/mika-common/src/config.rs` ~L1985-2011) construit les settings per-agent en ajoutant les sources dans cet ordre (avec `config::Config`, **la source ajoutée en dernier gagne**) :

```
1. global config file
2. agent config file
3. per-agent .env  (parsé en TOML, ajouté comme File source)   [conditionnel: si non vide]
4. Environment MIKA_*  (process env, ajouté EN DERNIER = priorité max)
```

Le commentaire du code assume : « In server mode, process env only has global .env values ». **Cette assomption est violée** : l'env du process spirit porte `MIKA_GITHUB_TOKEN = samidarko` (fichier root sourcé le 06/09 ~03Z). Comme la source `Environment` (#4) est prioritaire, elle **ombre** le `MIKA_GITHUB_TOKEN = mika-platform-qa` du `.env` per-agent (#3). `agent_github_token()` (config.rs:1446) rend donc **samidarko**, consommé à `server/handlers.rs:1393` → `run_gh pr review` de mika-qa signe sous `samidarko` = l'auteur des PR → GitHub refuse `--approve` (137 hits 09-07). Preuve : sur #2226, auteur ET reviewer = `samidarko`.

## Fix (cause, pas symptôme — F1)

**En mode serveur (`global_home != agent_home`), ajouter la source TOML du `.env` per-agent APRÈS la source `Environment`**, de sorte que le `.env` per-agent prime sur l'env process pour toute clé qu'il définit. Mécanisme `config::Config` : la source ajoutée en dernier l'emporte ; il suffit donc de déplacer l'ajout de la source per-agent (#3) après l'ajout de la source `Environment` (#4).

Justification (pourquoi corriger l'ordre, pas surcharger une seule clé) :
- En **mode serveur**, le `.env` per-agent EST la source d'autorité de la configuration propre à l'agent (ses secrets identitaires : `MIKA_GITHUB_TOKEN`, `MIKA_GITHUB_APP_*`, clés LLM par-agent). L'env du process spirit porte la config **globale** ; il ne doit pas surcharger une valeur que l'agent a explicitement posée dans son `.env`. Corriger la clé `github_token` seule laisserait la même classe de bug ouverte sur les autres clés identitaires (F1).
- La sémantique « shell-override = process env gagne » est un souci du **mode CLI** (`global_home == agent_home`), où cette branche per-agent ne s'exécute PAS (F3 / AC3). Le daemon spirit n'est pas un shell interactif où un opérateur surcharge ponctuellement une clé per-agent.

Alternative écartée (surcharge post-build de la seule clé `github_token`) : rustine qui contredit la priorité déclarative du builder et ne couvre pas la classe (verdict mika-arch F1).

## Fallback sécurisé (F3)

La source per-agent n'est ajoutée que **si le `.env` per-agent est non vide** (`if !dotenv_vars.is_empty()`, déjà le cas). Un agent hébergé **sans** `.env` per-agent (ou sans la clé) ne déclenche pas la source → il continue d'hériter de l'env process **inchangé**. Le déplacement d'ordre ne régresse donc PAS les agents sans `.env` propre : il ne change le résultat que pour les clés qu'un `.env` per-agent définit explicitement.

## Fusion #2229 / ADR-008 (F4)

#2229 (identité-machine de review distincte) se résout ici : `mika-platform-qa` est **le PAT-par-agent voulu par l'ADR-008**, et il **existe déjà** dans `mika-qa/.env`. Le bug n'est pas l'absence d'identité distincte — c'est le shadow qui l'empêche d'agir. Ce fix **implémente** la direction ADR-008 (aucune divergence à graver). **Hors scope, resté dans le périmètre bloqué en attente Vincent :** le *provisioning de NOUVELLES* identités-machine par agent (créer de nouveaux users/PAT). #2229 ne contenait pas d'autre élément que la question d'habilitation + les cautions de Prime, toutes absorbées ici ; le seul résidu légitimement séparé (provisioning futur) est explicitement Vincent-only.

## Surface

- `crates/mika-common/src/config.rs` — `Settings::load_for_agent` (~L1985-2011), ordre d'ajout des sources.
- Consommateurs inchangés : `agent_github_token()` (L1446), `resolve_github_token()` (L1464), `server/handlers.rs:1393`.

**Classifieur** : ce fix change l'ORDRE DE PRIORITÉ des sources de config d'un token existant ; il ne **minte ni ne valide** aucun token. Attendu classifieur-safe.

## Acceptance Criteria

- **AC1** — En mode serveur (`global_home != agent_home`), si le `.env` per-agent définit une clé `MIKA_*` (dont `MIKA_GITHUB_TOKEN`), `Settings::load_for_agent` rend la valeur du `.env` per-agent même si l'env process définit une valeur différente pour la même clé. **Test unitaire** : `.env` per-agent et env process avec valeurs divergentes → la valeur `.env` per-agent gagne.
- **AC2 (fallback — F3)** — Un agent sans `.env` per-agent (ou dont le `.env` ne définit pas la clé) conserve la valeur de l'env process. **Test unitaire** : `.env` per-agent vide → la valeur env process est rendue.
- **AC3 (CLI inchangé)** — En mode CLI (`global_home == agent_home`), la branche per-agent ne s'exécute pas ; comportement inchangé. **Test unitaire**.
- **AC4 (code — token identité, ex-F2.i)** — Le token chargé pour un tour hébergé mika-qa provient du `.env` per-agent. Couvert par AC1 pour la clé `github_token`.
- **AC5** — `cargo test -p mika-common` (AC1/AC2/AC3) + `cargo build` verts.

## Vérification opérateur pré-déploiement (hors AC bloquant — ex-F2.ii)

Non automatisable en CI (dépend de l'état live des rulesets GitHub) — **checklist opérateur avant de clore**, PAS un AC bloquant :
- `mika-platform-qa` est reviewer habilité : **vérifié** — user GitHub (id 274849584), permission `write` sur senara-solutions/mika → son `--approve` compte pour la 1-approbation requise.
- Nuance à confirmer : le ruleset `forge-gate perimeter` exige `require_code_owner_review` ; confirmer que `mika-platform-qa` est CODEOWNER sur les chemins périmètre, sinon `--approve` ne satisfera pas ce ruleset-là pour ces chemins (le `main protection` principal, lui, n'exige pas le code-owner).
- Reproduire → observer après deploy + restart mika-qa : review postée par `mika-platform-qa` (pas `samidarko`), `--approve` accepté, zéro « Can not approve your own pull request », merge devenu possible.

## Out of scope

- Provisioning de NOUVELLES identités-machine par agent → attente ratification Vincent.
- Hygiène de l'env root (retirer `MIKA_GITHUB_TOKEN` du fichier root sourcé) : remède alternatif côté déploiement ; le fix-code reste l'autorité. À noter, pas à implémenter ici.
- Minting/validation de token.

## Risques

- **Changement de priorité pour toutes les clés du `.env` per-agent en mode serveur** : assumé et justifié (le `.env` per-agent est autoritaire en mode serveur ; shell-override est CLI-only). AC2/AC3 bordent le fallback et le mode CLI.
- **Habilitation reviewer** : si `mika-platform-qa` n'est pas CODEOWNER sur les chemins du ruleset périmètre, `--approve` n'y satisfait pas le code-owner-review → couvert par la checklist opérateur pré-déploiement.
