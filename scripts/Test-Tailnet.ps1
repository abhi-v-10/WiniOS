#Requires -Version 7.4
param(
    [ValidateRange(1, 65535)][int]$BackendPort = 8766,
    [switch]$NativeClipboard,
    [switch]$ExpectPaused,
    [switch]$InspectCurrent
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Run on the Windows WiniOS host.' }
# Read-only. This script NEVER configures Serve/Funnel, adapters or firewall.
function Read-TailscaleJson([string[]]$CliArguments) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = Join-Path $env:ProgramFiles 'Tailscale\tailscale.exe'
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $CliArguments) { [void]$start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        [void]$process.Start()
        $output = $process.StandardOutput.ReadToEndAsync()
        $errorOutput = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(5000)) { $process.Kill(); throw 'Local Tailscale query timed out.' }
        if ($process.ExitCode -ne 0) { throw 'Local Tailscale query failed. Check installation, sign-in and Windows user permissions.' }
        $json = $output.GetAwaiter().GetResult()
        if ($json.Length -gt 1MB) { throw 'Unexpected Tailscale output size.' }
        try { return $json | ConvertFrom-Json } catch { throw 'Invalid Tailscale status response.' }
    } finally { $process.Dispose() }
}
$status = Read-TailscaleJson @('status', '--json', '--peers=false')
$hostName = ([string]$status.Self.DNSName).TrimEnd('.')
$suffix = [string]$status.CurrentTailnet.MagicDNSSuffix
if ($status.BackendState -ne 'Running' -or -not $status.Self.Online -or -not $status.CurrentTailnet.MagicDNSEnabled) { throw 'Tailscale must be signed in, connected and have MagicDNS enabled.' }
if ($hostName -notmatch '^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.[a-z0-9-]+\.ts\.net$' -or -not $hostName.EndsWith(".$suffix", [StringComparison]::Ordinal) -or $status.CertDomains -cnotcontains $hostName) { throw 'Node lacks a verified HTTPS tailnet DNS identity. Review HTTPS/MagicDNS prerequisites.' }
$serve = Read-TailscaleJson @('serve', 'status', '--json')
if ($serve.AllowFunnel -and @($serve.AllowFunnel.PSObject.Properties | Where-Object Value -ne $false).Count) { throw 'Funnel must be disabled. WiniOS will not enable or reset it.' }
$web = $serve.Web."${hostName}:443"
if (-not $serve.TCP.'443'.HTTPS -or $web.Handlers.'/'.Proxy -cne "http://127.0.0.1:$BackendPort") { throw 'Expected HTTPS Serve root proxy is not configured. Follow the user-approved setup in docs/portable-connectivity.md.' }
if (@($serve.TCP.PSObject.Properties).Count -ne 1 -or @($serve.Web.PSObject.Properties).Count -ne 1 -or @($web.Handlers.PSObject.Properties).Count -ne 1 -or ($serve.Foreground -and @($serve.Foreground.PSObject.Properties).Count) -or ($serve.Services -and @($serve.Services.PSObject.Properties).Count)) { throw 'Additional Serve listeners/routes require review; the WiniOS adapter accepts only its dedicated profile.' }
$resolved = [Net.Dns]::GetHostAddresses($hostName)
if (-not $resolved -or @($resolved | Where-Object { $_.ToString() -cnotin $status.Self.TailscaleIPs }).Count) { throw 'Tailnet DNS does not resolve exclusively to the current node addresses.' }
$baseUri = [uri]"https://$hostName/"
$handler = [Net.Http.SocketsHttpHandler]::new()
$handler.UseProxy = $false
$handler.AllowAutoRedirect = $false
$client = [Net.Http.HttpClient]::new($handler)
$client.Timeout = [TimeSpan]::FromSeconds(10)
try {
    $health = $client.GetAsync([uri]::new($baseUri, '/v1/health')).GetAwaiter().GetResult()
    if ([int]$health.StatusCode -ne 200) { throw "HTTPS health not ready: HTTP $([int]$health.StatusCode)." }
    foreach ($method in @('GET', 'POST')) {
        $request = [Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::new($method), [uri]::new($baseUri, '/v1/clipboard'))
        try {
            $response = $client.SendAsync($request).GetAwaiter().GetResult()
            try {
                if ([int]$response.StatusCode -ne 401 -or -not $response.Headers.CacheControl.NoStore) { throw 'Clipboard requests must require bearer authentication and prohibit caching.' }
            } finally { $response.Dispose() }
        } finally { $request.Dispose() }
    }
    Write-Output "PASS: verified HTTPS health, GET/POST authentication, no-store. Endpoint: $baseUri"
    if ($NativeClipboard -or $ExpectPaused -or $InspectCurrent) {
        # Deliberately overwrites clipboard with disposable text; never prints it.
        & (Join-Path $PSScriptRoot 'Test-ClipboardPost.ps1') -BaseUri $baseUri -Tailnet -ExpectPaused:$ExpectPaused -InspectCurrent:$InspectCurrent
    }
} finally { $client.Dispose(); $handler.Dispose() }
