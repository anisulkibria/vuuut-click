mod support;
use reqwest::StatusCode;
use serde_json::Value;
use support::Harness;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn owner_connections_are_private_encrypted_validated_and_applied() {
    let h = Harness::new().await;
    let owner = h.client();
    let customer = h.client();
    let account = h.register(&owner, "connections-owner@test.example").await;
    h.register(&customer, "connections-customer@test.example")
        .await;
    let id = Uuid::parse_str(account["id"].as_str().unwrap()).unwrap();
    sqlx::query("INSERT INTO platform_owners(user_id) VALUES($1)")
        .bind(id)
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        h.post(&owner, "/owner/auth/login")
            .form(&[
                ("email", "connections-owner@test.example"),
                ("password", "correct-horse-battery-staple")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    let settings = [
        ("service", "maxmind"),
        ("action", "save"),
        ("MAXMIND_ACCOUNT_ID", "123456"),
        ("MAXMIND_LICENSE_KEY", "private-maxmind-license"),
    ];
    assert!(!h
        .post(&customer, "/owner/connections")
        .form(&settings)
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    assert_eq!(
        owner
            .post(format!("{}/owner/connections", h.base))
            .header("Origin", "https://foreign.example")
            .form(&settings)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.post(&owner, "/owner/connections")
            .form(&settings)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    let encrypted: Vec<u8> =
        sqlx::query_scalar("SELECT ciphertext FROM platform_connections WHERE service='maxmind'")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert!(!encrypted.windows(7).any(|b| b == b"private"));
    let page = owner
        .get(format!("{}/owner/connections", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(page.headers()["cache-control"], "private, no-store");
    let html = page.text().await.unwrap();
    assert!(html.contains("123456"));
    assert!(html.contains("Saved · leave blank to keep"));
    assert!(!html.contains("private-maxmind-license"));
    assert_eq!(
        h.post(&owner, "/owner/connections")
            .form(&[
                ("service", "maxmind"),
                ("action", "save"),
                ("MAXMIND_ACCOUNT_ID", "invalid")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let revision: i64 =
        sqlx::query_scalar("SELECT revision FROM platform_connections WHERE service='maxmind'")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(revision, 1);
    // Blank passwords preserve the existing encrypted value.
    assert_eq!(
        h.post(&owner, "/owner/connections")
            .form(&[
                ("service", "maxmind"),
                ("action", "save"),
                ("MAXMIND_LICENSE_KEY", "")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    // A second server instance with the same external key can reload records.
    let smtp = [
        ("service", "smtp"),
        ("action", "save"),
        ("SMTP_HOST", "smtp.example.com"),
        ("SMTP_PORT", "587"),
        ("SMTP_FROM", "Vuuut <sender@example.com>"),
        ("SMTP_USERNAME", "secret-user"),
        ("SMTP_PASSWORD", "secret-password"),
        ("COMMERCIAL_TEST_EMAIL", "controlled@example.com"),
    ];
    assert_eq!(
        h.post(&owner, "/owner/connections")
            .form(&smtp)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(h.state.mail_service().mode(), "smtp");
    let html = owner
        .get(format!("{}/owner/connections", h.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!html.contains("secret-password"));
    assert!(!html.contains("secret-user"));
    assert_eq!(
        h.post(&owner, "/owner/connections")
            .form(&[("service", "smtp"), ("action", "reset")])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(h.state.mail_service().mode(), "local_file");
    let key = "0123456789abcdef0123456789abcdef";
    assert_eq!(
        h.post(&owner, "/owner/connections")
            .form(&[
                ("service", "hosting"),
                ("action", "platform"),
                ("platform_id", "vuuut-pro"),
                ("domain_suffix", "sites.example.com"),
                ("platform_key", key)
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    let token: String =
        sqlx::query_scalar("SELECT token_hash FROM native_platforms WHERE id='vuuut-pro'")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(token, click_core::digest(key));
    assert_ne!(token, key);
    let audit: Vec<Value> = sqlx::query_scalar("SELECT to_jsonb(p) FROM platform_audit p")
        .fetch_all(&h.store.pg)
        .await
        .unwrap();
    assert!(!format!("{audit:?}").contains("private-maxmind"));
    assert_eq!(
        h.post(&owner, "/owner/connections")
            .form(&[("service", "maxmind"), ("action", "reset")])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    h.finish().await;
}

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn country_setup_keeps_owner_access_but_blocks_collection_until_ready() {
    use axum::{body::Body, extract::ConnectInfo, http::Request};
    use tower::ServiceExt;
    let h = Harness::new().await;
    let customer = h.client();
    h.register(&customer, "country-bootstrap@test.example")
        .await;
    let org = h.org(&customer).await;
    let site = h.site(&customer, org).await;
    let mut state = h.state.clone();
    let mut config = (*state.config).clone();
    config.require_geoip = true;
    state.config = std::sync::Arc::new(config);
    let router = click_server::app(state.clone());
    let ready = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);
    let owner_page = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/owner/signin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(owner_page.status(), StatusCode::OK);
    let payload = serde_json::json!({"events":[{"event_id":Uuid::new_v4(),"session_id":Uuid::new_v4(),"name":"pageview","url":h.base}]});
    let request = Request::builder()
        .method("POST")
        .uri(format!(
            "/v1/collect/{}",
            site["collection_id"].as_str().unwrap()
        ))
        .header("Origin", &h.base)
        .header("Content-Type", "application/json")
        .extension(ConnectInfo(
            "127.0.0.1:4000".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .body(Body::from(payload.to_string()))
        .unwrap();
    let collected = router.oneshot(request).await.unwrap();
    assert_eq!(collected.status(), StatusCode::SERVICE_UNAVAILABLE);
    let accepted: i64 = sqlx::query_scalar("SELECT count(*) FROM event_outbox")
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(accepted, 0);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/GeoIP2-Country-Test.mmdb");
    state.geoip = std::sync::Arc::new(click_core::GeoIp::open(Some(&fixture)).unwrap());
    let ready = click_server::app(state)
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    h.finish().await;
}
