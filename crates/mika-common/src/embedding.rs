use anyhow::{Context, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, warn};

/// Error type for embedding API calls that preserves the HTTP status code.
#[derive(Debug, Error)]
#[error("embedding API HTTP error ({status_code})")]
struct EmbeddingApiError {
    status_code: u16,
}

const OPENAI_EMBEDDINGS_URL: &str = "https://api.openai.com/v1/embeddings";
const MAX_RETRIES: u32 = 2;

// -- Request / Response types --

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: EmbeddingInput<'a>,
    dimensions: u32,
}

#[derive(Serialize)]
#[serde(untagged)]
enum EmbeddingInput<'a> {
    Single(&'a str),
    Batch(&'a [&'a str]),
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

// -- Client --

#[derive(Clone)]
pub struct EmbeddingClient {
    client: reqwest::Client,
    api_key: String,
    model: String,
    dimensions: u32,
}

impl std::fmt::Debug for EmbeddingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingClient")
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl EmbeddingClient {
    pub fn new(api_key: String, model: String, dimensions: u32) -> Result<Self> {
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            anyhow::bail!("OpenAI API key is required but empty");
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build HTTP client for embeddings")?;

        Ok(Self {
            client,
            api_key,
            model,
            dimensions,
        })
    }

    /// Embed a single text string, returning a vector of floats.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let request = EmbeddingRequest {
            model: &self.model,
            input: EmbeddingInput::Single(text),
            dimensions: self.dimensions,
        };

        let mut response = self.send_request(&request).await?;
        response
            .data
            .pop()
            .map(|d| d.embedding)
            .context("empty embedding response")
    }

    /// Embed multiple texts in a single API call, returning one vector per input.
    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = EmbeddingRequest {
            model: &self.model,
            input: EmbeddingInput::Batch(texts),
            dimensions: self.dimensions,
        };

        let response = self.send_request(&request).await?;
        Ok(response.data.into_iter().map(|d| d.embedding).collect())
    }

    /// The configured embedding dimensions.
    pub fn dimensions(&self) -> u32 {
        self.dimensions
    }

    async fn send_request(&self, request: &EmbeddingRequest<'_>) -> Result<EmbeddingResponse> {
        let auth_value = HeaderValue::from_str(&format!("Bearer {}", self.api_key))
            .context("invalid API key characters")?;

        let mut last_error = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                warn!(
                    attempt,
                    delay_ms = delay.as_millis(),
                    "retrying embedding API call"
                );
                tokio::time::sleep(delay).await;
            }

            match self.send_once(request, auth_value.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    if attempt < MAX_RETRIES && is_retryable_status(&e) {
                        warn!(attempt, error = %e, "transient embedding API error");
                        last_error = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("max retries exceeded")))
    }

    async fn send_once(
        &self,
        request: &EmbeddingRequest<'_>,
        auth_value: HeaderValue,
    ) -> Result<EmbeddingResponse> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(AUTHORIZATION, auth_value);

        let response = self
            .client
            .post(OPENAI_EMBEDDINGS_URL)
            .headers(headers)
            .json(request)
            .send()
            .await
            .context("embedding API request failed")?;

        let status = response.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let body = response.text().await.unwrap_or_default();
            let truncated_body = crate::text::safe_truncate(&body, 500);
            warn!(status = status_code, body = %truncated_body, "embedding API error response");
            return Err(EmbeddingApiError { status_code }.into());
        }

        let resp: EmbeddingResponse = response
            .json()
            .await
            .context("failed to parse embedding response")?;

        debug!(
            count = resp.data.len(),
            dimensions = resp.data.first().map(|d| d.embedding.len()).unwrap_or(0),
            "embedding response received"
        );

        Ok(resp)
    }
}

fn is_retryable_status(error: &anyhow::Error) -> bool {
    if let Some(e) = error.downcast_ref::<EmbeddingApiError>() {
        matches!(e.status_code, 429 | 500 | 503)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_api_key_rejected() {
        assert!(EmbeddingClient::new("".to_string(), "model".to_string(), 512).is_err());
        assert!(EmbeddingClient::new("   ".to_string(), "model".to_string(), 512).is_err());
    }

    #[test]
    fn test_valid_client_creation() {
        let client = EmbeddingClient::new(
            "sk-test-key".to_string(),
            "text-embedding-3-small".to_string(),
            512,
        );
        assert!(client.is_ok());
        assert_eq!(client.unwrap().dimensions(), 512);
    }

    #[tokio::test]
    async fn test_embed_batch_empty() {
        let client = EmbeddingClient::new(
            "sk-test-key".to_string(),
            "text-embedding-3-small".to_string(),
            512,
        )
        .unwrap();
        let result = client.embed_batch(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_embedding_request_serialization() {
        let req = EmbeddingRequest {
            model: "text-embedding-3-small",
            input: EmbeddingInput::Single("hello world"),
            dimensions: 512,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "text-embedding-3-small");
        assert_eq!(json["input"], "hello world");
        assert_eq!(json["dimensions"], 512);
    }

    #[test]
    fn test_embedding_batch_request_serialization() {
        let texts = ["hello", "world"];
        let req = EmbeddingRequest {
            model: "text-embedding-3-small",
            input: EmbeddingInput::Batch(&texts),
            dimensions: 512,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["input"], serde_json::json!(["hello", "world"]));
    }

    #[test]
    fn test_embedding_response_deserialization() {
        let json = r#"{
            "object": "list",
            "data": [
                {"object": "embedding", "index": 0, "embedding": [0.1, 0.2, 0.3]}
            ],
            "model": "text-embedding-3-small",
            "usage": {"prompt_tokens": 5, "total_tokens": 5}
        }"#;
        let resp: EmbeddingResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].embedding, vec![0.1, 0.2, 0.3]);
    }
}
