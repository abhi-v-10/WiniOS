# Shared, read-only PE inspection; no external SDK tool or test framework required.
Set-StrictMode -Version Latest
function Get-WiniOSPackageFiles {
    # Explicit installer ownership; never stage a settings file, key or token.
    return @('winios.exe','winios-agent.exe','winios.ico','INSTALLATION.txt','README.md',
        'mission-010.md','mission-014.md','mission-015.md','mission-018.md','private-beta.md')
}
function Assert-WiniOSVersion([string]$Directory, [string]$Version) {
    foreach ($name in @('winios.exe','winios-agent.exe')) {
        $info = [Diagnostics.FileVersionInfo]::GetVersionInfo((Join-Path $Directory $name))
        if ($info.FileVersion -cne $Version -or $info.ProductVersion -cne $Version -or
            $info.ProductName -cne 'WiniOS' -or $info.CompanyName -cne 'Abhishyanth') {
            throw "Stale or incorrect Windows version/branding metadata: $name"
        }
    }
}
function Get-WiniOSPeDetails([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Path))
    if ($bytes.Length -lt 256 -or [BitConverter]::ToUInt16($bytes,0) -ne 0x5a4d) { throw 'Not a PE image.' }
    $pe = [BitConverter]::ToInt32($bytes,0x3c)
    if ($pe -lt 0 -or $pe + 256 -gt $bytes.Length -or [BitConverter]::ToUInt32($bytes,$pe) -ne 0x4550) { throw 'Invalid PE header.' }
    $optional = $pe + 24
    $magic = [BitConverter]::ToUInt16($bytes,$optional)
    $data = switch ($magic) { 0x20b { $optional + 112 } 0x10b { $optional + 96 } default { throw 'Unknown PE optional header.' } }
    $sectionCount = [BitConverter]::ToUInt16($bytes,$pe + 6)
    $sections = $optional + [BitConverter]::ToUInt16($bytes,$pe + 20)
    function Find-Offset([uint32]$Rva) {
        for ($n=0; $n -lt $sectionCount; $n++) {
            $s = $sections + $n*40
            if ($s+40 -gt $bytes.Length) { throw 'Invalid PE section table.' }
            $virtual = [BitConverter]::ToUInt32($bytes,$s+12)
            $size = [math]::Max([BitConverter]::ToUInt32($bytes,$s+8),[BitConverter]::ToUInt32($bytes,$s+16))
            if ($Rva -ge $virtual -and $Rva -lt $virtual+$size) { return [int]($Rva-$virtual+[BitConverter]::ToUInt32($bytes,$s+20)) }
        }
        throw 'PE RVA outside sections.'
    }
    $imports = [Collections.Generic.List[string]]::new()
    $rva = [BitConverter]::ToUInt32($bytes,$data+8)
    if ($rva) {
        $entry = Find-Offset $rva
        for ($n=0; $n -lt 512; $n++) {
            if ($entry+20 -gt $bytes.Length) { throw 'Invalid PE import table.' }
            $nameRva = [BitConverter]::ToUInt32($bytes,$entry+12)
            if (-not $nameRva) { break }
            $start = Find-Offset $nameRva
            $end = $start
            while ($end -lt $bytes.Length -and $end-$start -lt 256 -and $bytes[$end]) { $end++ }
            if ($end -ge $bytes.Length -or $end-$start -ge 256) { throw 'Invalid PE import name.' }
            $imports.Add([Text.Encoding]::ASCII.GetString($bytes,$start,$end-$start))
            $entry += 20
        }
    }
    [pscustomobject]@{ Machine=[BitConverter]::ToUInt16($bytes,$pe+4); Subsystem=[BitConverter]::ToUInt16($bytes,$optional+68); Imports=$imports.ToArray() }
}
function Assert-WiniOSPayload([string]$Directory) {
    foreach ($entry in @(@('winios.exe',2),@('winios-agent.exe',3))) {
        $details = Get-WiniOSPeDetails (Join-Path $Directory $entry[0])
        if ($details.Machine -ne 0x8664 -or $details.Subsystem -ne $entry[1]) { throw "Wrong architecture/subsystem: $($entry[0])" }
        if (@($details.Imports | Where-Object { $_ -match '^(vcruntime|msvcp|msvcr\d|ucrtbased)' }).Count) { throw "Dynamic/development VC runtime dependency in $($entry[0]); build the shipping payload with static CRT." }
    }
}
