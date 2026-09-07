use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};

use crate::trusted_proxy::TrustedNetwork;

const DEFAULT_BIND: &str = "127.0.0.1:8080";

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub data_dir: PathBuf,
    pub bind: SocketAddr,
    pub bootstrap_admin_username: String,
    pub bootstrap_admin_password: Option<String>,
    pub max_part_bytes: usize,
    pub upload_global_concurrency: usize,
    pub upload_per_account_concurrency: usize,
    pub metrics_token: Option<String>,
    pub require_https: bool,
    pub development: bool,
    pub trusted_proxy_cidrs: Vec<TrustedNetwork>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
        let data_dir = PathBuf::from(env::var("DATA_DIR").unwrap_or_else(|_| "./data".to_owned()));
        let bind = bind_address(env::var("BIND").ok())?;
        let bootstrap_admin_username = configured_administrator_username(
            env::var("BOOTSTRAP_ADMIN_USERNAME").unwrap_or_else(|_| "admin".into()),
        )?;
        let bootstrap_admin_password = env::var("BOOTSTRAP_ADMIN_PASSWORD").ok();
        if let Some(password) = &bootstrap_admin_password {
            sarmg_admin_auth::validate_password(password)
                .map_err(|error| anyhow::anyhow!("BOOTSTRAP_ADMIN_PASSWORD is invalid: {error}"))?;
        }
        let max_part_bytes = env::var("MAX_PART_BYTES")
            .unwrap_or_else(|_| (64 * 1024 * 1024).to_string())
            .parse()
            .context("MAX_PART_BYTES must be an integer")?;
        let upload_global_concurrency = positive_usize("UPLOAD_GLOBAL_CONCURRENCY", 16)?;
        let upload_per_account_concurrency = positive_usize("UPLOAD_PER_ACCOUNT_CONCURRENCY", 4)?;
        if upload_per_account_concurrency > upload_global_concurrency {
            anyhow::bail!("UPLOAD_PER_ACCOUNT_CONCURRENCY cannot exceed UPLOAD_GLOBAL_CONCURRENCY");
        }
        let metrics_token = env::var("METRICS_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let require_https: bool = env::var("REQUIRE_HTTPS")
            .unwrap_or_else(|_| "true".to_owned())
            .parse()
            .context("REQUIRE_HTTPS must be true or false")?;
        let development: bool = env::var("DEVELOPMENT")
            .unwrap_or_else(|_| "false".to_owned())
            .parse()
            .context("DEVELOPMENT must be true or false")?;
        validate_security_mode(bind, require_https, development)?;
        let trusted_proxy_cidrs = env::var("TRUSTED_PROXY_CIDRS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::parse)
            .collect::<std::result::Result<Vec<_>, String>>()
            .map_err(anyhow::Error::msg)
            .context(
                "TRUSTED_PROXY_CIDRS must be a comma-separated list of exact IP/CIDR values",
            )?;
        Ok(Self {
            database_url,
            data_dir,
            bind,
            bootstrap_admin_username,
            bootstrap_admin_password,
            max_part_bytes,
            upload_global_concurrency,
            upload_per_account_concurrency,
            metrics_token,
            require_https,
            development,
            trusted_proxy_cidrs,
        })
    }
}

fn bind_address(value: Option<String>) -> Result<SocketAddr> {
    value
        .unwrap_or_else(|| DEFAULT_BIND.to_owned())
        .parse()
        .context("BIND must be a socket address")
}

fn configured_administrator_username(value: String) -> Result<String> {
    sarmg_admin_auth::normalize_administrator_username(&value)
        .map_err(|error| anyhow::anyhow!("ADMIN_USERNAME is invalid: {error}"))
}

fn positive_usize(name: &str, default: usize) -> Result<usize> {
    let value = env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .parse::<usize>()
        .with_context(|| format!("{name} must be an integer"))?;
    if value == 0 {
        anyhow::bail!("{name} must be non-zero");
    }
    Ok(value)
}

fn validate_security_mode(bind: SocketAddr, require_https: bool, development: bool) -> Result<()> {
    if development && !bind.ip().is_loopback() {
        anyhow::bail!("DEVELOPMENT=true requires a loopback BIND address");
    }
    if !development && !require_https {
        anyhow::bail!("production mode requires REQUIRE_HTTPS=true");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{bind_address, configured_administrator_username, validate_security_mode};

    #[test]
    fn omitted_bind_is_exact_loopback_default() {
        assert_eq!(
            bind_address(None).unwrap(),
            "127.0.0.1:8080".parse().unwrap()
        );
        assert_eq!(
            bind_address(Some("192.0.2.10:8443".to_owned())).unwrap(),
            "192.0.2.10:8443".parse().unwrap()
        );
    }

    #[test]
    fn administrator_username_is_current_normalized_identity() {
        assert_eq!(
            configured_administrator_username(" Admin.Ops ".to_owned()).unwrap(),
            "admin.ops"
        );
        for invalid in ["ad", "admin@example.test", "-admin", "admin-", "管理员"] {
            assert!(configured_administrator_username(invalid.to_owned()).is_err());
        }
    }

    #[test]
    fn insecure_cookies_are_limited_to_explicit_loopback_development() {
        assert!(validate_security_mode("127.0.0.1:8080".parse().unwrap(), false, true).is_ok());
        assert!(validate_security_mode("[::1]:8080".parse().unwrap(), false, true).is_ok());
        assert!(validate_security_mode("0.0.0.0:8080".parse().unwrap(), false, true).is_err());
        assert!(validate_security_mode("127.0.0.1:8080".parse().unwrap(), false, false).is_err());
        assert!(validate_security_mode("0.0.0.0:8080".parse().unwrap(), true, false).is_ok());
    }
}
