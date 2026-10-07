<#
.SYNOPSIS
    Mide memoria privada, working set y CPU de un árbol de procesos.

.DESCRIPTION
    Suma el proceso raíz y todos sus descendientes, recalculando el árbol en
    cada muestra (WebView2 crea y cierra procesos sobre la marcha).

    - yt-mini-player: incluye todos los msedgewebview2.exe que cuelgan de él.
    - Un navegador (msedge, chrome): incluye todos sus procesos. Para comparar
      en igualdad de condiciones, ciérralo del todo y ábrelo con una sola
      pestaña del mismo video.

    La métrica principal es la memoria privada (Private Bytes). La suma de
    working sets cuenta varias veces las páginas compartidas entre procesos,
    así que sobreestima el consumo real; se muestra solo como referencia.

    Compatible con Windows PowerShell 5.1 y PowerShell 7.

.EXAMPLE
    .\tools\measure.ps1
    Mide yt-mini-player durante 5 minutos, una muestra cada 5 segundos.

.EXAMPLE
    .\tools\measure.ps1 -Name msedge -Seconds 600 -Csv edge.csv
    Mide Edge durante 10 minutos y guarda las muestras en edge.csv.
#>
[CmdletBinding()]
param(
    [string]$Name = 'yt-mini-player',
    [ValidateRange(10, 86400)][int]$Seconds = 300,
    [ValidateRange(1, 60)][int]$Interval = 5,
    [string]$Csv
)

Set-StrictMode -Version 2.0

# Devuelve los PID del árbol: todo proceso llamado $RootName más sus
# descendientes. $Processes son objetos con ProcessId, ParentProcessId y Name
# (como los de Win32_Process).
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
        if (-not $seen.Add($id)) { continue }  # también corta ciclos por PID reutilizado
        if ($children.ContainsKey($id)) {
            foreach ($child in $children[$id]) { $queue.Enqueue($child) }
        }
    }
    return ,([int[]]@($seen))
}

# Toma una muestra de los PID dados. $PrevCpu (PID → segundos de CPU) viene
# de la muestra anterior; la CPU solo se calcula para procesos presentes en
# ambas, así un proceso recién creado no produce un pico falso.
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
    $cpu = 'desconocida'
    $gpu = 'desconocida'
    try { $cpu = (Get-CimInstance -ClassName Win32_Processor | Select-Object -First 1).Name.Trim() } catch { }
    try { $gpu = (@(Get-CimInstance -ClassName Win32_VideoController | ForEach-Object { $_.Name }) -join ' + ') } catch { }
    "CPU: $cpu`nGPU: $gpu"
}

function Invoke-Measure {
    $cores = [Environment]::ProcessorCount
    $rows = New-Object 'System.Collections.Generic.List[object]'

    $ids = Get-TreeIds -Processes @(Get-CimInstance -ClassName Win32_Process) -RootName $Name
    if ($ids.Count -eq 0) {
        Write-Error "No se encontró '$Name' en ejecución."
        exit 1
    }

    Write-Host (Get-HardwareSummary)
    Write-Host ("Midiendo '{0}' durante {1} s (muestra cada {2} s, {3} núcleos lógicos)`n" -f $Name, $Seconds, $Interval, $cores)
    Write-Host ('{0,8}  {1,5}  {2,12}  {3,12}  {4,7}' -f 'Seg.', 'Proc.', 'Privada MB', 'WS MB', 'CPU %')

    $baseline = Get-TreeSample -Ids $ids -PrevCpu @{} -ElapsedSeconds 0 -Cores $cores
    $prevCpu = $baseline.CpuTimes
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $last = 0.0

    while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
        Start-Sleep -Seconds $Interval
        $ids = Get-TreeIds -Processes @(Get-CimInstance -ClassName Win32_Process) -RootName $Name
        if ($ids.Count -eq 0) {
            Write-Warning "'$Name' terminó; se detiene la medición."
            break
        }
        # El tiempo se toma justo antes de leer los contadores: si se tomara
        # antes de enumerar procesos (que puede tardar), la CPU se inflaría.
        $now = $clock.Elapsed.TotalSeconds
        $s = Get-TreeSample -Ids $ids -PrevCpu $prevCpu -ElapsedSeconds ($now - $last) -Cores $cores
        $prevCpu = $s.CpuTimes
        $last = $now

        $row = [pscustomobject]@{
            Segundos   = [Math]::Round($now)
            Procesos   = $s.Processes
            PrivadaMB  = $s.PrivateMB
            WorkingMB  = $s.WorkingMB
            CpuPercent = $s.CpuPercent
        }
        $rows.Add($row)
        Write-Host ('{0,8}  {1,5}  {2,12:N1}  {3,12:N1}  {4,7:N2}' -f $row.Segundos, $row.Procesos, $row.PrivadaMB, $row.WorkingMB, $row.CpuPercent)
    }

    if ($rows.Count -eq 0) { return }

    $priv = $rows | Measure-Object -Property PrivadaMB -Average -Maximum
    $ws = $rows | Measure-Object -Property WorkingMB -Average -Maximum
    $cpu = $rows | Measure-Object -Property CpuPercent -Average -Maximum

    Write-Host "`nResumen ($($rows.Count) muestras)"
    Write-Host ('  Memoria privada: promedio {0:N1} MB, máximo {1:N1} MB' -f $priv.Average, $priv.Maximum)
    Write-Host ('  Working set:     promedio {0:N1} MB, máximo {1:N1} MB (sobreestima)' -f $ws.Average, $ws.Maximum)
    Write-Host ('  CPU:             promedio {0:N2} %, máximo {1:N2} %' -f $cpu.Average, $cpu.Maximum)

    if ($Csv) {
        $rows | Export-Csv -Path $Csv -NoTypeInformation -Encoding UTF8
        Write-Host "`nMuestras guardadas en $Csv"
    }
}

# Al cargar el script con '. .\measure.ps1' solo se definen las funciones.
if ($MyInvocation.InvocationName -ne '.') { Invoke-Measure }
