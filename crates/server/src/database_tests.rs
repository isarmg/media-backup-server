use std::{path::Path, path::PathBuf, str::FromStr};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Method, Request, Response, StatusCode, header},
    response::IntoResponse,
};
use serde_json::{Value, json};
use sha2::Digest;
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

use xszs_protocol::{CreateUploadRequest, MediaKind, StorageEncoding, UploadPartSpec};

use crate::{
    admin, api_access,
    auth::AuthContext,
    config::Config,
    database, library,
    routes::{AppState, router},
    storage::LocalStorage,
    upload_commit,
    upload_commit::CommitFailpoint,
};

const ADMIN_USERNAME: &str = "test-admin";
const ADMIN_PASSWORD: &str = "test-admin-password";
const USERNAME: &str = "media-owner";
const PASSWORD: &str = "correct-horse-battery-staple";

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("xszs-sqlite-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create test workspace");
        Self { root }
    }

    fn database(&self) -> PathBuf {
        self.root.join("xszs.db")
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn storage_snapshot(root: &Path) -> Vec<(String, Option<Vec<u8>>)> {
    fn visit(root: &Path, current: &Path, entries: &mut Vec<(String, Option<Vec<u8>>)>) {
        let mut children = std::fs::read_dir(current)
            .expect("read storage directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect storage entries");
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            let path = child.path();
            let relative = path
                .strip_prefix(root)
                .expect("storage entry below root")
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = child.metadata().expect("read storage metadata");
            if metadata.is_dir() {
                entries.push((relative, None));
                visit(root, &path, entries);
            } else if metadata.is_file() {
                entries.push((
                    relative,
                    Some(std::fs::read(&path).expect("read storage file")),
                ));
            } else {
                panic!(
                    "unexpected special file in test storage: {}",
                    path.display()
                );
            }
        }
    }

    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    entries
}

async fn table_counts(pool: &SqlitePool, tables: &[&str]) -> Vec<i64> {
    let mut counts = Vec::with_capacity(tables.len());
    for table in tables {
        let count =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
                .fetch_one(pool)
                .await
                .unwrap_or_else(|error| panic!("count {table}: {error}"));
        counts.push(count);
    }
    counts
}

fn database_url(path: &Path) -> String {
    format!("sqlite://{}", path.display())
}

async fn test_state(database: &Path, data: &Path) -> (AppState, SqlitePool) {
    let database_url = database_url(database);
    if !database.exists() {
        database::initialize(&database_url).expect("explicit test initialization");
    }
    let pool = database::connect(&database_url)
        .await
        .expect("connect test SQLite");
    let storage = LocalStorage::new(data.to_path_buf())
        .await
        .expect("create test storage");
    let config = Config {
        database_url,
        data_dir: data.to_path_buf(),
        bind: "127.0.0.1:0".parse().expect("test bind address"),
        bootstrap_admin_username: ADMIN_USERNAME.to_owned(),
        bootstrap_admin_password: Some(ADMIN_PASSWORD.to_owned()),
        max_part_bytes: 1024 * 1024,
        upload_global_concurrency: 16,
        upload_per_account_concurrency: 4,
        metrics_token: None,
        require_https: false,
        development: true,
        trusted_proxy_cidrs: Vec::new(),
        credentials_key: [7; 32],
        sources: Default::default(),
    };
    let service = xcss::admin_core::AdministratorService::new(
        xcss::admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
    );
    use xcss::admin_core::AdministratorStore as _;
    if service.store().administrator_count().await.unwrap() == 0 {
        service
            .bootstrap_administrator(
                ADMIN_USERNAME,
                ADMIN_PASSWORD,
                crate::current_time_micros().unwrap(),
            )
            .await
            .unwrap();
    }
    let state = crate::build_state(&config, pool.clone(), storage)
        .await
        .expect("build xcss-backed test state");
    upload_commit::reconcile_all(&state)
        .await
        .expect("reconcile uploads on test startup");
    (state, pool)
}

async fn test_router(state: AppState) -> Router {
    let runtime =
        xcss::server_runtime::ServerRuntime::builder(xcss::server_runtime::ProductDescriptor {
            id: "xszs".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            common_revision: env!("XCSS_REVISION").into(),
            profile: "server-control-plane".into(),
            capabilities: vec![
                "embedded-web".into(),
                "admin-persistent".into(),
                "server-runtime".into(),
            ],
        })
        .build()
        .await
        .expect("build test runtime");
    router(state, runtime.handle()).expect("build current product router")
}

#[tokio::test]
async fn production_pool_applies_pragmas_and_persists_after_reopen() {
    let workspace = TestWorkspace::new();
    let database_path = workspace.database();
    assert!(!database_path.exists());

    let database_url = database_url(&database_path);
    database::initialize(&database_url).expect("explicit production initialization");
    let pool = database::connect(&database_url)
        .await
        .expect("connect production SQLite pool");
    assert!(database_path.is_file());

    let mut first = pool.acquire().await.expect("acquire first connection");
    let mut second = pool.acquire().await.expect("acquire second connection");
    for connection in [&mut *first, &mut *second] {
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut *connection)
            .await
            .expect("read journal mode");
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut *connection)
            .await
            .expect("read foreign key setting");
        let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&mut *connection)
            .await
            .expect("read busy timeout");
        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&mut *connection)
            .await
            .expect("read synchronous setting");

        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(foreign_keys, 1);
        assert_eq!(busy_timeout, 5_000);
        assert_eq!(synchronous, 2);
    }
    drop(first);
    drop(second);

    let invalid_foreign_key = sqlx::query(
        "INSERT INTO devices(\
             id, account_id, name, platform, token_hash, created_at, last_seen_at\
         ) VALUES (?, ?, 'invalid', 'test', ?, datetime('now'), datetime('now'))",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![7_u8; 32])
    .execute(&pool)
    .await;
    assert!(
        matches!(invalid_foreign_key, Err(sqlx::Error::Database(_))),
        "foreign key enforcement must reject an orphan device"
    );

    let account_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO accounts(\
             id, username, display_name, storage_path, quota_bytes, created_at\
         ) VALUES (?, 'persistent-user', 'Persistent User', 'blobs/persistent', 1, datetime('now'))",
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .expect("insert persistent account");

    let foreign_key_violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .expect("check foreign keys");
    assert!(foreign_key_violations.is_empty());
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .expect("check SQLite integrity");
    assert_eq!(integrity, "ok");

    // Wait for each SQLite worker rather than a released pool-return permit
    // before the read-only current-state validator reopens this generation.
    let mut connections = Vec::new();
    while connections.len() < pool.size() as usize {
        connections.push(pool.acquire().await.expect("drain fixture connection"));
    }
    let closed = pool.close();
    for connection in connections {
        connection
            .close()
            .await
            .expect("close fixture SQLite worker");
    }
    closed.await;
    let reopened = database::connect(&database_url)
        .await
        .expect("reopen production SQLite pool");
    let persisted: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE id = ? AND username = ?")
            .bind(account_id)
            .bind("persistent-user")
            .fetch_one(&reopened)
            .await
            .expect("read persistent account");
    assert_eq!(persisted, 1);
    reopened.close().await;
}

fn json_request(
    method: Method,
    uri: impl AsRef<str>,
    body: Value,
    bearer: Option<&str>,
    browser: Option<(&str, &str)>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header("sec-fetch-site", "same-origin");
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if let Some((cookie, csrf)) = browser {
        builder = builder
            .header(header::COOKIE, cookie)
            .header("x-csrf-token", csrf);
    }
    let mut request = builder
        .body(Body::from(body.to_string()))
        .expect("build JSON request");
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:41000"
            .parse::<std::net::SocketAddr>()
            .expect("test peer address"),
    ));
    request
}

fn authorized_request(
    method: Method,
    uri: impl AsRef<str>,
    bearer: &str,
    body: Body,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .body(body)
        .expect("build authorized request")
}

async fn send(app: &Router, request: Request<Body>, expected: StatusCode) -> Response<Body> {
    let request_description = format!("{} {}", request.method(), request.uri());
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("router should respond");
    if response.status() != expected {
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("read error response");
        panic!(
            "unexpected response status for {request_description}: expected {expected}, got {status}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    response
}

async fn json_body(response: Response<Body>) -> Value {
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read JSON response");
    serde_json::from_slice(&body).expect("decode JSON response")
}

async fn get_json(app: &Router, uri: impl AsRef<str>, bearer: &str) -> Value {
    let response = send(
        app,
        authorized_request(Method::GET, uri, bearer, Body::empty()),
        StatusCode::OK,
    )
    .await;
    json_body(response).await
}

fn empty_action_request(method: Method, uri: impl AsRef<str>, bearer: &str) -> Request<Body> {
    json_request(method, uri, json!({}), Some(bearer), None)
}

#[tokio::test]
async fn v02_wire_is_strict_across_the_real_sqlite_file_flow_and_restart() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let app = test_router(state).await;

    let mutation_tables = [
        "accounts",
        "devices",
        "_common_admin_sessions",
        "assets",
        "uploads",
        "upload_parts",
        "blobs",
        "resources",
        "audit_events",
        "account_changes",
    ];
    let empty_database = table_counts(&pool, &mutation_tables).await;
    let empty_storage = storage_snapshot(&workspace.data());

    let unknown_route = send(
        &app,
        json_request(
            Method::POST,
            "/not-a-route",
            json!({
                "username": USERNAME,
                "password": PASSWORD,
                "device_name": "Unknown Route Client",
                "platform": "test"
            }),
            None,
            None,
        ),
        StatusCode::NOT_FOUND,
    )
    .await;
    let request_id = unknown_route
        .headers()
        .get("x-request-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let unknown_route_error = json_body(unknown_route).await;
    assert!(!request_id.is_empty());
    assert_eq!(
        unknown_route_error,
        json!({
            "code": "not_found", "message": "Not Found", "retryable": false, "request_id": request_id
        })
    );

    let rejected_mobile_dto = send(
        &app,
        json_request(
            Method::POST,
            "/v1/auth/bootstrap",
            json!({
                "username": USERNAME,
                "password": PASSWORD,
                "device_name": "Loose Client",
                "platform": "test",
                "application_version": "noncurrent-version"
            }),
            None,
            None,
        ),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let request_id = rejected_mobile_dto
        .headers()
        .get("x-request-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(!request_id.is_empty());
    assert_eq!(
        json_body(rejected_mobile_dto).await,
        json!({
            "code": "contract_violation",
            "message": "The request does not satisfy the current input contract.",
            "retryable": false, "request_id": request_id,
            "details": {"reason": "UNKNOWN_FIELD"}
        })
    );
    send(
        &app,
        json_request(
            Method::POST,
            "/api/v1/auth/login",
            json!({
                "username": ADMIN_USERNAME,
                "password": ADMIN_PASSWORD,
                "unknown_session_field": true
            }),
            None,
            None,
        ),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(table_counts(&pool, &mutation_tables).await, empty_database);
    assert_eq!(storage_snapshot(&workspace.data()), empty_storage);

    let login = send(
        &app,
        json_request(
            Method::POST,
            "/api/v1/auth/login",
            json!({"username": ADMIN_USERNAME, "password": ADMIN_PASSWORD}),
            None,
            None,
        ),
        StatusCode::OK,
    )
    .await;
    let admin_cookie = login
        .headers()
        .get(header::SET_COOKIE)
        .expect("admin login cookie")
        .to_str()
        .expect("valid admin login cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();
    let administrator_session = json_body(login).await;
    assert_eq!(administrator_session.as_object().unwrap().len(), 5);
    assert_eq!(administrator_session["authenticated"], true);
    assert_eq!(administrator_session["username"], ADMIN_USERNAME);
    assert_eq!(administrator_session["role"], "admin");
    let administrator_id: String =
        sqlx::query_scalar("SELECT administrator_id FROM _common_administrators")
            .fetch_one(&pool)
            .await
            .expect("persisted administrator id");
    assert_eq!(administrator_session["user_id"], administrator_id);
    let admin_csrf = administrator_session["csrf_token"]
        .as_str()
        .expect("admin CSRF token")
        .to_owned();
    let mut missing_csrf = json_request(
        Method::POST,
        "/api/v1/admin/instances",
        json!({}),
        None,
        Some((&admin_cookie, &admin_csrf)),
    );
    missing_csrf.headers_mut().remove("x-csrf-token");
    let rejected = send(&app, missing_csrf, StatusCode::FORBIDDEN).await;
    assert_eq!(json_body(rejected).await["code"], "auth.csrf_rejected");
    send(
        &app,
        json_request(
            Method::POST,
            "/api/v1/admin/instances",
            json!({
                "username": "loose-admin-created-user",
                "password": PASSWORD,
                "name": "Loose Admin DTO",
                "storage_path": "blobs/loose-admin-dto",
                "quota_bytes": 1,
                "role": "user"
            }),
            None,
            Some((&admin_cookie, &admin_csrf)),
        ),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM accounts")
            .fetch_one(&pool)
            .await
            .expect("count accounts after rejected admin DTO"),
        0
    );
    assert_eq!(storage_snapshot(&workspace.data()), empty_storage);

    let created_instance = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/api/v1/admin/instances",
                json!({"name": "Media Owner"}),
                None,
                Some((&admin_cookie, &admin_csrf)),
            ),
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    let instance_id = Uuid::from_str(
        created_instance["id"]
            .as_str()
            .expect("created instance id"),
    )
    .expect("valid instance id");
    let account_id: Uuid = sqlx::query_scalar("SELECT account_id FROM devices WHERE id=?")
        .bind(instance_id)
        .fetch_one(&pool)
        .await
        .expect("created instance account");
    let admin_logs = json_body(
        send(
            &app,
            json_request(
                Method::GET,
                "/api/v1/admin/logs",
                json!({}),
                None,
                Some((&admin_cookie, &admin_csrf)),
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(admin_logs["logs"][0]["action"], "device.instance.create");
    assert_eq!(admin_logs["logs"][0]["entity_id"], instance_id.to_string());
    let storage_path: String = sqlx::query_scalar("SELECT storage_path FROM accounts WHERE id=?")
        .bind(account_id)
        .fetch_one(&pool)
        .await
        .expect("created instance storage path");
    let generated_username: String = sqlx::query_scalar("SELECT username FROM accounts WHERE id=?")
        .bind(account_id)
        .fetch_one(&pool)
        .await
        .expect("created internal storage identity");
    assert!(generated_username.starts_with("instance-"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT quota_bytes FROM accounts WHERE id=?")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("default instance quota"),
        100 * 1024 * 1024 * 1024_i64
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM devices")
            .fetch_one(&pool)
            .await
            .expect("count atomically created devices"),
        1
    );

    for invalid_storage_path in [
        "/tmp/xszs-outside",
        "../xszs-outside",
        "blobs//invalid",
        "uploads/account",
    ] {
        send(
            &app,
            json_request(
                Method::PUT,
                format!("/api/v1/admin/users/{account_id}"),
                json!({
                    "username": generated_username,
                    "display_name": "Media Owner",
                    "storage_path": invalid_storage_path,
                    "quota_bytes": 10_000_000
                }),
                None,
                Some((&admin_cookie, &admin_csrf)),
            ),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }

    send(
        &app,
        json_request(
            Method::PUT,
            format!("/api/v1/admin/users/{account_id}"),
            json!({
                "username": generated_username,
                "display_name": "Media Owner Updated",
                "storage_path": storage_path,
                "quota_bytes": 10_000_000
            }),
            None,
            Some((&admin_cookie, &admin_csrf)),
        ),
        StatusCode::OK,
    )
    .await;

    send(
        &app,
        json_request(
            Method::POST,
            "/api/v1/admin/users",
            json!({"name": "Removed two-step account creation"}),
            None,
            Some((&admin_cookie, &admin_csrf)),
        ),
        StatusCode::NOT_FOUND,
    )
    .await;
    send(
        &app,
        json_request(
            Method::POST,
            format!("/api/v1/admin/users/{account_id}/instances"),
            json!({"name": "Removed second creation step"}),
            None,
            Some((&admin_cookie, &admin_csrf)),
        ),
        StatusCode::NOT_FOUND,
    )
    .await;
    let disposable = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/api/v1/admin/instances",
                json!({"name": "Disposable Instance"}),
                None,
                Some((&admin_cookie, &admin_csrf)),
            ),
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    let disposable_id = Uuid::from_str(disposable["id"].as_str().expect("disposable instance id"))
        .expect("valid disposable instance id");
    let disposable_account_id: Uuid =
        sqlx::query_scalar("SELECT account_id FROM devices WHERE id=?")
            .bind(disposable_id)
            .fetch_one(&pool)
            .await
            .expect("disposable account id");
    let disposable_username: String =
        sqlx::query_scalar("SELECT username FROM accounts WHERE id=?")
            .bind(disposable_account_id)
            .fetch_one(&pool)
            .await
            .expect("disposable internal username");
    send(
        &app,
        json_request(
            Method::PUT,
            format!("/api/v1/admin/users/{disposable_account_id}"),
            json!({
                "username": disposable_username,
                "display_name": "Disposable Instance",
                "storage_path": format!("{storage_path}/nested"),
                "quota_bytes": 1
            }),
            None,
            Some((&admin_cookie, &admin_csrf)),
        ),
        StatusCode::CONFLICT,
    )
    .await;
    for expected_accounts in [2_i64, 1_i64] {
        send(
            &app,
            json_request(
                Method::DELETE,
                format!("/api/v1/admin/instances/{disposable_id}"),
                json!({}),
                None,
                Some((&admin_cookie, &admin_csrf)),
            ),
            StatusCode::NO_CONTENT,
        )
        .await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM accounts")
                .fetch_one(&pool)
                .await
                .expect("instance lifecycle account count"),
            expected_accounts
        );
    }
    let bootstrap = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/auth/bootstrap",
                json!({
                    "authorization_code": created_instance["authorization_code"],
                    "device_name": "Test Phone",
                    "platform": "test"
                }),
                None,
                None,
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(bootstrap["account_id"], account_id.to_string());
    let bearer = bootstrap["bearer_token"]
        .as_str()
        .expect("bootstrap bearer token")
        .to_owned();
    sqlx::query("UPDATE devices SET last_seen_at = '2000-01-01 00:00:00'")
        .execute(&pool)
        .await
        .expect("set device observation sentinel");

    let pre_upload_storage = storage_snapshot(&workspace.data());
    for rejected_upload in [
        json!({
            "source_asset_id": "unsupported-asset",
            "source_resource_id": "unsupported-resource",
            "media_kind": "photo",
            "role": "primary",
            "filename": "unsupported.jpg",
            "mime_type": "image/jpeg",
            "source_created_at_ms": 1_700_000_000_000_i64,
            "plaintext_size": 1,
            "dedup_token": "unsupported",
            "wrapped_key": "unsupported",
            "key_nonce": "unsupported",
            "nonce_prefix": "unsupported",
            "metadata_nonce": null,
            "metadata_ciphertext": null,
            "parts": [{
                "index": 0,
                "ciphertext_size": 1,
                "ciphertext_blake3": "0".repeat(64)
            }]
        }),
        json!({
            "source_asset_id": "loose-asset",
            "source_resource_id": "loose-resource",
            "media_kind": "photo",
            "role": "primary",
            "filename": "loose.jpg",
            "mime_type": "image/jpeg",
            "source_created_at_ms": 1_700_000_000_000_i64,
            "storage_encoding": "plain-v1",
            "content_size": 1,
            "content_blake3": "0".repeat(64),
            "metadata": null,
            "parts": [{"index": 0, "size": 1, "blake3": "0".repeat(64)}],
            "dedup_token": "unknown-alias"
        }),
    ] {
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/uploads",
                rejected_upload,
                Some(&bearer),
                None,
            ),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    send(
        &app,
        json_request(
            Method::POST,
            "/uploads",
            json!({"unknown_field": true}),
            Some(&bearer),
            None,
        ),
        StatusCode::NOT_FOUND,
    )
    .await;
    for table in ["assets", "uploads", "upload_parts", "blobs", "resources"] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                "SELECT COUNT(*) FROM {table}"
            )))
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|error| panic!("count {table}: {error}")),
            0,
            "rejected unsupported or malformed upload mutated {table}"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT last_seen_at FROM devices")
            .fetch_one(&pool)
            .await
            .expect("read rejected upload device observation"),
        "2000-01-01 00:00:00",
        "rejected upload DTO recorded device use"
    );
    assert_eq!(storage_snapshot(&workspace.data()), pre_upload_storage);

    let content = b"xszs-sqlite-regression";
    let content_hash = blake3::hash(content).to_hex().to_string();
    let created_upload = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/uploads",
                json!({
                    "source_asset_id": "asset-1",
                    "source_resource_id": "resource-1",
                    "media_kind": "photo",
                    "role": "primary",
                    "filename": "photo.jpg",
                    "mime_type": "image/jpeg",
                    "source_created_at_ms": 1_750_000_000_000_i64,
                    "storage_encoding": "plain-v1",
                    "content_size": content.len(),
                    "content_blake3": content_hash,
                    "metadata": {"favorite": false},
                    "parts": [{
                        "index": 0,
                        "size": content.len(),
                        "blake3": content_hash
                    }]
                }),
                Some(&bearer),
                None,
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    let upload_id = Uuid::from_str(
        created_upload["upload_id"]
            .as_str()
            .expect("created upload id"),
    )
    .expect("valid upload id");

    send(
        &app,
        authorized_request(
            Method::PUT,
            format!("/v1/uploads/{upload_id}/parts/0"),
            &bearer,
            Body::from(content.as_slice()),
        ),
        StatusCode::NO_CONTENT,
    )
    .await;
    let staged_storage = storage_snapshot(&workspace.data());
    sqlx::query("UPDATE devices SET last_seen_at = '2000-01-02 00:00:00'")
        .execute(&pool)
        .await
        .expect("reset device observation sentinel");
    let staged_state: String = sqlx::query_scalar("SELECT commit_state FROM uploads WHERE id = ?")
        .bind(upload_id)
        .fetch_one(&pool)
        .await
        .expect("read staged upload state");
    send(
        &app,
        authorized_request(
            Method::POST,
            format!("/v1/uploads/{upload_id}/complete"),
            &bearer,
            Body::empty(),
        ),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
    )
    .await;
    send(
        &app,
        json_request(
            Method::POST,
            format!("/v1/uploads/{upload_id}/complete"),
            json!({"unknown_field": true}),
            Some(&bearer),
            None,
        ),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT commit_state FROM uploads WHERE id = ?")
            .bind(upload_id)
            .fetch_one(&pool)
            .await
            .expect("read upload state after rejected complete DTO"),
        staged_state
    );
    assert_eq!(
        table_counts(&pool, &["blobs", "resources"]).await,
        vec![0, 0]
    );
    assert_eq!(storage_snapshot(&workspace.data()), staged_storage);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT last_seen_at FROM devices")
            .fetch_one(&pool)
            .await
            .expect("read rejected completion device observation"),
        "2000-01-02 00:00:00",
        "rejected completion DTO recorded device use"
    );
    let completed = json_body(
        send(
            &app,
            empty_action_request(
                Method::POST,
                format!("/v1/uploads/{upload_id}/complete"),
                &bearer,
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    let asset_id = Uuid::from_str(completed["asset_id"].as_str().expect("completed asset id"))
        .expect("valid asset id");
    let resource_id = Uuid::from_str(
        completed["resource_id"]
            .as_str()
            .expect("completed resource id"),
    )
    .expect("valid resource id");
    let content_response = send(
        &app,
        authorized_request(
            Method::GET,
            format!("/v1/resources/{resource_id}/content"),
            &bearer,
            Body::empty(),
        ),
        StatusCode::OK,
    )
    .await;
    let downloaded = to_bytes(content_response.into_body(), 1024 * 1024)
        .await
        .expect("read downloaded blob");
    assert_eq!(downloaded.as_ref(), content);
    let persisted_blob_key: String =
        sqlx::query_scalar("SELECT storage_path FROM blobs WHERE account_id = ?")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("read persisted blob key");
    assert!(persisted_blob_key.starts_with(&format!("{storage_path}/")));
    assert!(!Path::new(&persisted_blob_key).is_absolute());

    let default_timeline = get_json(&app, "/v1/timeline", &bearer).await;
    assert_eq!(default_timeline["items"].as_array().map(Vec::len), Some(1));

    let updated_asset = json_body(
        send(
            &app,
            json_request(
                Method::PATCH,
                format!("/v1/assets/{asset_id}"),
                json!({"favorite": true, "archived": true}),
                Some(&bearer),
                None,
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(updated_asset["favorite"], true);
    assert_eq!(updated_asset["archived"], true);

    sqlx::query("UPDATE devices SET last_seen_at = '2000-01-03 00:00:00'")
        .execute(&pool)
        .await
        .expect("reset device observation sentinel");

    send(
        &app,
        json_request(
            Method::POST,
            format!("/v1/assets/{asset_id}/trash"),
            json!({"unknown_field": true}),
            Some(&bearer),
            None,
        ),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM assets WHERE id = ? AND deleted_at IS NOT NULL"
        )
        .bind(asset_id)
        .fetch_one(&pool)
        .await
        .expect("check rejected trash DTO"),
        0
    );

    send(
        &app,
        authorized_request(
            Method::GET,
            "/v1/timeline?unknown_parameter=true",
            &bearer,
            Body::empty(),
        ),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT last_seen_at FROM devices")
            .fetch_one(&pool)
            .await
            .expect("read rejected action device observation"),
        "2000-01-03 00:00:00",
        "rejected action or query recorded device use"
    );

    send(
        &app,
        empty_action_request(
            Method::POST,
            format!("/v1/assets/{asset_id}/trash"),
            &bearer,
        ),
        StatusCode::NO_CONTENT,
    )
    .await;
    let trashed = get_json(&app, "/v1/timeline?trashed=true", &bearer).await;
    assert_eq!(trashed["items"].as_array().map(Vec::len), Some(1));
    send(
        &app,
        empty_action_request(
            Method::POST,
            format!("/v1/assets/{asset_id}/restore"),
            &bearer,
        ),
        StatusCode::NO_CONTENT,
    )
    .await;

    let album = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/albums",
                json!({
                    "source_album_id": "album-1",
                    "name": "Regression Album",
                    "source_asset_ids": ["asset-1"],
                    "replace_members": true
                }),
                Some(&bearer),
                None,
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    let album_id =
        Uuid::from_str(album["album_id"].as_str().expect("album id")).expect("valid album id");

    let tag = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/tags",
                json!({"name": "regression"}),
                Some(&bearer),
                None,
            ),
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    let tag_id = Uuid::from_str(tag["tag_id"].as_str().expect("tag id")).expect("valid tag id");
    send(
        &app,
        json_request(
            Method::PUT,
            format!("/v1/tags/{tag_id}/assets"),
            json!({"asset_ids": [asset_id]}),
            Some(&bearer),
            None,
        ),
        StatusCode::NO_CONTENT,
    )
    .await;
    send(
        &app,
        json_request(
            Method::DELETE,
            format!("/v1/tags/{tag_id}/assets/{asset_id}"),
            json!({"unknown_field": true}),
            Some(&bearer),
            None,
        ),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM tag_assets WHERE tag_id = ? AND asset_id = ?"
        )
        .bind(tag_id)
        .bind(asset_id)
        .fetch_one(&pool)
        .await
        .expect("check rejected tag removal DTO"),
        1
    );
    send(
        &app,
        empty_action_request(
            Method::DELETE,
            format!("/v1/tags/{tag_id}/assets/{asset_id}"),
            &bearer,
        ),
        StatusCode::NO_CONTENT,
    )
    .await;
    send(
        &app,
        empty_action_request(
            Method::POST,
            format!("/v1/tags/{tag_id}/assets/{asset_id}"),
            &bearer,
        ),
        StatusCode::NO_CONTENT,
    )
    .await;

    let filtered_timeline = get_json(
        &app,
        format!("/v1/timeline?favorite=true&archived=true&album_id={album_id}&tag_id={tag_id}"),
        &bearer,
    )
    .await;
    assert_eq!(filtered_timeline["items"].as_array().map(Vec::len), Some(1));

    let api_key = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/api-keys",
                json!({"name": "Regression Key"}),
                Some(&bearer),
                None,
            ),
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    let api_token = api_key["token"]
        .as_str()
        .expect("created API token")
        .to_owned();
    let api_keys = get_json(&app, "/v1/api-keys", &api_token).await;
    assert_eq!(api_keys.as_array().map(Vec::len), Some(1));

    let first_audit_page = get_json(&app, "/v1/audit-events?limit=2", &bearer).await;
    assert_eq!(first_audit_page["events"].as_array().map(Vec::len), Some(2));
    let before = first_audit_page["next_sequence"]
        .as_i64()
        .expect("audit pagination cursor");
    let second_audit_page = get_json(
        &app,
        format!("/v1/audit-events?limit=2&before={before}"),
        &bearer,
    )
    .await;
    assert!(
        !second_audit_page["events"]
            .as_array()
            .expect("second audit page")
            .is_empty()
    );
    let changes = get_json(&app, "/v1/sync?after=0", &bearer).await;
    assert!(
        !changes["events"]
            .as_array()
            .expect("sync events")
            .is_empty()
    );

    for table in [
        "accounts",
        "devices",
        "assets",
        "uploads",
        "blobs",
        "resources",
        "albums",
        "album_assets",
        "tags",
        "tag_assets",
        "api_keys",
        "audit_events",
        "account_changes",
    ] {
        let count: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
                .fetch_one(&pool)
                .await
                .unwrap_or_else(|error| panic!("count {table}: {error}"));
        assert!(count > 0, "expected persisted rows in {table}");
    }
    let foreign_key_violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .expect("check foreign keys");
    assert!(foreign_key_violations.is_empty());
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .expect("check SQLite integrity");
    assert_eq!(integrity, "ok");

    drop(app);
    pool.close().await;

    let (restarted_state, restarted_pool) =
        test_state(&workspace.database(), &workspace.data()).await;
    let restarted_app = test_router(restarted_state).await;
    let persisted_timeline = get_json(&restarted_app, "/v1/timeline", &bearer).await;
    assert_eq!(
        persisted_timeline["items"].as_array().map(Vec::len),
        Some(1)
    );
    drop(restarted_app);
    restarted_pool.close().await;
}

#[tokio::test]
async fn rotating_or_revoking_an_instance_invalidates_its_api_keys() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) =
        seed_account(&pool, "blobs/key-revocation", "key-revocation").await;
    let original_token = "key-revocation-device-token";
    sqlx::query("UPDATE devices SET token_hash = ? WHERE id = ?")
        .bind(sha2::Sha256::digest(original_token.as_bytes()).to_vec())
        .bind(device_id)
        .execute(&pool)
        .await
        .unwrap();
    let app = test_router(state.clone()).await;
    let key = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/api-keys",
                json!({"name":"Before rotation"}),
                Some(original_token),
                None,
            ),
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    let key_token = key["token"].as_str().unwrap();
    get_json(&app, "/v1/devices", key_token).await;

    let _ = admin::rotate_instance_authorization(
        axum::extract::State(state.clone()),
        xcss::server_cli::ContractPath(device_id),
    )
    .await
    .expect("rotate instance authorization");
    for token in [original_token, key_token] {
        send(
            &app,
            authorized_request(Method::GET, "/v1/devices", token, Body::empty()),
            StatusCode::UNAUTHORIZED,
        )
        .await;
    }
    let revoked_at: Option<String> =
        sqlx::query_scalar("SELECT revoked_at FROM api_keys WHERE account_id = ?")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(revoked_at.is_some());

    let encrypted: Vec<u8> =
        sqlx::query_scalar("SELECT authorization_code_enc FROM devices WHERE id = ?")
            .bind(device_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let code = state
        .secrets
        .decrypt_client_authorization(&device_id.to_string(), &encrypted)
        .unwrap();
    let bootstrap = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/auth/bootstrap",
                json!({"authorization_code":code,"device_name":"Repaired","platform":"test"}),
                None,
                None,
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    let new_token = bootstrap["bearer_token"].as_str().unwrap();
    get_json(&app, "/v1/devices", new_token).await;
    send(
        &app,
        authorized_request(Method::GET, "/v1/devices", key_token, Body::empty()),
        StatusCode::UNAUTHORIZED,
    )
    .await;

    for stale in [
        AuthContext {
            account_id,
            device_id,
            actor_kind: "device".to_owned(),
            actor_id: device_id,
            credential_hash: sha2::Sha256::digest(original_token.as_bytes()).to_vec(),
        },
        AuthContext {
            account_id,
            device_id,
            actor_kind: "api_key".to_owned(),
            actor_id: Uuid::from_str(key["api_key_id"].as_str().unwrap()).unwrap(),
            credential_hash: sha2::Sha256::digest(key_token.as_bytes()).to_vec(),
        },
    ] {
        let request = serde_json::from_value(json!({"name":"Stale in-flight key"})).unwrap();
        let error = api_access::create_api_key(
            axum::extract::State(state.clone()),
            axum::extract::Extension(stale),
            xcss::server_cli::ContractJson(request),
        )
        .await
        .expect_err("old in-flight credential must not mint a replacement key");
        assert_eq!(error.status, StatusCode::UNAUTHORIZED);
    }

    admin::remove_instance(
        axum::extract::State(state.clone()),
        xcss::server_cli::ContractPath(device_id),
    )
    .await
    .expect("revoke paired instance");
    send(
        &app,
        authorized_request(Method::GET, "/v1/devices", new_token, Body::empty()),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let error = admin::rotate_instance_authorization(
        axum::extract::State(state.clone()),
        xcss::server_cli::ContractPath(device_id),
    )
    .await
    .expect_err("revoked instance must not be reauthorized");
    assert_eq!(error.status, StatusCode::CONFLICT);
    admin::remove_instance(
        axum::extract::State(state.clone()),
        xcss::server_cli::ContractPath(device_id),
    )
    .await
    .expect("delete empty revoked instance with revoked API key history");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    drop(app);
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn account_storage_path_cannot_strand_active_uploads_or_committed_media() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) = seed_account(&pool, "blobs/path-original", "path-guard").await;
    let upload_id = seed_received_upload(
        &state,
        account_id,
        device_id,
        b"preserved media",
        "path-guard",
    )
    .await;
    let changed_path = "blobs/path-changed";
    let update = |path: &str| -> admin::UpdateUserRequest {
        serde_json::from_value(json!({
            "username":"commit-account-path-guard",
            "display_name":"Path Guard",
            "storage_path":path,
            "quota_bytes":100000000
        }))
        .unwrap()
    };
    let change = || {
        admin::update_user(
            axum::extract::State(state.clone()),
            xcss::server_cli::ContractPath(account_id),
            xcss::server_cli::ContractJson(update(changed_path)),
        )
    };
    let error = change()
        .await
        .expect_err("active upload blocks path change");
    assert_eq!(error.status, StatusCode::CONFLICT);
    assert!(!workspace.data().join(changed_path).exists());

    upload_commit::complete(&state, upload_id, account_id)
        .await
        .expect("complete upload after rejected path change");
    let error = change()
        .await
        .expect_err("committed blob blocks path change");
    assert_eq!(error.status, StatusCode::CONFLICT);
    let account_path: String = sqlx::query_scalar("SELECT storage_path FROM accounts WHERE id=?")
        .bind(account_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(account_path, "blobs/path-original");
    assert!(!workspace.data().join(changed_path).exists());

    let _ = admin::update_user(
        axum::extract::State(state.clone()),
        xcss::server_cli::ContractPath(account_id),
        xcss::server_cli::ContractJson(update(&account_path)),
    )
    .await
    .expect("name and quota may still be updated without changing storage path");
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn repeat_backup_does_not_resurrect_trashed_media() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) = seed_account(&pool, "blobs/trash-repeat", "trash-repeat").await;
    let bearer = "trash-repeat-device-token";
    sqlx::query("UPDATE devices SET token_hash = ? WHERE id = ?")
        .bind(sha2::Sha256::digest(bearer.as_bytes()).to_vec())
        .bind(device_id)
        .execute(&pool)
        .await
        .unwrap();
    let upload_id = seed_received_upload(
        &state,
        account_id,
        device_id,
        b"original media",
        "trash-repeat",
    )
    .await;
    let committed = upload_commit::complete(&state, upload_id, account_id)
        .await
        .unwrap();
    let original_request: Value = sqlx::query_scalar("SELECT request FROM uploads WHERE id = ?")
        .bind(upload_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let app = test_router(state.clone()).await;
    send(
        &app,
        empty_action_request(
            Method::POST,
            format!("/v1/assets/{}/trash", committed.asset_id),
            bearer,
        ),
        StatusCode::NO_CONTENT,
    )
    .await;
    let response = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/uploads",
                original_request,
                Some(bearer),
                None,
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(response["disposition"], "complete");
    assert_eq!(
        get_json(&app, "/v1/timeline", bearer).await["items"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        get_json(&app, "/v1/timeline?trashed=true", bearer).await["items"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    drop(app);
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn corrupt_deduplicated_blob_does_not_create_or_rewrite_asset_metadata() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) = seed_account(&pool, "blobs/corrupt-dedup", "corrupt-dedup").await;
    let bearer = "corrupt-dedup-device-token";
    sqlx::query("UPDATE devices SET token_hash = ? WHERE id = ?")
        .bind(sha2::Sha256::digest(bearer.as_bytes()).to_vec())
        .bind(device_id)
        .execute(&pool)
        .await
        .unwrap();
    let content = b"original valid media";
    let upload_id =
        seed_received_upload(&state, account_id, device_id, content, "corrupt-dedup").await;
    let committed = upload_commit::complete(&state, upload_id, account_id)
        .await
        .unwrap();
    let request: Value = sqlx::query_scalar("SELECT request FROM uploads WHERE id = ?")
        .bind(upload_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let object_path: String = sqlx::query_scalar(
        "SELECT b.storage_path FROM blobs b JOIN resources r ON r.blob_id = b.id WHERE r.id = ?",
    )
    .bind(committed.resource_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    std::fs::write(workspace.data().join(object_path), b"corrupt  valid media")
        .expect("replace blob with same-size different content");
    assert_eq!(content.len(), b"corrupt  valid media".len());
    let app = test_router(state.clone()).await;

    let mut changed = request.clone();
    changed["source_created_at_ms"] = json!(2_i64);
    send(
        &app,
        json_request(
            Method::POST,
            "/v1/uploads",
            changed.clone(),
            Some(bearer),
            None,
        ),
        StatusCode::CONFLICT,
    )
    .await;
    let stored_date: i64 =
        sqlx::query_scalar("SELECT source_created_at_ms FROM assets WHERE id = ?")
            .bind(committed.asset_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored_date, 1);

    changed["source_asset_id"] = json!("another-asset-with-bad-blob");
    send(
        &app,
        json_request(Method::POST, "/v1/uploads", changed, Some(bearer), None),
        StatusCode::CONFLICT,
    )
    .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM assets WHERE account_id = ?")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    drop(app);
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn administrator_pending_bytes_match_quota_reservations() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) = seed_account(&pool, "blobs/pending-count", "pending-count").await;
    let content = b"pending bytes";
    let upload_id =
        seed_received_upload(&state, account_id, device_id, content, "pending-count").await;
    let overview = serde_json::to_value(
        admin::overview(
            axum::extract::State(state.clone()),
            xcss::server_cli::ContractQuery(admin::OverviewQuery::default()),
        )
        .await
        .unwrap()
        .0,
    )
    .unwrap();
    assert_eq!(
        overview["pending_bytes"].as_u64(),
        Some(content.len() as u64)
    );
    assert_eq!(
        overview["users"][0]["pending_bytes"].as_u64(),
        Some(content.len() as u64)
    );

    sqlx::query("UPDATE uploads SET commit_state = 'failed' WHERE id = ?")
        .bind(upload_id)
        .execute(&pool)
        .await
        .unwrap();
    let overview = serde_json::to_value(
        admin::overview(
            axum::extract::State(state.clone()),
            xcss::server_cli::ContractQuery(admin::OverviewQuery::default()),
        )
        .await
        .unwrap()
        .0,
    )
    .unwrap();
    assert_eq!(overview["pending_bytes"], 0);
    assert_eq!(overview["users"][0]["pending_bytes"], 0);
    close_test_state(state, pool).await;
}

async fn seed_account(pool: &SqlitePool, storage_path: &str, suffix: &str) -> (Uuid, Uuid) {
    let account_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounts(
            id, username, display_name, storage_path, quota_bytes, created_at
        ) VALUES (?, ?, ?, ?, 100000000, datetime('now'))
        "#,
    )
    .bind(account_id)
    .bind(format!("commit-account-{suffix}"))
    .bind(format!("Commit Account {suffix}"))
    .bind(storage_path)
    .execute(pool)
    .await
    .expect("insert commit test account");
    let device_id = Uuid::new_v4();
    let authorization_code = format!("{}0000", Uuid::new_v4().simple());
    let authorization_hash = sha2::Sha256::digest(authorization_code.as_bytes()).to_vec();
    let authorization_encrypted = crate::crypto::SecretBox::new(&[7; 32])
        .encrypt_client_authorization(&device_id.to_string(), &authorization_code)
        .expect("encrypt commit test authorization code");
    sqlx::query(
        r#"
        INSERT INTO devices(
            id, account_id, name, platform, token_hash, authorization_code_hash,
            authorization_code_enc, pairing_status, created_at, last_seen_at
        ) VALUES (?, ?, ?, 'test', ?, ?, ?, 'paired', datetime('now'), datetime('now'))
        "#,
    )
    .bind(device_id)
    .bind(account_id)
    .bind(format!("Commit Device {suffix}"))
    .bind(
        blake3::hash(format!("token-{suffix}").as_bytes())
            .as_bytes()
            .to_vec(),
    )
    .bind(authorization_hash)
    .bind(authorization_encrypted)
    .execute(pool)
    .await
    .expect("insert commit test device");
    (account_id, device_id)
}

async fn seed_received_upload(
    state: &AppState,
    account_id: Uuid,
    device_id: Uuid,
    content: &[u8],
    suffix: &str,
) -> Uuid {
    let asset_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO assets(
            id, account_id, device_id, source_asset_id, media_kind, source_created_at_ms,
            created_at, updated_at
        ) VALUES (?, ?, ?, ?, 'photo', 1, datetime('now'), datetime('now'))
        "#,
    )
    .bind(asset_id)
    .bind(account_id)
    .bind(device_id)
    .bind(format!("commit-asset-{suffix}"))
    .execute(&state.pool)
    .await
    .expect("insert commit test asset");

    let content_hash = blake3::hash(content).to_hex().to_string();
    let part = UploadPartSpec {
        index: 0,
        size: content.len() as u64,
        blake3: content_hash.clone(),
    };
    let request = CreateUploadRequest {
        source_asset_id: format!("commit-asset-{suffix}"),
        source_resource_id: format!("commit-resource-{suffix}"),
        media_kind: MediaKind::Photo,
        role: "primary".to_owned(),
        filename: format!("{suffix}.jpg"),
        mime_type: "image/jpeg".to_owned(),
        source_created_at_ms: 1,
        storage_encoding: StorageEncoding::PlainV1,
        content_size: content.len() as u64,
        content_blake3: content_hash,
        metadata: None,
        parts: vec![part.clone()],
    };
    let upload_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO uploads(
            id, account_id, device_id, asset_id, source_resource_id, content_blake3, request,
            created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
        "#,
    )
    .bind(upload_id)
    .bind(account_id)
    .bind(device_id)
    .bind(asset_id)
    .bind(&request.source_resource_id)
    .bind(&request.content_blake3)
    .bind(serde_json::to_value(&request).expect("serialize commit request"))
    .execute(&state.pool)
    .await
    .expect("insert commit test upload");
    sqlx::query(
        r#"
        INSERT INTO upload_parts(
            upload_id, part_index, expected_size, expected_blake3
        ) VALUES (?, 0, ?, ?)
        "#,
    )
    .bind(upload_id)
    .bind(i64::try_from(part.size).expect("part size fits SQLite"))
    .bind(&part.blake3)
    .execute(&state.pool)
    .await
    .expect("insert commit test part");
    state
        .storage
        .put_part(upload_id, &part, Body::from(content.to_vec()), 1024 * 1024)
        .await
        .expect("persist commit test part");
    sqlx::query(
        "UPDATE upload_parts SET received_size = expected_size, received_at = datetime('now') WHERE upload_id = ? AND part_index = 0",
    )
    .bind(upload_id)
    .execute(&state.pool)
    .await
    .expect("mark commit test part received");
    upload_id
}

async fn close_test_state(state: AppState, pool: SqlitePool) {
    drop(pool);
    // A simulated restart shares a process with the old SQLx worker threads.
    // Close each native connection explicitly before opening raw snapshot FDs;
    // a real restarted process already has no surviving source descriptors.
    let native_connections = state.pool.size();
    for _ in 0..native_connections {
        state
            .pool
            .acquire()
            .await
            .expect("drain prior native test connection")
            .close()
            .await
            .expect("close prior native test connection");
    }
    state.pool.close().await;
}

#[tokio::test]
async fn pending_uploads_are_hidden_and_identical_resources_on_distinct_assets_do_not_merge() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account, device) = seed_account(&pool, "blobs/pending-gallery", "pending-gallery").await;
    let token = "token-pending-gallery";
    sqlx::query("UPDATE devices SET token_hash=? WHERE id=?")
        .bind(sha2::Sha256::digest(token.as_bytes()).to_vec())
        .bind(device)
        .execute(&pool)
        .await
        .unwrap();
    let app = test_router(state.clone()).await;
    let content = b"same resource bytes";
    let hash = blake3::hash(content).to_hex().to_string();
    let mut uploads = Vec::new();
    for source in ["first", "second"] {
        let response = json_body(
            send(
                &app,
                json_request(
                    Method::POST,
                    "/v1/uploads",
                    json!({
                        "source_asset_id": source,
                        "source_resource_id": "shared-resource-name",
                        "media_kind": "photo",
                        "role": "primary",
                        "filename": "photo.jpg",
                        "mime_type": "image/jpeg",
                        "source_created_at_ms": 1_750_000_000_000_i64,
                        "storage_encoding": "plain-v1",
                        "content_size": content.len(),
                        "content_blake3": hash,
                        "metadata": null,
                        "parts": [{"index": 0, "size": content.len(), "blake3": hash}]
                    }),
                    Some(token),
                    None,
                ),
                StatusCode::OK,
            )
            .await,
        )
        .await;
        assert_eq!(response["disposition"], "upload");
        uploads.push(Uuid::parse_str(response["upload_id"].as_str().unwrap()).unwrap());
    }
    assert_ne!(uploads[0], uploads[1]);
    assert!(
        get_json(&app, "/v1/timeline", token).await["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        get_json(&app, "/v1/library/snapshot", token).await["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let pending_asset: Uuid =
        sqlx::query_scalar("SELECT id FROM assets WHERE account_id=? AND source_asset_id='first'")
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap();
    send(
        &app,
        authorized_request(
            Method::GET,
            format!("/v1/assets/{pending_asset}"),
            token,
            Body::empty(),
        ),
        StatusCode::NOT_FOUND,
    )
    .await;
    let album = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/albums",
                json!({"source_album_id":"camera","name":"Camera",
                    "source_asset_ids":["first","second"],"replace_members":true}),
                Some(token),
                None,
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(album["asset_count"], 0);

    let mut completed_assets = Vec::new();
    for (index, upload_id) in uploads.into_iter().enumerate() {
        send(
            &app,
            authorized_request(
                Method::PUT,
                format!("/v1/uploads/{upload_id}/parts/0"),
                token,
                Body::from(content.as_slice()),
            ),
            StatusCode::NO_CONTENT,
        )
        .await;
        let completed = json_body(
            send(
                &app,
                empty_action_request(
                    Method::POST,
                    format!("/v1/uploads/{upload_id}/complete"),
                    token,
                ),
                StatusCode::OK,
            )
            .await,
        )
        .await;
        completed_assets.push(completed["asset_id"].as_str().unwrap().to_owned());
        let timeline = get_json(&app, "/v1/timeline", token).await;
        assert_eq!(timeline["items"].as_array().unwrap().len(), index + 1);
        assert_eq!(
            get_json(&app, "/v1/albums", token).await[0]["asset_count"],
            index + 1
        );
    }
    assert_ne!(completed_assets[0], completed_assets[1]);
    let snapshot = get_json(&app, "/v1/library/snapshot", token).await;
    let items = snapshot["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert!(
        items
            .iter()
            .all(|item| item["resources"].as_array().unwrap().len() == 1)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM blobs WHERE account_id=?")
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    send(
        &app,
        empty_action_request(
            Method::POST,
            format!("/v1/assets/{}/trash", completed_assets[0]),
            token,
        ),
        StatusCode::NO_CONTENT,
    )
    .await;
    assert_eq!(
        get_json(&app, "/v1/albums", token).await[0]["asset_count"],
        1
    );
    assert_eq!(
        get_json(&app, "/v1/timeline", token).await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    drop(app);
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn upload_commit_recovers_every_durable_crash_boundary_after_restart() {
    let workspace = TestWorkspace::new();
    let (mut state, mut pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) = seed_account(&pool, "blobs/recovery", "recovery").await;

    for (index, failpoint) in [
        CommitFailpoint::CommitStarted,
        CommitFailpoint::StageFsync,
        CommitFailpoint::Finalizing,
        CommitFailpoint::Published,
        CommitFailpoint::MetadataCommitted,
    ]
    .into_iter()
    .enumerate()
    {
        let content = format!("restart-safe-upload-{index}-{}", "x".repeat(128 * 1024));
        let upload_id = seed_received_upload(
            &state,
            account_id,
            device_id,
            content.as_bytes(),
            &format!("restart-{index}"),
        )
        .await;
        let error =
            upload_commit::complete_with_failpoint(&state, upload_id, account_id, failpoint)
                .await
                .expect_err("failpoint simulates process death");
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
        let resources_before: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM resources r JOIN uploads u ON u.asset_id = r.asset_id WHERE u.id = ?",
        )
        .bind(upload_id)
        .fetch_one(&pool)
        .await
        .expect("count pre-recovery resources");
        let blobs_before: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM blobs b JOIN uploads u ON u.account_id = b.account_id AND u.commit_blob_id = b.id WHERE u.id = ?",
        )
        .bind(upload_id)
        .fetch_one(&pool)
        .await
        .expect("count pre-recovery blobs");
        let expected_metadata = i64::from(failpoint == CommitFailpoint::MetadataCommitted);
        assert_eq!(
            resources_before, expected_metadata,
            "resource must appear exactly with the proven metadata transaction"
        );
        assert_eq!(
            blobs_before, expected_metadata,
            "quota-bearing blob must appear exactly with the proven metadata transaction"
        );

        close_test_state(state, pool).await;
        (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
        let commit_state: String =
            sqlx::query_scalar("SELECT commit_state FROM uploads WHERE id = ?")
                .bind(upload_id)
                .fetch_one(&pool)
                .await
                .expect("read recovered commit state");
        assert_eq!(commit_state, "committed");
        let counts: (i64, i64) = sqlx::query_as(
            r#"
            SELECT
                (SELECT COUNT(*) FROM blobs b JOIN uploads u ON u.commit_blob_id = b.id WHERE u.id = ?1),
                (SELECT COUNT(*) FROM resources r JOIN uploads u ON u.commit_resource_id = r.id WHERE u.id = ?1)
            "#,
        )
        .bind(upload_id)
        .fetch_one(&pool)
        .await
        .expect("count recovered metadata");
        assert_eq!(counts, (1, 1));
        let (staged_key, final_key): (Option<String>, String) =
            sqlx::query_as("SELECT commit_staged_key, commit_final_key FROM uploads WHERE id = ?")
                .bind(upload_id)
                .fetch_one(&pool)
                .await
                .expect("read recovered object keys");
        if let Some(staged_key) = staged_key {
            assert!(!workspace.data().join(staged_key).exists());
        }
        assert!(workspace.data().join(final_key).is_file());
    }
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn upload_commit_serializes_retries_and_deduplicates_concurrent_uploads() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) = seed_account(&pool, "blobs/concurrent", "concurrent").await;
    let content = vec![91_u8; 256 * 1024 + 17];
    let upload_id =
        seed_received_upload(&state, account_id, device_id, &content, "same-upload").await;
    let first_state = state.clone();
    let second_state = state.clone();
    let (first, second) = tokio::join!(
        upload_commit::complete(&first_state, upload_id, account_id),
        upload_commit::complete(&second_state, upload_id, account_id),
    );
    let first = first.expect("first retry completes");
    let second = second.expect("second retry is idempotent");
    assert_eq!(first.resource_id, second.resource_id);

    let upload_two =
        seed_received_upload(&state, account_id, device_id, &content, "same-content-two").await;
    let upload_three = seed_received_upload(
        &state,
        account_id,
        device_id,
        &content,
        "same-content-three",
    )
    .await;
    let state_two = state.clone();
    let state_three = state.clone();
    let (two, three) = tokio::join!(
        upload_commit::complete(&state_two, upload_two, account_id),
        upload_commit::complete(&state_three, upload_three, account_id),
    );
    two.expect("first concurrent content commit");
    three.expect("second concurrent content commit");
    let blob_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM blobs WHERE account_id = ? AND content_blake3 = ?",
    )
    .bind(account_id)
    .bind(blake3::hash(&content).to_hex().to_string())
    .fetch_one(&pool)
    .await
    .expect("count deduplicated blobs");
    let resource_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM resources WHERE asset_id IN (SELECT asset_id FROM uploads WHERE id IN (?1, ?2, ?3))",
    )
    .bind(upload_id)
    .bind(upload_two)
    .bind(upload_three)
    .fetch_one(&pool)
    .await
    .expect("count independently committed resources");
    assert_eq!(blob_count, 1);
    assert_eq!(resource_count, 3);
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn committed_history_survives_resource_updates_and_is_not_a_recovery_scan() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) = seed_account(&pool, "blobs/history", "history").await;

    let first_upload =
        seed_received_upload(&state, account_id, device_id, b"version-a", "history-a").await;
    let first = upload_commit::complete(&state, first_upload, account_id)
        .await
        .expect("commit first resource version");
    let (asset_id, mut first_request): (Uuid, Value) =
        sqlx::query_as("SELECT asset_id, request FROM uploads WHERE id = ?")
            .bind(first_upload)
            .fetch_one(&pool)
            .await
            .unwrap();

    let second_upload =
        seed_received_upload(&state, account_id, device_id, b"version-b", "history-b").await;
    let mut second_request: Value = sqlx::query_scalar("SELECT request FROM uploads WHERE id = ?")
        .bind(second_upload)
        .fetch_one(&pool)
        .await
        .unwrap();
    second_request["source_asset_id"] = first_request["source_asset_id"].take();
    second_request["source_resource_id"] = first_request["source_resource_id"].take();
    let source_resource_id = second_request["source_resource_id"]
        .as_str()
        .unwrap()
        .to_owned();
    sqlx::query(
        "UPDATE uploads SET asset_id = ?, source_resource_id = ?, request = ? WHERE id = ?",
    )
    .bind(asset_id)
    .bind(source_resource_id)
    .bind(second_request)
    .bind(second_upload)
    .execute(&pool)
    .await
    .unwrap();

    let second = upload_commit::complete(&state, second_upload, account_id)
        .await
        .expect("commit replacement resource version");
    assert_eq!(first.resource_id, second.resource_id);
    let retried = upload_commit::complete(&state, first_upload, account_id)
        .await
        .expect("historical receipt remains independently verifiable");
    assert_eq!(retried.resource_id, first.resource_id);

    let first_object: String =
        sqlx::query_scalar("SELECT commit_final_key FROM uploads WHERE id = ?")
            .bind(first_upload)
            .fetch_one(&pool)
            .await
            .unwrap();
    std::fs::write(
        workspace.data().join(first_object),
        b"corrupt only for scan detection",
    )
    .unwrap();
    let report = upload_commit::reconcile_all(&state)
        .await
        .expect("crash recovery excludes terminal uploads");
    assert_eq!(report.recovered, 0);
    assert_eq!(report.marked_unknown, 0);
    let states: Vec<String> =
        sqlx::query_scalar("SELECT commit_state FROM uploads WHERE id IN (?1, ?2) ORDER BY id")
            .bind(first_upload)
            .bind(second_upload)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(states, vec!["committed", "committed"]);
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn late_older_upload_cannot_replace_a_newer_committed_resource() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) =
        seed_account(&pool, "blobs/ordered-commits", "ordered-commits").await;
    let older = seed_received_upload(&state, account_id, device_id, b"older", "ordered-old").await;
    let newer = seed_received_upload(&state, account_id, device_id, b"newer", "ordered-new").await;
    let (asset_id, old_request): (Uuid, Value) =
        sqlx::query_as("SELECT asset_id, request FROM uploads WHERE id = ?")
            .bind(older)
            .fetch_one(&pool)
            .await
            .unwrap();
    let mut new_request: Value = sqlx::query_scalar("SELECT request FROM uploads WHERE id = ?")
        .bind(newer)
        .fetch_one(&pool)
        .await
        .unwrap();
    new_request["source_asset_id"] = old_request["source_asset_id"].clone();
    new_request["source_resource_id"] = old_request["source_resource_id"].clone();
    let source_resource_id = new_request["source_resource_id"]
        .as_str()
        .unwrap()
        .to_owned();
    sqlx::query(
        "UPDATE uploads SET asset_id = ?, source_resource_id = ?, request = ? WHERE id = ?",
    )
    .bind(asset_id)
    .bind(source_resource_id)
    .bind(new_request)
    .bind(newer)
    .execute(&pool)
    .await
    .unwrap();

    let newest = upload_commit::complete(&state, newer, account_id)
        .await
        .expect("newer upload commits first");
    let rejected = upload_commit::complete(&state, older, account_id)
        .await
        .expect_err("older upload must be superseded");
    assert_eq!(rejected.status, StatusCode::CONFLICT);
    assert!(rejected.message().contains("superseded"));
    assert_eq!(
        json_body(rejected.into_response()).await["code"],
        "upload_superseded"
    );
    let retry = upload_commit::complete(&state, older, account_id)
        .await
        .expect_err("retry keeps the terminal superseded outcome");
    assert_eq!(
        json_body(retry.into_response()).await["code"],
        "upload_superseded"
    );
    let visible_hash: String = sqlx::query_scalar(
        "SELECT b.content_blake3 FROM resources r JOIN blobs b ON b.id = r.blob_id WHERE r.id = ?",
    )
    .bind(newest.resource_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(visible_hash, blake3::hash(b"newer").to_hex().to_string());
    let states: Vec<String> =
        sqlx::query_scalar("SELECT commit_state FROM uploads WHERE id IN (?1, ?2) ORDER BY id")
            .bind(older)
            .bind(newer)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        states
            .iter()
            .filter(|state| state.as_str() == "failed")
            .count(),
        1
    );
    assert_eq!(
        states
            .iter()
            .filter(|state| state.as_str() == "committed")
            .count(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM blobs WHERE account_id = ?")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1,
        "superseded upload blob should be reclaimed without deleting the visible version"
    );
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn reconciler_never_accepts_same_size_different_content() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) = seed_account(&pool, "blobs/conflict", "conflict").await;
    let expected = vec![17_u8; 128 * 1024 + 9];
    let conflicting = vec![23_u8; expected.len()];
    let upload_id =
        seed_received_upload(&state, account_id, device_id, &expected, "hash-conflict").await;
    upload_commit::complete_with_failpoint(
        &state,
        upload_id,
        account_id,
        CommitFailpoint::Finalizing,
    )
    .await
    .expect_err("stop before publication");
    let final_key: String = sqlx::query_scalar("SELECT commit_final_key FROM uploads WHERE id = ?")
        .bind(upload_id)
        .fetch_one(&pool)
        .await
        .expect("read final key");
    let final_path = workspace.data().join(&final_key);
    std::fs::create_dir_all(final_path.parent().expect("final parent"))
        .expect("create conflicting final parent");
    std::fs::write(&final_path, &conflicting).expect("write conflicting final blob");

    close_test_state(state, pool).await;
    let (restarted, restarted_pool) = test_state(&workspace.database(), &workspace.data()).await;
    let commit_state: String = sqlx::query_scalar("SELECT commit_state FROM uploads WHERE id = ?")
        .bind(upload_id)
        .fetch_one(&restarted_pool)
        .await
        .expect("read conflicted commit state");
    assert_eq!(commit_state, "unknown");
    let metadata_counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM blobs WHERE account_id = ?1), (SELECT COUNT(*) FROM resources r JOIN uploads u ON u.asset_id = r.asset_id WHERE u.id = ?2)",
    )
    .bind(account_id)
    .bind(upload_id)
    .fetch_one(&restarted_pool)
    .await
    .expect("count conflicted metadata");
    assert_eq!(metadata_counts, (0, 0));
    assert_eq!(
        std::fs::read(final_path).expect("read preserved conflict"),
        conflicting
    );
    close_test_state(restarted, restarted_pool).await;
}

#[tokio::test]
async fn orphan_cleanup_is_scoped_and_preserves_cross_account_references() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_a, device_a) = seed_account(&pool, "blobs/account-a", "boundary-a").await;
    let (_account_b, _device_b) = seed_account(&pool, "blobs/account-b", "boundary-b").await;
    let content = b"cross-account-stage-proof";
    let upload_id =
        seed_received_upload(&state, account_a, device_a, content, "boundary-upload").await;
    upload_commit::complete_with_failpoint(
        &state,
        upload_id,
        account_a,
        CommitFailpoint::Finalizing,
    )
    .await
    .expect_err("stop with a durable stage");
    let original_stage: String =
        sqlx::query_scalar("SELECT commit_staged_key FROM uploads WHERE id = ?")
            .bind(upload_id)
            .fetch_one(&pool)
            .await
            .expect("read original stage key");
    let foreign_stage = format!(
        "blobs/account-b/staging/commit-{upload_id}-{}.stage",
        Uuid::new_v4()
    );
    let foreign_path = workspace.data().join(&foreign_stage);
    std::fs::create_dir_all(foreign_path.parent().expect("foreign stage parent"))
        .expect("create foreign stage parent");
    std::fs::write(&foreign_path, content).expect("write foreign staged content");
    sqlx::query("UPDATE uploads SET commit_staged_key = ? WHERE id = ?")
        .bind(&foreign_stage)
        .bind(upload_id)
        .execute(&pool)
        .await
        .expect("inject cross-account staged key");
    let orphan = workspace.data().join(format!(
        "blobs/account-a/staging/commit-{}-{}.stage",
        Uuid::new_v4(),
        Uuid::new_v4()
    ));
    std::fs::create_dir_all(orphan.parent().expect("orphan parent")).expect("create orphan parent");
    std::fs::write(&orphan, b"orphan").expect("write orphan stage");

    let report = upload_commit::reconcile_all(&state)
        .await
        .expect("run account-bound reconciler");
    let commit_state: String = sqlx::query_scalar("SELECT commit_state FROM uploads WHERE id = ?")
        .bind(upload_id)
        .fetch_one(&pool)
        .await
        .expect("read boundary state");
    assert_eq!(commit_state, "unknown");
    assert!(
        foreign_path.is_file(),
        "must not delete another account's referenced file"
    );
    assert!(
        !orphan.exists(),
        "unreferenced generated stage should be removed"
    );
    assert!(
        !workspace.data().join(original_stage).exists(),
        "superseded local stage is an orphan and should be removed"
    );
    assert!(report.orphan_stages_removed >= 2);
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn orphan_blob_reclamation_keeps_a_durable_row_until_unlink_succeeds() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account_id, device_id) = seed_account(&pool, "blobs/delete-retry", "delete-retry").await;
    let content = b"durable orphan blob deletion";
    let upload_id =
        seed_received_upload(&state, account_id, device_id, content, "delete-retry").await;
    let committed = upload_commit::complete(&state, upload_id, account_id)
        .await
        .expect("commit blob before deleting its asset");
    let object_key: String =
        sqlx::query_scalar("SELECT storage_path FROM blobs WHERE account_id = ?")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("read committed object key");
    let object_path = workspace.data().join(&object_key);
    assert_eq!(std::fs::read(&object_path).unwrap(), content);

    // Asset/resource removal is the durable user-visible deletion. The now
    // unreferenced blob row remains as the retry queue until rooted unlink and
    // the metadata delete can commit together.
    sqlx::query("DELETE FROM assets WHERE id = ?")
        .bind(committed.asset_id)
        .execute(&pool)
        .await
        .expect("delete asset and cascade resource");
    std::fs::remove_file(&object_path).expect("replace blob with a failing object type");
    std::fs::create_dir(&object_path).expect("create non-regular object at blob key");

    let failed = library::reconcile_orphan_blobs(&state, Some(account_id))
        .await
        .expect("scan orphan blobs despite one storage failure");
    assert_eq!(failed.removed, 0);
    assert_eq!(failed.errors, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM blobs WHERE account_id = ?")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1,
        "filesystem failure must roll back the blob-row delete"
    );

    std::fs::remove_dir(&object_path).expect("remove injected non-regular object");
    std::fs::write(&object_path, content).expect("restore retryable blob object");
    let recovered = library::reconcile_orphan_blobs(&state, Some(account_id))
        .await
        .expect("retry orphan blob reclamation");
    assert_eq!(recovered.removed, 1);
    assert_eq!(recovered.errors, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM blobs WHERE account_id = ?")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert!(!object_path.exists());
    close_test_state(state, pool).await;
}

#[tokio::test]
async fn gallery_filters_snapshot_previews_and_streaming_preserve_account_boundaries() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let (account, device) = seed_account(&pool, "blobs/gallery", "gallery").await;
    let (_, other_device) = seed_account(&pool, "blobs/gallery-other", "gallery-other").await;
    use sha2::{Digest, Sha256};
    for (id, token) in [
        (device, "token-gallery"),
        (other_device, "token-gallery-other"),
    ] {
        sqlx::query("UPDATE devices SET token_hash=? WHERE id=?")
            .bind(Sha256::digest(token.as_bytes()).to_vec())
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2000, 1000)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let bytes = png.into_inner();
    let upload = seed_received_upload(&state, account, device, &bytes, "gallery-photo").await;
    let video_upload =
        seed_received_upload(&state, account, device, b"0123456789", "gallery-video").await;
    let app = test_router(state).await;
    let result = json_body(
        send(
            &app,
            empty_action_request(
                Method::POST,
                format!("/v1/uploads/{upload}/complete"),
                "token-gallery",
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    let video = json_body(
        send(
            &app,
            empty_action_request(
                Method::POST,
                format!("/v1/uploads/{video_upload}/complete"),
                "token-gallery",
            ),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    let resource = result["resource_id"].as_str().unwrap();
    let asset = result["asset_id"].as_str().unwrap();
    let video_id = Uuid::parse_str(video["asset_id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE assets SET media_kind='video',source_created_at_ms=500 WHERE id=?")
        .bind(video_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE resources SET mime_type='video/mp4' WHERE id=?")
        .bind(Uuid::parse_str(video["resource_id"].as_str().unwrap()).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    let page = get_json(
        &app,
        format!("/v1/timeline?media_kind=video&from_ms=400&to_ms=600&device_id={device}&limit=1"),
        "token-gallery",
    )
    .await;
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["asset_id"], video["asset_id"]);
    assert_eq!(
        get_json(
            &app,
            format!("/v1/timeline?device_id={other_device}"),
            "token-gallery"
        )
        .await["items"],
        json!([])
    );
    send(
        &app,
        authorized_request(
            Method::GET,
            "/v1/timeline?from_ms=5&to_ms=5",
            "token-gallery",
            Body::empty(),
        ),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let all = get_json(&app, "/v1/timeline", "token-gallery").await;
    for summary in all["items"].as_array().unwrap() {
        assert_eq!(
            *summary,
            get_json(
                &app,
                format!("/v1/assets/{}", summary["asset_id"].as_str().unwrap()),
                "token-gallery"
            )
            .await
        );
    }
    let head = get_json(&app, "/v1/sync/head", "token-gallery").await;
    assert_eq!(head["snapshot_protocol"], "watermark-before-uuid-walk-v1");
    let first = get_json(&app, "/v1/library/snapshot?limit=1", "token-gallery").await;
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    // A concurrent UUID behind the walk cursor is omitted by later snapshot pages,
    // but its event after the captured watermark makes it recoverable.
    let concurrent = Uuid::nil();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO assets(id,account_id,device_id,source_asset_id,media_kind,source_created_at_ms,created_at,updated_at) VALUES (?,?,?,'concurrent','photo',0,datetime('now'),datetime('now'))")
        .bind(concurrent).bind(account).bind(device).execute(&mut *tx).await.unwrap();
    let existing_blob: Uuid = sqlx::query_scalar("SELECT blob_id FROM resources WHERE id=?")
        .bind(Uuid::parse_str(resource).unwrap())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO resources(id,asset_id,blob_id,source_resource_id,role,filename,mime_type,created_at) VALUES(?,?,?,'concurrent-resource','primary','concurrent.jpg','image/jpeg',datetime('now'))")
        .bind(Uuid::new_v4()).bind(concurrent).bind(existing_blob).execute(&mut *tx).await.unwrap();
    crate::audit::record_change(&mut tx, account, "asset", concurrent, "upsert")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let second = get_json(
        &app,
        format!(
            "/v1/library/snapshot?limit=1&cursor={}",
            first["next_cursor"].as_str().unwrap()
        ),
        "token-gallery",
    )
    .await;
    assert_ne!(
        first["items"][0]["asset_id"],
        second["items"][0]["asset_id"]
    );
    assert!(second["next_cursor"].is_null());
    send(
        &app,
        json_request(
            Method::PATCH,
            format!("/v1/assets/{asset}"),
            json!({"favorite":true}),
            Some("token-gallery"),
            None,
        ),
        StatusCode::OK,
    )
    .await;
    let delta = get_json(
        &app,
        format!("/v1/sync?after={}", head["sequence"]),
        "token-gallery",
    )
    .await;
    assert!(
        delta["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["entity_id"] == asset)
    );
    assert!(
        delta["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["entity_id"] == concurrent.to_string())
    );
    assert_eq!(
        get_json(&app, format!("/v1/assets/{concurrent}"), "token-gallery").await["source_asset_id"],
        "concurrent"
    );
    let tag = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/v1/tags",
                json!({"name":"gallery-tag"}),
                Some("token-gallery"),
                None,
            ),
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    let tag_path = format!("/v1/tags/{}/assets", tag["tag_id"].as_str().unwrap());
    send(
        &app,
        json_request(
            Method::PUT,
            &tag_path,
            json!({"asset_ids":[asset,video_id]}),
            Some("token-gallery"),
            None,
        ),
        StatusCode::NO_CONTENT,
    )
    .await;
    let before_tag = get_json(&app, "/v1/sync/head", "token-gallery").await;
    send(
        &app,
        json_request(
            Method::PUT,
            &tag_path,
            json!({"asset_ids":[asset]}),
            Some("token-gallery"),
            None,
        ),
        StatusCode::NO_CONTENT,
    )
    .await;
    let tag_events = get_json(
        &app,
        format!("/v1/sync?after={}", before_tag["sequence"]),
        "token-gallery",
    )
    .await;
    assert!(
        tag_events["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["entity_id"] == video_id.to_string() && e["entity_kind"] == "asset")
    );
    assert_eq!(
        get_json(&app, format!("/v1/assets/{video_id}"), "token-gallery").await["tag_names"],
        json!([])
    );
    assert_eq!(
        get_json(&app, "/v1/devices", "token-gallery")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let path = format!("/v1/resources/{resource}/content");
    let full = send(
        &app,
        authorized_request(Method::GET, &path, "token-gallery", Body::empty()),
        StatusCode::OK,
    )
    .await;
    let etag = full.headers()[header::ETAG].clone();
    assert_eq!(
        to_bytes(full.into_body(), bytes.len() + 1)
            .await
            .unwrap()
            .as_ref(),
        bytes
    );
    let mut request = authorized_request(Method::HEAD, &path, "token-gallery", Body::empty());
    request
        .headers_mut()
        .insert(header::IF_NONE_MATCH, etag.clone());
    send(&app, request, StatusCode::NOT_MODIFIED).await;
    let head_response = send(
        &app,
        authorized_request(Method::HEAD, &path, "token-gallery", Body::empty()),
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        head_response.headers()[header::CONTENT_LENGTH]
            .to_str()
            .unwrap(),
        bytes.len().to_string()
    );
    assert!(
        to_bytes(head_response.into_body(), 1)
            .await
            .unwrap()
            .is_empty()
    );
    for (range, start, end) in [
        ("bytes=2-5", 2, 6),
        ("bytes=-4", bytes.len() - 4, bytes.len()),
        ("bytes=5-", 5, bytes.len()),
    ] {
        let mut request = authorized_request(Method::GET, &path, "token-gallery", Body::empty());
        request
            .headers_mut()
            .insert(header::RANGE, range.parse().unwrap());
        let response = send(&app, request, StatusCode::PARTIAL_CONTENT).await;
        assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
        assert_eq!(
            to_bytes(response.into_body(), bytes.len())
                .await
                .unwrap()
                .as_ref(),
            &bytes[start..end]
        );
    }
    let mut invalid = authorized_request(Method::GET, &path, "token-gallery", Body::empty());
    invalid
        .headers_mut()
        .insert(header::RANGE, "bytes=9999999-".parse().unwrap());
    let response = send(&app, invalid, StatusCode::RANGE_NOT_SATISFIABLE).await;
    assert_eq!(
        response.headers()[header::CONTENT_RANGE].to_str().unwrap(),
        format!("bytes */{}", bytes.len())
    );
    let mut stale = authorized_request(Method::GET, &path, "token-gallery", Body::empty());
    stale
        .headers_mut()
        .insert(header::RANGE, "bytes=2-5".parse().unwrap());
    stale
        .headers_mut()
        .insert(header::IF_RANGE, "\"stale\"".parse().unwrap());
    send(&app, stale, StatusCode::OK).await;
    send(
        &app,
        authorized_request(Method::GET, &path, "token-gallery-other", Body::empty()),
        StatusCode::NOT_FOUND,
    )
    .await;
    let preview_path = format!("/v1/resources/{resource}/preview");
    send(
        &app,
        authorized_request(
            Method::GET,
            &preview_path,
            "token-gallery-other",
            Body::empty(),
        ),
        StatusCode::NOT_FOUND,
    )
    .await;
    let response = send(
        &app,
        authorized_request(Method::GET, &preview_path, "token-gallery", Body::empty()),
        StatusCode::OK,
    )
    .await;
    let derived_etag = response.headers()[header::ETAG].clone();
    let preview = image::load_from_memory(
        &to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!((preview.width(), preview.height()), (1600, 800));
    let mut conditional =
        authorized_request(Method::GET, &preview_path, "token-gallery", Body::empty());
    conditional
        .headers_mut()
        .insert(header::IF_NONE_MATCH, derived_etag);
    send(&app, conditional, StatusCode::NOT_MODIFIED).await;
    send(
        &app,
        authorized_request(
            Method::GET,
            format!(
                "/v1/resources/{}/preview",
                video["resource_id"].as_str().unwrap()
            ),
            "token-gallery",
            Body::empty(),
        ),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
    )
    .await;
    let original = send(
        &app,
        authorized_request(Method::GET, &path, "token-gallery", Body::empty()),
        StatusCode::OK,
    )
    .await;
    assert_eq!(original.headers()[header::ETAG], etag);
    assert_eq!(
        to_bytes(original.into_body(), bytes.len() + 1)
            .await
            .unwrap()
            .as_ref(),
        bytes
    );
}

#[tokio::test]
async fn administrator_logs_page_every_event_on_the_selected_server_day() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let app = test_router(state).await;
    let login = send(
        &app,
        json_request(
            Method::POST,
            "/api/v1/auth/login",
            json!({"username": ADMIN_USERNAME, "password": ADMIN_PASSWORD}),
            None,
            None,
        ),
        StatusCode::OK,
    )
    .await;
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let csrf = json_body(login).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned();
    send(
        &app,
        json_request(
            Method::POST,
            "/api/v1/admin/instances",
            json!({"name": "Log fixture"}),
            None,
            Some((&cookie, &csrf)),
        ),
        StatusCode::CREATED,
    )
    .await;
    let account_id: Uuid = sqlx::query_scalar("SELECT id FROM accounts LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let today: String = sqlx::query_scalar("SELECT date('now', 'localtime')")
        .fetch_one(&pool)
        .await
        .unwrap();
    let yesterday: String = sqlx::query_scalar("SELECT date(?1, '-1 day')")
        .bind(&today)
        .fetch_one(&pool)
        .await
        .unwrap();
    let tomorrow: String = sqlx::query_scalar("SELECT date(?1, '+1 day')")
        .bind(&today)
        .fetch_one(&pool)
        .await
        .unwrap();
    for index in 0..205 {
        sqlx::query("INSERT INTO audit_events(account_id, actor_kind, action, occurred_at) VALUES (?, 'administrator', ?, datetime('now'))")
            .bind(account_id)
            .bind(format!("bulk.{index}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    for (action, date, time) in [
        ("boundary.previous", yesterday.as_str(), "23:59:59"),
        ("boundary.start", today.as_str(), "00:00:00"),
        ("boundary.next", tomorrow.as_str(), "00:00:00"),
    ] {
        let occurred_at: String = sqlx::query_scalar("SELECT datetime(?1 || ' ' || ?2, 'utc')")
            .bind(date)
            .bind(time)
            .fetch_one(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO audit_events(account_id, actor_kind, action, occurred_at) VALUES (?, 'administrator', ?, ?)")
            .bind(account_id)
            .bind(action)
            .bind(occurred_at)
            .execute(&pool)
            .await
            .unwrap();
    }
    let get_logs =
        |uri: String| json_request(Method::GET, uri, json!({}), None, Some((&cookie, &csrf)));
    let default =
        json_body(send(&app, get_logs("/api/v1/admin/logs".into()), StatusCode::OK).await).await;
    assert_eq!(default["date"], today);
    assert!(default["previous_cursor"].is_null());
    assert!(default["instance_id"].is_null());
    let mut page = default;
    let mut all_logs = Vec::new();
    loop {
        let logs = page["logs"].as_array().unwrap();
        assert!(logs.len() <= 50);
        all_logs.extend(logs.iter().cloned());
        let Some(cursor) = page["next_cursor"].as_str() else {
            break;
        };
        let next = json_body(
            send(
                &app,
                get_logs(format!("/api/v1/admin/logs?date={today}&cursor={cursor}")),
                StatusCode::OK,
            )
            .await,
        )
        .await;
        let previous_cursor = next["previous_cursor"].as_str().unwrap();
        let backwards = json_body(
            send(
                &app,
                get_logs(format!(
                    "/api/v1/admin/logs?date={today}&cursor={previous_cursor}"
                )),
                StatusCode::OK,
            )
            .await,
        )
        .await;
        assert_eq!(
            backwards["logs"], page["logs"],
            "previous restores exactly the former page"
        );
        page = next;
    }
    assert_eq!(
        all_logs.len(),
        207,
        "every record is reachable through bounded pages"
    );
    assert!(
        all_logs
            .windows(2)
            .all(|logs| logs[0]["sequence"].as_i64().unwrap()
                > logs[1]["sequence"].as_i64().unwrap()),
        "same-time events must not repeat or disappear"
    );
    assert!(
        all_logs
            .iter()
            .all(|log| log["occurred_at"].as_str().unwrap().starts_with(&today))
    );
    assert!(all_logs.iter().any(|log| log["action"] == "bulk.0"));
    assert!(all_logs.iter().any(|log| log["action"] == "boundary.start"));
    assert!(
        !all_logs
            .iter()
            .any(|log| log["action"] == "boundary.previous" || log["action"] == "boundary.next")
    );
    for cursor in [
        "bad".to_owned(),
        format!("{today}:all:older:0"),
        format!("{today}:all:older:01"),
        format!("{today}:all:newer:9223372036854775808"),
        format!("{yesterday}:all:older:1"),
    ] {
        send(
            &app,
            get_logs(format!("/api/v1/admin/logs?date={today}&cursor={cursor}")),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    send(
        &app,
        get_logs(format!("/api/v1/admin/logs?cursor={today}:all:older:1")),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let empty = json_body(
        send(
            &app,
            get_logs(format!(
                "/api/v1/admin/logs?date={today}&cursor={today}:all:older:1"
            )),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert!(empty["logs"].as_array().unwrap().is_empty());
    assert!(empty["next_cursor"].is_null() && empty["previous_cursor"].is_null());

    let device_id: Uuid = sqlx::query_scalar("SELECT id FROM devices WHERE account_id=?")
        .bind(account_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let api_key_id = Uuid::new_v4();
    sqlx::query("INSERT INTO api_keys(id,account_id,device_id,name,prefix,token_hash,created_at) VALUES(?,?,?,'fixture','fixture',?,datetime('now'))")
        .bind(api_key_id).bind(account_id).bind(device_id).bind(vec![1_u8;32]).execute(&pool).await.unwrap();
    for (actor_kind, actor_id, action) in [
        ("device", device_id, "instance.device"),
        ("api_key", api_key_id, "instance.key"),
        ("device", Uuid::new_v4(), "instance.other"),
    ] {
        sqlx::query("INSERT INTO audit_events(account_id,actor_kind,actor_id,action,occurred_at) VALUES(?,?,?,?,datetime('now'))")
            .bind(account_id).bind(actor_kind).bind(actor_id).bind(action).execute(&pool).await.unwrap();
    }
    let filtered = json_body(
        send(
            &app,
            get_logs(format!(
                "/api/v1/admin/logs?date={today}&instance_id={device_id}"
            )),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(filtered["instance_id"], device_id.to_string());
    let actions: Vec<_> = filtered["logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|log| log["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        ["instance.key", "instance.device", "device.instance.create"],
        "account ownership alone must not leak other instance events"
    );
    send(
        &app,
        get_logs(format!(
            "/api/v1/admin/logs?date={today}&instance_id={device_id}&cursor={today}:all:older:1"
        )),
        StatusCode::BAD_REQUEST,
    )
    .await;
    let unknown = json_body(
        send(
            &app,
            get_logs(format!(
                "/api/v1/admin/logs?date={today}&instance_id={}",
                Uuid::new_v4()
            )),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert!(unknown["logs"].as_array().unwrap().is_empty());

    let previous = json_body(
        send(
            &app,
            get_logs(format!("/api/v1/admin/logs?date={yesterday}")),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(previous["date"], yesterday);
    assert_eq!(previous["logs"].as_array().unwrap().len(), 1);
    assert_eq!(previous["logs"][0]["action"], "boundary.previous");
    let next = json_body(
        send(
            &app,
            get_logs(format!("/api/v1/admin/logs?date={tomorrow}")),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(next["logs"].as_array().unwrap().len(), 1);
    assert_eq!(next["logs"][0]["action"], "boundary.next");
    for date in ["2025-02-29", "2026-13-01", "0000-01-01"] {
        send(
            &app,
            get_logs(format!("/api/v1/admin/logs?date={date}")),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    for query in [
        "start_date=2022-02-01",
        "end_date=2023-02-02",
        "start_date=2023-02-02&end_date=2022-02-01",
        "start_date=2022-02-29&end_date=2023-02-02",
        "start_date=0000-01-01&end_date=2023-02-02",
        "date=2022-02-01&start_date=2022-02-01&end_date=2023-02-02",
    ] {
        send(
            &app,
            get_logs(format!("/api/v1/admin/logs?{query}")),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    let mut ranged = Vec::new();
    let mut cursor = None;
    loop {
        let suffix = cursor
            .as_ref()
            .map(|cursor| format!("&cursor={cursor}"))
            .unwrap_or_default();
        let page = json_body(
            send(
                &app,
                get_logs(format!(
                    "/api/v1/admin/logs?start_date={yesterday}&end_date={today}{suffix}"
                )),
                StatusCode::OK,
            )
            .await,
        )
        .await;
        assert_eq!(page["date"], yesterday);
        assert_eq!(page["end_date"], today);
        ranged.extend(page["logs"].as_array().unwrap().iter().cloned());
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        let Some(next) = &cursor else {
            break;
        };
        send(
            &app,
            get_logs(format!(
                "/api/v1/admin/logs?start_date={yesterday}&end_date={tomorrow}&cursor={next}"
            )),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    assert!(
        ranged
            .iter()
            .any(|row| row["action"] == "boundary.previous")
    );
    assert!(ranged.iter().any(|row| row["action"] == "boundary.start"));
    assert!(!ranged.iter().any(|row| row["action"] == "boundary.next"));
    let unique: std::collections::HashSet<_> = ranged
        .iter()
        .map(|row| row["sequence"].as_i64().unwrap())
        .collect();
    assert_eq!(unique.len(), ranged.len());
}

#[tokio::test]
async fn administrator_instance_pages_keep_global_statistics_and_direct_details() {
    let workspace = TestWorkspace::new();
    let (state, pool) = test_state(&workspace.database(), &workspace.data()).await;
    let mut accounts = Vec::new();
    for index in 0..125 {
        let (account_id, device_id) = seed_account(
            &pool,
            &format!("blobs/page-{index}"),
            &format!("page-{index}"),
        )
        .await;
        sqlx::query("UPDATE accounts SET display_name='same',quota_bytes=? WHERE id=?")
            .bind(if index == 0 { 0_i64 } else { 100_000_000 })
            .bind(account_id)
            .execute(&pool)
            .await
            .unwrap();
        if index % 2 == 1 {
            sqlx::query("UPDATE devices SET last_seen_at=NULL WHERE id=?")
                .bind(device_id)
                .execute(&pool)
                .await
                .unwrap();
        }
        accounts.push((account_id, device_id));
    }
    accounts.sort_by_key(|(id, _)| *id);
    let (last_account, last_device) = *accounts.last().unwrap();
    let stored = b"stored outside the first page";
    let upload =
        seed_received_upload(&state, last_account, last_device, stored, "page-stored").await;
    upload_commit::complete(&state, upload, last_account)
        .await
        .unwrap();
    let pending = b"reserved outside the first page";
    seed_received_upload(&state, last_account, last_device, pending, "page-pending").await;
    let app = test_router(state).await;
    let login = send(
        &app,
        json_request(
            Method::POST,
            "/api/v1/auth/login",
            json!({"username":ADMIN_USERNAME,"password":ADMIN_PASSWORD}),
            None,
            None,
        ),
        StatusCode::OK,
    )
    .await;
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let csrf = json_body(login).await["csrf_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let get = |uri: String| json_request(Method::GET, uri, json!({}), None, Some((&cookie, &csrf)));
    let first =
        json_body(send(&app, get("/api/v1/admin/overview".into()), StatusCode::OK).await).await;
    assert!(first["previous_cursor"].is_null());
    let mut page = first.clone();
    let mut ids = Vec::new();
    for index in 0..3 {
        let users = page["users"].as_array().unwrap();
        assert_eq!(users.len(), if index == 2 { 25 } else { 50 });
        assert_eq!(page["total_users"], 125);
        assert_eq!(page["online_users"], 63);
        assert_eq!(page["unlimited_users"], 1);
        assert_eq!(page["quota_bytes"], 124 * 100_000_000_i64);
        assert_eq!(page["used_bytes"], stored.len());
        assert_eq!(page["pending_bytes"], pending.len());
        assert!(
            users
                .iter()
                .all(|user| user["instances"].as_array().unwrap().len() == 1)
        );
        ids.extend(
            users
                .iter()
                .map(|user| Uuid::parse_str(user["id"].as_str().unwrap()).unwrap()),
        );
        if index < 2 {
            let next = page["next_cursor"].as_str().unwrap();
            let forward = json_body(
                send(
                    &app,
                    get(format!("/api/v1/admin/overview?cursor={next}")),
                    StatusCode::OK,
                )
                .await,
            )
            .await;
            let back = forward["previous_cursor"].as_str().unwrap();
            let backwards = json_body(
                send(
                    &app,
                    get(format!("/api/v1/admin/overview?cursor={back}")),
                    StatusCode::OK,
                )
                .await,
            )
            .await;
            assert_eq!(backwards["users"], page["users"]);
            page = forward;
        }
    }
    assert_eq!(
        ids,
        accounts.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        "equal names need a stable UUID tiebreaker, with no missing or duplicate accounts"
    );
    assert!(page["next_cursor"].is_null());
    let target = accounts[124].0;
    // A corrupt unrelated authorization must not force a selected detail read
    // through the entire administrator list or expose another instance's code.
    let unrelated = accounts[0].1;
    let original: Vec<u8> =
        sqlx::query_scalar("SELECT authorization_code_enc FROM devices WHERE id=?")
            .bind(unrelated)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE devices SET authorization_code_enc=zeroblob(64) WHERE id=?")
        .bind(unrelated)
        .execute(&pool)
        .await
        .unwrap();
    let detail = json_body(
        send(
            &app,
            get(format!("/api/v1/admin/users/{target}")),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(detail["id"], target.to_string());
    assert_eq!(detail["instances"].as_array().unwrap().len(), 1);
    assert_eq!(detail["used_bytes"], stored.len());
    assert_eq!(detail["pending_bytes"], pending.len());
    sqlx::query("UPDATE devices SET authorization_code_enc=? WHERE id=?")
        .bind(original)
        .bind(unrelated)
        .execute(&pool)
        .await
        .unwrap();
    send(
        &app,
        json_request(
            Method::GET,
            format!("/api/v1/admin/users/{target}"),
            json!({}),
            None,
            None,
        ),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    send(
        &app,
        get(format!("/api/v1/admin/users/{}", Uuid::new_v4())),
        StatusCode::NOT_FOUND,
    )
    .await;
    for cursor in ["", "not-a-cursor", "e30", &"x".repeat(513)] {
        send(
            &app,
            get(format!("/api/v1/admin/overview?cursor={cursor}")),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    send(
        &app,
        get("/api/v1/admin/overview?limit=500".into()),
        StatusCode::BAD_REQUEST,
    )
    .await;
    // Keyset cursors carry the sort value, so deleting their row does not turn
    // continuation into offset drift or require reading the removed record.
    let anchor = accounts[49].0;
    sqlx::query("DELETE FROM accounts WHERE id=?")
        .bind(anchor)
        .execute(&pool)
        .await
        .unwrap();
    let cursor = first["next_cursor"].as_str().unwrap();
    let after_delete = json_body(
        send(
            &app,
            get(format!("/api/v1/admin/overview?cursor={cursor}")),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(after_delete["total_users"], 124);
    assert_eq!(after_delete["users"][0]["id"], accounts[50].0.to_string());
    let created = json_body(
        send(
            &app,
            json_request(
                Method::POST,
                "/api/v1/admin/instances",
                json!({"name":"same"}),
                None,
                Some((&cookie, &csrf)),
            ),
            StatusCode::CREATED,
        )
        .await,
    )
    .await;
    let created_device = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
    let created_account: Uuid = sqlx::query_scalar("SELECT account_id FROM devices WHERE id=?")
        .bind(created_device)
        .fetch_one(&pool)
        .await
        .unwrap();
    let created_detail = json_body(
        send(
            &app,
            get(format!("/api/v1/admin/users/{created_account}")),
            StatusCode::OK,
        )
        .await,
    )
    .await;
    assert_eq!(created_detail["instances"][0]["id"], created["id"]);
    let refreshed =
        json_body(send(&app, get("/api/v1/admin/overview".into()), StatusCode::OK).await).await;
    assert_eq!(refreshed["total_users"], 125);
    assert!(refreshed["previous_cursor"].is_null());
    let max_exact = (1_i64 << 53) - 1;
    sqlx::query("UPDATE accounts SET quota_bytes=? WHERE id IN (?,?)")
        .bind(max_exact)
        .bind(accounts[0].0)
        .bind(last_account)
        .execute(&pool)
        .await
        .unwrap();
    send(
        &app,
        get("/api/v1/admin/overview".into()),
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    let persisted: i64 = sqlx::query_scalar("SELECT quota_bytes FROM accounts WHERE id=?")
        .bind(last_account)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        persisted, max_exact,
        "an unrepresentable aggregate is rejected without rewriting data"
    );
}
