//! Native account, organization, team, credential and privacy workflows.
use crate::{
    api, auth,
    dashboard::{self, esc},
    jobs, manage, workspace, ApiError, AppResult, AppState,
};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use click_core::Role;
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;
const TABS: &[(&str, &str)] = &[
    ("workspaces", "Workspaces"),
    ("team", "Team"),
    ("websites", "Websites"),
    ("keys", "API keys"),
    ("billing", "Billing & usage"),
    ("privacy", "Privacy & exports"),
    ("account", "My account"),
];
#[derive(Deserialize, Default)]
pub struct SettingsQuery {
    pub org: Option<Uuid>,
    pub site: Option<Uuid>,
    pub tab: Option<String>,
}
fn field(label: &str, name: &str, value: &str, kind: &str) -> String {
    format!(
        r#"<label>{}<input type="{}" name="{}" value="{}" required maxlength="256"></label>"#,
        esc(label),
        esc(kind),
        esc(name),
        esc(value)
    )
}
fn form(action: &str, org: Option<Uuid>, site: Option<Uuid>, body: &str) -> String {
    format!(
        r#"<form method="post" action="/settings/action" class="settings-form"><input type="hidden" name="action" value="{}">{}{}{body}</form>"#,
        esc(action),
        org.map(|id| format!(r#"<input type="hidden" name="organization_id" value="{id}">"#))
            .unwrap_or_default(),
        site.map(|id| format!(r#"<input type="hidden" name="website_id" value="{id}">"#))
            .unwrap_or_default()
    )
}
fn hidden(name: &str, value: Uuid) -> String {
    format!(
        r#"<input type="hidden" name="{}" value="{value}">"#,
        esc(name)
    )
}
fn panel(title: &str, body: &str) -> String {
    format!(
        r#"<section class="panel settings-panel"><div class="panel-heading"><h2>{}</h2></div>{body}</section>"#,
        esc(title)
    )
}
fn button(label: &str) -> String {
    format!(
        r#"<button class="btn primary small">{}</button>"#,
        esc(label)
    )
}
fn table(head: &str, body: &str) -> String {
    format!(
        r#"<div class="owner-table"><table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table></div>"#
    )
}

pub async fn page(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<SettingsQuery>,
) -> AppResult<Response> {
    render(&s, &h, q, None, None).await
}
async fn render(
    s: &AppState,
    h: &HeaderMap,
    q: SettingsQuery,
    notice: Option<&str>,
    secret: Option<&str>,
) -> AppResult<Response> {
    let user = match auth::user(s, h).await {
        Ok(u) => u,
        Err(e) if e.status == StatusCode::UNAUTHORIZED => {
            return Ok(Redirect::to("/signin").into_response())
        }
        Err(e) => return Err(e),
    };
    let orgs = api::list_organizations(s, h).await?;
    let sites = api::list_websites(s, h).await?;
    let selected = q
        .org
        .or_else(|| {
            q.site
                .and_then(|id| sites.iter().find(|w| w.id == id).map(|w| w.organization_id))
        })
        .or_else(|| orgs.first().map(|o| o.id));
    let mut role = Role::Viewer;
    if let Some(org) = selected {
        role = workspace::membership(s, h, org, false).await?.1;
    }
    let org = selected.and_then(|id| orgs.iter().find(|o| o.id == id));
    let selected_sites: Vec<_> = sites
        .iter()
        .filter(|w| Some(w.organization_id) == selected)
        .collect();
    let site = q
        .site
        .and_then(|id| selected_sites.iter().find(|w| w.id == id).copied())
        .or_else(|| selected_sites.iter().find(|w| !w.is_demo).copied())
        .or_else(|| selected_sites.first().copied());
    if q.site.is_some()
        && !sites
            .iter()
            .any(|w| Some(w.id) == q.site && Some(w.organization_id) == selected)
    {
        return Err(ApiError::not_found());
    }
    let tab = q.tab.as_deref().unwrap_or("workspaces");
    if !TABS.iter().any(|(key, _)| *key == tab) {
        return Err(ApiError::not_found());
    }
    let org_id = org.map(|o| o.id);
    let site_id = site.map(|w| w.id);
    let mut nav = String::new();
    for (key, label) in TABS {
        nav.push_str(&format!(
            r#"<a href="/settings?tab={key}{}{}" class="{}" {}>{label}</a>"#,
            org_id
                .map(|id| format!("&amp;org={id}"))
                .unwrap_or_default(),
            site_id
                .map(|id| format!("&amp;site={id}"))
                .unwrap_or_default(),
            if tab == *key { "selected" } else { "" },
            if tab == *key {
                r#"aria-current="page""#
            } else {
                ""
            }
        ));
    }
    let mut org_options = String::new();
    for o in &orgs {
        org_options.push_str(&format!(
            r#"<option value="{}" {}>{} · {}</option>"#,
            o.id,
            if Some(o.id) == org_id { "selected" } else { "" },
            esc(&o.name),
            esc(&o.role)
        ));
    }
    let toolbar = if !org_options.is_empty() {
        format!(
            r#"<form class="settings-switch" action="/settings"><input type="hidden" name="tab" value="{tab}"><label>Workspace<select name="org">{org_options}</select></label><button class="btn outlined small">Switch workspace</button></form>"#
        )
    } else {
        String::new()
    };
    let notice = notice
        .map(|m| format!(r#"<div class="notice" role="status">{}</div>"#, esc(m)))
        .unwrap_or_default();
    let secret=secret.map(|m|format!(r#"<section class="panel"><h2>Save this securely</h2><p>This value is shown once. Store it before leaving this page.</p><pre><code>{}</code></pre><a class="btn outlined small" href="/settings?tab={tab}{}">Continue</a></section>"#,esc(m),org_id.map(|id|format!("&amp;org={id}")).unwrap_or_default())).unwrap_or_default();
    let mut content = String::new();
    match tab {
        "workspaces" => {
            let mut rows = String::new();
            for o in &orgs {
                rows.push_str(&format!(r#"<tr><td>{}</td><td>{}</td><td><a href="/settings?tab=team&amp;org={}">Open workspace →</a></td></tr>"#,esc(&o.name),esc(&o.role),o.id));
            }
            content.push_str(&panel(
                "Your workspaces",
                &table("<th>Workspace</th><th>Your role</th><th></th>", &rows),
            ));
            content.push_str(&panel(
                "Create a workspace",
                &form(
                    "create_workspace",
                    None,
                    None,
                    &(field("Workspace name", "name", "", "text") + &button("Create workspace")),
                ),
            ));
            if let Some(o) = org {
                if role.can_manage() {
                    content.push_str(&panel(
                        "Workspace name",
                        &form(
                            "rename_workspace",
                            Some(o.id),
                            None,
                            &(field("Workspace name", "name", &o.name, "text")
                                + &button("Save workspace")),
                        ),
                    ));
                }
            }
        }
        "team" => {
            if let Some(o) = org {
                let members =
                    workspace::members(State(s.clone()), h.clone(), axum::extract::Path(o.id))
                        .await?
                        .0;
                let mut rows = String::new();
                for member in members {
                    let mut controls = String::new();
                    if role == Role::Owner
                        || role == Role::Admin && !matches!(member.role.as_str(), "owner" | "admin")
                    {
                        let mut options = String::new();
                        for r in ["owner", "admin", "analyst", "viewer"] {
                            if role != Role::Owner && matches!(r, "owner" | "admin") {
                                continue;
                            }
                            options.push_str(&format!(
                                r#"<option value="{r}" {}>{r}</option>"#,
                                if member.role == r { "selected" } else { "" }
                            ));
                        }
                        controls = form(
                            "set_role",
                            Some(o.id),
                            None,
                            &(hidden("target_id", member.id)
                                + &format!(
                                    r#"<label>Role for {}<select name="role">{options}</select></label>"#,
                                    esc(&member.email)
                                )
                                + &button("Update role")),
                        );
                        controls.push_str(&form("remove_member",Some(o.id),None,&(hidden("target_id",member.id)+r#"<details class="settings-confirm"><summary>Remove member</summary><p>This removes their access to this workspace.</p><button class="btn outlined small">Confirm removal</button></details>"#)));
                    }
                    rows.push_str(&format!(
                        "<tr><td>{}</td><td>{}</td><td>{controls}</td></tr>",
                        esc(&member.email),
                        esc(&member.role)
                    ));
                }
                content.push_str(&panel(
                    "People in this workspace",
                    &table("<th>Email</th><th>Role</th><th>Access</th>", &rows),
                ));
                if role.can_manage() {
                    let mut roles = String::new();
                    for r in ["admin", "analyst", "viewer"] {
                        if r == "admin" && role != Role::Owner {
                            continue;
                        }
                        roles.push_str(&format!("<option value=\"{r}\">{r}</option>"));
                    }
                    content.push_str(&panel("Invite a teammate",&form("invite_member",Some(o.id),None,&(field("Teammate email","email","","email")+&format!("<label>Team role<select name=\"role\">{roles}</select></label>")+&button("Create invitation")))));
                    let invites=sqlx::query("SELECT id,email,role,expires_at FROM invitations WHERE organization_id=$1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at>now() ORDER BY created_at DESC").bind(o.id).fetch_all(&s.store.pg).await?;
                    let mut rows = String::new();
                    for i in invites {
                        let id: Uuid = i.try_get("id")?;
                        rows.push_str(&format!(
                            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                            esc(&i.try_get::<String, _>("email")?),
                            esc(&i.try_get::<String, _>("role")?),
                            form(
                                "revoke_invitation",
                                Some(o.id),
                                None,
                                &(hidden("target_id", id) + &button("Revoke invitation"))
                            )
                        ));
                    }
                    content.push_str(&panel(
                        "Pending invitations",
                        &table("<th>Email</th><th>Role</th><th></th>", &rows),
                    ));
                }
            }
        }
        "websites" => {
            for w in &selected_sites {
                if w.is_demo {
                    content.push_str(&panel(&w.name,"<p>Demonstration data is separate from live collection and event usage.</p>"));
                    continue;
                }
                let body = if role.can_manage() {
                    form(
                        "update_website",
                        org_id,
                        Some(w.id),
                        &(field("Website name", "name", &w.name, "text")
                            + &field("Website origin", "origin", &w.origin, "url")
                            + &format!(
                                r#"<label>Collection<select name="active"><option value="true" {}>Enabled</option><option value="false" {}>Paused</option></select></label><p class="muted">Changing the origin requires domain verification again.</p>"#,
                                if w.active { "selected" } else { "" },
                                if !w.active { "selected" } else { "" }
                            )
                            + &button("Save website")),
                    )
                } else {
                    format!(
                        "<p>{}</p><p class=muted>Administrators manage website settings.</p>",
                        esc(&w.origin)
                    )
                };
                content.push_str(&panel(&w.name, &body));
            }
            content.push_str(r#"<a class="btn outlined" href="/websites">Add a website or install your tracker →</a>"#);
        }
        "keys" => {
            if let Some(w) = site {
                content.push_str(&site_switch(&selected_sites, org_id, tab, w.id));
                if role.can_manage() {
                    let keys = manage::keys(State(s.clone()), h.clone(), axum::extract::Path(w.id))
                        .await?
                        .0;
                    let mut rows = String::new();
                    for key in keys {
                        let state = if key.revoked_at.is_some() {
                            "Revoked"
                        } else {
                            "Active"
                        };
                        let mut controls = String::new();
                        if key.revoked_at.is_none() {
                            controls = form(
                                "revoke_key",
                                org_id,
                                Some(w.id),
                                &(hidden("target_id", key.id) + &button("Revoke")),
                            );
                            controls.push_str(&form("rotate_key",org_id,Some(w.id),&(hidden("target_id",key.id)+&format!(r#"<input type="hidden" name="name" value="{}"><input type="hidden" name="scope" value="{}">"#,esc(&key.label),esc(&key.scope))+&button("Rotate"))));
                        }
                        rows.push_str(&format!(
                            "<tr><td>{}</td><td>{}</td><td>{state}</td><td>{controls}</td></tr>",
                            esc(&key.label),
                            esc(&key.scope)
                        ));
                    }
                    content.push_str(&panel(
                        "Website credentials",
                        &table("<th>Name</th><th>Scope</th><th>Status</th><th></th>", &rows),
                    ));
                    content.push_str(&panel("Create an API credential",&form("create_key",org_id,Some(w.id),&(field("Credential name","name","","text")+r#"<label>Permission<select name="scope"><option value="read">Read analytics</option><option value="integration">Send backend conversion events</option></select></label>"#+&button("Create credential")))));
                } else {
                    content.push_str(&panel(
                        "Website credentials",
                        "<p>Workspace owners and administrators manage API credentials.</p>",
                    ));
                }
            }
        }
        "billing" => {
            if let Some(o) = org {
                let row=sqlx::query("SELECT p.id,p.monthly_events,p.website_limit,p.seat_limit,p.retention_days,s.status,s.provider,coalesce(u.accepted_events,0) AS used FROM subscriptions s JOIN plans p ON p.id=s.plan_id LEFT JOIN usage_records u ON u.organization_id=s.organization_id AND u.month=date_trunc('month',now() AT TIME ZONE 'UTC')::date WHERE s.organization_id=$1").bind(o.id).fetch_one(&s.store.pg).await?;
                let used: i64 = row.try_get("used")?;
                let allowance: i64 = row.try_get("monthly_events")?;
                content.push_str(&panel("Your current plan",&format!(r#"<div class="settings-stat-grid"><div><span>Plan</span><strong>{}</strong></div><div><span>Events this month (UTC)</span><strong>{used} / {allowance}</strong></div><div><span>Websites</span><strong>{}</strong></div><div><span>Team seats</span><strong>{}</strong></div><div><span>Retention limit</span><strong>{} days</strong></div></div><p>Status: {} · Provider: {}</p><p class="muted">Only accepted live events count toward usage. Repeated event IDs and demo data do not create duplicate charges.</p>"#,esc(&row.try_get::<String,_>("id")?),row.try_get::<i32,_>("website_limit")?,row.try_get::<i32,_>("seat_limit")?,row.try_get::<i32,_>("retention_days")?,esc(&row.try_get::<String,_>("status")?),esc(&row.try_get::<String,_>("provider")?))));
                let plans=sqlx::query("SELECT id,monthly_events,website_limit,seat_limit,retention_days FROM plans ORDER BY monthly_events").fetch_all(&s.store.pg).await?;
                let mut rows = String::new();
                for p in plans {
                    rows.push_str(&format!(
                        "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{} days</td></tr>",
                        esc(&p.try_get::<String, _>("id")?),
                        p.try_get::<i64, _>("monthly_events")?,
                        p.try_get::<i32, _>("website_limit")?,
                        p.try_get::<i32, _>("seat_limit")?,
                        p.try_get::<i32, _>("retention_days")?
                    ));
                }
                content.push_str(&panel("Available plans",&table("<th>Plan</th><th>Monthly events</th><th>Websites</th><th>Seats</th><th>Retention</th>",&rows)));
                content.push_str(&crate::billing::settings(s, h, o.id, role).await?);
            }
        }
        "privacy" => {
            if let Some(w) = site {
                content.push_str(&site_switch(&selected_sites, org_id, tab, w.id));
                let days: i32 =
                    sqlx::query_scalar("SELECT retention_days FROM websites WHERE id=$1")
                        .bind(w.id)
                        .fetch_one(&s.store.pg)
                        .await?;
                if role.can_manage() {
                    content.push_str(&panel("Data retention",&form("retention",org_id,Some(w.id),&(field("Retention in days (7–90)","value",&days.to_string(),"number")+r#"<p class="muted">Older records are removed in hourly buckets by the maintenance worker. Shortening retention permanently removes older data.</p>"#+&button("Save retention")))));
                }
                if role.can_edit_goals() {
                    content.push_str(&panel("Export your events",&form("export",org_id,Some(w.id),&(String::from(r#"<label>Export period<select name="value"><option value="7">Last 7 days</option><option value="1">Today</option><option value="30">Last 30 days</option><option value="90">Last 90 days</option></select></label><p class="muted">CSV exports contain up to 100,000 events and expire after 24 hours. Narrow the period for larger datasets.</p>"#)+&button("Create export")))));
                    let exports =
                        jobs::exports(State(s.clone()), h.clone(), axum::extract::Path(w.id))
                            .await?
                            .0;
                    let mut rows = String::new();
                    for job in exports {
                        let download =
                            if job.state == "complete" && job.expires_at > chrono::Utc::now() {
                                format!(
                                    r#"<a href="/v1/exports/{}/download">Download CSV</a>"#,
                                    job.id
                                )
                            } else {
                                esc(job
                                    .error
                                    .as_deref()
                                    .unwrap_or("Refresh this page for the latest status"))
                            };
                        rows.push_str(&format!(
                            "<tr><td>{}</td><td>{}</td><td>{download}</td></tr>",
                            job.created_at.format("%d %b %Y %H:%M UTC"),
                            esc(&job.state)
                        ));
                    }
                    content.push_str(&panel(
                        "Recent exports",
                        &table("<th>Requested</th><th>Status</th><th></th>", &rows),
                    ));
                }
                if role.can_manage() {
                    let body = form(
                        "delete_website",
                        org_id,
                        Some(w.id),
                        &(format!(
                            r#"<details class="settings-confirm"><summary>Delete this website and its analytics</summary><p>This permanently removes analytics, API keys, goals and the website. Type <code>{}</code> to confirm.</p>"#,
                            esc(&w.origin)
                        ) + &field("Confirm website origin", "confirmation", "", "text")
                            + &button("Permanently delete website")
                            + "</details>"),
                    );
                    content.push_str(&panel("Delete website", &body));
                }
            }
            if let Some(o) = org {
                if role == Role::Owner {
                    content.push_str(&panel("Delete workspace",&form("delete_workspace",Some(o.id),None,&(format!(r#"<details class="settings-confirm"><summary>Delete workspace and all website analytics</summary><p>All websites, analytics, team memberships and credentials are removed. Cancel any paid subscription first. Type <code>{}</code> to confirm.</p>"#,esc(&o.name))+&field("Confirm workspace name","confirmation","","text")+&button("Permanently delete workspace")+"</details>"))));
                }
            }
        }
        "account" => {
            let profile =
                sqlx::query("SELECT display_name,email_verified_at FROM users WHERE id=$1")
                    .bind(user.id)
                    .fetch_one(&s.store.pg)
                    .await?;
            let name: String = profile.try_get("display_name")?;
            let verified: Option<chrono::DateTime<chrono::Utc>> =
                profile.try_get("email_verified_at")?;
            content.push_str(&panel(
                "Your profile",
                &form(
                    "profile",
                    org_id,
                    None,
                    &(format!(
                        "<p>Signed in as <strong>{}</strong></p><p class=muted>Email {}</p>",
                        esc(&user.email),
                        if verified.is_some() {
                            "verified"
                        } else {
                            "not verified"
                        }
                    ) + &field("Display name", "name", &name, "text")
                        + &button("Save profile")),
                ),
            ));
            if verified.is_none() {
                content.push_str(&panel(
                    "Verify your email",
                    &form(
                        "verify_email",
                        org_id,
                        None,
                        &button("Request verification link"),
                    ),
                ));
            }
            content.push_str(&panel("Change your password",&form("password",org_id,None,&(field("Current password","current_password","","password")+&field("New password (12–256 characters)","new_password","","password")+r#"<p class="muted">Changing your password signs out your other customer sessions.</p>"#+&button("Change password")))));
            content.push_str(&panel("Your sessions",&form("other_sessions",org_id,None,&(String::from("<p>Keep this browser signed in and revoke your other customer sessions.</p>")+&button("Sign out other devices")))));
        }
        _ => {}
    }
    if content.is_empty() {
        content = panel(
            "Start with a workspace",
            r#"<p>Create a workspace to manage websites, team access and analytics.</p><a class="btn primary" href="/settings?tab=workspaces">Create workspace</a>"#,
        );
    }
    let body = format!(
        r#"<header class="topbar"><div class="breadcrumb">Your workspace <span>/</span> Settings</div><a href="/dashboard" class="status">Back to analytics →</a></header><div class="page"><div class="page-heading"><div><span class="eyebrow">MAKE IT YOUR OWN</span><h1>A space that works for you.</h1><p>Your account, your people, and your websites.</p></div></div>{toolbar}<nav class="settings-tabs" aria-label="Settings sections">{nav}</nav>{notice}{secret}{content}</div>"#
    );
    Ok(dashboard::shell(h, &user.email, "settings", site_id, body).into_response())
}
fn site_switch(
    sites: &[&click_store::Website],
    org: Option<Uuid>,
    tab: &str,
    selected: Uuid,
) -> String {
    let mut options = String::new();
    for w in sites {
        options.push_str(&format!(
            r#"<option value="{}" {}>{}</option>"#,
            w.id,
            if w.id == selected { "selected" } else { "" },
            esc(&w.name)
        ));
    }
    format!(
        r#"<form class="settings-switch" action="/settings"><input type="hidden" name="tab" value="{tab}"><input type="hidden" name="org" value="{}"><label>Website<select name="site">{options}</select></label><button class="btn outlined small">Switch website</button></form>"#,
        org.map(|id| id.to_string()).unwrap_or_default()
    )
}

#[derive(Deserialize)]
pub struct Action {
    action: String,
    organization_id: Option<Uuid>,
    website_id: Option<Uuid>,
    target_id: Option<Uuid>,
    name: Option<String>,
    email: Option<String>,
    role: Option<String>,
    origin: Option<String>,
    active: Option<String>,
    scope: Option<String>,
    value: Option<String>,
    confirmation: Option<String>,
    current_password: Option<String>,
    new_password: Option<String>,
    token: Option<String>,
}
fn require(value: &Option<String>) -> AppResult<String> {
    value
        .clone()
        .ok_or_else(|| ApiError::bad("Required field is missing"))
}
fn id(value: Option<Uuid>) -> AppResult<Uuid> {
    value.ok_or_else(|| ApiError::bad("Select a workspace or website"))
}
pub async fn action(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<Action>,
) -> AppResult<Response> {
    let tab = match input.action.as_str() {
        "invite_member" | "set_role" | "remove_member" | "revoke_invitation" => "team",
        "update_website" => "websites",
        "create_key" | "rotate_key" | "revoke_key" => "keys",
        "retention" | "export" | "delete_website" => "privacy",
        "profile" | "password" | "other_sessions" | "verify_email" => "account",
        "checkout" | "billing_portal" | "billing_test" => "billing",
        _ => "workspaces",
    };
    let fallback = SettingsQuery {
        org: input.organization_id,
        site: input.website_id,
        tab: Some(tab.into()),
    };
    match apply_action(State(s.clone()), h.clone(), Form(input)).await {
        Err(e) if matches!(e.status, StatusCode::BAD_REQUEST | StatusCode::CONFLICT) => {
            let mut response = render(&s, &h, fallback, Some(e.message), None).await?;
            *response.status_mut() = e.status;
            Ok(response)
        }
        result => result,
    }
}
async fn apply_action(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<Action>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    let mut org = input.organization_id;
    let site = input.website_id;
    let (tab, secret, notice) = match input.action.as_str() {
        "create_workspace" => {
            org = Some(
                workspace::add_workspace(
                    &s,
                    &h,
                    workspace::WorkspaceInput {
                        name: require(&input.name)?,
                    },
                )
                .await?,
            );
            ("workspaces", None, "Workspace created")
        }
        "rename_workspace" => {
            workspace::rename_workspace(
                &s,
                &h,
                id(org)?,
                workspace::WorkspaceInput {
                    name: require(&input.name)?,
                },
            )
            .await?;
            ("workspaces", None, "Workspace updated")
        }
        "invite_member" => {
            let invite = workspace::invite_member(
                &s,
                &h,
                id(org)?,
                workspace::InviteInput {
                    email: require(&input.email)?,
                    role: require(&input.role)?,
                },
            )
            .await?;
            ("team",Some(invite.accept_url),"Share this invitation with your teammate. They must sign in with the invited email.")
        }
        "revoke_invitation" => {
            workspace::revoke_invitation(&s, &h, id(org)?, id(input.target_id)?).await?;
            ("team", None, "Invitation revoked")
        }
        "set_role" => {
            workspace::change_member(
                &s,
                &h,
                id(org)?,
                id(input.target_id)?,
                Some(&require(&input.role)?),
            )
            .await?;
            ("team", None, "Team role updated")
        }
        "remove_member" => {
            workspace::change_member(&s, &h, id(org)?, id(input.target_id)?, None).await?;
            return Ok(Redirect::to("/settings?tab=team").into_response());
        }
        "update_website" => {
            manage::update_website(
                &s,
                &h,
                id(site)?,
                manage::SiteInput {
                    name: require(&input.name)?,
                    origin: require(&input.origin)?,
                    active: input.active.as_deref() == Some("true"),
                },
            )
            .await?;
            ("websites", None, "Website settings saved")
        }
        "create_key" | "rotate_key" => {
            let key = manage::issue_key(
                &s,
                &h,
                id(site)?,
                manage::CredentialInput {
                    label: require(&input.name)?,
                    scope: require(&input.scope)?,
                },
                if input.action == "rotate_key" {
                    Some(id(input.target_id)?)
                } else {
                    None
                },
            )
            .await?;
            (
                "keys",
                Some(key.key),
                "Credential created. Keep it on your server; never publish it in your tracker.",
            )
        }
        "revoke_key" => {
            api::revoke_key(
                State(s.clone()),
                h.clone(),
                axum::extract::Path((id(site)?, id(input.target_id)?)),
            )
            .await?;
            ("keys", None, "Credential revoked")
        }
        "retention" => {
            let days = require(&input.value)?
                .parse()
                .map_err(|_| ApiError::bad("Enter a number of days"))?;
            jobs::retention(&s, &h, id(site)?, days).await?;
            (
                "privacy",
                None,
                "Retention saved. The maintenance worker applies the policy.",
            )
        }
        "export" => {
            let days = require(&input.value)?
                .parse()
                .map_err(|_| ApiError::bad("Select an export period"))?;
            jobs::queue_export(&s, &h, id(site)?, days).await?;
            (
                "privacy",
                None,
                "Export queued. Refresh this page to download it when ready.",
            )
        }
        "delete_website" => {
            jobs::delete_site(&s, &h, id(site)?, &require(&input.confirmation)?).await?;
            return render(&s,&h,SettingsQuery{org,site:None,tab:Some("websites".into())},Some("Deletion queued. Collection is stopped; storage is removed by the privacy worker."),None).await;
        }
        "delete_workspace" => {
            jobs::delete_workspace(&s, &h, id(org)?, &require(&input.confirmation)?).await?;
            return Ok(Redirect::to("/settings?tab=workspaces").into_response());
        }
        "profile" => {
            manage::profile(
                &s,
                &h,
                manage::ProfileInput {
                    display_name: require(&input.name)?,
                },
            )
            .await?;
            ("account", None, "Profile saved")
        }
        "password" => {
            let (token, user) = manage::change_password(
                &s,
                &h,
                manage::PasswordInput {
                    current_password: require(&input.current_password)?,
                    new_password: require(&input.new_password)?,
                },
            )
            .await?;
            let mut response = auth::session_response(&s, &token, user);
            *response.status_mut() = StatusCode::SEE_OTHER;
            response
                .headers_mut()
                .insert(header::LOCATION, "/settings?tab=account".parse().unwrap());
            return Ok(response);
        }
        "verify_email" => {
            let user = auth::user(&s, &h).await?;
            crate::mail::request_token(&s, &user.email, "verify").await?;
            ("account", None, s.mail.notice())
        }
        "other_sessions" => {
            manage::other_sessions(&s, &h).await?;
            ("account", None, "Other customer sessions signed out")
        }
        "accept_invitation" => {
            let org = workspace::accept_invitation(
                &s,
                &h,
                workspace::TokenInput {
                    token: require(&input.token)?,
                },
            )
            .await?;
            return Ok(Redirect::to(&format!("/settings?tab=team&org={org}")).into_response());
        }
        "billing_test" => {
            crate::billing::test_plan(&s, &h, id(org)?, &require(&input.value)?).await?;
            (
                "billing",
                None,
                "Development test plan updated. No payment was made.",
            )
        }
        "checkout" => {
            let url = crate::billing::checkout(&s, &h, id(org)?, &require(&input.value)?).await?;
            return Ok(Redirect::to(&url).into_response());
        }
        "billing_portal" => {
            let url = crate::billing::portal(&s, &h, id(org)?).await?;
            return Ok(Redirect::to(&url).into_response());
        }
        _ => return Err(ApiError::bad("Unknown settings action")),
    };
    render(
        &s,
        &h,
        SettingsQuery {
            org,
            site,
            tab: Some(tab.into()),
        },
        Some(notice),
        secret.as_deref(),
    )
    .await
}
pub async fn invite_page(h: HeaderMap) -> Html<String> {
    dashboard::document(
        "Accept invitation",
        &h,
        r#"<main id="main" class="page"><section class="panel setup-panel"><div><span class="eyebrow">A PLACE FOR YOUR TEAM</span><h1>You're invited.</h1><p>Sign in with the invited email, then accept the invitation.</p><p><a href="/signin" target="_blank" rel="noopener">Sign in</a> · <a href="/signup" target="_blank" rel="noopener">Create account</a></p></div><form action="/settings/action" method="post" class="stack"><input type="hidden" name="action" value="accept_invitation"><label>Invitation token<input id="invitation-token" name="token" required maxlength="64" autocomplete="off"></label><button class="btn primary">Accept invitation</button></form></section><script src="/assets/invite.js" defer></script></main>"#,
    )
}
pub async fn invite_script() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        r#"if(/^#[a-f0-9]{64}$/.test(location.hash)){document.getElementById('invitation-token').value=location.hash.slice(1);history.replaceState(null,'',location.pathname)}"#,
    )
}
