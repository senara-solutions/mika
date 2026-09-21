# mika#2247 — Le tenant famille tient son registre : typographie, langue, heure locale

**Ticket :** senara-solutions/mika#2247
**Type :** fix
**Tier concerné :** `family` (et `champion`, qui porte la même persona via
`CHAMPION_PERSONA_PLACEHOLDER`)
**Défaut de flotte visé :** tenant cloud grand-public, `MIKA_AGENT_TIER=family`,
modèle `z-ai/glm-5.2`

> **Note de re-grooming (2026-09-20, première passe).** Ce plan a été écrit le
> 18/09 et relu contre le code de HEAD le 20/09. Cinq corrections ont été
> portées, dont une qui change la conception : § 4a change le **site de vérité**
> de l'axe langue. Les quatre autres sont des dérives de référence et sont
> listées en § 8, avec ce qu'elles enseignent sur la façon d'ancrer un plan.
>
> **Note de re-grooming (2026-09-20, seconde passe).** Chaque site nommé par ce
> plan a été revérifié contre HEAD, un par un : les neuf em-dashes de
> `FAMILY_SOUL` et son sentinelle en dernière ligne, les trois assembleurs, les
> trois sites `strip_internal_tags`, la lecture `get_customer_config("timezone")`
> dans `load_agent_context`, le refus de `## Current Time` et le
> `section_count <= 5` du compact, la position 5e de la dernière garde de la
> famille. **Zéro dérive** — le code n'a pas bougé sous le plan en deux jours, et
> il n'y a donc rien à ajouter à § 8, qui recense des dérives et pas des lacunes.
>
> Ce qui a été trouvé est d'une autre nature : une **lacune d'instrumentation du
> plan lui-même**, révélée par la relecture de mika#2358 (déployé le 17/09, trois
> jours avant ce plan, dans le fichier même que § 4a propose de modifier). Ce
> ticket livre **deux** surfaces là où ce plan n'en prévoyait qu'une — le palier
> « valeur illisible » *et* l'événement de provenance — et c'est la seconde qui
> tranche la halte la plus probable de l'axe 2. Corrigé en § 4a (le modèle
> d'implémentation est nommé), en § 6 (cinquième signal + sonde 2 réécrite) et
> dans la Definition of Done.

---

## 1. Symptôme mesuré

Captures Telegram MikaSenara, 2026-09-06, trois fuites de registre sur un tenant
dont la persona prescrit « français natif », registre `tu`, zéro jargon :

1. **Em-dashes U+2014** — « Si tu parles de moi **—** je suis déjà là », « Pas
   besoin de rien connaître **—** tu me parles », « So **—** who are you ».
2. **Langue non tenue** — bascule EN↔FR dans un même fil (« So — who are you… »,
   « All good », puis « Bonjour ! Je suis Mika… »).
3. **Conscience temporelle** — « belle journée » envoyé le soir.

---

## 2. Ce que la lecture du code déplace — cinq rectifications

Le commentaire opérateur du 08/09 pose : *« Les trois fuites sont pilotées par le
modèle z-ai/glm-5.2, pas un bug de code : em-dashes (glm-5.2 en produit, rien
dans le persona ne l'interdit) ; flip FR↔EN (langue non épinglée par tenant) »*.
Cinq mesures corrigent ce diagnostic, et chacune change le remède.

### R1 — Une des trois occurrences em-dash est **copiée mot pour mot du constant**, pas produite par le modèle

`FAMILY_SOUL` (`crates/mika-common/src/home.rs`, constante `pub const
FAMILY_SOUL`) porte **neuf** lignes avec U+2014. L'une d'elles est dans la
section `## First-turn opening (référence — persona verbatim approuvé)` :

```
> Pas besoin de rien connaître — tu me parles comme à quelqu'un, en français,
```

C'est **textuellement** le symptôme n° 1 du ticket. La persona ne se contente
donc pas de « ne pas l'interdire » : elle le **prescrit**, sous la forme la plus
contraignante qu'un prompt connaisse — un exemple verbatim approuvé, placé dans
la section que le modèle reproduit à la première ouverture de chaque tenant, donc
à l'occurrence la plus visible de toute la population.

Conséquence sur le remède : « cadrer le style » par une consigne supplémentaire
mettrait une interdiction en concurrence avec un exemple, dans le même prompt.
Le premier geste n'est pas d'ajouter une règle, c'est de **cesser de prescrire**.

Les deux autres occurrences (« Si tu parles de moi — », « So — who are you »)
ne sont dans aucun constant : celles-là sont bien du style modèle. La cause est
donc **double**, et le ticket n'en voyait qu'une moitié — celle qui ne se répare
pas par une édition.

Note de portée : `DEFAULT_SOUL` porte aussi quatre em-dashes, et l'acceptance ne
parle que du tenant grand-public. Registre opérateur **hors périmètre**,
délibérément (§ 7).

### R2 — La langue **est déjà épinglée dans le prompt**, et ça n'a pas tenu

`FAMILY_SOUL` écrit « Tu réponds en **français** natif et chaleureux. » dans
`## Personnalité`, puis « Parle en français naturel, chaleureux, direct » dans
`## Style de communication`.

La prescription existe, en gras, deux fois, et la dérive a eu lieu quand même.
Donc « épingler la langue par tenant » — s'il s'agit d'une ligne de prompt de
plus — ajouterait une troisième formulation de ce qui a déjà échoué deux fois.
C'est exactement la classe que
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` borne
(mika#2120 : neuf récurrences sous enforcement par prompt contre zéro quand le
fait est posé par le code).

Ce qui manque n'est pas la consigne, ce sont **deux** choses : un axe **déclaré**
(la langue du tenant n'est un champ d'aucune configuration — recherche
exhaustive `lang|locale|language` sur `config.rs`, `config_keys.rs` et
`prompt.rs` : zéro) et une **moitié structurelle** qui refuse un tour dérivé.

### R3 — Le prompt ne pose **aucune** heure locale : il pose UTC et laisse convertir

`prompt::write_time_section` rend exactement :

```
## Current Time
UTC: 2026-09-06T20:14:03Z
User timezone: Asia/Singapore        ← seulement si déclarée
```

Pour dire « bonsoir » plutôt que « belle journée », le modèle doit faire **deux**
inférences non outillées : convertir UTC vers le fuseau, puis en déduire un
moment de la journée. Il n'y a pas de défaut de consigne ici — il n'y a pas de
fait posé. C'est la forme exacte du trou que mika#2290 a dû nommer sur
l'hébergement : *« il n'y avait rien à conditionner : il y avait un fait à poser
et une fabrication à empêcher »*.

Et le chemin compact (`build_compact_system_prompt`, ≤ 5 Ko,
`ProviderKind::MikaModel`) ne rend **aucune** section temporelle du tout. Ce
n'est pas un oubli : deux assertions le tiennent, et depuis le 18/09 elles sont
**plus** explicites qu'alors (§ 5a et § 8-B).

### R3b — Il y a **trois** assembleurs, pas deux, et le troisième est le plus suspect

`build_system_prompt`, `build_compact_system_prompt` et `build_silent_prompt`.
Le troisième appelle `write_time_section` et `write_runtime_section` — donc les
moitiés intention des axes 2 et 3 l'atteignent **sans threading nouveau**, les
deux fonctions étant partagées.

Il mérite d'être nommé plutôt que couvert par accident : les tours silencieux
(heartbeat, reflection, reminder) sont précisément ceux qui **ouvrent** un
échange sans message entrant. « Belle journée » envoyé le soir est bien plus
plausiblement un heartbeat proactif qu'une réponse — c'est le seul des trois
chemins qui salue sans qu'on lui ait parlé. Un plan qui ne le nommerait pas
livrerait AC3 sur le chemin où le symptôme a le moins de chances de naître.

Côté axe 1, les trois sites de sortie de § 3b couvrent le silencieux aussi : un
tour silencieux n'a pas de texte livré et passe obligatoirement par
`send_message` (site 3).

### R4 — Ce que le ticket propose en alternative n'est pas livrable dans ce dépôt

*« OU reconsidérer glm-5.2 pour les tenants family »* : le modèle d'un tenant est
dans son `config.toml`, écrit au provisionnement par `mika-cloud`. Et mika#1190
interdit tout changement de modèle sans `make calibrate-<role>` passant — or il
n'existe **aucune suite de calibration tier famille** (les suites couvrent
mika-dev, mika-arch, mika-qa, mika-orchestrator). Donc : geste côté cloud +
nouvelle suite de calibration = deux tickets de suivi (§ 7).

**Et c'est l'argument pour faire le présent travail d'abord, quelle que soit la
décision modèle** : les moitiés structurelles ci-dessous tiennent sous n'importe
quel modèle, alors qu'un échange de modèle ne ferme aucun des trois axes de façon
vérifiable et rouvre les trois le jour du modèle suivant.

### R5 — L'axe langue n'a pas besoin d'un nouveau mécanisme de configuration : il en existe un, et c'est le bon (correction du 20/09)

La version du 18/09 de ce plan proposait `MIKA_TENANT_LANGUAGE` (variable
d'environnement, lue une fois par process, non hot-swappable) plus une clé
`[locale].language` dans `identity.toml`, sur le modèle de `MIKA_DEPLOYMENT`
(mika#2290). **Quatre mesures réfutent ce choix**, et la quatrième est
éliminatoire.

1. **Le site existe déjà, et il porte déjà le voisin exact.** `customer_config`
   porte `timezone`, résolu dans `agent_loop::load_agent_context` par un
   `db.get_customer_config("timezone")` et posé sur `AgentContext` à côté de
   `soul_content`, `identity` et `core_memory`. L'axe 3 de ce plan lit **cette
   valeur-là**. Ajouter la langue à la même table, au même site de chargement,
   sur la même struct, est une ligne — contre un mécanisme complet (parse
   trois états, cache sur `AgentState`, section `identity.toml`, filetage
   depuis l'environnement du process).

2. **`SETTABLE_CONFIG_KEYS` *est* la surface d'outil.** `SetConfigTool::definition`
   construit son `enum` et sa description depuis cette constante, et
   `validate_config_value` valide par clé. Ajouter la langue là la rend
   réglable **par l'outil `set_config` déjà exposé au modèle** et par le
   `/config set` opérateur, sans écrire une ligne de prompt ni un outil.

3. **mika#2358 a déjà tranché ce choix de site, par écrit et pour une clé de
   même nature.** Son doc-comment (`config_keys.rs`) nomme la propriété
   mesurée : `customer_config` est *le seul site que rien ne réécrit au
   démarrage* — une annulation de row est levée par
   `revert_config_cancel_recurring_task` (mika#2271), et une édition
   d'`identity.toml` est exposée à la réconciliation des sections code-owned
   (mika#2330). Choisir `identity.toml` ici rejouerait un arbitrage déjà perdu.

4. **Éliminatoire : rien n'émet `MIKA_TENANT_LANGUAGE`, donc l'axe 2 serait
   entièrement inerte au déploiement.** La garde de § 4c ne s'arme **que** sur
   une langue déclarée ; sans déclaration, `Unknown`, rien n'est gardé. Poser
   la variable est un geste `mika-cloud`, hors de ce workspace — donc le tenant
   d'Al, **la seule population mesurée**, ne serait pas servi par cette
   livraison. C'est précisément la dépendance que mika#2290 a acceptée pour son
   signal `cloud`, mais mika#2290 pouvait se le permettre : sa garde 5d lit le
   texte sortant et ferme son p1 **sans** la variable. Ici la garde dépend de
   la valeur. La forme est la même, la conséquence est inverse.

   Avec `customer_config`, la langue se pose sur un tenant vivant **par une
   phrase dans la conversation** ou une commande opérateur, sans redéploiement
   de quoi que ce soit.

**Et le hot-swap n'est pas un bonus, c'est l'exigence.** « Parle-moi en
anglais » est une demande conversationnelle ordinaire. Un axe non
hot-swappable rendrait cette demande inexécutable — et mika#2358 a mesuré ce
que coûte une demande de réglage inexécutable : Mika promet une correction
qu'elle n'a aucun moyen d'appliquer, et la garde 5e a dû être écrite pour ça.

**Le risque du site, nommé plutôt que découvert.** Une garde armée par une
valeur que le modèle peut lui-même écrire n'est pas une garde contre un modèle
malveillant. Elle n'a jamais prétendu l'être : elle protège contre la **dérive**
— le tour qui bascule en EN alors que `fr` est posé. Changer délibérément la clé
est un **acte**, tracé par la ligne `audit_events` que `set_config` écrit déjà ;
basculer de langue en cours de fil est une dérive, et c'est elle qu'on refuse.
Même arbitrage que `timezone`, déjà réglable par le modèle depuis toujours.

---

## 3. Axe 1 — Typographie : zéro U+2014 dans la sortie famille (AC1)

Deux moitiés, parce que la cause est double (R1).

### 3a. Moitié intention — retirer la prescription de `FAMILY_SOUL`

Réécrire les neuf occurrences U+2014 de `FAMILY_SOUL` en ponctuation ASCII
simple. **Contrainte dure : on change la ponctuation, jamais un mot.** C'est un
constant approuvé par Vincent, dont une section est explicitement marquée
« persona verbatim approuvé » ; le diff doit être lisible comme une
dé-typographication et rien d'autre.

`DEFAULT_SOUL` n'est pas touché (R1, § 7).

Garde : `home::tests::mika2247_family_soul_carries_no_em_dash` — scan du constant,
zéro U+2014 / U+2013 / U+2026. Un test de constant et pas de comportement, parce
que la régression consisterait à **re-prescrire** : personne ne verrait rouge,
le modèle se remettrait simplement à recopier.

Effet de bord gratuit : le titre `# Mika — Compagnon personnel (famille)` est
précisément ce que le chemin compact rend comme `## Personality`
(`soul_content.lines().next()`), donc cette édition ferme aussi l'em-dash du
prompt compact sans y toucher.

**Précaution d'édition, vérifiée le 20/09 :** la dernière ligne de `FAMILY_SOUL`
est `<!-- MIKA_FAMILY_SOUL_MARKER -->`, le sentinelle de provisionnement lu par
`soul_has_family_marker` — l'un des deux axes de détection du garde de tier
(mika#1962). Une réécriture qui le déplacerait ou l'altérerait casserait ce
garde pour toute la population famille. Il ne porte aucun des trois caractères
visés, donc il ne doit tout simplement pas être touché ; un test l'épingle.

### 3b. Moitié structurelle — un normaliseur, pas une garde

**Substitution déterministe, et le choix du mécanisme est l'arbitrage central de
cet axe.** Une garde EndTurn (la forme de la famille 5c/5d/5e) dispose d'un
budget d'un seul re-prompt ; face à un modèle qui produit des em-dashes par
style, elle firerait à chaque tour, dépenserait son budget, et **laisserait tout
de même passer** — la forme littérale de ce que mika#2368 a dû rattraper par un
filet moteur. Un em-dash n'est pas une affirmation fausse qu'il faut faire
*réécrire* : c'est un défaut de **rendu**, dont la réparation correcte est
mécanique et préserve le sens. Une substitution ne peut pas échouer et ne coûte
aucun appel.

Règles, dans l'ordre, sur `—` (U+2014) et `–` (U+2013) :

| Contexte | → | Raison |
|---|---|---|
| début de ligne (après espaces éventuels) | `-` | tiret de liste ou de dialogue |
| espace des deux côtés | `,` (ou rien si le caractère précédent est déjà `,;:.!?`) | tiret d'apposition français |
| sans espace adjacente (`10—12`) | `-` | intervalle |

Plus `…` (U+2026) → `...`, que le commentaire opérateur nomme aussi.

Propriétés exigées : **idempotent** (la sortie ne contient plus aucun des trois
points de code, donc un second passage est un no-op) et **UTF-8 safe** par
itération sur `char` — jamais de découpe d'octets, `scripts/check-byte-slices.sh`
échouerait en CI.

Maison : `mika_common::text` (à côté de `safe_truncate`), pour que le gateway
puisse le lire un jour sans en écrire une seconde copie.

**Placement — un seul site par chemin, et l'ordre est porteur.** Le normaliseur
s'applique **immédiatement après `strip_internal_tags`**, aux trois sites qui
produisent le texte utilisateur :

| Site | Fonction | Couvre |
|---|---|---|
| extraction EndTurn | `agent_loop/mod.rs`, l'appel `strip_internal_tags(&response.text())` dont le résultat est `let mut text` | persistance **et** livraison, mode conversation / silent / team |
| tour de continuation | `agent_loop/mod.rs`, l'appel `strip_internal_tags(&resp.text())` dans `attempt_continuation_turn` | le résumé forcé au dépassement de pas |
| outil `send_message` | `tools/send_message.rs`, la liaison `let cleaned = …strip_internal_tags(text)` | les envois explicites |

*(Les trois sites sont désignés par leur expression, pas par un numéro de ligne :
voir § 8-A pour pourquoi.)*

Trois raisons pour cet ancrage précis :

- **Pas dans le gateway.** Il ignore la persona, et depuis mika#2291 il rend du
  HTML pour tout le monde : normaliser là toucherait le registre opérateur, que
  l'acceptance exclut.
- **Pas dans `server::handlers` (le site de mika#2136).** `handlers` envoie un
  texte que la boucle a **déjà persisté** ; y normaliser ferait diverger la base
  et le message reçu — et le résumé de compaction ré-enseignerait l'em-dash au
  tour suivant.
- **Après `strip_internal_tags`, avant les gardes.** `text` est `mut` au premier
  site et la garde d'ancrage mika#2037 le réécrit ; normaliser d'abord donne un
  seul texte à tout l'aval (gardes, persistance, livraison). Sur `send_message`,
  normaliser avant que `cleaned` ne soit capturé est **obligatoire** : le
  `DeliveryVerdict` de mika#2136 porte `cleaned` et son prédicat compare des
  textes par égalité — deux normalisations incohérentes rendraient une
  réparation méconnaissable et feraient partir une ligne « non reçu » à un
  utilisateur qui a reçu (le piège que le doc-comment de mika#2136 nomme mot
  pour mot).

**Portée : `PersonaProfile::Family` seule.** `ToolContext` porte déjà `tier`, et
`AgentTier::persona_profile()` est public — la garde 5d lit `tool_ctx.deployment`
par le même chemin, et `write_runtime_section` reçoit déjà un `persona:
PersonaProfile`. **Aucun threading nouveau.** Le croisement est un `match` sur
`PersonaProfile` sans `_ =>` (modèle mika#2290), pour que l'arrivée d'une persona
force une décision au lieu d'en hériter.

**Coût nommé :** le normaliseur ne distingue pas un bloc de code d'une prose. Le
tier famille n'en émet pas (sa persona interdit tout jargon) — donc on l'accepte
et on l'écrit, plutôt que d'ajouter un parseur de fences pour une population
vide.

---

## 4. Axe 2 — Langue tenue (AC2)

### 4a. L'axe déclaré : une clé `customer_config`, pas une variable de process

**Clé `language` dans `SETTABLE_CONFIG_KEYS`**, à côté de `timezone`, validée
par `validate_config_value` comme les autres. Trois états :

| Valeur | État | Effet |
|---|---|---|
| `fr` | `TenantLanguage::French` | fait posé + garde armée |
| `en` | `TenantLanguage::English` | idem |
| absente, vide | `Unknown` | **rien n'est posé, rien n'est gardé** |
| non reconnue | refusée **à la porte** par `validate_config_value` ; si présente en base malgré tout (écriture hors outil) → `Unknown` + `warn!` nommant la valeur entre guillemets | idem |

Le refus à la porte est ce que `validate_config_value` fait déjà pour
`chat_id` et `timezone` : un `set_config` avec une valeur hors domaine rend une
erreur au modèle, qui peut se corriger dans le tour. Le palier « illisible →
`Unknown` + WARN » subsiste pour l'écriture directe en base, comme mika#2358 le
fait pour ses deux clés.

**Trajectoire de lecture, calquée sur `timezone` :**
`db.get_customer_config("language")` dans `agent_loop::load_agent_context` →
champ sur `AgentContext` → `PromptContext` / `SilentPromptContext` → et un
paramètre de `run_loop` pour la garde, sur le modèle de `loaded_skill_names`
(mika#2355). C'est un filetage de champ, pas un nouveau mécanisme — et c'est le
**même** filetage quel que soit le site de vérité choisi, ce qui est précisément
pourquoi le choix du site se décide sur les quatre mesures de R5 et non sur le
coût du filetage. Vérifié contre HEAD : la lecture `timezone` est une ligne de
`load_agent_context`, le champ est déjà sur la struct, et il est déjà threadé aux
**trois** contextes d'appel.

**Le modèle d'implémentation est nommé, pas décrit — mika#2358, même fichier,
trois jours avant ce plan.** `PROACTIVE_DAILY_BUDGET_KEY` a exactement la forme
que cet axe demande, et la copier est plus court que la re-concevoir. Quatre
pièces à reprendre telles quelles :

| pièce mika#2358 | équivalent ici |
|---|---|
| `PROACTIVE_DAILY_BUDGET_KEY` (constante nommée dans `SETTABLE_CONFIG_KEYS`) | `TENANT_LANGUAGE_KEY` |
| bras dédié de `validate_config_value` avec message nommant le domaine | bras `{fr, en}` |
| `ResolvedProactiveBudget { budget, source }` + `ProactiveBudgetSource::{Config, Default}` avec son `as_str()` de format de fil | `ResolvedTenantLanguage` + sa provenance |
| `proactive_budget_invalid` (WARN) **et** `proactive_budget_resolved` (INFO dédupliqué) | § 6, et voir ci-dessous |

**La quatrième ligne est celle que la première passe de ce plan avait à moitié
manquée.** mika#2358 livre **deux** événements et non un : le WARN du palier
« illisible » *et* l'INFO de provenance. Ce plan ne prévoyait que le premier.
Or les deux répondent à des questions différentes — « quelqu'un a-t-il écrit une
valeur hors domaine ? » contre « quelle langue est réellement en vigueur pour ce
tenant ? » — et c'est la seconde qui sépare les deux causes d'un symptôme
persistant. Le doc-comment de `ProactiveBudgetSource` cite mika#2293 mot pour
mot sur ce point, et la raison vaut identiquement ici : *un réglage qu'on ne peut
pas observer n'est pas un réglage, c'est un espoir.*

**Pourquoi l'absence ne vaut pas « la langue de la persona ».** `FAMILY_SOUL`
prescrit le français en dur, et mika#2023 a nommé par écrit le prix de ce
codage : *« un champion anglophone obtient l'image miroir du bug pour lequel
mika#2023 a été ouvert »*. Faire de l'absence un français implicite écrirait ce
défaut à un second endroit — et le déduire du locale du compte est précisément ce
que Prime a tranché le 2026-09-09 (« un choix produit déguisé en défaut
technique »), arbitrage que mika#2290 a déjà reporté une fois.

Ensemble supporté : **exactement `{fr, en}`**. C'est la borne du détecteur
(§ 4c), et une valeur hors de cet ensemble est refusée plutôt que d'armer une
garde qui ne sait pas mesurer. Toute autre langue : § 7.

**Portée effective, dite plutôt que découverte.** `customer_config` est une
table de la base de l'agent, donc la clé est **par agent**. Les agents
d'ingénierie (mika-dev, mika-qa, mika-arch) ne la porteront pas : `Unknown`,
rien n'est gardé, comportement d'aujourd'hui mot pour mot. C'est le résultat
voulu, pas une lacune — l'acceptance vise le tenant grand-public.

### 4b. Moitié intention — le fait, pas la consigne

Une ligne dans `## Runtime` (le bloc déjà déclaré *ground truth*, déjà en amont
de Time / Channel / core-memory, et qui reçoit déjà `persona`) : la langue du
tenant, et la règle qu'un fil s'y tient quelle que soit la langue d'un message
entrant. En `Unknown` : **aucune ligne**, donc le comportement d'aujourd'hui.

R2 dit pourquoi cette moitié ne suffit pas et ne prétend pas suffire.

### 4c. Moitié structurelle — garde EndTurn `response_language_drift`

Elle reprend la forme, le budget `intent_guard_retries` (un re-prompt) et la
télémétrie `guard.*` de la famille #953, et se place **après la dernière garde
de cette famille** — aujourd'hui `unactioned_frequency_promise` (mika#2358),
étiquetée 5e. L'ordinal exact est à relire à l'implémentation plutôt qu'à
recopier d'ici : ces positions bougent (§ 8-C). Non sautée par
`skip_remaining_guards` (#1178), pour la raison littérale de 5c/5d/5e : une revue
de PR postée n'autorise pas à répondre dans la mauvaise langue.

Détecteur : discriminant par **mots-fonction** sur deux listes fermées FR/EN
(`le la les de des un une et est dans que pour` / `the a an of and is in that
for to`), sans dépendance nouvelle. Deux seuils : minimum de tokens et marge
minimale entre les deux scores.

**Fail-open, et c'est la propriété qui décide de la faisabilité.** Un texte trop
court, purement emoji, un nom propre seul, du code → `Undetermined` → **la garde
ne fire pas**. « Bonjour 🌸 », « OK », « All good » sont indécidables et doivent
le rester : un faux positif ferait re-prompter un tour honnête, et sur un tenant
grand-public le prix est une réponse retardée pour rien.

**Ce que cette garde ne fait pas, écrit ici plutôt que découvert :** un budget
d'un re-prompt ne *garantit* pas AC2. Il le **borne** et rend le résidu
comptable — `guard.response_language_drift_uncorrected` (WARN, régime attendu
zéro), exactement le geste que 5d et 5e font pour leur propre population
résiduelle. Le mot « tenue » de l'acceptance est donc livré comme *bornée et
mesurée*, pas comme *impossible*. Si la mesure post-déploiement montre un résidu
non négligeable, le remède est un filet moteur (forme mika#2368), pas un second
re-prompt — **et c'est un ticket, pas un réglage.**

---

## 5. Axe 3 — Salutations cohérentes avec l'heure locale (AC3)

### 5a. Moitié intention — poser l'heure locale calculée

`write_time_section` gagne, quand le fuseau est **résolu** :

```
Local time (Asia/Singapore): 2026-09-06 20:14 / Sunday evening
```

(séparateur ASCII, voir AC1 — la chaîne réelle ne porte pas d'em-dash.)

`chrono-tz` est déjà dépendance de `mika-agent` et le fuseau est déjà résolu
(`db.get_customer_config("timezone")` dans `load_agent_context`). **Aucune
dépendance, aucune requête nouvelle.** `db.rs` porte déjà trois usages du motif
`timezone.parse::<Tz>().unwrap_or(chrono_tz::UTC)` — la conversion est une
convention maison existante, pas une invention de ce plan.

Parse en **deux temps**, parce que les deux formes circulent : `chrono_tz::Tz`
(`Asia/Singapore`), puis `FixedOffset` (`+08:00` — la forme de fixtures de test
de `prompt.rs`, que `validate_config_value` refuse aujourd'hui à l'écriture mais
que des lignes anciennes peuvent porter).

Découpage du moment de la journée — un paramètre, donc nommé plutôt que dilué :
`morning 05–11`, `afternoon 12–17`, `evening 18–22`, `night 23–04`.

**Fuseau absent ou illisible → on le dit, on ne le tait pas.** La ligne devient
une déclaration d'ignorance plus l'interdiction d'employer une salutation
horodatée. Laisser le vide est ce qui a produit « belle journée » le soir : le
modèle a répondu sur son prior parce qu'aucune section ne disait l'heure — la
leçon mika#2290 à la lettre.

**Chemin compact : on ne rend rien, et on suit mika#2290 au lieu d'en diverger.**
Trois faits le refusent, et le troisième est le seul qui compte.

1. Le test de forme du compact **refuse explicitement** `## Current Time`, et
   l'assertion de compte de sections juste au-dessus énumère en commentaire les
   sections admises. Rendre la ligne demande donc de modifier **deux décisions
   épinglées** — le prix qu'un plan doit annoncer, pas découvrir à
   l'implémentation. Depuis le 18/09 ce commentaire porte en plus une phrase
   explicite : *« Raising it again means naming the section, its ticket, and
   what it guarantees »* (§ 8-B). L'argument est donc plus fort qu'alors, pas
   plus faible.
2. Le carve-out mika#2290 est écrit sur le site lui-même, avec son raisonnement :
   *« it withholds the intent half from this path, never the protection: the 5d
   guard reads outgoing text, not the prompt »*.
3. **Ce raisonnement s'applique ici mot pour mot.** La garde de § 5b lit le
   texte sortant, pas le prompt — donc elle protège le chemin compact que la
   ligne y soit rendue ou non. La divergence coûtait deux décisions épinglées
   pour un bénéfice que la moitié structurelle fournit déjà.

Le coût est nommé et il est réel : sur MikaModel le modèle n'a pas le fait posé,
donc la garde de § 5b y travaille seule, sans la moitié intention. C'est
exactement le régime que mika#2290 a accepté pour l'hébergement, et il se joint
au même suivi (mika#1925). Épinglé par
`mika2247_compact_prompt_omits_the_local_time_line`, rédigé sur le modèle du
test frère — **comme décision, pas comme oubli**, pour que le prochain lecteur
trouve l'argument au lieu de le refaire.

### 5b. Moitié structurelle — garde `time_of_day_greeting_mismatch`

Même forme, même budget, placée juste après celle de § 4c. Ensemble **fermé et
étroit** de formules explicitement horodatées, bilingue (« belle journée »,
« bonne journée », « bonjour », « bonsoir », « bonne nuit », « good morning »,
« good evening », « good night »), et elle ne fire que si (a) une heure locale
est **connue** et (b) la formule nomme un autre moment que celui calculé.

Elle est présente parce qu'AC3 est un critère d'acceptance et qu'une livraison
prompt-seule ne satisfait pas la règle de la maison. Elle est **étroite** parce
que le fait est désormais *calculé* et non inféré : le profil de risque est très
inférieur à celui de l'axe 2, la garde n'est plus le mécanisme principal mais un
filet. Fail-open sur fuseau inconnu — sans heure locale, il n'y a rien à
contredire.

---

## 6. Contrats de vérification

### Tests

| Axe | Test | Ce qu'il refuse |
|---|---|---|
| 1 | `home::tests::mika2247_family_soul_carries_no_em_dash` | la **re-prescription** dans le constant |
| 1 | `home::tests::mika2247_family_soul_marker_is_intact` | une réécriture qui déplace le sentinelle de tier (mika#1962) |
| 1 | `text::tests::mika2247_normalizer_is_idempotent_and_utf8_safe` | une boucle ou une panique sur multi-octets |
| 1 | `text::tests::mika2247_the_three_measured_occurrences` | les trois chaînes du ticket, vérifiées en sortie |
| 1 | `prompt/agent_loop` : contrôle négatif **opérateur** | que le normaliseur morde hors tier famille |
| 1 | garde structurelle : scan de source, `strip_internal_tags` suivi du normaliseur aux trois sites | un quatrième site muet — halte, pas d'allowlist |
| 2 | `config_keys::tests::mika2247_language_three_states` | qu'une absence arme quoi que ce soit |
| 2 | `config_keys::tests::mika2247_unknown_value_is_refused_at_the_door` | une valeur hors `{fr, en}` acceptée par `set_config` |
| 2 | `config_keys::tests::mika2247_resolved_source_is_a_wire_format` | deux orthographes d'une provenance, qui couperaient une population en deux sans le dire (modèle `ProactiveBudgetSource::as_str`) |
| 2 | `guards::tests::mika2247_short_text_is_undetermined` | le faux positif sur « OK » / « Bonjour 🌸 » |
| 2 | `tests/eval/doctrine_regressions/` : le fil mesuré (EN puis FR), **plus** un contrôle négatif par état | une garde qui fire en `Unknown` |
| 3 | `prompt::tests::mika2247_local_time_is_computed_not_inferred` | le retour à UTC seul |
| 3 | `prompt::tests::mika2247_unknown_timezone_says_so` | le vide silencieux |
| 3 | `prompt::tests::mika2247_compact_prompt_omits_the_local_time_line` | la réouverture du carve-out compact **comme si c'était un oubli** |
| 3 | `prompt::tests::mika2247_silent_prompt_carries_the_local_time_line` | la salutation proactive laissée sans heure (R3b) |

**Un contrôle négatif par terme, jamais un seul pour tous** (leçon mika#2277) :
un test qui neutraliserait les trois conditions à la fois passerait au vert sur
un prédicat n'en lisant qu'une.

### Surfaces opérateur

`$MIKA_SPIRIT_LOG_FILE` :

- `guard.response_language_drift` (WARN) — régime attendu **faible mais non nul** ;
  chaque ligne est un tour rattrapé. Un flot soutenu sur un même tenant dit que
  la moitié intention n'atteint pas ce chemin : **vérifier le carve-out compact
  avant de toucher au détecteur**.
- `guard.response_language_drift_uncorrected` (WARN) — **zéro attendu**. C'est la
  seule population que la garde ne ferme pas ; sans cet événement elle serait
  indistinguable d'un tour sain.
- `guard.time_of_day_greeting_mismatch` (+ `_uncorrected`) — même lecture.
- `tenant_language_unrecognized_value` (WARN) — **zéro attendu** ; ne peut venir
  que d'une écriture hors outil, `validate_config_value` refusant à la porte.
  Nomme la valeur entre guillemets, pour qu'un espace parasite se voie.
- `tenant_language_resolved` (INFO — champs `agent_id`, `language`, `source`) —
  **la réponse à « quelle langue est en vigueur pour ce tenant ? », sans lire la
  base.** Modèle et raison : `proactive_budget_resolved` (mika#2358), lui-même
  calqué sur `llm_budget_resolved` (mika#2293). Deux valeurs de `source` et
  elles nomment deux remèdes opposés : `config` → la consigne est en vigueur,
  donc un symptôme survivant est imputable à autre chose (et c'est § 5a du
  chemin compact qu'il faut lire en premier) ; `default` → l'écriture n'a pas
  atterri, la cause est dans `set_config` ou dans le tour qui aurait dû
  l'appeler, **pas** dans la garde. Dédupliqué sur le couple résolu, comme ses
  deux aînés : une répétition à l'identique est tue, un **changement** est
  ré-émis — c'est ce qui rend « l'utilisateur a demandé l'anglais en cours de
  fil » lisible sur une ligne.

En base : `SELECT * FROM audit_events WHERE tool_name = 'set_config';` répond à
« quand la langue de ce tenant a-t-elle été posée, et par quelle session ? » —
`set_config` écrit déjà cette ligne, rien n'est ajouté.

Le normaliseur typographique **n'émet rien**, délibérément : il tourne sur tous
les tours famille, une ligne par tour serait du churn que la doctrine mika#2131
borne, et son effet est vérifiable par test plutôt que par grep.

### Sondes post-déploiement, et leurs haltes

1. **AC1, 48 h.** Rejouer les trois échanges mesurés sur un tenant famille :
   zéro U+2014 en sortie. *Halte* : un em-dash qui réapparaît alors que les tests
   sont verts signifie un **quatrième** chemin de sortie — l'établir, ne pas
   élargir le normaliseur au gateway par réflexe.
2. **AC2.** Poser `language = fr` sur le tenant mesuré **par la conversation**
   (c'est le chemin qu'on teste, et c'est le seul qui atteste que la clé est
   réglable sans redéploiement), puis lire la provenance avant de rejouer quoi
   que ce soit :
   ```bash
   grep tenant_language_resolved "$MIKA_SPIRIT_LOG_FILE" \
     | jq 'select(.agent_id == "<tenant>") | {language, source}'
   ```
   Attendu : `{"language": "fr", "source": "config"}`. Rejouer ensuite le fil
   bilingue mesuré. **Trois haltes, et l'ordre est celui du coût.**
   *Halte 2a — `source: "default"`* : l'écriture n'a pas atterri. **Ne pas
   toucher à la garde ni au détecteur** — la cause est dans `set_config` ou dans
   le tour qui aurait dû l'appeler, et aucun réglage du seuil ne la corrigera.
   *Halte 2b — aucune ligne du tout alors que le tenant a tourné* : le binaire
   servi est antérieur au correctif (classe mika#2340) ; établir le déploiement
   avant toute conclusion sur le texte. *Halte 2c — `source: "config"` et la
   bascule persiste avec `guard.response_language_drift` **vide*** : le tour
   passe par un assembleur que la garde ne traverse pas ; **lire d'abord le
   carve-out compact de § 5a**, puis établir lequel.

   *(Cette sonde demandait en première passe de « vérifier la ligne
   `## Runtime` » — ce qui exige d'armer `MIKA_LOG_LLM_BODIES` **sur
   mika-spirit** et de le redémarrer, mika#2220. Un grep répond à la même
   question sans toucher au service, et c'est précisément ce que l'événement de
   provenance achète.)*
3. **AC3.** Une salutation le soir, fuseau déclaré, puis la même **sans** fuseau.
   Le second cas doit produire une salutation non horodatée, pas un pari.
4. **Contrôle négatif opérateur.** Sur la station de Vincent (tier `default`,
   aucune clé `language`) : em-dashes préservés, aucune garde armée. *Halte* :
   toute garde qui fire côté opérateur est une fuite de portée — désarmer et
   réparer le croisement persona, pas le seuil.

---

## 7. Hors périmètre, délibérément

- **`DEFAULT_SOUL` et le registre opérateur.** Quatre em-dashes, conservés :
  l'acceptance parle du tenant grand-public, et la ponctuation soignée est le
  registre que Vincent a choisi pour lui-même.
- **Le changement de modèle** (`z-ai/glm-5.2` → autre) pour les tenants famille.
  Deux tickets : (a) le geste de provisionnement, côté `mika-cloud`, hors de ce
  workspace ; (b) **une suite de calibration tier famille**, qui n'existe pas, et
  sans laquelle mika#1190 interdit l'échange. R4 dit pourquoi le présent travail
  ne l'attend pas.
- **Toute langue hors `{fr, en}`.** Le détecteur ne sait pas les mesurer ; elles
  sont refusées à l'écriture, donc le tenant reste au comportement d'aujourd'hui.
  Suivi.
- **Le prompt compact et mika#1925.** Le carve-out ne bouge pas : § 5a explique
  pourquoi l'axe 3 le **suit** au lieu d'y déroger, et la moitié intention y
  reste retenue — protection assurée par la garde de § 5b, qui lit le texte
  sortant. Même suivi que mika#2290.
- **mika#2245** (défaut-racine de contexte) et le ticket frère « boilerplate ».
  Le ticket le pose lui-même dans sa section *Portée* — trois clusters distincts.
- **Un filet moteur pour la langue** (forme mika#2368). Conditionné à la mesure
  du résidu `_uncorrected` : instruire avant de mesurer serait construire sur une
  hypothèse.
- **Le `curator_review` et le heartbeat comme producteurs de salutations.**
  mika#2358 vient de borner les réveils proactifs et de taire le curateur sur
  persona famille. R3b nomme le tour silencieux comme le chemin le plus
  plausible du symptôme n° 3 et le couvre par `write_time_section` partagée ;
  changer la **cadence** de ces tours est le périmètre de mika#2358, pas celui-ci.

---

## 8. Dérives relevées au re-grooming du 20/09, et ce qu'elles enseignent

Quatre écarts entre le plan du 18/09 et le code de HEAD, en deux jours. Ils sont
listés parce que trois d'entre eux sont des **défauts d'ancrage** qui se
reproduiraient sans la correction de forme correspondante.

**A. Les numéros de ligne ont tous bougé.** `prompt.rs` a pris ~240 lignes,
`home.rs` ~57, `agent_loop/mod.rs` ~70. Les dix références `fichier:ligne` du
plan étaient fausses. **Correction de forme :** ce plan désigne désormais les
sites par nom de fonction, de constante ou d'expression. Un numéro de ligne dans
un plan de grooming a une demi-vie de quelques jours dans ce dépôt ; un nom de
fonction survit aux refactors qui comptent.

**B. Le carve-out compact s'est resserré.** L'assertion de compte de sections est
passée de 4 à 5 (mika#1925 y a ajouté `## Stopped Topics`), et le commentaire
porte maintenant une exigence explicite pour toute augmentation. Le plan citait
« 4 » comme un argument ; le chiffre était faux et **l'argument est renforcé**,
pas affaibli. Corrigé en § 5a.

**C. La position de garde 5e est occupée.** mika#2358
(`unactioned_frequency_promise`) l'a prise. Le plan réservait 5e et 5f. Corrigé
en § 4c, et la leçon est écrite sur place : un ordinal de garde est un numéro de
file, pas une identité — le nommer dans un plan invite à le recopier périmé.

**D. Le site de configuration proposé était le mauvais** — c'est la correction de
conception, développée en R5 et § 4a, et la seule qui change ce que le plan
demande d'écrire.

---

## Definition of Done

- [ ] `FAMILY_SOUL` ne porte plus aucun U+2014 / U+2013 / U+2026, **aucun mot
      changé**, sentinelle `MIKA_FAMILY_SOUL_MARKER` intacte, et deux tests de
      constant le refusent en retour.
- [ ] `mika_common::text` expose le normaliseur typographique, idempotent et
      UTF-8 safe, appliqué aux **trois** sites de § 3b, scopé
      `PersonaProfile::Family` par un `match` sans `_ =>`.
- [ ] Une garde structurelle refuse un quatrième site de sortie non normalisé.
- [ ] `language` est dans `SETTABLE_CONFIG_KEYS`, validée par
      `validate_config_value` (`{fr, en}` seuls acceptés), résolue en trois
      états, lue dans `load_agent_context` à côté de `timezone`, filetée vers
      `PromptContext` / `SilentPromptContext` et vers `run_loop`.
- [ ] `## Runtime` porte la langue déclarée ; **rien** en `Unknown`.
- [ ] `tenant_language_resolved` émis (INFO, dédupliqué sur le couple résolu),
      portant `source` à deux valeurs — sur le modèle de
      `proactive_budget_resolved` (mika#2358), et **indépendant de tout réglage
      de télémétrie** : c'est un événement de configuration, qui doit rester
      lisible précisément quand on a réduit le bruit.
- [ ] Garde `response_language_drift` : un re-prompt, fail-open sur indécidable,
      résidu nommé par `_uncorrected`, non sautée par `skip_remaining_guards`.
- [ ] `write_time_section` pose l'heure locale et le moment de la journée
      calculés, parse `Tz` **et** `FixedOffset`, et **dit** l'absence de fuseau.
- [ ] Les **trois** assembleurs sont traités nommément (R3b) : full et silencieux
      portent la ligne par `write_time_section` partagée ; le compact ne la porte
      pas, et un test l'épingle **comme décision** avec l'argument de § 5a.
- [ ] Garde `time_of_day_greeting_mismatch`, ensemble fermé bilingue, fail-open
      sans heure connue.
- [ ] Contrôle négatif opérateur vert : aucun des trois axes ne mord hors famille.
- [ ] Un contrôle négatif **par terme** fail-safe, pas un global.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt` verts ;
      `scripts/check-byte-slices.sh` vert.
- [ ] Root `CLAUDE.md` documente la clé `language` (trois états, réglable par
      `set_config` et `/config set`, hot-swappable) et les **cinq** signaux de
      grep, dans le voisinage de mika#2358 — un opérateur qui cherche « comment
      épingler la langue d'un tenant » cherche là où il a trouvé « comment
      borner ses messages proactifs ».
- [ ] `crates/mika-agent/CLAUDE.md` documente les deux nouvelles gardes dans la
      table fabrication/registre, et `crates/mika-common/CLAUDE.md` le
      normaliseur dans § *Text*.

## Acceptance criteria

Reprises verbatim du corps du ticket, avec la borne de livraison de chacune.

- **AC1 — Zéro em-dash U+2014 dans la sortie du tenant grand-public
  (ASCII/registre simple).** Fermée des deux côtés : la prescription est retirée
  du constant (§ 3a) et la production modèle est normalisée mécaniquement à la
  frontière (§ 3b). Une substitution ne dépend pas de la coopération du modèle,
  donc AC1 est livrée comme **garantie** sur les trois sites recensés — et la
  garde structurelle est ce qui rend un quatrième site visible au lieu de muet.

- **AC2 — Langue tenue (une seule par fil, selon l'utilisateur/config).**
  Livrée comme **bornée et mesurée**, pas comme garantie, et § 4c dit pourquoi :
  la langue d'un texte ne se corrige pas mécaniquement, donc le mécanisme est un
  re-prompt à budget un, dont le résidu est compté par
  `guard.response_language_drift_uncorrected`. Le « selon la config » est la clé
  `customer_config` `{fr, en}` — **posable sur un tenant vivant sans
  redéploiement**, ce qui est ce qui rend AC2 livrable ici plutôt que suspendue à
  un ticket `mika-cloud` (R5). Le « selon l'utilisateur » reste le comportement
  d'aujourd'hui en `Unknown`, à dessein (§ 4a).

- **AC3 — Salutations cohérentes avec l'heure locale du tenant.** L'heure locale
  et le moment de la journée cessent d'être une inférence : ils sont calculés et
  posés (§ 5a), sur les deux assembleurs qui servent un tour famille — y compris
  le silencieux, qui est celui où la salutation proactive naît (R3b). Une
  incohérence résiduelle est rattrapée une fois par la garde de § 5b. Deux
  bornes dites plutôt que découvertes : fuseau non déclaré, la cohérence n'est
  pas asserted — l'ignorance est dite et la salutation horodatée interdite, ce
  qui est **vrai** plutôt que deviné ; et sur le chemin compact (MikaModel) la
  garde travaille seule, sans moitié intention, régime hérité de mika#2290 et
  joint à son suivi.
