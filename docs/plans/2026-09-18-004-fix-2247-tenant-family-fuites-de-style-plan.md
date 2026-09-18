# mika#2247 — Le tenant famille tient son registre : typographie, langue, heure locale

**Ticket :** senara-solutions/mika#2247
**Type :** fix
**Tier concerné :** `family` (et `champion`, qui porte la même persona via
`CHAMPION_PERSONA_PLACEHOLDER`)
**Défaut de flotte visé :** tenant cloud grand-public, `MIKA_AGENT_TIER=family`,
modèle `z-ai/glm-5.2`

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

## 2. Ce que la lecture du code déplace — trois rectifications

Le commentaire opérateur du 08/09 pose : *« Les trois fuites sont pilotées par le
modèle z-ai/glm-5.2, pas un bug de code : em-dashes (glm-5.2 en produit, rien
dans le persona ne l'interdit) ; flip FR↔EN (langue non épinglée par tenant) »*.
Trois mesures corrigent ce diagnostic, et chacune change le remède.

### R1 — Une des trois occurrences em-dash est **copiée mot pour mot du constant**, pas produite par le modèle

`FAMILY_SOUL` (`crates/mika-common/src/home.rs:726-778`) porte **neuf** lignes
avec U+2014. L'une d'elles est la ligne 769, dans la section
`## First-turn opening (référence — persona verbatim approuvé)` :

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

`FAMILY_SOUL:734` : « Tu réponds en **français** natif et chaleureux. »
`FAMILY_SOUL:742` : « Parle en français naturel, chaleureux, direct ».

La prescription existe, en gras, deux fois, et la dérive a eu lieu quand même.
Donc « épingler la langue par tenant » — s'il s'agit d'une ligne de prompte de
plus — ajouterait une troisième formulation de ce qui a déjà échoué deux fois.
C'est exactement la classe que
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` borne
(mika#2120 : neuf récurrences sous enforcement par prompt contre zéro quand le
fait est posé par le code).

Ce qui manque n'est pas la consigne, ce sont **deux** choses qui n'existent nulle
part : un axe **déclaré** (la langue du tenant n'est un champ d'aucune
configuration — recherche exhaustive `lang|locale|language` sur `config.rs` et
`prompt.rs` : zéro), et une **moitié structurelle** qui refuse un tour dérivé.

### R3 — Le prompt ne pose **aucune** heure locale : il pose UTC et laisse convertir

`prompt::write_time_section` (`prompt.rs:1162-1169`) rend exactement :

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

Et le chemin compact (`build_compact_system_prompt`, ≤ 5 Ko, `ProviderKind::MikaModel`)
ne rend **aucune** section temporelle du tout. Ce n'est pas un oubli : deux
assertions le tiennent (`prompt.rs:4422` refuse littéralement `## Current Time`,
et le `section_count <= 4` juste au-dessus énumère en commentaire les quatre
sections admises). § 5a en tire la conséquence, qui n'est pas celle qu'on croit.

### R3b — Il y a **trois** assembleurs, pas deux, et le troisième est le plus suspect

`build_system_prompt` (1219), `build_compact_system_prompt` (1624) et
`build_silent_prompt` (1738). Le troisième appelle `write_time_section` (1764)
et `write_runtime_section` (1752) — donc les moitiés intention des axes 2 et 3
l'atteignent **sans threading nouveau**, les deux fonctions étant partagées.

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

### 3b. Moitié structurelle — un normaliseur, pas une garde

**Substitution déterministe, et le choix du mécanisme est l'arbitrage central de
cet axe.** Une garde EndTurn (la forme des positions 5c/5d) dispose d'un budget
d'un seul re-prompt ; face à un modèle qui produit des em-dashes par style, elle
firerait à chaque tour, dépenserait son budget, et **laisserait tout de même
passer** — la forme littérale de ce que mika#2368 a dû rattraper par un filet
moteur. Un em-dash n'est pas une affirmation fausse qu'il faut faire *réécrire* :
c'est un défaut de **rendu**, dont la réparation correcte est mécanique et
préserve le sens. Une substitution ne peut pas échouer et ne coûte aucun appel.

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

| Site | Fichier | Couvre |
|---|---|---|
| extraction EndTurn | `agent_loop/mod.rs:1514` | persistance **et** livraison, mode conversation / silent / team |
| tour de continuation | `agent_loop/mod.rs:627` | le résumé forcé au dépassement de pas |
| outil `send_message` | `tools/send_message.rs:48` (`cleaned`) | les envois explicites |

Trois raisons pour cet ancrage précis :

- **Pas dans le gateway.** Il ignore la persona, et depuis mika#2291 il rend du
  HTML pour tout le monde : normaliser là toucherait le registre opérateur, que
  l'acceptance exclut.
- **Pas dans `server::handlers` (le site de mika#2136).** `handlers` envoie un
  texte que la boucle a **déjà persisté** ; y normaliser ferait diverger la base
  et le message reçu — et le résumé de compaction ré-enseignerait l'em-dash au
  tour suivant.
- **Après `strip_internal_tags`, avant les gardes.** `text` est `mut` à 1514 et
  la garde d'ancrage mika#2037 le réécrit ; normaliser d'abord donne un seul
  texte à tout l'aval (gardes, persistance, livraison). Sur `send_message`,
  normaliser avant que `cleaned` ne soit capturé est **obligatoire** : le
  `DeliveryVerdict` de mika#2136 porte `cleaned` et son prédicat compare des
  textes par égalité — deux normalisations incohérentes rendraient une réparation
  méconnaissable et feraient partir une ligne « non reçu » à un utilisateur qui a
  reçu (le piège que le doc-comment de mika#2136 nomme mot pour mot).

**Portée : `PersonaProfile::Family` seule.** `ToolContext` porte déjà `tier`
(`tools/mod.rs:114`) et `AgentTier::persona_profile()` est public — la garde 5d
lit `tool_ctx.deployment` par le même chemin. **Aucun threading nouveau.**
Le croisement est un `match` sur `PersonaProfile` sans `_ =>` (modèle mika#2290),
pour que l'arrivée d'une persona force une décision au lieu d'en hériter.

**Coût nommé :** le normaliseur ne distingue pas un bloc de code d'une prose. Le
tier famille n'en émet pas (sa persona interdit tout jargon) — donc on l'accepte
et on l'écrit, plutôt que d'ajouter un parseur de fences pour une population
vide.

---

## 4. Axe 2 — Langue tenue (AC2)

### 4a. L'axe déclaré, absent aujourd'hui

`MIKA_TENANT_LANGUAGE`, plus la clé per-agent `[locale].language` dans
`identity.toml`. Trois états, **même forme que `MIKA_DEPLOYMENT`** (mika#2290) :
lu une fois par process à `server::init_agent`, mis en cache sur `AgentState`,
fileté vers `PromptContext` et `ToolContext`, **non hot-swappable**, à poser dans
l'EnvironmentFile / ConfigMap **avant** le premier démarrage.

| Valeur | État | Effet |
|---|---|---|
| `fr` | `TenantLanguage::French` | fait posé + garde armée |
| `en` | `TenantLanguage::English` | idem |
| absente, vide, non reconnue | `Unknown` (+ `warn!` nommant la valeur entre guillemets si non vide) | **rien n'est posé, rien n'est gardé** |

**Pourquoi l'absence ne vaut pas « la langue de la persona ».** `FAMILY_SOUL`
prescrit le français en dur, et mika#2023 a nommé par écrit le prix de ce
codage : *« un champion anglophone obtient l'image miroir du bug pour lequel
mika#2023 a été ouvert »*. Faire de l'absence un français implicite écrirait ce
défaut à un second endroit — et le déduire du locale du compte est précisément ce
que Prime a tranché le 2026-09-09 (« un choix produit déguisé en défaut
technique »), arbitrage que mika#2290 a déjà reporté une fois.

Ensemble supporté : **exactement `{fr, en}`**. C'est la borne du détecteur
(§ 4c), et une valeur hors de cet ensemble tombe en `Unknown` plutôt que d'armer
une garde qui ne sait pas mesurer. Toute autre langue : § 7.

### 4b. Moitié intention — le fait, pas la consigne

Une ligne dans `## Runtime` (le bloc déjà déclaré *ground truth*, déjà en amont
de Time / Channel / core-memory) : la langue du tenant, et la règle qu'un fil s'y
tient quelle que soit la langue d'un message entrant. En `Unknown` : **aucune
ligne**, donc le comportement d'aujourd'hui, mot pour mot.

R2 dit pourquoi cette moitié ne suffit pas et ne prétend pas suffire.

### 4c. Moitié structurelle — garde EndTurn `response_language_drift`, position 5e

Position 5e, juste après 5d, dont elle reprend la forme, le budget
`intent_guard_retries` (un re-prompt) et la télémétrie `guard.*` de la famille
#953. Non sautée par `skip_remaining_guards` (#1178), pour la raison littérale de
5c/5d : une revue de PR postée n'autorise pas à répondre dans la mauvaise langue.

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
zéro), exactement le geste que 5d fait pour sa propre population résiduelle.
Le mot « tenue » de l'acceptance est donc livré comme *bornée et mesurée*, pas
comme *impossible*. Si la mesure post-déploiement montre un résidu non
négligeable, le remède est un filet moteur (forme mika#2368), pas un second
re-prompt — **et c'est un ticket, pas un réglage.**

---

## 5. Axe 3 — Salutations cohérentes avec l'heure locale (AC3)

### 5a. Moitié intention — poser l'heure locale calculée

`write_time_section` gagne, quand le fuseau est **résolu** :

```
Local time (Asia/Singapore): 2026-09-06 20:14 — Sunday evening
```

(sans em-dash dans la chaîne réelle, voir AC1 : ` / `.)

`chrono-tz` est déjà dépendance de `mika-agent` (`Cargo.toml:49`) et le fuseau est
déjà résolu (`db.get_customer_config("timezone")`, `agent_loop/mod.rs:434`).
**Aucune dépendance, aucune requête nouvelle.**

Parse en **deux temps**, parce que les deux formes circulent : `chrono_tz::Tz`
(`Asia/Singapore`), puis `FixedOffset` (`+08:00` — la forme des tests
`prompt.rs:2759` et `2894`, que `config_keys.rs:27` refuse aujourd'hui à
l'écriture mais que des lignes anciennes peuvent porter).

Découpage du moment de la journée — un paramètre, donc nommé plutôt que dilué :
`morning 05–11`, `afternoon 12–17`, `evening 18–22`, `night 23–04`.

**Fuseau absent ou illisible → on le dit, on ne le tait pas.** La ligne devient
une déclaration d'ignorance plus l'interdiction d'employer une salutation
horodatée. Laisser le vide est ce qui a produit « belle journée » le soir : le
modèle a répondu sur son prior parce qu'aucune section ne disait l'heure — la
leçon mika#2290 à la lettre.

**Chemin compact : on ne rend rien, et on suit mika#2290 au lieu d'en diverger.**
C'est une rectification d'une version antérieure de ce plan, qui proposait d'y
rendre la ligne en « assumant la divergence ». Trois faits la refusent, et le
troisième est le seul qui compte.

1. Le compact **refuse explicitement** `## Current Time` (`prompt.rs:4422`), et
   le `section_count <= 4` au-dessus énumère les quatre sections admises. Rendre
   la ligne demande donc de modifier **deux décisions épinglées** — le prix
   qu'un plan doit annoncer, pas découvrir à l'implémentation.
2. Le carve-out mika#2290 est écrit sur le site lui-même, avec son raisonnement :
   *« it withholds the intent half from this path, never the protection: the 5d
   guard reads outgoing text, not the prompt »*.
3. **Ce raisonnement s'applique ici mot pour mot.** La garde 5f (§ 5b) lit le
   texte sortant, pas le prompt — donc elle protège le chemin compact que la
   ligne y soit rendue ou non. La divergence coûtait deux décisions épinglées
   pour un bénéfice que la moitié structurelle fournit déjà.

Le coût est nommé et il est réel : sur MikaModel le modèle n'a pas le fait posé,
donc la garde 5f y travaille seule, sans la moitié intention. C'est exactement
le régime que mika#2290 a accepté pour l'hébergement, et il se joint au même
suivi (mika#1925). Épinglé par `mika2247_compact_prompt_omits_the_local_time_line`,
rédigé sur le modèle du test frère — **comme décision, pas comme oubli**, pour
que le prochain lecteur trouve l'argument au lieu de le refaire.

### 5b. Moitié structurelle — garde `time_of_day_greeting_mismatch`

Même forme, même budget, position 5f. Ensemble **fermé et étroit** de formules
explicitement horodatées, bilingue (« belle journée », « bonne journée »,
« bonjour », « bonsoir », « bonne nuit », « good morning », « good evening »,
« good night »), et elle ne fire que si (a) une heure locale est **connue** et
(b) la formule nomme un autre moment que celui calculé.

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
| 1 | `text::tests::mika2247_normalizer_is_idempotent_and_utf8_safe` | une boucle ou une panique sur multi-octets |
| 1 | `text::tests::mika2247_the_three_measured_occurrences` | les trois chaînes du ticket, vérifiées en sortie |
| 1 | `prompt/agent_loop` : contrôle négatif **opérateur** | que le normaliseur morde hors tier famille |
| 1 | garde structurelle : scan de source, `strip_internal_tags` suivi du normaliseur aux trois sites | un quatrième site muet — halte, pas d'allowlist |
| 2 | `config::tests::mika2247_language_three_states` | qu'une absence arme quoi que ce soit |
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
- `tenant_language_unrecognized_value` (WARN) — **zéro attendu** ; nomme la valeur
  entre guillemets, pour qu'un espace parasite se voie.

Le normaliseur typographique **n'émet rien**, délibérément : il tourne sur tous
les tours famille, une ligne par tour serait du churn que la doctrine mika#2131
borne, et son effet est vérifiable par test plutôt que par grep.

### Sondes post-déploiement, et leurs haltes

1. **AC1, 48 h.** Rejouer les trois échanges mesurés sur un tenant famille :
   zéro U+2014 en sortie. *Halte* : un em-dash qui réapparaît alors que les tests
   sont verts signifie un **quatrième** chemin de sortie — l'établir, ne pas
   élargir le normaliseur au gateway par réflexe.
2. **AC2.** Rejouer le fil bilingue. *Halte* : si la bascule persiste avec
   `guard.response_language_drift` **vide**, le tour passe par un assembleur que
   la garde ne traverse pas ; établir lequel d'abord.
3. **AC3.** Une salutation le soir, fuseau déclaré, puis la même **sans** fuseau.
   Le second cas doit produire une salutation non horodatée, pas un pari.
4. **Contrôle négatif opérateur.** Sur la station de Vincent (tier `default`,
   aucune variable de langue) : em-dashes préservés, aucune garde armée. *Halte* :
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
  tombent en `Unknown`, donc au comportement d'aujourd'hui. Suivi.
- **Le prompt compact et mika#1925.** Le carve-out ne bouge pas : § 5a explique
  pourquoi l'axe 3 le **suit** au lieu d'y déroger, et la moitié intention y
  reste retenue — protection assurée par la garde 5f, qui lit le texte sortant.
  Même suivi que mika#2290.
- **mika#2245** (défaut-racine de contexte) et le ticket frère « boilerplate ».
  Le ticket le pose lui-même dans sa section *Portée* — trois clusters distincts.
- **Un filet moteur pour la langue** (forme mika#2368). Conditionné à la mesure
  du résidu `_uncorrected` : instruire avant de mesurer serait construire sur une
  hypothèse.

---

## Definition of Done

- [ ] `FAMILY_SOUL` ne porte plus aucun U+2014 / U+2013 / U+2026, **aucun mot
      changé**, et un test de constant le refuse en retour.
- [ ] `mika_common::text` expose le normaliseur typographique, idempotent et
      UTF-8 safe, appliqué aux **trois** sites de § 3b, scopé
      `PersonaProfile::Family` par un `match` sans `_ =>`.
- [ ] Une garde structurelle refuse un quatrième site de sortie non normalisé.
- [ ] `MIKA_TENANT_LANGUAGE` + `[locale].language` résolus en trois états, lus
      une fois par process, mis en cache, filetés vers `PromptContext` et
      `ToolContext` par le chemin de `deployment`.
- [ ] `## Runtime` porte la langue déclarée ; **rien** en `Unknown`.
- [ ] Garde 5e `response_language_drift` : un re-prompt, fail-open sur
      indécidable, résidu nommé par `_uncorrected`.
- [ ] `write_time_section` pose l'heure locale et le moment de la journée
      calculés, parse `Tz` **et** `FixedOffset`, et **dit** l'absence de fuseau.
- [ ] Les **trois** assembleurs sont traités nommément (R3b) : full et silencieux
      portent la ligne par `write_time_section` partagée ; le compact ne la porte
      pas, et un test l'épingle **comme décision** avec l'argument de § 5a.
- [ ] Garde 5f `time_of_day_greeting_mismatch`, ensemble fermé bilingue,
      fail-open sans heure connue.
- [ ] Contrôle négatif opérateur vert : aucun des trois axes ne mord hors famille.
- [ ] Un contrôle négatif **par terme** fail-safe, pas un global.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt` verts ;
      `scripts/check-byte-slices.sh` vert.
- [ ] Root `CLAUDE.md` documente `MIKA_TENANT_LANGUAGE` (trois états, lecture
      unique, non hot-swappable) et les quatre signaux de grep.
- [ ] `crates/mika-agent/CLAUDE.md` documente les gardes 5e/5f dans la table
      fabrication/registre, et `crates/mika-common/CLAUDE.md` le normaliseur dans
      § *Text*.

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
  `guard.response_language_drift_uncorrected`. Le « selon la config » est
  l'axe déclaré `{fr, en}` ; le « selon l'utilisateur » reste le comportement
  d'aujourd'hui en `Unknown`, à dessein (§ 4a).

- **AC3 — Salutations cohérentes avec l'heure locale du tenant.** L'heure locale
  et le moment de la journée cessent d'être une inférence : ils sont calculés et
  posés (§ 5a), sur les deux assembleurs qui servent un tour famille — y compris
  le silencieux, qui est celui où la salutation proactive naît (R3b). Une
  incohérence résiduelle est rattrapée une fois par la garde 5f. Deux bornes
  dites plutôt que découvertes : fuseau non déclaré, la cohérence n'est pas
  asserted — l'ignorance est dite et la salutation horodatée interdite, ce qui est
  **vrai** plutôt que deviné ; et sur le chemin compact (MikaModel) la garde 5f
  travaille seule, sans moitié intention, régime hérité de mika#2290 et joint à
  son suivi.
