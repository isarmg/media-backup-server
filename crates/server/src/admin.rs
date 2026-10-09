use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::Response,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{TryRng, rngs::SysRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;
use xcss_server_cli::{ContractJson, ContractPath, ContractQuery};

use crate::{error::AppError, routes::AppState};

// Administrator JSON is consumed as exact JavaScript integers.
const MAX_JSON_INTEGER: i64 = (1_i64 << 53) - 1;

fn require_json_integer(value: i64) -> Result<i64, AppError> {
    if !(0..=MAX_JSON_INTEGER).contains(&value) {
        return Err(AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "administrator total is outside the JSON integer range",
        ));
    }
    Ok(value)
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
    previous_cursor: Option<String>,
    next_cursor: Option<String>,
    total_users: i64,
    online_users: i64,
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
    start_date: Option<String>,
    end_date: Option<String>,
    cursor: Option<String>,
    instance_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AdminLogs {
    date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    end_date: Option<String>,
    instance_id: Option<Uuid>,
    logs: Vec<AdminLog>,
    previous_cursor: Option<String>,
    next_cursor: Option<String>,
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
    let identity = match xcss_admin_axum::authenticate_request(
        &state.administrator,
        request.headers(),
        request.uri(),
        request.method(),
        "xszs",
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OverviewQuery {
    cursor: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceCursor {
    name: String,
    id: Uuid,
    before: bool,
}

impl InstanceCursor {
    fn encode(&self) -> Result<String, AppError> {
        Ok(
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).map_err(|_| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "cannot encode instance cursor",
                )
            })?),
        )
    }

    fn decode(value: &str) -> Result<Self, AppError> {
        let invalid = || AppError::bad_request("invalid instance cursor");
        if value.len() > 512 || value.is_empty() {
            return Err(invalid());
        }
        let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
        let cursor: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if cursor.id.is_nil()
            || cursor.name.trim() != cursor.name
            || !(1..=32).contains(&cursor.name.chars().count())
            || cursor.name.chars().any(char::is_control)
            || cursor.encode()? != value
        {
            return Err(invalid());
        }
        Ok(cursor)
    }
}

pub(crate) async fn overview(
    State(state): State<AppState>,
    ContractQuery(query): ContractQuery<OverviewQuery>,
) -> Result<Json<Overview>, AppError> {
    let cursor = query
        .cursor
        .as_deref()
        .map(InstanceCursor::decode)
        .transpose()?;
    let (predicate, order) = match cursor.as_ref() {
        None => ("", "ASC"),
        Some(cursor) if cursor.before => (
            "WHERE (a.display_name COLLATE NOCASE,a.display_name,a.id) < (?1 COLLATE NOCASE,?1,?2)",
            "DESC",
        ),
        Some(_) => (
            "WHERE (a.display_name COLLATE NOCASE,a.display_name,a.id) > (?1 COLLATE NOCASE,?1,?2)",
            "ASC",
        ),
    };
    // Names are not unique. Preserve SQLite NOCASE/name/UUID order in every
    // direction, including when the anchor is removed between requests.
    let sql = format!(
        "{USER_SELECT} {predicate} ORDER BY a.display_name COLLATE NOCASE {order},a.display_name {order},a.id {order} LIMIT 50"
    );
    let mut statement = sqlx::query(sqlx::AssertSqlSafe(sql));
    if let Some(cursor) = cursor.as_ref() {
        statement = statement.bind(&cursor.name).bind(cursor.id);
    }
    let mut users = statement
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|row| row_to_user(&state, row))
        .collect::<Result<Vec<_>, _>>()?;
    if cursor.as_ref().is_some_and(|cursor| cursor.before) {
        users.reverse();
    }
    let mut previous_cursor = None;
    let mut next_cursor = None;
    if let (Some(first), Some(last)) = (users.first(), users.last()) {
        for (comparison, before, user, destination) in [
            ("<", true, first, &mut previous_cursor),
            (">", false, last, &mut next_cursor),
        ] {
            let sql = format!(
                "SELECT EXISTS(SELECT 1 FROM accounts a WHERE (a.display_name COLLATE NOCASE,a.display_name,a.id) {comparison} (?1 COLLATE NOCASE,?1,?2))"
            );
            let exists: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .bind(&user.display_name)
                .bind(user.id)
                .fetch_one(&state.pool)
                .await?;
            if exists {
                *destination = Some(
                    InstanceCursor {
                        name: user.display_name.clone(),
                        id: user.id,
                        before,
                    }
                    .encode()?,
                );
            }
        }
    }
    // One scalar aggregate statement supplies global statistics independently
    // of the selected page, without collecting accounts/uploads/blobs in memory.
    let totals = sqlx::query("SELECT \
        (SELECT COUNT(*) FROM accounts) total_users, \
        (SELECT COUNT(*) FROM accounts WHERE quota_bytes=0) unlimited_users, \
        (SELECT COUNT(*) FROM devices WHERE pairing_status='paired' AND last_seen_at IS NOT NULL AND last_seen_at>=datetime('now','-10 minutes')) online_users, \
        (SELECT COALESCE(SUM(stored_size),0) FROM blobs) used_bytes, \
        (SELECT COALESCE(SUM(p.expected_size),0) FROM upload_parts p JOIN uploads u ON u.id=p.upload_id WHERE u.state='uploading' AND u.commit_state IN ('receiving','commit_started','finalizing','unknown')) pending_bytes, \
        (SELECT COALESCE(SUM(quota_bytes),0) FROM accounts WHERE quota_bytes>0) quota_bytes")
        .fetch_one(&state.pool).await?;
    let exact = |field| -> Result<i64, AppError> { require_json_integer(totals.try_get(field)?) };
    Ok(Json(Overview {
        users,
        previous_cursor,
        next_cursor,
        total_users: exact("total_users")?,
        online_users: exact("online_users")?,
        unlimited_users: exact("unlimited_users")?,
        used_bytes: exact("used_bytes")?,
        pending_bytes: exact("pending_bytes")?,
        quota_bytes: exact("quota_bytes")?,
    }))
}

pub(crate) async fn get_user(
    State(state): State<AppState>,
    ContractPath(id): ContractPath<Uuid>,
) -> Result<Json<AdminUser>, AppError> {
    Ok(Json(load_user(&state, id).await?))
}

// A cursor belongs to a specific date and instance scope. The sequence is the
// stable order even when many events have the same second-resolution timestamp.
#[derive(Clone, Copy)]
enum LogDirection {
    Older,
    Newer,
}

fn log_cursor(date: &str, instance_id: Option<Uuid>, direction: &str, sequence: i64) -> String {
    let scope = instance_id.map_or_else(|| "all".to_owned(), |id| id.to_string());
    format!("{date}:{scope}:{direction}:{sequence}")
}

fn parse_log_cursor(
    value: &str,
    date: &str,
    instance_id: Option<Uuid>,
) -> Result<(LogDirection, i64), AppError> {
    let invalid = || AppError::bad_request("cursor is invalid for the selected date and instance");
    if value.len() > 96 {
        return Err(invalid());
    }
    let parts: Vec<_> = value.split(':').collect();
    if parts.len() != 4 || parts[0] != date {
        return Err(invalid());
    }
    let scope = instance_id.map_or_else(|| "all".to_owned(), |id| id.to_string());
    if parts[1] != scope {
        return Err(invalid());
    }
    let direction = match parts[2] {
        "older" => LogDirection::Older,
        "newer" => LogDirection::Newer,
        _ => return Err(invalid()),
    };
    let sequence = parts[3].parse::<i64>().map_err(|_| invalid())?;
    if !(1..=MAX_JSON_INTEGER).contains(&sequence) || sequence.to_string() != parts[3] {
        return Err(invalid());
    }
    Ok((direction, sequence))
}

// Administrator actions identify their device entity; client actions identify
// their authenticated device or its API key. Account ownership alone does not
// prove which instance performed an action.
const LOG_INSTANCE_PREDICATE: &str = "(?3 IS NULL OR \
    (entity_kind = 'device' AND entity_id = ?3) OR \
    (actor_kind = 'device' AND actor_id = ?3) OR \
    (actor_kind = 'api_key' AND actor_id IN (SELECT id FROM api_keys WHERE device_id = ?3)))";

fn select_log_range(query: &LogsQuery) -> Result<Option<(&str, &str)>, AppError> {
    let (start, end) = match (&query.date, &query.start_date, &query.end_date) {
        (None, None, None) => return Ok(None),
        (Some(date), None, None) => (date.as_str(), date.as_str()),
        (None, Some(start), Some(end)) => (start.as_str(), end.as_str()),
        _ => {
            return Err(AppError::bad_request(
                "provide date or both start_date and end_date",
            ));
        }
    };
    if !valid_calendar_date(start) || !valid_calendar_date(end) {
        return Err(AppError::bad_request("dates must be valid YYYY-MM-DD"));
    }
    if start > end {
        return Err(AppError::bad_request(
            "end_date must not precede start_date",
        ));
    }
    Ok(Some((start, end)))
}

pub(crate) async fn logs(
    State(state): State<AppState>,
    ContractQuery(query): ContractQuery<LogsQuery>,
) -> Result<Json<AdminLogs>, AppError> {
    let explicit = query.date.is_some() || query.start_date.is_some() || query.end_date.is_some();
    if query.cursor.is_some() && !explicit {
        return Err(AppError::bad_request(
            "cursor requires an explicit date range",
        ));
    }
    let selected = select_log_range(&query)?;
    let (date, end_date) = match selected {
        Some((start, end)) => (start.to_owned(), end.to_owned()),
        None => {
            let today: String = sqlx::query_scalar("SELECT date('now', 'localtime')")
                .fetch_one(&state.pool)
                .await?;
            (today.clone(), today)
        }
    };
    let range_scope = if date == end_date {
        date.clone()
    } else {
        format!("{date}/{end_date}")
    };
    let cursor = query
        .cursor
        .as_deref()
        .map(|value| parse_log_cursor(value, &range_scope, query.instance_id))
        .transpose()?;
    // Convert local midnights separately so DST gives a 23- or 25-hour day.
    let (start_utc, end_utc): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT datetime(?1 || ' 00:00:00', 'utc'), \
                datetime(date(?2, '+1 day') || ' 00:00:00', 'utc')",
    )
    .bind(&date)
    .bind(&end_date)
    .fetch_one(&state.pool)
    .await?;
    let (Some(start_utc), Some(end_utc)) = (start_utc, end_utc) else {
        return Err(AppError::bad_request("date is outside the supported range"));
    };
    let (sequence_predicate, order, anchor) = match cursor {
        None => ("", "DESC", None),
        Some((LogDirection::Older, sequence)) => ("AND sequence < ?4", "DESC", Some(sequence)),
        Some((LogDirection::Newer, sequence)) => ("AND sequence > ?4", "ASC", Some(sequence)),
    };
    // Only internal literals are interpolated. All request values are bound.
    let sql = format!(
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
        WHERE occurred_at >= ?1 AND occurred_at < ?2 AND {LOG_INSTANCE_PREDICATE}
        {sequence_predicate} ORDER BY sequence {order} LIMIT 50
        "#
    );
    let mut statement = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(&start_utc)
        .bind(&end_utc)
        .bind(query.instance_id);
    if let Some(anchor) = anchor {
        statement = statement.bind(anchor);
    }
    let rows = statement.fetch_all(&state.pool).await?;
    let mut logs = rows
        .into_iter()
        .map(|row| {
            let sequence = row.try_get("sequence")?;
            if !(1..=MAX_JSON_INTEGER).contains(&sequence) {
                return Err(AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "audit sequence is outside the JSON integer range",
                ));
            }
            Ok(AdminLog {
                sequence,
                action: row.try_get("action")?,
                entity_id: row.try_get("entity_id")?,
                occurred_at: server_local_time(
                    row.try_get("occurred_at")?,
                    row.try_get("utc_offset_seconds")?,
                )?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    if matches!(cursor, Some((LogDirection::Newer, _))) {
        logs.reverse();
    }
    let mut previous_cursor = None;
    let mut next_cursor = None;
    if let (Some(first), Some(last)) = (logs.first(), logs.last()) {
        for (comparison, direction, sequence, destination) in [
            (">", "newer", first.sequence, &mut previous_cursor),
            ("<", "older", last.sequence, &mut next_cursor),
        ] {
            let sql = format!(
                "SELECT EXISTS(SELECT 1 FROM audit_events \
                WHERE occurred_at >= ?1 AND occurred_at < ?2 AND {LOG_INSTANCE_PREDICATE} \
                AND sequence {comparison} ?4)"
            );
            let exists: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .bind(&start_utc)
                .bind(&end_utc)
                .bind(query.instance_id)
                .bind(sequence)
                .fetch_one(&state.pool)
                .await?;
            if exists {
                *destination = Some(log_cursor(
                    &range_scope,
                    query.instance_id,
                    direction,
                    sequence,
                ));
            }
        }
    }
    Ok(Json(AdminLogs {
        end_date: (date != end_date).then_some(end_date),
        date,
        instance_id: query.instance_id,
        logs,
        previous_cursor,
        next_cursor,
    }))
}

pub(crate) async fn create_instance(
    State(state): State<AppState>,
    ContractJson(request): ContractJson<CreateInstanceRequest>,
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
    tracing::info!(event = "xszs.instance.created", instance_id = %id, instance_type = "device");
    Ok((StatusCode::CREATED, load_instance(&state, id).await?))
}

pub(crate) async fn update_user(
    State(state): State<AppState>,
    ContractPath(id): ContractPath<Uuid>,
    ContractJson(request): ContractJson<UpdateUserRequest>,
) -> Result<Json<AdminUser>, AppError> {
    let username = request.username.trim();
    let display_name = request.display_name.trim();
    let storage_path = request.storage_path.trim();
    validate_username(username)?;
    validate_policy(display_name, storage_path, request.quota_bytes)?;
    ensure_unique_username(&state, username, Some(id)).await?;
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    ensure_unique_path(&state, storage_path, Some(id)).await?;
    let current_path: Option<String> =
        sqlx::query_scalar("SELECT storage_path FROM accounts WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?;
    let current_path = current_path.ok_or_else(|| AppError::not_found("user not found"))?;
    if current_path != storage_path {
        let has_media_or_active_upload: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM blobs WHERE account_id = ?1) OR EXISTS(\
             SELECT 1 FROM uploads WHERE account_id = ?1\
             AND (commit_state IN ('receiving', 'commit_started', 'finalizing', 'unknown')
                  OR commit_staged_key IS NOT NULL))",
        )
        .bind(id)
        .fetch_one(&mut *transaction)
        .await?;
        if has_media_or_active_upload {
            return Err(AppError::conflict(
                "storage_path cannot change while media or active uploads exist",
            ));
        }
    }
    state.storage.validate_account_path(storage_path).await?;
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
    ContractPath(id): ContractPath<Uuid>,
) -> Result<Json<AdminInstance>, AppError> {
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let instance: Option<(Uuid, String)> =
        sqlx::query_as("SELECT account_id, pairing_status FROM devices WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?;
    let (account_id, status) = instance.ok_or_else(|| AppError::not_found("instance not found"))?;
    if status != "pending" && status != "paired" {
        return Err(AppError::conflict(
            "revoked instance cannot be reauthorized",
        ));
    }
    let code = random_authorization_code();
    let encrypted = state
        .secrets
        .encrypt_client_authorization(&id.to_string(), &code)?;
    let hash = Sha256::digest(code.as_bytes()).to_vec();
    sqlx::query("UPDATE devices SET token_hash=NULL,authorization_code_hash=?,authorization_code_enc=?,pairing_status='pending',last_seen_at=datetime('now') WHERE id=?")
        .bind(hash).bind(encrypted).bind(id).execute(&mut *transaction).await?;
    sqlx::query("UPDATE api_keys SET revoked_at = datetime('now') WHERE device_id = ? AND revoked_at IS NULL")
        .bind(id).execute(&mut *transaction).await?;
    write_instance_audit(
        &mut transaction,
        account_id,
        id,
        "device.authorization.rotate",
    )
    .await?;
    transaction.commit().await?;
    tracing::info!(event = "xszs.instance.authorization_rotated", instance_id = %id, instance_type = "device");
    load_instance(&state, id).await
}

pub(crate) async fn remove_instance(
    State(state): State<AppState>,
    ContractPath(id): ContractPath<Uuid>,
) -> Result<StatusCode, AppError> {
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT account_id,pairing_status FROM devices WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?;
    let Some((account_id, status)) = row else {
        return Err(AppError::not_found("instance not found"));
    };
    if status == "cancelled" || status == "revoked" {
        let references: i64 = sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM assets WHERE device_id=?) + (SELECT COUNT(*) FROM uploads WHERE device_id=?) + (SELECT COUNT(*) FROM albums WHERE device_id=?) + (SELECT COUNT(*) FROM api_keys WHERE device_id=? AND revoked_at IS NULL)")
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
        sqlx::query("UPDATE api_keys SET revoked_at = datetime('now') WHERE device_id = ? AND revoked_at IS NULL")
            .bind(id).execute(&mut *transaction).await?;
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
    let event = if status == "cancelled" || status == "revoked" {
        "xszs.instance.deleted"
    } else if status == "pending" {
        "xszs.instance.pairing_cancelled"
    } else {
        "xszs.instance.revoked"
    };
    tracing::info!(event, instance_id = %id, instance_type = "device");
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
        SysRng
            .try_fill_bytes(&mut bytes)
            .expect("operating system entropy is available");
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
    load_instances(state, id)
        .await?
        .into_iter()
        .next()
        .map(Json)
        .ok_or_else(|| AppError::not_found("instance not found"))
}

async fn load_instances(state: &AppState, only: Uuid) -> Result<Vec<AdminInstance>, AppError> {
    let rows = sqlx::query("SELECT id,account_id,name,platform,pairing_status,(pairing_status='paired' AND last_seen_at IS NOT NULL AND last_seen_at>=datetime('now','-10 minutes')) online,authorization_code_enc,strftime('%Y-%m-%dT%H:%M:%SZ',created_at) created_at,COALESCE(strftime('%Y-%m-%dT%H:%M:%SZ',last_seen_at),'') last_seen_at FROM devices WHERE id=? LIMIT 1")
        .bind(only).fetch_all(&state.pool).await?;
    rows.into_iter()
        .map(|row| {
            let id: Uuid = row.get("id");
            let encrypted: Vec<u8> = row.get("authorization_code_enc");
            Ok(AdminInstance {
                id,
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

// Current schema enforces devices_account_unique_idx. LEFT JOIN yields at most
// one instance per account; the page and a direct detail response stay bounded.
const USER_SELECT: &str = "SELECT a.id,a.username,a.display_name,a.storage_path,a.quota_bytes, \
    strftime('%Y-%m-%dT%H:%M:%SZ',a.created_at) AS created_at, \
    COALESCE((SELECT SUM(b.stored_size) FROM blobs b WHERE b.account_id=a.id),0) AS used_bytes, \
    COALESCE((SELECT SUM(p.expected_size) FROM upload_parts p JOIN uploads u ON u.id=p.upload_id WHERE u.account_id=a.id AND u.state='uploading' AND u.commit_state IN ('receiving','commit_started','finalizing','unknown')),0) AS pending_bytes, \
    CASE WHEN d.id IS NULL THEN 0 ELSE 1 END AS device_count, \
    (SELECT COUNT(*) FROM resources r JOIN assets s ON s.id=r.asset_id WHERE s.account_id=a.id) AS resource_count, \
    COALESCE(strftime('%Y-%m-%dT%H:%M:%SZ',d.last_seen_at),'') AS last_seen_at, \
    d.id instance_id,d.name instance_name,d.platform,d.pairing_status, \
    COALESCE(d.pairing_status='paired' AND d.last_seen_at IS NOT NULL AND d.last_seen_at>=datetime('now','-10 minutes'),0) online, \
    d.authorization_code_enc,strftime('%Y-%m-%dT%H:%M:%SZ',d.created_at) instance_created_at \
    FROM accounts a LEFT JOIN devices d ON d.account_id=a.id";

async fn load_user(state: &AppState, id: Uuid) -> Result<AdminUser, AppError> {
    let sql = format!("{USER_SELECT} WHERE a.id=?");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::not_found("user not found"))?;
    row_to_user(state, row)
}

fn row_to_user(state: &AppState, row: sqlx::sqlite::SqliteRow) -> Result<AdminUser, AppError> {
    let account_id = row.try_get("id")?;
    let instance_id: Option<Uuid> = row.try_get("instance_id")?;
    let last_seen_at: String = row.try_get("last_seen_at")?;
    let instances = if let Some(id) = instance_id {
        let encrypted: Vec<u8> = row.try_get("authorization_code_enc")?;
        vec![AdminInstance {
            id,
            name: row.try_get("instance_name")?,
            platform: row.try_get("platform")?,
            status: row.try_get("pairing_status")?,
            online: row.try_get("online")?,
            authorization_code: state
                .secrets
                .decrypt_client_authorization(&id.to_string(), &encrypted)?,
            created_at: row.try_get("instance_created_at")?,
            last_seen_at: last_seen_at.clone(),
        }]
    } else {
        Vec::new()
    };
    let exact = |field| -> Result<i64, AppError> { require_json_integer(row.try_get(field)?) };
    Ok(AdminUser {
        id: account_id,
        username: row.try_get("username")?,
        display_name: row.try_get("display_name")?,
        storage_path: row.try_get("storage_path")?,
        quota_bytes: exact("quota_bytes")?,
        used_bytes: exact("used_bytes")?,
        pending_bytes: exact("pending_bytes")?,
        device_count: row.try_get("device_count")?,
        resource_count: exact("resource_count")?,
        created_at: row.try_get("created_at")?,
        last_seen_at,
        instances,
    })
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

pub(crate) async fn web_page(State(state): State<AppState>, request: Request) -> Response {
    let path = request.uri().path().strip_prefix("/admin").unwrap_or("");
    let mut response = if let Some(directory) = state.web_directory.as_deref() {
        directory
            .response(path, request.method(), request.headers())
            .map(axum::body::Body::from)
    } else {
        crate::web_assets::response(path, request.method(), request.headers())
    };
    if path.is_empty() || path == "/" {
        secure_page(&mut response);
    }
    response
}

fn secure_page(response: &mut Response) {
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(
            "default-src 'self'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self' data:; font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, Method};

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
        const CHILD: &str = "XSZS_LOG_DST_TEST_CHILD";
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
        xcss_sqlite::block_on_sqlite_connection(async {
            use sqlx::Connection as _;
            let mut database = sqlx::SqliteConnection::connect("sqlite::memory:")
                .await
                .unwrap();
            for (day, expected_start, expected_end) in [
                ("2026-03-08", "2026-03-08 05:00:00", "2026-03-09 04:00:00"),
                ("2026-11-01", "2026-11-01 04:00:00", "2026-11-02 05:00:00"),
            ] {
                let actual: (String, String) = sqlx::query_as(
                    "SELECT datetime(?1 || ' 00:00:00', 'utc'), \
                            datetime(date(?2, '+1 day') || ' 00:00:00', 'utc')",
                )
                .bind(day)
                .bind(day)
                .fetch_one(&mut database)
                .await
                .unwrap();
                assert_eq!(actual, (expected_start.into(), expected_end.into()));
            }
            database.close().await.unwrap();
        });
    }

    #[test]
    fn generated_authorization_codes_have_the_shared_format() {
        for _ in 0..64 {
            let value = random_authorization_code();
            assert_eq!(value.len(), 36);
            assert!(
                value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
            );
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
        for value in [0, 123, MAX_JSON_INTEGER] {
            assert_eq!(require_json_integer(value).unwrap(), value);
        }
        for value in [-1, MAX_JSON_INTEGER + 1, i64::MAX] {
            assert!(require_json_integer(value).is_err());
        }
    }

    #[test]
    fn create_request_uses_the_server_default_when_name_is_omitted() {
        let request: CreateInstanceRequest = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(request.name.is_none());
        assert!(
            serde_json::from_value::<CreateInstanceRequest>(serde_json::json!({"other": true}))
                .is_err()
        );
    }

    #[tokio::test]
    async fn embedded_fonts_and_license_are_the_verified_release_bytes() {
        for asset in crate::web_assets::ASSETS {
            let path = asset.path;
            let bytes = asset.bytes;
            if !path.ends_with(".woff2") && !path.ends_with(".txt") {
                continue;
            }
            let response = crate::web_assets::response(path, &Method::GET, &HeaderMap::new());
            let content_type = if path.ends_with(".woff2") {
                "font/woff2"
            } else {
                "text/plain; charset=utf-8"
            };
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-cache")
            );
            assert!(response.headers().contains_key(header::ETAG));
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
            let body = axum::body::to_bytes(response.into_body(), 512 * 1024)
                .await
                .unwrap();
            assert_eq!(body.as_ref(), bytes);
            if path.ends_with(".woff2") {
                assert!(bytes.starts_with(b"wOF2"));
            } else {
                assert!(
                    std::str::from_utf8(bytes)
                        .unwrap()
                        .contains("SIL OPEN FONT LICENSE")
                );
            }
        }
        for name in ["../index.html", "missing.woff2", "MapleMono.woff2"] {
            assert_eq!(
                crate::web_assets::response(
                    &format!("assets/{name}"),
                    &Method::GET,
                    &HeaderMap::new()
                )
                .status(),
                StatusCode::NOT_FOUND
            );
        }
    }

    #[tokio::test]
    async fn embedded_assets_revalidate_without_retransmitting_the_body() {
        let initial =
            crate::web_assets::response("assets/admin.css", &Method::GET, &HeaderMap::new());
        let etag = initial.headers()[header::ETAG].clone();
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, etag);
        let cached = crate::web_assets::response("assets/admin.css", &Method::GET, &headers);
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
        assert!(
            axum::body::to_bytes(cached.into_body(), 1)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn administrator_page_cannot_execute_inline_or_external_scripts() {
        let mut response =
            crate::web_assets::response("index.html", &Method::GET, &HeaderMap::new());
        secure_page(&mut response);
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
