@echo off
rem Desinstala YT Mini Player. Conserva la sesion y los ajustes.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" -Uninstall %*
pause
