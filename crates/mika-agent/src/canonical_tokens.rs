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

    // ─────────────────────────────────────────────────────────────────────
    // mika#2590 R9 / U7a — le marqueur de convergence n'a qu'un lecteur.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test frère l'assert.**
    ///
    /// L'inventaire a été relevé avant rédaction : `GROOM_SUCCESS_MARKER` était
    /// lu par **deux** sites lâches — `db/tasks.rs` par `instr` en SQL et
    /// `task_engine/dispatcher.rs` par `contains` — que mika#2590 retire dans le
    /// même commit au profit de [`crate::task_state::tasks::groom_result_convergence`].
    /// Quand ce scan tire, **la résolution est de retirer la lecture**, jamais
    /// d'ajouter une entrée (doctrine mika#2201).
    const GROOM_MARKER_LOOSE_READERS_ALLOWED: &[&str] = &[];

    /// Le **propriétaire** du marqueur : il le définit et porte son unique
    /// lecteur. Hors population par construction, jamais par exemption.
    const GROOM_MARKER_OWNER: &str = "crates/mika-agent/src/task_state/tasks.rs";

    /// **U7a / R9 — aucun second lecteur lâche du marqueur de convergence.**
    ///
    /// Le défaut mesuré sur mika#2105 est une lecture par **sous-chaîne** : la
    /// note d'un saut `already_groomed` cite `Outcome: PLAN_GROOMED` en toutes
    /// lettres pour expliquer qu'aucune preuve n'est frappée, et
    /// `instr(child.result, …) > 0` en faisait la preuve. Le remède est un
    /// lecteur unique et ancré ; ce scan est ce qui empêche un troisième site de
    /// rouvrir la classe.
    ///
    /// **Aucun test comportemental ne peut voir cette classe.** Un second
    /// lecteur écrit par `contains` ne rend *aucune* décision fausse le jour où
    /// il est écrit — il diverge plus tard, en silence, avec toutes les
    /// assertions au vert. C'est très exactement ce que `grooming_marker.rs`
    /// (mika#2158) a dû graver une fois, et ce que la porte a repayé ici.
    ///
    /// **Ce que ce scan n'attrape pas, nommé :** un lecteur qui reconstruirait
    /// le littéral à la main (`"Outcome: " + "PLAN_GROOMED"`) échappe au
    /// prédicat, qui porte sur le **symbole**. Le scan d'exhaustivité
    /// `mika2201_every_match_site_is_declared`, lui, part du **jeton** et verrait
    /// le littéral : la composition des deux ferme le trou que chacun laisse.
    #[test]
    fn mika2590_le_marqueur_de_convergence_na_quun_lecteur() {
        // Composés à l'exécution pour que CE fichier ne se dénonce pas
        // lui-même — motif `mika2484_un_seul_lecteur_decisionnel_de_la_preuve`.
        let symbol = format!("GROOM_SUCCESS{}", "_MARKER");
        let loose = [".contains(", "instr(", ".find("];

        let mut offenders: Vec<String> = Vec::new();
        let mut owner_seen = false;

        for (rel, content) in production_sources() {
            if GROOM_MARKER_LOOSE_READERS_ALLOWED.contains(&rel.as_str()) {
                continue;
            }
            // Le corps de production seul : une doc-prose qui *parle* du
            // `contains` retiré n'est pas une lecture (classe mika#2050, dont
            // le faux positif a été mesuré sur le Signal S).
            let production = match content.find("\n#[cfg(test)]\nmod tests {") {
                Some(i) => &content[..i],
                None => &content[..],
            };
            let lines: Vec<&str> = production
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .filter(|l| l.contains(&symbol))
                .collect();

            if rel == GROOM_MARKER_OWNER {
                owner_seen = !lines.is_empty();
                continue;
            }
            if lines.iter().any(|l| loose.iter().any(|n| l.contains(n))) {
                offenders.push(rel);
            }
        }

        // Anti-vacuité, les deux moitiés. Sans elles un renommage rendrait ce
        // scan silencieusement inerte, ce qui se lit exactement comme un arbre
        // propre (mika#2205).
        assert!(
            !production_sources().is_empty(),
            "mika#2590 — la population examinée est vide : ce scan ne regarde rien"
        );
        assert!(
            owner_seen,
            "mika#2590 — `{symbol}` n'est lu nulle part dans {GROOM_MARKER_OWNER} : \
             ce scan vise un mort, il ne vérifie rien"
        );

        assert!(
            offenders.is_empty(),
            "mika#2590 — le marqueur de convergence a un second lecteur lâche : \
             {offenders:?}\n\n\
             RÉSOLUTION : passer par `task_state::tasks::groom_result_convergence`, \
             qui lit le marqueur EN POSITION DE VERDICT. Ne PAS ajouter le site à \
             GROOM_MARKER_LOOSE_READERS_ALLOWED — une lecture par sous-chaîne est \
             polluée par la prose qui nomme ce qu'elle cherche, et c'est le défaut \
             mesuré sur mika#2105 (la note d'un refus valait preuve)."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist de U7a.
    #[test]
    fn mika2590_lallowlist_des_lecteurs_laches_reste_vide() {
        assert!(
            GROOM_MARKER_LOOSE_READERS_ALLOWED.is_empty(),
            "GROOM_MARKER_LOOSE_READERS_ALLOWED est livrée vide et doit le rester : \
             quand le scan tire, on RETIRE la lecture. Une allowlist née vide est un \
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
    // mika#2260 — le nom d'audit de la porte d'entrée a un écrivain.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test frère l'assert.**
    ///
    /// Rien à excepter à la livraison : le nom
    /// `ci_success_handler_skipped_not_merge_actor` est **neuf**. Quand ce scan
    /// tire, **on retire le second écrivain**, on ne l'excepte pas (doctrine
    /// mika#2201).
    const ENTRY_GATE_SKIP_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// Le nom d'audit de la porte d'entrée de l'évaluateur n'a qu'un écrivain.
    ///
    /// La propriété est porteuse parce que `CLAUDE.md` déclare ce nom **compteur
    /// mesuré** — c'est la population qu'AC6 interroge après déploiement, et son
    /// comparaison avec `ci_success_handler_processed` sous le même `agent_id` est
    /// ce qui dit « un seul agent entre ». Deux écrivains rendraient ce compte
    /// inexact, et aucun test comportemental ne peut voir cette classe : un second
    /// site ne rendrait **aucune** décision fausse le jour où il est écrit.
    #[test]
    fn mika2260_the_entry_gate_skip_name_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("ci_success_handler_skipped{}", "_not_merge_actor");
        let owner = "crates/mika-agent/src/server/ci_success_handler.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if ENTRY_GATE_SKIP_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
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

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme un
        // scan propre (mika#2103 / mika#2205).
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2260 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2260 — le nom d'audit de la porte d'entrée a un second écrivain : \
             {strangers:?}\n\n\
             RÉSOLUTION : retirer le second site. Ne PAS l'ajouter à \
             ENTRY_GATE_SKIP_SOLE_WRITER_EXCEPTIONS — la mesure d'AC6 (« un seul \
             agent entre ») n'est exacte que tant qu'un seul site écrit ce nom."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2260_the_sole_writer_allowlist_is_empty() {
        assert!(
            ENTRY_GATE_SKIP_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "ENTRY_GATE_SKIP_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
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

    // ─────────────────────────────────────────────────────────────────────
    // mika#1833 — les deux noms de recensement KG ont UN écrivain chacun.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Les deux noms naissent avec mika#1833 : il n'y a rien à exempter.
    /// **Quand le scan tire, on retire le second écrivain ; on n'ajoute pas
    /// de ligne** (doctrine mika#2201).
    const KG_CENSUS_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// U3 — `domain_graph_empty` et `kg_budget_resolved` ont un écrivain
    /// unique (mika#1833).
    ///
    /// C'est cette propriété qui rend les comptes de la §4 du plan **exacts**
    /// plutôt que discutables entre deux sites. `domain_graph_empty` est la
    /// sonde S2 — *l'hypothèse « graphe vide » est-elle vraie ?* — et son
    /// régime attendu est zéro : un second écrivain rendrait une occurrence
    /// inattribuable, c'est-à-dire ferait de la seule ligne qui tranche le
    /// ticket une ligne qu'il faut d'abord enquêter. `kg_budget_resolved` est
    /// la sonde S0, dont l'opérateur lit le `budget_source` pour choisir entre
    /// trois remèdes.
    ///
    /// Aucun test comportemental ne voit cette classe : un second écrivain ne
    /// rend **aucune décision fausse** le jour où il est écrit.
    #[test]
    fn mika1833_the_census_event_names_have_a_single_writer() {
        // Composés à l'exécution pour que CE fichier ne se dénonce pas
        // lui-même quand un scan de source le lit.
        let owners: &[(String, &str)] = &[
            (
                format!("domain_graph{}", "_empty"),
                "crates/mika-agent/src/kg/domain_builder.rs",
            ),
            (
                format!("kg_budget{}", "_resolved"),
                "crates/mika-agent/src/server/mod.rs",
            ),
        ];

        let sources = production_sources();

        for (needle, owner) in owners {
            let mut writers = Vec::new();
            for (rel, content) in &sources {
                if KG_CENSUS_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
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
                    writers.push(rel.clone());
                }
            }

            // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement
            // comme un scan propre (mika#2103 / mika#2205).
            assert!(
                writers.iter().any(|w| w == owner),
                "mika#1833 — `{needle}` n'est écrit nulle part dans {owner} : \
                 ce scan vise un nom mort, il ne vérifie rien"
            );

            let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
            assert!(
                strangers.is_empty(),
                "mika#1833 — `{needle}` a un second écrivain : {strangers:?}\n\n\
                 RÉSOLUTION : retirer le second site. Ne PAS l'ajouter à \
                 KG_CENSUS_SOLE_WRITER_EXCEPTIONS — le compte que ce nom \
                 existe pour servir n'est exact que tant qu'un seul site \
                 l'écrit."
            );
        }
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika1833_the_census_sole_writer_allowlist_is_empty() {
        assert!(
            KG_CENSUS_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "KG_CENSUS_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : une allowlist née vide est un emplacement où déposer la \
             prochaine infraction (mika#2323)."
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
    // mika#2601 — l'enregistrement d'une récurrente a UN appelant, et ses
    // deux événements UN écrivain chacun.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// La population a été recensée avant d'écrire ce scan : un seul site de
    /// production appelle `create_recurring_task_if_absent` hors plomberie, et
    /// c'est celui qu'on attend. **Quand ce scan tire, on route le nouveau site
    /// par `ensure_recurring_task` ; on n'ajoute pas de ligne** (doctrine
    /// mika#2201) — un site qu'on ne veut pas armer est un site à supprimer.
    const RECURRING_REGISTRATION_SCAN_EXCEPTIONS: &[&str] = &[];

    /// **Un seul appelant de l'enregistrement, deux écrivains d'événement
    /// (mika#2601 R-8).**
    ///
    /// *Aucun test comportemental ne peut voir cette classe :* un second site
    /// d'enregistrement ne rend **aucune** décision fausse le jour où il est
    /// écrit — l'enregistrement fonctionne, toutes les assertions restent
    /// vertes, et seul le réessai disparaît, en silence.
    ///
    /// Les deux aiguilles sont composées à l'exécution pour que CE fichier ne
    /// se dénonce pas lui-même (motif mika#2496).
    ///
    /// **Limite héritée, nommée plutôt que découverte :** `production_sources`
    /// tronque chaque fichier au premier marqueur de module de test *textuel*,
    /// où qu'il soit. Un étranger placé après un tel marqueur est donc
    /// invisible — faux négatif partagé par tous les scans de ce module. Ce que
    /// l'anti-vacuité ci-dessous garantit, c'est que le scan n'est pas devenu
    /// aveugle **sur sa propre cible**, ce qui est le mode de panne qui se lit
    /// comme un arbre propre.
    #[test]
    fn mika2601_la_registration_recurrente_a_un_seul_appelant_et_deux_ecrivains() {
        let owner = "crates/mika-agent/src/task_engine/mod.rs";
        // La plomberie : la définition et son enveloppe asynchrone. Ce ne sont
        // pas des appelants, ce sont les deux maillons que tout appelant
        // traverse.
        let plumbing = [
            "crates/mika-agent/src/db/tasks.rs",
            "crates/mika-agent/src/async_db.rs",
        ];

        let call_needle = format!("create_recurring_task{}", "_if_absent(");
        let retried_needle = format!("recurring_registration{}", "_retried");
        let failed_needle = format!("recurring_registration{}", "_failed");

        let mut callers = Vec::new();
        let mut retried_writers = Vec::new();
        let mut failed_writers = Vec::new();

        for (rel, content) in production_sources() {
            if RECURRING_REGISTRATION_SCAN_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            let code: Vec<&str> = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .collect();

            if !plumbing.contains(&rel.as_str()) && code.iter().any(|l| l.contains(&call_needle)) {
                callers.push(rel.clone());
            }
            // Les noms d'événement sont des littéraux de chaîne : les chercher
            // comme tels évite de compter une mention en prose.
            let literal_carries = |needle: &str| {
                code.iter()
                    .any(|line| string_literals(line).iter().any(|lit| lit.contains(needle)))
            };
            if literal_carries(&retried_needle) {
                retried_writers.push(rel.clone());
            }
            if literal_carries(&failed_needle) {
                failed_writers.push(rel.clone());
            }
        }

        // ── Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement
        // comme un scan propre (mika#2103 / mika#2205). Les trois aiguilles
        // doivent apparaître, et sur le fichier attendu.
        for (what, found) in [
            ("l'appel à l'enregistrement", &callers),
            ("l'événement de réessai", &retried_writers),
            ("l'événement d'échec", &failed_writers),
        ] {
            assert!(
                found.iter().any(|f| f == owner),
                "mika#2601 — {what} est introuvable dans {owner} : ce scan vise \
                 un nom mort, il ne vérifie rien.\n\
                 Cause la plus probable : un marqueur de module de test est \
                 apparu plus haut dans ce fichier et `production_sources` l'a \
                 tronqué avant la cible."
            );
        }

        let stray_callers: Vec<&String> = callers.iter().filter(|c| *c != owner).collect();
        assert!(
            stray_callers.is_empty(),
            "mika#2601 — un second site de production enregistre une récurrente \
             sans passer par `ensure_recurring_task` : {stray_callers:?}\n\n\
             RÉSOLUTION : router ce site par `ensure_recurring_task`, qui porte \
             le réessai sous contention. Ne PAS l'ajouter à \
             RECURRING_REGISTRATION_SCAN_EXCEPTIONS — un site qu'on ne veut pas \
             armer est un site à supprimer (doctrine mika#2201)."
        );

        for (event, writers) in [
            (&retried_needle, &retried_writers),
            (&failed_needle, &failed_writers),
        ] {
            let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
            assert!(
                strangers.is_empty(),
                "mika#2601 — `{event}` a un second écrivain : {strangers:?}\n\n\
                 RÉSOLUTION : retirer le second site. Les deux régimes attendus \
                 (`_retried` non vide et faible, `_failed` vide) ne sont \
                 lisibles que tant qu'un seul site écrit chaque nom."
            );
        }
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2601_lallowlist_du_scan_est_livree_vide() {
        assert!(
            RECURRING_REGISTRATION_SCAN_EXCEPTIONS.is_empty(),
            "RECURRING_REGISTRATION_SCAN_EXCEPTIONS est livrée vide et doit le \
             rester : une allowlist née vide est un tiroir où déposer la \
             prochaine infraction (mika#2323)."
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

    // ─────────────────────────────────────────────────────────────────────
    // mika#2634 — le nom d'audit sous lequel vit la santé du LANCEUR a un
    // seul écrivain.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Zéro violation existante, et c'est vérifiable : le nom
    /// `pilot_launcher_health` est **neuf**. Il n'y a donc rien à excepter, ni
    /// de case où déposer la prochaine infraction (mika#2323). Quand le scan
    /// tire, **on retire le second site**, on ne l'allowliste pas (doctrine
    /// mika#2201).
    const LAUNCHER_HEALTH_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// Le prédicat du scan, extrait pour que son **contrôle de bonne foi**
    /// l'exerce plutôt qu'une copie qui peut en diverger (mika#2634 phase B).
    ///
    /// Comparaison **exacte** sur le littéral entier : voir le doc-comment du
    /// scan pour la mesure qui l'impose.
    fn launcher_health_literal_present(content: &str, needle: &str) -> bool {
        content
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
            })
            .any(|line| string_literals(line).iter().any(|lit| lit.trim() == needle))
    }

    /// **Contrôle de bonne foi du scan ci-dessous, et il est dû.**
    ///
    /// La phase B a resserré le prédicat de la sous-chaîne au littéral exact.
    /// Resserrer une garde sans montrer qu'elle mord encore est précisément la
    /// façon dont une garde devient inerte en silence (mika#2103 / mika#2205), et
    /// l'anti-vacuité du scan ne le dit pas : elle vérifie que le **propriétaire**
    /// porte le nom, pas qu'un **étranger** serait vu.
    ///
    /// Les trois contrôles négatifs sont les trois formes réellement présentes
    /// dans l'arbre : un nom d'événement **préfixé** par le nom d'audit (le
    /// fail-open du lecteur), un commentaire qui le cite, et le résidu de la
    /// phase A — aucun des trois n'est un écrivain de la ligne d'audit.
    #[test]
    fn mika2634_the_sole_writer_scan_still_catches_a_second_site() {
        let needle = format!("pilot_launcher{}", "_health");

        assert!(
            launcher_health_literal_present(
                &format!("    db.log_audit_event(s, \"{needle}\", k, None, v, None, None).await?;"),
                &needle
            ),
            "INVARIANT VIOLÉ : le prédicat resserré ne voit plus un second \
             écrivain de la ligne d'audit — le scan est devenu inerte et il se \
             lirait exactement comme un arbre propre"
        );

        for benign in [
            // Le nom d'événement de journal du fail-open, PRÉFIXÉ par le nom
            // d'audit : c'est la forme qui a fait rougir le scan à l'écriture de
            // la phase B, et c'est une autre surface.
            format!("        warn!(event = \"{needle}_unreadable\", error = %e);"),
            // Une mention en commentaire.
            format!("    // le nom `{needle}` vit dans executor.rs"),
            // Le résidu de la phase A, même famille, autre nom.
            "        warn!(event = \"pilot_launcher_dead_audit_failed\");".to_string(),
        ] {
            assert!(
                !launcher_health_literal_present(&benign, &needle),
                "faux positif du scan sur une ligne qui n'écrit pas la ligne \
                 d'audit : {benign}"
            );
        }
    }

    /// La propriété qui rend le `GROUP BY after_value` de l'opérateur exact.
    ///
    /// # Pourquoi un scan de source et pas un test comportemental
    ///
    /// Un second écrivain ne rendrait **aucune décision fausse** : les deux
    /// lignes partiraient, le moteur continuerait de classer, et chaque
    /// assertion comportementale de `skills::executor::tests::mika2634`
    /// resterait verte. Ce qui deviendrait faux est le **compte** — et c'est
    /// lui que la phase B du ticket (le frein à deux occurrences) lira pour
    /// décider d'arrêter la flotte. Un compte sur lequel deux sites peuvent
    /// diverger est un frein qui mord au mauvais moment.
    ///
    /// # La comparaison est EXACTE, et pas en sous-chaîne (mika#2634 phase B)
    ///
    /// Fait mesuré en écrivant la phase B, exactement comme mika#2649 l'a mesuré
    /// un ticket plus tôt sur son propre scan jumeau : les noms d'**événement de
    /// journal** de cette famille sont construits en **préfixant** le nom
    /// d'audit — `pilot_launcher_health_unreadable` pour le fail-open du lecteur
    /// — donc une comparaison par sous-chaîne compte une ligne de journal comme
    /// un second écrivain de la ligne d'audit. Ce sont deux surfaces distinctes :
    /// l'une répond à « combien de lanceurs sont morts » (`audit_events`), l'autre
    /// à « le ledger était-il lisible » (le journal), et aucune n'a à se taire pour
    /// que l'autre soit exacte.
    ///
    /// La propriété gardée est donc : **un seul site de production porte le
    /// littéral `pilot_launcher_health` entier**. Un second site qui l'écrirait —
    /// par `db.log_audit_event(…, "pilot_launcher_health", …)` — est toujours
    /// attrapé, ce qui est tout ce dont le `GROUP BY after_value` a besoin.
    /// Élargir à la sous-chaîne « pour être sûr » est ce qui rend ce scan
    /// permanemment rouge, et un lint rouge à la naissance se fait désarmer.
    #[test]
    fn mika2634_the_launcher_health_name_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("pilot_launcher{}", "_health");
        let owner = "crates/mika-agent/src/skills/executor.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if LAUNCHER_HEALTH_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            if launcher_health_literal_present(&content, &needle) {
                writers.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme
        // un scan propre (mika#2103 / mika#2205).
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2634 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2634 — le nom d'audit de la santé du lanceur a un second \
             écrivain : {strangers:?}\n\n\
             RÉSOLUTION : retirer le second site. Ne PAS l'ajouter à \
             LAUNCHER_HEALTH_SOLE_WRITER_EXCEPTIONS — le compte que le frein de \
             la phase B lira n'est exact que tant qu'un seul site l'écrit."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2634_the_sole_writer_allowlist_is_empty() {
        assert!(
            LAUNCHER_HEALTH_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "LAUNCHER_HEALTH_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain. Une \
             allowlist née vide est un emplacement où déposer la prochaine \
             infraction (mika#2323)."
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

    // ─────────────────────────────────────────────────────────────────────
    // mika#2573 — un écrivain du nom de l'événement de refus de création de
    // travail sur un tour Fallthrough.
    //
    // Frère immédiat du scan mika#2517 ci-dessus, et il vit à côté de lui pour
    // la même raison : deux gardes de la même famille se lisent mieux en paire.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// L'inventaire est **clos à un seul écrivain** au moment de la livraison :
    /// le nom `fallthrough_work_creation` n'existait nulle part dans l'arbre
    /// avant ce ticket, donc il n'y a **aucune violation existante** à excepter,
    /// ni de case où déposer la prochaine (mika#2323). Quand le scan tire, **on
    /// retire le second écrivain**, on ne l'allowliste pas (doctrine mika#2201).
    const FALLTHROUGH_WORK_CREATION_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// **Un seul écrivain du nom `fallthrough_work_creation` (mika#2573 U4).**
    ///
    /// Le fichier propriétaire est `evidence/guards.rs` — il porte la constante
    /// `FALLTHROUGH_WORK_CREATION_AUDIT_TOOL`, et `builtin_handlers.rs`
    /// l'**importe** plutôt que de retaper le littéral. C'est la propriété qui
    /// rend le `GROUP BY after_value` de l'opérateur exact plutôt qu'un nombre
    /// sur lequel deux sites peuvent diverger.
    ///
    /// Aucun test comportemental ne peut voir cette classe : un second écrivain
    /// ne rend **aucune décision fausse**, il rend le compte inexact.
    #[test]
    fn mika2573_the_work_creation_event_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("fallthrough_work{}", "_creation");
        let owner = "crates/mika-agent/src/evidence/guards.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if FALLTHROUGH_WORK_CREATION_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
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

        // Anti-vacuité : un scan qui vise un nom mort se lit exactement comme un
        // scan propre (mika#2103 / mika#2205).
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2573 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2573 — le nom de l'événement de refus a un second écrivain : \
             {strangers:?}\n\n\
             RÉSOLUTION : retirer le littéral et importer \
             `evidence::guards::FALLTHROUGH_WORK_CREATION_AUDIT_TOOL`. Ne PAS \
             l'ajouter à FALLTHROUGH_WORK_CREATION_SOLE_WRITER_EXCEPTIONS — le \
             `GROUP BY` qui mesure la population du défaut n'est exact que tant \
             qu'un seul site l'écrit."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2573_the_sole_writer_allowlist_is_empty() {
        assert!(
            FALLTHROUGH_WORK_CREATION_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "FALLTHROUGH_WORK_CREATION_SOLE_WRITER_EXCEPTIONS est livrée vide et \
             doit le rester : quand le scan tire, on retire le second écrivain. \
             Une allowlist née vide est un emplacement où déposer la prochaine \
             infraction (mika#2323)."
        );
    }

    /// **Un seul site définit les deux motifs (mika#2573 U2).**
    ///
    /// Ils atterrissent dans `audit_events.after_value` et un opérateur en fait
    /// des `GROUP BY` : deux orthographes d'un même motif couperaient une
    /// population en deux sans le dire (motif mika#2323 / mika#2536). Le test de
    /// format de fil d'`evidence::guards` fige les *valeurs* ; ce scan fige le
    /// *nombre de sites qui les écrivent*, ce qu'une assertion sur les valeurs ne
    /// peut pas voir.
    #[test]
    fn mika2573_the_work_creation_motifs_have_a_single_definition() {
        // Composés à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needles = [
            format!("issue{}", "_create"),
            format!("ready_label{}", "_add"),
        ];
        let owner = "crates/mika-agent/src/evidence/guards.rs";

        let mut definers = Vec::new();
        for (rel, content) in production_sources() {
            let literals: Vec<String> = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .flat_map(string_literals)
                .collect();
            // Appariement **exact**, jamais par sous-chaîne : un voisin
            // légitime portant `ready_label_received` ou `ready_label_outcome`
            // *contient* l'aiguille sans l'être, et une garde qui rougit sur du
            // code sain est une garde qu'on désarme (leçon mika#2517).
            let defines = needles
                .iter()
                .all(|needle| literals.iter().any(|lit| lit == needle));
            if defines {
                definers.push(rel);
            }
        }

        assert!(
            definers.iter().any(|d| d == owner),
            "mika#2573 — la conjonction {needles:?} n'est écrite nulle part dans \
             {owner} : ce scan vise un registre mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = definers.iter().filter(|d| *d != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2573 — les motifs de création de travail ont une seconde \
             définition : {strangers:?}\n\n\
             RÉSOLUTION : retirer les littéraux et importer \
             `WORK_CREATION_MOTIF_ISSUE_CREATE` / \
             `WORK_CREATION_MOTIF_READY_LABEL_ADD`."
        );
    }

    /// Les sources de production, tronquées au **module** de test et non au
    /// premier `#[cfg(test)]` (mika#2624).
    ///
    /// [`production_sources`] coupe au premier `#[cfg(test)]` **où qu'il soit**,
    /// et `builtin_handlers.rs` en porte un à la ligne 674 — une paire
    /// `#[cfg(test)]` / `#[cfg(not(test))]` sur une constante de test. Les deux
    /// scans de mika#2624 visent du code situé *après*, donc réutiliser cet
    /// énumérateur les rendait **verts par population vide** : leurs deux
    /// anti-vacuités l'ont dit, et c'est la seule raison pour laquelle on le sait.
    /// Un garde de ce même fichier (`run_gh`'s argv scan) avait déjà dû écrire ce
    /// contournement ; mika#2575 l'a écrit une seconde fois dans son module.
    ///
    /// La coupe est celle de [`crate::source_scan::production_half`] — ancrée
    /// sur la **déclaration du module** de test, pas sur l'attribut seul — que
    /// le scan mika#2597 emploie aussi : un découpage, deux lecteurs.
    ///
    /// Déliberément local : corriger [`production_sources`] élargirait la
    /// population des cinq scans voisins (mika#2573, mika#2522, mika#2405, …),
    /// ce qui est un changement de leur périmètre et non du nôtre.
    fn production_sources_to_test_module() -> Vec<(String, String)> {
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
                    if path.file_name().unwrap_or_default().to_string_lossy() == "target" {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs")
                    || crate::source_scan::is_test_source_path(&path)
                {
                    continue;
                }
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
                let production = crate::source_scan::production_half(&content).to_string();
                out.push((rel, production));
            }
        }

        assert!(
            !out.is_empty(),
            "aucune source de production trouvée — un scan qui ne scanne rien est \
             un laissez-passer vide, pas un scan propre (mika#2103)"
        );
        out
    }

    /// Sites de production autorisés à consommer un `ConvertToDraftEvent`
    /// autrement qu'en le passant à `classify_hold_verdict` (mika#2624 D2).
    ///
    /// **Livrée vide et épinglée vide** — quand le scan ci-dessous tire, on
    /// retire la seconde classification et on appelle le lecteur unique ; on
    /// n'ajoute pas de ligne ici (doctrine mika#2201 ; une allowlist née vide est
    /// un tiroir où déposer la prochaine infraction, mika#2323).
    const HOLD_CLASSIFICATION_EXCEPTIONS: &[&str] = &[];

    /// **Le prédicat de hold a un lecteur unique (mika#2624 D2).**
    ///
    /// AC1 exige que le discriminant de mika#2597 soit « factorisé ou appelé,
    /// jamais recopié ». Ce scan le tient sur la **forme** plutôt que sur la
    /// bonne volonté : tout appel de production à
    /// `fetch_convert_to_draft_events` doit être **l'argument immédiat** de
    /// `classify_hold_verdict`. Co-location sur la même expression — le motif que
    /// le dépôt emploie déjà pour `_pilot_max_turns` et `_pilot_log_dir`.
    ///
    /// # Pourquoi un scan de source et pas un test comportemental
    ///
    /// Une seconde classification écrite demain ne rend **aucune** décision
    /// fausse le jour où elle est écrite : les deux lecteurs répondraient d'abord
    /// la même chose, toutes les assertions resteraient vertes, et ils
    /// divergeraient des mois plus tard. C'est la classe exacte que mika#2158 a
    /// mesurée — `is_groomed` et `check_grooming_markers` ont divergé pendant des
    /// mois derrière un commentaire disant « mirrors ».
    ///
    /// # Pourquoi cette aiguille et pas le nom du type
    ///
    /// `ConvertToDraftEvent` apparaît légitimement dans la prose d'un corps de
    /// refus et dans des doc-comments ; l'accuser ferait rougir du code sain, et
    /// une garde qui rougit sur du sain est une garde qu'on désarme (leçon
    /// mika#2517, mesurée aussi par le faux positif du Signal S, mika#2050). Et
    /// `HoldVerdict::Held` ne discrimine pas non plus : un **motif** de `match`
    /// s'écrit comme une **construction**, donc le scan accuserait les
    /// consommateurs légitimes. Ce qui identifie une classification est de lire
    /// la collection d'événements — et elle ne vient que du fetch.
    ///
    /// La **déclaration** du fetch est reconnue par sa forme (`fn` juste avant),
    /// jamais par un nom de fichier : un périmètre par fichier serait une
    /// allowlist déguisée.
    #[test]
    fn mika2624_le_predicat_de_hold_a_un_lecteur_unique() {
        // Composés à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let fetch = format!("fetch_convert_to_draft{}(", "_events");
        let classify = format!("classify_hold{}(", "_verdict");

        let mut sites: Vec<(String, usize, usize)> = Vec::new();
        let mut total_calls = 0usize;

        for (rel, content) in production_sources_to_test_module() {
            if HOLD_CLASSIFICATION_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            // Source normalisée en espaces : rustfmt coupe l'appel sur
            // plusieurs lignes, donc un prédicat ancré sur la ligne raterait les
            // deux sites réels.
            let normalised: String = crate::source_scan::strip_comment_lines(&content)
                .split_whitespace()
                .collect();

            let mut calls = 0usize;
            let mut wrapped = 0usize;
            for (idx, _) in normalised.match_indices(fetch.as_str()) {
                // La déclaration elle-même n'est pas un appel.
                if normalised[..idx].ends_with("fn") {
                    continue;
                }
                calls += 1;
                // L'appel doit être l'argument immédiat de la classification,
                // avec ou sans son chemin de module.
                let prefix = &normalised[..idx];
                let immediate = prefix.ends_with(classify.as_str())
                    || (prefix.ends_with("crate::github_graphql::")
                        && prefix
                            .trim_end_matches("crate::github_graphql::")
                            .ends_with(classify.as_str()));
                if immediate {
                    wrapped += 1;
                }
            }
            total_calls += calls;
            if calls != wrapped {
                sites.push((rel, calls, wrapped));
            }
        }

        // Anti-vacuité : un scan qui vise une aiguille morte se lit exactement
        // comme un arbre propre (mika#2205). Deux appels sont attendus —
        // `wip_rescue` et le terme de hold de `run_gh`.
        assert!(
            total_calls >= 2,
            "mika#2624 — {total_calls} appel(s) au fetch de timeline trouvé(s) : \
             ce scan vise une aiguille morte, il ne vérifie rien"
        );

        assert!(
            sites.is_empty(),
            "mika#2624 — une seconde classification de `ConvertToDraftEvent` \
             existe : {sites:?} (fichier, appels, appels enveloppés)\n\n\
             RÉSOLUTION : passer le résultat du fetch à \
             `wip_rescue::classify_hold_verdict` sur la même expression. Ne PAS \
             l'ajouter à HOLD_CLASSIFICATION_EXCEPTIONS — le discriminant de \
             mika#2597 n'a de valeur que tant qu'un seul site le décide."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2624_lallowlist_de_classification_est_livree_vide() {
        assert!(
            HOLD_CLASSIFICATION_EXCEPTIONS.is_empty(),
            "HOLD_CLASSIFICATION_EXCEPTIONS est livrée vide et doit le rester : \
             une seconde classification est un site à router vers le lecteur \
             unique, jamais un site à exempter (mika#2201)."
        );
    }

    /// **Le nom de refus a un écrivain unique (mika#2624).**
    ///
    /// `pr_ready_undraft_blocked` atterrit dans `audit_events.tool_name` et
    /// l'opérateur en fait un `GROUP BY after_value` — exact seulement tant qu'un
    /// seul site l'écrit. Un second écrivain ne rendrait aucune décision fausse ;
    /// il rendrait ce compte inexact, ce qui est invisible à tout test
    /// comportemental.
    #[test]
    fn mika2624_le_nom_de_refus_a_un_ecrivain_unique() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("pr_ready_undraft{}", "_blocked");
        let owner = "crates/mika-agent/src/skills/builtin_handlers.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources_to_test_module() {
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

        // Anti-vacuité (mika#2103 / mika#2205).
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2624 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2624 — le nom de l'événement de refus a un second écrivain : \
             {strangers:?}\n\n\
             RÉSOLUTION : retirer le littéral et importer \
             `skills::builtin_handlers::PR_READY_UNDRAFT_AUDIT_TOOL`."
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
             RÉSOLUTION : remplacer la fixture par \
             `mika_common::dead_endpoint::DeadEndpoint`, qui RÉSERVE un port de boucle \
             locale au lieu de le libérer (mika#2569). Un proxy intercepte une adresse \
             non routable et rend un 400, jamais une erreur de transport — le test est \
             alors vert sur le CI et rouge en pilote. Ne PAS ajouter d'entrée à \
             ALLOWED_DOC_RANGE_FIXTURES.\n\n\
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
    // mika#2569 — un point de terminaison mort se RÉSERVE, il ne se libère pas.
    // ─────────────────────────────────────────────────────────────────────

    /// L'identifiant lié par une ligne `let <id> … = … TcpListener::bind …`.
    ///
    /// **Angle mort assumé et écrit** : une liaison dont `let <id> =` et
    /// `TcpListener::bind` sont sur deux lignes distinctes n'est pas vue. Les six
    /// sites mesurés à la livraison sont tous sur une ligne ; un scan qui attrape
    /// la forme réelle vaut mieux qu'un scan qui prétend attraper toutes les
    /// formes concevables.
    fn dead_listener_binding_name(line: &str) -> Option<String> {
        let t = line.trim_start();
        let rest = t.strip_prefix("let ")?;
        let eq = rest.find('=')?;
        // Couper l'annotation de type éventuelle : `let l: TcpListener = …`.
        let mut name = &rest[..eq];
        if let Some(colon) = name.find(':') {
            name = &name[..colon];
        }
        let name = name.trim().strip_prefix("mut ").unwrap_or(name).trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        Some(name.to_string())
    }

    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    /// `true` si l'offset `at` tombe **à l'intérieur d'un littéral de chaîne**,
    /// approximé par la parité des `"` non échappés qui le précèdent sur sa ligne.
    ///
    /// # Pourquoi ce terme existe, et il n'était pas dans le plan
    ///
    /// Sans lui, le scan **rate son propre site fondateur**. Mesuré à la
    /// livraison sur la version HEAD de `remote_ask_integration.rs` : la ligne
    /// `.expect_err("should fail when no listener accepts the connection")` porte
    /// le mot `listener` en frontière de mot, ni suivi de `.local_addr` ni
    /// précédé de `drop(` — donc comptée comme un usage légitime, et le site que
    /// mika#2569 existe pour refuser serait passé en vert.
    ///
    /// L'approximation est à la ligne, donc une chaîne multi-ligne peut être mal
    /// comptée. Le **sens** de cette erreur est sûr : elle fait ignorer une
    /// occurrence, donc elle penche vers l'accusation — jamais vers le silence.
    fn dead_listener_in_string_literal(window: &str, at: usize) -> bool {
        let line_start = window[..at].rfind('\n').map_or(0, |i| i + 1);
        let prefix = &window[line_start..at];
        let mut quotes = 0usize;
        let mut escaped = false;
        for c in prefix.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            match c {
                '\\' => escaped = true,
                '"' => quotes += 1,
                _ => {}
            }
        }
        quotes % 2 == 1
    }

    /// `true` si `window` porte au moins une occurrence de `name` qui ne soit ni
    /// `name.local_addr…`, ni `drop(name)`, ni dans un littéral de chaîne.
    ///
    /// La recherche traverse les retours à la ligne des deux côtés : sur
    /// `cadence.rs`, avant sa migration, l'usage s'écrivait `let port = listener`
    /// puis `.local_addr()` à la ligne suivante — un prédicat à la ligne aurait
    /// lu ça comme un usage légitime et raté le site.
    fn dead_listener_has_real_use(window: &str, name: &str) -> bool {
        let bytes = window.as_bytes();
        let mut from = 0usize;
        while let Some(rel) = window[from..].find(name) {
            let at = from + rel;
            let after = at + name.len();
            from = after;

            let prev_ok = at == 0 || !is_ident_byte(bytes[at - 1]);
            let next_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
            if !prev_ok || !next_ok {
                continue;
            }
            if dead_listener_in_string_literal(window, at) {
                continue;
            }
            if window[after..].trim_start().starts_with(".local_addr") {
                continue;
            }
            if window[..at].trim_end().ends_with("drop(") {
                continue;
            }
            return true;
        }
        false
    }

    /// Les identifiants liés à un `TcpListener::bind` qui n'existent que pour
    /// rendre leur adresse — le motif « lier puis libérer » de mika#2569.
    ///
    /// Fonction **pure**, testable sur chaîne, sur le modèle de `doc_range_hits`.
    ///
    /// La fenêtre d'un site est bornée par la **prochaine liaison du même
    /// identifiant**, et cette borne est porteuse : `transport_failures.rs` lie
    /// trois fois `listener` dans le même fichier, dont un site fautif encadré par
    /// deux sites légitimes. Sans la borne, le `listener.accept()` du troisième
    /// site rendrait le second vert.
    fn dead_listener_hits(src: &str) -> Vec<String> {
        let code = crate::source_scan::strip_comment_lines(src);

        // (début de ligne, fin de ligne, identifiant)
        let mut bindings: Vec<(usize, usize, String)> = Vec::new();
        let mut offset = 0usize;
        for line in code.lines() {
            let end = offset + line.len();
            if line.contains("TcpListener::bind")
                && let Some(name) = dead_listener_binding_name(line)
            {
                bindings.push((offset, end, name));
            }
            offset = end + 1; // le `\n` réinséré par `strip_comment_lines`
        }

        let mut hits = Vec::new();
        for (i, (_, line_end, name)) in bindings.iter().enumerate() {
            let limit = bindings[i + 1..]
                .iter()
                .find(|(_, _, n)| n == name)
                .map_or(code.len(), |(start, _, _)| *start);
            let from = (*line_end).min(code.len());
            let to = limit.max(from).min(code.len());
            if !dead_listener_has_real_use(&code[from..to], name) {
                hits.push(name.clone());
            }
        }
        hits
    }

    /// **Un recensement fermé, pas une allowlist — et la différence est de fond**
    /// (même distinction que `PILOT_DISPATCH_SITES` de mika#2506 ci-dessous).
    ///
    /// # Ce que le plan mika#2569 annonçait, et ce que l'arbre porte
    ///
    /// Le plan prévoyait cette constante **vide**, sur la foi d'un recensement
    /// (« R1 ») qui listait cinq sites. Le recensement est **incomplet d'un
    /// site** : `crates/mika-agent/tests/smoke.rs::free_port` porte exactement le
    /// même motif — lier `127.0.0.1:0`, relever le port, laisser l'écouteur
    /// mourir — et n'y figure pas. Livrer la constante vide aurait demandé soit
    /// de rétrécir le prédicat jusqu'à ne plus le voir, soit de retirer le
    /// littéral de ce fichier pour faire taire le scan. Les deux sont la
    /// décoration que ce module refuse ailleurs en toutes lettres.
    ///
    /// # Pourquoi ce site ne peut PAS être migré
    ///
    /// `free_port` a l'intention **inverse** : il veut un port qu'un
    /// `mika-spirit` fraîchement lancé pourra **lier**. Un `DeadEndpoint` tient le
    /// port, donc le serveur échouerait à démarrer. Sa course est réelle et de la
    /// même famille, mais son remède — tenir le port — lui est structurellement
    /// indisponible, puisqu'il doit passer le port à un autre processus.
    ///
    /// # Ce que ça n'autorise PAS
    ///
    /// **Quand le scan tire sur un site NEUF, on le migre vers `DeadEndpoint` ; on
    /// n'ajoute pas de ligne ici** (doctrine mika#2201, « on déclare, on
    /// n'allowliste pas »). Une entrée de plus se paie d'un ticket qui pèse
    /// pourquoi ce site-là ne peut pas tenir son port.
    ///
    /// Comparé **dans les deux sens** par le test plus bas : une entrée qui ne
    /// désigne plus un site fautif rougit, sans quoi elle exempterait
    /// silencieusement un futur homonyme (motif `FIRED_AT_LITERAL_WRITERS`).
    const DEAD_LISTENER_CENSUS: &[&str] = &["crates/mika-agent/tests/smoke.rs"];

    /// **AC1 structurellement — aucun site n'obtient un endpoint mort en libérant
    /// un port.**
    ///
    /// Aucun test comportemental ne peut voir cette classe. Un sixième site écrit
    /// demain ne rendrait **aucune** décision fausse : il passerait, sauf une fois
    /// sur cent, sur une machine chargée, dans un job dont on relance le rouge
    /// sans le lire. C'est cette signature qui justifie un scan de source.
    #[test]
    fn mika2569_aucune_fixture_nobtient_un_endpoint_mort_en_liberant_un_port() {
        let sources = all_rust_sources();

        // Anti-vacuité sur la POPULATION : le motif vit dans le code de test, et
        // `is_test_source_path` répond `false` pour `cadence.rs` (test inline sous
        // `src/`), donc la population est délibérément NON filtrée. Si quelqu'un
        // la recâble un jour sur `production_sources()` — le réflexe, puisque
        // c'est ce que font ses voisines — le scan se tairait en ayant l'air sain.
        assert!(
            sources
                .iter()
                .any(|(rel, _)| crate::source_scan::is_test_source_path(Path::new(rel))),
            "mika#2569 — la population du scan ne contient aucune source de test : elle a \
             été recâblée sur la moitié de production, où le motif cherché ne vit presque \
             jamais. Le scan est alors décoratif (mika#2103)."
        );

        let mut offenders = Vec::new();
        for (rel, content) in &sources {
            if DEAD_LISTENER_CENSUS.contains(&rel.as_str()) {
                continue;
            }
            for name in dead_listener_hits(content) {
                offenders.push(format!(
                    "{rel} : `{name}` n'est lié que pour rendre son adresse"
                ));
            }
        }

        assert!(
            offenders.is_empty(),
            "mika#2569 — un écouteur est lié puis libéré pour servir d'endpoint « mort » :\n  \
             {}\n\n\
             RÉSOLUTION : remplacer la fixture par `mika_common::dead_endpoint::DeadEndpoint`, \
             qui RÉSERVE le port (socket lié, jamais `listen()`, jamais `SO_REUSEADDR`) au lieu \
             de le libérer. Tant que le garde vit, aucun `bind(\"127.0.0.1:0\")` concurrent ne \
             peut recevoir ce port, et toute connexion vers lui est refusée immédiatement.\n\n\
             Ne PAS ajouter d'entrée à DEAD_LISTENER_CENSUS : une entrée se paie d'un ticket \
             qui établit pourquoi ce site ne peut pas tenir son port.\n\n\
             Un écouteur SERVI (`axum::serve(l, …)`) ou ACCEPTÉ (`l.accept()`) n'est pas \
             accusé. Si cette ligne apparaît pour de la prose, c'est le dépouillement de \
             `source_scan::strip_comment_lines` qu'il faut lire, pas l'aiguille qu'il faut \
             rétrécir.",
            offenders.join("\n  ")
        );
    }

    /// Plante une fixture en substituant `@BIND@` par l'appel de liaison,
    /// **assemblé à l'exécution**.
    ///
    /// Sans cette indirection, les fixtures ci-dessous feraient rougir le scan sur
    /// sa propre définition — et un scan rouge en permanence est un scan qu'on
    /// désarme. Même motif et même raison que `doc_range_needles` ci-dessus.
    fn plant_dead_listener(src: &str) -> String {
        src.replace("@BIND@", concat!("TcpListener", "::bind"))
    }

    /// **Contrôle de non-vacuité — le scan attrape les formes plantées.**
    ///
    /// Sans lui, « la garde se tait » et « la garde ne regarde rien » se lisent
    /// pareil. Les deux formes sont celles que l'arbre portait : `drop` explicite
    /// (`remote_ask_integration`, `cadence`) et bloc d'initialisation
    /// (`remote_ask_recovery` ×2, `transport_failures`).
    #[test]
    fn mika2569_le_scan_attrape_les_deux_formes_plantees() {
        let drop_explicite = plant_dead_listener(
            r#"
            let listener = @BIND@("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            drop(listener);
            let url = format!("http://{addr}/health");
        "#,
        );
        assert_eq!(
            dead_listener_hits(&drop_explicite),
            vec!["listener".to_string()],
            "le scan ne voit plus la forme `drop` explicite"
        );

        let bloc_initialisation = plant_dead_listener(
            r#"
            let addr = {
                let listener = @BIND@("127.0.0.1:0").await.unwrap();
                listener.local_addr().unwrap()
            };
        "#,
        );
        assert_eq!(
            dead_listener_hits(&bloc_initialisation),
            vec!["listener".to_string()],
            "le scan ne voit plus la forme « bloc d'initialisation »"
        );

        // La forme multi-ligne de `cadence.rs` : l'usage traverse un retour à la
        // ligne. Un prédicat à la ligne lirait `let port = listener` comme un
        // usage légitime et raterait le site.
        let usage_multiligne = plant_dead_listener(
            r#"
            let listener = std::net::@BIND@("127.0.0.1:0").unwrap();
            let port = listener
                .local_addr()
                .unwrap()
                .port();
        "#,
        );
        assert_eq!(
            dead_listener_hits(&usage_multiligne),
            vec!["listener".to_string()],
            "le scan rate l'usage qui traverse un retour à la ligne"
        );

        // **La forme EXACTE du site fondateur de mika#2569**, et le terme que le
        // plan n'avait pas : le mot `listener` apparaît dans une chaîne littérale
        // (`.expect_err("… no listener accepts …")`). Sans le dépouillement des
        // littéraux, il compte comme un usage légitime et le site que ce ticket
        // existe pour refuser passe en vert. Mesuré à la livraison sur la version
        // HEAD de `remote_ask_integration.rs`.
        let mot_dans_une_chaine = plant_dead_listener(
            r#"
            let listener = @BIND@("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            drop(listener);
            let err = dispatch_remote("hi", &url)
                .await
                .expect_err("should fail when no listener accepts the connection");
        "#,
        );
        assert_eq!(
            dead_listener_hits(&mot_dans_une_chaine),
            vec!["listener".to_string()],
            "le scan compte une occurrence dans un littéral de chaîne comme un usage : \
             il rate le site fondateur de mika#2569"
        );
    }

    /// **L'autre moitié — une fixture de bonne foi n'est PAS accusée.**
    ///
    /// Une aiguille qui n'attraperait plus rien et un prédicat qui accuserait tout
    /// produisent deux verts différents ; seul le couple les distingue. Un scan
    /// rouge en permanence est un scan qu'on désarme.
    #[test]
    fn mika2569_le_scan_epargne_une_fixture_de_bonne_foi() {
        let servi = plant_dead_listener(
            r#"
            let listener = @BIND@("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, app.into_make_service()).await.unwrap();
            });
        "#,
        );
        assert!(
            dead_listener_hits(&servi).is_empty(),
            "un écouteur SERVI est accusé : le scan rougirait sur tous les serveurs factices"
        );

        let accepte = plant_dead_listener(
            r#"
            let listener = @BIND@("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let held = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
            });
        "#,
        );
        assert!(
            dead_listener_hits(&accepte).is_empty(),
            "un écouteur ACCEPTÉ est accusé"
        );

        // La prose qui DÉCRIT le motif fautif n'est pas une violation — c'est le
        // faux positif que le doc-comment de chaque fixture réparée produirait.
        let prose = plant_dead_listener(
            r#"
            // let listener = @BIND@("127.0.0.1:0").await.unwrap();
            /// Le motif précédent était `drop(listener)` après `local_addr`.
        "#,
        );
        assert!(
            dead_listener_hits(&prose).is_empty(),
            "le scan accuse une mention en commentaire : la réparation deviendrait indicible"
        );

        // Trois liaisons du même nom dans un fichier, dont la fautive est ENCADRÉE
        // par deux légitimes. C'est la forme réelle de `transport_failures.rs`, et
        // la seule que le bornage par « prochaine liaison du même identifiant »
        // existe pour tenir.
        let encadree = plant_dead_listener(
            r#"
            let listener = @BIND@("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (s, _) = listener.accept().await.unwrap();

            let addr2 = {
                let listener = @BIND@("127.0.0.1:0").await.unwrap();
                listener.local_addr().unwrap()
            };

            let listener = @BIND@("127.0.0.1:0").await.unwrap();
            let addr3 = listener.local_addr().unwrap();
            let (s3, _) = listener.accept().await.unwrap();
        "#,
        );
        assert_eq!(
            dead_listener_hits(&encadree),
            vec!["listener".to_string()],
            "le bornage par la prochaine liaison du même identifiant ne tient pas : \
             un site fautif encadré par deux sites légitimes passe en vert"
        );
    }

    /// Le pendant auto-nettoyant du recensement : comparé **dans les deux sens**.
    ///
    /// Une entrée qui ne désigne plus un site fautif est retirée le jour de sa
    /// péremption, et non des mois plus tard — sans quoi elle exempterait
    /// silencieusement un futur homonyme au même chemin.
    #[test]
    fn mika2569_le_recensement_ne_nomme_que_des_sites_reellement_fautifs() {
        let sources = all_rust_sources();

        for entry in DEAD_LISTENER_CENSUS {
            let found = sources.iter().find(|(rel, _)| rel == entry);
            let Some((_, content)) = found else {
                panic!(
                    "mika#2569 — le recensement nomme `{entry}`, qui n'existe plus. Une \
                     exception périmée exempte silencieusement un futur homonyme : la \
                     retirer."
                );
            };
            assert!(
                !dead_listener_hits(content).is_empty(),
                "mika#2569 — `{entry}` est recensé mais ne porte plus le motif : le site a \
                 été réparé, l'entrée doit partir avec lui."
            );
        }
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

    /// Les deux motifs de refus sont un **format de fil** : ils atterrissent dans
    /// `audit_events.after_value` et dans le JSON de `tasks.result`, et
    /// l'opérateur en fait des `GROUP BY`. Un motif ajouté ou retiré est une
    /// RUPTURE à dater dans `CLAUDE.md`, jamais une mise à jour de ce nombre en
    /// silence — même contrat que `ALL_ITERATE_REFUSAL_REASONS` (mika#2506).
    #[test]
    fn mika2545_the_two_refusal_verdicts_are_a_wire_format() {
        use crate::skills::executor::ALL_GROOM_ESCALATE_VERDICTS;

        assert_eq!(
            ALL_GROOM_ESCALATE_VERDICTS.len(),
            2,
            "mika#2545 — deux motifs : « ce ticket a escaladé » et « on n'a pas \
             pu le savoir ». Ils appellent la même disposition et DEUX lectures \
             opérateur différentes (le premier est le régime attendu non vide, le \
             second doit rester vide), donc les fondre rendrait la seconde \
             population incomptable."
        );
        let mut sorted = ALL_GROOM_ESCALATE_VERDICTS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ALL_GROOM_ESCALATE_VERDICTS.len(),
            "deux motifs portent la même valeur de fil : une population serait \
             coupée en deux sans le dire"
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

    // ─────────────────────────────────────────────────────────────────────
    // mika#2474 — un écrivain du nom de dépassement, un lecteur du seuil.
    //
    // Les deux gardes vivent ici parce que c'est le module des scans de nom :
    // il porte déjà `production_sources()` (qui parcourt `crates/` ENTIER, donc
    // `mika-common` comme `mika-agent` — ce qu'un `ProductionScanner::for_crate`
    // ne saurait pas faire) et `string_literals()`.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Population pré-existante : **zéro, mesurée** — le nom est neuf, et
    /// `grep -rn brief_size_overrun crates/` ne rendait aucune ligne à HEAD
    /// `b6c95955`. Il n'y a donc rien à excepter, ni de case où déposer la
    /// prochaine infraction (mika#2323). Quand le scan tire, **on retire le
    /// second site**, on ne l'allowliste pas (doctrine mika#2201).
    const BRIEF_SIZE_OVERRUN_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// **Livrée vide**, même conduite. Un second lecteur du seuil serait, en
    /// pratique, le **refus** que mika#2474 décline de livrer sans sa
    /// précondition (§ 9) — une garde dans `validate_dispatch_readiness` ou dans
    /// `_arch_ask` — et il couperait un groom sur une valeur calibrée pour une
    /// **alerte**. L'asymétrie est écrite : un faux positif d'alerte coûte une
    /// ligne de journal, un faux positif de refus coûte une passe d'architecte
    /// et un point du budget de re-drive (mika#2020 : trois abandonnent un
    /// ticket sain).
    const BRIEF_SIZE_THRESHOLD_READERS_ALLOWED: &[&str] = &[];

    /// Le fichier qui **définit** l'accesseur : ses propres lignes de définition
    /// ne sont pas des lectures.
    const BRIEF_SIZE_THRESHOLD_OWNER: &str = "crates/mika-common/src/config.rs";

    /// Les lignes de production qui **appellent** l'accesseur du seuil.
    ///
    /// Le prédicat est lexical et en trois termes, chacun ajouté pour une raison :
    /// (1) hors commentaire — la prose *sur* le seuil n'est pas une lecture, et ce
    /// dépôt en porte beaucoup (la classe du faux positif du Signal S, mika#2050) ;
    /// (2) portant le nom de l'accesseur ; (3) **sans `fn `** — ce qui écarte la
    /// ligne de définition sans avoir à exempter son fichier, une exemption de
    /// fichier aveuglant aussi tout appel qu'il viendrait à contenir.
    fn brief_size_threshold_reader_sites(content: &str) -> Vec<(usize, String)> {
        let needle = format!("effective_brief_size{}", "_alert_bytes");
        content
            .lines()
            .enumerate()
            .filter_map(|(i, line)| {
                let t = line.trim_start();
                if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
                    return None;
                }
                if !t.contains(needle.as_str()) || t.contains("fn ") {
                    return None;
                }
                Some((i + 1, t.to_string()))
            })
            .collect()
    }

    /// Le nom du dépassement a un seul écrivain, journal **et** `audit_events`.
    ///
    /// # Pourquoi un scan de source et pas un test comportemental
    ///
    /// Un second écrivain ne rendrait **aucune décision fausse** le jour où il
    /// est écrit : la mesure continuerait de fonctionner et toutes les assertions
    /// resteraient vertes. Ce qu'il casserait est le `GROUP BY target_key` de la
    /// sonde S2 — plus tard, en silence, sur un compte que personne ne saurait
    /// être devenu inexact.
    #[test]
    fn mika2474_the_overrun_name_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("brief_size{}", "_overrun");
        let owner = "crates/mika-agent/src/agent_loop/mod.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if BRIEF_SIZE_OVERRUN_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
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
            "mika#2474 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2474 — le nom du dépassement de taille de brief a un second \
             écrivain : {strangers:?}\n\n\
             RÉSOLUTION : faire passer ce site par \
             `agent_loop::BRIEF_SIZE_OVERRUN_EVENT`, ou le retirer. Ne PAS \
             l'ajouter à BRIEF_SIZE_OVERRUN_SOLE_WRITER_EXCEPTIONS — le compte \
             par agent qui conditionne le suivi n'est exact que tant qu'un seul \
             site l'écrit."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2161 — chacun des trois noms de cause a UNE fonction écrivante.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test jumeau l'assert.**
    ///
    /// Quand ce scan tire, une **seconde fonction** écrit l'un des trois noms —
    /// donc les deux surfaces d'une même cause (ligne de journal et
    /// `audit_events.target_key`) peuvent désormais divergier. « Ce second site
    /// écrit-il la même population ? » est une question que la garde ne peut pas
    /// trancher à la place de l'humain, et une mauvaise réponse scinde
    /// silencieusement un compteur d'opérateur. La résolution est donc
    /// **halt-and-surface** : router le site par `emit_empty_backlog_signal`, ou
    /// le retirer — jamais une entrée ici (doctrine mika#2201 ; une allowlist née
    /// vide est un emplacement où déposer la prochaine infraction, mika#2323).
    const EMPTY_BACKLOG_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// La fonction de production qui englobe chaque ligne, par indentation.
    ///
    /// Le découpage est volontairement grossier — une `fn` au niveau d'un `impl`
    /// ou du module — parce que la propriété à tenir l'est aussi : *un seul site
    /// décide*.
    ///
    /// # Les commentaires sont dépouillés par le lecteur unique, pas sur place
    ///
    /// [`crate::source_scan::strip_comment_lines`] et pas un prédicat local,
    /// parce que le prédicat local naïf (`starts_with('*')`) est **exactement le
    /// bug que le doc-comment de ce lecteur refuse par écrit** : `*guard = x;` est
    /// du Rust valide, et `auto_pull.rs` en porte un
    /// (`*counts.entry(…).or_insert(0) += 1;`). Un second écrivain posé sur une
    /// ligne de cette forme aurait été sauté avant que `string_literals` ne le
    /// voie, et ce scan serait resté vert. Sans dépouillement du tout, à l'inverse,
    /// `auto_pull.rs` se dénonce quatre fois sur sa propre prose — le piège que
    /// mika#2329 a dû nommer et le faux positif de prose du Signal S (mika#2050).
    ///
    /// # La détection de `fn` est indépendante de la position
    ///
    /// Le même prédicat que [`crate::source_scan::fn_bodies`], et pour la raison
    /// que son doc-comment écrit : une énumération de préfixes rate les
    /// permutations de visibilité × `async` × `const` × `unsafe`. Mesuré : un
    /// `pub(super) async fn` en colonne 0 existe dans ce crate, et un `const fn`
    /// indenté aussi. Le mode de panne est **silencieux dans le mauvais sens** —
    /// une forme non reconnue en colonne 0 laisse la portée au module, donc le
    /// littéral de son corps est imputé à `<module scope>`, se confond avec le site
    /// attendu, et le scan passe.
    fn enclosing_fns_writing(content: &str, needle: &str) -> Vec<String> {
        const MODULE_SCOPE: &str = "<module scope>";
        let mut current = String::from(MODULE_SCOPE);
        let mut out: Vec<String> = Vec::new();

        let stripped = crate::source_scan::strip_comment_lines(content);
        for line in stripped.lines() {
            let trimmed = line.trim_start();
            // Une ligne en colonne 0 qui n'ouvre pas une `fn` est un item de
            // niveau module (`const`, `struct`, `impl`, ou le `}` qui ferme la
            // précédente) : la portée revient au module. Sans ce retour, un
            // `const` déclaré après un `impl` serait attribué à la dernière `fn`
            // de cet `impl` — mesuré : les trois `const EVENT_*` d'`auto_pull.rs`
            // étaient imputés à `ExclusionPhase::as_str`.
            //
            // La CONTINUATION d'une signature est exclue, et ce terme est
            // porteur : une `fn` dont les paramètres tiennent sur plusieurs
            // lignes ferme sa signature par `) {` ou `) -> T {` **en colonne 0**,
            // donc sans cette exclusion tout son corps retombait en portée de
            // module. Mesuré : un littéral planté dans `emit_empty_backlog_signal`
            // — dont la signature est multi-ligne — laissait le scan VERT. `>`
            // couvre la continuation d'un générique ou d'un type de retour.
            let is_signature_continuation = trimmed.starts_with(')')
                || trimmed.starts_with("where")
                || trimmed.starts_with('{')
                || trimmed.starts_with(',')
                || trimmed.starts_with('+')
                || trimmed.starts_with('>');
            if !trimmed.is_empty()
                && !line.starts_with(char::is_whitespace)
                && !is_signature_continuation
            {
                current = String::from(MODULE_SCOPE);
            }
            // `fn ` à n'importe quelle position, borné à gauche par une frontière
            // de mot pour que `some_fn (` et `impl Fn(` ne comptent pas.
            if let Some(i) = trimmed.find("fn ")
                && (i == 0 || trimmed.as_bytes()[i - 1] == b' ')
            {
                current = trimmed[i + 3..]
                    .split(['(', '<', ' '])
                    .next()
                    .unwrap_or("<unnamed>")
                    .to_string();
            }
            let carries = string_literals(line).iter().any(|lit| lit.contains(needle));
            // A function is one site however many times it spells the name; the
            // module scope is NOT — each module-level literal is its own
            // declaration, so deduplicating it hid a second `const` (review of
            // PR #2635).
            if carries && (current == MODULE_SCOPE || !out.contains(&current)) {
                out.push(current.clone());
            }
        }

        out
    }

    /// **V8 — chaque nom de cause a exactement un site d'écriture, et c'est sa
    /// déclaration.**
    ///
    /// # Ce que la mesure a déplacé par rapport à la Fire-Disposition du plan
    ///
    /// Le plan prescrit de compter des **fonctions écrivantes** — « exactement une
    /// par nom » — parce qu'il décrit le code d'**avant** le correctif, où `info!`
    /// et `log_audit_event` portaient tous deux le littéral, dans la même
    /// fonction. Mesuré après : **zéro** fonction porte le littéral. Les deux
    /// surfaces passent par `EmptyBacklogCause::event_name`, qui rend une
    /// `const`, donc le seul site littéral de production est la **déclaration**.
    ///
    /// C'est strictement plus fort que ce que le plan demandait, et le scan le dit
    /// plutôt que de viser la forme disparue : formulé sur « une fonction », il
    /// aurait trouvé zéro et rougi à la naissance — un lint rouge au premier
    /// `cargo test` se fait désarmer avant d'avoir servi, ce que la
    /// Fire-Disposition nomme elle-même comme le piège à éviter.
    ///
    /// La propriété tenue est donc : **un nom, un littéral, en portée de module de
    /// `auto_pull.rs`**. Elle interdit ce qu'il fallait interdire — un second site
    /// épelant `"auto_feeder_pool_in_flight"` à la main — et l'unicité du site
    /// *décisionnel* est tenue à côté par le `match` exhaustif sans bras `_ =>` de
    /// `event_name` (une quatrième cause ne compile pas tant qu'elle n'a pas
    /// décidé de son nom).
    ///
    /// # Pourquoi un scan de source et pas un test comportemental
    ///
    /// Un second littéral écrit demain ne rendrait **aucune décision fausse** le
    /// jour où il est écrit : le classifieur continuerait de classifier et toutes
    /// les assertions resteraient vertes. Ce qu'il casserait est le
    /// `GROUP BY target_key` de la sonde — plus tard, en silence, sur un compte
    /// que personne ne saurait être devenu inexact.
    #[test]
    fn mika2161_chaque_nom_a_un_seul_ecrivain() {
        // Composés à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let names = [
            format!("auto_feeder{}", "_no_backlog"),
            format!("auto_feeder{}", "_pool_in_flight"),
            format!("auto_feeder{}", "_in_flight_unreadable"),
        ];
        let expected_site = "crates/mika-agent/src/auto_pull.rs::<module scope>";
        let mut witnesses = 0usize;

        // Lu UNE fois pour les trois aiguilles : la marche lit ~375 fichiers et
        // 16 Mo, et les deux scans voisins de ce fichier l'appellent déjà au niveau
        // supérieur. La coupe au module de test est obligatoire : le test de format
        // de fil d'`auto_pull.rs` porte les trois noms en littéraux, et sans la
        // coupe il compterait comme un second site.
        let sources = production_sources_to_test_module();

        for needle in &names {
            let mut sites: Vec<String> = Vec::new();
            for (rel, content) in &sources {
                if EMPTY_BACKLOG_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                    continue;
                }
                for f in enclosing_fns_writing(content, needle) {
                    sites.push(format!("{rel}::{f}"));
                }
            }

            // Anti-vacuité par le NOMBRE : zéro se lit exactement comme un scan
            // propre (mika#2103 / mika#2205), donc l'égalité stricte est ce qui
            // rend la garde non décorative.
            assert_eq!(
                sites,
                vec![expected_site.to_string()],
                "mika#2161 — `{needle}` doit avoir EXACTEMENT un site littéral en \
                 production : sa déclaration `const`.\n\n\
                 Zéro = ce scan vise un nom mort et ne vérifie rien (la constante \
                 a-t-elle été renommée ?). Deux ou plus = un site épelle le nom à \
                 la main, et les deux surfaces de cette cause peuvent désormais \
                 divergier.\n\
                 RÉSOLUTION (halt-and-surface) : faire passer ce site par \
                 `EmptyBacklogCause::event_name`, ou le retirer. Ne PAS l'ajouter \
                 à EMPTY_BACKLOG_SOLE_WRITER_EXCEPTIONS — « ce second site écrit-il \
                 la même population ? » n'est pas une question qu'une garde peut \
                 trancher à la place de l'humain."
            );
            witnesses += 1;
        }

        assert_eq!(witnesses, names.len(), "un nom n'a pas été vérifié");
    }

    /// Le pendant auto-nettoyant de l'allowlist mika#2161.
    #[test]
    fn mika2161_the_empty_backlog_allowlist_is_empty() {
        assert!(
            EMPTY_BACKLOG_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "EMPTY_BACKLOG_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain."
        );
    }

    /// Contrôle de bonne foi : le scan voit-il seulement une seconde fonction ?
    ///
    /// Sans lui, « le scan tient » est indistinguable de « le scan ne regarde
    /// rien » — la classe que son anti-vacuité couvre par le nombre et que
    /// celui-ci couvre par la **forme** du prédicat.
    ///
    /// Trois formes ensemble, et chacune a été vue manquante :
    ///
    /// 1. **La signature multi-ligne.** Son `) {` est en colonne 0, donc une
    ///    première version du prédicat ramenait le corps de la fonction en portée
    ///    de module — et un littéral planté dans `emit_empty_backlog_signal`
    ///    laissait le scan **vert**. C'est le terme que ce contrôle existe pour
    ///    tenir.
    /// 2. **Le `const` après un `impl`.** Sans retour en portée de module sur un
    ///    item de colonne 0, il était imputé à la dernière `fn` de cet `impl`.
    /// 3. **La prose.** Un doc-comment et un commentaire de ligne portant le nom
    ///    ne sont pas des écritures : les compter rendrait le scan rouge sur la
    ///    documentation qu'il protège (mika#2050, mika#2329).
    #[test]
    fn mika2161_le_scan_voit_une_seconde_fonction() {
        let needle = "auto_feeder_pool_in_flight";
        let fixture = "\
const EVENT_POOL_IN_FLIGHT: &str = \"auto_feeder_pool_in_flight\";

impl Cause {
    fn event_name(self) -> &'static str {
        EVENT_POOL_IN_FLIGHT
    }
}

/// Prose citant `auto_feeder_pool_in_flight` — ne doit PAS compter.
// Ni ce commentaire portant \"auto_feeder_pool_in_flight\".
async fn emit_empty_backlog_signal(
    db: &AsyncDatabase,
    cause: Cause,
) {
    info!(event = \"auto_feeder_pool_in_flight\");
}

fn un_second_site() {
    log(\"auto_feeder_pool_in_flight\");
}
";
        assert_eq!(
            enclosing_fns_writing(fixture, needle),
            vec![
                "<module scope>".to_string(),
                "emit_empty_backlog_signal".to_string(),
                "un_second_site".to_string()
            ],
            "le scan doit voir la déclaration en portée de module, le corps d'une \
             fonction à signature MULTI-LIGNE, et une seconde fonction — et \
             IGNORER les deux commentaires. Un terme manquant le rend soit \
             aveugle, soit rouge sur la prose qu'il protège."
        );
    }

    /// Contrôle de bonne foi n°2 : un SECOND littéral en portée de module est un
    /// second site (revue de la PR #2635).
    ///
    /// Le scan dédoublonnait les portées : tous les littéraux de portée module
    /// d'`auto_pull.rs` se fondaient dans l'unique `<module scope>` attendu, donc
    /// `const LEGACY: &str = "auto_feeder_no_backlog";` posé à côté de la
    /// déclaration laissait l'égalité stricte verte — exactement le second
    /// écrivain que le scan interdit.
    ///
    /// Rouge-avant : avec la déduplication, la fixture rendait une seule entrée.
    #[test]
    fn mika2161_le_scan_compte_chaque_litteral_de_portee_module() {
        let needle = "auto_feeder_pool_in_flight";
        let fixture = "\
const EVENT_POOL_IN_FLIGHT: &str = \"auto_feeder_pool_in_flight\";
const LEGACY_POOL_IN_FLIGHT: &str = \"auto_feeder_pool_in_flight\";
";
        assert_eq!(
            enclosing_fns_writing(fixture, needle),
            vec!["<module scope>".to_string(), "<module scope>".to_string()],
            "deux littéraux en portée de module sont deux sites, pas un"
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2474_the_overrun_sole_writer_allowlist_is_empty() {
        assert!(
            BRIEF_SIZE_OVERRUN_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "BRIEF_SIZE_OVERRUN_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain. Une \
             allowlist née vide est un emplacement où déposer la prochaine \
             infraction (mika#2323)."
        );
    }

    /// Le seuil a un seul lecteur de production : le site d'émission.
    #[test]
    fn mika2474_the_threshold_has_a_single_reader() {
        let expected = "crates/mika-agent/src/agent_loop/mod.rs";
        let mut sites: Vec<String> = Vec::new();
        let mut owner_seen = false;

        for (rel, content) in production_sources() {
            if BRIEF_SIZE_THRESHOLD_READERS_ALLOWED.contains(&rel.as_str()) {
                continue;
            }
            if rel == BRIEF_SIZE_THRESHOLD_OWNER {
                // L'accesseur EXISTE-t-il encore ? Sans ce terme, un renommage
                // rendrait le scan silencieux et donc décoratif.
                owner_seen = content.contains("effective_brief_size_alert_bytes");
                continue;
            }
            for (line, text) in brief_size_threshold_reader_sites(&content) {
                sites.push(format!("{rel}:{line}: {text}"));
            }
        }

        assert!(
            owner_seen,
            "mika#2474 — `effective_brief_size_alert_bytes` n'existe plus dans \
             {BRIEF_SIZE_THRESHOLD_OWNER} : ce scan vise un nom mort"
        );
        assert_eq!(
            sites.len(),
            1,
            "mika#2474 — attendu EXACTEMENT un lecteur de production du seuil : le \
             site d'émission. Trouvé {} :\n{}\n\n\
             RÉSOLUTION : retirer le second lecteur, ne pas l'allowlister. Un \
             second lecteur est en pratique un REFUS, et il couperait un groom sur \
             un seuil calibré pour une alerte — un faux positif d'alerte coûte une \
             ligne, un faux positif de refus coûte une passe d'architecte.",
            sites.len(),
            sites.join("\n")
        );
        assert!(
            sites[0].starts_with(expected),
            "mika#2474 — le lecteur unique doit être le site d'émission dans \
             {expected} :\n{}",
            sites[0]
        );
    }

    /// **Contrôle de bonne foi du scan ci-dessus.**
    ///
    /// Il pourrait être vert parce qu'il ne regarde rien — très exactement le
    /// mode de panne qu'il existe pour rendre visible. On le montre donc rougir
    /// sur un lecteur ajouté ailleurs, et rester muet sur les trois formes qui
    /// nomment le seuil sans le lire.
    #[test]
    fn mika2474_the_reader_scan_reddens_on_a_second_reader() {
        let offending = "fn gate(settings: &Settings) -> bool {\n    \
             bytes > settings.effective_brief_size_alert_bytes()\n}\n";
        assert_eq!(
            brief_size_threshold_reader_sites(offending).len(),
            1,
            "le scan doit voir un lecteur ajouté hors du site d'émission"
        );

        // 1. La prose d'un doc-comment.
        let prose = "/// Voir `Settings::effective_brief_size_alert_bytes` pour les \
                     trois paliers.\n";
        assert!(
            brief_size_threshold_reader_sites(prose).is_empty(),
            "nommer le seuil dans un commentaire n'est pas le lire"
        );

        // 2. Un commentaire de bloc, et une ligne de continuation.
        let block = "/* effective_brief_size_alert_bytes */\n \
                     * effective_brief_size_alert_bytes\n";
        assert!(
            brief_size_threshold_reader_sites(block).is_empty(),
            "un commentaire de bloc n'est pas une lecture"
        );

        // 3. La DÉFINITION elle-même — c'est ce qui dispense d'exempter son
        //    fichier, une exemption de fichier aveuglant aussi tout appel qu'il
        //    viendrait à contenir.
        let definition = "    pub fn effective_brief_size_alert_bytes(&self) -> i64 {\n";
        assert!(
            brief_size_threshold_reader_sites(definition).is_empty(),
            "la ligne de définition n'est pas un site d'appel"
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist du lecteur.
    #[test]
    fn mika2474_the_threshold_reader_allowlist_is_empty() {
        assert!(
            BRIEF_SIZE_THRESHOLD_READERS_ALLOWED.is_empty(),
            "BRIEF_SIZE_THRESHOLD_READERS_ALLOWED est livrée vide et doit le \
             rester : quand le scan tire, on retire le second lecteur (doctrine \
             mika#2201)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2575 — le nom d'audit du ré-armement d'une récurrente en vol.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Le nom `recurring_restart_restore` est **neuf** : il n'y a rien à
    /// excepter à la livraison, et c'est vérifiable. Quand ce scan tire, **on
    /// retire le second écrivain** (doctrine mika#2201) — une allowlist née vide
    /// est un emplacement où déposer la prochaine infraction (mika#2323).
    const RECURRING_RESTART_RESTORE_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// Le nom sert de `tool_name` d'audit aux **deux** issues du ré-armement,
    /// l'issue étant portée par `after_value` (motif `ready_label_outcome`,
    /// mika#2323). C'est ce qui rend soustractible le `GROUP BY after_value` de
    /// la sonde S5 — et donc ce qui distingue « le ré-armement a tenu » de « le
    /// cron était cassé » par une requête plutôt que par un grep.
    ///
    /// Aucun test comportemental ne peut voir cette classe : un second écrivain
    /// ne rendrait **aucune** décision fausse le jour où il est écrit, il
    /// rendrait le compte inexact, en silence, tous les tests au vert.
    #[test]
    fn mika2575_le_nom_daudit_a_un_seul_ecrivain() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("recurring_restart{}", "_restore");
        let owner = "crates/mika-agent/src/task_engine/engine.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if RECURRING_RESTART_RESTORE_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
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
            "mika#2575 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2575 — le nom d'audit du ré-armement a un second écrivain : \
             {strangers:?}\n\n\
             RÉSOLUTION : faire passer ce site par \
             `task_engine::engine`'s `RECURRING_RESTART_RESTORE_AUDIT`, ou le \
             retirer. Ne PAS l'ajouter à \
             RECURRING_RESTART_RESTORE_SOLE_WRITER_EXCEPTIONS — le `GROUP BY \
             after_value` de la sonde S5 n'est exact que tant qu'un seul site \
             l'écrit."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2575_lallowlist_du_nom_daudit_est_vide() {
        assert!(
            RECURRING_RESTART_RESTORE_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "RECURRING_RESTART_RESTORE_SOLE_WRITER_EXCEPTIONS est livrée vide et \
             doit le rester : quand le scan tire, on retire le second écrivain."
        );
    }

    /// Les deux `after_value` du ré-armement sont un **format de fil** : ils
    /// atterrissent dans `audit_events.after_value` et l'opérateur en fait des
    /// `GROUP BY`. Deux orthographes couperaient une population en deux sans le
    /// dire — c'est ce que la scission datée de mika#2361 a dû écrire une fois.
    ///
    /// Les valeurs sont figées ici plutôt que dans `engine.rs` pour la même
    /// raison que les autres formats de fil de ce fichier : un renommage est une
    /// **rupture à dater**, jamais une mise à jour de test en silence.
    #[test]
    fn mika2575_les_valeurs_daudit_sont_un_format_de_fil() {
        let owner = repo_root().join("crates/mika-agent/src/task_engine/engine.rs");
        let src = std::fs::read_to_string(&owner).expect("engine.rs lisible");

        for (konst, value) in [
            ("RECURRING_RESTORE_OUTCOME_REARMED", "recurring_active"),
            ("RECURRING_RESTORE_OUTCOME_NO_CRON", "failed_no_cron"),
        ] {
            let decl = format!("const {konst}: &str = \"{value}\";");
            assert!(
                src.contains(&decl),
                "mika#2575 — `{konst}` ne vaut plus `{value}`.\n\n\
                 Ces deux valeurs sont un FORMAT DE FIL : l'opérateur en fait des \
                 `GROUP BY after_value`. Les changer est une rupture à dater dans \
                 `CLAUDE.md`, pas une mise à jour de test."
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#1745 — le nom du signal surface-for-adoption a un seul écrivain.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Zéro violation existante, et c'est vérifiable plutôt que cru : le nom
    /// `surface_for_adoption` est **créé par mika#1745**, donc la population
    /// des violations préexistantes est vide par construction. Il n'y a rien à
    /// excepter — et une allowlist née non vide serait un emplacement où
    /// déposer la prochaine infraction (doctrine mika#2323).
    ///
    /// **Quand le scan tire, on retire le second site d'écriture ; on n'ajoute
    /// pas d'entrée** (doctrine mika#2201).
    const SURFACE_FOR_ADOPTION_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// Le prédicat du scan mika#1745, extrait pour être exerçable sur un contenu
    /// fabriqué.
    ///
    /// Sans cette extraction, « le scan est propre » et « le scan ne regarde
    /// rien » rendent le même vert, et la seule façon de les distinguer est une
    /// mutation à la main que personne ne rejoue (classe mika#2205, appliquée au
    /// scan lui-même).
    fn carries_bare_surface_literal(content: &str, needle: &str) -> bool {
        content
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
            })
            .any(|line| string_literals(line).iter().any(|lit| lit.trim() == needle))
    }

    /// Le nom du signal est écrit à **un** endroit, dans le journal comme dans
    /// `audit_events`.
    ///
    /// # Pourquoi un scan de source et pas un test comportemental
    ///
    /// Un second écrivain ne rend **aucune décision fausse** le jour où il est
    /// écrit : le handler continue de surfacer, la notification continue de
    /// partir, et toutes les assertions restent vertes. Ce qu'il casse est la
    /// requête opérateur — `SELECT count(*) … WHERE tool_name =
    /// 'surface_for_adoption' GROUP BY target_key` — qui **est** la mesure de la
    /// population, et donc la précondition explicite de la décision
    /// d'auto-adoption qu'AC3 diffère (« until we have enough n »). Elle
    /// cesserait de compter un fait pour compter deux populations mêlées, en
    /// silence. C'est très exactement la classe qu'aucun test de comportement ne
    /// peut voir.
    ///
    /// # Le prédicat est l'ÉGALITÉ, jamais la sous-chaîne
    ///
    /// Trois faux positifs mesurés l'imposent, et ils ne vont pas tous dans le
    /// même sens : `surface_for_adoption_skipped` et
    /// `surface_for_adoption_audit_failed` (dans le fichier propriétaire) sont
    /// des noms d'événement **voisins** qui portent le nom sans être lui, et
    /// `surface_for_adoption_unrecognized_value` (`mika-common/src/config.rs`,
    /// la moitié réglage) est dans un **autre crate** — un prédicat par
    /// `contains` l'accuserait comme second écrivain alors qu'il ne touche ni le
    /// journal du signal ni `audit_events`. Ce qu'un second écrivain porterait
    /// réellement est le littéral nu.
    ///
    /// # Angle mort HÉRITÉ, mesuré, et nommé plutôt que découvert
    ///
    /// [`production_sources`] tronque chaque fichier à la **première**
    /// occurrence textuelle de `#[cfg(test)]`, « où qu'elle soit » — y compris
    /// dans un doc-comment. `webhook_dispatch.rs` en cite une ligne 110, donc un
    /// second écrivain planté ligne 266 de ce fichier-là est **invisible** à ce
    /// scan : vérifié par mutation pendant l'écriture de mika#1745, où la sonde
    /// n'a pas rougi. La mutation équivalente dans `ci_success_handler.rs` (dont
    /// le premier `#[cfg(test)]` est son vrai module de test) rougit bien.
    ///
    /// La limite est partagée par tous les scans de ce fichier et n'est pas le
    /// périmètre de mika#1745 — la réparer veut dire changer l'énumérateur pour
    /// tous. Ce qui la rend supportable est
    /// [`mika1745_the_writer_predicate_sees_a_bare_literal`], qui atteste que le
    /// **prédicat** mord indépendamment de ce que l'énumérateur lui donne à
    /// lire : quand la garde se taira, on saura lequel des deux interroger.
    #[test]
    fn mika1745_the_surface_name_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("surface_for{}", "_adoption");
        let owner = "crates/mika-agent/src/server/ci_failure_handler.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources() {
            if SURFACE_FOR_ADOPTION_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            if carries_bare_surface_literal(&content, &needle) {
                writers.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme
        // un scan propre (mika#2103 / mika#2205).
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#1745 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#1745 — le nom du signal surface-for-adoption a un second \
             écrivain : {strangers:?}\n\n\
             RÉSOLUTION : retirer le second site. Ne PAS l'ajouter à \
             SURFACE_FOR_ADOPTION_SOLE_WRITER_EXCEPTIONS — le compte qui \
             conditionne la décision d'auto-adoption (AC3) n'est exact que tant \
             qu'un seul site écrit ce nom."
        );
    }

    /// Contrôle de bonne foi : le prédicat voit un second écrivain, et il ne voit
    /// **pas** les trois formes voisines qui lui ressemblent.
    ///
    /// Les quatre cas négatifs sont ceux mesurés pendant l'écriture, et chacun
    /// serait un faux positif permanent — donc une garde qu'on finit par museler.
    #[test]
    fn mika1745_the_writer_predicate_sees_a_bare_literal() {
        let needle = format!("surface_for{}", "_adoption");

        assert!(
            carries_bare_surface_literal(
                &format!("    db.log_audit_event(sid, \"{needle}\", &key).await;"),
                &needle
            ),
            "un second écrivain porte le littéral nu — le prédicat doit le voir"
        );
        assert!(
            carries_bare_surface_literal(
                &format!("    info!(event = \"{needle}\", x = 1);"),
                &needle
            ),
            "le journal compte autant que la base"
        );

        for benign in [
            // Noms d'événement voisins, dans le fichier propriétaire.
            format!("    info!(event = \"{needle}_skipped\", reason = \"x\");"),
            format!("    warn!(event = \"{needle}_audit_failed\");"),
            // La moitié réglage, dans un AUTRE crate.
            format!("    tracing::warn!(event = \"{needle}_unrecognized_value\");"),
            // Une mention n'est pas une instruction (classe mika#2050).
            format!("    /// Voir `{needle}` pour le contrat."),
        ] {
            assert!(
                !carries_bare_surface_literal(&benign, &needle),
                "faux positif sur une forme voisine : {benign}"
            );
        }
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika1745_the_sole_writer_allowlist_is_empty() {
        assert!(
            SURFACE_FOR_ADOPTION_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "SURFACE_FOR_ADOPTION_SOLE_WRITER_EXCEPTIONS est livrée vide et doit \
             le rester : quand le scan tire, on retire le second écrivain. Une \
             allowlist née vide est un emplacement où déposer la prochaine \
             infraction (mika#2323)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2627 — le prédicat testimony a DEUX lecteurs de production
    // ─────────────────────────────────────────────────────────────────────

    /// Les **trois** lecteurs nommés : la garde 5h, le helper d'outil, et le
    /// point de pose du livrable d'équipe.
    ///
    /// # Pourquoi un troisième, et pourquoi ce n'est pas une exemption
    ///
    /// mika#2627 avait figé la population à deux, avec la résolution écrite
    /// « router ce site vers `tools::check_testimony_access_proposal` ». Ce
    /// helper prend un `&ToolContext` et rend un `Option<ToolOutput>` :
    /// `TeamEngine` n'a ni l'un ni l'autre, et surtout **un livrable refusé ne
    /// se répare ni par un renvoi ni par un découpage** — sa disposition est
    /// nécessairement différente (une re-rédaction, puis une ligne neutre,
    /// décision opérateur MPC 2026-10-02).
    ///
    /// Donc on **étend le recensement** de deux à trois, exactement comme
    /// mika#2627 l'avait étendu de un à deux. *Un recensement n'est pas une
    /// allowlist : on y ajoute, on n'y exempte pas* — et l'allowlist reste vide.
    ///
    /// # Ce qui reste partagé, et ce qui compense la garantie perdue
    ///
    /// Le **vocabulaire de canal** : les trois lecteurs émettent le même nom
    /// d'événement sous une valeur de `channel` distincte, ce que
    /// `mika2633_tout_emetteur_de_la_famille_porte_un_canal` tient. C'est ce qui
    /// rend `jq 'select(.channel == …)'` exact plutôt qu'un filtre sur lequel
    /// trois sites peuvent diverger — la garantie que le scan A donnait jusqu'ici
    /// *par accident*, en bornant la population à deux.
    const TESTIMONY_PREDICATE_READERS: &[&str] = &[
        "crates/mika-agent/src/agent_loop/mod.rs",
        "crates/mika-agent/src/tools/mod.rs",
        "crates/mika-agent/src/teams/engine.rs",
    ];

    /// Les fichiers de production qui lisent réellement
    /// `detect_testimony_access_proposal`.
    ///
    /// **Un seul lecteur de cette question**, partagé par le scan A (qui refuse
    /// un lecteur non recensé) et par le scan B (dont le terme
    /// `TESTIMONY_SENDER_COVERED_UPSTREAM` exige que le fichier amont déclaré en
    /// soit un). Deux copies pourraient diverger, et c'est la classe que
    /// `grooming_marker` a dû graver une fois (mika#2158).
    ///
    /// La définition n'est pas un lecteur, et un commentaire qui NOMME la
    /// fonction n'en est pas un non plus — le prédicat porte trois paragraphes
    /// de prose à son sujet (classe mika#2050, le faux positif mesuré sur le
    /// Signal S).
    fn testimony_predicate_readers() -> Vec<String> {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("detect_testimony_access{}(", "_proposal");
        let mut readers = Vec::new();
        for (rel, content) in production_sources() {
            let reads = crate::source_scan::strip_comment_lines(&content)
                .lines()
                .any(|line| {
                    line.contains(needle.as_str())
                        && !line.contains("fn detect_testimony_access_proposal(")
                });
            if reads {
                readers.push(rel);
            }
        }
        readers
    }

    /// **Livrée vide, et le test frère l'assert.**
    ///
    /// Quand ce scan tire, **on route le site vers le helper** ; on ne
    /// l'allowliste pas (doctrine mika#2201). Un troisième lecteur direct du
    /// prédicat est un site qui refait la composition refus + télémétrie à sa
    /// façon, c'est-à-dire qui peut en diverger.
    const TESTIMONY_PREDICATE_READERS_ALLOWED: &[&str] = &[];

    /// Scan A — `detect_testimony_access_proposal` n'est lu qu'à deux endroits
    /// de production (mika#2627 R3).
    ///
    /// **Il remplace la V4 par-`grep` de la phase 2, qui s'inverse une seconde
    /// fois.** Le doc-comment du prédicat affirmait « V4 now requires **exactly
    /// one** production wiring site […] a call anywhere else is the double wiring
    /// RK6 names » ; après mika#2627 il y en a deux, et cette V4 n'était pas un
    /// test automatisé — donc rien ne rougissait, et laissée en place elle aurait
    /// prescrit de supprimer le second site comme un doublon.
    ///
    /// Aucun test comportemental ne voit cette classe : un troisième lecteur ne
    /// rend **aucune** décision fausse le jour où il est écrit — tout reste vert
    /// et seule la couverture se perd, en silence (classe `grooming_marker`,
    /// mika#2158).
    #[test]
    fn mika2627_le_predicat_na_que_deux_lecteurs_de_production() {
        let readers: Vec<String> = testimony_predicate_readers()
            .into_iter()
            .filter(|r| !TESTIMONY_PREDICATE_READERS_ALLOWED.contains(&r.as_str()))
            .collect();

        // Anti-vacuité : un scan qui ne trouve personne se lit exactement comme
        // un scan propre (mika#2103 / mika#2205).
        for expected in TESTIMONY_PREDICATE_READERS {
            assert!(
                readers.iter().any(|r| r == expected),
                "mika#2627 — le prédicat testimony n'est lu nulle part dans {expected} : \
                 ce scan vise un nom mort, il ne vérifie rien. Lecteurs trouvés : \
                 {readers:?}"
            );
        }

        let strangers: Vec<&String> = readers
            .iter()
            .filter(|r| !TESTIMONY_PREDICATE_READERS.contains(&r.as_str()))
            .collect();
        assert!(
            strangers.is_empty(),
            "mika#2627 R3 — un lecteur non recensé du prédicat testimony : {strangers:?}\n\n\
             RÉSOLUTION : router ce site vers `tools::check_testimony_access_proposal`, \
             qui porte la composition refus + télémétrie. Ne PAS l'ajouter à \
             TESTIMONY_PREDICATE_READERS_ALLOWED — une composition de plus, c'est une \
             formulation de refus de plus libre de diverger. Un lecteur dont la \
             DISPOSITION est nécessairement différente (mika#2633 : un livrable refusé \
             ne se répare ni par un renvoi ni par un découpage) se déclare dans \
             TESTIMONY_PREDICATE_READERS, avec sa raison, et doit porter un `channel` \
             distinct (mika2633_tout_emetteur_de_la_famille_porte_un_canal)."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist du scan A.
    #[test]
    fn mika2627_lallowlist_du_scan_a_est_vide() {
        assert!(
            TESTIMONY_PREDICATE_READERS_ALLOWED.is_empty(),
            "TESTIMONY_PREDICATE_READERS_ALLOWED est livrée vide et doit le rester : \
             quand le scan tire, on route le site vers le helper. Une allowlist née \
             vide est un emplacement où déposer la prochaine infraction (mika#2323)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2627 — tout émetteur de texte sous `tools/` est gardé, ou nommé
    // ─────────────────────────────────────────────────────────────────────

    /// **Un PÉRIMÈTRE, pas une allowlist d'exemption** (sens de mika#2536).
    ///
    /// Ces deux sites consomment `ctx.message_sender` sans appeler la garde, et
    /// pour deux raisons **différentes** :
    ///
    /// - `delegate_task` passe le sender au délégué et n'envoie rien lui-même —
    ///   le délégué appelle `send_message`, donc il est couvert
    ///   **transitivement**. C'est un motif **différent** d'une garde en amont
    ///   sur le même appel, d'où son maintien ici plutôt qu'un déplacement vers
    ///   `TESTIMONY_SENDER_COVERED_UPSTREAM` ;
    /// - `tools/mod.rs` **déclare** le champ, il ne le consomme pas. Et c'est
    ///   aussi le site du helper, donc l'y compter serait compter la garde comme
    ///   un trou.
    ///
    /// `run_team.rs` **est sorti de ce périmètre** par mika#2633 AC3 : son
    /// livrable est désormais gardé en amont, au point de pose, dans
    /// `teams/engine.rs`. Voir [`TESTIMONY_SENDER_COVERED_UPSTREAM`].
    ///
    /// Comparé **dans les deux sens** : une entrée dont le site a disparu fait
    /// rougir (assertion auto-nettoyante), sans quoi elle exempterait en silence
    /// un futur homonyme.
    const TESTIMONY_SENDER_PERIMETER: &[&str] = &[
        "crates/mika-agent/src/tools/delegate_task.rs",
        "crates/mika-agent/src/tools/mod.rs",
    ];

    /// Un site dont la garde vit en **AMONT**, avec le fichier qui la porte
    /// (mika#2633 AC3/U5).
    ///
    /// # Pourquoi un terme neuf, et pas un simple retrait du périmètre
    ///
    /// `run_team.rs` consomme `ctx.message_sender` et n'appellera jamais le
    /// helper d'outil : le texte qu'il envoie est `run.deliverable`, déjà posé —
    /// et déjà gardé — avant qu'il ne le lise. Le retirer du périmètre **sans
    /// plus** le ferait compter `unguarded` et rendrait le scan rouge ; le
    /// laisser au périmètre contredirait l'AC3, qui demande qu'il soit vu
    /// **gardé**. Donc le prédicat gagne un troisième terme : *gardé sur place,
    /// **ou** déclaré couvert en amont par un fichier qui lit réellement le
    /// prédicat, ou au périmètre*.
    ///
    /// # Ce que la déclaration coûte, et ce qu'elle ne peut pas être
    ///
    /// Elle est vérifiée **dans les deux sens** : le fichier aval doit exister
    /// et consommer le sender, et le fichier **amont** doit figurer parmi les
    /// lecteurs réels du prédicat ([`testimony_predicate_readers`]). Le jour où
    /// `teams/engine.rs` cesse de porter la garde, cette ligne rougit — c'est
    /// l'assertion auto-nettoyante que l'AC3 appelle « test vu rouge », et c'est
    /// ce qui empêche la déclaration de devenir une exemption de confort.
    const TESTIMONY_SENDER_COVERED_UPSTREAM: &[(&str, &str)] = &[(
        "crates/mika-agent/src/tools/run_team.rs",
        "crates/mika-agent/src/teams/engine.rs",
    )];

    /// Scan B — tout consommateur de `ctx.message_sender` sous `tools/` appelle
    /// la garde, ou figure au périmètre ci-dessus (mika#2627 RK4).
    ///
    /// Ce que ce scan **ne** couvre pas, nommé plutôt que découvert : un outil
    /// qui atteindrait l'utilisateur sans passer par `ctx.message_sender` ni par
    /// un `action_type` planifiable. Aucun n'existe aujourd'hui, et armer un
    /// détecteur sur une population vide est ce que mika#2520 refuse.
    #[test]
    fn mika2627_tout_emetteur_sous_tools_est_garde_ou_nomme() {
        let guard_call = format!("check_testimony_access{}(", "_proposal");
        let upstream_readers = testimony_predicate_readers();

        let mut unguarded = Vec::new();
        let mut consumers = Vec::new();

        for (rel, content) in production_sources() {
            if !rel.starts_with("crates/mika-agent/src/tools/") {
                continue;
            }
            let stripped = crate::source_scan::strip_comment_lines(&content);
            let lines: Vec<&str> = stripped.lines().collect();

            if !lines.iter().any(|l| l.contains("message_sender")) {
                continue;
            }
            consumers.push(rel.clone());

            let guarded = lines.iter().any(|l| l.contains(guard_call.as_str()));
            // mika#2633 — troisième terme : la garde peut vivre en amont, à
            // condition que le fichier amont déclaré lise réellement le prédicat.
            let covered_upstream =
                TESTIMONY_SENDER_COVERED_UPSTREAM
                    .iter()
                    .any(|(site, upstream)| {
                        *site == rel.as_str() && upstream_readers.iter().any(|r| r == upstream)
                    });
            if !guarded && !covered_upstream && !TESTIMONY_SENDER_PERIMETER.contains(&rel.as_str())
            {
                unguarded.push(rel);
            }
        }

        // Anti-vacuité : sans ça, un renommage de répertoire rendrait ce scan
        // muet et un arbre vide se lirait comme un arbre propre (mika#2205).
        assert!(
            consumers.len() >= 4,
            "mika#2627 — moins de quatre consommateurs de `message_sender` trouvés \
             sous `tools/` ({consumers:?}) : ce scan ne regarde plus la population \
             qu'il existe pour surveiller"
        );

        assert!(
            unguarded.is_empty(),
            "mika#2627 RK4 — un émetteur de texte sous `tools/` ne passe pas par la \
             garde testimony : {unguarded:?}\n\n\
             RÉSOLUTION : appeler `tools::check_testimony_access_proposal` sur le corps \
             sortant AVANT l'envoi, et ajouter le nom de l'outil à \
             `agent_loop::TESTIMONY_GATED_TOOLS`. Si le texte qu'il émet est déjà gardé \
             EN AMONT de lui, le déclarer dans TESTIMONY_SENDER_COVERED_UPSTREAM avec le \
             fichier qui porte la garde. S'il n'émet aucun texte DU MODÈLE (sender \
             relayé, texte composé par le moteur), le déclarer dans \
             TESTIMONY_SENDER_PERIMETER avec sa raison."
        );

        // L'autre sens : une entrée de périmètre dont le site a disparu, ou qui
        // ne consomme plus le sender, est un tiroir — pas un périmètre.
        let stale: Vec<&&str> = TESTIMONY_SENDER_PERIMETER
            .iter()
            .filter(|p| !consumers.iter().any(|c| c == *p))
            .collect();
        assert!(
            stale.is_empty(),
            "mika#2627 — une entrée de périmètre ne consomme plus `message_sender` : \
             {stale:?}\n\n\
             RÉSOLUTION : retirer la ligne. Une entrée qui survit à son site exempterait \
             en silence un futur homonyme (c'est la différence entre un périmètre et un \
             tiroir, mika#2536)."
        );

        // mika#2633 AC3 — l'assertion auto-nettoyante du terme neuf, dans les
        // deux sens : le site aval doit consommer le sender, et le fichier amont
        // doit réellement lire le prédicat. C'est ce qui fait rougir le jour où
        // `teams/engine.rs` cesse de porter la garde.
        for (site, upstream) in TESTIMONY_SENDER_COVERED_UPSTREAM {
            assert!(
                consumers.iter().any(|c| c == site),
                "mika#2633 — {site} est déclaré couvert en amont mais ne consomme plus \
                 `message_sender` : retirer la ligne (un périmètre, pas un tiroir)."
            );
            assert!(
                upstream_readers.iter().any(|r| r == upstream),
                "mika#2633 AC3 — {upstream} est déclaré comme portant la garde du \
                 livrable de {site}, et il ne lit PLUS le prédicat testimony.\n\n\
                 Lecteurs réels : {upstream_readers:?}\n\n\
                 RÉSOLUTION : rétablir la garde en amont, ou remettre {site} dans \
                 TESTIMONY_SENDER_PERIMETER en le nommant canal ouvert. Ne PAS retirer \
                 cette assertion : sans elle, la déclaration survivrait à la garde \
                 qu'elle atteste, c'est-à-dire exempterait en silence (mika#2536)."
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2633 — le format de fil de la télémétrie, et l'inertie du bras
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test frère l'assert.**
    const TESTIMONY_CHANNEL_FIELD_ALLOWED: &[&str] = &[];

    /// V7 — tout émetteur de la famille `guard.testimony_access_proposal` porte
    /// un champ `channel` (mika#2633).
    ///
    /// # Ce que ça remplace
    ///
    /// Le scan A bornait la population des lecteurs du prédicat à **deux**, ce
    /// qui garantissait *par accident* que le vocabulaire de canal ne pouvait
    /// pas diverger. mika#2633 porte cette population à trois ; cette garantie
    /// doit donc être posée explicitement, et c'est ce qui rend
    /// `jq 'select(.channel == "team_deliverable")'` exact plutôt qu'un filtre
    /// sur lequel trois sites peuvent diverger.
    ///
    /// # La famille, pas le seul nom nominal
    ///
    /// Le résidu `…_uncorrected` est dans la population, et c'est porteur :
    /// jusqu'à mika#2633 la ligne résidu de la garde 5h ne portait **aucun**
    /// `channel`, donc un opérateur lisant la famille n'avait aucun moyen de
    /// distinguer son résidu d'un futur. Ce scan est ce qui a mesuré ce trou, et
    /// le même commit l'a comblé.
    ///
    /// Aucun test comportemental ne voit cette classe : un émetteur sans
    /// `channel` ne rend **aucune** décision fausse — il rend une population
    /// incomptable, en silence.
    #[test]
    fn mika2633_tout_emetteur_de_la_famille_porte_un_canal() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let event_prefix = format!("event = \"guard.testimony_access{}", "_proposal");

        let mut emitters = Vec::new();
        let mut channelless = Vec::new();

        for (rel, content) in production_sources() {
            if TESTIMONY_CHANNEL_FIELD_ALLOWED.contains(&rel.as_str()) {
                continue;
            }
            let stripped = crate::source_scan::strip_comment_lines(&content);
            // Un émetteur est un `warn!`/`info!` portant la ligne `event = "…"` ;
            // le champ `channel` est cherché dans la même invocation de macro,
            // bornée par le `);` qui la ferme. Un scan à la ligne seule ne peut
            // pas répondre, les champs étant sur des lignes distinctes.
            let mut cursor = 0usize;
            while let Some(pos) = stripped[cursor..].find(event_prefix.as_str()) {
                let abs = cursor + pos;
                // Remonter au début de l'invocation : le dernier `!(` avant le
                // champ `event`.
                let start = stripped[..abs].rfind("!(").map_or(0, |i| i + 2);
                let end = stripped[abs..]
                    .find(");")
                    .map_or(stripped.len(), |i| abs + i);
                let invocation = &stripped[start..end];
                emitters.push(rel.clone());
                if !invocation.contains("channel = ") {
                    channelless.push(format!("{rel} (…{})", &stripped[abs..end].trim()));
                }
                cursor = abs + event_prefix.len();
            }
        }

        // Anti-vacuité par PRÉSENCE NOMMÉE, fichier par fichier (constat de
        // revue, adversarial P3). Un plancher global `>= 4` laissait disparaître
        // l'un des cinq émetteurs connus en silence — typiquement en hissant son
        // nom d'événement dans une constante, ce qui le sort de la population
        // du scan sans rien rougir. Chaque fichier connu doit garder au moins
        // ses émetteurs ; un émetteur neuf dans un fichier neuf reste permis.
        // Un plancher porte une présence nommée, jamais le compte du jour seul
        // (« un plancher d'anti-vacuité peut se rembourrer avec le trou qu'il
        // garde », 2026-10-01).
        for (file, expected, who) in [
            (
                "crates/mika-agent/src/agent_loop/mod.rs",
                2,
                "la garde 5h, nominale et résidu",
            ),
            ("crates/mika-agent/src/tools/mod.rs", 1, "le helper d'outil"),
            (
                "crates/mika-agent/src/teams/engine.rs",
                2,
                "le point de pose du livrable d'équipe, nominal et résidu",
            ),
        ] {
            let found = emitters.iter().filter(|rel| rel.as_str() == file).count();
            assert!(
                found >= expected,
                "mika#2633 V7 — {file} porte {found} émetteur(s) de la famille au \
                 lieu d'au moins {expected} ({who}) : un émetteur a quitté la \
                 population de ce scan, et sa ligne n'est plus vérifiée. Si son nom \
                 d'événement a été hissé dans une constante, c'est le scan qu'il \
                 faut faire suivre, pas ce plancher qu'il faut baisser. Émetteurs \
                 vus : {emitters:?}"
            );
        }

        assert!(
            channelless.is_empty(),
            "mika#2633 V7 — un émetteur de `guard.testimony_access_proposal*` ne \
             porte pas de champ `channel` : {channelless:?}\n\n\
             RÉSOLUTION : poser `channel = TestimonyProposalChannel::<variante>.as_wire()` \
             sur la ligne. Ne PAS l'ajouter à TESTIMONY_CHANNEL_FIELD_ALLOWED — un \
             émetteur sans canal rend la population incomptable, ce qui est exactement \
             ce que ce scan existe pour refuser."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist de V7.
    #[test]
    fn mika2633_lallowlist_du_scan_de_canal_est_vide() {
        assert!(
            TESTIMONY_CHANNEL_FIELD_ALLOWED.is_empty(),
            "TESTIMONY_CHANNEL_FIELD_ALLOWED est livrée vide et doit le rester : \
             quand le scan tire, on pose le champ. Une allowlist née vide est un \
             emplacement où déposer la prochaine infraction (mika#2323)."
        );
    }

    /// V6 — l'inertie du bras `TeamDeliverable` de
    /// `check_testimony_access_proposal` est épinglée (mika#2633 U4).
    ///
    /// Le bras existe pour que l'`enum` reste exhaustif et pour que sa
    /// réapparition ne soit pas une réouverture silencieuse, exactement comme
    /// `C::EndTurn` — que la garde 5h n'atteint pas non plus. Sans cette
    /// assertion, l'inertie nommée sur la variante serait à re-vérifier à la
    /// main à chaque relecture, et une couverture inerte qui se lit comme une
    /// couverture est la classe mika#2205.
    #[test]
    fn mika2633_le_bras_team_deliverable_est_inerte() {
        let guard_call = format!("check_testimony_access{}(", "_proposal");

        let scanned: Vec<(String, String)> = production_sources()
            .into_iter()
            .filter(|(rel, _)| rel.starts_with("crates/mika-agent/src/teams/"))
            .collect();

        // Anti-vacuité (constat de revue, testing P3) : un scan « doit rester
        // vide » qui ne regarde aucun fichier est vert pour la mauvaise raison —
        // un renommage de `teams/` ou une panne de l'énumérateur le rendrait
        // muet, la classe mika#2205 que ce test invoque lui-même. Le fichier qui
        // porte le point de pose est exigé nommément.
        assert!(
            scanned
                .iter()
                .any(|(rel, _)| rel == "crates/mika-agent/src/teams/engine.rs"),
            "mika#2633 V6 — `teams/engine.rs` n'est pas dans la population de ce \
             scan ({} fichier(s) vus sous `teams/`) : il ne regarde plus ce qu'il \
             surveille (mika#2205).",
            scanned.len()
        );

        let callers: Vec<String> = scanned
            .into_iter()
            .filter(|(_, content)| {
                crate::source_scan::strip_comment_lines(content)
                    .lines()
                    .any(|l| l.contains(guard_call.as_str()))
            })
            .map(|(rel, _)| rel)
            .collect();

        assert!(
            callers.is_empty(),
            "mika#2633 V6 — un site de `teams/` appelle le helper d'outil : \
             {callers:?}\n\n\
             Le bras `TeamDeliverable` est documenté INERTE sur sa variante. S'il \
             devient atteignable, c'est cette documentation qu'il faut corriger — et \
             alors le refus composé par le helper (`ToolOutput::error`) doit être \
             confronté à la disposition du livrable, qui est une re-rédaction puis une \
             ligne neutre, pas un renvoi."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2641 — le motif d'un arrêt de groom illisible : un écrivain, et
    // deux moitiés qui ne divergent pas.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Rien à excepter à la livraison, et c'est vérifiable : le nom
    /// `groom_architect_unreadable` est **neuf**, donc aucune violation
    /// préexistante ne peut exister. Quand ce scan tire, **on retire le second
    /// écrivain**, on ne l'excepte pas (doctrine mika#2201) — une allowlist née
    /// vide est un emplacement où déposer la prochaine infraction (mika#2323).
    const ARCHITECT_UNREADABLE_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// **S-a — un seul écrivain du nom d'audit (mika#2641).**
    ///
    /// Le nom sert de nom d'événement de journal **et** de `tool_name` d'audit.
    /// La propriété est porteuse pour une raison précise : le compte
    /// `SELECT target_key, count(*) … WHERE tool_name = 'groom_architect_unreadable'`
    /// **est** la précondition explicite du ticket de suivi sur le taux de
    /// coupure architecte (R6 du plan — la mesure « aboutis / coupés / tronqués »
    /// que le commentaire opérateur demande, dont ce ticket ne livre que le
    /// troisième tiers). Un second écrivain ne rendrait **aucune** décision
    /// fausse ; il rendrait ce compte inexact, en silence, et le suivi
    /// s'ouvrirait sur un nombre que personne ne pourrait départager.
    ///
    /// Aucun test comportemental ne voit cette classe — d'où un scan de source.
    #[test]
    fn mika2641_the_architect_unreadable_name_has_a_single_writer() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("groom_architect{}", "_unreadable");
        let owner = "crates/mika-agent/src/task_engine/dispatcher.rs";

        // `production_sources_to_test_module` et non `production_sources` : le
        // second coupe au PREMIER `#[cfg(test)]` **où qu'il soit**, et
        // `builtin_handlers.rs` en porte un sur une paire de constantes bien
        // avant son module de test — ce qui cache ~5 500 lignes de production à
        // la détection d'un second écrivain. Un scan aveugle sur une partie de
        // l'arbre se lit exactement comme un scan propre (classe mika#2205).
        //
        // `strip_comment_lines` et non un filtre `starts_with('*')` écrit à la
        // main : ce filtre-là compte `*guard = x;` pour un commentaire, donc une
        // écriture sur une telle ligne lui est invisible. Le primitif partagé
        // documente ce défaut exact.
        let mut writers = Vec::new();
        for (rel, content) in production_sources_to_test_module() {
            if ARCHITECT_UNREADABLE_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
                continue;
            }
            let carries = crate::source_scan::strip_comment_lines(&content)
                .lines()
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
            "mika#2641 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2641 — le nom d'audit de la seconde passe illisible a un second \
             écrivain : {strangers:?}\n\n\
             RÉSOLUTION : retirer le second site. Ne PAS l'ajouter à \
             ARCHITECT_UNREADABLE_SOLE_WRITER_EXCEPTIONS — le compte qui dimensionne \
             le ticket de suivi n'est exact que tant qu'un seul site l'écrit."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2641_the_sole_writer_allowlist_is_empty() {
        assert!(
            ARCHITECT_UNREADABLE_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "ARCHITECT_UNREADABLE_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain."
        );
    }

    /// **Livrée vide, et le test plus bas l'assert.** Même contrat que ci-dessus.
    const ARCHITECT_UNREADABLE_WIRE_EXCEPTIONS: &[&str] = &[];

    /// **S-b — le format de fil ne diverge pas entre le shell et le Rust
    /// (mika#2641 D4).**
    ///
    /// Le motif a DEUX moitiés : `dispatch-lib.sh` l'écrit dans `tasks.result`,
    /// `dispatcher.rs` l'y lit et en fait une ligne `audit_events`. Les deux
    /// littéraux doivent être le même octet. Une divergence ne casse **rien** le
    /// jour où elle est écrite : le shell continue d'écrire, le lecteur continue
    /// de ne rien trouver, et la population devient **vide en silence** — c'est
    /// à dire indistinguable d'un champ sain (classe mika#2205). Aucun test
    /// comportemental de l'un ou l'autre côté ne peut voir ça, puisque chacun
    /// reste correct dans son propre monde.
    #[test]
    fn mika2641_the_halt_cause_wire_format_is_synchronised_shell_to_rust() {
        use crate::task_engine::dispatcher::{
            GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE, GROOM_HALT_CAUSE_LINE_PREFIX,
        };

        let shell = repo_root()
            .join("skills")
            .join("bundled")
            .join("_shared")
            .join("dispatch-lib.sh");
        let rel = "skills/bundled/_shared/dispatch-lib.sh";
        if ARCHITECT_UNREADABLE_WIRE_EXCEPTIONS.contains(&rel) {
            return;
        }
        // Un shell illisible ÉCHOUE plutôt que de rendre une comparaison vide :
        // un scan qui ne lit rien se lit comme un scan d'accord.
        let src = std::fs::read_to_string(&shell).unwrap_or_else(|e| {
            panic!(
                "mika#2641 — {rel} doit être lisible pour que ce scan vérifie quelque chose : {e}"
            )
        });

        // L'écrivain shell déclare les deux valeurs par une affectation en
        // colonne 0. C'est la forme que `test-dispatch-lib.sh` épingle aussi
        // comme site de définition unique.
        let expect_prefix =
            format!("GROOM_HALT_CAUSE_LINE_PREFIX=\"{GROOM_HALT_CAUSE_LINE_PREFIX}\"");
        let expect_cause = format!(
            "GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE=\"{GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE}\""
        );

        assert!(
            src.lines().any(|l| l == expect_prefix),
            "mika#2641 — le préfixe de ligne a divergé : le Rust lit \
             `{GROOM_HALT_CAUSE_LINE_PREFIX}` et {rel} ne porte pas la ligne \
             `{expect_prefix}`.\n\n\
             RÉSOLUTION : réaligner les deux littéraux. Ne PAS élargir le lecteur \
             Rust à une seconde orthographe — la population n'est exacte que tant \
             que les deux moitiés sont le même octet."
        );
        assert!(
            src.lines().any(|l| l == expect_cause),
            "mika#2641 — la valeur de cause a divergé : le Rust lit \
             `{GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE}` et {rel} ne porte pas la ligne \
             `{expect_cause}`.\n\n\
             RÉSOLUTION : réaligner les deux littéraux."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2641_the_wire_allowlist_is_empty() {
        assert!(
            ARCHITECT_UNREADABLE_WIRE_EXCEPTIONS.is_empty(),
            "ARCHITECT_UNREADABLE_WIRE_EXCEPTIONS est livrée vide et doit le rester : \
             excepter le fichier shell rendrait le scan vert en cessant de regarder \
             la moitié qu'il existe pour confronter (mika#2201)."
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // mika#2649 — le nom d'audit de la lignée a un écrivain ; la grammaire
    // d'événement a un recensement de lecteurs.
    // ─────────────────────────────────────────────────────────────────────

    /// **Livrée vide, et le test plus bas l'assert.**
    ///
    /// Rien à excepter à la livraison, et c'est vérifiable : le nom
    /// `webhook_dispatch_target_binding` est **neuf**, donc son unique écrivain
    /// est le site créé par cette PR. Quand ce scan tire, **on retire le second
    /// écrivain**, on ne l'excepte pas (doctrine mika#2201 — une allowlist née
    /// vide est un emplacement où déposer la prochaine infraction, mika#2323).
    const TARGET_BINDING_SOLE_WRITER_EXCEPTIONS: &[&str] = &[];

    /// **Le nom d'audit de la lignée a un seul écrivain (mika#2649 V12).**
    ///
    /// C'est cette propriété qui rend exact le `GROUP BY after_value` que
    /// l'opérateur exécute sur `tool_name = 'webhook_dispatch_target_binding'` —
    /// un nombre sur lequel deux sites peuvent diverger n'est pas une mesure.
    ///
    /// Aucun test comportemental ne peut voir cette classe : un second écrivain
    /// ne rend **aucune** décision fausse le jour où il est écrit — la garde
    /// continue de refuser, les tests restent verts — il rend le compte inexact,
    /// en silence.
    ///
    /// **L'énumérateur est `production_sources_to_test_module`, et c'est une
    /// mesure.** `production_sources` coupe au premier littéral `#[cfg(test)]`
    /// **où qu'il soit** ; `webhook_dispatch.rs` en porte un dans un
    /// doc-comment à ~600 lignes **au-dessus** de la constante, donc réutiliser
    /// cet énumérateur rendait ce scan vert par **population vide** — et c'est
    /// son anti-vacuité qui l'a dit, exactement comme pour les deux scans de
    /// mika#2624.
    ///
    /// # La comparaison est EXACTE, et pas en sous-chaîne comme chez ses voisins
    ///
    /// Le nom de l'événement de résidu de cette famille est, par construction,
    /// `<nom d'audit>_audit_failed` — donc le nom d'audit est son **préfixe**.
    /// Une comparaison par sous-chaîne compterait ce résidu comme un second
    /// écrivain, alors que c'est un **autre jeton** : le scan vise *qui écrit ce
    /// `tool_name`*, et un nom de journal qui le préfixe n'en est pas un.
    ///
    /// **Fait mesuré en écrivant ce scan, et nommé plutôt que laissé à
    /// redécouvrir :** les scans voisins de la même famille (mika#2573 et ses
    /// semblables) emploient la sous-chaîne et sont verts **par troncature
    /// accidentelle**, pas par prédicat —
    /// `builtin_handlers.rs:3105` porte bien `fallthrough_work_creation_audit_failed`,
    /// mais `production_sources` tronque ce fichier à son `#[cfg(test)]` de la
    /// ligne ~674 et ne voit jamais la ligne 3105. Leur réparation est un
    /// changement de **leur** périmètre, donc hors de ce ticket ; ce qui est à
    /// retenir est que la sous-chaîne n'est pas le prédicat de référence de cette
    /// famille, c'est son accident.
    #[test]
    fn mika2649_le_nom_daudit_a_un_seul_ecrivain() {
        // Composé à l'exécution pour que CE fichier ne se dénonce pas lui-même.
        let needle = format!("webhook_dispatch{}", "_target_binding");
        let owner = "crates/mika-agent/src/webhook_dispatch.rs";

        let mut writers = Vec::new();
        for (rel, content) in production_sources_to_test_module() {
            if TARGET_BINDING_SOLE_WRITER_EXCEPTIONS.contains(&rel.as_str()) {
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
                        .any(|lit| lit == needle.as_str())
                });
            if carries {
                writers.push(rel);
            }
        }

        // Anti-vacuité : un scan qui ne trouve PERSONNE se lit exactement comme
        // un scan propre (mika#2103 / mika#2205).
        assert!(
            writers.iter().any(|w| w == owner),
            "mika#2649 — `{needle}` n'est écrit nulle part dans {owner} : ce scan \
             vise un nom mort, il ne vérifie rien"
        );

        let strangers: Vec<&String> = writers.iter().filter(|w| *w != owner).collect();
        assert!(
            strangers.is_empty(),
            "mika#2649 — le nom d'audit de la lignée a un second écrivain : \
             {strangers:?}\n\n\
             RÉSOLUTION : faire passer ce site par \
             `webhook_dispatch::TARGET_BINDING_AUDIT_TOOL`. Ne PAS l'ajouter à \
             TARGET_BINDING_SOLE_WRITER_EXCEPTIONS — le `GROUP BY` de la sonde \
             n'est exact que tant qu'un seul site écrit ce nom."
        );
    }

    /// Le pendant auto-nettoyant de l'allowlist ci-dessus.
    #[test]
    fn mika2649_lallowlist_du_nom_daudit_est_vide() {
        assert!(
            TARGET_BINDING_SOLE_WRITER_EXCEPTIONS.is_empty(),
            "TARGET_BINDING_SOLE_WRITER_EXCEPTIONS est livrée vide et doit le \
             rester : quand le scan tire, on retire le second écrivain \
             (doctrine mika#2201)."
        );
    }

    /// Les fichiers qui portent une **regex de grammaire d'événement**
    /// `[GitHub] …`, et ce que chacun lit (mika#2649 V13).
    ///
    /// # Un RECENSEMENT, jamais une allowlist d'exemptions
    ///
    /// La distinction est celle que mika#2633 a déjà dû écrire : *on y ajoute
    /// quand un nouveau lecteur est justifié, on n'y exempte pas un doublon.* Et
    /// c'est une **rectification mesurée au plan de mika#2649**, dont la
    /// Fire-Disposition annonçait un `EVENT_GRAMMAR_PARSER_SITES_ALLOWED` livré
    /// **vide** en supposant que les sites existants qui citent ces préfixes
    /// soient des prédicats de préfixe (`starts_with`) et non des parseurs. La
    /// mesure réfute la supposition : **cinq** regex sur **trois** fichiers, dont
    /// `CHECK_SUITE_RE` en **double**. Une allowlist vide aurait rendu ce scan
    /// rouge à la naissance — et un lint rouge à la naissance se fait désarmer.
    ///
    /// # Le doublon est NOMMÉ plutôt que caché
    ///
    /// `webhook_queue.rs` et `webhook_queue_v2.rs` portent la même regex
    /// check-suite, la seconde se déclarant doublon assumé dans son propre
    /// commentaire (*« duplicated here to keep the v2 module self-contained »*).
    /// Ce recensement est précisément l'endroit où un futur éditeur apprend que
    /// le doublon existe **avant** d'en écrire un troisième — c'est la classe
    /// mika#2158, et ce ticket l'a évitée en appelant `classify_event` plutôt
    /// qu'en recopiant l'extraction de `(branch: …)`.
    const EVENT_GRAMMAR_PARSER_SITES: &[(&str, &str)] = &[
        (
            "crates/mika-agent/src/server/verdict.rs",
            "HEADER_RE — `[GitHub] PR review (…) on <repo>#<n> …`, lecteur unique \
             de la forme revue, consommé par `deadline_verdict::parse_pr_target`",
        ),
        (
            "crates/mika-agent/src/server/webhook_queue_v2.rs",
            "PR_ACTION_RE (`[GitHub] PR <action>: …`), CHECK_SUITE_RE \
             (`[GitHub] Check suite … (branch: …)`) et ISSUE_LABELED_RE — les \
             lecteurs que `classify_event` consomme",
        ),
        (
            "crates/mika-agent/src/server/webhook_queue.rs",
            "CHECK_SUITE_RE — DOUBLON assumé de celui de webhook_queue_v2 \
             (mécanisme mika#528, distinct). Nommé ici pour qu'un troisième ne \
             soit pas écrit par ignorance du second",
        ),
    ];

    /// **Aucun second parseur de grammaire d'événement (mika#2649 V13).**
    ///
    /// Le scan part de la **forme du parseur** — un `Regex::new(` dont un
    /// littéral porte `[GitHub]` — et confronte sa population au recensement
    /// ci-dessus, **dans les deux sens** : un fichier porteur hors recensement
    /// rougit, et une entrée du recensement dont le fichier ne porte plus de
    /// regex rougit aussi. La seconde direction est celle qui compte le plus :
    /// un recensement qui survit à son site est un tiroir, et il se lit comme
    /// une couverture.
    ///
    /// Le prédicat porte sur `Regex::new(`, donc un `starts_with` ou un
    /// `contains` sur le même préfixe est **hors population par sa forme** — ce
    /// qui est exact : `is_webhook_fallthrough_domain` teste un préfixe et
    /// n'extrait rien. C'est le même terme positionnel que mika#2496.
    ///
    /// Aucun test comportemental ne peut voir cette classe : une sixième regex
    /// recopiée ne rend aucune décision fausse le jour où elle est écrite, elle
    /// divergera plus tard, en silence, avec toutes les assertions vertes.
    #[test]
    fn mika2649_aucun_second_parseur_de_grammaire_devenement() {
        let needle = format!("[Git{}]", "Hub");
        let declared: Vec<&str> = EVENT_GRAMMAR_PARSER_SITES
            .iter()
            .map(|(path, _)| *path)
            .collect();

        let mut found: Vec<String> = Vec::new();
        for (rel, content) in production_sources_to_test_module() {
            let carries = content
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
                })
                .any(|line| {
                    line.contains("Regex::new(")
                        && string_literals(line)
                            .iter()
                            .any(|lit| lit.contains(needle.as_str()))
                });
            if carries {
                found.push(rel);
            }
        }

        // Anti-vacuité : le recensement porte trois fichiers, et le scan doit
        // les voir. Zéro trouvaille se lirait comme un arbre propre.
        assert!(
            found.len() >= EVENT_GRAMMAR_PARSER_SITES.len(),
            "mika#2649 — le scan n'a trouvé que {} site(s) porteur(s) de regex de \
             grammaire d'événement alors que le recensement en déclare {} : le \
             prédicat ne regarde plus ce qu'il existe pour surveiller.\n\
             trouvés : {found:?}",
            found.len(),
            EVENT_GRAMMAR_PARSER_SITES.len()
        );

        // Sens 1 — un porteur hors recensement.
        let undeclared: Vec<&String> = found
            .iter()
            .filter(|f| !declared.contains(&f.as_str()))
            .collect();
        assert!(
            undeclared.is_empty(),
            "mika#2649 — une regex de grammaire d'événement `[GitHub] …` vit hors \
             du recensement : {undeclared:?}\n\n\
             RÉSOLUTION : **appeler** le lecteur unique existant plutôt que de \
             recopier la grammaire — `deadline_verdict::parse_pr_target` pour la \
             forme PR, `webhook_queue_v2::classify_event` pour la forme \
             check-suite, `worktree_reaper::issue_number_from_branch` pour le \
             numéro porté par une branche. Si un nouveau lecteur est réellement \
             justifié, l'AJOUTER à EVENT_GRAMMAR_PARSER_SITES avec sa raison — \
             c'est un recensement, pas une allowlist d'exemptions (mika#2633)."
        );

        // Sens 2 — une entrée périmée, qui se lit comme une couverture.
        let stale: Vec<&&str> = declared
            .iter()
            .filter(|d| !found.iter().any(|f| f == *d))
            .collect();
        assert!(
            stale.is_empty(),
            "mika#2649 — le recensement nomme un fichier qui ne porte plus de \
             regex de grammaire d'événement : {stale:?}\n\n\
             RÉSOLUTION : retirer l'entrée. Une entrée qui survit à son site est \
             un tiroir, et elle se lit comme une couverture (mika#2205)."
        );

        // Les deux lecteurs uniques que ce ticket APPELLE doivent encore exister
        // sous ces noms : assertion auto-nettoyante. S'ils sont renommés ou
        // supprimés, ce scan rougit au lieu de cesser de regarder.
        let src = |rel: &str| {
            std::fs::read_to_string(repo_root().join(rel))
                .unwrap_or_else(|e| panic!("{rel} lisible : {e}"))
        };
        assert!(
            src("crates/mika-agent/src/server/deadline_verdict.rs")
                .contains("pub fn parse_pr_target("),
            "mika#2649 — `parse_pr_target` a disparu ou changé de nom : \
             `webhook_event_target` l'appelle, et sans lui ce ticket aurait écrit \
             un second parseur de la grammaire PR."
        );
        assert!(
            src("crates/mika-agent/src/worktree_reaper.rs")
                .contains("pub fn issue_number_from_branch("),
            "mika#2649 — `issue_number_from_branch` a disparu ou changé de nom : \
             `webhook_event_target` l'appelle pour lire le numéro porté par une \
             branche de check-suite."
        );
        assert!(
            src("crates/mika-agent/src/server/webhook_queue_v2.rs")
                .contains("pub fn classify_event("),
            "mika#2649 — `classify_event` a disparu ou changé de nom : \
             `webhook_event_target` l'appelle pour lire la grammaire check-suite \
             plutôt que d'en écrire une TROISIÈME copie."
        );
    }
}
