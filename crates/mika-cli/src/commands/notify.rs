use anyhow::{Context, Result};
use secrecy::{ExposeSecret, SecretString};

use crate::cli::{NotifyChannel, NotifySeverity};
use crate::init;

/// Well-known fixed-UUID for the notifications session.
/// Deliberately synthetic (zero-filled prefix + "notify" digits) to sort distinctly
/// from real UUIDs and be instantly recognizable in DB queries.
const NOTIFICATIONS_SESSION_ID: &str = "00000000-0000-0000-0000-700000710717";

/// Agent that owns the notifications session.
/// `mika notify` always writes to the default "mika" agent's DB.
const NOTIFICATIONS_AGENT: &str = "mika";

/// Send an operator notification.
///
/// Writes a message to the well-known notifications session in the DB.
/// Optionally sends via Telegram through the gateway's `/send` endpoint.
pub async fn run(text: &str, channel: &NotifyChannel, severity: &NotifySeverity) -> Result<()> {
    let ctx = init::init_db_only_for_agent(NOTIFICATIONS_AGENT)?;

    let severity_str = match severity {
        NotifySeverity::Info => "info",
        NotifySeverity::Warn => "warn",
        NotifySeverity::Escalate => "escalate",
    };

    let channel_str = match channel {
        NotifyChannel::Cli => "cli",
        NotifyChannel::Telegram => "telegram",
    };

    // Ensure the notifications session exists (idempotent).
    let session_metadata = serde_json::json!({
        "source": "mika-notify",
        "name": "mika notify",
    });
    ctx.async_db
        .create_session_if_not_exists(
            NOTIFICATIONS_SESSION_ID,
            NOTIFICATIONS_AGENT,
            "notifications",
            Some(&session_metadata.to_string()),
        )
        .await
        .context("failed to create notifications session")?;

    // Format the notification message with severity prefix.
    let formatted = format!("[{severity_str}] {text}");

    // Build message metadata.
    let metadata = serde_json::json!({
        "source": "mika-notify",
        "severity": severity_str,
        "channel": channel_str,
    });

    // Save the notification as a system-role message.
    ctx.async_db
        .save_message_with_metadata(
            NOTIFICATIONS_SESSION_ID,
            "system",
            &formatted,
            Some(&metadata.to_string()),
            None,  // no trace_id — CLI invocation
            false, // not internal — operator-visible
        )
        .await
        .context("failed to save notification message")?;

    // L'écriture en base a eu lieu : le fait acquis se dit **avant** le geste
    // qui peut échouer, sinon le `bail!` d'une livraison manquée emporterait la
    // confirmation d'un fait déjà acquis (discipline des trois sorties de
    // mika#1727 vol V2d-4c).
    eprintln!(
        "✓ Notification saved (session={}, severity={severity_str}).",
        NOTIFICATIONS_SESSION_ID
    );

    // Telegram delivery (optional).
    let (attempt, gw_url) = if matches!(channel, NotifyChannel::Telegram) {
        let gw_url = mika_common::gateway_endpoint::gateway_base_url();
        let attempt = match send_via_gateway(&ctx, &formatted).await {
            Ok(()) => {
                eprintln!("✓ Notification sent via Telegram.");
                Ok(())
            }
            Err(e) => Err(format!("{e:#}")),
        };
        (Some(attempt), gw_url)
    } else {
        (None, String::new())
    };

    delivery_verdict(attempt, &gw_url)
}

/// Ce que la commande doit rendre, une fois la base écrite et la livraison tentée.
///
/// `None` ⇒ aucune livraison n'était demandée (canal `cli`) : rien ne change, et
/// `url` n'est **pas lu** — c'est pourquoi le test de ce cas passe une chaîne
/// vide, seul montage qui distingue « non lu » de « lu puis ignoré ».
/// `Some(Ok(()))` ⇒ livré.
/// `Some(Err(cause))` ⇒ la notification est en base et n'a pas été livrée.
///
/// Elle est extraite en fonction pure pour être attestable sans base ni réseau.
/// Jusqu'à mika#3125 l'échec faisait un `eprintln!` et `run` rendait `Ok(())` :
/// la commande sortait en 0, et aucun appelant ne pouvait distinguer *livré* de
/// *perdu*.
///
/// **Pas de code de sortie dédié, et c'est délibéré.** `anyhow` sort en 1, comme
/// l'échec d'écriture en base. Un code dédié séparerait *« nulle part »* de
/// *« en base, pas livré »*, mais aucun appelant mesuré ne décide sur le code —
/// inventer un code que personne ne lit reproduirait le défaut qu'on corrige,
/// une surface qui affirme sans destinataire. La distinction est portée par le
/// message, qui a un lecteur.
fn delivery_verdict(attempt: Option<Result<(), String>>, url: &str) -> Result<()> {
    match attempt {
        // Aucune livraison demandée : le canal `cli` ne traverse rien.
        None => Ok(()),
        Some(Ok(())) => Ok(()),
        // Quatre faits, dont aucun n'est décoratif : la cause, l'adresse visée
        // (le seul moyen de voir un port faux sans lire le code), le geste de
        // configuration — porté par `CONFIG_HINT`, de sorte que ce fichier ne
        // nomme lui-même aucune variable d'environnement — et le fait que la
        // notification **est** en base, avec sa session, ce qui sépare « le
        // gateway est mort » de « l'appel n'a pas eu lieu ».
        Some(Err(cause)) => anyhow::bail!(
            "Telegram delivery failed: {cause}\n  \
             gateway targeted: {url} — {hint}\n  \
             the notification IS saved in the DB (session={session}) but was NOT delivered.",
            hint = mika_common::gateway_endpoint::CONFIG_HINT,
            session = NOTIFICATIONS_SESSION_ID,
        ),
    }
}

/// POST the notification text to the gateway's `/send` endpoint for Telegram delivery.
async fn send_via_gateway(ctx: &init::DbContext, text: &str) -> Result<()> {
    let gw_url = mika_common::gateway_endpoint::gateway_base_url();

    let internal_token: &SecretString = ctx.settings.internal_token.as_ref().ok_or_else(|| {
        anyhow::anyhow!("MIKA_INTERNAL_TOKEN not configured — cannot send via gateway")
    })?;

    // Resolve the operator's Telegram chat_id from the DB.
    let chat_id_str = ctx
        .async_db
        .get_customer_config("chat_id")
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "chat_id not configured — no Telegram pairing yet. \
                 Send a message to the bot on Telegram first."
            )
        })?;

    let chat_id: i64 = chat_id_str
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid chat_id in DB: {e}"))?;

    if chat_id == 0 {
        anyhow::bail!("chat_id is 0 (no reply channel) — cannot deliver via Telegram");
    }

    let payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
        "agent_name": NOTIFICATIONS_AGENT,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{gw_url}/send"))
        .bearer_auth(internal_token.expose_secret())
        .json(&payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                anyhow::anyhow!("gateway unreachable at {gw_url}: {e}")
            } else if e.is_timeout() {
                anyhow::anyhow!("gateway request timed out: {e}")
            } else {
                anyhow::anyhow!("gateway request failed: {e}")
            }
        })?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect::<String>();
        if body.is_empty() {
            anyhow::bail!("gateway /send returned {status}")
        } else {
            anyhow::bail!("gateway /send returned {status}: {body}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notifications_session_id_is_valid_uuid() {
        // The fixed UUID must parse as a valid UUID.
        uuid::Uuid::parse_str(NOTIFICATIONS_SESSION_ID)
            .expect("NOTIFICATIONS_SESSION_ID must be a valid UUID");
    }

    #[test]
    fn test_notifications_session_id_is_clearly_synthetic() {
        // Must start with zeros (clearly not a random v4 UUID).
        assert!(
            NOTIFICATIONS_SESSION_ID.starts_with("00000000-"),
            "session ID should be clearly synthetic"
        );
    }

    #[test]
    fn test_severity_formatting() {
        // Verify the formatted message includes severity prefix.
        let text = "test message";
        let formatted = format!("[info] {text}");
        assert_eq!(formatted, "[info] test message");

        let formatted = format!("[escalate] {text}");
        assert_eq!(formatted, "[escalate] test message");
    }

    // --- L'adresse du gateway a un seul résolveur (mika#3125) ---

    /// Les deux variables dont la lecture **est** une résolution d'adresse.
    ///
    /// Elles vivent ici comme littéraux de test, donc dans la région masquée par
    /// [`mika_common::source_guard`] : c'est précisément ce que le contre-témoin
    /// de la garde suivante atteste.
    const AIGUILLES: &[&str] = &["MIKA_GATEWAY_URL", "MIKA_GATEWAY_PORT"];

    /// Le fichier dont la production sert de témoin au balayage.
    const SITE_TEMOIN: &str = "commands/notify.rs";

    /// Balaie la moitié production du crate et rend `(chemin relatif, source)`.
    ///
    /// Un seul balayage pour la garde et pour ses témoins : deux balayages
    /// seraient deux lectures libres de diverger, et c'est le témoin — celui qui
    /// atteste que la garde a bien regardé — qui perdrait son sens.
    fn production_du_crate() -> Vec<(String, String)> {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let mut vues = Vec::new();
        scanner.for_each(|path, source| {
            let rel = path
                .strip_prefix(scanner.src_root())
                .unwrap_or(path)
                .display()
                .to_string();
            vues.push((rel, source.to_string()));
        });
        vues
    }

    #[test]
    fn mika3125_le_crate_cli_ne_resout_plus_ladresse_du_gateway() {
        // Livrée vide, et vide par construction : quand la garde rougit, le
        // remède est l'appel que son message nomme, jamais une entrée ici.
        const ALLOWED: &[&str] = &[];
        assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");

        let mut violations: Vec<String> = Vec::new();
        for (rel, source) in production_du_crate() {
            if ALLOWED.contains(&rel.as_str()) {
                continue;
            }
            for (idx, line) in source.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if AIGUILLES.iter().any(|needle| line.contains(needle)) {
                    violations.push(format!("{rel}:{} — {}", idx + 1, line.trim()));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "mika#3125 : l'adresse du gateway se résout à un seul site \
             (mika_common::gateway_endpoint). Site(s) supplémentaire(s) :\n{}\n\
             Le remède est d'appeler `gateway_base_url()`, PAS d'ajouter une entrée à \
             ALLOWED — qui doit rester vide.",
            violations.join("\n")
        );
    }

    #[test]
    fn mika3125_le_scan_a_bien_lu_la_production_et_masque_les_tests() {
        let vues = production_du_crate();
        let (_, production) = vues
            .iter()
            .find(|(rel, _)| rel == SITE_TEMOIN)
            .unwrap_or_else(|| panic!("le balayage n'a pas visité {SITE_TEMOIN}"));

        // Témoin : sans lui, une garde balayant une racine fausse serait verte
        // **par cécité** — l'exact défaut que `ProductionScanner` existe pour
        // refuser (mika#2398).
        assert!(
            production.contains("async fn send_via_gateway("),
            "le balayage n'a pas lu la moitié production de {SITE_TEMOIN}"
        );

        // Contre-témoin : le masquage remplace les régions `#[cfg(test)]` par des
        // lignes vides sans jamais tronquer. Sans cette assertion, les deux
        // aiguilles citées par `AIGUILLES` juste au-dessus feraient rougir la
        // garde, et on l'aurait « réparée » par une entrée d'allowlist.
        assert!(
            !production.contains("fn test_notifications_session_id_is_valid_uuid"),
            "le balayage n'a pas masqué la moitié test de {SITE_TEMOIN}"
        );
    }

    // --- Une livraison manquée ne sort plus en 0 (mika#3125) ---
    //
    // **Ce que ces tests ne couvrent pas, dit ici pour qu'un futur lecteur ne
    // prenne pas leur vert pour une preuve de livraison.** Aucun n'atteste une
    // livraison Telegram réelle : elle dépend d'un `chat_id` appairé et d'un
    // gateway vivant, deux états de l'hôte hors de ce harness. Ils attestent que
    // le **verdict** est juste, jamais qu'un message est arrivé.

    #[test]
    fn mika3125_sans_livraison_demandee_rien_ne_change() {
        // L'URL passée est vide : le seul montage qui distingue « `url` n'est pas
        // lu » de « lu puis ignoré ». Le canal `cli` ne traverse cette fonction
        // qu'avec `None`, et un `url` lu sur ce bras serait un message composé
        // pour une adresse que personne n'a visée.
        assert!(
            delivery_verdict(None, "").is_ok(),
            "le canal `cli` ne demande aucune livraison : il sort en 0"
        );
    }

    #[test]
    fn mika3125_une_livraison_reussie_sort_en_zero() {
        assert!(
            delivery_verdict(Some(Ok(())), "http://localhost:8080").is_ok(),
            "une livraison réussie sort en 0"
        );
    }

    #[test]
    fn mika3125_une_livraison_manquee_sort_non_zero_en_portant_les_quatre_faits() {
        let cause = "gateway unreachable at http://gw.test:9999";
        let erreur = delivery_verdict(Some(Err(cause.to_string())), "http://gw.test:9999")
            .expect_err("une livraison manquée doit sortir non-zéro");
        let message = format!("{erreur:#}");

        for (fait, attendu) in [
            ("la cause de l'échec", cause),
            ("l'adresse visée", "http://gw.test:9999"),
            (
                "le geste de configuration",
                mika_common::gateway_endpoint::CONFIG_HINT,
            ),
            ("la session en base", NOTIFICATIONS_SESSION_ID),
        ] {
            assert!(
                message.contains(attendu),
                "le message d'échec ne porte pas {fait} ({attendu:?}). Message : {message}"
            );
        }
    }

    #[test]
    fn mika3125_un_echec_ne_laisse_jamais_croire_a_une_livraison() {
        let erreur = delivery_verdict(Some(Err("boom".to_string())), "http://localhost:8080")
            .expect_err("une livraison manquée doit sortir non-zéro");
        let message = format!("{erreur:#}");

        // Contrôle négatif, et non une relecture du message : un texte qui
        // laisse croire à une livraison affirmerait l'état que le gateway ne
        // nous a précisément pas attesté (grounding rule du dépôt). Les trois
        // aiguilles sont les marqueurs de succès de cette commande — le `✓` de
        // ses deux lignes, et le littéral de celle de Telegram.
        for affirmation in ["✓", "Notification sent", "sent via Telegram"] {
            assert!(
                !message.contains(affirmation),
                "le message d'échec porte « {affirmation} », qui annonce une \
                 livraison que rien n'a attestée. Message : {message}"
            );
        }
    }
}
