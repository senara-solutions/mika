# fix(3123) — la garde `completion_claim` lit un nom d'état cité comme une déclaration de complétion

**Vol 1 de 2 — axe A : le nom cité.** Type : fix. Branche : `fix/3123/agent-loop-la-garde-completion-claim`.
Ticket : senara-solutions/mika#3123.

Le ticket nomme deux défauts indépendants, qui ne partagent ni leur site ni leur
correctif :

- **Axe A** — `COMPLETION_CLAIM_RE` tire sur `` `completed` `` cité en code inline,
  parce que le backtick n'est pas un caractère de mot pour `\b`.
- **Axe B** — le texte que `RejectEndTurn` écarte ne survit pas : il n'atteint ni
  l'utilisateur, ni `messages`, et n'atteint `llm_calls.response_text` que si
  `MIKA_STORE_LLM_CALLS` est armé.

Les deux ensemble pèsent ~350 lignes estimées, au-dessus de la borne par vol de 279.
Ce vol livre l'axe A seul, mergeable seul. L'axe B est nommé au § *Périmètre*.

## Le défaut, tel qu'il se mesure

`crates/mika-agent/src/agent_loop/mod.rs:11512-11538`, lu à HEAD (`35239e0d`) :

```rust
static COMPLETION_CLAIM_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(merged|deployed|completed?|shipped)\b")
        .expect("completion claim regex must compile")
});

fn detect_completion_claim(text: &str) -> Option<&str> {
    let lower = text.to_lowercase();
    if !lower.contains("merge") && !lower.contains("deploy")
        && !lower.contains("complete") && !lower.contains("ship") { return None; }
    COMPLETION_CLAIM_RE.find(text).map(|m| m.as_str())
}
```

La regex est appliquée au **texte brut**. En syntaxe regex, `\b` est une frontière
entre un caractère de mot (`[0-9A-Za-z_]`) et tout le reste : le backtick est « tout
le reste ». Donc `` `completed` `` contient deux frontières et matche exactement comme
`completed` nu. Il n'y a aucun traitement du Markdown en amont : ni fence, ni span.

| ce que le tour écrit | la regex | ce que la garde devrait faire |
|---|---|---|
| `J'ai mergé la PR #3069.` → `merged the PR` | matche | rejeter — déclaration nue |
| `` Le statut passe à `completed`. `` | matche | **passer** — nom d'état cité |
| ``` ```sql\nWHERE status = 'completed'\n``` ``` | matche | **passer** — bloc clôturé |
| `` La colonne `completed_at` reste NULL. `` | ne matche pas (`\b` échoue sur `_`) | passer |
| `La tâche est finie.` | ne matche pas (exclusion assumée) | passer |

Ce que la garde fait du match (`mod.rs:11545-11608`, `evaluate_completion_claim`) :
elle passe si l'outil `update_task_status` est absent du registre ou a déjà été
appelé dans le tour, puis si aucune tâche `pending`/`in_progress` n'existe. Sinon elle
émet un WARN `"Completion claim detected without update_task_status call — re-prompting"`
et rend `RejectEndTurn`. Aucune de ces trois portes ne protège le cas cité :

- `update_task_status` est dans `default_tools()`, donc présent pour tout agent
  non délégué ;
- une seule tâche active suffit — et un pilote qui travaille a toujours la sienne ;
- le tour qui cite un nom d'état est précisément celui qui n'a pas à écrire un statut.

**L'ampleur de la forme, mesurée.** Une citation en backticks de l'un des quatre mots
apparaît **543** fois dans `crates/mika-agent/src`, `skills/bundled/` et `docs/plans/`
de ce dépôt. C'est la forme normale de la prose technique du projet : un tour de revue,
de grooming ou d'oracle qui décrit une machine à états la produit sans y penser.
Et `completed` est bien un statut de ce dépôt (`db.rs:3885`, `db.rs:4401`,
`db.rs:4773`) — le nom à citer n'est pas hypothétique.

**Ce que ce plan ne corrobore pas.** Le ticket tient de « bearing de Prime sur #3069
perdu trois fois » (commentaire 41 du parking #2983). Ce commentaire n'a pas pu être
lu dans cette session : `gh issue view 2983 --comments` dépasse le délai de 120 s sur
ce parking, et la lecture de repli par `gh api .../issues/2983/comments` est refusée
par la permission-policy. Le fait est donc **pris comme donné du ticket**, non
re-mesuré. Il porte sur l'axe B ; l'axe A, lui, est établi au code ci-dessus et ne
dépend pas de lui.

**Ce que l'axe B est exactement** (lu, et plus précis que la formule du ticket). Sur
`RejectEndTurn` (`mod.rs:2486-2537`), le dispatch insère le label dans
`post_condition_retries`, pousse les blocs de la réponse **et** la correction dans
`request.messages` — en mémoire — puis `break` et `continue`. Le texte n'est donc pas
« perdu » au sens strict : il reste dans l'historique de requête du tour suivant, ce
qui est précisément ce qui permet au modèle de le réémettre. Ce qui manque est
ailleurs, et c'est mesurable : (1) il n'est jamais rendu à l'utilisateur ; (2) il n'est
jamais écrit dans `messages` ; (3) il n'atteint `llm_calls.response_text` que si
`store_llm_calls` est vrai (`mod.rs:1196-1322`), donc par défaut jamais ; (4) aucune
colonne ne marque une telle ligne comme retenue. Sur une réémission dégradée, la
substance du premier jet n'est recouvrable d'aucune trace. C'est un défaut de
persistance, et il se corrige à un autre site que l'axe A : d'où le découpage.

## Requirements

**R1.** Un mot-clé de complétion qui n'apparaît **que** dans une région de code
Markdown (span inline ou bloc clôturé) ne doit pas faire rendre `RejectEndTurn`.

**R2.** Un mot-clé qui apparaît au moins une fois **hors** région de code doit
continuer à faire rendre `RejectEndTurn`, quel qu'en soit le nombre d'occurrences
citées par ailleurs. Le rétrécissement ne désarme pas la garde.

**R3.** Le masquage ne doit jamais **créer** un match absent du texte brut : masquer
une région ne peut que retirer des occurrences, jamais en souder deux fragments
(`` com`x`pleted `` ne doit pas devenir `completed`).

**R4.** Aucun indexage par offset d'octet sur le texte. `scripts/check-byte-slices.sh`
reste vert sans nouvelle entrée d'allowlist, et un texte avec accents, em-dash ou emoji
dans ou autour d'une région de code ne panique pas.

**R5.** Le coût du masquage n'est payé que lorsqu'un mot-clé est effectivement
présent : le chemin rapide sur le texte brut reste en premier, et la branche
dominante (aucun mot-clé) ne paie aucune allocation nouvelle.

**R6.** Le rétrécissement est observable : une sonde INFO est émise **uniquement
lorsque le masquage a changé l'issue du tour** — c'est-à-dire lorsque la garde aurait
rejeté sans lui et ne rejette pas avec. Elle ne porte jamais le texte du tour.

**R7.** La grammaire des régions de code est écrite, bornée, et ses trois décisions
de bord sont justifiées en commentaire au site : fence non terminée, span non terminé,
bloc indenté de quatre espaces.

**R8.** La régression est tenue à deux niveaux : une table de vérité unitaire sur la
fonction de masquage, et un scénario eval de bout en bout qui atteste que le tour
citant un nom d'état est **accepté** (`EndTurn` honoré, pas de re-prompt).

## Design

### `code_masked` — remplacer les régions de code par des espaces

Site : `crates/mika-agent/src/agent_loop/mod.rs`, juste avant `detect_completion_claim`.

```rust
/// Rend `text` avec chaque caractère d'une région de code Markdown remplacé par
/// un espace, et le nombre de régions masquées.
fn code_masked(text: &str) -> (String, usize)
```

Pourquoi **remplacer par des espaces** plutôt que supprimer : supprimer souderait
les deux bords de la région, et `` com`x`pleted `` deviendrait `completed` — un
faux positif *créé* par le correctif (R3). L'espace préserve la longueur en
caractères, la structure des lignes, et surtout les frontières de mot : la regex
voit `com pleted`, qui ne matche pas. L'implémentation itère sur `char_indices()`
et pousse soit le caractère, soit `' '`, dans un `String` neuf — aucun slicing par
octet, donc R4 est tenu par construction et non par annotation.

Grammaire, et ses trois décisions de bord :

| forme | ouverture | fermeture | décision de bord |
|---|---|---|---|
| bloc clôturé | ≥ 3 backticks en début de ligne (espaces admis) | ligne de ≥ autant de backticks | **non terminé ⇒ masque jusqu'à la fin du texte** |
| span inline | N backticks (N < 3, ou N quelconque hors début de ligne) | exactement N backticks, **même ligne** | **non terminé ⇒ ne masque rien** |
| bloc indenté 4 espaces | — | — | **hors périmètre** |

- **Fence non terminée ⇒ masque jusqu'à la fin.** C'est la sémantique CommonMark, et
  c'est aussi le cas fréquent qui compte : une réponse coupée au plafond de tokens
  en plein bloc de code. La traiter comme du texte nu rejetterait un tour qui n'a
  rien déclaré. Le coût est nommé : un tour pourrait ouvrir une fence qu'il ne ferme
  pas et déclarer ensuite ce qu'il veut. Il reste détectable — la fence ouverte est
  visible dans le texte — et le contournement était déjà ouvert sans cela par les
  trois mots que la garde exclut volontairement (`done`, `built`, `finished`).
- **Span non terminé ⇒ ne masque rien.** CommonMark traite un backtick non apparié
  comme littéral, et c'est le choix anti-contournement : un backtick solitaire ne
  doit pas neutraliser la fin d'une ligne.
- **Bloc indenté de quatre espaces : hors périmètre**, nommé plutôt que tu. Le projet
  n'en produit pas (les fences sont la forme universelle de ses prompts et de ses
  rapports) et le reconnaître demanderait de distinguer l'indentation d'une
  continuation de liste de celle d'un bloc — un coût sans population mesurée.

Précédent dans le dépôt pour le découpage naïf par backticks :
`crates/mika-agent/src/skills/variants.rs:461-463` (« Odd indices are inside
backticks »). Ce plan ne le réutilise pas — il ne connaît ni les fences ni la borne
de ligne — mais il situe le besoin.

### `scan_completion_claim` — un seul lecteur de la regex, deux informations

`detect_completion_claim` rend aujourd'hui `Option<&str>` emprunté au texte. Comme
le texte masqué est une `String` locale, il devient `Option<String>` — ce qui ne
coûte rien au seul appelant, qui faisait déjà `.to_string()`.

Les deux informations dont `evaluate_completion_claim` a besoin (le mot trouvé hors
code, et — quand le masquage a changé l'issue — le mot retenu) sortent d'un seul
passage, pour ne masquer qu'une fois :

```rust
struct ClaimScan {
    /// Le mot-clé trouvé hors région de code : la déclaration.
    hit: Option<String>,
    /// Le mot-clé que le masquage a retenu, et le nombre de régions masquées.
    /// `Some` seulement quand `hit` est `None` : le masquage a changé l'issue.
    withheld: Option<(String, usize)>,
}

fn scan_completion_claim(text: &str) -> ClaimScan
```

Ordre interne, et il porte R5 :

1. chemin rapide inchangé sur `text.to_lowercase()` — aucun mot-clé plausible ⇒
   `ClaimScan::default()`, sans allocation nouvelle ;
2. `COMPLETION_CLAIM_RE.find(text)` sur le **brut**. S'il ne matche pas, le masqué
   ne peut pas matcher non plus (R3 : masquer ne retire que des occurrences) — donc
   on sort. C'est ce qui évite de masquer pour `relationship`, que le chemin rapide
   laisse passer sur `contains("ship")` ;
3. `code_masked(text)` puis `find` sur le masqué. Match ⇒ `hit`. Pas de match ⇒
   `withheld`, avec le mot du brut et le compte de régions.

`detect_completion_claim(text) -> Option<String>` devient
`scan_completion_claim(text).hit` et reste le prédicat que les tests existants
interrogent.

### `evaluate_completion_claim` — où la sonde doit tomber

La sonde de R6 doit mesurer les rejets **évités**, pas les citations rencontrées.
Elle doit donc tomber après les deux autres portes : un tour qui cite un nom d'état
alors qu'aucune tâche n'est active n'aurait pas été rejeté de toute façon, et le
compter gonflerait la mesure.

Mais l'ordre actuel paie `list_active_tasks()` — une lecture DB — seulement quand un
mot-clé est détecté. Déplacer les portes en tête ferait payer cette lecture à chaque
EndTurn : régression refusée. La forme retenue garde l'ordre et élargit juste la
condition d'entrée :

```rust
let scan = scan_completion_claim(text);
if scan.hit.is_none() && scan.withheld.is_none() {
    return GuardDecision::Pass;                 // le cas dominant, inchangé
}
// ... les deux portes existantes, inchangées (outil, tâches actives) ...
match (scan.hit, scan.withheld) {
    (Some(keyword), _) => { /* WARN + RejectEndTurn, inchangé */ }
    (None, Some((keyword, regions))) => {
        info!(target: "mika::otel",
              event = "guard.completion_claim_code_quoted",
              keyword = %keyword, masked_regions = regions,
              label = %mode_label, step,
              "Completion keyword appears only inside code regions — EndTurn allowed");
        GuardDecision::Pass
    }
    (None, None) => unreachable!("filtered above"),
}
```

La lecture DB est donc aussi payée dans le cas « citation seule » — rare, borné, et
c'est le prix de la mesure.

### Lire la sonde

Doctrine suivie : la sonde n'existe que lorsque le rétrécissement a changé l'issue du
tour (`guard.resume_scope_stood_down`, mika#3002), et elle se lit **en paire** avec le
WARN de re-prompt déjà émis :

- numérateur — `guard.completion_claim_code_quoted` : les tours sauvés par le
  rétrécissement ;
- dénominateur du reste — le WARN `Completion claim detected without
  update_task_status call` : les tours que la garde rejette encore.

Deux règles de lecture, toutes deux déjà établies : l'**absence** de la ligne de sonde
ne vaut pas zéro mais « binaire d'avant le vol » (mika#2340, absent-is-not-zero
mika#2331) ; et les deux populations ne se soustraient pas l'une de l'autre
(mika#2205) — un tour qui cite *et* déclare émet le WARN et pas la sonde, par
construction du `match`.

La sonde ne porte ni le texte, ni la région masquée : `keyword` (quatre valeurs
possibles), `masked_regions`, `label`, `step`. Rien de ce qui sort ne peut porter une
donnée du membre.

### Ce qui n'est pas touché

- La regex elle-même : les mots et l'exclusion volontaire de `done`/`built`/`finished`
  sont la décision de #483, documentée au site, et ce vol ne la relitige pas.
- Les trois portes de passage et le message de correction : inchangés.
- `post_condition.rs` : aucune entrée de registre nouvelle, aucune signature publique
  modifiée.

## Périmètre — ce vol et le suivant

**Ce vol (axe A).** `code_masked`, `scan_completion_claim`, le rebranchement de
`detect_completion_claim` et de `evaluate_completion_claim`, la sonde, et les trois
surfaces de test.

**Le vol suivant (axe B — la survie du texte retenu).** Hors périmètre ici, et son
correctif est à un autre site :

- persistance **inconditionnelle** du texte retenu, via
  `AsyncDatabase::log_audit_event` (`async_db.rs:2676-2709`) — pas de DDL, pas de
  dépendance à `MIKA_STORE_LLM_CALLS` — après `secret_scrubber::scrub_secrets` et
  `mika_common::text::safe_truncate` ;
- la clé de jointure qui permet de relier la ligne retenue au tour qui l'a écartée ;
- un message de correction qui exige la **réémission verbatim de la substance**, le
  défaut nommé par le ticket étant la dégradation du second jet, non le rejet ;
- re-servir le texte retenu quand la réémission est pire que le premier jet.

**Également hors périmètre, et nommé.** Les gardes sœurs portent la même frontière de
mot sur du texte brut : `MILESTONE_CLOSE_CLAIM_RE`, la garde assert-grounded, et
`equivalence_claim`. Elles citeront `code_masked` une fois qu'il existe ; les y
brancher demande leur propre table de vérité et leur propre mesure de population.

Tracked in: senara-solutions/mika#3123

## Verification Contract

1. `cargo test -p mika-agent` — vert. Les ~15 assertions existantes de
   `agent_loop/tests/detection_dans_le_texte.rs:148+` restent vertes après adaptation
   en `.as_deref()` : c'est le contrôle positif que R2 tient.
2. `cargo test -p mika-agent --test eval test_completion_claim_guard` — les sept
   scénarios existants plus le nouveau. Le nouveau atteste que le tour citant
   `` `completed` `` termine par un `EndTurn` honoré, sans re-prompt.
3. La table de vérité du masquage couvre, chacun en assertion nommée : span simple,
   span à N backticks, span non terminé, span avec accents et em-dash autour,
   fence clôturée, fence non terminée, fence imbriquant des backticks simples,
   citation **et** déclaration dans le même texte (⇒ rejet), soudure
   (`` com`x`pleted `` ⇒ aucun match), `completed_at` (⇒ aucun match, déjà vrai).
4. `scripts/check-byte-slices.sh` — vert, **et sans entrée d'allowlist nouvelle**.
   Une entrée ici serait l'aveu que `code_masked` indexe par octet ; elle n'a pas
   lieu d'être.
5. `cargo clippy --all-targets -- -D warnings` — vert.
6. `cargo fmt --check` — vert.
7. `scripts/verify-pipeline.sh origin/main` — vert.
8. `make verify-bundled-skills` — vert (aucun bundle touché ; c'est le contrôle que
   le vol n'a pas dérivé).
9. **Après déploiement, et ce n'est pas une condition de merge** :
   `grep -c 'guard.completion_claim_code_quoted' ~/.mika/logs/*.log` sur une fenêtre
   de sept jours, lu en paire avec le compte du WARN de re-prompt. Un numérateur nul
   avec un dénominateur nul ne dit rien ; un numérateur nul avec un dénominateur non
   nul dit que les rejets restants sont des déclarations véritables.

## Definition of Done

- [ ] `code_masked(text) -> (String, usize)` livrée, avec sa grammaire et ses trois
      décisions de bord documentées au site.
- [ ] `scan_completion_claim` livrée, masquant **une seule fois** et après le test
      sur le texte brut.
- [ ] `detect_completion_claim` rend `Option<String>` et délègue à
      `scan_completion_claim`.
- [ ] `evaluate_completion_claim` entre sur `hit || withheld`, conserve ses deux
      portes dans l'ordre, et émet la sonde sur le seul bras `withheld`.
- [ ] Sonde `guard.completion_claim_code_quoted` (INFO, `target: "mika::otel"`) avec
      `keyword`, `masked_regions`, `label`, `step` — et rien du texte.
- [ ] Table de vérité du masquage : les dix cas du point 3 du Verification Contract.
- [ ] Assertions existantes adaptées en `.as_deref()`, toutes vertes.
- [ ] Scénario eval `guard_skips_on_code_quoted_state_name` ajouté à
      `crates/mika-agent/tests/eval/test_completion_claim_guard.rs`.
- [ ] Les neuf points du Verification Contract exécutés (le 9 après déploiement).
- [ ] Le § *Périmètre* du plan est reporté dans le corps de la PR, ligne
      `Tracked in:` incluse.

## Acceptance criteria

Le corps de senara-solutions/mika#3123 ne porte pas de section
`## Acceptance criteria` — il porte deux lignes de source. Les critères ci-dessous
sont donc **dérivés** des Requirements R1–R8 et du Verification Contract, un critère
par propriété observable.

- **AC1.** Un texte dont le seul mot-clé est dans un span inline
  (`` Le statut passe à `completed`. ``) fait rendre `GuardDecision::Pass`, avec une
  tâche active et `update_task_status` enregistré non appelé. (R1)
- **AC2.** Même propriété pour un bloc clôturé, y compris pour un bloc dont la fence
  d'ouverture n'est jamais fermée. (R1, R7)
- **AC3.** Un texte portant le mot-clé hors région de code fait rendre
  `RejectEndTurn`, y compris lorsqu'il porte aussi des occurrences citées. (R2)
- **AC4.** `` com`x`pleted `` ne produit aucun match : le masquage ne soude pas les
  bords d'une région. (R3)
- **AC5.** `scripts/check-byte-slices.sh` est vert sans entrée d'allowlist nouvelle,
  et un texte avec accents et em-dash dans et autour d'une région de code ne panique
  pas. (R4)
- **AC6.** Un texte sans aucun des quatre mots-clés sort par le chemin rapide sans
  appeler `code_masked` — assertion de structure : `scan_completion_claim` rend
  `hit: None, withheld: None` et, pour un texte portant `relationship` seul, ne
  masque pas (le test observe que le compte de régions rendu est zéro alors que le
  texte contient des backticks). (R5)
- **AC7.** La sonde `guard.completion_claim_code_quoted` est émise sur le cas AC1 et
  **n'est pas** émise sur le cas AC3, ni sur un tour sans mot-clé, ni sur un tour que
  les deux portes existantes laissent passer. (R6)
- **AC8.** Un scénario eval de bout en bout atteste que le tour d'AC1 termine par un
  `EndTurn` honoré : la réponse atteint l'appelant, aucun re-prompt n'est émis, et
  `post_condition_retries` reste vide. (R8)

## Fire-Disposition

Ce vol livre des détecteurs — la table de vérité du masquage, les assertions
existantes conservées comme contrôle positif, le scénario eval — et la garde EndTurn
elle-même. **Option (a) : exception nommée en allowlist, avec une population
d'exceptions vide par construction.**

Pourquoi vide, et pourquoi par construction : ce vol **rétrécit** un détecteur
existant. La population violante d'un détecteur rétréci est un sous-ensemble de celle
du détecteur d'avant — tout ce qui passait continue de passer (R2, tenu par le
contrôle positif du Verification Contract point 1), et la seule chose qui change
d'issue est précisément le faux positif que le vol corrige. Il n'existe donc aucune
donnée préexistante à exempter : pas d'entrée d'allowlist à écrire, et aucune ne doit
être inventée pour remplir la section.

Le pendant de cette absence est que le rougissement doit rester possible, sinon la
section ne dit rien. Deux assertions le garantissent, et ce sont elles qui tiennent
le rôle de l'« assertion auto-nettoyante » :

- **Le contrôle positif** — les ~15 assertions de `detection_dans_le_texte.rs` sur
  les déclarations nues. Si un futur élargissement du masquage désarmait la garde,
  elles rougissent. Leur rougissement signale un désarmement, jamais une exception
  stale.
- **Le contrôle d'allowlist** — `scripts/check-byte-slices.sh` doit rester vert
  **sans entrée nouvelle** (Verification Contract point 4, AC5). Une entrée ajoutée
  ici serait exactement l'exception stale que mika#2309 condamne : elle dirait que le
  correctif indexe par octet. La contrainte est donc écrite comme une condition de
  merge, pas comme une préférence.

Aucun détecteur n'atterrit désarmé : il n'y a ni `#[ignore]` ni `#[cfg(skip)]` dans ce
vol, et rien ne remonte à l'opérateur.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/mod.rs` — `code_masked` + grammaire et décisions de bord documentées | 60 |
| `crates/mika-agent/src/agent_loop/mod.rs` — `ClaimScan` + `scan_completion_claim` + `detect_completion_claim` rebranché | 25 |
| `crates/mika-agent/src/agent_loop/mod.rs` — `evaluate_completion_claim` : condition d'entrée, `match` à trois bras, sonde | 20 |
| tests (`crates/mika-agent/src/agent_loop/tests/detection_dans_le_texte.rs`) — table de vérité du masquage, dix cas | 60 |
| tests (`crates/mika-agent/src/agent_loop/tests/detection_dans_le_texte.rs`) — adaptation des assertions existantes en `.as_deref()` | 20 |
| tests (`crates/mika-agent/tests/eval/test_completion_claim_guard.rs`) — `guard_skips_on_code_quoted_state_name` | 45 |

Total estimé : 230 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 230 estimées contre 279 : 49 lignes de marge, 18 %.
Les deux axes du ticket ensemble pèsent ~350 estimées — au-dessus de la borne — d'où
le découpage en deux vols, et non en deux phases d'un même vol : l'axe A est mergeable
seul et livre une valeur complète (la garde cesse de tirer sur une citation) sans rien
devoir à l'axe B. Les trois livrables de code touchent **un seul fichier** et une
seule fonction publique ; les trois livrables de test sont des ajouts à des surfaces
qui existent déjà, avec leur harnais et leurs helpers (`EvalHarness`,
`StubUpdateTaskStatusTool`, `seed_task`, `tools_with_update_task_status`). Aucune
migration, aucun DDL, aucun bundle, aucune surface HTTP, aucun composant UI : la fin
de pipeline est celle d'un fix mono-crate.

## Ordre de commit

Le plancher mergeable est au commit 3 : à ce point la garde cesse de tirer sur une
citation et la régression est tenue par la table de vérité. Les commits 4 et 5 ajoutent
la mesure et l'attestation de bout en bout.

1. `code_masked` + sa grammaire documentée, et la table de vérité du masquage.
   Vert seul : la fonction n'a encore aucun appelant.
2. `ClaimScan` + `scan_completion_claim`, `detect_completion_claim` rendant
   `Option<String>`, assertions existantes adaptées en `.as_deref()`.
3. `evaluate_completion_claim` rebranché sur `ClaimScan` — **le plancher** : R1 et R2
   sont tenus, la garde est corrigée.
4. La sonde `guard.completion_claim_code_quoted` et son assertion d'émission /
   non-émission.
5. Le scénario eval de bout en bout.
