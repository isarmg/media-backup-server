#!/usr/bin/env bash
set -euo pipefail

readonly release="/opt/isarmg/xszs/releases/1.0.0"
readonly binary="$release/bin/xszs"
readonly config="/etc/isarmg/xszs.env"
readonly unit="/etc/systemd/system/xszs.service"
readonly marker="# INITIAL-SECRETS-MUST-BE-REPLACED"

fail() {
  printf 'run error: %s\n' "$*" >&2
  exit 1
}

[[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]] ||
  fail "formal server startup requires Linux x86_64"

verify_installed_release() {
  local output line_marker product version revision target fingerprint extra directory mode
  for directory in /opt /opt/isarmg /opt/isarmg/xszs \
    /opt/isarmg/xszs/releases "$release"; do
    [[ -d "$directory" && ! -L "$directory" ]] || fail "invalid release directory: $directory"
    [[ "$(stat -c '%u:%g' -- "$directory")" == "0:0" ]] ||
      fail "release directory is not root-owned: $directory"
    mode="$(stat -c '%a' -- "$directory")"
    (( (8#$mode & 0022) == 0 )) || fail "release directory is group/other writable: $directory"
  done
  [[ -x "$binary" ]] || fail "missing installed xszs server binary"
  output="$("$binary" release-verify-installed "$release")" ||
    fail "installed release manifest, identity, or payload verification failed"
  [[ "$output" != *$'\n'* ]] || fail "release verifier returned multiple lines"
  IFS=$'\t' read -r line_marker product version revision target fingerprint extra <<<"$output"
  [[ -z "${extra:-}" && "$line_marker" == "XSZS_RELEASE_VERIFIED_V1" &&
    "$product" == "xszs" && "$version" == "1.0.0" &&
    "$revision" =~ ^[0-9a-f]{40}$ && "$target" == "x86_64-unknown-linux-gnu" &&
    "$fingerprint" == "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["identity"]["release_contract_sha256"])' "$release/release-manifest.json")" ]] || fail "installed release returned an unexpected identity"
  [[ -f "$unit" && ! -L "$unit" && "$(stat -c '%a:%u:%g:%h' -- "$unit")" == "644:0:0:1" ]] ||
    fail "installed systemd unit is not immutable root-owned release content"
  cmp --silent -- "$release/systemd/xszs.service" "$unit" ||
    fail "installed systemd unit differs from the verified release"
  [[ -L /opt/isarmg/xszs/current &&
    "$(stat -c '%u:%g' -- /opt/isarmg/xszs/current)" == "0:0" &&
    "$(readlink -- /opt/isarmg/xszs/current)" == "$release" ]] ||
    fail "installed deployment pointer differs from the verified release"
}

[[ "$EUID" -eq 0 ]] || fail "run this script as root (or with sudo)"
verify_installed_release
[[ -f "$config" && ! -L "$config" ]] || fail "missing regular production configuration: $config"
[[ "$(stat -c '%a:%u:%g:%h' -- "$config")" == "600:0:0:1" ]] ||
  fail "production configuration must be root-owned, mode 0600, and have one hard link"
if grep -Fqx "$marker" "$config"; then
  fail "replace the generated secrets in $config and remove the initial-secret marker first"
fi

systemctl start xszs.service
exec journalctl --unit xszs.service --follow
