use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
};
use xcss_config::{ConfigSource, EnvMapping, EnvValueKind, Override};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::trusted_proxy::TrustedNetwork;

const DEFAULT_BIND: &str = "127.0.0.1:8080";

#[derive(Clone)]
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
    pub credentials_key: [u8; 32],
    pub sources: BTreeMap<String, ConfigSource>,
}

impl Config {
    pub fn from_sources(
        config: Option<&Path>,
        data_dir: Option<&Path>,
        bind: Option<SocketAddr>,
    ) -> Result<Self> {
        let config = config.map(normalize_config_path).transpose()?;
        let file = config
            .as_deref()
            .map(xcss_config::read_private_file)
            .transpose()?;
        let environment = xcss_config::read_environment(&ENVIRONMENT, |name| env::var(name).ok())?;
        let mut command_line = Vec::new();
        if let Some(path) = data_dir {
            command_line.push(Override::new(
                "/data_dir",
                path.to_string_lossy().into_owned(),
            ));
        }
        if let Some(bind) = bind {
            command_line.push(Override::new("/bind", bind.to_string()));
        }
        let loaded = xcss_config::resolve_validated(
            &Settings::default(),
            file.as_deref(),
            &environment,
            &command_line,
            validate_intrinsic,
        )?;
        let settings = loaded.value;
        let data_dir = settings.data_dir.context("data_dir is required")?;
        anyhow::ensure!(data_dir.is_absolute(), "data_dir must be absolute");
        let database_url = settings
            .database_url
            .unwrap_or_else(|| format!("sqlite://{}", data_dir.join("xszs.sqlite3").display()));
        crate::database::database_path(&database_url)?;
        let bind = bind_address(Some(settings.bind))?;
        let bootstrap_admin_username =
            configured_administrator_username(settings.bootstrap_admin_username)?;
        let bootstrap_admin_password = settings.bootstrap_admin_password;
        if let Some(password) = &bootstrap_admin_password {
            xcss_admin_auth::validate_password(password)?;
        }
        anyhow::ensure!(
            settings.max_part_bytes > 0,
            "max_part_bytes must be positive"
        );
        anyhow::ensure!(
            settings.upload_global_concurrency > 0 && settings.upload_per_account_concurrency > 0,
            "upload concurrency must be positive"
        );
        anyhow::ensure!(
            settings.upload_per_account_concurrency <= settings.upload_global_concurrency,
            "upload_per_account_concurrency cannot exceed upload_global_concurrency"
        );
        validate_security_mode(bind, settings.require_https, settings.development)?;
        let trusted_proxy_cidrs = settings
            .trusted_proxy_cidrs
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::parse)
            .collect::<std::result::Result<Vec<TrustedNetwork>, String>>()
            .map_err(anyhow::Error::msg)?;
        let decoded_key = STANDARD
            .decode(
                settings
                    .credentials_key
                    .context("credentials_key is required")?,
            )
            .context("credentials_key must be valid base64")?;
        let credentials_key = decoded_key
            .try_into()
            .map_err(|_| anyhow::anyhow!("credentials_key must decode to exactly 32 bytes"))?;
        Ok(Self {
            database_url,
            data_dir,
            bind,
            bootstrap_admin_username,
            bootstrap_admin_password,
            max_part_bytes: settings.max_part_bytes,
            upload_global_concurrency: settings.upload_global_concurrency,
            upload_per_account_concurrency: settings.upload_per_account_concurrency,
            metrics_token: settings.metrics_token,
            require_https: settings.require_https,
            development: settings.development,
            trusted_proxy_cidrs,
            credentials_key,
            sources: loaded.sources,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    database_url: Option<String>,
    data_dir: Option<PathBuf>,
    bind: String,
    bootstrap_admin_username: String,
    bootstrap_admin_password: Option<String>,
    max_part_bytes: usize,
    upload_global_concurrency: usize,
    upload_per_account_concurrency: usize,
    metrics_token: Option<String>,
    require_https: bool,
    development: bool,
    trusted_proxy_cidrs: String,
    credentials_key: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            database_url: None,
            data_dir: None,
            bind: DEFAULT_BIND.into(),
            bootstrap_admin_username: "admin".into(),
            bootstrap_admin_password: None,
            max_part_bytes: 64 * 1024 * 1024,
            upload_global_concurrency: 16,
            upload_per_account_concurrency: 4,
            metrics_token: None,
            require_https: true,
            development: false,
            trusted_proxy_cidrs: String::new(),
            credentials_key: None,
        }
    }
}

const ENVIRONMENT: [EnvMapping<'static>; 13] = [
    EnvMapping {
        variable: "DATABASE_URL",
        path: "/database_url",
        kind: EnvValueKind::String,
    },
    EnvMapping {
        variable: "DATA_DIR",
        path: "/data_dir",
        kind: EnvValueKind::String,
    },
    EnvMapping {
        variable: "BIND",
        path: "/bind",
        kind: EnvValueKind::String,
    },
    EnvMapping {
        variable: "BOOTSTRAP_ADMIN_USERNAME",
        path: "/bootstrap_admin_username",
        kind: EnvValueKind::String,
    },
    EnvMapping {
        variable: "BOOTSTRAP_ADMIN_PASSWORD",
        path: "/bootstrap_admin_password",
        kind: EnvValueKind::String,
    },
    EnvMapping {
        variable: "MAX_PART_BYTES",
        path: "/max_part_bytes",
        kind: EnvValueKind::UnsignedInteger,
    },
    EnvMapping {
        variable: "UPLOAD_GLOBAL_CONCURRENCY",
        path: "/upload_global_concurrency",
        kind: EnvValueKind::UnsignedInteger,
    },
    EnvMapping {
        variable: "UPLOAD_PER_ACCOUNT_CONCURRENCY",
        path: "/upload_per_account_concurrency",
        kind: EnvValueKind::UnsignedInteger,
    },
    EnvMapping {
        variable: "METRICS_TOKEN",
        path: "/metrics_token",
        kind: EnvValueKind::String,
    },
    EnvMapping {
        variable: "REQUIRE_HTTPS",
        path: "/require_https",
        kind: EnvValueKind::Boolean,
    },
    EnvMapping {
        variable: "DEVELOPMENT",
        path: "/development",
        kind: EnvValueKind::Boolean,
    },
    EnvMapping {
        variable: "TRUSTED_PROXY_CIDRS",
        path: "/trusted_proxy_cidrs",
        kind: EnvValueKind::String,
    },
    EnvMapping {
        variable: "XSZS_CREDENTIALS_KEY",
        path: "/credentials_key",
        kind: EnvValueKind::String,
    },
];

fn bind_address(value: Option<String>) -> Result<SocketAddr> {
    value
        .unwrap_or_else(|| DEFAULT_BIND.to_owned())
        .parse()
        .context("BIND must be a socket address")
}

fn configured_administrator_username(value: String) -> Result<String> {
    xcss_admin_auth::normalize_administrator_username(&value)
        .map_err(|error| anyhow::anyhow!("ADMIN_USERNAME is invalid: {error}"))
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

fn validate_intrinsic(
    settings: &Settings,
    source: ConfigSource,
) -> Result<(), xcss_config::ConfigError> {
    let invalid =
        |path| xcss_config::ConfigError::new(xcss_config::Reason::InvalidValue, path, source);
    settings
        .bind
        .parse::<SocketAddr>()
        .map_err(|_| invalid("/bind"))?;
    if let Some(key) = &settings.credentials_key
        && STANDARD
            .decode(key)
            .map_err(|_| invalid("/credentials_key"))?
            .len()
            != 32
    {
        return Err(invalid("/credentials_key"));
    }
    configured_administrator_username(settings.bootstrap_admin_username.clone())
        .map_err(|_| invalid("/bootstrap_admin_username"))?;
    if let Some(password) = &settings.bootstrap_admin_password {
        xcss_admin_auth::validate_password(password)
            .map_err(|_| invalid("/bootstrap_admin_password"))?;
    }
    if settings
        .data_dir
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err(invalid("/data_dir"));
    }
    if let Some(url) = &settings.database_url {
        crate::database::database_path(url).map_err(|_| invalid("/database_url"))?;
    }
    for (path, value) in [
        ("/max_part_bytes", settings.max_part_bytes),
        (
            "/upload_global_concurrency",
            settings.upload_global_concurrency,
        ),
        (
            "/upload_per_account_concurrency",
            settings.upload_per_account_concurrency,
        ),
    ] {
        if value == 0 || value > tokio::sync::Semaphore::MAX_PERMITS {
            return Err(invalid(path));
        }
    }
    for network in settings
        .trusted_proxy_cidrs
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        network
            .parse::<TrustedNetwork>()
            .map_err(|_| invalid("/trusted_proxy_cidrs"))?;
    }
    Ok(())
}

#[cfg(test)]
mod precedence_contract_tests {
    use super::*;
    #[test]
    fn a_higher_priority_override_cannot_hide_an_invalid_file_value() {
        let file = serde_json::to_vec(&serde_json::json!({"upload_global_concurrency":0})).unwrap();
        let error = xcss_config::resolve_validated(
            &Settings::default(),
            Some(&file),
            &[],
            &[Override::new(
                "/upload_global_concurrency",
                serde_json::json!(16),
            )],
            validate_intrinsic,
        )
        .err()
        .expect("invalid lower layer must fail");
        assert_eq!(error.path, "/upload_global_concurrency");
        assert_eq!(error.source, ConfigSource::File);
        assert!(
            !serde_json::to_string(&error.envelope())
                .unwrap()
                .contains("private-invalid-secret")
        );
    }
}

/// Normalize the CLI file authority before private reads and resource reporting.
pub fn normalize_config_path(path: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !path
            .components()
            .any(|part| part == std::path::Component::ParentDir),
        "config path cannot contain parent traversal"
    );
    Ok(std::path::absolute(path)?)
}
