mod support;
use async_trait::async_trait;
use axum::http::HeaderMap;
use click_server::{billing::*, commercial, ApiError, AppResult};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use support::Harness;
use uuid::Uuid;

struct Provider {
    requests: Mutex<Vec<CheckoutRequest>>,
    fail: AtomicBool,
    cancel_fail: AtomicBool,
    status: Mutex<String>,
    price: Mutex<String>,
    customer: Mutex<String>,
    portal_gate: AtomicBool,
    portal_started: tokio::sync::Notify,
    portal_release: tokio::sync::Notify,
}
impl Provider {
    fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            fail: AtomicBool::new(true),
            cancel_fail: AtomicBool::new(true),
            status: Mutex::new("active".into()),
            price: Mutex::new("price_pro".into()),
            customer: Mutex::new("cus_fixture".into()),
            portal_gate: AtomicBool::new(false),
            portal_started: tokio::sync::Notify::new(),
            portal_release: tokio::sync::Notify::new(),
        }
    }
}
#[async_trait]
impl PaymentProvider for Provider {
    fn name(&self) -> &'static str {
        "stripe"
    }
    fn supports(&self, plan: &str, period: &str) -> bool {
        plan == "pro" && matches!(period, "monthly" | "annual")
    }
    fn resolve_price(&self, price: &str) -> Option<(String, String)> {
        (price == "price_pro").then(|| ("pro".into(), "monthly".into()))
    }
    fn verify(&self, _: &HeaderMap, _: &[u8]) -> AppResult<ProviderEvent> {
        Err(ApiError::bad("Use explicit test facts"))
    }
    async fn checkout(&self, r: CheckoutRequest) -> AppResult<Checkout> {
        let id = format!("cs_{}", r.attempt.simple());
        self.requests.lock().unwrap().push(r);
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(ApiError {
                status: StatusCode::BAD_GATEWAY,
                message: "Temporary test outage",
            });
        }
        Ok(Checkout {
            id,
            url: "https://checkout.stripe.com/fixture".into(),
        })
    }
    async fn portal(&self, _: &str, _: &str) -> AppResult<String> {
        if self.portal_gate.load(Ordering::SeqCst) {
            self.portal_started.notify_one();
            self.portal_release.notified().await;
        }
        Ok("https://billing.stripe.com/fixture".into())
    }
    async fn subscription(&self, id: &str) -> AppResult<Subscription> {
        Ok(Subscription {
            id: id.into(),
            customer: self.customer.lock().unwrap().clone(),
            status: self.status.lock().unwrap().clone(),
            price: self.price.lock().unwrap().clone(),
        })
    }
    async fn expire_checkout(&self, _: &str) -> AppResult<()> {
        if self.cancel_fail.swap(false, Ordering::SeqCst) {
            return Err(ApiError {
                status: StatusCode::BAD_GATEWAY,
                message: "Cancellation not acknowledged",
            });
        }
        Ok(())
    }
}
async fn action(
    h: &Harness,
    c: &reqwest::Client,
    org: Uuid,
    action: &str,
    value: &str,
) -> reqwest::Response {
    h.post(c, "/settings/action")
        .form(&[
            ("organization_id", org.to_string()),
            ("action", action.into()),
            ("value", value.into()),
        ])
        .send()
        .await
        .unwrap()
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn checkout_snapshots_cancellation_and_late_events_preserve_the_linked_subscription() {
    let p = Arc::new(Provider::new());
    let h = Harness::provider(Some(p.clone())).await;
    let c = h.client();
    h.register(&c, "checkout@commercial.test").await;
    let org = h.org(&c).await;
    assert_eq!(
        action(&h, &c, org, "checkout", "pro:monthly")
            .await
            .status(),
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(
        action(&h, &c, org, "checkout", "pro:annual").await.status(),
        StatusCode::CONFLICT
    );
    sqlx::query(
        "UPDATE users SET email='changed@commercial.test' WHERE email='checkout@commercial.test'",
    )
    .execute(&h.store.pg)
    .await
    .unwrap();
    assert_eq!(
        action(&h, &c, org, "checkout", "pro:monthly")
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    {
        let r = p.requests.lock().unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(
            serde_json::to_value(&r[0]).unwrap(),
            serde_json::to_value(&r[1]).unwrap()
        );
    }
    assert_eq!(
        action(&h, &c, org, "cancel_checkout", "").await.status(),
        StatusCode::BAD_GATEWAY
    );
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM billing_checkouts WHERE organization_id=$1 AND cancelled_at IS NULL",
    )
    .bind(org)
    .fetch_one(&h.store.pg)
    .await
    .unwrap();
    assert_eq!(pending, 1);
    assert_eq!(
        action(&h, &c, org, "cancel_checkout", "").await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        action(&h, &c, org, "checkout", "pro:annual").await.status(),
        StatusCode::SEE_OTHER
    );
    let (attempt,session):(Uuid,String)=sqlx::query_as("SELECT id,provider_reference FROM billing_checkouts WHERE organization_id=$1 AND cancelled_at IS NULL").bind(org).fetch_one(&h.store.pg).await.unwrap();
    apply_event(
        &h.state,
        p.as_ref(),
        ProviderEvent {
            id: "evt_complete".into(),
            kind: "checkout.session.completed".into(),
            created: 1000,
            change: Change::Checkout {
                attempt,
                session,
                subscription: "sub_current".into(),
            },
        },
    )
    .await
    .unwrap();
    // An older checkout from a different subscription cannot replace this mapping.
    let old = Uuid::new_v4();
    sqlx::query("INSERT INTO billing_checkouts(id,organization_id,plan_id,billing_period,provider_reference) VALUES($1,$2,'pro','monthly','cs_old')").bind(old).bind(org).execute(&h.store.pg).await.unwrap();
    let old_event = ProviderEvent {
        id: "evt_old".into(),
        kind: "checkout.session.completed".into(),
        created: 900,
        change: Change::Checkout {
            attempt: old,
            session: "cs_old".into(),
            subscription: "sub_old".into(),
        },
    };
    assert_eq!(
        apply_event(&h.state, p.as_ref(), old_event)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    let linked: String =
        sqlx::query_scalar("SELECT provider_reference FROM subscriptions WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(linked, "sub_current");
    // Billing status changes never clear an operator suspension.
    sqlx::query("UPDATE organizations SET status='suspended' WHERE id=$1")
        .bind(org)
        .execute(&h.store.pg)
        .await
        .unwrap();
    for (index, (provider_status, local)) in [
        ("trialing", "active"),
        ("past_due", "past_due"),
        ("unpaid", "past_due"),
        ("paused", "suspended"),
        ("incomplete", "suspended"),
        ("active", "active"),
    ]
    .into_iter()
    .enumerate()
    {
        *p.status.lock().unwrap() = provider_status.into();
        apply_event(
            &h.state,
            p.as_ref(),
            ProviderEvent {
                id: format!("evt_status_{index}"),
                kind: "customer.subscription.updated".into(),
                created: 800 - index as i64,
                change: Change::Subscription("sub_current".into()),
            },
        )
        .await
        .unwrap();
        let status:(String,String)=sqlx::query_as("SELECT s.status,o.status FROM subscriptions s JOIN organizations o ON o.id=s.organization_id WHERE o.id=$1").bind(org).fetch_one(&h.store.pg).await.unwrap();
        assert_eq!(status, (local.into(), "suspended".into()));
    }
    *p.status.lock().unwrap() = "canceled".into();
    *p.price.lock().unwrap() = "price_retired".into();
    apply_event(
        &h.state,
        p.as_ref(),
        ProviderEvent {
            id: "evt_cancel_retired_price".into(),
            kind: "customer.subscription.deleted".into(),
            created: 1100,
            change: Change::Subscription("sub_current".into()),
        },
    )
    .await
    .unwrap();
    let plan: String =
        sqlx::query_scalar("SELECT plan_id FROM subscriptions WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(plan, "free");
    h.finish().await;
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn mail_failures_expiry_owner_retry_and_fulfillment_evidence_are_durable() {
    let h = Harness::new().await;
    let c = h.client();
    h.register(&c, "operator@commercial.test").await;
    let user: Uuid =
        sqlx::query_scalar("SELECT id FROM users WHERE email='operator@commercial.test'")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    let malformed = Uuid::new_v4();
    sqlx::query("INSERT INTO mail_outbox(id,recipient,subject,body,expires_at) VALUES($1,'invalid-address','Fixture','private fixture body',now()+interval '1 hour')").bind(malformed).execute(&h.store.pg).await.unwrap();
    for _ in 0..10 {
        assert!(h.state.mail.drain_once(&h.store).await.is_err());
        sqlx::query("UPDATE mail_outbox SET available_at=now() WHERE id=$1")
            .bind(malformed)
            .execute(&h.store.pg)
            .await
            .unwrap();
    }
    let state: (String, i32) = sqlx::query_as("SELECT state,attempts FROM mail_outbox WHERE id=$1")
        .bind(malformed)
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(state, ("dead".into(), 10));
    assert_eq!(
        c.get(format!("{}/owner/v1/commercial", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("INSERT INTO platform_owners(user_id) VALUES($1)")
        .bind(user)
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        h.post(&c, "/owner/auth/login")
            .form(&[
                ("email", "operator@commercial.test"),
                ("password", "correct-horse-battery-staple")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        h.post(&c, "/owner/commercial/retry-mail")
            .form(&[("id", malformed.to_string())])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    sqlx::query(
        "UPDATE mail_outbox SET state='dead',expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(malformed)
    .execute(&h.store.pg)
    .await
    .unwrap();
    assert_eq!(
        h.post(&c, "/owner/commercial/retry-mail")
            .form(&[("id", malformed.to_string())])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    h.state.mail.drain_once(&h.store).await.unwrap();
    let cleared: (String, Option<String>) =
        sqlx::query_as("SELECT state,body FROM mail_outbox WHERE id=$1")
            .bind(malformed)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(cleared, ("expired".into(), None));
    assert!(commercial::run(&h.store, "exports", async {
        anyhow::bail!("private error must not be stored")
    })
    .await
    .is_err());
    let failed: Value = c
        .get(format!("{}/owner/v1/commercial", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(failed["checks"][0]["last_error"], "operation_failed");
    assert!(!failed.to_string().contains("private error"));
    commercial::run(&h.store, "exports", async { Ok(()) })
        .await
        .unwrap();
    let recorded = sqlx::query_scalar::<_, Option<String>>(
        "SELECT last_error FROM maintenance_checks WHERE name='exports'",
    )
    .fetch_one(&h.store.pg)
    .await
    .unwrap();
    assert_eq!(recorded, None);
    let old = chrono::Utc::now() - chrono::Duration::hours(1);
    commercial::record(&h.store, "exports", old, false)
        .await
        .unwrap();
    let record = sqlx::query_scalar::<_, Option<String>>(
        "SELECT last_error FROM maintenance_checks WHERE name='exports'",
    )
    .fetch_one(&h.store.pg)
    .await
    .unwrap();
    assert_eq!(record, None);
    let html = c
        .get(format!("{}/owner/commercial", h.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("Not observed"));
    assert!(!html.contains("private fixture body"));
    let audit_client = h.client();
    h.register(&audit_client, "retention-audit@commercial.test")
        .await;
    let audit_org = h.org(&audit_client).await;
    h.site(&audit_client, audit_org).await;
    let mut outage = h.store.clone();
    outage.ch = outage.ch.with_url("http://127.0.0.1:1");
    assert!(click_server::jobs::enforce_retention(&outage)
        .await
        .is_err());
    let failure: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM maintenance_checks WHERE name='retention'")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(failure.as_deref(), Some("operation_failed"));
    click_server::jobs::enforce_retention(&h.store)
        .await
        .unwrap();
    let recovered: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM maintenance_checks WHERE name='retention'")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(recovered, None);
    let mut locked = h.store.pg.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(78422916)")
        .execute(&mut *locked)
        .await
        .unwrap();
    let before: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT last_succeeded_at FROM maintenance_checks WHERE name='retention'",
    )
    .fetch_one(&h.store.pg)
    .await
    .unwrap();
    click_server::jobs::enforce_retention(&h.store)
        .await
        .unwrap();
    let after: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT last_succeeded_at FROM maintenance_checks WHERE name='retention'",
    )
    .fetch_one(&h.store.pg)
    .await
    .unwrap();
    assert_eq!(before, after);
    locked.rollback().await.unwrap();
    // Token requests remain throttled even after tokens are consumed.
    let customer = h.client();
    h.register(&customer, "recipient@commercial.test").await;
    for _ in 0..6 {
        h.post(&customer, "/v1/account/reset/request")
            .json(&json!({"email":"recipient@commercial.test"}))
            .send()
            .await
            .unwrap();
        sqlx::query("UPDATE account_tokens SET used_at=now() WHERE user_id=(SELECT id FROM users WHERE email='recipient@commercial.test')").execute(&h.store.pg).await.unwrap();
    }
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM mail_outbox WHERE recipient='recipient@commercial.test'",
    )
    .fetch_one(&h.store.pg)
    .await
    .unwrap();
    assert_eq!(queued, 3);
    h.state.mail.drain_once(&h.store).await.unwrap();
    let retired:i64=sqlx::query_scalar("SELECT count(*) FROM mail_outbox WHERE recipient='recipient@commercial.test' AND state='expired' AND body IS NULL").fetch_one(&h.store.pg).await.unwrap();
    assert_eq!(retired, 3);

    h.finish().await;
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn pending_invitations_reserve_downgrade_seats_and_native_plans_cannot_be_overridden() {
    let h = Harness::new().await;
    let c = h.client();
    h.register(&c, "downgrade@commercial.test").await;
    let org = h.org(&c).await;
    assert_eq!(
        action(&h, &c, org, "billing_test", "pro").await.status(),
        StatusCode::OK
    );
    for n in 0..3 {
        assert!(h
            .post(&c, &format!("/v1/organizations/{org}/invitations"))
            .json(&json!({"email":format!("invite-{n}@commercial.test"),"role":"viewer"}))
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
    }
    assert_eq!(
        action(&h, &c, org, "billing_test", "free").await.status(),
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE invitations SET revoked_at=now() WHERE id=(SELECT id FROM invitations WHERE organization_id=$1 LIMIT 1)").bind(org).execute(&h.store.pg).await.unwrap();
    assert_eq!(
        action(&h, &c, org, "billing_test", "free").await.status(),
        StatusCode::OK
    );
    sqlx::query("UPDATE subscriptions SET provider='native' WHERE organization_id=$1")
        .bind(org)
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        action(&h, &c, org, "billing_test", "pro").await.status(),
        StatusCode::CONFLICT
    );
    let plan: String =
        sqlx::query_scalar("SELECT plan_id FROM subscriptions WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(plan, "free");
    h.finish().await;
}

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn billing_portal_rechecks_ownership_after_the_provider_returns() {
    let p = Arc::new(Provider::new());
    let h = Harness::provider(Some(p.clone())).await;
    let client = h.client();
    let account = h.register(&client, "portal-race@commercial.test").await;
    let org = h.org(&client).await;
    sqlx::query("UPDATE subscriptions SET provider='stripe',provider_customer='cus_fixture',provider_reference='sub_current',status='active',plan_id='pro' WHERE organization_id=$1").bind(org).execute(&h.store.pg).await.unwrap();
    p.portal_gate.store(true, Ordering::SeqCst);
    let request = h.post(&client, "/settings/action").form(&[
        ("action", "billing_portal".into()),
        ("organization_id", org.to_string()),
    ]);
    let call = tokio::spawn(async move { request.send().await.unwrap() });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        p.portal_started.notified(),
    )
    .await
    .unwrap();
    let user = Uuid::parse_str(account["id"].as_str().unwrap()).unwrap();
    let mut tx = h.store.pg.begin().await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE memberships SET role='viewer' WHERE organization_id=$1 AND user_id=$2")
        .bind(org)
        .bind(user)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    p.portal_release.notify_one();
    assert_eq!(call.await.unwrap().status(), StatusCode::NOT_FOUND);
    h.finish().await;
}

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn billing_pause_recovery_and_downgrade_preserve_accepted_event_retries() {
    let h = Harness::new().await;
    let c = h.client();
    h.register(&c, "billing-quota@commercial.test").await;
    let org = h.org(&c).await;
    let site = h.site(&c, org).await;
    let collection = site["collection_id"].as_str().unwrap();
    let event = json!({"event_id":Uuid::new_v4(),"session_id":Uuid::new_v4(),"name":"pageview","url":h.base});
    let send = |event: Value| {
        h.post(&c, &format!("/v1/collect/{collection}"))
            .json(&json!({"events":[event]}))
    };
    assert_eq!(
        send(event.clone()).send().await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    for status in ["past_due", "suspended"] {
        sqlx::query("UPDATE subscriptions SET status=$1 WHERE organization_id=$2")
            .bind(status)
            .bind(org)
            .execute(&h.store.pg)
            .await
            .unwrap();
        let retry = send(event.clone()).send().await.unwrap();
        assert_eq!(retry.status(), StatusCode::ACCEPTED);
        assert_eq!(retry.json::<Value>().await.unwrap()["duplicates"], 1);
        let fresh = json!({"event_id":Uuid::new_v4(),"session_id":Uuid::new_v4(),"name":"pageview","url":h.base});
        let refused = send(fresh).send().await.unwrap();
        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(refused
            .text()
            .await
            .unwrap()
            .contains("billing is resolved"));
    }
    sqlx::query("UPDATE subscriptions SET status='active' WHERE organization_id=$1")
        .bind(org)
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(send(json!({"event_id":Uuid::new_v4(),"session_id":Uuid::new_v4(),"name":"pageview","url":h.base})).send().await.unwrap().status(),StatusCode::ACCEPTED);
    sqlx::query("UPDATE usage_records SET accepted_events=9999999 WHERE organization_id=$1")
        .bind(org)
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        send(event).send().await.unwrap().status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(send(json!({"event_id":Uuid::new_v4(),"session_id":Uuid::new_v4(),"name":"pageview","url":h.base})).send().await.unwrap().status(),StatusCode::TOO_MANY_REQUESTS);
    let used: i64 =
        sqlx::query_scalar("SELECT accepted_events FROM usage_records WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(used, 9999999);
    h.finish().await;
}
