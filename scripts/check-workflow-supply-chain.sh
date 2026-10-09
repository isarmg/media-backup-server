#!/usr/bin/env bash
set -euo pipefail

fail() {
  printf 'workflow supply-chain check failed: %s\n' "$*" >&2
  exit 1
}

repo_root="$(git rev-parse --show-toplevel)" || fail "not inside a Git worktree"
cd "$repo_root"

mapfile -d '' -t workflow_files < <(
  git ls-files -z -- '.github/workflows/*.yml' '.github/workflows/*.yaml'
)
((${#workflow_files[@]} > 0)) || fail "no tracked workflow files found"

total_uses=0
checkout_uses=0
checkout_persist_false=0
setup_gradle_uses=0
setup_gradle_basic_cache=0

current_file=""
current_job=""
job_runner=""
job_timeout=""
job_permissions_seen=0
job_permission_count=0
job_permission_name=""
job_permission_level=""
inside_jobs=false
inside_permissions=false

finish_job() {
  [[ -n "$current_job" ]] || return 0

  [[ -n "$job_runner" ]] ||
    fail "$current_file: job '$current_job' has no fixed runs-on value"
  if [[ "$current_job" == "ios" ]]; then
    [[ "$job_runner" == "macos-15" ]] ||
      fail "$current_file: iOS job must use macos-15, got '$job_runner'"
  else
    [[ "$job_runner" == "ubuntu-24.04" ]] ||
      fail "$current_file: job '$current_job' must use ubuntu-24.04, got '$job_runner'"
  fi

  [[ "$job_timeout" =~ ^[0-9]+$ ]] ||
    fail "$current_file: job '$current_job' has no numeric timeout-minutes"
  ((job_timeout >= 1 && job_timeout <= 120)) ||
    fail "$current_file: job '$current_job' timeout must be between 1 and 120 minutes"

  ((job_permissions_seen == 1)) ||
    fail "$current_file: job '$current_job' must define one job-level permissions map"
  ((job_permission_count == 1)) ||
    fail "$current_file: job '$current_job' must grant only the contents permission"
  [[ "$job_permission_name" == "contents" ]] ||
    fail "$current_file: job '$current_job' grants unexpected '$job_permission_name' permission"

  if [[ "$current_file" == ".github/workflows/release.yml" && "$current_job" == "publish" ]]; then
    [[ "$job_permission_level" == "write" ]] ||
      fail "$current_file: publish job requires contents: write"
  else
    [[ "$job_permission_level" == "read" ]] ||
      fail "$current_file: job '$current_job' must use contents: read"
  fi
}

for current_file in "${workflow_files[@]}"; do
  [[ -f "$current_file" && ! -L "$current_file" ]] ||
    fail "$current_file must be a regular, non-symlink file"

  current_job=""
  inside_jobs=false
  inside_permissions=false
  line_number=0

  while IFS= read -r line || [[ -n "$line" ]]; do
    ((++line_number))
    [[ "$line" != *$'\t'* ]] || fail "$current_file:$line_number contains a tab"

    if [[ "$line" =~ ^permissions: ]]; then
      fail "$current_file:$line_number uses workflow-level permissions; scope permissions per job"
    fi

    if [[ "$line" == "jobs:" ]]; then
      inside_jobs=true
      continue
    fi

    if [[ "$inside_jobs" == true && "$line" =~ ^\ \ ([A-Za-z0-9_-]+):[[:space:]]*$ ]]; then
      next_job="${BASH_REMATCH[1]}"
      finish_job
      current_job="$next_job"
      job_runner=""
      job_timeout=""
      job_permissions_seen=0
      job_permission_count=0
      job_permission_name=""
      job_permission_level=""
      inside_permissions=false
      continue
    fi

    if [[ -n "$current_job" && "$line" =~ ^\ \ \ \ runs-on:[[:space:]]*([^[:space:]#]+) ]]; then
      [[ -z "$job_runner" ]] || fail "$current_file:$line_number duplicates runs-on"
      job_runner="${BASH_REMATCH[1]}"
    fi

    if [[ -n "$current_job" && "$line" =~ ^\ \ \ \ timeout-minutes:[[:space:]]*([^[:space:]#]+) ]]; then
      [[ -z "$job_timeout" ]] || fail "$current_file:$line_number duplicates timeout-minutes"
      job_timeout="${BASH_REMATCH[1]}"
    fi

    if [[ -n "$current_job" && "$line" =~ ^\ \ \ \ permissions:[[:space:]]*$ ]]; then
      ((++job_permissions_seen))
      inside_permissions=true
      continue
    fi

    if [[ "$inside_permissions" == true ]]; then
      if [[ "$line" =~ ^\ \ \ \ \ \ ([a-z-]+):[[:space:]]*(read|write|none)([[:space:]]*#.*)?$ ]]; then
        ((++job_permission_count))
        job_permission_name="${BASH_REMATCH[1]}"
        job_permission_level="${BASH_REMATCH[2]}"
        continue
      fi
      if [[ "$line" =~ ^\ \ \ \ \ \ [^[:space:]#] ]]; then
        fail "$current_file:$line_number has an invalid permission entry"
      fi
      if [[ "$line" =~ ^\ \ \ \ [^[:space:]#] ]]; then
        inside_permissions=false
      fi
    fi

    if [[ "$line" =~ ^[[:space:]]*(-[[:space:]]+)?uses:[[:space:]]*([^[:space:]#]+) ]]; then
      action_ref="${BASH_REMATCH[2]}"
      ((++total_uses))
      [[ "$action_ref" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+(/[A-Za-z0-9_.@/-]+)?@[0-9a-f]{40}$ ]] ||
        fail "$current_file:$line_number action is not pinned to a full lowercase commit SHA: $action_ref"
      [[ "$line" == *" # "* ]] ||
        fail "$current_file:$line_number pinned action is missing a human-readable version comment"

      if [[ "$action_ref" == actions/checkout@* ]]; then
        ((++checkout_uses))
      elif [[ "$action_ref" == gradle/actions/setup-gradle@* ]]; then
        ((++setup_gradle_uses))
      fi
    fi

    if [[ "$line" =~ ^[[:space:]]*persist-credentials:[[:space:]]*([^[:space:]#]+) ]]; then
      [[ "${BASH_REMATCH[1]}" == "false" ]] ||
        fail "$current_file:$line_number checkout credentials must not persist"
      ((++checkout_persist_false))
    fi

    if [[ "$line" =~ ^[[:space:]]*cache-provider:[[:space:]]*([^[:space:]#]+) ]]; then
      [[ "${BASH_REMATCH[1]}" == "basic" ]] ||
        fail "$current_file:$line_number setup-gradle must use the basic cache provider"
      ((++setup_gradle_basic_cache))
    fi
  done < "$current_file"

  finish_job
done

((total_uses > 0)) || fail "no action references found"
((checkout_uses == checkout_persist_false)) ||
  fail "each checkout action must set persist-credentials: false"
((setup_gradle_uses == setup_gradle_basic_cache)) ||
  fail "each setup-gradle action must set cache-provider: basic"

if grep -n -E 'android-signing|secrets[.]|assembleRelease|assembleDebug|apksigner|clients/(android|ios)' "${workflow_files[@]}"; then
  fail "Server workflows must not build mobile clients or access mobile signing credentials"
fi

printf 'workflow supply-chain check passed (%d workflows, %d pinned actions)\n' \
  "${#workflow_files[@]}" "$total_uses"
