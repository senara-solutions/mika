# mika#3027 — une retenue d'ancrage écrite par le moteur traverse la porte mika#2545

Ticket : `senara-solutions/mika#3027` (bug, p1-important, ready, dispatch:loop,
loop-substrate). Source : parking #2983. Exception au gel bornée au 24/11.

## Contexte mesuré

Deux grooms du 2026-11-03 se sont arrêtés sur une **retenue d'ancrage du moteur**
et ont demandé un geste humain pour repartir :

| groom | ticket | `miss_reason` | reprise humaine | issue |
|---|---|---|---|---|
| `80d7aaed` (05:47Z) | #2996 | `QuoteNotInBrief` | `ready` retiré 06:03:40Z, reposé 06:03:42Z | `d17ff234` `PLAN_GROOMED` |
| `7e015156` (11:42Z) | #3022 | `NoAnchorLine` | repose de `ready` à 12:10Z | `1609aadc` `PLAN_GROOMED` |

Les deux ont **convergé dès le groom suivant**. 15 callbacks de groom portent
`Engine reason: F<n>: (BLOCKING) [mika-engine] review-anchor:` depuis le
2026-10-15, dont 7 depuis le 2026-10-20 — c'est la population que chaque reprise
humaine paie aujourd'hui.

Le frein mika#2786, qui existe précisément pour borner cette dépense, a **0 ligne
dans `audit_events` et 0 dans `server.log`**. Il n'a jamais mordu, et il ne peut
pas mordre : la porte mika#2545 refuse dès la **première** retenue sur une reprise
automatique, donc la série du frein ne dépasse jamais 1.

### La cause, lue sur `3693614d`

`crates/mika-agent/src/skills/executor.rs` :

| élément | ligne | lecture |
|---|---|---|
| `groom_escalate_verdict` | 2568 | `Escalated` dès qu'une ligne commence par `Outcome: ESCALATE`, **sans regarder qui l'a écrit** |
| `enum GroomVerdictState` | 2543 | trois états ; le `match` du site (l. 4326) n'a aucun bras `_` |
| porte mika#2545 | 4314 | n'évalue que si `originating_message.is_none()` — donc sur une reprise automatique |
| frein mika#2786 | 4138-4261 | sur `anchor_withheld_streak >= MIKA_GROOM_ANCHOR_BRAKE_MAX` (défaut 3), **en amont** de la porte |
| `anchor_withheld_streak` | 2786 | compte les `result` dont une **ligne** commence par `Engine reason: ` et contient `REVIEW_ANCHOR_WITHHELD_MARKER` |

Un ESCALATE d'architecte et une retenue du moteur rendent le **même** octet au
lecteur de la porte : `Outcome: ESCALATE`. Or ce ne sont pas les mêmes faits. Le
premier est un **jugement terminal** — la porte mika#2545 existe pour lui, et ses
huit rejeux mesurés le 2026-09-26 sont sa justification. Le second est un
**refus de plomberie** : le moteur a retiré une disposition que l'architecte ne
pouvait pas attester, et la mesure ci-dessus dit que la passe suivante converge.

La reconnaissance de l'escalade du moteur existe déjà **trois fois** dans la
plateforme :

1. `arch_disposition.rs` — `Tier::EngineEscalation` (« 0b »), regex
   `ENGINE_ESCALATION_ANCHOR` : `^[[:space:]]*F[0-9]+: \(BLOCKING\) \[mika-engine\] review-anchor:` ;
2. `dispatch-lib.sh::_engine_escalation_line` — le même motif, en `grep -m1 -E`,
   et c'est **l'écrivain** de la ligne `Engine reason: ${engine_line}` ;
3. `executor.rs::anchor_withheld_streak` — `starts_with("Engine reason: ")` +
   `contains(REVIEW_ANCHOR_WITHHELD_MARKER)`, la forme **faible** (pas de `F<n>:`).

La porte, elle, n'en lit aucune.

## Requirements

- **R1.** Sur une reprise automatique (`originating_message.is_none()`), un
  dernier callback de groom dont une **ligne** commence par `Engine reason: ` et
  dont le reste porte la forme fixe de l'escalade du moteur ne doit **pas** être
  refusé par `dispatch_groom_escalated`.
- **R2.** Tout le reste de la population de `Escalated` — et en particulier un
  ESCALATE d'architecte — reste refusé, inchangé.
- **R3.** Le nouvel état est décidé par le compilateur au `match` du site : aucun
  bras `_`.
- **R4.** Aucun deuxième lecteur du marqueur n'est créé. La reconnaissance passe
  par le lecteur **existant** d'`arch_disposition`, et le nombre de sites de
  lecture du préfixe `Engine reason: ` reste à **un**, épinglé par un scan.
- **R5.** Fail-closed sur l'auteur : trois formes qui *ressemblent* à une retenue
  sans en être une restent `Escalated`.
- **R6.** La dépense reste bornée : à la 3e retenue d'affilée le frein mika#2786
  refuse sous `dispatch_groom_anchor_brake`, et aucune reprise automatique ne part.
- **R7.** Le passage est **comptable** : une ligne par évaluation de porte dit
  qu'une retenue a traversé, y compris quand rien ne traverse.

## Design decisions

### D1 — Un quatrième état, et pas un `bool` sur l'existant

`GroomVerdictState` gagne un quatrième variant :

```rust
/// Le dernier callback de groom porte un `Outcome: ESCALATE` que le MOTEUR a
/// écrit en retenant l'attestation d'ancrage (mika#2338), pas un jugement de
/// l'architecte. Une reprise automatique de groom est **autorisée** : la mesure
/// du 2026-11-03 dit que la passe suivante converge, et la dépense est bornée
/// par le frein mika#2786 en amont de cette porte.
AnchorWithheld,
```

Pourquoi un état et non un drapeau sur `Escalated` : la porte est le seul
consommateur aujourd'hui, mais le `verdict` part dans `audit_events` et dans la
sonde, et « ce ticket a escaladé » / « le moteur a retenu une attestation »
appellent **deux lectures opérateur** — la première est le régime attendu non
vide, la seconde est la population qu'AC5 va mesurer. Motif `GroomedState`
(mika#2484 D1), déjà la raison pour laquelle cet `enum` a trois états et pas un
booléen.

La décision se prend **dans** `groom_escalate_verdict`, en raffinement du bras
`Escalated` existant : seul un `result` déjà reconnu comme escalade peut être
reclassé. C'est fail-closed par construction — un texte qui ne porte pas de ligne
`Outcome: ESCALATE` ancrée ne peut atteindre le nouvel état par aucun chemin, quel
que soit ce qu'il écrit sur `Engine reason:`.

```rust
pub(crate) fn groom_escalate_verdict(result: Option<String>) -> GroomVerdictState {
    match result {
        Some(text) => {
            if !text.lines().any(|line| line.starts_with(GROOM_ESCALATE_MARKER)) {
                GroomVerdictState::NotEscalated
            } else if carries_engine_anchor_withholding(&text) {
                GroomVerdictState::AnchorWithheld
            } else {
                GroomVerdictState::Escalated
            }
        }
        None => GroomVerdictState::Unreadable(/* inchangé */),
    }
}
```

### D2 — Le lecteur : UN site, et il délègue au lecteur existant d'`arch_disposition`

R4 interdit un second lecteur. Le prédicat par `result` est **extrait** de
`anchor_withheld_streak` vers une fonction nommée, qui devient le site unique :

```rust
/// Ce `result` porte-t-il une retenue d'ancrage écrite par le MOTEUR ?
/// (mika#3027)
///
/// **Site unique** de lecture de [`GROOM_ENGINE_REASON_PREFIX`], épinglé par
/// `canonical_tokens::mika3027_la_retenue_dancrage_a_un_seul_lecteur`. Deux
/// orthographes de ce prédicat, c'est le frein mika#2786 et la porte mika#2545
/// qui comptent deux populations différentes sous un seul nom.
///
/// Deux termes, et chacun est une décision :
///
/// - **`strip_prefix` sur une LIGNE**, jamais un `contains` sur le texte : la
///   prose d'un pilote cite ce préfixe en parlant du mécanisme — le faux positif
///   mesuré sur le Signal S (mika#2050) et sur les 651 détections d'`instr`
///   (mika#2105). `_escalate_groom` écrit la vraie ligne en tête de ligne.
/// - **La forme fixe est lue chez son propriétaire.** Le reste de la ligne est
///   passé à [`crate::arch_disposition::is_engine_escalation_line`], la
///   transcription Rust de `dispatch-lib.sh::_engine_escalation_line` qui sert
///   déjà le tier 0b. Une prose qui écrit `Engine reason: quelque chose` sans la
///   numérotation `F<n>:`, `(BLOCKING)`, `[mika-engine]` et `review-anchor:` ne
///   passe pas.
pub(crate) fn carries_engine_anchor_withholding(text: &str) -> bool {
    text.lines().any(|line| {
        line.strip_prefix(GROOM_ENGINE_REASON_PREFIX)
            .is_some_and(crate::arch_disposition::is_engine_escalation_line)
    })
}
```

et `anchor_withheld_streak` l'**appelle** au lieu de porter sa propre copie :

```rust
let withheld = carries_engine_anchor_withholding(text);
```

`arch_disposition` expose un prédicat ligne à ligne — pas le regex, pas la
`LazyLock` :

```rust
/// Cette ligne est-elle une ligne d'escalade du moteur ? (mika#2338)
///
/// Le prédicat de [`engine_escalation_line`], exposé au module voisin pour
/// mika#3027 : la porte mika#2545 doit distinguer une retenue du moteur d'un
/// ESCALATE d'architecte, et la forme de référence est **celle-ci**, pas une
/// seconde transcription du `grep -m1 -E` du bash. Interrogation ligne à ligne,
/// pour la raison écrite sur [`ENGINE_ESCALATION_ANCHOR`].
pub(crate) fn is_engine_escalation_line(line: &str) -> bool {
    ENGINE_ESCALATION_ANCHOR.is_match(line)
}
```

**Conséquence voulue : le compte de lecteurs baisse de trois à deux.**
`crate::agent_loop::REVIEW_ANCHOR_WITHHELD_MARKER` n'avait qu'un consommateur —
ce prédicat — et sa raison d'être documentée est *« the same marker, for the
anchor brake in `skills::executor` (mika#2786) »*. Il est **retiré**, et le
`const` privé `REVIEW_ANCHOR_ENGINE_FINDING_MARKER` qu'il aliasait reste intact,
sur une seule ligne, donc le `grep` d'une ligne de `test-dispatch-lib.sh` qui lit
cette déclaration n'est pas touché. La retenue d'ancrage se lit désormais dans
`arch_disposition` (Rust) et dans `dispatch-lib.sh` (shell, l'écrivain), et
`test-dispatch-lib.sh` compare déjà ces deux-là.

**Resserrement assumé du frein.** L'ancien prédicat acceptait
`Engine reason: (BLOCKING) [mika-engine] review-anchor: …` sans numérotation
`F<n>:`. Le nouveau l'exige. Ce n'est pas un élargissement — c'est un
rétrécissement strict, vers la forme que l'écrivain produit réellement
(`_escalate_groom` interpole la sortie de `_engine_escalation_line`, qui *est* la
ligne `F<n>:` complète). Le fixture `WITHHELD` relevé sur mika#2762 porte
`Engine reason: F1: (BLOCKING) …` et reste reconnu ; un test nouveau dit le
rétrécissement plutôt que de le laisser implicite.

### D3 — Le bras du site : laisser passer, et dire pourquoi sur place

Au `match` de la l. 4326, un quatrième bras qui rend `None` (aucun refus) :

```rust
// mika#3027 — une retenue d'ancrage du MOTEUR n'est pas un jugement de
// l'architecte, et la porte ne doit pas les confondre.
//
// POURQUOI LAISSER PASSER. mika#2545 refuse le rejeu parce qu'« il ne
// change pas le verdict » : un ESCALATE d'architecte est un jugement sur le
// plan, et huit rejeux mesurés le 2026-09-26 n'en ont pas déplacé un. Une
// retenue d'ancrage est l'inverse — le moteur a retiré une disposition qu'il
// n'a pas pu attester, et les deux reprises humaines du 2026-11-03 (#2996,
// #3022) ont convergé dès le groom SUIVANT. Refuser ici, c'est facturer à
// l'opérateur un retrait/repose de `ready` pour obtenir ce que la passe
// suivante aurait rendu.
//
// LA DÉPENSE RESTE BORNÉE, et pas par ce bras : le frein mika#2786, en
// amont de cette porte dans cette même fonction, refuse à la 3e retenue
// d'affilée sous `dispatch_groom_anchor_brake`. Ce bras est ce qui rend ce
// frein ATTEIGNABLE — la série ne pouvait pas dépasser 1 tant que cette
// porte refusait la première retenue (0 ligne de frein mesurée depuis le
// 2026-10-18).
GroomVerdictState::AnchorWithheld => None,
```

Pas de `_ =>` : R3 est tenu par le scan existant
`canonical_tokens::mika2545_the_verdict_match_has_no_wildcard_arm`, **amendé**
(voir D5).

### D4 — La sonde : un cinquième mot sur la ligne qui existe déjà, pas un événement de plus

`groom_verdict_state` émet déjà `groom_verdict_selected` à **chaque** lecture, sur
toutes ses branches, avec un champ `verdict`. Le quatrième état y ajoute un mot :

```rust
pub(crate) const GROOM_VERDICT_ANCHOR_WITHHELD: &str = "anchor_withheld";
```

**Délibérément pas une entrée d'`ALL_GROOM_ESCALATE_VERDICTS`**, exactement comme
`GROOM_VERDICT_NOT_ESCALATED` : cette table énumère les motifs de **refus** tels
qu'ils atterrissent dans `audit_events.after_value`, sa cardinalité de 2 est un
format de fil asserté par `mika2545_the_two_refusal_verdicts_are_a_wire_format`,
et une retenue qui traverse ne refuse rien et n'écrit rien là.

Aucun événement nouveau. `groom_verdict_selected` avec `verdict=anchor_withheld`
est **une ligne par évaluation de porte sur une reprise automatique**, c'est-à-dire
exactement le numérateur d'AC5 ; et son homologue `verdict=escalated` reste le
contrôle positif qui sépare « la porte a laissé passer une retenue » de « aucune
reprise automatique n'est passée par là » (classe mika#2205). Son **absence**
signifie un binaire antérieur à ce vol (classe mika#2340), jamais zéro retenue —
l'instrument n'est pas vivant avant `make deploy`.

### D5 — Le détecteur, et le scan existant qu'il faut amender

**Le scan nouveau (R4, AC2).** `canonical_tokens.rs`, à côté des autres scans de
nom, motif `mika2772_le_drapeau_de_forcage_a_un_seul_ecrivain_et_un_seul_lecteur`
(l. 17641) :

`mika3027_la_retenue_dancrage_a_un_seul_lecteur` énumère `production_sources()`,
retire les lignes de commentaire via `source_scan::strip_comment_lines`, densifie
(suppression des blancs, pour survivre à un reflow rustfmt) et compte les
occurrences du nom `GROOM_ENGINE_REASON_PREFIX`. Trois assertions :

1. **anti-vacuité sur la cardinalité** : `executor.rs` porte exactement **2**
   occurrences — la déclaration du `const` et l'unique usage dans
   `carries_engine_anchor_withholding`. Zéro = le scan vise un nom mort (`const`
   renommé ou retiré) et se lirait comme un arbre sain ; plus de 2 = un second
   lecteur ;
2. **anti-vacuité sur le scan lui-même** : `sources.len() > 200` ;
3. **aucun autre fichier** ne nomme le préfixe, allowlist
   `ENGINE_REASON_READER_ALLOWLIST` **livrée vide**.

Le message d'échec nomme la résolution : router le site par le lecteur, ne pas
ajouter d'entrée (doctrine mika#2201).

**Le scan existant à amender.** `mika2545_the_verdict_match_has_no_wildcard_arm`
(l. 4713) extrait la plage du `match` par `.lines().take(30)` et assure sa bonne
foi en vérifiant que les **trois** noms de bras y sont. Le `match` fait 25 lignes
aujourd'hui ; le quatrième bras et son commentaire le portent au-delà de 30, donc
la fenêtre passe à `.take(45)` et la conjonction de bonne foi gagne
`GroomVerdictState::AnchorWithheld`. Sans cet amendement le scan vérifierait
l'absence de joker sur une plage tronquée — un faux vert, et c'est la classe
exacte de défaut que ce ticket corrige ailleurs.

### D6 — Les quatre formes d'AC4, et pourquoi chacune a son test

| forme | prédicat qui refuse | issue |
|---|---|---|
| aucune ligne `Engine reason:` (ESCALATE d'architecte) | `strip_prefix` ne rend `Some` sur aucune ligne | `Escalated` |
| `Engine reason:` cité au milieu d'une ligne de prose | `strip_prefix` est sur la **ligne**, pas sur le texte | `Escalated` |
| ligne `Engine reason: ` suivie d'autre chose que `F<n>: (BLOCKING) [mika-engine] review-anchor:` | `is_engine_escalation_line` | `Escalated` |
| la forme complète, relevée sur mika#2762 | les deux termes tiennent | `AnchorWithheld` |

La quatrième est le **témoin positif** : sans elle, un prédicat qui rend toujours
`false` passerait les trois premières et le vol serait un no-op vert.

## Ordre de commit

1. `fix(3027): arch_disposition expose son prédicat de ligne d'escalade du moteur`
   — `is_engine_escalation_line` + doc. Compile seul, aucun consommateur encore.
2. `fix(3027): la retenue d'ancrage a un lecteur unique, qui délègue au tier 0b`
   — `carries_engine_anchor_withholding`, `anchor_withheld_streak` qui l'appelle,
   retrait de `REVIEW_ANCHOR_WITHHELD_MARKER`, le test du rétrécissement. Suite
   mika#2786 verte.
3. `fix(3027): une retenue d'ancrage se décide dans le match de la porte`
   — le variant `AnchorWithheld`, le raffinement de `groom_escalate_verdict`, le
   mot de fil, le bras du site, les tests AC1 et AC4.
4. `fix(3027): la serie 1-2-3 atteint le frein mika#2786`
   — le test de bout en bout AC3.
5. `test(3027): le lecteur de la retenue d'ancrage est epingle par un scan`
   — le scan nouveau + l'amendement du scan mika#2545.

Plancher de rebasage : `6de5d4dc0`.

## Verification Contract

| quoi | commande |
|---|---|
| suite du module | `cargo test -p mika-agent skills::executor::tests::mika3027` |
| non-régression du frein | `cargo test -p mika-agent mika2786` |
| non-régression de la porte | `cargo test -p mika-agent mika2545 mika2795` |
| les scans | `cargo test -p mika-agent canonical_tokens` |
| tier 0b | `cargo test -p mika-agent arch_disposition` |
| suite complète | `cargo test` |
| lints | `cargo clippy --all-targets -- -D warnings` && `cargo fmt --check` |
| gardes du dépôt | `scripts/check-secrets.sh`, `make verify-bundled-skills` |
| pipeline | `scripts/verify-pipeline.sh origin/main` |
| accord shell/Rust | `skills/bundled/_shared/test-dispatch-lib.sh` |

**Bonne foi du scan nouveau** : avant d'ouvrir la PR, vérifier à la main qu'il
rougit — renommer `GROOM_ENGINE_REASON_PREFIX` en local doit faire tomber
l'assertion de cardinalité (et non passer en silence), et ajouter un second usage
doit faire tomber l'assertion d'allowlist. Les deux gestes sont annulés avant
commit.

## Definition of Done

- Les cinq commits sont sur la branche, la suite complète est verte, clippy et
  fmt sont propres.
- `REVIEW_ANCHOR_WITHHELD_MARKER` n'existe plus et aucun `dead_code` n'est émis.
- Le `match` du site porte quatre bras et aucun `_`.
- `ALL_GROOM_ESCALATE_VERDICTS` fait toujours 2 entrées.
- `scripts/verify-pipeline.sh origin/main` passe.
- Le corps de PR porte `Closes #3027`, le tableau des AC et la ligne
  `Code review: delegated to mika-qa (mika#2694)`.
- AC5 est **ouvert**, nommé comme tel dans le corps de PR : c'est une mesure
  post-déploiement sur les 5 retenues suivantes, et son instrument
  (`groom_verdict_selected` avec `verdict=anchor_withheld`) n'est pas vivant avant
  `make deploy`.

## Acceptance criteria

Bornes posées par Prime et acceptées par Vincent le 2026-11-03 : AC1, AC3 et AC4.

- **AC1.** Sur une reprise automatique, la porte mika#2545 laisse passer un seul
  cas : un dernier callback de groom dont une ligne commence par
  `Engine reason: ` et porte le marqueur de retenue d'ancrage. Le même test, dans
  le même appel, porte le contrôle négatif : le même callback sans ligne
  `Engine reason:`, c'est-à-dire un ESCALATE de l'architecte, reste refusé par
  `dispatch_groom_escalated`.
- **AC2.** Le nouvel état se décide dans le `match` du site, sans bras `_`. Aucun
  deuxième lecteur du marqueur n'est créé : la reconnaissance passe par un lecteur
  existant ou par un seul nouveau, épinglé par un scan.
- **AC3.** À la 3e retenue d'affilée (`MIKA_GROOM_ANCHOR_BRAKE_MAX`, défaut 3), le
  frein mika#2786 refuse avec `dispatch_groom_anchor_brake`, et aucune reprise
  automatique ne part. Le test va de bout en bout sur la fonction de la porte,
  avec la série 1, 2, 3.
- **AC4.** Fail-closed sur l'auteur. Le résultat reste `Escalated` dans trois
  cas : la ligne `Engine reason:` est absente ; `Engine reason:` est cité au
  milieu d'une ligne de prose et ne la commence pas ; la ligne qui suit
  `Engine reason: ` n'a pas la forme fixe
  `F<n>: (BLOCKING) [mika-engine] review-anchor:`. Un test par cas, plus un
  témoin positif.
- **AC5.** Sonde : on relève, sur les 5 retenues d'ancrage suivantes, celles qui
  ont été reprises par un geste humain (retrait et repose de `ready`, ou
  `mika ask … groom`), et on l'écrit ici. La cible est 0, hors morsures du frein.

### Comment chaque AC est tenu

| AC | livrable | test |
|---|---|---|
| AC1 | D1 + D3 | `mika3027_une_retenue_traverse_la_porte_un_escalate_darchitecte_non` — un seul `#[tokio::test]`, `seeded(WITHHELD, 1)` puis `seeded(ESCALATED, 1)`, le premier non refusé sous `GROOM_ESCALATED_ERROR`, le second refusé avec `"verdict": "escalated"` |
| AC2 | D2 + D5 | `mika3027_la_retenue_dancrage_a_un_seul_lecteur` (scan, allowlist vide) + `mika2545_the_verdict_match_has_no_wildcard_arm` amendé |
| AC3 | D3 (le bras est ce qui rend le frein atteignable) | `mika3027_la_serie_1_2_3_atteint_le_frein` — `seeded(WITHHELD, n)` pour n ∈ {1, 2, 3} via `validate_dispatch_readiness`, les deux premiers sans `GROOM_ANCHOR_BRAKE_ERROR`, le troisième avec, plus une ligne d'audit |
| AC4 | D1 + D2 | quatre tests purs sur `groom_escalate_verdict` : `…_sans_ligne_engine_reason_reste_escalated`, `…_un_prefixe_cite_en_prose_reste_escalated`, `…_une_forme_non_conforme_reste_escalated`, `…_la_forme_complete_est_une_retenue` |
| AC5 | D4 | aucune ligne de code — mesure post-déploiement, instrument déjà livré |

**Protocole d'AC5**, écrit ici pour qu'il ne soit pas réinventé : sur les 15 jours
suivant le déploiement, `grep groom_verdict_selected server.log` filtré sur
`verdict=anchor_withheld` donne le dénominateur (les retenues qui ont traversé) ;
pour chacune, `gh issue view <n> --json timelineItems` dit s'il y a eu un
`unlabeled`/`labeled` de `ready` ou un `mika ask … groom` après le callback. Les
morsures du frein sont exclues en soustrayant les lignes
`groom_anchor_withheld_brake` d'`audit_events` sur la même fenêtre.

## Fire-Disposition

Ce vol livre des détecteurs : les onze tests ci-dessus et, surtout, le **scan de
source** `mika3027_la_retenue_dancrage_a_un_seul_lecteur`, dont le chemin de
succès est « aucun second lecteur trouvé ».

**Option retenue : (a) — exception nommée en allowlist, allowlist LIVRÉE VIDE.**

- La constante `ENGINE_ESCALATION_READER_ALLOWLIST: &[&str] = &[]` existe, est
  grep-visible, et porte en doc-comment la raison de son vide : la cible du scan
  (`GROOM_ENGINE_REASON_PREFIX` avec un lecteur unique) est **créée par ce vol**,
  donc il n'existe aucune violation préexistante à excepter. Une allowlist non
  vide signifierait qu'un second lecteur a été admis.
- **Assertion auto-nettoyante**, dans le même test :
  `assert!(ENGINE_ESCALATION_READER_ALLOWLIST.is_empty(), …)` — l'exception rougit
  dès qu'elle cesse d'être vide, donc le geste qui rend l'exception inutile est le
  geste qui la fait expirer. C'est ce qui l'empêche de devenir l'allowlist que
  mika#2309 condamne.
- Le message d'échec du scan **refuse explicitement** l'ajout d'une entrée et
  nomme la résolution : router le site par `carries_engine_anchor_withholding`
  (doctrine mika#2201 — on retire, on n'allowliste pas). Ticket de suivi : aucun,
  parce qu'il n'y a rien à suivre — zéro violation au décollage.
- Les tests ne sont pas désarmés : rien n'est livré en `#[ignore]`, et le détecteur
  est armé en CI dès le merge.

Le scan vit dans `canonical_tokens.rs`, module déjà atteint par `cargo test`
et donc par le job `Check` de `ci.yml` — aucune entrée à
`scripts/suite-wiring.tsv` n'est requise (ce n'est pas une suite sous `scripts/`
ni sous `skills/bundled/_shared/`).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/arch_disposition.rs` (`is_engine_escalation_line` + doc) | 12 |
| `crates/mika-agent/src/agent_loop/mod.rs` (retrait de l'alias + doc du `const` restant) | 10 |
| `crates/mika-agent/src/skills/executor.rs` — production (variant, raffinement, lecteur unique, mot de fil, bras du site) | 105 |
| `crates/mika-agent/src/skills/executor.rs` — tests (AC1, AC3, AC4 ×4, rétrécissement du frein) | 150 |
| `crates/mika-agent/src/canonical_tokens.rs` (scan nouveau + amendement du scan mika#2545) | 80 |

Total estimé : 357 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 357 estimées contre 429 : la marge
est de 17 %. Le précédent sur le même lecteur, mika#2795 (#2803), a fait +776 −121
— mais il réécrivait la **requête** et la **sélection de ligne**, c'est-à-dire la
moitié base du lecteur, et livrait quatre cas de sélection avec leurs compteurs.
Ce vol ne touche ni le SQL ni la sélection : il ajoute un variant à un `enum` de
trois, raffine un `if` de 17 lignes, et **déplace** un prédicat existant dans une
fonction nommée. Les deux fonctions visées font 17 et 25 lignes. La seule pièce
réellement nouvelle est le scan, dont le motif est recopié d'un voisin du même
fichier (l. 17624-17697).

Le poste le plus lourd est les tests (150) et il est irréductible : AC4 exige
quatre cas plus un témoin, AC1 un contrôle négatif dans le même appel, AC3 une
série de trois en bout en bout. Les deux harnais (`seeded`/`dispatch` de
`mika2786_anchor_brake`, `seeded` de `mika2545_escalate_terminal`) **existent
déjà** et sont réutilisés tels quels — c'est ce qui garde ce poste sous 200.

## Hors vol

Nommés par le ticket, et aucun n'est requis par les AC :

- **La tolérance du lecteur d'ancres pour `**A1** (…) :`**
  (`review_anchor.rs` l. ~805 : `trim_start_matches('*')` puis
  `strip_prefix("A1:")`). C'est la cause du `NoAnchorLine` de #3022 en amont ;
  élargir ce lecteur réduirait la *fréquence* des retenues, là où ce vol en change
  la *conséquence*. Deux axes, deux vols.
- **Un exemple de ligne d'ancre dans le texte de relance**
  (`agent_loop/mod.rs` l. ~4777) — même raison : c'est de la pédagogie de prompt,
  et un prompt ne porte pas de garde.
- **La porte mika#2545 pour un ESCALATE d'architecte** : inchangée, et R2 est ce
  qui l'atteste.
- **Le seuil du frein** (`GROOM_ANCHOR_BRAKE_DEFAULT = 3`) : il devient atteignable
  pour la première fois par ce vol, donc il n'a jamais été mesuré en production.
  Le recalibrer avant la première mesure serait un réglage sur une intuition.

`Tracked in:` — sans objet : aucun périmètre n'est renvoyé à une suite, les quatre
points ci-dessus sont des axes distincts, pas la queue d'un découpage.
