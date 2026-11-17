# mika#1727 — vol V3c-2b-A2 : une garde ferme la TUI au registre des skills

**Ticket** : senara-solutions/mika#1727 · **Vol** : V3c-2b-A2 (coupe A, sous-vol A2,
choisie par Vincent le 2026-11-17) · **Branche** :
`feat/1727/v3c2b-a2-une-garde-ferme-la-tui-au-registre`, neuve depuis main
(`3b60e2ef`) · **Label** : `loop-substrate`

**Les bornes tiennent, et ce plan ne s'arrête pas.** 113 lignes ajoutées pour une
borne de groom de 200, et 83 hors `docs/` et hors `CLAUDE.md` pour les 429 de
mika#2693 — voir § *Taille estimée*.

**Ce plan ne choisit aucune coupe et n'ajoute aucune halte** à celles de la tête du
ticket. La borne du groom (200) et la halte du pilote (280) sont posées par Vincent
pour ce vol ; ce plan les cite sans les changer.

---

## Ce que ce vol livre, et ce qu'il ne livre pas

A1 (mika#3105, `3b60e2ef`) a fait **partir** `SkillRegistry` de `crates/mika-cli/src/tui/`.
A2 ne refait pas ce geste : il pose la garde qui **refuse son retour**, corrige les deux
commentaires qu'A1 a rendus faux, et amène `crates/mika-cli/CLAUDE.md` à l'état final de
V3c-2b.

**Ce vol n'ajoute aucun code de production.** Tout son code neuf vit dans le module
`tests` d'un seul fichier, plus deux chaînes de commentaire et deux sections de doc.

**Hors de ce vol** : tout changement de comportement. **Puis en V3c-2c** : le test É1.
**Puis en V4** : les équipes et le verrou structurel, au merge duquel le ticket se ferme.
`Tracked in: senara-solutions/mika#1727`.

---

## L'état de main, mesuré sur `3b60e2ef`

### `SkillRegistry` : où il n'est plus, et où il reste légitime

`git grep -n SkillRegistry -- crates/mika-cli/src/tui/` ne rend **aucune ligne**, en
production comme en test. Les douze fichiers de `tui/` sont :
`app.rs`, `attachment.rs`, `commands/autocomplete.rs`, `commands/completers.rs`,
`commands/handlers.rs`, `commands/mod.rs`, `event.rs`, `input.rs`, `markdown.rs`,
`mod.rs`, `ui.rs`, `wrap.rs`.

Hors de `tui/`, le nom reste et doit rester :

| site | ce qu'il en fait |
|---|---|
| `commands/skills.rs` l. 11, 29, 57, 63, 308, 332, 555, 1713, 1715 | `mika skills` **construit** le registre sur le disque — c'est sa raison d'être |
| `commands/iterate.rs` l. 11 | le nomme dans un commentaire |
| `commands/session_remote.rs` l. 250 | le nomme dans un commentaire |

**C'est le fait qui fixe le périmètre de la garde** : elle vise le dossier `tui/`, et
non le crate. Une garde par crate aurait demandé une allowlist de neuf entrées sur
`skills.rs` seul, et une allowlist peuplée est exactement ce qui ne refuse plus rien
(mika#2309) — le dixième appelant s'y serait ajouté sans qu'un œil s'y arrête.

### Le modèle : la garde de V2c-1b

`mika1727_v2c1b_la_tui_ne_rembobine_plus_la_base_en_local`
(`tui/commands/handlers.rs` l. 2996-3046) porte les quatre propriétés que la tête
exige d'A2 : `ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"))`, une allowlist
livrée vide **et assertie vide**, un témoin nommé dans la moitié production
(`async fn handle_rewind_impl(`), et des motifs dont chacun nomme sa porte dans le
message d'échec. Elle filtre `!l.trim_start().starts_with("//")`, et numérote par
`i + 1`.

### Le balayage d'un dossier

`mika_common::source_guard` est un `pub mod` non gaté (`lib.rs` l. 28) :

- `rust_sources_under(&Path) -> Vec<PathBuf>` (l. 939) — tous les `.rs` sous une
  racine, récursivement, **triés**.
- `ProductionScanner::for_crate(manifest_dir)` (l. 969) — scanner sur
  `<manifest_dir>/src` ; `src_root()` (l. 983) rend cette racine ; `production_of(&Path)`
  (l. 1018) rend la moitié production d'un fichier, **en conservant la numérotation**
  (les régions de test sont blanchies, pas supprimées — `blank_like`).

Aucun de ces outils n'est à écrire : A2 les assemble.

### La garde de `chat.rs`, et le motif `skills_dirty`

`mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider`
(`commands/chat.rs` l. 1991-2086) porte **douze** motifs depuis A1 : `worker_settings`,
`make_llm_provider(`, `init_for_agent`, `AppContext`, `connect_mcp`, `McpManager`,
`override_model(`, `skills_dirty`, `assemble_agent_registry`,
`migrate_disabled_markers_async`, `get_skill_overrides`, `updated_skills`.

La raison du motif `skills_dirty` (l. 2016-2019) dit encore :

```
"le registre s'assemble une fois, au démarrage"
```

**C'est faux depuis A1.** La TUI n'assemble plus de registre du tout :
`spawn_agent_worker` lit la route (`chat.rs` l. 266) au démarrage, à la bascule
d'agent (l. 845-859) et au `/restart` (l. 958-967).

### L'en-tête de `skills_remote.rs`

L. 15-21 : « les six attentes `dead_code` que les items sans lecteur portaient sont
tombées avec lui ». Vrai, mais incomplet : deux attentes **de champ** les ont
remplacées (amendement `6d28f59d` du plan 006) —

- `SkillsMirror::agent` (l. 44-52) : `reason = "servi par la route, affiché par aucune des quatre surfaces"` ;
- `SkillMirror::origin` (l. 79-87) : même `reason`.

### `crates/mika-cli/CLAUDE.md`

87 145 octets pour un plafond de 100 000 (`crates/mika-common/tests/claude_md_size.rs`
l. 40). Deux passages y sont faux depuis A1 :

- **l. 755** : la garde de V3b-1 « passe à deux sites (`chat.rs`, `ask.rs`) et **huit**
  motifs » — elle en a douze.
- **l. 760** : le titre « La route des skills a son client, pas encore son appelant
  (mika#1727, vols V3c-2a-ii et V3c-2a-iii) ». Elle a son appelant depuis A1, et le
  corps de la section (l. 762-775) le dit déjà par une incise — le titre le contredit.

---

## Périmètre

**Seuls ces quatre fichiers changent :**

| fichier | ce qui change | moitié |
|---|---|---|
| `crates/mika-cli/src/tui/app.rs` | la garde neuve, sa détection, son témoin et ses deux contre-témoins | **test seulement** |
| `crates/mika-cli/src/commands/chat.rs` | la seule chaîne de raison du motif `skills_dirty` | test (table `MOTIFS`) |
| `crates/mika-cli/src/commands/skills_remote.rs` | la seule phrase d'en-tête sur les attentes `dead_code` | `//!` |
| `crates/mika-cli/CLAUDE.md` | deux sections, et rien d'autre | doc |

**Restent intacts** : tout le code de production de `crates/mika-cli/`, tout
`crates/mika-agent/`, `crates/mika-common/`, `crates/mika-a2a/`, `crates/mika-gateway/`,
`docs/egress/` et `dashboard/`. **Aucun fichier du plan de contrôle** : ni
`.github/workflows/`, ni `.claude/`, ni `skills/bundled/`, ni
`crates/mika-agent/src/well_known_agents.rs`, ni `.mika/`, ni aucun chemin de CODEOWNERS.

---

## La garde neuve

### Le fichier hôte : `tui/app.rs`, et pourquoi celui-là

La garde balaie **le dossier**, pas un fichier : son hôte n'a donc aucun effet sur ce
qu'elle couvre. Il en a un sur ce qu'elle atteste. `tui/app.rs` est retenu pour trois
raisons mesurées :

1. **C'est le fichier d'où le registre est parti.** Sur `7f2e5531`, `app.rs` portait
   cinq des sept sites de production de `SkillRegistry` dans `tui/` : l. 11 (`use`),
   332 (`updated_skills`), 578 (le champ), 690 (l'argument de `App::new`), 822
   (`SkillRegistry::empty()` du mode équipe). C'est l'arbre contre lequel le critère 4
   exige qu'elle rougisse.

   **Amendement du 2026-11-17 (pilote) : ce point disait « tous dans `app.rs` ».** Faux
   par omission — `commands/autocomplete.rs` en portait deux (l. 3 et 20), mesurés par
   la démonstration du critère 4. L'argument d'hôte ne bouge pas (cinq sites sur sept,
   dont le champ que le témoin vise), et le périmètre de la garde non plus : elle balaie
   le dossier, donc elle les couvrait déjà sans qu'on les ait nommés.
2. **Il porte la porte que le message nomme.** `app.skills: Arc<EtatSkills>` (l. 580)
   est ce que `skills_remote::lire` remplit. Le témoin de la garde vise ce champ : si
   `app.skills` disparaissait, la garde rougirait et demanderait à être re-visée — le
   bon signal, puisque le registre serait revenu par un autre chemin.
3. **Son module `tests` existe** (l. 1882-1883, `use super::*`) et n'a aucun import à
   ajouter : `mika_common::source_guard` se nomme en chemin complet, comme dans les
   deux gardes voisines du crate.

`tui/commands/handlers.rs` a été écarté : il héberge déjà `v2c1b`, une garde **par
fichier** sur son propre contenu, et y coller une garde **par dossier** ferait lire la
seconde comme une extension de la première.

### La détection, extraite

`mod tests` de `tui/app.rs` reçoit une constante et une fonction pure :

```rust
/// Le nom que la production de `tui/` n'a plus le droit de porter.
const MOTIF_REGISTRE: &str = "SkillRegistry";

/// Les lignes de `production` qui nomment [`MOTIF_REGISTRE`] hors commentaire,
/// préfixées du fichier et de leur numéro dans le fichier d'origine.
///
/// Extraite de la garde parce que son contre-témoin (critère 3) l'exerce sur
/// deux échantillons : une détection écrite inline ne se contre-témoigne qu'en
/// fabriquant un faux arbre sur le disque, et c'est alors le fabricant qu'on
/// teste, pas la détection.
fn lignes_du_registre(fichier: &str, production: &str) -> Vec<String> {
    production
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//") && l.contains(MOTIF_REGISTRE))
        .map(|(i, l)| format!("{fichier}:{}: {}", i + 1, l.trim()))
        .collect()
}
```

Le filtre et la numérotation sont **ceux de `v2c1b` et de `v3b1`**, au caractère près :
trois gardes du crate qui diffèreraient sur ce qu'est un commentaire se
contrediraient le jour où l'une d'elles rougit.

### La garde

```rust
/// La garde structurelle d'A2 : la production de `tui/` ne nomme plus le
/// registre des skills (mika#1727, vol V3c-2b-A2, critères 1 et 2).
///
/// **Par dossier, et non par fichier** — l'inverse de `v2c1b` et de `v3b1`, et
/// c'est délibéré : un fichier neuf de `tui/` est couvert sans qu'on y pense.
/// Le dossier, et non le crate, parce que `commands/skills.rs` construit
/// légitimement un `SkillRegistry` sur le disque en neuf endroits : une garde
/// par crate serait née avec neuf entrées d'allowlist, et une allowlist peuplée
/// ne refuse plus rien (mika#2309).
#[test]
fn mika1727_v3c2b_a2_la_tui_ne_nomme_plus_le_registre_des_skills() {
    // Livrée vide, et vide par construction : quand la garde rougit, le remède
    // est la porte que son message nomme, jamais une entrée ici.
    const ALLOWED: &[&str] = &[];
    assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");

    // Le témoin vise le champ que la route remplit, dans la moitié
    // **production** du fichier d'où le registre est parti. Sans lui, une garde
    // qui balaie un chemin faux — ou dont le masquage a dérapé — serait verte
    // **par cécité** (mika#2398).
    const TEMOIN_FICHIER: &str = "tui/app.rs";
    const TEMOIN: &str = "pub skills: Arc<";

    let scanner =
        mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
    let racine = scanner.src_root().join("tui");
    let fichiers = mika_common::source_guard::rust_sources_under(&racine);
    assert!(
        !fichiers.is_empty(),
        "la garde n'a lu aucun fichier sous {}",
        racine.display()
    );

    let mut temoin_vu = false;
    let mut violations: Vec<String> = Vec::new();
    for chemin in &fichiers {
        let relatif = chemin
            .strip_prefix(scanner.src_root())
            .unwrap_or(chemin)
            .display()
            .to_string();
        let production = scanner.production_of(chemin);
        if relatif == TEMOIN_FICHIER {
            temoin_vu = true;
            assert!(
                production.contains(TEMOIN),
                "la garde n'a pas lu la moitié production de {TEMOIN_FICHIER}"
            );
            // Contre-témoin de masquage : la table de ce test vit dans la moitié
            // test de ce même fichier et contient `SkillRegistry` en clair. Un
            // masquage perdu ferait donc la garde se dénoncer elle-même, et le
            // réflexe serait de « réparer » par une entrée d'allowlist —
            // précisément le piège.
            assert!(
                !production.contains("fn test_empty_assistant_row_skipped"),
                "la garde n'a pas masqué la moitié test de {TEMOIN_FICHIER}"
            );
        }
        violations.extend(lignes_du_registre(&relatif, &production));
    }
    assert!(temoin_vu, "le balayage n'a pas vu {TEMOIN_FICHIER}");

    assert!(
        violations.is_empty(),
        "mika#1727 vol V3c-2b-A2 — `SkillRegistry` revient dans la production de \
         `tui/`. La TUI lit les skills de son agent sur la route de mika-spirit : \
         la porte est `skills_remote::lire`, et ce qu'elle en rend se lit sur \
         `app.skills`. Le remède est de passer par là, jamais une entrée \
         d'allowlist :\n{}",
        violations.join("\n")
    );
}
```

**Trois façons de rougir, et c'est le critère 2** : le balayage vide
(`fichiers.is_empty()`), le fichier témoin absent du balayage (`temoin_vu`), et le
témoin absent de sa moitié production (`TEMOIN`). Les deux premières sont ce qui
distingue « rien à refuser » de « rien de lu ».

### Le contre-témoin de la détection (critère 3)

```rust
/// Contre-témoin : la détection voit le code et ignore le commentaire
/// (mika#1727, vol V3c-2b-A2, critère 3).
///
/// La **même** ligne dans les deux cas : un échantillon différent de chaque
/// côté prouverait seulement que deux lignes différentes se lisent
/// différemment.
#[test]
fn mika1727_v3c2b_a2_la_detection_distingue_le_code_du_commentaire() {
    const LIGNE: &str = "    let registre: Arc<SkillRegistry> = assemble(&dir);";

    let vues = lignes_du_registre("tui/echantillon.rs", LIGNE);
    assert_eq!(vues.len(), 1, "{vues:?}");
    assert!(vues[0].starts_with("tui/echantillon.rs:1: "), "{vues:?}");

    let en_commentaire = format!("    // {}", LIGNE.trim());
    assert!(
        lignes_du_registre("tui/echantillon.rs", &en_commentaire).is_empty(),
        "un commentaire n'est pas une violation"
    );
}
```

### La démonstration contre `7f2e5531` (critère 4)

La garde est **verte sur la branche** par construction : `git grep` ne rend aucune
ligne sous `tui/`. Ce qui reste à montrer, c'est qu'elle **rougit** sur l'arbre
d'avant A1. Le geste, dans un worktree détaché :

```
git worktree add --detach .pilot-scratch/arbre-7f2e5531 7f2e5531
```

Le pilote colle alors le bloc livré — `MOTIF_REGISTRE`, `lignes_du_registre`, les deux
`#[test]` — dans le `mod tests` de `crates/mika-cli/src/tui/app.rs` **de ce worktree**
(l. 1883), sans en adapter une seule ligne, et lance :

```
cargo test -p mika-cli --bins mika1727_v3c2b_a2 -- --nocapture
```

**Amendement du 2026-11-17 (pilote) : `--bins` est requis, et ne l'était pas dans ce
plan.** Sans lui, `cargo test -p mika-cli mika1727_v3c2b_a2` ne lance que les cibles
d'intégration et matche **zéro** test — mesuré d'abord sur la branche, puis sur l'arbre
détaché. Les tests de `tui/app.rs` vivent dans la cible binaire `mika`, et ni la lib
(`--lib` : 0 sur 90) ni les cibles d'intégration ne les portent. Une démonstration
lancée sans ce drapeau rend « 0 filtered out » et se lirait comme une garde verte.

**Rien n'est à adapter, et c'est une propriété du témoin choisi** : `pub skills: Arc<`
existe sur les deux arbres (`7f2e5531` l. 578 `Arc<SkillRegistry>`, branche l. 580
`Arc<EtatSkills>`), et `fn test_empty_assistant_row_skipped` aussi (`7f2e5531`
l. 1904). La garde passe donc ses trois témoins des deux côtés, et seule la liste des
violations diffère.

**Sortie mesurée, terme par terme** — **sept** lignes, dans **deux** fichiers :

| ligne | contenu sur `7f2e5531` |
|---|---|
| `tui/app.rs:11` | `use mika_agent::skills::SkillRegistry;` |
| `tui/app.rs:332` | `updated_skills: Option<Arc<SkillRegistry>>,` |
| `tui/app.rs:578` | `pub skills: Arc<SkillRegistry>,` |
| `tui/app.rs:690` | `skills: Arc<SkillRegistry>,` |
| `tui/app.rs:822` | `skills: Arc::new(SkillRegistry::empty()),` |
| `tui/commands/autocomplete.rs:3` | `use mika_agent::skills::SkillRegistry;` |
| `tui/commands/autocomplete.rs:20` | `pub skills: &'a SkillRegistry,` |

**Amendement du 2026-11-17 (pilote) : ce tableau annonçait cinq lignes, toutes dans
`tui/app.rs`.** Les deux dernières manquaient. La prédiction était dérivée du corps du
ticket, qui n'a mesuré `SkillRegistry` sur `7f2e5531` que dans `tui/app.rs` (l. 11, 332,
578) ; `commands/autocomplete.rs` en portait deux de plus, et le balayage **par
dossier** les voit. La garde livrée n'en change pas d'une ligne : l'écart est dans la
prédiction, pas dans le code. Et il **appuie** le choix du dossier contre la liste de
fichiers (§ *Pourquoi `app.rs`*) : une garde par fichier visant `app.rs` seul aurait
laissé passer `autocomplete.rs`, et c'est exactement le mode de panne que le dossier
supprime.

Et **aucune** pour les l. 2333 et 2357, qui nomment `SkillRegistry` dans le `mod tests`
de cet arbre (ouvert l. 1882) : elles sont masquées, et leur absence de la sortie est
ce qui atteste le masquage côté démonstration. Le contre-témoin de la détection y est
vert, le contre-témoin de masquage aussi.

Le pilote **note cette sortie dans la PR**. Le worktree détaché n'est pas supprimé :
il vit sous `.pilot-scratch/`, et il disparaît avec le worktree du vol (règle
mika#2548 — un brouillon ne se supprime jamais, même abandonné). Le premier
`cargo test` y recompile le workspace : c'est du temps, pas des tours.

---

## Les deux corrections de commentaire

### `commands/chat.rs` — la raison du motif `skills_dirty`

Le motif **reste** ; seule sa chaîne de remède change (l. 2016-2019) :

```rust
(
    "skills_dirty",
    "plus rien ne marque un registre à réassembler : la TUI n'en assemble plus",
),
```

Elle doit être vraie **et** distincte de celle d'`updated_skills`, qui dit déjà
« plus de rechargement à chaud : la route est relue au respawn » : deux motifs voisins
qui nommeraient la même porte ne diraient plus lequel a rougi.

### `commands/skills_remote.rs` — l'en-tête

Le paragraphe « État du vol » (l. 15-21) garde sa phrase sur les six attentes tombées
et nomme ce qui les a remplacées :

> Les six attentes `dead_code` que les items sans lecteur portaient sont tombées avec
> lui ; **deux attentes de champ les ont remplacées**, sur
> [`SkillsMirror::agent`] et [`SkillMirror::origin`] — servis par la route, affichés
> par aucune des quatre surfaces, et retirés du miroir ils se liraient comme absents
> de la route (motif mika#2331).

---

## La doc de `crates/mika-cli/CLAUDE.md`

**Deux sections, et rien d'autre.**

### l. 755 — le compte des motifs

« passe à deux sites (`chat.rs`, `ask.rs`) et **huit** motifs » → **douze**. Le
reste de la phrase (la liste « dont », l'allowlist vide, le témoin par site, le
contre-témoin) est déjà juste et ne bouge pas.

### l. 760 — la section de la route des skills

Le titre devient :

```
### La route des skills est la source de la TUI (mika#1727, vols V3c-2a-ii à V3c-2b)
```

Le corps dit l'état **final** de V3c-2b, sur cinq points et pas davantage :

1. **Par qui et quand.** `commands/chat.rs::lire_les_skills` (l. 136) appelle
   `skills_remote::lire(base_url, token, agent)` — nom d'agent en paramètre de requête
   encodé par reqwest, jeton interne de `session_remote::jeton_interne` en
   `Authorization`. L'appelant est `spawn_agent_worker` : au démarrage (l. 266), à la
   bascule d'agent (l. 845-859) et au `/restart` (l. 958-967). Pas de rechargement à
   chaud : la route est **relue au respawn**.
2. **Ce que porte `app.skills`.** `Arc<EtatSkills>`, soit
   `Result<SkillsMirror, String>` : le miroir des quatre clés de la route (treize
   champs par skill), ou la cause du refus **en une ligne**, figée à l'instant de la
   lecture. Une lecture en échec n'est pas fatale — la TUI démarre.
3. **Les cinq refus** de `SkillsRemoteIssue` : `Injoignable` (aucun statut, par
   construction du variant), `AgentInconnu` (404, le message nomme l'agent **demandé**,
   jamais une session), `JetonRefuse` (401, le message nomme `MIKA_INTERNAL_TOKEN`),
   `Statut` (tout autre statut que 200, rendu avec son corps), `CorpsIllisible` (un 200
   au corps illisible est un refus, jamais un miroir à champs vides).
4. **Ce que disent les quatre surfaces sur échec.** Le démarrage rend la ligne de cause
   telle quelle (`chat.rs::message_de_cause`), sans cadre qui la dirait deux fois.
   `/skills` (`handlers.rs` l. 912) et `/skill <nom>` (l. 1072) rendent la cause et
   **jamais** « No skills loaded. » ni « No skill found with name '…' » — ces deux
   phrases enverraient chercher la panne du côté des skills, la provenance fausse que
   mika#2331 refuse ; « No skills loaded. » est réservée au miroir **lu** dont les deux
   listes sont vides. L'autocomplétion de `/skill` (`completers.rs` l. 270) ne propose
   **rien** : une liste vide s'y lirait « cet agent n'a pas de skill », et la cause est
   déjà dite par les trois autres. Seuls `SkillsMirror::agent` et `SkillMirror::origin`
   gardent une attente `dead_code`, faute de surface qui les affiche.
5. **Le refus structurel (V3c-2b-A2).**
   `mika1727_v3c2b_a2_la_tui_ne_nomme_plus_le_registre_des_skills`, module `tests` de
   `tui/app.rs`, interdit `SkillRegistry` dans la moitié production de **chaque**
   fichier de `tui/`, balayé en entier — par dossier et non par fichier, pour qu'un
   fichier neuf soit couvert sans qu'on y pense ; par dossier et non par crate, parce
   que `commands/skills.rs` construit légitimement le registre sur le disque. Allowlist
   livrée vide et assertie vide, un témoin sur `app.skills`, un contre-témoin de
   masquage et un contre-témoin de détection. Quand elle rougit, le remède est
   `skills_remote::lire` par `app.skills`, jamais une ligne d'allowlist.

**Aucune autre section ne change.** Le fichier reste sous son plafond de 100 000
octets : 87 145 aujourd'hui, et ce vol y ajoute une trentaine de lignes.

---

## Ordre des commits

L'ordre est celui de la tête, et **chaque commit compile et passe ses tests**.

1. **`test(1727): la garde ferme la production de tui/ au registre des skills`**
   — `tui/app.rs` (`mod tests` seul) : `MOTIF_REGISTRE`, `lignes_du_registre`, la
   garde et son contre-témoin de détection. Verte dès ce commit, puisque main ne porte
   aucune violation. Mesure de la borne.
2. **`docs(1727): deux commentaires disent ce qui est vrai depuis A1`**
   — la raison du motif `skills_dirty` (`commands/chat.rs`) et l'en-tête
   (`commands/skills_remote.rs`). Aucun motif ne part ni n'arrive : la garde de `chat.rs`
   reste verte à douze motifs. Mesure de la borne.
3. **`docs(1727): CLAUDE.md dit l'état final de V3c-2b`**
   — les deux sections de `crates/mika-cli/CLAUDE.md`.
   `cargo test -p mika-common --test claude_md_size`. Mesure de la borne.

Puis `cargo build -p mika-cli --bin mika`, `cargo test -p mika-cli`,
`cargo clippy --all-targets -p mika-cli`, et `bash scripts/verify-pipeline.sh origin/main`
avant d'ouvrir la PR.

**La démonstration du critère 4** se fait après le commit 1 et avant la PR : elle ne
produit aucun commit (le worktree détaché est jetable), et sa sortie va dans le corps
de la PR.

**La halte du pilote.** Après chaque commit :

```
git diff --numstat origin/main...HEAD -- . ':!docs/plans' | awk '{a+=$1} END{print a}'
```

Au-dessus de **280**, le pilote s'arrête — pas de commit suivant, pas de push, pas de
PR — et son dernier message dit « HALTE borne : N lignes ajoutées pour 280 », avec le
compte après chaque commit. Cette halte est posée par Vincent ; ce plan ne la change
pas et ne la lève pas par amendement.

---

## Definition of Done

- La garde, sa détection, son témoin et ses deux contre-témoins vivent dans le module
  `tests` de `crates/mika-cli/src/tui/app.rs`. Aucun code de production n'a changé.
- La garde balaie `src/tui/` **en entier** par `rust_sources_under`, et non une liste
  de fichiers.
- `ALLOWED` est livrée vide et assertie vide dans le corps du test.
- La garde rougit sur un balayage vide, sur l'absence du fichier témoin, et sur
  l'absence du témoin dans sa moitié production.
- Son message d'échec nomme `skills_remote::lire` et `app.skills`, et dit que le remède
  n'est jamais une entrée d'allowlist.
- La démonstration contre `7f2e5531` est faite et sa sortie — les sept lignes, et
  l'absence des deux lignes de test — est recopiée dans le corps de la PR.
- La raison du motif `skills_dirty` ne dit plus que le registre s'assemble au démarrage,
  et se distingue de celle d'`updated_skills`. Le motif lui-même est toujours là, et la
  garde de `chat.rs` porte toujours douze motifs.
- L'en-tête de `skills_remote.rs` nomme `SkillsMirror::agent` et `SkillMirror::origin`.
- `crates/mika-cli/CLAUDE.md` : la section de la route des skills a un titre et un texte
  vrais après A1 et décrit la garde neuve ; la phrase du compte des motifs dit douze.
  Aucune autre section n'a changé.
- Aucune attente (`expect`) ni aucun `allow` neuf nulle part.
- `cargo build -p mika-cli --bin mika`, puis `cargo test -p mika-cli`,
  `cargo test -p mika-common --test claude_md_size` et
  `cargo clippy --all-targets -p mika-cli` sont verts.
- `bash scripts/verify-pipeline.sh origin/main` est vert. S'il est rouge, le pilote
  s'arrête et le dit.
- Le corps de la PR porte `Refs mika#1727` et aucun `Closes`, `Fixes` ni `Resolves` —
  y compris si le gabarit de reprise (`wip-rescue`) l'a ouvert, auquel cas le `Closes #N`
  qu'il écrit par défaut est corrigé **avant toute revue**, et
  `gh pr view <N> --json closingIssuesReferences` rend `[]`.
- Le corps de la PR porte `Tracked in: senara-solutions/mika#1727`.

---

## Acceptance criteria

Transcrits de la tête du ticket (§ *Critères de V3c-2b-A2*).

1. Une garde de test balaie la moitié production de chaque fichier Rust de
   `crates/mika-cli/src/tui/` et refuse `SkillRegistry`. Son allowlist est livrée vide
   et assertie vide.
2. La garde rougit si le balayage ne lit aucun fichier, ou si son témoin manque dans la
   moitié production qu'elle désigne.
3. Un contre-témoin montre que la détection rougit sur une ligne qui nomme
   `SkillRegistry` et pas sur la même ligne en commentaire.
4. La garde rougit contre l'arbre de `7f2e5531` sur les lignes de `tui/app.rs` qui
   portaient `SkillRegistry`, et passe sur la branche. La PR en donne la sortie.
5. La raison du motif `skills_dirty` dans la garde de `chat.rs` ne dit plus que le
   registre s'assemble au démarrage.
6. L'en-tête de `skills_remote.rs` nomme les deux attentes de champ qui restent.
7. Dans `crates/mika-cli/CLAUDE.md`, la section l. 760 a un titre et un texte vrais
   après A1 et décrit la garde neuve. La phrase l. 756 donne le compte juste des motifs.
   Aucune autre section ne change.
8. Aucun code de production ne change. `git diff origin/main...HEAD` ne touche que les
   fichiers listés au § *Périmètre*.
9. Aucune attente ni aucun `allow` neuf.
10. `cargo build -p mika-cli --bin mika` d'abord, puis `cargo test -p mika-cli` et
    `cargo test -p mika-common --test claude_md_size` sont verts.
    `cargo clippy --all-targets -p mika-cli` est vert. Le pilote lance
    `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il le trouve
    rouge, il s'arrête et le dit.
11. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si
    le gabarit de reprise l'ouvre.

### Comment chaque critère se mesure

| critère | mesure |
|---|---|
| 1 | `rust_sources_under(src_root().join("tui"))` dans le corps du test ; `assert!(ALLOWED.is_empty())` |
| 2 | les trois `assert!` — `!fichiers.is_empty()`, `temoin_vu`, `production.contains(TEMOIN)` |
| 3 | `mika1727_v3c2b_a2_la_detection_distingue_le_code_du_commentaire`, une seule ligne d'échantillon, deux lectures |
| 4 | worktree détaché sur `7f2e5531`, bloc collé sans adaptation, `cargo test -p mika-cli --bins mika1727_v3c2b_a2` ; sept lignes mesurées, aucune pour 2333/2357 ; sortie recopiée dans la PR |
| 5 | `git diff` sur `chat.rs` ; la chaîne neuve ne contient ni « s'assemble » ni « au démarrage » |
| 6 | `grep -n 'SkillsMirror::agent\|SkillMirror::origin' crates/mika-cli/src/commands/skills_remote.rs` rend l'en-tête |
| 7 | `git diff --stat` sur `CLAUDE.md` : deux emplacements ; `cargo test -p mika-common --test claude_md_size` |
| 8 | `git diff --name-only origin/main...HEAD` rend exactement cinq chemins, plan compris ; `git diff` sur `tui/app.rs` n'est que sous `mod tests` |
| 9 | `git diff origin/main...HEAD \| grep -c '^+.*\(expect(\|allow(\)'` rend 0 |
| 10 | les quatre commandes, dans cet ordre |
| 11 | `gh pr view <N> --json closingIssuesReferences` rend `[]` |

---

## Fire-Disposition

Ce vol livre **un** détecteur : la garde par dossier
`mika1727_v3c2b_a2_la_tui_ne_nomme_plus_le_registre_des_skills`. Ses deux
contre-témoins ne sont pas des détecteurs : ils testent la détection, et leur chemin de
succès n'est pas « aucune violation trouvée » mais « la détection voit ce qu'elle doit
voir ».

**Option retenue (mika#1574) : (c) halte-et-remontée.**

La garde est livrée **armée** et verte sur la branche : `git grep -n SkillRegistry --
crates/mika-cli/src/tui/` ne rend aucune ligne sur `3b60e2ef`, et A1 est précisément le
vol qui les a fait partir. `ALLOWED` est livrée vide **et assertie vide** dans le corps
du test. Quand elle rougira, `cargo test -p mika-cli` est rouge ; le remède est la
porte que son message nomme — router l'appelant par `skills_remote::lire` et lire
`app.skills` —, **jamais** une entrée d'allowlist (mika#2309, mika#2398, mika#2201). Si
un appelant futur ne peut pas être routé, le geste est de s'arrêter et de le déclarer à
Vincent, pas d'ouvrir la garde.

**Pourquoi pas (a) exception nommée en allowlist** : la population attendue à la
livraison est **zéro**, mesurée et non supposée. Une allowlist qui ne naît pas vide ne
mesure plus rien, et les gardes V1, V2a, V2c-1b, V2d-4a/b et V3b-1 de ce crate l'ont
toutes refusé. Les neuf sites de `commands/skills.rs` ne sont pas des exceptions : ils
sont **hors du périmètre balayé**, ce qui est la différence entre une garde visée et
une allowlist.

**Pourquoi pas (b) livrer désarmé** : la tête l'interdit nommément (« Chaque garde
arrive avec ses tests, dans le même commit »), et une garde que rien ne joue refuse en
silence (classe mika#2578 / mika#2752). Ici elle est jouée par `cargo test -p mika-cli`,
que le critère 10 exige vert, et le critère 4 exige en plus la preuve qu'elle **sait**
rougir — la démonstration contre `7f2e5531`.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `MOTIF_REGISTRE`, `lignes_du_registre` et son doc-comment (`tui/app.rs`, `mod tests`) | 16 |
| la garde, son doc-comment, le témoin et le contre-témoin de masquage (`tui/app.rs`, `mod tests`) | 45 |
| le contre-témoin de la détection (`tui/app.rs`, `mod tests`) | 14 |
| la raison du motif `skills_dirty` (`commands/chat.rs`, table `MOTIFS`) | 3 |
| l'en-tête sur les deux attentes de champ (`commands/skills_remote.rs`) | 5 |
| la section de la route des skills, titre compris (`crates/mika-cli/CLAUDE.md`) | 29 |
| le compte des motifs de la garde de `chat.rs` (`crates/mika-cli/CLAUDE.md`) | 1 |

Total estimé : 113 lignes

Estimation hors `docs/` et hors `CLAUDE.md` : 83 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

### Les bornes, et laquelle mord

| unité | estimé | borne | verdict |
|---|---|---|---|
| `git diff --numstat origin/main...HEAD`, plan exclu | **113** | 200 (borne du groom, tête du vol) | tient, 87 de marge |
| même unité, à la mesure du pilote | 113 | 280 (halte du pilote, tête du vol) | tient, 167 de marge |
| estimation hors `docs/` et hors `CLAUDE.md` | 83 | 429 (mika#2693) | tient, 346 de marge |

**La borne qui mord est celle du groom, 200**, posée par Vincent dans la tête de ce vol
(« Si ce total dépasse 200, le groom s'arrête et le dit. Il ne recoupe pas lui-même »).
Ce plan la cite et la respecte : 113 pour 200. Les deux autres sont larges.

`Total estimé :` porte l'unité de la tête — **la ligne ajoutée**, colonne 1 de
`git diff --numstat`, plan exclu, code, commentaires, doc et tests compris. Les retraits
ne comptent pas ; il n'y en a quasiment aucun ici, puisque ce vol ajoute du test et
réécrit trois passages de texte.

**Pourquoi ce vol tient en un seul vol** : la coupe est déjà faite. A2 est le second des
deux sous-vols de la coupe A, A1 est mergé (`3b60e2ef`), et la suite — le test É1 en
V3c-2c, puis V4 — est nommée hors périmètre par la tête et par la ligne
`Tracked in: senara-solutions/mika#1727`.

**Les tests sont comptés** : 75 des 83 lignes hors docs. La garde **est** son test, et
le contre-témoin arrive dans le même commit qu'elle. Aucun ne se reporte à un vol
suivant.

**Les commentaires sont comptés et ne se raboteront pas pour tenir.** Les 113 lignes
incluent le pourquoi de l'extraction de `lignes_du_registre` (pourquoi la détection ne
peut pas rester inline), celui du choix dossier-plutôt-que-crate, celui du témoin sur
`app.skills`, celui du contre-témoin de masquage (pourquoi la table se dénoncerait
elle-même) et celui de la ligne unique du contre-témoin de détection. Aucune n'est une
paraphrase du code.

**Lignes jetables : zéro.** Rien de ce que ce vol pose n'est livré pour être retiré au
vol suivant.

### Pourquoi 113, et non les 90 à 120 de la tête

La tête estime 50 à 70 pour la garde et ses deux témoins, 6 pour les deux commentaires,
30 à 45 pour la doc, soit 86 à 121. Ce plan chiffre 75, 8 et 30 : il est **dans** la
fourchette, haut sur la garde et bas sur la doc, pour deux raisons mesurées :

- **La garde à 75 et non 60** parce qu'elle porte **trois** témoins et non un : la
  non-vacuité du balayage, la présence du fichier témoin dans le balayage, et le témoin
  de production — le critère 2 en exige deux, et le troisième (`temoin_vu`) est ce qui
  empêche le second d'être vacuement vrai. S'y ajoute le contre-témoin de masquage, que
  la tête ne compte pas séparément mais que le fichier hôte rend indispensable : la
  table de ce test contient `SkillRegistry` en clair.
- **La doc à 30 et non 45** parce que le corps de la section l. 762-775 est **déjà**
  juste sur les cinq refus et sur le client : A1 l'a écrit. Ce qui change est le titre,
  l'incise sur l'appelant qui devient le sujet, les quatre surfaces sur échec (que la
  section ne disait pas) et le paragraphe de la garde neuve. Le reste est réécrit sur
  place, donc compté en ajouté une fois.
