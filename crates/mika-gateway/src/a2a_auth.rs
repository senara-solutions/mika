use anyhow::Result;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

/// Validate an A2A API key and return the customer_id it belongs to.
pub async fn validate_a2a_api_key(pool: &PgPool, api_key: &str) -> Result<Option<String>> {
    let key_hash = hex::encode(Sha256::digest(api_key.as_bytes()));

    let result = sqlx::query_scalar::<_, String>(
        "SELECT customer_id FROM a2a_api_keys WHERE key_hash = $1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > NOW())"
    )
    .bind(&key_hash)
    .fetch_optional(pool)
    .await?;

    Ok(result)
}
