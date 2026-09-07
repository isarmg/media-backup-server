use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use std::{fs::File, path::Component};

use anyhow::{ensure, Context};
#[cfg(target_os = "linux")]
use rustix::{
    fs::{
        flock, fstat, open, openat2, statat, AtFlags, FileType, FlockOperation, Mode, OFlags,
        ResolveFlags,
    },
    io::Errno,
};

pub(crate) struct RuntimeLock {
    #[cfg(target_os = "linux")]
    _files: Vec<File>,
}

#[cfg(target_os = "linux")]
impl RuntimeLock {
    pub(crate) fn acquire(database_url: &str, data_dir: &Path) -> anyhow::Result<Self> {
        let database = sqlite_database_path(database_url)?;
        let mut locations = vec![
            lock_location(&database, "SQLite database", ResourceKind::RegularFile)?,
            lock_location(data_dir, "DATA_DIR", ResourceKind::Directory)?,
        ];
        locations.sort_by(|left, right| left.path.cmp(&right.path));
        locations.dedup_by(|left, right| left.path == right.path);
        let filesystem_root = open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let mut files = Vec::with_capacity(locations.len());
        for location in locations {
            let relative_parent = location
                .parent
                .strip_prefix("/")
                .context("runtime lock parent must be absolute")?;
            let relative_parent = if relative_parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                relative_parent
            };
            let resolve =
                ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS;
            let parent_fd = openat2(
                &filesystem_root,
                relative_parent,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
                resolve,
            )
            .with_context(|| "open Media Backup runtime lock parent without following links")?;
            match statat(
                &parent_fd,
                &location.resource_name,
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(metadata) => {
                    let actual = FileType::from_raw_mode(metadata.st_mode);
                    ensure!(
                        actual == location.kind.file_type(),
                        "Media Backup runtime resource is a symbolic link or has the wrong type"
                    );
                    if actual == FileType::RegularFile {
                        ensure!(
                            metadata.st_nlink == 1,
                            "Media Backup SQLite resource has multiple hard links"
                        );
                    }
                }
                Err(Errno::NOENT) => {}
                Err(error) => {
                    return Err(std::io::Error::from(error))
                        .context("inspect Media Backup runtime resource")
                }
            }
            let fd = openat2(
                &parent_fd,
                &location.name,
                OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
                resolve,
            )
            .with_context(|| "open Media Backup runtime lock")?;
            let metadata = fstat(&fd)?;
            ensure!(
                FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile
                    && metadata.st_nlink == 1,
                "runtime lock is not a single-link regular file"
            );
            match flock(&fd, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => files.push(File::from(fd)),
                Err(Errno::WOULDBLOCK) => {
                    anyhow::bail!("Media Backup service or diagnostic command is already running")
                }
                Err(error) => {
                    return Err(std::io::Error::from(error)).context("lock Media Backup runtime")
                }
            }
        }
        Ok(Self { _files: files })
    }
}

#[cfg(not(target_os = "linux"))]
impl RuntimeLock {
    pub(crate) fn acquire(database_url: &str, _data_dir: &Path) -> anyhow::Result<Self> {
        sqlite_database_path(database_url)?;
        anyhow::bail!("secure Media Backup runtime locking requires Linux openat2")
    }
}

#[cfg(target_os = "linux")]
struct LockLocation {
    path: PathBuf,
    parent: PathBuf,
    name: std::ffi::OsString,
    resource_name: std::ffi::OsString,
    kind: ResourceKind,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum ResourceKind {
    Directory,
    RegularFile,
}

#[cfg(target_os = "linux")]
impl ResourceKind {
    fn file_type(self) -> FileType {
        match self {
            Self::Directory => FileType::Directory,
            Self::RegularFile => FileType::RegularFile,
        }
    }
}

#[cfg(target_os = "linux")]
fn lock_location(resource: &Path, label: &str, kind: ResourceKind) -> anyhow::Result<LockLocation> {
    let absolute = if resource.is_absolute() {
        resource.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolve runtime lock working directory")?
            .join(resource)
    };
    let mut clean = PathBuf::from("/");
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(value) => clean.push(value),
            Component::ParentDir => {
                anyhow::bail!("{label} must not contain parent-directory components")
            }
            Component::Prefix(_) => anyhow::bail!("{label} has an unsupported path prefix"),
        }
    }
    let resource_name = clean
        .file_name()
        .with_context(|| format!("{label} must name a filesystem entry"))?
        .to_os_string();
    let mut lock_name = std::ffi::OsString::from(".");
    lock_name.push(&resource_name);
    lock_name.push(".media-backup.lock");
    let parent = clean
        .parent()
        .with_context(|| format!("{label} must have a parent directory"))?
        .to_path_buf();
    Ok(LockLocation {
        path: parent.join(&lock_name),
        parent,
        name: lock_name,
        resource_name,
        kind,
    })
}

pub(crate) fn sqlite_database_path(database_url: &str) -> anyhow::Result<PathBuf> {
    let value = database_url
        .strip_prefix("sqlite://")
        .or_else(|| database_url.strip_prefix("sqlite:"))
        .context("DATABASE_URL must use the sqlite scheme")?;
    ensure!(
        !value.is_empty() && value != ":memory:",
        "Media Backup requires a file SQLite database"
    );
    ensure!(
        !value.contains(['?', '#', '%', '\0']),
        "Media Backup requires a plain unescaped SQLite file URL"
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
            .any(|component| matches!(component, std::path::Component::ParentDir)),
        "SQLite database path must not contain parent traversal"
    );
    Ok(path)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use uuid::Uuid;

    fn database_url(path: &Path) -> String {
        format!("sqlite://{}", path.display())
    }

    #[test]
    fn exclusive_runtime_lock_rejects_service_and_cli_overlap() {
        let root = std::env::temp_dir().join(format!("media-runtime-lock-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let data = root.join("data");
        let database = database_url(&root.join("app.sqlite3"));
        let first = RuntimeLock::acquire(&database, &data).unwrap();
        assert!(RuntimeLock::acquire(&database, &data).is_err());
        drop(first);
        RuntimeLock::acquire(&database, &data).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn either_shared_database_or_shared_data_directory_is_exclusive() {
        let root = std::env::temp_dir().join(format!("media-runtime-lock-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let database_a = database_url(&root.join("a.sqlite3"));
        let database_b = database_url(&root.join("b.sqlite3"));
        let data_a = root.join("data-a");
        let data_b = root.join("data-b");

        let same_database = RuntimeLock::acquire(&database_a, &data_a).unwrap();
        assert!(RuntimeLock::acquire(&database_a, &data_b).is_err());
        drop(same_database);

        let same_data = RuntimeLock::acquire(&database_a, &data_a).unwrap();
        assert!(RuntimeLock::acquire(&database_b, &data_a).is_err());
        drop(same_data);
        RuntimeLock::acquire(&database_b, &data_b).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_lock_rejects_a_symlink_in_the_parent_chain() {
        let root = std::env::temp_dir().join(format!("media-runtime-lock-{}", Uuid::new_v4()));
        let real = root.join("real");
        std::fs::create_dir_all(&real).unwrap();
        symlink(&real, root.join("linked-parent")).unwrap();
        let database = database_url(&root.join("app.sqlite3"));
        assert!(RuntimeLock::acquire(&database, &root.join("linked-parent/data")).is_err());
        assert!(!root.join("linked-parent/.data.media-backup.lock").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_lock_rejects_symlinked_database_and_data_resources() {
        let root = std::env::temp_dir().join(format!("media-runtime-lock-{}", Uuid::new_v4()));
        let real_data = root.join("real-data");
        std::fs::create_dir_all(&real_data).unwrap();
        let real_database = root.join("real.sqlite3");
        std::fs::write(&real_database, b"not-opened-by-lock-test").unwrap();
        let linked_database = root.join("linked.sqlite3");
        let linked_data = root.join("linked-data");
        symlink(&real_database, &linked_database).unwrap();
        symlink(&real_data, &linked_data).unwrap();

        assert!(RuntimeLock::acquire(&database_url(&linked_database), &real_data).is_err());
        assert!(RuntimeLock::acquire(&database_url(&real_database), &linked_data).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_lock_rejects_database_and_lock_file_hardlink_aliases() {
        let root = std::env::temp_dir().join(format!("media-runtime-lock-{}", Uuid::new_v4()));
        let data_a = root.join("data-a");
        let data_b = root.join("data-b");
        std::fs::create_dir_all(&data_a).unwrap();
        std::fs::create_dir_all(&data_b).unwrap();
        let database = root.join("app.sqlite3");
        let database_alias = root.join("alias.sqlite3");
        std::fs::write(&database, b"hardlink-identity-test").unwrap();
        std::fs::hard_link(&database, &database_alias).unwrap();
        assert!(RuntimeLock::acquire(&database_url(&database), &data_a).is_err());
        assert!(RuntimeLock::acquire(&database_url(&database_alias), &data_b).is_err());

        std::fs::remove_file(&database_alias).unwrap();
        let disposable = root.join("disposable.sqlite3");
        RuntimeLock::acquire(&database_url(&disposable), &data_a).unwrap();
        let lock = root.join(".disposable.sqlite3.media-backup.lock");
        let aliased_database = root.join("lock-alias.sqlite3");
        let lock_alias = root.join(".lock-alias.sqlite3.media-backup.lock");
        std::fs::hard_link(lock, lock_alias).unwrap();
        assert!(RuntimeLock::acquire(&database_url(&aliased_database), &data_b).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
