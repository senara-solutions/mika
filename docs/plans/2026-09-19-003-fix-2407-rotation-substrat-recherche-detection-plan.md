---
title: Une rotation ne doit plus pouvoir emporter le substrat de recherche en silence — Plan
type: fix
date: 2026-09-19
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
issue: senara-solutions/mika#2407
---

# Une rotation ne doit plus pouvoir emporter le substrat de recherche en silence — Plan

## Goal Capsule

- **Objectif :** une passerelle dont le substrat de recherche n'est pas câblé cesse de se déclarer saine. Aujourd'hui elle passe `/readyz`, elle écrit un `info!` que personne ne lit, et six tenants sont muets pendant vingt heures sans qu'aucune surface ne l'ait dit.
- **Moyens :** la configuration en vigueur devient **lisible** (U1, modèle `llm_budget_resolved`) ; l'attente de recherche devient **déclarable**, et sa trahison échoue le démarrage donc la rotation (U2) ; un smoke externe fait **une** recherche réelle et sépare 404 de 200 (U3) ; les deux variables rejoignent enfin les surfaces qu'un déployeur lit, et la ligne devenue fausse depuis mika#1971 est corrigée (U4) ; les tests, dont le contrôle négatif du smoke (U5).
- **Autorité :** le **commentaire 2/2** de mika#2407 (2026-09-19T10:50:06Z) > le corps du ticket > le plan `72acfc4f`. Le commentaire reframe le ticket sur une **mesure** et scinde explicitement le 429 en p2. Ce plan suit le reframe ; § *Problem Frame* M0 explique pourquoi le plan précédent ne pouvait pas le voir.
- **Conditions d'arrêt.**
  - **(a) DÉCLENCHÉE — rapportée, et elle recadre le périmètre sans suspendre le plan.** « Arrêter si le correctif vit dans mika-cloud. » Il y vit **pour moitié** : `values.yaml`, `setup-*.sh` et l'ordonnancement du smoke dans le pipeline de rotation sont hors de ce workspace (M6). L'autre moitié — celle qui rend l'état détectable, et qui est la seule à pouvoir l'être depuis le code — vit ici. Le plan livre le mécanisme et le script ; mika-cloud les câble (ticket de suivi nommé).
  - **(b) NON LEVÉE, et c'est un résultat.** « Pourquoi la rotation a-t-elle perdu ces deux variables ? » La réponse est dans l'historique de `mika-gateway-secrets`, hors d'atteinte. Le plan rend la **conséquence** impossible à ignorer ; il ne reconstitue pas la cause du geste d'infrastructure.
- **Profil d'exécution :** Rust, `crates/mika-gateway` (U1–U2) ; bash + python de test (U3, U5) ; documentation (U4). **Aucun changement de schéma, aucune modification côté agent.**
- **Finish/ship :** le pipeline `/mika` sur la branche `fix/2407/p1-tenant-rotation-vers-la-recherche-par` ouvre la PR qui clôt mika#2407.

---

## Product Contract

### Summary

Le 2026-09-18 vers 14 h, la rotation d'image vers `main-e1342dfa` a déplacé la recherche web derrière la passerelle (mika#1971 : `web_search` ne lit plus `ctx.brave_api_key`, il POST `/internal/search`). La passerelle, elle, ne portait ni `MIKA_BRAVE_API_KEY` ni `MIKA_SEARCH_UPSTREAM`. Les six tenants ont perdu la recherche pendant ~20 h. Le symptôme vécu — « clé API manquante » — était une lecture fidèle de ce que le substrat renvoyait ; ce n'était pas une hallucination.

Le défaut à corriger n'est pas la configuration manquante : elle est posée, avec preuve (`POST /internal/search → 200`, `upstream:brave`). Le défaut est qu'**aucune surface n'a dit que le substrat était mort**. La passerelle répondait `200 OK` sur `/readyz` pendant toute la panne. Le rollout Kubernetes a réussi. Le seul signal émis était un `info!` au démarrage, indistinguable du bruit de boot.

### Problem Frame

#### M0 — Pourquoi le plan précédent traite une autre cause, et ce que cela enseigne

Le plan `72acfc4f` est horodaté **10:39:32Z** ; le commentaire de reframe, **10:50:06Z**. Onze minutes. Le plan ne pouvait pas connaître la mesure de Vincent.

Ce qui mérite d'être relevé, parce que c'est réutilisable : **il avait nommé la bonne cause et n'avait pas pu la trancher**. Son M5 disait, mot pour mot, qu'un déploiement posant la clé sans poser `MIKA_SEARCH_UPSTREAM=brave` laisse `search_egress_client = None` → 404 `search_upstream_not_configured` → le texte exact qu'Al a reçu. Il posait une sonde et une halte, et refusait de conclure sans la mesure. La mesure est arrivée et lui a donné raison.

La rectification n'est donc pas « le plan précédent se trompait » mais **« il avait la cause en U6, une ligne de documentation, pendant que U1–U5 traitaient le 429 »**. C'est une erreur de priorité, pas de lecture. Ce plan inverse le rapport : la cause devient le plan, et le 429 sort du périmètre parce que l'opérateur l'en a sorti.

#### M1 — La passerelle se déclare prête sans son substrat de recherche

`handle_readiness` (`routes.rs:2737`) teste deux choses : le drapeau `state.ready` et un `SELECT 1` sur Postgres. Rien d'autre. `/health`, `/readyz` et `/livez` partagent ce handler pour les deux premiers.

`ready.store(true, Ordering::Release)` (`main.rs:279`) est **inconditionnel** : il suit le boot, quel que soit l'état de `search_egress_client`.

**C'est le mécanisme exact par lequel la rotation est passée.** Le contrôle de déploiement existe — c'est la readiness probe — et il ne pose simplement pas cette question. L'AC1 du reframe demande qu'un contrôle échoue le déploiement ; le contrôle qui a la charge de le faire était vert.

#### M2 — La validation est asymétrique, et le côté silencieux est celui qui a frappé

`GatewaySettings::validate` (`settings.rs:247`) :

| état de la configuration | comportement actuel |
|---|---|
| `MIKA_SEARCH_UPSTREAM='brave'` sans `MIKA_BRAVE_API_KEY` | **`bail!` — la passerelle ne démarre pas** |
| `MIKA_SEARCH_UPSTREAM='zorglub'` | **`bail!` — valeur non reconnue** |
| `MIKA_BRAVE_API_KEY` posée, sélecteur **absent** | **silence total** → `None` → 404 |
| les deux absents | **silence total** → `None` → 404 |

La moitié « le sélecteur ment » est fermée avec soin. La moitié « le sélecteur manque » est ouverte, et c'est celle qui est tombée. La ligne 4 du tableau est l'état mesuré du 18/09.

#### M3 — Le seul signal existant est un `info!` au démarrage

`main.rs:207` émet `info!("egress-search substrate disabled (MIKA_SEARCH_UPSTREAM not set)")`. L'information **existait** pendant les vingt heures de panne. Elle n'a rien déclenché : un `info!` de boot dans un flux de démarrage n'est lu que par quelqu'un qui cherche déjà ce qu'il a trouvé.

Cette ligne n'est pas à supprimer, elle est à **promouvoir et à enrichir** (U1). Un fait de configuration doit être lisible par une requête d'opérateur qui ne sait pas encore ce qu'elle cherche.

#### M4 — Les deux variables ne sont documentées sur aucune surface qu'un déployeur lit

Inventaire mesuré :

- `.env.example:38` — « Brave Search API key for web search skill ». **Faux depuis mika#1971** : la clé va sur la passerelle, pas sur l'agent. Un déployeur qui suit cette ligne pose la clé au mauvais endroit.
- `.env.example` — `MIKA_SEARCH_UPSTREAM` : **absente**.
- `CLAUDE.md` racine § *Environment Variables* — décrit `MIKA_BRAVE_API_KEY` comme servant au « `web_search` builtin skill » (même erreur) ; `MIKA_SEARCH_UPSTREAM` **absente**.
- `crates/mika-gateway/CLAUDE.md` — **zéro occurrence** de « brave » ou de « search upstream ». La documentation du composant qui porte ces deux variables ne les mentionne pas.
- Seule mention réelle : `docs/egress-search-searxng-contingency.md`, un document de contingence sur un upstream qui n'existe pas encore.

**La variable qui décide de l'activation n'est décrite que dans un document sur son remplaçant hypothétique.** C'est la moitié « durabilité » de l'AC3, et elle est intégralement réalisable ici.

#### M5 — Le smoke doit s'authentifier, et c'est une contrainte de conception

`/internal/search` porte `require_bearer_token` (`routes.rs:283`), le même middleware que `/send`. Un smoke externe a donc besoin de `MIKA_INTERNAL_TOKEN`. Ce n'est pas un obstacle — l'appelant légitime est le pipeline de déploiement, qui détient déjà ce secret — mais cela exclut un smoke « anonyme » depuis l'extérieur du cluster, et cela doit être écrit plutôt que découvert.

#### M6 — La moitié mika-cloud est hors de ce workspace, et l'AC3 est coupée en deux

`ls /data/workspace/mika-platform/` rend `claude-pilot` et `mika`. **Pas de `mika-cloud`.** Aucun `values.yaml`, aucun `setup-*.sh` dans l'arbre.

L'AC3 nomme « la `values.yaml` / doc du gateway et les scripts `setup-*.sh` », en précisant lui-même « source mise à jour dans mika-cloud ». Répartition :

| AC3, moitié | où | statut |
|---|---|---|
| doc du gateway (`crates/mika-gateway/CLAUDE.md`, `.env.example`, `CLAUDE.md` racine) | **ici** | U4 |
| `values.yaml`, `setup-*.sh`, appel du smoke au pipeline de rotation | mika-cloud | **ticket de suivi**, ouvert avec ce plan en référence |

Prétendre livrer la seconde moitié depuis ce worktree produirait un plan dont une unité serait invérifiable. La nommer est le livrable honnête.

#### M7 — Le 429 sort du périmètre, et c'est une décision d'opérateur, pas un oubli

Le commentaire 2/2 écrit : « Le backoff/retry sur 429 … reste un durcissement utile mais **distinct** — à traiter en p2 … Ce n'était pas la panne d'aujourd'hui. »

Les trois défauts que le plan précédent avait établis au site du 429 sont **réels** et restent vrais dans l'arbre : `RETRY_BACKOFF_MS = 500` sous la fenêtre `w=1` de Brave ; `UpstreamStatus(429)` aplati en `upstream_error`/502, ce qui rend l'ancien AC4 inatteignable ; et la fuite mika#1783 par `map_substrate_error`, dont le texte opérateur est servi au LLM sur toutes les branches non-config. **Ils ne sont pas traités ici parce que l'opérateur les a scindés**, pas parce qu'ils auraient cessé d'exister.

Le troisième mérite une mention à part dans le ticket de suivi : il n'est pas un durcissement de débit mais une **fuite de doctrine**, et sa gravité ne dépend pas du 429. Le plan `72acfc4f` en porte l'analyse complète et le ticket de suivi doit le citer plutôt que la refaire.

### Requirements

- **R1.** L'état effectif du substrat de recherche est lisible sur une surface d'opérateur, avec la provenance de la valeur, sans jamais exposer la clé. (M1, M3)
- **R2.** Un déploiement qui **attend** la recherche et ne l'a pas échoue de façon visible et immédiate, plutôt que de servir six tenants muets. (AC1)
- **R3.** Un contrôle externe fait une recherche réelle, consomme **une** requête, et sépare 404 (substrat non câblé) de 200 (sain). (AC1, AC2)
- **R4.** Les deux variables sont décrites sur les surfaces que lit un déployeur de la passerelle, et la ligne devenue fausse depuis mika#1971 est corrigée. (AC3, moitié `mika`)
- **R5.** Le contrôle est lui-même testé contre une passerelle sans configuration de recherche — il doit échouer. (AC4)

### Scope Boundaries

**Dans le périmètre :** l'observabilité et la validation de la configuration de recherche dans `mika-gateway` ; le script de smoke et son test ; la documentation des deux variables dans ce dépôt.

**Hors périmètre, délibérément :**
- **Le backoff/retry 429 et la taxonomie `rate_limited`** — scindés en p2 par l'opérateur (M7). Y compris la fuite mika#1783 par `map_substrate_error`, qui doit avoir **son propre** ticket : ce n'est pas un problème de débit.
- **`values.yaml`, `setup-*.sh`, l'ordonnancement du smoke dans le pipeline de rotation** — mika-cloud (M6).
- **Le tier payant Brave et les clés par tenant** — le ticket les exclut lui-même, et la panne mesurée n'était pas un problème de quota.
- **`fetch_url` / `egress_fetch`** — substrat sœur, non touchée par la panne (elle n'a pas de sélecteur d'upstream : `main.rs` la construit toujours).
- **Rendre `/readyz` rouge en l'absence de recherche** — refusé, voir KTD4.

---

## Planning Contract

### Key Technical Decisions

**KTD1 — Le remède principal n'est pas un script, c'est de rendre l'état ambigu indéployable en silence.** L'AC1 demande « un contrôle automatique … échoue le déploiement (ou alarme immédiate) ». Un smoke externe est une **sonde** : il faut qu'on l'appelle, au bon moment, avec le bon jeton, et son absence d'appel est indistinguable de son succès. Une garde de démarrage est **structurelle** : le pod ne démarre pas, le rollout Kubernetes échoue de lui-même, sans ordonnanceur à câbler et sans requête Brave consommée. Le plan livre les deux — la garde ferme la classe mesurée, le smoke couvre ce que la garde ne peut pas voir (une clé présente mais morte). Aucun des deux ne rend l'autre inutile.

**KTD2 — L'attente de recherche doit être DÉCLARÉE, parce que sans déclaration les deux états sont le même.** « Cette passerelle ne veut pas de recherche » et « cette passerelle voulait la recherche et l'a perdue » produisent aujourd'hui des octets identiques : `search_upstream = None`. Aucune garde, aucun smoke, aucune heuristique ne peut les séparer sans savoir ce qui est attendu. D'où `MIKA_SEARCH_REQUIRED`.

Trois paliers, selon la règle maison de mika#2023 (*« unrecognized values fail closed, absence does not »*) :

| valeur | résolution |
|---|---|
| absente ou vide | **non requis** — c'est la forme légitime du poste de dev et des tests, et l'appliquer autrement casserait tout déploiement existant qui ne veut pas de recherche |
| `1` / `true` / `on` / `yes` | **requis** — l'absence de substrat devient fatale au démarrage |
| `0` / `false` / `off` / `no` | **non requis**, explicitement |
| non vide et non reconnue | **requis**, avec un `warn!` nommant la valeur entre guillemets |

Le dernier palier penche vers le refus parce que l'asymétrie des coûts est franche : un rollout qui échoue bruyamment sur une coquille se répare en une minute ; un tenant muet pendant vingt heures ne se répare qu'après qu'un humain l'ait remarqué.

**Objection dure, et sa réponse.** *« Si la rotation a perdu `MIKA_SEARCH_UPSTREAM`, elle aurait perdu `MIKA_SEARCH_REQUIRED` de la même façon — la garde ne se déclenche pas et on est revenu au point de départ. »* C'est exact, et c'est précisément pourquoi le smoke externe n'est pas redondant : il ne lit **aucune** variable du pod, il interroge de l'extérieur et exige un 200. Les deux étages ne se couvrent pas par redondance mais par **provenance différente** — l'un lit ce que le pod croit, l'autre ce que le pod fait. Un plan qui ne livrerait que la garde serait vulnérable à la panne qu'il prétend fermer.

**KTD3 — Aucun appel à Brave au démarrage, et le refus est motivé par deux modes de panne créés.** Vérifier la clé au boot ferait *sonner juste* : on saurait qu'elle vit. Le prix : (a) un pod en crash-loop appellerait Brave à chaque tour, sur un quota de 2000 requêtes/mois partagé entre six tenants — le remède brûlerait l'enveloppe qu'il protège ; (b) la passerelle n'aurait plus le droit de démarrer quand Brave est indisponible, donc une panne Brave couperait **les webhooks, l'A2A et Telegram**. La garde de démarrage lit la configuration, jamais le réseau. La clé morte est le travail du smoke, dont l'appel est ponctuel et ordonnancé.

**KTD4 — `/readyz` ne devient pas rouge, et ce refus est le pendant du précédent.** Coucher la readiness d'une passerelle qui route Telegram, les webhooks GitHub et l'A2A parce que la recherche web est absente est disproportionné : Kubernetes retirerait du service un composant sain à 95 %. La garde de démarrage, elle, est acceptable **parce qu'elle est conditionnée à une déclaration explicite** : l'opérateur qui pose `MIKA_SEARCH_REQUIRED=1` demande exactement ce comportement. La différence entre les deux n'est pas de degré, c'est qu'un seul des deux a été demandé.

**KTD5 — La provenance, sur le modèle exact de `llm_budget_resolved` (mika#2293).** `search_upstream_resolved` (INFO, une fois au démarrage) porte `upstream` (`brave` / `none`), `upstream_source` (`process_env` / `default`), `api_key_present` (**booléen** — jamais la valeur, jamais un préfixe, jamais une longueur), `required`, `required_source`, `endpoint_is_default`. La leçon de mika#2293 s'applique mot pour mot : *un réglage qu'on ne peut pas observer n'est pas un réglage, c'est un espoir.* Sans cette ligne, l'opérateur du 18/09 aurait dû lire le secret Kubernetes pour répondre à « la recherche est-elle câblée ? ».

`api_key_present: true` avec `upstream: none` est le cas de M2 ligne 3 : la clé est là, le sélecteur manque. Il reçoit son propre `warn!` (`search_upstream_key_without_selector`) parce que c'est une demi-configuration dont la forme dit l'intention — personne ne pose une clé Brave par accident.

**KTD6 — Le smoke fait une requête, fixe, et rend trois verdicts distincts.** `scripts/smoke-search-substrate` : un `POST /internal/search` avec une requête constante et `max_results` minimal.

| statut | verdict | sortie |
|---|---|---|
| `200` | substrat sain | `0` |
| `404` `search_upstream_not_configured` | **la panne du 18/09** | `1`, message nommant les deux variables |
| `502` `unauthorized` | clé présente mais refusée | `1`, message distinct — rotation de clé, pas de sélecteur |
| `502` autre / `501` / transport | indéterminé | `2` — distinct d'un échec franc, pour qu'une panne réseau du smoke ne se lise pas comme une panne du substrat |

Le 404 et le 200 sont séparés par construction (AC2). La requête est constante et documentée dans le script pour qu'aucun lecteur de journal ne la prenne pour une recherche de tenant. Le token vient de l'environnement (M5) ; son absence rend `2`, jamais `0` — un smoke qui ne peut pas s'authentifier n'a rien vérifié, et le dire est la seule sortie honnête.

**KTD7 — Le smoke a son test compagnon, selon la convention du dépôt.** Onze scripts de `scripts/` portent un `test-*.sh`. `scripts/test-smoke-search-substrate.sh` lance un serveur HTTP local jetable (python3, déjà utilisé par `test-pilot-egress-proxy-status.py`) et rejoue les quatre lignes du tableau. **Le contrôle négatif est celui qui porte l'AC4** : un faux serveur qui rend 404 doit faire sortir le smoke en non-zéro. Sans lui, un smoke qui rendrait `0` en toute circonstance passerait pour vert et réinstallerait exactement le silence qu'il est censé rompre.

### High-Level Technical Design

```
                         ┌─ ÉTAGE 1 : structurel, dans le pod ─────────────┐
 MIKA_SEARCH_UPSTREAM ──▶│ GatewaySettings::validate                        │
 MIKA_BRAVE_API_KEY   ──▶│   + assert_search_substrate_expectation   [U2]   │
 MIKA_SEARCH_REQUIRED ──▶│       requis && upstream absent → bail!           │
                         │            └── le pod ne démarre pas             │
                         │                └── le rollout K8s ÉCHOUE  (AC1)  │
                         │                                                  │
                         │ main.rs ── search_upstream_resolved (INFO)  [U1]  │
                         │   upstream / upstream_source / api_key_present    │
                         │   required / required_source        (jamais la clé)│
                         └──────────────────────────────────────────────────┘

                         ┌─ ÉTAGE 2 : externe, après rotation ─────────────┐
 pipeline mika-cloud ───▶│ scripts/smoke-search-substrate            [U3]   │
   (ticket de suivi)     │   UNE requête → POST /internal/search            │
                         │     200 → 0    404 → 1 (la panne)               │
                         │     502 unauthorized → 1   autre → 2            │
                         └──────────────────────────────────────────────────┘

 L'étage 1 lit ce que le pod CROIT ; l'étage 2 lit ce que le pod FAIT.
 Une rotation qui emporte les variables de l'étage 1 est vue par l'étage 2.
```

### Assumptions

- **A1.** Le pipeline de rotation de mika-cloud peut appeler un script et lire son code de sortie. S'il ne le peut pas, l'étage 2 devient une alarme à câbler autrement ; l'étage 1 tient seul et ferme déjà la classe mesurée.
- **A2.** `MIKA_INTERNAL_TOKEN` est disponible au point d'appel du smoke (il l'est pour tout ce qui parle à `/send`). Sinon, le smoke rend `2` et le dit — il ne rend jamais un faux vert.
- **A3.** Un `bail!` dans `GatewaySettings::load` fait sortir le processus en non-zéro et donc échouer le rollout. C'est le comportement des deux `bail!` existants de `validate` (M2) ; ce plan emprunte un chemin déjà en service, il n'en crée pas.

---

## Implementation Units

### U1. La configuration de recherche devient lisible

`crates/mika-gateway/src/main.rs` — l'`info!` de la ligne 207 et celui de la ligne 201 sont remplacés par un événement unique `search_upstream_resolved`, émis **sur les trois branches** (brave / vide / absent) et non sur la seule branche désactivée. Champs : `upstream`, `upstream_source`, `api_key_present` (booléen), `required`, `required_source`, `endpoint_is_default`.

Un `warn!(event = "search_upstream_key_without_selector")` est ajouté pour le cas M2 ligne 3 — clé présente, sélecteur absent. **Aucun champ ne porte la clé, ni un préfixe, ni sa longueur** : la discipline Q4 STRIP TOTAL du module `egress_search` s'applique par extension au site qui le construit.

### U2. L'attente de recherche est déclarable, et sa trahison échoue le démarrage

`crates/mika-gateway/src/settings.rs` — champ `search_required: Option<String>` et fonction de résolution à trois paliers (KTD2), avec le `warn!` du palier non reconnu. La validation gagne : *requis et pas d'upstream résolu* → `bail!` nommant les deux variables et le geste qui répare. Le message d'erreur nomme `MIKA_SEARCH_UPSTREAM`, `MIKA_BRAVE_API_KEY` et `MIKA_SEARCH_REQUIRED` — un opérateur qui lit un crash-loop doit trouver le remède dans la ligne, pas dans le dépôt.

La validation existante (`upstream='brave'` ⇒ clé présente) est **inchangée** : elle ferme déjà sa moitié, et la toucher élargirait le rayon sans gain.

### U3. Le smoke post-rotation

`scripts/smoke-search-substrate` — bash, exécutable, en-tête documentaire à la manière de `check-image-tags-immutable.sh` (**la propriété, pas une liste de statuts**). Arguments : URL de base de la passerelle ; jeton lu dans l'environnement. Une requête, quatre verdicts (KTD6), sortie `0` / `1` / `2`. Le corps de la requête et le motif de sa constance sont écrits dans le script.

### U4. La documentation durable (moitié `mika` de l'AC3)

- `crates/mika-gateway/CLAUDE.md` — une section sur le substrat de recherche : les trois variables, la conséquence d'un oubli (404 `search_upstream_not_configured`, tenants muets), la ligne `search_upstream_resolved` à lire, et le smoke. C'est la surface qui n'en portait **aucune** mention (M4).
- `.env.example` — la ligne 38 est corrigée (la clé va sur la passerelle, pas sur l'agent) ; `MIKA_SEARCH_UPSTREAM` et `MIKA_SEARCH_REQUIRED` sont ajoutées avec la mention qu'aucune clé seule n'active le substrat.
- `CLAUDE.md` racine § *Environment Variables* — même correction, et les deux variables ajoutées sous une rubrique passerelle.

### U5. Les tests

**Settings (`#[cfg(test)]` inline) :** les trois paliers de `MIKA_SEARCH_REQUIRED`, dont le palier non reconnu qui doit résoudre **requis** ; *requis + upstream absent* → `Err` ; *requis + brave + clé* → `Ok` ; *non requis + rien* → `Ok` (**contrôle négatif porteur** : sans lui, une garde qui refuserait tout démarrage sans recherche passerait les autres tests).

**Smoke (`scripts/test-smoke-search-substrate.sh`) :** serveur HTTP local jetable rejouant 200, 404, 502 `unauthorized`, 502 `upstream_error`, et le cas sans jeton. Les codes de sortie attendus sont assertés un par un. **C'est ce fichier qui porte l'AC4.**

---

## Verification Contract

- **V1.** `cargo test -p mika-gateway settings` — vert, y compris les trois paliers et le contrôle négatif.
- **V2.** `scripts/test-smoke-search-substrate.sh` — vert ; en particulier le cas 404 sort en non-zéro (AC4).
- **V3.** `cargo clippy --workspace --all-targets` sans avertissement nouveau ; `cargo fmt --check`.
- **V4.** Non-régression au démarrage : une passerelle **sans aucune** variable de recherche démarre toujours (dev local, tests). Le seul démarrage refusé est celui qui déclare attendre la recherche et ne l'a pas.
- **V5 — sonde post-déploiement, avec ses haltes.** Sur la prochaine rotation réelle :
  ```bash
  grep search_upstream_resolved <log-gateway> | jq '{upstream, upstream_source, api_key_present, required}'
  scripts/smoke-search-substrate https://<gateway> ; echo "exit=$?"
  ```
  - `upstream: "brave"` + smoke `exit=0` → sain, c'est le régime attendu.
  - `upstream: "none"` avec `api_key_present: true` → demi-configuration (M2 ligne 3) : le sélecteur manque. **Halte : ne pas rajouter de clé** — le remède est `MIKA_SEARCH_UPSTREAM=brave`.
  - smoke `exit=1` sur `unauthorized` alors que `api_key_present: true` → la clé est là et refusée : rotation de clé, pas de correctif de code.
  - smoke `exit=2` → le smoke n'a rien vérifié (jeton, réseau). **Halte : ce n'est pas un vert** ; établir pourquoi avant de conclure quoi que ce soit sur le substrat.
  - `search_upstream_resolved` **absent du journal** alors que la passerelle tourne → le binaire déployé est antérieur au correctif. **Halte : établir le déploiement avant de toucher au code** (classe mika#2340).

---

## Definition of Done

- `search_upstream_resolved` est émis à chaque démarrage, sur les trois branches, et ne porte aucune forme de la clé.
- Une passerelle déclarant `MIKA_SEARCH_REQUIRED=1` sans substrat résolu ne démarre pas, et son message d'erreur nomme les trois variables et le geste qui répare.
- Une passerelle qui ne déclare rien démarre exactement comme aujourd'hui.
- `scripts/smoke-search-substrate` consomme une requête, sépare 404 de 200, et distingue l'indéterminé de l'échec.
- `scripts/test-smoke-search-substrate.sh` échoue si le smoke rend `0` face à un 404.
- Les trois variables sont documentées sur `crates/mika-gateway/CLAUDE.md`, `.env.example` et le `CLAUDE.md` racine ; la ligne `MIKA_BRAVE_API_KEY` devenue fausse depuis mika#1971 est corrigée sur les deux surfaces qui la portent.
- V1–V4 verts ; V5 posée dans le corps de la PR avec ses cinq haltes.
- **Deux tickets de suivi ouverts et référencés dans le corps de la PR :** (a) mika-cloud — `values.yaml`, `setup-*.sh`, appel du smoke au pipeline de rotation (M6) ; (b) mika — le p2 de durcissement 429, avec la fuite mika#1783 de `map_substrate_error` traitée **à part** et citant l'analyse du plan `72acfc4f` (M7).

---

## Acceptance criteria

Transcrites du **commentaire 2/2** de mika#2407 (2026-09-19T10:50:06Z), qui fait autorité sur le corps : il rectifie le diagnostic sur mesure et scinde explicitement le 429 en p2.

1. **Smoke post-rotation (test négatif)** : après toute rotation d'image gateway/tenant, un contrôle automatique fait une **recherche réelle** et exige `POST /internal/search → 200` ; un 404 `search_upstream_not_configured` **échoue le déploiement** (ou alarme immédiate), il ne passe pas silencieusement.
2. Le smoke distingue 404 (upstream non configuré) de 200 (ok) ; il ne consomme qu'une requête.
3. La `values.yaml` / doc du gateway et les scripts `setup-*.sh` listent `MIKA_BRAVE_API_KEY` + `MIKA_SEARCH_UPSTREAM` comme clés attendues du secret (durabilité — source mise à jour dans mika-cloud).
4. Non-régression : le smoke tourne sur une rotation réelle et attrape un gateway sans config search.

*Note sur l'AC1 :* réalisée en deux étages (KTD1). « Échoue le déploiement » est porté par la garde de démarrage (U2), qui fait échouer le rollout sans script ni requête Brave ; « fait une recherche réelle » est porté par le smoke (U3). La garde seule ne verrait pas une clé morte ; le smoke seul dépend d'être appelé. Ni l'un ni l'autre ne suffit.

*Note sur l'AC3 :* coupée en deux par M6. La moitié `mika` (doc gateway, `.env.example`, `CLAUDE.md` racine) est livrée en U4. La moitié mika-cloud (`values.yaml`, `setup-*.sh`) est **hors de ce workspace** — ticket de suivi, référencé dans le corps de la PR. `MIKA_SEARCH_REQUIRED` s'ajoute aux deux clés que l'AC nomme.

*Note sur l'AC4 :* la « rotation réelle » n'est pas rejouable depuis le dépôt. Elle est réalisée en deux temps — `scripts/test-smoke-search-substrate.sh` prouve en CI que le smoke échoue face à un 404 (la propriété), et V5 la vérifie sur la prochaine rotation réelle (l'instance).

*Note sur les AC du corps d'origine :* les cinq AC du corps (429, taxonomie `rate_limited`, `audit_events`) sont **délibérément non traitées** — le commentaire 2/2 les scinde en p2. Les trois défauts réels qu'elles recouvrent sont inventoriés en M7 et confiés au ticket de suivi ; ils n'ont pas disparu.

---

## Sources

- `crates/mika-gateway/src/routes.rs` — `handle_readiness` (2737–2746), `is_health_probe` (425), routage `/internal/search` + `require_bearer_token` (283–290), `handle_version` (76–83).
- `crates/mika-gateway/src/main.rs` — construction du substrat (185–210), `info!` de désactivation (207), `ready.store(true)` (279).
- `crates/mika-gateway/src/settings.rs` — `search_upstream` (127–137), `validate` (247–265).
- `crates/mika-gateway/src/egress_search/mod.rs` — `handle_internal_search` (309–352), branche 404 `search_upstream_not_configured` (322–334).
- `.env.example` (38–39) ; `CLAUDE.md` racine § *Environment Variables* ; `crates/mika-gateway/CLAUDE.md` (aucune mention) ; `docs/egress-search-searxng-contingency.md` (seule mention de `MIKA_SEARCH_UPSTREAM`).
- `scripts/check-image-tags-immutable.sh` — précédent d'une garde née d'un incident de rotation (mika#2143), et sa doctrine « étendre par propriété ».
- `scripts/canary-pilot-containment` — précédent de sonde post-déploiement livrée dans `mika` et appelée depuis l'extérieur.
- Plan `72acfc4f` (`docs/plans/2026-09-19-002-…`) — analyse du 429 et de la fuite mika#1783, à citer par le ticket de suivi p2.
- mika#1783 (doctrine substrat/tier), mika#1807 (E1 `SearchEgressClient`), mika#1808 (client Brave), mika#1971 (bascule de `web_search` vers le substrat), mika#2143 (rotation d'image), mika#2293 (`llm_budget_resolved`, garde de demi-configuration), mika#2023 (fail-closed sur valeur non reconnue, fail-open sur absence), mika#2340 (établir le déploiement avant de conclure sur le code).
