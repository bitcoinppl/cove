#!/usr/bin/env bash
# Decides which CI jobs a change needs, so a docs-only or one-platform change skips the runners
# it can't affect
#
#   ci-changes.sh classify < paths   prints <job>=true|false for the changed paths on stdin
#   ci-changes.sh                    in a workflow's changes job: lists the paths the triggering
#                                    event changed and appends the same lines to $GITHUB_OUTPUT
#
# Each rule maps a path to the jobs that read it, and a path no rule names runs every job, so a
# new top-level file or directory is tested until a rule here says otherwise. A push, a schedule,
# a dispatch without a base, any change under .github, and a diff that can't be computed run
# every job
#
#   GITHUB_EVENT_NAME  pull_request diffs the merge commit actions/checkout made against its
#                      first parent, the base branch; workflow_dispatch diffs against BASE_REF
#   BASE_REF           for workflow_dispatch, the branch to diff against, fetched as
#                      origin/<BASE_REF>, such as a stacked branch's parent
set -euo pipefail

JOBS="rustfmt clippy test tapsigner_emulator qa_manifest swift_lint ktlint compile_ios compile_android ios_ui android_instrumented"

# checklist rows name Rust tests and source files, so the manifest check runs with the Rust jobs
RUST="rustfmt clippy test tapsigner_emulator qa_manifest"

# every job for either app; both embed the Rust library and its generated bindings, so a change
# shared by both platforms runs all of them, the lint jobs included
MOBILE="compile_ios swift_lint ios_ui compile_android ktlint android_instrumented"

jobs_for() {
  case $1 in
    .github/*) echo "$JOBS" ;;
    # xtask, which the app builds and swift-lint run through, embeds the release seam markers, and
    # the app release builds run the seam check script
    rust/* | tools/release-seams/*) echo "$RUST $MOBILE" ;;
    # its recipes build both apps' Rust libraries and run the TAPSIGNER emulator and UI tests
    justfile) echo "$MOBILE tapsigner_emulator" ;;
    # checklist rows name app tests and source files too, and xtask's tests of the iOS UI test
    # planner read the Xcode project and the UI test sources
    ios/*) echo "compile_ios swift_lint qa_manifest ios_ui test" ;;
    android/*) echo "compile_android ktlint qa_manifest android_instrumented" ;;
    .swiftlint.yml | .swiftformat | .swift-ast-lint.yml | tools/cove-swift-lint/* | tools/swift-tools.json) echo swift_lint ;;
    # the device scripts and their tool wrappers are tested offline by the manifest job
    qa/* | tools/wda/* | tools/uia2/*) echo qa_manifest ;;
    tools/ui-test-plan.sh) echo "ios_ui android_instrumented" ;;
    # read by people and review bots, never by a build or test
    docs/* | images/* | designs/* | pubkeys/* | .codex/* | .greptile/* | .coderabbit.yaml | \
      .envrc.example | rust-analyzer.json | LICENSE | AGENTS.md | ARCHITECTURE.md | CHANGELOG.md | \
      CONTRIBUTING.md | README.md | SECURITY.md) ;;
    *) echo "$JOBS" ;;
  esac
}

# prints one <job>=true|false line per job, true when it's in the space-separated list $1
print_jobs() {
  local job
  for job in $JOBS; do
    case " $1 " in
      *" $job "*) echo "$job=true" ;;
      *) echo "$job=false" ;;
    esac
  done
}

classify() {
  local path selected=""
  # the second test keeps a last path that has no trailing newline
  while IFS= read -r path || [[ -n $path ]]; do
    [[ -n $path ]] && selected+=" $(jobs_for "$path")"
  done

  print_jobs "$selected"
}

# lists the paths the triggering event changed, or fails when it can't, which runs every job
changed_paths() {
  local base

  case ${GITHUB_EVENT_NAME:-} in
    pull_request)
      # without a second parent HEAD isn't the merge commit, so its first parent isn't the base
      git rev-parse --verify --quiet 'HEAD^2' >/dev/null || return 1
      git diff --no-renames --name-only 'HEAD^1' HEAD
      ;;
    workflow_dispatch)
      [[ -n ${BASE_REF:-} ]] || return 1
      git check-ref-format --branch "$BASE_REF" >/dev/null || return 1
      base=$(git merge-base "refs/remotes/origin/$BASE_REF" HEAD) || return 1
      git diff --no-renames --name-only "$base" HEAD
      ;;
    *) return 1 ;;
  esac
}

main() {
  local paths result

  # an empty diff is as unexpected as a failed one, so both run every job
  if paths=$(changed_paths) && [[ -n $paths ]]; then
    result=$(printf '%s\n' "$paths" | classify)
    echo "$(printf '%s\n' "$paths" | wc -l | tr -d ' ') changed paths:"
    printf '%s\n' "$paths" | sed 's/^/  /'
  else
    result=$(print_jobs "$JOBS")
    echo "running every job: no diff to filter for this ${GITHUB_EVENT_NAME:-local} run${BASE_REF:+ against $BASE_REF}"
  fi

  printf '%s\n' "$result"
  printf '%s\n' "$result" >>"${GITHUB_OUTPUT:-/dev/null}"
}

case ${1:-} in
  classify) classify ;;
  "") main ;;
  *)
    echo "usage: $0 [classify]" >&2
    exit 2
    ;;
esac
