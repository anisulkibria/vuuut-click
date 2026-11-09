//! Shared abuse budgets, bounded report leases and private operational metrics.
//! Persistent network subjects are minute-scoped HMACs; no raw IP is retained.
use crate::{ApiError, AppResult, AppState, TenantPermit};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use click_store::Store;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::Semaphore;
use uuid::Uuid;

pub struct Operations {
    pub distributed: bool,
    key: Vec<u8>,
    metrics_token: Option<String>,
    pub writes: Arc<Semaphore>,
    pub requests: Arc<Semaphore>,
    metrics: Mutex<BTreeMap<(String, String, u16), Measurement>>,
    started: Instant,
}
#[derive(Default)]
struct Measurement {
    count: u64,
    seconds: f64,
    buckets: [u64; 7],
}
const BOUNDS: [f64; 7] = [0.005, 0.025, 0.1, 0.5, 1.0, 5.0, 30.0];
impl Operations {
    pub fn local() -> Self {
        Self::configured(false, Vec::new(), None)
    }
    pub fn configured(distributed: bool, key: Vec<u8>, metrics_token: Option<String>) -> Self {
        Self {
            distributed,
            key,
            metrics_token,
            writes: Arc::new(Semaphore::new(4)),
            requests: Arc::new(Semaphore::new(64)),
            metrics: Mutex::new(BTreeMap::new()),
            started: Instant::now(),
        }
    }
    pub fn from_env() -> anyhow::Result<Self> {
        let production = std::env::var("APP_ENV").is_ok_and(|v| v == "production");
        let distributed = match std::env::var("DISTRIBUTED_LIMITS").as_deref() {
            Ok("true") => true,
            Ok("false") => false,
            Err(_) => production,
            _ => anyhow::bail!("DISTRIBUTED_LIMITS must be true or false"),
        };
        anyhow::ensure!(
            !production || distributed,
            "Production requires distributed request budgets"
        );
        let key = std::env::var("ABUSE_HASH_KEY").unwrap_or_default();
        let key = if distributed {
            let bytes = hex::decode(key)
                .map_err(|_| anyhow::anyhow!("ABUSE_HASH_KEY must be 64 hexadecimal characters"))?;
            anyhow::ensure!(
                bytes.len() == 32,
                "ABUSE_HASH_KEY must contain 32 random bytes"
            );
            bytes
        } else {
            Vec::new()
        };
        let token = std::env::var("METRICS_BEARER_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        if let Some(token) = &token {
            anyhow::ensure!(
                (32..=256).contains(&token.len())
                    && token
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                "METRICS_BEARER_TOKEN must be a random 32–256 character token"
            );
        }
        anyhow::ensure!(
            !production || token.is_some(),
            "Production requires METRICS_BEARER_TOKEN"
        );
        Ok(Self::configured(distributed, key, token))
    }
    pub fn observe(&self, route: &str, method: &str, status: u16, seconds: f64) {
        let route = route_group(route).to_owned();
        let method = match method {
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" => method,
            _ => "OTHER",
        }
        .to_owned();
        let mut rows = self.metrics.lock().expect("metrics mutex");
        let row = rows.entry((route, method, status / 100)).or_default();
        row.count += 1;
        row.seconds += seconds;
        for (index, bound) in BOUNDS.iter().enumerate() {
            if seconds <= *bound {
                row.buckets[index] += 1;
            }
        }
    }
    fn subject(&self, window: i64, ip: IpAddr) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts any key size");
        mac.update(&window.to_be_bytes());
        mac.update(ip.to_string().as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
    pub async fn edge(&self, store: &Store, ip: IpAddr, sensitive: bool) -> AppResult<bool> {
        if !self.distributed {
            return Ok(true);
        }
        let mut tx = store.pg.begin().await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        let window: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp())/60)::bigint")
                .fetch_one(&mut *tx)
                .await?;
        // Bound new subject rows by the cluster-wide admitted request allowance.
        let global = if sensitive { 2_000 } else { 120_000 };
        let admitted = charge(
            &mut tx,
            window,
            if sensitive { "auth" } else { "edge" },
            "cluster",
            1,
            global,
        )
        .await?;
        let allowed = if admitted {
            charge(
                &mut tx,
                window,
                if sensitive { "auth_ip" } else { "ip" },
                &self.subject(window, ip),
                1,
                if sensitive { 20 } else { 600 },
            )
            .await?
        } else {
            false
        };
        tx.commit().await?;
        Ok(allowed)
    }
    pub async fn collection(&self, store: &Store, org: Uuid, events: usize) -> AppResult<()> {
        if !self.distributed {
            return Ok(());
        }
        let mut tx = store.pg.begin().await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        let window: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp())/60)::bigint")
                .fetch_one(&mut *tx)
                .await?;
        let allowed = charge(
            &mut tx,
            window,
            "collection",
            &org.to_string(),
            events.max(1) as i64,
            60_000,
        )
        .await?;
        tx.commit().await?;
        if !allowed {
            return Err(ApiError {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: "Workspace collection request budget reached; retry shortly",
            });
        }
        Ok(())
    }
    pub async fn report(
        &self,
        store: &Store,
        org: Uuid,
        local: TenantPermit,
    ) -> AppResult<ReportPermit> {
        if !self.distributed {
            return Ok(ReportPermit {
                _local: local,
                _shared: None,
            });
        }
        let mut tx = store.pg.begin().await?;
        sqlx::query("SET LOCAL statement_timeout='2s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(78422991)")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM report_leases WHERE expires_at<=clock_timestamp()")
            .execute(&mut *tx)
            .await?;
        let (all, tenant): (i64, i64) = sqlx::query_as(
            "SELECT count(*),count(*) FILTER(WHERE organization_id=$1) FROM report_leases",
        )
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
        if all >= 16 || tenant >= 4 {
            return Err(ApiError {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: "Reports are busy across the service; retry shortly",
            });
        }
        let id = Uuid::new_v4();
        // HTTP work is cancelled at 30 seconds; crash leases last 45 seconds.
        sqlx::query(
            "INSERT INTO report_leases VALUES($1,$2,clock_timestamp()+interval '45 seconds')",
        )
        .bind(id)
        .bind(org)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ReportPermit {
            _local: local,
            _shared: Some(ReportLease {
                pool: store.pg.clone(),
                id,
            }),
        })
    }
}
async fn charge(
    tx: &mut sqlx::PgConnection,
    window: i64,
    scope: &str,
    subject: &str,
    cost: i64,
    limit: i64,
) -> Result<bool, sqlx::Error> {
    let hits:i64=sqlx::query_scalar("INSERT INTO request_budgets(bucket_minute,scope,subject,hits) VALUES($1,$2,$3,$4) ON CONFLICT(bucket_minute,scope,subject) DO UPDATE SET hits=least(request_budgets.hits+$4,$5+1) RETURNING hits")
        .bind(window).bind(scope).bind(subject).bind(cost).bind(limit).fetch_one(tx).await?;
    Ok(hits <= limit)
}
pub struct ReportPermit {
    _local: TenantPermit,
    _shared: Option<ReportLease>,
}
struct ReportLease {
    pool: sqlx::PgPool,
    id: Uuid,
}
impl Drop for ReportLease {
    fn drop(&mut self) {
        let pool = self.pool.clone();
        let id = self.id;
        tokio::spawn(async move {
            if sqlx::query("DELETE FROM report_leases WHERE id=$1")
                .bind(id)
                .execute(&pool)
                .await
                .is_err()
            {
                tracing::warn!("Report lease release failed; bounded expiry will recover it");
            }
        });
    }
}
pub fn route_group(path: &str) -> &'static str {
    if path.starts_with("/v1/collect/") {
        "collection"
    } else if path.starts_with("/v1/native/") || path.starts_with("/native/v1/") {
        "native"
    } else if path.contains("auth") || path.contains("account") {
        "authentication"
    } else if path.starts_with("/owner") {
        "owner"
    } else if path.starts_with("/v1/websites/") || matches!(path, "/dashboard" | "/insights") {
        "reports"
    } else if path.starts_with("/assets/") {
        "assets"
    } else if path.starts_with("/internal/") {
        "internal"
    } else if matches!(path, "/healthz" | "/readyz") {
        "health"
    } else {
        "application"
    }
}
pub async fn prune(store: &Store) -> anyhow::Result<()> {
    sqlx::query("UPDATE operational_runs SET status='failed',completed_at=now(),details='{\"error\":\"operator_run_expired\"}' WHERE status='running' AND started_at<now()-interval '6 hours'").execute(&store.pg).await?;
    sqlx::query("DELETE FROM operational_runs WHERE status<>'running' AND started_at<now()-interval '365 days'").execute(&store.pg).await?;
    sqlx::query("DELETE FROM request_budgets WHERE bucket_minute<floor(extract(epoch FROM clock_timestamp())/60)::bigint-2").execute(&store.pg).await?;
    sqlx::query("DELETE FROM report_leases WHERE expires_at<clock_timestamp()")
        .execute(&store.pg)
        .await?;
    Ok(())
}
pub async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(expected) = &state.operations.metrics_token else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    let a = click_core::digest(expected);
    let b = click_core::digest(supplied);
    if a.bytes()
        .zip(b.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        != 0
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut body=String::from("# TYPE click_http_requests_total counter\n# TYPE click_http_request_duration_seconds histogram\n");
    {
        let rows = state.operations.metrics.lock().expect("metrics mutex");
        for ((route, method, class), row) in rows.iter() {
            let labels =
                format!("route=\"{route}\",method=\"{method}\",status_class=\"{class}xx\"");
            body+=&format!("click_http_requests_total{{{labels}}} {}\nclick_http_request_duration_seconds_sum{{{labels}}} {}\nclick_http_request_duration_seconds_count{{{labels}}} {}\n",row.count,row.seconds,row.count);
            for (index, bound) in BOUNDS.iter().enumerate() {
                body += &format!(
                    "click_http_request_duration_seconds_bucket{{{labels},le=\"{bound}\"}} {}\n",
                    row.buckets[index]
                );
            }
            body += &format!(
                "click_http_request_duration_seconds_bucket{{{labels},le=\"+Inf\"}} {}\n",
                row.count
            );
        }
    }
    body+=&format!("click_process_uptime_seconds {}\nclick_pg_pool_connections {}\nclick_pg_pool_idle_connections {}\nclick_distributed_limits_enabled {}\n",state.operations.started.elapsed().as_secs(),state.store.pg.size(),state.store.pg.num_idle(),u8::from(state.operations.distributed));
    let result=tokio::time::timeout(std::time::Duration::from_secs(3),async{
        let values:serde_json::Value=sqlx::query_scalar("SELECT json_build_object('pending',(SELECT count(*) FROM event_outbox WHERE state='pending'),'dead',(SELECT count(*) FROM event_outbox WHERE state='dead'),'oldest',(SELECT coalesce(extract(epoch FROM now()-min(accepted_at)),0) FROM event_outbox WHERE state='pending'),'mail_dead',(SELECT count(*) FROM mail_outbox WHERE state='dead'),'exports_old',(SELECT count(*) FROM export_jobs WHERE state='pending' AND created_at<now()-interval '1 hour'),'deletions_old',(SELECT count(*) FROM deletion_jobs WHERE state='pending' AND created_at<now()-interval '1 hour'),'backup_success',(SELECT coalesce(extract(epoch FROM max(completed_at)),0) FROM operational_runs WHERE kind='backup' AND status='succeeded'),'restore_success',(SELECT coalesce(extract(epoch FROM max(completed_at)),0) FROM operational_runs WHERE kind='restore' AND status='succeeded'))").fetch_one(&state.store.pg).await?;
        let checks:Vec<(String,Option<chrono::DateTime<chrono::Utc>>,Option<String>)>=sqlx::query_as("SELECT name,last_succeeded_at,last_error FROM maintenance_checks").fetch_all(&state.store.pg).await?;
        Ok::<_,sqlx::Error>((values,checks))
    }).await;
    if let Ok(Ok((values, checks))) = result {
        body += "click_dependency_up{backend=\"postgres\"} 1\n";
        for (key, name) in [
            ("pending", "outbox_pending"),
            ("dead", "outbox_dead"),
            ("oldest", "outbox_oldest_pending_seconds"),
            ("mail_dead", "mail_dead"),
            ("exports_old", "exports_old_pending"),
            ("deletions_old", "deletions_old_pending"),
            ("backup_success", "backup_last_success_timestamp_seconds"),
            ("restore_success", "restore_last_success_timestamp_seconds"),
        ] {
            body += &format!("click_{name} {}\n", values[key]);
        }
        for name in ["retention", "deletion", "exports", "mail"] {
            let check = checks.iter().find(|c| c.0 == name);
            body+=&format!("click_fulfillment_last_success_timestamp_seconds{{operation=\"{name}\"}} {}\nclick_fulfillment_failed{{operation=\"{name}\"}} {}\n",check.and_then(|c|c.1).map(|t|t.timestamp()).unwrap_or(0),u8::from(check.is_none_or(|c|c.2.is_some())));
        }
    } else {
        body += "click_dependency_up{backend=\"postgres\"} 0\n";
    }
    let ch = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        state
            .store
            .ch
            .query("SELECT toUInt64(1) AS count")
            .fetch_one::<click_store::Count>(),
    )
    .await;
    body += &format!(
        "click_dependency_up{{backend=\"clickhouse\"}} {}\n",
        u8::from(matches!(ch, Ok(Ok(_))))
    );
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}
