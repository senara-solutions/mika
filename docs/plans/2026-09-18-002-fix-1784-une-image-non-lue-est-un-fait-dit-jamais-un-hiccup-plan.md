# mika#1784 — Une image non lue est un fait dit, jamais un « hiccup »

> Ticket : `senara-solutions/mika#1784` — « Agent/Gateway image ingestion cassé (Al) »
> Rapporteur : samidarko, 2026-07-20, retour d'Al B (testeur famille, tier `family`).
> Type : `fix`. Priorité portée par le ticket : `p2-normal`, « à re-tag si le
> diagnostic remonte à un défaut structural ». **Le diagnostic ci-dessous remonte
> à un défaut structural** — voir § *Ce que la mesure déplace*, point (3).

---

## Ce que la mesure déplace

Le ticket pose deux couches candidates et demande de trancher. La lecture du code
tranche, et elle déplace le diagnostic sur trois points. Chacun est vérifiable
sans reproduire l'incident.

### (1) La couche 1 (gateway) est INNOCENTÉE, et complètement

Le ticket demande de vérifier si le `file_id` est téléchargé puis forwardé en
multimodal. Il l'est, de bout en bout :

- `crates/mika-gateway/src/routes.rs:906` — `tg.download_image(file_id)`, qui
  enchaîne `getFile` (`telegram.rs:666`) puis le téléchargement
  (`telegram.rs:759`), avec les trois cas d'erreur **déjà** traités par des
  messages utilisateur distincts et clairs (trop grande, format non reconnu,
  échec de téléchargement — `routes.rs:908-935`).
- `routes.rs:942` — encodage base64, `routes.rs:977-986` — POST vers
  `{container}/message` avec un tableau `images: [{media_type, data}]`.
- Côté agent, `crates/mika-agent/src/server/handlers.rs:146` accepte le payload,
  `:169-185` valide le `media_type` contre `ALLOWED_IMAGE_MEDIA_TYPES`, et
  `:451-460` convertit en `Vec<LlmImage>` passé à `run_agent`.

**Aucune des trois réponses qu'Al a reçues n'est un message du gateway.** Les
trois messages d'échec de téléchargement sont nommément différents du « hiccup ».
Si le téléchargement avait échoué, Al aurait lu « Sorry, I couldn't download your
photo », pas « I had a hiccup processing your message ». Le plumbing du ticket
existe et fonctionne ; le point 1 de l'*Investigation attendue* est répondu par la
lecture, et l'AC3 bascule donc sur sa branche « couche 2 ».

### (2) Les DEUX symptômes d'Al sortent d'un seul prédicat, à deux endroits

Le « hiccup » a un site unique : `handlers.rs:1571`, la branche `Err(e)` de
`run_agent` (constante `AGENT_ERROR_REPLY`, `handlers.rs:1093`). C'est le message
d'échec **générique** du tour ; il ne sait rien des images.

La réponse « Je ne vois pas l'image » a une cause distincte et nommée dans le
code : `crates/mika-agent/src/agent_loop/mod.rs:4072-4078`.

```rust
if !params.user_images.is_empty() && !llm.supports_vision() {
    warn!(provider = …, model = …, "provider does not support vision; images will be ignored");
}
```

L'image est **droppée en silence**. Un `warn!` dans un fichier de log, et rien
d'autre : ni au modèle, ni à l'utilisateur. Le modèle reçoit « Extrait les
numéros de cette photo. » sans image et répond honnêtement qu'il ne la voit pas.
**C'est exactement la réponse 2, à la lettre.**

Le prédicat qui décide est `LlmProvider::supports_vision()`, et sa réponse est
donnée **par rail, jamais par modèle** :

| Rail | `supports_vision()` | Source |
|---|---|---|
| Anthropic | `true` | `llm/anthropic.rs:110` |
| OpenAI-compatible | `true` **ssi** `ProviderKind ∈ {OpenAi, OpenRouter, Mistral, Google, DeepSeek}` | `llm/openai.rs:666-675` |
| Ollama / MikaModel | `false` | `llm/ollama.rs:843` |
| défaut du trait | `false` | `llm/mod.rs:449` |

Les treize `ProviderKind` sont à `llm/mod.rs:504-524`. **`ZAi`, `Groq`, `Kimi`,
`Qwen`, `MiniMax` ne sont pas dans le `matches!`** : ils tombent donc sur le
`false` du rail OpenAI-compatible.

D'où les deux symptômes, selon la configuration effective du tenant d'Al :

- **`llm_provider = "zai"`** (routage GLM natif, documenté au `CLAUDE.md` racine
  comme la voie qui « bypasse la marge OpenRouter ») → `supports_vision()` rend
  `false` → drop muet → **« Je ne vois pas l'image »**.
- **`llm_provider = "openrouter"` avec un modèle GLM text-only** →
  `supports_vision()` rend `true` **par rail**, l'image part en `image_url`
  (`llm/openai.rs:889-893`), le provider refuse la requête, `run_agent` rend
  `Err` → **« hiccup »**.

Le prédicat est donc **à la fois trop permissif et trop restrictif**, pour la
même raison : *la vision est une propriété du modèle, et il répond au niveau du
rail*. Un `matches!` sur `ProviderKind` ne peut pas distinguer `glm-4.5v` de
`glm-5.2` — les deux arrivent par la même porte.

### (3) La même question est posée à quatre endroits, et un de ces endroits ment

`user_images` est lu à quatre sites qui posent tous, implicitement, « que
fait-on de cette image ? » — et qui n'y répondent pas de la même façon :

| Site | Ce qu'il fait | Consulte `supports_vision()` ? |
|---|---|---|
| `agent_loop/mod.rs:3577` (`run_agent`) | persiste `[N image(s) attached]` | **non** |
| `agent_loop/mod.rs:3675` (`run_agent_with_deadline`) | idem, **dupliqué** | **non** |
| `agent_loop/mod.rs:3977` (`run_agent_inner`) | attache les blocs `Image` | oui |
| `agent_loop/mod.rs:4072` (`run_agent_inner`) | émet le `warn!` | oui |

Les deux premiers écrivent `[1 image(s) attached]` dans `messages` **même quand
l'image a été droppée**. Au tour suivant, l'historique affirme donc au modèle
qu'une image était jointe, alors qu'il ne l'a jamais reçue. **C'est une invitation
directe à la fabrication** — précisément ce que l'AC2 demande de préserver. Al a
eu la chance que Mika refuse de fabriquer ; rien dans le code ne le garantissait,
et le ticket a raison d'écrire que « c'est la capacité qui manque, l'intégrité est
OK » : l'intégrité tenait par le modèle, pas par le substrat.

Note connexe : `AGENT_ERROR_REPLY` est en anglais. Al est un testeur **famille**,
dont `FAMILY_SOUL` prescrit le français. Le tour d'échec sort donc du registre que
Vincent a approuvé, sur la seule surface où le modèle ne peut rien corriger — le
message est écrit en dur côté serveur.

---

## Ce qui n'est PAS établi, et qui ne sera pas supposé

Le ticket affirme « Le déploiement family-tier utilise GLM (z.ai / OpenRouter) ».
**Le code ne pose pas cela.** `AgentTier` a exactement deux axes depuis
mika#2023 — `ToolsProfile` (`identity.toml`) et `PersonaProfile` (`soul.md`) — et
**aucun axe modèle** : `home.rs:471` écrit toujours `DEFAULT_CONFIG`, qui porte
`llm_provider = "anthropic"` (`home.rs:529`). Le provider d'un tenant famille
vient donc de l'environnement de service ou du `config.toml` posé au
provisionnement cloud, pas d'un gabarit de tier.

Conséquence pour ce plan : **il ne choisit pas de modèle et n'en bascule aucun.**
L'AC3 (« décision explicite tier/modèle ») demande une décision, et une décision
suppose une mesure que personne n'a aujourd'hui — aucune ligne de log d'Al n'est
citée dans le ticket. Ce plan **produit** cette mesure (bloc 3) et laisse la
décision à l'opérateur, avec le provider et le modèle réellement en vigueur sous
les yeux. Poser un modèle vision par défaut ici serait un choix produit déguisé en
correctif technique, exactement ce que Prime a écarté le 2026-09-09 sur mika#2023.

---

## Conception

Trois blocs. Les blocs 1 et 2 ferment chacun l'un des deux symptômes ; le bloc 3
rend l'AC3 décidable. Aucun ne dépend d'une hypothèse sur la configuration d'Al.

### Bloc 1 — Un seul lecteur du prédicat, et un drop qui se dit

**Lecteur unique.** Un module `crates/mika-agent/src/image_disposition.rs`
expose une fonction pure :

```rust
pub enum ImageDisposition {
    /// Aucune image sur ce tour.
    None,
    /// Le rail déclare la vision : les images sont transmises.
    Transmitted { count: usize },
    /// Le rail ne déclare pas la vision : les images sont retenues.
    Withheld { count: usize, provider: String, model: String },
}

pub fn decide(images: &[LlmImage], llm: &dyn LlmProvider) -> ImageDisposition;
```

Les quatre sites du tableau § (3) la lisent, et **elle seule** consulte
`supports_vision()`. C'est le motif que ce dépôt a déjà dû engraver deux fois —
`grooming_marker` (mika#2158), `live_pilot` (mika#2279) — après avoir mesuré ce
que coûte un prédicat recopié. Ici la divergence est **déjà installée** : deux des
quatre sites ne consultent pas le prédicat du tout.

**Le fait est posé dans le prompt.** Sur `Withheld`, le texte du dernier message
utilisateur est préfixé d'un marqueur explicite, à la place de l'actuel silence :

```
[1 image reçue — non transmise au modèle actif, qui ne lit pas les images]
Extrait les numéros de cette photo.
```

Le modèle **sait** alors, au lieu de devoir deviner. C'est la moitié *intent* de
l'AC2, et elle est structurelle : le fait vient du substrat, pas d'une règle de
prompt. Cf. `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
(mika#2120, neuf récurrences sous enforcement de prompt contre zéro à la main) —
une consigne « n'invente pas le contenu d'une image que tu n'as pas reçue » aurait
exactement la durée de vie que ce retour d'expérience lui prédit.

**La persistance dit la même chose.** `save_text` (sites 3577 et 3675) écrit le
marqueur `Withheld` au lieu de `[N image(s) attached]`. C'est ce qui ferme la
fabrication **au tour suivant**, où le modèle relit l'historique : aujourd'hui
l'historique affirme une pièce jointe que le modèle n'a jamais vue.

**La duplication des deux sites de persistance est supprimée** en passant par le
lecteur unique ; c'est un effet de bord du bloc, pas son objet.

**Trace (AC4).** Sur `Withheld`, une ligne `audit_events`
(`tool_name = 'image_withheld_no_vision'`, `target_key = 'agent:<name>'`,
`after_value = '<provider>/<model>'`) et un `warn!` `image_withheld_no_vision`
portant `provider`, `model`, `count`, `agent_id`, `session_id`. Le `warn!` actuel
(4072-4078) est remplacé, pas doublé : il ne nomme ni l'agent ni la session, donc
il ne permet pas de répondre « quel tenant a perdu une image ».

**Le tour n'est PAS mis en échec.** Sur `Withheld`, le tour tourne normalement
avec la légende seule. Mika répond quelque chose d'utile — et, avec le marqueur,
répond correctement *qu'elle ne peut pas lire l'image*. C'est l'AC1 branche B,
obtenue sans branche d'erreur.

### Bloc 2 — Un refus du provider n'est plus un « hiccup »

Ferme le symptôme 1/3 (le cas `supports_vision() == true` à tort).

`handlers.rs:1569` connaît déjà `has_images` (calculé ligne 146). Sur la branche
`Err`, l'erreur est classée avec le vocabulaire **qui existe déjà** —
`mika_common::llm::error::error_class`, dont `dispatcher.rs:241`
(`classify_delivery_error`) est le précédent : il traverse toute la chaîne de
causes `anyhow` via `downcast_ref::<LlmError>()` et est couvert par six tests
(`dispatcher.rs:3811-3878`). Aucun nouveau vocabulaire, aucune correspondance
par sous-chaîne sur un message rendu.

L'attribution est **conjonctive et étroite** :

- le tour portait au moins une image (`has_images`), **et**
- la classe est `http_4xx` **hors** `http_429`.

Alors, et alors seulement, la réponse est le message dédié « je ne peux pas
traiter les images » au lieu de `AGENT_ERROR_REPLY`. Tout le reste — transport,
timeout, 5xx, 429, `parse`, `provider`, `other` — garde le hiccup générique
inchangé.

**Pourquoi cette conjonction et pas une plus large.** Un 4xx sur un tour portant
une image a deux causes plausibles : le rejet multimodal et le dépassement de
taille. Les deux appellent le même message. Un 5xx ou un timeout n'apprend rien
sur l'image et l'attribuer serait une fausse attribution — le défaut que ce dépôt
combat sous le nom de fabrication. Un 429 est une limitation de débit, sans
rapport. La règle qui en découle et qu'il faut écrire : *une erreur n'est
attribuée à l'image que lorsque le provider a refusé la requête.*

**Angle mort assumé, nommé plutôt que découvert.** `llm/anthropic.rs` aplatit
toute erreur de son rail en `LlmError::ProviderError` (documenté au
`mika-common/CLAUDE.md`, et mika#2331 a contourné l'aplatissement sans le
corriger). Un 400 Anthropic ne serait donc pas classé `http_400` et retomberait
sur le hiccup générique. **C'est sans effet ici** : ce rail rend `true` à
`supports_vision()` et supporte réellement la vision, donc il ne produit pas la
population que ce bloc vise. Corriger l'aplatissement changerait la classe
d'erreur vue par tout le moteur sur ce rail — un format de fil — et mérite son
propre ticket.

**Registre.** Le message dédié existe en deux écritures, choisies sur
`PersonaProfile` (`AgentState.tier`, mis en cache à `init_agent` depuis
mika#1962) : formulation opérateur en anglais, formulation famille en français et
sans jargon. C'est exactement le motif que mika#2290 a déjà tranché pour la
phrase d'hébergement — `match` exhaustif, pas de bras `_ =>`, et **aucune règle
dérivée de la locale du compte** (arbitrage de Prime, 2026-09-09). Al étant un
testeur famille francophone, servir de l'anglais technique sur le tour d'échec
est une partie du défaut qu'il a subi, pas un détail cosmétique.

### Bloc 3 — Rendre l'AC3 décidable

`decide()` rend `provider` et `model` sur `Withheld`, et ces deux champs
atterrissent dans la ligne d'audit **et** dans le `warn!`. L'opérateur peut alors
répondre par une requête, et non par une intuition, à la seule question que l'AC3
pose : *quel couple provider/modèle sert ce tenant, et lit-il les images ?*

```sql
SELECT after_value, count(*)
FROM audit_events
WHERE tool_name = 'image_withheld_no_vision'
GROUP BY 1 ORDER BY 2 DESC;
```

La décision qui suit — basculer le tenant sur un modèle vision, ou assumer la
branche B — se prend avec cette distribution sous les yeux, et se pose dans le
`config.toml` per-agent. Elle n'est pas dans ce PR.

---

## Ce qui est écarté, délibérément

**Une allowlist de modèles vision.** C'est la correction qui vient à l'esprit
— faire consulter le nom du modèle à `supports_vision()` — et elle ne ferme rien.
Un catalogue de modèles multimodaux dérive à chaque sortie de modèle, et il se
trompe **dans les deux sens** : un `false` à tort reproduit le drop muet
d'aujourd'hui, un `true` à tort reproduit le hiccup. Elle déplacerait la frontière
sans la rendre fiable, en ajoutant une liste à tenir à jour. Les blocs 1 et 2
rendent les **deux** côtés de l'erreur inoffensifs, ce qui est strictement plus
fort qu'une frontière mieux placée.

**Un retry sans l'image après un refus 4xx.** Séduisant : le tour serait sauvé au
lieu d'être perdu. Écarté pour ce PR. (a) Les AC ne le demandent pas — l'AC1
offre A **ou** B, et le bloc 2 livre B. (b) Il ajoute un second appel LLM complet
et une branche de contrôle dans `run_agent`, sur un chemin dont **aucune ligne de
production n'est citée** : le ticket ne contient pas un seul extrait de log. (c)
Le bloc 3 produit précisément la mesure qui dirait si cette population existe et à
quel volume. Ticket de suivi **conditionné à la première occurrence mesurée** de
`image_request_refused`, pas ouvert d'avance.

**Corriger `supports_vision()` lui-même.** Le prédicat reste sur le rail. Les
blocs 1 et 2 rendent ses deux modes d'erreur sûrs ; le rendre exact demanderait le
catalogue écarté ci-dessus.

**Le drop silencieux d'images côté `ollama.rs`.** `ollama.rs:281-282` filtre les
blocs non-`Text` (`_ => None`) sans rien dire, donc une image y disparaît même en
amont du prédicat. Sans effet sur ce ticket — `supports_vision()` y rend `false`,
donc le bloc 1 intercepte avant. Réel mais orthogonal.

**L'aplatissement d'erreur du rail Anthropic.** Voir bloc 2 ; format de fil,
ticket propre.

---

## Surfaces opérateur

Dans `$MIKA_SPIRIT_LOG_FILE` :

- `image_withheld_no_vision` (WARN — champs `provider`, `model`, `count`,
  `agent_id`, `session_id`). **Régime attendu : non nul si et seulement si un
  tenant tourne sur un modèle sans vision.** Chaque ligne est une image qu'un
  utilisateur a envoyée et que Mika n'a pas lue — c'est le compteur de la
  capacité manquante, et c'est l'entrée de l'AC3.
- `image_request_refused` (WARN — champs `error_class`, `provider`, `model`,
  `image_count`). **Régime attendu : zéro.** Toute occurrence est un rail qui
  déclare la vision et dont le modèle ne l'a pas : la moitié « trop permissive »
  du prédicat, rendue visible. Une occurrence soutenue sur un même couple
  provider/modèle est l'argument chiffré pour le retry écarté ci-dessus.

En SQL : `image_withheld_no_vision` est un **sole writer** (le module
`image_disposition` est le seul site à écrire ce `tool_name`), donc la requête du
bloc 3 est la liste exacte, et non une approximation.

---

## Sondes post-déploiement, et leurs haltes

1. **Rejeu du cas d'Al** (une photo avec légende, sur un tenant famille). Deux
   issues acceptables et une seule inacceptable : soit l'OCR répond (branche A),
   soit Mika dit clairement qu'elle ne lit pas les images (branche B). **Un
   « hiccup » est une halte** : le bloc 2 n'a pas pris, et la classe d'erreur
   réellement rendue doit être lue (`grep image_request_refused`) avant toute
   autre modification.

2. **Intégrité (AC2), sur deux tours.** Envoyer la photo, puis demander au tour
   suivant « qu'est-ce qu'il y avait sur la photo ? ». Le modèle doit redire qu'il
   ne l'a pas reçue. **Une réponse inventée est une halte** : le marqueur ne
   survit pas à la persistance, donc c'est le site `save_text` qu'il faut
   reprendre, et pas le prompt.

3. **Attribution, sur 48 h.** `image_withheld_no_vision` non vide **et**
   `image_request_refused` vide est le régime nominal d'un tenant sans vision.
   **Les deux non vides sur le même agent est une halte** : cela signifie que
   `supports_vision()` répond différemment à deux instants pour un même agent, ce
   qu'aucun chemin de ce plan ne produit — le budget par skill (`[llm]` override)
   serait alors en cause, et c'est un autre défaut.

4. **Registre.** Le message de la branche B servi à un tenant famille doit être en
   français. S'il sort en anglais, c'est `PersonaProfile` qui n'est pas lu à ce
   site — vérifier `AgentState.tier` avant de toucher au message.

---

## Definition of Done

- [ ] `crates/mika-agent/src/image_disposition.rs` existe, expose `decide()` et
      `ImageDisposition`, et est le **seul** site de `crates/mika-agent/src/`
      consultant `supports_vision()` — garde structurelle par scan de source, sur
      le modèle de `grooming_marker::tests::no_grooming_regex_outside_this_module`.
      Une garde comportementale ne verrait pas cette classe : un cinquième site
      recopié ne rendrait aucune décision fausse, il la rendrait divergente.
- [ ] Les quatre sites de `agent_loop/mod.rs` (3577, 3675, 3977, 4072) lisent
      `decide()` ; la duplication entre `run_agent` et `run_agent_with_deadline`
      est supprimée.
- [ ] Sur `Withheld`, le marqueur est présent **dans le prompt** et **dans la
      ligne persistée** ; `[N image(s) attached]` ne peut plus être écrit pour une
      image retenue.
- [ ] Sur `Withheld`, une ligne `audit_events` (`image_withheld_no_vision`) et un
      `warn!` portant `provider`, `model`, `agent_id`, `session_id`.
- [ ] `handlers.rs` branche `Err` : classification via
      `mika_common::llm::error::error_class` (pas de correspondance par
      sous-chaîne), conjonction `has_images && http_4xx && !http_429`, message
      dédié ; toutes les autres classes gardent `AGENT_ERROR_REPLY`.
- [ ] Le message dédié existe en deux registres choisis sur `PersonaProfile`,
      par `match` exhaustif sans bras `_ =>`.
- [ ] Tests unitaires : `decide()` sur les trois variantes, sur un provider
      `supports_vision = true` et `false` (le `MockLlmProviderBuilder` expose déjà
      `supports_vision`, `llm/mock.rs:62`).
- [ ] Tests unitaires : la classification d'attribution du bloc 2 sur les sept
      classes d'erreur, incluant les négatifs `http_429`, `transport_timeout` et
      `has_images == false` — chacun doit conserver le hiccup générique.
- [ ] Test de non-fabrication : un tour `Withheld` suivi d'un second tour, dont
      l'historique persisté porte le marqueur et non `[N image(s) attached]`.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt` verts.
- [ ] Aucun modèle, aucun provider, aucun tier n'est modifié par ce PR.

---

## Acceptance criteria

Transcrits depuis le corps de `senara-solutions/mika#1784`.

1. Al envoie une photo → une des deux réponses correctes :
   - **A** : OCR/extraction fonctionne (numéros lus correctement)
   - **B** : Message clair « je ne peux pas traiter les images » (pas de
     « hiccup », pas de fabrication)
2. Aucune hallucination sur le contenu image (préserver l'honnêteté observée)
3. Si couche 1 (gateway) en cause : correctif ; si couche 2 (modèle sans
   vision) : décision explicite tier/modèle
4. Log ou audit_event pour tracer les cas image → sait pourquoi ça a raté

**Comment ce plan y répond, et ce qu'il laisse ouvert :**

- **AC1** — branche B garantie sur les deux chemins : le bloc 1 pour le drop muet
  (le tour tourne et le modèle sait qu'il n'a pas l'image), le bloc 2 pour le
  refus provider (message dédié au lieu du hiccup). La branche A reste
  disponible et inchangée sur tout rail dont le modèle lit réellement les images.
- **AC2** — fermé structurellement : le marqueur pose le fait dans le prompt et
  dans l'historique persisté. Le défaut inverse d'aujourd'hui — un historique qui
  affirme une pièce jointe jamais reçue — disparaît.
- **AC3** — la couche 1 est innocentée par la lecture (§ *Ce que la mesure
  déplace*, point 1), donc l'AC bascule sur sa branche « couche 2 ». **La
  décision tier/modèle n'est pas prise dans ce PR et c'est délibéré** : elle
  demande une mesure que personne n'a, que le bloc 3 produit, et qui se pose dans
  le `config.toml` per-agent. Voir § *Ce qui n'est PAS établi*.
- **AC4** — les deux événements et la requête SQL du bloc 3.

---

## Hors périmètre, délibérément

- Le choix du modèle du tenant famille (AC3, seconde moitié) — conditionné à la
  mesure du bloc 3, posé dans le `config.toml` per-agent, pas dans une variable de
  service fleet-wide (motif mika#2293).
- Un axe modèle sur `AgentTier` — ce serait un troisième axe, et mika#2023 a payé
  le prix de la conflation de deux ; à ouvrir sur une demande produit, pas ici.
- Le retry sans image, l'allowlist de modèles vision, le drop silencieux d'Ollama
  et l'aplatissement d'erreur du rail Anthropic — voir § *Ce qui est écarté*.
- L'OCR lui-même : ce plan ne rend aucun modèle capable de lire une image ; il
  rend l'incapacité **dite, tracée et sans panne**.
