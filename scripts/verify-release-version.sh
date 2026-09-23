#!/usr/bin/env bash
set -euo pipefail

release_tag="${1:-${GITHUB_REF_NAME:-}}"

if ! [[ "$release_tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Release tag must be a semantic version such as v0.3.21 (received: ${release_tag:-<empty>})." >&2
  exit 1
fi

release_version="${release_tag#v}"
cargo_version="$({
  awk '
    /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && /^version[[:space:]]*=/ {
      gsub(/^[^"]*"|".*$/, "")
      print
      exit
    }
  ' Cargo.toml
})"

if [[ -z "$cargo_version" ]]; then
  echo "Unable to read the Server Cargo version." >&2
  exit 1
fi

if ! [[ "$release_version" == "$cargo_version" ]]; then
  echo "Release versions do not match:" >&2
  echo "  tag:   $release_version" >&2
  echo "  Cargo: $cargo_version" >&2
  exit 1
fi

tag_commit="$(git rev-parse --verify "refs/tags/$release_tag^{commit}")" || {
  echo "Release tag does not exist locally: $release_tag" >&2
  exit 1
}
head_commit="$(git rev-parse --verify HEAD)"
if [[ "$tag_commit" != "$head_commit" ]]; then
  echo "Checked-out source is not the exact commit named by $release_tag." >&2
  exit 1
fi
if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
  echo "Release checkout contains uncommitted or untracked content." >&2
  exit 1
fi

echo "Release version $release_version and source revision $head_commit are consistent."
