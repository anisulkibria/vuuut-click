//! Explicit local-only sample dataset. Real websites, collection and metering are untouched.
use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use click_core::digest;
use click_store::{EventRow, Store};
use uuid::Uuid;

pub async fn seed(store: &Store, email: &str) -> Result<Uuid> {
    let org:Uuid=sqlx::query_scalar("SELECT m.organization_id FROM memberships m JOIN users u ON u.id=m.user_id WHERE u.email=$1 ORDER BY m.organization_id LIMIT 1")
        .bind(email.trim().to_lowercase()).fetch_optional(&store.pg).await?.context("Customer must have a workspace before seeding a demo")?;
    let mut tx = store.pg.begin().await?;
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
    let site: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM websites WHERE organization_id=$1 AND is_demo")
            .bind(org)
            .fetch_optional(&mut *tx)
            .await?;
    let site = match site {
        Some(id) => id,
        None => {
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO websites(id,organization_id,name,origin,collection_id,verification_token,verified_at,is_demo) VALUES($1,$2,'Vuuut Journal · Demo','https://demo.vuuut.test',$3,$4,now(),true)")
            .bind(id).bind(org).bind(format!("demo_{}",crate::auth::secret())).bind(crate::auth::secret()).execute(&mut *tx).await?;
            id
        }
    };
    // Full reset is restricted to this explicitly marked demonstration website.
    for table in ["events", "pageviews_hourly"] {
        store.ch.query(&format!("ALTER TABLE {table} DELETE WHERE organization_id=? AND website_id=? SETTINGS mutations_sync=2"))
            .bind(org.to_string()).bind(site.to_string()).execute().await?;
    }
    let now = Utc::now();
    let midnight = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let mut insert = store.ch.insert("events")?;
    let paths = [
        "/",
        "/journal/a-little-clarity",
        "/features",
        "/pricing",
        "/guides/growing-your-audience",
        "/about",
        "/newsletter",
        "/contact",
    ];
    let sources = [
        ("google.com", "google", "organic", "Search discovery"),
        ("", "", "", ""),
        ("github.com", "github", "referral", "Community launch"),
        ("", "newsletter", "email", "November newsletter"),
        ("linkedin.com", "linkedin", "social", "Creator stories"),
        (
            "duckduckgo.com",
            "duckduckgo",
            "organic",
            "Search discovery",
        ),
    ];
    let countries = [
        "US", "BD", "GB", "DE", "IN", "CA", "AU", "NL", "FR", "JP", "US", "GB",
    ];
    let mut sequence = 0usize;
    for day in 0..30_i64 {
        let sessions = 18 + day as usize * 3;
        for i in 0..sessions {
            let session = digest(&format!("{site}:day:{day}:session:{i}"));
            let visitor = digest(&format!("{site}:visitor:{}", (day as usize * 19 + i) % 490));
            let source = sources[(i + day as usize) % sources.len()];
            let base = (midnight - Duration::days(29 - day)).timestamp() as u32;
            let available = if day == 29 {
                (now.timestamp() as u32 - base).max(1)
            } else {
                86400
            };
            let time = if day == 29 && i + 8 >= sessions {
                now.timestamp() as u32 - (sessions - 1 - i) as u32 * 23
            } else {
                base + ((i as u32 * 1847 + day as u32 * 137) % available)
            };
            let pages = if i % 5 == 0 { 1 } else { 2 + i % 3 };
            for step in 0..pages {
                sequence += 1;
                let stamp = (time + step as u32 * 13).min(now.timestamp() as u32);
                let row = EventRow {
                    organization_id: org.to_string(),
                    website_id: site.to_string(),
                    event_id: digest(&format!("{site}:event:{sequence}")),
                    timestamp: stamp,
                    session_id: session.clone(),
                    visitor_id: visitor.clone(),
                    name: "pageview".into(),
                    path: if i % 3 == 0 && step == 0 {
                        "/"
                    } else if i % 3 == 0 && step == 1 {
                        "/pricing"
                    } else {
                        paths[(i + step + day as usize) % paths.len()]
                    }
                    .into(),
                    country: countries[(i * 7 + day as usize) % countries.len()].into(),
                    referrer: source.0.into(),
                    utm_source: source.1.into(),
                    utm_medium: source.2.into(),
                    utm_campaign: source.3.into(),
                    device: if i % 5 < 2 {
                        "Mobile"
                    } else if i % 9 == 0 {
                        "Tablet"
                    } else {
                        "Desktop"
                    }
                    .into(),
                    browser: ["Chrome", "Safari", "Firefox", "Edge"][i % 4].into(),
                    occurred_ms: stamp as i64 * 1000,
                    metric_name: String::new(),
                    metric_id: String::new(),
                    metric_value: 0.0,
                    amount_minor: 0,
                    currency: String::new(),
                };
                insert.write(&row).await?;
                if step == pages - 1 && i % 3 == 0 {
                    let mut event = row.clone();
                    sequence += 1;
                    event.event_id = digest(&format!("{site}:event:{sequence}"));
                    event.name = [
                        "newsletter_signup",
                        "trial_started",
                        "guide_download",
                        "pricing_click",
                    ][i % 4]
                        .into();
                    event.occurred_ms += 500;
                    insert.write(&event).await?;
                }
                if step == 0 {
                    // Independent metric instances, with a mix of good and slower visits.
                    for (metric, value) in [
                        ("LCP", 1150.0 + (i * 137 % 3900) as f64),
                        ("INP", 65.0 + (i * 31 % 460) as f64),
                        ("CLS", (i * 7 % 30) as f64 / 100.0),
                        ("FCP", 680.0 + (i * 97 % 2100) as f64),
                        ("TTFB", 95.0 + (i * 43 % 850) as f64),
                    ] {
                        let mut vital = row.clone();
                        vital.event_id = digest(&format!("{site}:{session}:{metric}"));
                        vital.name = "web_vital".into();
                        vital.metric_name = metric.into();
                        vital.metric_id = vital.event_id.clone();
                        vital.metric_value = value;
                        vital.occurred_ms += 750;
                        insert.write(&vital).await?;
                    }
                }
                if step == pages - 1 && i % 8 == 0 {
                    let mut payment = row.clone();
                    payment.event_id = digest(&format!("{site}:{session}:payment"));
                    payment.name = "server.payment".into();
                    payment.occurred_ms += 900;
                    payment.amount_minor = [1900, 2900, 4900][i % 3];
                    payment.currency = "USD".into();
                    insert.write(&payment).await?;
                    if i % 40 == 0 {
                        payment.event_id = digest(&format!("{site}:{session}:refund"));
                        payment.name = "server.refund".into();
                        payment.amount_minor = -500;
                        payment.occurred_ms += 100;
                        insert.write(&payment).await?;
                    }
                }
            }
        }
    }
    insert.end().await?;
    for (name, event) in [
        ("Newsletter signups", "newsletter_signup"),
        ("Trial starts", "trial_started"),
        ("Guide downloads", "guide_download"),
        ("Pricing interest", "pricing_click"),
    ] {
        sqlx::query("INSERT INTO goals(id,website_id,name,event_name) VALUES($1,$2,$3,$4) ON CONFLICT(website_id,name) DO UPDATE SET event_name=excluded.event_name")
            .bind(Uuid::new_v4()).bind(site).bind(name).bind(event).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO funnels(id,website_id,name,steps,window_seconds) VALUES($1,$2,'From visitor to trial',$3,86400) ON CONFLICT(website_id,name) DO UPDATE SET steps=excluded.steps,window_seconds=excluded.window_seconds")
        .bind(Uuid::new_v4()).bind(site).bind(serde_json::json!([{"kind":"page","value":"/"},{"kind":"page","value":"/pricing"},{"kind":"event","value":"trial_started"}])).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(site)
}
