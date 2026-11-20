//! `mika-cli` n'allume jamais `test-support` sur `mika-agent` (mika#1727 V4-c).
//!
//! Le verrou du vol V4-c rend les onze points d'entrée du défaut 44
//! `pub(crate)` **sauf** sous la feature `test-support` de `mika-agent`. Une
//! seule ligne de `[dependencies]` suffirait à le désarmer sans rien casser :
//! tout continuerait à compiler, et le lancement d'un agent en local
//! redeviendrait joignable depuis la production de ce crate. Le verrou est
//! structurel ; cette garde est ce qui tient la main qui le rouvrirait.
//!
//! La portée est `[dependencies]` seulement. `[dev-dependencies]` n'est pas
//! visé : les cibles de test de ce crate ne sont pas sa production.

/// L'entrée `mika-agent` de `[dependencies]`, si le manifeste en porte une.
///
/// Les deux formes du manifeste rendent la même table pour le parseur TOML :
/// `mika-agent.workspace = true` (la forme nue d'aujourd'hui) et
/// `mika-agent = { workspace = true, features = [...] }`. Rendre `Option`
/// plutôt que `bool` est ce qui permet au témoin de distinguer « l'entrée est
/// là et n'allume rien » de « rien n'a été lu » — un prédicat qui rend `false`
/// parce qu'il n'a pas trouvé sa cible n'est pas une garde (classe mika#3042).
fn entree_mika_agent(manifeste: &str) -> Option<toml::Value> {
    let doc: toml::Value = toml::from_str(manifeste).ok()?;
    doc.get("dependencies")?.get("mika-agent").cloned()
}

/// Le manifeste allume-t-il `test-support` sur `mika-agent` dans
/// `[dependencies]` ? Pur : le contre-témoin lui passe un littéral.
fn allume_test_support(manifeste: &str) -> bool {
    let Some(entree) = entree_mika_agent(manifeste) else {
        return false;
    };
    entree
        .get("features")
        .and_then(|f| f.as_array())
        .is_some_and(|features| features.iter().any(|f| f.as_str() == Some("test-support")))
}

fn manifeste_de_mika_cli() -> String {
    let chemin = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    std::fs::read_to_string(&chemin)
        .unwrap_or_else(|e| panic!("lecture de {} : {e}", chemin.display()))
}

/// La garde.
#[test]
fn mika1727_mika_cli_nallume_pas_test_support_sur_mika_agent() {
    assert!(
        !allume_test_support(&manifeste_de_mika_cli()),
        "crates/mika-cli/Cargo.toml allume `test-support` sur `mika-agent` dans \
         [dependencies] : cela rouvre les onze points d'entrée du défaut 44 à la \
         production de ce crate et désarme le verrou de mika#1727 V4-c. Retirer la \
         feature ; si un test de ce crate en a besoin, il la déclare dans \
         [dev-dependencies], jamais ici."
    );
}

/// Le témoin : la garde a bien lu l'entrée qu'elle prétend surveiller.
#[test]
fn mika1727_la_garde_a_trouve_lentree_mika_agent() {
    assert!(
        entree_mika_agent(&manifeste_de_mika_cli()).is_some(),
        "aucune entrée `mika-agent` dans [dependencies] de crates/mika-cli/Cargo.toml : \
         la garde ci-dessus rendrait vert sans rien avoir lu. Réancrer le prédicat sur \
         le nom qu'a pris la dépendance."
    );
}

/// Le contre-témoin : sur un manifeste qui l'allume, le prédicat rend `true`.
#[test]
fn mika1727_le_predicat_rougit_sur_un_manifeste_qui_lallume() {
    let fautif = r#"
[package]
name = "mika-cli"

[dependencies]
mika-agent = { workspace = true, features = ["test-support"] }
"#;
    assert!(allume_test_support(fautif));
    assert!(entree_mika_agent(fautif).is_some());
}
