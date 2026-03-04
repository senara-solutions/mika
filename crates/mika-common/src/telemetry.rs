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

    // Add auth header if configured (Langfuse uses Basic auth encoded as Base64)
    if let Some(ref auth) = settings.otlp_auth_header {
        let mut headers = std::collections::HashMap::new();
        headers.insert("Authorization".to_string(), format!("Basic {auth}"));
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

/// No-op when the telemetry feature is not enabled.
#[cfg(not(feature = "telemetry"))]
pub fn build_otel_layer(_settings: &Settings) -> Option<()> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_otel_layer_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let settings = Settings::load(tmp.path()).unwrap();
        // telemetry_enabled defaults to false
        assert!(build_otel_layer(&settings).is_none());
    }
}
