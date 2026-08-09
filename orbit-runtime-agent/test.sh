#!/usr/bin/env bash
set -euo pipefail

agent_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
workspace_root="$(cd "$agent_root/.." && pwd)"
agent_path="${1:-$workspace_root/target/debug/orbit-runtime-agent.jar}"
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

javac --release 17 -d "$classes_root" "$agent_root/tests/AgentFixture.java"
jar cf "$fixture_jar" -C "$classes_root" .
javac --release 17 -d "$harness_root" "$agent_root/tests/AgentIsolatedHarness.java"

encode_path() {
  printf '%s' "$1" | base64 | tr -d '\n=' | tr '+/' '-_'
}

root_encoded="$(encode_path "$instance_root")"
session_encoded="$(encode_path "$session_file")"
context_encoded="$(encode_path "$context_file")"
config_encoded="$(encode_path "$instance_root/config")"
fixture_hash="$(sha256sum "$fixture_jar" | awk '{print $1}')"
printf '2\tcontext\tend\nsource\t%s\t%s\tend\nreserved\t%s\tend\n' \
  "$fixture_hash" "$fixture_hash" "$config_encoded" > "$context_file"
java "-javaagent:$agent_path=root=$root_encoded;session=$session_encoded;context=$context_encoded" \
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
