# mika#2697 — une URL de battement n'est pas une route de rapport

**Ticket :** senara-solutions/mika#2697 (`bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`)
**Type :** fix
**Un seul vol.**

## Pourquoi

Depuis le 2026-09-27 06:08Z, aucun rapport de `mika-manager` n'est lisible. Le
puits disque `~/.mika/manager/sink/` n'a plus reçu un fichier, et chaque cycle
journalise `manager_cycle_delivered route=http`. Les deux faits sont le même
fait : `MIKA_MANAGER_DELIVERY_URL` vise
`…/api/v1/agents/mika-manager/heartbeat`, un endpoint qui enregistre un tick,
**ne conserve pas le corps**, et répond `201` pour n'importe quel nom d'entité.

Le POST réussit donc. Et c'est précisément la panne : `select_route`
(`cadence.rs:635`) ne reprend le puits que lorsque l'URL est absente ou que la
livraison **échoue** — un `2xx` ferme les deux portes à la fois. Le rapport est
compté livré, le repli ne se déclenche jamais, et l'instrument confirme :
`cm health --agent mika-manager` montre un tick frais avec `last_reason: null`,
`cm cat mika-manager` ne trouve aucune entité. Dix-sept jours de rapports
écrits puis jetés, avec un journal qui disait « livré ».

Le canal de vie a déjà sa variable depuis mika#1990 :
`MIKA_MANAGER_LIVENESS_URL`. C'est **elle** qui doit viser cet endpoint. La
confusion n'est pas une faute de frappe d'opérateur à corriger une fois : les
deux variables se ressemblent, cm répond `201` à l'une comme à l'autre, et rien
dans le moteur ne distingue une route d'ingestion de rapport d'une route de
battement. Tant que le moteur accepte l'une pour l'autre, la panne est
reproductible au prochain déploiement.

## Ce qui existe déjà, et pourquoi ça n'a pas tenu

- **`url_is_routable`** (`cadence.rs:106`) est le prédicat unique de routage —
  lu par `select_route` *et* par `emit_delivery_resolved`, donc l'événement ne
  peut pas annoncer une route que le cycle ne prendra pas (mika#2267). Il
  répond à « une URL est-elle posée », jamais à « est-ce la bonne ».
- **`ROUTE_OFFLINE_SINK` vs `ROUTE_OFFLINE_SINK_FALLBACK`** distinguent déjà le
  puits nominal de bring-up du puits-après-panne. La distinction est bonne et
  ce plan s'y range : une URL refusée par la garde rend le puits **nominal**,
  pas un fallback — rien n'est en panne, l'URL n'était simplement pas une route
  de rapport.
- **`manager_delivery_resolved`** (`spawn.rs:574`) émet une fois par démarrage
  `route_normal`, `route_escalation`, `delivery_url_set`, `liveness_url_set`.
  C'est déjà l'événement de configuration de ce module. Il a annoncé
  `route=http` tous les jours depuis le 27/09 — fidèlement, puisqu'une URL
  était posée. L'instrument n'a pas menti ; il n'avait pas la question.
- **Discipline de journal du module : jamais une URL en clair.** Le token est un
  booléen (`delivery_token_present`), la liveness aussi (`liveness_url_set`), et
  `manager_delivery_observability_2267.rs:309` balaie tous les champs de la
  ligne pour interdire la fuite. Ce plan ne rompt pas cette discipline — voir
  § Observabilité.

## Décision

**La garde vit à la frontière de lecture de l'environnement, et efface la
valeur.** `manager_config_from_env` (`spawn.rs:140`) est appelée une fois par
process, juste avant tous les lecteurs. Une `MIKA_MANAGER_DELIVERY_URL` dont le
chemin est un endpoint de battement y devient `None` — « comme si l'URL était
absente », mot pour mot l'attendu du ticket.

Pourquoi effacer plutôt que dévier la route plus bas, dans `select_route` :

1. `select_route`, `route_name_for`, `delivery_url_set` et
   `manager_cycle_delivered` deviennent cohérents **gratuitement**, par
   construction. Normaliser en aval laisserait `manager_delivery_resolved`
   annoncer `delivery_url_set=true route_normal=http` pendant que le cycle écrit
   dans le puits — un instrument qui mentirait d'une manière nouvelle pour en
   réparer une ancienne.
2. Le prédicat de routage reste à **lecteur unique** (`url_is_routable`), la
   propriété que mika#2267 a installée et qu'un second chemin de décision
   dissoudrait.
3. La ligne ERROR est émise **une fois au démarrage**, non à chaque cycle. Une
   erreur de configuration est un fait de démarrage ; la répéter toutes les cinq
   minutes pendant dix-sept jours est le bruit dans lequel la panne d'origine
   s'est cachée.

**Le prédicat porte sur la forme du chemin, jamais sur une liste d'URLs.** Le
dernier segment non vide du chemin, insensible à la casse, est-il `heartbeat` ?
Une liste d'URLs connues serait exacte le jour de l'atterrissage et fausse au
premier déploiement cm suivant.

- Sur le **chemin** et pas sur la chaîne entière : `https://reports.example/
  ingest?from=heartbeat` est une route de rapport, et `https://
  heartbeat.example.com/reports` aussi. Un `contains("heartbeat")` les
  refuserait toutes les deux.
- En position **terminale** et pas « contient un segment » : la règle décrit ce
  qu'un endpoint *est*, et `…/heartbeat/reports` ne l'est pas.
- `heartbeat` **seul**, pas `heartbeats` ni `ping` ni `alive` : cm n'expose que
  cette forme, et le reste serait de la spéculation sur des endpoints qui
  n'existent pas. Voir § Hors périmètre.
- **Une URL non parsable rend `false`**, et c'est une décision : refuser ce
  qu'on ne sait pas lire ferait du prédicat un second mode de panne. Une URL
  illisible partira en `http`, le POST échouera, et le repli
  `ROUTE_OFFLINE_SINK_FALLBACK` la rattrapera — route déjà livrée, déjà nommée,
  déjà testée.

**La garde couvre aussi `MIKA_MANAGER_ESCALATION_URL`.** Même fonction, même
site de lecture, trois lignes de plus. `select_route` prend l'escalation URL
pour exactement la même raison — poster un rapport — et un rapport `Blocked`
jeté par un endpoint de battement est le pire cas de la classe. Fermer une
moitié du défaut en laissant l'autre ouverte serait le choix à justifier, pas
celui-ci.

## Fichiers

| fichier | geste |
|---|---|
| `crates/mika-agent/src/milestone_manager/cadence.rs` | `pub(crate) fn url_is_heartbeat_endpoint(&str) -> bool`, voisine de `url_is_routable` — même classe, même voisinage. Parse via `url::Url` (déjà en dépendance du crate, `Cargo.toml:66`), aucun byte-slicing (lint mika#764). |
| `crates/mika-agent/src/milestone_manager/spawn.rs` | `fn reject_heartbeat_endpoint(Option<String>, &str) -> Option<String>` + application à `delivery_url` et `escalation_url` dans `manager_config_from_env`. |
| `docs/operator/environment-variables.md` | § manager : quelle variable vise quoi, le puits comme route lisible, la sonde de la garde, le geste de remédiation. |
| `.env.example` | Avertissement de deux lignes sur `MIKA_MANAGER_DELIVERY_URL` (ligne 312) renvoyant à `MIKA_MANAGER_LIVENESS_URL`. |

## Observabilité

Un événement, émis une fois par démarrage, au niveau **ERROR** :

```
event = "manager_report_url_rejected_heartbeat_endpoint"
env_var = "MIKA_MANAGER_DELIVERY_URL"     # ou MIKA_MANAGER_ESCALATION_URL
use_instead = "MIKA_MANAGER_LIVENESS_URL"
matched_segment = "heartbeat"
route = "offline_sink"
```

- **La ligne nomme les deux variables** — la fautive et la correcte — parce que
  l'opérateur qui la lit a besoin du geste, pas du diagnostic.
- **Aucune URL, aucun chemin.** La discipline du module est que le journal porte
  des booléens et des noms, jamais de matériel d'URL
  (`manager_delivery_observability_2267.rs:309` l'assère en balayant tous les
  champs). `matched_segment` est le littéral `"heartbeat"`, exact par
  construction puisque le prédicat ne reconnaît que lui — pas une valeur
  extraite de l'entrée.
- **Pas de second événement de configuration.** `manager_delivery_resolved`
  reste l'unique ligne de configuration et dira désormais
  `delivery_url_set=false route_normal=offline_sink` — honnête par
  construction, puisque la config qu'elle lit est déjà normalisée.

**Sonde opérateur**, après déploiement :

```
journalctl -u mika-spirit --since '-10min' | grep manager_report_url_rejected_heartbeat_endpoint
journalctl -u mika-spirit --since '-10min' | grep manager_delivery_resolved
ls -lt ~/.mika/manager/sink/ | head -3
```

La première ligne nomme la variable à déplacer ; la deuxième doit porter
`route_normal=offline_sink` ; la troisième est l'AC2. **Son silence ne prouve
rien avant le premier cycle** — le plancher de delivery est de 6 h, le poll de
5 min, et un cycle sans changement d'état ne délivre pas (`state_changed ||
heartbeat_fired`). Lire le troisième point au prochain changement d'état du
jalon ou à 6 h, pas à la minute du redémarrage.

## Fire-Disposition

Ce plan livre un détecteur : la garde est un validateur de configuration dont
le chemin de succès est « aucune violation trouvée » (une config saine traverse
`manager_config_from_env` sans émettre de ligne). Option retenue :
**(a) exception nommée en allowlist — allowlist vide par construction.**

- **Détail d'implémentation.** Le détecteur est un validateur d'entrée au
  démarrage, pas un scan de corpus : il n'existe **aucune population de
  violations dans l'arbre** à exempter. Aucune allowlist n'est livrée, et aucune
  ne doit être ajoutée — amnistier une URL de battement sur la route de rapport
  serait exactement le défaut mesuré, rendu légitime.
- **La seule violation existante vit hors du dépôt**, dans `~/.mika/.env` de
  l'hôte de production, et elle n'est pas amnistiée : elle est **détectée** et
  son effet est neutralisé (route puits), ce qui est déjà le comportement
  souhaité. Sa remédiation est le geste opérateur de l'attendu #3 du ticket —
  déplacer l'URL vers `MIKA_MANAGER_LIVENESS_URL` puis redémarrer — vérifié par
  l'AC2. **Le détecteur atterrit donc armé, et rend l'arbre vert le jour de son
  atterrissage** : il ne bloque aucun CI, ne refuse aucun démarrage, et ne peut
  pas devenir stale faute d'exception à périmer.
- **Assertion auto-nettoyante.** `url_is_heartbeat_endpoint` est testée sur sa
  table complète, dont les trois cas négatifs qui sont le vrai risque de
  relâchement — `?from=heartbeat` en query, `heartbeat.example.com` en host,
  `…/heartbeat/reports` en position non terminale. Un `contains("heartbeat")`
  écrit à la place rougit sur les trois. Le contrôle de bonne foi du détecteur
  est le **contrôle négatif de l'AC1** : garde retirée, le test de l'URL
  heartbeat doit passer au rouge — exécuté et noté dans le corps de PR.
- Ce vol **n'ajoute aucun scan source**, donc aucune allowlist de fichiers, et
  **aucune entrée dans `scripts/canonical-tokens.tsv`** : ce registre recense
  les jetons qu'un code *matche* (mika#2201), et aucun événement `manager_*` n'y
  figure aujourd'hui — les nouveaux noms ne sont lus que par leurs propres
  tests.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/cadence.rs` — prédicat + doc de décision | 30 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` — tests du prédicat (`mika2697_*`, table 8 cas) | 50 |
| `crates/mika-agent/src/milestone_manager/spawn.rs` — helper, application aux deux URLs, ligne ERROR | 35 |
| `crates/mika-agent/src/milestone_manager/spawn.rs` — tests de routage (`mika2697_*`, les trois cas de l'AC1 + escalation) | 55 |
| `.env.example` — avertissement | 6 |

Total estimé : 176 lignes

Le ticket borne à « une centaine de lignes tests compris » ; les 176 tiennent
dans cet ordre de grandeur, avec 105 des 176 en tests. Très largement sous le
seuil de découpage de mika#2636 (1 000 lignes) — aucune phase, un seul vol.

## Definition of Done

1. `url_is_heartbeat_endpoint` existe dans `cadence.rs`, parse via `url::Url`,
   et son jugement porte sur le dernier segment non vide du chemin.
2. `manager_config_from_env` efface `delivery_url` **et** `escalation_url`
   lorsqu'elles visent un endpoint de battement, en émettant
   `manager_report_url_rejected_heartbeat_endpoint` (ERROR) qui nomme la
   variable fautive et `MIKA_MANAGER_LIVENESS_URL`.
3. Aucune URL ni aucun chemin dans la ligne ERROR.
4. Tests sans réseau, suivant le modèle `#[serial]` + `clear_manager_env()` +
   `set_env` déjà en place dans `spawn.rs` (`spawn.rs:1619`).
5. Le contrôle négatif est exécuté : garde retirée, le test de l'URL heartbeat
   rougit ; résultat noté dans le corps de PR.
6. `docs/operator/environment-variables.md` dit quelle variable vise quoi, que
   cm **n'a pas** d'endpoint d'ingestion de rapports aujourd'hui, et que le
   puits disque est donc la route lisible — avec la sonde et le geste de
   remédiation.
7. `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
   `cargo fmt --check`, `make verify-bundled-skills`,
   `scripts/verify-pipeline.sh origin/main` passent.
8. Le corps de PR porte le geste opérateur de l'attendu #3 (déplacer l'URL dans
   `~/.mika/.env`, redémarrer) comme **étape de déploiement manuelle**, puisque
   le code seul ne restaure pas la lisibilité des rapports — il la rend
   atteignable.

## Acceptance criteria

- **AC1** — La garde et ses tests sont livrés : URL heartbeat → route puits ;
  URL d'ingestion → route http ; URL absente → route puits. Tests sans réseau.
  Avec le **contrôle négatif** : garde retirée, le test de l'URL heartbeat passe
  au rouge.
- **AC2** — Après déploiement, un fichier neuf dans `~/.mika/manager/sink/` à
  l'issue du premier cycle.
- **Taille** — petite, une centaine de lignes tests compris. Un seul vol.

**Note de vérification sur l'AC2 :** l'AC2 est post-déploiement et dépend du
geste opérateur de l'attendu #3. Elle ne peut pas être attestée depuis la
session d'implémentation, et le corps de PR ne doit pas la déclarer satisfaite —
il porte le geste, la sonde (§ Observabilité) et la fenêtre d'attente (prochain
changement d'état du jalon, ou 6 h au plancher de heartbeat). Revendiquer un
état système qu'aucun résultat d'outil ne confirme est interdit par la règle de
grounding du dépôt.

## Hors périmètre

- **Les autres formes d'endpoint de battement** (`heartbeats`, `ping`, `alive`,
  `/pulse`). cm n'expose que `…/heartbeat` ; élargir le prédicat à des endpoints
  qui n'existent pas est de la spéculation, et chaque forme ajoutée est un
  faux-positif de plus sur une route de rapport légitime. La fonction est à un
  seul endroit : le jour où cm en expose une autre, le geste est d'une ligne.
- **La garde symétrique sur `MIKA_MANAGER_LIVENESS_URL`** (refuser une URL
  d'ingestion sur le canal de vie). Le défaut n'est pas symétrique : un endpoint
  d'ingestion qui reçoit un POST léger de battement enregistre quelque chose de
  lisible — personne ne perd dix-sept jours de rapports.
- **`MIKA_MANAGER_HEALTH_URL`**, dont la sémantique est inverse (on y *lit* la
  santé d'un exécuteur). Elle ne poste aucun rapport ; `select_route` ne la
  regarde pas.
- **Un endpoint d'ingestion de rapports côté cm.** C'est le travail qui rendrait
  `MIKA_MANAGER_DELIVERY_URL` utile, et il vit dans `mika-cloud`. Ce plan dit
  seulement ce qui est vrai aujourd'hui : il n'en existe pas, donc la variable
  reste absente et le puits est la route lisible.
- **Le rafraîchissement du token App à ~1h** (`spawn.rs:188`), défaut voisin
  déjà nommé dans le code et déjà tracké ailleurs.
- **Les dix-sept jours de rapports perdus.** Ils n'ont jamais été persistés, ni
  côté cm ni côté puits. Rien à récupérer.
