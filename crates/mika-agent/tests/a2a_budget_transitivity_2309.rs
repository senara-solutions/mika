//! Garde B (mika#2309) — la chaîne `client >= total > http`, affirmée en un lieu.
//!
//! # Ce que ce test ajoute, et ce qu'il n'ajoute pas
//!
//! Les deux moitiés de l'ordre sont déjà tenues à l'exécution, et chacune ne
//! connaît que sa paire :
//!
//! - `mika_a2a::client::resolve_send_timeout` pose le plancher `client >= total`
//!   (mika#2297) ;
//! - `mika_agent::server::budget_guard` refuse le démarrage si `http >= total`
//!   (mika#2293), via [`LlmTimeoutBudget::validate`].
//!
//! Personne n'affirme la **chaîne complète**, et personne ne l'affirme sans
//! démarrer un serveur. C'est là, et seulement là, que mika#2309 ajoute quelque
//! chose : la **transitivité**, vérifiable en CI.
//!
//! # Pourquoi ici et pas dans `mika-common`
//!
//! `mika-common` ne dépend pas de `mika-a2a` (ce dernier n'a aucune dépendance
//! maison ; c'est `mika-agent` qui tire les deux). Un test posé à côté de
//! `LlmTimeoutBudget` ne pourrait pas voir le client, et la chaîne s'y réduirait
//! à `total > http` — la moitié que `budget_guard` tient déjà. `mika-agent` est
//! le seul crate qui voit les trois valeurs.
//!
//! # `>` et non `>=`
//!
//! Le ticket demandait `client >= total >= http`. Le code refuse `total == http`
//! (`LlmBudgetError::CapNotContained`, « must be **strictly less than** ») :
//! coder le `>=` ferait passer un couple que `assert_llm_budgets_valid` refuse
//! au démarrage, c'est-à-dire un test vert sur une configuration qui empêche
//! mika-spirit de démarrer. La garde porte l'invariant du code, pas sa
//! paraphrase (plan § M4).
//!
//! # Ce que ce test ne peut pas découvrir
//!
//! Sur la voie env, `client >= total` est vrai **par construction** :
//! `resolve_timeout_secs` applique `.max(total)`. Le premier `assert` est donc
//! un **épinglage de ce plancher** — il rougit si quelqu'un retire le `.max()`,
//! ce qui est une régression réelle et c'est le cas `mika2309_the_floor_lifts_a_client_below_the_envelope`
//! ci-dessous — mais il ne peut pas découvrir une divergence, parce que la seule
//! cascade que le client sache lire est celle de l'env du process. La cascade
//! **per-agent** lui est invisible, et c'est là qu'une divergence existe
//! aujourd'hui : voir le contrôle positif
//! `mika2309_client_default_is_below_the_arch_envelope` dans
//! `crates/mika-agent/src/well_known_agents.rs`.
//!
//! # Aucune valeur n'est recopiée
//!
//! Un test qui réécrit `600`, `300`, `120` ne teste que sa propre copie
//! (plan § M6). Les trois nombres sont lus des résolveurs et des constantes
//! exportées ; les seuls littéraux écrits ici sont les **entrées** posées par
//! env dans les cas non-défaut.

use mika_common::llm::LlmTimeoutBudget;
use mika_common::llm::budget::AGENT_TOTAL_TIMEOUT_ENV_VAR;
use mika_common::llm::{DEFAULT_HTTP_TIMEOUT_SECS, HTTP_TIMEOUT_ENV_VAR};

const A2A_TIMEOUT_ENV: &str = mika_a2a::client::TIMEOUT_ENV;

/// Les trois variables qui composent la cascade, remises dans l'état où le test
/// les a trouvées. Un test qui laisse une env posée fait échouer le suivant pour
/// une raison qui n'est pas la sienne.
struct EnvScope {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvScope {
    fn capture() -> Self {
        let keys = [
            A2A_TIMEOUT_ENV,
            AGENT_TOTAL_TIMEOUT_ENV_VAR,
            HTTP_TIMEOUT_ENV_VAR,
        ];
        let saved = keys
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect::<Vec<_>>();
        for (k, _) in &saved {
            // SAFETY: single-threaded via #[serial_test::serial].
            unsafe { std::env::remove_var(k) };
        }
        Self { saved }
    }

    fn set(&self, key: &str, value: &str) {
        // SAFETY: single-threaded via #[serial_test::serial].
        unsafe { std::env::set_var(key, value) };
    }
}

impl Drop for EnvScope {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            // SAFETY: single-threaded via #[serial_test::serial].
            unsafe {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

/// Lit les trois budgets **par les résolveurs de production**.
fn resolved_chain() -> (u64, u64, u64) {
    let client = mika_a2a::client::resolve_send_timeout().as_secs();
    let budget = LlmTimeoutBudget::from_env();
    (
        client,
        budget.agent_total_timeout_secs(),
        budget.http_timeout_secs(),
    )
}

fn assert_chain(client: u64, total: u64, http: u64, context: &str) {
    assert!(
        client >= total,
        "{context}: le budget client a2a ({client}s) est sous l'enveloppe moteur ({total}s) — \
         le client abandonnerait une génération que le moteur a encore le droit de finir \
         (mika#2297, plancher de resolve_timeout_secs)"
    );
    assert!(
        total > http,
        "{context}: l'enveloppe ({total}s) ne contient pas strictement le plafond par appel \
         ({http}s) — ce couple est refusé au démarrage par assert_llm_budgets_valid \
         (mika#2293) et par LlmTimeoutBudget::validate (mika#2189)"
    );
}

/// AC5, première moitié : la chaîne tient sur les défauts.
#[test]
#[serial_test::serial]
fn mika2309_transitivity_holds_on_the_defaults() {
    let _env = EnvScope::capture();

    let (client, total, http) = resolved_chain();
    assert_chain(client, total, http, "défauts");

    // Les trois nombres viennent bien des constantes exportées et non d'un
    // repli silencieux : si un résolveur cessait de lire sa constante, la
    // chaîne pourrait rester ordonnée tout en décrivant une autre géométrie
    // que celle en service.
    assert_eq!(
        client,
        mika_a2a::client::DEFAULT_TIMEOUT.as_secs(),
        "sans env posée, le client doit résoudre son propre DEFAULT_TIMEOUT"
    );
    assert_eq!(
        total,
        mika_common::llm::budget::DEFAULT_AGENT_TOTAL_TIMEOUT_SECS,
        "sans env posée, l'enveloppe doit être DEFAULT_AGENT_TOTAL_TIMEOUT_SECS"
    );
    assert_eq!(
        http, DEFAULT_HTTP_TIMEOUT_SECS,
        "sans env posée, le plafond doit être DEFAULT_HTTP_TIMEOUT_SECS"
    );
}

/// AC5, seconde moitié : la chaîne tient sur une cascade posée par env.
///
/// La géométrie choisie est celle que mika#2189 a réellement donnée à mika-arch
/// (240/900), pour que le cas ne soit pas une combinaison inventée.
#[test]
#[serial_test::serial]
fn mika2309_transitivity_holds_on_an_env_posed_cascade() {
    let env = EnvScope::capture();
    env.set(HTTP_TIMEOUT_ENV_VAR, "240");
    env.set(AGENT_TOTAL_TIMEOUT_ENV_VAR, "900");
    // MIKA_A2A_TIMEOUT_SECS reste absente : c'est le plancher qui doit remonter
    // le client, pas une troisième valeur posée à la main.

    let (client, total, http) = resolved_chain();
    assert_chain(client, total, http, "cascade 240/900 posée par env");

    assert_eq!(http, 240, "le plafond doit suivre l'env posée");
    assert_eq!(total, 900, "l'enveloppe doit suivre l'env posée");
}

/// Ce que le premier `assert` de la chaîne épingle réellement : le `.max()` de
/// `resolve_timeout_secs`.
///
/// Un client explicitement configuré **sous** l'enveloppe doit être remonté. Si
/// quelqu'un retire le plancher, ce cas rougit — c'est la seule façon dont la
/// moitié `client >= total` peut découvrir quoi que ce soit, le reste étant vrai
/// par construction.
#[test]
#[serial_test::serial]
fn mika2309_the_floor_lifts_a_client_below_the_envelope() {
    let env = EnvScope::capture();
    env.set(HTTP_TIMEOUT_ENV_VAR, "120");
    env.set(AGENT_TOTAL_TIMEOUT_ENV_VAR, "900");
    env.set(A2A_TIMEOUT_ENV, "100");

    let (client, total, http) = resolved_chain();
    assert_chain(client, total, http, "client explicitement sous l'enveloppe");

    assert_eq!(
        client, 900,
        "le plancher mika#2297 doit remonter un client configuré à 100s jusqu'à \
         l'enveloppe (900s) — un client sous l'enveloppe abandonne une génération \
         vivante"
    );
}
