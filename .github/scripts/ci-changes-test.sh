#!/usr/bin/env bash
# Tests ci-changes.sh: a few path rules, and every case that must fall back to running every job,
# against throwaway git repositories laid out like the pull_request and dispatch checkouts
#
#   .github/scripts/ci-changes-test.sh   exits non-zero on the first failure
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/ci-changes.sh"
all="android_instrumented clippy compile_android compile_ios ios_ui ktlint qa_manifest rustfmt swift_lint tapsigner_emulator test"
failures=0

# the jobs a ci-changes.sh output selects, sorted onto one line
selected() {
  grep '=true$' | cut -d= -f1 | sort | tr '\n' ' ' | sed 's/ $//'
}

expect() {
  local name=$1 want=$2 got=$3

  if [[ $got == "$want" ]]; then
    echo "ok   $name"
  else
    echo "FAIL $name"
    echo "     want: ${want:-<none>}"
    echo "     got:  ${got:-<none>}"
    failures=$((failures + 1))
  fi
}

classify() {
  printf '%b' "$1" | "$script" classify | selected
}

expect "docs only runs nothing" "" "$(classify 'docs/testing.md\nREADME.md\n')"
expect "android only runs the Android jobs" \
  "android_instrumented compile_android ktlint qa_manifest" "$(classify 'android/app/build.gradle.kts\n')"
expect "ios only runs the iOS jobs and the Rust tests" \
  "compile_ios ios_ui qa_manifest swift_lint test" "$(classify 'ios/Cove/CoveApp.swift\n')"
expect "rust runs every job" "$all" "$(classify 'rust/src/lib.rs\n')"
expect "justfile runs both apps' jobs" \
  "android_instrumented compile_android compile_ios ios_ui ktlint swift_lint tapsigner_emulator" "$(classify 'justfile\n')"
expect ".github runs every job" "$all" "$(classify 'docs/testing.md\n.github/workflows/ci.yml\n')"
expect "an unknown path runs every job" "$all" "$(classify 'docs/testing.md\nnew-dir/file\n')"
expect "a last path without a newline counts" "$all" "$(classify 'rust/src/lib.rs')"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
repo="$work/repo"
mkdir "$repo"
cd "$repo"
git init -q -b master
git config user.name test
git config user.email test@example.com
mkdir -p android ios docs
echo a >android/app.kt
echo d >docs/a.md
git add -A
git commit -q -m base
git tag base

# runs the script as the changes job does and prints the jobs its $GITHUB_OUTPUT selects, which
# is what the workflow reads; a failed run or an output without every job prints why instead
run() {
  local out="$work/output"
  : >"$out"

  if ! env -u BASE_REF GITHUB_OUTPUT="$out" "$@" "$script" >/dev/null 2>&1; then
    echo "<script failed>"
  elif [[ $(grep -c -E '^[a-z_]+=(true|false)$' "$out") -ne $(wc -w <<<"$all") ]]; then
    echo "<output lacks some jobs>"
  else
    selected <"$out"
  fi
}

# a branch that moves a file from android/ to ios/, so both the old and new path must count
git checkout -q -b feature
git mv android/app.kt ios/app.swift
git commit -q -m move

# HEAD is a merge commit whose first parent is the base, as actions/checkout leaves a pull_request
git checkout -q -b pr-merge base
git merge -q --no-ff -m merge feature
expect "pull_request counts a rename's old and new path" \
  "android_instrumented compile_android compile_ios ios_ui ktlint qa_manifest swift_lint test" \
  "$(run GITHUB_EVENT_NAME=pull_request)"

git checkout -q feature
expect "pull_request without a merge commit runs every job" "$all" "$(run GITHUB_EVENT_NAME=pull_request)"

# the parent moved on after the child left it, with the child's Android edit and a docs change of
# its own, so only a diff from the merge base still sees the child's Android change
git checkout -q -b child base
echo b >android/app.kt
git commit -q -am "child edit"
git checkout -q -b parent base
echo b >android/app.kt
echo e >docs/a.md
git commit -q -am "parent edit"
git update-ref refs/remotes/origin/parent parent

git checkout -q child
expect "dispatch with a base diffs from the merge base" \
  "android_instrumented compile_android ktlint qa_manifest" \
  "$(run GITHUB_EVENT_NAME=workflow_dispatch BASE_REF=parent)"
expect "dispatch without a base runs every job" "$all" "$(run GITHUB_EVENT_NAME=workflow_dispatch)"
expect "dispatch with a missing base runs every job" "$all" \
  "$(run GITHUB_EVENT_NAME=workflow_dispatch BASE_REF=no-such-branch)"
expect "dispatch with an invalid base runs every job" "$all" \
  "$(run GITHUB_EVENT_NAME=workflow_dispatch BASE_REF='bad..ref')"
git update-ref refs/remotes/origin/child HEAD
expect "dispatch with an empty diff runs every job" "$all" \
  "$(run GITHUB_EVENT_NAME=workflow_dispatch BASE_REF=child)"
expect "push runs every job" "$all" "$(run GITHUB_EVENT_NAME=push)"
expect "schedule runs every job" "$all" "$(run GITHUB_EVENT_NAME=schedule)"

if ((failures > 0)); then
  echo "$failures failed"
  exit 1
fi
