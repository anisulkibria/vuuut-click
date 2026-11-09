use crate::{ApiError, AppResult, AppState};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    Json,
};
use click_core::{digest, Role};
use click_store::Website;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Serialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
}
#[derive(Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
    pub organization: Option<String>,
}

pub fn secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (k, v) = part.trim().split_once('=')?;
            (k == name).then(|| v.to_owned())
        })
}
pub fn csrf(state: &AppState, headers: &HeaderMap) -> AppResult<()> {
    if headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        != Some(state.config.public_url.as_str())
    {
        return Err(ApiError::forbidden("Request origin is not allowed"));
    }
    Ok(())
}
pub fn session_response(state: &AppState, token: &str, user: User) -> Response {
    let secure = if state.config.secure_cookie {
        "; Secure"
    } else {
        ""
    };
    (
        [(
            header::SET_COOKIE,
            format!(
                "click_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=604800{secure}"
            ),
        )],
        Json(user),
    )
        .into_response()
}
pub async fn user(state: &AppState, headers: &HeaderMap) -> AppResult<User> {
    let token = cookie_value(headers, "click_session").ok_or_else(ApiError::unauthorized)?;
    sqlx::query_as("SELECT u.id,u.email FROM users u JOIN sessions s ON s.user_id=u.id WHERE s.token_hash=$1 AND s.expires_at>now() AND s.audience='user' AND NOT EXISTS (SELECT 1 FROM platform_owners p WHERE p.user_id=u.id)")
        .bind(digest(&token)).fetch_optional(&state.store.pg).await?.ok_or_else(ApiError::unauthorized)
}
pub async fn register(state: &AppState, body: Credentials) -> AppResult<(String, User)> {
    if !state.config.allow_localhost && !state.mail.available() {
        return Err(ApiError {
            status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
            message: "Account email delivery is not configured",
        });
    }
    let email = body.email.trim().to_lowercase();
    let organization = body.organization.unwrap_or_else(|| "My workspace".into());
    if email.parse::<lettre::Address>().is_err()
        || !email.contains('@')
        || email.len() > 254
        || email.chars().any(char::is_whitespace)
        || body.password.len() < 12
        || body.password.len() > 256
        || organization.trim().is_empty()
        || organization.len() > 100
    {
        return Err(ApiError::bad("Use a valid email, a 12–256 character password and a workspace name up to 100 characters"));
    }
    let _permit = state
        .password_slots
        .acquire()
        .await
        .map_err(ApiError::internal)?;
    let hash = tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(body.password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|h| h.to_string())
            .map_err(|_| anyhow::anyhow!("password hashing failed"))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    let user = User {
        id: Uuid::new_v4(),
        email,
    };
    let org = Uuid::new_v4();
    let token = secret();
    let mut tx = state.store.pg.begin().await?;
    let result = sqlx::query("INSERT INTO users(id,email,password_hash) VALUES($1,$2,$3)")
        .bind(user.id)
        .bind(&user.email)
        .bind(hash)
        .execute(&mut *tx)
        .await;
    if let Err(e) = result {
        if e.as_database_error()
            .is_some_and(|e| e.is_unique_violation())
        {
            return Err(ApiError::conflict(
                "Account cannot be created with these details",
            ));
        }
        return Err(e.into());
    }
    sqlx::query("INSERT INTO organizations(id,name) VALUES($1,$2)")
        .bind(org)
        .bind(organization.trim())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO memberships(organization_id,user_id,role) VALUES($1,$2,'owner')")
        .bind(org)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO subscriptions(organization_id,plan_id) VALUES($1,'free')")
        .bind(org)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '7 days')",
    )
    .bind(digest(&token))
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    if !state.config.allow_localhost {
        crate::mail::enqueue_account_token(
            &mut tx,
            user.id,
            &user.email,
            "verify",
            &state.config.public_url,
        )
        .await?;
    }
    tx.commit().await?;
    Ok((token, user))
}
pub async fn login(state: &AppState, body: Credentials) -> AppResult<(String, User)> {
    let mut tx = state.store.pg.begin().await?;
    let user = verify_locked_credentials(state, &mut tx, body).await?;
    let is_owner: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_owners WHERE user_id=$1)")
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?;
    if is_owner {
        return Err(ApiError::forbidden(
            "Use the owner sign-in at /owner/signin",
        ));
    }
    let token = secret();
    sqlx::query(
        "INSERT INTO sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '7 days')",
    )
    .bind(digest(&token))
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((token, user))
}

pub(crate) async fn hash_password(state: &AppState, password: String) -> AppResult<String> {
    if !(12..=256).contains(&password.len()) {
        return Err(ApiError::bad("Password must contain 12–256 characters"));
    }
    let _permit = state
        .password_slots
        .acquire()
        .await
        .map_err(ApiError::internal)?;
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|h| h.to_string())
            .map_err(|_| anyhow::anyhow!("Password hashing failed"))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)
}

pub(crate) async fn verify_locked_credentials(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    body: Credentials,
) -> AppResult<User> {
    if body.email.len() > 254 || body.password.len() > 256 {
        return Err(ApiError::unauthorized());
    }
    let row: Option<(Uuid, String, String)> =
        sqlx::query_as("SELECT id,email,password_hash FROM users WHERE email=$1 FOR UPDATE")
            .bind(body.email.trim().to_lowercase())
            .fetch_optional(tx)
            .await?;
    let (id, email, hash) = row.unwrap_or((
        Uuid::nil(),
        String::new(),
        state.dummy_password_hash.to_string(),
    ));
    if !verify_against_hash(state, hash, body.password).await? || id.is_nil() {
        return Err(ApiError::unauthorized());
    }
    Ok(User { id, email })
}

/// Password verification alone never creates a session or grants an authentication audience.
/// Verify a password against a hash already read from the database.
///
/// Exists so a caller that has the row — and, crucially, has it locked — can
/// prove the password *at write time* rather than trusting a check made
/// before the transaction opened. See `manage::change_password`.
pub(crate) async fn verify_against_hash(
    state: &AppState,
    hash: String,
    password: String,
) -> AppResult<bool> {
    if password.len() > 256 {
        return Ok(false);
    }
    let _permit = state
        .password_slots
        .acquire()
        .await
        .map_err(ApiError::internal)?;
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash).is_ok_and(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
    })
    .await
    .map_err(ApiError::internal)
}

/// Serialize customer account writes with reset, login and owner promotion.
pub(crate) async fn lock_customer(
    tx: &mut sqlx::PgConnection,
    h: &HeaderMap,
    user: Uuid,
) -> AppResult<String> {
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
    let token = cookie_value(h, "click_session").ok_or_else(ApiError::unauthorized)?;
    let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE user_id=$1 AND token_hash=$2 AND audience='user' AND expires_at>now() AND NOT EXISTS(SELECT 1 FROM platform_owners WHERE user_id=$1))").bind(user).bind(digest(&token)).fetch_one(tx).await?;
    if !valid {
        return Err(ApiError::unauthorized());
    }
    Ok(hash)
}

/// Authorize a read *and* take the workspace's analytical-query slot.
///
/// The entry point for every report that queries ClickHouse, so one workspace
/// cannot occupy the whole process's query budget — see
/// [`crate::TenantLoad`]. The returned permit must be held for as long as the
/// query runs, which is what binding it for the body of the handler does.
pub async fn authorize_report(
    state: &AppState,
    headers: &HeaderMap,
    site_id: Uuid,
) -> AppResult<(Website, Role, crate::operations::ReportPermit)> {
    let (site, role) = authorize_site(state, headers, site_id, false).await?;
    let permit = state
        .tenant_queries
        .try_acquire(site.organization_id)
        .ok_or(ApiError {
            status: axum::http::StatusCode::SERVICE_UNAVAILABLE,
            message:
                "This workspace already has several reports running. Please try again in a moment.",
        })?;
    let permit = state
        .operations
        .report(&state.store, site.organization_id, permit)
        .await?;
    Ok((site, role, permit))
}

pub async fn authorize_site(
    state: &AppState,
    headers: &HeaderMap,
    site_id: Uuid,
    manage: bool,
) -> AppResult<(Website, Role)> {
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    {
        if manage {
            return Err(ApiError::not_found());
        }
        if let Some(site) = crate::native::authorized_site(state, token, site_id).await? {
            return Ok((site, Role::Viewer));
        }
        let site: Option<Website> = sqlx::query_as("SELECT w.id,w.organization_id,w.name,w.origin,w.collection_id,w.verification_token,w.verified_at,w.active,w.is_demo FROM websites w JOIN api_keys k ON k.website_id=w.id WHERE w.id=$1 AND k.token_hash=$2 AND k.revoked_at IS NULL AND k.scope='read' AND w.deletion_requested_at IS NULL")
            .bind(site_id).bind(digest(token)).fetch_optional(&state.store.pg).await?;
        return site
            .map(|s| (s, Role::Viewer))
            .ok_or_else(ApiError::not_found);
    }
    let u = user(state, headers).await?;
    let role: Option<String> = sqlx::query_scalar("SELECT m.role FROM memberships m JOIN websites w ON w.organization_id=m.organization_id WHERE w.id=$1 AND m.user_id=$2 AND w.deletion_requested_at IS NULL").bind(site_id).bind(u.id).fetch_optional(&state.store.pg).await?;
    let role = role
        .as_deref()
        .and_then(Role::parse)
        .ok_or_else(ApiError::not_found)?;
    if manage && !role.can_manage() {
        return Err(ApiError::not_found());
    }
    let site = sqlx::query_as("SELECT id,organization_id,name,origin,collection_id,verification_token,verified_at,active,is_demo FROM websites WHERE id=$1").bind(site_id).fetch_one(&state.store.pg).await?;
    Ok((site, role))
}
