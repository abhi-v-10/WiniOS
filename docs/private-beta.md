# WiniOS 0.1.3 private-beta installation and blockers

## Install / upgrade

Keep the trusted previous installer and a protected backup of your current
installation. Verify Setup against its adjacent hash manifest from a trusted
source; a checksum alone is not publisher authentication. This build is unsigned.
Run Setup as your normal user. The fixed destination is
%LOCALAPPDATA%\Programs\WiniOS. Running tray agents quit gracefully before
replacement; busy agents abort installation, never get forcibly killed. Close a
console agent yourself. Choose optional launch on Finish; startup is not enabled
automatically. Search WiniOS in Start; pin using Windows itself if wanted.

Existing settings, Credential Manager token, TLS files, startup preference and
Windows startup policy are preserved. No Tailscale/firewall/trust changes occur.
Fresh Setup creates missing loopback settings only and generates a random local
credential on normal agent startup. Writes default OFF; images are separate opt-in.
Review MagicDNS/HTTPS and the dedicated Serve proxy yourself. No Funnel/public
exposure or TLS bypass. See mission-014.md for setup and mission-015.md for
numbered iPhone guidance. Install only the two links in mission-018.md; their
accessibility and action graphs still need physical iPhone verification.

## Uninstall / rollback

Uninstall via Installed Apps removes owned package files, shortcut and recognized
startup registration. Settings/credentials/TLS and unknown files remain. User-owned
Tailscale Serve may still target the stopped backend: review it yourself; never
reset unrelated routes. Only after explicit approval remove the dedicated listener
with `tailscale serve --https=443 off`. The uninstaller never runs that command.
Clean reinstall after uninstall defaults startup OFF. Re-enable only deliberately.

For rollback, Quit and run a saved trusted compatible older full installer over
the same directory. Do not delete settings/token/certificates or overwrite
uninstaller files/registry manually. Emergency binary rollback restores the
matching backed-up GUI/CLI while no agent runs; Installed Apps metadata can remain
newer. Full-package downgrade compatibility needs separate validation.

## Private-beta blocker checklist

- [ ] Review the supplied Smart Send/Receive links and action graphs on iPhone;
  verify both publisher-defined Import Questions and text/image transfers.
- [ ] Disposable Windows VM/profile: real clean install, upgrade, uninstall and
  clean reinstall; Start search/pin, permissions and startup policy on login.
  Current host has no Sandbox/VM; live user uninstall is intentionally prohibited.
- [ ] Interactive tray/setup visuals, keyboard/DPI/accessibility and physical
  iPhone imports/transfers. Native fixtures do not prove these hardware flows.
- [ ] Independent security/dependency/license review before broader distribution.
  Unsigned publisher identity remains a documented release limitation; resolving
  this checklist does not require purchasing signing services. Inno compiler
  reports non-commercial use; review its applicable license before commercial use.

Readiness is provisional, not a claim these unchecked flows passed. No automatic
update installation, cloud clipboard storage, public publishing or user-specific
Shortcut publication is performed. Use disposable test content with invited
technical testers only after accepting the listed limitations.
