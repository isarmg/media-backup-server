use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use anyhow::{ensure, Context};
use rusqlite::{params, Connection, OpenFlags};
use sarmg_schema_identity::{
    schema_fingerprint as foundation_schema_fingerprint, validate_product_metadata_columns,
    validate_product_metadata_ddl, verify_current_schema, ProductMetadataColumn,
    ProductMetadataRow, SchemaIdentity, SchemaRow, SQLITE_SCHEMA_ROWS_QUERY,
};
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    SqlitePool,
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

const APPLICATION: &str = "media-backup";
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const CURRENT_SCHEMA: &str = include_str!("../schema/generated/current_schema.sql");
pub(crate) const CURRENT_SCHEMA_REVISION: i64 = 2;
pub(crate) const CURRENT_SCHEMA_SHA256: &str =
    "6415edde88228d508f1c0c7582f119c8fe869d2d78fd85129f359a5d748cbbc2";

pub(crate) async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    prepare_current_database(database_url)?;
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT)
        .synchronous(SqliteSynchronous::Full);

    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect_with(options)
        .await?;
    // Revalidate after SQLx opens the production generation. A correctly
    // configured process holds RuntimeLock across both operations.
    if let Err(error) = validate_current_database(&database_path(database_url)?) {
        pool.close().await;
        return Err(error);
    }
    Ok(pool)
}

fn prepare_current_database(database_url: &str) -> anyhow::Result<()> {
    let path = database_path(database_url)?;
    require_real_parent(&path)?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            require_absent_sidecars(&path)?;
            initialize_current_database(&path)
        }
        Err(error) => Err(error.into()),
        Ok(_) => validate_current_database(&path),
    }
}

pub(crate) fn validate_current_database(path: &Path) -> anyhow::Result<()> {
    require_secure_database_file(path)?;
    let snapshot = snapshot_generation(path)?;
    let connection = Connection::open_with_flags(
        &snapshot.database,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .context("open private SQLite current-schema snapshot")?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    validate_current_connection(&connection)
}

pub(crate) fn integrity_and_foreign_key_check(path: &Path) -> anyhow::Result<()> {
    require_secure_database_file(path)?;
    let snapshot = snapshot_generation(path)?;
    let connection = Connection::open_with_flags(
        &snapshot.database,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    ensure!(
        integrity.eq_ignore_ascii_case("ok"),
        "SQLite integrity check failed"
    );
    let mut foreign_keys = connection.prepare("PRAGMA foreign_key_check")?;
    ensure!(
        foreign_keys.query([])?.next()?.is_none(),
        "SQLite foreign-key check failed"
    );
    Ok(())
}

pub(crate) fn database_path(database_url: &str) -> anyhow::Result<PathBuf> {
    let value = database_url
        .strip_prefix("sqlite://")
        .or_else(|| database_url.strip_prefix("sqlite:"))
        .context("DATABASE_URL must use the sqlite scheme")?;
    ensure!(!value.is_empty(), "SQLite database path must not be empty");
    ensure!(value != ":memory:", "in-memory SQLite is not supported");
    ensure!(
        !value.contains(['?', '#', '%', '\0']),
        "DATABASE_URL must be a plain, unescaped SQLite file URL"
    );
    let path = PathBuf::from(value);
    ensure!(path.is_absolute(), "SQLite database path must be absolute");
    ensure!(
        path.file_name().is_some(),
        "SQLite database path must name a file"
    );
    ensure!(
        !path
            .components()
            .any(|component| matches!(component, Component::ParentDir)),
        "SQLite database path must not contain parent traversal"
    );
    Ok(path)
}

struct ValidationSnapshot {
    _directory: tempfile::TempDir,
    database: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceSnapshot {
    hash: [u8; 32],
    length: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

#[derive(Debug, thiserror::Error)]
#[error("SQLite generation changed during current-schema validation")]
struct GenerationChanged;

fn snapshot_generation(path: &Path) -> anyhow::Result<ValidationSnapshot> {
    let mut last_change = None;
    for _ in 0..4 {
        match snapshot_generation_once(path) {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) if error.is::<GenerationChanged>() => {
                last_change = Some(error);
                std::thread::yield_now();
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_change.expect("snapshot retry loop records each generation change"))
}

fn snapshot_generation_once(path: &Path) -> anyhow::Result<ValidationSnapshot> {
    let directory = tempfile::Builder::new()
        .prefix("media-schema-check-")
        .tempdir()
        .context("create private current-schema validation directory")?;
    let database = directory.path().join("database.sqlite3");
    let sources = [
        path.to_path_buf(),
        sqlite_sidecar(path, "-wal"),
        sqlite_sidecar(path, "-journal"),
    ];
    let destinations = [
        database.clone(),
        sqlite_sidecar(&database, "-wal"),
        sqlite_sidecar(&database, "-journal"),
    ];

    // Even a SQLite read-only WAL connection writes lock bytes to `-shm`.
    // Validate a private generation so rejection leaves every source byte
    // unchanged, including schemas committed only in the WAL.
    let mut expected = Vec::with_capacity(sources.len());
    for (source, destination) in sources.iter().zip(&destinations) {
        expected.push(copy_generation_file(source, destination)?);
    }
    let _ = source_snapshot(&sqlite_sidecar(path, "-shm"))?;
    for (source, expected) in sources.iter().zip(expected) {
        if source_snapshot(source)? != expected {
            return Err(GenerationChanged.into());
        }
    }

    Ok(ValidationSnapshot {
        _directory: directory,
        database,
    })
}

fn copy_generation_file(
    source_path: &Path,
    destination_path: &Path,
) -> anyhow::Result<Option<SourceSnapshot>> {
    let Some((mut source, before)) = open_source_snapshot(source_path)? else {
        return Ok(None);
    };
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let mut destination = options.open(destination_path)?;
    std::io::copy(&mut source, &mut destination)?;
    destination.flush()?;
    destination.sync_all()?;
    destination.seek(SeekFrom::Start(0))?;
    let copied_hash = hash_reader(&mut destination)?;
    let Some(after) = source_snapshot(source_path)? else {
        return Err(GenerationChanged.into());
    };
    if before != after || copied_hash != after.hash {
        return Err(GenerationChanged.into());
    }
    Ok(Some(after))
}

fn source_snapshot(path: &Path) -> anyhow::Result<Option<SourceSnapshot>> {
    let Some((_, snapshot)) = open_source_snapshot(path)? else {
        return Ok(None);
    };
    Ok(Some(snapshot))
}

fn open_source_snapshot(path: &Path) -> anyhow::Result<Option<(File, SourceSnapshot)>> {
    let initial = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        initial.is_file() && !initial.file_type().is_symlink(),
        "SQLite generation must contain only regular files without symbolic links"
    );
    #[cfg(unix)]
    ensure!(
        initial.nlink() == 1,
        "SQLite generation files must not have hard-link aliases"
    );
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let opened = file.metadata()?;
    let named = fs::symlink_metadata(path)?;
    ensure!(
        opened.is_file() && named.is_file() && !named.file_type().is_symlink(),
        "SQLite generation must contain only regular files without symbolic links"
    );
    #[cfg(unix)]
    ensure!(
        opened.nlink() == 1
            && named.nlink() == 1
            && opened.dev() == named.dev()
            && opened.ino() == named.ino(),
        "SQLite generation files must not have hard-link aliases or change while opened"
    );
    let hash = hash_reader(&mut file)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(Some((
        file,
        SourceSnapshot {
            hash,
            length: opened.len(),
            #[cfg(unix)]
            device: opened.dev(),
            #[cfg(unix)]
            inode: opened.ino(),
        },
    )))
}

fn hash_reader(reader: &mut impl Read) -> anyhow::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn initialize_current_database(path: &Path) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let reserved = options
        .open(path)
        .with_context(|| format!("create current SQLite database {}", path.display()))?;
    reserved.sync_all()?;
    drop(reserved);

    let result = (|| {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        connection.execute_batch("PRAGMA foreign_keys=ON; BEGIN IMMEDIATE;")?;
        let transaction_result = (|| {
            connection.execute_batch(CURRENT_SCHEMA)?;
            connection.execute(
                "INSERT INTO _sarmg_platform_metadata(\
                 singleton,platform_generation,platform_schema_revision,profile,created_at_micros\
                 ) VALUES(1,1,1,'server-control-plane',?)",
                [0_i64],
            )?;
            let actual = schema_fingerprint(&connection)?;
            ensure!(
                actual == CURRENT_SCHEMA_SHA256,
                "compiled current schema fingerprint mismatch: {actual}"
            );
            connection.execute(
                "INSERT INTO product_metadata (
                     singleton, application, application_version, schema_revision, schema_sha256
                 ) VALUES (1, ?, ?, ?, ?)",
                params![
                    APPLICATION,
                    env!("CARGO_PKG_VERSION"),
                    CURRENT_SCHEMA_REVISION,
                    CURRENT_SCHEMA_SHA256
                ],
            )?;
            validate_current_connection(&connection)
        })();
        match transaction_result {
            Ok(()) => connection.execute_batch("COMMIT")?,
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK");
                return Err(error);
            }
        }
        drop(connection);
        File::open(path)?.sync_all()?;
        sync_parent(path)?;
        Ok(())
    })();

    if result.is_err() {
        for candidate in sqlite_generation_paths(path) {
            let _ = fs::remove_file(candidate);
        }
        let _ = sync_parent(path);
    }
    result
}

fn require_absent_sidecars(path: &Path) -> anyhow::Result<()> {
    for candidate in sqlite_generation_paths(path).into_iter().skip(1) {
        match fs::symlink_metadata(&candidate) {
            Ok(_) => anyhow::bail!(
                "SQLite main file is absent but its generation contains a sidecar; refusing initialization"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn validate_current_connection(connection: &Connection) -> anyhow::Result<()> {
    validate_product_metadata_table(connection)?;
    let mut metadata_statement = connection.prepare(
        "SELECT singleton, application, application_version, schema_revision, schema_sha256
         FROM product_metadata ORDER BY singleton",
    )?;
    let metadata_rows = metadata_statement
        .query_map([], |row| {
            Ok(ProductMetadataRow {
                singleton: row.get(0)?,
                application: row.get(1)?,
                application_version: row.get(2)?,
                schema_revision: row.get(3)?,
                schema_sha256: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let schema_rows = foundation_schema_rows(connection)?;
    verify_current_schema(
        &metadata_rows,
        &schema_rows,
        &current_schema_identity()?,
    )
    .context(
        "database is not the exact current Media Backup schema; use sarmg-upgrade for offline conversion",
    )?;
    Ok(())
}

fn validate_product_metadata_table(connection: &Connection) -> anyhow::Result<()> {
    let sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema
             WHERE type = 'table' AND name = 'product_metadata'",
            [],
            |row| row.get(0),
        )
        .context("database has no current product_metadata table")?;
    validate_product_metadata_ddl(&sql)
        .context("product_metadata DDL does not match the Foundation current contract")?;
    let mut columns = connection.prepare("PRAGMA table_info('product_metadata')")?;
    let actual = columns
        .query_map([], |row| {
            Ok(ProductMetadataColumn {
                cid: row.get(0)?,
                name: row.get(1)?,
                declared_type: row.get(2)?,
                not_null: row.get(3)?,
                default_sql: row.get(4)?,
                primary_key_position: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    validate_product_metadata_columns(&actual)
        .context("product_metadata columns do not match the Foundation current contract")?;
    Ok(())
}

fn schema_fingerprint(connection: &Connection) -> anyhow::Result<String> {
    foundation_schema_fingerprint(&foundation_schema_rows(connection)?)
        .context("Foundation rejected the canonical SQLite schema rows")
}

fn foundation_schema_rows(connection: &Connection) -> anyhow::Result<Vec<SchemaRow>> {
    let mut statement = connection.prepare(SQLITE_SCHEMA_ROWS_QUERY)?;
    let rows = statement.query_map([], |row| {
        Ok(SchemaRow::new(
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .context("collect canonical SQLite schema rows")
}

pub(crate) fn current_schema_identity() -> anyhow::Result<SchemaIdentity> {
    SchemaIdentity::new(
        APPLICATION,
        env!("CARGO_PKG_VERSION"),
        u64::try_from(CURRENT_SCHEMA_REVISION).context("schema revision must not be negative")?,
        CURRENT_SCHEMA_SHA256,
    )
    .context("compiled Media Backup schema identity is invalid")
}

fn require_secure_database_file(path: &Path) -> anyhow::Result<()> {
    require_real_parent(path)?;
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("SQLite database does not exist: {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "SQLite database must be a regular file without symbolic links"
    );
    #[cfg(unix)]
    ensure!(
        metadata.nlink() == 1,
        "SQLite database must not have hard-link aliases"
    );
    Ok(())
}

fn require_real_parent(path: &Path) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("SQLite database must have a parent")?;
    let mut current = PathBuf::new();
    for component in parent.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => current.push("."),
            Component::Normal(value) => current.push(value),
            Component::ParentDir => anyhow::bail!("SQLite path must not contain parent traversal"),
        }
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("SQLite parent does not exist: {}", current.display()))?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "SQLite path must not traverse symbolic links or special files"
        );
    }
    Ok(())
}

fn sqlite_generation_paths(path: &Path) -> [PathBuf; 4] {
    [
        path.to_path_buf(),
        sqlite_sidecar(path, "-wal"),
        sqlite_sidecar(path, "-shm"),
        sqlite_sidecar(path, "-journal"),
    ]
}

fn sqlite_sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn sync_parent(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn current_schema_fingerprint_matches_compiled_contract() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("current.sqlite3");
        initialize_current_database(&database).unwrap();
        validate_current_database(&database).unwrap();
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&database).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let connection = Connection::open(&database).unwrap();
        let metadata: (String, String, i64, String) = connection
            .query_row(
                "SELECT application, application_version, schema_revision, schema_sha256
                 FROM product_metadata WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            metadata,
            (
                APPLICATION.to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
                CURRENT_SCHEMA_REVISION,
                CURRENT_SCHEMA_SHA256.to_string()
            )
        );
    }

    #[tokio::test]
    async fn foreign_database_without_metadata_is_rejected_without_changing_bytes() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("foreign.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE unrelated_records(id TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO unrelated_records VALUES('record', 'foreign');",
            )
            .unwrap();
        drop(connection);
        assert_rejected_without_byte_changes(&database).await;
    }

    #[tokio::test]
    async fn existing_empty_file_is_not_initialized_or_modified() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("existing-empty.sqlite3");
        File::create(&database).unwrap();
        assert_rejected_without_byte_changes(&database).await;
        assert_eq!(fs::metadata(database).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn orphan_sidecar_prevents_initialization_without_byte_changes() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("missing-main.sqlite3");
        let wal = sqlite_sidecar(&database, "-wal");
        fs::write(&wal, b"orphan-wal-evidence").unwrap();
        let before = fs::read(&wal).unwrap();
        let error = connect(&format!("sqlite://{}", database.display()))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("sidecar"));
        assert!(!database.exists());
        assert_eq!(fs::read(wal).unwrap(), before);
    }

    #[tokio::test]
    async fn noncurrent_wal_generation_is_rejected_without_changing_bytes() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("noncurrent-wal.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA wal_autocheckpoint=0;
                 CREATE TABLE accounts(id TEXT PRIMARY KEY, username TEXT NOT NULL);
                 INSERT INTO accounts VALUES('unknown-user', 'noncurrent-format');",
            )
            .unwrap();
        assert!(sqlite_sidecar(&database, "-wal").exists());
        assert_rejected_without_byte_changes(&database).await;
        drop(connection);
    }

    #[test]
    fn current_schema_committed_only_in_wal_is_validated_without_changes() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("current-wal.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        connection.execute_batch(CURRENT_SCHEMA).unwrap();
        connection
            .execute(
                "INSERT INTO _sarmg_platform_metadata(\
                 singleton,platform_generation,platform_schema_revision,profile,created_at_micros\
                 ) VALUES(1,1,1,'server-control-plane',?)",
                [0_i64],
            )
            .unwrap();
        let fingerprint = schema_fingerprint(&connection).unwrap();
        connection
            .execute(
                "INSERT INTO product_metadata (
                     singleton, application, application_version, schema_revision, schema_sha256
                 ) VALUES (1, ?, ?, ?, ?)",
                params![
                    APPLICATION,
                    env!("CARGO_PKG_VERSION"),
                    CURRENT_SCHEMA_REVISION,
                    fingerprint
                ],
            )
            .unwrap();
        assert!(sqlite_sidecar(&database, "-wal").exists());
        let before = generation_bytes(&database);
        validate_current_database(&database).unwrap();
        assert!(
            generation_bytes(&database) == before,
            "current-schema validation changed SQLite generation bytes"
        );
        drop(connection);
    }

    #[tokio::test]
    async fn schema_drift_committed_only_in_wal_is_rejected_without_changes() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("drift-wal.sqlite3");
        initialize_current_database(&database).unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA wal_autocheckpoint=0;
                 CREATE TABLE unexpected_wal_table(id INTEGER);",
            )
            .unwrap();
        assert!(sqlite_sidecar(&database, "-wal").exists());
        assert_rejected_without_byte_changes(&database).await;
        drop(connection);
    }

    #[tokio::test]
    async fn metadata_table_contract_is_exact_and_read_only_on_rejection() {
        let temporary = tempfile::tempdir().unwrap();
        let database = temporary.path().join("metadata-contract.sqlite3");
        initialize_current_database(&database).unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch("PRAGMA journal_mode=DELETE;")
            .unwrap();
        connection
            .execute_batch(&format!(
                "DROP TABLE product_metadata;
                 CREATE TABLE product_metadata (
                     singleton INTEGER PRIMARY KEY,
                     application TEXT NOT NULL,
                     application_version TEXT NOT NULL,
                     schema_revision INTEGER NOT NULL,
                     schema_sha256 TEXT NOT NULL
                 );
                 INSERT INTO product_metadata VALUES
                     (1, '{APPLICATION}', '{}', {CURRENT_SCHEMA_REVISION}, '{CURRENT_SCHEMA_SHA256}');",
                env!("CARGO_PKG_VERSION")
            ))
            .unwrap();
        drop(connection);
        assert_rejected_without_byte_changes(&database).await;
    }

    #[tokio::test]
    async fn nonexact_metadata_and_schema_are_read_only_rejections() {
        for (name, statement) in [
            (
                "wrong-application",
                "UPDATE product_metadata SET application = 'another-product'",
            ),
            (
                "noncurrent-version",
                "UPDATE product_metadata SET application_version = 'noncurrent-version'",
            ),
            (
                "wrong-revision",
                "UPDATE product_metadata SET schema_revision = 3",
            ),
            (
                "wrong-fingerprint",
                "UPDATE product_metadata SET schema_sha256 = 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'",
            ),
            ("schema-tamper", "CREATE TABLE unexpected_product_table(id INTEGER)"),
        ] {
            let temporary = tempfile::tempdir().unwrap();
            let database = temporary.path().join(format!("{name}.sqlite3"));
            initialize_current_database(&database).unwrap();
            let connection = Connection::open(&database).unwrap();
            connection.execute_batch("PRAGMA journal_mode=DELETE;").unwrap();
            connection.execute_batch(statement).unwrap();
            drop(connection);
            assert_rejected_without_byte_changes(&database).await;
        }
    }

    async fn assert_rejected_without_byte_changes(path: &Path) {
        let before = generation_bytes(path);
        let url = format!("sqlite://{}", path.display());
        let error = connect(&url).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("database")
                || format!("{error:#}").contains("product_metadata")
                || format!("{error:#}").contains("schema"),
            "rejection must identify the current-state boundary: {error:#}"
        );
        assert!(
            generation_bytes(path) == before,
            "current-schema rejection changed SQLite generation bytes"
        );
    }

    fn generation_bytes(path: &Path) -> BTreeMap<String, Vec<u8>> {
        sqlite_generation_paths(path)
            .into_iter()
            .filter(|candidate| candidate.exists())
            .map(|candidate| {
                (
                    candidate
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(candidate).unwrap(),
                )
            })
            .collect()
    }
}
