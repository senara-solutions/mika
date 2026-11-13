# mika#2790 — La garde de déclaration lit les constantes, pas un seul littéral : `operator-review` et la famille `OPERATOR_HELD_LABELS` entrent dans le test

> Plan content-only produit par `/mika-groom-plan-only`. Ticket : senara-solutions/mika#2790.
> Labels portés : `enhancement`, `p3-nice-to-have`, `agent-core`, `ready`, `dispatch:loop`.

---

## 1. Contexte

### 1.1 La classe de défaut, et ce qui en est déjà gardé

`.github/labels.yml` est la source de vérité du vocabulaire de labels, et le
workflow `label-sync` tourne avec `delete-other-labels: true`. Un label que le
code pose ou lit mais que ce fichier ne déclare pas est donc **supprimé du dépôt
et de chaque ticket qui le portait**, sans événement `unlabeled` et sans ligne de
journal. C'est la classe mika#2127, et le `CLAUDE.md` racine la nomme :
« un label d'enforcement non déclaré échoue en silence », trois occurrences à ce
jour (`dispatch:ssc`, puis `operator-review`/`blocked`).

Deux gardes structurelles existent aujourd'hui, et elles sont partielles :

| garde | site | ce qu'elle couvre |
|---|---|---|
| `test_refusal_label_is_declared_in_labels_yml` | `crates/mika-agent/src/auto_pull.rs:5687` | **un seul** littéral : `REFUSAL_LABEL` (`operator-gated`, `auto_pull.rs:297`) |
| `labels_this_module_writes_are_declared_in_labels_yml` | `crates/mika-agent/src/wip_rescue.rs:2524` | `WIP_RESCUE_LABEL` et `HUMAN_REVIEW_LABEL` — exhaustive **pour ce module** |

Le doc comment de la seconde dit littéralement *« The equivalent guard for
`auto_pull.rs` remains mika#2127 AC2 »*. Ce ticket-ci est ce reste.

### 1.2 Les littéraux non gardés, mesurés dans ce worktree

| site | ligne | littéral | rôle |
|---|---|---|---|
| `auto_pull.rs` — `abandon_stuck_ready` | `3125` | `"operator-review"`, en dur dans l'appel `gh_apply_label` | le label dont l'apposition **conditionne** l'abandon : son échec fait `return` et laisse le ticket intact |
| `auto_pull.rs` — `abandon_stuck_ready` | `3126` | `"operator-review"` dans le message `warn!` | diagnostic de cet échec |
| `auto_pull.rs` — `AbandonReason::comment_body` | `3083`, `3085` | `"operator-review"` en prose | nomme à l'opérateur le label *qui vient d'être posé* |
| `webhook_dispatch.rs` | `525` | `OPERATOR_HELD_LABELS = &["blocked", "operator-review", "operator-gated"]` | exclusion des tickets tenus par l'opérateur — lue par `operator_held_label` (`:532`) et `label_read_by_loop` (`:586`) |

État de déclaration **mesuré le 2026-10-19** par `grep -n '^- name: '
.github/labels.yml` :

| label | ligne dans `labels.yml` |
|---|---|
| `ready` | 112 |
| `operator-gated` | 116 |
| `operator-review` | 120 |
| `blocked` | 124 |

Les quatre sont déclarés. Rien ne garantit qu'ils le resteront : c'est tout
l'objet du ticket.

### 1.3 Un fait trouvé au grooming : deux doc comments affirment l'inverse

`git log -S'- name: operator-review' -- .github/labels.yml` ne rend **qu'un**
commit : `46eeef98`, *« fix(labels): declare operator-review and blocked —
auto_pull les applique depuis toujours sans qu'ils existent »* (PR #2128,
**2026-09-01**). Même commit pour `blocked`.

Or deux doc comments du même fichier, écrits le même jour mais **avant** que
#2128 n'atterrisse, affirment au présent le contraire :

- `auto_pull.rs:242-247`, sur `REFUSAL_LABEL` :
  *« `operator-review` does not exist: it is absent from `gh label list` and
  undeclared in `.github/labels.yml` — verified 2026-09-01 […] `blocked`, the
  module's other exclusion label, is equally absent. »*
- `auto_pull.rs:5682-5684`, doc du test lui-même :
  *« `operator-review` is referenced a dozen times in this module and declared
  nowhere »*.

Ces phrases étaient vraies à l'écriture et fausses depuis `46eeef98`. Elles
citent aussi `labels.yml:102` et `:106` pour `ready` et `operator-gated`, qui
sont aujourd'hui aux lignes 112 et 116 — une citation par numéro de ligne qui a
déjà dérivé une fois.

**Pourquoi c'est dans le périmètre et non un extra.** Le livrable du ticket est
une assertion qui dit *« `operator-review` est déclaré »*. La laisser atterrir à
dix-huit lignes d'un doc comment qui dit *« `operator-review` n'est déclaré
nulle part »* produit un fichier qui se contredit, et le prochain lecteur devra
choisir lequel croire. Le plan réconcilie les deux passages en **historisant**
l'affirmation (« ce que mika#2123 a mesuré le 2026-09-01, et ce que #2128 a
changé ») plutôt qu'en la supprimant : la raison pour laquelle la porte de
promotion refuse avec `operator-gated` et non `operator-review` reste lisible.

### 1.4 Deux faiblesses du prédicat `declared`, et ce que le plan en fait

Le prédicat actuel est `yml.contains(&format!("- name: {name}"))`. Deux défauts,
tous deux rendus concrets par l'entrée de `blocked` dans l'ensemble gardé :

1. **Non ancré à droite.** `declared("block")` rend `true` aujourd'hui, parce que
   `- name: blocked` *contient* `- name: block`. Le jour où quelqu'un ajoute
   `blocked-upstream` et retire `blocked`, la garde reste verte sur exactement le
   retrait qu'elle existe pour attraper. Aucun label déclaré n'est préfixe d'un
   autre au 2026-10-19 — c'est une faiblesse latente, pas un faux négatif actuel.
2. **Aveugle aux noms cités.** `labels.yml` entoure de guillemets tout nom
   contenant un `:` — `"dispatch:loop"`, `"origin:manual"`, `"phase:1"` (12
   entrées). Le prédicat les lit comme non déclarés. Aucun membre de
   l'ensemble gardé n'en est aujourd'hui, mais `OPERATOR_HELD_LABELS` est une
   constante qui grandit, et un futur `operator:hold` rendrait la garde rouge
   pour une raison fausse.

Le ticket demande « le même harnais (`include_str!` de `labels.yml` et le
prédicat `declared`) ». Le plan garde le harnais — même fichier, même closure,
même nom — et durcit son corps de deux lignes, **avec un contrôle pour chaque
durcissement** (§ D3). Un prédicat qui rend `true` à tort est la seule manière
dont cette garde peut mentir.

---

## 2. Les livrables, site par site

### D1 — `ABANDON_LABEL` : le littéral de `auto_pull.rs:3125` devient une constante nommée

Dans `crates/mika-agent/src/auto_pull.rs`, juste **après** `REFUSAL_LABEL`
(`:297`), pour que les deux labels que ce module pose se lisent ensemble :

```rust
/// Le label que l'abandon de re-drive pose (mika#2020 R8, constante depuis
/// mika#2790).
///
/// Distinct de [`REFUSAL_LABEL`] : celui-ci marque *« l'auto-pull a renoncé,
/// un humain doit regarder »*, celui-là *« la porte de promotion a refusé »*.
/// Les deux sont dans [`crate::webhook_dispatch::OPERATOR_HELD_LABELS`], donc
/// les deux excluent le ticket des trois phases.
///
/// Constante et non littéral parce que son apposition **conditionne**
/// l'abandon : [`abandon_stuck_ready`] fait `return` quand elle échoue, et
/// elle échoue si `.github/labels.yml` ne déclare pas ce nom — c'est la
/// classe mika#2127, 48 fois en production. La garde est
/// [`tests::test_exclusion_labels_are_declared_in_labels_yml`], et elle lit
/// cette constante : un test qui recopierait le nom garderait sa copie, pas
/// le nom que le code emploie.
const ABANDON_LABEL: &str = "operator-review";
```

Puis les quatre sites du § 1.2 lisent la constante :

| site | avant | après |
|---|---|---|
| `:3125` | `gh_apply_label(label_auth, issue_number, "operator-review")` | `gh_apply_label(label_auth, issue_number, ABANDON_LABEL)` |
| `:3126` | `"auto_pull: abandon could not apply operator-review label; …"` | garder le texte, ajouter le champ structuré `label = ABANDON_LABEL` (même forme qu'au site de refus, `:2930`) |
| `:3083` | `` `operator-review` (posé ci-dessus) `` | `` `{ABANDON_LABEL}` (posé ci-dessus) `` |
| `:3085` | `` et `operator-review` posé `` | `` et `{ABANDON_LABEL}` posé `` |

Les deux occurrences de `comment_body` sont dans un `format!` ; l'argument nommé
en ligne résout la constante, et le **rendu est identique caractère pour
caractère** — les tests `:7742` et `:7757`, qui assertent
`body.contains("operator-review")`, restent verts sans modification. C'est
voulu : ils vérifient ce que l'opérateur lit, pas comment c'est construit.

Les mentions de `operator-review` restées en **commentaires** et en prose de
doc comment ne sont pas touchées (hors § D2) : un commentaire n'est pas un site
d'emploi.

### D2 — réconcilier les deux doc comments périmés du § 1.3

**`auto_pull.rs:242-247`** — l'affirmation devient datée et la suite est
nommée. Forme attendue (le fond est conservé : *pourquoi* la porte refuse avec
`operator-gated`) :

> **Not `operator-review`, and that is a measured constraint, not a style
> choice.** Le 2026-09-01, `operator-review` n'existait pas : absent de
> `gh label list` et non déclaré dans `.github/labels.yml`, alors que `ready`
> et `operator-gated` l'étaient — le fichier est la source de vérité.
> `blocked`, l'autre label d'exclusion du module, était également absent.
> `46eeef98` (#2128) a depuis déclaré les deux ; ce qui suit est la mesure qui
> a décidé de ce nom-ci, pas l'état actuel du registre, et
> [`tests::test_exclusion_labels_are_declared_in_labels_yml`] (mika#2790) tient
> maintenant les quatre noms.

Les deux citations par numéro de ligne (`labels.yml:102`, `:106`) perdent leur
numéro : elles ont déjà dérivé de dix lignes, et le nom du label suffit à
retrouver l'entrée. Même traitement au point 2 de la liste « lifted manually »
(`:278`), qui cite `.github/labels.yml:106`.

**`auto_pull.rs:5680-5685`** — le doc comment du test est réécrit par D3.

### D3 — le test étendu

Même site (`auto_pull.rs`, module `tests`), même harnais. Le test est **renommé**
`test_exclusion_labels_are_declared_in_labels_yml` : il garde désormais quatre
noms venus de deux modules, et `refusal_label` au singulier ne décrirait plus ce
qu'il lit. Son ancien nom est cité dans son propre doc comment, pour qu'un `grep`
parti de la note de solution de mika#2127 atterrisse (le renommage a **un seul**
autre site, traité en D5).

Doc comment attendu — il doit porter trois choses : la classe de défaut, la
raison de l'itération sur la constante, et la raison du durcissement du
prédicat.

Corps attendu :

```rust
#[test]
fn test_exclusion_labels_are_declared_in_labels_yml() {
    let yml = include_str!("../../../.github/labels.yml");

    // Ancré à droite sur la fin de ligne, et tolérant au nom cité :
    // `labels.yml` entoure de guillemets tout nom portant un `:`
    // (`"dispatch:loop"`). Un `contains("- name: {name}")` nu rendrait `true`
    // pour `block` tant que `blocked` est déclaré, et `false` pour tout nom
    // cité — les deux contrôlés ci-dessous.
    let declared = |name: &str| {
        yml.lines().any(|line| {
            line.strip_prefix("- name: ")
                .map(str::trim)
                .is_some_and(|d| d == name || d.trim_matches('"') == name)
        })
    };

    // -- contrôles du prédicat --
    // Positif, forme nue.
    assert!(declared("ready"), "labels.yml must declare `ready`");
    // Positif, forme citée : sans lui, le durcissement n'est pas couvert.
    assert!(
        declared("dispatch:loop"),
        "the predicate must see a quoted entry; labels.yml quotes every name \
         carrying a `:`"
    );
    // Négatif : un prédicat qui répond `true` à tout ne prouve rien.
    assert!(
        !declared("a-label-nobody-has-ever-declared"),
        "the guard must be able to detect an undeclared label"
    );
    // Négatif d'ancrage : `blocked` est déclaré, `block` ne l'est pas. Un
    // prédicat non ancré resterait vert au retrait de `blocked` au profit
    // d'un `blocked-*`.
    assert!(
        !declared("block"),
        "the predicate must anchor on the whole name, not on a prefix of it"
    );

    // -- l'ensemble gardé : chaque entrée nomme son site d'emploi --
    let mut guarded: Vec<(&str, &str)> = vec![
        (REFUSAL_LABEL, "auto_pull::refuse_promotion (REFUSAL_LABEL)"),
        (ABANDON_LABEL, "auto_pull::abandon_stuck_ready (ABANDON_LABEL)"),
    ];
    // Itération sur la constante, jamais sur une liste recopiée : un label
    // ajouté à OPERATOR_HELD_LABELS est couvert le jour où il atterrit, pas
    // le jour où quelqu'un y repense (même idiome que
    // `mika2361_each_held_label_names_itself`).
    for label in crate::webhook_dispatch::OPERATOR_HELD_LABELS {
        guarded.push((
            label,
            "webhook_dispatch::OPERATOR_HELD_LABELS — lu par operator_held_label \
             et label_read_by_loop (exclusion des trois phases)",
        ));
    }

    // Contrôle positif de l'itération : une constante vide rendrait la boucle
    // vacante et la garde verte pour rien.
    assert!(
        !crate::webhook_dispatch::OPERATOR_HELD_LABELS.is_empty(),
        "OPERATOR_HELD_LABELS must be non-empty, or the loop above guards nothing"
    );

    // Un seul panic, portant TOUS les sites : `operator-review` est employé
    // par deux d'entre eux, et un opérateur qui le retire de labels.yml doit
    // lire les deux, pas le premier.
    let offenders: Vec<String> = guarded
        .iter()
        .filter(|(label, _)| !declared(label))
        .map(|(label, site)| format!("  `{label}` — employé par {site}"))
        .collect();
    assert!(
        offenders.is_empty(),
        "`.github/labels.yml` ne déclare pas {} des labels que le code pose ou \
         lit. `label-sync` tourne avec `delete-other-labels: true` : le label \
         est supprimé du dépôt ET de chaque ticket qui le portait, sans \
         événement ni journal, et le site ci-dessous échoue en silence \
         (classe mika#2127, 48 occurrences en production). Déclare-le dans \
         `.github/labels.yml` :\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}
```

Le `vec!` est `mut` puis étendu, et non un `chain` : la provenance de chaque
entrée reste lisible à la lecture, et la paire `(label, site)` est ce qui
satisfait l'AC (« en nommant le site qui l'emploie »).

### D4 — `webhook_dispatch.rs` : la constante nomme sa garde

Au doc comment de `OPERATOR_HELD_LABELS` (`:525`), trois lignes :

> Chacun de ces noms doit être déclaré dans `.github/labels.yml` —
> `label-sync` tourne avec `delete-other-labels: true`, et un nom non déclaré
> est supprimé du dépôt et de chaque ticket sans bruit. La garde itère sur
> **cette** constante :
> `auto_pull::tests::test_exclusion_labels_are_declared_in_labels_yml`
> (mika#2790). Un label ajouté ici est donc couvert sans toucher au test ; il
> faut en revanche le déclarer dans `labels.yml` dans le même commit.

C'est la moitié qui manque pour que la garde soit trouvable depuis le site
qu'elle protège : le test vit dans un autre module, et rien ne le dit ici
aujourd'hui.

### D5 — la note de solution de mika#2127

`docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md`,
seul autre site citant l'ancien nom du test (`:77`, dans un bloc de code
illustratif). Mise à jour :

- le nom du test dans l'extrait ;
- l'extrait montre l'itération sur la constante, puisque c'est la leçon que
  mika#2790 ajoute à la note : **la garde lit la liste que le code lit, elle
  n'en tient pas une copie** ;
- la phrase « Le test coûte quatre lignes et ferme la classe entière » devient
  exacte sur ce qu'elle ferme : la classe est fermée pour les labels **atteignables
  depuis une constante**, et un littéral en dur reste hors de portée — d'où D1 ;
- les deux citations `.github/labels.yml:106` perdent leur numéro de ligne (même
  dérive qu'au § 1.3).

Hors du compte de lignes (`docs/`). Pas de contrainte `docs-sync` :
`docs/solutions/` n'est pas miroité dans `crates/mika-agent/docs/` (vérifié par
`ls crates/mika-agent/docs/`).

---

## Fire-Disposition

Le vol livre **un détecteur** : D3, un test dont le chemin de succès est
« aucune violation trouvée ». D1, D2, D4 et D5 n'en sont pas (une constante, des
doc comments, une note).

**Option retenue : (a) exception nommée en allowlist — et l'ensemble
d'exceptions est VIDE.** Ce n'est pas une facilité : c'est mesuré.

Le rayon d'action du détecteur est exactement trois lectures — `REFUSAL_LABEL`,
`ABANDON_LABEL`, et les membres de `OPERATOR_HELD_LABELS`. Il n'y a ni scan de
fichier, ni glob, ni liste à découvrir : l'ensemble gardé est énumérable au
grooming. Recensement complet, au 2026-10-19 (`grep -n '^- name: '
.github/labels.yml`) :

| label | venu de | déclaré ? | ligne |
|---|---|---|---|
| `operator-gated` (`REFUSAL_LABEL`) | `auto_pull.rs:297` | oui | 116 |
| `operator-review` (`ABANDON_LABEL`) | `auto_pull.rs`, D1 | oui | 120 |
| `blocked` | `OPERATOR_HELD_LABELS[0]` | oui | 124 |
| `operator-review` | `OPERATOR_HELD_LABELS[1]` | oui | 120 |
| `operator-gated` | `OPERATOR_HELD_LABELS[2]` | oui | 116 |

Zéro violation. Le détecteur atterrit donc **armé**, sans `#[ignore]`, sans
allowlist, et sans ticket de suivi — il n'y a rien à exempter et rien à armer
plus tard. `46eeef98` (#2128) avait déjà corrigé la seule violation que cette
classe ait connue ; le ticket est explicite là-dessus (« la cause de mika#2127
est déjà corrigée, et ce ticket ferme la porte à sa récidive »).

**Ce qui tient l'affirmation « zéro violation » honnête.** Une allowlist vide
est indistinguable d'un détecteur aveugle : les deux rapportent zéro. Ce qui
distingue les deux ici, ce sont les quatre contrôles du prédicat de D3 — deux
positifs (forme nue, forme citée) et deux négatifs (nom inventé, préfixe). Ils
sont le rôle que joue d'ordinaire « l'assertion auto-nettoyante qui rougit quand
l'exception devient stale » : ici, il n'y a pas d'exception à surveiller, mais
il y a un prédicat dont la capacité à voir une absence doit rougir le jour où
elle se perd. Le quatrième (`!declared("block")`) est celui qui rougirait si
quelqu'un ré-assouplissait le prédicat en `contains`.

S'ajoute le contrôle de non-vacuité sur `OPERATOR_HELD_LABELS` : une constante
vidée ferait passer la boucle sur zéro élément, et la garde serait verte en ne
gardant rien.

---

## 3. Ce que le vol ne fait PAS

- **Il ne garde pas « tout label apparaissant dans le code ».** Un scan de
  littéraux sur l'arbre serait un autre détecteur, avec d'autres faux positifs
  (noms en prose, fixtures de test, les 20+ `make_issue(…, &["ready",
  "operator-review"], …)` du module). Le ticket demande les sites de son
  tableau ; le plan les livre, et D1 est ce qui rend un site **atteignable**
  depuis une constante plutôt que de l'approcher par un scan.
- **Il ne déplace pas la garde dans `webhook_dispatch.rs`.** Le ticket dit
  « étendre le même test ». D4 est le renvoi qui rend la garde trouvable depuis
  la constante, pas un second test.
- **Il ne garde pas couleur et description** comme le fait
  `wip_rescue::the_declared_label_matches_the_one_the_daemon_creates`. Cette
  garde-là n'a de sens que pour un label que le démon **crée** au runtime avec
  une couleur ; `auto_pull` ne crée aucun label, il les pose.
- **Il ne touche pas aux fixtures de test** qui écrivent `"operator-review"` en
  dur dans des listes de labels d'`Issue` : ce sont des données d'entrée, pas
  des sites d'emploi.
- **Il ne change aucun comportement de production.** D1 est un remplacement
  littéral → constante de même valeur ; le rendu de `comment_body` est
  inchangé caractère pour caractère.

---

## 4. Contrat de vérification

Dans l'ordre, chacun avec le résultat attendu.

1. **Le détecteur est vert sur l'arbre tel quel.**
   `cargo test -p mika-agent --lib auto_pull::tests::test_exclusion_labels_are_declared_in_labels_yml -- --nocapture`
   → 1 passed.

2. **AC1, mesurée et non promise — retrait de `blocked`.** Avec l'outil `Edit`,
   retirer de `.github/labels.yml` les trois lignes de l'entrée `blocked`
   (`- name: blocked` + `color` + `description`). Relancer (1).
   Attendu : **rouge**, et le message nomme un site :

   ```text
   `.github/labels.yml` ne déclare pas 1 des labels que le code pose ou lit.
   […]
     `blocked` — employé par webhook_dispatch::OPERATOR_HELD_LABELS — lu par
       operator_held_label et label_read_by_loop (exclusion des trois phases)
   ```

   Restaurer : `git checkout -- .github/labels.yml`. Relancer (1) → vert.

3. **AC1, second cas — retrait de `operator-review`.** Même gesture sur l'entrée
   `operator-review`. Attendu : **rouge avec deux lignes d'offender**, une par
   site (`auto_pull::abandon_stuck_ready (ABANDON_LABEL)` et
   `webhook_dispatch::OPERATOR_HELD_LABELS`) — c'est ce que l'accumulation de D3
   achète sur un `assert!` par label. Restaurer, relancer → vert.

4. **AC2, le contrôle négatif survit.** Les quatre assertions de contrôle sont
   dans le corps du test et donc couvertes par (1). Vérification explicite du
   plus fragile : commenter temporairement l'assertion `!declared("block")`,
   remplacer le corps de `declared` par le `contains` d'origine, relancer →
   **vert** (c'est le faux positif que l'ancien prédicat laissait passer) ;
   restaurer le corps ancré et dé-commenter → `!declared("block")` rouge sur le
   `contains`, vert sur l'ancré. Reporter les deux observations dans le corps de
   PR. Restaurer par `git checkout --` du fichier source.

5. **Non-régression du rendu opérateur.**
   `cargo test -p mika-agent --lib auto_pull::tests` → la suite complète du
   module verte, en particulier les deux tests qui assertent
   `body.contains("operator-review")` (`:7742`, `:7757`) : ils prouvent que D1
   n'a pas changé ce que l'opérateur lit.

6. **Les trois gardes de la classe coexistent.**
   `cargo test -p mika-agent --lib labels_yml` (ou le filtre
   `declared_in_labels_yml`) → le test de `wip_rescue` et celui d'`auto_pull`
   passent tous deux ; aucun n'a été dupliqué ni cassé par le renommage.

7. **Lints et pipeline.** `cargo fmt --check`, `cargo clippy -p mika-agent
   --all-targets -- -D warnings`, `make verify-bundled-skills`,
   `scripts/verify-pipeline.sh origin/main`.

8. **Le renommage n'a pas laissé d'orphelin.**
   `grep -rn "test_refusal_label_is_declared_in_labels_yml" . --exclude-dir=.git`
   → exactement **une** occurrence, celle du doc comment du test qui cite son
   ancien nom comme ancre de `grep`. Au grooming, la mesure rendait deux sites
   (le test lui-même et la note de solution de D5).

---

## 5. Ordre de commit — contraint, pas esthétique

1. **D1** — `ABANDON_LABEL` + ses quatre sites. Doit précéder D3 : le test lit la
   constante, il ne compile pas sans elle.
2. **D3** — le test étendu et renommé. Après D1, avant D2 : c'est l'assertion
   qui rend caduque la prose de D2, et l'ordre fait que le commit de D2 peut
   citer le test par son nom final.
3. **D2** — réconciliation des doc comments du § 1.3.
4. **D4** puis **D5** — les deux renvois. En dernier parce qu'ils citent le nom
   final du test ; aucun des deux ne contraint l'autre.

Un seul commit est acceptable si le volume le justifie (il le justifie :
~114 lignes), à condition que le corps de PR garde la table des livrables.

---

## 6. Risques

| risque | probabilité | atténuation |
|---|---|---|
| Le durcissement du prédicat rend `false` pour une entrée réelle, et la garde rouge pour une raison fausse | faible | les deux contrôles positifs (nue + citée) de D3 l'attrapent au premier `cargo test` ; la mesure du § 1.4 énumère les deux seules formes que `labels.yml` emploie |
| `format!` avec argument nommé en ligne sur une `const` ne compile pas | très faible | l'idiome est déjà employé à `:913-914` du même fichier avec `{REFUSAL_LABEL}` |
| Le renommage casse un filtre `cargo test` dans un script ou un workflow | très faible | mesuré : un seul site hors du fichier (D5, une prose) ; vérifié par (8) |
| D2 efface une raison de conception en voulant la dater | moyenne | la forme attendue du § D2 **conserve** le paragraphe entier et n'ajoute qu'une phrase de datation plus le renvoi ; relire que « pourquoi `operator-gated` et pas `operator-review` » est toujours lisible après |
| Un lecteur futur croit la garde exhaustive sur tout l'arbre | moyenne | § 3 premier point, et le doc comment de D3 borne explicitement ce que le détecteur lit (trois constantes) |

---

## 7. Hors périmètre

- Garde générique « tout littéral de label dans le code est déclaré » (scan
  structurel) — autre détecteur, autre ticket si le besoin se présente.
- Garde couleur/description pour les labels d'`auto_pull` (le module n'en crée
  aucun).
- La famille `dispatch:*`, déjà gardée dans les deux sens par
  `scripts/check-dispatch-seats-declared.sh` (CI `dispatch-seats-lint`,
  mika#2092).
- `PILOT_LABEL_TURN_CEILINGS`, déjà gardée par
  `scripts/check-pilot-turn-ceiling-labels.sh`.
- Toute modification de `.github/labels.yml` : les quatre labels gardés y sont
  déjà déclarés (§ Fire-Disposition). Les mutations du § 4 sont temporaires et
  restaurées.

---

## Definition of Done

- [ ] `ABANDON_LABEL` existe dans `auto_pull.rs`, documentée, et les quatre
      sites du § 1.2 la lisent — aucun `"operator-review"` littéral ne subsiste
      dans le **code** d'`abandon_stuck_ready` ni de `comment_body`.
- [ ] `test_exclusion_labels_are_declared_in_labels_yml` asserte la déclaration
      de `REFUSAL_LABEL`, de `ABANDON_LABEL`, et de chaque membre de
      `crate::webhook_dispatch::OPERATOR_HELD_LABELS` — ce dernier **par
      itération sur la constante**, sans liste recopiée.
- [ ] Son message d'échec nomme, pour chaque label manquant, le ou les sites qui
      l'emploient, en un seul panic.
- [ ] Les quatre contrôles du prédicat sont présents : positif nu, positif cité,
      négatif inventé, négatif de préfixe.
- [ ] Le contrôle de non-vacuité de `OPERATOR_HELD_LABELS` est présent.
- [ ] Les deux doc comments du § 1.3 ne contredisent plus le registre ; la raison
      de conception (`operator-gated` et non `operator-review`) reste lisible.
- [ ] `OPERATOR_HELD_LABELS` nomme sa garde (D4).
- [ ] La note de solution de mika#2127 est à jour (D5).
- [ ] Les huit points du contrat de vérification (§ 4) sont exécutés, et les
      sorties observées des points 2, 3 et 4 figurent dans le corps de PR.
- [ ] `cargo fmt --check`, `cargo clippy -p mika-agent --all-targets -- -D
      warnings`, `cargo test -p mika-agent`, `make verify-bundled-skills` et
      `scripts/verify-pipeline.sh origin/main` passent.
- [ ] Corps de PR écrit dans `pr-body.md` à la racine du worktree, passé en
      `--body-file`, puis `rm pr-body.md` (mika#2211).
- [ ] `Code review: delegated to mika-qa (mika#2694)` dans le corps de PR ; ni
      `ce-simplify-code` ni `ce-code-review` invoqués.

---

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#2790 :

- **AC1.** Retirer `operator-review` ou `blocked` de `.github/labels.yml` fait
  échouer le test, **en nommant le site qui l'emploie**.
  → Mesuré par § 4 points 2 et 3. Pour `operator-review`, deux sites sont
  nommés, parce que deux sites l'emploient.
- **AC2.** Le contrôle négatif est conservé.
  → `!declared("a-label-nobody-has-ever-declared")` est conservé mot pour mot,
  et § 4 point 4 prouve qu'il discrimine encore. Deux contrôles négatifs
  figurent au lieu d'un : le second (`!declared("block")`) est ce qui tient
  l'ancrage ajouté au prédicat.

Dérivés du corps, et explicitement demandés par le « Travail attendu » :

- **AC3.** Pour `OPERATOR_HELD_LABELS`, l'itération se fait sur la constante :
  un label ajouté à la constante est couvert **sans toucher au test**.
  → Vérifiable par lecture de D3 (boucle `for label in
  crate::webhook_dispatch::OPERATOR_HELD_LABELS`) ; aucune des trois valeurs
  actuelles n'apparaît en littéral dans le test.
- **AC4.** Le littéral de `auto_pull.rs:3125` est sorti en constante nommée, que
  le test lit — pas une copie du littéral.
  → D1, et le test référence `ABANDON_LABEL`, jamais `"operator-review"`.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/auto_pull.rs` — D1 (`ABANDON_LABEL` + doc + 4 sites) | 22 |
| `crates/mika-agent/src/auto_pull.rs` — D2 (réconciliation des 2 doc comments) | 16 |
| `crates/mika-agent/src/auto_pull.rs` — D3 (test étendu : doc + corps) | 70 |
| `crates/mika-agent/src/webhook_dispatch.rs` — D4 (renvoi vers la garde) | 6 |
| `docs/solutions/best-practices/un-label-denforcement-non-declare-echoue-en-silence-2026-09-01.md` — D5 | 0 (hors `docs/`) |

Total estimé : 114 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 114 lignes estimées contre une borne
par vol de 279 et un seuil par PR de 1000 : la plus serrée des deux est la borne
par vol, et le total en est à 41 %. La marge n'est pas un pari sur le débit : le
vol ne touche que **deux fichiers de code**, n'ajoute aucun module, aucune
dépendance, aucune migration de schéma, et ne change aucun comportement de
production (D1 est un remplacement de même valeur, § 3). Le seul poste
substantiel est D3, dont le corps est déjà écrit au § D3 de ce plan — la
transcription, non la conception, est ce qui reste. Le contrat de vérification
(§ 4) coûte huit exécutions dont quatre sont des mutations de trois lignes dans
`labels.yml` restaurées par `git checkout --`, soit la part en tours la plus
prévisible du vol.
