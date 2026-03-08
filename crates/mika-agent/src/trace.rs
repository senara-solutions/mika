/// Trace ID generation for orthogonal observability.
///
/// Produces a 32-character lowercase hex string (128 bits).
/// When the `telemetry` feature is enabled and an OTel span is active,
/// extracts the trace ID from the current span context.
/// Otherwise, generates a random 128-bit hex string.
pub fn generate_trace_id() -> String {
    #[cfg(feature = "telemetry")]
    {
        use opentelemetry::trace::TraceContextExt;
        use tracing_opentelemetry::OpenTelemetrySpanExt;

        let span = tracing::Span::current();
        let ctx = span.context();
        let span_ref = ctx.span();
        let sc = span_ref.span_context();
        if sc.trace_id() != opentelemetry::trace::TraceId::INVALID {
            return format!("{}", sc.trace_id());
        }
    }
    // Fallback: random 128-bit value as 32-char hex
    let id = uuid::Uuid::new_v4();
    id.as_bytes()
        .iter()
        .fold(String::with_capacity(32), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_trace_id_format() {
        let id = generate_trace_id();
        assert_eq!(id.len(), 32, "trace_id should be 32 chars: {id}");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "trace_id should be hex: {id}"
        );
        // Should be lowercase
        assert_eq!(id, id.to_lowercase(), "trace_id should be lowercase: {id}");
    }

    #[test]
    fn test_generate_trace_id_unique() {
        let id1 = generate_trace_id();
        let id2 = generate_trace_id();
        assert_ne!(id1, id2, "sequential trace_ids should differ");
    }
}
