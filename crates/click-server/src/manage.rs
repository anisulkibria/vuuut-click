//! Website and credential management; all operations use authenticated membership.
use crate::{auth, workspace, ApiError, AppResult, AppState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct SiteInput {
    pub name: String,
    pub origin: String,
    pub active: bool,
}
pub async fn update_website(
    s: &AppState,
    h: &HeaderMap,
    id: Uuid,
    input: SiteInput,
) -> AppResult<()> {
    auth::csrf(s, h)?;
    let (site, _) = auth::authorize_site(s, h, id, true).await?;
    let user = auth::user(s, h).await?;
    if site.is_demo {
        return Err(ApiError::bad(
            "Demonstration websites cannot be connected to real collection",
        ));
    }
    let parsed = url::Url::parse(input.origin.trim())
        .map_err(|_| ApiError::bad("Invalid website origin"))?;
    let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if input.name.trim().is_empty()
        || input.name.len() > 100
        || input.origin.len() > 255
        || parsed.host_str().is_none()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !(parsed.scheme() == "https"
            || s.config.allow_localhost && local && parsed.scheme() == "http")
    {
        return Err(ApiError::bad(
            "Enter a name and a valid HTTPS origin without a path or credentials",
        ));
    }
    let origin = parsed.origin().ascii_serialization();
    let changed = origin != site.origin;
    let mut tx = s.store.pg.begin().await?;
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(site.organization_id)
        .fetch_one(&mut *tx)
        .await?;
    let result=sqlx::query("UPDATE websites SET name=$1,origin=$2,active=$3,verification_token=CASE WHEN $4 THEN $5 ELSE verification_token END,verified_at=CASE WHEN $4 THEN $6 ELSE verified_at END WHERE id=$7 AND deletion_requested_at IS NULL AND EXISTS(SELECT 1 FROM memberships WHERE organization_id=$8 AND user_id=$9 AND role IN ('owner','admin'))")
  .bind(input.name.trim()).bind(origin).bind(input.active).bind(changed).bind(auth::secret()).bind((local&&s.config.allow_localhost).then(chrono::Utc::now)).bind(id).bind(site.organization_id).bind(user.id).execute(&mut *tx).await;
    let result = match result {
        Ok(r) => r,
        Err(e)
            if e.as_database_error()
                .is_some_and(|e| e.is_unique_violation()) =>
        {
            return Err(ApiError::conflict(
                "This origin already exists in the workspace",
            ))
        }
        Err(e) => return Err(e.into()),
    };
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    workspace::audit(
        &mut tx,
        site.organization_id,
        user.id,
        "website.updated",
        id,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn update(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<SiteInput>,
) -> AppResult<StatusCode> {
    update_website(&s, &h, id, input).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
pub struct CredentialInput {
    pub label: String,
    #[serde(default = "read_scope")]
    pub scope: String,
}
fn read_scope() -> String {
    "read".into()
}
#[derive(Serialize)]
pub struct Credential {
    pub id: Uuid,
    pub key: String,
    pub scope: String,
}
pub async fn issue_key(
    s: &AppState,
    h: &HeaderMap,
    id: Uuid,
    input: CredentialInput,
    rotate: Option<Uuid>,
) -> AppResult<Credential> {
    auth::csrf(s, h)?;
    let (site, _) = auth::authorize_site(s, h, id, true).await?;
    let user = auth::user(s, h).await?;
    if input.label.trim().is_empty()
        || input.label.len() > 100
        || !matches!(input.scope.as_str(), "read" | "integration")
    {
        return Err(ApiError::bad("Enter a credential name and valid scope"));
    }
    let mut tx = s.store.pg.begin().await?;
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(site.organization_id)
        .fetch_one(&mut *tx)
        .await?;
    let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM memberships WHERE organization_id=$1 AND user_id=$2 AND role IN ('owner','admin'))").bind(site.organization_id).bind(user.id).fetch_one(&mut *tx).await?;
    if !allowed {
        return Err(ApiError::not_found());
    }
    if let Some(old) = rotate {
        let result=sqlx::query("UPDATE api_keys SET revoked_at=now() WHERE id=$1 AND website_id=$2 AND revoked_at IS NULL").bind(old).bind(id).execute(&mut *tx).await?;
        if result.rows_affected() == 0 {
            return Err(ApiError::not_found());
        }
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM api_keys WHERE website_id=$1 AND revoked_at IS NULL",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if count >= 20 {
        return Err(ApiError::conflict(
            "Revoke unused credentials before creating more",
        ));
    }
    let credential = Credential {
        id: Uuid::new_v4(),
        key: format!("vck_{}", auth::secret()),
        scope: input.scope,
    };
    sqlx::query(
        "INSERT INTO api_keys(id,website_id,token_hash,label,scope) VALUES($1,$2,$3,$4,$5)",
    )
    .bind(credential.id)
    .bind(id)
    .bind(click_core::digest(&credential.key))
    .bind(input.label.trim())
    .bind(&credential.scope)
    .execute(&mut *tx)
    .await?;
    workspace::audit(
        &mut tx,
        site.organization_id,
        user.id,
        if rotate.is_some() {
            "credential.rotated"
        } else {
            "credential.created"
        },
        credential.id,
    )
    .await?;
    tx.commit().await?;
    Ok(credential)
}
pub async fn create_key(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<CredentialInput>,
) -> AppResult<(StatusCode, Json<Credential>)> {
    Ok((
        StatusCode::CREATED,
        Json(issue_key(&s, &h, id, input, None).await?),
    ))
}
pub async fn rotate_key(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, key)): Path<(Uuid, Uuid)>,
    Json(input): Json<CredentialInput>,
) -> AppResult<Json<Credential>> {
    Ok(Json(issue_key(&s, &h, id, input, Some(key)).await?))
}
#[derive(Serialize, sqlx::FromRow)]
pub struct KeyRecord {
    pub id: Uuid,
    pub label: String,
    pub scope: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}
pub async fn keys(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Vec<KeyRecord>>> {
    auth::authorize_site(&s, &h, id, true).await?;
    Ok(Json(sqlx::query_as("SELECT id,label,scope,created_at,revoked_at FROM api_keys WHERE website_id=$1 ORDER BY created_at DESC LIMIT 100").bind(id).fetch_all(&s.store.pg).await?))
}
pub async fn delete_goal(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, goal)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    auth::csrf(&s, &h)?;
    let (site, role) = auth::authorize_site(&s, &h, id, false).await?;
    if !role.can_edit_goals() {
        return Err(ApiError::not_found());
    }
    let user = auth::user(&s, &h).await?;
    let mut tx = s.store.pg.begin().await?;
    let deleted = sqlx::query("DELETE FROM goals WHERE id=$1 AND website_id=$2")
        .bind(goal)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    workspace::audit(&mut tx, site.organization_id, user.id, "goal.deleted", goal).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
pub struct ProfileInput {
    pub display_name: String,
}
pub async fn profile(s: &AppState, h: &HeaderMap, input: ProfileInput) -> AppResult<()> {
    auth::csrf(s, h)?;
    let u = auth::user(s, h).await?;
    if input.display_name.len() > 100 {
        return Err(ApiError::bad("Name must contain at most 100 characters"));
    }
    sqlx::query("UPDATE users SET display_name=$1 WHERE id=$2")
        .bind(input.display_name.trim())
        .bind(u.id)
        .execute(&s.store.pg)
        .await?;
    Ok(())
}
#[derive(Deserialize)]
pub struct PasswordInput {
    pub current_password: String,
    pub new_password: String,
}
pub async fn change_password(
    s: &AppState,
    h: &HeaderMap,
    input: PasswordInput,
) -> AppResult<(String, auth::User)> {
    auth::csrf(s, h)?;
    let user = auth::user(s, h).await?;
    if input.new_password.len() < 12 || input.new_password.len() > 256 {
        return Err(ApiError::bad("New password must contain 12–256 characters"));
    }
    auth::verify_credentials(
        s,
        auth::Credentials {
            email: user.email.clone(),
            password: input.current_password,
            organization: None,
        },
    )
    .await?;
    let permit = s
        .password_slots
        .acquire()
        .await
        .map_err(ApiError::internal)?;
    let hash = tokio::task::spawn_blocking(move || {
        use argon2::{
            password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
            Argon2,
        };
        Argon2::default()
            .hash_password(
                input.new_password.as_bytes(),
                &SaltString::generate(&mut OsRng),
            )
            .map(|h| h.to_string())
            .map_err(|_| anyhow::anyhow!("Password hashing failed"))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    drop(permit);
    let token = auth::secret();
    let mut tx = s.store.pg.begin().await?;
    sqlx::query("UPDATE users SET password_hash=$1 WHERE id=$2")
        .bind(hash)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND audience='user'")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at,audience) VALUES($1,$2,now()+interval '7 days','user')").bind(click_core::digest(&token)).bind(user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok((token, user))
}
pub async fn other_sessions(s: &AppState, h: &HeaderMap) -> AppResult<()> {
    auth::csrf(s, h)?;
    let user = auth::user(s, h).await?;
    let token = auth::cookie_value(h, "click_session").ok_or_else(ApiError::unauthorized)?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND audience='user' AND token_hash<>$2")
        .bind(user.id)
        .bind(click_core::digest(&token))
        .execute(&s.store.pg)
        .await?;
    Ok(())
}

pub async fn profile_api(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<ProfileInput>,
) -> AppResult<StatusCode> {
    profile(&s, &h, input).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn password_api(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<PasswordInput>,
) -> AppResult<axum::response::Response> {
    let (token, user) = change_password(&s, &h, input).await?;
    Ok(auth::session_response(&s, &token, user))
}
pub async fn sessions_api(State(s): State<AppState>, h: HeaderMap) -> AppResult<StatusCode> {
    other_sessions(&s, &h).await?;
    Ok(StatusCode::NO_CONTENT)
}
