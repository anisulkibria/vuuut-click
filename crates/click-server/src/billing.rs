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

pub struct CheckoutRequest {
    pub attempt: Uuid,
    pub org: Uuid,
    pub email: String,
    pub plan: String,
    pub period: String,
    pub customer: Option<String>,
    pub return_url: String,
}
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
            return Ok(Self { provider: None });
        };
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
        if prices.values().any(|p| !p.starts_with("price_")) {
            anyhow::bail!("Stripe plan prices must contain price_ identifiers");
        }
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            provider: Some(Arc::new(Stripe {
                live: secret.starts_with("sk_live_"),
                secret,
                signing,
                prices,
                http,
            })),
        })
    }
}
struct Stripe {
    secret: String,
    signing: String,
    prices: HashMap<String, String>,
    http: reqwest::Client,
    live: bool,
}
impl Stripe {
    async fn post(
        &self,
        path: &str,
        form: &[(String, String)],
        idempotency: Option<&str>,
    ) -> AppResult<Value> {
        let mut request = self
            .http
            .post(format!("https://api.stripe.com/v1/{path}"))
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
        if url.scheme() != "https" || url.host_str() != Some(host) {
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
        let value = self
            .post(
                "billing_portal/sessions",
                &[
                    ("customer".into(), customer.into()),
                    ("return_url".into(), return_url.into()),
                ],
                None,
            )
            .await?;
        Self::redirect(&value, "billing.stripe.com")
    }
    async fn subscription(&self, id: &str) -> AppResult<Subscription> {
        if !id.starts_with("sub_") || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(ApiError::bad("Invalid provider subscription identifier"));
        }
        let response = self
            .http
            .get(format!("https://api.stripe.com/v1/subscriptions/{id}"))
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
    let paid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM subscriptions WHERE organization_id=$1 AND provider='stripe' AND provider_reference IS NOT NULL AND status IN ('active','past_due'))").bind(org).fetch_one(&mut *tx).await?;
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
    let previous:Option<(Uuid,Option<String>)>=sqlx::query_as("SELECT id,url FROM billing_checkouts WHERE organization_id=$1 AND plan_id=$2 AND billing_period=$3 AND created_at>now()-interval '25 minutes' AND completed_at IS NULL ORDER BY created_at DESC LIMIT 1").bind(org).bind(plan).bind(period).fetch_optional(&mut *tx).await?;
    let attempt = if let Some((id, url)) = previous {
        if let Some(url) = url {
            return Ok(url);
        }
        id
    } else {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO billing_checkouts(id,organization_id,plan_id,billing_period) VALUES($1,$2,$3,$4)").bind(id).bind(org).bind(plan).bind(period).execute(&mut *tx).await?;
        id
    };
    let customer: Option<String> =
        sqlx::query_scalar("SELECT provider_customer FROM subscriptions WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    tx.commit().await?;
    let result = provider
        .checkout(CheckoutRequest {
            attempt,
            org,
            email: user.email,
            plan: plan.into(),
            period: period.into(),
            customer,
            return_url: format!("{}/settings?tab=billing&org={org}", s.config.public_url),
        })
        .await?;
    sqlx::query("UPDATE billing_checkouts SET provider_reference=$1,url=$2 WHERE id=$3")
        .bind(&result.id)
        .bind(&result.url)
        .bind(attempt)
        .execute(&s.store.pg)
        .await?;
    Ok(result.url)
}
pub async fn portal(s: &AppState, h: &HeaderMap, org: Uuid) -> AppResult<String> {
    auth::csrf(s, h)?;
    let (_, role) = workspace::membership(s, h, org, true).await?;
    if role != Role::Owner {
        return Err(ApiError::not_found());
    }
    let provider = s.billing.provider.as_ref().ok_or_else(unavailable)?;
    let customer: Option<String> = sqlx::query_scalar(
        "SELECT provider_customer FROM subscriptions WHERE organization_id=$1 AND provider=$2",
    )
    .bind(org)
    .bind(provider.name())
    .fetch_optional(&s.store.pg)
    .await?
    .flatten();
    let customer =
        customer.ok_or_else(|| ApiError::bad("No provider subscription is linked yet"))?;
    provider
        .portal(
            &customer,
            &format!("{}/settings?tab=billing&org={org}", s.config.public_url),
        )
        .await
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
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
    let limits: Option<(i32, i32)> =
        sqlx::query_as("SELECT website_limit,seat_limit FROM plans WHERE id=$1")
            .bind(plan)
            .fetch_optional(&mut *tx)
            .await?;
    let (sites, seats) = limits.ok_or_else(ApiError::not_found)?;
    let used:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM websites WHERE organization_id=$1 AND NOT is_demo),(SELECT count(*) FROM memberships WHERE organization_id=$1)").bind(org).fetch_one(&mut *tx).await?;
    if used.0 > sites as i64 || used.1 > seats as i64 {
        return Err(ApiError::conflict(
            "Reduce websites or team seats before downgrading",
        ));
    }
    sqlx::query("UPDATE subscriptions SET plan_id=$1,provider='test',status='test' WHERE organization_id=$2 AND provider IN ('none','test')").bind(plan).bind(org).execute(&mut *tx).await?;
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
        let mut buttons = String::new();
        let plans: Vec<String> = sqlx::query_scalar("SELECT id FROM plans ORDER BY monthly_events")
            .fetch_all(&s.store.pg)
            .await?;
        for plan in plans {
            for period in ["monthly", "annual"] {
                if provider.supports(&plan, period) {
                    buttons.push_str(&format!(r#"<form method="post" action="/settings/action"><input type="hidden" name="action" value="checkout"><input type="hidden" name="organization_id" value="{org}"><button class="btn primary small" name="value" value="{}:{period}">{} · {period} checkout</button></form>"#,esc(&plan),esc(&plan)));
                }
            }
        }
        Ok(format!(
            r#"<section class="panel"><h2>Manage your subscription</h2><p>Price and payment details are shown by the payment provider before you confirm.</p><div class="settings-actions">{buttons}<form method="post" action="/settings/action"><input type="hidden" name="action" value="billing_portal"><input type="hidden" name="organization_id" value="{org}"><button class="btn outlined small">Manage billing, invoices & cancellation</button></form></div></section>"#
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
                "SELECT organization_id,provider_reference FROM billing_checkouts WHERE id=$1",
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
    let current = provider.subscription(&subscription).await?;
    let (plan, period) = provider
        .resolve_price(&current.price)
        .ok_or_else(|| ApiError::bad("Subscription price is not an analytics plan"))?;
    let status = match current.status.as_str() {
        "active" | "trialing" => "active",
        "past_due" | "unpaid" => "past_due",
        "canceled" | "incomplete_expired" => "cancelled",
        _ => "suspended",
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
