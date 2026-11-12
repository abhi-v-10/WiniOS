#Requires -Version 7.4
# Overwrites the system clipboard with disposable fixtures. No bodies, tokens,
# version identifiers or image files are logged/persisted. No network changes.
param([ValidateRange(1,65535)][int]$BackendPort=8766,[switch]$ExpectPaused)
$ErrorActionPreference='Stop'
if(-not$IsWindows){throw 'Windows is required.'}
& (Join-Path $PSScriptRoot 'Test-Tailnet.ps1') -BackendPort $BackendPort -ExpectPaused:$ExpectPaused
$start=[Diagnostics.ProcessStartInfo]::new();$start.FileName=Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
$start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
foreach($arg in @('status','--json','--peers=false')){[void]$start.ArgumentList.Add($arg)}
$process=[Diagnostics.Process]::Start($start)
try{$out=$process.StandardOutput.ReadToEndAsync();$err=$process.StandardError.ReadToEndAsync();if(-not$process.WaitForExit(5000)-or$process.ExitCode-ne 0){throw 'Cannot verify local tailnet identity.'};$status=$out.GetAwaiter().GetResult()|ConvertFrom-Json}finally{$process.Dispose()}
$name=([string]$status.Self.DNSName).TrimEnd('.');$suffix=[string]$status.CurrentTailnet.MagicDNSSuffix
if($status.BackendState-ne 'Running'-or-not$status.Self.Online-or-not$status.CurrentTailnet.MagicDNSEnabled-or$name-notmatch '^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.[a-z0-9-]+\.ts\.net$'-or-not$name.EndsWith(".$suffix",[StringComparison]::Ordinal)-or$status.CertDomains-cnotcontains$name){throw 'Connected verified tailnet HTTPS identity required.'}
$base=[uri]"https://$name/"
foreach($entry in @(@('WiniOSPostTestNative','Test-ClipboardPost.ps1'),@('WiniOSImageTestNative','Test-ClipboardImages.ps1'))){
 if(-not($entry[0]-as[type])){
  $source=Get-Content (Join-Path $PSScriptRoot $entry[1]) -Raw
  $helper=[regex]::Match($source,"(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")
  if(-not$helper.Success){throw 'Required native test helper missing.'}
  Add-Type -TypeDefinition $helper.Groups[1].Value
 }
}
$handler=[Net.Http.SocketsHttpHandler]::new();$handler.UseProxy=$false;$handler.AllowAutoRedirect=$false
$client=[Net.Http.HttpClient]::new($handler);$client.Timeout=[TimeSpan]::FromSeconds(15)
$token=[WiniOSPostTestNative]::Token()
function Request([string]$Method,[string]$Path,[byte[]]$Bytes=$null,[string]$Type='',[string]$Version='',[bool]$Authenticated=$true){
 $request=[Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::new($Method),[uri]::new($base,$Path))
 try{
  if($Authenticated){$request.Headers.Authorization=[Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer',$token)}
  if($Version){[void]$request.Headers.TryAddWithoutValidation('X-WiniOS-Clipboard-Version',$Version)}
  if($Method-eq 'POST'){$request.Content=[Net.Http.ByteArrayContent]::new($Bytes);$request.Content.Headers.ContentType=[Net.Http.Headers.MediaTypeHeaderValue]::new($Type)}
  return $client.SendAsync($request).GetAwaiter().GetResult()
 }finally{$request.Dispose()}
}
function Assert-Status($Response,[int]$Expected){
 if([int]$Response.StatusCode-ne$Expected-or-not$Response.Headers.CacheControl.NoStore){throw "Expected no-store HTTP $Expected, received $([int]$Response.StatusCode)."}
}
function Get-Metadata([string]$Kind){
 $deadline=[DateTime]::UtcNow.AddSeconds(4)
 do{
  $response=Request 'GET' 'v1/clipboard/type'
  try{
   if([int]$response.StatusCode-eq 200){
    Assert-Status $response 200
    $bytes=$response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
    if($bytes.Length-gt 256){throw 'Metadata is unexpectedly large.'}
    try{$meta=[Text.Encoding]::UTF8.GetString($bytes)|ConvertFrom-Json}catch{throw 'Invalid metadata JSON.'}
    if(@($meta.PSObject.Properties).Count-ne 2-or$meta.kind-notin @('text','image','empty')-or$meta.version-notmatch '^w1-[0-9a-f]{32}-[0-9a-f]{16}$'){throw 'Metadata schema mismatch.'}
    if($meta.kind-eq$Kind){return $meta}
   }elseif([int]$response.StatusCode-notin @(409,503)){throw 'Metadata request rejected.'}
  }finally{$response.Dispose()}
  Start-Sleep -Milliseconds 20
 }while([DateTime]::UtcNow-lt$deadline)
 throw 'Expected current clipboard kind did not become available.'
}
function Put-Text([string]$Text){
 $json=ConvertTo-Json -Compress @{text=$Text}
 $response=Request 'POST' 'v1/clipboard' ([Text.Encoding]::UTF8.GetBytes($json)) 'application/json'
 try{Assert-Status $response 200}finally{$response.Dispose()}
}
try{
 $response=Request 'GET' 'v1/clipboard/type' -Authenticated $false
 try{Assert-Status $response 401}finally{$response.Dispose()}
 if($ExpectPaused){$response=Request 'GET' 'v1/clipboard/type';try{Assert-Status $response 503}finally{$response.Dispose()};Write-Output 'PASS: paused type endpoint=503 authenticated, 401 unauthorized; no clipboard writes.';return}
 $text="WiniOS smart receive disposable 😀 café`r`n"
 Put-Text $text;$meta=Get-Metadata 'text';$oldTextVersion=$meta.version
 $response=Request 'GET' 'v1/clipboard' -Version $meta.version
 try{Assert-Status $response 200;if($response.Content.ReadAsStringAsync().GetAwaiter().GetResult()-cne$text){throw 'Bound Unicode text mismatch.'}}finally{$response.Dispose()}
 Add-Type -AssemblyName System.Drawing
 $bitmap=[Drawing.Bitmap]::new(2,2);$stream=[IO.MemoryStream]::new()
 try{$bitmap.SetPixel(0,0,[Drawing.Color]::Red);$bitmap.SetPixel(1,0,[Drawing.Color]::Green);$bitmap.SetPixel(0,1,[Drawing.Color]::Blue);$bitmap.SetPixel(1,1,[Drawing.Color]::White);$bitmap.Save($stream,[Drawing.Imaging.ImageFormat]::Png);$png=$stream.ToArray()}finally{$bitmap.Dispose();$stream.Dispose()}
 $response=Request 'POST' 'v1/clipboard/image' $png 'image/png';try{Assert-Status $response 200}finally{$response.Dispose()}
 $meta=Get-Metadata 'image';$oldImageVersion=$meta.version
 $response=Request 'GET' 'v1/clipboard' -Version $oldTextVersion
 try{Assert-Status $response 412;if($response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult().Length){throw 'Failed precondition returned content.'}}finally{$response.Dispose()}
 $response=Request 'GET' 'v1/clipboard/image' -Version $meta.version
 try{
  Assert-Status $response 200;if($response.Content.Headers.ContentType.MediaType-ne 'image/png'){throw 'Wrong image type.'}
  $stream=[IO.MemoryStream]::new($response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult(),$false);$received=[Drawing.Bitmap]::new($stream)
  try{if($received.Width-ne 2-or$received.Height-ne 2-or$received.GetPixel(0,0).ToArgb()-ne[Drawing.Color]::Red.ToArgb()-or$received.GetPixel(1,1).ToArgb()-ne[Drawing.Color]::White.ToArgb()){throw 'Bound PNG fixture mismatch.'}}finally{$received.Dispose();$stream.Dispose()}
 }finally{$response.Dispose()}
 $response=Request 'GET' 'v1/clipboard/image' -Version $meta.version -Authenticated $false
 try{Assert-Status $response 401}finally{$response.Dispose()}
 Put-Text $text;[void](Get-Metadata 'text')
 $response=Request 'GET' 'v1/clipboard/image' -Version $oldImageVersion
 try{Assert-Status $response 412}finally{$response.Dispose()}
 # Same-kind, identical recopy is still a new native observation/version.
 for($i=0;$i-lt 5;$i++){$meta=Get-Metadata 'text';Put-Text $text;$response=Request 'GET' 'v1/clipboard' -Version $meta.version;try{Assert-Status $response 412}finally{$response.Dispose()}}
 [WiniOSImageTestNative]::Put(0,$null,[IntPtr]::Zero);[void](Get-Metadata 'empty')
 [WiniOSImageTestNative]::Put([WiniOSImageTestNative]::RegisterClipboardFormatW('WiniOS.SmartReceive.Disposable'),[byte[]](1,2),[IntPtr]::Zero);[void](Get-Metadata 'empty')
 Put-Text 'WiniOS smart receive tests complete'
 Write-Output 'PASS: verified HTTPS smart metadata/auth/no-store; bound Unicode and PNG; stale cross-kind/same-kind rejection; empty/unsupported states; legacy APIs retained. No contents or version IDs logged.'
}finally{$token=$null;$client.Dispose();$handler.Dispose();[WiniOSImageTestNative]::Cleanup()}
