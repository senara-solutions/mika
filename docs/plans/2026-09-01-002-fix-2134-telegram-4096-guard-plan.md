---
issue: mika#2134
title: Aligner la garde de send_message sur la limite Telegram (4096) - Plan
type: fix
scope_repo: mika
priority: p2-normal
date: 2026-09-01
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Aligner la garde de `send_message` sur la limite Telegram (4096) - Plan

## Goal Capsule

**Objectif.** Fermer la fenêtre 4096–10 000 où l'outil `send_message` accepte un
texte que le transport Telegram refusera. Un fragment de 5000 caractères doit
mourir **dans l'outil, avec une raison exploitable**, pas au transport.

**Moyens.** Une constante unique nommée d'après le transport
(`mika_common::telegram::MAX_TEXT_UTF16_UNITS = 4096`), la garde de
`send_message` portée sur elle plutôt que sur `MAX_INPUT_LEN`, un message de
refus qui nomme Telegram et dit qui découpe, et une garde miroir au gateway pour
que le texte **préfixé** soit mesuré là où le préfixe est ajouté.

**Hiérarchie d'autorité.** ACs du ticket > ce plan > jugement de l'implémenteur.

**Conditions d'arrêt.**
- S'arrêter si le correctif refuse des messages qui passaient et qui arrivaient.
  AC3 est un contrôle négatif obligatoire : 4095 doit passer. Fermer la fenêtre
  par le bas n'est pas une réparation.
- S'arrêter si la garde mesure des **octets**. `text.len()` est en octets ; la
  limite Telegram est en unités UTF-16. Mesurer en octets refuserait ~3400
  caractères de français accentué et casserait AC3 en pratique.
- S'arrêter si le correctif implémente la découpe automatique. Ce ticket décide
  **à qui** elle appartient (AC5) et rend le refus exploitable ; il ne la code pas.

**Profil d'exécution.** Trois surfaces, un dépôt : `mika-common` (constante),
`mika-agent/src/tools/send_message.rs` (garde + message + description d'outil),
`mika-gateway/src/routes.rs` (garde miroir post-préfixe). Séquentiel.

**Tail ownership.** PR sur `mika`, routée vers mika-qa.

## Product Contract

### Résumé

`send_message` refuse aujourd'hui au-delà de `MAX_INPUT_LEN = 10_000`. Telegram
coupe à 4096. Entre les deux, l'outil dit oui et le transport dit non. On aligne
la garde sur le transport, on nomme le transport dans le refus, et on ajoute au
gateway la même mesure — appliquée au texte tel qu'il partira, préfixe compris.

### Cadrage du problème

Mesuré dans le code à `origin/main = 50d969a7` :

| couche | fichier:ligne | état constaté |
|---|---|---|
| garde de l'outil | `crates/mika-agent/src/tools/send_message.rs:46` | `text.len() > MAX_INPUT_LEN` → refus à 10 000 **octets** |
| constante | `crates/mika-agent/src/tools/mod.rs:80` | `MAX_INPUT_LEN = 10_000`, documentée « control fields (path, name, query) » |
| transport | `crates/mika-gateway/src/routes.rs:1977` | `// Send to Telegram (no message splitting — send as-is)` |
| échec transport | `crates/mika-gateway/src/routes.rs:2016` | `warn!` + `502 BAD_GATEWAY` |
| remontée agent | `crates/mika-agent/src/messaging.rs:142` | `bail!("gateway /send returned {status}")` |
| vu du modèle | `crates/mika-agent/src/tools/send_message.rs:81` | `ToolOutput::error("Message delivery failed: {reason}")` |

Le ticket a raison sur le point qui compte : **rien n'est avalé.** Le modèle a
reçu un refus chiffré. La plomberie fautive est le désalignement des deux
limites, pas un silence.

Deux faits que le ticket ne nomme pas et qui changent le correctif :

1. **`MAX_INPUT_LEN` est une garde d'octets, la limite Telegram est en unités
   UTF-16.** `str::len()` en Rust rend des octets. « Le voici en entier 👆 »
   coûte 4 octets pour l'emoji et 2 unités UTF-16. Une garde `bytes > 4096`
   refuserait ~3400 caractères de français accentué — AC3 échouerait sur du texte
   réel même en ayant l'air correct sur du `"a".repeat(4095)`. La mesure doit être
   `s.encode_utf16().count()`, ce que Telegram compte effectivement.

2. **Le gateway rallonge le texte après que l'agent l'a mesuré.**
   `format_outbound_text` (`routes.rs:1845`) préfixe `[<agent_name>] ` pour tout
   agent **autre que** `DEFAULT_AGENT` (`"mika"`). Le tenant d'Al est en
   agent par défaut : pas de préfixe, la fenêtre décrite dans le ticket est bien
   la seule cause pour ce cas. Mais pour un agent nommé (`work-mika`), jusqu'à
   35 unités s'ajoutent **après** la garde de l'agent. Une garde posée uniquement
   côté agent laisse donc une fenêtre résiduelle 4061–4096 pour ces agents-là.
   C'est pourquoi la garde miroir appartient au gateway : c'est la couche qui
   connaît le préfixe.

3. **L'ordre actuel mesure le mauvais texte.** La garde (`:46`) s'applique au
   `text` brut, avant `strip_internal_tags` (`:52`). Le texte qui part est
   `cleaned`, qui est toujours ≤ `text`. Mesurer le brut refuse des messages
   livrables. La garde doit passer après le nettoyage — donc aussi **avant** la
   persistance en base (`:57`), pour ne pas écrire en historique un message
   qu'on refuse d'envoyer.

### Décision AC5 — la découpe appartient à l'agent

**Elle reste à l'agent. Le gateway continue de déclarer `send as-is`.** Raison
technique, pas préférence : à `routes.rs:1984`, le gateway insère **une** ligne
`outbound_messages (telegram_message_id, chat_id, agent_name)` par envoi, et
c'est cette table qui porte le routage des réponses (`routes.rs:2173`). Un
gateway qui découpe produit N `message_id` pour une ligne — soit le routage
`reply_to_message` casse, soit il faut réécrire le modèle de données. Le gateway
est un transport ; il n'a pas la sémantique du message (où couper sans casser un
bloc de code ou une phrase).

Cette décision cesse d'être implicite : elle est écrite (a) dans le commentaire
du gateway, (b) dans la doc de la constante partagée, (c) **dans le message de
refus lui-même**, qui dit au modèle que la découpe lui revient.

### Hors périmètre

- **La découpe automatique.** Non implémentée ici. Le refus la rend possible et
  explicite ; il ne la fait pas.
- **Le second étage comportemental** (« Le voici en entier 👆 » après un refus
  chiffré ; Partie 2/4 envoyée sans dire que la 1/4 a échoué). Porté sur
  mika#2118 comme le ticket le demande.
- **Le markdown non parsé** — mika#2126.
- **Les ~15 autres appelants de `MessageSender::send`** (`server/handlers.rs:1385`
  et `:1531`, `task_engine/dispatcher.rs:160/746/1036`, `ci_failure_handler.rs:596`,
  `ci_success_handler.rs:461/553`, `verdict_handler.rs:2275`, `agent_loop/mod.rs:2600/4617/4669`,
  `run_team.rs:121/152`). Aucun ne passe par la garde de l'outil. La garde miroir
  du gateway (phase 3) les couvre **toutes** avec une raison nommée au lieu d'un
  502 opaque — c'est précisément ce qui justifie de la poser ici plutôt que de
  s'en remettre à quinze retouches d'appelants. Leur traitement individuel
  (découpe, troncature choisie) reste hors périmètre.
- `teams/notification.rs:11` tronque déjà à 4000 avec la bonne raison. On n'y
  touche pas : c'est déjà sous la limite. La faire pointer sur la nouvelle
  constante serait du bundling.

## Acceptance Criteria (repris du ticket, tie-back)

| AC | Engagement du plan |
|---|---|
| **AC1** — la limite appliquée est celle du transport (4096), pas `MAX_INPUT_LEN` | Phase 1 + Phase 2, étape 2.2 |
| **AC2** — le refus nomme le transport et sa limite | Phase 2, étape 2.3 |
| **AC3** — contrôle négatif : 4095 passe | Phase 4, test `refuse_...` / `accepte_4095` |
| **AC4** — un test couvre la fenêtre 4096–10 000 (5000 refusé par l'outil) | Phase 4, test `fenetre_5000_refusee_par_l_outil` |
| **AC5** — la propriété de la découpe est décidée et écrite | § Décision AC5 + Phase 1 (doc constante), Phase 2.3 (message), Phase 3.1 (commentaire gateway) |
| **AC6** — preuve de non-vacuité : rejouer 12 000 et 5000 | Phase 4, test `rejeu_2026_09_01` + § Preuve |

## Phases

### Phase 1 — La constante partagée

**1.1** Créer `crates/mika-common/src/telegram.rs` :

```rust
//! Limites du transport Telegram, nommées là où les deux crates les voient.
//!
//! `mika-agent` s'en sert pour refuser avant l'appel réseau ; `mika-gateway`
//! s'en sert pour refuser le texte tel qu'il partira, préfixe compris.
//!
//! **Qui découpe :** l'agent. Le gateway envoie tel quel (`send as-is`) parce
//! qu'il insère une ligne `outbound_messages` par envoi et que le routage des
//! réponses (`reply_to_message`) dépend de cette relation 1:1. Voir mika#2134.

/// Nombre maximal d'unités UTF-16 dans le champ `text` de `sendMessage`.
///
/// Telegram compte en unités UTF-16, pas en octets ni en `char`. Un emoji hors
/// BMP compte 2. `str::len()` (octets) sur-restreindrait tout texte accentué ;
/// `chars().count()` sous-restreindrait les emoji.
pub const MAX_TEXT_UTF16_UNITS: usize = 4096;

/// Longueur d'un texte telle que Telegram la compte.
pub fn text_len_utf16(s: &str) -> usize {
    s.encode_utf16().count()
}
```

**1.2** Déclarer `pub mod telegram;` dans `crates/mika-common/src/lib.rs`
(insertion alphabétique entre `team` et `telemetry`).

**1.3** Tests unitaires dans le module : ASCII (`"abcd"` → 4), accentué
(`"éé"` → 2, alors que `len()` → 4), emoji hors BMP (`"👆"` → 2, `len()` → 4),
vide → 0.

### Phase 2 — La garde de l'outil

Fichier : `crates/mika-agent/src/tools/send_message.rs`.

**2.1 — Réordonner.** Déplacer la garde de longueur **après**
`strip_internal_tags` et **avant** `save_message_with_task_context`. Ordre final :
`text` vide → nettoyage → vide après nettoyage → **longueur** → persistance →
envoi. Un message refusé n'entre pas dans l'historique.

**2.2 — Remplacer la borne.** Supprimer `text.len() > MAX_INPUT_LEN` ; mesurer
`mika_common::telegram::text_len_utf16(&cleaned)` contre
`MAX_TEXT_UTF16_UNITS`. Retirer `MAX_INPUT_LEN` de l'import `use super::{...}`
s'il n'est plus utilisé dans le fichier.

**2.3 — Le message de refus.** Il nomme le transport, sa limite, la longueur
mesurée, et dit qui découpe :

```
Telegram accepts at most 4096 characters per message; this text is <N>.
The gateway does not split messages — split it yourself into chunks under
4096 characters and send them in order, and tell the user you are sending
it in parts.
```

Chaîne en anglais, comme toutes les sorties d'outil du fichier (`'text' is
required.`, `Message delivery failed:`) : c'est un LLM qui la lit, et la
cohérence du fichier prime. AC2 exige que le refus **nomme le transport et sa
limite** — l'exemple français du ticket est illustratif, la clause liante est
respectée.

**2.4 — La description de l'outil.** Le refus arrive trop tard : le modèle doit
savoir avant d'appeler. Dans `definition()`, ajouter à la description du champ
`text` : `"The message to send (max 4096 characters — Telegram's per-message
limit). Longer content must be split by you and sent as several calls; nothing
downstream splits it."` C'est la moitié préventive d'AC5.

### Phase 3 — La garde miroir du gateway

Fichier : `crates/mika-gateway/src/routes.rs`.

**3.1** Après `let owned_text = format_outbound_text(...)` (`:1920`) et **avant**
la résolution du client Telegram, refuser si
`mika_common::telegram::text_len_utf16(&owned_text) > MAX_TEXT_UTF16_UNITS` :
`400 BAD_REQUEST` avec
`{"error": "text exceeds Telegram's 4096-character limit (<N> units, prefix included); the gateway does not split messages"}`.

**3.2** Remplacer le commentaire `// Send to Telegram (no message splitting —
send as-is)` par une version qui porte la décision AC5 et sa raison (relation
1:1 avec `outbound_messages`, cf. § Décision AC5), en citant mika#2134.

**Pourquoi cette phase existe.** C'est la seule couche qui mesure le texte
**préfixé**, et c'est le seul point que les ~15 appelants non-outil traversent
tous. Elle transforme un `502 BAD_GATEWAY` opaque en une raison nommée qui
remonte telle quelle : `messaging.rs:150` inclut déjà le corps de la réponse
dans le `bail!` (`gateway /send returned {status}: {body_snippet}`, 200
caractères), donc la phrase arrive au modèle via `SendOutcome::Failed { reason }`
puis `ToolOutput::error`. Aucune plomberie supplémentaire.

**Note de coût :** `messaging.rs` réessaie une fois après 2 s avant de conclure
à l'échec (`:203`). Un 400 sera donc payé deux fois. C'est le comportement
existant pour tout non-2xx ; ne pas le modifier ici (ce serait du bundling —
la distinction 4xx définitif / 5xx transitoire est un ticket à part).

### Phase 4 — Les preuves

Tests ajoutés dans le module `tests` existant de `send_message.rs`, avec le
`MockSender` déjà présent.

**4.1 `accepte_4095_controle_negatif` (AC3).** `"a".repeat(4095)` → pas
d'erreur, et `mock.sent()` contient bien le texte. Le contrôle négatif vérifie
la **livraison**, pas seulement l'absence d'erreur.

**4.2 `accepte_4096_a_la_borne`.** Exactement 4096 passe : la borne est
inclusive, `>` et non `>=`.

**4.3 `fenetre_5000_refusee_par_l_outil` (AC4).** `"a".repeat(5000)` →
`is_error == true`, le contenu cite `4096` et `5000`, et **`mock.sent()` est
vide** — l'assertion qui prouve que le refus est dans l'outil et non au
transport. C'est le cas exact de la Partie 1/4.

**4.4 `rejeu_2026_09_01` (AC6).** Les deux longueurs du jour même : 12 000 et
5000. Les deux refusées, les deux avec un message citant Telegram et 4096, aucune
n'atteignant le `MockSender`.

**4.5 `accentue_sous_la_limite_passe`.** `"é".repeat(4000)` (8000 octets, 4000
unités UTF-16) → **passe**. C'est le test qui échoue si quelqu'un remet
`text.len()` ; sans lui, la régression octets/UTF-16 est invisible.

**4.6 `tags_internes_ne_comptent_pas`.** Un texte dont le brut dépasse 4096 mais
dont la version nettoyée est en dessous → passe. Prouve l'ordre de la phase 2.1.

**4.7 Gateway.** Test unitaire sur la mesure post-préfixe : pour
`agent_name = Some("work-mika")`, un texte de 4090 unités donne
`text_len_utf16(format_outbound_text(...)) > 4096` — la fenêtre résiduelle des
agents nommés est bien attrapée là et nulle part ailleurs.

### Preuve de non-vacuité (AC6)

Le correctif n'est pas vide si, et seulement si, la suite **échoue sur `main`**.
Vérification obligatoire avant la PR : appliquer les tests 4.3 et 4.4 seuls sur
`origin/main` — ils doivent échouer (le texte de 5000 atteint `MockSender`).
Puis appliquer le correctif — ils passent, et 4.1 continue de passer. Sans ce
double sens, la suite atteste la garde qu'elle a écrite, pas le monde.

## Commandes de vérification

```bash
cargo test -p mika-agent send_message
cargo test -p mika-common telegram
cargo test -p mika-gateway send
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## Risques

| risque | mitigation |
|---|---|
| Mesure en octets réintroduite par un futur refactor | Test 4.5 (`"é".repeat(4000)`) échoue immédiatement |
| Garde trop stricte fermant la fenêtre par le bas | Tests 4.1 + 4.2, contrôle négatif exigé par AC3 |
| Le 400 du gateway payé deux fois par le retry | Documenté, comportement existant, hors périmètre |
| Fenêtre résiduelle des agents nommés (préfixe) | Fermée par la phase 3 avec raison nommée ; test 4.7 |
