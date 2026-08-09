#!/usr/bin/env bash
set -euo pipefail

agent_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
workspace_root="$(cd "$agent_root/.." && pwd)"
agent_path="${1:-$workspace_root/target/debug/orbit-runtime-agent.jar}"
java_command="${2:-java}"
test_root="$workspace_root/target/orbit-runtime-agent-test"
instance_root="$test_root/instance"
mods_root="$instance_root/mods"
classes_root="$test_root/classes"
harness_root="$test_root/harness"
fixture_jar="$mods_root/agent-fixture.jar"
session_file="$instance_root/.orbit/runtime-data/sessions/test.events"
context_file="$instance_root/.orbit/runtime-data/agent-context.tsv"

case "$test_root" in
  "$workspace_root"/target/*) ;;
  *)
    echo "Refusing to clean Agent test data outside target: $test_root" >&2
    exit 1
    ;;
esac
rm -rf -- "$test_root"
mkdir -p "$mods_root" "$instance_root/config" "$(dirname "$context_file")" "$classes_root" "$harness_root"

javac --release 8 -d "$classes_root" "$agent_root/tests/AgentFixture.java"
jar cf "$fixture_jar" -C "$classes_root" .
javac --release 8 -d "$harness_root" "$agent_root/tests/AgentIsolatedHarness.java"

encode_path() {
  printf '%s' "$1" | base64 | tr -d '\n=' | tr '+/' '-_'
}

root_encoded="$(encode_path "$instance_root")"
session_encoded="$(encode_path "$session_file")"
context_encoded="$(encode_path "$context_file")"
config_encoded="$(encode_path "$instance_root/config")"
fixture_hash="$(sha256sum "$fixture_jar" | awk '{print $1}')"
printf '3\tcontext\tend\ncapability\tjava\t8-25\tend\ncapability\tsource\tfile\tend\nsource\t%s\t%s\tend\nreserved\t%s\tend\n' \
  "$fixture_hash" "$fixture_hash" "$config_encoded" > "$context_file"
"$java_command" "-javaagent:$agent_path=root=$root_encoded;session=$session_encoded;context=$context_encoded" \
  -cp "$harness_root" AgentIsolatedHarness "$fixture_jar" "$instance_root"

record_count="$(wc -l < "$session_file")"
if (( record_count != 4 )); then
  echo "Expected three lasting creations and one published deletion, got $record_count records" >&2
  exit 1
fi
grep -q $'\ttree\t' "$session_file" || {
  echo "No owned directory tree was recorded" >&2
  exit 1
}
grep -q $'\tfile\t' "$session_file" || {
  echo "No owned file was recorded" >&2
  exit 1
}
grep -q $'^2\tdelete\tfile\t' "$session_file" || {
  echo "No published deletion tombstone was recorded" >&2
  exit 1
}
printf '%s\n' "$session_file"

# The Agent has a Java 8 baseline but must still rewrite APIs introduced by
# newer JDKs at application call sites.
modern_root="$test_root/java11-apis"
modern_classes="$modern_root/classes"
modern_instance="$modern_root/instance"
modern_jar="$modern_root/agent-modern-fixture.jar"
modern_session="$modern_instance/.orbit/runtime-data/sessions/test.events"
modern_context="$modern_instance/.orbit/runtime-data/agent-context.tsv"
mkdir -p "$modern_classes" "$modern_instance/config" "$(dirname "$modern_session")"
javac --release 11 -d "$modern_classes" "$agent_root/tests/AgentModernFixture.java"
jar cf "$modern_jar" -C "$modern_classes" .
modern_hash="$(sha256sum "$modern_jar" | awk '{print $1}')"
modern_root_encoded="$(encode_path "$modern_instance")"
modern_session_encoded="$(encode_path "$modern_session")"
modern_context_encoded="$(encode_path "$modern_context")"
printf '3\tcontext\tend\ncapability\tjava\t8-25\tend\ncapability\tsource\tfile\tend\nsource\t%s\t%s\tend\n' \
  "$modern_hash" "$modern_hash" > "$modern_context"
java "-javaagent:$agent_path=root=$modern_root_encoded;session=$modern_session_encoded;context=$modern_context_encoded" \
  -cp "$harness_root" AgentIsolatedHarness "$modern_jar" "$modern_instance" AgentModernFixture
if (( $(wc -l < "$modern_session") != 2 )); then
  echo "Java 11 write APIs were not both observed" >&2
  exit 1
fi
printf '%s\n' "$modern_session"

# Verify the first Forge union CodeSource generation as a named, unexported
# Java module. This exercises the same redefineModule path used by Forge 1.17.
compatibility_root="$test_root/forge-union-0.9.54"
dependency_root="$workspace_root/target/orbit-runtime-agent/compatibility"
mkdir -p "$compatibility_root/classes" "$compatibility_root/instance/config" \
  "$compatibility_root/instance/.orbit/runtime-data/sessions" "$dependency_root"
download_checked() {
  local name="$1" url="$2" expected="$3" path="$dependency_root/$name"
  if [[ ! -f "$path" ]]; then
    curl --fail --location --output "$path" "$url"
  fi
  local actual
  actual="$(sha256sum "$path" | cut -d' ' -f1)"
  [[ "$actual" == "$expected" ]] || {
    echo "$name SHA-256 mismatch: $actual" >&2
    exit 1
  }
}
download_checked securejarhandler-0.9.54.jar \
  https://maven.minecraftforge.net/cpw/mods/securejarhandler/0.9.54/securejarhandler-0.9.54.jar \
  823c9ff565c3f29013ab17d20a03e5ba178675f1f0d0a0e2b7b8355bbadb07db
download_checked asm-9.1.jar \
  https://repo1.maven.org/maven2/org/ow2/asm/asm/9.1/asm-9.1.jar \
  cda4de455fab48ff0bcb7c48b4639447d4de859a7afc30a094a986f0936beba2
download_checked asm-tree-9.1.jar \
  https://repo1.maven.org/maven2/org/ow2/asm/asm-tree/9.1/asm-tree-9.1.jar \
  fd00afa49e9595d7646205b09cecb4a776a8ff0ba06f2d59b8f7bf9c704b4a73

javac --release 17 -d "$compatibility_root/classes" "$agent_root/tests/AgentUnionHarness.java"
union_instance="$compatibility_root/instance"
union_session="$union_instance/.orbit/runtime-data/sessions/test.events"
union_context="$union_instance/.orbit/runtime-data/agent-context.tsv"
union_root_encoded="$(encode_path "$union_instance")"
union_session_encoded="$(encode_path "$union_session")"
union_context_encoded="$(encode_path "$union_context")"
printf '3\tcontext\tend\ncapability\tjava\t8-25\tend\ncapability\tsource\tunion\tend\nsource\t%s\t%s\tend\n' \
  "$fixture_hash" "$fixture_hash" > "$union_context"
module_path="$dependency_root/securejarhandler-0.9.54.jar:$dependency_root/asm-9.1.jar:$dependency_root/asm-tree-9.1.jar"
java "-javaagent:$agent_path=root=$union_root_encoded;session=$union_session_encoded;context=$union_context_encoded" \
  --module-path "$module_path" --add-modules cpw.mods.securejarhandler \
  -cp "$compatibility_root/classes" AgentUnionHarness "$fixture_jar" "$union_instance"
union_record_count="$(wc -l < "$union_session")"
if (( union_record_count != 4 )); then
  echo "Forge union CodeSource did not resolve to the fixture package" >&2
  exit 1
fi
printf '%s\n' "$union_session"

# Quilt 0.18.1+ uses a native CodeSource identity when classes come from its
# virtual filesystem. Test against the current public Quilt interfaces.
download_checked quilt-loader-0.30.1-beta.2.jar \
  https://maven.quiltmc.org/repository/release/org/quiltmc/quilt-loader/0.30.1-beta.2/quilt-loader-0.30.1-beta.2.jar \
  9e5801c55cdb881d5b29967096c08e39131a8fab7f88585bd06ec31b1c5144a6
quilt_root="$test_root/quilt-0.30.1"
quilt_instance="$quilt_root/instance"
quilt_classes="$quilt_root/classes"
quilt_session="$quilt_instance/.orbit/runtime-data/sessions/test.events"
quilt_context="$quilt_instance/.orbit/runtime-data/agent-context.tsv"
quilt_jar="$dependency_root/quilt-loader-0.30.1-beta.2.jar"
mkdir -p "$quilt_classes" "$quilt_instance/config" "$(dirname "$quilt_session")"
javac --release 17 -cp "$quilt_jar" -d "$quilt_classes" "$agent_root/tests/AgentQuiltHarness.java"
quilt_root_encoded="$(encode_path "$quilt_instance")"
quilt_session_encoded="$(encode_path "$quilt_session")"
quilt_context_encoded="$(encode_path "$quilt_context")"
module_encoded="$(encode_path agent-fixture)"
printf '3\tcontext\tend\ncapability\tjava\t8-25\tend\ncapability\tmodule\tquilt\tend\nmodule\t%s\t%s\tend\n' \
  "$module_encoded" "$fixture_hash" > "$quilt_context"
java "-javaagent:$agent_path=root=$quilt_root_encoded;session=$quilt_session_encoded;context=$quilt_context_encoded" \
  -cp "$quilt_classes:$quilt_jar" AgentQuiltHarness "$fixture_jar" "$quilt_instance"
quilt_record_count="$(wc -l < "$quilt_session")"
if (( quilt_record_count != 4 )); then
  echo "Quilt native module identity did not resolve to the fixture package" >&2
  exit 1
fi
printf '%s\n' "$quilt_session"
