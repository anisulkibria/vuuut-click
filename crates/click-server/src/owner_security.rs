//! Owner account management. All security writes serialize on the user row.
use crate::{
    auth,
    dashboard::{document, esc},
    owner, ApiError, AppResult, AppState,
};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use click_core::{digest, totp};
use rand::RngCore;
use serde::Deserialize;
use uuid::Uuid;

pub(crate) fn cookie(s: &AppState, token: &str, pending: bool) -> String {
    let name = if pending {
        "click_owner_pending"
    } else {
        "click_owner_session"
    };
    let ttl = if pending { 300 } else { 28800 };
    let secure = if s.config.secure_cookie {
        "; Secure"
    } else {
        ""
    };
    format!("{name}={token}; Path=/owner; HttpOnly; SameSite=Strict; Max-Age={ttl}{secure}")
}
fn page(h: &HeaderMap, title: &str, body: &str) -> Html<String> {
    document(
        title,
        h,
        &format!(
            r#"<main id="main" class="auth-main"><section class="auth-card"><a href="/owner/signin">← Owner sign in</a><h2>{}</h2>{body}</section></main>"#,
            esc(title)
        ),
    )
}
fn password_field() -> &'static str {
    r#"<label>Current password<input type="password" name="current_password" autocomplete="current-password" required maxlength="256"></label>"#
}
fn factor_field() -> &'static str {
    r#"<label>Authenticator or recovery code<input name="code" autocomplete="one-time-code" required maxlength="64"></label>"#
}
fn form(action: &str, fields: &str, label: &str) -> String {
    format!(
        r#"<form action="/owner/account/action" method="post" class="stack"><input type="hidden" name="action" value="{action}">{fields}<button class="btn primary">{label}</button></form>"#
    )
}

/// Re-read the session and grant after taking the same lock as password reset.
pub(crate) async fn lock_owner(
    h: &HeaderMap,
    tx: &mut sqlx::PgConnection,
) -> AppResult<(auth::User, String)> {
    let token = auth::cookie_value(h, "click_owner_session").ok_or_else(ApiError::unauthorized)?;
    let user:auth::User=sqlx::query_as("SELECT u.id,u.email FROM users u JOIN sessions s ON s.user_id=u.id JOIN platform_owners p ON p.user_id=u.id WHERE s.token_hash=$1 AND s.audience='owner' AND s.expires_at>now()")
        .bind(digest(&token)).fetch_optional(&mut *tx).await?.ok_or_else(ApiError::unauthorized)?;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1 FOR UPDATE")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions s JOIN platform_owners p ON p.user_id=s.user_id WHERE s.user_id=$1 AND s.token_hash=$2 AND s.audience='owner' AND s.expires_at>now())").bind(user.id).bind(digest(&token)).fetch_one(&mut *tx).await?;
    if !valid {
        return Err(ApiError::unauthorized());
    }
    Ok((user, hash))
}
async fn audit(tx: &mut sqlx::PgConnection, user: Uuid, action: &str) -> AppResult<()> {
    sqlx::query("INSERT INTO platform_audit(actor_id,action,resource_id) VALUES($1,$2,$1)")
        .bind(user)
        .bind(action)
        .execute(tx)
        .await?;
    Ok(())
}

/// Consumes a step or recovery code. Caller must hold the user row lock.
async fn factor(tx: &mut sqlx::PgConnection, user: Uuid, code: &str) -> AppResult<bool> {
    type Security = (
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<i64>,
    );
    let security: Option<Security> = sqlx::query_as(
        "SELECT totp_secret,confirmed_at,last_step FROM owner_security WHERE user_id=$1",
    )
    .bind(user)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((secret, confirmed, last)) = security else {
        return Ok(true);
    };
    if confirmed.is_none() {
        return Ok(true);
    }
    let Some(secret) = secret else {
        return Ok(false);
    };
    if let Some(bytes) = totp::decode_secret(&secret) {
        if let Some(step) = totp::matching_step(&bytes, code, chrono::Utc::now().timestamp() as u64)
        {
            if last.is_none_or(|last| step as i64 > last) {
                sqlx::query("UPDATE owner_security SET last_step=$2 WHERE user_id=$1")
                    .bind(user)
                    .bind(step as i64)
                    .execute(&mut *tx)
                    .await?;
                return Ok(true);
            }
        }
    }
    let consumed = sqlx::query("UPDATE owner_recovery_codes SET used_at=now() WHERE user_id=$1 AND code_hash=$2 AND used_at IS NULL").bind(user).bind(digest(code.trim())).execute(tx).await?;
    Ok(consumed.rows_affected() == 1)
}
async fn recovery_codes(tx: &mut sqlx::PgConnection, user: Uuid) -> AppResult<Vec<String>> {
    sqlx::query("DELETE FROM owner_recovery_codes WHERE user_id=$1")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    let mut codes = Vec::new();
    for _ in 0..10 {
        let code = auth::secret()[..24].to_string();
        sqlx::query("INSERT INTO owner_recovery_codes(code_hash,user_id) VALUES($1,$2)")
            .bind(digest(&code))
            .bind(user)
            .execute(&mut *tx)
            .await?;
        codes.push(code);
    }
    Ok(codes)
}
fn codes_page(h: &HeaderMap, codes: Vec<String>) -> Html<String> {
    let codes = codes
        .iter()
        .map(|c| format!("<li><code>{}</code></li>", esc(c)))
        .collect::<String>();
    page(
        h,
        "Save your recovery codes",
        &format!(
            r#"<p>Two-factor authentication is enabled. Store these codes somewhere private. Each works once, and they are shown only now. New codes replace all previous codes.</p><ul id="recovery-codes">{codes}</ul><a href="/owner/account">Return to your account</a>"#
        ),
    )
}

pub async fn account(State(s): State<AppState>, h: HeaderMap) -> AppResult<Html<String>> {
    let user = owner::authenticated_owner(&s, &h).await?;
    let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id=$1")
        .bind(user.id)
        .fetch_one(&s.store.pg)
        .await?;
    let enabled: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM owner_security WHERE user_id=$1 AND confirmed_at IS NOT NULL)",
    )
    .bind(user.id)
    .fetch_one(&s.store.pg)
    .await?;
    let profile = form(
        "profile",
        &format!(
            r#"<label>Display name<input name="display_name" maxlength="100" value="{}"></label>"#,
            esc(&name)
        ),
        "Save profile",
    );
    let factor_input = if enabled { factor_field() } else { "" };
    let password = form(
        "password",
        &format!(
            r#"{}{}<label>New password<input type="password" name="new_password" autocomplete="new-password" minlength="12" maxlength="256" required></label>"#,
            password_field(),
            factor_input
        ),
        "Change password",
    );
    let mfa = if enabled {
        format!(
            "<p>Two-factor authentication is enabled. Codes cannot be reused.</p>{}{}",
            form(
                "codes",
                &(password_field().to_owned() + factor_field()),
                "Replace recovery codes"
            ),
            form(
                "disable",
                &(password_field().to_owned() + factor_field()),
                "Disable two-factor authentication"
            )
        )
    } else {
        format!(
            "<p>Add an authenticator app to protect owner sign-in.</p>{}",
            form(
                "enroll",
                password_field(),
                "Set up two-factor authentication"
            )
        )
    };
    let sessions = form(
        "sessions",
        password_field(),
        "Sign out other owner sessions",
    );
    let body = format!(
        r#"<header class="topbar"><div class="breadcrumb">Your platform <span>/</span> Account &amp; security</div></header>
        <div class="page owner-account"><header class="page-heading"><div><span class="eyebrow">OWNER ACCOUNT</span><h1>Your account and security</h1><p>{email}</p></div></header>
        <section class="panel"><h2>Profile</h2>{profile}</section>
        <section class="panel"><h2>Password</h2>{password}</section>
        <section class="panel"><h2>Two-factor authentication</h2>{mfa}</section>
        <section class="panel"><h2>Sessions</h2>{sessions}</section></div>"#,
        email = esc(&user.email)
    );
    Ok(owner::owner_shell(&h, &user.email, "account", body))
}
#[derive(Deserialize)]
pub struct AccountAction {
    action: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    current_password: String,
    #[serde(default)]
    new_password: String,
    #[serde(default)]
    code: String,
}
pub async fn account_action(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<AccountAction>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    let mut tx = s.store.pg.begin().await?;
    let (user, hash) = lock_owner(&h, &mut tx).await?;
    if input.action == "profile" {
        if input.display_name.len() > 100 {
            return Err(ApiError::bad("Name must contain at most 100 characters"));
        }
        sqlx::query("UPDATE users SET display_name=$2 WHERE id=$1")
            .bind(user.id)
            .bind(input.display_name.trim())
            .execute(&mut *tx)
            .await?;
    } else {
        if !auth::verify_against_hash(&s, hash, input.current_password).await? {
            return Err(ApiError::unauthorized());
        }
        match input.action.as_str() {
            "enroll" => {
                let enabled:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM owner_security WHERE user_id=$1 AND confirmed_at IS NOT NULL)").bind(user.id).fetch_one(&mut *tx).await?;
                if enabled {
                    return Err(ApiError::conflict(
                        "Two-factor authentication is already enabled",
                    ));
                }
                let mut bytes = [0u8; 20];
                rand::rngs::OsRng.fill_bytes(&mut bytes);
                let secret = totp::encode_secret(&bytes);
                sqlx::query("INSERT INTO owner_security(user_id,totp_secret) VALUES($1,$2) ON CONFLICT(user_id) DO UPDATE SET totp_secret=$2,updated_at=now(),last_step=NULL").bind(user.id).bind(&secret).execute(&mut *tx).await?;
                audit(&mut tx, user.id, "owner.mfa.enrollment_started").await?;
                tx.commit().await?;
                let uri = totp::provisioning_uri("Vuuut Click", &user.email, &secret);
                return Ok(page(&h,"Connect your authenticator",&format!(r#"<p>Add a time-based account to your authenticator app with this setup key:</p><p><code id="totp-secret">{secret}</code></p><p>Issuer: Vuuut Click · Account: {email} · SHA1 · 6 digits · 30 seconds</p><details><summary>Authenticator setup URI</summary><code>{uri}</code></details><p>Confirm a code to finish. Setup alone does not enable two-factor authentication.</p>{confirm}"#,email=esc(&user.email),uri=esc(&uri),confirm=form("confirm",&(password_field().to_owned()+factor_field()),"Enable two-factor authentication"))).into_response());
            }
            "confirm" => {
                let secret:Option<String>=sqlx::query_scalar("SELECT totp_secret FROM owner_security WHERE user_id=$1 AND confirmed_at IS NULL AND updated_at>now()-interval '15 minutes'").bind(user.id).fetch_optional(&mut *tx).await?.flatten();
                let step = secret
                    .and_then(|v| totp::decode_secret(&v))
                    .and_then(|v| {
                        totp::matching_step(&v, &input.code, chrono::Utc::now().timestamp() as u64)
                    })
                    .ok_or_else(|| {
                        ApiError::bad(
                            "The code is invalid or setup expired. Start setup again if needed.",
                        )
                    })?;
                sqlx::query(
                    "UPDATE owner_security SET confirmed_at=now(),last_step=$2 WHERE user_id=$1",
                )
                .bind(user.id)
                .bind(step as i64)
                .execute(&mut *tx)
                .await?;
                let codes = recovery_codes(&mut tx, user.id).await?;
                revoke_other(&mut tx, &h, user.id).await?;
                audit(&mut tx, user.id, "owner.mfa.enabled").await?;
                tx.commit().await?;
                return Ok(codes_page(&h, codes).into_response());
            }
            "password" | "codes" | "disable" => {
                if input.action != "password" {
                    let enabled:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM owner_security WHERE user_id=$1 AND confirmed_at IS NOT NULL)").bind(user.id).fetch_one(&mut *tx).await?;
                    if !enabled {
                        return Err(ApiError::bad("Enable two-factor authentication first"));
                    }
                }
                if !factor(&mut tx, user.id, &input.code).await? {
                    return Err(ApiError::bad(
                        "Use a fresh authenticator code or an unused recovery code",
                    ));
                }
                match input.action.as_str() {
                    "password" => {
                        let hash = auth::hash_password(&s, input.new_password).await?;
                        sqlx::query("UPDATE users SET password_hash=$2 WHERE id=$1")
                            .bind(user.id)
                            .bind(hash)
                            .execute(&mut *tx)
                            .await?;
                        sqlx::query("UPDATE account_tokens SET used_at=now() WHERE user_id=$1 AND used_at IS NULL").bind(user.id).execute(&mut *tx).await?;
                        sqlx::query("DELETE FROM sessions WHERE user_id=$1")
                            .bind(user.id)
                            .execute(&mut *tx)
                            .await?;
                        let token = auth::secret();
                        sqlx::query("INSERT INTO sessions(token_hash,user_id,audience,expires_at) VALUES($1,$2,'owner',now()+interval '8 hours')").bind(digest(&token)).bind(user.id).execute(&mut *tx).await?;
                        audit(&mut tx, user.id, "owner.password.changed").await?;
                        tx.commit().await?;
                        return Ok((
                            [(header::SET_COOKIE, cookie(&s, &token, false))],
                            Redirect::to("/owner/account"),
                        )
                            .into_response());
                    }
                    "codes" => {
                        let codes = recovery_codes(&mut tx, user.id).await?;
                        revoke_other(&mut tx, &h, user.id).await?;
                        audit(&mut tx, user.id, "owner.recovery_codes.replaced").await?;
                        tx.commit().await?;
                        return Ok(codes_page(&h, codes).into_response());
                    }
                    _ => {
                        sqlx::query("DELETE FROM owner_security WHERE user_id=$1")
                            .bind(user.id)
                            .execute(&mut *tx)
                            .await?;
                        sqlx::query("DELETE FROM owner_recovery_codes WHERE user_id=$1")
                            .bind(user.id)
                            .execute(&mut *tx)
                            .await?;
                        revoke_other(&mut tx, &h, user.id).await?;
                    }
                }
            }
            "sessions" => {
                revoke_other(&mut tx, &h, user.id).await?;
            }
            _ => return Err(ApiError::bad("Unknown account action")),
        }
    }
    audit(&mut tx, user.id, &format!("owner.account.{}", input.action)).await?;
    tx.commit().await?;
    Ok(Redirect::to("/owner/account").into_response())
}
async fn revoke_other(tx: &mut sqlx::PgConnection, h: &HeaderMap, user: Uuid) -> AppResult<()> {
    let current =
        auth::cookie_value(h, "click_owner_session").ok_or_else(ApiError::unauthorized)?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND token_hash<>$2")
        .bind(user)
        .bind(digest(&current))
        .execute(tx)
        .await?;
    Ok(())
}

pub async fn challenge(h: HeaderMap) -> Html<String> {
    page(
        &h,
        "Verify your sign-in",
        &format!(
            r#"<p>Enter an authenticator code or one of your saved recovery codes. This sign-in expires in five minutes.</p><form action="/owner/auth/verify" method="post" class="stack">{}<button class="btn primary">Verify sign-in</button></form>"#,
            factor_field()
        ),
    )
}
#[derive(Deserialize)]
pub struct CodeInput {
    code: String,
}
pub async fn verify(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<CodeInput>,
) -> AppResult<Response> {
    auth::csrf(&s, &h)?;
    let pending =
        auth::cookie_value(&h, "click_owner_pending").ok_or_else(ApiError::unauthorized)?;
    let mut tx = s.store.pg.begin().await?;
    let user: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM sessions WHERE token_hash=$1 AND audience='owner_pending'",
    )
    .bind(digest(&pending))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::unauthorized)?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
    let attempts:Option<i32>=sqlx::query_scalar("UPDATE sessions SET factor_attempts=factor_attempts+1 WHERE token_hash=$1 AND audience='owner_pending' AND expires_at>now() AND factor_attempts<5 AND EXISTS(SELECT 1 FROM platform_owners WHERE user_id=$2) RETURNING factor_attempts").bind(digest(&pending)).bind(user).fetch_optional(&mut *tx).await?;
    if attempts.is_none() {
        return Err(ApiError::unauthorized());
    }
    if !factor(&mut tx, user, &input.code).await? {
        tx.commit().await?;
        return Err(ApiError::bad(
            "Use a fresh authenticator code or an unused recovery code",
        ));
    }
    sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
        .bind(digest(&pending))
        .execute(&mut *tx)
        .await?;
    let token = auth::secret();
    sqlx::query("INSERT INTO sessions(token_hash,user_id,audience,expires_at) VALUES($1,$2,'owner',now()+interval '8 hours')").bind(digest(&token)).bind(user).execute(&mut *tx).await?;
    audit(&mut tx, user, "owner.signin.second_factor").await?;
    tx.commit().await?;
    let mut headers = HeaderMap::new();
    headers.append(
        header::SET_COOKIE,
        cookie(&s, &token, false).parse().unwrap(),
    );
    headers.append(
        header::SET_COOKIE,
        format!(
            "click_owner_pending=; Path=/owner; HttpOnly; SameSite=Strict; Max-Age=0{}",
            if s.config.secure_cookie {
                "; Secure"
            } else {
                ""
            }
        )
        .parse()
        .unwrap(),
    );
    Ok((headers, Redirect::to("/owner/dashboard")).into_response())
}

pub async fn forgot(h: HeaderMap) -> Html<String> {
    page(
        &h,
        "Recover owner access",
        r#"<p>Request a password reset for your owner account. If two-factor authentication is enabled, you will also need an authenticator or recovery code.</p><form method="post" action="/owner/account/reset/request" class="stack"><label>Email address<input type="email" name="email" required maxlength="254"></label><button class="btn primary">Request owner reset</button></form>"#,
    )
}
pub async fn request_reset(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<crate::mail::ResetRequest>,
) -> AppResult<Html<String>> {
    auth::csrf(&s, &h)?;
    if !s.mail_service().available() {
        return Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "Email delivery is not configured. Contact the operator.",
        });
    }
    let email = input.email.trim().to_lowercase();
    if email.len() > 254 {
        return Err(ApiError::bad("Email is too long"));
    }
    let mut tx = s.store.pg.begin().await?;
    let user: Option<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE email=$1 FOR UPDATE")
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(user) = user {
        let owner: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_owners WHERE user_id=$1)")
                .bind(user)
                .fetch_one(&mut *tx)
                .await?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM account_tokens WHERE user_id=$1 AND purpose='owner_reset' AND ((used_at IS NULL AND expires_at>now()) OR created_at>now()-interval '15 minutes')").bind(user).fetch_one(&mut *tx).await?;
        if owner && count < 3 {
            let token = auth::secret();
            sqlx::query("INSERT INTO account_tokens(token_hash,user_id,purpose,expires_at) VALUES($1,$2,'owner_reset',now()+interval '30 minutes')").bind(digest(&token)).bind(user).execute(&mut *tx).await?;
            let body=format!("Reset your Vuuut Click owner password\n\nOpen {}/owner/account/reset#{}\n\nExpires in 30 minutes. Two-factor authentication remains required if enabled. If you did not request this, ignore it.",s.config.public_url,token);
            sqlx::query("INSERT INTO mail_outbox(id,recipient,subject,body,expires_at,account_token_hash) VALUES($1,$2,'Reset your Vuuut Click owner password',$3,now()+interval '30 minutes',$4)").bind(Uuid::new_v4()).bind(email).bind(body).bind(digest(&token)).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(page(
        &h,
        "Check your next step",
        &format!("<p>{}</p>", esc(s.mail_service().notice())),
    ))
}
pub async fn reset_page(h: HeaderMap) -> Html<String> {
    page(
        &h,
        "Reset owner password",
        r#"<p>If two-factor authentication is enabled, supply a fresh authenticator or unused recovery code.</p><form method="post" action="/owner/account/reset" class="stack"><label>Reset token<input id="account-token" name="token" required maxlength="64" autocomplete="off"></label><label>New password<input type="password" name="password" required minlength="12" maxlength="256" autocomplete="new-password"></label><label>Authenticator or recovery code<input name="code" maxlength="64" autocomplete="one-time-code"></label><button class="btn primary">Reset owner password</button></form><script src="/assets/account-token.js" defer></script>"#,
    )
}
#[derive(Deserialize)]
pub struct ResetInput {
    token: String,
    password: String,
    #[serde(default)]
    code: String,
}
pub async fn reset(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<ResetInput>,
) -> AppResult<Html<String>> {
    auth::csrf(&s, &h)?;
    if input.token.len() != 64 {
        return Err(ApiError::bad("Invalid reset link"));
    }
    let hash = auth::hash_password(&s, input.password).await?;
    let mut tx = s.store.pg.begin().await?;
    let user: Uuid = sqlx::query_scalar(
        "SELECT user_id FROM account_tokens WHERE token_hash=$1 AND purpose='owner_reset'",
    )
    .bind(digest(&input.token))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::bad("Invalid reset link"))?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
    let attempts:Option<i32>=sqlx::query_scalar("UPDATE account_tokens SET attempts=attempts+1 WHERE token_hash=$1 AND purpose='owner_reset' AND used_at IS NULL AND expires_at>now() AND attempts<5 AND EXISTS(SELECT 1 FROM platform_owners WHERE user_id=$2) RETURNING attempts").bind(digest(&input.token)).bind(user).fetch_optional(&mut *tx).await?;
    if attempts.is_none() {
        return Err(ApiError::bad(
            "This reset link has expired or has already been used",
        ));
    }
    if !factor(&mut tx, user, &input.code).await? {
        tx.commit().await?;
        return Err(ApiError::bad(
            "Use a fresh authenticator code or an unused recovery code",
        ));
    }
    sqlx::query("UPDATE users SET password_hash=$2 WHERE id=$1")
        .bind(user)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE account_tokens SET used_at=now() WHERE user_id=$1 AND used_at IS NULL")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, user, "owner.password.recovered").await?;
    tx.commit().await?;
    Ok(page(
        &h,
        "Owner password updated",
        r#"<p>All previous sessions have been signed out. Two-factor authentication remains enabled if you set it up.</p><a href="/owner/signin">Sign in as owner</a>"#,
    ))
}
