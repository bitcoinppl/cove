#!/usr/bin/env bash
# Tests ci-local-results.sh: which commit statuses let CI skip a macOS job, and that a failure to
# read them skips nothing
#
#   .github/scripts/ci-local-results-test.sh   exits non-zero on any failure
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/ci-local-results.sh"
failures=0
export TRUSTED="maintainer other-maintainer"

# the jobs a ci-local-results.sh output marks passed, sorted onto one line
passed() {
  grep '_local=true$' | sed 's/_local=true$//' | sort | tr '\n' ' ' | sed 's/ $//'
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

# one status as the API returns it: context suffix, state, creator login
status() {
  printf '{"context":"cove-local-ci/%s","state":"%s","creator":{"login":"%s"},"created_at":"2026-11-05T18:00:00Z"}' "$1" "$2" "$3"
}

classify() {
  printf '%s' "$1" | "$script" classify | passed
}

expect "no statuses skips nothing" "" "$(classify '[]')"
expect "a trusted success skips its job" "compile_ios" \
  "$(classify "[$(status compile-ios success maintainer)]")"
expect "each context maps to its job" "compile_ios swift_lint test_macos" \
  "$(classify "[$(status compile-ios success maintainer),$(status swift-lint success other-maintainer),$(status test success maintainer)]")"
expect "an untrusted creator counts for nothing" "" \
  "$(classify "[$(status compile-ios success someone-else)]")"
expect "a failure skips nothing" "" "$(classify "[$(status test failure maintainer)]")"
expect "a newer failure overrides an older success" "" \
  "$(classify "[$(status test failure maintainer),$(status test success maintainer)]")"
expect "a newer success overrides an older failure" "test_macos" \
  "$(classify "[$(status test success maintainer),$(status test failure maintainer)]")"
expect "other contexts count for nothing" "" \
  "$(classify '[{"context":"compile-ios","state":"success","creator":{"login":"maintainer"}}]')"
expect "a status without a creator counts for nothing" "" \
  "$(classify '[{"context":"cove-local-ci/test","state":"success","creator":null}]')"
expect "an empty trusted list trusts nobody" "" \
  "$(TRUSTED="" classify "[$(status compile-ios success maintainer)]")"

# main with a gh on PATH that prints $2 and exits with $1; the result is the jobs passed in the
# $GITHUB_OUTPUT file the workflow reads, not stdout, so a broken output write fails the test
bin=$(mktemp -d)
trap 'rm -rf "$bin"' EXIT

main_with_gh() {
  local exit_code=$1 body=$2

  printf '#!/bin/sh\nprintf "%%s\\n" %q\nexit %s\n' "$body" "$exit_code" >"$bin/gh"
  chmod +x "$bin/gh"
  : >"$bin/output"
  PATH="$bin:$PATH" REPO=o/r SHA=abc GITHUB_OUTPUT="$bin/output" GITHUB_STEP_SUMMARY=/dev/null \
    "$script" >/dev/null 2>&1
  passed <"$bin/output"
}

expect "fetched statuses are classified" "swift_lint" \
  "$(main_with_gh 0 "[$(status swift-lint success maintainer)]")"
expect "unreadable statuses skip nothing" "" "$(main_with_gh 1 "")"
expect "malformed statuses skip nothing" "" "$(main_with_gh 0 "not-json")"
expect "the output names every job" "3" "$(main_with_gh 1 "" >/dev/null; grep -c '_local=false$' "$bin/output")"

if ((failures > 0)); then
  echo "$failures failed"
  exit 1
fi
