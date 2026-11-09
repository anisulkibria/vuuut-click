mod support;
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use support::Harness;
use uuid::Uuid;

struct Signer {
    key: EncodingKey,
    jwks: Value,
}
impl Signer {
    fn new() -> Self {
        // Generate ephemeral test-only keys with the system OpenSSL provider.
        // No private-key material is written to disk or command arguments.
        let generated = Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
                "-pkeyopt",
                "rsa_keygen_pubexp:65537",
            ])
            .output()
            .unwrap();
        assert!(generated.status.success());
        let private = generated.stdout;
        let mut child = Command::new("openssl")
            .args(["rsa", "-noout", "-modulus"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&private).unwrap();
        let modulus = child.wait_with_output().unwrap();
        assert!(modulus.status.success());
        let modulus = String::from_utf8(modulus.stdout).unwrap();
        let n = hex::decode(modulus.trim().strip_prefix("Modulus=").unwrap()).unwrap();
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        Self {
            key: EncodingKey::from_rsa_pem(&private).unwrap(),
            jwks: json!({"keys":[{"kid":"cms-test","kty":"RSA","alg":"RS256","use":"sig","n":b64.encode(n),"e":b64.encode([1,0,1])}]}),
        }
    }
    fn token(&self, base: &str, origin: &str, site: Uuid, changes: Value) -> String {
        let now = chrono::Utc::now().timestamp();
        let mut c = json!({"iss":format!("{origin}/ghost/api/admin/"),"aud":format!("{base}/native/v1"),"sub":"staff-internal-id","role":"Owner","website_id":site,"iat":now,"exp":now+60,"jti":Uuid::new_v4().to_string()});
        for (k, v) in changes.as_object().unwrap() {
            c[k] = v.clone();
        }
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("cms-test".into());
        jsonwebtoken::encode(&header, &c, &self.key).unwrap()
    }
}
async fn platform(h: &Harness, id: &str) -> String {
    let token = format!("platform-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO native_platforms(id,token_hash,domain_suffix) VALUES($1,$2,'127.0.0.1')",
    )
    .bind(id)
    .bind(click_core::digest(&token))
    .execute(&h.store.pg)
    .await
    .unwrap();
    token
}
async fn provision(
    h: &Harness,
    c: &Client,
    key: &str,
    tenant: &str,
    revision: i64,
    state: &str,
    origin: &str,
) -> Value {
    let response=c.put(format!("{}/native/v1/tenants/{tenant}",h.base)).bearer_auth(key)
        .json(&json!({"revision":revision,"state":state,"origin":origin,"name":"Managed publication","plan_id":"pro"})).send().await.unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_str(&body).unwrap()
}
async fn post(
    h: &Harness,
    c: &Client,
    key: &str,
    site: Uuid,
    action: &str,
    b: Value,
) -> reqwest::Response {
    c.post(format!("{}/native/v1/websites/{site}/{action}", h.base))
        .bearer_auth(key)
        .json(&b)
        .send()
        .await
        .unwrap()
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn native_provisioning_staff_exchange_lifecycle_and_replay() {
    let h = Harness::new().await;
    let c = h.client();
    let platform_key = platform(&h, "managed").await;
    let first = provision(&h, &c, &platform_key, "tenant-a", 1, "active", &h.base).await;
    let replay = provision(&h, &c, &platform_key, "tenant-a", 1, "active", &h.base).await;
    assert_eq!(first, replay);
    let (a, b) = tokio::join!(
        provision(
            &h,
            &c,
            &platform_key,
            "tenant-concurrent",
            1,
            "active",
            &h.base
        ),
        provision(
            &h,
            &c,
            &platform_key,
            "tenant-concurrent",
            1,
            "active",
            &h.base
        )
    );
    assert_eq!(a, b);
    let site = Uuid::parse_str(first["website_id"].as_str().unwrap()).unwrap();
    let key = first["integration_key"].as_str().unwrap();
    let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM websites WHERE id=$1")
        .bind(site)
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT plan_id FROM subscriptions WHERE organization_id=$1"
        )
        .bind(org)
        .fetch_one(&h.store.pg)
        .await
        .unwrap(),
        "pro"
    );
    let conflict=c.put(format!("{}/native/v1/tenants/tenant-a",h.base)).bearer_auth(&platform_key).json(&json!({"revision":1,"state":"suspended","origin":h.base,"name":"Managed publication","plan_id":"pro"})).send().await.unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let signer = Signer::new();
    let connect =
        |origin: &str| json!({"issuer":format!("{origin}/ghost/api/admin/"),"jwks":signer.jwks});
    assert_eq!(
        post(
            &h,
            &c,
            key,
            site,
            "connect",
            connect("https://attacker.test")
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(&h, &c, key, site, "connect", connect(&h.base))
            .await
            .status(),
        StatusCode::OK
    );
    let valid = signer.token(&h.base, &h.base, site, json!({}));
    let access: Value = post(&h, &c, key, site, "exchange", json!({"token":valid}))
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let access = access["access_token"].as_str().unwrap();
    assert_eq!(
        post(&h, &c, key, site, "exchange", json!({"token":valid}))
            .await
            .status(),
        StatusCode::UNAUTHORIZED,
        "assertions are one use"
    );
    for changes in [
        json!({"aud":h.base}),
        json!({"iss":"https://attacker.test/ghost/api/admin/"}),
        json!({"website_id":Uuid::new_v4()}),
        json!({"role":"Author"}),
        json!({"exp":chrono::Utc::now().timestamp()-1}),
        json!({"iat":chrono::Utc::now().timestamp()+100}),
        json!({"exp":chrono::Utc::now().timestamp()+900}),
    ] {
        let token = signer.token(&h.base, &h.base, site, changes);
        assert_eq!(
            post(&h, &c, key, site, "exchange", json!({"token":token}))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let get = |site: Uuid, token: &str| {
        c.get(format!("{}/v1/websites/{site}/overview", h.base))
            .bearer_auth(token)
            .send()
    };
    assert_eq!(get(site, access).await.unwrap().status(), StatusCode::OK);
    assert_eq!(
        get(
            Uuid::parse_str(a["website_id"].as_str().unwrap()).unwrap(),
            access
        )
        .await
        .unwrap()
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        c.get(format!("{}/v1/websites", h.base))
            .bearer_auth(access)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(site, key).await.unwrap().status(),
        StatusCode::NOT_FOUND,
        "integration keys cannot directly read"
    );
    let backend = |key: &str| {
        c.post(format!("{}/v1/websites/{site}/backend-events",h.base)).bearer_auth(key).json(&json!({"external_id":"invoice-opaque-1","kind":"payment","amount_minor":1500,"currency":"USD"})).send()
    };
    let accepted: Value = backend(key)
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(accepted["accepted"], 1);
    let duplicate: Value = backend(key)
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(duplicate["duplicates"], 1);
    assert_eq!(
        backend(access).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    let new = click_server::auth::secret();
    assert_eq!(
        c.post(format!("{}/native/v1/websites/{site}/rotate", h.base))
            .bearer_auth(key)
            .header("x-click-replacement-key", &new)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        get(site, access).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(backend(key).await.unwrap().status(), StatusCode::NOT_FOUND);
    assert_eq!(
        post(&h, &c, &new, site, "connect", connect(&h.base))
            .await
            .status(),
        StatusCode::OK
    );
    let suspended = provision(&h, &c, &platform_key, "tenant-a", 2, "suspended", &h.base).await;
    assert_eq!(backend(&new).await.unwrap().status(), StatusCode::NOT_FOUND);
    assert_eq!(
        post(
            &h,
            &c,
            suspended["integration_key"].as_str().unwrap(),
            site,
            "connect",
            connect(&h.base)
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let resumed = provision(&h, &c, &platform_key, "tenant-a", 3, "active", &h.base).await;
    let resumed_key = resumed["integration_key"].as_str().unwrap();
    assert_eq!(
        post(&h, &c, resumed_key, site, "connect", connect(&h.base))
            .await
            .status(),
        StatusCode::OK
    );
    let custom = provision(
        &h,
        &c,
        &platform_key,
        "tenant-a",
        4,
        "active",
        "https://custom.example.test",
    )
    .await;
    assert_eq!(custom["verified"], false);
    assert_ne!(custom["verification_token"], first["verification_token"]);
    assert_eq!(
        post(
            &h,
            &c,
            custom["integration_key"].as_str().unwrap(),
            site,
            "connect",
            connect("https://custom.example.test")
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        backend(custom["integration_key"].as_str().unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let other = platform(&h, "another-platform").await;
    let isolated = provision(&h, &c, &other, "tenant-a", 1, "active", &h.base).await;
    assert_ne!(isolated["website_id"], first["website_id"]);
    let last = provision(
        &h,
        &c,
        &platform_key,
        "tenant-a",
        5,
        "deleted",
        "https://custom.example.test",
    )
    .await;
    assert!(last["integration_key"].is_null());
    assert_eq!(
        last,
        provision(
            &h,
            &c,
            &platform_key,
            "tenant-a",
            5,
            "deleted",
            "https://custom.example.test"
        )
        .await
    );
    let resurrect=c.put(format!("{}/native/v1/tenants/tenant-a",h.base)).bearer_auth(&platform_key).json(&json!({"revision":6,"state":"active","origin":h.base,"name":"Managed publication","plan_id":"pro"})).send().await.unwrap();
    assert_eq!(resurrect.status(), StatusCode::CONFLICT);
    // The deletion worker may erase all tenant rows; the tombstone survives.
    sqlx::query("DELETE FROM organizations WHERE id=$1")
        .bind(org)
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert!(provision(
        &h,
        &c,
        &platform_key,
        "tenant-a",
        5,
        "deleted",
        "https://custom.example.test"
    )
    .await["website_id"]
        .is_null());
    let iso_site = Uuid::parse_str(isolated["website_id"].as_str().unwrap()).unwrap();
    let iso_key = isolated["integration_key"].as_str().unwrap();
    assert_eq!(
        post(&h, &c, iso_key, iso_site, "connect", connect(&h.base))
            .await
            .status(),
        StatusCode::OK
    );
    let token = signer.token(&h.base, &h.base, iso_site, json!({}));
    let access: Value = post(
        &h,
        &c,
        iso_key,
        iso_site,
        "exchange",
        json!({"token":token}),
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    sqlx::query("UPDATE native_platforms SET active=false WHERE id='another-platform'")
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        get(iso_site, access["access_token"].as_str().unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(&h, &c, iso_key, iso_site, "connect", connect(&h.base))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    h.finish().await;
}

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn backend_credentials_are_rechecked_when_durable_acceptance_waits_on_revocation() {
    let h = Harness::new().await;
    let c = h.client();
    let platform = platform(&h, "revocation-race").await;
    let p = provision(&h, &c, &platform, "race", 1, "active", &h.base).await;
    let id = Uuid::parse_str(p["website_id"].as_str().unwrap()).unwrap();
    let key = p["integration_key"].as_str().unwrap().to_owned();
    let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM websites WHERE id=$1")
        .bind(id)
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    let mut tx = h.store.pg.begin().await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let url = format!("{}/v1/websites/{id}/backend-events", h.base);
    let client = c.clone();
    let request = tokio::spawn(async move {
        client.post(url).bearer_auth(key).json(&json!({"external_id":"contended-payment","kind":"payment","amount_minor":1000,"currency":"USD"})).send().await.unwrap()
    });
    let mut blocked = false;
    for _ in 0..100 {
        blocked=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'SELECT id FROM organizations%')").fetch_one(&h.store.pg).await.unwrap();
        if blocked {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        blocked,
        "request must have passed its initial credential check and reached durable acceptance"
    );
    sqlx::query("UPDATE api_keys SET revoked_at=now() WHERE website_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(request.await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM event_outbox WHERE website_id=$1")
            .bind(id)
            .fetch_one(&h.store.pg)
            .await
            .unwrap(),
        0
    );
    h.finish().await;
}

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn signed_native_events_reject_tampering_stale_send_times_and_read_credentials() {
    use hmac::{Hmac, Mac};
    let h = Harness::new().await;
    let c = h.client();
    let platform = platform(&h, "signed-events").await;
    let p = provision(&h, &c, &platform, "signed", 1, "active", &h.base).await;
    let id = Uuid::parse_str(p["website_id"].as_str().unwrap()).unwrap();
    let key = p["integration_key"].as_str().unwrap();
    let payload=json!({"external_id":"opaque-native-source","kind":"payment","amount_minor":1000,"currency":"USD","occurred_at_ms":chrono::Utc::now().timestamp_millis()}).to_string();
    let signature = |time: i64| {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).unwrap();
        mac.update(format!("{time}.").as_bytes());
        mac.update(payload.as_bytes());
        format!("t={time},v1={}", hex::encode(mac.finalize().into_bytes()))
    };
    let url = format!("{}/native/v1/websites/{id}/events", h.base);
    let now = chrono::Utc::now().timestamp();
    let send = |body: String, sig: String| {
        c.post(&url)
            .bearer_auth(key)
            .header("content-type", "application/json")
            .header("x-click-signature", sig)
            .body(body)
            .send()
    };
    assert_eq!(
        send(payload.replace("1000", "2000"), signature(now))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(payload.clone(), signature(now - 301))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(payload.clone(), signature(now + 120))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        c.post(&url)
            .bearer_auth(key)
            .header("content-type", "application/json")
            .body(payload.clone())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let accepted: Value = send(payload.clone(), signature(now))
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(accepted["accepted"], 1);
    let duplicate: Value = send(payload.clone(), signature(now))
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(duplicate["duplicates"], 1);
    let read = click_server::auth::secret();
    sqlx::query("INSERT INTO api_keys(id,website_id,token_hash,label,scope) VALUES($1,$2,$3,'read test','read')")
        .bind(Uuid::new_v4()).bind(id).bind(click_core::digest(&read)).execute(&h.store.pg).await.unwrap();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(read.as_bytes()).unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(payload.as_bytes());
    let sig = format!("t={now},v1={}", hex::encode(mac.finalize().into_bytes()));
    assert_eq!(
        c.post(&url)
            .bearer_auth(read)
            .header("content-type", "application/json")
            .header("x-click-signature", sig)
            .body(payload)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    h.finish().await;
}

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn native_reports_count_exact_activity_and_post_traffic_without_top_list_truncation() {
    let h = Harness::new().await;
    let c = h.client();
    let platform = platform(&h, "native-reports").await;
    let p = provision(&h, &c, &platform, "report", 1, "active", &h.base).await;
    let id = Uuid::parse_str(p["website_id"].as_str().unwrap()).unwrap();
    let key = p["integration_key"].as_str().unwrap();
    let site = h
        .store
        .collect_site(p["collection_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    let session = Uuid::new_v4();
    let mut events = Vec::new();
    for (path, name) in [
        ("/article", "pageview"),
        ("/article/", "pageview"),
        ("/other/", "pageview"),
        ("/article/", "scroll"),
    ] {
        events.push(
            click_core::BrowserEvent {
                event_id: Uuid::new_v4(),
                session_id: session,
                visitor_id: None,
                name: name.into(),
                url: format!("{}{path}", h.base),
                referrer: String::new(),
                utm_source: String::new(),
                utm_medium: String::new(),
                utm_campaign: String::new(),
                occurred_at_ms: None,
                vital: None,
                engagement: None,
            }
            .clean(id, &h.base)
            .unwrap(),
        );
    }
    // More than 100 higher-count custom names must not hide a CMS fact.
    for n in 0..150 {
        for _ in 0..2 {
            events.push(
                click_core::BrowserEvent {
                    event_id: Uuid::new_v4(),
                    session_id: session,
                    visitor_id: None,
                    name: format!("custom_{n}"),
                    url: format!("{}/other/", h.base),
                    referrer: String::new(),
                    utm_source: String::new(),
                    utm_medium: String::new(),
                    utm_campaign: String::new(),
                    occurred_at_ms: None,
                    vital: None,
                    engagement: None,
                }
                .clean(id, &h.base)
                .unwrap(),
            );
        }
    }
    h.store.accept(&site, events, "ZZ", "test").await.unwrap();
    let backend = format!("{}/v1/websites/{id}/backend-events", h.base);
    for _ in 0..2 {
        c.post(&backend)
            .bearer_auth(key)
            .json(&json!({"external_id":"signup-one","kind":"member_signup"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    // Simulate ClickHouse replay before background merges, on both a page
    // and a CMS activity row. Published counts must stay unchanged.
    let payloads:Vec<Value>=sqlx::query_scalar("SELECT payload FROM event_outbox WHERE website_id=$1 AND payload->>'name' IN ('pageview','server.member_signup')").bind(id).fetch_all(&h.store.pg).await.unwrap();
    assert_eq!(payloads.len(), 4);
    h.store.drain_once().await.unwrap();
    let mut insert = h
        .store
        .ch
        .insert::<click_store::EventRow>("events")
        .unwrap();
    for payload in payloads {
        insert
            .write(&serde_json::from_value::<click_store::EventRow>(payload).unwrap())
            .await
            .unwrap();
    }
    insert.end().await.unwrap();
    let read = click_server::auth::secret();
    sqlx::query("INSERT INTO api_keys(id,website_id,token_hash,label,scope) VALUES($1,$2,$3,'report read','read')").bind(Uuid::new_v4()).bind(id).bind(click_core::digest(&read)).execute(&h.store.pg).await.unwrap();
    let endpoint = format!("{}/v1/websites/{id}/native-report", h.base);
    let report: Value = c
        .get(&endpoint)
        .bearer_auth(&read)
        .query(&[("days", "7"), ("path", "/article/")])
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(report["page"]["pageviews"], 2);
    assert_eq!(report["page"]["visitors"], 1);
    assert_eq!(report["page"]["sessions"], 1);
    assert_eq!(
        report["activity"],
        json!([{"label":"server.member_signup","count":1}])
    );
    let empty: Value = c
        .get(&endpoint)
        .bearer_auth(&read)
        .query(&[("path", "/unvisited/")])
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(empty["page"]["pageviews"], 0);
    assert_eq!(
        c.get(&endpoint)
            .bearer_auth(&read)
            .query(&[("path", "/article/?private=secret")])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        c.get(format!(
            "{}/v1/websites/{}/native-report",
            h.base,
            Uuid::new_v4()
        ))
        .bearer_auth(&read)
        .send()
        .await
        .unwrap()
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        c.get(&endpoint)
            .bearer_auth(p["collection_id"].as_str().unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    h.finish().await;
}
