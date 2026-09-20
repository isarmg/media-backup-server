use axum::{
    extract::{Path, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{error::AppError, routes::AppState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateInstanceRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateUserRequest {
    username: String,
    display_name: String,
    storage_path: String,
    quota_bytes: i64,
    enabled: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct AdminUser {
    id: Uuid,
    username: String,
    display_name: String,
    storage_path: String,
    quota_bytes: i64,
    used_bytes: i64,
    pending_bytes: i64,
    device_count: i64,
    resource_count: i64,
    enabled: bool,
    created_at: String,
    last_seen_at: String,
    instances: Vec<AdminInstance>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AdminInstance {
    #[serde(skip)]
    account_id: Uuid,
    id: Uuid,
    name: String,
    platform: String,
    status: String,
    online: bool,
    authorization_code: String,
    created_at: String,
    last_seen_at: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Overview {
    users: Vec<AdminUser>,
    total_users: i64,
    unlimited_users: i64,
    used_bytes: i64,
    pending_bytes: i64,
    quota_bytes: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct AdminLog {
    sequence: i64,
    action: String,
    entity_id: String,
    occurred_at: String,
}

pub(crate) async fn require_admin(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let identity = match sarmg_admin_axum::authenticate_request(
        &state.administrator,
        request.headers(),
        request.uri(),
        request.method(),
        "media-backup",
        state.administrator_origin,
    )
    .await
    {
        Ok(identity) => identity,
        Err(response) => return *response,
    };
    request.extensions_mut().insert(identity);
    next.run(request).await
}

pub(crate) async fn overview(State(state): State<AppState>) -> Result<Json<Overview>, AppError> {
    let users = load_users(&state).await?;
    let total_users = users.len() as i64;
    let unlimited_users = users.iter().filter(|user| user.quota_bytes == 0).count() as i64;
    let used_bytes = users.iter().map(|user| user.used_bytes).sum();
    let pending_bytes = users.iter().map(|user| user.pending_bytes).sum();
    let quota_bytes = users
        .iter()
        .filter(|user| user.quota_bytes > 0)
        .map(|user| user.quota_bytes)
        .sum();
    Ok(Json(Overview {
        users,
        total_users,
        unlimited_users,
        used_bytes,
        pending_bytes,
        quota_bytes,
    }))
}

pub(crate) async fn logs(State(state): State<AppState>) -> Result<Json<Vec<AdminLog>>, AppError> {
    let rows = sqlx::query("SELECT sequence,action,COALESCE(entity_id,'') entity_id,strftime('%Y-%m-%dT%H:%M:%SZ',occurred_at) occurred_at FROM audit_events ORDER BY sequence DESC LIMIT 200")
        .fetch_all(&state.pool).await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| AdminLog {
                sequence: row.get("sequence"),
                action: row.get("action"),
                entity_id: row.get("entity_id"),
                occurred_at: row.get("occurred_at"),
            })
            .collect(),
    ))
}

pub(crate) async fn create_instance(
    State(state): State<AppState>,
    Json(request): Json<CreateInstanceRequest>,
) -> Result<(StatusCode, Json<AdminInstance>), AppError> {
    let account_id = Uuid::new_v4();
    let id = Uuid::new_v4();
    let name = request.name.trim();
    let storage_path = format!("blobs/{account_id}");
    let quota_bytes = 100 * 1024 * 1024 * 1024_i64;
    validate_policy(name, &storage_path, quota_bytes)?;
    ensure_unique_path(&state, &storage_path, None).await?;
    state.storage.validate_account_path(&storage_path).await?;
    let code = random_authorization_code();
    let encrypted = state
        .secrets
        .encrypt_client_authorization(&id.to_string(), &code)?;
    let hash = Sha256::digest(code.as_bytes()).to_vec();
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let username = format!("instance-{}", account_id.simple());
    sqlx::query("INSERT INTO accounts(id,username,display_name,storage_path,quota_bytes,enabled,created_at) VALUES(?,?,?,?,?,1,datetime('now'))")
        .bind(account_id).bind(username).bind(name).bind(&storage_path)
        .bind(quota_bytes).execute(&mut *transaction).await?;
    sqlx::query("INSERT INTO devices(id,account_id,name,platform,token_hash,authorization_code_hash,authorization_code_enc,pairing_status,created_at,last_seen_at) VALUES(?,?,?,'unknown',NULL,?,?,'pending',datetime('now'),NULL)")
        .bind(id).bind(account_id).bind(name).bind(hash).bind(encrypted).execute(&mut *transaction).await?;
    write_instance_audit(&mut transaction, account_id, id, "device.instance.create").await?;
    transaction.commit().await?;
    Ok((StatusCode::CREATED, load_instance(&state, id).await?))
}

pub(crate) async fn update_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<AdminUser>, AppError> {
    let username = request.username.trim();
    let display_name = request.display_name.trim();
    let storage_path = request.storage_path.trim();
    validate_username(username)?;
    validate_policy(display_name, storage_path, request.quota_bytes)?;
    ensure_unique_username(&state, username, Some(id)).await?;
    ensure_unique_path(&state, storage_path, Some(id)).await?;
    state.storage.validate_account_path(storage_path).await?;
    let mut transaction = state.pool.begin().await?;
    let changed = sqlx::query("UPDATE accounts SET username=?,display_name=?,storage_path=?,quota_bytes=?,enabled=? WHERE id=?")
        .bind(username).bind(display_name).bind(storage_path).bind(request.quota_bytes)
        .bind(request.enabled).bind(id).execute(&mut *transaction).await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::not_found("user not found"));
    }
    sqlx::query("UPDATE devices SET name=? WHERE account_id=?")
        .bind(display_name)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(Json(load_user(&state, id).await?))
}

pub(crate) async fn rotate_instance_authorization(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<AdminInstance>, AppError> {
    let account_id: Uuid = sqlx::query_scalar("SELECT account_id FROM devices WHERE id=?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("instance not found"))?;
    let code = random_authorization_code();
    let encrypted = state
        .secrets
        .encrypt_client_authorization(&id.to_string(), &code)?;
    let hash = Sha256::digest(code.as_bytes()).to_vec();
    let mut transaction = state.pool.begin().await?;
    sqlx::query("UPDATE devices SET token_hash=NULL,authorization_code_hash=?,authorization_code_enc=?,pairing_status='pending',last_seen_at=datetime('now') WHERE id=?")
        .bind(hash).bind(encrypted).bind(id).execute(&mut *transaction).await?;
    write_instance_audit(
        &mut transaction,
        account_id,
        id,
        "device.authorization.rotate",
    )
    .await?;
    transaction.commit().await?;
    load_instance(&state, id).await
}

pub(crate) async fn remove_instance(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT account_id,pairing_status FROM devices WHERE id=?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((account_id, status)) = row else {
        return Err(AppError::not_found("instance not found"));
    };
    let mut transaction = state.pool.begin().await?;
    if status == "cancelled" || status == "revoked" {
        let references: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM assets WHERE device_id=?) + (SELECT COUNT(*) FROM uploads WHERE device_id=?) + (SELECT COUNT(*) FROM albums WHERE device_id=?) + (SELECT COUNT(*) FROM api_keys WHERE device_id=?)")
            .bind(id).bind(id).bind(id).bind(id).fetch_one(&mut *transaction).await?;
        if references != 0 {
            return Err(AppError::conflict("instance still owns backup records"));
        }
        write_instance_audit(&mut transaction, account_id, id, "device.instance.delete").await?;
        sqlx::query("DELETE FROM accounts WHERE id=?")
            .bind(account_id)
            .execute(&mut *transaction)
            .await?;
    } else {
        sqlx::query("UPDATE devices SET token_hash=NULL,pairing_status=? WHERE id=?")
            .bind(if status == "pending" {
                "cancelled"
            } else {
                "revoked"
            })
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        write_instance_audit(
            &mut transaction,
            account_id,
            id,
            if status == "pending" {
                "device.pairing.cancel"
            } else {
                "device.instance.revoke"
            },
        )
        .await?;
    }
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn write_instance_audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: Uuid,
    id: Uuid,
    action: &str,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO audit_events(account_id,actor_kind,action,entity_kind,entity_id,occurred_at) VALUES(?,'administrator',?,'device',?,datetime('now'))")
        .bind(account_id).bind(action).bind(id).execute(&mut **transaction).await?;
    Ok(())
}

fn random_authorization_code() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

async fn load_instance(state: &AppState, id: Uuid) -> Result<Json<AdminInstance>, AppError> {
    load_instances(state, Some(id))
        .await?
        .into_iter()
        .next()
        .map(Json)
        .ok_or_else(|| AppError::not_found("instance not found"))
}

async fn load_instances(
    state: &AppState,
    only: Option<Uuid>,
) -> Result<Vec<AdminInstance>, AppError> {
    let rows = sqlx::query("SELECT id,account_id,name,platform,pairing_status,(pairing_status='paired' AND last_seen_at IS NOT NULL AND last_seen_at>=datetime('now','-10 minutes')) online,authorization_code_enc,strftime('%Y-%m-%dT%H:%M:%SZ',created_at) created_at,COALESCE(strftime('%Y-%m-%dT%H:%M:%SZ',last_seen_at),'') last_seen_at FROM devices WHERE (? IS NULL OR id=?) ORDER BY created_at")
        .bind(only).bind(only).fetch_all(&state.pool).await?;
    rows.into_iter()
        .map(|row| {
            let id: Uuid = row.get("id");
            let encrypted: Vec<u8> = row.get("authorization_code_enc");
            Ok(AdminInstance {
                id,
                account_id: row.get("account_id"),
                name: row.get("name"),
                platform: row.get("platform"),
                status: row.get("pairing_status"),
                online: row.get("online"),
                authorization_code: state
                    .secrets
                    .decrypt_client_authorization(&id.to_string(), &encrypted)?,
                created_at: row.get("created_at"),
                last_seen_at: row.get("last_seen_at"),
            })
        })
        .collect()
}

async fn load_users(state: &AppState) -> Result<Vec<AdminUser>, AppError> {
    let rows = sqlx::query(
        "SELECT a.id,a.username,a.display_name,a.storage_path,a.quota_bytes,a.enabled, \
         strftime('%Y-%m-%dT%H:%M:%SZ',a.created_at) AS created_at, \
         COALESCE((SELECT SUM(b.stored_size) FROM blobs b WHERE b.account_id=a.id),0) AS used_bytes, \
         COALESCE((SELECT SUM(p.expected_size) FROM upload_parts p JOIN uploads u ON u.id=p.upload_id WHERE u.account_id=a.id AND u.state='uploading'),0) AS pending_bytes, \
         (SELECT COUNT(*) FROM devices d WHERE d.account_id=a.id) AS device_count, \
         (SELECT COUNT(*) FROM resources r JOIN assets s ON s.id=r.asset_id WHERE s.account_id=a.id) AS resource_count, \
         COALESCE(strftime('%Y-%m-%dT%H:%M:%SZ',(SELECT MAX(d.last_seen_at) FROM devices d WHERE d.account_id=a.id)),'') AS last_seen_at \
         FROM accounts a ORDER BY a.created_at ASC",
    ).fetch_all(&state.pool).await?;
    let instances = load_instances(state, None).await?;
    let mut users = rows.into_iter().map(row_to_user).collect::<Vec<_>>();
    for user in &mut users {
        user.instances = instances
            .iter()
            .filter(|instance| instance.account_id == user.id)
            .cloned()
            .collect();
    }
    Ok(users)
}

async fn load_user(state: &AppState, id: Uuid) -> Result<AdminUser, AppError> {
    load_users(state)
        .await?
        .into_iter()
        .find(|user| user.id == id)
        .ok_or_else(|| AppError::not_found("user not found"))
}

fn row_to_user(row: sqlx::sqlite::SqliteRow) -> AdminUser {
    AdminUser {
        id: row.get("id"),
        username: row.get("username"),
        display_name: row.get("display_name"),
        storage_path: row.get("storage_path"),
        quota_bytes: row.get("quota_bytes"),
        used_bytes: row.get("used_bytes"),
        pending_bytes: row.get("pending_bytes"),
        device_count: row.get("device_count"),
        resource_count: row.get("resource_count"),
        enabled: row.get("enabled"),
        created_at: row.get("created_at"),
        last_seen_at: row.get("last_seen_at"),
        instances: Vec::new(),
    }
}

fn validate_policy(
    display_name: &str,
    storage_path: &str,
    quota_bytes: i64,
) -> Result<(), AppError> {
    if display_name.is_empty()
        || display_name.chars().count() > 32
        || display_name.chars().any(char::is_control)
    {
        return Err(AppError::bad_request(
            "display_name must contain 1 to 32 characters without controls",
        ));
    }
    if storage_path.is_empty()
        || storage_path.len() > 1024
        || storage_path.chars().any(char::is_control)
    {
        return Err(AppError::bad_request("invalid storage_path"));
    }
    if quota_bytes < 0 {
        return Err(AppError::bad_request("quota_bytes cannot be negative"));
    }
    Ok(())
}

fn validate_username(username: &str) -> Result<(), AppError> {
    if username.len() < 3
        || username.len() > 64
        || !username.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
    {
        return Err(AppError::bad_request(
            "username must contain 3 to 64 letters, digits, dots, dashes or underscores",
        ));
    }
    Ok(())
}

async fn ensure_unique_username(
    state: &AppState,
    username: &str,
    except_id: Option<Uuid>,
) -> Result<(), AppError> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM accounts WHERE lower(username)=lower(?1) AND (?2 IS NULL OR id<>?2)",
    )
    .bind(username)
    .bind(except_id)
    .fetch_optional(&state.pool)
    .await?;
    if existing.is_some() {
        return Err(AppError::conflict("username is already in use"));
    }
    Ok(())
}

async fn ensure_unique_path(
    state: &AppState,
    storage_path: &str,
    except_id: Option<Uuid>,
) -> Result<(), AppError> {
    let existing = sqlx::query("SELECT storage_path FROM accounts WHERE ?1 IS NULL OR id<>?1")
        .bind(except_id)
        .fetch_all(&state.pool)
        .await?;
    for row in existing {
        let assigned: String = row.get("storage_path");
        if state
            .storage
            .account_paths_overlap(storage_path, &assigned)?
        {
            return Err(AppError::conflict(
                "storage_path overlaps another user's directory",
            ));
        }
    }
    Ok(())
}

pub(crate) async fn page() -> Response {
    let mut response = Html(crate::web_assets::HTML).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self' data:; font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
        ),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

pub(crate) async fn script(headers: HeaderMap) -> Response {
    static_asset_response(
        crate::web_assets::SCRIPT,
        "text/javascript; charset=utf-8",
        &headers,
    )
}
pub(crate) async fn styles(headers: HeaderMap) -> Response {
    static_asset_response(
        crate::web_assets::STYLES,
        "text/css; charset=utf-8",
        &headers,
    )
}

pub(crate) async fn font_asset(headers: HeaderMap, Path(name): Path<String>) -> Response {
    let path = format!("share/web/assets/{name}");
    let Some((_, contents)) = crate::web_assets::RELEASE_FILES
        .iter()
        .find(|(candidate, _)| *candidate == path)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = if name.ends_with(".woff2") {
        "font/woff2"
    } else if name.ends_with(".txt") {
        "text/plain; charset=utf-8"
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    static_asset_response(contents, content_type, &headers)
}

fn static_asset_response(
    contents: &'static [u8],
    content_type: &'static str,
    request_headers: &HeaderMap,
) -> Response {
    let etag = format!("\"{}\"", blake3::hash(contents).to_hex());
    let not_modified = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag));
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        contents.into_response()
    };
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, no-cache"),
    );
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("BLAKE3 ETag is valid ASCII"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_name_policy_counts_unicode_characters() {
        for character in ["a", "中", "あ", "😀"] {
            assert!(validate_policy(&character.repeat(32), "blobs/test", 0).is_ok());
            assert!(validate_policy(&character.repeat(33), "blobs/test", 0).is_err());
        }
        assert!(validate_policy("bad\nname", "blobs/test", 0).is_err());
    }

    #[tokio::test]
    async fn embedded_fonts_and_license_are_the_verified_release_bytes() {
        for (path, bytes) in crate::web_assets::RELEASE_FILES {
            if !path.ends_with(".woff2") && !path.ends_with(".txt") {
                continue;
            }
            let name = path.strip_prefix("share/web/assets/").unwrap();
            let response = font_asset(HeaderMap::new(), Path(name.to_owned())).await;
            let content_type = if path.ends_with(".woff2") {
                "font/woff2"
            } else {
                "text/plain; charset=utf-8"
            };
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "public, no-cache"
            );
            assert!(response.headers().contains_key(header::ETAG));
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
            let body = axum::body::to_bytes(response.into_body(), 512 * 1024)
                .await
                .unwrap();
            assert_eq!(body.as_ref(), *bytes);
            if path.ends_with(".woff2") {
                assert!(bytes.starts_with(b"wOF2"));
            } else {
                assert!(std::str::from_utf8(bytes)
                    .unwrap()
                    .contains("SIL OPEN FONT LICENSE"));
            }
        }
        for name in [
            "../index.html",
            "missing.woff2",
            "MapleMono.woff2",
            "admin.js",
        ] {
            assert_eq!(
                font_asset(HeaderMap::new(), Path(name.to_owned()))
                    .await
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
    }

    #[tokio::test]
    async fn embedded_assets_revalidate_without_retransmitting_the_body() {
        let initial =
            static_asset_response(crate::web_assets::STYLES, "text/css", &HeaderMap::new());
        let etag = initial.headers()[header::ETAG].clone();
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, etag);
        let cached = static_asset_response(crate::web_assets::STYLES, "text/css", &headers);
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
        assert!(axum::body::to_bytes(cached.into_body(), 1)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn administrator_page_cannot_execute_inline_or_external_scripts() {
        let response = page().await;
        let policy = response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap();
        assert!(policy.contains("script-src 'self'"));
        assert!(policy.contains("frame-ancestors 'none'"));
        assert!(policy.contains("base-uri 'none'"));
        assert!(!policy.contains("unsafe-inline"));
        assert!(!policy.contains("unsafe-eval"));
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
    }
}
