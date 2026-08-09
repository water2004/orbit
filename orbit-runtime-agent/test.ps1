param(
    [string]$AgentPath = "target/debug/orbit-runtime-agent.jar"
)

$ErrorActionPreference = "Stop"
$AgentRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$WorkspaceRoot = Split-Path -Parent $AgentRoot
$TestRoot = Join-Path $WorkspaceRoot "target/orbit-runtime-agent-test"
$InstanceRoot = Join-Path $TestRoot "instance"
$ModsRoot = Join-Path $InstanceRoot "mods"
$ClassesRoot = Join-Path $TestRoot "classes"
$HarnessRoot = Join-Path $TestRoot "harness"
$FixtureJar = Join-Path $ModsRoot "agent-fixture.jar"
$SessionFile = Join-Path $InstanceRoot ".orbit/runtime-data/sessions/test.events"
$ContextFile = Join-Path $InstanceRoot ".orbit/runtime-data/agent-context.tsv"

if (Test-Path -LiteralPath $TestRoot) {
    $ResolvedWorkspace = [System.IO.Path]::GetFullPath((Join-Path $WorkspaceRoot "target"))
    $ResolvedTest = [System.IO.Path]::GetFullPath($TestRoot)
    if (-not $ResolvedTest.StartsWith($ResolvedWorkspace, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to clean Agent test data outside target"
    }
    Remove-Item -LiteralPath $TestRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $ModsRoot, (Join-Path $InstanceRoot "config"), (Split-Path -Parent $ContextFile), $ClassesRoot, $HarnessRoot | Out-Null
& javac --release 17 -d $ClassesRoot (Join-Path $AgentRoot "tests/AgentFixture.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile Agent fixture" }
& jar cf $FixtureJar -C $ClassesRoot .
if ($LASTEXITCODE -ne 0) { throw "Failed to package Agent fixture" }
& javac --release 17 -d $HarnessRoot (Join-Path $AgentRoot "tests/AgentIsolatedHarness.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile isolated-loader harness" }

$RootEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($InstanceRoot)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$SessionEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($SessionFile)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ContextEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($ContextFile)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ConfigEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes((Join-Path $InstanceRoot "config"))).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$FixtureHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $FixtureJar).Hash.ToLowerInvariant()
$ContextLines = @(
    "2`tcontext`tend"
    "source`t$FixtureHash`t$FixtureHash`tend"
    "reserved`t$ConfigEncoded`tend"
)
[System.IO.File]::WriteAllLines($ContextFile, $ContextLines, [System.Text.UTF8Encoding]::new($false))
$ResolvedAgent = [System.IO.Path]::GetFullPath((Join-Path $WorkspaceRoot $AgentPath))
& java "-javaagent:$ResolvedAgent=root=$RootEncoded;session=$SessionEncoded;context=$ContextEncoded" -cp $HarnessRoot AgentIsolatedHarness $FixtureJar $InstanceRoot
if ($LASTEXITCODE -ne 0) { throw "Agent fixture failed" }

$Records = Get-Content -LiteralPath $SessionFile
if ($Records.Count -ne 4) { throw "Expected three lasting creations and one published deletion, got $($Records.Count) records" }
if (-not ($Records -match "`ttree`t")) { throw "No owned directory tree was recorded" }
if (-not ($Records -match "`tfile`t")) { throw "No owned file was recorded" }
if (-not ($Records -match "2`tdelete`tfile`t")) { throw "No published deletion tombstone was recorded" }
Write-Output $SessionFile
