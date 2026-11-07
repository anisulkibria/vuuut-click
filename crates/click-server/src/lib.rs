pub mod analytics;
pub mod api;
pub mod auth;
pub mod billing;
pub mod config;
pub mod dashboard;
pub mod demo;
pub mod insights;
pub mod jobs;
pub mod landing;
pub mod mail;
pub mod manage;
pub mod owner;
pub mod settings;
pub mod workspace;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
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
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub billing: Arc<billing::Service>,
    pub mail: Arc<mail::Mailer>,
    pub config: Arc<Config>,
    pub geoip: Arc<GeoIp>,
    pub password_slots: Arc<Semaphore>,
    pub query_slots: Arc<Semaphore>,
    pub dummy_password_hash: Arc<str>,
    rate: Arc<Mutex<RateWindow>>,
}
struct RateWindow {
    since: Instant,
    buckets: HashMap<(IpAddr, bool), u32>,
}
impl AppState {
    pub fn new(store: Store, config: Config) -> anyhow::Result<Self> {
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
            billing: Arc::new(billing::Service::from_env()?),
            mail: Arc::new(mail::Mailer::from_env(config.allow_localhost)?),
            config: Arc::new(config),
            geoip: Arc::new(geoip),
            password_slots: Arc::new(Semaphore::new(4)),
            query_slots: Arc::new(Semaphore::new(16)),
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
        (
            self.status,
            Json(serde_json::json!({"error":{"code":self.status.as_u16(),"message":self.message}})),
        )
            .into_response()
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(api::ready))
        .route("/", get(landing::home))
        .route("/signin", get(dashboard::signin))
        .route("/signup", get(dashboard::signup))
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
        .route("/owner/auth/login", post(owner::login))
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
        .route(
            "/websites",
            get(dashboard::websites).post(dashboard::create_website),
        )
        .route("/websites/{id}/verify", post(dashboard::verify))
        .route("/goals", post(dashboard::create_goal))
        .route("/appearance", post(dashboard::appearance))
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
            axum::routing::delete(manage::delete_goal),
        )
        .route("/v1/websites/{id}/overview", get(api::overview))
        .route(
            "/v1/websites/{id}/funnels",
            get(analytics::list).post(analytics::create),
        )
        .route(
            "/v1/websites/{id}/funnels/{funnel}",
            get(analytics::report).delete(analytics::remove),
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
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let analytical = req.method() == axum::http::Method::GET
        && (matches!(path, "/dashboard" | "/insights") || path.starts_with("/v1/websites/"));
    let _query_permit = if analytical {
        match state.query_slots.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [(header::RETRY_AFTER, "2")],
                    "Reports are busy. Please retry shortly.",
                )
                    .into_response()
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
        || req.uri().path().starts_with("/account/")
        || req.uri().path().starts_with("/v1/account/");
    let limited = {
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
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            "Request allowance reached; retry in a minute",
        )
            .into_response();
    }
    let mut response = next.run(req).await;
    let h = response.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'self'; script-src 'self'; img-src 'self' data:; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'".parse().unwrap());
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
