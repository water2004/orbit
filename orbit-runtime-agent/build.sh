#!/usr/bin/env bash
set -euo pipefail

agent_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
workspace_root="$(cd "$agent_root/.." && pwd)"
output_path="${1:-$workspace_root/target/release/orbit-runtime-agent.jar}"
build_root="$workspace_root/target/orbit-runtime-agent"
dependency="$build_root/byte-buddy-1.18.7.jar"
classes="$build_root/classes"
manifest="$build_root/MANIFEST.MF"
expected_sha256="7c3b8fd43d75b2dc30bdb8cf7303d4b15f6f9a0ccb170f8b9f47de15864014f3"

mkdir -p "$build_root"
if [[ ! -f "$dependency" ]]; then
  curl --fail --location --output "$dependency" \
    "https://repo1.maven.org/maven2/net/bytebuddy/byte-buddy/1.18.7/byte-buddy-1.18.7.jar"
fi
actual_sha256="$(sha256sum "$dependency" | cut -d' ' -f1)"
[[ "$actual_sha256" == "$expected_sha256" ]] || {
  echo "Byte Buddy SHA-256 mismatch: $actual_sha256" >&2
  exit 1
}

rm -rf -- "$classes"
mkdir -p "$classes"
(cd "$classes" && jar xf "$dependency")
find "$classes/META-INF" -maxdepth 1 -type f \
  \( -name '*.SF' -o -name '*.RSA' -o -name '*.DSA' \) -delete 2>/dev/null || true
mapfile -t sources < <(find "$agent_root/src/main/java" -type f -name '*.java' -print)
javac --release 17 -cp "$dependency" -d "$classes" "${sources[@]}"
printf '%s\n' \
  'Manifest-Version: 1.0' \
  'Premain-Class: dev.orbit.agent.OrbitRuntimeAgent' \
  'Can-Redefine-Classes: false' \
  'Can-Retransform-Classes: false' \
  > "$manifest"
mkdir -p "$(dirname "$output_path")"
jar cfm "$output_path" "$manifest" -C "$classes" .
printf '%s\n' "$output_path"
