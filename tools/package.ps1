<#
.SYNOPSIS
    Compila en release y arma dist\YT-Mini-Player-<versión>-win64.zip.

.DESCRIPTION
    Requiere Rust y las Build Tools de Visual Studio (C++). Con MSVC el
    cargador de WebView2 queda enlazado dentro del .exe, así que el paquete
    no necesita WebView2Loader.dll.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\package.ps1
#>
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'La compilación falló.' }

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
$name = "YT-Mini-Player-$version-win64"
$out = Join-Path 'dist' $name
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force -Path $out | Out-Null

Copy-Item 'target\release\yt-mini-player.exe' $out
foreach ($file in 'install.ps1', 'Instalar.cmd', 'Desinstalar.cmd', 'LEEME.txt') {
    Copy-Item (Join-Path 'dist' $file) $out
}

$zip = Join-Path 'dist' "$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $out '*') -DestinationPath $zip
Write-Host "Paquete listo: $zip"
