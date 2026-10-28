#!/usr/bin/env bash
# Runs a pinned swiftformat or swiftlint release so local lint matches CI
#
# Usage: tools/swift-tool.sh <swiftformat|swiftlint> [args...]
#
# Homebrew only ships the latest release, so CI and local machines drift apart
# whenever a new version adds or changes a rule. This downloads the exact
# release binary once, verifies its checksum, and caches it per version
#
# To bump a tool, update its version and the sha256 of the release asset, which
# GitHub shows as the asset digest:
#   gh api repos/<owner>/<repo>/releases/tags/<version> -q '.assets[] | .name + " " + .digest'

set -euo pipefail

SWIFTFORMAT_VERSION="0.64.4"
SWIFTFORMAT_SHA256="bb42fbfb0b9d5f1b8b83db3147087220ea414916fce8c5cf71abaea5ad5a3175"

SWIFTLINT_VERSION="0.65.1"
SWIFTLINT_SHA256="c1e429b0599cf1b516f369a2d9ec04eaf0e436f3c12b637df8851fa52ff694d0"

tool="${1:-}"
if [[ $# -gt 0 ]]; then
    shift
fi

case "$tool" in
swiftformat)
    version="$SWIFTFORMAT_VERSION"
    sha256="$SWIFTFORMAT_SHA256"
    url="https://github.com/nicklockwood/SwiftFormat/releases/download/$version/swiftformat.zip"
    ;;
swiftlint)
    version="$SWIFTLINT_VERSION"
    sha256="$SWIFTLINT_SHA256"
    url="https://github.com/realm/SwiftLint/releases/download/$version/portable_swiftlint.zip"
    ;;
*)
    echo "usage: $0 <swiftformat|swiftlint> [args...]" >&2
    exit 2
    ;;
esac

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "error: $0 only supports macOS release binaries" >&2
    exit 1
fi

install_dir="${XDG_CACHE_HOME:-$HOME/.cache}/cove/swift-tools/$tool-$version"
binary="$install_dir/$tool"

if [[ ! -x "$binary" ]]; then
    echo "Installing $tool $version to $install_dir" >&2

    # download inside the install dir so the final rename is atomic on the same volume
    mkdir -p "$install_dir"
    tmp_dir="$(mktemp -d "$install_dir/download.XXXXXX")"
    trap 'rm -rf "$tmp_dir"' EXIT

    curl --fail --silent --show-error --location --retry 3 --output "$tmp_dir/$tool.zip" "$url"
    echo "$sha256  $tmp_dir/$tool.zip" | shasum -a 256 --check --status || {
        echo "error: checksum mismatch for $url" >&2
        exit 1
    }

    unzip -q "$tmp_dir/$tool.zip" "$tool" -d "$tmp_dir"
    chmod +x "$tmp_dir/$tool"
    mv -f "$tmp_dir/$tool" "$binary"
    rm -rf "$tmp_dir"
    trap - EXIT
fi

exec "$binary" "$@"
