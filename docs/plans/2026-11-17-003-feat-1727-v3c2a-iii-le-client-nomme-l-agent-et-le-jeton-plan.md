# mika#1727 — vol V3c-2a-iii : le client nomme l'agent et le jeton

- **Ticket** : senara-solutions/mika#1727 — *TUI as thin HTTP client of mika-spirit*
- **Vol** : V3c-2a-iii, le dernier des trois vols de V3c-2a. V3c-2a-i (le serveur, mika#3101) et V3c-2a-ii (le client et trois refus, mika#3102) sont mergés ; viennent ensuite V3c-2b (la TUI lit la route et cesse d'assembler), V3c-2c (le test É1) puis V4.
- **Branche** : `feat/1727/v3c2a-iii-le-client-nomme-l-agent-et-le-jeton`, neuve depuis `main`
- **Base mesurée** : `main` @ `d014e890` (le merge de V3c-2a-ii) — la tête sur laquelle MPC a mesuré, et la tête de cette branche
- **Label** : `loop-substrate` (plafond de tours : 200)
- **Borne du vol** : au plus **220 lignes ajoutées**, plan exclu, par `git diff --numstat origin/main...HEAD`
- **La PR porte `Refs mika#1727`**, jamais `Closes`, `Fixes` ni `Resolves` — ni dans le corps, ni dans un commit. Le ticket se ferme au merge de V4.
- **Ce vol ne câble rien.** Aucun lecteur de `app.skills` ne change, les attentes `dead_code` de `skills_remote.rs` restent telles quelles, et `lire` n'a toujours aucun appelant de production.

---

## 0. D'où vient ce plan, et ce qu'il hérite

La tête du ticket désigne sa source : **la décision D3 du plan 005**
(`2026-11-16-005-feat-1727-v3c2a-la-route-des-skills-a-son-client-plan.md`, sur
la branche `feat/1727/v3c2a-la-route-des-skills-a-son-client`, lu par
`git show FETCH_HEAD:…`). Ce plan 005 portait **une halte de groom** : son vol
unique pesait 517 lignes pour une borne de 452, et Vincent l'a coupé en trois.
D3 décrivait les **cinq** refus du client en un tableau ; trois ont été livrés
par V3c-2a-ii, et ce vol reprend les deux qui restent, **et rien d'autre** :

| variant de D3 | état |
|---|---|
| `Injoignable { url, cause }` | livré (V3c-2a-ii) |
| `Statut { status, url, body }` | livré (V3c-2a-ii) |
| `CorpsIllisible { url, body, cause }` | livré (V3c-2a-ii) |
| **`AgentInconnu { agent, url }`** | **ce vol** |
| **`JetonRefuse { url }`** | **ce vol** |

Ce qu'on hérite de D3, inchangé : la raison du **non-réemploi** de
`SessionRemoteIssue`. Son `NotFound` écrit « unknown agent, or unknown session »,
là où un 404 de cette route-ci ne peut vouloir dire qu'**agent inconnu** — un
message qui nomme une session enverrait l'opérateur chercher la panne là où elle
n'est pas. Cette phrase est déjà sur le type, en production (`skills_remote.rs`
l. 109-113) ; ce vol ne la réécrit pas, il la **rend vraie** en donnant au 404
son variant.

Ce qui est **re-mesuré** et non hérité : tout le reste. D3 décrivait un fichier
qui n'existait pas encore ; le § 1 le mesure tel qu'il est sur `main`.

---

## 1. Ce qui est mesuré sur `main` (@ `d014e890`)

### `crates/mika-cli/src/commands/skills_remote.rs` (416 l.)

- **L'en-tête du module (l. 1-20)** nomme « vol V3c-2a-ii », « un vocabulaire de
  refus à **trois** variants », et ses l. 18-20 **annoncent ce vol** : « V3c-2a-iii
  éclate avant cela les deux refus métier, 404 agent inconnu et 401 jeton
  refusé, qui sortent d'ici par [`SkillsRemoteIssue::Statut`] en attendant ».
- **Le commentaire du type (l. 109-123)** porte trois paragraphes : le
  non-réemploi de `SessionRemoteIssue` (l. 109-113), le pourquoi des variants
  distincts (l. 115-120), et **l'annonce de ce vol** (l. 122-123) : « 404 et 401
  sortent par `Statut` dans ce vol, et **aucun bras mort** ne les attend :
  V3c-2a-iii les éclate avec leurs tests. »
- **`SkillsRemoteIssue`** (l. 124-140) : trois variants, pas d'attente
  `dead_code` sur le type lui-même.
- **`Display`** (l. 143-161) : trois bras, écrit à la main, avec un commentaire
  sur `CorpsIllisible` qui dit pourquoi le corps voyage avec la cause.
- **`lire(base_url, token, agent)`** (l. 175-216) : l'attente
  `#[cfg_attr(not(test), expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727"))]`,
  puis le GET, puis **l. 200-210** la lecture du statut avant le corps et le
  `if statut != 200 { return Err(Statut { … }) }`.
- **Le module `tests`** (l. 223-415) : `ROUTE`, `JETON`, la constante `CORPS`, le
  helper `appel(statut, corps, agent)` (l. 263-275) qui monte
  `rewind_remote::tests::fake_spirit` et rend `(base, recues, rendu)`, puis
  **cinq** tests. Le test du 500 (l. 376-391) lit le statut, l'URL et le corps
  dans le message.

### Les voisins, et la route

- `session_remote.rs` l. 127-130 : le `Denied` écrit
  « mika-spirit denied the request: set MIKA_INTERNAL_TOKEN to the server's
  token » — la formule que le 401 de ce vol reprend, parce que le geste est le
  même et qu'une seconde formulation du même geste dérive.
- `session_remote.rs` l. 121-124 : le `NotFound` écrit « unknown agent, or
  unknown session ». C'est ce message que `AgentInconnu` existe pour **ne pas**
  dire ici.
- La route sert 404 `{"error": "agent '<nom>' not found"}` pour un agent
  introuvable, et elle est sous `require_dashboard_or_internal_token`, donc un
  jeton refusé rend 401.

### L'entrée d'egress `cli-spirit-skills` (lue, **non modifiée**)

`docs/egress/egress-manifest.toml` l. 374-382 déclare un `client_site` et un
`call_site` tous deux sur `skills_remote.rs`, et sa ligne `data` décrit **ce qui
part** : « un GET vers /api/v1/skills : le nom de l'agent en paramètre de
requête, et le jeton interne en en-tête Authorization ; aucun corps ». Ce vol ne
change ni le site de construction du client, ni ce qui part sur le fil — il
change la **lecture de la réponse**. L'entrée reste donc exacte et
`docs/egress/` reste intact, conformément à la tête du ticket.

### Le cap de `crates/mika-cli/CLAUDE.md`

Le cap est de 100 000 octets (`CLAUDE_MD_MAX_BYTES`,
`crates/mika-common/tests/claude_md_size.rs`). Ce vol **réécrit** un paragraphe
existant de huit lignes au lieu d'en ajouter un : la variation nette est de
quelques dizaines d'octets, dans un sens ou dans l'autre. Le critère 7 fait
tourner `cargo test -p mika-common --test claude_md_size` de toute façon.

### Écarts entre la tête du ticket et la mesure

**É-A — le texte à corriger est à trois endroits, pas deux.** La tête nomme « le
commentaire du type (l. 122 et 123) » et le paragraphe de `CLAUDE.md`. La mesure
trouve un **troisième** site dans le même fichier : l'en-tête du module (l. 2 :
« vol V3c-2a-ii » ; l. 3 : « à trois variants » ; l. 18-20 : l'annonce explicite
de V3c-2a-iii). Le **critère 4** l'exige — « Aucun texte de `skills_remote.rs` …
n'annonce V3c-2a-iii » — et il porte sur le fichier entier. Ce n'est donc pas un
élargissement : c'est le même fichier, déjà au périmètre, et le critère le
demande nommément. Résolution en D5.

**É-B — aucune attente `dead_code` neuve n'est requise, et c'est mesuré, pas
supposé.** Les deux variants neufs sont construits dans `lire`, qui porte déjà
son `expect(dead_code)`. `rustc` ajoute un item marqué `allow`/`expect(dead_code)`
aux **racines** de son analyse d'atteignabilité, donc ce que cet item construit
est vivant : c'est exactement pourquoi les trois variants actuels, construits au
même endroit, ne portent aucune attente aujourd'hui. Les deux neufs suivent le
même chemin. **Si une attente `dead_code` neuve devenait malgré tout nécessaire,
c'est une halte** (critère 5 : « Aucune attente ni aucun `allow` neuf ») — pas
une entrée de plus. La passe qui tranche est `cargo build -p mika-cli --bin mika`,
que le critère 7 place **en premier** pour cette raison.

---

## 2. Décisions

### D1 — `AgentInconnu { agent, url }` porte le nom **demandé**, et pas le corps du serveur

```rust
AgentInconnu { agent: String, url: String },
```

Message : `mika-spirit does not serve an agent named '{agent}' at {url}`.

Trois choses s'y décident, et chacune a sa raison :

- **Le nom vient de l'argument `agent` de `lire`, jamais du corps du 404.** Le
  serveur renvoie `{"error": "agent '<nom>' not found"}`, qui est une
  reformulation du nom qu'on vient d'envoyer : le relire coûterait une
  désérialisation qui peut échouer (un spirit d'une autre version, un proxy
  intermédiaire) pour retrouver une chaîne que l'appelant détient déjà. Et un
  échec de cette relecture rendrait un message **sans** nom d'agent, là où le
  seul fait utile est justement ce nom.
- **Le corps n'est pas porté par le variant.** Il n'apporte rien que le couple
  `(agent, url)` ne dise déjà, et le garder transformerait le message en
  redite : « does not serve an agent named 'x' … : agent 'x' not found ».
  `Statut` reste le variant qui montre un corps, parce que c'est là que le corps
  est la seule information disponible.
- **Le message ne contient pas le mot « session », et c'est tout l'objet du
  variant** (§ 0). Le critère 1 l'atteste par une assertion négative, qui est le
  seul geste capable de voir une régression de formulation.

**Ce que ce variant ne garantit pas, et qui est dit sur place.** Un 404 ne peut
vouloir dire qu'« agent inconnu » *sur cette route, servie par un spirit qui la
monte*. Un spirit qui ne monterait pas `/api/v1/skills` du tout rendrait aussi un
404, et le message nommerait alors l'agent à tort. C'est la lecture que la tête
du ticket a déjà tranchée — « Sur cette route, un 404 ne veut dire qu'agent
inconnu » — et la limite est écrite en commentaire plutôt que gardée : la route
existe depuis bien avant V3c-1, et une garde ici demanderait au client de
distinguer deux 404 que le serveur ne distingue pas.

### D2 — `JetonRefuse { url }` reprend la formule du voisin, mot pour mot

```rust
JetonRefuse { url: String },
```

Message : `mika-spirit denied the request for {url}: set MIKA_INTERNAL_TOKEN to the server's token`.

La seconde moitié est **la phrase de `session_remote::SessionRemoteIssue::Denied`**
(l. 129), reprise telle quelle : le geste de remédiation est identique, et deux
formulations du même geste dérivent — c'est l'argument qui a fondu les deux
lectures de jeton en un seul `jeton_interne` au vol V2d-4b. Elles ne sont pas
mises **en commun** pour autant : ce sont deux `Display` de deux types, et
partager un littéral entre deux énumérations de deux modules créerait une
dépendance de texte là où il n'y a aucune dépendance de domaine.

La première moitié **diffère** du voisin, qui ne nomme pas d'URL. Ici elle est
nommée, parce que le client peut viser un spirit autre que celui du shell
(`base_url` est un argument) et que « quel serveur a refusé » est la moitié de la
question quand deux daemons tournent.

Le variant ne porte **ni corps ni statut** : un 401 de cette route a un corps
vide ou un `{"error": …}` générique, et le `401` est déjà dans le nom du variant.

### D3 — le `match` de statut remplace le `if`, et garde sa place

```rust
    let statut = reponse.status().as_u16();
    let corps = reponse.text().await.unwrap_or_default();
    if statut != 200 {
        return Err(match statut {
            404 => SkillsRemoteIssue::AgentInconnu { agent: agent.to_string(), url },
            401 => SkillsRemoteIssue::JetonRefuse { url },
            _ => SkillsRemoteIssue::Statut { status: statut, url, body: corps },
        });
    }
```

- **Le `if statut != 200` est gardé**, et le `match` niché dedans plutôt que
  mis à sa place. Un `match statut { 200 => {}, … }` à plat réécrirait le chemin
  de succès pour deux cas d'erreur, et c'est un diff que la revue doit relire
  sans que rien n'ait changé de ce côté.
- **Le bras `_` reste `Statut`**, et il est le dernier : « Tout autre statut que
  200 sort toujours par `Statut` » (périmètre). Le critère 3 l'atteste sur le
  500, sans toucher au test existant.
- **La lecture du corps reste avant le branchement**, inchangée. Les deux
  nouveaux bras ne le consomment pas ; seul `Statut` le fait, et comme il est le
  dernier bras, aucun `clone` n'est nécessaire.
- **`agent.to_string()` au site de construction.** `lire` reçoit un `&str` et le
  variant porte un `String` : un variant à durée de vie obligerait tout lecteur
  du refus à tenir la vie de l'argument, pour un nom d'agent de vingt octets.

### D4 — deux tests, par le helper `appel`, et aucun second factice

Les deux tests empruntent `appel(statut, corps, agent)` (l. 263), donc
`rewind_remote::tests::fake_spirit`, exactement comme les trois refus déjà
livrés. **Aucun second factice**, pour la raison déjà écrite dans le module
(l. 226-228) : un second témoin de socket ouvrirait une seconde vérité sur ce que
le serveur reçoit, libre d'en diverger.

Chaque test sert le **corps que la route sert réellement** pour son statut — le
JSON `{"error": "agent 'fantome' not found"}` pour le 404, un corps vide pour le
401 — plutôt qu'une chaîne arbitraire : c'est ce qui rend le test capable de voir
qu'un futur éditeur a mis ce corps dans le message.

**Le test du 404 porte trois assertions, dont une négative** :

1. `matches!(probleme, SkillsRemoteIssue::AgentInconnu { .. })` — le variant ;
2. le message contient le nom d'agent **demandé** et l'URL complète ;
3. **le message ne contient pas « session »** — l'assertion sans laquelle ce
   variant ne prouve rien, puisqu'il n'existe que pour ne pas dire ce mot.

Le nom d'agent du test est choisi pour que l'assertion négative ne soit pas
vacante par accident : un nom sans rapport avec la formulation du voisin.

**Le test du 401** atteste le variant et que le message nomme
`MIKA_INTERNAL_TOKEN`.

Les deux sont **négatifs sur `main`** : aujourd'hui les deux statuts sortent par
`Statut`, dont le message n'a ni le nom d'agent ni le nom de la variable
d'environnement. C'est la propriété que les critères 1 et 2 exigent, et elle
tient par construction — les `matches!` échouent avant même les assertions de
message.

### D5 — les trois textes cessent d'annoncer ce vol, et décrivent cinq refus

Conséquence de É-A. Trois sites, tous dans `skills_remote.rs` sauf le dernier :

1. **L'en-tête du module** (l. 1-20) : « vol V3c-2a-ii » → V3c-2a-iii, « trois
   variants » → cinq, et les l. 18-20 perdent l'annonce. Ce qu'elles disaient
   d'autre — que la porte n'a aucun appelant de production et que les attentes
   `dead_code` tombent à V3c-2b — **reste vrai et reste écrit** : ce vol ne câble
   rien.
2. **Le commentaire du type** (l. 122-123) : le paragraphe d'annonce est
   remplacé par ce que les deux variants disent, c'est-à-dire pourquoi 404 et 401
   ne sont pas des `Statut` — le premier parce qu'un 404 d'ici a une seule cause
   et un seul remède, le second parce que son remède est une variable
   d'environnement et non une lecture de corps. Les deux paragraphes qui
   précèdent (l. 109-120) **ne changent pas** : ils sont déjà exacts, et les
   réécrire ferait un diff que rien ne demande.
3. **`crates/mika-cli/CLAUDE.md`**, paragraphe « La route des skills a son
   client, pas encore son appelant » : le titre gagne V3c-2a-iii, la phrase
   « Trois refus distincts » devient cinq, et la parenthèse « **404 et 401
   compris** jusqu'à V3c-2a-iii qui les éclate » disparaît au profit de ce que
   les deux refus nomment. La phrase « **Rien ne l'appelle** » **reste** : elle
   est toujours vraie.

Aucun de ces textes ne garde « jusqu'à V3c-2a-iii » (critère 4), et aucun ne dit
plus que 404 et 401 sortent par `Statut`.

### D6 — ce plan ne porte pas de section `## Fire-Disposition`, et voici la mesure

La règle mika#2306 définit un détecteur comme « tout livrable dont la fonction
primaire est de signaler une violation … tout code dont le chemin de succès est
*aucune violation trouvée* ». Les deux tests de ce vol ne sont pas de cette
population : leur chemin de succès est « `lire` a rendu le variant attendu et son
message porte ce qu'il doit porter », une **assertion de comportement** sur un
transport, mesurée contre un factice qui sert un statut connu. Ils ne balaient
aucun fichier, ne tiennent aucune liste d'exceptions, et ne peuvent rougir sur un
code qu'ils n'ont pas appelé.

Ce vol ne livre **aucun** scan structurel, aucune règle de lint, aucune garde CI
et aucune garde EndTurn — à la différence de la plupart des vols de ce ticket,
qui en portent (`mika1727_v3b1_…`, `mika1727_v2d4b_…`, …). Le gate est donc
**N/A**, et la section n'est pas inventée, conformément à la dernière phrase de
la règle.

### D7 — ce que ce vol ne fait pas

Nommé ici pour que la QA le juge contre le plan plutôt que contre une attente :

- **aucun câblage**. `app.skills` reste assemblé localement, l'autocomplétion,
  `/skills`, `/skill`, les messages de démarrage, `spawn_agent_worker`,
  `updated_skills` et `migrate_disabled_markers_async` ne sont pas touchés.
  C'est V3c-2b ;
- **les attentes `dead_code` de `skills_remote.rs` restent telles quelles** —
  mêmes items, même forme, même `reason`. Aucune n'est ajoutée (É-B), aucune
  n'est retirée : elles tombent à V3c-2b, avec leur appelant ;
- **le test É1** n'est pas écrit. C'est V3c-2c ;
- **`docs/egress/`** n'est pas touché : ce qui part sur le fil ne change pas
  (§ 1) ;
- **les cinq tests existants** de `skills_remote.rs` ne sont pas modifiés, et
  `fake_spirit` ne gagne rien — le témoin de requête lui a été donné à V3c-2a-ii.

`Tracked in: senara-solutions/mika#1727`

---

## 3. Ordre de commit

Un commit par garde, après chaque garde verte **avec son test**, jamais en fin de
vol. L'ordre est celui de la tête du ticket.

### C1 — `AgentInconnu`, son bras dans `lire`, son test (critère 1) — ≈ 50 lignes

1. Le variant `AgentInconnu { agent, url }` dans `SkillsRemoteIssue`.
2. Son bras de `Display`, avec le commentaire qui dit pourquoi le nom vient de
   l'argument et pas du corps (D1), et la limite du « un seul sens du 404 ».
3. Dans `lire`, le `return Err(match statut { … })` de D3, avec son bras `404`
   et le bras `_` conservé.
4. Le test `mika1727_un_404_nomme_lagent_demande_et_jamais_une_session`, ses
   trois assertions dont la négative (D4).

Vert : `cargo test -p mika-cli skills_remote` et
`cargo clippy --all-targets -p mika-cli`.

### C2 — `JetonRefuse`, son bras dans `lire`, son test (critère 2) — ≈ 35 lignes

1. Le variant `JetonRefuse { url }`.
2. Son bras de `Display`, reprenant la formule du voisin, avec le commentaire
   qui dit d'où elle vient et pourquoi elle n'est pas mise en commun (D2).
3. Le bras `401` du `match` déjà posé en C1 — **une ligne**, parce que C1 a fait
   la structure.
4. Le test `mika1727_un_401_nomme_la_variable_de_jeton`.

Vert : les mêmes deux commandes.

### C3 — les trois textes (critère 4) — ≈ 26 lignes, dont ≈ 12 hors unité architecte

Les trois sites de D5 : l'en-tête du module, le commentaire du type, le
paragraphe de `crates/mika-cli/CLAUDE.md`.

Vert : `cargo test -p mika-common --test claude_md_size`, plus une relecture
`grep -n 'V3c-2a-iii\|trois variants\|404 et 401' crates/mika-cli/src/commands/skills_remote.rs crates/mika-cli/CLAUDE.md`
qui ne doit plus rendre d'annonce de ce vol.

### Fin de pipeline

Dans cet ordre, et le premier en premier (critère 7) :

1. `cargo build -p mika-cli --bin mika` — c'est la passe qui tranche É-B ;
2. `cargo test -p mika-cli` ;
3. `cargo test -p mika-common --test claude_md_size` ;
4. `cargo clippy --all-targets -p mika-cli` ;
5. `cargo fmt` ;
6. `bash scripts/verify-pipeline.sh origin/main` — **rouge ⇒ halte**, le pilote
   s'arrête et le dit, sans ouvrir la PR.

Puis la PR, corps écrit dans `pr-body.md` **à la racine du worktree**
(mika#2211), portant `Refs mika#1727` et **aucun** `Closes` / `Fixes` /
`Resolves` (critère 8), la ligne
`Code review: delegated to mika-qa (mika#2694)` et
`Tracked in: senara-solutions/mika#1727`.

### Halte du pilote sur la borne

Après **chaque** commit, `git diff --numstat origin/main...HEAD`, plan exclu.
**Au-dessus de 220, le pilote s'arrête** : pas de commit suivant, pas de PR, et
son dernier message dit « HALTE borne : N lignes pour 220 » avec le compte par
commit. Une halte ne se lève jamais par un amendement du plan — c'est à Vincent
de la lever.

Les commentaires comptent dans la borne et **ne se rabotent pas** pour la tenir.
Si le code écrit dépasse, le pilote s'arrête ; il ne retire pas après coup les
commentaires qui portent un pourquoi.

### Si le code s'écarte de ce plan

La QA juge contre ce plan, pas contre le corps du ticket. Si le pilote doit
s'écarter d'un point — un fichier, un nom, un critère — il **amende ce plan dans
un commit `docs(plans)` avant** de pousser le code qui s'en écarte, en nommant le
point et la raison.

---

## 4. Definition of Done

- `SkillsRemoteIssue` porte cinq variants, dont `AgentInconnu { agent, url }` et
  `JetonRefuse { url }`, chacun avec son bras de `Display`.
- `lire` range un 404 dans `AgentInconnu`, un 401 dans `JetonRefuse`, et tout
  autre statut que 200 dans `Statut`.
- Deux tests neufs, par le helper `appel`, sans second factice ; les cinq tests
  existants passent **sans modification**.
- Aucun texte de `skills_remote.rs` ni de `crates/mika-cli/CLAUDE.md` n'annonce
  V3c-2a-iii ni ne dit que 404 et 401 sortent par `Statut`.
- Seuls `crates/mika-cli/src/commands/skills_remote.rs` et
  `crates/mika-cli/CLAUDE.md` changent.
- Aucune attente `dead_code` ni aucun `allow` neuf ; les attentes existantes sont
  inchangées.
- Le diff reste sous 220 lignes ajoutées, plan exclu.
- `cargo build -p mika-cli --bin mika`, `cargo test -p mika-cli`,
  `cargo test -p mika-common --test claude_md_size`,
  `cargo clippy --all-targets -p mika-cli` et
  `bash scripts/verify-pipeline.sh origin/main` sont verts.
- La PR porte `Refs mika#1727` et aucun `Closes` / `Fixes` / `Resolves`.

---

## 5. Acceptance criteria

Transcrits du corps de mika#1727, § *Critères de V3c-2a-iii* :

1. Contre `fake_spirit` servant un 404 sur la route, `lire` rend `AgentInconnu`.
   Son message porte le nom d'agent demandé et l'URL, et ne contient pas le mot
   « session ». Ce test est négatif sur main, où un 404 sort par `Statut`.
2. Contre `fake_spirit` servant un 401, `lire` rend `JetonRefuse`, dont le
   message nomme `MIKA_INTERNAL_TOKEN`. Ce test est négatif sur main, où un 401
   sort par `Statut`.
3. Les cinq tests existants de `skills_remote.rs` passent sans changement. Le 500
   sort toujours par `Statut`, avec son statut, son URL et son corps.
4. Aucun texte de `skills_remote.rs` ni de `crates/mika-cli/CLAUDE.md` ne dit
   encore que 404 et 401 sortent par `Statut`, ni n'annonce V3c-2a-iii.
5. Seuls changent les deux fichiers listés plus haut. Aucune attente ni aucun
   `allow` neuf, et les attentes `dead_code` existantes sont inchangées.
6. Le diff reste sous 220 lignes ajoutées, plan exclu.
7. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-cli` et
   `cargo test -p mika-common --test claude_md_size` sont verts.
   `cargo clippy --all-targets -p mika-cli` est vert. Le pilote lance
   `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il le
   trouve rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y
   compris si le gabarit de reprise l'ouvre.

---

## Taille estimée

La tête du ticket fixe l'unité de ce total : celle de **sa** borne, c'est-à-dire
`git diff --numstat origin/main...HEAD`, plan exclu et `crates/mika-cli/CLAUDE.md`
**compris**. Elle demande aussi l'estimation hors `docs/` et hors `CLAUDE.md`,
qui est l'unité de l'architecte et celle de la ligne `Borne par vol` ci-dessous ;
les deux sont données.

| livrable | lignes ajoutées |
|---|---|
| `skills_remote.rs` — les deux variants et leurs deux bras de `Display`, commentaires compris | 26 |
| `skills_remote.rs` — le `match` de statut dans `lire` et son commentaire | 16 |
| `skills_remote.rs` — l'en-tête du module et le commentaire du type, réécrits | 16 |
| tests (`skills_remote.rs`, module `tests`) — les deux tests, commentaires compris | 50 |
| `crates/mika-cli/CLAUDE.md` — le paragraphe réécrit (hors unité architecte) | 12 |

Total estimé : 120 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Dans l'unité de l'architecte** (hors `docs/`, hors `CLAUDE.md`, celle de la
ligne `Borne par vol` et de la seconde borne de la tête) : **108 lignes**, les
120 ci-dessus moins les 12 du paragraphe de `CLAUDE.md`.

Les deux bornes de la tête du ticket sont tenues, et largement :

| unité | estimé | borne | marge |
|---|---|---|---|
| borne du vol (`--numstat`, plan exclu, doc comprise) | 120 | 220 | +100 |
| architecte (hors `docs/`, hors `CLAUDE.md`) | 108 | 210 (tête) / 429 (mika#2693) | +102 / +321 |

**Aucune halte de groom.** C'est la différence exacte avec le plan 005 dont ce
vol descend, qui s'arrêtait à 517 estimées pour 452 : la coupe que Vincent a
faite en trois vols a tenu, et celui-ci est le plus petit des trois.

### Pourquoi ce vol tient, en un seul vol

Trois raisons mesurées, et non une impression de petitesse :

1. **Rien n'est construit, tout est réemployé.** Le fichier, son module `tests`,
   le helper `appel`, la constante `CORPS`, le factice `fake_spirit` et son
   témoin de requête existent déjà. Les deux tests sont deux appels à `appel` et
   une poignée d'assertions ; le vol V3c-2a-ii a payé tout le montage.
2. **La structure du branchement est posée une fois, en C1, et C2 y ajoute une
   ligne.** C'est pourquoi C2 pèse 35 et non 50 : le second variant arrive dans
   une place déjà faite.
3. **Le précédent chiffre la nature du travail.** V3c-2a-ii a coûté 354 lignes
   estimées pour cinq miroirs (23 champs), trois variants, cinq tests et une
   entrée d'egress. Ce vol n'a ni miroir, ni egress, ni factice à monter : deux
   variants et deux tests, soit la part de ce total qu'on peut en isoler — 108
   est de cet ordre, et la marge de 102 lignes absorbe une sous-estimation de
   presque 100 %.
