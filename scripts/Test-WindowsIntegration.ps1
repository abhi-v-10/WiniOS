#Requires -Version 7.4
param(
    [ValidateSet('Any','Off','On')][string]$ExpectedStartup = 'Any',
    [switch]$Launch
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows is required.' }
$install = Join-Path $env:LOCALAPPDATA 'Programs\WiniOS'
$exe = Join-Path $install 'winios.exe'
$shortcutPath = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)) 'WiniOS.lnk'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Installed winios.exe is missing.' }
if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf)) { throw 'Per-user WiniOS Start Menu shortcut is missing.' }
$wscript = New-Object -ComObject WScript.Shell
$shortcut = $wscript.CreateShortcut($shortcutPath)
if (-not [string]::Equals($shortcut.TargetPath,$exe,[StringComparison]::OrdinalIgnoreCase) -or $shortcut.Arguments -cne '--tray') { throw 'Shortcut target/arguments mismatch.' }
if (-not [string]::Equals($shortcut.WorkingDirectory.TrimEnd('\'),$install.TrimEnd('\'),[StringComparison]::OrdinalIgnoreCase)) { throw 'Shortcut working directory mismatch.' }
$iconComma = $shortcut.IconLocation.LastIndexOf(',')
if ($iconComma -lt 0) { throw 'Shortcut icon location is invalid.' }
$iconPath=$shortcut.IconLocation.Substring(0,$iconComma).Trim('"')
$packagedIcon=Join-Path $install 'winios.ico'
$expectedIcon=[string]::Equals($iconPath,$exe,[StringComparison]::OrdinalIgnoreCase) -or [string]::Equals($iconPath,$packagedIcon,[StringComparison]::OrdinalIgnoreCase)
if (-not $expectedIcon -or $shortcut.IconLocation.Substring($iconComma+1).Trim() -cne '0' -or -not (Test-Path -LiteralPath $iconPath -PathType Leaf)) { throw 'Shortcut must use the installed executable or packaged app icon at index zero.' }
$shell = New-Object -ComObject Shell.Application
$folder = $shell.NameSpace([IO.Path]::GetDirectoryName($shortcutPath))
$item = $folder.ParseName('WiniOS.lnk')
if ($item.ExtendedProperty('System.AppUserModel.ID') -cne 'WiniOS.Desktop') { throw 'Shortcut AppUserModelID mismatch.' }
$version = [Diagnostics.FileVersionInfo]::GetVersionInfo($exe)
if ($version.ProductName -cne 'WiniOS') { throw 'Embedded Windows app metadata is missing.' }
& (Join-Path $PSScriptRoot 'Test-DesktopBinary.ps1') -Directory $install
$run = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Run',$false)
try { $value = if ($run) { $run.GetValue('WiniOS',$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } else { $null } }
finally { if ($run) { $run.Dispose() } }
$expected = '"' + $exe + '" --startup'
$registered = $null -ne $value -and $value -ceq $expected
if ($null -ne $value -and -not $registered) { throw 'A conflicting startup entry exists; it was not changed.' }
if (($ExpectedStartup -eq 'On' -and -not $registered) -or ($ExpectedStartup -eq 'Off' -and $null -ne $value)) { throw 'Actual per-user Run registration does not match expected checkbox state.' }
if ($Launch) {
    # Explicitly requested launch only. Existing instances activate their popup;
    # no startup registration, settings or network configuration is changed.
    Start-Process -FilePath $shortcutPath -WorkingDirectory $install -WindowStyle Hidden
    $running = @()
    for ($attempt=0; $attempt -lt 20; $attempt++) {
        $running = @(Get-Process winios -ErrorAction SilentlyContinue)
        if ($running.Count -eq 1) { break }
        Start-Sleep -Milliseconds 200
    }
    if ($running.Count -ne 1 -or -not [string]::Equals($running[0].Path,$exe,[StringComparison]::OrdinalIgnoreCase)) { throw 'Start Menu launch did not yield exactly one installed tray agent.' }
}
Write-Output "PASS: installed GUI/console binaries; per-user Start Menu target, icon, working directory and AppUserModelID; startup registered=$registered."
