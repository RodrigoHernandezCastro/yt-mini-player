@echo off
rem Instala YT Mini Player en modo superposicion maxima: encima de apps a pantalla completa.
rem Pide permiso de administrador una vez. Desinstalar.cmd lo deshace todo.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" -Overlay %*
pause
