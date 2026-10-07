# Prueba de Get-TreeIds (tools/measure.ps1) con una tabla de procesos simulada.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../tools/measure.ps1')

$fake = @(
    [pscustomobject]@{ ProcessId = 10; ParentProcessId = 1;  Name = 'explorer.exe' },
    [pscustomobject]@{ ProcessId = 20; ParentProcessId = 10; Name = 'yt-mini-player.exe' },
    [pscustomobject]@{ ProcessId = 21; ParentProcessId = 20; Name = 'msedgewebview2.exe' },
    [pscustomobject]@{ ProcessId = 22; ParentProcessId = 21; Name = 'msedgewebview2.exe' },
    [pscustomobject]@{ ProcessId = 23; ParentProcessId = 21; Name = 'msedgewebview2.exe' },
    [pscustomobject]@{ ProcessId = 30; ParentProcessId = 10; Name = 'msedgewebview2.exe' }, # de otra app
    [pscustomobject]@{ ProcessId = 40; ParentProcessId = 40; Name = 'yt-mini-player.exe' }  # ciclo por PID reutilizado
)

$ids = Get-TreeIds -Processes $fake -RootName 'yt-mini-player'
$ids = $ids | Sort-Object
if (($ids -join ',') -ne '20,21,22,23,40') { throw "árbol incorrecto: $($ids -join ',')" }

$none = Get-TreeIds -Processes $fake -RootName 'chrome'
if ($none.Count -ne 0) { throw 'se esperaba un árbol vacío' }

'measure.ps1: todas las pruebas pasan'
