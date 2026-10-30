//! Le geste de groom de la relève — `MIKA_MANAGER_GROOM_WRITE`, OFF par défaut
//! (mika#2958, vols 1 et 2a).
//!
//! Ce fichier est la **troisième et dernière** exception nommée de l'allowlist
//! de `no_dispatch_test.rs`, pour l'unique jeton `message/send`. Tout le reste
//! du module reste en lecture seule, et le garde continue de rougir sur tout
//! autre jeton d'écriture — y compris ici, puisque l'exception porte sur un
//! couple `(fichier, jeton)` et jamais sur un fichier entier (propriété tenue
//! par `mika2698_une_exception_ne_couvre_quun_seul_jeton_du_fichier`).
//!
//! ## Cinq propriétés, et pourquoi elles sont écrites comme ça
//!
//! **1. L'interrupteur est fermé par défaut, et son lecteur est unique.**
//! [`groom_write_is_armed`] est le seul endroit du dépôt qui lit
//! `MIKA_MANAGER_GROOM_WRITE`. L'absence de la variable est l'état normal, pas
//! un cas d'erreur : un second lecteur, écrit un peu autrement, serait un second
//! interrupteur (doctrine mika#2201). Sa table de vérité est celle de
//! [`super::pool_writer`], **au prédicat près** : deux interrupteurs du même
//! module qui s'armeraient sur des valeurs différentes seraient un piège
//! d'opérateur, pas une nuance.
//!
//! **2. `armed` est un paramètre de [`trigger_relief_grooms`], pas une lecture
//! d'environnement.** La fonction est alors déterministe, testable en parallèle
//! sans `unsafe { set_var }`, et il reste exactement un lecteur de la variable.
//! La branche OFF est donc vérifiable *par un test de comportement* — « zéro
//! envoi » — et pas seulement par l'absence d'un chemin. Son **contrôle
//! négatif** (`armed == true` ⇒ exactement un envoi) est ce qui empêche « zéro
//! envoi » d'être aussi satisfait par un émetteur mort (classe mika#2205).
//!
//! **3. Le geste est déclaré à plat.** Le vol 1 de mika#2698 a documenté que la
//! forme éclatée (`.method("message").arg("send")`) **échappe** au grep plat du
//! garde. Un émetteur écrit naïvement n'aurait donc besoin d'aucune exception,
//! et l'entrée nommée que le ticket exige serait une fiction : le garde dirait
//! « rien à signaler » sur un fichier qui écrit. [`GESTE`] est la chaîne que le
//! garde cherche, et c'est elle que la sonde `manager_groom_triggered` porte.
//!
//! **4. Un seul groom par cycle, et c'est le moteur qui fixe le nombre.** Voir
//! [`GROOMS_PER_CYCLE`] : le moteur ne tient qu'un créneau `dispatch_class =
//! 'groom'` par agent (schéma v34). Au-delà, le manager compterait comme
//! déclenché ce que le moteur a différé — deux populations sous un nom
//! (mika#2205).
//!
//! **5. Un agent occupé n'est pas une erreur à retenter.** `Err(AgentBusy)`
//! arrête le cycle : le créneau est pris, le candidat suivant échouerait
//! identiquement, et le cycle suivant reprendra la relève telle qu'elle sera
//! alors. Le refus existe déjà côté A2A (`a2a_wait_queue`, mika#2163), donc ce
//! fichier ne l'invente pas, il le transcrit.
//!
//! ## Ce que ce vol ne fait pas
//!
//! **Il porte le rail, mais ne le branche pas.** [`A2aGroomSender`] est livré
//! par le vol 2a, et reste un objet qu'aucun cycle n'instancie : `sender` est
//! `None` au site d'appel, ce qui est le cas nominal et se lit comme tel
//! (`reason = "no_sender"`). Le câblage de cycle, la lecture « un groom est en
//! cours » et le rendu de rapport (AC4, moitié ON) sont les vols 2b et 2c —
//! voir le `Tracked in:` du corps de PR de ce vol.
//!
//! **Il n'ouvre pas l'interrupteur.** L'ouverture reste un geste opérateur, au
//! seuil écrit dans le vol 1 de la Phase 2 (c) : les interrupteurs de
//! l'évaluation des vols et du bassin `ready` ouverts, et stables sur un cycle
//! complet. Les deux sont OFF aujourd'hui, et ce vol n'en ouvre aucun.

use mika_a2a::jsonrpc::AGENT_BUSY;
use std::time::Duration;

/// L'interrupteur du point 1 du ticket. **OFF par défaut.**
pub const ENV_GROOM_WRITE: &str = "MIKA_MANAGER_GROOM_WRITE";

/// L'adresse du rail, **déclarée** — jamais dérivée d'une URL voisine.
///
/// Même forme maison que [`super::ENV_LIVENESS_URL`] (mika#1990 R2) : absente ou
/// vide ⇒ rail désarmé, zéro envoi, zéro erreur ; posée ⇒ rail armé. Le manager
/// connaît déjà trois autres URLs (livraison, escalade, santé) et aucune ne peut
/// servir de source : elles désignent des services différents, et le jour où
/// l'une déménage le rail de groom partirait avec elle sans que personne l'ait
/// demandé.
///
/// Le jeton, lui, n'a **pas** de variable à lui : `/a2a/{agent}` n'accepte que
/// `MIKA_INTERNAL_TOKEN` (`auth::require_internal_token`), donc une seconde
/// déclaration serait un second nom pour un fait unique, divergent au premier
/// roulement de secret. Il arrive en argument de [`resolve_groom_rail`], depuis
/// le site qui le tient déjà.
pub const ENV_GROOM_URL: &str = "MIKA_MANAGER_GROOM_URL";

/// Le geste autorisé, **à plat** — dans la forme même que
/// `no_dispatch_test.rs` grep. Voir le docstring du module, propriété 3 : ce
/// n'est pas une coquetterie, c'est ce qui rend l'exception honnête.
///
/// C'est la méthode A2A que `mika ask --agent mika-dev "groom mika#N"` emprunte
/// déjà : la même porte, jamais un spawn ni un dispatch d'implémentation.
const GESTE: &str = "message/send";

/// Un seul groom par cycle.
///
/// Le nombre n'est pas une prudence : le moteur ne tient qu'**un** créneau
/// `dispatch_class = 'groom'` par agent (schéma v34), donc un second envoi dans
/// le même cycle serait différé par le moteur et compté comme déclenché par le
/// manager — deux populations sous un nom (mika#2205).
pub const GROOMS_PER_CYCLE: usize = 1;

/// Le seul lecteur de [`ENV_GROOM_WRITE`] du dépôt.
pub fn groom_write_is_armed() -> bool {
    armed_from_value(std::env::var(ENV_GROOM_WRITE).ok().as_deref())
}

/// La table de vérité de l'interrupteur, séparée de l'environnement pour être
/// testable sans `unsafe`.
///
/// `None` — la variable absente — est **false**, et c'est le sens de « OFF par
/// défaut ». Toute valeur autre que `1`/`true` est également false : un
/// interrupteur d'écriture ne s'arme pas sur une faute de frappe. Le prédicat
/// est celui de [`super::pool_writer`] mot pour mot (propriété 1 du docstring).
pub(crate) fn armed_from_value(raw: Option<&str>) -> bool {
    match raw {
        Some(v) => {
            let v = v.trim();
            v.eq_ignore_ascii_case("1") || v.eq_ignore_ascii_case("true")
        }
        None => false,
    }
}

/// L'émetteur du geste, **injectable** — le rail est [`A2aGroomSender`].
#[async_trait::async_trait]
pub trait GroomSender: Send + Sync {
    /// `instruction` est le corps du message utilisateur, déjà formé :
    /// `"groom mika#N"`.
    async fn send(&self, instruction: &str) -> Result<(), GroomSendError>;
}

/// Pourquoi un envoi n'a pas eu lieu — **dérivé du statut HTTP**, jamais d'une
/// sous-chaîne de message (mika#2179), même forme que
/// [`super::LivenessFailureClass`].
///
/// Le vol 1 fixe l'énumération et la fige par un test de vocabulaire de fil
/// (mika#1990 D8) : les opérateurs groupent sur [`GroomSendError::classe`]. Le
/// vol 2a en dérive les variants, dans [`classify_send_outcome`] pour les
/// réponses HTTP et dans [`A2aGroomSender::send`] pour les échecs de transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroomSendError {
    /// mika-dev tient déjà un verrou de tour : la file d'attente A2A refuse
    /// (mika#2163).
    AgentBusy,
    /// Le jeton a été refusé (401/403).
    CredentialRefused,
    /// Rien n'a répondu.
    Unreachable,
    Other,
}

impl GroomSendError {
    /// La classe de fil de l'échec — ce que la sonde `manager_groom_failed`
    /// porte en `class`, et ce sur quoi un opérateur groupe.
    ///
    /// `match` exhaustif **sans bras `_`** : un variant neuf ne peut pas
    /// arriver sans que sa classe soit tranchée ici.
    pub fn classe(&self) -> &'static str {
        match self {
            Self::AgentBusy => "agent_busy",
            Self::CredentialRefused | Self::Unreachable | Self::Other => "send_failed",
        }
    }
}

/// Le rail du geste — un POST JSON-RPC `message/send` vers le spirit local.
///
/// Patron copié trait pour trait sur [`super::HttpLivenessSink`], le rail
/// sortant déjà déclaré de ce module : un client `reqwest` construit une fois
/// avec son budget, `client.post(url)` — jamais `Method::POST` ni le littéral,
/// que le scan du module refuse dans tout fichier —, le bearer sur le jeton, et
/// `e.without_url()` au site d'erreur pour que l'URL n'atteigne aucun journal.
///
/// **Il ne lit rien.** `url`, `token` et `budget` sont reçus en arguments de
/// [`A2aGroomSender::new`], et c'est la même ligne de coupe que `deja_groome` et
/// `groom_en_cours` côté candidats : un second lecteur du même fait divergerait
/// du premier le jour où la réponse compte (doctrine mika#2201). Leur résolution
/// depuis l'environnement arrive avec son site d'appel, au câblage de cycle.
pub struct A2aGroomSender {
    client: reqwest::Client,
    url: String,
    token: String,
}

impl A2aGroomSender {
    /// `url`, `token` et `budget` sont **reçus**, jamais lus ici.
    ///
    /// **Le budget doit majorer le budget de tour de mika-dev** (~300 s), et ce
    /// n'est pas une prudence. Le geste est synchrone : si le manager abandonne
    /// la requête avant que le tour ait rendu la main, le tour continue côté
    /// mika-dev, le groom part, et le manager le compte `send_failed` — puis le
    /// relance au cycle suivant. Ni l'une ni l'autre moitié n'est fausse
    /// localement ; ensemble elles produisent un double groom sous un rapport qui
    /// dit « échec ». C'est aussi pourquoi le budget est un **argument** et non
    /// une constante du fichier : celui qui le choisit est celui qui connaît le
    /// budget de tour.
    ///
    /// Un client qui ne se construit pas ne doit pas empêcher un cycle de
    /// **lecture** du manager de tourner, donc `unwrap_or_else` rend le client
    /// par défaut — qui échouera au premier envoi, et dont l'échec sera nommé.
    pub fn new(url: String, token: String, budget: Duration) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(budget)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            url,
            token,
        }
    }
}

/// Le budget du geste de groom, **dérivé** du budget de tour de la flotte.
///
/// `A2aGroomSender::new` reste agnostique de sa valeur — le budget y est un
/// argument, et le paragraphe ci-dessus dit pourquoi. Cette constante est le
/// choix du **résolveur**, c'est-à-dire du seul endroit qui sait vers quoi le
/// rail pointe : un tour de mika-dev.
///
/// Le facteur 2 est ce que ce paragraphe exige : il faut **majorer** le budget
/// de tour, pas l'égaler. À l'égalité, les deux horloges expirent ensemble et la
/// course est garantie plutôt qu'improbable — le manager abandonne, le tour part
/// quand même, le rapport dit « échec » et le cycle suivant relance. Dérivé de
/// [`mika_common::llm::budget::DEFAULT_AGENT_TOTAL_TIMEOUT_SECS`] et non écrit en
/// chiffres, pour que le jour où la flotte change de plafond le rail suive sans
/// qu'on y pense (mika#2189 D3, forme fractionnaire du même module).
pub const GROOM_SEND_BUDGET: Duration =
    Duration::from_secs(2 * mika_common::llm::budget::DEFAULT_AGENT_TOTAL_TIMEOUT_SECS);

/// Résout le rail depuis l'environnement, **au site qui tient le jeton**.
///
/// Trois issues, sur les trois paliers de la forme maison (mika#1990 D4) :
///
/// 1. `MIKA_MANAGER_GROOM_URL` absente, ou vide après `trim` ⇒ `None`. C'est
///    l'**état nominal** d'un conteneur qui n'a pas ouvert ce canal : le cycle le
///    dira `reason = "no_sender"`, et aucune erreur n'est levée. Une absence de
///    rail n'est pas une faute de démarrage — d'où `Option` et jamais `Result`.
/// 2. URL posée, `internal_token` absent ou vide ⇒ `None` également, mais la
///    sonde le dit à **WARN** : c'est une configuration à moitié faite, dont
///    l'effet est identique au cas 1 alors que l'intention ne l'est pas. Le
///    distinguer est tout ce qui sépare « personne n'a branché le rail » de
///    « quelqu'un l'a branché et le serveur n'a pas de jeton » (mika#2205).
/// 3. Les deux présents ⇒ `Some(A2aGroomSender)` armé sur
///    [`GROOM_SEND_BUDGET`].
///
/// **Aucune dérivation.** Le jeton vient de l'appelant, l'URL de sa propre
/// variable, et rien n'est reconstruit depuis une URL voisine : voir
/// [`ENV_GROOM_URL`].
pub fn resolve_groom_rail(internal_token: Option<&str>) -> Option<A2aGroomSender> {
    let url = std::env::var(ENV_GROOM_URL)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let token = internal_token.map(str::trim).filter(|t| !t.is_empty());
    let budget_secs = GROOM_SEND_BUDGET.as_secs();

    // La sonde est émise aux **trois** issues, y compris la nominale : sans la
    // ligne du cas 1, « le rail n'est pas déclaré » et « ce binaire ne sait pas
    // résoudre de rail » rendent les mêmes octets, et son absence doit se lire
    // comme un binaire antérieur à ce vol (motif mika#2340).
    //
    // Ni l'URL ni le jeton n'atteignent la ligne — discipline du module, et
    // promesse du manifeste d'egress pour ce sink. Les deux booléens
    // **déterminent** l'issue, donc aucun champ `reason` ne les redouble : un
    // champ dérivé d'un autre est une valeur de plus à relire, qui peut mentir
    // le jour où l'une des deux dérives. Ce qui sépare le refus du cas nominal
    // est le **niveau**.
    match (url, token) {
        (None, _) => {
            tracing::info!(
                event = "manager_groom_rail_resolved",
                url_set = false,
                token_set = token.is_some(),
                budget_secs,
                "rail de groom non déclaré — le cycle dira no_sender"
            );
            None
        }
        (Some(_), None) => {
            tracing::warn!(
                event = "manager_groom_rail_resolved",
                url_set = true,
                token_set = false,
                budget_secs,
                "rail de groom déclaré sans jeton interne — aucun groom ne partira"
            );
            None
        }
        (Some(url), Some(token)) => {
            tracing::info!(
                event = "manager_groom_rail_resolved",
                url_set = true,
                token_set = true,
                budget_secs,
                "rail de groom armé"
            );
            Some(A2aGroomSender::new(
                url,
                token.to_string(),
                GROOM_SEND_BUDGET,
            ))
        }
    }
}

#[async_trait::async_trait]
impl GroomSender for A2aGroomSender {
    async fn send(&self, instruction: &str) -> Result<(), GroomSendError> {
        let message_id = uuid::Uuid::new_v4().to_string();
        let body = build_send_body(instruction, &message_id);
        // `client.post(url)` et non une construction par `Method::POST` ni par
        // chaîne : `no_dispatch_test.rs` interdit le littéral `"POST"` dans tout
        // fichier du module, et les deux autres formes le porteraient.
        let envoi = self
            .client
            .post(&self.url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await;
        let res = match envoi {
            Ok(r) => r,
            Err(e) => {
                let class = if e.is_connect() || e.is_timeout() {
                    GroomSendError::Unreachable
                } else {
                    GroomSendError::Other
                };
                // Événement **distinct** de `manager_groom_failed`, que
                // [`envoyer`] émet par candidat : celui-là dit « le candidat N
                // n'est pas parti », celui-ci dit « le rail n'a pas pu poster »
                // et porte la seule moitié actionnable que l'autre ne peut pas
                // voir, le message de transport. Deux populations sous un nom ne
                // sont pas soustrayables (mika#2205).
                tracing::warn!(
                    target: "mika::milestone_manager",
                    event = "manager_groom_send_error",
                    class = class.classe(),
                    // `without_url()` et non `{e}` : le `Display` de
                    // `reqwest::Error` appende « for url (…) » à toute erreur qui
                    // en porte une, et le manifeste d'egress affirme que l'URL du
                    // geste n'atteint aucun journal. La classe est déjà calculée
                    // au-dessus, donc la ligne ne perd rien d'actionnable.
                    error = %e.without_url(),
                );
                return Err(class);
            }
        };
        let status = res.status().as_u16();
        // Un corps illisible est `None`, jamais une erreur : c'est le statut qui
        // tranche alors, et la table de [`classify_send_outcome`] le dit.
        let corps = res.json::<serde_json::Value>().await.ok();
        classify_send_outcome(status, corps.as_ref())
    }
}

/// Le corps JSON-RPC du geste — **fonction pure**, donc assertable au champ
/// près.
///
/// Rend l'enveloppe complète : `jsonrpc`, `id`, `method` et `params.message`,
/// dans la forme que `mika_a2a::types::Message` sérialise — `messageId`, `role`,
/// `parts` avec une seule part `{kind: "text", text}`, et `kind: "message"`.
/// C'est le corps que `mika ask --agent mika-dev "groom mika#N"` poste déjà, par
/// la même porte.
///
/// `method` vaut [`GESTE`] et **jamais une chaîne retypée** : c'est la constante
/// à plat que le scan de `no_dispatch_test.rs` grep, et ce rail est son premier
/// lecteur (propriété 3 du docstring de module).
///
/// **`configuration` est délibérément ABSENT du corps, et c'est le piège que
/// cette fonction garde.** Dans `server/a2a.rs`, la branche `returnImmediately`
/// « creates the task row and returns it in `submitted` » et **ne fait jamais
/// tourner la boucle d'agent**. Un manager qui l'emprunterait créerait une tâche
/// qui ne groome rien, pendant que son rapport dirait « groom déclenché ».
/// L'absence du champ n'est donc pas un oubli, c'est le choix, et
/// `mika2958_le_corps_nemet_aucune_configuration` l'assère nommément.
///
/// `message_id` est **reçu** et non tiré d'un `Uuid::new_v4()` interne : c'est ce
/// qui rend le corps entier déterministe, donc comparable dans un test. Patron
/// et raison : `classify_delivery_auth` (`cadence.rs`) — « passing it in rather
/// than reading the environment here keeps the function pure and testable ».
pub(crate) fn build_send_body(instruction: &str, message_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": message_id,
        "method": GESTE,
        "params": {
            "message": {
                "messageId": message_id,
                "role": "user",
                "parts": [{ "kind": "text", "text": instruction }],
                "kind": "message",
            }
        }
    })
}

/// Ce qu'un statut HTTP et un corps de réponse disent du geste — **fonction
/// pure**, donc sa table de lecture est épinglable.
///
/// | fait observé | classe |
/// |---|---|
/// | `error.code == AGENT_BUSY` (−32000) | [`GroomSendError::AgentBusy`] |
/// | `error.code` autre, dont `INTERNAL_ERROR` (−32603) | [`GroomSendError::Other`] |
/// | 2xx, aucun champ `error` | `Ok(())` |
/// | 401 ou 403 | [`GroomSendError::CredentialRefused`] |
/// | autre non-2xx | [`GroomSendError::Other`] |
///
/// Le corps est lu **avant** le statut, parce qu'un refus JSON-RPC ne dégrade pas
/// le statut HTTP : le refus busy arrive en 200 avec un champ `error`.
///
/// **La classification est sur le CODE, jamais sur le message** (mika#2179).
/// `a2a_wait_queue::busy_error` et `legacy_busy_error` portent tous deux le
/// message `"Agent is busy"` ; seul le code les sépare. [`AGENT_BUSY`] est donc
/// importé de `mika_a2a::jsonrpc` et jamais retapé : il n'existe qu'un endroit du
/// dépôt où ce nombre vit.
///
/// **Conséquence nommée, parce qu'elle est une lecture honnête et non un
/// défaut.** Quand la file d'attente A2A est désarmée, le refus busy arrive par
/// `legacy_busy_error()` — code −32603, indistinguable d'une panne interne. Le
/// rail le classe donc `Other`, et le motif au rapport sera `send_failed` et non
/// `agent_busy`. Un instrument qui devinerait la classe par le message mentirait
/// le jour où le message change ;
/// `mika2958_la_file_desarmee_nest_pas_maquillee_en_agent_busy` fige cette
/// lecture plutôt que de la laisser à l'interprétation.
pub(crate) fn classify_send_outcome(
    status: u16,
    body: Option<&serde_json::Value>,
) -> Result<(), GroomSendError> {
    if let Some(code) = body
        .and_then(|b| b.get("error"))
        .and_then(|e| e.get("code"))
        .and_then(serde_json::Value::as_i64)
    {
        return Err(if code == i64::from(AGENT_BUSY) {
            GroomSendError::AgentBusy
        } else {
            GroomSendError::Other
        });
    }
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(GroomSendError::CredentialRefused),
        _ => Err(GroomSendError::Other),
    }
}

/// Un candidat de la relève, avec les deux faits qui décident de son sort.
///
/// Les deux booléens sont des **lectures**, faites par l'appelant : au vol 2,
/// `deja_groome` vient du callout `Plan: docs/plans/` du corps du ticket et
/// `groom_en_cours` du siège `dispatch:` que le moteur y pose, plus le champ de
/// checkpoint qui couvre la fenêtre entre l'envoi et la pose du siège. Ce
/// fichier ne les relit pas : un second lecteur du même fait divergerait du
/// premier le jour où la réponse compte (mika#2201).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroomCandidate {
    pub number: u64,
    pub deja_groome: bool,
    pub groom_en_cours: bool,
}

/// Ce que le cycle a fait de la relève — la donnée que le rapport rendra au
/// vol 2 (AC4, moitié ON).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroomTriggerOutcome {
    /// Les numéros dont le groom a été **effectivement** envoyé.
    pub declenches: Vec<u64>,
    /// `(numéro, motif)` — motif dans le vocabulaire fermé des sondes.
    pub sautes: Vec<(u64, &'static str)>,
}

/// Déclenche le groom de la relève, **borné à [`GROOMS_PER_CYCLE`]**, et rend
/// ce qui a eu lieu.
///
/// Les refus sont **ordonnés et exhaustifs**. Les trois premiers portent sur le
/// cycle entier et vivent dans [`cycle_refusal`] ; les trois autres portent sur
/// un candidat :
///
/// 1. `!armed` → `disarmed`, et **retour immédiat** : `sender` n'est pas
///    touché. C'est la garantie structurelle d'AC1 — il n'existe pas un chemin
///    où `armed == false` et où l'émetteur est appelé.
/// 2. `sender.is_none()` → `no_sender` : le rail n'est pas configuré. Au vol 1,
///    c'est le cas nominal.
/// 3. `candidats.is_empty()` → `no_relief`.
/// 4. par candidat : `deja_groome` → `already_groomed` ; `groom_en_cours` →
///    `groom_in_flight` (AC2 : ni l'un ni l'autre n'est relancé).
/// 5. au-delà de [`GROOMS_PER_CYCLE`] envois réussis → `budget_spent`, pour
///    **tous** les restants.
/// 6. `Err(AgentBusy)` → `agent_busy`, et le cycle s'arrête là ; les autres
///    `Err` → `send_failed`, et le candidat n'entre **pas** dans `declenches`.
pub async fn trigger_relief_grooms(
    armed: bool,
    sender: Option<&dyn GroomSender>,
    candidats: &[GroomCandidate],
) -> GroomTriggerOutcome {
    match (
        cycle_refusal(armed, sender.is_some(), candidats.is_empty()),
        sender,
    ) {
        (Some(reason), _) => {
            // `candidates` est écrit parce qu'une tranche a été reçue, donc la
            // relève a été mesurée. Le cas « non mesurée » n'existe qu'au site
            // d'appel du vol 2, qui devra **omettre** le champ plutôt que
            // l'écrire à `0` (motif mika#2331). L'absence de cette ligne au
            // journal signifie un binaire antérieur à ce vol, pas un cycle sans
            // geste (motif mika#2340).
            tracing::info!(
                target: "mika::milestone_manager",
                event = "manager_groom_not_triggered",
                reason = reason,
                candidates = candidats.len(),
            );
            GroomTriggerOutcome::default()
        }
        (None, Some(sender)) => envoyer(sender, candidats).await,
        // Inatteignable : `cycle_refusal` rend `no_sender` quand l'émetteur
        // manque. Le bras est écrit plutôt que `unreachable!()` pour qu'aucune
        // branche de ce geste ne puisse paniquer un cycle du manager.
        (None, None) => GroomTriggerOutcome::default(),
    }
}

/// Les trois refus **de cycle**, dans l'ordre, ou `None` si le geste a lieu.
///
/// Extrait pour une seule raison : les trois motifs ne voyagent que par la
/// sonde, donc sans ce site pur ils ne seraient épinglables par aucun test —
/// et un vocabulaire de fil non épinglé se renomme en silence.
fn cycle_refusal(armed: bool, sender_present: bool, releve_vide: bool) -> Option<&'static str> {
    if !armed {
        return Some("disarmed");
    }
    if !sender_present {
        return Some("no_sender");
    }
    if releve_vide {
        return Some("no_relief");
    }
    None
}

/// La boucle par candidat — les refus 4 à 6 de [`trigger_relief_grooms`].
async fn envoyer(sender: &dyn GroomSender, candidats: &[GroomCandidate]) -> GroomTriggerOutcome {
    let mut issue = GroomTriggerOutcome::default();
    for candidat in candidats {
        let saute = if candidat.deja_groome {
            Some("already_groomed")
        } else if candidat.groom_en_cours {
            Some("groom_in_flight")
        } else if issue.declenches.len() >= GROOMS_PER_CYCLE {
            Some("budget_spent")
        } else {
            None
        };
        if let Some(reason) = saute {
            tracing::info!(
                target: "mika::milestone_manager",
                event = "manager_groom_skipped",
                issue = candidat.number,
                reason = reason,
            );
            issue.sautes.push((candidat.number, reason));
            continue;
        }

        let instruction = format!("groom mika#{}", candidat.number);
        match sender.send(&instruction).await {
            Ok(()) => {
                tracing::info!(
                    target: "mika::milestone_manager",
                    event = "manager_groom_triggered",
                    issue = candidat.number,
                    geste = GESTE,
                );
                issue.declenches.push(candidat.number);
            }
            Err(e) => {
                tracing::warn!(
                    target: "mika::milestone_manager",
                    event = "manager_groom_failed",
                    issue = candidat.number,
                    class = e.classe(),
                );
                issue.sautes.push((candidat.number, e.classe()));
                if e == GroomSendError::AgentBusy {
                    // Le créneau du moteur est pris : le suivant échouerait
                    // identiquement. Le cycle s'arrête, il ne retente pas.
                    break;
                }
            }
        }
    }
    issue
}

#[cfg(test)]
mod tests {
    use super::*;
    use mika_a2a::jsonrpc::INTERNAL_ERROR;
    use std::sync::Mutex;

    /// Le harnais est **redéclaré ici** plutôt que partagé avec les `mod tests`
    /// des deux autres gestes d'écriture : un `mod tests` n'est pas importable
    /// depuis un module frère sans le rendre visible en production.
    #[derive(Default)]
    struct EmetteurEnregistreur {
        envois: Mutex<Vec<String>>,
        /// Ce que chaque envoi rend — `None` est un succès.
        panne: Option<GroomSendError>,
    }

    impl EmetteurEnregistreur {
        fn en_panne(panne: GroomSendError) -> Self {
            Self {
                envois: Mutex::default(),
                panne: Some(panne),
            }
        }

        fn envois(&self) -> Vec<String> {
            self.envois.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl GroomSender for EmetteurEnregistreur {
        async fn send(&self, instruction: &str) -> Result<(), GroomSendError> {
            self.envois.lock().unwrap().push(instruction.to_string());
            match self.panne {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
    }

    fn propre(number: u64) -> GroomCandidate {
        GroomCandidate {
            number,
            deja_groome: false,
            groom_en_cours: false,
        }
    }

    fn deja_groome(number: u64) -> GroomCandidate {
        GroomCandidate {
            deja_groome: true,
            ..propre(number)
        }
    }

    fn en_vol(number: u64) -> GroomCandidate {
        GroomCandidate {
            groom_en_cours: true,
            ..propre(number)
        }
    }

    fn motifs(issue: &GroomTriggerOutcome) -> Vec<&'static str> {
        issue.sautes.iter().map(|(_, m)| *m).collect()
    }

    /// OFF par défaut, et toute valeur qui n'est pas un oui franc reste OFF.
    ///
    /// La dernière assertion est celle du **lecteur unique** : dans un processus
    /// qui ne pose pas la variable — c'est-à-dire partout, jusqu'à un geste
    /// opérateur — il rend faux. Elle est conditionnelle parce qu'un
    /// environnement de test qui porterait la variable ne met pas le lecteur en
    /// cause, et se taire vaut mieux que rougir à tort.
    #[test]
    fn mika2958_la_table_de_verite_de_linterrupteur() {
        assert!(!armed_from_value(None), "l'absence est OFF");
        assert!(!armed_from_value(Some("")), "le vide est OFF");
        assert!(!armed_from_value(Some("0")));
        assert!(!armed_from_value(Some("false")));
        assert!(!armed_from_value(Some("no")), "une valeur voisine est OFF");
        assert!(armed_from_value(Some("1")));
        assert!(armed_from_value(Some("true")));
        assert!(armed_from_value(Some("TRUE")));
        assert!(armed_from_value(Some("True")));
        assert!(armed_from_value(Some(" 1 ")), "les espaces sont tolérés");
        if std::env::var(ENV_GROOM_WRITE).is_err() {
            assert!(!groom_write_is_armed(), "le lecteur unique est OFF");
        }
    }

    /// **AC1.** Interrupteur fermé : aucun envoi, et son **contrôle négatif**
    /// dans le même test.
    ///
    /// Les deux moitiés sur le même semis, l'interrupteur en seule différence :
    /// sans la seconde, « zéro envoi » serait aussi rendu par un émetteur mort
    /// ou par un harnais qui n'enregistre rien (classe mika#2205).
    #[tokio::test]
    async fn mika2958_off_naucun_envoi_et_son_controle_negatif() {
        let candidats = [propre(2958), propre(2700)];

        let emetteur = EmetteurEnregistreur::default();
        let issue = trigger_relief_grooms(false, Some(&emetteur), &candidats).await;
        assert_eq!(
            issue,
            GroomTriggerOutcome::default(),
            "rien n'est déclenché ni même sauté : le geste n'a pas lieu"
        );
        assert!(
            emetteur.envois().is_empty(),
            "AC1 : zéro envoi avec l'interrupteur OFF, vu : {:?}",
            emetteur.envois()
        );

        let emetteur = EmetteurEnregistreur::default();
        let issue = trigger_relief_grooms(true, Some(&emetteur), &candidats).await;
        assert_eq!(
            emetteur.envois().len(),
            1,
            "contrôle négatif : armé, le même semis envoie — c'est l'interrupteur \
             qui retient, pas un émetteur mort : {:?}",
            emetteur.envois()
        );
        assert_eq!(issue.declenches, vec![2958]);
    }

    /// Le corps envoyé est `groom mika#N`, et le geste nommé est `message/send`.
    ///
    /// Une forme voisine passerait la revue, et l'exception nommée du scan
    /// deviendrait une fiction : c'est cette chaîne-là que le garde cherche.
    #[tokio::test]
    async fn mika2958_on_envoie_le_geste_exact() {
        let emetteur = EmetteurEnregistreur::default();
        let issue = trigger_relief_grooms(true, Some(&emetteur), &[propre(2958)]).await;
        assert_eq!(
            emetteur.envois(),
            vec!["groom mika#2958".to_string()],
            "le corps est celui de `mika ask --agent mika-dev`, au caractère près"
        );
        assert_eq!(issue.declenches, vec![2958]);
        assert!(issue.sautes.is_empty());
        assert_eq!(
            GESTE, "message/send",
            "le geste est déclaré à plat, dans la forme que le garde grep"
        );
    }

    /// **AC3.** Trois candidats propres ⇒ un seul groom, et les deux autres
    /// nommés `budget_spent`.
    #[tokio::test]
    async fn mika2958_un_seul_groom_par_cycle() {
        let emetteur = EmetteurEnregistreur::default();
        let issue =
            trigger_relief_grooms(true, Some(&emetteur), &[propre(10), propre(20), propre(30)])
                .await;
        assert_eq!(
            issue.declenches.len(),
            GROOMS_PER_CYCLE,
            "la borne est `GROOMS_PER_CYCLE`, pas un 1 littéral : {issue:?}"
        );
        assert_eq!(issue.declenches, vec![10]);
        assert_eq!(
            issue.sautes,
            vec![(20, "budget_spent"), (30, "budget_spent")],
            "les restants sont nommés, pas tus"
        );
        assert_eq!(
            emetteur.envois().len(),
            1,
            "le créneau unique du moteur n'est pas saturé : {:?}",
            emetteur.envois()
        );
    }

    /// **AC2.** Un ticket déjà groomé ou dont un groom est en cours n'est pas
    /// relancé — et l'émetteur n'est pas touché pour eux.
    ///
    /// Contrôle positif dans le même test : le candidat propre du **même**
    /// semis part bien, sans quoi un geste muet passerait (mika#2205).
    #[tokio::test]
    async fn mika2958_deja_groome_et_en_vol_ne_sont_pas_relances() {
        let emetteur = EmetteurEnregistreur::default();
        let issue = trigger_relief_grooms(
            true,
            Some(&emetteur),
            &[deja_groome(10), en_vol(20), propre(30)],
        )
        .await;
        assert_eq!(
            issue.sautes,
            vec![(10, "already_groomed"), (20, "groom_in_flight")],
            "les deux motifs d'idempotence, par ticket : {issue:?}"
        );
        assert_eq!(
            emetteur.envois(),
            vec!["groom mika#30".to_string()],
            "seul le candidat propre part : {:?}",
            emetteur.envois()
        );
        assert_eq!(issue.declenches, vec![30]);
    }

    /// **AC3, seconde moitié.** `AgentBusy` arrête le cycle, et ne compte pas
    /// comme déclenché.
    ///
    /// Un groom jamais fait rapporté comme fait ferait disparaître le ticket de
    /// la relève du cycle suivant sans que rien ne l'ait groomé.
    #[tokio::test]
    async fn mika2958_agent_busy_arrete_le_cycle_et_ne_compte_pas_comme_declenche() {
        let emetteur = EmetteurEnregistreur::en_panne(GroomSendError::AgentBusy);
        let issue = trigger_relief_grooms(true, Some(&emetteur), &[propre(10), propre(20)]).await;
        assert!(
            issue.declenches.is_empty(),
            "un envoi refusé n'est pas un groom déclenché : {issue:?}"
        );
        assert_eq!(
            issue.sautes,
            vec![(10, "agent_busy")],
            "le refus est nommé, et le second candidat n'est pas même tenté"
        );
        assert_eq!(
            emetteur.envois().len(),
            1,
            "le cycle s'arrête au créneau pris : il ne retente pas : {:?}",
            emetteur.envois()
        );

        // Contrôle négatif du `break` : un échec qui n'est PAS `AgentBusy` ne
        // ferme pas le cycle — sinon « le cycle s'arrête » serait vrai de tout
        // échec, et le `match` sur le variant ne prouverait rien.
        let emetteur = EmetteurEnregistreur::en_panne(GroomSendError::Unreachable);
        let issue = trigger_relief_grooms(true, Some(&emetteur), &[propre(10), propre(20)]).await;
        assert_eq!(
            issue.sautes,
            vec![(10, "send_failed"), (20, "send_failed")],
            "un rail injoignable laisse le cycle tenter les suivants : {issue:?}"
        );
        assert!(issue.declenches.is_empty());
    }

    /// Les huit motifs, **à la chaîne près** : c'est un format de fil.
    ///
    /// Les chaînes sont retapées ici plutôt que lues depuis une constante
    /// partagée, et c'est le point : un test qui lirait la même constante que la
    /// production passerait au premier renommage, en laissant les `GROUP BY` des
    /// opérateurs muets (mika#1990 D8).
    ///
    /// Le plan de ce vol en comptait sept ; il y en a huit — `send_failed`
    /// manquait au décompte, pas à l'énumération (§ 4.1 point 6 et § 4.4 le
    /// nomment tous les deux).
    #[tokio::test]
    async fn mika2958_le_vocabulaire_des_motifs_est_un_format_de_fil() {
        // Les trois refus de cycle, qui ne voyagent que par la sonde.
        assert_eq!(cycle_refusal(false, true, false), Some("disarmed"));
        assert_eq!(cycle_refusal(true, false, false), Some("no_sender"));
        assert_eq!(cycle_refusal(true, true, true), Some("no_relief"));
        assert_eq!(
            cycle_refusal(true, true, false),
            None,
            "aucun refus de cycle quand les trois conditions sont tenues"
        );
        // L'ordre compte : désarmé passe avant l'absence de rail, qui passe
        // avant une relève vide. Sans lui, un cycle OFF sans rail se lirait
        // `no_sender` et l'AC1 serait illisible au journal.
        assert_eq!(cycle_refusal(false, false, true), Some("disarmed"));

        // Les deux classes d'échec, par variant.
        assert_eq!(GroomSendError::AgentBusy.classe(), "agent_busy");
        for e in [
            GroomSendError::CredentialRefused,
            GroomSendError::Unreachable,
            GroomSendError::Other,
        ] {
            assert_eq!(e.classe(), "send_failed", "{e:?}");
        }

        // Les trois motifs de saut, par le comportement qui les produit.
        let emetteur = EmetteurEnregistreur::default();
        let issue = trigger_relief_grooms(
            true,
            Some(&emetteur),
            &[deja_groome(10), en_vol(20), propre(30), propre(40)],
        )
        .await;
        assert_eq!(
            motifs(&issue),
            vec!["already_groomed", "groom_in_flight", "budget_spent"],
        );

        // Et le vocabulaire est **fermé** : rien d'autre ne sort de ce geste.
        const VOCABULAIRE: &[&str] = &[
            "disarmed",
            "no_sender",
            "no_relief",
            "already_groomed",
            "groom_in_flight",
            "budget_spent",
            "agent_busy",
            "send_failed",
        ];
        assert_eq!(VOCABULAIRE.len(), 8);
        for motif in motifs(&issue) {
            assert!(
                VOCABULAIRE.contains(&motif),
                "motif hors vocabulaire : {motif}"
            );
        }
    }

    /// Le corps du geste, **au champ près** — il est déterministe, donc on le
    /// compare entier.
    ///
    /// Trois propriétés dans la même assertion, et c'est voulu : `method` vaut
    /// [`GESTE`] (la chaîne que le garde grep, pas une voisine), la seule part
    /// porte l'instruction telle quelle, et `messageId` vaut l'argument reçu —
    /// sans quoi la comparaison du corps entier serait impossible, et c'est
    /// précisément ce que `message_id` en paramètre achète.
    #[test]
    fn mika2958_le_corps_du_geste_nomme_le_geste_et_porte_linstruction() {
        let body = build_send_body("groom mika#2958", "msg-42");
        assert_eq!(
            body,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": "msg-42",
                "method": "message/send",
                "params": {
                    "message": {
                        "messageId": "msg-42",
                        "role": "user",
                        "parts": [{ "kind": "text", "text": "groom mika#2958" }],
                        "kind": "message",
                    }
                }
            }),
            "le corps est celui de `mika ask --agent mika-dev`, au caractère près"
        );
        assert_eq!(
            body["method"], GESTE,
            "le geste vient de la constante à plat, pas d'une chaîne retypée"
        );
    }

    /// **Le test du piège.** `params.configuration` est absent, et son absence
    /// est le choix.
    ///
    /// `server/a2a.rs` : la branche `returnImmediately` crée la ligne de tâche et
    /// la rend en `submitted` **sans jamais faire tourner la boucle d'agent**. Un
    /// manager qui l'emprunterait créerait une tâche qui ne groome rien pendant
    /// que son rapport dirait « groom déclenché » — un groom jamais fait,
    /// rapporté comme fait (mika#2958).
    #[test]
    fn mika2958_le_corps_nemet_aucune_configuration() {
        let body = build_send_body("groom mika#10", "msg-1");
        assert!(
            body["params"].get("configuration").is_none(),
            "aucune `configuration` : la branche `returnImmediately` ne fait pas \
             tourner la boucle d'agent, donc elle ne groome rien : {body}"
        );
    }

    /// La table de lecture de [`classify_send_outcome`], dans son ordre.
    ///
    /// Le corps est consulté avant le statut parce qu'un refus JSON-RPC n'abaisse
    /// pas le statut HTTP : le refus busy arrive en **200** avec un champ
    /// `error`. Un classement qui ne lirait que le statut appellerait ce refus un
    /// succès.
    #[test]
    fn mika2958_la_table_de_classification_des_reponses() {
        assert_eq!(
            classify_send_outcome(
                200,
                Some(&serde_json::json!({ "result": { "kind": "task" } }))
            ),
            Ok(()),
            "2xx sans champ `error` est le succès"
        );
        assert_eq!(
            classify_send_outcome(200, None),
            Ok(()),
            "un 2xx dont le corps n'a pas pu être lu reste un succès"
        );

        let busy = serde_json::json!({
            "error": { "code": AGENT_BUSY, "message": "Agent is busy" }
        });
        assert_eq!(
            classify_send_outcome(200, Some(&busy)),
            Err(GroomSendError::AgentBusy),
            "le refus busy arrive en 200 avec un code, et c'est le code qui le dit"
        );
        assert_eq!(GroomSendError::AgentBusy.classe(), "agent_busy");

        assert_eq!(
            classify_send_outcome(401, None),
            Err(GroomSendError::CredentialRefused),
            "401 : le jeton interne a été refusé"
        );
        assert_eq!(
            classify_send_outcome(403, None),
            Err(GroomSendError::CredentialRefused),
            "403 : idem, patron de `liveness.rs`"
        );
        assert_eq!(
            classify_send_outcome(500, None),
            Err(GroomSendError::Other),
            "un non-2xx sans corps lisible reste nommé, pas deviné"
        );
    }

    /// **La lecture honnête**, figée plutôt que laissée à l'interprétation.
    ///
    /// Quand la file d'attente A2A est désarmée, le refus busy arrive par
    /// `legacy_busy_error()` — code −32603, **même message** `"Agent is busy"`
    /// que `busy_error()`, et indistinguable d'une panne interne. Le rail le
    /// classe donc `Other` et le rapport dira `send_failed`, pas `agent_busy`.
    ///
    /// Les deux corps de ce test ne diffèrent que par le code, message compris :
    /// c'est ce qui prouve que la classification est sur le champ structuré et
    /// non sur une sous-chaîne (mika#2179). Un instrument qui devinerait par le
    /// message mentirait le jour où le message change.
    #[test]
    fn mika2958_la_file_desarmee_nest_pas_maquillee_en_agent_busy() {
        let legacy = serde_json::json!({
            "error": { "code": INTERNAL_ERROR, "message": "Agent is busy" }
        });
        assert_eq!(
            classify_send_outcome(200, Some(&legacy)),
            Err(GroomSendError::Other),
            "−32603 n'est pas maquillé en `agent_busy` : le message est le même, \
             seul le code sépare les deux refus"
        );
        assert_eq!(
            GroomSendError::Other.classe(),
            "send_failed",
            "et le motif au rapport dit `send_failed`, ce qui est la vérité : \
             le rail ne peut pas savoir que la file était désarmée"
        );
        assert_ne!(
            AGENT_BUSY, INTERNAL_ERROR,
            "les deux codes sont bien distincts, sans quoi ce test ne prouverait rien"
        );
    }

    /// Propriété 11 : le rail **reçoit** son adresse et son jeton, il ne les
    /// lit pas.
    ///
    /// C'est le contrôle que rien n'est résolu depuis l'environnement dans ce
    /// fichier : si une variable `MIKA_MANAGER_GROOM_URL` apparaissait un jour
    /// sous [`A2aGroomSender::new`], les deux assertions ci-dessous resteraient
    /// vraies — mais elles sont ici pour que le **jour où** quelqu'un remplace
    /// un argument par une lecture, le test dise où la ligne de coupe passait.
    /// Le budget est passé et non assert : il vit dans le client `reqwest`, qui
    /// n'expose pas son `timeout`. C'est aussi pourquoi le site d'appel, au vol
    /// 2b, est le seul endroit où le budget de tour de mika-dev se compare au
    /// budget du rail.
    #[test]
    fn mika2958_le_rail_recoit_son_adresse_et_son_jeton() {
        let rail = A2aGroomSender::new(
            "http://127.0.0.1:8080/a2a/mika-dev".to_string(),
            "jeton-de-test".to_string(),
            Duration::from_secs(420),
        );
        assert_eq!(
            rail.url, "http://127.0.0.1:8080/a2a/mika-dev",
            "l'URL est celle reçue, pas une URL par défaut ni une lecture d'environnement"
        );
        assert_eq!(
            rail.token, "jeton-de-test",
            "le jeton est celui reçu — ce fichier ne connaît aucun nom de variable de jeton"
        );
    }

    /// Propriété 12, **contrôle négatif de non-régression (AC1)** : le rail est
    /// présent, l'interrupteur est à `false`, et il ne part rien.
    ///
    /// C'est la forme qui distingue « désarmé » de « poseur mort ». Les tests
    /// de refus de cycle du vol 1 passaient `None` ou un double : avec un vrai
    /// `A2aGroomSender` au bout, un refus qui aurait régressé émettrait une
    /// requête HTTP réelle.
    ///
    /// **Le test ne dépend pas de l'adresse** — le refus de cycle précède toute
    /// construction de requête, et c'est précisément ce qu'il assère. L'adresse
    /// non routable et le budget court ne sont là que pour que la régression
    /// rougisse **vite** au lieu de pendre : si le refus tombait, l'envoi
    /// échouerait sur un refus de connexion local en quelques millisecondes, et
    /// l'outcome porterait `send_failed` au lieu d'être vide.
    #[tokio::test]
    async fn mika2958_le_rail_arme_a_false_nemet_aucune_requete() {
        let rail = A2aGroomSender::new(
            "http://127.0.0.1:1/a2a/mika-dev".to_string(),
            "jeton-de-test".to_string(),
            Duration::from_millis(50),
        );
        let candidats = vec![
            GroomCandidate {
                number: 2958,
                deja_groome: false,
                groom_en_cours: false,
            },
            GroomCandidate {
                number: 2959,
                deja_groome: false,
                groom_en_cours: false,
            },
        ];
        let outcome = trigger_relief_grooms(false, Some(&rail), &candidats).await;
        assert_eq!(
            outcome,
            GroomTriggerOutcome::default(),
            "interrupteur OFF : rien n'est déclenché et rien n'est sauté — \
             un `sautes` non vide voudrait dire que le cycle est entré dans la boucle"
        );
    }

    /// Pose [`ENV_GROOM_URL`] le temps d'un test et la retire à la sortie,
    /// **même en cas de panique** — patron d'`InterrupteurArme` (`cadence.rs`).
    struct AdressePosee;

    impl AdressePosee {
        fn poser(valeur: &str) -> Self {
            // SAFETY: single-threaded via serial_test.
            unsafe { std::env::set_var(ENV_GROOM_URL, valeur) };
            Self
        }

        /// L'absence est un état à poser explicitement : un test précédent peut
        /// avoir laissé la variable, et un environnement de CI peut la porter.
        fn retirer() -> Self {
            // SAFETY: single-threaded via serial_test.
            unsafe { std::env::remove_var(ENV_GROOM_URL) };
            Self
        }
    }

    impl Drop for AdressePosee {
        fn drop(&mut self) {
            // SAFETY: single-threaded via serial_test.
            unsafe { std::env::remove_var(ENV_GROOM_URL) };
        }
    }

    /// Le budget du rail **majore** le budget de tour, et il le majore par
    /// dérivation.
    ///
    /// Les deux moitiés comptent. L'égalité serait une course garantie (voir le
    /// docstring de [`GROOM_SEND_BUDGET`]), et une valeur écrite en chiffres
    /// cesserait de suivre le plafond de la flotte le jour où il bouge — le test
    /// compare donc à la constante, jamais à `600`.
    #[test]
    fn mika2958_le_budget_du_rail_majore_le_budget_de_tour() {
        let tour = mika_common::llm::budget::DEFAULT_AGENT_TOTAL_TIMEOUT_SECS;
        assert!(
            GROOM_SEND_BUDGET.as_secs() > tour,
            "le budget du geste ({} s) doit MAJORER le budget de tour ({tour} s) : \
             à l'égalité, les deux horloges expirent ensemble et le double groom \
             devient la règle plutôt que l'exception",
            GROOM_SEND_BUDGET.as_secs()
        );
        assert_eq!(
            GROOM_SEND_BUDGET.as_secs(),
            2 * tour,
            "le budget est dérivé du plafond de la flotte, pas écrit en chiffres"
        );
    }

    /// Les trois issues de la résolution, dans un seul test parce que c'est leur
    /// **contraste** qui est la propriété.
    ///
    /// Le premier cas est aussi le contrôle négatif du troisième : l'URL absente
    /// avec un jeton présent rend `None`, ce qui est ce qui prouve qu'aucune URL
    /// n'est dérivée d'une voisine ni d'un défaut de code. Si une dérivation
    /// existait, ce cas rendrait `Some` et le reste du test resterait vert.
    #[test]
    #[serial_test::serial]
    fn mika2958_la_resolution_du_rail_a_trois_issues() {
        {
            let _absente = AdressePosee::retirer();
            assert!(
                resolve_groom_rail(Some("jeton-de-test")).is_none(),
                "URL absente ⇒ rail désarmé, et c'est l'état NOMINAL : aucune \
                 adresse n'est dérivée des trois autres URLs du manager"
            );
        }
        {
            let _vide = AdressePosee::poser("   ");
            assert!(
                resolve_groom_rail(Some("jeton-de-test")).is_none(),
                "URL vide après trim ⇒ désarmé, troisième palier de la forme maison"
            );
        }
        {
            let _posee = AdressePosee::poser("http://127.0.0.1:8080/a2a/mika-dev");
            assert!(
                resolve_groom_rail(None).is_none(),
                "URL posée sans jeton ⇒ désarmé : le rail ne s'invente pas de jeton"
            );
            assert!(
                resolve_groom_rail(Some("  ")).is_none(),
                "un jeton vide n'est pas un jeton — même palier que l'URL vide"
            );
            let rail =
                resolve_groom_rail(Some("jeton-de-test")).expect("URL et jeton posés ⇒ rail armé");
            assert_eq!(
                rail.url, "http://127.0.0.1:8080/a2a/mika-dev",
                "l'adresse du rail est celle déclarée, à l'octet"
            );
            assert_eq!(
                rail.token, "jeton-de-test",
                "le jeton du rail est celui reçu en argument"
            );
        }
    }

    /// L'adresse est `trim`ée avant d'atteindre le rail.
    ///
    /// Une variable d'environnement posée dans un `.env` traîne régulièrement un
    /// espace ou un retour de ligne. Sans `trim`, le POST partirait vers une URL
    /// invalide et le cycle lirait `send_failed` — un échec de transport là où le
    /// fait est une faute de frappe.
    #[test]
    #[serial_test::serial]
    fn mika2958_ladresse_du_rail_est_trimee() {
        let _posee = AdressePosee::poser("  http://127.0.0.1:8080/a2a/mika-dev\n");
        let rail = resolve_groom_rail(Some("jeton-de-test")).expect("rail armé");
        assert_eq!(rail.url, "http://127.0.0.1:8080/a2a/mika-dev");
    }

    /// Le nom de la variable est un **format de fil** : il est écrit dans
    /// `.env.example`, dans le manifeste d'egress et dans les recettes de
    /// l'opérateur. Le renommer est un geste d'opérateur, pas un refactor.
    #[test]
    fn mika2958_le_nom_de_la_variable_dadresse_est_un_format_de_fil() {
        assert_eq!(ENV_GROOM_URL, "MIKA_MANAGER_GROOM_URL");
    }
}
