//! Typed analytics and backend event client. Keep credentials server-side.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct Client {
    http: reqwest::Client,
    base: url::Url,
    key: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Overview {
    pub pageviews: u64,
    pub visitors: u64,
    pub sessions: u64,
    pub realtime: u64,
    pub bounce_rate: f64,
    pub timeseries: Vec<Breakdown>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Breakdown {
    pub label: String,
    pub count: u64,
}
#[derive(Clone, Copy)]
pub enum Dimension {
    Countries,
    Pages,
    Referrers,
    Campaigns,
    Devices,
    Browsers,
    Events,
}
impl Client {
    pub fn new(base: &str, key: String) -> Result<Self, url::ParseError> {
        let base = url::Url::parse(base)?;
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("Rustls client"),
            base,
            key,
        })
    }
    pub async fn overview(&self, website: Uuid, days: u32) -> Result<Overview, reqwest::Error> {
        self.http
            .get(
                self.base
                    .join(&format!("/v1/websites/{website}/overview"))
                    .expect("fixed API path"),
            )
            .bearer_auth(&self.key)
            .query(&[("days", days)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
    pub async fn breakdown(
        &self,
        website: Uuid,
        dimension: Dimension,
        days: u32,
    ) -> Result<Vec<Breakdown>, reqwest::Error> {
        let dim = match dimension {
            Dimension::Countries => "countries",
            Dimension::Pages => "pages",
            Dimension::Referrers => "referrers",
            Dimension::Campaigns => "campaigns",
            Dimension::Devices => "devices",
            Dimension::Browsers => "browsers",
            Dimension::Events => "events",
        };
        self.http
            .get(
                self.base
                    .join(&format!("/v1/websites/{website}/breakdown/{dim}"))
                    .expect("fixed API path"),
            )
            .bearer_auth(&self.key)
            .query(&[("days", days)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        days: u32,
    ) -> Result<T, reqwest::Error> {
        self.http
            .get(self.base.join(path).expect("fixed API path"))
            .bearer_auth(&self.key)
            .query(&[("days", days)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
    pub async fn funnels(&self, website: Uuid) -> Result<Vec<Funnel>, reqwest::Error> {
        self.get(&format!("/v1/websites/{website}/funnels"), 7)
            .await
    }
    pub async fn funnel(
        &self,
        website: Uuid,
        funnel: Uuid,
        days: u32,
    ) -> Result<FunnelReport, reqwest::Error> {
        self.get(&format!("/v1/websites/{website}/funnels/{funnel}"), days)
            .await
    }
    pub async fn journeys(&self, website: Uuid, days: u32) -> Result<Vec<Journey>, reqwest::Error> {
        self.get(&format!("/v1/websites/{website}/journeys"), days)
            .await
    }
    pub async fn vitals(&self, website: Uuid, days: u32) -> Result<Vec<Vital>, reqwest::Error> {
        self.get(&format!("/v1/websites/{website}/vitals"), days)
            .await
    }
    pub async fn revenue(
        &self,
        website: Uuid,
        days: u32,
        model: Attribution,
        dimension: RevenueDimension,
    ) -> Result<Vec<Revenue>, reqwest::Error> {
        let model = match model {
            Attribution::FirstTouch => "first_touch",
            Attribution::LastTouch => "last_touch",
            Attribution::LastNonDirect => "last_non_direct",
        };
        let dimension = match dimension {
            RevenueDimension::Source => "source",
            RevenueDimension::Campaign => "campaign",
            RevenueDimension::Landing => "landing",
        };
        self.http
            .get(
                self.base
                    .join(&format!("/v1/websites/{website}/revenue"))
                    .expect("fixed API path"),
            )
            .bearer_auth(&self.key)
            .query(&[
                ("days", days.to_string()),
                ("model", model.into()),
                ("dimension", dimension.into()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
    /// Retry with the same external_id and kind; acceptance and usage are idempotent.
    pub async fn backend_event(
        &self,
        website: Uuid,
        event: &BackendEvent,
    ) -> Result<Acceptance, reqwest::Error> {
        self.http
            .post(
                self.base
                    .join(&format!("/v1/websites/{website}/backend-events"))
                    .expect("fixed API path"),
            )
            .bearer_auth(&self.key)
            .json(event)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunnelStep {
    pub kind: String,
    pub value: String,
}
#[derive(Debug, Deserialize)]
pub struct Funnel {
    pub id: Uuid,
    pub name: String,
    pub steps: Vec<FunnelStep>,
    pub window_seconds: i32,
}
#[derive(Debug, Deserialize)]
pub struct FunnelResult {
    pub kind: String,
    pub value: String,
    pub sessions: u64,
    pub conversion_rate: f64,
    pub drop_off: u64,
}
#[derive(Debug, Deserialize)]
pub struct FunnelReport {
    pub id: Uuid,
    pub name: String,
    pub window_seconds: i32,
    pub steps: Vec<FunnelResult>,
}
#[derive(Debug, Deserialize)]
pub struct Journey {
    pub session_id: String,
    pub country: String,
    pub events: u64,
    pub steps: Vec<(i64, String, String)>,
}
#[derive(Debug, Deserialize)]
pub struct Vital {
    pub metric: String,
    pub samples: u64,
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
}
#[derive(Debug, Deserialize)]
pub struct Revenue {
    pub label: String,
    pub currency: String,
    pub total_minor: String,
    pub payments: u64,
}
pub enum Attribution {
    FirstTouch,
    LastTouch,
    LastNonDirect,
}
pub enum RevenueDimension {
    Source,
    Campaign,
    Landing,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Payment,
    Refund,
    MemberSignup,
    SubscriptionStarted,
    SubscriptionCancelled,
    NewsletterSent,
    NewsletterOpen,
    NewsletterClick,
}
#[derive(Debug, Serialize)]
pub struct BackendEvent {
    pub external_id: String,
    pub kind: BackendKind,
    pub session_id: Option<Uuid>,
    pub visitor_id: Option<Uuid>,
    pub path: Option<String>,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub occurred_at_ms: Option<i64>,
}
#[derive(Debug, Deserialize)]
pub struct Acceptance {
    pub event_id: Uuid,
    pub accepted: usize,
    pub duplicates: usize,
}

/// A monotonically versioned desired tenant snapshot. Reuse the same revision
/// and exact body until acknowledged; bump it for any subsequent change.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeTenant {
    pub revision: i64,
    pub name: String,
    pub origin: String,
    pub plan_id: String,
    pub state: NativeState,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeState {
    Active,
    Suspended,
    Deleted,
}
#[derive(Deserialize)]
pub struct NativeProvisioned {
    pub website_id: Option<Uuid>,
    pub collection_id: Option<String>,
    pub integration_key: Option<String>,
    pub verification_token: Option<String>,
    pub verified: bool,
    pub revision: i64,
    pub state: NativeState,
}
#[derive(Deserialize)]
pub struct NativeAccess {
    pub access_token: String,
    pub expires_in: u32,
    pub website_id: Uuid,
}
impl Client {
    /// Use an operator-issued platform key for this method only. Tenant names
    /// are encoded as one URL segment; they cannot select another endpoint.
    pub async fn provision_native(
        &self,
        tenant: &str,
        desired: &NativeTenant,
    ) -> Result<NativeProvisioned, reqwest::Error> {
        let mut url = self.base.join("/native/v1/tenants/").expect("fixed path");
        url.path_segments_mut()
            .expect("HTTP origin")
            .pop_if_empty()
            .push(tenant);
        self.http
            .put(url)
            .bearer_auth(&self.key)
            .json(desired)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
    /// Send a minimized native fact with an authenticated raw body and send
    /// timestamp. Retry with the original external_id and occurred_at_ms.
    pub async fn native_event(
        &self,
        website: Uuid,
        event: &BackendEvent,
    ) -> Result<Acceptance, reqwest::Error> {
        use hmac::{Hmac, Mac};
        let body = serde_json::to_vec(event).expect("typed backend event is serializable");
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_secs();
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(self.key.as_bytes()).expect("HMAC key");
        mac.update(format!("{time}.").as_bytes());
        mac.update(&body);
        let signature = format!("t={time},v1={}", hex::encode(mac.finalize().into_bytes()));
        self.http
            .post(
                self.base
                    .join(&format!("/native/v1/websites/{website}/events"))
                    .expect("fixed path"),
            )
            .bearer_auth(&self.key)
            .header("content-type", "application/json")
            .header("x-click-signature", signature)
            .body(body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
    /// Pin the CMS issuer and public signing keys using a website integration
    /// credential. Click never discovers an issuer supplied in a staff JWT.
    pub async fn connect_native(
        &self,
        website: Uuid,
        issuer: &str,
        jwks: &serde_json::Value,
    ) -> Result<(), reqwest::Error> {
        self.http
            .post(
                self.base
                    .join(&format!("/native/v1/websites/{website}/connect"))
                    .expect("fixed path"),
            )
            .bearer_auth(&self.key)
            .json(&serde_json::json!({"issuer":issuer,"jwks":jwks}))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    pub async fn exchange_native(
        &self,
        website: Uuid,
        assertion: &str,
    ) -> Result<NativeAccess, reqwest::Error> {
        self.http
            .post(
                self.base
                    .join(&format!("/native/v1/websites/{website}/exchange"))
                    .expect("fixed path"),
            )
            .bearer_auth(&self.key)
            .json(&serde_json::json!({"token":assertion}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
    /// Persist the randomly generated replacement before calling, then retry
    /// with that key if the response is lost. Existing staff access is revoked.
    pub async fn rotate_native(
        &self,
        website: Uuid,
        replacement: &str,
    ) -> Result<(), reqwest::Error> {
        self.http
            .post(
                self.base
                    .join(&format!("/native/v1/websites/{website}/rotate"))
                    .expect("fixed path"),
            )
            .bearer_auth(&self.key)
            .header("x-click-replacement-key", replacement)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    pub async fn disconnect_native(&self, website: Uuid) -> Result<(), reqwest::Error> {
        self.http
            .post(
                self.base
                    .join(&format!("/native/v1/websites/{website}/disconnect"))
                    .expect("fixed path"),
            )
            .bearer_auth(&self.key)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct NativeReport {
    pub activity: Vec<Breakdown>,
    pub page: Option<NativePage>,
}
#[derive(Debug, Deserialize)]
pub struct NativePage {
    pub path: String,
    pub pageviews: u64,
    pub visitors: u64,
    pub sessions: u64,
}
impl Client {
    /// Exact CMS activity and optional per-post traffic, including slash aliases.
    pub async fn native_report(
        &self,
        website: Uuid,
        days: u32,
        path: Option<&str>,
    ) -> Result<NativeReport, reqwest::Error> {
        let mut request = self
            .http
            .get(
                self.base
                    .join(&format!("/v1/websites/{website}/native-report"))
                    .expect("fixed path"),
            )
            .bearer_auth(&self.key)
            .query(&[("days", days)]);
        if let Some(path) = path {
            request = request.query(&[("path", path)]);
        }
        request.send().await?.error_for_status()?.json().await
    }
}
