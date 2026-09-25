# fix(deploy) : `check-ngrok` décrit une topologie abandonnée et fabrique du faux diagnostic (mika#2135)

> Ticket : senara-solutions/mika#2135
> Type : fix (substrat de déploiement + observabilité opérateur)

---

## 1. Le défaut, mesuré

`mika/Makefile:59` termine la chaîne `deploy` par `check-ngrok`, dont la recette
(`Makefile:77-84`) sonde `http://localhost:4040/api/tunnels` — l'API locale d'un
ngrok — et imprime sur échec :

```
  ⚠  WARNING: ngrok is not running!
  Telegram webhooks will not reach the gateway.
  Start ngrok: ngrok http 8080
```

Les deux phrases sont fausses. La chaîne réelle est **Freebox (redirection de
port) → Synology (reverse proxy) → gentux:8080**, `setWebhook` posé à la main.
ngrok n'est nulle part dedans.

**Le coût n'est pas cosmétique** : le 2026-09-01 l'avertissement a produit une
fausse alerte publiée dans un rapport d'orchestration. Un avertissement émis à la
fin de *chaque* `make deploy` est lu comme un signal de substrat ; quand il décrit
une topologie morte, il fabrique du faux diagnostic au moment précis où l'on
regarde l'état du système.

### Ce que la lecture du code confirme et ajoute

- **ngrok n'a aucun consommateur vivant.** Un balayage de `*.rs`, `*.sh`,
  `*.toml`, `*.yml`, `*.md`, `Makefile`, `.env.example` ne rend que : la recette
  elle-même, une référence *textuelle* dans `scripts/smoke-webhook-flood.sh:21`
  (« FAIL-OPEN POSTURE (matches `check-ngrok`) »), et des mentions historiques
  dans `docs/plans/**` et `docs/solutions/**`. **Aucun code ne lit ngrok.** La
  cible est un vestige entier, pas une branche d'une topologie supportée.
- **La topologie réelle n'est écrite nulle part.** `grep -rl
  'gentux\|Synology\|Freebox\|dupont.tech' docs/` ne rend aucune page de
  topologie — uniquement des plans et des solutions sur d'autres sujets.
  `docs/deployment.md` § 1 décrit une architecture *hébergée* générique
  (« Ingress / Load Balancer »). AC4 est donc un manque réel, pas un rappel.
- **`check-ngrok` sonde une **API tierce**, jamais la chaîne.** Même un ngrok
  vivant n'aurait rien prouvé de Telegram → gateway. La cible n'a jamais été une
  vérification de chaîne ; c'était une vérification de *présence d'outil*.

---

## 2. Deux décisions de conception, tranchées ici

AC1 laisse deux issues au grooming. Ce plan les tranche.

### D1 — Remplacer, ne pas supprimer

**Décision : la cible est renommée et réécrite.**

La suppression est écartée pour la raison qu'AC3 écrit lui-même : « une
correction qui se contenterait de rendre l'avertissement inconditionnellement
silencieux n'aurait rien réparé — elle aurait juste supprimé un signal au lieu de
le rendre vrai. » Supprimer est la forme extrême de ce silence. Et le ticket
établit par ailleurs qu'une chaîne webhook cassée est un incident réel et
coûteux : le poste post-`restart` est exactement l'instant où il faut le savoir.

### D2 — La chaîne est **déclarée**, jamais dérivée

C'est la décision structurante, et elle vient d'une mesure : le gateway charge
sa configuration par `dotenvy::dotenv()` depuis son **CWD**, avec le commentaire
explicite `// Load .env from CWD (gateway has no ~/.mika/ home directory)`
(`crates/mika-gateway/src/main.rs:57-58`). Sous OpenRC le CWD est `/` et la
configuration vient de l'EnvironmentFile du service. **Il n'existe donc aucun
fichier que `make` pourrait lire pour apprendre l'URL publique.**

Trois dérivations sont refusées, chacune parce qu'elle rendrait une réponse
*fausse* plutôt qu'absente :

1. **Lire un `.env` du dépôt** — reconstruirait une cascade dont l'ordre réel est
   inconnu du Makefile ; une provenance fausse est strictement pire qu'aucune
   provenance (doctrine mika#2293).
2. **Parser le journal de démarrage du gateway** (`info!(settings = ?settings)`,
   `main.rs:79`) — c'est un rendu `Debug` non contractuel, dont le chemin de
   fichier est lui-même configurable (`MIKA_GATEWAY_LOG_FILE`).
3. **Deviner l'URL depuis le nom d'hôte** — discrimine zéro.

La sonde prend donc l'URL d'une **déclaration explicite**, et son absence vaut
« rien vérifié » — jamais « tout va bien ». Même trajectoire que
`metadata.dispatch_worktree_file` (mika#2249) : le producteur dit, le lecteur ne
devine pas. La déclaration est précisément le geste que la page AC4 documente.

---

## 3. La conception

### 3.1 Un script, trois verdicts

Nouveau `scripts/smoke-webhook-chain`, modelé sur `scripts/smoke-search-substrate`
(mika#2407) — même famille de problème : *un substrat non câblé est, vu du
dehors, indistinguable d'un substrat sain.*

| exit | Verdict | Ce que ça veut dire |
|---|---|---|
| `0` | **chaîne vivante** | une requête émise sur l'URL publique déclarée est revenue avec une réponse **applicative du gateway** |
| `1` | **chaîne cassée** | le gateway local ne répond pas, ou la requête publique n'atteint pas le gateway |
| `2` | **rien vérifié** | aucune URL déclarée, pas de réseau, ou réponse inclassable |

`2` **n'est pas un succès**, et c'est le verdict qui porte le plus de doctrine :
un smoke qui n'a rien pu joindre n'a rien vérifié, et le dire est la seule sortie
honnête (formulation reprise de `smoke-search-substrate` § EXIT CODES).

### 3.2 Deux sondes de **provenance différente**, pas une redondance

Reprise explicite de l'argument mika#2407 (« l'une lit ce que le pod croit,
l'autre ce que le pod fait ») :

- **Terminus local** — `GET http://localhost:<port>/health`. La route est non
  authentifiée et rend 200 si `state.ready` et `SELECT 1` passent
  (`routes.rs:359` + `handle_readiness` `routes.rs:2847-2856`). Port : déclaré
  via `MIKA_GATEWAY_PORT`, défaut `8080` (`settings.rs:54-56`).
- **Traversée publique** — une requête sur l'URL publique déclarée, qui traverse
  Freebox + Synology + gateway.

Leur croisement est ce qui rend le message d'échec **actionnable**, et c'est
toute la valeur ajoutée sur `check-ngrok` :

| local | traversée | message |
|---|---|---|
| KO | — | « le gateway n'écoute pas » → `rc-service mika-gateway status` |
| OK | KO | « le gateway écoute, le chemin public ne l'atteint pas » → Freebox / Synology |
| OK | OK | *silence* |

### 3.3 Le discriminant de traversée : une **famille** de réponses, jamais un code

La preuve du ticket mesure `HTTP 422` sur un POST vide. La lecture du code
explique pourquoi et impose de ne **pas** épingler ce chiffre :

`handle_webhook` déclare `Json(update): Json<TelegramUpdate>` en **extracteur**
(`routes.rs:445-449`). Axum exécute les extracteurs avant le corps du handler,
donc l'échec d'extraction précède le contrôle de secret (`routes.rs:463`) et même
la garde single-bot-mode (`routes.rs:450-455`). `TelegramUpdate.update_id` est un
`i64` requis (`telegram.rs:27-30`), donc un corps JSON sans lui est un
`JsonDataError` → 422. **Corollaire précieux : la sonde prouve la traversée sans
manipuler aucun secret et sans injecter d'update.**

Mais le code exact est une propriété de l'extracteur, pas un contrat : en Axum
0.8 la même route rend 415 sans `Content-Type` json, 400 sur JSON malformé, 401
si le secret échoue, 404 hors single-bot-mode. Épingler `422` ferait crier la
sonde le jour où l'un de ces détails bouge — une régression de la même famille
que celle qu'on répare.

**Ce que la sonde teste est donc la propriété, pas le chiffre :**

- **vivante** ⇐ réponse dans `{400, 401, 404, 415, 422}` — toutes sont des
  réponses qu'**Axum côté gateway** a produites, donc la chaîne a été traversée ;
- **cassée** ⇐ aucune réponse (DNS, TLS, connexion refusée, timeout) ou `502` /
  `504` — signature d'un reverse proxy qui ne joint pas son backend ;
- **rien vérifié** ⇐ tout le reste (2xx inattendu, 3xx, autre 5xx).

`503` est classé **rien vérifié** et non *cassé*, délibérément : il est ambigu
entre le shed-load du gateway (`routes.rs:468-473`) et un Synology en défaut, et
un avertissement qui se trompe de coupable est le défaut qu'on répare.

### 3.4 La course post-`restart` est le risque principal de faux positif

La cible tourne juste après `restart` dans la chaîne `deploy`. Un gateway qui n'a
pas encore levé `state.ready` rend `503` sur `/health` — et la sonde crierait sur
un système sain, reproduisant exactement le défaut du ticket dans l'autre sens.

**Le terminus local est donc sondé en poll borné** (défaut ~15 s, pas de ~1 s)
avant tout verdict `1`. Précédent maison direct :
`scripts/smoke-webhook-flood.sh` § « WHY SEQUENTIAL + PRECONDITIONED-IDLE »,
qui fait un bounded poll pour la même raison et *skip fail-open* plutôt que
d'émettre un faux signal de régression.

### 3.5 Ce que le Makefile fait des trois verdicts

`0` → **silence total** (AC3).
`1` → bloc `⚠ WARNING` nommant la vraie chaîne, le saut suspect, et la page AC4.
`2` → **une ligne `NOTE:`**, pas un `WARNING`.

La distinction `NOTE` / `WARNING` est ce qui réconcilie AC3 (« se termine **sans
avertissement** ») avec la doctrine mika#2205 (« un scan silencieusement inactif
se lit exactement comme un scan oisif »). Précédent littéral dans le même
fichier : `deploy-info` imprime `NOTE: could not reach origin (network/auth) —
skipping freshness check.` (`Makefile:126`) sans que ce soit un avertissement.

**Jamais fatal**, quelle que soit la sortie — posture fail-open de la chaîne
`deploy`, conservée de `check-ngrok` et partagée avec `smoke-webhook-flood.sh`.

### 3.6 Contrainte cross-repo — et c'est le point à ne pas rater

Le ticket écrit littéralement que « `make deploy` termine par **`make -C mika
check-ngrok`** ». `mika-platform/Makefile` est **hors de portée de ce worktree**
(lecture refusée par la permission-policy) donc non vérifiable ici, mais
`docs/plans/2026-05-20-fix-1210-…-plan.md:86` corrobore que le meta-repo ne
délègue *pas* à `make -C mika deploy` et appelle les sous-cibles nommément.

**Un renommage nu ferait échouer `make deploy` sur `No rule to make target
'check-ngrok'` — une régression pire que le défaut réparé.**

Résolution : la cible est renommée `check-webhook-chain` (AC1 « renommée et
réécrite ») **et** `check-ngrok` survit comme **alias de transition d'une ligne**
qui délègue à la nouvelle. L'alias ne contient aucune logique et n'émet rien de
lui-même : il ne nomme plus ngrok dans aucun message (AC2 porte sur le message,
pas sur le nom). Il porte un commentaire daté disant qu'il existe pour le seul
appelant cross-repo et qu'il disparaît quand celui-ci bascule. **Ticket de suivi
`mika-platform` à ouvrir** : basculer l'appelant, puis retirer l'alias.

---

## 4. Unités de travail

### U1 — `scripts/smoke-webhook-chain` (nouveau)

Bash, `set -uo pipefail`, en-tête commenté au format maison (la propriété,
l'incident fondateur, ce qu'il n'imprime jamais, les codes de sortie).

- **Résolution de l'URL, dans l'ordre, avec provenance** : `MIKA_WEBHOOK_CHAIN_URL`
  (surcharge propre à la sonde) → `MIKA_TELEGRAM_WEBHOOK_URL` (la variable que le
  gateway lit lui-même) → **aucune** ⇒ verdict `2` avec une ligne nommant les
  deux variables et la page AC4.
- **Terminus local** : poll borné sur `/health` (`MIKA_GATEWAY_PORT`, défaut
  `8080`), fenêtre `MIKA_WEBHOOK_CHAIN_LOCAL_TIMEOUT_SECS` (défaut `15`).
- **Traversée** : une requête, `Content-Type: application/json`, corps `{}`,
  timeout `MIKA_WEBHOOK_CHAIN_TIMEOUT_SECS` (défaut `10`). Classement par
  famille (§ 3.3).
- **Ce qu'il n'imprime jamais** : aucun secret, aucun corps de réponse. L'URL
  publique est imprimée (elle est publique par construction — c'est celle que
  Telegram appelle) ; le secret webhook n'est ni lu ni transmis.
- **Idempotent et sans effet de bord** : le corps `{}` est rejeté par l'extracteur
  avant toute logique métier (§ 3.3), donc aucun update n'entre dans le système.

### U2 — `Makefile`

- Renommer `check-ngrok` → `check-webhook-chain` ; réécrire la recette pour
  appeler U1 et router `0`/`1`/`2` vers silence/`WARNING`/`NOTE` (§ 3.5).
- Chaîne : `deploy: deploy-info build-dashboard build install restart check-webhook-chain`.
- Ajouter l'alias `check-ngrok` (§ 3.6), commenté et daté.
- `.PHONY` (`Makefile:4`) : ajouter `check-webhook-chain`, **conserver**
  `check-ngrok`.
- `help` : la ligne `##` de la nouvelle cible ne nomme pas ngrok.

### U3 — Garde de contrat côté gateway (`crates/mika-gateway/src/routes.rs`, `#[cfg(test)]`)

Un test qui construit `build_router(state(...))` — le helper existe déjà
(`routes.rs:3740-3765`) — et assert que `POST /webhook/telegram` **non
authentifié** rend un statut **dans `{400, 401, 404, 415, 422}`**, jamais un
`2xx`.

C'est la garde structurelle qui empêche la dérive silencieuse : elle épingle la
**famille** sur laquelle la sonde décide, pas un chiffre, et rougit si un futur
changement d'extracteur fait passer la route à `200`. Sans elle, la sonde
deviendrait un faux vert sans qu'aucun test ne bouge. Le doc-comment du test
nomme mika#2135 et `scripts/smoke-webhook-chain` comme son consommateur.

### U4 — `scripts/test-smoke-webhook-chain.sh` (nouveau) + câblage

Test de la sonde elle-même, patron `scripts/test-verify-no-secret-in-setenv.sh` /
`scripts/deploy-info-test.sh`. Serveur HTTP jetable local (ou `nc`) pour fabriquer
les réponses. **Câblé dans `make test`** à côté de `deploy-info-test.sh`
(`Makefile:145`), donc lancé en CI par le job `check`.

Cas couverts : chaque famille de statut → chaque verdict ; URL absente → `2` ;
hôte injoignable → `1` ; local KO → `1` avec le bon message ; `503` → `2` ;
**contrôle négatif** : chaîne saine → sortie vide et exit `0`.

### U5 — `docs/operator/local-webhook-topology.md` (nouveau) — AC4

Voisin de `docs/operator/agent-identity-reprovision.md`, même registre.

Contenu : le schéma **Telegram → Freebox (redirection de port) → Synology
(reverse proxy) → gentux:8080 (mika-gateway)** ; `setWebhook` est **manuel** et
n'est pas rejoué par le deploy (le gateway ne l'enregistre qu'en single-bot mode,
`main.rs:120-140`) ; le statut de ngrok — **repli non supporté**, jamais le chemin
nominal (périmètre explicite du ticket) ; comment **déclarer** la chaîne pour que
la sonde la vérifie (§ 3.2) ; comment refaire la preuve à la main (la traversée
du ticket, § 3.3) ; que lire quand la sonde crie.

**Pourquoi `docs/operator/` et pas `docs/deployment.md`** : ce dernier documente
le mode hébergé/conteneur et — décisif — il est dans la liste `DOCS` de
`scripts/sync-agent-docs.sh:13-23`, donc toute édition y **exige** de lancer le
script sous peine d'échec du job CI `docs-sync`. `docs/operator/**` n'y est pas.

### U6 — Pointeurs de découvrabilité (AC4)

AC4 demande « quelque part qu'un lecteur trouve ». Trois vecteurs, du plus utile
au moins :

1. **Le message d'échec de la sonde nomme le chemin du doc** — la découverte à
   l'instant où elle sert.
2. Une ligne dans `docs/deployment.md` § 1 renvoyant à U5 pour la topologie
   opérateur locale. **Si cette ligne est ajoutée, `bash
   scripts/sync-agent-docs.sh` est obligatoire dans le même commit** (job
   `docs-sync`).
3. `scripts/smoke-webhook-flood.sh:21` — corriger « FAIL-OPEN POSTURE (matches
   `check-ngrok`) » qui devient un renvoi mort.

---

## 5. Contrat de vérification

| # | Vérification | Comment |
|---|---|---|
| V1 | La sonde classe correctement chaque famille | `bash scripts/test-smoke-webhook-chain.sh` (U4) |
| V2 | Le contrat de statut du gateway ne peut plus dériver en silence | `cargo test -p mika-gateway` (U3) |
| V3 | **Contrôle négatif AC3** : chaîne saine ⇒ aucune sortie | cas dédié dans U4, + `make check-webhook-chain` sur gentux sain ⇒ zéro ligne |
| V4 | L'alias cross-repo ne casse rien | `make check-ngrok` s'exécute et se comporte comme `check-webhook-chain` |
| V5 | Aucune occurrence de ngrok dans un message ou une aide | `grep -n ngrok Makefile` ⇒ seulement le nom d'alias + son commentaire daté |
| V6 | La suite complète passe | `make test`, `make lint`, `make fmt` |
| V7 | Pas de dérive doc | `bash scripts/sync-agent-docs.sh` lancé si et seulement si `docs/deployment.md` a bougé |

**Vérification manuelle sur gentux, post-merge et post-deploy** (la seule qui
prouve la topologie réelle, les autres prouvant le code) :

1. `make deploy` sur checkout sain, chaîne en service ⇒ **aucun avertissement**
   (AC3).
2. Débrancher le dernier saut (arrêter le gateway) ⇒ `⚠ WARNING` nommant le
   gateway, **jamais ngrok** (AC1/AC2).
3. Relancer avec ngrok **arrêté** et la chaîne vivante ⇒ **silence** — c'est la
   régression fondatrice, et son contrôle direct.

---

## 6. Risques, et ce qu'on refuse

- **R1 — `make deploy` du meta-repo cassé par le renommage.** Le risque majeur.
  Fermé par l'alias (§ 3.6). Le retrait de l'alias est un ticket de suivi
  `mika-platform`, jamais fait à l'aveugle depuis ici.
- **R2 — faux positif post-`restart`.** Fermé par le poll borné (§ 3.4). C'est le
  défaut du ticket retourné ; l'accepter serait ne rien avoir appris.
- **R3 — une `NOTE:` permanente sur un poste où rien n'est déclaré.** Assumé et
  nommé : c'est la conséquence directe de D2, et c'est **honnête** — la sonde dit
  qu'elle n'a rien vérifié plutôt que de prétendre. Le remède est une déclaration
  unique, documentée par U5. La rendre silencieuse rouvrirait mika#2205.
- **R4 — la sonde dépend d'Internet.** Sur un deploy hors ligne : verdict `2`,
  `NOTE`, jamais `WARNING`. Par construction.

**Refus explicites :**

- **Ne pas** épingler `422` (§ 3.3) — fragile, et de la même famille de défaut
  que celui réparé.
- **Ne pas** poster un vrai update Telegram signé du secret : ça injecterait du
  trafic dans le système à chaque deploy et ferait circuler un secret dans un
  Makefile. Le `{}` prouve la traversée sans rien de tout ça.
- **Ne pas** classer `503` comme *cassé* (§ 3.3) — ambigu, et un avertissement qui
  se trompe de coupable est le défaut réparé.
- **Ne pas** rendre la cible fatale : la chaîne `deploy` est fail-open, et un
  deploy bloqué par une sonde réseau serait une régression opérationnelle.
- **Hors périmètre** (repris du ticket) : le per-customer mode et l'enregistrement
  webhook côté tenants cloud ; le statut de ngrok comme repli est **documenté**
  (U5), pas réhabilité.

---

## 7. Fichiers touchés

| Fichier | Nature |
|---|---|
| `scripts/smoke-webhook-chain` | **nouveau** — la sonde tri-états (U1) |
| `scripts/test-smoke-webhook-chain.sh` | **nouveau** — son test (U4) |
| `Makefile` | `check-ngrok` → `check-webhook-chain` + alias + `.PHONY` + chaîne `deploy` + câblage du test dans `test` (U2, U4) |
| `crates/mika-gateway/src/routes.rs` | **test seul** — garde de famille de statut (U3) |
| `docs/operator/local-webhook-topology.md` | **nouveau** — la topologie écrite (U5, AC4) |
| `docs/deployment.md` *(+ `crates/mika-agent/docs/deployment.md` via sync)* | pointeur, **optionnel** — si touché, sync obligatoire (U6) |
| `scripts/smoke-webhook-flood.sh` | commentaire ligne 21 — renvoi mort (U6) |

---

## Definition of Done

- [ ] `scripts/smoke-webhook-chain` existe, est exécutable, et rend `0`/`1`/`2`
      selon § 3.1–3.3.
- [ ] `Makefile` : `check-webhook-chain` remplace `check-ngrok` dans la chaîne
      `deploy` ; l'alias de transition est en place, commenté et daté ; `.PHONY`
      et `help` à jour.
- [ ] Aucun message émis par le Makefile ou la sonde ne nomme ngrok ni ne
      prescrit `ngrok http 8080`.
- [ ] U3 : le test de famille de statut sur `POST /webhook/telegram` passe et
      nomme mika#2135 + son consommateur.
- [ ] U4 : `scripts/test-smoke-webhook-chain.sh` couvre les trois verdicts **et**
      le contrôle négatif « chaîne saine ⇒ sortie vide », et il est câblé dans
      `make test`.
- [ ] U5 : `docs/operator/local-webhook-topology.md` écrit la topologie
      Freebox → Synology → gentux:8080 + `setWebhook` manuel + le statut de repli
      de ngrok + le geste de déclaration.
- [ ] Le message d'échec de la sonde nomme le chemin de U5.
- [ ] `make test`, `make lint`, `make fmt` passent.
- [ ] Si `docs/deployment.md` est touché : `bash scripts/sync-agent-docs.sh`
      lancé dans le même commit.
- [ ] Corps de PR : ticket de suivi `mika-platform` nommé (bascule de l'appelant
      `check-ngrok` puis retrait de l'alias), sous une ligne `Tracked in:`.

---

## Acceptance criteria

Transcrits du corps de mika#2135 § « Ce qui est demandé ».

- **AC1** — `check-ngrok` ne ment plus. Deux issues acceptables, à trancher au
  grooming : soit la cible est **supprimée** de la chaîne de `deploy` (si aucune
  topologie supportée n'utilise ngrok), soit elle est **renommée et réécrite**
  pour vérifier la chaîne réellement en service — par exemple que
  `telegram_webhook_url` est configurée et que le port du gateway écoute.
  *→ Tranché en D1/D2 : renommée `check-webhook-chain` et réécrite en sonde de
  chaîne à déclaration explicite (U1, U2).*
- **AC2** — Si la cible est conservée sous une forme quelconque, son message ne
  nomme plus ngrok comme le chemin, et ne prescrit plus `ngrok http 8080` comme
  remède. *→ V5, et l'alias de transition n'émet aucun message propre (§ 3.6).*
- **AC3** — Contrôle négatif : un `make deploy` sur un checkout sain se termine
  **sans avertissement** quand la chaîne webhook est en service. Une correction
  qui se contenterait de rendre l'avertissement inconditionnellement silencieux
  n'aurait rien réparé — elle aurait juste supprimé un signal au lieu de le rendre
  vrai. *→ V3 (cas de test dédié) + vérification manuelle 1 et 3 ; D1 refuse la
  suppression pour cette raison exacte.*
- **AC4** — La topologie retenue est **écrite** quelque part qu'un lecteur trouve :
  Freebox → Synology → gentux:8080, `setWebhook` manuel. Le coût de cet incident
  vient entièrement du fait qu'elle n'était nulle part sauf dans la mémoire de
  l'opérateur. *→ U5 + les trois vecteurs de découvrabilité de U6.*
