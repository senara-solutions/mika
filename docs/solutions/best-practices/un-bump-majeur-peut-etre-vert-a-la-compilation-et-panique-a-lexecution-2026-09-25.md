---
module: deps
tags: [dependabot, crypto, jwt, github-app, cargo-features]
problem_type: silent-runtime-failure
date: 2026-09-25
tickets: [senara-solutions/mika#2525, senara-solutions/mika#2454]
---

# Un bump majeur peut être vert à la compilation et paniquer à l'exécution

## La classe

Quand un crate déplace une décision **du code vers les features de Cargo**, le
compilateur cesse d'être le détecteur. Le bump compile, le binaire se lie, et le
premier appel réel panique. Aucun test qui ne *traverse* pas le chemin ne le voit.

C'est un motif d'architecture, pas un accident : `rustls` l'a introduit avec ses
`CryptoProvider`, `jsonwebtoken` 11 l'a repris en retirant `ring`. Le crate ne
choisit plus de backend à votre place — il exige que vous en déclariez un, et
**il ne peut pas vous le rappeler à la compilation** parce qu'une feature absente
n'est pas une erreur de type : c'est un `cfg` qui ne matche pas.

## Le cas mesuré (mika#2454 → mika#2525)

Dependabot a proposé `jsonwebtoken` 9.3.1 → 11.1.0. Le bump était **nu** : une
seule ligne de `Cargo.toml`, aucune feature touchée. CI : build vert, un test
rouge.

```
thread 'github_app::tests::test_generate_jwt_claims' panicked at
  jsonwebtoken-11.1.0/src/crypto/mod.rs:124:40:
Could not automatically determine the process-level CryptoProvider from
jsonwebtoken crate features.
```

Le chemin touché est l'authentification GitHub App — l'identité sous laquelle la
boucle autonome signe ses JWT. Un `generate_jwt` qui panique, c'est la boucle qui
perd sa forge.

## Les deux pièges de lecture, et ils mènent au remède INVERSE

### Piège 1 — « les deux features par défaut coexistent »

C'est la lecture naturelle du message d'erreur (« make sure **exactly one** … is
enabled »), et elle est fausse. Le `Cargo.toml` publié dit :

```toml
[features]
default = ["use_pem"]          # ← aucune feature crypto
aws_lc_rs = ["dep:aws-lc-rs"]
rust_crypto = ["dep:ed25519-dalek", "dep:hmac", "dep:p256", ...]
```

Et le site de panique est un `cfg` en **cascade exclusive** dont le troisième bras
est un fallback qui panique :

```rust
#[cfg(all(feature = "rust_crypto", not(feature = "aws_lc_rs")))] { ... }
#[cfg(all(feature = "aws_lc_rs", not(feature = "rust_crypto"))) ] { ... }
#[allow(unreachable_code)] { &INSTANCE }   // signer: |_, _| panic!(...)
```

Il y avait donc **zéro** backend, pas deux. La différence n'est pas cosmétique :
sous la lecture « deux backends » on en **retire** un (`default-features =
false`) ; sous la lecture mesurée on en **ajoute** un. C'est l'inverse.

**Vérifiez la cascade avant de choisir le geste.** Deux features activées *font*
aussi zéro backend — mais ce n'était pas l'état de départ.

### Piège 2 — `default-features = false` a l'air d'une hygiène et casse la production

Le réflexe « je désactive les defaults et j'active exactement ce qu'il me faut »
retire ici `use_pem`, donc **fait disparaître `EncodingKey::from_rsa_pem`**, qui
était appelé à trois sites de production (dont `mika doctor`). On échange une
panique runtime contre une erreur de compilation à trois endroits.

La forme correcte **ajoute** sans substituer :

```toml
jsonwebtoken = { version = "11", features = ["aws_lc_rs"] }
```

On déclare ce qu'on ajoute ; on ne recopie pas ce qu'on garde. `default-features =
false` + `features = ["use_pem", "aws_lc_rs"]` produirait aujourd'hui le même
ensemble et serait **plus fragile** : il fige une liste qui devrait suivre le
`default` amont, donc il casse en silence le jour où le crate ajoute une feature
par défaut.

## Compter le coût d'un backend : lisez le diff du lock, pas `cargo tree`

L'intuition « ce crate est déjà dans mon arbre, donc l'activer est gratuit » est
**fausse**, et `cargo tree` ne la corrige pas : il montre le graphe, pas ce qui est
compilé ni avec quelles features.

Mesuré sur ce cas : `aws-lc-rs 1.18.0` était déjà au lock (via
`reqwest → hyper-rustls → rustls`, et via `sqlx`). Activer `jsonwebtoken/aws_lc_rs`
a pourtant **ajouté deux crates** — `untrusted 0.7.1` et `zeroize_derive 1.5.0` —
parce que `rustls` déclare `aws-lc-rs` avec `default-features = false` là où
`jsonwebtoken` le déclare **sans**. Le même crate, deux jeux de features, deux
coûts.

> **Le geste qui ne trompe pas :** `git diff Cargo.lock` après le changement de
> feature. La résolution du lock aboutit même quand le téléchargement échoue, donc
> ce diff est lisible avant toute compilation réussie.

Le choix `aws_lc_rs` reste le bon ici (2 crates contre 3 pour `rust_crypto`, et une
toolchain native déjà satisfaite puisque `sqlx` active `tls-rustls-aws-lc-rs`) —
mais il est justifié par la mesure, pas par « c'est déjà là ».

## Pourquoi la feature plutôt que `CryptoProvider::install_default()`

Les deux ferment la panique. La feature est résolue **à la compilation, pour tout
binaire du workspace** ; `install_default()` exige un site d'appel dans chaque
point d'entrée — et le chemin de test est précisément celui qui n'a pas de `main`.
Un `install_default()` oublié dans un seul harnais rend la panique, à l'identique.

*Un invariant tenu par le système de features n'a pas de site d'initialisation à
rater.*

## Le détecteur : un test qui EXERCE le chemin

Un bump de cette classe n'est vu que par du code qui appelle réellement la
fonction. Deux niveaux, et ils ne disent pas la même chose :

| test | ce qu'il atteste |
|---|---|
| décoder le **payload** du JWT | qu'un provider **existe** (la panique a cessé) |
| **vérifier la signature** contre la clé publique | qu'il **signe correctement** |

Le test préexistant faisait le premier : il base64-décode le payload et ne touche
jamais la signature. Une fois le backend déclaré il repasse vert **sans rien dire
de la validité cryptographique**. Le second a dû être écrit, et il exerce en plus
le chemin de **vérification** du provider (`verifier_factory`), que le chemin de
signature seul ne traverse pas.

**Son contrôle négatif n'est pas optionnel :** altérez un octet de la signature et
voyez le test rougir en `InvalidSignature` avant de committer. Un `decode` jamais
vu rougir ne distingue pas « la signature est valide » de « `decode` ne vérifie
rien ici » — la forme de panne de départ, reproduite un cran plus loin.

## Ce qu'un test ne peut pas attester

« Le token est cryptographiquement valide » et « GitHub l'accepte » sont deux
propositions distinctes. La seconde exige des credentials App réels et un appel à
`api.github.com` ; elle relève d'une **sonde opérateur**, pas d'un test. Les
confondre, c'est refaire exactement le faux-vert qu'on vient de corriger.

## Pas de scan de source, et c'est raisonné

Une garde structurelle du type « exactement une feature crypto déclarée » serait
**strictement redondante** : les trois manières de rouvrir le défaut (feature
retirée, deuxième feature ajoutée, feature activée par une crate tierce via
l'unification) finissent toutes au même `panic!` d'un test qui tourne déjà. La
quatrième (`default-features = false`) **ne compile pas** — attrapée plus tôt et
plus fort. *Un détecteur redondant est un détecteur inventé.*

Ce qui a été livré à la place : le **commentaire au site de la déclaration**, dans
`Cargo.toml`, nommant la feature comme load-bearing et le piège
`default-features = false`. C'est là qu'un éditeur qui simplifie — ou un futur
Dependabot dont la PR touche cette ligne — lira la raison. Une entrée de doc seule
laisserait la ligne nue devant lui.

## À retenir

1. **Un bump majeur peut être vert à la compilation et paniquer à l'exécution.** La
   question à poser n'est pas « ça compile ? » mais « quel test *exerce* le chemin
   que ce crate a déplacé ? ».
2. **Lisez le `cfg` du site de panique** avant de choisir le remède. « Exactement
   une » peut vouloir dire qu'il y en a zéro.
3. **`default-features = false` n'est pas une hygiène neutre** — il retire des
   features non-crypto dont la production dépend.
4. **Comptez le coût dans `git diff Cargo.lock`**, pas dans `cargo tree` : le même
   crate peut arriver avec deux jeux de features et deux coûts.
5. **Un test de vérification de signature doit avoir été vu rouge** sur une
   signature fausse, sinon il n'atteste rien.

## Limite d'environnement rencontrée

Le sandbox de dispatch monte `~/.cargo` en **lecture seule** (containment
mika#2049) : un ticket qui ajoute une dépendance absente du cache de l'hôte ne peut
donc pas être compilé par un pilote dispatché, et `CARGO_HOME` n'est pas
surchargeable. Dans ce cas, la vérification du *code* peut être conduite sous un
backend dont les crates sont déjà présents (le test est backend-agnostique) et la
vérification du *backend livré* déléguée à la CI — en le disant, jamais en laissant
croire que tout a été vérifié localement.
