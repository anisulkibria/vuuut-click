mod support;
use click_store::{Count, EventRow};
use serde_json::json;
use support::Harness;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn retention_discards_backfilled_pending_events_and_private_mail_delivery() {
    let h = Harness::new().await;
    let client = h.client();
    h.register(&client, "privacy@retention.test").await;
    let org = h.org(&client).await;
    let site = h.site(&client, org).await;
    let id = Uuid::parse_str(site["id"].as_str().unwrap()).unwrap();
    let now = chrono::Utc::now().timestamp() as u32;
    let old = EventRow {
        organization_id: org.to_string(),
        website_id: id.to_string(),
        event_id: "old".into(),
        timestamp: now - 10 * 86400,
        session_id: "session-old".into(),
        visitor_id: "visitor-old".into(),
        name: "pageview".into(),
        path: "/expired".into(),
        country: "US".into(),
        referrer: String::new(),
        utm_source: String::new(),
        utm_medium: String::new(),
        utm_campaign: String::new(),
        device: "Desktop".into(),
        browser: "Chrome".into(),
        occurred_ms: (now as i64 - 10 * 86400) * 1000,
        metric_name: String::new(),
        metric_id: String::new(),
        metric_value: 0.0,
        amount_minor: 0,
        currency: String::new(),
    };
    let mut recent = old.clone();
    recent.event_id = "recent".into();
    recent.timestamp = now - 2 * 86400;
    recent.occurred_ms = recent.timestamp as i64 * 1000;
    recent.path = "/kept".into();
    let mut insert = h.store.ch.insert("events").unwrap();
    insert.write(&old).await.unwrap();
    insert.write(&recent).await.unwrap();
    insert.end().await.unwrap();
    let event = Uuid::new_v4();
    // Delivery is pending now, but the event occurred outside retention. Acceptance time is not retention time.
    sqlx::query("INSERT INTO event_outbox(website_id,event_id,organization_id,payload,accepted_at) VALUES($1,$2,$3,$4,now())").bind(id).bind(event).bind(org).bind(serde_json::to_value(&old).unwrap()).execute(&h.store.pg).await.unwrap();
    sqlx::query("UPDATE websites SET retention_days=7 WHERE id=$1")
        .bind(id)
        .execute(&h.store.pg)
        .await
        .unwrap();
    click_server::jobs::enforce_retention(&h.store)
        .await
        .unwrap();
    let state: String =
        sqlx::query_scalar("SELECT state FROM event_outbox WHERE website_id=$1 AND event_id=$2")
            .bind(id)
            .bind(event)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(state, "discarded");
    assert_eq!(h.store.drain_once().await.unwrap(), 0);
    let count: Count = h
        .store
        .ch
        .query("SELECT count() AS count FROM events FINAL WHERE organization_id=? AND website_id=?")
        .bind(org.to_string())
        .bind(id.to_string())
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(count.count, 1);
    let views = h
        .store
        .overview(
            &h.store
                .collect_site(site["collection_id"].as_str().unwrap())
                .await
                .unwrap()
                .unwrap(),
            now - 30 * 86400,
            now + 1,
        )
        .await
        .unwrap();
    assert_eq!(views.pageviews, 1);
    assert!(h
        .post(&client, "/v1/account/reset/request")
        .json(&json!({"email":"privacy@retention.test"}))
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    let mail:Uuid=sqlx::query_scalar("SELECT id FROM mail_outbox WHERE recipient='privacy@retention.test' AND state='pending' ORDER BY created_at DESC LIMIT 1").fetch_one(&h.store.pg).await.unwrap();
    h.state.mail.drain_once(&h.store).await.unwrap();
    let path = std::path::PathBuf::from(format!(".local/mail/{mail}.eml"));
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("/account/reset#"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let payload: Option<String> =
        sqlx::query_scalar("SELECT body FROM mail_outbox WHERE id=$1 AND state='delivered'")
            .bind(mail)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert!(payload.is_none());
    std::fs::remove_file(path).unwrap();
    h.finish().await;
}
