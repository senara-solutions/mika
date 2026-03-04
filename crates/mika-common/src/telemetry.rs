use crate::config::Settings;

/// Guard that flushes remaining spans on drop.
///
/// Must be held alive for the duration of the program.
#[cfg(feature = "telemetry")]
pub struct TelemetryGuard(opentelemetry_sdk::trace::SdkTracerProvider);

#[cfg(feature = "telemetry")]
impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        let _ = self.0.shutdown();
    }
}

/// Build an OpenTelemetry tracing layer that exports spans via OTLP.
///
/// Returns `None` when telemetry is disabled, the endpoint is not configured,
/// or the exporter fails to build (graceful degradation).
#[cfg(feature = "telemetry")]
pub fn build_otel_layer<S>(
    settings: &Settings,
) -> Option<(
    tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::SdkTracer>,
    TelemetryGuard,
)>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    use opentelemetry::KeyValue;
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_otlp::{SpanExporter, WithExportConfig, WithHttpConfig};
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::trace::SdkTracerProvider;

    if !settings.telemetry_enabled {
        return None;
    }

    let endpoint = match settings.otlp_endpoint.as_ref() {
        Some(ep) if !ep.trim().is_empty() => ep.clone(),
        _ => {
            tracing::warn!("telemetry_enabled=true but otlp_endpoint is not set, skipping OTel");
            return None;
        }
    };

    // Build OTLP HTTP exporter
    let mut exporter_builder = SpanExporter::builder().with_http().with_endpoint(&endpoint);

    // Add auth header if configured (Langfuse uses Basic auth encoded as Base64).
    // Auto-detect raw "publicKey:secretKey" vs pre-encoded base64: if the value
    // contains ':' it's raw (base64 alphabet never includes ':'), so encode it.
    if let Some(ref auth) = settings.otlp_auth_header {
        let encoded = normalize_auth_header(auth);
        let mut headers = std::collections::HashMap::new();
        headers.insert("Authorization".to_string(), format!("Basic {encoded}"));
        exporter_builder = exporter_builder.with_headers(headers);
    }

    let exporter = match exporter_builder.build() {
        Ok(e) => e,
        Err(err) => {
            tracing::warn!("failed to build OTLP exporter: {err}, continuing without telemetry");
            return None;
        }
    };

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            Resource::builder()
                .with_attributes([KeyValue::new("service.name", "mika-agent")])
                .build(),
        )
        .build();

    let tracer = provider.tracer("mika");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    Some((layer, TelemetryGuard(provider)))
}

/// Normalize an OTLP auth header value: if it contains `:` it's a raw
/// `publicKey:secretKey` pair and needs base64 encoding; otherwise it's
/// already encoded and is returned as-is.
#[cfg(feature = "telemetry")]
fn normalize_auth_header(value: &str) -> String {
    use base64::Engine;
    if value.contains(':') {
        base64::engine::general_purpose::STANDARD.encode(value)
    } else {
        value.to_string()
    }
}

/// No-op when the telemetry feature is not enabled.
#[cfg(not(feature = "telemetry"))]
pub fn build_otel_layer(_settings: &Settings) -> Option<()> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "telemetry")]
    #[test]
    fn test_normalize_auth_header_raw_keys() {
        let raw = "pk-lf-abc123:sk-lf-xyz789";
        let result = super::normalize_auth_header(raw);
        // Should be base64-encoded
        assert!(!result.contains(':'));
        // Decode and verify round-trip
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&result)
            .unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), raw);
    }

    #[cfg(feature = "telemetry")]
    #[test]
    fn test_normalize_auth_header_already_encoded() {
        // Pre-encoded base64 of "pk:sk" — no colon in the encoded form
        use base64::Engine;
        let pre_encoded = base64::engine::general_purpose::STANDARD.encode("pk:sk");
        let result = super::normalize_auth_header(&pre_encoded);
        assert_eq!(result, pre_encoded);
    }

    #[test]
    fn test_build_otel_layer_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        // telemetry_enabled defaults to false
        #[cfg(feature = "telemetry")]
        assert!(build_otel_layer::<tracing_subscriber::Registry>(&settings).is_none());
        #[cfg(not(feature = "telemetry"))]
        assert!(build_otel_layer(&settings).is_none());
    }
}
