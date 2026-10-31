param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("navop-shell", "navop-data")]
    [string]$Product,

    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$SourceDir,

    [string]$OutDir = "dist",

    [ValidateSet("x64", "x86")]
    [string]$Arch = "x64",

    [string]$IconPath = "resources/windows/navop.ico"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$installerDir = $PSScriptRoot

$products = @{
    "navop-shell" = @{
        ProductName        = "Navop Shell"
        ExeName            = "navop-shell.exe"
        UpgradeCode        = "6B438332-1757-4910-965B-F85C7B6B0F37"
        InstallDirName     = "Navop Shell"
        RegistryKey        = "NavopShell"
        BundleId           = "NavopShell"
        ProductDescription = "Navop Shell - SSH, SFTP, Terminal, Port Forwarding"
    }
    "navop-data" = @{
        ProductName        = "Navop Data"
        ExeName            = "navop-data.exe"
        UpgradeCode        = "E3605D1B-559A-4C46-B03A-A4770236A48A"
        InstallDirName     = "Navop Data"
        RegistryKey        = "NavopData"
        BundleId           = "NavopData"
        ProductDescription = "Navop Data - Database, Redis, MongoDB, SQL"
    }
}

$config = $products[$Product]
$resolvedSourceDir = (Resolve-Path $SourceDir).Path
$resolvedIconPath = (Resolve-Path (Join-Path $repoRoot $IconPath)).Path
$resolvedOutDir = Join-Path $repoRoot $OutDir
New-Item -ItemType Directory -Force -Path $resolvedOutDir | Out-Null

$msiVersion = ($Version -split "-")[0]
if ($msiVersion -notmatch "^\d+\.\d+\.\d+$") {
    $msiVersion = "0.0.0"
}

$msiPath = Join-Path $resolvedOutDir "$Product-$Version-$Arch.msi"
$exePath = Join-Path $resolvedOutDir "$Product-$Version-$Arch.exe"

$commonDefines = @(
    "-d", "ProductName=$($config.ProductName)",
    "-d", "ProductDescription=$($config.ProductDescription)",
    "-d", "Version=$msiVersion",
    "-d", "SourceDir=$resolvedSourceDir",
    "-d", "ExeName=$($config.ExeName)",
    "-d", "UpgradeCode=$($config.UpgradeCode)",
    "-d", "InstallDirName=$($config.InstallDirName)",
    "-d", "RegistryKey=$($config.RegistryKey)",
    "-d", "IconPath=$resolvedIconPath"
)

& wix build (Join-Path $installerDir "navop-product.wxs") `
    -ext WixToolset.UI.wixext `
    -culture en-US `
    -loc (Join-Path $installerDir "navop.wxl") `
    -arch $Arch `
    @commonDefines `
    -out $msiPath
if ($LASTEXITCODE -ne 0) {
    throw "wix build failed for $Product MSI"
}

& wix build (Join-Path $installerDir "navop-product-bundle.wxs") `
    -ext WixToolset.BootstrapperApplications.wixext `
    -arch $Arch `
    -d "ProductName=$($config.ProductName)" `
    -d "Version=$msiVersion" `
    -d "BundleId=$($config.BundleId)" `
    -d "IconPath=$resolvedIconPath" `
    -d "MsiPath=$msiPath" `
    -out $exePath
if ($LASTEXITCODE -ne 0) {
    throw "wix build failed for $Product bundle"
}

Write-Host "Built $msiPath"
Write-Host "Built $exePath"
