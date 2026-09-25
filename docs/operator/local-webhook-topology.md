# La chaîne de webhook entrante de la Mika locale

> **Cette page existe parce que son absence a coûté une fausse alerte.** Le
> 2026-09-01, `make deploy` a terminé sur `⚠ WARNING: ngrok is not running!
> Telegram webhooks will not reach the gateway.` — trois phrases fausses, émises
> pendant que la chaîne réelle acheminait des webhooks GitHub et répondait `200`.
> L'avertissement a produit une fausse alerte publiée dans un rapport
> d'orchestration, et il a fallu que l'opérateur corrige la topologie **de
> mémoire** pour que la vérification soit refaite à la source. Le coût entier de
> l'incident vient de là : la topologie n'était écrite nulle part (mika#2135).

---

## 1. La chaîne, dans l'ordre

```
Telegram (api.telegram.org)
    │  POST vers l'URL publique, avec l'en-tête X-Telegram-Bot-Api-Secret-Token
    ▼
Freebox — redirection de port  (443 public → Synology)
    │
    ▼
Synology — reverse proxy  (termine TLS pour webhook.dupont.tech)
    │
    ▼
gentux:8080 — mika-gateway  (POST /webhook/telegram/<customer_id>)
    │
    ▼
l'agent du tenant (mika-spirit, en local)
```

**Quatre sauts, et un seul vit dans ce dépôt** — le dernier. C'est toute la
difficulté du diagnostic : vu de gentux, un gateway qui écoute parfaitement est
indistinguable d'une chaîne qui l'atteint, et une chaîne morte est
indistinguable d'une chaîne vivante jusqu'à ce que quelque chose la traverse
réellement.

**ngrok n'est pas dans cette chaîne, et ne l'est plus depuis longtemps.** Voir
§ 6 pour son statut exact.

---

## 2. `setWebhook` est posé **à la main**, et le deploy ne le rejoue pas

C'est le fait le plus contre-intuitif de la page, et il est structurel plutôt que
conventionnel : **le gateway n'enregistre le webhook auprès de Telegram qu'en
mode single-bot.** Dans `crates/mika-gateway/src/main.rs`, l'appel
`tg.set_webhook(...)` est à l'intérieur d'un `if single_bot_mode`. Sur la branche
`else` — celle de gentux — il n'y a qu'une ligne de journal :

```
global Telegram client built for outbound delivery — inbound webhook registration skipped (per-customer mode)
```

Le client Telegram *est* construit (la livraison sortante en a besoin,
mika#1590) ; c'est **l'enregistrement entrant** qui est sauté.

**Conséquences pour l'opérateur, les deux :**

1. **Redémarrer le gateway ne répare jamais un `setWebhook` perdu.** Aucun
   `make deploy`, aucun `rc-service mika-gateway restart` ne repose l'URL chez
   Telegram. Si l'URL enregistrée chez Telegram est fausse ou absente, elle le
   reste jusqu'à un geste manuel.
2. **`MIKA_TELEGRAM_WEBHOOK_URL` n'est pas obligatoire sur ce poste.**
   `GatewaySettings::load` ne l'exige que sous `MIKA_TELEGRAM_SINGLE_BOT_MODE`
   (`crates/mika-gateway/src/settings.rs`). En mode par client elle est
   facultative — le gateway démarre sans elle. Sur gentux elle est posée quand
   même, et le journal de démarrage l'affiche :
   `telegram_webhook_url: Some("https://webhook.dupont.tech/webhook/telegram")`.
   C'est une déclaration, pas un mécanisme : rien ne la consomme en mode par
   client, sauf la sonde du § 4.

Vérifier ce que Telegram croit, pour un bot donné :

```bash
curl -s "https://api.telegram.org/bot<TOKEN>/getWebhookInfo" | jq
```

Les champs qui parlent : `url`, `pending_update_count`, `last_error_date`,
`last_error_message`. Un `last_error_message` non vide est le signal le plus
direct que la chaîne est cassée **vue de Telegram**, ce qu'aucune sonde locale ne
peut voir.

---

## 3. Deux routes entrantes, et la sonde ne traverse qu'une seule

Le gateway expose **deux** chemins Telegram
(`crates/mika-gateway/src/routes.rs`) :

| route | mode | authentification |
|---|---|---|
| `POST /webhook/telegram` | single-bot | secret global ; **404** hors single-bot mode |
| `POST /webhook/telegram/{customer_id}` | par client | `webhook_secret` du client en base ; **401 unifié** sur tout échec (anti-énumération) |

**Sur gentux, le trafic réel d'un tenant arrive sur la seconde**, avec son
`customer_id`. L'URL déclarée dans `MIKA_TELEGRAM_WEBHOOK_URL` désigne la
première.

**Ce que ça veut dire pour la lecture d'un verdict vert, et il faut le savoir
avant de s'en servir :** la sonde du § 4 prouve que la chaîne Freebox → Synology
→ gateway est traversée et que la pile Axum du gateway répond. Elle **ne prouve
pas** qu'un tenant donné est correctement câblé chez Telegram — ça, seul
`getWebhookInfo` (§ 2) le dit. Les deux mesures sont de provenances différentes
et aucune ne remplace l'autre.

---

## 4. Comment **déclarer** la chaîne pour que le deploy la vérifie

Depuis mika#2135, `make deploy` se termine par `check-webhook-chain`, qui appelle
`scripts/smoke-webhook-chain`. La cible remplace `check-ngrok`.

**La chaîne est déclarée, jamais dérivée.** Il n'existe aucun fichier que `make`
pourrait lire pour apprendre l'URL publique : le gateway charge sa configuration
par `dotenvy::dotenv()` depuis son **CWD**, et sous OpenRC ce CWD est `/` — la
configuration vient de l'EnvironmentFile du service. Trois dérivations ont été
refusées, chacune parce qu'elle rendrait une réponse *fausse* plutôt qu'absente :
lire un `.env` du dépôt (reconstruirait une cascade dont l'ordre réel est inconnu
ici — une provenance fausse est strictement pire qu'aucune, doctrine mika#2293) ;
parser le journal de démarrage (un rendu `Debug` non contractuel, à un chemin
lui-même configurable) ; deviner depuis le nom d'hôte (ne discrimine rien).

Donc : **l'URL vient d'une déclaration explicite, et son absence vaut « rien
vérifié » — jamais « tout va bien ».**

| variable | rôle | défaut |
|---|---|---|
| `MIKA_WEBHOOK_CHAIN_URL` | URL publique, surcharge propre à la sonde | — |
| `MIKA_TELEGRAM_WEBHOOK_URL` | URL publique, la variable que le gateway lit lui-même | — |
| `MIKA_GATEWAY_PORT` | port local du gateway | `8080` |
| `MIKA_WEBHOOK_CHAIN_LOCAL_TIMEOUT_SECS` | poll borné de `/health` après `restart` | `15` |
| `MIKA_WEBHOOK_CHAIN_TIMEOUT_SECS` | timeout de la traversée | `10` |

Le geste, une fois, dans l'**EnvironmentFile du service** (jamais dans un shell
interactif) :

```
MIKA_TELEGRAM_WEBHOOK_URL=https://webhook.dupont.tech/webhook/telegram
```

Rien n'est déclaré ⇒ `make deploy` imprime **une ligne `NOTE:`** et sort en `2`.
Ce n'est pas un avertissement et ce n'est pas un succès : c'est la sonde qui dit
qu'elle n'a rien vérifié. La rendre silencieuse serait rouvrir mika#2205 — *un
scan silencieusement inactif se lit exactement comme un scan oisif*.

### Ce que la sonde envoie, et pourquoi c'est inoffensif

Un `POST` avec `Content-Type: application/json` et le corps `{}`. Le handler
déclare `Json(update): Json<TelegramUpdate>` en **extracteur**, et Axum exécute
les extracteurs *avant* le corps du handler — donc l'échec d'extraction précède
le contrôle de secret **et** la garde single-bot-mode. `TelegramUpdate.update_id`
est un `i64` requis, donc `{}` ne peut pas se désérialiser.

**Corollaire qui vaut d'être gardé : la traversée est prouvée sans toucher à
aucun secret et sans injecter le moindre update dans le système.** Rien n'entre
dans la file.

### Les trois verdicts

| exit | ce que `make deploy` imprime | ce que ça veut dire |
|---|---|---|
| `0` | **rien du tout** | la chaîne est vivante — une requête sur l'URL publique est revenue avec une réponse applicative du gateway |
| `1` | bloc `⚠ WARNING` | la chaîne est cassée : le gateway local ne répond pas, ou la requête publique ne l'atteint pas |
| `2` | une ligne `NOTE:` | **rien n'a été vérifié** : aucune URL déclarée, pas de réseau, ou réponse inclassable |

**Aucune sortie n'est fatale.** La chaîne `deploy` est fail-open : un deploy
bloqué par une sonde réseau serait une régression opérationnelle.

### Ce qui est jugé, c'est une **famille** de réponses, jamais un code

L'incident fondateur a mesuré un `HTTP 422`, et épingler `422` serait une
régression de la famille même qu'on répare : la même route rend `415` sans
`Content-Type` json, `400` sur JSON malformé, `401` quand le contrôle de secret
échoue, `404` hors single-bot mode. **Toutes ont été produites par la pile Axum
du gateway**, donc toutes prouvent la traversée.

| observé | verdict | lecture |
|---|---|---|
| `400` `401` `404` `415` `422` | `0` | le gateway a répondu — la chaîne a été traversée |
| aucune réponse (DNS, TLS, connexion refusée, timeout) | `1` | le chemin public n'atteint pas le gateway |
| `502` `504` | `1` | un reverse proxy qui répond pour un backend qu'il ne joint pas |
| `503` | `2` | **ambigu** — shed-load du gateway *ou* Synology en défaut. Un avertissement qui se trompe de coupable est le défaut qu'on répare |
| tout le reste (2xx, 3xx, autre 5xx) | `2` | inclassable |

La famille est épinglée côté gateway par un test `#[cfg(test)]` de
`crates/mika-gateway/src/routes.rs`, pour qu'elle ne puisse pas dériver en
silence et transformer la sonde en faux vert sans qu'aucun test ne bouge.

---

## 5. Que lire quand la sonde crie

### `⚠ WARNING … broken at its last hop` — le gateway

Le message distingue deux états qui envoient à deux endroits différents :

- *« not answering on localhost:8080 (nothing listening) »* — rien n'écoute.
- *« answers … but is not ready (GET /health → 503 after 15s) »* — le processus
  est là et ne lève pas `state.ready` (`/health` est non authentifié et rend
  `200` si `state.ready` est levé **et** que `SELECT 1` passe — donc un Postgres
  injoignable produit exactement ce message).

```bash
sudo rc-service mika-gateway status
tail -n 100 /var/log/mika/gateway.log
```

### `⚠ WARNING … the public path does not reach it` — les sauts d'en face

Le gateway est sain, établi par la sonde locale **avant** ce verdict. Les
suspects sont, de l'extérieur vers l'intérieur : l'enregistrement chez Telegram
(§ 2), le DNS de `webhook.dupont.tech`, la redirection de port de la Freebox, le
reverse proxy du Synology (certificat expiré, backend pointant sur une mauvaise
adresse). Un `502` / `504` désigne le dernier tronçon : Synology répond, son
backend non.

### `NOTE:` — rien n'a été vérifié

Ce n'est pas une panne. Soit rien n'est déclaré (§ 4), soit le deploy est hors
ligne, soit la réponse est inclassable. **Ne pas la traiter comme un
avertissement, et surtout ne pas la faire taire** : c'est la seule sortie honnête
d'une sonde qui n'a rien pu joindre.

### Halte — la sonde est muette alors que les webhooks n'arrivent pas

`0` ne prouve que le § 3 : la chaîne jusqu'à la pile Axum. **Ne pas élargir la
sonde par réflexe.** Établir d'abord ce que Telegram croit
(`getWebhookInfo`, § 2) et sur quelle route le tenant est censé arriver (§ 3).
Un `last_error_message` non vide chez Telegram avec une sonde verte est
parfaitement cohérent : la chaîne est vivante et l'enregistrement est faux.

---

## 6. Refaire la preuve à la main

C'est la mesure du 2026-09-01, et c'est elle qui a réfuté `check-ngrok`. Elle
vaut d'être sue parce qu'elle **traverse** au lieu de lire un statut.

```bash
# 1. Qui écoute sur gentux
ss -ltnp | grep 8080

# 2. Ce que le gateway déclare (début du journal de démarrage)
grep -m1 'starting mika-gateway' /var/log/mika/gateway.log

# 3. La chaîne, traversée de bout en bout — depuis n'importe où
curl -si -X POST https://webhook.dupont.tech/webhook/telegram \
     -H 'Content-Type: application/json' --data '{}' | head -1

# 4. Le MÊME statut doit ressortir dans le journal LOCAL — c'est ça, la preuve
tail -n 20 /var/log/mika/gateway.log | grep '/webhook/telegram'
```

**L'étape 4 est la seule qui prouve quoi que ce soit.** Un `422` à l'étape 3 sans
ligne correspondante à l'étape 4 signifie qu'un intermédiaire a répondu à la
place du gateway. C'est le croisement des deux qui fait la preuve, pas le code de
statut tout seul.

Le 2026-09-01, avec **ngrok arrêté**, l'étape 3 a rendu `HTTP 422` et l'étape 4
la ligne correspondante (`{"status":422,"method":"POST","path":"/webhook/telegram","latency":"38.372µs"}`).
Le même jour, des webhooks GitHub `issues/edited` ont été reçus et répondus `200`.
La chaîne était vivante ; l'avertissement était faux.

---

## 7. Le statut de ngrok : **repli non supporté**

Pour éviter qu'un futur lecteur le réhabilite par inférence — et parce que le
ticket met explicitement sa réhabilitation hors périmètre :

- **ngrok n'est pas le chemin nominal.** Il ne l'est plus, et ce document est le
  seul endroit où son statut est écrit.
- **Aucun code ne le lit.** Au grooming de mika#2135, un balayage de `*.rs`,
  `*.sh`, `*.toml`, `*.yml`, `*.md`, `Makefile` et `.env.example` n'a rendu que
  l'ancienne recette elle-même, une référence *textuelle* dans un commentaire de
  `scripts/smoke-webhook-flood.sh`, et des mentions historiques dans
  `docs/plans/**` et `docs/solutions/**`. La cible était un vestige entier, pas
  une branche d'une topologie supportée.
- **`check-ngrok` n'a jamais vérifié la chaîne, même quand il passait.** Il
  sondait `http://localhost:4040/api/tunnels` — l'API locale d'un ngrok. Un ngrok
  vivant prouve qu'un ngrok est vivant, jamais que Telegram atteint quoi que ce
  soit. C'était une vérification de *présence d'outil* déguisée en vérification
  de chaîne.
- Un tunnel ngrok reste utilisable pour du développement ponctuel, à la main.
  S'il est utilisé, `MIKA_WEBHOOK_CHAIN_URL` est ce qui le déclare à la sonde
  pour le temps de la session — et c'est tout ce que le dépôt en sait.

`make check-ngrok` survit comme **alias de transition d'une ligne** vers
`check-webhook-chain`, uniquement pour l'appelant cross-repo
(`mika-platform/Makefile`, qui appelle les sous-cibles nommément). L'alias ne
contient aucune logique et n'émet aucun message propre. Il disparaît quand cet
appelant bascule.

---

## Voir aussi

- `scripts/smoke-webhook-chain` — la sonde ; son en-tête porte le raisonnement
  complet.
- `scripts/test-smoke-webhook-chain.sh` — son test, câblé dans `make test`.
- `crates/mika-gateway/CLAUDE.md` — routage entrant, registre des clients, DLQ.
- `docs/deployment.md` — le mode **hébergé / conteneur**, qui est une autre
  topologie. Cette page-ci décrit le poste local.
