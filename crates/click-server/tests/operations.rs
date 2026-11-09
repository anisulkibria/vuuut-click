mod support;
use click_server::{operations::Operations, AppState};
use reqwest::StatusCode;
use std::sync::Arc;
use support::Harness;
use uuid::Uuid;

const TOKEN: &str = "operational-fixture-token-32-characters-long";
fn replica(h: &Harness) -> AppState {
    let mut state = AppState::with_services(
        h.store.clone(),
        (*h.state.config).clone(),
        click_server::billing::Service { provider: None },
        click_server::mail::Mailer::local(".local/mail".into()),
    )
    .unwrap();
    state.operations = Arc::new(Operations::configured(
        true,
        vec![42; 32],
        Some(TOKEN.into()),
    ));
    state
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn replicas_share_private_abuse_budgets_and_report_leases() {
    let h = Harness::new().await;
    let a = replica(&h);
    let b = replica(&h);
    let ip = "203.0.113.29".parse().unwrap();
    for index in 0..20 {
        assert!(if index % 2 == 0 {
            a.operations.edge(&h.store, ip, true).await
        } else {
            b.operations.edge(&h.store, ip, true).await
        }
        .unwrap());
    }
    assert!(!b.operations.edge(&h.store, ip, true).await.unwrap());
    let subjects: Vec<String> =
        sqlx::query_scalar("SELECT subject FROM request_budgets WHERE scope='auth_ip'")
            .fetch_all(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(subjects.len(), 1);
    assert_eq!(subjects[0].len(), 64);
    assert!(!subjects[0].contains("203.0.113"));
    let c = h.client();
    h.register(&c, "operations@fixture.test").await;
    let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM memberships LIMIT 1")
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    let mut permits = Vec::new();
    for state in [&a, &b, &a, &b] {
        let local = state.tenant_queries.try_acquire(org).unwrap();
        permits.push(state.operations.report(&h.store, org, local).await.unwrap());
    }
    let local = a.tenant_queries.try_acquire(org).unwrap();
    assert_eq!(
        a.operations
            .report(&h.store, org, local)
            .await
            .err()
            .unwrap()
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(permits.pop());
    for _ in 0..100 {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM report_leases")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
        if count == 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let local = b.tenant_queries.try_acquire(org).unwrap();
    permits.push(b.operations.report(&h.store, org, local).await.unwrap());
    // Crash leases expire in the shared store and another replica recovers capacity.
    sqlx::query("UPDATE report_leases SET expires_at=now()-interval '1 second'")
        .execute(&h.store.pg)
        .await
        .unwrap();
    let local = a.tenant_queries.try_acquire(org).unwrap();
    let recovered = a.operations.report(&h.store, org, local).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM report_leases")
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(count, 1);
    drop(recovered);
    drop(permits);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    h.finish().await;
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn maintenance_barrier_stops_writers_and_delivery_but_preserves_readiness() {
    let h = Harness::new().await;
    let c = h.client();
    h.register(&c, "maintenance@fixture.test").await;
    let mut freeze = h.store.pg.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(78422990)")
        .execute(&mut *freeze)
        .await
        .unwrap();
    let refused = h
        .post(&c, "/v1/organizations")
        .json(&serde_json::json!({"name":"During maintenance"}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(refused.headers()["retry-after"], "30");
    assert_eq!(h.store.drain_once().await.unwrap(), 0);
    assert_eq!(
        c.get(format!("{}/readyz", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    freeze.rollback().await.unwrap();
    assert_eq!(
        h.post(&c, "/v1/organizations")
            .json(&serde_json::json!({"name":"After maintenance"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    h.finish().await;
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn metrics_are_private_bounded_and_exclude_request_secrets() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let h = Harness::new().await;
    let state = replica(&h);
    let router = click_server::app(state.clone());
    assert_eq!(
        router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/internal/metrics")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let marker = "request-path-that-must-not-be-exported";
    router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/{marker}?token=private-query"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let response = router
        .oneshot(
            Request::builder()
                .uri("/internal/metrics")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8(raw.to_vec()).unwrap();
    assert!(text.contains("click_dependency_up{backend=\"postgres\"} 1"));
    assert!(text.contains("click_http_request_duration_seconds_bucket"));
    for forbidden in [
        marker,
        "private-query",
        TOKEN,
        "203.0.113",
        "password",
        "recipient",
        "organization_id",
    ] {
        assert!(!text.contains(forbidden));
    }
    h.finish().await;
}

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn postgres_failure_does_not_hide_liveness_or_dependency_metrics() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let h = Harness::new().await;
    let mut state = replica(&h);
    state.store.pg = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(150))
        .connect_lazy("postgres://fixture:fixture@127.0.0.1:9/unavailable")
        .unwrap();
    let router = click_server::app(state);
    assert_eq!(
        router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let response = router
        .oneshot(
            Request::builder()
                .uri("/internal/metrics")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let raw = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(String::from_utf8(raw.to_vec())
        .unwrap()
        .contains("click_dependency_up{backend=\"postgres\"} 0"));
    h.finish().await;
}
