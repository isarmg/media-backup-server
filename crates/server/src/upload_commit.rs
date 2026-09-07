use std::collections::HashSet;

use axum::http::StatusCode;
use media_backup_protocol::CreateUploadRequest;
use serde_json::Value;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{
    audit,
    error::AppError,
    routes::AppState,
    storage::{CommitKeys, ObjectState},
};

const RECEIVING: &str = "receiving";
const COMMIT_STARTED: &str = "commit_started";
const FINALIZING: &str = "finalizing";
const COMMITTED: &str = "committed";
const UNKNOWN: &str = "unknown";
const FAILED: &str = "failed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommitOutcome {
    pub resource_id: Uuid,
    pub asset_id: Uuid,
    pub deduplicated: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ReconcileReport {
    pub recovered: u64,
    pub marked_unknown: u64,
    pub orphan_stages_removed: u64,
    pub orphan_blobs_removed: u64,
    pub errors: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitFailpoint {
    CommitStarted,
    StageFsync,
    Finalizing,
    Published,
    MetadataCommitted,
}

#[derive(Debug, Clone)]
struct CommitRecord {
    upload_id: Uuid,
    account_id: Uuid,
    asset_id: Uuid,
    request: CreateUploadRequest,
    commit_state: String,
    staged_key: Option<String>,
    final_key: Option<String>,
    commit_account_path: Option<String>,
    expected_size: Option<i64>,
    expected_blake3: Option<String>,
    blob_id: Option<Uuid>,
    resource_id: Option<Uuid>,
    deduplicated: bool,
    current_account_path: String,
}

#[derive(Debug, Clone)]
struct ReadyCommit {
    upload_id: Uuid,
    account_id: Uuid,
    asset_id: Uuid,
    request: CreateUploadRequest,
    staged_key: Option<String>,
    final_key: String,
    account_path: String,
    expected_size: u64,
    expected_blake3: String,
    proposed_blob_id: Uuid,
}

#[derive(Debug, Clone)]
struct BlobCandidate {
    id: Uuid,
    storage_path: String,
    stored_size: i64,
    content_blake3: String,
}

enum MetadataResult {
    Committed(CommitOutcome),
    ValidateCandidate(BlobCandidate),
    AccountPathChanged,
}

pub(crate) async fn complete(
    state: &AppState,
    upload_id: Uuid,
    account_id: Uuid,
) -> Result<CommitOutcome, AppError> {
    complete_inner(state, upload_id, Some(account_id), None).await
}

#[cfg(test)]
pub(crate) async fn complete_with_failpoint(
    state: &AppState,
    upload_id: Uuid,
    account_id: Uuid,
    failpoint: CommitFailpoint,
) -> Result<CommitOutcome, AppError> {
    complete_inner(state, upload_id, Some(account_id), Some(failpoint)).await
}

async fn complete_inner(
    state: &AppState,
    upload_id: Uuid,
    required_account_id: Option<Uuid>,
    #[cfg_attr(not(test), allow(unused_variables))] failpoint: Option<CommitFailpoint>,
) -> Result<CommitOutcome, AppError> {
    let _guard = state.storage.lock_upload_commit(upload_id).await?;
    let mut record = load_commit(&state.pool, upload_id, required_account_id).await?;
    match record.commit_state.as_str() {
        COMMITTED => return validate_committed(state, &record).await,
        UNKNOWN => return Err(AppError::conflict("upload commit state is unknown")),
        FAILED => return Err(AppError::conflict("upload commit has failed")),
        RECEIVING => {
            record = begin_commit(state, record).await?;
            stop_at(failpoint, CommitFailpoint::CommitStarted)?;
        }
        COMMIT_STARTED | FINALIZING => {}
        _ => return mark_unknown(state, &record, "invalid persisted commit state").await,
    }

    let ready = match ready_commit(&record) {
        Ok(ready) => ready,
        Err(message) => return mark_unknown(state, &record, message).await,
    };
    if record.commit_state == COMMIT_STARTED {
        match state
            .storage
            .inspect_object(
                &ready.account_path,
                &ready.final_key,
                ready.expected_size,
                &ready.expected_blake3,
            )
            .await
        {
            Ok(ObjectState::Matches) => {}
            Ok(ObjectState::Mismatch) => {
                return mark_unknown(state, &record, "final blob hash or size conflicts").await;
            }
            Ok(ObjectState::Missing) if ready.staged_key.is_none() => {
                return mark_unknown(state, &record, "deduplicated blob is missing").await;
            }
            Ok(ObjectState::Missing) => {
                let Some(staged_key) = ready.staged_key.as_deref() else {
                    return mark_unknown(state, &record, "upload commit has no staged key").await;
                };
                if let Err(error) = state
                    .storage
                    .assemble_commit(
                        &ready.account_path,
                        ready.upload_id,
                        &ready.request.parts,
                        staged_key,
                        ready.expected_size,
                        &ready.expected_blake3,
                    )
                    .await
                {
                    tracing::warn!(upload_id = %upload_id, ?error, "failed to assemble upload commit");
                    if error.status == StatusCode::CONFLICT {
                        return mark_failed_message(&state.pool, upload_id, error.message()).await;
                    }
                    return Err(error);
                }
                stop_at(failpoint, CommitFailpoint::StageFsync)?;
            }
            Err(error) => {
                return mark_unknown(
                    state,
                    &record,
                    &format!("cannot safely inspect final blob: {error}"),
                )
                .await;
            }
        }
        transition_to_finalizing(&state.pool, upload_id).await?;
        stop_at(failpoint, CommitFailpoint::Finalizing)?;
        record.commit_state = FINALIZING.to_owned();
    }

    if record.commit_state == FINALIZING {
        match state
            .storage
            .inspect_object(
                &ready.account_path,
                &ready.final_key,
                ready.expected_size,
                &ready.expected_blake3,
            )
            .await
        {
            Ok(ObjectState::Matches) => {}
            Ok(ObjectState::Mismatch) => {
                return mark_unknown(state, &record, "final blob hash or size conflicts").await;
            }
            Ok(ObjectState::Missing) => {
                let Some(staged_key) = ready.staged_key.as_deref() else {
                    return mark_unknown(state, &record, "deduplicated blob is missing").await;
                };
                match state
                    .storage
                    .inspect_commit_stage(
                        &ready.account_path,
                        ready.upload_id,
                        staged_key,
                        ready.expected_size,
                        &ready.expected_blake3,
                    )
                    .await
                {
                    Ok(ObjectState::Matches) => {}
                    Ok(ObjectState::Missing) => {
                        return mark_unknown(
                            state,
                            &record,
                            "finalizing upload has neither staged nor final blob",
                        )
                        .await;
                    }
                    Ok(ObjectState::Mismatch) => {
                        return mark_unknown(state, &record, "staged blob is corrupt").await;
                    }
                    Err(error) => {
                        return mark_unknown(
                            state,
                            &record,
                            &format!("cannot safely inspect staged blob: {error}"),
                        )
                        .await;
                    }
                }
                if let Err(error) = state
                    .storage
                    .publish_commit(
                        &ready.account_path,
                        ready.upload_id,
                        staged_key,
                        &ready.final_key,
                        ready.expected_size,
                        &ready.expected_blake3,
                    )
                    .await
                {
                    return mark_unknown(
                        state,
                        &record,
                        &format!("cannot prove final blob publication: {error}"),
                    )
                    .await;
                }
                stop_at(failpoint, CommitFailpoint::Published)?;
            }
            Err(error) => {
                return mark_unknown(
                    state,
                    &record,
                    &format!("cannot safely inspect final blob: {error}"),
                )
                .await;
            }
        }
    }

    let outcome = commit_metadata_with_race_retry(state, &ready).await?;
    stop_at(failpoint, CommitFailpoint::MetadataCommitted)?;
    if let Some(staged_key) = ready.staged_key.as_deref() {
        if let Err(error) =
            state
                .storage
                .remove_commit_stage(&ready.account_path, ready.upload_id, staged_key)
        {
            tracing::warn!(upload_id = %upload_id, ?error, "committed staged blob needs reconciliation cleanup");
        }
    }
    Ok(outcome)
}

async fn begin_commit(state: &AppState, record: CommitRecord) -> Result<CommitRecord, AppError> {
    if record.request.parts.is_empty()
        || record.request.content_size
            != record
                .request
                .parts
                .iter()
                .try_fold(0_u64, |sum, part| sum.checked_add(part.size))
                .ok_or_else(|| AppError::conflict("upload manifest size overflow"))?
    {
        return mark_failed(state, &record, "persisted upload manifest is invalid").await;
    }
    let missing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM upload_parts WHERE upload_id = ? AND received_at IS NULL",
    )
    .bind(record.upload_id)
    .fetch_one(&state.pool)
    .await?;
    if missing != 0 {
        return Err(AppError::conflict("upload still has missing parts"));
    }

    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query("SELECT enabled, storage_path, quota_bytes FROM accounts WHERE id = ?")
        .bind(record.account_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(AppError::unauthorized)?;
    if !row.get::<bool, _>("enabled") {
        return Err(AppError::new(StatusCode::FORBIDDEN, "account is disabled"));
    }
    let account_path: String = row.get("storage_path");
    let quota_bytes: i64 = row.get("quota_bytes");
    let existing = load_blob_candidate_in(
        &mut transaction,
        record.account_id,
        &record.request.content_blake3,
    )
    .await?;

    let (next_state, keys, blob_id) = if let Some(existing) = existing {
        (
            FINALIZING,
            CommitKeys {
                staged: String::new(),
                final_blob: existing.storage_path,
            },
            existing.id,
        )
    } else {
        ensure_commit_quota(
            &mut transaction,
            record.account_id,
            quota_bytes,
            record.upload_id,
        )
        .await?;
        (
            COMMIT_STARTED,
            state.storage.commit_keys(
                &account_path,
                record.upload_id,
                &record.request.content_blake3,
            )?,
            Uuid::new_v4(),
        )
    };
    let updated =
        sqlx::query(
            r#"
        UPDATE uploads
        SET commit_state = ?, commit_staged_key = ?, commit_final_key = ?,
            commit_account_path = ?, commit_expected_size = ?, commit_expected_blake3 = ?,
            commit_blob_id = ?, commit_resource_id = NULL, commit_error = NULL,
            commit_deduplicated = 0,
            commit_started_at = datetime('now'), finalized_at = NULL,
            updated_at = datetime('now')
        WHERE id = ? AND commit_state = 'receiving'
        "#,
        )
        .bind(next_state)
        .bind((!keys.staged.is_empty()).then_some(keys.staged))
        .bind(keys.final_blob)
        .bind(account_path)
        .bind(i64::try_from(record.request.content_size).map_err(|_| {
            AppError::conflict("upload expected size cannot be represented by SQLite")
        })?)
        .bind(&record.request.content_blake3)
        .bind(blob_id)
        .bind(record.upload_id)
        .execute(&mut *transaction)
        .await?;
    if updated.rows_affected() != 1 {
        return Err(AppError::conflict("upload commit changed concurrently"));
    }
    transaction.commit().await?;
    load_commit(&state.pool, record.upload_id, Some(record.account_id)).await
}

async fn transition_to_finalizing(pool: &SqlitePool, upload_id: Uuid) -> Result<(), AppError> {
    let updated = sqlx::query(
        "UPDATE uploads SET commit_state = 'finalizing', finalized_at = datetime('now'), updated_at = datetime('now') WHERE id = ? AND commit_state = 'commit_started'",
    )
    .bind(upload_id)
    .execute(pool)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(AppError::conflict("upload commit changed concurrently"));
    }
    Ok(())
}

async fn commit_metadata_with_race_retry(
    state: &AppState,
    ready: &ReadyCommit,
) -> Result<CommitOutcome, AppError> {
    let mut approved_candidate = None;
    for _ in 0..3 {
        if let Some(candidate) =
            load_blob_candidate(&state.pool, ready.account_id, &ready.expected_blake3).await?
        {
            validate_blob_candidate(state, ready, &candidate).await?;
            approved_candidate = Some(candidate);
        }
        match commit_metadata(state, ready, approved_candidate.as_ref()).await? {
            MetadataResult::Committed(outcome) => return Ok(outcome),
            MetadataResult::ValidateCandidate(candidate) => {
                validate_blob_candidate(state, ready, &candidate).await?;
                approved_candidate = Some(candidate);
            }
            MetadataResult::AccountPathChanged => {
                return mark_unknown_message(
                    &state.pool,
                    ready.upload_id,
                    "account storage path changed during upload commit",
                )
                .await;
            }
        }
    }
    Err(AppError::conflict(
        "blob metadata changed repeatedly during commit",
    ))
}

async fn commit_metadata(
    state: &AppState,
    ready: &ReadyCommit,
    approved_candidate: Option<&BlobCandidate>,
) -> Result<MetadataResult, AppError> {
    let mut transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let current_account_path: Option<String> =
        sqlx::query_scalar("SELECT storage_path FROM accounts WHERE id = ?")
            .bind(ready.account_id)
            .fetch_optional(&mut *transaction)
            .await?;
    if current_account_path.as_deref() != Some(ready.account_path.as_str()) {
        transaction.rollback().await?;
        return Ok(MetadataResult::AccountPathChanged);
    }
    sqlx::query(
        r#"
        INSERT INTO blobs(
            id, account_id, content_blake3, plaintext_size, stored_size, storage_path,
            part_manifest, storage_encoding, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, 'plain-v1', datetime('now'))
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(ready.proposed_blob_id)
    .bind(ready.account_id)
    .bind(&ready.expected_blake3)
    .bind(
        i64::try_from(ready.request.content_size).map_err(|_| {
            AppError::conflict("upload expected size cannot be represented by SQLite")
        })?,
    )
    .bind(
        i64::try_from(ready.expected_size).map_err(|_| {
            AppError::conflict("upload expected size cannot be represented by SQLite")
        })?,
    )
    .bind(&ready.final_key)
    .bind(serde_json::to_value(&ready.request.parts)?)
    .execute(&mut *transaction)
    .await?;

    let actual = load_blob_candidate_in(&mut transaction, ready.account_id, &ready.expected_blake3)
        .await?
        .ok_or_else(|| AppError::conflict("blob metadata insert did not persist"))?;
    let approved = actual.id == ready.proposed_blob_id && actual.storage_path == ready.final_key
        || approved_candidate.is_some_and(|candidate| {
            candidate.id == actual.id && candidate.storage_path == actual.storage_path
        });
    if !approved {
        transaction.rollback().await?;
        return Ok(MetadataResult::ValidateCandidate(actual));
    }

    let resource_id =
        upsert_resource(&mut transaction, ready.asset_id, actual.id, &ready.request).await?;
    audit::record_change(
        &mut transaction,
        ready.account_id,
        "asset",
        ready.asset_id,
        "upsert",
    )
    .await?;
    sqlx::query(
        r#"
        INSERT INTO audit_events(
            account_id, actor_kind, actor_id, action, entity_kind, entity_id, occurred_at
        ) VALUES (?, 'system', NULL, 'upload.commit', 'resource', ?, datetime('now'))
        "#,
    )
    .bind(ready.account_id)
    .bind(resource_id)
    .execute(&mut *transaction)
    .await?;
    let deduplicated = ready.staged_key.is_none() || actual.id != ready.proposed_blob_id;
    let updated = sqlx::query(
        r#"
        UPDATE uploads
        SET state = 'complete', commit_state = 'committed', commit_blob_id = ?,
            commit_resource_id = ?, commit_final_key = ?, commit_deduplicated = ?,
            commit_error = NULL,
            completed_at = datetime('now'),
            updated_at = datetime('now')
        WHERE id = ? AND commit_state = 'finalizing'
        "#,
    )
    .bind(actual.id)
    .bind(resource_id)
    .bind(&actual.storage_path)
    .bind(deduplicated)
    .bind(ready.upload_id)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(AppError::conflict("upload commit changed concurrently"));
    }
    transaction.commit().await?;
    Ok(MetadataResult::Committed(CommitOutcome {
        resource_id,
        asset_id: ready.asset_id,
        deduplicated,
    }))
}

async fn validate_blob_candidate(
    state: &AppState,
    ready: &ReadyCommit,
    candidate: &BlobCandidate,
) -> Result<(), AppError> {
    if candidate.stored_size < 0
        || u64::try_from(candidate.stored_size).ok() != Some(ready.expected_size)
        || candidate.content_blake3 != ready.expected_blake3
    {
        return mark_unknown_message(
            &state.pool,
            ready.upload_id,
            "existing blob metadata conflicts with upload manifest",
        )
        .await;
    }
    match state
        .storage
        .inspect_object(
            &ready.account_path,
            &candidate.storage_path,
            ready.expected_size,
            &ready.expected_blake3,
        )
        .await
    {
        Ok(ObjectState::Matches) => Ok(()),
        Ok(ObjectState::Missing | ObjectState::Mismatch) => {
            mark_unknown_message(
                &state.pool,
                ready.upload_id,
                "existing blob file is missing or corrupt",
            )
            .await
        }
        Err(error) => {
            mark_unknown_message(
                &state.pool,
                ready.upload_id,
                &format!("cannot safely validate existing blob: {error}"),
            )
            .await
        }
    }
}

async fn validate_committed(
    state: &AppState,
    record: &CommitRecord,
) -> Result<CommitOutcome, AppError> {
    let ready = match ready_commit(record) {
        Ok(ready) => ready,
        Err(message) => return mark_unknown(state, record, message).await,
    };
    let resource_id = match record.resource_id {
        Some(resource_id) => resource_id,
        None => return mark_unknown(state, record, "committed upload has no resource id").await,
    };
    let metadata = sqlx::query(
        r#"
        SELECT b.storage_path, b.stored_size, b.content_blake3
        FROM resources r
        JOIN blobs b ON b.id = r.blob_id
        WHERE r.id = ? AND r.asset_id = ? AND b.id = ? AND b.account_id = ?
        "#,
    )
    .bind(resource_id)
    .bind(ready.asset_id)
    .bind(ready.proposed_blob_id)
    .bind(ready.account_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(metadata) = metadata else {
        return mark_unknown(state, record, "committed blob/resource metadata is missing").await;
    };
    if metadata.get::<String, _>("storage_path") != ready.final_key
        || u64::try_from(metadata.get::<i64, _>("stored_size")).ok() != Some(ready.expected_size)
        || metadata.get::<String, _>("content_blake3") != ready.expected_blake3
    {
        return mark_unknown(state, record, "committed blob/resource metadata conflicts").await;
    }
    match state
        .storage
        .inspect_object(
            &ready.account_path,
            &ready.final_key,
            ready.expected_size,
            &ready.expected_blake3,
        )
        .await
    {
        Ok(ObjectState::Matches) => {}
        Ok(ObjectState::Missing | ObjectState::Mismatch) => {
            return mark_unknown(state, record, "committed blob is missing or corrupt").await;
        }
        Err(error) => {
            return mark_unknown(
                state,
                record,
                &format!("cannot safely validate committed blob: {error}"),
            )
            .await;
        }
    }
    if let Some(staged_key) = ready.staged_key.as_deref() {
        state
            .storage
            .remove_commit_stage(&ready.account_path, ready.upload_id, staged_key)?;
    }
    Ok(CommitOutcome {
        resource_id,
        asset_id: ready.asset_id,
        deduplicated: record.deduplicated,
    })
}

fn ready_commit(record: &CommitRecord) -> Result<ReadyCommit, &'static str> {
    let account_path = record
        .commit_account_path
        .clone()
        .ok_or("upload commit has no account path")?;
    if account_path != record.current_account_path {
        return Err("account storage path changed during upload commit");
    }
    let expected_size = record
        .expected_size
        .and_then(|value| u64::try_from(value).ok())
        .ok_or("upload commit has no valid expected size")?;
    let expected_blake3 = record
        .expected_blake3
        .clone()
        .ok_or("upload commit has no expected hash")?;
    if expected_size != record.request.content_size
        || expected_blake3 != record.request.content_blake3
    {
        return Err("upload commit proof does not match its request");
    }
    Ok(ReadyCommit {
        upload_id: record.upload_id,
        account_id: record.account_id,
        asset_id: record.asset_id,
        request: record.request.clone(),
        staged_key: record.staged_key.clone(),
        final_key: record
            .final_key
            .clone()
            .ok_or("upload commit has no final key")?,
        account_path,
        expected_size,
        expected_blake3,
        proposed_blob_id: record.blob_id.ok_or("upload commit has no blob id")?,
    })
}

async fn load_commit(
    pool: &SqlitePool,
    upload_id: Uuid,
    required_account_id: Option<Uuid>,
) -> Result<CommitRecord, AppError> {
    let row = sqlx::query(
        r#"
        SELECT u.id, u.account_id, u.asset_id, u.request, u.commit_state,
               u.commit_staged_key, u.commit_final_key, u.commit_account_path,
               u.commit_expected_size, u.commit_expected_blake3, u.commit_blob_id,
               u.commit_resource_id, u.commit_deduplicated,
               a.storage_path AS current_account_path
        FROM uploads u
        JOIN accounts a ON a.id = u.account_id
        WHERE u.id = ? AND (?2 IS NULL OR u.account_id = ?2)
        "#,
    )
    .bind(upload_id)
    .bind(required_account_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::not_found("upload not found"))?;
    Ok(CommitRecord {
        upload_id: row.get("id"),
        account_id: row.get("account_id"),
        asset_id: row.get("asset_id"),
        request: serde_json::from_value(row.get::<Value, _>("request"))?,
        commit_state: row.get("commit_state"),
        staged_key: row.get("commit_staged_key"),
        final_key: row.get("commit_final_key"),
        commit_account_path: row.get("commit_account_path"),
        expected_size: row.get("commit_expected_size"),
        expected_blake3: row.get("commit_expected_blake3"),
        blob_id: row.get("commit_blob_id"),
        resource_id: row.get("commit_resource_id"),
        deduplicated: row.get("commit_deduplicated"),
        current_account_path: row.get("current_account_path"),
    })
}

async fn load_blob_candidate(
    pool: &SqlitePool,
    account_id: Uuid,
    content_blake3: &str,
) -> Result<Option<BlobCandidate>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT id, storage_path, stored_size, content_blake3
        FROM blobs
        WHERE account_id = ? AND content_blake3 = ?
        "#,
    )
    .bind(account_id)
    .bind(content_blake3)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(blob_candidate_from_row))
}

async fn load_blob_candidate_in(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: Uuid,
    content_blake3: &str,
) -> Result<Option<BlobCandidate>, AppError> {
    let row = sqlx::query(
        r#"
        SELECT id, storage_path, stored_size, content_blake3
        FROM blobs
        WHERE account_id = ? AND content_blake3 = ?
        "#,
    )
    .bind(account_id)
    .bind(content_blake3)
    .fetch_optional(&mut **transaction)
    .await?;
    Ok(row.map(blob_candidate_from_row))
}

fn blob_candidate_from_row(row: sqlx::sqlite::SqliteRow) -> BlobCandidate {
    BlobCandidate {
        id: row.get("id"),
        storage_path: row.get("storage_path"),
        stored_size: row.get("stored_size"),
        content_blake3: row.get("content_blake3"),
    }
}

async fn ensure_commit_quota(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: Uuid,
    quota_bytes: i64,
    upload_id: Uuid,
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
          AND u.commit_state IN ('receiving', 'commit_started', 'finalizing')
        "#,
    )
    .bind(account_id)
    .fetch_one(&mut **transaction)
    .await?;
    let upload_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM uploads WHERE id = ? AND account_id = ?")
            .bind(upload_id)
            .bind(account_id)
            .fetch_one(&mut **transaction)
            .await?;
    if upload_exists != 1
        || used_bytes.checked_add(reserved_bytes).unwrap_or(i64::MAX) > quota_bytes
    {
        return Err(AppError::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "account storage quota exceeded",
        ));
    }
    Ok(())
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

async fn mark_unknown<T>(
    state: &AppState,
    record: &CommitRecord,
    reason: &str,
) -> Result<T, AppError> {
    mark_unknown_message(&state.pool, record.upload_id, reason).await
}

async fn mark_failed<T>(
    state: &AppState,
    record: &CommitRecord,
    reason: &str,
) -> Result<T, AppError> {
    mark_failed_message(&state.pool, record.upload_id, reason).await
}

async fn mark_failed_message<T>(
    pool: &SqlitePool,
    upload_id: Uuid,
    reason: &str,
) -> Result<T, AppError> {
    sqlx::query(
        "UPDATE uploads SET commit_state = 'failed', commit_error = ?, error = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(reason)
    .bind(reason)
    .bind(upload_id)
    .execute(pool)
    .await?;
    Err(AppError::conflict(format!(
        "upload commit failed validation: {reason}"
    )))
}

async fn mark_unknown_message<T>(
    pool: &SqlitePool,
    upload_id: Uuid,
    reason: &str,
) -> Result<T, AppError> {
    sqlx::query(
        "UPDATE uploads SET commit_state = 'unknown', commit_error = ?, error = ?, updated_at = datetime('now') WHERE id = ? AND commit_state <> 'unknown'",
    )
    .bind(reason)
    .bind(reason)
    .bind(upload_id)
    .execute(pool)
    .await?;
    Err(AppError::conflict(format!(
        "upload commit requires manual reconciliation: {reason}"
    )))
}

#[cfg(test)]
fn stop_at(current: Option<CommitFailpoint>, expected: CommitFailpoint) -> Result<(), AppError> {
    if current == Some(expected) {
        return Err(AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("test crash after {expected:?}"),
        ));
    }
    Ok(())
}

#[cfg(not(test))]
fn stop_at<T>(_current: Option<T>, _expected: T) -> Result<(), AppError> {
    Ok(())
}

pub(crate) async fn reconcile_all(state: &AppState) -> Result<ReconcileReport, AppError> {
    let upload_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"
        SELECT id FROM uploads
        WHERE commit_state IN ('commit_started', 'finalizing', 'committed')
        ORDER BY updated_at, id
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    let mut report = ReconcileReport::default();
    for upload_id in upload_ids {
        let before: String = sqlx::query_scalar("SELECT commit_state FROM uploads WHERE id = ?")
            .bind(upload_id)
            .fetch_one(&state.pool)
            .await?;
        match complete_inner(state, upload_id, None, None).await {
            Ok(_) => {
                if before != COMMITTED {
                    report.recovered = report.recovered.saturating_add(1);
                }
            }
            Err(error) => {
                let after: Option<String> =
                    sqlx::query_scalar("SELECT commit_state FROM uploads WHERE id = ?")
                        .bind(upload_id)
                        .fetch_optional(&state.pool)
                        .await?;
                if after.as_deref() == Some(UNKNOWN) {
                    report.marked_unknown = report.marked_unknown.saturating_add(1);
                } else {
                    report.errors = report.errors.saturating_add(1);
                }
                tracing::warn!(upload_id = %upload_id, ?error, "upload reconciliation did not complete");
            }
        }
    }

    let blob_report = crate::library::reconcile_orphan_blobs(state, None).await?;
    report.orphan_blobs_removed = report
        .orphan_blobs_removed
        .saturating_add(blob_report.removed);
    report.errors = report.errors.saturating_add(blob_report.errors);

    let accounts: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, storage_path FROM accounts ORDER BY id")
            .fetch_all(&state.pool)
            .await?;
    let referenced: HashSet<String> = sqlx::query_scalar(
        "SELECT commit_staged_key FROM uploads WHERE commit_staged_key IS NOT NULL",
    )
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .collect();
    for (account_id, account_path) in accounts {
        match state
            .storage
            .cleanup_orphan_staging(&account_path, &referenced)
        {
            Ok(removed) => {
                report.orphan_stages_removed = report.orphan_stages_removed.saturating_add(removed);
            }
            Err(error) => {
                report.errors = report.errors.saturating_add(1);
                tracing::warn!(account_id = %account_id, ?error, "failed to clean orphan commit staging files");
            }
        }
    }
    Ok(report)
}
