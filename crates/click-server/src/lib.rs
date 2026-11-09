pub mod analytics;
pub mod api;
pub mod auth;
pub mod billing;
pub mod commercial;
pub mod config;
pub mod costs;
pub mod dashboard;
pub mod demo;
pub mod explore;
mod explore_ui;
pub mod insights;
pub mod integrations;
pub mod jobs;
pub mod landing;
pub mod mail;
pub mod manage;
pub mod native;
pub mod newsletters;
pub mod operations;
pub mod owner;
pub mod owner_security;
pub mod settings;
pub mod telemetry;
pub mod workspace;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use click_core::GeoIp;
use click_store::Store;
use config::Config;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub integrations: Arc<integrations::Connections>,
    pub operations: Arc<operations::Operations>,
    pub billing: Arc<billing::Service>,
    pub mail: Arc<mail::Mailer>,
    pub config: Arc<Config>,
    pub geoip: Arc<GeoIp>,
    pub password_slots: Arc<Semaphore>,
    pub query_slots: Arc<Semaphore>,
    pub tenant_queries: Arc<TenantLoad>,
    pub dummy_password_hash: Arc<str>,
    rate: Arc<Mutex<RateWindow>>,
}
struct RateWindow {
    since: Instant,
    buckets: HashMap<(IpAddr, bool), u32>,
}

/// How many analytical queries one workspace may have in flight at once.
///
/// `query_slots` already bounds the whole process, but a single limit shared
/// by everyone is a limit one tenant can take all of: sixteen slow reports
/// from one busy workspace and every other customer's dashboard answers
/// "Reports are busy", through no fault of their own. This caps each
/// workspace well below the global figure, so a workspace can saturate its
/// own share and nobody else's.
///
/// Deliberately a count per organization rather than a `Semaphore` each: a
/// permit map is bounded by the number of *concurrently active* workspaces,
/// where a semaphore per workspace would be retained for every workspace that
/// ever ran a report.
pub struct TenantLoad {
    inner: Mutex<HashMap<Uuid, u32>>,
    limit: u32,
}

/// Holds one workspace's slot for as long as the request needs it.
pub struct TenantPermit {
    load: Arc<TenantLoad>,
    org: Uuid,
}

impl TenantLoad {
    fn new(limit: u32) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            limit,
        }
    }

    /// Take a slot for `org`, or `None` when it already holds its share.
    ///
    /// Never waits. A report that queues behind its own workspace's work is a
    /// request holding a connection and a thread to no purpose; telling the
    /// caller to retry is both cheaper and more honest.
    pub fn try_acquire(self: &Arc<Self>, org: Uuid) -> Option<TenantPermit> {
        let mut inner = self.inner.lock().expect("tenant load mutex poisoned");
        let count = inner.entry(org).or_insert(0);
        if *count >= self.limit {
            // Leaves a zero entry only if this is a first attempt that failed
            // with limit 0, which cannot happen; otherwise the entry is live.
            return None;
        }
        *count += 1;
        Some(TenantPermit {
            load: Arc::clone(self),
            org,
        })
    }

    #[cfg(test)]
    fn in_flight(&self, org: Uuid) -> u32 {
        self.inner
            .lock()
            .expect("tenant load mutex poisoned")
            .get(&org)
            .copied()
            .unwrap_or(0)
    }
}

impl Drop for TenantPermit {
    fn drop(&mut self) {
        let mut inner = self.load.inner.lock().expect("tenant load mutex poisoned");
        if let Some(count) = inner.get_mut(&self.org) {
            *count = count.saturating_sub(1);
            // Removed at zero so the map tracks active workspaces rather than
            // growing once per workspace that has ever run a report.
            if *count == 0 {
                inner.remove(&self.org);
            }
        }
    }
}
impl AppState {
    pub fn billing_service(&self) -> Arc<billing::Service> {
        self.integrations
            .billing()
            .unwrap_or_else(|| self.billing.clone())
    }
    pub fn mail_service(&self) -> Arc<mail::Mailer> {
        self.integrations
            .mail()
            .unwrap_or_else(|| self.mail.clone())
    }
    pub fn geoip_service(&self) -> Arc<GeoIp> {
        self.integrations
            .geoip()
            .unwrap_or_else(|| self.geoip.clone())
    }
    pub fn new(store: Store, config: Config) -> anyhow::Result<Self> {
        let billing = billing::Service::from_env()?;
        let mail = mail::Mailer::from_env(config.allow_localhost)?;
        let mut state = Self::with_services(store, config, billing, mail)?;
        state.operations = Arc::new(operations::Operations::from_env()?);
        Ok(state)
    }
    /// Explicit services keep isolated fixtures independent of operator credentials.
    pub fn with_services(
        store: Store,
        config: Config,
        billing: billing::Service,
        mail: mail::Mailer,
    ) -> anyhow::Result<Self> {
        use argon2::{
            password_hash::{rand_core::OsRng, SaltString},
            Argon2, PasswordHasher,
        };
        let geoip = GeoIp::open(config.geoip_path.as_deref())?;
        let dummy = Argon2::default()
            .hash_password(auth::secret().as_bytes(), &SaltString::generate(&mut OsRng))
            .map_err(|_| anyhow::anyhow!("Cannot initialize authentication"))?
            .to_string();
        Ok(Self {
            store,
            integrations: Arc::new(integrations::Connections::memory()),
            operations: Arc::new(operations::Operations::local()),
            billing: Arc::new(billing),
            mail: Arc::new(mail),
            config: Arc::new(config),
            geoip: Arc::new(geoip),
            password_slots: Arc::new(Semaphore::new(4)),
            query_slots: Arc::new(Semaphore::new(16)),
            tenant_queries: Arc::new(TenantLoad::new(4)),
            dummy_password_hash: dummy.into(),
            rate: Arc::new(Mutex::new(RateWindow {
                since: Instant::now(),
                buckets: HashMap::new(),
            })),
        })
    }
}
pub type AppResult<T> = Result<T, ApiError>;
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: &'static str,
}
impl ApiError {
    pub fn bad(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }
    pub fn forbidden(message: &'static str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message,
        }
    }
    pub fn conflict(message: &'static str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message,
        }
    }
    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "Sign in to continue",
        }
    }
    pub fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "Resource not found",
        }
    }
    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(%error,"request failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "The request could not be completed",
        }
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::internal(e)
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self::internal(e)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(serde_json::json!({"error":{"code":self.status.as_u16(),"message":self.message}})),
        )
            .into_response();
        if self.status == StatusCode::SERVICE_UNAVAILABLE {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "2".parse().unwrap());
        }
        response
    }
}

/// Render a failed request as a page when a browser asked for one.
///
/// Every error in this service is an [`ApiError`], which serializes as JSON
/// because the JSON API is the primary consumer. The dashboard, though, posts
/// real HTML forms — goals, funnels, retention, websites, settings — and a
/// browser that submitted a form and got `{"error":{"code":400,…}}` as its
/// next page has been shown the plumbing. The sign-in and sign-up forms
/// already avoided this by rendering their own page with the message inline,
/// which is better still; this is the floor beneath them, so that no form
/// anywhere can put raw JSON on screen.
///
/// Content-negotiated on `Accept`, so API clients and the tracker are
/// untouched: only a request that explicitly prefers `text/html` is converted.
async fn html_errors(req: Request, next: Next) -> Response {
    let wants_html = req
        .headers()
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"));
    // A form submission navigates; `fetch` from our own pages does not, and
    // sends the same Accept header the page did. `Sec-Fetch-Mode: navigate`
    // distinguishes them where the browser sends it, and falling back to
    // Accept alone is the safe direction: a page is readable either way.
    let navigating = req
        .headers()
        .get("sec-fetch-mode")
        .and_then(|value| value.to_str().ok())
        .map(|mode| mode == "navigate");
    let theme = auth::cookie_value(req.headers(), "click_theme").unwrap_or_default();
    let owner_area = req.uri().path().starts_with("/owner");
    let mut response = next.run(req).await;
    // Axum extractors reject malformed JSON, forms, paths and query strings
    // before handlers run. Normalize their plain-text responses too.
    if response.status().is_client_error()
        && !response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| {
                v.as_bytes().starts_with(b"application/json")
                    || v.as_bytes().starts_with(b"text/html")
            })
    {
        let status = response.status();
        let message = match status {
            StatusCode::NOT_FOUND => "Resource not found",
            StatusCode::METHOD_NOT_ALLOWED => "This action is not supported",
            StatusCode::PAYLOAD_TOO_LARGE => "The request is too large",
            StatusCode::UNSUPPORTED_MEDIA_TYPE => {
                "Send this request using the expected content type"
            }
            _ => "Check the required fields and their formats, then try again",
        };
        let headers = response.headers().clone();
        response = (
            status,
            Json(serde_json::json!({"error":{"code":status.as_u16(),"message":message}})),
        )
            .into_response();
        for (name, value) in &headers {
            if name != header::CONTENT_TYPE && name != header::CONTENT_LENGTH {
                response.headers_mut().insert(name.clone(), value.clone());
            }
        }
    }

    let is_error = response.status().is_client_error() || response.status().is_server_error();
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|kind| kind.starts_with("application/json"));
    if !is_error || !is_json || !wants_html || navigating == Some(false) {
        return response;
    }

    let status = response.status();
    let original_headers = response.headers().clone();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let (_, body) = response.into_parts();
    // Error bodies are one short sentence; this is a cap, not a budget.
    let message = match axum::body::to_bytes(body, 8 * 1024).await {
        Ok(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|json| json["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Something went wrong.".into()),
        Err(_) => "Something went wrong.".into(),
    };
    let heading = match status {
        StatusCode::NOT_FOUND => "Not found",
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "You cannot do that",
        StatusCode::CONFLICT => "That conflicts",
        StatusCode::TOO_MANY_REQUESTS => "Too many requests",
        s if s.is_server_error() => "Something went wrong",
        _ => "That did not work",
    };
    let retry_note = retry_after
        .as_deref()
        .map(|seconds| {
            format!(
                r#"<p class="muted">You can try again in {} seconds.</p>"#,
                dashboard::esc(seconds)
            )
        })
        .unwrap_or_default();
    let mut headers = original_headers;
    headers.remove(header::CONTENT_TYPE);
    headers.remove(header::CONTENT_LENGTH);
    if let Some(seconds) = &retry_after {
        if let Ok(value) = header::HeaderValue::from_str(seconds) {
            headers.insert(header::RETRY_AFTER, value);
        }
    }
    // Hand `document` the viewer's theme back, so an error page is not the one
    // page that ignores their colour scheme.
    let mut doc_headers = HeaderMap::new();
    if let Ok(value) = header::HeaderValue::from_str(&format!("click_theme={theme}")) {
        doc_headers.insert(header::COOKIE, value);
    }
    let destination = if owner_area {
        "/owner/dashboard"
    } else {
        "/dashboard"
    };
    let signin = if owner_area {
        "/owner/signin"
    } else {
        "/signin"
    };
    let page = dashboard::document(
        heading,
        &doc_headers,
        &format!(
            r#"<main id="main" class="auth-main"><div class="auth-card"><span class="pill">{code}</span><h2>{heading}</h2><p class="alert" role="alert">{message}</p>{retry_note}<p class="auth-other"><a href="{destination}">Return to dashboard</a> · <a href="{signin}">Sign in</a></p></div></main>"#,
            code = status.as_u16(),
            heading = dashboard::esc(heading),
            message = dashboard::esc(&message),
        ),
    );
    (status, headers, page).into_response()
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .merge(native::router())
        .merge(explore::router())
        .merge(costs::router())
        .route("/v1/websites/{id}/newsletters", get(newsletters::report))
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(api::ready))
        .route("/", get(landing::home))
        .route("/signin", get(dashboard::signin))
        .route("/signup", get(dashboard::signup))
        // `/login` and `/owner/login` are not routes and never were: the
        // pages are `/signin` and `/owner/signin`, and the `login` paths are
        // the POST form targets under `/auth`. They are the obvious thing to
        // type anyway, and the owner one answered 404 — the catch-all
        // `/owner/{section}` rejects an unknown section before it ever
        // considers who is asking, so the guess looked like a broken portal
        // rather than a wrong address. Both now point at the real page.
        .route("/login", get(|| async { Redirect::to("/signin") }))
        .route("/register", get(|| async { Redirect::to("/signup") }))
        .route("/forgot-password", get(mail::forgot))
        .route("/account/reset/request", post(mail::request_form))
        .route(
            "/account/reset",
            get(mail::reset_page).post(mail::reset_form),
        )
        .route(
            "/account/verify",
            get(mail::verify_page).post(mail::verify_form),
        )
        .route("/assets/account-token.js", get(mail::script))
        .route("/v1/account/reset/request", post(mail::reset_request_api))
        .route("/v1/account/reset", post(mail::reset_complete_api))
        .route(
            "/v1/account/verification",
            post(mail::verification_request_api),
        )
        .route("/v1/account/verify", post(mail::verify_complete_api))
        .route("/settings", get(settings::page))
        .route("/settings/action", post(settings::action))
        .route("/invite", get(settings::invite_page))
        .route("/assets/invite.js", get(settings::invite_script))
        .route("/owner", get(owner::home))
        .route("/owner/signin", get(owner::signin))
        // Ahead of `/owner/{section}`: a static path wins over a capture, so
        // this is reached rather than being treated as a directory name.
        .route(
            "/owner/login",
            get(|| async { Redirect::to("/owner/signin") }),
        )
        .route("/owner/auth/login", post(owner::login))
        .route(
            "/owner/auth/verify",
            get(owner_security::challenge).post(owner_security::verify),
        )
        .route("/owner/account", get(owner_security::account))
        .route(
            "/owner/account/action",
            post(owner_security::account_action),
        )
        .route("/owner/forgot-password", get(owner_security::forgot))
        .route(
            "/owner/account/reset/request",
            post(owner_security::request_reset),
        )
        .route(
            "/owner/account/reset",
            get(owner_security::reset_page).post(owner_security::reset),
        )
        .route("/owner/auth/logout", post(owner::logout))
        .route("/owner/dashboard", get(owner::dashboard))
        .route("/owner/appearance", post(owner::appearance))
        .route("/owner/actions", post(owner::action))
        .route("/owner/{section}", get(owner::directory))
        .route("/owner/v1/me", get(owner::me))
        .route("/auth/login", post(dashboard::login))
        .route("/auth/register", post(dashboard::register))
        .route("/auth/logout", post(dashboard::logout))
        .route("/dashboard", get(dashboard::overview))
        .route("/insights", get(insights::page))
        .route("/insights/action", post(insights::action))
        .route("/insights/cost", post(insights::record_cost))
        .route(
            "/websites",
            get(dashboard::websites).post(dashboard::create_website),
        )
        .route("/websites/{id}/verify", post(dashboard::verify))
        .route("/goals", post(dashboard::create_goal))
        .route("/goals/action", post(dashboard::goal_action))
        .route("/appearance", post(dashboard::appearance))
        .route("/internal/metrics", get(operations::metrics))
        .route("/owner/commercial", get(commercial::page))
        .route("/assets/connections.js", get(integrations::script))
        .route(
            "/owner/connections",
            get(integrations::page).post(integrations::action),
        )
        .route("/owner/v1/commercial", get(commercial::api))
        .route("/owner/commercial/retry-mail", post(commercial::retry_mail))
        .route("/assets/app.css", get(dashboard::stylesheet))
        .route("/assets/landing.css", get(landing::stylesheet))
        .route(
            "/vitals.js",
            get(|| async {
                (
                    [
                        (
                            header::CONTENT_TYPE,
                            "application/javascript; charset=utf-8",
                        ),
                        (header::CACHE_CONTROL, "public, max-age=300"),
                    ],
                    include_str!("../../../tracker/dist/vitals.js"),
                )
            }),
        )
        .route("/playground/{token}", get(dashboard::playground))
        .route(
            "/playground/{token}/{page}",
            get(dashboard::playground_page),
        )
        .route(
            "/tracker.js",
            get(|| async {
                (
                    [
                        (
                            header::CONTENT_TYPE,
                            "application/javascript; charset=utf-8",
                        ),
                        (header::CACHE_CONTROL, "public, max-age=300"),
                    ],
                    include_str!("../../../tracker/dist/tracker.js"),
                )
            }),
        )
        .route("/v1/auth/register", post(api::register))
        .route("/v1/billing/webhooks/stripe", post(billing::webhook))
        .route("/v1/auth/login", post(api::login))
        .route("/v1/auth/logout", post(api::logout))
        .route("/v1/me", get(api::me))
        .route(
            "/v1/account/profile",
            axum::routing::patch(manage::profile_api),
        )
        .route("/v1/account/password", post(manage::password_api))
        .route(
            "/v1/account/sessions/revoke-others",
            post(manage::sessions_api),
        )
        .route(
            "/v1/organizations",
            get(api::organizations).post(workspace::create),
        )
        .route(
            "/v1/organizations/{org}",
            axum::routing::patch(workspace::rename),
        )
        .route("/v1/organizations/{org}/members", get(workspace::members))
        .route(
            "/v1/organizations/{org}/members/{user}",
            axum::routing::patch(workspace::set_role).delete(workspace::remove_member),
        )
        .route(
            "/v1/organizations/{org}/invitations",
            post(workspace::invite),
        )
        .route(
            "/v1/organizations/{org}/invitations/{id}",
            axum::routing::delete(workspace::revoke),
        )
        .route("/v1/invitations/accept", post(workspace::accept))
        .route("/v1/websites", get(api::websites).post(api::create_website))
        .route("/v1/websites/{id}/verify", post(api::verify))
        .route(
            "/v1/websites/{id}/retention",
            axum::routing::put(jobs::set_retention),
        )
        .route(
            "/v1/websites/{id}/exports",
            get(jobs::exports).post(jobs::create_export),
        )
        .route("/v1/exports/{id}/download", get(jobs::download))
        .route("/v1/deletions/{id}", get(jobs::deletion_status))
        .route(
            "/v1/websites/{id}",
            axum::routing::patch(manage::update).delete(jobs::delete_website),
        )
        .route(
            "/v1/websites/{id}/goals/{goal}",
            axum::routing::patch(manage::update_goal).delete(manage::delete_goal),
        )
        .route("/v1/websites/{id}/overview", get(api::overview))
        .route(
            "/v1/websites/{id}/funnels",
            get(analytics::list).post(analytics::create),
        )
        .route(
            "/v1/websites/{id}/funnels/{funnel}",
            get(analytics::report)
                .patch(analytics::update)
                .delete(analytics::remove),
        )
        .route("/v1/websites/{id}/journeys", get(analytics::journeys))
        .route("/v1/websites/{id}/vitals", get(analytics::vitals))
        .route("/v1/websites/{id}/revenue", get(analytics::revenue))
        .route(
            "/v1/websites/{id}/backend-events",
            post(analytics::backend_event),
        )
        .route(
            "/v1/websites/{id}/breakdown/{dimension}",
            get(api::breakdown),
        )
        .route(
            "/v1/websites/{id}/goals",
            get(api::goals).post(api::create_goal),
        )
        .route(
            "/v1/websites/{id}/keys",
            get(manage::keys).post(manage::create_key),
        )
        .route(
            "/v1/websites/{id}/keys/{key_id}",
            axum::routing::delete(api::revoke_key).put(manage::rotate_key),
        )
        .route("/v1/organizations/{id}/usage", get(api::usage))
        .route(
            "/v1/collect/{token}",
            post(api::collect).options(api::collect_options),
        )
        .route(
            "/v1/openapi.json",
            get(|| async {
                Json(
                    serde_json::from_str::<serde_json::Value>(include_str!(
                        "../../../docs/openapi.json"
                    ))
                    .expect("checked OpenAPI JSON"),
                )
            }),
        )
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            telemetry::observe,
        ))
        // Outside `guard`, so its own 429 and 503 are negotiated too.
        .layer(middleware::from_fn(html_errors))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
        .with_state(state)
}
/// A refusal the whole stack can render: the same JSON shape every
/// [`ApiError`] uses, plus `Retry-After`.
///
/// These two used to answer with a bare string, so a browser that had simply
/// clicked something saw `Reports are busy. Please retry shortly.` as a plain
/// text document with no styling and no way back. Shaped like every other
/// error, `html_errors` turns them into the same page as the rest, and the
/// `Retry-After` becomes the "try again in N seconds" line on it.
fn refusal(status: StatusCode, retry_after: &'static str, message: &'static str) -> Response {
    (
        status,
        [(header::RETRY_AFTER, retry_after)],
        Json(serde_json::json!({"error":{"code":status.as_u16(),"message":message}})),
    )
        .into_response()
}

async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    // Liveness and protected metrics must remain available when PG is down.
    if matches!(path, "/healthz" | "/readyz" | "/internal/metrics") {
        return next.run(req).await;
    }
    let analytical = req.method() == axum::http::Method::GET
        && (matches!(path, "/dashboard" | "/insights") || path.starts_with("/v1/websites/"));
    let _query_permit = if analytical {
        match state.query_slots.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => {
                return refusal(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "2",
                    "Reports are busy right now. Please try again in a moment.",
                )
            }
        }
    } else {
        None
    };
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let ip = click_core::client_ip(
        peer,
        req.headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok()),
        &state.config.trusted_proxies,
    );
    let sensitive = req.uri().path().starts_with("/auth/")
        || req.uri().path().starts_with("/v1/auth/")
        || req.uri().path().starts_with("/owner/auth/")
        || req.uri().path().starts_with("/owner/account/")
        || req.uri().path().starts_with("/account/")
        || req.uri().path().starts_with("/v1/account/");
    let limited = if state.operations.distributed {
        match state.operations.edge(&state.store, ip, sensitive).await {
            Ok(allowed) => !allowed,
            Err(_) => {
                return refusal(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "2",
                    "Shared request budgets are unavailable; retry shortly",
                )
            }
        }
    } else {
        let mut window = state.rate.lock().expect("rate mutex poisoned");
        if window.since.elapsed() > Duration::from_secs(60) {
            window.buckets.clear();
            window.since = Instant::now();
        }
        if window.buckets.len() >= 10000 && !window.buckets.contains_key(&(ip, sensitive)) {
            true
        } else {
            let count = window.buckets.entry((ip, sensitive)).or_default();
            *count += 1;
            *count > if sensitive { 20 } else { 600 }
        }
    };
    if limited {
        return refusal(
            StatusCode::TOO_MANY_REQUESTS,
            "60",
            "Too many requests from this address. Please try again in a minute.",
        );
    }
    // Four coordinating writers leave database connections for handler queries.
    // A backup holds the exclusive counterpart; new mutations fail closed.
    let _write_slot;
    let _barrier;
    if !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) {
        _write_slot = match state.operations.writes.clone().acquire_owned().await {
            Ok(slot) => Some(slot),
            Err(_) => {
                return refusal(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "2",
                    "Writes are busy; retry shortly",
                )
            }
        };
        _barrier = match state.store.maintenance_guard().await {
            Ok(Some(guard)) => Some(guard),
            _ => {
                return refusal(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "30",
                    "Maintenance is in progress; retry shortly",
                )
            }
        };
    } else {
        _write_slot = None;
        _barrier = None;
    }
    next.run(req).await
}

async fn security_headers(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // Browsers enforce the initiating document's form-action across redirects.
    // Billing forms may navigate only to Stripe's two hosted billing origins.
    let form_action = if matches!(req.uri().path(), "/settings" | "/settings/action") {
        "'self' https://checkout.stripe.com https://billing.stripe.com"
    } else {
        "'self'"
    };
    let mut response = next.run(req).await;
    let h = response.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, format!("default-src 'none'; style-src 'self'; script-src 'self'; img-src 'self' data:; connect-src 'self'; form-action {form_action}; frame-ancestors 'none'; base-uri 'none'").parse().unwrap());
    h.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    h.insert(
        header::REFERRER_POLICY,
        "strict-origin-when-cross-origin".parse().unwrap(),
    );
    h.entry(header::CACHE_CONTROL)
        .or_insert("private, no-store".parse().unwrap());
    if state.config.secure_cookie {
        h.insert(
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=31536000".parse().unwrap(),
        );
    }
    response
}

pub async fn worker(store: Store, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            break;
        }
        match store.drain_once().await {
            Ok(n) if n > 0 => continue,
            Ok(_) => {}
            Err(error) => {
                tracing::error!(%error,"outbox delivery failed; batch retained for retry")
            }
        }
        tokio::select! { _=tokio::time::sleep(Duration::from_secs(1))=>{}, _=shutdown.changed()=>{} }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One workspace may hold its share and no more, and releasing a permit
    /// returns the slot. The whole point is that a busy workspace cannot take
    /// slots a quiet one needs.
    #[test]
    fn a_workspace_is_capped_and_releases_its_slots() {
        let load = Arc::new(TenantLoad::new(2));
        let busy = Uuid::new_v4();
        let quiet = Uuid::new_v4();

        let first = load.try_acquire(busy).expect("first slot");
        let second = load.try_acquire(busy).expect("second slot");
        assert!(
            load.try_acquire(busy).is_none(),
            "a workspace took more than its share"
        );
        // A different workspace is unaffected by the first one's load.
        let other = load
            .try_acquire(quiet)
            .expect("another workspace is not blocked");
        assert_eq!(load.in_flight(busy), 2);

        drop(second);
        assert_eq!(load.in_flight(busy), 1);
        load.try_acquire(busy).expect("a released slot is reusable");

        drop(first);
        drop(other);
    }

    /// The map tracks workspaces that are *active*, not every workspace that
    /// has ever run a report, so it cannot grow without bound.
    #[test]
    fn finished_workspaces_leave_no_entry_behind() {
        let load = Arc::new(TenantLoad::new(2));
        let org = Uuid::new_v4();
        {
            let _permit = load.try_acquire(org).expect("slot");
            assert_eq!(load.in_flight(org), 1);
        }
        assert_eq!(load.in_flight(org), 0);
        assert!(
            load.inner.lock().unwrap().is_empty(),
            "a finished workspace was left in the map"
        );
    }
}
