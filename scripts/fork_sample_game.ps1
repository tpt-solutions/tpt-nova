<#
.SYNOPSIS
    Fork nova-sample-game into a new standalone project.

.DESCRIPTION
    Copies crates/nova-sample-game into a new directory under $OutDir, rewrites
    the crate name in its Cargo.toml, and prints the next steps. This replaces
    the manual copy/rename-by-hand flow described in the onboarding docs.

.PARAMETER Name
    The new crate/directory name, e.g. "my-game".

.PARAMETER OutDir
    Where to create the fork (defaults to the current directory).

.EXAMPLE
    pwsh scripts/fork_sample_game.ps1 -Name my-game
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$Name,

    [string]$OutDir = "."
)

$ErrorActionPreference = 'Stop'

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$Src = Join-Path $RepoRoot 'crates' 'nova-sample-game'
if (-not (Test-Path $Src)) {
    Write-Error "source not found: $Src"
}

$Dst = Join-Path $OutDir $Name
if (Test-Path $Dst) {
    Write-Error "destination already exists: $Dst"
}

Write-Host "Forking nova-sample-game -> $Dst"
Copy-Item -Recurse -Force $Src $Dst

$Cargo = Join-Path $Dst 'Cargo.toml'
if (Test-Path $Cargo) {
    (Get-Content $Cargo) -replace 'name = "nova-sample-game"', "name = `"$Name`"" |
        Set-Content $Cargo
    Write-Host "  rewrote crate name in $Cargo"
}

Write-Host ""
Write-Host "Done. Next steps:"
Write-Host "  cd $Dst"
Write-Host "  cargo run"
Write-Host ""
Write-Host "If you want the fork to live outside this workspace, also add it to"
Write-Host "a Cargo workspace (or run it standalone) and update any asset paths."
