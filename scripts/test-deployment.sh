#!/usr/bin/env bash
set -euo pipefail

readonly archive_arg="${1:-${MEDIA_BACKUP_RELEASE_ARCHIVE:-}}"
readonly package="media-backup-server-0.3.16-x86_64-unknown-linux-gnu"
readonly version="0.3.16"
readonly contract="61e7c0ffc3e781c1160a7b70eb249ab43f91a6135f028cb735f16c10bb85ceef"
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly project_dir

test_root=""
server_pid=""

fail() {
  printf 'deployment test failed: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill -TERM "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  if [[ -n "$test_root" && -d "$test_root" && ! -L "$test_root" ]]; then
    chmod -R u+rwX "$test_root" 2>/dev/null || true
    rm -rf -- "$test_root"
  fi
}
trap cleanup EXIT

[[ -n "$archive_arg" ]] ||
  fail "usage: test-deployment.sh /absolute/path/media-backup-server-0.3.16-x86_64-unknown-linux-gnu.tar.gz"
[[ "$archive_arg" = /* && -f "$archive_arg" && ! -L "$archive_arg" ]] ||
  fail "release archive must be an absolute regular non-symlink file"
[[ "$(stat -c '%h' -- "$archive_arg")" == "1" ]] || fail "release archive has a hard-link alias"
archive="$(cd "$(dirname "$archive_arg")" && pwd -P)/$(basename "$archive_arg")"
[[ "$(basename "$archive")" == "$package.tar.gz" ]] || fail "unexpected release archive name"

test_root="$(mktemp -d)"
extract_dir="$test_root/extracted"
mkdir -m 0755 "$extract_dir"
if tar -tzf "$archive" | awk -F/ -v package="$package" '
  $1 != package || $0 ~ /(^|\/)\.\.?(\/|$)/ || $0 ~ /^\// { exit 1 }
'; then
  :
else
  fail "archive contains an unexpected root or non-normal path"
fi
tar --no-same-owner -xzf "$archive" -C "$extract_dir"
relocated_releases="$test_root/relocated/opt/isarmg/media-backup/releases"
mkdir -p "$relocated_releases"
release_root="$relocated_releases/$version"
mv -T -- "$extract_dir/$package" "$release_root"
real_binary="$release_root/bin/media-backup-server"
setup_script="$release_root/scripts/setup-wsl.sh"
unit_source="$release_root/systemd/media-backup.service"
[[ -d "$release_root" && ! -L "$release_root" && -x "$real_binary" ]] ||
  fail "archive does not contain the real server layout"

identity_file="$test_root/identity.json"
env -i PATH="$PATH" "$real_binary" release-identity >"$identity_file"
python3 - "$identity_file" <<'PY'
import json
from pathlib import Path
import re
import sys

identity = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
expected_keys = {
    "product", "version", "source_revision", "target", "api_version",
    "storage_encoding", "server_schema_revision", "server_schema_sha256",
    "web_assets_sha256",
    "release_contract_sha256",
}
assert isinstance(identity, dict) and set(identity) == expected_keys
assert identity["product"] == "media-backup-server"
assert identity["version"] == "0.3.16"
assert re.fullmatch(r"[0-9a-f]{40}", identity["source_revision"])
assert identity["target"] == "x86_64-unknown-linux-gnu"
assert identity["api_version"] == "v2"
assert identity["storage_encoding"] == "plain-v1"
assert identity["server_schema_revision"] == 4
assert identity["server_schema_sha256"] == "84f0e8032d8814b8815b0a6a8a499e0e0d7bb44d37932cf78c1e75bd0b5826fe"
assert identity["web_assets_sha256"] == "db9b3c1a4834afcb178e8984b20e81421ec80c812bad9d535ffb2b155300dd88"
assert identity["release_contract_sha256"] == "61e7c0ffc3e781c1160a7b70eb249ab43f91a6135f028cb735f16c10bb85ceef"
PY
source_revision="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["source_revision"])' "$identity_file")"
verification="$($real_binary release-verify "$release_root")"
[[ "$verification" == MEDIA_BACKUP_RELEASE_VERIFIED_V1$'\tmedia-backup-server\t'"$version"$'\t'"$source_revision"$'\tx86_64-unknown-linux-gnu\t'"$contract" ]] ||
  fail "real binary did not verify the extracted archive identity"
archive_digest="$(sha256sum "$archive" | awk '{print $1}')"
if "$project_dir/scripts/build-server-release.sh" "$real_binary" "$source_revision" \
  "$(dirname "$archive")" >/dev/null 2>&1; then
  fail "release builder reused an existing archive path"
fi
[[ "$(sha256sum "$archive" | awk '{print $1}')" == "$archive_digest" ]] ||
  fail "release builder changed an existing archive"
cmp --silent "$release_root/share/web/index.html" "$project_dir/web/dist/index.html" ||
  fail "archive omits or changes the real admin HTML"
cmp --silent "$release_root/share/web/assets/admin.css" "$project_dir/web/dist/assets/admin.css" ||
  fail "archive omits or changes the real admin CSS"
cmp --silent "$release_root/share/web/assets/admin.js" "$project_dir/web/dist/assets/admin.js" ||
  fail "archive omits or changes the real admin JavaScript"
for asset_path in "$project_dir"/web/dist/assets/*; do
  asset="${asset_path##*/}"
  cmp --silent "$release_root/share/web/assets/$asset" "$project_dir/web/dist/assets/$asset" ||
    fail "archive omits or changes the Foundation font asset: $asset"
done
python3 - "$release_root" <<'PY'
from pathlib import Path
import re
import sys

root = Path(sys.argv[1])
readme = (root / "README.md").read_text(encoding="utf-8")
references = set(re.findall(r"scripts/[A-Za-z0-9._-]+\.sh", readme))
assert references
for reference in references:
    assert (root / reference).is_file(), f"release README references missing {reference}"
PY

# Every rejected release startup must fail before it creates SQLite, storage, or
# adjacent runtime-lock state.
expect_start_rejected() {
  local label="$1"
  local executable="$2"
  shift 2
  local rejected_state="$test_root/start-rejected-$label"
  local log="$test_root/start-rejected-$label.log"
  local status
  mkdir -m 0755 "$rejected_state" "$rejected_state/db" "$rejected_state/data"
  set +e
  (
    cd /
    timeout 5s env \
      DATABASE_URL="sqlite://$rejected_state/db/app.db" \
      DATA_DIR="$rejected_state/data" \
      BIND=127.0.0.1:0 \
      BOOTSTRAP_ADMIN_USERNAME=admin \
      BOOTSTRAP_ADMIN_PASSWORD=deployment-rejection-password \
      MEDIA_BACKUP_CREDENTIALS_KEY=BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc= \
      REQUIRE_HTTPS=false \
      DEVELOPMENT=true \
      TRUSTED_PROXY_CIDRS= \
      RUST_LOG=warn \
      "$executable" "$@"
  ) >"$log" 2>&1
  status="$?"
  set -e
  [[ "$status" -ne 0 && "$status" -ne 124 ]] || fail "$label did not fail closed before serving"
  [[ -z "$(find "$rejected_state/db" "$rejected_state/data" -mindepth 1 -print -quit)" ]] ||
    fail "$label wrote application or lock state before rejection"
}

expect_start_rejected ordinary-serve "$real_binary" serve
expect_start_rejected implicit-serve "$real_binary"
expect_start_rejected non-normal-root "$real_binary" serve-release "$release_root/../$version"

wrong_layout_root="$test_root/wrong-layout/$version"
mkdir -p "$(dirname "$wrong_layout_root")"
cp -a -- "$release_root" "$wrong_layout_root"
expect_start_rejected wrong-layout-root \
  "$wrong_layout_root/bin/media-backup-server" serve-release "$wrong_layout_root"

outside_binary="$test_root/copied-media-backup-server"
cp -- "$real_binary" "$outside_binary"
chmod 0755 "$outside_binary"
expect_start_rejected copied-binary "$outside_binary" serve-release "$release_root"

alias_parent="$test_root/alias/opt/isarmg/media-backup/releases"
mkdir -p "$alias_parent"
ln -s "$release_root" "$alias_parent/$version"
expect_start_rejected symlink-root "$real_binary" serve-release "$alias_parent/$version"

current_alias="$test_root/mutable-alias/opt/isarmg/media-backup/current"
mkdir -p "$(dirname "$current_alias")"
ln -s "$release_root" "$current_alias"
expect_start_rejected current-alias "$real_binary" serve-release "$current_alias"

tampered_runtime="$test_root/tampered-runtime/opt/isarmg/media-backup/releases/$version"
mkdir -p "$(dirname "$tampered_runtime")"
cp -a -- "$release_root" "$tampered_runtime"
printf '\ntampered\n' >>"$tampered_runtime/README.md"
expect_start_rejected tampered-runtime \
  "$tampered_runtime/bin/media-backup-server" serve-release "$tampered_runtime"

extra_runtime="$test_root/extra-runtime/opt/isarmg/media-backup/releases/$version"
mkdir -p "$(dirname "$extra_runtime")"
cp -a -- "$release_root" "$extra_runtime"
touch "$extra_runtime/EXTRA"
chmod 0644 "$extra_runtime/EXTRA"
expect_start_rejected extra-runtime \
  "$extra_runtime/bin/media-backup-server" serve-release "$extra_runtime"

# Run the physically relocated binary from / against real SQLite and real HTTP.
# The same process verifies its root before it reads application configuration.
smoke_root="$test_root/smoke"
mkdir -m 0755 "$smoke_root" "$smoke_root/db" "$smoke_root/data"
smoke_port="$((20000 + BASHPID % 30000))"
(
  cd /
  exec env \
    DATABASE_URL="sqlite://$smoke_root/db/app.db" \
    DATA_DIR="$smoke_root/data" \
    BIND="127.0.0.1:$smoke_port" \
    BOOTSTRAP_ADMIN_USERNAME=admin \
    BOOTSTRAP_ADMIN_PASSWORD=deployment-smoke-password \
    MEDIA_BACKUP_CREDENTIALS_KEY=BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc= \
    REQUIRE_HTTPS=false \
    DEVELOPMENT=true \
    TRUSTED_PROXY_CIDRS= \
    RUST_LOG=warn \
    "$real_binary" serve-release "$release_root"
) >"$test_root/server.log" 2>&1 &
server_pid="$!"
smoke_ready=0
for _ in {1..120}; do
  if curl --silent --fail "http://127.0.0.1:$smoke_port/healthz" >/dev/null; then
    smoke_ready=1
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    cat "$test_root/server.log" >&2
    fail "real release binary exited during HTTP smoke"
  fi
  sleep 0.1
done
[[ "$smoke_ready" == "1" ]] || {
  cat "$test_root/server.log" >&2
  fail "real release binary did not become healthy"
}
[[ "$(curl --silent --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:$smoke_port/admin")" == "200" ]] ||
  fail "real release binary did not serve its embedded admin web"
curl --silent --fail "http://127.0.0.1:$smoke_port/admin" >"$test_root/served-admin.html"
cmp --silent "$test_root/served-admin.html" "$release_root/share/web/index.html" ||
  fail "served admin asset differs from the verified release copy"
for asset_path in "$project_dir"/web/dist/assets/*; do
  asset="${asset_path##*/}"
  curl --silent --fail --dump-header "$test_root/asset-headers" \
    "http://127.0.0.1:$smoke_port/admin/assets/$asset" >"$test_root/served-asset"
  cmp --silent "$test_root/served-asset" "$release_root/share/web/assets/$asset" ||
    fail "served asset differs from the verified release copy: $asset"
  if [[ "$asset" == *.woff2 ]]; then
    tr -d '\r' <"$test_root/asset-headers" | grep -Fxiq 'content-type: font/woff2' ||
      fail "embedded font has the wrong content type"
  fi
done
kill -TERM "$server_pid"
wait "$server_pid" || true
server_pid=""
[[ -f "$smoke_root/db/app.db" ]] || fail "real release smoke did not create SQLite state"

assert_unit_setting() {
  grep -Fqx "$1" "$unit_source" || fail "missing unit setting: $1"
}

expect_invalid_source() {
  local label="$1"
  local candidate="$2"
  local rejected_root="$test_root/rejected-$label"
  mkdir -m 0755 "$rejected_root"
  if "$real_binary" release-verify "$candidate" >/dev/null 2>&1; then
    fail "$label was accepted by the trusted real verifier"
  fi
  if MEDIA_BACKUP_SETUP_ROOT="$rejected_root" MEDIA_BACKUP_SETUP_TEST=1 \
    "$candidate/scripts/setup-wsl.sh" >/dev/null 2>&1; then
    fail "$label was accepted by setup"
  fi
  [[ -z "$(find "$rejected_root" -mindepth 1 -print -quit)" ]] ||
    fail "$label caused writes before release preflight completed"
}

negative_root="$test_root/negative"
fresh_negative() {
  if [[ -e "$negative_root" ]]; then
    rm -rf -- "$negative_root"
  fi
  cp -a -- "$release_root" "$negative_root"
}

fresh_negative
printf '#!/usr/bin/env bash\nexit 0\n' >"$negative_root/bin/media-backup-server"
chmod 0755 "$negative_root/bin/media-backup-server"
expect_invalid_source fake-binary "$negative_root"

fresh_negative
printf '\ntampered\n' >>"$negative_root/README.md"
expect_invalid_source tampered-file "$negative_root"

fresh_negative
touch "$negative_root/EXTRA"
chmod 0644 "$negative_root/EXTRA"
expect_invalid_source extra-file "$negative_root"

fresh_negative
rm "$negative_root/share/web/assets/admin.css"
expect_invalid_source missing-file "$negative_root"

fresh_negative
printf '\ntampered\n' >>"$negative_root/share/web/assets/MapleMonoNormalNL-Regular.woff2"
expect_invalid_source tampered-font "$negative_root"

# A self-consistent replacement manifest cannot authorize bytes absent from the binary.
fresh_negative
printf '\ntampered\n' >>"$negative_root/share/web/assets/MapleMonoNormalNL-Regular.woff2"
rm "$negative_root/release-manifest.json"
python3 "$project_dir/scripts/write-release-manifest.py" "$negative_root" "$source_revision"
expect_invalid_source rehashed-font "$negative_root"

fresh_negative
rm "$negative_root/share/web/assets/MapleMono-OFL.txt"
expect_invalid_source missing-font-license "$negative_root"

fresh_negative
chmod 0664 "$negative_root/README.md"
expect_invalid_source writable-payload "$negative_root"

fresh_negative
chmod 4755 "$negative_root/bin/media-backup-server"
expect_invalid_source privileged-mode-payload "$negative_root"

fresh_negative
ln "$negative_root/README.md" "$negative_root/docs/README.alias"
expect_invalid_source hard-linked-payload "$negative_root"

fresh_negative
rm "$negative_root/share/web/assets/admin.css"
ln -s /etc/passwd "$negative_root/share/web/assets/admin.css"
expect_invalid_source symlinked-payload "$negative_root"

fresh_negative
rm "$negative_root/share/web/assets/admin.css"
mkfifo "$negative_root/share/web/assets/admin.css"
expect_invalid_source special-payload "$negative_root"

fresh_negative
python3 - "$negative_root/release-manifest.json" version <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1]); value = json.loads(path.read_text())
value["identity"][sys.argv[2]] = "9.9.9"
path.write_text(json.dumps(value) + "\n")
PY
expect_invalid_source wrong-version "$negative_root"

fresh_negative
python3 - "$negative_root/release-manifest.json" product <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1]); value = json.loads(path.read_text())
value["identity"][sys.argv[2]] = "not-media-backup"
path.write_text(json.dumps(value) + "\n")
PY
expect_invalid_source wrong-product "$negative_root"

fresh_negative
python3 - "$negative_root/release-manifest.json" top <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1]); value = json.loads(path.read_text())
value["unexpected"] = True
path.write_text(json.dumps(value) + "\n")
PY
expect_invalid_source unknown-top-field "$negative_root"

fresh_negative
python3 - "$negative_root/release-manifest.json" nested <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1]); value = json.loads(path.read_text())
value["identity"]["unknown_alias"] = "unexpected"
path.write_text(json.dumps(value) + "\n")
PY
expect_invalid_source unknown-identity-field "$negative_root"

fresh_negative
python3 - "$negative_root/release-manifest.json" file <<'PY'
import json, pathlib, sys
path = pathlib.Path(sys.argv[1]); value = json.loads(path.read_text())
value["files"][0]["unknown_path"] = value["files"][0]["path"]
path.write_text(json.dumps(value) + "\n")
PY
expect_invalid_source unknown-file-field "$negative_root"

if MEDIA_BACKUP_SETUP_ROOT="$test_root/alternate-without-test-mode" \
  "$setup_script" >/dev/null 2>&1; then
  fail "alternate root was accepted outside test mode"
fi
if MEDIA_BACKUP_SETUP_ROOT=/ MEDIA_BACKUP_SETUP_TEST=1 \
  "$setup_script" >/dev/null 2>&1; then
  fail "test mode was allowed to target the real root"
fi

install_root="$test_root/install-root"
mkdir -m 0755 "$install_root"
first_output="$test_root/first-output"
MEDIA_BACKUP_SETUP_ROOT="$install_root" MEDIA_BACKUP_SETUP_TEST=1 \
  "$setup_script" >"$first_output" 2>&1

release_dir="$install_root/opt/isarmg/media-backup/releases/$version"
installed_binary="$release_dir/bin/media-backup-server"
config="$install_root/etc/isarmg/media-backup.env"
installed_unit="$install_root/etc/systemd/system/media-backup.service"

[[ -x "$installed_binary" && ! -L "$installed_binary" ]] || fail "real release binary was not installed"
cmp --silent "$release_root/release-manifest.json" "$release_dir/release-manifest.json" ||
  fail "installed immutable generation differs from the archive"
"$installed_binary" release-verify "$release_dir" >/dev/null || fail "installed release does not verify"
if [[ "$EUID" -eq 0 ]]; then
  "$installed_binary" release-verify-installed "$release_dir" >/dev/null ||
    fail "root-owned installed release does not pass the production ownership gate"
fi
[[ ! -e "$install_root/opt/isarmg/media-backup/current" &&
  ! -L "$install_root/opt/isarmg/media-backup/current" ]] ||
  fail "installer created a mutable current alias"
cmp --silent "$unit_source" "$installed_unit" || fail "installed unit differs from the archive"

[[ -f "$config" && ! -L "$config" && "$(stat -c '%a:%h' "$config")" == "600:1" ]] ||
  fail "configuration is not a private single-link regular file"
grep -Fqx '# INITIAL-SECRETS-MUST-BE-REPLACED' "$config" || fail "initial-secret marker is missing"
grep -Fqx 'DATABASE_URL=sqlite:///var/lib/isarmg/media-backup/db/app.db' "$config" ||
  fail "SQLite database path is incorrect"
grep -Fqx 'DATA_DIR=/var/lib/isarmg/media-backup/data' "$config" || fail "data path is incorrect"
[[ -d "$install_root/var/lib/isarmg/media-backup/db" &&
  -d "$install_root/var/lib/isarmg/media-backup/data" ]] || fail "separate database/data directories are missing"

admin_secret="$(awk -F= '/^BOOTSTRAP_ADMIN_PASSWORD=/ { print $2 }' "$config")"
credentials_secret="$(awk -F= '/^MEDIA_BACKUP_CREDENTIALS_KEY=/ { print $2 }' "$config")"
metrics_secret="$(awk -F= '/^METRICS_TOKEN=/ { print $2 }' "$config")"
[[ "$admin_secret" =~ ^[[:xdigit:]]{64}$ && "$metrics_secret" =~ ^[[:xdigit:]]{64}$ ]] ||
  fail "generated secrets are not 256-bit random hex"
[[ "$(printf '%s' "$credentials_secret" | base64 --decode | wc -c)" == "32" ]] || fail "generated credentials key is not 256-bit Base64"
[[ "$admin_secret" != "$metrics_secret" ]] || fail "independent generated secrets are equal"
if grep -Fq "$admin_secret" "$first_output" || grep -Fq "$credentials_secret" "$first_output" || grep -Fq "$metrics_secret" "$first_output"; then
  fail "setup output disclosed a generated secret"
fi

tree_digest() {
  tar -C "$1" --sort=name --format=gnu -cf - . | sha256sum | awk '{print $1}'
}

installed_digest="$(tree_digest "$install_root")"
second_output="$test_root/second-output"
if MEDIA_BACKUP_SETUP_ROOT="$install_root" MEDIA_BACKUP_SETUP_TEST=1 \
  "$setup_script" >"$second_output" 2>&1; then
  fail "a second installation of the same physical version was accepted"
fi
[[ "$(tree_digest "$install_root")" == "$installed_digest" ]] ||
  fail "rejected second installation changed installed state"

conflict_root="$test_root/conflict-root"
mkdir -m 0755 "$conflict_root"
MEDIA_BACKUP_SETUP_ROOT="$conflict_root" MEDIA_BACKUP_SETUP_TEST=1 "$setup_script" >/dev/null
conflict_file="$conflict_root/opt/isarmg/media-backup/releases/$version/README.md"
printf '\nconflict\n' >>"$conflict_file"
conflict_digest="$(sha256sum "$conflict_file" | awk '{print $1}')"
if MEDIA_BACKUP_SETUP_ROOT="$conflict_root" MEDIA_BACKUP_SETUP_TEST=1 \
  "$setup_script" >/dev/null 2>&1; then
  fail "same version silently accepted different installed content"
fi
[[ "$(sha256sum "$conflict_file" | awk '{print $1}')" == "$conflict_digest" ]] ||
  fail "immutable conflicting release was overwritten"

symlink_release_root="$test_root/symlink-release-root"
release_escape="$test_root/release-escape"
mkdir -p "$symlink_release_root/opt/isarmg/media-backup" "$release_escape"
ln -s "$release_escape" "$symlink_release_root/opt/isarmg/media-backup/releases"
if MEDIA_BACKUP_SETUP_ROOT="$symlink_release_root" MEDIA_BACKUP_SETUP_TEST=1 \
  "$setup_script" >/dev/null 2>&1; then
  fail "symlinked releases directory was accepted"
fi
[[ -z "$(find "$release_escape" -mindepth 1 -print -quit)" ]] || fail "release symlink escaped test root"

symlink_config_root="$test_root/symlink-config-root"
config_escape="$test_root/config-escape"
mkdir -p "$symlink_config_root/etc" "$config_escape"
ln -s "$config_escape" "$symlink_config_root/etc/isarmg"
if MEDIA_BACKUP_SETUP_ROOT="$symlink_config_root" MEDIA_BACKUP_SETUP_TEST=1 \
  "$setup_script" >/dev/null 2>&1; then
  fail "symlinked configuration directory was accepted"
fi
[[ -z "$(find "$config_escape" -mindepth 1 -print -quit)" ]] || fail "config symlink escaped test root"

malicious_current_root="$test_root/malicious-current-root"
mkdir -p "$malicious_current_root/opt/isarmg/media-backup"
ln -s /tmp "$malicious_current_root/opt/isarmg/media-backup/current"
if MEDIA_BACKUP_SETUP_ROOT="$malicious_current_root" MEDIA_BACKUP_SETUP_TEST=1 \
  "$setup_script" >/dev/null 2>&1; then
  fail "an unmanaged current symlink was accepted"
fi
[[ ! -e "$malicious_current_root/etc" && ! -e "$malicious_current_root/var" ]] ||
  fail "unmanaged current rejection wrote new install state"

special_config_root="$test_root/special-config-root"
mkdir -p "$special_config_root/etc/isarmg"
mkfifo "$special_config_root/etc/isarmg/media-backup.env"
if MEDIA_BACKUP_SETUP_ROOT="$special_config_root" MEDIA_BACKUP_SETUP_TEST=1 \
  "$setup_script" >/dev/null 2>&1; then
  fail "a special configuration target was accepted"
fi

hardlink_config_root="$test_root/hardlink-config-root"
mkdir -p "$hardlink_config_root/etc/isarmg"
touch "$hardlink_config_root/etc/isarmg/media-backup.env"
ln "$hardlink_config_root/etc/isarmg/media-backup.env" "$hardlink_config_root/config-alias"
if MEDIA_BACKUP_SETUP_ROOT="$hardlink_config_root" MEDIA_BACKUP_SETUP_TEST=1 \
  "$setup_script" >/dev/null 2>&1; then
  fail "a hard-linked configuration target was accepted"
fi

for setting in \
  'User=isarmg-media' \
  'Group=isarmg-media' \
  'UMask=0077' \
  'StateDirectory=isarmg/media-backup' \
  'RuntimeDirectory=isarmg/media-backup' \
  'EnvironmentFile=/etc/isarmg/media-backup.env' \
  'ExecStart=/opt/isarmg/media-backup/releases/0.3.16/bin/media-backup-server serve-release /opt/isarmg/media-backup/releases/0.3.16' \
  'ReadWritePaths=/var/lib/isarmg/media-backup /run/isarmg/media-backup' \
  'ProtectSystem=strict' \
  'ProtectHome=true' \
  'NoNewPrivileges=true' \
  'PrivateTmp=true' \
  'PrivateDevices=true' \
  'ProtectKernelTunables=true' \
  'ProtectKernelModules=true' \
  'ProtectKernelLogs=true' \
  'ProtectControlGroups=true' \
  'RestrictSUIDSGID=true' \
  'LockPersonality=true'; do
  assert_unit_setting "$setting"
done

if grep -q '^ExecStartPre=' "$unit_source"; then
  fail "systemd split release verification from the serving process"
fi

if grep -Eqi 'postgres|/mnt/|User=root|MEDIA_BACKUP_BINARY|Cargo\.toml' \
  "$setup_script" "$release_root/scripts/run-server-wsl.sh" \
  "$release_root/scripts/start-server-wsl.sh" "$unit_source"; then
  fail "release deployment still trusts source state, PostgreSQL, /mnt, or root service execution"
fi
if grep -Eq 'sed[[:space:]]+-i|systemctl[[:space:]]+(enable|start|restart)' "$setup_script"; then
  fail "setup mutates secrets with sed or starts the service"
fi
if grep -Fq '/opt/isarmg/media-backup/current' \
  "$project_dir/README.md" "$project_dir/docs/operations.md" \
  "$setup_script" "$release_root/scripts/run-server-wsl.sh" \
  "$release_root/scripts/start-server-wsl.sh" "$unit_source"; then
  fail "production deployment still references a mutable current alias"
fi
if grep -Eq -- '--clobber|gh[[:space:]]+release[[:space:]]+upload|"v\*\.\*\.\*"' \
  "$project_dir/.github/workflows/release.yml"; then
  fail "release workflow can overwrite assets or accepts mutable tags"
fi
if "$project_dir/scripts/verify-release-version.sh" v9.9.9 >/dev/null 2>&1; then
  fail "release version gate accepted a non-current tag"
fi

bash -n "$setup_script" "$release_root/scripts/run-server-wsl.sh" \
  "$release_root/scripts/start-server-wsl.sh" "$release_root/scripts/verify-server-wsl.sh" \
  "$project_dir/scripts/build-server-release.sh" "$0"
python3 - "$project_dir/scripts/write-release-manifest.py" <<'PY'
from pathlib import Path
import sys
compile(Path(sys.argv[1]).read_text(encoding="utf-8"), sys.argv[1], "exec")
PY
if command -v shellcheck >/dev/null; then
  shellcheck "$setup_script" "$release_root/scripts/run-server-wsl.sh" \
    "$release_root/scripts/start-server-wsl.sh" "$release_root/scripts/verify-server-wsl.sh" \
    "$project_dir/scripts/build-server-release.sh" "$0"
fi
if [[ "${MEDIA_BACKUP_VERIFY_SYSTEMD:-0}" == "1" ]]; then
  command -v systemd-analyze >/dev/null || fail "systemd-analyze is required for unit verification"
  if ! systemd-analyze --root="$install_root" --recursive-errors=no verify "$installed_unit" \
    >"$test_root/systemd-verify" 2>&1; then
    cat "$test_root/systemd-verify" >&2
    fail "systemd unit verification failed"
  fi
fi

printf 'real 0.2 release archive passed identity, smoke, tamper, and temporary-root deployment tests\n'
