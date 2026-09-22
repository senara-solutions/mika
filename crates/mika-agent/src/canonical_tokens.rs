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
}
