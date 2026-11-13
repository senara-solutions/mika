//! `check_onboarding` et l'amorce de la personne utilisatrice.
//!
//! Une base fraîche est en onboarding ; `seed_user_person` y dépose la
//! personne que le prompt citera ensuite.
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// -- check_onboarding tests --

#[tokio::test]
async fn test_check_onboarding_true_when_no_core_memory() {
    let db = test_async_db();
    // Fresh DB has no core memory — should be onboarding
    assert!(check_onboarding(&db).await);
}

#[tokio::test]
async fn test_check_onboarding_true_when_default_value() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    // Seeded with defaults — still onboarding
    assert!(check_onboarding(&db).await);
}

#[tokio::test]
async fn test_check_onboarding_false_when_customized() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("user_summary", "Sam, software engineer")
        .await
        .unwrap();
    assert!(!check_onboarding(&db).await);
}

#[tokio::test]
async fn test_check_onboarding_true_even_when_other_sections_customized() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("self_model", "Custom self model")
        .await
        .unwrap();
    assert!(check_onboarding(&db).await);
}

// -- seed_user_person tests --

#[tokio::test]
async fn test_seed_user_person_after_onboarding() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("user_summary", "Sam, software engineer at Senara Solutions")
        .await
        .unwrap();

    seed_user_person(&db).await.unwrap();

    let person = db.get_person("Sam").await.unwrap();
    assert!(person.is_some());
    let person = person.unwrap();
    assert_eq!(person.relationship.as_deref(), Some("The user"));
}

#[tokio::test]
async fn test_seed_user_person_skips_when_default() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    // user_summary is still at default

    seed_user_person(&db).await.unwrap();

    let people = db.list_people().await.unwrap();
    assert!(people.is_empty());
}

#[tokio::test]
async fn test_seed_user_person_skips_when_already_exists() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("user_summary", "Sam, software engineer")
        .await
        .unwrap();
    // Pre-create the person
    db.upsert_person("Sam", Some("Friend"), None).await.unwrap();

    seed_user_person(&db).await.unwrap();

    // Relationship should NOT be overwritten
    let person = db.get_person("Sam").await.unwrap().unwrap();
    assert_eq!(person.relationship.as_deref(), Some("Friend"));
}

#[tokio::test]
async fn test_seed_user_person_extracts_name_before_comma() {
    let db = test_async_db();
    db.seed_core_memory(None).await.unwrap();
    db.set_core_memory("user_summary", "Alice Johnson, product manager")
        .await
        .unwrap();

    seed_user_person(&db).await.unwrap();

    let person = db.get_person("Alice Johnson").await.unwrap();
    assert!(person.is_some());
}
