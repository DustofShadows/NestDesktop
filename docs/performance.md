# Resource measurements

## Measured setup

- NestDesktop **0.2.11**, existing installed instance, measured on **23 November 2026**.
- Windows 11 Pro for Workstations, build **26200**.
- **AMD Ryzen 7 9700X 8-Core Processor**, **16 logical processors**.
- **5 collapsed desktop groups and 53 collected shortcuts**; folder panels,
  settings and the colour panel stayed closed. Hover opening was enabled.
- Both the organizer and sleeping recovery helper were included. Neither process
  restarted, and no desktop-tile mouse events occurred during the observation.
- Duration **300.006 seconds**, **61 samples** at approximately
  **5 seconds** apart. Startup and interactions are outside this idle sample.

## Results

All memory figures below are MiB (1 MiB = 1,048,576 bytes). Ranges are the minimum
and maximum observed samples, not a promised limit or an interaction peak.

| Memory metric | Minimum MiB | Median MiB | Maximum MiB |
|---|---:|---:|---:|
| Combined working set | 46.77 | 46.77 | 52.11 |
| Combined private working set | 1.27 | 1.27 | 6.61 |
| Combined private committed memory | 12.85 | 12.85 | 12.85 |

| Process | Median working set MiB | Median private working set MiB | Median private committed MiB | CPU seconds during test |
|---|---:|---:|---:|---:|
| Organizer | 38.48 | 1.18 | 11.30 | 0.000 |
| Recovery helper | 8.29 | 0.10 | 1.55 | 0.000 |

Combined CPU time increased by **0.000 seconds** during the test: no measurable
CPU time at the counter's resolution. This corresponds to a measured **0.0000%
average machine CPU** for this interval. It is an observation of one idle run,
not a claim that the app always uses zero CPU. Opening, dragging, searching,
launching or repainting can consume CPU.

The application executable was **1,045,504 bytes (0.997 MiB)**. This is the
EXE size, not the installed package size or a memory measurement.

## What the memory counters mean

- **Working set** is resident process memory, including private and shared pages.
  The sum for both processes can count the same shared page more than once; it
  is a conservative combined process figure, not unique system-wide physical RAM.
- **Private working set** is resident memory private to the processes. Windows
  can trim or page out private pages while an app is idle, which happened during
  this observation. That lower figure is not used as the headline memory claim.
- **Private committed memory** includes private allocations regardless of whether
  their pages are currently resident. It is not another amount to add to working set.

These counters are distinguished in Microsoft's
[process memory reference](https://learn.microsoft.com/en-us/windows/win32/memory/memory-performance-information).

## Reproduce

With one normal NestDesktop instance running and its folder/settings panels
closed, run from the source checkout:

```powershell
.\scripts\measure-idle.ps1 -DurationSeconds 300 -IntervalSeconds 5 -OutputPath .\artifacts\idle.json
```

The script only reads counters for the selected executable and its recovery
helper, and writes the requested JSON report. It neither launches nor changes
NestDesktop. Use `-ExecutablePath` for a different install location. Keep the
app idle during the interval. Windows PowerShell 5.1 and PowerShell 7 syntax
were checked; this recorded run used PowerShell 7.

CPU is the difference between each process's cumulative
[TotalProcessorTime](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.process.totalprocessortime),
summed across both processes. Machine CPU percentage is:

`100 × combined CPU seconds / (elapsed seconds × logical processor count)`.

Memory comes from the Windows `Process` performance counters (`WorkingSet`,
`WorkingSetPrivate`, `PrivateBytes`). Minima, maxima and medians are computed
from the sampled sums. No application commands were sent inside the timed interval;
UI diagnostics were checked before and after it.

[Raw samples and executable SHA-256](performance-idle-0.2.11.json) are included.
The report contains counts and resource metrics, without desktop item names or
local user paths.

## Limits

Results vary with Windows, hardware, icon caches, contents, activity and memory
pressure. This measures idle process CPU and memory on one PC; it does not
measure GPU/compositor allocations, system-wide Explorer/DWM costs, startup,
interaction speed or large-folder workloads. It supports an idle background
resource statement, not a universal performance guarantee.
