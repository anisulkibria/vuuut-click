use click_server::{app, config::Config, worker, AppState};
use click_store::Store;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Exporter and SMTP dependencies may enable a second TLS provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "click_server=info,tower_http=info".into()),
        )
        .init();
    let mode = std::env::args().nth(1).unwrap_or_else(|| "serve".into());
    if matches!(mode.as_str(), "--help" | "-h" | "help") {
        println!("Vuuut Click commands: serve | worker | migrate | drain-once | requeue-dead | seed-demo <email> | grant-owner <email> | native-platform <id> <suffix> | native-platform-disable <id> | commercial-check | commercial-mail-test | commercial-dns-test");
        println!("Load trusted private environment for service commands. Commercial payment validation requires Stripe test keys; mail probes require COMMERCIAL_TEST_EMAIL. See docs/COMMERCIAL_VALIDATION.md.");
        return Ok(());
    }
    let config = Config::from_env()?;
    let store = Store::connect(
        &config.database_url,
        &config.clickhouse_url,
        &config.clickhouse_user,
        &config.clickhouse_password,
        &config.clickhouse_database,
    )
    .await?;
    store.migrate().await?;
    let connections = std::sync::Arc::new(click_server::integrations::Connections::open()?);
    let mut configuration_state = AppState::new(store.clone(), config.clone())?;
    configuration_state.integrations = connections.clone();
    // Startup-only exporter initialization. HTTP handlers never change env.
    for (key, value) in connections
        .telemetry_values(&configuration_state)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?
    {
        std::env::set_var(key, value);
    }
    let _telemetry = click_server::telemetry::initialize()?;
    let _command_guard = if matches!(mode.as_str(), "serve" | "worker") {
        None
    } else {
        Some(
            store
                .maintenance_guard()
                .await?
                .ok_or_else(|| anyhow::anyhow!("Maintenance is active; retry command later"))?,
        )
    };
    if mode == "migrate" {
        tracing::info!("migrations applied");
        return Ok(());
    }
    if mode == "native-platform" {
        let id = std::env::args().nth(2).ok_or_else(||anyhow::anyhow!("Usage: native-platform <id> <trusted-domain-suffix>; supply NATIVE_PLATFORM_TOKEN privately in the environment"))?;
        let suffix = std::env::args()
            .nth(3)
            .ok_or_else(|| anyhow::anyhow!("Missing trusted domain suffix"))?
            .to_ascii_lowercase();
        let token = std::env::var("NATIVE_PLATFORM_TOKEN")?;
        anyhow::ensure!(
            !id.is_empty()
                && id.len() <= 100
                && token.len() >= 32
                && token.len() <= 256
                && !suffix.is_empty()
                && suffix.len() <= 253
                && suffix
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)),
            "Invalid platform registration"
        );
        let mut tx = store.pg.begin().await?;
        sqlx::query("INSERT INTO native_platforms(id,token_hash,domain_suffix) VALUES($1,$2,$3) ON CONFLICT(id) DO UPDATE SET token_hash=EXCLUDED.token_hash,domain_suffix=EXCLUDED.domain_suffix,active=true")
            .bind(&id).bind(click_core::digest(&token)).bind(&suffix).execute(&mut *tx).await?;
        // Platform rotation invalidates all previous tenant credentials. A
        // replay under the new platform token must use a higher revision.
        sqlx::query("UPDATE api_keys SET revoked_at=now() WHERE website_id IN (SELECT website_id FROM integration_mappings WHERE platform_id=$1) AND revoked_at IS NULL").bind(&id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM native_sessions WHERE website_id IN (SELECT website_id FROM integration_mappings WHERE platform_id=$1)").bind(&id).execute(&mut *tx).await?;
        tx.commit().await?;
        tracing::info!(platform_id=%id,"native platform registered; credentials were not logged");
        return Ok(());
    }
    if mode == "native-platform-disable" {
        let id = std::env::args()
            .nth(2)
            .ok_or_else(|| anyhow::anyhow!("Usage: native-platform-disable <id>"))?;
        sqlx::query("UPDATE native_platforms SET active=false WHERE id=$1")
            .bind(&id)
            .execute(&store.pg)
            .await?;
        tracing::info!(platform_id=%id,"native platform disabled");
        return Ok(());
    }
    if mode == "commercial-dns-test" {
        let id: uuid::Uuid = std::env::var("COMMERCIAL_WEBSITE_ID")?.parse()?;
        let mut site:click_store::Website=sqlx::query_as("SELECT id,organization_id,name,origin,collection_id,verification_token,verified_at,active,is_demo FROM websites WHERE id=$1 AND active AND NOT is_demo AND deletion_requested_at IS NULL").bind(id).fetch_optional(&store.pg).await?.ok_or_else(||anyhow::anyhow!("Select an active real website for DNS validation"))?;
        let origin = url::Url::parse(&site.origin)?;
        anyhow::ensure!(
            origin.scheme() == "https"
                && !matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
            "Select a real public HTTPS origin"
        );
        click_server::api::check_website_dns(&site)
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        site.verification_token = click_server::auth::secret();
        let error = click_server::api::check_website_dns(&site)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("Unexpected mismatched DNS challenge acceptance"))?;
        anyhow::ensure!(
            error.message == "DNS verification challenge does not match",
            "DNS negative check did not prove a mismatched challenge"
        );
        println!("Exact public DNS challenge passed and a different challenge was rejected. No website verification state was changed.");
        return Ok(());
    }
    if mode == "commercial-mail-test" {
        let recipient = std::env::var("COMMERCIAL_TEST_EMAIL").map_err(|_| {
            anyhow::anyhow!("Configure COMMERCIAL_TEST_EMAIL with an inbox you control")
        })?;
        click_server::mail::Mailer::from_env(false)?
            .send_probe(&recipient)
            .await?;
        println!("SMTP accepted the requested probe. Confirm receipt and sender authentication in your inbox; acceptance alone is not inbox evidence.");
        return Ok(());
    }
    if mode == "commercial-check" {
        let mut state = AppState::new(store, config)?;
        state.integrations = connections;
        state.integrations.refresh(&state).await?;
        let status = click_server::commercial::status(&state)
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
        println!("{}", serde_json::to_string_pretty(&status)?);
        if let Some(provider) = &state.billing_service().provider {
            anyhow::ensure!(
                provider.mode() == "test",
                "Commercial validation requires Stripe test mode"
            );
            let catalog = provider
                .validate_catalog()
                .await
                .map_err(|e| anyhow::anyhow!(e.message))?;
            println!("{}", serde_json::to_string_pretty(&catalog)?);
            if let Some(prices) = catalog["prices"].as_array() {
                for price in prices {
                    let plan = price["mapping"]
                        .as_str()
                        .and_then(|key| key.split_once(':'))
                        .map(|(plan, _)| plan)
                        .ok_or_else(|| anyhow::anyhow!("Invalid validated price mapping"))?;
                    let exists: bool =
                        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM plans WHERE id=$1)")
                            .bind(plan)
                            .fetch_one(&state.store.pg)
                            .await?;
                    anyhow::ensure!(
                        exists,
                        "A configured Stripe price maps to an unknown local plan"
                    );
                }
            }
            anyhow::ensure!(catalog["portal_features_ready"]==true,"Configure the default Stripe test portal for invoices, payment methods, cancellation and subscription updates");
        } else {
            anyhow::bail!("Stripe test configuration is absent; provider validation has not run");
        }
        return Ok(());
    }
    if mode == "seed-demo" {
        if !config.allow_localhost || config.secure_cookie {
            anyhow::bail!("Demo seeding is allowed only in local development");
        }
        let email = std::env::args().nth(2).ok_or_else(|| {
            anyhow::anyhow!("Usage: click-server seed-demo <existing-customer-email>")
        })?;
        let site = click_server::demo::seed(&store, &email).await?;
        tracing::info!(website_id=%site,"development demo dataset ready; collection and billable usage excluded");
        return Ok(());
    }
    if mode == "grant-owner" {
        let email = std::env::args().nth(2).ok_or_else(|| {
            anyhow::anyhow!("Usage: click-server grant-owner <existing-account-email>")
        })?;
        let id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email=$1")
            .bind(email.trim().to_lowercase())
            .fetch_optional(&store.pg)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Account does not exist; create it before granting platform ownership"
                )
            })?;
        let mut tx = store.pg.begin().await?;
        sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("UPDATE account_tokens SET used_at=now() WHERE user_id=$1 AND used_at IS NULL")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO platform_owners(user_id) VALUES($1) ON CONFLICT DO NOTHING")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM sessions WHERE user_id=$1 AND audience='user'")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::info!(user_id=%id,"platform owner granted; use /owner/signin");
        return Ok(());
    }
    if mode == "drain-once" {
        let delivered = store.drain_once().await?;
        println!("{delivered}");
        return Ok(());
    }
    if mode == "requeue-dead" {
        let count=sqlx::query("UPDATE event_outbox SET state='pending',attempts=0,available_at=now() WHERE state='dead'").execute(&store.pg).await?.rows_affected();
        tracing::info!(count, "dead-letter batches requeued");
        return Ok(());
    }
    if mode != "serve" && mode != "worker" {
        anyhow::bail!("Unknown command; run click-server --help");
    }
    let mut state = AppState::new(store.clone(), config.clone())?;
    state.integrations = connections;
    state.integrations.refresh(&state).await?;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let run_workers = match std::env::var("RUN_WORKERS").as_deref() {
        Ok("false") => false,
        Ok("true") | Err(_) => true,
        _ => anyhow::bail!("RUN_WORKERS must be true or false"),
    };
    if mode == "worker" && !run_workers {
        anyhow::bail!("worker mode requires RUN_WORKERS=true");
    }
    let background = tokio::spawn({
        let store = store.clone();
        let stop_rx = stop_rx.clone();
        async move {
            if run_workers {
                worker(store, stop_rx).await
            }
        }
    });
    let control_worker = tokio::spawn({
        let state = state.clone();
        async move {
            if run_workers {
                click_server::jobs::worker(state, stop_rx).await
            }
        }
    });
    let connections_worker = tokio::spawn(click_server::integrations::worker(
        state.clone(),
        stop_tx.subscribe(),
    ));
    if mode == "worker" {
        shutdown_signal().await;
        stop_tx.send_replace(true);
        background.await?;
        control_worker.await?;
        connections_worker.await?;
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    tracing::info!(address=%listener.local_addr()?,"Vuuut Click listening");
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    stop_tx.send_replace(true);
    background.await?;
    control_worker.await?;
    connections_worker.await?;
    Ok(())
}
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("register SIGTERM");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("register interrupt");
    }
}
