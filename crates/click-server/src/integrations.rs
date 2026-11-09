//! Owner-managed provider configuration. AEAD keys never enter PostgreSQL;
//! secrets never reappear in forms, status, audits or provider error messages.
use crate::{auth, billing, dashboard::esc, mail, owner, ApiError, AppResult, AppState};
use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Form,
};
use click_core::GeoIp;
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use uuid::Uuid;

type Values = HashMap<String, String>;
#[derive(Default)]
struct Live {
    billing: Option<Arc<billing::Service>>,
    mail: Option<Arc<mail::Mailer>>,
    geoip: Option<Arc<GeoIp>>,
}
pub struct Connections {
    key: [u8; 32],
    baseline: Values,
    live: RwLock<Live>,
    gate: tokio::sync::Mutex<()>,
    versions: std::sync::Mutex<Vec<(String, i64)>>,
    data: PathBuf,
    geo_stamp: std::sync::Mutex<Option<std::time::SystemTime>>,
}
impl Connections {
    pub fn memory() -> Self {
        let mut key = [0; 32];
        SystemRandom::new().fill(&mut key).expect("OS randomness");
        Self::with_key(key, PathBuf::from(".local/connections"))
    }
    pub fn with_key(key: [u8; 32], data: PathBuf) -> Self {
        Self {
            key,
            baseline: std::env::vars().collect(),
            live: RwLock::new(Live::default()),
            gate: tokio::sync::Mutex::new(()),
            versions: std::sync::Mutex::new(Vec::new()),
            data,
            geo_stamp: std::sync::Mutex::new(None),
        }
    }
    pub fn open() -> anyhow::Result<Self> {
        let data = PathBuf::from(
            std::env::var("CONNECTIONS_DATA_DIR").unwrap_or_else(|_| ".local/connections".into()),
        );
        let key = if let Some(value) = std::env::var("CONNECTIONS_KEY")
            .ok()
            .filter(|v| !v.is_empty())
        {
            hex::decode(value)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("CONNECTIONS_KEY must be 32 bytes in hex"))?
        } else {
            private_dir(&data)?;
            let path = data.join("key");
            if !path.exists() {
                let mut key = [0; 32];
                SystemRandom::new()
                    .fill(&mut key)
                    .map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                match options.open(&path) {
                    Ok(mut file) => {
                        use std::io::Write;
                        file.write_all(&key)?;
                        file.sync_all()?;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.into()),
                }
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                anyhow::ensure!(
                    std::fs::metadata(&path)?.permissions().mode() & 0o077 == 0,
                    "Connection key must be private (0600)"
                );
            }
            std::fs::read(path)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("Connection key has invalid length"))?
        };
        Ok(Self::with_key(key, data))
    }
    pub fn billing(&self) -> Option<Arc<billing::Service>> {
        self.live.read().expect("connections lock").billing.clone()
    }
    pub fn mail(&self) -> Option<Arc<mail::Mailer>> {
        self.live.read().expect("connections lock").mail.clone()
    }
    pub fn geoip(&self) -> Option<Arc<GeoIp>> {
        self.live.read().expect("connections lock").geoip.clone()
    }
    fn seal(&self, service: &str, values: &Values) -> anyhow::Result<Vec<u8>> {
        let mut nonce = [0; 12];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
        let mut body = serde_json::to_vec(values)?;
        let key = aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::AES_256_GCM, &self.key)
                .map_err(|_| anyhow::anyhow!("Invalid encryption key"))?,
        );
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(service.as_bytes()),
            &mut body,
        )
        .map_err(|_| anyhow::anyhow!("Encryption failed"))?;
        Ok([nonce.to_vec(), body].concat())
    }
    fn unseal(&self, service: &str, bytes: &[u8]) -> anyhow::Result<Values> {
        anyhow::ensure!(bytes.len() >= 28, "Invalid connection record");
        let nonce: [u8; 12] = bytes[..12].try_into()?;
        let mut body = bytes[12..].to_vec();
        let key = aead::LessSafeKey::new(
            aead::UnboundKey::new(&aead::AES_256_GCM, &self.key)
                .map_err(|_| anyhow::anyhow!("Invalid encryption key"))?,
        );
        let plain = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(service.as_bytes()),
                &mut body,
            )
            .map_err(|_| anyhow::anyhow!("Cannot decrypt connection; restore the matching key"))?;
        Ok(serde_json::from_slice(plain)?)
    }
    async fn saved(&self, s: &AppState, service: &str) -> AppResult<Values> {
        let row: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT ciphertext FROM platform_connections WHERE service=$1")
                .bind(service)
                .fetch_optional(&s.store.pg)
                .await?;
        row.map(|b| {
            self.unseal(service, &b).map_err(|_| {
                ApiError::bad("Cannot read connection; restore the matching encryption key")
            })
        })
        .transpose()
        .map(|v| v.unwrap_or_default())
    }
    fn effective(&self, saved: &Values) -> Values {
        let mut values = self.baseline.clone();
        values.extend(saved.clone());
        values
    }
    pub async fn refresh(&self, s: &AppState) -> anyhow::Result<()> {
        let _guard = self.gate.lock().await;
        let rows: Vec<(String, i64, Vec<u8>)> = sqlx::query_as(
            "SELECT service,revision,ciphertext FROM platform_connections ORDER BY service",
        )
        .fetch_all(&s.store.pg)
        .await?;
        let versions = rows
            .iter()
            .map(|(id, v, _)| (id.clone(), *v))
            .collect::<Vec<_>>();
        if *self.versions.lock().expect("versions lock") == versions {
            if versions.iter().any(|(id, _)| id == "maxmind") {
                let stamp = self
                    .geo_path(s)
                    .and_then(|p| std::fs::metadata(p).ok())
                    .and_then(|m| m.modified().ok());
                if *self.geo_stamp.lock().expect("geo stamp") != stamp {
                    let reader = GeoIp::open(self.geo_path(s).as_deref())?;
                    self.live.write().expect("connections lock").geoip = Some(Arc::new(reader));
                    *self.geo_stamp.lock().expect("geo stamp") = stamp;
                }
            }
            return Ok(());
        }
        let mut live = Live::default();
        for (id, _, data) in rows {
            let values = self.effective(&self.unseal(&id, &data)?);
            match id.as_str() {
                "smtp" => {
                    live.mail = Some(Arc::new(mail::Mailer::from_config(
                        s.config.allow_localhost,
                        &values,
                    )?))
                }
                "stripe" => live.billing = Some(Arc::new(billing::Service::from_config(&values)?)),
                "maxmind" => {
                    let path = self.geo_path(s);
                    live.geoip = Some(Arc::new(GeoIp::open(path.as_deref())?));
                }
                _ => {}
            }
        }
        *self.live.write().expect("connections lock") = live;
        *self.versions.lock().expect("versions lock") = versions;
        Ok(())
    }
    fn geo_path(&self, s: &AppState) -> Option<PathBuf> {
        let managed = self.data.join("GeoLite2-Country.mmdb");
        if managed.exists() {
            Some(managed)
        } else {
            s.config.geoip_path.clone()
        }
    }
    /// Capture telemetry overrides before initializing its process-wide exporter.
    pub async fn telemetry_values(&self, s: &AppState) -> AppResult<Values> {
        self.saved(s, "telemetry").await
    }
}
fn private_dir(path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
struct Field {
    key: &'static str,
    label: &'static str,
    secret: bool,
}
fn fields(service: &str) -> &'static [Field] {
    match service {
        "maxmind" => &[
            Field {
                key: "MAXMIND_ACCOUNT_ID",
                label: "MaxMind account ID",
                secret: false,
            },
            Field {
                key: "MAXMIND_LICENSE_KEY",
                label: "MaxMind license key",
                secret: true,
            },
        ],
        "smtp" => &[
            Field {
                key: "SMTP_HOST",
                label: "SMTP host",
                secret: false,
            },
            Field {
                key: "SMTP_PORT",
                label: "TLS port (587 or 465)",
                secret: false,
            },
            Field {
                key: "SMTP_FROM",
                label: "Sender address",
                secret: false,
            },
            Field {
                key: "SMTP_USERNAME",
                label: "SMTP username",
                secret: true,
            },
            Field {
                key: "SMTP_PASSWORD",
                label: "SMTP password",
                secret: true,
            },
            Field {
                key: "COMMERCIAL_TEST_EMAIL",
                label: "Development delivery inbox",
                secret: false,
            },
        ],
        "stripe" => &[
            Field {
                key: "STRIPE_SECRET_KEY",
                label: "Stripe secret key",
                secret: true,
            },
            Field {
                key: "STRIPE_WEBHOOK_SECRET",
                label: "Webhook signing secret",
                secret: true,
            },
            Field {
                key: "STRIPE_PRICES_JSON",
                label: "Price mapping JSON (plan:monthly or plan:annual to price ID)",
                secret: false,
            },
            Field {
                key: "STRIPE_PORTAL_CONFIGURATION",
                label: "Billing portal configuration ID (optional)",
                secret: false,
            },
        ],
        "telemetry" => &[
            Field {
                key: "OTEL_EXPORTER_OTLP_ENDPOINT",
                label: "OTLP HTTPS endpoint",
                secret: false,
            },
            Field {
                key: "OTEL_EXPORTER_OTLP_HEADERS",
                label: "OTLP authorization headers",
                secret: true,
            },
        ],
        _ => &[],
    }
}
#[derive(Deserialize, Default)]
pub struct Page {
    message: Option<String>,
}
pub async fn page(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<Page>,
) -> AppResult<Response> {
    let user = match owner::authenticated_owner(&s, &h).await {
        Ok(u) => u,
        Err(e) if e.status == StatusCode::UNAUTHORIZED => {
            return Ok(Redirect::to("/owner/signin").into_response())
        }
        Err(e) => return Err(e),
    };
    let mut body=String::from("<header class=\"topbar\"><div class=\"breadcrumb\">Your platform / Connections</div></header><div class=\"page owner-console\"><div class=\"page-heading\"><div><h1>Connect your services.</h1><p>Manage analytics enrichment, email, payments, monitoring and managed hosting.</p></div></div>");
    if let Some(message) = q.message {
        body.push_str(&format!("<p role=\"status\">{}</p>", esc(&message)));
    }
    for (service,title,detail) in [
        ("maxmind","MaxMind countries","Save your GeoLite2 credentials, then download the licensed Country database. New events use it immediately; previous Unknown locations remain unchanged. Updates run daily."),
        ("smtp","Email delivery","Configure TLS email for invitations, verification and recovery. In development only the selected delivery inbox can receive mail."),
        ("stripe","Stripe billing","Use keys and recurring prices from the same Stripe account and mode. Register the webhook URL below in Stripe. Saving validates configuration; Test verifies the account, prices and portal."),
        ("telemetry","OpenTelemetry","Send traces to your OTLP collector. Exporter changes take effect after restarting the application and worker processes."),
    ] {
        let saved=s.integrations.saved(&s,service).await?;let values=s.integrations.effective(&saved);
        let configured=match service {"maxmind"=>s.geoip_service().available(),"smtp"=>s.mail_service().mode()=="smtp","stripe"=>s.billing_service().provider.is_some(),_=>values.get("OTEL_EXPORTER_OTLP_ENDPOINT").is_some_and(|v|!v.is_empty())};
        body.push_str(&format!("<section class=\"panel settings-panel\"><h2>{title}</h2><p>{detail}</p><p>Status: {} · {}</p><form method=\"post\" action=\"/owner/connections\" class=\"settings-form\"><input type=\"hidden\" name=\"service\" value=\"{service}\"/><p>Leave a field blank to keep its current value. Restore environment settings removes dashboard overrides.</p>",if configured {"Configured"}else{"Not connected"},if saved.is_empty(){"Environment settings"}else{"Dashboard settings"}));
        for field in fields(service) {
            let exists=values.get(field.key).is_some_and(|v|!v.is_empty());
            body.push_str(&format!("<label>{}<input name=\"{}\" type=\"{}\" value=\"{}\" maxlength=\"8192\" autocomplete=\"off\" placeholder=\"{}\"></label>",field.label,field.key,if field.secret {"password"}else{"text"},if field.secret{String::new()}else{esc(values.get(field.key).map(String::as_str).unwrap_or(""))},if exists&&field.secret{"Saved · leave blank to keep"}else{""}));
        }
        body.push_str("<button class=\"btn primary\" name=\"action\" value=\"save\">Save connection</button>");
        if service=="maxmind" {body.push_str("<button class=\"btn outlined\" name=\"action\" value=\"download\">Download / update database</button>");}
        if matches!(service,"smtp"|"stripe") {body.push_str("<button class=\"btn outlined\" name=\"action\" value=\"test\">Test connection</button>");}
        body.push_str("<button class=\"btn outlined\" name=\"action\" value=\"reset\">Restore environment settings</button></form></section>");
    }
    body.push_str(&format!("<section class=\"panel\"><h2>Stripe webhook</h2><p>Endpoint: <code>{}/v1/billing/webhooks/stripe</code>. Subscribe to checkout.session.completed, customer.subscription.created, customer.subscription.updated, customer.subscription.deleted, invoice.paid and invoice.payment_failed.</p></section>",esc(&s.config.public_url)));
    body.push_str("<section class=\"panel settings-panel\"><h2>Vuuut Pro automatic analytics</h2><p>Register your hosting platform once. Every active tenant on its delegated domain connects automatically, including existing tenants. Custom domains require ownership verification.</p><form method=\"post\" action=\"/owner/connections\" class=\"settings-form\"><input type=\"hidden\" name=\"service\" value=\"hosting\"><button type=\"button\" class=\"btn outlined\" data-generate-platform>Generate private credential</button><button type=\"button\" class=\"btn outlined\" data-copy-platform>Copy Vuuut Pro configuration</button><span role=\"status\" data-platform-notice></span><label>Platform ID<input name=\"platform_id\" required maxlength=\"100\"></label><label>Hosting domain suffix<input name=\"domain_suffix\" placeholder=\"sites.example.com\" required></label><label>Platform credential (32–256 characters)<input type=\"password\" name=\"platform_key\" required minlength=\"32\" maxlength=\"256\" autocomplete=\"new-password\"></label><p>Generate a credential and copy the Vuuut Pro configuration before registering. Paste it into the private hosting configuration once. Re-registering rotates platform access; restart hosting to reconcile tenant credentials.</p><button class=\"btn primary\" name=\"action\" value=\"platform\">Register / rotate platform</button></form>");
    let platforms: Vec<(String, String, bool)> =
        sqlx::query_as("SELECT id,domain_suffix,active FROM native_platforms ORDER BY id")
            .fetch_all(&s.store.pg)
            .await?;
    for (id, suffix, active) in platforms {
        body.push_str(&format!(
            "<p>{} · {} · {}</p>",
            esc(&id),
            esc(&suffix),
            if active { "Active" } else { "Disabled" }
        ));
    }
    body.push_str("</section><script src=\"/assets/connections.js\" defer></script><section class=\"panel\"><h2>Storage &amp; operations</h2><p>PostgreSQL and ClickHouse use the deployment's private database settings. Database migration, proxy trust and backup destinations remain deployment settings. <a href=\"/owner/commercial\">View delivery and maintenance status</a>.</p></section></div>");
    let mut response = owner::owner_shell(&h, &user.email, "connections", body).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
    Ok(response)
}

pub async fn action(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(form): Form<Values>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    let actor = owner::authenticated_owner(&s, &h).await?;
    let service = form.get("service").map(String::as_str).unwrap_or("");
    let action = form.get("action").map(String::as_str).unwrap_or("");
    if service == "hosting" && action == "platform" {
        return register_platform(&s, actor.id, &form).await;
    }
    if fields(service).is_empty() {
        return Err(ApiError::bad("Choose a supported service"));
    }
    let message = match action {
        "save" | "reset" => {
            let _guard = s.integrations.gate.lock().await;
            let mut tx = s.store.pg.begin().await?;
            sqlx::query("SELECT pg_advisory_xact_lock(72424015)")
                .execute(&mut *tx)
                .await?;
            let row: Option<Vec<u8>> = sqlx::query_scalar(
                "SELECT ciphertext FROM platform_connections WHERE service=$1 FOR UPDATE",
            )
            .bind(service)
            .fetch_optional(&mut *tx)
            .await?;
            let mut values = row
                .map(|r| s.integrations.unseal(service, &r))
                .transpose()
                .map_err(|_| ApiError::bad("Cannot decrypt connection; restore its key"))?
                .unwrap_or_default();
            if action == "reset" {
                sqlx::query("DELETE FROM platform_connections WHERE service=$1")
                    .bind(service)
                    .execute(&mut *tx)
                    .await?;
            } else {
                for field in fields(service) {
                    if let Some(value) = form.get(field.key).filter(|v| !v.is_empty()) {
                        if value.len() > 8192 || value.contains(['\r', '\n', '\0']) {
                            return Err(ApiError::bad(
                                "Use a single value of at most 8192 characters per field",
                            ));
                        }
                        values.insert(field.key.into(), value.clone());
                    }
                }
                validate(
                    service,
                    &s.integrations.effective(&values),
                    s.config.allow_localhost,
                )?;
                let encrypted = s
                    .integrations
                    .seal(service, &values)
                    .map_err(|_| ApiError::bad("Could not encrypt connection"))?;
                sqlx::query("INSERT INTO platform_connections(service,ciphertext) VALUES($1,$2) ON CONFLICT(service) DO UPDATE SET ciphertext=EXCLUDED.ciphertext,revision=nextval('platform_connection_revision'),updated_at=now()")
                    .bind(service).bind(encrypted).execute(&mut *tx).await?;
            }
            sqlx::query("INSERT INTO platform_audit(actor_id,action) VALUES($1,$2)")
                .bind(actor.id)
                .bind(format!("connection.{service}.{action}"))
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            drop(_guard);
            s.integrations.refresh(&s).await.map_err(|_|ApiError::bad("Connection saved but could not be activated; check configuration and encryption key"))?;
            if service == "telemetry" {
                "Saved. Restart application and workers to apply the exporter settings."
            } else {
                "Connection settings applied."
            }
        }
        "download" if service == "maxmind" => {
            update_geoip(&s).await?;
            "MaxMind database validated and activated for new events."
        }
        "test" if service == "smtp" => {
            // SMTP protocol test only: do not send unsolicited mail from settings.
            s.mail_service().test_connection().await.map_err(|_| {
                ApiError::bad(
                    "SMTP connection or authentication failed; check host, port and credentials",
                )
            })?;
            "SMTP accepted the TLS connection and authentication."
        }
        "test" if service == "stripe" => {
            let provider = s
                .billing_service()
                .provider
                .clone()
                .ok_or_else(|| ApiError::bad("Configure Stripe first"))?;
            provider.validate_catalog().await?;
            "Stripe account, price and portal checks completed. See Commercial status for delivery."
        }
        _ => return Err(ApiError::bad("Choose an available connection action")),
    };
    let mut url = url::Url::parse("http://localhost/owner/connections").expect("constant URL");
    url.query_pairs_mut().append_pair("message", message);
    Ok(Redirect::to(&format!(
        "/owner/connections?{}",
        url.query().unwrap_or_default()
    ))
    .into_response())
}
fn validate(service: &str, values: &Values, local: bool) -> AppResult<()> {
    let get = |key: &str| values.get(key).map(String::as_str).unwrap_or("");
    match service {
        "smtp" => {
            if get("SMTP_HOST").is_empty()
                || get("SMTP_FROM").is_empty()
                || !matches!(get("SMTP_PORT"), "" | "465" | "587")
            {
                return Err(ApiError::bad(
                    "Enter SMTP host, sender and TLS port 465 or 587",
                ));
            }
            mail::Mailer::from_config(local, values).map_err(|_| {
                ApiError::bad("Invalid SMTP settings; development also needs a delivery inbox")
            })?;
        }
        "stripe" => {
            billing::Service::from_config(values).map_err(|_|ApiError::bad("Use matching Stripe secret, webhook secret, price mapping and environment mode"))?;
            if get("STRIPE_SECRET_KEY").is_empty() {
                return Err(ApiError::bad("Enter Stripe credentials"));
            }
        }
        "maxmind" => {
            if get("MAXMIND_ACCOUNT_ID").is_empty()
                || !get("MAXMIND_ACCOUNT_ID")
                    .bytes()
                    .all(|c| c.is_ascii_digit())
                || get("MAXMIND_LICENSE_KEY").len() < 8
            {
                return Err(ApiError::bad(
                    "Enter your numeric MaxMind account ID and license key",
                ));
            }
        }
        "telemetry" => {
            let u = url::Url::parse(get("OTEL_EXPORTER_OTLP_ENDPOINT"))
                .map_err(|_| ApiError::bad("Enter an OTLP HTTPS endpoint"))?;
            if u.scheme() != "https"
                || u.host_str().is_none()
                || !u.username().is_empty()
                || u.password().is_some()
                || u.query().is_some()
                || u.fragment().is_some()
            {
                return Err(ApiError::bad(
                    "Use an OTLP HTTPS endpoint without credentials or query parameters",
                ));
            }
        }
        _ => return Err(ApiError::bad("Unsupported service")),
    }
    Ok(())
}
async fn register_platform(s: &AppState, actor: Uuid, form: &Values) -> AppResult<Response> {
    let get = |key: &str| form.get(key).map(String::as_str).unwrap_or("");
    let id = get("platform_id");
    let suffix = get("domain_suffix").trim().to_ascii_lowercase();
    let token = get("platform_key");
    if id.is_empty()
        || id.len() > 100
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        || !(32..=256).contains(&token.len())
        || token.bytes().any(|b| b.is_ascii_control())
        || suffix.len() > 253
        || !suffix.contains('.')
        || suffix.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err(ApiError::bad("Use a platform ID, exact hosting domain suffix and 32–256 character private credential"));
    }
    let mut tx = s.store.pg.begin().await?;
    sqlx::query("INSERT INTO native_platforms(id,token_hash,domain_suffix) VALUES($1,$2,$3) ON CONFLICT(id) DO UPDATE SET token_hash=EXCLUDED.token_hash,domain_suffix=EXCLUDED.domain_suffix,active=true").bind(id).bind(click_core::digest(token)).bind(suffix).execute(&mut *tx).await?;
    sqlx::query("UPDATE api_keys SET revoked_at=now() WHERE website_id IN (SELECT website_id FROM integration_mappings WHERE platform_id=$1) AND revoked_at IS NULL").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM native_sessions WHERE website_id IN (SELECT website_id FROM integration_mappings WHERE platform_id=$1)").bind(id).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO platform_audit(actor_id,action) VALUES($1,'connection.hosting.register')",
    )
    .bind(actor)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/connections?message=Hosting+platform+registered.+Configure+Vuuut+Pro+and+restart+hosting+to+reconcile+tenants.").into_response())
}
/// Download the official licensed archive; no user-supplied URL or archive path.
pub async fn update_geoip(s: &AppState) -> AppResult<()> {
    let _guard = s.integrations.gate.lock().await;
    let values = s
        .integrations
        .effective(&s.integrations.saved(s, "maxmind").await?);
    validate("maxmind", &values, s.config.allow_localhost)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let u = attempt.url();
            if attempt.previous().len() >= 3
                || u.scheme() != "https"
                || !u.host_str().is_some_and(|h| {
                    h == "download.maxmind.com" || h.ends_with(".r2.cloudflarestorage.com")
                })
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| ApiError::bad("Could not initialize MaxMind download"))?;
    let mut response = client
        .get("https://download.maxmind.com/geoip/databases/GeoLite2-Country/download?suffix=tar.gz")
        .basic_auth(
            &values["MAXMIND_ACCOUNT_ID"],
            Some(&values["MAXMIND_LICENSE_KEY"]),
        )
        .send()
        .await
        .map_err(|_| ApiError::bad("MaxMind download unavailable; try again later"))?;
    if !response.status().is_success() {
        return Err(ApiError::bad(
            "MaxMind refused the download; check license and database access",
        ));
    }
    const LIMIT: usize = 64 * 1024 * 1024;
    if response.content_length().is_some_and(|n| n > LIMIT as u64) {
        return Err(ApiError::bad("MaxMind archive exceeds download limit"));
    }
    let mut archive = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::bad("MaxMind download interrupted"))?
    {
        if archive.len() + chunk.len() > LIMIT {
            return Err(ApiError::bad("MaxMind archive exceeds download limit"));
        }
        archive.extend_from_slice(&chunk);
    }
    let data = s.integrations.data.clone();
    let path = tokio::task::spawn_blocking(move || -> anyhow::Result<PathBuf> {
        use std::io::{Read, Write};
        let decoder = flate2::read::GzDecoder::new(archive.as_slice());
        let mut tar = tar::Archive::new(decoder.take((LIMIT * 2) as u64));
        let mut mmdb = None;
        for entry in tar.entries()? {
            let mut entry = entry?;
            if entry.header().entry_type().is_file()
                && entry.path()?.file_name() == Some(std::ffi::OsStr::new("GeoLite2-Country.mmdb"))
            {
                anyhow::ensure!(entry.size() <= LIMIT as u64, "GeoIP database too large");
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                mmdb = Some(bytes);
                break;
            }
        }
        let bytes = mmdb.ok_or_else(|| anyhow::anyhow!("Country database absent"))?;
        private_dir(&data)?;
        let temp = data.join(format!("{}.mmdb", Uuid::new_v4()));
        let result = (|| -> anyhow::Result<PathBuf> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            let reader = GeoIp::open(Some(&temp))?;
            anyhow::ensure!(
                reader.database_type() == Some("GeoLite2-Country"),
                "Unexpected database type"
            );
            let path = data.join("GeoLite2-Country.mmdb");
            std::fs::rename(&temp, &path)?;
            Ok(path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    })
    .await
    .map_err(|_| ApiError::bad("MaxMind database validation failed"))?
    .map_err(|_| ApiError::bad("Could not validate or save MaxMind Country database"))?;
    let reader = GeoIp::open(Some(&path))
        .map_err(|_| ApiError::bad("Could not activate MaxMind database"))?;
    s.integrations.live.write().expect("connections lock").geoip = Some(Arc::new(reader));
    Ok(())
}
/// All serving and worker processes reload saved service revisions. GeoIP data
/// is refreshed daily by one cluster-wide lease, retaining the last good file.
pub async fn worker(s: AppState, mut stop: tokio::sync::watch::Receiver<bool>) {
    let mut tick = 0u64;
    loop {
        if *stop.borrow() {
            break;
        }
        if s.integrations.refresh(&s).await.is_err() {
            tracing::warn!("Saved service configuration could not be refreshed");
        }
        if tick.is_multiple_of(720) {
            let configured: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM platform_connections WHERE service='maxmind')",
            )
            .fetch_one(&s.store.pg)
            .await
            .unwrap_or(false);
            if configured {
                if let Ok(mut connection) = s.store.pg.acquire().await {
                    let lease: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(72424016)")
                        .fetch_one(&mut *connection)
                        .await
                        .unwrap_or(false);
                    if lease {
                        let fresh = s
                            .integrations
                            .geo_path(&s)
                            .and_then(|p| std::fs::metadata(p).ok())
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.elapsed().ok())
                            .is_some_and(|age| age.as_secs() < 86400);
                        if !fresh {
                            let started = chrono::Utc::now();
                            let ok = update_geoip(&s).await.is_ok();
                            let _ =
                                crate::commercial::record(&s.store, "maxmind", started, ok).await;
                        }
                        let _ = sqlx::query("SELECT pg_advisory_unlock(72424016)")
                            .execute(&mut *connection)
                            .await;
                    }
                }
            }
        }
        tick += 1;
        tokio::select! {_=stop.changed()=>{},_=tokio::time::sleep(std::time::Duration::from_secs(5))=>{}}
    }
}

pub async fn script() -> Response {
    ([(header::CONTENT_TYPE,"application/javascript; charset=utf-8")],r#"document.querySelector('[data-generate-platform]')?.addEventListener('click',event=>{const form=event.target.form;const bytes=crypto.getRandomValues(new Uint8Array(32));form.elements.platform_key.value=Array.from(bytes,b=>b.toString(16).padStart(2,'0')).join('');form.querySelector('[data-platform-notice]').textContent='Credential generated. Copy the hosting configuration before registering.'});document.querySelector('[data-copy-platform]')?.addEventListener('click',async event=>{const form=event.target.form;const notice=form.querySelector('[data-platform-notice]');const key=form.elements.platform_key.value;if(key.length<32){notice.textContent='Enter or generate a private credential first.';return}try{await navigator.clipboard.writeText(JSON.stringify({click:{baseUrl:location.origin,platformKey:key,defaultPlan:'free',consentRequired:true,engagement:false,persistentIdentity:false}},null,2));notice.textContent='Copied. Save in Vuuut Pro configuration and register this platform.'}catch{notice.textContent='Clipboard unavailable. Use this server URL and credential in the private Vuuut Pro Click configuration.'}});"#).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_authenticated_and_bound_to_their_service() {
        let c = Connections::with_key([42; 32], PathBuf::new());
        let values = Values::from([("SMTP_PASSWORD".into(), "private-provider-password".into())]);
        let mut encrypted = c.seal("smtp", &values).unwrap();
        assert!(!encrypted.windows(8).any(|b| b == b"private-"));
        assert_eq!(c.unseal("smtp", &encrypted).unwrap(), values);
        assert!(c.unseal("stripe", &encrypted).is_err());
        assert!(Connections::with_key([41; 32], PathBuf::new())
            .unseal("smtp", &encrypted)
            .is_err());
        encrypted[15] ^= 1;
        assert!(c.unseal("smtp", &encrypted).is_err());
    }
}
