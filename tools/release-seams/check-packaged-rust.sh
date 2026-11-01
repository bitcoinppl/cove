#!/usr/bin/env bash
# Fails when a Rust library a release build is about to package has a UI test seam
#
# Xcode and Gradle package whichever Rust library was built last, and a debug one carries the
# seams, see docs/testing.md. The markers in rust-markers.txt are strings only the seam code has;
# `cargo xtask check-release-seams` uses the same list
#
# Usage: check-packaged-rust.sh <library file or directory>...
set -euo pipefail

markers="$(dirname "$0")/rust-markers.txt"
status=0

for path in "$@"; do
    # grep can't tell a missing path from a clean one, so a library that isn't there fails
    if [ ! -e "$path" ]; then
        echo "error: no Rust library at $path" >&2
        exit 1
    fi
done

while IFS= read -r marker; do
    [ -n "$marker" ] || continue

    # grep exits 1 when nothing matches; anything above that, such as a read the Xcode script
    # sandbox denies, must fail rather than pass as clean
    found=$(LC_ALL=C grep -rlaF -- "$marker" "$@") && code=0 || code=$?
    case "$code" in
        0)
            echo "error: $found has the UI test seam marker $marker; rebuild the Rust library with the release-speed profile" >&2
            status=1
            ;;
        1) ;;
        *)
            echo "error: couldn't scan $* for UI test seams" >&2
            exit 1
            ;;
    esac
done < "$markers"

exit "$status"
