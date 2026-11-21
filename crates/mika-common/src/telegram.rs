//! Limites du transport Telegram, nommées là où les deux crates les voient.
//!
//! `mika-agent` s'en sert pour refuser avant l'appel réseau ; `mika-gateway`
//! s'en sert pour découper le texte tel qu'il partira, préfixe compris.
//!
//! **Qui découpe : le gateway** (mika#3114). `handle_send` coupe tout texte qui
//! dépasse la limite en plusieurs `sendMessage`, envoyés dans l'ordre et sans
//! perte ; le découpage lui-même vit dans
//! `crates/mika-gateway/src/telegram_split.rs`. La relation 1:1 que mika#2134
//! protégeait tient toujours : elle est par **morceau envoyé** et non par appel
//! à `/send` — une ligne `outbound_messages` et le même préfixe `[agent] ` sur
//! chacun, pour que `reply_to_message` route une réponse faite à n'importe
//! lequel. Jusqu'à mika#3114 ce fichier disait l'inverse, et prêtait au gateway
//! une garde à 4096 qu'il n'avait pas : un texte trop long partait tel quel,
//! Telegram répondait 400 et le message était perdu.
//!
//! L'outil `send_message` de l'agent garde sa garde propre, plus
//! conservatrice — refuser avant le réseau reste utile ; son message au modèle
//! dit encore que le gateway ne découpe pas. `Tracked in:`
//! senara-solutions/mika#2983.

/// Nombre maximal d'unités UTF-16 dans le champ `text` de `sendMessage`.
///
/// Telegram compte en unités UTF-16, pas en octets ni en `char`. Un emoji hors
/// BMP compte 2. `str::len()` (octets) sur-restreindrait tout texte accentué ;
/// `chars().count()` sous-restreindrait les emoji.
pub const MAX_TEXT_UTF16_UNITS: usize = 4096;

/// Longueur d'un texte telle que Telegram la compte.
pub fn text_len_utf16(s: &str) -> usize {
    s.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_counts_as_bytes() {
        assert_eq!(text_len_utf16("abcd"), 4);
    }

    #[test]
    fn accented_counts_utf16_not_bytes() {
        // "éé" est 4 octets mais 2 unités UTF-16 (chaque 'é' précomposé = 1 unité).
        assert_eq!(text_len_utf16("éé"), 2);
        assert_eq!("éé".len(), 4);
    }

    #[test]
    fn non_bmp_emoji_counts_two_utf16_units() {
        // "👆" (U+1F446) est hors BMP : 4 octets, 2 unités UTF-16 (paire de substitution).
        assert_eq!(text_len_utf16("👆"), 2);
        assert_eq!("👆".len(), 4);
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(text_len_utf16(""), 0);
    }
}
