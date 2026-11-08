//! Funnel, journey, performance and authoritative revenue reports.
use crate::{api, auth, workspace, ApiError, AppResult, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use click_store::{Store, Website};
use clickhouse::Row;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunnelStep {
    pub kind: String,
    pub value: String,
}
#[derive(Deserialize)]
pub struct FunnelInput {
    pub name: String,
    pub steps: Vec<FunnelStep>,
    #[serde(default = "default_window")]
    pub window_seconds: i32,
}
fn default_window() -> i32 {
    86400
}
#[derive(Serialize, sqlx::FromRow)]
pub struct Funnel {
    pub id: Uuid,
    pub name: String,
    pub steps: serde_json::Value,
    pub window_seconds: i32,
}
fn validate(input: &FunnelInput) -> AppResult<()> {
    if input.name.trim().is_empty()
        || input.name.len() > 100
        || !(2..=8).contains(&input.steps.len())
        || !(60..=604800).contains(&input.window_seconds)
    {
        return Err(ApiError::bad(
            "A funnel needs a name, 2–8 steps and a window between 1 minute and 7 days",
        ));
    }
    for step in &input.steps {
        let valid = match step.kind.as_str() {
            "page" => {
                step.value.starts_with('/')
                    && step.value.len() <= 1024
                    && !step.value.contains(['?', '#'])
                    && !step.value.chars().any(char::is_control)
            }
            "event" => {
                !step.value.is_empty()
                    && step.value.len() <= 64
                    && step
                        .value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
            }
            _ => false,
        };
        if !valid {
            return Err(ApiError::bad(
                "Funnel steps must be a page path or an event name",
            ));
        }
    }
    Ok(())
}
pub async fn create_funnel(
    s: &AppState,
    h: &HeaderMap,
    id: Uuid,
    input: FunnelInput,
) -> AppResult<Uuid> {
    auth::csrf(s, h)?;
    validate(&input)?;
    let (site, role) = auth::authorize_site(s, h, id, false).await?;
    if !role.can_edit_goals() {
        return Err(ApiError::not_found());
    }
    let u = auth::user(s, h).await?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_editor(&mut tx, site.organization_id, u.id).await?;
    let limits:(i64,i32)=sqlx::query_as("SELECT (SELECT count(*) FROM funnels WHERE website_id=$1),p.funnel_limit FROM subscriptions s JOIN plans p ON p.id=s.plan_id WHERE s.organization_id=$2").bind(id).bind(site.organization_id).fetch_one(&mut *tx).await?;
    if limits.0 >= limits.1 as i64 {
        return Err(ApiError::conflict("Funnel allowance reached"));
    }
    let funnel = Uuid::new_v4();
    let steps = serde_json::to_value(&input.steps).map_err(ApiError::internal)?;
    let result = sqlx::query(
        "INSERT INTO funnels(id,website_id,name,steps,window_seconds) VALUES($1,$2,$3,$4,$5)",
    )
    .bind(funnel)
    .bind(id)
    .bind(input.name.trim())
    .bind(steps)
    .bind(input.window_seconds)
    .execute(&mut *tx)
    .await;
    if let Err(e) = result {
        if e.as_database_error()
            .is_some_and(|e| e.is_unique_violation())
        {
            return Err(ApiError::conflict("A funnel with this name already exists"));
        }
        return Err(e.into());
    }
    workspace::audit(
        &mut tx,
        site.organization_id,
        u.id,
        "funnel.created",
        funnel,
    )
    .await?;
    tx.commit().await?;
    Ok(funnel)
}
pub async fn create(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<FunnelInput>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({"id":create_funnel(&s,&h,id,input).await?})),
    ))
}
pub async fn list(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<Funnel>>> {
    auth::authorize_site(&s, &h, id, false).await?;
    Ok(Json(sqlx::query_as("SELECT id,name,steps,window_seconds FROM funnels WHERE website_id=$1 ORDER BY created_at LIMIT 100").bind(id).fetch_all(&s.store.pg).await?))
}
/// Change a funnel's name, steps or conversion window.
///
/// Editing the steps is the point: a funnel is a sequence of page paths and
/// events, and the first attempt at one is rarely the right one. Previously
/// the only way to correct a step was to delete the funnel and build it
/// again. Steps are stored, not derived, so a rewrite only changes what the
/// report measures from here on — no stored event is touched.
pub async fn update_funnel(
    s: &AppState,
    h: &HeaderMap,
    id: Uuid,
    funnel: Uuid,
    input: FunnelInput,
) -> AppResult<()> {
    auth::csrf(s, h)?;
    validate(&input)?;
    let (site, role) = auth::authorize_site(s, h, id, false).await?;
    if !role.can_edit_goals() {
        return Err(ApiError::not_found());
    }
    let u = auth::user(s, h).await?;
    let steps = serde_json::to_value(&input.steps).map_err(ApiError::internal)?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_editor(&mut tx, site.organization_id, u.id).await?;
    // Scoped by website as well as id: a funnel id from another site must not
    // be reachable through a site this membership authorizes.
    let result = sqlx::query(
        "UPDATE funnels SET name=$1,steps=$2,window_seconds=$3 WHERE id=$4 AND website_id=$5",
    )
    .bind(input.name.trim())
    .bind(steps)
    .bind(input.window_seconds)
    .bind(funnel)
    .bind(site.id)
    .execute(&mut *tx)
    .await;
    match result {
        Ok(done) if done.rows_affected() == 0 => return Err(ApiError::not_found()),
        Ok(_) => {}
        Err(e) => {
            if e.as_database_error()
                .is_some_and(|e| e.is_unique_violation())
            {
                return Err(ApiError::conflict("A funnel with this name already exists"));
            }
            return Err(e.into());
        }
    }
    workspace::audit(
        &mut tx,
        site.organization_id,
        u.id,
        "funnel.updated",
        funnel,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn update(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, funnel)): Path<(Uuid, Uuid)>,
    Json(input): Json<FunnelInput>,
) -> AppResult<StatusCode> {
    update_funnel(&s, &h, id, funnel, input).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_funnel(s: &AppState, h: &HeaderMap, site: Uuid, funnel: Uuid) -> AppResult<()> {
    auth::csrf(s, h)?;
    let (site, role) = auth::authorize_site(s, h, site, false).await?;
    if !role.can_edit_goals() {
        return Err(ApiError::not_found());
    }
    let u = auth::user(s, h).await?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_editor(&mut tx, site.organization_id, u.id).await?;
    let result = sqlx::query("DELETE FROM funnels WHERE id=$1 AND website_id=$2")
        .bind(funnel)
        .bind(site.id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    workspace::audit(
        &mut tx,
        site.organization_id,
        u.id,
        "funnel.deleted",
        funnel,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn remove(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((site, funnel)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    remove_funnel(&s, &h, site, funnel).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Serialize)]
pub struct FunnelReport {
    pub id: Uuid,
    pub name: String,
    pub window_seconds: i32,
    pub steps: Vec<FunnelResult>,
}
#[derive(Serialize)]
pub struct FunnelResult {
    pub kind: String,
    pub value: String,
    pub sessions: u64,
    pub conversion_rate: f64,
    pub drop_off: u64,
}
#[derive(Deserialize, Row)]
struct LevelCount {
    level: u64,
    count: u64,
}
pub async fn funnel_report(
    store: &Store,
    site: &Website,
    funnel: Funnel,
    start: u32,
    end: u32,
) -> AppResult<FunnelReport> {
    let steps: Vec<FunnelStep> =
        serde_json::from_value(funnel.steps).map_err(ApiError::internal)?;
    validate(&FunnelInput {
        name: funnel.name.clone(),
        steps: steps.clone(),
        window_seconds: funnel.window_seconds,
    })?;
    let conditions = steps
        .iter()
        .map(|step| {
            if step.kind == "page" {
                "(name='pageview' AND path=?)"
            } else {
                "name=?"
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let sql=format!("SELECT level,count() AS count FROM (SELECT session_id,toUInt64(windowFunnel(?, 'strict_increase')(toUInt64(occurred_ms),{conditions})) AS level FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) GROUP BY session_id) GROUP BY level");
    let mut query = store
        .ch
        .query(&sql)
        .bind(funnel.window_seconds as u64 * 1000);
    for step in &steps {
        query = query.bind(&step.value);
    }
    let levels: Vec<LevelCount> = query
        .bind(site.organization_id.to_string())
        .bind(site.id.to_string())
        .bind(start)
        .bind(end)
        .fetch_all()
        .await
        .map_err(ApiError::internal)?;
    let counts: Vec<u64> = (1..=steps.len())
        .map(|step| {
            levels
                .iter()
                .filter(|r| r.level >= step as u64)
                .map(|r| r.count)
                .sum()
        })
        .collect();
    let first = counts.first().copied().unwrap_or(0);
    let mut results = Vec::new();
    for (index, step) in steps.into_iter().enumerate() {
        let previous = if index == 0 {
            counts[index]
        } else {
            counts[index - 1]
        };
        results.push(FunnelResult {
            kind: step.kind,
            value: step.value,
            sessions: counts[index],
            conversion_rate: if first == 0 {
                0.0
            } else {
                counts[index] as f64 * 100.0 / first as f64
            },
            drop_off: previous.saturating_sub(counts[index]),
        });
    }
    Ok(FunnelReport {
        id: funnel.id,
        name: funnel.name,
        window_seconds: funnel.window_seconds,
        steps: results,
    })
}
pub async fn report(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, funnel)): Path<(Uuid, Uuid)>,
    Query(p): Query<api::Period>,
) -> AppResult<Json<FunnelReport>> {
    let (site, _, _queries) = auth::authorize_report(&s, &h, id).await?;
    let funnel: Funnel = sqlx::query_as(
        "SELECT id,name,steps,window_seconds FROM funnels WHERE id=$1 AND website_id=$2",
    )
    .bind(funnel)
    .bind(id)
    .fetch_optional(&s.store.pg)
    .await?
    .ok_or_else(ApiError::not_found)?;
    let (start, end) = api::range(&p)?;
    Ok(Json(
        funnel_report(&s.store, &site, funnel, start, end).await?,
    ))
}

#[derive(Serialize, Deserialize, Row)]
pub struct Journey {
    pub session_id: String,
    pub country: String,
    pub events: u64,
    pub steps: Vec<(i64, String, String)>,
}
#[derive(Deserialize, Row)]
struct SessionId {
    session_id: String,
}
/// The most recent sessions, with their steps.
///
/// The step array is bounded *by the aggregate*, not sliced afterwards:
/// `groupArraySorted(100)` keeps a hundred elements in its state, where
/// `arraySlice(arraySort(groupArray(…)),1,100)` built the session's entire
/// history in memory first and then threw almost all of it away. For an
/// ordinary session the two are identical — verified equal across every
/// session in the development dataset — but a session with hundreds of
/// thousands of events is the case that decides how much memory one report
/// can ask for.
pub async fn journey_data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
) -> AppResult<Vec<Journey>> {
    let sessions:Vec<SessionId>=store.ch.query("SELECT session_id FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) AND name!='web_vital' GROUP BY session_id ORDER BY max(occurred_ms) DESC,session_id LIMIT 25").bind(site.organization_id.to_string()).bind(site.id.to_string()).bind(start).bind(end).fetch_all().await.map_err(ApiError::internal)?;
    if sessions.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = sessions.into_iter().map(|s| s.session_id).collect();
    store.ch.query("SELECT session_id,argMin(country,occurred_ms) AS country,count() AS events,groupArraySorted(100)((occurred_ms,path,name)) AS steps FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) AND name!='web_vital' AND session_id IN ? GROUP BY session_id ORDER BY max(occurred_ms) DESC,session_id").bind(site.organization_id.to_string()).bind(site.id.to_string()).bind(start).bind(end).bind(ids).fetch_all().await.map_err(ApiError::internal)
}
pub async fn journeys(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<api::Period>,
) -> AppResult<Json<Vec<Journey>>> {
    let (site, _, _queries) = auth::authorize_report(&s, &h, id).await?;
    let (start, end) = api::range(&p)?;
    Ok(Json(journey_data(&s.store, &site, start, end).await?))
}
#[derive(Serialize, Deserialize, Row)]
pub struct VitalReport {
    pub metric: String,
    pub samples: u64,
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
}
pub async fn vital_data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
) -> AppResult<Vec<VitalReport>> {
    store.ch.query("SELECT metric_name AS metric,count() AS samples,quantileExact(0.5)(latest_value) AS p50,quantileExact(0.75)(latest_value) AS p75,quantileExact(0.95)(latest_value) AS p95 FROM (SELECT metric_name,argMax(metric_value,occurred_ms) AS latest_value FROM events FINAL WHERE organization_id=? AND website_id=? AND name='web_vital' AND metric_name!='' AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) GROUP BY metric_name,metric_id) GROUP BY metric ORDER BY metric").bind(site.organization_id.to_string()).bind(site.id.to_string()).bind(start).bind(end).fetch_all().await.map_err(ApiError::internal)
}
pub async fn vitals(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<api::Period>,
) -> AppResult<Json<Vec<VitalReport>>> {
    let (site, _, _queries) = auth::authorize_report(&s, &h, id).await?;
    let (start, end) = api::range(&p)?;
    Ok(Json(vital_data(&s.store, &site, start, end).await?))
}
#[derive(Serialize, Deserialize, Row)]
pub struct Revenue {
    pub label: String,
    pub currency: String,
    pub total_minor: String,
    pub payments: u64,
}
#[derive(Deserialize)]
pub struct RevenueQuery {
    pub days: Option<u32>,
    pub model: Option<String>,
    pub dimension: Option<String>,
}
pub async fn revenue_data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
    model: &str,
    dimension: &str,
) -> AppResult<Vec<Revenue>> {
    let touch=match model{"first_touch"=>"touchpoints[1]","last_touch"=>"touchpoints[-1]","last_non_direct"=>"if(empty(arrayFilter(t->t.2!='Direct',touchpoints)),touchpoints[-1],arrayFilter(t->t.2!='Direct',touchpoints)[-1])",_=>return Err(ApiError::bad("Select a valid attribution model"))};
    let (index, fallback) = match dimension {
        "source" => (2, "Direct"),
        "campaign" => (3, "No campaign"),
        "landing" => (4, "Unattributed"),
        _ => {
            return Err(ApiError::bad(
                "Select source, campaign or landing page attribution",
            ))
        }
    };
    let label =
        format!("if(empty(touchpoints) OR ({touch}).{index}='', '{fallback}', ({touch}).{index})");
    let sql=format!("SELECT {label} AS label,currency,toString(sum(toDecimal128(amount_minor,0))) AS total_minor,countIf(amount_minor>0) AS payments FROM (SELECT purchases.*,arrayFilter(t->t.1<=purchases.occurred_ms AND t.1>=purchases.occurred_ms-2592000000,traffic.touches) AS touchpoints FROM (SELECT visitor_id,occurred_ms,currency,amount_minor FROM events FINAL WHERE organization_id=? AND website_id=? AND name IN ('server.payment','server.refund') AND amount_minor!=0 AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?)) AS purchases LEFT JOIN (SELECT visitor_id,arraySort(groupArray((started,source,campaign,landing))) AS touches FROM (SELECT visitor_id,session_id,min(occurred_ms) AS started,argMin(if(utm_source!='',utm_source,if(referrer!='',referrer,'Direct')),occurred_ms) AS source,argMin(utm_campaign,occurred_ms) AS campaign,argMin(path,occurred_ms) AS landing FROM events FINAL WHERE organization_id=? AND website_id=? AND name='pageview' AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) GROUP BY visitor_id,session_id) GROUP BY visitor_id) AS traffic USING visitor_id) GROUP BY label,currency ORDER BY currency,sum(toDecimal128(amount_minor,0)) DESC LIMIT 100");
    store
        .ch
        .query(&sql)
        .bind(site.organization_id.to_string())
        .bind(site.id.to_string())
        .bind(start)
        .bind(end)
        .bind(site.organization_id.to_string())
        .bind(site.id.to_string())
        .bind(start.saturating_sub(30 * 86400))
        .bind(end)
        .fetch_all()
        .await
        .map_err(ApiError::internal)
}
pub async fn revenue(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<RevenueQuery>,
) -> AppResult<Json<Vec<Revenue>>> {
    let (site, _, _queries) = auth::authorize_report(&s, &h, id).await?;
    let (start, end) = api::range(&api::Period { days: p.days })?;
    Ok(Json(
        revenue_data(
            &s.store,
            &site,
            start,
            end,
            p.model.as_deref().unwrap_or("last_non_direct"),
            p.dimension.as_deref().unwrap_or("source"),
        )
        .await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conversion {
    pub external_id: String,
    pub kind: String,
    pub session_id: Option<Uuid>,
    pub visitor_id: Option<Uuid>,
    pub path: Option<String>,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub occurred_at_ms: Option<i64>,
}
pub async fn backend_event(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<Conversion>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    let token = h
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthorized)?;
    let site:Website=sqlx::query_as("SELECT w.id,w.organization_id,w.name,w.origin,w.collection_id,w.verification_token,w.verified_at,w.active,w.is_demo FROM websites w JOIN api_keys k ON k.website_id=w.id JOIN organizations o ON o.id=w.organization_id WHERE w.id=$1 AND k.token_hash=$2 AND k.scope='integration' AND k.revoked_at IS NULL AND w.active AND NOT w.is_demo AND w.deletion_requested_at IS NULL AND o.status='active'").bind(id).bind(click_core::digest(token)).fetch_optional(&s.store.pg).await?.ok_or_else(ApiError::not_found)?;
    if input.external_id.is_empty()
        || input.external_id.len() > 200
        || input.external_id.chars().any(char::is_control)
        || !matches!(
            input.kind.as_str(),
            "payment"
                | "refund"
                | "member_signup"
                | "subscription_started"
                | "subscription_cancelled"
                | "newsletter_sent"
                | "newsletter_open"
                | "newsletter_click"
        )
    {
        return Err(ApiError::bad(
            "Use a stable external event identifier and supported backend event kind",
        ));
    }
    let monetary = matches!(input.kind.as_str(), "payment" | "refund");
    let amount = input.amount_minor.unwrap_or(0);
    let currency = input.currency.unwrap_or_default();
    if monetary
        && (amount <= 0
            || amount > 1_000_000_000_000
            || !matches!(
                currency.as_str(),
                "USD" | "EUR" | "GBP" | "CAD" | "AUD" | "NZD" | "SGD" | "INR" | "BDT" | "JPY"
            ))
    {
        return Err(ApiError::bad(
            "Payments require a positive amount in supported currency minor units",
        ));
    }
    if !monetary && (amount != 0 || !currency.is_empty()) {
        return Err(ApiError::bad("Only payment and refund events carry money"));
    }
    let hash = click_core::digest(&format!("{id}:{}:{}", input.kind, input.external_id));
    let bytes = hex::decode(&hash[..32]).map_err(ApiError::internal)?;
    let event_id = Uuid::from_slice(&bytes).map_err(ApiError::internal)?;
    let session = input.session_id.unwrap_or(event_id);
    let path = input.path.unwrap_or_else(|| "/".into());
    if !path.starts_with('/') || path.starts_with("//") || path.len() > 1024 {
        return Err(ApiError::bad("Use a local website path"));
    }
    let clean = click_core::BrowserEvent {
        event_id,
        session_id: session,
        visitor_id: input.visitor_id,
        name: "conversion".into(),
        url: format!("{}{path}", site.origin),
        referrer: String::new(),
        utm_source: String::new(),
        utm_medium: String::new(),
        utm_campaign: String::new(),
        occurred_at_ms: input.occurred_at_ms,
        vital: None,
    }
    .clean(id, &site.origin)
    .map_err(|e| ApiError::bad(e.0))?;
    let mut clean = clean;
    clean.name = format!("server.{}", input.kind);
    clean.amount_minor = if input.kind == "refund" {
        -amount
    } else {
        amount
    };
    clean.currency = currency;
    let accepted = s
        .store
        .accept_integration(&site, vec![clean], &click_core::digest(token))
        .await
        .map_err(|e| match e {
            click_store::AcceptError::Quota => ApiError {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: "Monthly event allowance reached",
            },
            click_store::AcceptError::Disabled => ApiError::forbidden("Collection is disabled"),
            other => ApiError::internal(other),
        })?;
    Ok((
        StatusCode::ACCEPTED,
        Json(
            serde_json::json!({"event_id":event_id,"accepted":accepted.inserted,"duplicates":accepted.duplicates}),
        ),
    ))
}

pub fn money(minor: &str, currency: &str) -> AppResult<String> {
    let value = minor.parse::<i128>().map_err(ApiError::internal)?;
    Ok(if currency == "JPY" {
        format!("JPY {value}")
    } else {
        let sign = if value < 0 { "-" } else { "" };
        let absolute = value.unsigned_abs();
        format!("{currency} {sign}{}.{:02}", absolute / 100, absolute % 100)
    })
}
