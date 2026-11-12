#Requires -Version 7.4
param([string]$Directory = (Join-Path $PSScriptRoot '..\target\release'))
$ErrorActionPreference = 'Stop'
function Get-Subsystem([string]$path) {
    $bytes = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $path))
    if ($bytes.Length -lt 128 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) { throw 'Not a Windows PE executable.' }
    $pe = [BitConverter]::ToInt32($bytes, 0x3c)
    if ($pe -lt 0 -or $pe + 94 -gt $bytes.Length -or [BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550) { throw 'Invalid PE header.' }
    return [BitConverter]::ToUInt16($bytes, $pe + 24 + 68)
}
if ((Get-Subsystem (Join-Path $Directory 'winios.exe')) -ne 2) { throw 'Desktop binary is not IMAGE_SUBSYSTEM_WINDOWS_GUI.' }
if ((Get-Subsystem (Join-Path $Directory 'winios-agent.exe')) -ne 3) { throw 'CLI binary is not IMAGE_SUBSYSTEM_WINDOWS_CUI.' }
Write-Output 'PASS: winios.exe is GUI-subsystem (no automatic console); winios-agent.exe retains the console subsystem.'
