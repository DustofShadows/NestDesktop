[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$Target = '',
    [string]$Cargo = 'cargo',
    [string]$OutputDirectory = ''
)

$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifestPath = Join-Path $projectRoot 'Cargo.toml'
$manifest = Get-Content -LiteralPath $manifestPath -Raw
$versionMatch = [regex]::Match($manifest, '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+(?:[-+][^"\r\n]+)?)"')
if (-not $versionMatch.Success) { throw 'Could not read the package version from Cargo.toml.' }
$version = $versionMatch.Groups[1].Value
if ($Target -and $Target -ne 'x86_64-pc-windows-gnu' -and $Target -ne 'x86_64-pc-windows-msvc') {
    throw 'This release packager currently supports Windows x64 targets only.'
}
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $projectRoot 'dist' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null

Push-Location $projectRoot
try {
    if (-not $SkipBuild) {
        $cargoArguments = @('test', '--locked')
        if ($Target) { $cargoArguments += @('--target', $Target) }
        & $Cargo @cargoArguments
        if ($LASTEXITCODE -ne 0) { throw "Cargo tests failed with exit code $LASTEXITCODE." }
        $cargoArguments = @('build', '--release', '--locked')
        if ($Target) { $cargoArguments += @('--target', $Target) }
        & $Cargo @cargoArguments
        if ($LASTEXITCODE -ne 0) { throw "Cargo release build failed with exit code $LASTEXITCODE." }
    }
    $binaryFolder = Join-Path $projectRoot 'target'
    if ($Target) { $binaryFolder = Join-Path $binaryFolder $Target }
    $binary = Join-Path $binaryFolder 'release\nest-desktop.exe'
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw "Release executable not found: $binary" }

    $stageName = "NestDesktop-$version-win-x64"
    $stageContainer = Join-Path $projectRoot ('target\package\' + [Guid]::NewGuid().ToString('N'))
    $stage = Join-Path $stageContainer $stageName
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'nest-desktop.exe')
    foreach ($name in @('LICENSE', 'THIRD_PARTY_NOTICES.md', 'CHANGELOG.md')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination (Join-Path $stage $name)
    }
    # The hosted/source README contains a GIF linked to the full demo. Keep
    # those presentation assets out of the application download and install.
    $readme = [IO.File]::ReadAllText((Join-Path $projectRoot 'README.md'))
    $distributionReadme = [regex]::Replace($readme, '(?s)<!-- nestdesktop-demo:start -->.*?<!-- nestdesktop-demo:end -->\r?\n(?:\r?\n)?', '')
    [IO.File]::WriteAllText((Join-Path $stage 'README.md'), $distributionReadme, (New-Object Text.UTF8Encoding $false))
    New-Item -ItemType Directory -Path (Join-Path $stage 'docs') -Force | Out-Null
    foreach ($name in @('usage.md', 'building.md', 'testing.md', 'architecture.md', 'performance.md', 'performance-idle-0.2.11.json')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot "docs\$name") -Destination (Join-Path $stage "docs\$name")
    }
    New-Item -ItemType Directory -Path (Join-Path $stage 'installer') -Force | Out-Null
    foreach ($name in @('install.ps1', 'uninstall.ps1')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot "installer\$name") -Destination (Join-Path $stage "installer\$name")
    }

    $csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
    if (-not (Test-Path -LiteralPath $csc)) { $csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe' }
    if (-not (Test-Path -LiteralPath $csc)) { throw 'The Windows .NET Framework C# compiler was not found.' }
    $setup = Join-Path $stage 'NestDesktop-Setup.exe'
    $compilerArguments = @(
        '/nologo', '/target:winexe', '/optimize+', '/platform:anycpu',
        '/reference:System.dll', '/reference:System.Core.dll', '/reference:System.Drawing.dll',
        '/reference:System.Windows.Forms.dll', '/reference:Microsoft.CSharp.dll',
        ('/win32manifest:' + (Join-Path $projectRoot 'installer\app.manifest')),
        ('/win32icon:' + (Join-Path $projectRoot 'assets\nest-desktop.ico')),
        ('/out:' + $setup), (Join-Path $projectRoot 'installer\Setup.cs')
    )
    & $csc @compilerArguments
    if ($LASTEXITCODE -ne 0) { throw "Setup compilation failed with exit code $LASTEXITCODE." }

    $fileHashes = foreach ($file in @(Get-ChildItem -LiteralPath $stage -File -Recurse)) {
        $relative = $file.FullName.Substring($stage.Length + 1).Replace('\', '/')
        '{0}  {1}' -f (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $relative
    }
    $fileHashes | Set-Content -LiteralPath (Join-Path $stage 'SHA256SUMS.txt') -Encoding ASCII
    $archive = Join-Path $OutputDirectory ($stageName + '.zip')
    Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Optimal -Force
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    ('{0}  {1}' -f $hash, [IO.Path]::GetFileName($archive)) | Set-Content -LiteralPath ($archive + '.sha256') -Encoding ASCII
    Write-Output "Release folder: $stage"
    Write-Output "Release archive: $archive"
    Write-Output "SHA-256: $hash"
}
finally { Pop-Location }

