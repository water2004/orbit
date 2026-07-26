#!/usr/bin/env bash
set -euo pipefail

target="x86_64-unknown-linux-gnu"
package="orbit"
skip_cargo_build=false

if [[ "${1:-}" == "--skip-cargo-build" ]]; then
	skip_cargo_build=true
elif [[ $# -ne 0 ]]; then
	echo "usage: $0 [--skip-cargo-build]" >&2
	exit 2
fi

if [[ "$(uname -s)" != "Linux" ]]; then
	echo "Debian packages must be built on Linux. Use the tag release GitHub Actions workflow from Windows." >&2
	exit 1
fi

if ! command -v cargo-deb >/dev/null 2>&1; then
	echo "cargo-deb 3.7.0 is required: cargo install cargo-deb --version 3.7.0 --locked" >&2
	exit 1
fi
if [[ "$(cargo deb --version)" != "cargo-deb 3.7.0" ]]; then
	echo "cargo-deb 3.7.0 is required; found: $(cargo deb --version)" >&2
	exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

version="$(
	cargo metadata \
		--format-version 1 \
		--no-deps \
		--manifest-path orbit-cli/Cargo.toml |
		python3 -c 'import json, sys; print(next(package["version"] for package in json.load(sys.stdin)["packages"] if package["name"] == "orbit"))'
)"
rustup target add "$target"

if [[ "$skip_cargo_build" != true ]]; then
	cargo build \
		--release \
		--locked \
		--package "$package" \
		--target "$target"
fi

binary="target/$target/release/orbit"
if [[ ! -x "$binary" ]]; then
	echo "release executable not found at '$binary'" >&2
	exit 1
fi

"$binary" --help >/dev/null

cargo deb \
	--package "$package" \
	--target "$target" \
	--no-build

deb="target/debian/orbit_${version}-1_amd64.deb"
if [[ ! -f "$deb" ]]; then
	echo "cargo-deb did not produce an amd64 Orbit package" >&2
	exit 1
fi

dpkg-deb --info "$deb"
dpkg-deb --contents "$deb"
printf '%s\n' "$(realpath "$deb")"
