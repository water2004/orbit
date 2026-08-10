#!/usr/bin/env bash
set -euo pipefail

target="x86_64-unknown-linux-gnu"
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

metadata="$(cargo metadata --format-version 1 --no-deps)"
version="$(
	python3 -c 'import json, sys; print(next(package["version"] for package in json.load(sys.stdin)["packages"] if package["name"] == "orbit"))' \
		<<<"$metadata"
)"

for package in orbit-core orbit-launcher orbit-launcher-core orbit-gui; do
	package_version="$(
		python3 -c 'import json, sys; name = sys.argv[1]; print(next(package["version"] for package in json.load(sys.stdin)["packages"] if package["name"] == name))' \
			"$package" <<<"$metadata"
	)"
	if [[ "$package_version" != "$version" ]]; then
		echo "$package is $package_version, but the Orbit suite is $version" >&2
		exit 1
	fi
done

package_assets=(
	"README.md"
	"docs/orbit-cli-commands.md"
	"docs/orbit-launcher-cli.md"
	"docs/orbit-gui.md"
	"assets/orbit.desktop"
	"assets/orbit.svg"
)
for asset in "${package_assets[@]}"; do
	if [[ ! -f "$asset" ]]; then
		echo "Debian package asset does not exist: $asset" >&2
		exit 1
	fi
done

rustup target add "$target"

if [[ "$skip_cargo_build" != true ]]; then
	cargo build \
		--release \
		--locked \
		--package orbit \
		--package orbit-launcher \
		--package orbit-gui \
		--target "$target"
fi

bash ./orbit-runtime-agent/build.sh \
	"$repo_root/target/$target/release/orbit-runtime-agent.jar"
bash ./orbit-runtime-agent/test.sh \
	"$repo_root/target/$target/release/orbit-runtime-agent.jar"

for binary_name in orbit orbit-launcher orbit-gui; do
	binary="target/$target/release/$binary_name"
	if [[ ! -x "$binary" ]]; then
		echo "release executable not found at '$binary'" >&2
		exit 1
	fi
done
if [[ ! -f "target/$target/release/orbit-runtime-agent.jar" ]]; then
	echo "Orbit Runtime Agent was not built" >&2
	exit 1
fi

"target/$target/release/orbit" --help >/dev/null
"target/$target/release/orbit-launcher" --help >/dev/null

packages=(orbit orbit-launcher orbit-gui)
for package in "${packages[@]}"; do
	deb="target/debian/${package}_${version}-1_amd64.deb"
	mkdir -p "$(dirname "$deb")"
	cargo deb \
		--package "$package" \
		--target "$target" \
		--no-build \
		--output "$deb"

	if [[ ! -f "$deb" ]]; then
		echo "cargo-deb did not produce the expected $package amd64 package" >&2
		exit 1
	fi
	if [[ "$(dpkg-deb --field "$deb" Package)" != "$package" ]]; then
		echo "$deb has an unexpected Debian package name" >&2
		exit 1
	fi
	if [[ "$(dpkg-deb --field "$deb" Version)" != "$version-1" ]]; then
		echo "$deb has an unexpected Debian version" >&2
		exit 1
	fi
	if [[ "$package" == "orbit-gui" ]]; then
		depends="$(dpkg-deb --field "$deb" Depends)"
		grep -Fq "orbit (= $version-1)" <<<"$depends" || {
			echo "orbit-gui must depend on the matching orbit package" >&2
			exit 1
		}
		grep -Fq "orbit-launcher (= $version-1)" <<<"$depends" || {
			echo "orbit-gui must depend on the matching orbit-launcher package" >&2
			exit 1
		}
	fi
	if [[ "$package" == "orbit" ]]; then
		dpkg-deb --contents "$deb" | grep -F "./usr/lib/orbit/orbit-runtime-agent.jar" >/dev/null || {
			echo "orbit package must contain the Orbit Runtime Agent" >&2
			exit 1
		}
	fi

	dpkg-deb --info "$deb"
	dpkg-deb --contents "$deb"
	printf '%s\n' "$(realpath "$deb")"
done
