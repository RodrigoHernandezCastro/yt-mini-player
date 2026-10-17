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

# Los planes de pago de "Mi música" necesitan los datos de la tienda (ver
# README, "Selling the plans"). Sin ellos la compilación sale igual, con
# las funciones de pago bloqueadas.
if (-not $env:YTMP_LS_STORE_ID) {
    Write-Warning 'YTMP_LS_STORE_ID no está definido: los planes de pago quedarán bloqueados en esta compilación.'
}
if ($env:YTMP_DEV_UNLOCK -eq '1') {
    throw 'YTMP_DEV_UNLOCK=1 desbloquea todo: no se empaqueta una compilación de desarrollo.'
}

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'La compilación falló.' }

# Variante de superposición máxima (manifiesto con uiAccess="true"); se
# compila aparte para no pisar la normal.
$env:YTMP_UIACCESS = '1'
try {
    cargo build --release --target-dir target\uia
    if ($LASTEXITCODE -ne 0) { throw 'La compilación de la variante de superposición falló.' }
} finally {
    Remove-Item Env:\YTMP_UIACCESS -ErrorAction SilentlyContinue
}

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
$name = "YT-Mini-Player-$version-win64"
$out = Join-Path 'dist' $name
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force -Path $out | Out-Null

Copy-Item 'target\release\yt-mini-player.exe' $out
Copy-Item 'target\uia\release\yt-mini-player.exe' (Join-Path $out 'yt-mini-player-uia.exe')
foreach ($file in 'install.ps1', 'Instalar.cmd', 'Instalar-superposicion.cmd', 'Desinstalar.cmd', 'LEEME.txt') {
    Copy-Item (Join-Path 'dist' $file) $out
}

# Huellas SHA-256 de todo el paquete: el instalador las comprueba antes de
# instalar (y, en superposición máxima, antes de firmar). Formato de sha256sum.
$sums = Get-ChildItem $out -File | Sort-Object Name | ForEach-Object {
    '{0}  {1}' -f (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant(), $_.Name
}
[IO.File]::WriteAllLines((Join-Path $out 'SHA256SUMS.txt'), [string[]]$sums)

$zip = Join-Path 'dist' "$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $out '*') -DestinationPath $zip
Write-Host "Paquete listo: $zip"
Write-Host "SHA-256 del zip: $((Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant())"
