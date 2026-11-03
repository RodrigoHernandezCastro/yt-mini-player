@echo off
rem Uninstalls YT Mini Player (both versions). Keeps the session and settings.
rem If install.ps1 is gone (an antivirus quarantined it), say why and what to do.
if not exist "%~dp0install.ps1" goto missing
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" -Uninstall %*
pause
exit /b

:missing
set "YTMP_HERE=%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -Command "$l = (Get-UICulture).TwoLetterISOLanguageName; $f = Join-Path $env:YTMP_HERE 'ANTIVIRUS.txt'; if (-not (Test-Path -LiteralPath $f)) { exit 1 }; $s = [regex]::Split([IO.File]::ReadAllText($f), '(?m)^\[([a-z]{2})\]\r?$'); $h = @{}; for ($i = 1; $i + 1 -lt $s.Count; $i += 2) { $h[$s[$i]] = $s[$i + 1].Trim() }; if (-not $h.ContainsKey($l)) { $l = 'en' }; Write-Host ''; Write-Host $h[$l]; Write-Host ''"
if errorlevel 1 echo install.ps1 is missing: your antivirus may have quarantined it. Unpack the zip again into a new folder, or restore it from Windows Security, Protection history.
pause
exit /b 1
