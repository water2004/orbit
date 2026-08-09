#!/usr/bin/env bash
set -euo pipefail

agent_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
workspace_root="$(cd "$agent_root/.." && pwd)"
output_path="${1:-$workspace_root/target/release/orbit-runtime-agent.jar}"
build_root="$workspace_root/target/orbit-runtime-agent"
dependency="$build_root/asm-9.9.1.jar"
classes="$build_root/classes"
manifest="$build_root/MANIFEST.MF"
expected_sha256="6f3828a215c920059a5efa2fb55c233d6c54ec5cadca99ce1b1bdd10077c7ddd"

mkdir -p "$build_root"
if [[ ! -f "$dependency" ]]; then
  curl --fail --location --output "$dependency" \
    "https://repo1.maven.org/maven2/org/ow2/asm/asm/9.9.1/asm-9.9.1.jar"
fi
actual_sha256="$(sha256sum "$dependency" | cut -d' ' -f1)"
[[ "$actual_sha256" == "$expected_sha256" ]] || {
  echo "ASM SHA-256 mismatch: $actual_sha256" >&2
  exit 1
}

rm -rf -- "$classes"
mkdir -p "$classes"
(cd "$classes" && jar xf "$dependency")
find "$classes/META-INF" -maxdepth 1 -type f \
  \( -name '*.SF' -o -name '*.RSA' -o -name '*.DSA' \) -delete 2>/dev/null || true
find "$classes" -type f -name 'module-info.class' -delete
mapfile -t sources < <(find "$agent_root/src/main/java" -type f -name '*.java' -print)
javac --release 8 -cp "$dependency" -d "$classes" "${sources[@]}"
mapfile -t java11_sources < <(find "$agent_root/src/main/java11" -type f -name '*.java' -print)
javac --release 11 -cp "$classes:$dependency" -d "$classes" "${java11_sources[@]}"
printf '%s\n' \
  'Manifest-Version: 1.0' \
  'Premain-Class: dev.orbit.agent.OrbitRuntimeAgent' \
  'Can-Redefine-Classes: false' \
  'Can-Retransform-Classes: false' \
  > "$manifest"
mkdir -p "$(dirname "$output_path")"
jar cfm "$output_path" "$manifest" -C "$classes" .
printf '%s\n' "$output_path"
