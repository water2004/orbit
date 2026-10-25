[CmdletBinding()]
param(
    [string] $OutputDirectory,
    [switch] $SkipCargoBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw "The Windows MSI must be built on Windows."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $repoRoot "orbit-cli\Cargo.toml"
$wixSource = Join-Path $repoRoot "installer\windows\Package.wxs"
$wixUiSource = Join-Path $repoRoot "installer\windows\OrbitUI.wxs"
$licenseRtf = Join-Path $repoRoot "installer\windows\License.rtf"
$toolManifest = Join-Path $repoRoot ".config\dotnet-tools.json"
$executable = Join-Path $repoRoot "target\release\orbit.exe"
$launcherExecutable = Join-Path $repoRoot "target\release\orbit-launcher.exe"
$guiExecutable = Join-Path $repoRoot "target\release\orbit-gui.exe"
$runtimeAgent = Join-Path $repoRoot "target\release\orbit-runtime-agent.jar"
$cleanupActions = Join-Path $repoRoot "target\release\orbit_installer_actions.dll"
$guiBuildDirectory = Join-Path $repoRoot "target\release\build"
$license = Join-Path $repoRoot "LICENSE"

if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repoRoot "target\wix"
}

$toolConfiguration = Get-Content -Raw -LiteralPath $toolManifest |
    ConvertFrom-Json
$wixVersion = $toolConfiguration.tools.wix.version
if (-not $wixVersion) {
    throw "The .NET tool manifest does not define a WiX version."
}

$cargoMetadata = cargo metadata --format-version 1 --no-deps --manifest-path $manifestPath |
    ConvertFrom-Json
if ($LASTEXITCODE -ne 0) {
    throw "Failed to read the Orbit package version from Cargo metadata."
}

$orbitPackage = $cargoMetadata.packages |
    Where-Object { $_.manifest_path -eq $manifestPath.Replace("\", "/") } |
    Select-Object -First 1
if (-not $orbitPackage) {
    $orbitPackage = $cargoMetadata.packages |
        Where-Object { $_.name -eq "orbit" } |
        Select-Object -First 1
}
if (-not $orbitPackage) {
    throw "Cargo metadata did not contain the orbit CLI package."
}

$version = $orbitPackage.version
if ($version -notmatch "^\d+\.\d+\.\d+$") {
    throw "MSI versions must contain exactly three numeric fields; Cargo version '$version' is unsupported."
}

$versionParts = $version.Split(".") | ForEach-Object { [int] $_ }
if ($versionParts[0] -gt 255 -or
    $versionParts[1] -gt 255 -or
    $versionParts[2] -gt 65535) {
    throw "Cargo version '$version' exceeds Windows Installer version limits (255.255.65535)."
}

Push-Location $repoRoot
try {
    if (-not $SkipCargoBuild) {
        npm --prefix orbit-gui/ui ci
        if ($LASTEXITCODE -ne 0) { throw "Restoring GUI dependencies failed." }
        npm --prefix orbit-gui/ui run build
        if ($LASTEXITCODE -ne 0) { throw "Building the GUI frontend failed." }
        cargo build --release --locked --package orbit --package orbit-launcher --package orbit-gui --package orbit-installer-actions
        if ($LASTEXITCODE -ne 0) {
            throw "The release build failed."
        }
    }

    & (Join-Path $repoRoot "orbit-runtime-agent\build.ps1") -OutputPath "target/release/orbit-runtime-agent.jar"
    if ($LASTEXITCODE -ne 0) {
        throw "The Orbit Runtime Agent build failed."
    }
    & (Join-Path $repoRoot "orbit-runtime-agent\test.ps1") -AgentPath "target/release/orbit-runtime-agent.jar"
    if ($LASTEXITCODE -ne 0) {
        throw "The Orbit Runtime Agent fixture failed."
    }

    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "Release executable not found at '$executable'."
    }
    if (-not (Test-Path -LiteralPath $launcherExecutable -PathType Leaf)) {
        throw "Release executable not found at '$launcherExecutable'."
    }
    if (-not (Test-Path -LiteralPath $guiExecutable -PathType Leaf)) {
        throw "Release executable not found at '$guiExecutable'."
    }
    if (-not (Test-Path -LiteralPath $runtimeAgent -PathType Leaf)) {
        throw "Orbit Runtime Agent not found at '$runtimeAgent'."
    }
    if (-not (Test-Path -LiteralPath $cleanupActions -PathType Leaf)) {
        throw "Installer cleanup actions not found at '$cleanupActions'."
    }
    $guiIcon = Get-ChildItem -LiteralPath $guiBuildDirectory -Directory -Filter "orbit-gui-*" |
        ForEach-Object { Join-Path $_.FullName "out\orbit-gui.ico" } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
        Sort-Object { (Get-Item -LiteralPath $_).LastWriteTimeUtc } -Descending |
        Select-Object -First 1
    if (-not $guiIcon) {
        throw "Generated GUI icon not found below '$guiBuildDirectory'."
    }

    dotnet tool restore
    if ($LASTEXITCODE -ne 0) {
        throw "Restoring the pinned WiX tool failed."
    }

    dotnet tool run wix -- extension add `
        "WixToolset.UI.wixext/$wixVersion" `
        -acceptEula wix7
    if ($LASTEXITCODE -ne 0) {
        throw "Restoring the pinned WiX UI extension failed."
    }

    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $outputPath = Join-Path $OutputDirectory "orbit-$version-x86_64.msi"
    $intermediatePath = Join-Path $repoRoot "target\wix\obj"

    dotnet tool run wix -- build `
        -acceptEula wix7 `
        $wixSource `
        $wixUiSource `
        -arch x64 `
        -d "OrbitVersion=$version" `
        -d "OrbitExecutable=$executable" `
        -d "OrbitLauncherExecutable=$launcherExecutable" `
        -d "OrbitGuiExecutable=$guiExecutable" `
        -d "OrbitRuntimeAgent=$runtimeAgent" `
        -d "OrbitCleanupActions=$cleanupActions" `
        -d "OrbitGuiIcon=$guiIcon" `
        -d "OrbitLicense=$license" `
        -d "OrbitGuiLicense=$(Join-Path $repoRoot 'orbit-gui\ui\LICENSE')" `
        -d "OrbitGuiAttribution=$(Join-Path $repoRoot 'orbit-gui\ui\UPSTREAM.md')" `
        -d "OrbitLicenseRtf=$licenseRtf" `
        -d "ProductDisplayName=Orbit" `
        -d "ProductCommand=orbit" `
        -ext WixToolset.UI.wixext `
        -intermediatefolder $intermediatePath `
        -out $outputPath
    if ($LASTEXITCODE -ne 0) {
        throw "WiX failed to build the MSI."
    }

    # ICE61 warns whenever the upgrade range includes the current three-field
    # MSI version. That inclusion is intentional: each build receives a fresh
    # ProductCode and must replace an earlier build with the same Cargo version.
    dotnet tool run wix -- msi validate `
        -acceptEula wix7 `
        -sice ICE61 `
        $outputPath
    if ($LASTEXITCODE -ne 0) {
        throw "Windows Installer validation failed."
    }

    & (Join-Path $repoRoot "scripts\test-windows-msi.ps1") -Path $outputPath

    Write-Output (Resolve-Path $outputPath)
}
finally {
    Pop-Location
}
