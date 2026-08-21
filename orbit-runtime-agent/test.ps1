param(
    [string]$AgentPath = "target/debug/orbit-runtime-agent.jar",
    [string]$JavaCommand = "java"
)

$ErrorActionPreference = "Stop"
$AgentRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$WorkspaceRoot = Split-Path -Parent $AgentRoot
$TestRoot = Join-Path $WorkspaceRoot "target/orbit-runtime-agent-test"
$InstanceRoot = Join-Path $TestRoot "instance"
$ModsRoot = Join-Path $InstanceRoot "mods"
$ClassesRoot = Join-Path $TestRoot "classes"
$HarnessRoot = Join-Path $TestRoot "harness"
$AsmDependency = Join-Path $WorkspaceRoot "target/orbit-runtime-agent/asm-9.9.1.jar"
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
& javac --release 8 -d $ClassesRoot (Join-Path $AgentRoot "tests/AgentFixture.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile Agent fixture" }
& jar cf $FixtureJar -C $ClassesRoot .
if ($LASTEXITCODE -ne 0) { throw "Failed to package Agent fixture" }
& javac --release 8 -d $HarnessRoot `
    (Join-Path $AgentRoot "tests/AgentClasspathHarness.java") `
    (Join-Path $AgentRoot "tests/AgentDelegationHarness.java") `
    (Join-Path $AgentRoot "tests/AgentIsolatedHarness.java") `
    (Join-Path $AgentRoot "tests/AgentOwnershipHarness.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile isolated-loader harness" }

$RootEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($InstanceRoot)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$SessionEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($SessionFile)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ContextEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($ContextFile)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ConfigEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes((Join-Path $InstanceRoot "config"))).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$FixtureHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $FixtureJar).Hash.ToLowerInvariant()
$FixturePackage = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("agent-fixture")).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ContextLines = @(
    "4`tcontext`tend"
    "capability`tjava`t8-25`tend"
    "capability`tsource`tfile`tend"
    "source`t$FixtureHash`t$FixtureHash`tend"
    "package`t$FixtureHash`t$FixturePackage`tend"
    "reserved`t$ConfigEncoded`tend"
)
[System.IO.File]::WriteAllLines($ContextFile, $ContextLines, [System.Text.UTF8Encoding]::new($false))
$ResolvedAgent = [System.IO.Path]::GetFullPath((Join-Path $WorkspaceRoot $AgentPath))
$AgentEntries = & jar tf $ResolvedAgent
if ($LASTEXITCODE -ne 0) { throw "Failed to inspect Orbit Runtime Agent" }
if ($AgentEntries -contains "org/objectweb/asm/ClassReader.class") {
    throw "Orbit Runtime Agent exposes an unrelocated ASM ClassReader"
}
if (-not ($AgentEntries -contains "dev/orbit/shd/asm/ClassReader.class")) {
    throw "Orbit Runtime Agent is missing its relocated ASM ClassReader"
}
if (-not (Test-Path -LiteralPath $AsmDependency -PathType Leaf)) {
    throw "ASM test dependency is missing; build the Agent before running its tests"
}

$ClasspathRoot = Join-Path $TestRoot "classpath-isolation"
$ClasspathInstance = Join-Path $ClasspathRoot "instance"
$ClasspathSession = Join-Path $ClasspathInstance ".orbit/runtime-data/sessions/test.events"
$ClasspathContext = Join-Path $ClasspathInstance ".orbit/runtime-data/agent-context.tsv"
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $ClasspathSession) | Out-Null
$ClasspathRootEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($ClasspathInstance)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ClasspathSessionEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($ClasspathSession)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ClasspathContextEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($ClasspathContext)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
[System.IO.File]::WriteAllLines($ClasspathContext, @(
    "4`tcontext`tend"
    "capability`tjava`t8-25`tend"
    "capability`tsource`tfile`tend"
), [System.Text.UTF8Encoding]::new($false))
$PublicAsmClasspath = "$HarnessRoot$([System.IO.Path]::PathSeparator)$AsmDependency"
& $JavaCommand "-javaagent:$ResolvedAgent=root=$ClasspathRootEncoded;session=$ClasspathSessionEncoded;context=$ClasspathContextEncoded" `
    -cp $PublicAsmClasspath AgentClasspathHarness
if ($LASTEXITCODE -ne 0) { throw "Agent ASM classpath isolation failed" }

& $JavaCommand "-javaagent:$ResolvedAgent=root=$RootEncoded;session=$SessionEncoded;context=$ContextEncoded" -cp $HarnessRoot AgentIsolatedHarness $FixtureJar $InstanceRoot
if ($LASTEXITCODE -ne 0) { throw "Agent fixture failed" }

$Records = Get-Content -LiteralPath $SessionFile
if ($Records.Count -ne 5) { throw "Expected one header, three lasting creations and one published deletion, got $($Records.Count) lines" }
if ($Records[0] -notmatch '^4\tsnapshot\t') { throw "No v4 snapshot header was recorded" }
if (-not ($Records -match "`ttree`t")) { throw "No owned directory tree was recorded" }
if (-not ($Records -match "`tfile`t")) { throw "No owned file was recorded" }
if (-not ($Records -match "4`tdelete`tfile`t")) { throw "No published deletion tombstone was recorded" }
if (Test-Path -LiteralPath (Join-Path $InstanceRoot ".orbit/runtime-data/observation.active")) {
    throw "Observation activity marker survived JVM shutdown"
}
if (-not (Test-Path -LiteralPath (Join-Path $InstanceRoot ".orbit/runtime-data/observation.epoch"))) {
    throw "Observation epoch marker was not published"
}
Write-Output $SessionFile

# Ownership is the package that performed the final successful mutation, not
# the creator and not an accumulated set of writers.
$OwnershipRoot = Join-Path $TestRoot "last-writer"
$OwnershipInstance = Join-Path $OwnershipRoot "instance"
$OwnerAClasses = Join-Path $OwnershipRoot "owner-a"
$OwnerBClasses = Join-Path $OwnershipRoot "owner-b"
$OwnerAJar = Join-Path $OwnershipRoot "owner-a.jar"
$OwnerBJar = Join-Path $OwnershipRoot "owner-b.jar"
$OwnershipSession = Join-Path $OwnershipInstance ".orbit/runtime-data/sessions/test.events"
$OwnershipContext = Join-Path $OwnershipInstance ".orbit/runtime-data/agent-context.tsv"
New-Item -ItemType Directory -Force -Path `
    $OwnerAClasses, $OwnerBClasses, (Join-Path $OwnershipInstance "config"), `
    (Split-Path -Parent $OwnershipSession) | Out-Null
& javac --release 8 -d $OwnerAClasses (Join-Path $AgentRoot "tests/AgentOwnerA.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile owner A fixture" }
& javac --release 8 -d $OwnerBClasses (Join-Path $AgentRoot "tests/AgentOwnerB.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile owner B fixture" }
& jar cf $OwnerAJar -C $OwnerAClasses .
if ($LASTEXITCODE -ne 0) { throw "Failed to package owner A fixture" }
& jar cf $OwnerBJar -C $OwnerBClasses .
if ($LASTEXITCODE -ne 0) { throw "Failed to package owner B fixture" }
$OwnerAHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $OwnerAJar).Hash.ToLowerInvariant()
$OwnerBHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $OwnerBJar).Hash.ToLowerInvariant()
$OwnerAPackage = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("owner-a")).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$OwnerBPackage = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("owner-b")).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$OwnershipRootEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($OwnershipInstance)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$OwnershipSessionEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($OwnershipSession)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$OwnershipContextEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($OwnershipContext)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
[System.IO.File]::WriteAllLines($OwnershipContext, @(
    "4`tcontext`tend"
    "capability`tjava`t8-25`tend"
    "capability`tsource`tfile`tend"
    "source`t$OwnerAHash`t$OwnerAHash`tend"
    "source`t$OwnerBHash`t$OwnerBHash`tend"
    "package`t$OwnerAHash`t$OwnerAPackage`tend"
    "package`t$OwnerBHash`t$OwnerBPackage`tend"
), [System.Text.UTF8Encoding]::new($false))
& $JavaCommand "-javaagent:$ResolvedAgent=root=$OwnershipRootEncoded;session=$OwnershipSessionEncoded;context=$OwnershipContextEncoded" `
    -cp $HarnessRoot AgentOwnershipHarness $OwnerAJar $OwnerBJar $OwnershipInstance
if ($LASTEXITCODE -ne 0) { throw "Last-writer Agent fixture failed" }
$OwnershipRecords = Get-Content -LiteralPath $OwnershipSession
if ($OwnershipRecords.Count -ne 2) { throw "Last-writer snapshot was not compacted to one path" }
if ($OwnershipRecords[1] -notmatch "^4`twrite`tfile`t$OwnerAPackage`t3`t") {
    throw "The package that performed the final write did not own the file"
}
$FirstSnapshotHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $OwnershipSession).Hash
& $JavaCommand "-javaagent:$ResolvedAgent=root=$OwnershipRootEncoded;session=$OwnershipSessionEncoded;context=$OwnershipContextEncoded" `
    -cp $HarnessRoot AgentOwnershipHarness $OwnerAJar $OwnerBJar $OwnershipInstance
if ($LASTEXITCODE -ne 0) { throw "Restarted last-writer Agent fixture failed" }
$OwnershipSnapshots = Get-ChildItem -LiteralPath (Split-Path -Parent $OwnershipSession) -Filter "*.events"
if ($OwnershipSnapshots.Count -ne 2) { throw "A restarted JVM overwrote the previous snapshot" }
if ((Get-FileHash -Algorithm SHA256 -LiteralPath $OwnershipSession).Hash -ne $FirstSnapshotHash) {
    throw "The original JVM snapshot changed after restart"
}
Write-Output $OwnershipSession

# A package may delegate persistence to a declared library dependency. The
# outer logical caller, rather than the helper JAR containing Files.write, owns
# the resulting path.
$DelegationRoot = Join-Path $TestRoot "delegation"
$DelegationInstance = Join-Path $DelegationRoot "instance"
$DelegationLibraryClasses = Join-Path $DelegationRoot "library-classes"
$DelegationConsumerClasses = Join-Path $DelegationRoot "consumer-classes"
$DelegationLibraryJar = Join-Path $DelegationRoot "library.jar"
$DelegationConsumerJar = Join-Path $DelegationRoot "consumer.jar"
$DelegationSession = Join-Path $DelegationInstance ".orbit/runtime-data/sessions/test.events"
$DelegationContext = Join-Path $DelegationInstance ".orbit/runtime-data/agent-context.tsv"
New-Item -ItemType Directory -Force -Path `
    $DelegationLibraryClasses, $DelegationConsumerClasses, `
    (Join-Path $DelegationInstance "config"), (Split-Path -Parent $DelegationSession) | Out-Null
& javac --release 8 -d $DelegationLibraryClasses (Join-Path $AgentRoot "tests/AgentDelegateLibrary.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile delegation library fixture" }
& jar cf $DelegationLibraryJar -C $DelegationLibraryClasses .
if ($LASTEXITCODE -ne 0) { throw "Failed to package delegation library fixture" }
& javac --release 8 -cp $DelegationLibraryJar -d $DelegationConsumerClasses `
    (Join-Path $AgentRoot "tests/AgentDelegateConsumer.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile delegation consumer fixture" }
& jar cf $DelegationConsumerJar -C $DelegationConsumerClasses .
if ($LASTEXITCODE -ne 0) { throw "Failed to package delegation consumer fixture" }
$DelegationLibraryHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $DelegationLibraryJar).Hash.ToLowerInvariant()
$DelegationConsumerHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $DelegationConsumerJar).Hash.ToLowerInvariant()
$DelegationLibraryPackage = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("delegation-library")).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$DelegationConsumerPackage = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("delegation-consumer")).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$DelegationRootEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($DelegationInstance)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$DelegationSessionEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($DelegationSession)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$DelegationContextEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($DelegationContext)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
[System.IO.File]::WriteAllLines($DelegationContext, @(
    "4`tcontext`tend"
    "capability`tjava`t8-25`tend"
    "capability`tsource`tfile`tend"
    "source`t$DelegationLibraryHash`t$DelegationLibraryHash`tend"
    "source`t$DelegationConsumerHash`t$DelegationConsumerHash`tend"
    "delegation`t$DelegationConsumerHash`t$DelegationLibraryHash`tend"
    "package`t$DelegationLibraryHash`t$DelegationLibraryPackage`tend"
    "package`t$DelegationConsumerHash`t$DelegationConsumerPackage`tend"
), [System.Text.UTF8Encoding]::new($false))
& $JavaCommand "-javaagent:$ResolvedAgent=root=$DelegationRootEncoded;session=$DelegationSessionEncoded;context=$DelegationContextEncoded" `
    -cp $HarnessRoot AgentDelegationHarness $DelegationConsumerJar $DelegationLibraryJar $DelegationInstance
if ($LASTEXITCODE -ne 0) { throw "Delegated writer Agent fixture failed" }
$DelegationRecords = Get-Content -LiteralPath $DelegationSession
if ($DelegationRecords.Count -ne 2 -or $DelegationRecords[1] -notmatch "^4`tcreate`tfile`t$DelegationConsumerPackage`t") {
    throw "Delegated file write was not attributed to the logical caller"
}
Write-Output $DelegationSession

# The Agent itself targets Java 8, while its call-site transformer must still
# cover mutating JDK APIs introduced by later runtimes.
$ModernRoot = Join-Path $TestRoot "java11-apis"
$ModernClasses = Join-Path $ModernRoot "classes"
$ModernInstance = Join-Path $ModernRoot "instance"
$ModernJar = Join-Path $ModernRoot "agent-modern-fixture.jar"
$ModernSession = Join-Path $ModernInstance ".orbit/runtime-data/sessions/test.events"
$ModernContext = Join-Path $ModernInstance ".orbit/runtime-data/agent-context.tsv"
New-Item -ItemType Directory -Force -Path $ModernClasses, (Join-Path $ModernInstance "config"), (Split-Path -Parent $ModernSession) | Out-Null
& javac --release 11 -d $ModernClasses (Join-Path $AgentRoot "tests/AgentModernFixture.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile modern Agent fixture" }
& jar cf $ModernJar -C $ModernClasses .
if ($LASTEXITCODE -ne 0) { throw "Failed to package modern Agent fixture" }
$ModernHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $ModernJar).Hash.ToLowerInvariant()
$ModernPackage = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("modern-fixture")).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ModernRootEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($ModernInstance)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ModernSessionEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($ModernSession)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ModernContextEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($ModernContext)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
[System.IO.File]::WriteAllLines($ModernContext, @(
    "4`tcontext`tend"
    "capability`tjava`t8-25`tend"
    "capability`tsource`tfile`tend"
    "source`t$ModernHash`t$ModernHash`tend"
    "package`t$ModernHash`t$ModernPackage`tend"
), [System.Text.UTF8Encoding]::new($false))
& java "-javaagent:$ResolvedAgent=root=$ModernRootEncoded;session=$ModernSessionEncoded;context=$ModernContextEncoded" `
    -cp $HarnessRoot AgentIsolatedHarness $ModernJar $ModernInstance AgentModernFixture
if ($LASTEXITCODE -ne 0) { throw "Modern Java API Agent fixture failed" }
if ((Get-Content -LiteralPath $ModernSession).Count -ne 3) {
    throw "Java 11 write APIs were not both observed"
}
Write-Output $ModernSession

# Forge 1.17 introduced SecureJarHandler union CodeSources. Exercise the
# original unexported Java module so the Agent must use redefineModule before
# resolving the physical primary JAR.
$CompatibilityRoot = Join-Path $TestRoot "forge-union-0.9.54"
$DependencyRoot = Join-Path $WorkspaceRoot "target/orbit-runtime-agent/compatibility"
New-Item -ItemType Directory -Force -Path $CompatibilityRoot, $DependencyRoot | Out-Null
$Dependencies = @(
    @{ Name = "securejarhandler-0.9.54.jar"; Uri = "https://maven.minecraftforge.net/cpw/mods/securejarhandler/0.9.54/securejarhandler-0.9.54.jar"; Sha256 = "823c9ff565c3f29013ab17d20a03e5ba178675f1f0d0a0e2b7b8355bbadb07db" },
    @{ Name = "asm-9.1.jar"; Uri = "https://repo1.maven.org/maven2/org/ow2/asm/asm/9.1/asm-9.1.jar"; Sha256 = "cda4de455fab48ff0bcb7c48b4639447d4de859a7afc30a094a986f0936beba2" },
    @{ Name = "asm-tree-9.1.jar"; Uri = "https://repo1.maven.org/maven2/org/ow2/asm/asm-tree/9.1/asm-tree-9.1.jar"; Sha256 = "fd00afa49e9595d7646205b09cecb4a776a8ff0ba06f2d59b8f7bf9c704b4a73" }
)
foreach ($Dependency in $Dependencies) {
    $Dependency.Path = Join-Path $DependencyRoot $Dependency.Name
    if (-not (Test-Path -LiteralPath $Dependency.Path -PathType Leaf)) {
        Invoke-WebRequest -UseBasicParsing -Uri $Dependency.Uri -OutFile $Dependency.Path
    }
    $Actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Dependency.Path).Hash.ToLowerInvariant()
    if ($Actual -ne $Dependency.Sha256) {
        throw "$($Dependency.Name) SHA-256 mismatch: $Actual"
    }
}
$UnionClasses = Join-Path $CompatibilityRoot "classes"
$UnionInstance = Join-Path $CompatibilityRoot "instance"
$UnionSession = Join-Path $UnionInstance ".orbit/runtime-data/sessions/test.events"
$UnionContext = Join-Path $UnionInstance ".orbit/runtime-data/agent-context.tsv"
New-Item -ItemType Directory -Force -Path $UnionClasses, (Join-Path $UnionInstance "config"), (Split-Path -Parent $UnionSession) | Out-Null
& javac --release 17 -d $UnionClasses (Join-Path $AgentRoot "tests/AgentUnionHarness.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile Forge union harness" }
$UnionRootEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($UnionInstance)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$UnionSessionEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($UnionSession)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$UnionContextEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($UnionContext)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
[System.IO.File]::WriteAllLines($UnionContext, @(
    "4`tcontext`tend"
    "capability`tjava`t8-25`tend"
    "capability`tsource`tunion`tend"
    "source`t$FixtureHash`t$FixtureHash`tend"
    "package`t$FixtureHash`t$FixturePackage`tend"
), [System.Text.UTF8Encoding]::new($false))
$ModulePath = ($Dependencies | ForEach-Object { $_.Path }) -join [System.IO.Path]::PathSeparator
& java "-javaagent:$ResolvedAgent=root=$UnionRootEncoded;session=$UnionSessionEncoded;context=$UnionContextEncoded" `
    --module-path $ModulePath --add-modules cpw.mods.securejarhandler `
    -cp $UnionClasses AgentUnionHarness $FixtureJar $UnionInstance
if ($LASTEXITCODE -ne 0) { throw "Forge union Agent fixture failed" }
if ((Get-Content -LiteralPath $UnionSession).Count -ne 5) {
    throw "Forge union CodeSource did not resolve to the fixture package"
}
Write-Output $UnionSession

# Quilt 0.18.1+ may define classes from a virtual Quilt filesystem. Validate
# the public QuiltCodeSource identity contract instead of path guessing.
$QuiltPath = Join-Path $DependencyRoot "quilt-loader-0.30.1-beta.2.jar"
if (-not (Test-Path -LiteralPath $QuiltPath -PathType Leaf)) {
    Invoke-WebRequest -UseBasicParsing `
        -Uri "https://maven.quiltmc.org/repository/release/org/quiltmc/quilt-loader/0.30.1-beta.2/quilt-loader-0.30.1-beta.2.jar" `
        -OutFile $QuiltPath
}
$QuiltHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $QuiltPath).Hash.ToLowerInvariant()
if ($QuiltHash -ne "9e5801c55cdb881d5b29967096c08e39131a8fab7f88585bd06ec31b1c5144a6") {
    throw "Quilt Loader SHA-256 mismatch: $QuiltHash"
}
$QuiltRoot = Join-Path $TestRoot "quilt-0.30.1"
$QuiltClasses = Join-Path $QuiltRoot "classes"
$QuiltInstance = Join-Path $QuiltRoot "instance"
$QuiltSession = Join-Path $QuiltInstance ".orbit/runtime-data/sessions/test.events"
$QuiltContext = Join-Path $QuiltInstance ".orbit/runtime-data/agent-context.tsv"
New-Item -ItemType Directory -Force -Path $QuiltClasses, (Join-Path $QuiltInstance "config"), (Split-Path -Parent $QuiltSession) | Out-Null
& javac --release 17 -cp $QuiltPath -d $QuiltClasses (Join-Path $AgentRoot "tests/AgentQuiltHarness.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile Quilt identity harness" }
$QuiltRootEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($QuiltInstance)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$QuiltSessionEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($QuiltSession)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$QuiltContextEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($QuiltContext)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
$ModuleEncoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("agent-fixture")).TrimEnd('=').Replace('+', '-').Replace('/', '_')
[System.IO.File]::WriteAllLines($QuiltContext, @(
    "4`tcontext`tend"
    "capability`tjava`t8-25`tend"
    "capability`tmodule`tquilt`tend"
    "module`t$ModuleEncoded`t$FixtureHash`tend"
    "package`t$FixtureHash`t$FixturePackage`tend"
), [System.Text.UTF8Encoding]::new($false))
& java "-javaagent:$ResolvedAgent=root=$QuiltRootEncoded;session=$QuiltSessionEncoded;context=$QuiltContextEncoded" `
    -cp "$QuiltClasses$([System.IO.Path]::PathSeparator)$QuiltPath" `
    AgentQuiltHarness $FixtureJar $QuiltInstance
if ($LASTEXITCODE -ne 0) { throw "Quilt identity Agent fixture failed" }
if ((Get-Content -LiteralPath $QuiltSession).Count -ne 5) {
    throw "Quilt native module identity did not resolve to the fixture package"
}
Write-Output $QuiltSession
