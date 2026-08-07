$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$nativeDesktop = "Microsoft.VisualStudio.Workload.NativeDesktop"
$atlComponent = "Microsoft.VisualStudio.Component.VC.ATL"
$vs2022VersionRange = "[17.0,18.0)"
$installerRoot = Join-Path ${env:ProgramFiles(x86)} `
    "Microsoft Visual Studio\Installer"
$vswhere = Join-Path $installerRoot "vswhere.exe"
$setup = Join-Path $installerRoot "setup.exe"

function Invoke-NativeCommand {
    param(
        [string] $FilePath,
        [string[]] $Arguments,
        [int[]] $SuccessCodes = @(0)
    )

    & $FilePath @Arguments
    $exitCode = $LASTEXITCODE
    if ($SuccessCodes -notcontains $exitCode) {
        throw "$FilePath failed with exit code $exitCode"
    }
}

function Get-VisualStudioPath {
    if (-not (Test-Path -LiteralPath $vswhere)) {
        return $null
    }

    $path = & $vswhere `
        -latest `
        -products * `
        -version $vs2022VersionRange `
        -property installationPath
    if ($LASTEXITCODE -ne 0) {
        throw "vswhere failed with exit code $LASTEXITCODE"
    }
    return @($path)[0]
}

function Get-ScoopRoot {
    if (-not [string]::IsNullOrWhiteSpace($env:SCOOP)) {
        return $env:SCOOP
    }
    return Join-Path $HOME "scoop"
}

function Add-ScoopToCurrentPath {
    param([string] $ScoopRoot)

    $shims = Join-Path $ScoopRoot "shims"
    if (-not (Test-Path -LiteralPath $shims)) {
        throw "Scoop shims directory was not found: $shims"
    }

    $pathEntries = @($env:Path -split ";" | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_)
    })
    if ($pathEntries -notcontains $shims) {
        $env:Path = "$shims;$env:Path"
    }
}

Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser

if ($null -eq (Get-Command scoop -ErrorAction SilentlyContinue)) {
    Invoke-RestMethod -Uri https://get.scoop.sh | Invoke-Expression
}

$scoopRoot = Get-ScoopRoot
Add-ScoopToCurrentPath -ScoopRoot $scoopRoot
if ($null -eq (Get-Command scoop -ErrorAction SilentlyContinue)) {
    throw "Scoop command was not found after installation"
}

$visualStudioPath = Get-VisualStudioPath
if ([string]::IsNullOrWhiteSpace($visualStudioPath)) {
    $override = @(
        "--wait",
        "--quiet",
        "--norestart",
        "--add $nativeDesktop",
        "--add $atlComponent",
        "--includeRecommended"
    ) -join " "
    Invoke-NativeCommand -FilePath "winget" -Arguments @(
        "install",
        "--id",
        "Microsoft.VisualStudio.2022.Community",
        "--exact",
        "--silent",
        "--accept-package-agreements",
        "--accept-source-agreements",
        "--override",
        $override
    )
    $visualStudioPath = Get-VisualStudioPath
}

if ([string]::IsNullOrWhiteSpace($visualStudioPath)) {
    throw "Visual Studio 2022 installation was not found"
}
if (-not (Test-Path -LiteralPath $setup)) {
    throw "Visual Studio installer was not found: $setup"
}

Invoke-NativeCommand -FilePath $setup -SuccessCodes @(0, 3010) -Arguments @(
    "modify",
    "--installPath",
    $visualStudioPath,
    "--quiet",
    "--wait",
    "--norestart",
    "--add",
    $nativeDesktop,
    "--add",
    $atlComponent,
    "--includeRecommended"
)

$extrasBucket = Join-Path $scoopRoot "buckets\extras"
if (-not (Test-Path -LiteralPath $extrasBucket)) {
    Invoke-NativeCommand -FilePath "scoop" -Arguments @(
        "bucket",
        "add",
        "extras"
    )
}

$cmakeCurrent = Join-Path $scoopRoot "apps\cmake\current"
if (-not (Test-Path -LiteralPath $cmakeCurrent)) {
    Invoke-NativeCommand -FilePath "scoop" -Arguments @("install", "cmake")
}
