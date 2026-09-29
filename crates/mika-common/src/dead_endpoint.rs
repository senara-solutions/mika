//! Un point de terminaison de boucle locale dont la connexion est **refusée**
//! et dont le port ne peut être attribué à personne tant que le garde vit
//! (mika#2569).
//!
//! # Le défaut que ça ferme
//!
//! Le motif historique était « lier `127.0.0.1:0`, relever le port, **libérer**
//! l'écouteur, puis appeler ce port en supposant que personne ne l'a repris ».
//! Les tests d'un même binaire tournent en parallèle et montent chacun un
//! serveur factice sur `127.0.0.1:0` : l'un d'eux peut recevoir le port libéré
//! entre le `drop` et l'appel. Mesuré le 2026-09-28 sur le job `Check` de
//! mika#2561 — l'appel « vers un port mort » a été servi par le serveur factice
//! d'un test voisin, et `Check` est un check **requis** sur `main`.
//!
//! # Le remède : on ne ferme pas du tout
//!
//! Un socket TCP lié à `127.0.0.1:P` sur lequel `listen()` n'a **jamais** été
//! appelé donne les deux propriétés d'un coup :
//!
//! - **Le port est réservé.** `inet_bind` insère le socket dans le bind bucket,
//!   `listen()` ou non. Un `bind("127.0.0.1:0")` concurrent parcourt la plage
//!   éphémère et saute les buckets occupés. La collision n'est levée que si
//!   **tous** les sockets liés portent `SO_REUSEADDR` (et qu'aucun n'est en
//!   `LISTEN`), ou `SO_REUSEPORT` des deux côtés.
//! - **La connexion est refusée.** Le socket n'étant pas en `LISTEN`, un SYN
//!   entrant ne rencontre aucun socket d'écoute : le noyau répond RST,
//!   `connect()` rend `ECONNREFUSED` **immédiatement**, `reqwest` rend
//!   `is_connect() == true`, et `TransportFailure::classify` rend `Unreachable`.
//!   Aucun délai à attendre, aucune classe à deviner.
//!
//! **Ne pas poser `SO_REUSEADDR` est porteur, et c'est pourquoi
//! [`std::net::TcpListener`] ne convient pas** : sa `bind` le pose sur Unix, et
//! deux sockets `SO_REUSEADDR` dont aucun n'est en `LISTEN` peuvent lier la même
//! adresse exacte. Le garde perdrait précisément la propriété de réservation
//! qu'on lui demande. D'où [`socket2::Socket::new`], qui ne pose rien.
//!
//! # Deux voies refusées, avec leur mesure
//!
//! - **Viser une adresse non routable** (`203.0.113.x`, RFC 5737) : refusé
//!   structurellement par la garde mika#2495, et pour une raison mesurée — le
//!   bac à sable de dispatch porte un proxy d'egress (mika#2049) qui intercepte
//!   une telle adresse et rend un statut HTTP, jamais une erreur de transport.
//!   Le test serait alors vert sur le CI et rouge en pilote.
//! - **Sérialiser le test fautif** : `cargo test` lance les binaires de test en
//!   parallèle, donc un `bind(:0)` d'un **autre** binaire peut recevoir le port
//!   libéré ; et rien ne force un futur test du même binaire à prendre le mutex.
//!
//! # Le raisonnement est ASSERTÉ, jamais supposé
//!
//! Un raisonnement noyau dans un doc-comment est une hypothèse. Les deux
//! propriétés sont donc vérifiées **à la réservation**, sur tous les sites d'un
//! coup, et [`DeadEndpoint::reserve`] panique en nommant celle qui a lâché. Si
//! une plateforme se comporte autrement, elle rougit **dans le garde**, avec un
//! message qui la nomme, au lieu de dériver dans un test distant.
//!
//! # Usage
//!
//! ```ignore
//! let dead = DeadEndpoint::reserve();
//! let url = format!("http://{}/a2a/cust-5/mika-prime", dead.addr());
//! // `dead` doit rester VIVANT pendant tout l'appel : c'est lui qui tient le port.
//! ```
//!
//! Test-only par construction : il n'entre dans aucun binaire de production
//! (motif [`crate::source_guard`], mika#2398).

use socket2::{Domain, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::time::Duration;

/// Budget de la sonde de refus. Large : on veut distinguer « refusé » de
/// « quelque chose écoute », pas mesurer une latence. Sur un port réservé et non
/// écoutant, `connect` rend `ECONNREFUSED` en microsecondes.
const REFUSAL_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Un port de boucle locale tenu par un socket lié et **non écoutant**.
///
/// RAII : tant que la valeur vit, le port est à nous et toute connexion vers lui
/// est refusée. Un site qui oublie de la tenir (`let _ = DeadEndpoint::reserve();`)
/// libère immédiatement et retrouve le défaut mika#2569 — d'où un nom de liaison
/// explicite sur chaque site.
#[derive(Debug)]
pub struct DeadEndpoint {
    /// Tenu, jamais lu : c'est sa **durée de vie** qui est le mécanisme.
    _socket: Socket,
    addr: SocketAddr,
}

impl DeadEndpoint {
    /// Réserve un port de boucle locale et **vérifie les deux propriétés** avant
    /// de rendre.
    ///
    /// # Panics
    ///
    /// En nommant laquelle a lâché — une fixture muette est pire qu'absente :
    ///
    /// - la création, la liaison ou la relecture d'adresse a échoué ;
    /// - la **réservation** ne mord pas (un `TcpListener::bind` concurrent sur ce
    ///   port exact réussit) ;
    /// - le **refus** ne mord pas (une connexion vers ce port aboutit).
    #[must_use]
    pub fn reserve() -> Self {
        let socket = Socket::new(Domain::IPV4, Type::STREAM, None)
            .expect("mika#2569 — création du socket de réservation impossible");

        // Aucun `set_reuse_address` : c'est l'absence de `SO_REUSEADDR` qui donne
        // la propriété de réservation, et c'est pourquoi la bibliothèque standard
        // ne convient pas ici (cf. doc du module). Vu rouge à la livraison : poser
        // ce seul réglage fait passer `assert_reservation_holds` au rouge.
        let wildcard = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);
        socket
            .bind(&wildcard.into())
            .expect("mika#2569 — liaison de 127.0.0.1:0 impossible");

        // Pas de `listen()` : c'est son absence qui fait répondre RST au SYN.

        let addr = socket
            .local_addr()
            .expect("mika#2569 — relecture de l'adresse liée impossible")
            .as_socket()
            .expect("mika#2569 — l'adresse liée n'est pas une adresse IP");

        assert_reservation_holds(addr);
        assert_refusal_holds(addr);

        Self {
            _socket: socket,
            addr,
        }
    }

    /// L'adresse tenue. Valide tant que `self` vit.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

/// Propriété 1 — **la réservation mord** : personne d'autre ne peut lier ce port.
///
/// La sonde passe par [`std::net::TcpListener`], donc par un socket qui **pose**
/// `SO_REUSEADDR` — c'est le pire cas, et celui qu'on veut refuser : si même un
/// binder permissif échoue, un `bind("127.0.0.1:0")` éphémère saute le bucket.
fn assert_reservation_holds(addr: SocketAddr) {
    let taken = std::net::TcpListener::bind(addr);
    assert!(
        taken.is_err(),
        "mika#2569 — la RÉSERVATION ne mord pas : un second bind sur {addr} a réussi \
         alors qu'un socket lié le tient. Sur cette plateforme, tenir un socket lié \
         ne réserve pas le port, et le garde ne protège de rien — la course que ce \
         helper existe pour rendre impossible est restée ouverte."
    );
}

/// Propriété 2 — **le refus mord** : une connexion vers ce port échoue, vite.
///
/// `connect_timeout` plutôt que `connect` : sur une plateforme où le SYN serait
/// silencieusement absorbé au lieu d'être rejeté, un `connect` nu pendrait
/// jusqu'au budget du système et le test appelant attendrait son propre budget
/// client (600 s côté A2A) avant de rougir.
fn assert_refusal_holds(addr: SocketAddr) {
    let connected = TcpStream::connect_timeout(&addr, REFUSAL_PROBE_TIMEOUT);
    assert!(
        connected.is_err(),
        "mika#2569 — le REFUS ne mord pas : une connexion vers {addr} a abouti alors \
         qu'aucun `listen()` n'a été appelé. Sur cette plateforme un socket lié et \
         non écoutant accepte les connexions, donc un test qui attend une erreur de \
         transport verrait une réponse."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **V5 — la réservation mord.** Le contrôle négatif de cette assertion est
    /// V12 (remplacer `socket2` par `std::net::TcpListener` dans `reserve`),
    /// exécuté à la main : il ne peut pas vivre en test permanent sans dupliquer
    /// l'implémentation.
    #[test]
    fn mika2569_la_reservation_empeche_un_second_bind() {
        let dead = DeadEndpoint::reserve();
        assert!(
            std::net::TcpListener::bind(dead.addr()).is_err(),
            "le port tenu par le garde a pu être lié par quelqu'un d'autre"
        );
    }

    /// **V6 — le refus mord, et il est RAPIDE.** La borne temporelle est le
    /// second contrat : un point de terminaison qui pendrait au lieu de refuser
    /// ferait attendre le budget client de l'appelant (600 s sur le chemin A2A)
    /// avant de rougir.
    #[test]
    fn mika2569_la_connexion_est_refusee_immediatement() {
        let dead = DeadEndpoint::reserve();
        let started = std::time::Instant::now();
        let outcome = TcpStream::connect_timeout(&dead.addr(), REFUSAL_PROBE_TIMEOUT);
        let waited = started.elapsed();

        assert!(outcome.is_err(), "une connexion vers le port mort a abouti");
        assert!(
            waited < Duration::from_secs(1),
            "le refus a demandé {waited:?} : le port absorbe le SYN au lieu de le \
             rejeter, et un test appelant attendrait son propre budget client"
        );
    }

    /// Le port est rendu **à la libération du garde**, et pas avant. Sans cette
    /// moitié, « le garde réserve » serait indistinguable de « le garde réserve
    /// pour toujours », ce qui ferait fuir un port par test.
    #[test]
    fn mika2569_le_port_est_rendu_quand_le_garde_meurt() {
        let addr = {
            let dead = DeadEndpoint::reserve();
            dead.addr()
        };
        assert!(
            std::net::TcpListener::bind(addr).is_ok(),
            "le port {addr} est resté tenu après la mort du garde : chaque \
             réservation fuirait un port éphémère"
        );
    }

    /// Deux gardes vivants ne peuvent pas tenir le même port — la propriété que
    /// la course mesurée violait.
    #[test]
    fn mika2569_deux_gardes_tiennent_des_ports_distincts() {
        let a = DeadEndpoint::reserve();
        let b = DeadEndpoint::reserve();
        assert_ne!(
            a.addr(),
            b.addr(),
            "deux réservations concurrentes ont reçu le même port"
        );
    }
}
