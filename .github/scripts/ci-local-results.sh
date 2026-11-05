#!/usr/bin/env bash
# Finds the macOS CI jobs a trusted maintainer already passed on their own Mac for this commit, so
# CI can skip them instead of waiting for one of GitHub's few macOS runners
#
#   ci-local-results.sh classify < statuses.json   prints <job>_local=true|false for a commit's
#                                                  statuses, as GET /commits/{sha}/statuses lists them
#   ci-local-results.sh                            in CI's changes job: fetches the statuses of $SHA
#                                                  and appends the same lines to $GITHUB_OUTPUT
#
# A local run posts a commit status for each job it ran, with the context cove-local-ci/<job>. A
# job counts as passed only when the newest status with its context is success and a login in
# $TRUSTED created it. Only accounts with push access can create statuses, and $TRUSTED narrows
# that to the maintainers whose machines run the same commands, so a fork can't skip a job. Any
# failure to fetch or parse the statuses prints a warning and reports nothing passed, so every job
# runs. ci-local-results-test.sh tests the rules and the fallback
#
#   REPO     owner/name, such as bitcoinppl/cove
#   SHA      the commit the local run tested: a pull request's head, not its merge commit
#   TRUSTED  space-separated logins whose statuses count
set -euo pipefail

# each CI job this can skip, as <output name>:<status context suffix>
JOBS="compile_ios:compile-ios swift_lint:swift-lint test_macos:test"

# prints one <job>_local=true|false line per job, from the statuses JSON on stdin
classify() {
  local statuses job output context

  statuses=$(cat)
  for job in $JOBS; do
    output=${job%%:*}
    context="cove-local-ci/${job#*:}"

    # the API lists statuses newest first, so the first one with the context is the current one
    if jq -e --arg context "$context" --arg trusted "${TRUSTED:-}" '
      ($trusted | split(" ") | map(select(. != ""))) as $logins
      | [.[] | select(.context == $context)][0]
      | . != null and .state == "success" and ((.creator.login // "") as $login | $logins | index($login) != null)
    ' <<<"$statuses" >/dev/null; then
      echo "${output}_local=true"
    else
      echo "${output}_local=false"
    fi
  done
}

# prints every job as not passed, for when the statuses can't be read
none_passed() {
  local job

  for job in $JOBS; do
    echo "${job%%:*}_local=false"
  done
}

main() {
  local statuses result

  : "${REPO:?}" "${SHA:?}"

  # a pull request's statuses fit on one page: each local run posts one per job
  if statuses=$(gh api "repos/$REPO/commits/$SHA/statuses?per_page=100") &&
    result=$(classify <<<"$statuses"); then
    echo "cove-local-ci statuses on $SHA:"
    jq -r '.[] | select(.context | startswith("cove-local-ci/")) | "  \(.context) \(.state) by \(.creator.login // "?") at \(.created_at)"' <<<"$statuses" || true
  else
    echo "::warning::couldn't read the commit statuses of $SHA, so every macOS job runs"
    result=$(none_passed)
  fi

  printf '%s\n' "$result"
  printf '%s\n' "$result" >>"${GITHUB_OUTPUT:-/dev/null}"

  # a skipped job looks the same in the checks whatever skipped it, so the summary says which
  # ones a local run stood in for
  if grep -q '=true$' <<<"$result"; then
    {
      echo "### macOS jobs passed by cove-local-ci on $SHA"
      grep '=true$' <<<"$result" | sed 's/_local=true$//; s/^/- /'
    } >>"${GITHUB_STEP_SUMMARY:-/dev/null}"
  fi
}

case ${1:-} in
  classify) classify ;;
  "") main ;;
  *)
    echo "usage: $0 [classify]" >&2
    exit 2
    ;;
esac
