[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$destination = [IO.Path]::GetFullPath($OutputDirectory)
if ($destination -eq $projectRoot -or $projectRoot.StartsWith($destination.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -or $destination.StartsWith($projectRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Choose a separate output folder, outside the source folder and its parents.'
}
if ((Test-Path -LiteralPath $destination) -and @(Get-ChildItem -LiteralPath $destination -Force).Count) {
    throw 'The public-source output folder must be empty. Existing files are never removed.'
}

$files = @(
    '.gitignore', 'Cargo.toml', 'Cargo.lock', 'build.rs', 'README.md',
    'LICENSE', 'THIRD_PARTY_NOTICES.md', 'CHANGELOG.md', '.github\FUNDING.yml',
    'docs\usage.md', 'docs\building.md', 'docs\testing.md', 'docs\architecture.md',
    'docs\performance.md', 'docs\performance-idle-0.2.11.json',
    'docs\media\demo.gif', 'docs\media\demo.mp4', 'docs\media\thumbnail.jpg',
    'installer\Setup.cs', 'installer\app.manifest',
    'installer\install.ps1', 'installer\uninstall.ps1'
)
foreach ($selection in @(
    @{ Directory = 'src'; Pattern = '*.rs' },
    @{ Directory = 'assets'; Pattern = '*.ico' },
    @{ Directory = 'assets'; Pattern = '*.manifest' },
    @{ Directory = 'scripts'; Pattern = '*.py' },
    @{ Directory = 'scripts'; Pattern = '*.ps1' }
)) {
    foreach ($file in @(Get-ChildItem -LiteralPath (Join-Path $projectRoot $selection.Directory) -Filter $selection.Pattern -File)) {
        $files += $file.FullName.Substring($projectRoot.Length + 1)
    }
}
$files = @($files | Sort-Object -Unique)

# Verify the complete allowlist before copying any file.
foreach ($relative in $files) {
    $source = [IO.Path]::GetFullPath((Join-Path $projectRoot $relative))
    if (-not $source.StartsWith($projectRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or -not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Missing or invalid public source file: $relative"
    }
}
New-Item -ItemType Directory -Path $destination -Force | Out-Null
foreach ($relative in $files) {
    $source = Join-Path $projectRoot $relative
    $target = [IO.Path]::GetFullPath((Join-Path $destination $relative))
    if (-not $target.StartsWith($destination.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'A public source path escaped its destination.'
    }
    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    Copy-Item -LiteralPath $source -Destination $target
    if ((Get-FileHash -LiteralPath $source).Hash -ne (Get-FileHash -LiteralPath $target).Hash) {
        throw "Copy verification failed: $relative"
    }
}
Write-Output "Public source: $destination"
Write-Output "Verified files: $($files.Count)"
