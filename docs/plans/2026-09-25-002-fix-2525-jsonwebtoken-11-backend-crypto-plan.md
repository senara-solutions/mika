# mika#2525 — Le backend crypto de `jsonwebtoken` 11 est déclaré, et la signature du JWT App est vérifiée

**Ticket :** senara-solutions/mika#2525 (p1, deps + auth)
**Dependabot :** senara-solutions/mika#2454 (`jsonwebtoken` 9.3.1 → 11.1.0)
**Branche :** `feat/2525/jsonwebtoken-11-test-generate-jwt-claims`

---

## 0. Ce que la lecture du code déplace dans le ticket

Le ticket est juste sur le **symptôme** (panique runtime, build vert, test rouge)
et juste sur la **préférence** (`aws_lc_rs`, cohérent avec le rail rustls). Quatre
faits lus dans la source de `jsonwebtoken-11.1.0` — présente dans le registre local
(`~/.cargo/registry/src/index.crates.io-*/jsonwebtoken-11.1.0/`) — corrigent sa
**cause** et, surtout, **la lettre de son AC1**.

### R1 — Il n'y a pas « deux backends qui coexistent ». Il y en a ZÉRO.

Le ticket écrit : *« Le build compile (les deux features par défaut coexistent) »*.
Le `Cargo.toml` publié du crate dit l'inverse :

```toml
[features]
aws_lc_rs  = ["dep:aws-lc-rs"]
default    = ["use_pem"]                      # ← aucune feature crypto
rust_crypto = ["dep:ed25519-dalek", "dep:hmac", "dep:p256",
               "dep:p384", "dep:rand", "dep:rsa", "dep:sha2"]
use_pem    = ["dep:pem", "dep:simple_asn1"]
```

Et le site de panique (`src/crypto/mod.rs:104-131`) est un `cfg` en **cascade
exclusive**, dont le troisième bras est le fallback :

```rust
fn from_crate_features() -> &'static Self {
    #[cfg(all(feature = "rust_crypto", not(feature = "aws_lc_rs")))]
    { return &rust_crypto::DEFAULT_PROVIDER; }

    #[cfg(all(feature = "aws_lc_rs", not(feature = "rust_crypto")))]
    { return &aws_lc::DEFAULT_PROVIDER; }

    #[allow(unreachable_code)]
    {   // signer_factory: |_, _| panic!("{}", NOT_INSTALLED_ERROR)
        &INSTANCE                                  // ← où mika atterrit
    }
}
```

Le bras atteint l'est parce que **ni** `rust_crypto` **ni** `aws_lc_rs` n'est
activée — pas parce que les deux le sont. La différence n'est pas cosmétique :
elle décide du remède. Sous la lecture « deux backends », on en **retire** un
(`default-features = false`) ; sous la lecture mesurée, on en **ajoute** un.
C'est l'inverse, et R2 dit ce que la première coûte.

### R2 — La lettre de l'AC1 (`default-features = false`) CASSE la compilation, production comprise

`default = ["use_pem"]`, et `EncodingKey::from_rsa_pem` est derrière
`#[cfg(feature = "use_pem")]` (`src/encoding.rs:59`). Poser
`default-features = false` retire `use_pem` et fait disparaître la fonction. Cinq
sites l'appellent, dont **trois en production** :

| site | nature |
|---|---|
| `crates/mika-common/src/github_app.rs:95` | **production** — `GitHubApp::from_settings` |
| `crates/mika-common/src/github_app.rs:131` | **production** — construction depuis la clé configurée |
| `crates/mika-cli/src/commands/doctor.rs:536` | **production** — `mika doctor`, validation de la clé App |
| `crates/mika-common/src/github_app.rs:207` | `cfg(any(test, feature = "test-utils"))`, exposé aux crates aval |
| `github_app.rs:573`, `:712`, `mika-gateway/src/github.rs:4246` | tests |

La forme correcte est donc **`features = ["aws_lc_rs"]` SANS toucher aux
defaults** — ajouter le backend, conserver `use_pem`. L'**intention** de l'AC1
(« activer exactement une feature, `aws_lc_rs` de préférence ») est satisfaite à
la lettre près ; c'est le `default-features = false` qui est refusé, et le refus
est mesuré, pas stylistique. **Sans cette rectification, un implémenteur suivant
l'AC1 au mot échange une panique runtime contre une erreur de compilation à trois
sites de production.**

### R3 — `aws_lc_rs` coûte ZÉRO nouvelle dépendance ; `rust_crypto` en ajouterait trois

`aws-lc-rs` v1.18.0 est **déjà compilé dans `mika-common`**, par
`reqwest` → `hyper-rustls` → `rustls` (`cargo tree -i aws-lc-rs`, mesuré sur ce
worktree) — et aussi via `sqlx` `tls-rustls-aws-lc-rs` côté gateway. Sa toolchain
native est donc déjà satisfaite partout où mika se construit, images comprises
(`Dockerfile.agent` : `rust:1.93-bookworm` + `gcc libc-dev pkg-config` ;
`Dockerfile.gateway` : `rust:1.93-bookworm`) — un build actuel qui réussit *est*
la preuve que `aws-lc-sys` compile déjà dans ces images.

Le décompte des sept crates de `rust_crypto` contre `Cargo.lock` :

| crate | déjà au lock ? |
|---|---|
| `rsa`, `hmac` | oui (1 version) |
| `sha2` | oui (2 versions) |
| `ed25519-dalek`, `p256`, `p384` | **non — trois nouveaux** |

La préférence du ticket est donc **confirmée par la mesure**, et non seulement par
la cohérence de style : `aws_lc_rs` n'élargit pas la surface de dépendances, et
`RS256` — le seul algorithme que mika signe (`Header::new(Algorithm::RS256)`,
`github_app.rs:383`) — est servi par ce provider
(`src/crypto/aws_lc/mod.rs:89`, `:111`).

### R4 — La branche Dependabot est un bump nu

`git diff main...origin/dependabot/cargo/jsonwebtoken-11.1.0 -- Cargo.toml` rend
exactement une ligne : `jsonwebtoken = "9"` → `jsonwebtoken = "11"`. Aucune feature
n'est touchée. Le correctif tient donc dans **cette même ligne**, complétée.

### R5 — Le test que l'AC2 nomme n'atteste pas ce que l'AC3 demande

`test_generate_jwt_claims` (`github_app.rs:571`) décode le **payload** base64 et
vérifie `iss`, `iat`, `exp`. Il ne touche jamais la signature. Une fois le backend
déclaré, il redevient vert **sans rien dire de la validité cryptographique du
token** — il atteste qu'un provider existe, pas qu'il signe correctement. L'AC3
(« produit un token valide ») demande donc un livrable de plus, et c'est le §4.

### Ce que le MSRV ne bloque pas

`jsonwebtoken` 11.1.0 exige `rust-version = 1.88.0` et l'édition 2024 ; le
workspace pose `rust-version = "1.91"` et `edition = "2024"`, les images
construisent sur `rust:1.93`. Aucun obstacle.

---

## 1. Requirements

- **R-A** — Le backend crypto de `jsonwebtoken` 11 est déclaré **exactement une
  fois** dans la déclaration workspace, sans retirer `use_pem`.
- **R-B** — `cargo test` est vert sur l'ensemble du workspace, y compris
  `github_app::tests::test_generate_jwt_claims`, sans pin en arrière ni
  `#[ignore]` (AC2, AC4).
- **R-C** — La signature du JWT produit est **vérifiée cryptographiquement** par un
  test déterministe et hors réseau (AC3, moitié testable).
- **R-D** — La moitié d'AC3 qui exige des credentials réels est **nommée comme
  sonde opérateur**, jamais revendiquée comme livrée par un test.
- **R-E** — Le choix `aws_lc_rs` est **documenté au site de la déclaration**, avec
  sa raison, pour qu'un futur éditeur ne le retire pas en croyant nettoyer.

---

## 2. Conception — la déclaration

Un seul site, `Cargo.toml` du workspace, ligne 134 :

```toml
# JWT — GitHub App RS256 signing (crates/mika-common/src/github_app.rs).
#
# `features = ["aws_lc_rs"]` est LOAD-BEARING, pas cosmétique (mika#2525).
# jsonwebtoken 11 a retiré `ring` pour une architecture à provider enfichable :
# son `default` ne porte QUE `use_pem`, donc aucun backend crypto n'est actif
# par défaut et tout appel à `encode`/`decode` PANIQUE à l'exécution — un build
# parfaitement vert. Exactement une des deux features `rust_crypto`/`aws_lc_rs`
# doit être active : deux en activent zéro (le `cfg` est exclusif).
#
# `aws_lc_rs` plutôt que `rust_crypto` : aws-lc-rs est déjà compilé dans l'arbre
# via reqwest -> hyper-rustls -> rustls, donc coût zéro ; `rust_crypto`
# ajouterait ed25519-dalek, p256 et p384.
#
# NE PAS poser `default-features = false` : ça retire `use_pem`, donc
# `EncodingKey::from_rsa_pem`, appelé à trois sites de PRODUCTION
# (github_app.rs:95, :131, mika-cli doctor.rs:536).
jsonwebtoken = { version = "11", features = ["aws_lc_rs"] }
```

Trois propriétés de cette forme, chacune choisie contre une alternative :

- **Ajout plutôt que substitution.** `default-features = false` +
  `features = ["use_pem", "aws_lc_rs"]` produirait aujourd'hui le même ensemble de
  features, et serait **plus fragile** : il fige une liste qui devrait suivre le
  `default` amont, donc il se casse en silence le jour où le crate ajoute une
  feature par défaut. On déclare ce qu'on ajoute, on ne recopie pas ce qu'on garde.
- **Site unique.** Les trois crates consommatrices (`mika-common`,
  `mika-cli`, `mika-gateway`) héritent par `workspace = true` ; aucune ne déclare de
  features locales. Une feature posée dans une seule crate serait **unifiée** par
  Cargo pour tout le workspace de toute façon, mais invisiblement — la déclaration
  workspace est le seul endroit où elle se lit.
- **Aucun `CryptoProvider::install_default()`.** C'est la seconde branche offerte
  par l'AC1 et elle est **refusée**, avec sa raison : elle exigerait un point
  d'entrée unique où l'appeler, or mika en a quatre (`mika-spirit`, `mika`,
  `mika-gateway`, plus chaque binaire de test) — et le chemin de test est
  précisément celui qui n'a pas de `main`. Un `install_default()` oublié dans un
  seul harnais rend la panique, à l'identique. La feature est résolue **à la
  compilation**, pour tout binaire du workspace, sans site d'appel à ne pas
  oublier. *Un invariant tenu par le système de features n'a pas de site
  d'initialisation à rater.*

`Cargo.lock` suit le bump (`jsonwebtoken` 11.1.0 + activation de `aws-lc-rs` comme
dépendance directe). Il est régénéré par `cargo build`, jamais édité à la main.

**Rapport à #2454.** Ce ticket porte le bump **et** sa configuration dans une seule
PR : la ligne `Cargo.toml` est la même. #2454 devient redondant et se ferme au
merge de celle-ci (AC4 : aucun pin arrière, aucune exclusion). Si l'ordre imposait
l'inverse, le correctif serait à appliquer **sur** la branche Dependabot — mais un
merge de #2454 seul laisserait `main` rouge, ce que l'AC4 interdit.

---

## 3. Conception — ce que le test existant redevient

`test_generate_jwt_claims` repasse vert **sans être modifié**. C'est voulu :
il est la preuve de non-régression de la classe, et le toucher dans la même PR
rendrait son retour au vert inattribuable. Il couvre déjà, à lui seul, les trois
manières de rouvrir le défaut, parce que les trois finissent au même `panic!` :

| régression | ce que `from_crate_features` fait |
|---|---|
| la feature est retirée | zéro bras actif → fallback panic |
| `rust_crypto` est ajoutée **en plus** | les deux `cfg` exclusifs échouent → fallback panic |
| une crate tierce active `rust_crypto` (unification Cargo) | idem |

**C'est pourquoi ce plan ne livre aucun scan de source sur `Cargo.toml`.** Une
garde structurelle qui vérifierait « exactement une feature crypto déclarée »
serait strictement redondante avec un test qui panique déjà sur les trois cas —
et la règle de grooming l'interdit explicitement (*« ne l'invente pas »*). La
seule régression qu'un scan verrait et que le test ne voit pas est
`default-features = false`, qui **ne compile pas** : elle est attrapée plus tôt
et plus fort.

---

## 4. Conception — AC3, et ce qui n'est pas testable

### La moitié testable : vérifier la signature, pas seulement la présence d'une signature

Nouveau test `mika2525_le_jwt_produit_porte_une_signature_rs256_verifiable`, dans
le même `mod tests` de `crates/mika-common/src/github_app.rs` :

1. `let jwt = app.generate_jwt().unwrap();`
2. `let key = DecodingKey::from_rsa_pem(TEST_RSA_PEM_PUBLIC.as_bytes()).unwrap();`
3. `let mut v = Validation::new(Algorithm::RS256); v.validate_exp = true;`
4. `decode::<JwtClaims>(&jwt, &key, &v).expect(…)` puis assertion sur `claims.iss`.

Ce test exerce le **chemin de vérification** du provider (`verifier_factory`), que
le chemin de signature seul ne touche pas — donc il attrape aussi une déclaration
de feature qui signerait sans savoir vérifier. `Validation::new(RS256)` exige `exp`
et le valide ; le JWT porte `exp = iat + 540` avec `iat` rétrodaté de 60 s, donc il
est valide à l'instant du test sans horloge simulée.

**La clé publique est une nouvelle constante de test**, dérivée **une fois** de
`TEST_RSA_PEM` par l'implémenteur :

```bash
# depuis le worktree, sans jamais écrire la clé privée sur disque
openssl rsa -pubout   # PEM privé sur stdin, PEM public sur stdout
```

Trois contraintes sur cette constante, chacune à respecter :

- **Nom.** `TEST_RSA_PEM_PUBLIC` — il doit contenir la sous-chaîne `TEST_RSA_PEM`,
  parce que `scripts/check-secrets.sh:121` filtre par `grep -v 'TEST_RSA_PEM'`.
  *(Le filtre n'est en pratique pas sollicité : `SECRET_REGEX` vise
  `-----BEGIN (RSA |EC )?PRIVATE KEY` et un PEM public commence par
  `-----BEGIN PUBLIC KEY`. Le nom est choisi pour la cohérence et pour survivre à
  un durcissement futur du motif, pas parce que la garde tire aujourd'hui.)*
- **Portée.** `const` dans `mod tests` uniquement. Pas d'exposition `test-utils` :
  aucun crate aval n'en a besoin, et la `new_with_test_token` exposée (`:207`) ne
  vérifie rien.
- **Correspondance.** Elle doit être dérivée de la clé privée **de `mod tests`**
  (`github_app.rs:462`), et non de l'homonyme de `new_with_test_token`
  (`github_app.rs:179`) — les deux constantes portent le même nom dans deux portées
  distinctes. Le test rougit immédiatement en cas d'erreur (`InvalidSignature`),
  donc l'erreur est attrapée à l'écriture, pas plus tard.

**Repli si `openssl` est indisponible dans l'environnement d'implémentation :**
ajouter `rsa = "0.9"` en `[dev-dependencies]` de `mika-common` et dériver la
publique depuis la privée dans le test. Le repli est nommé pour éviter une halte,
mais il est **moins bon** (une dépendance de test contre une constante inerte) et
n'est à prendre que si la première voie est bloquée.

### La moitié NON testable, dite plutôt que revendiquée

L'AC3 demande *« l'auth App du pilote fonctionne réellement, pas seulement le
test »*. Aucun test de ce dépôt ne peut l'établir : `exchange_jwt_for_token`
(`github_app.rs:394`) appelle `api.github.com` et exige un App ID, une clé privée
et un installation ID **réels**. Le test ci-dessus prouve que le token est
cryptographiquement valide et bien formé ; il ne prouve pas que GitHub l'accepte.

Ces deux propositions ne sont pas la même, et les confondre serait exactement la
classe de faux-vert que ce ticket corrige — un build vert qui n'attestait rien de
l'exécution. La vérification bout-en-bout est donc posée comme **sonde opérateur**
au §8, avec sa halte.

---

## 5. Contrat de vérification

| # | Vérification | Commande | Attendu |
|---|---|---|---|
| V1 | Le bump compile | `cargo build --workspace` | succès ; `Cargo.lock` porte `jsonwebtoken 11.1.0` |
| V2 | **AC2** — le test fondateur | `cargo test -p mika-common github_app::tests::test_generate_jwt_claims` | vert |
| V3 | **AC2** — la suite complète de `mika-common` | `cargo test -p mika-common` | **640/640**, zéro `failed`, zéro `ignored` nouveau |
| V4 | **AC3** — la signature est vérifiable | `cargo test -p mika-common mika2525_` | vert |
| V5 | **Contrôle négatif de V4** — le test regarde quelque chose | altérer un octet de la signature avant `decode`, à la main, en local | `ErrorKind::InvalidSignature` ; **à voir rouge avant de committer** |
| V6 | Aucune régression ailleurs | `cargo test` | vert (job CI `Check`, step `Test`) |
| V7 | Le chemin telemetry aussi | `cargo test --workspace --features telemetry` | vert (job CI `Check`) |
| V8 | Lint et format | `cargo clippy --workspace --all-targets -- -D warnings` puis `cargo fmt --check` | vert |
| V9 | **R-A** — exactement une feature crypto | `grep -n 'jsonwebtoken' Cargo.toml` | `features = ["aws_lc_rs"]`, **pas** de `default-features = false`, **pas** de `rust_crypto` |
| V10 | Aucun pin arrière (**AC4**) | `grep -c 'jsonwebtoken = "9"' Cargo.toml` ; `git diff main -- Cargo.toml` | `0` ; le diff ne porte que la ligne `jsonwebtoken` |
| V11 | La clé de test n'est pas un secret | `bash scripts/check-secrets.sh` | vert |

**V5 est la vérification porteuse et elle n'est pas optionnelle.** Un test qui
`decode` sans jamais avoir été vu rougir sur une signature fausse ne distingue pas
« la signature est valide » de « `decode` ne vérifie rien ici » — et c'est très
exactement la forme de panne que ce ticket ferme, reproduite un cran plus loin.
L'altération est locale et transitoire ; elle n'est pas committée.

---

## 6. Fire-Disposition

**Détecteur livré :** un seul —
`mika2525_le_jwt_produit_porte_une_signature_rs256_verifiable` (§4). Le test
existant `test_generate_jwt_claims` n'est pas un livrable de ce plan : il est
préexistant et n'est pas modifié.

**Disposition retenue : (a) exception nommée en allowlist — allowlist livrée
VIDE.**

Détail d'implémentation et justification :

- **Aucune violation préexistante à excepter.** Le détecteur porte sur un seul
  objet — le JWT que `generate_jwt` produit sur la clé de test — et il n'existe
  qu'un producteur. Il n'y a pas de corpus à balayer, donc pas de population
  historique dont on hériterait. L'allowlist est vide parce qu'elle n'a rien à
  contenir, pas parce qu'on a renoncé à la remplir.
- **L'assertion auto-nettoyante est l'allowlist elle-même.** Étant vide et sans
  mécanisme d'exception, toute violation future **rougit** ; il n'existe aucune
  voie d'évitement à laisser pourrir. La résolution quand le test tire est de
  **réparer la déclaration de feature**, jamais d'ajouter une exception — et
  c'est ce que dit le commentaire load-bearing du §2, posé au seul endroit où un
  éditeur qui s'apprête à casser l'invariant regardera.
- **Ni (b) ni (c).** Le détecteur ne peut pas atterrir désarmé : il n'a de sens
  qu'avec le correctif du §2, dont il est la vérification, et un `#[ignore]` ici
  rendrait AC3 non livrée tout en la déclarant. Et il n'y a rien à remonter à
  l'opérateur : aucune violation existante ne demande d'arbitrage.

**État au moment de l'atterrissage :** vert. Le détecteur est écrit **après** le
correctif du §2 dans la même PR — il rougirait sur `main` d'avant (panique) et sur
la branche Dependabot nue (panique), ce qui est le comportement voulu et ce que la
sonde du §8 rejoue.

---

## 7. Documentation

Trois écritures, aucune hors du strict nécessaire :

1. **Le commentaire du §2, au site de la déclaration.** C'est la documentation
   principale, et son emplacement est le livrable : un futur éditeur qui simplifie
   `Cargo.toml` — ou un futur Dependabot dont la PR touche cette ligne — lit la
   raison **là où le geste se fait**, pas dans un fichier qu'il n'ouvrira pas.
   Répondre à l'AC1 (« décider et documenter le choix ») par une entrée de
   `CLAUDE.md` seule laisserait la ligne nue devant l'éditeur.
2. **Une entrée de `docs/solutions/`** —
   `docs/solutions/best-practices/un-bump-majeur-peut-etre-vert-a-la-compilation-et-panique-a-lexecution-2026-09-25.md`,
   frontmatter YAML (`module: deps`, `tags: [dependabot, crypto, jwt, github-app]`,
   `problem_type: silent-runtime-failure`). La leçon est transportable et dépasse
   ce crate : **un bump majeur qui déplace une décision du code vers les features
   de Cargo est invisible au compilateur**, et le seul détecteur est un test qui
   *exerce* le chemin. Elle nomme la classe (`rustls` et `jsonwebtoken` partagent
   ce motif de provider enfichable), le piège `default-features = false` qui a
   l'air d'une hygiène, et la lecture du `cfg` exclusif (deux features activées ⇒
   zéro backend).
3. **Pas d'entrée dans `mika/CLAUDE.md`.** Ce fichier documente des surfaces
   opérateur — variables d'environnement, signaux, sondes. Ce correctif n'en crée
   aucune : il n'ajoute ni variable, ni événement de journal, ni ligne
   `audit_events`. L'y écrire gonflerait un index déjà dense d'un fait qui vit
   correctement dans `Cargo.toml` et dans `docs/solutions/`.

---

## 8. Surfaces opérateur et sondes

**Ce travail ne crée aucune surface d'observabilité**, et c'est à dire plutôt qu'à
masquer : pas de compteur, pas d'événement de journal, pas de ligne
`audit_events`. Le défaut est une panique — elle est déjà bruyante, par
construction, et sa trace est le `panic!` lui-même. Inventer un signal pour une
panique serait ajouter une ligne que personne ne lira avant le crash.

Le seul instrument est donc le rejeu, et il a deux moitiés.

### Sonde S1 — le symptôme fondateur est éteint (CI, immédiat)

Le job `Check`, step `Test`, doit rendre `640 passed; 0 failed` sur `mika-common`.
C'est la vérification que #2454 attendait.

**Halte 1 — le test reste rouge sur une panique `CryptoProvider`.** Ne pas toucher
au test. Vérifier d'abord que la feature est bien **unifiée** jusqu'au binaire de
test : `cargo tree -p mika-common -f '{p} {f}' | grep jsonwebtoken` doit montrer
`aws_lc_rs` dans la liste de features. Si elle n'y est pas alors que `Cargo.toml`
la porte, la cause est une déclaration locale concurrente dans une crate
(`grep -rn 'jsonwebtoken' crates/*/Cargo.toml`), pas la déclaration workspace.

**Halte 2 — le test échoue en `InvalidSignature` et non en panique.** Le backend
est bien installé et c'est la clé publique de V4 qui ne correspond pas à la clé
privée (la confusion des deux `TEST_RSA_PEM` homonymes, §4). C'est un défaut du
test, pas du correctif : **ne pas retoucher `Cargo.toml`.**

### Sonde S2 — l'auth App fonctionne réellement (opérateur, post-déploiement)

C'est la moitié d'AC3 qu'aucun test ne livre (§4). Geste opérateur sur un hôte
portant des credentials App réels, **après** `make deploy` :

```bash
mika doctor                       # valide le parse de la clé (chemin from_rsa_pem)
grep manager_token_refreshed "$MIKA_SPIRIT_LOG_FILE" | tail
grep gh_app_token_exchange_failed "$MIKA_SPIRIT_LOG_FILE" | tail
```

| événement | régime attendu | lecture |
|---|---|---|
| `manager_token_refreshed` | **non vide** après un cycle | le JWT a été signé **et accepté par GitHub** — l'AC3 bout-en-bout |
| `gh_app_token_exchange_failed` | **vide** | toute occurrence est un échange refusé ; lire le code HTTP avant de conclure |

**Halte 3 — `manager_token_refreshed` est vide et `gh_app_token_exchange_failed`
l'est aussi.** On ne peut **rien** conclure : c'est le contrôle positif qui manque,
pas la preuve d'une panne. Établir d'abord que l'auth App est configurée sur cet
hôte (`MIKA_GITHUB_APP_ID` / `_PRIVATE_KEY` / `_INSTALLATION_ID`, les trois) — sans
elles `GitHubApp::from_settings` rend `None` et le chemin ne tourne jamais. *Un
chemin jamais emprunté se lit exactement comme un chemin sain.* Établir ensuite
que le binaire servi porte le correctif (classe mika#2340).

**Halte 4 — `gh_app_token_exchange_failed` en `401`.** Le JWT est signé mais
refusé. Ce n'est **pas** nécessairement ce correctif : `iat`/`exp` sont pinnés à
540 s pour tolérer ~120 s de dérive d'horloge positive (mika#1042), et un hôte
dérivant davantage produit le même symptôme. Lire l'horloge de l'hôte **avant**
de soupçonner le backend crypto — le test V4 a déjà attesté que la signature est
correcte.

---

## Definition of Done

- [ ] `Cargo.toml` : `jsonwebtoken = { version = "11", features = ["aws_lc_rs"] }`,
      avec le commentaire load-bearing du §2.
- [ ] `Cargo.lock` régénéré par `cargo build` (non édité à la main).
- [ ] Constante `TEST_RSA_PEM_PUBLIC` ajoutée dans `mod tests` de
      `crates/mika-common/src/github_app.rs`, dérivée de la clé privée de **ce
      module**.
- [ ] Test `mika2525_le_jwt_produit_porte_une_signature_rs256_verifiable` écrit,
      et son contrôle négatif V5 **vu rouge** avant commit.
- [ ] `test_generate_jwt_claims` **non modifié** et vert.
- [ ] V1 à V11 vertes.
- [ ] `docs/solutions/best-practices/un-bump-majeur-peut-etre-vert-a-la-compilation-et-panique-a-lexecution-2026-09-25.md`
      écrit, avec frontmatter YAML.
- [ ] Corps de PR : nomme les rectifications R1/R2/R3 du §0 (le `default-features =
      false` de l'AC1 est refusé **avec sa mesure**), et déclare que la moitié
      bout-en-bout d'AC3 est une sonde opérateur (S2), pas un test.
- [ ] Corps de PR : `Closes #2525`, et la relation à #2454 explicitée (le bump est
      porté ici ; #2454 devient redondant).

---

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2525, avec la rectification du §0
signalée là où la lettre diverge de ce qui est livré.

- **AC1** — Configurer le backend crypto de jsonwebtoken 11 : soit
  `default-features = false` + activer exactement une feature (`aws_lc_rs` de
  préférence, cohérent avec rustls-tls du reste du stack), soit
  `CryptoProvider::install_default()` au démarrage. Décider et documenter le choix.
  > **Livré avec une divergence mesurée (§0 R2), et l'intention est tenue.**
  > `features = ["aws_lc_rs"]` est activée — exactement une, et celle que l'AC
  > préfère. `default-features = false` est **refusé** : il retire `use_pem`, donc
  > `EncodingKey::from_rsa_pem`, appelé à trois sites de production. La seconde
  > branche (`install_default()`) est refusée aussi, avec sa raison (§2) : quatre
  > points d'entrée, dont les harnais de test qui n'ont pas de `main`. Le choix est
  > documenté au site de la déclaration (§7.1) et dans `docs/solutions/` (§7.2).
- **AC2** — `github_app::tests::test_generate_jwt_claims` VERT (et toute la suite
  mika-common : 640/640). → V2, V3.
- **AC3** — Vérif fonctionnelle : la génération du JWT App produit un token valide
  (l'auth App du pilote fonctionne réellement, pas seulement le test).
  > **Livré en deux moitiés, et la seconde est nommée comme non testable (§4).**
  > Moitié testable : V4 vérifie la signature RS256 contre la clé publique, avec
  > son contrôle négatif V5. Moitié bout-en-bout : sonde opérateur S2 (§8), parce
  > qu'elle exige des credentials App réels et un appel à `api.github.com` —
  > la revendiquer comme livrée par un test reproduirait le faux-vert que ce
  > ticket corrige.
- **AC4** — #2454 ne merge QUE ce test vert. Aucun contournement (pas de pin
  jsonwebtoken 9, pas d'exclusion). → V10 (aucun pin arrière), V3 (aucun `ignored`
  nouveau), et le §2 : le bump est porté par cette PR, donc #2454 se ferme au merge
  plutôt que de merger sur une `main` rouge.

---

## 9. Hors périmètre, délibérément

- **Un scan de source sur `Cargo.toml`** vérifiant « exactement une feature
  crypto ». Refusé avec sa mesure (§3) : les trois régressions de la classe font
  paniquer un test qui tourne déjà, et la quatrième (`default-features = false`) ne
  compile pas. Un détecteur redondant est un détecteur inventé.
- **La migration `opentelemetry`** (senara-solutions/mika#2523), citée par le
  ticket comme voisine. Deux bumps majeurs, deux arbres de dépendances disjoints,
  deux PR.
- **`Algorithm` autre que RS256.** mika ne signe que RS256
  (`github_app.rs:383`) ; `aws_lc_rs` le sert, et vérifier la couverture des autres
  algorithmes serait tester le crate amont.
- **Le chemin JWK.** `KeyUtils::new_unimplemented` panique aussi sur le fallback,
  mais mika n'appelle aucune fonction JWK — population vide, aucun test à écrire.
- **Le durcissement de `SECRET_REGEX`** pour couvrir `-----BEGIN PUBLIC KEY`.
  Ce n'est pas un secret, et l'y ajouter ferait rougir la garde sur une donnée
  publique. Nommé parce que la constante du §4 croise ce motif, pas parce qu'il y a
  quelque chose à faire.
- **Le pin de `aws-lc-rs`.** Il arrive transitivement par `rustls` et suit ce
  rail ; le figer ici créerait un second lieu de décision pour une version qui en a
  déjà un.
