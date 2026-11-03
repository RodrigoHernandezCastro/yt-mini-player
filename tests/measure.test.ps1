# Test of Get-TreeIds (tools/measure.ps1) with a simulated process table.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../tools/measure.ps1')

$fake = @(
    [pscustomobject]@{ ProcessId = 10; ParentProcessId = 1;  Name = 'explorer.exe' },
    [pscustomobject]@{ ProcessId = 20; ParentProcessId = 10; Name = 'yt-mini-player.exe' },
    [pscustomobject]@{ ProcessId = 21; ParentProcessId = 20; Name = 'msedgewebview2.exe' },
    [pscustomobject]@{ ProcessId = 22; ParentProcessId = 21; Name = 'msedgewebview2.exe' },
    [pscustomobject]@{ ProcessId = 23; ParentProcessId = 21; Name = 'msedgewebview2.exe' },
    [pscustomobject]@{ ProcessId = 30; ParentProcessId = 10; Name = 'msedgewebview2.exe' }, # another app's
    [pscustomobject]@{ ProcessId = 40; ParentProcessId = 40; Name = 'yt-mini-player.exe' }  # cycle from a reused PID
)

$ids = Get-TreeIds -Processes $fake -RootName 'yt-mini-player'
$ids = $ids | Sort-Object
if (($ids -join ',') -ne '20,21,22,23,40') { throw "wrong tree: $($ids -join ',')" }

$none = Get-TreeIds -Processes $fake -RootName 'chrome'
if ($none.Count -ne 0) { throw 'an empty tree was expected' }

'measure.ps1: all tests pass'
