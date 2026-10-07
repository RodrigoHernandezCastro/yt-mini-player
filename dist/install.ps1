<#
.SYNOPSIS
    Instala o desinstala YT Mini Player para el usuario actual.

.DESCRIPTION
    Instalar:
      - Copia yt-mini-player.exe (y WebView2Loader.dll si existe) a
        %LOCALAPPDATA%\Programs\YT Mini Player. No requiere administrador.
      - Crea accesos directos en el menú Inicio y en el escritorio.
      - Abre la aplicación.
    Desinstalar (-Uninstall):
      - Cierra la aplicación, borra la carpeta, los accesos directos y el
        inicio con Windows. La sesión y los ajustes
        (%LOCALAPPDATA%\yt_mini_player) se conservan salvo con -RemoveData.

.EXAMPLE
    .\install.ps1
.EXAMPLE
    .\install.ps1 -NoDesktop -NoLaunch
.EXAMPLE
    .\install.ps1 -Uninstall -RemoveData
#>
[CmdletBinding()]
param(
    [switch]$Uninstall,
    [switch]$RemoveData,
    [switch]$NoDesktop,
    [switch]$NoLaunch
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$AppName = 'YT Mini Player'
$ExeName = 'yt-mini-player.exe'
$InstallDir = Join-Path $env:LOCALAPPDATA "Programs\$AppName"
$DataDir = Join-Path $env:LOCALAPPDATA 'yt_mini_player'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$StartMenuLink = Join-Path ([Environment]::GetFolderPath('Programs')) "$AppName.lnk"
$DesktopLink = Join-Path ([Environment]::GetFolderPath('Desktop')) "$AppName.lnk"

function Stop-App {
    $running = @(Get-Process -Name 'yt-mini-player' -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        Write-Host 'Cerrando YT Mini Player...'
        $running | Stop-Process -Force
        Start-Sleep -Milliseconds 800
    }
}

function New-Shortcut([string]$Path, [string]$Target) {
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($Path)
    $link.TargetPath = $Target
    $link.WorkingDirectory = Split-Path $Target
    $link.IconLocation = "$Target,0"
    $link.Description = 'Reproductor flotante de YouTube'
    $link.Save()
}

if ($Uninstall) {
    Stop-App
    foreach ($link in @($StartMenuLink, $DesktopLink)) {
        if (Test-Path $link) { Remove-Item $link -Force }
    }
    if (Get-ItemProperty -Path $RunKey -Name $AppName -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -Path $RunKey -Name $AppName
    }
    if (Test-Path $InstallDir) { Remove-Item $InstallDir -Recurse -Force }
    if ($RemoveData -and (Test-Path $DataDir)) {
        Remove-Item $DataDir -Recurse -Force
        Write-Host 'Se borraron la sesión, los ajustes y las extensiones.'
    } elseif (Test-Path $DataDir) {
        Write-Host "Se conservan la sesión y los ajustes en $DataDir"
    }
    Write-Host "$AppName se desinstaló."
    exit 0
}

$source = Join-Path $PSScriptRoot $ExeName
if (-not (Test-Path $source)) {
    throw "No se encontró $ExeName junto a este script."
}

Stop-App
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item $source $InstallDir -Force
$loader = Join-Path $PSScriptRoot 'WebView2Loader.dll'
if (Test-Path $loader) { Copy-Item $loader $InstallDir -Force }

$target = Join-Path $InstallDir $ExeName
# Quita la marca "descargado de Internet" de la copia instalada.
Get-ChildItem $InstallDir | Unblock-File -ErrorAction SilentlyContinue

New-Shortcut $StartMenuLink $target
if (-not $NoDesktop) { New-Shortcut $DesktopLink $target }

# Si el inicio con Windows estaba activo, que apunte a la nueva ubicación.
if (Get-ItemProperty -Path $RunKey -Name $AppName -ErrorAction SilentlyContinue) {
    Set-ItemProperty -Path $RunKey -Name $AppName -Value "`"$target`" --hidden"
}

Write-Host ''
Write-Host "$AppName quedó instalado en:"
Write-Host "  $InstallDir"
Write-Host ('Accesos directos: menú Inicio' + $(if ($NoDesktop) { '' } else { ' y escritorio' }))
Write-Host ''

if (-not $NoLaunch) { Start-Process $target }
