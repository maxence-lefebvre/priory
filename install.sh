#!/bin/sh
# Installs the prebuilt priory binary for this machine from the latest GitHub release.
#   curl -fsSL https://raw.githubusercontent.com/maxence-lefebvre/priory/main/install.sh | sh
#   sh install.sh --prefix /usr/local
# PRIORY_RELEASE_URL picks another release, for example
#   https://github.com/maxence-lefebvre/priory/releases/download/v0.1.0
set -eu

prefix="$HOME/.local"
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) [ $# -ge 2 ] || { echo "install.sh: --prefix needs a directory" >&2; exit 2; }
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
    *) echo "install.sh: no prebuilt priory for $(uname -s) $(uname -m); build it from source" >&2
       exit 1 ;;
esac

if command -v sha256sum >/dev/null; then
    sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
    sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
asset="priory-$target"
echo "Downloading $url/$asset"
curl -fsSL -o "$tmp/$asset" "$url/$asset"
curl -fsSL -o "$tmp/$asset.sha256" "$url/$asset.sha256"

expected="$(cut -d' ' -f1 "$tmp/$asset.sha256")"
actual="$(sha256 "$tmp/$asset")"
if [ "$expected" != "$actual" ]; then
    echo "install.sh: checksum mismatch for $asset: expected $expected, got $actual" >&2
    exit 1
fi

mkdir -p "$prefix/bin"
chmod 755 "$tmp/$asset"
mv -f "$tmp/$asset" "$prefix/bin/priory"
echo "Installed $prefix/bin/priory"
case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "Add $prefix/bin to your PATH to run priory." ;;
esac
