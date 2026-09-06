---
module: mika-agent
tags: [github-auth, adr-008, github-app, pat, task-engine, auto-pull, wip-rescue, structural-guard]
problem_type: latent-outage
issues: [2205, 2013]
date: 2026-09-06
---

# Un accesseur étroit posé à côté du résolveur canonique est une panne par
# site d'appel, pas une dette

## Le symptôme, mesuré

Le 2026-09-05 vers 16:20, le PAT `MIKA_GITHUB_TOKEN` disparaît de
l'environnement du processus mika-spirit. Deux scans périodiques du
`TaskDispatcher` meurent à la même seconde :

```
auto_pull: running groomed ticket selection   → dernier 16:20:00.320868Z
wip_rescue: running auto-resume scan          → dernier 16:20:00.320882Z
```

Zéro occurrence ensuite. Pendant ce temps le chemin GitHub App était **sain** :
`manager_token_refreshed` jusqu'à 23:17Z, zéro `gh_app_token_exchange_failed`.
L'authentification dont les deux scans avaient besoin était disponible, à portée
d'un champ déjà présent dans la même struct, et ils ne l'ont jamais demandée.

Ni l'un ni l'autre n'a crié : le skip était en `debug!`. Un scan périodique
silencieusement inactif se lit exactement comme un scan qui n'a rien trouvé à
faire.

## La cause

`Settings` porte deux accesseurs voisins, et un seul des deux fait ce que son
nom laisse croire :

| Accesseur | Comportement |
|---|---|
| `agent_github_token()` (config.rs:1414) | `MIKA_GITHUB_TOKEN` **seul**. Aucun repli. |
| `resolve_github_token(app)` (config.rs:1432) | PAT d'abord (ADR-008), puis token d'installation App. |

`TaskDispatcher.github_token` est peuplé depuis le premier
(`server/mod.rs:440`). Les deux scans lisaient ce champ. Le dispatcher détenait
pourtant déjà `settings: Settings` **et** `github_app: Option<Arc<GitHubApp>>` :
le correctif ne réclame ni champ nouveau, ni dépendance, ni changement de
signature — seulement de poser la bonne question.

Ce n'était pas la première fois. mika#2013 avait corrigé **exactement cette
forme** sur le cycle mika-manager (`milestone_manager/spawn.rs:164`), quatre mois
plus tôt, après seize `auth_class=401` en une nuit. Le correctif de 2013 s'était
arrêté à son propre site d'appel.

## La leçon durable

**Quand un résolveur canonique est ajouté à côté d'un accesseur plus étroit,
l'accesseur étroit ne devient pas de la dette technique répartie : il devient une
panne latente indépendante *par site d'appel*.** Chacun tombe séparément, dans un
sous-système différent, avec une signature de log différente, des mois plus tard
— si bien que rien ne les relie. mika#2013 et mika#2205 sont le même bug déclaré
deux fois, à quatre mois d'écart, et découvert deux fois de zéro.

Le corollaire opérationnel : **corriger un site d'appel de cette classe sans
inventorier les autres, c'est planifier la prochaine panne.** L'inventaire est
une seule commande.

### Inventaire au 2026-09-06 (après ce correctif)

`grep -rn "agent_github_token()" crates/` — sept sites subsistent, tous PAT-seul :

| Site | Nature |
|---|---|
| `server/mod.rs:440` | peuple `TaskDispatcher.github_token` — la **source** du défaut d'ici |
| `server/mod.rs:1473`, `server/handlers.rs:1393` | contexte de turn |
| `teams/engine.rs:301,356` | agents d'équipe |
| `tools/delegate_task.rs:298` | délégation |
| `cli/commands/chat.rs:168`, `cli/commands/skills.rs:299` | CLI |

Aucun n'est corrigé ici, et c'est délibéré : la décision se prend **par site**,
sur un critère d'identité (voir ci-dessous), pas par un remplacement mécanique.
Ce tableau existe pour que le prochain incident de cette classe commence par une
lecture au lieu d'une enquête.

## Le critère qui décide, par site

ADR-008 n'exige pas le PAT partout. Il l'exige **là où GitHub lit l'auteur ou le
reviewer** de l'action — revue et merge de PR, où `mika-qa` approuvant une PR
`mika-dev` sous l'identité App partagée est refusé par GitHub lui-même
(`Review Can not approve your own pull request`).

- **Bascule vers `resolve_github_token` légitime** — les opérations dont
  l'auteur n'est pas lu : bascule de label (`gh issue edit --add-label ready`),
  lectures `gh`, rebase, push de branche de brouillon, `gh pr ready`, commentaire.
  C'est le cas des deux scans corrigés ici.
- **Bascule = décision d'identité distincte** — tout ce qui mène à une revue ou
  un merge de PR. Le troisième site PAT-seul de `dispatcher.rs` (l'auto-fire
  après grooming, qui passe `self.github_token.as_deref()` à
  `try_dispatch_pilot_after_groom_success`) est resté hors périmètre pour cette
  raison : il aboutit à une création de PR.

## Le piège de test, et la garde qui le ferme

Le premier réflexe est de tester le résolveur : PAT absent + App saine ⇒ token de
l'App. Ces tests passent — et ne prouvent rien de ce qui était cassé.

**Le défaut de mika#2205 n'était pas une mauvaise résolution, c'était un appelant
qui ne résolvait pas.** Un test du résolveur seul est structurellement aveugle à
ça : il aurait été vert pendant toute la panne.

Ce qui ferme réellement la boucle est une garde sur le **corps des appelants**,
dans la forme déjà employée par
`grooming_marker::tests::no_grooming_regex_outside_this_module` : lire le source,
extraire le corps des deux fonctions, refuser `self.github_token` et exiger
`resolve_periodic_scan_token`. Voir
`dispatcher.rs::tests::mika2205_periodic_scans_do_not_read_the_pat_field_directly`.

Généralisation : **quand le défaut est « l'appelant n'appelle pas », le test doit
porter sur l'appelant.** Tester la fonction appelée est le contrôle qui a l'air
juste et ne peut pas échouer au bon moment.

## Le niveau de log fait partie du correctif

Le skip reste un skip — ni PAT ni App ⇒ le scan ne fait rien ce tick, fail-safe
inchangé. Ce qui change est qu'il le **dit** : `debug!` → `warn!`, avec deux noms
d'événement distincts pour que le grep discrimine.

Grep opérateur (`$MIKA_SPIRIT_LOG_FILE`) :

```sh
grep -E 'auto_pull_no_token|wip_rescue_no_token' "$MIKA_SPIRIT_LOG_FILE"
```

Attente en régime nominal : zéro ligne. Toute occurrence signifie que ni le PAT
ni l'App ne résolvent — un tick perdu par scan, tant que ça dure.

## Sonde post-déploiement

Le seul test qui prouve le correctif en production : redémarrer **sans PAT, avec
App saine**, et vérifier que les deux scans reparaissent au tick suivant.

```sh
grep -E 'auto_pull: running groomed ticket selection|wip_rescue: running auto-resume scan' \
  "$MIKA_SPIRIT_LOG_FILE"
```

S'ils ne reparaissent pas : **halt-and-surface**. Ne pas redéployer à l'aveugle —
la résolution de token est alors elle-même en cause, et c'est une autre enquête.

## Hors périmètre, nommé

- La **cause** de la disparition du PAT de l'environnement (geste opérateur,
  fichier 600 sourcé par conf.d, pattern gh-cron-token). Ce correctif rend la
  panne survivable ; il ne l'empêche pas.
- Les sept sites `agent_github_token()` inventoriés ci-dessus.
- Le troisième site PAT-seul de `dispatcher.rs` (auto-fire post-grooming).

## Liens

- Plan : `docs/plans/2026-09-06-001-fix-2205-auto-pull-wip-rescue-app-fallback-plan.md`
- Précédent identique : mika#2013 (`milestone_manager/spawn.rs`, `SettingsTokenResolver`)
- ADR-008 — identité machine par agent pour revue/merge de PR
