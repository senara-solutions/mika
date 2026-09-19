# mika#2304 — `mika ask --model` est ignoré sur le chemin A2A, et l'instrument qui devrait le dire confirme l'override qu'il n'a pas fait

- **Ticket :** senara-solutions/mika#2304
- **Priorité :** p1 (substrat — faux vert sur un instrument de mesure)
- **Branche :** `feat/2304/mika-ask-model-est-ignor-sur-le-chemin`
- **Lignage :** mika#1727 (la CLI cesse d'être la surface d'exécution — la cause), mika#2363 (le canal de config par metadata A2A — le précédent structurel), mika#1591 (la sémantique « model-id seul, jamais de re-dispatch de provider »), mika#2070 (la première clé de fil `mika.*`), mika#2270 (le point d'intervention sur le `Task` rendu par `message/send`), mika#2290 (le motif « la moitié qui ferme le défaut tient seule »)

---

## Contexte

Le ticket rapporte que `mika ask --model <id>` est ignoré sur le chemin A2A : le
body envoyé au fournisseur porte le modèle du `config.toml` de l'agent, pas
l'override. Mesuré le 2026-09-11 à 17:48Z sur quatre passes (trois
`moonshotai/kimi-k2.5`, une `deepseek/deepseek-v4.1-flash`), en inspectant le
body a2a.

**Le défaut est réel, il est déjà écrit en commentaire dans le code, et il est
plus large que le ticket ne le dit.** Trois lectures déplacent le périmètre et la
troisième décide du remède.

---

## Ce qui est établi, et comment le vérifier

### E1 — Le périmètre n'est pas `--remote`, c'est `mika ask` tout entier

Depuis mika#1727, `mika ask` **sans** `--remote` passe aussi par A2A : il expédie
le prompt au démon mika-spirit local sur `{spirit_url}/a2a/{agent}` et rend le
`Task` retourné, **sans repli en process**
(`crates/mika-cli/src/commands/ask.rs:381-388`). Le commentaire qui précède cet
appel énonce le défaut mot pour mot (`ask.rs:316-319`) :

> `--enable-skill` / `--disable-skill` / `--model` configure the *local*
> registry/LLM, which is no longer the execution surface. Their arg-level
> validation is preserved, but they do not yet reach spirit.

Donc le ticket décrit un cas particulier d'un défaut à deux chemins :

| Chemin | `--model` reçu ? | Dépensé où ? | Atteint l'exécutant ? |
|---|---|---|---|
| `mika ask` local (défaut depuis #1727) | oui (`main.rs:345`) | `ctx.override_model` → provider local **inutilisé** | **non** |
| `mika ask --remote` | **non** — `main.rs:322-327` ne le passe pas à `run_remote` | nulle part | **non** |
| `mika chat --model` | oui (`chat.rs:613`) | `ctx.llm`, qui **exécute** (`chat.rs:186`, `agent::run_agent`) | **oui** |

Vérification : `grep -n "model" crates/mika-cli/src/main.rs` sur la branche `Ask`
— `args.model` apparaît à la ligne 345 (chemin local) et nulle part dans le bloc
`run_remote` des lignes 311-336.

**Conséquence sur la preuve du ticket :** la mesure du 11/09 est décrite comme une
comparaison hors-moteur mika-arch. Les deux chemins étant atteints, la mesure est
valide quelle que soit la porte empruntée — mais le correctif doit couvrir les
deux, et un correctif limité à `remote_ask.rs` (la piste du ticket) laisserait le
chemin **par défaut** cassé.

**Asymétrie à nommer :** le même flag, `--model`, fonctionne sur `mika chat` et
est inopérant sur `mika ask`. Rien dans l'aide ne le dit.

### E2 — `remote_ask.rs` n'est pas le site du défaut, c'est le site du canal

La piste du ticket (« chemin `mika-cli/src/remote_ask.rs` ») désigne le bon
fichier pour la **moitié émission**, et le mauvais pour la cause. `remote_ask.rs`
est déjà le porteur du canal de configuration : `build_send_params`
(`remote_ask.rs:82-123`) écrit deux clés de metadata, `mika.caller_session_id`
(#2070) et `mika.only_skills` (#2363). Une troisième clé s'y ajoute sans
invention d'architecture.

La cause est en amont : `AppContext::override_model` (`crates/mika-cli/src/init.rs:90-105`)
mute `self.db_ctx.settings` puis reconstruit `self.llm` — un provider que plus
personne n'appelle sur ce chemin.

### E3 — Le faux vert est doublement construit : l'instrument confirme l'override

C'est la lecture qui manque au ticket et elle change la hiérarchie du remède.

`override_model` appelle `settings.set_provider_model(provider, Some(model_id))`
(`init.rs:100-102`), donc `ctx.settings` porte le modèle **demandé**. Or
`ask.rs:440-444` lit exactement ce champ pour peupler `metadata.model` :

```rust
let model_string = {
    let provider = ctx.settings.llm_provider;
    let (model_name, _, _) = ctx.settings.provider_fields(provider);
    model_name.map(|m| format!("{provider}/{m}"))
};
```

Donc **`mika ask --model moonshotai/kimi-k2.5 --verbose` affiche
`model: openrouter/moonshotai/kimi-k2.5`** — le modèle demandé — alors que le
tour a tourné chez spirit sous le modèle du `config.toml`. Le champ n'est pas
absent ni nul : il affirme, avec autorité, l'override qui n'a pas eu lieu.

C'est ce qui explique la forme de la preuve du ticket. L'opérateur a dû
**inspecter le body a2a** pour découvrir le défaut : c'était le seul endroit qui
disait la vérité, parce que la surface prévue pour la dire mentait. Un
`--verbose` honnête aurait rendu le défaut visible en une passe au lieu de
quatre.

Vérification : lire `init.rs:100-102` puis `ask.rs:440-444` — le même champ
`Settings` est écrit par le premier et lu par le second, et rien entre les deux
ne consulte le serveur.

### E4 — Le précédent structurel existe et il porte sa propre politique de défaillance

mika#2363 a construit exactement ce canal pour `--only-skill` : clé posée dans
`mika-a2a::params` (le crate que les deux bords partagent, `mika-cli` et
`mika-agent` n'ayant aucune arête entre eux), écrite par `build_send_params`,
lue par `requested_only_skills` (`server/a2a.rs:268-284`), appliquée au tour.

Sa politique de défaillance est **fail-soft** : clé absente, `null`, non-tableau,
tableau vide — tout signifie « aucune restriction ». Le commentaire donne la
raison : *« a caller from an older or a newer version of the protocol must not be
able to fail a turn with a field this server is free to ignore »*.

**Cette politique ne se transpose pas ici, et c'est le point de conception
central du ticket** — voir D2.

### E5 — Un provider par tour existe déjà, avec son budget

`resolve_skill_llm_override` (`agent_loop/mod.rs:5855-5975`) construit un provider
pour la durée d'un tour via `settings.make_provider_for(provider_kind, model)`
(`config.rs:1850-1872`), qui passe par `create_provider_with_budget` avec
`self.llm_timeout_budget()` — donc l'invariant de confinement mika#2189
(`plafond < enveloppe`) est préservé sans travail supplémentaire.

`run_a2a_agent` détient `agent_state.settings` (passé en `params.settings`,
`server/a2a.rs:224`), donc le constructeur est atteignable au site exact où la
clé est lue.

### E6 — `--remote` est délibérément dépouillé de ses deux clés sœurs, avec raisons écrites

`dispatch_remote` (`remote_ask.rs:350-362`) n'envoie **ni** `mika.caller_session_id`
**ni** `mika.only_skills`, et le commentaire donne les deux raisons :

> `--remote` sends no caller session id (mika#2070). The local bookkeeping
> session lives in this machine's database; a remote agent normally holds a
> different one and would refuse the id. […] `--remote` sends no skill
> restriction either (mika#2363): a remote agent's skill names are not this
> machine's to guess.

Le ticket porte pourtant explicitement sur `--remote`. Faire traverser une
troisième clé là où les deux premières sont refusées demande donc une raison qui
sépare — pas un oubli de lecture. Elle est écrite en D9.

### E7 — Le provider qui sert le tour n'est pas celui que `a2a.rs` connaît

C'est la lecture qui décide du **site** de l'attestation, et elle contredit une
rédaction antérieure de ce plan (voir *Revision history*).

`run_a2a_agent` passe `llm: agent_state.llm.as_ref()` (`server/a2a.rs:201`). Mais
en aval, `agent_loop` recalcule le provider effectif :

```rust
let skill_llm_override = resolve_skill_llm_override(&matched, params.settings, llm);
let effective_llm: &dyn LlmProvider = match &skill_llm_override {
    Some(override_llm) => override_llm.as_ref(),
    None => llm,
};
```

Donc une attestation prise dans `a2a.rs` rapporterait le provider **d'entrée**,
pas celui qui a servi. Sur un tour où un skill porte un override `[llm]`, elle
affirmerait avec autorité un modèle qui n'a pas tourné — c'est-à-dire qu'elle
reconstruirait exactement la classe de défaut E3 que ce ticket existe pour
fermer, déplacée d'un champ. **Cette conclusion est confirmée et elle décide le
site de l'attestation.**

`AgentOutput` est le canal de retour, et son propre doc-commentaire porte le
précédent : mika#2276 a ajouté `deadline_exceeded` comme **champ** plutôt qu'en
élargissant le retour en `LoopResult`, avec la raison (*« les appelants de
`run_agent` consomment déjà cette struct »*).

### E7b — Les deux sites de recalcul ne sont pas deux chemins du même tour

La rédaction précédente de ce plan traitait les deux appels de
`resolve_skill_llm_override` comme deux chemins interchangeables d'un même tour,
et en tirait deux exigences (attestation sur les deux, précédence sur les deux).
L'inventaire des fonctions porteuses réduit le périmètre, et c'est le livrable
principal de cette troisième passe. Les citations sont par **nom de fonction** :
les numéros de ligne de la passe précédente ont dérivé de ~285 lignes en un
rebase, ce qui est précisément la raison de ne pas s'y ancrer.

| Site | Fonction porteuse | Struct de params | Type de retour |
|---|---|---|---|
| #1 | `run_agent_inner` | `AgentParams` | `AgentOutput` |
| #2 | `run_team_agent_inner_impl` | `TeamAgentParams` | `TeamAgentOutcome` |

Trois conséquences, chacune vérifiable :

1. **`run_a2a_agent` n'atteint que le site #1.** Il appelle `agent::run_agent`
   (`server/a2a.rs:232`), donc `run_agent_inner`. Le chemin team n'est pas
   joignable par `message/send`.
2. **`run_silent_inner` n'appelle `resolve_skill_llm_override` à aucun endroit.**
   Aucun tour silencieux (callback, heartbeat, réflexion) ne porte d'override
   per-skill. Le périmètre du recalcul est donc exactement deux fonctions, pas
   « partout dans le moteur ».
3. **`TeamAgentParams` est une struct distincte d'`AgentParams`.** Un champ ajouté
   à la seconde est invisible depuis le chemin team, par typage.

Vérification : `grep -n "resolve_skill_llm_override" crates/mika-agent/src/agent_loop/mod.rs`
rend deux sites d'appel et une définition ; croiser leurs numéros avec
`grep -n "^async fn \|^pub async fn " ` sur le même fichier donne les deux
fonctions porteuses.

### E7c — Le fail-closed ne peut pas être délégué au constructeur de provider

Lecture qui décide de **l'ordre des opérations** en V7 et de la construction de
T4. `make_provider_for` appelle `create_provider_with_budget`, dont la branche
`_ =>` couvre dix variantes OpenAI-compatibles — **dont OpenRouter, le rail des
quatre passes mesurées du ticket**. Cette branche construit
`OpenAiCompatibleProvider::new(...)`, qui **ne rend pas de `Result`** et ne
consulte jamais `api_key` :

```rust
let provider = openai::OpenAiCompatibleProvider::new(
    base_url, spec.api_key.clone(), spec.model.clone(), /* … */
);
Ok(Arc::new(provider))
```

Donc **`make_provider_for` réussit avec une clé API absente sur le rail qui porte
le ticket.** Les seules causes d'échec à la construction y sont une `base_url`
absente et un budget de timeout invalide (mika#2189). Deux populations à séparer,
que la rédaction précédente de D2 rangeait ensemble :

| Cause | Détectable avant l'appel réseau ? | Qui l'attrape |
|---|---|---|
| Clé API absente sur un fournisseur qui en exige une | **oui** | un `check_provider_key` **explicite**, à écrire côté serveur |
| `base_url` absente | oui | `create_provider_with_budget` |
| Modèle inexistant chez le fournisseur | **non, à aucun endroit** | le fournisseur, en 400/404 au premier appel |

Conséquence pour D2 : le fail-closed sur la clé doit être **posé**, jamais espéré
du constructeur. Et « modèle inconnu du fournisseur » n'est pas une validation
préalable implémentable — c'est un échec de tour qui survient à l'appel, et il
n'y a rien à écrire pour l'obtenir.

### E8 — Ce qui n'est PAS établi

La mesure du 11/09 ne dit pas par quelle porte elle est passée (locale ou
`--remote`), et les logs de ce jour ne sont pas rejoués ici. Cela ne bloque pas :
les deux portes sont cassées par lecture du code, et le correctif couvre les
deux. Ce qui resterait à trancher si le symptôme survivait au déploiement est
écrit en § *Sonde post-déploiement*.

---

## Décisions

### D1 — Propager, pas refuser

Le ticket laisse le choix ouvert (« propager, ou faire échouer avec un message
clair »). Le « pourquoi ça compte » tranche : le pré-vol d'un fournisseur et
toute comparaison de modèles **reposent** sur `--model`. Refuser ferme le faux
vert et supprime la capacité : chaque modèle testé demanderait alors un edit de
`config.toml` **plus un redémarrage de mika-spirit**, c'est-à-dire exactement ce
que le flag existe pour éviter, sur l'agent partagé par toute la flotte.

Refuser reste le repli si l'architecte écarte D2 ; dans ce cas la moitié D3
ci-dessous tient seule et ferme le faux vert quand même.

### D2 — Fail-**closed** sur l'application, à l'inverse de `only_skills`

Un override de modèle que le serveur ne peut pas appliquer **fait échouer le
tour**. Il n'est jamais dégradé en « pas d'override ».

**Ce que « ne peut pas appliquer » recouvre, et par quel mécanisme** — la
distinction vient de E7c et elle est load-bearing pour V7 et T4 :

- **clé API absente** sur un fournisseur qui en exige une → refusé **avant**
  l'appel, par un `check_provider_key` explicite côté serveur. C'est la seule
  branche que le correctif doit écrire, et la seule que T4 peut tester sans
  réseau ;
- **`base_url` absente** → `create_provider_with_budget` échoue déjà ;
- **modèle inexistant chez le fournisseur** → non détectable avant l'appel, à
  aucun endroit. Le tour échoue à l'appel, sur l'erreur du fournisseur. Aucun
  code à écrire : c'est déjà un échec de tour, et c'est déjà fail-closed.

Formulation à ne pas reprendre : *« modèle inconnu du fournisseur configuré »*
comme condition d'un refus préalable. Elle suggère une validation qui n'existe
pas et qui demanderait un aller-retour réseau pour exister.

L'asymétrie avec mika#2363 est délibérée et sa raison est mesurée par ce
ticket même :

- une restriction de skills non appliquée rend le tour **plus large**, ce qui est
  visible (`active_skill_count`, l'absence de garde de contrat) et ne falsifie
  aucune mesure ;
- un modèle non appliqué rend la **mesure fausse tout en produisant une réponse
  plausible**. C'est la définition du faux vert que le ticket dénonce : « on croit
  tester un modèle et on en teste un autre ».

Un no-op silencieux ici *est* le défaut. Le fail-soft reste en vigueur pour la
**lecture de la clé** (clé absente, `null`, non-chaîne, chaîne vide → aucun
override, corps d'avant le correctif byte pour byte) ; le fail-closed ne
s'applique qu'à partir du moment où un override a été **effectivement déclaré**.

Coût nommé : un appelant authentifié de `/a2a/{agent}` peut faire échouer **son
propre** tour en envoyant un modèle bidon. Ce n'est pas une surface nouvelle — il
peut déjà faire échouer son tour de mille façons — et l'échec ne déborde pas sur
les autres appelants.

### D3 — L'attestation est ce qui ferme le faux vert ; la propagation est ce qui rend la capacité

Séparation load-bearing, motif mika#2290 (« le garde ferme le p1 sans le ticket
compagnon »).

La propagation seule remplacerait un faux vert par une **confiance** : rien, côté
appelant, ne prouverait que le serveur a honoré la clé — et c'est précisément le
défaut qu'on vient de mesurer. Donc :

- le serveur pose le modèle **effectivement utilisé** dans `Task.metadata`
  (le champ existe déjà, `mika-a2a/src/types.rs:147`), sur tout tour, override ou
  pas — **pris sur `effective_llm`, en aval de l'override per-skill** (E7), et
  remonté par `AgentOutput`. Une attestation prise au site d'entrée serait un
  second champ qui affirme un modèle qu'il n'a pas vu servir ;
- le CLI rend **cette** valeur sous `--verbose`, et cesse de lire `ctx.settings` ;
- **en l'absence d'attestation, le CLI n'affiche pas de modèle** — il dit que le
  serveur n'en a pas attesté un. Un serveur antérieur au correctif, ou un agent
  distant d'une autre version, tombe dans ce cas : c'est exactement la population
  où afficher une valeur locale serait un mensonge.

Cette moitié ferme le défaut **quelle que soit** la suite : même si un override
per-skill gagnait la précédence, même si la clé était ignorée, l'opérateur lit le
modèle réel.

### D4 — La chaîne brute voyage ; la résolution se fait côté exécutant

La sémantique mika#1591 est conservée telle quelle : un `préfixe/` n'est retiré
que s'il nomme **le fournisseur configuré**, et le préfixe ne re-dispatche jamais
vers un fournisseur natif. Mais cette décision exige de connaître le fournisseur
**qui exécute**, et en `--remote` ce n'est pas celui de la machine locale.

Donc le CLI envoie la chaîne telle que l'opérateur l'a tapée (`sonnet`,
`moonshotai/kimi-k2.5`) et **le serveur résout** : alias, puis strip de préfixe
conditionnel, puis vérification de clé API. Résoudre localement produirait, sur
`--remote`, un id résolu contre le mauvais fournisseur — un second faux vert,
plus discret que le premier.

### D5 — Conséquence de D4 : un seul site de résolution, remonté dans `mika-common`

`resolve_model_alias` (`mika-cli/src/cli.rs:1261`), `parse_model_override` et
`check_provider_key` (`mika-cli/src/init.rs:116-144`) deviennent atteignables par
`mika-agent`. Ils remontent dans `mika-common` (où vivent `ProviderKind` et
`Settings`) plutôt que d'être copiés.

La règle est celle que mika#2158 a dû graver après coup : *un résolveur écrit une
seconde fois est un résolveur qui peut diverger du premier*. `mika-cli` continue
de les appeler pour `mika chat`, qui reste in-process.

### D6 — Clé de fil `mika.model_override`, dans `mika-a2a::params`

Troisième clé de la famille, même justification écrite sur les deux premières :
`mika-cli` l'écrit, `mika-agent` la lit, et ils ne partagent aucune arête de
dépendance le long de laquelle un renommage pourrait voyager. Épinglée par un
test d'orthographe, comme ses deux sœurs (`params.rs:89-102`).

Clé absente ⇒ `metadata` reste absent quand les autres clés le sont aussi
(propriété R3 de mika#2363 : un appelant qui ne déclare rien produit le corps
d'avant).

### D7 — `--model` gagne sur un override per-skill, sur le seul site où la question se pose

Un opérateur qui nomme un modèle ne veut pas qu'un skill le remplace en silence :
ce serait le même faux vert déplacé d'un cran. `resolve_skill_llm_override`
s'abstient quand le tour porte un override explicite d'appelant.

**Périmètre : le site `run_agent_inner` uniquement** (E7b). Le canal de la
précédence est un champ d'`AgentParams` ; `run_team_agent_inner_impl` lit
`TeamAgentParams`, donc il ne peut pas le voir — et n'a rien à en faire, aucun
appelant de tour d'équipe ne pouvant demander un modèle. Le site team **garde son
comportement actuel**, qui est correct.

La rédaction précédente exigeait de traiter « les deux sites ensemble » et en
faisait un mode de défaillance (FD6). Cette exigence tombe : l'asymétrie n'est pas
une divergence, c'est un typage. Ce qui reste utile est **une note au site team**
disant pourquoi il ne consulte rien — sans elle, un lecteur futur ne peut pas
distinguer « délibérément hors canal » de « oubli ».

L'attestation (D3) reste le filet : si cette précédence était mal câblée,
l'opérateur lirait le modèle réel plutôt que celui qu'il a demandé.

### D8 — Angle mort déclaré : le per-skill n'a que l'attestation, pas l'événement

Comme mika#2293 l'a fait pour `llm_budget_resolved`, l'angle mort est écrit au
site d'émission plutôt que découvert : l'événement de configuration
(`a2a_model_override_applied`) n'est **pas** étendu au chemin per-skill par ce
ticket — il ne dit que les overrides d'appelant.

L'attestation du modèle effectif, elle, couvre ce chemin — **mais par le site
choisi en E7, pas « par construction »**. C'est une propriété du câblage, donc
elle se teste (T10) au lieu d'être supposée.

Second angle mort, nommé par E7b : **le chemin team n'atteste rien.** Il ne
retourne pas d'`AgentOutput` et n'est pas joignable par `message/send`, donc il
n'y a pas de surface d'affichage à qui mentir. Étendre l'attestation à
`TeamAgentOutcome` serait du travail sans lecteur.

### D9 — `--model` traverse `--remote`, quand ses deux clés sœurs ne traversent pas

L'asymétrie est réelle (E6) et demandée par le ticket, qui nomme `--remote` dans
son titre même. Les raisons écrites sur les deux refus ne se transposent pas :

- `caller_session_id` est refusé parce qu'il désigne **une ligne de la base de
  cette machine**, que le distant n'a pas ;
- `only_skills` est refusé parce qu'il désigne **des noms de skills de cette
  machine**, que le distant ne porte pas forcément.

Les deux sont des références locales que l'appelant *devine*. Un model-id n'en
est pas une : c'est une chaîne que l'opérateur a tapée, résolue contre le
fournisseur **de l'exécutant** (D4), et sa validité est une propriété du
fournisseur distant — pas une inférence de cette machine.

Et le fail-closed (D2) est ce qui rend l'asymétrie sûre : un id que le distant ne
peut pas servir fait échouer le tour en nommant le modèle et le fournisseur.
C'est le comportement que le ticket réclame explicitement en repli (« faire
échouer avec un message clair »), obtenu ici sans renoncer à la capacité.

**Ce que ça ne fait pas :** `--remote` n'affiche aucun modèle aujourd'hui —
`render` ne rend que `remote_task_id` sous `--verbose`
(`remote_ask.rs:378-384`). Sur ce chemin, D3 est donc un **ajout** d'attestation,
pas la correction d'un mensonge. Le faux vert mesuré en E3 est propre au chemin
local, et il ne faut pas le sur-attribuer à `--remote`.

---

## Volets d'implémentation

### V1 — `crates/mika-a2a/src/params.rs`

`MODEL_OVERRIDE_KEY = "mika.model_override"`, doc-commentée sur le modèle des
deux clés existantes : ce qu'elle porte, qui la lit, et — contrairement à ses
sœurs — que son application est fail-closed, avec la raison (D2). Test
d'orthographe de fil.

### V2 — `crates/mika-common` (nouveau site de résolution, D5)

Déplacement de `resolve_model_alias`, `parse_model_override`,
`provider_requires_api_key`, `check_provider_key` depuis `mika-cli`, avec leurs
tests. Une fonction d'entrée unique qui, prenant `&Settings` et la chaîne brute,
rend soit `(ProviderKind, String)` soit l'erreur nommée déjà existante
(`Provider '<name>' has no API key configured. Cannot route model '<id>'.`).

### V3 — `crates/mika-cli/src/init.rs` + `cli.rs`

`override_model` délègue au site commun (comportement de `mika chat` inchangé,
attesté par ses tests existants). Les copies locales disparaissent — garde
structurelle en T6.

### V4 — `crates/mika-cli/src/remote_ask.rs`

`build_send_params` gagne le paramètre `model_override: Option<&str>` et pose la
clé. `send_message_to_agent` et `dispatch_remote` la threadent. `dispatch_remote`
**reçoit enfin le modèle** — aujourd'hui il ne le reçoit pas (E1), et il passe
délibérément `None`/`&[]` pour les deux clés sœurs : ce commentaire est **mis à
jour**, pas contourné, pour porter la raison qui sépare les trois cas (D9).
Laisser le commentaire dire « `--remote` n'envoie pas de configuration » pendant
qu'une clé la traverse recréerait la divergence commentaire-vs-code que E1 vient
de mesurer sur `ask.rs:316-319`.

Le `Task` rendu expose son modèle attesté via un accesseur (lecture de
`Task.metadata`), pour que les deux surfaces d'affichage — `render` ici,
l'enveloppe de `ask.rs` — lisent la même chose.

### V5 — `crates/mika-cli/src/main.rs`

La branche `run_remote` passe `args.model.as_deref()`.

### V6 — `crates/mika-cli/src/commands/ask.rs`

- `model_override` cesse d'être dépensé sur `ctx.override_model` et part dans
  `send_message_to_agent` ;
- **la validation d'arg reste locale et précoce** — un modèle mal formé doit
  échouer avant l'aller-retour réseau ; seule la *résolution contre le
  fournisseur exécutant* part au serveur ;
- `metadata.model` est peuplé depuis l'attestation du `Task`, plus depuis
  `ctx.settings` (D3). Absence d'attestation ⇒ champ absent en JSON et ligne
  explicite en mode texte, jamais une valeur locale.

### V7 — `crates/mika-agent/src/server/a2a.rs`

- `requested_model_override(&MessageSendParams) -> Option<&str>` : fail-soft en
  lecture (absent / `null` / non-chaîne / vide ⇒ `None`), sœur de
  `requested_only_skills` (`a2a.rs:268-284`) ;
- `run_a2a_agent` **vérifie la clé d'abord, construit ensuite** : le
  `check_provider_key` remonté en V2 est appelé **avant**
  `agent_state.settings.make_provider_for(...)` (E5), parce que le constructeur ne
  peut pas attraper ce cas sur le rail du ticket (E7c). Les deux échecs font
  échouer le tour en nommant le modèle et le fournisseur.

  Ordre imposé, pas cosmétique : inverser les deux donnerait un provider
  construit avec succès, une clé absente, et un tour qui part quand même — le
  fail-closed serait écrit dans le plan et absent du binaire ;

  Forme imposée par les types : `AgentParams.llm` est un `&'a dyn LlmProvider`
  (`mod.rs:3563`) et `make_provider_for` rend un `Arc`. L'`Arc` est donc lié à une
  variable de la portée de `run_a2a_agent` et `params.llm` reçoit `arc.as_ref()`.
  Sans override, `agent_state.llm.as_ref()` est passé inchangé — aucun provider
  construit, aucun coût sur le chemin nominal ;
- le modèle attesté est lu sur `AgentOutput` (V8) et posé dans `Task.metadata` au
  point d'intervention mika#2270 — le bras `Ok(Some(mut task))` de
  `a2a_build_task` (`a2a.rs:~742`), où le `Task` est déjà `mut` et où
  `ensure_send_task_carries_text` intervient déjà ;
- **périmètre d'attestation, nommé plutôt que supposé** : le chemin couvert est
  `message/send` **synchrone**, celui qu'emprunte `send_message_to_agent`, donc
  les deux portes du ticket. `message/stream` et la branche `returnImmediately`
  ne sont pas couverts par ce ticket — la seconde ne fait tourner aucun tour, il
  n'y a donc rien à attester. Ces chemins tombent dans la population « pas
  d'attestation », que D3 traite déjà honnêtement : le CLI n'affiche rien plutôt
  qu'une valeur locale ;
- un événement INFO `a2a_model_override_applied` (agent, task_id, demandé,
  résolu), sœur de `a2a_only_skills_applied` (`a2a.rs:183`).

### V8 — `crates/mika-agent/src/agent_loop/mod.rs` — les deux moitiés manquantes

**(a) L'attestation remonte.** `AgentOutput` gagne
`effective_model: Option<String>`, peuplé depuis `effective_llm.provider_name()`
/ `model_name()` — les valeurs sont **déjà lues** au site `run_agent_inner`
(elles alimentent `turn_usage` et la construction de la requête), il n'y a pas de
calcul à ajouter. Champ plutôt qu'élargissement du retour : le motif que
`deadline_exceeded` documente déjà sur cette même struct (mika#2276).

Coût chiffré, que la rédaction précédente ne donnait pas — **trois sites de
construction** en production :

| Site | Fonction | Ce qu'il porte |
|---|---|---|
| `LoopResult::Done` | `run_agent_inner` | tour conclu — variable locale, triviale |
| continuation max-steps | `run_agent_inner` | idem, même portée |
| repli deadline | `persist_deadline_fallback` | **fonction auxiliaire** : reçoit la valeur en paramètre |

Le troisième est le seul qui demande un changement de signature, et il porte le
chemin mika#2276 : un tour coupé par sa deadline a bien tourné sous un modèle, et
c'est une population où l'opérateur a particulièrement besoin de savoir lequel.

Un tour qui échoue **avant** le recalcul (`load_agent_context`,
`get_customer_config`) ne produit aucun `AgentOutput` — il rend `Err`, servi en
tâche `failed`. Il n'y a donc rien à attester, et cette population tombe
honnêtement dans « pas d'attestation » que D3 traite déjà. C'est ce qui rend T5
atteignable dans sa forme exacte : *tout tour qui produit un `AgentOutput`
atteste*.

**(b) La précédence a besoin d'un canal.** D7 dit que `--model` gagne sur un
override per-skill, mais `resolve_skill_llm_override(&matched, params.settings,
llm)` ne reçoit aujourd'hui **aucune** information d'appelant. `AgentParams`
gagne donc un champ (`caller_model_override: bool`, `false` partout ailleurs) que
la fonction consulte pour s'abstenir.

**Un seul site d'appel est concerné** (E7b) : `run_agent_inner`. La fonction
`resolve_skill_llm_override` étant partagée, le paramètre qu'elle gagne est passé
`false` depuis `run_team_agent_inner_impl`, **avec le commentaire qui dit
pourquoi** (D7) — un `false` littéral sans raison écrite se lit comme un oubli, et
c'est la forme sous laquelle un futur ticket le « corrigerait » à tort.

### V9 — Documentation

- l'aide de `--model` sur `AskArgs` : dire qu'il atteint la surface d'exécution
  et que son application est fail-closed ;
- `crates/mika-cli/CLAUDE.md` § `mika ask` : `--model` rejoint `--only-skill`
  dans la liste des flags qui **atteignent** spirit, et la note #1727 qui le
  range parmi les abandonnés en transit est corrigée ;
- `crates/mika-agent/CLAUDE.md` § per-turn restriction : la clé sœur, avec
  l'asymétrie fail-soft/fail-closed et sa raison ;
- `CLAUDE.md` racine : surfaces opérateur.

---

## Verification contract

### T1 — La clé voyage (`remote_ask.rs`, unitaire)

`build_send_params` avec un override pose `mika.model_override` à la chaîne
**brute**, non résolue (D4). Avec les trois clés, `metadata` en porte trois et
aucune n'en masque une autre (motif `the_two_metadata_keys_are_independent`).

### T2 — L'absence est byte-identique

Sans override et sans les deux autres clés, `metadata` est **absent** du corps
sérialisé, pas `null` (motif `send_params_without_a_session_serialize_without_metadata`).

### T3 — La lecture serveur est fail-soft

`requested_model_override` rend `None` pour : clé absente, `metadata` absent,
`null`, nombre, tableau, objet, chaîne vide, chaîne d'espaces.

### T4 — L'application est fail-closed (D2, le cœur du ticket)

Un override déclaré et inapplicable **fait échouer le tour** avec l'erreur
nommée. Contrôle négatif explicite : il ne tombe **pas** en silence sur le
provider de config — c'est l'assertion qui rougirait si quelqu'un alignait cette
clé sur la politique fail-soft de `only_skills`.

**Le cas à choisir est « fournisseur sans clé API », et c'est le seul testable
sans réseau** (E7c). Deux pièges de construction, chacun rendant le test vert pour
la mauvaise raison :

- viser « modèle inexistant » : aucune validation préalable ne l'attrape, le test
  ne pourrait passer qu'en atteignant le fournisseur — ce n'est plus un test
  unitaire et il mesure OpenRouter, pas ce correctif ;
- viser une clé absente **sur le rail OpenAI-compatible en s'appuyant sur l'échec
  de `make_provider_for`** : ce constructeur réussit (E7c). Le test rougirait, et
  le remède qu'il suggérerait — « rendre le constructeur faillible » — modifierait
  la classe d'erreur de tout le moteur sur ce rail.

Le test doit donc viser la branche `check_provider_key` explicitement, et une
assertion structurelle doit vérifier qu'elle est appelée **avant** la
construction (V7).

### T5 — L'attestation existe sur tout tour qui produit un `AgentOutput`

Un tour **sans** override atteste quand même le modèle utilisé. Sans cela,
l'absence d'attestation serait ambiguë entre « vieux serveur » et « pas
d'override », et D3 ne tiendrait plus.

Formulation exacte : *tout tour qui produit un `AgentOutput`*, non pas « tout
tour » (V8a). Les trois sites de construction sont couverts, **le repli deadline
compris** — c'est celui qui passe par une fonction auxiliaire et donc le seul qui
peut être oublié sans qu'aucun autre test ne rougisse.

### T6 — Le CLI n'affiche jamais un modèle qu'il n'a pas reçu (D3)

Attestation présente ⇒ `metadata.model` la porte. Attestation absente ⇒ champ
absent en JSON, mention explicite en texte, et **jamais** la valeur locale. Le
test construit un `Task` sans attestation et vérifie que la chaîne demandée
n'apparaît nulle part dans la sortie.

### T7 — Garde structurelle : un seul site de résolution (D5)

Scan de source refusant une seconde définition d'alias/strip-de-préfixe dans
`mika-cli`, allowlist vide. Un test comportemental ne voit pas cette classe : une
copie qui diverge ne rend aucune décision testée fausse, elle fait diverger deux
chemins dont un seul est couvert (précédent : mika#2158, un regex de grooming
copié qui n'a jamais suivi deux élargissements).

### T8 — `mika chat` n'a pas bougé

Les tests existants de `override_model` / `parse_model_override` passent
inchangés après le déplacement — c'est ce qui atteste que V2/V3 sont un
déplacement et non une réécriture.

### T9 — Précédence (D7)

Un tour portant un override d'appelant **et** un skill à override `[llm]` tourne
sous le modèle de l'appelant, au site `run_agent_inner`.

**Le test ne couvre pas le chemin team, et c'est une décision** (E7b) : ce chemin
lit `TeamAgentParams`, qui ne porte pas le champ, donc il n'existe aucun état où
un override d'appelant l'atteindrait. Un test qui y poserait un override
n'exercerait que sa propre plomberie. La garde qui vaut est **structurelle** : le
`false` passé depuis le site team porte un commentaire, et T7 (scan de source)
refuse qu'il devienne un `true` sans canal.

### T10 — L'attestation suit le provider qui a servi, pas celui d'entrée (E7)

Le test qui aurait rougi sur la rédaction précédente de ce plan, donc le test qui
décide que la correction a pris.

Un tour **sans** override d'appelant mais **avec** un skill à override `[llm]`
atteste le modèle **du skill**, pas celui de `agent_state.llm`. Une attestation
prise au site d'entrée passerait T5 (« l'attestation existe ») et T6 (« le CLI
n'invente rien ») en affirmant tranquillement le mauvais modèle : c'est la
répétition de E3 un champ plus loin.

### T11 — L'aller-retour complet, sur la mesure fondatrice

Test d'intégration : un `message/send` portant `mika.model_override` fait tourner
le tour sous ce modèle **et** le `Task` rendu l'atteste. C'est le seul test qui
relie les deux moitiés (propagation D1 + attestation D3) ; T1..T6 les vérifient
chacune de son côté et ne peuvent pas voir un câblage où les deux marchent en
décrivant deux modèles différents.

---

## Fire-Disposition

**FD1 — T4 passe alors que le fail-closed n'est pas câblé.** Signifie que le
contrôle négatif ne discrimine pas. Halte : reconstruire le test contre un
fournisseur réellement dépourvu de clé avant d'aller plus loin — un test qui ne
peut pas rougir sur le défaut central du ticket ne l'atteste pas.

**FD1b — T4 rougit en signalant que `make_provider_for` a réussi.** Ce n'est pas
un défaut du test : c'est E7c, et le test dit la vérité. Ne pas rendre
`OpenAiCompatibleProvider::new` faillible pour le satisfaire — ça changerait la
classe d'erreur de tout le moteur sur le rail qui porte la majorité du trafic,
c'est-à-dire un format de fil, pour un p1 de CLI. Le remède est le
`check_provider_key` explicite de V7, en amont de la construction.

**FD2 — T8 rougit.** Le déplacement V2/V3 a changé un comportement. Ne pas
ajuster le test : c'est `mika chat` qui a régressé, et il fonctionnait.

**FD3 — L'attestation n'est pas lisible parce que `Task.metadata` est écrasé en
aval.** Halte. Ne pas basculer l'attestation vers un `Part` du message : la
sortie du tour est du texte destiné à l'opérateur, y injecter de la télémétrie
recréerait la classe que mika#2270 a dû nettoyer. Établir d'abord qui écrit ce
champ.

**FD5 — T10 rougit en rapportant le modèle de config sur un tour à skill
override.** L'attestation a été câblée au site d'entrée. Ne pas ajuster le test
et ne pas restreindre son périmètre : c'est exactement le défaut E7, et le laisser
passer livrerait un champ qui ment sur la population per-skill tout en portant
l'autorité d'une attestation serveur.

**FD6 — retirée.** Elle décrivait « deux précédences opposées selon le chemin »
comme un risque à surveiller sur les deux sites d'appel. E7b établit que le canal
de la précédence est un champ d'`AgentParams` et que le chemin team lit
`TeamAgentParams` : l'état redouté n'est pas atteignable par typage. La garde
résiduelle est celle de T9 (commentaire au site team + scan de source), pas une
disposition de tir.

**FD7 — l'attestation manque sur le repli deadline seul.** T5 est vert sur les
tours conclus et le champ est `None` après une coupure d'enveloppe. Signifie que
`persist_deadline_fallback` n'a pas reçu la valeur en paramètre (V8a, le seul des
trois sites hors portée locale). Ne pas requalifier la population en « pas
d'attestation légitime » : un tour coupé **a** tourné sous un modèle, et c'est
précisément la population où l'opérateur enquête.

**FD4 — Après déploiement, le body a2a porte toujours le modèle de config alors
que T1..T5 sont verts.** Le corps observé ne vient pas de ce chemin. Halte :
**ne pas élargir la clé**. Établir quel processus l'a émis (l'attestation le dit :
son absence signifie que l'exécutant est un binaire antérieur au correctif — donc
un problème de déploiement, pas de code).

---

## Definition of Done

- [ ] `--model` atteint la surface d'exécution sur les deux chemins de `mika ask`
      (local spirit et `--remote`).
- [ ] Un override inapplicable fait échouer le tour avec un message nommant le
      modèle et le fournisseur, jamais un repli silencieux.
- [ ] `--verbose` rapporte le modèle **attesté par le serveur**, ou rien — et
      l'attestation est prise sur le provider qui a servi le tour, override
      per-skill compris.
- [ ] Un appelant qui ne déclare pas d'override produit le corps A2A d'avant le
      correctif, byte pour byte.
- [ ] La résolution alias/préfixe/clé-API vit à un seul endroit, avec une garde
      structurelle.
- [ ] `mika chat --model` inchangé, attesté par ses tests d'origine.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check` verts.
- [ ] Les quatre surfaces de documentation de V9 à jour, dont la correction de la
      note #1727 qui range `--model` parmi les flags abandonnés en transit.

## Acceptance criteria

*(Le ticket ne porte pas de section `## Acceptance criteria` ; ceux-ci sont
dérivés des Requirements et du Verification contract ci-dessus.)*

- **AC1** — `mika ask --model <id>` (sans `--remote`) : le fournisseur reçoit
  `<id>` résolu contre le `llm_provider` de l'agent exécutant. Vérifié par T1 +
  T5, et par la sonde post-déploiement.
- **AC2** — `mika ask --remote <url> --model <id>` : la clé est posée dans les
  params `message/send`. Le flag n'est plus muet sur ce chemin (E1). Vérifié par
  T1 et par la lecture de `main.rs`.
- **AC3** — Un override déclaré que le serveur ne peut pas appliquer fait
  **échouer** le tour ; il n'est jamais dégradé en « pas d'override ». Le refus
  sur clé API absente est **posé explicitement en amont de la construction du
  provider** (E7c) ; le modèle inexistant échoue à l'appel du fournisseur, sans
  code dédié. Vérifié par T4, contrôle négatif compris.
- **AC4** — La lecture de la clé reste tolérante : absente, `null`, non-chaîne,
  vide ⇒ aucun override, aucun échec. Vérifié par T3.
- **AC5** — Un appelant ne déclarant aucune des trois clés `mika.*` produit un
  corps sans champ `metadata`. Vérifié par T2.
- **AC6** — `--verbose` n'affiche jamais un modèle que le serveur n'a pas
  attesté ; l'attestation est émise sur tout tour, avec ou sans override.
  Vérifié par T5 + T6.
- **AC7** — La sémantique mika#1591 est conservée : le préfixe d'un id ne
  re-dispatche jamais vers un fournisseur natif, et n'est retiré que s'il nomme
  le fournisseur **exécutant**. Vérifié par T7 + T8.
- **AC8** — Un override d'appelant l'emporte sur un override `[llm]` per-skill au
  site `run_agent_inner`, le seul que `message/send` atteint. Le chemin team est
  hors canal par typage et garde son comportement, avec la raison écrite au site
  (E7b, D7). Vérifié par T9.
- **AC9** — L'attestation rapporte le provider qui a **servi** le tour, y compris
  quand un override per-skill l'a substitué en aval du site d'entrée (E7), et sur
  les trois sites de construction d'`AgentOutput` — repli deadline compris.
  Vérifié par T10 + T5.
- **AC10** — Propagation et attestation décrivent le même modèle sur un
  aller-retour réel. Vérifié par T11.

---

## Surfaces opérateur et sonde post-déploiement

**Journal (`$MIKA_SPIRIT_LOG_FILE`) :**

- `a2a_model_override_applied` (INFO) — un tour a tourné sous un modèle
  d'appelant. Régime attendu : rare, corrélé aux campagnes de pré-vol. Un flot
  soutenu signifie qu'un appelant automatisé impose un modèle et mérite d'être
  identifié.
- `a2a_model_override_refused` (WARN) — un override déclaré n'a pas pu être
  appliqué et le tour a échoué (D2). **Régime attendu : zéro ligne.** Toute
  occurrence nomme un modèle ou une clé API manquante ; c'est une erreur de
  frappe d'opérateur ou une clé absente sur l'agent, pas un défaut du canal.

**Sonde, sur la mesure fondatrice du 11/09 :** rejouer
`mika ask --agent mika-arch --model moonshotai/kimi-k2.5 --verbose "ping"`, puis
recouper les deux bords :

```
grep turn_usage $MIKA_SPIRIT_LOG_FILE | jq 'select(.model) | {provider, model}' | tail -1
```

Les deux doivent porter `moonshotai/kimi-k2.5`. **Le recoupement est la sonde, pas
la sortie du CLI seule** : c'est précisément la surface qui mentait (E3), et la
croire sur parole reproduirait la méthode qui a laissé le défaut passer.

**Halte :** si le CLI affiche le modèle demandé et que `turn_usage` en porte un
autre, l'attestation n'est pas lue depuis le serveur — ne pas ajuster
l'affichage, vérifier V6 (D3). Si le CLI n'affiche aucun modèle et que
`turn_usage` porte le bon, le binaire spirit qui tourne est antérieur au
correctif : c'est la population que D3 rend visible, et le remède est un
déploiement.

---

## Hors périmètre, délibérément

- **`--enable-skill` / `--disable-skill`.** La moitié **additive** du canal
  mika#1727, refusée par mika#2363 avec sa raison écrite : elle laisserait tout
  appelant authentifié de `/a2a/{agent}` forcer une skill en `always_on`. Un
  override de modèle est borné au fournisseur configuré de l'agent et aux clés
  dont il dispose — il ne peut pas élargir la surface d'outils du tour. Cette
  différence est ce qui autorise l'un et pas l'autre.
- **Les tokens par run absents du `Task`** (note #1727, `tokens.*` dégradé à
  absent). Même canal, autre mesure, autre ticket.
- **`mika chat`.** In-process, `--model` y fonctionne ; le déplacement V2/V3 doit
  être un no-op pour lui (T8).
- **Le littéral `120s` du rail Anthropic** et l'événement `llm_budget_resolved`
  sur le chemin per-skill (mika#2189 / mika#2293) : angles morts voisins, déjà
  nommés par leurs tickets.
- **La cause du 11/09 côté fournisseur** — ce travail rend l'override effectif et
  vérifiable ; il ne dit rien de ce que les modèles testés valaient.

---

## Revision history

- 2026-09-18 — rédaction initiale (contenu seul ; revue architecte en aval).
- 2026-09-18 — seconde passe de grooming, relecture du plan contre le code. E1 à
  E5 sont confirmées ligne à ligne et inchangées. Trois corrections :
  - **E7 / D8 / V7 / V8a / T10 / FD5 — contradiction interne levée.** D8
    affirmait que l'attestation couvrait le chemin per-skill « par construction »,
    alors que le site nommé en V7 (`a2a.rs`) ne connaît que le provider d'entrée :
    `agent_loop` recalcule `effective_llm` en aval, à deux endroits. Livrée telle
    quelle, cette moitié aurait posé un champ affirmant un modèle qui n'a pas
    servi — la classe E3, déplacée d'un cran et revêtue de l'autorité d'une
    attestation serveur. L'attestation remonte désormais par `AgentOutput`, avec
    le test qui discrimine.
  - **E6 / D9 / V4 — l'asymétrie de `--remote` était passée sous silence.** Les
    deux clés sœurs sont délibérément *non* envoyées sur ce chemin, avec leurs
    raisons écrites dans le code. Faire traverser la troisième exigeait de dire
    ce qui sépare ; c'est écrit, et le commentaire du code est mis à jour plutôt
    que contourné.
  - **V8b / T9 / FD6 — D7 n'avait pas de canal.** `resolve_skill_llm_override` ne
    reçoit aucune information d'appelant ; la précédence demandée exigeait un
    champ `AgentParams` et le traitement conjoint des deux sites d'appel.
  - Ajouts mineurs : durée de vie de l'`Arc` du provider par tour (contrainte de
    `AgentParams.llm: &dyn`), périmètre d'attestation nommé (`message/send`
    synchrone ; `message/stream` et `returnImmediately` hors périmètre, et
    honnêtement lus comme « non attesté »), T11 sur l'aller-retour complet.
- 2026-09-18 — troisième passe. E1 à E6 re-vérifiées ligne à ligne et
  **inchangées** (`build_send_params` aux lignes 82-123, `override_model` →
  `set_provider_model` → lecture par `ask.rs`, `run_remote` sans `model`,
  `make_provider_for` passant bien `llm_timeout_budget()`, point d'intervention
  `Ok(Some(mut task))` présent avec `task` déjà `mut`). Deux corrections majeures,
  toutes deux réduisant le périmètre :
  - **E7b / D7 / D8 / V8 / T9 / FD6 — l'inventaire des deux sites de recalcul
    était faux, et il gonflait le travail.** Les deux appels de
    `resolve_skill_llm_override` ne sont pas deux chemins du même tour : ce sont
    `run_agent_inner` (`AgentParams` → `AgentOutput`) et
    `run_team_agent_inner_impl` (`TeamAgentParams` → `TeamAgentOutcome`).
    `run_a2a_agent` n'atteint que le premier, et `run_silent_inner` n'appelle la
    fonction nulle part. Donc l'attestation par `AgentOutput` couvre exactement le
    périmètre déclaré, et la précédence n'a qu'un site : l'état que FD6 redoutait
    n'est pas atteignable par typage. FD6 est retirée, remplacée par une garde
    structurelle et un commentaire au site team.
  - **E7c / D2 / V7 / T4 / FD1b — le fail-closed reposait sur une validation que
    le rail du ticket ne fait pas.** `create_provider_with_budget` construit
    `OpenAiCompatibleProvider::new` — qui ne rend pas de `Result` et ne consulte
    pas `api_key` — pour les dix variantes OpenAI-compatibles, OpenRouter compris.
    `make_provider_for` réussit donc sur une clé absente, sur le rail des quatre
    passes mesurées. Le refus doit être **posé** par un `check_provider_key`
    explicite en amont de la construction. Et « modèle inconnu du fournisseur »,
    que D2 rangeait avec la clé absente, n'est détectable à aucun endroit avant
    l'appel réseau : les deux populations sont désormais séparées, avec le
    mécanisme qui attrape chacune.
  - Ajouts mineurs : coût chiffré de V8a (trois constructeurs d'`AgentOutput`,
    dont un dans `persist_deadline_fallback` qui demande un changement de
    signature — nouvelle FD7) ; T5 reformulée en « tout tour qui produit un
    `AgentOutput` », les échecs antérieurs au recalcul ne produisant pas de sortie
    à attester ; citations par nom de fonction, les numéros de ligne de la passe
    précédente ayant dérivé de ~285 lignes en un rebase.
