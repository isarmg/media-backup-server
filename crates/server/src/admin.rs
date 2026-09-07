use axum::{
    extract::{Path, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::{error::AppError, password, routes::AppState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateUserRequest {
    username: String,
    password: String,
    display_name: String,
    storage_path: String,
    quota_bytes: i64,
    enabled: bool,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResetPasswordRequest {
    password: String,
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
}

#[derive(Debug, Serialize)]
pub(crate) struct Overview {
    users: Vec<AdminUser>,
    total_users: i64,
    active_users: i64,
    unlimited_users: i64,
    used_bytes: i64,
    pending_bytes: i64,
    quota_bytes: i64,
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
    let active_users = users.iter().filter(|user| user.enabled).count() as i64;
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
        active_users,
        unlimited_users,
        used_bytes,
        pending_bytes,
        quota_bytes,
    }))
}

pub(crate) async fn create_user(
    State(state): State<AppState>,
    Json(request): Json<CreateUserRequest>,
) -> Result<Json<AdminUser>, AppError> {
    let id = Uuid::new_v4();
    let username = request.username.trim();
    let display_name = request.display_name.trim();
    let storage_path = if request.storage_path.trim().is_empty() {
        format!("blobs/{id}")
    } else {
        request.storage_path.trim().to_owned()
    };
    validate_username(username)?;
    password::require_current_policy(&request.password)?;
    validate_policy(display_name, &storage_path, request.quota_bytes)?;
    ensure_unique_username(&state, username, None).await?;
    ensure_unique_path(&state, &storage_path, None).await?;
    state.storage.validate_account_path(&storage_path).await?;
    let password_hash = password::hash_current_password(request.password).await?;
    sqlx::query("INSERT INTO accounts(id,username,password_hash,display_name,storage_path,quota_bytes,enabled,created_at) VALUES(?,?,?,?,?,?,?,datetime('now'))")
        .bind(id).bind(username).bind(password_hash).bind(display_name).bind(&storage_path)
        .bind(request.quota_bytes).bind(request.enabled).execute(&state.pool).await?;
    Ok(Json(load_user(&state, id).await?))
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
    let changed = sqlx::query("UPDATE accounts SET username=?,display_name=?,storage_path=?,quota_bytes=?,enabled=? WHERE id=?")
        .bind(username).bind(display_name).bind(storage_path).bind(request.quota_bytes)
        .bind(request.enabled).bind(id).execute(&state.pool).await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::not_found("user not found"));
    }
    Ok(Json(load_user(&state, id).await?))
}

pub(crate) async fn reset_user_password(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<ResetPasswordRequest>,
) -> Result<StatusCode, AppError> {
    password::require_current_policy(&request.password)?;
    let hash = password::hash_current_password(request.password).await?;
    let changed = sqlx::query("UPDATE accounts SET password_hash=? WHERE id=?")
        .bind(hash)
        .bind(id)
        .execute(&state.pool)
        .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::not_found("user not found"));
    }
    Ok(StatusCode::NO_CONTENT)
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
    Ok(rows.into_iter().map(row_to_user).collect())
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

pub(crate) async fn script() -> Response {
    static_asset_response(crate::web_assets::SCRIPT, "text/javascript; charset=utf-8")
}
pub(crate) async fn styles() -> Response {
    static_asset_response(crate::web_assets::STYLES, "text/css; charset=utf-8")
}

pub(crate) async fn font_asset(Path(name): Path<String>) -> Response {
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
    static_asset_response(contents, content_type)
}

fn static_asset_response(contents: &'static [u8], content_type: &'static str) -> Response {
    let mut response = contents.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
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
            let response = font_asset(Path(name.to_owned())).await;
            let content_type = if path.ends_with(".woff2") {
                "font/woff2"
            } else {
                "text/plain; charset=utf-8"
            };
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
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
                font_asset(Path(name.to_owned())).await.status(),
                StatusCode::NOT_FOUND
            );
        }
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
