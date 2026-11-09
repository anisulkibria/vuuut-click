//! Provider-neutral billing workflows and a Stripe HTTP adapter. No credential means no payment.
use crate::{auth, dashboard::esc, workspace, ApiError, AppResult, AppState};
use async_trait::async_trait;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use click_core::Role;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckoutRequest {
    pub attempt: Uuid,
    pub org: Uuid,
    pub email: String,
    pub plan: String,
    pub period: String,
    pub customer: Option<String>,
    pub return_url: String,
    pub expires_at: i64,
}
type PendingCheckout = (Uuid, String, String, Option<String>, Option<Value>);
pub struct Checkout {
    pub id: String,
    pub url: String,
}
pub struct Subscription {
    pub id: String,
    pub customer: String,
    pub price: String,
    pub status: String,
}
pub enum Change {
    Checkout {
        attempt: Uuid,
        session: String,
        subscription: String,
    },
    Subscription(String),
    Ignored,
}
pub struct ProviderEvent {
    pub id: String,
    pub kind: String,
    pub created: i64,
    pub change: Change,
}
#[async_trait]
pub trait PaymentProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, plan: &str, period: &str) -> bool;
    fn resolve_price(&self, price: &str) -> Option<(String, String)>;
    fn verify(&self, headers: &HeaderMap, body: &[u8]) -> AppResult<ProviderEvent>;
    async fn checkout(&self, request: CheckoutRequest) -> AppResult<Checkout>;
    async fn portal(&self, customer: &str, return_url: &str) -> AppResult<String>;
    async fn subscription(&self, id: &str) -> AppResult<Subscription>;
    async fn expire_checkout(&self, _id: &str) -> AppResult<()> {
        Err(ApiError::conflict(
            "Checkout cancellation is not supported by this provider",
        ))
    }
    fn mode(&self) -> &'static str {
        "unverified"
    }
    async fn validate_catalog(&self) -> AppResult<Value> {
        Err(ApiError::conflict(
            "Provider catalog validation is not available",
        ))
    }
}
pub struct Service {
    pub provider: Option<Arc<dyn PaymentProvider>>,
}
impl Service {
    pub fn from_env() -> anyhow::Result<Self> {
        let secret = std::env::var("STRIPE_SECRET_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        let Some(secret) = secret else {
            if ["STRIPE_WEBHOOK_SECRET", "STRIPE_PRICES_JSON"]
                .iter()
                .any(|key| std::env::var(key).is_ok_and(|v| !v.is_empty()))
            {
                anyhow::bail!("Stripe configuration requires STRIPE_SECRET_KEY, STRIPE_WEBHOOK_SECRET and STRIPE_PRICES_JSON together");
            }
            return Ok(Self { provider: None });
        };
        if secret.starts_with("sk_live_") && std::env::var("APP_ENV").as_deref() != Ok("production")
        {
            anyhow::bail!(
                "Stripe live keys require APP_ENV=production; use test keys for validation"
            );
        }
        if !secret.starts_with("sk_test_") && !secret.starts_with("sk_live_") {
            anyhow::bail!("STRIPE_SECRET_KEY must be a Stripe secret key");
        }
        let signing = std::env::var("STRIPE_WEBHOOK_SECRET").map_err(|_| {
            anyhow::anyhow!("STRIPE_WEBHOOK_SECRET is required when Stripe is enabled")
        })?;
        if !signing.starts_with("whsec_") {
            anyhow::bail!("Invalid Stripe webhook signing secret");
        }
        let prices: HashMap<String, String> = serde_json::from_str(
            &std::env::var("STRIPE_PRICES_JSON").unwrap_or_else(|_| "{}".into()),
        )?;
        validate_prices(&prices)?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            provider: Some(Arc::new(Stripe {
                api_base: "https://api.stripe.com/v1".into(),
                portal_configuration: std::env::var("STRIPE_PORTAL_CONFIGURATION")
                    .ok()
                    .filter(|v| !v.is_empty())
                    .map(|v| {
                        anyhow::ensure!(
                            v.starts_with("bpc_")
                                && v.len() <= 255
                                && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                            "Invalid Stripe portal configuration identifier"
                        );
                        Ok(v)
                    })
                    .transpose()?,
                live: secret.starts_with("sk_live_"),
                secret,
                signing,
                prices,
                http,
            })),
        })
    }
}
fn validate_prices(prices: &HashMap<String, String>) -> anyhow::Result<()> {
    let mut identifiers = std::collections::HashSet::new();
    anyhow::ensure!(
        !prices.is_empty(),
        "Configure at least one recurring Stripe price"
    );
    for (key, value) in prices {
        let (plan, period) = key
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("Price keys must be plan:monthly or plan:annual"))?;
        anyhow::ensure!(
            !plan.is_empty()
                && plan != "free"
                && plan
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
                && matches!(period, "monthly" | "annual"),
            "Invalid Stripe plan/interval mapping"
        );
        anyhow::ensure!(
            value.starts_with("price_")
                && value.len() <= 255
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && identifiers.insert(value),
            "Stripe prices must be unique price_ identifiers"
        );
    }
    Ok(())
}
struct Stripe {
    api_base: String,
    portal_configuration: Option<String>,
    secret: String,
    signing: String,
    prices: HashMap<String, String>,
    http: reqwest::Client,
    live: bool,
}
impl Stripe {
    async fn get(&self, path: &str) -> AppResult<Value> {
        let response = self
            .http
            .get(format!("{}/{path}", self.api_base))
            .bearer_auth(&self.secret)
            .header("Stripe-Version", "2025-02-24.acacia")
            .send()
            .await
            .map_err(ApiError::internal)?;
        if !response.status().is_success() {
            return Err(ApiError {
                status: StatusCode::BAD_GATEWAY,
                message: "Payment provider lookup failed",
            });
        }
        response.json().await.map_err(ApiError::internal)
    }
    async fn post(
        &self,
        path: &str,
        form: &[(String, String)],
        idempotency: Option<&str>,
    ) -> AppResult<Value> {
        let mut request = self
            .http
            .post(format!("{}/{path}", self.api_base))
            .bearer_auth(&self.secret)
            .header("Stripe-Version", "2025-02-24.acacia")
            .form(form);
        if let Some(key) = idempotency {
            request = request.header("Idempotency-Key", key);
        }
        let response = request.send().await.map_err(ApiError::internal)?;
        if !response.status().is_success() {
            tracing::error!(status=%response.status(),"payment provider rejected request");
            return Err(ApiError {
                status: StatusCode::BAD_GATEWAY,
                message: "Payment provider could not complete the request",
            });
        }
        response.json().await.map_err(ApiError::internal)
    }
    fn redirect(value: &Value, host: &str) -> AppResult<String> {
        let raw = value["url"]
            .as_str()
            .ok_or_else(|| ApiError::internal("Provider did not return a URL"))?;
        let url = url::Url::parse(raw).map_err(ApiError::internal)?;
        if url.scheme() != "https"
            || url.host_str() != Some(host)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return Err(ApiError::internal("Unexpected payment redirect host"));
        }
        Ok(raw.into())
    }
}
#[async_trait]
impl PaymentProvider for Stripe {
    fn name(&self) -> &'static str {
        "stripe"
    }
    fn mode(&self) -> &'static str {
        if self.live {
            "live"
        } else {
            "test"
        }
    }
    async fn validate_catalog(&self) -> AppResult<Value> {
        let mut checks = Vec::new();
        let mut mappings: Vec<_> = self.prices.iter().collect();
        mappings.sort();
        for (mapping, id) in mappings {
            let v = self.get(&format!("prices/{id}")).await?;
            let (_, period) = mapping
                .split_once(':')
                .ok_or_else(|| ApiError::bad("Invalid configured interval"))?;
            if v["id"].as_str() != Some(id.as_str())
                || v["active"] != true
                || v["livemode"].as_bool() != Some(self.live)
                || v["type"] != "recurring"
                || v["recurring"]["interval"].as_str()
                    != Some(if period == "annual" { "year" } else { "month" })
                || v["recurring"]["interval_count"] != 1
                || v["recurring"]["usage_type"] != "licensed"
                || v["billing_scheme"] != "per_unit"
                || v["unit_amount"].as_i64().is_none_or(|n| n <= 0)
            {
                return Err(ApiError::bad("Configured Stripe price is inactive, wrong mode, or not the expected fixed recurring interval"));
            }
            checks.push(serde_json::json!({"mapping":mapping,"currency":v["currency"],"amount_minor":v["unit_amount"],"interval":period,"active":true}));
        }
        let portals = if let Some(id) = &self.portal_configuration {
            serde_json::json!({"data":[self.get(&format!("billing_portal/configurations/{id}")).await?]})
        } else {
            self.get("billing_portal/configurations?active=true&is_default=true&limit=1")
                .await?
        };
        let portal = portals["data"].as_array().and_then(|rows| {
            rows.iter().find(|v| {
                v["active"] == true
                    && v["livemode"].as_bool() == Some(self.live)
                    && (self.portal_configuration.is_some() || v["is_default"] == true)
            })
        });
        let features = portal.map(|v| &v["features"]);
        let portal_ready = features.is_some_and(|f| {
            f["invoice_history"]["enabled"] == true
                && f["payment_method_update"]["enabled"] == true
                && f["subscription_cancel"]["enabled"] == true
                && f["subscription_update"]["enabled"] == true
        });
        Ok(
            serde_json::json!({"mode":self.mode(),"prices":checks,"default_portal_configured":portal.is_some_and(|p|p["is_default"]==true),"selected_portal_configured":portal.is_some(),"portal_features_ready":portal_ready}),
        )
    }

    async fn expire_checkout(&self, id: &str) -> AppResult<()> {
        if !id.starts_with("cs_") || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(ApiError::bad("Invalid checkout identifier"));
        }
        let current = self.get(&format!("checkout/sessions/{id}")).await?;
        match current["status"].as_str() {
            Some("expired") => Ok(()),
            Some("open") => {
                let result = self
                    .post(&format!("checkout/sessions/{id}/expire"), &[], None)
                    .await?;
                if result["status"] != "expired" {
                    return Err(ApiError::conflict(
                        "Provider has not acknowledged checkout cancellation",
                    ));
                }
                Ok(())
            }
            _ => Err(ApiError::conflict(
                "This checkout is already complete; manage the subscription through billing",
            )),
        }
    }
    fn supports(&self, plan: &str, period: &str) -> bool {
        self.prices.contains_key(&format!("{plan}:{period}"))
    }
    fn resolve_price(&self, price: &str) -> Option<(String, String)> {
        self.prices.iter().find_map(|(key, value)| {
            if value == price {
                key.split_once(':')
                    .map(|(p, i)| (p.to_owned(), i.to_owned()))
            } else {
                None
            }
        })
    }
    fn verify(&self, headers: &HeaderMap, body: &[u8]) -> AppResult<ProviderEvent> {
        let signature = headers
            .get("stripe-signature")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| ApiError::bad("Missing webhook signature"))?;
        verify_signature(
            &self.signing,
            signature,
            body,
            chrono::Utc::now().timestamp(),
        )?;
        let event: Value =
            serde_json::from_slice(body).map_err(|_| ApiError::bad("Invalid billing event"))?;
        if event["livemode"].as_bool() != Some(self.live) {
            return Err(ApiError::bad(
                "Webhook mode does not match provider configuration",
            ));
        }
        let object = &event["data"]["object"];
        let kind = event["type"]
            .as_str()
            .ok_or_else(|| ApiError::bad("Invalid billing event type"))?
            .to_owned();
        let change = match kind.as_str() {
            "checkout.session.completed" => {
                let attempt = object["metadata"]["checkout_attempt_id"]
                    .as_str()
                    .and_then(|s| Uuid::parse_str(s).ok());
                match (
                    attempt,
                    object["id"].as_str(),
                    object["subscription"].as_str(),
                ) {
                    (Some(attempt), Some(session), Some(subscription)) => Change::Checkout {
                        attempt,
                        session: session.into(),
                        subscription: subscription.into(),
                    },
                    _ => Change::Ignored,
                }
            }
            "customer.subscription.created"
            | "customer.subscription.updated"
            | "customer.subscription.deleted" => object["id"]
                .as_str()
                .map(|id| Change::Subscription(id.into()))
                .unwrap_or(Change::Ignored),
            _ => Change::Ignored,
        };
        Ok(ProviderEvent {
            id: event["id"]
                .as_str()
                .filter(|s| s.len() <= 255)
                .ok_or_else(|| ApiError::bad("Missing event identifier"))?
                .into(),
            kind,
            created: event["created"]
                .as_i64()
                .ok_or_else(|| ApiError::bad("Invalid event timestamp"))?,
            change,
        })
    }
    async fn checkout(&self, r: CheckoutRequest) -> AppResult<Checkout> {
        let price = self
            .prices
            .get(&format!("{}:{}", r.plan, r.period))
            .ok_or_else(|| ApiError::bad("This billing interval is not configured"))?;
        let mut form = vec![
            ("mode".into(), "subscription".into()),
            ("expires_at".into(), r.expires_at.to_string()),
            ("client_reference_id".into(), r.org.to_string()),
            ("line_items[0][price]".into(), price.clone()),
            ("line_items[0][quantity]".into(), "1".into()),
            ("success_url".into(), r.return_url.clone()),
            ("cancel_url".into(), r.return_url),
            (
                "metadata[checkout_attempt_id]".into(),
                r.attempt.to_string(),
            ),
            (
                "subscription_data[metadata][organization_id]".into(),
                r.org.to_string(),
            ),
        ];
        if let Some(customer) = r.customer {
            form.push(("customer".into(), customer));
        } else {
            form.push(("customer_email".into(), r.email));
        }
        let value = self
            .post("checkout/sessions", &form, Some(&r.attempt.to_string()))
            .await?;
        Ok(Checkout {
            id: value["id"]
                .as_str()
                .ok_or_else(|| ApiError::internal("Missing checkout identifier"))?
                .into(),
            url: Self::redirect(&value, "checkout.stripe.com")?,
        })
    }
    async fn portal(&self, customer: &str, return_url: &str) -> AppResult<String> {
        let mut form = vec![
            ("customer".into(), customer.into()),
            ("return_url".into(), return_url.into()),
        ];
        if let Some(id) = &self.portal_configuration {
            form.push(("configuration".into(), id.clone()));
        }
        let value = self.post("billing_portal/sessions", &form, None).await?;
        Self::redirect(&value, "billing.stripe.com")
    }
    async fn subscription(&self, id: &str) -> AppResult<Subscription> {
        if !id.starts_with("sub_") || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(ApiError::bad("Invalid provider subscription identifier"));
        }
        let response = self
            .http
            .get(format!("{}/subscriptions/{id}", self.api_base))
            .header("Stripe-Version", "2025-02-24.acacia")
            .bearer_auth(&self.secret)
            .send()
            .await
            .map_err(ApiError::internal)?
            .error_for_status()
            .map_err(ApiError::internal)?;
        let v: Value = response.json().await.map_err(ApiError::internal)?;
        if v["items"]["data"].as_array().is_none_or(|a| a.len() != 1) {
            return Err(ApiError::bad(
                "Subscription must contain one configured analytics price",
            ));
        }
        if v["items"]["data"][0]["quantity"].as_u64() != Some(1) {
            return Err(ApiError::bad(
                "Analytics subscriptions require quantity one",
            ));
        }
        Ok(Subscription {
            id: id.into(),
            customer: v["customer"]
                .as_str()
                .ok_or_else(|| ApiError::internal("Missing billing customer"))?
                .into(),
            price: v["items"]["data"][0]["price"]["id"]
                .as_str()
                .ok_or_else(|| ApiError::internal("Missing subscription price"))?
                .into(),
            status: v["status"].as_str().unwrap_or("incomplete").into(),
        })
    }
}
pub fn verify_signature(secret: &str, header: &str, body: &[u8], now: i64) -> AppResult<()> {
    let time = header
        .split(',')
        .find_map(|p| p.trim().strip_prefix("t="))
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or_else(|| ApiError::bad("Invalid webhook timestamp"))?;
    if now.abs_diff(time) > 300 {
        return Err(ApiError::bad(
            "Webhook signature timestamp is outside tolerance",
        ));
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(ApiError::internal)?;
    mac.update(format!("{time}.").as_bytes());
    mac.update(body);
    let valid = header
        .split(',')
        .filter_map(|p| p.trim().strip_prefix("v1="))
        .filter_map(|s| hex::decode(s).ok())
        .any(|bytes| mac.clone().verify_slice(&bytes).is_ok());
    if !valid {
        return Err(ApiError::bad("Webhook signature does not match"));
    }
    Ok(())
}
fn unavailable() -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: "Online billing is not configured; no payment has been made",
    }
}
pub async fn checkout(s: &AppState, h: &HeaderMap, org: Uuid, value: &str) -> AppResult<String> {
    auth::csrf(s, h)?;
    let (user, role) = workspace::membership(s, h, org, true).await?;
    if role != Role::Owner {
        return Err(ApiError::not_found());
    }
    let provider = s.billing.provider.as_ref().ok_or_else(unavailable)?;
    let (plan, period) = value
        .split_once(':')
        .ok_or_else(|| ApiError::bad("Select a plan and billing interval"))?;
    if plan == "free" || !matches!(period, "monthly" | "annual") || !provider.supports(plan, period)
    {
        return Err(ApiError::bad("Selected plan is not available for checkout"));
    }
    let mut tx = s.store.pg.begin().await?;
    // Ownership is re-checked, but the workspace's status deliberately is
    // not: a suspended or past-due workspace pays to recover, and refusing
    // checkout would trap it in the state it is trying to leave.
    workspace::lock_member_even_if_suspended(&mut tx, org, user.id, |role| role == Role::Owner)
        .await?;
    let paid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM subscriptions WHERE organization_id=$1 AND provider='stripe' AND provider_reference IS NOT NULL AND status NOT IN ('free','cancelled'))").bind(org).fetch_one(&mut *tx).await?;
    if paid {
        return Err(ApiError::conflict(
            "Manage your existing subscription through the billing portal",
        ));
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM plans WHERE id=$1)")
        .bind(plan)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Err(ApiError::not_found());
    }
    let managed: bool =
        sqlx::query_scalar("SELECT provider='native' FROM subscriptions WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    if managed {
        return Err(ApiError::conflict(
            "Manage this plan through your hosting platform",
        ));
    }
    let previous:Option<PendingCheckout>=sqlx::query_as("SELECT id,plan_id,billing_period,url,request_snapshot FROM billing_checkouts WHERE organization_id=$1 AND completed_at IS NULL AND cancelled_at IS NULL AND expires_at>now() ORDER BY created_at DESC LIMIT 1").bind(org).fetch_optional(&mut *tx).await?;
    let (attempt, request) = if let Some((attempt, old_plan, old_period, url, snapshot)) = previous
    {
        if old_plan != plan || old_period != period {
            return Err(ApiError::conflict(
                "Resume or cancel the pending checkout before selecting another plan",
            ));
        }
        if let Some(url) = url {
            return Ok(url);
        }
        let request = snapshot.ok_or_else(|| {
            ApiError::conflict("Legacy checkout is pending; cancel it or wait for expiry")
        })?;
        (
            attempt,
            serde_json::from_value::<CheckoutRequest>(request).map_err(ApiError::internal)?,
        )
    } else {
        let attempt = Uuid::new_v4();
        let customer: Option<String> = sqlx::query_scalar(
            "SELECT provider_customer FROM subscriptions WHERE organization_id=$1",
        )
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
        let expires_at = chrono::Utc::now().timestamp() + 3600;
        let request = CheckoutRequest {
            attempt,
            org,
            email: user.email,
            plan: plan.into(),
            period: period.into(),
            customer,
            return_url: format!("{}/settings?tab=billing&org={org}", s.config.public_url),
            expires_at,
        };
        sqlx::query("INSERT INTO billing_checkouts(id,organization_id,plan_id,billing_period,request_snapshot,expires_at) VALUES($1,$2,$3,$4,$5,to_timestamp($6))").bind(attempt).bind(org).bind(plan).bind(period).bind(serde_json::to_value(&request).map_err(ApiError::internal)?).bind(expires_at as f64).execute(&mut *tx).await?;
        (attempt, request)
    };
    tx.commit().await?;
    let result = provider.checkout(request).await?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_member_even_if_suspended(&mut tx, org, user.id, |r| r == Role::Owner).await?;
    let changed=sqlx::query("UPDATE billing_checkouts SET provider_reference=$1,url=$2 WHERE id=$3 AND completed_at IS NULL AND cancelled_at IS NULL AND expires_at>now()").bind(&result.id).bind(&result.url).bind(attempt).execute(&mut *tx).await?.rows_affected();
    if changed != 1 {
        return Err(ApiError::conflict(
            "Checkout is complete, expired or cancelled; refresh billing",
        ));
    }
    tx.commit().await?;
    Ok(result.url)
}

pub async fn cancel_checkout(s: &AppState, h: &HeaderMap, org: Uuid) -> AppResult<()> {
    auth::csrf(s, h)?;
    let (user, role) = workspace::membership(s, h, org, true).await?;
    if role != Role::Owner {
        return Err(ApiError::not_found());
    }
    let provider = s.billing.provider.as_ref().ok_or_else(unavailable)?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_member_even_if_suspended(&mut tx, org, user.id, |r| r == Role::Owner).await?;
    let row:Option<(Uuid,Option<String>,Option<Value>)>=sqlx::query_as("SELECT id,provider_reference,request_snapshot FROM billing_checkouts WHERE organization_id=$1 AND completed_at IS NULL AND cancelled_at IS NULL AND expires_at>now() ORDER BY created_at DESC LIMIT 1").bind(org).fetch_optional(&mut *tx).await?;
    tx.commit().await?;
    let Some((id, reference, snapshot)) = row else {
        return Ok(());
    };
    let reference = if let Some(reference) = reference {
        reference
    } else {
        // A lost creation response can still leave an open provider session.
        // Resolve the same idempotent request before claiming cancellation.
        let request = snapshot.ok_or_else(|| {
            ApiError::conflict("Legacy checkout has no provider reference; wait for expiry")
        })?;
        provider
            .checkout(serde_json::from_value(request).map_err(ApiError::internal)?)
            .await?
            .id
    };
    provider.expire_checkout(&reference).await?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_member_even_if_suspended(&mut tx, org, user.id, |r| r == Role::Owner).await?;
    sqlx::query("UPDATE billing_checkouts SET cancelled_at=now(),url=NULL WHERE id=$1 AND completed_at IS NULL").bind(id).execute(&mut *tx).await?;
    workspace::audit(&mut tx, org, user.id, "billing.checkout_cancelled", id).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn portal(s: &AppState, h: &HeaderMap, org: Uuid) -> AppResult<String> {
    auth::csrf(s, h)?;
    let (user, role) = workspace::membership(s, h, org, true).await?;
    if role != Role::Owner {
        return Err(ApiError::not_found());
    }
    let provider = s.billing.provider.as_ref().ok_or_else(unavailable)?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_member_even_if_suspended(&mut tx, org, user.id, |r| r == Role::Owner).await?;
    let customer: Option<String> = sqlx::query_scalar(
        "SELECT provider_customer FROM subscriptions WHERE organization_id=$1 AND provider=$2",
    )
    .bind(org)
    .bind(provider.name())
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    let customer =
        customer.ok_or_else(|| ApiError::bad("No provider subscription is linked yet"))?;
    tx.commit().await?;
    let url = provider
        .portal(
            &customer,
            &format!("{}/settings?tab=billing&org={org}", s.config.public_url),
        )
        .await?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_member_even_if_suspended(&mut tx, org, user.id, |r| r == Role::Owner).await?;
    let still_linked: bool = sqlx::query_scalar(
        "SELECT provider=$1 AND provider_customer=$2 FROM subscriptions WHERE organization_id=$3",
    )
    .bind(provider.name())
    .bind(&customer)
    .bind(org)
    .fetch_one(&mut *tx)
    .await?;
    if !still_linked {
        return Err(ApiError::conflict(
            "Billing customer changed; refresh billing",
        ));
    }
    tx.commit().await?;
    Ok(url)
}
pub async fn test_plan(s: &AppState, h: &HeaderMap, org: Uuid, plan: &str) -> AppResult<()> {
    auth::csrf(s, h)?;
    let (user, role) = workspace::membership(s, h, org, true).await?;
    if role != Role::Owner
        || !s.config.allow_localhost
        || s.config.secure_cookie
        || s.billing.provider.is_some()
    {
        return Err(ApiError::not_found());
    }
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_member_even_if_suspended(&mut tx, org, user.id, |r| r == Role::Owner).await?;
    let limits: Option<(i32, i32)> =
        sqlx::query_as("SELECT website_limit,seat_limit FROM plans WHERE id=$1")
            .bind(plan)
            .fetch_optional(&mut *tx)
            .await?;
    let (sites, seats) = limits.ok_or_else(ApiError::not_found)?;
    let used:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM websites WHERE organization_id=$1 AND NOT is_demo),(SELECT count(*) FROM memberships WHERE organization_id=$1)+(SELECT count(*) FROM invitations WHERE organization_id=$1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at>now())").bind(org).fetch_one(&mut *tx).await?;
    if used.0 > sites as i64 || used.1 > seats as i64 {
        return Err(ApiError::conflict(
            "Reduce websites or team seats before downgrading",
        ));
    }
    let updated=sqlx::query("UPDATE subscriptions SET plan_id=$1,provider='test',status='test' WHERE organization_id=$2 AND provider IN ('none','test')").bind(plan).bind(org).execute(&mut *tx).await?.rows_affected();
    if updated != 1 {
        return Err(ApiError::conflict(
            "Provider or hosting plans cannot be changed with test billing",
        ));
    }
    workspace::audit(&mut tx, org, user.id, "billing.test_plan_changed", org).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn settings(s: &AppState, _h: &HeaderMap, org: Uuid, role: Role) -> AppResult<String> {
    if role != Role::Owner {
        return Ok(
            "<section class=panel><p>Workspace owners manage subscriptions.</p></section>".into(),
        );
    }
    if let Some(provider) = &s.billing.provider {
        let (can_checkout,has_customer):(bool,bool)=sqlx::query_as("SELECT NOT(provider=$1 AND provider_reference IS NOT NULL AND status NOT IN ('free','cancelled')),provider=$1 AND provider_customer IS NOT NULL FROM subscriptions WHERE organization_id=$2").bind(provider.name()).bind(org).fetch_one(&s.store.pg).await?;
        let pending:Option<(String,String,chrono::DateTime<chrono::Utc>)>=sqlx::query_as("SELECT plan_id,billing_period,expires_at FROM billing_checkouts WHERE organization_id=$1 AND completed_at IS NULL AND cancelled_at IS NULL AND expires_at>now() ORDER BY created_at DESC LIMIT 1").bind(org).fetch_optional(&s.store.pg).await?;
        let pending=pending.map(|(plan,period,expiry)|format!(r#"<p>Pending checkout: {} · {}. Expires {} UTC. Resume the same selection below, or cancel before choosing another.</p><form method="post" action="/settings/action"><input type="hidden" name="action" value="cancel_checkout"><input type="hidden" name="organization_id" value="{org}"><button class="btn outlined small">Cancel pending checkout</button></form>"#,esc(&plan),esc(&period),expiry.format("%d %b %H:%M"))).unwrap_or_default();
        let mut buttons = String::new();
        let plans: Vec<String> = sqlx::query_scalar("SELECT id FROM plans ORDER BY monthly_events")
            .fetch_all(&s.store.pg)
            .await?;
        for plan in plans {
            for period in ["monthly", "annual"] {
                if can_checkout && provider.supports(&plan, period) {
                    buttons.push_str(&format!(r#"<form method="post" action="/settings/action"><input type="hidden" name="action" value="checkout"><input type="hidden" name="organization_id" value="{org}"><button class="btn primary small" name="value" value="{}:{period}">{} · {period} checkout</button></form>"#,esc(&plan),esc(&plan)));
                }
            }
        }
        let portal = if has_customer {
            format!(
                r#"<form method="post" action="/settings/action"><input type="hidden" name="action" value="billing_portal"><input type="hidden" name="organization_id" value="{org}"><button class="btn outlined small">Manage billing, invoices & cancellation</button></form>"#
            )
        } else {
            String::new()
        };
        Ok(format!(
            r#"<section class="panel"><h2>Manage your subscription</h2><p>Price and payment details are shown by the payment provider before you confirm.</p><p>Past-due payments pause collection. Cancelling returns to Free and its retention policy; older data may be removed.</p>{pending}<div class="settings-actions">{buttons}{portal}</div></section>"#
        ))
    } else if s.config.allow_localhost {
        Ok(format!(
            r#"<section class="panel"><h2>Development billing mode</h2><p>Online payments are not configured. You can explore plan limits locally; no card is charged and no payment is recorded.</p><form method="post" action="/settings/action" class="settings-form"><input type="hidden" name="action" value="billing_test"><input type="hidden" name="organization_id" value="{org}"><label>Test plan<select name="value"><option value="free">Free · test mode</option><option value="pro">Pro · test mode</option></select></label><button class="btn outlined small">Apply test plan</button></form></section>"#
        ))
    } else {
        Ok("<section class=panel><h2>Billing</h2><p>Online billing has not been configured by the operator. Your current allowance remains in effect.</p></section>".into())
    }
}
pub async fn webhook(
    State(s): State<AppState>,
    h: HeaderMap,
    body: Bytes,
) -> AppResult<StatusCode> {
    let provider = s.billing.provider.as_ref().ok_or_else(unavailable)?;
    let event = provider.verify(&h, &body)?;
    apply_event(&s, provider.as_ref(), event).await?;
    Ok(StatusCode::OK)
}
pub async fn apply_event(
    s: &AppState,
    provider: &dyn PaymentProvider,
    event: ProviderEvent,
) -> AppResult<()> {
    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM billing_events WHERE provider=$1 AND event_id=$2)",
    )
    .bind(provider.name())
    .bind(&event.id)
    .fetch_one(&s.store.pg)
    .await?;
    if duplicate {
        return Ok(());
    }
    let (org, subscription, attempt) = match &event.change {
        Change::Checkout {
            attempt,
            session,
            subscription,
        } => {
            let row: Option<(Uuid, Option<String>)> = sqlx::query_as(
                "SELECT organization_id,provider_reference FROM billing_checkouts WHERE id=$1 AND cancelled_at IS NULL",
            )
            .bind(attempt)
            .fetch_optional(&s.store.pg)
            .await?;
            let Some((org, expected)) = row else {
                return Ok(());
            };
            if expected.as_deref() != Some(session) {
                return Err(ApiError::conflict(
                    "Checkout mapping is not ready; retry webhook",
                ));
            }
            (org, subscription.clone(), Some(*attempt))
        }
        Change::Subscription(id) => {
            let org:Option<Uuid>=sqlx::query_scalar("SELECT organization_id FROM subscriptions WHERE provider=$1 AND provider_reference=$2").bind(provider.name()).bind(id).fetch_optional(&s.store.pg).await?;
            let Some(org) = org else { return Ok(()) };
            (org, id.clone(), None)
        }
        Change::Ignored => return Ok(()),
    };
    let mut tx = s.store.pg.begin().await?;
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM billing_events WHERE provider=$1 AND event_id=$2)",
    )
    .bind(provider.name())
    .bind(&event.id)
    .fetch_one(&mut *tx)
    .await?;
    if duplicate {
        return Ok(());
    }
    let linked:(String,Option<String>,String,Option<String>)=sqlx::query_as("SELECT provider,provider_reference,status,provider_customer FROM subscriptions WHERE organization_id=$1").bind(org).fetch_one(&mut *tx).await?;
    if linked.0 == "native" {
        return Err(ApiError::conflict("Hosting controls this subscription"));
    }
    if linked.0 == provider.name()
        && linked.1.as_deref() != Some(subscription.as_str())
        && !matches!(linked.2.as_str(), "free" | "cancelled")
    {
        return Err(ApiError::conflict(
            "A different provider subscription is already linked; operator review required",
        ));
    }
    let current = provider.subscription(&subscription).await?;
    if current.id != subscription || linked.3.as_ref().is_some_and(|id| id != &current.customer) {
        return Err(ApiError::conflict(
            "Provider subscription/customer does not match this workspace",
        ));
    }
    let status = match current.status.as_str() {
        "active" | "trialing" => "active",
        "past_due" | "unpaid" => "past_due",
        "canceled" | "incomplete_expired" => "cancelled",
        _ => "suspended",
    };
    let (plan, period) = if status == "cancelled" {
        ("free".into(), "monthly".into())
    } else {
        provider
            .resolve_price(&current.price)
            .ok_or_else(|| ApiError::bad("Subscription price is not an analytics plan"))?
    };
    let target_plan = if status == "cancelled" { "free" } else { &plan };
    sqlx::query("UPDATE subscriptions SET plan_id=$1,provider=$2,provider_reference=$3,provider_customer=$4,status=$5,billing_period=$6,provider_event_created=greatest(provider_event_created,$7) WHERE organization_id=$8").bind(target_plan).bind(provider.name()).bind(&current.id).bind(&current.customer).bind(if status=="cancelled"{"free"}else{status}).bind(period).bind(event.created).bind(org).execute(&mut *tx).await?;
    if let Some(attempt) = attempt {
        sqlx::query("UPDATE billing_checkouts SET completed_at=now(),url=NULL WHERE id=$1")
            .bind(attempt)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("INSERT INTO billing_events(provider,event_id,event_type) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(provider.name()).bind(&event.id).bind(&event.kind).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_logs(organization_id,action) VALUES($1,'billing.provider_subscription_updated')").bind(org).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_reject_tampering_expiry_and_accept_rotation() {
        let body = b"{\"id\":\"evt_test\"}";
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(b"1000.");
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());
        let header = format!("t=1000,v1=bad,v1={sig}");
        assert!(verify_signature("secret", &header, body, 1001).is_ok());
        assert!(verify_signature("secret", &header, b"forged", 1001).is_err());
        assert!(verify_signature("secret", &header, body, 1400).is_err());
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use axum::{extract::State, routing::any, Router};
    use std::sync::Mutex;
    type WireRequest = (String, HeaderMap, String);
    #[derive(Clone)]
    struct Fixture {
        requests: Arc<Mutex<Vec<WireRequest>>>,
        bad_price: Arc<std::sync::atomic::AtomicBool>,
    }
    async fn handle(
        State(f): State<Fixture>,
        uri: axum::http::Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        let path = uri.path().to_owned();
        f.requests.lock().unwrap().push((
            path.clone(),
            headers,
            String::from_utf8(body.to_vec()).unwrap(),
        ));
        let v = if path == "/checkout/sessions" {
            serde_json::json!({"id":"cs_fixture","url":"https://checkout.stripe.com/fixture"})
        } else if path == "/billing_portal/sessions" {
            serde_json::json!({"url":"https://billing.stripe.com/fixture"})
        } else if path.starts_with("/billing_portal/configurations") {
            serde_json::json!({"data":[{"is_default":true,"active":true,"livemode":false,"features":{"invoice_history":{"enabled":true},"payment_method_update":{"enabled":true},"subscription_cancel":{"enabled":true},"subscription_update":{"enabled":true}}}]})
        } else if path == "/checkout/sessions/cs_fixture/expire" {
            serde_json::json!({"status":"expired"})
        } else if path == "/checkout/sessions/cs_fixture" {
            serde_json::json!({"status":"open"})
        } else if path.starts_with("/prices/") {
            serde_json::json!({"id":"price_fixture","active":!f.bad_price.load(std::sync::atomic::Ordering::SeqCst),"livemode":false,"type":"recurring","unit_amount":500,"currency":"usd","billing_scheme":"per_unit","recurring":{"interval":"month","interval_count":1,"usage_type":"licensed"}})
        } else if path == "/subscriptions/sub_fixture" {
            serde_json::json!({"id":"sub_fixture","customer":"cus_fixture","status":"active","items":{"data":[{"quantity":1,"price":{"id":"price_fixture"}}]}})
        } else {
            return (
                StatusCode::BAD_GATEWAY,
                "secret provider detail must not reach clients",
            )
                .into_response();
        };
        axum::Json(v).into_response()
    }
    #[tokio::test]
    async fn stripe_wire_contract_and_catalog_use_pinned_requests_and_validated_redirects() {
        let fixture = Fixture {
            requests: Arc::default(),
            bad_price: Arc::new(false.into()),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .fallback(any(handle))
            .with_state(fixture.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let stripe = Stripe {
            api_base: base,
            portal_configuration: None,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            secret: "sk_test_fixture".into(),
            signing: "whsec_fixture".into(),
            prices: HashMap::from([("pro:monthly".into(), "price_fixture".into())]),
            live: false,
        };
        let attempt = Uuid::new_v4();
        let org = Uuid::new_v4();
        let expires_at = chrono::Utc::now().timestamp() + 3600;
        let r = CheckoutRequest {
            attempt,
            org,
            email: "fixture@example.test".into(),
            plan: "pro".into(),
            period: "monthly".into(),
            customer: None,
            return_url: "https://analytics.example/settings?tab=billing".into(),
            expires_at,
        };
        assert_eq!(stripe.checkout(r).await.unwrap().id, "cs_fixture");
        assert!(stripe
            .portal("cus_fixture", "https://analytics.example/settings")
            .await
            .is_ok());
        assert_eq!(
            stripe.subscription("sub_fixture").await.unwrap().customer,
            "cus_fixture"
        );
        assert!(stripe.subscription("sub_bad/path").await.is_err());
        stripe.expire_checkout("cs_fixture").await.unwrap();
        let catalog = stripe.validate_catalog().await.unwrap();
        assert_eq!(catalog["default_portal_configured"], true);
        assert_eq!(catalog["mode"], "test");
        fixture
            .bad_price
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(stripe.validate_catalog().await.is_err());
        let requests = fixture.requests.lock().unwrap();
        let (_, headers, body) = &requests[0];
        assert_eq!(headers["stripe-version"], "2025-02-24.acacia");
        assert_eq!(headers["idempotency-key"], attempt.to_string());
        let form: HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(form["expires_at"], expires_at.to_string());
        assert_eq!(form["metadata[checkout_attempt_id]"], attempt.to_string());
        assert_eq!(
            form["subscription_data[metadata][organization_id]"],
            org.to_string()
        );
        assert_eq!(form["line_items[0][quantity]"], "1");
        for raw in [
            "http://checkout.stripe.com/x",
            "https://evil.test/",
            "https://name:secret@checkout.stripe.com/x",
            "https://checkout.stripe.com:8443/x",
        ] {
            assert!(
                Stripe::redirect(&serde_json::json!({"url":raw}), "checkout.stripe.com").is_err()
            );
        }
        server.abort();
    }
    #[test]
    fn price_maps_and_signed_mode_are_strict() {
        assert!(validate_prices(&HashMap::new()).is_err());
        for map in [
            HashMap::from([("free:monthly".into(), "price_a".into())]),
            HashMap::from([("pro:weekly".into(), "price_a".into())]),
            HashMap::from([
                ("pro:monthly".into(), "price_a".into()),
                ("pro:annual".into(), "price_a".into()),
            ]),
        ] {
            assert!(validate_prices(&map).is_err());
        }
        let stripe = Stripe {
            api_base: "https://api.stripe.com/v1".into(),
            portal_configuration: None,
            http: reqwest::Client::new(),
            secret: "sk_test_fixture".into(),
            signing: "whsec_fixture".into(),
            prices: HashMap::from([("pro:monthly".into(), "price_fixture".into())]),
            live: false,
        };
        for (live, valid) in [(false, true), (true, false)] {
            let time = chrono::Utc::now().timestamp();
            let body=serde_json::to_vec(&serde_json::json!({"id":"evt_fixture","created":time,"type":"customer.subscription.updated","livemode":live,"data":{"object":{"id":"sub_fixture"}}})).unwrap();
            let mut mac = Hmac::<Sha256>::new_from_slice(b"whsec_fixture").unwrap();
            mac.update(format!("{time}.").as_bytes());
            mac.update(&body);
            let mut headers = HeaderMap::new();
            headers.insert(
                "stripe-signature",
                format!("t={time},v1={}", hex::encode(mac.finalize().into_bytes()))
                    .parse()
                    .unwrap(),
            );
            assert_eq!(stripe.verify(&headers, &body).is_ok(), valid);
        }
    }
}
