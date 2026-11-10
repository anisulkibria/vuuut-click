//! Platform-owner authentication is independent of customer authentication at every layer.
//! No public signup grants ownership; only an operator may grant it through the CLI.
use crate::{
    auth,
    dashboard::{appearance_response, document, esc, navigation_link, with_sidebar, Sidebar},
    ApiError, AppResult, AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Form, Json,
};
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

const COOKIE: &str = "click_owner_session";

pub async fn authenticated_owner(s: &AppState, h: &HeaderMap) -> AppResult<auth::User> {
    let token = auth::cookie_value(h, COOKIE).ok_or_else(ApiError::unauthorized)?;
    sqlx::query_as("SELECT u.id,u.email FROM users u JOIN sessions s ON s.user_id=u.id JOIN platform_owners p ON p.user_id=u.id WHERE s.token_hash=$1 AND s.audience='owner' AND s.expires_at>now()")
        .bind(click_core::digest(&token)).fetch_optional(&s.store.pg).await?.ok_or_else(ApiError::unauthorized)
}

pub async fn home(State(s): State<AppState>, h: HeaderMap) -> AppResult<Redirect> {
    match authenticated_owner(&s, &h).await {
        Ok(_) => Ok(Redirect::to("/owner/dashboard")),
        Err(e) if e.status == StatusCode::UNAUTHORIZED => Ok(Redirect::to("/owner/signin")),
        Err(e) => Err(e),
    }
}

fn sign_in_page(h: &HeaderMap, error: Option<&str>) -> Html<String> {
    let error = error
        .map(|message| format!(r#"<p class="alert" role="alert">{}</p>"#, esc(message)))
        .unwrap_or_default();
    document(
        "Owner sign in",
        h,
        &format!(
            r#"<div class="auth-layout"><section class="auth-story"><a class="brand" href="/owner"><span class="brand-mark">v<span>↗</span></span><span>vuuut<span class="brand-sub">CLICK</span></span></a><div><span class="eyebrow">YOUR PLATFORM. YOUR PERSPECTIVE.</span><h1>A place for<br>the bigger picture.</h1><p>Oversee your analytics platform<br>from your own space.</p></div><p class="auth-foot">Vuuut Click · Owner console</p></section><main id="main" class="auth-main"><div class="auth-card"><span class="pill">Platform owner</span><h2>Welcome to your console.</h2><p>Sign in with your platform owner account.</p>{error}<form action="/owner/auth/login" method="post" class="stack"><label>Email address<input type="email" name="email" autocomplete="username" maxlength="254" required></label><label>Password<input type="password" name="password" autocomplete="current-password" maxlength="256" required></label><button class="btn primary">Sign in as owner</button></form><p><a href="/owner/forgot-password">Forgot your owner password?</a></p><p class="auth-other">Looking for your website analytics? <a href="/signin">User sign in</a></p></div></main></div>"#
        ),
    )
}
pub async fn signin(h: HeaderMap) -> Html<String> {
    sign_in_page(&h, None)
}

pub async fn login(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(body): Form<auth::Credentials>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    let mut tx = s.store.pg.begin().await?;
    let user = match auth::verify_locked_credentials(&s, &mut tx, body).await {
        Ok(user) => user,
        Err(e) => return Ok((e.status, sign_in_page(&h, Some(e.message))).into_response()),
    };
    let granted: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_owners WHERE user_id=$1)")
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?;
    if !granted {
        return Ok((
            StatusCode::FORBIDDEN,
            sign_in_page(&h, Some("This account does not have platform owner access")),
        )
            .into_response());
    }
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM owner_security WHERE user_id=$1 AND confirmed_at IS NOT NULL)",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    let token = auth::secret();
    let (audience, ttl) = if pending {
        ("owner_pending", 300)
    } else {
        ("owner", 28800)
    };
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at,audience) VALUES($1,$2,now()+make_interval(secs=>$3::int),$4)")
        .bind(click_core::digest(&token)).bind(user.id).bind(ttl).bind(audience).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((
        [(
            header::SET_COOKIE,
            crate::owner_security::cookie(&s, &token, pending),
        )],
        Redirect::to(if pending {
            "/owner/auth/verify"
        } else {
            "/owner/dashboard"
        }),
    )
        .into_response())
}

pub async fn logout(State(s): State<AppState>, h: HeaderMap) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    for (name, audience) in [(COOKIE, "owner"), ("click_owner_pending", "owner_pending")] {
        if let Some(token) = auth::cookie_value(&h, name) {
            sqlx::query("DELETE FROM sessions WHERE token_hash=$1 AND audience=$2")
                .bind(click_core::digest(&token))
                .bind(audience)
                .execute(&s.store.pg)
                .await?;
        }
    }
    let secure = if s.config.secure_cookie {
        "; Secure"
    } else {
        ""
    };
    let mut headers = HeaderMap::new();
    for name in [COOKIE, "click_owner_pending"] {
        headers.append(
            header::SET_COOKIE,
            format!("{name}=; Path=/owner; HttpOnly; SameSite=Strict; Max-Age=0{secure}")
                .parse()
                .unwrap(),
        );
    }
    Ok((headers, Redirect::to("/owner/signin")).into_response())
}

pub async fn me(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<auth::User>> {
    Ok(Json(authenticated_owner(&s, &h).await?))
}

#[derive(Deserialize)]
pub struct OwnerAction {
    action: String,
    organization_id: Uuid,
    value: Option<String>,
}
pub async fn action(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<OwnerAction>,
) -> AppResult<Redirect> {
    auth::csrf(&s, &h)?;
    let mut tx = s.store.pg.begin().await?;
    let (owner, _) = crate::owner_security::lock_owner(&h, &mut tx).await?;
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM organizations WHERE id=$1 FOR UPDATE")
            .bind(input.organization_id)
            .fetch_optional(&mut *tx)
            .await?;
    if status.is_none() || status.as_deref() == Some("deleting") {
        return Err(ApiError::not_found());
    }
    let (section, audit) = match input.action.as_str() {
        "status" => {
            let value = input
                .value
                .as_deref()
                .filter(|v| matches!(*v, "active" | "suspended"))
                .ok_or_else(|| ApiError::bad("Choose an active or suspended workspace status"))?;
            sqlx::query("UPDATE organizations SET status=$1 WHERE id=$2")
                .bind(value)
                .bind(input.organization_id)
                .execute(&mut *tx)
                .await?;
            ("workspaces", format!("workspace.status.{value}"))
        }
        "plan" => {
            let plan = input
                .value
                .as_deref()
                .ok_or_else(|| ApiError::bad("Select a plan"))?;
            let provider: String =
                sqlx::query_scalar("SELECT provider FROM subscriptions WHERE organization_id=$1")
                    .bind(input.organization_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if !matches!(provider.as_str(), "none" | "test" | "manual") {
                return Err(ApiError::conflict(
                    "Provider subscriptions must be managed through their payment provider",
                ));
            }
            let limits: Option<(i32, i32)> =
                sqlx::query_as("SELECT website_limit,seat_limit FROM plans WHERE id=$1")
                    .bind(plan)
                    .fetch_optional(&mut *tx)
                    .await?;
            let (sites, seats) = limits.ok_or_else(ApiError::not_found)?;
            let used:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM websites WHERE organization_id=$1 AND NOT is_demo),(SELECT count(*) FROM memberships WHERE organization_id=$1)+(SELECT count(*) FROM invitations WHERE organization_id=$1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at>now())").bind(input.organization_id).fetch_one(&mut *tx).await?;
            if used.0 > sites as i64 || used.1 > seats as i64 {
                return Err(ApiError::conflict(
                    "Reduce website or reserved team seats before downgrading",
                ));
            }
            sqlx::query("UPDATE subscriptions SET plan_id=$1,provider='manual',status='active' WHERE organization_id=$2").bind(plan).bind(input.organization_id).execute(&mut *tx).await?;
            ("subscriptions", format!("subscription.manual_plan.{plan}"))
        }
        "requeue" => {
            if status.as_deref() != Some("active") {
                return Err(ApiError::conflict(
                    "Reactivate the workspace before retrying delivery",
                ));
            }
            // Old payloads remain discarded; retrying must never resurrect retained-out data.
            sqlx::query("UPDATE event_outbox e SET state=CASE WHEN (e.payload->>'timestamp')::bigint < extract(epoch FROM now()-make_interval(days=>least(w.retention_days,p.retention_days))) THEN 'discarded' ELSE 'pending' END,attempts=0,available_at=now() FROM websites w,subscriptions s,plans p WHERE e.website_id=w.id AND s.organization_id=w.organization_id AND p.id=s.plan_id AND e.organization_id=$1 AND e.state='dead' AND w.deletion_requested_at IS NULL")
                .bind(input.organization_id).execute(&mut *tx).await?;
            ("delivery", "delivery.dead_letters_requeued".into())
        }
        _ => return Err(ApiError::bad("Unknown platform action")),
    };
    sqlx::query("INSERT INTO platform_audit(actor_id,action,resource_id) VALUES($1,$2,$3)")
        .bind(owner.id)
        .bind(audit)
        .bind(input.organization_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/owner/{section}")))
}

const SECTIONS: &[(&str, &str, &str)] = &[
    ("dashboard", "Overview", "Your platform at a glance."),
    ("workspaces", "Workspaces", "A home for every customer."),
    ("users", "Users", "The people behind your platform."),
    ("websites", "Websites", "Every connected corner of the web."),
    (
        "subscriptions",
        "Subscriptions",
        "Plans and usage, in one place.",
    ),
    ("delivery", "Event delivery", "Keep the insights flowing."),
    (
        "commercial",
        "Commercial status",
        "Keep the promises behind your plans.",
    ),
    (
        "connections",
        "Connections",
        "Connect your external services.",
    ),
    ("audit", "Audit log", "A record of what changed."),
];

pub(crate) fn owner_shell(h: &HeaderMap, email: &str, section: &str, body: String) -> Html<String> {
    let mut navigation = String::new();
    for (key, label, _) in SECTIONS {
        let glyph = if *key == "dashboard" {
            "overview"
        } else if *key == "websites" {
            "sites"
        } else {
            key
        };
        navigation.push_str(&navigation_link(
            &format!("/owner/{key}"),
            label,
            glyph,
            *key == section,
        ));
    }
    let utilities = navigation_link(
        "/owner/account",
        "Account & security",
        "settings",
        section == "account",
    ) + &navigation_link("/v1/openapi.json", "API reference", "info", false)
        + r#"<a href="/dashboard" class="nav-item" aria-label="User dashboard" target="_blank" rel="noopener"><span class="ic tint-teal" aria-hidden="true">↗</span><span>User dashboard</span></a>"#;
    with_sidebar(
        h,
        email,
        Sidebar {
            title: "Owner console",
            heading: "YOUR PLATFORM",
            home: "/owner/dashboard",
            navigation,
            utilities,
            appearance: "/owner/appearance",
            logout: "/owner/auth/logout",
            logout_label: "Sign out of owner console",
        },
        body,
    )
}

#[derive(Deserialize)]
pub struct PageQuery {
    #[serde(default = "first_page")]
    page: u32,
}
fn first_page() -> u32 {
    1
}

pub async fn dashboard(State(s): State<AppState>, h: HeaderMap) -> AppResult<Response> {
    render_page(&s, &h, "dashboard", 1).await
}

pub async fn directory(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(section): Path<String>,
    Query(query): Query<PageQuery>,
) -> AppResult<Response> {
    if !SECTIONS.iter().any(|(key, _, _)| *key == section) {
        return Err(ApiError::not_found());
    }
    if query.page == 0 || query.page > 10000 {
        return Err(ApiError::bad("Page must be between 1 and 10000"));
    }
    render_page(&s, &h, &section, query.page).await
}

#[derive(Deserialize)]
pub struct Appearance {
    theme: String,
}
pub async fn appearance(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<Appearance>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    authenticated_owner(&s, &h).await?;
    appearance_response(&s, &input.theme, "/owner/dashboard")
}

async fn render_page(s: &AppState, h: &HeaderMap, section: &str, page: u32) -> AppResult<Response> {
    let owner = match authenticated_owner(s, h).await {
        Ok(user) => user,
        Err(e) if e.status == StatusCode::UNAUTHORIZED => {
            return Ok(Redirect::to("/owner/signin").into_response())
        }
        Err(e) => return Err(e),
    };
    let (_, label, title) = SECTIONS
        .iter()
        .find(|(key, _, _)| *key == section)
        .ok_or_else(ApiError::not_found)?;
    let mut content = String::new();
    if section == "dashboard" {
        let (users,organizations,websites,pending,dead):(i64,i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM users),(SELECT count(*) FROM organizations),(SELECT count(*) FROM websites),(SELECT count(*) FROM event_outbox WHERE state='pending'),(SELECT count(*) FROM event_outbox WHERE state='dead')").fetch_one(&s.store.pg).await?;
        content.push_str(r#"<section class="metrics" aria-label="Platform totals">"#);
        for (label, count) in [
            ("Accounts", users),
            ("Workspaces", organizations),
            ("Websites", websites),
            ("Queued events", pending),
            ("Dead letters", dead),
        ] {
            content.push_str(&format!(r#"<div class="metric"><span class="metric-label">{label}</span><strong>{count}</strong></div>"#));
        }
        content.push_str("</section>");
    }
    if matches!(section, "workspaces" | "subscriptions" | "delivery") {
        let organizations: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id,name FROM organizations WHERE status<>'deleting' ORDER BY name,id LIMIT 500",
        )
        .fetch_all(&s.store.pg)
        .await?;
        let options = organizations
            .into_iter()
            .map(|(id, name)| format!(r#"<option value="{id}">{}</option>"#, esc(&name)))
            .collect::<String>();
        let (action,controls,explanation)=match section {
            "workspaces" => ("status",r#"<label>Workspace status<select name="value"><option value="active">Active</option><option value="suspended">Suspended</option></select></label>"#.to_owned(),"Suspension stops new collection immediately. Existing reports remain accessible to the customer."),
            "subscriptions" => {
                let plans:Vec<String>=sqlx::query_scalar("SELECT id FROM plans ORDER BY monthly_events").fetch_all(&s.store.pg).await?;
                let options=plans.iter().map(|p|format!(r#"<option value="{}">{}</option>"#,esc(p),esc(p))).collect::<String>();
                ("plan",format!(r#"<label>Manual plan<select name="value">{options}</select></label>"#),"Grant a manual allowance for a workspace without a payment provider. This records no payment. Provider subscriptions are managed through their billing portal.")
            },
            _ => ("requeue",String::new(),"Retry dead letters for an active workspace. Event identifiers preserve delivery idempotency and expired records stay discarded."),
        };
        content.push_str(&format!(r#"<section class="panel settings-panel"><h2>Manage {label}</h2><p>{explanation}</p><form method="post" action="/owner/actions" class="settings-form"><input type="hidden" name="action" value="{action}"><label>Workspace<select name="organization_id" required>{options}</select></label>{controls}<button class="btn primary small">Apply change</button></form></section>"#));
    }
    let limit = if section == "dashboard" {
        6_i64
    } else {
        101_i64
    };
    let offset = if section == "dashboard" {
        0
    } else {
        (page as i64 - 1) * 100
    };
    let query=match section {
        "dashboard"|"workspaces"=>"SELECT o.name,s.plan_id AS plan,o.status,(SELECT count(*) FROM websites w WHERE w.organization_id=o.id) AS websites,(SELECT count(*) FROM memberships m WHERE m.organization_id=o.id) AS members FROM organizations o JOIN subscriptions s ON s.organization_id=o.id ORDER BY o.created_at DESC,o.id LIMIT $1 OFFSET $2",
        "users"=>"SELECT u.email,EXISTS(SELECT 1 FROM platform_owners p WHERE p.user_id=u.id) AS owner,(SELECT count(*) FROM memberships m WHERE m.user_id=u.id) AS workspaces,u.created_at FROM users u ORDER BY u.created_at DESC,u.id LIMIT $1 OFFSET $2",
        "websites"=>"SELECT w.name,w.origin,o.name AS workspace,w.verified_at IS NOT NULL AS verified,w.active,w.created_at FROM websites w JOIN organizations o ON o.id=w.organization_id ORDER BY w.created_at DESC,w.id LIMIT $1 OFFSET $2",
        "subscriptions"=>"SELECT o.name,p.id AS plan,s.status,s.provider,coalesce(u.accepted_events,0) AS used,p.monthly_events AS allowance FROM subscriptions s JOIN organizations o ON o.id=s.organization_id JOIN plans p ON p.id=s.plan_id LEFT JOIN usage_records u ON u.organization_id=o.id AND u.month=date_trunc('month',now() AT TIME ZONE 'UTC')::date ORDER BY o.created_at DESC,o.id LIMIT $1 OFFSET $2",
        "delivery"=>"SELECT o.name AS workspace,w.name AS website,e.state,e.attempts,e.accepted_at FROM event_outbox e JOIN websites w ON w.id=e.website_id JOIN organizations o ON o.id=e.organization_id ORDER BY e.accepted_at DESC,e.website_id,e.event_id LIMIT $1 OFFSET $2",
        "audit"=>"SELECT * FROM (SELECT o.name AS workspace,u.email AS actor,a.action,a.created_at,a.id AS record_id FROM audit_logs a JOIN organizations o ON o.id=a.organization_id LEFT JOIN users u ON u.id=a.actor_id UNION ALL SELECT coalesce(o.name,'Deleted workspace') AS workspace,u.email AS actor,a.action,a.created_at,a.id AS record_id FROM platform_audit a LEFT JOIN organizations o ON o.id=a.resource_id LEFT JOIN users u ON u.id=a.actor_id) combined ORDER BY created_at DESC,record_id DESC LIMIT $1 OFFSET $2",
        _=>return Err(ApiError::not_found()),
    };
    let mut rows = sqlx::query(query)
        .bind(limit)
        .bind(offset)
        .fetch_all(&s.store.pg)
        .await?;
    let visible = if section == "dashboard" { 5 } else { 100 };
    let more = rows.len() > visible;
    rows.truncate(visible);
    let headers: &[&str] = match section {
        "dashboard" | "workspaces" => &["Workspace", "Plan", "Status", "Websites", "Members"],
        "users" => &["Email", "Platform access", "Workspaces", "Joined (UTC)"],
        "websites" => &[
            "Website",
            "Origin",
            "Workspace",
            "Verification",
            "Collection",
        ],
        "subscriptions" => &[
            "Workspace",
            "Plan",
            "Status",
            "Provider",
            "Events this month",
            "Allowance",
        ],
        "delivery" => &[
            "Workspace",
            "Website",
            "Delivery status",
            "Retries",
            "Accepted (UTC)",
        ],
        _ => &["Workspace", "Actor", "Action", "Recorded (UTC)"],
    };
    let mut values = Vec::new();
    for row in rows {
        let text = |column: &str| row.try_get::<String, _>(column);
        let count = |column: &str| row.try_get::<i64, _>(column).map(|n| n.to_string());
        let date = |column: &str| {
            row.try_get::<chrono::DateTime<chrono::Utc>, _>(column)
                .map(|d| d.format("%d %b %Y · %H:%M").to_string())
        };
        let cells = match section {
            "dashboard" | "workspaces" => vec![
                text("name")?,
                text("plan")?,
                text("status")?,
                count("websites")?,
                count("members")?,
            ],
            "users" => vec![
                text("email")?,
                if row.try_get::<bool, _>("owner")? {
                    "Platform owner"
                } else {
                    "Customer"
                }
                .into(),
                count("workspaces")?,
                date("created_at")?,
            ],
            "websites" => vec![
                text("name")?,
                text("origin")?,
                text("workspace")?,
                if row.try_get::<bool, _>("verified")? {
                    "Verified"
                } else {
                    "Pending"
                }
                .into(),
                if row.try_get::<bool, _>("active")? {
                    "Active"
                } else {
                    "Paused"
                }
                .into(),
            ],
            "subscriptions" => {
                let provider = text("provider")?;
                vec![
                    text("name")?,
                    text("plan")?,
                    text("status")?,
                    if provider == "none" {
                        "Not connected".into()
                    } else {
                        provider
                    },
                    count("used")?,
                    count("allowance")?,
                ]
            }
            "delivery" => vec![
                text("workspace")?,
                text("website")?,
                text("state")?,
                row.try_get::<i32, _>("attempts")?.to_string(),
                date("accepted_at")?,
            ],
            _ => vec![
                text("workspace")?,
                row.try_get::<Option<String>, _>("actor")?
                    .unwrap_or_else(|| "Deleted account".into()),
                text("action")?,
                date("created_at")?,
            ],
        };
        values.push(cells);
    }
    let table_title = if section == "dashboard" {
        "Customer workspaces"
    } else {
        label
    };
    let badge = if section == "dashboard" {
        "Latest 5"
    } else {
        "100 per page"
    };
    content.push_str(&format!(r#"<section class="panel"><div class="panel-heading"><h2>{}</h2><span class="badge">{badge}</span></div><div class="owner-table"><table><thead><tr>"#,esc(table_title)));
    for heading in headers {
        content.push_str(&format!("<th>{}</th>", esc(heading)));
    }
    content.push_str("</tr></thead><tbody>");
    if values.is_empty() {
        content.push_str(&format!(r#"<tr><td colspan="{}"><div class="empty-table"><span>No records to show here yet.</span></div></td></tr>"#,headers.len()));
    }
    for row in values {
        content.push_str("<tr>");
        for value in row {
            content.push_str(&format!("<td>{}</td>", esc(&value)));
        }
        content.push_str("</tr>");
    }
    content.push_str("</tbody></table></div>");
    if section == "dashboard" {
        content.push_str(
            r#"<a class="btn outlined small" href="/owner/workspaces">View all workspaces →</a>"#,
        );
    } else {
        content.push_str(&format!(r#"<nav class="owner-pagination" aria-label="Pagination"><span class="muted">Page {page}</span>"#));
        if page > 1 {
            content.push_str(&format!(
                r#"<a class="btn outlined small" href="/owner/{section}?page={}">Previous</a>"#,
                page - 1
            ));
        }
        if more && page < 10000 {
            content.push_str(&format!(
                r#"<a class="btn outlined small" href="/owner/{section}?page={}">Next</a>"#,
                page + 1
            ));
        }
        content.push_str("</nav>");
    }
    content.push_str("</section>");
    let description = match section {
        "dashboard" => "Customer workspaces, websites and event delivery.",
        "workspaces" => "Customer organizations and their current plans.",
        "users" => "Registered accounts and their platform access.",
        "websites" => "Connected websites and their collection status.",
        "subscriptions" => "Current plans, billing connections and accepted monthly usage.",
        "delivery" => "Recent accepted events and their delivery status.",
        _ => "Activity recorded in your platform audit trail.",
    };
    let body = format!(
        r#"<header class="topbar"><div class="breadcrumb">Your platform <span>/</span> {label}</div><span class="status"><span class="tiny-dot"></span> Owner console</span></header><div class="page owner-console"><div class="page-heading"><div><span class="eyebrow">PLATFORM OWNER</span><h1>{title}</h1><p>{description}</p></div><a class="btn outlined" href="/owner/{section}?page={page}">↻ Refresh</a></div>{content}<footer class="page-footer"><span><span class="tiny-dot"></span> A clear view of your platform.</span></footer></div>"#
    );
    Ok(owner_shell(h, &owner.email, section, body).into_response())
}
