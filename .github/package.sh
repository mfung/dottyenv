#!/usr/bin/env bash
# Package a built binary into dist/dottyenv-<version>-<target>.tar.gz
#
# Shared by the macos and linux jobs so the archive layout cannot drift between
# platforms. The Homebrew formula and the install script both depend on the
# binary sitting at the root of the archive.
set -euo pipefail

target="${1:?usage: package.sh <target-triple>}"
version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
name="dottyenv-${version}-${target}"

staging=$(mktemp -d)
cp "target/${target}/release/dottyenv" "${staging}/"
cp README.md "${staging}/"
[ -f LICENSE ] && cp LICENSE "${staging}/"

mkdir -p dist
tar -czf "dist/${name}.tar.gz" -C "${staging}" .
rm -rf "${staging}"

echo "packaged dist/${name}.tar.gz"
tar -tzf "dist/${name}.tar.gz"
