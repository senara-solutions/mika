//! Le geste de groom de la relève — `MIKA_MANAGER_GROOM_WRITE`, OFF par défaut
//! (mika#2958, vol 1/2).
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
//! **Il ne porte pas le rail.** Aucune implémentation de [`GroomSender`] n'est
//! livrée ici : au vol 1, `sender` est `None` au site d'appel, ce qui est le cas
//! nominal et se lit comme tel (`reason = "no_sender"`). Le rail A2A, le câblage
//! de cycle, la lecture « un groom est en cours » et le rendu de rapport (AC4,
//! moitié ON) sont le vol 2 — voir le `Tracked in:` du corps de PR de ce vol.
//!
//! **Il n'ouvre pas l'interrupteur.** L'ouverture reste un geste opérateur, au
//! seuil écrit dans le vol 1 de la Phase 2 (c) : les interrupteurs de
//! l'évaluation des vols et du bassin `ready` ouverts, et stables sur un cycle
//! complet. Les deux sont OFF aujourd'hui, et ce vol n'en ouvre aucun.

/// L'interrupteur du point 1 du ticket. **OFF par défaut.**
pub const ENV_GROOM_WRITE: &str = "MIKA_MANAGER_GROOM_WRITE";

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

/// L'émetteur du geste, **injectable** — le rail arrive au vol 2.
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
/// vol 2, qui portera le rail, en dérive les variants du code de statut.
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
}
