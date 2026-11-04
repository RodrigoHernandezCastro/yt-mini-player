<#
.SYNOPSIS
    Installs or uninstalls YT Mini Player.

.DESCRIPTION
    Normal install (default):
      - Copies yt-mini-player.exe (and WebView2Loader.dll if present) to
        %LOCALAPPDATA%\Programs\YT Mini Player. No administrator needed.
      - Creates shortcuts in the Start menu and on the desktop.

    Maximum overlay (-Overlay):
      For the player to stay above full-screen apps and games, Windows
      requires the same mechanism the on-screen keyboard uses (UIAccess): a
      signed .exe installed in Program Files. This mode, with administrator
      permission once:
      - Copies yt-mini-player-uia.exe to C:\Program Files\YT Mini Player.
      - Checks the installed copy's SHA-256 fingerprint against
        SHA256SUMS.txt BEFORE signing it (the copy is already in a folder only
        an administrator can change).
      - Creates a local signing certificate ("YT Mini Player (local signing)"),
        for code signing only and unable to issue other certificates, trusts it
        on this PC, signs the .exe and DELETES the private key: the certificate
        cannot sign anything else.
      - Removes the user's normal install so there are not two copies.
      Uninstall.cmd undoes all of it, certificate included.

    Uninstall (-Uninstall):
      - Removes both installs, shortcuts, starting with Windows and the
        certificate. The session and settings (%LOCALAPPDATA%\yt_mini_player)
        are kept unless -RemoveData is given.

    Messages follow the Windows display language (English, Spanish, Russian,
    Polish or Japanese; English for any other).

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
    # Internal use: the parts that need an administrator.
    [switch]$MachineInstall,
    [switch]$MachineUninstall,
    [string]$LogPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$AppName = 'YT Mini Player'
$ExeName = 'yt-mini-player.exe'
$UiaExeName = 'yt-mini-player-uia.exe'
$CertSubject = 'CN=YT Mini Player (local signing)'
# Subjects older versions used, so their certificates are found and removed.
$LegacyCertSubjects = @('CN=YT Mini Player (firma local)')

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
# Messages, written for each language (not word for word). In OverlayIntro,
# lines 6 to 8 are about the local certificate: a signed package replaces
# them with OverlaySignedLine.
# ---------------------------------------------------------------------------

$Messages = @{
    en = @{
        Closing          = 'Closing YT Mini Player...'
        ShortcutInfo     = 'Floating player for YouTube and other services'
        NeedAdmin        = 'Administrator permission is needed, and the Windows prompt was cancelled.'
        NoSums           = 'SHA256SUMS.txt is missing, so the files cannot be checked.'
        NoSumFor         = 'SHA256SUMS.txt has no fingerprint for {0}.'
        SumMismatch      = "{0} doesn't match SHA256SUMS.txt: the download is damaged or was changed. Download it again."
        SumOk            = '  SHA-256 {0}: {1} (matches)'
        NotFound         = "{0} wasn't found next to this script. Your antivirus may have quarantined it: see ANTIVIRUS.txt."
        AlreadySigned    = 'The program is signed by {0}: no local certificate is needed.'
        OverlaySignedLine = '  - the program is already signed by {1}, so no certificate is created'
        CreatingCert     = 'Creating the local signing certificate...'
        Signing          = 'Signing the program...'
        BadSignature     = "The program's signature isn't valid: {0}"
        InstalledSigned  = 'Installed and signed in {0}'
        OverlayRemoved   = 'The overlay version and its certificate were removed.'
        MustBeAdmin      = 'This part must run as administrator.'
        Error            = 'ERROR: {0}'
        RemovingOverlay  = 'Removing the overlay version (Windows will ask for permission)...'
        RemoveOverlayFailed = "{0} and the certificate couldn't be removed. Run Uninstall.cmd again."
        DataRemoved      = 'The session, settings and extensions were deleted.'
        DataKept         = 'The session and settings are kept in {0}'
        Uninstalled      = '{0} was uninstalled.'
        OverlayIntro     = @(
            'Maximum overlay: the player can stay above full-screen apps and games',
            "(except those in EXCLUSIVE full screen).",
            '',
            'For that, with administrator permission once:',
            '  - it is installed in {0}',
            '  - a local certificate, "YT Mini Player (local signing)", is created and',
            '    trusted on this PC, the program is signed and the private key is deleted',
            '    (the certificate cannot sign anything else)',
            '  - the normal install is removed so there are not two copies',
            'Uninstall.cmd undoes all of it.')
        Continue         = 'Continue? (Y/N)'
        Yes              = '^[yY]'
        Cancelled        = 'Cancelled.'
        OverlayFailed    = "The overlay version couldn't be installed (details above)."
        OverlayInstalled = '{0} (maximum overlay) is installed in:'
        SwitchingToNormal = 'Switching from the overlay version to the normal one (Windows will ask for permission)...'
        SwitchFailed     = "{0} couldn't be removed. You can do it later with Uninstall.cmd."
        Installed        = '{0} is installed in:'
        ShortcutsBoth    = 'Shortcuts: Start menu and desktop'
        ShortcutsStart   = 'Shortcuts: Start menu'
        OverlayHint      = 'To keep it above full-screen apps, use Install-overlay.cmd.'
    }
    es = @{
        Closing          = 'Cerrando YT Mini Player...'
        ShortcutInfo     = 'Reproductor flotante de YouTube y otros servicios'
        NeedAdmin        = 'Hace falta permiso de administrador y la solicitud de Windows se canceló.'
        NoSums           = 'Falta SHA256SUMS.txt: no se puede comprobar que los archivos estén intactos.'
        NoSumFor         = 'SHA256SUMS.txt no tiene la huella de {0}.'
        SumMismatch      = '{0} no coincide con SHA256SUMS.txt: la descarga está dañada o fue modificada. Vuelve a descargarla.'
        SumOk            = '  SHA-256 {0}: {1} (correcta)'
        NotFound         = 'No se encontró {0} junto a este script. Puede que tu antivirus lo haya puesto en cuarentena: mira ANTIVIRUS.txt.'
        AlreadySigned    = 'El programa viene firmado por {0}: no hace falta un certificado local.'
        OverlaySignedLine = '  - el programa ya viene firmado por {1}, así que no se crea ningún certificado'
        CreatingCert     = 'Creando el certificado de firma local...'
        Signing          = 'Firmando el ejecutable...'
        BadSignature     = 'La firma del ejecutable no es válida: {0}'
        InstalledSigned  = 'Instalado y firmado en {0}'
        OverlayRemoved   = 'Se quitaron la versión de superposición y su certificado.'
        MustBeAdmin      = 'Esta parte debe ejecutarse como administrador.'
        Error            = 'ERROR: {0}'
        RemovingOverlay  = 'Quitando la versión de superposición (Windows pedirá permiso)...'
        RemoveOverlayFailed = 'No se pudo quitar {0} ni el certificado. Vuelve a ejecutar Uninstall.cmd.'
        DataRemoved      = 'Se borraron la sesión, los ajustes y las extensiones.'
        DataKept         = 'Se conservan la sesión y los ajustes en {0}'
        Uninstalled      = '{0} se desinstaló.'
        OverlayIntro     = @(
            'Superposición máxima: el reproductor podrá quedar encima de apps y juegos',
            'a pantalla completa (excepto los de pantalla completa EXCLUSIVA).',
            '',
            'Para eso, con permiso de administrador una sola vez:',
            '  - se instala en {0}',
            '  - se crea un certificado local "YT Mini Player (local signing)" de confianza',
            '    en este equipo, se firma el programa y se borra la clave privada',
            '    (el certificado no puede firmar nada más)',
            '  - se quita la instalación normal para no tener dos copias',
            'Uninstall.cmd lo deshace todo.')
        Continue         = '¿Continuar? (S/N)'
        Yes              = '^[sSyY]'
        Cancelled        = 'Cancelado.'
        OverlayFailed    = 'No se pudo instalar la versión de superposición (los detalles están arriba).'
        OverlayInstalled = '{0} (superposición máxima) quedó instalado en:'
        SwitchingToNormal = 'Cambiando de la versión de superposición a la normal (Windows pedirá permiso)...'
        SwitchFailed     = 'No se pudo quitar {0}. Puedes hacerlo luego con Uninstall.cmd.'
        Installed        = '{0} quedó instalado en:'
        ShortcutsBoth    = 'Accesos directos: menú Inicio y escritorio'
        ShortcutsStart   = 'Accesos directos: menú Inicio'
        OverlayHint      = 'Para que quede encima de apps a pantalla completa, usa Install-overlay.cmd.'
    }
    ru = @{
        Closing          = 'Закрываем YT Mini Player...'
        ShortcutInfo     = 'Плавающий плеер для YouTube и других сервисов'
        NeedAdmin        = 'Нужны права администратора, а запрос Windows был отменён.'
        NoSums           = 'Нет файла SHA256SUMS.txt, поэтому проверить целостность файлов нельзя.'
        NoSumFor         = 'В SHA256SUMS.txt нет отпечатка для {0}.'
        SumMismatch      = '{0} не совпадает с SHA256SUMS.txt: загрузка повреждена или изменена. Скачайте её заново.'
        SumOk            = '  SHA-256 {0}: {1} (совпадает)'
        NotFound         = 'Рядом с этим сценарием не найден {0}. Возможно, антивирус поместил его в карантин: см. ANTIVIRUS.txt.'
        AlreadySigned    = 'Программа подписана издателем {0}: локальный сертификат не нужен.'
        OverlaySignedLine = '  - программа уже подписана издателем {1}, поэтому сертификат не создаётся'
        CreatingCert     = 'Создаём локальный сертификат для подписи...'
        Signing          = 'Подписываем программу...'
        BadSignature     = 'Подпись программы недействительна: {0}'
        InstalledSigned  = 'Установлено и подписано в {0}'
        OverlayRemoved   = 'Версия с наложением и её сертификат удалены.'
        MustBeAdmin      = 'Эту часть нужно запускать от имени администратора.'
        Error            = 'ОШИБКА: {0}'
        RemovingOverlay  = 'Удаляем версию с наложением (Windows запросит разрешение)...'
        RemoveOverlayFailed = 'Не удалось удалить {0} и сертификат. Запустите Uninstall.cmd ещё раз.'
        DataRemoved      = 'Сеанс, настройки и расширения удалены.'
        DataKept         = 'Сеанс и настройки сохранены в {0}'
        Uninstalled      = '{0} удалён.'
        OverlayIntro     = @(
            'Максимальное наложение: плеер сможет оставаться поверх полноэкранных',
            'приложений и игр (кроме работающих в ЭКСКЛЮЗИВНОМ полноэкранном режиме).',
            '',
            'Для этого, с правами администратора один раз:',
            '  - программа устанавливается в {0}',
            '  - создаётся локальный сертификат "YT Mini Player (local signing)", которому',
            '    доверяет этот компьютер; программа подписывается, а закрытый ключ удаляется',
            '    (ничего другого этим сертификатом подписать нельзя)',
            '  - обычная установка удаляется, чтобы не было двух копий',
            'Uninstall.cmd отменяет всё это.')
        Continue         = 'Продолжить? (Д/Н)'
        Yes              = '^[дДyY]'
        Cancelled        = 'Отменено.'
        OverlayFailed    = 'Не удалось установить версию с наложением (подробности выше).'
        OverlayInstalled = '{0} (максимальное наложение) установлен в:'
        SwitchingToNormal = 'Переходим с версии с наложением на обычную (Windows запросит разрешение)...'
        SwitchFailed     = 'Не удалось удалить {0}. Это можно сделать позже с помощью Uninstall.cmd.'
        Installed        = '{0} установлен в:'
        ShortcutsBoth    = 'Ярлыки: меню «Пуск» и рабочий стол'
        ShortcutsStart   = 'Ярлыки: меню «Пуск»'
        OverlayHint      = 'Чтобы плеер оставался поверх полноэкранных приложений, запустите Install-overlay.cmd.'
    }
    pl = @{
        Closing          = 'Zamykanie YT Mini Player...'
        ShortcutInfo     = 'Pływający odtwarzacz YouTube i innych serwisów'
        NeedAdmin        = 'Potrzebne są uprawnienia administratora, a monit systemu Windows został anulowany.'
        NoSums           = 'Brak pliku SHA256SUMS.txt, więc nie można sprawdzić, czy pliki są nienaruszone.'
        NoSumFor         = 'W SHA256SUMS.txt nie ma odcisku pliku {0}.'
        SumMismatch      = 'Plik {0} nie zgadza się z SHA256SUMS.txt: pobrany plik jest uszkodzony lub został zmieniony. Pobierz go ponownie.'
        SumOk            = '  SHA-256 {0}: {1} (zgodny)'
        NotFound         = 'Nie znaleziono pliku {0} obok tego skryptu. Możliwe, że antywirus przeniósł go do kwarantanny: zobacz ANTIVIRUS.txt.'
        AlreadySigned    = 'Program jest podpisany przez {0}: lokalny certyfikat nie jest potrzebny.'
        OverlaySignedLine = '  - program jest już podpisany przez {1}, więc żaden certyfikat nie powstanie'
        CreatingCert     = 'Tworzenie lokalnego certyfikatu do podpisu...'
        Signing          = 'Podpisywanie programu...'
        BadSignature     = 'Podpis programu jest nieprawidłowy: {0}'
        InstalledSigned  = 'Zainstalowano i podpisano w {0}'
        OverlayRemoved   = 'Usunięto wersję z nakładką i jej certyfikat.'
        MustBeAdmin      = 'Tę część trzeba uruchomić jako administrator.'
        Error            = 'BŁĄD: {0}'
        RemovingOverlay  = 'Usuwanie wersji z nakładką (system Windows poprosi o zgodę)...'
        RemoveOverlayFailed = 'Nie udało się usunąć {0} ani certyfikatu. Uruchom ponownie Uninstall.cmd.'
        DataRemoved      = 'Usunięto sesję, ustawienia i rozszerzenia.'
        DataKept         = 'Sesja i ustawienia zostają w {0}'
        Uninstalled      = 'Odinstalowano {0}.'
        OverlayIntro     = @(
            'Maksymalna nakładka: odtwarzacz może być widoczny nad aplikacjami i grami',
            'na pełnym ekranie (poza tymi w WYŁĄCZNYM trybie pełnoekranowym).',
            '',
            'W tym celu, jednorazowo z uprawnieniami administratora:',
            '  - program zostanie zainstalowany w {0}',
            '  - powstanie lokalny certyfikat "YT Mini Player (local signing)", zaufany',
            '    na tym komputerze; program zostanie podpisany, a klucz prywatny usunięty',
            '    (certyfikatem nie da się podpisać niczego innego)',
            '  - zwykła instalacja zostanie usunięta, żeby nie było dwóch kopii',
            'Uninstall.cmd cofa wszystkie te zmiany.')
        Continue         = 'Kontynuować? (T/N)'
        Yes              = '^[tTyY]'
        Cancelled        = 'Anulowano.'
        OverlayFailed    = 'Nie udało się zainstalować wersji z nakładką (szczegóły powyżej).'
        OverlayInstalled = '{0} (maksymalna nakładka) zainstalowano w:'
        SwitchingToNormal = 'Przełączanie z wersji z nakładką na zwykłą (system Windows poprosi o zgodę)...'
        SwitchFailed     = 'Nie udało się usunąć {0}. Możesz to zrobić później za pomocą Uninstall.cmd.'
        Installed        = '{0} zainstalowano w:'
        ShortcutsBoth    = 'Skróty: menu Start i pulpit'
        ShortcutsStart   = 'Skróty: menu Start'
        OverlayHint      = 'Aby odtwarzacz był widoczny nad aplikacjami pełnoekranowymi, użyj Install-overlay.cmd.'
    }
    ja = @{
        Closing          = 'YT Mini Player を終了しています...'
        ShortcutInfo     = 'YouTube などのサービスを再生するフローティング プレーヤー'
        NeedAdmin        = '管理者の許可が必要ですが、Windows の確認がキャンセルされました。'
        NoSums           = 'SHA256SUMS.txt がないため、ファイルが改変されていないか確認できません。'
        NoSumFor         = 'SHA256SUMS.txt に {0} のハッシュがありません。'
        SumMismatch      = '{0} が SHA256SUMS.txt と一致しません。ダウンロードが壊れているか、改変されています。もう一度ダウンロードしてください。'
        SumOk            = '  SHA-256 {0}: {1}（一致）'
        NotFound         = 'このスクリプトと同じ場所に {0} が見つかりません。ウイルス対策ソフトが検疫した可能性があります。ANTIVIRUS.txt をご覧ください。'
        AlreadySigned    = 'プログラムは {0} によって署名済みのため、ローカル証明書は必要ありません。'
        OverlaySignedLine = '  - プログラムは {1} によって署名済みのため、証明書は作成しません'
        CreatingCert     = 'ローカル署名用の証明書を作成しています...'
        Signing          = 'プログラムに署名しています...'
        BadSignature     = 'プログラムの署名が無効です: {0}'
        InstalledSigned  = '{0} にインストールして署名しました'
        OverlayRemoved   = 'オーバーレイ版とその証明書を削除しました。'
        MustBeAdmin      = 'この処理は管理者として実行する必要があります。'
        Error            = 'エラー: {0}'
        RemovingOverlay  = 'オーバーレイ版を削除しています（Windows が許可を求めます）...'
        RemoveOverlayFailed = '{0} と証明書を削除できませんでした。Uninstall.cmd をもう一度実行してください。'
        DataRemoved      = 'セッション、設定、拡張機能を削除しました。'
        DataKept         = 'セッションと設定は {0} に残っています'
        Uninstalled      = '{0} をアンインストールしました。'
        OverlayIntro     = @(
            '最大オーバーレイ: 全画面のアプリやゲームの上にもプレーヤーを表示できます',
            '（排他的フルスクリーンのものを除く）。',
            '',
            'そのために、管理者の許可を 1 回だけ得て次のことを行います:',
            '  - {0} にインストールします',
            '  - ローカル証明書「YT Mini Player (local signing)」を作成してこの PC で信頼し、',
            '    プログラムに署名したあと秘密キーを削除します',
            '    （この証明書でほかのものに署名することはできません）',
            '  - 2 つのコピーができないよう、通常版を削除します',
            'Uninstall.cmd ですべて元に戻せます。')
        Continue         = '続行しますか? (Y/N)'
        # "はい" as a word: a class with は and い would take "いいえ" (no) for a yes.
        Yes              = '^(?:[yYｙＹ]|はい)'
        Cancelled        = 'キャンセルしました。'
        OverlayFailed    = 'オーバーレイ版をインストールできませんでした（詳細は上記のとおりです）。'
        OverlayInstalled = '{0}（最大オーバーレイ）を次の場所にインストールしました:'
        SwitchingToNormal = 'オーバーレイ版から通常版に切り替えています（Windows が許可を求めます）...'
        SwitchFailed     = '{0} を削除できませんでした。後で Uninstall.cmd から削除できます。'
        Installed        = '{0} を次の場所にインストールしました:'
        ShortcutsBoth    = 'ショートカット: スタート メニューとデスクトップ'
        ShortcutsStart   = 'ショートカット: スタート メニュー'
        OverlayHint      = '全画面アプリの上にも表示するには、Install-overlay.cmd を使ってください。'
    }
}

$Lang = (Get-UICulture).TwoLetterISOLanguageName
if (-not $Messages.ContainsKey($Lang)) { $Lang = 'en' }

# A message in the Windows display language, with its arguments.
function T([string]$Key) {
    $text = $Messages[$Lang][$Key]
    if ($null -eq $text) { $text = $Messages['en'][$Key] }
    if ($args.Count -gt 0) { return ($text -f $args) }
    return $text
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# The elevated part writes its log through one handle, on a file it creates
# itself (CreateNew fails if anything, a link included, is already there).
# The log is in the user's Temp folder: opening the path again for every
# line would let another program swap it for a link between two lines and
# have this administrator process write somewhere else.
$script:LogWriter = $null
if ($LogPath) {
    try {
        $stream = [IO.FileStream]::new($LogPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
        $script:LogWriter = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
        $script:LogWriter.AutoFlush = $true
    } catch {
        $script:LogWriter = $null
    }
}

# A message for the user; in the elevated part it also goes to the log the
# original window shows afterwards.
function Say([string]$Text) {
    Write-Host $Text
    if ($script:LogWriter) { $script:LogWriter.WriteLine($Text) }
}

function Test-Admin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    ([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Stop-App {
    $running = @(Get-Process -Name 'yt-mini-player' -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        Write-Host (T 'Closing')
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
    $link.Description = T 'ShortcutInfo'
    $link.Save()
}

function Remove-IfExists([string[]]$Paths) {
    foreach ($path in $Paths) {
        if (Test-Path $path) { Remove-Item $path -Recurse -Force }
    }
}

# Starting with Windows: a UIAccess .exe only starts through ShellExecute,
# so it is launched through shell32 (as the app itself does).
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

# Runs this same script as administrator and returns its exit code. What it
# writes is kept in a log and shown here.
function Invoke-Elevated([string]$Mode) {
    $log = Join-Path $env:TEMP ('ytmp-install-' + [guid]::NewGuid().ToString('N') + '.log')
    $arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -$Mode -LogPath `"$log`""
    if ($NoDesktop) { $arguments += ' -NoDesktop' }
    try {
        $process = Start-Process -FilePath 'powershell.exe' -Verb RunAs -ArgumentList $arguments -Wait -PassThru
    } catch {
        throw (T 'NeedAdmin')
    }
    if (Test-Path $log) {
        Get-Content $log -Encoding UTF8 | ForEach-Object { Write-Host "  $_" }
        Remove-Item $log -Force -ErrorAction SilentlyContinue
    }
    return $process.ExitCode
}

function Test-OverlayCertificate($Certificate) {
    return ($Certificate.Subject -eq $CertSubject) -or ($LegacyCertSubjects -contains $Certificate.Subject)
}

function Remove-OverlayCertificates {
    foreach ($store in 'Root', 'TrustedPublisher', 'My') {
        Get-ChildItem "Cert:\LocalMachine\$store" |
            Where-Object { Test-OverlayCertificate $_ } |
            ForEach-Object {
                if ($store -eq 'My') { Remove-Item $_.PSPath -DeleteKey -Force }
                else { Remove-Item $_.PSPath -Force }
            }
    }
}

# ---------------------------------------------------------------------------
# Integrity: SHA-256 fingerprints published with the release
# ---------------------------------------------------------------------------

# Checks `$Path` against `$Name`'s line in SHA256SUMS.txt (sha256sum format:
# "<hash>  <name>"). Without the fingerprint file it only warns.
# -Required: the overlay copy is signed and trusted by Windows (UIAccess),
# so it is never installed unchecked.
function Test-Checksum([string]$Path, [string]$Name, [switch]$Required) {
    $sums = Join-Path $PSScriptRoot 'SHA256SUMS.txt'
    if (-not (Test-Path $sums)) {
        if ($Required) { throw (T 'NoSums') }
        Write-Warning (T 'NoSums')
        return
    }
    $line = Get-Content $sums | Where-Object { $_ -match ('^([0-9a-fA-F]{64})\s+\*?' + [regex]::Escape($Name) + '$') } | Select-Object -First 1
    if (-not $line) { throw (T 'NoSumFor' $Name) }
    $expected = ($line -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw (T 'SumMismatch' $Name)
    }
    Write-Host (T 'SumOk' $Name $actual)
}

# A valid signature by a publisher (a certificate Windows already trusts,
# not this installer's local one): the signer's name, or $null.
function Get-Publisher([string]$Path) {
    $signature = Get-AuthenticodeSignature -FilePath $Path
    if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate) { return $null }
    $subject = $signature.SignerCertificate.Subject
    if ($subject -eq $CertSubject -or $LegacyCertSubjects -contains $subject) { return $null }
    return $signature.SignerCertificate.GetNameInfo([Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false)
}

# ---------------------------------------------------------------------------
# Parts that need an administrator
# ---------------------------------------------------------------------------

function Install-Machine {
    $source = Join-Path $PSScriptRoot $UiaExeName
    if (-not (Test-Path $source)) { throw (T 'NotFound' $UiaExeName) }

    Stop-App
    New-Item -ItemType Directory -Force -Path $MachineDir | Out-Null
    Copy-Item $source $MachineExe -Force
    $loader = Join-Path $PSScriptRoot 'WebView2Loader.dll'
    if (Test-Path $loader) { Copy-Item $loader $MachineDir -Force }
    Get-ChildItem $MachineDir | Unblock-File -ErrorAction SilentlyContinue

    try {
        # The COPY is checked, already in Program Files: nobody without
        # administrator rights can change it between the check and the
        # signature.
        Test-Checksum $MachineExe $UiaExeName -Required
        $machineLoader = Join-Path $MachineDir 'WebView2Loader.dll'
        if (Test-Path $machineLoader) { Test-Checksum $machineLoader 'WebView2Loader.dll' -Required }

        Remove-OverlayCertificates
        # Signed by the publisher (an official release): Windows already
        # trusts it, so no local certificate is created.
        $publisher = Get-Publisher $MachineExe
        if ($publisher) {
            Say (T 'AlreadySigned' $publisher)
        } else {
            Install-LocalSignature
        }
    } catch {
        Remove-OverlayCertificates
        Remove-IfExists @($MachineDir)
        throw
    }

    New-Shortcut $CommonStartLink $MachineExe
    if (-not $NoDesktop) { New-Shortcut $CommonDesktopLink $MachineExe }
    Say (T 'InstalledSigned' $MachineDir)
}

# Signs the installed copy with a certificate made for it alone, then
# destroys the private key.
function Install-LocalSignature {
    Say (T 'CreatingCert')
    $certArgs = @{
        Type              = 'CodeSigningCert'
        Subject           = $CertSubject
        FriendlyName      = 'YT Mini Player (local signing)'
        CertStoreLocation = 'Cert:\LocalMachine\My'
        KeyAlgorithm      = 'RSA'
        KeyLength         = 3072
        HashAlgorithm     = 'SHA256'
        KeyExportPolicy   = 'NonExportable'
        NotAfter          = (Get-Date).AddYears(10)
    }
    try {
        # Basic constraints: not an authority (it cannot issue other
        # certificates). Usage: code signing only (CodeSigningCert sets it).
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
        Say (T 'Signing')
        Set-AuthenticodeSignature -FilePath $MachineExe -Certificate $cert -HashAlgorithm SHA256 | Out-Null
    } finally {
        # The private key is always destroyed, whether it went well or
        # not: the certificate is left only as the "seal" of this .exe.
        Remove-Item -Path ('Cert:\LocalMachine\My\' + $cert.Thumbprint) -DeleteKey -Force
    }
    $signature = Get-AuthenticodeSignature -FilePath $MachineExe
    if ($signature.Status -ne 'Valid') {
        throw (T 'BadSignature' $signature.StatusMessage)
    }
}

function Uninstall-Machine {
    Stop-App
    Remove-IfExists @($CommonStartLink, $CommonDesktopLink, $MachineDir)
    Remove-OverlayCertificates
    Say (T 'OverlayRemoved')
}

if ($MachineInstall -or $MachineUninstall) {
    if (-not (Test-Admin)) { Write-Error (T 'MustBeAdmin'); exit 1 }
    $code = 0
    try {
        if ($MachineInstall) { Install-Machine } else { Uninstall-Machine }
    } catch {
        Say (T 'Error' $_.Exception.Message)
        $code = 1
    }
    exit $code
}

# ---------------------------------------------------------------------------
# Uninstall
# ---------------------------------------------------------------------------

if ($Uninstall) {
    Stop-App
    Remove-IfExists @($UserStartLink, $UserDesktopLink, $UserDir)
    if (Get-ItemProperty -Path $RunKey -Name $AppName -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -Path $RunKey -Name $AppName
    }
    $hasCert = @(Get-ChildItem 'Cert:\LocalMachine\Root' | Where-Object { Test-OverlayCertificate $_ }).Count -gt 0
    if ((Test-Path $MachineDir) -or $hasCert) {
        Write-Host (T 'RemovingOverlay')
        if ((Invoke-Elevated 'MachineUninstall') -ne 0) {
            Write-Warning (T 'RemoveOverlayFailed' $MachineDir)
        }
    }
    if ($RemoveData -and (Test-Path $DataDir)) {
        Remove-Item $DataDir -Recurse -Force
        Write-Host (T 'DataRemoved')
    } elseif (Test-Path $DataDir) {
        Write-Host (T 'DataKept' $DataDir)
    }
    Write-Host (T 'Uninstalled' $AppName)
    exit 0
}

# ---------------------------------------------------------------------------
# Install in maximum overlay mode
# ---------------------------------------------------------------------------

if ($Overlay) {
    if (-not $Yes) {
        Write-Host ''
        $intro = @(T 'OverlayIntro')
        $uia = Join-Path $PSScriptRoot $UiaExeName
        $publisher = if (Test-Path $uia) { Get-Publisher $uia } else { $null }
        if ($publisher) {
            # A signed package: no certificate (lines 6 to 8 say otherwise).
            $intro = @($intro[0..4]) + @(T 'OverlaySignedLine') + @($intro[8..($intro.Count - 1)])
        }
        foreach ($line in $intro) { Write-Host ($line -f $MachineDir, $publisher) }
        Write-Host ''
        $answer = Read-Host (T 'Continue')
        if ($answer -notmatch (T 'Yes')) { Write-Host (T 'Cancelled'); exit 0 }
    }
    Stop-App
    if ((Invoke-Elevated 'MachineInstall') -ne 0) {
        throw (T 'OverlayFailed')
    }
    # No two copies: this user's normal install is removed.
    Remove-IfExists @($UserStartLink, $UserDesktopLink, $UserDir)
    Update-Autostart $MachineExe $true
    Write-Host ''
    Write-Host (T 'OverlayInstalled' $AppName)
    Write-Host "  $MachineDir"
    Write-Host ''
    # Opened as a normal user: this process is not an administrator.
    if (-not $NoLaunch) { Start-Process $MachineExe }
    exit 0
}

# ---------------------------------------------------------------------------
# Normal install
# ---------------------------------------------------------------------------

$source = Join-Path $PSScriptRoot $ExeName
if (-not (Test-Path $source)) {
    throw (T 'NotFound' $ExeName)
}

Stop-App
if (Test-Path $MachineDir) {
    Write-Host (T 'SwitchingToNormal')
    if ((Invoke-Elevated 'MachineUninstall') -ne 0) {
        Write-Warning (T 'SwitchFailed' $MachineDir)
    }
}

Test-Checksum $source $ExeName
$loader = Join-Path $PSScriptRoot 'WebView2Loader.dll'
if (Test-Path $loader) { Test-Checksum $loader 'WebView2Loader.dll' }

New-Item -ItemType Directory -Force -Path $UserDir | Out-Null
Copy-Item $source $UserDir -Force
if (Test-Path $loader) { Copy-Item $loader $UserDir -Force }

$target = Join-Path $UserDir $ExeName
# Removes the "downloaded from the Internet" mark from the installed copy.
Get-ChildItem $UserDir | Unblock-File -ErrorAction SilentlyContinue

New-Shortcut $UserStartLink $target
if (-not $NoDesktop) { New-Shortcut $UserDesktopLink $target }

# If starting with Windows was on, point it to the new location.
Update-Autostart $target $false

Write-Host ''
Write-Host (T 'Installed' $AppName)
Write-Host "  $UserDir"
Write-Host $(if ($NoDesktop) { T 'ShortcutsStart' } else { T 'ShortcutsBoth' })
Write-Host (T 'OverlayHint')
Write-Host ''

if (-not $NoLaunch) { Start-Process $target }
