use axum::{
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, Extension, Path, State},
    http::StatusCode,
    middleware,
    routing::{get, post, put},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use media_backup_protocol::{
    BootstrapRequest, BootstrapResponse, CompleteUploadResponse, CreateUploadRequest,
    CreateUploadResponse, EmptyRequest, MediaKind, ResourceManifest, StorageEncoding,
    UploadDisposition, UploadPartSpec, UploadStatusResponse, API_BASE_PATH,
};
use rand::{rngs::OsRng, RngCore};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};
use uuid::Uuid;

use crate::{
    admin, api_access, audit,
    auth::{require_auth, require_secure_transport, AuthContext},
    config::Config,
    error::AppError,
    library,
    login_admission::LoginAdmission,
    metrics,
    storage::{LocalStorage, ObjectState},
    upload_commit,
};

#[derive(Clone)]
pub struct AppState {
    pub secrets: crate::crypto::SecretBox,
    pub pool: SqlitePool,
    pub storage: LocalStorage,
    pub config: Config,
    pub login_admission: LoginAdmission,
    pub upload_admission: UploadAdmission,
    pub administrator:
        Arc<sarmg_admin_core::AdministratorService<sarmg_admin_sqlite::SqliteAdministratorStore>>,
    pub administrator_origin: sarmg_admin_auth::AdministratorOriginMode,
}

#[derive(Clone)]
pub struct UploadAdmission {
    global: std::sync::Arc<Semaphore>,
    per_account_limit: usize,
    accounts: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<Uuid, std::sync::Weak<Semaphore>>>,
    >,
}

struct UploadPermit {
    _global: OwnedSemaphorePermit,
    _account: OwnedSemaphorePermit,
}

impl UploadAdmission {
    pub fn new(global_limit: usize, per_account_limit: usize) -> Self {
        Self {
            global: std::sync::Arc::new(Semaphore::new(global_limit)),
            per_account_limit,
            accounts: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    async fn acquire(&self, account_id: Uuid) -> Result<UploadPermit, AppError> {
        let account = {
            let mut accounts = self
                .accounts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            accounts.retain(|_, semaphore| semaphore.strong_count() > 0);
            if let Some(semaphore) = accounts.get(&account_id).and_then(std::sync::Weak::upgrade) {
                semaphore
            } else {
                let semaphore = std::sync::Arc::new(Semaphore::new(self.per_account_limit));
                accounts.insert(account_id, std::sync::Arc::downgrade(&semaphore));
                semaphore
            }
        };
        let wait = async {
            let global = self.global.clone().acquire_owned().await;
            let account = account.acquire_owned().await;
            match (global, account) {
                (Ok(global), Ok(account)) => Ok(UploadPermit {
                    _global: global,
                    _account: account,
                }),
                _ => Err(AppError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "upload admission is unavailable",
                )),
            }
        };
        timeout(std::time::Duration::from_secs(30), wait)
            .await
            .map_err(|_| AppError::too_many_requests_with_message(5, "upload capacity is busy"))?
    }
}

pub fn router(
    state: AppState,
    runtime: sarmg_server_runtime::RuntimeHandle,
) -> Result<Router, AppError> {
    const JSON_BODY_LIMIT: usize = 256 * 1024;
    const UPLOAD_MANIFEST_BODY_LIMIT: usize = 64 * 1024;
    let max_part_bytes = state.config.max_part_bytes;
    let protected = Router::new()
        .route(
            "/uploads",
            post(create_upload).layer(DefaultBodyLimit::max(UPLOAD_MANIFEST_BODY_LIMIT)),
        )
        .route("/uploads/{id}", get(upload_status))
        .route(
            "/uploads/{id}/parts/{index}",
            put(put_part).layer(DefaultBodyLimit::max(max_part_bytes)),
        )
        .route("/uploads/{id}/complete", post(complete_upload))
        .route("/resources", get(list_resources))
        .route("/resources/{id}", get(resource_manifest))
        .route(
            "/resources/{id}/content",
            get(crate::media_delivery::content),
        )
        .route(
            "/resources/{id}/preview",
            get(crate::media_delivery::preview),
        )
        .route("/sync/head", get(library::sync_head))
        .route("/library/snapshot", get(library::snapshot))
        .route("/devices", get(library::devices))
        .route("/timeline", get(library::timeline))
        .route("/sync", get(library::sync_changes))
        .route(
            "/assets/{id}",
            get(library::get_asset)
                .patch(library::update_asset)
                .delete(library::delete_asset_permanently),
        )
        .route("/assets/{id}/trash", post(library::trash_asset))
        .route("/assets/{id}/restore", post(library::restore_asset))
        .route(
            "/albums",
            get(library::list_albums).post(library::sync_album),
        )
        .route("/tags", get(library::list_tags).post(library::create_tag))
        .route("/tags/{id}/assets", put(library::set_tag_assets))
        .route(
            "/tags/{tag_id}/assets/{asset_id}",
            post(library::add_tag_asset).delete(library::remove_tag_asset),
        )
        .route("/duplicates", get(library::duplicate_groups))
        .route(
            "/api-keys",
            get(api_access::list_api_keys).post(api_access::create_api_key),
        )
        .route(
            "/api-keys/{id}",
            axum::routing::delete(api_access::revoke_api_key),
        )
        .route("/audit-events", get(api_access::audit_events))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    let admin_protected = Router::new()
        .route("/api/v2/admin/overview", get(admin::overview))
        .route("/api/v2/admin/logs", get(admin::logs))
        .route("/api/v2/admin/users/{id}", put(admin::update_user))
        .route("/api/v2/admin/instances", post(admin::create_instance))
        .route(
            "/api/v2/admin/instances/{id}/authorization",
            put(admin::rotate_instance_authorization),
        )
        .route(
            "/api/v2/admin/instances/{id}",
            axum::routing::delete(admin::remove_instance),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            admin::require_admin,
        ));

    let mobile_api = Router::new()
        .route(
            "/auth/bootstrap",
            post(bootstrap).layer(DefaultBodyLimit::max(
                crate::login_admission::LOGIN_BODY_LIMIT_BYTES,
            )),
        )
        .merge(protected);

    let sensitive = Router::new()
        .route("/metrics", get(metrics::prometheus))
        .route("/admin", get(admin::page))
        .route("/admin/", get(admin::page))
        .route("/admin/assets/admin.js", get(admin::script))
        .route("/admin/assets/admin.css", get(admin::styles))
        .route("/admin/assets/{name}", get(admin::font_asset))
        .nest(API_BASE_PATH, mobile_api)
        .merge(admin_protected)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_secure_transport,
        ));

    let platform = sarmg_server_runtime::platform_router(
        runtime,
        "media-backup",
        state.administrator_origin,
        Arc::clone(&state.administrator),
    )
    .map_err(|error| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let product = sensitive
        .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT))
        .layer(middleware::from_fn(crate::error::normalize_error_response))
        .with_state(state);
    Ok(Router::new().merge(platform).merge(product))
}

async fn bootstrap(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(request): Json<BootstrapRequest>,
) -> Result<Json<BootstrapResponse>, AppError> {
    let authorization_code = request.authorization_code.trim();
    if !valid_pairing_authorization_code(authorization_code)
        || request.device_name.trim().is_empty()
        || request.platform.trim().is_empty()
    {
        return Err(AppError::bad_request(
            "authorization_code, device_name and platform are required",
        ));
    }
    let source = crate::trusted_proxy::resolve_client_ip(
        peer.ip(),
        &headers,
        &state.config.trusted_proxy_cidrs,
    )?;
    state.login_admission.check_source(source)?;
    let authorization_hash = Sha256::digest(authorization_code.as_bytes()).to_vec();
    state.login_admission.check_account(&format!(
        "instance:{}",
        URL_SAFE_NO_PAD.encode(&authorization_hash[..8])
    ))?;
    let instance: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id,account_id FROM devices \
         WHERE authorization_code_hash=? AND pairing_status='pending' AND token_hash IS NULL",
    )
    .bind(&authorization_hash)
    .fetch_optional(&state.pool)
    .await?;
    let Some((device_id, account_id)) = instance else {
        return Err(AppError::unauthorized());
    };
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let mut token_bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let bearer_token = URL_SAFE_NO_PAD.encode(token_bytes);
    let token_hash = Sha256::digest(bearer_token.as_bytes()).to_vec();
    let changed = sqlx::query(
        "UPDATE devices SET platform=?,token_hash=?,pairing_status='paired',last_seen_at=datetime('now') \
         WHERE id=? AND authorization_code_hash=? AND pairing_status='pending' AND token_hash IS NULL",
    )
    .bind(request.platform.trim()).bind(token_hash).bind(device_id).bind(authorization_hash)
    .execute(&mut *transaction).await?;
    if changed.rows_affected() != 1 {
        return Err(AppError::conflict("instance was paired concurrently"));
    }
    sqlx::query(
        r#"
        INSERT INTO audit_events(
            account_id, actor_kind, actor_id, action, entity_kind, entity_id, occurred_at
        )
        VALUES (?, 'device', ?, 'device.bootstrap', 'device', ?, datetime('now'))
        "#,
    )
    .bind(account_id)
    .bind(device_id)
    .bind(device_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Json(BootstrapResponse {
        account_id,
        device_id,
        bearer_token,
    }))
}

fn valid_pairing_authorization_code(value: &str) -> bool {
    value.len() == 36
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
}

async fn create_upload(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<CreateUploadRequest>,
) -> Result<Json<CreateUploadResponse>, AppError> {
    validate_upload_request(&request, state.config.max_part_bytes)?;
    let mut transaction = state.pool.begin().await?;
    let policy = account_policy_for_update(&mut transaction, auth.account_id).await?;
    let asset_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO assets(
            id, account_id, device_id, source_asset_id, media_kind, source_created_at_ms, favorite,
            created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
        ON CONFLICT(account_id, device_id, source_asset_id) DO UPDATE SET
            media_kind = excluded.media_kind,
            source_created_at_ms = excluded.source_created_at_ms,
            updated_at = datetime('now'),
            deleted_at = NULL
        RETURNING id
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(auth.account_id)
    .bind(auth.device_id)
    .bind(&request.source_asset_id)
    .bind(request.media_kind.as_str())
    .bind(request.source_created_at_ms)
    .bind(
        request
            .metadata
            .as_ref()
            .and_then(|value| value.get("favorite"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
    .fetch_one(&mut *transaction)
    .await?;

    let existing_blob = sqlx::query(
        "SELECT id, storage_path, stored_size FROM blobs WHERE account_id = ? AND content_blake3 = ?",
    )
    .bind(auth.account_id)
    .bind(&request.content_blake3)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some(existing_blob) = existing_blob {
        let blob_id: Uuid = existing_blob.get("id");
        let storage_path: String = existing_blob.get("storage_path");
        let stored_size: i64 = existing_blob.get("stored_size");
        transaction.commit().await?;
        let size_matches = u64::try_from(stored_size).ok() == Some(request.content_size);
        let file_matches = size_matches
            && state
                .storage
                .inspect_object(
                    &policy.storage_path,
                    &storage_path,
                    request.content_size,
                    &request.content_blake3,
                )
                .await?
                == ObjectState::Matches;
        if !file_matches {
            return Err(AppError::conflict(
                "deduplicated blob is missing or does not match its full hash",
            ));
        }
        let mut transaction = state.pool.begin().await?;
        let resource_id = upsert_resource(&mut transaction, asset_id, blob_id, &request).await?;
        audit::record_change(
            &mut transaction,
            auth.account_id,
            "asset",
            asset_id,
            "upsert",
        )
        .await?;
        audit::record_in_transaction(
            &mut transaction,
            &auth,
            "resource.deduplicate",
            Some("resource"),
            Some(resource_id),
        )
        .await?;
        transaction.commit().await?;
        return Ok(Json(CreateUploadResponse {
            disposition: UploadDisposition::Complete,
            upload_id: None,
            resource_id: Some(resource_id),
            missing_parts: Vec::new(),
        }));
    }

    let existing_upload: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT id FROM uploads
        WHERE account_id = ? AND device_id = ? AND source_resource_id = ?
          AND content_blake3 = ? AND state = 'uploading'
          AND commit_state IN ('receiving', 'commit_started', 'finalizing')
        ORDER BY created_at DESC LIMIT 1
        "#,
    )
    .bind(auth.account_id)
    .bind(auth.device_id)
    .bind(&request.source_resource_id)
    .bind(&request.content_blake3)
    .fetch_optional(&mut *transaction)
    .await?;
    if let Some(upload_id) = existing_upload {
        transaction.commit().await?;
        let missing_parts = missing_parts(&state.pool, upload_id).await?;
        return Ok(Json(CreateUploadResponse {
            disposition: UploadDisposition::Upload,
            upload_id: Some(upload_id),
            resource_id: None,
            missing_parts,
        }));
    }

    ensure_quota(
        &mut transaction,
        auth.account_id,
        policy.quota_bytes,
        request_storage_size(&request)?,
    )
    .await?;

    let request_json = serde_json::to_value(&request)?;
    let upload_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO uploads(
            id, account_id, device_id, asset_id, source_resource_id, content_blake3, request,
            created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
        RETURNING id
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(auth.account_id)
    .bind(auth.device_id)
    .bind(asset_id)
    .bind(&request.source_resource_id)
    .bind(&request.content_blake3)
    .bind(request_json)
    .fetch_one(&mut *transaction)
    .await?;
    for part in &request.parts {
        sqlx::query(
            "INSERT INTO upload_parts(upload_id, part_index, expected_size, expected_blake3) VALUES (?, ?, ?, ?)",
        )
        .bind(upload_id)
        .bind(part.index as i32)
        .bind(part.size as i64)
        .bind(&part.blake3)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Json(CreateUploadResponse {
        disposition: UploadDisposition::Upload,
        upload_id: Some(upload_id),
        resource_id: None,
        missing_parts: request.parts.iter().map(|part| part.index).collect(),
    }))
}

async fn put_part(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((upload_id, index)): Path<(Uuid, u32)>,
    body: Body,
) -> Result<StatusCode, AppError> {
    let row = sqlx::query(
        r#"
        SELECT p.expected_size, p.expected_blake3, u.state, u.commit_state
        FROM upload_parts p JOIN uploads u ON u.id = p.upload_id
        WHERE p.upload_id = ? AND p.part_index = ? AND u.account_id = ?
        "#,
    )
    .bind(upload_id)
    .bind(index as i32)
    .bind(auth.account_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::not_found("upload part not found"))?;
    if row.get::<String, _>("commit_state") == "committed" {
        return Ok(StatusCode::NO_CONTENT);
    }
    if row.get::<String, _>("commit_state") != "receiving" {
        return Err(AppError::conflict(
            "upload parts are frozen after commit starts",
        ));
    }
    let spec = UploadPartSpec {
        index,
        size: row.get::<i64, _>("expected_size") as u64,
        blake3: row.get("expected_blake3"),
    };
    let _admission = state.upload_admission.acquire(auth.account_id).await?;
    state
        .storage
        .put_part(upload_id, &spec, body, state.config.max_part_bytes)
        .await?;
    sqlx::query(
        "UPDATE upload_parts SET received_size = expected_size, received_at = datetime('now') WHERE upload_id = ? AND part_index = ?",
    )
    .bind(upload_id)
    .bind(index as i32)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn upload_status(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(upload_id): Path<Uuid>,
) -> Result<Json<UploadStatusResponse>, AppError> {
    let state_value: String =
        sqlx::query_scalar("SELECT commit_state FROM uploads WHERE id = ? AND account_id = ?")
            .bind(upload_id)
            .bind(auth.account_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::not_found("upload not found"))?;
    Ok(Json(UploadStatusResponse {
        upload_id,
        state: state_value,
        missing_parts: missing_parts(&state.pool, upload_id).await?,
    }))
}

async fn complete_upload(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(upload_id): Path<Uuid>,
    Json(_request): Json<EmptyRequest>,
) -> Result<Json<CompleteUploadResponse>, AppError> {
    let outcome = upload_commit::complete(&state, upload_id, auth.account_id).await?;
    Ok(Json(CompleteUploadResponse {
        resource_id: outcome.resource_id,
        asset_id: outcome.asset_id,
        deduplicated: outcome.deduplicated,
    }))
}

async fn list_resources(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<Vec<ResourceManifest>>, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT r.id AS resource_id, r.asset_id, r.source_resource_id, r.role, r.filename,
               r.mime_type, r.metadata,
               a.source_asset_id, a.media_kind, a.source_created_at_ms,
               b.plaintext_size, b.content_blake3, b.part_manifest
        FROM resources r
        JOIN assets a ON a.id = r.asset_id
        JOIN blobs b ON b.id = r.blob_id
        WHERE a.account_id = ? AND a.deleted_at IS NULL
        ORDER BY a.source_created_at_ms DESC, r.created_at DESC
        LIMIT 1000
        "#,
    )
    .bind(auth.account_id)
    .fetch_all(&state.pool)
    .await?;
    let resources = rows
        .iter()
        .map(resource_manifest_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(resources))
}

async fn resource_manifest(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(resource_id): Path<Uuid>,
) -> Result<Json<ResourceManifest>, AppError> {
    Ok(Json(
        load_manifest(&state.pool, auth.account_id, resource_id).await?,
    ))
}

fn validate_upload_request(
    request: &CreateUploadRequest,
    max_part_bytes: usize,
) -> Result<(), AppError> {
    if request.source_asset_id.is_empty()
        || request.source_resource_id.is_empty()
        || request.filename.is_empty()
        || request.mime_type.is_empty()
        || !is_blake3(&request.content_blake3)
        || request.parts.is_empty()
        || request.filename.len() > 1024
        || request.mime_type.len() > 255
        || request.metadata.as_ref().is_some_and(|value| {
            serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() > 64 * 1024)
        })
    {
        return Err(AppError::bad_request("upload manifest is incomplete"));
    }
    for (position, part) in request.parts.iter().enumerate() {
        if part.index as usize != position
            || (part.size == 0 && request.content_size != 0)
            || part.size > max_part_bytes as u64
            || !is_blake3(&part.blake3)
        {
            return Err(AppError::bad_request("invalid part manifest"));
        }
    }
    let total = request
        .parts
        .iter()
        .try_fold(0_u64, |sum, part| sum.checked_add(part.size))
        .ok_or_else(|| AppError::bad_request("upload size overflow"))?;
    if total != request.content_size {
        return Err(AppError::bad_request(
            "part sizes do not match content_size",
        ));
    }
    Ok(())
}

fn is_blake3(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn missing_parts(pool: &SqlitePool, upload_id: Uuid) -> Result<Vec<u32>, AppError> {
    let values: Vec<i32> = sqlx::query_scalar(
        "SELECT part_index FROM upload_parts WHERE upload_id = ? AND received_at IS NULL ORDER BY part_index",
    )
    .bind(upload_id)
    .fetch_all(pool)
    .await?;
    Ok(values.into_iter().map(|value| value as u32).collect())
}

async fn upsert_resource(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    asset_id: Uuid,
    blob_id: Uuid,
    request: &CreateUploadRequest,
) -> Result<Uuid, AppError> {
    Ok(sqlx::query_scalar(
        r#"
        INSERT INTO resources(
            id, asset_id, blob_id, source_resource_id, role, filename, mime_type,
            metadata, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
        ON CONFLICT(asset_id, source_resource_id) DO UPDATE SET
            blob_id = excluded.blob_id,
            role = excluded.role,
            filename = excluded.filename,
            mime_type = excluded.mime_type,
            metadata = excluded.metadata
        RETURNING id
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(asset_id)
    .bind(blob_id)
    .bind(&request.source_resource_id)
    .bind(&request.role)
    .bind(&request.filename)
    .bind(&request.mime_type)
    .bind(&request.metadata)
    .fetch_one(&mut **transaction)
    .await?)
}

async fn load_manifest(
    pool: &SqlitePool,
    account_id: Uuid,
    resource_id: Uuid,
) -> Result<ResourceManifest, AppError> {
    let row = sqlx::query(
        r#"
        SELECT r.id AS resource_id, r.asset_id, r.source_resource_id, r.role, r.filename,
               r.mime_type, r.metadata,
               a.source_asset_id, a.media_kind, a.source_created_at_ms,
               b.plaintext_size, b.content_blake3, b.part_manifest
        FROM resources r
        JOIN assets a ON a.id = r.asset_id
        JOIN blobs b ON b.id = r.blob_id
        WHERE r.id = ? AND a.account_id = ?
        "#,
    )
    .bind(resource_id)
    .bind(account_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::not_found("resource not found"))?;
    resource_manifest_from_row(&row)
}

fn resource_manifest_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ResourceManifest, AppError> {
    let resource_id: Uuid = row.get("resource_id");
    let media_kind = match row.get::<String, _>("media_kind").as_str() {
        "photo" => MediaKind::Photo,
        "video" => MediaKind::Video,
        _ => MediaKind::Other,
    };
    Ok(ResourceManifest {
        resource_id: row.get("resource_id"),
        asset_id: row.get("asset_id"),
        source_asset_id: row.get("source_asset_id"),
        source_resource_id: row.get("source_resource_id"),
        media_kind,
        role: row.get("role"),
        filename: row.get("filename"),
        mime_type: row.get("mime_type"),
        source_created_at_ms: row.get("source_created_at_ms"),
        content_size: row.get::<i64, _>("plaintext_size") as u64,
        content_blake3: row.get("content_blake3"),
        storage_encoding: StorageEncoding::PlainV1,
        metadata: row.get("metadata"),
        parts: serde_json::from_value(row.get::<Value, _>("part_manifest"))?,
        content_path: format!("{API_BASE_PATH}/resources/{resource_id}/content"),
    })
}

struct AccountPolicy {
    storage_path: String,
    quota_bytes: i64,
}

async fn account_policy_for_update(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: Uuid,
) -> Result<AccountPolicy, AppError> {
    let row = sqlx::query("SELECT storage_path, quota_bytes FROM accounts WHERE id = ?")
        .bind(account_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or_else(AppError::unauthorized)?;
    Ok(AccountPolicy {
        storage_path: row.get("storage_path"),
        quota_bytes: row.get("quota_bytes"),
    })
}

async fn ensure_quota(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: Uuid,
    quota_bytes: i64,
    requested_bytes: i64,
) -> Result<(), AppError> {
    if quota_bytes == 0 {
        return Ok(());
    }
    let used_bytes: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(stored_size), 0) FROM blobs WHERE account_id = ?")
            .bind(account_id)
            .fetch_one(&mut **transaction)
            .await?;
    let reserved_bytes: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(p.expected_size), 0)
        FROM upload_parts p
        JOIN uploads u ON u.id = p.upload_id
        WHERE u.account_id = ? AND u.state = 'uploading'
          AND u.commit_state IN ('receiving', 'commit_started', 'finalizing', 'unknown')
        "#,
    )
    .bind(account_id)
    .fetch_one(&mut **transaction)
    .await?;
    let projected = used_bytes
        .checked_add(reserved_bytes)
        .and_then(|value| value.checked_add(requested_bytes))
        .unwrap_or(i64::MAX);
    if projected > quota_bytes {
        return Err(AppError::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "account storage quota exceeded",
        ));
    }
    Ok(())
}

fn request_storage_size(request: &CreateUploadRequest) -> Result<i64, AppError> {
    let total = request
        .parts
        .iter()
        .try_fold(0_u64, |sum, part| sum.checked_add(part.size));
    let total = total.ok_or_else(|| AppError::bad_request("upload size overflow"))?;
    i64::try_from(total).map_err(|_| AppError::bad_request("upload is too large"))
}
