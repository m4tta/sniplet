[CmdletBinding()]
param(
    [string]$OutputDirectory = "dist",
    [ValidateSet("release", "debug")]
    [string]$Profile = "release",
    [switch]$SkipBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$WorkspaceRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
if ([IO.Path]::IsPathRooted($OutputDirectory) -or $OutputDirectory -split '[\\/]' -contains '..') {
    throw "OutputDirectory must be a relative path inside the workspace"
}
$OutputRoot = [IO.Path]::GetFullPath((Join-Path $WorkspaceRoot $OutputDirectory))
$WorkspacePrefix = $WorkspaceRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
if (-not $OutputRoot.StartsWith($WorkspacePrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Package output must remain inside $WorkspaceRoot"
}

function Invoke-Cargo {
    param([Parameter(Mandatory)][string[]]$CargoArgs)

    Write-Host "`n> cargo $($CargoArgs -join ' ')" -ForegroundColor Cyan
    & cargo @CargoArgs
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($CargoArgs -join ' ') failed with exit code $LASTEXITCODE"
    }
}

Push-Location $WorkspaceRoot
try {
    if (-not $SkipBuild) {
        $BuildArgs = @("build", "-p", "sniplet-app", "--locked")
        if ($Profile -eq "release") {
            $BuildArgs += "--release"
        }
        Invoke-Cargo $BuildArgs
    }

    $Metadata = (& cargo metadata --format-version 1 --no-deps --locked | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE"
    }
    $Binary = Join-Path ([string]$Metadata.target_directory) "$Profile\sniplet.exe"
    if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
        throw "$Profile executable not found at $Binary"
    }

    $HostTarget = (& rustc -vV | Select-String '^host: ' | ForEach-Object { $_.Line.Substring(6) })
    if (-not $HostTarget) {
        throw "Could not determine the Rust host target"
    }

    $BundleSuffix = if ($Profile -eq "debug") { "-debug" } else { "" }
    $Bundle = [IO.Path]::GetFullPath((Join-Path $OutputRoot "sniplet-$HostTarget$BundleSuffix"))
    $OutputPrefix = $OutputRoot.TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if (-not $Bundle.StartsWith($OutputPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Package bundle must remain inside $OutputRoot"
    }
    if (Test-Path -LiteralPath $Bundle) {
        Remove-Item -LiteralPath $Bundle -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $Bundle | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $Bundle "licenses") | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $Bundle "documentation") | Out-Null

    Copy-Item -LiteralPath $Binary -Destination (Join-Path $Bundle "Sniplet.exe")
    Copy-Item -LiteralPath (Join-Path $WorkspaceRoot "README.md") -Destination (Join-Path $Bundle "README.md")
    Copy-Item -LiteralPath (Join-Path $WorkspaceRoot "LICENSE") -Destination (Join-Path $Bundle "LICENSE")
    Copy-Item -LiteralPath (Join-Path $WorkspaceRoot "assets\fonts\OFL.txt") -Destination (Join-Path $Bundle "licenses\NotoSans-OFL.txt")
    Copy-Item -Path (Join-Path $WorkspaceRoot "docs\*.md") -Destination (Join-Path $Bundle "documentation")

    Write-Host "`nCreated portable Windows package: $Bundle" -ForegroundColor Green
}
finally {
    Pop-Location
}
