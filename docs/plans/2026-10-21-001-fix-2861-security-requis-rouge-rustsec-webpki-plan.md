---
date: 2026-10-21
type: fix
---

# mika#2861 — Security requis rouge : RUSTSEC-2026-0381/0382/0383 sur `rustls-webpki` et `webpki-roots`

**Ticket :** senara-solutions/mika#2861
**Branche :** `fix/2861/deps-security-requis-rouge-rustsec-2026`
**Précédents :** mika#2685 (`quinn-proto`, PR #2685/#2686), mika#2692 (`rustls` 0.23.46)
**Doctrine amont :** `docs/solutions/dependency-maintenance/2026-10-13-bump-quinn-proto-clear-rustsec-2026-0338-0340-0343.md`

---

## Contexte

Le job **Security** de `.github/workflows/ci.yml:1072-1084` enchaîne deux
commandes :

```yaml
      - name: Run cargo-audit
        run: cargo audit
      - name: Run cargo-deny
        run: cargo deny check
```

Depuis la publication de quatre avis RustSec le 2026-10-20, `cargo audit`
rougit sur **toute nouvelle tête**, parce que le défaut est dans le
`Cargo.lock` de `main` (`c5e64079`) et non dans une PR. #2858 (`abb9ab67`) est
la première tête poussée après la publication et elle rougit — run
39492775140, job 118869969070, 2026-10-20T20:34Z. Security étant requis,
**aucune PR neuve ne peut plus atteindre 28/28**, et les PR plus anciennes
passeront au rouge dès leur prochain push, rebase ou retarget.

### Les quatre nœuds du lock, et qui les tire

Le graphe est lu dans `Cargo.lock` de `main` (`c5e64079`). Chacun des quatre
nœuds est **transitif** : aucun n'est déclaré dans un `Cargo.toml` du
workspace.

| nœud du lock | avis | consommateur(s) dans le lock | correctif annoncé |
|---|---|---|---|
| `rustls-webpki 0.103.15` (`Cargo.lock:3773`) | RUSTSEC-2026-0382 | `rustls 0.23.46` (`:3718`), `rustls-platform-verifier 0.7.0` (`:3759`) | `>=0.103.16, <0.104.0-alpha.1` |
| `webpki-root-certs 1.0.9` (`:5360`) | RUSTSEC-2026-0383 | `rustls-platform-verifier 0.7.0` (`:3762`) — **seul** | `>=1.0.10` |
| `webpki-roots 1.0.9` (`:5378`) | RUSTSEC-2026-0381 | `webpki-roots 0.26.11` (`:5374`) — **seul** | `>=1.0.10` |
| `webpki-roots 0.26.11` (`:5369`) | RUSTSEC-2026-0381 | `sqlx-core 0.8.6` (`:4216`) — **seul** | `>=1.0.10`, ou une 0.26.x qui réexporte une 1.0.10+ |

Les deux chaînes d'entrée :

- `reqwest 0.13.4` (`:3506`, transitif — le `reqwest` du workspace est 0.12.28)
  → `rustls-platform-verifier 0.7.0` → `webpki-root-certs 1.0.9` + `rustls-webpki 0.103.15`
- `sqlx-core 0.8.6` → `webpki-roots 0.26.11` → `webpki-roots 1.0.9`

Déclarations directes, pour mémoire : `Cargo.toml:22` (`reqwest = "0.12"`),
`Cargo.toml:128` (`sqlx = "0.8"`), `crates/mika-gateway/Cargo.toml:50`
(`rustls = "0.23"`). Aucune ne nomme les quatre nœuds, et un correctif de
niveau patch est satisfait par chacune : **`Cargo.toml` reste intouché** dans
le cas nominal.

### Pourquoi le nœud `0.26.11` est la seule vraie difficulté

`webpki-roots 0.26.11` est une **coquille de réexport** : son unique
dépendance est `webpki-roots 1.0.9` (`Cargo.lock:5374`). Monter le nœud 1.0 à
1.0.10 corrige donc ce que la coquille **sert à l'exécution**.

Mais `cargo audit` **lit le lockfile, pas le graphe de build** — c'est la
leçon déjà écrite pour `quinn-proto` — et il confronte le couple
*(nom, version)* aux champs `patched` / `unaffected` de l'avis. Le nœud
`webpki-roots 0.26.11` reste donc visible à l'audit sous sa propre version,
indépendamment de ce qu'il réexporte.

Conséquence : **la décision sur ce nœud ne se prend pas au jugé, elle se lit
dans l'avis**, et l'avis se lit localement, dans la base que `cargo audit`
utilise lui-même. Le § 3 ci-dessous en fait une procédure à trois branches.
Le libellé du ticket — « ou une 0.26.x qui réexporte une 1.0.10+ » — décrit la
branche B1, qui est la forme habituelle d'un correctif amont sur une coquille
de réexport ; **ce plan ne postule pas qu'une `0.26.12` existe.** Le pilote
lit le champ `patched`.

### Le second échec qui se cache derrière le premier

`cargo audit` tombant depuis le 2026-10-20, **`cargo deny check` n'a tourné
sur aucune tête récente** : il est la seconde étape du même job, et une étape
qui ne tourne pas ne montre pas ses propres échecs. C'est exactement ce qui
s'est produit sur mika#2685, où `unsafe-libyaml` attendait derrière
`quinn-proto` et a demandé son propre ticket (mika#2686).

Donc AC2 (« `cargo deny check` reste vert ») n'est pas une vérification de
non-régression : c'est une **première** vérification. Elle doit tourner
localement, et elle peut découvrir une matière hors périmètre. Le § 5 dit quoi
en faire.

---

## 1. Ce que ce vol livre

Un seul artefact de code : le `Cargo.lock`. Plus un document de solution
(`docs/`, hors compte de taille) portant la leçon non évidente — *la version
qu'un audit lit sur une coquille de réexport n'est pas celle qu'elle sert*.

**Interdits, posés par le ticket :**

- aucune entrée ajoutée à `.cargo/audit.toml` `[advisories] ignore` (qui porte
  aujourd'hui une seule entrée, `RUSTSEC-2023-0071`) ;
- aucune entrée ajoutée à `deny.toml` `[[advisories.ignore]]` (qui en porte
  trois : `RUSTSEC-2026-0253`, `-0221`, `-0205`) ;
- aucun contournement par `deny.toml` `[bans]`, ni par retrait d'une feature
  TLS dans un `Cargo.toml` pour faire disparaître un nœud du graphe.

---

## 2. Les trois montées sans ambiguïté

À faire dans cet ordre, **une commande par étape**, en relisant
`git diff Cargo.lock` après chacune.

```bash
cargo update -p rustls-webpki@0.103.15 --precise 0.103.16
cargo update -p webpki-root-certs@1.0.9 --precise 1.0.10
cargo update -p webpki-roots@1.0.9     --precise 1.0.10
```

**La forme qualifiée `-p <crate>@<version>` est obligatoire pour
`webpki-roots`** : deux versions du même nom cohabitent dans le lock (0.26.11
et 1.0.9) et `cargo` refuse un `-p webpki-roots` nu comme ambigu. Elle est
employée partout pour l'uniformité et pour que le diff dise de quel nœud il
parle.

**Cible = la borne basse de la plage corrigée**, pas la dernière publiée : le
diff doit rester lisible comme « cet avis, et rien d'autre ». Si la borne
basse n'existe pas sur le registre (yankée, par exemple), monter au plus petit
successeur disponible **dans la plage**, et le dire dans le corps de PR.

### Discipline sur le diff (piège mesuré sur mika#2685)

`cargo update -p X --precise V` peut déplacer des arêtes sans rapport : sur le
vol `quinn-proto`, il a aussi bougé l'arête `getrandom` de `tempfile`. Après
chaque commande :

1. `git diff Cargo.lock` — le diff attendu est, par nœud, une ligne `version`
   et une ligne `checksum` ;
2. tout hunk que l'avis ne réclame pas est **révoqué** (`git checkout -p`, ou
   réécriture ciblée), sauf s'il est imposé par la résolution — dans ce cas il
   est nommé dans le corps de PR ;
3. `cargo metadata --locked` confirme que le lock est cohérent et n'a plus
   besoin d'être réécrit.

### Interdit : éditer `Cargo.lock` à la main

Si `cargo update` est refusé dans le bac à sable — il l'a été dans la session
de grooming (`no matching policy rule`) — **arrêter et remonter**. Une montée
à la main est impossible honnêtement : le champ `checksum` ne se calcule pas
sans le registre, et un checksum faux empoisonne le lock pour tout le monde,
en CI comme chez l'opérateur. Ce n'est pas un contournement acceptable.

De même, si `cargo audit` n'est pas installé : `cargo install cargo-audit
cargo-deny --locked`, la commande même que le job Security utilise
(`ci.yml:1080`). Elle coûte du temps d'horloge, pas des tours. Si la base
d'avis (`~/.cargo/advisory-db`, clone git de `rustsec/advisory-db` déclaré en
`deny.toml:5-6`) n'est pas joignable depuis le bac à sable, AC1 n'est pas
vérifiable localement : le dire explicitement dans le corps de PR et laisser
CI être le vérificateur. **Ne jamais affirmer un `cargo audit` vert qui n'a
pas tourné.**

---

## 3. Le nœud `webpki-roots 0.26.11` — procédure à trois branches

**Étape 0 — nommer le consommateur**, comme le ticket le demande :

```bash
cargo tree -i webpki-roots@0.26.11
```

Le lock dit déjà la réponse : `sqlx-core 0.8.6` (`Cargo.lock:4216`) est le
**seul** consommateur. La commande est là pour l'attester plutôt que pour la
découvrir, et pour que le corps de PR cite une sortie et non une lecture.

**Étape 1 — lire la plage corrigée dans la base d'avis**, source de vérité
unique et locale :

```bash
cargo audit --json > .pilot-scratch/audit-2861.json
```

puis y lire, pour `RUSTSEC-2026-0381`, le champ
`vulnerabilities.list[].versions.patched` (et `.unaffected`). C'est ce champ,
et lui seul, qui décide de la branche. Un brouillon sous `.pilot-scratch/` ne
se supprime jamais (mika#2548).

### B1 — la plage corrigée inclut une `0.26.x` *(branche attendue)*

```bash
cargo update -p webpki-roots@0.26.11 --precise <la 0.26.x annoncée>
```

C'est la forme habituelle du correctif amont d'une coquille de réexport : la
nouvelle 0.26.x ne change que sa contrainte sur `webpki-roots` 1.0, pour
exiger `>=1.0.10`. Vérification : le nœud 1.0 du lock est bien à 1.0.10 ou
au-delà, et le diff ne touche que ces deux paquets.

### B2 — elle ne l'inclut pas, mais une `sqlx` 0.8.x sort du nœud 0.26

Si `sqlx` publie une 0.8.x (> 0.8.6) dont la contrainte `webpki-roots` est
passée en `1.0` :

```bash
cargo update -p sqlx --precise <la 0.8.x>
```

`sqlx` est un workspace amont : ses six paquets (`sqlx`, `sqlx-core`,
`sqlx-macros`, `sqlx-macros-core`, `sqlx-mysql`, `sqlx-postgres`,
`sqlx-sqlite`) bougent ensemble. La contrainte du dépôt reste satisfaite —
`Cargo.toml:128` déclare `sqlx = "0.8"` — donc `Cargo.toml` n'est toujours pas
touché.

Attestation du retrait du nœud, et non de son remplacement :

```bash
grep -c 'webpki-roots 0.26' Cargo.lock   # attendu : 0
```

Risque propre à cette branche : `sqlx` porte le registre Postgres de
`mika-gateway`. `cargo check -p mika-gateway --locked` puis
`cargo test -p mika-gateway` sont obligatoires **avant** la suite complète, et
une rupture d'API y est une raison légitime de basculer en B3 plutôt que de
réécrire du code de gateway dans un vol de dépendances.

### B3 — ni l'une ni l'autre : **halte et remontée**

AC4 interdit l'ignore, et le ticket interdit le contournement par
`Cargo.toml`. Il n'y a donc pas de troisième geste : le pilote **s'arrête** et
remonte, en nommant dans le corps de PR (a) `sqlx-core 0.8.6` comme seul
consommateur, (b) les deux mouvements amont qui débloqueraient — une 0.26.x
corrigée, ou une `sqlx` 0.8.x sur `webpki-roots` 1.0 — et (c) un ticket de
suivi pour la veille.

Dans ce cas, **les trois montées du § 2 sont quand même livrées** : elles
lèvent RUSTSEC-2026-0382 et -0383 et réduisent le rouge à un seul avis. Le job
Security reste rouge, AC1 n'est pas atteint, et le corps de PR le dit sans
l'habiller. Une PR qui lève trois avis sur quatre en le disant vaut mieux
qu'une PR qui en lève quatre en mentant sur le quatrième.

---

## 4. Hors périmètre, explicitement

### `chacha20 0.10.1` yanké

Consommateur : `rand 0.10.3` (`Cargo.lock:3228-3237`). C'est un
**avertissement**, pas une vulnérabilité :

- `cargo audit` signale un paquet yanké en warning et sort en 0 — le job
  n'invoque pas `--deny warnings` (`ci.yml:1082`) ;
- `deny.toml` `[advisories]` (lignes 5-6) ne déclare **pas** de clé `yanked`,
  donc le défaut de `cargo-deny` (`warn`) s'applique, et AC2 n'est pas en jeu.

**Ne pas le poursuivre dans ce vol.** Il n'est pas la cause du rouge, et le
chasser élargit le diff sur un nœud (`rand` 0.10) qui touche tout le
workspace. Si `cargo deny check` devait malgré tout tomber dessus, c'est le
§ 5 qui s'applique.

### Les autres avis que `cargo deny check` pourrait révéler

Voir § 5. Hors périmètre par construction.

---

## 5. Si `cargo deny check` tombe sur autre chose

`cargo deny check` n'a pas tourné sur une tête récente (§ Contexte). S'il
tombe sur une matière **hors des quatre avis de ce ticket** — un avis frais
sur un autre paquet, une licence nouvelle, un yanked durci :

1. **ne pas** l'absorber dans ce vol, et surtout **ne pas** l'ignorer en
   `deny.toml` pour faire passer le job — AC4 porte sur les listes d'ignore de
   l'audit, et l'esprit du ticket (« pas d'ignore pour faire passer le
   check ») couvre les deux fichiers ;
2. ouvrir un ticket de suivi, et le nommer par une ligne `Tracked in:` dans le
   corps de PR — c'est la trajectoire mika#2685 → mika#2686, déjà parcourue ;
3. si cette matière **bloque** AC2, le vol ne peut pas atteindre son plancher :
   le dire, et remonter à l'opérateur pour cadrage.

Le seul cas où elle entre dans ce vol est celui où la montée du § 2 **en est
la cause** — un hunk collatéral mal révoqué, par exemple. Alors c'est une
régression de ce vol, et elle se corrige ici.

---

## 6. Ordre de commit

Un seul commit, le lock et son document de solution.

```
fix(deps): rustls-webpki 0.103.16 et webpki-roots 1.0.10 (RUSTSEC-2026-0381/0382/0383) (mika#2861)
```

Le sujet nomme les versions **réellement posées** : si la branche B2 ou B3 est
retenue, il est réécrit en conséquence. Un sujet qui annonce une version que
le lock ne porte pas est une fausse attestation.

**Plancher du vol** (en deçà, rien n'est livrable) :

```bash
cargo metadata --locked              # le lock se tient
cargo check --workspace --locked     # le workspace compile sur ce lock
cargo audit                          # sortie 0 — AC1
cargo deny check                     # sortie 0 — AC2
```

Puis la fin de pipeline usuelle : `cargo test --workspace`,
`cargo test -p mika-common mika2398`, `cargo clippy`, `cargo fmt --check`,
`make lint`, `scripts/verify-pipeline.sh origin/main`.

---

## 7. Document de solution

`docs/solutions/dependency-maintenance/2026-10-21-bump-rustls-webpki-webpki-roots-clear-rustsec-2026-0381-0382-0383.md`,
sur le modèle du document `quinn-proto`. Frontmatter
`title` / `date` / `tags: [dependency, security, cargo-audit, rustls-webpki, webpki-roots, lockfile]`.

Ce qu'il doit porter, et qui n'est pas dans le document amont :

- **la leçon centrale** — une coquille de réexport est auditée sur *sa* version,
  pas sur celle qu'elle sert ; monter le nœud réexporté ne retire pas la
  coquille du rapport ;
- la procédure « lire `patched` dans `cargo audit --json` plutôt que deviner
  la version corrigée » ;
- la forme qualifiée `-p <crate>@<version>`, obligatoire dès que deux versions
  d'un nom cohabitent ;
- le rappel que `cargo deny check` n'a pas tourné pendant que `cargo audit`
  tombait — seconde occurrence de la leçon mika#2685, donc elle est désormais
  un motif et non un accident.

---

## 8. Aucun détecteur livré — la porte Fire-Disposition est N/A

Dit explicitement pour que l'absence de section `## Fire-Disposition`
(mika#2306) se lise comme une décision et non comme un oubli.

Ce vol ne livre **aucun** livrable dont le chemin de succès serait « aucune
violation trouvée » : pas de test, pas de lint, pas de garde CI, pas de
validateur. Le détecteur existe déjà et c'est le job Security lui-même
(`cargo audit` + `cargo deny check`, `ci.yml:1072-1084`) : il a correctement
détecté les quatre avis, et c'est son rouge qui a produit ce ticket. Ce vol
**désarme la violation**, il n'ajoute pas d'instrument.

Et il n'en ajoute pas délibérément : un test qui assert sur des versions du
lock doublerait l'audit, et deviendrait stale au prochain avis — exactement le
mode de défaillance que mika#2309 condamne, un prédicat écrit sur une valeur
plutôt que sur le site qui la borne.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `Cargo.lock` — 3 nœuds du § 2 (`version` + `checksum`) | 6 |
| `Cargo.lock` — nœud `webpki-roots 0.26.11` (B1 : 2 lignes ; B2 : 7 paquets `sqlx` × 2) | 14 |
| `docs/solutions/dependency-maintenance/…` | 0 (sous `docs/`) |

Total estimé : 20 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 20 lignes contre une borne de 429 et un seuil
par PR de 1000 : les deux bornes sont passées d'un ordre de grandeur. Le coût
réel de ce vol n'est pas en lignes, il est en **durée d'horloge** —
l'installation de `cargo-audit`/`cargo-deny`, le clone de la base d'avis, et
`cargo test --workspace` sur ~3463 tests. Chacun est **un** tour, et la
procédure du § 3 est écrite pour qu'aucune branche ne demande d'exploration :
la décision se lit dans un champ JSON, pas dans une enquête. L'estimation
retient la branche B2, la plus coûteuse, pour ne pas être vraie seulement dans
le cas favorable.

---

## Definition of Done

- [ ] Les trois montées du § 2 sont dans `Cargo.lock`, et `git diff Cargo.lock`
      ne porte que des lignes `version`/`checksum` des nœuds visés — tout hunk
      collatéral est révoqué ou nommé dans le corps de PR.
- [ ] `cargo tree -i webpki-roots@0.26.11` est cité dans le corps de PR, et la
      branche retenue (B1, B2 ou B3) est nommée avec la plage `patched` qui l'a
      décidée.
- [ ] `cargo metadata --locked` et `cargo check --workspace --locked` sortent en 0.
- [ ] `cargo audit` sort en 0, ou — branche B3 — le corps de PR nomme l'avis
      résiduel, son unique consommateur et son ticket de suivi, sans affirmer
      un vert qui n'existe pas.
- [ ] `cargo deny check` sort en 0, ou sa matière hors périmètre est renvoyée
      par une ligne `Tracked in:` (§ 5).
- [ ] `cargo test --workspace` et `cargo test -p mika-common mika2398` sont verts.
- [ ] `.cargo/audit.toml` et `deny.toml` sont **inchangés** — `git diff --stat`
      ne les liste pas.
- [ ] Aucun `Cargo.toml` modifié ; si une contrainte l'a imposé, elle est
      nommée dans le corps de PR avec le fichier et la ligne.
- [ ] Le document de solution du § 7 est écrit.
- [ ] `cargo clippy`, `cargo fmt --check`, `make lint` et
      `scripts/verify-pipeline.sh origin/main` sont verts.
- [ ] Le corps de PR porte `Closes #2861` et
      `Code review: delegated to mika-qa (mika#2694)`.

---

## Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#2861.

- **AC1** : `cargo audit` sort en 0 sur la branche, et le job Security de la PR
  est vert.
- **AC2** : `cargo deny check` reste vert.
- **AC3** : `cargo test --workspace` et `cargo test -p mika-common mika2398`
  sont verts.
- **AC4** : aucune entrée ajoutée aux listes d'ignore de l'audit.
