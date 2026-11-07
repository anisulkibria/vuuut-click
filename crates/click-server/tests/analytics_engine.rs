mod support;
use reqwest::StatusCode;
use serde_json::{json, Value};
use support::Harness;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires development PostgreSQL and ClickHouse"]
async fn ordered_funnels_journeys_revenue_and_latest_vital_samples() {
    let h = Harness::new().await;
    let owner = h.client();
    let foreign = h.client();
    h.register(&owner, "analyst@reports.test").await;
    h.register(&foreign, "other@reports.test").await;
    let org = h.org(&owner).await;
    let site = h.site(&owner, org).await;
    let id = site["id"].as_str().unwrap();
    let token = site["collection_id"].as_str().unwrap();
    let s1 = Uuid::new_v4();
    let s2 = Uuid::new_v4();
    let s3 = Uuid::new_v4();
    let visitor = Uuid::new_v4();
    let other_visitor = Uuid::new_v4();
    let t = chrono::Utc::now().timestamp_millis() - 30000;
    let mut events = Vec::new();
    for (session, v, time, path, source, campaign, name) in [
        (s1, visitor, t, "/", "google", "Spring launch", "pageview"),
        (s1, visitor, t + 1000, "/pricing", "", "", "pageview"),
        (s1, visitor, t + 2000, "/pricing", "", "", "trial_started"),
        (
            s2,
            visitor,
            t + 6000,
            "/landing",
            "newsletter",
            "Product letter",
            "pageview",
        ),
        (s2, visitor, t + 7000, "/pricing", "", "", "pageview"),
        (
            s3,
            other_visitor,
            t + 3000,
            "/",
            "linkedin",
            "Creator stories",
            "pageview",
        ),
    ] {
        events.push(json!({"event_id":Uuid::new_v4(),"session_id":session,"visitor_id":v,"occurred_at_ms":time,"name":name,"url":format!("{}{path}",h.base),"utm_source":source,"utm_campaign":campaign}));
    }
    for (metric_id, name, value, time) in [
        ("lcp-one", "LCP", 1000.0, t + 13000),
        ("lcp-one", "LCP", 2800.0, t + 13100),
        ("lcp-two", "LCP", 2000.0, t + 14000),
        ("cls-one", "CLS", 0.08, t + 15000),
        ("inp-one", "INP", 130.0, t + 16000),
    ] {
        events.push(json!({"event_id":Uuid::new_v4(),"session_id":s1,"visitor_id":visitor,"occurred_at_ms":time,"name":"web_vital","url":format!("{}/",h.base),"vital":{"id":metric_id,"name":name,"value":value}}));
    }
    assert_eq!(
        h.post(&owner, &format!("/v1/collect/{token}"))
            .json(&json!({"events":events}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    let duplicate: Value = h
        .post(&owner, &format!("/v1/collect/{token}"))
        .json(&json!({"events":events}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(duplicate["accepted"], 0);
    assert_eq!(duplicate["duplicates"], 11);
    let integration: Value = h
        .post(&owner, &format!("/v1/websites/{id}/keys"))
        .json(&json!({"label":"Payments","scope":"integration"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let key = integration["key"].as_str().unwrap();
    let body = json!({"external_id":"payment-1","kind":"payment","session_id":s2,"visitor_id":visitor,"path":"/pricing","amount_minor":2500,"currency":"USD","occurred_at_ms":t+8000});
    let payment = h
        .post(&owner, &format!("/v1/websites/{id}/backend-events"))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(payment.status(), StatusCode::ACCEPTED);
    let replay: Value = h
        .post(&owner, &format!("/v1/websites/{id}/backend-events"))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay["duplicates"], 1);
    for body in [
        json!({"external_id":"refund-1","kind":"refund","session_id":s2,"visitor_id":visitor,"amount_minor":500,"currency":"USD","occurred_at_ms":t+10000}),
        json!({"external_id":"payment-2","kind":"payment","session_id":s3,"visitor_id":other_visitor,"amount_minor":1000,"currency":"JPY","occurred_at_ms":t+9000}),
    ] {
        assert_eq!(
            h.post(&owner, &format!("/v1/websites/{id}/backend-events"))
                .bearer_auth(key)
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::ACCEPTED
        );
    }
    assert_eq!(
        h.post(&owner, &format!("/v1/websites/{id}/backend-events"))
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        foreign
            .get(format!("{}/v1/websites/{id}/revenue", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        foreign
            .get(format!("{}/v1/websites/{id}/journeys", h.base))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(h.store.drain_once().await.unwrap(), 14);
    let funnel=h.post(&owner,&format!("/v1/websites/{id}/funnels")).json(&json!({"name":"Visitor to trial","window_seconds":3600,"steps":[{"kind":"page","value":"/"},{"kind":"page","value":"/pricing"},{"kind":"event","value":"trial_started"}]})).send().await.unwrap();
    assert_eq!(funnel.status(), StatusCode::CREATED);
    let funnel: Value = funnel.json().await.unwrap();
    let funnel_id = funnel["id"].as_str().unwrap();
    let response = owner
        .get(format!("{}/v1/websites/{id}/funnels/{funnel_id}", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await.unwrap()
    );
    let report: Value = owner
        .get(format!("{}/v1/websites/{id}/funnels/{funnel_id}", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(report["steps"][0]["sessions"], 2);
    assert_eq!(report["steps"][1]["sessions"], 1);
    assert_eq!(report["steps"][2]["sessions"], 1);
    assert_eq!(report["steps"][2]["conversion_rate"], 50.0);
    let journeys = owner
        .get(format!("{}/v1/websites/{id}/journeys", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(
        journeys.status(),
        StatusCode::OK,
        "{}",
        journeys.text().await.unwrap()
    );
    let metrics: Value = owner
        .get(format!("{}/v1/websites/{id}/vitals", h.base))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let lcp = metrics
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric"] == "LCP")
        .unwrap();
    assert_eq!(lcp["samples"], 2);
    assert_eq!(lcp["p75"], 2800.0);
    for (model, dimension, label) in [
        ("first_touch", "source", "google"),
        ("last_touch", "source", "newsletter"),
        ("last_non_direct", "source", "newsletter"),
        ("first_touch", "campaign", "Spring launch"),
        ("last_touch", "landing", "/landing"),
    ] {
        let response = owner
            .get(format!(
                "{}/v1/websites/{id}/revenue?model={model}&dimension={dimension}",
                h.base
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{}",
            response.text().await.unwrap()
        );
        let revenue: Value = owner
            .get(format!(
                "{}/v1/websites/{id}/revenue?model={model}&dimension={dimension}",
                h.base
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let usd = revenue
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["currency"] == "USD")
            .unwrap();
        assert_eq!(usd["label"], label);
        assert_eq!(usd["total_minor"], "2000");
        assert_eq!(usd["payments"], 1);
        let jpy = revenue
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["currency"] == "JPY")
            .unwrap();
        assert_eq!(jpy["total_minor"], "1000");
    }
    // Replaying an already delivered raw row never creates an extra metric sample or revenue.
    let row: click_store::EventRow = h
        .store
        .ch
        .query(
            "SELECT ?fields FROM events FINAL WHERE website_id=? AND name='server.payment' LIMIT 1",
        )
        .bind(id)
        .fetch_one()
        .await
        .unwrap();
    let mut insert = h.store.ch.insert("events").unwrap();
    insert.write(&row).await.unwrap();
    insert.end().await.unwrap();
    let revenue: Value = owner
        .get(format!("{}/v1/websites/{id}/revenue", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let usd = revenue
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["currency"] == "USD")
        .unwrap();
    assert_eq!(usd["total_minor"], "2000");
    for event in [
        json!({"event_id":Uuid::new_v4(),"session_id":s1,"name":"server.payment","url":h.base}),
        json!({"event_id":Uuid::new_v4(),"session_id":s1,"name":"web_vital","url":h.base,"vital":{"name":"LCP","value":-1}}),
        json!({"event_id":Uuid::new_v4(),"session_id":s1,"name":"pageview","url":h.base,"occurred_at_ms":t-10*86400000}),
    ] {
        assert_eq!(
            h.post(&owner, &format!("/v1/collect/{token}"))
                .json(&json!({"events":[event]}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    for view in ["funnels", "journeys", "revenue", "performance"] {
        let page = owner
            .get(format!("{}/insights?site={id}&view={view}", h.base))
            .send()
            .await
            .unwrap();
        assert_eq!(page.status(), StatusCode::OK, "insight view {view}");
    }
    h.finish().await;
}
