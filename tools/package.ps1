<#
.SYNOPSIS
    Builds in release mode and puts together dist\YT-Mini-Player-<version>-win64.zip.

.DESCRIPTION
    Needs Rust and the Visual Studio Build Tools (C++). With MSVC the
    WebView2 loader is linked into the .exe, so the package does not need
    WebView2Loader.dll.

.PARAMETER BuildOnly
    Only builds both programs (and checks they are not the test build).

.PARAMETER Prebuilt
    Packages programs already in target\release and target\uia\release
    without building anything. release.yml builds in one job and signs in
    another, so no dependency's build code runs where signing is possible.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\package.ps1
#>
[CmdletBinding()]
param([switch]$BuildOnly, [switch]$Prebuilt)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
if ($BuildOnly -and $Prebuilt) { throw 'Either -BuildOnly or -Prebuilt.' }

if (-not $Prebuilt) {
    # My music's paid plans need the store's details (see README, "Selling
    # the plans"). Without them the build still works, with the paid
    # features locked.
    if (-not $env:YTMP_LS_STORE_ID) {
        Write-Warning 'YTMP_LS_STORE_ID is not set: the paid plans will be locked in this build.'
    }
    if (-not $env:YTMP_TOGETHER_RELAY) {
        Write-Warning 'YTMP_TOGETHER_RELAY is not set: Watch together will ask for a server address.'
    }
    if ($env:YTMP_DEV_UNLOCK -eq '1') {
        throw 'YTMP_DEV_UNLOCK=1 unlocks everything: a development build is not packaged.'
    }

    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'The build failed.' }

    # The maximum overlay variant (manifest with uiAccess="true"); built
    # separately so it does not overwrite the normal one.
    $env:YTMP_UIACCESS = '1'
    try {
        cargo build --release --locked --target-dir target\uia
        if ($LASTEXITCODE -ne 0) { throw 'The build of the overlay variant failed.' }
    } finally {
        Remove-Item Env:\YTMP_UIACCESS -ErrorAction SilentlyContinue
    }
}

# Never the drivable test build (src/e2e.rs).
& (Join-Path $PSScriptRoot 'check-release-exe.ps1') -Files 'target\release\yt-mini-player.exe', 'target\uia\release\yt-mini-player.exe'
if ($BuildOnly) {
    Write-Host 'Built: target\release\yt-mini-player.exe and target\uia\release\yt-mini-player.exe'
    return
}

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
$name = "YT-Mini-Player-$version-win64"
$out = Join-Path 'dist' $name
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force -Path $out | Out-Null

Copy-Item 'target\release\yt-mini-player.exe' $out
Copy-Item 'target\uia\release\yt-mini-player.exe' (Join-Path $out 'yt-mini-player-uia.exe')
foreach ($file in 'install.ps1', 'Install.cmd', 'Install-overlay.cmd', 'Uninstall.cmd', 'README.txt', 'ANTIVIRUS.txt') {
    Copy-Item (Join-Path 'dist' $file) $out
}

# Authenticode, if a signing method is set up (docs/SIGNING.md); before the
# fingerprints, which must be those of the signed files.
& (Join-Path $PSScriptRoot 'sign.ps1') -Files @(
    (Join-Path $out 'yt-mini-player.exe'),
    (Join-Path $out 'yt-mini-player-uia.exe'))

# SHA-256 fingerprints of the whole package: the installer checks them
# before installing (and, in maximum overlay, before signing). sha256sum
# format.
$sums = Get-ChildItem $out -File | Sort-Object Name | ForEach-Object {
    '{0}  {1}' -f (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant(), $_.Name
}
[IO.File]::WriteAllLines((Join-Path $out 'SHA256SUMS.txt'), [string[]]$sums)

$zip = Join-Path 'dist' "$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $out '*') -DestinationPath $zip
Write-Host "Package ready: $zip"
Write-Host "Zip SHA-256: $((Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant())"
