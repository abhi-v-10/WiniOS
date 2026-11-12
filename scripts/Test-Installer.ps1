#Requires -Version 7.4
[CmdletBinding()]
param(
    [string]$SetupPath,
    [string]$PreviousSetupPath,
    [switch]$Lifecycle
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$project=Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Installer.Common.ps1')
$cargoVersion=[regex]::Match((Get-Content -LiteralPath (Join-Path $project 'Cargo.toml') -Raw),'(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"').Groups[1].Value
if(-not $SetupPath){$SetupPath=Join-Path $project "target\dist\$cargoVersion\WiniOS-Setup.exe"}
function Assert([bool]$Condition,[string]$Message){if(-not $Condition){throw $Message}}
function Assert-Artifact([string]$Path){
    $manifestPath=Join-Path (Split-Path $Path) 'WiniOS-Setup.manifest.json'
    $manifest=Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    Assert ((Get-FileHash -LiteralPath $Path).Hash -ceq $manifest.InstallerSHA256) 'Installer hash differs from the build manifest.'
    Assert ((Get-WiniOSPeDetails $Path).Subsystem -eq 2) 'Installer must use the GUI subsystem.'
    Assert ($manifest.StaticCrt -eq $true) 'Shipping payload must use static CRT.'
    $info=[Diagnostics.FileVersionInfo]::GetVersionInfo($Path)
    $numericVersion=[version]::new($info.ProductMajorPart,$info.ProductMinorPart,$info.ProductBuildPart,$info.ProductPrivatePart)
    Assert ($numericVersion -eq [version]($manifest.PackageVersion + $(if($manifest.PackageVersion.Split('.').Count-eq 3){'.0'}else{''}))) 'Installer version differs from manifest.'
    Assert ($info.ProductName.Trim() -ceq 'WiniOS' -and $info.CompanyName.Trim() -ceq 'Abhishyanth') 'Installer branding metadata mismatch.'
    return $manifest
}
$SetupPath=(Resolve-Path -LiteralPath $SetupPath).Path
$manifest=Assert-Artifact $SetupPath
$source=Get-Content -LiteralPath (Join-Path $project 'installer\WiniOS.iss') -Raw
foreach($required in @('AppId={{D2286755-6B43-49F1-AC97-09151B20B0A8}','PrivilegesRequired=lowest','DefaultDirName={localappdata}\Programs\WiniOS','UsePreviousAppDir=no','CloseApplications=no','RestartApplications=no','AppUserModelID: "WiniOS.Desktop"','Parameters: "--tray"; WorkingDir: "{app}"; IconFilename: "{app}\winios.ico"','StopAndLockAgent','SnapshotStartup','NativeCreateMutex','UninstallLogMode=append')){Assert ($source.Contains($required)) "Missing installer policy: $required"}
foreach($forbidden in @('[UninstallDelete]','[InstallDelete]','HKLM','recursesubdirs','TerminateProcess','taskkill','RegDeleteKey','CredDelete','Exec(','DelTree(')){Assert (-not $source.Contains($forbidden)) "Forbidden installer action: $forbidden"}
$fileSection=[regex]::Match($source,'(?s)\[Files\](.*?)\[Icons\]').Groups[1].Value
$owned=@(Get-WiniOSPackageFiles)
$sources=@([regex]::Matches($fileSection,'(?m)^Source: "\{#PayloadDir\}\\([^"\\]+)";') | ForEach-Object {$_.Groups[1].Value})
Assert (@(Compare-Object $owned $sources).Count -eq 0) 'Installer sources differ from the reviewed payload allowlist.'
Assert (([regex]::Matches($fileSection,'(?m)^Source:')).Count -eq $owned.Count) 'Unexpected installer source.'
Assert (-not $fileSection.Contains('*')) 'Wildcard installer payload is forbidden.'
Assert ($source.Contains('ConfirmUninstall=') -and $source.Contains('Tailscale Serve route may remain')) 'Uninstall must explain retained data and Serve.'
Assert-WiniOSPayload (Join-Path $project 'target\installer-build\release')
Assert-WiniOSVersion (Join-Path $project 'target\installer-build\release') $cargoVersion
Assert ($manifest.ApplicationVersion -ceq $cargoVersion) 'Artifact uses a stale application version.'
Assert (@(Compare-Object $owned @($manifest.Payload.Name)).Count -eq 0) 'Manifest must cover every owned file exactly once.'
$stage=Join-Path $project "target\installer\stage\$($manifest.PackageVersion)"
Assert (@(Compare-Object $owned @(Get-ChildItem -LiteralPath $stage | Select-Object -ExpandProperty Name)).Count -eq 0) 'Staging includes missing or unowned files.'
foreach($entry in $manifest.Payload){Assert ((Get-FileHash -LiteralPath (Join-Path $stage $entry.Name)).Hash -ceq $entry.SHA256) 'Staged payload hash mismatch.'}
foreach($guide in @('mission-010.md','mission-014.md','mission-015.md')){
    $guideText=Get-Content -LiteralPath (Join-Path $stage $guide) -Raw
    Assert ($guideText -notmatch '(?m)^## (?:Automated verification and rollback|Verified results|Verification and deployment|Recorded verification)') 'Historical machine-specific results must not be shipped as setup instructions.'
}
foreach($name in @('winios.exe','winios-agent.exe')){Assert ((Get-FileHash -LiteralPath (Join-Path $stage $name)).Hash -ceq (Get-FileHash -LiteralPath (Join-Path $project "target\installer-build\release\$name")).Hash) 'Staged binary differs from validated release.'}
foreach($name in @('winios.exe','winios-agent.exe')){
    $binaryText=[Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes((Join-Path $stage $name)))
    Assert ($binaryText.Contains('<assemblyIdentity version="'+$cargoVersion+'.0"') -and $binaryText.Contains('requestedExecutionLevel level="asInvoker"') -and $binaryText.Contains('PerMonitorV2')) 'Embedded app manifest version/privilege/DPI mismatch.'
}
$icon=[IO.File]::ReadAllBytes((Join-Path $stage 'winios.ico'))
Assert ($icon.Length -ge 118 -and [BitConverter]::ToUInt16($icon,0)-eq 0 -and [BitConverter]::ToUInt16($icon,2)-eq 1 -and [BitConverter]::ToUInt16($icon,4)-eq 7) 'Expected multi-resolution WiniOS icon.'
$sizes=@(16,24,32,48,64,128,256)
for($i=0;$i-lt $sizes.Count;$i++){
    $offset=6+16*$i;$size=$sizes[$i]%256
    Assert ($icon[$offset]-eq $size -and $icon[$offset+1]-eq $size -and [BitConverter]::ToUInt16($icon,$offset+6)-eq 32) 'Icon size/depth mismatch.'
    $length=[BitConverter]::ToUInt32($icon,$offset+8);$start=[BitConverter]::ToUInt32($icon,$offset+12)
    Assert ($start-ge 118 -and $length-gt 0 -and [long]$start+$length-le $icon.Length) 'Icon payload outside file.'
}
$invalid=Join-Path $project 'target\invalid-pe-test.bin'
[IO.File]::WriteAllBytes($invalid,[byte[]](1,2,3))
try { $rejected=$false;try{Get-WiniOSPeDetails $invalid | Out-Null}catch{$rejected=$true};Assert $rejected 'Malformed PE must be rejected.' } finally { Remove-Item -LiteralPath $invalid }
Write-Output 'PASS: installer/payload hashes, version/branding metadata, seven icon sizes, GUI/static-CRT payload, explicit ownership, per-user/Start Menu policy, graceful-lock code, and no personal-data/network deletion.'
if(-not $Lifecycle){return}
if(-not $PreviousSetupPath){throw '-Lifecycle requires -PreviousSetupPath pointing to an older-version fixture built from the same verified payload.'}
$PreviousSetupPath=(Resolve-Path -LiteralPath $PreviousSetupPath).Path
$previous=Assert-Artifact $PreviousSetupPath
Assert ([version]$previous.PackageVersion -lt [version]$manifest.PackageVersion) 'Fixture must have an older installer version.'
$install=Join-Path $env:LOCALAPPDATA 'Programs\WiniOS'
$exe=Join-Path $install 'winios.exe'
$personal=Join-Path $env:LOCALAPPDATA 'WiniOS'
$link=Join-Path ([Environment]::GetFolderPath('Programs')) 'WiniOS.lnk'
$arp='Software\Microsoft\Windows\CurrentVersion\Uninstall\{D2286755-6B43-49F1-AC97-09151B20B0A8}_is1'
$runPath='Software\Microsoft\Windows\CurrentVersion\Run'
Assert (Test-Path -LiteralPath (Join-Path $personal 'settings.json')) 'Safe lifecycle test expects existing settings; fresh-profile testing is a separate manual scenario.'
$stamp=Get-Date -Format 'yyyyMMdd-HHmmss'
$backup=Join-Path $project "target\installer-validation\$stamp"
[void](New-Item -ItemType Directory -Path $backup -Force)
foreach($name in @('winios.exe','winios-agent.exe','winios.ico')){if(Test-Path -LiteralPath (Join-Path $install $name)){Copy-Item -LiteralPath (Join-Path $install $name) -Destination (Join-Path $backup $name)}}
if(Test-Path -LiteralPath $link){Copy-Item -LiteralPath $link -Destination (Join-Path $backup 'WiniOS.lnk')}
$run=[Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($runPath)
$originalStartup=$run.GetValue('WiniOS',$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$originalKind=if($null -ne $originalStartup){$run.GetValueKind('WiniOS')}else{$null}
if($null -ne $originalStartup){Assert ($originalKind -eq [Microsoft.Win32.RegistryValueKind]::String -and $originalStartup -match '^"[A-Za-z]:\\[^"\r\n]+\\winios\.exe" --startup$') 'Existing foreign startup value must be reviewed before lifecycle testing.'}
[pscustomobject]@{StartupWasRegistered=$null -ne $originalStartup;StartupCommand=$originalStartup;InstalledPath=$exe} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $backup 'restore-metadata.json')
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices; using System.Security.Cryptography;
public static class WiniOSInstallerCredentialFingerprint {
 [StructLayout(LayoutKind.Sequential)] struct Credential { public uint Flags,Type; public IntPtr Target,Comment; public long Written; public uint Size; public IntPtr Blob; public uint Persist,Count; public IntPtr Attributes,Alias,User; }
 [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool CredRead(string target,uint type,uint flags,out IntPtr pointer);
 [DllImport("advapi32.dll")] static extern void CredFree(IntPtr pointer);
 public static string Read(){IntPtr p; if(!CredRead("WiniOS/Agent/Bearer/v1",1,0,out p)){if(Marshal.GetLastWin32Error()==1168)return "missing";throw new Exception("Credential fingerprint unavailable.");}
  try{var c=Marshal.PtrToStructure<Credential>(p);if(c.Size!=64||c.Blob==IntPtr.Zero)throw new Exception("Unexpected credential format.");var b=new byte[c.Size];try{Marshal.Copy(c.Blob,b,0,b.Length);return Convert.ToHexString(SHA256.HashData(b));}finally{Array.Clear(b,0,b.Length);}}finally{CredFree(p);}}
}
'@
function Serve-Digest {
    $start=[Diagnostics.ProcessStartInfo]::new();$start.FileName=Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe';$start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    foreach($arg in @('serve','status','--json')){[void]$start.ArgumentList.Add($arg)}
    $process=[Diagnostics.Process]::new();$process.StartInfo=$start
    try{[void]$process.Start();$out=$process.StandardOutput.ReadToEndAsync();$err=$process.StandardError.ReadToEndAsync();if(-not $process.WaitForExit(5000)){throw 'Read-only Serve query timed out.'};Assert ($process.ExitCode -eq 0) 'Read-only Serve query failed.';return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($out.GetAwaiter().GetResult())))}finally{$process.Dispose()}
}
function Protected-Snapshot {
    $result=@{}
    foreach($file in Get-ChildItem -LiteralPath $personal -Recurse -File){$result[$file.FullName]=(Get-FileHash -LiteralPath $file.FullName).Hash}
    $result.credential=[WiniOSInstallerCredentialFingerprint]::Read()
    $result.serve=Serve-Digest
    $approved=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run')
    try{ $value=if($approved){$approved.GetValue('WiniOS',$null)}else{$null};$result.startupApproved=if($value -is [byte[]]){[Convert]::ToHexString($value)}else{[string]$value} }finally{if($approved){$approved.Dispose()}}
    return $result
}
$protected=Protected-Snapshot
function Assert-Protected {
    $after=Protected-Snapshot;Assert ($protected.Count -eq $after.Count) 'Protected personal file count changed.'
    foreach($key in $protected.Keys){Assert ($protected[$key] -ceq $after[$key]) 'Protected settings/TLS/credential/Serve/StartupApproved changed.'}
}
function Execute([string]$Image,[string[]]$Arguments,[string]$Phase){
    $start=[Diagnostics.ProcessStartInfo]::new();$start.FileName=$Image;$start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.WorkingDirectory=Split-Path $Image
    foreach($arg in $Arguments){[void]$start.ArgumentList.Add($arg)}
    $process=[Diagnostics.Process]::new();$process.StartInfo=$start
    try{[void]$process.Start();if(-not $process.WaitForExit(60000)){throw "$Phase timed out; process was not terminated."};return $process.ExitCode}finally{$process.Dispose()}
}
function Install([string]$Image,[string]$Phase,[string[]]$Extra=@()){
    $launchArguments=@('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-',('/LOG='+ (Join-Path $backup "$Phase.log")))+$Extra
    Assert ((Execute $Image $launchArguments $Phase) -eq 0) "$Phase failed; see its metadata-only installer log."
    Assert-Protected
}
function Arp-Value([string]$Name){$key=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($arp);try{if($key){return $key.GetValue($Name,$null)}}finally{if($key){$key.Dispose()}}}
function Assert-Installed([string]$Version){
    Assert ((Arp-Value 'DisplayName') -ceq 'WiniOS') 'Installed Apps registration missing.'
    Assert ((Arp-Value 'DisplayVersion') -ceq $Version) 'Wrong Installed Apps version.'
    Assert (Test-Path -LiteralPath (Join-Path $install 'unins000.exe')) 'Uninstaller missing.'
    Assert (Test-Path -LiteralPath $link) 'Start Menu shortcut missing.'
    $shell=New-Object -ComObject WScript.Shell;$shortcut=$shell.CreateShortcut($link)
    Assert ($shortcut.TargetPath -ieq $exe -and $shortcut.Arguments -ceq '--tray' -and $shortcut.WorkingDirectory -ieq $install) 'Shortcut target/arguments/working directory incorrect.'
    foreach($entry in $manifest.Payload){
        $relative=if($entry.Name-in @('winios.exe','winios-agent.exe','winios.ico','INSTALLATION.txt')){$entry.Name}else{Join-Path 'docs' $entry.Name}
        Assert ((Get-FileHash -LiteralPath (Join-Path $install $relative)).Hash -ceq $entry.SHA256) 'Installed payload hash mismatch.'
    }
}
function Wait-Agent {
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    do{ $agents=@(Get-Process -Name winios,winios-agent -ErrorAction SilentlyContinue);if($agents.Count){Assert ($agents.Count -eq 1 -and $agents[0].Path -ieq $exe) 'Duplicate/unexpected agent found.';return $agents[0]};Start-Sleep -Milliseconds 100 }while([DateTime]::UtcNow -lt $deadline)
    throw 'Expected installed agent did not launch.'
}
function Stop-Agent {
    $agents=@(Get-Process -Name winios,winios-agent -ErrorAction SilentlyContinue)
    if(-not $agents.Count){return}
    Assert ($agents.Count -eq 1 -and $agents[0].Path -ieq $exe) 'Unexpected process; refusing to stop it.'
    Assert ((Execute $exe @('--quit') 'graceful-quit') -eq 0) 'Quit request failed.'
    Assert ($agents[0].WaitForExit(20000)) 'Agent did not exit gracefully.'
}
$sentinel=Join-Path $install "unowned-$stamp.txt"
$completed=$false
try{
    [void](New-Item -ItemType Directory -Path $install -Force)
    Set-Content -LiteralPath $sentinel -Value 'Disposable unowned-file preservation fixture.'
    Install $PreviousSetupPath 'initial-install' @('/LAUNCH=1')
    Assert-Installed $previous.PackageVersion
    $oldAgent=Wait-Agent
    # Explicitly exercise recognized stale-path repair; do not execute this value.
    $run.SetValue('WiniOS','"C:\WiniOS-Installer-OldPath\winios.exe" --startup',[Microsoft.Win32.RegistryValueKind]::String)
    Install $SetupPath 'upgrade' @('/LAUNCH=1')
    Assert-Installed $manifest.PackageVersion
    Assert ($oldAgent.WaitForExit(5000)) 'Upgrade did not stop the old agent.'
    $newAgent=Wait-Agent;Assert ($newAgent.Id -ne $oldAgent.Id) 'Upgrade reused the old process.'
    Assert ($run.GetValue('WiniOS') -ceq ('"'+$exe+'" --startup')) 'Startup was not repaired to the fixed installed path.'
    Install $SetupPath 'reinstall' @('/LAUNCH=1')
    Assert-Installed $manifest.PackageVersion;[void](Wait-Agent)
    Stop-Agent
    $before=(Get-FileHash -LiteralPath $exe).Hash
    $created=$false;$mutex=[Threading.Mutex]::new($false,'Local\WiniOS.Agent',[ref]$created)
    try{Assert $created 'Test agent mutex unexpectedly existed.';$code=Execute $SetupPath @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-') 'blocked-agent';Assert ($code -ne 0) 'Installer overwrote a busy agent.';Assert ((Get-FileHash -LiteralPath $exe).Hash -ceq $before) 'Busy-instance failure changed the binary.'}finally{$mutex.Dispose()}
    $created=$false;$mutex=[Threading.Mutex]::new($false,'Local\WiniOS.Agent',[ref]$created)
    try{Assert $created 'Uninstall guard fixture mutex unexpectedly existed.';$code=Execute (Join-Path $install 'unins000.exe') @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART') 'blocked-uninstall';Assert ($code -ne 0) 'Uninstaller accepted a busy instance.';Assert ((Get-FileHash -LiteralPath $exe).Hash -ceq $before) 'Blocked uninstall changed the app.';Assert ((Arp-Value 'DisplayName') -ceq 'WiniOS') 'Blocked uninstall removed registration.'}finally{$mutex.Dispose()}
    $code=Execute $SetupPath @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-',('/DIR='+ (Join-Path $backup 'rejected-path'))) 'wrong-directory'
    Assert ($code -ne 0) 'Nonstandard installation path was accepted.'
    $saved=$run.GetValue('WiniOS');$run.SetValue('WiniOS','foreign command: preserve me',[Microsoft.Win32.RegistryValueKind]::String)
    try{$code=Execute $SetupPath @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-') 'startup-conflict';Assert ($code -ne 0 -and $run.GetValue('WiniOS') -ceq 'foreign command: preserve me') 'Foreign startup registration was overwritten.'}finally{$run.SetValue('WiniOS',$saved,[Microsoft.Win32.RegistryValueKind]::String)}
    # Exercise uninstall while the real app is running.
    [void](Start-Process -FilePath $link -WindowStyle Hidden -PassThru)
    $uninstallAgent=Wait-Agent
    $uninstaller=Join-Path $install 'unins000.exe'
    Assert ((Execute $uninstaller @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/LOG='+ (Join-Path $backup 'uninstall.log'))) 'uninstall') -eq 0) 'Uninstall failed.'
    Assert ($uninstallAgent.WaitForExit(5000)) 'Uninstall left the agent running.'
    $deadline=[DateTime]::UtcNow.AddSeconds(5)
    while((Test-Path -LiteralPath $uninstaller) -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 100}
    Assert (-not (Test-Path -LiteralPath $exe)) 'GUI binary survived uninstall.'
    Assert (-not (Test-Path -LiteralPath (Join-Path $install 'winios-agent.exe'))) 'CLI binary survived uninstall.'
    Assert (-not (Test-Path -LiteralPath $link)) 'Start Menu shortcut survived uninstall.'
    Assert ($null -eq (Arp-Value 'DisplayName')) 'Installed Apps registration survived uninstall.'
    Assert ($null -eq $run.GetValue('WiniOS',$null)) 'Startup registration survived uninstall.'
    Assert (Test-Path -LiteralPath $sentinel) 'Uninstaller deleted an unowned file.'
    Assert-Protected
    Install $SetupPath 'clean-program-reinstall' @('/LAUNCH=1')
    Assert-Installed $manifest.PackageVersion
    Assert ($null -eq $run.GetValue('WiniOS',$null)) 'Clean reinstall unexpectedly enabled startup.'
    [void](Wait-Agent)
    Assert-Protected
    $completed=$true
    [pscustomobject]@{Artifact=$SetupPath;PackageVersion=$manifest.PackageVersion;RealWindowsInstallUpgradeReinstallUninstall=$true;CleanProgramReinstallWithRetainedData=$true;StartMenuLaunch=$true;BusyInstanceAndWrongDirectoryRejected=$true;StartupRepairAndConflictProtection=$true;PersonalSettingsCredentialTlsServeAndStartupApprovedUnchanged=$true;Backup=$backup} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $backup 'results.json')
    Write-Output "PASS: real Windows silent install, upgrade, reinstall, Start Menu launch, graceful uninstall and clean program reinstall. Protected data/Serve/credential unchanged. Backup: $backup"
}catch{
    $failure=$_
    Write-Warning 'Lifecycle test failed. Attempting the verified current installer to leave WiniOS usable; protected data will not be overwritten.'
    try{Stop-Agent;Install $SetupPath 'recovery-install' @('/LAUNCH=1')}catch{Write-Warning 'Automatic recovery install failed. Retained binary backup and restore metadata require manual review.'}
    throw $failure
}finally{
    # Testing temporarily changes ONLY the WiniOS Run value. Restore the user's
    # original enabled/disabled choice to the correct installed path.
    if($null -ne $originalStartup){$run.SetValue('WiniOS',('"'+$exe+'" --startup'),[Microsoft.Win32.RegistryValueKind]::String)}else{$run.DeleteValue('WiniOS',$false)}
    $run.Dispose()
    # Only remove the exact disposable file created above, never a directory.
    if(Test-Path -LiteralPath $sentinel){Remove-Item -LiteralPath $sentinel}
}
