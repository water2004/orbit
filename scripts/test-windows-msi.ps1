[CmdletBinding()]
param([Parameter(Mandatory = $true)][string] $Path)

# Read-only MSI database contract tests. Never install, uninstall or launch UI.
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$packagePath = (Resolve-Path -LiteralPath $Path).Path
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.OpenDatabase($packagePath, 0)
function Get-MsiRows([string] $Query, [int] $Columns) {
    $view = $database.OpenView($Query)
    try {
        [void]$view.Execute()
        while ($record = $view.Fetch()) {
            $row = @()
            for ($index = 1; $index -le $Columns; $index++) { $row += $record.StringData($index) }
            ,$row
        }
    } finally { [void]$view.Close() }
}
function Assert-Contract([bool] $Condition, [string] $Description) {
    if (-not $Condition) { throw "MSI contract failed: $Description" }
}
$properties = @{}
foreach ($row in (Get-MsiRows 'SELECT `Property`, `Value` FROM `Property`' 2)) { $properties[$row[0]] = $row[1] }
Assert-Contract ($properties.UNINSTALL_DATA_LEVEL -eq 'cache') 'default cleanup must preserve settings and instances'
Assert-Contract (-not $properties.ContainsKey('REMOVE_APPDATA')) 'obsolete blanket AppData deletion must not remain'
$levels = @(Get-MsiRows 'SELECT `Value` FROM `RadioButton` WHERE `Property` = ''UNINSTALL_DATA_LEVEL''' 1 | ForEach-Object { $_[0] } | Sort-Object)
Assert-Contract (($levels -join ',') -eq 'all,cache,settings') 'uninstall must expose exactly three levels'
$actions = @{}
foreach ($row in (Get-MsiRows 'SELECT `Action`, `Type`, `Source`, `Target` FROM `CustomAction`' 4)) { $actions[$row[0]] = $row }
foreach ($name in @('OrbitRollbackCleanup', 'OrbitStageCleanup', 'OrbitCommitCleanup')) {
    Assert-Contract ($actions.ContainsKey($name)) "missing $name"
    $actionType = [int]$actions[$name][1]
    Assert-Contract (($actionType -band 0x400) -ne 0) "$name must run in the MSI transaction"
    Assert-Contract (($actionType -band 0x800) -eq 0) "$name must not clean as LocalSystem"
    Assert-Contract (($actionType -band 0x2000) -ne 0) "$name must hide its private action data"
}
Assert-Contract (([int]$actions.OrbitRollbackCleanup[1] -band 0x100) -ne 0) 'rollback action type'
Assert-Contract (([int]$actions.OrbitCommitCleanup[1] -band 0x200) -ne 0) 'commit action type'
Assert-Contract ($actions.ConfigureOrbitGuiBindings[2] -eq 'OrbitGuiExecutable') 'GUI owns its preference writer'
Assert-Contract ($actions.ConfigureOrbitGuiBindings[3] -eq '--install-cli-bindings') 'installer must not patch preference JSON'
$sequence = @{}
foreach ($row in (Get-MsiRows 'SELECT `Action`, `Condition`, `Sequence` FROM `InstallExecuteSequence`' 3)) { $sequence[$row[0]] = $row }
Assert-Contract ([int]$sequence.InstallValidate[2] -lt [int]$sequence.OrbitPrepareCleanup[2]) 'prepare after REMOVE=ALL is established'
Assert-Contract ([int]$sequence.OrbitPrepareCleanup[2] -lt [int]$sequence.InstallInitialize[2]) 'confirmation before transactional writes'
Assert-Contract ([int]$sequence.InstallInitialize[2] -lt [int]$sequence.OrbitRollbackCleanup[2]) 'rollback registration after transaction start'
Assert-Contract ([int]$sequence.OrbitRollbackCleanup[2] -lt [int]$sequence.OrbitStageCleanup[2]) 'rollback registered before staging'
Assert-Contract ([int]$sequence.OrbitStageCleanup[2] -lt [int]$sequence.RemoveFiles[2]) 'user data staged before program removal'
Assert-Contract ($sequence.OrbitPrepareCleanup[1].Contains('NOT UPGRADINGPRODUCTCODE')) 'upgrades cannot prepare cleanup'
Assert-Contract ($sequence.ConfigureOrbitGuiBindings[1].Contains('GuiFeature')) 'CLI-only installs cannot alter GUI preferences'
Write-Output "MSI contract verified: $packagePath"
