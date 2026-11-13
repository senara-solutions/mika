# Vol V2b-2b-4 — la reconnexion et le recul du SSE (mika#1727)

> Le fil des tâches se rouvre de lui-même, après une fermeture comme après un
> échec d'ouverture, en attendant un délai qui double jusqu'à un plafond. Il
> s'arrête quand la TUI lâche la couture.

Branche : `feat/1727/tui-v2b2b4-reconnexion-et-recul-du-sse`, neuve depuis `main`.
La PR portera `Refs mika#1727`. Jamais `Closes`, `Fixes` ni `Resolves` : le
ticket se ferme au merge de V4.

---

## 0. Ce qui est mesuré, avant toute conception

Chaque ligne de cette section est une mesure faite dans ce worktree sur
`0d34872e2`, pas un souvenir. Elles décident de la forme du vol.

**La borne est entière.** `git fetch origin main` puis `git rev-parse FETCH_HEAD`
donne `0d34872e259754b6d8f140c27a7ad246654bf3d4` ; `git rev-parse HEAD` donne le
même, et `git diff --numstat origin/main...HEAD` est vide. Zéro ligne consommée,
les 279 sont disponibles.

**`open_for` ouvre avant de lancer sa tâche, et c'est la cause.**
`tasks.rs:739-757` : `TaskEventStream::open(…).await.ok()?` **précède** le
`tokio::spawn`. L'échec d'ouverture est donc fatal par construction — il sort de
la fonction avant qu'aucune tâche n'existe — et la tâche spawnée ne contient
qu'un seul `consume`. Les deux défauts du ticket ont une seule et même cause :
l'ouverture vit du mauvais côté du `spawn`. Le remède est de l'y faire passer.

**Le drop de la couture est déjà le signal d'arrêt, il n'est simplement pas lu.**
`CallbackFeed` (`tasks.rs:727-731`) possède le `UnboundedReceiver` ; la tâche
possède le `UnboundedSender`. Quand la TUI lâche la couture, `rx` est droppé et
`tx.is_closed()` devient vrai. Aucun canal d'annulation neuf n'est nécessaire :
le signal existe, il suffit de le consulter.

**`consume` prend `self` par valeur** (`tasks.rs:565`). Un flux est donc
consommé par sa lecture, ce qui rend la boucle du superviseur naturelle : une
ouverture par tour, jamais de réutilisation.

**`tokio` est en `features = ["full"]`** (`Cargo.toml:18`, `mika-cli` par
`tokio.workspace = true`). `sleep` et `timeout` sont disponibles **sans ajouter
ni dépendance ni feature**. `test-util` n'y est pas et n'y entre pas : les délais
des tests voyagent par paramètre.

**Le filet du poll passe toutes les ~5 s.** `POLL_INTERVAL_TICKS = 167`
(`app.rs:669`) à 30 ms par tick (`chat.rs:703`, `EventReader::new`) donne 5,01 s.
C'est le chiffre qui calibre le plafond de recul au § 1.

**Le serveur ne rejoue rien.** `tasks_stream.rs:288` sert
`Sse::new(stream).keep_alive(KeepAlive::default())` sur un `broadcast` : aucune
reprise par curseur, aucun `Last-Event-ID`. Une trame émise pendant une coupure
est perdue pour le fil. C'est la dette du § 6, et ce vol ne la traite pas.

**`commands/tasks.rs` n'est pas dans le périmètre du lint a2a.**
`scripts/check-a2a-timeout-literals.sh:98-104` liste `crates/mika-a2a/src`,
`remote_ask.rs`, `budget_floor.rs`, `a2a_call.rs` et `crates/mika-common/src/llm`.
La forme des constantes de recul n'est donc pas contrainte par la règle A2 — et
ce vol n'ajoute aucun site d'appel a2a qui mériterait de l'y faire entrer.

**Egress Manifest Lint est vert, `docs/egress/` intact.**
`bash scripts/verify-egress-manifest.sh --report` :
`400 fichier(s) .rs de production scanné(s), 31 portant une construction de
client, 22 host(s) distinct(s), 31 entrée(s) [[sink]] confrontée(s)` puis
`No egress-manifest violations found.` Ce vol n'ouvre **aucun client HTTP neuf** :
la reconnexion rappelle `TaskEventStream::open`, déjà déclaré sous
`cli-spirit-tasks`. Le manifeste n'a rien à apprendre. Le pilote le relance avant
d'ouvrir la PR (critère 6) ; s'il le trouve rouge, il s'arrête et le dit.

**Les deux phrases « sans recul ni reconnexion » sont localisées.**
`grep -rn "sans recul ni reconnexion" crates/` donne exactement deux sites de
code : `tasks.rs:734` (doc d'`open_for`) et `chat.rs:564` (commentaire du site
d'appel). La troisième occurrence est dans un plan déjà mergé — un plan décrit
l'état d'un vol passé et ne se réécrit pas.

**La doc de `TaskEventStream` n'a rien à corriger.** `tasks.rs:507-508` dit « the
TUI's subscription (vol V2b-2) backs off on a failed **open** while an ended
stream is only a reconnect ». La phrase était prospective ; ce vol la rend vraie
sans la changer. Zéro ligne, et c'est dit ici pour que le pilote ne la touche pas
par réflexe.

**Un seul test existant devient rouge, et il est nommé ici.**
`mika1727_v2b2b3_un_completed_et_un_failed_de_lagent_entrent_dans_le_canal`
(`tasks.rs:1674`) lit `couture.rx.recv()` jusqu'à `None`, et sa doc explique
pourquoi (« le corps du factice est fini : le drainage lâche son expéditeur »).
Sous reconnexion, l'expéditeur n'est plus lâché : le test boucle sans fin. Son
traitement est au § 4, et c'est **exactement** ce que le critère 5 autorise — la
terminaison change, les assertions non.

Deux mesures disent aussi ce qu'il n'y a **pas** à faire :

- **`tui/app.rs` n'est pas touché.** Ses dix sites de couture passent tous par
  `CallbackFeed::from_channel` (`app.rs:2438`) et par `next_candidate`
  (`app.rs:1639`), que ce vol ne modifie pas. Le drainage ne sait pas, et n'a pas
  à savoir, combien de fois le fil s'est ouvert.
- **`poll_callback_tasks` (`app.rs:1670-1778`) n'est pas modifié**, pas une ligne,
  pas un commentaire. Il reste le filet de ce que le fil manque pendant une
  coupure — et le § 6 dit pourquoi il ne peut pas encore partir.

---

## 1. Le recul : une fonction pure, et ses deux valeurs

Un type à deux champs, et une méthode sans effet de bord :

```rust
/// Le délai entre deux ouvertures du fil. Fonction **pure** de la tentative :
/// elle ne lit ni horloge ni état, donc elle se mesure sans en suspendre une.
struct Recul {
    initial_ms: u64,
    plafond_ms: u64,
}

impl Recul {
    const DEFAUT: Self = Self {
        initial_ms: 1_000,
        plafond_ms: 30_000,
    };

    fn delai(&self, tentative: u32) -> std::time::Duration {
        let ms = 2u64
            .checked_pow(tentative)
            .and_then(|facteur| self.initial_ms.checked_mul(facteur))
            .unwrap_or(self.plafond_ms)
            .min(self.plafond_ms);
        std::time::Duration::from_millis(ms)
    }
}
```

**`checked_pow` puis `checked_mul`, et non un décalage.** `initial_ms << tentative`
paraît équivalent et ne l'est pas : `checked_shl` ne valide que la **largeur** du
décalage, pas la perte de bits, donc `1_000u64 << 60` rend un nombre tronqué
plutôt que `None` — un recul qui redeviendrait court après avoir été long. Les
deux `checked_*` enchaînés font retomber tout débordement sur le plafond, et le
`.min()` final le garantit une seconde fois pour les tentatives qui ne débordent
pas encore. C'est la lecture de « ne panique pas et ne déborde pas pour une
tentative grande » du périmètre.

**1 s en initial.** Le cas nominal de fermeture est un redémarrage de
mika-spirit (`make deploy`), qui prend quelques secondes : un recul initial de
100 ms y produirait une rafale de dizaines de connexions refusées pour rien, et
un recul de 10 s ferait payer dix secondes à une fermeture passagère. Une seconde
est aussi en dessous du filet du poll (~5 s, § 0) : une reconnexion réussie du
premier coup rétablit le fil avant que l'opérateur ne voie une attente.

**30 s en plafond.** C'est la latence maximale de reprise après une coupure
longue — six passages du poll, qui couvre pendant ce temps. Un plafond plus haut
rendrait la reprise invisible à l'échelle d'une session de travail ; plus bas, il
maintiendrait un trafic permanent contre un daemon durablement à terre, ce que
le recul exponentiel existe précisément pour éviter.

**Remis à zéro après une ouverture réussie.** Une connexion qui a tenu une heure
puis s'est fermée n'est pas la cinquième tentative d'un daemon absent : son
prochain délai repart à 1 s.

**Pas de constructeur.** Le module `tests` est un enfant de ce module et voit
les champs privés : un test écrit `Recul { initial_ms: 20, plafond_ms: 20 }`
directement. Un `Recul::en_ms(…)` n'aurait eu que des appelants de test, ce que
clippy signale et ce qu'une ligne de moins règle mieux.

---

## 2. Les gardes et leur domicile

### 2.1 La tâche d'`open_for` devient un superviseur (`commands/tasks.rs`)

`open_for` perd son `async` et son `Option`, et ne fait plus qu'une chose :
construire le canal et lancer le superviseur.

```rust
pub(crate) fn open_for(agent_id: &str, base_url: String, token: String) -> Self {
    Self::open_for_avec_recul(agent_id, base_url, token, Recul::DEFAUT)
}

fn open_for_avec_recul(
    agent_id: &str,
    base_url: String,
    token: String,
    recul: Recul,
) -> Self {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let agent = agent_id.to_string();
    let base = base_url.clone();
    let jeton = token.clone();
    tokio::spawn(async move {
        let mut tentative: u32 = 0;
        loop {
            if tx.is_closed() {
                return;
            }
            if let Ok(flux) = TaskEventStream::open(&base, &jeton).await {
                tentative = 0;
                let _ = flux
                    .consume(Some(&agent), |_, trame| {
                        if let Some(id) = terminal_callback_frame(trame) {
                            let _ = tx.send(id);
                        }
                    })
                    .await;
            }
            tokio::time::sleep(recul.delai(tentative)).await;
            tentative = tentative.saturating_add(1);
        }
    });
    Self { rx, base_url, token }
}
```

Quatre propriétés, chacune exigée par un critère :

- **`open_for` n'est plus `async` et ne rend plus `Option`.** Plus rien ne
  s'ouvre avant le `spawn`, donc il n'y a plus d'échec à rendre : la couture
  existe toujours (critère 3). `async` sans `await` rougirait sous
  `clippy::unused_async`, ce qui est la bonne pression — la fonction ne fait plus
  d'attente.
- **Un échec d'ouverture et une fin de flux mènent au même endroit.** Le `if let
  Ok` sans `else` n'est pas une négligence : les deux chemins tombent sur le
  `sleep` qui suit, et c'est la forme qui rend vraie la phrase du périmètre
  « mènent tous les deux à une nouvelle ouverture après le recul ». `consume` rend
  `Result<()>` et son `Err` (lecture coupée) est traité comme son `Ok`
  (fermeture propre) : l'un comme l'autre est une reconnexion.
- **Le recul est remis à zéro sur l'ouverture, pas sur la livraison.** Un flux
  ouvert qui ne livre rien est un fil sain — le keep-alive du serveur le dit — et
  faire dépendre la remise à zéro d'une trame reçue ferait grandir le recul d'un
  agent calme.
- **`saturating_add`**, parce qu'une TUI laissée ouverte un week-end contre un
  daemon mort ne doit pas déborder son compteur. Au-delà de 5 tentatives le délai
  est déjà au plafond ; la saturation n'est là que pour qu'aucune valeur
  n'existe où elle panique.

La doc d'`open_for` est réécrite dans le même commit : la phrase « Ouvrir **une
fois** le fil, sans recul ni reconnexion » devient fausse à la ligne où ce code
atterrit. Elle dira ce que le superviseur garantit, ce qu'il ne garantit pas (le
rattrapage, § 6), et pourquoi le drop du récepteur suffit à l'arrêter.

### 2.2 Le superviseur s'arrête quand la couture est lâchée (`commands/tasks.rs`)

Les deux lignes `if tx.is_closed() { return; }` **avant** l'ouverture, et leur
commentaire. Trois choses à savoir, qui sont la raison du commentaire :

- **Avant l'ouverture, et non pendant le `sleep`.** Un `select!` sur la
  fermeture du canal sortirait quelques centaines de millisecondes plus tôt et
  coûterait un chemin de plus à raisonner ; la propriété que le critère 4 mesure
  est « plus **aucune connexion** après le lâcher », et elle est portée
  entièrement par le test en tête de boucle.
- **La tâche survit au drop jusqu'à son prochain réveil**, au plus un recul
  plafond. Elle ne tient alors qu'un `Sender` fermé, ne lit aucune socket et ne
  rouvre rien ; ce n'est pas une fuite, c'est un réveil tardif, et c'est la seule
  lecture honnête du code.
- **Aucun second canal.** Le `Sender` que le superviseur possède déjà *est* le
  témoin de vie du récepteur (§ 0). Un `CancellationToken` ajouterait une
  seconde vérité sur le même fait.

### 2.3 Le site d'appel suit la signature (`commands/chat.rs`)

```rust
if let Ok(token) = crate::commands::dashboard::auth_token() {
    app.callback_feed = Some(CallbackFeed::open_for(
        agent_name,
        crate::commands::dashboard::spirit_url(),
        token,
    ));
}
```

Le `if let Ok(token)` ne bouge pas : **sans jeton, aucune couture n'est créée**,
et c'est le cas que le périmètre laisse à `chat.rs`. L'`Option` de
`app.callback_feed` garde donc un sens — elle dit « ce shell n'a pas de jeton »,
non plus « le daemon était à terre au lancement ». Le commentaire de
`chat.rs:563-566` est réécrit dans le même commit : il dira la reconnexion, le
recul, et que `run_team` ne passe pas par ce chemin.

### 2.4 Ce qui ne change pas

Le canal (un `task_id`, pas de statut), le prédicat
`est_un_callback_a_livrer`, `next_candidate`, `terminal_callback_frame`,
`frame_agent_id`, `TaskDetailMirror`, `fetch_task_detail`, `from_channel`,
`fake_spirit::spawn`, `fake_spirit::spawn_detail` et tout `tui/app.rs`. **La
reconnexion ne change rien à ce qui est livré, seulement au moment où le fil est
ouvert** — c'est la phrase du périmètre, et la liste ci-dessus est la forme
qu'elle prend dans le diff.

---

## 3. Ordre des commits

Le pilote commite après chaque garde verte **avec son test**, pas en fin de vol.
Deux commits.

**Commit 1 — le fil se rouvre** (`commands/tasks.rs`, `commands/chat.rs`)
§ 1, § 2.1, § 2.3, avec T1, T2, T3, le factice à séquence et la terminaison de T5
(§ 4). À la fin de ce commit, les critères 1, 2, 3 et 5 tiennent.

**Pourquoi le § 1 ne part pas seul**, bien qu'il soit la première garde du
ticket : `Recul::delai` n'aurait alors que des appelants de test, et
`cargo clippy --all-targets -- -D warnings` (CI, `ci.yml:76`) compile aussi la
cible non-test, où la méthode est morte. La garde pure et son unique appelant de
production sont indissociables à la granularité du commit ; ils restent deux
livrables distincts, avec deux tests distincts.

**Commit 2 — le superviseur s'arrête** (`commands/tasks.rs`)
§ 2.2, avec T4. L'état laissé par le commit 1 est compilable et vert mais
**incomplet par construction** : une TUI fermée y laisse une tâche qui rouvre le
fil indéfiniment. Les deux commits partent donc dans la même PR et aucun n'est
mergé seul.

---

## 4. Le jeu de tests

Cinq tests, dont un est la modification d'un test existant. Un seul constructeur
de serveur factice neuf, comme le périmètre l'autorise.

### Le factice à séquence (`fake_spirit`)

```rust
pub(crate) async fn spawn_sequence(
    reponses: Vec<(u16, String)>,
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>)
```

Sert `/api/v1/dashboard/tasks/stream` en lisant la séquence **par index de
requête** : la n-ième connexion reçoit `reponses[n]`, et au-delà de la séquence
un `200` à corps vide — qui se ferme aussitôt et provoque donc une reconnexion
de plus, ce dont T4 a besoin pour mesurer un compteur qui monte avant de
s'arrêter. Le compteur rendu est le nombre de requêtes **reçues sur cette
route**, incrémenté avant de servir ; c'est la définition opérationnelle de
« connexion » dans les quatre tests, et elle est écrite dans sa doc plutôt que
supposée.

Il vit dans le bloc `#[cfg(test)] pub(crate) mod fake_spirit` **en fin de
fichier**, à côté de `spawn` et `spawn_detail` : la doc de ce module
(`tasks.rs:2293-2301`) explique que la coupure du scanner egress dépend de cette
position, et un factice placé plus haut rendrait invisibles au lint toutes les
lignes qui suivent.

### Commit 1

**T1 — `mika1727_v2b2b4_le_recul_double_jusquau_plafond_et_ne_panique_pas`.**
Test synchrone, sans runtime et sans serveur : `Recul { initial_ms: 1_000,
plafond_ms: 30_000 }` rend 1 s, 2 s, 4 s, 8 s, 16 s pour les tentatives 0 à 4,
puis 30 s à la tentative 5 (32 s plafonnées) et 30 s pour 64, 1 000 et
`u32::MAX` — les trois valeurs qui font déborder `checked_pow`, `checked_mul` et
rien du tout respectivement. C'est le critère 1 en entier, et c'est le seul test
du vol qui ne dépend d'aucune horloge.

**T2 — `mika1727_v2b2b4_apres_une_fermeture_le_fil_se_rouvre`.** Séquence de
deux réponses : la première sert un `task_claimed` de `mika-dev` — une trame que
le filtre laisse passer mais que `terminal_callback_frame` ne retient pas, donc
**rien n'entre dans le canal par cette connexion** — puis se ferme ; la seconde
sert un `task_completed` de `mika-dev` sur `t-7`. Recul de test
`{ initial_ms: 20, plafond_ms: 20 }`. Deux assertions : `t-7` arrive dans le
canal sous `tokio::time::timeout`, et le compteur du factice vaut au moins 2. La
première dit que la trame est arrivée, la seconde qu'elle n'a pu arriver que par
la **seconde** connexion (critère 2).

**T3 — `mika1727_v2b2b4_un_echec_douverture_rend_quand_meme_la_couture`.**
Séquence `[(503, "daemon à terre"), (200, …t-8…)]`. Trois assertions : la couture
est rendue — c'est un fait de type, `open_for_avec_recul` ne rend pas d'`Option`,
et le test l'exerce en lisant son canal —, `t-8` arrive, et le compteur vaut au
moins 2. C'est le critère 3, et c'est le cas « daemon à terre au lancement de la
TUI » du ticket.

**T5 — la terminaison de
`mika1727_v2b2b3_un_completed_et_un_failed_de_lagent_entrent_dans_le_canal`.**
Les six charges SSE littérales ne changent pas, et l'assertion non :
`assert_eq!(recus, vec!["t-2", "t-6"])`. Seule la lecture change — deux `recv()`
sous `timeout`, puis la vérification que **rien ne suit** pendant 200 ms :

```rust
let silence = tokio::time::timeout(
    std::time::Duration::from_millis(200),
    couture.rx.recv(),
)
.await;
assert!(silence.is_err(), "rien ne suit les deux identifiants");
```

Le test continue d'appeler `open_for`, donc le **recul par défaut** (1 s) : la
fenêtre de silence de 200 ms est à 5× sous le premier recul, ce qui rend
structurellement impossible qu'une seconde connexion rejoue `t-2` dedans. C'est
pour ça que cette fenêtre ne dépend pas de la vitesse de la machine : elle est
bornée par une constante du code, pas par un temps de réponse. La doc du test est
corrigée au passage — elle affirme aujourd'hui que « le drainage lâche son
expéditeur et `recv` rend `None` », ce que la reconnexion rend faux.

Les autres tests de V2b-2b-2 et V2b-2b-3 restent verts **sans être touchés** :
ils passent par `from_channel` (`app.rs`) ou par `spawn_detail`, ni l'un ni
l'autre modifié (§ 2.4).

### Commit 2

**T4 — `mika1727_v2b2b4_une_couture_lachee_ne_rouvre_plus_le_fil`.** Séquence
vide : chaque connexion reçoit un `200` à corps vide et se ferme, donc le
superviseur reboucle à chaque recul. Recul de test
`{ initial_ms: 20, plafond_ms: 20 }`. Quatre temps :

1. laisser tourner 150 ms, puis `assert!(connexions >= 2)` — le **témoin
   positif** que mika#2205 exige à côté d'un refus : sans lui, un superviseur qui
   n'ouvrirait jamais rien rendrait ce test vert ;
2. `drop(couture)` ;
3. attendre 50 ms, le temps qu'une connexion déjà en vol atterrisse, puis
   **relever** le compteur — la propriété mesurée est « cesse de monter », pas
   « n'a jamais monté », et une ouverture lancée avant le drop est permise ;
4. attendre 200 ms — dix reculs de test — et `assert_eq!` sur le compteur relevé.

C'est le critère 4, « un intervalle qui couvre plusieurs reculs » inclus.

---

## 5. Ce que ce vol ne fait pas

- **Il ne retire pas le poll.** C'est V2b-3, et le § 6 dit à quelle condition.
- **Il ne rattrape rien.** Voir § 6.
- **Il ne touche ni `crates/mika-agent/` ni `crates/mika-a2a/`**, ni aucun
  fichier du plan de contrôle (`.github/workflows/`, `.claude/`,
  `skills/bundled/`, `well_known_agents.rs`, `.mika/`, CODEOWNERS), ni
  `docs/egress/`.
- **Il n'ajoute ni dépendance ni feature**, `tokio/test-util` compris (§ 0).
- **Il ne modifie pas `tui/app.rs`.**
- **Il n'ouvre aucun ticket**, pas même de suivi (gel jusqu'au 01/12). La suite
  se note `Tracked in: senara-solutions/mika#1727`.

---

## 6. La dette, nommée et non traitée

**Le fil ne rattrape pas ce qu'il a manqué.** Le serveur diffuse par un
`broadcast` sans reprise par curseur (§ 0) : les trames émises pendant une
coupure, et celles qu'un `OverflowMarker` déclare perdues, ne sont rejouées par
aucune reconnexion. Ce vol réduit la durée des trous — ils se refermaient
jamais, ils se refermeront en au plus 30 s — il ne les comble pas.

Ce qui couvre aujourd'hui est `poll_callback_tasks`, qui balaye la base toutes
les ~5 s et ne dépend d'aucun fil. **V2b-3 ne pourra donc retirer le poll
qu'avec un rattrapage**, dont la forme reste à décider : un balayage des tâches
non livrées à chaque reconnexion réussie (côté TUI, sans toucher au serveur), ou
une reprise par curseur côté `tasks_stream.rs` (côté serveur, hors périmètre de
tout vol V2b). La PR le dit et ne le fait pas.
`Tracked in: senara-solutions/mika#1727`.

---

## Fire-Disposition

Ce plan livre des détecteurs : T4 a pour chemin de succès « aucune connexion
nouvelle trouvée », et T5 « rien ne suit les deux identifiants ». La disposition
est donc requise (mika#2306).

**Option retenue : (a) atterrissage armé, allowlist expédiée vide et épinglée
vide.**

Les deux détecteurs mesurent une surface **neuve de ce vol** : un superviseur qui
n'existe pas sur `0d34872e2`. Il n'y a aucune violation préexistante à amnistier,
et l'allowlist part vide — elle n'est pas un inventaire de dérogations mais la
place nommée où une exception future devrait se voir au `grep`, avec son ticket
et sa condition d'extinction.

Si l'un des deux rougit à l'atterrissage, la conduite est celle de mika#2201 :
corriger le superviseur, **pas** élargir la fenêtre de mesure. Allonger le
silence de T5 ou tolérer un delta sur le compteur de T4 est exactement le geste
que mika#2309 condamne — une allowlist pleine de cas « presque corrects ».

Les options (b) et (c) sont écartées : (b) — expédier désarmé par `#[ignore]` —
laisserait sans preuve la seule garantie qui empêche une TUI fermée de marteler
le daemon, au moment même où ce vol crée la boucle qui pourrait le faire ; (c) —
s'arrêter et escalader — n'a pas lieu d'être, puisque les deux questions qui
pouvaient bloquer ce vol (le drop du récepteur suffit-il comme signal d'arrêt ?
`tokio::time` est-il disponible sans feature neuve ?) sont mesurées et tranchées
au § 0.

---

## Definition of Done

- Les deux commits du § 3 sont sur la branche, chacun vert avec ses tests.
- `cargo test -p mika-cli` est vert ; les tests de V2b-2b-2 et V2b-2b-3 le sont
  sans avoir été affaiblis — seule la terminaison de T5 a changé, ses assertions
  sont inchangées.
- `cargo clippy --all-targets --all-features -- -D warnings` et
  `cargo fmt --check` sont verts.
- `bash scripts/verify-egress-manifest.sh --report` est vert, `docs/egress/`
  intact.
- `bash scripts/verify-pipeline.sh origin/main` est vert.
- `git diff --numstat origin/main...HEAD` donne au plus 279 lignes ajoutées, plan
  exclu.
- `poll_callback_tasks` et `crates/mika-cli/src/tui/app.rs` sont identiques à
  leur forme sur `0d34872e2`.
- Les deux phrases « sans recul ni reconnexion » ont disparu du code.
- La PR porte `Refs mika#1727`, et ni `Closes`, ni `Fixes`, ni `Resolves`, nulle
  part — ni dans le corps, ni dans un message de commit.
- La PR nomme la dette du rattrapage (§ 6) et porte
  `Tracked in: senara-solutions/mika#1727`.
- `Code review: delegated to mika-qa (mika#2694)` figure dans le corps de la PR.

---

## Acceptance criteria

1. Le délai de recul part de sa valeur initiale, double à chaque tentative et ne
   dépasse jamais le plafond. Un test le montre, y compris pour une tentative
   très grande, sans panique.
2. Après une **fermeture** du flux, le fil se rouvre, et une trame
   `TaskCompleted` de l'agent servie par la **seconde** connexion arrive dans le
   canal. Un test le montre avec le serveur factice à séquence.
3. Si la **première ouverture échoue** (par exemple en 503), la couture est quand
   même rendue, le fil se rouvre après le recul, et une trame de l'agent arrive.
   Un test le montre.
4. Une fois la couture **lâchée**, le compteur de connexions du factice cesse de
   monter. Un test le mesure sur un intervalle qui couvre plusieurs reculs.
5. Le test du critère 4 de V2b-2b-3 garde **ses assertions** : `t-2` puis `t-6`,
   rien d'autre. Seule sa façon de terminer change : il lit deux identifiants,
   puis vérifie que rien ne suit pendant un court délai. Les autres tests de
   V2b-2b-2 et V2b-2b-3 restent verts sans être affaiblis.
6. `poll_callback_tasks` n'est pas modifié. Egress Manifest Lint est vert **avec
   `docs/egress/` intact** : le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR, et
   s'il le trouve rouge, il s'arrête et le dit. La PR porte `Refs mika#1727` et
   aucun `Closes`, `Fixes` ni `Resolves`. Elle nomme la dette du rattrapage.

---

## Taille estimée

Lignes de code ajoutées, commentaires et tests compris, `docs/` et `CLAUDE.md`
exclus, dans l'unité de `git diff --numstat origin/main...HEAD`.

### Commit 1 — `commands/tasks.rs` et `commands/chat.rs`

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 1 `Recul` : deux champs, `DEFAUT`, `delai`, doc des deux valeurs et du `checked_pow` | 26 |
| § 2.1 superviseur : boucle, remise à zéro, `saturating_add`, `open_for_avec_recul` | 24 |
| § 2.1 doc d'`open_for` réécrite : ce qui est garanti, ce qui ne l'est pas | 14 |
| § 2.3 `commands/chat.rs` : site d'appel et commentaire réécrit | 9 |
| § 4 `fake_spirit::spawn_sequence` + doc de la définition de « connexion » | 38 |
| T1 recul pur, six tentatives dont trois de débordement | 22 |
| T2 reconnexion après fermeture | 26 |
| T3 échec de première ouverture | 23 |
| T5 terminaison par deux `recv` + fenêtre de silence, doc corrigée | 8 |

Sous-total : 190

### Commit 2 — `commands/tasks.rs`

| livrable | lignes de code (hors `docs/`) |
|---|---|
| § 2.2 garde `is_closed` + commentaire (avant l'ouverture, réveil tardif, pas de second canal) | 8 |
| T4 le compteur cesse de monter, témoin positif inclus | 32 |

Sous-total : 40

Total estimé : 230 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

La marge est de 49 lignes, soit 18 % de la borne. Elle n'est pas de la réserve
pour élargir : elle absorbe le biais d'estimation sur les trois tests de
reconnexion, dont la mise en place passe par le factice à séquence plutôt que par
trois serveurs recopiés. Le vol tient en **un seul** vol parce que son périmètre
est un seul fichier de production et une signature de fonction : rien n'y est
découpable en deux PR mergeables séparément — un superviseur sans recul n'a pas
de sens, et un recul sans superviseur est du code mort (§ 3). Si le total réel
dépassait 279, le vol s'arrête et le dit — il ne recoupe pas de lui-même, et il
ne reporte aucun test.

`Tracked in: senara-solutions/mika#1727`
