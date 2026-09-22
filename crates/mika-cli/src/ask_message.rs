//! Résolution du message d'un `mika ask` — le lecteur unique (mika#1982).
//!
//! # Pourquoi un module et pas deux lignes au site d'appel
//!
//! `mika ask` a **trois** chemins d'exécution — l'agent local, `--team`, et
//! `--remote` — et le message y était résolu en trois endroits différents :
//! deux copies identiques de la lecture de la sentinelle `-`
//! (`commands/ask.rs`, chemin local et chemin team) et **rien du tout** sur
//! `--remote`, qui envoyait au serveur distant la chaîne littérale `"-"`, soit
//! un octet, et rendait une réponse plausible à une question jamais posée.
//!
//! Ajouter l'inférence TTY au geste évident — éditer les deux sites existants —
//! l'aurait posée en deux exemplaires et aurait laissé le troisième chemin muet.
//! C'est la classe « lecteur unique » que la maison a déjà dû engraver trois
//! fois : `grooming_marker` (mika#2158, deux regex divergentes pendant des mois
//! sans que rien ne casse), `parse_log_llm_bodies` (mika#2220, deux tables de
//! vérité dont l'une acceptait `True` et l'autre non), `LlmUsage::accumulate`
//! (mika#1883, un second site d'addition rendant un total faux avec tous les
//! tests au vert). Les trois partagent une propriété : la divergence ne rend
//! aucune décision fausse, elle rend **deux décisions différentes**, et rien ne
//! rougit.
//!
//! # La table de décision
//!
//! | positionnel | stdin TTY | comportement |
//! |---|---|---|
//! | `"hello"` | indifférent | `"hello"`, l'entrée standard n'est **jamais** lue |
//! | `"-"` | indifférent | lire l'entrée standard |
//! | absent | non | lire l'entrée standard |
//! | absent | oui | erreur d'usage |
//!
//! Deux choix y sont posés plutôt que subis. **`-` est lu même sur un TTY** :
//! c'est la sémantique Unix de la sentinelle (`cat -` attend la saisie), c'est
//! le comportement actuel, et le conditionner au non-TTY serait une régression
//! silencieuse pour l'opérateur qui tape `mika ask -` puis son message.
//! **Un positionnel présent n'entraîne aucune lecture** — pas même une
//! tentative non bloquante : c'est ce qui rend le cas nominal byte-identique.
//!
//! # Contrainte dure : `-` ne peut pas régresser
//!
//! La sentinelle porte un consommateur de production critique — `_arch_ask`
//! dans `skills/bundled/_shared/dispatch-lib.sh`, c'est-à-dire le chemin de
//! **tout le grooming architecte** — et elle est documentée publiquement
//! (`docs/getting-started.md`). Toute conception qui la dégraderait casserait
//! le grooming sur tous les tickets.

use std::fmt;
use std::io::{IsTerminal, Read};

/// La sentinelle Unix qui demande explicitement la lecture de l'entrée standard.
///
/// Écrite une fois : deux orthographes de la même sentinelle seraient la
/// divergence que ce module existe pour fermer.
pub const STDIN_SENTINEL: &str = "-";

/// Pourquoi la résolution du message a échoué.
///
/// Les deux variantes disent des choses différentes à l'opérateur et sortent
/// par le même chemin d'erreur ordinaire du binaire — la résolution échoue
/// **avant** tout appel réseau, donc son code de sortie ne peut pas se
/// confondre avec le code transport de mika#2278.
#[derive(Debug)]
pub enum AskMessageError {
    /// Aucun positionnel et l'entrée standard est un terminal : lire bloquerait
    /// sans que rien ne le dise. C'est l'erreur d'usage que clap émettait quand
    /// le positionnel était obligatoire, ré-émise à la main et nommant les
    /// trois gestes qui marchent.
    MissingOnTty,
    /// Une entrée a bien été consultée, et elle ne porte aucun message.
    Empty,
    /// L'entrée standard n'a pas pu être lue.
    Read(std::io::Error),
}

impl fmt::Display for AskMessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingOnTty => f.write_str(
                "No message provided. Pass it as an argument — mika ask \"your message\" — \
                 or pipe it in — echo \"your message\" | mika ask — \
                 or read the standard input explicitly with the \"-\" sentinel.",
            ),
            Self::Empty => f.write_str(
                "Empty message. Provide a message argument, or pipe one in \
                 (with or without the \"-\" sentinel).",
            ),
            Self::Read(e) => write!(f, "Failed to read the message from standard input: {e}"),
        }
    }
}

impl std::error::Error for AskMessageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(e) => Some(e),
            _ => None,
        }
    }
}

/// Résout le message d'un `mika ask` selon la table de décision de mika#1982.
///
/// `positional` est `None` quand clap n'a pas reçu l'argument. `stdin_is_tty`
/// et `reader` sont des **paramètres** — et non des appels à
/// `std::io::stdin()` enfouis — parce qu'aucun test unitaire ne peut fabriquer
/// un terminal : c'est ce qui rend l'AC3 vérifiable autrement qu'à la main.
///
/// `reader` n'est pas touché quand le positionnel est présent et n'est pas la
/// sentinelle. C'est une garantie, pas une optimisation : elle est épinglée par
/// un lecteur de test qui panique si on le lit.
pub fn resolve_ask_message<R: Read>(
    positional: Option<&str>,
    stdin_is_tty: bool,
    reader: R,
) -> Result<String, AskMessageError> {
    let read_stdin = match positional {
        // La sentinelle : lue inconditionnellement, TTY ou non.
        Some(STDIN_SENTINEL) => true,
        // Un positionnel ordinaire est le message, et rien d'autre n'est
        // consulté. Pas de `trim` : c'est le comportement historique et le
        // changer serait étranger au ticket.
        Some(message) => return non_empty(message.to_string()),
        // Rien en argument : convention Unix — l'absence d'argument déduit
        // l'entrée standard, sauf sur un terminal où elle bloquerait.
        None if stdin_is_tty => return Err(AskMessageError::MissingOnTty),
        None => true,
    };
    debug_assert!(read_stdin);

    let mut buf = String::new();
    let mut reader = reader;
    reader
        .read_to_string(&mut buf)
        .map_err(AskMessageError::Read)?;
    // Le `trim` est conservé tel quel : les deux sites d'origine trimmaient, et
    // `_arch_ask` pipe un markdown dont les bords blancs n'ont pas de sens.
    non_empty(buf.trim().to_string())
}

fn non_empty(message: String) -> Result<String, AskMessageError> {
    if message.is_empty() {
        return Err(AskMessageError::Empty);
    }
    Ok(message)
}

/// L'enrobage impur : les deux seuls appels au vrai descripteur du processus.
///
/// C'est lui que les trois chemins de `mika ask` appellent. Sa raison d'être
/// est de contenir ces deux appels, ce qui est aussi pourquoi ce fichier est
/// exclu par construction du périmètre du garde structurel ci-dessous.
pub fn resolve_from_process_stdin(positional: Option<&str>) -> Result<String, AskMessageError> {
    let handle = std::io::stdin();
    let is_tty = handle.is_terminal();
    resolve_ask_message(positional, is_tty, handle.lock())
}

/// Le prédicat du garde R6 : ce texte contient-il un **site d'appel** à
/// l'entrée standard ?
///
/// La règle porte sur le site d'appel (`\bstdin\s*\(`), pas sur un chemin
/// qualifié, parce que les trois écritures atteignables sont
/// `std::io::stdin()`, `io::stdin()` (après `use std::io;`) et `stdin()`
/// (après `use std::io::stdin;`), et que **deux d'entre elles sont déjà en
/// usage dans ce crate**. Un garde ancré sur `std::io::stdin` seul serait donc
/// évadé par un `use` que le code pratique déjà.
///
/// Elle couvre `is_terminal` sans second prédicat : tester la tty-ness exige de
/// tenir le handle, et les deux formes en usage
/// (`std::io::stdin().is_terminal()` et
/// `std::io::IsTerminal::is_terminal(&std::io::stdin())`) contiennent l'une et
/// l'autre le site d'appel.
///
/// **Limite assumée, écrite plutôt que découverte :** `use std::io::stdin as
/// lire;` y échappe. Ce garde borne la divergence de bonne foi — un second
/// lecteur écrit sans malice, qui est la forme qu'ont prise les trois
/// divergences citées en tête de module — et non l'évasion délibérée, qu'aucun
/// scan de source n'atteint.
#[cfg(test)]
fn has_stdin_call_site(source: &str) -> bool {
    const NEEDLE: &str = "stdin";
    source.match_indices(NEEDLE).any(|(at, _)| {
        // `\b` à gauche : `child.stdin` et `std::io::stdin` qualifient, un
        // hypothétique `my_stdin` non.
        let left_is_boundary = source[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        // `\s*\(` à droite : c'est ce qui sépare un appel d'une mention en
        // prose ou d'un champ (`child.stdin.take()`).
        left_is_boundary && source[at + NEEDLE.len()..].trim_start().starts_with('(')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un lecteur qui panique dès qu'on le touche.
    ///
    /// C'est la seule façon d'asserter « l'entrée standard n'est **jamais**
    /// lue » plutôt que « le message rendu est le bon » — une implémentation
    /// qui lirait puis jetterait passerait la seconde assertion.
    struct PanickingReader;

    impl Read for PanickingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            panic!("the standard input was read while a positional message was present");
        }
    }

    // ---- La table de décision, ligne par ligne ----

    #[test]
    fn mika1982_a_positional_message_wins_and_never_touches_the_input() {
        // AC2, dans sa forme forte : stdin porte des données *et* un lecteur
        // qui panique — le positionnel doit court-circuiter les deux.
        let resolved = resolve_ask_message(Some("hello"), false, PanickingReader).unwrap();
        assert_eq!(resolved, "hello");
    }

    #[test]
    fn mika1982_a_positional_message_wins_on_a_terminal_too() {
        let resolved = resolve_ask_message(Some("hello"), true, PanickingReader).unwrap();
        assert_eq!(resolved, "hello");
    }

    #[test]
    fn mika1982_the_sentinel_is_read_when_the_input_is_piped() {
        // AC5 / R4 — la non-régression dont `_arch_ask` dépend.
        let resolved = resolve_ask_message(Some("-"), false, &b"piped body\n"[..]).unwrap();
        assert_eq!(resolved, "piped body");
    }

    #[test]
    fn mika1982_the_sentinel_is_read_on_a_terminal_too() {
        // Sémantique Unix de la sentinelle : `cat -` attend la saisie. La
        // conditionner au non-TTY serait une régression silencieuse pour
        // l'opérateur qui tape `mika ask -` puis son message.
        let resolved = resolve_ask_message(Some("-"), true, &b"typed at the prompt"[..]).unwrap();
        assert_eq!(resolved, "typed at the prompt");
    }

    #[test]
    fn mika1982_an_absent_positional_infers_the_piped_input() {
        // AC1 — le défaut que le ticket nomme.
        let resolved = resolve_ask_message(None, false, &b"hello\n"[..]).unwrap();
        assert_eq!(resolved, "hello");
    }

    #[test]
    fn mika1982_an_absent_positional_on_a_terminal_is_a_usage_error() {
        // AC3 — immédiat, jamais un blocage silencieux en lecture. Le lecteur
        // qui panique est ce qui prouve la seconde moitié de la phrase.
        let err = resolve_ask_message(None, true, PanickingReader).unwrap_err();
        assert!(matches!(err, AskMessageError::MissingOnTty));
    }

    #[test]
    fn mika1982_the_usage_error_names_the_three_doors() {
        // Le positionnel n'étant plus obligatoire, clap n'émet plus son
        // « required arguments were not provided ». Le texte qui le remplace
        // doit nommer les trois gestes qui marchent, sans quoi l'opérateur
        // perd l'information que l'ancienne erreur portait.
        let text = AskMessageError::MissingOnTty.to_string();
        assert!(text.contains("mika ask \"your message\""), "{text}");
        assert!(text.contains("| mika ask"), "{text}");
        assert!(text.contains("\"-\""), "{text}");
    }

    // ---- Les bords ----

    #[test]
    fn mika1982_multiline_input_survives_intact() {
        // Le cas d'usage du ticket : un prompt long et multiligne. Seuls les
        // bords sont rognés, jamais l'intérieur.
        let body = "# Plan\n\nligne une\n\nligne deux\n";
        let resolved = resolve_ask_message(None, false, body.as_bytes()).unwrap();
        assert_eq!(resolved, "# Plan\n\nligne une\n\nligne deux");
    }

    #[test]
    fn mika1982_an_empty_piped_input_is_an_empty_message_not_a_usage_error() {
        // Population nommée et délibérément non couverte : `mika ask </dev/null`,
        // et le cron ou l'unité systemd dont l'entrée est fermée. La lecture
        // rend immédiatement une chaîne vide — pas de blocage — et le binaire
        // échoue sur « Empty message ». Le texte d'erreur **change** pour cette
        // population : elle voyait l'erreur d'usage de clap.
        //
        // C'est accepté et non corrigé : les deux textes disent la même chose à
        // l'opérateur, et les distinguer demanderait de deviner l'intention
        // derrière une entrée vide, ce qu'aucun signal ne permet. Le cas est
        // testé pour que le changement soit **constaté** plutôt que découvert.
        let err = resolve_ask_message(None, false, &b""[..]).unwrap_err();
        assert!(matches!(err, AskMessageError::Empty));
    }

    #[test]
    fn mika1982_a_whitespace_only_input_is_empty() {
        let err = resolve_ask_message(None, false, &b"   \n\t\n"[..]).unwrap_err();
        assert!(matches!(err, AskMessageError::Empty));
    }

    #[test]
    fn mika1982_the_sentinel_over_an_empty_input_is_empty_not_the_sentinel() {
        // Le pire résultat possible serait d'envoyer `"-"` comme message —
        // c'est très exactement le faux vert que `--remote` produisait.
        let err = resolve_ask_message(Some("-"), false, &b""[..]).unwrap_err();
        assert!(matches!(err, AskMessageError::Empty));
    }

    #[test]
    fn mika1982_an_empty_positional_is_empty() {
        // Comportement historique conservé : `mika ask ""` échouait déjà.
        let err = resolve_ask_message(Some(""), false, PanickingReader).unwrap_err();
        assert!(matches!(err, AskMessageError::Empty));
    }

    #[test]
    fn mika1982_a_read_failure_is_surfaced_not_swallowed() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("broken pipe"))
            }
        }
        let err = resolve_ask_message(None, false, Failing).unwrap_err();
        assert!(matches!(err, AskMessageError::Read(_)));
        assert!(err.to_string().contains("broken pipe"));
    }

    // ---- Le garde structurel R6 ----

    /// Le périmètre du garde : un **file set concret**, pas une notion de
    /// call-graph.
    ///
    /// `ask_message.rs` en est exclu par construction — c'est le lecteur, sa
    /// raison d'être est de contenir les deux appels impurs. `cli.rs` est hors
    /// périmètre : il ne porte que la déclaration clap et aucune lecture.
    ///
    /// `main.rs` est **inclus** bien qu'il porte toutes les sous-commandes, et
    /// l'état du code autorise ce choix sans allowlist : les sous-commandes qui
    /// lisent légitimement l'entrée standard (`credential_helper`, `setup`,
    /// `agents`, `config`, `tasks`, `kg`, `skills`, `provider`, `teams`) le font
    /// toutes depuis leur propre module `commands/*.rs`, jamais depuis le
    /// dispatch. L'inclure ferme le trou qu'un périmètre
    /// `{ask.rs, remote_ask.rs}` laisserait ouvert : une ré-implémentation de la
    /// lecture directement dans le bras `Commands::Ask`, c'est-à-dire
    /// précisément là où la résolution est câblée et donc l'endroit le plus
    /// probable d'un second lecteur.
    const GUARDED_SOURCES: &[(&str, &str)] = &[
        ("crates/mika-cli/src/main.rs", include_str!("main.rs")),
        (
            "crates/mika-cli/src/commands/ask.rs",
            include_str!("commands/ask.rs"),
        ),
        (
            "crates/mika-cli/src/remote_ask.rs",
            include_str!("remote_ask.rs"),
        ),
    ];

    /// mika#1982 R6 — le chemin `ask` n'a qu'un seul lecteur de l'entrée
    /// standard, et c'est ce module.
    ///
    /// **Allowlist : vide, et cette vacuité est l'invariant, pas l'état
    /// initial.** Quand ce test tire, il y a trois cas et deux d'entre eux ne
    /// sont pas des exceptions :
    ///
    /// 1. Il tire sur l'un des deux lecteurs historiques de `commands/ask.rs` —
    ///    alors la migration n'a pas été faite, et la disposition est de la
    ///    faire.
    /// 2. Il tire sur un site imprévu du file set — la disposition est de **le
    ///    replier dans ce module**, c'est-à-dire d'en faire un appelant du
    ///    lecteur unique. **Jamais de l'allowlister.** C'est ce que les trois
    ///    divergences citées en tête de module ont coûté quand la règle
    ///    n'existait pas.
    /// 3. Le repliement est structurellement impossible — un site qui aurait
    ///    besoin de l'entrée standard sur le chemin `ask` pour une raison
    ///    étrangère à la résolution du message. Aucun n'est connu. Ce cas est
    ///    hors périmètre : il rouvre la question de savoir si le chemin `ask` a
    ///    un second usage légitime, ce qui est une décision d'opérateur et non
    ///    un réglage de garde. La disposition est de **halter et de la poser**,
    ///    pas d'ajouter une entrée pour débloquer le build.
    ///
    /// Un test comportemental ne peut pas attraper cette classe : un second
    /// lecteur ne rendrait aucune décision fausse, il rendrait deux décisions
    /// différentes, et toutes les assertions resteraient vertes.
    #[test]
    fn mika1982_the_ask_path_has_a_single_reader_of_the_standard_input() {
        let offenders: Vec<&str> = GUARDED_SOURCES
            .iter()
            .filter(|(_, source)| has_stdin_call_site(source))
            .map(|(path, _)| *path)
            .collect();
        assert!(
            offenders.is_empty(),
            "a second reader of the standard input appeared on the `ask` path, in: {offenders:?}\n\
             When this fires, fold the site into `ask_message::resolve_from_process_stdin` — \
             do NOT allowlist it (mika#1982 R6, § Fire-Disposition)."
        );
    }

    /// La fixture d'évasion, et elle n'est pas décorative.
    ///
    /// Sans elle, un garde qui n'attraperait que la forme qualifiée
    /// `std::io::stdin()` passerait au vert **en ne gardant rien** dès qu'un
    /// second lecteur serait écrit `io::stdin()` — l'écriture importée que ce
    /// crate pratique déjà dans `agents.rs` et `teams.rs`. C'est la panne
    /// silencieuse que tout ce garde existe pour fermer.
    #[test]
    fn mika1982_the_guard_rule_catches_every_reachable_spelling() {
        // Les trois écritures atteignables.
        assert!(has_stdin_call_site("std::io::stdin().lock()"));
        assert!(has_stdin_call_site(
            "    io::stdin().read_to_string(&mut b)?;"
        ));
        assert!(has_stdin_call_site("let h = stdin();"));
        // La tty-ness, sous ses deux formes en usage dans le crate — couverte
        // sans second prédicat.
        assert!(has_stdin_call_site("if !std::io::stdin().is_terminal() {"));
        assert!(has_stdin_call_site(
            "std::io::IsTerminal::is_terminal(&std::io::stdin())"
        ));
        // Et la forme séparée par une espace, que `\s*` couvre.
        assert!(has_stdin_call_site("std::io::stdin ()"));
    }

    /// Les contrôles négatifs : la règle ne doit pas rougir sur ce qui n'est
    /// pas un site d'appel, sans quoi elle serait désarmée par allowlist à la
    /// première fausse alerte.
    #[test]
    fn mika1982_the_guard_rule_spares_prose_and_field_access() {
        // Une mention en prose — le file set en contient (messages d'erreur,
        // commentaires) et elles sont légitimes.
        assert!(!has_stdin_call_site(
            "// read the message from the standard input when the positional is absent"
        ));
        assert!(!has_stdin_call_site(
            "anyhow::bail!(\"Empty message. Provide a message argument or pipe via stdin.\")"
        ));
        // Un accès de champ : `child.stdin.take()` n'est pas une acquisition du
        // descripteur du processus.
        assert!(!has_stdin_call_site(
            "let Some(mut stdin) = child.stdin.take() else {"
        ));
        // Un identifiant qui *contient* le mot n'est pas le mot.
        assert!(!has_stdin_call_site("let my_stdin() = 1;"));
    }
}
