# mika#3138 — les TUI démarrent avant que mika-spirit réponde et meurent au lancement

> Ticket : senara-solutions/mika#3138
> Branche : `fix/3138/deploy-les-tui-d-marrent-avant-que-mika`
> Deux occurrences mesurées : déploiement de `feac6737` le 2026-11-22 à 17:24Z
> (5 TUI mortes), déploiement de `35239e0d` le même jour à 19:22Z (4 TUI mortes).

## Le défaut, tel qu'il se mesure

La chaîne est complète et chacun de ses maillons est dans l'arbre.

**1. `make restart` rend la main avant que le port écoute.** `Makefile:26-30` :

```makefile
restart: ## Restart mika-spirit and mika-gateway (via OpenRC)
	@for bin in mika-spirit mika-gateway; do \
		echo "Restarting $$bin..."; \
		sudo rc-service "$$bin" restart || true; \
	done
```

`rc-service … restart` rend dès que `supervise-daemon` a *lancé* le processus, et
non quand celui-ci écoute. C'est la forme voulue depuis mika#505
(`docs/solutions/integration-issues/makefile-tmux-deploy-silent-noop-on-openrc.md`)
et rien dans `deploy:` (`Makefile:59`) n'attend quoi que ce soit après elle.

**2. Le lanceur part aussitôt** et ouvre les cinq `mika --agent …`.

**3. Chaque TUI meurt sur son premier appel.** `chat.rs::run` appelle
`spawn_agent_worker` avec un `?`, qui appelle `ouvrir_session_par_spirit`
(`crates/mika-cli/src/commands/chat.rs:68`), dont le bras sans `--session-id` est
un **essai unique** :

```rust
let Some(id) = session_id else {
    let miroir = session_remote::ouvrir(base_url, &jeton, agent)
        .await
        .map_err(|probleme| {
            anyhow::anyhow!("La TUI n'a pas pu ouvrir sa session : {probleme}")
        })?;
    return Ok((miroir.session_id, miroir.canonical));
};
```

`session_remote::ouvrir` passe par `post`
(`crates/mika-cli/src/commands/session_remote.rs:201`), dont le bras d'échec de
transport est le **seul producteur** de `status: None` :

```rust
.map_err(|cause| SessionRemoteIssue::Unexpected {
    status: None,
    url: url.clone(),
    body: cause.to_string(),
})?
```

et dont le `Display` (`session_remote.rs:131-134`) rend mot pour mot la phrase du
ticket :

```rust
Self::Unexpected { status, url, body } => match status {
    Some(statut) => write!(f, "mika-spirit returned {statut} for {url}: {body}"),
    None => write!(f, "mika-spirit is unreachable at {url}: {body}"),
},
```

Le `?` remonte dans `run` **avant** `enable_raw_mode` et avant le hook de panique
— l'écran n'existe jamais, ce que « meurent au lancement » décrit exactement.
Depuis mika#1727 la TUI est un client mince du fil : elle n'a aucun mode
autonome où se replier, le refus est donc légitime. Ce qui manque n'est pas un
repli, c'est une **attente**.

### Deux fenêtres, et c'est ce qui décide de la forme de la sonde

`crates/mika-agent/src/server/mod.rs:2002-2008` :

```rust
let app = build_router(state.clone());
let port = settings.spirit_port;
let listener = TcpListener::bind(("0.0.0.0", port)).await?;
info!(port, "mika-spirit listening");
ready.store(true, Ordering::Release);
```

- **Avant le `bind`** : la connexion est refusée immédiatement (`ECONNREFUSED`).
- **Après le `bind`, avant qu'`axum::serve` traite** : le backlog du noyau
  *accepte* la connexion et aucune réponse n'arrive — le `timeout(5 s)` de `post`
  expire. Entre ces deux lignes et le `serve` final il y a tout le travail de
  démarrage (balayage des tâches récurrentes orphelines, `mod.rs:2014`, puis la
  suite).

Les deux fenêtres rendent la **même** classe : `Unexpected { status: None }`. Et
elles condamnent ensemble une sonde TCP : *un `bind` réussi n'est pas une
réponse*. Le commentaire de l'opérateur l'avait déjà établi sur le terrain — « 4
TUI d'agents mortes au lancement, relancées une fois que spirit répondait 200 sur
`/health` ». La seule preuve utilisable est une réponse HTTP.

Note pour qui lira `handlers.rs:83` : `state.ready` n'a **qu'un lecteur**,
`handle_health`, et il est posé à `true` juste après le `bind`. La route
`/api/v1/sessions` ne rend donc jamais 503 pour cause de readiness — il n'existe
aucun statut à réessayer dans cette fenêtre, seulement une absence de réponse.
C'est ce qui rend la classe retenue ci-dessous à la fois nécessaire et
suffisante.

## Requirements

- **R1** — Le démarrage d'une TUI survit à un mika-spirit qui n'écoute pas
  encore : l'ouverture de session est réessayée sous une borne, et la TUI
  démarre dès que le fil répond.
- **R2** — Une seule classe de refus est réessayée : « aucune réponse reçue ».
  Toute réponse du serveur — 401, 404, 409, un autre statut, un 200 illisible —
  arrête le démarrage au premier essai, sans consommer la borne.
- **R3** — Le refus final **dit que l'attente a eu lieu** : nombre de tentatives
  et temps écoulé. Un message identique à celui d'avant ce vol se lirait « rien
  n'a attendu ».
- **R4** — L'attente n'est pas câblée dans le transport partagé : `/clear`,
  `/compact` et les trois sorties de `clore_session_de_sortie` gardent leur
  comportement d'aujourd'hui, à la ligne près.
- **R5** — La borne est pilotable depuis un test, sans mutation
  d'environnement ni horloge virtuelle, et la suite reste rapide.

## Design

### D1 — Le prédicat vit sur le type, dans `session_remote.rs`

Un seul juge du vocabulaire d'erreur. `SessionRemoteIssue` reçoit une méthode
inhérente, dans l'idiome français du fichier :

```rust
impl SessionRemoteIssue {
    /// Vrai quand **aucune réponse n'a été reçue** — rien n'écoute encore, ou le
    /// connect a expiré sur un port lié dont le serveur ne sert pas encore. C'est
    /// la seule classe qu'une attente peut changer.
    pub(crate) fn attendre_peut_changer_lissue(&self) -> bool {
        matches!(self, Self::Unexpected { status: None, .. })
    }
}
```

La classe est exactement celle-là, et le choix se justifie cas par cas :

| classe | réessayée ? | pourquoi |
|---|---|---|
| `Unexpected { status: None }` | **oui** | aucune réponse : le serveur n'est pas là, ou pas encore en train de servir |
| `Denied` (401) | non | un jeton faux ne devient pas juste en attendant, et c'est la panne la plus fréquente après celle-ci : la faire attendre 30 s la rendrait illisible |
| `NotFound` (404) | non | agent ou session inconnus — une réponse |
| `Busy` (409) | non | un tour tient le verrou — une réponse, et attendre un verrou n'est pas attendre un démarrage |
| `Unexpected { status: Some(_) }` | non | une réponse |
| 200 au corps illisible | non | une réponse |

Le discriminant est « une réponse a-t-elle été reçue », jamais « est-ce que ça
pourrait marcher plus tard ». Une réponse reçue **est** une provenance ;
l'attendre à nouveau fabriquerait une provenance fausse, qui coûte plus cher
qu'une provenance absente (motif mika#2331, que le doc-comment de l'enum invoque
déjà pour le `None` de `status`).

### D2 — La boucle vit au site du démarrage, et pas dans `post`

**Refus argumenté, à ne pas rouvrir sans sa prémisse.** `post` est partagé par
les trois portes : `ouvrir`, `clore` et `compacter`
(`session_remote.rs:291/315/352`). Une boucle posée là ferait attendre la borne
entière à `/clear`, à `/compact` et aux **trois** sites de
`clore_session_de_sortie` — c'est-à-dire que quitter une TUI dont le fil est à
terre coûterait le budget complet, sur le chemin précis où l'utilisateur veut
partir. L'attente est une politique du **démarrage**, pas du transport.

Nouvelle fonction dans `chat.rs`, à côté de `ouvrir_session_par_spirit` :

```rust
/// Combien de temps le démarrage accepte d'attendre que le fil réponde, et à
/// quel rythme il redemande (mika#3138).
///
/// Par argument, comme l'URL de base et la source d'environnement de
/// `ouvrir_session_par_spirit` : une durée touche le monde, donc elle se pilote
/// depuis un test. Sans cela, le test du point injoignable paierait le budget
/// réel à chaque exécution de la suite.
pub(crate) struct AttenteOuverture {
    budget: Duration,
    pause: Duration,
}

impl AttenteOuverture {
    /// Celle du démarrage réel.
    pub(crate) fn du_demarrage() -> Self { … }
    /// Celle des tests : un budget en dizaines de millisecondes.
    #[cfg(test)]
    fn courte(budget: Duration, pause: Duration) -> Self { … }
}

/// Ouvre la session en réessayant tant que **rien n'a répondu**.
///
/// Rend le dernier refus, le nombre de tentatives et le temps écoulé : le
/// message final doit pouvoir dire que l'attente a eu lieu.
async fn ouvrir_en_attendant_spirit(
    base_url: &str,
    jeton: &str,
    agent: &str,
    attente: &AttenteOuverture,
) -> Result<CreatedSessionMirror, (SessionRemoteIssue, u32, Duration)>
```

Corps : un `std::time::Instant` au départ ; boucle sur `session_remote::ouvrir` ;
sur `Err`, on sort immédiatement si `!attendre_peut_changer_lissue()`, sinon on
dort `pause` **seulement si** `ecoule + pause < budget`, sans quoi on sort. Le
compteur de tentatives est incrémenté à chaque appel réellement parti. `Instant`
de `std` et non de `tokio`, et aucun test sous `start_paused` : les tests font un
vrai appel réseau contre un endpoint réservé, donc une horloge virtuelle rendrait
l'écoulé incohérent avec ce que la socket vit.

### D3 — Le câblage, et ce que le refus final dit

`ouvrir_session_par_spirit` reçoit un septième paramètre `attente:
&AttenteOuverture` et son bras sans `--session-id` devient :

```rust
let Some(id) = session_id else {
    let miroir = ouvrir_en_attendant_spirit(base_url, &jeton, agent, attente)
        .await
        .map_err(|(probleme, tentatives, ecoule)| {
            anyhow::anyhow!(
                "La TUI n'a pas pu ouvrir sa session après {tentatives} tentative(s) \
                 en {:.1} s : {probleme}",
                ecoule.as_secs_f64()
            )
        })?;
    return Ok((miroir.session_id, miroir.canonical));
};
```

Le septième paramètre ne contredit pas la doctrine du site, il l'applique : le
doc-comment de `ouvrir_session_par_spirit` dit déjà « tout ce qui touche le monde
arrive par argument ». Un seul site de production à mettre à jour,
`spawn_agent_worker` (`chat.rs:290`), qui passe `AttenteOuverture::du_demarrage()`.

**Le texte du refus est le livrable, pas son emballage.** Sans les deux
grandeurs, l'opérateur de la troisième occurrence lirait le message d'avant ce
vol, mot pour mot, et conclurait que rien n'a attendu : non plus une provenance
fausse, mais une **absence** de provenance sur un fait qui a eu lieu. Le bras
`--session-id` (une reprise, jamais une ouverture) et `lire_les_skills`
(délibérément non fatal) ne sont pas touchés.

### D4 — Les deux nombres, et ce qu'ils ne savent pas

`budget = 30 s`, `pause = 1 500 ms`. Pire cas borné à ~35 s (le budget, plus un
dernier `timeout(5 s)` de `post` déjà parti). Dans la fenêtre `ECONNREFUSED`, où
chaque essai revient en microsecondes, cela fait ~20 tentatives ; dans la fenêtre
« port lié, rien ne sert », ~6.

**Ce que le ticket n'atteste pas, et il faut le dire :** la durée réelle de
démarrage de mika-spirit. Les trois minutes entre 17:24Z et la relance de 17:27Z
incluent le temps de réaction d'un humain. Les deux nombres sont donc **choisis**
et non mesurés — et c'est R3 qui ouvre le canal de la mesure : si une troisième
occurrence survient, le refus dira combien de tentatives et combien de secondes
n'ont pas suffi. Un budget trop court ne répare rien ; un budget qui tiendrait un
humain devant un écran noir plus longtemps qu'il ne patiente n'est pas une
réparation non plus. 30 s est le compromis, et il est désormais lisible.

**Pas de nouvelle variable `MIKA_*`**, délibérément : elle devrait être déclarée
dans `docs/operator/environment-variables.md` (jamais dans un `CLAUDE.md`), et un
réglage que personne ne pose est une surface à maintenir sans lecteur. Deux
constantes nommées au site, et un message qui les rend visibles, répondent à la
même question pour un coût nul.

## Périmètre

**Un seul vol, et il tient** : 250 lignes estimées hors `docs/` contre une borne
par vol de 279 (§ *Taille estimée*). Il tient pour une raison de forme, pas de
chance — le vol touche **deux fichiers**, ajoute **un** prédicat sur un type
existant, **une** fonction, **une** structure de paramètres, et aucun schéma,
aucune route, aucune migration, aucun script. La moitié du volume est en tests.

**Le vol ferme l'attendu du ticket.** Celui-ci offre deux remèdes disjoints — « la
cible `deploy` attend que mika-spirit réponde (sonde bornée) avant de lancer les
TUI, **ou** les TUI réessaient l'ouverture de session ». Ce vol livre le second,
et c'est le seul des deux qui répare **toute** chaîne de lancement : un lancement
à la main, un lancement hors `deploy`, un redémarrage de service sans deploy, et
les deux occurrences constatées.

### Hors périmètre, nommé et non pratiqué

**(i) La sonde bornée dans `Makefile::restart`, héritée par `deploy`.** Elle est
atteignable depuis ce dépôt et elle est une défense en profondeur réelle : un
`rc-service restart` suivi d'une attente de réponse HTTP sur `/health` supprime la
fenêtre à la source plutôt qu'au client. La forme est déjà écrite dans l'arbre —
`check-webhook-chain` (`Makefile:77-89`) délègue à un script qui *possède chaque
message qu'il émet*, reste **fail-open** (`|| true`, jamais fatal : un deploy
bloqué par une sonde réseau serait une régression opérationnelle) et sépare NOTE
de WARNING pour qu'une sonde inerte ne se lise pas comme une sonde au repos
(mika#2205). Coût estimé : ~70 lignes de sonde, ~95 lignes de suite offline
jumelle sur le modèle de `scripts/test-smoke-webhook-chain.sh`, ~12 lignes de
Makefile, plus un câblage de la suite dans `make test` (faute de quoi
`suite-wiring-lint` la refuse). Soit ~177 lignes, qui feraient passer le vol à
427 et au-dessus de la borne. Elle n'est pas pratiquée ici.
`Tracked in:` suivi à ouvrir sur `senara-solutions/mika` au merge — *« deploy :
une sonde bornée sur `/health` après `rc-service restart` »*. Aucun numéro n'est
écrit ici : une référence inventée serait une provenance fausse.

**(ii) L'ordonnancement côté lanceur, hors de ce dépôt.** `mika-platform-agents-tmux`
et la cible `deploy` du méta-dépôt sont ceux qui lancent les TUI ; `grep -rln
tmux` ne les trouve pas dans cet arbre, et par mika#2108 le workspace de l'hôte
n'est pas monté dans ce bac à sable — le chemin cité par le ticket est de la
description, pas une adresse. Aucune ligne n'en est écrite ici.
`Tracked in:` suivi à ouvrir sur `senara-solutions/mika-platform` — *« le lanceur
des TUI attend la réponse de mika-spirit, ou délègue l'attente à `make -C mika
deploy` »*. C'est aussi là que se vérifie le seul fait que ce bac à sable ne peut
pas établir : si le méta-`deploy` appelle bien `make -C mika deploy` **avant** de
lancer les TUI, alors (i) suffit ; sinon (i) est hors du chemin et seul ce vol
protège. Cette incertitude est une raison de plus de livrer le réessai d'abord.

## Verification Contract

Cinq tests neufs, un test existant étendu. Tous dans les `mod tests` en place —
aucun fichier de test neuf, aucune fixture hors de l'arbre.

**Dans `session_remote.rs` (le prédicat) :**

1. `mika3138_seule_labsence_de_reponse_se_reessaie` — table des six cas :
   `Unexpected { status: None }` → `true` ; `Unexpected { status: Some(503) }`,
   `Unexpected { status: Some(500) }`, `Denied`, `NotFound`, `Busy` → `false`.
   Le `Some(503)` est le cas qui compte : c'est le statut qu'un lecteur futur
   ajoutera par analogie avec `/health`, et le tableau de D1 dit pourquoi non.

**Dans `chat.rs` (le comportement) :**

2. `mika3138_rien_ne_repond_la_tui_reessaie_jusqua_sa_borne` — contre
   `DeadEndpoint::reserve()` (mika#2569 : le garde est tenu pour toute la durée
   de l'appel, jamais un `bind(:0)` relâché), avec `AttenteOuverture::courte(120
   ms, 20 ms)`. Assertions : tentatives ≥ 3, écoulé ≥ 100 ms, dernier refus
   `Unexpected { status: None, .. }`.
3. `mika3138_un_401_ne_consomme_pas_la_borne` — contre `fake_spirit` armé en
   `UNAUTHORIZED`, même attente courte. Assertions : **exactement une** requête
   vue par le témoin du banc, et écoulé < pause. C'est le contrôle qui sépare
   « la bonne classe est réessayée » de « tout est réessayé ».
4. `mika3138_un_200_ouvre_sans_aucune_pause` — le chemin nominal ne paie rien :
   une requête, écoulé < pause.
5. `mika3138_le_refus_final_dit_combien_de_fois_et_combien_de_temps` — le message
   rendu par `ouvrir_session_par_spirit` contient le compte de tentatives et un
   nombre de secondes. C'est R3, et c'est le test qui rougit si quelqu'un
   « simplifie » le message en celui d'avant le vol.

**Contrôles positifs existants, qui doivent rester verts sans qu'une de leurs
lignes bouge** (ils mesurent que le vol n'a pas déplacé la frontière du refus) :
`mika1727_v2d4a_sans_jeton_interne_le_demarrage_refuse_avant_tout_appel`,
`mika1727_v2d4a_un_jeton_de_tableau_de_bord_ne_suffit_pas` (banc armé, zéro
requête : le jeton se lit **avant** la boucle, donc une variable absente n'attend
pas 30 s), `mika1727_v2d4a_un_401_nomme_la_variable_a_poser`,
`mika1727_v2d4a_une_session_inconnue_est_refusee_en_nommant_son_identifiant`,
`mika1727_v2d4a_une_session_connue_est_reprise_sans_ouverture`, et les deux tests
`canonical`.

**Un seul test existant est modifié :**
`mika1727_v2d4a_un_point_injoignable_nomme_lurl` (`chat.rs:1624`) reçoit
l'attente courte — sans quoi il paierait le budget réel à chaque `cargo test` —
et ses deux assertions (`"unreachable"`, l'URL complète) sont **conservées mot
pour mot**, augmentées de celles de R3. Il passe ainsi de « un essai nomme
l'URL » à « l'attente nomme l'URL, ses tentatives et son écoulé », ce qui est
l'énoncé du vol.

**Portes de dépôt :** `cargo test -p mika-cli`, `cargo clippy -p mika-cli
--all-targets` (le fichier déclare n'avoir plus aucune attente de `dead_code`),
`cargo fmt --check`, `bash scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- Le prédicat existe sur `SessionRemoteIssue`, dans `session_remote.rs`, et c'est
  le seul site qui décide de la classe réessayable.
- `ouvrir_en_attendant_spirit` et `AttenteOuverture` existent dans `chat.rs` ;
  `session_remote::post` est inchangé, à la ligne près.
- `ouvrir_session_par_spirit` réessaie sur le seul bras sans `--session-id`, après
  la lecture du jeton, et son refus porte tentatives et écoulé.
- Les six tests du Verification Contract passent ; les sept contrôles positifs
  existants passent sans modification.
- `cargo test -p mika-cli`, `cargo clippy -p mika-cli --all-targets`, `cargo fmt
  --check` et `scripts/verify-pipeline.sh origin/main` passent.
- La durée de la suite `mika-cli` n'augmente pas de plus d'une seconde (les tests
  d'attente tiennent en dizaines de millisecondes).
- Les deux moitiés hors périmètre sont nommées dans le corps de PR avec leurs
  lignes `Tracked in:`, et aucune ligne de code ne les pratique.

## Acceptance criteria

- **AC1** — Une TUI lancée pendant que mika-spirit démarre **ne meurt pas** : elle
  réessaie l'ouverture de session et démarre dès que le fil répond. Attesté par
  le test 2 pour la branche d'attente et par les tests 4 et les deux contrôles
  `canonical` pour la branche qui aboutit.
- **AC2** — La seule classe réessayée est « aucune réponse reçue ». Un 401, un
  404, un 409, un 500, un 503 et un 200 illisible arrêtent le démarrage au
  **premier** essai. Attesté par les tests 1 et 3.
- **AC3** — Le refus final nomme le nombre de tentatives et le temps écoulé ; il
  n'est jamais identique au message d'avant ce vol. Attesté par les tests 5 et
  par la version étendue de `mika1727_v2d4a_un_point_injoignable_nomme_lurl`.
- **AC4** — `MIKA_INTERNAL_TOKEN` absent refuse **avant** tout appel et **sans**
  attendre : zéro requête vue par le banc, écoulé négligeable. Attesté par les
  deux contrôles positifs du jeton, inchangés.
- **AC5** — `session_remote::post` n'est pas modifié, donc `/clear`, `/compact` et
  les trois sorties de `clore_session_de_sortie` gardent leur comportement :
  quitter une TUI dont le fil est à terre ne coûte pas le budget d'attente.
  Attesté par `git diff origin/main` sur la fonction `post` et par la suite
  `rewind_remote`/`clore` existante, verte et non modifiée.
- **AC6** — Le bras `--session-id` — une reprise, jamais une ouverture — est
  inchangé, y compris ses trois refus (session inconnue, session d'un autre
  agent, base illisible). Attesté par les tests de reprise existants, non
  modifiés.
- **AC7** — Aucune variable d'environnement n'est introduite :
  `git diff origin/main` ne touche pas `docs/operator/environment-variables.md`
  ni `.env.example`.
- **AC8** — Aucune ligne ne touche `Makefile`, `scripts/` ou un fichier hors
  `crates/mika-cli/` : le périmètre du vol est exactement deux fichiers.
- **AC9** — Chaque test neuf porte un doc-comment nommant le fait mesuré et le
  ticket (mika#3138).
- **AC10** — `cargo test -p mika-cli`, `cargo clippy -p mika-cli --all-targets`,
  `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` passent.

## Fire-Disposition

Ce vol livre des détecteurs : cinq tests comportementaux dont le chemin de succès
est « le réessai n'a **pas** eu lieu sur une classe qui a reçu une réponse » et
« le refus ne **tait** pas l'attente ».

**Option retenue : (a) — exception nommée en allowlist, et l'allowlist est vide
par construction.**

La population des violations existantes est **connue, finie et close** : elle
compte **une** entrée, le test `mika1727_v2d4a_un_point_injoignable_nomme_lurl`,
qui pinne aujourd'hui l'essai unique en n'asserant que l'URL et le mot
`unreachable`. Il ne reçoit **pas** d'entrée d'exception : il est **étendu dans le
commit qui arme le détecteur** (§ Verification Contract), ses deux assertions
conservées mot pour mot et augmentées de celles de R3. C'est la forme la plus
auto-nettoyante possible de (a) — il n'y a aucune entrée à faire expirer parce
qu'il n'y a aucune entrée.

L'assertion auto-nettoyante qui tient la borne du détecteur est la **paire** que
mika#2306 demande, et ses deux moitiés tirent en sens opposés : le test 3 (un 401
ne consomme pas la borne, exactement une requête) rougit si un éditeur futur
élargit le réessai à « tout refus » ; les tests 1 et 4 plus les sept contrôles
positifs existants rougissent si quelqu'un le rétrécit jusqu'à ne plus réessayer
du tout, ou fait payer une pause au chemin nominal. Un détecteur qui ne peut
devenir vert ni en se taisant ni en s'élargissant.

**Aucune garde structurelle n'est livrée** — ni lint, ni scan de source, ni garde
CI — et c'est un refus, pas un report. Le prédicat de ce vol est un `matches!`
dans une méthode inhérente, pas une invariante de dépôt ; un balayage de source
sur `status: None` serait vert par construction dès que le nom de la variante
change, donc vert en mesurant son propre vide (classe mika#2205). La propriété
« un seul site décide de la classe réessayable » est tenue par la **visibilité**
du prédicat (`pub(crate)`, un seul appelant) et non par un scan.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-cli/src/commands/session_remote.rs` — prédicat + doc-comment | 26 |
| `crates/mika-cli/src/commands/session_remote.rs` — test 1 (table des six cas) | 30 |
| `crates/mika-cli/src/commands/chat.rs` — `AttenteOuverture` + `ouvrir_en_attendant_spirit` + doc | 70 |
| `crates/mika-cli/src/commands/chat.rs` — câblage (`ouvrir_session_par_spirit`, `spawn_agent_worker`) + message de refus | 24 |
| `crates/mika-cli/src/commands/chat.rs` — tests 2 à 5 + extension du test existant | 100 |

Total estimé : 250 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Le vol tient sous les deux bornes : 250 contre 279 par vol, et très loin des 1000
par PR. Il n'y a donc **pas** de découpage en vols, et § *Périmètre* dit pourquoi
le périmètre retenu ferme malgré tout l'attendu du ticket : celui-ci offre deux
remèdes disjoints par un « ou », et c'est le remède livré ici qui couvre toutes
les chaînes de lancement. Les deux moitiés laissées dehors sont chiffrées
(~177 lignes pour (i)) et nommées, chacune avec sa ligne `Tracked in:`.

## Ordre de commit

1. **`fix(3138): seule l'absence de réponse se réessaie`** — le prédicat
   `attendre_peut_changer_lissue` sur `SessionRemoteIssue` et son test de table.
   Commit autonome : aucun appelant encore, et `cargo clippy -p mika-cli
   --all-targets` reste vert parce que le test est le lecteur.
2. **`fix(3138): le démarrage de la TUI attend que mika-spirit réponde`** —
   `AttenteOuverture`, `ouvrir_en_attendant_spirit`, le câblage dans
   `ouvrir_session_par_spirit` et `spawn_agent_worker`, le message de refus, les
   tests 2 à 5, et l'extension du test existant. C'est le commit qui arme le
   détecteur et retourne la seule violation existante — les deux gestes dans le
   même commit, comme le demande la disposition (a) ci-dessus.

Plancher : le commit 1 seul laisse le défaut entier mais le dépôt vert ; c'est le
commit 2 qui ferme AC1 à AC6. Les deux sont dans la même PR.
