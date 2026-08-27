# Rebuilds the kvc CLI and copies it over the installed one the Krita docker/plugin calls,
# so backend changes show up without a full `npm run tauri build` + reinstall.
# Usage: .\scripts\update-installed-kvc.ps1

$repoRoot = Join-Path $PSScriptRoot ".."
$dest = Join-Path $env:LOCALAPPDATA "krita-vc\kvc.exe"

Push-Location (Join-Path $repoRoot "src-tauri")
cargo build --release --bin kvc
$buildOk = $?
Pop-Location

if (-not $buildOk) {
    Write-Error "cargo build failed; not touching the installed kvc.exe"
    exit 1
}

Copy-Item -Path (Join-Path $repoRoot "src-tauri\target\release\kvc.exe") -Destination $dest -Force
Write-Output "Updated $dest"
