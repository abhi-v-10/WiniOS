# Mission 007 — per-user Windows installer

## Install, launch and upgrade

Use **`WiniOS-Setup.exe`** from a trusted release. Setup uses Inno Setup 7's modern
dark wizard and always installs for the current user without requesting admin.
The fixed path is `%LOCALAPPDATA%\Programs\WiniOS`; `/DIR` overrides are rejected
because startup and native desktop integration require this stable identity.
The x64 app supports compatible Windows hosts; real testing here uses Windows 11
x64. The installer minimum is Windows 10 build 19041; other OS versions/ARM64
emulation remain unverified.

1. Run Setup, review the preservation/network notice, then Install.
2. A running **tray app** is sent its normal graceful Close/Quit message. Setup
   waits up to 20 seconds for the actual process to exit. It then atomically holds
   `Local\WiniOS.Agent`, the mutex all current app binaries already check, until
   replacement and startup repair finish. Launch races fail before replacement.
3. A console/unknown instance, failed shutdown or unavailable mutex causes a
   clear failure. Quit the tray normally or Ctrl+C the console yourself and retry.
   No process/thread is forcibly terminated. Restart Manager automatic closure
   and automatic restart are disabled. No reboot replacement is requested.
4. A single Start Menu **WiniOS** shortcut uses `--tray`, the installation working
   directory, app icon and `WiniOS.Desktop` AppUserModelID. It is searchable and can
   be pinned using Windows' own UI. Setup never pins anything automatically.
5. Select **Launch WiniOS** on Finish if wanted. New users see native first-run
   Setup; existing users retain their working settings. Normal startup has no
   terminal. Silent installs do not launch unless explicitly given `/LAUNCH=1`.

Re-running Setup upgrades/reinstalls the same directory and Installed Apps entry.
The stable AppId is `{D2286755-6B43-49F1-AC97-09151B20B0A8}`; do not change it for
ordinary releases. Files are replaced with the reviewed package even if their
embedded version is the same. Setup does not implement an updater or enforce a
minimum downgrade version: only deliberately install older trusted releases after
reviewing compatibility/security. There is no automatic downgrade/fallback.

## Ownership, startup and retained data

The explicit payload is GUI/CLI binaries, `winios.ico`, `INSTALLATION.txt` and ten
documentation files (14 files total in release 0.1.1). No wildcard source, Tailscale binary, settings, certificate,
private key, token, clipboard contents, frontend or runtime web engine is included.
The icon/resources needed by the GUI are also embedded in its executable.

Startup is **not** a new default-ON installer task. Immediately before replacement
Setup inspects the actual HKCU `Run\WiniOS` value after stopping the app:

- Absent stays OFF. An existing recognized quoted `winios.exe --startup` command
  stays enabled and is repaired to the correctly quoted installed path.
- Unrecognized values/types stop installation without overwriting that value.
  Concurrent changes are checked again before repair. Windows `StartupApproved`
  and administrator policy are never modified or overridden.
- Uninstall deletes the current recognized WiniOS startup registration. An
  unrecognized/foreign command is retained with a warning for manual review.
  A clean reinstall after uninstall defaults to OFF because the Run value is gone.

Personal data remains outside installer ownership:

- `%LOCALAPPDATA%\WiniOS\settings.json` and other settings;
- Credential Manager `WiniOS/Agent/Bearer/v1`;
- `%LOCALAPPDATA%\WiniOS\tls` and the existing CA/leaf identity;
- user-managed Tailscale Serve, account/login and device state;
- clipboard contents, firewall rules, OS trust, network settings and StartupApproved.

The installer contains no credential API, network configuration command or
personal-data deletion action. There is deliberately no bundled "remove all
personal data" option. Installation does not regenerate certificates, rotate a
token, enable clipboard writes, unpause sharing or create pairing.

## Uninstall and Serve warning

Use **Settings → Apps → Installed apps → WiniOS → Uninstall**, or run
`%LOCALAPPDATA%\Programs\WiniOS\unins000.exe`. It uses the same graceful stop and
singleton gate, removes registered package files/shortcut/startup and its HKCU
uninstall entry, and preserves settings/credentials/TLS by default. Unknown files
and old manual backup folders are not recursively deleted; an install directory
containing these may remain. Keep/review those backups yourself.

The interactive confirmation and completion messages warn that **Tailscale Serve
may remain**, because the user owns its lifetime. It can continue pointing at a
stopped loopback backend. Setup/uninstall do not run any removal command. Review:

```powershell
& "$env:ProgramFiles\Tailscale\tailscale.exe" serve status
# ONLY after reviewing and approving removal of the dedicated WiniOS HTTPS 443
# listener; do not affect another application's routes:
& "$env:ProgramFiles\Tailscale\tailscale.exe" serve --https=443 off
```

Do not use `serve reset` or enable Funnel as cleanup. See
[Serve lifetime/removal/rollback](portable-connectivity.md#serve-lifetime-removal-and-rollback).
Stop or restrict that route yourself if uninstalling permanently; loopback is not
process authentication and a different local program could later bind the port.

## Build and release

Use an x64 Windows MSVC Rust toolchain/Windows SDK and **maintained Inno Setup 7**.
Download from the [official page](https://jrsoftware.org/isdl.php), follow its
[verification instructions](https://jrsoftware.org/isdl-verify.php), and review
the applicable license. The development compiler installed here is 7.1.1,
Authenticode **Valid**, publisher **Pyrsys B.V.**, in
`C:\WiniOS\target\tools\InnoSetup7`. Compiler setup was per-user, without app
shortcuts/associations or admin; it is a build prerequisite, not shipped with WiniOS.
The observed compiler reports non-commercial use. Commercial distribution needs
the appropriate Inno license; do not treat this prototype build as commercial clearance.

```powershell
cd C:\WiniOS
.\scripts\Build-Installer.ps1 -ISCCPath C:\WiniOS\target\tools\InnoSetup7\ISCC.exe
.\scripts\Test-Installer.ps1
```

The build script separately compiles GUI/CLI into `target\installer-build\release`
with **static CRT**, restoring environment flags afterward. A PE import check
rejects dynamic/development VC runtime dependencies, so no VC redistributable or
admin installation is required for this payload. Normal `target\release` output
remains available for debugging. Static CRT changes packaging/linkage, not the
clipboard engine, networking or application settings.

Output:

- `target\dist\<version>\WiniOS-Setup.exe` — local per-user installer;
- `target\dist\<version>\WiniOS-Setup.manifest.json` — package/application/compiler version, installer
  SHA-256 and hashes for every explicit payload file. Treat a checksum as integrity metadata,
  not publisher authentication; obtain expected hashes through a trusted channel.

`-SkipRustBuild` reuses the verified static-CRT payload. `-PackageVersion 0.0.9`
is an explicit test-fixture override; real release versions should come from
`Cargo.toml`, and changing app version/resources should be reviewed together.
`-OutputDirectory` selects an artifact directory, not the installed app path.
Offline builds require Cargo dependencies to be fetched beforehand.

These local prototype artifacts are **unsigned**. No publisher signing certificate
was supplied. Before public release, sign the GUI/CLI, configure Inno's SignTool
and SignedUninstaller support, sign/timestamp Setup, rebuild the hash manifest and
repeat verification with the signed payload. Use publisher-controlled code-signing
credentials outside Git/build logs. Do not use the LAN TLS CA as a signing CA,
disable verification, or tell users to bypass Windows/SmartScreen warnings.

## Verification and actual-flow tests

```powershell
# Build an older INSTALLER-METADATA fixture using the same current binary payload:
.\scripts\Build-Installer.ps1 -SkipRustBuild -PackageVersion 0.0.9 -OutputDirectory C:\WiniOS\target\installer-fixture
# Reversible but real: modifies the installed program/shortcut/Run value, not data.
.\scripts\Test-Installer.ps1 -Lifecycle -PreviousSetupPath C:\WiniOS\target\installer-fixture\WiniOS-Setup.exe
# After exactly one installed agent is running:
cargo test --offline existing_setup_read_only_https_probe -- --ignored --nocapture
.\scripts\Test-Tailnet.ps1 -NativeClipboard
```

The lifecycle test backs up the current binary pair/icon/shortcut under
`target\installer-validation\<timestamp>` and records restore metadata. It checks
real per-user install, version upgrade, same-version reinstall, optional silent
launch, actual Start Menu launch, busy-instance/path/startup-conflict rejection,
uninstall while the app runs, and clean **program** reinstall with retained data.
The simulated busy-instance case is a native test-owned mutex, not a hung real
console. Original startup enabled/disabled choice is restored at the end.
Content hashes of all personal files, in-memory Credential Manager fingerprint,
Serve JSON and StartupApproved bytes are compared throughout; no credential or
private key is exported/logged. Only read-only Tailscale CLI queries are used.
The API integration uses disposable clipboard text and ordinary verified HTTPS.

This fixture validates installer version transitions using identical current app
code; it is not a claim of compatibility with arbitrary future/older binaries.
Retained-data reinstall is not a fresh Windows user/profile test.

## Rollback and remaining manual checks

Cancel before Install leaves files/startup unchanged. After a successful install,
Quit gracefully and rerun a reviewed older compatible full installer if needed;
retain its AppId. Do not blindly overwrite/delete uninstaller logs or registry
entries. For emergency manual binary rollback, preserve the current package
uninstaller and restore the matching GUI/CLI backup only while no agent runs;
then reinstall the matching full package to restore ownership metadata. Never
delete settings/token/CA to fix an installer problem. Backup restore metadata
records whether startup was registered; keep the correctly quoted installed path.

Manual acceptance still needed beyond silent real-host tests:

1. Standard non-admin **new Windows user/clean VM**, with no developer VC runtime:
   visible dark installer, no elevation prompt, optional launch checked/unchecked,
   Start Menu search/pin, Installed Apps entry and fresh native first-run wizard.
2. Interactive Cancel/back/error dialogs, accessibility, high contrast and DPI,
   uninstall confirmation and final Serve notice; verify keyboard operation.
3. Upgrade/quit while a real clipboard operation or token warning is active;
   manual console Ctrl+C, hung-instance timeout, and two simultaneous installers.
4. Real logout/login startup behavior with checkbox ON/OFF and Windows Startup
   Apps disabled; installer must preserve that separate disabled state.
5. Safari HTTPS and both physical iPhone Shortcuts after installation. Native/API
   tests do not prove a new iPhone or roaming test passed.

## Changed files

`installer/WiniOS.iss` owns the package and lifecycle policy;
`installer/INSTALLATION.txt` supplies the install/uninstall preservation notice;
`scripts/Build-Installer.ps1` stages/builds the static-CRT package;
`scripts/Installer.Common.ps1` inspects PE architecture/subsystem/imports;
`scripts/Test-Installer.ps1` verifies packaging and actual Windows lifecycle;
README and this guide document releases, testing and rollback.
No Rust clipboard, HTTP, authentication, TLS, tray or autostart code was rewritten.

## Recorded verification — 2026-11-06

- `cargo fmt --check`, `cargo check --offline`, `cargo build --release --offline`:
  passed. Regular Rust tests: **80 passed, 0 failed, 2 ignored**. The ignored live
  HTTPS setup probe was run separately against the installer-deployed app and
  passed; the clipboard-mutating private-export fixture was not rerun in this mission.
- Static-CRT shipping build and PE import/subsystem verification passed. No
  dynamic/development VC runtime dependency was found in the GUI/CLI payload.
- `Test-Installer.ps1` artifact/policy checks passed. Real Windows **silent**
  lifecycle tests passed: initial registration/program install, older installer
  metadata → current version, same-version reinstall, explicit launch, actual
  Start Menu shortcut launch, graceful running-app replacement/uninstall, startup
  stale-path repair, conflict rejection, fixed-path rejection, busy install AND
  uninstall refusal, uninstall cleanup, unknown-file retention and clean program
  reinstall with personal data retained.
- Personal settings/TLS content, the in-memory Credential Manager fingerprint,
  Serve state and Windows StartupApproved bytes compared unchanged throughout.
  Uninstall removed the recognized Run value; clean reinstall left it OFF.
  The test restored the user's original startup choice afterward.
- Final complete lifecycle logs/results and binary/icon/shortcut backups are in
  `C:\WiniOS\target\installer-validation\20261106-182813`. The original pre-installer
  manual binary/shortcut backup is in `20261106-181801` under the same parent.
  Installer logs explicitly recorded **User privileges: None**, **Administrative
  install mode: No**, and **HKEY_CURRENT_USER** registration.
- An initial test caught a launch Check evaluated too early by Inno; that timing
  dependency was removed and all lifecycle phases were rerun successfully. No
  personal data was changed during that failed attempt.
- `Test-Tailnet.ps1 -NativeClipboard` passed against the installed static-runtime
  package: valid HTTPS health/no-store, missing bearer=401, accepted/repeated/
  retried POST=200, exhausted contention=503, exact native Unicode/LF/CRLF readback,
  GET and later local-copy recovery. Only disposable clipboard data was used.

The installer is available at `C:\WiniOS\target\dist\WiniOS-Setup.exe`, with its
hash manifest beside it. The app is left installed/running at the existing path.
These are actual Windows process/registry/shortcut/uninstall flows, not mocked
file operations. They do **not** claim interactive wizard/Finish-checkbox UI,
fresh-profile/VM, arbitrary old-version compatibility, code signing or new physical
iPhone validation passed. Those limitations and exact manual steps are above.
