param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$BinDir = "",
    [string]$OutDir = "",
    [switch]$SkipInstaller,
    [string]$IsccPath = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$cleanVersion = $Version.Trim() -replace '^v', ''
if ($cleanVersion -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$') {
    throw "Invalid version '$Version'. Expected semver such as 0.1.0 or v0.1.0."
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($BinDir)) {
    if (-not [string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
        $BinDir = Join-Path $env:CARGO_TARGET_DIR "release"
    } else {
        $BinDir = Join-Path $repoRoot "target\release"
    }
}
$BinDir = (Resolve-Path -LiteralPath $BinDir).Path

if ([string]::IsNullOrWhiteSpace($OutDir)) {
    $OutDir = Join-Path $repoRoot "dist"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path

$stageRoot = Join-Path $OutDir "stage\prime-agent-app"
if (Test-Path -LiteralPath $stageRoot) {
    Remove-Item -LiteralPath $stageRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $stageRoot | Out-Null

$excludedDirs = @("__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", ".venv", "test", ".git")

function Copy-CleanTree {
    param(
        [string]$Source,
        [string]$Destination,
        [string[]]$ExcludedDirs,
        [string[]]$ExcludedExts = @(".pyc", ".pyo")
    )
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    foreach ($item in Get-ChildItem -LiteralPath $Source -Force) {
        if ($item.PSIsContainer) {
            if ($ExcludedDirs -contains $item.Name) {
                continue
            }
            Copy-CleanTree -Source $item.FullName -Destination (Join-Path $Destination $item.Name) -ExcludedDirs $ExcludedDirs -ExcludedExts $ExcludedExts
        } else {
            if ($ExcludedExts -contains $item.Extension.ToLowerInvariant()) {
                continue
            }
            if ($item.Name -eq ".gitignore") {
                continue
            }
            Copy-Item -LiteralPath $item.FullName -Destination (Join-Path $Destination $item.Name) -Force
        }
    }
}

foreach ($bin in @("prime-agent-app.exe", "pa-daemon.exe")) {
    $srcBin = Join-Path $BinDir $bin
    if (-not (Test-Path -LiteralPath $srcBin -PathType Leaf)) {
        throw "Missing required binary: $srcBin"
    }
    Copy-Item -LiteralPath $srcBin -Destination (Join-Path $stageRoot $bin) -Force
}

Copy-CleanTree `
    -Source (Join-Path $repoRoot "prime-agent-runtime") `
    -Destination (Join-Path $stageRoot "prime-agent-runtime") `
    -ExcludedDirs $excludedDirs

Copy-CleanTree `
    -Source (Join-Path $repoRoot "skills") `
    -Destination (Join-Path $stageRoot "skills") `
    -ExcludedDirs $excludedDirs

$requiredFiles = @(
    "prime-agent-app.exe",
    "pa-daemon.exe",
    "prime-agent-runtime\pyproject.toml",
    "prime-agent-runtime\src\rlm\__init__.py",
    "prime-agent-runtime\src\rlm\repl.py",
    "skills\edit\SKILL.md"
)
foreach ($rel in $requiredFiles) {
    $full = Join-Path $stageRoot $rel
    if (-not (Test-Path -LiteralPath $full -PathType Leaf)) {
        throw "Staged bundle is missing required file: $rel"
    }
}

$forbiddenDirs = @(
    Get-ChildItem -LiteralPath $stageRoot -Recurse -Directory -Force |
        Where-Object { $excludedDirs -contains $_.Name }
)
if ($forbiddenDirs.Count -gt 0) {
    throw "Staged bundle contains forbidden cache directory: $($forbiddenDirs[0].FullName)"
}

$artifacts = New-Object System.Collections.Generic.List[string]

$zipName = "prime-agent-app-$cleanVersion-windows-x64.zip"
$zipPath = Join-Path $OutDir $zipName
if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
}
Get-ChildItem -LiteralPath $stageRoot -Force |
    Compress-Archive -DestinationPath $zipPath -CompressionLevel Optimal
$artifacts.Add($zipPath)

if (-not $SkipInstaller) {
    if ([string]::IsNullOrWhiteSpace($IsccPath)) {
        $cmd = Get-Command iscc -ErrorAction SilentlyContinue
        if ($cmd) {
            $IsccPath = $cmd.Source
        } else {
            $candidates = @(
                "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
                "C:\Program Files\Inno Setup 6\ISCC.exe"
            )
            $IsccPath = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
        }
    }
    if ([string]::IsNullOrWhiteSpace($IsccPath) -or -not (Test-Path -LiteralPath $IsccPath -PathType Leaf)) {
        throw "Inno Setup compiler (ISCC.exe) not found. Install Inno Setup 6 or pass -SkipInstaller."
    }

    $issPath = Join-Path $repoRoot "packaging\windows\prime-agent-app.iss"
    $iconPath = Join-Path $repoRoot "crates\pa-ui\assets\icon.ico"
    foreach ($req in @($issPath, $iconPath)) {
        if (-not (Test-Path -LiteralPath $req -PathType Leaf)) {
            throw "Missing required packaging file: $req"
        }
    }

    & $IsccPath `
        "/DMyAppVersion=$cleanVersion" `
        "/DSourceDir=$stageRoot" `
        "/DOutputDir=$OutDir" `
        "/DIconPath=$iconPath" `
        $issPath
    if ($LASTEXITCODE -ne 0) {
        throw "ISCC.exe failed with exit code $LASTEXITCODE"
    }

    $setupPath = Join-Path $OutDir "PrimeAgent-Setup-$cleanVersion-windows-x64.exe"
    if (-not (Test-Path -LiteralPath $setupPath -PathType Leaf)) {
        throw "Expected installer was not generated: $setupPath"
    }
    $artifacts.Add($setupPath)
}

$checksumLines = foreach ($artifact in $artifacts) {
    $hash = (Get-FileHash -LiteralPath $artifact -Algorithm SHA256).Hash.ToLowerInvariant()
    $leaf = Split-Path -Leaf $artifact
    "$hash  $leaf"
}
$checksumPath = Join-Path $OutDir "SHA256SUMS.txt"
$checksumContent = ($checksumLines -join "`n") + "`n"
[System.IO.File]::WriteAllText($checksumPath, $checksumContent, (New-Object System.Text.UTF8Encoding($false)))

Write-Host "Packaged Prime Agent $cleanVersion into $OutDir"
foreach ($line in $checksumLines) {
    Write-Host "  $line"
}
