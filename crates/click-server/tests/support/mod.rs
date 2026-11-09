#![allow(dead_code)]
use click_server::{app, config::Config, AppState};
use click_store::Store;
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use uuid::Uuid;
pub struct Harness {
    pub store: Store,
    pub state: AppState,
    pub base: String,
    admin: Store,
    database: String,
    server: tokio::task::JoinHandle<()>,
}
impl Harness {
    pub async fn new() -> Self {
        Self::provider(None).await
    }
    pub async fn provider(
        provider: Option<std::sync::Arc<dyn click_server::billing::PaymentProvider>>,
    ) -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("click_server=error")
            .try_init();
        let base_config = Config::from_env().expect("load the local development environment");
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
        let mut config = base_config;
        config.public_url = base.clone();
        config.allow_localhost = true;
        config.require_geoip = false;
        config.geoip_path = None;
        config.secure_cookie = false;
        let mut state = AppState::with_services(
            store.clone(),
            config,
            click_server::billing::Service { provider: None },
            click_server::mail::Mailer::local(std::path::PathBuf::from(".local/mail")),
        )
        .unwrap();
        if let Some(provider) = provider {
            state.billing = std::sync::Arc::new(click_server::billing::Service {
                provider: Some(provider),
            });
        }
        let router = app(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self {
            store,
            state,
            base,
            admin,
            database,
            server,
        }
    }
    pub fn client(&self) -> Client {
        Client::builder()
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }
    pub fn post(&self, c: &Client, path: &str) -> RequestBuilder {
        c.post(format!("{}{path}", self.base))
            .header("Origin", &self.base)
    }
    pub fn patch(&self, c: &Client, path: &str) -> RequestBuilder {
        c.patch(format!("{}{path}", self.base))
            .header("Origin", &self.base)
    }
    pub fn delete(&self, c: &Client, path: &str) -> RequestBuilder {
        c.delete(format!("{}{path}", self.base))
            .header("Origin", &self.base)
    }
    pub async fn register(&self, c: &Client, email: &str) -> Value {
        self.post(c,"/v1/auth/register").json(&serde_json::json!({"email":email,"password":"correct-horse-battery-staple","organization":"Test studio"})).send().await.unwrap().error_for_status().unwrap().json().await.unwrap()
    }
    pub async fn org(&self, c: &Client) -> Uuid {
        let orgs: Value = c
            .get(format!("{}/v1/organizations", self.base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        Uuid::parse_str(orgs[0]["id"].as_str().unwrap()).unwrap()
    }
    pub async fn site(&self, c: &Client, org: Uuid) -> Value {
        self.post(c, "/v1/websites")
            .json(&serde_json::json!({"organization_id":org,"name":"Test site","origin":self.base}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }
    pub async fn finish(self) {
        self.server.abort();
        self.store.pg.close().await;
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.database))
            .execute(&self.admin.pg)
            .await
            .unwrap();
        self.admin
            .ch
            .query(&format!("DROP DATABASE {}", self.database))
            .execute()
            .await
            .unwrap();
        self.admin.pg.close().await;
    }
}
