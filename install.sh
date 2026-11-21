#!/bin/sh
# Installs the prebuilt priory binary for this machine from the latest GitHub release.
#   curl -fsSL https://raw.githubusercontent.com/maxence-lefebvre/priory/main/install.sh | sh
#   sh install.sh --prefix /usr/local
# PRIORY_RELEASE_URL picks another release, for example
#   https://github.com/maxence-lefebvre/priory/releases/download/v0.1.0
set -eu

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

sha256() {
    if command -v sha256sum >/dev/null; then
        sha256sum "$1"
    else
        shasum -a 256 "$1"
    fi
}

# The whole script is one function, so a download cut short runs nothing.
main() {
    prefix="$HOME/.local"
    while [ $# -gt 0 ]; do
        case "$1" in
            --prefix) [ $# -ge 2 ] || fail "--prefix needs a directory"
                      prefix="$2"; shift 2 ;;
            --prefix=*) prefix="${1#--prefix=}"; shift ;;
            *) echo "usage: install.sh [--prefix DIR]" >&2; exit 2 ;;
        esac
    done
    url="${PRIORY_RELEASE_URL:-https://github.com/maxence-lefebvre/priory/releases/latest/download}"

    case "$(uname -s)/$(uname -m)" in
        Linux/x86_64 | Linux/amd64) target=x86_64-unknown-linux-musl ;;
        Linux/aarch64 | Linux/arm64) target=aarch64-unknown-linux-musl ;;
        Darwin/arm64) target=aarch64-apple-darwin ;;
        *) fail "no prebuilt priory for $(uname -s) $(uname -m); build it from source" ;;
    esac

    # Staging inside the destination keeps the final mv an atomic rename.
    mkdir -p "$prefix/bin"
    tmp="$(mktemp -d "$prefix/bin/.priory-install.XXXXXX")"
    trap 'rm -rf "$tmp"' EXIT
    asset="priory-$target"
    echo "Downloading $url/$asset"
    curl -fsSL -o "$tmp/$asset" "$url/$asset"
    curl -fsSL -o "$tmp/$asset.sha256" "$url/$asset.sha256"

    expected="$(cut -d' ' -f1 "$tmp/$asset.sha256")"
    case "$expected" in
        *[!0-9a-f]* | "") fail "no valid checksum in $asset.sha256" ;;
    esac
    [ "${#expected}" -eq 64 ] || fail "no valid checksum in $asset.sha256"
    actual="$(sha256 "$tmp/$asset")" || fail "could not compute the checksum of $asset"
    actual="${actual%% *}"
    [ "$expected" = "$actual" ] || fail "checksum mismatch for $asset: expected $expected, got $actual"

    chmod 755 "$tmp/$asset"
    mv -f "$tmp/$asset" "$prefix/bin/priory"
    echo "Installed $prefix/bin/priory"
    case ":$PATH:" in
        *":$prefix/bin:"*) ;;
        *) echo "Add $prefix/bin to your PATH to run priory." ;;
    esac
}

main "$@"
