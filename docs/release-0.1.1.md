# WiniOS 0.1.1 — local Windows release

This unsigned, locally built release packages the existing text/image Smart
Send/Receive and Mission 011 status/recovery improvements. It adds no new
clipboard functionality, UI redesign, permission or network configuration.

## Install or upgrade deliberately

1. Obtain `WiniOS-Setup.exe` and its adjacent `WiniOS-Setup.manifest.json` from
   `C:\WiniOS\target\dist\0.1.1`. Check the installer's SHA-256 against that
   manifest. A matching hash is an integrity check, not publisher authentication.
   This build is unsigned; do not bypass Windows security warnings. Public
   distribution requires a separate signing/security/license review.
2. Keep a trusted prior full installer and manifest for rollback. Back up the
   installed program directory before an upgrade; keep any personal-data backup
   separately protected, especially TLS private keys. Never copy keys/tokens into
   this release folder or send them to support.
3. Run Setup as your ordinary Windows user. It installs at the unchanged path
   `%LOCALAPPDATA%\Programs\WiniOS` with no admin requirement. A running tray app
   is asked to quit gracefully; close console agents yourself with Ctrl+C.
   A busy agent aborts replacement rather than being killed.
4. Choose whether to launch on Finish. Launch/search **WiniOS** from Start; pin
   through Windows if desired. Setup does not enable autostart. Existing settings,
   token, certificates, startup preference/Windows Startup Apps policy and
   Tailscale Serve remain in place. No credentials or TLS material are bundled.

Verify Connected, Pause/Resume, authenticated text/image Shortcuts and tray Quit.
Reopen the app to check it starts once without a console. These are manual checks
for this release, not claims that installation or iPhone flows were retested.
The laptop must stay awake, connected to Tailscale and running WiniOS. Existing
verified private HTTPS and bearer authorization remain required.

## Uninstall and rollback

Uninstall through Windows Installed Apps. It removes package files, its Start
Menu shortcut and recognized WiniOS Run registration. Settings, Credential Manager
token, TLS files and unrelated files are retained. Reinstalling after uninstall
leaves startup OFF; record your preference before uninstalling if you plan to
restore it. Windows Startup Apps policy is not changed.

Tailscale Serve is user-managed and may still point at the stopped backend.
Review it yourself with `tailscale serve status`. Only with explicit approval,
and only for the dedicated WiniOS 443 listener, use
`tailscale serve --https=443 off`. Never reset unrelated routes. Loopback does not
authenticate local processes; do not leave an unintended route unattended.

For rollback, Quit WiniOS and run the saved trusted compatible full installer
over the same directory. Review compatibility/security before choosing an older
version. Do not uninstall first merely to downgrade, delete settings/token/CA,
or overwrite uninstaller data manually. There is no automatic rollback updater.
For this machine, the pre-Mission-011 full installer and original program backup
are at `C:\WiniOS\target\mission011-deploy-backup-20261110-142518`; those are
local rollback materials, not part of this package.

## Build and verify without installing

```powershell
cd C:\WiniOS
.\scripts\Build-Installer.ps1 -ISCCPath C:\WiniOS\target\tools\InnoSetup7\ISCC.exe
.\scripts\Test-Installer.ps1
```

The build command performs one offline locked static-CRT release build for the
GUI and CLI, then one Inno compilation. Output uses the versioned folder above;
older folders/installers are retained. All 14 explicit package files are hashed
in the manifest. `-SkipRustBuild` is for an already built, matching-version
payload; stale version/branding is rejected. A different `-PackageVersion` is
solely an explicit lifecycle-test fixture override, not a release-version bump.
Normal packaging verification does not launch/install/uninstall the app, query
credentials or modify Windows integration. `Test-Installer.ps1 -Lifecycle` is a
separate opt-in, disruptive real-install test; do not run it on a working install
without preparation and authorization.

Remaining release checks: clean non-admin Windows profile/VM, interactive upgrade
and uninstall, startup ON/OFF and disabled-policy retention, install cancellation,
DPI/accessibility, real iPhone transfer after upgrade, and public-release signing
and applicable dependency/tool licensing review. Windows 11 x64 is the primary
target; other supported OS/emulation combinations need validation.
