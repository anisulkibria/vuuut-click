//! Recorded campaign costs with provenance, revision replay and explicit coverage.
use crate::{analytics, api, auth, workspace, ApiError, AppResult, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap},
    routing::get,
    Json, Router,
};
use chrono::{NaiveDate, Utc};
use click_core::{digest, Role};
use click_store::{Store, Website};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub fn router() -> Router<AppState> {
    Router::new().route("/v1/websites/{id}/campaign-costs", get(report).post(record))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostInput {
    pub reference: String,
    pub revision: i64,
    pub day: NaiveDate,
    pub campaign: String,
    pub currency: String,
    pub amount_minor: i64,
    pub source: String,
}
#[derive(Serialize)]
pub struct RecordedCost {
    pub id: Uuid,
    pub revision: i64,
}
fn validate(input: &CostInput) -> AppResult<()> {
    if input.reference.is_empty()
        || input.reference.len() > 200
        || input.reference.chars().any(char::is_control)
        || input.revision < 1
        || input.day > Utc::now().date_naive()
        || input.campaign.trim().is_empty()
        || input.campaign.len() > 120
        || input.campaign.chars().any(char::is_control)
        || input.source.is_empty()
        || input.source.len() > 32
        || !input
            .source
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
        || !(0..=1_000_000_000_000).contains(&input.amount_minor)
        || !matches!(
            input.currency.as_str(),
            "USD" | "EUR" | "GBP" | "CAD" | "AUD" | "NZD" | "SGD" | "INR" | "BDT" | "JPY"
        )
    {
        return Err(ApiError::bad("Provide a stable source reference, positive revision, UTC date, campaign, source and supported currency minor units"));
    }
    Ok(())
}
const INTEGRATION:&str="SELECT w.id,w.organization_id,w.name,w.origin,w.collection_id,w.verification_token,w.verified_at,w.active,w.is_demo FROM websites w JOIN api_keys k ON k.website_id=w.id JOIN organizations o ON o.id=w.organization_id WHERE w.id=$1 AND k.token_hash=$2 AND k.scope='integration' AND k.revoked_at IS NULL AND w.active AND NOT w.is_demo AND w.verified_at IS NOT NULL AND w.deletion_requested_at IS NULL AND o.status='active' AND NOT EXISTS(SELECT 1 FROM integration_mappings m JOIN native_platforms p ON p.id=m.platform_id WHERE m.website_id=w.id AND NOT p.active)";
pub async fn record_cost(
    s: &AppState,
    h: &HeaderMap,
    id: Uuid,
    input: CostInput,
) -> AppResult<RecordedCost> {
    validate(&input)?;
    let user = auth::user(s, h).await;
    let (site, actor, token_hash) = match user {
        Ok(user) => {
            auth::csrf(s, h)?;
            let (site, role) = auth::authorize_site(s, h, id, false).await?;
            if !role.can_manage() {
                return Err(ApiError::not_found());
            }
            (site, Some(user.id), None)
        }
        Err(e) if e.status == axum::http::StatusCode::UNAUTHORIZED => {
            let token = h
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .filter(|v| v.len() <= 256)
                .ok_or_else(ApiError::unauthorized)?;
            let hash = digest(token);
            let site = sqlx::query_as(INTEGRATION)
                .bind(id)
                .bind(&hash)
                .fetch_optional(&s.store.pg)
                .await?
                .ok_or_else(ApiError::not_found)?;
            (site, None, Some(hash))
        }
        Err(e) => return Err(e),
    };
    if site.is_demo {
        return Err(ApiError::bad(
            "Demo costs are managed by the sample dataset",
        ));
    }
    let mut tx = s.store.pg.begin().await?;
    if let Some(actor) = actor {
        workspace::lock_member(&mut tx, site.organization_id, actor, Role::can_manage).await?;
    } else {
        sqlx::query("SELECT id FROM organizations WHERE id=$1 AND status='active' FOR UPDATE")
            .bind(site.organization_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(ApiError::not_found)?;
        sqlx::query_as::<_, Website>(INTEGRATION)
            .bind(id)
            .bind(token_hash)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(ApiError::not_found)?;
    }
    let writable:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM websites WHERE id=$1 AND active AND deletion_requested_at IS NULL)").bind(id).fetch_one(&mut *tx).await?;
    if !writable {
        return Err(ApiError::not_found());
    }
    let retention:i32=sqlx::query_scalar("SELECT least(w.retention_days,p.retention_days) FROM websites w JOIN subscriptions s ON s.organization_id=w.organization_id JOIN plans p ON p.id=s.plan_id WHERE w.id=$1").bind(id).fetch_one(&mut *tx).await?;
    if input.day < (Utc::now() - chrono::Duration::days(retention as i64)).date_naive() {
        return Err(ApiError::bad(
            "Cost date is outside the website's retained history",
        ));
    }
    let reference = digest(&format!("{id}:{}:{}", input.source, input.reference));
    let fingerprint = digest(&serde_json::to_string(&input).map_err(ApiError::internal)?);
    let existing:Option<(Uuid,i64,String)>=sqlx::query_as("SELECT id,revision,request_hash FROM campaign_costs WHERE website_id=$1 AND reference_hash=$2 FOR UPDATE").bind(id).bind(&reference).fetch_optional(&mut *tx).await?;
    if let Some((existing_id, revision, hash)) = &existing {
        if input.revision < *revision || input.revision == *revision && fingerprint != *hash {
            return Err(ApiError::conflict("Stale or conflicting cost revision"));
        }
        if input.revision == *revision {
            tx.commit().await?;
            return Ok(RecordedCost {
                id: *existing_id,
                revision: *revision,
            });
        }
    }
    if existing.is_none() {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM campaign_costs WHERE website_id=$1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if count >= 50000 {
            return Err(ApiError::conflict("Website limit of 50000 retained cost records reached; update an existing source record"));
        }
    }
    let cost = existing.map(|e| e.0).unwrap_or_else(Uuid::new_v4);
    sqlx::query("INSERT INTO campaign_costs(id,website_id,reference_hash,revision,request_hash,day,campaign,currency,amount_minor,source) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT(website_id,reference_hash) DO UPDATE SET revision=EXCLUDED.revision,request_hash=EXCLUDED.request_hash,day=EXCLUDED.day,campaign=EXCLUDED.campaign,currency=EXCLUDED.currency,amount_minor=EXCLUDED.amount_minor,source=EXCLUDED.source,updated_at=now()")
        .bind(cost).bind(id).bind(reference).bind(input.revision).bind(fingerprint).bind(input.day).bind(input.campaign).bind(input.currency).bind(input.amount_minor).bind(input.source).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_logs(organization_id,actor_id,action,resource_id) VALUES($1,$2,'campaign_cost.recorded',$3)").bind(site.organization_id).bind(actor).bind(cost).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(RecordedCost {
        id: cost,
        revision: input.revision,
    })
}
async fn record(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<CostInput>,
) -> AppResult<Json<RecordedCost>> {
    Ok(Json(record_cost(&s, &h, id, input).await?))
}
#[derive(Deserialize)]
pub struct CostQuery {
    pub days: Option<u32>,
    pub model: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct CampaignReturn {
    pub campaign: String,
    pub currency: String,
    pub net_revenue_minor: String,
    pub revenue_recorded: bool,
    pub attribution_complete: bool,
    pub cost_minor: Option<String>,
    pub covered_days: i64,
    pub expected_days: i64,
    pub sources: Vec<String>,
    pub roi_percent: Option<f64>,
    pub roas: Option<f64>,
}
#[derive(sqlx::FromRow)]
struct CostTotal {
    campaign: String,
    currency: String,
    amount: String,
    covered_days: i64,
    sources: Vec<String>,
}
pub async fn returns_data(
    store: &Store,
    site: &Website,
    start: u32,
    end: u32,
    model: &str,
) -> AppResult<Vec<CampaignReturn>> {
    let from = chrono::DateTime::from_timestamp(start as i64, 0)
        .ok_or_else(ApiError::not_found)?
        .date_naive();
    let to = chrono::DateTime::from_timestamp(end.saturating_sub(1) as i64, 0)
        .ok_or_else(ApiError::not_found)?
        .date_naive();
    let expected_days = (to - from).num_days() + 1;
    let costs:Vec<CostTotal>=sqlx::query_as("SELECT campaign,currency,sum(amount_minor)::text AS amount,count(DISTINCT day) AS covered_days,array_agg(DISTINCT source ORDER BY source) AS sources FROM campaign_costs WHERE website_id=$1 AND day>=$2 AND day<=$3 GROUP BY campaign,currency ORDER BY campaign,currency LIMIT 1001").bind(site.id).bind(from).bind(to).fetch_all(&store.pg).await?;
    if costs.len() > 1000 {
        return Err(ApiError::bad(
            "More than 1000 cost groups; narrow the report period",
        ));
    }
    let revenue = analytics::revenue_data(store, site, start, end, model, "campaign").await?;
    let mut groups = std::collections::BTreeMap::<(String, String), CampaignReturn>::new();
    for row in revenue {
        let key = (row.label.clone(), row.currency.clone());
        groups.insert(
            key,
            CampaignReturn {
                campaign: row.label,
                currency: row.currency,
                net_revenue_minor: row.total_minor,
                revenue_recorded: true,
                attribution_complete: row.dimension_present == 1,
                cost_minor: None,
                covered_days: 0,
                expected_days,
                sources: Vec::new(),
                roi_percent: None,
                roas: None,
            },
        );
    }
    for cost in costs {
        let row = groups
            .entry((cost.campaign.clone(), cost.currency.clone()))
            .or_insert(CampaignReturn {
                campaign: cost.campaign,
                currency: cost.currency,
                net_revenue_minor: "0".into(),
                revenue_recorded: false,
                attribution_complete: false,
                cost_minor: None,
                covered_days: 0,
                expected_days,
                sources: Vec::new(),
                roi_percent: None,
                roas: None,
            });
        row.covered_days = cost.covered_days;
        row.sources = cost.sources;
        let amount = cost.amount.parse::<i128>().map_err(ApiError::internal)?;
        let net = row
            .net_revenue_minor
            .parse::<i128>()
            .map_err(ApiError::internal)?;
        if row.revenue_recorded
            && row.attribution_complete
            && amount > 0
            && cost.covered_days == expected_days
        {
            row.roi_percent = Some((net - amount) as f64 / amount as f64 * 100.0);
            row.roas = Some(net as f64 / amount as f64);
        }
        row.cost_minor = Some(cost.amount);
    }
    Ok(groups.into_values().collect())
}
async fn report(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(p): Query<CostQuery>,
) -> AppResult<Json<Vec<CampaignReturn>>> {
    let (site, _, _permit) = auth::authorize_report(&s, &h, id).await?;
    let (a, b) = api::range(&api::Period { days: p.days })?;
    Ok(Json(
        returns_data(
            &s.store,
            &site,
            a,
            b,
            p.model.as_deref().unwrap_or("last_non_direct"),
        )
        .await?,
    ))
}
