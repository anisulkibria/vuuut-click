//! Advanced analytics, rendered in Rust from authorized queries.
use crate::{
    analytics::{self, Funnel, FunnelInput, FunnelStep},
    api, auth,
    dashboard::{self, esc},
    ApiError, AppResult, AppState,
};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use uuid::Uuid;
#[derive(Deserialize, Default)]
pub struct InsightsQuery {
    site: Option<Uuid>,
    view: Option<String>,
    days: Option<u32>,
    model: Option<String>,
    dimension: Option<String>,
}
const VIEWS: &[(&str, &str, &str)] = &[
    ("funnels", "Funnels", "From discovery to the next step."),
    ("journeys", "Journeys", "Every visit tells a story."),
    ("revenue", "Revenue", "Understand what brings value."),
    ("performance", "Web Vitals", "Make every visit feel better."),
];
fn panel(title: &str, body: &str) -> String {
    format!(
        r#"<section class="panel"><div class="panel-heading"><h2>{}</h2></div>{body}</section>"#,
        esc(title)
    )
}
fn table(headers: &[&str], rows: &str) -> String {
    let head = headers
        .iter()
        .map(|h| format!("<th>{}</th>", esc(h)))
        .collect::<String>();
    format!(
        r#"<div class="owner-table"><table><thead><tr>{head}</tr></thead><tbody>{rows}</tbody></table></div>"#
    )
}
pub async fn page(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<InsightsQuery>,
) -> AppResult<Response> {
    let user = match auth::user(&s, &h).await {
        Ok(u) => u,
        Err(e) if e.status == StatusCode::UNAUTHORIZED => {
            return Ok(Redirect::to("/signin").into_response())
        }
        Err(e) => return Err(e),
    };
    let sites = api::list_websites(&s, &h).await?;
    let Some(id) = q.site.or_else(|| sites.first().map(|w| w.id)) else {
        return Ok(Redirect::to("/websites").into_response());
    };
    let (site, role, _queries) = auth::authorize_report(&s, &h, id).await?;
    let view = q.view.as_deref().unwrap_or("funnels");
    let (_, label, title) = VIEWS
        .iter()
        .find(|(key, _, _)| *key == view)
        .ok_or_else(ApiError::not_found)?;
    let days = q.days.unwrap_or(7);
    let (start, end) = api::range(&api::Period { days: Some(days) })?;
    let mut nav = String::new();
    for (key, label, _) in VIEWS {
        nav.push_str(&format!(r#"<a href="/insights?view={key}&amp;site={id}&amp;days={days}" class="{}" {}>{label}</a>"#,if view==*key{"selected"}else{""},if view==*key{r#"aria-current="page""#}else{""}));
    }
    let mut options = String::new();
    for w in sites {
        options.push_str(&format!(
            r#"<option value="{}" {}>{}</option>"#,
            w.id,
            if w.id == id { "selected" } else { "" },
            esc(&w.name)
        ));
    }
    let mut periods = String::new();
    for d in [1, 7, 30, 90] {
        periods.push_str(&format!(
            r#"<option value="{d}" {}>{d} days</option>"#,
            if d == days { "selected" } else { "" }
        ));
    }
    let mut content = String::new();
    if site.is_demo {
        content.push_str(r#"<div class="notice"><strong>Demo data</strong> · Sample journeys, goals and performance data for exploring the application. These events are excluded from billable usage.</div>"#);
    }
    match view {
        "funnels" => {
            let funnels:Vec<Funnel>=sqlx::query_as("SELECT id,name,steps,window_seconds FROM funnels WHERE website_id=$1 ORDER BY created_at LIMIT 100").bind(id).fetch_all(&s.store.pg).await?;
            if funnels.is_empty() {
                content.push_str(&panel("See the steps that count","<p>Create a funnel using page paths and custom events to measure sessions that reach each step.</p>"));
            }
            for funnel in funnels {
                let report = analytics::funnel_report(&s.store, &site, funnel, start, end).await?;
                let mut rows = String::new();
                let first = report.steps.first().map(|s| s.sessions).unwrap_or(0);
                for (index, step) in report.steps.iter().enumerate() {
                    let width = if first == 0 {
                        0.0
                    } else {
                        step.sessions as f64 / first as f64 * 100.0
                    };
                    rows.push_str(&format!(r#"<tr><td>{}. {}<small class="muted"> · {}</small><svg class="row-bar" viewBox="0 0 100 3" preserveAspectRatio="none" aria-hidden="true"><rect width="{width}" height="3"/></svg></td><td>{}</td><td>{:.1}%</td><td>{}</td></tr>"#,index+1,esc(&step.value),esc(&step.kind),step.sessions,step.conversion_rate,step.drop_off));
                }
                let mut body = table(&["Step", "Sessions", "Conversion", "Drop-off"], &rows);
                body.push_str(&format!("<p class=muted>Ordered within {} hours. Counts are distinct sessions, with a strictly increasing event time.</p>",report.window_seconds/3600));
                if role.can_edit_goals() {
                    // Pre-filled with the funnel as it stands, in the same
                    // `kind:value` lines the create form accepts, so fixing a
                    // step is editing the steps rather than rebuilding the
                    // funnel from memory. Folded away by default: the report
                    // is what the panel is for.
                    let steps = report
                        .steps
                        .iter()
                        .map(|step| format!("{}:{}", esc(&step.kind), esc(&step.value)))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let windows = [(3600, "1 hour"), (86400, "24 hours"), (604800, "7 days")]
                        .iter()
                        .map(|(seconds, label)| {
                            let selected = if *seconds == report.window_seconds {
                                " selected"
                            } else {
                                ""
                            };
                            format!(r#"<option value="{seconds}"{selected}>{label}</option>"#)
                        })
                        .collect::<String>();
                    body.push_str(&format!(
                        r#"<details class="row-edit"><summary>Edit this funnel</summary><form action="/insights/action" method="post" class="stack"><input type="hidden" name="action" value="update"><input type="hidden" name="site" value="{id}"><input type="hidden" name="funnel" value="{funnel}"><label>Funnel name<input name="name" maxlength="100" required value="{name}"></label><label>Steps, one per line<textarea name="steps" rows="4" required maxlength="9000">{steps}</textarea></label><p class="muted">Use <code>page:/your-path</code> or <code>event:your_event</code>. Include 2–8 steps.</p><label>Conversion window<select name="window_seconds">{windows}</select></label><button class="btn primary">Save funnel</button></form></details><form action="/insights/action" method="post" onsubmit="return confirm('Remove this funnel? Your sessions and events are not affected.')"><input type="hidden" name="action" value="delete"><input type="hidden" name="site" value="{id}"><input type="hidden" name="funnel" value="{funnel}"><button class="btn outlined small">Remove funnel</button></form>"#,
                        funnel = report.id,
                        name = esc(&report.name),
                    ));
                }
                content.push_str(&panel(&report.name, &body));
            }
            if role.can_edit_goals() {
                content.push_str(&panel("Create a funnel",&format!(r#"<form action="/insights/action" method="post" class="stack"><input type="hidden" name="action" value="create"><input type="hidden" name="site" value="{id}"><label>Funnel name<input name="name" maxlength="100" required placeholder="From reader to subscriber"></label><label>Steps, one per line<textarea name="steps" rows="4" required maxlength="9000" placeholder="page:/&#10;page:/pricing&#10;event:trial_started"></textarea></label><p class="muted">Use <code>page:/your-path</code> or <code>event:your_event</code>. Include 2–8 steps.</p><label>Conversion window<select name="window_seconds"><option value="3600">1 hour</option><option value="86400" selected>24 hours</option><option value="604800">7 days</option></select></label><button class="btn primary">Create funnel</button></form>"#)));
            }
        }
        "journeys" => {
            let journeys = analytics::journey_data(&s.store, &site, start, end).await?;
            if journeys.is_empty() {
                content.push_str(&panel("A story starts with a visit","<p>Accepted page views and custom events will appear here as session journeys.</p>"));
            }
            for journey in journeys {
                let mut steps = String::new();
                for (time, path, name) in &journey.steps {
                    let time = chrono::DateTime::from_timestamp_millis(*time)
                        .ok_or_else(|| ApiError::internal("Invalid event occurrence time"))?;
                    steps.push_str(&format!(
                        r#"<li><time>{}</time><span>{}</span><small>{}</small></li>"#,
                        time.format("%H:%M:%S UTC"),
                        esc(path),
                        esc(name)
                    ));
                }
                content.push_str(&format!(r#"<section class="panel"><div class="panel-heading"><h2>Session {}</h2><span class="badge">{} · {} events</span></div><ol class="journey-list">{steps}</ol><p class="muted">Showing the first {} events of this pseudonymous session.</p></section>"#,esc(&journey.session_id.chars().take(10).collect::<String>()),esc(&journey.country),journey.events,journey.steps.len()));
            }
            content.push_str("<p class=muted>The 25 most recently active sessions are shown. Random identifiers do not identify individual people.</p>");
        }
        "revenue" => {
            let model = q.model.as_deref().unwrap_or("last_non_direct");
            let dimension = q.dimension.as_deref().unwrap_or("source");
            let rows =
                analytics::revenue_data(&s.store, &site, start, end, model, dimension).await?;
            let mut models = String::new();
            for (key, label) in [
                ("first_touch", "First touch"),
                ("last_touch", "Last touch"),
                ("last_non_direct", "Last non-direct"),
            ] {
                models.push_str(&format!(
                    r#"<option value="{key}" {}>{label}</option>"#,
                    if key == model { "selected" } else { "" }
                ));
            }
            let mut dimensions = String::new();
            for (key, label) in [
                ("source", "Source"),
                ("campaign", "Campaign"),
                ("landing", "Landing page"),
            ] {
                dimensions.push_str(&format!(
                    r#"<option value="{key}" {}>{label}</option>"#,
                    if key == dimension { "selected" } else { "" }
                ));
            }
            content.push_str(&format!(r#"<form class="settings-switch" method="get" action="/insights"><input type="hidden" name="view" value="revenue"><input type="hidden" name="site" value="{id}"><input type="hidden" name="days" value="{days}"><label>Attribution model<select name="model">{models}</select></label><label>Group revenue by<select name="dimension">{dimensions}</select></label><button class="btn outlined small">Apply attribution</button></form>"#));
            let mut body = String::new();
            for row in rows {
                body.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    esc(&row.label),
                    esc(&row.currency),
                    esc(&analytics::money(&row.total_minor, &row.currency)?),
                    row.payments
                ));
            }
            if body.is_empty() {
                body =
                    "<tr><td colspan=4>No backend payment events in this period.</td></tr>".into();
            }
            content.push_str(&panel(
                "Revenue attribution",
                &table(
                    &["Attribution", "Currency", "Net revenue", "Payments"],
                    &body,
                ),
            ));
            content.push_str(&panel("Connect authoritative revenue",&format!(r#"<p>Send payment and refund events from your backend using a website integration credential. Net revenue deducts recorded refunds, with a 30-day session lookback. Currencies are reported separately.</p><a class="btn outlined small" href="/settings?tab=keys&amp;org={}&amp;site={id}">Manage integration credentials →</a>"#,site.organization_id)));
        }
        "performance" => {
            let metrics = analytics::vital_data(&s.store, &site, start, end).await?;
            let mut body = String::new();
            for metric in metrics {
                let unit = if metric.metric == "CLS" { "" } else { " ms" };
                let quality = match metric.metric.as_str() {
                    "LCP" => quality(metric.p75, 2500.0, 4000.0),
                    "INP" => quality(metric.p75, 200.0, 500.0),
                    "CLS" => quality(metric.p75, 0.1, 0.25),
                    _ => "Measured",
                };
                body.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{:.2}{unit}</td><td>{:.2}{unit}</td><td>{:.2}{unit}</td><td>{quality}</td></tr>",esc(&metric.metric),metric.samples,metric.p50,metric.p75,metric.p95));
            }
            if body.is_empty() {
                body = "<tr><td colspan=6>No browser performance samples in this period.</td></tr>"
                    .into();
            }
            content.push_str(&panel(
                "The experience behind each visit",
                &table(
                    &[
                        "Metric",
                        "Samples",
                        "Median",
                        "75th percentile",
                        "95th percentile",
                        "At p75",
                    ],
                    &body,
                ),
            ));
            content.push_str(&panel("Measure Web Vitals",r#"<p>Load the optional <code>vitals.js</code> addon immediately after your tracker script. It measures LCP, INP, CLS, FCP and TTFB using the web-vitals library. Your tracker consent settings still apply.</p><pre><code>&lt;script defer src="YOUR_ANALYTICS_ORIGIN/vitals.js"&gt;&lt;/script&gt;</code></pre><p class="muted">Percentiles use the latest report per metric instance. Browsers report supported metrics when a measurement is available; INP requires an interaction.</p>"#));
        }
        _ => {}
    }
    let body = format!(
        r#"<header class="topbar"><div class="breadcrumb">Your workspace <span>/</span> {label}</div><a class="status" href="/dashboard?site={id}">Overview →</a></header><div class="page"><div class="page-heading"><div><span class="eyebrow">FOLLOW THE DETAILS</span><h1>{title}</h1><p>Deeper insights for {}.</p></div><a class="btn outlined" href="/insights?site={id}&amp;view={view}&amp;days={days}">↻ Refresh</a></div><div class="toolbar"><form method="get" action="/insights"><input type="hidden" name="view" value="{view}"><label class="sr-only" for="insight-site">Website</label><select id="insight-site" name="site">{options}</select><label class="sr-only" for="insight-period">Period</label><select id="insight-period" name="days">{periods}</select><button class="btn small">Apply</button></form><span class="muted">All dates and times in UTC</span></div><nav class="settings-tabs" aria-label="Advanced analytics">{nav}</nav>{content}</div>"#,
        esc(&site.name)
    );
    Ok(dashboard::shell(&h, &user.email, "insights", Some(id), body).into_response())
}
fn quality(value: f64, good: f64, poor: f64) -> &'static str {
    if value <= good {
        "Good"
    } else if value <= poor {
        "Needs improvement"
    } else {
        "Poor"
    }
}
/// The textarea's `kind:value` lines, as funnel steps.
///
/// Shared by create and update so the two cannot disagree about what a step
/// line means; `analytics::validate` then has the final say on the values.
fn parse_steps(steps: Option<String>) -> AppResult<Vec<FunnelStep>> {
    steps
        .ok_or_else(|| ApiError::bad("Add funnel steps"))?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (kind, value) = line
                .trim()
                .split_once(':')
                .ok_or_else(|| ApiError::bad("Use page:/path or event:name for each step"))?;
            Ok(FunnelStep {
                kind: kind.trim().into(),
                value: value.trim().into(),
            })
        })
        .collect()
}

#[derive(Deserialize)]
pub struct Action {
    action: String,
    site: Uuid,
    funnel: Option<Uuid>,
    name: Option<String>,
    steps: Option<String>,
    window_seconds: Option<i32>,
}
pub async fn action(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<Action>,
) -> AppResult<Redirect> {
    match input.action.as_str() {
        "create" => {
            let steps = parse_steps(input.steps)?;
            analytics::create_funnel(
                &s,
                &h,
                input.site,
                FunnelInput {
                    name: input
                        .name
                        .ok_or_else(|| ApiError::bad("Name your funnel"))?,
                    steps,
                    window_seconds: input.window_seconds.unwrap_or(86400),
                },
            )
            .await?;
        }
        "update" => {
            let steps = parse_steps(input.steps)?;
            analytics::update_funnel(
                &s,
                &h,
                input.site,
                input.funnel.ok_or_else(ApiError::not_found)?,
                FunnelInput {
                    name: input
                        .name
                        .ok_or_else(|| ApiError::bad("Name your funnel"))?,
                    steps,
                    window_seconds: input.window_seconds.unwrap_or(86400),
                },
            )
            .await?;
        }
        "delete" => {
            analytics::remove_funnel(
                &s,
                &h,
                input.site,
                input.funnel.ok_or_else(ApiError::not_found)?,
            )
            .await?
        }
        _ => return Err(ApiError::bad("Unknown funnel action")),
    }
    Ok(Redirect::to(&format!(
        "/insights?view=funnels&site={}",
        input.site
    )))
}
