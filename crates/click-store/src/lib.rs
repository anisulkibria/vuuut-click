use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Utc};
use click_core::CleanEvent;
use clickhouse::{Client, Row};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

#[derive(Clone)]
pub struct Store {
    pub pg: PgPool,
    pub ch: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize, Row)]
pub struct EventRow {
    pub organization_id: String,
    pub website_id: String,
    pub event_id: String,
    pub timestamp: u32,
    pub session_id: String,
    pub visitor_id: String,
    pub name: String,
    pub path: String,
    pub country: String,
    pub referrer: String,
    pub utm_source: String,
    pub utm_medium: String,
    pub utm_campaign: String,
    pub device: String,
    pub browser: String,
    #[serde(default)]
    pub occurred_ms: i64,
    #[serde(default)]
    pub metric_name: String,
    #[serde(default)]
    pub metric_id: String,
    #[serde(default)]
    pub metric_value: f64,
    #[serde(default)]
    pub amount_minor: i64,
    #[serde(default)]
    pub currency: String,
    #[serde(default = "unknown_mode")]
    pub visitor_mode: String,
    #[serde(default = "unknown_os")]
    pub os: String,
    #[serde(default)]
    pub engagement_ms: u32,
    #[serde(default)]
    pub scroll_percent: u8,
    #[serde(default)]
    pub newsletter_id: String,
    #[serde(default)]
    pub recipient_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Website {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    pub origin: String,
    pub collection_id: String,
    pub verification_token: String,
    pub verified_at: Option<DateTime<Utc>>,
    pub active: bool,
    pub is_demo: bool,
}

pub struct Acceptance {
    pub inserted: usize,
    pub duplicates: usize,
}
#[derive(Debug, thiserror::Error)]
pub enum AcceptError {
    #[error("Monthly event allowance reached")]
    Quota,
    #[error("Collection is paused until billing is resolved")]
    BillingPaused,
    #[error("Website collection is paused or disabled")]
    Disabled,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Store {
    pub async fn connect(
        pg_url: &str,
        ch_url: &str,
        ch_user: &str,
        ch_password: &str,
        ch_database: &str,
    ) -> Result<Self> {
        let pg = PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(pg_url)
            .await?;
        let ch = Client::default()
            .with_url(ch_url)
            .with_user(ch_user)
            .with_password(ch_password)
            .with_database(ch_database)
            // Bound reader/FINAL fan-out as well as aggregate memory. Several
            // CTE readers can otherwise allocate large shared decompression
            // buffers outside the per-query aggregation limit.
            .with_option("max_threads", "2")
            .with_option("max_final_threads", "2")
            .with_option("max_execution_time", "10")
            .with_option("max_memory_usage", "268435456")
            .with_option("max_result_rows", "10000");
        Ok(Self { pg, ch })
    }
    /// Paired with the backup operator's exclusive advisory transaction lock.
    pub async fn maintenance_guard(
        &self,
    ) -> Result<Option<sqlx::Transaction<'static, sqlx::Postgres>>, sqlx::Error> {
        let mut tx = self.pg.begin().await?;
        let allowed: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock_shared(78422990)")
            .fetch_one(&mut *tx)
            .await?;
        Ok(allowed.then_some(tx))
    }
    pub async fn migrate(&self) -> Result<()> {
        let _maintenance = self
            .maintenance_guard()
            .await?
            .context("Maintenance is active; retry startup later")?;
        sqlx::migrate!("../../migrations/postgres")
            .run(&self.pg)
            .await?;
        // A transaction advisory lock also serializes ClickHouse DDL across startup processes.
        let mut tx = self.pg.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(78422915)")
            .execute(&mut *tx)
            .await?;
        for (version, sql) in [
            (
                "0001",
                include_str!("../../../migrations/clickhouse/0001_events.sql"),
            ),
            (
                "0002",
                include_str!("../../../migrations/clickhouse/0002_extended_events.sql"),
            ),
            (
                "0003",
                include_str!("../../../migrations/clickhouse/0003_richer_reports.sql"),
            ),
        ] {
            let checksum = click_core::digest(sql);
            let previous: Option<String> =
                sqlx::query_scalar("SELECT checksum FROM control_migrations WHERE version=$1")
                    .bind(version)
                    .fetch_optional(&mut *tx)
                    .await?;
            if let Some(previous) = previous {
                if previous != checksum {
                    anyhow::bail!("Applied ClickHouse migration {version} was changed");
                }
                continue;
            }
            for statement in sql.split(';').filter(|s| !s.trim().is_empty()) {
                self.ch
                    .query(statement)
                    .execute()
                    .await
                    .context("ClickHouse schema migration")?;
            }
            sqlx::query("INSERT INTO control_migrations(version,checksum) VALUES($1,$2)")
                .bind(version)
                .bind(checksum)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn collect_site(&self, token: &str) -> Result<Option<Website>, sqlx::Error> {
        sqlx::query_as("SELECT id,organization_id,name,origin,collection_id,verification_token,verified_at,active,is_demo FROM websites WHERE collection_id=$1 AND active=true AND is_demo=false AND deletion_requested_at IS NULL AND EXISTS(SELECT 1 FROM organizations WHERE id=websites.organization_id AND status='active')")
            .bind(token).fetch_optional(&self.pg).await
    }
    pub async fn accept(
        &self,
        site: &Website,
        events: Vec<CleanEvent>,
        country: &str,
        agent: &str,
    ) -> Result<Acceptance, AcceptError> {
        self.accept_checked(site, events, country, agent, None)
            .await
    }
    /// Integration credentials are rechecked under the acceptance lock so a
    /// revoked key cannot complete a request that waited behind its revocation.
    pub async fn accept_integration(
        &self,
        site: &Website,
        events: Vec<CleanEvent>,
        token_hash: &str,
    ) -> Result<Acceptance, AcceptError> {
        self.accept_checked(site, events, "ZZ", "backend", Some(token_hash))
            .await
    }
    async fn accept_checked(
        &self,
        site: &Website,
        events: Vec<CleanEvent>,
        country: &str,
        agent: &str,
        integration_hash: Option<&str>,
    ) -> Result<Acceptance, AcceptError> {
        if site.is_demo {
            return Err(AcceptError::Quota);
        }
        let mut tx = self.pg.begin().await?;
        let allowed: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock_shared(78422990)")
            .fetch_one(&mut *tx)
            .await?;
        if !allowed {
            return Err(AcceptError::Disabled);
        }
        // Serializes usage and duplicate decisions across concurrent batches for the organization.
        sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
            .bind(site.organization_id)
            .fetch_one(&mut *tx)
            .await?;
        if let Some(hash) = integration_hash {
            let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_keys WHERE website_id=$1 AND token_hash=$2 AND scope='integration' AND revoked_at IS NULL)")
                .bind(site.id).bind(hash).fetch_one(&mut *tx).await?;
            if !allowed {
                return Err(AcceptError::Disabled);
            }
        }
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM websites w JOIN organizations o ON o.id=w.organization_id WHERE w.id=$1 AND w.active AND NOT w.is_demo AND w.deletion_requested_at IS NULL AND w.verified_at IS NOT NULL AND o.status='active' AND NOT EXISTS(SELECT 1 FROM integration_mappings m JOIN native_platforms np ON np.id=m.platform_id WHERE m.website_id=w.id AND NOT np.active))").bind(site.id).fetch_one(&mut *tx).await?;
        if !active {
            return Err(AcceptError::Disabled);
        }
        let quota: Option<i64> = sqlx::query_scalar("SELECT p.monthly_events FROM subscriptions s JOIN plans p ON p.id=s.plan_id WHERE s.organization_id=$1 AND s.status IN ('free','test','active')")
            .bind(site.organization_id).fetch_optional(&mut *tx).await?;
        let now = Utc::now();
        let month = now.date_naive().with_day(1).expect("first day exists");
        sqlx::query(
            "INSERT INTO usage_records(organization_id,month) VALUES($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(site.organization_id)
        .bind(month)
        .execute(&mut *tx)
        .await?;
        let used: i64 = sqlx::query_scalar(
            "SELECT accepted_events FROM usage_records WHERE organization_id=$1 AND month=$2",
        )
        .bind(site.organization_id)
        .bind(month)
        .fetch_one(&mut *tx)
        .await?;
        let mut inserted = 0;
        let mut duplicates = 0;
        let agent = agent.to_ascii_lowercase();
        let device = if agent.contains("ipad") || agent.contains("tablet") {
            "Tablet"
        } else if agent.contains("mobile") {
            "Mobile"
        } else {
            "Desktop"
        };
        let browser = if agent.contains("edg/") {
            "Edge"
        } else if agent.contains("firefox/") {
            "Firefox"
        } else if agent.contains("chrome/") || agent.contains("crios/") {
            "Chrome"
        } else if agent.contains("safari/") {
            "Safari"
        } else {
            "Other"
        };
        for e in events {
            let row = EventRow {
                organization_id: site.organization_id.to_string(),
                website_id: site.id.to_string(),
                event_id: e.event_id.to_string(),
                timestamp: now.timestamp() as u32,
                session_id: e.session_id,
                visitor_id: e.visitor_id,
                name: e.name,
                path: e.path,
                country: country.to_owned(),
                referrer: e.referrer,
                utm_source: e.utm_source,
                utm_medium: e.utm_medium,
                utm_campaign: e.utm_campaign,
                device: device.into(),
                browser: browser.into(),
                visitor_mode: e.visitor_mode,
                os: operating_system(&agent).into(),
                engagement_ms: e.engagement.as_ref().map(|e| e.duration_ms).unwrap_or(0),
                scroll_percent: e.engagement.as_ref().map(|e| e.scroll_percent).unwrap_or(0),
                newsletter_id: e.newsletter_id,
                recipient_id: e.recipient_id,
                occurred_ms: e.occurred_at_ms.unwrap_or(now.timestamp_millis()),
                metric_name: e.vital.as_ref().map(|v| v.name.clone()).unwrap_or_default(),
                metric_id: e
                    .vital
                    .as_ref()
                    .map(|v| v.id.clone())
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| e.event_id.to_string()),
                metric_value: e.vital.map(|v| v.value).unwrap_or(0.0),
                amount_minor: e.amount_minor,
                currency: e.currency,
            };
            let result = sqlx::query("INSERT INTO event_outbox(website_id,event_id,organization_id,payload,accepted_at) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING")
                .bind(site.id).bind(e.event_id).bind(site.organization_id).bind(serde_json::to_value(row)?).bind(now).execute(&mut *tx).await?;
            if result.rows_affected() == 0 {
                duplicates += 1;
            } else {
                inserted += 1;
            }
        }
        // A retry of already accepted IDs stays acknowledged after a downgrade
        // or payment pause. Only newly inserted events need current allowance.
        if inserted > 0 {
            let quota = quota.ok_or(AcceptError::BillingPaused)?;
            if used + inserted as i64 > quota {
                return Err(AcceptError::Quota);
            }
        }
        sqlx::query("UPDATE usage_records SET accepted_events=accepted_events+$3 WHERE organization_id=$1 AND month=$2").bind(site.organization_id).bind(month).bind(inserted as i64).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Acceptance {
            inserted,
            duplicates,
        })
    }
    /// Holds locks during the bounded ClickHouse request. Other workers skip these rows.
    pub async fn drain_once(&self) -> Result<usize> {
        let mut tx = self.pg.begin().await?;
        let allowed: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock_shared(78422990)")
            .fetch_one(&mut *tx)
            .await?;
        if !allowed {
            return Ok(0);
        }
        let batch: Vec<(Uuid,Uuid,serde_json::Value)> = sqlx::query_as("SELECT website_id,event_id,payload FROM event_outbox WHERE state='pending' AND available_at<=now() ORDER BY accepted_at LIMIT 500 FOR UPDATE SKIP LOCKED")
            .fetch_all(&mut *tx).await?;
        if batch.is_empty() {
            tx.commit().await?;
            return Ok(0);
        }
        let mut rows = Vec::with_capacity(batch.len());
        for (_, _, payload) in &batch {
            let mut row = serde_json::from_value::<EventRow>(payload.clone())?;
            if row.occurred_ms == 0 {
                row.occurred_ms = row.timestamp as i64 * 1000;
            }
            rows.push(row);
        }
        let delivery = async {
            let mut insert = self.ch.insert("events")?.with_timeouts(
                Some(std::time::Duration::from_secs(10)),
                Some(std::time::Duration::from_secs(15)),
            );
            for row in &rows {
                insert.write(row).await?;
            }
            insert.end().await
        }
        .await;
        match delivery {
            Ok(()) => {
                for (site, event, _) in &batch {
                    sqlx::query("UPDATE event_outbox SET state='delivered',delivered_at=now(),payload=NULL WHERE website_id=$1 AND event_id=$2").bind(site).bind(event).execute(&mut *tx).await?;
                }
                tx.commit().await?;
                Ok(batch.len())
            }
            Err(error) => {
                for (site, event, _) in &batch {
                    sqlx::query("UPDATE event_outbox SET attempts=attempts+1,available_at=now()+make_interval(secs=>LEAST(60, power(2,LEAST(attempts,6))::int)),state=CASE WHEN attempts>=19 THEN 'dead' ELSE 'pending' END WHERE website_id=$1 AND event_id=$2").bind(site).bind(event).execute(&mut *tx).await?;
                }
                tx.commit().await?;
                Err(error.into())
            }
        }
    }
    pub async fn overview(&self, site: &Website, start: u32, end: u32) -> Result<Overview> {
        let totals: Totals = self.ch.query("SELECT uniqExactMerge(views) AS pageviews,uniqExactMerge(visitors) AS visitors,uniqExactMerge(sessions) AS sessions FROM pageviews_hourly WHERE organization_id=? AND website_id=? AND bucket>=toDateTime(?) AND bucket<toDateTime(?)")
            .bind(site.organization_id.to_string()).bind(site.id.to_string()).bind(start).bind(end).fetch_one().await?;
        let realtime: Count = self.ch.query("SELECT uniqExact(visitor_id) AS count FROM events FINAL WHERE organization_id=? AND website_id=? AND name='pageview' AND timestamp>=now()-INTERVAL 5 MINUTE AND timestamp<=now()")
            .bind(site.organization_id.to_string()).bind(site.id.to_string()).fetch_one().await?;
        let bounced: Count = self.ch.query("SELECT count() AS count FROM (SELECT session_id FROM events FINAL WHERE organization_id=? AND website_id=? AND name='pageview' AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) GROUP BY session_id HAVING count()=1)")
            .bind(site.organization_id.to_string()).bind(site.id.to_string()).bind(start).bind(end).fetch_one().await?;
        let timeseries = self.ch.query("SELECT toString(toDate(bucket)) AS label,uniqExactMerge(views) AS count FROM pageviews_hourly WHERE organization_id=? AND website_id=? AND bucket>=toDateTime(?) AND bucket<toDateTime(?) GROUP BY label ORDER BY label")
            .bind(site.organization_id.to_string()).bind(site.id.to_string()).bind(start).bind(end).fetch_all().await?;
        Ok(Overview {
            pageviews: totals.pageviews,
            visitors: totals.visitors,
            sessions: totals.sessions,
            realtime: realtime.count,
            bounce_rate: if totals.sessions == 0 {
                0.0
            } else {
                bounced.count as f64 * 100.0 / totals.sessions as f64
            },
            timeseries,
        })
    }
    pub async fn goal_count(
        &self,
        site: &Website,
        event: &str,
        start: u32,
        end: u32,
    ) -> Result<u64> {
        let count:Count=self.ch.query("SELECT count() AS count FROM events FINAL WHERE organization_id=? AND website_id=? AND name=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?)")
            .bind(site.organization_id.to_string()).bind(site.id.to_string()).bind(event).bind(start).bind(end).fetch_one().await?;
        Ok(count.count)
    }
    pub async fn goal_sessions(
        &self,
        site: &Website,
        event: &str,
        start: u32,
        end: u32,
    ) -> Result<u64> {
        let count:Count=self.ch.query("WITH base AS (SELECT session_id,name FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?)) SELECT uniqExact(session_id) AS count FROM base WHERE name=? AND session_id IN (SELECT session_id FROM base WHERE name='pageview')").bind(site.organization_id.to_string()).bind(site.id.to_string()).bind(start).bind(end).bind(event).fetch_one().await?;
        Ok(count.count)
    }
    pub async fn breakdown(
        &self,
        site: &Website,
        dimension: Dimension,
        start: u32,
        end: u32,
    ) -> Result<Vec<Breakdown>> {
        // Only compile-time identifiers enter SQL. All values are bound by the official client.
        let column = dimension.column();
        let filter = if matches!(dimension, Dimension::Events) {
            "name NOT IN ('pageview','web_vital','web_engagement')"
        } else {
            "name='pageview'"
        };
        let query = format!("SELECT {column} AS label,count() AS count FROM events FINAL WHERE organization_id=? AND website_id=? AND {filter} AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) GROUP BY label ORDER BY count DESC,label LIMIT 20");
        Ok(self
            .ch
            .query(&query)
            .bind(site.organization_id.to_string())
            .bind(site.id.to_string())
            .bind(start)
            .bind(end)
            .fetch_all()
            .await?)
    }
}

#[derive(Debug, Serialize, Deserialize, Row)]
pub struct Totals {
    pub pageviews: u64,
    pub visitors: u64,
    pub sessions: u64,
}
#[derive(Debug, Serialize, Deserialize, Row)]
pub struct Count {
    pub count: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, Row)]
pub struct Breakdown {
    pub label: String,
    pub count: u64,
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
#[derive(Clone, Copy)]
pub enum Dimension {
    Countries,
    Pages,
    Referrers,
    Campaigns,
    Devices,
    Browsers,
    Events,
    OperatingSystems,
}
impl Dimension {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "countries" => Some(Self::Countries),
            "pages" => Some(Self::Pages),
            "referrers" => Some(Self::Referrers),
            "campaigns" => Some(Self::Campaigns),
            "devices" => Some(Self::Devices),
            "browsers" => Some(Self::Browsers),
            "events" => Some(Self::Events),
            "operating_systems" => Some(Self::OperatingSystems),
            _ => None,
        }
    }
    fn column(self) -> &'static str {
        match self {
            Self::Countries => "country",
            Self::Pages => "path",
            Self::Referrers => "referrer",
            Self::Campaigns => "utm_campaign",
            Self::Devices => "device",
            Self::Browsers => "browser",
            Self::Events => "name",
            Self::OperatingSystems => "os",
        }
    }
}

/// Coarse categories only; the original user agent is never persisted.
fn operating_system(agent: &str) -> &'static str {
    if agent.contains("android") {
        "Android"
    } else if agent.contains("iphone") || agent.contains("ipad") || agent.contains("ipod") {
        "iOS"
    } else if agent.contains("windows") {
        "Windows"
    } else if agent.contains("cros") {
        "ChromeOS"
    } else if agent.contains("macintosh") || agent.contains("mac os") {
        "macOS"
    } else if agent.contains("linux") {
        "Linux"
    } else {
        "Unknown"
    }
}

fn unknown_mode() -> String {
    "unknown".into()
}
fn unknown_os() -> String {
    "Unknown".into()
}

#[cfg(test)]
mod os_tests {
    #[test]
    fn pending_events_from_older_builds_remain_unclassified() {
        let row:super::EventRow=serde_json::from_value(serde_json::json!({"organization_id":"org","website_id":"site","event_id":"event","timestamp":1,"session_id":"session","visitor_id":"visitor","name":"pageview","path":"/","country":"ZZ","referrer":"","utm_source":"","utm_medium":"","utm_campaign":"","device":"Desktop","browser":"Unknown"})).unwrap();
        assert_eq!(row.visitor_mode, "unknown");
        assert_eq!(row.os, "Unknown");
        assert_eq!(row.engagement_ms, 0);
    }
    #[test]
    fn overlapping_user_agents_resolve_to_coarse_categories() {
        for (agent, expected) in [
            ("mozilla iphone cpu iphone os like mac os x", "iOS"),
            ("mozilla linux android", "Android"),
            ("mozilla cros linux", "ChromeOS"),
            ("mozilla windows nt", "Windows"),
            ("mozilla macintosh mac os", "macOS"),
            ("mozilla linux x11", "Linux"),
            ("", "Unknown"),
        ] {
            assert_eq!(super::operating_system(agent), expected);
        }
    }
}
