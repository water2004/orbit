param(
    [string]$OutputPath = "target/release/orbit-runtime-agent.jar"
)

$ErrorActionPreference = "Stop"
$AgentRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$WorkspaceRoot = Split-Path -Parent $AgentRoot
$BuildRoot = Join-Path $WorkspaceRoot "target/orbit-runtime-agent"
$DependencyPath = Join-Path $BuildRoot "byte-buddy-1.18.7.jar"
$ClassesPath = Join-Path $BuildRoot "classes"
$ManifestPath = Join-Path $BuildRoot "MANIFEST.MF"
$ExpectedSha256 = "7c3b8fd43d75b2dc30bdb8cf7303d4b15f6f9a0ccb170f8b9f47de15864014f3"

New-Item -ItemType Directory -Force -Path $BuildRoot | Out-Null
if (-not (Test-Path -LiteralPath $DependencyPath -PathType Leaf)) {
    Invoke-WebRequest -UseBasicParsing `
        -Uri "https://repo1.maven.org/maven2/net/bytebuddy/byte-buddy/1.18.7/byte-buddy-1.18.7.jar" `
        -OutFile $DependencyPath
}
$ActualSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $DependencyPath).Hash.ToLowerInvariant()
if ($ActualSha256 -ne $ExpectedSha256) {
    throw "Byte Buddy SHA-256 mismatch: $ActualSha256"
}

if (Test-Path -LiteralPath $ClassesPath) {
    $ResolvedBuild = [System.IO.Path]::GetFullPath($BuildRoot)
    $ResolvedClasses = [System.IO.Path]::GetFullPath($ClassesPath)
    if (-not $ResolvedClasses.StartsWith($ResolvedBuild, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to clean classes outside the Agent build directory"
    }
    Remove-Item -LiteralPath $ClassesPath -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $ClassesPath | Out-Null

Push-Location $ClassesPath
try {
    & jar xf $DependencyPath
    if ($LASTEXITCODE -ne 0) { throw "Failed to unpack Byte Buddy" }
} finally {
    Pop-Location
}
Get-ChildItem -LiteralPath (Join-Path $ClassesPath "META-INF") -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Extension -in ".SF", ".RSA", ".DSA" } |
    Remove-Item -Force

$Sources = Get-ChildItem -LiteralPath (Join-Path $AgentRoot "src/main/java") -Recurse -Filter "*.java" |
    ForEach-Object { $_.FullName }
& javac --release 17 -cp $DependencyPath -d $ClassesPath @Sources
if ($LASTEXITCODE -ne 0) { throw "Failed to compile Orbit Runtime Agent" }

@"
Manifest-Version: 1.0
Premain-Class: dev.orbit.agent.OrbitRuntimeAgent
Can-Redefine-Classes: false
Can-Retransform-Classes: false

"@ | Set-Content -LiteralPath $ManifestPath -Encoding ascii -NoNewline

$AbsoluteOutput = [System.IO.Path]::GetFullPath((Join-Path $WorkspaceRoot $OutputPath))
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $AbsoluteOutput) | Out-Null
& jar cfm $AbsoluteOutput $ManifestPath -C $ClassesPath .
if ($LASTEXITCODE -ne 0) { throw "Failed to package Orbit Runtime Agent" }
Write-Output $AbsoluteOutput
