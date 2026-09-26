---
title: La copie utilisateur du gateway cesse d'être anglaise en dur, et l'action cesse d'être terminale — Plan
type: fix
date: 2026-09-19
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
issue: senara-solutions/mika#2025
---

# La copie utilisateur du gateway cesse d'être anglaise en dur, et l'action cesse d'être terminale — Plan

## Goal Capsule

- **Objectif :** les seize chaînes que le gateway sert directement à un utilisateur Telegram cessent d'être des littéraux anglais posés au site d'envoi. Elles passent par un producteur unique qui rend la langue de l'utilisateur, et le message `/unlink` place l'action en tête au lieu de la mettre en dernière ligne d'un avertissement.
- **Moyens :** le `language_code` que Telegram envoie déjà et que le gateway jetait traverse la frontière (U1) ; un module de copie à `enum` exhaustif devient le producteur unique, sur le motif `hosting_ground_truth_line` de mika#2290 (U2) ; les seize sites basculent, ce qui absorbe au passage un doublon littéral (U3) ; la copie `/unlink` est restructurée (U4) ; le suffixe non reconnu cesse d'être confondu avec le `/unlink` nu (U5) ; observabilité bornée et gardes structurelles (U6).
- **Autorité :** le corps de mika#2025 > le commentaire opérateur du 19/09, qui porte le motif du dé-parquage et **aucune correction de trajectoire technique**. Le corps pose deux pistes ; ce plan en retient une, en diffère une, et écrit pourquoi (D1).
- **Conditions d'arrêt.**
  - **(a) NON DÉCLENCHÉE.** « Arrêter si mika#2023 corrige déjà ceci. » Il ne le corrige pas, et le corps du ticket l'a établi avant nous : #2023 porte sur la persona de l'agent, la copie traitée ici ne traverse aucune persona — elle est émise par le gateway avant tout appel à l'agent, et sur les chemins pré-appariement il n'y a aucun agent à appeler.
  - **(b) DÉCLENCHÉE, et elle réduit le périmètre.** « Arrêter si la piste retenue vit dans `mika-cloud`. » La piste *locale du compte* y vit à moitié : `ls /data/workspace/mika-platform/` rend `claude-pilot` et `mika`, **pas `mika-cloud`**. Elle est donc **différée** avec son critère de reprise (D1) ; la piste retenue est entièrement dans `crates/mika-gateway`.
- **Profil d'exécution :** Rust, `crates/mika-gateway` seul. **Aucune migration, aucun changement de schéma, aucun appel réseau nouveau, aucune modification côté agent, aucun changement de statut HTTP, aucun `parse_mode` touché.**
- **Finish/ship :** le pipeline `/mika` sur la branche `ux/2025/gateway-la-copie-telegram-unlink-est-en` ouvre la PR qui clôt mika#2025.

---

## Product Contract

### Summary

Vincent, champion francophone, a envoyé `/unlink` deux fois sans exécuter la confirmation. Le ticket en tire deux constats, tous deux réels, et propose deux pistes dont **aucune n'est disponible aujourd'hui dans le code** : ni la locale du compte ni la langue du dernier message reçu n'existent dans le gateway. Il manque en outre un troisième défaut, qui est celui que la correction du premier **rend plus probable**.

Le travail livre : la langue, la saillance, et la fermeture du troisième défaut — le tout dans un seul crate, sans migration.

### Problem Frame

#### M1 — La copie citée est exacte, mais ses coordonnées ont bougé

Le ticket cite `routes.rs:1741-1744`. La copie vit aujourd'hui en `routes.rs:1916-1919` (`handle_unlink`, déclaré `routes.rs:1908`) et est **verbatim identique** à la citation :

```
⚠️ Unlinking will release your Telegram from this Mika account.
You will need a new invite link from your admin to re-pair.
This cannot be undone.

To confirm, send: /unlink confirm
```

Le diagnostic du ticket sur le fond tient donc entièrement. Seules les citations sont à rafraîchir.

#### M2 — Les deux pistes du ticket demandent chacune un champ à créer, et une seule est additive

Le ticket propose « locale du compte, ou langue du dernier message reçu ». Lecture du code :

| piste | état réel | coût |
|---|---|---|
| **locale du compte** | `customers` n'a **pas** de colonne de langue. Relecture de `migrations/001_customers.sql` (schéma initial), `008_customer_telegram_bot.sql` et `010_customers_pairing_rejection.sql` — les trois seuls `ALTER TABLE customers` du crate : `name`, `plan`, `status`, `telegram_chat_id`, `timezone`, `pairing_token`, `pairing_expires_at`, `last_update_id`, `paired_at`, `bot_token`, `bot_username`, `webhook_secret`, `pairing_rejected_at`, `pairing_rejection_reason`. Aucune langue. | migration 011 **+ un écrivain**, qui est la console `mika-cloud`, absente de ce worktree |
| **langue du dernier message reçu** | `TelegramMessage` (`telegram.rs:32-46`) désérialise `chat`, `text`, `photo`, `caption`, `document`, `reply_to_message`. **`from` n'est pas désérialisé du tout**, donc `from.language_code` n'existe nulle part dans le processus. | un champ `#[serde(default)]` additif, **zéro migration** |

**Conséquence :** il n'y a aujourd'hui **aucun** signal de langue dans le gateway. Les deux pistes sont des créations, pas des lectures — ce que le ticket, écrit de l'extérieur, ne pouvait pas savoir. La seconde est additive ; la première est cross-repo et contrainte en ordre.

#### M3 — Le périmètre est de seize chaînes, pas d'une

Inventaire exhaustif des chaînes que `crates/mika-gateway` sert *directement* à un utilisateur Telegram (tout argument littéral de `CustomerTelegramClient::send_message`, plus les deux constantes) :

| # | site | chaîne (tête) | chemin |
|---|---|---|---|
| 1 | `routes.rs:674` | `Welcome! If you have an invite link…` | `/start` nu |
| 2 | `routes.rs:689` | `I can read text and image messages…` | média non supporté |
| 3 | `routes.rs:747` | `Please pair your account first…` | non apparié |
| 4 | `routes.rs:924` | `That image is too large…` | photo > 5 Mo |
| 5 | `routes.rs:933` | `I couldn't recognize that image format…` | format d'image |
| 6 | `routes.rs:943` | `Sorry, I couldn't download your photo…` | échec de téléchargement |
| 7 | `routes.rs:1763` | `Invalid or expired invite link.` | token malformé |
| 8 | `routes.rs:1810` | `Invalid or expired invite link.` | **même littéral que 7** |
| 9 | `routes.rs:1822` | `This Telegram account is already linked…` | 23505 sur `telegram_chat_id` |
| 10 | `routes.rs:1826` | `Pairing failed. Please contact support.` | 23505 autre |
| 11 | `routes.rs:1916` | `⚠️ Unlinking will release…` | **le sujet du ticket** |
| 12 | `routes.rs:1924` | `Your Telegram is not linked…` | `/unlink` à froid |
| 13 | `routes.rs:1955` | `✅ Unlinked. Your invite link…` | confirmation réussie |
| 14 | `routes.rs:1964` | `Nothing to unlink…` | confirmation à froid |
| 15 | `routes.rs:2924` | `TRANSIENT_ERROR_MSG` | erreur transitoire |
| 16 | `routes.rs:2927` | `OFFLINE_ERROR_MSG` | conteneur injoignable |

**Seize littéraux, quinze clés distinctes** (7 et 8 sont le même texte à deux sites).

Ce tableau décide le périmètre. Localiser la seule entrée 11 donnerait à un même utilisateur, dans une même session, un `/unlink` français entouré de quatorze messages anglais — une incohérence plus visible que le défaut d'origine, parce qu'elle prouve que quelqu'un a regardé et s'est arrêté. Le ticket dit d'ailleurs « localiser la copie utilisateur du gateway », pas « la copie de `/unlink` ».

**Hors périmètre, et nommé :** `routes.rs:1798` envoie `"Hello!"` au conteneur de l'agent, pas à l'utilisateur. C'est l'amorce que l'agent reçoit et à laquelle il répond *dans sa persona* — la surface de mika#2023, pas la nôtre. Le toucher déplacerait la frontière que le corps du ticket a explicitement tracée.

#### M4 — Le troisième défaut : trois états distincts, une seule réponse — et ce ticket l'aggrave

`telegram.rs:194-200` :

```rust
if text == "/unlink" || text.starts_with("/unlink ") {
    let canonical: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if canonical == "/unlink confirm" {
        return ParsedMessage::UnlinkConfirm { chat_id };
    }
    return ParsedMessage::Unlink { chat_id };
}
```

Le parseur **sait** qu'il vient d'écarter un suffixe (il a évalué `canonical != "/unlink confirm"`) et **jette cette information**. `ParsedMessage::Unlink` confond donc :

1. `/unlink` nu — première demande ;
2. `/unlink` nu répété — la demande a déjà été faite ;
3. `/unlink confirmer`, `/unlink oui`, `/unlink yes` — une confirmation **tentée** et non reconnue.

Les trois reçoivent le même avertissement. Le commentaire du code assume le cas 3 (« a stray suffix (typo) falls back to the warning path ») ; ce qu'il ne dit pas, c'est que l'utilisateur ne peut pas distinguer sa tentative refusée d'un simple rappel.

**Le couplage qui rend cette moitié non optionnelle :** servir la copie en français à un francophone rend `/unlink confirmer` *plus* probable, pas moins. La correction du défaut 1 augmente mécaniquement la population du défaut 3. Livrer la localisation sans fermer ce cas, c'est déplacer la friction d'un cran et la rendre plus difficile à attribuer — l'utilisateur aurait alors tapé la bonne intention dans la bonne langue et n'obtiendrait toujours rien.

Le double-envoi de Vincent est le cas 2 ; la lecture du ticket le couvre. Le cas 3 est celui que la correction crée.

#### M5 — Le rendu typographique existe déjà, mais il n'est pas garanti

mika#2291 a armé `parse_mode=HTML` par défaut : `telegram.rs:753-782` fait passer **toute** copie sortante par `telegram_markdown::tokenize` puis `render_html`, qui produit `<b>` sur `**gras**`, `<code>` sur les backticks, et échappe `<`, `>`, `&` (`telegram_markdown.rs:508-537`).

La saillance typographique est donc gratuite. **Mais elle n'est pas garantie**, et sur les deux mêmes chemins :

- `MIKA_TELEGRAM_HTML_RENDER=0` désarme le rendu et le texte part brut ;
- un `400` de Telegram déclenche le repli plain-text de mika#2291 (`telegram.rs:784`), également brut.

Dans les deux cas l'utilisateur lit les marqueurs — ce qui est exactement le défaut mesuré chez Al le 2026-09-11 et que mika#2291 a été écrit pour fermer.

**Conséquence de conception :** la saillance de l'action doit tenir **sans aucun rendu**. L'ordre des lignes tient toujours ; le markdown est un renfort qui dégrade proprement. Ce plan ne fait reposer aucune exigence sur le rendu (D3).

#### M6 — Un seul site de dispatch, deux sites de parse : le compilateur peut tenir l'invariant

`parse_update` est appelé deux fois (`routes.rs:475` mono-bot, `routes.rs:572` per-customer) ; `dispatch_parsed_message` (`routes.rs:598`) est **unique** et reçoit les deux flux.

Un argument `locale` obligatoire sur `dispatch_parsed_message` force donc les deux appelants à en fournir un — **par erreur de compilation, pas par convention**. Aucun scan n'est nécessaire pour cette moitié de l'invariant ; il l'est pour l'autre (U6).

#### M7 — Le handler est déjà correct sur le fond, et rien ici ne le touche

`handle_unlink` (`routes.rs:1908`) ne fait qu'un `SELECT` et ne mute jamais la base ; `handle_unlink_confirm` (`routes.rs:1937`) est un `UPDATE … RETURNING` atomique et idempotent. Le ticket le note et il a raison. **Aucune unité de ce plan ne modifie une requête SQL**, ce qui laisse intact le test d'intégration `tests/unlink.rs`, dont l'en-tête demande explicitement qu'on le tienne synchrone du SQL de production.

### Requirements

- **R1** — Le gateway lit la langue déclarée par l'utilisateur dans Telegram et la résout en une valeur fermée, sur **tous** les chemins, y compris pré-appariement.
- **R2** — Les quinze clés de copie de M3 sont servies en français et en anglais. Aucune n'est disponible dans une seule langue.
- **R3** — Le message `/unlink` place la commande de confirmation **avant** l'avertissement, et sa saillance ne dépend d'aucun rendu.
- **R4** — Une confirmation tentée mais non reconnue reçoit une réponse qui le dit, distincte du rappel.
- **R5** — La forme française de la confirmation est acceptée.
- **R6** — Aucune copie utilisateur littérale ne peut être réintroduite au site d'envoi sans faire rougir un test.
- **R7** — Aucune migration, aucun changement de SQL, aucun changement du contrat `parse_mode`.
- **R8** — La langue résolue et sa provenance sont lisibles par un opérateur, sans que l'instrument écrive une ligne par message.

### Non-goals

- **Un framework i18n** (`fluent`, `gettext`, fichiers de ressources). Quinze chaînes statiques, aucune interpolation, aucune pluralisation, deux langues : la maison a déjà tranché cette forme sur `hosting_ground_truth_line` (mika#2290) et sur `MIKA_DOCTRINE_BODY_OPERATOR`/`_FAMILY` (mika#2292) — le même fait écrit deux fois derrière un `match` exhaustif. Une dépendance et un pipeline d'extraction pour quinze `&'static str` coûteraient plus que ce qu'ils rendent.
- **Une troisième langue.** Le `match` exhaustif la rendra obligatoire et non oubliable le jour où elle sera demandée ; l'ajouter aujourd'hui serait traduire pour une population qui n'existe pas.
- **La colonne `customers.locale`.** Différée avec son critère (D1).
- **Localiser la commande elle-même** au-delà de l'alias de R5. Une commande localisée est une surface de parsing supplémentaire à documenter et à faire vivre ; l'alias ferme le cas que ce ticket crée, sans ouvrir cette porte.
- **La copie du Console, de l'agent, ou l'amorce `"Hello!"`** (M3).
- **Toucher au rendu HTML** de mika#2291, à son kill-switch ou à son repli.

---

## Planning Contract

### D1 — Le signal de langue est le `language_code` de Telegram ; la colonne est différée, avec son critère

**Retenu :** `message.from.language_code`, désérialisé additivement, normalisé (`fr-FR` → `Fr`), replié sur `En` quand il est absent ou non reconnu.

**Pourquoi celui-là :**

1. **Il couvre les chemins que l'autre ne peut pas couvrir.** Les entrées 1, 3, 7, 8, 9 et 10 de M3 sont servies à quelqu'un qui n'est **pas** apparié — invite invalide, `/start` nu, compte déjà lié. Il n'y a alors aucun `customer_id`, donc aucune ligne d'où lire une locale de compte. Or ces chemins *sont* l'onboarding, c'est-à-dire exactement le parcours mesuré par ce ticket. Une solution qui ne les couvre pas manque le terrain où le défaut a été observé.
2. **Il ne dépend d'aucun dépôt absent.** La colonne demanderait un écrivain, et l'écrivain est la console `mika-cloud`, hors de ce worktree. L'ordre serait contraint — `mika` d'abord (lire avec un défaut sûr), `mika-cloud` ensuite — exactement la forme mika#2023 → mika-cloud#242 et mika#2290 → mika-cloud. Cette forme est acceptable quand le lecteur seul ferme déjà le défaut ; ici il ne le fermerait pas : tant que personne n'écrit, la colonne est NULL et **rien ne change pour Vincent**.
3. **Son coût est un champ `#[serde(default)]`.** Aucune migration, aucun déploiement coordonné, aucune réversion à préparer.

**Ce que ce choix ne couvre pas, écrit plutôt que caché :** `language_code` est la langue de l'**interface Telegram** du client, pas celle de son compte Mika. Un francophone dont le téléphone est en anglais continuera de recevoir de l'anglais. C'est une population réelle et ce plan ne la ferme pas.

**Critère de reprise de la colonne — une mesure, pas une intuition :** ouvrir le ticket compagnon `mika-cloud` le jour où un tenant est observé recevant une langue qui n'est pas la sienne (M3 entrée 11 en anglais chez un francophone, malgré le correctif). Le résolveur est écrit en cascade à porte nommée précisément pour que cette colonne s'insère en une porte de plus, devant le `language_code`, sans que rien d'autre bouge. **Tant que la mesure n'existe pas, la migration serait un schéma posé pour un écrivain hypothétique.**

**Écarté — une variable de déploiement `MIKA_GATEWAY_LOCALE` :** le gateway est un routeur **multi-tenant** (un processus, un registre Postgres, un bot par client depuis la migration 008). Une langue par déploiement ne peut pas servir deux tenants de langues différentes ; elle transformerait un défaut individuel en politique. Elle ne mérite même pas d'être le plancher : le plancher est `En`, qui est le comportement d'aujourd'hui.

### D2 — Deux langues, `match` exhaustif, aucun bras `_ =>`

Le producteur est un `match` sur `(UserMessage, Locale)` sans bras attrape-tout, sur le modèle littéral de `hosting_ground_truth_line` (`crates/mika-agent/src/prompt.rs:1120`).

La propriété achetée est celle que mika#2290 a nommée : **le compilateur, pas un relecteur, force chaque nouveau message et chaque nouvelle langue à décider**. Un bras `_ => <anglais>` rendrait le manque silencieux et ramènerait exactement le défaut d'origine, une clé à la fois. Il est donc interdit, et U6 l'épingle.

### D3 — La saillance est structurelle ; le markdown est un renfort qui dégrade proprement

Par M5, la copie `/unlink` est restructurée ainsi (français ; l'anglais est la même structure) :

```
Pour confirmer, envoie : `/unlink confirm`

⚠️ Cela libérera ton Telegram de ce compte Mika.
Il te faudra un nouveau lien d'invitation pour te reconnecter.
Cette action est définitive.
```

Trois propriétés, dans l'ordre de leur robustesse :

1. **L'action est en première ligne.** Vrai sous rendu HTML, vrai sous kill-switch désarmé, vrai sous repli plain-text. C'est la seule qui ferme le comportement décrit par le ticket — « le lecteur voit ⚠️, cannot be undone, et renvoie la commande qu'il connaît ».
2. **La commande est en `<code>` sous rendu.** Elle se détache et devient tappable.
3. **Sous repli, l'utilisateur lit des backticks autour d'une commande.** Dégradation acceptée et nommée : un backtick encadrant `/unlink confirm` reste lisible comme une citation de commande, là où le `**gras**` littéral mesuré chez Al ne signifiait rien pour son lecteur. **Le gras est écarté pour cette raison précise** — il est le marqueur dont mika#2291 a mesuré le coût en clair.

### D4 — Le suffixe non reconnu obtient sa propre variante, et `confirmer` est accepté

Deux gestes, tous deux petits, qui ferment M4 :

- `ParsedMessage::Unlink` gagne un discriminant portant l'information que le parseur calculait déjà et jetait. Le handler sert alors une copie distincte, qui **cite le suffixe refusé** et redonne la commande exacte.
- `confirmer` rejoint `confirm` comme forme reconnue de la confirmation.

**Pourquoi les deux et pas l'un des deux.** L'alias seul laisserait `/unlink oui`, `/unlink yes`, `/unlink ok` silencieux — il déplacerait la frontière sans la rendre lisible. Le message distinct seul serait honnête mais laisserait un francophone, servi en français, buter sur la forme anglaise que la copie lui demande : le plan aurait alors créé le piège et documenté sa sortie sans l'ouvrir.

**Ce que l'alias ne fait pas :** la copie continue de prescrire `/unlink confirm` dans les deux langues. Une commande unique, une seule chose à écrire dans la documentation et dans le support ; l'alias est une tolérance en entrée, pas une seconde interface.

### D5 — Un site de résolution, un argument obligatoire

Par M6, `resolve_locale(&TelegramUpdate) -> (Locale, LocaleSource)` est appelé aux deux sites de `parse_update`, et `dispatch_parsed_message` prend `locale: Locale`.

**Écarté — porter la locale dans chaque variante de `ParsedMessage` :** neuf variantes à élargir, cinq tests de parsing à réécrire, et surtout `parse_update` cesserait d'être une fonction pure du contenu du message. Le fait qu'elle le soit est ce qui rend ses tests bon marché.

**Écarté — résoudre dans `parse_update` en changeant son type de retour :** casse les cinq tests existants (`telegram.rs:1338-1395`) et mêle deux questions — « qu'a demandé l'utilisateur ? » et « dans quelle langue lui répondre ? » — qui n'ont pas la même durée de vie.

### D6 — L'observabilité est bornée par la rareté du chemin, pas par un seuil

Le gateway voit **tout** le trafic Telegram de tous les tenants. Une ligne de journal par message résolu noierait le signal qu'elle existe pour lever — la doctrine que mika#2131 a dû écrire (« une observabilité qui journalise tout le monde ne distingue plus personne ») et que mika#2334 applique en n'émettant son agrégat que lorsque le tick agit.

Deux événements, tous deux sur des chemins rares par nature, et **aucun** sur le chemin `Text` qui porte le volume :

- `gateway_locale_resolved` (INFO) — émis sur les chemins de **commande** seulement (`/start`, `/unlink`, `/unlink confirm`). Champs : `chat_id`, `locale`, `locale_source`. C'est la réponse à « ce tenant reçoit-il du français, et par quelle porte ? » sans lire la base, sur le modèle de `llm_budget_resolved` (mika#2293) et pour la même leçon : *un réglage qu'on ne peut pas observer n'est pas un réglage.* Une provenance `default` signifie que Telegram n'a envoyé aucun `language_code` — le remède est alors la colonne différée (D1), pas un élargissement de la normalisation.
- `unlink_suffix_unrecognized` (INFO) — champs `chat_id`, `locale`. **Le suffixe refusé n'est jamais journalisé** : c'est du contenu utilisateur, au standard de mika#2126 que mika#2291 rappelle (« the body is NEVER logged »). Il est cité à l'utilisateur, pas à l'opérateur. **Régime attendu : non vide.** C'est la mesure du cas M4, et sa valeur est de dire si l'alias de D4 couvre les formes réellement tapées ou s'il en manque.

**Aucun événement sur la copie elle-même.** Quelle clé a été servie est déterminé par le chemin, que le journal existant décrit déjà.

### Risks

- **RI1 — `language_code` mal formé ou inattendu.** Normalisation par préfixe avant le premier `-`, comparaison ASCII-insensible à la casse ; tout ce qui n'est pas reconnu tombe sur `En` avec `LocaleSource::Default`. Aucune valeur d'entrée ne peut produire un panic ni une absence de copie — le `match` est total sur `Locale`, qui est fermé.
- **RI2 — Une dix-septième copie ajoutée en anglais en dur.** C'est la régression principale et **aucun test comportemental ne peut la voir** : elle ne rend aucune décision fausse, elle rétablit le défaut sur une clé. Fermée par le scan structurel de U6, sur le modèle de `mika2131_exclusion_skips_never_return_to_an_uncollected_debug` et `mika2323_no_gate_predicate_reads_the_actor`.
- **RI3 — La copie française casse le rendu HTML.** Les apostrophes et les accents ne sont pas réservés ; `escape_html` traite `<`, `>`, `&` en une passe (`telegram_markdown.rs:552`). Aucune copie de ce plan n'introduit ces trois caractères. Épinglé par un test de rendu de bout en bout sur la copie `/unlink` dans les deux langues (U6), qui vaut mieux qu'un raisonnement sur l'échappement.
- **RI4 — Le repli plain-text montre les backticks.** Accepté, mesuré et nommé en D3. C'est un renfort qui dégrade, pas une exigence qui tombe : R3 est satisfait par l'ordre des lignes seul.
- **RI5 — Traduction approximative.** La copie française est relue contre le registre `FAMILY_SOUL` : tutoiement, aucun jargon d'infrastructure. `console.getmika.ai` (entrée 16) et les emojis `⚠️` / `✅` restent inchangés — ce sont des invariants, pas du texte.
- **RI6 — Sur-périmètre.** Ce plan touche deux fichiers de production (`telegram.rs`, `routes.rs`) plus un module neuf. Aucune requête SQL, aucune migration, aucun `parse_mode`. Si une unité demande une migration à l'implémentation, c'est que la piste différée de D1 a été reprise par inadvertance : **halte**, et rouvrir D1 explicitement.

---

## Implementation Units

### U1 — Le `language_code` traverse la frontière

`crates/mika-gateway/src/telegram.rs`.

- Ajouter `TelegramUser { language_code: Option<String> }` et le champ `from: Option<TelegramUser>` sur `TelegramMessage`, tous deux `#[serde(default)]`.
- Ajouter `Locale { Fr, En }` et `LocaleSource { TelegramLanguageCode, Default }`.
- Ajouter `resolve_locale(&TelegramUpdate) -> (Locale, LocaleSource)` : préfixe avant `-`, casse ignorée, `fr` → `Fr`, tout le reste → `(En, Default)`.

Additif de bout en bout. Aucune variante de `ParsedMessage` n'est touchée (D5), donc les cinq tests de parsing existants continuent de compiler sans modification.

### U2 — Le producteur unique de copie

`crates/mika-gateway/src/copy.rs` (module neuf), déclaré dans `lib.rs`.

- `enum UserMessage` — une variante par clé de M3 (quinze), plus `UnlinkSuffixUnrecognized` introduite par U5. La variante nommée est ce qui rend le site d'envoi lisible : `send_message(chat_id, copy::render(UserMessage::UnlinkWarning, locale))`.
- `fn render(msg: UserMessage, locale: Locale) -> &'static str` — `match` exhaustif sur `(UserMessage, Locale)`, **aucun bras `_ =>`** (D2), chaque paire portant son texte.
- `UnlinkSuffixUnrecognized` cite le suffixe refusé, donc elle rend une `String` par une fonction voisine dédiée — `render` conserve sa signature `&'static str` pour les seize clés statiques, la fonction citante étant le seul site à allouer.

### U3 — Les seize sites basculent

`crates/mika-gateway/src/routes.rs`.

- `dispatch_parsed_message` prend `locale: Locale` ; les deux appelants (`routes.rs:475`, `routes.rs:572`) appellent `resolve_locale` et le fournissent. Le compilateur tient l'invariant (M6).
- La locale descend jusqu'aux handlers qui émettent : `handle_pairing`, `handle_unlink`, `handle_unlink_confirm`, `handle_photo_message`, le chemin non apparié, et les deux constantes via `forward_error_message` / `reply_transient_error`.
- `TRANSIENT_ERROR_MSG` et `OFFLINE_ERROR_MSG` disparaissent en tant que constantes ; `forward_error_message(is_connect, locale)` conserve sa forme de fonction pure — elle était déjà l'ancre exacte de ce branchement.
- **Le doublon de M3 (7/8) est absorbé sans geste dédié :** les deux sites appellent la même variante.

### U4 — La copie `/unlink` est restructurée

`crates/mika-gateway/src/copy.rs`.

La variante `UnlinkWarning` porte, dans les deux langues, la structure de D3 : action en première ligne, avertissement ensuite, commande en backticks. Aucun code de `handle_unlink` ne change au-delà de l'appel à `render` — le `SELECT` est intact (M7).

### U5 — Le suffixe non reconnu cesse d'être confondu

`crates/mika-gateway/src/telegram.rs` et `routes.rs`.

- `ParsedMessage::Unlink` porte le discriminant du suffixe refusé.
- `confirmer` rejoint `confirm` comme forme reconnue (R5), sur la canonicalisation de blancs qui existe déjà.
- `handle_unlink` sert `UnlinkSuffixUnrecognized` sur ce discriminant, `UnlinkWarning` sinon.
- Le chemin `Ok(None)` (non lié) est inchangé : il précède la question du suffixe.

### U6 — Observabilité et gardes

- Les deux événements de D6, aux sites nommés.
- **Scan structurel (RI2)** : aucun appel à `send_message` dans `crates/mika-gateway/src/` ne reçoit un littéral de chaîne. Le seul argument admis est un produit de `copy::`. Allowlist livrée **vide** ; quand il rougit, la résolution est de passer par `copy::`, jamais d'ajouter une entrée — la règle que mika#2323 a dû écrire pour `ACTOR_READING_PREDICATES_ALLOWED`.
- **Scan structurel (D2)** : le `match` de `render` ne contient aucun bras `_ =>`, sur le modèle de `mika2305_the_label_match_has_no_wildcard_arm`.
- **Test de rendu (RI3)** : la copie `/unlink` française et anglaise, passée par `tokenize` + `render_html`, produit un HTML dont les trois caractères réservés sont échappés et dont la commande est en `<code>`.
- **Test d'ordre (R3)** : dans les deux langues, l'index de `/unlink confirm` dans la copie est **inférieur** à celui du marqueur d'avertissement. C'est l'assertion qui porte la correction du ticket, et elle ne dépend d'aucun rendu.

---

## Verification Contract

Tous les tests sont **purs** — aucun Postgres, aucun réseau, aucune variable d'environnement. C'est possible parce que `resolve_locale` et `render` sont des fonctions totales de leurs arguments, et c'est ce qui les rend exécutables dans la CI du crate telle qu'elle est.

| # | Assertion | Unité | Forme |
|---|---|---|---|
| V1 | `fr`, `fr-FR`, `FR-ca` → `(Fr, TelegramLanguageCode)` | U1 | unitaire |
| V2 | `en`, `de`, `""`, absent, `from` absent → `(En, Default)` | U1 | unitaire |
| V3 | Les seize variantes rendent un texte non vide dans les deux langues | U2 | unitaire, itération sur un inventaire const |
| V4 | Les rendus `Fr` et `En` d'une même variante **diffèrent** — une traduction oubliée est une copie identique | U2 | unitaire |
| V5 | `/unlink confirm` apparaît avant le marqueur d'avertissement, dans les deux langues | U4 | unitaire |
| V6 | `/unlink confirmer` → `UnlinkConfirm` ; `/unlink   confirmer` aussi | U5 | unitaire |
| V7 | `/unlink oui` → `Unlink` avec le discriminant « suffixe refusé » ; `/unlink` nu → sans | U5 | unitaire |
| V8 | `/unlinkxxx` reste du texte libre — non-régression du commentaire de `telegram.rs:189-193` | U5 | unitaire |
| V9 | La copie `/unlink` rendue en HTML échappe `< > &` et porte `<code>` | U6 | unitaire, via `telegram_markdown` |
| V10 | Aucun littéral de chaîne en argument de `send_message` sous `src/` | U6 | scan de source |
| V11 | Le `match` de `render` n'a aucun bras `_ =>` | U6 | scan de source |
| V12 | `cargo test -p mika-gateway` et `cargo clippy` au vert | — | CI |

**Contrôle négatif de V10 et V11.** Chaque scan est accompagné d'une assertion de bonne foi qui le fait rougir sur une entrée fabriquée, pour distinguer « le scan ne trouve rien » de « le scan ne cherche rien » — la classe que mika#2205 a dû nommer.

**Sonde post-déploiement, et ses trois haltes.** Rejouer le parcours sur un tenant francophone : `/start` avec une invite invalide, puis `/unlink`, puis `/unlink confirmer`.

- Attendu : les trois réponses en français, l'action en première ligne de la deuxième, la troisième confirmant le délien.
- **Halte 1 — les réponses restent en anglais.** Lire `gateway_locale_resolved` **avant de toucher à la normalisation** : `locale_source: "default"` signifie que Telegram n'a envoyé aucun `language_code` pour ce client, ce qui est la population que la colonne différée couvrirait (D1) — c'est un résultat, pas une panne. Aucune ligne du tout signifie que le binaire déployé est antérieur au correctif : établir le déploiement d'abord, classe mika#2340.
- **Halte 2 — l'utilisateur lit des backticks bruts.** Vérifier `MIKA_TELEGRAM_HTML_RENDER` et `telegram_html_render_fallback` **avant de retirer le markdown de la copie** : c'est la dégradation nommée en D3/RI4, et si le repli se déclenche c'est mika#2291 qui a quelque chose à dire, pas cette copie.
- **Halte 3 — `unlink_suffix_unrecognized` est vide après plusieurs `/unlink`.** Ne pas conclure que M4 était imaginaire : l'événement ne se déclenche que sur suffixe, et son absence peut signifier que personne n'en a tapé. Vérifier qu'au moins un `/unlink` a été journalisé avant de conclure quoi que ce soit — un scan silencieusement inactif se lit exactement comme un scan qui n'a rien trouvé (mika#2205).

---

## Definition of Done

- [ ] `from.language_code` est désérialisé ; `resolve_locale` est le seul lecteur de la question « quelle langue ».
- [ ] `copy.rs` est le seul producteur de copie utilisateur du crate ; son `match` est exhaustif et sans bras `_ =>`.
- [ ] Les seize sites de M3 passent par `copy::`, le doublon 7/8 ayant fusionné sur une clé unique.
- [ ] La copie `/unlink` porte l'action en première ligne, dans les deux langues.
- [ ] Un suffixe non reconnu reçoit une réponse distincte ; `confirmer` est accepté.
- [ ] `gateway_locale_resolved` et `unlink_suffix_unrecognized` sont émis, et aucun événement n'est émis sur le chemin `Text`.
- [ ] V1–V12 au vert, contrôles négatifs inclus.
- [ ] Zéro migration, zéro requête SQL modifiée, `tests/unlink.rs` inchangé.
- [ ] Les citations de ligne de ce plan sont exactes sur le `HEAD` de la PR.

---

## Acceptance criteria

Le corps de mika#2025 ne porte **pas** de section `## Acceptance criteria` ; les critères ci-dessous sont dérivés de ses deux pistes, de ses deux constats, et des requirements R1–R8.

- **AC1** — Un utilisateur dont Telegram déclare `fr` reçoit en français les seize messages que le gateway lui sert directement. Un utilisateur déclarant autre chose, ou ne déclarant rien, reçoit l'anglais — le comportement d'aujourd'hui.
- **AC2** — Le message de confirmation `/unlink` porte la commande exacte **avant** l'avertissement, et cette propriété tient sans rendu HTML.
- **AC3** — `/unlink confirmer` déclenche le délien. `/unlink <autre chose>` reçoit une réponse qui nomme le suffixe refusé et redonne la commande — distincte du rappel servi à `/unlink` nu.
- **AC4** — Aucune chaîne utilisateur littérale ne subsiste au site d'envoi, et un test rougit si l'une est réintroduite.
- **AC5** — Une clé de copie ne peut pas exister dans une seule langue : le `match` exhaustif l'interdit à la compilation, et V4 interdit la traduction copiée-collée.
- **AC6** — Aucune migration, aucun changement de SQL, aucun changement du contrat `parse_mode` de mika#2291, aucune modification côté agent ou console.
- **AC7** — Un opérateur peut lire la langue résolue et sa provenance pour un tenant donné, sans que l'instrument écrive une ligne par message de conversation.

---

## Sources

- `crates/mika-gateway/src/routes.rs` — `handle_unlink` (1908), `handle_unlink_confirm` (1937), `handle_pairing` (1754), `dispatch_parsed_message` (598), les deux sites de `parse_update` (475, 572), `TRANSIENT_ERROR_MSG` (2924), `OFFLINE_ERROR_MSG` (2927), `forward_error_message` (2934).
- `crates/mika-gateway/src/telegram.rs` — `TelegramMessage` (32-46), `ParsedMessage` (89-137), le bloc `/unlink` de `parse_update` (194-200), `send_message_impl` (745-803), les cinq tests de parsing (1338-1395).
- `crates/mika-gateway/src/telegram_markdown.rs` — `render_html` (508-537), `escape_html` (552).
- `crates/mika-gateway/migrations/` — `001_customers.sql`, `008_customer_telegram_bot.sql`, `010_customers_pairing_rejection.sql` : les trois seuls lieux qui définissent les colonnes de `customers`.
- `crates/mika-gateway/tests/unlink.rs` — le test d'intégration dont l'en-tête exige la synchronisation avec le SQL de production, laissé intact.
- `crates/mika-agent/src/prompt.rs:1120` — `hosting_ground_truth_line`, le motif de double registre repris en D2.
- mika#2291 — rendu HTML sortant, son kill-switch et son repli (M5, D3, RI4).
- mika#2126 — le contenu utilisateur ne se journalise pas (D6).
- mika#2131 — une observabilité qui journalise tout le monde ne distingue plus personne (D6).
- mika#2205 — un scan silencieusement inactif se lit comme un scan oisif (contrôles négatifs, Halte 3).
- mika#2290, mika#2292 — le même fait écrit deux fois derrière un `match` exhaustif, registre suivant l'axe persona (D2).
- mika#2293 — un réglage qu'on ne peut pas observer n'est pas un réglage (D6).
- mika#2323 — allowlist livrée vide, la résolution est de retirer la lecture (U6).
- mika#2023 — l'axe langue sur la surface persona, distincte de celle-ci (corps du ticket, condition d'arrêt (a)).
- mika#1749 — le flux self-unlink d'origine.
- mika#2025 — le ticket, corps et commentaire opérateur du 19/09.
