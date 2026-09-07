use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use media_backup_protocol::{
    AlbumRecord, AssetSummary, CreateTagRequest, DuplicateGroup, EmptyRequest, MediaKind,
    ResourceSummary, SetTagAssetsRequest, StorageEncoding, SyncAlbumRequest, SyncEvent, SyncPage,
    TagRecord, TimelinePage, UpdateAssetRequest, API_BASE_PATH,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::{audit, auth::AuthContext, error::AppError, routes::AppState};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineQuery {
    cursor: Option<String>,
    limit: Option<u32>,
    #[serde(default)]
    trashed: bool,
    favorite: Option<bool>,
    archived: Option<bool>,
    album_id: Option<Uuid>,
    tag_id: Option<Uuid>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimelineCursor {
    created_at_ms: i64,
    asset_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncQuery {
    #[serde(default)]
    after: i64,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DuplicateQuery {
    limit: Option<u32>,
}

/// Result of draining blob rows that no resource references. The blob row is
/// the durable deletion intent: it is removed in the same SQLite transaction
/// that unlinks the object, so a filesystem failure never creates an
/// untracked object outside the database.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OrphanBlobReconcileReport {
    pub removed: u64,
    pub errors: u64,
}

pub async fn timeline(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<TimelineQuery>,
) -> Result<Json<TimelinePage>, AppError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 250) as i64;
    let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let before_ms = cursor.as_ref().map(|value| value.created_at_ms);
    let before_id = cursor.as_ref().map(|value| value.asset_id);
    let rows = sqlx::query(
        r#"
        SELECT id, source_created_at_ms
        FROM assets
        WHERE account_id = ?1
          AND ((?2 AND deleted_at IS NOT NULL) OR (NOT ?2 AND deleted_at IS NULL))
          AND (?3 IS NULL OR source_created_at_ms < ?3 OR (source_created_at_ms = ?3 AND id < ?4))
          AND (?5 IS NULL OR favorite = ?5)
          AND (?6 IS NULL OR archived = ?6)
          AND (?7 IS NULL OR EXISTS (
              SELECT 1 FROM album_assets aa WHERE aa.album_id = ?7 AND aa.asset_id = assets.id
          ))
          AND (?8 IS NULL OR EXISTS (
              SELECT 1 FROM tag_assets ta WHERE ta.tag_id = ?8 AND ta.asset_id = assets.id
          ))
        ORDER BY source_created_at_ms DESC, id DESC
        LIMIT ?9
        "#,
    )
    .bind(auth.account_id)
    .bind(query.trashed)
    .bind(before_ms)
    .bind(before_id)
    .bind(query.favorite)
    .bind(query.archived)
    .bind(query.album_id)
    .bind(query.tag_id)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let has_more = rows.len() as i64 > limit;
    let selected = rows.into_iter().take(limit as usize).collect::<Vec<_>>();
    let mut items = Vec::with_capacity(selected.len());
    for row in &selected {
        items.push(load_asset_summary(&state.pool, auth.account_id, row.get("id")).await?);
    }
    let next_cursor = if has_more {
        selected.last().map(|row| {
            encode_cursor(&TimelineCursor {
                created_at_ms: row.get("source_created_at_ms"),
                asset_id: row.get("id"),
            })
        })
    } else {
        None
    };
    Ok(Json(TimelinePage { items, next_cursor }))
}

pub async fn sync_changes(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<SyncQuery>,
) -> Result<Json<SyncPage>, AppError> {
    if query.after < 0 {
        return Err(AppError::bad_request("sync cursor cannot be negative"));
    }
    let limit = query.limit.unwrap_or(250).clamp(1, 1000) as i64;
    let rows = sqlx::query(
        r#"
        SELECT sequence, entity_kind, entity_id, operation,
               (CAST(strftime('%s', changed_at) AS INTEGER) * 1000) AS changed_at_ms
        FROM account_changes
        WHERE account_id = ? AND sequence > ?
        ORDER BY sequence
        LIMIT ?
        "#,
    )
    .bind(auth.account_id)
    .bind(query.after)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let has_more = rows.len() as i64 > limit;
    let events = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| SyncEvent {
            sequence: row.get("sequence"),
            entity_kind: row.get("entity_kind"),
            entity_id: row.get("entity_id"),
            operation: row.get("operation"),
            changed_at_ms: row.get("changed_at_ms"),
        })
        .collect::<Vec<_>>();
    let next_sequence = events
        .last()
        .map(|event| event.sequence)
        .unwrap_or(query.after);
    Ok(Json(SyncPage {
        events,
        next_sequence,
        has_more,
    }))
}

pub async fn update_asset(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(asset_id): Path<Uuid>,
    Json(request): Json<UpdateAssetRequest>,
) -> Result<Json<AssetSummary>, AppError> {
    if request.favorite.is_none() && request.archived.is_none() {
        return Err(AppError::bad_request("no asset state was supplied"));
    }
    let mut transaction = state.pool.begin().await?;
    let updated: Option<Uuid> = sqlx::query_scalar(
        r#"
        UPDATE assets SET
            favorite = COALESCE(?1, favorite),
            archived = COALESCE(?2, archived),
            updated_at = datetime('now')
        WHERE id = ?3 AND account_id = ?4
        RETURNING id
        "#,
    )
    .bind(request.favorite)
    .bind(request.archived)
    .bind(asset_id)
    .bind(auth.account_id)
    .fetch_optional(&mut *transaction)
    .await?;
    updated.ok_or_else(|| AppError::not_found("asset not found"))?;
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
        "asset.update",
        Some("asset"),
        Some(asset_id),
    )
    .await?;
    transaction.commit().await?;
    Ok(Json(
        load_asset_summary(&state.pool, auth.account_id, asset_id).await?,
    ))
}

pub async fn get_asset(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(asset_id): Path<Uuid>,
) -> Result<Json<AssetSummary>, AppError> {
    Ok(Json(
        load_asset_summary(&state.pool, auth.account_id, asset_id).await?,
    ))
}

pub async fn trash_asset(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(asset_id): Path<Uuid>,
    Json(_request): Json<EmptyRequest>,
) -> Result<StatusCode, AppError> {
    set_trashed(&state, &auth, asset_id, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn restore_asset(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(asset_id): Path<Uuid>,
    Json(_request): Json<EmptyRequest>,
) -> Result<StatusCode, AppError> {
    set_trashed(&state, &auth, asset_id, false).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_trashed(
    state: &AppState,
    auth: &AuthContext,
    asset_id: Uuid,
    trashed: bool,
) -> Result<(), AppError> {
    let mut transaction = state.pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE assets \
         SET deleted_at = CASE WHEN ?1 THEN datetime('now') ELSE NULL END, \
             updated_at = datetime('now') \
         WHERE id = ?2 AND account_id = ?3",
    )
    .bind(trashed)
    .bind(asset_id)
    .bind(auth.account_id)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::not_found("asset not found"));
    }
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
        auth,
        if trashed {
            "asset.trash"
        } else {
            "asset.restore"
        },
        Some("asset"),
        Some(asset_id),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn delete_asset_permanently(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(asset_id): Path<Uuid>,
    Json(_request): Json<EmptyRequest>,
) -> Result<StatusCode, AppError> {
    let mut transaction = state.pool.begin().await?;
    let blob_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT r.blob_id FROM resources r
        JOIN assets a ON a.id = r.asset_id
        WHERE a.id = ? AND a.account_id = ? AND a.deleted_at IS NOT NULL
        "#,
    )
    .bind(asset_id)
    .bind(auth.account_id)
    .fetch_all(&mut *transaction)
    .await?;
    if blob_ids.is_empty() {
        return Err(AppError::not_found("trashed asset not found"));
    }
    sqlx::query("DELETE FROM assets WHERE id = ? AND account_id = ? AND deleted_at IS NOT NULL")
        .bind(asset_id)
        .bind(auth.account_id)
        .execute(&mut *transaction)
        .await?;
    audit::record_change(
        &mut transaction,
        auth.account_id,
        "asset",
        asset_id,
        "delete",
    )
    .await?;
    audit::record_in_transaction(
        &mut transaction,
        &auth,
        "asset.delete",
        Some("asset"),
        Some(asset_id),
    )
    .await?;
    transaction.commit().await?;

    // The asset deletion is already durable. Physical reclamation is a
    // separate, retryable close-out step; never return 500 after committing
    // the user-visible delete because that would invite an unsafe replay.
    match reconcile_orphan_blobs(&state, Some(auth.account_id)).await {
        Ok(report) if report.errors == 0 => Ok(StatusCode::NO_CONTENT),
        Ok(report) => {
            tracing::warn!(
                account_id = %auth.account_id,
                pending = report.errors,
                "asset metadata was deleted; blob reclamation remains queued"
            );
            Ok(StatusCode::ACCEPTED)
        }
        Err(error) => {
            tracing::error!(
                account_id = %auth.account_id,
                ?error,
                "asset metadata was deleted; blob reconciliation could not be scanned"
            );
            Ok(StatusCode::ACCEPTED)
        }
    }
}

/// Remove unreferenced blob objects without ever losing their durable lookup
/// row first. `DELETE ... RETURNING` obtains SQLite's write lock and makes the
/// row unavailable to concurrent deduplication while the rooted unlink runs.
/// On unlink failure the transaction rolls back, leaving both row and file for
/// the next startup/periodic/manual reconciliation pass. If commit fails after
/// unlink, the restored row points to a missing unreferenced file and the next
/// pass safely finishes the metadata deletion.
pub(crate) async fn reconcile_orphan_blobs(
    state: &AppState,
    account_filter: Option<Uuid>,
) -> Result<OrphanBlobReconcileReport, AppError> {
    let blob_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"
        SELECT b.id FROM blobs b
        WHERE (?1 IS NULL OR b.account_id = ?1)
          AND NOT EXISTS (SELECT 1 FROM resources r WHERE r.blob_id = b.id)
          AND NOT EXISTS (SELECT 1 FROM uploads u WHERE u.commit_blob_id = b.id)
        ORDER BY b.created_at, b.id
        "#,
    )
    .bind(account_filter)
    .fetch_all(&state.pool)
    .await?;
    let mut report = OrphanBlobReconcileReport::default();
    for blob_id in blob_ids {
        let mut transaction = state.pool.begin().await?;
        let orphan: Option<(Uuid, String)> = sqlx::query_as(
            r#"
            DELETE FROM blobs
            WHERE id = ?
              AND NOT EXISTS (SELECT 1 FROM resources r WHERE r.blob_id = blobs.id)
              AND NOT EXISTS (SELECT 1 FROM uploads u WHERE u.commit_blob_id = blobs.id)
            RETURNING account_id, storage_path
            "#,
        )
        .bind(blob_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((account_id, object_path)) = orphan else {
            transaction.rollback().await?;
            continue;
        };
        let account_path: String =
            sqlx::query_scalar("SELECT storage_path FROM accounts WHERE id = ?")
                .bind(account_id)
                .fetch_one(&mut *transaction)
                .await?;
        match state.storage.remove_blob(&account_path, &object_path).await {
            Ok(()) => {
                transaction.commit().await?;
                report.removed = report.removed.saturating_add(1);
            }
            Err(error) => {
                transaction.rollback().await?;
                report.errors = report.errors.saturating_add(1);
                tracing::warn!(
                    blob_id = %blob_id,
                    account_id = %account_id,
                    ?error,
                    "orphan blob reclamation will be retried"
                );
            }
        }
    }
    Ok(report)
}

pub async fn list_albums(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<Vec<AlbumRecord>>, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT a.id, a.source_album_id, a.name,
               COUNT(aa.asset_id) AS asset_count,
               (CAST(strftime('%s', a.updated_at) AS INTEGER) * 1000) AS updated_at_ms
        FROM albums a LEFT JOIN album_assets aa ON aa.album_id = a.id
        WHERE a.account_id = ?
        GROUP BY a.id ORDER BY a.updated_at DESC
        "#,
    )
    .bind(auth.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.into_iter().map(album_from_row).collect()))
}

pub async fn sync_album(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<SyncAlbumRequest>,
) -> Result<Json<AlbumRecord>, AppError> {
    validate_name(&request.source_album_id, &request.name)?;
    if request.source_asset_ids.len() > 10_000 {
        return Err(AppError::bad_request(
            "album contains too many assets in one request",
        ));
    }
    let mut transaction = state.pool.begin().await?;
    let album_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO albums(
            id, account_id, device_id, source_album_id, name, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, datetime('now'), datetime('now'))
        ON CONFLICT(account_id, device_id, source_album_id) DO UPDATE SET
            name = excluded.name,
            updated_at = datetime('now')
        RETURNING id
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(auth.account_id)
    .bind(auth.device_id)
    .bind(&request.source_album_id)
    .bind(&request.name)
    .fetch_one(&mut *transaction)
    .await?;
    if request.replace_members {
        sqlx::query("DELETE FROM album_assets WHERE album_id = ?")
            .bind(album_id)
            .execute(&mut *transaction)
            .await?;
    }
    for source_asset_id in &request.source_asset_ids {
        let asset_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM assets \
             WHERE account_id = ? AND device_id = ? AND source_asset_id = ?",
        )
        .bind(auth.account_id)
        .bind(auth.device_id)
        .bind(source_asset_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(asset_id) = asset_id {
            sqlx::query(
                "INSERT INTO album_assets(album_id, asset_id, added_at) \
                 VALUES(?, ?, datetime('now')) \
                 ON CONFLICT(album_id, asset_id) DO NOTHING",
            )
            .bind(album_id)
            .bind(asset_id)
            .execute(&mut *transaction)
            .await?;
        }
    }
    audit::record_change(
        &mut transaction,
        auth.account_id,
        "album",
        album_id,
        "upsert",
    )
    .await?;
    audit::record_in_transaction(
        &mut transaction,
        &auth,
        "album.sync",
        Some("album"),
        Some(album_id),
    )
    .await?;
    transaction.commit().await?;
    let row = sqlx::query(
        r#"
        SELECT a.id, a.source_album_id, a.name,
               COUNT(aa.asset_id) AS asset_count,
               (CAST(strftime('%s', a.updated_at) AS INTEGER) * 1000) AS updated_at_ms
        FROM albums a LEFT JOIN album_assets aa ON aa.album_id = a.id
        WHERE a.id = ? AND a.account_id = ? GROUP BY a.id
        "#,
    )
    .bind(album_id)
    .bind(auth.account_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(album_from_row(row)))
}

pub async fn list_tags(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<Vec<TagRecord>>, AppError> {
    let rows = sqlx::query(
        r#"
        SELECT t.id, t.name, COUNT(ta.asset_id) AS asset_count,
               (CAST(strftime('%s', t.updated_at) AS INTEGER) * 1000) AS updated_at_ms
        FROM tags t LEFT JOIN tag_assets ta ON ta.tag_id = t.id
        WHERE t.account_id = ? GROUP BY t.id ORDER BY t.updated_at DESC
        "#,
    )
    .bind(auth.account_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(rows.into_iter().map(tag_from_row).collect()))
}

pub async fn create_tag(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(request): Json<CreateTagRequest>,
) -> Result<(StatusCode, Json<TagRecord>), AppError> {
    validate_name("tag", &request.name)?;
    let mut transaction = state.pool.begin().await?;
    let row = sqlx::query(
        r#"
        INSERT INTO tags(id, account_id, name, created_at, updated_at)
        VALUES (?, ?, ?, datetime('now'), datetime('now'))
        RETURNING id, name, 0 AS asset_count,
                  (CAST(strftime('%s', updated_at) AS INTEGER) * 1000) AS updated_at_ms
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(auth.account_id)
    .bind(request.name)
    .fetch_one(&mut *transaction)
    .await?;
    let tag = tag_from_row(row);
    audit::record_change(
        &mut transaction,
        auth.account_id,
        "tag",
        tag.tag_id,
        "upsert",
    )
    .await?;
    audit::record_in_transaction(
        &mut transaction,
        &auth,
        "tag.create",
        Some("tag"),
        Some(tag.tag_id),
    )
    .await?;
    transaction.commit().await?;
    Ok((StatusCode::CREATED, Json(tag)))
}

pub async fn set_tag_assets(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(tag_id): Path<Uuid>,
    Json(request): Json<SetTagAssetsRequest>,
) -> Result<StatusCode, AppError> {
    if request.asset_ids.len() > 10_000 {
        return Err(AppError::bad_request("too many tag assets in one request"));
    }
    let mut transaction = state.pool.begin().await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tags WHERE id = ? AND account_id = ?)")
            .bind(tag_id)
            .bind(auth.account_id)
            .fetch_one(&mut *transaction)
            .await?;
    if !exists {
        return Err(AppError::not_found("tag not found"));
    }
    sqlx::query("DELETE FROM tag_assets WHERE tag_id = ?")
        .bind(tag_id)
        .execute(&mut *transaction)
        .await?;
    for asset_id in &request.asset_ids {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM assets WHERE account_id = ? AND id = ?)",
        )
        .bind(auth.account_id)
        .bind(asset_id)
        .fetch_one(&mut *transaction)
        .await?;
        if exists {
            sqlx::query(
                "INSERT INTO tag_assets(tag_id, asset_id, added_at) \
                 VALUES(?, ?, datetime('now')) \
                 ON CONFLICT(tag_id, asset_id) DO NOTHING",
            )
            .bind(tag_id)
            .bind(asset_id)
            .execute(&mut *transaction)
            .await?;
        }
    }
    audit::record_change(&mut transaction, auth.account_id, "tag", tag_id, "upsert").await?;
    audit::record_in_transaction(
        &mut transaction,
        &auth,
        "tag.assets.set",
        Some("tag"),
        Some(tag_id),
    )
    .await?;
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn add_tag_asset(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((tag_id, asset_id)): Path<(Uuid, Uuid)>,
    Json(_request): Json<EmptyRequest>,
) -> Result<StatusCode, AppError> {
    change_tag_asset(&state, &auth, tag_id, asset_id, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_tag_asset(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((tag_id, asset_id)): Path<(Uuid, Uuid)>,
    Json(_request): Json<EmptyRequest>,
) -> Result<StatusCode, AppError> {
    change_tag_asset(&state, &auth, tag_id, asset_id, false).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn change_tag_asset(
    state: &AppState,
    auth: &AuthContext,
    tag_id: Uuid,
    asset_id: Uuid,
    add: bool,
) -> Result<(), AppError> {
    let mut transaction = state.pool.begin().await?;
    let valid: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM tags t, assets a
            WHERE t.id = ?1 AND t.account_id = ?2
              AND a.id = ?3 AND a.account_id = ?2
        )
        "#,
    )
    .bind(tag_id)
    .bind(auth.account_id)
    .bind(asset_id)
    .fetch_one(&mut *transaction)
    .await?;
    if !valid {
        return Err(AppError::not_found("tag or asset not found"));
    }
    if add {
        sqlx::query(
            "INSERT INTO tag_assets(tag_id, asset_id, added_at) \
             VALUES (?, ?, datetime('now')) ON CONFLICT DO NOTHING",
        )
        .bind(tag_id)
        .bind(asset_id)
        .execute(&mut *transaction)
        .await?;
    } else {
        sqlx::query("DELETE FROM tag_assets WHERE tag_id = ? AND asset_id = ?")
            .bind(tag_id)
            .bind(asset_id)
            .execute(&mut *transaction)
            .await?;
    }
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
        auth,
        if add {
            "tag.asset.add"
        } else {
            "tag.asset.remove"
        },
        Some("asset"),
        Some(asset_id),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn duplicate_groups(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<DuplicateQuery>,
) -> Result<Json<Vec<DuplicateGroup>>, AppError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200) as i64;
    let rows = sqlx::query(
        r#"
        SELECT b.id AS blob_id, b.content_blake3, b.plaintext_size,
               COUNT(DISTINCT a.id) AS asset_count
        FROM blobs b
        JOIN resources r ON r.blob_id = b.id
        JOIN assets a ON a.id = r.asset_id
        WHERE b.account_id = ? AND a.deleted_at IS NULL AND r.role = 'primary'
        GROUP BY b.id, b.content_blake3, b.plaintext_size
        HAVING COUNT(DISTINCT a.id) > 1
        ORDER BY COUNT(DISTINCT a.id) DESC, b.created_at
        LIMIT ?
        "#,
    )
    .bind(auth.account_id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    let mut groups = Vec::with_capacity(rows.len());
    for row in rows {
        let blob_id: Uuid = row.get("blob_id");
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT a.id FROM assets a \
             JOIN resources r ON r.asset_id = a.id \
             WHERE r.blob_id = ? AND a.deleted_at IS NULL AND r.role = 'primary' \
             ORDER BY a.created_at",
        )
        .bind(blob_id)
        .fetch_all(&state.pool)
        .await?;
        let mut assets = Vec::with_capacity(ids.len());
        for asset_id in ids {
            assets.push(load_asset_summary(&state.pool, auth.account_id, asset_id).await?);
        }
        groups.push(DuplicateGroup {
            content_blake3: row.get("content_blake3"),
            content_size: row.get::<i64, _>("plaintext_size") as u64,
            assets,
        });
    }
    Ok(Json(groups))
}

pub(crate) async fn load_asset_summary(
    pool: &SqlitePool,
    account_id: Uuid,
    asset_id: Uuid,
) -> Result<AssetSummary, AppError> {
    let row = sqlx::query(
        r#"
        SELECT id, source_asset_id, media_kind, source_created_at_ms, favorite, archived,
               CASE WHEN deleted_at IS NULL THEN NULL
                    ELSE (CAST(strftime('%s', deleted_at) AS INTEGER) * 1000) END AS trashed_at_ms
        FROM assets WHERE id = ? AND account_id = ?
        "#,
    )
    .bind(asset_id)
    .bind(account_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::not_found("asset not found"))?;
    let resource_rows = sqlx::query(
        r#"
        SELECT r.id, r.role, r.filename, r.mime_type, r.metadata, b.plaintext_size
        FROM resources r JOIN blobs b ON b.id = r.blob_id
        WHERE r.asset_id = ? ORDER BY r.created_at, r.id
        "#,
    )
    .bind(asset_id)
    .fetch_all(pool)
    .await?;
    let tag_names: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT t.name FROM tags t JOIN tag_assets ta ON ta.tag_id = t.id
        WHERE ta.asset_id = ? ORDER BY lower(t.name), t.id
        "#,
    )
    .bind(asset_id)
    .fetch_all(pool)
    .await?;
    let resources = resource_rows
        .into_iter()
        .map(|resource| {
            let resource_id: Uuid = resource.get("id");
            ResourceSummary {
                resource_id,
                role: resource.get("role"),
                filename: resource.get("filename"),
                mime_type: resource.get("mime_type"),
                content_size: resource.get::<i64, _>("plaintext_size") as u64,
                storage_encoding: StorageEncoding::PlainV1,
                metadata: resource.get("metadata"),
                manifest_path: format!("{API_BASE_PATH}/resources/{resource_id}"),
                content_path: format!("{API_BASE_PATH}/resources/{resource_id}/content"),
            }
        })
        .collect();
    Ok(AssetSummary {
        asset_id,
        source_asset_id: row.get("source_asset_id"),
        media_kind: parse_media_kind(row.get::<String, _>("media_kind")),
        source_created_at_ms: row.get("source_created_at_ms"),
        favorite: row.get("favorite"),
        archived: row.get("archived"),
        trashed_at_ms: row.get("trashed_at_ms"),
        tag_names,
        resources,
    })
}

fn parse_media_kind(value: String) -> MediaKind {
    match value.as_str() {
        "photo" => MediaKind::Photo,
        "video" => MediaKind::Video,
        _ => MediaKind::Other,
    }
}

fn album_from_row(row: sqlx::sqlite::SqliteRow) -> AlbumRecord {
    AlbumRecord {
        album_id: row.get("id"),
        source_album_id: row.get("source_album_id"),
        name: row.get("name"),
        asset_count: row.get::<i64, _>("asset_count") as u64,
        updated_at_ms: row.get("updated_at_ms"),
    }
}

fn tag_from_row(row: sqlx::sqlite::SqliteRow) -> TagRecord {
    TagRecord {
        tag_id: row.get("id"),
        name: row.get("name"),
        asset_count: row.get::<i64, _>("asset_count") as u64,
        updated_at_ms: row.get("updated_at_ms"),
    }
}

fn validate_name(source_id: &str, name: &str) -> Result<(), AppError> {
    if source_id.trim().is_empty()
        || source_id.len() > 1024
        || name.trim().is_empty()
        || name.len() > 512
    {
        return Err(AppError::bad_request("invalid name"));
    }
    Ok(())
}

fn encode_cursor(cursor: &TimelineCursor) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).expect("timeline cursor is serializable"))
}

fn decode_cursor(value: &str) -> Result<TimelineCursor, AppError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| AppError::bad_request("invalid timeline cursor"))?;
    serde_json::from_slice(&bytes).map_err(|_| AppError::bad_request("invalid timeline cursor"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_cursor_round_trips() {
        let value = TimelineCursor {
            created_at_ms: 1_725_000_000_000,
            asset_id: Uuid::new_v4(),
        };
        assert_eq!(
            decode_cursor(&encode_cursor(&value)).unwrap().asset_id,
            value.asset_id
        );
    }

    #[test]
    fn timeline_cursor_rejects_invalid_input() {
        assert!(decode_cursor("not-a-cursor").is_err());
    }
}
