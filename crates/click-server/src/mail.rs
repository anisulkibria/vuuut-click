//! Transactional account mail. Local development writes private files, never reset tokens to logs.
use crate::{
    auth,
    dashboard::{document, esc},
    ApiError, AppResult, AppState,
};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    Form, Json,
};
use lettre::{
    transport::smtp::authentication::Credentials, AsyncSmtpTransport, AsyncTransport, Message,
    Tokio1Executor,
};
use serde::Deserialize;
use uuid::Uuid;

fn config_var(
    values: &std::collections::HashMap<String, String>,
    key: &str,
) -> Result<String, std::env::VarError> {
    values
        .get(key)
        .filter(|v| !v.is_empty())
        .cloned()
        .ok_or(std::env::VarError::NotPresent)
}

pub struct Mailer {
    mode: Mode,
}
enum Mode {
    Disabled,
    File(std::path::PathBuf),
    Smtp {
        transport: AsyncSmtpTransport<Tokio1Executor>,
        from: lettre::message::Mailbox,
        allowed_recipient: Option<String>,
    },
}
impl Mailer {
    pub fn local(directory: std::path::PathBuf) -> Self {
        Self {
            mode: Mode::File(directory),
        }
    }
    pub async fn test_connection(&self) -> anyhow::Result<()> {
        let Mode::Smtp { transport, .. } = &self.mode else {
            anyhow::bail!("SMTP is not configured")
        };
        anyhow::ensure!(
            transport.test_connection().await?,
            "SMTP did not accept connection"
        );
        Ok(())
    }
    pub async fn send_probe(&self, recipient: &str) -> anyhow::Result<()> {
        let Mode::Smtp {
            transport, from, ..
        } = &self.mode
        else {
            anyhow::bail!("SMTP must be configured for the delivery probe")
        };
        let address: lettre::Address = recipient.parse()?;
        let message=Message::builder().from(from.clone()).to(lettre::message::Mailbox::new(None,address)).subject("Vuuut Click commercial SMTP validation").body("This message was requested to validate Vuuut Click SMTP delivery. No account or payment has been changed. SMTP acceptance must be confirmed in the recipient inbox before marking delivery verified.".to_owned())?;
        transport
            .send(message)
            .await
            .map_err(|_| anyhow::anyhow!("SMTP probe was rejected or unavailable"))?;
        Ok(())
    }

    pub fn from_env(local: bool) -> anyhow::Result<Self> {
        Self::from_config(local, &std::env::vars().collect())
    }
    pub fn from_config(
        local: bool,
        values: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<Self> {
        if let Ok(host) = config_var(values, "SMTP_HOST") {
            if !host.trim().is_empty() {
                let from = config_var(values, "SMTP_FROM")
                    .map_err(|_| anyhow::anyhow!("SMTP_FROM is required"))?
                    .parse()?;
                let port: u16 = config_var(values, "SMTP_PORT")
                    .unwrap_or_else(|_| "587".into())
                    .parse()?;
                let mut builder = if port == 465 {
                    AsyncSmtpTransport::<Tokio1Executor>::relay(&host)?
                } else {
                    AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)?
                }
                .port(port)
                .timeout(Some(std::time::Duration::from_secs(15)));
                match (
                    config_var(values, "SMTP_USERNAME").ok(),
                    config_var(values, "SMTP_PASSWORD").ok(),
                ) {
                    (Some(user), Some(pass)) => {
                        builder = builder.credentials(Credentials::new(user, pass))
                    }
                    (None, None) => {}
                    _ => anyhow::bail!("Set both SMTP_USERNAME and SMTP_PASSWORD"),
                }
                return Ok(Self {
                    mode: Mode::Smtp {
                        transport: builder.build(),
                        from,
                        allowed_recipient: if local {
                            let recipient=config_var(values, "COMMERCIAL_TEST_EMAIL").map_err(|_|anyhow::anyhow!("Development SMTP requires COMMERCIAL_TEST_EMAIL to limit delivery to your test inbox"))?;
                            let _: lettre::Address = recipient.parse()?;
                            Some(recipient)
                        } else {
                            None
                        },
                    },
                });
            }
        }
        Ok(Self {
            mode: if local {
                Mode::File(std::path::PathBuf::from(".local/mail"))
            } else {
                Mode::Disabled
            },
        })
    }
    pub fn mode(&self) -> &'static str {
        match self.mode {
            Mode::Disabled => "disabled",
            Mode::File(_) => "local_file",
            Mode::Smtp { .. } => "smtp",
        }
    }
    pub fn available(&self) -> bool {
        !matches!(self.mode, Mode::Disabled)
    }
    pub fn notice(&self) -> &'static str {
        match self.mode{Mode::File(_)=>"Development email is queued in the private local mailbox (.local/mail). No external email is sent.",Mode::Smtp{allowed_recipient:Some(_),..}=>"Development SMTP delivers only to the configured test inbox.",Mode::Smtp{..}=>"If the account exists, a message has been queued for delivery. Check your inbox.",Mode::Disabled=>"Email delivery is not configured. Contact the operator."}
    }
    pub async fn drain_once(&self, store: &click_store::Store) -> anyhow::Result<()> {
        sqlx::query("UPDATE mail_outbox SET state='expired',body=NULL,last_error='link_expired' WHERE state IN ('pending','dead') AND (expires_at<=now() OR EXISTS(SELECT 1 FROM account_tokens t WHERE t.token_hash=mail_outbox.account_token_hash AND (t.used_at IS NOT NULL OR t.expires_at<=now())) OR EXISTS(SELECT 1 FROM invitations i WHERE i.id=mail_outbox.invitation_id AND (i.accepted_at IS NOT NULL OR i.revoked_at IS NOT NULL OR i.expires_at<=now())))").execute(&store.pg).await?;
        sqlx::query("DELETE FROM mail_outbox WHERE state IN ('delivered','expired') AND created_at<now()-interval '30 days'").execute(&store.pg).await?;
        if !self.available() {
            return Ok(());
        }
        let mut tx = store.pg.begin().await?;
        let row:Option<(Uuid,String,String,String)>=sqlx::query_as("SELECT id,recipient,subject,body FROM mail_outbox WHERE state='pending' AND body IS NOT NULL AND available_at<=now() AND (expires_at IS NULL OR expires_at>now()) ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED").fetch_optional(&mut *tx).await?;
        let Some((id, to, subject, body)) = row else {
            return Ok(());
        };
        // Keep construction/file/SMTP failures inside the same retry boundary.
        // An invalid address must consume attempts, not roll back forever.
        let result: anyhow::Result<()> = async {
            let _: lettre::Address = to.parse()?;
            match &self.mode {
                Mode::File(directory) => {
                    let directory = directory.clone();
                    let message = format!("To: {to}\nSubject: {subject}\n\n{body}\n");
                    tokio::task::spawn_blocking(move || {
                        std::fs::create_dir_all(&directory)?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            std::fs::set_permissions(
                                &directory,
                                std::fs::Permissions::from_mode(0o700),
                            )?;
                        }
                        let path = directory.join(format!("{id}.eml"));
                        let mut options = std::fs::OpenOptions::new();
                        options.write(true).create(true).truncate(true);
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::OpenOptionsExt;
                            options.mode(0o600);
                        }
                        let mut file = options.open(path)?;
                        use std::io::Write;
                        file.write_all(message.as_bytes())?;
                        file.sync_all()?;
                        Ok::<_, anyhow::Error>(())
                    })
                    .await??;
                    Ok(())
                }
                Mode::Smtp {
                    transport,
                    from,
                    allowed_recipient,
                } => {
                    if !test_recipient_allowed(allowed_recipient.as_deref(), &to) {
                        anyhow::bail!(
                            "Development SMTP recipient is outside the configured test inbox"
                        );
                    }
                    let message = Message::builder()
                        .from(from.clone())
                        .to(to.parse()?)
                        .subject(subject)
                        .body(body)?;
                    transport
                        .send(message)
                        .await
                        .map(|_| ())
                        .map_err(Into::into)
                }
                Mode::Disabled => Ok(()),
            }
        }
        .await;
        if result.is_err() {
            sqlx::query("UPDATE mail_outbox SET attempts=attempts+1,available_at=now()+make_interval(secs=>least(3600,60*(1<<least(attempts,6)))),last_error='delivery_failed',state=CASE WHEN attempts>=9 THEN 'dead' ELSE 'pending' END WHERE id=$1").bind(id).execute(&mut *tx).await?;
            tx.commit().await?;
            anyhow::bail!("Mail delivery failed; durable retry state was recorded");
        }
        sqlx::query("UPDATE mail_outbox SET state='delivered',body=NULL,delivered_at=now(),last_error=NULL WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

pub async fn enqueue_account_token(
    tx: &mut sqlx::PgConnection,
    user: Uuid,
    email: &str,
    purpose: &str,
    base: &str,
) -> AppResult<()> {
    let token = auth::secret();
    let ttl = if purpose == "reset" { 1800 } else { 86400 };
    sqlx::query("INSERT INTO account_tokens(token_hash,user_id,purpose,expires_at) VALUES($1,$2,$3,now()+make_interval(secs=>$4::int))").bind(click_core::digest(&token)).bind(user).bind(purpose).bind(ttl).execute(&mut *tx).await?;
    let subject = if purpose == "reset" {
        "Reset your Vuuut Click password"
    } else {
        "Verify your Vuuut Click email"
    };
    let body=format!("{subject}\n\nOpen {base}/account/{purpose}#{token}\n\nThis link expires in {} minutes and can be used once.",ttl/60);
    sqlx::query("INSERT INTO mail_outbox(id,recipient,subject,body,expires_at,account_token_hash) VALUES($1,$2,$3,$4,now()+make_interval(secs=>$5::int),$6)")
        .bind(Uuid::new_v4())
        .bind(email)
        .bind(subject)
        .bind(body)
        .bind(ttl)
        .bind(click_core::digest(&token))
        .execute(tx)
        .await?;
    Ok(())
}

pub async fn request_token(s: &AppState, email: &str, purpose: &str) -> AppResult<()> {
    if !s.mail_service().available() {
        return Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "Email delivery is not configured",
        });
    }
    if !matches!(purpose, "verify" | "reset") {
        return Err(ApiError::bad("Invalid account token purpose"));
    }
    let email = email.trim().to_lowercase();
    if email.len() > 254 {
        return Err(ApiError::bad("Email is too long"));
    }
    let user:Option<Uuid>=sqlx::query_scalar("SELECT id FROM users WHERE email=$1 AND NOT EXISTS(SELECT 1 FROM platform_owners WHERE user_id=users.id)").bind(&email).fetch_optional(&s.store.pg).await?;
    let Some(user) = user else { return Ok(()) };
    let mut tx = s.store.pg.begin().await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
    let owner: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platform_owners WHERE user_id=$1)")
            .bind(user)
            .fetch_one(&mut *tx)
            .await?;
    if owner {
        return Ok(());
    }
    let outstanding:i64=sqlx::query_scalar("SELECT count(*) FROM account_tokens WHERE user_id=$1 AND purpose=$2 AND ((expires_at>now() AND used_at IS NULL) OR created_at>now()-interval '15 minutes')").bind(user).bind(purpose).fetch_one(&mut *tx).await?;
    if outstanding >= 3 {
        return Ok(());
    }
    let token = auth::secret();
    let ttl = if purpose == "reset" { 1800 } else { 86400 };
    sqlx::query("INSERT INTO account_tokens(token_hash,user_id,purpose,expires_at) VALUES($1,$2,$3,now()+make_interval(secs=>$4::int))").bind(click_core::digest(&token)).bind(user).bind(purpose).bind(ttl).execute(&mut *tx).await?;
    let label = if purpose == "reset" {
        "Reset your Vuuut Click password"
    } else {
        "Verify your Vuuut Click email"
    };
    let link = format!("{}/account/{purpose}#{token}", s.config.public_url);
    let body=format!("{label}\n\nOpen this link to continue:\n{link}\n\nThe link expires in {} minutes and can be used once. If you did not request this, you can ignore it.",ttl/60);
    sqlx::query("INSERT INTO mail_outbox(id,recipient,subject,body,expires_at,account_token_hash) VALUES($1,$2,$3,$4,now()+make_interval(secs=>$5::int),$6)")
        .bind(Uuid::new_v4())
        .bind(email)
        .bind(label)
        .bind(body)
        .bind(ttl)
        .bind(click_core::digest(&token))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
#[derive(Deserialize)]
pub struct ResetRequest {
    pub email: String,
}
pub async fn reset_request_api(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<ResetRequest>,
) -> AppResult<StatusCode> {
    auth::csrf(&s, &h)?;
    request_token(&s, &input.email, "reset").await?;
    Ok(StatusCode::ACCEPTED)
}
pub async fn verification_request_api(
    State(s): State<AppState>,
    h: HeaderMap,
) -> AppResult<StatusCode> {
    auth::csrf(&s, &h)?;
    let user = auth::user(&s, &h).await?;
    request_token(&s, &user.email, "verify").await?;
    Ok(StatusCode::ACCEPTED)
}
#[derive(Deserialize)]
pub struct CompleteInput {
    pub token: String,
    pub password: Option<String>,
}
pub async fn complete(s: &AppState, input: CompleteInput, purpose: &str) -> AppResult<()> {
    if !matches!(purpose, "verify" | "reset") {
        return Err(ApiError::bad("Invalid account token purpose"));
    }
    if input.token.len() != 64 {
        return Err(ApiError::bad("Invalid account link"));
    }
    let token_hash = click_core::digest(&input.token);
    let user:Uuid=sqlx::query_scalar("SELECT t.user_id FROM account_tokens t WHERE t.token_hash=$1 AND t.purpose=$2 AND t.used_at IS NULL AND t.expires_at>now() AND NOT EXISTS(SELECT 1 FROM platform_owners WHERE user_id=t.user_id)").bind(&token_hash).bind(purpose).fetch_optional(&s.store.pg).await?.ok_or_else(||ApiError::bad("This link has expired or has already been used"))?;
    let hash = if purpose == "reset" {
        let password = input
            .password
            .ok_or_else(|| ApiError::bad("Enter a new password"))?;
        if password.len() < 12 || password.len() > 256 {
            return Err(ApiError::bad("Password must contain 12–256 characters"));
        }
        let _permit = s
            .password_slots
            .acquire()
            .await
            .map_err(ApiError::internal)?;
        Some(
            tokio::task::spawn_blocking(move || {
                use argon2::{
                    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
                    Argon2,
                };
                Argon2::default()
                    .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
                    .map(|h| h.to_string())
                    .map_err(|_| anyhow::anyhow!("Password hashing failed"))
            })
            .await
            .map_err(ApiError::internal)?
            .map_err(ApiError::internal)?,
        )
    } else {
        None
    };
    let mut tx = s.store.pg.begin().await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
    let used=sqlx::query("UPDATE account_tokens SET used_at=now() WHERE token_hash=$1 AND used_at IS NULL AND expires_at>now() AND NOT EXISTS(SELECT 1 FROM platform_owners WHERE user_id=$2)").bind(token_hash).bind(user).execute(&mut *tx).await?;
    if used.rows_affected() == 0 {
        return Err(ApiError::bad("This account link is no longer valid"));
    }
    if let Some(hash) = hash {
        sqlx::query("UPDATE users SET password_hash=$1 WHERE id=$2")
            .bind(hash)
            .bind(user)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND audience='user'")
            .bind(user)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("UPDATE users SET email_verified_at=now() WHERE id=$1")
            .bind(user)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE account_tokens SET used_at=now() WHERE user_id=$1 AND purpose=$2 AND used_at IS NULL").bind(user).bind(purpose).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn reset_complete_api(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<CompleteInput>,
) -> AppResult<StatusCode> {
    auth::csrf(&s, &h)?;
    complete(&s, input, "reset").await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn verify_complete_api(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<CompleteInput>,
) -> AppResult<StatusCode> {
    auth::csrf(&s, &h)?;
    complete(&s, input, "verify").await?;
    Ok(StatusCode::NO_CONTENT)
}
fn account_page(h: &HeaderMap, title: &str, body: &str) -> Html<String> {
    document(
        title,
        h,
        &format!(
            r#"<main id="main" class="auth-main"><section class="auth-card"><a href="/">← Vuuut Click</a><h2>{}</h2>{body}</section></main>"#,
            esc(title)
        ),
    )
}
pub async fn forgot(h: HeaderMap) -> Html<String> {
    account_page(
        &h,
        "A fresh start.",
        r#"<p>Enter your customer account email to request a password reset.</p><form action="/account/reset/request" method="post" class="stack"><label>Email address<input type="email" name="email" required maxlength="254" autocomplete="email"></label><button class="btn primary">Request reset link</button></form><p><a href="/signin">Back to sign in</a></p>"#,
    )
}
pub async fn request_form(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<ResetRequest>,
) -> AppResult<Html<String>> {
    auth::csrf(&s, &h)?;
    request_token(&s, &input.email, "reset").await?;
    Ok(account_page(
        &h,
        "Check your next step.",
        &format!(
            "<p>{}</p><p><a href=\"/signin\">Return to sign in</a></p>",
            esc(s.mail_service().notice())
        ),
    ))
}
pub async fn reset_page(h: HeaderMap) -> Html<String> {
    account_page(
        &h,
        "Choose a new password.",
        r#"<form action="/account/reset" method="post" class="stack"><label>Reset token<input id="account-token" name="token" required maxlength="64" autocomplete="off"></label><label>New password<input type="password" name="password" required minlength="12" maxlength="256" autocomplete="new-password"></label><button class="btn primary">Reset password</button></form><script src="/assets/account-token.js" defer></script>"#,
    )
}
pub async fn verify_page(h: HeaderMap) -> Html<String> {
    account_page(
        &h,
        "Confirm your email.",
        r#"<form action="/account/verify" method="post" class="stack"><label>Verification token<input id="account-token" name="token" required maxlength="64" autocomplete="off"></label><button class="btn primary">Verify email</button></form><script src="/assets/account-token.js" defer></script>"#,
    )
}
pub async fn reset_form(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<CompleteInput>,
) -> AppResult<Html<String>> {
    auth::csrf(&s, &h)?;
    complete(&s, input, "reset").await?;
    Ok(account_page(
        &h,
        "Password updated.",
        r#"<p>Your other customer sessions are signed out.</p><a class="btn primary" href="/signin">Sign in with your new password</a>"#,
    ))
}
pub async fn verify_form(
    State(s): State<AppState>,
    h: HeaderMap,
    Form(input): Form<CompleteInput>,
) -> AppResult<Html<String>> {
    auth::csrf(&s, &h)?;
    complete(&s, input, "verify").await?;
    Ok(account_page(
        &h,
        "Email verified.",
        r#"<p>Your email address has been confirmed.</p><a class="btn primary" href="/settings?tab=account">Return to account settings</a>"#,
    ))
}
pub async fn script() -> Response {
    ([(header::CONTENT_TYPE,"application/javascript; charset=utf-8")],r#"if(/^#[a-f0-9]{64}$/.test(location.hash)){document.getElementById('account-token').value=location.hash.slice(1);history.replaceState(null,'',location.pathname)}"#).into_response()
}

fn test_recipient_allowed(allowed: Option<&str>, recipient: &str) -> bool {
    allowed.is_none_or(|address| address.eq_ignore_ascii_case(recipient))
}
#[cfg(test)]
mod delivery_scope_tests {
    #[test]
    fn development_smtp_does_not_send_fixtures_to_other_addresses() {
        assert!(super::test_recipient_allowed(
            Some("Operator@example.com"),
            "operator@example.com"
        ));
        assert!(!super::test_recipient_allowed(
            Some("operator@example.com"),
            "fixture@test.example"
        ));
        assert!(!super::test_recipient_allowed(
            Some("operator@example.com"),
            "operator@example.com.evil"
        ));
        assert!(super::test_recipient_allowed(
            None,
            "registered-customer@example.org"
        ));
    }
}
