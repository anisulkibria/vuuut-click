use anyhow::{bail, Context, Result};
use ipnet::IpNet;
use std::{env, path::PathBuf};
use url::Url;

#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub public_url: String,
    pub database_url: String,
    pub clickhouse_url: String,
    pub clickhouse_user: String,
    pub clickhouse_password: String,
    pub clickhouse_database: String,
    pub geoip_path: Option<PathBuf>,
    pub trusted_proxies: Vec<IpNet>,
    pub allow_localhost: bool,
    pub secure_cookie: bool,
    pub require_geoip: bool,
}
impl Config {
    pub fn from_env() -> Result<Self> {
        let public_url = env::var("PUBLIC_URL").unwrap_or_else(|_| "http://localhost:8082".into());
        let url = Url::parse(&public_url).context("PUBLIC_URL")?;
        if !matches!(url.scheme(), "https" | "http")
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            bail!("PUBLIC_URL must be an HTTP(S) origin");
        }
        let production = env::var("APP_ENV").is_ok_and(|s| s == "production");
        let allow_localhost = env::var("ALLOW_LOCALHOST").is_ok_and(|s| s == "true");
        if production && (url.scheme() != "https" || allow_localhost) {
            bail!("Production requires HTTPS and ALLOW_LOCALHOST=false");
        }
        let config = Self {
            bind: env::var("BIND").unwrap_or_else(|_| "127.0.0.1:8082".into()),
            public_url: url.origin().ascii_serialization(),
            database_url: env::var("DATABASE_URL").context("DATABASE_URL is required")?,
            clickhouse_url: env::var("CLICKHOUSE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8123".into()),
            clickhouse_user: env::var("CLICKHOUSE_USER").unwrap_or_else(|_| "click".into()),
            clickhouse_password: env::var("CLICKHOUSE_PASSWORD")
                .context("CLICKHOUSE_PASSWORD is required")?,
            clickhouse_database: env::var("CLICKHOUSE_DATABASE").unwrap_or_else(|_| "click".into()),
            geoip_path: env::var_os("GEOIP_PATH")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            trusted_proxies: env::var("TRUSTED_PROXIES")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().parse())
                .collect::<std::result::Result<_, _>>()
                .context("Invalid TRUSTED_PROXIES CIDR")?,
            allow_localhost,
            secure_cookie: url.scheme() == "https",
            require_geoip: production || env::var("REQUIRE_GEOIP").is_ok_and(|s| s == "true"),
        };
        if production
            && (config.trusted_proxies.is_empty()
                || config
                    .trusted_proxies
                    .iter()
                    .any(|net| net.prefix_len() == 0))
        {
            bail!("Production requires exact trusted reverse-proxy CIDRs, never /0");
        }

        Ok(config)
    }
}
