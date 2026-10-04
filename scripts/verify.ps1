[CmdletBinding()]
param(
    [ValidateSet("all", "core", "workspace", "ui")]
    [string]$Suite = "all",
    [switch]$Native
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$WorkspaceRoot = Split-Path -Parent $PSScriptRoot

function Invoke-Cargo {
    param([Parameter(Mandatory)][string[]]$CargoArgs)

    Write-Host "`n> cargo $($CargoArgs -join ' ')" -ForegroundColor Cyan
    & cargo @CargoArgs
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($CargoArgs -join ' ') failed with exit code $LASTEXITCODE"
    }
}

function Invoke-BoundedProcess {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][string[]]$ProcessArgs,
        [Parameter(Mandatory)][int]$TimeoutSeconds
    )

    Write-Host "`n> $FilePath $($ProcessArgs -join ' ')" -ForegroundColor Cyan
    $StartInfo = [Diagnostics.ProcessStartInfo]::new()
    $StartInfo.FileName = $FilePath
    $StartInfo.UseShellExecute = $false
    $StartInfo.CreateNoWindow = $true
    foreach ($Argument in $ProcessArgs) {
        $StartInfo.ArgumentList.Add($Argument)
    }

    $Process = [Diagnostics.Process]::Start($StartInfo)
    if (-not $Process.WaitForExit($TimeoutSeconds * 1000)) {
        $Process.Kill($true)
        $Process.WaitForExit()
        throw "$FilePath timed out after $TimeoutSeconds seconds"
    }
    if ($Process.ExitCode -ne 0) {
        throw "$FilePath failed with exit code $($Process.ExitCode)"
    }
}

Push-Location $WorkspaceRoot
try {
    Invoke-Cargo @("fmt", "--all", "--", "--check")

    switch ($Suite) {
        "core" {
            Invoke-Cargo @("check", "-p", "sniplet-core", "--all-targets", "--locked")
            Invoke-Cargo @("test", "-p", "sniplet-core", "--all-targets", "--locked")
            Invoke-Cargo @("clippy", "-p", "sniplet-core", "--all-targets", "--locked", "--", "-D", "warnings")
        }
        "workspace" {
            Invoke-Cargo @("check", "--workspace", "--all-targets", "--locked")
            Invoke-Cargo @("test", "--workspace", "--all-targets", "--locked")
            Invoke-Cargo @("clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings")
        }
        "ui" {
            Invoke-Cargo @("check", "-p", "sniplet-app", "--all-targets", "--features", "ui-tests", "--locked")
            Invoke-Cargo @("test", "-p", "sniplet-app", "--all-targets", "--features", "ui-tests", "--locked")
            Invoke-Cargo @("clippy", "-p", "sniplet-app", "--all-targets", "--features", "ui-tests", "--locked", "--", "-D", "warnings")
        }
        "all" {
            Invoke-Cargo @("check", "--workspace", "--all-targets", "--all-features", "--locked")
            Invoke-Cargo @("build", "--workspace", "--all-targets", "--all-features", "--locked")

            # This remains a distinct command so the platform-free core can be
            # diagnosed without relying on a desktop backend.
            Invoke-Cargo @("test", "-p", "sniplet-core", "--all-targets", "--locked")
            Invoke-Cargo @("test", "--workspace", "--exclude", "sniplet-core", "--all-targets", "--locked")
            Invoke-Cargo @("test", "-p", "sniplet-app", "--all-targets", "--features", "ui-tests", "--locked")

            Invoke-Cargo @("clippy", "--workspace", "--all-targets", "--all-features", "--locked", "--", "-D", "warnings")
        }
    }

    if ($Native) {
        Invoke-Cargo @("build", "-p", "sniplet-app", "--locked")
        $Metadata = (& cargo metadata --format-version 1 --no-deps --locked | ConvertFrom-Json)
        if ($LASTEXITCODE -ne 0) {
            throw "cargo metadata failed with exit code $LASTEXITCODE"
        }
        $Executable = Join-Path ([string]$Metadata.target_directory) "debug\sniplet.exe"
        if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
            throw "Sniplet executable not found at $Executable"
        }

        Invoke-BoundedProcess -FilePath $Executable -ProcessArgs @("--demo", "--smoke", "--normal-window") -TimeoutSeconds 20
        Invoke-BoundedProcess -FilePath $Executable -ProcessArgs @("--self-test", (Join-Path $WorkspaceRoot "artifacts\self-test")) -TimeoutSeconds 60
    }
}
finally {
    Pop-Location
}
