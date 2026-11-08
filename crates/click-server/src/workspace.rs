//! Customer control-plane operations. Membership and limits are checked transactionally.
use crate::{auth, ApiError, AppResult, AppState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use click_core::{digest, Role};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

pub async fn membership(
    s: &AppState,
    h: &HeaderMap,
    org: Uuid,
    manage: bool,
) -> AppResult<(auth::User, Role)> {
    let user = auth::user(s, h).await?;
    let role:Option<String>=sqlx::query_scalar("SELECT m.role FROM memberships m JOIN organizations o ON o.id=m.organization_id WHERE m.organization_id=$1 AND m.user_id=$2 AND o.status<>'deleting'").bind(org).bind(user.id).fetch_optional(&s.store.pg).await?;
    let role = role
        .as_deref()
        .and_then(Role::parse)
        .ok_or_else(ApiError::not_found)?;
    if manage && !role.can_manage() {
        return Err(ApiError::not_found());
    }
    Ok((user, role))
}
async fn lock_org(tx: &mut PgConnection, org: Uuid) -> AppResult<()> {
    let active: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM organizations WHERE id=$1 AND status='active' FOR UPDATE",
    )
    .bind(org)
    .fetch_optional(tx)
    .await?;
    active.ok_or_else(|| ApiError::conflict("Workspace is not active"))?;
    Ok(())
}
/// Lock the workspace and confirm the actor *still* holds a sufficient role,
/// inside the transaction that is about to mutate.
///
/// The role checked before a transaction opens is a role the actor had a
/// moment ago. Between that check and the write, their membership can be
/// removed or downgraded, or the workspace can be suspended — and the write
/// would still land. Mutations therefore re-check here, holding the
/// organization row, so the role that authorizes a write is the role in force
/// when it commits.
///
/// `allow` is the same predicate the pre-transaction check used, so the two
/// cannot disagree: pass `Role::can_edit_goals` or `Role::can_manage` rather
/// than respelling either as an `IN (…)` list in SQL, which is a list that
/// goes stale silently when a role is added.
///
/// Missing privileges are reported as not-found, matching `authorize_site`:
/// whether a resource exists is itself information.
pub async fn lock_member(
    tx: &mut PgConnection,
    org: Uuid,
    actor: Uuid,
    allow: fn(Role) -> bool,
) -> AppResult<()> {
    lock_org(tx, org).await?;
    recheck_role(tx, org, actor, allow).await
}

/// The same, for the few paths that must keep working while a workspace is
/// suspended.
///
/// Paying is how a past-due workspace recovers, so refusing checkout to a
/// suspended workspace would trap the customer in the state they are trying
/// to leave. The privilege re-check still applies — only the active-workspace
/// requirement is waived, and the name says so at the call site.
pub async fn lock_member_even_if_suspended(
    tx: &mut PgConnection,
    org: Uuid,
    actor: Uuid,
    allow: fn(Role) -> bool,
) -> AppResult<()> {
    let exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
            .bind(org)
            .fetch_optional(&mut *tx)
            .await?;
    exists.ok_or_else(ApiError::not_found)?;
    recheck_role(tx, org, actor, allow).await
}

/// Who the actor is *now*, by the workspace's own membership row.
async fn recheck_role(
    tx: &mut PgConnection,
    org: Uuid,
    actor: Uuid,
    allow: fn(Role) -> bool,
) -> AppResult<()> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE organization_id=$1 AND user_id=$2")
            .bind(org)
            .bind(actor)
            .fetch_optional(&mut *tx)
            .await?;
    if !role.as_deref().and_then(Role::parse).is_some_and(allow) {
        return Err(ApiError::not_found());
    }
    Ok(())
}

/// [`lock_member`] for the goal and funnel mutations, which share one rule.
pub async fn lock_editor(tx: &mut PgConnection, org: Uuid, actor: Uuid) -> AppResult<()> {
    lock_member(tx, org, actor, Role::can_edit_goals).await
}

pub async fn audit(
    tx: &mut PgConnection,
    org: Uuid,
    actor: Uuid,
    action: &str,
    resource: Uuid,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO audit_logs(organization_id,actor_id,action,resource_id) VALUES($1,$2,$3,$4)",
    )
    .bind(org)
    .bind(actor)
    .bind(action)
    .bind(resource)
    .execute(tx)
    .await?;
    Ok(())
}
#[derive(Deserialize)]
pub struct WorkspaceInput {
    pub name: String,
}
pub async fn add_workspace(s: &AppState, h: &HeaderMap, input: WorkspaceInput) -> AppResult<Uuid> {
    auth::csrf(s, h)?;
    let user = auth::user(s, h).await?;
    let name = input.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(ApiError::bad(
            "Workspace name must contain 1–100 characters",
        ));
    }
    let mut tx = s.store.pg.begin().await?;
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM memberships WHERE user_id=$1 AND role='owner'")
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?;
    if count >= 20 {
        return Err(ApiError::conflict("Workspace creation limit reached"));
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO organizations(id,name) VALUES($1,$2)")
        .bind(id)
        .bind(name)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO memberships(organization_id,user_id,role) VALUES($1,$2,'owner')")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO subscriptions(organization_id,plan_id) VALUES($1,'free')")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, id, user.id, "workspace.created", id).await?;
    tx.commit().await?;
    Ok(id)
}
pub async fn create(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<WorkspaceInput>,
) -> AppResult<(StatusCode, Json<serde_json::Value>)> {
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({"id":add_workspace(&s,&h,input).await?})),
    ))
}
pub async fn rename_workspace(
    s: &AppState,
    h: &HeaderMap,
    org: Uuid,
    input: WorkspaceInput,
) -> AppResult<()> {
    auth::csrf(s, h)?;
    let (user, _) = membership(s, h, org, true).await?;
    let name = input.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(ApiError::bad(
            "Workspace name must contain 1–100 characters",
        ));
    }
    let mut tx = s.store.pg.begin().await?;
    lock_org(&mut tx, org).await?;
    let changed=sqlx::query("UPDATE organizations SET name=$1 WHERE id=$2 AND EXISTS(SELECT 1 FROM memberships WHERE organization_id=$2 AND user_id=$3 AND role IN ('owner','admin'))").bind(name).bind(org).bind(user.id).execute(&mut *tx).await?;
    if changed.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    audit(&mut tx, org, user.id, "workspace.renamed", org).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn rename(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(input): Json<WorkspaceInput>,
) -> AppResult<StatusCode> {
    rename_workspace(&s, &h, id, input).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Serialize, sqlx::FromRow)]
pub struct Member {
    pub id: Uuid,
    pub email: String,
    pub role: String,
    pub display_name: String,
}
pub async fn members(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(org): Path<Uuid>,
) -> AppResult<Json<Vec<Member>>> {
    membership(&s, &h, org, false).await?;
    Ok(Json(sqlx::query_as("SELECT u.id,u.email,u.display_name,m.role FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.organization_id=$1 ORDER BY u.email LIMIT 500").bind(org).fetch_all(&s.store.pg).await?))
}
#[derive(Deserialize)]
pub struct InviteInput {
    pub email: String,
    pub role: String,
}
#[derive(Serialize)]
pub struct Invitation {
    pub id: Uuid,
    pub accept_url: String,
    /// Whether the invitation was queued for delivery to the invitee.
    ///
    /// The link is returned either way: the caller still needs something to
    /// show, and an operator with no mail configured has to be able to pass
    /// it on by hand. This says whether they *have* to.
    pub emailed: bool,
}
pub async fn invite_member(
    s: &AppState,
    h: &HeaderMap,
    org: Uuid,
    input: InviteInput,
) -> AppResult<Invitation> {
    auth::csrf(s, h)?;
    let (user, role) = membership(s, h, org, true).await?;
    let email = input.email.trim().to_lowercase();
    if email.parse::<lettre::Address>().is_err()
        || !email.contains('@')
        || email.len() > 254
        || email.chars().any(char::is_whitespace)
        || !matches!(input.role.as_str(), "admin" | "analyst" | "viewer")
    {
        return Err(ApiError::bad("Enter a valid email and team role"));
    }
    if input.role == "admin" && role != Role::Owner {
        return Err(ApiError::forbidden(
            "Only workspace owners can invite administrators",
        ));
    }
    let mut tx = s.store.pg.begin().await?;
    lock_org(&mut tx, org).await?;
    let current: Option<String> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE organization_id=$1 AND user_id=$2")
            .bind(org)
            .bind(user.id)
            .fetch_optional(&mut *tx)
            .await?;
    if !current
        .as_deref()
        .and_then(Role::parse)
        .is_some_and(Role::can_manage)
    {
        return Err(ApiError::not_found());
    }
    if input.role == "admin" && current.as_deref() != Some("owner") {
        return Err(ApiError::not_found());
    }
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.organization_id=$1 AND u.email=$2)").bind(org).bind(&email).fetch_one(&mut *tx).await?;
    if exists {
        return Err(ApiError::conflict(
            "This account is already a workspace member",
        ));
    }
    let occupied:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM memberships WHERE organization_id=$1)+(SELECT count(*) FROM invitations WHERE organization_id=$1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at>now())").bind(org).fetch_one(&mut *tx).await?;
    let limit:i32=sqlx::query_scalar("SELECT p.seat_limit FROM subscriptions s JOIN plans p ON p.id=s.plan_id WHERE s.organization_id=$1").bind(org).fetch_one(&mut *tx).await?;
    if occupied >= limit as i64 {
        return Err(ApiError::conflict("Team seat allowance reached"));
    }
    let outstanding:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM invitations WHERE organization_id=$1 AND email=$2 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at>now())").bind(org).bind(&email).fetch_one(&mut *tx).await?;
    if outstanding {
        return Err(ApiError::conflict(
            "An invitation for this email is already pending",
        ));
    }
    let token = auth::secret();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO invitations(id,organization_id,email,role,token_hash,invited_by,expires_at) VALUES($1,$2,$3,$4,$5,$6,now()+interval '7 days')").bind(id).bind(org).bind(&email).bind(&input.role).bind(digest(&token)).bind(user.id).execute(&mut *tx).await?;
    audit(&mut tx, org, user.id, "team.invited", id).await?;

    // Queue the invitation itself, in this transaction. Until now an invite
    // produced a link and nothing else, so every invitation had to be
    // delivered by hand — the one step of the team flow that left the
    // product. Enqueued inside the transaction so an invitation and its
    // message cannot disagree about existing: no row, no mail, and a mail
    // that fails is retried by the outbox worker rather than lost.
    let accept_url = format!("{}/invite#{}", s.config.public_url, token);
    let emailed = s.mail.available();
    if emailed {
        let workspace: String = sqlx::query_scalar("SELECT name FROM organizations WHERE id=$1")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
        let subject = format!("{} invited you to {}", user.email, workspace);
        // The role is named because accepting changes what the invitee can
        // see, and the expiry because a week-old link failing silently is
        // worse than being told.
        let body = format!(
            concat!(
                "{} invited you to join the {} workspace on Vuuut Click as {}.\n",
                "\n",
                "Open this link to accept:\n",
                "{}\n",
                "\n",
                "You will need a Vuuut Click account for {}. The invitation expires in 7 days ",
                "and can be used once. If you were not expecting this, you can ignore it.\n",
            ),
            user.email, workspace, input.role, accept_url, email
        );
        sqlx::query("INSERT INTO mail_outbox(id,recipient,subject,body) VALUES($1,$2,$3,$4)")
            .bind(Uuid::new_v4())
            .bind(&email)
            .bind(subject)
            .bind(body)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Invitation {
        id,
        accept_url,
        emailed,
    })
}
pub async fn invite(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(org): Path<Uuid>,
    Json(input): Json<InviteInput>,
) -> AppResult<(StatusCode, Json<Invitation>)> {
    Ok((
        StatusCode::CREATED,
        Json(invite_member(&s, &h, org, input).await?),
    ))
}
#[derive(Deserialize)]
pub struct TokenInput {
    pub token: String,
}
pub async fn accept_invitation(s: &AppState, h: &HeaderMap, input: TokenInput) -> AppResult<Uuid> {
    auth::csrf(s, h)?;
    let user = auth::user(s, h).await?;
    if input.token.len() != 64 {
        return Err(ApiError::bad("Invalid invitation token"));
    }
    let org: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM invitations WHERE token_hash=$1")
            .bind(digest(&input.token))
            .fetch_optional(&s.store.pg)
            .await?
            .ok_or_else(ApiError::not_found)?;
    let mut tx = s.store.pg.begin().await?;
    lock_org(&mut tx, org).await?;
    let invitation:Option<(Uuid,String,String)>=sqlx::query_as("SELECT id,email,role FROM invitations WHERE token_hash=$1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at>now() FOR UPDATE").bind(digest(&input.token)).fetch_optional(&mut *tx).await?;
    let (id, email, role) = invitation
        .ok_or_else(|| ApiError::bad("Invitation is expired, revoked or already used"))?;
    if email != user.email {
        return Err(ApiError::forbidden(
            "Sign in with the email this invitation was sent to",
        ));
    }
    let seats:(i64,i32)=sqlx::query_as("SELECT (SELECT count(*) FROM memberships WHERE organization_id=$1),p.seat_limit FROM subscriptions s JOIN plans p ON p.id=s.plan_id WHERE s.organization_id=$1").bind(org).fetch_one(&mut *tx).await?;
    if seats.0 >= seats.1 as i64 {
        return Err(ApiError::conflict("Team seat allowance reached"));
    }
    sqlx::query("INSERT INTO memberships(organization_id,user_id,role) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(org).bind(user.id).bind(role).execute(&mut *tx).await?;
    sqlx::query("UPDATE invitations SET accepted_at=now() WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, org, user.id, "team.joined", user.id).await?;
    tx.commit().await?;
    Ok(org)
}
pub async fn accept(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<TokenInput>,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(
        serde_json::json!({"organization_id":accept_invitation(&s,&h,input).await?}),
    ))
}
#[derive(Deserialize)]
pub struct RoleInput {
    pub role: String,
}
pub async fn change_member(
    s: &AppState,
    h: &HeaderMap,
    org: Uuid,
    target: Uuid,
    new_role: Option<&str>,
) -> AppResult<()> {
    auth::csrf(s, h)?;
    let (user, _) = membership(s, h, org, true).await?;
    if new_role.is_some_and(|r| Role::parse(r).is_none()) {
        return Err(ApiError::bad("Invalid role"));
    }
    let mut tx = s.store.pg.begin().await?;
    lock_org(&mut tx, org).await?;
    let actor: String =
        sqlx::query_scalar("SELECT role FROM memberships WHERE organization_id=$1 AND user_id=$2")
            .bind(org)
            .bind(user.id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(ApiError::not_found)?;
    let prior: String =
        sqlx::query_scalar("SELECT role FROM memberships WHERE organization_id=$1 AND user_id=$2")
            .bind(org)
            .bind(target)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(ApiError::not_found)?;
    if actor != "owner"
        && (actor != "admin"
            || prior == "owner"
            || prior == "admin"
            || new_role.is_some_and(|r| r == "owner" || r == "admin"))
    {
        return Err(ApiError::not_found());
    }
    if prior == "owner" && new_role != Some("owner") {
        let owners: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM memberships WHERE organization_id=$1 AND role='owner'",
        )
        .bind(org)
        .fetch_one(&mut *tx)
        .await?;
        if owners <= 1 {
            return Err(ApiError::conflict(
                "Assign another owner before removing the last owner",
            ));
        }
    }
    if let Some(role) = new_role {
        sqlx::query("UPDATE memberships SET role=$3 WHERE organization_id=$1 AND user_id=$2")
            .bind(org)
            .bind(target)
            .bind(role)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("DELETE FROM memberships WHERE organization_id=$1 AND user_id=$2")
            .bind(org)
            .bind(target)
            .execute(&mut *tx)
            .await?;
    }
    audit(
        &mut tx,
        org,
        user.id,
        if new_role.is_some() {
            "team.role_changed"
        } else {
            "team.removed"
        },
        target,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn set_role(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((org, target)): Path<(Uuid, Uuid)>,
    Json(input): Json<RoleInput>,
) -> AppResult<StatusCode> {
    change_member(&s, &h, org, target, Some(&input.role)).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn remove_member(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((org, target)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    change_member(&s, &h, org, target, None).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn revoke_invitation(s: &AppState, h: &HeaderMap, org: Uuid, id: Uuid) -> AppResult<()> {
    auth::csrf(s, h)?;
    let (user, _) = membership(s, h, org, true).await?;
    let mut tx = s.store.pg.begin().await?;
    lock_org(&mut tx, org).await?;
    let result = sqlx::query("UPDATE invitations SET revoked_at=now() WHERE id=$1 AND organization_id=$2 AND EXISTS(SELECT 1 FROM memberships WHERE organization_id=$2 AND user_id=$3 AND role IN ('owner','admin'))")
        .bind(id)
        .bind(org)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    audit(&mut tx, org, user.id, "team.invitation_revoked", id).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn revoke(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((org, id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    revoke_invitation(&s, &h, org, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
