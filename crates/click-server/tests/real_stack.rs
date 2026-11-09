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
    let state = AppState::with_services(
        store.clone(),
        config,
        click_server::billing::Service { provider: None },
        click_server::mail::Mailer::local(std::path::PathBuf::from(".local/mail")),
    )
    .unwrap();
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
    // ---- editing goals and funnels -------------------------------------
    //
    // Both were create-and-delete only: a goal whose event was renamed in the
    // site's code, or a funnel with one wrong step, could not be corrected,
    // only destroyed and rebuilt under a new name. These cover the edit paths
    // and the privilege checks that run inside the mutating transaction.
    let goal: Value = owner
        .post(format!("{base}/v1/websites/{id}/goals"))
        .header("Origin", &base)
        .json(&json!({"name":"Signups","event_name":"signup"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let goal_id = goal["id"].as_str().unwrap().to_string();
    let patch_goal = |body: Value| {
        owner
            .patch(format!("{base}/v1/websites/{id}/goals/{goal_id}"))
            .header("Origin", &base)
            .json(&body)
    };

    // The name and the event are both editable, and the edit is what is
    // stored afterwards.
    assert_eq!(
        patch_goal(json!({"name":"Newsletter signups","event_name":"newsletter_signup"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let listed: Value = owner
        .get(format!("{base}/v1/websites/{id}/goals"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let stored = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["id"] == goal["id"])
        .expect("the edited goal is still listed");
    assert_eq!(stored["name"], "Newsletter signups");
    assert_eq!(stored["event_name"], "newsletter_signup");

    // A rename onto a name already taken on this site is a conflict, not a
    // silent overwrite of either goal.
    owner
        .post(format!("{base}/v1/websites/{id}/goals"))
        .header("Origin", &base)
        .json(&json!({"name":"Trials","event_name":"trial_started"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        patch_goal(json!({"name":"Trials","event_name":"newsletter_signup"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    // Validation matches creation's: an event name is not free text.
    assert_eq!(
        patch_goal(json!({"name":"Newsletter signups","event_name":"not a valid name"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    // An unknown goal is not found rather than silently doing nothing.
    assert_eq!(
        owner
            .patch(format!("{base}/v1/websites/{id}/goals/{}", Uuid::new_v4()))
            .header("Origin", &base)
            .json(&json!({"name":"Ghost","event_name":"ghost"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );

    // A goal belonging to another website is unreachable through a website
    // this membership does authorize: the update is scoped by both ids.
    let second: Value = owner
        .post(format!("{base}/v1/websites"))
        .header("Origin", &base)
        .json(&json!({"organization_id":org,"name":"Second","origin":"https://second.example"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_id = second["id"].as_str().unwrap();
    assert_eq!(
        owner
            .patch(format!("{base}/v1/websites/{second_id}/goals/{goal_id}"))
            .header("Origin", &base)
            .json(&json!({"name":"Moved","event_name":"signup"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );

    // A viewer may read the reports and neither edit nor delete.
    for method in ["patch", "delete"] {
        let request = if method == "patch" {
            other
                .patch(format!("{base}/v1/websites/{id}/goals/{goal_id}"))
                .json(&json!({"name":"Viewer","event_name":"signup"}))
        } else {
            other.delete(format!("{base}/v1/websites/{id}/goals/{goal_id}"))
        };
        assert_eq!(
            request
                .header("Origin", &base)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND,
            "a viewer completed a {method}"
        );
    }

    // A role that was downgraded before the request arrives. Caught by
    // `authorize_site`, which re-reads the membership per request — asserted
    // because it is the outer half of the defence, not because it exercises
    // the transaction guard.
    let org_uuid = Uuid::parse_str(org).unwrap();
    let owner_user: Value = owner
        .get(format!("{base}/v1/me"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let owner_id = Uuid::parse_str(owner_user["id"].as_str().unwrap()).unwrap();
    let restore_owner = |pool: sqlx::PgPool| async move {
        sqlx::query("UPDATE memberships SET role='owner' WHERE organization_id=$1 AND user_id=$2")
            .bind(org_uuid)
            .bind(owner_id)
            .execute(&pool)
            .await
            .unwrap();
    };
    sqlx::query("UPDATE memberships SET role='viewer' WHERE organization_id=$1 AND user_id=$2")
        .bind(org_uuid)
        .bind(owner_id)
        .execute(&store.pg)
        .await
        .unwrap();
    assert_eq!(
        patch_goal(json!({"name":"Downgraded","event_name":"signup"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    restore_owner(store.pg.clone()).await;

    // The race itself. Holding the organization row makes the request wait
    // inside `lock_editor`, so the role is taken away *after* it has already
    // passed its per-request check. It must still not write.
    //
    // Deterministic in both orderings: while the row is held the request
    // cannot reach the write, and if it arrives only after the commit its own
    // pre-check sees the downgrade. Either way it must not succeed.
    let mut race = store.pg.begin().await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org_uuid)
        .fetch_one(&mut *race)
        .await
        .unwrap();
    let racing = {
        let client = owner.clone();
        let url = format!("{base}/v1/websites/{id}/goals/{goal_id}");
        let origin = base.clone();
        tokio::spawn(async move {
            client
                .patch(url)
                .header("Origin", origin)
                .json(&json!({"name":"Raced","event_name":"signup"}))
                .send()
                .await
                .unwrap()
                .status()
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    sqlx::query("UPDATE memberships SET role='viewer' WHERE organization_id=$1 AND user_id=$2")
        .bind(org_uuid)
        .bind(owner_id)
        .execute(&mut *race)
        .await
        .unwrap();
    race.commit().await.unwrap();
    assert_eq!(
        racing.await.unwrap(),
        StatusCode::NOT_FOUND,
        "a write completed with a role that had been revoked"
    );
    restore_owner(store.pg.clone()).await;

    // A suspended workspace cannot be edited either. This is the assertion
    // that fails outright if `lock_editor` is removed: `authorize_site`
    // checks membership but never the workspace's status, so the transaction
    // guard is the only thing that refuses a suspended workspace.
    sqlx::query("UPDATE organizations SET status='suspended' WHERE id=$1")
        .bind(Uuid::parse_str(org).unwrap())
        .execute(&store.pg)
        .await
        .unwrap();
    assert_eq!(
        patch_goal(json!({"name":"Suspended","event_name":"signup"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE organizations SET status='active' WHERE id=$1")
        .bind(Uuid::parse_str(org).unwrap())
        .execute(&store.pg)
        .await
        .unwrap();

    // Funnels edit the same way, steps included.
    let funnel: Value = owner
        .post(format!("{base}/v1/websites/{id}/funnels"))
        .header("Origin", &base)
        .json(&json!({"name":"Reader to subscriber","steps":[{"kind":"page","value":"/"},{"kind":"page","value":"/pricing"}],"window_seconds":86400}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let funnel_id = funnel["id"].as_str().unwrap();
    assert_eq!(
        owner
            .patch(format!("{base}/v1/websites/{id}/funnels/{funnel_id}"))
            .header("Origin", &base)
            .json(&json!({"name":"Reader to trial","steps":[{"kind":"page","value":"/"},{"kind":"event","value":"trial_started"}],"window_seconds":3600}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let funnels: Value = owner
        .get(format!("{base}/v1/websites/{id}/funnels"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let edited = &funnels.as_array().unwrap()[0];
    assert_eq!(edited["name"], "Reader to trial");
    assert_eq!(edited["window_seconds"], 3600);
    assert_eq!(edited["steps"][1]["value"], "trial_started");
    // Invalid steps are refused, so a funnel cannot be edited into nonsense.
    assert_eq!(
        owner
            .patch(format!("{base}/v1/websites/{id}/funnels/{funnel_id}"))
            .header("Origin", &base)
            .json(&json!({"name":"Broken","steps":[{"kind":"page","value":"no-leading-slash"}],"window_seconds":3600}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        other
            .patch(format!("{base}/v1/websites/{id}/funnels/{funnel_id}"))
            .header("Origin", &base)
            .json(&json!({"name":"Viewer","steps":[{"kind":"page","value":"/"},{"kind":"page","value":"/a"}],"window_seconds":3600}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );

    // ---- the same recheck on the remaining workspace mutations ---------
    //
    // These held the organization row but either never looked at the
    // membership again, or did it as an `EXISTS` inside the write — which
    // changes no rows and reports success.
    //
    // Retention was the worst of the three: its membership test was an
    // `EXISTS` inside the UPDATE, so losing the role changed no rows and the
    // call still returned success and wrote an audit entry saying the
    // retention had been changed. Only reachable in the race window — the
    // per-request check catches a role already gone — so this forces that
    // window open by holding the organization row while the request waits.
    let retention_url = format!("{base}/v1/websites/{id}/retention");
    let before: i32 = sqlx::query_scalar("SELECT retention_days FROM websites WHERE id=$1")
        .bind(Uuid::parse_str(id).unwrap())
        .fetch_one(&store.pg)
        .await
        .unwrap();
    let mut held = store.pg.begin().await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org_uuid)
        .fetch_one(&mut *held)
        .await
        .unwrap();
    let waiting = {
        let client = owner.clone();
        let url = retention_url.clone();
        let origin = base.clone();
        tokio::spawn(async move {
            client
                .put(url)
                .header("Origin", origin)
                .json(&json!({"days":30}))
                .send()
                .await
                .unwrap()
                .status()
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    sqlx::query("UPDATE memberships SET role='viewer' WHERE organization_id=$1 AND user_id=$2")
        .bind(org_uuid)
        .bind(owner_id)
        .execute(&mut *held)
        .await
        .unwrap();
    held.commit().await.unwrap();
    assert_eq!(
        waiting.await.unwrap(),
        StatusCode::NOT_FOUND,
        "a privilege-lost retention change reported success"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT retention_days FROM websites WHERE id=$1")
            .bind(Uuid::parse_str(id).unwrap())
            .fetch_one(&store.pg)
            .await
            .unwrap(),
        before,
        "retention changed for a member who had lost the role"
    );
    restore_owner(store.pg.clone()).await;

    // A suspended workspace must not be able to issue a credential — the
    // credential grants access to the data the suspension withdrew — nor
    // change where collection comes from.
    sqlx::query("UPDATE organizations SET status='suspended' WHERE id=$1")
        .bind(org_uuid)
        .execute(&store.pg)
        .await
        .unwrap();
    assert_eq!(
        owner
            .post(format!("{base}/v1/websites/{id}/keys"))
            .header("Origin", &base)
            .json(&json!({"label":"While suspended","scope":"read"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        owner
            .patch(format!("{base}/v1/websites/{id}"))
            .header("Origin", &base)
            .json(&json!({"name":"Renamed while suspended","origin":"https://elsewhere.example","active":true}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        owner
            .put(&retention_url)
            .header("Origin", &base)
            .json(&json!({"days":30}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    // Erasing your own data stays possible while suspended, deliberately, so
    // a suspension cannot strand a customer's data. Asserted as the boundary
    // of the rule above: this one must *not* be a conflict.
    assert_ne!(
        owner
            .delete(format!("{base}/v1/websites/{second_id}"))
            .header("Origin", &base)
            .json(&json!({"confirmation":"https://second.example"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT,
        "a suspended workspace could not delete its own website"
    );
    sqlx::query("UPDATE organizations SET status='active' WHERE id=$1")
        .bind(org_uuid)
        .execute(&store.pg)
        .await
        .unwrap();

    // Deleting still works, and the goal is gone afterwards.
    assert_eq!(
        owner
            .delete(format!("{base}/v1/websites/{id}/goals/{goal_id}"))
            .header("Origin", &base)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let remaining: Value = owner
        .get(format!("{base}/v1/websites/{id}/goals"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !remaining
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["id"] == goal["id"]),
        "the deleted goal is still listed"
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
    // The sign-in pages are `/signin` and `/owner/signin`, but `/login` is
    // what a person types. It used to 404 on the owner side — the
    // `/owner/{section}` catch-all rejected the unknown section before
    // looking at who was asking — which read as a broken portal rather than
    // a wrong address. Each alias now lands on the real page.
    for (typed, page) in [
        ("/login", "/signin"),
        ("/register", "/signup"),
        ("/owner/login", "/owner/signin"),
    ] {
        let landed = anonymous
            .get(format!("{base}{typed}"))
            .send()
            .await
            .unwrap();
        assert_eq!(landed.status(), StatusCode::OK, "{typed} did not resolve");
        assert_eq!(
            landed.url().path(),
            page,
            "{typed} went somewhere unexpected"
        );
    }
    // And the alias must not shadow a real owner section.
    let real_section = anonymous
        .get(format!("{base}/owner/websites"))
        .send()
        .await
        .unwrap();
    assert!(real_section.url().path().ends_with("/owner/signin"));
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
