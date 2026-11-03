<#
.SYNOPSIS
    Measures private memory, working set and CPU of a process tree.

.DESCRIPTION
    Adds up the root process and all its descendants, working the tree out
    again on every sample (WebView2 creates and closes processes on the fly).

    - yt-mini-player: includes every msedgewebview2.exe below it.
    - A browser (msedge, chrome): includes all its processes. For a fair
      comparison, close it completely and open it with a single tab of the
      same video.

    The main metric is private memory (Private Bytes). The sum of working
    sets counts pages shared between processes several times, so it
    overestimates the real use; it is shown only for reference.

    Works with Windows PowerShell 5.1 and PowerShell 7.

.EXAMPLE
    .\tools\measure.ps1
    Measures yt-mini-player for 5 minutes, one sample every 5 seconds.

.EXAMPLE
    .\tools\measure.ps1 -Name msedge -Seconds 600 -Csv edge.csv
    Measures Edge for 10 minutes and saves the samples to edge.csv.
#>
[CmdletBinding()]
param(
    [string]$Name = 'yt-mini-player',
    [ValidateRange(10, 86400)][int]$Seconds = 300,
    [ValidateRange(1, 60)][int]$Interval = 5,
    [string]$Csv
)

Set-StrictMode -Version 2.0

# Returns the tree's PIDs: every process called $RootName plus its
# descendants. $Processes are objects with ProcessId, ParentProcessId and
# Name (like Win32_Process's).
function Get-TreeIds {
    param([object[]]$Processes, [string]$RootName)

    $exe = if ($RootName -like '*.exe') { $RootName } else { "$RootName.exe" }

    $children = @{}
    foreach ($p in $Processes) {
        $parent = [int]$p.ParentProcessId
        if (-not $children.ContainsKey($parent)) {
            $children[$parent] = New-Object 'System.Collections.Generic.List[int]'
        }
        $children[$parent].Add([int]$p.ProcessId)
    }

    $seen = New-Object 'System.Collections.Generic.HashSet[int]'
    $queue = New-Object 'System.Collections.Generic.Queue[int]'
    foreach ($p in $Processes) {
        if ($p.Name -ieq $exe) { $queue.Enqueue([int]$p.ProcessId) }
    }
    while ($queue.Count -gt 0) {
        $id = $queue.Dequeue()
        if (-not $seen.Add($id)) { continue }  # also breaks cycles from reused PIDs
        if ($children.ContainsKey($id)) {
            foreach ($child in $children[$id]) { $queue.Enqueue($child) }
        }
    }
    return ,([int[]]@($seen))
}

# Takes a sample of the given PIDs. $PrevCpu (PID → CPU seconds) comes from
# the previous sample; CPU is only worked out for processes present in both,
# so a freshly created process does not produce a false spike.
function Get-TreeSample {
    param([int[]]$Ids, [hashtable]$PrevCpu, [double]$ElapsedSeconds, [int]$Cores)

    $private = [long]0
    $workingSet = [long]0
    $cpuSeconds = 0.0
    $count = 0
    $cpuNow = @{}

    if ($Ids.Count -gt 0) {
        foreach ($p in @(Get-Process -Id $Ids -ErrorAction SilentlyContinue)) {
            $count++
            $private += $p.PrivateMemorySize64
            $workingSet += $p.WorkingSet64
            $total = $p.TotalProcessorTime
            if ($null -ne $total) {
                $cpuNow[$p.Id] = $total.TotalSeconds
                if ($PrevCpu.ContainsKey($p.Id)) {
                    $cpuSeconds += [Math]::Max(0, $total.TotalSeconds - $PrevCpu[$p.Id])
                }
            }
        }
    }

    $cpuPercent = $null
    if ($PrevCpu.Count -gt 0 -and $ElapsedSeconds -gt 0) {
        $cpuPercent = 100.0 * $cpuSeconds / ($ElapsedSeconds * $Cores)
    }

    [pscustomobject]@{
        Processes  = $count
        PrivateMB  = [Math]::Round($private / 1MB, 1)
        WorkingMB  = [Math]::Round($workingSet / 1MB, 1)
        CpuPercent = if ($null -eq $cpuPercent) { $null } else { [Math]::Round($cpuPercent, 2) }
        CpuTimes   = $cpuNow
    }
}

function Get-HardwareSummary {
    $cpu = 'unknown'
    $gpu = 'unknown'
    try { $cpu = (Get-CimInstance -ClassName Win32_Processor | Select-Object -First 1).Name.Trim() } catch { }
    try { $gpu = (@(Get-CimInstance -ClassName Win32_VideoController | ForEach-Object { $_.Name }) -join ' + ') } catch { }
    "CPU: $cpu`nGPU: $gpu"
}

function Invoke-Measure {
    $cores = [Environment]::ProcessorCount
    $rows = New-Object 'System.Collections.Generic.List[object]'

    $ids = Get-TreeIds -Processes @(Get-CimInstance -ClassName Win32_Process) -RootName $Name
    if ($ids.Count -eq 0) {
        Write-Error "'$Name' is not running."
        exit 1
    }

    Write-Host (Get-HardwareSummary)
    Write-Host ("Measuring '{0}' for {1} s (a sample every {2} s, {3} logical cores)`n" -f $Name, $Seconds, $Interval, $cores)
    Write-Host ('{0,8}  {1,5}  {2,12}  {3,12}  {4,7}' -f 'Sec.', 'Proc.', 'Private MB', 'WS MB', 'CPU %')

    $baseline = Get-TreeSample -Ids $ids -PrevCpu @{} -ElapsedSeconds 0 -Cores $cores
    $prevCpu = $baseline.CpuTimes
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $last = 0.0

    while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
        Start-Sleep -Seconds $Interval
        $ids = Get-TreeIds -Processes @(Get-CimInstance -ClassName Win32_Process) -RootName $Name
        if ($ids.Count -eq 0) {
            Write-Warning "'$Name' ended; stopping the measurement."
            break
        }
        # The time is taken right before reading the counters: taken before
        # listing the processes (which can be slow), CPU would be inflated.
        $now = $clock.Elapsed.TotalSeconds
        $s = Get-TreeSample -Ids $ids -PrevCpu $prevCpu -ElapsedSeconds ($now - $last) -Cores $cores
        $prevCpu = $s.CpuTimes
        $last = $now

        $row = [pscustomobject]@{
            Seconds    = [Math]::Round($now)
            Processes  = $s.Processes
            PrivateMB  = $s.PrivateMB
            WorkingMB  = $s.WorkingMB
            CpuPercent = $s.CpuPercent
        }
        $rows.Add($row)
        Write-Host ('{0,8}  {1,5}  {2,12:N1}  {3,12:N1}  {4,7:N2}' -f $row.Seconds, $row.Processes, $row.PrivateMB, $row.WorkingMB, $row.CpuPercent)
    }

    if ($rows.Count -eq 0) { return }

    $priv = $rows | Measure-Object -Property PrivateMB -Average -Maximum
    $ws = $rows | Measure-Object -Property WorkingMB -Average -Maximum
    $cpu = $rows | Measure-Object -Property CpuPercent -Average -Maximum

    Write-Host "`nSummary ($($rows.Count) samples)"
    Write-Host ('  Private memory: average {0:N1} MB, maximum {1:N1} MB' -f $priv.Average, $priv.Maximum)
    Write-Host ('  Working set:    average {0:N1} MB, maximum {1:N1} MB (overestimates)' -f $ws.Average, $ws.Maximum)
    Write-Host ('  CPU:            average {0:N2} %, maximum {1:N2} %' -f $cpu.Average, $cpu.Maximum)

    if ($Csv) {
        $rows | Export-Csv -Path $Csv -NoTypeInformation -Encoding UTF8
        Write-Host "`nSamples saved to $Csv"
    }
}

# Loading the script with '. .\measure.ps1' only defines the functions.
if ($MyInvocation.InvocationName -ne '.') { Invoke-Measure }
