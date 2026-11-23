//! Le seul site du dépôt qui connaît l'adresse du gateway (mika#3125).
//!
//! # Pourquoi ce module existe
//!
//! Le client écrivait `http://localhost:3001` comme défaut, et le gateway écoute
//! sur `8080` : le défaut du client n'était le défaut de rien. Deux commandes du
//! CLI portaient chacune leur propre `fn gateway_url()` — donc deux déclarations
//! du même port, libres de diverger, et c'est l'une des deux qui a été mesurée
//! injoignable. Le port est désormais une donnée unique, lue par le serveur pour
//! écouter et par le client pour composer son adresse.
//!
//! # Pourquoi dans `mika-common` et pas dans `mika-gateway`
//!
//! `mika-cli` ne dépend pas de `mika-gateway` et ne doit pas commencer à en
//! dépendre pour lire un `u16` ; les deux crates dépendent déjà d'ici.
//!
//! # La limite, dite plutôt que découverte
//!
//! [`DEFAULT_GATEWAY_PORT`] et le défaut de `MIKA_SPIRIT_PORT` valent tous deux
//! `8080` : sur un hôte qui fait tourner mika-spirit **et** le gateway, l'un des
//! deux est nécessairement configuré, et c'est alors sa variable qui tranche. Le
//! défaut du client ne peut pas être meilleur que celui du serveur ; ce module le
//! rend égal, il ne le rend pas devin.

use tracing::warn;

/// Port d'écoute par défaut du gateway — le seul site de vérité du dépôt.
///
/// `crates/mika-gateway/src/settings.rs::default_port` le rend, et c'est la
/// valeur que `GatewaySettings` applique quand `MIKA_GATEWAY_PORT` est absent.
/// Le client le lit pour composer son adresse par défaut : avant mika#3125 il
/// écrivait `3001`, qui n'était le défaut de rien.
pub const DEFAULT_GATEWAY_PORT: u16 = 8080;

/// Le geste de configuration, nommé une fois pour tous les messages d'erreur.
///
/// Il vit ici parce que c'est ici que les deux variables sont lues : un message
/// composé ailleurs serait une seconde déclaration de la même configuration,
/// qui divergerait au premier renommage.
pub const CONFIG_HINT: &str =
    "configurable par MIKA_GATEWAY_URL, ou par MIKA_GATEWAY_PORT que le gateway lit lui-même";

/// Adresse de base du gateway, lue dans l'environnement du process.
///
/// Charge le `.env` de la home avant de relire — geste repris de l'ancien
/// `webhook.rs::gateway_url` et conservé bien que `main.rs` le fasse déjà avant
/// le dispatch : `load_dotenv` est idempotent (dotenvy n'écrase pas), et la
/// garder rend la fonction correcte pour un appelant futur qui l'atteindrait par
/// un chemin court.
pub fn gateway_base_url() -> String {
    if let Ok(home) = crate::home::resolve_home_dir() {
        crate::dotenv::load_dotenv(&home);
    }
    gateway_base_url_from(|key| std::env::var(key).ok())
}

/// La même résolution, sur un lecteur d'environnement donné.
///
/// Ordre : `MIKA_GATEWAY_URL` → `MIKA_GATEWAY_PORT` → [`DEFAULT_GATEWAY_PORT`].
/// Une valeur vide est traitée comme absente, et un `MIKA_GATEWAY_PORT`
/// illisible retombe sur le défaut **en nommant la cause** par un `warn!` — un
/// port qui ne parse pas n'est pas un zéro, et refuser l'envoi là-dessus
/// transformerait une faute de frappe cosmétique en perte d'escalade.
pub fn gateway_base_url_from(read: impl Fn(&str) -> Option<String>) -> String {
    if let Some(url) = declared(read("MIKA_GATEWAY_URL")) {
        return url;
    }

    let port = match declared(read("MIKA_GATEWAY_PORT")) {
        Some(raw) => raw.trim().parse::<u16>().unwrap_or_else(|e| {
            warn!(
                value = %raw,
                error = %e,
                default_port = DEFAULT_GATEWAY_PORT,
                "MIKA_GATEWAY_PORT ne se lit pas comme un port — repli sur le défaut partagé"
            );
            DEFAULT_GATEWAY_PORT
        }),
        None => DEFAULT_GATEWAY_PORT,
    };

    format!("http://localhost:{port}")
}

/// Une variable posée mais vide n'est pas une variable posée.
///
/// La valeur rendue l'est **telle quelle**, sans rognage : c'est l'adresse que
/// l'opérateur a écrite, et c'est elle que le message d'erreur doit citer.
fn declared(value: Option<String>) -> Option<String> {
    value.filter(|raw| !raw.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lecteur figé : aucune mutation de `std::env`, donc aucune fragilité en
    /// exécution parallèle — l'environnement du process est un état partagé que
    /// `cargo test` fait lire à d'autres tests au même instant.
    fn reader(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        }
    }

    fn shared_default() -> String {
        format!("http://localhost:{DEFAULT_GATEWAY_PORT}")
    }

    #[test]
    fn mika3125_une_url_posee_est_rendue_telle_quelle() {
        let url = gateway_base_url_from(reader(&[("MIKA_GATEWAY_URL", "http://gw.test:9999")]));
        assert_eq!(url, "http://gw.test:9999");
    }

    #[test]
    fn mika3125_une_url_vide_est_traitee_comme_absente() {
        let url = gateway_base_url_from(reader(&[("MIKA_GATEWAY_URL", "")]));
        assert_eq!(url, shared_default());
        assert!(
            !url.contains("3001"),
            "mika#3125 : `3001` n'est le défaut de rien — le gateway écoute sur \
             {DEFAULT_GATEWAY_PORT}. Adresse rendue : {url}"
        );
    }

    #[test]
    fn mika3125_un_port_pose_compose_ladresse() {
        let url = gateway_base_url_from(reader(&[("MIKA_GATEWAY_PORT", "8081")]));
        assert_eq!(url, "http://localhost:8081");
    }

    #[test]
    fn mika3125_lurl_garde_la_priorite_sur_le_port() {
        let url = gateway_base_url_from(reader(&[
            ("MIKA_GATEWAY_URL", "http://gw.test:9999"),
            ("MIKA_GATEWAY_PORT", "8081"),
        ]));
        assert_eq!(
            url, "http://gw.test:9999",
            "l'URL est l'adresse complète : un port à côté ne peut que la contredire"
        );
    }

    #[test]
    fn mika3125_un_port_illisible_retombe_sur_le_defaut_sans_paniquer() {
        let url = gateway_base_url_from(reader(&[("MIKA_GATEWAY_PORT", "plif")]));
        assert_eq!(
            url,
            shared_default(),
            "une faute de frappe sur le port ne doit pas coûter l'escalade"
        );
    }

    #[test]
    fn mika3125_sans_rien_le_defaut_est_celui_que_le_gateway_applique() {
        let url = gateway_base_url_from(reader(&[]));
        assert_eq!(url, shared_default());
        assert!(
            !url.contains("3001"),
            "mika#3125 : l'assertion qui rougit si le littéral revient dans la \
             chaîne de résolution. Adresse rendue : {url}"
        );
    }
}
