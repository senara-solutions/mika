# Plan — mika#1982 : `mika ask` infère stdin quand le positionnel est absent, et la sentinelle `-` cesse d'être ignorée par `--remote`

- **Ticket :** senara-solutions/mika#1982
- **Type :** feat (ergonomie CLI) + fix (faux vert sur `--remote`)
- **Date :** 2026-09-21
- **Branche :** `cli/1982/ask-mika-ask-should-accept-message-from`

---

## Contexte

### La rectification du ticket est le premier livrable

Le ticket pose que piper un prompt est **impossible** :

> `cat prompt.txt | mika ask --agent mika-prime` → `error: the following required arguments were not provided: <MESSAGE>`
> Contournement actuel : `mika ask --agent X "$(cat prompt.txt)"` — fragile (quoting, taille ARG_MAX, historique shell).

**La lecture du code réfute la prémisse, et la réfutation change la conception.** `mika ask` lit stdin depuis
longtemps, par la sentinelle Unix `-` :

- `crates/mika-cli/src/commands/ask.rs:268-275` — chemin agent local ;
- `crates/mika-cli/src/commands/ask.rs:757-764` — chemin `--team`.

Cette surface n'est ni accidentelle ni oubliée : elle est **documentée publiquement**
(`docs/getting-started.md:307` et `:311`, `echo "…" | mika ask "-"`, `cat meeting-notes.txt | mika ask "-"`) et elle
porte **un consommateur de production critique** — `_arch_ask` dans
`skills/bundled/_shared/dispatch-lib.sh:4782`, c'est-à-dire **le chemin de tout le grooming architecte**, dont le
commentaire d'origine (mika#1283) dit en toutes lettres pourquoi il pipe : *« Fix: pipe content via stdin (mika ask
"-" reads the message from stdin per `mika ask --help`) »*.

Trois conséquences, dans l'ordre de leur poids :

1. **Le « contournement fragile » nommé par le ticket n'est pas le contournement réel.** L'argument
   quoting / `ARG_MAX` / historique shell tombe : `cat prompt.txt | mika ask --agent X -` n'a aucun de ces trois
   problèmes. Le ticket n'est donc **pas** un correctif de robustesse, et le présenter comme tel conduirait à
   sur-dimensionner le travail.
2. **Le défaut demandé reste néanmoins réel, et il est exactement l'AC1.** `echo "hello" | mika ask --agent x`
   — *sans* le `-` — échoue aujourd'hui. C'est une **ergonomie** : la convention Unix veut qu'un outil déduise
   stdin de l'absence d'argument quand stdin n'est pas un TTY, plutôt que d'exiger une sentinelle. Le ticket est
   légitime ; c'est son cadrage qui est faux.
3. **La non-régression de `-` devient une contrainte dure, pas une politesse.** Toute conception qui
   dégraderait `-` casserait le grooming architecte sur tous les tickets. C'est ce qui décide la table de
   décision ci-dessous, et notamment le fait que `-` reste lu **inconditionnellement**, TTY ou non.

### Le défaut que le ticket ne nomme pas, et il est plus grave que celui qu'il nomme

`--remote` **ignore la sentinelle `-` entièrement**. `crates/mika-cli/src/main.rs:331-338` passe `&args.message`
brut à `mika_cli::remote_ask::run_remote`, dont la signature (`remote_ask.rs:614-621`) prend `message: &str` et
qui **ne contient aucune lecture de stdin**. Donc :

```bash
cat plan.md | mika ask --remote https://gw.example.com/a2a/cust/mika-arch -
```

envoie au serveur distant la chaîne littérale `"-"` — **un octet** — et rend une réponse plausible à une question
qui n'a jamais été posée. Ni erreur, ni avertissement.

C'est **la classe mika#2304** à la lettre : un canal qui n'atteint pas l'exécutant et dont le no-op est
indistinguable d'un succès. mika#2304 l'a mesurée sur `--model`, et son entrée de `CLAUDE.md` énonce la règle qui
s'applique ici mot pour mot : *« un modèle non appliqué rend la mesure fausse tout en produisant une réponse
plausible. Un no-op silencieux ici est le défaut. »* Un message non appliqué est strictement pire : ce n'est plus
la mesure qui est fausse, c'est la question.

Ce défaut n'a pas de consommateur mesuré aujourd'hui (`_arch_ask` n'emploie pas `--remote`), ce qui explique
qu'il n'ait jamais été vu. Il est corrigé ici parce que le correctif demandé **traverse exactement ce site** :
livrer l'inférence TTY sans réparer `--remote` produirait un troisième comportement divergent sur le même
drapeau, et rendrait le faux vert *plus* probable en encourageant l'usage du pipe.

### Trois sites, deux implémentations copiées, une absente

| chemin | site | gestion de `-` | gestion TTY |
|---|---|---|---|
| agent local | `commands/ask.rs:268` | oui (copie A) | non |
| `--team` | `commands/ask.rs:757` | oui (copie B, identique à A) | non |
| `--remote` | `main.rs:332` → `remote_ask.rs:614` | **non** | non |

Ajouter l'inférence TTY par le geste évident — éditer les deux sites existants — la poserait en **deux**
exemplaires et laisserait le troisième chemin muet. C'est la classe « lecteur unique » que la maison a déjà dû
engraver trois fois : `grooming_marker` (mika#2158, deux regex divergentes pendant des mois sans que rien ne
casse), `parse_log_llm_bodies` (mika#2220, deux tables de vérité dont l'une acceptait `True` et l'autre non),
`LlmUsage::accumulate` (mika#1883, second site d'addition écrit à la main rendant un total faux avec tous les
tests au vert). Les trois partagent une propriété : **la divergence ne rend aucune décision fausse, elle rend
deux décisions différentes**, et rien ne rougit.

---

## Requirements

- **R1** — `echo "hello" | mika ask --agent x` envoie `hello` (AC1 du ticket).
- **R2** — `mika ask --agent x "hello"` est **inchangé**, y compris quand stdin porte des données (le positionnel
  est prioritaire et stdin n'est alors **jamais lu**).
- **R3** — stdin TTY **et** positionnel absent → erreur d'usage immédiate, jamais un blocage silencieux en
  lecture (AC3).
- **R4** — `mika ask … -` reste lu depuis stdin **à l'identique**, sur les chemins local et `--team`, TTY ou non.
  Non-régression dure : `_arch_ask` en dépend.
- **R5** — `--remote` applique la **même** résolution que les deux autres chemins : `-` y est lu depuis stdin, et
  l'inférence TTY y opère. Le faux vert est fermé.
- **R6** — La résolution a **un seul lecteur** dans l'arbre. Aucun second site ne ré-implémente la table de
  décision.
- **R7** — La fonction de résolution est **pure et testable sans TTY réel** : la « tty-ness » et la source de
  lecture sont des paramètres, jamais des appels à `std::io::stdin()` enfouis (AC4 ne peut pas être satisfaite
  autrement — aucun test unitaire ne peut fabriquer un terminal).
- **R8** — Aucun changement au reste de la surface `mika ask` : drapeaux, format de sortie, codes de sortie,
  `--task-complete`, plafond `MAX_CALLBACK_RESULT`.

---

## Approche / Conception

### La table de décision, à quatre lignes

| positionnel | stdin TTY | comportement | motif |
|---|---|---|---|
| `"hello"` | indifférent | `"hello"`, **stdin jamais lu** | R2 / AC2 |
| `"-"` | indifférent | lire stdin | R4 — non-régression `_arch_ask` et doc |
| absent | **non** | lire stdin | R1 / AC1 |
| absent | **oui** | erreur d'usage | R3 / AC3 |

Deux choix méritent d'être posés plutôt que subis :

- **`-` est lu même sur un TTY.** C'est la sémantique Unix de la sentinelle (`cat -` attend la saisie) et c'est le
  comportement actuel. Le conditionner au non-TTY serait une régression silencieuse pour l'opérateur qui tape
  `mika ask -` puis son message, et n'achèterait rien.
- **Le positionnel présent n'entraîne aucune lecture de stdin.** Pas même une tentative non bloquante : c'est ce
  qui rend AC2 byte-identique et ce qui évite qu'un `mika ask x "msg" < gros-fichier` consomme un descripteur
  pour rien.

### Un lecteur unique, pur, dans la lib

Nouveau module `crates/mika-cli/src/ask_message.rs`, exposé par `lib.rs` (qui expose déjà `remote_ask` et
`supervision`, et que le binaire importe déjà — `commands/ask.rs` fait `mika_cli::remote_ask::…`).

Forme visée, la signature étant le cœur de la testabilité (R7) :

```rust
/// Résout le message d'un `mika ask` selon la table de décision de mika#1982.
///
/// `positional` est `None` quand clap n'a pas reçu l'argument. `stdin_is_tty` et
/// `reader` sont des paramètres — et non des appels à `std::io::stdin()` — parce
/// qu'aucun test unitaire ne peut fabriquer un terminal : c'est ce qui rend l'AC3
/// vérifiable autrement qu'à la main.
pub fn resolve_ask_message<R: std::io::Read>(
    positional: Option<&str>,
    stdin_is_tty: bool,
    reader: R,
) -> Result<String, AskMessageError>
```

avec un `AskMessageError` distinguant au moins `MissingOnTty` (R3, texte d'usage) de `Empty` (entrée lue mais
vide). Le `trim()` de la lecture stdin est **conservé** tel quel : les deux sites actuels trimment, `_arch_ask`
pipe un markdown dont les bords blancs n'ont pas de sens, et le retirer serait un changement de comportement
étranger au ticket.

Un enrobage fin — `resolve_from_process_stdin(positional)` — fait les deux seuls appels impurs
(`std::io::stdin().is_terminal()` et `std::io::stdin().lock()`) et délègue. C'est lui que les trois chemins
appellent.

### Où la résolution est appelée, et pourquoi pas plus bas

`args.message` est lu à **trois** endroits de `main.rs`, et le branchement `--team` (ligne ~121-143) se produit
**avant** celui de `--remote` / local (ligne ~311+). Il n'existe donc pas de point unique en aval des trois.

La résolution est faite **une fois, en amont des trois branchements**, dans le bras `Commands::Ask` de `main.rs`,
et la `String` résolue est passée aux trois appelants (`run_team_ask`, `run_remote`, `ask::run`).

Deux corollaires :

- **La résolution ne doit pas être hissée au-dessus du bras `Commands::Ask`.** D'autres sous-commandes lisent
  stdin pour leur propre compte — `credential_helper` (protocole git sur stdin), `setup`, `agents`, `skills`,
  `config`, `tasks`. Une lecture au démarrage du binaire leur volerait leur entrée. Le périmètre est le bras
  `Ask`, strictement.
- **Les lectures internes de `commands/ask.rs:268` et `:757` sont supprimées**, pas doublées. Les deux fonctions
  reçoivent désormais un message déjà résolu. C'est ce qui satisfait R6 : après ce changement, `std::io::stdin()`
  n'apparaît plus qu'une fois sur le chemin `ask`.

### Le positionnel devient `Option<String>`

`cli.rs:243` (`pub message: String`) devient `Option<String>`, et le commentaire d'aide passe de
`/// The message to send (use "-" to read from stdin)` à une formulation qui nomme les trois portes.

Conséquence assumée : clap cesse d'émettre `error: the following required arguments were not provided:
<MESSAGE>`. R3 exige que le cas TTY reste une erreur d'usage — elle est donc **ré-émise à la main**, avec un
texte qui nomme les trois gestes qui marchent (positionnel, pipe, `-`). Le code de sortie doit rester celui d'une
erreur d'usage et ne doit pas se confondre avec le code transport de mika#2278 : la résolution échoue **avant**
tout appel réseau, donc elle sort par le chemin d'erreur ordinaire du binaire.

### Population nommée, non couverte, et c'est délibéré

**Non-TTY, positionnel absent, et aucune donnée sur stdin** — le cas `mika ask --agent x </dev/null`, et celui
d'un cron ou d'une unité systemd dont stdin est fermé. La lecture rend une chaîne vide immédiatement (pas de
blocage), et le binaire échoue sur `Empty message`. **Le message d'erreur change** pour cette population : elle
voyait l'erreur clap d'usage, elle verra l'erreur de message vide.

C'est accepté et non corrigé : les deux textes disent la même chose à l'opérateur (« tu n'as pas fourni de
message »), et les distinguer demanderait de deviner l'intention derrière un stdin vide — ce qu'aucun signal ne
permet. Le cas est **testé** pour que le changement soit constaté plutôt que découvert.

Cas voisin, non couvert lui non plus et plus honnêtement : `mika ask --agent x < <(sleep 999)` bloque. C'est la
convention Unix (`cat` fait de même) et c'est exactement ce que l'AC3 borne sur le seul cas où le blocage serait
une surprise — le TTY.

### Trois voies écartées, chacune avec son motif

1. **Éditer les deux sites existants et laisser `--remote` tel quel.** Le geste minimal, et il produit trois
   comportements divergents sur un même drapeau tout en laissant ouvert le faux vert de la section Contexte. Le
   ticket ne le demande pas ; la conception le refuse parce que le correctif traverse ce site.
2. **Faire de `-` un alias déprécié et pousser vers l'inférence.** Casserait `_arch_ask` à la première
   dépréciation effective, pour un gain nul : les deux formes coexistent sans ambiguïté, la table de décision
   n'a pas de case en conflit.
3. **Lire stdin même quand le positionnel est présent, et concaténer.** Aucune demande, et une régression
   silencieuse pour tout appelant qui passe un message court avec un stdin hérité non vide — dont `_arch_ask`
   n'est pas loin.

---

## Phases d'implémentation

### Phase 1 — Le lecteur unique

- Créer `crates/mika-cli/src/ask_message.rs` : `AskMessageError`, `resolve_ask_message` (pure, générique sur
  `Read`), `resolve_from_process_stdin`.
- Déclarer `pub mod ask_message;` dans `crates/mika-cli/src/lib.rs`.
- Tests unitaires du module couvrant les quatre lignes de la table + la population vide non-TTY.

### Phase 2 — Le positionnel devient optionnel

- `crates/mika-cli/src/cli.rs` : `pub message: Option<String>`, aide réécrite.
- Corriger les sites compilés qui en découlent — **énumérés ci-dessous, pas estimés**.

#### Le ripple, énuméré et borné (F3)

`pub message: String` → `Option<String>` est un changement sur une structure publique du crate ; R8 ne
peut prétendre « aucun changement au reste de la surface » qu'en montrant la borne plutôt qu'en la supposant.
Elle est établie ici, au temps du plan, et non renvoyée à l'implémentation.

| site | ce qu'il fait de `message` | effet du changement |
|---|---|---|
| `main.rs:137` (`run_team_ask`) | `&args.message` | réécrit — reçoit la `String` résolue (Phase 3) |
| `main.rs:332` (`run_remote`) | `&args.message` | réécrit — reçoit la `String` résolue (Phase 3) |
| `main.rs:353` (`ask::run`) | `&args.message` | réécrit — reçoit la `String` résolue (Phase 3) |
| `cli.rs:1440` | **ne touche pas `message`** — assertion sur `args.continue_session` | inchangé |
| tests `main.rs:865-955` (7 tests) | passent `"hello world"` en positionnel, assertent sur `format`, `model` | inchangés — aucun n'accède à `args.message` |

Le hedge « s'il touche `message` » de la première rédaction est levé : `cli.rs:1440` est
`Some(Commands::Ask(args)) => assert!(args.continue_session)`. Il ne touche pas le champ.

**Personne ne construit `AskArgs` à la main.** La structure n'a que deux mentions dans tout l'arbre — sa
déclaration (`cli.rs:228`) et la variante qui la porte (`cli.rs:67`) ; aucun littéral `AskArgs { … }` n'existe,
dans ce crate ni ailleurs. Elle n'est instanciée que par clap au parse. Et `lib.rs` n'expose que `remote_ask` et
`supervision` : `cli` n'est pas un module public, donc aucun crate tiers ne peut construire ni filtrer sur
`AskArgs`. Le ripple est donc **exactement les trois sites de `main.rs`**, tous réécrits en Phase 3.

*Citation : review-guide § Orthogonality — « changes propagate minimally » se démontre par l'énumération de la
propagation, pas par le geste vers elle ; doctrine de la Unresolved-Decision Gate (mika#1244) — un fait de
conception vérifiable au temps du plan se vérifie au temps du plan.*

### Phase 3 — Câblage des trois chemins

- `main.rs`, bras `Commands::Ask` : résoudre le message **une fois**, en amont du branchement `--team`.
- Passer la `String` résolue à `run_team_ask`, `run_remote`, `ask::run`.
- **Supprimer** les lectures internes `ask.rs:268-275` et `ask.rs:757-764`.
- Vérifier que le plafond `MAX_CALLBACK_RESULT` et le `bail!("Empty message…")` restent en vigueur sur le message
  résolu (R8).

### Phase 4 — Garde structurelle et tests

#### Le garde R6 : son périmètre est un file set, sa règle de match couvre les trois écritures (F2)

Modèle : `mika2220_no_local_reparse_of_the_llm_bodies_env_var`. Ce modèle scanne **une** chaîne à orthographe
unique ; `std::io::stdin()` n'en a pas, donc le garde doit nommer son périmètre et sa règle plutôt que les
laisser à l'intuition — un garde dont la règle est plus floue que la chose gardée n'est pas simple, il est
poreux (*review-guide § KISS*).

**Périmètre — un file set concret, pas une notion de call-graph :**

```
crates/mika-cli/src/main.rs
crates/mika-cli/src/commands/ask.rs
crates/mika-cli/src/remote_ask.rs
```

`crates/mika-cli/src/ask_message.rs` est **exclu par construction** : c'est le lecteur, sa raison d'être est de
contenir les deux appels impurs. `cli.rs` est hors périmètre — il ne porte que la déclaration clap et aucune
lecture (sa seule occurrence du mot est le texte d'aide, sans parenthèse, donc hors de la règle de match
ci-dessous).

`main.rs` est **inclus** bien qu'il porte toutes les sous-commandes, et l'état actuel autorise ce choix sans
allowlist : `main.rs` et `remote_ask.rs` ne contiennent **aucune** occurrence de `stdin` aujourd'hui — les huit
sous-commandes qui lisent légitimement stdin (`credential_helper`, `setup`, `agents`, `config`, `tasks`, `kg`,
`skills`, `provider`, `teams`) le font toutes depuis leur propre module `commands/*.rs`, jamais depuis le
dispatch. L'inclure ferme le trou qu'un périmètre `{ask.rs, remote_ask.rs}` laisserait ouvert : une
ré-implémentation de la lecture directement dans le bras `Commands::Ask`, c'est-à-dire précisément là où la
résolution est câblée et donc l'endroit le plus probable d'un second lecteur.

**Règle de match — une regex sur le site d'appel, pas sur le chemin qualifié :**

```
\bstdin\s*\(
```

Elle couvre les trois écritures atteignables, dont deux sont **déjà employées** ailleurs dans le crate :

| écriture | d'où elle vient | employée aujourd'hui |
|---|---|---|
| `std::io::stdin()` | qualifiée complète | oui (majorité des sites) |
| `io::stdin()` | après `use std::io;` | oui (`agents.rs`, `teams.rs`) |
| `stdin()` | après `use std::io::stdin;` | non, mais atteignable |

Un garde ancré sur `std::io::stdin` seul serait donc évadé par un `use` que le crate pratique déjà — c'est le cas
de figure exact que F2 nomme. La règle porte sur le site d'appel, qui est invariant par import.

**Elle couvre `is_terminal` sans second prédicat.** Tester la tty-ness de l'entrée exige d'en tenir le handle :
les deux formes en usage dans le crate — `std::io::stdin().is_terminal()` et
`std::io::IsTerminal::is_terminal(&std::io::stdin())` — contiennent l'une et l'autre `stdin(`. Un second
prédicat sur `is_terminal` n'attraperait rien de plus et ajouterait une surface de faux positif.

**Faux positifs vérifiés absents sur le file set.** La forme parasite est `.stdin(Stdio::piped())` du builder
`Command` (présente dans `tui/input.rs`, hors périmètre) : ni `ask.rs` ni `remote_ask.rs` ni `main.rs` ne
construisent de `Command`/`Stdio`. Le file set est donc propre après Phase 3, sans exception.

**Limite assumée, écrite plutôt que découverte :** `use std::io::stdin as lire;` échappe à la règle. Ce garde
borne l'inattention — un second lecteur écrit de bonne foi, qui est la forme qu'ont prise les trois divergences
citées en Contexte — et non l'évasion délibérée, qu'aucun scan de source n'atteint.

**Fixture d'évasion (obligatoire, pas décorative) :** le test du garde inclut au moins un cas
`io::stdin()` — l'écriture importée que le crate pratique déjà — vérifiant que la règle le rejette. Sans cette
fixture, un garde qui n'attrape que la forme qualifiée passerait au vert en ne gardant rien, ce qui est la
panne silencieuse que tout ce garde existe pour fermer.

#### Test de non-régression `--remote`

La résolution est appliquée avant `run_remote` (R5). Le site est vérifié structurellement plutôt qu'en réseau —
le point à tenir est que `run_remote` ne reçoit plus jamais `"-"`.

### Phase 5 — Documentation

- `docs/getting-started.md:307,311` : ajouter la forme sans sentinelle à côté de la forme `-`, sans retirer
  cette dernière (elle reste la forme canonique d'un appelant scripté, et `_arch_ask` l'emploie).
- Aide clap (`cli.rs`) : les trois portes nommées.
- Ne **pas** toucher au commentaire de `_arch_ask` : son choix de `-` reste correct et explicite.

---

## Fire-Disposition

Ce plan livre **deux détecteurs** — des livrables dont le chemin de succès est « aucune violation trouvée ».
mika#1574 exige que la disposition d'échec soit pré-spécifiée *avant* la livraison, pour la raison qui décide
tout le reste : un détecteur dont la disposition se décide au moment où il tire se décide sous la pression de
faire passer le build, et cette pression a une réponse par défaut — allowlister — qui vide le détecteur de son
objet sans que rien ne le dise.

### Détecteur 1 — garde de source R6 (`std::io::stdin()` sur le chemin `ask`)

**Disposition : (a) exception par allowlist nommée — zéro entrée.**

Le garde est livré avec une allowlist **vide**, et cette vacuité est l'invariant, pas l'état initial. Trois cas,
et deux d'entre eux ne sont pas des exceptions :

1. **Il tire sur `ask.rs:271` ou `ask.rs:760`** — les deux lecteurs que la Phase 3 supprime. Ce n'est pas un
   « tir sur données existantes » : c'est la Phase 3 qui n'a pas été faite. La disposition est de la faire.
2. **Il tire sur un site du file set que ce plan n'a pas prévu.** Le plan affirme que le file set est propre
   après Phase 3, et cette affirmation est vérifiée au temps du plan : `main.rs` et `remote_ask.rs` ne
   contiennent aucune occurrence de `stdin` aujourd'hui (Phase 4). Si un site apparaît malgré tout, la
   disposition est de **le replier dans `ask_message.rs`** — c'est-à-dire d'en faire un appelant du lecteur
   unique — jamais de l'allowlister. C'est ce que R6 énonce et ce que les trois divergences citées en Contexte
   (`grooming_marker`, `parse_log_llm_bodies`, `LlmUsage::accumulate`) ont coûté quand la règle n'existait pas.
3. **Le repliement est structurellement impossible** — un site qui aurait besoin de stdin sur le chemin `ask`
   pour une raison étrangère à la résolution du message. Aucun n'est connu, et aucune hypothèse n'en est faite
   ici. Ce cas est **hors périmètre de ce plan** : il rouvrirait la question de savoir si le chemin `ask` a un
   second usage légitime de stdin, ce qui est une décision d'opérateur et non un réglage de garde. La
   disposition est alors de **halter et de la poser**, pas d'ajouter une entrée pour débloquer le build.

**Ce que ce détecteur ne peut pas voir**, dit ici pour que son silence ne se lise pas comme une preuve :
l'aliasing d'import (`use std::io::stdin as …`) et tout site hors du file set de la Phase 4. Le garde borne la
divergence de bonne foi, qui est la forme mesurée du défaut, pas l'évasion.

### Détecteur 2 — test structurel `--remote` (`run_remote` ne reçoit jamais `"-"`)

**Disposition : (a) exception par allowlist nommée — zéro entrée, et aucune n'est concevable.**

Ce détecteur n'affirme pas un invariant sur du code préexistant : il affirme la propriété **que la Phase 3
crée**. Il ne peut donc pas « tirer sur des données existantes » au sens de mika#1574 — il ne peut échouer que
si la Phase 3 est incomplète ou régresse plus tard. Dans les deux cas la disposition est **de casser le build**,
sans exception et sans entrée d'allowlist : un `"-"` qui atteint `run_remote` est très exactement le faux vert
de la section Contexte, c'est-à-dire une réponse plausible à une question jamais posée. Une allowlist ici
rouvrirait le défaut que le ticket ferme.

La disposition est écrite même si son cas d'application est trivial, parce que « trivialement (a) » et « non
spécifié » produisent le même texte dans un plan et des conduites opposées le jour où le test rougit.

*Citation : mika#1574 (corps d'issue — « the scope-bind MUST pre-specify the failure-disposition for when the
detector fires on existing data ») ; Fire-Disposition Gate du skill groom-ticket, branche 2.*

---

## Contrat de vérification

- `cargo test -p mika-cli` — vert, dont les nouveaux tests d'`ask_message` et la garde structurelle.
- `cargo clippy --workspace --all-targets` — sans avertissement nouveau.
- `cargo fmt --check`.
- Probe manuelle, les quatre lignes de la table, sur un binaire construit :
  ```bash
  echo "ping" | mika ask --agent mika-arch            # AC1 — doit répondre à "ping"
  mika ask --agent mika-arch "ping"                   # AC2 — inchangé
  mika ask --agent mika-arch                          # AC3 — erreur d'usage, pas de blocage
  echo "ping" | mika ask --agent mika-arch -          # R4 — inchangé
  ```
- **Probe de non-régression du grooming (la plus importante) :** un `_arch_ask` réel, c'est-à-dire un
  dev-groom dispatché de bout en bout, doit produire un verdict architecte sur le contenu du plan et non sur un
  message d'un octet. **Halte :** si l'architecte rend un verdict qui ne cite pas le plan, désarmer et vérifier
  que la lecture `-` du chemin local est intacte **avant** de toucher au reste.

---

## Definition of Done

- Le lecteur unique existe, est pur, et est le seul site `std::io::stdin()` du chemin `ask`.
- Les trois chemins (local, `--team`, `--remote`) partagent cette résolution.
- `-` fonctionne exactement comme avant sur local et `--team`, et fonctionne **désormais** sur `--remote`.
- La garde structurelle est livrée avec une allowlist vide, son file set et sa règle de match écrits au site du
  garde, et une fixture d'évasion (`io::stdin()`) qui rougit si la règle ne couvre que la forme qualifiée.
- La section `## Fire-Disposition` nomme la disposition des deux détecteurs, et le code du garde porte en
  commentaire la phrase qui la rend contraignante : *quand elle tire, on retire le second lecteur, on ne
  l'allowliste pas*.
- La documentation nomme les trois portes sans retirer la sentinelle.
- `cargo test`, `clippy`, `fmt` verts.

---

## Acceptance criteria

Transcrits du corps de mika#1982, plus les deux que la lecture du code ajoute (AC5, AC6) et qui sont la part non
demandée mais traversée par le correctif.

1. `echo "hello" | mika ask --agent x` fonctionne — le message reçu est `hello`.
2. `mika ask --agent x "hello"` est inchangé, y compris quand stdin porte des données (stdin n'est alors pas lu).
3. stdin TTY **et** MESSAGE absent → l'erreur d'usage, immédiate, sans blocage silencieux en lecture.
4. Un test couvre les trois cas ci-dessus, sans dépendre d'un TTY réel (la fonction de résolution est pure et
   paramétrée).
5. `mika ask … -` reste lu depuis stdin sur les chemins local et `--team` — non-régression de `_arch_ask`
   (`dispatch-lib.sh:4782`) et de `docs/getting-started.md:307,311`.
6. `--remote` applique la même résolution : `cat f | mika ask --remote <url> -` envoie le contenu de `f`, et non
   la chaîne `"-"`.
7. `std::io::stdin()` n'apparaît qu'une fois sur le chemin `ask`, tenu par une garde de source à allowlist vide,
   dont le file set et la règle de match sont écrits (Phase 4) et dont la disposition de tir est pré-spécifiée
   (§ Fire-Disposition).

---

## Revision history

- **rev 2 (2026-09-21)** — révision sur findings de première passe architecte (`.iterate/findings-1.md`).
  - **F1 (bloquant) adressé** par l'ajout d'une section `## Fire-Disposition` couvrant les deux détecteurs
    (garde de source R6, test structurel `--remote`). Option **(a) exception par allowlist nommée — zéro
    entrée** pour les deux, avec, pour le garde R6, la distinction des trois cas de tir : Phase 3 non faite
    (faire la phase), site imprévu du file set (replier dans `ask_message.rs`, jamais allowlister), repliement
    impossible (halter — décision d'opérateur hors périmètre). Pour le test `--remote`, la disposition est
    écrite bien que triviale, parce que « trivialement (a) » et « non spécifié » produisent le même texte et des
    conduites opposées. La limite de chaque détecteur (aliasing d'import, hors-file-set) est nommée pour que son
    silence ne se lise pas comme une preuve. Citation mika#1574 préservée.
  - **F2 adressé** en Phase 4 : le périmètre devient un **file set concret** (`main.rs`, `commands/ask.rs`,
    `remote_ask.rs` ; `ask_message.rs` exclu par construction) avec le motif d'inclusion de `main.rs` écrit, et
    la règle de match devient `\bstdin\s*\(` — portant sur le **site d'appel**, invariant par import. La table
    des trois écritures montre que deux d'entre elles (`std::io::stdin()`, `io::stdin()`) sont **déjà** en usage
    dans le crate, donc qu'un garde ancré sur la forme qualifiée serait évadé par un `use` que le code pratique.
    Ajout : la couverture gratuite de `is_terminal` (les deux formes contiennent `stdin(`), la vérification que
    le file set ne porte aucun `.stdin(Stdio::piped())`, et une **fixture d'évasion obligatoire** sur
    `io::stdin()`. Citation review-guide § KISS préservée.
  - **F3 adressé** en Phase 2 : le hedge « s'il touche `message` » est **levé par vérification** plutôt que
    reformulé. `cli.rs:1440` est `assert!(args.continue_session)` — il ne touche pas le champ. Table
    d'énumération des cinq populations de sites, plus la borne externe : `AskArgs` n'a que deux mentions dans
    l'arbre (déclaration + variante), aucun littéral ne le construit, et `lib.rs` n'exporte que `remote_ask` et
    `supervision` — `cli` n'étant pas public, aucun crate tiers ne peut le construire. Le ripple est exactement
    les trois sites de `main.rs`. Citation review-guide § Orthogonality + doctrine mika#1244 préservée.
  - Répercussions : `## Definition of Done` et AC7 étendus pour que la fire-disposition et la fixture d'évasion
    soient des conditions de livraison et non des intentions.
  - Aucun AC affaibli, aucun retiré. Aucune décision de conception de la rev 1 renversée.
