<#
.SYNOPSIS
    Tests of the PowerShell side: install.ps1's messages and helpers, the
    refusals of check-release-exe.ps1 and sign.ps1, measure.ps1's process
    tree, the launchers and ANTIVIRUS.txt.

.DESCRIPTION
    Nothing is installed, signed or changed on this PC: install.ps1 is only
    parsed (its message table and a few pure functions are taken from the
    syntax tree), the other scripts run against files in a temporary folder
    that is deleted at the end. Works in Windows PowerShell 5.1 and in
    PowerShell 7.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File tests\scripts.tests.ps1
.EXAMPLE
    pwsh -NoProfile -File tests/scripts.tests.ps1 -Filter checksum
#>
[CmdletBinding()]
param([string]$Filter)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$Root = Split-Path $PSScriptRoot -Parent
$Temp = Join-Path ([IO.Path]::GetTempPath()) ('ytmp-script-tests-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Temp | Out-Null
$script:Results = New-Object 'System.Collections.Generic.List[object]'

# ---------------------------------------------------------------------------
# A very small runner
# ---------------------------------------------------------------------------

# (Its own variables have unusual names: a test body sees them, PowerShell
# scopes being dynamic, and must not mistake them for the script's.)
function It([string]$__TestName, [scriptblock]$__TestBody) {
    if ($Filter -and $__TestName -notlike "*$Filter*") { return }
    $__watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        & $__TestBody | Out-Null
        $script:Results.Add([pscustomobject]@{ Name = $__TestName; Ok = $true; Detail = ''; Ms = $__watch.ElapsedMilliseconds })
    } catch {
        $script:Results.Add([pscustomobject]@{ Name = $__TestName; Ok = $false; Detail = $_.Exception.Message; Ms = $__watch.ElapsedMilliseconds })
    }
}

function Expect([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

# The message of the error a block throws, or $null if it does not throw.
function Get-Thrown([scriptblock]$__Block) {
    try { & $__Block *>&1 | Out-Null; return $null } catch { return $_.Exception.Message }
}

function New-TempFile([string]$Name, [byte[]]$Bytes) {
    $path = Join-Path $Temp $Name
    $folder = Split-Path $path -Parent
    if (-not (Test-Path -LiteralPath $folder)) { New-Item -ItemType Directory -Path $folder | Out-Null }
    [IO.File]::WriteAllBytes($path, $Bytes)
    return $path
}

function Ascii([string]$Text) { return [Text.Encoding]::ASCII.GetBytes($Text) }

# Runs a block with some environment variables set ($null removes one) and
# puts them back afterwards.
function Invoke-WithEnv([hashtable]$__Vars, [scriptblock]$__Block) {
    $__saved = @{}
    foreach ($__var in $__Vars.Keys) {
        $__saved[$__var] = [Environment]::GetEnvironmentVariable($__var, 'Process')
        [Environment]::SetEnvironmentVariable($__var, $__Vars[$__var], 'Process')
    }
    try { & $__Block } finally {
        foreach ($__var in $__saved.Keys) { [Environment]::SetEnvironmentVariable($__var, $__saved[$__var], 'Process') }
    }
}

# ---------------------------------------------------------------------------
# install.ps1, from its syntax tree (never run)
# ---------------------------------------------------------------------------

$InstallPath = Join-Path $Root 'dist\install.ps1'
$tokens = $null
$parseErrors = $null
$InstallAst = [Management.Automation.Language.Parser]::ParseFile($InstallPath, [ref]$tokens, [ref]$parseErrors)
$InstallText = [IO.File]::ReadAllText($InstallPath)

function Get-AssignmentText([string]$Variable) {
    $found = $InstallAst.Find({
        param($node)
        $node -is [Management.Automation.Language.AssignmentStatementAst] -and $node.Left.Extent.Text -eq ('$' + $Variable)
    }, $true)
    if (-not $found) { throw "install.ps1 no longer sets `$$Variable" }
    return $found.Extent.Text
}

function Get-FunctionText([string]$Name) {
    $found = $InstallAst.Find({
        param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $Name
    }, $true)
    if (-not $found) { throw "install.ps1 no longer has the function $Name" }
    return $found.Extent.Text
}

# The data and the pure helpers, defined here (script scope).
Invoke-Expression (Get-AssignmentText 'Messages')
Invoke-Expression (Get-AssignmentText 'CertSubject')
Invoke-Expression (Get-AssignmentText 'LegacyCertSubjects')
$Lang = 'en'
foreach ($helper in 'T', 'Get-AutostartCommand', 'Test-OverlayCertificate') {
    Invoke-Expression (Get-FunctionText $helper)
}
# Test-Checksum looks for SHA256SUMS.txt next to the script: here, in $SumsDir.
$SumsDir = $Temp
Invoke-Expression ((Get-FunctionText 'Test-Checksum') -replace '\$PSScriptRoot', '$script:SumsDir')

$Languages = @('en', 'es', 'ru', 'pl', 'ja')
$EnglishKeys = @($Messages['en'].Keys | Sort-Object)

function Get-Placeholders($Value) {
    $found = @()
    foreach ($line in @($Value)) { $found += @([regex]::Matches([string]$line, '\{(\d+)\}') | ForEach-Object { $_.Groups[1].Value }) }
    return @($found | Sort-Object -Unique)
}

# ---------------------------------------------------------------------------
# Every script parses, and is saved so Windows PowerShell reads it right
# ---------------------------------------------------------------------------

$Scripts = @('dist\install.ps1', 'tools\check-release-exe.ps1', 'tools\sign.ps1', 'tools\package.ps1',
             'tools\measure.ps1', 'tests\measure.test.ps1', 'tests\scripts.tests.ps1')
foreach ($relative in $Scripts) {
    It "$relative parses without errors" {
        $t = $null; $e = $null
        [Management.Automation.Language.Parser]::ParseFile((Join-Path $Root $relative), [ref]$t, [ref]$e) | Out-Null
        Expect ($e.Count -eq 0) (($e | ForEach-Object { $_.Message }) -join '; ')
    }
    It "$relative has a BOM if it has any non-ASCII character (5.1 reads it as UTF-8)" {
        $bytes = [IO.File]::ReadAllBytes((Join-Path $Root $relative))
        $bom = $bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF
        $wide = [Text.Encoding]::UTF8.GetString($bytes) -match '[^\x00-\x7F]'
        Expect ($bom -or -not $wide) 'non-ASCII text without a BOM'
    }
}

It 'install.ps1 parses with no errors in its own tree' {
    Expect ($parseErrors.Count -eq 0) (($parseErrors | ForEach-Object { $_.Message }) -join '; ')
}

It 'install.ps1 keeps its switches' {
    $names = @($InstallAst.ParamBlock.Parameters | ForEach-Object { $_.Name.VariablePath.UserPath })
    foreach ($expected in 'Overlay', 'Uninstall', 'RemoveData', 'NoDesktop', 'NoLaunch', 'Yes', 'MachineInstall',
                          'MachineUninstall', 'LogPath') {
        Expect ($names -contains $expected) "no -$expected"
    }
}

It 'install.ps1 stops on errors and runs in strict mode' {
    Expect ($InstallText -match "\`$ErrorActionPreference = 'Stop'") 'no Stop'
    Expect ($InstallText -match 'Set-StrictMode -Version 2\.0') 'no strict mode'
}

# ---------------------------------------------------------------------------
# The messages, in five languages
# ---------------------------------------------------------------------------

It 'install.ps1 has messages in the five languages' {
    $have = @($Messages.Keys | Sort-Object)
    Expect (($have -join ',') -eq ((@($Languages) | Sort-Object) -join ',')) ($have -join ',')
}

foreach ($code in $Languages) {
    It "messages in $code have exactly the English keys" {
        $keys = @($Messages[$code].Keys | Sort-Object)
        $missing = @($EnglishKeys | Where-Object { $keys -notcontains $_ })
        $extra = @($keys | Where-Object { $EnglishKeys -notcontains $_ })
        Expect ($missing.Count -eq 0 -and $extra.Count -eq 0) ("missing: $($missing -join ', '); extra: $($extra -join ', ')")
    }
}

foreach ($code in $Languages) {
    foreach ($key in $EnglishKeys) {
        It "message $key in ${code}: formats with its arguments, same placeholders as English" {
            $value = $Messages[$code][$key]
            Expect ($null -ne $value) 'missing'
            foreach ($line in @($value)) {
                $formatted = [string]$line -f 'ARG0', 'ARG1', 'ARG2'
                Expect ($formatted -notmatch '\{\d+\}') "left unformatted: $formatted"
            }
            $mine = (Get-Placeholders $value) -join ','
            $english = (Get-Placeholders $Messages['en'][$key]) -join ','
            Expect ($mine -eq $english) "placeholders {$mine} vs English {$english}"
            foreach ($line in @($value)) {
                Expect ([string]$line -eq ([string]$line).TrimEnd()) 'spaces at the end of a line'
            }
        }
    }
}

foreach ($code in $Languages) {
    It "the overlay intro in $code has 10 lines, the certificate named on line 6" {
        $intro = @($Messages[$code]['OverlayIntro'])
        Expect ($intro.Count -eq 10) "$($intro.Count) lines"
        Expect ($intro[5] -like '*YT Mini Player (local signing)*') $intro[5]
        Expect ($intro[2] -eq '') 'line 3 is not the blank one'
    }
    It "the overlay intro in $code mentions Uninstall.cmd on its last line" {
        $intro = @($Messages[$code]['OverlayIntro'])
        Expect ($intro[9] -like '*Uninstall.cmd*') $intro[9]
    }
    It "messages in $code that point at a file name it as is" {
        foreach ($pair in @(@('NotFound', 'ANTIVIRUS.txt'), @('NoSums', 'SHA256SUMS.txt'), @('OverlayHint', 'Install-overlay.cmd'),
                            @('SwitchFailed', 'Uninstall.cmd'), @('RemoveOverlayFailed', 'Uninstall.cmd'))) {
            Expect ($Messages[$code][$pair[0]] -like "*$($pair[1])*") "$($pair[0]) does not name $($pair[1])"
        }
    }
}

# "Continue? (Y/N)": what each language takes for a yes.
$D = [string][char]0x0434        # d (Cyrillic)
$DU = [string][char]0x0414
$NETT = "$([char]0x043D)$([char]0x0435)$([char]0x0442)"    # net
$DA = "$([char]0x0434)$([char]0x0430)"                     # da
$HAI = "$([char]0x306F)$([char]0x3044)"                    # hai
$IIE = "$([char]0x3044)$([char]0x3044)$([char]0x3048)"     # iie
$FULLY = [string][char]0xFF59                              # full-width y
$FULLYU = [string][char]0xFF39
$YesCases = @(
    @('en', 'y', $true), @('en', 'Y', $true), @('en', 'yes', $true), @('en', 'n', $false), @('en', 'no', $false), @('en', '', $false),
    @('es', 's', $true), @('es', 'S', $true), @('es', 'si', $true), @('es', 'y', $true), @('es', 'n', $false), @('es', 'no', $false),
    @('ru', $D, $true), @('ru', $DU, $true), @('ru', $DA, $true), @('ru', 'y', $true), @('ru', $NETT, $false), @('ru', 'n', $false),
    @('pl', 't', $true), @('pl', 'T', $true), @('pl', 'tak', $true), @('pl', 'y', $true), @('pl', 'nie', $false), @('pl', 'n', $false),
    @('ja', 'y', $true), @('ja', $FULLY, $true), @('ja', $FULLYU, $true), @('ja', $HAI, $true), @('ja', $IIE, $false), @('ja', 'n', $false)
)
$caseNumber = 0
foreach ($case in $YesCases) {
    $caseNumber++
    $code = $case[0]; $answer = $case[1]; $yes = $case[2]
    It "yes/no in ${code}: answer #$caseNumber is taken as $(if ($yes) { 'yes' } else { 'no' })" {
        $taken = $answer -match $Messages[$code]['Yes']
        Expect ($taken -eq $yes) "'$answer' -> $taken"
    }
}

# ---------------------------------------------------------------------------
# T: a message in the display language, with its arguments
# ---------------------------------------------------------------------------

# ($Lang, the variable T reads, is set in each test's own scope.)
foreach ($code in $Languages) {
    It "T gives the $code text" {
        $Lang = $code
        Expect ((T 'Cancelled') -eq $Messages[$code]['Cancelled']) (T 'Cancelled')
    }
    It "T fills the arguments in $code" {
        $Lang = $code
        $text = T 'Uninstalled' 'YT Mini Player'
        Expect ($text -eq ($Messages[$code]['Uninstalled'] -f 'YT Mini Player')) $text
        Expect ($text -like '*YT Mini Player*') $text
    }
    It "T fills two arguments in $code (SumOk)" {
        $Lang = $code
        $text = T 'SumOk' 'a.exe' 'abc123'
        Expect ($text -like '*a.exe*' -and $text -like '*abc123*') $text
    }
    It "T returns the whole overlay intro in $code" {
        $Lang = $code
        $intro = T 'OverlayIntro'
        Expect (@($intro).Count -eq 10) 'not 10 lines'
    }
}

It 'T falls back to English for a key a language lacks' {
    $Messages = @{ en = @{ Only = 'english {0}' }; xx = @{} }
    $Lang = 'xx'
    Expect ((T 'Only' 'A') -eq 'english A') (T 'Only' 'A')
}

It 'T without arguments leaves the braces alone' {
    $Messages = @{ en = @{ Raw = 'keep {0} as is' } }
    $Lang = 'en'
    Expect ((T 'Raw') -eq 'keep {0} as is') (T 'Raw')
}

# ---------------------------------------------------------------------------
# Starting with Windows
# ---------------------------------------------------------------------------

It 'autostart: the normal install starts the program itself, hidden' {
    $command = Get-AutostartCommand 'C:\Users\Ana\AppData\Local\Programs\YT Mini Player\yt-mini-player.exe' $false
    Expect ($command -eq '"C:\Users\Ana\AppData\Local\Programs\YT Mini Player\yt-mini-player.exe" --hidden') $command
}

It 'autostart: the overlay install starts through shell32 (UIAccess needs ShellExecute)' {
    $target = 'C:\Program Files\YT Mini Player\yt-mini-player.exe'
    $command = Get-AutostartCommand $target $true
    $rundll = Join-Path $env:SystemRoot 'System32\rundll32.exe'
    Expect ($command -eq "`"$rundll`" shell32.dll,ShellExec_RunDLL `"$target`" --hidden") $command
}

It 'autostart: paths with spaces stay quoted' {
    $command = Get-AutostartCommand 'C:\A B\C D\x.exe' $false
    Expect ($command.StartsWith('"C:\A B\C D\x.exe"')) $command
}

# ---------------------------------------------------------------------------
# The overlay certificate: only this installer's own
# ---------------------------------------------------------------------------

$CertCases = @(
    @('CN=YT Mini Player (local signing)', $true),
    @('CN=YT Mini Player (firma local)', $true),
    @('CN=YT Mini Player (local signing) ', $false),
    @('CN=YT Mini Player', $false),
    @('CN=Microsoft Windows', $false),
    @('cn=yt mini player (local signing)x', $false)
)
foreach ($case in $CertCases) {
    $subject = $case[0]; $mine = $case[1]
    It "certificate '$subject' is $(if ($mine) { '' } else { 'not ' })the installer's" {
        $isMine = Test-OverlayCertificate ([pscustomobject]@{ Subject = $subject })
        Expect ($isMine -eq $mine) "$isMine"
    }
}

# ---------------------------------------------------------------------------
# Fingerprints (SHA256SUMS.txt)
# ---------------------------------------------------------------------------

function Get-Sha256([string]$Path) { return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant() }

function New-SumsCase([string]$Name, $SumsText) {
    $folder = Join-Path $Temp ('sums-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $folder | Out-Null
    $file = Join-Path $folder $Name
    [IO.File]::WriteAllBytes($file, (Ascii 'the program'))
    $hash = Get-Sha256 $file
    if ($SumsText) {
        [IO.File]::WriteAllText((Join-Path $folder 'SHA256SUMS.txt'), ($SumsText -replace '\{hash\}', $hash))
    }
    return @{ Folder = $folder; File = $file; Hash = $hash }
}

$SumsCases = @(
    @('a matching line passes', 'yt-mini-player.exe', "{hash}  yt-mini-player.exe`n", $null),
    @('an upper-case fingerprint passes', 'yt-mini-player.exe', "UPPER  yt-mini-player.exe`n", $null),
    @('the binary marker (*name) passes', 'yt-mini-player.exe', "{hash} *yt-mini-player.exe`n", $null),
    @('Windows line ends pass', 'yt-mini-player.exe', "{hash}  yt-mini-player.exe`r`n", $null),
    @('the right line among others passes', 'yt-mini-player-uia.exe',
      "0000000000000000000000000000000000000000000000000000000000000000  yt-mini-player.exe`n{hash}  yt-mini-player-uia.exe`n", $null),
    @('a wrong fingerprint stops', 'yt-mini-player.exe',
      "1111111111111111111111111111111111111111111111111111111111111111  yt-mini-player.exe`n", 'SumMismatch'),
    @('a missing line stops', 'yt-mini-player.exe', "{hash}  other.exe`n", 'NoSumFor'),
    @('a longer name does not count', 'yt-mini-player.exe', "{hash}  yt-mini-player.exe.bak`n", 'NoSumFor'),
    @('a dot in the name is not a wildcard', 'a.b.exe', "{hash}  aXb.exe`n", 'NoSumFor'),
    @('a short fingerprint does not count', 'yt-mini-player.exe', "abc  yt-mini-player.exe`n", 'NoSumFor'),
    @('a name with brackets and plus signs is matched literally', 'a+b (1).exe', "{hash}  a+b (1).exe`n", $null)
)
foreach ($case in $SumsCases) {
    $title = $case[0]; $fileName = $case[1]; $sums = $case[2]; $expectedError = $case[3]
    It "checksum: $title" {
        $Lang = 'en'
        $c = New-SumsCase $fileName $sums
        if ($sums -like 'UPPER*') {
            [IO.File]::WriteAllText((Join-Path $c.Folder 'SHA256SUMS.txt'), ($c.Hash.ToUpperInvariant() + "  $fileName`n"))
        }
        $script:SumsDir = $c.Folder
        $thrown = Get-Thrown { Test-Checksum $c.File $fileName }
        if ($expectedError) {
            Expect ($thrown -eq (T $expectedError $fileName)) "threw: $thrown"
        } else {
            Expect ($null -eq $thrown) "threw: $thrown"
        }
    }
}

It 'checksum: without SHA256SUMS.txt it only warns' {
    $Lang = 'en'
    $c = New-SumsCase 'yt-mini-player.exe' $null
    $script:SumsDir = $c.Folder
    $thrown = Get-Thrown { Test-Checksum $c.File 'yt-mini-player.exe' 3>$null }
    Expect ($null -eq $thrown) "threw: $thrown"
    $warnings = @(Test-Checksum $c.File 'yt-mini-player.exe' 3>&1 | Where-Object { $_ -is [Management.Automation.WarningRecord] })
    Expect ($warnings.Count -eq 1 -and $warnings[0].Message -eq (T 'NoSums')) 'no warning'
}

It 'checksum: a changed byte is caught' {
    $Lang = 'en'
    $c = New-SumsCase 'yt-mini-player.exe' "{hash}  yt-mini-player.exe`n"
    [IO.File]::WriteAllBytes($c.File, (Ascii 'the program!'))
    $script:SumsDir = $c.Folder
    $thrown = Get-Thrown { Test-Checksum $c.File 'yt-mini-player.exe' }
    Expect ($thrown -eq (T 'SumMismatch' 'yt-mini-player.exe')) "threw: $thrown"
}

# ---------------------------------------------------------------------------
# Every message install.ps1 uses exists
# ---------------------------------------------------------------------------

$UsedKeys = @([regex]::Matches($InstallText, "\bT '([A-Za-z]+)'") | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
It 'install.ps1 uses messages (the pattern still finds them)' {
    Expect ($UsedKeys.Count -ge 20) "$($UsedKeys.Count) found"
}
foreach ($key in $UsedKeys) {
    It "install.ps1 uses message $key and every language has it" {
        foreach ($code in $Languages) { Expect ($Messages[$code].ContainsKey($key)) "$code lacks $key" }
    }
}

# ---------------------------------------------------------------------------
# check-release-exe.ps1: the test build is refused
# ---------------------------------------------------------------------------

$CheckExe = Join-Path $Root 'tools\check-release-exe.ps1'
$Marker = 'YTMP-E2E-BUILD-NOT-FOR-RELEASE'
$ExeCases = @(
    @('a program without the marker passes', 'MZ just a program', $true),
    @('the marker at the start is caught', "$Marker and more", $false),
    @('the marker in the middle is caught', "MZ.....$Marker.....", $false),
    @('the marker at the very end is caught', "MZ.....$Marker", $false),
    @('the marker after a false start is caught', "MZ YTMP-E2E-BUILD-NOT-$Marker", $false),
    @('the marker after its own first letter is caught', "YY$Marker", $false),
    @('the marker without its last letter passes', 'MZ YTMP-E2E-BUILD-NOT-FOR-RELEAS', $true),
    @('the marker in lower case passes (bytes, not words)', 'mz ytmp-e2e-build-not-for-release', $true),
    @('a file shorter than the marker passes', 'YTMP', $true),
    @('a file made only of the first letter passes', ('Y' * 200), $true)
)
foreach ($case in $ExeCases) {
    $title = $case[0]; $content = $case[1]; $passes = $case[2]
    It "release check: $title" {
        $file = New-TempFile ('exe-' + [guid]::NewGuid().ToString('N') + '.exe') (Ascii $content)
        $thrown = Get-Thrown { & $CheckExe -Files $file }
        if ($passes) { Expect ($null -eq $thrown) "refused: $thrown" }
        else { Expect ($thrown -like '*must not be released*' -and $thrown -like "*$file*") "not refused: $thrown" }
    }
}

It 'release check: an empty file passes' {
    $file = New-TempFile 'empty.exe' ([byte[]]@())
    $thrown = Get-Thrown { & $CheckExe -Files $file }
    Expect ($null -eq $thrown) "refused: $thrown"
}

It 'release check: the marker split by a zero byte (UTF-16) is a different text' {
    $file = New-TempFile 'wide.exe' ([Text.Encoding]::Unicode.GetBytes($Marker))
    $thrown = Get-Thrown { & $CheckExe -Files $file }
    Expect ($null -eq $thrown) "refused: $thrown"
}

It 'release check: of several files, only the test build is named' {
    $good = New-TempFile 'good-1.exe' (Ascii 'MZ fine')
    $bad = New-TempFile 'bad-1.exe' (Ascii "MZ $Marker")
    $thrown = Get-Thrown { & $CheckExe -Files $good, $bad }
    Expect ($thrown -like "*$bad*" -and $thrown -notlike "*$good*") "$thrown"
}

It 'release check: two test builds are both named' {
    $a = New-TempFile 'bad-a.exe' (Ascii "$Marker")
    $b = New-TempFile 'bad-b.exe' (Ascii "xx$Marker")
    $thrown = Get-Thrown { & $CheckExe -Files $a, $b }
    Expect ($thrown -like "*$a*" -and $thrown -like "*$b*") "$thrown"
}

It 'release check: says how many files it checked' {
    $files = 1..3 | ForEach-Object { New-TempFile "count-$_.exe" (Ascii "MZ $_") }
    $said = (& $CheckExe -Files $files 6>&1 | Out-String)
    Expect ($said -like '*3 file(s) checked*') $said
}

It 'release check: a missing file stops it' {
    $thrown = Get-Thrown { & $CheckExe -Files (Join-Path $Temp 'no-such-file.exe') }
    Expect ($null -ne $thrown) 'a missing file passed'
}

It 'release check: a 4 MB program is checked in under 10 s' {
    $bytes = New-Object byte[] (4MB)
    for ($i = 0; $i -lt $bytes.Length; $i += 97) { $bytes[$i] = 89 }   # many false starts ("Y")
    $file = New-TempFile 'big.exe' $bytes
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $thrown = Get-Thrown { & $CheckExe -Files $file }
    Expect ($null -eq $thrown) "refused: $thrown"
    Expect ($watch.Elapsed.TotalSeconds -lt 10) "$($watch.Elapsed.TotalSeconds) s"
}

It 'release check: the marker is the same text as src/e2e.rs' {
    $e2e = [IO.File]::ReadAllText((Join-Path $Root 'src\e2e.rs'))
    Expect ($e2e -match ('pub const MARKER: &str = "' + [regex]::Escape($Marker) + '"')) 'e2e.rs has another marker'
    Expect (([IO.File]::ReadAllText($CheckExe)) -like "*'$Marker'*") 'the script has another marker'
}

# ---------------------------------------------------------------------------
# sign.ps1: refusals before anything is signed
# ---------------------------------------------------------------------------

$Sign = Join-Path $Root 'tools\sign.ps1'
$NoSigning = @{ YTMP_SIGN_ARTIFACT_DLIB = $null; YTMP_SIGN_ARTIFACT_METADATA = $null; YTMP_SIGN_THUMBPRINT = $null;
                YTMP_SIGNTOOL = $null; YTMP_SIGN_TIMESTAMP = $null; YTMP_SIGN_REQUIRED = $null }
$FakeSignTool = New-TempFile 'fake-signtool.exe' (Ascii 'never run')
$ToSign = New-TempFile 'to-sign.exe' (Ascii 'MZ program')

function Get-SignEnv([hashtable]$Changes) {
    $vars = @{}
    foreach ($k in $NoSigning.Keys) { $vars[$k] = $NoSigning[$k] }
    foreach ($k in $Changes.Keys) { $vars[$k] = $Changes[$k] }
    return $vars
}

It 'sign: with nothing set up it warns and leaves the files' {
    Invoke-WithEnv (Get-SignEnv @{}) {
        $warnings = @(& $Sign -Files $ToSign 3>&1 | Where-Object { $_ -is [Management.Automation.WarningRecord] })
        Expect ($warnings.Count -eq 1 -and $warnings[0].Message -like 'Not signed*') 'no warning'
    }
}

It 'sign: with nothing set up and -Required it stops' {
    Invoke-WithEnv (Get-SignEnv @{}) {
        $thrown = Get-Thrown { & $Sign -Files $ToSign -Required }
        Expect ($thrown -like 'Not signed*') "$thrown"
    }
}

foreach ($value in '1', '0', 'yes', '') {
    It "sign: YTMP_SIGN_REQUIRED='$value' with nothing set up" {
        Invoke-WithEnv (Get-SignEnv @{ YTMP_SIGN_REQUIRED = $value }) {
            $thrown = Get-Thrown { & $Sign -Files $ToSign }
            if ($value -eq '1') { Expect ($thrown -like 'Not signed*') "did not stop: $thrown" }
            else { Expect ($null -eq $thrown) "stopped: $thrown" }
        }
    }
}

It 'sign: a signtool path that does not exist stops it' {
    Invoke-WithEnv (Get-SignEnv @{ YTMP_SIGN_THUMBPRINT = ('a' * 40); YTMP_SIGNTOOL = (Join-Path $Temp 'missing-signtool.exe') }) {
        $thrown = Get-Thrown { & $Sign -Files $ToSign }
        Expect ($thrown -like 'YTMP_SIGNTOOL points to a missing file*') "$thrown"
    }
}

$BadThumbprints = @('abc', ('a' * 39), ('a' * 41), ('g' * 40), ('0123456789abcdef0123456789abcdef0123456-'))
foreach ($thumbprint in $BadThumbprints) {
    It "sign: the thumbprint '$thumbprint' is refused before signing" {
        Invoke-WithEnv (Get-SignEnv @{ YTMP_SIGN_THUMBPRINT = $thumbprint; YTMP_SIGNTOOL = $FakeSignTool }) {
            $thrown = Get-Thrown { & $Sign -Files $ToSign }
            Expect ($thrown -like "*40-character SHA-1 thumbprint*") "$thrown"
        }
    }
}

It 'sign: a file that is not there stops it before signing' {
    Invoke-WithEnv (Get-SignEnv @{ YTMP_SIGN_THUMBPRINT = ('a' * 40); YTMP_SIGNTOOL = $FakeSignTool }) {
        $missing = Join-Path $Temp 'not-built.exe'
        $thrown = Get-Thrown { & $Sign -Files $missing }
        Expect ($thrown -like "Nothing to sign at*") "$thrown"
    }
}

It 'sign: Artifact Signing with a missing dlib stops it' {
    Invoke-WithEnv (Get-SignEnv @{ YTMP_SIGN_ARTIFACT_DLIB = (Join-Path $Temp 'no.dll'); YTMP_SIGN_ARTIFACT_METADATA = $ToSign;
                                   YTMP_SIGNTOOL = $FakeSignTool }) {
        $thrown = Get-Thrown { & $Sign -Files $ToSign }
        Expect ($thrown -like "Missing signing file*no.dll*") "$thrown"
    }
}

It 'sign: Artifact Signing with missing metadata stops it' {
    Invoke-WithEnv (Get-SignEnv @{ YTMP_SIGN_ARTIFACT_DLIB = $ToSign; YTMP_SIGN_ARTIFACT_METADATA = (Join-Path $Temp 'no.json');
                                   YTMP_SIGNTOOL = $FakeSignTool }) {
        $thrown = Get-Thrown { & $Sign -Files $ToSign }
        Expect ($thrown -like "Missing signing file*no.json*") "$thrown"
    }
}

It 'sign: Artifact Signing comes before a certificate in the store' {
    Invoke-WithEnv (Get-SignEnv @{ YTMP_SIGN_ARTIFACT_DLIB = (Join-Path $Temp 'no.dll'); YTMP_SIGN_ARTIFACT_METADATA = $ToSign;
                                   YTMP_SIGN_THUMBPRINT = 'bad'; YTMP_SIGNTOOL = $FakeSignTool }) {
        $thrown = Get-Thrown { & $Sign -Files $ToSign }
        Expect ($thrown -like "Missing signing file*") "the store was chosen: $thrown"
    }
}

It 'sign: half an Artifact Signing setup is no setup' {
    Invoke-WithEnv (Get-SignEnv @{ YTMP_SIGN_ARTIFACT_DLIB = $ToSign }) {
        $warnings = @(& $Sign -Files $ToSign 3>&1 | Where-Object { $_ -is [Management.Automation.WarningRecord] })
        Expect ($warnings.Count -eq 1 -and $warnings[0].Message -like 'Not signed*') 'it tried to sign'
    }
}

It 'sign: nothing in it prints a secret (no Write-Host of the environment)' {
    $text = [IO.File]::ReadAllText($Sign)
    Expect ($text -notmatch 'Write-(Host|Output|Verbose)[^\r\n]*\$env:YTMP_SIGN_(THUMBPRINT|ARTIFACT)') 'prints a signing setting'
}

# ---------------------------------------------------------------------------
# measure.ps1: the app's process tree
# ---------------------------------------------------------------------------

. (Join-Path $Root 'tools\measure.ps1')

function P([int]$Id, [int]$Parent, [string]$Name) {
    return [pscustomobject]@{ ProcessId = $Id; ParentProcessId = $Parent; Name = $Name }
}

$TreeCases = @(
    @('nothing running', @(), 'yt-mini-player', ''),
    @('the app alone', @((P 5 1 'yt-mini-player.exe')), 'yt-mini-player', '5'),
    @('the name with .exe', @((P 5 1 'yt-mini-player.exe')), 'yt-mini-player.exe', '5'),
    @('the name in capitals', @((P 5 1 'YT-MINI-PLAYER.EXE')), 'yt-mini-player', '5'),
    @('WebView2 children and grandchildren', @((P 5 1 'yt-mini-player.exe'), (P 6 5 'msedgewebview2.exe'), (P 7 6 'msedgewebview2.exe')), 'yt-mini-player', '5,6,7'),
    @('another app''s WebView2 is left out', @((P 5 1 'yt-mini-player.exe'), (P 9 1 'msedgewebview2.exe')), 'yt-mini-player', '5'),
    @('two copies of the app', @((P 5 1 'yt-mini-player.exe'), (P 8 1 'yt-mini-player.exe'), (P 6 8 'msedgewebview2.exe')), 'yt-mini-player', '5,6,8'),
    @('a process that is its own parent', @((P 5 5 'yt-mini-player.exe')), 'yt-mini-player', '5'),
    @('a cycle between two processes', @((P 5 6 'yt-mini-player.exe'), (P 6 5 'msedgewebview2.exe')), 'yt-mini-player', '5,6'),
    @('a similar name is not the app', @((P 5 1 'yt-mini-player-old.exe')), 'yt-mini-player', '')
)
foreach ($case in $TreeCases) {
    $title = $case[0]; $table = $case[1]; $rootName = $case[2]; $expected = $case[3]
    It "process tree: $title" {
        # (It returns its array as one object: assigned first, then sorted.)
        $ids = Get-TreeIds -Processes $table -RootName $rootName
        $sorted = @($ids | Sort-Object)
        Expect (($sorted -join ',') -eq $expected) ($sorted -join ',')
    }
}

It 'process tree: a chain of 300 processes is followed to the end' {
    $table = @(P 1000 1 'yt-mini-player.exe')
    for ($i = 1; $i -lt 300; $i++) { $table += P (1000 + $i) (999 + $i) 'msedgewebview2.exe' }
    $ids = Get-TreeIds -Processes $table -RootName 'yt-mini-player'
    Expect (@($ids).Count -eq 300) "$(@($ids).Count)"
}

It 'process tree: 5000 processes in under 3 s' {
    $table = New-Object 'System.Collections.Generic.List[object]'
    $table.Add((P 10 1 'yt-mini-player.exe'))
    for ($i = 0; $i -lt 4999; $i++) { $table.Add((P (100 + $i) (1 + ($i % 50)) 'svchost.exe')) }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $ids = Get-TreeIds -Processes $table.ToArray() -RootName 'yt-mini-player'
    Expect (@($ids).Count -ge 1) 'the app was not found'
    Expect ($watch.Elapsed.TotalSeconds -lt 3) "$($watch.Elapsed.TotalSeconds) s"
}

# ---------------------------------------------------------------------------
# The launchers and ANTIVIRUS.txt
# ---------------------------------------------------------------------------

$Launchers = @(@('Install.cmd', ''), @('Install-overlay.cmd', ' -Overlay'), @('Uninstall.cmd', ' -Uninstall'))
foreach ($launcher in $Launchers) {
    $file = $launcher[0]; $extra = $launcher[1]
    It "$file runs install.ps1$extra and passes its arguments on" {
        $text = [IO.File]::ReadAllText((Join-Path $Root "dist\$file"))
        Expect ($text -like "*-File `"%~dp0install.ps1`"$extra %*") 'another command line'
        Expect ($text -like '*-ExecutionPolicy Bypass*' -and $text -like '*-NoProfile*') 'missing options'
    }
    It "$file explains a quarantined install.ps1 and fails" {
        $text = [IO.File]::ReadAllText((Join-Path $Root "dist\$file"))
        Expect ($text -like '*if not exist "%~dp0install.ps1" goto missing*') 'no check'
        Expect ($text -like '*ANTIVIRUS.txt*' -and $text -like '*exit /b 1*') 'no explanation or no failure'
    }
}

$Antivirus = [IO.File]::ReadAllText((Join-Path $Root 'dist\ANTIVIRUS.txt'))
$AntivirusParts = [regex]::Split($Antivirus, '(?m)^\[([a-z]{2})\]\r?$')
$AntivirusSections = @{}
for ($i = 1; $i + 1 -lt $AntivirusParts.Count; $i += 2) { $AntivirusSections[$AntivirusParts[$i]] = $AntivirusParts[$i + 1].Trim() }
foreach ($code in $Languages) {
    It "ANTIVIRUS.txt, read the launchers' way, has the $code section with the steps" {
        Expect ($AntivirusSections.ContainsKey($code)) 'no section'
        $section = $AntivirusSections[$code]
        foreach ($needle in 'SHA-256', 'Install.cmd', 'filesubmission') { Expect ($section -like "*$needle*") "no $needle" }
    }
}

It 'ANTIVIRUS.txt has no section the launchers would not use' {
    $unknown = @($AntivirusSections.Keys | Where-Object { $Languages -notcontains $_ })
    Expect ($unknown.Count -eq 0) ($unknown -join ', ')
}

# ---------------------------------------------------------------------------
# Results
# ---------------------------------------------------------------------------

Remove-Item -LiteralPath $Temp -Recurse -Force -ErrorAction SilentlyContinue

$failed = @($script:Results | Where-Object { -not $_.Ok })
foreach ($r in $failed) { Write-Host ("  FAIL  {0}`n        {1}" -f $r.Name, $r.Detail) }
Write-Host ("{0}/{1} passed" -f ($script:Results.Count - $failed.Count), $script:Results.Count)
if ($failed.Count -gt 0) { exit 1 }
