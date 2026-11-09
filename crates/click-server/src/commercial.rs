//! Operator-only commercial fulfillment evidence. No message bodies or credentials.
use crate::{auth, dashboard::esc, owner, ApiError, AppResult, AppState};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Form, Json,
};
use chrono::{DateTime, Utc};
use click_store::Store;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, sqlx::FromRow)]
pub struct Check {
    pub name: String,
    pub last_started_at: DateTime<Utc>,
    pub last_succeeded_at: Option<DateTime<Utc>>,
    pub last_failed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub failures: i64,
}
#[derive(Serialize, sqlx::FromRow)]
pub struct Queues {
    pub pending_mail: i64,
    pub dead_mail: i64,
    pub expired_mail: i64,
    pub old_pending_mail: i64,
    pub pending_exports: i64,
    pub old_pending_exports: i64,
    pub pending_deletions: i64,
    pub old_pending_deletions: i64,
}
#[derive(Serialize)]
pub struct Status {
    pub observed_at: DateTime<Utc>,
    pub billing_provider: String,
    pub billing_mode: String,
    pub mail_mode: String,
    pub checks: Vec<Check>,
    pub queues: Queues,
}
pub async fn record(
    store: &Store,
    name: &str,
    started: DateTime<Utc>,
    success: bool,
) -> anyhow::Result<()> {
    // A later-started run wins if concurrent workers finish in reverse order.
    sqlx::query("INSERT INTO maintenance_checks(name,last_started_at,last_succeeded_at,last_failed_at,last_error,failures) VALUES($1,$2,CASE WHEN $3 THEN now() ELSE NULL END,CASE WHEN $3 THEN NULL ELSE now() END,CASE WHEN $3 THEN NULL ELSE 'operation_failed' END,CASE WHEN $3 THEN 0 ELSE 1 END) ON CONFLICT(name) DO UPDATE SET last_started_at=EXCLUDED.last_started_at,last_succeeded_at=coalesce(EXCLUDED.last_succeeded_at,maintenance_checks.last_succeeded_at),last_failed_at=coalesce(EXCLUDED.last_failed_at,maintenance_checks.last_failed_at),last_error=EXCLUDED.last_error,failures=CASE WHEN $3 THEN 0 ELSE maintenance_checks.failures+1 END WHERE maintenance_checks.last_started_at<=EXCLUDED.last_started_at")
        .bind(name).bind(started).bind(success).execute(&store.pg).await?;
    Ok(())
}
pub async fn run(
    store: &Store,
    name: &str,
    operation: impl std::future::Future<Output = anyhow::Result<()>>,
) -> anyhow::Result<()> {
    let started = Utc::now();
    let result = operation.await;
    record(store, name, started, result.is_ok()).await?;
    result
}
pub async fn status(s: &AppState) -> AppResult<Status> {
    let checks = sqlx::query_as("SELECT * FROM maintenance_checks ORDER BY name")
        .fetch_all(&s.store.pg)
        .await?;
    let queues=sqlx::query_as("SELECT (SELECT count(*) FROM mail_outbox WHERE state='pending') AS pending_mail,(SELECT count(*) FROM mail_outbox WHERE state='dead') AS dead_mail,(SELECT count(*) FROM mail_outbox WHERE state='expired') AS expired_mail,(SELECT count(*) FROM mail_outbox WHERE state='pending' AND created_at<now()-interval '15 minutes') AS old_pending_mail,(SELECT count(*) FROM export_jobs WHERE state='pending') AS pending_exports,(SELECT count(*) FROM export_jobs WHERE state='pending' AND created_at<now()-interval '1 hour') AS old_pending_exports,(SELECT count(*) FROM deletion_jobs WHERE state='pending') AS pending_deletions,(SELECT count(*) FROM deletion_jobs WHERE state='pending' AND created_at<now()-interval '1 hour') AS old_pending_deletions")
        .fetch_one(&s.store.pg).await?;
    Ok(Status {
        observed_at: Utc::now(),
        billing_provider: s
            .billing_service()
            .provider
            .as_ref()
            .map(|p| p.name())
            .unwrap_or("disabled")
            .into(),
        billing_mode: s
            .billing_service()
            .provider
            .as_ref()
            .map(|p| p.mode())
            .unwrap_or("disabled")
            .into(),
        mail_mode: s.mail_service().mode().into(),
        checks,
        queues,
    })
}
pub async fn api(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Status>> {
    owner::authenticated_owner(&s, &h).await?;
    Ok(Json(status(&s).await?))
}
fn when(value: Option<DateTime<Utc>>) -> String {
    value
        .map(|v| v.format("%d %b %Y %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "Not recorded".into())
}
type DeadMail = (Uuid, DateTime<Utc>, i32, Option<DateTime<Utc>>);
pub async fn page(State(s): State<AppState>, h: HeaderMap) -> AppResult<Response> {
    let user = match owner::authenticated_owner(&s, &h).await {
        Ok(u) => u,
        Err(e) if e.status == StatusCode::UNAUTHORIZED => {
            return Ok(Redirect::to("/owner/signin").into_response())
        }
        Err(e) => return Err(e),
    };
    let report = status(&s).await?;
    let mut rows = String::new();
    for name in ["retention", "deletion", "exports", "mail"] {
        let check = report.checks.iter().find(|c| c.name == name);
        let state = match check {
            None => "Not observed",
            Some(c) if c.last_error.is_some() => "Failed; retry required",
            Some(c)
                if c.last_succeeded_at.is_some_and(|t| {
                    report.observed_at.signed_duration_since(t).num_seconds() <= 300
                }) =>
            {
                "Recent success"
            }
            _ => "Stale; inspect worker",
        };
        rows.push_str(&format!(
            "<tr><th scope=\"row\">{name}</th><td>{state}</td><td>{}</td><td>{}</td></tr>",
            when(check.and_then(|c| c.last_succeeded_at)),
            when(check.and_then(|c| c.last_failed_at))
        ));
    }
    let q = &report.queues;
    let mut content = format!(
        r#"<section class="panel"><div class="panel-heading"><h2>Provider configuration</h2></div><p>Billing: {} · {}. Mail: {}.</p><p class="muted">Configuration and worker success do not prove a payment or inbox delivery. Complete the commercial validation runbook with your own test account and inbox.</p></section><section class="panel"><div class="panel-heading"><h2>Fulfillment checks</h2></div><div class="owner-table"><table><thead><tr><th>Operation</th><th>Status</th><th>Last success</th><th>Last failure</th></tr></thead><tbody>{rows}</tbody></table></div><p class="muted">Missing or older-than-five-minute success needs investigation. Skipped retention locks are not counted as successful sweeps.</p></section><section class="panel"><div class="panel-heading"><h2>Pending work</h2></div><div class="owner-table"><table><thead><tr><th>Queue</th><th>Pending</th><th>Old pending</th></tr></thead><tbody><tr><th>Mail</th><td>{}</td><td>{}</td></tr><tr><th>Exports</th><td>{}</td><td>{}</td></tr><tr><th>Deletion</th><td>{}</td><td>{}</td></tr></tbody></table></div><p class="muted">Old pending means over 15 minutes for mail, one hour for exports/deletion. Dead mail: {}. Expired mail: {}. Expired links are never retried.</p></section>"#,
        esc(&report.billing_provider),
        esc(&report.billing_mode),
        esc(&report.mail_mode),
        q.pending_mail,
        q.old_pending_mail,
        q.pending_exports,
        q.old_pending_exports,
        q.pending_deletions,
        q.old_pending_deletions,
        q.dead_mail,
        q.expired_mail
    );
    let dead:Vec<DeadMail>=sqlx::query_as("SELECT id,created_at,attempts,expires_at FROM mail_outbox WHERE state='dead' ORDER BY created_at DESC LIMIT 20").fetch_all(&s.store.pg).await?;
    let mut rows = String::new();
    for (id, created, attempts, expiry) in dead {
        let action = if expiry.is_some_and(|t| t <= Utc::now()) {
            "Expired; no retry".into()
        } else {
            format!(
                r#"<form action="/owner/commercial/retry-mail" method="post"><input type="hidden" name="id" value="{id}"><button class="btn outlined small">Retry mail</button></form>"#
            )
        };
        rows.push_str(&format!(
            "<tr><td>{id}</td><td>{}</td><td>{attempts}</td><td>{action}</td></tr>",
            when(Some(created))
        ));
    }
    if rows.is_empty() {
        rows = "<tr><td colspan=4>No dead mail.</td></tr>".into();
    }
    content.push_str(&format!(r#"<section class="panel"><div class="panel-heading"><h2>Recent dead mail</h2></div><div class="owner-table"><table><thead><tr><th>Record</th><th>Queued</th><th>Attempts</th><th>Action</th></tr></thead><tbody>{rows}</tbody></table></div><p class="muted">Only the latest 20 dead records are shown. Message bodies and private links are never displayed.</p></section>"#));
    let body = format!(
        r#"<header class="topbar"><div class="breadcrumb">Your platform <span>/</span> Commercial status</div><span class="status">Owner console</span></header><div class="page owner-console"><div class="page-heading"><div><span class="eyebrow">YOUR PLATFORM</span><h1>Keep the promises behind your plans.</h1><p>Commercial delivery and retention status.</p></div><a class="btn outlined" href="/owner/commercial">Refresh</a></div>{content}</div>"#
    );
    Ok(owner::owner_shell(&h, &user.email, "commercial", body).into_response())
}
#[derive(Deserialize)]
pub struct Retry {
    pub id: Uuid,
}
pub async fn retry_mail(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<Retry>,
) -> AppResult<Redirect> {
    auth::csrf(&s, &h)?;
    let user = owner::authenticated_owner(&s, &h).await?;
    let mut tx = s.store.pg.begin().await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    let granted: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_owners WHERE user_id=$1)")
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?;
    if !granted {
        return Err(ApiError::not_found());
    }
    let changed=sqlx::query("UPDATE mail_outbox SET state='pending',attempts=0,available_at=now(),last_error=NULL WHERE id=$1 AND state='dead' AND body IS NOT NULL AND (expires_at IS NULL OR expires_at>now()) AND NOT EXISTS(SELECT 1 FROM account_tokens t WHERE t.token_hash=mail_outbox.account_token_hash AND (t.used_at IS NOT NULL OR t.expires_at<=now())) AND NOT EXISTS(SELECT 1 FROM invitations i WHERE i.id=mail_outbox.invitation_id AND (i.accepted_at IS NOT NULL OR i.revoked_at IS NOT NULL OR i.expires_at<=now()))").bind(input.id).execute(&mut *tx).await?.rows_affected();
    if changed != 1 {
        return Err(ApiError::conflict(
            "Mail is not retryable or its link has expired",
        ));
    }
    sqlx::query("INSERT INTO platform_audit(actor_id,action,resource_id) VALUES($1,'mail.dead_letter_requeued',$2)").bind(user.id).bind(input.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/commercial"))
}
