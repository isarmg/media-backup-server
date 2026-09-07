use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use media_backup_protocol::{
    ApiKeyCreated, ApiKeyRecord, AuditEvent, AuditPage, CreateApiKeyRequest, EmptyRequest,
};
use rand::{rngs::OsRng, RngCore};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::{audit, auth::AuthContext, error::AppError, routes::AppState};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditQuery {
    before: Option<i64>,
    limit: Option<u32>,
}

pub async fn create_api_key(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiKeyCreated>), AppError> {
    let name = request.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::bad_request(
            "API key name must contain 1 to 100 characters",
        ));
    }
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = format!("pbk_{}", URL_SAFE_NO_PAD.encode(bytes));
    let prefix = token.chars().take(12).collect::<String>();
    let hash = Sha256::digest(token.as_bytes()).to_vec();
    let mut transaction = state.pool.begin().await?;
    let row = sqlx::query(
        r#"
        INSERT INTO api_keys(id, account_id, device_id, name, prefix, token_hash, created_at)
        VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
        RETURNING id, (CAST(strftime('%s', created_at) AS INTEGER) * 1000) AS created_at_ms
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(auth.account_id)
    .bind(auth.device_id)
    .bind(name)
    .bind(&prefix)
    .bind(hash)
    .fetch_one(&mut *transaction)
    .await?;
    let api_key_id = row.get("id");
    audit::record_in_transaction(
        &mut transaction,
        &auth,
        "api_key.create",
        Some("api_key"),
        Some(api_key_id),
    )
    .await?;
    transaction.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiKeyCreated {
            api_key_id,
            name: name.to_owned(),
            token,
            prefix,
            created_at_ms: row.get("created_at_ms"),
        }),
    ))
}

pub async fn list_api_keys(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<Vec<ApiKeyRecord>>, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT id, name, prefix,
               (CAST(strftime('%s', created_at) AS INTEGER) * 1000) AS created_at_ms,
               CASE WHEN last_used_at IS NULL THEN NULL
                    ELSE (CAST(strftime('%s', last_used_at) AS INTEGER) * 1000) END AS last_used_at_ms
        FROM api_keys
        WHERE account_id = ? AND revoked_at IS NULL
        ORDER BY created_at DESC
        "#,
    )
    .bind(auth.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| ApiKeyRecord {
                api_key_id: row.get("id"),
                name: row.get("name"),
                prefix: row.get("prefix"),
                created_at_ms: row.get("created_at_ms"),
                last_used_at_ms: row.get("last_used_at_ms"),
            })
            .collect(),
    ))
}

pub async fn revoke_api_key(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(api_key_id): Path<Uuid>,
    Json(_request): Json<EmptyRequest>,
) -> Result<StatusCode, AppError> {
    let mut transaction = state.pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE api_keys SET revoked_at = datetime('now') WHERE id = ? AND account_id = ? AND revoked_at IS NULL",
    )
    .bind(api_key_id)
    .bind(auth.account_id)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::not_found("API key not found"));
    }
    audit::record_in_transaction(
        &mut transaction,
        &auth,
        "api_key.revoke",
        Some("api_key"),
        Some(api_key_id),
    )
    .await?;
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn audit_events(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<AuditQuery>,
) -> Result<Json<AuditPage>, AppError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500) as i64;
    let rows = sqlx::query(
        r#"
        SELECT sequence, actor_kind, action, entity_kind, entity_id,
               (CAST(strftime('%s', occurred_at) AS INTEGER) * 1000) AS occurred_at_ms
        FROM audit_events
        WHERE account_id = ?1 AND (?2 IS NULL OR sequence < ?2)
        ORDER BY sequence DESC LIMIT ?3
        "#,
    )
    .bind(auth.account_id)
    .bind(query.before)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let has_more = rows.len() as i64 > limit;
    let events = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| AuditEvent {
            sequence: row.get("sequence"),
            actor_kind: row.get("actor_kind"),
            action: row.get("action"),
            entity_kind: row.get("entity_kind"),
            entity_id: row.get("entity_id"),
            occurred_at_ms: row.get("occurred_at_ms"),
        })
        .collect::<Vec<_>>();
    let next_sequence = has_more
        .then(|| events.last().map(|event| event.sequence))
        .flatten();
    Ok(Json(AuditPage {
        events,
        next_sequence,
    }))
}
