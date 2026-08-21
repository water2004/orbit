param(
    [string]$AgentJar,
    [ValidateRange(1, 1000000)]
    [int]$Iterations = 10000,
    [ValidateRange(1, 20)]
    [int]$Trials = 5,
    [string]$JavaCommand = "java"
)

$ErrorActionPreference = "Stop"
$AgentRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$WorkspaceRoot = Split-Path -Parent $AgentRoot
$ProfileRoot = Join-Path $WorkspaceRoot "target/orbit-runtime-agent-profile"
$ResolvedProfileRoot = [System.IO.Path]::GetFullPath($ProfileRoot)
$ResolvedWorkspaceRoot = [System.IO.Path]::GetFullPath($WorkspaceRoot)
if (-not $ResolvedProfileRoot.StartsWith($ResolvedWorkspaceRoot + [System.IO.Path]::DirectorySeparatorChar)) {
    throw "Profile output escaped the workspace: $ResolvedProfileRoot"
}

if ([string]::IsNullOrWhiteSpace($AgentJar)) {
    $AgentJar = Join-Path $WorkspaceRoot "target/debug/orbit-runtime-agent.jar"
}
if (-not (Test-Path -LiteralPath $AgentJar -PathType Leaf)) {
    & (Join-Path $AgentRoot "build.ps1") -OutputPath $AgentJar
    if ($LASTEXITCODE -ne 0) { throw "Failed to build Orbit Runtime Agent" }
}
$AgentJar = (Resolve-Path -LiteralPath $AgentJar).Path

if (Test-Path -LiteralPath $ProfileRoot) {
    Remove-Item -LiteralPath $ProfileRoot -Recurse -Force
}
$LibraryClasses = Join-Path $ProfileRoot "library-classes"
$ConsumerClasses = Join-Path $ProfileRoot "consumer-classes"
$LibraryJar = Join-Path $ProfileRoot "library.jar"
$ConsumerJar = Join-Path $ProfileRoot "consumer.jar"
New-Item -ItemType Directory -Force -Path $LibraryClasses, $ConsumerClasses | Out-Null

& javac --release 8 -d $LibraryClasses (Join-Path $AgentRoot "tests/AgentDelegateLibrary.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile profile helper" }
& jar cf $LibraryJar -C $LibraryClasses .
if ($LASTEXITCODE -ne 0) { throw "Failed to package profile helper" }
& javac --release 8 -cp $LibraryJar -d $ConsumerClasses `
    (Join-Path $AgentRoot "tests/AgentDelegateConsumer.java")
if ($LASTEXITCODE -ne 0) { throw "Failed to compile profile consumer" }
& jar cf $ConsumerJar -C $ConsumerClasses .
if ($LASTEXITCODE -ne 0) { throw "Failed to package profile consumer" }

$LibraryHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $LibraryJar).Hash.ToLowerInvariant()
$ConsumerHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $ConsumerJar).Hash.ToLowerInvariant()
$ClassPath = "$ConsumerJar$([System.IO.Path]::PathSeparator)$LibraryJar"

function ConvertTo-UrlBase64([string]$Value) {
    return [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Value)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
}

function Invoke-ProfileProcess([string[]]$Arguments) {
    $StartInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $StartInfo.FileName = $JavaCommand
    $StartInfo.UseShellExecute = $false
    $StartInfo.RedirectStandardOutput = $true
    $StartInfo.RedirectStandardError = $true
    $QuotedArguments = foreach ($Argument in $Arguments) {
        if ($Argument.Contains('"')) {
            throw "Java profile argument contains an unsupported quote"
        }
        if ($Argument -match '[\s]') {
            '"' + $Argument + '"'
        } else {
            $Argument
        }
    }
    $StartInfo.Arguments = $QuotedArguments -join ' '

    $Process = [System.Diagnostics.Process]::new()
    $Process.StartInfo = $StartInfo
    if (-not $Process.Start()) { throw "Failed to start Java profile process" }
    $StandardOutput = $Process.StandardOutput.ReadToEndAsync()
    $StandardError = $Process.StandardError.ReadToEndAsync()
    [long]$PeakWorkingSet = 0
    while (-not $Process.WaitForExit(10)) {
        $Process.Refresh()
        $PeakWorkingSet = [Math]::Max($PeakWorkingSet, $Process.WorkingSet64)
    }
    $Process.Refresh()
    $PeakWorkingSet = [Math]::Max($PeakWorkingSet, $Process.PeakWorkingSet64)
    $Output = $StandardOutput.GetAwaiter().GetResult().Trim()
    $ErrorOutput = $StandardError.GetAwaiter().GetResult().Trim()
    if ($Process.ExitCode -ne 0) {
        throw "Java profile process failed ($($Process.ExitCode)): $ErrorOutput"
    }
    $Measurement = $Output | ConvertFrom-Json
    [pscustomobject]@{
        elapsed_nanos = [long]$Measurement.elapsed_nanos
        nanos_per_operation = [double]$Measurement.elapsed_nanos / [double]$Measurement.iterations
        used_heap_bytes = [long]$Measurement.used_heap_bytes
        peak_working_set_bytes = $PeakWorkingSet
    }
}

function Invoke-ModeTrial([string]$Mode, [bool]$UseAgent, [bool]$Delegated, [int]$Trial) {
    $Instance = Join-Path $ProfileRoot "$Mode-$Trial"
    $Session = Join-Path $Instance ".orbit/runtime-data/sessions/profile.events"
    $Context = Join-Path $Instance ".orbit/runtime-data/agent-context.tsv"
    New-Item -ItemType Directory -Force -Path (Join-Path $Instance "config"), (Split-Path -Parent $Session) | Out-Null

    $Arguments = @()
    if ($UseAgent) {
        $ContextLines = @(
            "4`tcontext`tend",
            "capability`tjava`t8-25`tend",
            "capability`tsource`tfile`tend",
            "source`t$LibraryHash`t$LibraryHash`tend",
            "source`t$ConsumerHash`t$ConsumerHash`tend",
            "package`t$LibraryHash`t$(ConvertTo-UrlBase64 'profile-library')`tend",
            "package`t$ConsumerHash`t$(ConvertTo-UrlBase64 'profile-consumer')`tend"
        )
        if ($Delegated) {
            $ContextLines += "delegation`t$ConsumerHash`t$LibraryHash`tend"
        }
        [System.IO.File]::WriteAllLines($Context, $ContextLines, [System.Text.UTF8Encoding]::new($false))
        $Arguments += "-javaagent:$AgentJar=root=$(ConvertTo-UrlBase64 $Instance);session=$(ConvertTo-UrlBase64 $Session);context=$(ConvertTo-UrlBase64 $Context)"
    }
    $Arguments += @("-cp", $ClassPath, "AgentDelegateConsumer", $Instance, "$Iterations")
    Invoke-ProfileProcess $Arguments
}

$Modes = @(
    [pscustomobject]@{ mode = "baseline"; agent = $false; delegated = $false },
    [pscustomobject]@{ mode = "agent-direct-owner"; agent = $true; delegated = $false },
    [pscustomobject]@{ mode = "agent-delegated-owner"; agent = $true; delegated = $true }
)
$Measurements = @{}
foreach ($Mode in $Modes) { $Measurements[$Mode.mode] = @() }
for ($Trial = 1; $Trial -le $Trials; $Trial++) {
    $Offset = ($Trial - 1) % $Modes.Count
    for ($Index = 0; $Index -lt $Modes.Count; $Index++) {
        $Mode = $Modes[($Index + $Offset) % $Modes.Count]
        $Measurements[$Mode.mode] += Invoke-ModeTrial $Mode.mode $Mode.agent $Mode.delegated $Trial
    }
}

$Results = foreach ($Mode in $Modes) {
    $Sorted = @($Measurements[$Mode.mode] | Sort-Object nanos_per_operation)
    $Median = $Sorted[[Math]::Floor($Sorted.Count / 2)]
    [pscustomobject]@{
        mode = $Mode.mode
        trials = $Trials
        iterations_per_trial = $Iterations
        median_nanos_per_operation = [Math]::Round($Median.nanos_per_operation, 1)
        median_used_heap_mib = [Math]::Round($Median.used_heap_bytes / 1MB, 2)
        median_peak_working_set_mib = [Math]::Round($Median.peak_working_set_bytes / 1MB, 2)
    }
}
$Results = @($Results)
$Baseline = $Results[0].median_nanos_per_operation
foreach ($Result in $Results) {
    $Result | Add-Member -NotePropertyName overhead_percent -NotePropertyValue (
        [Math]::Round((($Result.median_nanos_per_operation / $Baseline) - 1.0) * 100.0, 2)
    )
}
$Results | ConvertTo-Json
