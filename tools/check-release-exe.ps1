<#
.SYNOPSIS
    Refuses the drivable test build: fails if any of -Files carries the
    test build's marker (src/e2e.rs, `cargo build --features e2e`).

.DESCRIPTION
    That build opens WebView2's DevTools port and runs side by side with its
    own data folder: useful for the Windows end-to-end tests, never for
    users. tools/package.ps1 runs this on the programs it packages and
    release.yml on the published ones; CI runs it on the test build to show
    that it is caught.

.EXAMPLE
    .\tools\check-release-exe.ps1 -Files target\release\yt-mini-player.exe
#>
[CmdletBinding()]
param([Parameter(Mandatory)][string[]]$Files)
$ErrorActionPreference = 'Stop'

# The same text as e2e::MARKER.
$marker = [Text.Encoding]::ASCII.GetBytes('YTMP-E2E-BUILD-NOT-FOR-RELEASE')

function Test-Contains([byte[]]$Data, [byte[]]$Needle) {
    $first = $Needle[0]
    $last = $Data.Length - $Needle.Length
    $i = [Array]::IndexOf($Data, $first, 0)
    while ($i -ge 0 -and $i -le $last) {
        $match = $true
        for ($j = 1; $j -lt $Needle.Length; $j++) {
            if ($Data[$i + $j] -ne $Needle[$j]) { $match = $false; break }
        }
        if ($match) { return $true }
        $i = [Array]::IndexOf($Data, $first, $i + 1)
    }
    return $false
}

$bad = @()
foreach ($file in $Files) {
    if (Test-Contains ([IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $file))) $marker) { $bad += $file }
}
if ($bad) {
    throw "The test build (features e2e) must not be released: $($bad -join ', ')"
}
Write-Host "Not the test build: $($Files.Count) file(s) checked."
