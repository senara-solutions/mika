# mika#3129 — `/model` vers un autre provider envoie un override que le fil ne sait pas servir

> Ticket : senara-solutions/mika#3129
> Branche : `fix/3129/tui-model-vers-un-autre-provider-envoie`
> Source du ticket : commentaire 51 du parking #2983

## Le défaut, tel qu'il se mesure

Le corps du ticket nomme deux lignes. La première est le défaut ; la seconde est
la raison pour laquelle il **ne se voit pas**.

`crates/mika-cli/src/tui/commands/handlers.rs:674` — dans l'arm de bascule de
`handle_model`, le `SetModel` part **inconditionnellement**, y compris quand
`switch_model` vient de changer de provider dans `config.toml` :

```rust
let target_provider: mika_common::llm::ProviderKind =
    result.provider.parse().unwrap_or(app.provider);
// …
let worker_model =
    crate::commands::provider::format_worker_model(target_provider, &result.model);
app.model = display_id.clone();
if app.agent_tx.send(AgentRequest::SetModel { model: worker_model }).is_err() { … }
```

Le worker pose ensuite cette chaîne brute sous `mika.model_override` sur **chaque
tour suivant** (`commands/chat_turn.rs`, vol V3b-1 de mika#1727). Côté serveur,
`crates/mika-common/src/llm/model_override.rs:72` ne retire le préfixe **que**
lorsqu'il nomme le provider *configuré* :

```rust
if let Some((prefix, rest)) = resolved.split_once('/')
    && let Ok(parsed) = prefix.parse::<ProviderKind>()
    && parsed == configured
{
    return (configured, rest.to_string());
}
(configured, resolved)
```

Donc après `/model deepseek` depuis un spirit en `anthropic`, la chaîne
`deepseek/deepseek-chat` traverse intacte, `resolve_model_override` rend
`(Anthropic, "deepseek/deepseek-chat")`, et `check_provider_key` **passe** — le
provider en service a bien sa clé. **Rien ne refuse.** Chaque tour suivant part
chez Anthropic avec un identifiant de modèle qu'Anthropic ne connaît pas, et
meurt sur un 400/404 du provider, jusqu'au prochain redémarrage de mika-spirit.

La réponse rendue à l'opérateur dit l'inverse :

```
Switched to deepseek/deepseek-chat. (switched provider to deepseek in config.toml
— mika-spirit takes it at its next restart, for every turn of the agent)
```

Elle annonce une bascule différée et un modèle appliqué. Le modèle **est** parti ;
c'est le provider qui n'a pas suivi, et le couple des deux n'est servi par
personne.

**Le test qui pinne le défaut est déjà au dépôt**, et il nomme le mode de panne
sans le savoir : `handlers.rs:1905` `test_model_direct_name_sets_model` fait
`/provider deepseek` (qui écrit `config.toml` et ne bouge rien d'autre, V3b-2),
puis `/model deepseek-reasoner`, et atteste qu'un `SetModel`
`deepseek/deepseek-reasoner` part au worker — alors qu'`app.provider` vaut
toujours `Anthropic`. Et dans ce montage `result.provider_changed` vaut **false**,
puisque `config.toml` portait déjà `deepseek` avant et après.

C'est la mesure qui fixe le prédicat du correctif : **`provider_changed` compare
`config.toml` avant/après, pas ce que le fil sert.** Le défaut atteint deux
populations, dont une seule est visible par ce champ :

| geste | `provider_changed` | `target_provider != app.provider` | le fil est-il cassé ? |
|---|---|---|---|
| `/model deepseek` depuis spirit anthropic | `true` | `true` | oui |
| `/provider deepseek` puis `/model deepseek-reasoner` | **`false`** | `true` | **oui** |
| `/model claude-opus-4-6` depuis spirit anthropic | `false` | `false` | non |
| `/model sonnet` depuis spirit openrouter | `false` | `false` | non |

`app.provider` est, depuis le vol V3b-2 de mika#1727, le provider que mika-spirit
a chargé au lancement de la TUI (`chat.rs:691`, posé depuis
`worker._ctx.settings.llm_provider`) et que ni `/provider` ni `/model` ne
déplacent, délibérément. C'est la seule valeur de ce processus qui réponde à la
question « qui sert les tours de cette session ».

## Requirements

**R1** — Un `/model` dont le provider cible diffère de celui que le fil sert
n'envoie **aucun** `AgentRequest::SetModel`.

**R2** — Dans ce cas, `app.model` **ne bouge pas**. Afficher le modèle cible
pendant que les tours sont servis par un autre serait une fausse provenance
(mika#2331) — exactement ce que `/provider` refuse déjà de faire.

**R3** — La réponse nomme (a) que la bascule est écrite dans `config.toml`,
(b) que mika-spirit la prend à son prochain démarrage, et (c) que les tours de
**cette** session restent servis par le provider en cours. Elle n'affirme pas que
le modèle est appliqué.

**R4** — Le prédicat est `target_provider != app.provider`, **jamais**
`result.provider_changed`. La ligne du milieu du tableau ci-dessus est la
mesure qui l'exige.

**R5** — Les gestes qui ne changent pas de provider sont inchangés, octet pour
octet : `SetModel` préfixé par le provider cible, `app.model` à jour, même
réponse. Cela couvre le même-provider nu (`claude-opus-4-6` sous anthropic), les
identifiants à slash d'OpenRouter (`qwen/qwen-plus`) et les alias résolus sous
OpenRouter (`sonnet` → `anthropic/claude-sonnet-4-6`).

**R6** — `crates/mika-common/src/llm/model_override.rs` n'est **pas** modifié.

**R7** — Le commentaire de `handle_provider` qui affirme qu'un override
cross-provider « est refusé fermement » est corrigé : la mesure ci-dessus montre
qu'il ne l'est pas. Un commentaire qui annonce une garde inexistante est ce qui a
laissé le `SetModel` de `handle_model` passer trois revues.

## Design

### Le correctif vit dans `handle_model`, et nulle part ailleurs

L'arm `else` de `handle_model` (`handlers.rs:655-698`) se scinde en deux selon le
prédicat de R4 :

```rust
Ok(result) => {
    let target_provider: mika_common::llm::ProviderKind =
        result.provider.parse().unwrap_or(app.provider);

    if target_provider != app.provider {
        // mika#3129 — la cible n'est pas le provider que le fil sert.
        // Un `SetModel` partirait brut sur `mika.model_override` et
        // `parse_model_override` le rendrait tel quel au provider EN COURS
        // (mika#1591) : chaque tour suivant mourrait sur un 400 du provider,
        // sans que rien refuse. On écrit, et rien de plus — la doctrine de
        // `handle_provider`. `app.model` ne bouge pas : il nommerait un
        // modèle que le fil ne porte pas (mika#2331).
        let mut out = format!(…);
        for w in &result.warnings { … }
        return out;
    }

    // … le chemin inchangé : display_id, worker_model, app.model, SetModel.
}
```

Trois choses sont délibérées dans cette forme :

- **`format_display_model` et `format_worker_model` ne sont plus appelés** sur la
  branche cross-provider. Le premier n'a plus de destinataire (`app.model` ne
  bouge pas) et le second produirait précisément la chaîne qu'on refuse
  d'envoyer. La réponse nomme `result.provider` et `result.model` directement,
  comme le fait déjà `handle_provider`.
- **`app.needs_redraw` n'est pas posé.** Rien n'a changé à l'écran : ni le
  modèle, ni le provider. Le poser annoncerait une mutation qui n'a pas eu lieu.
- **Les `result.warnings` sont conservés.** Ce sont les échecs d'écriture de
  `config.toml` (« failed to save provider »), et ils portent sur le seul geste
  que cette branche accomplit encore. Les perdre rendrait une bascule non
  persistée indiscernable d'une bascule écrite.

### Pourquoi le correctif n'est pas côté serveur

`model_override.rs:72` est la seconde ligne que le ticket cite, et la tentation
est de l'y poser : « si le préfixe nomme un provider connu différent du
configuré, refuse ». **Cette garde casserait OpenRouter**, et c'est irréparable
depuis ce site.

Les identifiants d'OpenRouter sont préfixés par le **vendeur**, et ces préfixes
parsent en `ProviderKind` : `anthropic/claude-sonnet-4-6`, `qwen/qwen-plus`,
`moonshotai/kimi-k2.5`. Sous `llm_provider = "openrouter"`, le prédicat « le
préfixe nomme un provider ≠ configuré » est **vrai** sur chacun d'eux — ce sont
pourtant les identifiants exacts qu'OpenRouter attend, et c'est la raison même
pour laquelle mika#1591 préserve la chaîne entière au lieu de dépréfixer.

Le discriminant qui sépare les deux cas est `ProviderKind::model_names_contain_slash()`,
et il n'est lisible que du **côté client** : c'est `parse_provider_model`
(`commands/model.rs:148`) qui s'en sert déjà pour décider si un slash est un
vendeur ou une bascule de provider. Le serveur, lui, reçoit une chaîne brute sans
cette intention. Le doc-comment de `model_override.rs` le dit de son côté : *« Whether
the provider actually serves that model id. No local validation exists for it at
any layer. »* Ce n'est pas une omission à combler ici — c'est la frontière.

Donc : **le client ne pose pas un override que le serveur devra refuser** (R1),
plutôt que **le serveur devine une intention que seul le client avait** (R6).

### Ce que ce vol ne ferme pas, nommément

`app.provider` est lu dans `config.toml` au lancement de la TUI, non attesté par
mika-spirit. La séquence « éditer `config.toml` à la main, relancer la TUI sans
redémarrer le daemon » rend donc un `app.provider` qui n'est pas celui qui sert,
et le prédicat de R4 lit faux dans cette fenêtre. La surface d'attestation existe
— `GET /api/v1/agents/{id}/budget` (mika#2457 / mika#2328), qui sert le record
figé à l'`init_agent` — mais la câbler au lancement de la TUI est un livrable
distinct, au-dessus de la borne par vol de ce ticket. Le périmètre de ce vol est
la population mesurée du ticket : les deux gestes `/provider` et `/model` tapés
dans une TUI en vol, qui est précisément celle où `app.provider` est juste.

**Tracked in:** à ouvrir — « `app.provider` est attesté par mika-spirit, non lu
dans `config.toml` au lancement » (consommateur : `chat.rs::spawn_agent_worker`,
porte : `commands::budget`).

## Périmètre

Modifié :
- `crates/mika-cli/src/tui/commands/handlers.rs` — l'arm de bascule de
  `handle_model` (R1-R5), le commentaire de `handle_provider` (R7), et le module
  `tests` du même fichier.

Explicitement **hors** périmètre :
- `crates/mika-common/src/llm/model_override.rs` (R6, voir le § ci-dessus).
- `crates/mika-cli/src/commands/model.rs` — `switch_model` écrit `config.toml` et
  c'est ce qu'on veut qu'il continue de faire ; `provider_changed` reste dans son
  `ModelSwitchResult` pour `mika model <name>` en ligne de commande, qui n'a pas
  de worker à qui envoyer quoi que ce soit.
- `crates/mika-cli/src/commands/provider.rs` — `format_worker_model` et
  `format_display_model` sont corrects ; c'est leur appel qui était mal gardé.
- `app.provider` et sa provenance (voir le `Tracked in:` ci-dessus).

## Verification Contract

Tests retournés dans `handlers.rs` (module `tests`), chacun avec le doc-comment
qui dit **pourquoi l'inversion plutôt qu'un test neuf à côté** — la forme de
`test_provider_switch_writes_config_and_sends_no_setmodel` (`handlers.rs:2438`),
le précédent du vol V3b-2 :

1. `test_model_cross_provider_alias_switches_provider` (`:2351`) — devient
   `test_model_cross_provider_alias_writes_config_and_sends_no_setmodel` :
   après `/model deepseek` depuis anthropic, `drain_requests` est **vide**,
   `app.model` vaut toujours `claude-sonnet-4-6`, `app.provider` toujours
   `Anthropic`, `config.toml` porte `llm_provider = "deepseek"`, et la réponse
   contient `next restart`.
2. `test_model_direct_name_sets_model` (`:1905`) — devient
   `test_model_direct_name_apres_un_provider_nenvoie_rien` : c'est la ligne du
   milieu du tableau, celle où `provider_changed` vaut `false`. Même assertions.
   Son doc-comment porte la mesure : ce test est la preuve que le prédicat ne
   peut pas être `provider_changed`.
3. `test_model_direct_with_provider_prefix` (`:2390`) — son assertion de sortie
   (`output.contains("deepseek/deepseek-reasoner")`) reste vraie : la nouvelle
   réponse nomme le modèle cible. Elle gagne l'assertion négative du `SetModel`
   et celle sur `app.model`.

Test neuf :

4. `mika3129_le_predicat_est_le_provider_du_fil_pas_celui_de_config` — le contrôle
   positif du prédicat : `app.provider` posé à `DeepSeek` (le fil sert deepseek),
   `config.toml` en `deepseek`, `/model deepseek-reasoner` → le `SetModel`
   **part**, préfixé `deepseek/`. Sans lui, R1 serait indiscernable de « `/model`
   n'envoie plus jamais rien ».

Contrôles positifs **qui doivent rester verts sans être touchés** — ce sont eux
qui bornent R5, et leur verdeur est la mesure que le correctif ne déborde pas :

- `test_model_switch_worker_gets_provider_prefix` (`:2543`) — même provider nu,
  `anthropic/claude-opus-4-6` part.
- `test_model_alias_on_openrouter_uses_provider_prefix` (`:2621`) —
  `openrouter/anthropic/claude-sonnet-4-6` part. **C'est le test qui aurait
  rougi** si le correctif avait été posé dans `model_override.rs`.
- `test_model_on_openrouter_keeps_slash_model_name` (`:2674`) —
  `openrouter/qwen/qwen-plus` part.
- `test_provider_set_model_still_sends_setmodel` (`:2482`) — `/provider set model`
  envoie toujours, préfixé `app.provider`.
- `test_provider_switch_writes_config_and_sends_no_setmodel` (`:2448`) — la
  doctrine miroir, inchangée.

Pipeline : `cargo test -p mika-cli`, `cargo clippy -p mika-cli --all-targets`,
`cargo fmt --check`, puis `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- [ ] `handle_model` ne pose aucun `SetModel` quand `target_provider != app.provider`.
- [ ] `app.model` et `app.provider` sont inchangés sur cette branche, et
      `needs_redraw` n'est pas posé.
- [ ] La réponse nomme `config.toml`, le prochain démarrage de mika-spirit, et le
      fait que les tours en cours restent servis par le provider courant.
- [ ] Les `result.warnings` sont toujours rendus sur cette branche.
- [ ] `model_override.rs` ne figure pas dans le diff.
- [ ] Les trois tests nommés ci-dessus sont retournés, chacun avec un doc-comment
      qui dit ce que l'inversion retire.
- [ ] Le contrôle positif du prédicat (test 4) existe et passe.
- [ ] Les cinq contrôles positifs listés passent **sans modification**.
- [ ] Le commentaire de `handle_provider` ne dit plus « refusé fermement ».
- [ ] `cargo test -p mika-cli`, `cargo clippy -p mika-cli --all-targets` et
      `cargo fmt --check` passent.
- [ ] `scripts/verify-pipeline.sh origin/main` passe.
- [ ] Le corps de PR porte `Closes #3129` et
      `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

Le corps de mika#3129 ne porte pas de section `## Acceptance criteria` — deux
lignes de source. Les critères ci-dessous sont dérivés des Requirements et du
Verification Contract ci-dessus.

- **AC1** — Depuis une TUI dont `app.provider` vaut `Anthropic`, `/model deepseek`
  n'envoie **aucun** `AgentRequest::SetModel` au worker. Mesuré par
  `drain_requests(&mut rx).is_empty()`.
- **AC2** — Dans le cas d'AC1, `app.model` conserve sa valeur d'avant la commande
  et `app.provider` reste `Anthropic`. Mesuré par deux `assert_eq!`.
- **AC3** — Dans le cas d'AC1, `config.toml` porte `llm_provider = "deepseek"` :
  la persistance n'est pas sacrifiée au refus d'envoi. Mesuré par lecture du
  fichier.
- **AC4** — Dans le cas d'AC1, la réponse contient `next restart` et ne contient
  pas de formulation affirmant que le modèle est en service pour les tours en
  cours.
- **AC5** — Après `/provider deepseek` puis `/model deepseek-reasoner` depuis un
  `app.provider` à `Anthropic` — le cas où `result.provider_changed` vaut
  `false` — AC1 à AC4 tiennent identiquement. C'est le critère qui refuse
  `provider_changed` comme prédicat.
- **AC6** — Avec `app.provider` à `DeepSeek` et `config.toml` en `deepseek`,
  `/model deepseek-reasoner` envoie un `SetModel` portant exactement
  `deepseek/deepseek-reasoner`. Le refus est conditionnel, pas global.
- **AC7** — Les cinq contrôles positifs du Verification Contract passent sans
  qu'une seule de leurs lignes soit modifiée, les deux cas OpenRouter inclus.
- **AC8** — `git diff origin/main --name-only` ne contient pas
  `crates/mika-common/src/llm/model_override.rs`.
- **AC9** — Chaque test retourné porte un doc-comment nommant le fait que
  l'inversion retire et le ticket (mika#3129).
- **AC10** — `cargo test -p mika-cli`, `cargo clippy -p mika-cli --all-targets`,
  `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` passent.

## Fire-Disposition

Ce vol livre des détecteurs : quatre tests unitaires comportementaux dont le
chemin de succès est « le `SetModel` interdit n'est pas parti ».

**Option retenue : (a) — exception nommée en allowlist, et l'allowlist est vide
par construction.**

La population des violations existantes est **connue, finie et close** : ce sont
les trois tests qui pinnent aujourd'hui le défaut
(`test_model_cross_provider_alias_switches_provider`,
`test_model_direct_name_sets_model`, `test_model_direct_with_provider_prefix`).
Ils ne reçoivent pas d'entrée d'exception — ils sont **retournés dans le commit
qui arme le détecteur**, ce qui est la forme la plus auto-nettoyante possible de
(a) : il n'y a aucune entrée à faire expirer parce qu'il n'y a aucune entrée.

L'assertion auto-nettoyante qui tient la borne du détecteur est le contrôle
positif du prédicat (test 4, AC6) doublé des cinq contrôles positifs d'AC7 : si
un éditeur futur élargit le refus en « `/model` n'envoie plus rien », ces six
tests rougissent **par nom**, chacun disant lequel des gestes légitimes il vient
de couper. C'est la paire que mika#2306 demande — un détecteur qui ne peut pas
devenir vert en se taisant.

Aucune garde structurelle (lint, scan de source, garde CI) n'est livrée : le
prédicat de ce vol est un `if` dans une fonction, pas une invariante de dépôt, et
un balayage de source sur `SetModel` serait vert par construction dès que le
nom change (classe mika#2205).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-cli/src/tui/commands/handlers.rs` (production : arm de `handle_model`, commentaires, R7) | 45 |
| tests retournés (`handlers.rs`, 3 tests + doc-comments) | 75 |
| test neuf du prédicat (`handlers.rs`, mika3129_…) | 55 |

Total estimé : 175 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol tient, et voici pourquoi.** 175 est sous la borne par vol de 279 et
loin sous le seuil par PR de 1000. Le volume est concentré sur **un seul
fichier** : aucun crate traversé, aucune signature publique changée, aucune
migration de base, aucun appelant à router. La production tient en un `if` et sa
branche courte — le reste du total est du test, dont les trois quarts sont des
inversions de tests existants (le montage `test_app()` / `drain_requests()` est
déjà là, on retourne des assertions, on n'en écrit pas l'échafaudage). Les deux
livrables qui auraient fait déborder sont nommés hors périmètre : l'attestation
d'`app.provider` par mika-spirit (le `Tracked in:` du § Design) et toute garde
côté `model_override.rs` (R6).

## Ordre de commit

1. **`fix(tui): /model vers un autre provider n'envoie plus d'override (mika#3129)`**
   — le prédicat `target_provider != app.provider`, la branche courte, les
   commentaires (R1-R5, R7), et dans le **même** commit les trois tests retournés
   plus le contrôle positif du prédicat. **C'est le plancher mergeable** : il
   ferme AC1 à AC9 et ne peut pas être scindé — un commit qui poserait le
   prédicat sans retourner les tests laisserait la suite rouge, et un commit qui
   retournerait les tests d'abord la laisserait rouge dans l'autre sens.

2. **PR** — corps portant `Closes #3129`,
   `Code review: delegated to mika-qa (mika#2694)`, la ligne `Tracked in:` du
   suivi d'attestation d'`app.provider`, et le tableau des quatre populations du
   § *Le défaut, tel qu'il se mesure* (c'est ce tableau qui explique au relecteur
   pourquoi le prédicat n'est pas `provider_changed`).

Il n'y a pas de second commit : le vol n'a qu'un livrable et son plancher est
son plafond.
