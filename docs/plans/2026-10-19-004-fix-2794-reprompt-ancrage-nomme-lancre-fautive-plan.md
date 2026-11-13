---
title: "mika#2794 — le re-prompt d'ancrage nomme l'ancre qui échoue, et la mesure qui le juge"
type: fix
status: active
date: 2026-10-19
issue: senara-solutions/mika#2794
---

# mika#2794 — le re-prompt d'ancrage nomme l'ancre qui échoue, et la mesure qui le juge

## Pourquoi

La garde d'ancrage de mika#2037 retient une passe d'architecte dont les lignes
`A1:`…`A3:` ne citent pas le brief du tour. Après mika#2786 (frein à trois retenues +
tolérance `**A1:**`), la retenue qui reste est toujours la même forme : **deux ancres sur
trois sont valides, et la manquante est la première**, qui cite la phrase d'ouverture du
corps du ticket lu par `gh_read` au lieu du brief.

Le rejeu hors ligne du matcher sur les briefs (`messages`) et réponses (`llm_calls`) réels
du 2026-10-18 (table du ticket) :

| session arch | ticket | ancres trouvées / valides |
|---|---|---|
| `49ffa230` | mika#2762 | 3 / 2, puis 3 / 2 après re-prompt |
| `a7f1ccad` | mika#2780 | 3 / 2 aux trois réponses |
| `f167ff53` | mika#2762 | 3 / 1, puis 3 / 0 |

Le re-prompt correctif actuel (`agent_loop/mod.rs:4138-4165`) nomme le `miss_reason` et
redemande le contrat **en entier**. Il ne dit pas **quelle** ligne a échoué, ni que les
deux autres sont acceptées. Le modèle réécrit donc les trois, et `a7f1ccad` a rendu
**trois fois la même A1 fautive**.

L'information manque parce que le matcher ne la retourne pas : `verify_review_anchors`
agrège en `anchors_found` / `anchors_valid` / `reason` et jette l'identité des lignes —
le `filter_map` de `review_anchor.rs:253-261` ne garde même pas le préfixe qui a identifié
la ligne.

### Ce que ce vol est, et ce qu'il n'est pas

C'est **un pari sur l'obéissance du modèle**, et le ticket le dit. Aucun test hors ligne
ne le mesure : un test prouve que le re-prompt *porte* l'information, jamais que le modèle
la suit. Ce vol livre donc deux choses indissociables — le re-prompt nommé **et
l'instrument qui le juge** (D3), dont la lecture avant/après est un geste d'opérateur sur
une fenêtre de déploiement, pas une assertion de CI.

## Décisions

### D1 — Le matcher retourne « quelle ligne », et rien d'autre

`verify_review_anchors` garde ses seuils, son verbatim, sa dispersion, son dédoublonnage,
sa fenêtre de corps et son fail-closed. Le seul changement est le retour d'une liste
par ligne d'ancrage, dans l'ordre du corps :

```rust
/// One anchor line's fate, in body order.
pub(crate) struct AnchorOutcome {
    /// The declared prefix that identified the line (`A1:`) — never the `**` the model
    /// may have wrapped it in (mika#2786 strips those before the prefix is looked for).
    pub(crate) label: String,
    /// Start of the line's normalized content, truncated to `ANCHOR_EXCERPT_CHARS`.
    pub(crate) excerpt: String,
    /// `None` when the line claimed a region of the brief; else what blocked it.
    pub(crate) refused: Option<AnchorMissReason>,
}
```

porté par `AnchorVerdict::Missing { anchors_found, anchors_valid, reason, per_anchor }`.

Quatre points de conception, chacun parce que l'inverse se devine mal :

- **`Satisfied` ne porte rien de plus.** Le re-prompt n'existe pas sur ce chemin, et un
  champ qu'aucun site ne lit est un second producteur d'une information déjà agrégée.
- **La liste ne peut pas dériver des compteurs.** `per_anchor.len() == anchors_found` et
  `per_anchor.iter().filter(|a| a.refused.is_none()).count() == anchors_valid` sont
  **asserties** (U1). Les compteurs sont déjà portés par les deux lignes de journal et par
  la ligne de constat de l'ESCALATE (`escalate_unattested_disposition`) ; deux populations
  sous un même nom ne sont pas soustractables (motif mika#2205).
- **Les deux retours anticipés rendent une liste vide**, et c'est le bon état :
  `NoAnchorLine` (aucune ligne n'est une ancre : il n'y a rien à nommer) et le brief
  normalisé vide (inatteignable en production — `min_brief_chars` garde le site — mais
  fail-closed s'il l'était). Le re-prompt retombe alors **mot pour mot** sur sa forme
  actuelle (D2, contrôle négatif).
- **L'extrait est tronqué à `ANCHOR_EXCERPT_CHARS = 72` caractères**, indexés en `chars`
  et jamais en octets (`scripts/check-byte-slices.sh` ; les briefs de ce dépôt sont
  français). Un `…` est ajouté **seulement** si la ligne était plus longue, pour que
  « tronqué » et « court » ne rendent pas les mêmes octets. 72 > `min_quote_chars` (40) :
  l'extrait suffit toujours à reconnaître la ligne et à voir où la citation a dérivé.

`escalate_unattested_disposition` et la ligne de constat `REVIEW_ANCHOR_ENGINE_FINDING_MARKER`
sont **inchangés** : `dispatch-lib` les lit par forme fixe, et le diagnostic appartient au
re-prompt, pas au rapport d'échec.

### D2 — Le re-prompt nomme la refusée, et garde les acceptées

Une fonction pure dans `agent_loop/mod.rs` :

```rust
fn format_anchor_diagnosis(per_anchor: &[review_anchor::AnchorOutcome]) -> Option<String>
```

`None` sur liste vide — le re-prompt est alors celui d'aujourd'hui, inchangé. Sinon, inséré
**après** la ligne `What blocked it: {reason}` et **avant** le rappel du contrat, qui reste
entier (le modèle a besoin des seuils pour écrire la ligne de remplacement) :

1. **Les acceptées, nommées, à garder.** `Anchors accepted: A2, A3 — keep them exactly as
   written; do not rewrite them.` C'est le geste que le re-prompt actuel ne demande pas et
   qui explique la boucle : redemander le contrat entier fait tout réécrire, et la même
   erreur revient. Sur la forme mesurée, 2 des 3 lignes étaient bonnes.
2. **Les refusées, une par ligne, bornées à `MAX_DIAGNOSED_ANCHORS = 3`** :
   `A1: "<extrait>…" — refused: <reason.describe()>`. La borne tient le re-prompt sous une
   taille fixe quand le contrat autorise dix préfixes.
3. **La phrase sur l'origine, conditionnelle et gated sur `QuoteNotInBrief` :** « cette
   ligne ne se trouve pas dans le brief de ce tour ; si elle vient d'un document que tu as
   lu avec un outil ce tour-ci — le corps du ticket via `gh_read`, un fichier — ce n'est
   pas le document en revue. Le brief est le message utilisateur de ce tour ; c'est lui, et
   lui seul, que l'ancre doit citer. »

   **Au conditionnel, parce que le moteur ne sait pas d'où vient la ligne** — il ne compare
   qu'au brief. Affirmer l'origine serait une affirmation non fondée, celle-là même que le
   dépôt interdit à ses agents (grounding rule, root `CLAUDE.md`). **Gated sur
   `QuoteNotInBrief`**, parce que la dire sur un `OverlappingRegions` enverrait le mauvais
   correctif — c'est le motif déjà écrit au site qui choisit `reason`
   (`review_anchor.rs:319-329`).

Un seul site de formatage, appelé une fois. La garde reste fail-closed : rien ne change
dans le chemin de la seconde retenue.

### D3 — L'instrument qui juge le pari

Le ticket exige une mesure avant/après sur des passes réelles : **le nombre de retenues
après re-prompt, rapporté au nombre de re-prompts**. Les deux lignes existent déjà et sont
exactement ce ratio — une par re-prompt, une par retenue :

| ligne | niveau | cardinalité |
|---|---|---|
| `event = "guard.review_anchor"` | WARN | une par re-prompt correctif (dénominateur) |
| `event = "guard.review_anchor_withheld"` | ERROR | une par retenue après re-prompt (numérateur) |

Elles gagnent **deux champs**, les mêmes sur les deux : `refused_labels` et
`accepted_labels` (labels joints par `,` — un champ `tracing` porte un scalaire). C'est ce
qui rend la mesure lisible **par classe d'ancre** : la thèse du ticket est « c'est toujours
la première », et seul ce champ la confirme ou la réfute en production.

Trois propriétés de lecture :

- **Vide est une chaîne vide, jamais un champ absent.** L'**absence** du champ signifie un
  binaire antérieur à ce correctif (classe mika#2340), jamais « aucune ancre refusée ».
- **Aucune sonde nouvelle.** Une troisième ligne serait un second producteur du même
  nombre ; la cardinalité des deux existantes *est* le ratio.
- **L'instrument n'est pas vivant avant `make deploy`** (mika#2340) : le journal est écrit
  par le binaire servi, pas par le checkout.

Protocole, à lire sur `$MIKA_SPIRIT_LOG_FILE` (`/var/log/mika/server.log`) :

```sh
jq -r 'select(.event == "guard.review_anchor" or .event == "guard.review_anchor_withheld")
       | [.timestamp, .event, .refused_labels, .accepted_labels, .miss_reason] | @tsv' \
  < /var/log/mika/server.log
```

- dénominateur = lignes `guard.review_anchor` ; numérateur = lignes `guard.review_anchor_withheld`
- **mesure « avant », déjà acquise** (rejeu hors ligne du ticket, fenêtre 2026-10-18) :
  3 retenues sur 3 re-prompts, soit un ratio de **1,0** ; l'ancre refusée est la première
  dans 2 des 3 sessions
- la **décision** au vu de la mesure « après » (garder, renforcer, ou passer à
  l'attribution déterministe ci-dessous) est un geste d'opérateur sur une fenêtre de
  déploiement. Ce vol ne la prend pas et n'ouvre pas de ticket pour elle : un ticket ouvert
  sur une hypothèse non mesurée est précisément ce que mika#2786 a refusé de faire pour
  cette piste-là.

### Pistes écartées

- **Attribuer l'origine de façon déterministe** — chercher l'extrait refusé dans les blocs
  de résultat d'outil du tour, pour pouvoir dire « ton A1 cite la sortie de `gh_read` »
  plutôt que de le supposer. C'est la version **fondée** de la phrase de D2, et elle serait
  meilleure. Hors de ce vol pour trois raisons : elle ouvre un second accès au site (les
  `tool_result` de `request.messages`), elle met un budget de comparaison verbatim sur des
  sorties d'outil qui atteignent les centaines de kilo-octets, et elle serait un **deuxième
  pari dans un vol dont le premier n'est pas encore mesuré**. C'est la suite naturelle si
  D3 rend un ratio inchangé.
- **Relâcher la garde de mika#2037** — interdit par le brief, et c'est la faille qu'elle
  ferme.
- **Une consigne de plus dans `mika-arch-*/system_prompt.md`** — écartée par mika#2786 et
  la raison tient (`feedback_prompt_enforcement_fragile.md`) : la consigne existe déjà
  (*What does NOT satisfy the contract*) et le modèle ne la tient pas. Le re-prompt moteur
  n'est pas une consigne de plus : il arrive **après** l'échec, et il porte la mesure de
  cet échec-là.
- **Changer de modèle** — interdit par le brief (mika#1190). Aucun modèle ne bouge dans ce
  vol, donc `make calibrate-mika-arch` n'est pas une précondition ; le scénario
  `required_review_anchor` de la suite mika-arch fait un seul appel et ne traverse pas le
  re-prompt.

## Unités

### U1 — `per_anchor` dans le matcher

`crates/mika-agent/src/agent_loop/review_anchor.rs` :

- `AnchorOutcome` + `ANCHOR_EXCERPT_CHARS` + doc-comment portant les quatre points de D1.
- Le `filter_map` de collecte garde le couple `(préfixe, reste)` :
  `find_map(|p| trimmed.strip_prefix(p.as_str()).map(|rest| (p.as_str(), rest)))`.
- La boucle de vérification pousse un `AnchorOutcome` par ligne — `refused: None` sur
  `QuoteSearch::Found`, sinon la raison de cette ligne (le doublon détecté par `seen_lines`
  prend `OverlappingRegions`, comme le compteur aujourd'hui).
- `AnchorVerdict::Missing` gagne `per_anchor`.
- `crates/mika-agent/src/calibration/roles/mika_arch.rs:1032` — le `match` y est exhaustif :
  un `..` (une ligne). Aucun autre site ne détruit `Missing` hors tests.

### U2 — Le diagnostic dans le re-prompt

`crates/mika-agent/src/agent_loop/mod.rs` :

- `format_anchor_diagnosis` (D2) + `MAX_DIAGNOSED_ANCHORS`, avec son doc-comment.
- Le `format!` du re-prompt (ligne ~4140) reçoit le diagnostic entre `What blocked it:` et
  le rappel du contrat ; chaîne vide quand `None`.
- `refused_labels` / `accepted_labels` sur `guard.review_anchor` (WARN, ~4115) et
  `guard.review_anchor_withheld` (ERROR, ~4197).

### U3 — Le rejeu de la forme mesurée, bout en bout

`crates/mika-agent/tests/eval/grounding_regressions/review_anchor.rs` — un scénario sur le
harnais existant, avec une fixture où **A1** est la fautive (la phrase d'ouverture d'un
corps de ticket, A2/A3 verbatim sur régions disjointes). Via `harness.mock_provider`
→ `captured_requests()`, il assert que le **second** `LlmRequest` porte un message
utilisateur nommant `A1`, son extrait, et les acceptées `A2`/`A3` ; et que le fail-closed
n'a pas bougé (seconde réponse non corrigée ⇒ `Disposition: ESCALATE`).

### U4 — Documentation

- Le doc-comment de `review_anchor.rs` nomme `per_anchor` et la mesure qui l'a motivé.
- `docs/agent-internals/agent-loop.md` — le protocole de D3 (les deux événements, les deux
  champs, la requête, la mesure « avant », et l'absence de champ = binaire antérieur).

## Fire-Disposition

**Option (a) — exception nommée en allowlist, allowlist vide par construction.**

Ce vol livre des détecteurs au sens de mika#1574 : les tests et assertions de U1/U2/U3.
Détail d'implémentation :

- **Ils atterrissent armés.** Aucun `#[ignore]`, aucun `#[cfg(skip)]`, aucune ligne
  désactivée : ils doivent être verts au premier `cargo test -p mika-agent`.
- **L'allowlist est vide, et elle l'est par construction, pas par choix.** Aucun livrable
  n'inspecte un corpus préexistant — pas de règle de lint, pas de garde CI, pas de scan
  structurel sur `src/**`. Les assertions portent sur le retour d'une fonction appelée avec
  ses propres entrées, et sur un `LlmRequest` produit par le harnais du test. Il n'existe
  donc **aucune violation existante** à excepter : zéro entrée, et rien qui puisse devenir
  stale.
- **Conséquence pour la revue :** si l'un de ces tests rougit à l'implémentation, le
  remède est le code, jamais une exception. Une allowlist qui apparaît dans ce vol est le
  signal qu'un détecteur de corpus a été ajouté hors périmètre.

Les deux détecteurs **préexistants** du voisinage ne sont pas livrés et ne changent pas de
comportement : la garde mika#2037 elle-même (son chemin de refus est intouché) et
`tests/review_anchor_prompt_contract.rs` (aucun `system_prompt.md` n'est modifié).

## Vérification

**U1** — `per_anchor.len() == anchors_found` et le compte des `refused: None` ==
`anchors_valid`, sur toutes les branches ; labels dans l'ordre du corps ; `NoAnchorLine`
⇒ liste vide ; extrait tronqué à 72 avec `…` et **non** suffixé en dessous de 72 ; un brief
multi-octets ne panique pas.

**U1 bis — le rejeu de la forme mesurée.** Fixture A1 fautive (ouverture de corps de
ticket) + A2/A3 verbatim disjointes ⇒ `per_anchor` nomme `A1:` refusé en `QuoteNotInBrief`,
`A2:`/`A3:` acceptés, `anchors_found=3`, `anchors_valid=2`. C'est l'assertion qui dit que
le livrable vise le défaut mesuré et non une forme inventée.

**U2** — une refusée + deux acceptées ⇒ les deux paragraphes **et** la phrase d'origine ;
**contrôle négatif** une seule refusée en `OverlappingRegions` ⇒ les paragraphes **sans**
la phrase d'origine ; **contrôle négatif** liste vide ⇒ `None`, et le re-prompt est
caractère pour caractère celui d'avant ce vol ; cinq refusées ⇒ trois nommées.

**U3** — le second `LlmRequest` capturé contient `A1`, l'extrait de la ligne fautive,
`A2` et `A3` ; et la réponse finale, sur une seconde réponse non corrigée, finit sur
`Disposition: ESCALATE` avec `anchors_valid=2` — le fail-closed de mika#2037 et le
fail-visible de mika#2338 sont inchangés.

**Non-régression** — `mod matrix` et les vingt tests de `review_anchor.rs` passent sans que
leur **intention** change (les quatre `assert_eq!` stricts sur `Missing` déclarent en plus
le `per_anchor` attendu, ce qui les transforme en pins du livrable) ; la matrice de seuils,
elle, n'est pas touchée — c'est le contrôle que D1 n'a relâché aucun seuil.

**Suite complète** — `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt`,
`make verify-bundled-skills`, les lints du dépôt (`scripts/check-byte-slices.sh` en
particulier, l'extrait étant une troncature), `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

Le re-prompt correctif nomme l'ancre refusée et les acceptées, les deux lignes de journal
portent `refused_labels` / `accepted_labels`, le protocole de mesure est écrit dans
`docs/agent-internals/agent-loop.md`, et la garde de mika#2037 est inchangée dans son
refus comme dans ses seuils.

## Acceptance criteria

- [ ] `verify_review_anchors` retourne, sur `Missing`, une entrée par ligne d'ancrage
      portant son préfixe, le début de son contenu (tronqué, indexé en caractères) et la
      raison de son refus ou `None` ; les deux égalités avec `anchors_found` /
      `anchors_valid` sont asserties.
- [ ] Sur la forme mesurée du ticket (A1 citant l'ouverture du corps du ticket, A2/A3
      verbatim disjointes), le re-prompt correctif nomme `A1` et son début, dit que cette
      ligne ne se trouve pas dans le brief de ce tour, et demande de **garder** `A2` et
      `A3` tels quels.
- [ ] La phrase sur l'origine (« un document lu par outil, pas le brief ») est au
      conditionnel et n'est rendue que pour un refus `QuoteNotInBrief` — un refus
      `OverlappingRegions` seul ne la rend pas.
- [ ] Quand aucune ligne du corps n'est une ancre (`NoAnchorLine`), le re-prompt est
      caractère pour caractère celui d'avant ce vol.
- [ ] `guard.review_anchor` et `guard.review_anchor_withheld` portent chacun
      `refused_labels` et `accepted_labels`, vides sous forme de chaîne vide et jamais
      absents ; le protocole de lecture du ratio retenues/re-prompts est documenté.
- [ ] La garde reste fail-closed et verbatim : aucun seuil, aucune règle de dispersion,
      aucun dédoublonnage, aucune fenêtre de corps modifiés ; `mod matrix` passe inchangé ;
      aucun `system_prompt.md` et aucun modèle touchés.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/review_anchor.rs` (U1 : `AnchorOutcome`, `per_anchor`, capture par ligne, troncature) | 75 |
| tests dans `review_anchor.rs` (U1 + U1 bis : 4 assertions étendues, 4 nouveaux tests) | 95 |
| `crates/mika-agent/src/agent_loop/mod.rs` (U2 : `format_anchor_diagnosis`, insertion, 2 × 2 champs de sonde) | 80 |
| tests inline dans `agent_loop/mod.rs` (U2 : formateur + ses deux contrôles négatifs) | 70 |
| `crates/mika-agent/src/calibration/roles/mika_arch.rs` (un `..` dans le `match`) | 1 |
| eval (`tests/eval/grounding_regressions/review_anchor.rs`, U3) | 55 |

Total estimé : 376 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 376 < 429 et 376 < 1000 : les deux bornes
passent sans découpage. Trois raisons rendent l'estimation peu susceptible de déborder :
le périmètre est **additif** — un champ sur une variante, un formateur, deux champs de
journal — et ne réécrit aucun algorithme ; le seul site de production hors
`review_anchor.rs` et `agent_loop/mod.rs` est une ligne dans `mika_arch.rs`, vérifiée par
`grep AnchorVerdict::Missing` ; et la part de test (220 des 376 lignes) porte sur des
fonctions pures appelées directement, sans nouveau harnais ni nouvelle fixture de base.
Le risque résiduel est le nombre d'`assert_eq!` stricts à étendre dans `review_anchor.rs`,
qui est connu et vaut quatre.
