# mika#1727 — vol V3a-1 : le fil sait porter un tour de callback

> Vol serveur. La clé de métadonnées `mika.callback_turn` est **déclarée** dans
> `mika_a2a::params` et **lue** par la porte A2A. Rien ne la pose encore : le bras
> `CallbackResult` de la TUI part sur le fil au vol V3a-2.
>
> `Refs mika#1727` — jamais `Closes`. Le ticket se ferme au merge de V4.
>
> Tracked in: senara-solutions/mika#1727

---

## 0. Pourquoi ce vol tient en un seul vol

Le vol pose **une** constante, **un** lecteur, et remplace **un** littéral. Le
modèle est entièrement écrit sur `main` : `mika.session_isolated` (mika#1951) est
la même forme de clé booléenne, avec son lecteur strict, son `shape_of` et sa
batterie de tests — à six fonctions de distance dans le même fichier. Le vol
n'invente aucune forme ; il en instancie une.

Le gros poste du vol n'est pas le code de production (≈ 126 lignes), c'est le
**fixture d'état** qu'exige le critère 3 : lire le fait *du côté de la boucle*
demande un tour réel, donc un `AppState`, et le constructeur de test du crate vit
dans un module privé que le périmètre de ce vol interdit de toucher (§ D5). Cela
reste sous les deux bornes, avec ≈ 68 lignes de marge sur la plus serrée.

---

## 1. Ce qui est mesuré sur `main` (@ `f9855e96`)

| fait | site |
|---|---|
| la porte A2A écrit `is_callback_turn: false` en dur, à **une** occurrence | `crates/mika-agent/src/server/a2a.rs:343` |
| cette occurrence est dans l'**unique** `AgentParams` du crate serveur A2A | `a2a.rs:312` (construction), `a2a.rs:249` (`run_a2a_agent`) |
| `run_a2a_agent` a exactement deux appelants | `a2a.rs:1337` (`message/send`), `a2a.rs:1830` (`run_a2a_stream_turn`, atteint par `message/stream`) |
| les deux lisent déjà `mika.session_isolated` par un lecteur unique et le passent | `a2a.rs:1214`, `a2a.rs:1535`, champ `StreamTurn.session_isolated` `a2a.rs:1778` |
| le lecteur strict modèle, avec son `shape_of` sans écho de valeur | `a2a.rs:511-540` et `a2a.rs:542-552` |
| `SESSION_ISOLATED_KEY`, sa doc et son épinglage | `crates/mika-a2a/src/params.rs:126`, tests `params.rs:404-410` et `params.rs:432-452` |
| la famille `mika.*` compte 8 clés, et un test interdit deux orthographes égales | `params.rs:432` `every_key_of_the_family_is_distinct` |
| `run_a2a_agent` porte déjà `#[allow(clippy::too_many_arguments)]` (mika#2304) | `a2a.rs:248` |
| `is_callback_turn` ne se lit nulle part ailleurs dans la production de `server/` | `handlers.rs:2179`, `investigate.rs:778` sont d'autres `AgentParams`, hors périmètre |
| `"mika.callback_turn"` n'existe nulle part dans le dépôt | `grep -rn "callback_turn" crates/mika-a2a/` → vide |
| le `ProductionScanner` qui masque les régions de test existe et est utilisé dans ce fichier | `mika_common::source_guard::ProductionScanner::for_crate`, employé par `a2a.rs:2727` |

Ce que `is_callback_turn: true` **fait** dans la boucle (`agent_loop/mod.rs`, inchangé par ce vol) :

1. arme la garde #716 sur les affirmations d'état non vérifiées (l. 2613) ;
2. compte le tour comme déclenchement automatique pour la frontière `send_message` (l. 5497) ;
3. **saute l'enregistrement du message utilisateur** (l. 6083) — l'appelant a déjà enregistré le résultat brut en `tool_result` ;
4. saute le contexte de dispatch webhook (l. 6869).

L'effet 3 est le fait observable du critère 3 : il est en base, il est binaire, et
il ne demande ni fournisseur réel ni réseau.

---

## 2. Décisions

### D1 — `CALLBACK_TURN_KEY`, cinquième clé de requête, et un booléen par contrainte

`crates/mika-a2a/src/params.rs` :

```rust
pub const CALLBACK_TURN_KEY: &str = "mika.callback_turn";
```

Sa doc dit trois choses, dans cet ordre :

- **Ce que la clé arme et lève**, nommément les quatre effets du § 1. Un lecteur
  qui ne voit que le nom lit « c'est un drapeau de traçage » ; ce n'en est pas un :
  il change ce que la boucle écrit en base et quelle garde tourne.
- **Pourquoi un booléen, et pas le payload du callback.** `/a2a/{agent}` est
  joignable par tout appelant authentifié. Une clé qui porterait l'identifiant de
  tâche, le texte du résultat ou un nom de déclencheur laisserait un appelant
  *fabriquer* un cadrage de callback que rien n'a produit ; un booléen laisse le
  cadrage là où il est déjà bâti — chez l'appelant, par
  `build_callback_trigger_context`, et dans le texte du message. La clé dit
  « ce tour EST un tour de callback », rien de plus. Même raisonnement que la
  contrainte « un bool, jamais un nom de scope » de `SESSION_ISOLATED_KEY`.
- **Pourquoi la lecture est souple à l'absence et fermée au malformé.** Absente,
  `null` ou `false` ⇒ le tour d'avant la clé, bit pour bit, donc un appelant plus
  ancien produit le même tour. Mais un `"true"` de chaîne lu comme `false` ferait
  **enregistrer une seconde fois** le résultat brut du callback — déjà en
  `tool_result` chez l'appelant — et **désarmerait la garde #716** pour ce tour.
  Les deux défauts sont silencieux et produisent une réponse plausible : c'est la
  forme exacte de mika#1951, où un no-op muet *était* le défaut. Donc `Err`, jamais
  `false`.

La chaîne est épinglée par un test à elle, et la constante entre dans la liste de
`every_key_of_the_family_is_distinct` — c'est cette liste, et non le test par clé,
qui attrape un copier-coller ayant collapsé deux orthographes.

### D2 — un lecteur strict, de la forme exacte de son modèle

`crates/mika-agent/src/server/a2a.rs` :

```rust
fn requested_callback_turn(params: &MessageSendParams) -> Result<bool, JsonRpcError>
```

Même squelette que `requested_session_isolation` : `None` ⇒ `Ok(false)` ; `Null` ⇒
`Ok(false)` ; `Bool(b)` ⇒ `Ok(*b)` ; tout le reste ⇒
`Err(JsonRpcError::with_message(INVALID_PARAMS, …))`, message qui **nomme la clé**
et **nomme la forme** par le `shape_of` existant, et **n'écho jamais la valeur** —
elle vient d'un appelant et le message atterrit dans un journal.

`shape_of` est **réutilisé**, pas redéclaré : c'est déjà une fonction du module,
écrite pour ce besoin.

### D3 — `run_a2a_agent` prend le booléen, et aucun `allow` n'est ajouté

Le paramètre `callback_turn: bool` se range **après** `session_isolated`, dans
l'ordre où les clés sont nées, et alimente `AgentParams.is_callback_turn` à la
place du littéral `false` de la l. 343. Un commentaire d'une phrase y dit ce que
le champ commande dans la boucle, en renvoyant aux quatre effets.

Deux choses que ce vol **ne** fait **pas**, et le plan le dit pour qu'un relecteur
n'ait pas à se demander si c'est un oubli :

- il ne regroupe pas `session_isolated` et `callback_turn` dans une struct de
  requête. Ce serait la bonne forme à trois ou quatre drapeaux ; à deux, elle
  coûte plus de lignes qu'elle n'en épargne et touche les deux appelants une fois
  de plus. Le vol reste à la forme du modèle.
- il n'ajoute **aucun** `#[allow]`. Le `clippy::too_many_arguments` présent en
  l. 248 vient de mika#2304 et couvre déjà le dixième paramètre ; le critère 7 dit
  « sans `allow` », ce qui se lit « sans *nouvel* `allow` » — l'existant n'est pas
  un acquis de ce vol.

### D4 — les deux portes lisent par ce lecteur, et refusent avant tout tour

Dans `handle_message_send` et dans `handle_message_stream`, le bloc de lecture se
place **immédiatement après** celui de `session_isolated`, à l'identique :

```rust
let callback_turn = match requested_callback_turn(&params) {
    Ok(v) => v,
    Err(err) => {
        warn!(event = "a2a_callback_turn_refused", agent = …, task_id = …, port = "send", …);
        return Json(JsonRpcResponse::error(request.id.clone(), err)).into_response();
    }
};
```

Le placement n'est pas cosmétique : il est **avant le verrou d'agent, avant la
ligne de tâche, et avant l'ouverture du flux SSE**. Sur `message/stream` c'est
l'unique fenêtre où un appelant peut encore être informé que sa requête était
malformée — en plein SSE il n'y a plus de phrase à lui dire. C'est la raison que la
doc de `StreamTurn.session_isolated` donne déjà pour son voisin.

`StreamTurn` gagne donc un champ `callback_turn: bool`, documenté de la même
raison : `params` ne survit pas au `tokio::spawn`, et le refus ne peut pas être
différé dans la tâche.

### D5 — le critère 3 demande un tour réel, et le fixture d'état est **local**, faute d'un chemin dans le périmètre

Le critère 3 interdit d'asserter sur une variable intermédiaire : il veut le fait
*du côté de la boucle*. Le fait retenu est l'effet 3 du § 1 — **le message
utilisateur n'est pas enregistré** — lu en base après un tour réel, avec un
`MockLlmProvider`.

Un tour réel passe par `run_a2a_agent`, qui demande un `AppState` et un
`Arc<AgentState>`. Le constructeur de test du crate (`test_state_full`, puis
`test_state`, `mika1727_state_compact`, `mika2855_state_repondant`) vit dans
`#[cfg(test)] mod tests` de `crates/mika-agent/src/server/mod.rs`. Ce module est
privé : ses fonctions ne sont **pas** visibles depuis `server::a2a::tests`, qui en
est un cousin et non un descendant. Les rendre `pub(super)` coûterait **une
ligne**, dans un bloc `#[cfg(test)]`, sans un octet de production — et ferait
changer un cinquième fichier, ce que le critère 7 interdit nommément.

**Disposition retenue : le fixture est recopié dans `a2a.rs::tests`**, soit
≈ 110 lignes de littéraux de struct. C'est une répétition de **forme**, pas de
connaissance : un champ ajouté à `AppState` ou `AgentState` casse la compilation du
fixture recopié. La dérive est tenue par le compilateur, pas par la vigilance — ce
qui est la seule raison pour laquelle cette recopie est acceptable ici. Le fixture
porte un commentaire qui dit **pourquoi** il est local, pour qu'il ne se lise pas
comme une ignorance du constructeur voisin.

**Tension de périmètre, déclarée plutôt que tranchée.** Le dépôt a déjà rencontré
ce besoin et a tranché dans l'autre sens : mika#2070 — la clé
`mika.caller_session_id`, exactement la même famille — a placé son test
bout-en-bout dans `server/mod.rs::tests`, en POSTant un vrai corps JSON-RPC à
travers `build_router`, et son commentaire dit textuellement que les tests
unitaires de part et d'autre « passent tous les deux » avec les deux sites
d'appel inertes. Si Vincent préfère ce précédent, le vol troque ≈ 110 lignes de
fixture contre **une** ligne de `pub(super)` dans un bloc `#[cfg(test)]` de
`server/mod.rs`, et le test s'écrit par le routeur. Le groom ne prend pas cette
décision : elle élargit le périmètre que le corps du ticket a fermé, et le corps
dit de le dire plutôt que d'élargir. **Le plan livre sans elle.**

### D6 — le texte du mock ne porte aucune affirmation d'état, et le contrôle positif est obligatoire

`is_callback_turn: true` **arme la garde #716**. Un mock qui répondrait « j'ai
posté le commentaire » ferait relancer la boucle par la garde, et le test
mesurerait un tour qui n'est pas celui qu'il croit. Le mock rend donc un texte
anodin, sans aucune affirmation sur un système en aval, et en fournit assez
d'exemplaires pour que la boucle n'épuise pas la séquence (la forme
`(0..8).map(…)` de `mika2855_state_repondant`).

Le test vient **par paire** :

- avec la clé à `true`, le message utilisateur est **absent** de la base ;
- avec la clé à `false`, il est **présent**.

Sans le second, le premier passerait aussi avec un tour qui n'enregistre jamais
rien — par exemple parce que la session n'existe pas, ou parce que le tour a
échoué avant. C'est le contrôle de non-vacuité, et il est la moitié qui compte.

### D7 — le critère 6 est une garde lexicale, et son aiguille est assemblée

Le critère 6 veut que `is_callback_turn: false` ait disparu de la production de
`a2a.rs`. La garde lit la production du fichier par
`ProductionScanner::production_of`, qui masque les régions de test — sans ce
masquage, le contrôle positif de D6 (qui passe `false`) ferait rougir la garde.

L'aiguille est assemblée par `concat!`, comme celle de
`mika2304_the_attestation_is_taken_on_the_serving_provider` : sinon le fichier
contiendrait la chaîne qu'il interdit, et la garde se trouverait elle-même.

### D8 — le critère 5 se prouve par l'unicité du lecteur et le compte de ses appels

Une garde lexicale sur la production de `a2a.rs` asserte **un** `fn
requested_callback_turn` et **deux** appels. C'est la forme que le corps du ticket
autorise explicitement pour ce critère, et c'est la bonne : un second lecteur
copié-collé ne ferait échouer aucune assertion de comportement, puisqu'il
donnerait d'abord les mêmes réponses — il divergerait à la première évolution, en
silence. C'est la raison même pour laquelle
`mika1951_the_isolation_flag_has_exactly_two_readers_in_the_loop` existe.

### D9 — la doc dit la clé, ce que la porte en fait, et que personne ne la pose encore

- `crates/mika-a2a/CLAUDE.md` : la clé rejoint la liste des clés de métadonnées
  `mika.*`, avec sa sémantique booléenne et son contrat de lecture.
- `crates/mika-agent/CLAUDE.md` : ce que la porte A2A en fait — lecture stricte sur
  les deux portes, refus avant tout tour, passage à `AgentParams.is_callback_turn`.

Les deux disent la même phrase, et c'est la phrase qui compte : **rien ne pose
cette clé aujourd'hui** ; V3a-2 la posera depuis la TUI. Une doc qui décrirait la
clé sans le dire laisserait croire à un chemin vivant, et le prochain lecteur
chercherait l'appelant pendant un quart d'heure.

### D10 — aucun ticket n'est ouvert

Gel des tickets jusqu'au 01/12. Le vol n'ouvre ni suivi ni sous-ticket ; le reste
se note `Tracked in: senara-solutions/mika#1727` dans la PR. La tension de D5 est
dite **ici** et dans le corps de la PR, pas dans un ticket.

---

## 3. Livrables

| livrable | ce qu'il contient |
|---|---|
| `crates/mika-a2a/src/params.rs` | `CALLBACK_TURN_KEY` + sa doc (D1) ; son test d'épinglage ; son entrée dans la liste d'unicité |
| `crates/mika-agent/src/server/a2a.rs` (production) | `requested_callback_turn` (D2) ; dixième paramètre de `run_a2a_agent` et passage à `AgentParams` (D3) ; les deux blocs de lecture et le champ `StreamTurn.callback_turn` (D4) |
| `crates/mika-agent/src/server/a2a.rs` (tests) | batterie du lecteur (critères 1, 2, 4) ; fixture d'état local + paire de tours réels (critère 3, D5/D6) ; gardes lexicales (critères 5 et 6, D7/D8) |
| `crates/mika-a2a/CLAUDE.md` | la clé parmi les clés de métadonnées (D9) |
| `crates/mika-agent/CLAUDE.md` | ce que la porte A2A fait de la clé (D9) |

Rien d'autre ne change. En particulier : pas `crates/mika-cli/`, pas
`crates/mika-agent/src/agent_loop/` (le sens d'`is_callback_turn` est inchangé),
pas `crates/mika-gateway/`, pas `docs/openapi/`, pas `docs/egress/`, pas
`docs/slash-commands.md`, et aucun fichier du plan de contrôle.

---

## 4. Ordre des commits

Chaque garde arrive **avec ses tests, dans le même commit**. Après chaque commit,
le pilote mesure `git diff --numstat origin/main...HEAD`, plan exclu ; **au-delà de
450 lignes ajoutées il s'arrête** — pas de commit suivant, pas de PR — et son
dernier message dit « HALTE borne : N lignes pour 450 », avec le compte par commit.

1. **`feat(1727): la clé `mika.callback_turn` est déclarée`** — `params.rs` : la
   constante, sa doc, son épinglage, son entrée dans la liste d'unicité.
   *Vert :* `cargo test -p mika-a2a`.
2. **`feat(1727): la porte A2A lit `mika.callback_turn``** — `a2a.rs` :
   `requested_callback_turn` + la batterie du lecteur (critères 1, 2, 4).
   Le lecteur n'est appelé par personne à ce commit ; `cargo clippy` le dirait,
   donc ce commit et le suivant peuvent fusionner si le pilote préfère — et dans ce
   cas le message le dit.
3. **`feat(1727): le tour A2A sait qu'il est un tour de callback`** — le dixième
   paramètre, le passage à `AgentParams`, les deux portes, `StreamTurn` ; plus les
   deux gardes lexicales (critères 5 et 6).
4. **`test(1727): un tour de callback n'enregistre pas le message utilisateur`** —
   le fixture d'état local et la paire de tours réels (critère 3).
   C'est le commit le plus lourd ; le mettre en dernier des commits de code rend la
   halte la plus fine possible sur le poste le plus incertain.
5. **`docs(1727): la clé `mika.callback_turn` dans les deux CLAUDE.md`** —
   la doc (D9).

Avant d'ouvrir la PR : `cargo clippy --all-targets -p mika-a2a -p mika-agent`
vert sans nouvel `allow`, `cargo test -p mika-a2a -p mika-agent`,
`bash scripts/verify-egress-manifest.sh --report` — **rouge ⇒ halte et dire**, et
`bash scripts/verify-pipeline.sh origin/main`.

---

## 5. Jeu de tests

| test | critère | forme |
|---|---|---|
| `callback_turn_key_is_the_wire_spelling` (`params.rs`) | 1 | `assert_eq!(CALLBACK_TURN_KEY, "mika.callback_turn")` |
| `every_key_of_the_family_is_distinct` (étendu) | 1 | la neuvième clé entre dans la liste |
| `mika1727_une_demande_absente_nest_pas_un_tour_de_callback` | 2 | métadonnées absentes, map vide, `null`, `false` ⇒ `false` ; `true` ⇒ `true` |
| `mika1727_une_demande_malformee_refuse_le_tour` | 4 | `"true"`, `"yes"`, `1`, `0`, `[true]`, `{…}`, `""` ⇒ `INVALID_PARAMS`, message nommant la clé |
| `mika1727_le_refus_nomme_la_forme_sans_echo_de_la_valeur` | 4 | une chaîne sentinelle ⇒ le message contient `a string` et **pas** la sentinelle |
| `mika1727_un_tour_de_callback_nenregistre_pas_le_message_utilisateur` | 3 | tour réel, clé `true`, base sans le message utilisateur |
| `mika1727_un_tour_ordinaire_enregistre_le_message_utilisateur` | 3 | **contrôle positif** : même tour, clé `false`, message présent |
| `mika1727_la_cle_na_quun_lecteur_et_deux_appelants` | 5 | garde lexicale sur la production de `a2a.rs` |
| `mika1727_la_porte_a2a_necrit_plus_le_littéral_en_dur` | 6 | garde lexicale, aiguille assemblée par `concat!` |

Les deux gardes lexicales lisent par `ProductionScanner::production_of`, donc le
contrôle positif du critère 3 ne les fait pas rougir.

---

## 6. Definition of Done

1. `mika_a2a::params::CALLBACK_TURN_KEY` existe, vaut `"mika.callback_turn"`, porte
   une doc qui dit les quatre effets, la contrainte booléenne et l'asymétrie
   absence/malformé, et son orthographe est épinglée.
2. `requested_callback_turn` existe dans `server/a2a.rs`, est le **seul** lecteur de
   la clé, réutilise `shape_of`, et ne redéclare rien.
3. `message/send` et `message/stream` l'appellent, chacun avant le verrou d'agent,
   avant la ligne de tâche et avant l'ouverture du flux, et refusent par une
   `JsonRpcError` `INVALID_PARAMS` qui nomme la clé et la forme sans écho de valeur.
4. `AgentParams.is_callback_turn` reçoit le booléen de l'appelant ;
   `grep -n "is_callback_turn: false" crates/mika-agent/src/server/a2a.rs` ne rend
   plus rien en production.
5. Un tour réel avec la clé à `true` n'enregistre pas le message utilisateur, et le
   même tour avec `false` l'enregistre.
6. Les deux `CLAUDE.md` portent la clé et disent que rien ne la pose encore.
7. Seuls les quatre fichiers du § 3 changent.
   `cargo clippy --all-targets -p mika-a2a -p mika-agent` vert sans nouvel `allow`.
   `cargo test -p mika-a2a -p mika-agent` vert.
   `bash scripts/verify-egress-manifest.sh --report` vert.
   `bash scripts/verify-pipeline.sh origin/main` vert.
8. La PR porte `Refs mika#1727` et aucun `Closes`, `Fixes` ni `Resolves` — corps et
   messages de commit compris. Si le gabarit de reprise a ouvert la PR (label
   `wip-rescue`), le `Closes #N` qu'il écrit par défaut est corrigé **avant toute
   revue**, et `gh pr view <N> --json closingIssuesReferences` rend `[]`.
9. `Tracked in: senara-solutions/mika#1727` figure dans le corps de la PR.
10. Aucun ticket n'a été ouvert.

---

## 7. Acceptance criteria

Transcrits du corps de mika#1727, § « Critères de V3a-1 ».

1. `mika_a2a::params::CALLBACK_TURN_KEY` vaut `"mika.callback_turn"`, et un test
   l'épingle.
2. Sur la porte A2A, une requête sans la clé, ou avec `null` ou `false`, donne un
   tour dont `is_callback_turn` est faux, comme sur main. Un test par cas.
3. Une requête avec la clé à `true` donne un tour dont `is_callback_turn` est vrai.
   Le test lit le fait du côté de la boucle (par exemple l'absence d'enregistrement
   du message utilisateur, ou ce que voit un fournisseur factice), et non sur une
   variable intermédiaire.
4. Une requête avec la clé à une valeur non booléenne est refusée avant tout tour.
   Le refus nomme la clé et la forme reçue, et ne contient pas la valeur envoyée.
   C'est le test négatif de ce vol.
5. `message/send` et `message/stream` lisent la clé par le même lecteur. Un test par
   porte, ou un test qui atteste qu'il n'existe qu'un lecteur et que les deux
   appelants l'appellent.
6. `grep -n "is_callback_turn: false" crates/mika-agent/src/server/a2a.rs` ne rend
   plus rien en production.
7. Seuls `crates/mika-a2a/src/params.rs`, `crates/mika-agent/src/server/a2a.rs`,
   `crates/mika-a2a/CLAUDE.md` et `crates/mika-agent/CLAUDE.md` changent. Le pilote
   lance `cargo clippy --all-targets` sur `mika-a2a` et `mika-agent`, vert, sans
   `allow`. Il lance aussi `bash scripts/verify-egress-manifest.sh --report` avant
   d'ouvrir la PR. S'il le trouve rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris
   si le gabarit de reprise l'ouvre.

---

## Fire-Disposition

Ce vol livre des détecteurs : la batterie du lecteur, la paire de tours réels, et
**deux gardes lexicales** (critères 5 et 6) dont le chemin de succès est « aucune
violation trouvée ».

**Option retenue : (a) exception nommée en allowlist — avec une allowlist VIDE.**

La mesure qui l'autorise : sur `main` @ `f9855e96`,
`grep -rn "is_callback_turn" crates/mika-agent/src/server/a2a.rs` rend **une seule**
occurrence, la l. 343, et c'est précisément celle que le commit 3 remplace. De
même, `grep -rn "callback_turn" crates/mika-a2a/` est vide : il n'existe aucun
second lecteur à excepter. Les deux gardes atterrissent donc **armées, vertes, et
sans aucune entrée d'exception** — il n'y a rien à excepter, et une allowlist
inventée pour la forme serait une entrée qui ne nomme aucune donnée, c'est-à-dire
exactement ce que mika#1574 interdit.

Ni (b) ni (c) ne s'appliquent : rien ne justifie de livrer désarmé un détecteur qui
passe au vert dans le commit qui l'introduit, et il n'y a pas de cadrage à remonter
sur ce point — la seule question ouverte du vol est la tension de périmètre du § D5,
qui n'est pas une question de détecteur.

**Ce qui fait rougir ces gardes plus tard**, et c'est leur raison d'être : un
`is_callback_turn: false` réintroduit en dur dans la production de `a2a.rs` (critère
6), ou un second lecteur de la clé copié-collé à côté du premier (critère 5) — la
divergence silencieuse que `mika1951_the_isolation_flag_has_exactly_two_readers_in_the_loop`
existe déjà pour empêcher sur son propre drapeau.

---

## 8. Ce que ce vol ne fait pas

- **Il ne pose pas la clé.** Aucun appelant ne l'écrit à la fin de ce vol ; c'est
  le travail de V3a-2, où le bras `CallbackResult` de `commands/chat.rs` part sur le
  fil. La doc le dit, pour que l'absence d'appelant se lise comme un état voulu.
- **Il ne touche pas la boucle.** Le sens d'`is_callback_turn` dans
  `agent_loop/mod.rs` est exactement celui de `main`, à l'octet.
- **Il ne retire rien de la TUI.** Le provider, les outils, le MCP et les skills
  locaux partent en V3b ; `ask.rs:167` avec eux.
- **Il n'atteste rien en retour.** `mika.session_isolated` a une clé d'attestation
  parce qu'un appelant doit pouvoir savoir si une **mesure** a vraiment tourné
  isolée. Ici l'appelant est celui qui *produit* le cadrage de callback : il sait ce
  qu'il a envoyé, et l'effet observable du tour est en base chez lui. Une
  attestation serait une clé de plus sans lecteur, et le vol en a déjà une.
- **Il n'ouvre aucun ticket** (gel jusqu'au 01/12).

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-a2a/src/params.rs` — constante + doc + épinglage + liste d'unicité | 46 |
| `crates/mika-agent/src/server/a2a.rs` — `requested_callback_turn` + doc | 45 |
| `crates/mika-agent/src/server/a2a.rs` — paramètre, passage, deux portes, `StreamTurn` | 35 |
| tests — batterie du lecteur (critères 1, 2, 4) | 40 |
| tests — fixture d'état local (critère 3, D5) | 110 |
| tests — paire de tours réels (critère 3, D6) | 45 |
| tests — gardes lexicales (critères 5 et 6) | 40 |

Total estimé : 361 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Les deux `CLAUDE.md` ajoutent ≈ 30 lignes, hors de l'unité de l'architecte mais
dans celle du corps du ticket : **≈ 391 lignes ajoutées pour une borne de 450**,
mesurées par `git diff --numstat origin/main...HEAD`, plan exclu. Les deux bornes
sont tenues, la plus serrée avec ≈ 68 lignes de marge.

Un seul vol, et voici pourquoi il tient : le code de production pèse ≈ 126 lignes
et instancie une forme déjà écrite six fonctions plus haut dans le même fichier.
Le poste qui pourrait faire glisser l'estimation est le fixture d'état du § D5
(110 lignes de littéraux de struct recopiés) ; il est isolé dans le **dernier**
commit de code, ce qui rend la halte sur borne la plus fine exactement là où
l'incertitude est la plus grande. Les commentaires comptent dans la borne et ne se
rabotent pas pour la tenir : si le code écrit dépasse, le pilote s'arrête et le
dit, selon la halte du § 4.
