//! All three real adapters, HTTP Click and real PostgreSQL/ClickHouse, with
//! isolated CMS/platform SQLite stores. No remote API mocks.
#[path = "../../../crates/click-server/tests/support/mod.rs"]
mod support;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
};
use support::Harness;
use tokio::sync::RwLock;
use tower::ServiceExt;
use uuid::Uuid;

async fn cms(origin: &str, host: Value) -> (ghost_api::AppState, String) {
    let db = ghost_db::connect_in_memory().await.unwrap();
    ghost_db::seed::seed_if_empty(&db).await.unwrap();
    let owner =
        ghost_db::repos::users::setup_owner(&db, "Native owner", "owner@native.test", "test-hash")
            .await
            .unwrap();
    let session = ghost_db::repos::sessions::create(&db, &owner.id)
        .await
        .unwrap();
    let settings = Arc::new(ghost_db::SettingsCache::from_values([
        ("site_url".into(), json!(origin)),
        ("web_analytics".into(), json!(true)),
        ("title".into(), json!("Native publication")),
    ]));
    let mut config:ghost_core::Config=serde_json::from_value(json!({"url":origin,"server":{},"database":{"connection":{"filename":":memory:"}},"paths":{},"hostSettings":host})).unwrap();
    config.env = "development".into();
    let routes = ghost_routing::routes_yaml::parse_routes_yaml(
        ghost_routing::routes_yaml::DEFAULT_ROUTES_YAML,
    )
    .unwrap();
    let router = Arc::new(ghost_routing::RouterTree::build(&routes).unwrap());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../Vuuut/content/themes/source");
    let deps = ghost_render::HelperDeps {
        pool: Arc::new(db.clone()),
        runtime: tokio::runtime::Handle::current(),
        url_resolver: ghost_api::themes::url_resolver(router.clone()),
        asset_hashes: Arc::new(HashMap::new()),
        global_asset_hash: "native-test".into(),
        theme_dir: dir.clone(),
        site_origin: origin.into(),
        content_api_key: String::new(),
    };
    let theme = ghost_render::load_and_activate(&dir, &deps, HashMap::new()).unwrap();
    (
        ghost_api::AppState {
            db,
            config: Arc::new(config),
            settings,
            theme: Arc::new(RwLock::new(Arc::new(theme))),
            router: Arc::new(RwLock::new(router)),
            redirects: Arc::new(RwLock::new(Arc::new(
                ghost_routing::redirects::Redirects::default(),
            ))),
            setup_done: Arc::new(AtomicBool::new(true)),
            frontend_key: Arc::new(RwLock::new(None)),
            limits: Arc::new(ghost_db::limits::Limits::load(&host)),
        },
        format!("ghost-admin-api-session={session}"),
    )
}
async fn member(db: &ghost_db::Db, id: &str) {
    sqlx::query("INSERT INTO members(id,uuid,transient_id,email,status,created_at) VALUES(?,?,?,?,'free',?)")
        .bind(id).bind(Uuid::new_v4().to_string()).bind(Uuid::new_v4().to_string()).bind(format!("{id}@native.test")).bind(ghost_core::dates::now_db()).execute(db).await.unwrap();
}
async fn ui(state: &ghost_api::AppState, cookie: &str) -> (axum::http::StatusCode, String) {
    let app = axum::Router::new()
        .nest("/ghost", ghost_admin_ui::admin_ui_router(state.clone()))
        .with_state(state.clone());
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/ghost/analytics/click/")
                .header("cookie", cookie)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 10_000_000)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse; all three sibling repositories"]
async fn actual_platform_cms_and_click_adapters_work_together() {
    let h = Harness::new().await;
    let platform_key = format!("native-bridge-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO native_platforms(id,token_hash,domain_suffix) VALUES('bridge',$1,'localhost')",
    )
    .bind(click_core::digest(&platform_key))
    .execute(&h.store.pg)
    .await
    .unwrap();
    let root = std::env::temp_dir().join(format!("vuuut-native-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let mut config:pro_core::Config=serde_json::from_value(json!({"url":"http://pro.localhost:8080","subdomainBase":"localhost","server":{"port":8080},"paths":{"ghostBinary":"/usr/bin/true","dataDir":root},"tenants":{"env":"development"},"click":{"baseUrl":h.base,"platformKey":platform_key,"consentRequired":false,"planMap":{"creator":"pro"}}})).unwrap();
    config.env = "development".into();
    config.base_dir = root.clone();
    let config = Arc::new(config);
    let supervisor = pro_supervisor::Supervisor::new(config.clone()).unwrap();
    let db = pro_db::open_in_memory().await.unwrap();
    let account = pro_db::accounts::create(&db, "native@test.local", "Native", "unused")
        .await
        .unwrap();
    let tenant = pro_db::tenants::create(
        &db,
        &account.id,
        "bridge",
        "Native publication",
        "creator",
        &pro_core::now_iso(),
        25000,
        26000,
    )
    .await
    .unwrap();
    pro_db::domains::add(
        &db,
        &tenant.id,
        "bridge.localhost",
        pro_core::DomainKind::Subdomain,
        true,
    )
    .await
    .unwrap();
    let dir = config.tenant_dir(&tenant.id);
    std::fs::create_dir_all(&dir).unwrap();
    pro_provision::config_file::write(
        &dir,
        "development",
        &json!({"url":"http://bridge.localhost:8080","hostSettings":{}}),
    )
    .await
    .unwrap();
    pro_db::tenants::set_status(&db, &tenant.id, pro_core::TenantStatus::Active)
        .await
        .unwrap();
    assert!(pro_provision::click::sync_once(&db, &config, &supervisor)
        .await
        .unwrap());
    let synced = pro_provision::config_file::read(&dir, "development")
        .await
        .unwrap();
    let c = &synced["hostSettings"]["click"];
    assert!(c["integrationKey"].as_str().is_some());
    assert!(!synced.to_string().contains(&platform_key));
    let website = Uuid::parse_str(c["websiteId"].as_str().unwrap()).unwrap();
    let (state, cookie) = cms(
        "http://bridge.localhost:8080",
        synced["hostSettings"].clone(),
    )
    .await;
    // Boot activates source capture; pre-connection history is not backfilled.
    // Old standalone settings cannot override explicit managed credentials.
    let stale = json!({"baseUrl":h.base,"websiteId":Uuid::new_v4().to_string(),"integrationKey":click_server::auth::secret(),"collectionId":"old-public-id"});
    sqlx::query("UPDATE click_connection SET config=? WHERE id=1")
        .bind(stale.to_string())
        .execute(&state.db)
        .await
        .unwrap();
    // A downloadable backup keeps hosted disable policy while removing its
    // key, so restoring it cannot activate the stale standalone record.
    let archive = root.join("native-backup.zip");
    let unpacked = root.join("unpacked-native");
    pro_provision::pack(&dir, &archive, None).await.unwrap();
    pro_provision::unpack(&archive, &unpacked).await.unwrap();
    let backup_config = pro_provision::config_file::read(&unpacked, "development")
        .await
        .unwrap();
    assert_eq!(backup_config["hostSettings"]["click"]["disabled"], true);
    assert!(!backup_config
        .to_string()
        .contains(c["integrationKey"].as_str().unwrap()));
    let mut backup_state = state.clone();
    let mut backup_policy = (*state.config).clone();
    backup_policy.host_settings = backup_config["hostSettings"].clone();
    backup_state.config = Arc::new(backup_policy);
    assert!(ghost_api::click::connection(&backup_state)
        .await
        .unwrap()
        .is_none());

    assert_eq!(ghost_api::click::drain_once(&state).await.unwrap(), 0);
    member(&state.db, "member-1").await;
    ghost_api::member_events::record_payment_once(
        &state,
        "member-1",
        2000,
        Some("usd"),
        "stripe",
        "invoice-one",
    )
    .await
    .unwrap();
    ghost_api::member_events::record_payment_once(
        &state,
        "member-1",
        2000,
        Some("usd"),
        "stripe",
        "invoice-one",
    )
    .await
    .unwrap();
    ghost_api::member_events::record_payment_once(
        &state,
        "member-1",
        -500,
        Some("usd"),
        "stripe",
        "refund-one",
    )
    .await
    .unwrap();
    // Rolled-back member mutations must not emit accepted analytics.
    let mut tx = state.db.begin().await.unwrap();
    sqlx::query("INSERT INTO members(id,uuid,transient_id,email,created_at) VALUES('rollback','rollback','rollback','rollback@native.test',?)").bind(ghost_core::dates::now_db()).execute(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM click_outbox")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        3
    );
    assert_eq!(ghost_api::click::drain_once(&state).await.unwrap(), 3);
    assert_eq!(ghost_api::click::drain_once(&state).await.unwrap(), 0);
    // Reintroduce the same delivery after acceptance to model a lost ack.
    sqlx::query("INSERT INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at) SELECT 'payment:' || id,'payment',amount,upper(currency),created_at,'' FROM members_payment_events WHERE amount>0").execute(&state.db).await.unwrap();
    assert_eq!(ghost_api::click::drain_once(&state).await.unwrap(), 1);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT sum(accepted_events)::bigint FROM usage_records WHERE organization_id=(SELECT organization_id FROM websites WHERE id=$1)").bind(website).fetch_one(&h.store.pg).await.unwrap(),3);
    h.store.drain_once().await.unwrap();
    let report = ghost_api::click::report(&state, "native-staff", "Owner", 30)
        .await
        .unwrap();
    assert_eq!(report["revenue"][0]["total_minor"], "1500");
    assert!(report["native"]["activity"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["label"] == "server.member_signup" && r["count"] == 1));

    let (status, html) = ui(&state, &cookie).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(html.contains("Net revenue by source") && html.contains("USD 15.00"));
    assert!(
        html.contains("Member growth and subscriptions")
            && html.contains("Newsletter performance")
            && html.contains("Current CMS membership")
    );
    assert!(!html.contains(c["integrationKey"].as_str().unwrap()));
    assert!(!html.contains(&platform_key));
    sqlx::query("INSERT INTO posts(id,uuid,title,slug,status,type,created_at) VALUES('native-post','native-post','Native post','native-post','published','post',?)").bind(ghost_core::dates::now_db()).execute(&state.db).await.unwrap();
    let post_app = axum::Router::new()
        .nest("/ghost", ghost_admin_ui::admin_ui_router(state.clone()))
        .with_state(state.clone());
    let response = post_app
        .oneshot(
            axum::http::Request::builder()
                .uri("/ghost/posts/native-post/analytics/")
                .header("cookie", &cookie)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let post_html = String::from_utf8(
        axum::body::to_bytes(response.into_body(), 10_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(post_html.contains("Vuuut Click post report"));

    let front = ghost_front::site_router(state.clone());
    let response = front
        .oneshot(
            axum::http::Request::builder()
                .uri("/")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = String::from_utf8(
        axum::body::to_bytes(response.into_body(), 10_000_000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        html.contains(c["collectionId"].as_str().unwrap())
            && html.contains("data-consent=\"anonymous\""),
        "tracker absent from theme"
    );
    assert!(!html.contains(c["integrationKey"].as_str().unwrap()));
    // A platform outage is persisted and leaves the last working tenant config.
    let mut broken = (*config).clone();
    broken.click["baseUrl"] = json!("http://127.0.0.1:1");
    let broken = Arc::new(broken);
    pro_db::tenants::set_plan(&db, &tenant.id, "starter")
        .await
        .unwrap();
    pro_provision::click::sync_once(&db, &broken, &supervisor)
        .await
        .unwrap();
    let (attempts, status): (i64, String) =
        sqlx::query_as("SELECT attempts,state FROM click_sync WHERE tenant_id=?")
            .bind(&tenant.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(attempts, 1);
    assert_eq!(status, "pending");
    pro_provision::click::action(&db, &tenant.id, "retry")
        .await
        .unwrap();
    pro_provision::click::sync_once(&db, &config, &supervisor)
        .await
        .unwrap();
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT plan_id FROM subscriptions WHERE organization_id=(SELECT organization_id FROM websites WHERE id=$1)").bind(website).fetch_one(&h.store.pg).await.unwrap(),"free");
    // A hosted suspension revokes the previous CMS key and pauses collection.
    pro_db::tenants::suspend(&db, &tenant.id, "test")
        .await
        .unwrap();
    pro_provision::click::sync_once(&db, &config, &supervisor)
        .await
        .unwrap();
    assert!(ghost_api::click::report(&state, "native-staff", "Owner", 7)
        .await
        .is_err());
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT active FROM websites WHERE id=$1")
            .bind(website)
            .fetch_one(&h.store.pg)
            .await
            .unwrap()
    );
    let disabled = pro_provision::config_file::read(&dir, "development")
        .await
        .unwrap();
    assert_eq!(disabled["hostSettings"]["click"]["disabled"], true);
    let mut disabled_state = state.clone();
    let mut disabled_config = (*state.config).clone();
    disabled_config.host_settings = disabled["hostSettings"].clone();
    disabled_state.config = Arc::new(disabled_config);
    assert!(ghost_api::click::connection(&disabled_state)
        .await
        .unwrap()
        .is_none());
    let (_, disabled_html) = ui(&disabled_state, &cookie).await;
    assert!(disabled_html.contains("Your hosting platform manages this connection"));
    assert!(!disabled_html.contains("name=\"integration_key\""));

    // Lifecycle trigger work disappears with a rolled-back hosting change.
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM click_sync WHERE tenant_id=?")
        .bind(&tenant.id)
        .fetch_one(&db)
        .await
        .unwrap();
    let mut tx = db.begin().await.unwrap();
    sqlx::query("UPDATE tenants SET status='active' WHERE id=?")
        .bind(&tenant.id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT revision FROM click_sync WHERE tenant_id=?")
            .bind(&tenant.id)
            .fetch_one(&db)
            .await
            .unwrap(),
        revision
    );
    pro_db::tenants::mark_deleted(&db, &tenant.id)
        .await
        .unwrap();
    pro_provision::click::sync_once(&db, &config, &supervisor)
        .await
        .unwrap();
    click_server::jobs::process_deletion(&h.store)
        .await
        .unwrap();
    assert!(sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT website_id FROM integration_mappings WHERE tenant_id=$1"
    )
    .bind(&tenant.id)
    .fetch_one(&h.store.pg)
    .await
    .unwrap()
    .is_none());
    // Restoring after actual erasure creates a new mapping; the original
    // tombstone still cannot resurrect its historical events or credentials.
    member(&state.db, "queued-before-restore").await;
    assert!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM click_outbox")
            .fetch_one(&state.db)
            .await
            .unwrap()
            > 0
    );
    pro_db::tenants::clear_deletion(&db, &tenant.id)
        .await
        .unwrap();
    pro_provision::click::sync_once(&db, &config, &supervisor)
        .await
        .unwrap();
    let restored = pro_provision::config_file::read(&dir, "development")
        .await
        .unwrap();
    let new_id = Uuid::parse_str(
        restored["hostSettings"]["click"]["websiteId"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_ne!(new_id, website);
    let mut restored_state = state.clone();
    let mut restored_config = (*state.config).clone();
    restored_config.host_settings = restored["hostSettings"].clone();
    restored_state.config = Arc::new(restored_config);
    assert_eq!(
        ghost_api::click::drain_once(&restored_state).await.unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM click_outbox")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM event_outbox WHERE website_id=$1")
            .bind(new_id)
            .fetch_one(&h.store.pg)
            .await
            .unwrap(),
        0
    );
    // Local file/restart failures are durable failures too, with backoff and
    // an operator-visible recovery path after remote provisioning committed.
    let file = dir.join("config.development.json");
    let saved = dir.join("saved-config.json");
    std::fs::rename(&file, &saved).unwrap();
    pro_provision::click::action(&db, &tenant.id, "rotate")
        .await
        .unwrap();
    pro_provision::click::sync_once(&db, &config, &supervisor)
        .await
        .unwrap();
    let (attempts, error): (i64, Option<String>) =
        sqlx::query_as("SELECT attempts,last_error FROM click_sync WHERE tenant_id=?")
            .bind(&tenant.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(attempts, 1);
    assert!(error.unwrap().contains("configuration"));
    std::fs::rename(&saved, &file).unwrap();
    pro_provision::click::action(&db, &tenant.id, "retry")
        .await
        .unwrap();
    pro_provision::click::sync_once(&db, &config, &supervisor)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM click_sync WHERE tenant_id=?")
            .bind(&tenant.id)
            .fetch_one(&db)
            .await
            .unwrap(),
        "synced"
    );
    state.db.close().await;
    db.close().await;
    std::fs::remove_dir_all(root).unwrap();
    h.finish().await;
}

async fn form(
    c: &reqwest::Client,
    url: &str,
    cookie: &str,
    origin: &str,
    fields: &[(&str, &str)],
) -> reqwest::Response {
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish();
    c.post(url)
        .header("cookie", cookie)
        .header("origin", origin)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(encoded)
        .send()
        .await
        .unwrap()
}
async fn browser(origin: &str, cookie: &str, mode: &str) {
    if std::env::var("NATIVE_TEST_BROWSER").as_deref() != Ok("1") {
        return;
    }
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("browser.mjs");
    let screenshots = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-results");
    let status = tokio::process::Command::new("node")
        .arg(script)
        .arg(mode)
        .env("NATIVE_TEST_CMS", origin)
        .env(
            "NATIVE_TEST_COOKIE",
            cookie.trim_start_matches("ghost-admin-api-session="),
        )
        .env("NATIVE_TEST_SCREENSHOTS", screenshots)
        .status()
        .await
        .unwrap();
    assert!(status.success(), "native browser workflow failed");
}
#[tokio::test]
#[ignore = "requires development PostgreSQL/ClickHouse; optional NATIVE_TEST_BROWSER=1"]
async fn standalone_native_admin_consent_rotation_disconnect_and_privilege_boundaries() {
    let h = Harness::new().await;
    let c = h.client();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    h.register(&c, "native-standalone@test.local").await;
    let org = h.org(&c).await;
    let site: Value = h
        .post(&c, "/v1/websites")
        .json(&json!({"organization_id":org,"name":"Native standalone","origin":origin}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = Uuid::parse_str(site["id"].as_str().unwrap()).unwrap();
    let key = click_server::auth::secret();
    sqlx::query("INSERT INTO api_keys(id,website_id,token_hash,label,scope) VALUES($1,$2,$3,'standalone test','integration')").bind(Uuid::new_v4()).bind(id).bind(click_core::digest(&key)).execute(&h.store.pg).await.unwrap();
    let (state, cookie) = cms(&origin, Value::Null).await;
    let admin = axum::Router::new()
        .nest("/ghost", ghost_admin_ui::admin_ui_router(state.clone()))
        .with_state(state.clone());
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            admin.merge(ghost_front::site_router(server_state)),
        )
        .await
        .unwrap()
    });
    let url = format!("{origin}/ghost/analytics/click/");
    let no_origin = c
        .post(&url)
        .header("cookie", &cookie)
        .header("content-type", "application/x-www-form-urlencoded")
        .body("action=connect")
        .send()
        .await
        .unwrap();
    assert_eq!(no_origin.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        form(
            &c,
            &url,
            &cookie,
            "https://attacker.test",
            &[("action", "connect")]
        )
        .await
        .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    let response = form(
        &c,
        &url,
        &cookie,
        &origin,
        &[
            ("action", "connect"),
            ("base_url", &h.base),
            ("website_id", site["id"].as_str().unwrap()),
            ("integration_key", &key),
            ("consent_required", "1"),
        ],
    )
    .await;
    assert_eq!(response.status(), reqwest::StatusCode::SEE_OTHER);
    let configured = ghost_api::click::connection(&state).await.unwrap().unwrap();
    assert_eq!(
        configured.collection_id,
        site["collection_id"].as_str().unwrap()
    );
    assert!(configured.consent_required);
    let html = c
        .get(&url)
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("Connected to"));
    assert!(!html.contains(&key));
    // A less privileged staff account cannot read the native report or change
    // the connector, even with the same website and a valid staff session.
    let author = ghost_core::ids::new_id();
    sqlx::query("INSERT INTO users(id,name,slug,email,password,status,created_at) VALUES(?,'Author','native-author','author@native.test','unused','active',?)").bind(&author).bind(ghost_core::dates::now_db()).execute(&state.db).await.unwrap();
    sqlx::query(
        "INSERT INTO roles_users(id,role_id,user_id) SELECT ?,id,? FROM roles WHERE name='Author'",
    )
    .bind(ghost_core::ids::new_id())
    .bind(&author)
    .execute(&state.db)
    .await
    .unwrap();
    let author_session = ghost_db::repos::sessions::create(&state.db, &author)
        .await
        .unwrap();
    let author_cookie = format!("ghost-admin-api-session={author_session}");
    assert_eq!(
        c.get(&url)
            .header("cookie", &author_cookie)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    assert_eq!(
        form(
            &c,
            &url,
            &author_cookie,
            &origin,
            &[("action", "disconnect")]
        )
        .await
        .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    browser(&origin, &cookie, "collect").await;
    if std::env::var("NATIVE_TEST_BROWSER").as_deref() == Ok("1") {
        h.store.drain_once().await.unwrap();
        browser(&origin, &cookie, "report").await;
    }
    assert_eq!(
        form(&c, &url, &cookie, &origin, &[("action", "rotate")])
            .await
            .status(),
        reqwest::StatusCode::SEE_OTHER
    );
    let rotated = ghost_api::click::connection(&state).await.unwrap().unwrap();
    assert_ne!(rotated.integration_key, key);
    assert!(rotated.pending_key.is_none());
    assert_eq!(
        c.post(format!("{}/native/v1/websites/{id}/connect", h.base))
            .bearer_auth(&key)
            .json(&json!({"issuer":format!("{origin}/ghost/api/admin/"),"jwks":{"keys":[]}}))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    assert_eq!(
        c.post(format!("{}/v1/websites/{id}/backend-events", h.base))
            .bearer_auth(&key)
            .json(&json!({"external_id":"old-key","kind":"member_signup"}))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    // Model a committed rotation whose HTTP response was lost. The persisted
    // pending replacement must recover without generating a third key.
    let replacement = click_server::auth::secret();
    let mut pending = rotated.clone();
    pending.pending_key = Some(replacement.clone());
    sqlx::query("UPDATE click_connection SET config=? WHERE id=1")
        .bind(serde_json::to_string(&pending).unwrap())
        .execute(&state.db)
        .await
        .unwrap();
    c.post(format!("{}/native/v1/websites/{id}/rotate", h.base))
        .bearer_auth(&rotated.integration_key)
        .header("x-click-replacement-key", &replacement)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    ghost_api::click::rotate(&state).await.unwrap();
    assert_eq!(
        ghost_api::click::connection(&state)
            .await
            .unwrap()
            .unwrap()
            .integration_key,
        replacement
    );
    member(&state.db, "disconnect-pending").await;
    // Model a lost disconnect response. A retry can confirm only the already
    // disconnected matching credential and finish local cleanup.
    let endpoint = format!("{}/native/v1/websites/{id}/disconnect", h.base);
    assert_eq!(
        c.post(&endpoint)
            .bearer_auth(&replacement)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(
        form(&c, &url, &cookie, &origin, &[("action", "disconnect")])
            .await
            .status(),
        reqwest::StatusCode::SEE_OTHER
    );
    assert!(ghost_api::click::connection(&state)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM click_outbox")
            .fetch_one(&state.db)
            .await
            .unwrap(),
        0
    );
    let html = c.get(&origin).send().await.unwrap().text().await.unwrap();
    assert!(!html.contains(&format!("data-website=\"{}\"", configured.collection_id)));
    server.abort();
    state.db.close().await;
    h.finish().await;
}
