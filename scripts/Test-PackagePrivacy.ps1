# Read-only scan of the explicit staged payload; never prints matching values.
param([Parameter(Mandatory)][string]$Directory)
$ErrorActionPreference='Stop'
Add-Type -TypeDefinition @'
using System; using System.Text; using System.Runtime.InteropServices;
public static class WiniOSPackageSecretAudit {
 public static bool ContainsBytes(byte[] candidate, byte[] pattern) { return pattern.Length>32 && candidate.AsSpan().IndexOf(pattern.AsSpan())>=0; }
 [StructLayout(LayoutKind.Sequential)] struct Credential { public uint Flags,Type; public IntPtr Target,Comment; public long Written; public uint Size; public IntPtr Blob; public uint Persist,Count; public IntPtr Attributes,Alias,User; }
 [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool CredRead(string target,uint type,uint flags,out IntPtr pointer);
 [DllImport("advapi32.dll")] static extern void CredFree(IntPtr pointer);
 public static bool ContainsLiveToken(byte[] candidate) {
  IntPtr p; if(!CredRead("WiniOS/Agent/Bearer/v1",1,0,out p)){if(Marshal.GetLastWin32Error()==1168)return false;throw new Exception("Credential audit unavailable");}
  byte[] b=null,u=null;
  try {var c=Marshal.PtrToStructure<Credential>(p);if(c.Size!=64||c.Blob==IntPtr.Zero)throw new Exception("Unexpected credential format");b=new byte[c.Size];Marshal.Copy(c.Blob,b,0,b.Length);u=Encoding.Unicode.GetBytes(Encoding.ASCII.GetString(b));return candidate.AsSpan().IndexOf(b.AsSpan())>=0||candidate.AsSpan().IndexOf(u.AsSpan())>=0;}
  finally{if(b!=null)Array.Clear(b,0,b.Length);if(u!=null)Array.Clear(u,0,u.Length);CredFree(p);}
 }
}
'@
$hostname=$null
$cli=Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
if(Test-Path -LiteralPath $cli){
 $start=[Diagnostics.ProcessStartInfo]::new();$start.FileName=$cli;$start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
 foreach($arg in @('status','--json','--peers=false')){[void]$start.ArgumentList.Add($arg)}
 $p=[Diagnostics.Process]::Start($start)
 try{$out=$p.StandardOutput.ReadToEndAsync();$err=$p.StandardError.ReadToEndAsync();if(-not $p.WaitForExit(5000)-or $p.ExitCode-ne 0){throw 'Read-only hostname audit failed'};$hostname=([string](($out.GetAwaiter().GetResult()|ConvertFrom-Json).Self.DNSName)).TrimEnd('.')}finally{$p.Dispose()}
}
foreach($file in Get-ChildItem -LiteralPath $Directory -File){
 $bytes=[IO.File]::ReadAllBytes($file.FullName)
 if([WiniOSPackageSecretAudit]::ContainsLiveToken($bytes)){throw 'Live credential detected in payload; packaging blocked'}
 $text=[Text.Encoding]::UTF8.GetString($bytes);$wide=[Text.Encoding]::Unicode.GetString($bytes)
 if(($hostname-and ($text.Contains($hostname)-or $wide.Contains($hostname)))-or ($file.Extension-ne '.exe'-and $text-match '-----BEGIN (?:[A-Z ]*PRIVATE KEY|CERTIFICATE)-----')-or $file.Extension-in @('.pem','.key','.pfx','.p12','.cer','.crt')){throw 'Private identity/certificate material detected; packaging blocked'}
 $tls=Join-Path $env:LOCALAPPDATA 'WiniOS\tls'
 if(Test-Path -LiteralPath $tls){foreach($identity in Get-ChildItem -LiteralPath $tls -File -Recurse){if([WiniOSPackageSecretAudit]::ContainsBytes($bytes,[IO.File]::ReadAllBytes($identity.FullName))){throw 'Live TLS identity material detected; packaging blocked'}}}
}
Write-Output 'PASS: staged payload has no live token, current personal hostname or PEM/certificate files. This is a focused scan, not proof against every possible secret encoding.'
