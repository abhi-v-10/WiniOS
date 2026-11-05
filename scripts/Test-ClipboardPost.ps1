param([Parameter(Mandatory = $true)][uri]$BaseUri)
$ErrorActionPreference = 'Stop'
if ($BaseUri.Scheme -ne 'http' -or $BaseUri.UserInfo -or $BaseUri.Query) { throw 'Use the printed http://private-IP:port base URL without credentials or a query.' }
# Intentionally overwrites the clipboard with disposable test data. Never prints
# clipboard contents, HTTP bodies, Authorization headers, or the stored token.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public static class WiniOSPostTestNative {
    [StructLayout(LayoutKind.Sequential)]
    struct Credential {
        public uint Flags, Type;
        public IntPtr TargetName, Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint BlobSize;
        public IntPtr Blob;
        public uint Persist, AttributeCount;
        public IntPtr Attributes, TargetAlias, UserName;
    }
    [DllImport("advapi32.dll", CharSet=CharSet.Unicode)] static extern bool CredReadW(string target, uint type, uint flags, out IntPtr credential);
    [DllImport("advapi32.dll")] static extern void CredFree(IntPtr credential);
    [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool CloseClipboard();
    [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint format);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr handle);
    [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr handle);
    [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr handle);
    public static string Token() {
        IntPtr pointer;
        if (!CredReadW("WiniOS/Agent/Bearer/v1", 1, 0, out pointer)) throw new Exception("Credential access failed.");
        try {
            var credential = Marshal.PtrToStructure<Credential>(pointer);
            if (credential.BlobSize != 64) throw new Exception("Invalid credential size.");
            var bytes = new byte[64];
            Marshal.Copy(credential.Blob, bytes, 0, 64);
            return Encoding.ASCII.GetString(bytes);
        } finally { CredFree(pointer); }
    }
    public static string Text() {
        if (!OpenClipboard(IntPtr.Zero)) throw new Exception("Test clipboard read could not open clipboard.");
        try {
            IntPtr handle = GetClipboardData(13);
            if (handle == IntPtr.Zero) throw new Exception("No Unicode text.");
            int units = (int)Math.Min(GlobalSize(handle).ToUInt64() / 2, 524289UL);
            IntPtr pointer = GlobalLock(handle);
            if (pointer == IntPtr.Zero) throw new Exception("Test clipboard lock failed.");
            try {
                int length = 0;
                while (length < units && Marshal.ReadInt16(pointer, length * 2) != 0) length++;
                if (length == units) throw new Exception("Missing test terminator.");
                return Marshal.PtrToStringUni(pointer, length);
            } finally { GlobalUnlock(handle); }
        } finally { CloseClipboard(); }
    }
    public static Thread Hold(int milliseconds) {
        var ready = new ManualResetEventSlim();
        bool opened = false;
        var thread = new Thread(() => {
            opened = OpenClipboard(IntPtr.Zero);
            ready.Set();
            if (!opened) return;
            try { Thread.Sleep(milliseconds); } finally { CloseClipboard(); }
        });
        thread.IsBackground = true;
        thread.Start();
        ready.Wait();
        ready.Dispose();
        if (!opened) { thread.Join(); throw new Exception("Contention helper could not open clipboard."); }
        return thread;
    }
}
'@
$handler = [System.Net.Http.SocketsHttpHandler]::new()
$handler.UseProxy = $false
$client = [System.Net.Http.HttpClient]::new($handler)
$client.Timeout = [TimeSpan]::FromSeconds(8)
$endpoint = [uri]::new($BaseUri, '/v1/clipboard')
function Send-TestPost([string]$text) {
    $json = @{ text = $text } | ConvertTo-Json -Compress
    $content = [System.Net.Http.StringContent]::new($json, [Text.Encoding]::UTF8, 'application/json')
    try { return $client.PostAsync($endpoint, $content).GetAwaiter().GetResult() }
    finally { $content.Dispose() }
}
try {
    $response = Send-TestPost 'disposable unauthorized test'
    if ([int]$response.StatusCode -ne 401) { throw 'POST must reject missing credentials.' }
    $client.DefaultRequestHeaders.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', [WiniOSPostTestNative]::Token())
    $sample = 'WiniOS disposable POST test 😀' + [char]10 + 'Line 2: é' + [char]13 + [char]10 + 'End'
    $response = Send-TestPost $sample
    if ([int]$response.StatusCode -ne 200) { throw "POST acceptance failed: HTTP $([int]$response.StatusCode)" }
    $receipt = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult() | ConvertFrom-Json
    if ($receipt.status -ne 'ok') { throw 'Missing acceptance acknowledgement.' }
    if ([WiniOSPostTestNative]::Text() -cne $sample) { throw 'Native clipboard text mismatch.' }
    $get = $client.GetAsync($endpoint).GetAwaiter().GetResult()
    if ([int]$get.StatusCode -ne 200 -or $get.Content.ReadAsStringAsync().GetAwaiter().GetResult() -cne $sample) { throw 'GET readback mismatch.' }
    $repeated = Send-TestPost $sample
    $secondReceipt = $repeated.Content.ReadAsStringAsync().GetAwaiter().GetResult() | ConvertFrom-Json
    if ([int]$repeated.StatusCode -ne 200 -or $secondReceipt.operation -le $receipt.operation) { throw 'Repeated write operation tracking failed.' }
    $hold = [WiniOSPostTestNative]::Hold(500)
    try { $retried = Send-TestPost $sample } finally { $hold.Join() }
    if ([int]$retried.StatusCode -ne 200) { throw 'Write did not recover from temporary contention.' }
    $hold = [WiniOSPostTestNative]::Hold(1800)
    try { $busy = Send-TestPost 'disposable text that must not be written' } finally { $hold.Join() }
    if ([int]$busy.StatusCode -ne 503) { throw 'Write contention did not exhaust bounded retries.' }
    if ([WiniOSPostTestNative]::Text() -cne $sample) { throw 'Failed write modified clipboard.' }
    # This is a deliberate local copy, independent of the agent write API.
    Set-Clipboard -Value $sample
    $localReadbackOk = $false
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        $get = $client.GetAsync($endpoint).GetAwaiter().GetResult()
        if ([int]$get.StatusCode -eq 200 -and $get.Content.ReadAsStringAsync().GetAwaiter().GetResult() -ceq $sample) { $localReadbackOk = $true; break }
        Start-Sleep -Milliseconds 50
    }
    if (-not $localReadbackOk) { throw 'GET did not recover after a deliberate local copy.' }
    Write-Output 'PASS: unauthorized POST=401; accepted/repeated/retried POST=200; exhausted contention=503; exact native Unicode + LF/CRLF readback; GET and local-copy recovery.'
} finally {
    $sample = $null
    $client.Dispose()
    $handler.Dispose()
}
