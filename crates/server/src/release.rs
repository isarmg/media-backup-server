use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

const MANIFEST_VERSION: u32 = 1;
const PRODUCT: &str = "media-backup-server";
const VERSION: &str = "0.3.18";
const TARGET: &str = sarmg_server_target::SERVER_TARGET_TRIPLE;
const API_VERSION: &str = media_backup_protocol::API_VERSION;
const STORAGE_ENCODING: &str = "plain-v1";
const MANIFEST_FILENAME: &str = "release-manifest.json";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
pub(crate) const PRODUCTION_RELEASE_ROOT: &str = "/opt/isarmg/media-backup/releases/0.3.18";
const RELOCATABLE_RELEASE_SUFFIX: &str = "opt/isarmg/media-backup/releases/0.3.18";

const EXPECTED_DIRECTORIES: &[&str] = &[
    "bin",
    "config",
    "docs",
    "scripts",
    "share",
    "share/web",
    "share/web/assets",
    "systemd",
];

const EXPECTED_FILES: &[(&str, u32)] = &[
    ("LICENSE", 0o644),
    ("bin/media-backup-server", 0o755),
    ("config/media-backup.env.example", 0o644),
    ("docs/feature-inventory-and-tradeoffs.md", 0o644),
    ("README.md", 0o644),
    ("scripts/run-server-wsl.sh", 0o755),
    ("scripts/setup-wsl.sh", 0o755),
    ("scripts/start-server-wsl.sh", 0o755),
    ("scripts/verify-server-wsl.sh", 0o755),
    ("systemd/media-backup.service", 0o644),
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReleaseIdentity {
    pub product: String,
    pub version: String,
    pub source_revision: String,
    pub target: String,
    pub api_version: String,
    pub storage_encoding: String,
    pub server_schema_revision: i64,
    pub server_schema_sha256: String,
    pub web_assets_sha256: String,
    pub release_contract_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseFile {
    path: String,
    mode: u32,
    size: u64,
    sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseManifest {
    manifest_version: u32,
    identity: ReleaseIdentity,
    files: Vec<ReleaseFile>,
}

pub(crate) fn identity() -> ReleaseIdentity {
    let web_assets_sha256 = bundle_sha256(crate::web_assets::RELEASE_FILES);
    let contract = format!(
        "product={PRODUCT}\nversion={VERSION}\napi_version={API_VERSION}\nstorage_encoding={STORAGE_ENCODING}\nserver_schema_revision={}\nserver_schema_sha256={}\nweb_assets_sha256={web_assets_sha256}\n",
        crate::database::CURRENT_SCHEMA_REVISION,
        crate::database::CURRENT_SCHEMA_SHA256,
    );
    ReleaseIdentity {
        product: PRODUCT.to_owned(),
        version: VERSION.to_owned(),
        source_revision: env!("MEDIA_BACKUP_SOURCE_REVISION").to_owned(),
        target: env!("MEDIA_BACKUP_BUILD_TARGET").to_owned(),
        api_version: API_VERSION.to_owned(),
        storage_encoding: STORAGE_ENCODING.to_owned(),
        server_schema_revision: crate::database::CURRENT_SCHEMA_REVISION,
        server_schema_sha256: crate::database::CURRENT_SCHEMA_SHA256.to_owned(),
        web_assets_sha256,
        release_contract_sha256: sha256_hex(contract.as_bytes()),
    }
}

pub(crate) fn identity_json() -> Result<String> {
    Ok(serde_json::to_string(&identity())?)
}

pub(crate) fn verify(root: &Path) -> Result<ReleaseIdentity> {
    verify_with_ownership(root, false)
}

pub(crate) fn verify_installed(root: &Path) -> Result<ReleaseIdentity> {
    verify_with_ownership(root, true)
}

pub(crate) fn verify_runtime(root: &Path) -> Result<ReleaseIdentity> {
    ensure_supported_runtime_host()?;
    let root = validate_runtime_root(root)?;
    let expected_executable = root.join("bin/media-backup-server");
    let executing = fs::canonicalize(
        std::env::current_exe().context("resolve executing server binary for release startup")?,
    )
    .context("resolve physical executing server binary for release startup")?;
    ensure!(
        executing == expected_executable,
        "serve-release must execute the binary physically contained by RELEASE_ROOT"
    );

    if root == Path::new(PRODUCTION_RELEASE_ROOT) {
        for directory in [
            "/opt",
            "/opt/isarmg",
            "/opt/isarmg/media-backup",
            "/opt/isarmg/media-backup/releases",
        ] {
            require_directory(
                Path::new(directory),
                0o755,
                true,
                "production release parent",
            )?;
        }
        verify_with_ownership(&root, true)
    } else {
        verify_with_ownership(&root, false)
    }
}

fn ensure_supported_runtime_host() -> Result<()> {
    let host = rustix::system::uname();
    ensure!(
        host.sysname().to_bytes() == b"Linux" && host.machine().to_bytes() == b"x86_64",
        "formal Media Backup server runtime requires Linux x86_64"
    );
    Ok(())
}

pub(crate) fn ensure_unbound_development_serve() -> Result<()> {
    ensure!(
        env!("MEDIA_BACKUP_SOURCE_REVISION") == "unversioned",
        "a source-bound Media Backup release cannot use ordinary serve; use serve-release RELEASE_ROOT"
    );
    Ok(())
}

fn validate_runtime_root(root: &Path) -> Result<PathBuf> {
    ensure!(root.is_absolute(), "RELEASE_ROOT must be absolute");
    ensure!(
        root.components()
            .all(|component| { matches!(component, Component::RootDir | Component::Normal(_)) }),
        "RELEASE_ROOT must be a normalized absolute path"
    );
    let named = fs::symlink_metadata(root).context("inspect RELEASE_ROOT")?;
    ensure!(
        named.is_dir() && !named.file_type().is_symlink(),
        "RELEASE_ROOT must be a real directory"
    );
    let canonical = fs::canonicalize(root).context("resolve RELEASE_ROOT")?;
    ensure!(
        canonical == root,
        "RELEASE_ROOT and every parent component must be physical and normalized"
    );
    ensure!(
        canonical.ends_with(RELOCATABLE_RELEASE_SUFFIX),
        "RELEASE_ROOT must end in the fixed Media Backup 0.3.18 physical release path"
    );
    Ok(canonical)
}

fn verify_with_ownership(root: &Path, require_root_owned: bool) -> Result<ReleaseIdentity> {
    let root = fs::canonicalize(root).context("resolve release root")?;
    ensure!(
        root.is_absolute(),
        "release root must resolve to an absolute path"
    );
    require_directory(&root, 0o755, require_root_owned, "release root")?;

    let manifest_path = root.join(MANIFEST_FILENAME);
    let manifest_bytes = read_small_regular_file(
        &manifest_path,
        0o644,
        MAX_MANIFEST_BYTES,
        require_root_owned,
        "release manifest",
    )?;
    let manifest: ReleaseManifest =
        serde_json::from_slice(&manifest_bytes).context("parse strict release manifest")?;
    ensure!(
        manifest.manifest_version == MANIFEST_VERSION,
        "unsupported release manifest version"
    );

    let binary_identity = identity();
    validate_identity(&binary_identity)?;
    ensure!(
        manifest.identity == binary_identity,
        "release manifest identity differs from the executing binary"
    );

    let expected_modes: BTreeMap<&str, u32> = EXPECTED_FILES
        .iter()
        .copied()
        .chain(
            crate::web_assets::RELEASE_FILES
                .iter()
                .map(|(path, _)| (*path, 0o644)),
        )
        .collect();
    ensure!(
        manifest.files.len() == expected_modes.len(),
        "release manifest file count differs from the current contract"
    );

    // Reject foreign paths before hashing the complete embedded font bundle.
    // This keeps malformed-layout rejection bounded as the CJK inventory grows.
    let (actual_directories, actual_files) = collect_layout(&root, require_root_owned)?;
    let expected_directories: BTreeSet<String> = EXPECTED_DIRECTORIES
        .iter()
        .map(|path| (*path).to_owned())
        .collect();
    let mut expected_actual_files: BTreeSet<String> = expected_modes
        .keys()
        .map(|path| (*path).to_owned())
        .collect();
    expected_actual_files.insert(MANIFEST_FILENAME.to_owned());
    ensure!(
        actual_directories == expected_directories,
        "release contains missing or extra directories"
    );
    ensure!(
        actual_files == expected_actual_files,
        "release contains missing or extra files"
    );

    let mut previous_path: Option<&str> = None;
    let mut declared_paths = BTreeSet::new();
    let mut binary_record = None;
    for entry in &manifest.files {
        validate_relative_path(&entry.path)?;
        if let Some(previous) = previous_path {
            ensure!(
                previous < entry.path.as_str(),
                "release manifest files must be unique and sorted by path"
            );
        }
        previous_path = Some(&entry.path);
        let expected_mode = expected_modes
            .get(entry.path.as_str())
            .context("release manifest contains an unexpected file")?;
        ensure!(
            entry.mode == *expected_mode,
            "release manifest mode differs from the current layout for {}",
            entry.path
        );
        ensure!(
            is_lower_sha256(&entry.sha256),
            "release manifest contains an invalid SHA-256 for {}",
            entry.path
        );
        let snapshot = hash_regular_file(
            &root.join(&entry.path),
            entry.mode,
            require_root_owned,
            &format!("release file {}", entry.path),
        )?;
        ensure!(
            snapshot.size == entry.size && snapshot.sha256 == entry.sha256,
            "release file size or SHA-256 mismatch for {}",
            entry.path
        );
        if entry.path == "bin/media-backup-server" {
            binary_record = Some((entry.size, entry.sha256.clone()));
        }
        declared_paths.insert(entry.path.clone());
    }
    ensure!(
        declared_paths
            == expected_modes
                .keys()
                .map(|path| (*path).to_owned())
                .collect(),
        "release manifest does not contain the exact current file set"
    );

    let web_files = crate::web_assets::RELEASE_FILES
        .iter()
        .map(|(path, _)| {
            read_small_regular_file(
                &root.join(path),
                0o644,
                4 * 1024 * 1024,
                require_root_owned,
                "admin Web asset",
            )
            .map(|bytes| (*path, bytes))
        })
        .collect::<Result<Vec<_>>>()?;
    let web_assets_sha256 = bundle_sha256(
        &web_files
            .iter()
            .map(|(path, bytes)| (*path, bytes.as_slice()))
            .collect::<Vec<_>>(),
    );
    ensure!(
        web_assets_sha256 == binary_identity.web_assets_sha256,
        "release web assets differ from the binary embedded-assets fingerprint"
    );

    let (binary_size, binary_sha256) = binary_record.context("manifest omits server binary")?;
    let executing = hash_unconstrained_regular_file(
        &std::env::current_exe().context("resolve executing server binary")?,
        "executing server binary",
    )?;
    ensure!(
        executing.size == binary_size && executing.sha256 == binary_sha256,
        "release verifier is not the binary declared by the manifest"
    );

    Ok(binary_identity)
}

pub(crate) fn verification_line(identity: &ReleaseIdentity) -> String {
    format!(
        "MEDIA_BACKUP_RELEASE_VERIFIED_V1\t{}\t{}\t{}\t{}\t{}",
        identity.product,
        identity.version,
        identity.source_revision,
        identity.target,
        identity.release_contract_sha256,
    )
}

fn validate_identity(identity: &ReleaseIdentity) -> Result<()> {
    ensure!(
        identity.product == PRODUCT,
        "release product identity mismatch"
    );
    ensure!(
        identity.version == VERSION,
        "release version identity mismatch"
    );
    ensure!(
        env!("CARGO_PKG_VERSION") == VERSION,
        "Cargo package version differs from the fixed release identity"
    );
    ensure!(
        identity.target == TARGET,
        "unsupported server release target"
    );
    ensure!(
        identity.api_version == API_VERSION,
        "release API identity mismatch"
    );
    ensure!(
        identity.storage_encoding == STORAGE_ENCODING,
        "release storage identity mismatch"
    );
    ensure!(
        identity.server_schema_revision == crate::database::CURRENT_SCHEMA_REVISION,
        "release server schema revision mismatch"
    );
    ensure!(
        identity.server_schema_sha256 == crate::database::CURRENT_SCHEMA_SHA256,
        "release server schema fingerprint mismatch"
    );
    ensure!(
        is_source_revision(&identity.source_revision),
        "release source revision must be 40 lowercase hexadecimal characters"
    );
    for (label, hash) in [
        ("schema", identity.server_schema_sha256.as_str()),
        ("web assets", identity.web_assets_sha256.as_str()),
        (
            "release contract",
            identity.release_contract_sha256.as_str(),
        ),
    ] {
        ensure!(is_lower_sha256(hash), "invalid {label} SHA-256 identity");
    }
    Ok(())
}

fn bundle_sha256(entries: &[(&str, &[u8])]) -> String {
    let mut hasher = Sha256::new();
    for (path, bytes) in entries {
        hasher.update((path.len() as u64).to_be_bytes());
        hasher.update(path.as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    format!("{:x}", hasher.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_source_revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_relative_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    ensure!(
        !value.is_empty() && !path.is_absolute(),
        "invalid release file path"
    );
    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "release file path contains traversal or non-normal components"
    );
    ensure!(
        path.to_str() == Some(value) && !value.contains('\\'),
        "release file path must be normalized UTF-8 with slash separators"
    );
    Ok(())
}

#[derive(Debug)]
struct FileHash {
    size: u64,
    sha256: String,
}

fn hash_regular_file(
    path: &Path,
    mode: u32,
    require_root_owned: bool,
    label: &str,
) -> Result<FileHash> {
    let mut file = open_regular_file(path, Some(mode), require_root_owned, label)?;
    let size = file.metadata()?.len();
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    revalidate_open_file(path, &file, size, label)?;
    Ok(FileHash {
        size,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn hash_unconstrained_regular_file(path: &Path, label: &str) -> Result<FileHash> {
    let mut file = open_regular_file(path, None, false, label)?;
    let size = file.metadata()?.len();
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    revalidate_open_file(path, &file, size, label)?;
    Ok(FileHash {
        size,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn read_small_regular_file(
    path: &Path,
    mode: u32,
    limit: u64,
    require_root_owned: bool,
    label: &str,
) -> Result<Vec<u8>> {
    let mut file = open_regular_file(path, Some(mode), require_root_owned, label)?;
    let size = file.metadata()?.len();
    ensure!(size <= limit, "{label} exceeds its size limit");
    let mut bytes = Vec::with_capacity(size as usize);
    file.read_to_end(&mut bytes)?;
    revalidate_open_file(path, &file, size, label)?;
    Ok(bytes)
}

fn open_regular_file(
    path: &Path,
    expected_mode: Option<u32>,
    require_root_owned: bool,
    label: &str,
) -> Result<File> {
    let named = fs::symlink_metadata(path).with_context(|| format!("inspect {label}"))?;
    ensure!(
        named.is_file() && !named.file_type().is_symlink(),
        "{label} must be a regular non-symlink file"
    );
    #[cfg(unix)]
    {
        ensure!(
            named.nlink() == 1,
            "{label} must not have hard-link aliases"
        );
        if require_root_owned {
            ensure!(
                named.uid() == 0 && named.gid() == 0,
                "{label} must be owned by root"
            );
        }
        if let Some(mode) = expected_mode {
            ensure!(
                named.permissions().mode() & 0o7777 == mode,
                "{label} has an unexpected mode"
            );
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .with_context(|| format!("open {label}"))?;
    #[cfg(unix)]
    {
        let opened = file.metadata()?;
        if let Some(mode) = expected_mode {
            ensure!(
                opened.permissions().mode() & 0o7777 == mode,
                "{label} changed mode while it was opened"
            );
        }
        if require_root_owned {
            ensure!(
                opened.uid() == 0 && opened.gid() == 0,
                "{label} changed ownership while it was opened"
            );
        }
    }
    revalidate_open_file(path, &file, named.len(), label)?;
    Ok(file)
}

fn revalidate_open_file(path: &Path, file: &File, expected_size: u64, label: &str) -> Result<()> {
    let opened = file.metadata()?;
    let named = fs::symlink_metadata(path)?;
    ensure!(
        opened.is_file()
            && named.is_file()
            && !named.file_type().is_symlink()
            && opened.len() == expected_size
            && named.len() == expected_size,
        "{label} changed while it was verified"
    );
    #[cfg(unix)]
    ensure!(
        opened.dev() == named.dev()
            && opened.ino() == named.ino()
            && opened.nlink() == 1
            && named.nlink() == 1
            && opened.mtime() == named.mtime()
            && opened.mtime_nsec() == named.mtime_nsec(),
        "{label} changed identity while it was verified"
    );
    Ok(())
}

fn require_directory(path: &Path, mode: u32, require_root_owned: bool, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).with_context(|| format!("inspect {label}"))?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{label} must be a real directory"
    );
    #[cfg(unix)]
    {
        ensure!(
            metadata.permissions().mode() & 0o7777 == mode,
            "{label} has an unexpected mode"
        );
        if require_root_owned {
            ensure!(
                metadata.uid() == 0 && metadata.gid() == 0,
                "{label} must be owned by root"
            );
        }
    }
    Ok(())
}

fn collect_layout(
    root: &Path,
    require_root_owned: bool,
) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let mut directories = BTreeSet::new();
    let mut files = BTreeSet::new();
    collect_layout_at(
        root,
        Path::new(""),
        require_root_owned,
        &mut directories,
        &mut files,
    )?;
    Ok((directories, files))
}

fn collect_layout_at(
    root: &Path,
    relative: &Path,
    require_root_owned: bool,
    directories: &mut BTreeSet<String>,
    files: &mut BTreeSet<String>,
) -> Result<()> {
    let directory = root.join(relative);
    let mut entries = fs::read_dir(&directory)
        .with_context(|| format!("read release directory {}", relative.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("release contains a non-UTF-8 path"))?;
        ensure!(name != "." && name != "..", "invalid release entry name");
        let child = relative.join(name);
        let child_string = child
            .to_str()
            .context("release path is not UTF-8")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "release contains a symbolic link: {child_string}"
        );
        if metadata.is_dir() {
            require_directory(
                &entry.path(),
                0o755,
                require_root_owned,
                &format!("release directory {child_string}"),
            )?;
            directories.insert(child_string);
            collect_layout_at(root, &child, require_root_owned, directories, files)?;
        } else if metadata.is_file() {
            files.insert(child_string);
        } else {
            bail!("release contains a special file: {child_string}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn identity_is_exact_and_contract_fingerprint_is_stable() {
        let identity = identity();
        assert_eq!(identity.product, PRODUCT);
        assert_eq!(identity.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(identity.target, env!("MEDIA_BACKUP_BUILD_TARGET"));
        assert_eq!(identity.api_version, media_backup_protocol::API_VERSION);
        assert_eq!(identity.storage_encoding, "plain-v1");
        assert_eq!(identity.server_schema_revision, 5);
        assert_eq!(
            identity.web_assets_sha256,
            "b306e7abaf157397614932b8d7399e669e3b9a59748b71f1bcfd6524a3967b30"
        );
        assert_eq!(
            identity.release_contract_sha256,
            "caa062726e5eda07d930584c8e26e009c9ad0e692d765b26783e58f7c5e21128"
        );
    }

    #[test]
    fn manifest_and_nested_identity_deny_unknown_fields() {
        let identity_value = serde_json::to_value(identity()).unwrap();
        let mut manifest = json!({
            "manifest_version": MANIFEST_VERSION,
            "identity": identity_value,
            "files": [],
        });
        manifest["unexpected"] = json!(true);
        assert!(serde_json::from_value::<ReleaseManifest>(manifest).is_err());

        let mut identity_value = serde_json::to_value(identity()).unwrap();
        identity_value["unknown_version_field"] = json!("noncurrent-version");
        let manifest = json!({
            "manifest_version": MANIFEST_VERSION,
            "identity": identity_value,
            "files": [],
        });
        assert!(serde_json::from_value::<ReleaseManifest>(manifest).is_err());
    }

    #[test]
    fn release_paths_and_hashes_are_canonical() {
        assert!(validate_relative_path("bin/media-backup-server").is_ok());
        assert!(validate_relative_path("../media-backup-server").is_err());
        assert!(validate_relative_path("/bin/media-backup-server").is_err());
        assert!(validate_relative_path("bin\\media-backup-server").is_err());
        assert!(is_source_revision(
            "0123456789abcdef0123456789abcdef01234567"
        ));
        assert!(!is_source_revision("v0.3.18"));
        assert!(is_lower_sha256(&"a".repeat(64)));
        assert!(!is_lower_sha256(&"A".repeat(64)));
    }
}
