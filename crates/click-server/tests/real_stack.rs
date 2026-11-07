//! Real PostgreSQL + ClickHouse acceptance/replay/isolation test. Each run gets isolated databases.
use click_server::{app, config::Config, AppState};
use click_store::{EventRow, Store};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires running PostgreSQL and ClickHouse; see README"]
async fn real_event_pipeline_is_durable_deduplicated_and_tenant_isolated() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("click_server=error")
        .try_init();
    let base_config = Config::from_env().expect("source the development .env before this test");
    let admin = Store::connect(
        &base_config.database_url,
        &base_config.clickhouse_url,
        &base_config.clickhouse_user,
        &base_config.clickhouse_password,
        &base_config.clickhouse_database,
    )
    .await
    .unwrap();
    let database = format!("click_it_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {database}"))
        .execute(&admin.pg)
        .await
        .unwrap();
    admin
        .ch
        .query(&format!("CREATE DATABASE {database}"))
        .execute()
        .await
        .unwrap();
    let mut pg = url::Url::parse(&base_config.database_url).unwrap();
    pg.set_path(&database);
    let store = Store::connect(
        pg.as_str(),
        &base_config.clickhouse_url,
        &base_config.clickhouse_user,
        &base_config.clickhouse_password,
        &database,
    )
    .await
    .unwrap();
    store.migrate().await.unwrap();
    store.migrate().await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut config = base_config.clone();
    config.public_url = base.clone();
    config.allow_localhost = true;
    config.trusted_proxies = vec!["127.0.0.1/32".parse().unwrap()];
    config.geoip_path = Some(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/GeoIP2-Country-Test.mmdb"),
    );
    let state = AppState::new(store.clone(), config).unwrap();
    assert_eq!(state.geoip.country("81.2.69.142".parse().unwrap()), "GB");
    assert_eq!(state.geoip.country("2001:218::".parse().unwrap()), "JP");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let owner = Client::builder().cookie_store(true).build().unwrap();
    let other = Client::builder().cookie_store(true).build().unwrap();
    let anonymous = Client::new();
    for (client, email) in [
        (&owner, "owner@test.example"),
        (&other, "other@test.example"),
    ] {
        let res=client.post(format!("{base}/v1/auth/register")).header("Origin",&base).json(&json!({"email":email,"password":"correct-horse-battery-staple","organization":"Test studio"})).send().await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "{}",
            res.text().await.unwrap()
        );
    }
    for client in [&anonymous, &owner] {
        let home = client.get(format!("{base}/")).send().await.unwrap();
        assert_eq!(home.status(), StatusCode::OK);
        assert_eq!(home.url().path(), "/");
        let body = home.text().await.unwrap();
        assert!(body.contains("Behind every"));
        assert!(!body.contains("owner@test.example"));
    }
    let orgs: Value = owner
        .get(format!("{base}/v1/organizations"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let org = orgs[0]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("Unexpected organizations response: {orgs}"));
    let created = owner
        .post(format!("{base}/v1/websites"))
        .header("Origin", &base)
        .json(&json!({"organization_id":org,"name":"Test publication","origin":base}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let site: Value = created.json().await.unwrap();
    let id = site["id"].as_str().unwrap();
    let token = site["collection_id"].as_str().unwrap();
    let event_id = Uuid::new_v4();
    let session = Uuid::new_v4();
    let event = json!({"event_id":event_id,"session_id":session,"name":"pageview","url":format!("{base}/post?token=do-not-store#private"),"referrer":"https://search.example/path?token=do-not-store","utm_source":"search"});
    let collect = |body: Value| {
        anonymous
            .post(format!("{base}/v1/collect/{token}"))
            .header("Origin", &base)
            .header("X-Forwarded-For", "81.2.69.142")
            .json(&body)
    };
    let response = collect(json!({"events":[event.clone()]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted: Value = response.json().await.unwrap();
    assert_eq!(accepted["accepted"], 1);
    let payload: Value = sqlx::query_scalar("SELECT payload FROM event_outbox WHERE event_id=$1")
        .bind(event_id)
        .fetch_one(&store.pg)
        .await
        .unwrap();
    assert!(!payload.to_string().contains("do-not-store"));
    assert!(!payload.to_string().contains("81.2.69.142"));
    assert_eq!(payload["country"], "GB");
    // Database durability across an independent connection, before any ClickHouse write.
    let reconnected = Store::connect(
        pg.as_str(),
        &base_config.clickhouse_url,
        &base_config.clickhouse_user,
        &base_config.clickhouse_password,
        &database,
    )
    .await
    .unwrap();
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE state='pending'")
        .fetch_one(&reconnected.pg)
        .await
        .unwrap();
    assert_eq!(queued, 1);
    // Outage leaves durable payload and retry state intact.
    let mut outage = reconnected.clone();
    outage.ch = clickhouse::Client::default()
        .with_url("http://127.0.0.1:9")
        .with_database(&database);
    assert!(outage.drain_once().await.is_err());
    let retry: (i32, String) =
        sqlx::query_as("SELECT attempts,state FROM event_outbox WHERE event_id=$1")
            .bind(event_id)
            .fetch_one(&store.pg)
            .await
            .unwrap();
    assert_eq!(retry, (1, "pending".into()));
    sqlx::query("UPDATE event_outbox SET available_at=now()")
        .execute(&store.pg)
        .await
        .unwrap();
    // Simulate a process crash after CH commit but before PostgreSQL delivery acknowledgement.
    let replay: EventRow = serde_json::from_value(payload).unwrap();
    let mut insert = store.ch.insert("events").unwrap();
    insert.write(&replay).await.unwrap();
    insert.end().await.unwrap();
    assert_eq!(reconnected.drain_once().await.unwrap(), 1);
    let duplicate: Value = collect(json!({"events":[event.clone()]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(duplicate["duplicates"], 1);
    assert_eq!(duplicate["accepted"], 0);
    let overview_url = format!("{base}/v1/websites/{id}/overview");
    let report: Value = owner
        .get(&overview_url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(report["pageviews"], 1);
    assert_eq!(report["visitors"], 1);
    assert_eq!(report["sessions"], 1);
    assert_eq!(report["bounce_rate"], 100.0);
    let countries: Value = owner
        .get(format!("{base}/v1/websites/{id}/breakdown/countries"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(countries[0]["label"], "GB");
    assert_eq!(countries[0]["count"], 1);
    let html = owner
        .get(format!("{base}/dashboard?site={id}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("The bigger picture."));
    assert!(html.contains("GB"));
    assert!(html.contains("/post"));
    assert!(!html.contains("do-not-store"));
    assert_eq!(
        other.get(&overview_url).send().await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        anonymous.get(&overview_url).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        anonymous
            .get(&overview_url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        anonymous
            .post(format!("{base}/v1/collect/{token}"))
            .header("Origin", "https://evil.example")
            .json(&json!({"events":[event.clone()]}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let mut forged = event.clone();
    forged["url"] = json!("https://evil.example/");
    assert_eq!(
        collect(json!({"events":[forged]}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let key: Value = owner
        .post(format!("{base}/v1/websites/{id}/keys"))
        .header("Origin", &base)
        .json(&json!({"label":"Integration test"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        anonymous
            .get(&overview_url)
            .bearer_auth(key["key"].as_str().unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        anonymous
            .post(format!("{base}/v1/websites/{id}/goals"))
            .header("Origin", &base)
            .bearer_auth(key["key"].as_str().unwrap())
            .json(&json!({"name":"Forbidden","event_name":"signup"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    // A viewer membership cannot manage a site or create goals.
    let other_user: Value = other
        .get(format!("{base}/v1/me"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    sqlx::query("INSERT INTO memberships(organization_id,user_id,role) VALUES($1,$2,'viewer')")
        .bind(Uuid::parse_str(org).unwrap())
        .bind(Uuid::parse_str(other_user["id"].as_str().unwrap()).unwrap())
        .execute(&store.pg)
        .await
        .unwrap();
    assert_eq!(
        other
            .post(format!("{base}/v1/websites/{id}/goals"))
            .header("Origin", &base)
            .json(&json!({"name":"Forbidden","event_name":"signup"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    // Quota rejection rolls back the whole new batch; retries are never billed twice.
    sqlx::query("UPDATE plans SET monthly_events=1 WHERE id='free'")
        .execute(&store.pg)
        .await
        .unwrap();
    let mut extra = event.clone();
    extra["event_id"] = json!(Uuid::new_v4());
    assert_eq!(
        collect(json!({"events":[extra]}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let usage: Value = owner
        .get(format!("{base}/v1/organizations/{org}/usage"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(usage["accepted_events"], 1);
    assert_eq!(
        collect(json!({"events":[event]}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    let ledger: i64 = sqlx::query_scalar("SELECT count(*) FROM event_outbox")
        .fetch_one(&store.pg)
        .await
        .unwrap();
    assert_eq!(ledger, 1);
    let response = owner
        .post(format!("{base}/v1/websites"))
        .header("Origin", "https://evil.example")
        .json(&json!({"organization_id":org,"name":"csrf","origin":"https://site.example"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        owner
            .delete(format!(
                "{base}/v1/websites/{id}/keys/{}",
                key["id"].as_str().unwrap()
            ))
            .header("Origin", &base)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        anonymous
            .get(&overview_url)
            .bearer_auth(key["key"].as_str().unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    // The same cookie jar/browser can hold two independently authorized identities.
    let shared = Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let denied = shared
        .post(format!("{base}/owner/auth/login"))
        .header("Origin", &base)
        .form(&[
            ("email", "owner@test.example"),
            ("password", "correct-horse-battery-staple"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(
        denied.status(),
        StatusCode::FORBIDDEN,
        "workspace ownership alone must never grant platform ownership"
    );
    let admin_id: Uuid =
        sqlx::query_scalar("SELECT id FROM users WHERE email='owner@test.example'")
            .fetch_one(&store.pg)
            .await
            .unwrap();
    sqlx::query("INSERT INTO platform_owners(user_id) VALUES($1)")
        .bind(admin_id)
        .execute(&store.pg)
        .await
        .unwrap();
    assert_eq!(
        owner
            .get(format!("{base}/v1/me"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED,
        "prior customer sessions cannot authorize a platform owner"
    );
    let regular = shared
        .post(format!("{base}/v1/auth/login"))
        .header("Origin", &base)
        .json(&json!({"email":"other@test.example","password":"correct-horse-battery-staple"}))
        .send()
        .await
        .unwrap();
    assert_eq!(regular.status(), StatusCode::OK);
    let user_cookie = regular.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(
        shared
            .get(format!("{base}/owner/v1/me"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        shared
            .post(format!("{base}/owner/auth/login"))
            .header("Origin", "https://evil.example")
            .form(&[
                ("email", "owner@test.example"),
                ("password", "correct-horse-battery-staple")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let signed = shared
        .post(format!("{base}/owner/auth/login"))
        .header("Origin", &base)
        .form(&[
            ("email", "owner@test.example"),
            ("password", "correct-horse-battery-staple"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(signed.status(), StatusCode::SEE_OTHER);
    for section in [
        "workspaces",
        "users",
        "websites",
        "subscriptions",
        "delivery",
        "audit",
    ] {
        let response = shared
            .get(format!("{base}/owner/{section}"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Owner directory {section}"
        );
        let html = response.text().await.unwrap();
        assert!(html.contains("class=\"sidebar\""));
        assert!(html.contains("aria-current=\"page\""));
        let denied = anonymous
            .get(format!("{base}/owner/{section}"))
            .send()
            .await
            .unwrap();
        assert!(denied.url().path().ends_with("/owner/signin"));
    }
    let home = shared.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(home.status(), StatusCode::OK);
    assert!(home.headers().get("location").is_none());
    let owner_cookie = signed.headers()["set-cookie"].to_str().unwrap();
    assert!(owner_cookie.contains("Path=/owner;"));
    assert!(owner_cookie.contains("HttpOnly"));
    assert!(owner_cookie.contains("SameSite=Strict"));
    let owner_token = owner_cookie
        .split(';')
        .next()
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_owned();
    let user_me: Value = shared
        .get(format!("{base}/v1/me"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(user_me["email"], "other@test.example");
    let owner_me: Value = shared
        .get(format!("{base}/owner/v1/me"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(owner_me["email"], "owner@test.example");
    assert_eq!(
        shared
            .get(format!("{base}/owner/dashboard"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        anonymous
            .get(format!("{base}/v1/me"))
            .header("Cookie", format!("click_session={owner_token}"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        anonymous
            .get(format!("{base}/owner/v1/me"))
            .header(
                "Cookie",
                user_cookie.replace("click_session=", "click_owner_session=")
            )
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let wrong_entry = shared
        .post(format!("{base}/v1/auth/login"))
        .header("Origin", &base)
        .json(&json!({"email":"owner@test.example","password":"correct-horse-battery-staple"}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_entry.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        shared
            .post(format!("{base}/owner/auth/logout"))
            .header("Origin", &base)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        shared
            .get(format!("{base}/owner/v1/me"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        shared
            .get(format!("{base}/v1/me"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        shared
            .post(format!("{base}/owner/auth/login"))
            .header("Origin", &base)
            .form(&[
                ("email", "owner@test.example"),
                ("password", "correct-horse-battery-staple")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        shared
            .post(format!("{base}/v1/auth/logout"))
            .header("Origin", &base)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        shared
            .get(format!("{base}/v1/me"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        shared
            .get(format!("{base}/owner/v1/me"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    sqlx::query("DELETE FROM platform_owners WHERE user_id=$1")
        .bind(admin_id)
        .execute(&store.pg)
        .await
        .unwrap();
    assert_eq!(
        shared
            .get(format!("{base}/owner/v1/me"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED,
        "revocation invalidates an existing owner session"
    );
    server.abort();
    reconnected.pg.close().await;
    store.pg.close().await;
    sqlx::query(&format!("DROP DATABASE {database} WITH (FORCE)"))
        .execute(&admin.pg)
        .await
        .unwrap();
    admin
        .ch
        .query(&format!("DROP DATABASE {database}"))
        .execute()
        .await
        .unwrap();
    admin.pg.close().await;
}
