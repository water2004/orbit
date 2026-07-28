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
        cargo build --release --locked --package orbit --package orbit-launcher --package orbit-gui
        if ($LASTEXITCODE -ne 0) {
            throw "The release build failed."
        }
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

    dotnet tool run wix -- extension add `
        "WixToolset.Util.wixext/$wixVersion" `
        -acceptEula wix7
    if ($LASTEXITCODE -ne 0) {
        throw "Restoring the pinned WiX Util extension failed."
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
        -d "OrbitLicense=$license" `
        -d "OrbitLicenseRtf=$licenseRtf" `
        -d "ProductDisplayName=Orbit" `
        -d "ProductCommand=orbit" `
        -d "ProductDataDescription=Deletes Orbit and Orbit Launcher configuration, account metadata, encrypted local credentials, instance registries, managed Java runtimes, and caches from the AppData paths recorded during installation. Minecraft instances and custom paths are never removed." `
        -ext WixToolset.UI.wixext `
        -ext WixToolset.Util.wixext `
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

    Write-Output (Resolve-Path $outputPath)
}
finally {
    Pop-Location
}
