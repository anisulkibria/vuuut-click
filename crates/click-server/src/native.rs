//! Versioned native control plane and website-limited staff exchange.
//! Issuers and keys are pinned by an authenticated connector; JWT input never
//! chooses a URL to fetch. Neither CMS staff nor a native session is a Click user.
use crate::{auth, ApiError, AppResult, AppState};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    routing::{get, post, put},
    Json, Router,
};
use click_core::digest;
use click_store::Website;
use hmac::{Hmac, Mac};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/websites/{id}/native-report", get(report))
        .route("/native/v1/tenants/{tenant}", put(provision))
        .route("/native/v1/websites/{id}/events", post(events))
        .route("/native/v1/websites/{id}/connect", post(connect))
        .route("/native/v1/websites/{id}/exchange", post(exchange))
        .route("/native/v1/websites/{id}/rotate", post(rotate))
        .route("/native/v1/websites/{id}/disconnect", post(disconnect))
        .route("/native/v1/websites/{id}/verify", post(verify))
}
fn bearer(h: &HeaderMap) -> AppResult<&str> {
    h.get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|s| s.len() <= 256)
        .ok_or_else(ApiError::unauthorized)
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provision {
    pub revision: i64,
    pub name: String,
    pub origin: String,
    pub plan_id: String,
    pub state: String,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct Provisioned {
    pub website_id: Option<Uuid>,
    pub collection_id: Option<String>,
    pub integration_key: Option<String>,
    pub verification_token: Option<String>,
    pub verified: bool,
    pub revision: i64,
    pub state: String,
}
fn origin(raw: &str, local: bool) -> AppResult<url::Url> {
    let u = url::Url::parse(raw).map_err(|_| ApiError::bad("Use an HTTPS website origin"))?;
    if raw.len() > 255
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.path() != "/"
        || u.query().is_some()
        || u.fragment().is_some()
        || !(u.scheme() == "https"
            || local
                && u.scheme() == "http"
                && u.host_str().is_some_and(|h| {
                    matches!(h, "localhost" | "127.0.0.1" | "[::1]") || h.ends_with(".localhost")
                }))
    {
        return Err(ApiError::bad(
            "Use an HTTPS website origin without a path or credentials",
        ));
    }
    Ok(u)
}
fn derived_key(platform_token: &str, website: Uuid, revision: i64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(platform_token.as_bytes()).expect("HMAC key");
    mac.update(format!("vuuut-click/native/v1/{website}/{revision}").as_bytes());
    format!("vcn_{}", hex::encode(mac.finalize().into_bytes()))
}
async fn audit(tx: &mut sqlx::PgConnection, org: Uuid, site: Uuid, action: &str) -> AppResult<()> {
    sqlx::query("INSERT INTO audit_logs(organization_id,action,resource_id) VALUES($1,$2,$3)")
        .bind(org)
        .bind(action)
        .bind(site)
        .execute(tx)
        .await?;
    Ok(())
}
async fn revoke(tx: &mut sqlx::PgConnection, id: Uuid) -> AppResult<()> {
    sqlx::query("DELETE FROM native_sessions WHERE website_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE native_connections SET connected=false,generation=generation+1,updated_at=now() WHERE website_id=$1")
        .bind(id).execute(&mut *tx).await?;
    Ok(())
}
async fn provision(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(tenant): Path<String>,
    Json(b): Json<Provision>,
) -> AppResult<Json<Provisioned>> {
    let token = bearer(&h)?;
    if tenant.is_empty()
        || tenant.len() > 100
        || !tenant
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        || b.revision < 1
        || b.name.trim().is_empty()
        || b.name.len() > 100
        || !matches!(b.state.as_str(), "active" | "suspended" | "deleted")
    {
        return Err(ApiError::bad(
            "Invalid native tenant, revision, name or lifecycle state",
        ));
    }
    let u = origin(&b.origin, s.config.allow_localhost)?;
    let canonical = u.origin().ascii_serialization();
    let fingerprint = digest(&serde_json::to_string(&b).map_err(ApiError::internal)?);
    let mut tx = s.store.pg.begin().await?;
    let (platform, suffix): (String, String) = sqlx::query_as(
        "SELECT id,domain_suffix FROM native_platforms WHERE token_hash=$1 AND active FOR UPDATE",
    )
    .bind(digest(token))
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(ApiError::unauthorized)?;
    // Serializes first provisioning too, before a mapping row exists.
    let existing:Option<(Option<Uuid>,i64,String,String)>=sqlx::query_as("SELECT website_id,revision,request_hash,state FROM integration_mappings WHERE platform_id=$1 AND tenant_id=$2 FOR UPDATE")
        .bind(&platform).bind(&tenant).fetch_optional(&mut *tx).await?;
    if let Some((_, revision, hash, state)) = &existing {
        if b.revision < *revision
            || b.revision == *revision && fingerprint != *hash
            || state == "deleted" && b.state != "deleted"
        {
            return Err(ApiError::conflict(
                "Stale, conflicting or deleted native tenant revision",
            ));
        }
    }
    if b.state == "deleted" && existing.as_ref().is_none_or(|e| e.0.is_none()) {
        sqlx::query("INSERT INTO integration_mappings(platform_id,tenant_id,revision,request_hash,state) VALUES($1,$2,$3,$4,'deleted') ON CONFLICT(platform_id,tenant_id) DO UPDATE SET revision=EXCLUDED.revision,request_hash=EXCLUDED.request_hash")
            .bind(&platform).bind(&tenant).bind(b.revision).bind(&fingerprint).execute(&mut *tx).await?;
        tx.commit().await?;
        return Ok(Json(Provisioned {
            website_id: None,
            collection_id: None,
            integration_key: None,
            verification_token: None,
            verified: false,
            revision: b.revision,
            state: b.state,
        }));
    }
    let plan_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM plans WHERE id=$1)")
        .bind(&b.plan_id)
        .fetch_one(&mut *tx)
        .await?;
    if !plan_exists {
        return Err(ApiError::bad("Unknown Click entitlement plan"));
    }
    let id = if let Some(id) = existing.as_ref().and_then(|e| e.0) {
        id
    } else {
        let org = Uuid::new_v4();
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations(id,name) VALUES($1,$2)")
            .bind(org)
            .bind(b.name.trim())
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO subscriptions(organization_id,plan_id,provider,status) VALUES($1,$2,'native','active')").bind(org).bind(&b.plan_id).execute(&mut *tx).await?;
        let host = u.host_str().unwrap_or_default();
        let trusted = host == suffix || host.ends_with(&format!(".{suffix}"));
        sqlx::query("INSERT INTO websites(id,organization_id,name,origin,collection_id,verification_token,verified_at) VALUES($1,$2,$3,$4,$5,$6,CASE WHEN $7 THEN now() ELSE NULL END)")
            .bind(id).bind(org).bind(b.name.trim()).bind(&canonical).bind(format!("vc_{}",auth::secret())).bind(auth::secret()).bind(trusted).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO integration_mappings(platform_id,tenant_id,website_id) VALUES($1,$2,$3)",
        )
        .bind(&platform)
        .bind(&tenant)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        id
    };
    let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM websites WHERE id=$1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    let org_status: String =
        sqlx::query_scalar("SELECT status FROM organizations WHERE id=$1 FOR UPDATE")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    if org_status != "active" && b.state != "deleted" {
        return Err(ApiError::forbidden("Workspace is unavailable"));
    }
    let replay = existing.as_ref().is_some_and(|e| e.1 == b.revision);
    let key = derived_key(token, id, b.revision);
    if !replay {
        revoke(&mut tx, id).await?;
        sqlx::query("UPDATE api_keys SET revoked_at=now() WHERE website_id=$1 AND label='Native connector' AND revoked_at IS NULL").bind(id).execute(&mut *tx).await?;
        let host = u.host_str().unwrap_or_default();
        let trusted = host == suffix || host.ends_with(&format!(".{suffix}"));
        sqlx::query("UPDATE websites SET name=$2,origin=$3,verified_at=CASE WHEN origin=$3 THEN verified_at WHEN $4 THEN now() ELSE NULL END,verification_token=CASE WHEN origin=$3 THEN verification_token ELSE $5 END,active=$6 WHERE id=$1")
            .bind(id).bind(b.name.trim()).bind(&canonical).bind(trusted).bind(auth::secret()).bind(b.state=="active").execute(&mut *tx).await?;
        sqlx::query("UPDATE subscriptions SET plan_id=$2,status=$3 WHERE organization_id=$1 AND provider='native'")
            .bind(org).bind(&b.plan_id).bind(if b.state=="active" {"active"} else {"suspended"}).execute(&mut *tx).await?;
        if b.state != "deleted" {
            sqlx::query("INSERT INTO api_keys(id,website_id,token_hash,label,scope) VALUES($1,$2,$3,'Native connector','integration')")
                .bind(Uuid::new_v4()).bind(id).bind(digest(&key)).execute(&mut *tx).await?;
        } else {
            sqlx::query("UPDATE websites SET active=false,deletion_requested_at=now() WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE api_keys SET revoked_at=now() WHERE website_id=$1 AND revoked_at IS NULL",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO deletion_jobs(id,organization_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
                .bind(Uuid::new_v4()).bind(org).execute(&mut *tx).await?;
            sqlx::query("UPDATE organizations SET status='deleting' WHERE id=$1")
                .bind(org)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE integration_mappings SET revision=$3,request_hash=$4,state=$5 WHERE platform_id=$1 AND tenant_id=$2")
            .bind(&platform).bind(&tenant).bind(b.revision).bind(&fingerprint).bind(&b.state).execute(&mut *tx).await?;
        audit(&mut tx, org, id, &format!("native.{}", b.state)).await?;
    }
    let (collection, verification, verified): (String, String, bool) = sqlx::query_as(
        "SELECT collection_id,verification_token,verified_at IS NOT NULL FROM websites WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(Provisioned {
        website_id: Some(id),
        collection_id: Some(collection),
        integration_key: (b.state != "deleted").then_some(key),
        verification_token: Some(verification),
        verified,
        revision: b.revision,
        state: b.state,
    }))
}

/// Signed native deliveries authenticate the exact bytes and a bounded send
/// time. The original event time is separate and remains stable on retry.
async fn events(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> AppResult<(StatusCode, Json<Value>)> {
    let token = bearer(&h)?;
    let signature = h
        .get("x-click-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(ApiError::unauthorized)?;
    let (time, tag) = signature
        .split_once(',')
        .ok_or_else(ApiError::unauthorized)?;
    let time = time
        .strip_prefix("t=")
        .and_then(|v| v.parse::<i64>().ok())
        .ok_or_else(ApiError::unauthorized)?;
    let now = chrono::Utc::now().timestamp();
    if time < now - 300 || time > now + 60 || body.len() > 16384 {
        return Err(ApiError::unauthorized());
    }
    let tag = tag
        .strip_prefix("v1=")
        .and_then(|v| hex::decode(v).ok())
        .filter(|v| v.len() == 32)
        .ok_or_else(ApiError::unauthorized)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC key");
    mac.update(format!("{time}.").as_bytes());
    mac.update(&body);
    mac.verify_slice(&tag)
        .map_err(|_| ApiError::unauthorized())?;
    let event = serde_json::from_slice::<crate::analytics::Conversion>(&body)
        .map_err(|_| ApiError::bad("Invalid native backend event"))?;
    crate::analytics::backend_event(State(s), h, Path(id), Json(event)).await
}

/// Recheck the integration credential under the workspace lock in every write.
async fn lock_connector(
    s: &AppState,
    tx: &mut sqlx::PgConnection,
    h: &HeaderMap,
    id: Uuid,
) -> AppResult<(Website, Uuid)> {
    let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM websites WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let active: bool =
        sqlx::query_scalar("SELECT status='active' FROM organizations WHERE id=$1 FOR UPDATE")
            .bind(org)
            .fetch_one(&mut *tx)
            .await?;
    if !active {
        return Err(ApiError::not_found());
    }
    let key_id:Uuid=sqlx::query_scalar("SELECT k.id FROM api_keys k JOIN websites w ON w.id=k.website_id WHERE w.id=$1 AND k.token_hash=$2 AND k.scope='integration' AND k.revoked_at IS NULL AND w.active AND NOT w.is_demo AND w.deletion_requested_at IS NULL AND NOT EXISTS(SELECT 1 FROM integration_mappings m JOIN native_platforms p ON p.id=m.platform_id WHERE m.website_id=w.id AND NOT p.active)")
        .bind(id).bind(digest(bearer(h)?)).fetch_optional(&mut *tx).await?.ok_or_else(ApiError::not_found)?;
    let site:Website=sqlx::query_as("SELECT id,organization_id,name,origin,collection_id,verification_token,verified_at,active,is_demo FROM websites WHERE id=$1")
        .bind(id).fetch_one(&mut *tx).await?;
    let _ = s;
    Ok((site, key_id))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connect {
    pub issuer: String,
    pub jwks: Value,
}
fn validate_keys(jwks: &Value) -> AppResult<jsonwebtoken::jwk::JwkSet> {
    if serde_json::to_vec(jwks).map_err(ApiError::internal)?.len() > 16000 {
        return Err(ApiError::bad("Signing key set is too large"));
    }
    if jwks
        .get("keys")
        .and_then(Value::as_array)
        .is_some_and(|keys| {
            keys.iter().any(|key| {
                ["d", "p", "q", "dp", "dq", "qi", "oth", "k"]
                    .iter()
                    .any(|field| key.get(*field).is_some())
            })
        })
    {
        return Err(ApiError::bad("Register public signing keys only"));
    }
    let set: jsonwebtoken::jwk::JwkSet = serde_json::from_value(jwks.clone())
        .map_err(|_| ApiError::bad("Invalid signing key set"))?;
    if set.keys.is_empty() || set.keys.len() > 4 {
        return Err(ApiError::bad("Pin between one and four RSA signing keys"));
    }
    let mut kids = std::collections::HashSet::new();
    for key in &set.keys {
        let kid = key
            .common
            .key_id
            .as_deref()
            .filter(|k| !k.is_empty() && k.len() <= 100)
            .ok_or_else(|| ApiError::bad("Signing key needs a unique kid"))?;
        if !kids.insert(kid)
            || key
                .common
                .key_algorithm
                .is_some_and(|a| a != jsonwebtoken::jwk::KeyAlgorithm::RS256)
            || !matches!(&key.algorithm,jsonwebtoken::jwk::AlgorithmParameters::RSA(rsa) if rsa.n.len()>=342 && rsa.n.len()<=1366)
            || DecodingKey::from_jwk(key).is_err()
        {
            return Err(ApiError::bad(
                "Use distinct RS256 RSA signing keys of at least 2048 bits",
            ));
        }
    }
    Ok(set)
}
async fn connect(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Connect>,
) -> AppResult<Json<Value>> {
    let jwks = serde_json::to_value(validate_keys(&b.jwks)?).map_err(ApiError::internal)?;
    let mut tx = s.store.pg.begin().await?;
    let (site, key) = lock_connector(&s, &mut tx, &h, id).await?;
    if b.issuer != format!("{}/ghost/api/admin/", site.origin) || site.verified_at.is_none() {
        return Err(ApiError::forbidden(
            "Verify the website origin before connecting its staff issuer",
        ));
    }
    // Identical retries must not revoke in-flight staff requests.
    let changed:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM native_connections WHERE website_id=$1 AND key_id=$2 AND issuer=$3 AND jwks=$4 AND connected)")
        .bind(id).bind(key).bind(&b.issuer).bind(&jwks).fetch_one(&mut *tx).await?;
    if changed {
        revoke(&mut tx, id).await?;
        sqlx::query("INSERT INTO native_connections(website_id,key_id,issuer,jwks) VALUES($1,$2,$3,$4) ON CONFLICT(website_id) DO UPDATE SET key_id=EXCLUDED.key_id,issuer=EXCLUDED.issuer,jwks=EXCLUDED.jwks,connected=true,updated_at=now()")
            .bind(id).bind(key).bind(&b.issuer).bind(&jwks).execute(&mut *tx).await?;
        audit(&mut tx, site.organization_id, id, "native.connected").await?;
    }
    tx.commit().await?;
    Ok(Json(
        json!({"website_id":id,"collection_id":site.collection_id,"audience":format!("{}/native/v1",s.config.public_url)}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    pub token: String,
}
#[derive(Clone, Deserialize)]
struct Claims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    website_id: Uuid,
    role: String,
}
async fn exchange(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Json(b): Json<Exchange>,
) -> AppResult<Json<Value>> {
    if b.token.len() > 8192 {
        return Err(ApiError::unauthorized());
    }
    let mut tx = s.store.pg.begin().await?;
    let (site, key) = lock_connector(&s, &mut tx, &h, id).await?;
    let (issuer,jwks,generation):(String,Value,i64)=sqlx::query_as("SELECT issuer,jwks,generation FROM native_connections WHERE website_id=$1 AND key_id=$2 AND connected FOR UPDATE")
        .bind(id).bind(key).fetch_optional(&mut *tx).await?.ok_or_else(ApiError::unauthorized)?;
    if site.verified_at.is_none() || issuer != format!("{}/ghost/api/admin/", site.origin) {
        return Err(ApiError::unauthorized());
    }
    let header = jsonwebtoken::decode_header(&b.token).map_err(|_| ApiError::unauthorized())?;
    if header.alg != Algorithm::RS256 {
        return Err(ApiError::unauthorized());
    }
    let set = validate_keys(&jwks)?;
    let jwk = header
        .kid
        .as_deref()
        .and_then(|kid| set.find(kid))
        .ok_or_else(ApiError::unauthorized)?;
    let mut validation = Validation::new(Algorithm::RS256);
    let audience = format!("{}/native/v1", s.config.public_url);
    validation.set_audience(&[&audience]);
    validation.set_issuer(&[&issuer]);
    validation.leeway = 0;
    validation.validate_nbf = true;
    validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub", "jti"]);
    let claims = jsonwebtoken::decode::<Claims>(
        &b.token,
        &DecodingKey::from_jwk(jwk).map_err(|_| ApiError::unauthorized())?,
        &validation,
    )
    .map_err(|_| ApiError::unauthorized())?
    .claims;
    let now = chrono::Utc::now().timestamp();
    if claims.website_id != id
        || claims.iss != issuer
        || claims.aud != audience
        || claims.iat > now + 30
        || claims.iat < now - 300
        || claims.exp <= now
        || claims.exp - claims.iat > 300
        || claims.sub.is_empty()
        || claims.sub.len() > 100
        || claims.jti.is_empty()
        || claims.jti.len() > 100
        || !matches!(claims.role.as_str(), "Owner" | "Administrator")
    {
        return Err(ApiError::unauthorized());
    }
    sqlx::query("DELETE FROM native_assertions WHERE expires_at<=now()")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM native_sessions WHERE expires_at<=now()")
        .execute(&mut *tx)
        .await?;
    let inserted=sqlx::query("INSERT INTO native_assertions(website_id,jti,expires_at) VALUES($1,$2,to_timestamp($3)) ON CONFLICT DO NOTHING")
        .bind(id).bind(&claims.jti).bind(claims.exp as f64).execute(&mut *tx).await?;
    if inserted.rows_affected() != 1 {
        return Err(ApiError::unauthorized());
    }
    let token = auth::secret();
    sqlx::query("INSERT INTO native_sessions(token_hash,website_id,generation,expires_at) VALUES($1,$2,$3,now()+interval '60 seconds')")
        .bind(digest(&token)).bind(id).bind(generation).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"access_token":token,"token_type":"Bearer","expires_in":60,"website_id":id}),
    ))
}
async fn disconnect(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    // Confirm an already committed disconnect after a lost response. A
    // revoked credential can confirm only its own disconnected connection;
    // it cannot read data, reconnect, rotate or change any other state.
    let done:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM native_connections c JOIN api_keys k ON k.id=c.key_id WHERE c.website_id=$1 AND k.token_hash=$2 AND k.revoked_at IS NOT NULL AND NOT c.connected)")
        .bind(id).bind(digest(bearer(&h)?)).fetch_one(&s.store.pg).await?;
    if done {
        return Ok(StatusCode::NO_CONTENT);
    }
    let mut tx = s.store.pg.begin().await?;
    let (site, key) = lock_connector(&s, &mut tx, &h, id).await?;
    revoke(&mut tx, id).await?;
    sqlx::query("UPDATE api_keys SET revoked_at=now() WHERE id=$1")
        .bind(key)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, site.organization_id, id, "native.disconnected").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn rotate(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let mut tx = s.store.pg.begin().await?;
    let (site, key) = lock_connector(&s, &mut tx, &h, id).await?;
    // Caller supplies the replacement, so a lost response is recoverable:
    // try the new key, then the old key; never create an unknowable secret.
    let new = h
        .get("x-click-replacement-key")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() == 64 && v.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| ApiError::bad("Supply a random 32-byte hex replacement key"))?;
    if new == bearer(&h)? {
        return Err(ApiError::bad("Choose a new replacement key"));
    }
    revoke(&mut tx, id).await?;
    sqlx::query("UPDATE api_keys SET token_hash=$2 WHERE id=$1")
        .bind(key)
        .bind(digest(new))
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, site.organization_id, id, "native.rotated").await?;
    tx.commit().await?;
    Ok(Json(json!({"website_id":id})))
}
async fn verify(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let mut tx = s.store.pg.begin().await?;
    let (site, _) = lock_connector(&s, &mut tx, &h, id).await?;
    tx.rollback().await?;
    crate::api::check_website_dns(&site).await?;
    let mut tx = s.store.pg.begin().await?;
    let (current, _) = lock_connector(&s, &mut tx, &h, id).await?;
    if current.origin != site.origin || current.verification_token != site.verification_token {
        return Err(ApiError::conflict("Origin changed during verification"));
    }
    sqlx::query("UPDATE websites SET verified_at=now() WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, site.organization_id, id, "native.verified").await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The native bearer only authorizes one site's read APIs; no customer cookie
/// or workspace membership is minted. Revocation and suspension are immediate.
pub async fn authorized_site(s: &AppState, token: &str, id: Uuid) -> AppResult<Option<Website>> {
    Ok(sqlx::query_as("SELECT w.id,w.organization_id,w.name,w.origin,w.collection_id,w.verification_token,w.verified_at,w.active,w.is_demo FROM native_sessions ns JOIN native_connections nc ON nc.website_id=ns.website_id JOIN api_keys k ON k.id=nc.key_id JOIN websites w ON w.id=ns.website_id JOIN organizations o ON o.id=w.organization_id WHERE ns.token_hash=$1 AND ns.website_id=$2 AND ns.expires_at>now() AND ns.generation=nc.generation AND nc.connected AND k.revoked_at IS NULL AND w.active AND w.deletion_requested_at IS NULL AND w.verified_at IS NOT NULL AND nc.issuer=w.origin || '/ghost/api/admin/' AND o.status='active' AND NOT EXISTS(SELECT 1 FROM integration_mappings m JOIN native_platforms p ON p.id=m.platform_id WHERE m.website_id=w.id AND NOT p.active)")
        .bind(digest(token)).bind(id).fetch_optional(&s.store.pg).await?)
}

#[derive(Deserialize)]
struct ReportQuery {
    days: Option<u32>,
    path: Option<String>,
}
#[derive(Serialize)]
struct NativeReport {
    activity: Vec<click_store::Breakdown>,
    page: Option<PageReport>,
}
#[derive(Serialize)]
struct PageReport {
    path: String,
    pageviews: u64,
    visitors: u64,
    sessions: u64,
}
/// Basic CMS reports use exact authoritative event counts, independently of
/// the general top-100 event breakdown. CMS slash aliases are one post.
async fn report(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<ReportQuery>,
) -> AppResult<Json<NativeReport>> {
    let (site, _, _permit) = auth::authorize_report(&s, &h, id).await?;
    let (start, end) = crate::api::range(&crate::api::Period { days: q.days })?;
    if q.path.as_ref().is_some_and(|p| {
        !p.starts_with('/')
            || p.starts_with("//")
            || p.len() > 1024
            || p.contains(['?', '#'])
            || p.chars().any(char::is_control)
    }) {
        return Err(ApiError::bad(
            "Use a local post path without query or fragment",
        ));
    }
    let activity=s.store.ch.query("SELECT name AS label,uniqExact(event_id) AS count FROM events FINAL WHERE organization_id=? AND website_id=? AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) AND name IN ('server.member_signup','server.subscription_started','server.subscription_cancelled','server.newsletter_sent','server.newsletter_open','server.newsletter_click','server.payment','server.refund') GROUP BY name ORDER BY name")
        .bind(site.organization_id.to_string()).bind(id.to_string()).bind(start).bind(end).fetch_all::<click_store::Breakdown>().await.map_err(ApiError::internal)?;
    let page = if let Some(path) = q.path {
        let bare = path.trim_end_matches('/');
        let totals=s.store.ch.query("SELECT uniqExact(event_id) AS pageviews,uniqExact(visitor_id) AS visitors,uniqExact(session_id) AS sessions FROM events FINAL WHERE organization_id=? AND website_id=? AND name='pageview' AND timestamp>=toDateTime(?) AND timestamp<toDateTime(?) AND path IN (?,?)")
            .bind(site.organization_id.to_string()).bind(id.to_string()).bind(start).bind(end).bind(bare).bind(format!("{bare}/"))
            .fetch_one::<click_store::Totals>().await.map_err(ApiError::internal)?;
        Some(PageReport {
            path,
            pageviews: totals.pageviews,
            visitors: totals.visitors,
            sessions: totals.sessions,
        })
    } else {
        None
    };
    Ok(Json(NativeReport { activity, page }))
}
