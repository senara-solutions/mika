---
issue: mika#2126
title: Une URL part nue vers Telegram - Plan
type: fix
scope_repo: mika
priority: p2-normal
date: 2026-09-01
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Une URL part nue vers Telegram - Plan

## Goal Capsule

**Objectif.** Une URL envoyée à un utilisateur par Telegram doit être cliquable et
mener où elle dit. Aujourd'hui l'agent écrit `**https://…**`, Telegram reçoit du
texte brut, absorbe les astérisques finales dans le lien, et l'utilisateur atterrit
sur un 404.

**Moyen.** Une fonction pure de nettoyage, appliquée au **point d'émission unique**
(`send_message_impl`), qui retire les marqueurs markdown **collés aux URL** — et
rien d'autre. La voie MarkdownV2 est écartée, avec sa raison écrite là où un futur
lecteur ira la chercher.

**Hiérarchie d'autorité.** AC du ticket > ce plan > jugement de l'implémenteur.

**Conditions d'arrêt.**
- S'arrêter si le correctif devient un désenrobeur markdown générique. Le périmètre
  est *les URL*, pas le rendu. Un nettoyage global toucherait des messages sains et
  violerait l'AC3.
- S'arrêter si le correctif touche une URL déjà saine. Un correctif qui réécrit les
  URL correctes n'a rien réparé : il a ajouté une seconde façon de les casser (AC3).
- S'arrêter si le nettoyage peut **empêcher un envoi**. Un message non envoyé est
  strictement pire qu'un lien cassé — c'est précisément le défaut de la voie 2.
- S'arrêter si l'implémentation glisse vers `parse_mode` sans repasser par le
  grooming. C'est la voie 2, explicitement écartée en KTD1.

**Profil d'exécution.** Une surface, un fichier : `crates/mika-gateway/src/telegram.rs`.
Séquentiel, trois unités courtes.

**Propriété de la queue.** PR sur `mika`, routée vers mika-qa.

## Product Contract

### Résumé

Ajouter une passe de nettoyage à l'émission Telegram : toute URL présente dans un
message sortant part **nue**, sans marqueur markdown accolé. Le message reste en
texte brut ; `parse_mode` n'est pas posé. La décision est consignée à l'endroit
exact où quelqu'un voudrait la défaire.

### Cadrage du problème

Le 2026-09-01 à 12:03, Al reçoit de sa Mika l'adresse du dépôt sous la forme
`**https://github.com/senara-solutions/mika**`. Telegram ne parse pas le gras : les
astérisques restent, et — mesuré dans le rapport — **elles sont absorbées dans le
lien cliquable**, qui pointe vers `…/mika**`. Vérifié à la source : `/mika` rend
**200**, `/mika**` rend **404**.

La cause est au `file:line`. `crates/mika-gateway/src/telegram.rs:313` :

```rust
#[derive(Debug, Serialize)]
struct SendMessagePayload {
    chat_id: i64,
    text: String,
}
```

Aucun champ `parse_mode`. Telegram reçoit du texte brut ; l'agent écrit du markdown
parce que c'est sa langue par défaut. **Les deux côtés sont cohérents avec eux-mêmes
et incompatibles entre eux** — et rien, entre les deux, ne vérifie que le message
tient dans le monde où il arrive.

**Troisième friction du même utilisateur en deux jours**, même racine : mika#2118
(une limite par conception racontée comme une panne), mika#2119 (une capacité
absente contournée par une invention), celui-ci (un message bien formé pour
l'émetteur, cassé pour le destinataire).

### Décisions clés

- **Voie 1 (texte brut + nettoyage à l'émission), pas voie 2 (MarkdownV2).** Régit
  KTD1, U2.
- **Garde structurelle, pas consigne de prompt.** Le nettoyage vit dans le code qui
  envoie, pas dans une instruction à l'agent. Régit KTD2.
- **Ancré sur l'URL, jamais sur le message.** Le périmètre est le voisinage
  immédiat d'un schéma `http://`/`https://`. Régit KTD3, AC3.
- **Le nettoyage ne peut jamais empêcher un envoi.** Régit Fire-Disposition.

### Exigences

- **R1** — Toute URL d'un message sortant vers Telegram est émise nue : aucun
  marqueur markdown collé avant ou après, aucun caractère de décoration absorbé
  dans l'URL. (AC1)
- **R2** — Le test porte sur **l'effet** : l'URL *telle qu'elle serait cliquée* est
  extractible et bien formée, pour gras, italique, backticks et lien markdown
  `[texte](url)`. (AC2)
- **R3** — Un message dont l'URL est déjà saine passe **octet pour octet
  inchangé**. Contrôle négatif obligatoire, dans la même suite que le positif. (AC3)
- **R4** — Le cas exact du rapport est une fixture figée, et la suite **échoue** si
  le nettoyage est retiré. Falsification démontrée et consignée dans la PR. (AC4)
- **R5** — Le choix voie 1 / voie 2 est écrit **avec sa raison**, à l'endroit où un
  lecteur qui voudrait « juste activer MarkdownV2 » passera. (AC5)
- **R6** — Le nettoyage s'applique au **point d'émission unique**, pas à un appelant
  particulier : tout appel présent ou futur en hérite.
- **R7** — Le nettoyage est **infaillible par construction** : il ne renvoie pas de
  `Result`, ne panique pas, et n'a aucun chemin qui interrompe l'envoi.

### Exemples d'acceptation

| Entrée (texte sortant) | Sortie attendue |
|---|---|
| `**https://github.com/senara-solutions/mika**` | `https://github.com/senara-solutions/mika` |
| `` `https://example.com/a` `` | `https://example.com/a` |
| `_https://example.com/a_` | `https://example.com/a` |
| `[le dépôt](https://example.com/a)` | `le dépôt : https://example.com/a` |
| `Va voir https://example.com/a` | *inchangé* (contrôle négatif) |
| `https://example.com/path_with_underscore_` | *inchangé* (contrôle négatif) |
| `https://example.com/~vincent` | *inchangé* (contrôle négatif) |
| `Voir https://example.com/a.` | *inchangé* — le point est de la ponctuation de phrase, pas du markdown |
| `…at console.getmika.ai.` | *inchangé* — nom d'hôte sans schéma, hors ancrage (cas réel : `OFFLINE_ERROR_MSG`) |
| `Bonjour Sonia 🌸` | *inchangé* — aucune URL, rien à faire |

### Sources

- `crates/mika-gateway/src/telegram.rs:313` — `SendMessagePayload`, sans `parse_mode`.
- `crates/mika-gateway/src/telegram.rs:368-380` — `send_message_impl`, le POST.
- `crates/mika-gateway/src/telegram.rs:701` — l'unique appelant de `send_message_impl`.
- `crates/mika-gateway/src/routes.rs:2217` — `OFFLINE_ERROR_MSG`, le seul message du
  gateway portant un nom d'hôte (sans schéma).
- Vérification de la cause : `/mika` → 200, `/mika**` → 404 (curl, samidarko, 2026-09-01).
- Même classe, même utilisateur : mika#2118, mika#2119.
- Source du rapport : Vincent via Al, 2026-09-01 12:03.

## Planning Contract

### Décisions techniques clés

- **KTD1 — Voie 1, et voici pourquoi la voie 2 est écartée.** MarkdownV2 exige
  d'échapper `_ * [ ] ( ) ~ \` > # + - = | { } . !` dans **tout** le texte, URL
  comprises. Un seul caractère non échappé fait **rejeter le message entier** par
  l'API Telegram. On remplacerait alors un lien cassé par un **message absent** —
  une régression franche : aujourd'hui l'utilisateur reçoit au moins le message.
  La voie 1 a la propriété qui décide : **le rendu ne peut pas casser ce qu'il ne
  parse pas**. Régit U2.

- **KTD2 — Garde à l'émission, pas consigne de prompt.** Le ticket propose déjà
  cette forme et elle est la bonne : une consigne de prompt cède, une garde
  structurelle tient. Elle vit dans `send_message_impl` — le point d'émission
  unique. **Vérifié :** `sendMessage` n'apparaît que dans `telegram.rs`, `api_url`
  n'a pas d'autre méthode d'envoi de texte, et `send_message_impl` a un seul
  appelant (`CustomerTelegramClient::send_message:701`) que les ~15 sites d'appel
  du gateway traversent tous. Un site posé demain en hérite sans rien faire.

- **KTD3 — L'ambiguïté est irréductible, et c'est *la* raison du bug.** `*`, `_` et
  `~` sont des caractères **légaux** dans une URL (`*` sous-délimiteur, `_` et `~`
  non réservés). Aucune grammaire d'URL ne peut donc distinguer `…/mika**` (URL +
  décoration) de `…/mika**` (URL qui finit par deux astérisques) — c'est
  exactement pourquoi l'autolieur de Telegram les avale. **Aucun algorithme ne peut
  être à la fois complet et sûr ici.** On choisit la sûreté :

  1. **Liens markdown.** `[label](url)` où `url` commence par `http://`/`https://`,
     sans espace dans l'url, sans crochet ni parenthèse imbriqués → réécrit en
     `label : url`. L'URL finit la séquence et est bornée par des espaces. Label
     vide ou identique à l'url → l'url seule. Toute autre forme est laissée
     intacte (conservatisme = sûreté AC3).
  2. **Décoration en bordure.** Sur chaque **jeton** délimité par des espaces qui
     contient un schéma, retirer en bordure :
     - `*` et `` ` `` — **inconditionnellement** (une URL réelle ne finit
       pratiquement jamais par une astérisque ou un backtick ; un backtick devrait
       de toute façon être percent-encodé) ;
     - `_` et `~` — **seulement appariés**, c'est-à-dire quand la même séquence
       borde les deux extrémités du jeton (`_url_`, `~~url~~`). Un `_` final isolé
       (`…/foo_`) est un caractère d'URL et **reste**.

  **Risque résiduel nommé :** `_texte https://url_` (italique soulignée autour d'une
  phrase entière) laisse un `_` collé — non apparié au niveau du jeton. Accepté :
  le cas observé et de loin le plus fréquent est `**url**`, couvert
  inconditionnellement. Élargir demanderait un vrai analyseur markdown, qui est
  précisément ce que « hors périmètre » écarte.

- **KTD4 — Pas de nouvelle dépendance.** `mika-gateway` n'a pas `regex`. Les deux
  passes sont des balayages linéaires ; une regex encoderait la même ambiguïté de
  façon moins lisible pour le prix d'une dépendance de plus sur ce crate.

- **KTD5 — Un seul fichier : la règle et sa raison au même endroit.**
  *(Tranché par mika-arch première passe, A1 — il écarte le module séparé que ce
  plan proposait d'abord.)* `strip_markdown_around_urls` et la note de décision
  vivent toutes deux dans `telegram.rs`, la fonction juste au-dessus de
  `send_message_impl`, la note sur `SendMessagePayload` (`:313`). Ce que ça achète :
  **l'endroit exact que modifierait quelqu'un venu ajouter `parse_mode`** est aussi
  celui où il trouve la règle et la raison. Un module séparé aurait rangé la raison
  ailleurs que le geste qu'elle doit retenir. Sert R5/AC5 — « écrite dans le
  module », et le module est `telegram.rs`.

- **KTD6 — Signature infaillible.** `fn strip_markdown_around_urls(text: &str) -> String`.
  Pas de `Result`, pas d'`unwrap`, pas d'indexation d'octets pouvant paniquer sur
  une frontière UTF-8 (itérer par `char_indices`, jamais par `text[i..j]` sur un
  index arbitraire). Un nettoyage qui peut échouer pourrait empêcher un envoi, ce
  que la Fire-Disposition interdit. Sert R7.

### Contraintes vérifiées (mesurées le 2026-09-01, non supposées)

- `send_message_impl` est le **seul** émetteur de texte vers Telegram.
  `grep -rn 'api\.telegram\.org'` → `telegram.rs` uniquement ; aucune autre méthode
  `send*` du Bot API n'est appelée (les champs `caption` du fichier sont **entrants**).
- Les ~15 sites d'appel de `send_message` dans `routes.rs` ont été **énumérés un par
  un**. Tous sont soit des chaînes statiques que nous écrivons, soit `routes.rs:1979`
  qui porte le texte de l'agent (via `format_outbound_text:1845`). **Aucune ne
  contient de marqueur markdown**, et une seule porte un nom d'hôte —
  `OFFLINE_ERROR_MSG` (`routes.rs:2217`), `console.getmika.ai`, **sans schéma**,
  donc hors de l'ancrage de la règle. L'AC3 tient donc gratuitement sur l'intégralité
  de nos propres messages, et ce cas devient un contrôle négatif d'U1.
- **Rectification d'un contrôle du ticket.** Le corps affirme que `parse_mode`
  « n'apparaît dans aucun fichier ». `grep -rl 'parse_mode'` rend en fait
  `crates/mika-cli/src/init.rs` — mais uniquement comme **sous-chaîne** de
  `parse_model_override`. La conclusion portante du ticket est confirmée : il n'y a
  aucun `parse_mode` Telegram dans le dépôt. Signalé ici plutôt que corrigé en
  silence ; **ne change aucun AC, aucun périmètre, aucune séquence**.
- `crates/mika-gateway/Cargo.toml` a `url` mais pas `regex` (cf. KTD4).

### Séquencement

U1 (fonction pure + tests) → U2 (câblage + note de décision) → U3 (démonstration de
falsification). U3 dépend de U1 et U2 tous deux posés.

## Fire-Disposition

Requis par le Fire-Disposition Gate (mika#1574), soulevé par mika-arch en première
passe. Ce plan porte **deux** surfaces qui « tirent », et elles n'ont pas la même
disposition parce qu'elles n'ont pas les mêmes conséquences.

**Rectification de cadrage, dite plutôt que tue.** Ce plan ne livre pas un détecteur
au sens habituel du gate — rien ici ne balaye un arbre existant ni ne fait échouer
un build sur des données préexistantes. Il livre un **transformateur** en chemin
chaud, plus une suite de tests. Le gate reste la bonne question à poser : *que se
passe-t-il quand ça tire ?* Voici les deux réponses, contre le schéma canonique
**(a) exception nommée en liste blanche / (b) posé-désactivé / (c) halte-et-remontée**.

**1. Le nettoyage en production → aucune des trois : transformer-et-observer.**
- **Aucune halte n'est admissible.** Interrompre un envoi parce que le texte semble
  décoré reproduirait exactement le défaut pour lequel la voie 2 a été écartée : un
  lien cassé remplacé par un message absent. `strip_markdown_around_urls` ne peut
  donc pas échouer — KTD6 le rend vrai par signature, pas par intention.
- **Aucune liste blanche.** Une exception par expéditeur ou par chat rouvrirait le
  trou pour l'utilisateur exact qui l'a signalé.
- **Ce qui est exigé à la place : l'observabilité.** Quand le texte émis diffère du
  texte reçu, une ligne `debug!` structurée le dit — `chat_id`, longueur avant,
  longueur après, nombre d'URL touchées. **Jamais le contenu du message** (donnée
  utilisateur). Cette trace n'est pas décorative : elle est le seul moyen de
  distinguer, après déploiement, « la règle ne tire plus parce que l'agent a cessé
  de décorer » de « la règle ne tire plus parce qu'elle a cessé de matcher ». Sans
  compteur, ces deux mondes sont identiques vus depuis la passerelle.

**2. Les tests d'U1/U3 → (c) halte-et-remontée.**
Un test rouge bloque la PR, et c'est le résultat voulu, pas un faux positif : les
fixtures sont figées sur un cas réel mesuré (AC4). **Aucune liste blanche, aucun
`#[ignore]`.** Si un test rougit après un changement de règle, la règle est
re-tranchée en grooming — elle n'est pas contournée dans la suite.

**Pourquoi pas (b) posé-désactivé.** Le correctif est étroit, ancré sur un schéma
d'URL, et sa surface de faux positif a été **énumérée** puis fermée : aucun message
du gateway ne porte de markdown, et le seul nom d'hôte présent est sans schéma. Un
déploiement désactivé n'achèterait aucune information et laisserait le 404 en place
chez l'utilisateur qui l'a signalé.

## Implementation Units

### U1. La fonction pure de nettoyage, et ses deux contrôles

- **Fichier :** `crates/mika-gateway/src/telegram.rs` (fonction placée juste
  au-dessus de `send_message_impl`, tests dans le `mod tests` existant).
- **Approche.** `fn strip_markdown_around_urls(text: &str) -> String`, implémentant
  les deux passes de KTD3 dans cet ordre : liens markdown d'abord, décoration en
  bordure ensuite. Signature infaillible par KTD6. Doc de fonction portant la règle,
  son ancrage sur le schéma, et le risque résiduel nommé.
- **Vérification.** `cargo test -p mika-gateway`. La suite contient, **dans le même
  module** (une sonde porte ses deux contrôles ou elle ne prouve rien) :
  - *Positifs* : chaque ligne du tableau des exemples d'acceptation qui transforme —
    gras, italique, backticks, lien markdown. Chaque test assert sur **l'URL
    extraite**, pas sur la forme du message (R2/AC2).
  - *Négatifs* : chaque ligne qui doit rester inchangée, en `assert_eq!(out, input)`
    — URL nue, URL finissant par `_`, URL avec `~`, URL suivie d'un point de phrase,
    nom d'hôte sans schéma (`console.getmika.ai`, cas réel), message sans URL.
  - *Fixture figée* : `**https://github.com/senara-solutions/mika**` →
    `https://github.com/senara-solutions/mika`, test nommé d'après le ticket (R4).
- **Couvre :** R1, R2, R3, R4 (partie test), R7.

### U2. Le câblage au point d'émission, et la décision écrite

- **Fichier :** `crates/mika-gateway/src/telegram.rs`.
- **Approche.** Dans `send_message_impl`, construire le payload à partir de
  `strip_markdown_around_urls(text)` plutôt que de `text`. Un seul endroit. Émettre
  la trace `debug!` de la Fire-Disposition quand la sortie diffère de l'entrée —
  métriques seulement, jamais le contenu. Ajouter au-dessus de `SendMessagePayload`
  (`:313`) une note nommant le choix : pas de `parse_mode`, pourquoi MarkdownV2 a
  été pesé et écarté (un caractère non échappé → message entier rejeté → lien cassé
  remplacé par message absent), et un renvoi vers `strip_markdown_around_urls` pour
  la règle.
- **Vérification.** `cargo test -p mika-gateway`. La note existe et nomme les deux
  voies — vérifiable en lisant le diff.
- **Couvre :** R1, R5, R6, et l'observabilité de la Fire-Disposition.

### U3. La preuve que le test n'est pas vide

- **Fichiers :** aucun durablement — manipulation temporaire, restaurée.
- **Approche.** Rendre `strip_markdown_around_urls` transparente (retour de l'entrée
  telle quelle), lancer `cargo test -p mika-gateway`, **capturer la sortie rouge**,
  restaurer, relancer, capturer le vert. Coller les deux dans le corps de la PR.
- **Vérification.** Le corps de PR contient les deux sorties. Un test qui ne rougit
  pas quand on retire ce qu'il teste ne teste rien.
- **Couvre :** R4.

## Suites (hors périmètre de ce ticket)

Le corps du ticket note que « l'audit vaut la peine d'être fait » pour les autres
canaux (dashboard, CLI, e-mail). Il n'est **pas** dans ce périmètre. S'il est
souhaité, il fait un ticket par canal, avec sa propre preuve d'effet — pas un
élargissement de celui-ci.

## Verification Contract

- `cargo test -p mika-gateway` — vert, incluant les contrôles positifs **et**
  négatifs d'U1.
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- **Preuve de non-vacuité (U3)** : sortie rouge et sortie verte, toutes deux
  consignées dans le corps de la PR.
- **Post-déploiement, opérateur :** renvoyer le message exact du rapport à un chat
  Telegram réel et cliquer le lien. C'est le seul contrôle qui mesure le monde et
  non notre modèle du monde ; le test unitaire mesure notre modèle. La trace
  `debug!` de la Fire-Disposition dit, elle, si la règle tire encore.

## Acceptance criteria

Transcrits du ticket, avec l'unité qui satisfait chacun.

- [x] **AC1** — Toute URL sortante est émise nue, sans marqueur collé ni caractère
  de décoration absorbé. → **U1** (règle) + **U2** (application au point unique).
- [x] **AC2** — Test d'effet, pas de forme : l'URL *telle qu'elle serait cliquée*
  est extractible et bien formée, pour gras, italique, lien markdown, backticks.
  → **U1**, assertions sur l'URL extraite.
- [x] **AC3** — Contrôle négatif obligatoire : une URL saine passe inchangée.
  → **U1**, `assert_eq!(out, input)` sur URL nue, `_` final, `~`, ponctuation de
  phrase, nom d'hôte sans schéma. Renforcé par la contrainte vérifiée qu'aucune des
  ~15 chaînes sortantes du gateway ne porte de markdown.
- [x] **AC4** — Fixture figée du cas rapporté, et la suite échoue si le nettoyage
  est retiré, démontré et consigné dans la PR. → **U1** (fixture) + **U3** (preuve).
- [x] **AC5** — La décision voie 1 / voie 2 est écrite avec sa raison, trouvable par
  qui voudrait « juste activer MarkdownV2 ». → **U2**, note sur `SendMessagePayload`
  (l'endroit qu'il modifierait), la règle immédiatement au-dessus de
  `send_message_impl` dans le même fichier (KTD5).

## Definition of Done

**Global.**
- R1–R7 satisfaits, chacun tracé à une unité posée.
- Aucune URL sortante ne porte de marqueur markdown ; aucune URL saine n'est
  réécrite.
- `parse_mode` reste absent, et la raison est écrite — pas seulement appliquée.
- Le nettoyage est au point d'émission unique : aucun site d'appel n'est modifié
  individuellement, et un site futur en hérite.
- Le nettoyage ne peut pas empêcher un envoi — vérifiable par l'absence de `Result`,
  d'`unwrap`, d'`expect` et d'indexation d'octets brute dans la fonction.
- Aucune dépendance ni aucun module ajoutés à `mika-gateway` (KTD4, KTD5).
- La trace `debug!` existe et ne journalise **aucun contenu de message**.
- La preuve de falsification d'U3 est dans le corps de la PR.

**Par unité.** La Vérification de chaque unité passe.
