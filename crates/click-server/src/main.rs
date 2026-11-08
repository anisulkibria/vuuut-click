use click_server::{app, config::Config, worker, AppState};
use click_store::Store;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "click_server=info,tower_http=info".into()),
        )
        .init();
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
    let mode = std::env::args().nth(1).unwrap_or_else(|| "serve".into());
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
    if mode == "requeue-dead" {
        let count=sqlx::query("UPDATE event_outbox SET state='pending',attempts=0,available_at=now() WHERE state='dead'").execute(&store.pg).await?.rows_affected();
        tracing::info!(count, "dead-letter batches requeued");
        return Ok(());
    }
    if mode != "serve" && mode != "worker" {
        anyhow::bail!(
            "Usage: click-server [serve|worker|migrate|requeue-dead|grant-owner <email>]"
        );
    }
    let state = AppState::new(store.clone(), config.clone())?;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let background = tokio::spawn(worker(store.clone(), stop_rx.clone()));
    let control_worker = tokio::spawn(click_server::jobs::worker(
        store.clone(),
        state.mail.clone(),
        stop_rx,
    ));
    if mode == "worker" {
        shutdown_signal().await;
        stop_tx.send(true)?;
        background.await?;
        control_worker.await?;
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
    stop_tx.send(true)?;
    background.await?;
    control_worker.await?;
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
