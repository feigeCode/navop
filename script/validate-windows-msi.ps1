param(
  [Parameter(Mandatory = $true)]
  [string]$Path,

  [Parameter(Mandatory = $true)]
  [int]$ExpectedLanguage
)

$ErrorActionPreference = "Stop"

function Read-MsiValue {
  param([object]$Database, [string]$Query)

  $view = $Database.OpenView($Query)
  try {
    $null = $view.Execute()
    $record = $view.Fetch()
    if ($null -eq $record) {
      throw "MSI query returned no rows: $Query"
    }
    $value = [string]$record.StringData(1)
    return $value.Trim()
  } finally {
    $null = $view.Close()
  }
}

function Assert-MsiValue {
  param([object]$Database, [string]$Expected, [string]$Query)

  $actual = Read-MsiValue -Database $Database -Query $Query
  if ($actual -ne $Expected) {
    throw "MSI value mismatch. Expected '$Expected', got '$actual'. Query: $Query"
  }
  Write-Host "Verified: $Query => $actual"
}

$resolvedPath = (Resolve-Path $Path).Path
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.OpenDatabase($resolvedPath, 0)

Assert-MsiValue $database "$ExpectedLanguage" `
  "SELECT Value FROM Property WHERE Property = 'ProductLanguage'"
Assert-MsiValue $database "INSTALLROOT" `
  "SELECT Value FROM Property WHERE Property = 'WIXUI_INSTALLDIR'"
Assert-MsiValue $database "Programs" `
  "SELECT DefaultDir FROM Directory WHERE Directory = 'INSTALLROOT'"
Assert-MsiValue $database "INSTALLROOT" `
  "SELECT Directory_Parent FROM Directory WHERE Directory = 'INSTALLFOLDER'"
Assert-MsiValue $database "Navop" `
  "SELECT DefaultDir FROM Directory WHERE Directory = 'INSTALLFOLDER'"
Assert-MsiValue $database "DesktopFolder" `
  "SELECT Directory_ FROM Shortcut WHERE Shortcut = 'DesktopShortcut'"
Assert-MsiValue $database "ApplicationProgramsFolder" `
  "SELECT Directory_ FROM Shortcut WHERE Shortcut = 'StartMenuShortcut'"
Assert-MsiValue $database "DesktopShortcutComponent" `
  "SELECT Component_ FROM Shortcut WHERE Shortcut = 'DesktopShortcut'"
Assert-MsiValue $database "StartMenuShortcutComponent" `
  "SELECT Component_ FROM Shortcut WHERE Shortcut = 'StartMenuShortcut'"
Assert-MsiValue $database "[#NavopExecutable]" `
  "SELECT Target FROM Shortcut WHERE Shortcut = 'DesktopShortcut'"
Assert-MsiValue $database "[#NavopExecutable]" `
  "SELECT Target FROM Shortcut WHERE Shortcut = 'StartMenuShortcut'"
Assert-MsiValue $database "DesktopShortcutRegistry" `
  "SELECT KeyPath FROM Component WHERE Component = 'DesktopShortcutComponent'"
Assert-MsiValue $database "StartMenuShortcutRegistry" `
  "SELECT KeyPath FROM Component WHERE Component = 'StartMenuShortcutComponent'"
Assert-MsiValue $database "1" `
  "SELECT Root FROM Registry WHERE Registry = 'DesktopShortcutRegistry'"
Assert-MsiValue $database "1" `
  "SELECT Root FROM Registry WHERE Registry = 'StartMenuShortcutRegistry'"
Assert-MsiValue $database "InstallDirDlg" `
  "SELECT Dialog FROM Dialog WHERE Dialog = 'InstallDirDlg'"

Write-Host "Validated MSI: $resolvedPath"
