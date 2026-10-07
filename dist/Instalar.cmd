@echo off
rem Instala YT Mini Player para este usuario (no requiere administrador).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
if errorlevel 1 pause
