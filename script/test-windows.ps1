[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$nativeDesktop = "Microsoft.VisualStudio.Workload.NativeDesktop"
$atlComponent = "Microsoft.VisualStudio.Component.VC.ATL"
$vswhere = Join-Path ${env:ProgramFiles(x86)} `
    "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere)) {
    throw "vswhere.exe not found: $vswhere"
}

$installationPath = & $vswhere `
    -latest `
    -products * `
    -version "[17.0,18.0)" `
    -requires $nativeDesktop $atlComponent `
    -property installationPath
$exitCode = $LASTEXITCODE
if ($exitCode -ne 0) {
    throw "vswhere failed with exit code $exitCode"
}
if ([string]::IsNullOrWhiteSpace($installationPath)) {
    throw "Visual Studio 2022 with NativeDesktop and VC.ATL was not found"
}

$vcvarsall = Join-Path $installationPath "VC\Auxiliary\Build\vcvarsall.bat"
if (-not (Test-Path -LiteralPath $vcvarsall)) {
    throw "vcvarsall.bat not found: $vcvarsall"
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$commands = @(
    "@echo off",
    "chcp 65001 >nul",
    "call `"$vcvarsall`" x64",
    "if errorlevel 1 exit /b %errorlevel%",
    "cd /d `"$repoRoot`"",
    "cargo test --all",
    "exit /b %errorlevel%"
)
$tempScript = Join-Path ([IO.Path]::GetTempPath()) `
    ("navop-windows-tests-{0}.cmd" -f [guid]::NewGuid())

try {
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllLines($tempScript, $commands, $encoding)
    & $env:ComSpec /d /s /c "`"$tempScript`""
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        throw "Windows workspace tests failed with exit code $exitCode"
    }
}
finally {
    Remove-Item `
        -LiteralPath $tempScript `
        -Force `
        -ErrorAction SilentlyContinue
}
