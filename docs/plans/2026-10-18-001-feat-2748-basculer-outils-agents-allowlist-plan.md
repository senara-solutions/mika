# Basculer les outils des agents bien connus de la dénylist vers une allowlist (mika#2748)

**Ticket :** senara-solutions/mika#2748
**Parent :** mika#2593 § 8 (a) — `docs/plans/2026-10-16-005-feat-2593-peser-tool-defs-bytes-arch-plan.md`
**Type :** feat
**Vol :** 1 et unique (voir § Taille estimée)

## Contexte

mika#2593 a retiré neuf définitions d'outils du tableau servi à mika-arch par un
critère structurel C1 ∧ C2 ∧ C3 (lectures orphelines + un registre de créneaux de
dispatch différé), et a armé deux gardes qui l'épinglent. Le vol 1 est mergé
(PR #2749, commit `1d6d6d32`, 2026-10-16T22:08:34Z).

Le gain est réel mais **réérodable** : il est obtenu en *ajoutant* des noms à une
dénylist. Le prochain `register(...)` dans `default_tools()` ajoute une définition
au tableau servi sans que personne ait décidé qu'elle devait y être. La garde
`mika2593_le_tableau_servi_a_mika_arch_est_un_roster_atteste` arrête la croissance
**silencieuse** — elle rougit au prochain `register(...)` — mais pas la croissance
**décidée par inattention** : un implémenteur qui lit le message de la garde et
ajoute le nom au roster a satisfait la garde sans avoir rien décidé.

Sous une allowlist, un nouvel outil n'atteint mika-arch que si quelqu'un l'y écrit.
Le défaut change de sens : *absent* au lieu de *présent*. C'est ce qui rend le gain
permanent.

Le doc-comment d'`apply_agent_tool_visibility` (`agent_loop/mod.rs:8844`) invite
déjà ce basculement en ces termes — et la promesse qu'il porte borne le périmètre :

> *Future migration (when well-known agents move from denylist to allowlist for
> tools): extend this hook to handle both shapes — same call site, different
> predicate, no caller changes.*

Le commentaire du site d'appel (`inject_skills_and_resolve_tools`, lignes 9171–9173)
le redit : *« Apply per-agent visibility filter BEFORE skill tools are added — this
is the named hook that future allowlist migration will reuse. »*

## Résolution de la précondition

Le ticket pose comme précondition le relevé P2/P3 **mesuré** après déploiement du
vol mika#2593. Ce relevé n'est pas obtenable depuis la session de grooming
(`MIKA_SPIRIT_LOG_FILE` n'est pas défini dans le bac à sable de dispatch, et
`~/.mika` ne porte pas de `logs/`). Il n'est pas non plus **nécessaire**, et c'est
une propriété de ce vol-ci, pas une facilité :

**Le critère de ce vol est la préservation de valeur, pas le gain.** L'allowlist est
semée depuis exactement l'ensemble que la dénylist laisse debout aujourd'hui — 19
noms, § *L'allowlist* ci-dessous. Après basculement, `tool_defs_bytes` doit sortir
**inchangé**. Aucun nom n'est arbitré sur des octets.

Deux conséquences :

1. **Si la valeur mesurée sort de la bande 15,6–17,6 Ko de mika#2593**, ce n'est pas
   ce vol qui est invalidé : c'est l'**estimateur** de mika#2593 (sa prédiction par
   sérialisation de 4 252 o). C'est son suivi, pas le nôtre, et cela ne change pas
   une ligne d'ici.
2. **Le recensement par outil de `tool_defs_bytes` (§ 8 (d) de mika#2593) n'est ni
   un préalable ni un livrable de ce vol.** C'est la réponse à la question ouverte
   du ticket. Il serait l'instrument d'un vol qui *arbitre* nom par nom ; celui-ci
   **transcrit** un roster déjà attesté. Le jour où quelqu'un voudra retirer un nom
   *de plus* sur un critère budgétaire, cet instrument sera dû — et ce jour-là
   l'allowlist en fera le périmètre exact, ce qu'une dénylist ne permettait pas.
   `Tracked in: senara-solutions/mika#2593` (§ 8 (d) y est nommé ; un ticket dédié
   sera ouvert en fin de vol s'il n'existe pas encore).

Le relevé post-déploiement reste **dû**, mais comme **vérification** de ce vol
(§ Contrat de vérification, sondes post-déploiement), pas comme sa condition
d'entrée.

## Décision 1 — le prédicat

`apply_agent_tool_visibility` reçoit un paramètre de plus et garde **un seul** site
d'appel par chemin, conformément à la promesse de son doc-comment.

```rust
pub(crate) fn apply_agent_tool_visibility(
    tool_defs: &mut Vec<mika_common::claude::ToolDefinition>,
    disabled: &[String],
    allowed: Option<&[String]>,
    agent: &str,
)
```

Retour anticipé : `if disabled.is_empty() && allowed.is_none() { return; }`.

Prédicat de conservation, pour chaque définition `d` :

```
garder(d)  ⇔  (allowed.is_none() || allowed.contient(d.name))  &&  !disabled.contient(d.name)
```

Comparaison `eq_ignore_ascii_case` des deux côtés, comme aujourd'hui.

Trois propriétés, chacune délibérée :

- **`Option` porte le trois-états**, et c'est pour cela que le champ n'est pas un
  `Vec` à défaut vide : `None` = « pas d'allowlist, comportement actuel » ;
  `Some([])` = « n'autoriser aucun builtin » ; `Some([…])` = roster explicite.
  Avec un `Vec` nu, `Some([])` et `None` seraient le même octet et le trois-états
  serait inexprimable — un agent voulant tout refuser obtiendrait tout.
- **La dénylist soustrait *par-dessus* l'allowlist.** C'est ce qui préserve
  l'élargissement de fallthrough de mika#2517 (`FALLTHROUGH_WITHHELD_TOOLS`,
  `effective_disabled_tools` renvoyant un `Cow`) avec **zéro** changement à cette
  fonction : un tour Webhook Fallthrough ajoute `create_task` à la dénylist
  effective, et l'intersection reste la bonne réponse même sur un agent à
  allowlist.
- **Les deux formes coexistent, l'ordre n'importe pas** (intersection), mais
  `allowed ∩ disabled = ∅` est asserté pour mika-arch (§ Détecteur, assertion C) :
  un nom écrit des deux côtés est une décision écrite deux fois, pas une
  contradiction à arbitrer silencieusement.

**Borne dure à écrire explicitement** : le filtre tourne **avant** l'ajout des
outils de skill (lignes 9225–9229) et avant les outils MCP. L'allowlist ne gouverne
donc **que les builtins**. `gh_read` — l'outil le plus utilisé de mika-arch —
traverse sans être nommé, et c'est voulu : écrire les outils de skill dans
`[tools].allowed` les rendrait inertes, et le jour où ce point d'application
changerait, mika-arch perdrait son outil principal en silence. Même invariant que
le caveat « `[tools].disabled` ne peut pas refuser un outil MCP » déjà écrit dans
`docs/mcp.md:55,319` et `docs/architecture.md:430`.

## Décision 2 — l'allowlist, et pourquoi 19

`MIKA_ARCH_ALLOWED_TOOLS` = les 18 noms de
`MIKA_ARCH_SERVED_BUILTINS_FROM_DEFAULT_TOOLS` (`tools/mod.rs:1683`) **plus**
`list_agents`.

C'est exactement le `kept_count = 19` que P2 journalise. Le 18 ≠ 19 est déjà
expliqué par le doc-comment du roster existant : les 18 viennent de
`default_tools()`, et `list_agents` est le seul survivant des 14 outils de
`management_tools_if_needed` (13 sont dans la dénylist ; `list_agents` est conservé
délibérément par mika#2593 — § 8 (c), question sur le contrat d'auto-connaissance
de mika#740, hors périmètre ici).

Deux noms de management sont donc séparés dans la déclaration :

```rust
/// Les builtins de `default_tools()` que mika-arch a le droit de voir.
pub const MIKA_ARCH_ALLOWED_TOOLS: &[&str] = &[ /* 18 + list_agents, un par ligne */ ];
```

L'unique survivant de management (`list_agents`) est **nommé dans le doc-comment**
de la constante avec son ticket (#740 / mika#2593 § 8 (c)), pas isolé dans une
seconde constante : une constante de plus pour un élément ferait payer un site de
vérité supplémentaire à chaque lecture.

## Décision 3 — `tools.allowed` est code-owned, et le dépôt le sait déjà

`tools.allowed` rejoint `CODE_OWNED_IDENTITY_SECTIONS`
(`well_known_agents.rs:632`), à côté de `tools.disabled`.

**Sans cela, la déclaration serait inerte pour tout agent déjà provisionné** —
`write_default_if_missing` ne réécrit jamais un `identity.toml` existant, donc
mika-arch, provisionné bien avant ce vol, ne verrait jamais l'allowlist et
continuerait de tourner sous dénylist seule, avec une suite de tests verte
par-dessus. C'est exactement la classe mika#1833 (`[kg]`) / mika#2330
(`[context.history]`), et la question du ticket (« il doit l'être ») est la bonne
réponse.

Ce qui est notable, et qui réduit le risque de l'oublier à zéro : **le dépôt refuse
déjà l'omission.** `test_no_code_owned_drift_outside_constant`
(`well_known_agents.rs:4038`) asserte que tout chemin de profondeur ≤ 2 émis par un
template d'identité et non opérateur-owned figure dans
`CODE_OWNED_IDENTITY_SECTIONS`. Émettre `tools.allowed` sans le déclarer **fait
rougir la suite automatiquement**. Et `test_code_owned_sections_have_reconciler_coverage`
(`:4008`) exige en retour que la section déclarée soit couverte par le réconciliateur.
La classe inerte est structurellement prévenue — pas par un livrable de ce vol, par
un livrable de mika#2330 qui tient.

`reconcile_well_known_identity` (`:708`) n'a besoin d'**aucun** changement : il
remplace le sous-arbre d'un chemin pointé via `get_path`/`set_path` et fonctionne
déjà sur un chemin à deux segments. Le livrable est la ligne dans la constante, pas
du code de réconciliation.

## Décision 4 — le chemin fail-closed garde son comportement, et le coût est nommé

`prompt.rs::fail_closed_identity()` (`:1069`) est un **littéral de structure** :
ajouter un champ à `ToolsIdentityConfig` force une édition là, le compilateur ne
laisse pas le choix. Ce vol y écrit `allowed: None`.

C'est délibéré et le plan le dit plutôt que de le laisser deviner. Durcir le chemin
fail-closed en y écrivant l'allowlist de mika-arch toucherait **tout agent sans
`identity.toml`** — pas « les agents bien connus », toute la flotte, y compris
`mika` elle-même sur un hôte mal provisionné. C'est une décision de flotte, et elle
ferait perdre des outils à un agent sur un défaut de provisionnement, c'est-à-dire
au pire moment. `mika2027_absent_identity_toml_fails_closed` (`:3040`) reste vert
inchangé ; un commentaire au site nomme le coût et renvoie au suivi.

`Tracked in:` un ticket de suivi « durcir le chemin fail-closed à une allowlist »
sera ouvert en fin de vol (DoD 9).

## Décision 5 — la garde mika#2593 est remplacée, pas simplement supprimée

`mika2593_le_tableau_servi_a_mika_arch_est_un_roster_atteste` (`tools/mod.rs:1721`)
et sa constante `MIKA_ARCH_SERVED_BUILTINS_FROM_DEFAULT_TOOLS` (`:1683`) sont
**retirées**, et c'est le cœur du ticket, donc il faut dire pourquoi avec précision :

Cette garde détecte « un outil est servi à mika-arch sans que personne l'ait
décidé ». Sous une allowlist, ce fait **ne peut plus se produire** — ce n'est pas
qu'il devient indétecté, c'est qu'il devient impossible : un `register(...)` nouveau
n'atteint pas mika-arch tant que son nom n'est pas écrit dans
`MIKA_ARCH_ALLOWED_TOOLS`. Une garde qui surveille un état inatteignable n'est pas
une sécurité, c'est une égalité à recalculer — exactement ce que le ticket demande
de remplacer (« l'assertion que l'allowlist **est** le roster, plutôt qu'une égalité
à recalculer depuis la dénylist »).

**Ne pas réintroduire l'égalité `allowlist == registre − dénylist` comme garde
permanente.** Ce serait reconduire le couplage que ce vol défait : la dénylist
redeviendrait la source de vérité et l'allowlist sa copie dérivée, donc réérodable
par la même porte. L'égalité est vérifiée **une fois**, à l'implémentation, par les
sondes post-déploiement (§ Contrat de vérification) — pas par un test qui la
réaffirme à chaque `cargo test`.

Sous allowlist, **la constante *est* le registre de décision** : grep-visible,
relue dans un diff, un nom par ligne. C'est ce qui remplace la garde, et c'est plus
fort qu'elle.

`MIKA_ARCH_DISABLED_TOOLS` (`well_known_agents.rs:438`), en revanche, est
**conservée** — trois raisons indépendantes, chacune suffisante : le chemin
fail-closed la lit ; la garde C1 (`mika2593_chaque_lecture_orpheline_a_sa_famille_decriture_hors_datteinte`,
`:2296`) la lit ; et son doc-comment est le **record de pourquoi** neuf lectures ont
été coupées. Supprimer la dénylist effacerait le raisonnement de mika#2593.

## Décision 6 — la nouvelle garde, et le nouveau mode de défaillance qu'elle couvre

**Le basculement crée une classe de défaillance que la dénylist n'avait pas**, et
c'est le risque n° 1 de ce vol : une faute de frappe dans une **dénylist** *garde*
silencieusement un outil (dégradation visible, outil en trop) ; une faute de frappe
dans une **allowlist** *retire* silencieusement un outil (mika-arch perd un outil
constitutif, et rien ne le dit). Le sens de l'erreur s'inverse.

Nouvelle garde `mika2748_lallowlist_de_mika_arch_est_le_roster`, dans le `mod tests`
de `well_known_agents.rs` (à côté de la constante qu'elle garde, pas dans
`tools/mod.rs` : `BUILTIN_TOOL_NAMES` est `pub`, la localité est meilleure). Trois
assertions, et aucune n'est redondante avec une autre :

- **A — anti-typo :** `MIKA_ARCH_ALLOWED_TOOLS ⊆ BUILTIN_TOOL_NAMES`
  (`tools/mod.rs:1333`, surface builtin déclarée et déjà testée en parité). C'est
  l'oracle qui transforme la faute de frappe silencieuse en rouge. Vérifié :
  les 19 noms, `list_agents` compris (`:1376`), y sont présents aujourd'hui.
- **B — les outils constitutifs sont *présents* :**
  `MIKA_ARCH_AGENT_SELF_STATE_TOOLS ∪ MIKA_ARCH_C3_PROTECTED ⊆ MIKA_ARCH_ALLOWED_TOOLS`.
  **C'est le changement de sens que le ticket demande**, et la raison pour laquelle
  `test_mika_arch_disabled_tools_excludes_agent_self_state` (#811, `:2153`) survit :
  son contenu ne change pas (3 noms), seul le sens de l'appartenance s'inverse —
  *absent de la dénylist* devient *présent dans l'allowlist*. Même chose pour
  `MIKA_ARCH_C3_PROTECTED` (`:2170`, 10 noms). Les deux gardes existantes restent et
  restent vertes ; B est leur contrepartie sur l'autre forme.
- **C — pas de décision écrite deux fois :**
  `MIKA_ARCH_ALLOWED_TOOLS ∩ MIKA_ARCH_DISABLED_TOOLS = ∅`.

Les trois sont **bidirectionnelles là où cela a un sens** (A et B sont des
inclusions orientées par nature ; chaque message nomme le ou les noms fautifs, pas
seulement le fait d'un écart).

**Vérifié à la naissance, les trois sont à zéro violation :** les 3 noms de
`MIKA_ARCH_AGENT_SELF_STATE_TOOLS` et les 10 de `MIKA_ARCH_C3_PROTECTED` figurent
tous dans le roster de 18 ; l'intersection avec la dénylist est vide par
construction (l'allowlist *est* le complément). D'où l'option (a) sans exception —
§ Fire-Disposition.

## Livrables

### D1 — `prompt.rs` : le schéma

Champ `allowed: Option<Vec<String>>` sur `ToolsIdentityConfig`, avec `#[serde(default)]`
(donc `None` quand la section ne le porte pas). Doc-comment : les trois états, et le
renvoi à la borne dure (builtins uniquement, les outils de skill et MCP s'ajoutent
après le filtre). Le doc-comment existant de `ToolsIdentityConfig` (`:597–609`) invite
déjà ce champ frère ; il est mis à jour pour décrire ce qui existe au lieu de ce qui
viendra.

`fail_closed_identity()` : `allowed: None` + commentaire nommant le coût (Décision 4).

### D2 — `agent_loop/mod.rs` : le prédicat et son observabilité

- `apply_agent_tool_visibility` : signature + prédicat (Décision 1), doc-comment
  mis à jour — l'invitation à la migration est **remplacée** par la description des
  deux formes et par la borne dure.
- Nouveau champ sur la ligne `applied identity tool-visibility filter` :
  `filter_shape` ∈ `{"denylist", "allowlist", "both"}`. **C'est ce qui rend le
  basculement observable sans changement numérique** (mika#2293 : un réglage qu'on
  ne peut pas observer n'est pas un réglage) : `hidden_count`, `kept_count`,
  `kept_bytes` ne bougent pas — c'est l'attendu — donc sans ce champ, « le
  basculement est servi » et « le binaire est l'ancien » rendent des octets
  identiques (classe mika#2205 / mika#2340). `allowed_size` est émis à côté de
  `disabled_size`, **absent** (et non `0`) quand `allowed` est `None` : absent ≠
  zéro (mika#2331).
- Déclenchement du log : la condition `removed > 0` est conservée. Un agent à
  allowlist retirant zéro outil est possible (allowlist = registre entier) mais pas
  le cas servi ; le laisser muet reste cohérent avec l'existant.

### D3 — `agent_loop/mod.rs` : le câblage

`inject_skills_and_resolve_tools` prend `allowed: Option<&[String]>` et le passe au
hook, sans déplacer le point d'application. Trois sites de production threadent
`ctx.identity.tools.allowed.as_deref()` : conversation (`:5535`, à côté de
`effective_disabled_tools`), silencieux (`:6842`), équipe (`:7651`). Cinq sites
d'appel de test inline reçoivent `None` (`:12811`, `:12824`, `:12840`, `:12878`,
`:16997`).

Rejeté : une structure `ToolVisibility<'_>` regroupant les deux. Plus de lignes pour
le même trois-états, et elle déplacerait la décision hors du site qui la lit.

### D4 — `well_known_agents.rs` : la constante

`MIKA_ARCH_ALLOWED_TOOLS` (19 noms, un par ligne, Décision 2) avec son doc-comment :
d'où viennent les 19, pourquoi `list_agents` y est, et la borne dure.

### D5 — `well_known_agents.rs` : l'émission

`build_mika_arch_identity` (`:502`) rend aujourd'hui `[tools]\ndisabled = [...]`.
Il rend désormais **les deux listes** dans la même section, sur le modèle de la
boucle existante.

### D6 — `well_known_agents.rs` : `tools.allowed` code-owned

Une entrée dans `CODE_OWNED_IDENTITY_SECTIONS` (`:632`) + une ligne de commentaire
renvoyant à la classe mika#1833 / mika#2330 (Décision 3).

### D7 — la garde

`mika2748_lallowlist_de_mika_arch_est_le_roster` (3 assertions, Décision 6) et
retrait de `MIKA_ARCH_SERVED_BUILTINS_FROM_DEFAULT_TOOLS` + de
`mika2593_le_tableau_servi_a_mika_arch_est_un_roster_atteste` (`tools/mod.rs`), le
doc-comment de la nouvelle garde nommant mika#2593 et ce qui l'a remplacée
(Décision 5).

### D8 — tests du prédicat (inline, `agent_loop/mod.rs`)

Un test **tabulaire**, 5 cas, sur `apply_agent_tool_visibility` directement :

| cas | `disabled` | `allowed` | attendu |
|---|---|---|---|
| 1 | `[]` | `None` | retour anticipé, tableau intact |
| 2 | `["b"]` | `None` | dénylist seule (comportement actuel) |
| 3 | `[]` | `Some(["a"])` | allowlist seule — `b`, `c` retirés |
| 4 | `["a"]` | `Some(["a","b"])` | intersection — seul `b` reste (`a` écrit des deux côtés perd) |
| 5 | `[]` | `Some([])` | tout retiré — le trois-états, distinct du cas 1 |

Le cas 5 est le **contrôle négatif du trois-états** : c'est lui qui redevient rouge
si quelqu'un « simplifie » `Option<Vec<String>>` en `Vec<String>`.

### D9 — test de réconciliation (inline, `well_known_agents.rs`)

Extension de `test_reconcile_adds_mika_arch_missing_context_summary` (`:3859`), qui
pré-ensemence déjà mika-arch avec `[tools]\ndisabled = []` puis réconcilie : une
assertion de plus que `tools.allowed` est **présent et égal au roster** après
réconciliation. **C'est le test de la classe mika#1833** — un agent déjà provisionné
reçoit bien la déclaration — et il coûte 8 lignes au lieu d'un fichier parce que le
scénario exact existait déjà.

Plus, sur `test_mika_arch_identity_toml_has_allowlist_and_disabled_tools` (`:3456`),
une assertion `identity.tools.allowed` de longueur `MIKA_ARCH_ALLOWED_TOOLS.len()` —
fidélité de l'émission D5, pas préservation de valeur.

### D10 — test comportemental (`tests/eval/test_mika_arch_tool_allowlist_2748.rs`)

Ce que ni D8 ni D9 ne peuvent voir : **le câblage**. Un test qui vérifie le prédicat
et un test qui vérifie le chargement TOML restent tous deux verts si les trois sites
de production ne passent jamais le champ. Il faut un tour réel.

`EvalHarness` + `MockLlmProvider`, un agent à allowlist, et sur le tableau d'outils
capturé : un builtin **hors** allowlist est absent, un builtin **dans** l'allowlist
est présent. Les deux assertions dans le même bloc, une seule construction de
harnais. Enregistrement : une ligne `mod test_mika_arch_tool_allowlist_2748;` dans
`crates/mika-agent/tests/eval.rs`.

### D11 — docs

- `docs/agent-internals/skills-system.md:28` — la phrase *« Future migration: extend
  to support `[tools].allowlist` … »* devient la description de ce qui existe.
  Fichier **absent** de la liste `DOCS` de `scripts/sync-agent-docs.sh` : éditable
  sans lancer le script.
- `docs/mcp.md:55,319` — le caveat « `[tools].disabled` ne peut pas refuser un outil
  MCP » gagne sa moitié allowlist (« ni `[tools].allowed` les autoriser »). Fichier
  également absent de `DOCS`.
- `docs/architecture.md:430` — même caveat. **Ce fichier EST dans `DOCS`** :
  l'éditer oblige à lancer `scripts/sync-agent-docs.sh` dans le même commit, sinon
  le job CI `docs-sync` rougit.
- `crates/mika-agent/CLAUDE.md` — hors comptage (règle mika#2636). La section
  `[tools]` du § Tools renvoie à `docs/agent-internals/tools.md` ; c'est là que la
  nouvelle forme se documente, pas dans le `CLAUDE.md`.

## Fire-Disposition

Ce plan livre des détecteurs : la garde `mika2748_lallowlist_de_mika_arch_est_le_roster`
(D7, 3 assertions) et les tests de D8/D9/D10 dont le chemin de succès est « aucune
violation trouvée ».

**Option retenue : (a) — exception nommée en allowlist, avec une liste d'exceptions
VIDE et assertée vide.**

Les trois assertions de D7 ont été vérifiées à **zéro violation** sur l'état actuel
du dépôt avant d'être planifiées (§ Décision 6, dernier paragraphe) :

| assertion | état constaté | source |
|---|---|---|
| A — `⊆ BUILTIN_TOOL_NAMES` | 19/19 présents, `list_agents` compris | `tools/mod.rs:1333`, `:1376` |
| B — constitutifs présents | 3/3 self-state + 10/10 C3-protected dans le roster de 18 | `well_known_agents.rs:2149`, `:2170`, `tools/mod.rs:1683` |
| C — intersection vide | vide par construction (l'allowlist est le complément) | § Décision 2 |

Implémentation de l'exception, sur le modèle de `MIKA2593_CRITERION_EXCEPTIONS`
(`well_known_agents.rs:2278`) :

```rust
/// Exceptions nommées à `mika2748_lallowlist_de_mika_arch_est_le_roster`.
/// Chaque entrée : le nom d'outil précis, le ticket de suivi, la raison.
/// VIDE à la naissance — les trois assertions passent sur l'état mergé de mika#2593.
const MIKA2748_ALLOWLIST_EXCEPTIONS: &[(&str, &str)] = &[];
```

Plus l'**assertion auto-nettoyante** (même forme que celle de mika#2593, qui asserte
sa propre liste vide) :

```rust
assert!(
    MIKA2748_ALLOWLIST_EXCEPTIONS.is_empty(),
    "exception stale : la liste est vide à la naissance ; si une entrée a été \
     ajoutée, elle doit être retirée dès que son ticket de suivi est clos, et \
     cette assertion est ce qui le rappelle"
);
```

La garde atterrit donc **armée**, sans `#[ignore]`, sans halte. Les trois assertions
sont vraies au moment du merge ; l'exception existe pour que la prochaine violation
légitime ait une porte grep-visible au lieu d'un affaiblissement du prédicat.

## Contrat de vérification

### Pré-merge (CI)

1. `cargo build` — le littéral de `fail_closed_identity()` force l'édition D1 ; une
   omission ne compile pas.
2. `cargo test -p mika-agent` — y compris :
   - `mika2748_lallowlist_de_mika_arch_est_le_roster` (D7, 3 assertions + exception
     vide)
   - la table du prédicat (D8, 5 cas dont le contrôle négatif du trois-états)
   - `test_reconcile_adds_mika_arch_missing_context_summary` étendu (D9 — classe
     mika#1833)
   - `test_mika_arch_identity_toml_has_allowlist_and_disabled_tools` (D9, fidélité
     d'émission)
   - **inchangés et devant rester verts** :
     `test_mika_arch_disabled_tools_excludes_agent_self_state` (#811),
     `mika2027_absent_identity_toml_fails_closed`,
     `mika2593_chaque_lecture_orpheline_a_sa_famille_decriture_hors_datteinte`
     (garde C1),
     `test_no_code_owned_drift_outside_constant`,
     `test_code_owned_sections_have_reconciler_coverage`
3. `cargo test -p mika-agent --test eval` — D10.
4. `cargo clippy` + `cargo fmt --check`.
5. `make verify-bundled-skills` — aucun bundle touché, mais le gate tourne sur
   chaque PR.
6. `scripts/verify-pipeline.sh origin/main`.
7. Job CI `docs-sync` — vert **uniquement** si `scripts/sync-agent-docs.sh` a été
   lancé dans le même commit que l'édition de `docs/architecture.md` (D11).

**Contrôle de bonne foi sur D7** : avant de livrer, introduire temporairement un nom
inexistant (`"nonexistent_tool"`) dans `MIKA_ARCH_ALLOWED_TOOLS` et vérifier que
l'assertion A rougit en le **nommant** ; puis retirer `send_message` et vérifier que
B rougit en le nommant. Une garde qu'on n'a pas vue rougir n'est pas une garde.

### Post-déploiement (après `make deploy`)

L'arbitre de ce vol est la **préservation**, donc les deux sondes de mika#2593 §6
rendent les **mêmes nombres** qu'après mika#2593 :

**P2 — le filtre**

```bash
grep "tool-visibility" "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.agent == "mika-arch") | {hidden_count, kept_count, kept_bytes, filter_shape, allowed_size}'
```

| champ | attendu |
|---|---|
| `hidden_count` | **32**, inchangé |
| `kept_count` | **19**, inchangé |
| `kept_bytes` | **inchangé** au sens strict (égalité, pas « en baisse ») |
| `filter_shape` | `"denylist"` → **`"both"`** |
| `allowed_size` | **19** |

`filter_shape` est le seul champ qui bouge, et c'est tout l'intérêt : il est la
preuve que le binaire servi est bien le nouveau. **Son absence signifie un binaire
antérieur à ce vol** (classe mika#2340), jamais « pas d'allowlist » — pour cela, lire
`allowed_size`, qui est *absent* et non `0` quand `allowed` est `None` (mika#2331).

**P3 — l'arbitre**

```bash
grep context_window_assembled "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.agent_id == "mika-arch") | {tool_defs_bytes}' | sort | uniq -c
```

Attendu : **une seule valeur, identique à celle d'après mika#2593**. Une valeur
différente signifie que le roster transcrit n'est pas le roster servi — c'est-à-dire
qu'un nom a été perdu ou gagné à la transcription, le mode de défaillance n° 1 de ce
vol (§ Décision 6). La **constance** reste la moitié de l'attendu : une valeur qui
varierait d'un tour à l'autre dirait que le filtre dépend d'une entrée, ce qu'il ne
doit pas.

**P4 — la réconciliation a bien atteint l'agent déjà provisionné**

```bash
grep -A3 '^\[tools\]' ~/.mika/agents/mika-arch/identity.toml
```

Attendu : `allowed = [...]` présent, 19 noms. Son **absence** sur le disque après un
redémarrage signifie que `tools.allowed` n'a pas pris effet comme code-owned — la
classe mika#1833, et la raison d'être de la Décision 3.

**P5 — mika-arch garde `gh_read`**

Un tour de revue réel (`mika-arch-groom-ticket` sur n'importe quel plan) doit
appeler `gh_read` sans erreur. C'est la vérification de la borne dure : l'allowlist
ne gouverne que les builtins.

## Ordre de commit

Chaque étape compile et la suite est verte à chaque palier.

1. **D1** — champ `allowed` + `fail_closed_identity`. Le schéma accepte la forme ;
   rien ne la produit encore, rien ne la lit. Suite verte.
2. **D2 + D3** — prédicat, `filter_shape`, câblage des 3 + 5 sites. Le moteur sait
   appliquer une allowlist ; aucun agent n'en déclare. Comportement identique. Suite
   verte.
3. **D8** — table du prédicat. Le moteur est épinglé avant d'avoir un client.
4. **D4 + D5 + D6** — constante, émission, code-owned. `test_no_code_owned_drift_outside_constant`
   garantit que D5 sans D6 rougit : **les trois doivent tenir dans ce commit**.
5. **D7** — nouvelle garde, retrait de la garde mika#2593 et de sa constante.
6. **D9 + D10** — réconciliation et tour réel.
7. **D11** — docs + `scripts/sync-agent-docs.sh` dans le **même** commit.

L'ordre n'est pas interchangeable sur un point : **D4/D5/D6 forment un palier
indivisible.** Émettre `tools.allowed` (D5) sans le déclarer code-owned (D6) laisse
un commit rouge ; le déclarer sans l'émettre fait rougir
`test_code_owned_sections_have_reconciler_coverage`.

## Hors périmètre

- **Le registre global par agent** (§ 8 (e) de mika#2593) — supprimerait le filtre
  de présentation au profit d'une construction ciblée. Beaucoup plus gros, et pas
  nécessaire : le filtre est déjà le bon point d'application, le modèle ne voit
  jamais l'outil (#811).
- **`system_prompt_bytes` sur `context_window_assembled`** (§ 8 (b)) — défaut de
  mesure, pas d'arbitrage. À ouvrir séparément.
- **Rouvrir `list_agents`** (§ 8 (c)) — conservé délibérément par mika#2593 ; le
  retirer demande de trancher si le contrat d'auto-connaissance de mika#740 s'appuie
  dessus. Une question sur #740, pas sur la taille. Ici, `list_agents` est **dans**
  l'allowlist, ce qui préserve le statu quo exactement.
- **Le recensement par outil de `tool_defs_bytes`** (§ 8 (d)) — ni préalable ni
  livrable, § Résolution de la précondition. `Tracked in: senara-solutions/mika#2593`
  (un ticket dédié sera ouvert en fin de vol, DoD 9).
- **Durcir le chemin fail-closed à une allowlist** — décision de flotte, § Décision 4.
  Ticket de suivi en fin de vol (DoD 9).
- **Basculer mika-dev, mika-qa, mika-relay, mika** — ce vol livre le **mécanisme**
  et l'applique au seul agent dont le roster est attesté (mika-arch, par mika#2593).
  Les autres agents n'ont pas de roster attesté ; leur basculer les outils
  demanderait de refaire le travail de mika#2593 pour chacun. Suivi à ouvrir si
  l'opérateur le veut.
- **Les outils de skill et MCP** — borne dure du point d'application, § Décision 1.

## Risques

| risque | gravité | mitigation |
|---|---|---|
| **Un nom perdu à la transcription** retire silencieusement un outil à mika-arch (inversion du sens de l'erreur, § Décision 6) | élevée | assertion A (anti-typo) + B (constitutifs présents) + sonde P3 (`tool_defs_bytes` strictement inchangé) + contrôle de bonne foi vu rougir |
| `tools.allowed` **omis** de `CODE_OWNED_IDENTITY_SECTIONS` ⇒ déclaration inerte (classe mika#1833) | élevée | `test_no_code_owned_drift_outside_constant` rougit **automatiquement** ; D9 le vérifie comportementalement ; sonde P4 sur le disque |
| Le câblage oublie un des 3 sites de production ⇒ un chemin (silencieux ou équipe) ignore l'allowlist, suite verte | moyenne | D10 (tour réel) ; le trois-états `Option` ne permet pas d'« oublier » sans que le compilateur réclame un argument |
| `Option<Vec<String>>` « simplifié » en `Vec<String>` par un futur éditeur ⇒ `Some([])` devient inexprimable | moyenne | cas 5 de D8 est le contrôle négatif exact |
| L'allowlist écrite trop large (y mettre `gh_read`) ⇒ entrée inerte, qui *lira* comme une autorisation | moyenne | borne dure écrite dans le doc-comment de D1, D2 et D4 ; assertion A la refuse si le nom n'est pas un builtin |
| Perte du raisonnement de mika#2593 avec la dénylist | faible | `MIKA_ARCH_DISABLED_TOOLS` et son doc-comment sont **conservés**, § Décision 5 |
| `docs/architecture.md` édité sans lancer le script de sync ⇒ CI `docs-sync` rouge | faible | D11 et l'étape 7 de l'ordre de commit le disent explicitement |
| `tool_defs_bytes` hors de la bande 15,6–17,6 Ko de mika#2593 | faible | ne concerne pas ce vol : c'est l'estimateur de mika#2593, § Résolution de la précondition |

## Definition of Done

1. `ToolsIdentityConfig` porte `allowed: Option<Vec<String>>` avec `#[serde(default)]`,
   et son doc-comment décrit les trois états et la borne dure (builtins uniquement).
2. `apply_agent_tool_visibility` applique le prédicat de la Décision 1, garde un seul
   site d'appel par chemin, et son doc-comment ne porte plus d'invitation à migrer
   mais la description des deux formes.
3. La ligne `applied identity tool-visibility filter` porte `filter_shape` et
   `allowed_size`, `allowed_size` **absent** (non `0`) quand `allowed` est `None`.
4. Les 3 sites de production et les 5 sites de test inline sont câblés ;
   `effective_disabled_tools` n'est **pas** modifié.
5. `MIKA_ARCH_ALLOWED_TOOLS` existe avec les 19 noms, est émise par
   `build_mika_arch_identity`, et `tools.allowed` figure dans
   `CODE_OWNED_IDENTITY_SECTIONS`.
6. `mika2748_lallowlist_de_mika_arch_est_le_roster` est armée (3 assertions), sa
   liste d'exceptions est vide et assertée vide, et chacune des 3 assertions a été
   **vue rougir** sur une violation introduite exprès puis retirée.
7. `MIKA_ARCH_SERVED_BUILTINS_FROM_DEFAULT_TOOLS` et
   `mika2593_le_tableau_servi_a_mika_arch_est_un_roster_atteste` sont retirées, et le
   doc-comment de la nouvelle garde nomme mika#2593 et ce qui l'a remplacée.
   `MIKA_ARCH_DISABLED_TOOLS` est **conservée**.
8. Tout le § Contrat de vérification pré-merge est vert, `docs-sync` compris.
9. **Deux tickets de suivi ouverts en fin de vol**, et nommés dans le corps de la PR
   par des lignes `Tracked in:` : (i) le recensement par outil de `tool_defs_bytes`
   (§ 8 (d) de mika#2593) ; (ii) durcir le chemin fail-closed à une allowlist
   (§ Décision 4).
10. Le corps de la PR porte les sondes P2–P5 **telles qu'écrites ici**, pour que
    l'opérateur les lance après `make deploy` — l'instrument n'est pas vivant avant
    le déploiement.

## Acceptance criteria

Le corps du ticket mika#2748 ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de son § *Périmètre proposé*, de son § *Hors
périmètre*, de sa § *Précondition* et du § Contrat de vérification de ce plan.

- **AC1** — `Identity.tools` porte un champ `allowed: Option<Vec<String>>` à côté de
  `disabled`, et un `identity.toml` sans ce champ se désérialise en `None` sans
  erreur (rétro-compatibilité).
- **AC2** — Le prédicat d'`apply_agent_tool_visibility` traite les **deux** formes au
  **même** site d'appel, sans changement d'appelant autre que l'ajout de l'argument —
  c'est la promesse littérale de son doc-comment. Les 5 cas de D8, dont `Some([])`
  distinct de `None`, passent.
- **AC3** — `tools.allowed` est **code-owned** : il figure dans
  `CODE_OWNED_IDENTITY_SECTIONS`, `test_code_owned_sections_have_reconciler_coverage`
  et `test_no_code_owned_drift_outside_constant` sont verts, et un mika-arch **déjà
  provisionné** reçoit `tools.allowed` à la réconciliation (D9 + sonde P4) — la classe
  mika#1833 / mika#2330 est fermée, pas seulement évitée.
- **AC4** — `mika2593_le_tableau_servi_a_mika_arch_est_un_roster_atteste` est
  remplacée par l'assertion que l'allowlist **est** le roster. Aucune garde permanente
  ne recalcule `allowlist == registre − dénylist` (§ Décision 5).
- **AC5** — `test_mika_arch_disabled_tools_excludes_agent_self_state` (#811) et
  `MIKA_ARCH_C3_PROTECTED` **survivent inchangés**, et leur contrepartie sur l'autre
  forme (assertion B : les 3 + 10 noms sont **présents** dans l'allowlist) est verte.
  Le sens de l'appartenance s'est inversé ; le contenu, non.
- **AC6** — `tool_defs_bytes` pour mika-arch est **strictement inchangé** par rapport
  à sa valeur post-mika#2593, et constant d'un tour à l'autre (sonde P3). `hidden_count`
  reste 32 et `kept_count` reste 19 (sonde P2).
- **AC7** — Le basculement est **observable** : `filter_shape` passe de `"denylist"` à
  `"both"` sur la ligne du filtre, et son absence se lit comme un binaire antérieur,
  jamais comme une absence d'allowlist.
- **AC8** — mika-arch conserve `gh_read` et ses autres outils de skill : l'allowlist
  ne gouverne que les builtins, et cette borne est écrite au code, non seulement au
  plan (sonde P5).
- **AC9** — Rien du § *Hors périmètre* n'est touché : pas de registre par agent, pas
  de `system_prompt_bytes`, `list_agents` reste servi, aucun autre agent bien connu
  n'est basculé.
- **AC10** — La question ouverte du ticket est **répondue dans le plan et dans la
  PR** : le recensement par outil de `tool_defs_bytes` n'est ni préalable ni livré
  ici, et la raison est que ce vol transcrit un roster attesté au lieu d'arbitrer nom
  par nom.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/prompt.rs` (D1 — champ `allowed` + `fail_closed_identity`) | 23 |
| `crates/mika-agent/src/agent_loop/mod.rs` (D2 + D3 — prédicat, `filter_shape`, câblage 3 + 5 sites) | 52 |
| `crates/mika-agent/src/well_known_agents.rs` (D4 + D5 + D6 + D7 — constante, émission, code-owned, garde) | 93 |
| `crates/mika-agent/src/tools/mod.rs` (D7 — retrait du roster mika#2593 et de sa garde) | 0 |
| tests inline (D8 table du prédicat, D9 réconciliation + fidélité d'émission) | 47 |
| `crates/mika-agent/tests/eval/test_mika_arch_tool_allowlist_2748.rs` + enregistrement (D10) | 40 |

Total estimé : 255 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 255 < 279, et sous le seuil par PR de
1000. Le découpage a été examiné et **rejeté sur le fond, pas sur la marge** : la
coupure naturelle serait « vol 1 = le mécanisme (D1–D3, D8), vol 2 = le client
(D4–D7, D9, D10) ». Le vol 1 livrerait alors un prédicat qu'aucun agent n'exerce —
du code mort, mergé, avec une suite verte par-dessus : exactement la classe
mika#1833 que la Décision 3 passe son temps à refuser. Et le vol 2 serait trivial. On
ne découpe pas un vol en une moitié inerte et une moitié décorative.

Les 0 lignes de `tools/mod.rs` sont une **suppression** (constante de 18 noms + garde
+ doc-comments, ≈ 100 lignes retirées du fichier) : rien d'écrit, donc rien de compté.
La garde de D7 est **nouvelle** (45 des 93 lignes de `well_known_agents.rs`), pas un
amendement de celle de mika#2593, parce qu'elle change de fichier et de prédicat ;
seules ses 3 assertions coûtent, les deux gardes existantes (#811, C3-protected) ne
sont pas touchées.

**Soupape, si l'implémenteur franchit la borne.** Les deux premiers postes où couper
sont D11 (docs — hors comptage, mais pas hors tours : `scripts/sync-agent-docs.sh`
coûte un cycle CI complet) et D10 (le test comportemental). Dans cet ordre inverse de
valeur : **ne pas compresser D10**, c'est le seul livrable qui voit le câblage. Si la
borne mord, **s'arrêter et ouvrir le suivi** — livrer D1→D9 est mergeable seul (le
mécanisme est câblé, l'allowlist est servie, la réconciliation est épinglée) et D10
+ D11 partent dans un vol 2 nommé par une ligne `Tracked in:` dans le corps de la PR.
Ne pas compresser les tests pour tenir : c'est ce qui a tué mika#2161 et mika#2633.
