//! Reçu de lecture colocalisé : « cet artefact a-t-il déjà été servi à cet
//! agent ? » (mika#2807, vol 2 de mika#1747).
//!
//! Un reçu est un fichier d'une ligne, caché, posé **à côté** de l'artefact
//! qu'il marque : `a/b.md` → `a/.b.md.read`, portant l'instant ISO 8601 du
//! service.
//!
//! # Ce module ne nomme aucune famille d'artefacts, et c'est délibéré
//!
//! Il répond à « cet artefact a-t-il déjà été servi ? » sans savoir d'où vient
//! l'artefact. mika#2807 l'applique à `transmissions/`, mika#2805 (vol 2 de
//! mika#1748) l'appliquera à `handsoffs/` — sans DDL, sans toucher à
//! `Database`, `AsyncDatabase` ni `migrate`. Deux marques seraient deux endroits
//! où la réponse peut diverger, et la marque est précisément la pièce dont la
//! correction décide si un agent relit la même lettre à chaque session ou ne la
//! lit jamais.
//!
//! # D1 — un fichier, pas une table
//!
//! Le corps de mika#2807 demande « l'état que le vol 1 a refusé (table,
//! migration v56→v57, **ou équivalent**) ». C'est l'équivalent, pour quatre
//! raisons :
//!
//! 1. **La doctrine du vol 1 est déjà celle-là** : sa D3-1 dit *le fichier EST
//!    la marque*. Une table à côté créerait deux régimes de persistance pour un
//!    rituel d'une dizaine de fichiers.
//! 2. **Le rituel se lit entièrement dans un `ls`** — la lettre, le signal, la
//!    réponse et le reçu sont dans le même répertoire.
//! 3. **Zéro couplage au schéma**, donc réellement réutilisable (voir ci-dessus).
//! 4. **Le budget** : table + migration + méthodes `Database` + enveloppes
//!    `AsyncDatabase` + test d'idempotence ≈ 85 lignes, contre ≈ 32 ici.
//!
//! Ce que la table aurait apporté et qui est abandonné, dit plutôt que tu : une
//! requête d'agrégat (« combien de lettres non lues sur la flotte ? ») demande
//! un parcours de systèmes de fichiers et non un `SELECT`. Personne ne l'a
//! demandée.
//!
//! # Le mode de dégradation est CHOISI
//!
//! [`mark_read`] ne rend pas de `Result` : elle est fail-open sous un WARN nommé
//! (`read_receipt_write_failed`). Un reçu qui ne s'écrit pas fait **re-servir**
//! l'artefact au tour suivant — le bloc devient permanent, bruyant, visible — au
//! lieu de le perdre en silence. Des deux pannes possibles, c'est la moins
//! mauvaise, et c'est la raison pour laquelle la signature n'offre à l'appelant
//! aucun moyen de propager l'autre.
//!
//! # Réarmement
//!
//! `rm {agent_home}/<dir>/.<artefact>.read` — un fichier caché, sans SQL, sans
//! accès à la base d'un conteneur. C'est le geste documenté au runbook de
//! [`docs/architecture/model-transmission.md`](../../../docs/architecture/model-transmission.md).

use std::path::Path;

/// Le chemin du reçu d'un artefact, relatif au même répertoire que lui.
///
/// Insère un `.` devant le dernier segment et suffixe `.read` :
/// `transmissions/2026-10-19-from-a-to-b.md` →
/// `transmissions/.2026-10-19-from-a-to-b.md.read`.
///
/// Opère sur [`str::rsplit_once`], jamais sur des octets :
/// `scripts/check-byte-slices.sh` refuse le découpage d'octets sur `&str`, et un
/// chemin d'artefact est composé à partir d'un nom de fichier sur disque.
pub fn receipt_path(artifact_rel: &str) -> String {
    match artifact_rel.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/.{name}.read"),
        None => format!(".{artifact_rel}.read"),
    }
}

/// Cet artefact a-t-il déjà été servi à cet agent ?
///
/// `false` est le régime nominal et porte la décision : l'artefact n'a pas
/// encore été servi. **Ne rend jamais d'erreur** — un répertoire illisible rend
/// `false`, donc re-sert, conformément au mode de dégradation choisi ci-dessus.
pub async fn is_read(agent_home: &Path, artifact_rel: &str) -> bool {
    tokio::fs::try_exists(agent_home.join(receipt_path(artifact_rel)))
        .await
        .unwrap_or(false)
}

/// Pose le reçu : cet artefact vient d'être servi.
///
/// Rend `()`, jamais `Result` — voir le mode de dégradation au doc-module. Le
/// répertoire parent est créé si besoin : un artefact peut être servi depuis un
/// répertoire que seul l'agent a écrit.
pub async fn mark_read(agent_home: &Path, artifact_rel: &str) {
    let path = agent_home.join(receipt_path(artifact_rel));
    let written = match path.parent() {
        Some(parent) => tokio::fs::create_dir_all(parent).await,
        None => Ok(()),
    };
    let written = match written {
        Ok(()) => tokio::fs::write(&path, format!("{}\n", crate::timestamp::now())).await,
        Err(e) => Err(e),
    };
    if let Err(e) = written {
        tracing::warn!(
            target: "mika::otel",
            event = "read_receipt_write_failed",
            artifact = %artifact_rel,
            error = %e,
            "reçu de lecture non écrit — l'artefact sera re-servi au tour suivant"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// V1 — la forme du chemin, à la racine comme en sous-répertoire, et sur un
    /// nom à points multiples (le `.` inséré va devant le **segment**, pas
    /// devant une extension).
    #[test]
    fn v1_forme_du_chemin_de_recu() {
        assert_eq!(
            receipt_path("transmissions/2026-10-19-from-a-to-b.md"),
            "transmissions/.2026-10-19-from-a-to-b.md.read"
        );
        assert_eq!(receipt_path("lettre.md"), ".lettre.md.read");
        assert_eq!(
            receipt_path("a/b/c.tar.gz.md"),
            "a/b/.c.tar.gz.md.read",
            "le point va devant le dernier segment, jamais devant une extension"
        );
    }

    /// V2 — `is_read` faux avant, vrai après, et le parent inexistant est créé.
    #[tokio::test]
    async fn v2_le_recu_ferme_la_question_et_cree_son_parent() {
        let home = tempfile::TempDir::new().unwrap();
        let artifact = "transmissions/2026-10-19-from-a-to-b.md";

        assert!(
            !is_read(home.path(), artifact).await,
            "sans reçu, l'artefact n'a pas été servi"
        );

        // Le répertoire n'existe pas encore : `mark_read` le crée.
        assert!(!home.path().join("transmissions").exists());
        mark_read(home.path(), artifact).await;

        assert!(is_read(home.path(), artifact).await);
        let content = std::fs::read_to_string(home.path().join(receipt_path(artifact))).unwrap();
        crate::timestamp::parse(content.trim()).expect("le reçu porte un instant ISO 8601 lisible");
    }

    /// Le reçu d'un artefact ne ferme pas la question d'un autre — la marque est
    /// par artefact, pas par répertoire.
    #[tokio::test]
    async fn v2_le_recu_est_par_artefact() {
        let home = tempfile::TempDir::new().unwrap();
        mark_read(home.path(), "transmissions/une.md").await;
        assert!(is_read(home.path(), "transmissions/une.md").await);
        assert!(!is_read(home.path(), "transmissions/autre.md").await);
    }
}
