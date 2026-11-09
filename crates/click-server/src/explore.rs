//! Exact, bounded analytics for content, acquisition and retained visitor cohorts.
use crate::{analytics, api, auth, ApiError, AppResult, AppState};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::get,
    Json, Router,
};
use click_store::{Store, Website};
use clickhouse::Row;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/websites/{id}/comparison", get(comparison))
        .route("/v1/websites/{id}/events", get(events))
        .route("/v1/websites/{id}/content", get(content))
        .route("/v1/websites/{id}/acquisition", get(acquisition))
        .route("/v1/websites/{id}/retention", get(retention))
        .route("/v1/websites/{id}/vitals/comparison", get(vital_comparison))
}
fn scoped(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
    sql: &str,
) -> clickhouse::query::Query {
    store
        .ch
        .query(sql)
        .bind(site.organization_id.to_string())
        .bind(site.id.to_string())
        .bind(start)
        .bind(end)
}
/// Consecutive equal-duration windows, including the same observed amount of today.
pub fn previous(start: u32, end: u32) -> (u32, u32) {
    (start.saturating_sub(end - start), start)
}
#[derive(Debug, Serialize, Deserialize, Row)]
pub struct Snapshot {
    pub pageviews: u64,
    pub visitors: u64,
    pub sessions: u64,
    pub bounced_sessions: u64,
    pub visible_ms: u64,
    pub measured_sessions: u64,
}
impl Snapshot {
    pub fn bounce(&self) -> f64 {
        if self.sessions == 0 {
            0.0
        } else {
            self.bounced_sessions as f64 * 100.0 / self.sessions as f64
        }
    }
    pub fn visible_seconds(&self) -> Option<f64> {
        (self.measured_sessions > 0)
            .then(|| self.visible_ms as f64 / 1000.0 / self.measured_sessions as f64)
    }
}
pub async fn snapshot(store: &Store, site: &Website, start: u32, end: u32) -> AppResult<Snapshot> {
    scoped(store,site,start,end,"WITH base AS (SELECT * FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?)) SELECT pageviews,visitors,sessions,bounced_sessions,visible_ms,measured_sessions FROM (SELECT uniqExact(event_id) AS pageviews,uniqExact(visitor_id) AS visitors,uniqExact(session_id) AS sessions FROM base WHERE name='pageview') AS page_totals CROSS JOIN (SELECT countIf(views=1) AS bounced_sessions FROM (SELECT session_id,uniqExact(event_id) AS views FROM base WHERE name='pageview' GROUP BY session_id)) AS bounce_totals CROSS JOIN (SELECT sum(engagement_ms) AS visible_ms,uniqExact(session_id) AS measured_sessions FROM base WHERE name='web_engagement' AND session_id IN (SELECT session_id FROM base WHERE name='pageview')) AS engagement_totals")
        .fetch_one().await.map_err(ApiError::internal)
}
#[derive(Serialize, Deserialize)]
pub struct Comparison {
    pub start: u32,
    pub end: u32,
    pub previous_start: u32,
    pub previous_end: u32,
    pub current: Snapshot,
    pub previous: Snapshot,
}
pub async fn comparison_data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
) -> AppResult<Comparison> {
    let (a, b) = previous(start, end);
    Ok(Comparison {
        start,
        end,
        previous_start: a,
        previous_end: b,
        current: snapshot(store, site, start, end).await?,
        previous: snapshot(store, site, a, b).await?,
    })
}
async fn comparison(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<api::Period>,
) -> AppResult<Json<Comparison>> {
    let (site, _, _permit) = auth::authorize_report(&s, &h, id).await?;
    let (a, b) = api::range(&p)?;
    Ok(Json(comparison_data(&s.store, &site, a, b).await?))
}
#[derive(Debug, Serialize, Deserialize, Row)]
pub struct ContentRow {
    pub path: String,
    pub pageviews: u64,
    pub visitors: u64,
    pub sessions: u64,
    pub entries: u64,
    pub exits: u64,
    pub converting_sessions: u64,
    pub visible_ms: u64,
    pub measured_sessions: u64,
    pub mean_scroll_percent: Option<f64>,
}
pub async fn content_data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
) -> AppResult<Vec<ContentRow>> {
    let goals: Vec<String> = sqlx::query_scalar("SELECT event_name FROM goals WHERE website_id=$1")
        .bind(site.id)
        .fetch_all(&store.pg)
        .await?;
    let sql="WITH base AS (SELECT * FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?)), visits AS (SELECT session_id,argMin(path,(occurred_ms,event_id)) AS entry,argMax(path,(occurred_ms,event_id)) AS exit FROM base WHERE name='pageview' GROUP BY session_id), pages AS (SELECT path,uniqExact(event_id) AS pageviews,uniqExact(visitor_id) AS visitors,uniqExact(session_id) AS sessions,uniqExactIf(session_id,session_id IN (SELECT session_id FROM base WHERE name IN ?)) AS converting_sessions FROM base WHERE name='pageview' GROUP BY path), boundaries AS (SELECT path,countIf(entry=path) AS entries,countIf(exit=path) AS exits FROM (SELECT entry AS path,entry,exit FROM visits UNION ALL SELECT exit AS path,entry,exit FROM visits WHERE entry!=exit) GROUP BY path), engagement AS (SELECT path,sum(visible) AS visible_ms,count() AS measured_sessions,avg(toFloat64(depth)) AS depth FROM (SELECT path,session_id,sum(engagement_ms) AS visible,max(scroll_percent) AS depth FROM base WHERE name='web_engagement' AND (path,session_id) IN (SELECT path,session_id FROM base WHERE name='pageview') GROUP BY path,session_id) GROUP BY path) SELECT pages.path,pageviews,visitors,sessions,entries,exits,converting_sessions,visible_ms,measured_sessions,if(measured_sessions=0,NULL,depth) AS mean_scroll_percent FROM pages LEFT JOIN boundaries USING path LEFT JOIN engagement USING path ORDER BY pageviews DESC,pages.path LIMIT 100";
    scoped(store, site, start, end, sql)
        .bind(goals)
        .fetch_all()
        .await
        .map_err(ApiError::internal)
}
async fn content(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<api::Period>,
) -> AppResult<Json<Vec<ContentRow>>> {
    let (site, _, _permit) = auth::authorize_report(&s, &h, id).await?;
    let (a, b) = api::range(&p)?;
    Ok(Json(content_data(&s.store, &site, a, b).await?))
}
pub(crate) const CHANNEL:&str="multiIf(lower(medium) IN ('cpc','ppc','paid','paid_search'),'Paid search',lower(medium) IN ('paid_social','paidsocial'),'Paid social',lower(medium)='email','Email',lower(medium) IN ('affiliate','affiliates'),'Affiliates',lower(medium)='organic','Organic search',lower(medium)='social' OR source IN ('facebook.com','instagram.com','linkedin.com','t.co','twitter.com','x.com','youtube.com'),'Organic social',source='Direct' AND medium='','Direct',source IN ('google.com','bing.com','duckduckgo.com','search.yahoo.com'),'Organic search',medium='referral' OR source!='Direct','Referral','Other')";
#[derive(Deserialize)]
pub struct AcquisitionQuery {
    pub days: Option<u32>,
    pub dimension: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, Row)]
pub struct AcquisitionRow {
    pub label: String,
    pub sessions: u64,
    pub pageviews: u64,
    pub visitors: u64,
    pub converting_sessions: u64,
}
pub async fn acquisition_data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
    dimension: &str,
) -> AppResult<Vec<AcquisitionRow>> {
    let expression = match dimension {
        "source" => "source",
        "medium" => "if(medium='','Unspecified',medium)",
        "campaign" => "if(campaign='','No campaign',campaign)",
        "landing" => "landing",
        "channel" => CHANNEL,
        _ => {
            return Err(ApiError::bad(
                "Select source, medium, campaign, landing or channel",
            ))
        }
    };
    let goals: Vec<String> = sqlx::query_scalar("SELECT event_name FROM goals WHERE website_id=$1")
        .bind(site.id)
        .fetch_all(&store.pg)
        .await?;
    let sql=format!("WITH base AS (SELECT * FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?)) SELECT {expression} AS label,count() AS sessions,sum(views) AS pageviews,uniqExact(visitor) AS visitors,countIf(converting) AS converting_sessions FROM (SELECT session_id,uniqExact(event_id) AS views,argMin(visitor_id,(occurred_ms,event_id)) AS visitor,argMin(if(utm_source!='',utm_source,if(referrer='','Direct',referrer)),(occurred_ms,event_id)) AS source,argMin(utm_medium,(occurred_ms,event_id)) AS medium,argMin(utm_campaign,(occurred_ms,event_id)) AS campaign,argMin(path,(occurred_ms,event_id)) AS landing,session_id IN (SELECT session_id FROM base WHERE name IN ?) AS converting FROM base WHERE name='pageview' GROUP BY session_id) GROUP BY label ORDER BY sessions DESC,label LIMIT 100");
    scoped(store, site, start, end, &sql)
        .bind(goals)
        .fetch_all()
        .await
        .map_err(ApiError::internal)
}
async fn acquisition(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<AcquisitionQuery>,
) -> AppResult<Json<Vec<AcquisitionRow>>> {
    let (site, _, _permit) = auth::authorize_report(&s, &h, id).await?;
    let (a, b) = api::range(&api::Period { days: p.days })?;
    Ok(Json(
        acquisition_data(
            &s.store,
            &site,
            a,
            b,
            p.dimension.as_deref().unwrap_or("channel"),
        )
        .await?,
    ))
}
#[derive(Debug, Serialize, Deserialize, Row)]
pub struct IdentityCoverage {
    pub new_visitors: u64,
    pub returning_visitors: u64,
    pub session_only_sessions: u64,
    pub unclassified_sessions: u64,
}
#[derive(Deserialize, Row)]
struct CohortCell {
    cohort: String,
    week: u64,
    retained: u64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Cohort {
    pub week: String,
    pub visitors: u64,
    pub retained: Vec<Option<u64>>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct RetentionReport {
    pub history_start: u32,
    pub identity: IdentityCoverage,
    pub cohorts: Vec<Cohort>,
}
pub async fn retention_data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
) -> AppResult<RetentionReport> {
    let days:i32=sqlx::query_scalar("SELECT least(w.retention_days,p.retention_days) FROM websites w JOIN subscriptions s ON s.organization_id=w.organization_id JOIN plans p ON p.id=s.plan_id WHERE w.id=$1").bind(site.id).fetch_one(&store.pg).await?;
    let history_start = (chrono::Utc::now() - chrono::Duration::days(days as i64))
        .timestamp()
        .max(0) as u32;
    let first="SELECT visitor_id,min(timestamp) AS first_seen FROM events FINAL WHERE organization_id=? AND website_id=? AND name='pageview' AND visitor_mode='provided' AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) GROUP BY visitor_id";
    let sql=format!("WITH first AS ({first}),current AS (SELECT visitor_id FROM events FINAL WHERE organization_id=? AND website_id=? AND name='pageview' AND visitor_mode='provided' AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) GROUP BY visitor_id) SELECT countIf(first_seen>=toDateTime(?)) AS new_visitors,countIf(first_seen<toDateTime(?)) AS returning_visitors FROM first INNER JOIN current USING visitor_id");
    #[derive(Deserialize, Row)]
    struct NewReturning {
        new_visitors: u64,
        returning_visitors: u64,
    }
    let ids: NewReturning = scoped(store, site, history_start, end, &sql)
        .bind(site.organization_id.to_string())
        .bind(site.id.to_string())
        .bind(start)
        .bind(end)
        .bind(start)
        .bind(start)
        .fetch_one()
        .await
        .map_err(ApiError::internal)?;
    #[derive(Deserialize, Row)]
    struct Other {
        session_only_sessions: u64,
        unclassified_sessions: u64,
    }
    let other:Other=scoped(store,site,start,end,"SELECT uniqExactIf(session_id,visitor_mode='session') AS session_only_sessions,uniqExactIf(session_id,visitor_mode NOT IN ('session','provided')) AS unclassified_sessions FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) AND name='pageview'").fetch_one().await.map_err(ApiError::internal)?;
    let sql=format!("WITH first AS ({first}) SELECT toString(cohort_start) AS cohort,toUInt64(intDiv(dateDiff('day',cohort_start,active_week),7)) AS week,uniqExact(visitor_id) AS retained FROM (SELECT visitor_id,toMonday(first_seen) AS cohort_start FROM first WHERE first_seen>=toDateTime(?)) AS cohort_population INNER JOIN (SELECT visitor_id,toMonday(timestamp) AS active_week FROM events FINAL WHERE organization_id=? AND website_id=? AND name='pageview' AND visitor_mode='provided' AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) GROUP BY visitor_id,active_week) AS activity USING visitor_id WHERE active_week>=cohort_start AND dateDiff('day',cohort_start,active_week)<35 GROUP BY cohort_start,week ORDER BY cohort_start,week LIMIT 100");
    let cells: Vec<CohortCell> = scoped(store, site, history_start, end, &sql)
        .bind(start)
        .bind(site.organization_id.to_string())
        .bind(site.id.to_string())
        .bind(history_start)
        .bind(end)
        .fetch_all()
        .await
        .map_err(ApiError::internal)?;
    let mut cohorts = Vec::<Cohort>::new();
    for cell in &cells {
        if cell.week != 0 {
            continue;
        }
        let date = chrono::NaiveDate::parse_from_str(&cell.cohort, "%Y-%m-%d")
            .map_err(ApiError::internal)?;
        let mut retained = Vec::new();
        for week in 0..=4 {
            let complete = (date + chrono::Duration::days(7 * (week + 1)))
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp()
                <= end as i64;
            retained.push(if week > 0 && !complete {
                None
            } else {
                Some(
                    cells
                        .iter()
                        .find(|c| c.cohort == cell.cohort && c.week == week as u64)
                        .map(|c| c.retained)
                        .unwrap_or(0),
                )
            });
        }
        cohorts.push(Cohort {
            week: cell.cohort.clone(),
            visitors: cell.retained,
            retained,
        });
    }
    Ok(RetentionReport {
        history_start,
        identity: IdentityCoverage {
            new_visitors: ids.new_visitors,
            returning_visitors: ids.returning_visitors,
            session_only_sessions: other.session_only_sessions,
            unclassified_sessions: other.unclassified_sessions,
        },
        cohorts,
    })
}
async fn retention(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<api::Period>,
) -> AppResult<Json<RetentionReport>> {
    let (site, _, _permit) = auth::authorize_report(&s, &h, id).await?;
    let (a, b) = api::range(&p)?;
    Ok(Json(retention_data(&s.store, &site, a, b).await?))
}
#[derive(Serialize, Deserialize)]
pub struct VitalComparison {
    pub current: Vec<analytics::VitalReport>,
    pub previous: Vec<analytics::VitalReport>,
    pub start: u32,
    pub end: u32,
    pub previous_start: u32,
    pub previous_end: u32,
}
pub async fn vital_comparison_data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
) -> AppResult<VitalComparison> {
    let (a, b) = previous(start, end);
    Ok(VitalComparison {
        current: analytics::vital_data(store, site, start, end).await?,
        previous: analytics::vital_data(store, site, a, b).await?,
        start,
        end,
        previous_start: a,
        previous_end: b,
    })
}
async fn vital_comparison(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<api::Period>,
) -> AppResult<Json<VitalComparison>> {
    let (site, _, _permit) = auth::authorize_report(&s, &h, id).await?;
    let (a, b) = api::range(&p)?;
    Ok(Json(vital_comparison_data(&s.store, &site, a, b).await?))
}

#[derive(Clone, Default, Deserialize)]
pub struct EventQuery {
    pub days: Option<u32>,
    pub event: Option<String>,
    pub path: Option<String>,
    pub campaign: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, Row)]
pub struct EventDetail {
    pub event: String,
    pub path: String,
    pub campaign: String,
    pub count: u64,
    pub sessions: u64,
    pub visitors: u64,
}
pub async fn events_data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
    filter: &EventQuery,
) -> AppResult<Vec<EventDetail>> {
    let event = filter.event.as_deref().unwrap_or("");
    let path = filter.path.as_deref().unwrap_or("");
    let campaign = filter.campaign.as_deref().unwrap_or("");
    if event.len() > 64
        || !event
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        || !path.is_empty()
            && (!path.starts_with('/')
                || path.starts_with("//")
                || path.contains(['?', '#'])
                || path.len() > 1024
                || path.chars().any(char::is_control))
        || campaign.len() > 120
        || campaign.chars().any(char::is_control)
    {
        return Err(ApiError::bad(
            "Filter by a valid event name, local path or campaign label",
        ));
    }
    scoped(store,site,start,end,"SELECT name AS event,path,utm_campaign AS campaign,uniqExact(event_id) AS count,uniqExact(session_id) AS sessions,uniqExact(visitor_id) AS visitors FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) AND ((?='' AND name NOT IN ('pageview','web_vital','web_engagement')) OR name=?) AND (?='' OR path=?) AND (?='' OR utm_campaign=?) GROUP BY event,path,campaign ORDER BY count DESC,event,path,campaign LIMIT 100")
        .bind(event).bind(event).bind(path).bind(path).bind(campaign).bind(campaign).fetch_all().await.map_err(ApiError::internal)
}
async fn events(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<EventQuery>,
) -> AppResult<Json<Vec<EventDetail>>> {
    let (site, _, _permit) = auth::authorize_report(&s, &h, id).await?;
    let (a, b) = api::range(&api::Period { days: p.days })?;
    Ok(Json(events_data(&s.store, &site, a, b, &p).await?))
}
