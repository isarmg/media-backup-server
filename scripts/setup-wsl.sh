#!/usr/bin/env bash
set -euo pipefail

readonly product="media-backup-server"
readonly version="0.3.6"
readonly target="x86_64-unknown-linux-gnu"
readonly release_contract_sha256="5fa62beb0bef2a9234f8707c146f24546fc6a60391ac92473042335a40557257"
readonly service_user="isarmg-media"
readonly service_group="isarmg-media"
readonly app_dir="/opt/isarmg/media-backup"
readonly releases_dir="$app_dir/releases"
readonly state_dir="/var/lib/isarmg/media-backup"
readonly config_file="/etc/isarmg/media-backup.env"
readonly unit_file="/etc/systemd/system/media-backup.service"
readonly initial_secret_marker="# INITIAL-SECRETS-MUST-BE-REPLACED"

setup_root="${MEDIA_BACKUP_SETUP_ROOT:-/}"
test_mode="${MEDIA_BACKUP_SETUP_TEST:-0}"
release_staging=""
unit_staging=""

die() {
  printf 'setup error: %s\n' "$*" >&2
  exit 1
}

[[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]] ||
  die "formal server installation requires Linux x86_64"

rooted() {
  if [[ "$setup_root" == "/" ]]; then
    printf '%s\n' "$1"
  else
    printf '%s%s\n' "$setup_root" "$1"
  fi
}

cleanup() {
  if [[ -n "$unit_staging" && -f "$unit_staging" && ! -L "$unit_staging" ]]; then
    case "$unit_staging" in
      "$(rooted "/etc/systemd/system")"/.media-backup.service.install-*) rm -f -- "$unit_staging" ;;
      *) printf 'setup error: refusing to clean unexpected unit staging path\n' >&2 ;;
    esac
  fi
  if [[ -n "$release_staging" && -d "$release_staging" && ! -L "$release_staging" ]]; then
    case "$release_staging" in
      "$(rooted "$releases_dir")"/.install-*) rm -rf -- "$release_staging" ;;
      *) printf 'setup error: refusing to clean unexpected staging path\n' >&2 ;;
    esac
  fi
}
trap cleanup EXIT

[[ "$test_mode" == "0" || "$test_mode" == "1" ]] ||
  die "MEDIA_BACKUP_SETUP_TEST must be 0 or 1"
if [[ "$test_mode" == "1" ]]; then
  [[ "$setup_root" != "/" ]] || die "test mode refuses the real filesystem root"
else
  [[ "$setup_root" == "/" ]] || die "an alternate root is allowed only in test mode"
  [[ "$EUID" -eq 0 ]] || die "run this setup script as root (or with sudo)"
fi

[[ "$setup_root" = /* ]] || die "MEDIA_BACKUP_SETUP_ROOT must be absolute"
[[ -d "$setup_root" && ! -L "$setup_root" ]] || die "setup root must be a real directory"
setup_root="$(cd "$setup_root" && pwd -P)"
shopt -s nullglob dotglob

invoked_script="${BASH_SOURCE[0]}"
[[ "$invoked_script" = /* ]] || invoked_script="$PWD/$invoked_script"
[[ -f "$invoked_script" && ! -L "$invoked_script" ]] ||
  die "setup must be run from a regular file in an extracted release"
script_dir="$(cd "$(dirname "$invoked_script")" && pwd -P)"
release_source_dir="$(cd "$script_dir/.." && pwd -P)"
[[ "$script_dir" == "$release_source_dir/scripts" ]] ||
  die "setup is not located in the release scripts directory"

ensure_directory_chain() {
  local logical_path="$1"
  local prefix=""
  local actual
  local component
  local -a components
  [[ "$logical_path" = /* && "$logical_path" != *".."* ]] ||
    die "invalid installation directory path"
  IFS='/' read -r -a components <<<"${logical_path#/}"
  for component in "${components[@]}"; do
    [[ -n "$component" && "$component" != "." && "$component" != ".." ]] ||
      die "invalid installation directory component"
    prefix="$prefix/$component"
    actual="$(rooted "$prefix")"
    if [[ -e "$actual" || -L "$actual" ]]; then
      [[ -d "$actual" && ! -L "$actual" ]] ||
        die "installation directory chain contains a symlink or special entry: $prefix"
    else
      mkdir -m 0755 -- "$actual"
    fi
  done
}

validate_existing_directory_chain() {
  local logical_path="$1"
  local prefix=""
  local actual
  local component
  local -a components
  [[ "$logical_path" = /* && "$logical_path" != *".."* ]] ||
    die "invalid installation directory path"
  IFS='/' read -r -a components <<<"${logical_path#/}"
  for component in "${components[@]}"; do
    prefix="$prefix/$component"
    actual="$(rooted "$prefix")"
    if [[ ! -e "$actual" && ! -L "$actual" ]]; then
      return
    fi
    [[ -d "$actual" && ! -L "$actual" ]] ||
      die "installation directory chain contains a symlink or special entry: $prefix"
  done
}

ensure_single_link_regular_file() {
  local path="$1"
  local label="$2"
  [[ -f "$path" && ! -L "$path" ]] || die "$label must be a regular file"
  [[ "$(stat -c '%h' -- "$path")" == "1" ]] || die "$label must not have hard-link aliases"
}

ensure_root_owned_directory() {
  local path="$1"
  local label="$2"
  local mode
  [[ "$(stat -c '%u:%g' -- "$path")" == "0:0" ]] || die "$label must be owned by root"
  mode="$(stat -c '%a' -- "$path")"
  (( (8#$mode & 0022) == 0 )) || die "$label must not be writable by group or other"
}

# Run this independent pass before any payload binary. It rejects replaced fake
# binaries, extended manifests, and every missing, extra, linked, special, mode-
# changed, size-changed, or hash-changed payload entry.
validate_manifest_and_payload() {
  local release_path="$1"
  python3 - "$release_path" <<'PY'
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys

root = Path(sys.argv[1])
expected_identity_keys = {
    "product", "version", "source_revision", "target", "api_version",
    "storage_encoding", "server_schema_revision", "server_schema_sha256",
    "web_assets_sha256",
    "release_contract_sha256",
}
expected_identity = {
    "product": "media-backup-server",
    "version": "0.3.6",
    "target": "x86_64-unknown-linux-gnu",
    "api_version": "v2",
    "storage_encoding": "plain-v1",
    "server_schema_revision": 3,
    "server_schema_sha256": "d65bf1183bc5bf3546738226c49711dbdbd520c5120a18df075273d5904bf51e",
    "web_assets_sha256": "8a56e8105830e45516c527c6ebec2bc998fc9b92c0f6a274f0102bac36863ee1",
    "release_contract_sha256": "5fa62beb0bef2a9234f8707c146f24546fc6a60391ac92473042335a40557257",
}
expected_directories = {
    "bin", "config", "docs", "scripts", "share", "share/web",
    "share/web/assets", "systemd",
}
expected_files = {
    "LICENSE": 0o644,
    "bin/media-backup-server": 0o755,
    "config/media-backup.env.example": 0o644,
    "docs/feature-inventory-and-tradeoffs.md": 0o644,
    "README.md": 0o644,
    "scripts/run-server-wsl.sh": 0o755,
    "scripts/setup-wsl.sh": 0o755,
    "scripts/start-server-wsl.sh": 0o755,
    "scripts/verify-server-wsl.sh": 0o755,
    "share/web/index.html": 0o644,
    "share/web/assets/admin.js": 0o644,
    "share/web/assets/admin.css": 0o644,
    "share/web/assets/CJK-LICENSE.txt": 0o644,
    "share/web/assets/MapleMonoBootstrap-Regular.woff2": 0o644,
    "share/web/assets/MapleMonoBootstrap-Bold.woff2": 0o644,
    "share/web/assets/MapleMonoNormalNL-Regular.woff2": 0o644,
    "share/web/assets/MapleMonoNormalNL-Bold.woff2": 0o644,
    "share/web/assets/MapleMono-OFL.txt": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-4D4-50CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-50CF-52CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-52CF-54CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-54CF-56CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-56CF-58CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-58CF-5ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-5ACF-5CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-5CCF-5ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-5ECF-60CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-60CF-62CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-62CF-64CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-64CF-66CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-66CF-68CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-68CF-6ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-6ACF-6CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-6CCF-6ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-6ECF-70CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-70CF-72CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-72CF-74CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-74CF-76CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-76CF-78CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-78CF-7ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-7ACF-7CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-7CCF-7ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-7ECF-80CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-80CF-82CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-82CF-84CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-84CF-86CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-86CF-88CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-88CF-8ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-8ACF-8CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-8CCF-8ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-8ECF-90CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-90CF-92CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-92CF-94CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-94CF-96CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-96CF-98CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-98CF-9ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-9ACF-9CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-9CCF-9E67.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-9E68-FFEE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-4D4-50CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-50CF-52CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-52CF-54CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-54CF-56CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-56CF-58CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-58CF-5ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-5ACF-5CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-5CCF-5ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-5ECF-60CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-60CF-62CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-62CF-64CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-64CF-66CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-66CF-68CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-68CF-6ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-6ACF-6CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-6CCF-6ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-6ECF-70CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-70CF-72CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-72CF-74CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-74CF-76CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-76CF-78CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-78CF-7ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-7ACF-7CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-7CCF-7ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-7ECF-80CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-80CF-82CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-82CF-84CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-84CF-86CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-86CF-88CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-88CF-8ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-8ACF-8CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-8CCF-8ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-8ECF-90CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-90CF-92CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-92CF-94CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-94CF-96CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-96CF-98CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-98CF-9ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-9ACF-9CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-9CCF-9E67.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-9E68-FFEE.woff2": 0o644,
    "systemd/media-backup.service": 0o644,
}

def fail(message):
    raise SystemExit("independent release verification failed: " + message)

def metadata(path, mode, label):
    try:
        value = path.lstat()
    except OSError as error:
        fail(f"cannot inspect {label}: {error}")
    if not stat.S_ISREG(value.st_mode) or path.is_symlink():
        fail(f"{label} is not a regular non-symlink file")
    if value.st_nlink != 1:
        fail(f"{label} has a hard-link alias")
    if stat.S_IMODE(value.st_mode) != mode:
        fail(f"{label} has the wrong mode")
    return value

try:
    root_metadata = root.lstat()
except OSError as error:
    fail(f"cannot inspect release root: {error}")
if not root.is_absolute() or root.is_symlink() or not stat.S_ISDIR(root_metadata.st_mode):
    fail("release root must be an absolute real directory")
if stat.S_IMODE(root_metadata.st_mode) != 0o755:
    fail("release root has the wrong mode")

manifest_path = root / "release-manifest.json"
manifest_metadata = metadata(manifest_path, 0o644, "release manifest")
if manifest_metadata.st_size > 1024 * 1024:
    fail("release manifest exceeds its size limit")
try:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
except (OSError, UnicodeError, json.JSONDecodeError) as error:
    fail(f"cannot parse release manifest: {error}")
if not isinstance(manifest, dict) or set(manifest) != {"manifest_version", "identity", "files"}:
    fail("release manifest has an unknown or missing field")
if type(manifest["manifest_version"]) is not int or manifest["manifest_version"] != 1:
    fail("release manifest version is not 1")
identity = manifest["identity"]
if not isinstance(identity, dict) or set(identity) != expected_identity_keys:
    fail("release identity has an unknown or missing field")
for field, expected in expected_identity.items():
    if identity.get(field) != expected or type(identity.get(field)) is not type(expected):
        fail(f"release identity mismatch for {field}")
revision = identity.get("source_revision")
if not isinstance(revision, str) or re.fullmatch(r"[0-9a-f]{40}", revision) is None:
    fail("source revision is not 40 lowercase hexadecimal characters")
for field in ("server_schema_sha256", "web_assets_sha256", "release_contract_sha256"):
    if re.fullmatch(r"[0-9a-f]{64}", identity[field]) is None:
        fail(f"invalid SHA-256 identity field {field}")

files = manifest["files"]
if not isinstance(files, list) or len(files) != len(expected_files):
    fail("manifest file count differs from the current contract")
declared = []
for entry in files:
    if not isinstance(entry, dict) or set(entry) != {"path", "mode", "size", "sha256"}:
        fail("manifest file entry has an unknown or missing field")
    relative = entry["path"]
    if not isinstance(relative, str) or relative not in expected_files:
        fail("manifest contains an unexpected file")
    if type(entry["mode"]) is not int or entry["mode"] != expected_files[relative]:
        fail(f"manifest mode mismatch for {relative}")
    if type(entry["size"]) is not int or entry["size"] < 0:
        fail(f"manifest size is invalid for {relative}")
    digest = entry["sha256"]
    if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
        fail(f"manifest SHA-256 is invalid for {relative}")
    value = metadata(root / relative, expected_files[relative], relative)
    hasher = hashlib.sha256()
    with (root / relative).open("rb") as source:
        while chunk := source.read(65536):
            hasher.update(chunk)
    if value.st_size != entry["size"] or hasher.hexdigest() != digest:
        fail(f"payload size or SHA-256 mismatch for {relative}")
    declared.append(relative)
if declared != sorted(expected_files):
    fail("manifest files are not the exact sorted current file set")

actual_directories = set()
actual_files = set()
for directory, directory_names, file_names in os.walk(root, topdown=True, followlinks=False):
    relative_directory = Path(directory).relative_to(root)
    for name in directory_names:
        child = Path(directory) / name
        value = child.lstat()
        relative = (relative_directory / name).as_posix()
        if child.is_symlink() or not stat.S_ISDIR(value.st_mode):
            fail(f"release contains a linked or special directory: {relative}")
        if stat.S_IMODE(value.st_mode) != 0o755:
            fail(f"release directory has the wrong mode: {relative}")
        actual_directories.add(relative)
    for name in file_names:
        child = Path(directory) / name
        value = child.lstat()
        relative = (relative_directory / name).as_posix()
        if child.is_symlink() or not stat.S_ISREG(value.st_mode):
            fail(f"release contains a linked or special file: {relative}")
        actual_files.add(relative)
if actual_directories != expected_directories:
    fail("release has missing or extra directories")
if actual_files != set(expected_files) | {"release-manifest.json"}:
    fail("release has missing or extra files")
PY
}

verified_revision=""
verify_release() {
  local release_path="$1"
  local ownership_mode="$2"
  local binary="$release_path/bin/media-backup-server"
  local command="release-verify"
  local output
  local marker
  local output_product
  local output_version
  local output_revision
  local output_target
  local output_contract
  local extra

  validate_manifest_and_payload "$release_path" || die "release manifest or payload is invalid"
  if [[ "$ownership_mode" == "installed" ]]; then
    command="release-verify-installed"
  fi
  output="$("$binary" "$command" "$release_path")" || die "release binary rejected its manifest"
  [[ "$output" != *$'\n'* ]] || die "release verifier returned multiple lines"
  IFS=$'\t' read -r marker output_product output_version output_revision output_target output_contract extra <<<"$output"
  [[ -z "${extra:-}" && "$marker" == "MEDIA_BACKUP_RELEASE_VERIFIED_V1" &&
    "$output_product" == "$product" && "$output_version" == "$version" &&
    "$output_revision" =~ ^[0-9a-f]{40}$ && "$output_target" == "$target" &&
    "$output_contract" == "$release_contract_sha256" ]] ||
    die "release verifier returned an unexpected 0.2 identity"
  verified_revision="$output_revision"
}

validate_empty_release_destination() {
  local app_path
  local releases_path
  local entry
  local -a entries
  app_path="$(rooted "$app_dir")"
  releases_path="$(rooted "$releases_dir")"

  if [[ ! -e "$app_path" && ! -L "$app_path" ]]; then
    return
  fi
  [[ -d "$app_path" && ! -L "$app_path" ]] || die "application destination must be a real directory"
  entries=("$app_path"/*)
  for entry in "${entries[@]}"; do
    [[ "$entry" == "$releases_path" ]] ||
      die "application destination contains an unexpected pre-existing entry"
  done
  if [[ -e "$releases_path" || -L "$releases_path" ]]; then
    [[ -d "$releases_path" && ! -L "$releases_path" ]] ||
      die "releases destination must be a real directory"
    entries=("$releases_path"/*)
    ((${#entries[@]} == 0)) || die "releases destination is not empty"
  fi
}

random_hex_256() {
  local value
  value="$(LC_ALL=C od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
  [[ "$value" =~ ^[[:xdigit:]]{64}$ ]] || die "failed to generate a 256-bit secret"
  printf '%s\n' "$value"
}

random_base64_256() {
  local value
  value="$(LC_ALL=C head -c 32 /dev/urandom | base64 | tr -d '\n')"
  [[ "$value" =~ ^[A-Za-z0-9+/]{43}=$ ]] || die "failed to generate a Base64 256-bit secret"
  printf '%s\n' "$value"
}

verify_release "$release_source_dir" archive
source_revision="$verified_revision"

# Reject incompatible or unsafe pre-existing targets before creating any install,
# state, configuration, or systemd directory.
for logical_directory in "$releases_dir" "/etc/isarmg" "/etc/systemd/system" \
  "$state_dir/db" "$state_dir/data"; do
  validate_existing_directory_chain "$logical_directory"
done
preflight_release_path="$(rooted "$releases_dir/$version")"
preflight_config_path="$(rooted "$config_file")"
preflight_unit_path="$(rooted "$unit_file")"
validate_empty_release_destination
if [[ -e "$preflight_release_path" || -L "$preflight_release_path" ]]; then
  die "release 0.3.6 destination already exists; installation is one-shot and no-clobber"
fi
if [[ -e "$preflight_config_path" || -L "$preflight_config_path" ]]; then
  ensure_single_link_regular_file "$preflight_config_path" "configuration"
fi
if [[ -e "$preflight_unit_path" || -L "$preflight_unit_path" ]]; then
  die "systemd unit destination already exists; refusing to overwrite it"
fi

ensure_directory_chain "$releases_dir"
ensure_directory_chain "/etc/isarmg"
ensure_directory_chain "/etc/systemd/system"
ensure_directory_chain "$state_dir/db"
ensure_directory_chain "$state_dir/data"

releases_path="$(rooted "$releases_dir")"
release_path="$(rooted "$releases_dir/$version")"
state_path="$(rooted "$state_dir")"
config_path="$(rooted "$config_file")"
unit_path="$(rooted "$unit_file")"

if [[ "$test_mode" == "0" ]]; then
  ensure_root_owned_directory /opt "/opt"
  ensure_root_owned_directory /opt/isarmg "/opt/isarmg"
  ensure_root_owned_directory "$(rooted "$app_dir")" "$app_dir"
  ensure_root_owned_directory "$releases_path" "$releases_dir"
  ensure_root_owned_directory /etc "/etc"
  ensure_root_owned_directory /etc/isarmg "/etc/isarmg"
  ensure_root_owned_directory /etc/systemd/system "/etc/systemd/system"
  ensure_root_owned_directory /var/lib/isarmg "/var/lib/isarmg"
fi

[[ ! -e "$release_path" && ! -L "$release_path" ]] ||
  die "release 0.3.6 destination appeared during installation"
if [[ -e "$config_path" || -L "$config_path" ]]; then
  ensure_single_link_regular_file "$config_path" "configuration"
fi
if [[ -e "$unit_path" || -L "$unit_path" ]]; then
  die "systemd unit destination appeared during installation"
fi

if [[ "$test_mode" == "0" ]]; then
  service_home=""
  service_shell=""
  if ! getent group "$service_group" >/dev/null; then
    groupadd --system "$service_group"
  fi
  if ! id -u "$service_user" >/dev/null 2>&1; then
    useradd --system --gid "$service_group" --home-dir "$state_dir" --no-create-home \
      --shell /usr/sbin/nologin "$service_user"
  fi
  [[ "$(id -u "$service_user")" != "0" ]] || die "service account must not be root"
  [[ "$(id -g "$service_user")" == "$(getent group "$service_group" | cut -d: -f3)" ]] ||
    die "existing service account does not use the dedicated group"
  IFS=: read -r _ _ _ _ _ service_home service_shell < <(getent passwd "$service_user")
  [[ "$service_home" == "$state_dir" ]] || die "existing service account has the wrong home"
  [[ "$service_shell" == "/usr/sbin/nologin" || "$service_shell" == "/sbin/nologin" ]] ||
    die "existing service account must use a nologin shell"
fi

chmod 0755 "$(rooted "$app_dir")" "$releases_path"
release_staging="$(mktemp -d -- "$releases_path/.install-$version.XXXXXX")"
chmod 0755 "$release_staging"
cp -a --no-preserve=ownership -- "$release_source_dir/." "$release_staging/"
if [[ "$test_mode" == "0" ]]; then
  chown -R root:root "$release_staging"
  verify_release "$release_staging" installed
else
  verify_release "$release_staging" archive
fi
mv -T -n -- "$release_staging" "$release_path"
[[ ! -e "$release_staging" ]] || die "release destination appeared concurrently"
release_staging=""
[[ -d "$release_path" && ! -L "$release_path" ]] || die "could not install immutable release"
if [[ "$test_mode" == "0" ]]; then
  verify_release "$release_path" installed
else
  verify_release "$release_path" archive
fi
[[ "$verified_revision" == "$source_revision" ]] || die "installed release revision changed during setup"
cmp --silent -- "$release_source_dir/release-manifest.json" "$release_path/release-manifest.json" ||
  die "installed release differs from the supplied immutable archive"

chmod 0750 "$state_path" "$state_path/db" "$state_path/data"
if [[ "$test_mode" == "0" ]]; then
  chown "$service_user:$service_group" "$state_path" "$state_path/db" "$state_path/data"
fi

config_created=0
if [[ -e "$config_path" || -L "$config_path" ]]; then
  ensure_single_link_regular_file "$config_path" "configuration"
else
  admin_password="$(random_hex_256)"
  credentials_key="$(random_base64_256)"
  metrics_token="$(random_hex_256)"
  if (
    umask 077
    set -o noclobber
    printf '%s\n' \
      "$initial_secret_marker" \
      '# Store all generated secrets safely, then remove the marker above before first start.' \
      'DATABASE_URL=sqlite:///var/lib/isarmg/media-backup/db/app.db' \
      'DATA_DIR=/var/lib/isarmg/media-backup/data' \
      'BIND=127.0.0.1:8080' \
      'BOOTSTRAP_ADMIN_USERNAME=admin' \
      "BOOTSTRAP_ADMIN_PASSWORD=$admin_password" \
      "MEDIA_BACKUP_CREDENTIALS_KEY=$credentials_key" \
      'MAX_PART_BYTES=67108864' \
      'REQUIRE_HTTPS=true' \
      'DEVELOPMENT=false' \
      'TRUSTED_PROXY_CIDRS=127.0.0.1/32,::1/128' \
      "METRICS_TOKEN=$metrics_token" \
      'RUST_LOG=media_backup_server=info,tower_http=info' >"$config_path"
  ) 2>/dev/null; then
    config_created=1
  elif [[ ! -f "$config_path" || -L "$config_path" ]]; then
    die "could not create configuration without overwriting an existing path"
  fi
  unset admin_password credentials_key metrics_token
  ensure_single_link_regular_file "$config_path" "configuration"
fi
chmod 0600 "$config_path"
if [[ "$test_mode" == "0" ]]; then
  chown root:root "$config_path"
fi

unit_staging="$(mktemp -- "$(rooted "/etc/systemd/system")/.media-backup.service.install-XXXXXX")"
install -m 0644 "$release_path/systemd/media-backup.service" "$unit_staging"
mv -T -n -- "$unit_staging" "$unit_path"
[[ ! -e "$unit_staging" ]] || die "systemd unit destination appeared concurrently"
unit_staging=""
ensure_single_link_regular_file "$unit_path" "systemd unit"
if [[ "$test_mode" == "0" ]]; then
  chown root:root "$unit_path"
fi

if [[ "$test_mode" == "0" ]]; then
  "$release_path/bin/media-backup-server" release-verify-installed "$release_path" >/dev/null
  systemctl daemon-reload
fi

if [[ "$config_created" == "1" ]]; then
  printf 'Created %s with private random initial secrets; values were not printed.\n' "$config_file"
else
  printf 'Preserved existing %s without changing its contents.\n' "$config_file"
fi
printf 'Before first start, use sudoedit %s to securely record or replace all three generated secrets and remove %s.\n' \
  "$config_file" "$initial_secret_marker"
printf 'Installed Media Backup %s from source revision %s; the service was not started.\n' \
  "$version" "$source_revision"
