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

# 快捷方式不得引用 Icon 表：那份图标会被 MSI 另存成独立文件，丢失后快捷方式图标
# 退化成通用白纸（issue #325）。空列在自动化的 StringData 上取值为 "" 或抛异常，两者都算通过。
function Assert-MsiEmptyValue {
  param([object]$Database, [string]$Query)

  $view = $Database.OpenView($Query)
  try {
    $null = $view.Execute()
    $record = $view.Fetch()
    if ($null -eq $record) {
      throw "MSI query returned no rows: $Query"
    }
    $value = ""
    try {
      $value = [string]$record.StringData(1)
    } catch {
      $value = ""
    }
    if (-not [string]::IsNullOrWhiteSpace($value)) {
      throw "MSI value should be empty, got '$value'. Query: $Query"
    }
    Write-Host "Verified empty: $Query"
  } finally {
    $null = $view.Close()
  }
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
Assert-MsiEmptyValue $database `
  "SELECT Icon_ FROM Shortcut WHERE Shortcut = 'DesktopShortcut'"
Assert-MsiEmptyValue $database `
  "SELECT Icon_ FROM Shortcut WHERE Shortcut = 'StartMenuShortcut'"
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
