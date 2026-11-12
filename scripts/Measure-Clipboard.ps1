#Requires -Version 7.4
# Disposable generated pixels only. No image files, bodies, tokens or pixel hashes
# are written. Run with image sharing/writes enabled and an otherwise idle agent.
param([ValidateRange(1,20)][int]$Samples=5,[int]$AgentProcessId,
      [string]$OutputPath=(Join-Path $PSScriptRoot '..\target\mission009-benchmark.json'))
$ErrorActionPreference='Stop'
if(-not $IsWindows){throw 'Windows is required.'}
& (Join-Path $PSScriptRoot 'Test-Tailnet.ps1')
$start=[Diagnostics.ProcessStartInfo]::new()
$start.FileName=Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
$start.UseShellExecute=$false;$start.CreateNoWindow=$true
$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
foreach($arg in @('status','--json','--peers=false')){[void]$start.ArgumentList.Add($arg)}
$process=[Diagnostics.Process]::Start($start)
try{
 $out=$process.StandardOutput.ReadToEndAsync();$err=$process.StandardError.ReadToEndAsync()
 if(-not $process.WaitForExit(5000)-or$process.ExitCode-ne 0){throw 'Cannot verify Tailscale.'}
 $status=$out.GetAwaiter().GetResult()|ConvertFrom-Json
}finally{$process.Dispose()}
$name=([string]$status.Self.DNSName).TrimEnd('.')
if($status.BackendState-ne 'Running'-or-not$status.Self.Online-or$status.CertDomains-cnotcontains$name){throw 'Verified tailnet identity required.'}
$base=[uri]"https://$name/"
$agents=@(Get-Process -Name winios,winios-agent -ErrorAction SilentlyContinue)
if($agents.Count-ne 1-or($AgentProcessId-and$agents[0].Id-ne$AgentProcessId)){throw 'Expected the single selected agent.'}
$agent=$agents[0]
$source=Get-Content (Join-Path $PSScriptRoot 'Test-ClipboardPost.ps1') -Raw
$helper=[regex]::Match($source,"(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")
if(-not$helper.Success){throw 'Existing credential helper not found.'}
if(-not('WiniOSPostTestNative'-as[type])){Add-Type -TypeDefinition $helper.Groups[1].Value}
Add-Type -AssemblyName System.Drawing
$drawing=[Drawing.Bitmap].Assembly.Location
Add-Type -ReferencedAssemblies @($drawing,'System.Drawing.Primitives','System.Runtime.InteropServices','System.Threading','System.Threading.Thread','System.Diagnostics.Process','System.ComponentModel.Primitives','System.Security.Cryptography',(Join-Path $PSHOME 'System.Private.Windows.GdiPlus.dll'),(Join-Path $PSHOME 'System.Private.Windows.Core.dll')) -TypeDefinition @'
using System; using System.Drawing; using System.Drawing.Imaging;
using System.IO; using System.Runtime.InteropServices; using System.Threading;
using System.Diagnostics; using System.Security.Cryptography;
public static class WiniOSBenchFixture {
 public static byte[] Png(int width,int height,bool noise) {
  using(var bitmap=new Bitmap(width,height,PixelFormat.Format32bppArgb)) {
   var data=bitmap.LockBits(new Rectangle(0,0,width,height),ImageLockMode.WriteOnly,PixelFormat.Format32bppArgb);
   try {uint seed=7;var row=new byte[width*4];for(int y=0;y<height;y++) {
    for(int x=0;x<width;x++){for(int c=0;c<3;c++){seed=unchecked(seed*1664525+1013904223);row[x*4+c]=noise?(byte)(seed>>24):(byte)((x/32+y/24+c*40)%256);}row[x*4+3]=255;}
    Marshal.Copy(row,0,IntPtr.Add(data.Scan0,y*data.Stride),row.Length);
   }}finally{bitmap.UnlockBits(data);}
   using(var stream=new MemoryStream()){bitmap.Save(stream,ImageFormat.Png);return stream.ToArray();}
  }
 }
 static byte[] Pixels(byte[] png) {
  using(var stream=new MemoryStream(png,false))using(var bitmap=new Bitmap(stream)) {
   var data=bitmap.LockBits(new Rectangle(0,0,bitmap.Width,bitmap.Height),ImageLockMode.ReadOnly,PixelFormat.Format32bppArgb);
   try{var bytes=new byte[bitmap.Width*bitmap.Height*4];for(int y=0;y<bitmap.Height;y++)Marshal.Copy(IntPtr.Add(data.Scan0,y*data.Stride),bytes,y*bitmap.Width*4,bitmap.Width*4);return bytes;}
   finally{bitmap.UnlockBits(data);}
  }
 }
 public static bool Equal(byte[] a,byte[] b){return CryptographicOperations.FixedTimeEquals(SHA256.HashData(Pixels(a)),SHA256.HashData(Pixels(b)));}
}
public sealed class WiniOSBenchSampler:IDisposable {
 readonly Process process;readonly Thread thread;volatile bool stop;
 public long PeakPrivate,PeakWorking;public int GdiStart,GdiEnd;public volatile bool Failed;
 [DllImport("user32.dll")]static extern uint GetGuiResources(IntPtr process,uint flags);
 public WiniOSBenchSampler(int id){process=Process.GetProcessById(id);GdiStart=(int)GetGuiResources(process.Handle,0);thread=new Thread(Sample){IsBackground=true};thread.Start();}
 void Sample(){try{while(!stop){process.Refresh();PeakPrivate=Math.Max(PeakPrivate,process.PrivateMemorySize64);PeakWorking=Math.Max(PeakWorking,process.WorkingSet64);Thread.Sleep(5);}}catch{Failed=true;}}
 public void Dispose(){stop=true;thread.Join();try{GdiEnd=(int)GetGuiResources(process.Handle,0);}catch{Failed=true;}finally{process.Dispose();}}
}
'@
$handler=[Net.Http.SocketsHttpHandler]::new();$handler.UseProxy=$false;$handler.AllowAutoRedirect=$false
$client=[Net.Http.HttpClient]::new($handler);$client.Timeout=[TimeSpan]::FromSeconds(15)
$token=[WiniOSPostTestNative]::Token()
function Request([string]$Method,[string]$Path,[byte[]]$Bytes,[string]$Type){
 $request=[Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::new($Method),[uri]::new($base,$Path))
 try{
  $request.Headers.Authorization=[Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer',$token)
  if($Method-eq 'POST'){$request.Content=[Net.Http.ByteArrayContent]::new($Bytes);$request.Content.Headers.ContentType=[Net.Http.Headers.MediaTypeHeaderValue]::new($Type)}
  return $client.SendAsync($request).GetAwaiter().GetResult()
 }finally{$request.Dispose()}
}
function Percentile($Values,[double]$Percent){$sorted=@($Values|Sort-Object);return [Math]::Round($sorted[[Math]::Max(0,[Math]::Ceiling($Percent*$sorted.Count)-1)],2)}
$sampler=[WiniOSBenchSampler]::new($agent.Id);$results=@()
try{
 foreach($spec in @(@('1080p-pattern',1920,1080,$false),@('1080p-noise',1920,1080,$true),@('4k-pattern',3840,2160,$false))){
  $png=[WiniOSBenchFixture]::Png($spec[1],$spec[2],$spec[3]);$post=@();$get=@();$received=0
  for($i=0;$i-lt$Samples;$i++){
   $clock=[Diagnostics.Stopwatch]::StartNew();$response=Request 'POST' 'v1/clipboard/image' $png 'image/png'
   try{if([int]$response.StatusCode-ne 200){throw "PNG POST rejected: HTTP $([int]$response.StatusCode). Enable images, 8 MiB and writes for this disposable benchmark."};$post+=$clock.Elapsed.TotalMilliseconds}finally{$response.Dispose()}
   $clock.Restart();$response=Request 'GET' 'v1/clipboard/image' $null ''
   try{if([int]$response.StatusCode-ne 200){throw "PNG GET rejected: HTTP $([int]$response.StatusCode)."};$bytes=$response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult();$get+=$clock.Elapsed.TotalMilliseconds;$received=$bytes.Length;if(-not[WiniOSBenchFixture]::Equal($png,$bytes)){throw 'Disposable fixture pixels changed.'}}finally{$response.Dispose()}
  }
  $results+=[pscustomobject]@{Fixture=$spec[0];Width=$spec[1];Height=$spec[2];Samples=$Samples;UploadBytes=$png.Length;DownloadBytes=$received;PostMedianMs=(Percentile $post .5);PostP95Ms=(Percentile $post .95);GetMedianMs=(Percentile $get .5);GetP95Ms=(Percentile $get .95);ExactPixels=$true}
  $png=$null;$bytes=$null
 }
}finally{
 # Replace the disposable image with known text through the normal ack path.
 try{$response=Request 'POST' 'v1/clipboard' ([Text.Encoding]::UTF8.GetBytes('{"text":"WiniOS disposable benchmark complete"}')) 'application/json';$response.Dispose()}finally{$sampler.Dispose();$token=$null;$client.Dispose();$handler.Dispose()}
}
if($sampler.Failed){throw 'Agent memory sampling was interrupted; no valid benchmark result.'}
Start-Sleep -Milliseconds 500;$agent.Refresh()
$record=[pscustomobject]@{Scope='Same Windows host through existing validated Tailscale HTTPS Serve; not Wi-Fi/iPhone latency';AgentProcessId=$agent.Id;SampleIntervalMs=5;AgentPeakPrivateMiB=[Math]::Round($sampler.PeakPrivate/1MB,2);AgentPeakWorkingMiB=[Math]::Round($sampler.PeakWorking/1MB,2);AfterClearPrivateMiB=[Math]::Round($agent.PrivateMemorySize64/1MB,2);AfterClearWorkingMiB=[Math]::Round($agent.WorkingSet64/1MB,2);GdiHandlesBefore=$sampler.GdiStart;GdiHandlesAfter=$sampler.GdiEnd;Transfers=$results}
$record|ConvertTo-Json -Depth 5|Set-Content -LiteralPath $OutputPath
$record|ConvertTo-Json -Depth 5
