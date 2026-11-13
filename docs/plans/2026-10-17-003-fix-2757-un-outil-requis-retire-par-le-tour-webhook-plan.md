# mika#2757 — Un outil requis retiré par le tour webhook ne doit pas forcer un re-prompt (vol 2 de mika#2755)

> Plan content-only produit par `/mika-groom-plan-only`. Ticket : senara-solutions/mika#2757.
> Labels portés : `bug`, `p2-normal`, `ready`, `dispatch:loop`, `loop-substrate`.
> Vol 1 : PR mika#2756, plan `docs/plans/2026-10-17-002-fix-2755-une-branche-citee-nest-pas-du-texte-dintention-plan.md`.

---

## 1. Contexte : la contradiction, et non plus sa cause particulière

Le vol 1 a fermé **une porte d'entrée** : un slug de branche cité par un
événement `check_suite` ne peut plus être lu comme du texte d'intention
(`mask_cited_branch`). Il n'a volontairement couvert ni le titre ni le corps
cités, et le § 2.3 de son plan a nommé le vol 2 : *« `required_tools` et
`webhook_no_unauthorized_dispatch` ne peuvent pas se contredire sur un même
tour »*.

L'occurrence du 2026-10-17 ≈ 01:01Z est passée par une **autre** porte : le
message `[GitHub] Issue closed: senara-solutions/mika#2750 — …` cite le **corps**
du ticket, et ce corps contient le mot `groom`. Aucun masque de donnée citée ne
l'attrape — un corps de ticket *est* du texte humain. Ce vol ne ferme donc pas
une porte de plus : il ferme la pièce.

### 1.1 Ce que le code fait aujourd'hui, site par site

Trois couches du moteur se prononcent sur ce tour, et deux d'entre elles se
contredisent.

**(1) L'exigence.** `dev-groom` déclare
(`skills/bundled/dev-groom/skill.toml:17`) :

```toml
keywords = ["groom", "groom ticket", …]
required_tools = ["run_claude_pilot_groom"]
```

`match_skills` voit `groom` dans le corps cité, rend `MatchReason::Keyword`, et
`collect_required_tools` (`crates/mika-agent/src/agent_loop/mod.rs:8187`) —
qui ne lit que `Keyword` (#463) — pose l'exigence
`{"run_claude_pilot_groom"}`.

**(2) L'interdit.** `webhook_no_unauthorized_dispatch` est une entrée de
`INTENT_GUARDS` (`agent_loop/mod.rs:10269`). Son trigger est
`is_unauthorized_webhook_dispatch` (`webhook_dispatch.rs:72`), alias du domaine
Webhook Fallthrough : un message qui commence par `[GitHub]`, qui n'est ni le
marqueur ready-label, ni un `[GitHub] PR …`, ni un `[GitHub] Check suite …`.
`Issue closed:` satisfait les quatre conditions — **le trigger est vrai**. Son
prédicat satisfait (`agent_loop/mod.rs:10483`) :

```rust
!summaries.iter().any(|s|
    (s.name == "run_claude_pilot" || s.name == "run_claude_pilot_groom") && s.success)
```

c'est-à-dire : appeler `run_claude_pilot_groom` **avec succès** sur ce tour est
une violation (mika#841 couche 1, incidents #798 / #838 / #910).

**(3) Le retrait.** `effective_disabled_tools` (`agent_loop/mod.rs:8809`) élargit
la denylist d'identité avec `FALLTHROUGH_WITHHELD_TOOLS` — qui vaut
`["create_task"]` et rien d'autre (mika#2517 U2). C'est ce retrait que la ligne
`Webhook Fallthrough turn — tools withheld (mika#2517)` journalise, et c'est
pourquoi le ticket parle d'« outils de dispatch retirés » : **la lecture est
approximative et il faut la corriger ici, parce que le correctif en dépend.**
Sur ce tour précis, `run_claude_pilot_groom` n'était pas retiré du tableau
d'outils — il était *servi* et *interdit de réussir*. C'est la couche (2) qui
contredisait (1), pas la (3).

### 1.2 Pourquoi le défaut est la contradiction et rien d'autre

Le tour tenait deux ordres incompatibles : *appelle `run_claude_pilot_groom`* et
*n'appelle pas `run_claude_pilot_groom`*. Le premier a tiré — `WARN … agent
responded without calling required tools — re-prompting`
(`agent_loop/mod.rs:2217`) — et le modèle a passé le reste du tour à chercher une
sortie qui ressemble à une action de grooming sans franchir l'interdit : trois
tâches `Placeholder…` créées puis annulées, et une conclusion fausse (« un
`create_task` manquant à restaurer »).

Un point de mesure à conserver, parce qu'il borne ce que ce vol prouve :
`create_task` **était retiré du tableau d'outils de ce tour** (couche 3). Une
tâche née de ce tour-là ne peut donc pas être venue de `create_task` ; la
cascade de tâches a eu lieu ailleurs — sur un tour ultérieur, hors du domaine, où
l'outil est servi à nouveau. Ce vol retire la contradiction qui a lancé la
cascade ; il ne reproduit pas la cascade et ne la revendique pas (§ 7).

### 1.3 Le troisième site de retrait, et ce qu'il raconte

`filter_ci_excluded_tools` (`agent_loop/mod.rs:10758`) retire **vraiment** les
deux outils de dispatch du tableau servi sur un événement de suite de checks :

```rust
const CI_EXCLUDED_TOOLS: &[&str] = &["run_claude_pilot", "run_claude_pilot_groom"];
```

C'est exactement l'ensemble qu'écrit `webhook_no_unauthorized_dispatch_satisfied`
— **une deuxième copie, déjà écrite.** Et c'est la couche qui produisait la
contradiction du **vol 1** : sur le `check_suite` du 2026-10-16, l'outil requis
par le slug était retiré du tableau. Les deux occurrences du ticket parent ont
donc chacune leur couche : la (3) pour le vol 1, la (2) pour le vol 2. Une règle
qui désarme sur **les deux** couvre les deux occurrences — c'est l'affirmation du
ticket, et c'est ce qui la rend vraie.

### 1.4 Le site où l'exigence effective est calculée

Il existe et il est unique : `filter_available_required_tools`
(`agent_loop/mod.rs:8550`), appelée une seule fois, en tête de `run_loop`
(`agent_loop/mod.rs:1447`), par où passent les **trois** modes (conversation
`:6004`, silent `:7147`, team `:7766`). Elle filtre déjà pour une raison
voisine — *l'outil n'existe pas* (#516/#517) — et son résultat
`effective_required_tools` est la seule donnée que lit le re-prompt
(`:2165`, `:2168`, `:2192`). C'est le site que le ticket désigne.

Deux faits rendent le site exact plutôt que commode :

- `enabled_tool_names` — le **tableau d'outils réellement servi au modèle**,
  pris en instantané au début du tour (#862) après denylist d'identité, filtre
  CI et MCP — est déjà un paramètre de `run_loop` (`:1376`).
- `user_input_text` (`:1549`) — la chaîne que `run_loop` extrait de
  `request.messages` et sur laquelle **les `INTENT_GUARDS` sont évalués**
  (`:3258`, `(guard.trigger)(&user_input_text)`) — est extraite dans la même
  fonction, quelques lignes plus bas.

Autrement dit : **la chaîne qui arme l'interdit et l'ensemble d'outils servi sont
tous deux déjà en main, dans la fonction même où l'exigence effective est
calculée.** Le désarmement n'a besoin d'aucun nouveau canal.

---

## 2. La décision

### 2.1 Le principe

**Une exigence que le tour lui-même rend impossible à satisfaire n'est pas une
exigence : c'est une contradiction, et le moteur la désarme en la nommant.**

Deux formes d'impossibilité, et une seule règle :

| forme | prédicat | couche qui la produit |
|---|---|---|
| l'outil **n'est pas servi** sur ce tour | `!served.contains(t)` | denylist d'identité, `FALLTHROUGH_WITHHELD_TOOLS` (mika#2517 U2), `filter_ci_excluded_tools` (mika#1324), filtre provider compact (mika#1491) |
| l'outil est servi mais le tour **interdit qu'il réussisse** | `is_dispatch_tool(t) && is_unauthorized_webhook_dispatch(msg)` | `webhook_no_unauthorized_dispatch` (#910) |

Le désarmement est un **retrait de l'exigence**, jamais un retrait de capacité :
l'outil reste dans le tableau servi quand il y était, le prompt de la skill reste
injecté, et l'interdit de la couche (2) reste entier. Ce qui disparaît est
l'injonction de l'appeler — exactement la forme du geste de mika#2517 U3, qui a
retiré l'injonction de `webhook_zero_tools` sur ce même domaine sans toucher à la
protection contre le dispatch.

### 2.2 Pourquoi la règle lit la même chaîne que le guard, et non une autre

Le désarmement reçoit `user_input_text` — **le même `&str`, dans la même
fonction, que `(guard.trigger)(&user_input_text)`**. C'est ce qui rend
l'invariant structurel plutôt que conventionnel : les deux couches ne peuvent pas
être en désaccord sur *quel tour* est concerné, dans aucun des trois modes, parce
qu'elles lisent un unique binding. Une seconde extraction du message aurait été
libre de désigner un autre tour, et c'est précisément la classe de divergence
que ce dépôt a payée deux fois (mika#2158, mika#2362).

Conséquence d'ordre, et c'est tout le changement de structure : `user_input_text`
remonte de `:1549` à juste au-dessus de `:1447`. Entre les deux lignes, rien ne
lit ni `request.messages` ni `effective_required_tools` — le déplacement est une
remontée de déclaration à sémantique nulle, et les deux valeurs restent calculées
une fois chacune, avant la boucle.

### 2.3 Pourquoi un site unique pour l'ensemble de dispatch, et jusqu'où

L'ensemble `{run_claude_pilot, run_claude_pilot_groom}` est aujourd'hui écrit
**sept fois** en code de production, toutes dans `agent_loop/mod.rs`
(`:3798`, `:10410`, `:10488`, `:10587`, `:10651`, `:10713`, `:10762`). Le vol 1 a
promis de *« lire l'ensemble au seul site qui le définit plutôt que d'en écrire
une seconde copie »* — or il y en a déjà sept, et le désarmement en voudrait une
huitième.

Ce vol en fait **une seule**, et convertit les sept. La conversion est une
substitution à sémantique nulle (même ensemble, même question, même résultat),
chacune sur une à deux lignes, et chacune est déjà couverte par ses tests
existants. C'est le prix d'honnêteté du mot « un seul site » : une constante
nommée dont il reste six copies à côté est une constante décorative.

Un site ne bouge pas, et il faut le dire pour qu'on ne le lise pas comme un
oubli : `:2536` (`s.name == "run_claude_pilot_groom" && s.success`, garde de
fabrication dev-groom #1133/#1254) parle d'**un** outil, pas de l'ensemble. Ce
n'est pas une copie.

### 2.4 Le niveau de journal : `INFO`, et c'est une décision

Le désarmement est **nominal** sur le domaine Webhook Fallthrough : il tirera
des dizaines de fois par jour, pour un comportement correct. Un `WARN` à ce
volume est un `WARN` qu'on apprend à ignorer — c'est la leçon des « 25+
documented misfires » de mika#1469. La ligne est donc `INFO`, avec un champ
`reason` pour que l'opérateur puisse séparer les deux formes par `GROUP BY`. Le
`WARN` existant de la branche « outil inexistant » (`:8563`, config de skill
obsolète) reste un `WARN` : celui-là accuse une erreur de configuration, pas un
tour.

---

## 3. Les livrables, site par site

### D1 — `crates/mika-agent/src/webhook_dispatch.rs` : le site unique de l'ensemble de dispatch

Ce module possède déjà les prédicats du domaine et son doc-module porte la
doctrine : *« One definition, so the four consumers cannot drift »*. L'ensemble
d'outils que ces prédicats gouvernent appartient au même endroit.

**D1a — la constante.**

```rust
pub(crate) const DISPATCH_TOOLS: &[&str] = &["run_claude_pilot", "run_claude_pilot_groom"];
```

Doc-comment nommant : les sept sites convertis par D2, la raison du site unique
(§ 2.3), et le fait que `:2536` n'est pas un huitième.

**D1b — le prédicat.**

```rust
pub(crate) fn is_dispatch_tool(name: &str) -> bool
```

Comparaison exacte sur `DISPATCH_TOOLS`. Un prédicat et non la constante nue,
parce que c'est la forme que les sept sites appellent — ils demandent tous « ce
nom est-il un outil de dispatch ? », jamais « donne-moi la liste ».

### D2 — `crates/mika-agent/src/agent_loop/mod.rs` : les sept sites passent par la couture

Substitutions, sémantique inchangée :

| site | avant | après |
|---|---|---|
| `:3798` | `(s.name == "run_claude_pilot" \|\| s.name == "run_claude_pilot_groom")` | `is_dispatch_tool(&s.name)` |
| `:10410`, `:10587`, `:10651`, `:10713` | idem (quatre prédicats « un dispatch a-t-il eu lieu ? ») | idem |
| `:10488` | idem `&& s.success` | `is_dispatch_tool(&s.name) && s.success` |
| `:10762` | `const CI_EXCLUDED_TOOLS: &[&str] = &[…]` | la constante locale **disparaît** ; `retain(\|d\| !is_dispatch_tool(&d.name))` |

Le doc-comment de `filter_ci_excluded_tools` gagne une phrase : l'ensemble qu'il
retire est celui que `webhook_no_unauthorized_dispatch` interdit, et c'est la
même liste parce que c'est le même ensemble — ce que la constante locale laissait
être une coïncidence.

### D3 — `filter_available_required_tools` : le désarmement

Nouvelle signature, deux paramètres :

```rust
fn filter_available_required_tools(
    required: &HashSet<String>,
    tools: &ToolRegistry,
    skill_tool_map: &HashMap<String, &ResolvedSkillTool>,
    mcp_manager: Option<&McpManager>,
    // mika#2757 — le tableau d'outils réellement servi (#862) et la chaîne sur
    // laquelle les INTENT_GUARDS sont évalués. Les deux déjà en main dans `run_loop`.
    served: &HashSet<String>,
    turn_message: &str,
) -> HashSet<String>
```

Trois raisons de retrait, trois journaux distincts, dans cet ordre :

1. **l'outil n'existe pas** — comportement actuel inchangé, `WARN` inchangé
   (#516/#517) ;
2. **l'outil existe mais n'est pas servi sur ce tour** —
   `INFO event = "required_tool_disarmed", tool = …, reason = "not_served"` ;
3. **l'outil est servi mais le tour interdit qu'il réussisse** —
   `is_dispatch_tool(t) && webhook_dispatch::is_unauthorized_webhook_dispatch(turn_message)`
   → `INFO event = "required_tool_disarmed", tool = …, reason = "turn_forbids_dispatch"`.

Les branches 1 et 2 ne sont **pas** redondantes et le doc-comment doit le dire,
sinon un relecteur verra un doublon : `served` est un sous-ensemble strict de ce
que `tools.get()` connaît (denylist d'identité, filtre CI, filtre provider
compact), et les deux messages ne disent pas la même chose — « corrige la config
de la skill » contre « ce tour-ci ne sert pas cet outil ». Confondre les deux
ferait lire une restriction normale comme une erreur de configuration.

Doc-comment : le contrat des trois branches, le renvoi au § 2.1 pour la règle, et
la portée — la valeur rendue n'est lue que par le re-prompt et par
`has_terminal_required_tool_failure` ; rien d'autre ne la voit, et aucune
capacité n'est retirée.

**Effet de bord assumé, et nommé parce qu'il dépasse l'AC :** sur un provider
compact (mika#1491), le tableau servi est réduit à
`COMPACT_PROVIDER_CORE_TOOLS` et les outils de skill sont absents — la branche 2
y désarmera donc des exigences qui, aujourd'hui, produisent un re-prompt pour un
outil que le modèle ne peut pas voir. C'est le même défaut et le même remède ;
le `reason = "not_served"` le rend comptable au lieu de le rendre invisible.

### D4 — `run_loop` : le site d'appel

`user_input_text` (`:1549`) remonte juste au-dessus de `effective_required_tools`
(`:1447`), et l'appel passe `&enabled_tool_names` et `&user_input_text`. Un
commentaire de quatre lignes au site, portant l'argument du § 2.2 : c'est le
**même binding** que celui des `INTENT_GUARDS`, et c'est ce qui interdit aux deux
couches de désigner deux tours différents.

Les trois appelants de `run_loop` ne changent pas d'une ligne : les deux valeurs
sont déjà des paramètres ou des locales.

### D5 — tests de `filter_available_required_tools` (`mod tests` de `agent_loop/mod.rs`)

Cinq cas, chacun pour une affirmation distincte :

1. **le cas du ticket** — `required = {"run_claude_pilot_groom"}`, outil servi,
   `turn_message` = le message `[GitHub] Issue closed: senara-solutions/mika#2750 — …`
   mot pour mot → `∅` ;
2. **le contrôle négatif humain** — même `required`, même tableau servi,
   `turn_message = "groom mika#2750"` → `{"run_claude_pilot_groom"}`. Sans lui,
   le cas 1 passerait aussi sur un désarmement qui désarme tout ;
3. **un outil non servi** — `required = {"create_task"}` absent de `served`,
   message hors domaine → `∅`, `reason = not_served` (la forme du vol 1 et du
   provider compact) ;
4. **mika-qa n'est pas désarmé** — `required = {"qa_pr_view", "run_gh"}`, tous
   servis, `turn_message = "[GitHub] PR review (approved) on …"`. Ce message
   n'est **pas** dans le domaine fallthrough (préfixe PR) et aucun de ces outils
   n'est un outil de dispatch → l'ensemble est rendu intact. C'est l'assertion
   qui distingue cette règle de « désarmer sur tout événement webhook » ;
5. **le dispatch ready-label n'est pas désarmé** —
   `required = {"run_claude_pilot"}`, outil servi,
   `turn_message = "[GitHub] Issue labeled ready on senara-solutions/mika#1234 — …"`
   → intact. Le seul tour webhook qui **doit** dispatcher (mika#841, mika#1384)
   garde son exigence.

### D6 — le test de l'AC : la chaîne complète (`mod tests` de `agent_loop/mod.rs`)

Les cas de D5 épinglent la fonction ; l'AC du ticket porte sur la chaîne qui
mène au re-prompt. Un test l'exerce de bout en bout sur le message mesuré :

```
match_skills(&skills, MSG) → collect_required_tools(…) → filter_available_required_tools(…) → ∅
```

Scène construite avec `make_skill_entry_with_constraints` (`:12337`), qui pose
`keywords = [name]` : un skill nommé `"groom"` portant
`required_tools = ["run_claude_pilot_groom"]` reproduit le tour sans ajouter un
builder. Deux assertions, et la seconde est ce qui donne sa valeur à la première :

- sur le message du ticket, `collect_required_tools` rend bien
  `{"run_claude_pilot_groom"}` — **le mot-clé s'arme toujours**, le masque du
  vol 1 ne couvre pas un corps cité — puis l'exigence effective est `∅` ;
- sur `"groom mika#2750"`, les deux étapes rendent `{"run_claude_pilot_groom"}`.

C'est cette première assertion qui situe le correctif : il désarme, il n'empêche
pas de matcher.

### D7 — le détecteur : un seul corps écrit l'ensemble de dispatch (`canonical_tokens.rs`)

Voisin immédiat de `mika2517_the_fallthrough_domain_has_a_single_definition`, et
sur son modèle exact.

- **Le corpus** est `production_sources_to_test_module()` (`:2670`) — la source
  de production coupée avant son `mod tests`, donc le needle écrit par le test
  ne s'auto-accuse pas.
- **Le needle est la forme du prédicat, pas le littéral** :
  `name == "run_claude_pilot"` (guillemet fermant inclus, donc
  `run_claude_pilot_groom` ne matche pas). Le littéral nu `"run_claude_pilot"`
  apparaît dans dix-sept fichiers de production pour des raisons légitimes
  (labels de tâches, allowlists de skills, fixtures de calibration) — un scan sur
  le littéral serait faux et son allowlist serait un alibi. La forme du prédicat,
  elle, n'apparaît aujourd'hui que sept fois, **toutes** dans
  `agent_loop/mod.rs`, et zéro fois après D2.
- **La fonction de détection est extraite** (`fn dispatch_pair_predicate_sites(src: &str) -> Vec<usize>`)
  pour pouvoir être exercée sur une chaîne fabriquée, parce que le scan réel
  attend `∅` : un détecteur dont le résultat attendu est le vide ne se distingue
  pas d'un détecteur mort (mika#2103 / mika#2205). Trois tests :
  1. le scan réel sur le corpus → aucun site ;
  2. le détecteur sur un extrait fabriqué portant la forme → exactement un site
     accusé (l'anti-vacuité) ;
  3. l'allowlist est vide (test auto-nettoyant, § Fire-Disposition).

Message d'échec portant la résolution : *appeler
`webhook_dispatch::is_dispatch_tool` ; ne pas ajouter d'entrée à l'allowlist.*

---

## Fire-Disposition

Le vol livre des tests et un détecteur structurel dont le chemin de succès est
« aucune violation trouvée » : le gate s'applique.
**Option (a) — exception nommée en allowlist — et l'allowlist est livrée vide,
par construction.**

- **Violations existantes : zéro, et c'est le vol lui-même qui les met à zéro.**
  Le seul détecteur à corpus est D7. Son corpus porte aujourd'hui **sept**
  violations — les sept sites de `agent_loop/mod.rs` recensés au § 2.3 — et
  **D2 les convertit toutes les sept dans le même vol**. Le détecteur atterrit
  donc armé sur un corpus propre. C'est l'ordre de commit du § 5 qui le garantit
  mécaniquement : D7 arrive après D2.
- **Aucune entrée d'exception n'est écrite.** Le recensement est clos et
  vérifiable (`grep -n 'name == "run_claude_pilot"' crates/*/src/**/*.rs`), donc
  il n'y a rien à allowlister : « on déclare, on n'allowliste pas »
  (`canonical_tokens.rs`, § du même nom).
- **L'assertion auto-nettoyante** est `mika2757_the_dispatch_pair_allowlist_is_empty`,
  sur le modèle de `mika2517_the_domain_definition_allowlist_is_empty` : une
  allowlist née vide est un emplacement où déposer la prochaine infraction
  (mika#2323), et ce test rougit le jour où quelqu'un y dépose quoi que ce soit.
  La constante `DISPATCH_PAIR_PREDICATE_ALLOWED` est livrée vide et doit le
  rester.
- **L'anti-vacuité est un livrable, pas une précaution** (D7 test 2). Le scan
  réel attend le vide ; sans le test sur chaîne fabriquée, un détecteur dont le
  needle ne viserait plus rien — parce que la forme du prédicat a changé — se
  lirait exactement comme un dépôt propre.
- **D5 et D6 ne sont pas des détecteurs à corpus** : ce sont des assertions de
  comportement sur des entrées qu'elles construisent elles-mêmes. Elles n'ont
  rien à allowlister et ne reçoivent pas d'entrée.
- **Les gardes existantes que ce vol réveille** — les tests des sept sites
  convertis (`test_webhook_no_unauthorized_dispatch_satisfied_rejects_both_tools`,
  les trois tests mika#1324 de `filter_ci_excluded_tools`, les tests
  `test_collect_required_tools_*`) — sont livrées armées et satisfaites par le
  vol. Aucune n'est désarmée, aucune n'est exemptée, aucune n'est réécrite : une
  substitution à sémantique nulle doit passer ses tests **sans y toucher**, et
  c'est la vérification que D2 est bien une substitution.

Aucun livrable n'atterrit avec `#[ignore]`, et rien ne remonte à l'opérateur.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| D1 `crates/mika-agent/src/webhook_dispatch.rs` (constante + prédicat + doc) | 45 |
| D2 `crates/mika-agent/src/agent_loop/mod.rs` (sept sites convertis) | 25 |
| D3 `filter_available_required_tools` (signature, trois branches, journaux, doc) | 70 |
| D4 `run_loop` (remontée de `user_input_text`, site d'appel, commentaire) | 20 |
| D5 tests de `filter_available_required_tools` (cinq cas) | 100 |
| D6 test d'AC, chaîne complète + contre-épreuve | 75 |
| D7 détecteur `canonical_tokens.rs` (fonction extraite + trois tests + allowlist) | 65 |

Total estimé : 400 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 400 est sous la borne par vol (429)
et très en deçà du seuil par PR (1000). Les sept livrables tiennent sur une
**unique couture** — l'ensemble d'outils de dispatch et le site de calcul de
l'exigence effective — et aucun n'est séparable sans livrer un demi-correctif :
D3 sans D1/D2 écrirait la huitième copie que le vol 1 a explicitement refusée ;
D7 sans D2 atterrirait sur sept violations ; D5/D6 sans D3 n'auraient rien à
mesurer. Le volume de code de production (D1–D4 ≈ 160 lignes) est petit ; les
240 lignes restantes sont des tests, dont le coût en tours est linéaire et sans
surprise — pas de nouvelle couche, pas de migration de schéma, pas de nouveau
canal de données.

---

## 4. Contrat de vérification

| # | affirmation | comment elle est tenue |
|---|---|---|
| V1 | le message mesuré ne produit plus de re-prompt `required tools` | D6 — l'exigence effective est `∅`, et le re-prompt est gardé par `!effective_required_tools.is_empty()` (`:2165`) |
| V2 | …et le mot-clé s'arme toujours | D6, première assertion — `collect_required_tools` rend `{"run_claude_pilot_groom"}` sur le message du ticket. Le vol désarme, il n'empêche pas de matcher |
| V3 | une vraie demande humaine arme toujours l'outil | D5 cas 2 + D6 contre-épreuve — `"groom mika#2750"` rend `{"run_claude_pilot_groom"}` de bout en bout |
| V4 | une ligne de journal nomme l'outil désarmé | D3 — `event = "required_tool_disarmed"`, champs `tool` et `reason`, deux valeurs de `reason` séparables par `GROUP BY` |
| V5 | la règle couvre les deux occurrences du ticket parent | D5 cas 1 (couche « interdit », l'occurrence `Issue closed`) + D5 cas 3 (couche « non servi », l'occurrence `check_suite` du vol 1) |
| V6 | mika-qa et le dispatch ready-label ne sont pas désarmés | D5 cas 4 et 5 — les deux tours webhook qui portent des exigences légitimes les gardent intactes |
| V7 | la conversion des sept sites est à sémantique nulle | les tests existants de ces sept sites passent **sans modification** (`cargo test -p mika-agent --lib agent_loop`) |
| V8 | l'ensemble de dispatch n'a qu'un corps | D7 — scan réel vide, anti-vacuité sur chaîne fabriquée, allowlist vide |

Commandes, dans l'ordre :

```
cargo test -p mika-agent --lib webhook_dispatch
cargo test -p mika-agent --lib agent_loop
cargo test -p mika-agent --lib canonical_tokens
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/verify-pipeline.sh origin/main
```

**Rouge-avant exigé pour V1.** Le test de D6 doit être vu échouer sur l'arbre
non modifié — c'est-à-dire avant D3. Un test d'AC qui n'a jamais été rouge
n'atteste de rien : il pourrait passer parce que la scène est mal construite
(mauvais mot-clé, skill mal nommé, message qui n'entre pas dans le domaine)
plutôt que parce que le désarmement fonctionne. L'ordre de commit du § 5 rend ce
rouge-avant mécanique. Même exigence pour le test 2 de D7, qui doit accuser son
extrait fabriqué.

---

## 5. Ordre de commit

1. **D1 + D2** — la constante, le prédicat, les sept sites convertis. Compile et
   passe seul, avec les tests existants **inchangés** : c'est la preuve que la
   substitution est une substitution (V7).
2. **D6 seul, attendu ROUGE** — le test d'AC sur l'arbre où
   `filter_available_required_tools` ne désarme encore rien. Constater le rouge
   et le reporter dans le corps de PR.
3. **D3 + D4** — le désarmement et son site d'appel. D6 passe au vert ; c'est le
   commit qui porte le correctif.
4. **D5** — les cinq cas, dont les trois contrôles négatifs sans lesquels le
   correctif n'a pas montré qu'il ne désarmait pas mika-qa ni le ready-label.
5. **D7** — le détecteur, sur un corpus que l'étape 1 a déjà mis à zéro.
6. Suite complète, clippy, fmt, `verify-pipeline.sh`.

Le plancher de ce vol est l'étape 3 : en deçà, rien n'est corrigé. Les étapes 4 à
6 ne sont pas facultatives pour autant — sans D5 cas 4 et 5, le vol n'a pas
montré qu'il ne désarmait pas les deux tours webhook qui doivent agir.

---

## 6. Risques

| risque | évaluation |
|---|---|
| le désarmement sur « non servi » mord plus large que l'AC | Vrai, assumé, nommé (§ D3) : il couvre aussi le provider compact et le filtre CI. Dans chacun de ces cas, exiger un outil absent du tableau servi est la même contradiction et le même défaut. Le `reason = "not_served"` rend la population comptable ; sans la branche, le vol ne couvrirait pas l'occurrence du vol 1 (V5) |
| `enabled_tool_names` ne serait pas le vrai tableau servi | Vérifié avant d'écrire : `inject_skills_and_resolve_tools` part de `tools.definitions().to_vec()` (**tous** les builtins, `:9046`), applique la denylist d'identité, ajoute les outils de skill, puis l'appelant étend avec MCP et applique `filter_ci_excluded_tools` **avant** l'instantané (`:5574-5580`). `served` est donc le tableau exact, builtins compris. Un `served` qui aurait manqué les builtins aurait désarmé en masse — c'est le risque qui a motivé la vérification |
| un `s.name` comparé par `is_dispatch_tool` change de casse | Comparaison exacte, identique aux sept `==` remplacés. Pas de `eq_ignore_ascii_case` : l'introduire serait un changement de sémantique déguisé en refactor, et les noms d'outils sont produits par le moteur, pas saisis |
| la remontée de `user_input_text` change un comportement | Remontée de déclaration à sémantique nulle : la valeur ne dépend que de `request.messages`, que rien ne modifie entre `:1447` et `:1549` (le premier `push` a lieu dans la boucle). Les deux valeurs restent calculées une fois chacune |
| le détecteur D7 devient faux en silence | C'est la fonction du test 2 (anti-vacuité sur chaîne fabriquée). Un needle qui ne vise plus rien rougit au lieu de se lire comme un dépôt propre |
| un tour silent ou team porterait un message `[GitHub]` | Alors le désarmement s'y applique — et c'est correct : les `INTENT_GUARDS` y sont évalués aussi (le bloc `:3250` n'est gardé par aucun test de mode), donc l'interdit y vaut aussi. Les deux couches restent d'accord par construction (§ 2.2) |
| la tâche fantôme déjà en base | Hors périmètre code, § 7 |

---

## 7. Hors périmètre

- **La cascade de tâches `Placeholder…` et la fausse piste du `create_task`
  manquant.** Ce vol retire la contradiction qui a lancé la cascade ; il ne
  reproduit pas la cascade et ne revendique pas de la fermer. Le fait
  établissable est négatif et borné : sur le tour mesuré, `create_task` était
  retiré du tableau servi (mika#2517 U2), donc une tâche née de **ce** tour-là
  n'a pas pu venir de cet outil. Où elle est née exactement — un tour ultérieur
  hors domaine, où l'outil est servi à nouveau, est la lecture la plus simple —
  demanderait une mesure que ce ticket ne porte pas. L'AC « ni tâche manuelle »
  est donc tenu **médiatement**, par la suppression de la cause, et le plan le
  dit plutôt que de l'affirmer.
- **Les tâches fantômes déjà en base** (`Placeholder…` annulées, et la
  `Groom mika#2750 (retry — engine correction)` du vol 1). Donnée de production,
  pas défaut de code. À mentionner dans le corps de PR pour que le geste
  opérateur ne se perde pas.
- **Le masque du titre cité.** Le raisonnement du vol 1 s'appliquerait au
  `— {titre}` des gabarits `issues` et `pull_request`, mais un titre *est* du
  texte humain et le ticket parent l'a laissé dehors. Ce vol le rend d'ailleurs
  inutile pour la classe qu'il ferme : un mot-clé dans un titre cité armera
  toujours le match, et le désarmement s'en occupera au même endroit que pour un
  corps.
- **La huitième écriture de l'ensemble de dispatch hors `agent_loop/mod.rs`.**
  Les dix-sept fichiers qui portent le littéral `"run_claude_pilot"` pour des
  raisons légitimes (labels de tâches, allowlists de skills, fixtures) ne sont
  pas des définitions de l'ensemble interdit et D7 ne les vise pas (§ D7).
- **`FALLTHROUGH_WITHHELD_TOOLS`.** Le vol ne l'élargit pas. Retirer un outil de
  plus du tableau servi est une décision de mika#2517, pas de celui-ci ; ce vol
  se contente de ne plus exiger ce que ce retrait rend impossible.

---

## Definition of Done

- [ ] `DISPATCH_TOOLS` et `is_dispatch_tool` vivent dans `webhook_dispatch.rs`, et
      les sept sites de `agent_loop/mod.rs` les consomment (`CI_EXCLUDED_TOOLS`
      supprimée).
- [ ] `filter_available_required_tools` désarme sur les deux formes
      d'impossibilité et journalise `required_tool_disarmed` avec `tool` et
      `reason`.
- [ ] `run_loop` lui passe `enabled_tool_names` et le **même** `user_input_text`
      que les `INTENT_GUARDS`.
- [ ] D5 (cinq cas), D6 (chaîne complète + contre-épreuve) et D7 (scan +
      anti-vacuité + allowlist vide) passent.
- [ ] Les tests existants des sept sites convertis passent **sans
      modification**.
- [ ] Le rouge-avant de D6 est constaté et reporté dans le corps de PR.
- [ ] `cargo test`, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` : verts.
- [ ] Corps de PR : le geste opérateur du § 7 (tâches fantômes en base) est
      nommé.

---

## Acceptance criteria

Transcrits du § *Critère d'acceptation minimal* de senara-solutions/mika#2757 :

1. **Un tour webhook en fallthrough (outils de dispatch retirés), dont le message
   contient `groom`, ne produit pas de re-prompt
   `agent responded without calling required tools`.**
   Tenu par D6 (exigence effective `∅`, et le re-prompt est gardé par
   `!effective_required_tools.is_empty()`).
2. **…ni de tâche manuelle.**
   Tenu médiatement : la contradiction qui produisait l'évasion disparaît, et
   `create_task` est déjà retiré du tableau servi sur ce tour (mika#2517 U2). La
   borne de cette affirmation est explicite au § 7.
3. **Une ligne de journal nomme l'outil requis désarmé.**
   Tenu par D3 : `event = "required_tool_disarmed"`, champ `tool`, champ
   `reason` ∈ {`not_served`, `turn_forbids_dispatch`}.
4. **Une vraie demande humaine « groom mika#N » arme toujours l'outil.**
   Tenu par D5 cas 2 et la contre-épreuve de D6 : hors du domaine fallthrough,
   l'exigence traverse intacte.
5. **La règle ne dépend pas de l'endroit où le mot-clé apparaît (slug, titre,
   corps), et couvre donc les deux occurrences de mika#2755.**
   Tenu par construction — le prédicat ne lit jamais *où* le mot-clé a été
   trouvé — et mesuré par D5 cas 1 (couche « interdit ») et cas 3 (couche « non
   servi »).
