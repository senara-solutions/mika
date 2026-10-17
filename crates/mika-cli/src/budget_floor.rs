//! The per-agent send floor, read from spirit's attestation (mika#2759).
//!
//! The `client >= total` floor of `mika_a2a::client` is resolved on the
//! **process** environment; the per-agent envelopes of mika#2189 live in the
//! target agent's `config.toml`. The two never met, so a call to mika-arch
//! resolved **600 s** against a **900 s** envelope and abandoned a generation
//! the engine still had the right to finish — the 2026-09-11 incident that
//! motivated 300 → 600 in the first place.
//!
//! This module is the one site that closes the gap, and it closes it by
//! **asking**, never by resolving. Three constraints shape it:
//!
//! - **Spirit attests, the CLI renders; never the inverse (mika#2457).** The
//!   envelope is read off `GET /api/v1/agents/{id}/budget`, the record spirit
//!   froze at `init_agent`. A CLI that resolved the envelope itself would read
//!   *this shell's* environment and floor the budget at a value the serving
//!   process is not running under — mika#2304's defect one field over.
//! - **Loopback only.** The attestation endpoint is on the target's own
//!   authority, so asking a *remote* URL for its budget would turn every
//!   `mika ask --remote` into an outbound request derived from caller input.
//!   `local_a2a_agent` therefore admits only a loopback host; `a2a_call.rs`
//!   holds the mirror predicate (it *refuses* loopback, by SSRF design).
//! - **Absent is not zero (mika#2331).** Every degradation returns `None`,
//!   which `resolve_send_timeout_with_floor` reads as "nothing was attested"
//!   and answers with today's budget. A floor is never fabricated, and the
//!   cause is always named rather than swallowed.

use std::time::Duration;

use tracing::{debug, warn};
use url::{Host, Url};

/// Budget for the attestation read itself.
///
/// This is a dashboard-route lookup of a record already in memory, not a
/// generation, and it sits in front of every `mika ask` on the local path: a
/// spirit that is slow to answer must cost the call a few seconds at most, then
/// degrade to the unattested budget. Deliberately **not** a `Duration` const —
/// it is the argument of a resolver, not a policy a `MIKA_*` var overrides.
const ATTESTATION_TIMEOUT_SECS: u64 = 5;

/// The agent named by a **local** A2A URL, or `None` when this URL is not one.
///
/// Two conditions, both necessary:
///
/// 1. the host is loopback — `localhost`, `127.0.0.1` (the whole `127.0.0.0/8`
///    is this machine), or `::1` — under an `http`/`https` scheme;
/// 2. the path is `/a2a/{name}` with a single segment after `a2a`.
///
/// A trailing `/` is accepted: `/a2a/mika-arch/` names the same agent, and
/// making the floor depend on a cosmetic difference would be a worse surprise
/// than accepting it. Anything else — a gateway's `/a2a/{customer}/{agent}`, a
/// remote host, a non-HTTP scheme, an empty name — yields `None`, and the
/// caller keeps the unattested budget.
pub fn local_a2a_agent(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    let is_loopback = match parsed.host()? {
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    };
    if !is_loopback {
        return None;
    }

    let mut segments: Vec<&str> = parsed.path_segments()?.collect();
    // A trailing `/` shows up as a final empty segment.
    if segments.last() == Some(&"") {
        segments.pop();
    }
    match segments.as_slice() {
        ["a2a", name] if !name.is_empty() => Some((*name).to_string()),
        _ => None,
    }
}

/// The per-agent envelope carried by an attestation body, or `None`.
///
/// Pure, so the shape contract with `handle_agent_budget` is testable without a
/// server: the field is `budget.agent_total_timeout_secs`, and a body that does
/// not carry it as an unsigned integer — a 404 error body, a null, a string, a
/// spirit older than the field — attests nothing rather than attesting zero.
pub fn envelope_from_attestation(body: &serde_json::Value) -> Option<u64> {
    body.get("budget")?
        .get("agent_total_timeout_secs")?
        .as_u64()
}

/// Ask the target's own spirit what envelope it serves for the agent this URL
/// names, for use as the send floor.
///
/// The attestation URL is built from the **target URL itself**
/// (`{scheme}://{authority}/api/v1/agents/{name}/budget`), not from a locally
/// resolved spirit address: the question is "what is *this* server running for
/// *this* agent?", and answering it against a different server would be the
/// false provenance this whole module exists to avoid.
///
/// `token` is a parameter, not an env read — this function resolves nothing
/// from the environment. A missing token is **not** short-circuited: the
/// request goes out without `Authorization` and the server answers `401`, which
/// takes the named-cause branch like any other refusal. Short-circuiting would
/// hide "your shell has no token" behind the same silence as "the daemon is
/// down".
///
/// Returns `None` on every degradation, naming the cause:
///
/// | situation | event | level |
/// |---|---|---|
/// | not a local `/a2a/{name}` URL | `a2a_floor_not_local` | DEBUG |
/// | spirit unreachable | `a2a_floor_unattested`, `cause = "unreachable"` | WARN |
/// | any non-2xx | `a2a_floor_unattested`, `cause = "status_<code>"` | WARN |
/// | body without the field | `a2a_floor_unattested`, `cause = "malformed"` | WARN |
/// | attested | `a2a_floor_resolved` | DEBUG |
///
/// The three status cases are **one** tree with the code in a field rather than
/// three event names: 404 ("this server does not resolve that agent"), 401
/// ("this shell's token is not the server's") and the rest are the same
/// question — *the server answered, and the answer was not an envelope* — and
/// splitting them would make a grep for the degradation miss two thirds of it.
///
/// The nominal tree is DEBUG on purpose: this runs on every local `mika ask`,
/// thousands of times a day, and a per-call INFO line is how a log stops being
/// read. Only the degraded trees are WARN.
pub async fn attested_envelope_secs(url: &str, token: Option<&str>) -> Option<u64> {
    let agent = match local_a2a_agent(url) {
        Some(agent) => agent,
        None => {
            debug!(event = "a2a_floor_not_local", %url, "no per-agent send floor");
            return None;
        }
    };

    let parsed = Url::parse(url).ok()?;
    let authority = parsed.authority();
    let attestation = format!(
        "{}://{}/api/v1/agents/{}/budget",
        parsed.scheme(),
        authority,
        agent
    );

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(ATTESTATION_TIMEOUT_SECS))
        .build()
        .ok()?;
    let mut request = http.get(&attestation);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            warn!(
                event = "a2a_floor_unattested",
                cause = "unreachable",
                %agent,
                error = %err,
                "per-agent send floor unattested; keeping the unattested budget"
            );
            return None;
        }
    };

    let status = response.status();
    if !status.is_success() {
        warn!(
            event = "a2a_floor_unattested",
            cause = format!("status_{}", status.as_u16()),
            %agent,
            "per-agent send floor unattested; keeping the unattested budget"
        );
        return None;
    }

    let body: serde_json::Value = match response.json().await {
        Ok(body) => body,
        Err(err) => {
            warn!(
                event = "a2a_floor_unattested",
                cause = "malformed",
                %agent,
                error = %err,
                "per-agent send floor unattested; keeping the unattested budget"
            );
            return None;
        }
    };

    match envelope_from_attestation(&body) {
        Some(envelope) => {
            debug!(
                event = "a2a_floor_resolved",
                %agent,
                attested_envelope_secs = envelope,
                "per-agent send floor read from spirit's attestation"
            );
            Some(envelope)
        }
        None => {
            warn!(
                event = "a2a_floor_unattested",
                cause = "malformed",
                %agent,
                "per-agent send floor unattested; keeping the unattested budget"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mika2759_the_predicate_admits_a_local_a2a_url() {
        assert_eq!(
            local_a2a_agent("http://localhost:8080/a2a/mika-arch").as_deref(),
            Some("mika-arch")
        );
        assert_eq!(
            local_a2a_agent("http://127.0.0.1:8080/a2a/mika-dev").as_deref(),
            Some("mika-dev")
        );
        // Bracketed IPv6 loopback is the same machine, spelled differently.
        assert_eq!(
            local_a2a_agent("http://[::1]:8080/a2a/mika-qa").as_deref(),
            Some("mika-qa")
        );
        // A trailing slash names the same agent.
        assert_eq!(
            local_a2a_agent("http://localhost:8080/a2a/mika-arch/").as_deref(),
            Some("mika-arch")
        );
        // No port is as local as any port.
        assert_eq!(
            local_a2a_agent("http://localhost/a2a/mika-arch").as_deref(),
            Some("mika-arch")
        );
    }

    #[test]
    fn mika2759_the_predicate_refuses_everything_that_is_not_this_machine() {
        // A remote host: asking it for its budget would be an outbound request
        // derived from caller input.
        assert_eq!(
            local_a2a_agent("https://gw.example.com/a2a/mika-arch"),
            None
        );
        // ...including one whose name merely contains "localhost".
        assert_eq!(
            local_a2a_agent("http://localhost.evil.example/a2a/mika-arch"),
            None
        );
        // The gateway's two-segment form names a customer, then an agent.
        assert_eq!(
            local_a2a_agent("http://localhost:8080/a2a/acme/mika-arch"),
            None
        );
        // A path that is not an A2A endpoint at all.
        assert_eq!(
            local_a2a_agent("http://localhost:8080/api/v1/agents/mika-arch"),
            None
        );
        // `/a2a/` names no agent.
        assert_eq!(local_a2a_agent("http://localhost:8080/a2a/"), None);
        assert_eq!(local_a2a_agent("http://localhost:8080/a2a"), None);
        // A non-HTTP scheme has no attestation endpoint behind it.
        assert_eq!(local_a2a_agent("file:///a2a/mika-arch"), None);
        // Not a URL.
        assert_eq!(local_a2a_agent("mika-arch"), None);
    }

    #[test]
    fn mika2759_the_parser_reads_the_attested_envelope_and_nothing_else() {
        // The shape `handle_agent_budget` actually serves.
        let attested = serde_json::json!({
            "budget": {"agent_total_timeout_secs": 900, "llm_http_timeout_secs": 240},
            "model_drift": {"state": "in_sync"},
        });
        assert_eq!(envelope_from_attestation(&attested), Some(900));

        // Absent is not zero (mika#2331), in each of the ways it can be absent.
        assert_eq!(
            envelope_from_attestation(&serde_json::json!({"budget": {}})),
            None
        );
        assert_eq!(
            envelope_from_attestation(
                &serde_json::json!({"budget": {"agent_total_timeout_secs": null}})
            ),
            None
        );
        assert_eq!(
            envelope_from_attestation(
                &serde_json::json!({"budget": {"agent_total_timeout_secs": "900"}})
            ),
            None
        );
        assert_eq!(
            envelope_from_attestation(
                &serde_json::json!({"budget": {"agent_total_timeout_secs": -900}})
            ),
            None
        );
        // The 404 body: an error, not an envelope, and never a default.
        assert_eq!(
            envelope_from_attestation(
                &serde_json::json!({"error": "agent 'nope' is not resolved on this server"})
            ),
            None
        );
        assert_eq!(envelope_from_attestation(&serde_json::Value::Null), None);
    }

    /// Mount a throwaway attestation server and return its address.
    ///
    /// The handle is returned so the caller can `abort()` it; a test that drops
    /// it instead leaves the task running for the rest of the binary.
    async fn serve(app: axum::Router) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app.into_make_service())
                .await
                .unwrap();
        });
        (addr, handle)
    }

    /// A well-formed internal token. Nothing in these tests verifies it; it is
    /// here because the production call passes one and the shape should match.
    fn token() -> String {
        "ab".repeat(32)
    }

    /// AC1 — a local target whose spirit attests `agent_total_timeout_secs = 900`
    /// floors the send budget at 900 s. This is mika#2759 itself, end to end
    /// over a real socket.
    ///
    /// The second assertion is a **property**, not the number 600 → 900: this
    /// test binary inherits whatever `MIKA_A2A_TIMEOUT_SECS` the shell carries,
    /// and pinning the arithmetic would mean either a `#[serial]` env dance or a
    /// green that depends on the machine. The number lives in
    /// `mika_a2a::client`'s own tests, on the pure core; what is this site's to
    /// hold is that the attested envelope reaches the `max` and never loses it.
    #[tokio::test]
    async fn mika2759_une_enveloppe_attestee_a_900_remonte_le_plancher_du_client() {
        use axum::{Router, routing::get};

        let app = Router::new().route(
            "/api/v1/agents/mika-arch/budget",
            get(|| async {
                axum::Json(serde_json::json!({
                    "attested": true,
                    "budget": {"agent_total_timeout_secs": 900, "llm_http_timeout_secs": 240},
                }))
            }),
        );
        let (addr, server) = serve(app).await;

        let target = format!("http://{addr}/a2a/mika-arch");
        let attested = attested_envelope_secs(&target, Some(&token())).await;
        assert_eq!(
            attested,
            Some(900),
            "l'enveloppe per-agent attestée par le serveur visé doit être lue"
        );

        let floored = mika_a2a::client::resolve_send_timeout_with_floor(attested);
        assert!(
            floored >= Duration::from_secs(900),
            "le client ne doit jamais abandonner avant l'enveloppe du moteur \
             (mika#2297) : {floored:?} < 900 s"
        );
        assert_eq!(
            floored,
            mika_a2a::client::resolve_send_timeout().max(Duration::from_secs(900)),
            "le plancher est un `max` : il remonte le budget, il ne le remplace pas"
        );

        server.abort();
    }

    /// AC3 — a spirit nobody answers for attests nothing, and the budget is
    /// exactly the pre-existing one (`cause = "unreachable"`).
    ///
    /// The endpoint comes from `DeadEndpoint::reserve()` and the guard is held
    /// for the whole call (mika#2569): a `bind(:0)` released before the request
    /// is handed to a parallel test's server, which then *answers* the call that
    /// was supposed to fail.
    #[tokio::test]
    async fn mika2759_un_serveur_injoignable_natteste_rien() {
        let dead = mika_common::dead_endpoint::DeadEndpoint::reserve();
        let target = format!("http://{}/a2a/mika-arch", dead.addr());

        let attested = attested_envelope_secs(&target, Some(&token())).await;
        assert_eq!(
            attested, None,
            "un serveur injoignable n'atteste rien — et surtout pas zéro \
             (mika#2331)"
        );
        assert_eq!(
            mika_a2a::client::resolve_send_timeout_with_floor(attested),
            mika_a2a::client::resolve_send_timeout(),
            "sans attestation, le budget est octet pour octet celui d'avant"
        );
    }

    /// AC3 — every non-2xx attests nothing, 404 and 401 included, and each
    /// degrades to `resolve_send_timeout()`.
    ///
    /// The three live in one tree with the code in a field: 404 ("this server
    /// does not resolve that agent"), 401 ("this shell's token is not the
    /// server's") and a 5xx are the same answer — *the server replied, and the
    /// reply was not an envelope*. `inconnu` is routed nowhere, so it exercises
    /// the 404 the router itself produces, not a hand-written one.
    #[tokio::test]
    async fn mika2759_aucun_non_2xx_natteste_une_enveloppe() {
        use axum::{Router, http::StatusCode, routing::get};

        let app = Router::new()
            .route(
                "/api/v1/agents/denied/budget",
                get(|| async { (StatusCode::UNAUTHORIZED, "{\"error\":\"unauthorized\"}") }),
            )
            .route(
                "/api/v1/agents/ghost/budget",
                get(|| async {
                    (
                        StatusCode::NOT_FOUND,
                        "{\"error\":\"agent 'ghost' is not resolved on this server\"}",
                    )
                }),
            )
            .route(
                "/api/v1/agents/broken/budget",
                get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
            );
        let (addr, server) = serve(app).await;

        for agent in ["denied", "ghost", "broken", "inconnu"] {
            let target = format!("http://{addr}/a2a/{agent}");
            let attested = attested_envelope_secs(&target, Some(&token())).await;
            assert_eq!(
                attested, None,
                "« {agent} » : une réponse qui n'est pas une enveloppe n'atteste rien"
            );
            assert_eq!(
                mika_a2a::client::resolve_send_timeout_with_floor(attested),
                mika_a2a::client::resolve_send_timeout(),
                "« {agent} » : l'arbre status_<code> rend le budget d'avant"
            );
        }

        server.abort();
    }

    /// AC3 — a 2xx whose body carries no envelope attests nothing
    /// (`cause = "malformed"`), in both of the ways a body can fail.
    ///
    /// `illisible` is not JSON at all (a proxy's HTML, a truncated stream);
    /// `ancien` is valid JSON from a spirit predating the field. One cause
    /// covers both because the caller's question is identical — *can I read an
    /// envelope out of this?* — and the answer is no in the same way.
    #[tokio::test]
    async fn mika2759_un_2xx_sans_enveloppe_lisible_natteste_rien() {
        use axum::{Router, routing::get};

        let app = Router::new()
            .route(
                "/api/v1/agents/illisible/budget",
                get(|| async { "<html>not json at all</html>" }),
            )
            .route(
                "/api/v1/agents/ancien/budget",
                get(|| async {
                    axum::Json(serde_json::json!({"budget": {"llm_http_timeout_secs": 240}}))
                }),
            );
        let (addr, server) = serve(app).await;

        for agent in ["illisible", "ancien"] {
            let target = format!("http://{addr}/a2a/{agent}");
            let attested = attested_envelope_secs(&target, Some(&token())).await;
            assert_eq!(
                attested, None,
                "« {agent} » : un corps sans enveloppe lisible n'atteste rien"
            );
            assert_eq!(
                mika_a2a::client::resolve_send_timeout_with_floor(attested),
                mika_a2a::client::resolve_send_timeout(),
                "« {agent} » : l'arbre malformed rend le budget d'avant"
            );
        }

        server.abort();
    }

    /// AC4 — a target the predicate refuses issues **no request at all**, and a
    /// missing token does not suppress the one it should issue.
    ///
    /// A counting fallback is what makes both observable. The predicate's
    /// refusals are already asserted purely above; what cannot be seen from a
    /// pure test is whether the refusal happens *before* the `GET` — and the
    /// server here is live and would answer every one of these paths. The same
    /// counter is this test's non-vacuity control: the final call, deliberately
    /// made with `token: None`, must move it to 1. A short-circuit on the
    /// absent token would hide "your shell has no token" behind the silence of
    /// "the daemon is down".
    #[tokio::test]
    async fn mika2759_une_cible_refusee_nemet_aucun_appel_et_un_jeton_absent_nen_supprime_aucun() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let hits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&hits);
        let app = axum::Router::new().fallback(move || {
            let counted = Arc::clone(&counted);
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                axum::Json(serde_json::json!({
                    "budget": {"agent_total_timeout_secs": 900},
                }))
            }
        });
        let (addr, server) = serve(app).await;

        for refused in [
            // The gateway's two-segment form: a customer, then an agent.
            format!("http://{addr}/a2a/acme/mika-arch"),
            // Not an A2A endpoint.
            format!("http://{addr}/api/v1/agents/mika-arch"),
            // `/a2a/` names no agent.
            format!("http://{addr}/a2a/"),
            // A `--remote` target: never this machine, whatever it would answer.
            "https://gw.example.com/a2a/mika-arch".to_string(),
        ] {
            let attested = attested_envelope_secs(&refused, Some(&token())).await;
            assert_eq!(attested, None, "« {refused} » : le prédicat doit refuser");
            assert_eq!(
                hits.load(Ordering::SeqCst),
                0,
                "« {refused} » : aucun GET ne doit partir avant le prédicat"
            );
        }

        // Non-vacuity, and the token contract: the request goes out without an
        // `Authorization` header rather than not going out.
        let target = format!("http://{addr}/a2a/mika-arch");
        assert_eq!(
            attested_envelope_secs(&target, None).await,
            Some(900),
            "un jeton absent ne court-circuite pas l'appel"
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "le compteur doit compter — sinon les assertions à 0 ci-dessus ne \
             prouvent rien (mika#2103)"
        );

        server.abort();
    }

    /// The floored resolver has **one** call site in this crate, and it is
    /// `remote_ask.rs`.
    ///
    /// The floor is a *per-exchange* decision: it depends on the URL, so a
    /// second site would be a second place where an agent is derived from a URL
    /// and an envelope asked for. The two would drift — one of them would keep
    /// calling `A2aClient::new` and resolve 600 s against a 900 s envelope,
    /// which is mika#2759 itself, re-created one caller over.
    ///
    /// **The exception is a *path*, not an allowlist entry.** `ALLOWED` ships
    /// empty and must stay empty (the mika#2425 / mika#2457 form): when a second
    /// site appears, the remedy is to route it through the one that exists, not
    /// to list it here. An allowlist that stops being empty is the false green
    /// this guard exists to refuse.
    #[test]
    fn mika2759_le_plancher_a_un_seul_site() {
        const ALLOWED: &[&str] = &[];
        const NEEDLE: &str = "resolve_send_timeout_with_floor";
        const SITE: &str = "remote_ask.rs";

        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));

        let mut violations: Vec<String> = Vec::new();
        scanner.for_each(|path, source| {
            let rel = path
                .strip_prefix(scanner.src_root())
                .unwrap_or(path)
                .display()
                .to_string();
            if rel == SITE || ALLOWED.contains(&rel.as_str()) {
                return;
            }
            for (idx, line) in source.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if line.contains(NEEDLE) {
                    violations.push(format!("{rel}:{} — {}", idx + 1, line.trim()));
                }
            }
        });

        assert!(
            violations.is_empty(),
            "mika#2759 : le plancher d'envoi per-agent doit se résoudre à un \
             seul site ({SITE}). Site(s) supplémentaire(s) :\n{}\n\
             Le remède est de router l'appel par le site existant, PAS d'ajouter \
             une entrée à ALLOWED — qui doit rester vide.",
            violations.join("\n")
        );
    }

    /// ...and the scan that asserts it actually looked.
    ///
    /// A `for_each` over an empty file set passes the guard above in silence,
    /// which is the decoration mika#2103 is about. Two conditions: the sweep
    /// sees a plausible number of files, and it sees the one file the exception
    /// names — a path exception for a file the scanner never visits protects
    /// nothing.
    #[test]
    fn mika2759_le_scan_du_plancher_nest_pas_vide() {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let files = scanner.files();

        assert!(
            files.len() > 10,
            "mika#2759 : le scan du plancher est vide ou quasi vide \
             ({} fichier(s)) — il ne protège rien.",
            files.len()
        );
        assert!(
            files.iter().any(|p| p.ends_with("remote_ask.rs")),
            "mika#2759 : le scan ne voit pas remote_ask.rs, le fichier que \
             l'exception de chemin nomme."
        );
    }
}
