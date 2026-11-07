mod support;
use reqwest::StatusCode;
use serde_json::{json, Value};
use support::Harness;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires the development PostgreSQL and ClickHouse stack"]
async fn workspaces_teams_keys_exports_privacy_and_account_lifecycle() {
    let h = Harness::new().await;
    let owner = h.client();
    let viewer = h.client();
    let outsider = h.client();
    let owner_user = h.register(&owner, "owner@saas.test").await;
    let viewer_user = h.register(&viewer, "viewer@saas.test").await;
    h.register(&outsider, "outsider@saas.test").await;
    let org = h.org(&owner).await;
    let created = h
        .post(&owner, "/v1/organizations")
        .json(&json!({"name":"Another workspace"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let second: Value = created.json().await.unwrap();
    let second_id = second["id"].as_str().unwrap();
    assert_eq!(
        h.patch(&owner, &format!("/v1/organizations/{second_id}"))
            .json(&json!({"name":"Renamed workspace"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.patch(&outsider, &format!("/v1/organizations/{org}"))
            .json(&json!({"name":"Cross tenant"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let invite = h
        .post(&owner, &format!("/v1/organizations/{org}/invitations"))
        .json(&json!({"email":"viewer@saas.test","role":"viewer"}))
        .send()
        .await
        .unwrap();
    assert_eq!(invite.status(), StatusCode::CREATED);
    let invite: Value = invite.json().await.unwrap();
    let token = invite["accept_url"]
        .as_str()
        .unwrap()
        .split('#')
        .nth(1)
        .unwrap();
    assert_eq!(
        h.post(&outsider, "/v1/invitations/accept")
            .json(&json!({"token":token}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.post(&viewer, "/v1/invitations/accept")
            .json(&json!({"token":token}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        h.post(&viewer, "/v1/invitations/accept")
            .json(&json!({"token":token}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.post(&viewer, &format!("/v1/organizations/{org}/invitations"))
            .json(&json!({"email":"bad@saas.test","role":"admin"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.patch(
            &owner,
            &format!(
                "/v1/organizations/{org}/members/{}",
                owner_user["id"].as_str().unwrap()
            )
        )
        .json(&json!({"role":"viewer"}))
        .send()
        .await
        .unwrap()
        .status(),
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE plans SET seat_limit=2 WHERE id='free'")
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        h.post(&owner, &format!("/v1/organizations/{org}/invitations"))
            .json(&json!({"email":"outsider@saas.test","role":"analyst"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        h.post(&owner, "/settings/action")
            .form(&[
                ("action", "billing_test"),
                ("organization_id", &org.to_string()),
                ("value", "pro")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let subscription: (String, String) =
        sqlx::query_as("SELECT plan_id,provider FROM subscriptions WHERE organization_id=$1")
            .bind(org)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(subscription, ("pro".into(), "test".into()));
    let second_invite: Value = h
        .post(&owner, &format!("/v1/organizations/{org}/invitations"))
        .json(&json!({"email":"outsider@saas.test","role":"admin"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = second_invite["accept_url"]
        .as_str()
        .unwrap()
        .split('#')
        .nth(1)
        .unwrap();
    assert_eq!(
        h.post(&outsider, "/v1/invitations/accept")
            .json(&json!({"token":token}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        h.post(&outsider, &format!("/v1/organizations/{org}/invitations"))
            .json(&json!({"email":"elevate@saas.test","role":"admin"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let site = h.site(&owner, org).await;
    let id = site["id"].as_str().unwrap();
    let collection = site["collection_id"].as_str().unwrap();
    let site_uuid = Uuid::parse_str(id).unwrap();
    let session = Uuid::new_v4();
    let now = chrono::Utc::now().timestamp_millis();
    let mut events = Vec::new();
    for (index, path) in ["/", "/pricing", "/newsletter"].iter().enumerate() {
        events.push(json!({"event_id":Uuid::new_v4(),"session_id":session,"name":"pageview","url":format!("{}{path}",h.base),"utm_campaign":"=unsafe-spreadsheet-formula","occurred_at_ms":now-1000+index as i64*100}));
    }
    let accepted = h
        .post(&owner, &format!("/v1/collect/{collection}"))
        .json(&json!({"events":events}))
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let export: Value = h
        .post(&owner, &format!("/v1/websites/{id}/exports"))
        .json(&json!({"days":7}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let export_id = export["id"].as_str().unwrap();
    click_server::jobs::process_export(&h.store).await.unwrap();
    let state: String = sqlx::query_scalar("SELECT state FROM export_jobs WHERE id=$1")
        .bind(Uuid::parse_str(export_id).unwrap())
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(state, "pending");
    assert_eq!(h.store.drain_once().await.unwrap(), 3);
    click_server::jobs::process_export(&h.store).await.unwrap();
    let download = owner
        .get(format!("{}/v1/exports/{export_id}/download", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    let csv = download.text().await.unwrap();
    assert!(csv.contains("/pricing"));
    assert!(csv.contains("'=unsafe-spreadsheet-formula"));
    assert_eq!(
        viewer
            .get(format!("{}/v1/exports/{export_id}/download", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let key: Value = h
        .post(&owner, &format!("/v1/websites/{id}/keys"))
        .json(&json!({"label":"Read service"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let read = h.client();
    assert_eq!(
        read.get(format!("{}/v1/websites/{id}/overview", h.base))
            .bearer_auth(key["key"].as_str().unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let rotated = owner
        .put(format!(
            "{}/v1/websites/{id}/keys/{}",
            h.base,
            key["id"].as_str().unwrap()
        ))
        .header("Origin", &h.base)
        .json(&json!({"label":"Rotated","scope":"read"}))
        .send()
        .await
        .unwrap();
    assert_eq!(rotated.status(), StatusCode::OK);
    let rotated: Value = rotated.json().await.unwrap();
    assert_eq!(
        read.get(format!("{}/v1/websites/{id}/overview", h.base))
            .bearer_auth(key["key"].as_str().unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        read.get(format!("{}/v1/websites/{id}/overview", h.base))
            .bearer_auth(rotated["key"].as_str().unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let before: i64 =
        sqlx::query_scalar("SELECT coalesce(sum(accepted_events),0)::bigint FROM usage_records")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    let demo = click_server::demo::seed(&h.store, "viewer@saas.test")
        .await
        .unwrap();
    let demo_site = click_server::auth::authorize_site(
        &h.state,
        &cookie_headers(&viewer, &h.base).await,
        demo,
        false,
    )
    .await
    .unwrap()
    .0;
    let (start, end) =
        click_server::api::range(&click_server::api::Period { days: Some(30) }).unwrap();
    let totals = h.store.overview(&demo_site, start, end).await.unwrap();
    assert!(totals.pageviews > 1000);
    assert!(h
        .store
        .collect_site(&demo_site.collection_id)
        .await
        .unwrap()
        .is_none());
    let after: i64 =
        sqlx::query_scalar("SELECT coalesce(sum(accepted_events),0)::bigint FROM usage_records")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        h.patch(&owner, &format!("/v1/websites/{id}"))
            .json(&json!({"name":"Paused site","origin":h.base,"active":false}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.post(&owner, &format!("/v1/collect/{collection}"))
            .json(&json!({"events":events}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.patch(&owner, &format!("/v1/websites/{id}"))
            .json(&json!({"name":"Moved site","origin":"https://changed.test","active":true}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let verified: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT verified_at FROM websites WHERE id=$1")
            .bind(site_uuid)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert!(verified.is_none());
    let deletion = h
        .delete(&owner, &format!("/v1/websites/{id}"))
        .json(&json!({"confirmation":"https://changed.test"}))
        .send()
        .await
        .unwrap();
    assert_eq!(deletion.status(), StatusCode::ACCEPTED);
    let deletion: Value = deletion.json().await.unwrap();
    assert_eq!(
        owner
            .get(format!("{}/v1/websites/{id}/overview", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    click_server::jobs::process_deletion(&h.store)
        .await
        .unwrap();
    let status: Value = owner
        .get(format!(
            "{}/v1/deletions/{}",
            h.base,
            deletion["id"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["state"], "complete");
    let raw: click_store::Count = h
        .store
        .ch
        .query("SELECT count() AS count FROM events WHERE website_id=?")
        .bind(id)
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(raw.count, 0);
    assert_eq!(
        h.patch(&owner, "/v1/account/profile")
            .json(&json!({"display_name":"<script>safe name</script>"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let account = owner
        .get(format!("{}/settings?tab=account&org={org}", h.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(account.contains("&lt;script&gt;safe name&lt;/script&gt;"));
    assert_eq!(
        h.post(&owner, "/v1/account/verification")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    let mail:String=sqlx::query_scalar("SELECT body FROM mail_outbox WHERE recipient='owner@saas.test' AND subject LIKE 'Verify%' ORDER BY created_at DESC LIMIT 1").fetch_one(&h.store.pg).await.unwrap();
    let token = mail.split('#').nth(1).unwrap().lines().next().unwrap();
    assert_eq!(
        h.post(&owner, "/v1/account/verify")
            .json(&json!({"token":token}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.post(&owner, "/v1/account/verify")
            .json(&json!({"token":token}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(h.post(&owner,"/v1/account/password").json(&json!({"current_password":"correct-horse-battery-staple","new_password":"another-strong-password-123"})).send().await.unwrap().status(),StatusCode::OK);
    assert_eq!(
        h.post(&read, "/v1/auth/login")
            .json(&json!({"email":"owner@saas.test","password":"correct-horse-battery-staple"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post(&read, "/v1/auth/login")
            .json(&json!({"email":"owner@saas.test","password":"another-strong-password-123"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        h.post(&read, "/v1/account/reset/request")
            .json(&json!({"email":"owner@saas.test"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    let mail:String=sqlx::query_scalar("SELECT body FROM mail_outbox WHERE recipient='owner@saas.test' AND subject LIKE 'Reset%' ORDER BY created_at DESC LIMIT 1").fetch_one(&h.store.pg).await.unwrap();
    let token = mail.split('#').nth(1).unwrap().lines().next().unwrap();
    let anon = h.client();
    assert_eq!(
        h.post(&anon, "/v1/account/reset")
            .json(&json!({"token":token,"password":"final-strong-password-456"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        owner
            .get(format!("{}/v1/me", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        read.get(format!("{}/v1/me", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post(&anon, "/v1/account/reset")
            .json(&json!({"token":token,"password":"final-strong-password-456"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_ne!(viewer_user["id"], owner_user["id"]);
    h.finish().await;
}
async fn cookie_headers(c: &reqwest::Client, base: &str) -> axum::http::HeaderMap {
    // Obtain a session token through a fresh login in this isolated fixture; do not expose it in output.
    let response = c
        .post(format!("{base}/v1/auth/login"))
        .header("Origin", base)
        .json(&json!({"email":"viewer@saas.test","password":"correct-horse-battery-staple"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let mut h = axum::http::HeaderMap::new();
    h.insert("cookie", cookie.parse().unwrap());
    h
}
