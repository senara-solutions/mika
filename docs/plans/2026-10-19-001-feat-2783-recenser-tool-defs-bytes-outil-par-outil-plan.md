# mika#2783 — recenser `tool_defs_bytes` outil par outil : une carte `{nom: octets}` sur une ligne qui grepe seule

> - **Ticket :** senara-solutions/mika#2783
> - **Labels :** `p2-normal`, `ready`, `dispatch:loop`, `loop-substrate`
> - **Suivi de :** `docs/plans/2026-10-16-005-feat-2593-peser-tool-defs-bytes-arch-plan.md` § 8 (d)

---

## 1. Contexte, et ce que le code dit exactement

`context_window_assembled` porte `tool_defs_bytes` : **un total**, calculé par
`tool_defs_byte_weight` (`agent_loop/mod.rs:8889`) comme
`serde_json::to_string(tool_defs).len()` sur le tableau **entier**. On sait ce que
le tableau coûte ; on ne sait pas ce que coûte un nom.

mika#2593 a dû transporter sa proportion depuis la source — 4 252 o pour neuf
définitions, dont 899 pour `promote_deferred_callback` — et l'a dit comme une
**prédiction**, l'arbitre restant le total après déploiement. Ce ticket livre
l'instrument qui aurait rendu ce transport inutile.

### 1.1 Trois faits de code que le corps du ticket ne porte pas, et qui tranchent

**(a) Le basculement de mika#2748 n'a PAS atterri.** Le corps de mika#2783 l'écrit
au passé (« qui a basculé les outils de mika-arch vers une allowlist ») ; la PR
#2781 est **ouverte** au 2026-10-19, et `grep -rn "ALLOWED_TOOLS" --include="*.rs"`
ne retourne rien. Conséquence pour ce vol, et elle est bonne : **le recensement
est indifférent à la forme du filtre**. Il lit le tableau *servi*, après
`apply_agent_tool_visibility`, après le filtre compact-provider, après l'ajout des
outils de skill, après les outils MCP et après `filter_ci_excluded_tools` — donc
après la dénylist aujourd'hui et après l'allowlist demain, sans une ligne à
changer. Ce plan ne dépend pas de #2781 et ne la bloque pas ; leur seul point de
contact est le fichier `agent_loop/mod.rs`, à ~3 300 lignes de distance.

**(b) Le tableau servi N'EST PAS constant pour un agent.** `skill_tool_defs` varie
d'un tour à l'autre : les outils de skill dépendent des skills *matchés* par le
message (`inject_skills_and_resolve_tools`, `mod.rs:9195-9230`), la dénylist est
élargie sur un tour Webhook Fallthrough (`effective_disabled_tools`, mika#2517),
les outils MCP dépendent des serveurs connectés, et `filter_ci_excluded_tools`
retire des noms sur un préfixe CI. **C'est ce fait qui tranche la question
principale du ticket** (§ 2.2) : une émission conditionnée à la première assemblée
d'un agent atteste un roster qui n'est pas celui des tours suivants.

**(c) La somme des octets par outil ne peut pas égaler `tool_defs_bytes`
nue — il y a un cadrage JSON, et il est exactement calculable.** `serde_json`
sérialise un tableau en compact, sans espace : `[` + `e₁` + `,` + `e₂` + … + `]`.
Donc, pour `n` définitions :

```
tool_defs_bytes  ==  Σᵢ to_string(defᵢ).len()  +  2  +  max(n − 1, 0)
                     └──── per_tool_sum ────┘     └─ framing_bytes ─┘
```

La sérialisation d'un élément est identique en isolé et dans le tableau
(`serde_json` compact est sans contexte ; `ToolDefinition` est une struct à trois
champs dérivée, `input_schema` est une `Value` dont l'ordre de clés est stable
d'une sérialisation à l'autre). **Le ticket demande d'assertir cette égalité, pas
de la supposer** — c'est exactement ce que fait le § 4.3, et le terme de cadrage
est ce que la garde doit connaître pour ne pas rougir à tort sur un écart qui est
en réalité une virgule.

---

## 2. La direction retenue

### 2.1 Un événement DISTINCT, au même site, sur le même tour

`tool_defs_census` — INFO, `target: "mika::otel"`, émis **immédiatement après**
`emit_context_window_assembled`, au **site de production unique** où une fenêtre
de conversation est assemblée (`mod.rs:5715`), sur **le même `&skill_tool_defs`**.

Il porte quatre champs numériques et une carte :

| champ | valeur |
|---|---|
| `tool_defs_bytes` | le total, **par `tool_defs_byte_weight`** — l'expression unique déjà derrière `context_window_assembled.tool_defs_bytes` et `kept_bytes` |
| `tool_defs_count` | `n`, le nombre de définitions servies |
| `per_tool_sum` | `Σᵢ to_string(defᵢ).len()` |
| `per_tool_bytes` | la carte `{nom: octets}`, JSON, clés ordonnées |

plus `agent_id`, `session_id`, `trace_id`, `mode` — les quatre axes de corrélation
que portent déjà ses deux voisins.

**Pourquoi un nom distinct plutôt qu'un champ de plus sur
`context_window_assembled`.** Le motif est celui que mika#2474 a écrit pour
`brief_size_overrun` et mika#2293 pour `llm_budget_resolved` : *il doit greper
seul*. La carte pèse ~1,4 Ko pour l'agent au roster le plus large, soit un
décuplement de la ligne `context_window_assembled` que lisent cinq procédures
d'opérateur qui n'ont que faire du détail. Un nom à part leur laisse leur ligne
intacte, et donne au recensement le `grep` qui précède le `jq`.

**Pourquoi la ligne reste self-suffisante.** `tool_defs_bytes` et
`tool_defs_count` voyagent **sur la ligne du recensement**, pas seulement sur
celle du voisin. Sans eux, vérifier l'égalité du § 1.1 (c) demanderait une
jointure sur `trace_id` — et l'égalité est le seul moyen de savoir que les deux
instruments mesurent **une** population et non deux sous un nom (classe
mika#2205). Ce n'est pas une seconde source de vérité : c'est la même valeur, au
même instant, sur la ligne qui en a besoin — l'arbitrage déjà rendu pour
`system_prompt_bytes` sur `brief_size_overrun`.

### 2.2 Pourquoi pas les deux autres formes, que le ticket nomme

**Une ligne par outil par tour.** ≈ 50 lignes par tour par agent, pour un contenu
dont chaque ligne est inutilisable seule (le sens d'un octet est sa part du
total). L'agrégation redeviendrait un `group by trace_id` sur 19 Go. Refusée sur
le volume **et** sur l'absence d'unité de lecture.

**Une émission conditionnée à la première assemblée d'un agent.** Refusée par le
fait § 1.1 (b) : le tableau servi varie d'un tour à l'autre, donc la première
assemblée atteste un roster que les suivantes contredisent — et l'opérateur qui
pèse un nom lirait une photo périmée sans rien pour le lui dire. Un gate « émettre
quand le roster change » ferait pire : il exigerait une mémoire par agent, perdue
au redémarrage, et **l'absence de ligne signifierait alors « inchangé »**, ce qui
entre en collision frontale avec la lecture mika#2340 que le ticket impose
(absence totale = binaire antérieur au correctif). L'émission inconditionnelle est
ce qui garde l'absence univoque.

**Le levier, s'il faut un jour réduire le volume :** c'est précisément le nom
distinct. Un filtre de journalisation peut faire taire `tool_defs_census` sans
toucher `context_window_assembled`. Avec un champ de plus sur le voisin, il
n'existerait aucun levier de ce genre.

### 2.3 Précédent de forme, et il est à une fonction de distance

`emit_system_prompt_assembled` (`mod.rs:9247`) porte déjà
`per_skill_bytes = per_skill_json.as_deref().unwrap_or("{}")` : une carte
`{nom: octets}` sérialisée dans **un** champ d'**un** événement, pour exactement la
même question posée sur les skills. Ce plan ne propose pas une forme, il applique
celle que ce fichier a déjà choisie pour l'autre moitié du brief.

### 2.4 Absent n'est jamais zéro — et à trois niveaux distincts

Le ticket pose la règle ; voici ses trois lectures, qui ne sont pas la même :

| ce qui est absent | ce que ça veut dire | comment c'est tenu |
|---|---|---|
| **une clé** de `per_tool_bytes` | cet outil **n'a pas été servi** à cet agent sur ce tour | propriété de la carte : un outil absent du tableau n'a pas d'entrée. Jamais `0` (mika#2331) |
| **la ligne entière** | le binaire servi **précède** ce correctif | l'émission est inconditionnelle et non gatée (mika#2340) |
| **le champ `per_tool_bytes`** | n'arrive jamais | il vaut `"{}"` au pire — un tableau vide est `{}` avec `tool_defs_count = 0`, jamais une ligne muette |

Le couple `(per_tool_bytes = "{}", tool_defs_count = n)` distingue donc un tableau
réellement vide (`n = 0`) d'un échec de sérialisation (`n > 0`) — et ce dernier
porte en plus son propre WARN (§ 3.3).

### 2.5 Portée : le mode conversation, et c'est le site de la grandeur

`emit_context_window_assembled` a **un** appelant (`mod.rs:5715`, mode
`conversation`) ; les deux autres appels d'`inject_skills_and_resolve_tools`
(silent `mod.rs:6834`, team `mod.rs:7643`) ne construisent pas de
`ContextWindowFields` et ne calculent donc pas `tool_defs_bytes`. Le ticket dit
« émis au point d'assemblage du tableau, là où `tool_defs_bytes` est déjà
calculé » : c'est ce site-là, et un seul. Le champ `mode` voyage quand même, de
sorte qu'une extension future aux deux autres modes n'ait pas à changer le
schéma de la ligne. Les passes architecte, population motrice du ticket, sont des
tours de conversation (`_arch_ask` → `mika ask`) : la portée couvre le besoin.

---

## 3. Le mécanisme, en quatre pièces

### 3.1 La couche pure : `tool_defs_census`

```rust
struct ToolDefsCensus {
    /// Ordonnée — donc la ligne de journal est stable et diffable d'un tour à
    /// l'autre, ce qu'un `HashMap` ne garantit pas.
    per_tool: BTreeMap<String, usize>,
    per_tool_sum: usize,
    count: usize,
    duplicates: Vec<String>,
}

fn tool_defs_census(tool_defs: &[ToolDefinition]) -> ToolDefsCensus
fn json_array_framing_bytes(count: usize) -> usize   // 2 + count.saturating_sub(1)
```

Pure, sans I/O — même découpe que `build_context_window_fields` et
`brief_size_overrun_verdict` juste à côté, et pour la même raison : l'égalité du
§ 1.1 (c) doit être assertable sans souscripteur `tracing`.

**`count` est `tool_defs.len()`, jamais `per_tool.len()`.** C'est le `n` du terme
de cadrage, et en présence d'un doublon les deux divergent (§ 3.2).

### 3.2 Les doublons de nom : sommés, et nommés

`skill_tool_defs.extend_from_slice(mcp.tool_definitions())` (`mod.rs:5606`)
**n'est pas dédoublonné** — le `HashSet seen` d'`inject_skills_and_resolve_tools`
protège les outils de skill contre les builtins, pas les outils MCP contre le
reste. Les noms MCP sont préfixés `mcp__{serveur}__{outil}`, donc une collision
est improbable ; elle n'est pas structurellement exclue.

Une `BTreeMap` naïve **écraserait** silencieusement la seconde définition et
casserait l'égalité : le recensement dirait moins que le total sans que personne
sache pourquoi. Donc : **on somme sur collision**, et on nomme.

- `per_tool[nom] += octets` — l'égalité reste vraie **par construction**, et la
  valeur d'un nom en doublon signifie alors « le total des définitions portant ce
  nom », ce qui est la seule lecture honnête d'un tableau qui en contient deux.
- un WARN `tool_defs_census_duplicate_name` par nom en doublon, portant
  `agent_id`, `tool`, `occurrences`. Sans lui, « deux définitions sous un nom »
  serait indiscernable d'« une définition deux fois plus grosse » — classe
  mika#2205, sur un tableau que le modèle reçoit réellement tel quel.

### 3.3 La réconciliation : assertie à l'exécution, pas seulement en test

```rust
let framing = json_array_framing_bytes(census.count);
if census.per_tool_sum + framing != tool_defs_bytes { /* WARN */ }
```

Trois soustractions d'entiers par tour, sur des valeurs déjà calculées. Le WARN
`tool_defs_census_mismatch` porte `agent_id`, `trace_id`, `tool_defs_bytes`,
`per_tool_sum`, `tool_defs_count`, `framing_bytes` et `delta`.

**Pourquoi un contrôle d'exécution alors qu'un test suffit en théorie.** Le test
(§ 4.3) couvre le registre tel qu'il est à la compilation. Ce qui atteint le
modèle, c'est le registre **plus** les outils de skill du tour, **plus** les
outils MCP d'un serveur tiers dont le schéma n'est pas dans ce dépôt. Le test ne
peut pas voir ce dernier. Et l'exigence du ticket est « à assertir, pas à
supposer » : une égalité vraie en test et jamais regardée en production est
exactement une supposition. La ligne est **émise dans tous les cas** — le WARN
s'ajoute, il ne remplace pas : un instrument qui se tait quand il se trouve faux
est pire que pas d'instrument.

### 3.4 L'émission

```rust
const TOOL_DEFS_CENSUS_EVENT: &str = "tool_defs_census";

fn emit_tool_defs_census(
    agent_id: &str, session_id: &str, trace_id: &str, mode: &str,
    census: &ToolDefsCensus, tool_defs_bytes: usize,
)
```

Appelée au site unique, avec le `tool_defs_bytes` **du même
`ContextWindowFields`** — donc littéralement le nombre que l'autre ligne publie,
pas un second calcul qui pourrait diverger.

La sérialisation de la carte (`BTreeMap<String, usize>` → JSON) ne peut pas
échouer en pratique ; le `unwrap_or` dégrade en `"{}"` **et émet**
`tool_defs_census_unserializable` avec `tool_defs_count`, parce que `"{}"` muet
avec `n = 51` serait un mensonge lisible comme un tableau vide.

---

## 4. Livrables et ordre de commit

**Commit 1 — `feat(obs)` : l'instrument.** `crates/mika-agent/src/agent_loop/mod.rs` :
`TOOL_DEFS_CENSUS_EVENT`, `ToolDefsCensus`, `tool_defs_census`,
`json_array_framing_bytes`, `emit_tool_defs_census` (avec ses trois WARN), et
l'appel au site unique après `emit_context_window_assembled`. L'arbre compile et
rien n'est mort.

**Commit 2 — `test(obs)` : les gardes.** Tests unitaires dans le `mod tests` en
ligne de `mod.rs` (§ 4.3), plus la garde de scribe unique dans
`crates/mika-agent/src/canonical_tokens.rs`.

**Commit 3 — `docs(obs)`.** `docs/agent-internals/observability.md` : la section
de lecture, la table d'absence du § 2.4, les cinq sondes du § 5. **Rien dans
`CLAUDE.md`** — ni la racine (plafonnée, `claude_md_size.rs`) ni celui de
`mika-agent`, qui renvoie déjà l'observabilité vers ce fichier.

### 4.3 Les tests, et ce que chacun voit que les autres ne voient pas

| test | ce qu'il tient |
|---|---|
| `mika2783_la_somme_par_outil_plus_le_cadrage_egale_le_total` | l'égalité du § 1.1 (c) sur **le vrai registre** (`default_tools()`, sans argument, `definitions()` publique — la même autorité que `mika2593_le_tableau_servi_a_mika_arch_est_un_roster_atteste`) |
| `…_sur_un_tableau_synthetique_de_trois_definitions` | la même égalité sur un tableau construit à la main : le registre réel pourrait changer, l'arithmétique non |
| `…_un_tableau_vide_rend_une_carte_vide_et_un_cadrage_de_deux` | `n = 0` → `{}`, `per_tool_sum = 0`, cadrage `2` (`[]`). Le cas limite de `saturating_sub` |
| `…_une_definition_unique_na_pas_de_virgule` | `n = 1` → cadrage `2`. Le second cas limite, celui qu'un `2 + n - 1` naïf rate |
| `mika2783_un_outil_non_servi_na_pas_de_cle` | contrôle négatif de mika#2331 : un nom retiré du tableau est **absent** de la carte, et `get(nom)` rend `None`, pas `Some(0)` |
| `mika2783_un_nom_en_doublon_est_somme_et_legalite_tient` | deux définitions homonymes : `count = 2`, `per_tool.len() = 1`, la valeur est la somme, l'égalité tient, et `duplicates` nomme le coupable |
| `mika2783_la_carte_est_ordonnee` | l'ordre des clés ne dépend pas de l'ordre du tableau — ce qui rend deux lignes diffables |
| `mika2783_le_nom_de_levenement_a_un_seul_scribe` (`canonical_tokens.rs`) | scan de source : un seul site de production écrit `tool_defs_census` |

Le scan de scribe unique suit la forme de
`mika2474_the_overrun_name_has_a_single_writer` : aiguille composée à l'exécution
pour que le fichier ne se dénonce pas, lignes de commentaire exclues, message
d'échec nommant les deux remèdes. **Sans liste d'exceptions** — voir § 7.

---

## 5. Verification Contract

Cinq sondes, toutes post-déploiement (`make deploy`), sur `$MIKA_SPIRIT_LOG_FILE`.

**P1 — l'instrument existe.**
```bash
grep tool_defs_census "$MIKA_SPIRIT_LOG_FILE" | tail -1 \
  | jq '{agent_id, mode, tool_defs_count, per_tool_sum, tool_defs_bytes}'
```
Attendu : une ligne, `mode = "conversation"`, les trois nombres non nuls. Aucune
ligne ⇒ le binaire servi précède le correctif (mika#2340), **jamais** « aucun
outil servi ».

**P2 — l'égalité tient en production. C'est l'arbitre du ticket.**
```bash
grep tool_defs_census "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.tool_defs_bytes
      != (.per_tool_sum + 2 + (if .tool_defs_count > 0 then .tool_defs_count - 1 else 0 end)))
      | {agent_id, trace_id, tool_defs_bytes, per_tool_sum, tool_defs_count}'
```
Attendu : **vide**. Et son contrôle positif, qui dit que la sonde regarde bien
quelque chose :
```bash
grep -c tool_defs_census_mismatch "$MIKA_SPIRIT_LOG_FILE"   # attendu : 0
grep -c '"event":"tool_defs_census"' "$MIKA_SPIRIT_LOG_FILE" # attendu : > 0
```

**P3 — la question pour laquelle ce ticket existe : que coûte UN nom ?**
```bash
grep tool_defs_census "$MIKA_SPIRIT_LOG_FILE" \
  | jq -r 'select(.agent_id == "mika-arch") | .per_tool_bytes | fromjson
           | to_entries[] | "\(.value)\t\(.key)"' \
  | sort -rn | uniq | head -20
```
Attendu : dix-neuf noms classés par coût décroissant. C'est la réponse que
mika#2748 ne pouvait pas obtenir, et elle arrive **avant** la décision au lieu
d'après le déploiement.

**P4 — contrôle négatif : absent n'est pas zéro.**
```bash
grep tool_defs_census "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.agent_id == "mika-arch") | .per_tool_bytes | fromjson
           | {promote_deferred_callback: (has("promote_deferred_callback"))}' | tail -1
```
Attendu : `false`. `promote_deferred_callback` est dans `MIKA_ARCH_DISABLED_TOOLS`
(`well_known_agents.rs:483`, mika#2593) : il n'est **pas servi**, donc il n'a pas
de clé — pas une clé à `0`. La même sonde sur un agent qui le reçoit (`mika-dev`)
doit rendre `true` : c'est la paire qui sépare « non servi » de « instrument
muet ».

**P5 — pas de doublon de nom.**
```bash
grep -c tool_defs_census_duplicate_name "$MIKA_SPIRIT_LOG_FILE"   # attendu : 0
```
Non nul ⇒ un serveur MCP sert un nom déjà pris ; l'égalité tient quand même
(§ 3.2), mais la valeur de ce nom agrège deux définitions et doit être lue comme
telle.

**Pré-vol (worktree, sans déploiement) :** `cargo test -p mika-agent`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`scripts/verify-pipeline.sh origin/main`.

---

## 6. Hors périmètre, et pourquoi

- **`system_prompt_bytes` sur `context_window_assembled`** (§ 8 (b) de mika#2593) :
  défaut de mesure distinct, nommé hors périmètre par le ticket lui-même.
- **Les modes silent et team.** Ils ne calculent pas `tool_defs_bytes` (§ 2.5).
  Les y étendre demanderait d'y porter `build_context_window_fields`, soit un
  second site pour une grandeur qui n'en a qu'un — précisément ce que le
  doc-comment d'`emit_context_window_assembled` met en garde contre.
- **Toute décision de retrait d'outil.** Ce vol livre l'instrument ; s'en servir
  est un autre vol, et le ticket le dit.
- **Le basculement allowlist de mika#2748 (PR #2781).** Indépendant dans les deux
  sens (§ 1.1 (a)).
- **Une persistance en base du recensement.** Le journal est la surface de ce
  genre de mesure dans ce dépôt (`system_prompt_assembled`,
  `context_window_assembled`, `llm_budget_resolved`) ; une colonne demanderait une
  migration de schéma pour une donnée dont la question est « classe-moi ces noms »,
  à quoi `jq` répond.
- **Le dédoublonnage des outils MCP à l'ajout** (`mod.rs:5606`). Un défaut réel,
  en amont, dont ce vol livre la **détection** (§ 3.2) et non la correction :
  corriger exigerait de trancher qui gagne entre un builtin et un outil MCP
  homonyme, ce qui est une décision de surface d'outils, pas de mesure. Si P5
  tire, c'est le ticket à ouvrir.

---

## 7. Fire-Disposition

Ce vol livre **trois détecteurs**, et la disposition est **(c) halte-et-remontée**
pour les trois, sans aucune liste d'exceptions.

| détecteur | chemin de succès | ce qu'il signale |
|---|---|---|
| `tool_defs_census_mismatch` (exécution, § 3.3) | silence | la somme par outil et le total mesurent deux populations (classe mika#2205) |
| `mika2783_la_somme_par_outil_plus_le_cadrage_egale_le_total` (test) | vert | la même rupture, sur le registre compilé |
| `mika2783_le_nom_de_levenement_a_un_seul_scribe` (scan de source) | un scribe | un second site écrit le nom, donc le `group by agent_id` cesse d'être exact |

**Recensement des violations existantes : zéro**, et il est établi, pas supposé.
Pour les deux premiers, par l'arithmétique du § 1.1 (c) — l'égalité est une
propriété de `serde_json` compact, et le test sur le **vrai** registre
(`default_tools()`) est le relevé qui le confirme à la compilation ; il atterrit
vert, et son échec serait une nouvelle. Pour le troisième, par construction :
`tool_defs_census` est un nom neuf, dont `grep -rn "tool_defs_census" crates/`
retourne exactement les sites que ce vol crée.

**Pourquoi (c) et non (a), qui est le défaut.** Une liste d'exceptions ne se
justifie que s'il y a quelque chose à excepter ; il n'y a rien. Et une allowlist
née vide est un emplacement où déposer la prochaine infraction (mika#2323,
doctrine mika#2201) — le dépôt l'écrit déjà noir sur blanc pour
`EMPTY_BACKLOG_SOLE_WRITER_EXCEPTIONS`. Les trois détecteurs atterrissent donc
**armés et sans échappatoire**, et chaque message d'échec nomme les deux remèdes
réels plutôt qu'un troisième qui serait de se taire :

- pour le scan de scribe : *router le site par `emit_tool_defs_census`, ou le
  retirer — jamais une entrée dans une liste* ;
- pour l'égalité : *la carte et le total ne décrivent plus le même tableau ;
  réparer le calcul, jamais ajuster le terme de cadrage pour faire passer le
  test.*

Aucun détecteur n'atterrit désarmé (pas de `#[ignore]`), et rien ne remonte à
l'opérateur à l'implémentation : il n'y a pas de cadrage en suspens.

---

## 8. Risques

**R1 — L'égalité est fausse pour une raison que je n'ai pas vue.** Si
`serde_json` insérait quoi que ce soit entre les éléments, le WARN d'exécution
tirerait sur **chaque** tour et noierait le journal. *Mitigation :* le test
synthétique du § 4.3 tire à la compilation, bien avant tout déploiement — un
`cargo test` rouge est la forme sous laquelle ce risque se manifeste, et c'est la
bonne. Le WARN de production ne peut donc atteindre le journal que pour une
cause que le test ne voit pas, c'est-à-dire un outil MCP — exactement ce pour
quoi il existe.

**R2 — Le volume.** La carte pèse ≈ 25–30 o par nom servi, soit ≈ 1,4 Ko pour
l'agent au roster le plus large (≈ 51 builtins) et ≈ 0,5 Ko pour mika-arch
(19 noms), **une fois par tour de conversation**. Je n'ai pas, depuis un
worktree, le nombre de tours par jour qui en ferait un chiffre quotidien : la
sonde P1 le donne le lendemain du déploiement (`grep -c`). *Mitigation :* c'est
le nom distinct (§ 2.1) qui est le levier — un filtre peut faire taire ce seul
événement sans toucher `context_window_assembled`, ce qu'un champ de plus sur le
voisin n'aurait pas permis.

**R3 — Un outil au schéma énorme écrase la lisibilité de la ligne.** Non : la
carte porte des **tailles**, jamais des schémas. Sa taille est bornée par la somme
des longueurs des *noms*, pas des définitions.

**R4 — Conflit avec la PR #2781, ouverte sur le même fichier.** Les deux touchent
`agent_loop/mod.rs` : #2781 autour d'`apply_agent_tool_visibility` (~l. 8844),
ce vol autour de `mod.rs:5715` et `mod.rs:9500`. Aucun chevauchement de hunk
attendu. *Mitigation :* si le rebase mord, c'est un conflit textuel sans conflit
sémantique — le recensement lit le tableau après le filtre, quelle que soit la
forme du prédicat.

---

## 9. Definition of Done

1. `tool_defs_census` est émis à chaque tour de conversation, au site unique, non
   gaté par `MIKA_STORE_LLM_CALLS`.
2. La ligne porte `agent_id`, `session_id`, `trace_id`, `mode`,
   `tool_defs_bytes`, `tool_defs_count`, `per_tool_sum`, `per_tool_bytes`.
3. `tool_defs_bytes` provient de `tool_defs_byte_weight` — l'expression unique —
   et vaut le même nombre que sur `context_window_assembled` du même tour.
4. L'égalité `per_tool_sum + framing == tool_defs_bytes` est vérifiée à
   l'exécution ; un écart émet `tool_defs_census_mismatch` **et** la ligne est
   émise quand même.
5. Un outil non servi est **absent** de la carte ; un tableau vide rend `{}` avec
   `tool_defs_count = 0` ; l'absence de la ligne signifie un binaire antérieur.
6. Un nom en doublon est sommé et émet `tool_defs_census_duplicate_name`.
7. Les huit tests du § 4.3 passent, dont celui sur le registre réel.
8. `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
   `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` sont verts.
9. `docs/agent-internals/observability.md` porte la section de lecture, la table
   d'absence et les cinq sondes. Aucun `CLAUDE.md` n'est élargi.
10. Le corps de PR porte le relevé P1–P5 **post-déploiement** si le déploiement a
    lieu dans le vol ; sinon il dit explicitement que les sondes sont dues au
    prochain `make deploy`, sans affirmer de valeur non relevée.

---

## 10. Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; ces critères
sont dérivés de son § *Périmètre proposé* (quatre puces) et du § 5 ci-dessus.

- **AC1 — Un recensement par outil existe.** Un événement émis au point
  d'assemblage du tableau porte les octets de la définition sérialisée de **chaque**
  outil servi, nommément. Vérifié par P3 : la liste des dix-neuf noms de mika-arch
  classés par coût.
- **AC2 — La somme égale le total du même tour.** `per_tool_sum` plus le cadrage
  JSON (`2 + max(n−1, 0)`) égale `tool_defs_bytes` du même tour. **Asserti** — par
  un test sur le registre réel, par un test synthétique, et à l'exécution par un
  WARN dédié. Vérifié par P2 et son contrôle positif.
- **AC3 — Le canal est tranché, et le motif est écrit.** Un événement unique
  portant une carte `{nom: octets}`, distinct de `context_window_assembled`. Les
  deux formes concurrentes (une ligne par outil ; une émission à la première
  assemblée) sont refusées au § 2.2 avec leur motif propre, dont le fait de code
  qui tranche la seconde (le tableau servi varie d'un tour à l'autre).
- **AC4 — Absent n'est jamais zéro, aux trois niveaux.** Un outil non servi n'a
  pas de clé (mika#2331) ; l'absence de la ligne signifie un binaire antérieur au
  correctif (mika#2340) et jamais un tableau vide ; un tableau vide rend `{}` avec
  `tool_defs_count = 0`. Vérifié par P4 et sa paire positive sur mika-dev.
- **AC5 — Le nom a un scribe unique.** Une garde de source refuse un second site
  de production écrivant `tool_defs_census`, sans liste d'exceptions.
- **AC6 — Rien du hors-périmètre n'est touché.** Aucun outil n'est retiré ou
  ajouté à un agent ; `system_prompt_bytes` n'est pas ajouté à
  `context_window_assembled` ; aucun schéma de base ne change.

---

## 11. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/agent_loop/mod.rs` — const, `ToolDefsCensus`, `tool_defs_census`, `json_array_framing_bytes`, `emit_tool_defs_census`, site d'appel (doc-comments inclus, au format du fichier) | 150 |
| tests (`mod tests` en ligne de `agent_loop/mod.rs`, 7 tests) | 140 |
| garde de scribe unique (`crates/mika-agent/src/canonical_tokens.rs`) | 45 |

Total estimé : 335 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 335 < 429 et 335 < 1000 : les deux
bornes passent, la plus serrée (429) avec ~22 % de marge. Le travail est
structurellement borné : **une** fonction pure, **une** fonction d'émission, **un**
site d'appel, et aucun changement de signature chez les appelants — `tool_defs_bytes`
reste calculé où il l'était, `tool_defs_byte_weight` n'est pas touché, et le
`&skill_tool_defs` du site est déjà en main. Les deux postes qui font déborder ce
genre de vol sont absents : pas de migration de schéma (le journal est la surface)
et pas de fan-out multi-sites (§ 2.5 borne la portée au mode conversation, qui a
un appelant). Aucune ligne `Tracked in:` n'est due : rien n'est renvoyé à un vol
suivant — les hors-périmètre du § 6 sont des hors-périmètre ordinaires, dont deux
ont déjà leur ticket (mika#2748 pour l'allowlist, § 8 (b) de mika#2593 pour
`system_prompt_bytes`) et un ne s'ouvre que si P5 tire.
