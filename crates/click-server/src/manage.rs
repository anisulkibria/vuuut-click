//! Website and credential management; all operations use authenticated membership.
use crate::{
    api::{Goal, NewGoal},
    auth, workspace, ApiError, AppResult, AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use click_core::Role;
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
    let mut tx = s.store.pg.begin().await?;
    // The membership condition that used to be an `EXISTS` inside this
    // UPDATE now lives here, with the workspace's status, so the role is
    // named once and a suspended workspace cannot change where collection
    // comes from.
    workspace::lock_member(&mut tx, site.organization_id, user.id, Role::can_manage).await?;
    let result=sqlx::query("UPDATE websites SET name=$1,origin=$2,active=$3,verification_token=CASE WHEN origin<>$2 THEN $4 ELSE verification_token END,verified_at=CASE WHEN origin<>$2 THEN $5 ELSE verified_at END WHERE id=$6 AND deletion_requested_at IS NULL")
  .bind(input.name.trim()).bind(origin).bind(input.active).bind(auth::secret()).bind((local&&s.config.allow_localhost).then(chrono::Utc::now)).bind(id).execute(&mut *tx).await;
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
    workspace::lock_member(&mut tx, site.organization_id, user.id, Role::can_manage).await?;
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
/// Delete a goal. Separate from its handler so the dashboard's own form can
/// call it, as the funnel pair already does.
pub async fn remove_goal(s: &AppState, h: &HeaderMap, id: Uuid, goal: Uuid) -> AppResult<()> {
    auth::csrf(s, h)?;
    let (site, role) = auth::authorize_site(s, h, id, false).await?;
    if !role.can_edit_goals() {
        return Err(ApiError::not_found());
    }
    let user = auth::user(s, h).await?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_editor(&mut tx, site.organization_id, user.id).await?;
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
    Ok(())
}

pub async fn delete_goal(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, goal)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    remove_goal(&s, &h, id, goal).await?;
    Ok(StatusCode::NO_CONTENT)
}
/// Rename a goal, or point it at a different custom event.
///
/// The event name is editable on purpose: a goal whose event was renamed in
/// the site's own code was previously unfixable — the only route was to
/// delete the goal and lose its name, because creation was the only writer.
/// Completions are counted by matching the event name at query time, so
/// changing it re-reads history rather than losing it.
pub async fn edit_goal(
    s: &AppState,
    h: &HeaderMap,
    id: Uuid,
    goal: Uuid,
    b: NewGoal,
) -> AppResult<Goal> {
    auth::csrf(s, h)?;
    let (site, role) = auth::authorize_site(s, h, id, false).await?;
    if !role.can_edit_goals() {
        return Err(ApiError::not_found());
    }
    let name = b.name.trim().to_string();
    // The same rules creation applies; validated here rather than shared with
    // it because a goal is also the shape the API accepts, and two callers
    // that disagree about what is valid is the bug this avoids.
    if name.is_empty()
        || name.len() > 100
        || b.event_name.is_empty()
        || b.event_name.len() > 64
        || !b
            .event_name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
    {
        return Err(ApiError::bad(
            "Use a goal name and a valid custom event name",
        ));
    }
    let user = auth::user(s, h).await?;
    let mut tx = s.store.pg.begin().await?;
    workspace::lock_editor(&mut tx, site.organization_id, user.id).await?;
    // Scoped by website as well as id, so a goal id belonging to another
    // site cannot be reached through a site this membership does authorize.
    let result =
        sqlx::query("UPDATE goals SET name=$1,event_name=$2 WHERE id=$3 AND website_id=$4")
            .bind(&name)
            .bind(&b.event_name)
            .bind(goal)
            .bind(id)
            .execute(&mut *tx)
            .await;
    match result {
        Ok(done) if done.rows_affected() == 0 => return Err(ApiError::not_found()),
        Ok(_) => {}
        Err(e) => {
            if e.as_database_error()
                .is_some_and(|e| e.is_unique_violation())
            {
                return Err(ApiError::conflict("A goal with this name already exists"));
            }
            return Err(e.into());
        }
    }
    workspace::audit(&mut tx, site.organization_id, user.id, "goal.updated", goal).await?;
    tx.commit().await?;
    Ok(Goal {
        id: goal,
        name,
        event_name: b.event_name,
    })
}

pub async fn update_goal(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, goal)): Path<(Uuid, Uuid)>,
    Json(b): Json<NewGoal>,
) -> AppResult<Json<Goal>> {
    Ok(Json(edit_goal(&s, &h, id, goal, b).await?))
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
    let mut tx = s.store.pg.begin().await?;
    auth::lock_customer(&mut tx, h, u.id).await?;
    sqlx::query("UPDATE users SET display_name=$1 WHERE id=$2")
        .bind(input.display_name.trim())
        .bind(u.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
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
    // Hash the new password before opening the transaction. Argon2 is
    // deliberately slow, and holding the user's row for the duration would
    // make a password change block every other operation on that account.
    let permit = s
        .password_slots
        .acquire()
        .await
        .map_err(ApiError::internal)?;
    let new_password = input.new_password;
    let hash = tokio::task::spawn_blocking(move || {
        use argon2::{
            password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
            Argon2,
        };
        Argon2::default()
            .hash_password(new_password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|h| h.to_string())
            .map_err(|_| anyhow::anyhow!("Password hashing failed"))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;
    drop(permit);

    let token = auth::secret();
    let mut tx = s.store.pg.begin().await?;
    // The current password is proved here, against the row as it is *now* and
    // with it locked, not against a hash read before the transaction opened.
    // Otherwise two concurrent changes could each verify the same old
    // password and the later write would silently win — so a stolen session
    // holding the old password could overwrite a legitimate change made
    // moments earlier. Serialized on this row, the second attempt is asked
    // for a password that no longer exists.
    let current = auth::lock_customer(&mut tx, h, user.id).await?;
    if !auth::verify_against_hash(s, current, input.current_password).await? {
        return Err(ApiError::unauthorized());
    }
    sqlx::query("UPDATE users SET password_hash=$1 WHERE id=$2")
        .bind(hash)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE account_tokens SET used_at=now() WHERE user_id=$1 AND purpose='reset' AND used_at IS NULL").bind(user.id).execute(&mut *tx).await?;
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
    let mut tx = s.store.pg.begin().await?;
    auth::lock_customer(&mut tx, h, user.id).await?;
    let token = auth::cookie_value(h, "click_session").ok_or_else(ApiError::unauthorized)?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND audience='user' AND token_hash<>$2")
        .bind(user.id)
        .bind(click_core::digest(&token))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
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
