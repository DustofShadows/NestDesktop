[CmdletBinding()]
param(
    [ValidateRange(10, 3600)]
    [int]$DurationSeconds = 300,
    [ValidateRange(1, 30)]
    [int]$IntervalSeconds = 5,
    [string]$ExecutablePath = (Join-Path $env:LOCALAPPDATA 'Programs\NestDesktop\nest-desktop.exe'),
    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
$executable = (Resolve-Path -LiteralPath $ExecutablePath).Path
$processName = [IO.Path]::GetFileName($executable).Replace("'", "''")
$instances = @(Get-CimInstance Win32_Process -Filter "Name='$processName'" | Where-Object {
    $_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath) -eq $executable
})
if ($instances.Count -ne 2) { throw 'Run one NestDesktop instance and its recovery helper before measuring.' }
$main = @($instances | Where-Object { $_.CommandLine -notmatch '--watch-parent=' })
$helper = @($instances | Where-Object { $_.CommandLine -match '--watch-parent=' })
if ($main.Count -ne 1 -or $helper.Count -ne 1 -or $helper[0].ParentProcessId -ne $main[0].ProcessId) {
    throw 'Could not identify the organizer and its recovery helper.'
}
$roles = @(
    @{ Name = 'organizer'; Id = [int]$main[0].ProcessId },
    @{ Name = 'recovery_helper'; Id = [int]$helper[0].ProcessId }
)
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$logical = [int](Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors
$os = Get-CimInstance Win32_OperatingSystem
$identity = @{}
foreach ($role in $roles) {
    $process = Get-Process -Id $role.Id
    $identity[$role.Name] = $process.StartTime.ToUniversalTime().Ticks
}

function Read-Sample([double]$Elapsed) {
    $filter = ($roles | ForEach-Object { 'IDProcess=' + $_.Id }) -join ' OR '
    $memory = @(Get-CimInstance Win32_PerfRawData_PerfProc_Process -Filter $filter)
    $perProcess = foreach ($role in $roles) {
        $process = Get-Process -Id $role.Id
        if ($process.StartTime.ToUniversalTime().Ticks -ne $identity[$role.Name]) {
            throw 'A measured process restarted; discard this run.'
        }
        $counter = @($memory | Where-Object { $_.IDProcess -eq $role.Id })
        if ($counter.Count -ne 1) { throw 'A process memory counter is unavailable.' }
        [pscustomobject]@{
            role = $role.Name
            cpu_seconds = $process.TotalProcessorTime.TotalSeconds
            working_set_bytes = [long]$counter[0].WorkingSet
            private_working_set_bytes = [long]$counter[0].WorkingSetPrivate
            private_committed_bytes = [long]$counter[0].PrivateBytes
        }
    }
    [pscustomobject]@{
        elapsed_seconds = $Elapsed
        processes = @($perProcess)
        cpu_seconds = [double](($perProcess | Measure-Object cpu_seconds -Sum).Sum)
        working_set_bytes = [long](($perProcess | Measure-Object working_set_bytes -Sum).Sum)
        private_working_set_bytes = [long](($perProcess | Measure-Object private_working_set_bytes -Sum).Sum)
        private_committed_bytes = [long](($perProcess | Measure-Object private_committed_bytes -Sum).Sum)
    }
}

function Summarize-Memory($Samples, [string]$Property) {
    $values = @($Samples | ForEach-Object { [long]$_.$Property } | Sort-Object)
    $middle = [int][Math]::Floor($values.Count / 2)
    $median = if ($values.Count % 2) { $values[$middle] } else { ($values[$middle - 1] + $values[$middle]) / 2.0 }
    [pscustomobject]@{ min_bytes = $values[0]; median_bytes = $median; max_bytes = $values[-1] }
}

# The script only reads counters. Close folder/settings panels before running it;
# it does not launch, close, move or modify the measured application.
$started = [DateTime]::UtcNow
$samples = New-Object 'System.Collections.Generic.List[object]'
$watch = [Diagnostics.Stopwatch]::StartNew()
$samples.Add((Read-Sample 0))
$next = $IntervalSeconds
$lastProgress = 0
while ($watch.Elapsed.TotalSeconds -lt $DurationSeconds) {
    $remaining = [Math]::Min($next, $DurationSeconds) - $watch.Elapsed.TotalSeconds
    if ($remaining -gt 0) { Start-Sleep -Milliseconds ([int][Math]::Ceiling($remaining * 1000)) }
    $samples.Add((Read-Sample $watch.Elapsed.TotalSeconds))
    $next += $IntervalSeconds
    if ($watch.Elapsed.TotalSeconds - $lastProgress -ge 30) {
        Write-Output ('Sampled {0:N0}s / {1}s' -f $watch.Elapsed.TotalSeconds, $DurationSeconds)
        $lastProgress = $watch.Elapsed.TotalSeconds
    }
}
$elapsed = $samples[-1].elapsed_seconds - $samples[0].elapsed_seconds
$cpuSeconds = $samples[-1].cpu_seconds - $samples[0].cpu_seconds
if ($cpuSeconds -lt 0) { throw 'CPU counters changed unexpectedly; discard this run.' }
$report = [pscustomobject]@{
    schema_version = 1
    started_utc = $started.ToString('o')
    duration_seconds = $elapsed
    sample_interval_seconds = $IntervalSeconds
    sample_count = $samples.Count
    includes_recovery_helper = $true
    windows = @{ caption = $os.Caption; version = $os.Version; build = $os.BuildNumber }
    processor = @{ name = $cpu.Name.Trim(); logical_processors = $logical }
    executable_bytes = (Get-Item -LiteralPath $executable).Length
    executable_sha256 = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
    cpu_seconds = $cpuSeconds
    average_cpu_percent_one_core = 100 * $cpuSeconds / $elapsed
    average_cpu_percent_machine = 100 * $cpuSeconds / ($elapsed * $logical)
    combined_working_set = (Summarize-Memory $samples 'working_set_bytes')
    combined_private_working_set = (Summarize-Memory $samples 'private_working_set_bytes')
    combined_private_committed = (Summarize-Memory $samples 'private_committed_bytes')
    samples = @($samples.ToArray())
}
$output = [IO.Path]::GetFullPath($OutputPath)
New-Item -ItemType Directory -Path (Split-Path -Parent $output) -Force | Out-Null
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $output -Encoding UTF8
Write-Output ('Measured {0:N3}s; combined CPU {1:N6}s; private resident median {2:N2} MiB; total working-set median {3:N2} MiB' -f $elapsed, $cpuSeconds, ($report.combined_private_working_set.median_bytes / 1MB), ($report.combined_working_set.median_bytes / 1MB))
