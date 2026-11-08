#!/usr/bin/env bash
# Plans one job of the UI tests workflow: whether it runs, and which tests
#
# With neither IOS_TESTS nor ANDROID_TESTS set, every job runs its full share: each iOS shard its
# slice of IOS_UI_SUITES, and each Android flavour all its tests. With either set, only the listed
# tests run, so a fix can be checked without the full suites: the iOS shards split the listed iOS
# tests as they split the full suites, and an Android flavour runs the listed tests its test
# sources hold. A job with nothing to run is skipped. Splitting a flavour's tests across its
# shards happens later in the job, in `xtask android-ui-shard`, which needs the Rust toolchain
#
#   PLATFORM=ios SHARD=<index>/<count>  or  PLATFORM=android FLAVOUR=<flavour>
#   IOS_TESTS      comma-separated <Target>/<Class>[/<method>] entries, <Target> CoveUITests or CoveCloudBackupUITests
#   ANDROID_TESTS  comma-separated <package>.<Class>[#<method>] entries
#   IOS_UI_SUITES  the iOS suites a full run covers
#   ANDROID_FLAVOURS  every flavour, which all run the shared androidTest sources
#
# Writes run, tests and, for iOS, shard to $GITHUB_OUTPUT, or prints them when it's unset
set -euo pipefail

: "${PLATFORM:?}"
ios_tests_input=$(printf '%s' "${IOS_TESTS:-}" | tr -d '[:space:]')
android_tests_input=$(printf '%s' "${ANDROID_TESTS:-}" | tr -d '[:space:]')
android_src=${ANDROID_SRC:-android/app/src}
targeted=false
[ -n "$ios_tests_input$android_tests_input" ] && targeted=true

# whether the entry's class is in the flavour's instrumented test sources, shared or its own
flavour_holds() {
    local flavour=$1
    local class=${2%%#*}
    local path=${class//.//}

    for set in androidTest "androidTest$flavour"; do
        for root in java kotlin; do
            for ext in kt java; do
                [ -f "$android_src/$set/$root/$path.$ext" ] && return 0
            done
        done
    done

    return 1
}

run=true
tests=
shard=

case $PLATFORM in
ios)
    : "${SHARD:?}" "${IOS_UI_SUITES:?}"

    # a listed class can hold as many tests as a full run, so listed tests are sharded too; the
    # workflow asks xtask for the shard's tests and skips the build when it gets none
    shard=$SHARD
    if [ "$targeted" = false ]; then
        tests=$IOS_UI_SUITES
    elif [ -n "$ios_tests_input" ]; then
        tests=$ios_tests_input
    else
        run=false
    fi
    ;;
android)
    : "${FLAVOUR:?}" "${ANDROID_FLAVOURS:?}"

    if [ "$targeted" = true ]; then
        IFS=',' read -ra entries <<<"$android_tests_input"
        held=()
        for entry in "${entries[@]}"; do
            [ -n "$entry" ] || continue

            # an entry no flavour holds is a typo, which would otherwise pass by running nothing
            found=false
            for flavour in $ANDROID_FLAVOURS; do
                flavour_holds "$flavour" "$entry" && found=true
            done

            if [ "$found" = false ]; then
                echo "::error::no Android instrumented test source holds ${entry%%#*}" >&2
                exit 1
            fi

            flavour_holds "$FLAVOUR" "$entry" && held+=("$entry")
        done

        tests=$(IFS=','; echo "${held[*]}")
        [ -n "$tests" ] || run=false
    fi
    ;;
*)
    echo "::error::PLATFORM must be ios or android, got $PLATFORM" >&2
    exit 1
    ;;
esac

{
    echo "run=$run"
    echo "tests=$tests"
    if [ "$PLATFORM" = ios ]; then
        echo "shard=$shard"
    fi
} >>"${GITHUB_OUTPUT:-/dev/stdout}"
