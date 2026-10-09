use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};
use futures_util::TryStreamExt;
use serde::Serialize;
use sqlx::{ConnectOptions, Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use uuid::Uuid;
use xszs_protocol::CreateUploadRequest;

use crate::{
    config::Config,
    database,
    rooted_fs::{RootEntryKind, RootedFs},
    runtime_lock::RuntimeLock,
};

#[derive(Debug, Serialize)]
pub(crate) struct DoctorSummary {
    status: &'static str,
    files: usize,
    blobs: u64,
    uploads: u64,
    schema_revision: i64,
}

#[derive(Debug)]
struct FileFact {
    size: u64,
    blake3: String,
}

type FileIndex = BTreeMap<String, FileFact>;

// Diagnostic evidence must be complete within these budgets or fail explicitly.
// None of these limits silently skip records, prune data, or change runtime APIs.
const MAX_METADATA_ENTRIES: usize = 100_000;
const MAX_INDEX_BYTES: usize = 64 * 1024 * 1024;
const MAX_TREE_DEPTH: usize = 64;
const MAX_HASH_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
const HASH_TIMEOUT: Duration = Duration::from_secs(600);

struct TreeBudget {
    entries: usize,
    index_bytes: usize,
    hashed_bytes: u64,
    deadline: Instant,
}

pub(crate) fn run(config: &Config) -> anyhow::Result<DoctorSummary> {
    let lock = RuntimeLock::acquire(&config.database_url, &config.data_dir)?;
    let database = database::database_path(&config.database_url)?;
    database::validate_current_database(&database)?;
    database::integrity_and_foreign_key_check(&database)?;
    database_write_rollback_probe(&lock)?;
    storage_write_cleanup_probe(&config.data_dir)?;
    let files = index_data_tree(&config.data_dir)?;
    let (blobs, uploads) = validate_database_files(&database, &files)?;
    lock.verify_original_paths()?;
    Ok(DoctorSummary {
        status: "ok",
        files: files.len(),
        blobs,
        uploads,
        schema_revision: database::CURRENT_SCHEMA_REVISION,
    })
}

fn validate_database_files(database: &Path, files: &FileIndex) -> anyhow::Result<(u64, u64)> {
    // Recover and read only a held private generation. SQLx deliberately has no
    // SQLITE_OPEN_NOFOLLOW option; the private snapshot owns its guarded path.
    let snapshot = xcss_server_cli::ValidationSnapshot::capture(database)?;
    xcss_sqlite::block_on_sqlite_connection(async {
        let mut connection = SqliteConnectOptions::new()
            .filename(snapshot.database_path())
            .create_if_missing(false)
            .busy_timeout(Duration::from_secs(2))
            .connect()
            .await?;
        let result = async {
            database::harden_connection(&mut connection).await?;
            let deadline = Instant::now() + Duration::from_secs(3);
            connection
                .lock_handle()
                .await?
                .set_progress_handler(1000, move || Instant::now() < deadline);
            sqlx::query("PRAGMA query_only=ON")
                .execute(&mut connection)
                .await?;
            validate_account_storage_paths(&mut connection).await?;
            validate_upload_manifests(&mut connection, files).await?;
            validate_database_file_rows(&mut connection, files).await
        }
        .await;
        // Join the SQLx worker on success and on failure before snapshot cleanup.
        let closed = connection.close().await;
        let counts = result?;
        closed?;
        Ok(counts)
    })
}

async fn validate_database_file_rows(
    connection: &mut SqliteConnection,
    files: &FileIndex,
) -> anyhow::Result<(u64, u64)> {
    let unknown_uploads: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM uploads WHERE commit_state = 'unknown'")
            .fetch_one(&mut *connection)
            .await?;
    ensure!(
        unknown_uploads == 0,
        "an upload has unknown commit state; reconcile it before continuing"
    );
    let broken_committed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM uploads u
         LEFT JOIN accounts a ON a.id = u.account_id
         LEFT JOIN blobs b ON b.id = u.commit_blob_id
         LEFT JOIN resources r ON r.id = u.commit_resource_id
         WHERE u.commit_state = 'committed' AND (
             u.state <> 'complete' OR u.completed_at IS NULL OR
             u.commit_blob_id IS NULL OR u.commit_resource_id IS NULL OR
             u.commit_final_key IS NULL OR u.commit_account_path IS NULL OR
             u.commit_expected_size IS NULL OR u.commit_expected_blake3 IS NULL OR
             b.id IS NULL OR b.account_id <> u.account_id OR
             b.storage_path <> u.commit_final_key OR b.stored_size <> u.commit_expected_size OR
             b.content_blake3 IS NULL OR b.content_blake3 <> u.commit_expected_blake3 OR
             b.storage_encoding <> 'plain-v1' OR
             r.id IS NULL OR r.blob_id <> b.id OR r.asset_id <> u.asset_id OR
             a.id IS NULL OR a.storage_path <> u.commit_account_path
         )",
    )
    .fetch_one(&mut *connection)
    .await?;
    ensure!(
        broken_committed == 0,
        "a committed upload has incomplete metadata"
    );
    let orphan_blobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM blobs b \
         WHERE NOT EXISTS (SELECT 1 FROM resources r WHERE r.blob_id = b.id) \
           AND NOT EXISTS (SELECT 1 FROM uploads u WHERE u.commit_blob_id = b.id)",
    )
    .fetch_one(&mut *connection)
    .await?;
    ensure!(
        orphan_blobs == 0,
        "an unreferenced blob is awaiting reconciliation; run `reconcile scan`"
    );

    let mut rows = sqlx::query_as::<_, (String, String, i64, String)>(
        "SELECT a.storage_path, b.storage_path, b.stored_size, b.content_blake3
         FROM blobs b JOIN accounts a ON a.id = b.account_id ORDER BY b.id",
    )
    .fetch(&mut *connection);
    let mut row_count = 0_usize;
    while let Some(row) = rows.try_next().await? {
        row_count += 1;
        ensure!(
            row_count <= MAX_METADATA_ENTRIES,
            "diagnostic metadata row budget exceeded"
        );
        let (account_path, storage_path, stored_size, content_hash) = row;
        ensure_scoped_key(&account_path, &storage_path)?;
        let entry = files
            .get(&storage_path)
            .context("a database blob is missing from DATA_DIR")?;
        ensure!(
            entry.size == u64::try_from(stored_size)?,
            "a database blob size does not match DATA_DIR"
        );
        ensure!(
            valid_blake3(&content_hash) && entry.blake3 == content_hash.to_ascii_lowercase(),
            "a database blob hash does not match DATA_DIR"
        );
    }

    drop(rows);
    validate_active_commits(connection, files).await?;
    let blobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
        .fetch_one(&mut *connection)
        .await?;
    let uploads: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM uploads")
        .fetch_one(&mut *connection)
        .await?;
    Ok((u64::try_from(blobs)?, u64::try_from(uploads)?))
}

async fn validate_active_commits(
    connection: &mut SqliteConnection,
    files: &FileIndex,
) -> anyhow::Result<()> {
    let mut rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
        ),
    >(
        "SELECT u.id, u.commit_state, u.commit_staged_key, u.commit_final_key,
                u.commit_expected_size, u.commit_expected_blake3, u.commit_account_path,
                a.storage_path, u.commit_blob_id
         FROM uploads u JOIN accounts a ON a.id = u.account_id
         WHERE u.commit_state IN ('commit_started','finalizing') ORDER BY u.id",
    )
    .fetch(&mut *connection);
    let mut row_count = 0_usize;
    while let Some(row) = rows.try_next().await? {
        row_count += 1;
        ensure!(
            row_count <= MAX_METADATA_ENTRIES,
            "diagnostic metadata row budget exceeded"
        );
        let (id, state, staged, final_key, size, hash, commit_account, account, blob_id) = row;
        let upload_id = Uuid::parse_str(&id).context("active upload ID is invalid")?;
        ensure!(
            commit_account.as_deref() == Some(account.as_str()),
            "upload commit account path conflicts with its account"
        );
        ensure!(
            blob_id
                .as_deref()
                .is_some_and(|value| Uuid::parse_str(value).is_ok()),
            "upload commit blob ID is missing or invalid"
        );
        let size = u64::try_from(size.context("upload commit size is missing")?)?;
        let hash = hash.context("upload commit hash is missing")?;
        ensure!(valid_blake3(&hash), "upload commit hash is invalid");
        let final_key = final_key.context("upload commit final key is missing")?;
        ensure_scoped_key(&account, &final_key)?;
        if state == "commit_started" {
            ensure!(
                staged.is_some(),
                "commit-started upload has no staged object key"
            );
        }
        if let Some(path) = staged.as_deref() {
            ensure_commit_stage_key(&account, upload_id, path)?;
        }
        let stage_matches = staged
            .as_deref()
            .and_then(|path| files.get(path))
            .is_some_and(|entry| entry.size == size && entry.blake3 == hash);
        let final_matches = files
            .get(&final_key)
            .is_some_and(|entry| entry.size == size && entry.blake3 == hash);
        if let Some(path) = staged.as_deref()
            && let Some(entry) = files.get(path)
        {
            ensure!(
                entry.size == size && entry.blake3 == hash,
                "staged upload object is corrupt"
            );
        }
        if let Some(entry) = files.get(&final_key) {
            ensure!(
                entry.size == size && entry.blake3 == hash,
                "finalizing upload object is corrupt"
            );
        }
        if state == "finalizing" {
            ensure!(
                stage_matches || final_matches,
                "finalizing upload has no recoverable staged or final object"
            );
        }
    }
    Ok(())
}

async fn validate_account_storage_paths(connection: &mut SqliteConnection) -> anyhow::Result<()> {
    let mut rows = sqlx::query_scalar::<_, String>("SELECT storage_path FROM accounts ORDER BY id")
        .fetch(&mut *connection);
    let mut paths = BTreeSet::new();
    let mut bytes = 0_usize;
    while let Some(path) = rows.try_next().await? {
        validate_storage_key(&path)?;
        ensure!(
            Path::new(&path).components().next() != Some(Component::Normal("uploads".as_ref())),
            "an account storage path uses the reserved uploads directory"
        );
        bytes = bytes
            .checked_add(path.len())
            .context("diagnostic path budget overflow")?;
        ensure!(
            paths.len() < MAX_METADATA_ENTRIES && bytes <= MAX_INDEX_BYTES,
            "diagnostic account path budget exceeded"
        );
        paths.insert(path);
    }
    for path in &paths {
        for ancestor in Path::new(path).ancestors().skip(1) {
            ensure!(
                !ancestor.to_str().is_some_and(|value| paths.contains(value)),
                "account storage paths overlap"
            );
        }
    }
    Ok(())
}

async fn validate_upload_manifests(
    connection: &mut SqliteConnection,
    files: &FileIndex,
) -> anyhow::Result<()> {
    let mut expected_parts = BTreeMap::new();
    let mut manifest_bytes = 0_usize;
    let mut rows = sqlx::query_as::<_, (String, String, String, String, String, String)>(
        "SELECT u.id, u.request, u.source_resource_id, u.content_blake3,
                a.source_asset_id, a.media_kind
         FROM uploads u JOIN assets a ON a.id = u.asset_id ORDER BY u.id",
    )
    .fetch(&mut *connection);
    let mut row_count = 0_usize;
    while let Some(row) = rows.try_next().await? {
        row_count += 1;
        ensure!(
            row_count <= MAX_METADATA_ENTRIES,
            "diagnostic metadata row budget exceeded"
        );
        let (upload_id, request, source_resource_id, content_blake3, source_asset_id, media_kind) =
            row;
        ensure!(Uuid::parse_str(&upload_id).is_ok(), "upload ID is invalid");
        let request: CreateUploadRequest =
            serde_json::from_str(&request).context("an upload request is invalid")?;
        ensure!(
            !request.source_asset_id.is_empty()
                && !request.source_resource_id.is_empty()
                && !request.filename.is_empty()
                && !request.mime_type.is_empty()
                && valid_blake3(&request.content_blake3)
                && !request.parts.is_empty()
                && request.source_asset_id == source_asset_id
                && request.source_resource_id == source_resource_id
                && request.content_blake3 == content_blake3
                && request.media_kind.as_str() == media_kind,
            "an upload request conflicts with its database metadata"
        );
        let mut total = 0_u64;
        for (position, part) in request.parts.into_iter().enumerate() {
            ensure!(
                usize::try_from(part.index)? == position
                    && (part.size != 0 || request.content_size == 0)
                    && valid_blake3(&part.blake3),
                "an upload part manifest is invalid"
            );
            total = total
                .checked_add(part.size)
                .context("upload part size overflow")?;
            manifest_bytes = manifest_bytes
                .checked_add(upload_id.len() + part.blake3.len())
                .context("diagnostic manifest budget overflow")?;
            ensure!(
                expected_parts.len() < MAX_METADATA_ENTRIES && manifest_bytes <= MAX_INDEX_BYTES,
                "diagnostic upload manifest budget exceeded"
            );
            ensure!(
                expected_parts
                    .insert(
                        (upload_id.clone(), i64::from(part.index)),
                        (part.size, part.blake3)
                    )
                    .is_none(),
                "an upload contains duplicate part indexes"
            );
        }
        ensure!(
            total == request.content_size,
            "upload parts do not match the requested content size"
        );
    }

    drop(rows);
    let mut durable_parts = BTreeSet::new();
    let mut rows = sqlx::query_as::<_, (String, i64, i64, String, Option<i64>, Option<String>)>(
        "SELECT upload_id, part_index, expected_size, expected_blake3, received_size, received_at
         FROM upload_parts ORDER BY upload_id, part_index",
    )
    .fetch(&mut *connection);
    let mut row_count = 0_usize;
    while let Some(row) = rows.try_next().await? {
        row_count += 1;
        ensure!(
            row_count <= MAX_METADATA_ENTRIES,
            "diagnostic metadata row budget exceeded"
        );
        let (upload_id, index, size, hash, received_size, received_at) = row;
        let expected = expected_parts
            .remove(&(upload_id.clone(), index))
            .context("an upload part is absent from its persisted request")?;
        let size = u64::try_from(size)?;
        ensure!(
            expected == (size, hash.clone()),
            "an upload part conflicts with its persisted request"
        );
        if received_at.is_some() {
            ensure!(
                received_size.and_then(|value| u64::try_from(value).ok()) == Some(size),
                "a received upload part has invalid durable size metadata"
            );
            let index = u32::try_from(index)?;
            let path = format!("uploads/{upload_id}/{index:08}.part");
            validate_expected_file(files, &path, size, &hash, "upload part")?;
            durable_parts.insert(path);
        } else {
            ensure!(
                received_size.is_none(),
                "an unreceived upload part has durable size metadata"
            );
        }
    }
    ensure!(
        expected_parts.is_empty(),
        "an upload request part is absent from SQLite"
    );
    let disk_parts: BTreeSet<String> = files
        .keys()
        .filter(|path| is_upload_part_path(path))
        .cloned()
        .collect();
    ensure!(
        disk_parts == durable_parts,
        "DATA_DIR upload parts do not match durable SQLite state"
    );
    Ok(())
}

fn index_data_tree(root: &Path) -> anyhow::Result<FileIndex> {
    fn visit(
        rooted: &RootedFs,
        directory: &Path,
        index: &mut FileIndex,
        budget: &mut TreeBudget,
    ) -> anyhow::Result<()> {
        ensure!(
            directory.components().count() <= MAX_TREE_DEPTH,
            "diagnostic directory depth budget exceeded"
        );
        ensure!(
            Instant::now() < budget.deadline,
            "diagnostic file hashing deadline exceeded"
        );
        let mut entries = rooted.list_entries_bounded(
            directory,
            MAX_METADATA_ENTRIES.saturating_sub(budget.entries),
        )?;
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        budget.entries += entries.len();
        for (name, kind) in entries {
            let name = name
                .to_str()
                .context("DATA_DIR paths must be valid UTF-8")?;
            let path = if directory.as_os_str().is_empty() {
                PathBuf::from(name)
            } else {
                directory.join(name)
            };
            validate_relative_path(&path)?;
            let path_text = path.to_str().context("DATA_DIR path is not UTF-8")?;
            budget.index_bytes = budget
                .index_bytes
                .checked_add(path_text.len())
                .context("diagnostic file index budget overflow")?;
            ensure!(
                budget.index_bytes <= MAX_INDEX_BYTES,
                "diagnostic file index budget exceeded"
            );
            match kind {
                RootEntryKind::Directory => visit(rooted, &path, index, budget)?,
                RootEntryKind::RegularFile => {
                    let mut file = rooted.open_read_std(&path)?;
                    let mut hasher = blake3::Hasher::new();
                    let mut size = 0_u64;
                    let mut buffer = vec![0_u8; 1024 * 1024];
                    loop {
                        ensure!(
                            Instant::now() < budget.deadline,
                            "diagnostic file hashing deadline exceeded"
                        );
                        let read = file.read(&mut buffer)?;
                        if read == 0 {
                            break;
                        }
                        size = size
                            .checked_add(u64::try_from(read)?)
                            .context("file is too large")?;
                        budget.hashed_bytes = budget
                            .hashed_bytes
                            .checked_add(u64::try_from(read)?)
                            .context("diagnostic hashing budget overflow")?;
                        ensure!(
                            budget.hashed_bytes <= MAX_HASH_BYTES,
                            "diagnostic file hashing byte budget exceeded"
                        );
                        hasher.update(&buffer[..read]);
                    }
                    index.insert(
                        path.to_str()
                            .context("DATA_DIR path is not UTF-8")?
                            .to_owned(),
                        FileFact {
                            size,
                            blake3: hasher.finalize().to_hex().to_string(),
                        },
                    );
                }
            }
        }
        Ok(())
    }

    let rooted = RootedFs::new(root).context("open DATA_DIR without following links")?;
    let mut index = BTreeMap::new();
    let mut budget = TreeBudget {
        entries: 0,
        index_bytes: 0,
        hashed_bytes: 0,
        deadline: Instant::now() + HASH_TIMEOUT,
    };
    visit(&rooted, Path::new(""), &mut index, &mut budget)?;
    Ok(index)
}

fn validate_expected_file(
    files: &FileIndex,
    path: &str,
    size: u64,
    hash: &str,
    kind: &str,
) -> anyhow::Result<()> {
    ensure!(valid_blake3(hash), "{kind} hash is invalid");
    let entry = files
        .get(path)
        .with_context(|| format!("{kind} is missing from DATA_DIR"))?;
    ensure!(
        entry.size == size && entry.blake3 == hash.to_ascii_lowercase(),
        "{kind} hash or size does not match DATA_DIR"
    );
    Ok(())
}

fn ensure_scoped_key(account: &str, object: &str) -> anyhow::Result<()> {
    validate_storage_key(account)?;
    validate_storage_key(object)?;
    let relative = Path::new(object)
        .strip_prefix(account)
        .context("blob is outside its account storage path")?;
    ensure!(
        !relative.as_os_str().is_empty(),
        "blob path must be below its account storage path"
    );
    Ok(())
}

fn ensure_commit_stage_key(account: &str, upload_id: Uuid, staged: &str) -> anyhow::Result<()> {
    validate_storage_key(staged)?;
    let staged = Path::new(staged);
    ensure!(
        staged.parent() == Some(Path::new(account).join("staging").as_path()),
        "staged upload object is outside its account staging directory"
    );
    let name = staged
        .file_name()
        .and_then(|value| value.to_str())
        .context("staged upload object name is invalid")?;
    let nonce = name
        .strip_prefix(&format!("commit-{upload_id}-"))
        .and_then(|value| value.strip_suffix(".stage"))
        .context("staged upload object name does not match its upload")?;
    ensure!(
        Uuid::parse_str(nonce).is_ok(),
        "staged upload object nonce is invalid"
    );
    Ok(())
}

fn validate_storage_key(value: &str) -> anyhow::Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 4096
            && value.trim() == value
            && !value.contains(['\\', ':', '\0'])
            && !value.chars().any(char::is_control),
        "storage path is invalid"
    );
    validate_relative_path(Path::new(value))?;
    ensure!(
        value.split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && !component.starts_with('.')
                && component.trim() == component
        }),
        "database storage key is invalid"
    );
    Ok(())
}

fn validate_relative_path(path: &Path) -> anyhow::Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && !path.is_absolute()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "storage path contains an escape or non-normal component"
    );
    Ok(())
}

fn is_upload_part_path(path: &str) -> bool {
    path.starts_with("uploads/") && path.ends_with(".part")
}

fn valid_blake3(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn database_write_rollback_probe(lock: &RuntimeLock) -> anyhow::Result<()> {
    let url = lock.database_url()?;
    xcss_sqlite::block_on_sqlite_connection(async {
        let mut connection = url
            .parse::<SqliteConnectOptions>()?
            .create_if_missing(false)
            .busy_timeout(Duration::from_secs(2))
            .connect()
            .await?;
        let result = async {
            database::harden_connection(&mut connection).await?;
            sqlx::query("BEGIN IMMEDIATE")
                .execute(&mut connection)
                .await?;
            let probe = async {
                sqlx::query("CREATE TABLE __xszs_doctor_probe(value INTEGER NOT NULL)")
                    .execute(&mut connection)
                    .await?;
                sqlx::query("INSERT INTO __xszs_doctor_probe(value) VALUES(1)")
                    .execute(&mut connection)
                    .await?;
                let value: i64 = sqlx::query_scalar("SELECT value FROM __xszs_doctor_probe")
                    .fetch_one(&mut connection)
                    .await?;
                ensure!(value == 1, "database write/read probe failed");
                Ok::<(), anyhow::Error>(())
            }
            .await;
            let rollback = sqlx::query("ROLLBACK").execute(&mut connection).await;
            probe?;
            rollback?;
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name='__xszs_doctor_probe'",
            )
            .fetch_one(&mut connection)
            .await?;
            ensure!(exists == 0, "database rollback probe left persistent state");
            Ok::<(), anyhow::Error>(())
        }
        .await;
        let closed = connection.close().await;
        result?;
        closed?;
        Ok::<(), anyhow::Error>(())
    })?;
    lock.verify_original_paths()
}

fn storage_write_cleanup_probe(data_dir: &Path) -> anyhow::Result<()> {
    let rooted = RootedFs::new(data_dir)?;
    let relative = PathBuf::from(format!(".xszs-doctor-{}", Uuid::new_v4()));
    let mut file = rooted.create_new_std(&relative)?;
    let probe_result = (|| -> anyhow::Result<()> {
        file.write_all(b"xszs-doctor-v1")?;
        file.sync_all()?;
        drop(file);
        let mut file = rooted.open_read_std(&relative)?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        ensure!(
            contents == b"xszs-doctor-v1",
            "storage read/write probe failed"
        );
        Ok(())
    })();
    let cleanup_result = rooted.remove_file(&relative);
    probe_result?;
    ensure!(cleanup_result?, "storage probe cleanup failed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_account_prefix_check_detects_nonadjacent_ancestors() {
        xcss_sqlite::block_on_sqlite_connection(async {
            let mut connection = SqliteConnection::connect("sqlite::memory:").await.unwrap();
            database::harden_connection(&mut connection).await.unwrap();
            sqlx::query(
                "CREATE TABLE accounts(id INTEGER PRIMARY KEY, storage_path TEXT NOT NULL)",
            )
            .execute(&mut connection)
            .await
            .unwrap();
            for (id, path) in [(1, "media"), (2, "media!/else"), (3, "other/child")] {
                sqlx::query("INSERT INTO accounts(id,storage_path) VALUES(?,?)")
                    .bind(id)
                    .bind(path)
                    .execute(&mut connection)
                    .await
                    .unwrap();
            }
            validate_account_storage_paths(&mut connection)
                .await
                .unwrap();
            sqlx::query("UPDATE accounts SET storage_path='media/child' WHERE id=3")
                .execute(&mut connection)
                .await
                .unwrap();
            let error = validate_account_storage_paths(&mut connection)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("overlap"));
            connection.close().await.unwrap();
        });
    }

    #[test]
    fn diagnostic_write_failure_preserves_existing_table_and_rows() {
        let temporary = tempfile::tempdir().unwrap();
        let database_path = temporary.path().join("media.sqlite3");
        let data = temporary.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let database_url = format!("sqlite://{}", database_path.display());
        database::initialize(&database_url).unwrap();
        xcss_sqlite::block_on_sqlite_connection(async {
            let mut connection = SqliteConnection::connect(&database_url).await.unwrap();
            sqlx::raw_sql("CREATE TABLE __xszs_doctor_probe(value INTEGER NOT NULL); INSERT INTO __xszs_doctor_probe(value) VALUES(7)")
                .execute(&mut connection).await.unwrap();
            connection.close().await.unwrap();
        });
        let lock = RuntimeLock::acquire(&database_url, &data).unwrap();
        assert!(database_write_rollback_probe(&lock).is_err());
        xcss_sqlite::block_on_sqlite_connection(async {
            let mut connection = SqliteConnection::connect(&lock.database_url().unwrap())
                .await
                .unwrap();
            let rows: Vec<i64> = sqlx::query_scalar("SELECT value FROM __xszs_doctor_probe")
                .fetch_all(&mut connection)
                .await
                .unwrap();
            assert_eq!(rows, [7]);
            connection.close().await.unwrap();
        });
        lock.verify_original_paths().unwrap();
    }

    #[test]
    fn diagnostic_directory_depth_refuses_without_removing_files() {
        let temporary = tempfile::tempdir().unwrap();
        let mut deepest = temporary.path().to_owned();
        for _ in 0..=MAX_TREE_DEPTH {
            deepest.push("d");
            std::fs::create_dir(&deepest).unwrap();
        }
        let file = deepest.join("preserved");
        std::fs::write(&file, b"kept").unwrap();
        let error = index_data_tree(temporary.path()).unwrap_err();
        assert!(error.to_string().contains("depth budget"));
        assert_eq!(std::fs::read(file).unwrap(), b"kept");
    }

    #[tokio::test]
    async fn doctor_checks_a_current_empty_installation() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("media.sqlite3");
        let data = temporary.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let database_url = format!("sqlite://{}", database.display());
        database::initialize(&database_url).unwrap();
        let pool = database::connect(&database_url).await.unwrap();
        pool.close().await;
        let config = Config {
            database_url,
            data_dir: data,
            bind: "127.0.0.1:0".parse().unwrap(),
            bootstrap_admin_username: "doctor-admin".to_owned(),
            bootstrap_admin_password: Some("doctor-password".to_owned()),
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

        let summary = run(&config).unwrap();
        assert_eq!(summary.status, "ok");
        assert_eq!(summary.files, 0);
        assert_eq!(summary.blobs, 0);
        assert_eq!(summary.uploads, 0);
        assert_eq!(summary.schema_revision, 1);
    }
}
