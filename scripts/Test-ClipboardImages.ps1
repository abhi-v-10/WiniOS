#Requires -Version 7.4
# Overwrites the clipboard with disposable fixtures. Never stores image files,
# prints bodies/tokens, changes trust, or modifies Tailscale/Serve/firewall.
param([ValidateRange(1,65535)][int]$BackendPort=8766)
$ErrorActionPreference='Stop'
if(-not $IsWindows){throw 'Windows is required.'}
& (Join-Path $PSScriptRoot 'Test-Tailnet.ps1') -BackendPort $BackendPort
$start=[Diagnostics.ProcessStartInfo]::new();$start.FileName=Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe';$start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
foreach($argument in @('status','--json','--peers=false')){[void]$start.ArgumentList.Add($argument)}
$process=[Diagnostics.Process]::new();$process.StartInfo=$start
try{[void]$process.Start();$output=$process.StandardOutput.ReadToEndAsync();$errors=$process.StandardError.ReadToEndAsync();if(-not $process.WaitForExit(5000)-or$process.ExitCode-ne 0){throw 'Cannot verify local tailnet identity.'};$status=$output.GetAwaiter().GetResult()|ConvertFrom-Json}finally{$process.Dispose()}
$hostName=([string]$status.Self.DNSName).TrimEnd('.')
if($status.BackendState-ne 'Running'-or-not$status.Self.Online-or$status.CertDomains-cnotcontains$hostName){throw 'Connected HTTPS tailnet identity is required.'}
$base=[uri]"https://$hostName/"
$source=Get-Content -LiteralPath (Join-Path $PSScriptRoot 'Test-ClipboardPost.ps1') -Raw
$helper=[regex]::Match($source,"(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")
if(-not $helper.Success){throw 'Existing credential/native test helper missing.'}
if(-not ('WiniOSPostTestNative' -as [type])){Add-Type -TypeDefinition $helper.Groups[1].Value}
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class WiniOSImageTestNative {
 static IntPtr OwnerWindow;
 static bool Acquire(IntPtr owner){for(int attempt=0;attempt<=6;attempt++){if(OpenClipboard(owner))return true;if(attempt<6)System.Threading.Thread.Sleep(20<<attempt);}return false;}
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr CreateWindowExW(uint ex,string cls,string name,uint style,int x,int y,int w,int h,IntPtr parent,IntPtr menu,IntPtr instance,IntPtr param);
 [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr window);
 [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr owner);
 [DllImport("user32.dll")] static extern bool CloseClipboard();
 [DllImport("user32.dll")] static extern bool EmptyClipboard();
 [DllImport("user32.dll")] static extern IntPtr SetClipboardData(uint format,IntPtr memory);
 [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint format);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern uint RegisterClipboardFormatW(string name);
 [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint flags,UIntPtr size);
 [DllImport("kernel32.dll")] static extern IntPtr GlobalFree(IntPtr memory);
 [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr memory);
 [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr memory);
 [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr memory);
 [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr value);
 [DllImport("gdi32.dll")] static extern bool GdiFlush();
 [DllImport("gdi32.dll")] static extern IntPtr CreateBitmap(int width,int height,uint planes,uint bits,byte[] pixels);
 public static IntPtr FixtureBitmap(){var pixels=new byte[8*6*4];for(int y=0;y<6;y++)for(int x=0;x<8;x++){int i=(y*8+x)*4;pixels[i]=(byte)((x+y)*17);pixels[i+1]=(byte)(y*41);pixels[i+2]=(byte)(x*29);pixels[i+3]=255;}var bitmap=CreateBitmap(8,6,1,32,pixels);if(bitmap==IntPtr.Zero)throw new Exception("Native fixture bitmap creation failed.");return bitmap;}
 public static void Put(uint format,byte[] bytes,IntPtr bitmap) {
  IntPtr owner=IntPtr.Zero,memory=bitmap;bool opened=false;bool isBitmap=bitmap!=IntPtr.Zero;
  try {
   if(isBitmap && !GdiFlush())throw new Exception("Fixture GDI flush failed.");
   if(OwnerWindow==IntPtr.Zero)OwnerWindow=CreateWindowExW(0,"STATIC","WiniOS disposable image fixture",0,0,0,0,0,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero);
   owner=OwnerWindow;
   if(owner==IntPtr.Zero)throw new Exception("Fixture owner creation failed.");
   if(!isBitmap && bytes!=null){memory=GlobalAlloc(0x42,(UIntPtr)bytes.Length);if(memory==IntPtr.Zero)throw new Exception("Fixture allocation failed.");var p=GlobalLock(memory);if(p==IntPtr.Zero)throw new Exception("Fixture lock failed.");try{Marshal.Copy(bytes,0,p,bytes.Length);}finally{GlobalUnlock(memory);}}
   if(!Acquire(owner))throw new Exception("Fixture clipboard contention.");opened=true;
   if(!EmptyClipboard())throw new Exception("Fixture clear failed.");
   if(memory!=IntPtr.Zero){if(SetClipboardData(format,memory)==IntPtr.Zero)throw new Exception("Fixture write failed.");memory=IntPtr.Zero;}
  }finally{if(opened)CloseClipboard();if(memory!=IntPtr.Zero){if(isBitmap)DeleteObject(memory);else GlobalFree(memory);}}
 }
 public static void Cleanup(){if(OwnerWindow!=IntPtr.Zero){DestroyWindow(OwnerWindow);OwnerWindow=IntPtr.Zero;}}
 public static byte[] Read(uint format) {
  if(!Acquire(IntPtr.Zero))throw new Exception("Fixture read contention.");
  try {var h=GetClipboardData(format);if(h==IntPtr.Zero)throw new Exception("Required native image format missing.");ulong size=GlobalSize(h).ToUInt64();if(size==0||size>33558528)throw new Exception("Invalid native fixture size.");var p=GlobalLock(h);if(p==IntPtr.Zero)throw new Exception("Fixture read lock failed.");try{var bytes=new byte[(int)size];Marshal.Copy(p,bytes,0,bytes.Length);return bytes;}finally{GlobalUnlock(h);}}
  finally{CloseClipboard();}
 }
}
'@
$handler=[Net.Http.SocketsHttpHandler]::new();$handler.UseProxy=$false;$handler.AllowAutoRedirect=$false
$client=[Net.Http.HttpClient]::new($handler);$client.Timeout=[TimeSpan]::FromSeconds(15)
$token=[WiniOSPostTestNative]::Token()
function Request-Image([string]$Method,[byte[]]$Bytes,[bool]$Authenticated=$true,[string]$ContentType='image/png'){
 $request=[Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::new($Method),[uri]::new($base,'v1/clipboard/image'))
 try{if($Authenticated){$request.Headers.Authorization=[Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer',$token)}
  if($Method-eq 'POST'){$request.Content=[Net.Http.ByteArrayContent]::new($Bytes);$request.Content.Headers.ContentType=[Net.Http.Headers.MediaTypeHeaderValue]::new($ContentType)}
  return $client.SendAsync($request).GetAwaiter().GetResult()
 }finally{$request.Dispose()}
}
$fixture=[Drawing.Bitmap]::new(8,6)
for($y=0;$y-lt 6;$y++){for($x=0;$x-lt 8;$x++){$fixture.SetPixel($x,$y,[Drawing.Color]::FromArgb(255,$x*29,$y*41,($x+$y)*17))}}
$stream=[IO.MemoryStream]::new();$fixture.Save($stream,[Drawing.Imaging.ImageFormat]::Png);$png=$stream.ToArray();$stream.Dispose()
function Assert-Png([byte[]]$Bytes){
 $stream=[IO.MemoryStream]::new($Bytes,$false);$bitmap=$null
 try{$bitmap=[Drawing.Bitmap]::new($stream);if($bitmap.Width-ne 8-or$bitmap.Height-ne 6){throw 'Image dimension mismatch.'};for($y=0;$y-lt 6;$y++){for($x=0;$x-lt 8;$x++){if($bitmap.GetPixel($x,$y).ToArgb()-ne$fixture.GetPixel($x,$y).ToArgb()){throw 'Image pixel mismatch.'}}}}
 finally{if($bitmap){$bitmap.Dispose()};$stream.Dispose()}
}
function Assert-Get([int]$Expected=200){
 $deadline=[DateTime]::UtcNow.AddSeconds(6);$lastStatus=0
 do{$response=Request-Image 'GET' $null
  try{$lastStatus=[int]$response.StatusCode;if($lastStatus-eq$Expected){if(-not$response.Headers.CacheControl.NoStore){throw 'Image response must prohibit caching.'};if($Expected-eq 200){if($response.Content.Headers.ContentType.MediaType-ne 'image/png'){throw 'Wrong image media type.'};Assert-Png ($response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult())};return};if($lastStatus-eq 403){throw 'Enable Images, apply/restart, and allow clipboard writes before this test.'}}
  finally{$response.Dispose()};Start-Sleep -Milliseconds 30
 }while([DateTime]::UtcNow-lt$deadline)
 throw "Expected HTTP $Expected image state did not arrive (last HTTP $lastStatus)."
}
try{
 foreach($method in @('GET','POST')){$response=Request-Image $method $png $false;try{if([int]$response.StatusCode-ne 401){throw 'Image authentication failed.'}}finally{$response.Dispose()}}
 foreach($test in @(@('image/jpeg',$png,415),@('image/png',[byte[]](1,2,3),422),@('image/png',[byte[]]::new(8MB+1),413))){$response=Request-Image 'POST' $test[1] $true $test[0];try{if([int]$response.StatusCode-ne$test[2]){throw 'Image validation rejection mismatch.'}}finally{$response.Dispose()}}
 Write-Output 'Image fixture: acknowledged PNG POST -> GET'
 $response=Request-Image 'POST' $png;try{if([int]$response.StatusCode-ne 200){throw 'Native image write acknowledgement failed.'}}finally{$response.Dispose()};Assert-Get
 $v5=[WiniOSImageTestNative]::Read(17);$dib=[WiniOSImageTestNative]::Read(8)
 if([BitConverter]::ToUInt32($v5,0)-ne 124-or[BitConverter]::ToInt32($v5,4)-ne 8-or[BitConverter]::ToInt32($v5,8)-ne 6){throw 'CF_DIBV5 layout mismatch.'}
 foreach($test in @(@(17,$v5),@(8,$dib))){Write-Output "Image fixture: native format $($test[0]) -> GET";[WiniOSImageTestNative]::Put($test[0],$test[1],[IntPtr]::Zero);Assert-Get}
 Write-Output 'Image fixture: native padded PNG -> GET'
 $padded=[byte[]]::new($png.Length+32);[Array]::Copy($png,$padded,$png.Length);[WiniOSImageTestNative]::Put([WiniOSImageTestNative]::RegisterClipboardFormatW('PNG'),$padded,[IntPtr]::Zero);Assert-Get
 Write-Output 'Image fixture: bitmap-only -> GET'
 [WiniOSImageTestNative]::Put(2,$null,[WiniOSImageTestNative]::FixtureBitmap())
 Assert-Get
 $hold=[WiniOSPostTestNative]::Hold(80);$response=$null;try{$response=Request-Image 'POST' $png;if([int]$response.StatusCode-ne 200){throw 'Image contention retry did not recover.'}}finally{if($response){$response.Dispose()};$hold.Join()};Assert-Get
 $hold=[WiniOSPostTestNative]::HoldUntilDisposed();$response=$null;try{$response=Request-Image 'POST' $png;if(-not$hold.IsHeld){throw 'Image contention fixture expired before response.'};if([int]$response.StatusCode-notin @(503,504)){throw "Exhausted image contention returned HTTP $([int]$response.StatusCode)."}}finally{if($response){$response.Dispose()};$hold.Dispose()}
 [WiniOSImageTestNative]::Put(0,$null,[IntPtr]::Zero);Assert-Get 204
 [WiniOSImageTestNative]::Put(8,[byte[]]::new(40),[IntPtr]::Zero);Assert-Get 503
 & (Join-Path $PSScriptRoot 'Test-ClipboardPost.ps1') -BaseUri $base -Tailnet
 Assert-Get 204
 Write-Output 'PASS: verified HTTPS image GET/POST/auth/limits; PNG, DIBV5, DIB24, bitmap-only and padded PNG capture; exact pixels; contention retries/exhaustion; clear/malformed invalidation; Unicode text recovery. No image files persisted.'
}finally{$token=$null;$fixture.Dispose();$client.Dispose();$handler.Dispose();[WiniOSImageTestNative]::Cleanup()}
