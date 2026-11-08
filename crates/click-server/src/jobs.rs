//! Durable exports and privacy deletion. Accepted ingestion is never discarded on a storage error.
use crate::{api, auth, workspace, ApiError, AppResult, AppState};
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use click_core::Role;
use click_store::{EventRow, Store};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, sqlx::FromRow)]
pub struct Export {
    pub id: Uuid,
    pub state: String,
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
pub async fn queue_export(
    s: &AppState,
    h: &HeaderMap,
    site_id: Uuid,
    days: u32,
) -> AppResult<Uuid> {
    auth::csrf(s, h)?;
    let (site, role) = auth::authorize_site(s, h, site_id, false).await?;
    if !role.can_edit_goals() {
        return Err(ApiError::not_found());
    }
    let u = auth::user(s, h).await?;
    let (start, end) = api::range(&api::Period { days: Some(days) })?;
    let mut tx = s.store.pg.begin().await?;
    // This path held the organization row but never looked at the membership
    // again, so a member removed after their request began could still queue
    // an export of the whole workspace's data — the one operation where that
    // matters most.
    workspace::lock_editor(&mut tx, site.organization_id, u.id).await?;
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM export_jobs WHERE organization_id=$1 AND created_at>now()-interval '1 day'").bind(site.organization_id).fetch_one(&mut *tx).await?;
    if count >= 20 {
        return Err(ApiError::conflict("Daily export limit reached"));
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO export_jobs(id,organization_id,website_id,requested_by,start_at,end_at) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(site.organization_id).bind(site_id).bind(u.id).bind(start as i64).bind(end as i64).execute(&mut *tx).await?;
    workspace::audit(&mut tx, site.organization_id, u.id, "export.requested", id).await?;
    tx.commit().await?;
    Ok(id)
}
pub async fn create_export(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(p): Json<api::Period>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"id":queue_export(&s,&h,id,p.days.unwrap_or(7)).await?})),
    ))
}
pub async fn exports(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<Export>>> {
    let (_, role) = auth::authorize_site(&s, &h, id, false).await?;
    if !role.can_edit_goals() {
        return Err(ApiError::not_found());
    }
    Ok(Json(sqlx::query_as("SELECT id,state,error,created_at,expires_at FROM export_jobs WHERE website_id=$1 ORDER BY created_at DESC LIMIT 20").bind(id).fetch_all(&s.store.pg).await?))
}
pub async fn download(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Response> {
    let site: Uuid =
        sqlx::query_scalar("SELECT website_id FROM export_jobs WHERE id=$1 AND expires_at>now()")
            .bind(id)
            .fetch_optional(&s.store.pg)
            .await?
            .ok_or_else(ApiError::not_found)?;
    let (_, role) = auth::authorize_site(&s, &h, site, false).await?;
    if !role.can_edit_goals() {
        return Err(ApiError::not_found());
    }
    let bytes: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT payload FROM export_jobs WHERE id=$1 AND state='complete' AND expires_at>now()",
    )
    .bind(id)
    .fetch_optional(&s.store.pg)
    .await?
    .flatten();
    let bytes = bytes.ok_or_else(|| ApiError::conflict("Export is not ready or has expired"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"vuuut-click-{id}.csv\""),
            ),
        ],
        bytes,
    )
        .into_response())
}
#[derive(Deserialize)]
pub struct DeleteInput {
    pub confirmation: String,
}
pub async fn delete_site(
    s: &AppState,
    h: &HeaderMap,
    id: Uuid,
    confirmation: &str,
) -> AppResult<Uuid> {
    auth::csrf(s, h)?;
    let (site, _) = auth::authorize_site(s, h, id, true).await?;
    let user = auth::user(s, h).await?;
    if confirmation != site.origin {
        return Err(ApiError::bad("Type the website origin to confirm deletion"));
    }
    let mut tx = s.store.pg.begin().await?;
    // Not gated on the workspace being active: erasing your own data has to
    // remain possible while a workspace is suspended.
    workspace::lock_member_even_if_suspended(
        &mut tx,
        site.organization_id,
        user.id,
        Role::can_manage,
    )
    .await?;
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM deletion_jobs WHERE website_id=$1 AND state='pending'")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let job = existing.unwrap_or_else(Uuid::new_v4);
    if existing.is_none() {
        sqlx::query("INSERT INTO deletion_jobs(id,organization_id,website_id,requested_by) VALUES($1,$2,$3,$4)").bind(job).bind(site.organization_id).bind(id).bind(user.id).execute(&mut *tx).await?;
    }
    sqlx::query("UPDATE websites SET active=false,deletion_requested_at=now() WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    workspace::audit(
        &mut tx,
        site.organization_id,
        user.id,
        "website.deletion_requested",
        id,
    )
    .await?;
    tx.commit().await?;
    Ok(job)
}
pub async fn delete_website(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<DeleteInput>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"id":delete_site(&s,&h,id,&input.confirmation).await?})),
    ))
}
pub async fn delete_workspace(
    s: &AppState,
    h: &HeaderMap,
    org: Uuid,
    confirmation: &str,
) -> AppResult<Uuid> {
    auth::csrf(s, h)?;
    let (user, role) = workspace::membership(s, h, org, true).await?;
    if role != click_core::Role::Owner {
        return Err(ApiError::not_found());
    }
    let mut tx = s.store.pg.begin().await?;
    let name: String = sqlx::query_scalar("SELECT name FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
    if confirmation != name {
        return Err(ApiError::bad("Type the workspace name to confirm deletion"));
    }
    let is_owner:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM memberships WHERE organization_id=$1 AND user_id=$2 AND role='owner')").bind(org).bind(user.id).fetch_one(&mut *tx).await?;
    if !is_owner {
        return Err(ApiError::not_found());
    }
    let has_paid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM subscriptions WHERE organization_id=$1 AND provider='stripe' AND status IN ('active','past_due'))").bind(org).fetch_one(&mut *tx).await?;
    if has_paid {
        return Err(ApiError::conflict(
            "Cancel your paid subscription before deleting the workspace",
        ));
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO deletion_jobs(id,organization_id,requested_by) VALUES($1,$2,$3)")
        .bind(id)
        .bind(org)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE organizations SET status='deleting' WHERE id=$1")
        .bind(org)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE websites SET active=false,deletion_requested_at=now() WHERE organization_id=$1",
    )
    .bind(org)
    .execute(&mut *tx)
    .await?;
    workspace::audit(&mut tx, org, user.id, "workspace.deletion_requested", org).await?;
    tx.commit().await?;
    Ok(id)
}
#[derive(Deserialize)]
pub struct RetentionInput {
    pub days: i32,
}
pub async fn retention(s: &AppState, h: &HeaderMap, id: Uuid, days: i32) -> AppResult<()> {
    auth::csrf(s, h)?;
    let (site, _) = auth::authorize_site(s, h, id, true).await?;
    if site.is_demo {
        return Err(ApiError::bad(
            "Demo retention is managed by the sample dataset",
        ));
    }
    let user = auth::user(s, h).await?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_member(&mut tx, site.organization_id, user.id, Role::can_manage).await?;
    let limit:i32=sqlx::query_scalar("SELECT p.retention_days FROM subscriptions s JOIN plans p ON p.id=s.plan_id WHERE s.organization_id=$1").bind(site.organization_id).fetch_one(&mut *tx).await?;
    if !(7..=90).contains(&days) || days > limit {
        return Err(ApiError::bad(
            "Retention must be between 7 days and your plan's retention limit",
        ));
    }
    let changed = sqlx::query("UPDATE websites SET retention_days=$1 WHERE id=$2")
        .bind(days)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // Say so rather than reporting a success that changed nothing and
    // writing an audit entry for it.
    if changed.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    workspace::audit(
        &mut tx,
        site.organization_id,
        user.id,
        "privacy.retention_changed",
        id,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn set_retention(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<RetentionInput>,
) -> AppResult<StatusCode> {
    retention(&s, &h, id, input.days).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn deletion_status(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let user = auth::user(&s, &h).await?;
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT state,error FROM deletion_jobs WHERE id=$1 AND requested_by=$2")
            .bind(id)
            .bind(user.id)
            .fetch_optional(&s.store.pg)
            .await?;
    let (state, error) = row.ok_or_else(ApiError::not_found)?;
    Ok(Json(
        serde_json::json!({"id":id,"state":state,"error":error}),
    ))
}
fn csv_cell(input: &str) -> String {
    let value = if input.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{input}")
    } else {
        input.to_owned()
    };
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub async fn process_export(store: &Store) -> anyhow::Result<()> {
    let mut tx = store.pg.begin().await?;
    let job:Option<(Uuid,Uuid,Uuid,i64,i64)>=sqlx::query_as("SELECT id,organization_id,website_id,start_at,end_at FROM export_jobs WHERE state='pending' AND expires_at>now() ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED").fetch_optional(&mut *tx).await?;
    let Some((id, org, site, start, end)) = job else {
        return Ok(());
    };
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM event_outbox WHERE website_id=$1 AND accepted_at<to_timestamp($2::bigint::double precision) AND state='pending')").bind(site).bind(end).fetch_one(&mut *tx).await?;
    if pending {
        return Ok(());
    }
    let dead:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM event_outbox WHERE website_id=$1 AND accepted_at>=to_timestamp($2::bigint::double precision) AND accepted_at<to_timestamp($3::bigint::double precision) AND state='dead')").bind(site).bind(start).bind(end).fetch_one(&mut *tx).await?;
    if dead {
        sqlx::query("UPDATE export_jobs SET state='failed',error='Resolve dead-letter events before exporting this period' WHERE id=$1").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        return Ok(());
    }
    let rows:Vec<EventRow>=store.ch.clone().with_option("max_result_rows","100001").query("SELECT ?fields FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) ORDER BY timestamp,event_id LIMIT 100001").bind(org.to_string()).bind(site.to_string()).bind(start as u32).bind(end as u32).fetch_all().await?;
    if rows.len() > 100000 {
        sqlx::query("UPDATE export_jobs SET state='failed',error='More than 100,000 events; select a shorter period' WHERE id=$1").bind(id).execute(&mut *tx).await?;
    } else {
        let mut csv=String::from("timestamp_utc,event_id,event,path,country,referrer,utm_source,utm_medium,utm_campaign,device,browser,occurred_ms,metric,metric_value,amount_minor,currency\r\n");
        for row in rows {
            let time = chrono::DateTime::from_timestamp(row.timestamp as i64, 0)
                .unwrap()
                .to_rfc3339();
            let cells = [
                time,
                row.event_id,
                row.name,
                row.path,
                row.country,
                row.referrer,
                row.utm_source,
                row.utm_medium,
                row.utm_campaign,
                row.device,
                row.browser,
                row.occurred_ms.to_string(),
                row.metric_name,
                row.metric_value.to_string(),
                row.amount_minor.to_string(),
                row.currency,
            ];
            csv.push_str(
                &cells
                    .iter()
                    .map(|s| csv_cell(s))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            csv.push_str("\r\n");
        }
        sqlx::query("UPDATE export_jobs SET state='complete',payload=$1,error=NULL WHERE id=$2")
            .bind(csv.into_bytes())
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
pub async fn process_deletion(store: &Store) -> anyhow::Result<()> {
    let candidate:Option<(Uuid,Uuid,Option<Uuid>)>=sqlx::query_as("SELECT id,organization_id,website_id FROM deletion_jobs WHERE state='pending' ORDER BY created_at LIMIT 1").fetch_optional(&store.pg).await?;
    let Some((id, org, site)) = candidate else {
        return Ok(());
    };
    let mut tx = store.pg.begin().await?;
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_optional(&mut *tx)
        .await?;
    let job: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM deletion_jobs WHERE id=$1 AND state='pending' FOR UPDATE SKIP LOCKED",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    if job.is_none() {
        return Ok(());
    }
    if let Some(site) = site {
        sqlx::query("DELETE FROM event_outbox WHERE website_id=$1")
            .bind(site)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("DELETE FROM event_outbox WHERE organization_id=$1")
            .bind(org)
            .execute(&mut *tx)
            .await?;
    }
    for table in ["events", "pageviews_hourly"] {
        let query = if site.is_some() {
            format!("ALTER TABLE {table} DELETE WHERE organization_id=? AND website_id=? SETTINGS mutations_sync=2")
        } else {
            format!("ALTER TABLE {table} DELETE WHERE organization_id=? SETTINGS mutations_sync=2")
        };
        let mut query = store.ch.query(&query).bind(org.to_string());
        if let Some(site) = site {
            query = query.bind(site.to_string());
        }
        query.execute().await?;
    }
    if let Some(site) = site {
        sqlx::query("DELETE FROM websites WHERE id=$1 AND organization_id=$2")
            .bind(site)
            .bind(org)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("DELETE FROM organizations WHERE id=$1")
            .bind(org)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "UPDATE deletion_jobs SET state='complete',completed_at=now(),error=NULL WHERE id=$1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn enforce_retention(store: &Store) -> anyhow::Result<()> {
    let sites:Vec<(Uuid,Uuid,i32)>=sqlx::query_as("SELECT w.id,w.organization_id,least(w.retention_days,p.retention_days) FROM websites w JOIN subscriptions s ON s.organization_id=w.organization_id JOIN plans p ON p.id=s.plan_id WHERE NOT w.is_demo AND w.deletion_requested_at IS NULL").fetch_all(&store.pg).await?;
    for (site, org, days) in sites {
        let cutoff = ((chrono::Utc::now().timestamp() - days as i64 * 86400) / 3600 * 3600) as u32;
        let mut tx = store.pg.begin().await?;
        sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
            .bind(org)
            .fetch_optional(&mut *tx)
            .await?;
        sqlx::query("UPDATE event_outbox SET state='discarded',payload=NULL WHERE website_id=$1 AND (payload->>'timestamp')::bigint<$2 AND state IN ('pending','dead')").bind(site).bind(cutoff as i64).execute(&mut *tx).await?;
        for (table, column) in [("events", "timestamp"), ("pageviews_hourly", "bucket")] {
            store.ch.query(&format!("ALTER TABLE {table} DELETE WHERE organization_id=? AND website_id=? AND {column}<toDateTime(?) SETTINGS mutations_sync=2")).bind(org.to_string()).bind(site.to_string()).bind(cutoff).execute().await?;
        }
        tx.commit().await?;
    }
    sqlx::query("DELETE FROM export_jobs WHERE expires_at<now()")
        .execute(&store.pg)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at<now()")
        .execute(&store.pg)
        .await?;
    Ok(())
}
pub async fn worker(
    store: Store,
    mail: std::sync::Arc<crate::mail::Mailer>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut ticks = 0u64;
    loop {
        if *shutdown.borrow() {
            break;
        }
        for result in [
            process_deletion(&store).await,
            process_export(&store).await,
            mail.drain_once(&store).await,
        ] {
            if let Err(error) = result {
                tracing::error!(%error,"durable control job retained for retry");
            }
        }
        if ticks.is_multiple_of(30) {
            if let Err(error) = enforce_retention(&store).await {
                tracing::error!(%error,"retention maintenance failed");
            }
        }
        ticks += 1;
        tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs(2))=>{},_=shutdown.changed()=>{}}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_prevents_formula_execution_and_escapes_quotes() {
        assert_eq!(csv_cell("=cmd()"), "\"'=cmd()\"");
        assert_eq!(csv_cell("a,\"b\""), "\"a,\"\"b\"\"\"");
    }
}
