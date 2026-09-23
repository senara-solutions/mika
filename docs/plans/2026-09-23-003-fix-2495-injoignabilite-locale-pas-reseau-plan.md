# mika#2495 — L'injoignabilité d'une fixture de test est locale, jamais réseau

> **Parent umbrella :** #2491 (Défaut 3). Enfant du cadre substrat — 1 PR atomique.

## Le défaut, reproduit et attribué

`milestone_manager::cadence::tests::probe_executor_health_returns_none_on_bogus_host`
sonde `http://192.0.2.1:1/health` (RFC 5737) et attend `None`. Reproduit dans le
bac à sable pilote de ce grooming, 2026-09-23 :

```
thread '…probe_executor_health_returns_none_on_bogus_host' panicked at
  crates/mika-agent/src/milestone_manager/cadence.rs:1042:9:
assertion `left == right` failed
  left: Some(false)
 right: None
test result: FAILED. 1 passed; 1 failed; … finished in 0.03s
```

**Le `0,03 s` est la mesure qui attribue la cause.** Une tentative de connexion
vers une adresse non routable depuis un `--unshare-net` rend `ENETUNREACH`, ou
consomme le `timeout(5s)` du client. Trois centièmes de seconde sont un
aller-retour de boucle locale.

### Le mécanisme, lu et non deviné

`probe_executor_health` (`cadence.rs:694`) construit son client par
`reqwest::Client::builder()`, dont la détection de proxy par variables
d'environnement est **active par défaut**. Le bac à sable Phase 2b pose
(`dispatch-lib.sh:1229-1232`) :

```
--setenv HTTP_PROXY  "http://127.0.0.1:$_PILOT_EGRESS_TCP_PORT"
--setenv HTTPS_PROXY "http://127.0.0.1:$_PILOT_EGRESS_TCP_PORT"
--setenv NO_PROXY    "localhost,127.0.0.1"
```

`192.0.2.1` n'est pas dans `NO_PROXY`, donc la requête part **vers le relais**
en forme absolue : `GET http://192.0.2.1:1/health HTTP/1.1`. Dans
`handle_host_client` (`scripts/mika-pilot-egress-proxy:1119-1131`), cette ligne
n'apparie ni `ANTHROPIC_METHOD_RE` (qui exige un chemin préfixé
`/anthropic-proxy/`) ni `CONNECT_RE` (qui exige le verbe `CONNECT`), donc le
relais répond `HTTP/1.1 400 Bad Request`. `res.status().is_success()` vaut
`false` ⇒ **`Some(false)`**.

**L'adresse n'est jamais résolue et jamais composée.** C'est la première
rectification que ce plan apporte au ticket.

## Ce que le ticket propose, et pourquoi les deux pistes sont écartées

### Piste 1 — « le proxy d'egress ne doit pas répondre aux adresses RFC 5737/3849 »

Écartée sur trois motifs, dont le premier est factuel.

1. **Elle décrit un comportement que le relais n'a pas.** Il ne « répond pas à
   une adresse » : il répond `400` à une requête de proxy malformée, sans
   jamais regarder l'hôte. Il n'y a pas de branche à corriger — il faudrait en
   *créer* une, qui inspecte l'hôte d'une forme absolue que le relais ne sert
   volontairement pas.
2. **Elle met la connaissance des fixtures de test dans la frontière de
   containment.** Depuis mika#2049 ce relais est *fail-closed* : relais
   indisponible ⇒ dispatch refusé (`CONTAINMENT REFUSAL`, exit 78). Y ajouter
   un cas particulier pour faire verdir un test unitaire est un changement à
   rayon d'explosion maximal pour un gain nul en production.
3. **Elle laisse la classe ouverte.** Le défaut n'est pas « RFC 5737 » : c'est
   « une fixture dont l'injoignabilité est une propriété du **réseau** », que
   tout proxy intercepte. Le prochain test qui sondera un autre hôte
   injoignable retombera dans le même trou, sous un autre littéral.

### Piste 2 — « le test skip sous `MIKA_PILOT_CONTAINED` »

Écartée sur trois motifs également.

1. **Elle désarme le détecteur exactement sur le chemin qui édite ce code.**
   Le pilote est l'auteur des modifications de `cadence.rs` ; un test qui se
   tait chez lui ne protège plus rien de ce qu'il écrit. C'est l'option (b) de
   mika#1574 (« livrer désarmé »), légitime en dernier recours et évitable ici.
2. **La variable est un proxy pour la vraie condition.** Ce qui casse le test
   est « un proxy HTTP est configuré », pas « je suis contenu ». Un poste de
   développement portant `HTTP_PROXY` (proxy d'entreprise, mitmproxy de
   débogage) resterait rouge, et le défaut serait déclaré fermé.
3. **La fixture peut simplement cesser de dépendre de l'environnement**, ce qui
   est strictement meilleur que de s'en abstraire par un saut.

## Le correctif : exprimer l'injoignabilité comme une propriété LOCALE

L'intention du test est écrite dans le doc-comment de la fonction : *« `None`
when the URL is unset or the request errors out (fail-open) »*. `192.0.2.1:1`
exprimait « la requête échoue » par *une adresse que le réseau ne sait pas
router* — une propriété du réseau. La remplacer par **un port de boucle locale
que rien n'écoute** l'exprime par *une socket sur laquelle personne n'écoute* —
une propriété de la machine, qu'aucun proxy n'intercepte (la boucle locale est
exclue du proxying par `NO_PROXY=localhost,127.0.0.1` dans le bac à sable, et
par l'absence de tout proxy sur le vrai CI).

Le motif est le classique « bind puis drop » : le noyau attribue un port libre,
on le libère, la connexion suivante rend `ECONNREFUSED`.

```rust
#[tokio::test]
async fn probe_executor_health_returns_none_on_unreachable_endpoint() {
    // mika#2495 — l'injoignabilité doit être LOCALE, jamais réseau.
    //
    // La fixture précédente sondait `192.0.2.1:1` (RFC 5737) et n'a jamais
    // testé ce qu'elle annonçait dans un bac à sable pilote : `HTTP_PROXY`
    // y est posé, `192.0.2.1` n'est pas dans `NO_PROXY`, donc reqwest
    // envoyait `GET http://192.0.2.1:1/health` au relais d'egress, qui
    // répond 400 à une forme absolue — d'où `Some(false)` au lieu de `None`,
    // en 0,03 s. L'adresse n'était ni résolue ni composée.
    //
    // Un port de boucle locale que rien n'écoute rend `ECONNREFUSED` sous
    // proxy comme sans, parce que la boucle locale n'est jamais proxifiée.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("la boucle locale doit être bindable");
    let port = listener
        .local_addr()
        .expect("le port attribué doit être lisible")
        .port();
    drop(listener);

    let url = format!("http://127.0.0.1:{port}/health");
    let res = probe_executor_health(Some(&url)).await;
    assert_eq!(
        res, None,
        "une connexion refusée doit fail-open à None (url={url})"
    );
}
```

**Renommé** `…_on_bogus_host` → `…_on_unreachable_endpoint` : « bogus host »
décrivait l'ancienne prémisse (un hôte fantaisiste) ; ce qui est asserté est un
point de terminaison injoignable.

### Prémisse validée, pas supposée

Cette forme a été exécutée **dans le bac à sable de ce grooming**, donc dans
l'environnement qui porte `HTTP_PROXY` :

```
test milestone_manager::cadence::tests::probe_executor_health_returns_none_on_unset_url ... ok
test milestone_manager::cadence::tests::probe_executor_health_returns_none_on_bogus_host ... ok
test result: ok. 2 passed; 0 failed; …
```

C'est une mesure plus forte qu'un raisonnement sur `NO_PROXY` : la fixture passe
*à l'intérieur* de l'environnement qui casse l'ancienne. La sonde a été révoquée
avant commit ; l'arbre ne porte que ce plan.

### Résidu nommé

Un environnement posant `HTTP_PROXY` **sans** exclure la boucle locale de
`NO_PROXY` reproduirait le défaut. Ce n'est pas la forme du bac à sable (qui
l'exclut, et doit l'exclure : `ANTHROPIC_BASE_URL` et le shim TCP→unix vivent
tous deux sur `127.0.0.1`) ni celle du CI (aucun proxy). La fermer
complètement exigerait d'injecter le transport dans `probe_executor_health` —
un refactor de production, hors du périmètre atomique de ce ticket, et nommé
ci-dessous comme suivi conditionnel.

## La garde : la classe ne doit pas revenir sous un autre littéral

Le défaut a coûté une session QA (a0886164, 6,95 USD) et **il était invisible** :
vert sur le vrai CI, rouge uniquement en pilote. Aucun test comportemental ne
peut voir cette classe — une nouvelle fixture `203.0.113.x` serait verte partout
où on la regarde et rouge là où personne ne regarde. C'est la signature qui
justifie un scan de source.

`canonical_tokens::tests::mika2495_aucune_fixture_ne_sonde_une_plage_de_documentation`
— scan sur **tout** `crates/**/*.rs`, code de test inclus.

**La population est inversée par rapport à ses voisines**, et c'est délibéré :
les gardes existantes de ce module scannent la production et écartent les tests
(`production_sources`). Ici le motif fautif vit *dans* le code de test, donc le
scan doit l'inclure. Un littéral de plage de documentation n'a par ailleurs
aucun usage légitime en production (la production ne code jamais une IP en dur),
donc la population « toutes les sources Rust » est la bonne et ne crée pas de
faux positif de côté production.

Aiguilles : les trois plages IPv4 de la RFC 5737 (`192.0.2.`, `198.51.100.`,
`203.0.113.`) et le préfixe IPv6 de la RFC 3849 (`2001:db8`, `2001:0db8`).

Trois propriétés, chacune née d'un piège réel :

- **Les commentaires sont retirés avant la recherche.** Sans ça, le doc-comment
  du test corrigé ci-dessus — qui *nomme* `192.0.2.1` pour expliquer pourquoi on
  ne s'en sert plus — ferait rougir la garde. C'est exactement le faux positif
  que la première exécution de la garde de mika#2323 a rapporté, et que
  `source_scan::fn_bodies` documente. L'implémentation extrait le prédicat de
  ligne déjà présent dans `fn_bodies` en un
  `source_scan::strip_comment_lines(src) -> String` que `fn_bodies` consomme
  ensuite — **un seul lecteur**, dans l'esprit écrit du module (« un seul
  lecteur, pas une copie par garde »), et non une seconde copie du prédicat.
  Une simple recherche par corps de fonction serait insuffisante : un
  `const BOGUS: &str = "192.0.2.1"` à portée de module échapperait à
  `fn_bodies`.
- **La garde ne s'accuse pas elle-même.** Ses aiguilles sont assemblées par
  `concat!` de fragments (`concat!("192.0", ".2.")`), donc son propre fichier ne
  porte aucun des motifs recherchés. Sans cela le scan rougirait à la première
  exécution, sur sa propre définition.
- **Contrôle de non-vacuité.** Un test compagnon
  (`…_le_scan_attrape_une_fixture_plantee`) passe une source synthétique
  portant le littéral et exige que le scan l'accuse. Sans lui, « la garde se
  tait » et « la garde ne regarde rien » se lisent pareil — la classe mika#2103
  que ce module nomme déjà (*« une garde qui ne vérifie rien est une
  décoration »*).

Le message d'échec nomme le remède : *remplacer la fixture par un port de boucle
locale fermé (`TcpListener::bind("127.0.0.1:0")` puis `drop`) ; un proxy
intercepte une adresse non routable et rend un 400, jamais une erreur de
transport.*

### Hors population, avec sa mesure

Les hôtes `.invalid` **ne sont pas dans l'aiguille**, et c'est un choix mesuré,
pas un oubli. Les cinq occurrences de l'arbre ont été relues une par une :

| site | usage | composé ? |
|---|---|---|
| `builtin_handlers.rs:5059` `http://gateway.invalid` | `ctx.gateway_url` d'un test qui assert `internal_token missing` | non — refus de validation avant tout appel |
| `tests/eval/test_mcp_secret_boundary_2281.rs:66` | chaîne sentinelle `db.invalid` | non |
| `tests/fixtures/mcp_secret_boundary_server.py:50` | valeur de variable publique | non |
| `scripts/test-smoke-search-substrate.sh` | URL de fixture shell | non |
| `test_sandbox_git_usable.sh` | adresse e-mail de committer | non |

**Aucune n'assert une injoignabilité ; aucune n'est composée.** Les ajouter à
l'aiguille produirait cinq faux positifs à exempter, c'est-à-dire une allowlist
née non vide — précisément la forme que la doctrine mika#2201 refuse (*« une
allowlist née vide est un emplacement où déposer la prochaine violation »*). La
classe reste ouverte pour un `.invalid` réellement composé ; le jour où une
mesure en montre un, l'aiguille s'étend d'une ligne.

**Ce relevé est une justification, jamais une couverture** — distinction à
garder explicite dans le corps de PR. La population du scan est `crates/**/*.rs`
et rien d'autre ; deux des cinq sites recensés (`test-smoke-search-substrate.sh`,
`test_sandbox_git_usable.sh`) sont des sources shell, donc hors de cette
population par construction. Ils ont été relus **plus loin que le scan ne
regarde** pour établir qu'aucun d'eux n'assert une injoignabilité, c'est-à-dire
pour justifier de **ne pas** élargir l'aiguille. Lire ce tableau comme une
promesse de couverture serait un contresens : le scan ne dit rien des sources
non-Rust, et ce plan ne prétend pas qu'il le fasse.

## Fire-Disposition

Ce plan livre deux détecteurs : le test réparé
`probe_executor_health_returns_none_on_unreachable_endpoint` et le scan de
source `mika2495_aucune_fixture_ne_sonde_une_plage_de_documentation`.

**Option retenue : (a) — exception nommée en allowlist, livrée VIDE.**

`ALLOWED_DOC_RANGE_FIXTURES: &[&str] = &[]`.

La population a été recensée exhaustivement sur tout l'arbre, à HEAD
`702558bc` :

```
$ grep -rnE --include="*.rs" "192\.0\.2\.|198\.51\.100\.|203\.0\.113\.|2001:db8|2001:0db8" crates
crates/mika-agent/src/milestone_manager/cadence.rs:1041: …"http://192.0.2.1:1/health"…
```

**Une seule occurrence, et c'est celle que ce plan corrige.** Le scan tire donc
**zéro fois** à la livraison, et l'allowlist n'a aucune entrée à porter — l'état
que la maison préfère explicitement.

**Assertion auto-nettoyante** :
`mika2495_l_allowlist_des_plages_de_documentation_est_livree_vide` refuse que
cette allowlist cesse d'être vide, sur le modèle textuel de ses quatre sœurs du
même module (`mika2242_the_sole_writer_allowlist_is_empty`,
`mika2484_l_allowlist_du_lecteur_unique_est_livree_vide`,
`mika2498_the_sole_writer_allowlist_is_empty`,
`mika2201_the_exhaustiveness_allowlist_is_empty`). Son message : *« quand le
scan tire, on RÉPARE la fixture, on ne l'exempte pas — une adresse de
documentation dans une fixture est verte sur le CI et rouge en pilote, ce qui
est la panne que mika#2495 a payée 6,95 USD. »*

Ni (b) ni (c) : (b) — livrer désarmé — est ce que la piste 2 du ticket propose
et que ce plan refuse avec sa raison ; (c) — halte-et-remontée — n'a pas lieu,
la population étant vide et le cadrage déjà tranché par l'umbrella #2491.

## Périmètre

### Fichiers touchés

| fichier | changement |
|---|---|
| `crates/mika-agent/src/milestone_manager/cadence.rs` | la fixture du test (≈ 12 lignes, doc-comment inclus) + son nom |
| `crates/mika-agent/src/source_scan.rs` | extraction de `strip_comment_lines`, consommée par `fn_bodies` — aucun changement de comportement |
| `crates/mika-agent/src/canonical_tokens.rs` | le scan, son allowlist vide, son contrôle de non-vacuité, son assertion auto-nettoyante |

### Ce que ce plan ne touche PAS

- **`scripts/mika-pilot-egress-proxy`** — zéro ligne. La frontière de
  containment n'apprend rien des fixtures de test (piste 1, écartée ci-dessus).
- **`skills/bundled/_shared/dispatch-lib.sh`** — zéro ligne. `HTTP_PROXY`,
  `NO_PROXY` et `MIKA_PILOT_CONTAINED` sont inchangés.
- **`probe_executor_health`** — zéro ligne de production. Son contrat
  fail-open, son `timeout(5s)` et sa construction de client sont inchangés ;
  seule sa fixture de test bouge.
- **Aucune variable d'environnement** n'est créée, aucun réglage n'est déplacé.
- **Aucun job CI** n'est ajouté : le scan est un `#[test]`, donc `cargo test`
  le porte déjà.

## Item 0 — ratification opérateur du corps du ticket (AVANT implémentation)

**Ce plan écarte les deux pistes que le corps de #2495 propose et renomme le test
que sa DoD nomme.** La substance de ce refus est mesurée (§ *Le mécanisme*,
§ *Ce que le ticket propose*) — mais **le corps du ticket est le contrat
versionné, et l'opérateur seul ratifie une divergence.** L'en-tête de #2495 le
dit lui-même : *« Ne PAS mettre `ready` sans ratification opérateur »*. Le
correctif ne doit donc pas partir sur un corps qui prescrit encore deux voies
refusées et un nom de test qui n'existera plus.

Convention appliquée : mika#2169 / mika#2158 — un plan qui réfute son ticket
fournit l'encadré de rectification **rédigé et daté**, plus le texte du
commentaire d'avis d'édition. Le geste est opérateur ; ce plan ne l'exécute pas,
il le rend exécutable sans rédaction supplémentaire.

Les deux blocs ci-dessous sont **du texte à appliquer tel quel** dans le corps de
#2495. Ils ne touchent aucune autre section : ni l'en-tête, ni le contexte, ni
le lien vers l'umbrella #2491.

> **Note de provenance.** Les citations verbatim du corps (« Fix proposé », la
> ligne DoD, la clause d'en-tête) proviennent de la relecture `gh_read` de la
> première passe architecte, `gh` n'étant pas authentifié dans le bac à sable de
> révision. L'opérateur qui applique R1/R2 relit le corps courant avant de
> remplacer : si une section a bougé depuis, c'est le remplacement qu'il faut
> ajuster, jamais la voie retenue.

### R1 — remplacement de la section « Fix proposé »

La section actuelle propose : *« Le proxy d'egress ne doit pas répondre aux
adresses RFC 5737/3849 ; OU le test skip sous `MIKA_PILOT_CONTAINED` »*. Elle est
remplacée en entier par :

```markdown
## Fix proposé (rectifié le 2026-09-23 — grooming mika#2495)

**Aucune des deux pistes initialement proposées n'est retenue**, et le motif du
refus de la première est factuel : le grooming a reproduit le défaut dans le bac
à sable pilote et mesuré une cause différente de celle que ce corps supposait.
L'adresse `192.0.2.1` n'est **jamais résolue ni composée**. `reqwest` détecte
`HTTP_PROXY` par défaut ; `192.0.2.1` n'est pas dans le `NO_PROXY` du bac à
sable ; la requête part donc au relais d'egress en forme absolue
(`GET http://192.0.2.1:1/health HTTP/1.1`), forme que le relais ne sert pas — il
rend `400 Bad Request`, d'où `Some(false)` au lieu de `None`, en 0,03 s.

Pistes écartées :

- *« Le proxy d'egress ne doit pas répondre aux adresses RFC 5737/3849 »* — le
  relais ne répond pas à une adresse : il répond `400` à une requête de proxy
  malformée, sans jamais regarder l'hôte. Il n'y a pas de branche à corriger, il
  faudrait en créer une. Elle ferait de surcroît entrer la connaissance des
  fixtures de test dans la frontière de containment, fail-closed depuis
  mika#2049 — rayon d'explosion maximal pour un gain nul en production. Et elle
  laisserait la classe ouverte : le défaut n'est pas « RFC 5737 », c'est « une
  fixture dont l'injoignabilité est une propriété du réseau », que tout proxy
  intercepte sous n'importe quel littéral.
- *« Le test skip sous `MIKA_PILOT_CONTAINED` »* — cela désarme le détecteur
  exactement sur le chemin qui édite ce code, le pilote étant l'auteur des
  modifications de `cadence.rs`. La variable est par ailleurs un proxy pour la
  vraie condition, qui est « un proxy HTTP est configuré » : un poste de
  développement portant `HTTP_PROXY` resterait rouge pendant que le défaut serait
  déclaré fermé.

**Voie retenue — exprimer l'injoignabilité comme une propriété LOCALE.** La
fixture bind `127.0.0.1:0`, lit le port que le noyau lui attribue, libère la
socket, puis sonde ce port fermé : la connexion rend `ECONNREFUSED` sous proxy
comme sans, la boucle locale n'étant jamais proxifiée. La forme a été exécutée
verte *à l'intérieur* du bac à sable qui casse l'ancienne fixture. Un scan de
source refuse en outre le retour d'un littéral de plage de documentation
(RFC 5737 / RFC 3849) dans `crates/**/*.rs`, avec une allowlist livrée vide.

Aucune ligne du relais d'egress, de `dispatch-lib.sh` ou du corps de
`probe_executor_health` n'est touchée.
```

### R2 — remplacement de la ligne DoD

La ligne actuelle nomme `probe_executor_health_returns_none_on_bogus_host` et
demande « vert dans le bwrap (skip ou proxy) ». Elle est remplacée par :

```markdown
**DoD** : `probe_executor_health_returns_none_on_unreachable_endpoint` passe vert
dans le bwrap du pilote **et** sur le CI réel — sans saut conditionnel et sans
modification du relais d'egress. Le test est renommé : `…_on_bogus_host`
décrivait la prémisse écartée (un hôte fantaisiste), alors que ce qui est asserté
est un point de terminaison injoignable.
```

**Le renommage devient ainsi ratifié plutôt que silencieux.** Sans R2, un `grep`
du nom que la DoD porte ne trouverait plus rien après merge.

### D1 — commentaire d'avis d'édition à poster sur #2495

À poster **après** application de R1 et R2, pour que l'édition du corps laisse
une trace datée et attribuée plutôt qu'une réécriture muette :

```markdown
Avis d'édition du corps — grooming mika#2495, 2026-09-23.

Le corps de ce ticket vient d'être amendé sur deux points : la section « Fix
proposé » (remplacée) et la ligne DoD (remplacée). Aucune autre section n'est
touchée.

Motif : le grooming a reproduit le défaut dans le bac à sable pilote et mesuré
une cause différente de celle que le corps supposait. L'adresse RFC 5737 n'est
jamais résolue ni composée — `reqwest` route la requête vers le relais d'egress
via `HTTP_PROXY`, et le relais rend `400` sur la forme absolue, d'où
`Some(false)` en 0,03 s. Les deux pistes proposées sont donc écartées : la
première sur un défaut de prémisse (le relais ne regarde jamais l'hôte) doublé
d'un rayon d'explosion sur une frontière de containment fail-closed (mika#2049) ;
la seconde parce qu'elle désarme le détecteur sur le chemin même qui édite ce
code, et parce que `MIKA_PILOT_CONTAINED` est un proxy pour la vraie condition
(« un proxy HTTP est configuré »).

Voie retenue : la fixture exprime son injoignabilité localement — bind sur
`127.0.0.1:0` puis drop, ce qui rend `ECONNREFUSED` sous proxy comme sans. Le
test est renommé `probe_executor_health_returns_none_on_unreachable_endpoint`,
d'où la rectification de la ligne DoD : le nom qu'elle porte doit être celui qui
existera après le correctif.

Le raisonnement complet, les motifs de refus, la garde de non-retour et son
allowlist vide sont dans le plan committé sur la branche de ce ticket.
```

### Halte

Si l'opérateur refuse la voie retenue, **le correctif ne part pas** : ni le
renommage ni le scan de source ne sont exécutables sous un corps qui prescrit
encore une des deux pistes écartées. Le geste est alors de rouvrir le grooming,
pas d'implémenter contre le contrat.

## Definition of Done

- **Item 0 — le corps de #2495 est rectifié par l'opérateur** (R1 + R2 appliqués,
  D1 posté en commentaire) **avant toute ligne de code.** Le ticket ne porte plus
  aucune prescription des deux pistes écartées, et sa DoD nomme le test tel qu'il
  existera.
- `probe_executor_health_returns_none_on_unreachable_endpoint` passe **dans le
  bwrap du pilote** (mesuré, pas supposé) et **sur le CI réel** de la PR.
- Aucune adresse de plage de documentation (RFC 5737 / RFC 3849) ne subsiste
  dans `crates/**/*.rs`, hors les mentions en commentaire qui expliquent le
  défaut.
- Le scan de source refuse le retour de la classe, avec une allowlist vide et
  un contrôle de non-vacuité vu vert **et** vu rouge.
- `cargo test -p mika-agent`, `cargo clippy` et `cargo fmt --check` propres.
- Aucun fichier hors des trois listés n'est modifié.

## Acceptance criteria

Le corps du ticket porte une DoD d'une ligne et pas de section
`## Acceptance criteria` ; les critères ci-dessous en sont dérivés, et AC1 est
la DoD verbatim — **après** la rectification que AC0 exige, puisque ce plan
écarte les deux voies que cette DoD suppose.

0. **AC0 — la divergence est ratifiée, pas subie.** Le corps de #2495 porte les
   remplacements R1 et R2 du § *Item 0*, et le commentaire D1 y est posté.
   Vérification : lecture du corps et de son fil de commentaires. **Bloquant
   pour le démarrage** : aucune ligne de code ne part sous un corps prescrivant
   encore le patch du relais ou le saut sous `MIKA_PILOT_CONTAINED`.
1. **AC1 — vert des deux côtés.**
   `probe_executor_health_returns_none_on_unreachable_endpoint` passe dans le
   bac à sable pilote (`--unshare-net` + `HTTP_PROXY` posé) et sur le CI réel.
   Vérification : l'exécution dans le worktree de dispatch **et** le job CI de
   la PR, les deux cités dans le corps de PR.
2. **AC2 — la fixture ne dépend plus du réseau.** L'URL sondée est une adresse
   de boucle locale dont le port a été attribué puis libéré dans le test
   lui-même. Aucun littéral d'adresse routable ni non routable n'y subsiste.
3. **AC3 — l'attribution est écrite au site.** Le doc-comment du test nomme le
   mécanisme (proxy → forme absolue → `400` → `Some(false)`) et la mesure
   (0,03 s), pour qu'un futur éditeur qui trouverait le `bind`/`drop` inutilement
   tortueux ne remette pas une adresse non routable.
4. **AC4 — la classe est refusée structurellement.** Un littéral de plage de
   documentation ajouté à n'importe quel `crates/**/*.rs` (hors commentaire)
   fait rougir le scan. Vérifié par le contrôle de non-vacuité, **vu rouge** en
   plantant le littéral dans une source réelle avant de le retirer.
5. **AC5 — la garde ne rougit pas sur du code sain.** Le doc-comment de AC3, les
   cinq sites `.invalid` recensés et la définition de la garde elle-même ne sont
   pas accusés. Vérifié par une exécution du scan sur l'arbre après correction.
6. **AC6 — l'allowlist naît vide et le reste.** Une assertion dédiée échoue si
   elle cesse de l'être, et son message prescrit la réparation de la fixture
   plutôt que l'exemption.
7. **AC7 — aucun changement de production ni de containment.** `git diff --stat`
   ne montre que les trois fichiers du § Périmètre ; ni le relais d'egress ni
   `dispatch-lib.sh` ni le corps de `probe_executor_health` ne bougent.

## Contrat de vérification

| # | geste | attendu | halte |
|---|---|---|---|
| V1 | `cargo test -p mika-agent --lib probe_executor_health` **dans le worktree de dispatch** | 2 passed, 0 failed | si `Some(false)` persiste, la boucle locale est proxifiée : lire `NO_PROXY` du bac à sable **avant** de toucher la fixture |
| V2 | `cargo test -p mika-agent --lib mika2495` | scan + non-vacuité + allowlist vide, verts | — |
| V3 | planter `"203.0.113.9"` dans un `crates/**/*.rs`, relancer V2 | **rouge**, message nommant le fichier et le remède | si vert : le scan ne regarde pas cette population — réparer la classification, ne pas élargir l'aiguille |
| V4 | retirer le littéral planté, relancer V2 | vert | — |
| V5 | `cargo test -p mika-agent` complet | aucune régression, `source_scan::tests` inclus | si `fn_bodies` régresse, l'extraction de `strip_comment_lines` a changé le prédicat de ligne — c'est un refactor à somme nulle, pas un ajustement |
| V6 | CI de la PR | vert | si rouge uniquement sur le CI, la prémisse « pas de proxy sur le CI » est fausse — halte, ne pas ajouter de saut conditionnel |

**Halte générale.** Si le test reste rouge en pilote après le correctif, ne pas
reprendre la piste 2 par réflexe : relire le mécanisme du § *Le mécanisme*, puis
établir **quelle** variable d'environnement du bac à sable a changé. Le
correctif repose sur une seule propriété — la boucle locale n'est pas
proxifiée — et c'est celle-là qu'il faut mesurer avant toute autre conclusion.

## Ce que ce travail n'achète pas

- **Aucun compteur, aucun événement de journal, aucune ligne d'`audit_events`.**
  Le défaut est un test rouge : son signal est son propre rouge.
- **Il ne rend pas `probe_executor_health` testable sans socket.** Le résidu du
  § *Résidu nommé* (un `HTTP_PROXY` sans exclusion de boucle locale) reste
  ouvert. **Suivi conditionnel** : injecter le transport dans
  `probe_executor_health`, sur le modèle du `GhRunner` que ce même module porte
  déjà pour `gh`. **Précondition** : qu'une mesure montre un environnement réel
  posant `HTTP_PROXY` sans `NO_PROXY` couvrant la boucle locale. Ouvrir ce
  ticket avant cette mesure serait instruire sans mesurer.
- **Il ne dit rien des autres tests réseau de la suite.** Le recensement
  ci-dessus porte sur les plages de documentation et sur `.invalid` ; un test
  composant un hôte public réel serait une troisième classe, non mesurée ici.
- **Il ne ferme pas les autres défauts de l'umbrella #2491.** Un seul, le n° 3.

## Références

- #2491 (Défaut 3), parent umbrella.
- Session QA a0886164 (#2487) — le coût mesuré du défaut, 6,95 USD.
- mika#2049 — posture fail-closed du relais d'egress, qui interdit la piste 1.
- mika#1574 — les trois options de Fire-Disposition.
- mika#2201 — « on déclare, on n'allowliste pas ».
- mika#2103 — une garde qui ne vérifie rien est une décoration.
- mika#2321 / `crate::source_scan` — le lecteur unique de la classification de
  source et du découpage par fonction.
- mika#2323 — le faux positif d'une garde accusant sa propre prose.
- mika#2169 / mika#2158 — convention « un plan qui réfute son ticket fournit
  l'encadré de rectification rédigé et daté, plus l'avis d'édition ». Appliquée
  au § *Item 0*.
- `docs/architecture/review-guide.md` § spec-fidelity — le corps du ticket est le
  contrat versionné ; l'opérateur ratifie la divergence, pas l'architecte.

## Revision history

- **rev 2 (2026-09-23)** — première passe architecte, `Disposition: ITERATE`,
  une finding bloquante et deux de sharpening.
  - **F1 (bloquante) — plan réfutant le ticket sans encadré de rectification
    rédigé.** Adressée par une section `## Item 0 — ratification opérateur du
    corps du ticket`, placée **avant** la DoD, portant les trois artefacts que la
    finding exige : (R1) le texte de remplacement complet de la section « Fix
    proposé », nommant les deux pistes écartées avec leurs motifs et la voie
    retenue ; (R2) le texte de remplacement de la ligne DoD, qui **ratifie le
    renommage** `…_on_bogus_host` → `…_on_unreachable_endpoint` au lieu de le
    laisser silencieux ; (D1) le texte du commentaire d'avis d'édition à poster
    sur #2495. La clause d'en-tête du ticket (*« Ne PAS mettre `ready` sans
    ratification opérateur »*) est désormais citée, et la convention mika#2169 /
    mika#2158 nommée avec sa citation review-guide § spec-fidelity. La DoD gagne
    un item 0 et les AC un AC0, tous deux bloquants pour le démarrage ; une
    halte explicite dit ce qui se passe si l'opérateur refuse la voie retenue
    (rouvrir le grooming, ne pas implémenter contre le contrat). Une note de
    provenance dit d'où viennent les citations verbatim du corps, `gh` n'étant
    pas authentifié dans le bac à sable de révision.
  - **S1 (sharpening) — distinction population / justification sur le relevé
    `.invalid`.** Adressée dans le § *Hors population, avec sa mesure* : la
    population du scan est `crates/**/*.rs` et rien d'autre ; le relevé des cinq
    sites lit délibérément **plus loin que le scan ne regarde**, pour justifier
    de ne pas élargir l'aiguille. Deux des cinq sont des sources shell, donc hors
    population par construction. La phrase nomme explicitement le contresens à
    éviter dans le corps de PR.
  - **S2 (sharpening) — le renommage laissait la DoD du ticket littéralement
    inassouvie.** Adressée par R2 ci-dessus, qui fait du renommage un acte
    ratifié par l'opérateur dans le corps, et non un écart découvert après merge
    par un `grep` qui ne trouve plus rien.
  - Aucune AC n'a été affaiblie ; AC0 en ajoute une, bloquante.
