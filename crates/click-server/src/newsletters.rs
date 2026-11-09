//! Newsletter send cohorts. No rates are inferred from missing or anonymous facts.
use crate::{api, auth, ApiError, AppResult, AppState};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use click_store::{Store, Website};
use clickhouse::Row;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Serialize, Deserialize, Row)]
pub struct NewsletterEdition {
    pub newsletter_id: String,
    pub sent_recipients: u64,
    pub opened_recipients: u64,
    pub clicked_recipients: u64,
    pub link_click_facts: u64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct NewsletterReport {
    pub editions: Vec<NewsletterEdition>,
    pub unassigned_events: u64,
    pub activity_outside_send_cohort: u64,
}
pub async fn data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
) -> AppResult<NewsletterReport> {
    let sql="WITH sends AS (SELECT newsletter_id,recipient_id,min(occurred_ms) AS sent_at FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) AND name='server.newsletter_sent' AND newsletter_id!='' AND recipient_id!='' GROUP BY newsletter_id,recipient_id) SELECT sends.newsletter_id,uniqExact(sends.recipient_id) AS sent_recipients,uniqExactIf(sends.recipient_id,activity.name='server.newsletter_open' AND activity.occurred_ms>=sends.sent_at) AS opened_recipients,uniqExactIf(sends.recipient_id,activity.name='server.newsletter_click' AND activity.occurred_ms>=sends.sent_at) AS clicked_recipients,uniqExactIf(activity.event_id,activity.name='server.newsletter_click' AND activity.occurred_ms>=sends.sent_at) AS link_click_facts FROM sends LEFT JOIN (SELECT newsletter_id,recipient_id,event_id,name,occurred_ms FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) AND name IN ('server.newsletter_open','server.newsletter_click')) AS activity USING (newsletter_id,recipient_id) GROUP BY sends.newsletter_id ORDER BY sent_recipients DESC,sends.newsletter_id LIMIT 100";
    let days:i32=sqlx::query_scalar("SELECT least(w.retention_days,p.retention_days) FROM websites w JOIN subscriptions s ON s.organization_id=w.organization_id JOIN plans p ON p.id=s.plan_id WHERE w.id=$1").bind(site.id).fetch_one(&store.pg).await?;
    let history = (chrono::Utc::now() - chrono::Duration::days(days as i64))
        .timestamp()
        .max(0) as u32;
    let editions = store
        .ch
        .query(sql)
        .bind(site.organization_id.to_string())
        .bind(site.id.to_string())
        .bind(start)
        .bind(end)
        .bind(site.organization_id.to_string())
        .bind(site.id.to_string())
        .bind(history)
        .bind(end)
        .fetch_all()
        .await
        .map_err(ApiError::internal)?;
    #[derive(Deserialize, Row)]
    struct Coverage {
        unassigned_events: u64,
        activity_outside_send_cohort: u64,
    }
    let coverage:Coverage=store.ch.query("WITH base AS (SELECT * FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?)) SELECT uniqExactIf(event_id,newsletter_id='' OR recipient_id='') AS unassigned_events,uniqExactIf(event_id,newsletter_id!='' AND recipient_id!='' AND name IN ('server.newsletter_open','server.newsletter_click') AND (newsletter_id,recipient_id) NOT IN (SELECT newsletter_id,recipient_id FROM base WHERE name='server.newsletter_sent')) AS activity_outside_send_cohort FROM base WHERE name IN ('server.newsletter_sent','server.newsletter_open','server.newsletter_click')").bind(site.organization_id.to_string()).bind(site.id.to_string()).bind(start).bind(end).fetch_one().await.map_err(ApiError::internal)?;
    Ok(NewsletterReport {
        editions,
        unassigned_events: coverage.unassigned_events,
        activity_outside_send_cohort: coverage.activity_outside_send_cohort,
    })
}
pub async fn report(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<api::Period>,
) -> AppResult<Json<NewsletterReport>> {
    let (site, _, _permit) = auth::authorize_report(&s, &h, id).await?;
    let (a, b) = api::range(&p)?;
    Ok(Json(data(&s.store, &site, a, b).await?))
}
