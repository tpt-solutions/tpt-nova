<#
.SYNOPSIS
    Fork nova-sample-game into a new standalone project.

.DESCRIPTION
    Copies crates/nova-sample-game into a user-supplied target directory and
    performs a best-effort, project-name rename across Cargo.toml and the Rust
    sources. This replaces the manual copy/rename-by-hand flow described in the
    project README.

    It is NOT a perfect rename-everywhere; it renames the crate name and the
    default lib identifier. Any remaining manual steps are printed at the end.

.PARAMETER Target
    Destination path for the new project (a directory). The final path
    component is used as the new crate name, e.g. "../my-game" -> "my-game".

.PARAMETER Source
    Source crate to fork. Defaults to "crates/nova-sample-game" relative to the
    repository root (the script resolves the repo root automatically).

.EXAMPLE
    pwsh scripts/scaffold_sample_game.ps1 ../my-awesome-game
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$Target,

    [string]$Source = "crates/nova-sample-game"
)

$ErrorActionPreference = "Stop"

# --- Locate repository root (the dir containing a Cargo.toml with [workspace]).
function Find-RepoRoot {
    $dir = (Get-Location).Path
    while ($true) {
        if (Test-Path (Join-Path $dir "Cargo.toml")) {
            $text = Get-Content -LiteralPath (Join-Path $dir "Cargo.toml") -Raw
            if ($text -match "\[workspace\]") { return $dir }
        }
        $parent = Split-Path -Parent $dir
        if ($parent -eq $dir -or -not $parent) {
            throw "Could not locate workspace root (Cargo.toml with [workspace])."
        }
        $dir = $parent
    }
}

$repoRoot = Find-RepoRoot
$sourcePath = Join-Path $repoRoot $Source

if (-not (Test-Path -LiteralPath $sourcePath)) {
    Write-Error "Source crate not found: $sourcePath"
    exit 1
}

# --- Resolve destination and derive the new crate name.
$destPath = Resolve-Path -Path $Target -ErrorAction SilentlyContinue
if (-not $destPath) {
    # Resolve-Path fails for not-yet-existing paths; build it manually.
    $destPath = if ([System.IO.Path]::IsPathRooted($Target)) {
        $Target
    } else {
        Join-Path (Get-Location).Path $Target
    }
}
$destPath = $destPath.TrimEnd('\', '/')

$newName = Split-Path -Leaf $destPath
# Cargo crate names must be valid: lowercase, digits, `-`, `_`. Force lower-case.
$newName = $newName.ToLowerInvariant()
$newLib = $newName -replace '-', '_'

# Old identifiers (nova-sample-game -> nova_sample_game).
$oldName = "nova-sample-game"
$oldLib = "nova_sample_game"

Write-Host "Repo root : $repoRoot"
Write-Host "Source    : $sourcePath"
Write-Host "Target    : $destPath"
Write-Host "New crate : $newName  (lib ident: $newLib)"
Write-Host ""

# --- Safety: never overwrite an existing directory without confirmation.
if (Test-Path -LiteralPath $destPath) {
    $reply = Read-Host "Target '$destPath' already exists. Overwrite its contents? [y/N]"
    if ($reply -notmatch '^[yY]') {
        Write-Host "Aborted. Nothing was changed."
        exit 0
    }
    Remove-Item -LiteralPath $destPath -Recurse -Force
}

# --- Copy the crate.
Write-Host "Copying crate..."
Copy-Item -LiteralPath $sourcePath -Destination $destPath -Recurse -Force

# --- Best-effort rename of the crate name and lib identifier.
Write-Host "Renaming identifiers ($oldName -> $newName, $oldLib -> $newLib)..."
$files = Get-ChildItem -LiteralPath $destPath -Recurse -File |
    Where-Object { $_.Extension -in '.toml', '.rs', '.md' }

foreach ($file in $files) {
    $text = Get-Content -LiteralPath $file.FullName -Raw
    if ($null -eq $text) { continue }
    $original = $text
    $text = $text.Replace($oldName, $newName)
    $text = $text.Replace($oldLib, $newLib)
    if ($text -ne $original) {
        Set-Content -LiteralPath $file.FullName -Value $text -NoNewline
        Write-Host "    updated $($file.FullName.Substring($destPath.Length + 1))"
    }
}

Write-Host ""
Write-Host "Done. Scaffold created at: $destPath"
Write-Host ""
Write-Host "Remaining manual steps:"
Write-Host "  1. Default lib/binary name is derived from '$newName'. If you want a"
Write-Host "     custom [lib] name or [[bin]] path, edit $destPath/Cargo.toml."
Write-Host "  2. Re-run 'cargo build -p $newName' from the repo root to verify."
Write-Host "  3. If you added this as a workspace member, add '$Target' to the"
Write-Host "     root Cargo.toml [workspace.members] list."
Write-Host "  4. Strings/paths/identifiers not matching '$oldName'/'$oldLib'"
Write-Host "     (e.g. doc comments, asset file names) were left untouched."
