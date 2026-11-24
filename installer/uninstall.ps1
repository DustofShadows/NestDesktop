[CmdletBinding(SupportsShouldProcess = $true)]
param()

$ErrorActionPreference = 'Stop'
$installRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs\NestDesktop'))
$expectedParent = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs'))
if ([IO.Path]::GetDirectoryName($installRoot) -ne $expectedParent -or [IO.Path]::GetFileName($installRoot) -ne 'NestDesktop') {
    throw 'The installation directory could not be verified.'
}
if ((Test-Path -LiteralPath $installRoot) -and ((Get-Item -LiteralPath $installRoot).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'The installation directory is a link. Refusing to remove it.'
}
$installedExecutable = Join-Path $installRoot 'nest-desktop.exe'
foreach ($process in @(Get-Process -Name 'nest-desktop' -ErrorAction SilentlyContinue)) {
    $processPath = $null
    try { $processPath = $process.MainModule.FileName } catch { }
    if ($processPath -and [IO.Path]::GetFullPath($processPath) -eq $installedExecutable) {
        throw 'Quit NestDesktop from its tray menu, then uninstall again.'
    }
}
if (-not $PSCmdlet.ShouldProcess($installRoot, 'Remove NestDesktop program files, desktop menu and registrations; keep saved layouts')) { return }

# Restore collected desktop shortcuts before the executable is removed.
# Conflicts stay in the retained collection ledger; existing names are not overwritten.
if (Test-Path -LiteralPath $installedExecutable -PathType Leaf) {
    $restore = Start-Process -FilePath $installedExecutable -ArgumentList '--restore-shortcuts','--background' -WorkingDirectory $installRoot -WindowStyle Hidden -PassThru
    try {
        if (-not $restore.WaitForExit(30000)) {
            throw 'Shortcut restore has not finished. Wait, then retry uninstalling. Program files and layouts were kept.'
        }
        if ($restore.ExitCode -ne 0) {
            throw 'Shortcut restore reported an error. Program files and layouts were kept. Restore the shortcuts in NestDesktop before retrying.'
        }
    }
    finally { $restore.Dispose() }
}

# Only known installed files are removed. Never recurse through user additions.
foreach ($name in @('nest-desktop.exe', 'NestDesktop-Setup.exe', 'README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'nest-desktop.exe.installing', 'NestDesktop-Setup.exe.installing')) {
    $installedFile = Join-Path $installRoot $name
    if (Test-Path -LiteralPath $installedFile -PathType Leaf) { Remove-Item -LiteralPath $installedFile -Force }
}
if ((Test-Path -LiteralPath $installRoot) -and -not @(Get-ChildItem -LiteralPath $installRoot -Force).Count) {
    Remove-Item -LiteralPath $installRoot -Force
}
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
Remove-ItemProperty -LiteralPath $runKey -Name NestDesktop -ErrorAction SilentlyContinue
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\NestDesktop'
if (Test-Path -LiteralPath $uninstallKey) { Remove-Item -LiteralPath $uninstallKey -Force }
$desktopMenuKey = 'HKCU:\Software\Classes\DesktopBackground\Shell\NestDesktop'
if (Test-Path -LiteralPath $desktopMenuKey) {
    # Registry recursion is confined to NestDesktop's own exact verb key.
    Remove-Item -LiteralPath $desktopMenuKey -Recurse -Force
}
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'NestDesktop.lnk'
if (Test-Path -LiteralPath $shortcutPath -PathType Leaf) { Remove-Item -LiteralPath $shortcutPath -Force }
Write-Output 'NestDesktop was removed. Collected shortcuts were restored where names were available. Saved layouts and any conflicting collected shortcuts remain in Local AppData\NestDesktop.'

