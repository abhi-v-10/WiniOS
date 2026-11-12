# WiniOS 0.1.3 — private beta

Native Windows text/image clipboard transfer with an iPhone using private
Tailscale HTTPS and authenticated Apple Shortcuts. Windows 11 x64 is the primary
tested platform. The laptop must remain awake, connected and running WiniOS.

Run WiniOS-Setup.exe as your ordinary user; installation is per-user with no
admin requirement. Launch WiniOS from Start and follow native Setup. Install
Tailscale separately on both devices; review and approve private Serve yourself.
WiniOS never provisions Tailscale, firewall rules or certificate trust.

Install only WiniOS Send and WiniOS Receive using docs/mission-018.md. Their
published links and action graphs still require verification on an iPhone.
Do not enter credentials into unreviewed Shortcuts.
Review docs/private-beta.md before testing. Existing working Shortcuts stay valid.

This local artifact is unsigned and is not approved for public distribution.
Hashes detect changes but do not authenticate the publisher. Do not bypass
Windows security warnings. No credentials, certificates or personal endpoint
are supplied; each user configures their own private connection.
