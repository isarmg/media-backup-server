use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Response},
    Json,
};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{error::AppError, routes::AppState};

// Administrator JSON is consumed as exact JavaScript integers.
const MAX_JSON_INTEGER: i64 = (1_i64 << 53) - 1;

fn sum_overview_bytes(mut values: impl Iterator<Item = i64>) -> Result<i64, AppError> {
    values.try_fold(0_i64, |total, value| {
        total
            .checked_add(value)
            .filter(|sum| value >= 0 && *sum <= MAX_JSON_INTEGER)
            .ok_or_else(|| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "administrator byte total is outside the JSON integer range",
                )
            })
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateInstanceRequest {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateUserRequest {
    username: String,
    display_name: String,
    storage_path: String,
    quota_bytes: i64,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogsQuery {
    date: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AdminLogs {
    date: String,
    logs: Vec<AdminLog>,
}

fn valid_calendar_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return false;
    }
    let year = date[..4].parse::<u32>().unwrap_or(0);
    let month = date[5..7].parse::<u32>().unwrap_or(0);
    let day = date[8..10].parse::<u32>().unwrap_or(0);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    year > 0 && (1..=days).contains(&day)
}

fn server_local_time(local: String, utc_offset_seconds: i64) -> Result<String, AppError> {
    if utc_offset_seconds.unsigned_abs() > 24 * 3600 {
        return Err(AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server time offset is invalid",
        ));
    }
    let sign = if utc_offset_seconds < 0 { '-' } else { '+' };
    let minutes = utc_offset_seconds.unsigned_abs() / 60;
    let seconds = utc_offset_seconds.unsigned_abs() % 60;
    if seconds == 0 {
        Ok(format!(
            "{local} {sign}{:02}:{:02}",
            minutes / 60,
            minutes % 60
        ))
    } else {
        Ok(format!(
            "{local} {sign}{:02}:{:02}:{seconds:02}",
            minutes / 60,
            minutes % 60
        ))
    }
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
    let used_bytes = sum_overview_bytes(users.iter().map(|user| user.used_bytes))?;
    let pending_bytes = sum_overview_bytes(users.iter().map(|user| user.pending_bytes))?;
    let quota_bytes = sum_overview_bytes(
        users
            .iter()
            .filter(|user| user.quota_bytes > 0)
            .map(|user| user.quota_bytes),
    )?;
    Ok(Json(Overview {
        users,
        total_users,
        unlimited_users,
        used_bytes,
        pending_bytes,
        quota_bytes,
    }))
}

pub(crate) async fn logs(
    State(state): State<AppState>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<AdminLogs>, AppError> {
    let date = match query.date {
        Some(date) if valid_calendar_date(&date) => date,
        Some(_) => return Err(AppError::bad_request("date must be a valid YYYY-MM-DD")),
        None => {
            sqlx::query_scalar("SELECT date('now', 'localtime')")
                .fetch_one(&state.pool)
                .await?
        }
    };
    // SQLite audit timestamps are UTC. Convert each server-local midnight
    // separately so a daylight-saving transition gives a 23- or 25-hour day.
    let (start_utc, end_utc): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT datetime(?1 || ' 00:00:00', 'utc'), \
                datetime(date(?1, '+1 day') || ' 00:00:00', 'utc')",
    )
    .bind(&date)
    .fetch_one(&state.pool)
    .await?;
    let (Some(start_utc), Some(end_utc)) = (start_utc, end_utc) else {
        return Err(AppError::bad_request("date is outside the supported range"));
    };
    let rows = sqlx::query(
        r#"
        SELECT sequence, action,
               CASE
                 WHEN entity_id IS NULL THEN ''
                 WHEN typeof(entity_id) = 'blob' AND length(entity_id) = 16 THEN
                   lower(substr(hex(entity_id), 1, 8) || '-' ||
                         substr(hex(entity_id), 9, 4) || '-' ||
                         substr(hex(entity_id), 13, 4) || '-' ||
                         substr(hex(entity_id), 17, 4) || '-' ||
                         substr(hex(entity_id), 21, 12))
                 ELSE CAST(entity_id AS TEXT)
               END AS entity_id,
               strftime('%Y-%m-%d %H:%M:%S', occurred_at, 'localtime') AS occurred_at,
               CAST(strftime('%s', occurred_at, 'localtime') AS INTEGER) -
               CAST(strftime('%s', occurred_at) AS INTEGER) AS utc_offset_seconds
        FROM audit_events
        WHERE occurred_at >= ? AND occurred_at < ?
        ORDER BY sequence DESC
        "#,
    )
    .bind(start_utc)
    .bind(end_utc)
    .fetch_all(&state.pool)
    .await?;
    let logs = rows
        .into_iter()
        .map(|row| {
            Ok(AdminLog {
                sequence: row.try_get("sequence")?,
                action: row.try_get("action")?,
                entity_id: row.try_get("entity_id")?,
                occurred_at: server_local_time(
                    row.try_get("occurred_at")?,
                    row.try_get("utc_offset_seconds")?,
                )?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(Json(AdminLogs { date, logs }))
}

pub(crate) async fn create_instance(
    State(state): State<AppState>,
    Json(request): Json<CreateInstanceRequest>,
) -> Result<(StatusCode, Json<AdminInstance>), AppError> {
    let account_id = Uuid::new_v4();
    let id = Uuid::new_v4();
    let name = request.name.as_deref().unwrap_or("新实例").trim();
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
    sqlx::query("INSERT INTO accounts(id,username,display_name,storage_path,quota_bytes,created_at) VALUES(?,?,?,?,?,datetime('now'))")
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
    let changed = sqlx::query(
        "UPDATE accounts SET username=?,display_name=?,storage_path=?,quota_bytes=? WHERE id=?",
    )
    .bind(username)
    .bind(display_name)
    .bind(storage_path)
    .bind(request.quota_bytes)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
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
    const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut value = String::with_capacity(36);
    let mut bytes = [0_u8; 64];
    while value.len() < 36 {
        OsRng.fill_bytes(&mut bytes);
        for byte in bytes {
            if byte < 252 {
                value.push(ALPHABET[usize::from(byte % 36)] as char);
                if value.len() == 36 {
                    break;
                }
            }
        }
    }
    value
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
    let rows = sqlx::query("SELECT id,account_id,name,platform,pairing_status,(pairing_status='paired' AND last_seen_at IS NOT NULL AND last_seen_at>=datetime('now','-10 minutes')) online,authorization_code_enc,strftime('%Y-%m-%dT%H:%M:%SZ',created_at) created_at,COALESCE(strftime('%Y-%m-%dT%H:%M:%SZ',last_seen_at),'') last_seen_at FROM devices WHERE (? IS NULL OR id=?) ORDER BY name COLLATE NOCASE,name,id")
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
        "SELECT a.id,a.username,a.display_name,a.storage_path,a.quota_bytes, \
         strftime('%Y-%m-%dT%H:%M:%SZ',a.created_at) AS created_at, \
         COALESCE((SELECT SUM(b.stored_size) FROM blobs b WHERE b.account_id=a.id),0) AS used_bytes, \
         COALESCE((SELECT SUM(p.expected_size) FROM upload_parts p JOIN uploads u ON u.id=p.upload_id WHERE u.account_id=a.id AND u.state='uploading'),0) AS pending_bytes, \
         (SELECT COUNT(*) FROM devices d WHERE d.account_id=a.id) AS device_count, \
         (SELECT COUNT(*) FROM resources r JOIN assets s ON s.id=r.asset_id WHERE s.account_id=a.id) AS resource_count, \
         COALESCE(strftime('%Y-%m-%dT%H:%M:%SZ',(SELECT MAX(d.last_seen_at) FROM devices d WHERE d.account_id=a.id)),'') AS last_seen_at \
         FROM accounts a ORDER BY a.display_name COLLATE NOCASE,a.display_name,a.id",
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
    if !(0..=MAX_JSON_INTEGER).contains(&quota_bytes) {
        return Err(AppError::bad_request(
            "quota_bytes must be a non-negative safe JSON integer",
        ));
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
    fn log_dates_are_strict_gregorian_calendar_dates() {
        for date in ["2024-02-29", "2026-09-23", "2000-02-29"] {
            assert!(valid_calendar_date(date), "{date}");
        }
        for date in [
            "",
            "2025-02-29",
            "1900-02-29",
            "2026-04-31",
            "2026-00-01",
            "2026-13-01",
            "0000-01-01",
            "2026-01-00",
            "2026-1-01",
            "2026-01-01Z",
            "２０２６-01-01",
        ] {
            assert!(!valid_calendar_date(date), "{date}");
        }
    }

    #[test]
    fn log_times_include_the_server_offset_for_repeated_clock_hours() {
        assert_eq!(
            server_local_time("2026-11-01 01:30:00".into(), -4 * 3600).unwrap(),
            "2026-11-01 01:30:00 -04:00"
        );
        assert_eq!(
            server_local_time("2026-11-01 01:30:00".into(), -5 * 3600).unwrap(),
            "2026-11-01 01:30:00 -05:00"
        );
        assert_eq!(
            server_local_time("2026-09-23 14:30:00".into(), 5 * 3600 + 30 * 60).unwrap(),
            "2026-09-23 14:30:00 +05:30"
        );
    }

    #[test]
    fn local_midnight_bounds_follow_daylight_saving_transitions() {
        const CHILD: &str = "MEDIA_BACKUP_LOG_DST_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .env("TZ", "America/New_York")
                .env(CHILD, "1")
                .arg("--exact")
                .arg("admin::tests::local_midnight_bounds_follow_daylight_saving_transitions")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let database = rusqlite::Connection::open_in_memory().unwrap();
        let bounds = |day: &str| {
            database
                .query_row(
                    "SELECT datetime(?1 || ' 00:00:00', 'utc'), \
                            datetime(date(?1, '+1 day') || ' 00:00:00', 'utc')",
                    [day],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .unwrap()
        };
        assert_eq!(
            bounds("2026-03-08"),
            ("2026-03-08 05:00:00".into(), "2026-03-09 04:00:00".into())
        );
        assert_eq!(
            bounds("2026-11-01"),
            ("2026-11-01 04:00:00".into(), "2026-11-02 05:00:00".into())
        );
    }

    #[test]
    fn generated_authorization_codes_have_the_shared_format() {
        for _ in 0..64 {
            let value = random_authorization_code();
            assert_eq!(value.len(), 36);
            assert!(value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase()));
        }
    }

    #[test]
    fn instance_name_policy_counts_unicode_characters() {
        for character in ["a", "中", "あ", "😀"] {
            assert!(validate_policy(&character.repeat(32), "blobs/test", 0).is_ok());
            assert!(validate_policy(&character.repeat(33), "blobs/test", 0).is_err());
        }
        assert!(validate_policy("bad\nname", "blobs/test", 0).is_err());
    }

    #[test]
    fn quota_and_overview_totals_keep_exact_json_integer_boundaries() {
        for quota in [0, 1, MAX_JSON_INTEGER] {
            assert!(validate_policy("instance", "blobs/test", quota).is_ok());
        }
        for quota in [-1, MAX_JSON_INTEGER + 1, i64::MAX] {
            assert!(validate_policy("instance", "blobs/test", quota).is_err());
        }
        assert_eq!(sum_overview_bytes([0, 100, 23].into_iter()).unwrap(), 123);
        assert_eq!(
            sum_overview_bytes([MAX_JSON_INTEGER, 0].into_iter()).unwrap(),
            MAX_JSON_INTEGER
        );
        for values in [[MAX_JSON_INTEGER, 1], [i64::MAX, i64::MAX], [-1, 2]] {
            assert!(sum_overview_bytes(values.into_iter()).is_err());
        }
    }

    #[test]
    fn create_request_uses_the_server_default_when_name_is_omitted() {
        let request: CreateInstanceRequest = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(request.name.is_none());
        assert!(serde_json::from_value::<CreateInstanceRequest>(
            serde_json::json!({"other": true})
        )
        .is_err());
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
