mod admin;
mod api_access;
mod audit;
mod auth;
mod config;
mod crypto;
mod database;
mod doctor;
mod error;
mod library;
mod login_admission;
mod media_delivery;
mod metrics;
mod release;
mod rooted_fs;
mod routes;
mod runtime_lock;
mod storage;
mod trusted_proxy;
mod upload_commit;
mod web_assets;

#[cfg(test)]
mod database_tests;

use anyhow::{Context, Result};
use config::Config;
use routes::AppState;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use storage::LocalStorage;
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let command = Command::parse(std::env::args().skip(1))?;
    match &command {
        Command::ReleaseIdentity => {
            println!("{}", release::identity_json()?);
            return Ok(());
        }
        Command::ReleaseVerify(root) => {
            let identity = release::verify(root)?;
            println!("{}", release::verification_line(&identity));
            return Ok(());
        }
        Command::ReleaseVerifyInstalled(root) => {
            let identity = release::verify_installed(root)?;
            println!("{}", release::verification_line(&identity));
            return Ok(());
        }
        Command::ServeRelease(root) => {
            release::verify_runtime(root)?;
        }
        Command::Serve => release::ensure_unbound_development_serve()?,
        Command::Doctor | Command::ReconcileScan => {}
    }
    let config = Config::from_env()?;
    match command {
        Command::Doctor => {
            println!("{}", serde_json::to_string(&doctor::run(&config)?)?);
            return Ok(());
        }
        Command::ReconcileScan => {
            let _runtime_lock =
                runtime_lock::RuntimeLock::acquire(&config.database_url, &config.data_dir)?;
            let pool = database::connect(&config.database_url).await?;
            let storage = LocalStorage::new(config.data_dir.clone()).await?;
            let state = build_state(&config, pool, storage).await?;
            println!(
                "{}",
                serde_json::to_string(&upload_commit::reconcile_all(&state).await?)?
            );
            return Ok(());
        }
        Command::Serve | Command::ServeRelease(_) => {}
        Command::ReleaseIdentity
        | Command::ReleaseVerify(_)
        | Command::ReleaseVerifyInstalled(_) => {
            unreachable!("release commands return before configuration is loaded")
        }
    }
    let signals = sarmg_server_runtime::ProcessSignals::install()?;
    let listeners = sarmg_server_runtime::BoundListeners::bind([config.bind])?;
    let transport = sarmg_server_runtime::HttpServer::new(listeners, signals);
    let _runtime_lock = runtime_lock::RuntimeLock::acquire(&config.database_url, &config.data_dir)?;
    let pool = database::connect(&config.database_url).await?;
    let storage = LocalStorage::new(config.data_dir.clone()).await?;
    let state = build_state(&config, pool, storage).await?;
    let reconciliation = upload_commit::reconcile_all(&state).await?;
    info!(
        recovered = reconciliation.recovered,
        marked_unknown = reconciliation.marked_unknown,
        orphan_stages_removed = reconciliation.orphan_stages_removed,
        orphan_blobs_removed = reconciliation.orphan_blobs_removed,
        errors = reconciliation.errors,
        "upload commit reconciliation finished"
    );
    let health_pool = state.pool.clone();
    let health_storage = state.storage.clone();
    let reconcile_state = state.clone();
    let runtime =
        sarmg_server_runtime::ServerRuntime::builder(sarmg_server_runtime::ProductDescriptor {
            id: "media-backup".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            foundation_revision: env!("SARMG_FOUNDATION_REVISION").into(),
            profile: "server-control-plane".into(),
            capabilities: vec![
                "admin-persistent".into(),
                "server-runtime".into(),
                "server-health".into(),
                "mobile-ffi".into(),
            ],
        })
        .with_schema_identity(database::current_schema_identity()?)
        .register_health_check(
            "database",
            sarmg_server_runtime::health_check(move || {
                let pool = health_pool.clone();
                async move {
                    sqlx::query_scalar::<_, i64>("SELECT 1")
                        .fetch_one(&pool)
                        .await
                        .is_ok_and(|value| value == 1)
                }
            }),
        )
        .register_health_check(
            "storage",
            sarmg_server_runtime::health_check(move || {
                let storage = health_storage.clone();
                async move { storage.probe_readiness().await.is_ok() }
            }),
        )
        .register_background_task(
            "upload-reconciliation",
            sarmg_server_runtime::TaskCriticality::Degrading,
            move |mut shutdown| async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                interval.tick().await;
                loop {
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                        }
                        _ = interval.tick() => {
                            upload_commit::reconcile_all(&reconcile_state)
                                .await
                                .map_err(|error| error.to_string())?;
                        }
                    }
                }
            },
        )
        .build()
        .await?;
    let runtime_handle = runtime.handle();
    let app = routes::router(state, runtime_handle.clone())?.layer(TraceLayer::new_for_http());
    info!(address = %config.bind, "media backup server listening");
    runtime.serve(transport, app).await?;
    Ok(())
}

async fn build_state(
    config: &Config,
    pool: sqlx::SqlitePool,
    storage: LocalStorage,
) -> Result<AppState> {
    let administrator = Arc::new(sarmg_admin_core::AdministratorService::new(
        sarmg_admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
    ));
    use sarmg_admin_core::AdministratorStore as _;
    if administrator.store().administrator_count().await? == 0 {
        let password = config
            .bootstrap_admin_password
            .as_deref()
            .context("BOOTSTRAP_ADMIN_PASSWORD is required while no administrators exist")?;
        administrator
            .bootstrap_administrator(
                &config.bootstrap_admin_username,
                password,
                current_time_micros()?,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error))?;
    }
    administrator.store().validate_all_administrators().await?;
    let administrator_origin = if config.development {
        sarmg_admin_auth::AdministratorOriginMode::LoopbackDevelopmentHttp
    } else {
        sarmg_admin_auth::AdministratorOriginMode::ProductionHttps
    };
    Ok(AppState {
        secrets: crypto::SecretBox::new(&config.credentials_key),
        pool,
        storage,
        config: config.clone(),
        login_admission: login_admission::LoginAdmission::default(),
        upload_admission: routes::UploadAdmission::new(
            config.upload_global_concurrency,
            config.upload_per_account_concurrency,
        ),
        administrator,
        administrator_origin,
    })
}

fn current_time_micros() -> Result<u64> {
    let value = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros();
    u64::try_from(value).context("current time is outside SQLite range")
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Serve,
    ServeRelease(PathBuf),
    Doctor,
    ReconcileScan,
    ReleaseIdentity,
    ReleaseVerify(PathBuf),
    ReleaseVerifyInstalled(PathBuf),
}

impl Command {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self> {
        let arguments: Vec<String> = arguments.collect();
        let parsed: Result<Self> = match arguments.as_slice() {
            [] => Ok(Self::Serve),
            [command] if command == "serve" => Ok(Self::Serve),
            [command, root] if command == "serve-release" => {
                Ok(Self::ServeRelease(PathBuf::from(root)))
            }
            [command] if command == "doctor" => Ok(Self::Doctor),
            [command, subcommand] if command == "reconcile" && subcommand == "scan" => {
                Ok(Self::ReconcileScan)
            }
            [command] if command == "release-identity" => Ok(Self::ReleaseIdentity),
            [command, root] if command == "release-verify" => {
                Ok(Self::ReleaseVerify(PathBuf::from(root)))
            }
            [command, root] if command == "release-verify-installed" => {
                Ok(Self::ReleaseVerifyInstalled(PathBuf::from(root)))
            }
            _ => anyhow::bail!(
                "usage: media-backup-server [serve|serve-release RELEASE_ROOT|doctor|reconcile scan|release-identity|release-verify RELEASE_ROOT|release-verify-installed RELEASE_ROOT]"
            ),
        };
        parsed.context("invalid command")
    }
}

#[cfg(test)]
mod command_tests {
    use std::path::PathBuf;

    use super::Command;

    fn parse(arguments: &[&str]) -> anyhow::Result<Command> {
        Command::parse(arguments.iter().map(|value| (*value).to_owned()))
    }

    #[test]
    fn only_current_product_commands_are_accepted() {
        assert_eq!(parse(&[]).unwrap(), Command::Serve);
        assert_eq!(parse(&["serve"]).unwrap(), Command::Serve);
        assert_eq!(
            parse(&["serve-release", "/opt/isarmg/media-backup/releases/0.3.14"]).unwrap(),
            Command::ServeRelease(PathBuf::from("/opt/isarmg/media-backup/releases/0.3.14"))
        );
        assert_eq!(parse(&["doctor"]).unwrap(), Command::Doctor);
        assert_eq!(
            parse(&["reconcile", "scan"]).unwrap(),
            Command::ReconcileScan
        );
        assert_eq!(
            parse(&["release-identity"]).unwrap(),
            Command::ReleaseIdentity
        );
        assert_eq!(
            parse(&["release-verify", "/opt/isarmg/media-backup/releases/0.3.14"]).unwrap(),
            Command::ReleaseVerify(PathBuf::from("/opt/isarmg/media-backup/releases/0.3.14"))
        );
        assert_eq!(
            parse(&[
                "release-verify-installed",
                "/opt/isarmg/media-backup/releases/0.3.14"
            ])
            .unwrap(),
            Command::ReleaseVerifyInstalled(PathBuf::from(
                "/opt/isarmg/media-backup/releases/0.3.14"
            ))
        );
        assert!(parse(&["backup", "create"]).is_err());
        assert!(parse(&["backup", "create", "--output", "/tmp/new"]).is_err());
        assert!(parse(&["restore", "--input", "/tmp/snapshot"]).is_err());
    }
}
