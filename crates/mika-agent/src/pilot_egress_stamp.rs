//! Le lecteur — unique — du marqueur de panne du relais d'egress (mika#2049).
//!
//! # Ce que ce module N'EST PAS : la protection
//!
//! Le lot mika#2049 porte quatre gardes, et **une seule protège** :
//!
//! | | Où | Rôle | Lit un état persistant ? | Fail-* |
//! |---|---|---|---|---|
//! | **C** | `dispatch-lib::_run_pilot_sandboxed` | **la protection** — refuse le lancement | **non**, sonde à chaque fois | fail-**closed** |
//! | **B** | `auto_pull` Phase 2 | économie — ne consomme pas de budget de re-drive | oui (ce module) | fail-**open** |
//! | **A** | `ready_label_handler` | économie — ne crée ni tâche ni différé | oui (ce module) | fail-**open** |
//! | **D** | `self-dev-callback` | honnêteté — le refus n'est pas rapporté comme un succès | non | — |
//!
//! **A et B sont des optimisations de confort opérateur, faillibles et
//! fail-open ; C est la protection, inconditionnelle, et ne lit aucun état
//! persistant.** Un futur lecteur tenté de durcir A ou B au motif qu'elles sont
//! fail-open doit savoir que la sûreté ne repose pas sur elles : elle repose sur
//! C, qui sonde le socket à chaque dispatch. Inversement, quiconque
//! affaiblirait C en lui faisant lire ce marqueur transformerait la protection
//! en cache — et **un cache périmé est un fail-open avec une étape de plus**.
//!
//! # Ce que A et B achètent : la reprise sans geste sur les tickets (R5)
//!
//! La garde C seule satisfait la sûreté et **casse** le test négatif de
//! l'opérateur (« proxy relancé ⇒ le dispatch reprend *sans intervention sur les
//! tickets* »). L'arithmétique, sans A ni B :
//!
//! 1. le ticket promu `ready` est dispatché, le sandbox refuse ;
//! 2. le ticket **garde `ready`** et n'est pas dispatché ;
//! 3. `auto_pull` Phase 2 le voit `ready` depuis plus que le seuil de
//!    stuck-ready et le re-drive — `redrive_count` passe à 1 ;
//! 4. à `MIKA_AUTO_PULL_MAX_REDRIVES` (défaut 3), Phase 2 **abandonne** le
//!    ticket : `operator-review` posé, `ready` retiré, commentaire (mika#2020).
//!
//! Une panne de relais d'environ une heure parquerait donc chaque ticket `ready`
//! derrière un label opérateur, et la reprise exigerait un geste par ticket.
//!
//! # Un seul sondeur, un seul lecteur
//!
//! Le **sondeur** est la garde C, en shell. A et B **lisent ce marqueur et ne
//! sondent jamais** : dupliquer la sonde en Rust créerait un second lecteur de
//! la même question, ce que la maison a dû défaire une fois
//! ([`crate::grooming_marker`], mika#2158). Et le chemin du marqueur vit **ici
//! et nulle part ailleurs**, pour la raison que [`crate::auto_pull_stop`] a dû
//! écrire : une copie ne rendrait aucune décision fausse le jour où elle est
//! écrite, donc aucun test comportemental ne peut la voir — elle divergerait
//! plus tard, en silence.
//!
//! # La frontière que ce marqueur traverse, et le piège qui s'y trouve
//!
//! **Le producteur est le shell, le consommateur est le moteur.** C'est le
//! premier fichier de `state/` dans ce sens : `pilot-gitconfig` et
//! `pr-origin-epoch` sont shell des deux côtés, et le marqueur d'arrêt de
//! mika#2329 est posé à la main par l'opérateur et lu par le seul Rust.
//!
//! `scrub_mika_env_vars` ([`crate::skills::executor`]) retire de l'enfant de
//! dispatch **toute** variable commençant par `MIKA_`, `MIKA_HOME` comprise.
//! Côté moteur, la résolution du home est `$MIKA_HOME > ~/.mika`. Donc :
//!
//! | | Process | Résout |
//! |---|---|---|
//! | Garde C (écrit) | enfant de dispatch, **scrubé** | `$HOME/.mika/state/…` |
//! | Gardes A et B (lisent) | mika-spirit | `global_home/state/…` |
//!
//! Sur une installation qui pose `MIKA_HOME`, ces deux chemins divergent. **La
//! protection tiendrait** — la garde C ne lit aucun marqueur — mais A et B,
//! fail-open par construction, liraient « pas de panne » et laisseraient passer
//! chaque dispatch : R5 faux en production avec une suite de tests verte,
//! puisqu'un harnais pose le même home des deux côtés ou n'en pose aucun.
//!
//! D'où l'invariant, écrit **aux deux extrémités** : le shell écrit
//! `$HOME/.mika` en dur et n'a pas le droit d'employer `${MIKA_HOME:-…}` ; le
//! Rust lit depuis `global_home_dir`. Tenu par un scan de source dans
//! `skills/bundled/_shared/test-dispatch-lib.sh` — un test comportemental ne
//! peut pas l'attraper.
//!
//! Le mode de panne est **muet du côté rassurant** : une divergence de chemin ne
//! peut acheter que de l'inertie, jamais un faux positif. C'est l'asymétrie
//! exacte que mika#2249 a dû écrire pour `MIKA_PILOT_LOG_DIR` / `PILOT_LOG_DIR`,
//! et c'est ce qui rend ce défaut supportable mais invisible.
//!
//! # La péremption est porteuse, pas un réglage
//!
//! Si A refusait sur marqueur présent sans jamais re-sonder, **personne ne
//! sonde**, le marqueur ne se lève jamais et la boucle est bloquée
//! définitivement — le mode de panne classique d'un disjoncteur sans
//! ré-armement. Le marqueur est donc réputé **périmé** au-delà de
//! [`DOWN_TTL_ENV`] : passé ce délai A laisse passer, la garde C re-sonde, et
//! soit elle réussit (marqueur retiré, reprise annoncée) soit elle refuse
//! (marqueur rafraîchi, **pas** de nouvelle alerte). Le pire cas pendant une
//! panne est une tentative de dispatch par TTL écoulé.
//!
//! **Alternative écartée** : rendre `Skip` à la garde B sur marqueur présent
//! *même périmé*. Elle ferme le tableau ci-dessus mais rend le déblocage
//! dépendant d'un dispatch qui ne viendra jamais — A et B bloquant tous les
//! chemins, plus rien ne re-sonde. C'est le blocage définitif réintroduit par
//! l'autre bout.

use std::path::{Path, PathBuf};

/// Répertoire d'état, sous le home **global** — même emplacement et même
/// raisonnement que [`crate::auto_pull_stop`].
const STAMP_DIR: &str = "state";

/// Nom du fichier marqueur, écrit par `dispatch-lib.sh`.
///
/// **Occurrence unique sous `crates/`**, tenue par le scan de source côté
/// shell. Ne pas le réécrire ailleurs, doc-comment compris : la garde est
/// littérale, exactement comme celle de mika#2329.
const STAMP_FILE: &str = "pilot-egress-down";

/// Le TTL au-delà duquel le marqueur est réputé périmé.
///
/// # La relation avec le seuil de stuck-ready est PORTEUSE
///
/// Ce défaut **doit rester strictement supérieur** à
/// `MIKA_AUTO_PULL_STUCK_READY_THRESHOLD_SECS` (défaut 900 s, constante
/// `STUCK_READY_THRESHOLD_ENV` dans [`crate::auto_pull`]), avec marge. Le nombre
/// qui gouverne n'est **pas** la cadence du tick d'`auto_pull` (600 s) mais
/// l'âge que le label `ready` doit atteindre pour que Phase 2 re-drive.
///
/// Une rédaction antérieure du plan posait 600 s « soit un tick d'`auto_pull` »,
/// et ce dimensionnement **annulait la garde B**. Phase 2 re-drive par `remove`
/// → `add`, ce qui remet l'âge du label à zéro, donc :
///
/// | t | événement | `redrive_count` |
/// |---|---|---|
/// | 0 | panne ; dispatch refusé ; marqueur écrit | 0 |
/// | 900 | âge label = 900 ≥ seuil ; marqueur vieux de 900 > 600 ⇒ **périmé, B laisse passer** ; re-drive ; C refuse ; marqueur rafraîchi | 1 |
/// | 1800 | idem | 2 |
/// | 2700 | idem | 3 |
/// | 3600 | budget épuisé ⇒ **abandon : `operator-review` posé, `ready` retiré** | — |
///
/// La garde, à ce dimensionnement, ne change pas une ligne du calcul : elle
/// n'est jamais consultée avec un marqueur frais sur le seul chemin qu'elle
/// existe pour couvrir.
///
/// Le couplage est écrit **ici, là où il se lit**, faute de quoi quelqu'un
/// baissera l'un des deux nombres et rouvrira ce trou en silence. Précédent
/// maison exact : mika#2362, où une enveloppe multiple exact du plafond rendait
/// la dernière tentative nominale **inatteignable** sans qu'aucun test ne
/// rougisse, parce que la relation entre les deux nombres n'était écrite nulle
/// part. Tenu par `auto_pull::tests::mika2049_le_ttl_du_marqueur_depasse_le_seuil_de_stuck_ready`.
///
/// # Ce que le défaut coûte, nommé
///
/// Le TTL borne la latence de reprise : le marqueur n'est retiré que par un
/// dispatch réussi, donc après le retour du relais la boucle repart en **au plus
/// 30 min** au lieu de 10. C'est conforme au test négatif de l'opérateur, qui
/// exige une reprise *sans intervention sur les tickets* — jamais une reprise
/// instantanée — et c'est le bon côté de l'arbitrage : trente minutes d'attente
/// contre un ticket parqué qui, lui, exige un geste humain.
pub const DOWN_TTL_DEFAULT_SECS: i64 = 1800;

/// La variable d'environnement qui règle [`DOWN_TTL_DEFAULT_SECS`].
pub const DOWN_TTL_ENV: &str = "MIKA_PILOT_EGRESS_DOWN_TTL_SECS";

/// `{global_home}/state/pilot-egress-down`.
///
/// **Ne recomposez pas ce chemin ailleurs.** Voir la section « Un seul sondeur,
/// un seul lecteur » de la doc de module.
pub fn stamp_path(global_home: &Path) -> PathBuf {
    global_home.join(STAMP_DIR).join(STAMP_FILE)
}

/// Ce que le marqueur dit du relais, du point de vue des gardes A et B.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayVerdict {
    /// Aucune panne fraîche connue. **C'est aussi la réponse à toute lecture
    /// impossible** — voir [`relay_verdict`].
    Serving,
    /// Une panne a été constatée par la garde C il y a moins d'un TTL.
    Down {
        /// Le motif structuré posé par le shell (`egress_binary_missing`,
        /// `egress_bind_timeout`). Rapporté, jamais interprété : les gardes A et
        /// B ne décident rien sur sa valeur, elles le journalisent pour que
        /// l'opérateur sache quel organe réparer.
        motif: String,
        /// Âge du marqueur en secondes, pour le journal.
        age_secs: i64,
    },
}

impl RelayVerdict {
    /// Le relais est-il en panne fraîche ?
    ///
    /// Existe pour que personne n'écrive `!matches!(v, Serving)` — la seule
    /// inversion qui transformerait une lecture impossible en refus, c'est-à-dire
    /// qui gèlerait la boucle sur une supposition. Même geste que
    /// `LivePilotVerdict::is_alive` (mika#2279).
    pub fn is_down(&self) -> bool {
        matches!(self, Self::Down { .. })
    }
}

/// Le TTL en vigueur. Trois paliers maison : absent/vide → défaut ; illisible,
/// `0` ou négatif → défaut avec un WARN.
///
/// **`0` ne désarme pas.** Un TTL de zéro rendrait le marqueur périmé à
/// l'instant même où il est écrit, ce qui restaurerait exactement le tableau de
/// [`DOWN_TTL_DEFAULT_SECS`] — un désarmement silencieux déguisé en réglage.
pub fn ttl_secs() -> i64 {
    parse_ttl(std::env::var(DOWN_TTL_ENV).ok().as_deref())
}

fn parse_ttl(raw: Option<&str>) -> i64 {
    match raw.map(str::trim) {
        Some(v) if !v.is_empty() => match v.parse::<i64>() {
            Ok(n) if n > 0 => n,
            _ => {
                tracing::warn!(
                    event = "pilot_egress_ttl_invalid",
                    value = %v,
                    default = DOWN_TTL_DEFAULT_SECS,
                    "pilot_egress_stamp: invalid {DOWN_TTL_ENV}, using default"
                );
                DOWN_TTL_DEFAULT_SECS
            }
        },
        _ => DOWN_TTL_DEFAULT_SECS,
    }
}

/// Lire le marqueur.
///
/// # Fail-open, et c'est sûr **ici précisément parce que la garde C existe**
///
/// Marqueur absent, illisible, vide, inparsable, horodatage dans le futur,
/// horodatage plus vieux que le TTL ⇒ [`RelayVerdict::Serving`] ⇒ le dispatch
/// est tenté ⇒ la garde C tranche sur une sonde fraîche. **Aucune de ces
/// lectures ne peut ouvrir le réseau** : le pire qu'un faux `Serving` achète est
/// un aller-retour de dispatch refusé.
///
/// L'horodatage futur (dérive d'horloge) est traité comme illisible plutôt que
/// comme « très frais » : un marqueur daté de demain gèlerait les gardes A et B
/// jusqu'à demain, ce qui est la seule façon dont ce lecteur pourrait coûter
/// quelque chose.
pub fn relay_verdict(global_home: &Path, ttl_secs: i64) -> RelayVerdict {
    let path = stamp_path(global_home);
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return RelayVerdict::Serving;
    };
    parse_stamp(&contents, ttl_secs, chrono::Utc::now())
}

/// La moitié pure de [`relay_verdict`] — séparée pour que le fail-open soit
/// testable sans horloge ni système de fichiers.
fn parse_stamp(contents: &str, ttl_secs: i64, now: chrono::DateTime<chrono::Utc>) -> RelayVerdict {
    let line = contents.lines().next().unwrap_or("").trim();
    let mut parts = line.splitn(2, char::is_whitespace);
    let Some(stamped_at) = parts.next().filter(|s| !s.is_empty()) else {
        return RelayVerdict::Serving;
    };
    // Un motif absent n'invalide pas le marqueur : la panne est le fait, le
    // motif est le confort. Refuser de lire un marqueur dont le motif manque
    // rendrait les gardes A et B inertes pour une raison cosmétique.
    let motif = parts.next().unwrap_or("").trim();
    let motif = if motif.is_empty() {
        "egress_unavailable"
    } else {
        motif
    };

    let Ok(stamped_at) = crate::timestamp::parse(stamped_at) else {
        return RelayVerdict::Serving;
    };

    let age_secs = (now - stamped_at).num_seconds();
    if age_secs < 0 || age_secs > ttl_secs {
        return RelayVerdict::Serving;
    }

    RelayVerdict::Down {
        motif: motif.to_string(),
        age_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn at(age_secs: i64) -> String {
        format!(
            "{} egress_bind_timeout\n",
            crate::timestamp::format(&(Utc::now() - Duration::seconds(age_secs)))
        )
    }

    /// Un marqueur frais est lu comme une panne, motif compris.
    #[test]
    fn mika2049_un_marqueur_frais_est_une_panne() {
        let verdict = parse_stamp(&at(60), DOWN_TTL_DEFAULT_SECS, Utc::now());
        assert!(verdict.is_down(), "un marqueur de 60 s doit être frais");
        match verdict {
            RelayVerdict::Down { motif, age_secs } => {
                assert_eq!(motif, "egress_bind_timeout");
                assert!(
                    (55..=65).contains(&age_secs),
                    "l'âge rapporté doit être celui du marqueur, pas une constante : {age_secs}"
                );
            }
            other => panic!("attendu Down, obtenu {other:?}"),
        }
    }

    /// **Le contrôle de la péremption.** Au-delà du TTL, la garde laisse passer
    /// pour que la garde C re-sonde — sans quoi le marqueur ne se lèverait
    /// jamais et la boucle serait bloquée définitivement.
    #[test]
    fn mika2049_un_marqueur_perime_laisse_passer() {
        assert!(
            !parse_stamp(
                &at(DOWN_TTL_DEFAULT_SECS + 1),
                DOWN_TTL_DEFAULT_SECS,
                Utc::now()
            )
            .is_down(),
            "au-delà du TTL le marqueur est périmé : personne d'autre ne re-sonde"
        );
        // Et la borne est inclusive du côté frais : à l'âge exact du TTL, le
        // marqueur mord encore. La frontière est écrite plutôt que supposée.
        assert!(
            parse_stamp(
                &at(DOWN_TTL_DEFAULT_SECS),
                DOWN_TTL_DEFAULT_SECS,
                Utc::now()
            )
            .is_down()
        );
    }

    /// Toute lecture impossible rend `Serving`. Quatre formes, une par ligne,
    /// parce qu'un test qui les neutraliserait ensemble ne prouverait pas
    /// lesquelles sont couvertes (leçon mika#2277).
    #[test]
    fn mika2049_toute_lecture_impossible_laisse_passer() {
        let now = Utc::now();
        assert!(
            !parse_stamp("", DOWN_TTL_DEFAULT_SECS, now).is_down(),
            "vide"
        );
        assert!(
            !parse_stamp("   \n", DOWN_TTL_DEFAULT_SECS, now).is_down(),
            "blanc"
        );
        assert!(
            !parse_stamp(
                "pas-une-date egress_bind_timeout\n",
                DOWN_TTL_DEFAULT_SECS,
                now
            )
            .is_down(),
            "horodatage inparsable"
        );
        assert!(
            !parse_stamp(&at(-600), DOWN_TTL_DEFAULT_SECS, now).is_down(),
            "horodatage dans le futur : traité comme illisible, jamais comme très frais — \
             sinon une dérive d'horloge gèlerait les gardes A et B jusqu'à cette date"
        );
    }

    /// Un marqueur sans motif reste une panne : le fait est la panne, le motif
    /// est le confort.
    #[test]
    fn mika2049_un_motif_absent_n_invalide_pas_le_marqueur() {
        let contents = format!("{}\n", crate::timestamp::now());
        let verdict = parse_stamp(&contents, DOWN_TTL_DEFAULT_SECS, Utc::now());
        assert!(verdict.is_down());
        assert!(
            matches!(verdict, RelayVerdict::Down { ref motif, .. } if motif == "egress_unavailable")
        );
    }

    /// Un fichier absent n'est pas une panne — et c'est le cas nominal sur toute
    /// installation dont le relais sert.
    #[test]
    fn mika2049_un_fichier_absent_n_est_pas_une_panne() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!relay_verdict(tmp.path(), DOWN_TTL_DEFAULT_SECS).is_down());
    }

    /// Le chemin est bien `{global_home}/state/<fichier>` — la moitié Rust de
    /// l'invariant de frontière.
    #[test]
    fn mika2049_le_chemin_est_sous_le_home_global() {
        let p = stamp_path(Path::new("/home/x/.mika"));
        assert!(p.starts_with("/home/x/.mika"));
        assert_eq!(p.parent().unwrap().file_name().unwrap(), "state");
    }

    /// Les trois paliers du TTL, et le fait que `0` ne désarme pas.
    #[test]
    fn mika2049_le_ttl_a_trois_paliers_et_zero_ne_desarme_pas() {
        assert_eq!(parse_ttl(None), DOWN_TTL_DEFAULT_SECS);
        assert_eq!(parse_ttl(Some("")), DOWN_TTL_DEFAULT_SECS);
        assert_eq!(parse_ttl(Some("   ")), DOWN_TTL_DEFAULT_SECS);
        assert_eq!(parse_ttl(Some("plif")), DOWN_TTL_DEFAULT_SECS);
        assert_eq!(parse_ttl(Some("-1")), DOWN_TTL_DEFAULT_SECS);
        assert_eq!(
            parse_ttl(Some("0")),
            DOWN_TTL_DEFAULT_SECS,
            "`0` rendrait le marqueur périmé à l'instant où il est écrit — \
             un désarmement silencieux déguisé en réglage"
        );
        assert_eq!(parse_ttl(Some("3600")), 3600);
    }
}
