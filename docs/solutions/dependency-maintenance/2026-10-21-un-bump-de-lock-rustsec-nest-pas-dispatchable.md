---
title: "Un bump de `Cargo.lock` pour RUSTSEC n'est pas dispatchable — le bac à sable refuse `cargo update`"
date: 2026-10-21
category: dependency-maintenance
module: Cargo.lock
component: development_workflow
problem_type: best-practice
severity: high
symptoms:
  - "`cargo update -p rustls-webpki@0.103.15 --precise 0.103.16` → `no matching policy rule -- denied by default (production posture; widen rules to allow new tool footprints)`"
  - "`cargo update --help` refusé lui aussi — le refus porte sur le préfixe de sous-commande, pas sur un drapeau"
  - "`cargo audit --version` refusé, et `~/.cargo/advisory-db` absent : la base d'avis n'a jamais été clonée dans ce bac à sable"
  - "`~/.cargo/registry/index/index.crates.io-*/` ne contient que `config.json` — aucun cache d'index épars, donc aucune résolution hors ligne non plus"
  - "PR #540 (bump `rand`, 2026-04-11) est signée `app/mika-platform-bot` — le loop savait le faire ; PR #2685 (bump `quinn-proto`, 2026-10-13) est signée `samidarko` avec `pipeline-exempt` — la capacité a été retirée entre les deux"
root_cause: ticket_class_outside_sandbox_capability
resolution_type: routing
tags: [mika-2861, mika-2685, dependency, security, cargo-audit, cargo-update, lockfile, rustsec, dispatch, bac-a-sable, preflight]
---

# Un bump de `Cargo.lock` pour RUSTSEC n'est pas dispatchable — le bac à sable refuse `cargo update`

## Le problème

mika#2861 demandait de monter quatre nœuds de `Cargo.lock` pour éteindre
RUSTSEC-2026-0381/0382/0383, par `cargo update -p <crate> --precise <version>`.
Le ticket a été groomé (READY → GROOMED) puis dispatché à un pilote. Dans le bac
à sable du pilote, **toutes** les sous-commandes `cargo` qui touchent au registre
sont refusées par la permission-policy :

| commande | verdict |
|---|---|
| `cargo update -p rustls-webpki@0.103.15 --precise 0.103.16` | refusé |
| `cargo update --dry-run -p …` | refusé |
| `cargo update --help` | refusé |
| `cargo generate-lockfile --offline` | refusé |
| `cargo fetch --locked` | refusé |
| `cargo audit --version` | refusé |
| `cargo tree -i webpki-roots@0.26.11` | **autorisé** |
| `cargo metadata --locked --format-version 1` | **autorisé** |

Le message est le même pour les six :
`no matching policy rule -- denied by default (production posture; widen rules to
allow new tool footprints)`.

Que `cargo update --help` soit refusé est la mesure qui tranche : le refus porte
sur le **préfixe de sous-commande**, pas sur `--precise` ni sur `--dry-run`. Il
n'y a donc pas de forme de la commande qui passe. La règle d'autorisation
(`bash-cargo`) couvre l'inspection, la compilation et les tests ; elle exclut
délibérément ce qui mute le graphe de dépendances.

## Pourquoi un refus sur `cargo update` n'a pas de contournement honnête

C'est ce qui distingue ce refus de la plupart des autres. Quand `md5sum` est
refusé, l'outil `Read` atteint le même but. Ici, non :

- **Éditer `Cargo.lock` à la main est impossible honnêtement.** Le champ
  `checksum` ne se calcule pas sans le registre, et un checksum faux empoisonne
  le lock pour tout le monde — en CI comme chez l'opérateur. Ce n'est pas un
  contournement, c'est un sabotage silencieux.
- **La résolution hors ligne n'y arrive pas non plus.** Deux faits mesurés :
  `~/.cargo/registry/index/index.crates.io-1949cf8c6b5b557f/` ne contient que
  `config.json` — pas de cache d'index épars — et
  `~/.cargo/registry/cache/*/` porte `rustls-webpki-0.103.{9,13,14,15}` et
  `webpki-roots-1.0.{6,7,9}`, mais **ni `0.103.16` ni `1.0.10`**. Les versions
  corrigées ne sont nulle part sur le disque. Retirer une entrée du lock pour
  laisser `cargo` re-résoudre ne peut donc rien trouver, même si le geste était
  permis.
- **Monter la version dans un `Cargo.toml`** est interdit par le ticket, et ne
  changerait rien : ce sont des nœuds transitifs, et le lock reste à réécrire.

## La moitié analytique est bloquée aussi, pas seulement l'écriture

C'est le piège non évident, et il invalide le repli que le plan avait prévu.

Le plan de mika#2861 traitait trois nœuds comme « sans ambiguïté » et faisait
dépendre la décision sur le quatrième (`webpki-roots 0.26.11`, coquille de
réexport dont `sqlx-core 0.8.6` est le **seul** consommateur — attesté par
`cargo tree -i webpki-roots@0.26.11`) de la lecture du champ
`vulnerabilities.list[].versions.patched` dans `cargo audit --json`. Trois
branches en sortaient, dont une branche de halte qui livrait quand même les
trois montées faciles.

Or `cargo audit` est refusé **et** `~/.cargo/advisory-db` n'existe pas. La plage
`patched` n'est pas lisible. La branche ne peut pas être *choisie*, pas
seulement pas *exécutée*. Le repli « livrer trois avis sur quatre et nommer le
quatrième » est hors d'atteinte par la même porte que le plan principal.

**Leçon : un plan dont la procédure de décision lit un champ dans la sortie
d'une commande refusée n'a pas de mode dégradé.** Le repli hérite du blocage de
la branche nominale. Un plan à branches ne vaut que si la commande qui
discrimine entre les branches est elle-même disponible.

## La leçon de routage, qui est la vraie

Le refus avait **déjà été rencontré pendant le grooming**. Le plan le consigne,
à `docs/plans/2026-10-21-001-…-plan.md` § « Interdit : éditer `Cargo.lock` à la
main » : *« Si `cargo update` est refusé dans le bac à sable — il l'a été dans la
session de grooming (`no matching policy rule`) — arrêter et remonter. »*

Il a été consigné, et le ticket a été dispatché quand même. Le coût est un vol
complet de pilote pour produire une halte que le grooming connaissait déjà.

> Un refus rencontré au grooming est un fait de **routage**, pas une note de bas
> de page. Il ne se range pas en branche de halte à l'intérieur du plan : il
> retire le ticket de la file de dispatch.

### Un précédent atteste une capacité *à sa date*, pas aujourd'hui

Les deux précédents du répertoire ne disent pas la même chose, et c'est là que
se cache l'erreur de lecture :

| PR | date | auteur | lecture |
|---|---|---|---|
| #540 (`rand` 0.9.3) | 2026-04-11 | `app/mika-platform-bot` | le **loop** l'a fait — `cargo update` passait alors |
| #2685 (`quinn-proto` 0.11.19) | 2026-10-13 | `samidarko`, label `pipeline-exempt` | geste **opérateur**, à la main |

Donc la classe n'a pas toujours été une classe opérateur : en avril, un pilote
dispatché montait un lock et ouvrait la PR. La posture du bac à sable s'est
resserrée entre avril et octobre, et le précédent d'octobre est déjà à la main.

> Un précédent atteste qu'une capacité existait **à sa date**. Il n'atteste pas
> qu'elle existe aujourd'hui. Une permission-policy se resserre sans émettre
> d'événement, et sans invalider le document qui cite l'ancien geste.

C'est précisément ce qui rend le préflight non négociable : la question « ce vol
est-il exécutable ? » ne se répond pas en lisant un document de solution, même
récent, même du même répertoire. Elle se répond en une commande, dans le bac à
sable du jour.

## Quoi faire

1. **Préflight, en un tour, en tête de tout plan de bump de lock :**

   ```bash
   cargo update --help
   ```

   Sans effet de bord, et il répond à la seule question qui décide si le vol est
   exécutable. Refusé ⇒ le ticket est opérateur, et le grooming s'arrête là.
   C'est un préflight, pas une branche de halte au § 2 : la différence est qu'un
   préflight tombe **avant** le dispatch.

2. **Faire le bump en geste opérateur**, hors bac à sable — la trajectoire de
   #2685, avec la discipline de diff du document `quinn-proto` (relire
   `git diff Cargo.lock` après chaque commande, révoquer tout hunk que l'avis ne
   réclame pas, confirmer par `cargo metadata --locked`). Pour mika#2861, les
   trois montées sans ambiguïté sont :

   ```bash
   cargo update -p rustls-webpki@0.103.15   --precise 0.103.16
   cargo update -p webpki-root-certs@1.0.9  --precise 1.0.10
   cargo update -p webpki-roots@1.0.9       --precise 1.0.10
   ```

   Le quatrième nœud (`webpki-roots 0.26.11`) demande d'abord de lire
   `versions.patched` de RUSTSEC-2026-0381 dans `cargo audit --json`.

3. **Ou élargir la policy**, si cette classe doit devenir dispatchable : il faut
   `cargo update`, `cargo fetch` et `cargo audit`, plus un clone joignable de
   `rustsec/advisory-db` (déclaré en `deny.toml:5-6`). Ce n'est pas un détail de
   confort : c'est accorder à un pilote le droit de muter le graphe de
   dépendances du dépôt. La décision est à l'opérateur, et le refus actuel
   (`production posture`) se lit comme déjà prise dans l'autre sens.

## Ce que ce bac à sable autorise quand même

Utile à savoir, parce que cela permet d'attester le graphe sans le modifier, et
donc de produire la moitié documentaire d'un tel ticket :

- `cargo tree -i <crate>@<version>` — nommer les consommateurs d'un nœud. **La
  forme qualifiée `@<version>` est obligatoire dès que deux versions d'un nom
  cohabitent dans le lock** (ici `webpki-roots` 0.26.11 et 1.0.9) ; `cargo`
  refuse le `-p webpki-roots` nu comme ambigu.
- `cargo metadata --locked` — attester que le lock se tient.
- La lecture directe de `Cargo.lock`, qui donne les arêtes : une coquille de
  réexport comme `webpki-roots 0.26.11` n'a qu'une dépendance, le
  `webpki-roots 1.0.9` qu'elle réexporte.

Et le rappel du document `quinn-proto`, qui vaut toujours : `cargo audit` lit le
**lockfile**, pas le graphe de build. Une coquille de réexport est donc auditée
sur *sa* version, pas sur celle qu'elle sert — monter le nœud réexporté ne
retire pas la coquille du rapport. C'est pourquoi le quatrième nœud demandait
une décision propre, et non le même geste que les trois autres.

## Seconde occurrence : `cargo deny check` reste invisible

Le job Security enchaîne `cargo audit` puis `cargo deny check`
(`.github/workflows/ci.yml:1072-1084`). Depuis le 2026-10-20, `cargo audit`
tombe sur toute nouvelle tête, donc **`cargo deny check` n'a tourné sur aucune
tête récente** et ses propres échecs ne sont pas visibles. C'est exactement ce
qui s'est produit sur mika#2685, où `unsafe-libyaml` attendait derrière
`quinn-proto` et a demandé son propre ticket (mika#2686).

Deux occurrences : ce n'est plus un accident, c'est un motif. **Quand une étape
d'un job échoue, les étapes suivantes n'ont pas tourné, et leurs échecs ne sont
pas encore visibles.** Ne jamais lire « AC2 reste vert » comme une
non-régression quand l'étape qui la mesure n'a pas tourné : c'est une
**première** vérification, et elle peut découvrir une matière hors périmètre.

> **Suite (2026-10-21) :** les trois montées du § « Quoi faire » ont été faites par un spawn MPC hors bac à sable ; la lecture de `patched` (`>=1.0.10`, aucune `0.26.x`, toutes les `sqlx-core` 0.8.x sur `webpki-roots ^0.26`) place le quatrième nœud en branche B3 : RUSTSEC-2026-0381 reste sur `webpki-roots 0.26.11`.
