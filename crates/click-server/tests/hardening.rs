mod support;
use click_core::{digest, totp};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use support::Harness;
use uuid::Uuid;
const PASSWORD: &str = "correct-horse-battery-staple";
async fn promote(h: &Harness, id: Uuid) {
    sqlx::query("INSERT INTO platform_owners(user_id) VALUES($1)")
        .bind(id)
        .execute(&h.store.pg)
        .await
        .unwrap();
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(id)
        .execute(&h.store.pg)
        .await
        .unwrap();
}
async fn signin(h: &Harness, c: &Client, email: &str, password: &str) -> reqwest::Response {
    h.post(c, "/owner/auth/login")
        .form(&[("email", email), ("password", password)])
        .send()
        .await
        .unwrap()
}
async fn action(h: &Harness, c: &Client, action: &str, code: &str) -> reqwest::Response {
    h.post(c, "/owner/account/action")
        .form(&[
            ("action", action),
            ("current_password", PASSWORD),
            ("code", code),
        ])
        .send()
        .await
        .unwrap()
}
async fn enabled(h: &Harness, id: Uuid) {
    sqlx::query("INSERT INTO owner_security(user_id,totp_secret,confirmed_at) VALUES($1,$2,now())")
        .bind(id)
        .bind(totp::encode_secret(b"12345678901234567890"))
        .execute(&h.store.pg)
        .await
        .unwrap();
    sqlx::query("INSERT INTO owner_recovery_codes(code_hash,user_id) VALUES($1,$2)")
        .bind(digest("known-private-recovery-code"))
        .bind(id)
        .execute(&h.store.pg)
        .await
        .unwrap();
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn owner_mfa_enrollment_challenge_replay_and_session_isolation() {
    let h = Harness::new().await;
    let owner = h.client();
    let id = Uuid::parse_str(
        h.register(&owner, "security@test.example").await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    promote(&h, id).await;
    assert_eq!(
        signin(&h, &owner, "security@test.example", PASSWORD)
            .await
            .status(),
        StatusCode::SEE_OTHER
    );
    let other = h.client();
    signin(&h, &other, "security@test.example", PASSWORD)
        .await
        .error_for_status()
        .unwrap();
    let setup = action(&h, &owner, "enroll", "").await;
    assert_eq!(setup.status(), StatusCode::OK);
    let setup = setup.text().await.unwrap();
    let secret = setup
        .split("id=\"totp-secret\">")
        .nth(1)
        .unwrap()
        .split('<')
        .next()
        .unwrap();
    let bytes = totp::decode_secret(secret).unwrap();
    assert_eq!(
        action(&h, &owner, "confirm", "bad").await.status(),
        StatusCode::BAD_REQUEST
    );
    let code = totp::code_at(&bytes, chrono::Utc::now().timestamp() as u64);
    let result = action(&h, &owner, "confirm", &code).await;
    assert_eq!(result.status(), StatusCode::OK);
    let html = result.text().await.unwrap();
    let codes: Vec<&str> = html
        .split("<li><code>")
        .skip(1)
        .map(|s| s.split('<').next().unwrap())
        .collect();
    assert_eq!(codes.len(), 10);
    assert_eq!(
        other
            .get(format!("{}/owner/v1/me", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let account = owner
        .get(format!("{}/owner/account", h.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(account.contains("Two-factor authentication is enabled"));
    assert!(!account.contains(secret));
    assert!(!account.contains(codes[0]));
    let pending = h.client();
    let login = signin(&h, &pending, "security@test.example", PASSWORD).await;
    assert_eq!(login.headers()["location"], "/owner/auth/verify");
    assert_eq!(
        pending
            .get(format!("{}/owner/v1/me", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post(&pending, "/owner/auth/verify")
            .form(&[("code", code.as_str())])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST,
        "enrollment step cannot be replayed"
    );
    assert_eq!(
        h.post(&pending, "/owner/auth/verify")
            .form(&[("code", codes[0])])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        pending
            .get(format!("{}/owner/v1/me", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        pending
            .get(format!("{}/v1/me", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let retry = h.client();
    signin(&h, &retry, "security@test.example", PASSWORD)
        .await
        .error_for_status()
        .unwrap();
    assert_eq!(
        h.post(&retry, "/owner/auth/verify")
            .form(&[("code", codes[0])])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.post(&retry, "/owner/auth/verify")
            .form(&[("code", codes[1])])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    h.finish().await;
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn owner_recovery_preserves_mfa_and_revokes_all_sessions() {
    let h = Harness::new().await;
    let owner = h.client();
    let id = Uuid::parse_str(
        h.register(&owner, "recover@test.example").await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    promote(&h, id).await;
    signin(&h, &owner, "recover@test.example", PASSWORD)
        .await
        .error_for_status()
        .unwrap();
    enabled(&h, id).await;
    let anon = h.client();
    assert_eq!(
        h.post(&anon, "/v1/account/reset/request")
            .json(&json!({"email":"recover@test.example"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM mail_outbox")
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(before, 0, "customer recovery excludes owner accounts");
    assert_eq!(
        h.post(&anon, "/owner/account/reset/request")
            .form(&[("email", "recover@test.example")])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let body: String =
        sqlx::query_scalar("SELECT body FROM mail_outbox WHERE recipient='recover@test.example'")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    let token = body
        .split('#')
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    assert_eq!(
        h.post(&anon, "/v1/account/reset")
            .json(&json!({"token":token,"password":"new-password-for-owner"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.post(&anon, "/owner/account/reset")
            .form(&[
                ("token", token),
                ("password", "new-password-for-owner"),
                ("code", "")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.post(&anon, "/owner/account/reset")
            .form(&[
                ("token", token),
                ("password", "new-password-for-owner"),
                ("code", "known-private-recovery-code")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        owner
            .get(format!("{}/owner/v1/me", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.post(&anon, "/owner/account/reset")
            .form(&[
                ("token", token),
                ("password", "another-owner-password"),
                ("code", "known-private-recovery-code")
            ])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        signin(&h, &anon, "recover@test.example", PASSWORD)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let login = signin(&h, &anon, "recover@test.example", "new-password-for-owner").await;
    assert_eq!(
        login.headers()["location"],
        "/owner/auth/verify",
        "email reset never disables MFA"
    );
    let attempts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM owner_recovery_codes WHERE user_id=$1 AND used_at IS NOT NULL",
    )
    .bind(id)
    .fetch_one(&h.store.pg)
    .await
    .unwrap();
    assert_eq!(attempts, 1);
    h.finish().await;
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn owner_challenges_have_a_durable_attempt_limit() {
    let h = Harness::new().await;
    let owner = h.client();
    let id = Uuid::parse_str(
        h.register(&owner, "attempts@test.example").await["id"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    promote(&h, id).await;
    enabled(&h, id).await;
    signin(&h, &owner, "attempts@test.example", PASSWORD)
        .await
        .error_for_status()
        .unwrap();
    for _ in 0..5 {
        assert_eq!(
            h.post(&owner, "/owner/auth/verify")
                .form(&[("code", "invalid")])
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        h.post(&owner, "/owner/auth/verify")
            .form(&[("code", "known-private-recovery-code")])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    // A fresh password proof is needed after five failures.
    signin(&h, &owner, "attempts@test.example", PASSWORD)
        .await
        .error_for_status()
        .unwrap();
    assert_eq!(
        h.post(&owner, "/owner/auth/verify")
            .form(&[("code", "known-private-recovery-code")])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SEE_OTHER
    );
    h.finish().await;
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn account_races_invitation_mail_and_tenant_report_limits() {
    let h = Harness::new().await;
    let client = h.client();
    let user = h.register(&client, "race@test.example").await;
    let org = h.org(&client).await;
    let site = h.site(&client, org).await;
    let invitation: Value = h
        .post(&client, &format!("/v1/organizations/{org}/invitations"))
        .json(&json!({"email":"invitee@test.example","role":"viewer"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invitation["emailed"], true);
    let body: String =
        sqlx::query_scalar("SELECT body FROM mail_outbox WHERE recipient='invitee@test.example'")
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert!(body.contains(invitation["accept_url"].as_str().unwrap()));
    assert!(body.contains("viewer"));
    assert!(body.contains("Test studio"));
    // Hold all this workspace's slots without using any other tenant's budget.
    let permits: Vec<_> = (0..4)
        .map(|_| h.state.tenant_queries.try_acquire(org).unwrap())
        .collect();
    let url = format!(
        "{}/v1/websites/{}/overview",
        h.base,
        site["id"].as_str().unwrap()
    );
    let busy = client.get(&url).send().await.unwrap();
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(busy.headers()["retry-after"], "2");
    drop(permits);
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        StatusCode::OK
    );
    // A long session retains its total event count but only its first 100
    // ordered steps, even with out-of-order input and replayed event rows.
    let now = chrono::Utc::now().timestamp_millis() - 5000;
    let session = Uuid::new_v4().to_string();
    let mut insert = h.store.ch.insert("events").unwrap();
    for index in (0..1200).rev() {
        let row = click_store::EventRow {
            organization_id: org.to_string(),
            website_id: site["id"].as_str().unwrap().into(),
            event_id: Uuid::new_v4().to_string(),
            timestamp: (now / 1000) as u32,
            session_id: session.clone(),
            visitor_id: session.clone(),
            name: "pageview".into(),
            path: format!("/step/{index}"),
            country: "GB".into(),
            referrer: String::new(),
            utm_source: String::new(),
            utm_medium: String::new(),
            utm_campaign: String::new(),
            device: "desktop".into(),
            browser: "test".into(),
            occurred_ms: now + index,
            metric_name: String::new(),
            metric_id: String::new(),
            metric_value: 0.0,
            amount_minor: 0,
            currency: String::new(),
        };
        insert.write(&row).await.unwrap();
        if index == 0 {
            insert.write(&row).await.unwrap();
        }
    }
    insert.end().await.unwrap();
    let journeys: Value = client
        .get(format!(
            "{}/v1/websites/{}/journeys",
            h.base,
            site["id"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(journeys[0]["events"], 1200);
    assert_eq!(journeys[0]["steps"].as_array().unwrap().len(), 100);
    assert_eq!(journeys[0]["steps"][0][1], "/step/0");
    assert_eq!(journeys[0]["steps"][99][1], "/step/99");
    // Both requests start with the same live session and old password. The
    // loser must re-read the session/password after it gets the account lock.
    let id = Uuid::parse_str(user["id"].as_str().unwrap()).unwrap();
    let mut tx = h.store.pg.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let first = h
        .post(&client, "/v1/account/password")
        .json(&json!({"current_password":PASSWORD,"new_password":"first-new-password-123"}));
    let second = h
        .post(&client, "/v1/account/password")
        .json(&json!({"current_password":PASSWORD,"new_password":"second-new-password-456"}));
    let first = tokio::spawn(async move { first.send().await.unwrap().status() });
    let second = tokio::spawn(async move { second.send().await.unwrap().status() });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    tx.commit().await.unwrap();
    let (a, b) = (first.await.unwrap(), second.await.unwrap());
    assert!(
        matches!(
            (a, b),
            (StatusCode::OK, StatusCode::UNAUTHORIZED) | (StatusCode::UNAUTHORIZED, StatusCode::OK)
        ),
        "concurrent changes returned {a} and {b}"
    );
    h.finish().await;
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn malformed_requests_and_overload_negotiate_html_or_json() {
    let h = Harness::new().await;
    let c = h.client();
    for accept in ["application/json", "text/html"] {
        let response = h
            .post(&c, "/v1/auth/register")
            .header("Accept", accept)
            .header("Content-Type", "application/json")
            .body("{bad")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with(accept));
        assert!(response.headers().contains_key("content-security-policy"));
        let body = response.text().await.unwrap();
        assert!(body.contains("Check the required fields"));
        assert!(!body.contains("{bad"));
    }
    let slots = h
        .state
        .query_slots
        .clone()
        .acquire_many_owned(16)
        .await
        .unwrap();
    let busy = c
        .get(format!("{}/insights", h.base))
        .header("Accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(busy.headers()["retry-after"], "2");
    assert!(busy.text().await.unwrap().contains("Return to dashboard"));
    drop(slots);
    for _ in 0..18 {
        h.post(&c, "/v1/auth/login")
            .json(&json!({}))
            .send()
            .await
            .unwrap();
    }
    let limited = h
        .post(&c, "/v1/auth/login")
        .header("Accept", "text/html")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.headers()["retry-after"], "60");
    assert!(limited.headers().contains_key("content-security-policy"));
    assert!(limited.text().await.unwrap().contains("Too many requests"));
    h.finish().await;
}

// Verify that a request has actually reached the contended lock before
// changing the row, so the assertions exercise the race, not an early check.
async fn waiting_for_org(h: &Harness) {
    for _ in 0..100 {
        let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'SELECT id FROM organizations%')").fetch_one(&h.store.pg).await.unwrap();
        if waiting {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("request did not reach the workspace lock");
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn website_origin_and_dns_verification_recheck_the_locked_state() {
    let h = Harness::new().await;
    let client = h.client();
    let user = h.register(&client, "dns-race@test.example").await;
    let org = h.org(&client).await;
    let site = h.site(&client, org).await;
    let id = Uuid::parse_str(site["id"].as_str().unwrap()).unwrap();
    let user = Uuid::parse_str(user["id"].as_str().unwrap()).unwrap();
    let login = h
        .post(&client, "/v1/auth/login")
        .json(&json!({"email":"dns-race@test.example","password":PASSWORD}))
        .send()
        .await
        .unwrap();
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("origin", h.base.parse().unwrap());
    headers.insert("cookie", cookie.parse().unwrap());
    sqlx::query(
        "UPDATE websites SET origin='https://original.example',verified_at=now() WHERE id=$1",
    )
    .bind(id)
    .execute(&h.store.pg)
    .await
    .unwrap();
    let mut tx = h.store.pg.begin().await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    // The request still sees its original origin before the lock.
    let request = h
        .patch(&client, &format!("/v1/websites/{id}"))
        .json(&json!({"name":"Kept name","origin":"https://original.example","active":true}));
    let request = tokio::spawn(async move { request.send().await.unwrap() });
    waiting_for_org(&h).await;
    // Simulate a completed origin change and verification while it waits.
    sqlx::query("UPDATE websites SET origin='https://changed.example',verification_token='changed-challenge',verified_at=now() WHERE id=$1").bind(id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(request.await.unwrap().status(), StatusCode::NO_CONTENT);
    // Returning to the original domain must rotate its challenge and clear verification.
    let token: String = sqlx::query_scalar("SELECT verification_token FROM websites WHERE id=$1")
        .bind(id)
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    assert_ne!(
        token, "changed-challenge",
        "origin comparison must use the locked row"
    );
    let verified: bool =
        sqlx::query_scalar("SELECT verified_at IS NOT NULL FROM websites WHERE id=$1")
            .bind(id)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert!(
        !verified,
        "a concurrent origin change must clear verification"
    );
    let snapshot:click_store::Website=sqlx::query_as("SELECT id,organization_id,name,origin,collection_id,verification_token,verified_at,active,is_demo FROM websites WHERE id=$1").bind(id).fetch_one(&h.store.pg).await.unwrap();
    click_server::api::confirm_website_verification(&h.state, &headers, &snapshot)
        .await
        .unwrap();
    let mut tx = h.store.pg.begin().await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let state = h.state.clone();
    let h2 = headers.clone();
    let s2 = snapshot.clone();
    let confirmation = tokio::spawn(async move {
        click_server::api::confirm_website_verification(&state, &h2, &s2).await
    });
    waiting_for_org(&h).await;
    sqlx::query("UPDATE websites SET origin='https://different.example',verification_token='different-challenge',verified_at=NULL WHERE id=$1").bind(id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        confirmation.await.unwrap().unwrap_err().status,
        StatusCode::CONFLICT
    );
    let verified: bool =
        sqlx::query_scalar("SELECT verified_at IS NOT NULL FROM websites WHERE id=$1")
            .bind(id)
            .fetch_one(&h.store.pg)
            .await
            .unwrap();
    assert!(
        !verified,
        "old DNS evidence must not verify a different origin"
    );
    let mut tx = h.store.pg.begin().await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let state = h.state.clone();
    let h2 = headers.clone();
    let s2 = snapshot.clone();
    let confirmation = tokio::spawn(async move {
        click_server::api::confirm_website_verification(&state, &h2, &s2).await
    });
    waiting_for_org(&h).await;
    sqlx::query("UPDATE memberships SET role='viewer' WHERE organization_id=$1 AND user_id=$2")
        .bind(org)
        .bind(user)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        confirmation.await.unwrap().unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    sqlx::query("UPDATE memberships SET role='owner' WHERE organization_id=$1 AND user_id=$2")
        .bind(org)
        .bind(user)
        .execute(&h.store.pg)
        .await
        .unwrap();
    sqlx::query("UPDATE organizations SET status='suspended' WHERE id=$1")
        .bind(org)
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        click_server::api::confirm_website_verification(&h.state, &headers, &snapshot)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    h.finish().await;
}
