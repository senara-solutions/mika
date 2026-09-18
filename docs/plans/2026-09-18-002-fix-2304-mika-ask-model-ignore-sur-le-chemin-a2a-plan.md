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

### E6 — Ce qui n'est PAS établi

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

Un override de modèle que le serveur ne peut pas appliquer — modèle inconnu du
fournisseur configuré, fournisseur sans clé API — **fait échouer le tour**. Il
n'est jamais dégradé en « pas d'override ».

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
  pas ;
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

### D7 — `--model` gagne sur un override per-skill

Un opérateur qui nomme un modèle ne veut pas qu'un skill le remplace en silence :
ce serait le même faux vert déplacé d'un cran. `resolve_skill_llm_override`
s'abstient quand le tour porte un override explicite d'appelant.

L'attestation (D3) reste le filet : si cette précédence était mal câblée,
l'opérateur lirait le modèle réel plutôt que celui qu'il a demandé.

### D8 — Angle mort déclaré : le per-skill continue de n'attester que par D3

Comme mika#2293 l'a fait pour `llm_budget_resolved`, l'angle mort est écrit au
site d'émission plutôt que découvert : l'événement de configuration n'est pas
étendu au chemin per-skill par ce ticket. L'attestation du modèle effectif, elle,
couvre ce chemin par construction puisqu'elle est prise sur le provider qui a
servi le tour.

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
**reçoit enfin le modèle** — aujourd'hui il ne le reçoit pas (E1).

Le `Task` rendu expose son modèle attesté via un accesseur (lecture de
`Task.metadata`), pour que les deux surfaces d'affichage lisent la même chose.

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
  `requested_only_skills` ;
- `run_a2a_agent` construit le provider par tour via `make_provider_for` (E5) et
  **échoue le tour** si la construction échoue (D2), en nommant le modèle et le
  fournisseur ;
- le modèle effectif (`provider.provider_name()` / `model_name()`) est posé dans
  le `Task` au point d'intervention mika#2270, sur **tout** tour ;
- un événement INFO `a2a_model_override_applied` (agent, task_id, demandé,
  résolu), sœur de `a2a_only_skills_applied`.

### V8 — `crates/mika-agent/src/agent_loop/mod.rs`

`resolve_skill_llm_override` s'abstient sous override d'appelant (D7).

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

Un override déclaré et inapplicable (fournisseur sans clé API) **fait échouer le
tour** avec l'erreur nommée. Contrôle négatif explicite : il ne tombe **pas** en
silence sur le provider de config — c'est l'assertion qui rougirait si quelqu'un
alignait cette clé sur la politique fail-soft de `only_skills`.

### T5 — L'attestation existe sur tout tour, override ou non

Un tour **sans** override atteste quand même le modèle utilisé. Sans cela,
l'absence d'attestation serait ambiguë entre « vieux serveur » et « pas
d'override », et D3 ne tiendrait plus.

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
sous le modèle de l'appelant.

---

## Fire-Disposition

**FD1 — T4 passe alors que le fail-closed n'est pas câblé.** Signifie que le
contrôle négatif ne discrimine pas. Halte : reconstruire le test contre un
fournisseur réellement dépourvu de clé avant d'aller plus loin — un test qui ne
peut pas rougir sur le défaut central du ticket ne l'atteste pas.

**FD2 — T8 rougit.** Le déplacement V2/V3 a changé un comportement. Ne pas
ajuster le test : c'est `mika chat` qui a régressé, et il fonctionnait.

**FD3 — L'attestation n'est pas lisible parce que `Task.metadata` est écrasé en
aval.** Halte. Ne pas basculer l'attestation vers un `Part` du message : la
sortie du tour est du texte destiné à l'opérateur, y injecter de la télémétrie
recréerait la classe que mika#2270 a dû nettoyer. Établir d'abord qui écrit ce
champ.

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
- [ ] `--verbose` rapporte le modèle **attesté par le serveur**, ou rien.
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
  **échouer** le tour ; il n'est jamais dégradé en « pas d'override ». Vérifié
  par T4, contrôle négatif compris.
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
- **AC8** — Un override d'appelant l'emporte sur un override `[llm]` per-skill.
  Vérifié par T9.

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
