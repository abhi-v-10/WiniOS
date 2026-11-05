#Requires -Version 7.4
param(
    [string]$IpAddress = '192.168.1.7',
    [string]$CaDirectory = (Join-Path $env:LOCALAPPDATA 'WiniOS\ca-offline'),
    [string]$ServerDirectory = (Join-Path $env:LOCALAPPDATA 'WiniOS\tls'),
    [Security.SecureString]$CaPassword,
    [switch]$Renew
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This provisioning workflow requires Windows PowerShell 7.4+.' }
$ip = [Net.IPAddress]::Parse($IpAddress)
$bytes = $ip.GetAddressBytes()
if ($bytes.Length -ne 4 -or -not ($bytes[0] -eq 10 -or ($bytes[0] -eq 172 -and $bytes[1] -ge 16 -and $bytes[1] -le 31) -or ($bytes[0] -eq 192 -and $bytes[1] -eq 168))) { throw 'Choose a specific RFC1918 private IPv4.' }
$repo = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent)).TrimEnd('\') + '\'
$CaDirectory = [IO.Path]::GetFullPath($CaDirectory).TrimEnd('\')
$ServerDirectory = [IO.Path]::GetFullPath($ServerDirectory).TrimEnd('\')
foreach ($directory in @($CaDirectory, $ServerDirectory)) {
    if (($directory + '\').StartsWith($repo, [StringComparison]::OrdinalIgnoreCase)) { throw 'Private keys must be outside the Git workspace.' }
    $ancestor = $directory
    while ($ancestor) {
        if ((Test-Path -LiteralPath $ancestor) -and ((Get-Item -LiteralPath $ancestor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Reparse-point key paths are not accepted.' }
        $ancestor = Split-Path $ancestor -Parent
    }
}
if (($CaDirectory + '\').StartsWith(($ServerDirectory + '\'), [StringComparison]::OrdinalIgnoreCase) -or ($ServerDirectory + '\').StartsWith(($CaDirectory + '\'), [StringComparison]::OrdinalIgnoreCase)) { throw 'CA and runtime key directories must be separate, non-nested directories.' }
function Protect-KeyDirectory([string]$directory) {
    [void][IO.Directory]::CreateDirectory($directory)
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    $inherit = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    foreach ($principal in @($sid, [Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
        $rule = [Security.AccessControl.FileSystemAccessRule]::new($principal, 'FullControl', $inherit, 'None', 'Allow')
        [void]$acl.AddAccessRule($rule)
    }
    # Apply only the DACL. Set-Acl can also try to write owner/SACL metadata,
    # which unnecessarily requires SeSecurityPrivilege on repeat issuance.
    [IO.FileSystemAclExtensions]::SetAccessControl([IO.DirectoryInfo]::new($directory), $acl)
}
Protect-KeyDirectory $CaDirectory
Protect-KeyDirectory $ServerDirectory
if (-not $Renew -and ((Test-Path -LiteralPath (Join-Path $ServerDirectory 'server.key.pem')) -or (Test-Path -LiteralPath (Join-Path $CaDirectory 'ca.key.encrypted.pem')))) { throw 'Identity already exists. Use -Renew for a new leaf under the same CA, or separate directories for CA rotation.' }
if (-not $CaPassword) { $CaPassword = Read-Host 'CA signing-key password (use a unique strong password; never give it to the agent or phone)' -AsSecureString }
if ($CaPassword.Length -lt 20) { throw 'CA password must be at least 20 characters. Prefer a long random password.' }
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Net;
using System.Security;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using System.Runtime.InteropServices;
using System.Formats.Asn1;
public static class WiniOSCertificateSetup {
    static byte[] Constraints(IPAddress ip) {
        var w = new AsnWriter(AsnEncodingRules.DER);
        var permitted = new Asn1Tag(TagClass.ContextSpecific, 0, true);
        w.PushSequence(); w.PushSequence(permitted);
        // Restrict DNS identities too: IP-only constraints would leave DNS broad.
        w.PushSequence();
        w.WriteCharacterString(UniversalTagNumber.IA5String, "winios.invalid", new Asn1Tag(TagClass.ContextSpecific, 2));
        w.PopSequence();
        foreach (var address in new [] { ip, IPAddress.Loopback }) {
            var range = new byte[8];
            address.GetAddressBytes().CopyTo(range, 0);
            for (int n=4; n<8; n++) range[n] = 255;
            w.PushSequence();
            w.WriteOctetString(range, new Asn1Tag(TagClass.ContextSpecific, 7));
            w.PopSequence();
        }
        w.PopSequence(permitted); w.PopSequence();
        return w.Encode();
    }
    static string Pem(string label, byte[] bytes) => PemEncoding.WriteString(label, bytes);
    static void PrivateWrite(string path, string value) {
        // Directory ACL is set before any key is written. New files inherit it.
        string temporary = path + ".new";
        using (var stream = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None))
        using (var writer = new StreamWriter(stream)) writer.Write(value);
        File.Move(temporary, path, true);
    }
    public static string Issue(string caDir, string serverDir, string address, SecureString password, bool renew) {
        IntPtr bstr = Marshal.SecureStringToBSTR(password);
        char[] chars = new char[password.Length];
        try {
            Marshal.Copy(bstr, chars, 0, chars.Length);
            return IssueInner(caDir, serverDir, IPAddress.Parse(address), chars, renew);
        } finally {
            Array.Clear(chars, 0, chars.Length);
            Marshal.ZeroFreeBSTR(bstr);
        }
    }
    static string IssueInner(string caDir, string serverDir, IPAddress ip, char[] password, bool renew) {
        string caKeyPath = Path.Combine(caDir, "ca.key.encrypted.pem");
        string caCertPath = Path.Combine(caDir, "winios-ca.cer");
        var now = DateTimeOffset.UtcNow;
        using var caKey = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        byte[] rootBytes;
        if (renew) {
            if (!File.Exists(caKeyPath) || !File.Exists(caCertPath)) throw new Exception("Renewal requires the existing CA certificate and encrypted signing key.");
            caKey.ImportFromEncryptedPem(File.ReadAllText(caKeyPath), password);
            rootBytes = File.ReadAllBytes(caCertPath);
        } else {
            var request = new CertificateRequest("CN=WiniOS Development CA " + ip, caKey, HashAlgorithmName.SHA256);
            request.CertificateExtensions.Add(new X509BasicConstraintsExtension(true, true, 0, true));
            request.CertificateExtensions.Add(new X509KeyUsageExtension(X509KeyUsageFlags.KeyCertSign | X509KeyUsageFlags.CrlSign, true));
            request.CertificateExtensions.Add(new X509EnhancedKeyUsageExtension(new OidCollection { new Oid("1.3.6.1.5.5.7.3.1") }, true));
            request.CertificateExtensions.Add(new X509SubjectKeyIdentifierExtension(request.PublicKey, false));
            request.CertificateExtensions.Add(new X509Extension("2.5.29.30", Constraints(ip), true));
            using var root = request.CreateSelfSigned(now.AddMinutes(-5), now.AddDays(365));
            rootBytes = root.Export(X509ContentType.Cert);
            var encrypted = caKey.ExportEncryptedPkcs8PrivateKeyPem(password,
                new PbeParameters(PbeEncryptionAlgorithm.Aes256Cbc, HashAlgorithmName.SHA256, 210000));
            PrivateWrite(caKeyPath, encrypted);
            File.WriteAllBytes(caCertPath, rootBytes);
        }
        #pragma warning disable SYSLIB0057
        using var caPublic = new X509Certificate2(rootBytes);
        #pragma warning restore SYSLIB0057
        // Renewal is only for the original constrained address.
        var constraint = caPublic.Extensions["2.5.29.30"];
        if (constraint == null || !CryptographicOperations.FixedTimeEquals(constraint.RawData, Constraints(ip)))
            throw new Exception("CA scope does not match this IP. Rotate the CA in new directories and explicitly replace device trust.");
        using var ca = caPublic.CopyWithPrivateKey(caKey);
        if (ca.NotAfter.ToUniversalTime() < now.UtcDateTime.AddDays(30)) throw new Exception("CA expires soon; rotate it before issuing.");
        using var leafKey = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        // Windows also evaluates the legacy CN against DNS constraints when
        // the SAN contains IPs only. Keep it inside the reserved namespace;
        // actual URL identity is still exclusively the IP subjectAltName.
        var leaf = new CertificateRequest("CN=server.winios.invalid", leafKey, HashAlgorithmName.SHA256);
        leaf.CertificateExtensions.Add(new X509BasicConstraintsExtension(false, false, 0, true));
        leaf.CertificateExtensions.Add(new X509KeyUsageExtension(X509KeyUsageFlags.DigitalSignature, true));
        leaf.CertificateExtensions.Add(new X509EnhancedKeyUsageExtension(new OidCollection { new Oid("1.3.6.1.5.5.7.3.1") }, true));
        leaf.CertificateExtensions.Add(new X509SubjectKeyIdentifierExtension(leaf.PublicKey, false));
        var san = new SubjectAlternativeNameBuilder();
        san.AddIpAddress(ip); san.AddIpAddress(IPAddress.Loopback);
        leaf.CertificateExtensions.Add(san.Build());
        byte[] serial = RandomNumberGenerator.GetBytes(16); serial[0] &= 0x7f; serial[0] |= 1;
        var until = now.AddDays(90);
        if (until.UtcDateTime > ca.NotAfter.ToUniversalTime()) until = new DateTimeOffset(ca.NotAfter.ToUniversalTime()).AddMinutes(-1);
        using var certificate = leaf.Create(ca, now.AddMinutes(-4), until, serial);
        // Exercise Windows' actual constraint/EKU/date validation before
        // publishing a runtime identity. This changes no OS trust store.
        using var chain = new X509Chain();
        chain.ChainPolicy.TrustMode = X509ChainTrustMode.CustomRootTrust;
        chain.ChainPolicy.CustomTrustStore.Add(caPublic);
        chain.ChainPolicy.ApplicationPolicy.Add(new Oid("1.3.6.1.5.5.7.3.1"));
        chain.ChainPolicy.RevocationMode = X509RevocationMode.NoCheck;
        if (!chain.Build(certificate)) throw new Exception("Windows rejected the issued certificate chain. No runtime identity was published.");
        PrivateWrite(Path.Combine(serverDir, "server.key.pem"), leafKey.ExportPkcs8PrivateKeyPem());
        PrivateWrite(Path.Combine(serverDir, "server.cert.pem"), Pem("CERTIFICATE", certificate.Export(X509ContentType.Cert)));
        File.WriteAllBytes(Path.Combine(serverDir, "winios-ca.cer"), rootBytes); // Public only.
        return Convert.ToHexString(SHA256.HashData(rootBytes));
    }
}
'@
$fingerprint = [WiniOSCertificateSetup]::Issue($CaDirectory, $ServerDirectory, $IpAddress, $CaPassword, $Renew.IsPresent)
Write-Output "Issued ECDSA P-256/SHA-256 server certificate, SAN IP=$IpAddress and 127.0.0.1; leaf validity <=90 days."
Write-Output "Public CA SHA-256 fingerprint: $fingerprint"
Write-Output "Public CA for manual device review: $(Join-Path $ServerDirectory 'winios-ca.cer')"
Write-Output 'No trust was installed. Keep the encrypted CA signing key/password offline and separate from runtime. Never send a key, PEM private identity, or PFX/PKCS#12 to iPhone.'
