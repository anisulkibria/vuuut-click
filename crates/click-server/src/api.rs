use crate::{auth, ApiError, AppResult, AppState};
use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{Duration, Utc};
use click_core::{BrowserEvent, Role};
use click_store::{AcceptError, Dimension, Overview, Website};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use url::Url;
use uuid::Uuid;

pub async fn ready(State(state): State<AppState>) -> AppResult<&'static str> {
    sqlx::query("SELECT 1").execute(&state.store.pg).await?;
    state
        .store
        .ch
        .query("SELECT 1")
        .execute()
        .await
        .map_err(ApiError::internal)?;
    Ok("ready")
}
pub async fn register(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(body): Json<auth::Credentials>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    let (token, user) = auth::register(&s, body).await?;
    Ok(auth::session_response(&s, &token, user))
}
pub async fn login(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(body): Json<auth::Credentials>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    let (token, user) = auth::login(&s, body).await?;
    Ok(auth::session_response(&s, &token, user))
}
pub async fn logout(State(s): State<AppState>, h: HeaderMap) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    if let Some(t) = auth::cookie_value(&h, "click_session") {
        sqlx::query("DELETE FROM sessions WHERE token_hash=$1 AND audience='user'")
            .bind(click_core::digest(&t))
            .execute(&s.store.pg)
            .await?;
    }
    let secure = if s.config.secure_cookie {
        "; Secure"
    } else {
        ""
    };
    Ok((
        [(
            header::SET_COOKIE,
            format!("click_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}"),
        )],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}
pub async fn me(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<auth::User>> {
    Ok(Json(auth::user(&s, &h).await?))
}
#[derive(Serialize, sqlx::FromRow)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub role: String,
}
pub async fn list_organizations(s: &AppState, h: &HeaderMap) -> AppResult<Vec<Organization>> {
    let u = auth::user(s, h).await?;
    Ok(sqlx::query_as("SELECT o.id,o.name,m.role FROM organizations o JOIN memberships m ON m.organization_id=o.id WHERE m.user_id=$1 AND o.status<>'deleting' ORDER BY o.created_at").bind(u.id).fetch_all(&s.store.pg).await?)
}
pub async fn organizations(
    State(s): State<AppState>,
    h: HeaderMap,
) -> AppResult<Json<Vec<Organization>>> {
    Ok(Json(list_organizations(&s, &h).await?))
}
pub async fn list_websites(s: &AppState, h: &HeaderMap) -> AppResult<Vec<Website>> {
    let u = auth::user(s, h).await?;
    Ok(sqlx::query_as("SELECT w.id,w.organization_id,w.name,w.origin,w.collection_id,w.verification_token,w.verified_at,w.active,w.is_demo FROM websites w JOIN memberships m ON m.organization_id=w.organization_id WHERE m.user_id=$1 AND w.deletion_requested_at IS NULL ORDER BY w.is_demo DESC,w.created_at").bind(u.id).fetch_all(&s.store.pg).await?)
}
pub async fn websites(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Vec<Website>>> {
    Ok(Json(list_websites(&s, &h).await?))
}
#[derive(Deserialize)]
pub struct NewWebsite {
    pub organization_id: Uuid,
    pub name: String,
    pub origin: String,
}
pub async fn add_website(s: &AppState, h: &HeaderMap, b: NewWebsite) -> AppResult<Website> {
    auth::csrf(s, h)?;
    let u = auth::user(s, h).await?;
    if !s.config.allow_localhost {
        let verified: bool =
            sqlx::query_scalar("SELECT email_verified_at IS NOT NULL FROM users WHERE id=$1")
                .bind(u.id)
                .fetch_one(&s.store.pg)
                .await?;
        if !verified {
            return Err(ApiError::forbidden(
                "Verify your email before adding a website",
            ));
        }
    }
    let url =
        Url::parse(b.origin.trim()).map_err(|_| ApiError::bad("Enter a valid website origin"))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if b.name.trim().is_empty()
        || b.name.len() > 100
        || b.origin.len() > 255
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || !(url.scheme() == "https" || s.config.allow_localhost && local && url.scheme() == "http")
    {
        return Err(ApiError::bad("Use an HTTPS origin without path, query or credentials and a name up to 100 characters"));
    }
    let mut tx = s.store.pg.begin().await?;
    let locked: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM organizations WHERE id=$1 AND status='active' FOR UPDATE",
    )
    .bind(b.organization_id)
    .fetch_optional(&mut *tx)
    .await?;
    if locked.is_none() {
        return Err(ApiError::not_found());
    }
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE organization_id=$1 AND user_id=$2")
            .bind(b.organization_id)
            .bind(u.id)
            .fetch_optional(&mut *tx)
            .await?;
    if !role
        .as_deref()
        .and_then(Role::parse)
        .is_some_and(Role::can_manage)
    {
        return Err(ApiError::not_found());
    }
    let (used,limit):(i64,i32)=sqlx::query_as("SELECT (SELECT count(*) FROM websites WHERE organization_id=$1 AND is_demo=false),p.website_limit FROM subscriptions s JOIN plans p ON p.id=s.plan_id WHERE s.organization_id=$1").bind(b.organization_id).fetch_one(&mut *tx).await?;
    if used >= limit as i64 {
        return Err(ApiError::conflict("Website allowance reached"));
    }
    let site = Website {
        id: Uuid::new_v4(),
        organization_id: b.organization_id,
        name: b.name.trim().to_owned(),
        origin: url.origin().ascii_serialization(),
        collection_id: format!("vc_{}", auth::secret()),
        verification_token: auth::secret(),
        verified_at: (local && s.config.allow_localhost).then(Utc::now),
        active: true,
        is_demo: false,
    };
    let result=sqlx::query("INSERT INTO websites(id,organization_id,name,origin,collection_id,verification_token,verified_at) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(site.id).bind(site.organization_id).bind(&site.name).bind(&site.origin).bind(&site.collection_id).bind(&site.verification_token).bind(site.verified_at).execute(&mut *tx).await;
    if let Err(e) = result {
        if e.as_database_error()
            .is_some_and(|e| e.is_unique_violation())
        {
            return Err(ApiError::conflict(
                "Website already exists in this workspace",
            ));
        }
        return Err(e.into());
    }
    sqlx::query("INSERT INTO audit_logs(organization_id,actor_id,action,resource_id) VALUES($1,$2,'website.created',$3)").bind(site.organization_id).bind(u.id).bind(site.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(site)
}
pub async fn create_website(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(b): Json<NewWebsite>,
) -> AppResult<(StatusCode, Json<Website>)> {
    Ok((StatusCode::CREATED, Json(add_website(&s, &h, b).await?)))
}
pub async fn verify_website(s: &AppState, h: &HeaderMap, id: Uuid) -> AppResult<()> {
    auth::csrf(s, h)?;
    let (site, _) = auth::authorize_site(s, h, id, true).await?;
    let url = Url::parse(&site.origin).map_err(ApiError::internal)?;
    let host = url
        .host_str()
        .ok_or_else(|| ApiError::bad("Invalid website host"))?;
    let resolver = hickory_resolver::TokioResolver::builder_tokio()
        .map_err(ApiError::internal)?
        .build()
        .map_err(ApiError::internal)?;
    let records = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        resolver.txt_lookup(format!("_vuuut-click.{host}.")),
    )
    .await
    .map_err(|_| ApiError::bad("DNS lookup timed out"))?
    .map_err(|_| ApiError::bad("Verification TXT record was not found"))?;
    let expected = format!("vuuut-click={}", site.verification_token);
    if !records.answers().iter().any(|r| match &r.data {
        hickory_resolver::proto::rr::RData::TXT(txt) => {
            txt.txt_data
                .iter()
                .flat_map(|v| v.iter().copied())
                .collect::<Vec<_>>()
                == expected.as_bytes()
        }
        _ => false,
    }) {
        return Err(ApiError::bad("DNS verification challenge does not match"));
    }
    sqlx::query("UPDATE websites SET verified_at=now() WHERE id=$1")
        .bind(id)
        .execute(&s.store.pg)
        .await?;
    Ok(())
}
pub async fn verify(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    verify_website(&s, &h, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize, Default)]
pub struct Period {
    pub days: Option<u32>,
}
pub fn range(p: &Period) -> AppResult<(u32, u32)> {
    let days = p.days.unwrap_or(7);
    if !matches!(days, 1 | 7 | 30 | 90) {
        return Err(ApiError::bad("days must be 1, 7, 30 or 90"));
    }
    let now = Utc::now();
    let start = (now.date_naive() - Duration::days((days - 1) as i64))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    Ok((start.timestamp() as u32, now.timestamp() as u32 + 1))
}
pub async fn overview(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<Period>,
) -> AppResult<Json<Overview>> {
    let (site, _) = auth::authorize_site(&s, &h, id, false).await?;
    let (a, b) = range(&p)?;
    Ok(Json(s.store.overview(&site, a, b).await?))
}
pub async fn breakdown(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, d)): Path<(Uuid, String)>,
    Query(p): Query<Period>,
) -> AppResult<Json<Vec<click_store::Breakdown>>> {
    let (site, _) = auth::authorize_site(&s, &h, id, false).await?;
    let dimension = Dimension::parse(&d).ok_or_else(ApiError::not_found)?;
    let (a, b) = range(&p)?;
    Ok(Json(s.store.breakdown(&site, dimension, a, b).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub events: Vec<BrowserEvent>,
}
async fn collection_site(s: &AppState, h: &HeaderMap, token: &str) -> AppResult<Website> {
    let site = s
        .store
        .collect_site(token)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if site.verified_at.is_none() {
        return Err(ApiError::forbidden(
            "Verify website ownership before collecting events",
        ));
    }
    if h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(site.origin.as_str()) {
        return Err(ApiError::forbidden(
            "Collection origin does not match website",
        ));
    }
    Ok(site)
}
fn collection_response(site: &Website, status: StatusCode, value: serde_json::Value) -> Response {
    (
        status,
        [
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, site.origin.clone()),
            (header::VARY, "Origin".to_owned()),
        ],
        Json(value),
    )
        .into_response()
}
pub async fn collect_options(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(token): Path<String>,
) -> AppResult<Response> {
    let site = collection_site(&s, &h, &token).await?;
    Ok((
        StatusCode::NO_CONTENT,
        [
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, site.origin),
            (header::ACCESS_CONTROL_ALLOW_METHODS, "POST, OPTIONS".into()),
            (header::ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type".into()),
            (header::VARY, "Origin".into()),
        ],
    )
        .into_response())
}
pub async fn collect(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    h: HeaderMap,
    Path(token): Path<String>,
    body: axum::body::Bytes,
) -> AppResult<Response> {
    let site = collection_site(&s, &h, &token).await?;
    // Accept text/plain JSON for sendBeacon, as well as application/json.
    let batch: Batch =
        serde_json::from_slice(&body).map_err(|_| ApiError::bad("Invalid event batch"))?;
    if batch.events.is_empty() || batch.events.len() > 50 {
        return Err(ApiError::bad("A batch must contain 1–50 events"));
    }
    if h.get("dnt").is_some_and(|v| v == "1") || h.get("sec-gpc").is_some_and(|v| v == "1") {
        return Ok(collection_response(
            &site,
            StatusCode::ACCEPTED,
            json!({"accepted":0,"duplicates":0,"ignored":true}),
        ));
    }
    let agent = h
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let events = batch
        .events
        .into_iter()
        .map(|e| {
            e.clean(site.id, &site.origin)
                .map_err(|e| ApiError::bad(e.0))
        })
        .collect::<AppResult<Vec<_>>>()?;
    let ip = click_core::client_ip(
        peer.ip(),
        h.get("x-forwarded-for").and_then(|v| v.to_str().ok()),
        &s.config.trusted_proxies,
    );
    let country = s.geoip.country(ip);
    let result = s
        .store
        .accept(&site, events, &country, agent)
        .await
        .map_err(|e| match e {
            AcceptError::Disabled => {
                ApiError::forbidden("Website collection is paused or disabled")
            }
            AcceptError::Quota => ApiError {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: "Monthly event allowance reached",
            },
            other => ApiError::internal(other),
        })?;
    Ok(collection_response(
        &site,
        StatusCode::ACCEPTED,
        json!({"accepted":result.inserted,"duplicates":result.duplicates}),
    ))
}
#[derive(Deserialize)]
pub struct KeyInput {
    pub label: String,
}
pub async fn create_key(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<KeyInput>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    auth::csrf(&s, &h)?;
    let (site, _) = auth::authorize_site(&s, &h, id, true).await?;
    if b.label.is_empty() || b.label.len() > 100 {
        return Err(ApiError::bad("Use a label up to 100 characters"));
    }
    let id = Uuid::new_v4();
    let key = format!("vck_{}", auth::secret());
    sqlx::query(
        "INSERT INTO api_keys(id,website_id,token_hash,label,scope) VALUES($1,$2,$3,$4,'read')",
    )
    .bind(id)
    .bind(site.id)
    .bind(click_core::digest(&key))
    .bind(b.label)
    .execute(&s.store.pg)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":id,"key":key,"scope":"read"})),
    ))
}
pub async fn revoke_key(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, key_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    auth::csrf(&s, &h)?;
    auth::authorize_site(&s, &h, id, true).await?;
    sqlx::query("UPDATE api_keys SET revoked_at=now() WHERE id=$1 AND website_id=$2")
        .bind(key_id)
        .bind(id)
        .execute(&s.store.pg)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
pub struct NewGoal {
    pub name: String,
    pub event_name: String,
}
#[derive(Serialize, sqlx::FromRow)]
pub struct Goal {
    pub id: Uuid,
    pub name: String,
    pub event_name: String,
}
pub async fn add_goal(s: &AppState, h: &HeaderMap, id: Uuid, b: NewGoal) -> AppResult<Goal> {
    auth::csrf(s, h)?;
    let (_, role) = auth::authorize_site(s, h, id, false).await?;
    if !role.can_edit_goals() {
        return Err(ApiError::not_found());
    }
    if b.name.trim().is_empty()
        || b.name.len() > 100
        || b.event_name.is_empty()
        || b.event_name.len() > 64
        || !b
            .event_name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
    {
        return Err(ApiError::bad(
            "Use a goal name and a valid custom event name",
        ));
    }
    let user = auth::user(s, h).await?;
    let (site, _) = auth::authorize_site(s, h, id, false).await?;
    let mut tx = s.store.pg.begin().await?;
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(site.organization_id)
        .fetch_one(&mut *tx)
        .await?;
    let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM memberships WHERE organization_id=$1 AND user_id=$2 AND role IN ('owner','admin','analyst'))").bind(site.organization_id).bind(user.id).fetch_one(&mut *tx).await?;
    if !allowed {
        return Err(ApiError::not_found());
    }
    let limits:(i64,i32)=sqlx::query_as("SELECT (SELECT count(*) FROM goals WHERE website_id=$1),p.goal_limit FROM subscriptions s JOIN plans p ON p.id=s.plan_id WHERE s.organization_id=$2").bind(id).bind(site.organization_id).fetch_one(&mut *tx).await?;
    if limits.0 >= limits.1 as i64 {
        return Err(ApiError::conflict("Goal allowance reached"));
    }
    let goal = Goal {
        id: Uuid::new_v4(),
        name: b.name.trim().into(),
        event_name: b.event_name,
    };
    let result =
        sqlx::query("INSERT INTO goals(id,website_id,name,event_name) VALUES($1,$2,$3,$4)")
            .bind(goal.id)
            .bind(id)
            .bind(&goal.name)
            .bind(&goal.event_name)
            .execute(&mut *tx)
            .await;
    if let Err(e) = result {
        if e.as_database_error()
            .is_some_and(|e| e.is_unique_violation())
        {
            return Err(ApiError::conflict("A goal with this name already exists"));
        }
        return Err(e.into());
    }
    crate::workspace::audit(
        &mut tx,
        site.organization_id,
        user.id,
        "goal.created",
        goal.id,
    )
    .await?;
    tx.commit().await?;
    Ok(goal)
}

pub async fn create_goal(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<NewGoal>,
) -> AppResult<(StatusCode, Json<Goal>)> {
    Ok((StatusCode::CREATED, Json(add_goal(&s, &h, id, b).await?)))
}
pub async fn goals(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<Goal>>> {
    auth::authorize_site(&s, &h, id, false).await?;
    Ok(Json(
        sqlx::query_as(
            "SELECT id,name,event_name FROM goals WHERE website_id=$1 ORDER BY created_at",
        )
        .bind(id)
        .fetch_all(&s.store.pg)
        .await?,
    ))
}
pub async fn usage(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let u = auth::user(&s, &h).await?;
    let row:Option<(String,i64,i64,String)>=sqlx::query_as("SELECT p.id,p.monthly_events,coalesce(u.accepted_events,0),s.provider FROM subscriptions s JOIN plans p ON p.id=s.plan_id JOIN memberships m ON m.organization_id=s.organization_id LEFT JOIN usage_records u ON u.organization_id=s.organization_id AND u.month=date_trunc('month',now() AT TIME ZONE 'UTC')::date WHERE s.organization_id=$1 AND m.user_id=$2")
        .bind(id).bind(u.id).fetch_optional(&s.store.pg).await?;
    let (plan, limit, used, provider) = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(
        json!({"plan":plan,"monthly_events":limit,"accepted_events":used,"billing_provider":provider}),
    ))
}
