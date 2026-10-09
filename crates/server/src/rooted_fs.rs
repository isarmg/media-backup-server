#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootEntryKind {
    Directory,
    RegularFile,
}

#[cfg(target_os = "linux")]
mod platform {
    use super::RootEntryKind;
    use rustix::{
        fd::OwnedFd,
        fs::{
            AtFlags, Dir, FileType, FlockOperation, Mode, OFlags, ResolveFlags, flock, fstat,
            fsync, linkat, mkdirat, open, openat2, statat, unlinkat,
        },
        io::{Errno, dup},
    };
    use std::{
        fmt,
        fs::File,
        os::unix::ffi::OsStrExt,
        path::{Component, Path},
        sync::Arc,
    };

    #[cfg(test)]
    use std::sync::Mutex;

    #[derive(Clone)]
    pub(crate) struct RootedFs {
        inner: Arc<RootedFsInner>,
    }

    struct RootedFsInner {
        root: File,
        resolve: ResolveFlags,
        #[cfg(test)]
        before_operation: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    struct OpenedParent {
        fd: OwnedFd,
        name: std::ffi::OsString,
    }

    impl fmt::Debug for RootedFs {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.debug_struct("RootedFs").finish_non_exhaustive()
        }
    }

    impl RootedFs {
        pub(crate) fn new(root_path: &Path) -> std::io::Result<Self> {
            let root = File::from(
                open(
                    root_path,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(std::io::Error::from)?,
            );
            if !root.metadata()?.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "DATA_DIR is not a directory",
                ));
            }
            let resolve =
                ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS;
            openat2(
                &root,
                ".",
                OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
                resolve,
            )
            .map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    format!("Linux openat2 is required for secure DATA_DIR access: {error}"),
                )
            })?;
            Ok(Self {
                inner: Arc::new(RootedFsInner {
                    root,
                    resolve,
                    #[cfg(test)]
                    before_operation: Mutex::new(None),
                }),
            })
        }

        pub(crate) fn ensure_dir(&self, path: &Path) -> std::io::Result<()> {
            validate_relative(path)?;
            self.run_before_operation_hook();
            self.open_directory(path, true).map(drop)
        }

        pub(crate) fn require_directory(&self, path: &Path) -> std::io::Result<()> {
            validate_relative(path)?;
            self.open_directory(path, false).map(drop)
        }

        pub(crate) fn create_new(&self, path: &Path) -> std::io::Result<tokio::fs::File> {
            self.create_new_std(path).map(tokio::fs::File::from_std)
        }

        pub(crate) fn create_new_std(&self, path: &Path) -> std::io::Result<File> {
            validate_relative(path)?;
            let parent = path.parent().unwrap_or_else(|| Path::new(""));
            if !parent.as_os_str().is_empty() {
                self.open_directory(parent, true)?;
            }
            self.run_before_operation_hook();
            let fd = openat2(
                &self.inner.root,
                path,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
                self.inner.resolve,
            )
            .map_err(std::io::Error::from)?;
            require_regular(&fd)?;
            Ok(File::from(fd))
        }

        pub(crate) fn open_read(&self, path: &Path) -> std::io::Result<tokio::fs::File> {
            self.open_read_std(path).map(tokio::fs::File::from_std)
        }

        pub(crate) fn open_read_std(&self, path: &Path) -> std::io::Result<File> {
            validate_relative(path)?;
            self.run_before_operation_hook();
            let fd = openat2(
                &self.inner.root,
                path,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
                self.inner.resolve,
            )
            .map_err(std::io::Error::from)?;
            require_regular(&fd)?;
            Ok(File::from(fd))
        }

        pub(crate) fn open_lock_file(&self, path: &Path) -> std::io::Result<File> {
            validate_relative(path)?;
            let parent = path.parent().unwrap_or_else(|| Path::new(""));
            if !parent.as_os_str().is_empty() {
                self.open_directory(parent, true)?;
            }
            self.run_before_operation_hook();
            let fd = openat2(
                &self.inner.root,
                path,
                OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
                self.inner.resolve,
            )
            .map_err(std::io::Error::from)?;
            require_regular(&fd)?;
            Ok(File::from(fd))
        }

        pub(crate) fn lock_exclusive(file: &File) -> std::io::Result<()> {
            flock(file, FlockOperation::LockExclusive).map_err(std::io::Error::from)
        }

        pub(crate) fn sync_parent(&self, path: &Path) -> std::io::Result<()> {
            validate_relative(path)?;
            self.run_before_operation_hook();
            let parent = self.open_parent(path, false)?;
            fsync(&parent.fd).map_err(std::io::Error::from)
        }

        pub(crate) fn list_regular_files(
            &self,
            directory: &Path,
        ) -> std::io::Result<Vec<std::ffi::OsString>> {
            validate_relative(directory)?;
            self.run_before_operation_hook();
            let fd = match self.open_directory(directory, false) {
                Ok(fd) => fd,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(error) => return Err(error),
            };
            let mut entries = Vec::new();
            for entry in Dir::new(fd).map_err(std::io::Error::from)? {
                let entry = entry.map_err(std::io::Error::from)?;
                let name = entry.file_name();
                if name.to_bytes() == b"." || name.to_bytes() == b".." {
                    continue;
                }
                let metadata = statat(
                    &self.inner.root,
                    directory.join(std::ffi::OsStr::from_bytes(name.to_bytes())),
                    AtFlags::SYMLINK_NOFOLLOW,
                )
                .map_err(std::io::Error::from)?;
                if FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile {
                    entries.push(std::ffi::OsString::from(std::ffi::OsStr::from_bytes(
                        name.to_bytes(),
                    )));
                }
            }
            Ok(entries)
        }

        #[cfg(test)]
        pub(crate) fn list_entries(
            &self,
            directory: &Path,
        ) -> std::io::Result<Vec<(std::ffi::OsString, RootEntryKind)>> {
            self.list_entries_bounded(directory, usize::MAX)
        }

        pub(crate) fn list_entries_bounded(
            &self,
            directory: &Path,
            max_entries: usize,
        ) -> std::io::Result<Vec<(std::ffi::OsString, RootEntryKind)>> {
            if !directory.as_os_str().is_empty() {
                validate_relative(directory)?;
            }
            self.run_before_operation_hook();
            let fd = self.open_directory(directory, false)?;
            let metadata_fd = dup(&fd).map_err(std::io::Error::from)?;
            let mut entries = Vec::new();
            for entry in Dir::new(fd).map_err(std::io::Error::from)? {
                let entry = entry.map_err(std::io::Error::from)?;
                let name = entry.file_name();
                if name.to_bytes() == b"." || name.to_bytes() == b".." {
                    continue;
                }
                if entries.len() == max_entries {
                    return Err(std::io::Error::other(
                        "media directory entry budget exceeded",
                    ));
                }
                let name = std::ffi::OsString::from(std::ffi::OsStr::from_bytes(name.to_bytes()));
                let metadata = statat(&metadata_fd, &name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(std::io::Error::from)?;
                let kind = match FileType::from_raw_mode(metadata.st_mode) {
                    FileType::RegularFile => RootEntryKind::RegularFile,
                    FileType::Directory => RootEntryKind::Directory,
                    _ => return Err(unsafe_entry("refusing a symbolic link or special entry")),
                };
                entries.push((name, kind));
            }
            Ok(entries)
        }

        pub(crate) fn remove_file(&self, path: &Path) -> std::io::Result<bool> {
            validate_relative(path)?;
            self.run_before_operation_hook();
            let parent = match self.open_parent(path, false) {
                Ok(parent) => parent,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error),
            };
            let metadata = match statat(&parent.fd, &parent.name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(metadata) => metadata,
                Err(Errno::NOENT) => return Ok(false),
                Err(error) => return Err(std::io::Error::from(error)),
            };
            if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile {
                return Err(unsafe_entry("refusing to remove a non-regular object"));
            }
            match unlinkat(&parent.fd, &parent.name, AtFlags::empty()) {
                Ok(()) => {
                    fsync(&parent.fd).map_err(std::io::Error::from)?;
                    Ok(true)
                }
                Err(Errno::NOENT) => Ok(false),
                Err(error) => Err(std::io::Error::from(error)),
            }
        }

        pub(crate) fn link_no_replace(
            &self,
            source: &Path,
            destination: &Path,
        ) -> std::io::Result<bool> {
            validate_relative(source)?;
            validate_relative(destination)?;
            self.run_before_operation_hook();
            let source_parent = self.open_parent(source, false)?;
            let destination_parent = self.open_parent(destination, false)?;
            let source_metadata = statat(
                &source_parent.fd,
                &source_parent.name,
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(std::io::Error::from)?;
            if FileType::from_raw_mode(source_metadata.st_mode) != FileType::RegularFile {
                return Err(unsafe_entry("refusing to publish a non-regular object"));
            }
            match linkat(
                &source_parent.fd,
                &source_parent.name,
                &destination_parent.fd,
                &destination_parent.name,
                AtFlags::empty(),
            ) {
                Ok(()) => {}
                Err(Errno::EXIST) => return Ok(false),
                Err(error) => return Err(std::io::Error::from(error)),
            }
            let published = statat(
                &destination_parent.fd,
                &destination_parent.name,
                AtFlags::SYMLINK_NOFOLLOW,
            );
            let published_is_source = published.is_ok_and(|metadata| {
                FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile
                    && metadata.st_dev == source_metadata.st_dev
                    && metadata.st_ino == source_metadata.st_ino
            });
            if !published_is_source {
                let _ = unlinkat(
                    &destination_parent.fd,
                    &destination_parent.name,
                    AtFlags::empty(),
                );
                let _ = fsync(&destination_parent.fd);
                return Err(unsafe_entry("published object identity changed"));
            }
            fsync(&destination_parent.fd).map_err(std::io::Error::from)?;
            Ok(true)
        }

        fn open_parent(&self, path: &Path, create: bool) -> std::io::Result<OpenedParent> {
            validate_relative(path)?;
            let name = path
                .file_name()
                .ok_or_else(|| invalid_path("path must name an object"))?
                .to_os_string();
            let parent = path.parent().unwrap_or_else(|| Path::new(""));
            let fd = self.open_directory(parent, create)?;
            Ok(OpenedParent { fd, name })
        }

        fn open_directory(&self, path: &Path, create: bool) -> std::io::Result<OwnedFd> {
            if path.as_os_str().is_empty() {
                // A dup shares the directory cursor with the pinned root. Enumeration
                // needs its own open file description so retries and concurrent lists
                // always start at the beginning of the same anchored directory.
                return openat2(
                    &self.inner.root,
                    ".",
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    Mode::empty(),
                    self.inner.resolve,
                )
                .map_err(std::io::Error::from);
            }
            validate_relative(path)?;
            if !create {
                return openat2(
                    &self.inner.root,
                    path,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    Mode::empty(),
                    self.inner.resolve,
                )
                .map_err(std::io::Error::from);
            }

            let mut current = dup(&self.inner.root).map_err(std::io::Error::from)?;
            for component in path.components() {
                let Component::Normal(component) = component else {
                    return Err(invalid_path("path contains a non-normal component"));
                };
                current = match openat2(
                    &current,
                    component,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                    Mode::empty(),
                    self.inner.resolve,
                ) {
                    Ok(directory) => directory,
                    Err(Errno::NOENT) => {
                        match mkdirat(&current, component, Mode::from_raw_mode(0o700)) {
                            Ok(()) | Err(Errno::EXIST) => {
                                fsync(&current).map_err(std::io::Error::from)?;
                            }
                            Err(error) => return Err(std::io::Error::from(error)),
                        }
                        openat2(
                            &current,
                            component,
                            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                            Mode::empty(),
                            self.inner.resolve,
                        )
                        .map_err(std::io::Error::from)?
                    }
                    Err(error) => return Err(std::io::Error::from(error)),
                };
            }
            Ok(current)
        }

        #[cfg(test)]
        pub(crate) fn inject_before_operation_once(&self, hook: impl FnOnce() + Send + 'static) {
            let previous = self
                .inner
                .before_operation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .replace(Box::new(hook));
            assert!(
                previous.is_none(),
                "a rooted filesystem hook is already set"
            );
        }

        fn run_before_operation_hook(&self) {
            #[cfg(test)]
            if let Some(hook) = self
                .inner
                .before_operation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                hook();
            }
        }
    }

    fn validate_relative(path: &Path) -> std::io::Result<()> {
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(invalid_path(
                "path must contain only normal relative components",
            ));
        }
        Ok(())
    }

    fn require_regular(fd: &OwnedFd) -> std::io::Result<()> {
        let metadata = fstat(fd).map_err(std::io::Error::from)?;
        if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile {
            return Err(unsafe_entry("object is not a regular file"));
        }
        Ok(())
    }

    fn invalid_path(message: &'static str) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
    }

    fn unsafe_entry(message: &'static str) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, message)
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use std::path::Path;

    use super::RootEntryKind;

    #[derive(Clone, Debug)]
    pub(crate) struct RootedFs;

    impl RootedFs {
        pub(crate) fn new(_root_path: &Path) -> std::io::Result<Self> {
            Err(unsupported())
        }

        pub(crate) fn ensure_dir(&self, _path: &Path) -> std::io::Result<()> {
            Err(unsupported())
        }

        pub(crate) fn require_directory(&self, _path: &Path) -> std::io::Result<()> {
            Err(unsupported())
        }

        pub(crate) fn create_new(&self, _path: &Path) -> std::io::Result<tokio::fs::File> {
            Err(unsupported())
        }

        pub(crate) fn create_new_std(&self, _path: &Path) -> std::io::Result<std::fs::File> {
            Err(unsupported())
        }

        pub(crate) fn open_read(&self, _path: &Path) -> std::io::Result<tokio::fs::File> {
            Err(unsupported())
        }

        pub(crate) fn open_read_std(&self, _path: &Path) -> std::io::Result<std::fs::File> {
            Err(unsupported())
        }

        pub(crate) fn open_lock_file(&self, _path: &Path) -> std::io::Result<std::fs::File> {
            Err(unsupported())
        }

        pub(crate) fn lock_exclusive(_file: &std::fs::File) -> std::io::Result<()> {
            Err(unsupported())
        }

        pub(crate) fn sync_parent(&self, _path: &Path) -> std::io::Result<()> {
            Err(unsupported())
        }

        pub(crate) fn list_regular_files(
            &self,
            _directory: &Path,
        ) -> std::io::Result<Vec<std::ffi::OsString>> {
            Err(unsupported())
        }

        #[cfg(test)]
        pub(crate) fn list_entries(
            &self,
            directory: &Path,
        ) -> std::io::Result<Vec<(std::ffi::OsString, RootEntryKind)>> {
            self.list_entries_bounded(directory, usize::MAX)
        }

        pub(crate) fn list_entries_bounded(
            &self,
            _directory: &Path,
            _max_entries: usize,
        ) -> std::io::Result<Vec<(std::ffi::OsString, RootEntryKind)>> {
            Err(unsupported())
        }

        pub(crate) fn remove_file(&self, _path: &Path) -> std::io::Result<bool> {
            Err(unsupported())
        }

        pub(crate) fn link_no_replace(
            &self,
            _source: &Path,
            _destination: &Path,
        ) -> std::io::Result<bool> {
            Err(unsupported())
        }
    }

    fn unsupported() -> std::io::Error {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "secure media storage requires Linux openat2",
        )
    }
}

pub(crate) use platform::RootedFs;

#[cfg(all(test, target_os = "linux"))]
mod bounded_tests {
    use super::*;

    #[test]
    fn directory_budget_rejects_excess_without_removing_entries() {
        let root = tempfile::tempdir().unwrap();
        for name in ["a", "b", "c"] {
            std::fs::write(root.path().join(name), name).unwrap();
        }
        let fs = RootedFs::new(root.path()).unwrap();
        assert!(
            fs.list_entries_bounded(std::path::Path::new(""), 2)
                .is_err()
        );
        assert_eq!(
            fs.list_entries_bounded(std::path::Path::new(""), 3)
                .unwrap()
                .len(),
            3
        );
        assert_eq!(fs.list_entries(std::path::Path::new("")).unwrap().len(), 3);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 3);
        assert!(
            fs.list_entries_bounded(std::path::Path::new(""), 0)
                .is_err()
        );
    }
}
