use std::path::Path;

use serde::{Deserialize, Serialize};

use super::ProviderKind;

/// Metadata about a single model available from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Cached model list for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCache {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub fetched_at: String, // ISO 8601
    pub models: Vec<ModelInfo>,
}

/// Response shape for OpenAI-compatible `/models` endpoint.
#[derive(Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModel>,
}

#[derive(Deserialize)]
struct OpenAiModel {
    id: String,
}

/// Response shape for Ollama `/api/tags` endpoint.
#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: String,
}

const CACHE_TTL_SECS: i64 = 24 * 60 * 60; // 24 hours
const FETCH_TIMEOUT_SECS: u64 = 10;

/// Hardcoded model lists for providers without a public list API.
fn hardcoded_models(provider: ProviderKind) -> Vec<ModelInfo> {
    match provider {
        ProviderKind::Anthropic => vec![
            ModelInfo {
                id: "claude-fable-5".into(),
                name: Some("Claude Fable 5 (Mythos tier)".into()),
            },
            ModelInfo {
                id: "claude-sonnet-4-6".into(),
                name: Some("Claude Sonnet 4.6".into()),
            },
            ModelInfo {
                id: "claude-opus-4-6".into(),
                name: Some("Claude Opus 4.6".into()),
            },
            ModelInfo {
                id: "claude-haiku-4-5".into(),
                name: Some("Claude Haiku 4.5".into()),
            },
        ],
        ProviderKind::Google => vec![
            ModelInfo {
                id: "gemini-2.5-flash".into(),
                name: Some("Gemini 2.5 Flash".into()),
            },
            ModelInfo {
                id: "gemini-2.5-pro".into(),
                name: Some("Gemini 2.5 Pro".into()),
            },
            ModelInfo {
                id: "gemini-2.0-flash".into(),
                name: Some("Gemini 2.0 Flash".into()),
            },
        ],
        _ => vec![],
    }
}

/// Fetch the model list from a provider's API.
///
/// Returns `None` if the provider uses hardcoded models or if the fetch fails.
async fn fetch_models_from_api(
    provider: ProviderKind,
    base_url: &str,
    api_key: Option<&str>,
) -> Option<Vec<ModelInfo>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
        .ok()?;

    match provider {
        // Hardcoded providers — no API fetch needed
        ProviderKind::Anthropic | ProviderKind::Google => None,

        // Ollama uses a different endpoint shape
        ProviderKind::Ollama => {
            // Ollama default base_url is http://localhost:11434 (native provider).
            // Strip /v1 suffix for operators who explicitly override with an OpenAI-compat URL.
            let root_url = base_url
                .trim_end_matches('/')
                .strip_suffix("/v1")
                .unwrap_or(base_url);
            let url = format!("{root_url}/api/tags");
            let resp = client.get(&url).send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let body: OllamaTagsResponse = resp.json().await.ok()?;
            Some(
                body.models
                    .into_iter()
                    .map(|m| ModelInfo {
                        id: m.name,
                        name: None,
                    })
                    .collect(),
            )
        }

        // All OpenAI-compatible providers
        _ => {
            let url = format!("{}/models", base_url.trim_end_matches('/'));
            let mut req = client.get(&url);
            if let Some(key) = api_key {
                req = req.bearer_auth(key);
            }
            let resp = req.send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let body: OpenAiModelsResponse = resp.json().await.ok()?;
            let mut models: Vec<ModelInfo> = body
                .data
                .into_iter()
                .map(|m| ModelInfo {
                    id: m.id,
                    name: None,
                })
                .collect();
            models.sort_by(|a, b| a.id.cmp(&b.id));
            Some(models)
        }
    }
}

fn cache_path(agent_home: &Path, provider: ProviderKind) -> std::path::PathBuf {
    agent_home
        .join("cache")
        .join("models")
        .join(format!("{}.json", provider.config_prefix()))
}

fn read_cache(path: &Path) -> Option<ModelCache> {
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn is_cache_fresh(cache: &ModelCache, base_url: Option<&str>) -> bool {
    // Invalidate if base_url changed
    if cache.base_url.as_deref() != base_url {
        return false;
    }
    let Ok(fetched) = chrono::DateTime::parse_from_rfc3339(&cache.fetched_at) else {
        return false;
    };
    let age = chrono::Utc::now().signed_duration_since(fetched);
    age.num_seconds() < CACHE_TTL_SECS
}

fn write_cache(path: &Path, cache: &ModelCache) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(data) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(path, &data);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

/// Get the model list for a provider, using cache when available.
///
/// Resolution order:
/// 1. Fresh cache → return cached models
/// 2. API fetch succeeds → write cache, return fresh models
/// 3. Stale cache exists → return stale models
/// 4. Hardcoded models available → return those
/// 5. Empty list
pub async fn get_models(
    agent_home: &Path,
    provider: ProviderKind,
    base_url: Option<&str>,
    api_key: Option<&str>,
) -> Vec<ModelInfo> {
    // Hardcoded providers skip cache entirely
    let hardcoded = hardcoded_models(provider);
    if !hardcoded.is_empty() {
        return hardcoded;
    }

    let path = cache_path(agent_home, provider);

    // Check fresh cache
    if let Some(cache) = read_cache(&path)
        && is_cache_fresh(&cache, base_url)
    {
        return cache.models;
    }

    // Try fetching from API
    let effective_url = base_url.or(provider.default_base_url()).unwrap_or_default();
    if let Some(models) = fetch_models_from_api(provider, effective_url, api_key).await {
        let cache = ModelCache {
            provider: provider.config_prefix().to_string(),
            base_url: base_url.map(String::from),
            fetched_at: chrono::Utc::now().to_rfc3339(),
            models: models.clone(),
        };
        write_cache(&path, &cache);
        return models;
    }

    // Fall back to stale cache (any age)
    if let Some(cache) = read_cache(&path) {
        return cache.models;
    }

    // No cache, no API — empty
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardcoded_anthropic_models() {
        let models = hardcoded_models(ProviderKind::Anthropic);
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id == "claude-sonnet-4-6"));
    }

    #[test]
    fn hardcoded_google_models() {
        let models = hardcoded_models(ProviderKind::Google);
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id == "gemini-2.5-flash"));
    }

    #[test]
    fn cache_path_construction() {
        let path = cache_path(
            Path::new("/home/test/.mika/agents/default"),
            ProviderKind::DeepSeek,
        );
        assert_eq!(
            path.to_str().unwrap(),
            "/home/test/.mika/agents/default/cache/models/deepseek.json"
        );
    }

    #[test]
    fn cache_freshness_checks() {
        let fresh_cache = ModelCache {
            provider: "deepseek".into(),
            base_url: Some("https://api.deepseek.com".into()),
            fetched_at: chrono::Utc::now().to_rfc3339(),
            models: vec![],
        };
        assert!(is_cache_fresh(
            &fresh_cache,
            Some("https://api.deepseek.com")
        ));
        // Different base_url invalidates
        assert!(!is_cache_fresh(
            &fresh_cache,
            Some("https://custom.endpoint.com")
        ));
        // None vs Some invalidates
        assert!(!is_cache_fresh(&fresh_cache, None));

        let stale_cache = ModelCache {
            provider: "deepseek".into(),
            base_url: None,
            fetched_at: "2020-01-01T00:00:00Z".into(),
            models: vec![],
        };
        assert!(!is_cache_fresh(&stale_cache, None));
    }

    #[test]
    fn cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache/models/test.json");
        let cache = ModelCache {
            provider: "deepseek".into(),
            base_url: None,
            fetched_at: chrono::Utc::now().to_rfc3339(),
            models: vec![
                ModelInfo {
                    id: "deepseek-chat".into(),
                    name: Some("DeepSeek Chat".into()),
                },
                ModelInfo {
                    id: "deepseek-reasoner".into(),
                    name: None,
                },
            ],
        };
        write_cache(&path, &cache);
        let loaded = read_cache(&path).unwrap();
        assert_eq!(loaded.models.len(), 2);
        assert_eq!(loaded.models[0].id, "deepseek-chat");
    }
}
