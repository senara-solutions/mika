---
title: "mika#3003 — le re-prompt d'ancrage nomme l'outil qui a produit la citation, et le dénominateur qui le juge"
type: fix
status: active
date: 2026-11-01
issue: senara-solutions/mika#3003
---

# mika#3003 — le re-prompt d'ancrage nomme l'outil qui a produit la citation, et le dénominateur qui le juge

## Pourquoi

La garde d'ancrage de mika#2037 retient une disposition de mika-arch quand ses lignes
`A1:`…`A3:` ne citent pas le brief du tour. Le 2026-10-31 l'architecte a cité le **corps du
ticket**, lu avec `gh_read issue_view`, au lieu du plan revu — et la relance corrective
livrée par mika#2794 n'y a rien changé.

**Mesuré le 2026-10-31** (mika#2995, trace `f2a64bd7`, session `460a81ba`, 17:27:04Z) :

- le brief (message 123051, 31 761 caractères) est identique **octet pour octet** au plan
  commité (`61a3fb572`) — la garde n'a donc pas comparé au mauvais document ;
- les trois ancres de la réponse (message 123052) sont **absentes du brief** et présentes
  **mot pour mot** dans le corps de mika#2995 ;
- `guard.review_anchor_withheld` : 2 le 17/10, 5 le 18/10, 2 le 20/10, 1 le 29/10, 2 le
  31/10. **Le dénominateur n'est pas mesuré** — c'est la moitié manquante d'AC3, et elle
  n'existe nulle part aujourd'hui (§ D3).

La garde a eu raison : elle n'est pas en cause et rien dans son chemin de refus ne bouge.

### Ce vol est la piste que mika#2794 a écartée en la nommant

Le plan de mika#2794 (`docs/plans/2026-10-19-004-fix-2794-…-plan.md`, § *Pistes écartées*)
décrit ce vol mot pour mot :

> **Attribuer l'origine de façon déterministe** — chercher l'extrait refusé dans les blocs
> de résultat d'outil du tour, pour pouvoir dire « ton A1 cite la sortie de `gh_read` »
> plutôt que de le supposer. C'est la version **fondée** de la phrase de D2, et elle serait
> meilleure. Hors de ce vol pour trois raisons : elle ouvre un second accès au site (les
> `tool_result` de `request.messages`), elle met un budget de comparaison verbatim sur des
> sorties d'outil qui atteignent les centaines de kilo-octets, et elle serait un **deuxième
> pari dans un vol dont le premier n'est pas encore mesuré**. **C'est la suite naturelle si
> D3 rend un ratio inchangé.**

Les trois objections sont levées, et c'est ce qui autorise ce vol maintenant :

| objection de mika#2794 | levée par |
|---|---|
| second accès aux `tool_result` de `request.messages` | D1 — **aucun** accès à `request.messages`. Un registre alimenté **en amont**, au site unique où un résultat d'outil est déjà appendu au tour. Le site de la garde reçoit une valeur, il ne parcourt rien. |
| budget verbatim sur des centaines de kilo-octets | D1 — le registre est **plafonné et normalisé à la poussée**, et le besoin est la recherche d'une aiguille de 72 caractères, pas une comparaison croisée. |
| deuxième pari avant mesure du premier | la mesure est tombée : le 2026-10-31 le re-prompt nommé de mika#2794 était en service et **n'a rien changé** (récidive de la classe mika#2794, le ticket le dit). |

Ce que mika#2794 a laissé derrière lui est une **phrase au conditionnel**, gated sur
`QuoteNotInBrief` : « si elle vient d'un document que tu as lu avec un outil ce tour-ci […]
ce n'est pas le document en revue ». Au conditionnel **parce que le moteur ne savait pas** —
affirmer l'origine aurait été une affirmation non fondée. Ce vol lui donne la connaissance,
donc le droit d'affirmer. **C'est le seul changement de nature du vol** : la phrase passe du
conditionnel à l'indicatif, et seulement quand une recherche verbatim l'a établie.

## Décisions

### D1 — Le registre de lecture du tour : ce que la garde sait, et comment elle le sait

**C'est la réponse directe à la dernière phrase d'AC1** (« Le plan dit comment la garde
connaît le texte lu par `gh_read` dans le tour »).

Un type dans `crates/mika-agent/src/agent_loop/review_anchor.rs`, à côté du matcher qui le
consomme :

```rust
/// Normalized text of what the turn's tools returned, for anchor attribution (mika#3003).
///
/// Filled forward, at the single site where a tool result is appended to the turn — never
/// by walking `request.messages` back at the guard site. The guard receives a value; it
/// traverses nothing. This is the shape mika#2794 refused when the lookup would have been
/// a second reader of the conversation.
pub(crate) struct ToolReadLedger {
    /// `(tool name, normalized output)`, in call order. Normalized with the SAME
    /// `normalize_for_anchor_match` the matcher uses — a needle normalized one way and a
    /// haystack another is a comparison that silently never matches.
    entries: Vec<(String, String)>,
    /// Set when a push was clipped by `TOOL_READ_LEDGER_CHARS`. A miss in a truncated
    /// ledger is NOT an absence (mika#2331) — see the fail-silent rule below.
    truncated: bool,
}
```

Trois propriétés, chacune parce que l'inverse se devine mal :

- **Alimenté vers l'avant, au site unique d'append d'un résultat d'outil.** Pas d'accès à
  `request.messages`, donc pas de second lecteur de la conversation. Le site est celui qui
  appende déjà le `tool_result` du tour ; l'implémentation le localise par cet append
  existant — **il y en a un seul**, et D4 livre le détecteur qui le maintient unique.
- **Normalisé et plafonné à la poussée, jamais à la lecture.** `TOOL_READ_LEDGER_CHARS =
  200_000` caractères normalisés au total, indexés en `chars` et jamais en octets
  (`scripts/check-byte-slices.sh` ; les corps de ticket de ce dépôt sont français). Le coût
  est payé une fois par résultat d'outil, et il est linéaire ; la recherche, elle, est celle
  d'une aiguille de 72 caractères (`ANCHOR_EXCERPT_CHARS`) dans un foin borné. C'est ce qui
  retire l'objection de budget de mika#2794.
- **La portée est le TOUR, pas l'étape.** Le registre vit de l'entrée dans le corps de
  boucle jusqu'à la garde, et accumule toutes les étapes du tour — « lu pendant le tour »
  est exactement ce qu'AC1 demande.

L'attribution, fonction pure du même module :

```rust
/// Which tool's output a refused anchor's excerpt was found in, verbatim, this turn.
pub(crate) enum AnchorAttribution {
    /// Found: the excerpt appears in this tool's normalized output.
    Tool(String),
    /// Not found, and the ledger was WHOLE — the engine may say so.
    AbsentFromReads,
    /// Not found, but the ledger was CLIPPED — the engine may NOT say so.
    Unknown,
}
```

**`Unknown` est la décision de conception qui tient tout le reste.** Un échec de recherche
dans un registre tronqué et un échec dans un registre complet rendent le même `false` ; les
confondre ferait dire au moteur « cette ligne ne vient d'aucun outil » sur une sortie qu'il
avait coupée. Sur `Unknown`, le re-prompt retombe **mot pour mot** sur la phrase au
conditionnel de mika#2794 — la connaissance manque, donc l'affirmation ne se fait pas. C'est
la grounding rule du dépôt (root `CLAUDE.md`) appliquée au moteur lui-même, et c'est aussi
le troisième cas d'AC2 par construction.

L'attribution ne tourne **que** sur les `AnchorOutcome` dont `refused == Some(QuoteNotInBrief)`.
Un `OverlappingRegions` ou un `QuoteTooShort` ne pose pas la question « d'où vient ce
texte ? » : le texte **est** dans le brief, c'est sa géométrie ou sa longueur qui échoue.
C'est le gate déjà écrit par mika#2794 au même endroit, réutilisé tel quel.

### D2 — Le re-prompt nomme l'outil, et rien de plus

`format_anchor_diagnosis` (`agent_loop/mod.rs:9633`, livrée par mika#2794) prend un second
argument — les attributions, dans l'ordre des refusées — et **seul son troisième paragraphe
change**. Les deux premiers (les acceptées à garder, les refusées nommées avec leur extrait)
sont intouchés, parce qu'ils ne sont pas en cause.

| attribution | ce que le re-prompt dit |
|---|---|
| `Tool(nom)` | « A1 cite verbatim la sortie de l'outil `<nom>` que tu as appelé ce tour-ci. Ce n'est pas le document en revue : le brief est le message utilisateur de ce tour, et c'est lui, et lui seul, que l'ancre doit citer. » |
| `AbsentFromReads` | la phrase actuelle, **au conditionnel**, mot pour mot celle de mika#2794 |
| `Unknown` | la phrase actuelle, **au conditionnel**, mot pour mot celle de mika#2794 |

**Pas de cas spécial `gh_read issue_view`, délibérément.** AC1 donne « cette citation vient
du corps du ticket » comme **exemple** (« Par exemple »), pas comme gabarit. Nommer l'outil
est strictement plus fort que nommer « le corps du ticket » : c'est ce que le registre sait
vraiment, c'est vrai quand la citation vient d'un `read_file` ou d'un `run_gh`, et ça ne
devient pas faux le jour où le nom de l'outil change. Une branche par outil serait un second
site de vérité sur « quel outil lit un ticket », et elle mentirait sur la première lecture
qui n'en vient pas.

**Un seul site de formatage, appelé une fois.** La garde reste fail-closed : le chemin de la
seconde retenue, les seuils, la dispersion, le dédoublonnage, la fenêtre de corps,
`escalate_unattested_disposition` et `REVIEW_ANCHOR_ENGINE_FINDING_MARKER` ne bougent pas
d'un octet — `dispatch-lib` lit ces deux derniers par forme fixe.

### D3 — Le dénominateur qu'AC3 demande n'existe pas, et ce n'est pas une sonde en double

AC3 veut `review_anchor_withheld` **avec son dénominateur : les passages non terminaux**.
Ce n'est pas le dénominateur de mika#2794, et c'est pour ça qu'une ligne nouvelle est due
ici alors que mika#2794 D3 en refusait une :

| ratio | numérateur | dénominateur | existe ? |
|---|---|---|---|
| mika#2794 D3 | `guard.review_anchor_withheld` | `guard.review_anchor` (une par re-prompt) | **oui**, livré |
| mika#3003 AC3 | `guard.review_anchor_withheld` | **passages non terminaux** (une par tour où la garde s'arme) | **non** |

`guard.review_anchor` n'est émise **que** quand des ancres manquent : elle compte les
échecs, jamais les passages. Un tour où la garde s'arme et où les ancres sont bonnes
n'émet rien du tout — donc « la garde n'a retenu personne » et « aucune revue n'a eu lieu »
rendent zéro tous les deux (classe mika#2205), et la cible d'AC3 *est* un ratio qui baisse
vers zéro.

Une ligne, INFO, `target: "mika::otel"` : **`guard.review_anchor_armed`**, émise sur
**toutes** les branches de la garde armée y compris la saine, portant `anchors_found`,
`anchors_valid`, `brief_chars`, et `withheld: bool`. C'est le dénominateur, et un seul
`jq` rend le ratio. Ce n'est pas un second producteur du nombre de mika#2794 : les deux
populations sont disjointes (`armed` ⊇ `review_anchor`), et c'est précisément la raison pour
laquelle mika#2794 avait raison de la refuser pour **son** ratio.

Les deux lignes existantes gagnent **un** champ, le même sur les deux :
`attributed_sources` — les noms d'outils attribués, joints par `,` (un champ `tracing` porte
un scalaire), **chaîne vide** quand aucune attribution n'a abouti. C'est ce qui rend la thèse
du ticket lisible en production : « c'est toujours le corps du ticket » se confirme ou se
réfute sur ce champ seul.

**Vide est une chaîne vide, jamais un champ absent.** L'absence du champ — ou de la ligne
`armed` — signifie un binaire antérieur à ce correctif (classe mika#2340), jamais « aucune
attribution ». L'instrument n'est pas vivant avant `make deploy`.

Protocole, sur `$MIKA_SPIRIT_LOG_FILE` (`/var/log/mika/server.log`) :

```sh
jq -r 'select(.event | startswith("guard.review_anchor"))
       | [.timestamp, .event, .anchors_valid, .attributed_sources, .miss_reason] | @tsv' \
  < /var/log/mika/server.log
```

- **après** : dénominateur = lignes `guard.review_anchor_armed` ; numérateur = lignes
  `guard.review_anchor_withheld`.
- **avant** : le dénominateur est **inobtenable sur le journal**, et le plan le dit plutôt
  que de fabriquer un nombre. La ligne n'existait pas pendant la fenêtre. Il se reconstruit
  **hors ligne** sur la base, et une seule fois : les réponses de mika-arch sont dans
  `llm_calls.response_text` (colonne ajoutée en v31), et un passage non terminal est une
  réponse portant une ligne `Disposition:`/`Verdict:` non terminale sur un brief
  ≥ `min_brief_chars`. Le numérateur « avant » est déjà acquis et écrit au ticket (12 retenues
  sur la fenêtre du 17/10 au 31/10).
- la lecture avant/après sur 14 + 14 jours et son écriture au ticket sont **un geste
  d'opérateur sur une fenêtre de déploiement**, pas une assertion de CI. Ce vol livre
  l'instrument et le protocole ; il ne peut pas livrer la mesure d'une fenêtre qui n'a pas
  eu lieu.

### D4 — Le registre a un seul écrivain, et un détecteur le tient

Un second site de poussée est la régression qui ne rougit rien : les attributions restent
correctes, le dénominateur reste correct, et le registre compte deux fois la même sortie
d'outil — ou, pire, un site qui pousse **sans** normaliser rend un registre où aucune
recherche n'aboutit jamais, donc une garde qui retombe silencieusement sur le conditionnel
de mika#2794 et un ticket qu'on rouvre pour la troisième fois. C'est la classe
`grooming_marker` (mika#2158) : toutes les assertions comportementales restent vertes.

Un scan de source dans les tests inline d'`agent_loop/mod.rs` refuse un second appelant de
la méthode de poussée sous `src/`, avec son contrôle positif de bonne foi (le scan rougit
sur un second appelant injecté). Le scan consulte `crate::source_scan::is_test_source_path`
pour la classification production/test, jamais une troncature sur `#[cfg(test)]` — la
prémisse invalidée par mika#2321 et déjà réparée pour les trois gardes voisines.

### Pistes écartées

- **Relâcher la garde de mika#2037** — interdit par le brief, et c'est la faille qu'elle
  ferme. Le ticket le dit en autant de mots : « la garde a eu raison : elle n'est pas en
  cause ».
- **Une consigne de plus dans `mika-arch-*/system_prompt.md`** — écartée par mika#2786 puis
  par mika#2794, et la raison tient (`feedback_prompt_enforcement_fragile.md`) : la consigne
  existe déjà (*What does NOT satisfy the contract*) et le modèle ne la tient pas. Le
  re-prompt moteur n'est pas une consigne de plus : il arrive **après** l'échec et il porte
  la mesure de cet échec-là.
- **Retirer `gh_read issue_view` de l'outillage des passes d'architecte** — l'architecte doit
  pouvoir lire le ticket ; c'est la citation qui est fautive, pas la lecture.
- **Filtrer le registre sur une liste d'outils « lisants »** (`gh_read`, `read_file`, …) —
  un second site de vérité sur « quel outil produit du texte citable », faux au premier outil
  ajouté. Le registre prend tout, borné ; c'est l'attribution qui nomme ce qu'elle a trouvé.
- **Changer de modèle** — interdit par le brief (mika#1190). Aucun modèle ne bouge, donc
  `make calibrate-mika-arch` n'est pas une précondition ; le scénario `required_review_anchor`
  de la suite mika-arch fait un seul appel et ne traverse pas le re-prompt.
- **Armer AC3 par un script livré** — un binaire ou un script d'agrégation de plus pour un
  `jq` d'une ligne et une requête SQL ponctuelle ; le protocole écrit en `docs/` est le
  livrable, et il tient la borne par vol.

## Unités

### U1 — Le registre et l'attribution (`review_anchor.rs`)

`crates/mika-agent/src/agent_loop/review_anchor.rs` :

- `ToolReadLedger` + `TOOL_READ_LEDGER_CHARS` + `AnchorAttribution`, doc-comments portant les
  trois propriétés de D1 et la règle `Unknown`.
- `push(tool_name, output)` — normalise via `normalize_for_anchor_match`, clippe au plafond
  **restant** en indexant en `chars`, pose `truncated` quand elle a clippé.
- `attribute(&AnchorOutcome) -> AnchorAttribution` — recherche verbatim de l'extrait
  normalisé (`…` de troncature retiré avant la recherche : c'est un marqueur de rendu, pas
  du texte lu) dans les entrées, dans l'ordre d'appel ; `Tool` au premier trouvé,
  `Unknown` si rien et `truncated`, `AbsentFromReads` sinon.
- `normalize_for_anchor_match` passe `pub(crate)` (elle est privée aujourd'hui,
  `review_anchor.rs:258`) — **sans second site de normalisation**, c'est tout l'intérêt.

### U2 — Le feed, le diagnostic, les sondes (`agent_loop/mod.rs`)

`crates/mika-agent/src/agent_loop/mod.rs` :

- Le `ToolReadLedger` est construit à l'entrée du corps de boucle conversationnel et poussé
  au **site unique** où un résultat d'outil est appendu au tour.
- `format_anchor_diagnosis` prend les attributions en second argument ; son troisième
  paragraphe branche sur les trois arms de D2. Les deux premiers paragraphes sont intouchés.
- Au site de la garde (`mod.rs:4703`), les attributions sont calculées sur les
  `per_anchor` refusés en `QuoteNotInBrief` et passées au formateur.
- `attributed_sources` sur `guard.review_anchor` (WARN, `mod.rs:4738`) et
  `guard.review_anchor_withheld` (ERROR, `mod.rs:4834`).
- `guard.review_anchor_armed` (INFO, `target: "mika::otel"`) sur **toutes** les branches de
  la garde armée, y compris `AnchorVerdict::Satisfied`.

### U3 — Le rejeu de la forme mesurée, bout en bout

`crates/mika-agent/tests/eval/grounding_regressions/review_anchor.rs` — un scénario sur le
harnais existant reproduisant la forme du 2026-10-31 : un brief = plan, un appel d'outil
dont la sortie est un corps de ticket, et une réponse dont les trois ancres citent ce corps.
Via `harness.mock_provider` → `captured_requests()`, il assert que le **second**
`LlmRequest` nomme l'outil ; et que le fail-closed n'a pas bougé (seconde réponse non
corrigée ⇒ `Disposition: ESCALATE`). **Contrôle négatif dans le même fichier** : les mêmes
ancres sans l'appel d'outil ⇒ le re-prompt est celui d'avant ce vol, au conditionnel.

### U4 — Documentation

- `docs/agent-internals/agent-loop.md` — le registre, son site unique d'alimentation, la
  règle `Unknown`, la ligne `armed`, le champ `attributed_sources`, le protocole d'AC3 et la
  **reconstruction hors ligne du dénominateur « avant »**.
- `crates/mika-agent/CLAUDE.md` — trois lignes dans la section de la garde d'ancrage,
  nommant le dénominateur et pourquoi la ligne `armed` n'est pas la sonde que mika#2794
  refusait.

## Fire-Disposition

**Option (a) — exception nommée en allowlist, allowlist vide par construction.**

Ce vol livre des détecteurs au sens de mika#1574 : les tests et assertions de U1/U2/U3, et
**le scan de source de D4**. Détail d'implémentation :

- **Ils atterrissent armés.** Aucun `#[ignore]`, aucun `#[cfg(skip)]`, aucune ligne
  désactivée : verts au premier `cargo test -p mika-agent`.
- **L'allowlist du scan de D4 est livrée vide, et elle l'est par construction.** Le scan
  cherche les appelants de la méthode de poussée d'un type **créé par ce vol** : il ne peut
  exister aucun appelant préexistant, donc aucune violation à excepter. Elle est **pinnée
  vide** par un test frère (le motif de `mika2425_identity_context_history_has_a_single_reader`),
  de sorte qu'une entrée future ne puisse pas y être glissée sans rougir — c'est l'assertion
  auto-nettoyante que mika#1574 demande, et c'est ce qui empêche cette allowlist de devenir
  celle que mika#2309 condamne.
- **Aucun autre livrable n'inspecte un corpus préexistant** — pas de règle de lint, pas de
  garde CI, pas de scan sur `src/**` hors celui de D4. Les autres assertions portent sur le
  retour de fonctions pures appelées avec leurs propres entrées et sur un `LlmRequest`
  produit par le harnais.
- **Conséquence pour la revue :** si l'un de ces tests rougit à l'implémentation, le remède
  est le code, jamais une exception. Une allowlist non vide dans ce vol est le signal qu'un
  détecteur de corpus a été ajouté hors périmètre.

Les détecteurs **préexistants** du voisinage ne sont pas livrés et ne changent pas de
comportement : la garde mika#2037 elle-même (chemin de refus et seuils intouchés),
`tests/review_anchor_prompt_contract.rs` (aucun `system_prompt.md` modifié), et les quatre
`assert_eq!` stricts sur `Missing` de `review_anchor.rs`, qui restent les pins de mika#2794.

## Vérification

**U1 — le registre.** Poussée sous le plafond ⇒ `truncated` faux et le texte normalisé
retrouvé ; poussée au-dessus ⇒ clippée, `truncated` vrai ; la normalisation du registre est
**la même fonction** que celle du matcher (une entrée dont seuls les espaces diffèrent de
l'extrait est trouvée) ; une sortie multi-octets (corps de ticket français) ne panique pas et
est clippée en caractères.

**U1 bis — l'attribution et ses trois arms.** Extrait présent dans la sortie d'un outil
⇒ `Tool(nom)` ; absent, registre complet ⇒ `AbsentFromReads` ; absent, registre tronqué
⇒ `Unknown`. Le `…` de troncature de l'extrait est retiré avant recherche (un extrait
tronqué s'attribue comme un court). Un `AnchorOutcome` refusé en `OverlappingRegions`
n'est pas attribué du tout.

**U2 — le re-prompt, avec le contrôle négatif d'AC2 dans le même appel.** Une refusée
attribuée à un outil + deux acceptées ⇒ les deux premiers paragraphes **inchangés** et le
troisième **à l'indicatif, nommant l'outil** ; **contrôle négatif** la même refusée en
`AbsentFromReads` ⇒ le troisième paragraphe est **caractère pour caractère** celui d'avant ce
vol ; **contrôle négatif** `Unknown` ⇒ idem ; liste vide ⇒ `None` et le re-prompt entier est
celui d'avant ce vol. Les deux derniers sont la seconde phrase d'AC2 (« une ancre absente des
deux produit le re-prompt actuel ») et le premier est la première phrase.

**U2 bis — les sondes.** `guard.review_anchor_armed` est émise sur `Satisfied` **et** sur
`Missing` ; `attributed_sources` est présent et vide sur les deux lignes existantes quand
rien n'est attribué, et jamais absent.

**U3 — bout en bout.** Le second `LlmRequest` capturé nomme l'outil et l'extrait de la
ligne fautive ; la réponse finale, sur une seconde réponse non corrigée, finit sur
`Disposition: ESCALATE` avec `anchors_valid` inchangé — le fail-closed de mika#2037 et le
fail-visible de mika#2338 ne bougent pas.

**U4 / D4 — le scan.** Un second appelant injecté de la méthode de poussée fait rougir le
scan (contrôle positif de bonne foi) ; l'allowlist est pinnée vide.

**Non-régression** — `mod matrix` et les tests de `review_anchor.rs` passent sans que leur
**intention** change ; les quatre `assert_eq!` stricts de mika#2794 sur `Missing` sont
inchangés (ce vol n'ajoute aucun champ à `AnchorOutcome`) ; la matrice de seuils n'est pas
touchée — c'est le contrôle que D1 n'a relâché aucun seuil.

**Suite complète** — `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt`,
`make verify-bundled-skills`, les lints du dépôt (`scripts/check-byte-slices.sh` en
particulier : le registre clippe, l'extrait est tronqué), `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

Le re-prompt correctif nomme à l'indicatif l'outil dont la sortie contient verbatim l'ancre
refusée, retombe mot pour mot sur le conditionnel de mika#2794 quand il ne le sait pas ou ne
peut pas le savoir, la ligne `guard.review_anchor_armed` fournit le dénominateur qu'AC3
demande, le protocole de lecture avant/après est écrit dans
`docs/agent-internals/agent-loop.md`, et la garde de mika#2037 est inchangée dans son refus
comme dans ses seuils.

## Acceptance criteria

- [ ] **AC1.** Quand une ancre est refusée parce qu'elle est absente du brief, le re-prompt
      dit si son texte est présent dans ce que le tour a lu par outil, en **nommant l'outil**.
      Le plan dit comment la garde le sait (§ D1) : un registre plafonné et normalisé,
      alimenté vers l'avant au site unique d'append d'un résultat d'outil, jamais par un
      parcours de `request.messages` au site de la garde.
- [ ] **AC2.** Dans le même appel et avec contrôle négatif : une ancre prise dans la sortie
      d'un outil du tour produit le re-prompt qui **nomme sa source** ; une ancre absente du
      brief **et** des lectures produit le re-prompt actuel, caractère pour caractère.
- [ ] Une recherche infructueuse dans un registre **tronqué** ne produit jamais l'affirmation
      d'absence : elle retombe sur le conditionnel de mika#2794 (`Unknown`).
- [ ] L'attribution ne tourne que sur un refus `QuoteNotInBrief` ; un `OverlappingRegions`
      ou un `QuoteTooShort` ne produit aucune phrase d'origine.
- [ ] `guard.review_anchor_armed` est émise une fois par passage non terminal armé, sur
      **toutes** les branches y compris `Satisfied` ; `guard.review_anchor` et
      `guard.review_anchor_withheld` portent `attributed_sources`, vide sous forme de chaîne
      vide et jamais absent.
- [ ] **AC3.** Le protocole de lecture du ratio `withheld` / passages non terminaux est
      documenté, **y compris la reconstruction hors ligne du dénominateur de la fenêtre
      « avant »** (`llm_calls.response_text`, v31), que le journal ne porte pas. La mesure
      14 jours avant / 14 jours après et son écriture au ticket sont un geste d'opérateur sur
      une fenêtre de déploiement, hors périmètre de cette PR.
- [ ] Le registre a un seul écrivain sous `src/`, tenu par un scan de source à allowlist
      vide **pinnée** vide, avec contrôle positif de bonne foi.
- [ ] La garde reste fail-closed et verbatim : aucun seuil, aucune règle de dispersion, aucun
      dédoublonnage, aucune fenêtre de corps modifiés ; `mod matrix` passe inchangé ; aucun
      `system_prompt.md` et aucun modèle touchés.

## Risque nommé

Le ticket le pose et le plan ne le minimise pas : **c'est un défaut de comportement du
modèle, et un re-prompt plus précis ne garantit pas l'obéissance** (doctrine mika#1190).
Ce vol est un pari, exactement comme mika#2794 — la différence est qu'il remplace une
hypothèse au conditionnel par un fait établi par recherche verbatim, ce qui est la seule
chose qu'un moteur puisse faire de plus sans toucher au modèle. Aucun test hors ligne ne
mesure l'obéissance : les tests prouvent que le re-prompt **porte** l'information, jamais que
le modèle la suit. C'est AC3 qui tranche, et c'est pourquoi le dénominateur de D3 est un
livrable de ce vol et non un reste.

Si AC3 rend un ratio inchangé, la piste suivante n'est **pas** un troisième re-prompt :
c'est le constat que le levier moteur est épuisé sur cette classe, et la décision remonte à
l'opérateur (changement de modèle sous calibration mika#1190, ou retrait de la lecture du
ticket de l'outillage des passes d'architecte). Ce vol n'ouvre pas ce ticket : un ticket
ouvert sur une hypothèse non mesurée est ce que mika#2786 a refusé de faire pour cette
piste-là.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/review_anchor.rs` (U1 : `ToolReadLedger`, `AnchorAttribution`, `push`, `attribute`, plafond, `pub(crate)` sur le normaliseur) | 85 |
| tests dans `review_anchor.rs` (U1 + U1 bis : registre, plafond, multi-octets, trois arms, gate sur `QuoteNotInBrief`) | 80 |
| `crates/mika-agent/src/agent_loop/mod.rs` (U2 : feed au site unique, attribution au site de la garde, 3e paragraphe du formateur, ligne `armed`, 1 champ × 2 lignes) | 75 |
| tests inline dans `agent_loop/mod.rs` (U2 : formateur + ses trois contrôles négatifs, sondes) | 70 |
| scan de source + contrôle positif + pin d'allowlist vide (D4, tests inline `agent_loop/mod.rs`) | 30 |
| eval (`tests/eval/grounding_regressions/review_anchor.rs`, U3 : forme mesurée + contrôle négatif) | 60 |

Total estimé : 400 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 400 < 429 et 400 < 1000 : les deux bornes
passent sans découpage. Quatre raisons rendent l'estimation peu susceptible de déborder.

Le périmètre est **additif sur une structure déjà livrée** : mika#2794 a posé
`per_anchor`, `AnchorOutcome`, `format_anchor_diagnosis`, `anchor_label_fields` et les deux
lignes de journal — ce vol ajoute un argument au formateur, un paragraphe dans son corps, un
champ sur deux lignes existantes et une ligne nouvelle. **Aucun algorithme n'est réécrit** :
le matcher, ses seuils, sa dispersion et son fail-closed ne sont pas touchés, et
`AnchorOutcome` ne gagne aucun champ — donc les quatre `assert_eq!` stricts qui avaient
coûté le risque résiduel de mika#2794 ne sont **pas** à étendre cette fois.

Les sites de production sont **trois, tous connus et lus** :
`review_anchor.rs` (le nouveau type), le site de la garde (`mod.rs:4703`) et le site unique
d'append d'un résultat d'outil. Le seul qui demande une localisation à
l'implémentation est le troisième, et D4 livre le détecteur qui prouve qu'il n'y en a qu'un.

Enfin la part de test (240 des 400 lignes) porte sur des **fonctions pures appelées
directement** — `push`, `attribute`, `format_anchor_diagnosis` — sans nouveau harnais ni
nouvelle fixture de base ; l'eval réutilise `grounding_regressions/review_anchor.rs`, fichier
que mika#2794 a déjà créé et câblé.

Le risque résiduel est la **localisation du site de feed** : si l'append d'un résultat
d'outil s'avérait exister à plus d'un endroit, D4 rougit immédiatement et l'arbitrage est une
décision de conception, pas un volume — router les sites par un helper unique coûte une
dizaine de lignes, pas une phase.
