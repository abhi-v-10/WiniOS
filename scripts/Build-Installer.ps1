#Requires -Version 7.4
[CmdletBinding()]
param(
    [string]$ISCCPath,
    [string]$OutputDirectory,
    [ValidatePattern('^\d+\.\d+\.\d+(\.\d+)?$')][string]$PackageVersion,
    [switch]$SkipRustBuild
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'Build the Windows installer on Windows.' }
$project = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'Installer.Common.ps1')
if (-not $ISCCPath) {
    $candidates = @((Join-Path $project 'target\tools\InnoSetup7\ISCC.exe'),(Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 7\ISCC.exe'),(Join-Path $env:ProgramFiles 'Inno Setup 7\ISCC.exe'))
    $ISCCPath = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if (-not $ISCCPath) { throw 'Install maintained Inno Setup 7 from the official site, verify its signature, then supply -ISCCPath <absolute ISCC.exe>.' }
$ISCCPath = (Resolve-Path -LiteralPath $ISCCPath).Path
$compilerVersion = (& $ISCCPath --version | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $compilerVersion -notmatch '\b7\.\d+\.\d+') { throw 'Inno Setup 7 is required.' }
$match = [regex]::Match((Get-Content -LiteralPath (Join-Path $project 'Cargo.toml') -Raw),'(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"')
if (-not $match.Success) { throw 'Cargo package version not found.' }
$applicationVersion = $match.Groups[1].Value
if (-not $PackageVersion) { $PackageVersion = $applicationVersion }
if ($PackageVersion -cne $applicationVersion) { Write-Warning 'Installer version override is for lifecycle fixtures only; application metadata is unchanged.' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $project "target\dist\$PackageVersion" }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$build = Join-Path $project 'target\installer-build'
$binary = Join-Path $build 'release'
if (-not $SkipRustBuild) {
    $oldFlags = $env:RUSTFLAGS
    $oldTarget = $env:CARGO_TARGET_DIR
    try {
        $env:RUSTFLAGS = '-C target-feature=+crt-static'
        $env:CARGO_TARGET_DIR = $build
        Push-Location -LiteralPath $project
        try {
            & cargo build --release --locked --offline
            if ($LASTEXITCODE -ne 0) { throw 'Static-CRT release build failed.' }
        } finally { Pop-Location }
    } finally { $env:RUSTFLAGS=$oldFlags; $env:CARGO_TARGET_DIR=$oldTarget }
}
Assert-WiniOSPayload $binary
Assert-WiniOSVersion $binary $applicationVersion
$stage = Join-Path $project "target\installer\stage\$PackageVersion"
[void](New-Item -ItemType Directory -Path $stage,$OutputDirectory -Force)
foreach ($name in @('winios.exe','winios-agent.exe')) { Copy-Item -LiteralPath (Join-Path $binary $name) -Destination (Join-Path $stage $name) -Force }
Copy-Item -LiteralPath (Join-Path $project 'resources\winios.ico') -Destination (Join-Path $stage 'winios.ico') -Force
foreach ($entry in @(@('installer\INSTALLATION.txt','INSTALLATION.txt'),@('installer\BETA-README.md','README.md'),@('docs\mission-010.md','mission-010.md'),@('docs\mission-014.md','mission-014.md'),@('docs\mission-015.md','mission-015.md'),@('docs\mission-018.md','mission-018.md'),@('docs\private-beta.md','private-beta.md'))) {
    $text = Get-Content -LiteralPath (Join-Path $project $entry[0]) -Raw
    # Distribute instructions, not machine-specific historical verification reports.
    if ($entry[1] -in @('mission-014.md','mission-015.md')) { $text = ($text -split '(?m)^## (?:Verification and deployment|Recorded verification)')[0] }
    if ($entry[1] -eq 'mission-010.md') { $text = ($text -split '(?m)^## Automated verification and rollback')[0] }
    [IO.File]::WriteAllText((Join-Path $stage $entry[1]), $text)
}
if (@(Compare-Object @(Get-WiniOSPackageFiles) @(Get-ChildItem -LiteralPath $stage | Select-Object -ExpandProperty Name)).Count) { throw 'Unexpected staging contents; only the reviewed payload may be packaged.' }
& (Join-Path $PSScriptRoot 'Test-PackagePrivacy.ps1') -Directory $stage
& $ISCCPath '--no-ide-signtools' "--define=PayloadDir=$stage" "--define=PackageVersion=$PackageVersion" "--output-dir=$OutputDirectory" (Join-Path $project 'installer\WiniOS.iss')
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup compilation failed.' }
$setup = Join-Path $OutputDirectory 'WiniOS-Setup.exe'
$payload = foreach ($name in Get-WiniOSPackageFiles) { [pscustomobject]@{Name=$name;SHA256=(Get-FileHash -LiteralPath (Join-Path $stage $name)).Hash} }
[pscustomobject]@{PackageVersion=$PackageVersion;ApplicationVersion=$applicationVersion;Compiler=$compilerVersion;StaticCrt=$true;InstallerSHA256=(Get-FileHash -LiteralPath $setup).Hash;Payload=$payload} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'WiniOS-Setup.manifest.json') -Encoding utf8
Copy-Item -LiteralPath (Join-Path $stage 'private-beta.md') -Destination (Join-Path $OutputDirectory 'INSTALLATION.md') -Force
Write-Output "BUILT: $setup"
