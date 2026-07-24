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

# Keep one ProductCode per released x64 version. This makes rebuilding the same
# version enter maintenance mode instead of registering a duplicate product.
$productCodeTail = "{0:X2}{1:X2}{2:X4}A64C" -f `
    $versionParts[0], $versionParts[1], $versionParts[2]
$productCode = "F56A3B38-5646-4B0E-A73F-$productCodeTail"

Push-Location $repoRoot
try {
    if (-not $SkipCargoBuild) {
        cargo build --release --locked --package orbit
        if ($LASTEXITCODE -ne 0) {
            throw "The release build failed."
        }
    }

    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "Release executable not found at '$executable'."
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
        -d "OrbitProductCode=$productCode" `
        -d "OrbitExecutable=$executable" `
        -d "OrbitLicense=$license" `
        -d "OrbitLicenseRtf=$licenseRtf" `
        -ext WixToolset.UI.wixext `
        -intermediatefolder $intermediatePath `
        -out $outputPath
    if ($LASTEXITCODE -ne 0) {
        throw "WiX failed to build the MSI."
    }

    dotnet tool run wix -- msi validate `
        -acceptEula wix7 `
        $outputPath
    if ($LASTEXITCODE -ne 0) {
        throw "Windows Installer validation failed."
    }

    Write-Output (Resolve-Path $outputPath)
}
finally {
    Pop-Location
}
