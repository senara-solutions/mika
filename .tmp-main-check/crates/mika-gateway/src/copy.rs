//! The gateway's user-facing copy — one producer, two languages (mika#2025).
//!
//! # Why this module exists
//!
//! Sixteen string literals sat at their send sites, in hard-coded English,
//! served to users whose account, invite and conversation were in French. Six of
//! them are reached *before* pairing, so there is no customer row to read a
//! locale from and no agent to route through: this copy never traverses a
//! persona, which is why mika#2023 (the agent's English greeting) does not and
//! cannot fix it. It is a distinct surface, and the body of mika#2025 established
//! that before we did.
//!
//! # The shape, and why it is not an i18n framework
//!
//! Fifteen static keys, no interpolation, no pluralization, two languages. The
//! house has already settled this form twice — `hosting_ground_truth_line`
//! (mika#2290) and `MIKA_DOCTRINE_BODY_OPERATOR`/`_FAMILY` (mika#2292): the same
//! fact written twice behind an exhaustive `match`. A dependency and an
//! extraction pipeline for fifteen `&'static str` would cost more than they
//! return.
//!
//! **No `_ =>` arm, and that is the whole property.** The compiler — not a
//! reviewer — forces every new message and every new language to decide. A
//! `_ => <english>` arm would make a missing translation silent and reinstate
//! the original defect one key at a time. `copy::tests::mika2025_v11_*` pins it.
//!
//! # What holds the other half
//!
//! The regression this module cannot see behaviourally is a *seventeenth* literal
//! added at a send site: it makes no decision wrong, it merely restores the
//! defect on one key. `routes::tests::mika2025_v10_*` scans production sources
//! and refuses a string literal in any argument of `send_message`. Its allowlist
//! ships **empty**; when it reddens, the resolution is to route through this
//! module, never to add an entry (the rule mika#2323 had to write for
//! `ACTOR_READING_PREDICATES_ALLOWED`).

use crate::telegram::Locale;

/// One key per user-facing message the gateway sends directly to a Telegram
/// user.
///
/// The variant name is what makes a send site readable — `send_message(chat_id,
/// copy::render(UserMessage::UnlinkWarning, locale))` says what is being sent
/// without quoting it.
///
/// Note `InvalidInvite` has **two** call sites (a malformed token, and a token
/// the atomic pairing UPDATE did not match). They were two identical literals
/// before this module; they are one key now, which is the deduplication rather
/// than a merge that lost a distinction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UserMessage {
    /// Bare `/start`, no invite payload.
    BareStartWelcome,
    /// Media we do not forward (sticker, voice, video…).
    UnsupportedMedia,
    /// A message from a chat_id bound to no customer.
    NotPaired,
    /// Photo above the 5 MB download ceiling.
    PhotoTooLarge,
    /// Photo whose format is outside `SUPPORTED_IMAGE_MIMES`.
    PhotoUnsupportedFormat,
    /// Photo download from Telegram failed.
    PhotoDownloadFailed,
    /// Invite token malformed, expired, already used, or unmatched.
    InvalidInvite,
    /// `telegram_chat_id` UNIQUE violation — one Telegram, one Mika.
    TelegramAlreadyLinked,
    /// Any other UNIQUE violation during pairing.
    PairingFailed,
    /// `/unlink` on a paired chat — the warning-then-confirm step.
    UnlinkWarning,
    /// `/unlink` on a chat bound to nothing.
    UnlinkNotLinked,
    /// `/unlink confirm` that released a binding.
    UnlinkConfirmed,
    /// `/unlink confirm` with nothing to release.
    UnlinkNothingToUnlink,
    /// Timeout, broken pipe, failed query — try again.
    TransientError,
    /// Agent container unreachable (connect error).
    AgentOffline,
    /// `/unlink <something we do not recognize>` (mika#2025 D4).
    ///
    /// Rendered through [`render_unlink_suffix_unrecognized`], which appends the
    /// quoted suffix. The static half lives here like every other key, so this
    /// module stays the only place the crate's user-facing wording is written.
    UnlinkSuffixUnrecognized,
}

/// Every key, for tests that must iterate the surface rather than trust a list
/// kept by hand.
///
/// A variant missing here is caught by `mika2025_v3_inventory_is_complete`,
/// which counts this array against the `match` in [`render`] — otherwise "all
/// sixteen render in both languages" would be a statement about whatever subset
/// someone remembered.
#[cfg(test)]
const ALL_USER_MESSAGES: &[UserMessage] = &[
    UserMessage::BareStartWelcome,
    UserMessage::UnsupportedMedia,
    UserMessage::NotPaired,
    UserMessage::PhotoTooLarge,
    UserMessage::PhotoUnsupportedFormat,
    UserMessage::PhotoDownloadFailed,
    UserMessage::InvalidInvite,
    UserMessage::TelegramAlreadyLinked,
    UserMessage::PairingFailed,
    UserMessage::UnlinkWarning,
    UserMessage::UnlinkNotLinked,
    UserMessage::UnlinkConfirmed,
    UserMessage::UnlinkNothingToUnlink,
    UserMessage::TransientError,
    UserMessage::AgentOffline,
    UserMessage::UnlinkSuffixUnrecognized,
];

/// The exact confirmation the copy prescribes, in both languages.
///
/// Test-only, and named rather than repeated: three guards assert on it — that
/// it leads the `/unlink` copy (mika#2025 R3), that the HTML rendering wraps it
/// in `<code>`, and that `parse_update` actually accepts what the copy tells the
/// user to type. That last one is the coupling this constant exists for: the
/// command is written in two modules, and nothing but a test joins them.
#[cfg(test)]
const UNLINK_CONFIRM_COMMAND: &str = "/unlink confirm";

/// Marker the `/unlink` warning is built around; R3 says the command comes
/// before it.
#[cfg(test)]
const WARNING_MARKER: &str = "⚠️";

/// How many characters of a refused suffix are quoted back.
///
/// Bounded because the suffix is user input on a line the user reads: an
/// unbounded echo turns a corrective message into a wall. Counted in
/// **characters**, never bytes — a `&str` byte range would panic mid-codepoint
/// on exactly the accented input this ticket exists to serve (mika#764).
const MAX_CITED_SUFFIX_CHARS: usize = 48;

/// Render one static key in one language.
///
/// The `match` is over the **pair**, with no wildcard arm. That is the guarantee
/// (mika#2025 D2): a new key or a new language does not compile until every
/// combination is written.
pub(crate) fn render(msg: UserMessage, locale: Locale) -> &'static str {
    match (msg, locale) {
        // -- Onboarding -------------------------------------------------------
        (UserMessage::BareStartWelcome, Locale::Fr) => {
            "Bienvenue ! Si tu as un lien d'invitation, utilise-le pour commencer. \
             Si tu es déjà installé, écris-moi simplement un message."
        }
        (UserMessage::BareStartWelcome, Locale::En) => {
            "Welcome! If you have an invite link, please use it to get started. \
             If you're already set up, just type a message."
        }

        (UserMessage::NotPaired, Locale::Fr) => {
            "Ton compte n'est pas encore associé. Utilise ton lien d'invitation pour commencer."
        }
        (UserMessage::NotPaired, Locale::En) => {
            "Please pair your account first. Use your invite link to get started."
        }

        (UserMessage::InvalidInvite, Locale::Fr) => "Ce lien d'invitation est invalide ou expiré.",
        (UserMessage::InvalidInvite, Locale::En) => "Invalid or expired invite link.",

        (UserMessage::TelegramAlreadyLinked, Locale::Fr) => {
            "Ce compte Telegram est déjà associé à un autre compte Mika. \
             Si ce compte t'appartient, envoie d'abord /unlink depuis celui-ci, \
             puis clique à nouveau sur ton lien d'invitation. Sinon, contacte le support."
        }
        (UserMessage::TelegramAlreadyLinked, Locale::En) => {
            "This Telegram account is already linked to another Mika account. \
             If it's an account you control, send /unlink from that account \
             first, then click your invite link again. Otherwise, contact support."
        }

        (UserMessage::PairingFailed, Locale::Fr) => "L'association a échoué. Contacte le support.",
        (UserMessage::PairingFailed, Locale::En) => "Pairing failed. Please contact support.",

        // -- Media ------------------------------------------------------------
        (UserMessage::UnsupportedMedia, Locale::Fr) => {
            "Je sais lire les messages texte et les images. \
             Ce type de contenu n'est pas encore pris en charge."
        }
        (UserMessage::UnsupportedMedia, Locale::En) => {
            "I can read text and image messages. This media type isn't supported yet."
        }

        (UserMessage::PhotoTooLarge, Locale::Fr) => {
            "Cette image est trop grande pour que je puisse la traiter. \
             Envoie une photo plus légère (moins de 5 Mo)."
        }
        (UserMessage::PhotoTooLarge, Locale::En) => {
            "That image is too large for me to process. \
             Please send a smaller photo (under 5 MB)."
        }

        (UserMessage::PhotoUnsupportedFormat, Locale::Fr) => {
            "Je n'ai pas reconnu le format de cette image. \
             Envoie une image JPEG, PNG, GIF ou WebP."
        }
        (UserMessage::PhotoUnsupportedFormat, Locale::En) => {
            "I couldn't recognize that image format. \
             Please send a JPEG, PNG, GIF, or WebP image."
        }

        (UserMessage::PhotoDownloadFailed, Locale::Fr) => {
            "Je n'ai pas réussi à récupérer ta photo. Essaie de l'envoyer à nouveau."
        }
        (UserMessage::PhotoDownloadFailed, Locale::En) => {
            "Sorry, I couldn't download your photo. Please try sending it again."
        }

        // -- /unlink ----------------------------------------------------------
        //
        // The action comes FIRST and the warning second (mika#2025 R3/D3). The
        // reported behaviour was a reader skimming "⚠️ … cannot be undone" and
        // sending back the command they already knew — `/unlink` — instead of
        // the one on the last line they never reached. Line order is the only
        // half of the salience that survives the kill-switch and the plain-text
        // fallback of mika#2291; the backticks are a reinforcement that degrades
        // into a legible quotation, which is why the command is NOT bolded (the
        // raw `**` is precisely the marker whose cost mika#2291 measured).
        (UserMessage::UnlinkWarning, Locale::Fr) => {
            "Pour confirmer, envoie : `/unlink confirm`\n\n\
             ⚠️ Cela libérera ton Telegram de ce compte Mika.\n\
             Il te faudra un nouveau lien d'invitation pour te reconnecter.\n\
             Cette action est définitive."
        }
        (UserMessage::UnlinkWarning, Locale::En) => {
            "To confirm, send: `/unlink confirm`\n\n\
             ⚠️ Unlinking will release your Telegram from this Mika account.\n\
             You will need a new invite link from your admin to re-pair.\n\
             This cannot be undone."
        }

        // Same salience discipline: the command first, the diagnosis second.
        // The trailing colon is load-bearing — `render_unlink_suffix_unrecognized`
        // appends the quoted suffix right after it.
        (UserMessage::UnlinkSuffixUnrecognized, Locale::Fr) => {
            "Pour confirmer, envoie exactement : `/unlink confirm`\n\n\
             Je n'ai pas reconnu ce que tu as écrit après /unlink :"
        }
        (UserMessage::UnlinkSuffixUnrecognized, Locale::En) => {
            "To confirm, send exactly: `/unlink confirm`\n\n\
             I didn't recognize what you wrote after /unlink:"
        }

        (UserMessage::UnlinkNotLinked, Locale::Fr) => {
            "Ton Telegram n'est associé à aucun compte Mika."
        }
        (UserMessage::UnlinkNotLinked, Locale::En) => {
            "Your Telegram is not linked to any Mika account."
        }

        (UserMessage::UnlinkConfirmed, Locale::Fr) => {
            "✅ C'est fait. Ton lien d'invitation (ou un nouveau) ouvrira \
             une nouvelle session quand tu voudras."
        }
        (UserMessage::UnlinkConfirmed, Locale::En) => {
            "✅ Unlinked. Your invite link (or a new one) will pair a fresh \
             session when you're ready."
        }

        (UserMessage::UnlinkNothingToUnlink, Locale::Fr) => {
            "Il n'y a rien à délier. Envoie d'abord /unlink si tu voulais \
             libérer une association Telegram."
        }
        (UserMessage::UnlinkNothingToUnlink, Locale::En) => {
            "Nothing to unlink. Send /unlink first if you meant to release \
             a Telegram binding."
        }

        // -- Failure paths ----------------------------------------------------
        (UserMessage::TransientError, Locale::Fr) => {
            "J'ai un souci en ce moment. Réessaie dans un instant."
        }
        (UserMessage::TransientError, Locale::En) => {
            "I'm having trouble right now. Please try again in a moment."
        }

        // `console.getmika.ai` is an invariant, not copy: it stays byte-identical
        // in both languages.
        (UserMessage::AgentOffline, Locale::Fr) => {
            "Ton assistante Mika est actuellement hors ligne. \
             Contacte ton administrateur ou vérifie ton abonnement \
             sur console.getmika.ai."
        }
        (UserMessage::AgentOffline, Locale::En) => {
            "Your Mika assistant is currently offline. \
             Please contact your administrator or check your subscription status \
             at console.getmika.ai."
        }
    }
}

/// Render the refused-suffix reply, quoting back what the user actually typed.
///
/// The only site in this module that allocates, and the only one that touches
/// user content. Everything localized still comes from [`render`]; all this adds
/// is the suffix itself, so the single-producer property holds.
///
/// Two sanitisations, both about the *reading* rather than about safety — the
/// escaping of `<`, `>` and `&` is `telegram_markdown`'s job and happens after
/// this:
///
/// - backticks are dropped, because the suffix is quoted inside a code span and
///   a stray backtick would close it early;
/// - newlines become spaces and the whole is capped at
///   [`MAX_CITED_SUFFIX_CHARS`] characters, so a pasted wall of text cannot turn
///   a corrective message into one.
pub(crate) fn render_unlink_suffix_unrecognized(suffix: &str, locale: Locale) -> String {
    let cited: String = suffix
        .chars()
        .filter(|c| *c != '`')
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_CITED_SUFFIX_CHARS)
        .collect();

    format!(
        "{} `{}`",
        render(UserMessage::UnlinkSuffixUnrecognized, locale),
        cited.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram_markdown::{render_html, tokenize};

    const LOCALES: &[Locale] = &[Locale::Fr, Locale::En];

    /// mika#2025 V3 — every key renders non-empty text in both languages.
    #[test]
    fn mika2025_v3_every_key_renders_in_both_languages() {
        for msg in ALL_USER_MESSAGES {
            for locale in LOCALES {
                let text = render(*msg, *locale);
                assert!(
                    !text.trim().is_empty(),
                    "{msg:?} renders empty in {}",
                    locale.as_str()
                );
            }
        }
    }

    /// mika#2025 V3 — the inventory the tests iterate is the whole surface.
    ///
    /// Without this, every assertion in this module would be a statement about
    /// whatever subset of variants someone remembered to list, and a key added
    /// to the `enum` but not to `ALL_USER_MESSAGES` would be untested in
    /// silence. The `match` in `render` is the ground truth: one arm per
    /// (variant, locale) pair, so its arm count is twice the variant count.
    #[test]
    fn mika2025_v3_inventory_is_complete() {
        let body = render_fn_body();
        let fr_arms = body.matches(", Locale::Fr) =>").count();
        let en_arms = body.matches(", Locale::En) =>").count();

        assert_eq!(
            fr_arms,
            ALL_USER_MESSAGES.len(),
            "render() has {fr_arms} French arms but ALL_USER_MESSAGES lists \
             {} keys — one of the two is out of date, and the tests that iterate \
             the inventory are only as complete as it is",
            ALL_USER_MESSAGES.len()
        );
        assert_eq!(fr_arms, en_arms, "a locale is missing arms");
    }

    /// mika#2025 V4 — a forgotten translation is a copied one.
    ///
    /// Exhaustiveness makes a *missing* arm a compile error; it says nothing
    /// about an arm filled by pasting the English. This is the half the compiler
    /// cannot hold.
    #[test]
    fn mika2025_v4_the_two_languages_differ_on_every_key() {
        for msg in ALL_USER_MESSAGES {
            assert_ne!(
                render(*msg, Locale::Fr),
                render(*msg, Locale::En),
                "{msg:?} renders identically in both languages — a translation \
                 was pasted rather than written"
            );
        }
    }

    /// mika#2025 V5 / R3 / AC2 — the action comes before the warning.
    ///
    /// **This is the assertion that carries the ticket's correction**, and it
    /// deliberately depends on no rendering: it holds under HTML, under
    /// `MIKA_TELEGRAM_HTML_RENDER=0`, and under mika#2291's plain-text fallback.
    #[test]
    fn mika2025_v5_the_command_precedes_the_warning() {
        for locale in LOCALES {
            let text = render(UserMessage::UnlinkWarning, *locale);
            let command = text
                .find(UNLINK_CONFIRM_COMMAND)
                .unwrap_or_else(|| panic!("{} copy lost the command", locale.as_str()));
            let warning = text
                .find(WARNING_MARKER)
                .unwrap_or_else(|| panic!("{} copy lost the warning marker", locale.as_str()));

            assert!(
                command < warning,
                "{}: the command must come before the warning — a reader who \
                 skims sees ⚠️ and replies with the command they already know",
                locale.as_str()
            );
        }
    }

    /// The command sits on the first line, not merely before the warning.
    #[test]
    fn mika2025_r3_the_command_is_on_the_first_line() {
        for locale in LOCALES {
            let text = render(UserMessage::UnlinkWarning, *locale);
            let first = text.lines().next().unwrap_or("");
            assert!(
                first.contains(UNLINK_CONFIRM_COMMAND),
                "{}: first line is {first:?}",
                locale.as_str()
            );
        }
    }

    /// mika#2025 V9 / RI3 — the copy survives the HTML rendering of mika#2291.
    ///
    /// Worth more than reasoning about escaping: the accents and apostrophes the
    /// French copy introduces are not reserved, and this proves it against the
    /// real renderer rather than against a belief about it.
    #[test]
    fn mika2025_v9_unlink_copy_renders_as_html_with_a_code_span() {
        for locale in LOCALES {
            let html = render_html(&tokenize(render(UserMessage::UnlinkWarning, *locale)));
            assert!(
                html.contains(&format!("<code>{UNLINK_CONFIRM_COMMAND}</code>")),
                "{}: the command must render as a code span, got {html:?}",
                locale.as_str()
            );
            assert!(
                html.contains(WARNING_MARKER),
                "{}: the warning marker was lost in rendering",
                locale.as_str()
            );
        }
    }

    /// RI3 — no copy introduces a character the HTML rendering has to escape.
    ///
    /// `<`, `>` and `&` are the three reserved characters. Keeping them out of
    /// the copy means the rendered body is the copy, which is what makes the
    /// assertion above readable.
    #[test]
    fn mika2025_ri3_no_copy_carries_a_reserved_character() {
        for msg in ALL_USER_MESSAGES {
            for locale in LOCALES {
                let text = render(*msg, *locale);
                for reserved in ['<', '>', '&'] {
                    assert!(
                        !text.contains(reserved),
                        "{msg:?}/{} carries {reserved:?}, which the HTML \
                         rendering must escape",
                        locale.as_str()
                    );
                }
            }
        }
    }

    /// The refused suffix is quoted back, in the caller's language.
    #[test]
    fn mika2025_d4_the_refused_suffix_is_quoted_back() {
        let fr = render_unlink_suffix_unrecognized("oui", Locale::Fr);
        assert!(fr.contains("`oui`"), "{fr:?}");
        assert!(fr.contains("Je n'ai pas reconnu"), "{fr:?}");
        assert!(fr.starts_with("Pour confirmer"), "{fr:?}");

        let en = render_unlink_suffix_unrecognized("oui", Locale::En);
        assert!(en.contains("`oui`"), "{en:?}");
        assert!(en.starts_with("To confirm"), "{en:?}");

        // The command still leads, exactly as in the plain warning.
        for text in [&fr, &en] {
            assert!(
                text.lines()
                    .next()
                    .unwrap_or("")
                    .contains(UNLINK_CONFIRM_COMMAND),
                "{text:?}"
            );
        }
    }

    /// The citation is bounded and cannot break its own code span.
    ///
    /// The suffix is the one piece of user input this module handles. A backtick
    /// would close the span early; an unbounded paste would bury the command the
    /// message exists to repeat. Truncation counts characters — a byte range
    /// would panic mid-codepoint on the accented input this ticket serves
    /// (mika#764).
    #[test]
    fn mika2025_the_cited_suffix_is_sanitised_and_bounded() {
        let backticked = render_unlink_suffix_unrecognized("a`b", Locale::Fr);
        assert!(!backticked.contains("a`b"), "{backticked:?}");
        assert!(backticked.contains("`ab`"), "{backticked:?}");

        let long = "é".repeat(500);
        let rendered = render_unlink_suffix_unrecognized(&long, Locale::Fr);
        let cited = rendered
            .rsplit_once('`')
            .and_then(|(head, _)| head.rsplit_once('`').map(|(_, c)| c.to_string()))
            .expect("the citation is delimited by backticks");
        assert_eq!(cited.chars().count(), MAX_CITED_SUFFIX_CHARS);

        let multiline = render_unlink_suffix_unrecognized("a\nb", Locale::Fr);
        assert!(multiline.ends_with("`a b`"), "{multiline:?}");

        // A suffix that sanitises away to nothing (`/unlink ```` `) leaves an
        // empty citation. Pinned rather than branched on: the message still
        // leads with the command and still says the suffix was not recognized,
        // which is the whole job. A special case here would be a branch written
        // for an adversarial input nobody types, and it is cheaper to know this
        // behaviour than to guess at it later.
        let all_stripped = render_unlink_suffix_unrecognized("```", Locale::Fr);
        assert!(all_stripped.ends_with("``"), "{all_stripped:?}");
        assert!(
            all_stripped
                .lines()
                .next()
                .unwrap_or("")
                .contains(UNLINK_CONFIRM_COMMAND),
            "the command must lead even when the citation is empty: {all_stripped:?}"
        );
    }

    /// The command the copy prescribes is the command the parser accepts.
    ///
    /// The command is written in two modules — the copy here, the recognized
    /// forms in `telegram::UNLINK_CONFIRM_FORMS` — and nothing but this joins
    /// them. Renaming one alone would tell every user, in both languages, to
    /// send something the gateway refuses: the exact friction mika#2025 was
    /// filed about, reintroduced by a rename.
    #[test]
    fn mika2025_the_prescribed_command_is_the_parsed_command() {
        use crate::telegram::{ParsedMessage, TelegramChat, TelegramMessage, TelegramUpdate};

        let parsed = crate::telegram::parse_update(&TelegramUpdate {
            update_id: 1,
            message: Some(TelegramMessage {
                chat: TelegramChat { id: 42 },
                text: Some(UNLINK_CONFIRM_COMMAND.to_string()),
                photo: None,
                caption: None,
                document: None,
                reply_to_message: None,
                from: None,
            }),
        });

        assert_eq!(
            parsed,
            ParsedMessage::UnlinkConfirm { chat_id: 42 },
            "the copy prescribes {UNLINK_CONFIRM_COMMAND:?} but the parser does \
             not commit the release on it"
        );
    }

    /// mika#2025 V11 / D2 — `render` has no wildcard arm.
    ///
    /// A behavioural test cannot see this: a `_ => <english>` arm makes no
    /// assertion in this module fail, it silently restores the defect on every
    /// key added after it. So the guard is structural, on the model of
    /// `mika2305_the_label_match_has_no_wildcard_arm`.
    #[test]
    fn mika2025_v11_render_has_no_wildcard_arm() {
        let body = render_fn_body();

        for forbidden in ["_ =>", "(_,", ", _)", "_ if "] {
            assert!(
                !body.contains(forbidden),
                "render() contains {forbidden:?} — a catch-all arm makes a \
                 missing translation silent and reinstates mika#2025 one key at \
                 a time. Resolution: write the arm."
            );
        }

        // Anti-vacuity (mika#2205): distinguish "the scan found nothing" from
        // "the scan looked at nothing". Without this, a broken extractor that
        // returned an empty body would leave every assertion above green.
        assert!(
            body.contains("(UserMessage::UnlinkWarning, Locale::Fr) =>"),
            "the extractor did not capture render()'s body — the guard above \
             asserted over nothing"
        );
        assert!(
            arm_scan_detects_a_wildcard(),
            "the wildcard detector does not detect a wildcard"
        );
    }

    /// Negative control for the scan above, on a fabricated body.
    fn arm_scan_detects_a_wildcard() -> bool {
        let fabricated = "match (msg, locale) {\n    _ => \"english\",\n}";
        ["_ =>", "(_,", ", _)", "_ if "]
            .iter()
            .any(|f| fabricated.contains(f))
    }

    /// Body of `fn render`, comments stripped.
    ///
    /// Code, never prose: this module's own doc comments *name* the constructs
    /// the guard forbids, and a guard that could not tolerate being described
    /// would force the documentation to go quiet about the rule it carries — the
    /// reasoning `telegram_markdown`'s mika#2291 guard already had to write.
    fn render_fn_body() -> String {
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/copy.rs"),
        )
        .expect("the guard must be able to read copy.rs");

        let start = src
            .find("pub(crate) fn render(msg: UserMessage, locale: Locale)")
            .expect("render() must be findable by its signature");
        let rest = &src[start..];
        let end = rest
            .find("\n}\n")
            .expect("render() must close at column zero");

        rest[..end]
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
