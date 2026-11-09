mod support;
use click_store::EventRow;
use serde_json::{json, Value};
use support::Harness;
use uuid::Uuid;

fn row(
    tenant: (Uuid, Uuid),
    id: &str,
    session: &str,
    visitor: &str,
    path: &str,
    name: &str,
    time: u32,
) -> EventRow {
    let (org, site) = tenant;
    EventRow {
        organization_id: org.to_string(),
        website_id: site.to_string(),
        event_id: id.into(),
        timestamp: time,
        session_id: session.into(),
        visitor_id: visitor.into(),
        name: name.into(),
        path: path.into(),
        country: "GB".into(),
        referrer: String::new(),
        utm_source: String::new(),
        utm_medium: String::new(),
        utm_campaign: String::new(),
        device: "Desktop".into(),
        browser: "Chrome".into(),
        occurred_ms: time as i64 * 1000,
        metric_name: String::new(),
        metric_id: String::new(),
        metric_value: 0.0,
        amount_minor: 0,
        currency: String::new(),
        visitor_mode: "provided".into(),
        os: "Linux".into(),
        engagement_ms: 0,
        scroll_percent: 0,
        newsletter_id: String::new(),
        recipient_id: String::new(),
    }
}
#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn exact_content_comparisons_acquisition_and_identity_coverage() {
    let h = Harness::new().await;
    let c = h.client();
    let foreign = h.client();
    h.register(&c, "breadth@report.test").await;
    h.register(&foreign, "foreign@report.test").await;
    let org = h.org(&c).await;
    let site = h.site(&c, org).await;
    let id = Uuid::parse_str(site["id"].as_str().unwrap()).unwrap();
    let (start, end) =
        click_server::api::range(&click_server::api::Period { days: Some(1) }).unwrap();
    let (prev_start, prev_end) = click_server::explore::previous(start, end);
    let current = start + (end - start) / 2;
    let old = prev_start + (prev_end - prev_start) / 2;
    let mut events = Vec::new();
    events.push(row(
        (org, id),
        "old",
        "old-session",
        "returning",
        "/old",
        "pageview",
        old,
    ));
    for (key, session, visitor, path, name) in [
        ("01", "return-session", "returning", "/a", "pageview"),
        ("02", "return-session", "returning", "/b", "pageview"),
        ("03", "return-session", "returning", "/a", "pageview"),
        ("04", "return-session", "returning", "/a", "trial_started"),
        ("05", "return-session", "returning", "/a", "scroll_custom"),
        ("06", "new-session", "new", "/a", "pageview"),
        ("07", "anonymous", "anonymous", "/anonymous", "pageview"),
        ("08", "legacy", "legacy", "/legacy", "pageview"),
    ] {
        let mut e = row((org, id), key, session, visitor, path, name, current);
        // Occurrence ties deliberately use event_id as the stable tiebreak.
        if session == "return-session" {
            e.utm_source = "google".into();
            e.utm_medium = "cpc".into();
            e.utm_campaign = "Launch".into();
        }
        if session == "new-session" {
            e.utm_source = "letter".into();
            e.utm_medium = "email".into();
        }
        if session == "anonymous" {
            e.visitor_mode = "session".into();
        }
        if session == "legacy" {
            e.visitor_mode = "unknown".into();
        }
        events.push(e);
    }
    for (key, session, visitor, path, ms, depth) in [
        ("09", "return-session", "returning", "/a", 10000, 50),
        ("10", "return-session", "returning", "/b", 5000, 100),
        ("11", "new-session", "new", "/a", 2000, 20),
    ] {
        let mut e = row(
            (org, id),
            key,
            session,
            visitor,
            path,
            "web_engagement",
            current,
        );
        e.engagement_ms = ms;
        e.scroll_percent = depth;
        events.push(e.clone());
        events.push(e);
    }
    let mut insert = h.store.ch.insert("events").unwrap();
    for e in &events {
        insert.write(e).await.unwrap();
    }
    insert.end().await.unwrap();
    sqlx::query(
        "INSERT INTO goals(id,website_id,name,event_name) VALUES($1,$2,'Trial','trial_started')",
    )
    .bind(Uuid::new_v4())
    .bind(id)
    .execute(&h.store.pg)
    .await
    .unwrap();
    let endpoint = format!("/v1/websites/{id}");
    let report = |suffix: &str| {
        c.get(format!("{}{endpoint}/{suffix}", h.base))
            .query(&[("days", "1")])
    };
    let comparison: Value = report("comparison")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(comparison["current"]["pageviews"], 6);
    assert_eq!(comparison["current"]["sessions"], 4);
    assert_eq!(comparison["current"]["bounced_sessions"], 3);
    assert_eq!(comparison["current"]["visible_ms"], 17000);
    assert_eq!(comparison["previous"]["pageviews"], 1);
    assert_eq!(
        comparison["start"].as_u64().unwrap() - comparison["previous_start"].as_u64().unwrap(),
        comparison["end"].as_u64().unwrap() - comparison["start"].as_u64().unwrap()
    );
    let pages: Value = report("content")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let a = pages
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == "/a")
        .unwrap();
    assert_eq!(a["pageviews"], 3);
    assert_eq!(a["entries"], 2);
    assert_eq!(a["exits"], 2);
    assert_eq!(a["sessions"], 2);
    assert_eq!(a["converting_sessions"], 1);
    assert_eq!(a["visible_ms"], 12000);
    assert_eq!(a["mean_scroll_percent"], 35.0);
    let acquisition: Value = report("acquisition")
        .query(&[("dimension", "channel")])
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let paid = acquisition
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["label"] == "Paid search")
        .unwrap();
    assert_eq!(paid["sessions"], 1);
    assert_eq!(paid["pageviews"], 3);
    assert_eq!(paid["converting_sessions"], 1);
    let retention: Value = report("retention")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        retention["identity"],
        json!({"new_visitors":1,"returning_visitors":1,"session_only_sessions":1,"unclassified_sessions":1})
    );
    assert_eq!(retention["cohorts"].as_array().unwrap().len(), 1);
    assert_eq!(retention["cohorts"][0]["visitors"], 1);
    assert_eq!(retention["cohorts"][0]["retained"][0], 1);
    assert!(retention["cohorts"][0]["retained"][1].is_null());
    let vitals: Value = report("vitals/comparison")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(vitals["current"], json!([]));
    assert_eq!(vitals["previous"], json!([]));
    for suffix in [
        "comparison",
        "content",
        "acquisition",
        "retention",
        "vitals/comparison",
    ] {
        assert_eq!(
            foreign
                .get(format!("{}{endpoint}/{suffix}", h.base))
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::NOT_FOUND
        );
        assert_eq!(
            h.client()
                .get(format!("{}{endpoint}/{suffix}", h.base))
                .bearer_auth(site["collection_id"].as_str().unwrap())
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::NOT_FOUND
        );
    }
    h.finish().await;
}

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn provided_visitor_funnels_newsletter_cohorts_and_recorded_cost_coverage() {
    let h = Harness::new().await;
    let c = h.client();
    let outsider = h.client();
    h.register(&c, "campaigns@report.test").await;
    h.register(&outsider, "viewer@report.test").await;
    let org = h.org(&c).await;
    let site = h.site(&c, org).await;
    let id = Uuid::parse_str(site["id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE websites SET verified_at=now() WHERE id=$1")
        .bind(id)
        .execute(&h.store.pg)
        .await
        .unwrap();
    sqlx::query("UPDATE subscriptions SET plan_id='pro',status='active' WHERE organization_id=$1")
        .bind(org)
        .execute(&h.store.pg)
        .await
        .unwrap();
    let key: Value = h
        .post(&c, &format!("/v1/websites/{id}/keys"))
        .json(&json!({"label":"Authoritative facts","scope":"integration"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = key["key"].as_str().unwrap();
    let backend = h.client();
    let visitor = Uuid::new_v4();
    let session1 = Uuid::new_v4();
    let session2 = Uuid::new_v4();
    let anonymous = Uuid::new_v4();
    let t = chrono::Utc::now().timestamp_millis() - 30000;
    let mut events = Vec::new();
    for (session, v, time, path, name) in [
        (session1, Some(visitor), t, "/section/read", "pageview"),
        (session2, Some(visitor), t + 1000, "/done", "joined"),
        (anonymous, None, t, "/section/anonymous", "pageview"),
        (anonymous, None, t + 1000, "/done", "joined"),
    ] {
        events.push(json!({"event_id":Uuid::new_v4(),"session_id":session,"visitor_id":v,"occurred_at_ms":time,"name":name,"url":format!("{}{path}",h.base),"utm_source":"google","utm_medium":"cpc","utm_campaign":"Launch"}));
    }
    h.post(
        &c,
        &format!("/v1/collect/{}", site["collection_id"].as_str().unwrap()),
    )
    .json(&json!({"events":events}))
    .send()
    .await
    .unwrap()
    .error_for_status()
    .unwrap();
    let submit = |body: Value| {
        backend
            .post(format!("{}/v1/websites/{id}/backend-events", h.base))
            .bearer_auth(key)
            .json(&body)
    };
    for (ext, kind, amount) in [("paid", "payment", 1500), ("returned", "refund", 500)] {
        submit(json!({"external_id":ext,"kind":kind,"amount_minor":amount,"currency":"USD","visitor_id":visitor,"session_id":session2,"occurred_at_ms":t+5000})).send().await.unwrap().error_for_status().unwrap();
    }
    let edition = click_core::digest("edition one");
    let a = click_core::digest("edition one recipient a");
    let b = click_core::digest("edition one recipient b");
    let orphan = click_core::digest("orphan");
    for (ext, kind, recipient, time) in [
        ("send-a", "newsletter_sent", &a, t + 10000),
        ("send-b", "newsletter_sent", &b, t + 10000),
        ("open-a", "newsletter_open", &a, t + 20000),
        ("another-open-a", "newsletter_open", &a, t + 21000),
        ("click-a-link1", "newsletter_click", &a, t + 25000),
        ("click-a-link2", "newsletter_click", &a, t + 26000),
        ("click-a-link1", "newsletter_click", &a, t + 25000),
        ("early-b", "newsletter_click", &b, t + 9000),
        ("orphan-open", "newsletter_open", &orphan, t + 20000),
    ] {
        submit(json!({"external_id":ext,"kind":kind,"newsletter_id":edition,"recipient_id":recipient,"occurred_at_ms":time})).send().await.unwrap().error_for_status().unwrap();
    }
    submit(json!({"external_id":"legacy-open","kind":"newsletter_open"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(submit(json!({"external_id":"bad","kind":"newsletter_open","newsletter_id":"personal@example.com","recipient_id":a})).send().await.unwrap().status(),reqwest::StatusCode::BAD_REQUEST);
    h.store.drain_once().await.unwrap();
    let read = |suffix: &str| {
        c.get(format!("{}/v1/websites/{id}/{suffix}", h.base))
            .query(&[("days", "1")])
    };
    for scope in ["session", "visitor"] {
        let funnel:Value=h.post(&c,&format!("/v1/websites/{id}/funnels")).json(&json!({"name":format!("Prefix {scope}"),"scope":scope,"steps":[{"kind":"page_prefix","value":"/section/"},{"kind":"event","value":"joined"}],"window_seconds":3600})).send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        let report: Value = read(&format!("funnels/{}", funnel["id"].as_str().unwrap()))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(report["scope"], scope);
        assert_eq!(report["steps"][1]["participants"], 1);
        assert_eq!(
            report["steps"][0]["participants"],
            if scope == "session" { 2 } else { 1 }
        );
        assert_eq!(
            report["steps"][1]["sessions"],
            if scope == "session" { 1 } else { 0 }
        );
    }
    let details: Value = read("events")
        .query(&[
            ("event", "joined"),
            ("path", "/done"),
            ("campaign", "Launch"),
        ])
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(details[0]["count"], 2);
    assert_eq!(details[0]["sessions"], 2);
    assert_eq!(
        read("events")
            .query(&[("path", "/?private=value")])
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let newsletters: Value = read("newsletters")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        newsletters["editions"][0],
        json!({"newsletter_id":edition,"sent_recipients":2,"opened_recipients":1,"clicked_recipients":1,"link_click_facts":2})
    );
    assert_eq!(newsletters["unassigned_events"], 1);
    assert_eq!(newsletters["activity_outside_send_cohort"], 1);
    for (dimension, label) in [
        ("medium", "cpc"),
        ("channel", "Paid search"),
        ("country", "ZZ"),
        ("device", "Desktop"),
    ] {
        let revenue: Value = read("revenue")
            .query(&[("dimension", dimension)])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(revenue[0]["label"], label);
        assert_eq!(revenue[0]["total_minor"], "1000");
    }
    let today = chrono::Utc::now().date_naive();
    let cost = json!({"reference":"ledger-row-1","revision":1,"day":today,"campaign":"Launch","currency":"USD","amount_minor":500,"source":"ledger"});
    let endpoint = format!("/v1/websites/{id}/campaign-costs");
    let first: Value = h
        .post(&c, &endpoint)
        .json(&cost)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let repeated: Value = h
        .post(&c, &endpoint)
        .json(&cost)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first, repeated);
    let mut conflict = cost.clone();
    conflict["amount_minor"] = json!(400);
    assert_eq!(
        h.post(&c, &endpoint)
            .json(&conflict)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    let roi: Value = read("campaign-costs")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(roi[0]["roi_percent"], 100.0);
    assert_eq!(roi[0]["roas"], 2.0);
    assert_eq!(roi[0]["sources"], json!(["ledger"]));
    conflict["revision"] = json!(2);
    h.post(&c, &endpoint)
        .json(&conflict)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(
        h.post(&c, &endpoint)
            .json(&cost)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    let partial: Value = c
        .get(format!("{}{endpoint}?days=7", h.base))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(partial[0]["covered_days"], 1);
    assert!(partial[0]["roi_percent"].is_null());
    // Explicit daily zero records establish coverage; missing days never do.
    for day in 1..7 {
        let mut input = cost.clone();
        input["reference"] = json!(format!("day-{day}"));
        input["day"] = json!(today - chrono::Duration::days(day));
        input["amount_minor"] = json!(0);
        backend
            .post(format!("{}{endpoint}", h.base))
            .bearer_auth(key)
            .json(&input)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }
    let covered: Value = c
        .get(format!("{}{endpoint}?days=7", h.base))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(covered[0]["covered_days"], 7);
    assert_eq!(covered[0]["roi_percent"], 150.0);
    let mut zero = cost.clone();
    zero["revision"] = json!(3);
    zero["amount_minor"] = json!(0);
    h.post(&c, &endpoint)
        .json(&zero)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let zero_return: Value = read("campaign-costs")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(zero_return[0]["roi_percent"].is_null());
    assert!(zero_return[0]["roas"].is_null());
    for (field, value) in [
        ("amount_minor", json!(-1)),
        ("currency", json!("XXX")),
        ("day", json!(today + chrono::Duration::days(1))),
    ] {
        let mut invalid = cost.clone();
        invalid[field] = value;
        assert_eq!(
            h.post(&c, &endpoint)
                .json(&invalid)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::BAD_REQUEST
        );
    }
    // A cost in a currency with no observed revenue cannot imply FX or -100% ROI.
    let mut eur = cost.clone();
    eur["reference"] = json!("eur-only");
    eur["currency"] = json!("EUR");
    h.post(&c, &endpoint)
        .json(&eur)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let separated: Value = read("campaign-costs")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let eur = separated
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["currency"] == "EUR")
        .unwrap();
    assert!(eur["roi_percent"].is_null());
    assert_eq!(eur["revenue_recorded"], false);
    let user: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email='viewer@report.test'")
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        outsider
            .get(format!("{}{endpoint}", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    sqlx::query("INSERT INTO memberships(organization_id,user_id,role) VALUES($1,$2,'viewer')")
        .bind(org)
        .bind(user)
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        h.post(&outsider, &endpoint)
            .json(&cost)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        c.post(format!("{}{endpoint}", h.base))
            .json(&cost)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    let hash: String = sqlx::query_scalar("SELECT reference_hash FROM campaign_costs WHERE id=$1")
        .bind(Uuid::parse_str(first["id"].as_str().unwrap()).unwrap())
        .fetch_one(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(hash.len(), 64);
    assert!(!hash.contains("ledger"));
    // A backend credential which passed its first check cannot write after
    // waiting behind its revocation at the organization acceptance lock.
    let mut lock = h.store.pg.begin().await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(org)
        .fetch_one(&mut *lock)
        .await
        .unwrap();
    let mut blocked_cost = cost.clone();
    blocked_cost["reference"] = json!("blocked-write");
    let url = format!("{}{endpoint}", h.base);
    let secret = key.to_owned();
    let client = backend.clone();
    let request = tokio::spawn(async move {
        client
            .post(url)
            .bearer_auth(secret)
            .json(&blocked_cost)
            .send()
            .await
            .unwrap()
    });
    let mut waiting = false;
    for _ in 0..200 {
        waiting=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'SELECT id FROM organizations%')").fetch_one(&h.store.pg).await.unwrap();
        if waiting {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(waiting);
    sqlx::query("UPDATE api_keys SET revoked_at=now() WHERE website_id=$1")
        .bind(id)
        .execute(&mut *lock)
        .await
        .unwrap();
    lock.commit().await.unwrap();
    assert_eq!(
        request.await.unwrap().status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let blocked_hash = click_core::digest(&format!("{id}:ledger:blocked-write"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM campaign_costs WHERE website_id=$1 AND reference_hash=$2"
        )
        .bind(id)
        .bind(blocked_hash)
        .fetch_one(&h.store.pg)
        .await
        .unwrap(),
        0
    );
    sqlx::query("DELETE FROM websites WHERE id=$1")
        .bind(id)
        .execute(&h.store.pg)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM campaign_costs")
            .fetch_one(&h.store.pg)
            .await
            .unwrap(),
        0
    );
    h.finish().await;
}

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn mature_cohorts_and_vital_history_respect_identity_and_retention_boundaries() {
    let h = Harness::new().await;
    let c = h.client();
    h.register(&c, "cohorts@report.test").await;
    let org = h.org(&c).await;
    let site = h.site(&c, org).await;
    let id = Uuid::parse_str(site["id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE subscriptions SET plan_id='pro' WHERE organization_id=$1")
        .bind(org)
        .execute(&h.store.pg)
        .await
        .unwrap();
    let now = chrono::Utc::now();
    use chrono::Datelike;
    let monday = now.date_naive()
        - chrono::Duration::days(now.weekday().num_days_from_monday() as i64)
        - chrono::Duration::weeks(7);
    let old = monday.and_hms_opt(1, 0, 0).unwrap().and_utc().timestamp() as u32;
    let (start, end) =
        click_server::api::range(&click_server::api::Period { days: Some(1) }).unwrap();
    let current = start + (end - start) / 2;
    let (prev_start, prev_end) = click_server::explore::previous(start, end);
    let prior = prev_start + (prev_end - prev_start) / 2;
    let mut events = vec![
        row((org, id), "a0", "a0", "a", "/", "pageview", old),
        row((org, id), "b0", "b0", "b", "/", "pageview", old),
        row((org, id), "z0", "z0", "z", "/", "pageview", end - 1),
    ];
    for week in [1, 2, 4] {
        events.push(row(
            (org, id),
            &format!("a{week}"),
            &format!("a{week}"),
            "a",
            "/",
            "pageview",
            old + week * 7 * 86400,
        ));
    }
    let mut unknown = row(
        (org, id),
        "unknown-a",
        "legacy-a",
        "a",
        "/",
        "pageview",
        old + 3 * 7 * 86400,
    );
    unknown.visitor_mode = "unknown".into();
    events.push(unknown);
    for (id, time, offset, value) in [
        ("prior-vital", prior, 0, 1000.0),
        ("current-vital-1", current, 0, 1200.0),
        ("current-vital-2", current, 1, 2000.0),
    ] {
        let mut e = row(
            (org, Uuid::parse_str(site["id"].as_str().unwrap()).unwrap()),
            id,
            "vital-session",
            "vital-visitor",
            "/",
            "web_vital",
            time,
        );
        e.metric_id = "one-instance".into();
        e.metric_name = "LCP".into();
        e.metric_value = value;
        e.occurred_ms += offset;
        events.push(e.clone());
        events.push(e);
    }
    let mut insert = h.store.ch.insert("events").unwrap();
    for event in &events {
        insert.write(event).await.unwrap();
    }
    insert.end().await.unwrap();
    let report: Value = c
        .get(format!("{}/v1/websites/{id}/retention?days=90", h.base))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let cohort = report["cohorts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["week"] == monday.to_string())
        .unwrap();
    assert_eq!(cohort["visitors"], 2);
    assert_eq!(cohort["retained"], json!([2, 1, 1, 0, 1]));
    let vitals: Value = c
        .get(format!(
            "{}/v1/websites/{id}/vitals/comparison?days=1",
            h.base
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(vitals["current"][0]["samples"], 1);
    assert_eq!(vitals["current"][0]["p75"], 2000.0);
    assert_eq!(vitals["previous"][0]["p75"], 1000.0);
    let website = h
        .store
        .collect_site(site["collection_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    let empty = click_server::explore::snapshot(&h.store, &website, end - 3, end - 2)
        .await
        .unwrap();
    assert_eq!(empty.pageviews, 0);
    assert!(empty.visible_seconds().is_none());
    sqlx::query("UPDATE websites SET retention_days=7 WHERE id=$1")
        .bind(id)
        .execute(&h.store.pg)
        .await
        .unwrap();
    let short: Value = c
        .get(format!("{}/v1/websites/{id}/retention?days=90", h.base))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(short["cohorts"].as_array().unwrap().len(), 1);
    assert_ne!(short["cohorts"][0]["week"], monday.to_string());
    sqlx::query("INSERT INTO campaign_costs(id,website_id,reference_hash,revision,request_hash,day,campaign,currency,amount_minor,source) VALUES($1,$2,'expired',1,'expired',$3,'Expired','USD',1,'test')").bind(Uuid::new_v4()).bind(id).bind(now.date_naive()-chrono::Duration::days(10)).execute(&h.store.pg).await.unwrap();
    click_server::jobs::enforce_retention(&h.store)
        .await
        .unwrap();
    let mutation_count = || {
        h.store.ch.query("SELECT count() AS count FROM system.mutations WHERE database=currentDatabase() AND table IN ('events','pageviews_hourly')").fetch_one::<click_store::Count>()
    };
    let before = mutation_count().await.unwrap().count;
    click_server::jobs::enforce_retention(&h.store)
        .await
        .unwrap();
    assert_eq!(
        mutation_count().await.unwrap().count,
        before,
        "empty sweeps must not rewrite shared parts"
    );
    let mut guard = h.store.pg.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(78422916)")
        .execute(&mut *guard)
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        click_server::jobs::enforce_retention(&h.store),
    )
    .await
    .unwrap()
    .unwrap();
    guard.commit().await.unwrap();

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM campaign_costs WHERE website_id=$1")
            .bind(id)
            .fetch_one(&h.store.pg)
            .await
            .unwrap(),
        0
    );
    h.finish().await;
}
