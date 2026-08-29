---
issue: senara-solutions/mika#2013
type: fix
status: groomed
branch: bug/2013/auth-per-agent-github-app-token-json
date: 2026-08-29
---

# mika#2013 — le token du milestone-manager est gelé au spawn, pas périmé sur disque

## Résumé exécutif

Le symptôme du ticket est réel et mesuré : `manager_cycle_error auth_class=401` en boucle,
16 occurrences la nuit du 26 au 27, Mika Manager incapable de lire ses milestones.

**La cause nommée dans le corps du ticket n'est pas la bonne.** Le corps attribue le 401 à des
caches `github_app_token.json` par agent « qu'aucun chemin ne renouvelle ». La lecture du code
montre l'inverse : tout chemin qui *lit* ces fichiers les renouvelle, et le manager ne les lit
jamais. Le 401 vient d'un token résolu **une seule fois au spawn** et gelé en mémoire pour toute
la vie du processus.

Le correctif attendu n°2 du ticket (« un 401 ne doit pas être avalé par *continuing loop* ») est
valide tel quel et reste dans le périmètre.

## Ce que dit le code (preuves)

### Le cache par agent se renouvelle — il n'est simplement lu par personne

`installation_token_with_file_cache` (`crates/mika-common/src/github_app.rs:254`) :

1. `read_file_cache` parse `expires_at` et applique `is_valid` (même buffer 5 min que le cache mémoire) ;
2. si périmé → `None` → retombe sur `installation_token()` (échange JWT) ;
3. réécrit le fichier avec l'expiry réelle, permissions `0o600`.

Les **trois** sites qui touchent `github_app_token.json` passent tous par ce helper, sans exception :

| site | chemin du cache | via le helper ? |
|---|---|---|
| `crates/mika-cli/src/commands/token.rs:67` | global | oui (`:69`) |
| `crates/mika-cli/src/commands/credential_helper.rs:95` | global | oui (`:97`) |
| `crates/mika-cli/src/commands/skills.rs:292` | **par agent** (`agent_home`) | oui (`:293`) |

`grep -rn "github_app_token" crates/ --include="*.rs"` hors `github_app.rs` ne rend que ces trois
lignes. **Aucun lecteur brut.** Un fichier périmé sur disque est inoffensif : le prochain appel qui
le lit le régénère.

Les cinq `expires_at` mesurés par samidarko portent des horodatages à 3 secondes d'intervalle
(`08:21:04` → `08:21:07`) : c'est **une seule passe CLI** qui les a écrits, puis plus rien ne les a
relus. Ils sont vieux parce qu'inutilisés, pas parce que le renouvellement manque.

### Le vrai chemin : un token gelé au spawn

```
spawn.rs:157   let github_token = settings.resolve_github_token(github_app).await;   // UNE FOIS
      ↓        stocké dans ManagerConfig.github_token: Option<String>
cadence.rs:405 let reader = Reader::new(cfg.github_token.clone());                   // CHAQUE CYCLE
      ↓
reader.rs      cmd.env("GH_TOKEN", t)                                                // CHAQUE `gh`
```

`Settings::resolve_github_token` (`crates/mika-common/src/config.rs:1179`) rend une `String`, pas
une poignée renouvelable :

- PAT d'abord (`MIKA_GITHUB_TOKEN`) — identité machine par agent, ADR-008 ;
- sinon `app.installation_token()` — **noter : pas la variante `_with_file_cache`**, donc ce chemin
  ne touche jamais les fichiers par agent.

Quand le PAT est absent et que l'App résout, le token a un TTL ~1 h. Au bout d'une heure le manager
cycle en 401 jusqu'au redémarrage du processus, alors que `verify_gh_auth` est passé au boot.

### Le défaut était déjà écrit dans le code

`spawn.rs:144-157`, commentaire verbatim laissé par mika#1968 :

> **A3 P1 note — App token lifetime hazard (deferred per plan §5c).** […] `ManagerConfig.github_token`
> is populated ONCE at spawn time and forwarded verbatim to `gh` on every cycle. After 1h the manager
> cycles silently 401 until the process restarts […] **Follow-up ticket needed to periodically refresh
> via `resolve_github_token` inside the cycle loop.**

**mika#2013 est ce follow-up.** Le diagnostic était en dépôt avant l'incident ; ce qui manquait, c'est
que la panne crie (correctif n°2).

### L'avalement

`spawn.rs:345-361` — le bras d'erreur de la boucle est un `warn!` et rien d'autre. Pas de compteur,
pas de seuil, pas d'escalade — alors que `ManagerConfig` porte **déjà** `escalation_url` et que
`classify_cycle_error` rend **déjà** un `AuthClass`. Les matériaux de l'alarme sont là, non câblés.

## Correctif

### Volet A — renouveler dans la boucle

Le renouvellement est **déjà implémenté** dans `installation_token()` (cache mémoire + buffer 5 min).
Le bug est purement qu'on l'appelle une fois au lieu d'à chaque tour. Le correctif est donc de
re-résoudre avant chaque cycle plutôt que d'introduire une quelconque machinerie de refresh.

Forme retenue : la boucle de `spawn.rs` conserve `Settings` + `Option<GitHubApp>` et rafraîchit
`cfg.github_token` avant chaque appel à `run_manager_cycle`.

Pourquoi cette forme :
- coût nul dans le cas courant — PAT configuré → `resolve_github_token` retourne immédiatement ;
- coût nul aussi en mode App tant que le token est valide — le cache mémoire répond ;
- l'échange JWT n'a lieu qu'à l'approche réelle de l'expiry ;
- `run_manager_cycle` et `run_manager_cycle_with` ne changent pas de signature, donc les tests purs
  de `cadence.rs` (qui construisent `ManagerConfig { github_token: None, .. }`, ex. `:489`) restent
  intacts. Rayon d'explosion minimal.

Émettre un événement `manager_token_refreshed` (INFO) quand la valeur résolue **change**, pour que le
renouvellement soit observable et non supposé.

### Volet B — le 401 doit crier

Compter la **durée** d'échec d'authentification continu, pas le nombre de cycles.

Décision tranchée (passe architecte 1, point 3) : un seuil en cycles n'a pas de sémantique temporelle
stable, parce que `poll_interval` est configurable par l'opérateur — `N=3` peut valoir 15 minutes ou
trois heures selon la cadence. Le seuil est donc une durée.

- La boucle retient l'instant du **premier** cycle consécutif classé `AuthClass::Unauthorized`.
- Tout cycle réussi efface cet instant (remise à zéro).
- Quand `elapsed()` depuis cet instant dépasse **30 minutes**, émettre un
  `manager_auth_persistent_failure` en ERROR portant la durée et le nombre de cycles échoués, et
  escalader par la surface existante `escalation_url` quand elle est configurée.
- L'alarme ne se répète pas à chaque cycle une fois franchie : ré-émettre au plus une fois par heure
  tant que l'état persiste, pour ne pas reproduire le bruit que ce ticket corrige.

Valeur retenue : **30 minutes**, en constante nommée. Elle est très en deçà des ~14 h de silence
observées la nuit du 26 au 27, et au-dessus de tout hoquet réseau plausible. Pas de surcharge par
variable d'environnement en v1 — décision explicite, pas une omission (voir Hors périmètre).

## Critères d'acceptation

- **AC1** — Le token utilisé par le manager est re-résolu à chaque cycle, pas une seule fois au spawn.
  Test : deux cycles avec une `GitHubApp` dont le token mémoire est forcé expiré entre les deux ;
  le second cycle doit émettre un token différent du premier.
- **AC2** — `run_manager_cycle` / `run_manager_cycle_with` gardent leur signature ; les tests
  existants de `cadence.rs` passent sans modification.
- **AC3** — Un échec `AuthClass::Unauthorized` continu depuis plus de 30 minutes produit un
  `manager_auth_persistent_failure` en ERROR **et** une escalade sur `escalation_url` quand elle est
  configurée. Le test pilote l'horloge (instants injectés), il n'attend pas 30 minutes réelles.
  Test anti-vacuité : la même séquence, même durée, avec `AuthClass::Other` ne déclenche **ni** l'un
  **ni** l'autre — et un `Unauthorized` de 29 minutes non plus.
- **AC4** — Un cycle réussi efface l'instant de départ. Test : `Unauthorized` pendant 29 min → un
  cycle réussi → `Unauthorized` pendant 29 min ne déclenche pas l'alarme, alors que la durée cumulée
  dépasse le seuil.
- **AC4b** — L'alarme franchie ne se ré-émet pas à chaque cycle : au plus une fois par heure tant que
  l'état persiste.
- **AC5** — `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` verts.

## Hors périmètre

- **Les caches `github_app_token.json` par agent.** Ils se renouvellent déjà ; la mitigation manuelle
  décrite dans le ticket était sans effet sur la panne. Ne pas les modifier, ne pas les supprimer :
  ce ticket ne les touche pas.
- Faire lire aux agents le cache global plutôt que le leur (option 1 du corps du ticket) — sans objet,
  le chemin fautif ne lit aucun des deux.
- L'alarme côté veilleur famille (le ticket la mentionne comme porteur possible) — le présent ticket
  livre l'événement et l'escalade ; qui les regarde est un autre lot.
- Rendre le seuil de 30 minutes configurable par variable d'environnement — décision explicite de v1,
  pas une omission. Une constante nommée suffit tant qu'un opérateur n'a pas exprimé le besoin de la
  régler ; l'ajouter d'avance serait de la surface non demandée.

## Divergence corps↔code déclarée

Le corps du ticket affirme : *« les caches par agent […] restent périmés indéfiniment. Aucun chemin ne
les renouvelle. »* Le code dit le contraire (`github_app.rs:254`, trois sites, aucun lecteur brut).
Le corps de l'issue a été corrigé au grooming avec les références fichier:ligne ; l'intention, le
symptôme mesuré et le correctif attendu n°2 sont conservés intégralement. Voir le commentaire de
clôture sur l'issue.

## Historique de grooming

- Passe architecte 1 (`mika-arch`, 2026-08-29T03:17:58Z) — **ITERATE**. Points 1, 2 et 4 validés
  (placement du rafraîchissement, exhaustivité du tracé des consommateurs, sûreté de la mise hors
  périmètre des caches par agent). Point 3 : le seuil non fixé est une décision chargeante non
  résolue ; trancher pour une durée plutôt que des cycles, `poll_interval` étant configurable.
  Appliqué ci-dessus.

## Lié

- mika#1968 — a introduit `resolve_github_token` au spawn et a **écrit le défaut en commentaire**
  (`spawn.rs:144-157`) en demandant ce follow-up.
- mika#1781 — l'autre cause silencieuse du débit à plat, trouvée le même matin.
- RT#009 — « la panne ne crie pas, donc personne ne la voit » : le volet B en est une instance.
