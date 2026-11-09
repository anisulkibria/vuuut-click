//! OTLP spans contain only fixed route groups, methods, status and duration.
//! URI queries, credentials, identifiers, bodies and network addresses are excluded.
use crate::AppState;
use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use opentelemetry::{
    global,
    trace::{Span, SpanKind, Status, Tracer},
    KeyValue,
};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    trace::{Sampler, SdkTracerProvider},
    Resource,
};

pub struct Provider(Option<SdkTracerProvider>);
impl Drop for Provider {
    fn drop(&mut self) {
        if let Some(provider) = self.0.take() {
            if provider.shutdown().is_err() {
                tracing::warn!("Trace exporter shutdown failed");
            }
        }
    }
}
pub fn initialize() -> anyhow::Result<Provider> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .filter(|s| !s.is_empty());
    let Some(endpoint) = endpoint else {
        return Ok(Provider(None));
    };
    let url = url::Url::parse(&endpoint)?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "OTLP endpoint must be a trusted HTTP(S) collector URL without credentials or query"
    );
    let ratio: f64 = std::env::var("OTEL_TRACES_SAMPLER_ARG")
        .unwrap_or_else(|_| "0.1".into())
        .parse()?;
    anyhow::ensure!(
        ratio.is_finite() && (0.0..=1.0).contains(&ratio),
        "OTEL_TRACES_SAMPLER_ARG must be between 0 and 1"
    );
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
        .with_endpoint(format!("{}/v1/traces", endpoint.trim_end_matches('/')))
        .with_timeout(std::time::Duration::from_secs(3))
        .build()?;
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_sampler(Sampler::TraceIdRatioBased(ratio))
        .with_resource(
            Resource::builder_empty()
                .with_attribute(KeyValue::new("service.name", "vuuut-click"))
                .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
                .build(),
        )
        .build();
    global::set_tracer_provider(provider.clone());
    Ok(Provider(Some(provider)))
}
pub async fn observe(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let _request_slot = if matches!(
        req.uri().path(),
        "/healthz" | "/readyz" | "/internal/metrics"
    ) {
        None
    } else {
        match state.operations.requests.clone().try_acquire_owned() {
            Ok(slot) => Some(slot),
            Err(_) => {
                return crate::refusal(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "2",
                    "Service request capacity is busy; retry shortly",
                )
            }
        }
    };
    let route = crate::operations::route_group(req.uri().path());
    let method = match req.method().as_str() {
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" => req.method().as_str(),
        _ => "OTHER",
    }
    .to_owned();
    let tracer = global::tracer("click-server");
    let mut span = tracer
        .span_builder(format!("{method} {route}"))
        .with_kind(SpanKind::Server)
        .with_attributes([
            KeyValue::new("http.request.method", method.clone()),
            KeyValue::new("http.route", route),
        ])
        .start(&tracer);
    let started = std::time::Instant::now();
    let response =
        match tokio::time::timeout(std::time::Duration::from_secs(30), next.run(req)).await {
            Ok(response) => response,
            Err(_) => crate::refusal(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "2",
                "The request exceeded its time budget; retry shortly",
            ),
        };
    let code = response.status().as_u16();
    span.set_attribute(KeyValue::new("http.response.status_code", i64::from(code)));
    if code >= 500 {
        span.set_status(Status::error("request_failed"));
    }
    span.end();
    state
        .operations
        .observe(route, &method, code, started.elapsed().as_secs_f64());
    response
}
