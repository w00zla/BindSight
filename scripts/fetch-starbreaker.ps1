# Fetch the StarBreaker CLI binaries BindSight bundles as a Tauri sidecar
# (used at runtime to pull SC's config files out of Data.p4k).
#
# PowerShell counterpart of fetch-starbreaker.sh. Pinned to one release; every
# binary is verified against its SHA256 before it is installed. Re-run after
# bumping the version/hashes below (keep both scripts in sync).
#
# Usage: scripts\fetch-starbreaker.ps1
#
# Source / releases: https://github.com/diogotr7/StarBreaker (MIT)
$ErrorActionPreference = 'Stop'

$Version = '0.3.2'
$BaseUrl = "https://github.com/diogotr7/StarBreaker/releases/download/v$Version"

$Out = Join-Path $PSScriptRoot '..\src-tauri\binaries'
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("starbreaker-" + [System.Guid]::NewGuid())

# $Asset: release asset, $Inner: file inside the archive, $Target: sidecar
# name (Tauri target-triple naming), $Expected: SHA256 of the extracted binary
function Fetch-One($Asset, $Inner, $Target, $Expected) {
    Write-Host "--== $Asset ==--"
    $archive = Join-Path $Tmp $Asset
    Invoke-WebRequest -Uri "$BaseUrl/$Asset" -OutFile $archive -MaximumRetryCount 3

    switch -Wildcard ($Asset) {
        '*.tar.gz' { tar -xzf $archive -C $Tmp $Inner; if ($LASTEXITCODE -ne 0) { throw "tar failed for $Asset" } }
        '*.zip' { Expand-Archive -Path $archive -DestinationPath $Tmp -Force }
        default { throw "unknown archive type: $Asset" }
    }

    $extracted = Join-Path $Tmp $Inner
    $actual = (Get-FileHash -Path $extracted -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Expected) {
        throw "SHA256 mismatch for ${Inner}: expected $Expected, actual $actual"
    }
    Move-Item -Path $extracted -Destination (Join-Path $Out $Target) -Force
    Write-Host "  -> $(Join-Path $Out $Target)"
}

New-Item -ItemType Directory -Path $Out -Force | Out-Null
New-Item -ItemType Directory -Path $Tmp -Force | Out-Null
try {
    Fetch-One "starbreaker-cli-v$Version-linux-x86_64.tar.gz" 'starbreaker' `
        'starbreaker-x86_64-unknown-linux-gnu' `
        '93cd5a7b756131a900e3131c05c994c1de17ad4f6cf2e47321ca5967a071990d'
    Fetch-One "starbreaker-cli-v$Version-windows-x86_64.zip" 'starbreaker.exe' `
        'starbreaker-x86_64-pc-windows-msvc.exe' `
        '82439e45cd5337f058f06ded63ee633bea8de8e4d75525f5a083aa7955b91a10'
}
finally {
    Remove-Item -Path $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ''
Write-Host "~~~~ DONE: StarBreaker v$Version installed to src-tauri/binaries/ ~~~~"
