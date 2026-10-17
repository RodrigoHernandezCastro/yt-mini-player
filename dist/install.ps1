<#
.SYNOPSIS
    Instala o desinstala YT Mini Player.

.DESCRIPTION
    Instalación normal (por defecto):
      - Copia yt-mini-player.exe (y WebView2Loader.dll si existe) a
        %LOCALAPPDATA%\Programs\YT Mini Player. No requiere administrador.
      - Crea accesos directos en el menú Inicio y en el escritorio.

    Superposición máxima (-Overlay):
      Para que el reproductor quede encima de apps y juegos a pantalla
      completa, Windows exige el mismo mecanismo que usa el teclado en
      pantalla (UIAccess): un .exe firmado e instalado en Archivos de
      programa. Este modo, con permiso de administrador una sola vez:
      - Copia yt-mini-player-uia.exe a C:\Program Files\YT Mini Player.
      - Comprueba la huella SHA-256 de la copia instalada contra
        SHA256SUMS.txt ANTES de firmarla (la copia ya está en una carpeta que
        solo un administrador puede modificar).
      - Crea un certificado de firma local ("YT Mini Player (firma local)"),
        solo para firmar código y sin poder emitir otros certificados, lo
        marca como de confianza en este equipo, firma el .exe y BORRA la
        clave privada: el certificado no puede firmar nada más.
      - Quita la instalación normal del usuario para no tener dos copias.
      Desinstalar.cmd lo deshace todo, certificado incluido.

    Desinstalar (-Uninstall):
      - Quita ambas instalaciones, accesos directos, inicio con Windows y el
        certificado. La sesión y los ajustes (%LOCALAPPDATA%\yt_mini_player)
        se conservan salvo con -RemoveData.

.EXAMPLE
    .\install.ps1
.EXAMPLE
    .\install.ps1 -Overlay
.EXAMPLE
    .\install.ps1 -Uninstall -RemoveData
#>
[CmdletBinding()]
param(
    [switch]$Overlay,
    [switch]$Uninstall,
    [switch]$RemoveData,
    [switch]$NoDesktop,
    [switch]$NoLaunch,
    [switch]$Yes,
    # Uso interno: las partes que necesitan administrador.
    [switch]$MachineInstall,
    [switch]$MachineUninstall,
    [string]$LogPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$AppName = 'YT Mini Player'
$ExeName = 'yt-mini-player.exe'
$UiaExeName = 'yt-mini-player-uia.exe'
$CertSubject = 'CN=YT Mini Player (firma local)'

$UserDir = Join-Path $env:LOCALAPPDATA "Programs\$AppName"
$DataDir = Join-Path $env:LOCALAPPDATA 'yt_mini_player'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$UserStartLink = Join-Path ([Environment]::GetFolderPath('Programs')) "$AppName.lnk"
$UserDesktopLink = Join-Path ([Environment]::GetFolderPath('Desktop')) "$AppName.lnk"

$MachineDir = Join-Path $env:ProgramFiles $AppName
$MachineExe = Join-Path $MachineDir $ExeName
$CommonStartLink = Join-Path ([Environment]::GetFolderPath('CommonPrograms')) "$AppName.lnk"
$CommonDesktopLink = Join-Path ([Environment]::GetFolderPath('CommonDesktopDirectory')) "$AppName.lnk"

# ---------------------------------------------------------------------------
# Utilidades
# ---------------------------------------------------------------------------

# Mensaje al usuario; en la parte elevada también queda en el registro que
# la ventana original muestra después.
function Say([string]$Text) {
    Write-Host $Text
    if ($LogPath) { Add-Content -Path $LogPath -Value $Text -Encoding UTF8 }
}

function Test-Admin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    ([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

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
    $link.Description = 'Reproductor flotante de YouTube y otros servicios'
    $link.Save()
}

function Remove-IfExists([string[]]$Paths) {
    foreach ($path in $Paths) {
        if (Test-Path $path) { Remove-Item $path -Recurse -Force }
    }
}

# Inicio con Windows: un .exe con UIAccess solo arranca mediante ShellExecute,
# así que se lanza a través de shell32 (igual que hace la propia app).
function Get-AutostartCommand([string]$Target, [bool]$UiAccess) {
    if ($UiAccess) {
        $rundll32 = Join-Path $env:SystemRoot 'System32\rundll32.exe'
        return "`"$rundll32`" shell32.dll,ShellExec_RunDLL `"$Target`" --hidden"
    }
    return "`"$Target`" --hidden"
}

function Update-Autostart([string]$Target, [bool]$UiAccess) {
    if (Get-ItemProperty -Path $RunKey -Name $AppName -ErrorAction SilentlyContinue) {
        Set-ItemProperty -Path $RunKey -Name $AppName -Value (Get-AutostartCommand $Target $UiAccess)
    }
}

# Ejecuta este mismo script como administrador y devuelve su código de
# salida. Lo que escribe se guarda en un registro y se muestra aquí.
function Invoke-Elevated([string]$Mode) {
    $log = Join-Path $env:TEMP ('ytmp-install-' + [guid]::NewGuid().ToString('N') + '.log')
    $arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -$Mode -LogPath `"$log`""
    if ($NoDesktop) { $arguments += ' -NoDesktop' }
    try {
        $process = Start-Process -FilePath 'powershell.exe' -Verb RunAs -ArgumentList $arguments -Wait -PassThru
    } catch {
        throw 'Hace falta permiso de administrador y la solicitud de Windows se canceló.'
    }
    if (Test-Path $log) {
        Get-Content $log -Encoding UTF8 | ForEach-Object { Write-Host "  $_" }
        Remove-Item $log -Force -ErrorAction SilentlyContinue
    }
    return $process.ExitCode
}

function Remove-OverlayCertificates {
    foreach ($store in 'Root', 'TrustedPublisher', 'My') {
        Get-ChildItem "Cert:\LocalMachine\$store" |
            Where-Object { $_.Subject -eq $CertSubject } |
            ForEach-Object {
                if ($store -eq 'My') { Remove-Item $_.PSPath -DeleteKey -Force }
                else { Remove-Item $_.PSPath -Force }
            }
    }
}

# ---------------------------------------------------------------------------
# Integridad: huellas SHA-256 publicadas con la versión
# ---------------------------------------------------------------------------

# Comprueba `$Path` contra la línea de `$Name` en SHA256SUMS.txt (formato de
# sha256sum: "<hash>  <nombre>"). Sin el archivo de huellas solo avisa.
function Test-Checksum([string]$Path, [string]$Name) {
    $sums = Join-Path $PSScriptRoot 'SHA256SUMS.txt'
    if (-not (Test-Path $sums)) {
        Write-Warning 'Falta SHA256SUMS.txt: no se puede comprobar que los archivos estén intactos.'
        return
    }
    $line = Get-Content $sums | Where-Object { $_ -match ('^([0-9a-fA-F]{64})\s+\*?' + [regex]::Escape($Name) + '$') } | Select-Object -First 1
    if (-not $line) { throw "SHA256SUMS.txt no tiene la huella de $Name." }
    $expected = ($line -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "$Name no coincide con SHA256SUMS.txt: la descarga está dañada o fue modificada. Vuelve a descargarla."
    }
    Write-Host "  SHA-256 $Name`: $actual (correcta)"
}

# ---------------------------------------------------------------------------
# Partes que necesitan administrador
# ---------------------------------------------------------------------------

function Install-Machine {
    $source = Join-Path $PSScriptRoot $UiaExeName
    if (-not (Test-Path $source)) { throw "No se encontró $UiaExeName junto a este script." }

    Stop-App
    New-Item -ItemType Directory -Force -Path $MachineDir | Out-Null
    Copy-Item $source $MachineExe -Force
    $loader = Join-Path $PSScriptRoot 'WebView2Loader.dll'
    if (Test-Path $loader) { Copy-Item $loader $MachineDir -Force }
    Get-ChildItem $MachineDir | Unblock-File -ErrorAction SilentlyContinue

    try {
        # Se comprueba la COPIA, ya en Archivos de programa: nadie sin
        # permisos de administrador puede cambiarla entre la comprobación y
        # la firma.
        Test-Checksum $MachineExe $UiaExeName
        $machineLoader = Join-Path $MachineDir 'WebView2Loader.dll'
        if (Test-Path $machineLoader) { Test-Checksum $machineLoader 'WebView2Loader.dll' }

        Remove-OverlayCertificates
        Say 'Creando el certificado de firma local...'
        $certArgs = @{
            Type              = 'CodeSigningCert'
            Subject           = $CertSubject
            FriendlyName      = 'YT Mini Player (firma local)'
            CertStoreLocation = 'Cert:\LocalMachine\My'
            KeyAlgorithm      = 'RSA'
            KeyLength         = 3072
            HashAlgorithm     = 'SHA256'
            KeyExportPolicy   = 'NonExportable'
            NotAfter          = (Get-Date).AddYears(10)
        }
        try {
            # Restricciones básicas: no es una autoridad (no puede emitir otros
            # certificados). Uso: solo firma de código (lo pone CodeSigningCert).
            $cert = New-SelfSignedCertificate @certArgs -TextExtension @('2.5.29.19={critical}{text}ca=0')
        } catch {
            $cert = New-SelfSignedCertificate @certArgs
        }
        try {
            $cer = Join-Path $env:TEMP ('ytmp-' + [guid]::NewGuid().ToString('N') + '.cer')
            Export-Certificate -Cert $cert -FilePath $cer | Out-Null
            Import-Certificate -FilePath $cer -CertStoreLocation 'Cert:\LocalMachine\Root' | Out-Null
            Import-Certificate -FilePath $cer -CertStoreLocation 'Cert:\LocalMachine\TrustedPublisher' | Out-Null
            Remove-Item $cer -Force
            Say 'Firmando el ejecutable...'
            Set-AuthenticodeSignature -FilePath $MachineExe -Certificate $cert -HashAlgorithm SHA256 | Out-Null
        } finally {
            # La clave privada se destruye siempre, haya ido bien o no: el
            # certificado queda solo como "sello" de este .exe concreto.
            Remove-Item -Path ('Cert:\LocalMachine\My\' + $cert.Thumbprint) -DeleteKey -Force
        }
        $signature = Get-AuthenticodeSignature -FilePath $MachineExe
        if ($signature.Status -ne 'Valid') {
            throw "La firma del ejecutable no es válida: $($signature.StatusMessage)"
        }
    } catch {
        Remove-OverlayCertificates
        Remove-IfExists @($MachineDir)
        throw
    }

    New-Shortcut $CommonStartLink $MachineExe
    if (-not $NoDesktop) { New-Shortcut $CommonDesktopLink $MachineExe }
    Say "Instalado y firmado en $MachineDir"
}

function Uninstall-Machine {
    Stop-App
    Remove-IfExists @($CommonStartLink, $CommonDesktopLink, $MachineDir)
    Remove-OverlayCertificates
    Say 'Se quitaron la versión de superposición y su certificado.'
}

if ($MachineInstall -or $MachineUninstall) {
    if (-not (Test-Admin)) { Write-Error 'Esta parte debe ejecutarse como administrador.'; exit 1 }
    $code = 0
    try {
        if ($MachineInstall) { Install-Machine } else { Uninstall-Machine }
    } catch {
        Say "ERROR: $($_.Exception.Message)"
        $code = 1
    }
    exit $code
}

# ---------------------------------------------------------------------------
# Desinstalar
# ---------------------------------------------------------------------------

if ($Uninstall) {
    Stop-App
    Remove-IfExists @($UserStartLink, $UserDesktopLink, $UserDir)
    if (Get-ItemProperty -Path $RunKey -Name $AppName -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -Path $RunKey -Name $AppName
    }
    $hasCert = @(Get-ChildItem 'Cert:\LocalMachine\Root' | Where-Object { $_.Subject -eq $CertSubject }).Count -gt 0
    if ((Test-Path $MachineDir) -or $hasCert) {
        Write-Host 'Quitando la versión de superposición (Windows pedirá permiso)...'
        if ((Invoke-Elevated 'MachineUninstall') -ne 0) {
            Write-Warning "No se pudo quitar $MachineDir ni el certificado. Vuelve a ejecutar Desinstalar.cmd."
        }
    }
    if ($RemoveData -and (Test-Path $DataDir)) {
        Remove-Item $DataDir -Recurse -Force
        Write-Host 'Se borraron la sesión, los ajustes y las extensiones.'
    } elseif (Test-Path $DataDir) {
        Write-Host "Se conservan la sesión y los ajustes en $DataDir"
    }
    Write-Host "$AppName se desinstaló."
    exit 0
}

# ---------------------------------------------------------------------------
# Instalar en modo superposición máxima
# ---------------------------------------------------------------------------

if ($Overlay) {
    if (-not $Yes) {
        Write-Host ''
        Write-Host 'Superposición máxima: el reproductor podrá quedar encima de apps y juegos'
        Write-Host 'a pantalla completa (excepto los de pantalla completa EXCLUSIVA).'
        Write-Host ''
        Write-Host 'Para eso, con permiso de administrador una sola vez:'
        Write-Host "  - se instala en $MachineDir"
        Write-Host '  - se crea un certificado local "YT Mini Player (firma local)" de confianza'
        Write-Host '    en este equipo, se firma el programa y se borra la clave privada'
        Write-Host '    (el certificado no puede firmar nada más)'
        Write-Host '  - se quita la instalación normal para no tener dos copias'
        Write-Host 'Desinstalar.cmd lo deshace todo.'
        Write-Host ''
        $answer = Read-Host '¿Continuar? (S/N)'
        if ($answer -notmatch '^[sSyY]') { Write-Host 'Cancelado.'; exit 0 }
    }
    Stop-App
    if ((Invoke-Elevated 'MachineInstall') -ne 0) {
        throw 'No se pudo instalar la versión de superposición (los detalles están arriba).'
    }
    # Sin dos copias: se quita la instalación normal de este usuario.
    Remove-IfExists @($UserStartLink, $UserDesktopLink, $UserDir)
    Update-Autostart $MachineExe $true
    Write-Host ''
    Write-Host "$AppName (superposición máxima) quedó instalado en:"
    Write-Host "  $MachineDir"
    Write-Host ''
    # Se abre como usuario normal: este proceso no es administrador.
    if (-not $NoLaunch) { Start-Process $MachineExe }
    exit 0
}

# ---------------------------------------------------------------------------
# Instalación normal
# ---------------------------------------------------------------------------

$source = Join-Path $PSScriptRoot $ExeName
if (-not (Test-Path $source)) {
    throw "No se encontró $ExeName junto a este script."
}

Stop-App
if (Test-Path $MachineDir) {
    Write-Host 'Cambiando de la versión de superposición a la normal (Windows pedirá permiso)...'
    if ((Invoke-Elevated 'MachineUninstall') -ne 0) {
        Write-Warning "No se pudo quitar $MachineDir. Puedes hacerlo luego con Desinstalar.cmd."
    }
}

Test-Checksum $source $ExeName
$loader = Join-Path $PSScriptRoot 'WebView2Loader.dll'
if (Test-Path $loader) { Test-Checksum $loader 'WebView2Loader.dll' }

New-Item -ItemType Directory -Force -Path $UserDir | Out-Null
Copy-Item $source $UserDir -Force
if (Test-Path $loader) { Copy-Item $loader $UserDir -Force }

$target = Join-Path $UserDir $ExeName
# Quita la marca "descargado de Internet" de la copia instalada.
Get-ChildItem $UserDir | Unblock-File -ErrorAction SilentlyContinue

New-Shortcut $UserStartLink $target
if (-not $NoDesktop) { New-Shortcut $UserDesktopLink $target }

# Si el inicio con Windows estaba activo, que apunte a la nueva ubicación.
Update-Autostart $target $false

Write-Host ''
Write-Host "$AppName quedó instalado en:"
Write-Host "  $UserDir"
Write-Host ('Accesos directos: menú Inicio' + $(if ($NoDesktop) { '' } else { ' y escritorio' }))
Write-Host 'Para que quede encima de apps a pantalla completa, usa Instalar-superposicion.cmd.'
Write-Host ''

if (-not $NoLaunch) { Start-Process $target }
