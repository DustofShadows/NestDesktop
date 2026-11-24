[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$SourceDirectory = (Split-Path -Parent $PSScriptRoot),
    [switch]$NoStartup,
    [switch]$NoLaunch
)

$ErrorActionPreference = 'Stop'
$installRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs\NestDesktop'))
$expectedParent = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs'))
if ([IO.Path]::GetDirectoryName($installRoot) -ne $expectedParent -or [IO.Path]::GetFileName($installRoot) -ne 'NestDesktop') {
    throw 'The installation directory could not be verified.'
}
if ((Test-Path -LiteralPath $installRoot) -and ((Get-Item -LiteralPath $installRoot).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'The installation directory must be a normal local directory, not a link.'
}
$installedExecutable = Join-Path $installRoot 'nest-desktop.exe'
foreach ($process in @(Get-Process -Name 'nest-desktop' -ErrorAction SilentlyContinue)) {
    $processPath = $null
    try { $processPath = $process.MainModule.FileName } catch { }
    if ($processPath -and [IO.Path]::GetFullPath($processPath) -eq $installedExecutable) {
        throw 'Quit NestDesktop from its tray menu, then run the installer again.'
    }
}
$sourceExecutable = Join-Path $SourceDirectory 'nest-desktop.exe'
if (-not (Test-Path -LiteralPath $sourceExecutable -PathType Leaf)) {
    throw 'Extract the complete release ZIP first. nest-desktop.exe is missing.'
}
if (-not $PSCmdlet.ShouldProcess($installRoot, 'Install NestDesktop, add the desktop New app folder menu and optional sign-in startup')) {
    [pscustomobject]@{
        InstallDirectory = $installRoot
        Executable = $installedExecutable
        SourceExecutable = [IO.Path]::GetFullPath($sourceExecutable)
        StartAtSignIn = -not $NoStartup
        OpenAfterInstall = -not $NoLaunch
    }
    return
}
New-Item -ItemType Directory -Path $installRoot -Force | Out-Null
foreach ($name in @('nest-desktop.exe', 'NestDesktop-Setup.exe', 'README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md')) {
    $source = [IO.Path]::GetFullPath((Join-Path $SourceDirectory $name))
    $destination = Join-Path $installRoot $name
    if ((Test-Path -LiteralPath $source -PathType Leaf) -and $source -ne $destination) {
        $temporary = "$destination.installing"
        Copy-Item -LiteralPath $source -Destination $temporary -Force
        if (Test-Path -LiteralPath $destination) { [IO.File]::Replace($temporary, $destination, [System.Management.Automation.Language.NullString]::Value) }
        else { [IO.File]::Move($temporary, $destination) }
    }
}
$programsDirectory = [Environment]::GetFolderPath('Programs')
New-Item -ItemType Directory -Path $programsDirectory -Force | Out-Null
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut((Join-Path $programsDirectory 'NestDesktop.lnk'))
try {
    $shortcut.TargetPath = $installedExecutable
    $shortcut.WorkingDirectory = $installRoot
    $shortcut.Description = 'Android-inspired folders for your Windows desktop'
    $shortcut.IconLocation = "$installedExecutable,0"
    $shortcut.Save()
}
finally {
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
}
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
if ($NoStartup) {
    Remove-ItemProperty -LiteralPath $runKey -Name NestDesktop -ErrorAction SilentlyContinue
}
else {
    New-Item -Path $runKey -Force | Out-Null
    New-ItemProperty -LiteralPath $runKey -Name NestDesktop -Value ('"' + $installedExecutable + '" --background') -PropertyType String -Force | Out-Null
}
$desktopMenuKey = 'HKCU:\Software\Classes\DesktopBackground\Shell\NestDesktop'
New-Item -Path $desktopMenuKey -Force | Out-Null
New-ItemProperty -LiteralPath $desktopMenuKey -Name MUIVerb -Value 'New app folder' -PropertyType String -Force | Out-Null
New-ItemProperty -LiteralPath $desktopMenuKey -Name Icon -Value $installedExecutable -PropertyType String -Force | Out-Null
$desktopCommandKey = Join-Path $desktopMenuKey 'command'
New-Item -Path $desktopCommandKey -Force | Out-Null
Set-Item -LiteralPath $desktopCommandKey -Value ('"' + $installedExecutable + '" --new-folder')
$setupPath = Join-Path $installRoot 'NestDesktop-Setup.exe'
if (Test-Path -LiteralPath $setupPath) {
    $uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\NestDesktop'
    New-Item -Path $uninstallKey -Force | Out-Null
    $values = @{
        DisplayName = 'NestDesktop'
        DisplayVersion = '0.2.11'
        Publisher = 'NestDesktop'
        InstallLocation = $installRoot
        DisplayIcon = $installedExecutable
        UninstallString = ('"' + $setupPath + '" --uninstall')
    }
    foreach ($entry in $values.GetEnumerator()) {
        New-ItemProperty -LiteralPath $uninstallKey -Name $entry.Key -Value $entry.Value -PropertyType String -Force | Out-Null
    }
    New-ItemProperty -LiteralPath $uninstallKey -Name NoModify -Value 1 -PropertyType DWord -Force | Out-Null
    New-ItemProperty -LiteralPath $uninstallKey -Name NoRepair -Value 1 -PropertyType DWord -Force | Out-Null
}
Write-Output "Installed NestDesktop in $installRoot"
if (-not $NoLaunch) { Start-Process -FilePath $installedExecutable -ArgumentList '--reveal' -WorkingDirectory $installRoot -WindowStyle Hidden }

