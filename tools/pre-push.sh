#!/usr/bin/env bash
# Runs the format, lint and manifest checks CI runs that fail locally in seconds, for the kinds of
# files a range of commits changes, so those failures never cost a CI round trip. `just pre-push`
# runs it, and so does the git hook `just install-hooks` enables; docs/testing.md#pre-push-checks
# describes it
#
# Usage: just pre-push [<revision>...]
# The revisions select the commits to check, as `git log` takes them; with none it checks the
# commits on HEAD that origin/master doesn't have. Skip some checks with
# COVE_PRE_PUSH_SKIP=rustfmt,qa,bindings,swiftformat,ktlint (any of them, or `all`)
#
# Exit codes, which the git hook and other callers rely on:
#   0  every check that ran passed, or the commits need none
#   1  a check failed
#   2  the revisions or the repository couldn't be read

# bash 3.2, the macOS system bash, runs this, so no associative arrays and no set -u, under which
# it treats an empty array as unset
set -o pipefail

warn() {
    printf 'pre-push: %s\n' "$*" >&2
}

root=$(git rev-parse --show-toplevel) || exit 2
cd "$root" || exit 2

# the checks never download anything: a missing crate or Gradle dependency fails instead
export CARGO_NET_OFFLINE=true

if [ $# -eq 0 ]; then
    if ! git rev-parse -q --verify 'origin/master^{commit}' >/dev/null; then
        warn "no origin/master to compare HEAD with; pass the revisions to check"
        exit 2
    fi

    set -- origin/master..HEAD
fi

# the files the commits change; --cc lists a merge's files only where it differs from every
# parent, its conflict resolutions, since the merged commits are listed on their own
paths=$(git -c core.quotePath=false -c log.showRoot=true log --format= --name-only --no-renames --cc "$@") || {
    warn "couldn't read the commits in: $*"
    exit 2
}

if [ -z "$paths" ]; then
    warn "no commits to check in: $*"
    exit 0
fi

# the commits at the tips of the range, whose trees the checks stand in for
tips=$(git rev-parse "$@" | grep -E '^[0-9a-f]{40,64}$')

# which checks the changed files need; CI's own path rules are in .github/scripts/ci_changes.py
run_rustfmt=false
run_qa=false
run_bindings=false
run_swiftformat=false
run_ktlint=false
while IFS= read -r path; do
    case $path in
        rust/*)
            run_rustfmt=true
            run_qa=true
            run_bindings=true
            ;;
        # checklist rows name app tests and device scripts too
        ios/* | android/* | qa/*) run_qa=true ;;
    esac

    case $path in
        *.swift | .swiftformat | tools/swift-tools.json) run_swiftformat=true ;;
        *.kt | *.kts | android/.editorconfig) run_ktlint=true ;;
    esac
done <<<"$paths"

skip_requested() {
    case ",${COVE_PRE_PUSH_SKIP:-}," in
        *",$1,"* | *,all,*) return 0 ;;
        *) return 1 ;;
    esac
}

have() {
    command -v "$1" >/dev/null 2>&1
}

# the checks read the working tree, so say so when it isn't what is being checked
dirty=""
for oid in $tips; do
    if ! git diff --quiet "$oid" -- rust ios android qa ||
        [ -n "$(git ls-files --others --exclude-standard -- rust ios android qa)" ]; then
        dirty=true
    fi
done

if [ -n "$dirty" ]; then
    warn "the working tree differs from the checked commits, so the checks read the working tree"
fi

failed=""
hints=""

# runs one check unless it was skipped or a tool it needs is missing, and on failure records how to
# fix it; `needs` is a function that prints why the check can't run here, or nothing
run_check() {
    local name=$1 needs=$2 hint=$3 missing started
    shift 3

    if skip_requested "$name"; then
        warn "skipping $name: COVE_PRE_PUSH_SKIP"
        return
    fi

    missing=$($needs)
    if [ -n "$missing" ]; then
        warn "skipping $name: $missing"
        return
    fi

    warn "running $name"
    started=$SECONDS
    if "$@" </dev/null; then
        warn "$name passed in $((SECONDS - started))s"
        return
    fi

    warn "$name FAILED in $((SECONDS - started))s"
    failed="$failed $name"
    hints="$hints"$'\n'"  $name: $hint"
}

needs_rustfmt() {
    if ! have cargo; then
        echo "cargo isn't installed"
    elif ! cargo fmt --version >/dev/null 2>&1; then
        echo "rustfmt isn't installed"
    fi
}

needs_cargo_and_just() {
    if ! have cargo; then
        echo "cargo isn't installed"
    elif ! have just; then
        echo "just isn't installed"
    fi
}

# the pinned swiftformat is a macOS binary that `just swift-tool` downloads on first use, which the
# hook must not do
needs_swiftformat() {
    if [ "$(uname -s)" != Darwin ]; then
        echo "the pinned swiftformat only runs on macOS"
        return
    fi

    local missing
    missing=$(needs_cargo_and_just)
    if [ -n "$missing" ]; then
        echo "$missing"
        return
    fi

    (cd rust && cargo --locked -q xtask swift-tools path swiftformat) >/dev/null 2>&1 ||
        echo "the pinned swiftformat isn't downloaded, or 'cargo xtask swift-tools path swiftformat' failed; 'just swift-tool swiftformat --version' downloads it"
}

# a first host build of the Rust library takes minutes, far over the hook's budget, so the check
# runs once `just bindings-check` has built it in this checkout; rebuilding it for a push's changes
# then takes seconds
needs_host_library() {
    local missing target_dir
    missing=$(needs_cargo_and_just)
    if [ -n "$missing" ]; then
        echo "$missing"
        return
    fi

    target_dir=$(cd rust && cargo metadata --format-version 1 --no-deps --locked 2>/dev/null |
        grep -o '"target_directory":"[^"]*"' | cut -d'"' -f4)
    if [ ! -f "$target_dir/debug/libcove.a" ]; then
        echo "no host build of the Rust library yet, run 'just bindings-check' once to check bindings on later pushes"
    fi
}

# gradlew stops at a JAVA_HOME without Java rather than falling back to the java on PATH, and
# downloads the Gradle release it pins unless an earlier unpack finished, which it marks with a
# `.zip.ok` file; --offline doesn't prevent the download
needs_gradle_and_just() {
    local distribution installed
    distribution=$(sed -n 's|^distributionUrl=.*/\(gradle-[^/]*\)\.zip$|\1|p' \
        android/gradle/wrapper/gradle-wrapper.properties)
    installed=$(find "${GRADLE_USER_HOME:-$HOME/.gradle}/wrapper/dists/$distribution" -mindepth 2 \
        -maxdepth 2 -name "$distribution.zip.ok" 2>/dev/null | head -1)

    if ! have just; then
        echo "just isn't installed"
    elif [ -n "${JAVA_HOME:-}" ] && [ ! -x "$JAVA_HOME/bin/java" ]; then
        echo "JAVA_HOME ($JAVA_HOME) has no bin/java"
    elif [ -z "${JAVA_HOME:-}" ] && ! have java; then
        echo "Java isn't installed"
    elif [ -z "$distribution" ] || [ -z "$installed" ]; then
        echo "${distribution:-the pinned Gradle} isn't fully downloaded, any ./gradlew command in android/ downloads it"
    fi
}

check_rustfmt() {
    (cd rust && cargo fmt --all -- --check)
}

# each runs the command of the CI job or step named beside it, with the same configs

# rustfmt job
if [ "$run_rustfmt" = true ]; then
    run_check rustfmt needs_rustfmt "run 'just fmt'" check_rustfmt
fi

# qa-manifest job, its manifest check
if [ "$run_qa" = true ]; then
    run_check qa needs_cargo_and_just "fix what 'just qa-check' reports, see qa/README.md" \
        just qa-check
fi

# swift-lint job, its SwiftFormat step
if [ "$run_swiftformat" = true ]; then
    run_check swiftformat needs_swiftformat "run 'just fmt-swift'" just lint-swift-format
fi

# ktlint job, its ktlint step; detekt stays in CI, since it compiles the app first
if [ "$run_ktlint" = true ]; then
    run_check ktlint needs_gradle_and_just "run 'just fmt-android'" just ktlint --offline
fi

# compile-android's and compile-ios's bindings checks, from a host build instead of device builds
if [ "$run_bindings" = true ]; then
    run_check bindings needs_host_library \
        "run 'just build-android' and 'just build-ios', or 'just regenerate-bindings' to have CI commit them" \
        just bindings-check
fi

if [ -n "$failed" ]; then
    warn "failed:$failed$hints"
    warn "skip a check with COVE_PRE_PUSH_SKIP=<check>, or push without the hook's checks with 'git push --no-verify'"
    exit 1
fi
