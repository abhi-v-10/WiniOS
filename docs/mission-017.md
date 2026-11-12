# Mission 017 — blocker resolution, 2026-11-11

## Resolved

The 0.1.2 payload still included the Smart Receive guide's historical local
verification/rollback sections. Packaging now keeps its actual API/Shortcut and
manual acceptance instructions but omits those sections. A focused packaging
assertion prevents this regression in all three shipped mission guides.
Signing text now states the unsigned-publisher limitation without requiring any
purchase. No runtime, API, UI, credential or network changes were needed.

Updated artifact: `target/dist/0.1.2-r1/WiniOS-Setup.exe`. This is a packaging-only
revision with unchanged 0.1.2 GUI/CLI; the original `target/dist/0.1.2` installer
and manifest remain retained. One Inno compilation, zero Cargo builds/full tests.

## Verified now

Original and revised focused installer checks passed. Revised staging passed the
live-token/current-hostname/TLS-byte scan. All explicit payload hashes, versions,
PE/subsystem/static-CRT/icon metadata and per-user lifecycle policy checks passed.
The existing Mission 016 rollback GUI/CLI hashes and PE details were checked
against the protected saved manifest; no restore or full-package downgrade was
performed. No app change required deployment, so no restart or installation ran.

## Still blocked — not claimed passed

- At the time of Mission 017, both placeholder pages were inaccessible and the user had
  not yet supplied actual Smart links. Mission 018 supplies two new links. That does not prove the links invalid: action
  graphs, Smart/image support, Import Questions and absence of embedded secrets
  were unverified at that time. The app then kept placeholder labels; no replacement URL
  was invented, customized or published. Review actual links on iPhone before
  entering any token. See mission-015.md for setup precautions.
- Read-only discovery found no Windows Sandbox executable/feature or VirtualBox,
  VMware or QEMU CLI. No isolated Windows lifecycle environment is available.
  Installer fresh install, old-version upgrade, uninstall and clean reinstall
  remain untested for this revision. No live profile was used as a substitute.
- Physical iPhone and interactive GUI/accessibility/login tests remain manual.
  Signing is a documented release limitation, not a purchase requirement.

## Safe isolated lifecycle handoff

Use a disposable Windows VM/Sandbox supplied by the tester. Do not enable or
install virtualization automatically on the working laptop. Map only a clean
folder containing the installer, its manifest and an older trusted installer
read-only; never map the project root, backups, personal settings, TLS or tokens.
Keep network disabled for install/uninstall tests; no Tailscale setup is needed.

In the disposable guest only:

1. Snapshot the fresh guest. Run the current installer without optional launch;
   inspect per-user path, Start Menu target/icon/working directory and Installed
   Apps/uninstaller version. Confirm no elevation request.
2. Revert the guest; install the trusted older package, then upgrade to this one.
   Verify one app when explicitly launched, graceful Quit, version and shortcuts.
3. Create disposable settings and an unrelated sentinel file. Uninstall; verify
   owned files/shortcut/Run registration removed while settings/sentinel remain.
   Do not create or import real user credentials or certificate trust.
4. Clean-program reinstall; check settings retained and startup stays OFF. Test
   startup ON/OFF and Windows policy separately with disposable login sessions.
5. Record exact artifact hashes, guest OS, exits and observed results. Only then
   mark the lifecycle checklist complete. Discard the guest, never the live setup.

Existing operational rollback remains
`C:\WiniOS\target\mission016-deploy-backup-20261111-125948`.
Quit before restoring the matching executable pair. Do not restore personal
configuration, overwrite uninstaller metadata or alter Serve to roll back binaries.
