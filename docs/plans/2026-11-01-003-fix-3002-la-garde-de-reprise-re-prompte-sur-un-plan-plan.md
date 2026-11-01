# mika#3002 — la garde de reprise re-prompte mika-arch sur un brief qui porte un plan

**Ticket :** senara-solutions/mika#3002
**Labels :** `bug`, `p2-normal`, `ready`, `dispatch:loop`, `loop-substrate`
**Type :** fix
**Vols :** un seul (voir § 8)
**Dépend de :** mika#2997 (mergé — `8c5477a6d`, déployé)

---

## 1. Où en est le ticket, et pourquoi ce vol-là

### 1.1 Le défaut, reproduit dans l'arbre

La garde `resume_reconcile` est une entrée du registre `INTENT_GUARDS`
(`crates/mika-agent/src/agent_loop/mod.rs:11976-11986`, #702). Son déclencheur
est `detect_resume_intent` (`mod.rs:12203-12212`), une **conjonction de deux
expressions régulières** :

| regex | site | ce qu'elle attrape |
|---|---|---|
| `(?i)\b(resume\|continue)\b` | `mod.rs:12188` | `RESUME_INTENT_VERB_RE` |
| `(?i)\b(milestone\|project)\s*#\d+` | `mod.rs:12192` | `RESUME_INTENT_PROCESS_RE` |

Les deux conjoints sont satisfaits par un plan de groom français **sans qu'aucune
intention de reprise ne soit exprimée**, et le plan de l'incident mesuré le
démontre ligne par ligne :

| fichier | ligne | texte | conjoint satisfait |
|---|---|---|---|
| `docs/plans/2026-10-31-006-fix-2995-consent-mika-dev-peut-se-poser-lui-m-me-plan.md` | 592 | « le prompt servi **continue** de prescrire » | verbe — le « continue » **français** collide avec l'anglais |
| idem | 557 | `milestone #33` | référence de processus — le `\s*#` tolère l'espace |

Les deux lignes vivent dans le même fichier, donc `detect_resume_intent` rend
`true` sur un brief qui est un plan, et la garde s'arme. Son `satisfied`
(`resume_reconcile_satisfied`, `mod.rs:12219`) exige `check_task` ou
`list_tasks` — deux outils que mika-arch, agent de revue en lecture seule, n'a
aucune raison d'appeler. Le re-prompt est donc **inévitable** sur cette
population, et son message correctif (`mod.rs:11980-11985`) ordonne à
l'architecte d'aller « trouver la tâche parente, localiser l'enfant suivant et
reprendre l'exécution » au lieu de rendre sa revue.

### 1.2 Ce que cela a coûté, deux fois mesuré

**En production, le 2026-10-31.** Première passe du groom de mika#2995, trace
`3a066c59`, session `460a81ba`. L'architecte ne rend pas de revue : il répond en
264 caractères qu'il a « cherché des engagements (tâches) actifs pour réconcilier
l'état comme demandé par l'instruction de reprise ». Sans ligne `Disposition:`,
le tour donne `UNPARSED`, puis la relance de mika#1823 — un appel LLM entier
dépensé à obéir à une instruction que personne n'a donnée.

**En éval.** Le constat est déjà écrit dans l'arbre, par le spawn de mika#2996 :
`crates/mika-agent/tests/eval/grounding_regressions/review_anchor_unparsed_retry_2996.rs:186-193`
documente `REPLIES = 4` en ces termes — « sur un brief qui porte ce plan, la garde
de sémantique de reprise re-prompte une fois avant la garde d'ancrage. Elle tire
aussi, en production, sur le tour d'origine ». Ce commentaire est la mesure, et il
est le point d'ancrage de ce vol.

**Et depuis mika#2997 la surface a doublé.** `_append_retry_plan_envelope`
(`skills/bundled/_shared/dispatch-lib.sh:11098-11107`) recopie le plan dans les
**deux** prompts de relance, donc le brief de chaque tour de relance porte
désormais le plan, donc la garde peut tirer à chaque tour de relance et non plus
seulement à la première passe.

### 1.3 Pourquoi l'enveloppe mika#2996 ne suffit pas, et ce qui décide

AC1 énumère deux voies. La première — l'enveloppe `BEGIN PLAN UNDER REVIEW` —
**ne peut pas fermer l'incident mesuré**, et `review_anchor.rs` le dit déjà en
propres termes dans la doc de `anchor_reference` : « une première passe, dont le
message *est* le plan, est rendue entière ». Les marqueurs ne sont écrits que par
les deux prompts de relance ; le brief de première passe n'en porte aucun. Or
l'incident `3a066c59` est **une première passe**.

C'est donc la seconde voie qui décide : **le contrat du skill**. Et le
discriminateur est déjà dans l'arbre, déjà résolu par tour, déjà passé à
`run_loop` :

```rust
// mod.rs:63-70 — le point de vérité unique
const VERDICT_PRODUCER_SKILLS: &[&str] = &["mika-arch-groom-ticket", "mika-arch-second-review"];
```

ce sont **exactement** les deux skills qu'AC1 nomme.
`has_verdict_producer_skill` les résout en `mod.rs:6838`, et
`is_verdict_producer: bool` est déjà un paramètre de `run_loop` (`mod.rs:1460-1560`),
lu par deux autres gardes. Le correctif n'a donc **aucun chemin de résolution à
créer** : il a seulement besoin qu'une entrée du registre puisse consulter un fait
de tour.

L'enveloppe n'est pas écartée pour autant : elle **affine**. Le contrat du skill
dit « ce tour est une revue de plan » ; l'enveloppe dit, quand elle est là, « voici
où s'arrête le plan et où commence ce qui m'est adressé ». Les deux ensemble
donnent la règle du § 2.1, et c'est elle qui fait tenir le contrôle négatif d'AC2.

### 1.4 La doctrine que ce vol applique, et celle qu'il n'ouvre pas

Deux précédents cadrent le geste, et il faut les nommer parce qu'ils décident la
**forme** autant que le fond.

**mika#2517 U3 — le stand-down.** `webhook_zero_tools_trigger`
(`mod.rs:12111-12131`) se désarme sur tout le domaine Webhook Fallthrough, et son
commentaire dit pourquoi ce n'est pas un relâchement : « sur cette population la
prémisse de la garde — *un événement webhook exige une action* — est FAUSSE par
contrat ». Mot pour mot le cas présent : sur un tour de revue, la prémisse « ce
message est une instruction adressée à l'agent » est fausse par contrat, puisque
le message **est la donnée à juger**. Le précédent fixe aussi la propriété
structurelle : le stand-down est une propriété de la **garde**, jamais du routage
en aval (cf. `crates/mika-agent/CLAUDE.md` § MessageSender, mika#1381 vol 4).

**mika#2755 / mika#2768 — citation ≠ intention.** `mask_cited_branch` masque un
slug de branche *cité* avant l'évaluation des mots-clés, en tête de
`skills::matcher::match_skills`. Sa portée documentée **exclut explicitement** les
`INTENT_GUARDS` (« les `INTENT_GUARDS` et le prompt lisent tous le message
intact »). Ce vol n'étend pas ce masque : il applique la même **doctrine** au
registre, sous une déclaration locale et par-entrée, sans toucher au masque ni à
sa portée.

**Ce que ce vol n'ouvre pas.** La collision du verbe français « continue » avec
l'anglais, et celle d'une référence `milestone #N` *citée*, sont une classe plus
large que les deux skills d'AC1 — elles frappent n'importe quel message français
qui discute d'un jalon. Une réforme des deux regex **n'est pas dans le périmètre**
de ce ticket (AC1 et AC2 nomment le tour de revue) et serait un changement de
comportement sur toute la flotte, mesuré par rien. Voir § 2.4 : elle part en
suivi.

---

## 2. Le dessin

### 2.1 La règle sémantique, écrite une fois

> Sur un tour de revue d'architecte (`is_verdict_producer == true`), le
> déclencheur de `resume_reconcile` est évalué sur la **région du brief située
> hors du plan revu** — jamais sur le plan. Hors d'un tour de revue, il est
> évalué sur le message entier, octet pour octet comme aujourd'hui.

Trois cas, et ils couvrent la totalité des briefs que `_iterate_groom_loop`
produit (table des trois usages : `crates/mika-agent/CLAUDE.md` § *Session
contract of the architect passes*) :

| brief | marqueurs mika#2996 | région hors plan | la garde ? |
|---|---|---|---|
| première passe (le brief **est** le plan) | aucun | vide | **stand-down** — AC1, et c'est l'incident `3a066c59` |
| relance UNPARSED / seconde passe | les deux | le texte correctif, avant `BEGIN` (et après `END`) | s'arme **si et seulement si** cette région porte les deux conjoints |
| enveloppe mutilée (un seul marqueur) | un seul | le brief entier | s'arme — *fail-closed*, § 2.2 |

Et hors tour de revue : aucune des trois lignes ci-dessus ne s'applique, le
déclencheur reçoit `user_input_text` tel quel.

C'est cette règle qui rend AC2 satisfiable dans ses deux sens à la fois : un plan
qui dit « reprendre le vol suivant » vit *dans* le plan, donc hors de la région
lue ; une vraie instruction de reprise adressée à l'agent vit *hors* du plan,
donc dedans.

### 2.2 Livrable 1 — `review_anchor::outside_reviewed_plan`

Nouvelle fonction dans `crates/mika-agent/src/agent_loop/review_anchor.rs`,
**frère** de `anchor_reference` et posée juste à côté d'elle, parce que les deux
découpent le même brief sur les mêmes deux constantes et qu'une seconde lecture
des marqueurs ailleurs prendrait du retard sans que rien ne rougisse (classe
`grooming_marker`, mika#2158) :

```rust
/// La région du brief qui n'est PAS le plan revu : ce qui s'adresse à l'agent.
///
/// Exact complément de [`anchor_reference`], sur les mêmes deux constantes.
/// Rend DEUX tranches — la tête (avant `RETRY_PLAN_BEGIN`) et la queue (après
/// `RETRY_PLAN_END`) — parce que les deux sont adressables à l'agent et qu'un
/// `&str` unique ne peut pas porter deux régions disjointes. Aujourd'hui
/// `_append_retry_plan_envelope` n'écrit rien après `END`, donc la queue est
/// vide ; la rendre quand même est ce qui fait que le jour où la relance
/// ajoutera un post-scriptum, il sera lu au lieu d'être muet.
///
/// - deux marqueurs → `(tête, queue)`
/// - **zéro** marqueur → `("", "")`. Sur un tour de revue, un brief sans
///   enveloppe EST le plan (`anchor_reference` le dit : « une première passe,
///   dont le message *est* le plan, est rendue entière ») — donc rien n'y est
///   adressé à l'agent.
/// - **un seul** marqueur → `(brief, "")`, le brief entier. Fail-closed, même
///   posture qu'`anchor_reference` : une enveloppe mutilée est un défaut du
///   shell, et la direction conservatrice est que la garde s'arme — elle ne
///   coûte qu'un re-prompt, et la régression mika#2996 rougit déjà sur ce cas.
///
/// Référence : mika#3002, mika#2996.
pub fn outside_reviewed_plan(brief: &str) -> (&str, &str)
```

Pas de regex : la doctrine du fichier (mika#864 — « regex is a footgun — silent
failure to fire when pattern is malformed ») tient ici aussi, et `split_once` /
`rsplit_once` sur les deux constantes suffisent.

### 2.3 Livrable 2 — `IntentScope`, la portée déclarée par entrée

Le registre ne peut pas, tel quel, consulter un fait de tour : `trigger: fn(&str) -> bool`
« voit le message seul », et la doc de `loaded_skill_names` (`mod.rs:1460-1560`)
énonce déjà cette contrainte, dont le précédent canonique est la garde inline
`qa_build_callback_verdict` (mika#2355, `mod.rs:1476-1479`).

Quatre formes étaient ouvertes. Celle retenue, et pourquoi les trois autres ne le
sont pas :

| forme | coût | verdict |
|---|---|---|
| **portée déclarée par entrée** (`IntentScope`) | un champ, 5 déclarations dont 4 triviales | **retenue** : la condition est posée **à côté de la garde qu'elle désarme**, et le registre reste le seul endroit où un lecteur regarde |
| sortir `resume_reconcile` du registre, en garde inline | ~40 lignes de plomberie de re-prompt dupliquée | refusée : duplique le site d'injection du message correctif, que #702 avait précisément unifié |
| élargir la signature de `trigger` pour les 5 entrées | 5 signatures, 5 sites | refusée : rayon de souffle maximal pour un besoin d'une entrée |
| `if guard.label == "resume_reconcile" && … { continue; }` au site d'évaluation | 3 lignes | refusée : sépare la condition de la déclaration, et un `label` comparé en littéral est exactement ce que le champ `label` n'est pas censé porter |

```rust
/// Sur quelle région du message le déclencheur d'une entrée est évalué.
///
/// `fn(&str) -> bool` voit le message seul (cf. la doc de `loaded_skill_names`),
/// donc une entrée qui doit tenir compte d'un fait de tour le DÉCLARE ici, à
/// côté d'elle, plutôt que de migrer en garde inline.
#[derive(Clone, Copy)]
enum IntentScope {
    /// Le message entier. Portée de toutes les entrées sauf une.
    WholeMessage,
    /// mika#3002 — sur un tour de revue d'architecte, la région HORS du plan
    /// revu ; partout ailleurs, le message entier.
    OutsideReviewedPlan,
}

impl IntentScope {
    fn triggered(self, trigger: fn(&str) -> bool, msg: &str, is_verdict_producer: bool) -> bool {
        match self {
            Self::WholeMessage => trigger(msg),
            Self::OutsideReviewedPlan if is_verdict_producer => {
                let (head, tail) = review_anchor::outside_reviewed_plan(msg);
                trigger(head) || trigger(tail)
            }
            Self::OutsideReviewedPlan => trigger(msg),
        }
    }
}
```

Le champ sur la structure, avec sa doc :

```rust
struct IntentPrecondition {
    label: &'static str,
    trigger: fn(&str) -> bool,
    /// mika#3002 — sur quelle région du message `trigger` est évalué.
    /// `WholeMessage` pour toute entrée dont la prémisse ne dépend pas du
    /// contrat du skill : c'est le défaut de lecture, pas un défaut de langage
    /// (il n'y a pas de `Default` ici, et c'est voulu — une nouvelle entrée
    /// doit nommer sa portée).
    scope: IntentScope,
    satisfied: fn(&[ToolCallSummary]) -> bool,
    correction_message: &'static str,
}
```

Pas de `#[derive(Default)]` et pas de valeur par défaut : une nouvelle entrée
**ne compile pas** sans nommer sa portée. C'est la même forme que le `severity`
sans défaut de mika#1381 — « forgot to classify it » n'est pas un état
atteignable, et aucun relecteur n'a à l'attraper.

Les cinq déclarations : `webhook_ready_label_dispatch`, `webhook_zero_tools`,
`callback_*` et `webhook_no_unauthorized_dispatch` prennent
`scope: IntentScope::WholeMessage` ; `resume_reconcile` prend
`scope: IntentScope::OutsideReviewedPlan` avec un commentaire de trois lignes
citant mika#3002 et la trace `3a066c59`.

Le site d'évaluation (`mod.rs:3674-3703`) change d'**une** ligne :

```rust
-                            if (guard.trigger)(&user_input_text)
+                            if guard.scope.triggered(guard.trigger, &user_input_text, is_verdict_producer)
                                 && !(guard.satisfied)(&all_tool_summaries)
```

`is_verdict_producer` est déjà en portée à ce point : c'est un paramètre de
`run_loop`.

### 2.4 Livrable 3 — AC3 : la sonde, et son contrôle positif

AC3 demande un **compte** sur les 20 revues suivantes. Aujourd'hui il n'est pas
calculable : le `warn!` du site (`mod.rs:3682-3687`) ne porte ni `event =` (seul
champ sur lequel les autres gardes se grepent, cf. `guard.response_cjk_script_uncorrected`
en `mod.rs:3660`) ni rien qui dise si le tour était une revue. Deux lignes, et
elles ne sont pas redondantes :

**1. La ligne de tir, enrichie** — `event = "guard.intent_precondition_fired"`
sur le `warn!` existant, plus un champ `is_verdict_producer`. C'est le numérateur
d'AC3 : le compte cible est le nombre de lignes portant à la fois
`intent_guard = "resume_reconcile"` et `is_verdict_producer = true`. **Cible 0.**

**2. Le contrôle positif** — une ligne INFO `event = "guard.resume_scope_stood_down"`,
émise quand la narrowing a effectivement changé l'issue (le message entier
déclenchait, la région hors plan non), avec `brief_chars`,
`envelope: bool` et `skill`. Sans elle, « la garde ne tire plus » et « aucune
revue n'a eu lieu dans la fenêtre » rendent les mêmes octets — la classe
mika#2205, que ce dépôt a déjà payée. Donc le protocole de lecture d'AC3 est une
**paire**, et il faut l'écrire tel quel sur le ticket :

```
tirs    = lignes  intent_guard="resume_reconcile"  is_verdict_producer=true   → cible 0
sorties = lignes  event="guard.resume_scope_stood_down"                       → doit être ≥ 1
```

`tirs == 0` **et** `sorties == 0` ne vaut pas succès : cela veut dire qu'aucun
tour de revue n'a été observé, et la fenêtre de 20 revues n'est pas commencée.

**Caveat de déploiement (mika#2340).** Ces deux lignes sont du code moteur, donc
leur chemin est `make deploy` seul — pas la chaîne `rebuild → seed → read` des
prompts de `skills/bundled/`. Mais la conséquence de lecture est la même :
**l'absence de la ligne ne signifie pas une absence de tir, elle signifie un
binaire antérieur au vol**. La sonde n'est pas vivante avant `make deploy`, et
AC3 ne peut pas être renseigné depuis cette PR — c'est une mesure post-déploiement,
et le commentaire qui la portera sur mika#3002 est le dernier geste du ticket, pas
de ce vol.

### 2.5 Livrable 4 — AC2 : la paire, dans le même appel

Maison : `crates/mika-agent/tests/eval/test_intent_precondition_guard.rs`, qui
porte déjà `tools_with_reconciliation()` et les huit tests `resume_guard_*`.

Un test, deux moitiés, **même fixture de plan et même harnais** — c'est la paire
qui dit quelque chose, et le fichier mika#2996 a déjà engravé pourquoi (« le refus
seul se lirait aussi bien comme *la garde refuse tout* ; l'acceptation seule comme
*la garde ne s'arme pas* ») :

```rust
/// AC1 + AC2 (mika#3002) — sur un tour de revue, les mots de reprise du PLAN ne
/// déclenchent pas la garde ; une instruction de reprise HORS du plan, si.
#[tokio::test]
async fn mika3002_le_plan_revu_ne_declenche_pas_la_garde_mais_une_instruction_hors_plan_si()
```

- **Positif (stand-down, AC1).** Brief = le plan seul, sans enveloppe — la forme
  exacte de la première passe et de l'incident `3a066c59`. Le plan porte les deux
  conjoints, et dans les mots qu'AC2 nomme : une ligne « il faudra **reprendre**
  le **vol suivant** » et une ligne citant `milestone #33`. Attendu : **zéro**
  re-prompt de `resume_reconcile`, un seul pas.
- **Négatif (la garde vit toujours, AC2).** Même plan, mais encadré par
  `RETRY_PLAN_BEGIN` / `RETRY_PLAN_END`, et précédé d'une **vraie** instruction
  adressée à l'agent : `resume mika milestone#8`. Attendu : **un** re-prompt, puis
  l'appel `list_tasks` qui le satisfait.

Deux points de méthode, chacun parce que sans lui le test ne mesurerait rien :

- **Le tour doit être un tour de revue.** `EvalHarness::builder().skills(…)`
  existe (`tests/eval/harness.rs:358-360`, défaut `SkillRegistry::empty()` en
  ligne 650). Le registre injecté porte **une** entrée `always_on = true` nommée
  par `VERDICT_PRODUCER_SKILLS[0]` — lu depuis la constante, **jamais retapé en
  littéral**, sinon un renommage en production laisserait le test vert sur un nom
  mort. `[output]` est laissé **par défaut** (contrat d'ancrage vide), à la
  différence de `registry_from_manifest` du fichier mika#2996 : ici la garde
  d'ancrage n'est pas le sujet, et son re-prompt consommerait les réponses
  simulées sans rien dire de celle-ci — c'est le même raisonnement que la doc de
  `registry_from_manifest` tient sur `required_tools`.
- **L'assertion porte sur le re-prompt, pas sur un compte de pas.** On compte les
  requêtes LLM dont le dernier message commence par la tête du message correctif
  de `resume_reconcile`, exactement comme `anchor_reprompts` dans le fichier
  mika#2996 — un `assert_exact_steps` resterait vert, ou rougirait, pour les
  raisons d'une autre garde.

Et un contrôle de non-régression du contrat lui-même : une assertion que chacun
des deux noms de `VERDICT_PRODUCER_SKILLS` nomme bien un répertoire existant sous
`skills/bundled/`. C'est la seule chose qui pourrit en silence dans tout ce
dessin.

### 2.6 Ce que ce vol ne fait pas, et qui le porte

- **La réforme des deux regex de reprise** (`\b(resume|continue)\b` qui attrape
  le « continue » français, `(milestone|project)\s*#\d+` qui attrape une référence
  citée) reste entière hors tour de revue. C'est la classe mika#2755, elle frappe
  tout message français parlant d'un jalon, et aucune mesure ne borne aujourd'hui
  sa population. `Tracked in: senara-solutions/mika#3002` (commentaire de suivi à
  ouvrir au tri suivant, avec le compte d'AC3 comme première donnée).
- **Les quatre autres entrées du registre** gardent `WholeMessage`. Aucune n'a de
  prémisse fausse sur un tour de revue — toutes exigent un préfixe `[GitHub]` ou
  `[callback: …]` qu'un brief de plan ne porte pas — et élargir le stand-down à
  l'ensemble du registre serait un changement non mesuré déguisé en cohérence.
- **`mask_cited_branch` et sa portée** ne bougent pas : la doctrine est reprise,
  le masque n'est pas étendu aux `INTENT_GUARDS`.
- **AC3 n'est pas renseigné par cette PR** — § 2.4, caveat de déploiement.

---

## 3. Fichiers touchés

| fichier | geste |
|---|---|
| `crates/mika-agent/src/agent_loop/review_anchor.rs` | + `outside_reviewed_plan` (frère d'`anchor_reference`) + ses tests inline |
| `crates/mika-agent/src/agent_loop/mod.rs` | + `IntentScope` et son `impl`, + champ `scope` sur `IntentPrecondition`, 5 déclarations, 1 ligne au site d'évaluation, 2 sondes, + tests inline |
| `crates/mika-agent/tests/eval/test_intent_precondition_guard.rs` | + le test AC1/AC2 et son helper de registre |
| `crates/mika-agent/tests/eval/grounding_regressions/review_anchor_unparsed_retry_2996.rs` | commentaire de `REPLIES` seul : il affirme que la garde de reprise re-prompte une fois sur ce brief, ce qui devient faux. Le nombre reste à 4 (sur-provisionné, inoffensif) ; c'est l'affirmation qui est corrigée, en citant mika#3002 |
| `crates/mika-agent/CLAUDE.md` | 1 paragraphe sous la table des trois usages des passes architecte (hors compte de lignes) |

---

## 4. Ordre de commit

Chaque commit laisse l'arbre compilable et la suite verte.

1. **`outside_reviewed_plan` + tests inline.** Fonction pure, zéro appelant.
   Plancher : `cargo test -p mika-agent review_anchor` vert, y compris les trois
   cas de marqueurs (deux / zéro / un seul).
2. **`IntentScope` + champ `scope` + les 5 déclarations + la ligne du site.**
   À ce commit le comportement change. Plancher : `cargo test -p mika-agent` vert
   en entier — en particulier les huit `resume_guard_*` existants, qui tournent
   tous sur `SkillRegistry::empty()` donc `is_verdict_producer == false`, donc
   doivent rester **octet pour octet** verts. S'ils rougissent, le stand-down a
   fui hors de sa population.
3. **Le test AC1/AC2.** Plancher : rouge si on retire la ligne du commit 2
   (vérifié à la main avant de pousser — un détecteur qu'on n'a pas vu rougir ne
   détecte rien).
4. **Les deux sondes + le commentaire `REPLIES` + `CLAUDE.md`.** Plancher :
   `cargo clippy` et `cargo fmt --check` verts, `scripts/verify-pipeline.sh origin/main` vert.

---

## 5. Definition of Done

- [ ] `review_anchor::outside_reviewed_plan` existe, rend `(tête, queue)`, et ses
      trois cas de marqueurs (deux / zéro / un seul) sont couverts par des tests
      inline.
- [ ] `IntentPrecondition` porte un champ `scope` **sans valeur par défaut** ;
      les cinq entrées le déclarent ; `resume_reconcile` est la seule à
      `OutsideReviewedPlan`.
- [ ] Le site d'évaluation des `INTENT_GUARDS` passe par `guard.scope.triggered(…)`
      et n'évalue plus `(guard.trigger)` directement.
- [ ] Hors tour de revue, le comportement est inchangé : les huit tests
      `resume_guard_*` existants passent sans modification.
- [ ] Le test AC1/AC2 existe dans un seul `#[tokio::test]`, nomme le skill depuis
      `VERDICT_PRODUCER_SKILLS`, et assertionne sur le compte de re-prompts de
      `resume_reconcile` (pas sur un compte de pas).
- [ ] Il a été vu **rouge** avec la ligne du commit 2 retirée.
- [ ] Les deux sondes sont émises : `guard.intent_precondition_fired` (avec
      `is_verdict_producer`) et `guard.resume_scope_stood_down`.
- [ ] Le commentaire de `REPLIES` dans `review_anchor_unparsed_retry_2996.rs` ne
      laisse plus une affirmation devenue fausse.
- [ ] `crates/mika-agent/CLAUDE.md` dit le stand-down et le protocole de lecture
      en paire d'AC3.
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`,
      `make verify-bundled-skills` et `scripts/verify-pipeline.sh origin/main` verts.
- [ ] Corps de PR : `Code review: delegated to mika-qa (mika#2694)`, et la ligne
      `Tracked in:` du § 2.6.

---

## 6. Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#3002.

- **AC1.** Sur un tour de revue d'architecte (skills `mika-arch-groom-ticket` et
  `mika-arch-second-review`), la garde ne s'arme pas sur des mots de reprise
  contenus dans le **plan revu**. Le plan dit comment elle distingue le plan revu
  d'une instruction adressée à l'agent, par exemple par l'enveloppe
  `BEGIN PLAN UNDER REVIEW` de mika#2996 ou par le contrat du skill.
- **AC2.** Test dans le même appel, avec contrôle négatif : un brief de revue
  dont le plan contient « reprendre le vol suivant » ne déclenche pas la garde.
  Une vraie instruction de reprise adressée à l'agent, hors du plan, la déclenche
  toujours.
- **AC3.** Sonde après déploiement : sur les 20 revues d'architecte suivantes, le
  compte des relances de cette garde est écrit ici. La cible est 0.

Où chacun est traité : AC1 → § 1.3 (le contrat du skill décide, l'enveloppe
affine) et § 2.1-2.3. AC2 → § 2.5. AC3 → § 2.4, avec son caveat de déploiement.

---

## 7. Fire-Disposition

Ce vol livre des détecteurs : le test AC1/AC2 (`tests/eval/test_intent_precondition_guard.rs`),
les tests inline d'`outside_reviewed_plan`, et l'assertion que les deux noms de
`VERDICT_PRODUCER_SKILLS` nomment des répertoires existants sous `skills/bundled/`.
La section est donc requise.

**Option retenue : (a) exception nommée en allowlist — avec une allowlist livrée
VIDE, et épinglée vide.**

Mesure qui justifie le choix, faite avant de l'écrire :

| détecteur | violations existantes | mesure |
|---|---|---|
| test AC1/AC2 | 0 | le comportement visé est livré dans le même vol (commit 2 avant commit 3) : à l'atterrissage le détecteur est vert par construction, pas par exception |
| tests inline `outside_reviewed_plan` | 0 | fonction neuve, aucun appelant antérieur |
| noms de `VERDICT_PRODUCER_SKILLS` | 0 | `skills/bundled/mika-arch-groom-ticket/` et `skills/bundled/mika-arch-second-review/` existent tous les deux dans l'arbre à ce commit |

Les trois détecteurs atterrissent donc **armés et verts**, et l'allowlist de la
troisième est livrée vide. Ce qui est livré avec elle, et qui est la moitié
portante de l'option (a) :

- **l'assertion auto-nettoyante** — l'allowlist est épinglée vide par une
  assertion sœur (`assert!(ALLOWLIST.is_empty(), …)` avec le message qui dit quoi
  faire), si bien qu'une entrée ajoutée un jour doit l'être *délibérément*, en
  éteignant l'épingle, ce qui est le geste visible que l'option (a) exige. C'est
  l'idiome déjà en place dans ce dépôt (« allowlist shipped empty, pinned empty by
  a sibling test », mika#2425 / mika#1833) ;
- **le contrôle de bonne foi** — le test AC1/AC2 a été vu rouge avec la ligne du
  commit 2 retirée (§ 4, étape 3, et c'est une case du § 5). Un détecteur dont on
  n'a pas constaté le rouge ne détecte rien : il passe la revue, il passe la CI
  par construction, et le défaut atterrit (doctrine `suite-wiring-lint`,
  mika#2578 / mika#2752).

Ni (b) — livrer désarmé — ni (c) — halte-et-remontée — ne sont retenus : il n'y a
aucune violation préexistante à absorber, donc rien à désarmer et rien à faire
cadrer par l'opérateur.

---

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/review_anchor.rs` (`outside_reviewed_plan` + doc) | 40 |
| `crates/mika-agent/src/agent_loop/mod.rs` (`IntentScope` + `impl` + champ + 5 déclarations + site + 2 sondes) | 85 |
| tests inline (`review_anchor.rs` : 3 cas de marqueurs ; `mod.rs` : `IntentScope::triggered`) | 75 |
| tests (`tests/eval/test_intent_precondition_guard.rs` — AC1/AC2 + helper de registre) | 125 |
| `tests/eval/grounding_regressions/review_anchor_unparsed_retry_2996.rs` (commentaire `REPLIES`) | 10 |

Total estimé : 335 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 335 < 429, donc sous la borne par
vol, et très loin des 1000 lignes du seuil par PR. Trois faits bornent
l'estimation plutôt que de l'espérer :

1. **Le discriminateur n'est pas à construire.** `VERDICT_PRODUCER_SKILLS`,
   `has_verdict_producer_skill` et le paramètre `is_verdict_producer` de
   `run_loop` existent déjà et sont déjà lus par deux autres gardes (§ 1.3). Le
   vol ne crée aucun chemin de résolution, aucun threading de paramètre, aucune
   signature de `run_loop` modifiée — ce sont exactement les postes qui font
   exploser un vol de moteur.
2. **Le site de production change d'une ligne.** La table du § 2.3 compare quatre
   formes ; les trois écartées coûtaient entre 3 et 40 lignes de plomberie, et la
   retenue coûte un `match` à deux bras. Le volume est dans les **tests** (200 des
   335 lignes), qui ne sont pas des tours de débogage.
3. **Aucune migration, aucun schéma, aucun prompt.** Rien sous
   `skills/bundled/`, donc pas de chaîne `rebuild → seed → read` à faire tenir
   dans le vol ; rien sous `src/db/`, donc pas de version de schéma.

Le découpage en vols n'est donc pas requis, et aucun reste n'est renvoyé par une
ligne `Tracked in:` au titre de la taille. La seule ligne `Tracked in:` du vol est
celle du § 2.6, qui renvoie une **classe de défaut** hors périmètre d'AC1 — pas
une tranche de ce travail.
