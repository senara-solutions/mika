# mika#3000 — la base de `self-check` nomme trois outils qui n'existent pas

**Ticket :** senara-solutions/mika#3000
**Labels :** `bug`, `p2-normal`, `ready`, `dispatch:loop`, `loop-substrate`
**Type :** fix
**Vols :** un seul (voir § 8)

---

## 1. Où en est le ticket, et pourquoi ce vol-là

### 1.1 Le défaut, vérifié dans l'arbre

`skills/bundled/self-check/system_prompt.md` nomme quatre fois un outil que le
moteur ne sert pas :

| ligne | texte actuel | ce que le tour appelle |
|---|---|---|
| 19 | ``Path: `data/self-check/YYYY-MM-DDTHH-MM-SS.json` (relative to agent home, use `write_file`)`` | `write_file` |
| 25 | ``1. **Find previous report** via `list_home_files` on `data/self-check/`.`` | `list_home_files` |
| 27 | ``2. **Load previous report** via `read_home_file`.`` | `read_home_file` |
| 94 | ``11. **Write report** via `write_file` to `data/self-check/…`.`` | `write_file` |

Les trois noms sont absents du registre des outils servis, qui a deux termes et
dont la réunion est l'univers complet :

- `crates/mika-agent/src/tools/mod.rs:1357` — `BUILTIN_TOOL_NAMES` (60 noms),
  dont la parité avec `default_tools()` + `management_tools_if_needed()` +
  `skills::builtin_handlers::KNOWN_BUILTINS` est déjà tenue par
  `test_builtin_tool_names_parity` ;
- `crates/mika-agent/src/bundled_skills.rs:275` — `all_bundled_tool_names()`
  (24 noms), la réunion des `name` de tous les `tools.json`, bundled **et**
  templates legacy, donc `run_shell`, `read_file`, `tmux_*`, `qa_pr_view`…

Un détail que la lecture du ticket ne donne pas et qui change la forme du
correctif : **`read_file` EST servi** — c'est l'outil du skill `file-reader`. Ce
n'est donc pas « la famille `*_file` n'existe pas », c'est précisément
`write_file` et `read_home_file` / `list_home_files` qui n'existent pas. Les
trois outils réels sont `list_agent_files`, `read_agent_file`,
`write_agent_file` (`tools/mod.rs:1380-1382`).

### 1.2 Pourquoi le défaut n'a rien coûté pendant six mois, et pourquoi il coûte maintenant

La base n'a reçu aucun commit depuis le 2026-04-18, mais elle n'était pas
**servie** : la variante `generated/` de juin, qui emploie les bons noms, la
remplaçait. Le deploy de `8f7b2260` (2026-11-01, 06:27Z) a écarté cette variante,
et la base est servie telle quelle depuis. C'est exactement la mécanique que
mika#2961 vient de documenter une ligne plus bas dans le même fichier de tests :
*« la variante périmée masquait le défaut, et le retour à la base l'a révélé »*.

Qui est touché : `self-check` figure dans l'`[skills].allowlist` de **mika-dev**
et de **mika-qa** (`well_known_agents.rs:151` et `:328`). Les deux portent aussi
`shell-exec`, donc le `run_shell` des étapes 4 et 5 du prompt, lui, est servi — il
n'est pas en cause.

### 1.3 Ce que ce vol corrige, et ce que seul le deploy achève

`skills/bundled/` n'est lu qu'**au build** : `build.rs` le compile dans
`BUNDLED_SKILL_MANIFESTS`, et `~/.mika/skills/` est une projection du binaire
(mika#2340). Le vol corrige donc la **source** ; aucun agent ne voit la
correction avant un `make deploy`. Ce geste est opérateur et hors PR — il est
nommé au § 5 comme suite, pas comme critère.

### 1.4 Périmètre, face à l'arbre existant

- Aucune variante de prompt sous `skills/bundled/` : 26 skills, un seul
  `system_prompt.md` chacun, vérifié par `find skills/bundled -type f -name '*.md'`.
  La portée « `system_prompt.md` seul » du détecteur n'est donc pas une
  restriction en pratique aujourd'hui, mais elle est dite comme borne au § 2.2.
- Aucun détecteur existant ne couvre cette classe. `make verify-bundled-skills`
  (mika#1575, Check 4) vérifie les tokens de `[constraints] required_tools` dans
  les `skill.toml` — pas les noms cités dans la **prose du prompt**. `self-check`
  ne déclare aucun `required_tools`, ce qui est précisément pourquoi ce gate l'a
  laissée passer pendant six mois. Les deux détecteurs ne se recouvrent pas.

---

## 2. Le dessin

### 2.1 Livrable 1 — AC1 : les quatre occurrences, avec les arguments réels

Signatures lues dans l'arbre, et non supposées :

| outil | `required` | propriétés |
|---|---|---|
| `list_agent_files` | *(aucune)* | `path` (omis ou `""` ⇒ racine du home), `agent` |
| `read_agent_file` | `path` | `path`, `agent` |
| `write_agent_file` | `path`, `content` | `path`, `content`, `confirm`, `agent` |

Les quatre réécritures :

- **l. 19** — `use write_file` devient : ``use `write_agent_file` with `path` and
  `content` ``. Le `path` reste relatif au home de l'agent, ce que la ligne dit
  déjà (« relative to agent home ») et ce que l'outil attend littéralement
  (« Relative path within the agent's home directory »).
- **l. 25** — ``via `list_home_files` on `data/self-check/` `` devient
  ``via `list_agent_files` with `path: "data/self-check"` ``.
- **l. 27** — ``via `read_home_file` `` devient ``via `read_agent_file` with
  `path` set to the file found at step 1``.
- **l. 94** — ``via `write_file` to `data/self-check/…` `` devient ``via
  `write_agent_file` with `path: "data/self-check/YYYY-MM-DDTHH-MM-SS.json"` and
  the report JSON as `content` ``.

**`confirm` n'est pas ajouté, et c'est une décision, pas un oubli.**
`write_agent_file` exige `confirm: true` pour **écraser** un fichier existant ; le
chemin d'écriture de `self-check` est horodaté à la seconde, donc jamais
préexistant. Ajouter `confirm: true` au prompt apprendrait au modèle à passer le
garde-fou d'écrasement par réflexe sur **tous** ses appels, y compris ceux où il
protège quelque chose. L'omission est le comportement correct ; le prompt dirait
le contraire du moteur en le mentionnant.

### 2.2 Livrable 2 — AC2 et AC3 : le détecteur

**Où il vit.** `crates/mika-agent/src/bundled_skills.rs`, dans son `mod tests`
existant, immédiatement après
`mika2961_un_marqueur_de_contexte_declare_est_injecte_exactement_une_fois`. Trois
raisons, et chacune élimine une alternative :

1. **Il lit les octets servis, pas l'arbre.** `ENTRIES` — le `static` que
   `build.rs` génère depuis `skills/bundled/` — est privé au module
   `bundled_skills`, et `mod tests` est son enfant, donc il le voit sans qu'aucun
   accesseur public soit ajouté. Un module frère (`src/phantom_tool_scan.rs`)
   obligerait à exposer `ENTRIES`, c'est-à-dire à élargir la surface publique
   pour un détecteur.
2. **Le registre est déjà là.** `all_bundled_tool_names()` est défini dans ce
   fichier ; `crate::tools::BUILTIN_TOOL_NAMES` est `pub`. Le détecteur et son
   référentiel au même endroit, c'est ce qui rend la prochaine lecture possible.
3. **C'est l'idiome du dépôt pour un prédicat de détecteur.**
   `tools/mod.rs:1565` porte déjà `fn declared_tool_names(src: &str)` comme
   helper de son `mod tests`. Un prédicat consommé uniquement par des tests
   déclaré hors de `mod tests` déclencherait `dead_code`.

**Le prédicat.** Un helper pur, pour que AC3 puisse l'appeler sur une fixture
sans aucune E/S :

```rust
/// Rend, pour un prompt donné, les noms d'outils FANTÔMES qu'il nomme : un
/// token entre accents graves qui imite la forme d'un outil servi sans en
/// être un. Chaque violation porte l'outil du registre qu'elle imite — c'est
/// la correction, pas seulement le diagnostic.
fn phantom_tool_mentions(
    prompt: &str,
    served: &std::collections::HashSet<&str>,
) -> Vec<(usize, String, String)> // (ligne, token, outil imité)
```

**Comment il distingue un nom d'outil d'un autre identifiant — la question
qu'AC2 pose explicitement.** Deux filtres successifs, et le second est celui qui
porte tout le travail.

*Filtre de forme.* Le token entre accents graves doit matcher
`^[a-z][a-z0-9_]*$` et compter au moins deux segments séparés par `_`. Ce filtre
écarte **structurellement** trois populations que les prompts bundled citent
massivement et qui ne sont pas des outils du moteur : les outils de Claude Code
en PascalCase (`Write`, `Edit`, `Grep`, `Bash`), les chemins et fichiers
(`skills/bundled/…`, `dispatch-lib.sh`), et les clés à tirets
(`origin:loop`, `pr-body.md`). Il ne les met pas en allowlist : il ne les voit
pas.

*Filtre d'imitation.* Le token est une violation s'il existe au registre un
outil `A` tel que `premier_segment(token) == premier_segment(A)` **et**
`dernier_segment(token) == dernier_segment(A)` — le **même** `A` pour les deux —
et que `token` lui-même n'est pas au registre.

- `list_home_files` → `A = list_agent_files` (`list`…`files`) ⇒ violation.
- `read_home_file` → `A = read_agent_file` (`read`…`file`) ⇒ violation.
- `write_file` → `A = write_agent_file` (`write`…`file`) ⇒ violation.

**Pourquoi ce discriminant et pas le préfixe seul — mesuré, pas supposé.** Les
26 prompts sous `skills/bundled/` portent **166** tokens `snake_case` distincts
entre accents graves (`grep -roh '`[a-z][a-z0-9_]*_[a-z0-9_]*`' skills/bundled/
--include=system_prompt.md | sort -u`, exécuté le 2026-11-01). Sur cette
population :

| discriminant | vraies violations | faux positifs |
|---|---|---|
| premier segment seul | 3 | **14** |
| premier **et** dernier segment | 3 | **0** |

Les quatorze que le préfixe seul produirait sont `pr_diff`, `pr_url`, `pr_state`,
`pr_number`, `pr_closed`, `pr_view`, `check_suite`, `review_failed`,
`review_requested`, `qa_retry_count`, `qa_build_callback_verdict`,
`update_task_metadata`, `build_command_exceeds_tool_budget`, `list_people` —
un marqueur de contexte, un type d'événement GitHub, des champs JSON, des noms
de tests. **Aucun ne référence un ticket de suivi, parce qu'il n'y a rien à
suivre** : ce ne sont pas des violations en attente de correction, ce sont des
identifiants légitimes. Une allowlist de quatorze entrées sans ticket derrière
est exactement l'allowlist que mika#2309 condamne — celle dont le contenu légitime
noie la seule entrée qui aurait dû attirer l'œil. Le filtre d'imitation obtient
le même rendement de détection avec une allowlist **vide**, et c'est le seul
argument qui le choisit.

**La borne, dite plutôt que découverte plus tard.** Le détecteur couvre la classe
du défaut — un nom qui **imite** un outil servi — et non la classe « nom
entièrement libre ». Un `frobnicate_widget` dans un prompt bundled, dont ni le
verbe ni le nom final n'existent au registre, passerait. Borne inférieure
assumée, même forme que mika#2961 : un détecteur qui sous-approxime rend des
faux verts bornés, jamais des faux rouges, et c'est dans ce sens-là qu'on
accepte l'erreur. Autres bornes, pour les mêmes raisons :

- **Fichier** : `system_prompt.md` seul. `skill.toml` porte de la prose de
  commentaire et `tools.json` des descriptions ; ni l'un ni l'autre n'est servi
  au modèle comme instruction d'appel.
- **Arbre** : `ENTRIES` seul, donc `skills/bundled/`, ce qu'AC2 demande
  littéralement. Les skills legacy sous `crates/mika-agent/templates/skills/`
  sont hors portée de ce ticket — ils **alimentent** le registre via
  `all_bundled_tool_names()`, ils ne sont pas scannés.

**Assertion de population non vide, et elle vient d'abord.** `ENTRIES` vaut `&[]`
si la découverte de `build.rs` casse ou si `skills/bundled/` disparaît — le
commentaire de `bundled_skills.rs:202` le dit en propres termes. Sans cette
assertion, le détecteur est vert exactement le jour où un vert cesse de vouloir
dire « aucune violation ». Motif et formulation reprises de mika#2961 ; le
compteur porte sur le nombre de **prompts scannés** et sur le nombre de **noms
au registre**, les deux pouvant s'effondrer indépendamment.

**Message d'échec.** Nomme le skill, le numéro de ligne, le token, et l'outil du
registre qu'il imite — ce dernier étant la correction à appliquer, pas seulement
le diagnostic.

**AC3 — le contrôle négatif, dans un seul test avec son positif.** Une seule
fonction de test porte la paire :

```rust
#[test]
fn mika3000_le_scan_rougit_sur_un_outil_plante_et_passe_une_fois_corrige() {
    // fixture plantée — la ligne 25 d'avant le correctif
    // fixture corrigée — la même ligne, avec `list_agent_files`
}
```

Les deux moitiés dans le même test est une décision : séparées, le contrôle
négatif peut être supprimé sans que rien ne redevienne rouge, et un détecteur
dont on a retiré la preuve qu'il mord est un détecteur qui passe par
construction. Ensemble, le geste qui supprime l'un supprime l'autre.

### 2.3 Ce que ce vol ne fait pas, et qui le porte

- **Il ne déploie pas.** `make deploy` est le geste qui met la correction en
  service (mika#2340). Hors PR, nommé au § 5 comme suite opérateur.
- **Il n'audite pas les prompts des templates legacy**
  (`crates/mika-agent/templates/skills/`). Hors portée AC2. Si le scan y était
  étendu un jour, ce serait un ticket avec sa propre mesure de faux positifs.
- **Il ne couvre pas la classe « nom entièrement libre »** (§ 2.2, borne). Pas
  de ticket ouvert : aucune occurrence mesurée dans la population actuelle, donc
  rien à suivre aujourd'hui.
- **Il ne touche pas `required_tools`.** `self-check` n'en déclare pas, et lui en
  ajouter changerait la garde `required_tools` du moteur sur ce skill — hors
  périmètre d'un fix de prompt.

---

## 3. Fichiers touchés

| fichier | nature du changement |
|---|---|
| `skills/bundled/self-check/system_prompt.md` | AC1 — quatre occurrences réécrites (l. 19, 25, 27, 94) |
| `crates/mika-agent/src/bundled_skills.rs` | AC2/AC3 — helper `phantom_tool_mentions`, allowlist `PHANTOM_TOOL_EXCEPTIONS` (vide), deux tests, dans `mod tests` |

Aucun autre fichier. Pas de migration, pas de nouveau module, pas de nouvelle
surface publique, pas de changement de `CLAUDE.md` (le détecteur se documente
dans son doc-comment, à l'endroit où le prochain lecteur le rencontrera).

---

## 4. Ordre de commit

Deux commits, dans cet ordre, parce que l'ordre inverse rendrait le second
commit vert pour une mauvaise raison :

1. **`fix(3000): le détecteur refuse un nom d'outil qui imite un outil servi`**
   — helper, allowlist vide, les deux tests. `cargo test -p mika-agent
   bundled_skills` est **ROUGE** à ce commit, sur les trois occurrences de
   `self-check`, et ce rouge est la preuve que le détecteur mord sur le défaut
   réel et non seulement sur une fixture.
2. **`fix(3000): la base de self-check nomme les trois outils que le moteur sert`**
   — les quatre réécritures du prompt. La suite repasse au vert.

**Plancher de ce vol :** le commit 2 seul, sans le commit 1, corrigerait le
symptôme et laisserait la classe entière non gardée — c'est l'état qui a tenu six
mois. Les deux commits sont le plancher ; aucun n'est mergeable seul.

---

## 5. Definition of Done

- [ ] `skills/bundled/self-check/system_prompt.md` ne nomme plus
      `list_home_files`, `read_home_file` ni `write_file` ; les trois outils
      réels y figurent avec les arguments que leur `input_schema` exige.
- [ ] `phantom_tool_mentions` est un helper pur, sans E/S, appelable sur une
      `&str` de fixture.
- [ ] Le test de scan parcourt `ENTRIES`, lit `system_prompt.md`, consomme
      `BUILTIN_TOOL_NAMES ∪ all_bundled_tool_names()`, et porte son assertion de
      population non vide **avant** son assertion de violations.
- [ ] `PHANTOM_TOOL_EXCEPTIONS` est livrée vide, et une assertion rougit sur
      toute entrée devenue stale (§ 7).
- [ ] Le test de fixture porte le contrôle planté **et** le contrôle corrigé
      dans la même fonction.
- [ ] `cargo test -p mika-agent` vert.
- [ ] `cargo clippy --all-targets -- -D warnings` vert.
- [ ] `cargo fmt --check` vert.
- [ ] `make verify-bundled-skills` vert (le prompt reste structurellement valide).
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] Corps de PR : `Code review: delegated to mika-qa (mika#2694)`.

**Suite opérateur, hors DoD parce que hors PR :** `make deploy`, puis
`cat ~/.mika/skills/.manifest-writer` pour vérifier que le `git_hash` servi est
bien celui du merge — c'est le seul instant où un tour de `self-check` cesse
d'appeler trois outils inexistants (mika#2340).

---

## 6. Acceptance criteria

Transcrits verbatim du corps du ticket, avec leur disposition.

- **AC1.** Les quatre occurrences sont remplacées par `list_agent_files`,
  `read_agent_file` et `write_agent_file`, chacun avec les arguments que l'outil
  réel attend.
  → **Livré**, § 2.1. Les arguments viennent des `input_schema` lus dans
  `tools/{list_agent_files,read_agent_file,write_agent_file}.rs`, pas d'une
  supposition. L'omission délibérée de `confirm` est argumentée au § 2.1.

- **AC2.** Un test de scan refuse, dans les prompts sous `skills/bundled/`, un
  nom d'outil écrit entre accents graves qui ne figure ni au registre des outils
  servis ni dans une liste nommée d'exceptions. Le plan dit comment le scan
  distingue un nom d'outil d'un autre identifiant.
  → **Livré**, § 2.2. Registre = `BUILTIN_TOOL_NAMES ∪ all_bundled_tool_names()`.
  Liste nommée = `PHANTOM_TOOL_EXCEPTIONS`, livrée vide. Le discriminant — filtre
  de forme puis filtre d'imitation premier-et-dernier-segment — est exposé avec
  la mesure qui l'a choisi contre son alternative (3 violations / 0 faux positif
  contre 3 / 14), et ses bornes sont nommées.

- **AC3.** Contrôle négatif dans le même appel : un prompt de fixture qui nomme un
  outil planté (`list_home_files`) fait rougir le scan, et le même prompt corrigé
  le fait passer.
  → **Livré**, § 2.2, dernier paragraphe. Les deux moitiés sont dans la **même**
  fonction de test, pour que le contrôle négatif ne soit pas supprimable seul.

---

## 7. Fire-Disposition

**Option (a) — exception nommée en allowlist, et l'allowlist est livrée VIDE.**

Le détecteur livré par ce vol a une population de violations connue : trois, et
le même vol les corrige (AC1). Après le commit 2, il n'en reste aucune — mesuré,
pas espéré : les 166 tokens de la population actuelle ont été passés au
discriminant, et seuls les trois du ticket sortent (§ 2.2).

```rust
/// Exceptions nommées au détecteur de noms d'outils fantômes (mika#3000).
///
/// LIVRÉE VIDE, et ce n'est pas un oubli : le discriminant du § 2.2 du plan a
/// été choisi précisément parce qu'il ne produit aucun faux positif sur la
/// population actuelle, et les trois seules violations réelles sont corrigées
/// par le même vol. Une allowlist non vide ici serait le signal qu'un
/// identifiant légitime est devenu détectable — auquel cas la réparation est
/// le discriminant, jamais l'entrée.
///
/// Toute entrée ajoutée DOIT nommer (token, ticket de suivi), et elle est
/// auto-nettoyante : `…_aucune_exception_nest_devenue_stale` rougit dès que
/// le token nommé n'est plus rencontré par le scan.
const PHANTOM_TOOL_EXCEPTIONS: &[(&str, &str)] = &[];
```

**L'assertion auto-nettoyante, et ce qu'elle garde exactement.** Une allowlist
vide n'a pas d'entrée stale aujourd'hui ; l'assertion existe pour le jour où elle
en aura une. Le test
`mika3000_aucune_exception_de_scan_nest_devenue_stale` collecte les tokens
effectivement **rencontrés** par le scan sur `ENTRIES`, et rougit sur toute entrée
de `PHANTOM_TOOL_EXCEPTIONS` absente de cet ensemble — c'est-à-dire sur toute
exception dont la violation a été corrigée sans que l'exception soit retirée. Le
geste qui rend l'exception inutile est le geste qui la fait expirer, ce qui est
ce qui sépare cette allowlist de celle que mika#2309 condamne.

Sur l'allowlist vide, ce test passe trivialement. Il est néanmoins livré **armé
dès maintenant** plutôt qu'au premier ajout : une assertion de propreté écrite
le jour où on ajoute une exception est une assertion écrite par la personne qui
avait une raison de contourner le détecteur.

Ni (b) ni (c) : le détecteur atterrit **armé** — il n'y a rien à désarmer, sa
population de violations est vide à la fin du vol — et aucune question de cadrage
ne reste ouverte pour l'opérateur.

---

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/self-check/system_prompt.md` (AC1, 4 occurrences) | 5 |
| `crates/mika-agent/src/bundled_skills.rs` — helper `phantom_tool_mentions` + doc | 50 |
| `crates/mika-agent/src/bundled_skills.rs` — `PHANTOM_TOOL_EXCEPTIONS` + doc | 15 |
| `crates/mika-agent/src/bundled_skills.rs` — test de scan sur `ENTRIES` (AC2) | 55 |
| `crates/mika-agent/src/bundled_skills.rs` — test de fixture, paire (AC3) | 35 |
| `crates/mika-agent/src/bundled_skills.rs` — test anti-stale de l'allowlist (§ 7) | 20 |

Total estimé : 180 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 180 lignes contre une borne de 429
et un seuil par PR de 1000 : les deux bornes sont franches, et c'est la borne par
vol — la plus serrée des deux ici — qui mord en premier. Le vol est petit pour
une raison structurelle plutôt que par découpage : il n'ajoute ni module, ni
migration, ni surface publique, ni fixture sur disque. Le prédicat est pur et
tient en une cinquantaine de lignes ; le registre qu'il consomme existe déjà en
deux constantes publiques ; les prompts qu'il scanne sont déjà embarqués par
`build.rs` et déjà parcourus par un test voisin (mika#2961) dont la forme est
reprise telle quelle. L'essentiel du volume est du doc-comment — la mesure des
166 tokens, le choix du discriminant contre son alternative, et les trois bornes
assumées — c'est-à-dire ce qui évite que le prochain lecteur refasse la mesure.

Aucun périmètre n'est renvoyé : pas de ligne `Tracked in:`, parce que les trois
critères du ticket sont livrés entiers par ce vol. Les deux non-objectifs
nommés au § 2.3 (prompts des templates legacy, classe « nom entièrement libre »)
n'ont aucune occurrence mesurée aujourd'hui — ouvrir un ticket pour une
population vide serait un suivi sans objet.
