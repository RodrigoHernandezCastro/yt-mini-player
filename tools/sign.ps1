<#
.SYNOPSIS
    Signs files with Authenticode (optional; see docs/SIGNING.md).

.DESCRIPTION
    Uses the first signing method that is set up:

      Azure Artifact Signing (CI or a PC signed in with `az login`):
        YTMP_SIGN_ARTIFACT_DLIB      path to Azure.CodeSigning.Dlib.dll (x64)
        YTMP_SIGN_ARTIFACT_METADATA  path to metadata.json (endpoint, account,
                                     certificate profile)

      A certificate in this PC's certificate store (Certum SimplySign
      Desktop, a hardware token, an organization's certificate...):
        YTMP_SIGN_THUMBPRINT         its SHA-1 thumbprint

    Optional:
        YTMP_SIGNTOOL                signtool.exe to use (else the newest
                                     Windows SDK one)
        YTMP_SIGN_TIMESTAMP          RFC 3161 timestamp server
        YTMP_SIGN_REQUIRED=1         fail instead of leaving files unsigned

    Every signed file is verified afterwards (signtool verify /pa and
    Get-AuthenticodeSignature); a file that does not verify stops the build.
    Nothing here reads or prints secrets: Azure signs in through its own
    credentials (azure/login in CI) and certificate stores keep their keys.

.EXAMPLE
    $env:YTMP_SIGN_THUMBPRINT = '0123...'; .\tools\sign.ps1 -Files dist\x\yt-mini-player.exe
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string[]]$Files,
    [switch]$Required
)
$ErrorActionPreference = 'Stop'

$required = $Required -or $env:YTMP_SIGN_REQUIRED -eq '1'
$method = if ($env:YTMP_SIGN_ARTIFACT_DLIB -and $env:YTMP_SIGN_ARTIFACT_METADATA) { 'artifact' }
          elseif ($env:YTMP_SIGN_THUMBPRINT) { 'store' }
          else { $null }

if (-not $method) {
    $message = 'Not signed: no signing method is set up (see docs/SIGNING.md). SmartScreen and antivirus programs will treat the files as from an unknown publisher.'
    if ($required) { throw $message }
    Write-Warning $message
    return
}

function Find-SignTool {
    if ($env:YTMP_SIGNTOOL) {
        if (-not (Test-Path -LiteralPath $env:YTMP_SIGNTOOL)) { throw "YTMP_SIGNTOOL points to a missing file: $env:YTMP_SIGNTOOL" }
        return $env:YTMP_SIGNTOOL
    }
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $found = Get-ChildItem -Path $kits -Filter signtool.exe -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.Directory.Name -eq 'x64' } |
        Sort-Object { [version]($_.Directory.Parent.Name -replace '[^0-9.]', '') } -Descending |
        Select-Object -First 1
    if (-not $found) { throw 'signtool.exe was not found: install the Windows SDK (signing tools) or set YTMP_SIGNTOOL.' }
    return $found.FullName
}

$signtool = Find-SignTool
Write-Host "Signing with $signtool ($method)"

foreach ($file in $Files) {
    if (-not (Test-Path -LiteralPath $file)) { throw "Nothing to sign at $file" }
    $arguments = @('sign', '/fd', 'SHA256', '/td', 'SHA256')
    switch ($method) {
        'artifact' {
            foreach ($path in $env:YTMP_SIGN_ARTIFACT_DLIB, $env:YTMP_SIGN_ARTIFACT_METADATA) {
                if (-not (Test-Path -LiteralPath $path)) { throw "Missing signing file: $path" }
            }
            # The certificates live three days: the timestamp keeps the
            # signature valid afterwards.
            $timestamp = if ($env:YTMP_SIGN_TIMESTAMP) { $env:YTMP_SIGN_TIMESTAMP } else { 'http://timestamp.acs.microsoft.com' }
            $arguments += @('/tr', $timestamp, '/dlib', $env:YTMP_SIGN_ARTIFACT_DLIB, '/dmdf', $env:YTMP_SIGN_ARTIFACT_METADATA)
        }
        'store' {
            $thumbprint = $env:YTMP_SIGN_THUMBPRINT -replace '\s', ''
            if ($thumbprint -notmatch '^[0-9a-fA-F]{40}$') { throw 'YTMP_SIGN_THUMBPRINT must be the certificate''s 40-character SHA-1 thumbprint.' }
            $timestamp = if ($env:YTMP_SIGN_TIMESTAMP) { $env:YTMP_SIGN_TIMESTAMP } else { 'http://timestamp.digicert.com' }
            $arguments += @('/tr', $timestamp, '/sha1', $thumbprint)
        }
    }
    $arguments += @('/d', 'YT Mini Player', $file)
    & $signtool @arguments
    if ($LASTEXITCODE -ne 0) { throw "Signing failed: $file" }

    & $signtool verify /pa /q $file
    if ($LASTEXITCODE -ne 0) { throw "The signature does not verify: $file" }
    $signature = Get-AuthenticodeSignature -FilePath $file
    if ($signature.Status -ne 'Valid') { throw "The signature is not valid ($($signature.Status)): $file" }
    if (-not $signature.TimeStamperCertificate) { throw "The signature has no timestamp: $file" }
    Write-Host ("  signed: {0} by {1}" -f (Split-Path $file -Leaf), $signature.SignerCertificate.GetNameInfo('SimpleName', $false))
}
