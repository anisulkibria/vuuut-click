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
