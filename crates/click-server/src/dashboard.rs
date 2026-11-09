//! Native Rust HTML/SVG dashboard. No client framework or dashboard build step.
use crate::{api, auth, manage, ApiError, AppResult, AppState};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use chrono::{Duration, Utc};
use click_store::{Breakdown, Dimension, Website};
use serde::Deserialize;
use uuid::Uuid;

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn icon(name: &str) -> String {
    let path=match name {
        "overview"=>"M3 3h7v7H3zm11 0h7v7h-7zM3 14h7v7H3zm11 0h7v7h-7z",
        "audience"|"users"=>"M16 11c1.66 0 3-1.34 3-3s-1.34-3-3-3-3 1.34-3 3 1.34 3 3 3zM8 11c1.66 0 3-1.34 3-3S9.66 5 8 5 5 6.34 5 8s1.34 3 3 3zm0 2c-2.33 0-7 1.17-7 3.5V19h14v-2.5C15 14.17 10.33 13 8 13zm8 0c-.29 0-.62.02-.97.05C16.19 13.89 17 15.02 17 16.5V19h6v-2.5c0-2.33-4.67-3.5-7-3.5z",
        "acquisition"=>"M3 13h4v8H3zm7-6h4v14h-4zm7-6h4v20h-4z",
        "content"|"workspaces"=>"M14 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V8zm-1 7V3.5L18.5 9zM8 13h8v2H8zm0 4h8v2H8z",
        "events"|"delivery"=>"m13 2-3 8H5l6 12 3-8h5z",
        "sites"=>"M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm6.9 6h-3.1a16 16 0 0 0-1.5-3.6A8 8 0 0 1 18.9 8zM12 4c.9 1.2 1.5 2.6 1.9 4h-3.8c.4-1.4 1-2.8 1.9-4zM4.3 10h3.4a15 15 0 0 0 0 4H4.3a8 8 0 0 1 0-4zm.8 6h3.1c.3 1.3.8 2.5 1.5 3.6A8 8 0 0 1 5.1 16zM12 20c-.9-1.2-1.5-2.6-1.9-4h3.8c-.4 1.4-1 2.8-1.9 4zm2.3-6H9.7a13 13 0 0 1 0-4h4.6a13 13 0 0 1 0 4zm0 5.6c.7-1.1 1.2-2.3 1.5-3.6h3.1a8 8 0 0 1-4.6 3.6zM16.3 14a15 15 0 0 0 0-4h3.4a8 8 0 0 1 0 4zM5.1 8a8 8 0 0 1 4.6-3.6A16 16 0 0 0 8.2 8z",
        "subscriptions"=>"M20 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 14H4v-6h16v6zm0-10H4V6h16v2z",
        "audit"=>"M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm1 5v4.6l3.2 1.9-1 1.7-4.2-2.5V7z",
        "arrow"=>"M5 11h11.17l-4.59-4.59L13 5l7 7-7 7-1.41-1.41L16.17 13H5z",
        "check"=>"m9 16.17-4.17-4.17-1.42 1.41L9 19 12-12-1.41-1.41z",
        _=>"M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm-1 5h2v2h-2zm0 4h2v6h-2z",
    };
    format!(
        r#"<svg class="icon" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="{path}"/></svg>"#
    )
}
pub(crate) fn document(title: &str, h: &HeaderMap, body: &str) -> Html<String> {
    let theme = auth::cookie_value(h, "click_theme")
        .filter(|s| matches!(s.as_str(), "light" | "dark"))
        .unwrap_or_else(|| "system".into());
    Html(format!(
        r##"<!doctype html><html lang="en" class="{theme}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light dark"><title>{} · Vuuut Click</title><link rel="stylesheet" href="/assets/app.css"></head><body><a class="skip" href="#main">Skip to content</a>{body}</body></html>"##,
        esc(title)
    ))
}
pub(crate) fn appearance_controls(action: &str) -> String {
    format!(
        r#"<form action="{}" method="post" class="theme-switch" aria-label="Color scheme"><button name="theme" value="light" title="Light theme">☀<span class="sr-only">Light</span></button><button name="theme" value="system" title="System theme">◐<span class="sr-only">System</span></button><button name="theme" value="dark" title="Dark theme">☾<span class="sr-only">Dark</span></button></form>"#,
        esc(action)
    )
}

pub(crate) fn navigation_link(href: &str, label: &str, glyph: &str, active: bool) -> String {
    format!(
        r#"<a href="{}" class="nav-item {}" aria-label="{}" {}>{}<span>{}</span></a>"#,
        esc(href),
        if active { "selected" } else { "" },
        esc(label),
        if active { r#"aria-current="page""# } else { "" },
        icon(glyph),
        esc(label)
    )
}

pub(crate) struct Sidebar<'a> {
    pub title: &'a str,
    pub heading: &'a str,
    pub home: &'a str,
    pub navigation: String,
    pub utilities: String,
    pub appearance: &'a str,
    pub logout: &'a str,
    pub logout_label: &'a str,
}

pub(crate) fn with_sidebar(
    h: &HeaderMap,
    email: &str,
    sidebar: Sidebar<'_>,
    body: String,
) -> Html<String> {
    let avatar = esc(&email
        .chars()
        .next()
        .unwrap_or('V')
        .to_uppercase()
        .to_string());
    document(
        sidebar.title,
        h,
        &format!(
            r#"<div class="app"><aside class="sidebar"><a class="brand" href="{}"><span class="brand-mark">v<span>↗</span></span><span>vuuut<span class="brand-sub">CLICK</span></span></a><div class="workspace-label">{}</div><nav aria-label="{}">{}</nav><div class="nav-divider"></div>{}<div class="sidebar-bottom"><div class="privacy-note"><span class="tiny-dot"></span> Your data. Your infrastructure.<small>Independent analytics, powered by Rust.</small></div>{}<div class="profile"><span class="avatar">{avatar}</span><span class="email">{}</span><form action="{}" method="post"><button class="logout" title="{}" aria-label="{}">↪</button></form></div></div></aside><main id="main">{body}</main></div>"#,
            esc(sidebar.home),
            esc(sidebar.heading),
            esc(sidebar.title),
            sidebar.navigation,
            sidebar.utilities,
            appearance_controls(sidebar.appearance),
            esc(email),
            esc(sidebar.logout),
            esc(sidebar.logout_label),
            esc(sidebar.logout_label)
        ),
    )
}

pub(crate) fn shell(
    h: &HeaderMap,
    email: &str,
    active: &str,
    site: Option<Uuid>,
    body: String,
) -> Html<String> {
    let mut navigation = String::new();
    for (key, label) in [
        ("overview", "Overview"),
        ("audience", "Audience"),
        ("acquisition", "Acquisition"),
        ("content", "Content"),
        ("events", "Events & goals"),
    ] {
        let href = site
            .map(|id| format!("/dashboard?site={id}&view={key}"))
            .unwrap_or_else(|| "/websites".into());
        navigation.push_str(&navigation_link(&href, label, key, active == key));
    }
    let insights_url = site
        .map(|id| format!("/insights?site={id}"))
        .unwrap_or_else(|| "/insights".into());
    navigation.push_str(&navigation_link(
        &insights_url,
        "Advanced analytics",
        "acquisition",
        active == "insights",
    ));
    let utilities = navigation_link("/websites", "Websites & setup", "sites", active == "sites")
        + &navigation_link(
            "/settings",
            "Workspace & settings",
            "workspaces",
            active == "settings",
        )
        + &navigation_link("/v1/openapi.json", "API reference", "info", false);
    with_sidebar(
        h,
        email,
        Sidebar {
            title: "Analytics",
            heading: "YOUR WORKSPACE",
            home: "/dashboard",
            navigation,
            utilities,
            appearance: "/appearance",
            logout: "/auth/logout",
            logout_label: "Sign out",
        },
        format!(r#"<div class="customer-console">{body}</div>"#),
    )
}
fn auth_page(h: &HeaderMap, signup: bool, error: Option<&str>) -> Html<String> {
    let (title, subtitle, action, button, other) = if signup {
        (
            "Make every visit count.",
            "Understand your audience. Build something that lasts.",
            "/auth/register",
            "Create your workspace",
            r#"Already have an account? <a href="/signin">Sign in</a>"#,
        )
    } else {
        (
            "A clearer view starts here.",
            "Welcome back to your corner of the internet.",
            "/auth/login",
            "Sign in",
            r#"New to Vuuut Click? <a href="/signup">Create a workspace</a>"#,
        )
    };
    let error = error
        .map(|e| format!(r#"<p class="alert" role="alert">{}</p>"#, esc(e)))
        .unwrap_or_default();
    let workspace = if signup {
        r#"<label>Workspace name<input name="organization" autocomplete="organization" maxlength="100" required placeholder="Your studio"></label>"#
    } else {
        ""
    };
    document(
        if signup { "Create account" } else { "Sign in" },
        h,
        &format!(
            r#"<div class="auth-layout"><section class="auth-story"><a class="brand" href="/"><span class="brand-mark">v<span>↗</span></span><span>vuuut<span class="brand-sub">CLICK</span></span></a><div><span class="eyebrow">A LITTLE CLARITY GOES A LONG WAY</span><h1>Your audience.<br>Your next big idea.</h1><p>Thoughtful analytics for people<br>who care about what they create.</p><div class="story-art" aria-hidden="true"><svg viewBox="0 0 500 210"><path class="art-grid" d="M0 50H500M0 100H500M0 150H500M50 0V210M150 0V210M250 0V210M350 0V210M450 0V210"/><path class="art-fill" d="M0 180Q60 180 95 150T170 130T245 100T320 100T400 45T500 20V210H0Z"/><path class="art-line" d="M0 180Q60 180 95 150T170 130T245 100T320 100T400 45T500 20"/></svg><span>Good things grow with understanding.</span></div></div><p class="auth-foot">Built with Rust. Designed around you.</p></section><main id="main" class="auth-main"><div class="auth-card"><span class="pill">Privacy-conscious analytics</span><h2>{title}</h2><p>{subtitle}</p>{error}<form action="{action}" method="post" class="stack">{workspace}<label>Email address<input type="email" name="email" autocomplete="email" maxlength="254" required placeholder="you@example.com"></label><label>Password<input type="password" name="password" autocomplete="{}" minlength="12" maxlength="256" required placeholder="At least 12 characters"></label><button class="btn primary" type="submit">{button} {}</button></form><p class="auth-other">{other}</p><p class="auth-other"><a href="/forgot-password">Forgot your password?</a></p></div><div class="auth-theme">{}</div></main></div>"#,
            if signup {
                "new-password"
            } else {
                "current-password"
            },
            icon("arrow"),
            appearance_controls("/appearance")
        ),
    )
}
pub async fn signin(h: HeaderMap) -> Html<String> {
    auth_page(&h, false, None)
}
pub async fn signup(h: HeaderMap) -> Html<String> {
    auth_page(&h, true, None)
}
fn redirect_session(s: &AppState, token: &str, user: auth::User) -> Response {
    let mut r = auth::session_response(s, token, user);
    *r.status_mut() = StatusCode::SEE_OTHER;
    r.headers_mut()
        .insert(header::LOCATION, "/dashboard".parse().unwrap());
    r
}
pub async fn register(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(b): Form<auth::Credentials>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    match auth::register(&s, b).await {
        Ok((t, u)) => Ok(redirect_session(&s, &t, u)),
        Err(e) => Ok((e.status, auth_page(&h, true, Some(e.message))).into_response()),
    }
}
pub async fn login(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(b): Form<auth::Credentials>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    match auth::login(&s, b).await {
        Ok((t, u)) => Ok(redirect_session(&s, &t, u)),
        Err(e) => Ok((e.status, auth_page(&h, false, Some(e.message))).into_response()),
    }
}
pub async fn logout(State(s): State<AppState>, h: HeaderMap) -> AppResult<Response> {
    let mut r = api::logout(State(s), h).await?;
    *r.status_mut() = StatusCode::SEE_OTHER;
    r.headers_mut()
        .insert(header::LOCATION, "/signin".parse().unwrap());
    Ok(r)
}
#[derive(Deserialize)]
pub struct Appearance {
    theme: String,
}
pub async fn appearance(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(b): Form<Appearance>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    appearance_response(&s, &b.theme, "/dashboard")
}

pub(crate) fn appearance_response(
    s: &AppState,
    theme: &str,
    return_to: &str,
) -> AppResult<Response> {
    if !matches!(theme, "light" | "dark" | "system") {
        return Err(ApiError::bad("Invalid color scheme"));
    }
    let secure = if s.config.secure_cookie {
        "; Secure"
    } else {
        ""
    };
    Ok((
        [(
            header::SET_COOKIE,
            format!(
                "click_theme={theme}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000{secure}"
            ),
        )],
        Redirect::to(return_to),
    )
        .into_response())
}
#[derive(Deserialize, Default)]
pub struct DashboardQuery {
    site: Option<Uuid>,
    days: Option<u32>,
    view: Option<String>,
    dimension: Option<String>,
    event: Option<String>,
    path: Option<String>,
    campaign: Option<String>,
}
pub async fn overview(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<DashboardQuery>,
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
    let days = q.days.unwrap_or(7);
    let (start, end) = api::range(&api::Period { days: Some(days) })?;
    let view = q.view.as_deref().unwrap_or("overview");
    let title = match view {
        "overview" => "The bigger picture.",
        "audience" => "Meet your audience.",
        "acquisition" => "See what brings them here.",
        "content" => "Stories that find their people.",
        "events" => "Small actions. Real progress.",
        _ => return Err(ApiError::not_found()),
    };
    let data = s.store.overview(&site, start, end).await?;
    let mut options = String::new();
    for w in &sites {
        options.push_str(&format!(
            r#"<option value="{}" {}>{}</option>"#,
            w.id,
            if w.id == id { "selected" } else { "" },
            esc(&w.name)
        ));
    }
    let mut ranges = String::new();
    for n in [1, 7, 30, 90] {
        ranges.push_str(&format!(
            r#"<option value="{n}" {}>{}</option>"#,
            if n == days { "selected" } else { "" },
            if n == 1 {
                "Today".into()
            } else {
                format!("Last {n} days")
            }
        ));
    }
    let verified = if site.is_demo {
        "Demonstration website"
    } else if site.verified_at.is_some() {
        "Verified website"
    } else {
        "Awaiting domain verification"
    };
    let banner = if site.is_demo {
        r#"<div class="notice"><strong>Demo data</strong> · Explore sample traffic, audiences, campaigns and goals. Select another website to see real analytics. Demo events are excluded from billing usage.</div>"#
    } else if site.verified_at.is_none() {
        r#"<div class="notice">One last step: verify your domain to start collecting real traffic. <a href="/websites">Finish setup →</a></div>"#
    } else {
        ""
    };
    let mut metrics = format!(
        r#"<section class="metrics" aria-label="Key metrics">{}{}{}{}<div class="metric live-metric"><span class="metric-label"><span class="tiny-dot"></span> Active now</span><strong>{}</strong><span class="metric-note">Visitors in the last 5 minutes</span></div></section>"#,
        metric(
            "Unique visitors",
            data.visitors,
            "Distinct IDs in this period"
        ),
        metric("Page views", data.pageviews, "Every page, every visit"),
        metric("Sessions", data.sessions, "Visits with a page view"),
        format_args!(
            r#"<div class="metric"><span class="metric-label">Bounce rate</span><strong>{:.1}<span class="unit">%</span></strong><span class="metric-note">Sessions with one page view</span></div>"#,
            data.bounce_rate
        ),
        data.realtime
    );
    let chart = if view == "overview" {
        traffic_chart(&data.timeseries, days, start)
    } else {
        String::new()
    };
    let mut panels = String::new();
    let dimensions: &[(Dimension, &str, &str)] = match view {
        "audience" => &[
            (Dimension::Countries, "Around the world", "Country"),
            (Dimension::Devices, "Their devices", "Device"),
            (Dimension::Browsers, "Their browsers", "Browser"),
            (
                Dimension::OperatingSystems,
                "Their operating systems",
                "Operating system",
            ),
        ],
        "acquisition" => &[
            (Dimension::Referrers, "Where they find you", "Source"),
            (Dimension::Campaigns, "Your campaigns", "Campaign"),
        ],
        "content" => &[(Dimension::Pages, "Pages worth a visit", "Page")],
        "events" => &[(Dimension::Events, "Actions that matter", "Event")],
        _ => &[
            (Dimension::Pages, "Pages worth a visit", "Page"),
            (Dimension::Referrers, "Where they find you", "Source"),
            (Dimension::Countries, "Around the world", "Country"),
            (Dimension::Devices, "Their devices", "Device"),
        ],
    };
    let mut highlights = String::new();
    for (dim, title, label) in dimensions {
        let rows = s.store.breakdown(&site, *dim, start, end).await?;
        let leading = rows.first();
        let highlight_label = match dim {
            Dimension::Countries => "Leading country",
            Dimension::Devices => "Top device",
            Dimension::Browsers => "Top browser",
            Dimension::Referrers => "Leading source",
            Dimension::Campaigns => "Leading campaign",
            Dimension::Pages => "Most visited page",
            Dimension::Events => "Top action",
            Dimension::OperatingSystems => "Top operating system",
        };
        let value = leading
            .map(|r| {
                if matches!(dim, Dimension::Countries) && r.label == "ZZ" {
                    "Unknown country".into()
                } else if r.label.is_empty() {
                    if matches!(dim, Dimension::Referrers) {
                        "Direct / internal".into()
                    } else {
                        "No campaign".into()
                    }
                } else {
                    r.label.clone()
                }
            })
            .unwrap_or_else(|| "No activity yet".into());
        let note = leading
            .map(|r| {
                format!(
                    "{} {}",
                    format_number(r.count),
                    if matches!(dim, Dimension::Events) {
                        "events"
                    } else {
                        "page views"
                    }
                )
            })
            .unwrap_or_else(|| "Activity will appear here".into());
        highlights.push_str(&format!(r#"<div class="metric detail-metric"><span class="metric-label">{highlight_label}</span><strong>{}</strong><span class="metric-note">{}</span></div>"#,esc(&value),esc(&note)));
        panels.push_str(&breakdown_panel(
            title,
            label,
            &rows,
            matches!(dim, Dimension::Countries),
        ));
    }
    if view != "overview" {
        metrics = format!(
            r#"<section class="metrics report-metrics" aria-label="Report highlights">{}{highlights}</section>"#,
            metric(
                if view == "content" {
                    "Page views"
                } else {
                    "Unique visitors"
                },
                if view == "content" {
                    data.pageviews
                } else {
                    data.visitors
                },
                "In the selected period"
            )
        );
    }
    match view {
        "overview" => {
            panels.push_str(&crate::explore_ui::comparison(&s.store, &site, start, end).await?)
        }
        "content" => {
            panels.push_str(&crate::explore_ui::content(&s.store, &site, start, end).await?)
        }
        "acquisition" => panels.push_str(
            &crate::explore_ui::acquisition(
                &s.store,
                &site,
                start,
                end,
                q.dimension.as_deref().unwrap_or("channel"),
                days,
            )
            .await?,
        ),
        "audience" => {
            panels.push_str(&crate::explore_ui::audience(&s.store, &site, start, end, days).await?)
        }
        _ => {}
    }
    let event_context = if view == "events" {
        crate::explore_ui::events(
            &s.store,
            &site,
            start,
            end,
            &crate::explore::EventQuery {
                days: Some(days),
                event: q.event,
                path: q.path,
                campaign: q.campaign,
            },
            days,
        )
        .await?
    } else {
        String::new()
    };
    let mut goals = String::new();
    if view == "events" {
        let rows: Vec<api::Goal> = sqlx::query_as(
            "SELECT id,name,event_name FROM goals WHERE website_id=$1 ORDER BY created_at",
        )
        .bind(id)
        .fetch_all(&s.store.pg)
        .await?;
        let editable = role.can_edit_goals();
        for g in rows {
            let count = s.store.goal_count(&site, &g.event_name, start, end).await?;
            let converting = s
                .store
                .goal_sessions(&site, &g.event_name, start, end)
                .await?;
            let conversion = if data.sessions == 0 {
                "Unavailable".into()
            } else {
                format!("{:.1}%", converting as f64 / data.sessions as f64 * 100.0)
            };
            // The row stays a readable row: name and event as text, as a
            // table of values should be. Editing is a secondary action,
            // folded into a disclosure on its own full-width row — the same
            // shape the funnel panel uses. Putting the fields in the cells
            // themselves was tried first and is worse: it takes the values
            // out of the table's text, so the table stops being readable as
            // one, which the browser suite caught immediately.
            let actions = if editable {
                format!(
                    r#"<td class="row-actions"><form action="/goals/action" method="post" onsubmit="return confirm('Remove this goal? Its completions stay in your analytics.')"><input type="hidden" name="action" value="delete"><input type="hidden" name="site" value="{id}"><input type="hidden" name="goal" value="{goal}"><button class="btn outlined small">Remove</button></form></td>"#,
                    goal = g.id,
                )
            } else {
                String::new()
            };
            goals.push_str(&format!(
                "<tr><td>{}</td><td><code>{}</code></td><td class=number>{count}</td><td>{converting}</td><td>{conversion}</td>{actions}</tr>",
                esc(&g.name),
                esc(&g.event_name)
            ));
            if editable {
                goals.push_str(&format!(
                    r#"<tr class="edit-row"><td colspan=6><details class="row-edit"><summary>Edit {name}</summary><form action="/goals/action" method="post" class="goal-form"><input type="hidden" name="action" value="update"><input type="hidden" name="site" value="{id}"><input type="hidden" name="goal" value="{goal}"><label>Goal name<input name="name" value="{name}" required maxlength="100"></label><label>Event name<input name="event_name" value="{event}" required maxlength="64"></label><button class="btn primary">Save goal</button></form></details></td></tr>"#,
                    goal = g.id,
                    name = esc(&g.name),
                    event = esc(&g.event_name),
                ));
            }
        }
        let columns = if editable { 6 } else { 5 };
        if goals.is_empty() {
            goals = format!(
                "<tr><td colspan={columns}>No goals yet. Give an important action a name.</td></tr>"
            );
        }
        let actions_header = if editable { "<th>Actions</th>" } else { "" };
        let form = if editable {
            format!(
                r#"<form action="/goals" method="post" class="goal-form"><input type="hidden" name="site" value="{id}"><label>Goal name<input name="name" required maxlength="100" placeholder="Newsletter signup"></label><label>Event name<input name="event_name" required maxlength="64" placeholder="newsletter_signup"></label><button class="btn primary">Create goal</button></form>"#
            )
        } else {
            String::new()
        };
        goals = format!(
            r#"<section class="panel goals-panel"><div class="panel-heading"><h2>Your goals</h2><span class="pill">Custom events</span></div><div class="owner-table"><table><thead><tr><th>Goal</th><th>Event</th><th class="number">Completions</th><th>Converting sessions</th><th>Session conversion rate</th>{actions_header}</tr></thead><tbody>{goals}</tbody></table></div>{form}<p class="muted">Completions count accepted event IDs. Session rates count distinct associated sessions with a pageview in this period; unmatched backend facts do not imply browser association. Trigger an event with <code>vuuutClick.track('newsletter_signup')</code>.</p></section>"#
        );
    }
    let geo = if site.is_demo {
        "Sample countries for demonstration"
    } else if s.geoip_service().available() {
        "Local country lookup enabled"
    } else {
        "GeoIP not configured · countries appear as Unknown"
    };
    let report_class = if view == "events" {
        " events-report"
    } else {
        ""
    };
    let body = format!(
        r#"<header class="topbar"><div class="breadcrumb">Your workspace <span>/</span> Analytics</div><span class="status"><span class="tiny-dot"></span> {verified}</span></header><div class="page{report_class}"><div class="page-heading"><div><span class="eyebrow">A LITTLE INSIGHT. A LOT OF POSSIBILITY.</span><h1>{title}</h1><p>A thoughtful look at the people discovering {}.</p></div><a class="btn outlined" href="/dashboard?site={id}&days={days}&view={view}">↻ Refresh</a></div>{banner}<div class="toolbar"><form method="get" action="/dashboard"><input type="hidden" name="view" value="{view}"><label class="sr-only" for="website">Website</label><select id="website" name="site">{options}</select><label class="sr-only" for="period">Time period</label><select id="period" name="days">{ranges}</select><button class="btn small">Apply</button></form><span class="muted">UTC · {} – {}</span></div>{metrics}{chart}<div class="panel-grid">{panels}</div>{goals}{event_context}<footer class="page-footer"><span><span class="tiny-dot"></span> Real data. A little more understanding.</span><span>{geo}</span></footer></div>"#,
        esc(&site.name),
        (Utc::now().date_naive() - Duration::days(days as i64 - 1)).format("%d %b"),
        Utc::now().format("%d %b %Y")
    );
    Ok(shell(&h, &user.email, view, Some(id), body).into_response())
}
fn metric(label: &str, value: u64, note: &str) -> String {
    format!(
        r#"<div class="metric"><span class="metric-label">{label}</span><strong>{}</strong><span class="metric-note">{note}</span></div>"#,
        format_number(value)
    )
}
// Adapted from Tinybird's MIT kFormatter. The full notice is in docs/TINYBIRD-LICENSE.
fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n > 999 {
        format!("{:.1}K", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}
fn traffic_chart(rows: &[Breakdown], days: u32, start: u32) -> String {
    let first = chrono::DateTime::from_timestamp(start as i64, 0)
        .unwrap()
        .date_naive();
    let points: Vec<(String, u64)> = (0..days)
        .map(|i| {
            let day = (first + Duration::days(i as i64)).to_string();
            let count = rows
                .iter()
                .find(|r| r.label == day)
                .map(|r| r.count)
                .unwrap_or(0);
            (day, count)
        })
        .collect();
    let max = points.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1);
    let mut path = String::new();
    let mut dots = String::new();
    let mut table = String::new();
    for (i, (label, count)) in points.iter().enumerate() {
        let x = 60.0 + i as f64 / (days - 1).max(1) as f64 * 920.0;
        let y = 204.0 - *count as f64 / max as f64 * 160.0;
        path.push_str(&format!(
            "{} {x:.1} {y:.1} ",
            if i == 0 { "M" } else { "L" }
        ));
        dots.push_str(&format!(r#"<circle cx="{x:.1}" cy="{y:.1}" r="3.5"><title>{label}: {count} page views</title></circle>"#));
        table.push_str(&format!("<tr><td>{label}</td><td>{count}</td></tr>"));
    }
    let empty = if max == 1 && points.iter().all(|(_, v)| *v == 0) {
        r#"<div class="chart-empty"><strong>Your next visitor starts the story.</strong><span>Install your tracker and watch your audience take shape.</span><a href="/websites">Get your tracking code →</a></div>"#
    } else {
        ""
    };
    let last_x = if days == 1 { 60 } else { 980 };
    format!(
        r#"<section class="panel traffic"><div class="panel-heading"><div><h2>The rhythm of your traffic</h2><p>Page views over time</p></div><span class="legend"><span></span> Page views</span></div><div class="chart-wrap"><svg class="traffic-chart" viewBox="0 0 1020 250" role="img" aria-label="Daily page views chart; accessible values in table below"><defs><linearGradient id="traffic-fill" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="currentColor" stop-opacity=".18"/><stop offset="100%" stop-color="currentColor" stop-opacity="0"/></linearGradient></defs><g class="chart-grid"><path d="M60 44H980M60 97H980M60 151H980M60 204H980"/></g><g class="chart-labels"><text x="12" y="49">{max}</text><text x="12" y="209">0</text><text x="60" y="238">{}</text><text x="980" y="238" text-anchor="end">{}</text></g><path class="chart-fill" d="{path}L{last_x} 204 L60 204 Z"/><path class="chart-line" d="{path}"/><g class="chart-dots">{dots}</g></svg>{empty}</div><details class="chart-values"><summary>View daily values</summary><table><thead><tr><th>Date (UTC)</th><th>Page views</th></tr></thead><tbody>{table}</tbody></table></details></section>"#,
        first.format("%d %b"),
        (first + Duration::days(days as i64 - 1)).format("%d %b")
    )
}
fn breakdown_panel(title: &str, label: &str, rows: &[Breakdown], countries: bool) -> String {
    let total: u64 = rows.iter().map(|r| r.count).sum();
    let max = rows.first().map(|r| r.count).unwrap_or(1).max(1);
    let mut body = String::new();
    for r in rows {
        let name = if countries {
            if r.label == "ZZ" {
                "Unknown country".into()
            } else {
                r.label.clone()
            }
        } else if r.label.is_empty() {
            if label == "Source" {
                "Direct / internal".into()
            } else {
                "No campaign".into()
            }
        } else {
            r.label.clone()
        };
        let width = r.count as f64 / max as f64 * 100.0;
        let share = if total == 0 {
            0.0
        } else {
            r.count as f64 / total as f64 * 100.0
        };
        body.push_str(&format!(r#"<tr><td><span class="dimension-label">{}</span><svg class="row-bar" viewBox="0 0 100 3" preserveAspectRatio="none" aria-hidden="true"><rect width="{width:.1}" height="3" rx="1.5"/></svg></td><td class="number">{}</td><td class="number muted">{share:.0}%</td></tr>"#,esc(&name),format_number(r.count)));
    }
    if rows.is_empty() {
        body=r#"<tr><td colspan="3"><div class="empty-table"><span>Nothing here just yet.</span><small>Your visitors will bring this space to life.</small></div></td></tr>"#.into();
    }
    format!(
        r#"<section class="panel breakdown"><div class="panel-heading"><h2>{title}</h2><span class="badge">Top 20</span></div><table><thead><tr><th>{label}</th><th class="number">Views</th><th class="number">Top 20 share</th></tr></thead><tbody>{body}</tbody></table></section>"#
    )
}
pub async fn websites(State(s): State<AppState>, h: HeaderMap) -> AppResult<Response> {
    let user = match auth::user(&s, &h).await {
        Ok(u) => u,
        Err(e) if e.status == StatusCode::UNAUTHORIZED => {
            return Ok(Redirect::to("/signin").into_response())
        }
        Err(e) => return Err(e),
    };
    let sites = api::list_websites(&s, &h).await?;
    let orgs = api::list_organizations(&s, &h).await?;
    let mut cards = String::new();
    for site in &sites {
        cards.push_str(&website_card(
            site,
            &s.config.public_url,
            s.config.allow_localhost,
        ));
    }
    let mut opts = String::new();
    for org in orgs {
        if matches!(org.role.as_str(), "owner" | "admin") {
            opts.push_str(&format!(
                "<option value=\"{}\">{}</option>",
                org.id,
                esc(&org.name)
            ));
        }
    }
    let empty = if sites.is_empty() {
        r#"<div class="welcome"><span class="welcome-icon">↗</span><h2>A home for your next insight.</h2><p>Add your first website. A small script connects your site<br>to a clearer understanding of your audience.</p></div>"#
    } else {
        ""
    };
    let form = if opts.is_empty() {
        String::new()
    } else {
        format!(
            r#"<section class="panel setup-panel"><div><span class="eyebrow">LET'S GET CONNECTED</span><h2>Add a website</h2><p>Works with any website. Built to fit your own infrastructure.</p></div><form method="post" action="/websites" class="stack"><label>Workspace<select name="organization_id">{opts}</select></label><label>Website name<input name="name" maxlength="100" required placeholder="My publication"></label><label>Website origin<input name="origin" type="url" required maxlength="255" placeholder="https://your-website.com"></label><button class="btn primary">Add website {}</button></form></section>"#,
            icon("arrow")
        )
    };
    let body = format!(
        r#"<header class="topbar"><div class="breadcrumb">Your workspace <span>/</span> Websites</div><span class="status">A simple connection. A clearer picture.</span></header><div class="page"><div class="page-heading"><div><span class="eyebrow">GREAT THINGS START WITH A CONNECTION</span><h1>Your websites.</h1><p>One thoughtful view of everything you publish.</p></div></div>{empty}{cards}{form}<footer class="page-footer"><span>No analytics data is fabricated. Your first visit starts your dashboard.</span></footer></div>"#
    );
    Ok(shell(&h, &user.email, "sites", sites.first().map(|s| s.id), body).into_response())
}
fn website_card(s: &Website, public: &str, local: bool) -> String {
    if s.is_demo {
        return format!(
            r#"<section class="panel website-card"><div class="panel-heading"><div><h2>{}</h2><p>Sample traffic, countries, campaigns and conversions for exploring the dashboard.</p></div><span class="pill">Demo data</span><a class="btn outlined small" href="/dashboard?site={}">Explore demo →</a></div><p class="muted">This example website does not collect visits or consume your event allowance.</p></section>"#,
            esc(&s.name),
            s.id
        );
    }

    let status = if s.verified_at.is_some() {
        r#"<span class="pill success">Verified</span>"#
    } else {
        r#"<span class="pill">Needs verification</span>"#
    };
    let host = url::Url::parse(&s.origin)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    let verify = if s.verified_at.is_none() {
        format!(
            r#"<div class="setup-step"><span class="step-number">1</span><div><h3>Verify your domain</h3><p>Add this DNS TXT record, then check verification.</p><code>_vuuut-click.{} = vuuut-click={}</code><form action="/websites/{}/verify" method="post"><button class="btn outlined small">Check verification</button></form></div></div>"#,
            esc(&host),
            esc(&s.verification_token),
            s.id
        )
    } else {
        String::new()
    };
    let test_link = if local && s.origin == public {
        format!(
            r#"<p><a class="btn outlined small" href="/playground/{}">Open real tracker test page →</a></p>"#,
            esc(&s.collection_id)
        )
    } else {
        String::new()
    };
    let snippet = format!(
        r#"<script defer src="{public}/tracker.js" data-website="{}" data-consent="required"></script>"#,
        s.collection_id
    );
    format!(
        r#"<section class="panel website-card"><div class="panel-heading"><div><h2>{}</h2><p>{}</p></div>{status}<a class="btn outlined small" href="/dashboard?site={}">Open analytics →</a></div>{verify}<div class="setup-step"><span class="step-number">{}</span><div><h3>Give your website a little insight</h3><p>Add this script to your site's HTML. After your visitor consents, call <code>vuuutClick.consent(true)</code>.</p><pre><code>{}</code></pre>{test_link}<p class="muted">Session-only identity by default. Add <code>data-identity="persistent"</code> only after consent to support returning-browser reports. Add <code>data-engagement="true"</code> to opt into visible-time and scroll measurement; these additional events count toward usage. DNT and Global Privacy Control are respected. Choose anonymous collection with <code>data-consent="anonymous"</code> only after reviewing your consent requirements.</p></div></div></section>"#,
        esc(&s.name),
        esc(&s.origin),
        s.id,
        if s.verified_at.is_some() { 1 } else { 2 },
        esc(&snippet)
    )
}
pub async fn create_website(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(b): Form<api::NewWebsite>,
) -> AppResult<Redirect> {
    api::add_website(&s, &h, b).await?;
    Ok(Redirect::to("/websites"))
}
pub async fn verify(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Redirect> {
    api::verify_website(&s, &h, id).await?;
    Ok(Redirect::to("/websites"))
}
#[derive(Deserialize)]
pub struct GoalForm {
    site: Uuid,
    name: String,
    event_name: String,
}
pub async fn create_goal(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(b): Form<GoalForm>,
) -> AppResult<Redirect> {
    api::add_goal(
        &s,
        &h,
        b.site,
        api::NewGoal {
            name: b.name,
            event_name: b.event_name,
        },
    )
    .await?;
    Ok(Redirect::to(&format!(
        "/dashboard?site={}&view=events",
        b.site
    )))
}
/// The goal controls on the Events & goals panel.
///
/// One POST endpoint naming the action, because an HTML form can only GET or
/// POST — the JSON API uses PATCH and DELETE on
/// `/v1/websites/{id}/goals/{goal}`, which a browser form cannot send. The
/// same shape as `/insights/action` and `/settings/action`.
#[derive(Deserialize)]
pub struct GoalAction {
    action: String,
    site: Uuid,
    goal: Uuid,
    name: Option<String>,
    event_name: Option<String>,
}
pub async fn goal_action(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(b): Form<GoalAction>,
) -> AppResult<Redirect> {
    match b.action.as_str() {
        "update" => {
            manage::edit_goal(
                &s,
                &h,
                b.site,
                b.goal,
                api::NewGoal {
                    name: b.name.ok_or_else(|| ApiError::bad("Name your goal"))?,
                    event_name: b
                        .event_name
                        .ok_or_else(|| ApiError::bad("Name the event this goal counts"))?,
                },
            )
            .await?;
        }
        "delete" => {
            manage::remove_goal(&s, &h, b.site, b.goal).await?;
        }
        _ => return Err(ApiError::bad("Unknown goal action")),
    }
    Ok(Redirect::to(&format!(
        "/dashboard?site={}&view=events",
        b.site
    )))
}
pub async fn stylesheet() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        include_str!("../../../dashboard/app.css"),
    )
}

pub async fn playground(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(token): Path<String>,
) -> AppResult<Html<String>> {
    playground_view(&s, &h, &token, "Welcome").await
}
pub async fn playground_page(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((token, page)): Path<(String, String)>,
) -> AppResult<Html<String>> {
    if page != "about" {
        return Err(ApiError::not_found());
    }
    playground_view(&s, &h, &token, "About this little corner").await
}
async fn playground_view(
    s: &AppState,
    h: &HeaderMap,
    token: &str,
    title: &str,
) -> AppResult<Html<String>> {
    if !s.config.allow_localhost {
        return Err(ApiError::not_found());
    }
    let site = s
        .store
        .collect_site(token)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if site.origin != s.config.public_url || site.verified_at.is_none() {
        return Err(ApiError::not_found());
    }
    Ok(document(
        "Tracker test page",
        h,
        &format!(
            r#"<main id="main" class="page"><span class="eyebrow">VUUUT CLICK · LOCAL TRACKER TEST</span><h1>{}</h1><p>This development page sends a real page view to {} using the same browser tracker you install on your website.</p><p>Wait a few seconds, then open your analytics to see the visit. The country is Unknown for local addresses.</p><p><a class="btn primary" href="/playground/{}/about">Visit another page →</a> <a class="btn outlined" href="/dashboard?site={}">View your analytics</a></p><script defer src="/tracker.js" data-website="{}" data-consent="anonymous"></script></main>"#,
            esc(title),
            esc(&site.name),
            esc(token),
            site.id,
            esc(token)
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escapes_all_html_contexts() {
        assert_eq!(
            esc("<script x='\"'>&"),
            "&lt;script x=&#39;&quot;&#39;&gt;&amp;"
        );
    }
    #[test]
    fn real_chart_has_accessible_values_and_no_fake_series() {
        let s = traffic_chart(
            &[Breakdown {
                label: "2026-11-08".into(),
                count: 3,
            }],
            1,
            1794096000,
        );
        assert!(s.contains("View daily values"));
        assert!(s.contains("Daily page views"));
    }
}
