mod support;
use async_trait::async_trait;
use axum::http::HeaderMap;
use click_server::{billing::*, ApiError, AppResult};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use support::Harness;

struct Provider {
    status: Mutex<String>,
    checkout_fail: AtomicBool,
    lookup_fail: AtomicBool,
    lookups: AtomicUsize,
    attempts: Mutex<Vec<uuid::Uuid>>,
}
#[async_trait]
impl PaymentProvider for Provider {
    fn name(&self) -> &'static str {
        "stripe"
    }
    fn supports(&self, plan: &str, period: &str) -> bool {
        plan == "pro" && period == "monthly"
    }
    fn resolve_price(&self, price: &str) -> Option<(String, String)> {
        (price == "price_pro").then(|| ("pro".into(), "monthly".into()))
    }
    fn verify(&self, h: &HeaderMap, body: &[u8]) -> AppResult<ProviderEvent> {
        if h.get("x-test-signature").and_then(|v| v.to_str().ok()) != Some("verified") {
            return Err(ApiError::bad("Invalid signature"));
        }
        let v: Value = serde_json::from_slice(body).map_err(ApiError::internal)?;
        Ok(ProviderEvent {
            id: v["id"].as_str().unwrap().into(),
            kind: "test.subscription".into(),
            created: v["created"].as_i64().unwrap(),
            change: if let Some(attempt) = v["attempt"].as_str() {
                Change::Checkout {
                    attempt: attempt.parse().unwrap(),
                    session: "cs_demo".into(),
                    subscription: "sub_demo".into(),
                }
            } else {
                Change::Subscription(v["subscription"].as_str().unwrap_or("sub_demo").into())
            },
        })
    }
    async fn checkout(&self, r: CheckoutRequest) -> AppResult<Checkout> {
        self.attempts.lock().unwrap().push(r.attempt);
        if self.checkout_fail.swap(false, Ordering::SeqCst) {
            return Err(ApiError {
                status: StatusCode::BAD_GATEWAY,
                message: "Temporary provider failure",
            });
        }
        Ok(Checkout {
            id: "cs_demo".into(),
            url: "https://checkout.stripe.com/demo".into(),
        })
    }
    async fn portal(&self, customer: &str, _: &str) -> AppResult<String> {
        assert_eq!(customer, "cus_demo");
        Ok("https://billing.stripe.com/demo".into())
    }
    async fn subscription(&self, id: &str) -> AppResult<Subscription> {
        assert_eq!(id, "sub_demo");
        self.lookups.fetch_add(1, Ordering::SeqCst);
        if self.lookup_fail.swap(false, Ordering::SeqCst) {
            return Err(ApiError {
                status: StatusCode::BAD_GATEWAY,
                message: "Temporary provider failure",
            });
        }
        Ok(Subscription {
            id: id.into(),
            customer: "cus_demo".into(),
            price: "price_pro".into(),
            status: self.status.lock().unwrap().clone(),
        })
    }
}

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn billing_retries_verified_events_and_owner_actions() {
    let provider = Arc::new(Provider {
        status: Mutex::new("active".into()),
        checkout_fail: AtomicBool::new(true),
        lookup_fail: AtomicBool::new(false),
        lookups: AtomicUsize::new(0),
        attempts: Mutex::new(Vec::new()),
    });
    let h = Harness::provider(Some(provider.clone())).await;
    let customer = h.client();
    h.register(&customer, "customer@billing.test").await;
    let org = h.org(&customer).await;
    let site = h.site(&customer, org).await;
    let checkout = [
        ("action", "checkout".to_owned()),
        ("organization_id", org.to_string()),
        ("value", "pro:monthly".to_owned()),
    ];
    assert_eq!(
        h.post(&customer, "/settings/action")
            .form(&checkout)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_GATEWAY
    );
    let response = h
        .post(&customer, "/settings/action")
        .form(&checkout)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()["location"],
        "https://checkout.stripe.com/demo"
    );
    let attempts = provider.attempts.lock().unwrap().clone();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0], attempts[1]);
    let plan: String =
        sqlx::query_scalar("SELECT plan_id FROM subscriptions WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(plan, "free");
    let event = json!({"id":"evt_checkout","created":1000,"attempt":attempts[0]});
    assert_eq!(
        customer
            .post(format!("{}/v1/billing/webhooks/stripe", h.base))
            .json(&event)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    provider.lookup_fail.store(true, Ordering::SeqCst);
    let webhook = |body: Value| {
        customer
            .post(format!("{}/v1/billing/webhooks/stripe", h.base))
            .header("x-test-signature", "verified")
            .json(&body)
    };
    assert_eq!(
        webhook(event.clone()).send().await.unwrap().status(),
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(
        webhook(event.clone()).send().await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        webhook(event).send().await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(provider.lookups.load(Ordering::SeqCst), 2);
    let usage: Value = customer
        .get(format!("{}/v1/organizations/{org}/usage", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(usage["plan"], "pro");
    assert_eq!(usage["billing_provider"], "stripe");
    // A delayed notification retrieves authoritative current state rather than replaying stale status.
    *provider.status.lock().unwrap() = "past_due".into();
    assert_eq!(
        webhook(json!({"id":"evt_delayed","created":900}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let status: String =
        sqlx::query_scalar("SELECT status FROM subscriptions WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(status, "past_due");
    *provider.status.lock().unwrap() = "canceled".into();
    assert_eq!(
        webhook(json!({"id":"evt_cancel","created":1100}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let plan: String =
        sqlx::query_scalar("SELECT plan_id FROM subscriptions WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(plan, "free");
    let owner = h.client();
    h.register(&owner, "operator@billing.test").await;
    sqlx::query("INSERT INTO platform_owners(user_id) SELECT id FROM users WHERE email='operator@billing.test'").execute(&h.store.pg).await.unwrap();
    assert_eq!(
        h.post(&owner, "/owner/auth/login")
            .form(&[
                ("email", "operator@billing.test"),
                ("password", "correct-horse-battery-staple")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    let suspend = [
        ("action", "status".to_owned()),
        ("organization_id", org.to_string()),
        ("value", "suspended".to_owned()),
    ];
    assert_eq!(
        h.post(&customer, "/owner/actions")
            .form(&suspend)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post(&owner, "/owner/actions")
            .form(&suspend)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    assert!(h
        .store
        .collect_site(site["collection_id"].as_str().unwrap())
        .await
        .unwrap()
        .is_none());
    let grant = [
        ("action", "plan".to_owned()),
        ("organization_id", org.to_string()),
        ("value", "pro".to_owned()),
    ];
    assert_eq!(
        h.post(&owner, "/owner/actions")
            .form(&grant)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let activate = [
        ("action", "status".to_owned()),
        ("organization_id", org.to_string()),
        ("value", "active".to_owned()),
    ];
    assert_eq!(
        h.post(&owner, "/owner/actions")
            .form(&activate)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    assert!(h
        .store
        .collect_site(site["collection_id"].as_str().unwrap())
        .await
        .unwrap()
        .is_some());
    let audit = owner
        .get(format!("{}/owner/audit", h.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(audit.contains("workspace.status.suspended"));
    assert!(audit.contains("workspace.status.active"));
    h.finish().await;
}
