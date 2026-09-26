//! La garde d'exhaustivité de la liste canonique des jetons machine (mika#2201).
//!
//! # La borne de fermeture de Prime, et pourquoi elle demande un scan de source
//!
//! mika#2201 pose une exigence que le lint seul ne satisfait pas : « la liste
//! canonique doit être EXHAUSTIVE contre le code qui matche […] Un jeton
//! oublié, et "seconde passe" revient sous un autre nom. »
//!
//! La régression que AC4 vise ne rend **aucune décision fausse** : elle rend un
//! site **invisible à la liste**. Toutes les assertions de comportement
//! resteraient vertes pendant que l'exhaustivité se perd. C'est exactement la
//! raison que mika#2131 a déjà dû écrire pour sa propre garde, et c'est
//! pourquoi ceci est un scan de source plutôt qu'un test comportemental.
//!
//! # Deux moitiés, deux directions, et elles ne se recouvrent PAS
//!
//! `scripts/canonical-tokens-survey.sh --check` part des **formes de lecture** :
//! il énumère `Regex::new` / `starts_with` / `strip_prefix` / `match_indices` /
//! `grep` / `sed -n` / une constante nommée `*_MARKER|_PREFIX|_KEY|_LINE`, et
//! exige que chaque site trouvé soit déclaré.
//!
//! [`tests::mika2201_every_match_site_is_declared`] part des **jetons** : il
//! prend les littéraux de classe B du TSV et exige que tout fichier de
//! production qui en porte un soit cité dans la colonne `site de match`.
//!
//! La composition est ce qui ferme le trou que chacune laisse ouvert. Le survey
//! ne voit pas un site qui lirait `GROOMED` par `contains` — forme
//! **délibérément** exclue de son périmètre, parce qu'elle est la forme *lâche*
//! (le test de sous-chaîne `docs/plans/` d'`executor`) et que l'y admettre
//! rangerait la moitié tolérante d'une asymétrie assumée dans l'inventaire
//! strict. Ce scan-ci voit ce site, parce qu'il cherche le jeton et non la
//! forme. Symétriquement, le scan ne voit pas un site dont le jeton est neuf ;
//! le survey le voit.
//!
//! # « On déclare, on n'allowliste pas »
//!
//! L'allowlist est **livrée vide** et le reste. Quand le scan tire, la
//! résolution est une ligne de `scripts/canonical-tokens.tsv` — jamais une
//! entrée d'exemption. Un site de match qu'on ne veut pas déclarer est un site
//! de match qu'il faut supprimer. C'est la discipline de
//! `mika2323_no_gate_predicate_reads_the_actor` et de
//! `mika1883_run_usage_accumulates_only_via_the_one_helper`, et elle n'a pas de
//! cas particulier ici.
//!
//! Le fichier d'exceptions `scripts/canonical-tokens-exceptions.tsv` existe pour
//! le **lint** (une accusation sur un texte préexistant), jamais pour cette
//! garde-ci.
//!
//! # Author ≠ outside-check
//!
//! L'exigence Prime sépare l'écriture du lint de la vérification d'exhaustivité.
//! Cette passe ne peut pas être verte par accord avec l'auteur du plan : elle
//! lit l'arbre, pas le plan. C'est aussi pourquoi le TSV est **produit** par le
//! survey puis vérifié ici, dans cet ordre — l'inverse ferait passer la garde
//! pour verte par accord avec son propre producteur.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    /// Racine du dépôt, depuis `crates/mika-agent`.
    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .canonicalize()
            .expect("la garde doit pouvoir résoudre la racine du dépôt")
    }

    fn tsv_path() -> PathBuf {
        repo_root().join("scripts").join("canonical-tokens.tsv")
    }

    struct Row {
        token: String,
        class: String,
        file: String,
        symbol: String,
    }

    /// Lit `scripts/canonical-tokens.tsv`.
    ///
    /// Un TSV illisible ou vide **échoue** plutôt que de rendre une liste vide :
    /// une garde qui ne trouve rien à vérifier et se tait est la décoration que
    /// mika#2103 a dû nommer.
    fn read_rows() -> Vec<Row> {
        let path = tsv_path();
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("la garde doit pouvoir lire {}: {e}", path.display()));

        let mut rows = Vec::new();
        for line in content.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            assert!(
                fields.len() >= 4,
                "ligne du TSV à moins de quatre colonnes — les quatre sont obligatoires \
                 (jeton, classe, site de match, tolérance) : {line:?}"
            );
            let site = fields[2];
            let (file, symbol) = site.split_once("::").unwrap_or_else(|| {
                panic!(
                    "un site est désigné `chemin::symbole`, jamais un numéro de ligne \
                     (qui pourrit en silence au premier commit qui insère une ligne \
                     au-dessus) : {site:?}"
                )
            });
            rows.push(Row {
                token: fields[0].to_string(),
                class: fields[1].to_string(),
                file: file.to_string(),
                symbol: symbol.to_string(),
            });
        }

        assert!(
            !rows.is_empty(),
            "le TSV n'a produit aucune ligne — une garde qui ne vérifie rien est une décoration"
        );
        rows
    }

    /// Énumère les fichiers `.rs` de production sous `crates/*/src`.
    ///
    /// Le code de test est écarté par [`crate::source_scan::is_test_source_path`]
    /// **et** par troncature au premier `#[cfg(test)]` : une fixture porte
    /// légitimement chacun des jetons que cette garde cherche, et la compter
    /// ferait de l'inventaire un recensement de sa propre suite de tests.
    fn production_sources() -> Vec<(String, String)> {
        let crates_dir = repo_root().join("crates");
        let mut out = Vec::new();
        let mut stack = vec![crates_dir.clone()];

        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries {
                let path = entry.expect("entrée de répertoire lisible").path();
                if path.is_dir() {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    // `target/` est un artefact de build, pas de la source.
                    if name == "target" {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                if crate::source_scan::is_test_source_path(&path) {
                    continue;
                }
                // Seule la source sous un `src/` compte : `build.rs`, `tests/`
                // et `benches/` ne portent pas de site de match de production.
                let rel = path
                    .strip_prefix(repo_root())
                    .expect("chemin sous la racine")
                    .to_string_lossy()
                    .replace('\\', "/");
                if !rel.contains("/src/") {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let production = match content.find("#[cfg(test)]") {
                    Some(i) => content[..i].to_string(),
                    None => content,
                };
                out.push((rel, production));
            }
        }

        assert!(
            !out.is_empty(),
            "aucune source de production trouvée sous crates/*/src — un scan qui ne scanne \
             rien est un laissez-passer vide, pas un scan propre (mika#2103)"
        );
        out
    }

    /// Les formes de lecture que cette garde reconnaît.
    ///
    /// Les cinq du plan (§ M5), dont **`contains`** — la forme que le survey
    /// exclut délibérément de son périmètre parce qu'elle est la forme *lâche*.
    /// C'est précisément ce que cette moitié-ci apporte : `executor.rs` lit le
    /// callout `Plan` par sous-chaîne, et le survey ne le voit pas.
    const READING_FORMS: &[&str] = &[
        "Regex::new(",
        ".starts_with(",
        ".strip_prefix(",
        ".contains(",
        ".match_indices(",
    ];

    /// Les littéraux de chaîne d'une ligne, guillemets exclus.
    ///
    /// Chercher le jeton dans le **littéral** et non dans la ligne entière est
    /// ce qui sépare un site de match du nom d'une constante :
    /// `const PR_OPENED_PREFIX: &str = "[GitHub] PR opened…"` porte `PR_OPENED`
    /// dans son NOM, et l'accuser serait accuser la déclaration d'être son
    /// propre lecteur.
    fn string_literals(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '"' {
                continue;
            }
            let mut buf = String::new();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        buf.push(next);
                    }
                    continue;
                }
                if c == '"' {
                    break;
                }
                buf.push(c);
            }
            out.push(buf);
        }
        out
    }

    /// Les lignes de production d'un fichier qui portent une forme de lecture.
    ///
    /// Les commentaires sont écartés : un doc-comment qui **décrit** un site de
    /// match n'en est pas un, et l'en-tête de module de `grooming_marker.rs`
    /// cite ses quatre regex. Les compter ferait rougir la garde sur la
    /// documentation qu'elle protège.
    fn reading_lines(src: &str) -> Vec<&str> {
        src.lines()
            .filter(|l| {
                let t = l.trim_start();
                !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
            })
            .filter(|l| READING_FORMS.iter().any(|f| l.contains(f)))
            .collect()
    }

    /// **Livrée vide, et elle le reste.**
    ///
    /// Quand cette garde tire, on déclare le site dans
    /// `scripts/canonical-tokens.tsv` ; on ne l'allowliste pas. Une allowlist
    /// née vide est un emplacement où déposer la prochaine infraction
    /// (mika#2323).
    const ALLOWED_UNDECLARED_SITES: &[&str] = &[];

    /// Les jetons trop courts ou trop communs pour être cherchés par
    /// sous-chaîne dans de la source Rust.
    ///
    /// `STATUS`, `DEPTH`, `VERDICT`, `READY`, `MERGE-READY-SIGNAL` : chacun
    /// apparaît dans des dizaines d'identifiants (`TaskStatus`, `DEPTH_RE`,
    /// `VerdictAction`, `is_ready`) que cette garde n'a aucune raison
    /// d'accuser. Ce n'est PAS une allowlist de sites — c'est une borne sur ce
    /// qu'une recherche par sous-chaîne peut décider, et elle est nommée pour
    /// que personne ne la confonde avec la première.
    ///
    /// Ces jetons restent couverts par l'autre moitié, `--check`, qui part de
    /// la forme de lecture et ne dépend d'aucun seuil de longueur.
    fn is_searchable_token(token: &str) -> bool {
        // Un jeton composé ou ponctué est assez spécifique pour être cherché ;
        // un mot unique court ne l'est pas.
        token.len() >= 12
            || token.contains(' ')
            || token.contains('*')
            || token.contains('[')
            || token.contains('-')
            || token.contains('_')
    }

    /// AC4 — tout fichier de production portant un jeton de classe B est cité
    /// dans la colonne `site de match` du TSV.
    #[test]
    fn mika2201_every_match_site_is_declared() {
        let rows = read_rows();

        // Fichiers déclarés, par jeton. Un jeton peut avoir plusieurs sites.
        let mut declared: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for row in &rows {
            if row.class == "B" {
                declared
                    .entry(row.token.clone())
                    .or_default()
                    .insert(row.file.clone());
            }
        }
        assert!(
            !declared.is_empty(),
            "aucun jeton de classe B — le lint n'aurait rien à accuser"
        );

        let sources = production_sources();
        let mut offenders: Vec<String> = Vec::new();
        let mut searched = 0usize;

        for (token, files) in &declared {
            if !is_searchable_token(token) {
                continue;
            }
            searched += 1;
            for (rel, content) in &sources {
                if files.contains(rel) {
                    continue;
                }
                if ALLOWED_UNDECLARED_SITES.contains(&rel.as_str()) {
                    continue;
                }
                let carries = reading_lines(content).iter().any(|line| {
                    string_literals(line)
                        .iter()
                        .any(|lit| lit.contains(token.as_str()))
                });
                if carries {
                    offenders.push(format!(
                        "{rel} lit le jeton de classe B {token:?} sans être déclaré"
                    ));
                }
            }
        }

        assert!(
            searched > 0,
            "aucun jeton cherchable — le scan n'a rien vérifié (mika#2103)"
        );

        assert!(
            offenders.is_empty(),
            "des sites de match ne sont pas déclarés dans scripts/canonical-tokens.tsv :\n  {}\n\n\
             RÉSOLUTION : déclarer le site dans le TSV (colonne `site de match`, \
             format `chemin::symbole`), et non l'allowlister. Un site de match qu'on ne veut \
             pas déclarer est un site de match qu'il faut supprimer (mika#2201 § D5/D6).\n\
             Le relevé se produit avec `scripts/canonical-tokens-survey.sh --tsv`.",
            offenders.join("\n  ")
        );
    }

    /// Le pendant de la garde ci-dessus pour la **classe A**.
    ///
    /// Le survey énumère les formes de lecture STRICTES, donc il ne peut pas
    /// découvrir un lecteur tolérant : une alternation `(?i)` ou un tier fuzzy
    /// lui sont invisibles, et `--check` ne compare donc pas la classe A dans le
    /// sens « périmée ». Sans cette garde-ci, une ligne de classe A pourrait
    /// survivre au symbole qu'elle nomme, en silence — et la classe A est
    /// précisément ce qui empêche un futur auteur de ré-accuser
    /// « seconde passe » (§ R1).
    ///
    /// Elle borne la pourriture sans prétendre à la découverte : c'est la limite
    /// écrite en tête de `canonical-tokens.tsv`, tenue plutôt que promise.
    #[test]
    fn mika2201_every_declared_symbol_still_exists() {
        let rows = read_rows();
        let root = repo_root();
        let mut offenders = Vec::new();
        let mut checked = 0usize;

        for row in &rows {
            // Les libellés ne sont pas de la source ; leur vocabulaire a sa
            // propre garde (`scripts/check-dispatch-seats-declared.sh` pour la
            // famille `dispatch:`, la règle L5 du lint pour le reste).
            if row.file == ".github/labels.yml" {
                continue;
            }
            let path = root.join(&row.file);
            let Ok(content) = std::fs::read_to_string(&path) else {
                offenders.push(format!(
                    "{}::{} — le fichier est illisible ou absent",
                    row.file, row.symbol
                ));
                continue;
            };
            checked += 1;
            if row.symbol == "<file-scope>" {
                continue;
            }
            if !content.contains(&row.symbol) {
                offenders.push(format!(
                    "{}::{} — le symbole n'existe plus dans ce fichier",
                    row.file, row.symbol
                ));
            }
        }

        assert!(
            checked > 0,
            "aucun site vérifié — le scan n'a rien lu (mika#2103)"
        );
        assert!(
            offenders.is_empty(),
            "des sites déclarés ne désignent plus rien :\n  {}\n\n\
             RÉSOLUTION : re-produire le relevé \
             (`scripts/canonical-tokens-survey.sh --tsv`) et corriger la ligne. \
             Un site qui survit à son symbole est une ligne qui ment avec l'autorité \
             d'un inventaire.",
            offenders.join("\n  ")
        );
    }

    /// Le TSV ne désigne jamais un site par un numéro de ligne.
    ///
    /// Un numéro pourrit au premier commit qui insère une ligne au-dessus, et il
    /// pourrit **en silence** : la colonne resterait syntaxiquement valide en
    /// désignant autre chose. [`read_rows`] refuse déjà un site sans `::` ; ce
    /// test refuse la forme `chemin::1234`, qui en porte un.
    #[test]
    fn mika2201_a_site_is_never_a_line_number() {
        let rows = read_rows();
        let numeric: Vec<&str> = rows
            .iter()
            .filter(|r| r.symbol.chars().all(|c| c.is_ascii_digit()))
            .map(|r| r.symbol.as_str())
            .collect();
        assert!(
            numeric.is_empty(),
            "des sites sont désignés par un numéro de ligne : {numeric:?} — \
             le format est `chemin::symbole` (mika#2201 § M1.0)"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2242 — les deux noms d'audit du dé-groomage ont un writer chacun.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test en dessous l'assert.**
    ///
    /// Quand ce scan tire, c'est qu'un **second** site écrit l'un des deux noms
    /// — donc que la propriété SOLE WRITER dont dépendent les requêtes SQL de
    /// `CLAUDE.md` est fausse. La résolution est de **renommer** ce second site,
    /// jamais de l'excepter : une exception rendrait les `GROUP BY` de
    /// l'opérateur silencieusement faux, ce qui est strictement pire que le
    /// silence que mika#2242 remplace.
    const SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// Les deux noms, et le module de production qui a le droit de les écrire.
    ///
    /// Les aiguilles sont composées à l'exécution pour que **ce fichier ne se
    /// dénonce pas lui-même** — le motif de `worktree_reaper.rs`.
    fn degroom_audit_names() -> [(String, &'static str); 2] {
        [
            (
                format!("closing_pr{}", "_closed_unmerged"),
                "crates/mika-agent/src/server/upstream_close_handler.rs",
            ),
            (
                format!("ready_label{}", "_degroomed"),
                "crates/mika-agent/src/server/ready_label_handler.rs",
            ),
        ]
    }

    /// Un nom d'audit qui a deux écrivains rend une requête opérateur inexacte
    /// **sans rien casser** — aucune décision ne devient fausse, seules les deux
    /// populations cessent d'être soustractibles. C'est la classe qu'aucun test
    /// comportemental ne peut voir, d'où un scan de source.
    #[test]
    fn mika2242_the_two_audit_names_have_a_single_writer() {
        let names = degroom_audit_names();
        let sources = production_sources();
        let mut offenders = Vec::new();
        let mut witnesses = 0usize;

        for (needle, owner) in &names {
            let mut writers = Vec::new();
            for (rel, content) in &sources {
                if SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                    continue;
                }
                // Le nom cherché dans les LITTÉRAUX, jamais dans la ligne
                // entière : la déclaration `const CLOSING_PR_CLOSED_UNMERGED_TOOL`
                // porte le nom dans son IDENTIFIANT, et l'accuser reviendrait à
                // accuser la déclaration d'être son propre second writer.
                let carries = content
                    .lines()
                    .filter(|l| {
                        let t = l.trim_start();
                        !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                    })
                    .any(|line| {
                        string_literals(line)
                            .iter()
                            .any(|lit| lit.contains(needle.as_str()))
                    });
                if carries {
                    writers.push(rel.clone());
                }
            }

            // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement
            // comme un scan propre (mika#2103 / mika#2205). Le propriétaire doit
            // être trouvé, sans quoi la garde est décorative.
            assert!(
                writers.iter().any(|w| w == owner),
                "mika#2242 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
                 vise un nom mort, il ne vérifie rien"
            );
            witnesses += 1;

            for w in writers.iter().filter(|w| *w != owner) {
                offenders.push(format!("{needle} est aussi écrit par {w} (owner: {owner})"));
            }
        }

        assert_eq!(witnesses, names.len(), "un nom n'a pas été vérifié");
        assert!(
            offenders.is_empty(),
            "mika#2242 — chacun de ces deux noms d'audit doit avoir UN SEUL site \
             d'écriture en production :\n  {}\n\n\
             RÉSOLUTION : renommer le second site. Ne PAS l'ajouter à \
             SOLE_WRITER_EXCEPTIONS — les deux populations (toute fermeture \
             unmerged, vs. celles qui ont effectivement reflué en groom) ne sont \
             soustractibles que tant que chaque nom a un écrivain.",
            offenders.join("\n  ")
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2484 — un seul lecteur décisionnel de la preuve de grooming, et
    // deux noms d'audit à écrivain unique.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// La population a été relevée avant rédaction : `has_completed_groom_for_issue`
    /// avait exactement **un** appelant décisionnel, `evaluate_grooming_gate`,
    /// que mika#2484 remplace par `groomed_state`. Quand ce scan tire, **la
    /// résolution est de retirer la lecture**, jamais d'ajouter une entrée
    /// (doctrine mika#2201, « on déclare, on n'allowliste pas ») : c'est
    /// exactement la divergence que mika#2158 a dû graver une fois, où deux
    /// sites répondaient différemment à la même question pendant des mois sans
    /// qu'aucune assertion ne rougisse.
    const GROOM_PROOF_READERS_ALLOWED: &[&str] = &[];

    /// Les deux fichiers d'**infrastructure** de la preuve : la définition SQL
    /// et son enveloppe asynchrone. Ni l'un ni l'autre ne *décide* quoi que ce
    /// soit — ils transportent. Les accuser reviendrait à interdire à la
    /// fonction d'exister.
    const GROOM_PROOF_PLUMBING: &[&str] = &[
        "crates/mika-agent/src/db/tasks.rs",
        "crates/mika-agent/src/async_db.rs",
    ];

    /// **Test 11 / R2 — un seul lecteur décisionnel de la preuve.**
    ///
    /// Deux niveaux, et le second est le porteur. (a) hors plomberie, le seul
    /// fichier de production qui *appelle* la preuve est `skills/executor.rs` ;
    /// (b) dans ce fichier, la seule fonction qui la lit est `groomed_state`.
    ///
    /// Aucun test comportemental ne peut voir cette classe : un second lecteur
    /// ne rendrait aucune décision fausse **le jour où il est écrit**. Il
    /// divergerait plus tard, en silence, avec toutes les assertions vertes —
    /// ce qui est littéralement ce qui est arrivé entre l'étape 5 du handler et
    /// sa porte 9d entre mika#1620 et mika#2484.
    #[test]
    fn mika2484_un_seul_lecteur_decisionnel_de_la_preuve() {
        // Composée à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("has_completed{}", "_groom_for_issue");
        let owner = "crates/mika-agent/src/skills/executor.rs";

        let mut callers = Vec::new();
        for (rel, content) in production_sources() {
            if GROOM_PROOF_PLUMBING.contains(&rel.as_str())
                || GROOM_PROOF_READERS_ALLOWED.contains(&rel.as_str())
            {
                continue;
            }
            let reads = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .any(|line| line.contains(&needle));
            if reads {
                callers.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme
        // un scan propre (mika#2103 / mika#2205).
        assert!(
            callers.iter().any(|c| c == owner),
            "mika#2484 — `{needle}` n'est lue nulle part dans {owner} : ce scan vise \
             un mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = callers.iter().filter(|c| *c != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2484 — la preuve de grooming a un second lecteur : {strangers:?}\n\n\
             RÉSOLUTION : retirer la lecture et passer par `groomed_state`. Ne PAS \
             l'ajouter à GROOM_PROOF_READERS_ALLOWED — deux lecteurs, c'est la \
             divergence de mika#2158 rouverte, et elle est invisible aux tests."
        );

        // (b) Dans le fichier propriétaire, une seule fonction lit la preuve.
        //     Le contenu est celui que `production_sources` a déjà lu — une
        //     seconde lecture disque du même fichier n'apporterait rien.
        let src = callers
            .iter()
            .find(|c| *c == owner)
            .and_then(|_| {
                production_sources()
                    .into_iter()
                    .find(|(rel, _)| rel == owner)
                    .map(|(_, content)| content)
            })
            .expect("le propriétaire vient d'être trouvé dans production_sources");
        let production = match src.find("\n#[cfg(test)]\nmod tests {") {
            Some(i) => &src[..i],
            None => &src[..],
        };
        let readers: Vec<String> = crate::source_scan::fn_bodies(production)
            .into_iter()
            .filter(|(_, body)| body.contains(&needle))
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            readers,
            vec!["groomed_state".to_string()],
            "mika#2484 — la preuve doit être lue par `groomed_state` et par elle \
             seule ; `evaluate_grooming_gate` et le routage du ready-label en \
             descendent. Trouvé : {readers:?}"
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist du test 11.
    #[test]
    fn mika2484_l_allowlist_du_lecteur_unique_est_livree_vide() {
        assert!(
            GROOM_PROOF_READERS_ALLOWED.is_empty(),
            "GROOM_PROOF_READERS_ALLOWED est livrée vide et doit le rester : quand le \
             scan tire, on RETIRE la lecture. Une allowlist née vide est un \
             emplacement où déposer la prochaine infraction (mika#2323)."
        );
    }

    /// **Test 12 — les deux noms d'événement du routage sont un format de fil.**
    ///
    /// Ils atterrissent dans `audit_events.tool_name` et l'opérateur en fait des
    /// `GROUP BY` (sondes S1 et S4). Deux écrivains rendraient les deux
    /// populations — « groomé hors moteur » et « la base ne répond pas » — non
    /// soustractibles, c'est-à-dire feraient lire une panne comme un succès du
    /// correctif. Septième emploi du motif après `phantom_aged_out` /
    /// `phantom_sweep_spared` (mika#2156).
    #[test]
    fn mika2484_les_noms_d_evenement_sont_un_format_de_fil() {
        let owner = "crates/mika-agent/src/server/ready_label_handler.rs";
        let names = [
            format!("ready_label{}", "_markers_without_proof"),
            format!("ready_label{}", "_groom_proof_unreadable"),
        ];
        let sources = production_sources();
        let mut offenders = Vec::new();

        for needle in &names {
            let mut writers = Vec::new();
            for (rel, content) in &sources {
                // Le nom cherché dans les LITTÉRAUX : la déclaration
                // `const READY_LABEL_…_TOOL` le porte dans son IDENTIFIANT, et
                // l'accuser reviendrait à accuser la déclaration d'être son
                // propre second écrivain.
                let carries = content
                    .lines()
                    .filter(|l| {
                        let t = l.trim_start();
                        !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                    })
                    .any(|line| {
                        string_literals(line)
                            .iter()
                            .any(|lit| lit.contains(needle.as_str()))
                    });
                if carries {
                    writers.push(rel.clone());
                }
            }
            assert!(
                writers.iter().any(|w| w == owner),
                "mika#2484 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
                 vise un nom mort"
            );
            for w in writers.iter().filter(|w| *w != owner) {
                offenders.push(format!("{needle} est aussi écrit par {w}"));
            }
        }

        assert!(
            offenders.is_empty(),
            "mika#2484 — chacun de ces deux noms doit avoir UN SEUL site \
             d'écriture en production :\n  {}\n\n\
             RÉSOLUTION : renommer le second site. Les deux populations ne sont \
             comptables séparément que tant que chaque nom a un écrivain.",
            offenders.join("\n  ")
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2242_the_sole_writer_allowlist_is_empty() {
        assert!(
            SOLE_WRITER_EXCEPTIONS.is_empty(),
            "SOLE_WRITER_EXCEPTIONS est livrée vide et doit le rester : quand le scan \
             tire, on renomme le second écrivain. Une allowlist née vide est un \
             emplacement où déposer la prochaine infraction (mika#2323)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2498 — le nom de journal du refus d'auto-fire a un écrivain.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Rien à excepter à la livraison, et c'est vérifiable : le nom
    /// `groom_pilot_autofire_stopped` est **neuf**, donc aucune infraction
    /// préexistante ne peut exister. Quand ce scan tire, **on retire le second
    /// écrivain**, on ne l'excepte pas (doctrine mika#2201, « on déclare, on
    /// n'allowliste pas ») : une exception rendrait le grep opérateur du § 9 du
    /// plan silencieusement faux, ce qui est strictement pire que le silence
    /// qu'il remplace.
    const GROOM_PILOT_STOPPED_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// **Test 6 / AC7 — un seul écrivain du nom de journal du refus.**
    ///
    /// Aucun test comportemental ne peut voir cette classe : un second écrivain
    /// ne rendrait **aucune décision fausse**, il rendrait seulement les deux
    /// populations — « le frein a refusé un dispatch » et tout le reste — non
    /// soustractibles. Même motif et même raison que
    /// `mika2242_the_two_audit_names_have_a_single_writer` ci-dessus.
    #[test]
    fn mika2498_le_nom_du_refus_a_un_seul_ecrivain() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même —
        // le motif de `worktree_reaper.rs`.
        let needle = format!("groom_pilot{}", "_autofire_stopped");
        let owner = "crates/mika-agent/src/task_engine/dispatcher.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if GROOM_PILOT_STOPPED_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            let carries = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .any(|line| {
                    string_literals(line)
                        .iter()
                        .any(|lit| lit.contains(needle.as_str()))
                });
            if carries {
                writers.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme
        // un scan propre (mika#2103 / mika#2205). Le propriétaire attendu doit
        // être trouvé, sinon la garde est décorative.
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2498 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2498 — le nom de journal du refus d'auto-fire a un second \
             écrivain : {strangers:?}\n\n\
             RÉSOLUTION : retirer le second site. Ne PAS l'ajouter à \
             GROOM_PILOT_STOPPED_SOLE_WRITER_EXCEPTIONS — la ligne ne compte les \
             refus que tant qu'un seul site l'écrit."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus. Le jour où quelqu'un y
    /// dépose une entrée, c'est ce test qui rougit — et non un `grep` qui ment
    /// des mois plus tard.
    #[test]
    fn mika2498_the_sole_writer_allowlist_is_empty() {
        assert!(
            GROOM_PILOT_STOPPED_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "GROOM_PILOT_STOPPED_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain. Une \
             allowlist née vide est un emplacement où déposer la prochaine \
             infraction (mika#2323)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2522 — le nom d'audit du tour A2A échoué a un écrivain.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test frère l'assert.**
    ///
    /// Rien à excepter à la livraison : le nom `a2a_turn_failed` est **neuf**.
    /// Quand ce scan tire, **on retire le second écrivain**, on ne l'excepte pas
    /// (doctrine mika#2201).
    const TURN_FAILURE_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// Le nom d'audit du tour A2A échoué n'a qu'un écrivain (mika#2522 D1).
    ///
    /// C'est ce qui rend
    /// `SELECT target_key, after_value, count(*) … WHERE tool_name = 'a2a_turn_failed'`
    /// **exact** plutôt qu'un nombre sur lequel deux sites peuvent diverger — et
    /// ce `GROUP BY` est le livrable d'AC2 : quel modèle perd des tours, et sur
    /// quelle classe. Un second écrivain ne rendrait aucune décision fausse ; il
    /// rendrait ce compte faux, en silence, avec toutes les assertions au vert.
    /// Aucun test comportemental ne voit cette classe — d'où un scan de source.
    #[test]
    fn mika2522_the_turn_failure_name_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("a2a_turn{}", "_failed");
        let owner = "crates/mika-agent/src/server/a2a.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if TURN_FAILURE_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            let carries = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .any(|line| {
                    string_literals(line)
                        .iter()
                        .any(|lit| lit.contains(needle.as_str()))
                });
            if carries {
                writers.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme
        // un scan propre (mika#2103 / mika#2205).
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2522 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2522 — le nom d'audit du tour A2A échoué a un second écrivain : \
             {strangers:?}\n\n\
             RÉSOLUTION : retirer le second site. Ne PAS l'ajouter à \
             TURN_FAILURE_SOLE_WRITER_EXCEPTIONS — le `GROUP BY` par modèle que \
             ce nom existe pour servir n'est exact que tant qu'un seul site l'écrit."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2522_the_sole_writer_allowlist_is_empty() {
        assert!(
            TURN_FAILURE_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "TURN_FAILURE_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain. Une \
             allowlist née vide est un emplacement où déposer la prochaine \
             infraction (mika#2323)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2496 — le nom d'audit du dépassement de coût a un écrivain.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Rien à excepter à la livraison, et c'est vérifiable : le nom
    /// `pilot_cost_overrun` est **neuf**. Quand ce scan tire, **on retire le
    /// second écrivain**, on ne l'excepte pas (doctrine mika#2201).
    const PILOT_COST_OVERRUN_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// **Un seul écrivain du nom d'audit du dépassement de coût (mika#2496 U4).**
    ///
    /// La propriété est porteuse pour une raison précise : la requête opérateur
    /// publiée dans `CLAUDE.md` —
    /// `SELECT count(*), avg(after_value) … WHERE tool_name = 'pilot_cost_overrun'`
    /// — **est** la précondition explicite du ticket de suivi sur
    /// `senara-solutions/claude-pilot`, qui doit dimensionner le frein dollars
    /// manquant. Un second écrivain ne rendrait aucune décision fausse ; il
    /// rendrait ce compte inexact, et le ticket s'ouvrirait sur un nombre que
    /// personne ne pourrait départager. Aucun test comportemental ne voit cette
    /// classe — d'où un scan de source.
    #[test]
    fn mika2496_the_cost_overrun_name_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("pilot_cost{}", "_overrun");
        let owner = "crates/mika-agent/src/task_engine/dispatcher.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if PILOT_COST_OVERRUN_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            let carries = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .any(|line| {
                    string_literals(line)
                        .iter()
                        .any(|lit| lit.contains(needle.as_str()))
                });
            if carries {
                writers.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme
        // un scan propre (mika#2103 / mika#2205).
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2496 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2496 — le nom d'audit du dépassement de coût a un second \
             écrivain : {strangers:?}\n\n\
             RÉSOLUTION : retirer le second site. Ne PAS l'ajouter à \
             PILOT_COST_OVERRUN_SOLE_WRITER_EXCEPTIONS — le compte qui dimensionne \
             le ticket de suivi cpp n'est exact que tant qu'un seul site l'écrit."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2496_the_sole_writer_allowlist_is_empty() {
        assert!(
            PILOT_COST_OVERRUN_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "PILOT_COST_OVERRUN_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain. Une \
             allowlist née vide est un emplacement où déposer la prochaine \
             infraction (mika#2323)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2532 — la clé sous laquelle vit la cause d'un crash pré-résultat
    // a un seul écrivain.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Zéro violation existante, et c'est vérifiable : le nom
    /// `handler_failure` est **neuf**. Il n'y a donc rien à excepter, ni de
    /// case où déposer la prochaine infraction (mika#2323). Quand le scan
    /// tire, **on retire le second site**, on ne l'allowliste pas (doctrine
    /// mika#2201).
    const HANDLER_FAILURE_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// Le nom de la clé existe **une fois**, dans la constante, et tout
    /// consommateur passe par elle.
    ///
    /// # Pourquoi un scan de source et pas un test comportemental
    ///
    /// Un second site écrivant `$.handler_failure` ne rendrait **aucune
    /// décision fausse** le jour où il est écrit : la persistance continuerait
    /// de fonctionner et toutes les assertions resteraient vertes. Ce qu'il
    /// casserait est la requête opérateur de D6 —
    /// `SELECT … WHERE json_extract(metadata,'$.handler_failure') IS NOT NULL`
    /// — qui **est** la mesure de la classe : elle cesserait de compter « un
    /// handler long-running a crashé » pour compter deux populations mêlées,
    /// en silence. C'est exactement la classe qu'aucun test de comportement ne
    /// peut voir.
    ///
    /// Le lecteur CLI (`mika tasks get`) vit dans un autre crate et importe la
    /// constante : c'est pour ça qu'elle est `pub`. Le faire porter son propre
    /// littéral aurait été la dérive `grooming_marker` (mika#2158) — deux
    /// orthographes d'un même nom, que rien n'oblige à rester d'accord.
    #[test]
    fn mika2532_the_handler_failure_key_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("handler{}", "_failure");
        let owner = "crates/mika-agent/src/task_engine/engine.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if HANDLER_FAILURE_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            let carries = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .any(|line| {
                    string_literals(line).iter().any(|lit| {
                        // Le prédicat porte sur la CLÉ, jamais sur la
                        // sous-chaîne. Deux faux positifs mesurés l'imposent,
                        // et ils vont dans les deux sens : le nom d'événement
                        // `long_running_handler_failure_not_persisted`
                        // (`executor.rs`) contient la clé sans être elle, et
                        // une prose de test qui la cite entre backticks n'est
                        // pas un site d'écriture (classe mika#2050 — une
                        // mention n'est pas une instruction). Ce qu'un second
                        // écrivain porterait réellement est le littéral nu ou
                        // un chemin JSON `$.<clé>`.
                        *lit == needle || lit.contains(&format!("$.{needle}"))
                    })
                });
            if carries {
                writers.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme
        // un scan propre (mika#2103 / mika#2205).
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2532 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2532 — la clé de metadata du crash de handler a un second \
             écrivain : {strangers:?}\n\n\
             RÉSOLUTION : faire passer ce site par \
             `task_engine::engine::HANDLER_FAILURE_METADATA_KEY`. Ne PAS l'ajouter \
             à HANDLER_FAILURE_SOLE_WRITER_EXCEPTIONS — la requête opérateur qui \
             compte la classe n'est exacte que tant qu'un seul nom existe."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2532_the_sole_writer_allowlist_is_empty() {
        assert!(
            HANDLER_FAILURE_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "HANDLER_FAILURE_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on fait passer le second site par la \
             constante. Une allowlist née vide est un emplacement où déposer la \
             prochaine infraction (mika#2323)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2517 — une définition du domaine Webhook Fallthrough, un écrivain
    // du nom de son événement.
    //
    // Les deux gardes vivent ici plutôt que dans `agent_loop::tests`, où le
    // plan les nommait : c'est le module des scans de nom, il porte déjà
    // `production_sources()` et `string_literals()`, et garder les deux gardes
    // d'un même ticket côte à côte est ce qui rend leur paire lisible. Les
    // NOMS de test du plan sont conservés — ils apparaissent dans les messages
    // de CI et sont, eux, une surface.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Population mesurée : **un** site (`webhook_dispatch.rs`). Il n'y a donc
    /// rien à excepter, ni de case où déposer la prochaine infraction
    /// (mika#2323). Quand le scan tire, **on retire le second corps**, on ne
    /// l'allowliste pas (doctrine mika#2201).
    const FALLTHROUGH_DOMAIN_DEFINITION_ALLOWED: &[&str] = &[];

    /// Les deux littéraux dont la **conjonction** définit le domaine.
    ///
    /// L'appariement est **exact**, jamais par sous-chaîne, et c'est ce qui
    /// sépare une reconstruction d'un voisin légitime :
    /// `webhook_zero_tools_trigger` porte `"[GitHub] PR closed:"` et
    /// `"[GitHub] Check suite success on"` — deux littéraux qui *contiennent*
    /// les aiguilles ci-dessous sans les être. Un prédicat par sous-chaîne
    /// l'accuserait, et une garde qui rougit sur du code sain est une garde
    /// qu'on désarme.
    ///
    /// Le prix, écrit plutôt que découvert : ce scan attrape la **copie** du
    /// corps (la forme d'un second lecteur, qui copie les `starts_with`) et
    /// rate une paraphrase qui écrirait les mêmes préfixes autrement. C'est le
    /// bon arbitrage — la classe mika#2158 est née d'une regex copiée, pas
    /// d'une regex ré-écrite.
    fn fallthrough_domain_needles() -> [String; 2] {
        // Composés à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        [
            format!("[GitHub] PR{}", " "),
            format!("[GitHub] Check suite{}", " "),
        ]
    }

    /// **Un seul corps définit le domaine Webhook Fallthrough (mika#2517 U5).**
    ///
    /// Aucun test comportemental ne peut voir cette classe : un second corps ne
    /// rend **aucune décision fausse le jour où il est écrit**. Il diverge plus
    /// tard, en silence, avec toutes les assertions vertes — exactement la
    /// leçon mika#2158 (une regex copiée dont le commentaire disait
    /// « Mirrors … » et qui a ensuite raté deux élargissements). Le domaine a
    /// désormais **quatre** consommateurs (les deux refus de mika#910/#933/#1102,
    /// plus U2 et U3), donc quatre occasions de diverger.
    #[test]
    fn mika2517_the_fallthrough_domain_has_a_single_definition() {
        let needles = fallthrough_domain_needles();
        let owner = "crates/mika-agent/src/webhook_dispatch.rs";

        let mut definers = Vec::new();
        for (rel, content) in production_sources() {
            if FALLTHROUGH_DOMAIN_DEFINITION_ALLOWED.contains(&rel.as_str()) {
                continue;
            }
            let literals: Vec<String> = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .flat_map(string_literals)
                .collect();
            let defines = needles
                .iter()
                .all(|needle| literals.iter().any(|lit| lit == needle));
            if defines {
                definers.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme
        // un scan propre (mika#2103 / mika#2205). Si le corps est réécrit sans
        // ces littéraux, ce scan cesse de viser quoi que ce soit et doit le dire.
        assert!(
            definers.iter().any(|d| d == owner),
            "mika#2517 — la conjonction {needles:?} n'est écrite nulle part dans \
             {owner} : ce scan vise un corps mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = definers.iter().filter(|d| *d != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2517 — le domaine Webhook Fallthrough a une seconde définition : \
             {strangers:?}\n\n\
             RÉSOLUTION : retirer le second corps et appeler \
             `webhook_dispatch::is_webhook_fallthrough_domain`. Ne PAS l'ajouter à \
             FALLTHROUGH_DOMAIN_DEFINITION_ALLOWED — quatre consommateurs lisent ce \
             domaine, et deux corps ne rendent aucune décision fausse le jour où le \
             second est écrit."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2517_the_domain_definition_allowlist_is_empty() {
        assert!(
            FALLTHROUGH_DOMAIN_DEFINITION_ALLOWED.is_empty(),
            "FALLTHROUGH_DOMAIN_DEFINITION_ALLOWED est livrée vide et doit le \
             rester : quand le scan tire, on retire le second corps. Une allowlist \
             née vide est un emplacement où déposer la prochaine infraction \
             (mika#2323)."
        );
    }

    /// **Livrée vide, et le test plus bas l'assert.**
    const FALLTHROUGH_TURN_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// **Un seul écrivain du nom d'événement `webhook_fallthrough_turn`
    /// (mika#2517 U5).**
    ///
    /// La propriété est porteuse pour une raison précise : cette ligne **est**
    /// le contrôle positif de l'acceptation du ticket. L'AC est une *absence*
    /// (zéro phantom `pending`), et sans un compte des tours qui auraient pu en
    /// produire un, zéro phantom se lit exactement comme zéro tour (mika#2205).
    /// Un second écrivain ne rendrait aucune décision fausse ; il rendrait ce
    /// compte inexact — invisible à tout test comportemental, d'où un scan.
    #[test]
    fn mika2517_the_fallthrough_turn_event_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("webhook_fallthrough{}", "_turn");
        let owner = "crates/mika-agent/src/agent_loop/mod.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if FALLTHROUGH_TURN_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            let carries = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .any(|line| {
                    string_literals(line)
                        .iter()
                        .any(|lit| lit.contains(needle.as_str()))
                });
            if carries {
                writers.push(rel);
            }
        }

        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2517 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2517 — le nom d'événement du tour fallthrough a un second \
             écrivain : {strangers:?}\n\n\
             RÉSOLUTION : retirer le second site. Ne PAS l'ajouter à \
             FALLTHROUGH_TURN_SOLE_WRITER_EXCEPTIONS — le `GROUP BY` qui mesure la \
             population du défaut n'est exact que tant qu'un seul site l'écrit."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2517_the_sole_writer_allowlist_is_empty() {
        assert!(
            FALLTHROUGH_TURN_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "FALLTHROUGH_TURN_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain."
        );
    }

    /// L'allowlist du scan d'exhaustivité est livrée vide, et le reste.
    ///
    /// Sans ce test, la doctrine « on déclare, on n'allowliste pas » ne vivrait
    /// que dans un doc-comment — et un doc-comment n'a jamais fait rougir une
    /// CI. Le jour où quelqu'un ajoute une entrée, il doit d'abord supprimer ce
    /// test, ce qui est un geste visible en revue.
    #[test]
    fn mika2201_the_exhaustiveness_allowlist_is_empty() {
        assert!(
            ALLOWED_UNDECLARED_SITES.is_empty(),
            "ALLOWED_UNDECLARED_SITES est livrée vide et doit le rester : quand la garde \
             tire, on déclare le site dans scripts/canonical-tokens.tsv. Une allowlist née \
             vide est un emplacement où déposer la prochaine infraction (mika#2323)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2495 — aucune fixture ne sonde une plage d'adresses de documentation.
    // ─────────────────────────────────────────────────────────────────────

    /// Les plages réservées à la documentation : les trois IPv4 de la RFC 5737
    /// et le préfixe IPv6 de la RFC 3849.
    ///
    /// **Assemblées par `concat!` de fragments**, donc ce fichier ne porte
    /// littéralement aucun des motifs qu'il cherche. Sans cela le scan rougirait
    /// à sa première exécution, sur sa propre définition — le motif de
    /// `worktree_reaper.rs` et de `mika2498_le_nom_du_refus_a_un_seul_ecrivain`
    /// ci-dessus.
    fn doc_range_needles() -> Vec<&'static str> {
        vec![
            concat!("192.0", ".2."),
            concat!("198.51", ".100."),
            concat!("203.0", ".113."),
            concat!("2001:", "db8"),
            concat!("2001:", "0db8"),
        ]
    }

    /// Les plages de documentation qu'une source porte **hors commentaire**.
    ///
    /// Le dépouillement passe par [`crate::source_scan::strip_comment_lines`] —
    /// le lecteur unique du prédicat, extrait par mika#2495 de `fn_bodies` — et
    /// c'est ce qui rend la garde compatible avec sa propre réparation : le
    /// doc-comment de `probe_executor_health_returns_none_on_unreachable_endpoint`
    /// **nomme** l'adresse fautive pour expliquer pourquoi on ne s'en sert plus.
    ///
    /// Une simple recherche par corps de fonction (`fn_bodies`) serait
    /// insuffisante : un `const BOGUS: &str = "…"` à portée de module lui
    /// échapperait.
    fn doc_range_hits(src: &str, needles: &[&'static str]) -> Vec<&'static str> {
        let code = crate::source_scan::strip_comment_lines(src);
        needles
            .iter()
            .copied()
            .filter(|n| code.contains(n))
            .collect()
    }

    /// **Toutes** les sources Rust sous `crates/`, code de test INCLUS.
    ///
    /// La population est **inversée** par rapport à [`production_sources`], et
    /// c'est délibéré : les gardes voisines de ce module cherchent un motif
    /// qu'un site de production ne doit pas porter et écartent les fixtures, qui
    /// le posent légitimement. Ici le motif fautif vit *dans* le code de test —
    /// c'est même son habitat exclusif, une source de production ne codant
    /// jamais une IP en dur — donc le scan doit l'inclure, et l'inclusion ne
    /// crée aucun faux positif côté production.
    fn all_rust_sources() -> Vec<(String, String)> {
        let crates_dir = repo_root().join("crates");
        let mut out = Vec::new();
        let mut stack = vec![crates_dir];

        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries {
                let path = entry.expect("entrée de répertoire lisible").path();
                if path.is_dir() {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    // `target/` est un artefact de build, pas de la source.
                    if name == "target" {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let rel = path
                    .strip_prefix(repo_root())
                    .expect("chemin sous la racine")
                    .to_string_lossy()
                    .replace('\\', "/");
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                out.push((rel, content));
            }
        }

        assert!(
            !out.is_empty(),
            "aucune source Rust trouvée sous crates/ — un scan qui ne scanne rien est un \
             laissez-passer vide, pas un scan propre (mika#2103)"
        );
        out
    }

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Le recensement exhaustif de l'arbre à la livraison ne rendait qu'une
    /// occurrence — celle que mika#2495 corrige — donc il n'y a rien à excepter.
    /// Quand ce scan tire, **on répare la fixture**, on ne l'exempte pas
    /// (doctrine mika#2201, « on déclare, on n'allowliste pas ») : une adresse
    /// de documentation dans une fixture est verte sur le CI et rouge en
    /// pilote, ce qui est exactement la panne invisible que ce ticket a payée
    /// 6,95 USD de session QA.
    const ALLOWED_DOC_RANGE_FIXTURES: &[&str] = &[];

    /// **AC4 — la classe est refusée structurellement.**
    ///
    /// Aucun test comportemental ne peut voir cette classe. Une nouvelle fixture
    /// sondant `203.0.113.x` serait verte partout où on la regarde — sur le CI
    /// réel, qui ne porte aucun proxy — et rouge uniquement dans le bac à sable
    /// pilote, où personne ne regarde jusqu'à ce qu'une revue QA se bloque
    /// dessus. C'est cette signature qui justifie un scan de source plutôt
    /// qu'une assertion.
    #[test]
    fn mika2495_aucune_fixture_ne_sonde_une_plage_de_documentation() {
        let needles = doc_range_needles();
        let sources = all_rust_sources();

        // Anti-vacuité sur la POPULATION, pas sur le résultat : le scan ne vaut
        // que si sa population est bien celle qui est inversée. Si quelqu'un le
        // recâble un jour sur `production_sources()` — le réflexe, puisque c'est
        // ce que font ses quatre voisines — il ne regarderait plus le seul
        // endroit où le motif vit, et se tairait en ayant l'air sain.
        assert!(
            sources
                .iter()
                .any(|(rel, _)| crate::source_scan::is_test_source_path(Path::new(rel))),
            "mika#2495 — la population du scan ne contient aucune source de test : elle a \
             été recâblée sur la moitié de production, où le motif cherché ne vit jamais. \
             Le scan est alors décoratif (mika#2103)."
        );

        let mut offenders = Vec::new();
        for (rel, content) in &sources {
            if ALLOWED_DOC_RANGE_FIXTURES.contains(&rel.as_str()) {
                continue;
            }
            for hit in doc_range_hits(content, &needles) {
                offenders.push(format!("{rel} porte {hit}…"));
            }
        }

        assert!(
            offenders.is_empty(),
            "mika#2495 — une fixture sonde une plage d'adresses réservée à la \
             documentation (RFC 5737 / RFC 3849) :\n  {}\n\n\
             RÉSOLUTION : remplacer la fixture par un port de boucle locale fermé \
             (`TcpListener::bind(\"127.0.0.1:0\")` puis `drop`). Un proxy intercepte \
             une adresse non routable et rend un 400, jamais une erreur de transport — \
             le test est alors vert sur le CI et rouge en pilote. Ne PAS ajouter \
             d'entrée à ALLOWED_DOC_RANGE_FIXTURES.\n\n\
             Une mention en COMMENTAIRE n'est pas accusée : si cette ligne apparaît \
             pour de la prose, c'est le dépouillement de \
             `source_scan::strip_comment_lines` qu'il faut lire, pas l'aiguille qu'il \
             faut rétrécir.",
            offenders.join("\n  ")
        );
    }

    /// **Contrôle de non-vacuité — le scan attrape une fixture plantée.**
    ///
    /// Sans lui, « la garde se tait » et « la garde ne regarde rien » se lisent
    /// pareil : c'est la classe que ce module nomme déjà (*une garde qui ne
    /// vérifie rien est une décoration*, mika#2103). Il assert les deux moitiés
    /// du prédicat — le littéral est vu, la prose ne l'est pas — parce qu'une
    /// aiguille qui n'attraperait plus rien et un dépouillement qui avalerait
    /// tout produisent le même vert.
    #[test]
    fn mika2495_le_scan_attrape_une_fixture_plantee() {
        let needles = doc_range_needles();

        // Composée à l'exécution, pour la raison qui vaut pour l'aiguille : ce
        // fichier ne doit porter aucun des motifs qu'il cherche.
        let planted = format!("const BOGUS: &str = \"http://{}9:1/health\";", needles[2]);
        assert_eq!(
            doc_range_hits(&planted, &needles),
            vec![needles[2]],
            "le scan ne voit plus un littéral planté : l'aiguille ou le dépouillement \
             est cassé, et la garde est devenue décorative"
        );

        // L'autre moitié : la même adresse, en prose, n'est PAS une violation.
        // C'est le faux positif que le doc-comment de la fixture réparée
        // produirait — il nomme l'adresse pour expliquer pourquoi on ne s'en
        // sert plus.
        let prose = format!(
            "    /// La fixture précédente sondait `{}9` (RFC 5737).",
            needles[2]
        );
        assert!(
            doc_range_hits(&prose, &needles).is_empty(),
            "le scan accuse une mention en commentaire : il rougirait sur la prose qui \
             documente le défaut, et la réparation deviendrait indicible"
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus. Le jour où quelqu'un y
    /// dépose une entrée, c'est ce test qui rougit — et non un `grep` qui ment
    /// des mois plus tard.
    #[test]
    fn mika2495_l_allowlist_des_plages_de_documentation_est_livree_vide() {
        assert!(
            ALLOWED_DOC_RANGE_FIXTURES.is_empty(),
            "ALLOWED_DOC_RANGE_FIXTURES est livrée vide et doit le rester : quand le scan \
             tire, on RÉPARE la fixture, on ne l'exempte pas — une adresse de \
             documentation dans une fixture est verte sur le CI et rouge en pilote, ce \
             qui est la panne que mika#2495 a payée 6,95 USD."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2506 — le dispatch pilote a un recensement fermé, et le nom
    // d'audit du geste opérateur a un seul écrivain.
    // ─────────────────────────────────────────────────────────────────────

    /// Les sites de production qui **composent** un spawn de pilote.
    ///
    /// # Ce n'est PAS une allowlist, et la différence est de fond
    ///
    /// Une allowlist est un endroit où déposer la prochaine infraction
    /// (mika#2323). Ceci est un **recensement fermé**, comparé **dans les deux
    /// sens** — motif `FIRED_AT_LITERAL_WRITERS` (mika#2133) : une entrée qui ne
    /// correspond plus à aucun site fait rougir, sans quoi elle exempterait
    /// silencieusement un futur homonyme.
    ///
    /// # Correction mesurée du plan mika#2506
    ///
    /// Sa Fire-Disposition annonce une cardinalité de **2** et « aucune
    /// violation existante à excepter ». La mesure en donne **quatre** : les
    /// deux handlers de `server/` composent leur propre dispatch depuis
    /// mika#1572 / mika#1630, ce que mika#2335 a déjà dû nommer par écrit
    /// (« the last two exist because the first was copied, and say so in their
    /// own comments »), et `task_engine/dispatcher.rs` en est un quatrième. Une
    /// garde figée à 2 serait **rouge à la naissance**, et un lint rouge à la
    /// naissance se fait désarmer.
    ///
    /// Ce que la garde tient donc, et qui est vrai et vérifiable : **mika#2506
    /// n'ajoute aucun site.** Son appelant passe par
    /// `verdict_handler::try_engine_dispatch_for`, qui est l'un des quatre.
    /// Un **cinquième** est refusé.
    ///
    /// Unifier les quatre est un travail réel, avec un rayon d'action sur toute
    /// la boucle — **suivi**, précondition : que ce recensement cesse de
    /// décroître de lui-même.
    const ENGINE_PILOT_DISPATCH_SITES: &[&str] = &[
        "crates/mika-agent/src/skills/executor.rs",
        "crates/mika-agent/src/server/verdict_handler.rs",
        "crates/mika-agent/src/server/ready_label_handler.rs",
        "crates/mika-agent/src/task_engine/dispatcher.rs",
    ];

    /// Un cinquième site de dispatch pilote ne rendrait **aucune décision
    /// fausse le jour où il est écrit** — il dispatcherait, tous les tests
    /// resteraient verts — et divergerait plus tard en silence sur
    /// `mark_parent_dispatched` (le défaut que mika#2335 a dû fermer après trois
    /// copies), sur le bras `Deferred`, ou sur l'estampille `fired_at`. C'est la
    /// classe que seul un scan de source voit.
    #[test]
    fn mika2506_le_dispatch_deterministe_a_un_recensement_ferme() {
        // Composé à l'exécution pour que CE fichier ne se recense pas lui-même.
        let needle = format!("spawn_long_running{}", "_exec");

        let mut sites: Vec<String> = Vec::new();
        for (rel, content) in production_sources() {
            let calls = content.lines().any(|l| {
                let t = l.trim_start();
                if t.starts_with("//") || t.starts_with('*') {
                    return false;
                }
                // Le SITE D'APPEL, jamais la déclaration ni un `use`.
                l.contains(&format!("{needle}(")) && !t.starts_with("pub(crate) fn")
            });
            if calls {
                sites.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve personne se lit exactement comme
        // un arbre propre (mika#2103 / mika#2205).
        assert!(
            !sites.is_empty(),
            "mika#2506 — aucun site de dispatch pilote trouvé : ce scan vise un \
             symbole mort, il ne vérifie rien"
        );

        let newcomers: Vec<&String> = sites
            .iter()
            .filter(|s| !ENGINE_PILOT_DISPATCH_SITES.contains(&s.as_str()))
            .collect();
        assert!(
            newcomers.is_empty(),
            "mika#2506 — un site de dispatch pilote hors recensement : \
             {newcomers:?}\n\n\
             RÉSOLUTION : router ce site vers \
             `verdict_handler::try_engine_dispatch_for`, qui compose déjà la \
             chaîne complète (résolution d'outil, readiness, row callback, \
             `mark_parent_dispatched`, spawn). Ne PAS ajouter de ligne au \
             recensement : un cinquième site divergera en silence."
        );

        // Le sens inverse — une entrée périmée exempterait un futur homonyme.
        let stale: Vec<&&str> = ENGINE_PILOT_DISPATCH_SITES
            .iter()
            .filter(|declared| !sites.iter().any(|s| s == *declared))
            .collect();
        assert!(
            stale.is_empty(),
            "mika#2506 — le recensement nomme des sites qui ne dispatchent plus : \
             {stale:?}. Les retirer — un recensement périmé est une exemption \
             silencieuse."
        );
    }

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Zéro violation existante, et c'est vérifiable : `operator_iterate_dispatch`
    /// est un nom **neuf**. Il n'y a donc rien à excepter, ni de case où déposer
    /// la prochaine infraction (mika#2323).
    const OPERATOR_ITERATE_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// mika#2506 AC8 — le nom d'audit du geste opérateur a **un seul** écrivain.
    ///
    /// C'est ce qui rend
    /// `SELECT after_value, count(*) … WHERE tool_name = 'operator_iterate_dispatch'
    /// GROUP BY 1` exact plutôt qu'un nombre sur lequel deux sites peuvent
    /// diverger — et cette requête est la sonde S3 du plan, le **contrôle
    /// positif** sans lequel le silence de S1/S2 ne prouve rien.
    ///
    /// Assertion auto-nettoyante incluse : un scan visant un nom mort se lit
    /// exactement comme un scan propre.
    #[test]
    fn mika2506_le_nom_daudit_a_un_seul_ecrivain() {
        let needle = format!("operator_iterate{}", "_dispatch");
        let owner = "crates/mika-agent/src/server/iterate_dispatch.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if OPERATOR_ITERATE_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            let carries = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .any(|line| {
                    string_literals(line)
                        .iter()
                        .any(|lit| lit.contains(needle.as_str()))
                });
            if carries {
                writers.push(rel);
            }
        }

        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2506 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2506 — le nom d'audit du geste d'itération a un second \
             écrivain : {strangers:?}\n\n\
             RÉSOLUTION : faire passer ce site par \
             `server::iterate_dispatch::OPERATOR_ITERATE_AUDIT_NAME`. Ne PAS \
             l'ajouter à OPERATOR_ITERATE_SOLE_WRITER_EXCEPTIONS — la sonde S3 \
             n'est exacte que tant qu'un seul site écrit ce nom."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2506_l_allowlist_du_nom_daudit_est_vide() {
        assert!(
            OPERATOR_ITERATE_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "OPERATOR_ITERATE_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain \
             (doctrine mika#2201)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2545 — le nom du refus d'un re-dispatch sur ESCALATE a un écrivain.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Rien à excepter à la livraison, et c'est vérifiable : le nom
    /// `groom_escalate_redispatch_refused` est **neuf**. Quand ce scan tire,
    /// **on retire le second écrivain**, on ne l'excepte pas (doctrine
    /// mika#2201) — une allowlist née vide est un emplacement où déposer la
    /// prochaine infraction (mika#2323).
    const GROOM_ESCALATE_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// Le nom sert de nom d'événement de journal **et** de `tool_name` d'audit.
    /// C'est ce qui rend soustractibles les deux populations de mika#2545 — le
    /// producteur (`Outcome: ESCALATE` dans `tasks.result`) et le lecteur (les
    /// rejeux interceptés) — et donc ce qui rend la table de lecture de §7
    /// exacte plutôt qu'un nombre sur lequel deux sites peuvent divergir.
    ///
    /// Aucun test comportemental ne peut voir cette classe : un second écrivain
    /// ne rend **aucune** décision fausse le jour où il est écrit, il rend le
    /// compte inexact, en silence.
    #[test]
    fn mika2545_the_escalate_refusal_name_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("groom_escalate{}", "_redispatch_refused");
        let owner = "crates/mika-agent/src/skills/executor.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if GROOM_ESCALATE_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            let carries = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .any(|line| {
                    string_literals(line)
                        .iter()
                        .any(|lit| lit.contains(needle.as_str()))
                });
            if carries {
                writers.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme
        // un scan propre (mika#2103 / mika#2205).
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2545 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2545 — le nom du refus a un second écrivain : {strangers:?}\n\n\
             RÉSOLUTION : retirer le second site. Ne PAS l'ajouter à \
             GROOM_ESCALATE_SOLE_WRITER_EXCEPTIONS — les deux populations de \
             mika#2545 ne sont soustractibles que tant qu'un seul site l'écrit."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2545_the_sole_writer_allowlist_is_empty() {
        assert!(
            GROOM_ESCALATE_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "GROOM_ESCALATE_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain."
        );
    }

    /// Le `match` sur [`crate::skills::executor::GroomVerdictState`] n'a **aucun
    /// bras joker** — motif `GroomedState` (mika#2484 D1). Un quatrième état
    /// devra être décidé par le compilateur, jamais absorbé par un `_ =>` qui le
    /// ferait tomber du côté « laisser passer » en silence.
    ///
    /// Scan de source plutôt que test comportemental : un joker ajouté ne rend
    /// fausse aucune des décisions couvertes aujourd'hui.
    #[test]
    fn mika2545_the_verdict_match_has_no_wildcard_arm() {
        let owner = "crates/mika-agent/src/skills/executor.rs";
        let content = production_sources()
            .into_iter()
            .find(|(rel, _)| rel == owner)
            .map(|(_, c)| c)
            .unwrap_or_else(|| panic!("mika#2545 — {owner} introuvable : le scan ne lit rien"));

        // La plage du `match`, bornée sur la ligne qui le porte — composée à
        // l'exécution pour la même raison que le needle ci-dessus.
        let opener = format!("let refusal = match {}", "verdict {");
        let start = content.find(opener.as_str()).unwrap_or_else(|| {
            panic!(
                "mika#2545 — le `match` du verdict est introuvable dans {owner} : \
                 ce scan vise une forme morte"
            )
        });
        let body: String = content[start..]
            .lines()
            .take(30)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            body.contains("GroomVerdictState::NotEscalated")
                && body.contains("GroomVerdictState::Escalated")
                && body.contains("GroomVerdictState::Unreadable"),
            "bonne foi : la plage extraite doit bien porter les trois bras — {body}"
        );
        for wildcard in ["_ =>", "_=>"] {
            assert!(
                !body.contains(wildcard),
                "mika#2545 — le `match` du verdict porte un bras joker (`{wildcard}`) : \
                 un quatrième état y tomberait sans décision. RÉSOLUTION : \
                 énumérer l'état, ne pas l'absorber."
            );
        }
    }

    /// Le vocabulaire de refus est un **format de fil** à site unique, et cette
    /// garde vit ici — avec les autres scans de nom — plutôt que dans le module,
    /// parce que c'est la valeur telle qu'elle atterrit dans `audit_events` qui
    /// compte, pas la forme de l'`enum`.
    #[test]
    fn mika2506_les_motifs_de_refus_sont_declares_une_fois() {
        use crate::server::iterate_dispatch::ALL_ITERATE_REFUSAL_REASONS;

        assert_eq!(
            ALL_ITERATE_REFUSAL_REASONS.len(),
            7,
            "mika#2506 — sept motifs, pas six : la divergence avec l'AC3 du plan \
             est datée sur `IterateRefusal::EngineRefused`. Un motif ajouté ou \
             retiré est une RUPTURE de format de fil, à dater dans CLAUDE.md — \
             jamais une mise à jour de ce nombre en silence."
        );
        let mut sorted = ALL_ITERATE_REFUSAL_REASONS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ALL_ITERATE_REFUSAL_REASONS.len(),
            "deux motifs portent la même valeur de fil : une population serait \
             coupée en deux sans le dire"
        );
    }
}
